//! Session-local stdio MCP client. No network transports or extension ABI.
#[cfg(test)]
mod tests;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
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
use tokio::sync::{oneshot, watch};
use tokio_util::sync::CancellationToken;

const START_TIMEOUT: Duration = Duration::from_secs(10);
const CALL_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_LINE: u64 = 16 * 1024 * 1024;

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
}
#[derive(Default, Deserialize)]
struct Config {
    #[serde(default, rename = "mcpServers")]
    servers: BTreeMap<String, ServerConfig>,
}

/// Merge whole server entries, never consulting project configuration unless trusted.
pub fn load_config(
    agent_dir: &Path,
    cwd: &Path,
    trusted: bool,
) -> Result<BTreeMap<String, ServerConfig>, String> {
    let mut servers = BTreeMap::new();
    let mut paths = vec![agent_dir.join("mcp.json")];
    if trusted {
        paths.push(cwd.join(".icers/mcp.json"));
    }
    for path in paths {
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        let config: Config =
            serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        for (name, server) in config.servers {
            if name.is_empty()
                || !name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            {
                return Err(format!("invalid MCP server name: {name:?}"));
            }
            servers.insert(name, server);
        }
    }
    Ok(servers)
}

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>;

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
    pending: Pending,
    next_id: AtomicU64,
    closed: Arc<AtomicBool>,
    _process: Process,
}
impl Client {
    fn spawn(name: &str, config: &ServerConfig, cwd: &Path) -> Result<Arc<Self>, String> {
        if config.command.is_empty() {
            return Err("missing MCP command".into());
        }
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
        let reader_pending = pending.clone();
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
                // Server-initiated requests are not supported. Notifications need no response.
                if value.get("method").is_some() {
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
            pending,
            next_id: AtomicU64::new(1),
            closed,
            _process: process,
        }))
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
        };
        if signal.is_cancelled() {
            return Err("MCP call cancelled".into());
        }
        self.outgoing
            .send(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))
            .map_err(|_| "MCP writer closed")?;
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
    ) -> Result<Vec<McpTool>, String> {
        let signal = CancellationToken::new();
        let initialized = self
            .request(
                "initialize",
                json!({
                    "protocolVersion":"2025-06-18", "capabilities":{},
                    "clientInfo":{"name":"icers","version":crate::VERSION}
                }),
                signal.clone(),
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
        self.notify("notifications/initialized", json!({}));
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
                tools.push(McpTool {
                    schema: Tool {
                        name: format!("{server}_{name}"),
                        description: item
                            .get("description")
                            .and_then(Value::as_str)
                            .unwrap_or("MCP tool")
                            .into(),
                        parameters: serde_json::from_value(schema)
                            .map_err(|e| format!("invalid MCP inputSchema: {e}"))?,
                        constrained_sampling: None,
                    },
                    remote_name: name.into(),
                    client: self.clone(),
                    approval: config.approve_tools.requires(name),
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
fn fail_pending(pending: &Pending, reason: &str) {
    for (_, tx) in pending.lock().unwrap().drain() {
        let _ = tx.send(Err(reason.into()));
    }
}
struct RequestGuard<'a> {
    client: &'a Client,
    id: u64,
    completed: bool,
}
impl Drop for RequestGuard<'_> {
    fn drop(&mut self) {
        self.client.pending.lock().unwrap().remove(&self.id);
        if !self.completed {
            self.client.notify(
                "notifications/cancelled",
                json!({"requestId":self.id,"reason":"client cancelled or timed out"}),
            );
        }
    }
}

struct McpTool {
    schema: Tool,
    remote_name: String,
    client: Arc<Client>,
    approval: bool,
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
        let result = self
            .client
            .request(
                "tools/call",
                json!({"name":self.remote_name,"arguments":params}),
                signal,
                CALL_TIMEOUT,
            )
            .await
            .map_err(AgentError::Tool)?;
        map_result(result)
    }
}
fn map_result(result: Value) -> Result<AgentToolResult, AgentError> {
    if result.get("isError").and_then(Value::as_bool) == Some(true) {
        return Err(AgentError::Tool(format!("MCP tool error: {result}")));
    }
    let mut content = Vec::new();
    for item in result
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| AgentError::Tool("MCP result missing content".into()))?
    {
        let block = match item.get("type").and_then(Value::as_str) {
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
            Some("resource") if item["resource"]["text"].is_string() => {
                TextContentOrImage::text(item["resource"]["text"].as_str().unwrap())
            }
            _ => TextContentOrImage::text(item.to_string()),
        };
        content.push(block);
    }
    Ok(AgentToolResult {
        content,
        details: result,
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
pub fn start(
    harness: &AgentHarness,
    args: &crate::args::Args,
    cwd: &Path,
    trusted: bool,
    approvals: Arc<Mutex<HashSet<String>>>,
    registry: Arc<Mutex<Vec<HarnessTool>>>,
) -> watch::Receiver<bool> {
    let (ready_tx, ready) = watch::channel(false);
    let servers = match crate::config::agent_dir()
        .map_err(|e| e.to_string())
        .and_then(|dir| load_config(&dir, cwd, trusted))
    {
        Ok(servers) => servers,
        Err(error) => {
            tracing::warn!(%error, "MCP configuration failed");
            let _ = ready_tx.send(true);
            return ready;
        }
    };
    publish_servers(harness, args, cwd, approvals, registry, servers, ready_tx);
    ready
}

fn publish_servers(
    harness: &AgentHarness,
    args: &crate::args::Args,
    cwd: &Path,
    approvals: Arc<Mutex<HashSet<String>>>,
    registry: Arc<Mutex<Vec<HarnessTool>>>,
    servers: BTreeMap<String, ServerConfig>,
    ready_tx: watch::Sender<bool>,
) {
    if servers.values().all(|config| config.disabled) {
        let _ = ready_tx.send(true);
        return;
    }
    // A single publisher serializes registry updates across servers.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    for (name, config) in servers {
        if config.disabled {
            continue;
        }
        let tx = tx.clone();
        let cwd = cwd.to_path_buf();
        tokio::spawn(async move {
            let result = discover_server(&name, &config, &cwd).await;
            match result {
                Ok(tools) => {
                    let _ = tx.send(tools);
                }
                Err(error) => tracing::warn!(server = %name, %error, "MCP startup failed"),
            }
        });
    }
    drop(tx);
    let weak = harness.downgrade();
    let args = args.clone();
    tokio::spawn(async move {
        while let Some(discovered) = rx.recv().await {
            let Some(harness) = weak.upgrade() else {
                break;
            };
            let Ok(mut tools) = harness.get_tools().await else {
                break;
            };
            let Ok(mut active) = harness.lane("main").get_active_tools().await else {
                break;
            };
            if let Some(tool) = discovered.first() {
                watch_shutdown(harness.downgrade(), Arc::downgrade(&tool.client));
            }
            for tool in discovered {
                let name = tool.schema.name.clone();
                if !crate::session::tool_name_allowed(&name, &args) {
                    continue;
                }
                // Never shadow a built-in/plugin or an ambiguous server_tool name.
                if tools.iter().any(|t| t.tool.schema().name == name) {
                    tracing::warn!(%name, "MCP tool name collision");
                    continue;
                }
                if tool.approval {
                    approvals.lock().unwrap().insert(name.clone());
                }
                active.push(name);
                let tool = HarnessTool::new(Arc::new(tool));
                registry.lock().unwrap().push(tool.clone());
                tools.push(tool);
            }
            let _ = harness.set_tools(tools, Some(active)).await;
        }
        let _ = ready_tx.send(true);
    });
}
fn watch_shutdown(
    harness: ice_harness::agent_harness::WeakAgentHarness,
    client: std::sync::Weak<Client>,
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
    let client = Client::spawn(name, config, cwd)?;
    tokio::time::timeout(START_TIMEOUT, client.discover(name, config))
        .await
        .map_err(|_| "MCP startup timed out".to_string())?
}

/// Compose with the plugin hook: a plugin veto always wins, then MCP approval.
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
    let id = format!("mcp-{}", uuid::Uuid::new_v4());
    ui.handle(json!({"requestId":id,"toolCallId":call,"ui":{"purpose":"permission","kind":"confirm","summary":format!("Allow MCP tool {tool}?"),"options":[{"title":"Deny","permission":"reject_once"},{"title":"Allow","permission":"allow_once"}]}}))?;
    let _guard = ApprovalGuard { ui, id: id.clone() };
    loop {
        let state = ui.poll(&id)?;
        match state["status"].as_str() {
            Some("answered") if state["answer"]["values"][0] == "Allow" => return Ok(()),
            Some("answered" | "cancelled") => return Err("MCP tool approval denied".into()),
            _ => {}
        }
        tokio::select! {
            _ = signal.cancelled() => { let _ = ui.cancel(&id); let _ = ui.poll(&id); return Err("MCP approval cancelled".into()); }
            _ = tokio::time::sleep(Duration::from_millis(25)) => {}
        }
    }
}

/// Inspection does not resolve a model or require credentials.
pub async fn list(args: &crate::args::Args, cwd: &Path) -> Result<String, String> {
    let dir = crate::config::agent_dir().map_err(|e| e.to_string())?;
    let servers = load_config(&dir, cwd, crate::session::resolve_project_trust(args, cwd))?;
    let mut tasks = Vec::new();
    for (name, config) in servers {
        let cwd = cwd.to_path_buf();
        tasks.push(tokio::spawn(async move {
            if config.disabled {
                return format!("{name}: disabled\n");
            }
            match discover_server(&name, &config, &cwd).await {
                Ok(tools) => format!(
                    "{name}: ready\n{}",
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
                Err(error) => format!("{name}: failed ({error})\n"),
            }
        }));
    }
    let mut output = String::new();
    for task in tasks {
        output.push_str(&task.await.map_err(|e| e.to_string())?);
    }
    if output.is_empty() {
        output.push_str("No MCP servers configured.\n");
    }
    Ok(output)
}
