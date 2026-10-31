//! Server behavior tests. Mirror `ice:packages/coding-agent/test/observe-server.test.ts`
//! as described by `docs/ice-gap/observability.md` OBS-014..OBS-016.

use super::*;
use crate::observe::recorder::{ObservePersistence, ObserveRecorderOptions, ObserveSessionInfo};
use ice_agent::{AgentEvent, AgentMessage};
use ice_ai::types::UserMessage;

const TOKEN: &str = "test-token-0123";

fn user(text: &str) -> AgentEvent {
    AgentEvent::MessageStart {
        message: AgentMessage::User(UserMessage::new(text.to_string(), 0)),
    }
}

fn recorder(store: Option<&Arc<ObserveRunStore>>, session: &str) -> ObserveRecorder {
    ObserveRecorder::new(ObserveRecorderOptions {
        session_id: session.into(),
        info: ObserveSessionInfo::default(),
        persistence: store.map(|s| Arc::clone(s) as Arc<dyn ObservePersistence>),
    })
}

async fn server(recorder: ObserveRecorder, store: Option<Arc<ObserveRunStore>>) -> ObserveServer {
    start_observe_server(ObserveServerOptions {
        port: 0,
        recorder,
        store,
        clickhouse: None,
        token: Some(TOKEN.into()),
    })
    .await
    .unwrap()
}

async fn read_until(stream: &mut TcpStream, received: &mut String, needle: &str) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !received.contains(needle) {
        let mut buf = [0u8; 4096];
        let n = tokio::time::timeout_at(deadline, stream.read(&mut buf))
            .await
            .unwrap_or_else(|_| panic!("timed out waiting for {needle}: {received}"))
            .unwrap();
        assert!(n > 0, "stream closed: {received}");
        received.push_str(&String::from_utf8_lossy(&buf[..n]));
    }
}

/// Send a raw request; return (status, headers, body).
async fn request(port: u16, path: &str, extra: &str) -> (u16, String, String) {
    request_with_host(port, path, extra, &format!("127.0.0.1:{port}")).await
}

async fn request_with_host(
    port: u16,
    path: &str,
    extra: &str,
    host: &str,
) -> (u16, String, String) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let head = format!("GET {path} HTTP/1.1\r\nHost: {host}\r\n{extra}\r\n");
    stream.write_all(head.as_bytes()).await.unwrap();
    let mut out = Vec::new();
    stream.read_to_end(&mut out).await.unwrap();
    let out = String::from_utf8_lossy(&out).into_owned();
    let (head, body) = out.split_once("\r\n\r\n").unwrap();
    let status = head.split(' ').nth(1).unwrap().parse().unwrap();
    (status, head.to_string(), body.to_string())
}

#[test]
fn host_and_token_checks() {
    assert!(host_allowed(Some("127.0.0.1:4659")));
    assert!(host_allowed(Some("LOCALHOST")));
    assert!(host_allowed(Some("[::1]:80")));
    assert!(!host_allowed(Some("localhost.evil.com")));
    assert!(!host_allowed(Some("127.0.0.1:")));
    assert!(!host_allowed(Some("example.com")));
    assert!(!host_allowed(None));

    assert!(tokens_match("abc", Some("abc")));
    assert!(!tokens_match("abc", Some("abd")));
    assert!(!tokens_match("abc", Some("ab")));
    assert!(!tokens_match("abc", None));
    assert!(!tokens_match("", Some("")));
}

#[test]
fn request_parsing() {
    let request = parse_request(
        "GET /replay?runId=a%20b&limit=5&x HTTP/1.1\r\nHost: localhost\r\nCookie: a=1; icers_observe_token=t=v\r\nAuthorization: bearer  tok",
    )
    .unwrap();
    assert_eq!(request.path, "/replay");
    assert_eq!(request.query.get("runId").unwrap(), "a b");
    assert_eq!(request.query_number("limit"), Some(5.0));
    assert_eq!(request.query_number("x"), None);
    assert_eq!(request.cookie(OBSERVE_COOKIE), Some("t=v"));
    assert_eq!(request.bearer(), Some("tok"));
    assert!(parse_request("garbage").is_none());
    assert!(parse_request("GET / HTTP/1.foo\r\nHost: localhost").is_none());
    assert!(parse_request("GET / HTTP/2.0\r\nHost: localhost").is_none());
}

#[tokio::test]
async fn invalid_analytics_windows_return_bad_request() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(ObserveRunStore::open(&dir.path().join("o.sqlite")).unwrap());
    let srv = server(recorder(Some(&store), "s1"), Some(store)).await;
    let auth = format!("Authorization: Bearer {TOKEN}\r\n");
    for route in ["/api/runs", "/api/overview"] {
        for query in [
            "until=-1",
            "until=-1e100",
            "since=9223372036854775807&until=9223372036854775807",
            "since=0&until=9223372036854775807",
            "since=10&until=5",
            "since=-1&until=5",
        ] {
            let path = format!("{route}?{query}");
            let (status, _, _) = request(srv.port, &path, &auth).await;
            assert_eq!(status, 400, "{path}");
        }
    }
    srv.close().await;
}

#[test]
fn token_file_is_created_once_and_private() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agent/observe.token");
    let first = load_observe_token(&path).unwrap();
    assert_eq!(first.len(), 64);
    assert!(first.bytes().all(|b| b.is_ascii_hexdigit()));
    assert_eq!(load_observe_token(&path).unwrap(), first);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    std::fs::write(&path, "  custom \n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    }
    assert_eq!(load_observe_token(&path).unwrap(), "custom");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}

#[test]
fn sse_frames() {
    assert_eq!(
        sse_frame("runs", &serde_json::json!([1])),
        "event: runs\ndata: [1]\n\n"
    );
}

#[tokio::test]
async fn auth_host_and_dashboard_cookie() {
    let rec = recorder(None, "s1");
    let srv = server(rec, None).await;
    let port = srv.port;
    assert!(!srv.shared);
    assert!(srv.dashboard_url.ends_with(&format!("/?token={TOKEN}")));

    let (status, _, body) = request_with_host(port, "/state", "", "evil.example").await;
    assert_eq!((status, body.as_str()), (403, "forbidden"));
    let (status, _, _) = request(port, "/state", "").await;
    assert_eq!(status, 401);
    let (status, _, _) = request(port, "/state?token=wrong", "").await;
    assert_eq!(status, 401);

    let (status, head, body) = request(port, &format!("/?token={TOKEN}"), "").await;
    assert_eq!(status, 200);
    assert!(head.contains("text/html"));
    assert!(head.contains("Referrer-Policy: no-referrer"));
    assert!(head.contains("Content-Security-Policy: default-src 'none'; script-src 'self';"));
    assert!(head.contains(&format!(
        "Set-Cookie: {OBSERVE_COOKIE}={TOKEN}; HttpOnly; SameSite=Strict; Path=/"
    )));
    assert!(body.contains("<html") || body.contains("<!doctype"));

    let cookie = format!("Cookie: {OBSERVE_COOKIE}={TOKEN}\r\n");
    let (status, head, _) = request(port, "/", &cookie).await;
    assert_eq!(status, 200);
    assert!(
        !head.contains("Set-Cookie"),
        "only query auth sets the cookie"
    );
    let (status, _, body) = request(port, "/state", &cookie).await;
    assert_eq!(status, 200);
    let state: ObserveState = serde_json::from_str(&body).unwrap();
    assert_eq!(state.header.session_id, "s1");

    let bearer = format!("Authorization: Bearer {TOKEN}\r\n");
    let (status, _, _) = request(port, "/nope", &bearer).await;
    assert_eq!(status, 404);
    // No store: history routes do not exist.
    let (status, _, _) = request(port, "/runs", &bearer).await;
    assert_eq!(status, 404);
    let (status, _, body) = request(port, "/api/runs", &bearer).await;
    assert_eq!(
        (status, body.as_str()),
        (404, r#"{"error":"no history store"}"#)
    );
    for (asset, content_type) in [
        ("/main.js", "text/javascript"),
        ("/styles.css", "text/css"),
        ("/licenses.txt", "text/plain"),
    ] {
        let (status, head, _) = request(port, asset, &bearer).await;
        assert_eq!(status, 200, "{asset}");
        assert!(head.contains(content_type), "{asset}: {head}");
    }
    let (status, _, _) = request(port, "/main.js", "").await;
    assert_eq!(status, 401, "assets need the token too");
    srv.close().await;
}

#[tokio::test]
async fn analytics_routes_read_the_sqlite_history() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(ObserveRunStore::open(&dir.path().join("o.sqlite")).unwrap());
    let rec = recorder(Some(&store), "s1");
    rec.handle_agent_event(&user("hello"));
    rec.handle_agent_event(&AgentEvent::ToolExecutionStart {
        tool_call_id: "c1".into(),
        tool_name: "write".into(),
        args: serde_json::json!({ "path": "notes.md", "content": "x" }),
    });
    let run_id = rec.run_id();
    let srv = server(rec.clone(), Some(Arc::clone(&store))).await;
    let bearer = format!("Authorization: Bearer {TOKEN}\r\n");

    let (status, _, body) = request(srv.port, "/api/meta", &bearer).await;
    assert_eq!(status, 200);
    let meta: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(meta["sources"], serde_json::json!(["sqlite"]));
    assert_eq!(meta["defaultSource"], "sqlite");
    assert_eq!(meta["liveRunId"], run_id.as_str());
    assert!(meta["clickhouse"].is_null());

    let (status, _, body) = request(srv.port, "/api/runs?since=0", &bearer).await;
    assert_eq!(status, 200, "{body}");
    let list: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(list["source"], "sqlite");
    assert!(list.get("sourceError").is_none());
    let row = &list["runs"][0];
    assert_eq!(row["runId"], run_id.as_str());
    assert_eq!(
        (row["toolCalls"].as_u64(), row["files"].as_u64()),
        (Some(1), Some(1))
    );

    let (status, _, body) = request(srv.port, "/api/overview?since=0&tz=60", &bearer).await;
    assert_eq!(status, 200, "{body}");
    let overview: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(overview["totals"]["runs"], 1);
    assert_eq!(overview["totals"]["toolCalls"], 1);
    assert_eq!(overview["files"][0]["path"], "notes.md");
    assert_eq!(overview["heatmap"].as_array().unwrap().len(), 7);

    let (status, _, body) = request(srv.port, "/api/overview?source=clickhouse", &bearer).await;
    assert_eq!(
        (status, body.as_str()),
        (404, r#"{"error":"source clickhouse is not configured"}"#)
    );
    srv.close().await;
    rec.stop();
}

#[tokio::test]
async fn runs_replay_and_combined_state() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(ObserveRunStore::open(&dir.path().join("o.sqlite")).unwrap());
    let other = recorder(Some(&store), "other-session");
    other.handle_agent_event(&user("from another process"));
    let current = recorder(Some(&store), "current");
    current.handle_agent_event(&user("local"));
    let srv = server(current.clone(), Some(Arc::clone(&store))).await;
    let port = srv.port;
    let auth = format!("Authorization: Bearer {TOKEN}\r\n");

    let (status, _, body) = request(port, "/runs?limit=10", &auth).await;
    assert_eq!(status, 200);
    let runs: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(runs["runs"].as_array().unwrap().len(), 2);

    let encoded =
        percent_encoding::utf8_percent_encode(&other.run_id(), percent_encoding::NON_ALPHANUMERIC)
            .to_string();
    let (status, _, body) = request(port, &format!("/replay/{encoded}?after=1"), &auth).await;
    assert_eq!(status, 200);
    let replay: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(replay["run"]["runId"], other.run_id());
    assert_eq!(replay["events"].as_array().unwrap().len(), 1);
    assert_eq!(replay["events"][0]["sequence"], 2);

    let (status, _, body) = request(port, "/replay?sessionId=other-session", &auth).await;
    assert_eq!(status, 200);
    assert!(body.contains("from another process"));
    let (status, _, body) = request(port, "/replay/%FF", &auth).await;
    assert_eq!(
        (status, body.as_str()),
        (400, r#"{"error":"malformed run id"}"#)
    );
    let (status, _, _) = request(port, "/replay?runId=missing", &auth).await;
    assert_eq!(status, 404);
    let (status, _, _) = request(port, "/state?runId=missing", &auth).await;
    assert_eq!(status, 404);
    let (status, _, body) = request(port, &format!("/state?runId={encoded}"), &auth).await;
    assert_eq!(status, 200);
    assert!(body.contains("from another process"));

    let (status, _, body) = request(port, "/state?view=all", &auth).await;
    assert_eq!(status, 200);
    let combined: CombinedObserveState = serde_json::from_str(&body).unwrap();
    assert_eq!(combined.view, "all");
    assert_eq!(combined.live_run_id, current.run_id());
    assert_eq!(combined.state.header.session_id, "current");
    assert_eq!(combined.runs.len(), 2);
    let mut texts: Vec<_> = combined
        .state
        .items
        .iter()
        .map(|i| i.text.clone().unwrap())
        .collect();
    // Cross-run events can share millisecond timestamps, so run-ID tie-breaking
    // does not guarantee that their display order matches creation order.
    texts.sort_unstable();
    assert_eq!(texts, ["from another process", "local"]);
    assert_eq!(combined.state.threads.len(), 2);
    srv.close().await;
}

#[tokio::test]
async fn events_stream_snapshot_live_and_foreign_activity() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(ObserveRunStore::open(&dir.path().join("o.sqlite")).unwrap());
    let other = recorder(Some(&store), "other");
    other.handle_agent_event(&user("old foreign"));
    let current = recorder(Some(&store), "current");
    let srv = server(current.clone(), Some(Arc::clone(&store))).await;

    let mut stream = TcpStream::connect(("127.0.0.1", srv.port)).await.unwrap();
    let head = format!(
        "GET /events?token={TOKEN} HTTP/1.1\r\nHost: localhost:{}\r\n\r\n",
        srv.port
    );
    stream.write_all(head.as_bytes()).await.unwrap();

    let mut received = String::new();
    read_until(&mut stream, &mut received, "event: snapshot").await;
    current.handle_agent_event(&user("live local"));
    read_until(&mut stream, &mut received, "live local").await;
    other.handle_agent_event(&user("new foreign"));
    read_until(&mut stream, &mut received, "new foreign").await;
    assert!(received.contains("text/event-stream"));
    assert!(received.contains("event: activity\ndata: "));
    // Historical foreign events are not re-sent on the stream after the snapshot.
    assert_eq!(received.matches("\"text\":\"old foreign\"").count(), 1);
    srv.close().await;
}

#[tokio::test]
async fn a_busy_port_is_reported_as_shared() {
    let first = server(recorder(None, "a"), None).await;
    let second = start_observe_server(ObserveServerOptions {
        port: first.port,
        recorder: recorder(None, "b"),
        store: None,
        clickhouse: None,
        token: Some(TOKEN.into()),
    })
    .await
    .unwrap();
    assert!(second.shared);
    assert_eq!(second.port, first.port);
    second.close().await;
    first.close().await;
}

#[tokio::test]
async fn a_busy_port_without_a_persistent_token_fails_closed() {
    let first = server(recorder(None, "a"), None).await;
    let result = start_observe_server(ObserveServerOptions {
        port: first.port,
        recorder: recorder(None, "b"),
        store: None,
        clickhouse: None,
        token: None,
    })
    .await;
    first.close().await;

    assert!(result.is_err());
    assert!(result.err().unwrap().contains("persistent observe token"));
}

#[test]
fn dashboard_bundle_is_self_contained() {
    // No external scripts, styles or fonts: the page must work offline and
    // never leak the token-bearing URL to a third party.
    let bundle: Vec<(&str, String)> = std::iter::once(("/", DASHBOARD_HTML.to_string()))
        .chain(
            UI_ASSETS
                .iter()
                .filter(|(path, _, _)| path.ends_with(".js") || path.ends_with(".css"))
                .map(|(path, _, body)| (*path, String::from_utf8_lossy(body).into_owned())),
        )
        .collect();
    for (path, text) in &bundle {
        assert!(
            !text.is_empty(),
            "{path} is empty; run scripts/build-observe-ui.sh"
        );
        for needle in [
            "src=\"http",
            "href=\"http",
            "@import",
            "url(http",
            "url(\"http",
        ] {
            assert!(!text.contains(needle), "{path} references {needle}");
        }
    }
    // The page loads the served script and styles, with no inline script
    // that the CSP would block.
    for path in ["main.js", "styles.css"] {
        assert!(
            DASHBOARD_HTML.contains(path),
            "index.html does not load {path}"
        );
    }
    assert!(
        !DASHBOARD_HTML.contains("<script>"),
        "inline scripts violate the CSP"
    );
    assert!(
        !DASHBOARD_HTML.contains("onload="),
        "inline handlers violate the CSP"
    );
}
