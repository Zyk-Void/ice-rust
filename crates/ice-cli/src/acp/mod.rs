//! `icers --acp`: an Agent Client Protocol (ACP v1) agent over stdio, for
//! clients such as T3 Code and Zed.
//!
//! The parent process loads no model. Each ACP session is served by one native
//! `icers --mode rpc` child rooted at the session cwd ([`session`]), and
//! [`translate`] maps its events to ACP. Stdout carries only JSON-RPC frames;
//! diagnostics go to stderr.
//!
//! Behaviour reference: upstream `rpi-acp` (`pi-rust/rpi-package`, MIT).

mod session;
mod translate;

use serde_json::{json, Value};
use session::{Session, Spawner};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufRead, AsyncWrite, AsyncWriteExt};
use tokio::sync::oneshot;

const PROTOCOL_VERSION: u64 = 1;
/// Largest accepted inbound frame. Prompts can embed whole files as context.
const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const INTERNAL_ERROR: i64 = -32603;
/// ACP `RESOURCE_NOT_FOUND`, used for unknown session ids.
const RESOURCE_NOT_FOUND: i64 = -32002;

/// Flags the bridge owns: each child is already `--mode rpc` with its own
/// session, so these would conflict or make the child exit.
const RESERVED_CHILD_FLAGS: &[&str] = &[
    "--mode",
    "--print",
    "-p",
    "--connect",
    "--server",
    "--session",
    "--session-id",
    "--session-dir",
    "--continue",
    "-c",
    "--resume",
    "-r",
    "--fork",
    "--no-session",
    "--export",
    "--list-models",
    "--list-mcp",
    "--help",
    "-h",
    "--version",
    "-v",
];

/// Check the arguments `--acp` forwards to every session child. Returns the
/// first reserved flag, if any (`--flag=value` forms included).
pub fn check_child_args(args: &[String]) -> Result<(), String> {
    match args.iter().find(|arg| {
        let flag = arg.split_once('=').map_or(arg.as_str(), |(flag, _)| flag);
        RESERVED_CHILD_FLAGS.contains(&flag)
    }) {
        Some(flag) => Err(flag.clone()),
        None => Ok(()),
    }
}

/// Serve ACP on this process's stdin/stdout until the client disconnects.
/// `child_args` are appended to every session's `icers --mode rpc` command.
pub async fn run_stdio(child_args: Vec<String>) -> i32 {
    let program = match std::env::current_exe() {
        Ok(program) => program,
        Err(error) => {
            eprintln!("[icers acp] cannot locate the icers executable: {error}");
            return 1;
        }
    };
    let session_dir = match crate::settings::load_settings() {
        Ok(settings) => settings.session_dir,
        Err(error) => {
            eprintln!("[icers acp] cannot read session settings: {error}");
            return 1;
        }
    };
    serve(
        tokio::io::BufReader::new(tokio::io::stdin()),
        tokio::io::stdout(),
        session::process_spawner(program, child_args),
        session_dir,
    )
    .await
}

/// Line-delimited JSON-RPC writer shared by the server and every session.
pub(crate) struct Writer {
    out: tokio::sync::Mutex<Box<dyn AsyncWrite + Send + Unpin>>,
    next_id: AtomicU64,
    pending: Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>,
}

impl Writer {
    fn new(out: Box<dyn AsyncWrite + Send + Unpin>) -> Self {
        Self {
            out: tokio::sync::Mutex::new(out),
            next_id: AtomicU64::new(1),
            pending: Mutex::new(HashMap::new()),
        }
    }

    async fn write(&self, frame: Value) {
        let mut line = match serde_json::to_vec(&frame) {
            Ok(line) => line,
            Err(error) => {
                eprintln!("[icers acp] failed to encode frame: {error}");
                return;
            }
        };
        line.push(b'\n');
        let mut out = self.out.lock().await;
        if out.write_all(&line).await.is_err() || out.flush().await.is_err() {
            eprintln!("[icers acp] client stdout closed");
        }
    }

    async fn respond(&self, id: Value, result: Value) {
        self.write(json!({"jsonrpc": "2.0", "id": id, "result": result}))
            .await;
    }

    async fn error(&self, id: Value, code: i64, message: impl Into<String>) {
        self.write(json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {"code": code, "message": message.into()},
        }))
        .await;
    }

    pub(crate) async fn notify(&self, method: &str, params: Value) {
        self.write(json!({"jsonrpc": "2.0", "method": method, "params": params}))
            .await;
    }

    /// Send an agent→client request and wait for its result. Resolves to `Err`
    /// on a JSON-RPC error or when the client disconnects first.
    pub(crate) async fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        self.write(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
            .await;
        rx.await
            .unwrap_or_else(|_| Err("client disconnected".to_string()))
    }

    /// Route a client response to the matching [`request`](Self::request).
    fn resolve(&self, frame: &Value) {
        let Some(id) = frame["id"].as_u64() else {
            return;
        };
        let Some(waiter) = self.pending.lock().unwrap().remove(&id) else {
            return;
        };
        let result = match frame.get("error") {
            Some(error) => Err(error["message"]
                .as_str()
                .unwrap_or("client error")
                .to_string()),
            None => Ok(frame.get("result").cloned().unwrap_or(Value::Null)),
        };
        let _ = waiter.send(result);
    }

    /// Fail every outstanding client request (client gone).
    fn close(&self) {
        self.pending.lock().unwrap().clear();
    }
}

struct Server {
    writer: Arc<Writer>,
    spawner: Spawner,
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    loading: tokio::sync::Mutex<()>,
    session_dir: Option<PathBuf>,
}

/// Run the ACP server over the given streams. Generic so tests can drive it
/// in memory with a fake child.
pub(crate) async fn serve<R, W>(
    mut input: R,
    output: W,
    spawner: Spawner,
    session_dir: Option<PathBuf>,
) -> i32
where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Send + Unpin + 'static,
{
    let server = Arc::new(Server {
        writer: Arc::new(Writer::new(Box::new(output))),
        spawner,
        sessions: Mutex::new(HashMap::new()),
        loading: tokio::sync::Mutex::new(()),
        session_dir,
    });
    let mut tasks = tokio::task::JoinSet::new();
    loop {
        let line = match read_frame(&mut input).await {
            Ok(Some(line)) => line,
            Ok(None) => break,
            Err(error) => {
                eprintln!("[icers acp] stdin error: {error}");
                break;
            }
        };
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let frame: Value = match serde_json::from_slice(&line) {
            Ok(frame) => frame,
            Err(error) => {
                server
                    .writer
                    .error(Value::Null, PARSE_ERROR, format!("invalid JSON: {error}"))
                    .await;
                continue;
            }
        };
        // Requests run concurrently so `session/cancel` and permission answers
        // arrive while a `session/prompt` is still in flight.
        while tasks.try_join_next().is_some() {}
        let server = Arc::clone(&server);
        tasks.spawn(async move { server.dispatch(frame).await });
    }
    // Stdin closing does not retract requests already sent: let quick ones
    // (initialize, config changes) answer before tearing everything down.
    let drain = async { while tasks.join_next().await.is_some() {} };
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), drain).await;
    server.writer.close();
    tasks.abort_all();
    let sessions: Vec<_> = server
        .sessions
        .lock()
        .unwrap()
        .drain()
        .map(|(_, s)| s)
        .collect();
    for session in sessions {
        session.shutdown().await;
    }
    0
}

async fn read_frame<R: AsyncBufRead + Unpin>(reader: &mut R) -> std::io::Result<Option<Vec<u8>>> {
    crate::rpc::framing::read_frame_limited(reader, MAX_FRAME_BYTES).await
}

impl Server {
    async fn dispatch(&self, frame: Value) {
        let method = frame.get("method").and_then(Value::as_str);
        let id = frame.get("id").cloned();
        match (method, id) {
            (Some(method), Some(id)) => {
                let params = frame.get("params").cloned().unwrap_or(Value::Null);
                match self.handle(method, params).await {
                    Ok(result) => {
                        let created = (method == "session/new" || method == "session/load")
                            .then(|| result["sessionId"].as_str().map(str::to_owned))
                            .flatten();
                        self.writer.respond(id, result).await;
                        // Commands go out after the response, once the client
                        // knows the session id they belong to.
                        if let Some(created) = created {
                            let session = self.sessions.lock().unwrap().get(&created).cloned();
                            if let Some(session) = session {
                                session.publish_commands().await;
                            }
                        }
                    }
                    Err((code, message)) => self.writer.error(id, code, message).await,
                }
            }
            (Some(method), None) => self.handle_notification(method, &frame["params"]).await,
            (None, Some(_)) => self.writer.resolve(&frame),
            (None, None) => {
                self.writer
                    .error(Value::Null, INVALID_REQUEST, "not a JSON-RPC message")
                    .await
            }
        }
    }

    async fn handle_notification(&self, method: &str, params: &Value) {
        if method == "session/cancel" {
            if let Ok(session) = self.session(params) {
                session.cancel().await;
            }
        }
    }

    async fn handle(&self, method: &str, params: Value) -> Result<Value, (i64, String)> {
        match method {
            "initialize" => Ok(initialize_result()),
            "authenticate" => Ok(json!({})),
            "session/new" => self.new_session(params).await,
            "session/load" => self.load_session(params).await,
            "session/prompt" => {
                let session = self.session(&params)?;
                let blocks = params["prompt"]
                    .as_array()
                    .ok_or((INVALID_PARAMS, "`prompt` must be an array".to_string()))?;
                let (text, images) =
                    translate::prompt_content(blocks).map_err(|e| (INVALID_PARAMS, e))?;
                let changes_config = ["/model", "/thinking"]
                    .iter()
                    .any(|command| text.trim_start().starts_with(command));
                let stop = session
                    .prompt(text, images)
                    .await
                    .map_err(|e| (INTERNAL_ERROR, e))?;
                if changes_config {
                    session.publish_config_options().await;
                }
                Ok(json!({"stopReason": stop}))
            }
            // Also accepted as a request, for clients that send it with an id.
            "session/cancel" => {
                self.session(&params)?.cancel().await;
                Ok(Value::Null)
            }
            "session/set_model" => {
                let session = self.session(&params)?;
                let value = params["modelId"]
                    .as_str()
                    .ok_or((INVALID_PARAMS, "`modelId` must be a string".to_string()))?;
                session
                    .set_config_option("model", value)
                    .await
                    .map_err(|e| (INVALID_PARAMS, e))?;
                session.publish_config_options().await;
                Ok(json!({}))
            }
            "session/set_config_option" => {
                let session = self.session(&params)?;
                let config_id = params["configId"].as_str().unwrap_or_default();
                let value = params["value"]
                    .as_str()
                    .ok_or((INVALID_PARAMS, "`value` must be a string".to_string()))?;
                session
                    .set_config_option(config_id, value)
                    .await
                    .map_err(|e| (INVALID_PARAMS, e))?;
                Ok(json!({"configOptions": session.config_options().await}))
            }
            _ => Err((METHOD_NOT_FOUND, format!("method not found: {method}"))),
        }
    }

    async fn new_session(&self, params: Value) -> Result<Value, (i64, String)> {
        let cwd = params["cwd"]
            .as_str()
            .ok_or((INVALID_PARAMS, "`cwd` is required".to_string()))?;
        let cwd = PathBuf::from(cwd);
        if !cwd.is_absolute() || !cwd.is_dir() {
            return Err((
                INVALID_PARAMS,
                format!(
                    "`cwd` must be an existing absolute directory: {}",
                    cwd.display()
                ),
            ));
        }
        match params.get("mcpServers") {
            None | Some(Value::Null) => {}
            Some(Value::Array(servers)) if servers.is_empty() => {}
            Some(Value::Array(servers)) => {
                // Accepted (rejecting breaks clients that always send theirs),
                // but not yet started per session. Never log their env: it can
                // carry bearer tokens.
                let names: Vec<&str> = servers.iter().filter_map(|s| s["name"].as_str()).collect();
                eprintln!(
                    "[icers acp] client MCP servers are not supported yet; ignoring: {}",
                    names.join(", ")
                );
            }
            Some(_) => return Err((INVALID_PARAMS, "`mcpServers` must be an array".to_string())),
        }
        let id = uuid::Uuid::new_v4().to_string();
        let session = Session::start(
            id.clone(),
            cwd,
            None,
            &self.spawner,
            Arc::clone(&self.writer),
        )
        .await
        .map_err(|e| (INTERNAL_ERROR, e))?;
        let config_options = session.config_options().await;
        self.sessions.lock().unwrap().insert(id.clone(), session);
        Ok(json!({"sessionId": id, "configOptions": config_options}))
    }

    async fn load_session(&self, params: Value) -> Result<Value, (i64, String)> {
        let id = params["sessionId"]
            .as_str()
            .filter(|id| uuid::Uuid::parse_str(id).is_ok())
            .ok_or((
                INVALID_PARAMS,
                "`sessionId` must be an ACP session UUID".to_string(),
            ))?;
        let cwd = params["cwd"]
            .as_str()
            .ok_or((INVALID_PARAMS, "`cwd` is required".to_string()))?;
        let cwd_path = PathBuf::from(cwd);
        if !cwd_path.is_absolute() || !cwd_path.is_dir() {
            return Err((
                INVALID_PARAMS,
                "`cwd` must be an existing absolute directory".to_string(),
            ));
        }
        if params
            .get("mcpServers")
            .is_some_and(|servers| !servers.is_null() && !servers.is_array())
        {
            return Err((INVALID_PARAMS, "`mcpServers` must be an array".to_string()));
        }
        // Serialize loads so concurrent requests cannot spawn two owners of the
        // same native file. Do not hold the sessions map lock across awaits.
        let _loading = self.loading.lock().await;
        if self.sessions.lock().unwrap().contains_key(id) {
            return Err((INVALID_PARAMS, format!("session is already open: {id}")));
        }
        let dir = self
            .session_dir
            .as_ref()
            .map(|dir| cwd_path.join(dir))
            .unwrap_or_else(|| crate::session::default_session_dir(&cwd_path));
        let discovery = crate::session::discover_session_metadata_in_dir(cwd, &dir)
            .await
            .map_err(|error| (INTERNAL_ERROR, error.to_string()))?;
        let meta = discovery
            .sessions
            .into_iter()
            .find(|meta| meta.id == id)
            .ok_or((
                RESOURCE_NOT_FOUND,
                format!("saved session not found or unreadable: {id}"),
            ))?;
        if std::fs::canonicalize(&meta.cwd).ok() != std::fs::canonicalize(&cwd_path).ok() {
            return Err((
                INVALID_PARAMS,
                "saved session belongs to a different working directory".to_string(),
            ));
        }
        let saved_path = PathBuf::from(&meta.path);
        let session = Session::start(
            id.to_string(),
            cwd_path,
            Some(&saved_path),
            &self.spawner,
            Arc::clone(&self.writer),
        )
        .await
        .map_err(|error| {
            (
                INTERNAL_ERROR,
                format!("cannot load saved session {id}: {error}"),
            )
        })?;
        if let Err(error) = session.replay_history().await {
            session.shutdown().await;
            return Err((
                INTERNAL_ERROR,
                format!("cannot replay saved session {id}: {error}"),
            ));
        }
        let config_options = session.config_options().await;
        self.sessions
            .lock()
            .unwrap()
            .insert(id.to_string(), session);
        Ok(json!({"sessionId": id, "configOptions": config_options}))
    }

    fn session(&self, params: &Value) -> Result<Arc<Session>, (i64, String)> {
        let id = params["sessionId"]
            .as_str()
            .ok_or((INVALID_PARAMS, "`sessionId` is required".to_string()))?;
        self.sessions
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or((RESOURCE_NOT_FOUND, format!("unknown session: {id}")))
    }
}

/// ACP v1 `initialize` result. Advertises only what is implemented.
fn initialize_result() -> Value {
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "agentCapabilities": {
            "loadSession": true,
            "promptCapabilities": {"image": true, "audio": false, "embeddedContext": true},
            "mcpCapabilities": {"http": false, "sse": false},
        },
        "agentInfo": {
            "name": "icers",
            "title": "icers",
            "version": env!("CARGO_PKG_VERSION"),
        },
        "authMethods": [],
    })
}

#[cfg(test)]
mod tests;
