//! Session-local stdio MCP client. No network transports or extension ABI.
mod elicitation;
mod elicitation_schema;
mod import;
mod manager;
mod output_guard;
mod prompts;
mod sampling;
pub mod setup;
pub use prompts::Prompt;
#[cfg(test)]
mod prompt_tests;
#[cfg(test)]
mod resource_tests;
#[cfg(test)]
mod tests;
pub use manager::{ensure_config, McpManager, PanelChange, ServerStatus, ServerView, ToolInfo};
use output_guard::OutputGuard;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use futures::future::BoxFuture;
use ice_agent::agent_tool::AgentTool;
use ice_agent::types::{AgentToolResult, TextContentOrImage, ToolResultPartial};
use ice_agent::AgentError;
use ice_ai::types::{ImageContent, Tool};
use ice_extensions::UiDialogMailbox;
use ice_harness::agent_harness::AgentHarness;
use ice_harness::types::{HarnessLifecycleHooks, HarnessTool};
use ice_plugin_sdk::process::ProcessTree;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot, watch, Semaphore};
use tokio_util::sync::CancellationToken;

const START_TIMEOUT: Duration = Duration::from_secs(10);
const CALL_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_LINE: u64 = 16 * 1024 * 1024;
/// Bounds dialogs or model calls a single server can have waiting on this client.
const MAX_INBOUND: usize = 8;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(untagged)]
pub enum Approval {
    All(bool),
    Tools(Vec<String>),
    #[default]
    None,
}
impl Approval {
    fn requires(&self, name: &str) -> bool {
        match self {
            Self::All(value) => *value,
            Self::Tools(names) => names.iter().any(|n| n == name),
            Self::None => false,
        }
    }
}

/// Which discovered tools register with the agent. Same `directTools` key and
/// shape as pi-mcp-adapter; absent means every tool, since there is no proxy tool.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum DirectTools {
    All(bool),
    Tools(Vec<String>),
}
impl Default for DirectTools {
    fn default() -> Self {
        Self::All(true)
    }
}
impl DirectTools {
    pub fn allows(&self, name: &str) -> bool {
        match self {
            Self::All(value) => *value,
            Self::Tools(names) => names.iter().any(|n| n == name),
        }
    }
    fn to_json(&self) -> Value {
        match self {
            Self::All(value) => json!(value),
            Self::Tools(names) => json!(names),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerConfig {
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    pub cwd: Option<PathBuf>,
    #[serde(default)]
    pub disabled: bool,
    #[serde(default)]
    pub approve_tools: Approval,
    #[serde(default)]
    pub direct_tools: DirectTools,
    #[serde(default = "expose_resources_default")]
    pub expose_resources: bool,
}
fn expose_resources_default() -> bool {
    true
}
#[derive(Default, Deserialize)]
struct Config {
    #[serde(default, rename = "mcpServers")]
    servers: BTreeMap<String, ServerConfig>,
    /// Other tools whose stdio servers are imported as the lowest layer.
    #[serde(default)]
    imports: Vec<String>,
    /// Read leniently: a malformed value falls back to defaults, never rejects the file.
    #[serde(default)]
    settings: Value,
}

/// Global configuration first, then the project file only when trusted.
fn read_configs(
    agent_dir: &Path,
    cwd: &Path,
    trusted: bool,
) -> Result<Vec<(PathBuf, Config)>, String> {
    let mut paths = vec![agent_dir.join("mcp.json")];
    if trusted {
        paths.push(cwd.join(".icers/mcp.json"));
    }
    let mut configs = Vec::new();
    for path in paths {
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        let config = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        configs.push((path, config));
    }
    Ok(configs)
}

/// Top-level `settings`, merged per key; a trusted project file wins.
fn load_settings(
    agent_dir: &Path,
    cwd: &Path,
    trusted: bool,
) -> Result<serde_json::Map<String, Value>, String> {
    let mut settings = serde_json::Map::new();
    for (_, config) in read_configs(agent_dir, cwd, trusted)? {
        if let Value::Object(values) = config.settings {
            settings.extend(values);
        }
    }
    Ok(settings)
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Effective servers, each with the file that provides it, plus import problems.
pub(crate) struct Layered {
    pub servers: BTreeMap<String, (ServerConfig, PathBuf)>,
    pub notes: Vec<String>,
    /// Entries replaced by a higher layer or an earlier import, lowest first.
    pub shadowed: Vec<(String, PathBuf)>,
}

/// Imports are the lowest layer, then global, then the project file only when trusted.
/// Whole server entries are replaced by name, never merged.
pub(crate) fn load_layered(agent_dir: &Path, cwd: &Path, trusted: bool) -> Result<Layered, String> {
    load_layered_from(dirs::home_dir().as_deref(), agent_dir, cwd, trusted)
}
fn load_layered_from(
    home: Option<&Path>,
    agent_dir: &Path,
    cwd: &Path,
    trusted: bool,
) -> Result<Layered, String> {
    let configs = read_configs(agent_dir, cwd, trusted)?;
    let kinds: Vec<String> = configs
        .iter()
        .flat_map(|(_, config)| config.imports.iter().cloned())
        .collect();
    let imported = import::load(&kinds, home, cwd, trusted);
    let mut servers = imported.servers;
    let mut shadowed = imported.shadowed;
    for (path, config) in configs {
        for (name, server) in config.servers {
            if !valid_name(&name) {
                return Err(format!("invalid MCP server name: {name:?}"));
            }
            if let Some((_, old)) = servers.insert(name.clone(), (server, path.clone())) {
                shadowed.push((name, old));
            }
        }
    }
    Ok(Layered {
        servers,
        notes: imported.notes,
        shadowed,
    })
}

pub fn load_config(
    agent_dir: &Path,
    cwd: &Path,
    trusted: bool,
) -> Result<BTreeMap<String, ServerConfig>, String> {
    Ok(load_layered(agent_dir, cwd, trusted)?
        .servers
        .into_iter()
        .map(|(name, (server, _))| (name, server))
        .collect())
}

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>;

/// JSON-RPC error returned to the server for one of its requests.
#[derive(Debug)]
pub(crate) struct RpcError {
    pub code: i64,
    pub message: String,
}
type RequestHandler = Arc<
    dyn Fn(Value, CancellationToken) -> BoxFuture<'static, Result<Value, RpcError>> + Send + Sync,
>;

/// Server-to-client handling. A capability is advertised only for a registered method.
#[derive(Clone, Default)]
pub(crate) struct Handlers {
    requests: BTreeMap<&'static str, (Value, RequestHandler)>,
    /// Receives the catalog (`tools`, `prompts`, `resources`) named by a list_changed notification.
    list_changed: Option<Arc<dyn Fn(&str) + Send + Sync>>,
    elicitation_complete: Option<Arc<dyn Fn(Value) + Send + Sync>>,
}
impl Handlers {
    /// `method` is e.g. `sampling/createMessage`; its prefix names the advertised capability.
    #[cfg_attr(not(test), allow(dead_code))] // Registered by the sampling and elicitation ports.
    pub fn request(
        mut self,
        method: &'static str,
        capability: Value,
        handler: RequestHandler,
    ) -> Self {
        self.requests.insert(method, (capability, handler));
        self
    }
    fn capabilities(&self) -> Value {
        let mut capabilities = serde_json::Map::new();
        for (method, (capability, _)) in &self.requests {
            let name = method.split('/').next().unwrap_or(method);
            capabilities.insert(name.into(), capability.clone());
        }
        Value::Object(capabilities)
    }
}

enum Inbound {
    CancelHandlers,
    Shutdown,
    Request {
        id: Value,
        method: String,
        params: Value,
    },
    Notification {
        method: String,
        params: Value,
    },
}

/// What the server declared in its `initialize` result.
struct ServerInit {
    capabilities: Value,
    instructions: Option<String>,
}

struct Process {
    child: Child,
    tree: ProcessTree,
}
impl Drop for Process {
    fn drop(&mut self) {
        self.tree.kill();
        let _ = self.child.wait();
    }
}

struct Client {
    outgoing: std::sync::mpsc::Sender<Value>,
    inbound: mpsc::UnboundedSender<Inbound>,
    pending: Pending,
    next_id: AtomicU64,
    closed: Arc<AtomicBool>,
    capabilities: Value,
    init: OnceLock<ServerInit>,
    _process: Process,
}
impl Client {
    fn spawn(
        name: &str,
        config: &ServerConfig,
        cwd: &Path,
        handlers: Handlers,
    ) -> Result<Arc<Self>, String> {
        if config.command.is_empty() {
            return Err("missing MCP command".into());
        }
        let runtime = tokio::runtime::Handle::try_current()
            .map_err(|_| "MCP client requires a Tokio runtime")?;
        let mut command = Command::new(&config.command);
        command
            .args(&config.args)
            .envs(&config.env)
            .current_dir(
                config
                    .cwd
                    .as_ref()
                    .map(|p| cwd.join(p))
                    .unwrap_or_else(|| cwd.to_path_buf()),
            )
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        ProcessTree::prepare(&mut command);
        let mut child = command.spawn().map_err(|e| format!("start {name}: {e}"))?;
        let tree = match ProcessTree::attach(&child) {
            Ok(tree) => tree,
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e);
            }
        };
        let mut process = Process { child, tree };
        let mut stdin = process.child.stdin.take().ok_or("missing stdin")?;
        let stdout = process.child.stdout.take().ok_or("missing stdout")?;
        let stderr = process.child.stderr.take().ok_or("missing stderr")?;
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let (outgoing, receiver) = std::sync::mpsc::channel::<Value>();
        let closed = Arc::new(AtomicBool::new(false));
        let writer_closed = closed.clone();
        let reader_closed = closed.clone();
        let writer_pending = pending.clone();
        std::thread::spawn(move || {
            for value in receiver {
                if writeln!(stdin, "{value}")
                    .and_then(|_| stdin.flush())
                    .is_err()
                {
                    writer_closed.store(true, Ordering::SeqCst);
                    fail_pending(&writer_pending, "MCP stdin closed");
                    break;
                }
            }
        });
        let capabilities = handlers.capabilities();
        let (inbound, inbound_rx) = mpsc::unbounded_channel();
        runtime.spawn(dispatch(inbound_rx, outgoing.clone(), handlers));
        let client_inbound = inbound.clone();
        let reader_pending = pending.clone();
        let reader_outgoing = outgoing.clone();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                // Bound individual protocol frames, including a server that never sends a newline.
                let mut line = Vec::new();
                let read =
                    std::io::Read::take(&mut reader, MAX_LINE + 1).read_until(b'\n', &mut line);
                if !matches!(read, Ok(n) if n > 0 && n as u64 <= MAX_LINE) {
                    break;
                }
                let value: Value = match serde_json::from_slice(&line) {
                    Ok(value) => value,
                    Err(_) => break,
                };
                if value.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
                    break;
                }
                if let Some(method) = value.get("method") {
                    let method = method.as_str().unwrap_or_default().to_owned();
                    let params = value.get("params").cloned().unwrap_or(Value::Null);
                    let sent = match value.get("id") {
                        // Answered here so a busy dispatcher can never delay liveness checks.
                        Some(id) if method == "ping" => reader_outgoing
                            .send(json!({"jsonrpc":"2.0","id":id,"result":{}}))
                            .is_ok(),
                        Some(id) => inbound
                            .send(Inbound::Request {
                                id: id.clone(),
                                method,
                                params,
                            })
                            .is_ok(),
                        None => inbound
                            .send(Inbound::Notification { method, params })
                            .is_ok(),
                    };
                    if !sent {
                        break;
                    }
                    continue;
                }
                if let Some(id) = value.get("id").and_then(Value::as_u64) {
                    if let Some(reply) = reader_pending.lock().unwrap().remove(&id) {
                        let result = if let Some(error) = value.get("error") {
                            Err(format!("MCP RPC error: {error}"))
                        } else {
                            value
                                .get("result")
                                .cloned()
                                .ok_or_else(|| "MCP response has no result".into())
                        };
                        let _ = reply.send(result);
                    }
                }
            }
            reader_closed.store(true, Ordering::SeqCst);
            let _ = inbound.send(Inbound::Shutdown);
            fail_pending(
                &reader_pending,
                "MCP server closed or sent an invalid frame",
            );
        });
        let server = name.to_owned();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stderr);
            loop {
                let mut line = Vec::new();
                match std::io::Read::take(&mut reader, 8192).read_until(b'\n', &mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        tracing::debug!(server = %server, stderr = %String::from_utf8_lossy(&line), "MCP stderr")
                    }
                }
            }
        });
        Ok(Arc::new(Self {
            outgoing,
            inbound: client_inbound,
            pending,
            next_id: AtomicU64::new(1),
            closed,
            capabilities,
            init: OnceLock::new(),
            _process: process,
        }))
    }

    fn instructions(&self) -> Option<&str> {
        self.init.get()?.instructions.as_deref()
    }

    /// Spec: list_changed is only meaningful for a catalog the server declared.
    fn declares(&self, catalog: &str) -> bool {
        self.init
            .get()
            .is_some_and(|init| init.capabilities.get(catalog).is_some_and(Value::is_object))
    }

    fn notify(&self, method: &str, params: Value) {
        let _ = self
            .outgoing
            .send(json!({"jsonrpc":"2.0", "method":method, "params":params}));
    }

    async fn request(
        &self,
        method: &str,
        params: Value,
        signal: CancellationToken,
        timeout: Duration,
    ) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().unwrap();
            if self.closed.load(Ordering::SeqCst) {
                return Err("MCP server disconnected".into());
            }
            pending.insert(id, tx);
        }
        // Drop also handles the caller dropping this future before it observes cancellation.
        let mut guard = RequestGuard {
            client: self,
            id,
            completed: false,
            sent: false,
        };
        if signal.is_cancelled() {
            return Err("MCP call cancelled".into());
        }
        self.outgoing
            .send(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))
            .map_err(|_| "MCP writer closed")?;
        guard.sent = true;
        let result = tokio::select! {
            biased;
            _ = signal.cancelled() => Err("MCP call cancelled".into()),
            _ = tokio::time::sleep(timeout) => Err("MCP request timed out".into()),
            reply = rx => {
                guard.completed = true;
                reply.unwrap_or_else(|_| Err("MCP server disconnected".into()))
            }
        };
        result
    }

    async fn discover(
        self: &Arc<Self>,
        server: &str,
        config: &ServerConfig,
        guard: &OutputGuard,
    ) -> Result<Vec<McpTool>, String> {
        let initialized = self
            .request(
                "initialize",
                json!({
                    "protocolVersion":"2025-06-18", "capabilities":self.capabilities,
                    "clientInfo":{"name":"icers","version":crate::VERSION}
                }),
                CancellationToken::new(),
                START_TIMEOUT,
            )
            .await?;
        if initialized
            .get("protocolVersion")
            .and_then(Value::as_str)
            .is_none()
        {
            return Err("MCP initialize missing protocolVersion".into());
        }
        let _ = self.init.set(ServerInit {
            capabilities: initialized
                .get("capabilities")
                .cloned()
                .unwrap_or_else(|| json!({})),
            instructions: initialized
                .get("instructions")
                .and_then(Value::as_str)
                .map(str::to_owned),
        });
        self.notify("notifications/initialized", json!({}));
        let mut tools = if self.declares("tools") {
            self.list_tools(server, config, guard).await?
        } else {
            Vec::new()
        };
        tools.extend(self.list_resources(server, config, guard).await?);
        Ok(tools)
    }

    async fn list_resources(
        self: &Arc<Self>,
        server: &str,
        config: &ServerConfig,
        guard: &OutputGuard,
    ) -> Result<Vec<McpTool>, String> {
        if !config.expose_resources || !self.declares("resources") {
            return Ok(Vec::new());
        }
        let mut cursor = None;
        let mut cursors = HashSet::new();
        let mut tools = Vec::new();
        for _ in 0..100 {
            let params = cursor
                .as_ref()
                .map(|c| json!({"cursor":c}))
                .unwrap_or_else(|| json!({}));
            let page = self
                .request(
                    "resources/list",
                    params,
                    CancellationToken::new(),
                    START_TIMEOUT,
                )
                .await?;
            for item in page
                .get("resources")
                .and_then(Value::as_array)
                .ok_or("MCP resources/list missing resources")?
            {
                let name = item
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or("MCP resource missing name")?;
                let uri = item
                    .get("uri")
                    .and_then(Value::as_str)
                    .ok_or("MCP resource missing URI")?;
                let name = format!("read_{}", resource_tool_name(name));
                let description = item
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or("Read MCP resource");
                tools.push(McpTool {
                    schema: Tool {
                        name: format!("{server}_{name}"),
                        description: description.chars().filter(|c| !c.is_control()).collect(),
                        parameters: serde_json::from_value(
                            json!({"type":"object","properties":{},"additionalProperties":false}),
                        )
                        .unwrap(),
                        constrained_sampling: None,
                    },
                    remote_name: name.clone(),
                    // Resource tools are not listed in the `/mcp` catalog.
                    tokens: 0,
                    resource_uri: Some(uri.to_owned()),
                    client: self.clone(),
                    approval: config.approve_tools.requires(&name),
                    guard: guard.clone(),
                });
            }
            cursor = page
                .get("nextCursor")
                .and_then(Value::as_str)
                .map(str::to_owned);
            match &cursor {
                Some(cursor) if !cursors.insert(cursor.clone()) => {
                    return Err("MCP repeated resource pagination cursor".into())
                }
                None => return Ok(tools),
                _ => {}
            }
        }
        Err("MCP resources/list exceeded 100 pages".into())
    }

    async fn list_tools(
        self: &Arc<Self>,
        server: &str,
        config: &ServerConfig,
        guard: &OutputGuard,
    ) -> Result<Vec<McpTool>, String> {
        let signal = CancellationToken::new();
        let mut cursor = None;
        let mut cursors = HashSet::new();
        let mut names = HashSet::new();
        let mut tools = Vec::new();
        loop {
            let params = cursor
                .as_ref()
                .map(|c| json!({"cursor":c}))
                .unwrap_or_else(|| json!({}));
            let page = self
                .request("tools/list", params, signal.clone(), START_TIMEOUT)
                .await?;
            for item in page
                .get("tools")
                .and_then(Value::as_array)
                .ok_or("MCP tools/list missing tools")?
            {
                let name = item
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or("MCP tool missing name")?;
                if name.is_empty()
                    || !name
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                    || !names.insert(name.to_owned())
                {
                    return Err(format!("invalid or duplicate MCP tool name: {name}"));
                }
                let schema = item
                    .get("inputSchema")
                    .cloned()
                    .ok_or("MCP tool missing inputSchema")?;
                let description = item.get("description").and_then(Value::as_str);
                // Same rough estimate as pi-mcp-adapter's panel.
                let tokens =
                    (name.len() + description.map_or(0, str::len) + schema.to_string().len())
                        .div_ceil(4)
                        + 10;
                tools.push(McpTool {
                    schema: Tool {
                        name: format!("{server}_{name}"),
                        description: description.unwrap_or("MCP tool").into(),
                        parameters: serde_json::from_value(schema)
                            .map_err(|e| format!("invalid MCP inputSchema: {e}"))?,
                        constrained_sampling: None,
                    },
                    remote_name: name.into(),
                    tokens,
                    resource_uri: None,
                    client: self.clone(),
                    approval: config.approve_tools.requires(name),
                    guard: guard.clone(),
                });
            }
            cursor = page
                .get("nextCursor")
                .and_then(Value::as_str)
                .map(str::to_owned);
            if let Some(c) = &cursor {
                if !cursors.insert(c.clone()) {
                    return Err("MCP repeated pagination cursor".into());
                }
            } else {
                break;
            }
        }
        Ok(tools)
    }
}
/// Runs each server request in its own task so a slow dialog never blocks other traffic.
async fn dispatch(
    mut inbound: mpsc::UnboundedReceiver<Inbound>,
    outgoing: std::sync::mpsc::Sender<Value>,
    handlers: Handlers,
) {
    let in_flight: Arc<Mutex<HashMap<String, CancellationToken>>> = Default::default();
    let limit = Arc::new(Semaphore::new(MAX_INBOUND));
    // Reader exit (server gone) ends the loop and cancels every handler still running.
    let root = CancellationToken::new();
    let _cancel_all = root.clone().drop_guard();
    let reply = |outgoing: &std::sync::mpsc::Sender<Value>, id: Value, result| {
        let _ = outgoing.send(match result {
            Ok(result) => json!({"jsonrpc":"2.0","id":id,"result":result}),
            Err(RpcError { code, message }) => {
                json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
            }
        });
    };
    while let Some(message) = inbound.recv().await {
        match message {
            Inbound::Shutdown => break,
            Inbound::CancelHandlers => {
                for token in in_flight.lock().unwrap().values() {
                    token.cancel();
                }
            }
            Inbound::Notification { method, params } => match method.as_str() {
                "notifications/cancelled" => {
                    let key = params.get("requestId").map(Value::to_string);
                    if let Some(token) =
                        key.and_then(|key| in_flight.lock().unwrap().get(&key).cloned())
                    {
                        token.cancel();
                    }
                }
                "notifications/tools/list_changed"
                | "notifications/prompts/list_changed"
                | "notifications/resources/list_changed" => {
                    if let Some(callback) = &handlers.list_changed {
                        callback(method.split('/').nth(1).unwrap_or_default());
                    }
                }
                "notifications/elicitation/complete" => {
                    if let Some(callback) = &handlers.elicitation_complete {
                        callback(params);
                    }
                }
                _ => tracing::debug!(%method, "ignored MCP notification"),
            },
            Inbound::Request { id, method, params } => {
                let Some((_, handler)) = handlers.requests.get(method.as_str()) else {
                    reply(
                        &outgoing,
                        id,
                        Err(RpcError {
                            code: -32601,
                            message: "Method not found".into(),
                        }),
                    );
                    continue;
                };
                let Ok(permit) = limit.clone().try_acquire_owned() else {
                    reply(
                        &outgoing,
                        id,
                        Err(RpcError {
                            code: -32603,
                            message: "Too many concurrent requests".into(),
                        }),
                    );
                    continue;
                };
                let key = id.to_string();
                let token = root.child_token();
                {
                    let mut in_flight = in_flight.lock().unwrap();
                    if in_flight.contains_key(&key) {
                        reply(
                            &outgoing,
                            id,
                            Err(RpcError {
                                code: -32600,
                                message: "Duplicate request id".into(),
                            }),
                        );
                        continue;
                    }
                    in_flight.insert(key.clone(), token.clone());
                }
                let future = handler(params, token.clone());
                let outgoing = outgoing.clone();
                let in_flight = in_flight.clone();
                tokio::spawn(async move {
                    let _permit = permit;
                    // A cancelled request gets no response, per the MCP cancellation rules.
                    let result = tokio::select! {
                        biased;
                        _ = token.cancelled() => None,
                        result = future => Some(result),
                    };
                    in_flight.lock().unwrap().remove(&key);
                    if let Some(result) = result {
                        reply(&outgoing, id, result);
                    }
                });
            }
        }
    }
}
fn fail_pending(pending: &Pending, reason: &str) {
    for (_, tx) in pending.lock().unwrap().drain() {
        let _ = tx.send(Err(reason.into()));
    }
}
struct RequestGuard<'a> {
    client: &'a Client,
    id: u64,
    completed: bool,
    sent: bool,
}
impl Drop for RequestGuard<'_> {
    fn drop(&mut self) {
        self.client.pending.lock().unwrap().remove(&self.id);
        if self.sent && !self.completed {
            // Servers need not cooperate with cancellation; never leave their
            // outstanding questions alive after an aborted/timed-out call.
            let _ = self.client.inbound.send(Inbound::CancelHandlers);
            self.client.notify(
                "notifications/cancelled",
                json!({"requestId":self.id,"reason":"client cancelled or timed out"}),
            );
        }
    }
}

fn resource_tool_name(name: &str) -> String {
    let mut sanitized = String::new();
    for part in name
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|s| !s.is_empty())
    {
        if !sanitized.is_empty() {
            sanitized.push('_');
        }
        sanitized.push_str(&part.to_ascii_lowercase());
    }
    if sanitized.is_empty() || sanitized.starts_with(|c: char| c.is_ascii_digit()) {
        if !sanitized.is_empty() {
            sanitized.insert(0, '_');
        }
        sanitized.insert_str(0, "resource");
    }
    sanitized
}

#[derive(Clone)]
struct McpTool {
    schema: Tool,
    remote_name: String,
    tokens: usize,
    resource_uri: Option<String>,
    client: Arc<Client>,
    approval: bool,
    guard: OutputGuard,
}
#[async_trait]
impl AgentTool for McpTool {
    fn schema(&self) -> &Tool {
        &self.schema
    }
    fn label(&self) -> &str {
        &self.schema.name
    }
    async fn execute(
        &self,
        _: &str,
        params: Value,
        signal: CancellationToken,
        _: Arc<dyn Fn(ToolResultPartial) + Send + Sync>,
    ) -> Result<AgentToolResult, AgentError> {
        let (method, params) = match &self.resource_uri {
            Some(uri) => ("resources/read", json!({"uri":uri})),
            None => (
                "tools/call",
                json!({"name":self.remote_name,"arguments":params}),
            ),
        };
        let mut result = self
            .client
            .request(method, params, signal, CALL_TIMEOUT)
            .await
            .map_err(AgentError::Tool)?;
        if self.resource_uri.is_some() {
            let contents = result
                .get_mut("contents")
                .and_then(Value::as_array_mut)
                .ok_or_else(|| AgentError::Tool("MCP resources/read missing contents".into()))?;
            let content: Vec<Value> = std::mem::take(contents).into_iter().map(|resource| {
                if resource["blob"].is_string() && resource["mimeType"].as_str().is_some_and(|mime| mime.starts_with("image/")) {
                    json!({"type":"image","data":resource["blob"],"mimeType":resource["mimeType"]})
                } else {
                    json!({"type":"resource","resource":resource})
                }
            }).collect();
            result.as_object_mut().unwrap().remove("contents");
            result["content"] = json!(content);
        }
        map_result(result, &self.guard)
    }
}
fn map_result(mut result: Value, guard: &OutputGuard) -> Result<AgentToolResult, AgentError> {
    if result.get("isError").and_then(Value::as_bool) == Some(true) {
        return Err(AgentError::Tool(
            guard.bound_text(format!("MCP tool error: {result}")),
        ));
    }
    let mut content = Vec::new();
    for item in result
        .get_mut("content")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| AgentError::Tool("MCP result missing content".into()))?
    {
        let kind = item.get("type").and_then(Value::as_str).map(str::to_owned);
        let block = match kind.as_deref() {
            Some("text") => {
                TextContentOrImage::text(item.get("text").and_then(Value::as_str).unwrap_or(""))
            }
            Some("image") => TextContentOrImage::Image(ImageContent {
                kind: Default::default(),
                data: item
                    .get("data")
                    .and_then(Value::as_str)
                    .ok_or_else(|| AgentError::Tool("MCP image missing data".into()))?
                    .into(),
                mime_type: item
                    .get("mimeType")
                    .and_then(Value::as_str)
                    .ok_or_else(|| AgentError::Tool("MCP image missing mimeType".into()))?
                    .into(),
            }),
            Some("resource") if item["resource"]["blob"].is_string() => {
                TextContentOrImage::text(guard.materialize_blob(&mut item["resource"]))
            }
            Some("resource") if item["resource"]["text"].is_string() => {
                TextContentOrImage::text(item["resource"]["text"].as_str().unwrap())
            }
            _ => TextContentOrImage::text(item.to_string()),
        };
        content.push(block);
    }
    Ok(AgentToolResult {
        content: guard.bound_content(content),
        details: guard.bound_details(result),
        ..Default::default()
    })
}

/// Gate prompt admission, not TUI startup, until discovery has published tools.
pub struct McpReadyGate {
    pub inner: Option<Arc<dyn HarnessLifecycleHooks>>,
    pub ready: watch::Receiver<bool>,
}

impl McpReadyGate {
    async fn wait_ready(&self, budget: Duration) {
        let mut ready = self.ready.clone();
        let _ = tokio::time::timeout(budget, async {
            while !*ready.borrow_and_update() {
                if ready.changed().await.is_err() {
                    break;
                }
            }
        })
        .await;
    }
}

#[async_trait]
impl HarnessLifecycleHooks for McpReadyGate {
    async fn input(
        &self,
        message: ice_agent::AgentMessage,
    ) -> ice_harness::result::HarnessResult<ice_agent::AgentMessage> {
        self.wait_ready(START_TIMEOUT + Duration::from_secs(1))
            .await;
        match &self.inner {
            Some(inner) => inner.input(message).await,
            None => Ok(message),
        }
    }

    async fn pre_compact(&self, trigger: &str) {
        if let Some(inner) = &self.inner {
            inner.pre_compact(trigger).await;
        }
    }

    async fn stop(&self) {
        if let Some(inner) = &self.inner {
            inner.stop().await;
        }
    }
}

/// Startup is asynchronous; each server gets a total ten-second discovery budget.
pub fn start(manager: McpManager) -> watch::Receiver<bool> {
    let (ready_tx, ready) = watch::channel(false);
    tokio::spawn(async move {
        if let Err(error) = manager.reconnect(None).await {
            tracing::warn!(%error, "MCP configuration failed");
        }
        let _ = ready_tx.send(true);
    });
    ready
}

fn watch_shutdown(
    harness: ice_harness::agent_harness::WeakAgentHarness,
    client: std::sync::Weak<Client>,
    spill: Arc<output_guard::SpillStore>,
) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(100)).await;
            let closed = match harness.upgrade() {
                Some(harness) => harness.get_tools().await.is_err(),
                None => true,
            };
            if closed {
                if let Some(client) = client.upgrade() {
                    client.closed.store(true, Ordering::SeqCst);
                    client._process.tree.kill();
                    fail_pending(&client.pending, "MCP session ended");
                }
                spill.cleanup();
                break;
            }
            if client.strong_count() == 0 {
                break;
            }
        }
    });
}
async fn discover_server(
    name: &str,
    config: &ServerConfig,
    cwd: &Path,
) -> Result<Vec<McpTool>, String> {
    let client = Client::spawn(name, config, cwd, Handlers::default())?;
    // Inspection never executes tools, so nothing is ever spilled.
    let guard = OutputGuard::default();
    tokio::time::timeout(START_TIMEOUT, client.discover(name, config, &guard))
        .await
        .map_err(|_| "MCP startup timed out".to_string())?
}

/// Run inner plugin/command hooks first; only surviving calls need MCP approval.
pub fn approval_hook(
    previous: Option<ice_agent::BeforeToolCall>,
    approvals: Arc<Mutex<HashSet<String>>>,
    ui: UiDialogMailbox,
) -> ice_agent::BeforeToolCall {
    Arc::new(move |ctx, signal| {
        let name = ctx.tool_call.name.clone();
        let call = ctx.tool_call.id.clone();
        let previous = previous.as_ref().map(|hook| hook(ctx, signal.clone()));
        let approvals = approvals.clone();
        let ui = ui.clone();
        Box::pin(async move {
            let previous_result = match previous {
                Some(future) => future.await,
                None => None,
            };
            if previous_result.as_ref().is_some_and(|result| result.block) {
                return previous_result;
            }
            if !approvals.lock().unwrap().contains(&name) {
                return previous_result;
            }
            match approve(&ui, &call, &name, &signal).await {
                Ok(()) => previous_result,
                Err(reason) => Some(ice_agent::BeforeToolCallResult {
                    block: true,
                    reason: Some(reason),
                    terminate: false,
                    args: None,
                }),
            }
        })
    })
}
struct ApprovalGuard<'a> {
    ui: &'a UiDialogMailbox,
    id: String,
}
impl Drop for ApprovalGuard<'_> {
    fn drop(&mut self) {
        let _ = self.ui.cancel(&self.id);
        let _ = self.ui.poll(&self.id);
    }
}
async fn approve(
    ui: &UiDialogMailbox,
    call: &str,
    tool: &str,
    signal: &CancellationToken,
) -> Result<(), String> {
    confirm(
        ui,
        call,
        &format!("Allow MCP tool {tool}?"),
        "MCP tool approval denied",
        signal,
    )
    .await
}
/// Allow/Deny permission dialog; `Ok` only when the user picked Allow. Permission
/// dialogs require a non-empty `call` id.
async fn confirm(
    ui: &UiDialogMailbox,
    call: &str,
    summary: &str,
    denied: &str,
    signal: &CancellationToken,
) -> Result<(), String> {
    let id = format!("mcp-{}", uuid::Uuid::new_v4());
    ui.handle(json!({"requestId":id,"toolCallId":call,"ui":{"purpose":"permission","kind":"confirm","summary":summary,"options":[{"title":"Deny","permission":"reject_once"},{"title":"Allow","permission":"allow_once"}]}}))?;
    let _guard = ApprovalGuard { ui, id: id.clone() };
    loop {
        let state = ui.poll(&id)?;
        match state["status"].as_str() {
            Some("answered") if state["answer"]["values"][0] == "Allow" => return Ok(()),
            Some("answered" | "cancelled") => return Err(denied.into()),
            _ => {}
        }
        tokio::select! {
            _ = signal.cancelled() => { let _ = ui.cancel(&id); let _ = ui.poll(&id); return Err("MCP approval cancelled".into()); }
            _ = tokio::time::sleep(Duration::from_millis(25)) => {}
        }
    }
}

/// Inspection does not resolve a model or require credentials.
pub async fn list(args: &crate::args::Args, cwd: &Path) -> Result<(String, bool), String> {
    let dir = crate::config::agent_dir().map_err(|e| e.to_string())?;
    let layered = load_layered(&dir, cwd, crate::session::resolve_project_trust(args, cwd))?;
    let own = [dir.join("mcp.json"), cwd.join(".icers/mcp.json")];
    let mut tasks = Vec::new();
    for (name, (config, source)) in layered.servers {
        let cwd = cwd.to_path_buf();
        let label = if own.contains(&source) {
            name.clone()
        } else {
            format!("{name} (imported from {})", source.display())
        };
        tasks.push(tokio::spawn(async move {
            if config.disabled {
                return (format!("{label}: disabled\n"), false);
            }
            match discover_server(&name, &config, &cwd).await {
                Ok(tools) => (
                    format!(
                        "{label}: ready\n{}",
                        tools
                            .iter()
                            .map(|t| format!(
                                "  {}{}\n",
                                t.schema.name,
                                if t.approval {
                                    " (approval required)"
                                } else {
                                    ""
                                }
                            ))
                            .collect::<String>()
                    ),
                    false,
                ),
                Err(error) => (format!("{label}: failed ({error})\n"), true),
            }
        }));
    }
    let mut output = String::new();
    let mut failed = false;
    for task in tasks {
        let (report, server_failed) = task.await.map_err(|e| e.to_string())?;
        output.push_str(&report);
        failed |= server_failed;
    }
    if output.is_empty() {
        output.push_str("No MCP servers configured.\n");
    }
    for note in layered.notes {
        output.push_str(&format!("skipped {note}\n"));
    }
    Ok((output, failed))
}
