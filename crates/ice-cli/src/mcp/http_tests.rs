use super::*;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader as AsyncBufReader};
use tokio::net::{TcpListener, TcpStream};

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Streamable,
    Legacy,
    Unauthorized,
}

/// What the fixture saw, in arrival order: `METHOD path session? body-method`.
#[derive(Clone, Default)]
struct Log(Arc<Mutex<Vec<String>>>);
impl Log {
    fn push(&self, line: String) {
        self.0.lock().unwrap().push(line);
    }
    fn lines(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
    async fn wait(&self, needle: &str) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while !self.lines().iter().any(|l| l.contains(needle)) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("never saw {needle:?} in {:?}", self.lines()));
    }
}

struct Request {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

async fn read_request(stream: &mut AsyncBufReader<TcpStream>) -> Option<Request> {
    let mut line = String::new();
    stream.read_line(&mut line).await.ok()?;
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_owned();
    let path = parts.next()?.to_owned();
    let mut headers = HashMap::new();
    loop {
        let mut line = String::new();
        stream.read_line(&mut line).await.ok()?;
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        let (name, value) = line.split_once(':')?;
        headers.insert(name.to_ascii_lowercase(), value.trim().to_owned());
    }
    let length = headers
        .get("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0; length];
    stream.read_exact(&mut body).await.ok()?;
    Some(Request {
        method,
        path,
        headers,
        body,
    })
}

fn sse(value: &Value) -> String {
    format!("event: message\ndata: {value}\n\n")
}

async fn respond(stream: &mut AsyncBufReader<TcpStream>, status: &str, headers: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.get_mut().write_all(response.as_bytes()).await;
}

/// An MCP server over HTTP with one `echo` tool. A `tools/call` streams a
/// `ping` request before its result; the GET stream (streamable) sends one too.
async fn fixture(mode: Mode) -> (String, Log) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    let log = Log::default();
    // Legacy replies travel over the open GET stream.
    let (legacy_tx, legacy_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let legacy_rx = Arc::new(tokio::sync::Mutex::new(legacy_rx));
    let server_log = log.clone();
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let (log, legacy_tx, legacy_rx) =
                (server_log.clone(), legacy_tx.clone(), legacy_rx.clone());
            tokio::spawn(async move {
                let mut stream = AsyncBufReader::new(stream);
                let Some(request) = read_request(&mut stream).await else {
                    return;
                };
                let session = request.headers.get(SESSION).cloned().unwrap_or_default();
                let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
                let what = body["method"]
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("reply:{}", body["id"]));
                log.push(format!(
                    "{} {} {session} {what}",
                    request.method, request.path
                ));
                if mode == Mode::Unauthorized {
                    return respond(&mut stream, "401 Unauthorized", "", "").await;
                }
                if request.method == "DELETE" {
                    return respond(&mut stream, "200 OK", "", "").await;
                }
                if request.method == "GET" {
                    let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n";
                    let _ = stream.get_mut().write_all(head.as_bytes()).await;
                    if mode == Mode::Legacy {
                        let _ = stream
                            .get_mut()
                            .write_all(b"event: endpoint\ndata: /messages?s=1\n\n")
                            .await;
                        let mut replies = legacy_rx.lock().await;
                        while let Some(event) = replies.recv().await {
                            if stream.get_mut().write_all(event.as_bytes()).await.is_err() {
                                break;
                            }
                        }
                    } else {
                        let ping = json!({"jsonrpc":"2.0","id":"listen-ping","method":"ping"});
                        let _ = stream.get_mut().write_all(sse(&ping).as_bytes()).await;
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                    return;
                }
                if mode == Mode::Legacy && request.path == "/mcp" {
                    return respond(&mut stream, "405 Method Not Allowed", "", "").await;
                }
                let reply =
                    |result: Value| json!({"jsonrpc":"2.0","id":body["id"],"result":result});
                let result = match what.as_str() {
                    "initialize" => reply(json!({
                        "protocolVersion":"2025-06-18",
                        "capabilities":{"tools":{}},
                        "serverInfo":{"name":"http-fixture","version":"1"},
                        "instructions":"Over HTTP."
                    })),
                    "tools/list" => {
                        reply(json!({"tools":[{"name":"echo","inputSchema":{"type":"object"}}]}))
                    }
                    "tools/call" => reply(
                        json!({"content":[{"type":"text","text":format!("echo {}", body["params"]["arguments"])}]}),
                    ),
                    _ => return respond(&mut stream, "202 Accepted", "", "").await,
                };
                if mode == Mode::Legacy {
                    let _ = legacy_tx.send(sse(&result));
                    return respond(&mut stream, "202 Accepted", "", "").await;
                }
                if what == "tools/call" {
                    // A server request related to the call, then the result.
                    let ping = json!({"jsonrpc":"2.0","id":"call-ping","method":"ping"});
                    let events = format!("{}{}", sse(&ping), sse(&result));
                    let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n";
                    let _ = stream.get_mut().write_all(head.as_bytes()).await;
                    let _ = stream.get_mut().write_all(events.as_bytes()).await;
                    return;
                }
                let headers = if what == "initialize" {
                    format!("Content-Type: application/json\r\n{SESSION}: session-1\r\n")
                } else {
                    "Content-Type: application/json\r\n".into()
                };
                respond(&mut stream, "200 OK", &headers, &result.to_string()).await;
            });
        }
    });
    (url, log)
}

fn config(url: &str) -> ServerConfig {
    serde_json::from_value(json!({"url":url,"headers":{"x-test":"${ICERS_MCP_HTTP_TEST_HEADER}"}}))
        .unwrap()
}

#[tokio::test]
async fn abandoned_http_calls_are_not_sent_from_queue() {
    for cancel in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        let log = Log::default();
        let release = Arc::new(tokio::sync::Semaphore::new(0));
        let server_log = log.clone();
        let server_release = release.clone();
        let server = tokio::spawn(async move {
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                let log = server_log.clone();
                let release = server_release.clone();
                tokio::spawn(async move {
                    let mut stream = AsyncBufReader::new(stream);
                    let request = read_request(&mut stream).await.unwrap();
                    let body: Value = serde_json::from_slice(&request.body).unwrap();
                    let method = body["method"].as_str().unwrap();
                    log.push(method.to_owned());
                    if method == "test/block" {
                        release.acquire().await.unwrap().forget();
                    }
                    if body.get("id").is_some() {
                        let reply = json!({"jsonrpc":"2.0","id":body["id"],"result":{}});
                        respond(
                            &mut stream,
                            "200 OK",
                            "Content-Type: application/json\r\n",
                            &reply.to_string(),
                        )
                        .await;
                    } else {
                        respond(&mut stream, "202 Accepted", "", "").await;
                    }
                });
            }
        });
        let config: ServerConfig = serde_json::from_value(json!({"url":url})).unwrap();
        let client =
            Client::start("web", &config, Path::new("."), Handlers::default(), None).unwrap();
        client.notify("test/block", json!({}));
        log.wait("test/block").await;

        let signal = CancellationToken::new();
        let mut call = Box::pin(client.request(
            "tools/call",
            json!({"name":"must-not-run","arguments":{}}),
            signal.clone(),
            Duration::from_millis(50),
        ));
        assert!(futures::poll!(&mut call).is_pending());
        if cancel {
            signal.cancel();
        }
        let error = call.await.unwrap_err();
        assert!(error.contains(if cancel { "cancelled" } else { "timed out" }));
        assert!(client.pending.lock().unwrap().is_empty());

        release.add_permits(1);
        client
            .request(
                "test/barrier",
                json!({}),
                CancellationToken::new(),
                Duration::from_secs(5),
            )
            .await
            .unwrap();
        // Allow any concurrently spawned POST to reach the local fixture too.
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(
            !log.lines().iter().any(|method| method == "tools/call"),
            "abandoned call reached the server: {:?}",
            log.lines()
        );
        client.shut_down("test done");
        server.abort();
    }
}

#[tokio::test]
async fn streamable_http_initializes_calls_and_answers_server_requests() {
    std::env::set_var("ICERS_MCP_HTTP_TEST_HEADER", "yes");
    let (url, log) = fixture(Mode::Streamable).await;
    let config = config(&url);
    let client = Client::start("web", &config, Path::new("."), Handlers::default(), None).unwrap();
    let tools = client
        .discover("web", &config, &OutputGuard::default())
        .await
        .unwrap();
    assert_eq!(tools[0].schema.name, "web_echo");
    assert_eq!(client.instructions(), Some("Over HTTP."));
    let result = client
        .request(
            "tools/call",
            json!({"name":"echo","arguments":{"a":1}}),
            CancellationToken::new(),
            CALL_TIMEOUT,
        )
        .await
        .unwrap();
    assert_eq!(result["content"][0]["text"], "echo {\"a\":1}");
    // Pings sent inside the call's SSE response and over the GET stream are answered.
    log.wait("POST /mcp session-1 reply:\"call-ping\"").await;
    log.wait("POST /mcp session-1 reply:\"listen-ping\"").await;
    let lines = log.lines();
    assert!(lines[0].starts_with("POST /mcp  initialize"), "{lines:?}");
    assert!(lines.contains(&"POST /mcp session-1 notifications/initialized".to_string()));
    assert!(lines.iter().any(|l| l.starts_with("GET /mcp session-1")));
    client.shut_down("done");
    log.wait("DELETE /mcp session-1").await;
}

#[tokio::test]
async fn legacy_sse_servers_are_reached_through_their_endpoint() {
    let (url, log) = fixture(Mode::Legacy).await;
    let config: ServerConfig = serde_json::from_value(json!({"url":url})).unwrap();
    let client = Client::start("old", &config, Path::new("."), Handlers::default(), None).unwrap();
    let tools = client
        .discover("old", &config, &OutputGuard::default())
        .await
        .unwrap();
    assert_eq!(tools[0].schema.name, "old_echo");
    let lines = log.lines();
    assert!(
        lines.iter().any(|l| l.starts_with("POST /mcp  initialize")),
        "{lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("POST /messages?s=1  initialize")),
        "{lines:?}"
    );
}

#[tokio::test]
async fn http_errors_surface_and_missing_variables_fail_before_sending() {
    let (url, _) = fixture(Mode::Unauthorized).await;
    let config: ServerConfig = serde_json::from_value(json!({"url":url})).unwrap();
    let client =
        Client::start("locked", &config, Path::new("."), Handlers::default(), None).unwrap();
    let error = client
        .discover("locked", &config, &OutputGuard::default())
        .await
        .err()
        .unwrap();
    assert!(
        error.contains("requires authentication (HTTP 401)"),
        "{error}"
    );

    let missing: ServerConfig =
        serde_json::from_value(json!({"url":"http://127.0.0.1:9/${ICERS_MCP_HTTP_TEST_MISSING}"}))
            .unwrap();
    let error = Client::start("x", &missing, Path::new("."), Handlers::default(), None)
        .err()
        .unwrap();
    assert!(error.contains("ICERS_MCP_HTTP_TEST_MISSING"), "{error}");
    let both: ServerConfig =
        serde_json::from_value(json!({"url":"http://127.0.0.1:9/","command":"c"})).unwrap();
    assert!(Client::start("x", &both, Path::new("."), Handlers::default(), None).is_err());
}

#[test]
fn interpolation_bearer_tokens_and_sse_parsing() {
    std::env::set_var("ICERS_MCP_HTTP_TEST_TOKEN", "tok");
    assert_eq!(
        interpolate("a${ICERS_MCP_HTTP_TEST_TOKEN}b").unwrap(),
        "atokb"
    );
    assert_eq!(
        interpolate("x $env:ICERS_MCP_HTTP_TEST_TOKEN/y").unwrap(),
        "x tok/y"
    );
    assert_eq!(interpolate("cost $5").unwrap(), "cost $5");
    assert!(interpolate("${unterminated").is_err());

    let bearer: ServerConfig = serde_json::from_value(json!({
        "url":"https://example.test/mcp","auth":"bearer","bearerTokenEnv":"ICERS_MCP_HTTP_TEST_TOKEN"
    }))
    .unwrap();
    let resolved = target(&bearer, None).unwrap();
    assert_eq!(resolved.headers["authorization"], "Bearer tok");
    assert!(resolved.headers["authorization"].is_sensitive());
    let ftp: ServerConfig = serde_json::from_value(json!({"url":"ftp://example.test/"})).unwrap();
    assert!(target(&ftp, None).is_err());

    let mut parser = SseParser::default();
    let mut events = Vec::new();
    parser
        .feed(
            b"event: endpoint\r\ndata: /a\r\n\r\ndata: {\"x\":",
            &mut events,
        )
        .unwrap();
    parser
        .feed(b"1}\ndata: 2\n\n: comment\n\n", &mut events)
        .unwrap();
    assert_eq!(
        events,
        [
            ("endpoint".to_string(), "/a".to_string()),
            ("message".to_string(), "{\"x\":1}\n2".to_string()),
        ]
    );
}

#[tokio::test]
async fn list_mcp_inspects_http_servers_too() {
    let (url, _) = fixture(Mode::Streamable).await;
    let config: ServerConfig = serde_json::from_value(json!({"url":url})).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let tools = discover_server("web", &config, dir.path(), dir.path())
        .await
        .unwrap();
    assert_eq!(tools[0].schema.name, "web_echo");
    let oauth: ServerConfig = serde_json::from_value(json!({"url":url,"auth":"oauth"})).unwrap();
    let error = discover_server("web", &oauth, dir.path(), dir.path())
        .await
        .err()
        .unwrap();
    assert!(error.contains("/mcp-auth web"), "{error}");
}
