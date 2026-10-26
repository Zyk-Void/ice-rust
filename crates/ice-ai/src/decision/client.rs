//! One-shot HTTP transport for the typed decision codecs.
//!
//! A [`DecisionClient`] is bound to one endpoint, one wire protocol, and one
//! private API key. It never streams, never falls back to a chat API, and never
//! retries automatically: a decision POST is billable and has no documented
//! safe-replay contract. Redirects are disabled so the bearer credential is
//! only ever sent to the configured origin.

use std::collections::BTreeMap;
use std::fmt;
use std::time::Duration;

use thiserror::Error;
use tokio_util::sync::CancellationToken;

use super::{inception, openai, typesafe, DecisionCodecError, MAX_DECISION_RESPONSE_JSON_BYTES};

/// Default whole-request timeout for one decision call.
pub const DEFAULT_DECISION_TIMEOUT: Duration = Duration::from_secs(120);

/// A decision wire protocol. Shared HTTP transport does not make protocols
/// interchangeable; every request type is bound to exactly one of these.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DecisionProtocol {
    /// OpenAI `POST {base}/decisions` with an ordered question array.
    OpenAiDecisions,
    /// TypeSafe native System One `POST {base}/systemone` with a named map.
    TypeSafeSystemOne,
    /// Inception `POST {base}/decisions` with a named map.
    InceptionDecisions,
}

impl DecisionProtocol {
    pub const ALL: [Self; 3] = [
        Self::OpenAiDecisions,
        Self::TypeSafeSystemOne,
        Self::InceptionDecisions,
    ];

    /// Parse the configuration identifier (see [`Self::as_str`]).
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|protocol| protocol.as_str() == value)
    }

    /// Stable configuration identifier for this protocol.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OpenAiDecisions => "openai-decisions",
            Self::TypeSafeSystemOne => "typesafe-systemone",
            Self::InceptionDecisions => "inception-decisions",
        }
    }

    /// Operation path appended to the configured base URL (which includes the
    /// version segment, e.g. `https://api.typesafe.ai/v1`).
    pub const fn path(self) -> &'static str {
        match self {
            Self::OpenAiDecisions | Self::InceptionDecisions => "/decisions",
            Self::TypeSafeSystemOne => "/systemone",
        }
    }
}

impl fmt::Display for DecisionProtocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

mod sealed {
    pub trait Sealed {}
    impl Sealed for super::openai::Request {}
    impl Sealed for super::typesafe::Request {}
    impl Sealed for super::inception::Request {}
}

/// A typed decision request that knows its protocol and how to validate its
/// own response. Sealed: only the protocol modules in this crate implement it.
pub trait DecisionRequest: sealed::Sealed {
    type Response;
    const PROTOCOL: DecisionProtocol;
    fn encode(&self) -> Result<Vec<u8>, DecisionCodecError>;
    fn decode(&self, bytes: &[u8]) -> Result<Self::Response, DecisionCodecError>;
}

impl DecisionRequest for openai::Request {
    type Response = openai::Response;
    const PROTOCOL: DecisionProtocol = DecisionProtocol::OpenAiDecisions;
    fn encode(&self) -> Result<Vec<u8>, DecisionCodecError> {
        self.encode_json()
    }
    fn decode(&self, bytes: &[u8]) -> Result<Self::Response, DecisionCodecError> {
        openai::Response::decode_json(bytes, self)
    }
}

impl DecisionRequest for typesafe::Request {
    type Response = typesafe::Response;
    const PROTOCOL: DecisionProtocol = DecisionProtocol::TypeSafeSystemOne;
    fn encode(&self) -> Result<Vec<u8>, DecisionCodecError> {
        self.encode_json()
    }
    fn decode(&self, bytes: &[u8]) -> Result<Self::Response, DecisionCodecError> {
        typesafe::Response::decode_json(bytes, self)
    }
}

impl DecisionRequest for inception::Request {
    type Response = inception::Response;
    const PROTOCOL: DecisionProtocol = DecisionProtocol::InceptionDecisions;
    fn encode(&self) -> Result<Vec<u8>, DecisionCodecError> {
        self.encode_json()
    }
    fn decode(&self, bytes: &[u8]) -> Result<Self::Response, DecisionCodecError> {
        inception::Response::decode_json(bytes, self)
    }
}

/// Errors from a decision call. Messages never contain the API key, request
/// content, or response body.
#[derive(Debug, Error)]
pub enum DecisionError {
    #[error("invalid decision endpoint: {0}")]
    InvalidEndpoint(&'static str),
    #[error("decision request protocol {request} does not match endpoint protocol {endpoint}")]
    ProtocolMismatch {
        request: DecisionProtocol,
        endpoint: DecisionProtocol,
    },
    #[error(transparent)]
    Codec(#[from] DecisionCodecError),
    #[error("decision provider rejected the credentials (HTTP {status})")]
    Auth { status: u16 },
    #[error("decision provider returned HTTP {status}")]
    Status { status: u16 },
    #[error("decision transport failed: {0}")]
    Transport(String),
    #[error("decision request timed out")]
    Timeout,
    #[error("decision request was cancelled")]
    Cancelled,
}

/// Explicit endpoint binding. Debug output redacts the key and headers.
#[derive(Clone)]
pub struct DecisionEndpoint {
    pub protocol: DecisionProtocol,
    /// Base URL including the version segment, without the operation path.
    pub base_url: String,
    pub api_key: String,
    /// Extra request headers (e.g. organization IDs). `authorization` is
    /// always the bearer key and cannot be overridden here.
    pub headers: BTreeMap<String, String>,
}

impl fmt::Debug for DecisionEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DecisionEndpoint")
            .field("protocol", &self.protocol)
            .field("base_url", &self.base_url)
            .field("api_key", &"<redacted>")
            .field("headers", &self.headers.keys().collect::<Vec<_>>())
            .finish()
    }
}

/// A reusable client for one decision endpoint.
#[derive(Clone)]
pub struct DecisionClient {
    endpoint: DecisionEndpoint,
    url: String,
    http: reqwest::Client,
}

impl fmt::Debug for DecisionClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DecisionClient")
            .field("endpoint", &self.endpoint)
            .finish()
    }
}

impl DecisionClient {
    /// Validate the endpoint and build a pooled HTTP client honoring the
    /// system proxy and idle-timeout policy of [`crate::http`].
    pub fn new(
        endpoint: DecisionEndpoint,
        idle_timeout_ms: Option<i64>,
    ) -> Result<Self, DecisionError> {
        if endpoint.api_key.trim().is_empty() {
            return Err(DecisionError::InvalidEndpoint("API key must not be empty"));
        }
        let base = endpoint.base_url.trim().trim_end_matches('/');
        let parsed = reqwest::Url::parse(base)
            .map_err(|_| DecisionError::InvalidEndpoint("base URL is not a valid URL"))?;
        if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
            return Err(DecisionError::InvalidEndpoint(
                "base URL must be an http(s) URL with a host",
            ));
        }
        if !parsed.username().is_empty() || parsed.password().is_some() {
            return Err(DecisionError::InvalidEndpoint(
                "base URL must not embed credentials",
            ));
        }
        if parsed.query().is_some() || parsed.fragment().is_some() {
            return Err(DecisionError::InvalidEndpoint(
                "base URL must not have a query or fragment",
            ));
        }
        let url = format!("{base}{}", endpoint.protocol.path());
        let http = crate::http::client_builder_for(&url, idle_timeout_ms, None)
            .map_err(|_| DecisionError::InvalidEndpoint("proxy configuration is invalid"))?
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| DecisionError::Transport(error.without_url().to_string()))?;
        Ok(Self {
            endpoint,
            url,
            http,
        })
    }

    pub fn protocol(&self) -> DecisionProtocol {
        self.endpoint.protocol
    }

    /// Full operation URL this client posts to.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Send one decision request and return its validated typed response.
    ///
    /// Protocol mismatches and invalid requests fail before any network I/O.
    /// `timeout` bounds the whole exchange, including the response body.
    pub async fn decide<R: DecisionRequest>(
        &self,
        request: &R,
        timeout: Duration,
        cancel: &CancellationToken,
    ) -> Result<R::Response, DecisionError> {
        if R::PROTOCOL != self.endpoint.protocol {
            return Err(DecisionError::ProtocolMismatch {
                request: R::PROTOCOL,
                endpoint: self.endpoint.protocol,
            });
        }
        let body = request.encode()?;
        if cancel.is_cancelled() {
            return Err(DecisionError::Cancelled);
        }

        let mut builder = self.http.post(&self.url).timeout(timeout);
        for (name, value) in &self.endpoint.headers {
            if !name.eq_ignore_ascii_case("authorization") {
                builder = builder.header(name, value);
            }
        }
        if !self
            .endpoint
            .headers
            .keys()
            .any(|name| name.eq_ignore_ascii_case("user-agent"))
        {
            builder = builder.header("user-agent", crate::http::user_agent());
        }
        let builder = builder
            .header("content-type", "application/json")
            .header("accept", "application/json")
            .bearer_auth(&self.endpoint.api_key)
            .body(body);

        let bytes = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(DecisionError::Cancelled),
            result = send_and_read(builder) => result?,
        };
        Ok(request.decode(&bytes)?)
    }
}

async fn send_and_read(builder: reqwest::RequestBuilder) -> Result<Vec<u8>, DecisionError> {
    let mut response = builder.send().await.map_err(transport_error)?;
    let status = response.status().as_u16();
    match status {
        200..=299 => {}
        401 | 403 => return Err(DecisionError::Auth { status }),
        _ => return Err(DecisionError::Status { status }),
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_DECISION_RESPONSE_JSON_BYTES as u64)
    {
        return Err(DecisionCodecError::ResponseBodyTooLarge.into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(transport_error)? {
        if bytes.len() + chunk.len() > MAX_DECISION_RESPONSE_JSON_BYTES {
            return Err(DecisionCodecError::ResponseBodyTooLarge.into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn transport_error(error: reqwest::Error) -> DecisionError {
    if error.is_timeout() {
        DecisionError::Timeout
    } else {
        DecisionError::Transport(error.without_url().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decision::NativeContent;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn typesafe_request() -> typesafe::Request {
        typesafe::Request::new(
            "jev-latest",
            NativeContent::text("Please refund the duplicate charge."),
            BTreeMap::from([(
                "billing".into(),
                typesafe::Question::Noul {
                    instructions: Some(NativeContent::text("Is this about billing?")),
                    criteria: None,
                },
            )]),
        )
    }

    fn endpoint(protocol: DecisionProtocol, base_url: String) -> DecisionEndpoint {
        DecisionEndpoint {
            protocol,
            base_url,
            api_key: "sk-test-secret".into(),
            headers: BTreeMap::from([
                ("x-org".into(), "org-1".into()),
                ("Authorization".into(), "Bearer hijack".into()),
            ]),
        }
    }

    /// Serve one canned HTTP response and return the raw request it received.
    async fn serve_once(response: Vec<u8>) -> (String, tokio::task::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buffer = [0u8; 4096];
            loop {
                let read = socket.read(&mut buffer).await.unwrap();
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);
                let text = String::from_utf8_lossy(&request);
                if let Some(header_end) = text.find("\r\n\r\n") {
                    let length = text[..header_end]
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|value| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if request.len() >= header_end + 4 + length {
                        break;
                    }
                }
            }
            socket.write_all(&response).await.unwrap();
            socket.shutdown().await.ok();
            String::from_utf8(request).unwrap()
        });
        (format!("http://{address}/v1/"), server)
    }

    fn http_response(status: &str, body: &str) -> Vec<u8> {
        format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .into_bytes()
    }

    #[tokio::test]
    async fn posts_exact_endpoint_body_and_bearer_auth_then_validates_response() {
        let body = r#"{"model":"jev-1.14.0","answers":{"billing":{"type":"noul","noul":0.98}},"usage":{"input_tokens":12,"output_tokens":1}}"#;
        let (base, server) = serve_once(http_response("200 OK", body)).await;
        let client =
            DecisionClient::new(endpoint(DecisionProtocol::TypeSafeSystemOne, base), None).unwrap();
        let request = typesafe_request();
        let response = client
            .decide(
                &request,
                DEFAULT_DECISION_TIMEOUT,
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        assert!(matches!(
            response.answers()["billing"],
            typesafe::Answer::Noul { noul } if noul.get() == 0.98
        ));

        let raw = server.await.unwrap();
        let lower = raw.to_ascii_lowercase();
        assert!(raw.starts_with("POST /v1/systemone HTTP/1.1\r\n"), "{raw}");
        assert!(lower.contains("authorization: bearer sk-test-secret\r\n"));
        assert!(!lower.contains("hijack"));
        assert!(lower.contains("x-org: org-1\r\n"));
        assert!(lower.contains("content-type: application/json\r\n"));
        let sent = raw.split("\r\n\r\n").nth(1).unwrap();
        assert_eq!(sent.as_bytes(), request.encode_json().unwrap());
    }

    #[tokio::test]
    async fn invalid_requests_and_protocol_mismatch_fail_without_network() {
        // Nothing listens here; both calls must fail before connecting.
        let base = "http://127.0.0.1:9/v1".to_string();
        let client = DecisionClient::new(
            endpoint(DecisionProtocol::OpenAiDecisions, base.clone()),
            None,
        )
        .unwrap();
        let error = client
            .decide(
                &typesafe_request(),
                DEFAULT_DECISION_TIMEOUT,
                &CancellationToken::new(),
            )
            .await
            .unwrap_err();
        assert!(matches!(error, DecisionError::ProtocolMismatch { .. }));

        let client =
            DecisionClient::new(endpoint(DecisionProtocol::TypeSafeSystemOne, base), None).unwrap();
        let empty = typesafe::Request::new("jev", NativeContent::text("x"), BTreeMap::new());
        let error = client
            .decide(&empty, DEFAULT_DECISION_TIMEOUT, &CancellationToken::new())
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            DecisionError::Codec(DecisionCodecError::InvalidRequest(_))
        ));
    }

    #[tokio::test]
    async fn http_errors_are_typed_and_never_echo_body_or_key() {
        for (status, expect_auth) in [("401 Unauthorized", true), ("500 Server Error", false)] {
            let (base, server) =
                serve_once(http_response(status, r#"{"error":"state was: secret"}"#)).await;
            let client =
                DecisionClient::new(endpoint(DecisionProtocol::TypeSafeSystemOne, base), None)
                    .unwrap();
            let error = client
                .decide(
                    &typesafe_request(),
                    DEFAULT_DECISION_TIMEOUT,
                    &CancellationToken::new(),
                )
                .await
                .unwrap_err();
            assert_eq!(matches!(error, DecisionError::Auth { .. }), expect_auth);
            let message = error.to_string();
            assert!(!message.contains("secret"), "{message}");
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn invalid_response_and_redirect_are_rejected() {
        let (base, server) = serve_once(http_response(
            "200 OK",
            r#"{"model":"jev","answers":{},"usage":{"input_tokens":1,"output_tokens":1}}"#,
        ))
        .await;
        let client =
            DecisionClient::new(endpoint(DecisionProtocol::TypeSafeSystemOne, base), None).unwrap();
        let error = client
            .decide(
                &typesafe_request(),
                DEFAULT_DECISION_TIMEOUT,
                &CancellationToken::new(),
            )
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            DecisionError::Codec(DecisionCodecError::InvalidResponse(_))
        ));
        server.await.unwrap();

        let redirect = b"HTTP/1.1 307 Temporary Redirect\r\nLocation: http://attacker.test/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec();
        let (base, server) = serve_once(redirect).await;
        let client =
            DecisionClient::new(endpoint(DecisionProtocol::TypeSafeSystemOne, base), None).unwrap();
        let error = client
            .decide(
                &typesafe_request(),
                DEFAULT_DECISION_TIMEOUT,
                &CancellationToken::new(),
            )
            .await
            .unwrap_err();
        assert!(matches!(error, DecisionError::Status { status: 307 }));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn oversized_response_timeout_and_cancellation_fail_safely() {
        let oversized = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            MAX_DECISION_RESPONSE_JSON_BYTES + 1
        )
        .into_bytes();
        let (base, server) = serve_once(oversized).await;
        let client =
            DecisionClient::new(endpoint(DecisionProtocol::TypeSafeSystemOne, base), None).unwrap();
        let error = client
            .decide(
                &typesafe_request(),
                DEFAULT_DECISION_TIMEOUT,
                &CancellationToken::new(),
            )
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            DecisionError::Codec(DecisionCodecError::ResponseBodyTooLarge)
        ));
        server.await.unwrap();

        // A server that accepts but never answers.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/v1", listener.local_addr().unwrap());
        let hold = tokio::spawn(async move {
            let mut open = Vec::new();
            loop {
                open.push(listener.accept().await.unwrap().0);
            }
        });
        let client =
            DecisionClient::new(endpoint(DecisionProtocol::TypeSafeSystemOne, base), None).unwrap();
        let error = client
            .decide(
                &typesafe_request(),
                Duration::from_millis(50),
                &CancellationToken::new(),
            )
            .await
            .unwrap_err();
        assert!(matches!(error, DecisionError::Timeout), "{error}");

        let cancel = CancellationToken::new();
        let trigger = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            trigger.cancel();
        });
        let error = client
            .decide(&typesafe_request(), DEFAULT_DECISION_TIMEOUT, &cancel)
            .await
            .unwrap_err();
        assert!(matches!(error, DecisionError::Cancelled));
        hold.abort();
    }

    #[test]
    fn endpoint_validation_and_debug_redaction() {
        let bad = |base: &str| {
            DecisionClient::new(
                endpoint(DecisionProtocol::TypeSafeSystemOne, base.into()),
                None,
            )
            .unwrap_err()
        };
        assert!(matches!(
            bad("ftp://x.test/v1"),
            DecisionError::InvalidEndpoint(_)
        ));
        assert!(matches!(
            bad("https://user:pw@x.test/v1"),
            DecisionError::InvalidEndpoint(_)
        ));
        assert!(matches!(
            bad("https://x.test/v1?key=1"),
            DecisionError::InvalidEndpoint(_)
        ));

        let client = DecisionClient::new(
            endpoint(
                DecisionProtocol::TypeSafeSystemOne,
                "https://api.typesafe.ai/v1/".into(),
            ),
            None,
        )
        .unwrap();
        assert_eq!(client.url(), "https://api.typesafe.ai/v1/systemone");
        let debug = format!("{client:?}");
        assert!(!debug.contains("sk-test-secret"), "{debug}");
        assert!(!debug.contains("org-1"), "{debug}");

        for protocol in DecisionProtocol::ALL {
            assert_eq!(DecisionProtocol::parse(protocol.as_str()), Some(protocol));
        }
        assert_eq!(DecisionProtocol::parse("openai-completions"), None);
    }
}
