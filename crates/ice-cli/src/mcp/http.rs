//! Streamable HTTP transport (MCP 2025-06-18) with the legacy HTTP+SSE fallback.
//! Messages go out as POSTs; replies come back as JSON or an SSE stream, and a
//! GET stream carries server-initiated messages. Ports the transport choice in
//! pi-mcp-adapter `server-manager.ts`; frames feed the shared `Router`.
use super::*;

use futures::StreamExt;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, ACCEPT, CONTENT_TYPE};

const SESSION: &str = "mcp-session-id";
const PROTOCOL: &str = "mcp-protocol-version";
/// How long the legacy fallback waits for the server's `endpoint` event.
const ENDPOINT_WAIT: Duration = Duration::from_secs(10);
const LISTEN_RETRY_MAX: Duration = Duration::from_secs(30);

/// Expand `${VAR}` and `$env:VAR`; a missing variable is an error, so nothing is
/// sent with a half-resolved URL or header.
pub(super) fn interpolate(value: &str) -> Result<String, String> {
    let mut out = String::new();
    let mut rest = value;
    while let Some(at) = rest.find('$') {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        let (name, used) = if let Some(braced) = tail.strip_prefix("${") {
            match braced.find('}') {
                Some(end) => (&braced[..end], end + 3),
                None => return Err(format!("unterminated variable in {value:?}")),
            }
        } else if let Some(env) = tail.strip_prefix("$env:") {
            let end = env
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .unwrap_or(env.len());
            (&env[..end], end + 5)
        } else {
            out.push('$');
            rest = &tail[1..];
            continue;
        };
        if name.is_empty() {
            return Err(format!("empty variable name in {value:?}"));
        }
        out.push_str(
            &std::env::var(name).map_err(|_| format!("environment variable {name} is not set"))?,
        );
        rest = &tail[used..];
    }
    out.push_str(rest);
    Ok(out)
}

/// The resolved request target: URL and headers, including any bearer token.
pub(super) struct Target {
    pub url: reqwest::Url,
    pub headers: HeaderMap,
}
pub(super) fn target(config: &ServerConfig) -> Result<Target, String> {
    let raw = config.url.as_deref().ok_or("missing MCP url")?;
    let url =
        reqwest::Url::parse(&interpolate(raw)?).map_err(|e| format!("invalid MCP url: {e}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("MCP url must use http or https".into());
    }
    let mut headers = HeaderMap::new();
    for (name, value) in &config.headers {
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| format!("invalid MCP header name {name:?}"))?;
        let value = HeaderValue::from_str(&interpolate(value)?)
            .map_err(|_| format!("invalid value for MCP header {name}"))?;
        headers.insert(name, value);
    }
    if config.auth == Some(Auth::Bearer) {
        let token = match (&config.bearer_token, &config.bearer_token_env) {
            (Some(token), _) => interpolate(token)?,
            (None, Some(env)) => {
                std::env::var(env).map_err(|_| format!("environment variable {env} is not set"))?
            }
            (None, None) => {
                return Err("auth \"bearer\" needs bearerToken or bearerTokenEnv".into())
            }
        };
        let mut value = HeaderValue::from_str(&format!("Bearer {}", token.trim()))
            .map_err(|_| "invalid bearer token")?;
        value.set_sensitive(true);
        headers.insert(reqwest::header::AUTHORIZATION, value);
    }
    Ok(Target { url, headers })
}

/// Server-sent events: `(event, data)` pairs, split across chunks as needed.
#[derive(Default)]
struct SseParser {
    line: Vec<u8>,
    event: String,
    data: String,
}
impl SseParser {
    fn feed(&mut self, chunk: &[u8], out: &mut Vec<(String, String)>) -> Result<(), String> {
        for &byte in chunk {
            if byte != b'\n' {
                self.line.push(byte);
                if self.line.len() as u64 > MAX_LINE {
                    return Err("MCP SSE line exceeds 16 MiB".into());
                }
                continue;
            }
            let mut line = std::mem::take(&mut self.line);
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            let line = String::from_utf8_lossy(&line);
            if line.is_empty() {
                if !self.data.is_empty() {
                    let event = std::mem::take(&mut self.event);
                    let data = std::mem::take(&mut self.data);
                    out.push((
                        if event.is_empty() {
                            "message".into()
                        } else {
                            event
                        },
                        data,
                    ));
                }
                self.event.clear();
                continue;
            }
            let (field, value) = line.split_once(':').unwrap_or((&line, ""));
            let value = value.strip_prefix(' ').unwrap_or(value);
            match field {
                "event" => self.event = value.into(),
                "data" => {
                    if !self.data.is_empty() {
                        self.data.push('\n');
                    }
                    self.data.push_str(value);
                    if self.data.len() as u64 > MAX_LINE {
                        return Err("MCP SSE event exceeds 16 MiB".into());
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
}

struct Endpoint {
    http: reqwest::Client,
    target: Target,
    session: Mutex<Option<String>>,
    protocol: Mutex<Option<String>>,
    /// Set once the server is known to speak the legacy HTTP+SSE transport.
    legacy: Mutex<Option<reqwest::Url>>,
    router: Router,
    inbound: mpsc::Sender<Inbound>,
    stop: CancellationToken,
}

/// Ends the HTTP session: stops the tasks and best-effort DELETEs the session.
pub(super) struct HttpLink {
    stop: CancellationToken,
}
impl HttpLink {
    pub(super) fn close(&self) {
        self.stop.cancel();
    }
}
impl Drop for HttpLink {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

impl Client {
    pub(super) fn connect_http(
        config: &ServerConfig,
        handlers: Handlers,
    ) -> Result<Arc<Self>, String> {
        let target = target(config)?;
        let runtime = tokio::runtime::Handle::try_current()
            .map_err(|_| "MCP client requires a Tokio runtime")?;
        let stop = CancellationToken::new();
        let (client, router, inbound, receiver) =
            Self::new(Link::Http(HttpLink { stop: stop.clone() }), handlers)?;
        let http = reqwest::Client::builder()
            .connect_timeout(START_TIMEOUT)
            .build()
            .map_err(|e| e.to_string())?;
        let endpoint = Arc::new(Endpoint {
            http,
            target,
            session: Mutex::new(None),
            protocol: Mutex::new(None),
            legacy: Mutex::new(None),
            router,
            inbound,
            stop,
        });
        // The outgoing queue is synchronous (shared with stdio); bridge it to a task.
        let (queue, mut queued) = mpsc::unbounded_channel::<Value>();
        std::thread::spawn(move || {
            for value in receiver {
                if queue.send(value).is_err() {
                    break;
                }
            }
        });
        runtime.spawn(async move {
            loop {
                let message = tokio::select! {
                    _ = endpoint.stop.cancelled() => break,
                    message = queued.recv() => match message {
                        Some(message) => message,
                        None => break,
                    },
                };
                // Requests may stream for a long time; everything else is sent in
                // order so `notifications/initialized` precedes later requests.
                if message.get("method").is_some() && message.get("id").is_some() {
                    let endpoint = endpoint.clone();
                    tokio::spawn(async move { endpoint.post(message).await });
                } else {
                    endpoint.post(message).await;
                }
            }
            endpoint.shut_down().await;
        });
        Ok(client)
    }
}

impl Endpoint {
    fn headers(&self) -> HeaderMap {
        let mut headers = self.target.headers.clone();
        if let Some(session) = self.session.lock().unwrap().as_deref() {
            if let Ok(value) = HeaderValue::from_str(session) {
                headers.insert(SESSION, value);
            }
        }
        if let Some(protocol) = self.protocol.lock().unwrap().as_deref() {
            if let Ok(value) = HeaderValue::from_str(protocol) {
                headers.insert(PROTOCOL, value);
            }
        }
        headers
    }

    async fn deliver(&self, value: Value) -> bool {
        match self.router.route(value) {
            Ok(Some(frame)) => self.inbound.send(frame).await.is_ok(),
            Ok(None) => true,
            Err(()) => false,
        }
    }

    /// A message the server will never answer: fail the matching call, if any.
    fn fail(&self, message: &Value, reason: String) {
        if message.get("method").is_none() {
            return;
        }
        if let Some(id) = message.get("id").and_then(Value::as_u64) {
            if let Some(reply) = self.router.pending.lock().unwrap().remove(&id) {
                let _ = reply.send(Err(reason));
            }
        }
    }

    async fn post(self: &Arc<Self>, message: Value) {
        let legacy = self.legacy.lock().unwrap().clone();
        let url = legacy.clone().unwrap_or_else(|| self.target.url.clone());
        let mut headers = self.headers();
        headers.insert(
            ACCEPT,
            HeaderValue::from_static("application/json, text/event-stream"),
        );
        let request = self.http.post(url).headers(headers).json(&message).send();
        let response = tokio::select! {
            _ = self.stop.cancelled() => return,
            response = request => response,
        };
        let response = match response {
            Ok(response) => response,
            Err(error) => return self.fail(&message, format!("MCP HTTP request failed: {error}")),
        };
        let status = response.status();
        let initialize = message.get("method").and_then(Value::as_str) == Some("initialize");
        // Endpoints that reject Streamable HTTP outright speak the legacy transport.
        if initialize && legacy.is_none() && matches!(status.as_u16(), 404 | 405 | 406 | 415) {
            return match self.open_legacy().await {
                Ok(()) => Box::pin(self.post(message)).await,
                Err(error) => self.fail(&message, error),
            };
        }
        if matches!(status.as_u16(), 401 | 403) {
            return self.fail(
                &message,
                format!(
                    "MCP server requires authentication (HTTP {})",
                    status.as_u16()
                ),
            );
        }
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let body: String = body.chars().take(200).collect();
            return self.fail(&message, format!("MCP HTTP {status}: {}", body.trim()));
        }
        if let Some(session) = response
            .headers()
            .get(SESSION)
            .and_then(|v| v.to_str().ok())
        {
            *self.session.lock().unwrap() = Some(session.to_owned());
        }
        if status == reqwest::StatusCode::ACCEPTED {
            return;
        }
        let event_stream = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("text/event-stream"));
        if event_stream {
            self.read_events(response, |_, _| {}).await;
        } else {
            let bytes = match response.bytes().await {
                Ok(bytes) if bytes.len() as u64 <= MAX_LINE => bytes,
                Ok(_) => return self.fail(&message, "MCP HTTP response exceeds 16 MiB".into()),
                Err(error) => {
                    return self.fail(&message, format!("MCP HTTP response failed: {error}"))
                }
            };
            if bytes.is_empty() {
                return;
            }
            let value: Value = match serde_json::from_slice(&bytes) {
                Ok(value) => value,
                Err(error) => {
                    return self.fail(&message, format!("MCP HTTP response is not JSON: {error}"))
                }
            };
            if initialize {
                self.remember_protocol(&value);
            }
            let values = match value {
                Value::Array(values) => values,
                value => vec![value],
            };
            for value in values {
                if !self.deliver(value).await {
                    return;
                }
            }
        }
        if initialize && legacy.is_none() {
            let endpoint = self.clone();
            tokio::spawn(async move { endpoint.listen().await });
        }
    }

    fn remember_protocol(&self, response: &Value) {
        if let Some(version) = response["result"]["protocolVersion"].as_str() {
            *self.protocol.lock().unwrap() = Some(version.to_owned());
        }
    }

    /// Route every `message` event; `other` sees the rest (the legacy `endpoint`).
    async fn read_events(&self, response: reqwest::Response, mut other: impl FnMut(&str, &str)) {
        let mut parser = SseParser::default();
        let mut stream = response.bytes_stream();
        let mut events = Vec::new();
        loop {
            let chunk = tokio::select! {
                _ = self.stop.cancelled() => return,
                chunk = stream.next() => chunk,
            };
            let Some(Ok(chunk)) = chunk else {
                return;
            };
            if parser.feed(&chunk, &mut events).is_err() {
                return;
            }
            for (event, data) in events.drain(..) {
                if event != "message" {
                    other(&event, &data);
                    continue;
                }
                let Ok(value) = serde_json::from_str::<Value>(&data) else {
                    continue;
                };
                if value["result"]["protocolVersion"].is_string() {
                    self.remember_protocol(&value);
                }
                if !self.deliver(value).await {
                    return;
                }
            }
        }
    }

    /// The GET stream for server-initiated messages; servers may not offer one
    /// (405). It is reopened with backoff until the session ends.
    async fn listen(self: Arc<Self>) {
        let mut wait = Duration::from_secs(1);
        while !self.stop.is_cancelled() {
            let mut headers = self.headers();
            headers.insert(ACCEPT, HeaderValue::from_static("text/event-stream"));
            let request = self
                .http
                .get(self.target.url.clone())
                .headers(headers)
                .send();
            let response = tokio::select! {
                _ = self.stop.cancelled() => return,
                response = request => response,
            };
            match response {
                Ok(response) if response.status().is_success() => {
                    wait = Duration::from_secs(1);
                    self.read_events(response, |_, _| {}).await;
                }
                Ok(response) if response.status() == reqwest::StatusCode::METHOD_NOT_ALLOWED => {
                    return
                }
                _ => {}
            }
            tokio::select! {
                _ = self.stop.cancelled() => return,
                _ = tokio::time::sleep(wait) => {}
            }
            wait = (wait * 2).min(LISTEN_RETRY_MAX);
        }
    }

    /// Legacy HTTP+SSE: GET the stream, whose `endpoint` event names the POST URL.
    async fn open_legacy(self: &Arc<Self>) -> Result<(), String> {
        let mut headers = self.target.headers.clone();
        headers.insert(ACCEPT, HeaderValue::from_static("text/event-stream"));
        let response = self
            .http
            .get(self.target.url.clone())
            .headers(headers)
            .send()
            .await
            .map_err(|e| format!("MCP SSE connection failed: {e}"))?;
        if !response.status().is_success() {
            return Err(format!(
                "MCP SSE connection failed: HTTP {}",
                response.status()
            ));
        }
        let (found, endpoint) = oneshot::channel::<String>();
        let found = Mutex::new(Some(found));
        let this = self.clone();
        tokio::spawn(async move {
            this.read_events(response, |event, data| {
                if event == "endpoint" {
                    if let Some(found) = found.lock().unwrap().take() {
                        let _ = found.send(data.trim().to_owned());
                    }
                }
            })
            .await;
            // The legacy stream carries every reply; losing it ends the client.
            if !this.stop.is_cancelled() {
                this.stop.cancel();
            }
        });
        let path = tokio::time::timeout(ENDPOINT_WAIT, endpoint)
            .await
            .map_err(|_| "MCP SSE server sent no endpoint".to_string())?
            .map_err(|_| "MCP SSE stream closed before its endpoint".to_string())?;
        let url = self
            .target
            .url
            .join(&path)
            .map_err(|e| format!("invalid MCP SSE endpoint: {e}"))?;
        // Never let a server redirect messages (and our headers) to another origin.
        if url.origin() != self.target.url.origin() {
            return Err("MCP SSE endpoint is on a different origin".into());
        }
        *self.legacy.lock().unwrap() = Some(url);
        Ok(())
    }

    async fn shut_down(&self) {
        let session = self.session.lock().unwrap().take();
        if let Some(session) = session {
            let mut headers = self.target.headers.clone();
            if let Ok(value) = HeaderValue::from_str(&session) {
                headers.insert(SESSION, value);
            }
            let delete = self
                .http
                .delete(self.target.url.clone())
                .headers(headers)
                .send();
            let _ = tokio::time::timeout(Duration::from_secs(2), delete).await;
        }
        let _ = self.inbound.send(Inbound::Shutdown).await;
        self.router.fail("MCP HTTP session closed");
    }
}

#[cfg(test)]
#[path = "http_tests.rs"]
mod tests;
