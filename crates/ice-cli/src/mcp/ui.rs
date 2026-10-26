//! MCP UI (MCP Apps): when a called tool declares `_meta.ui.resourceUri`, its
//! `ui://` resource is served from a token-guarded loopback server and opened
//! in the browser. A host page written here (D8) speaks the MCP Apps
//! postMessage protocol to the app in a sandboxed iframe: initialize, tool
//! input and results, tool calls and resource reads back to the server (behind
//! a per-server consent prompt), and `ui/message` prompts that wake the agent.
//! Ports pi-mcp-adapter `ui-session.ts`, `ui-server.ts`,
//! `ui-resource-handler.ts` and `host-html-template.ts`.
use super::*;

use base64::Engine;

use crate::observe::server::{host_allowed, parse_request, sse_frame, tokens_match};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// MCP Apps protocol version this host implements.
const PROTOCOL: &str = "2026-01-26";
const MAX_HEAD: usize = 16 * 1024;
const MAX_BODY: usize = 1024 * 1024;
const MAX_QUEUED_MESSAGES: usize = 100;

/// `_meta.ui` on a tool.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct UiMeta {
    pub resource_uri: String,
    /// `visibility: ["app"]`: only the app may call it, never the model.
    pub app_only: bool,
}
pub(crate) fn meta(item: &Value) -> Option<UiMeta> {
    let ui = item.get("_meta")?.get("ui")?;
    let resource_uri = ui.get("resourceUri")?.as_str()?;
    if !resource_uri.starts_with("ui://") {
        return None;
    }
    let visibility: Vec<&str> = ui
        .get("visibility")
        .and_then(Value::as_array)
        .map(|v| v.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    Some(UiMeta {
        resource_uri: resource_uri.into(),
        app_only: visibility == ["app"],
    })
}

/// A direct UI tool: runs the MCP tool, then opens or updates its app window
/// in the background so the result is not held up.
pub(super) struct UiTool {
    pub tool: McpTool,
    pub manager: std::sync::Weak<manager::Inner>,
}
#[async_trait]
impl AgentTool for UiTool {
    fn schema(&self) -> &Tool {
        &self.tool.schema
    }
    fn label(&self) -> &str {
        &self.tool.schema.name
    }
    async fn execute(
        &self,
        id: &str,
        params: Value,
        signal: CancellationToken,
        on_update: Arc<dyn Fn(ToolResultPartial) + Send + Sync>,
    ) -> Result<AgentToolResult, AgentError> {
        let result = self
            .tool
            .execute(id, params.clone(), signal, on_update)
            .await?;
        if let Some(inner) = self.manager.upgrade() {
            McpManager(inner).show_ui(&self.tool, params, &result);
        }
        Ok(result)
    }
}

/// The `CallToolResult` for the app: the raw result kept in details, or (when
/// details were summarized) the text the model saw.
fn call_result(result: &AgentToolResult) -> Value {
    if result.details["content"].is_array() {
        return result.details.clone();
    }
    let content: Vec<Value> = result
        .content
        .iter()
        .filter_map(|block| match block {
            TextContentOrImage::Text(text) => Some(json!({"type":"text","text":text.text})),
            TextContentOrImage::Image(_) => None,
        })
        .collect();
    json!({"content":content})
}

/// One open app window.
struct Session {
    server: String,
    tool: String,
    html: String,
    csp: String,
    /// Every event so far, replayed to a (re)connecting host page.
    log: Mutex<Vec<(String, Value)>>,
    live: tokio::sync::broadcast::Sender<(String, Value)>,
}
impl Session {
    fn push(&self, event: &str, data: Value) {
        let mut log = self.log.lock().unwrap();
        log.push((event.into(), data.clone()));
        let _ = self.live.send((event.into(), data));
    }
}

/// UI state owned by the manager: the loopback server, open sessions, the
/// queue the agent reads with `mcp({action: "ui-messages"})`, and consent.
#[derive(Default)]
pub(crate) struct Host {
    server: tokio::sync::Mutex<Option<(u16, String)>>,
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    messages: Mutex<Vec<Value>>,
    /// Per server: may its app call MCP tools (asked once per session)?
    consent: Mutex<HashMap<String, bool>>,
    /// Prompts from apps, for the TUI to submit as user messages.
    pub prompts: Option<tokio::sync::broadcast::Sender<String>>,
}

/// CSP for the app document from `_meta.ui.csp` (`connectDomains`, `resourceDomains`).
fn csp(meta: &Value) -> String {
    let list = |key: &str| -> String {
        meta[key]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            // Only plain origins: no spaces, quotes or semicolons that could add directives.
            .filter(|d| {
                !d.is_empty()
                    && !d.contains(|c: char| {
                        c.is_whitespace() || matches!(c, ';' | '\'' | '"' | ',')
                    })
            })
            .collect::<Vec<_>>()
            .join(" ")
    };
    let (connect, resource) = (list("connectDomains"), list("resourceDomains"));
    format!(
        "default-src 'none'; script-src 'unsafe-inline' {resource}; style-src 'unsafe-inline' {resource}; img-src data: blob: {resource}; font-src data: {resource}; media-src {resource}; connect-src {connect}; frame-src 'none'; form-action 'none'; base-uri 'none'"
    )
}

impl McpManager {
    /// After a successful call to a UI tool: push the result into its open
    /// window, or read its `ui://` resource and open a new one. Only in the TUI.
    pub(super) fn show_ui(&self, tool: &McpTool, args: Value, result: &AgentToolResult) {
        if tool.ui.is_none() || self.0.elicitation_ui.is_none() {
            return;
        }
        let (manager, tool, result) = (self.clone(), tool.clone(), call_result(result));
        tokio::spawn(async move { manager.open_ui(&tool, args, result).await });
    }

    async fn open_ui(&self, tool: &McpTool, args: Value, result: Value) {
        let (Some(ui), server) = (&tool.ui, tool.server.as_str()) else {
            return;
        };
        if let Err(error) = self.try_open_ui(server, tool, ui, args, result).await {
            tracing::warn!(%server, tool = %tool.remote_name, %error, "MCP UI not opened");
            let _ = self
                .0
                .elicitation_notices
                .send(format!("Could not open the {server} app: {error}"));
        }
    }

    async fn try_open_ui(
        &self,
        server: &str,
        tool: &McpTool,
        ui: &UiMeta,
        args: Value,
        result: Value,
    ) -> Result<(), String> {
        let host = &self.0.ui;
        let reuse = host
            .sessions
            .lock()
            .unwrap()
            .values()
            .find(|s| s.server == server && s.tool == tool.remote_name)
            .cloned();
        if let Some(session) = reuse {
            // Same tool again: update the open window instead of replacing it.
            session.push("tool-input", json!({"arguments":args}));
            session.push("tool-result", result);
            return Ok(());
        }
        let client = self
            .0
            .servers
            .lock()
            .unwrap()
            .get(server)
            .and_then(|s| s.client.clone())
            .ok_or("server is not connected")?;
        let read = client
            .request(
                "resources/read",
                json!({"uri":ui.resource_uri}),
                CancellationToken::new(),
                CALL_TIMEOUT,
            )
            .await?;
        let content = &read["contents"][0];
        let html = match (content["text"].as_str(), content["blob"].as_str()) {
            (Some(text), _) => text.to_owned(),
            (None, Some(blob)) => String::from_utf8(
                base64::engine::general_purpose::STANDARD
                    .decode(blob)
                    .map_err(|_| "UI resource is not valid base64")?,
            )
            .map_err(|_| "UI resource is not UTF-8")?,
            _ => return Err("UI resource has no content".into()),
        };
        if html.len() > MAX_BODY * 8 {
            return Err("UI resource exceeds 8 MiB".into());
        }
        let csp_meta = content["_meta"]["ui"]["csp"].clone();
        let (live, _) = tokio::sync::broadcast::channel(64);
        let session = Arc::new(Session {
            server: server.into(),
            tool: tool.remote_name.clone(),
            html,
            csp: csp(&csp_meta),
            log: Mutex::new(Vec::new()),
            live,
        });
        session.push("tool-input", json!({"arguments":args}));
        session.push("tool-result", result);
        let id = uuid::Uuid::new_v4().simple().to_string();
        let (port, token) = self.ui_server().await?;
        host.sessions.lock().unwrap().insert(id.clone(), session);
        let url = format!("http://127.0.0.1:{port}/s/{id}?t={token}");
        let viewer = std::env::var("ICERS_MCP_UI_VIEWER").unwrap_or_default();
        if matches!(viewer.as_str(), "none" | "off" | "disabled")
            || crate::codex_auth::open_browser(&url).is_err()
        {
            let _ = self
                .0
                .elicitation_notices
                .send(format!("The {server} app is ready at {url}"));
        }
        Ok(())
    }

    /// The loopback server's port and token, starting it on first use.
    async fn ui_server(&self) -> Result<(u16, String), String> {
        let mut server = self.0.ui.server.lock().await;
        if let Some(running) = server.as_ref() {
            return Ok(running.clone());
        }
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| e.to_string())?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let token = oauth_codex_token()?;
        let manager = Arc::downgrade(&self.0);
        let expected = token.clone();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let Some(inner) = manager.upgrade() else {
                    break;
                };
                let token = expected.clone();
                tokio::spawn(async move { McpManager(inner).serve_ui(stream, &token).await });
            }
        });
        *server = Some((port, token.clone()));
        Ok((port, token))
    }

    async fn serve_ui(&self, mut stream: TcpStream, token: &str) {
        let Some((request, body)) = read_request(&mut stream).await else {
            return respond(&mut stream, 400, "text/plain", "", "Bad request").await;
        };
        // Loopback Host only (DNS rebinding) and the session token on every route.
        if !host_allowed(request.headers.get("host").map(String::as_str))
            || !tokens_match(token, request.query.get("t").map(String::as_str))
        {
            return respond(&mut stream, 403, "text/plain", "", "Forbidden").await;
        }
        let mut parts = request.path.trim_start_matches('/').split('/');
        let (Some("s"), Some(id)) = (parts.next(), parts.next()) else {
            return respond(&mut stream, 404, "text/plain", "", "Not found").await;
        };
        let Some(session) = self.0.ui.sessions.lock().unwrap().get(id).cloned() else {
            return respond(
                &mut stream,
                404,
                "text/plain",
                "",
                "This app window was closed",
            )
            .await;
        };
        match (request.method.as_str(), parts.next()) {
            ("GET", None) => {
                let page = HOST_PAGE
                    .replace("{TOKEN}", token)
                    .replace(
                        "{TITLE}",
                        &html_escape(&format!("{} · {}", session.server, session.tool)),
                    )
                    .replace("{PROTOCOL}", PROTOCOL)
                    .replace("{VERSION}", env!("CARGO_PKG_VERSION"));
                respond(
                    &mut stream,
                    200,
                    "text/html; charset=utf-8",
                    "Content-Security-Policy: default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; frame-src 'self'; connect-src 'self'\r\n",
                    &page,
                )
                .await
            }
            ("GET", Some("app")) => {
                let header = format!("Content-Security-Policy: {}\r\n", session.csp);
                respond(
                    &mut stream,
                    200,
                    "text/html; charset=utf-8",
                    &header,
                    &session.html,
                )
                .await
            }
            ("GET", Some("events")) => self.stream_events(stream, &session).await,
            ("POST", Some("rpc")) => {
                let reply = self.app_request(&session, id, &body).await;
                respond(&mut stream, 200, "application/json", "", &reply.to_string()).await
            }
            ("POST", Some("message")) => {
                self.app_message(&session, id, &body);
                respond(&mut stream, 200, "application/json", "", "{}").await
            }
            ("POST", Some("complete")) => {
                if let Some(session) = self.0.ui.sessions.lock().unwrap().remove(id) {
                    session.push("close", json!({}));
                }
                respond(&mut stream, 200, "application/json", "", "{}").await
            }
            _ => respond(&mut stream, 404, "text/plain", "", "Not found").await,
        }
    }

    async fn stream_events(&self, mut stream: TcpStream, session: &Session) {
        let (replay, mut live) = {
            let log = session.log.lock().unwrap();
            (log.clone(), session.live.subscribe())
        };
        let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n";
        if stream.write_all(head.as_bytes()).await.is_err() {
            return;
        }
        for (event, data) in replay {
            if stream
                .write_all(sse_frame(&event, &data).as_bytes())
                .await
                .is_err()
            {
                return;
            }
        }
        while let Ok((event, data)) = live.recv().await {
            let close = event == "close";
            if stream
                .write_all(sse_frame(&event, &data).as_bytes())
                .await
                .is_err()
                || close
            {
                return;
            }
        }
    }

    /// `tools/call` and `resources/read` from the app, to its own server only.
    async fn app_request(&self, session: &Session, id: &str, body: &[u8]) -> Value {
        let error = |message: String| json!({"error":{"code":-32603,"message":message}});
        let Ok(request) = serde_json::from_slice::<Value>(body) else {
            return error("invalid request".into());
        };
        let params = request["params"].clone();
        let method = request["method"].as_str().unwrap_or_default();
        if !matches!(method, "tools/call" | "resources/read") {
            return json!({"error":{"code":-32601,"message":"Method not found"}});
        }
        if method == "tools/call" {
            if let Err(message) = self.app_consent(&session.server, id).await {
                return error(message);
            }
            let name = params["name"].as_str().unwrap_or_default();
            let tool = self
                .0
                .servers
                .lock()
                .unwrap()
                .get(&session.server)
                .and_then(|s| s.discovered.iter().find(|t| t.remote_name == name).cloned());
            let Some(tool) = tool else {
                return error(format!("{name} is not a tool of {}", session.server));
            };
            if tool.approval {
                let Some(ui) = self.0.approval_ui.clone() else {
                    return error("MCP tool approval denied".into());
                };
                if let Err(message) = approve(
                    &ui,
                    &format!("mcp-ui-{id}"),
                    &tool.schema.name,
                    &CancellationToken::new(),
                )
                .await
                {
                    return error(message);
                }
            }
        }
        let client = self
            .0
            .servers
            .lock()
            .unwrap()
            .get(&session.server)
            .and_then(|s| s.client.clone());
        let Some(client) = client else {
            return error(format!("{} is not connected", session.server));
        };
        match client
            .request(method, params, CancellationToken::new(), CALL_TIMEOUT)
            .await
        {
            Ok(result) => json!({"result":result}),
            Err(message) => error(message),
        }
    }

    /// Ask once per server whether its app may call MCP tools.
    async fn app_consent(&self, server: &str, id: &str) -> Result<(), String> {
        if let Some(allowed) = self.0.ui.consent.lock().unwrap().get(server) {
            return if *allowed {
                Ok(())
            } else {
                Err("The user did not allow this app to call tools".into())
            };
        }
        let ui = self
            .0
            .approval_ui
            .clone()
            .ok_or("The user did not allow this app to call tools")?;
        let allowed = confirm(
            &ui,
            &format!("mcp-ui-{id}"),
            &format!(
                "Allow the {} app to call its MCP tools?",
                clean_line(server)
            ),
            "The user did not allow this app to call tools",
            &CancellationToken::new(),
        )
        .await;
        self.0
            .ui
            .consent
            .lock()
            .unwrap()
            .insert(server.to_owned(), allowed.is_ok());
        allowed
    }

    /// `ui/message` and `ui/update-model-context`: queued for the agent; a
    /// prompt also goes to the TUI to start or steer a turn.
    fn app_message(&self, session: &Session, id: &str, body: &[u8]) {
        let Ok(request) = serde_json::from_slice::<Value>(body) else {
            return;
        };
        let text: String = request["params"]["content"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|block| block["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let kind = match request["method"].as_str() {
            Some("ui/message") => "prompt",
            Some("ui/update-model-context") => "context",
            _ => return,
        };
        let entry = json!({
            "type": kind,
            "sessionId": id,
            "serverName": session.server,
            "toolName": session.tool,
            "timestamp": std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_millis() as u64),
            "prompt": text,
            "params": request["params"],
        });
        {
            let mut messages = self.0.ui.messages.lock().unwrap();
            messages.push(entry);
            let excess = messages.len().saturating_sub(MAX_QUEUED_MESSAGES);
            messages.drain(..excess);
        }
        if kind == "prompt" && !text.trim().is_empty() {
            if let Some(prompts) = &self.0.ui.prompts {
                let _ = prompts.send(format!(
                    "[{} app] {}",
                    clean_line(&session.server),
                    text.chars()
                        .filter(|c| !c.is_control() || *c == '\n')
                        .collect::<String>()
                ));
            }
        }
    }

    /// Drain queued app messages for `mcp({action: "ui-messages"})`.
    pub(super) fn take_ui_messages(&self) -> Vec<Value> {
        std::mem::take(&mut *self.0.ui.messages.lock().unwrap())
    }

    /// App prompts for the TUI to submit; `None` when the manager has no TUI.
    pub fn ui_prompts(&self) -> Option<tokio::sync::broadcast::Receiver<String>> {
        self.0.ui.prompts.as_ref().map(|p| p.subscribe())
    }
}

fn oauth_codex_token() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).map_err(|e| e.to_string())?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

fn clean_line(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).collect()
}

fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

async fn read_request(
    stream: &mut TcpStream,
) -> Option<(crate::observe::server::Request, Vec<u8>)> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        if let Some(at) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
            break at;
        }
        if buffer.len() > MAX_HEAD {
            return None;
        }
        let read = tokio::time::timeout(Duration::from_secs(10), stream.read(&mut chunk))
            .await
            .ok()?
            .ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
    };
    let request = parse_request(std::str::from_utf8(&buffer[..head_end]).ok()?)?;
    let length: usize = request
        .headers
        .get("content-length")
        .map_or(Some(0), |v| v.parse().ok())?;
    if length > MAX_BODY {
        return None;
    }
    let mut body = buffer[head_end + 4..].to_vec();
    while body.len() < length {
        let read = tokio::time::timeout(Duration::from_secs(10), stream.read(&mut chunk))
            .await
            .ok()?
            .ok()?;
        if read == 0 {
            return None;
        }
        body.extend_from_slice(&chunk[..read]);
    }
    body.truncate(length);
    Some((request, body))
}

async fn respond(stream: &mut TcpStream, status: u16, kind: &str, headers: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status} X\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\n{headers}Connection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.write_all(body.as_bytes()).await;
}

/// The host side of the MCP Apps postMessage protocol. The app runs in a
/// sandboxed iframe (no same-origin), so it can reach icers only through here.
const HOST_PAGE: &str = r#"<!doctype html>
<html><head><meta charset="utf-8"><title>{TITLE}</title>
<style>
body{margin:0;font:14px system-ui,sans-serif;display:flex;flex-direction:column;height:100vh}
header{display:flex;gap:8px;align-items:center;padding:6px 10px;border-bottom:1px solid #8884}
header span{flex:1;opacity:.8}
iframe{flex:1;border:0;width:100%}
</style></head>
<body><header><span>{TITLE}</span><button id="done">Done</button><button id="cancel">Cancel</button></header>
<iframe id="app" sandbox="allow-scripts allow-forms"></iframe>
<script>
const token = "{TOKEN}", base = location.pathname, frame = document.getElementById("app");
const post = (path, body) => fetch(`${base}/${path}?t=${token}`, {method: "POST", headers: {"content-type": "application/json"}, body: JSON.stringify(body)}).then(r => r.json());
const send = message => frame.contentWindow && frame.contentWindow.postMessage(message, "*");
let ready = false;
const waiting = [];
const notify = (method, params) => { const m = {jsonrpc: "2.0", method, params}; ready ? send(m) : waiting.push(m); };
window.addEventListener("message", async event => {
  if (event.source !== frame.contentWindow) return;
  const m = event.data;
  if (!m || m.jsonrpc !== "2.0" || typeof m.method !== "string") return;
  const reply = (result, error) => { if (m.id !== undefined) send(error ? {jsonrpc: "2.0", id: m.id, error} : {jsonrpc: "2.0", id: m.id, result}); };
  try {
    switch (m.method) {
      case "ui/initialize":
        reply({protocolVersion: "{PROTOCOL}", hostInfo: {name: "icers", version: "{VERSION}"},
          hostCapabilities: {openLinks: {}, serverTools: {}, serverResources: {}, logging: {}},
          hostContext: {theme: matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light", displayMode: "inline", availableDisplayModes: ["inline"]}});
        break;
      case "ui/notifications/initialized":
        ready = true; waiting.splice(0).forEach(send); break;
      case "tools/call": case "resources/read": {
        const r = await post("rpc", {method: m.method, params: m.params});
        r.error ? reply(null, r.error) : reply(r.result); break;
      }
      case "ui/message": case "ui/update-model-context":
        await post("message", {method: m.method, params: m.params}); reply({}); break;
      case "ui/open-link": {
        const url = new URL(m.params && m.params.url);
        if (url.protocol === "https:" || url.protocol === "http:") window.open(url.href, "_blank", "noopener,noreferrer");
        reply({}); break;
      }
      case "ui/request-display-mode": reply({mode: "inline"}); break;
      case "ui/notifications/size-changed":
        if (m.params && m.params.height > 0) frame.style.height = Math.min(m.params.height, 4000) + "px";
        break;
      case "notifications/message": break;
      default: reply(null, {code: -32601, message: "Method not found"});
    }
  } catch (error) { reply(null, {code: -32603, message: String(error)}); }
});
frame.src = `${base}/app?t=${token}`;
const events = new EventSource(`${base}/events?t=${token}`);
for (const kind of ["tool-input", "tool-result", "tool-cancelled"])
  events.addEventListener(kind, e => notify(`ui/notifications/${kind}`, JSON.parse(e.data)));
events.addEventListener("close", () => { events.close(); document.body.textContent = "This app window was closed."; });
const complete = async reason => {
  send({jsonrpc: "2.0", id: "teardown", method: "ui/resource-teardown", params: {}});
  await post("complete", {reason}).catch(() => {});
  events.close(); document.body.textContent = "Closed. You can close this tab.";
};
document.getElementById("done").onclick = () => complete("done");
document.getElementById("cancel").onclick = () => complete("cancel");
</script></body></html>
"#;

#[cfg(test)]
#[path = "ui_tests.rs"]
mod tests;
