//! The bounded Cognee HTTP client. Ports `ice-cognee-client.ts`
//! (COG-009..020).
//!
//! Every request runs under one deadline that covers headers, body reads and
//! decoding, and a parent cancellation token. Response bodies are streamed
//! against a byte cap. The transport is a trait so tests can inject
//! responses without a network, as the TypeScript client injects `fetch`.

use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::{Stream, StreamExt};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::blackhole::js::{len16, prefix16};
use crate::blackhole::regex;

pub const MAX_RECALL_RESPONSE_BYTES: usize = 128 * 1024;
pub const MAX_SETTINGS_RESPONSE_BYTES: usize = 64 * 1024;
pub const MAX_ERROR_RESPONSE_BYTES: usize = 4 * 1024;
const RECALL_PREREQUISITES_ERROR: &str = "Recall prerequisites not met";
const MAX_GRAPH_NODE_CHARS: usize = 400;

/// `CogneeErrorKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CogneeErrorKind {
    AuthFailed,
    ServerError,
    Unreachable,
    Timeout,
    Aborted,
    NotFound,
    Malformed,
    /// The caller passed an argument that was rejected before any request.
    InvalidRequest,
    ResponseTooLarge,
}

impl CogneeErrorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AuthFailed => "auth_failed",
            Self::ServerError => "server_error",
            Self::Unreachable => "unreachable",
            Self::Timeout => "timeout",
            Self::Aborted => "aborted",
            Self::NotFound => "not_found",
            Self::Malformed => "malformed",
            Self::InvalidRequest => "invalid_request",
            Self::ResponseTooLarge => "response_too_large",
        }
    }
}

/// `CogneeError`: a kind, a message without server bodies or keys, and the
/// HTTP status when one was received.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CogneeError {
    pub kind: CogneeErrorKind,
    pub message: String,
    pub status: Option<u16>,
}

impl CogneeError {
    pub fn new(kind: CogneeErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            status: None,
        }
    }
}

impl std::fmt::Display for CogneeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CogneeError {}

// ── transport ──

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
}

/// One multipart form field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormPart {
    pub name: String,
    pub value: String,
    pub filename: Option<String>,
    pub content_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Body {
    Empty,
    Json(String),
    /// The transport generates the boundary and content type.
    Multipart(Vec<FormPart>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    pub method: Method,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Body,
}

pub type BodyStream = Pin<Box<dyn Stream<Item = Result<Vec<u8>, String>> + Send>>;

pub struct HttpResponse {
    pub status: u16,
    /// Lowercase header names.
    pub headers: Vec<(String, String)>,
    pub body: BodyStream,
}

impl HttpResponse {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

#[async_trait::async_trait]
pub trait CogneeTransport: Send + Sync {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, String>;
}

/// The reqwest-backed transport.
pub struct ReqwestTransport {
    client: reqwest::Client,
}

impl ReqwestTransport {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }
}

impl Default for ReqwestTransport {
    fn default() -> Self {
        Self::new()
    }
}

fn multipart_body(parts: &[FormPart]) -> (String, Vec<u8>) {
    let boundary = format!("----icers-cognee-{}", uuid::Uuid::new_v4().simple());
    let mut body = Vec::new();
    for part in parts {
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        let quoted = |text: &str| text.replace('"', "%22").replace(['\r', '\n'], " ");
        match &part.filename {
            Some(filename) => body.extend_from_slice(
                format!(
                    "Content-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\n",
                    quoted(&part.name),
                    quoted(filename)
                )
                .as_bytes(),
            ),
            None => body.extend_from_slice(
                format!(
                    "Content-Disposition: form-data; name=\"{}\"\r\n",
                    quoted(&part.name)
                )
                .as_bytes(),
            ),
        }
        if let Some(content_type) = &part.content_type {
            body.extend_from_slice(format!("Content-Type: {content_type}\r\n").as_bytes());
        }
        body.extend_from_slice(b"\r\n");
        body.extend_from_slice(part.value.as_bytes());
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

#[async_trait::async_trait]
impl CogneeTransport for ReqwestTransport {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, String> {
        let mut builder = match request.method {
            Method::Get => self.client.get(&request.url),
            Method::Post => self.client.post(&request.url),
        };
        for (name, value) in &request.headers {
            builder = builder.header(name, value);
        }
        builder = match request.body {
            Body::Empty => builder,
            Body::Json(text) => builder.body(text),
            Body::Multipart(parts) => {
                let (content_type, body) = multipart_body(&parts);
                builder.header("content-type", content_type).body(body)
            }
        };
        let response = builder.send().await.map_err(|error| error.to_string())?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_ascii_lowercase(),
                    value.to_str().unwrap_or_default().to_string(),
                )
            })
            .collect();
        let body = response.bytes_stream().map(|chunk| {
            chunk
                .map(|bytes| bytes.to_vec())
                .map_err(|error| error.to_string())
        });
        Ok(HttpResponse {
            status,
            headers,
            body: Box::pin(body),
        })
    }
}

// ── request plumbing ──

/// One request deadline.
#[derive(Debug, Clone, Copy)]
pub struct Deadline {
    at: Instant,
}

impl Deadline {
    pub fn after_ms(budget_ms: u64) -> Self {
        Self {
            at: Instant::now() + Duration::from_millis(budget_ms.max(1)),
        }
    }

    pub fn expired(&self) -> bool {
        Instant::now() >= self.at
    }
}

pub fn build_url(base_url: &str, path: &str) -> Result<String, CogneeError> {
    let mut url = reqwest::Url::parse(base_url).map_err(|_| {
        CogneeError::new(CogneeErrorKind::InvalidRequest, "Invalid Cognee base URL")
    })?;
    let joined = format!(
        "{}{path}",
        url.path().strip_suffix('/').unwrap_or(url.path())
    );
    url.set_path(&joined);
    url.set_query(None);
    url.set_fragment(None);
    Ok(url.to_string())
}

pub(crate) fn headers(api_key: Option<&str>, content_type: Option<&str>) -> Vec<(String, String)> {
    let mut headers = vec![("accept".to_string(), "application/json".to_string())];
    if let Some(content_type) = content_type {
        headers.push(("content-type".into(), content_type.into()));
    }
    if let Some(key) = api_key.filter(|key| !key.is_empty()) {
        headers.push(("x-api-key".into(), key.into()));
    }
    headers
}

pub fn classify_status(status: u16) -> CogneeErrorKind {
    match status {
        401 | 403 => CogneeErrorKind::AuthFailed,
        404 => CogneeErrorKind::NotFound,
        408 | 429 => CogneeErrorKind::ServerError,
        status if status >= 500 => CogneeErrorKind::ServerError,
        _ => CogneeErrorKind::Malformed,
    }
}

pub(crate) fn assert_ok(response: &HttpResponse, action: &str) -> Result<(), CogneeError> {
    if response.ok() {
        return Ok(());
    }
    Err(CogneeError {
        kind: classify_status(response.status),
        message: format!("Cognee {action} failed with HTTP {}", response.status),
        status: Some(response.status),
    })
}

fn too_large() -> CogneeError {
    CogneeError::new(
        CogneeErrorKind::ResponseTooLarge,
        "Cognee response exceeded the configured limit",
    )
}

fn response_limit(response: &HttpResponse, max_bytes: usize) -> Result<Option<usize>, CogneeError> {
    let Some(header) = response.header("content-length") else {
        return Ok(None);
    };
    let length = header
        .parse::<u64>()
        .ok()
        .filter(|_| !header.is_empty() && header.bytes().all(|b| b.is_ascii_digit()))
        .filter(|length| *length <= 9_007_199_254_740_991)
        .ok_or_else(|| {
            CogneeError::new(
                CogneeErrorKind::Malformed,
                "Cognee returned an invalid Content-Length",
            )
        })?;
    if length > max_bytes as u64 {
        return Err(too_large());
    }
    Ok(Some(length as usize))
}

/// Decode bytes cut at a byte cap: invalid sequences become U+FFFD, but an
/// incomplete sequence at the cut is dropped instead.
pub fn decode_truncated(bytes: &[u8]) -> String {
    let mut out = String::new();
    let mut rest = bytes;
    loop {
        match std::str::from_utf8(rest) {
            Ok(valid) => {
                out.push_str(valid);
                return out;
            }
            Err(error) => {
                let valid = error.valid_up_to();
                out.push_str(std::str::from_utf8(&rest[..valid]).expect("valid prefix"));
                match error.error_len() {
                    None => return out,
                    Some(len) => {
                        out.push('\u{FFFD}');
                        rest = &rest[valid + len..];
                    }
                }
            }
        }
    }
}

/// Read at most `max_bytes`. With `truncate`, a longer body is cut and
/// reported; otherwise it fails as `response_too_large`. Truncating reads
/// skip the Content-Length check.
pub async fn read_text(
    response: &mut HttpResponse,
    max_bytes: usize,
    truncate: bool,
) -> Result<(String, bool), CogneeError> {
    let declared = if truncate {
        None
    } else {
        response_limit(response, max_bytes)?
    };
    let mut bytes: Vec<u8> = Vec::new();
    let mut truncated = false;
    while let Some(chunk) = response.body.next().await {
        let chunk = chunk.map_err(|_| {
            CogneeError::new(
                CogneeErrorKind::Unreachable,
                "Cognee service is unreachable",
            )
        })?;
        if bytes.len() + chunk.len() > max_bytes {
            if !truncate {
                return Err(too_large());
            }
            let remaining = max_bytes - bytes.len();
            bytes.extend_from_slice(&chunk[..remaining]);
            truncated = true;
            break;
        }
        bytes.extend_from_slice(&chunk);
    }
    let encoding = response.header("content-encoding");
    if let Some(length) = declared {
        if encoding.is_none_or(|encoding| encoding == "identity") && bytes.len() != length {
            return Err(CogneeError::new(
                CogneeErrorKind::Malformed,
                "Cognee response did not match Content-Length",
            ));
        }
    }
    let text = if truncated {
        decode_truncated(&bytes)
    } else {
        String::from_utf8_lossy(&bytes).into_owned()
    };
    Ok((text, truncated))
}

pub async fn read_json(
    response: &mut HttpResponse,
    max_bytes: usize,
) -> Result<Value, CogneeError> {
    let (text, _) = read_text(response, max_bytes, false).await?;
    serde_json::from_str(&text)
        .map_err(|_| CogneeError::new(CogneeErrorKind::Malformed, "Cognee returned malformed JSON"))
}

/// A never-cognified dataset answers recall with this 404: empty memory.
pub async fn is_uncognified_dataset(response: &mut HttpResponse) -> bool {
    if response.status != 404 {
        return false;
    }
    match read_json(response, MAX_ERROR_RESPONSE_BYTES).await {
        Ok(payload) => {
            payload.get("error").and_then(Value::as_str) == Some(RECALL_PREREQUISITES_ERROR)
        }
        Err(_) => false,
    }
}

/// Run one request under a deadline and an optional parent signal. Error
/// priority: timeout, abort, a Cognee error, then unreachable.
pub async fn request<T, F, Fut>(
    transport: &dyn CogneeTransport,
    http: HttpRequest,
    parent: Option<&CancellationToken>,
    timeout_ms: u64,
    handle: F,
) -> Result<T, CogneeError>
where
    F: FnOnce(HttpResponse) -> Fut,
    Fut: std::future::Future<Output = Result<T, CogneeError>>,
{
    let deadline = Deadline::after_ms(timeout_ms);
    let aborted = || CogneeError::new(CogneeErrorKind::Aborted, "Cognee request was aborted");
    let timed_out = || CogneeError::new(CogneeErrorKind::Timeout, "Cognee request timed out");
    if parent.is_some_and(CancellationToken::is_cancelled) {
        return Err(aborted());
    }
    let never = CancellationToken::new();
    let parent = parent.unwrap_or(&never);
    let work = async {
        let response = transport.send(http).await.map_err(|_| {
            CogneeError::new(
                CogneeErrorKind::Unreachable,
                "Cognee service is unreachable",
            )
        })?;
        if deadline.expired() {
            return Err(timed_out());
        }
        handle(response).await
    };
    let result = tokio::select! {
        biased;
        _ = parent.cancelled() => Err(aborted()),
        _ = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline.at)) => Err(timed_out()),
        result = work => result,
    };
    // Late completion is still bounded by the same deadline and signal.
    if deadline.expired() {
        return Err(timed_out());
    }
    if parent.is_cancelled() {
        return Err(aborted());
    }
    result
}

// ── recall ──

/// `RecallResult`.
#[derive(Debug, Clone, PartialEq)]
pub struct RecallResult {
    pub text: String,
    pub score: Option<f64>,
    pub metadata: Option<Value>,
}

/// Keep one bullet per graph node with real content.
pub fn condense_graph_context(text: &str) -> String {
    if !text.contains("__node_content_start__") {
        return text.to_string();
    }
    let node = regex!(r"Node: ([^\n]*)\n__node_content_start__\n((?s:.)*?)\n__node_content_end__");
    let session_line = regex!(r"(?m)^Session ID: \S+\s*$");
    let mut lines: Vec<String> = Vec::new();
    for captures in node.captures_iter(text) {
        let title = captures[1].trim();
        let content = super::super::blackhole::js::collapse_whitespace(
            &session_line.replace_all(&captures[2], ""),
        );
        if content.is_empty() || content == title {
            continue;
        }
        let content = if len16(&content) > MAX_GRAPH_NODE_CHARS {
            format!("{}…", prefix16(&content, MAX_GRAPH_NODE_CHARS - 1))
        } else {
            content
        };
        let line = format!("- {content}");
        if !lines.contains(&line) {
            lines.push(line);
        }
    }
    if lines.is_empty() {
        text.to_string()
    } else {
        lines.join("\n")
    }
}

pub fn normalize_recall(payload: &Value) -> Result<Vec<RecallResult>, CogneeError> {
    let values = match payload {
        Value::Array(items) => Some(items),
        Value::Object(map) => match map.get("results").filter(|v| !v.is_null()) {
            Some(Value::Array(items)) => Some(items),
            Some(_) => None,
            None => map.get("data").and_then(Value::as_array),
        },
        _ => None,
    }
    .ok_or_else(|| {
        CogneeError::new(
            CogneeErrorKind::Malformed,
            "Cognee recall returned an unexpected shape",
        )
    })?;
    Ok(values
        .iter()
        .filter_map(|value| match value {
            Value::String(text) => Some(RecallResult {
                text: text.clone(),
                score: None,
                metadata: None,
            }),
            Value::Object(item) => {
                let raw = item
                    .get("text")
                    .and_then(Value::as_str)
                    .or_else(|| item.get("content").and_then(Value::as_str))?;
                let text = condense_graph_context(raw);
                if text.is_empty() {
                    return None;
                }
                Some(RecallResult {
                    text,
                    score: item.get("score").and_then(Value::as_f64),
                    metadata: item
                        .get("metadata")
                        .filter(|metadata| metadata.is_object() || metadata.is_array())
                        .cloned(),
                })
            }
            Value::Array(_) => None,
            _ => None,
        })
        .collect())
}

/// Client configuration (`CogneeClientConfig`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientConfig {
    pub base_url: String,
    pub dataset: String,
    pub api_key: Option<String>,
    pub recall_budget_ms: u64,
    pub max_response_chars: usize,
}

#[derive(Debug, Clone, Default)]
pub struct RecallOptions {
    pub top_k: Option<u32>,
    pub dataset: Option<String>,
    pub session_id: Option<String>,
    pub scope: Vec<String>,
    pub signal: Option<CancellationToken>,
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Default)]
pub struct CallOptions {
    pub signal: Option<CancellationToken>,
    pub timeout_ms: Option<u64>,
}

/// `CogneeServerModel`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServerModel {
    pub provider: Option<String>,
    pub model: Option<String>,
}

/// The memory client used by the extension runtime.
#[derive(Clone)]
pub struct CogneeClient {
    pub config: ClientConfig,
    transport: Arc<dyn CogneeTransport>,
}

impl CogneeClient {
    pub fn new(config: ClientConfig, transport: Arc<dyn CogneeTransport>) -> Self {
        Self { config, transport }
    }

    fn json_post(&self, path: &str, body: Value) -> Result<HttpRequest, CogneeError> {
        Ok(HttpRequest {
            method: Method::Post,
            url: build_url(&self.config.base_url, path)?,
            headers: headers(self.config.api_key.as_deref(), Some("application/json")),
            body: Body::Json(body.to_string()),
        })
    }

    /// Context-only recall for one dataset, optionally including the session
    /// cache. Text is capped cumulatively at `max_response_chars`; metadata
    /// is not.
    pub async fn recall(
        &self,
        query: &str,
        options: RecallOptions,
    ) -> Result<Vec<RecallResult>, CogneeError> {
        let scope = if options.scope.is_empty() {
            vec!["session".to_string(), "trace".into(), "graph".into()]
        } else {
            options.scope.clone()
        };
        let top_k = options.top_k.unwrap_or(5);
        let mut body = json!({
            "query": query,
            "topK": top_k,
            "onlyContext": true,
            "scope": scope,
        });
        if let Some(session) = options.session_id.as_ref().filter(|id| !id.is_empty()) {
            body["sessionId"] = json!(session);
        }
        body["datasets"] = json!([options
            .dataset
            .clone()
            .unwrap_or_else(|| self.config.dataset.clone())]);
        let http = self.json_post("/api/v1/recall", body)?;
        let payload = request(
            self.transport.as_ref(),
            http,
            options.signal.as_ref(),
            options.timeout_ms.unwrap_or(self.config.recall_budget_ms),
            |mut response| async move {
                if is_uncognified_dataset(&mut response).await {
                    return Ok(Value::Array(Vec::new()));
                }
                assert_ok(&response, "recall")?;
                read_json(&mut response, MAX_RECALL_RESPONSE_BYTES).await
            },
        )
        .await?;
        let results = normalize_recall(&payload)?;
        let mut remaining = self.config.max_response_chars;
        Ok(results
            .into_iter()
            .take(top_k as usize)
            .filter_map(|mut result| {
                if remaining == 0 {
                    return None;
                }
                let text = prefix16(&result.text, remaining).to_string();
                remaining = remaining.saturating_sub(len16(&text));
                if text.is_empty() {
                    return None;
                }
                result.text = text;
                Some(result)
            })
            .collect())
    }

    /// Background ingestion into a dataset/node set. Success means accepted.
    pub async fn remember(
        &self,
        text: &str,
        node_set: &str,
        dataset: Option<&str>,
        session_id: Option<&str>,
        options: CallOptions,
    ) -> Result<(), CogneeError> {
        let mut parts = vec![
            FormPart {
                name: "datasetName".into(),
                value: dataset.unwrap_or(&self.config.dataset).into(),
                filename: None,
                content_type: None,
            },
            FormPart {
                name: "node_set".into(),
                value: node_set.into(),
                filename: None,
                content_type: None,
            },
            FormPart {
                name: "run_in_background".into(),
                value: "true".into(),
                filename: None,
                content_type: None,
            },
        ];
        if let Some(session) = session_id.filter(|id| !id.is_empty()) {
            parts.push(FormPart {
                name: "session_id".into(),
                value: session.into(),
                filename: None,
                content_type: None,
            });
        }
        parts.push(FormPart {
            name: "data".into(),
            value: text.into(),
            filename: Some("ice-cognee.txt".into()),
            content_type: Some("text/plain".into()),
        });
        let http = HttpRequest {
            method: Method::Post,
            url: build_url(&self.config.base_url, "/api/v1/remember")?,
            headers: headers(self.config.api_key.as_deref(), None),
            body: Body::Multipart(parts),
        };
        request(
            self.transport.as_ref(),
            http,
            options.signal.as_ref(),
            options
                .timeout_ms
                .unwrap_or(self.config.recall_budget_ms.max(30_000)),
            |response| async move { assert_ok(&response, "remember") },
        )
        .await
    }

    /// Store a QA/trace entry in the session cache. A malformed or oversized
    /// acknowledgement after HTTP success is tolerated so the accepted write
    /// is not replayed.
    pub async fn remember_entry(
        &self,
        entry: &Value,
        dataset: Option<&str>,
        session_id: &str,
        options: CallOptions,
    ) -> Result<Option<String>, CogneeError> {
        let http = self.json_post(
            "/api/v1/remember/entry",
            json!({
                "entry": entry,
                "dataset_name": dataset.unwrap_or(&self.config.dataset),
                "session_id": session_id,
            }),
        )?;
        let cap = self.config.max_response_chars;
        request(
            self.transport.as_ref(),
            http,
            options.signal.as_ref(),
            options.timeout_ms.unwrap_or(30_000),
            |mut response| async move {
                assert_ok(&response, "remember/entry")?;
                match read_json(&mut response, cap).await {
                    Ok(payload) => Ok(payload
                        .get("entry_id")
                        .and_then(Value::as_str)
                        .map(str::to_string)),
                    Err(error)
                        if matches!(
                            error.kind,
                            CogneeErrorKind::Malformed | CogneeErrorKind::ResponseTooLarge
                        ) =>
                    {
                        Ok(None)
                    }
                    Err(error) => Err(error),
                }
            },
        )
        .await
    }

    /// Ask Cognee to consolidate the session cache (accepted, not completed).
    pub async fn improve(
        &self,
        dataset: Option<&str>,
        session_ids: &[String],
        options: CallOptions,
    ) -> Result<(), CogneeError> {
        let mut body = json!({"datasetName": dataset.unwrap_or(&self.config.dataset)});
        if !session_ids.is_empty() {
            body["sessionIds"] = json!(session_ids);
        }
        body["runInBackground"] = json!(true);
        let http = self.json_post("/api/v1/improve", body)?;
        request(
            self.transport.as_ref(),
            http,
            options.signal.as_ref(),
            options.timeout_ms.unwrap_or(120_000),
            |response| async move { assert_ok(&response, "improve") },
        )
        .await
    }

    /// Register this session as a hybrid-memory API agent; a 404 counts as
    /// success on servers without the endpoint.
    pub async fn register_agent(
        &self,
        agent_session_name: &str,
        session_id: Option<&str>,
        dataset_names: &[String],
        options: CallOptions,
    ) -> Result<(), CogneeError> {
        let mut body = json!({
            "agent_session_name": agent_session_name,
            "type": "api",
            "memory_mode": "hybrid",
            "source": "api",
        });
        if let Some(session) = session_id.filter(|id| !id.is_empty()) {
            body["session_id"] = json!(session);
        }
        if !dataset_names.is_empty() {
            body["dataset_names"] = json!(dataset_names);
        }
        let http = self.json_post("/api/v1/agents/register", body)?;
        request(
            self.transport.as_ref(),
            http,
            options.signal.as_ref(),
            options.timeout_ms.unwrap_or(15_000),
            |response| async move {
                if response.status == 404 {
                    return Ok(());
                }
                assert_ok(&response, "agents/register")
            },
        )
        .await
    }

    pub async fn unregister_agent(
        &self,
        agent_session_name: &str,
        options: CallOptions,
    ) -> Result<(), CogneeError> {
        let http = self.json_post(
            "/api/v1/agents/unregister",
            json!({"agent_session_name": agent_session_name}),
        )?;
        request(
            self.transport.as_ref(),
            http,
            options.signal.as_ref(),
            options.timeout_ms.unwrap_or(15_000),
            |response| async move {
                if response.status == 404 {
                    return Ok(());
                }
                assert_ok(&response, "agents/unregister")
            },
        )
        .await
    }

    /// The server's model identity; credential-bearing settings are dropped.
    pub async fn server_model(&self, options: CallOptions) -> Result<ServerModel, CogneeError> {
        let http = HttpRequest {
            method: Method::Get,
            url: build_url(&self.config.base_url, "/api/v1/settings")?,
            headers: headers(self.config.api_key.as_deref(), None),
            body: Body::Empty,
        };
        request(
            self.transport.as_ref(),
            http,
            options.signal.as_ref(),
            options.timeout_ms.unwrap_or(5_000),
            |mut response| async move {
                assert_ok(&response, "settings")?;
                let payload = read_json(&mut response, MAX_SETTINGS_RESPONSE_BYTES).await?;
                let Some(llm) = payload
                    .get("llm")
                    .filter(|llm| llm.is_object() || llm.is_array())
                else {
                    return Ok(ServerModel::default());
                };
                let text = |key: &str| {
                    llm.get(key)
                        .and_then(Value::as_str)
                        .filter(|text| !text.is_empty())
                        .map(str::to_string)
                };
                Ok(ServerModel {
                    provider: text("provider"),
                    model: text("model"),
                })
            },
        )
        .await
    }

    /// `GET <base>/health` with a 3 s timeout and a bounded body. The
    /// TypeScript probe reads an unbounded body; icers caps it at 4 KiB.
    pub async fn probe_health(&self) -> (bool, String) {
        let url = format!(
            "{}/health",
            self.config
                .base_url
                .strip_suffix('/')
                .unwrap_or(&self.config.base_url)
        );
        let mut probe_headers = Vec::new();
        if let Some(key) = self.config.api_key.as_ref().filter(|key| !key.is_empty()) {
            probe_headers.push(("x-api-key".to_string(), key.clone()));
        }
        let http = HttpRequest {
            method: Method::Get,
            url,
            headers: probe_headers,
            body: Body::Empty,
        };
        let result = request(
            self.transport.as_ref(),
            http,
            None,
            3_000,
            |mut response| async move {
                if !response.ok() {
                    return Ok((false, format!("HTTP {}", response.status)));
                }
                let (text, _) = read_text(&mut response, MAX_ERROR_RESPONSE_BYTES, true).await?;
                Ok((true, text.chars().take(120).collect()))
            },
        )
        .await;
        match result {
            Ok(outcome) => outcome,
            Err(error) => (false, error.message.chars().take(120).collect()),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::Mutex;

    /// A scripted transport. Each response is (status, headers, chunks, delay).
    #[derive(Default)]
    pub(crate) struct FakeTransport {
        pub(crate) requests: Mutex<Vec<HttpRequest>>,
        responses: Mutex<std::collections::VecDeque<FakeResponse>>,
    }

    pub(crate) enum FakeResponse {
        Ok {
            status: u16,
            headers: Vec<(String, String)>,
            chunks: Vec<Vec<u8>>,
            delay_ms: u64,
        },
        Fail,
    }

    impl FakeResponse {
        pub(crate) fn json(status: u16, value: Value) -> Self {
            Self::Ok {
                status,
                headers: Vec::new(),
                chunks: vec![value.to_string().into_bytes()],
                delay_ms: 0,
            }
        }
    }

    impl FakeTransport {
        pub(crate) fn push(&self, response: FakeResponse) {
            self.responses.lock().unwrap().push_back(response);
        }
        pub(crate) fn sent(&self) -> Vec<HttpRequest> {
            self.requests.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl CogneeTransport for FakeTransport {
        async fn send(&self, request: HttpRequest) -> Result<HttpResponse, String> {
            self.requests.lock().unwrap().push(request);
            let next = self.responses.lock().unwrap().pop_front();
            match next {
                Some(FakeResponse::Ok {
                    status,
                    headers,
                    chunks,
                    delay_ms,
                }) => {
                    if delay_ms > 0 {
                        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                    }
                    Ok(HttpResponse {
                        status,
                        headers,
                        body: Box::pin(futures::stream::iter(chunks.into_iter().map(Ok))),
                    })
                }
                Some(FakeResponse::Fail) | None => Err("connection refused".into()),
            }
        }
    }

    pub(crate) fn client(transport: Arc<FakeTransport>) -> CogneeClient {
        CogneeClient::new(
            ClientConfig {
                base_url: "http://cognee.test/base/".into(),
                dataset: "ice".into(),
                api_key: Some("key".into()),
                recall_budget_ms: 1_000,
                max_response_chars: 12_000,
            },
            transport,
        )
    }

    fn body_json(request: &HttpRequest) -> Value {
        match &request.body {
            Body::Json(text) => serde_json::from_str(text).unwrap(),
            other => panic!("expected JSON body, got {other:?}"),
        }
    }

    #[test]
    fn urls_keep_the_base_path_and_drop_query() {
        assert_eq!(
            build_url("http://h:1/base/?x=1#f", "/api/v1/recall").unwrap(),
            "http://h:1/base/api/v1/recall"
        );
        assert_eq!(classify_status(403), CogneeErrorKind::AuthFailed);
        assert_eq!(classify_status(429), CogneeErrorKind::ServerError);
        assert_eq!(classify_status(400), CogneeErrorKind::Malformed);
    }

    #[test]
    fn truncated_decode_drops_a_partial_trailing_character() {
        let bytes = "a€".as_bytes();
        assert_eq!(decode_truncated(&bytes[..3]), "a");
        assert_eq!(decode_truncated(&[0x61, 0xff, 0x62]), "a\u{FFFD}b");
    }

    #[tokio::test]
    async fn recall_posts_the_exact_body_and_caps_text() {
        let transport = Arc::new(FakeTransport::default());
        transport.push(FakeResponse::json(
            200,
            json!({"results": [{"text": "x".repeat(10), "score": 0.5, "metadata": {"a": 1}}, "plain", null, {"content": ""}]}),
        ));
        let mut client = client(transport.clone());
        client.config.max_response_chars = 12;
        let results = client
            .recall(
                "q",
                RecallOptions {
                    session_id: Some("ice_s".into()),
                    ..RecallOptions::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].score, Some(0.5));
        assert_eq!(results[1].text, "pl");
        let sent = transport.sent();
        assert_eq!(sent[0].url, "http://cognee.test/base/api/v1/recall");
        assert!(sent[0]
            .headers
            .contains(&("x-api-key".into(), "key".into())));
        assert_eq!(
            body_json(&sent[0]),
            json!({"query":"q","topK":5,"onlyContext":true,"scope":["session","trace","graph"],"sessionId":"ice_s","datasets":["ice"]})
        );
    }

    #[tokio::test]
    async fn uncognified_dataset_is_empty_memory_but_other_404s_fail() {
        let transport = Arc::new(FakeTransport::default());
        transport.push(FakeResponse::json(
            404,
            json!({"error": "Recall prerequisites not met"}),
        ));
        transport.push(FakeResponse::json(404, json!({"error": "nope"})));
        let client = client(transport);
        assert!(client
            .recall("q", RecallOptions::default())
            .await
            .unwrap()
            .is_empty());
        let error = client
            .recall("q", RecallOptions::default())
            .await
            .unwrap_err();
        assert_eq!(error.kind, CogneeErrorKind::NotFound);
        assert_eq!(error.status, Some(404));
    }

    #[tokio::test]
    async fn byte_limits_and_content_length_are_enforced() {
        let transport = Arc::new(FakeTransport::default());
        transport.push(FakeResponse::Ok {
            status: 200,
            headers: vec![("content-length".into(), "999999999".into())],
            chunks: vec![],
            delay_ms: 0,
        });
        transport.push(FakeResponse::Ok {
            status: 200,
            headers: vec![("content-length".into(), "12x".into())],
            chunks: vec![],
            delay_ms: 0,
        });
        transport.push(FakeResponse::Ok {
            status: 200,
            headers: vec![("content-length".into(), "100".into())],
            chunks: vec![b"[]".to_vec()],
            delay_ms: 0,
        });
        transport.push(FakeResponse::Ok {
            status: 200,
            headers: vec![],
            chunks: vec![vec![b' '; MAX_RECALL_RESPONSE_BYTES], b"[]".to_vec()],
            delay_ms: 0,
        });
        let client = client(transport);
        for expected in [
            CogneeErrorKind::ResponseTooLarge,
            CogneeErrorKind::Malformed,
            CogneeErrorKind::Malformed,
            CogneeErrorKind::ResponseTooLarge,
        ] {
            let error = client
                .recall("q", RecallOptions::default())
                .await
                .unwrap_err();
            assert_eq!(error.kind, expected);
        }
    }

    #[tokio::test]
    async fn deadline_and_parent_signal_classify_errors() {
        let transport = Arc::new(FakeTransport::default());
        transport.push(FakeResponse::Ok {
            status: 200,
            headers: vec![],
            chunks: vec![b"[]".to_vec()],
            delay_ms: 200,
        });
        let client = client(transport.clone());
        let error = client
            .recall(
                "q",
                RecallOptions {
                    timeout_ms: Some(20),
                    ..RecallOptions::default()
                },
            )
            .await
            .unwrap_err();
        assert_eq!(error.kind, CogneeErrorKind::Timeout);

        let signal = CancellationToken::new();
        signal.cancel();
        let error = client
            .recall(
                "q",
                RecallOptions {
                    signal: Some(signal),
                    ..RecallOptions::default()
                },
            )
            .await
            .unwrap_err();
        assert_eq!(error.kind, CogneeErrorKind::Aborted);

        transport.push(FakeResponse::Fail);
        let error = client
            .recall("q", RecallOptions::default())
            .await
            .unwrap_err();
        assert_eq!(error.kind, CogneeErrorKind::Unreachable);
    }

    #[tokio::test]
    async fn write_endpoints_use_their_wire_shapes() {
        let transport = Arc::new(FakeTransport::default());
        transport.push(FakeResponse::json(200, json!({})));
        transport.push(FakeResponse::Ok {
            status: 200,
            headers: vec![],
            chunks: vec![b"not json".to_vec()],
            delay_ms: 0,
        });
        transport.push(FakeResponse::json(202, json!({})));
        transport.push(FakeResponse::json(404, json!({})));
        transport.push(FakeResponse::json(404, json!({})));
        transport.push(FakeResponse::json(
            200,
            json!({"llm": {"provider": "openai", "model": "gpt", "api_key": "secret"}}),
        ));
        let client = client(transport.clone());
        client
            .remember(
                "fact",
                "user_context",
                None,
                Some("ice_s"),
                CallOptions::default(),
            )
            .await
            .unwrap();
        assert_eq!(
            client
                .remember_entry(&json!({"type":"qa"}), None, "ice_s", CallOptions::default())
                .await
                .unwrap(),
            None
        );
        client
            .improve(None, &["ice_s".into()], CallOptions::default())
            .await
            .unwrap();
        client
            .register_agent(
                "ice_s",
                Some("ice_s"),
                &["ice".into()],
                CallOptions::default(),
            )
            .await
            .unwrap();
        client
            .unregister_agent("ice_s", CallOptions::default())
            .await
            .unwrap();
        let model = client.server_model(CallOptions::default()).await.unwrap();
        assert_eq!(model.provider.as_deref(), Some("openai"));
        assert_eq!(model.model.as_deref(), Some("gpt"));

        let sent = transport.sent();
        let Body::Multipart(parts) = &sent[0].body else {
            panic!("remember is multipart")
        };
        let names: Vec<&str> = parts.iter().map(|part| part.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "datasetName",
                "node_set",
                "run_in_background",
                "session_id",
                "data"
            ]
        );
        assert_eq!(parts[4].filename.as_deref(), Some("ice-cognee.txt"));
        assert_eq!(
            body_json(&sent[1]),
            json!({"entry":{"type":"qa"},"dataset_name":"ice","session_id":"ice_s"})
        );
        assert_eq!(
            body_json(&sent[2]),
            json!({"datasetName":"ice","sessionIds":["ice_s"],"runInBackground":true})
        );
        assert_eq!(
            body_json(&sent[3]),
            json!({"agent_session_name":"ice_s","type":"api","memory_mode":"hybrid","source":"api","session_id":"ice_s","dataset_names":["ice"]})
        );
    }

    #[test]
    fn graph_context_condenses_to_node_lines() {
        let text = "Nodes:\nNode: Feature\n__node_content_start__\nFeature\n__node_content_end__\nNode: Cache\n__node_content_start__\nSession ID: ice_1\nThe cache uses   LRU eviction\n__node_content_end__\nNode: Cache2\n__node_content_start__\nThe cache uses LRU eviction\n__node_content_end__";
        assert_eq!(
            condense_graph_context(text),
            "- The cache uses LRU eviction"
        );
        assert_eq!(condense_graph_context("plain"), "plain");
    }

    #[test]
    fn multipart_encoding_quotes_names() {
        let (content_type, body) = multipart_body(&[FormPart {
            name: "data".into(),
            value: "hello".into(),
            filename: Some("ice-cognee.txt".into()),
            content_type: Some("text/plain".into()),
        }]);
        assert!(content_type.starts_with("multipart/form-data; boundary="));
        let body = String::from_utf8(body).unwrap();
        assert!(body.contains("name=\"data\"; filename=\"ice-cognee.txt\"\r\nContent-Type: text/plain\r\n\r\nhello\r\n"));
    }
}
