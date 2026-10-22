use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const FAR_FUTURE: i64 = 4_000_000_000;

const RESPONSE: &str = r#"{"models":[
    {"slug":"gpt-6.1-sol","display_name":"GPT-6.1-Sol","visibility":"list",
     "context_window":272000,"input_modalities":["text","image"],
     "supported_reasoning_levels":[{"effort":"low"},{"effort":"max"}]},
    {"slug":"gpt-text-only","display_name":"Text Only","visibility":"list",
     "input_modalities":["text"],"supported_reasoning_levels":[{"effort":"high"}]},
    {"slug":"codex-auto-review","display_name":"Review","visibility":"hide"}
]}"#;

fn credential() -> CodexCredential {
    CodexCredential::from_stored(
        "access-token".into(),
        "refresh-token".into(),
        FAR_FUTURE,
        Some("acct-1".into()),
    )
    .unwrap()
}

/// Serve one canned response and report the raw request head.
async fn fake_server(status: u16, body: &'static str) -> (String, tokio::task::JoinHandle<String>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut chunk = [0u8; 2048];
        while !request.windows(4).any(|window| window == b"\r\n\r\n") {
            let read = socket.read(&mut chunk).await.unwrap();
            request.extend_from_slice(&chunk[..read]);
        }
        let response = format!(
            "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        socket.write_all(response.as_bytes()).await.unwrap();
        String::from_utf8(request).unwrap().to_ascii_lowercase()
    });
    (base, server)
}

#[tokio::test]
async fn fetch_sends_account_identity_and_keeps_only_listed_models() {
    let (base, server) = fake_server(200, RESPONSE).await;
    let models = fetch(&base, &credential(), &CancellationToken::new())
        .await
        .unwrap();
    let request = server.await.unwrap();

    assert!(request.starts_with(&format!(
        "get /codex/models?client_version={} ",
        env!("CARGO_PKG_VERSION")
    )));
    assert!(request.contains("authorization: bearer access-token"));
    assert!(request.contains("chatgpt-account-id: acct-1"));
    assert_eq!(
        models,
        vec![
            DiscoveredModel {
                id: "gpt-6.1-sol".into(),
                name: "GPT-6.1-Sol".into(),
                context_window: 272_000,
                supports_images: true,
                supports_max_thinking: true,
            },
            DiscoveredModel {
                id: "gpt-text-only".into(),
                name: "Text Only".into(),
                context_window: 272_000,
                supports_images: false,
                supports_max_thinking: false,
            },
        ]
    );
}

#[tokio::test]
async fn fetch_reports_http_and_shape_errors() {
    let (base, _server) = fake_server(401, r#"{"detail":"no"}"#).await;
    let error = fetch(&base, &credential(), &CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(error, "HTTP 401");

    let (base, _server) = fake_server(200, r#"{"unexpected":true}"#).await;
    let error = fetch(&base, &credential(), &CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(error, "Unrecognized model list response");
}

#[test]
fn models_url_accepts_every_base_shape() {
    for base in [
        "https://chatgpt.com/backend-api",
        "https://chatgpt.com/backend-api/",
        "https://chatgpt.com/backend-api/codex",
    ] {
        assert_eq!(
            models_url(base).unwrap().path(),
            "/backend-api/codex/models",
            "{base}"
        );
    }
}

#[test]
fn cache_round_trips_and_an_empty_cache_is_ignored() {
    let _guard = crate::config::test_support::env_lock().lock().unwrap();
    let previous = std::env::var_os(config::CONFIG_DIR_ENV);
    let dir = tempfile::TempDir::new().unwrap();
    std::env::set_var(config::CONFIG_DIR_ENV, dir.path());

    assert_eq!(load(), None);
    let models = vec![DiscoveredModel {
        id: "gpt-6-luna".into(),
        name: "GPT-6-Luna".into(),
        context_window: 272_000,
        supports_images: true,
        supports_max_thinking: false,
    }];
    store(&models).unwrap();
    assert_eq!(load(), Some(models));
    store(&[]).unwrap();
    assert_eq!(load(), None);
    assert!(clear());
    assert!(!clear());
    assert!(!dir.path().join(CACHE_FILE).exists());

    match previous {
        Some(value) => std::env::set_var(config::CONFIG_DIR_ENV, value),
        None => std::env::remove_var(config::CONFIG_DIR_ENV),
    }
}
