use super::*;

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use base64::engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD};
use base64::Engine;
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;

#[tokio::test]
async fn callback_listener_accepts_ipv6_localhost_on_the_ipv4_port() {
    let listener = CallbackListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .await
        .unwrap();
    let Some(ipv6) = listener.ipv6_listener.as_ref() else {
        // IPv6 is optional on hosts where the loopback family is unavailable.
        assert!(TcpListener::bind((std::net::Ipv6Addr::LOCALHOST, 0))
            .await
            .is_err());
        return;
    };
    assert_eq!(
        listener.local_addr().unwrap().port(),
        ipv6.local_addr().unwrap().port()
    );
    let address = ipv6.local_addr().unwrap();
    let client = TcpStream::connect(address);
    let (connected, accepted) = tokio::join!(client, listener.accept());
    connected.unwrap();
    assert!(accepted.unwrap().1.is_ipv6());
}

fn jwt_with_account_id(account_id: &str, padded: bool) -> String {
    let payload = serde_json::to_vec(&json!({
        "https://api.openai.com/auth": {
            "chatgpt_account_id": account_id,
        }
    }))
    .unwrap();
    let payload = if padded {
        URL_SAFE.encode(payload)
    } else {
        URL_SAFE_NO_PAD.encode(payload)
    };
    format!("header.{payload}.signature")
}

fn successful_token_response(account_id: &str) -> String {
    serde_json::to_string(&json!({
        "access_token": jwt_with_account_id(account_id, false),
        "refresh_token": "refresh-secret",
        "expires_in": 3600,
    }))
    .unwrap()
}

struct CapturedRequest {
    method: String,
    target: String,
    headers: HashMap<String, String>,
    body: String,
}

async fn spawn_token_server(
    status: u16,
    body: String,
) -> (String, tokio::task::JoinHandle<CapturedRequest>) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let captured = read_captured_request(&mut stream).await;
        let status_text = if status == 200 { "OK" } else { "Unauthorized" };
        let response = format!(
            "HTTP/1.1 {status} {status_text}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).await.unwrap();
        captured
    });
    (format!("http://{address}/oauth/token"), server)
}

async fn read_captured_request(stream: &mut TcpStream) -> CapturedRequest {
    let mut bytes = Vec::new();
    let header_end = loop {
        let mut chunk = [0_u8; 1024];
        let read = stream.read(&mut chunk).await.unwrap();
        assert_ne!(read, 0, "client closed before writing the request");
        bytes.extend_from_slice(&chunk[..read]);
        if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break index + 4;
        }
    };
    let headers = String::from_utf8(bytes[..header_end].to_vec()).unwrap();
    let mut lines = headers.split("\r\n");
    let mut request_line = lines.next().unwrap().split_ascii_whitespace();
    let method = request_line.next().unwrap().to_owned();
    let target = request_line.next().unwrap().to_owned();
    let mut parsed_headers = HashMap::new();
    for line in lines.filter(|line| !line.is_empty()) {
        if let Some((name, value)) = line.split_once(':') {
            parsed_headers.insert(name.to_ascii_lowercase(), value.trim().to_owned());
        }
    }
    let content_length = parsed_headers
        .get("content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    while bytes.len() < header_end + content_length {
        let mut chunk = [0_u8; 1024];
        let read = stream.read(&mut chunk).await.unwrap();
        assert_ne!(read, 0, "client closed before writing its full body");
        bytes.extend_from_slice(&chunk[..read]);
    }
    CapturedRequest {
        method,
        target,
        headers: parsed_headers,
        body: String::from_utf8(bytes[header_end..header_end + content_length].to_vec()).unwrap(),
    }
}

fn form_fields(body: &str) -> HashMap<String, String> {
    Url::parse(&format!("http://localhost/?{body}"))
        .unwrap()
        .query_pairs()
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect()
}

fn test_client(token_url: String) -> (CodexOAuthClient, OAuthEndpoints) {
    let endpoints = OAuthEndpoints::new(AUTHORIZE_URL, token_url, REDIRECT_URI);
    let client = CodexOAuthClient::with_client_and_endpoints(
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap(),
        endpoints.clone(),
    );
    (client, endpoints)
}

#[tokio::test]
async fn authorization_request_uses_reference_constants_and_pkce() {
    let request = AuthorizationRequest::new().unwrap();
    let url = Url::parse(request.authorization_url()).unwrap();
    let params: HashMap<String, String> = url
        .query_pairs()
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();

    assert_eq!(url.scheme(), "https");
    assert_eq!(url.host_str(), Some("auth.openai.com"));
    assert_eq!(url.path(), "/oauth/authorize");
    assert_eq!(
        params.get("response_type").map(String::as_str),
        Some("code")
    );
    assert_eq!(params.get("client_id").map(String::as_str), Some(CLIENT_ID));
    assert_eq!(
        params.get("redirect_uri").map(String::as_str),
        Some(REDIRECT_URI)
    );
    assert_eq!(params.get("scope").map(String::as_str), Some(SCOPE));
    assert_eq!(
        params.get("code_challenge_method").map(String::as_str),
        Some("S256")
    );
    assert_eq!(
        params.get("id_token_add_organizations").map(String::as_str),
        Some("true")
    );
    assert_eq!(
        params.get("codex_cli_simplified_flow").map(String::as_str),
        Some("true")
    );
    assert_eq!(params.get("originator").map(String::as_str), Some("ice"));

    let verifier_bytes = URL_SAFE_NO_PAD
        .decode(&request.code_verifier)
        .expect("PKCE verifier is base64url");
    assert_eq!(verifier_bytes.len(), 32);
    assert_eq!(params.get("state").unwrap().len(), 32);
    assert!(params
        .get("state")
        .unwrap()
        .bytes()
        .all(|byte| byte.is_ascii_hexdigit()));
    assert_eq!(
        params.get("code_challenge"),
        Some(&URL_SAFE_NO_PAD.encode(Sha256::digest(request.code_verifier.as_bytes())))
    );

    let debug = format!("{request:?}");
    assert!(!debug.contains(&request.code_verifier));
    assert!(!debug.contains(&request.state));
    assert!(!debug.contains(request.authorization_url()));
}

#[test]
fn manual_input_accepts_supported_shapes_and_checks_optional_state() {
    let request = AuthorizationRequest::new().unwrap();
    let state = request.state.clone();
    let cases = [
        (
            "https://localhost/auth/callback?code=full%2Bcode&state=".to_owned() + &state,
            "full+code",
        ),
        ("hash-code#".to_owned() + &state, "hash-code"),
        ("?code=query-code&state=".to_owned() + &state, "query-code"),
        ("bare-code".to_owned(), "bare-code"),
    ];
    for (input, expected) in cases {
        let code = request.parse_manual_input(&input).unwrap();
        assert_eq!(code.as_str(), expected);
        assert!(!format!("{code:?}").contains(expected));
    }
    assert!(matches!(
        request.parse_manual_input("code#wrong-state"),
        Err(OAuthError::StateMismatch)
    ));
    assert!(matches!(
        request.parse_manual_input("?error=access_denied"),
        Err(OAuthError::ProviderDenied)
    ));
    assert!(matches!(
        request.parse_manual_input(" "),
        Err(OAuthError::MissingAuthorizationCode)
    ));
}

#[test]
fn account_id_extraction_accepts_padded_or_unpadded_base64url() {
    let unpadded = jwt_with_account_id("acct-unpadded", false);
    let padded = jwt_with_account_id("acct-padded", true);
    assert_eq!(
        extract_account_id(&unpadded).unwrap().as_str(),
        "acct-unpadded"
    );
    assert_eq!(extract_account_id(&padded).unwrap().as_str(), "acct-padded");
    assert!(matches!(
        extract_account_id("not.a.jwt"),
        Err(OAuthError::InvalidCredential)
    ));
    assert!(matches!(
        extract_account_id(&jwt_with_account_id("", false)),
        Err(OAuthError::InvalidCredential)
    ));
}

#[test]
fn stored_credentials_validate_expiry_and_account_id_or_derive_it_from_access() {
    let access = jwt_with_account_id("stored-account", false);
    let derived =
        CodexCredential::from_stored(access.clone(), "refresh".to_owned(), 123, None).unwrap();
    assert_eq!(derived.account_id().as_str(), "stored-account");
    assert_eq!(derived.expires_at_epoch_seconds(), 123);

    let supplied = CodexCredential::from_stored(
        access.clone(),
        "refresh".to_owned(),
        123,
        Some("account-from-store".to_owned()),
    )
    .unwrap();
    assert_eq!(supplied.account_id().as_str(), "account-from-store");

    for invalid_account_id in ["", " \t", "account\r\nInjected: true"] {
        assert!(CodexCredential::from_stored(
            access.clone(),
            "refresh".to_owned(),
            123,
            Some(invalid_account_id.to_owned()),
        )
        .is_err());
    }
    assert!(CodexCredential::from_stored(access, "refresh".to_owned(), 0, None).is_err());
}

#[tokio::test]
async fn code_exchange_posts_reference_form_and_returns_validated_credentials() {
    let (token_url, server) =
        spawn_token_server(200, successful_token_response("account-123")).await;
    let (client, endpoints) = test_client(token_url);
    let request = AuthorizationRequest::new_with_endpoints(&endpoints).unwrap();
    let code = request.parse_manual_input("oauth-code").unwrap();
    let credential = client
        .exchange_code(&request, &code, 1_800_000_000, CancellationToken::new())
        .await
        .unwrap();

    assert_eq!(
        credential.access_token(),
        jwt_with_account_id("account-123", false)
    );
    assert_eq!(credential.refresh_token(), "refresh-secret");
    assert_eq!(credential.account_id().as_str(), "account-123");
    assert_eq!(credential.expires_at_epoch_seconds(), 1_800_003_600);

    let captured = server.await.unwrap();
    assert_eq!(captured.method, "POST");
    assert_eq!(captured.target, "/oauth/token");
    assert!(captured.headers["content-type"].starts_with("application/x-www-form-urlencoded"));
    let fields = form_fields(&captured.body);
    assert_eq!(
        fields.get("grant_type").map(String::as_str),
        Some("authorization_code")
    );
    assert_eq!(fields.get("client_id").map(String::as_str), Some(CLIENT_ID));
    assert_eq!(fields.get("code").map(String::as_str), Some("oauth-code"));
    assert_eq!(fields.get("code_verifier"), Some(&request.code_verifier));
    assert_eq!(
        fields.get("redirect_uri").map(String::as_str),
        Some(REDIRECT_URI)
    );

    let debug = format!("{credential:?}");
    assert!(!debug.contains(credential.access_token()));
    assert!(!debug.contains(credential.refresh_token()));
    assert!(!debug.contains(credential.account_id().as_str()));

    let stored = serde_json::to_value(&credential).unwrap();
    assert_eq!(stored["expires"], json!(1_800_003_600));
    assert_eq!(stored["accountId"], json!("account-123"));
    assert_eq!(
        serde_json::from_value::<CodexCredential>(stored)
            .unwrap()
            .account_id()
            .as_str(),
        "account-123"
    );
    assert!(serde_json::from_value::<AccountId>(json!("  ")).is_err());
}

#[tokio::test]
async fn refresh_posts_reference_form_and_rotates_tokens() {
    let (token_url, server) =
        spawn_token_server(200, successful_token_response("rotated-account")).await;
    let (client, _) = test_client(token_url);
    let credential = client
        .refresh_token(
            "old-refresh-secret",
            2_000_000_000,
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(credential.account_id().as_str(), "rotated-account");
    assert_eq!(credential.expires_at_epoch_seconds(), 2_000_003_600);

    let captured = server.await.unwrap();
    let fields = form_fields(&captured.body);
    assert_eq!(
        fields.get("grant_type").map(String::as_str),
        Some("refresh_token")
    );
    assert_eq!(fields.get("client_id").map(String::as_str), Some(CLIENT_ID));
    assert_eq!(
        fields.get("refresh_token").map(String::as_str),
        Some("old-refresh-secret")
    );
    assert!(!fields.contains_key("code"));
    assert!(!fields.contains_key("redirect_uri"));
}

#[test]
fn token_response_validation_rejects_missing_fields_and_expiry_overflow() {
    let malformed =
        serde_json::from_str::<TokenResponse>(r#"{"access_token":"access","expires_in":3600}"#);
    assert!(malformed.is_err());

    let response = TokenResponse {
        access_token: jwt_with_account_id("acct", false),
        refresh_token: "refresh".to_owned(),
        expires_in: 0,
    };
    assert!(matches!(
        credential_from_token_response(response, 1, TokenOperation::Exchange),
        Err(OAuthError::InvalidTokenResponse(TokenOperation::Exchange))
    ));

    let response = TokenResponse {
        access_token: jwt_with_account_id("acct", false),
        refresh_token: "refresh".to_owned(),
        expires_in: 10,
    };
    assert!(matches!(
        credential_from_token_response(response, i64::MAX - 1, TokenOperation::Refresh),
        Err(OAuthError::ExpiryOverflow)
    ));

    let response = TokenResponse {
        access_token: "access-without-jwt-account".to_owned(),
        refresh_token: "refresh".to_owned(),
        expires_in: 10,
    };
    assert!(matches!(
        credential_from_token_response(response, 1, TokenOperation::Refresh),
        Err(OAuthError::InvalidCredential)
    ));
}

#[tokio::test]
async fn token_http_errors_do_not_include_response_body_or_tokens() {
    let (token_url, server) = spawn_token_server(
        401,
        r#"{"error":"invalid_grant","access_token":"leaked-access-token"}"#.to_owned(),
    )
    .await;
    let (client, _) = test_client(token_url);
    let error = client
        .refresh_token("refresh-secret", 1, CancellationToken::new())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        OAuthError::TokenHttpStatus {
            operation: TokenOperation::Refresh,
            status: 401
        }
    ));
    let message = error.to_string();
    assert!(!message.contains("refresh-secret"));
    assert!(!message.contains("leaked-access-token"));
    let _ = server.await.unwrap();
}

async fn send_callback(address: SocketAddr, target: &str) -> String {
    let mut stream = TcpStream::connect(address).await.unwrap();
    let request = format!("GET {target} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();
    String::from_utf8(response).unwrap()
}

#[tokio::test]
async fn callback_rejects_other_paths_and_bad_state_then_accepts_valid_callback() {
    let request = AuthorizationRequest::new().unwrap();
    let expected_state = request.state.clone();
    let listener = CallbackListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let cancellation = CancellationToken::new();
    let wait = tokio::spawn(async move { listener.wait_for_code(&request, cancellation).await });

    let wrong_path = send_callback(address, "/elsewhere?code=secret&state=anything").await;
    assert!(wrong_path.starts_with("HTTP/1.1 404"));
    let wrong_state =
        send_callback(address, &format!("{CALLBACK_PATH}?code=secret&state=wrong")).await;
    assert!(wrong_state.starts_with("HTTP/1.1 400"));
    let missing_code =
        send_callback(address, &format!("{CALLBACK_PATH}?state={expected_state}")).await;
    assert!(missing_code.starts_with("HTTP/1.1 400"));

    let success = send_callback(
        address,
        &format!("{CALLBACK_PATH}?code=callback-secret&state={expected_state}"),
    )
    .await;
    assert!(success.starts_with("HTTP/1.1 200"));
    assert!(!success.contains("callback-secret"));
    let code = wait.await.unwrap().unwrap();
    assert_eq!(code.as_str(), "callback-secret");
    assert!(!format!("{code:?}").contains("callback-secret"));
}

#[tokio::test]
async fn callback_is_loopback_only_bounded_and_cancellable() {
    let non_loopback =
        CallbackListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0)).await;
    assert!(matches!(non_loopback, Err(error) if error.kind() == io::ErrorKind::InvalidInput));

    let (mut writer, mut reader) = tokio::io::duplex(MAX_CALLBACK_REQUEST_BYTES + 32);
    tokio::spawn(async move {
        let request = vec![b'x'; MAX_CALLBACK_REQUEST_BYTES + 8];
        let _ = writer.write_all(&request).await;
    });
    assert!(matches!(
        read_http_request(&mut reader).await,
        Err(CallbackReadError::TooLarge)
    ));

    let (_writer, mut reader) = tokio::io::duplex(16);
    assert!(
        timeout(Duration::from_millis(25), read_http_request(&mut reader))
            .await
            .is_err()
    );

    let request = AuthorizationRequest::new().unwrap();
    let listener = CallbackListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let cancellation = CancellationToken::new();
    let cancel_wait = cancellation.clone();
    let wait = tokio::spawn(async move {
        listener
            .wait_for_code_with_timeout(&request, cancel_wait, Duration::from_secs(5))
            .await
    });
    cancellation.cancel();
    assert!(matches!(wait.await.unwrap(), Err(OAuthError::Cancelled)));
    assert!(TcpListener::bind(address).await.is_ok());

    let timeout_request = AuthorizationRequest::new().unwrap();
    let timeout_listener =
        CallbackListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
            .await
            .unwrap();
    let timeout_address = timeout_listener.local_addr().unwrap();
    let timed_out = timeout_listener
        .wait_for_code_with_timeout(
            &timeout_request,
            CancellationToken::new(),
            Duration::from_millis(25),
        )
        .await;
    assert!(matches!(timed_out, Err(OAuthError::CallbackTimeout)));
    assert!(TcpListener::bind(timeout_address).await.is_ok());
}

#[tokio::test]
async fn login_flow_accepts_channel_manual_input_without_terminal_reads() {
    let (token_url, server) =
        spawn_token_server(200, successful_token_response("manual-account")).await;
    let (client, _) = test_client(token_url);
    let callback_address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
    let (url, flow) = LoginFlow::begin_at(client, callback_address).await.unwrap();
    assert!(flow.callback_available());
    assert!(url.starts_with(AUTHORIZE_URL));

    let (manual_tx, manual_rx) = mpsc::channel(1);
    let state = Url::parse(&url)
        .unwrap()
        .query_pairs()
        .find_map(|(key, value)| (key == "state").then(|| value.into_owned()))
        .unwrap();
    manual_tx
        .send(format!("?code=manual-login-code&state={state}"))
        .await
        .unwrap();
    let credential = flow
        .complete_at(manual_rx, CancellationToken::new(), 3_000)
        .await
        .unwrap();
    assert_eq!(credential.account_id().as_str(), "manual-account");
    assert_eq!(credential.expires_at_epoch_seconds(), 6_600);
    let captured = server.await.unwrap();
    assert_eq!(
        form_fields(&captured.body).get("code").map(String::as_str),
        Some("manual-login-code")
    );
}

#[tokio::test]
async fn login_flow_cancellation_stops_exchange_and_releases_callback_listener() {
    let token_listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let token_address = token_listener.local_addr().unwrap();
    let (request_seen_tx, request_seen_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut stream, _) = token_listener.accept().await.unwrap();
        let captured = read_captured_request(&mut stream).await;
        let _ = request_seen_tx.send(captured);
        std::future::pending::<()>().await;
    });

    let (client, _) = test_client(format!("http://{token_address}/oauth/token"));
    let probe = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let callback_address = probe.local_addr().unwrap();
    drop(probe);
    let (_url, flow) = LoginFlow::begin_at(client, callback_address).await.unwrap();
    let (manual_tx, manual_rx) = mpsc::channel(1);
    manual_tx
        .send("cancelled-exchange-code".to_owned())
        .await
        .unwrap();
    drop(manual_tx);

    let cancellation = CancellationToken::new();
    let completion = tokio::spawn(flow.complete(manual_rx, cancellation.clone()));
    let captured = request_seen_rx.await.unwrap();
    assert_eq!(
        form_fields(&captured.body).get("code").map(String::as_str),
        Some("cancelled-exchange-code")
    );

    // Manual input winning the race must drop the pending callback listener
    // before the token exchange finishes.
    let rebound_listener = TcpListener::bind(callback_address).await.unwrap();
    cancellation.cancel();
    assert!(matches!(
        timeout(Duration::from_secs(1), completion)
            .await
            .unwrap()
            .unwrap(),
        Err(OAuthError::Cancelled)
    ));

    drop(rebound_listener);
    server.abort();
}

#[tokio::test]
async fn login_flow_keeps_callback_after_manual_channel_eof_and_falls_back_when_port_is_busy() {
    let (token_url, server) =
        spawn_token_server(200, successful_token_response("callback-account")).await;
    let (client, _) = test_client(token_url);

    let probe = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let callback_address = probe.local_addr().unwrap();
    drop(probe);
    let (url, flow) = LoginFlow::begin_at(client, callback_address).await.unwrap();
    assert!(flow.callback_available());
    let state = Url::parse(&url)
        .unwrap()
        .query_pairs()
        .find_map(|(key, value)| (key == "state").then(|| value.into_owned()))
        .unwrap();
    let (manual_tx, manual_rx) = mpsc::channel(1);
    drop(manual_tx);
    let cancellation = CancellationToken::new();
    let complete = tokio::spawn(flow.complete_at(manual_rx, cancellation, 4_000));

    let callback_response = send_callback(
        callback_address,
        &format!("{CALLBACK_PATH}?code=callback-login-code&state={state}"),
    )
    .await;
    assert!(callback_response.starts_with("HTTP/1.1 200"));
    let credential = complete.await.unwrap().unwrap();
    assert_eq!(credential.account_id().as_str(), "callback-account");
    assert_eq!(credential.expires_at_epoch_seconds(), 7_600);
    let captured = server.await.unwrap();
    assert_eq!(
        form_fields(&captured.body).get("code").map(String::as_str),
        Some("callback-login-code")
    );

    let occupied = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let occupied_address = occupied.local_addr().unwrap();
    let (client, _) = test_client("http://127.0.0.1:9/oauth/token".to_owned());
    let (_url, fallback_flow) = LoginFlow::begin_at(client, occupied_address).await.unwrap();
    assert!(!fallback_flow.callback_available());
    let (manual_tx, manual_rx) = mpsc::channel(1);
    drop(manual_tx);
    assert!(matches!(
        fallback_flow
            .complete_at(manual_rx, CancellationToken::new(), 1)
            .await,
        Err(OAuthError::ManualInputUnavailable)
    ));
}
