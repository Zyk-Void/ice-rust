use super::*;
use base64::Engine;
use sha2::Digest;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader as AsyncBufReader};
use tokio::net::TcpListener;

/// An MCP endpoint at `/mcp` that wants `Bearer good`, and an authorization
/// server whose issuer is `<base>/auth` (so metadata paths are inserted).
struct Fixture {
    base: String,
    /// Token endpoint form bodies, in order.
    tokens: Arc<Mutex<Vec<String>>>,
}

async fn fixture(wrong_issuer: bool) -> Fixture {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let tokens = Arc::new(Mutex::new(Vec::new()));
    let (server_base, server_tokens) = (base.clone(), tokens.clone());
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let (base, tokens) = (server_base.clone(), server_tokens.clone());
            tokio::spawn(async move {
                let mut stream = AsyncBufReader::new(stream);
                let mut line = String::new();
                stream.read_line(&mut line).await.unwrap();
                let mut parts = line.split_whitespace();
                let (method, path) = (
                    parts.next().unwrap().to_owned(),
                    parts.next().unwrap().to_owned(),
                );
                let mut headers = HashMap::new();
                loop {
                    let mut line = String::new();
                    stream.read_line(&mut line).await.unwrap();
                    let line = line.trim_end().to_owned();
                    if line.is_empty() {
                        break;
                    }
                    let (name, value) = line.split_once(':').unwrap();
                    headers.insert(name.to_ascii_lowercase(), value.trim().to_owned());
                }
                let length = headers
                    .get("content-length")
                    .map_or(0, |v| v.parse().unwrap());
                let mut body = vec![0; length];
                stream.read_exact(&mut body).await.unwrap();
                let body = String::from_utf8(body).unwrap();
                let json = |value: Value| (200, value.to_string(), String::new());
                let (status, text, extra) = match (method.as_str(), path.as_str()) {
                    ("POST", "/mcp") if headers.get("authorization").map(String::as_str) == Some("Bearer good") => {
                        let message: Value = serde_json::from_str(&body).unwrap();
                        let reply = |result: Value| json!({"jsonrpc":"2.0","id":message["id"],"result":result});
                        match message["method"].as_str() {
                            Some("initialize") => json(reply(json!({"protocolVersion":"2025-06-18","capabilities":{"tools":{}},"serverInfo":{"name":"oauth","version":"1"}}))),
                            Some("tools/list") => json(reply(json!({"tools":[{"name":"whoami","inputSchema":{"type":"object"}}]}))),
                            _ => (202, String::new(), String::new()),
                        }
                    }
                    ("POST", "/mcp") => (
                        401,
                        String::new(),
                        format!("WWW-Authenticate: Bearer resource_metadata=\"{base}/.well-known/oauth-protected-resource/mcp\"\r\n"),
                    ),
                    ("GET", "/.well-known/oauth-protected-resource/mcp") => json(json!({
                        "resource": format!("{base}/mcp"),
                        "authorization_servers": [format!("{base}/auth")],
                    })),
                    ("GET", "/.well-known/oauth-authorization-server/auth") => json(json!({
                        "issuer": if wrong_issuer { "https://evil.example".to_string() } else { format!("{base}/auth") },
                        "authorization_endpoint": format!("{base}/auth/authorize"),
                        "token_endpoint": format!("{base}/auth/token"),
                        "registration_endpoint": format!("{base}/auth/register"),
                        "code_challenge_methods_supported": ["S256"],
                    })),
                    ("POST", "/auth/register") => json(json!({"client_id":"dyn-client"})),
                    ("POST", "/auth/token") => {
                        tokens.lock().unwrap().push(body.clone());
                        if body.contains("code=bad") {
                            (400, "{\"error\":\"invalid_grant\"}".into(), String::new())
                        } else {
                            json(json!({"access_token":"good","refresh_token":"r2","expires_in":3600}))
                        }
                    }
                    _ => (404, String::new(), String::new()),
                };
                let response = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{text}",
                    text.len()
                );
                let _ = stream.get_mut().write_all(response.as_bytes()).await;
            });
        }
    });
    Fixture { base, tokens }
}

async fn session(dir: &Path, server: Value) -> (AgentHarness, McpManager) {
    std::fs::write(
        dir.join("mcp.json"),
        serde_json::to_vec(&json!({"mcpServers":{"remote":server}})).unwrap(),
    )
    .unwrap();
    let harness = AgentHarness::create(Default::default()).await.unwrap();
    let manager = McpManager::new(
        &harness,
        &Default::default(),
        dir,
        dir.into(),
        false,
        Default::default(),
    );
    manager.reconnect(None).await.unwrap();
    (harness, manager)
}
fn status(manager: &McpManager) -> ServerStatus {
    manager.snapshot().remove(0).status
}
async fn connected(manager: &McpManager) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !matches!(status(manager), ServerStatus::Connected { .. }) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("never connected: {:?}", status(manager)));
}
fn query(url: &str) -> HashMap<String, String> {
    reqwest::Url::parse(url)
        .unwrap()
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect()
}
fn form(body: &str) -> HashMap<String, String> {
    query(&format!("http://x/?{body}"))
}

#[tokio::test]
async fn browser_sign_in_discovers_registers_and_stores_tokens() {
    let fixture = fixture(false).await;
    let dir = tempfile::tempdir().unwrap();
    let url = format!("{}/mcp", fixture.base);
    let (_harness, manager) = session(dir.path(), json!({"url":url,"auth":"oauth"})).await;
    assert!(
        matches!(status(&manager), ServerStatus::Failed(ref e) if e.contains("/mcp-auth remote")),
        "{:?}",
        status(&manager)
    );

    let authorize = manager.auth_start("remote").await.unwrap();
    assert!(authorize.starts_with(&format!("{}/auth/authorize?", fixture.base)));
    let params = query(&authorize);
    assert_eq!(params["client_id"], "dyn-client");
    assert_eq!(params["resource"], url);
    assert_eq!(params["code_challenge_method"], "S256");
    // The browser lands on the loopback callback.
    let callback = format!(
        "{}?code=good-code&state={}",
        params["redirect_uri"], params["state"]
    );
    let page = reqwest::get(&callback).await.unwrap();
    assert!(page.status().is_success());
    connected(&manager).await;

    let exchange = form(&fixture.tokens.lock().unwrap()[0]);
    assert_eq!(exchange["grant_type"], "authorization_code");
    assert_eq!(exchange["resource"], url);
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(sha2::Sha256::digest(exchange["code_verifier"].as_bytes()));
    assert_eq!(challenge, params["code_challenge"]);
    let stored = load(dir.path(), "remote", &url).unwrap();
    assert_eq!(
        (
            stored.access_token.as_str(),
            stored.refresh_token.as_deref()
        ),
        ("good", Some("r2"))
    );
    assert!(load(dir.path(), "remote", "https://other.example/mcp").is_none());
    assert!(!format!("{stored:?}").contains("good"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.path().join(FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    assert!(manager.logout("remote").await.unwrap());
    assert!(matches!(status(&manager), ServerStatus::Failed(_)));
    assert!(load(dir.path(), "remote", &url).is_none());
}

#[tokio::test]
async fn pasted_callbacks_must_match_the_sign_in_and_refresh_reuses_tokens() {
    let fixture = fixture(false).await;
    let dir = tempfile::tempdir().unwrap();
    let url = format!("{}/mcp", fixture.base);
    let (_harness, manager) = session(dir.path(), json!({"url":url,"auth":"oauth"})).await;
    let authorize = manager.auth_start("remote").await.unwrap();
    let state = query(&authorize)["state"].clone();
    let error = manager
        .auth_complete(
            "remote",
            "http://127.0.0.1:1/callback?code=good-code&state=other",
        )
        .await
        .unwrap_err();
    assert!(error.contains("different sign-in"), "{error}");
    manager
        .auth_complete(
            "remote",
            &format!("http://127.0.0.1:1/callback?code=good-code&state={state}"),
        )
        .await
        .unwrap();
    connected(&manager).await;
    assert!(manager.auth_complete("remote", "good-code").await.is_err());

    // An expired token is refreshed before connecting.
    let mut record = load(dir.path(), "remote", &url).unwrap();
    record.expires_at = Some(0);
    save(dir.path(), "remote", record).unwrap();
    manager.reconnect(Some("remote")).await.unwrap();
    connected(&manager).await;
    let last = form(fixture.tokens.lock().unwrap().last().unwrap());
    assert_eq!(last["grant_type"], "refresh_token");
    assert_eq!(last["refresh_token"], "r2");
}

#[tokio::test]
async fn issuer_mismatch_is_refused_and_client_credentials_need_no_browser() {
    let evil = fixture(true).await;
    let dir = tempfile::tempdir().unwrap();
    let (_harness, manager) = session(
        dir.path(),
        json!({"url":format!("{}/mcp", evil.base),"auth":"oauth"}),
    )
    .await;
    let error = manager.auth_start("remote").await.unwrap_err();
    assert!(error.contains("issuer mismatch"), "{error}");

    let good = fixture(false).await;
    let dir = tempfile::tempdir().unwrap();
    let (_harness, manager) = session(
        dir.path(),
        json!({"url":format!("{}/mcp", good.base),"auth":"oauth","oauth":{"grantType":"client_credentials","clientId":"svc","clientSecret":"s3cret"}}),
    )
    .await;
    connected(&manager).await;
    let grant = form(&good.tokens.lock().unwrap()[0]);
    assert_eq!(grant["grant_type"], "client_credentials");
    assert_eq!(
        (grant["client_id"].as_str(), grant["client_secret"].as_str()),
        ("svc", "s3cret")
    );
    assert!(manager.auth_start("remote").await.is_err());
}

#[test]
fn resource_metadata_hints_parse_quoted_and_bare() {
    assert_eq!(
        resource_metadata_hint(r#"Bearer error="x", resource_metadata="https://a/.well-known/r""#)
            .as_deref(),
        Some("https://a/.well-known/r")
    );
    assert_eq!(
        resource_metadata_hint("Bearer resource_metadata=https://a/r, scope=x").as_deref(),
        Some("https://a/r")
    );
    assert_eq!(resource_metadata_hint("Bearer realm=x"), None);
    let url = reqwest::Url::parse("https://h/api/mcp/").unwrap();
    assert_eq!(
        well_known(&url, "oauth-protected-resource")
            .unwrap()
            .as_str(),
        "https://h/.well-known/oauth-protected-resource/api/mcp"
    );
    assert!(secure(
        &reqwest::Url::parse("http://127.0.0.1:9/x").unwrap()
    ));
    assert!(!secure(
        &reqwest::Url::parse("http://example.com/x").unwrap()
    ));
}
