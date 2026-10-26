//! Private protocol support for the ChatGPT subscription (Codex) OAuth flow.
//!
//! The CLI and TUI own prompting, browser launch, and input routing. This
//! module owns the protocol pieces they share: PKCE and authorization URL
//! construction, a bounded loopback callback, manual response parsing, token
//! exchange/refresh, and validated credentials.
//!
//! Reference: `~/Zks/ice/packages/ai/src/auth/oauth/openai-codex.ts` and
//! `pkce.ts`, from the local ice reference checkout at `3ab05193147f5abb8eb8ee6c0178f6dbce8918b0`.

use std::fmt;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD};
use base64::Engine;
use futures::StreamExt;
use getrandom::getrandom;
use reqwest::Url;
use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::time::{timeout, timeout_at, Instant};
use tokio_util::sync::CancellationToken;

const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";
const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const REDIRECT_URI: &str = "http://localhost:1455/auth/callback";
const SCOPE: &str = "openid profile email offline_access";
const JWT_AUTH_CLAIM: &str = "https://api.openai.com/auth";
const CALLBACK_PATH: &str = "/auth/callback";
const CALLBACK_PORT: u16 = 1455;
const CALLBACK_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const CALLBACK_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const CALLBACK_RESPONSE_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_CALLBACK_REQUEST_BYTES: usize = 16 * 1024;
const TOKEN_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_TOKEN_RESPONSE_BYTES: usize = 64 * 1024;

/// A validated account identifier extracted from the access-token JWT payload.
///
/// This only extracts metadata. It does not verify the JWT signature.
#[derive(Clone, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub(crate) struct AccountId(String);

impl AccountId {
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    fn parse(value: String) -> Result<Self, AccountIdParseError> {
        if value.trim().is_empty() || value.chars().any(char::is_control) {
            return Err(AccountIdParseError);
        }
        Ok(Self(value))
    }
}

impl fmt::Debug for AccountId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("AccountId").field(&"[REDACTED]").finish()
    }
}

impl<'de> Deserialize<'de> for AccountId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Error)]
#[error("account ID must be non-empty and contain no control characters")]
struct AccountIdParseError;

/// OAuth credential suitable for the CLI's private credential store.
///
/// Expiry is always Unix epoch seconds. Serde uses the existing OAuth field
/// names plus the additive `accountId` field. Secret values are redacted from
/// `Debug` output.
#[derive(Clone, Serialize)]
pub(crate) struct CodexCredential {
    #[serde(rename = "access")]
    access_token: String,
    #[serde(rename = "refresh")]
    refresh_token: String,
    #[serde(rename = "expires")]
    expires_at_epoch_seconds: i64,
    #[serde(rename = "accountId")]
    account_id: AccountId,
}

impl CodexCredential {
    /// Build a validated credential from the host's stored OAuth fields.
    /// Older imported entries may omit `accountId`; in that case it is
    /// derived from the access token's JWT payload.
    pub(crate) fn from_stored(
        access_token: String,
        refresh_token: String,
        expires_at_epoch_seconds: i64,
        account_id: Option<String>,
    ) -> Result<Self, OAuthError> {
        let account_id = match account_id {
            Some(value) => AccountId::parse(value).map_err(|_| OAuthError::InvalidCredential)?,
            None => extract_account_id(&access_token)?,
        };
        Self::validate(
            access_token,
            refresh_token,
            expires_at_epoch_seconds,
            account_id,
        )
    }

    pub(crate) fn access_token(&self) -> &str {
        &self.access_token
    }

    pub(crate) fn refresh_token(&self) -> &str {
        &self.refresh_token
    }

    pub(crate) fn expires_at_epoch_seconds(&self) -> i64 {
        self.expires_at_epoch_seconds
    }

    pub(crate) fn account_id(&self) -> &AccountId {
        &self.account_id
    }

    fn validate(
        access_token: String,
        refresh_token: String,
        expires_at_epoch_seconds: i64,
        account_id: AccountId,
    ) -> Result<Self, OAuthError> {
        if access_token.trim().is_empty()
            || refresh_token.trim().is_empty()
            || expires_at_epoch_seconds <= 0
        {
            return Err(OAuthError::InvalidCredential);
        }
        Ok(Self {
            access_token,
            refresh_token,
            expires_at_epoch_seconds,
            account_id,
        })
    }
}

impl fmt::Debug for CodexCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CodexCredential")
            .field("access_token", &"[REDACTED]")
            .field("refresh_token", &"[REDACTED]")
            .field("expires_at_epoch_seconds", &self.expires_at_epoch_seconds)
            .field("account_id", &"[REDACTED]")
            .finish()
    }
}

impl<'de> Deserialize<'de> for CodexCredential {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct StoredCredential {
            access: String,
            refresh: String,
            expires: i64,
            #[serde(rename = "accountId")]
            account_id: AccountId,
        }

        let value = StoredCredential::deserialize(deserializer)?;
        Self::validate(value.access, value.refresh, value.expires, value.account_id)
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TokenOperation {
    Exchange,
    Refresh,
}

impl fmt::Display for TokenOperation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exchange => f.write_str("exchange"),
            Self::Refresh => f.write_str("refresh"),
        }
    }
}

#[derive(Debug, Error)]
pub(crate) enum OAuthError {
    #[error("OpenAI Codex OAuth was cancelled")]
    Cancelled,
    #[error("OpenAI Codex OAuth callback timed out")]
    CallbackTimeout,
    #[error("OpenAI Codex OAuth callback request was invalid")]
    InvalidCallbackRequest,
    #[error("OpenAI Codex OAuth callback state did not match")]
    StateMismatch,
    #[error("OpenAI Codex authorization was denied")]
    ProviderDenied,
    #[error("OpenAI Codex authorization input did not contain a code")]
    MissingAuthorizationCode,
    #[error("no callback listener or manual OAuth input source is available")]
    ManualInputUnavailable,
    #[error("OpenAI Codex loopback callback listener could not be started")]
    CallbackBindFailed,
    #[error("OpenAI Codex OAuth endpoint configuration was invalid")]
    InvalidEndpoint,
    #[error("OpenAI Codex OAuth could not generate secure random values")]
    RandomnessUnavailable,
    #[error("OpenAI Codex OAuth HTTP client could not be initialized")]
    ClientInitialization,
    #[error("OpenAI Codex token {0} request failed")]
    TokenRequestFailed(TokenOperation),
    #[error("OpenAI Codex token {operation} failed with HTTP status {status}")]
    TokenHttpStatus {
        operation: TokenOperation,
        status: u16,
    },
    #[error("OpenAI Codex token {0} request timed out")]
    TokenRequestTimeout(TokenOperation),
    #[error("OpenAI Codex token {0} response was invalid")]
    InvalidTokenResponse(TokenOperation),
    #[error("OpenAI Codex credentials were invalid")]
    InvalidCredential,
    #[error("OpenAI Codex token expiry overflowed Unix epoch seconds")]
    ExpiryOverflow,
}

/// OAuth endpoints are injectable so tests can use a local fake server.
#[derive(Clone)]
pub(crate) struct OAuthEndpoints {
    authorize_url: String,
    token_url: String,
    redirect_uri: String,
}

impl OAuthEndpoints {
    pub(crate) fn new(
        authorize_url: impl Into<String>,
        token_url: impl Into<String>,
        redirect_uri: impl Into<String>,
    ) -> Self {
        Self {
            authorize_url: authorize_url.into(),
            token_url: token_url.into(),
            redirect_uri: redirect_uri.into(),
        }
    }
}

impl Default for OAuthEndpoints {
    fn default() -> Self {
        Self::new(AUTHORIZE_URL, TOKEN_URL, REDIRECT_URI)
    }
}

/// One browser authorization attempt. The CLI/TUI can display `authorization_url`
/// and separately race `CallbackListener::wait_for_code` against its own
/// cancellable manual-input channel.
#[derive(Clone)]
pub(crate) struct AuthorizationRequest {
    state: String,
    code_verifier: String,
    authorization_url: String,
    redirect_uri: String,
}

impl AuthorizationRequest {
    #[cfg(test)]
    pub(crate) fn new() -> Result<Self, OAuthError> {
        Self::new_with_endpoints(&OAuthEndpoints::default())
    }

    pub(crate) fn new_with_endpoints(endpoints: &OAuthEndpoints) -> Result<Self, OAuthError> {
        let verifier = create_code_verifier()?;
        let challenge = create_code_challenge(&verifier);
        let state = create_state()?;
        let mut url =
            Url::parse(&endpoints.authorize_url).map_err(|_| OAuthError::InvalidEndpoint)?;
        url.query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", CLIENT_ID)
            .append_pair("redirect_uri", &endpoints.redirect_uri)
            .append_pair("scope", SCOPE)
            .append_pair("code_challenge", &challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("state", &state)
            .append_pair("id_token_add_organizations", "true")
            .append_pair("codex_cli_simplified_flow", "true")
            .append_pair("originator", "ice");

        Ok(Self {
            state,
            code_verifier: verifier,
            authorization_url: url.to_string(),
            redirect_uri: endpoints.redirect_uri.clone(),
        })
    }

    pub(crate) fn authorization_url(&self) -> &str {
        &self.authorization_url
    }

    /// Parses a pasted URL, `code#state`, query string, or bare code.
    ///
    /// The provider allows pasting a bare code; if the pasted form includes a
    /// state value it must match this request. Automatic loopback callbacks
    /// always require a state value.
    pub(crate) fn parse_manual_input(&self, input: &str) -> Result<AuthorizationCode, OAuthError> {
        parse_manual_code(input, &self.state)
    }
}

/// Parses a pasted callback URL, `code#state`, query string, or bare code. A
/// state in the input must equal `expected_state`.
pub(crate) fn parse_manual_code(
    input: &str,
    expected_state: &str,
) -> Result<AuthorizationCode, OAuthError> {
    {
        let value = input.trim();
        if value.is_empty() {
            return Err(OAuthError::MissingAuthorizationCode);
        }

        let (code, supplied_state, provider_error) = if let Ok(url) = Url::parse(value) {
            query_authorization_values(url.query().unwrap_or_default())
        } else if let Some((code, state)) = value.split_once('#') {
            (Some(code.to_string()), Some(state.to_string()), None)
        } else if value.starts_with('?') || contains_query_code(value) {
            query_authorization_values(value.strip_prefix('?').unwrap_or(value))
        } else {
            (Some(value.to_string()), None, None)
        };

        if provider_error.is_some() {
            return Err(OAuthError::ProviderDenied);
        }
        if let Some(state) = supplied_state {
            if state != expected_state {
                return Err(OAuthError::StateMismatch);
            }
        }
        AuthorizationCode::new(code.ok_or(OAuthError::MissingAuthorizationCode)?)
    }
}

impl fmt::Debug for AuthorizationRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthorizationRequest")
            .field("state", &"[REDACTED]")
            .field("code_verifier", &"[REDACTED]")
            .field("authorization_url", &"[REDACTED]")
            .field("redirect_uri", &self.redirect_uri)
            .finish()
    }
}

fn query_authorization_values(query: &str) -> (Option<String>, Option<String>, Option<String>) {
    let Ok(url) = Url::parse(&format!("http://localhost/?{query}")) else {
        return (None, None, None);
    };
    let mut code = None;
    let mut state = None;
    let mut provider_error = None;
    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "code" => code = Some(value.into_owned()),
            "state" => state = Some(value.into_owned()),
            "error" => provider_error = Some(value.into_owned()),
            _ => {}
        }
    }
    (code, state, provider_error)
}

fn contains_query_code(value: &str) -> bool {
    value
        .split('&')
        .any(|part| part.split_once('=').is_some_and(|(key, _)| key == "code"))
}

/// Sensitive authorization code returned by either the callback or manual
/// input parser. Its `Debug` representation is always redacted.
pub(crate) struct AuthorizationCode(String);

impl AuthorizationCode {
    fn new(value: String) -> Result<Self, OAuthError> {
        if value.trim().is_empty() {
            return Err(OAuthError::MissingAuthorizationCode);
        }
        Ok(Self(value))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for AuthorizationCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("AuthorizationCode")
            .field(&"[REDACTED]")
            .finish()
    }
}

pub(crate) fn create_code_verifier() -> Result<String, OAuthError> {
    let mut random = [0_u8; 32];
    getrandom(&mut random).map_err(|_| OAuthError::RandomnessUnavailable)?;
    Ok(URL_SAFE_NO_PAD.encode(random))
}

pub(crate) fn create_code_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

pub(crate) fn create_state() -> Result<String, OAuthError> {
    let mut random = [0_u8; 16];
    getrandom(&mut random).map_err(|_| OAuthError::RandomnessUnavailable)?;
    Ok(random.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// The callback path and the pages the browser shows when it lands there.
#[derive(Clone)]
pub(crate) struct CallbackPages {
    pub path: std::borrow::Cow<'static, str>,
    pub success: &'static str,
    pub error: &'static str,
}
const CODEX_PAGES: CallbackPages = CallbackPages {
    path: std::borrow::Cow::Borrowed(CALLBACK_PATH),
    success: CALLBACK_SUCCESS_HTML,
    error: CALLBACK_ERROR_HTML,
};

/// Loopback listener for `http://localhost:1455/auth/callback`, or another
/// callback path (MCP OAuth uses its own pages).
pub(crate) struct CallbackListener {
    listener: TcpListener,
    ipv6_listener: Option<TcpListener>,
    pages: CallbackPages,
}

impl CallbackListener {
    /// Bind a caller-selected loopback address. Production callers should use
    /// loopback port 1455; the address form enables isolated tests.
    pub(crate) async fn bind(address: SocketAddr) -> io::Result<Self> {
        Self::bind_with(address, CODEX_PAGES).await
    }

    pub(crate) async fn bind_with(address: SocketAddr, pages: CallbackPages) -> io::Result<Self> {
        if !address.ip().is_loopback() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Codex OAuth callback must bind to loopback",
            ));
        }
        let listener = TcpListener::bind(address).await?;
        let ipv6_listener = if address.ip() == IpAddr::V4(Ipv4Addr::LOCALHOST) {
            // The registered localhost redirect can resolve to either family.
            // Keep IPv4 available on hosts without IPv6 support.
            TcpListener::bind((Ipv6Addr::LOCALHOST, listener.local_addr()?.port()))
                .await
                .ok()
        } else {
            None
        };
        Ok(Self {
            listener,
            ipv6_listener,
            pages,
        })
    }

    pub(crate) fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    async fn accept(&self) -> io::Result<(TcpStream, SocketAddr)> {
        match &self.ipv6_listener {
            Some(ipv6) => tokio::select! {
                result = self.listener.accept() => result,
                result = ipv6.accept() => result,
            },
            None => self.listener.accept().await,
        }
    }

    /// Wait for a valid callback. The future owns the listener, so dropping it
    /// when a manual-input branch wins also closes the socket.
    pub(crate) async fn wait_for_code(
        self,
        request: &AuthorizationRequest,
        cancellation: CancellationToken,
    ) -> Result<AuthorizationCode, OAuthError> {
        self.wait_for_state(&request.state, cancellation, CALLBACK_TIMEOUT)
            .await
    }

    /// Wait for a callback carrying `state`, for any caller's authorization request.
    pub(crate) async fn wait_for_state(
        self,
        expected_state: &str,
        cancellation: CancellationToken,
        max_wait: Duration,
    ) -> Result<AuthorizationCode, OAuthError> {
        let error_page = self.pages.error;
        let deadline = Instant::now() + max_wait;
        loop {
            let accepted = tokio::select! {
                _ = cancellation.cancelled() => return Err(OAuthError::Cancelled),
                result = timeout_at(deadline, self.accept()) => {
                    result.map_err(|_| OAuthError::CallbackTimeout)?
                        .map_err(|_| OAuthError::InvalidCallbackRequest)?
                }
            };
            let (mut stream, _) = accepted;
            let request_deadline = (Instant::now() + CALLBACK_REQUEST_TIMEOUT).min(deadline);
            let request_bytes = tokio::select! {
                _ = cancellation.cancelled() => return Err(OAuthError::Cancelled),
                result = timeout_at(request_deadline, read_http_request(&mut stream)) => {
                    match result {
                        Err(_) => {
                            let _ = write_callback_response(
                                &mut stream,
                                408,
                                "Request timed out",
                                error_page,
                            ).await;
                            continue;
                        }
                        Ok(Ok(bytes)) => bytes,
                        Ok(Err(CallbackReadError::TooLarge)) => {
                            let _ = write_callback_response(
                                &mut stream,
                                413,
                                "Request too large",
                                error_page,
                            ).await;
                            continue;
                        }
                        Ok(Err(CallbackReadError::Invalid)) => {
                            let _ = write_callback_response(
                                &mut stream,
                                400,
                                "Bad request",
                                error_page,
                            ).await;
                            continue;
                        }
                    }
                }
            };

            match parse_callback_request(&request_bytes, expected_state, &self.pages.path) {
                CallbackResult::Retry(status, reason) => {
                    let _ = write_callback_response(&mut stream, status, reason, error_page).await;
                }
                CallbackResult::Denied => {
                    let _ = write_callback_response(
                        &mut stream,
                        400,
                        "Authorization denied",
                        error_page,
                    )
                    .await;
                    return Err(OAuthError::ProviderDenied);
                }
                CallbackResult::Code(code) => {
                    let _ =
                        write_callback_response(&mut stream, 200, "OK", self.pages.success).await;
                    return Ok(code);
                }
            }
        }
    }
}

/// Browser/manual login orchestration shared by the CLI and TUI.
///
/// This type never reads stdin or owns a terminal prompt. The caller supplies
/// manual responses over an async channel; EOF disables that branch while a
/// listener can still complete the flow.
pub(crate) struct LoginFlow {
    request: AuthorizationRequest,
    listener: Option<CallbackListener>,
    client: CodexOAuthClient,
}

impl LoginFlow {
    /// Create the authorization request and bind the loopback callback before
    /// returning its URL. If port 1455 is already occupied, the flow remains
    /// usable through manual input.
    pub(crate) async fn begin() -> Result<(String, Self), OAuthError> {
        let client = CodexOAuthClient::new()?;
        Self::begin_with_address(
            client,
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), CALLBACK_PORT),
        )
        .await
    }

    /// Begin with an injected Codex client and loopback address. This is the
    /// test seam for fake token endpoints and ephemeral callback ports.
    #[cfg(test)]
    pub(crate) async fn begin_at(
        client: CodexOAuthClient,
        callback_address: SocketAddr,
    ) -> Result<(String, Self), OAuthError> {
        Self::begin_with_address(client, callback_address).await
    }

    async fn begin_with_address(
        client: CodexOAuthClient,
        callback_address: SocketAddr,
    ) -> Result<(String, Self), OAuthError> {
        let request = AuthorizationRequest::new_with_endpoints(&client.endpoints)?;
        let listener = match CallbackListener::bind(callback_address).await {
            Ok(listener) => Some(listener),
            Err(error) if error.kind() == io::ErrorKind::AddrInUse => None,
            Err(_) => return Err(OAuthError::CallbackBindFailed),
        };
        let authorization_url = request.authorization_url().to_owned();
        Ok((
            authorization_url,
            Self {
                request,
                listener,
                client,
            },
        ))
    }

    pub(crate) fn callback_available(&self) -> bool {
        self.listener.is_some()
    }

    /// Complete browser or manual login, then exchange the code for validated
    /// credentials. `manual_responses` must be supplied by the CLI/TUI; no
    /// blocking terminal input is hidden here.
    pub(crate) async fn complete(
        self,
        manual_responses: mpsc::Receiver<String>,
        cancellation: CancellationToken,
    ) -> Result<CodexCredential, OAuthError> {
        self.complete_with_epoch(manual_responses, cancellation, None)
            .await
    }

    #[cfg(test)]
    async fn complete_at(
        self,
        manual_responses: mpsc::Receiver<String>,
        cancellation: CancellationToken,
        now_epoch_seconds: i64,
    ) -> Result<CodexCredential, OAuthError> {
        self.complete_with_epoch(manual_responses, cancellation, Some(now_epoch_seconds))
            .await
    }

    async fn complete_with_epoch(
        self,
        mut manual_responses: mpsc::Receiver<String>,
        cancellation: CancellationToken,
        injected_now_epoch_seconds: Option<i64>,
    ) -> Result<CodexCredential, OAuthError> {
        let code = if let Some(listener) = self.listener {
            let callback = listener.wait_for_code(&self.request, cancellation.clone());
            tokio::pin!(callback);
            let mut manual_input_open = true;
            loop {
                tokio::select! {
                    _ = cancellation.cancelled() => return Err(OAuthError::Cancelled),
                    result = &mut callback => break result?,
                    input = manual_responses.recv(), if manual_input_open => {
                        match input {
                            Some(input) => break self.request.parse_manual_input(&input)?,
                            None => manual_input_open = false,
                        }
                    }
                }
            }
        } else {
            loop {
                tokio::select! {
                    _ = cancellation.cancelled() => return Err(OAuthError::Cancelled),
                    input = manual_responses.recv() => match input {
                        Some(input) => break self.request.parse_manual_input(&input)?,
                        None => return Err(OAuthError::ManualInputUnavailable),
                    }
                }
            }
        };

        let now_epoch_seconds = match injected_now_epoch_seconds {
            Some(now) => now,
            None => current_epoch_seconds()?,
        };
        self.client
            .exchange_code(&self.request, &code, now_epoch_seconds, cancellation)
            .await
    }
}

fn current_epoch_seconds() -> Result<i64, OAuthError> {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| OAuthError::InvalidCredential)?
        .as_secs();
    i64::try_from(seconds).map_err(|_| OAuthError::ExpiryOverflow)
}

const CALLBACK_SUCCESS_HTML: &str = "<!doctype html><html><head><meta charset=\"utf-8\"><title>OpenAI authentication complete</title></head><body><p>OpenAI authentication completed. You can close this window.</p></body></html>";
const CALLBACK_ERROR_HTML: &str = "<!doctype html><html><head><meta charset=\"utf-8\"><title>OpenAI authentication failed</title></head><body><p>OpenAI authentication could not be completed. Return to the terminal and try again.</p></body></html>";

#[derive(Debug)]
enum CallbackReadError {
    TooLarge,
    Invalid,
}

enum CallbackResult {
    Retry(u16, &'static str),
    Denied,
    Code(AuthorizationCode),
}

async fn read_http_request<R>(reader: &mut R) -> Result<Vec<u8>, CallbackReadError>
where
    R: AsyncRead + Unpin,
{
    let mut request = Vec::with_capacity(1024);
    let mut chunk = [0_u8; 1024];
    loop {
        if request.len() >= MAX_CALLBACK_REQUEST_BYTES {
            return Err(CallbackReadError::TooLarge);
        }
        let max_read = chunk
            .len()
            .min(MAX_CALLBACK_REQUEST_BYTES - request.len() + 1);
        let read = reader
            .read(&mut chunk[..max_read])
            .await
            .map_err(|_| CallbackReadError::Invalid)?;
        if read == 0 {
            return Err(CallbackReadError::Invalid);
        }
        request.extend_from_slice(&chunk[..read]);
        if request.len() > MAX_CALLBACK_REQUEST_BYTES {
            return Err(CallbackReadError::TooLarge);
        }
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            return Ok(request);
        }
    }
}

fn parse_callback_request(request: &[u8], expected_state: &str, path: &str) -> CallbackResult {
    let Some(line_end) = request.windows(2).position(|window| window == b"\r\n") else {
        return CallbackResult::Retry(400, "Bad request");
    };
    let Ok(request_line) = std::str::from_utf8(&request[..line_end]) else {
        return CallbackResult::Retry(400, "Bad request");
    };
    let mut fields = request_line.split_ascii_whitespace();
    let (Some(method), Some(target), Some(_version)) =
        (fields.next(), fields.next(), fields.next())
    else {
        return CallbackResult::Retry(400, "Bad request");
    };
    if fields.next().is_some() {
        return CallbackResult::Retry(400, "Bad request");
    }
    if method != "GET" {
        return CallbackResult::Retry(405, "Method not allowed");
    }
    if !target.starts_with('/') {
        return CallbackResult::Retry(400, "Bad request");
    }
    let Ok(url) = Url::parse(&format!("http://localhost{target}")) else {
        return CallbackResult::Retry(400, "Bad request");
    };
    if url.path() != path {
        return CallbackResult::Retry(404, "Not found");
    }

    let mut state = None;
    let mut code = None;
    let mut provider_error = false;
    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "state" => state = Some(value.into_owned()),
            "code" => code = Some(value.into_owned()),
            "error" => provider_error = true,
            _ => {}
        }
    }
    if state.as_deref() != Some(expected_state) {
        return CallbackResult::Retry(400, "State mismatch");
    }
    if provider_error {
        return CallbackResult::Denied;
    }
    let Some(code) = code else {
        return CallbackResult::Retry(400, "Missing authorization code");
    };
    match AuthorizationCode::new(code) {
        Ok(code) => CallbackResult::Code(code),
        Err(_) => CallbackResult::Retry(400, "Missing authorization code"),
    }
}

async fn write_callback_response(
    stream: &mut TcpStream,
    status: u16,
    reason: &str,
    body: &str,
) -> io::Result<()> {
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n{body}",
        body.len()
    );
    let _ = timeout(
        CALLBACK_RESPONSE_TIMEOUT,
        stream.write_all(response.as_bytes()),
    )
    .await;
    Ok(())
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: String,
    expires_in: u64,
}

impl fmt::Debug for TokenResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TokenResponse")
            .field("access_token", &"[REDACTED]")
            .field("refresh_token", &"[REDACTED]")
            .field("expires_in", &self.expires_in)
            .finish()
    }
}

struct TokenRequestForm<'a> {
    grant_type: &'a str,
    client_id: &'a str,
    code: Option<&'a str>,
    code_verifier: Option<&'a str>,
    redirect_uri: Option<&'a str>,
    refresh_token: Option<&'a str>,
}

impl TokenRequestForm<'_> {
    fn as_form_pairs(&self) -> Vec<(&'static str, &str)> {
        let mut pairs = vec![
            ("grant_type", self.grant_type),
            ("client_id", self.client_id),
        ];
        if let Some(value) = self.code {
            pairs.push(("code", value));
        }
        if let Some(value) = self.code_verifier {
            pairs.push(("code_verifier", value));
        }
        if let Some(value) = self.redirect_uri {
            pairs.push(("redirect_uri", value));
        }
        if let Some(value) = self.refresh_token {
            pairs.push(("refresh_token", value));
        }
        pairs
    }
}

/// Concrete HTTP client for Codex token exchange and refresh.
pub(crate) struct CodexOAuthClient {
    http: reqwest::Client,
    endpoints: OAuthEndpoints,
}

impl CodexOAuthClient {
    pub(crate) fn new() -> Result<Self, OAuthError> {
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| OAuthError::ClientInitialization)?;
        Ok(Self::with_client_and_endpoints(
            http,
            OAuthEndpoints::default(),
        ))
    }

    pub(crate) fn with_client_and_endpoints(
        http: reqwest::Client,
        endpoints: OAuthEndpoints,
    ) -> Self {
        Self { http, endpoints }
    }

    pub(crate) async fn exchange_code(
        &self,
        request: &AuthorizationRequest,
        code: &AuthorizationCode,
        now_epoch_seconds: i64,
        cancellation: CancellationToken,
    ) -> Result<CodexCredential, OAuthError> {
        let form = TokenRequestForm {
            grant_type: "authorization_code",
            client_id: CLIENT_ID,
            code: Some(code.as_str()),
            code_verifier: Some(&request.code_verifier),
            redirect_uri: Some(&request.redirect_uri),
            refresh_token: None,
        };
        self.send_token_form(
            form.as_form_pairs(),
            TokenOperation::Exchange,
            now_epoch_seconds,
            cancellation,
        )
        .await
    }

    pub(crate) async fn refresh_token(
        &self,
        refresh_token: &str,
        now_epoch_seconds: i64,
        cancellation: CancellationToken,
    ) -> Result<CodexCredential, OAuthError> {
        if refresh_token.trim().is_empty() {
            return Err(OAuthError::InvalidCredential);
        }
        let form = TokenRequestForm {
            grant_type: "refresh_token",
            client_id: CLIENT_ID,
            code: None,
            code_verifier: None,
            redirect_uri: None,
            refresh_token: Some(refresh_token),
        };
        self.send_token_form(
            form.as_form_pairs(),
            TokenOperation::Refresh,
            now_epoch_seconds,
            cancellation,
        )
        .await
    }

    async fn send_token_form(
        &self,
        form: Vec<(&'static str, &str)>,
        operation: TokenOperation,
        now_epoch_seconds: i64,
        cancellation: CancellationToken,
    ) -> Result<CodexCredential, OAuthError> {
        let endpoint =
            Url::parse(&self.endpoints.token_url).map_err(|_| OAuthError::InvalidEndpoint)?;
        let request = async {
            let response = self
                .http
                .post(endpoint)
                .form(&form)
                .send()
                .await
                .map_err(|_| OAuthError::TokenRequestFailed(operation))?;
            let status = response.status();
            if !status.is_success() {
                return Err(OAuthError::TokenHttpStatus {
                    operation,
                    status: status.as_u16(),
                });
            }

            let mut body = Vec::new();
            let mut stream = response.bytes_stream();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|_| OAuthError::InvalidTokenResponse(operation))?;
                if body.len().saturating_add(chunk.len()) > MAX_TOKEN_RESPONSE_BYTES {
                    return Err(OAuthError::InvalidTokenResponse(operation));
                }
                body.extend_from_slice(&chunk);
            }
            let response: TokenResponse = serde_json::from_slice(&body)
                .map_err(|_| OAuthError::InvalidTokenResponse(operation))?;
            credential_from_token_response(response, now_epoch_seconds, operation)
        };

        tokio::select! {
            _ = cancellation.cancelled() => Err(OAuthError::Cancelled),
            result = timeout(TOKEN_REQUEST_TIMEOUT, request) => {
                result.map_err(|_| OAuthError::TokenRequestTimeout(operation))?
            }
        }
    }
}

fn credential_from_token_response(
    response: TokenResponse,
    now_epoch_seconds: i64,
    operation: TokenOperation,
) -> Result<CodexCredential, OAuthError> {
    if response.access_token.trim().is_empty()
        || response.refresh_token.trim().is_empty()
        || response.expires_in == 0
    {
        return Err(OAuthError::InvalidTokenResponse(operation));
    }
    let expires_in = i64::try_from(response.expires_in)
        .map_err(|_| OAuthError::InvalidTokenResponse(operation))?;
    let expires_at_epoch_seconds = now_epoch_seconds
        .checked_add(expires_in)
        .ok_or(OAuthError::ExpiryOverflow)?;
    let account_id = extract_account_id(&response.access_token)?;
    CodexCredential::validate(
        response.access_token,
        response.refresh_token,
        expires_at_epoch_seconds,
        account_id,
    )
}

fn extract_account_id(access_token: &str) -> Result<AccountId, OAuthError> {
    let mut segments = access_token.split('.');
    let (Some(_header), Some(payload), Some(_signature), None) = (
        segments.next(),
        segments.next(),
        segments.next(),
        segments.next(),
    ) else {
        return Err(OAuthError::InvalidCredential);
    };
    let decoded = URL_SAFE_NO_PAD
        .decode(payload)
        .or_else(|_| URL_SAFE.decode(payload))
        .map_err(|_| OAuthError::InvalidCredential)?;

    let claims: serde_json::Value =
        serde_json::from_slice(&decoded).map_err(|_| OAuthError::InvalidCredential)?;
    let account_id = claims
        .get(JWT_AUTH_CLAIM)
        .and_then(|auth| auth.get("chatgpt_account_id"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or(OAuthError::InvalidCredential)?;
    AccountId::parse(account_id).map_err(|_| OAuthError::InvalidCredential)
}

#[cfg(test)]
#[path = "oauth_codex/tests.rs"]
mod tests;
