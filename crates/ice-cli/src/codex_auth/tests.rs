use super::*;
use crate::config::{self, Credential};
use crate::oauth_codex::{CodexCredential, CodexOAuthClient, OAuthEndpoints};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde_json::{json, Value};
use std::io;
use std::sync::MutexGuard;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::{timeout, Duration};
use tokio_util::sync::CancellationToken;

struct TempConfig {
    _env_guard: MutexGuard<'static, ()>,
    _temp: tempfile::TempDir,
    previous: Option<std::ffi::OsString>,
}

impl TempConfig {
    fn new() -> Self {
        let env_guard = config::test_support::env_lock().lock().unwrap();
        let previous = std::env::var_os(config::CONFIG_DIR_ENV);
        let temp = tempfile::TempDir::new().unwrap();
        std::env::set_var(config::CONFIG_DIR_ENV, temp.path());
        Self {
            _env_guard: env_guard,
            _temp: temp,
            previous,
        }
    }
}

impl Drop for TempConfig {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(value) => std::env::set_var(config::CONFIG_DIR_ENV, value),
            None => std::env::remove_var(config::CONFIG_DIR_ENV),
        }
    }
}

struct FakeTokenServer {
    url: String,
    requests: mpsc::UnboundedReceiver<String>,
    shutdown: CancellationToken,
    task: JoinHandle<()>,
}

impl FakeTokenServer {
    async fn start(response: Value, response_gate: Option<oneshot::Receiver<()>>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (request_sender, requests) = mpsc::unbounded_channel();
        let shutdown = CancellationToken::new();
        let task_shutdown = shutdown.clone();
        let task = tokio::spawn(async move {
            let mut response_gate = response_gate;
            loop {
                let accepted = tokio::select! {
                    _ = task_shutdown.cancelled() => return,
                    accepted = listener.accept() => accepted,
                };
                let Ok((mut stream, _)) = accepted else {
                    return;
                };
                let request = match read_http_request(&mut stream).await {
                    Ok(request) => request,
                    Err(_) => return,
                };
                let _ = request_sender.send(request);

                if let Some(gate) = response_gate.take() {
                    tokio::select! {
                        _ = task_shutdown.cancelled() => return,
                        _ = gate => {}
                    }
                }

                let body = serde_json::to_vec(&response).unwrap();
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                if stream.write_all(header.as_bytes()).await.is_err()
                    || stream.write_all(&body).await.is_err()
                {
                    return;
                }
            }
        });
        Self {
            url: format!("http://{address}/token"),
            requests,
            shutdown,
            task,
        }
    }

    async fn next_request(&mut self) -> String {
        timeout(Duration::from_secs(3), self.requests.recv())
            .await
            .expect("fake token endpoint was not called")
            .expect("fake token server stopped")
    }

    async fn assert_no_request(&mut self) {
        assert!(
            timeout(Duration::from_millis(150), self.requests.recv())
                .await
                .is_err(),
            "unexpected refresh request"
        );
    }
}

impl Drop for FakeTokenServer {
    fn drop(&mut self) {
        self.shutdown.cancel();
        self.task.abort();
    }
}

async fn read_http_request(stream: &mut TcpStream) -> io::Result<String> {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 1024];
    loop {
        let count = timeout(Duration::from_secs(3), stream.read(&mut chunk))
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "request timed out"))??;
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..count]);
        if bytes.len() > 64 * 1024 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "fake request was too large",
            ));
        }
        let Some(header_end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") else {
            continue;
        };
        let headers = std::str::from_utf8(&bytes[..header_end])
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid request headers"))?;
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })
            .unwrap_or(0);
        if bytes.len() >= header_end + 4 + content_length {
            break;
        }
    }
    String::from_utf8(bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "request was not UTF-8"))
}

fn access_token(account_id: &str) -> String {
    let payload = json!({
        "https://api.openai.com/auth": {
            "chatgpt_account_id": account_id,
        }
    });
    format!(
        "e30.{}.signature",
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).unwrap())
    )
}

fn token_response(access_account: &str, refresh: &str, expires_in: u64) -> Value {
    json!({
        "access_token": access_token(access_account),
        "refresh_token": refresh,
        "expires_in": expires_in,
    })
}

fn oauth_credential(access: &str, refresh: &str, expires: i64, account: &str) -> Credential {
    Credential::Oauth {
        access: access.to_string(),
        refresh: refresh.to_string(),
        expires,
        account_id: Some(account.to_string()),
    }
}

fn test_client(token_url: String) -> CodexOAuthClient {
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    CodexOAuthClient::with_client_and_endpoints(
        http,
        OAuthEndpoints::new(
            "http://127.0.0.1/authorize",
            token_url,
            "http://127.0.0.1/callback",
        ),
    )
}

fn temp_store(credential: Credential) -> TempConfig {
    let temp_config = TempConfig::new();
    config::upsert_credential(PROVIDER_ID, credential).unwrap();
    temp_config
}

#[tokio::test]
async fn cancelled_persist_waiting_for_store_lock_never_saves_later() {
    let _config = TempConfig::new();
    let store_lock = config::lock_auth_store().unwrap();
    let credential = CodexCredential::from_stored(
        "new-access".into(),
        "new-refresh".into(),
        now_seconds().unwrap() + 3600,
        Some("acct-new".into()),
    )
    .unwrap();
    let cancellation = CancellationToken::new();
    let mut task = tokio::spawn({
        let cancellation = cancellation.clone();
        async move { persist_cancellable(credential, cancellation).await }
    });

    tokio::time::sleep(Duration::from_millis(75)).await;
    cancellation.cancel();
    let error = timeout(Duration::from_secs(1), &mut task)
        .await
        .expect("cancelled save remained blocked on the store lock")
        .unwrap()
        .unwrap_err();
    assert_eq!(error, "Request aborted");

    drop(store_lock);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(config::read_oauth_token(PROVIDER_ID).unwrap().is_none());
}

#[tokio::test]
async fn refreshes_once_across_independent_holders_and_persists_rotation() {
    let _config = temp_store(oauth_credential(
        "old-access",
        "old-refresh",
        now_seconds().unwrap() + 10,
        "acct-old",
    ));
    let (release_response, response_gate) = oneshot::channel();
    let mut server = FakeTokenServer::start(
        token_response("acct-new", "rotated-refresh", 3600),
        Some(response_gate),
    )
    .await;
    let original = oauth_credential(
        "old-access",
        "old-refresh",
        now_seconds().unwrap() + 10,
        "acct-old",
    );
    let first = CodexToken::with_client(original.clone(), test_client(server.url.clone())).unwrap();
    let second = CodexToken::with_client(original, test_client(server.url.clone())).unwrap();

    let first_cancel = CancellationToken::new();
    let first_task = tokio::spawn({
        let first = Arc::clone(&first);
        async move { first.fresh(first_cancel).await }
    });
    let request = server.next_request().await;
    assert!(request.contains("refresh_token=old-refresh"));

    let second_task = tokio::spawn({
        let second = Arc::clone(&second);
        async move { second.fresh(CancellationToken::new()).await }
    });
    tokio::time::sleep(Duration::from_millis(40)).await;
    release_response.send(()).unwrap();

    let first_result = timeout(Duration::from_secs(3), first_task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let second_result = timeout(Duration::from_secs(3), second_task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(first_result.access_token(), second_result.access_token());
    assert_eq!(first_result.refresh_token(), "rotated-refresh");
    assert_eq!(first_result.account_id().as_str(), "acct-new");
    server.assert_no_request().await;

    let persisted = config::read_oauth_token(PROVIDER_ID).unwrap().unwrap();
    assert!(matches!(
        persisted,
        Credential::Oauth { access, refresh, expires, account_id }
            if access == first_result.access_token()
                && refresh == "rotated-refresh"
                && expires == first_result.expires_at_epoch_seconds()
                && account_id.as_deref() == Some("acct-new")
    ));
}

#[tokio::test]
async fn does_not_refresh_outside_the_five_minute_window() {
    let _config = temp_store(oauth_credential(
        "still-valid",
        "still-valid-refresh",
        now_seconds().unwrap() + 600,
        "acct-valid",
    ));
    let mut server =
        FakeTokenServer::start(token_response("acct-new", "new-refresh", 3600), None).await;
    let holder = CodexToken::with_client(
        oauth_credential(
            "still-valid",
            "still-valid-refresh",
            now_seconds().unwrap() + 600,
            "acct-valid",
        ),
        test_client(server.url.clone()),
    )
    .unwrap();

    let fresh = holder.fresh(CancellationToken::new()).await.unwrap();
    assert_eq!(fresh.access_token(), "still-valid");
    server.assert_no_request().await;
}

#[tokio::test]
async fn cancellation_aborts_refresh_and_releases_the_store_lock() {
    let _config = temp_store(oauth_credential(
        "old-access",
        "old-refresh",
        now_seconds().unwrap() + 10,
        "acct-old",
    ));
    let (release_response, response_gate) = oneshot::channel();
    let mut server = FakeTokenServer::start(
        token_response("acct-new", "rotated-refresh", 3600),
        Some(response_gate),
    )
    .await;
    let holder = CodexToken::with_client(
        oauth_credential(
            "old-access",
            "old-refresh",
            now_seconds().unwrap() + 10,
            "acct-old",
        ),
        test_client(server.url.clone()),
    )
    .unwrap();
    let cancellation = CancellationToken::new();
    let task = tokio::spawn({
        let holder = Arc::clone(&holder);
        let cancellation = cancellation.clone();
        async move { holder.fresh(cancellation).await }
    });
    let _request = server.next_request().await;
    cancellation.cancel();

    let error = timeout(Duration::from_secs(3), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert_eq!(error, "Request aborted");
    assert!(matches!(
        config::read_oauth_token(PROVIDER_ID).unwrap(),
        Some(Credential::Oauth { access, refresh, .. })
            if access == "old-access" && refresh == "old-refresh"
    ));
    // Another process can briefly contend with the released file lock.
    // Preserve a bounded liveness assertion instead of assuming zero delay.
    let lock = timeout(Duration::from_secs(3), async {
        loop {
            if let Some(lock) = config::try_lock_auth_store().unwrap() {
                break lock;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("cancelled refresh must release the store lock promptly");
    drop(lock);
    let _ = release_response.send(());
}

#[tokio::test]
async fn cancellation_while_waiting_for_the_store_lock_is_prompt() {
    let _config = temp_store(oauth_credential(
        "valid-access",
        "valid-refresh",
        now_seconds().unwrap() + 600,
        "acct-valid",
    ));
    let mut server =
        FakeTokenServer::start(token_response("acct-new", "new-refresh", 3600), None).await;
    let holder = CodexToken::with_client(
        oauth_credential(
            "valid-access",
            "valid-refresh",
            now_seconds().unwrap() + 600,
            "acct-valid",
        ),
        test_client(server.url.clone()),
    )
    .unwrap();
    let store_lock = config::lock_auth_store().unwrap();
    let cancellation = CancellationToken::new();
    let task = tokio::spawn({
        let holder = Arc::clone(&holder);
        let cancellation = cancellation.clone();
        async move { holder.fresh(cancellation).await }
    });

    tokio::time::sleep(Duration::from_millis(75)).await;
    cancellation.cancel();
    let error = timeout(Duration::from_secs(1), task)
        .await
        .expect("cancelled lock wait did not stop promptly")
        .unwrap()
        .unwrap_err();
    assert_eq!(error, "Request aborted");
    drop(store_lock);
    server.assert_no_request().await;

    let acquired = tokio::task::spawn_blocking(config::try_lock_auth_store)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    drop(acquired);
}

#[tokio::test]
async fn removed_credentials_are_not_resurrected_by_an_old_holder() {
    let _config = temp_store(oauth_credential(
        "old-access",
        "old-refresh",
        now_seconds().unwrap() + 10,
        "acct-old",
    ));
    let mut server =
        FakeTokenServer::start(token_response("acct-new", "new-refresh", 3600), None).await;
    let holder = CodexToken::with_client(
        oauth_credential(
            "old-access",
            "old-refresh",
            now_seconds().unwrap() + 10,
            "acct-old",
        ),
        test_client(server.url.clone()),
    )
    .unwrap();
    assert!(config::delete_credential(PROVIDER_ID).unwrap());

    let error = holder.fresh(CancellationToken::new()).await.unwrap_err();
    assert!(error.contains("Codex credentials were removed"));
    server.assert_no_request().await;
    assert!(config::read_oauth_token(PROVIDER_ID).unwrap().is_none());
}

#[tokio::test]
async fn logout_waiting_for_refresh_removes_the_rotated_credential() {
    let _config = temp_store(oauth_credential(
        "old-access",
        "old-refresh",
        now_seconds().unwrap() + 10,
        "acct-old",
    ));
    let (release_response, response_gate) = oneshot::channel();
    let mut server = FakeTokenServer::start(
        token_response("acct-new", "rotated-refresh", 3600),
        Some(response_gate),
    )
    .await;
    let holder = CodexToken::with_client(
        oauth_credential(
            "old-access",
            "old-refresh",
            now_seconds().unwrap() + 10,
            "acct-old",
        ),
        test_client(server.url.clone()),
    )
    .unwrap();
    let refresh_task = tokio::spawn(async move { holder.fresh(CancellationToken::new()).await });
    let _request = server.next_request().await;
    let logout_task = tokio::task::spawn_blocking(|| config::delete_credential(PROVIDER_ID));
    tokio::time::sleep(Duration::from_millis(40)).await;
    assert!(
        !logout_task.is_finished(),
        "logout bypassed the refresh lock"
    );

    release_response.send(()).unwrap();
    refresh_task.await.unwrap().unwrap();
    assert!(logout_task.await.unwrap().unwrap());
    assert!(config::read_oauth_token(PROVIDER_ID).unwrap().is_none());
}

#[tokio::test]
async fn persistence_failure_is_reported_and_does_not_claim_success() {
    let _config = temp_store(oauth_credential(
        "old-access",
        "old-refresh",
        now_seconds().unwrap() + 10,
        "acct-old",
    ));
    let (release_response, response_gate) = oneshot::channel();
    let mut server = FakeTokenServer::start(
        token_response("acct-new", "rotated-refresh", 3600),
        Some(response_gate),
    )
    .await;
    let holder = CodexToken::with_client(
        oauth_credential(
            "old-access",
            "old-refresh",
            now_seconds().unwrap() + 10,
            "acct-old",
        ),
        test_client(server.url.clone()),
    )
    .unwrap();
    let refresh_task = tokio::spawn(async move { holder.fresh(CancellationToken::new()).await });
    let _request = server.next_request().await;

    // Inject a deterministic write/read failure after the transaction has
    // loaded the existing entry and while it still owns the store lock.
    let auth_path = config::auth_path().unwrap();
    std::fs::remove_file(&auth_path).unwrap();
    std::fs::create_dir(&auth_path).unwrap();
    release_response.send(()).unwrap();

    let error = refresh_task.await.unwrap().unwrap_err();
    assert!(error.contains("Could not save refreshed credentials"));
    assert!(config::read_oauth_token(PROVIDER_ID).is_err());

    std::fs::remove_dir(auth_path).unwrap();
}
