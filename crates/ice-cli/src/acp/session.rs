//! One ACP session = one native `icers --mode rpc` child rooted at the session
//! cwd. The child is the ordinary, fully configured icers runtime (providers,
//! tools, extensions, hooks, skills, MCP, memory, compaction), so ACP gets the
//! same agent the TUI gets; this module only moves frames between the two.

use super::translate::{self, Translator};
use super::Writer;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

/// How long a fresh child may take to print its `ready` frame.
const READY_TIMEOUT: Duration = Duration::from_secs(60);
/// How long a child may take to answer a non-prompt command.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

/// The stdio of a native child (a real process, or an in-memory fake in tests).
pub(crate) struct ChildIo {
    pub stdin: Box<dyn AsyncWrite + Send + Unpin>,
    pub stdout: Box<dyn AsyncBufRead + Send + Unpin>,
    pub process: Option<tokio::process::Child>,
}

/// Starts the native child for `(cwd, session_id)`.
pub(crate) type Spawner =
    Arc<dyn Fn(&Path, &str, Option<&Path>) -> std::io::Result<ChildIo> + Send + Sync>;

/// Spawn `program --mode rpc --session-id <id> <extra_args>` in `cwd`. The
/// child loads exactly what a normal `icers` run in that directory loads (plus
/// whatever the user passed to `--acp`, e.g. `--observe`), and stores its
/// session where `icers --resume` finds it.
pub(crate) fn process_spawner(program: PathBuf, extra_args: Vec<String>) -> Spawner {
    Arc::new(move |cwd, session_id, saved_path| {
        let mut command = tokio::process::Command::new(&program);
        command.args(["--mode", "rpc"]).args(&extra_args);
        if let Some(path) = saved_path {
            // Open-only: never recreate a missing or unreadable saved session.
            command.arg("--session").arg(path);
        } else {
            command.args(["--session-id", session_id]);
        }
        let mut child = command
            .current_dir(cwd)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            // Child diagnostics go to our stderr; ACP stdout stays protocol-only.
            .stderr(std::process::Stdio::inherit())
            .kill_on_drop(true)
            .spawn()?;
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        Ok(ChildIo {
            stdin: Box::new(stdin),
            stdout: Box::new(tokio::io::BufReader::new(stdout)),
            process: Some(child),
        })
    })
}

pub(crate) struct Session {
    id: String,
    writer: Arc<Writer>,
    elicitation_form: bool,
    stdin: tokio::sync::Mutex<Box<dyn AsyncWrite + Send + Unpin>>,
    process: Mutex<Option<tokio::process::Child>>,
    next_id: AtomicU64,
    pending: Mutex<HashMap<u64, oneshot::Sender<Value>>>,
    settled: Mutex<Option<oneshot::Sender<Value>>>,
    translator: Mutex<Translator>,
    turn: Mutex<Option<Arc<CancellationToken>>>,
    /// Serializes turn transitions with delivery of an abort to the child.
    turn_gate: tokio::sync::Mutex<()>,
    /// Cancelled when the child's stdout closes (exit, crash or shutdown).
    closed: CancellationToken,
}

impl Session {
    /// Spawn the child, start its reader and wait for its `ready` frame.
    pub(crate) async fn start(
        id: String,
        cwd: PathBuf,
        saved_path: Option<&Path>,
        spawner: &Spawner,
        writer: Arc<Writer>,
        elicitation_form: bool,
    ) -> Result<Arc<Self>, String> {
        let io = spawner(&cwd, &id, saved_path)
            .map_err(|error| format!("failed to start icers: {error}"))?;
        let session = Arc::new(Self {
            translator: Mutex::new(Translator::new(cwd)),
            id,
            writer,
            elicitation_form,
            stdin: tokio::sync::Mutex::new(io.stdin),
            process: Mutex::new(io.process),
            next_id: AtomicU64::new(1),
            pending: Mutex::new(HashMap::new()),
            settled: Mutex::new(None),
            turn: Mutex::new(None),
            turn_gate: tokio::sync::Mutex::new(()),
            closed: CancellationToken::new(),
        });
        let (ready_tx, ready_rx) = oneshot::channel();
        tokio::spawn(Arc::clone(&session).pump(io.stdout, ready_tx));
        let ready = tokio::select! {
            ready = ready_rx => ready.is_ok(),
            _ = tokio::time::sleep(READY_TIMEOUT) => false,
        };
        if !ready {
            session.shutdown().await;
            return Err("icers did not become ready (see stderr for details)".to_string());
        }
        Ok(session)
    }

    /// Read child frames until EOF, routing responses and settlement and
    /// translating everything else into `session/update` notifications. Updates
    /// are written before settlement is signalled, so the client always sees a
    /// turn's output before its `session/prompt` response.
    async fn pump(
        self: Arc<Self>,
        stdout: Box<dyn AsyncBufRead + Send + Unpin>,
        ready: oneshot::Sender<()>,
    ) {
        let mut ready = Some(ready);
        let mut lines = stdout.lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let Ok(frame) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            match frame["type"].as_str() {
                Some("ready") => {
                    if let Some(ready) = ready.take() {
                        let _ = ready.send(());
                    }
                }
                Some("response") => {
                    let waiter = frame["id"]
                        .as_u64()
                        .and_then(|id| self.pending.lock().unwrap().remove(&id));
                    if let Some(waiter) = waiter {
                        let _ = waiter.send(frame);
                    }
                }
                Some("agent_settled") => {
                    if let Some(settled) = self.settled.lock().unwrap().take() {
                        let _ = settled.send(frame);
                    }
                }
                Some("ui_dialog_request") => {
                    // Capture this turn before spawning, so an old dialog
                    // cannot attach itself to a later prompt.
                    let cancel = self
                        .turn
                        .lock()
                        .unwrap()
                        .as_ref()
                        .map(|token| token.as_ref().clone());
                    if let Some(cancel) = cancel {
                        tokio::spawn(Arc::clone(&self).answer_dialog(frame, cancel));
                    }
                }
                _ => {
                    let updates = self.translator.lock().unwrap().translate(&frame);
                    for update in updates {
                        self.notify_update(update).await;
                    }
                }
            }
        }
        self.closed.cancel();
        self.pending.lock().unwrap().clear();
        self.settled.lock().unwrap().take();
    }

    async fn notify_update(&self, update: Value) {
        self.writer
            .notify(
                "session/update",
                json!({"sessionId": self.id, "update": update}),
            )
            .await;
    }

    /// Forward permission dialogs and negotiated question forms. Unsupported
    /// dialogs are cancelled so the asking extension never waits indefinitely.
    async fn answer_dialog(self: Arc<Self>, frame: Value, cancel: CancellationToken) {
        let request_id = frame["requestId"].as_str().unwrap_or_default().to_string();
        let ui = &frame["ui"];
        let response = match translate::permission_options(ui) {
            Some(options) => {
                let tool_call_id = frame["toolCallId"].as_str().unwrap_or(&request_id);
                let tool_call = self.translator.lock().unwrap().tool_call(tool_call_id);
                let mut params =
                    translate::permission_params(ui, &options, tool_call, tool_call_id);
                params["sessionId"] = json!(self.id);
                let outcome = self
                    .writer
                    .request("session/request_permission", params, &cancel)
                    .await;
                translate::permission_answer(&request_id, &options, outcome)
            }
            None => {
                if let Some(form) = self
                    .elicitation_form
                    .then(|| translate::question_form(&frame))
                    .flatten()
                {
                    let mut params = json!({
                        "sessionId": self.id,
                        "mode": "form",
                        "message": form.message,
                        "requestedSchema": form.requested_schema,
                    });
                    if let Some(tool_call_id) = frame["toolCallId"].as_str() {
                        params["toolCallId"] = json!(tool_call_id);
                    }
                    let outcome = self
                        .writer
                        .request("session/elicitation", params, &cancel)
                        .await;
                    form.answer(&request_id, outcome)
                } else {
                    json!({"type": "ui_dialog_response", "requestId": request_id, "cancelled": true})
                }
            }
        };
        // The native abort/settlement clears its own dialog. Do not send a
        // late answer (or a second cancellation) into the next turn.
        let _gate = self.turn_gate.lock().await;
        if !cancel.is_cancelled() {
            let _ = self.send(&response).await;
        }
    }

    async fn send(&self, frame: &Value) -> Result<(), String> {
        let mut line = serde_json::to_vec(frame).map_err(|error| error.to_string())?;
        line.push(b'\n');
        if line.len() > crate::rpc::protocol::MAX_FRAME_BYTES {
            return Err("prompt exceeds the native RPC frame limit (16 MiB)".to_string());
        }
        let mut stdin = self.stdin.lock().await;
        stdin
            .write_all(&line)
            .await
            .map_err(|_| "icers process is not running".to_string())?;
        stdin
            .flush()
            .await
            .map_err(|_| "icers process is not running".to_string())
    }

    /// Send a native command and wait for its `response` frame.
    pub(crate) async fn command(&self, mut frame: Value) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        frame["id"] = json!(id);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        if let Err(error) = self.send(&frame).await {
            self.pending.lock().unwrap().remove(&id);
            return Err(error);
        }
        let response = tokio::select! {
            response = rx => response.map_err(|_| "icers process exited".to_string())?,
            _ = tokio::time::sleep(COMMAND_TIMEOUT) => {
                self.pending.lock().unwrap().remove(&id);
                return Err("icers did not answer in time".to_string());
            }
        };
        if response["success"].as_bool() == Some(true) {
            Ok(response.get("data").cloned().unwrap_or(Value::Null))
        } else {
            Err(response["error"]
                .as_str()
                .unwrap_or("icers command failed")
                .to_string())
        }
    }

    /// Run one prompt turn to its native settlement and return the ACP
    /// `stopReason`. The native `accepted` ack is not completion.
    pub(crate) async fn prompt(
        &self,
        text: String,
        images: Vec<ice_ai::types::ImageContent>,
    ) -> Result<&'static str, String> {
        let gate = self.turn_gate.lock().await;
        let cancel = {
            let mut turn = self.turn.lock().unwrap();
            if turn.is_some() {
                return Err("a prompt is already running in this session".to_string());
            }
            let cancel = Arc::new(self.closed.child_token());
            *turn = Some(cancel.clone());
            cancel
        };
        let result = self.prompt_inner(text, images, &cancel, gate).await;
        cancel.cancel();
        let _gate = self.turn_gate.lock().await;
        self.turn.lock().unwrap().take();
        result
    }

    async fn prompt_inner(
        &self,
        text: String,
        images: Vec<ice_ai::types::ImageContent>,
        cancel: &CancellationToken,
        gate: tokio::sync::MutexGuard<'_, ()>,
    ) -> Result<&'static str, String> {
        let (tx, rx) = oneshot::channel();
        *self.settled.lock().unwrap() = Some(tx);
        if let Err(error) = self
            .command(json!({"type": "prompt", "message": text, "images": images}))
            .await
        {
            self.settled.lock().unwrap().take();
            return Err(error);
        }
        drop(gate);
        let settled = tokio::select! {
            settled = rx => settled.map_err(|_| "icers process exited during the turn".to_string())?,
            _ = self.closed.cancelled() => return Err("icers process exited during the turn".to_string()),
        };
        // `/compact` reports its result as a settlement summary.
        if let Some(summary) = settled["summary"].as_str().filter(|text| !text.is_empty()) {
            self.notify_update(json!({
                "sessionUpdate": "agent_message_chunk",
                "content": {"type": "text", "text": summary},
            }))
            .await;
        }
        let stop = self.translator.lock().unwrap().take_stop_reason();
        translate::stop_reason(&settled, cancel.is_cancelled(), stop.as_deref())
    }

    /// `session/cancel`: abort the running turn. Its `session/prompt` then
    /// resolves with `cancelled` once the child settles.
    pub(crate) async fn cancel(&self) {
        let cancel = self.turn.lock().unwrap().clone();
        if let Some(cancel) = cancel {
            let _gate = self.turn_gate.lock().await;
            let current = self
                .turn
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|turn| Arc::ptr_eq(turn, &cancel));
            if current {
                cancel.cancel();
                let _ = self.send(&json!({"type": "abort"})).await;
            }
        }
    }

    /// Replay the complete active branch, oldest first, before responding to
    /// session/load. Native get_messages pages backwards from the newest entry.
    pub(crate) async fn replay_history(&self) -> Result<(), String> {
        let mut pages = Vec::new();
        let mut before = Value::Null;
        let mut cursors = std::collections::HashSet::new();
        let mut saved_config = Value::Null;
        loop {
            let page = self
                .command(json!({"type": "get_messages", "limit": 400, "beforeId": before}))
                .await?;
            if pages.is_empty() {
                saved_config = page["savedConfig"].clone();
            }
            let messages = page["messages"]
                .as_array()
                .ok_or_else(|| "invalid native history response".to_string())?
                .clone();
            let has_more = page["hasMore"].as_bool().unwrap_or(false);
            let cursor = page["firstId"].clone();
            pages.push(messages);
            if !has_more {
                break;
            }
            let id = cursor
                .as_str()
                .filter(|id| !id.is_empty())
                .ok_or_else(|| "missing history pagination cursor".to_string())?;
            if !cursors.insert(id.to_string()) {
                return Err("repeated history pagination cursor".to_string());
            }
            before = cursor;
        }
        self.apply_saved_config(&saved_config).await?;
        // Use a separate translator: replay must not leave a historical stop
        // reason or partial text in the live prompt's streaming state.
        let mut translator = self.translator.lock().unwrap().clone_for_replay();
        for messages in pages.into_iter().rev() {
            for message in messages {
                for update in translator.replay_message(&message) {
                    self.writer
                        .notify(
                            "session/update",
                            json!({"sessionId": self.id, "update": update}),
                        )
                        .await;
                }
            }
        }
        Ok(())
    }

    /// Restore only persisted selections; resume must not replay transcript updates.
    pub(crate) async fn restore_saved_config(&self) -> Result<(), String> {
        let page = self
            .command(json!({"type": "get_messages", "limit": 1}))
            .await?;
        if !page["messages"].is_array() {
            return Err("invalid native history response".to_string());
        }
        self.apply_saved_config(&page["savedConfig"]).await
    }

    async fn apply_saved_config(&self, saved_config: &Value) -> Result<(), String> {
        if saved_config.is_object() {
            let state = self.command(json!({"type": "get_state"})).await?;
            if let (Some(provider), Some(model)) = (
                saved_config["provider"].as_str(),
                saved_config["modelId"].as_str(),
            ) {
                if state["provider"] != provider || state["modelId"] != model {
                    // Reset the startup thinking default before switching to
                    // the saved model; restore its persisted level below.
                    self.command(
                        json!({"type": "set_thinking_level", "level": "off", "persist": false}),
                    )
                    .await?;
                    self.command(json!({"type": "set_model", "provider": provider, "modelId": model, "persist": false})).await?;
                }
            }
            if let Some(level) = saved_config["thinkingLevel"].as_str() {
                self.command(
                    json!({"type": "set_thinking_level", "level": level, "persist": false}),
                )
                .await?;
            }
        }
        Ok(())
    }

    /// ACP `configOptions` for model and thinking level, built from the
    /// child's live state. Errors degrade to no options rather than failing
    /// session setup.
    pub(crate) async fn config_options(&self) -> Value {
        let state = self.command(json!({"type": "get_state"})).await;
        let models = self.command(json!({"type": "get_available_models"})).await;
        let levels = self
            .command(json!({"type": "get_available_thinking_levels"}))
            .await;
        let (Ok(state), Ok(models)) = (state, models) else {
            return json!([]);
        };
        let mut options = Vec::new();
        let current = model_value(
            state["provider"].as_str().unwrap_or_default(),
            state["modelId"].as_str().unwrap_or_default(),
        );
        let mut choices: Vec<Value> = models["models"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|model| {
                let id = model["id"].as_str().unwrap_or_default();
                let provider = model["provider"].as_str().unwrap_or_default();
                let name = model["name"]
                    .as_str()
                    .filter(|name| !name.is_empty())
                    .unwrap_or(id);
                json!({"value": model_value(provider, id), "name": name, "description": provider})
            })
            .collect();
        // Keep the current model first without filtering the full catalog.
        if let Some(index) = choices.iter().position(|c| c["value"] == current) {
            let selected = choices.remove(index);
            choices.insert(0, selected);
        }
        if !choices.is_empty() {
            options.push(json!({
                "id": "model",
                "name": "Model",
                "category": "model",
                "type": "select",
                "currentValue": current,
                "options": choices,
            }));
        }
        let level_choices: Vec<Value> = levels
            .ok()
            .and_then(|levels| levels["levels"].as_array().cloned())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|level| level.as_str().map(str::to_owned))
            .map(|level| json!({"value": level, "name": level}))
            .collect();
        if let (false, Some(current)) = (level_choices.is_empty(), state["thinkingLevel"].as_str())
        {
            options.push(json!({
                "id": "thinking",
                "name": "Thinking",
                "category": "thought_level",
                "type": "select",
                "currentValue": current,
                "options": level_choices,
            }));
        }
        Value::Array(options)
    }

    /// Advertise the session's slash commands (built-ins, extension
    /// commands, prompt templates, skills) with `available_commands_update`.
    pub(crate) async fn publish_commands(&self) {
        let Ok(native) = self.command(json!({"type": "get_commands"})).await else {
            return;
        };
        let commands = translate::available_commands(&native);
        self.notify_update(json!({
            "sessionUpdate": "available_commands_update",
            "availableCommands": commands,
        }))
        .await;
    }

    /// Tell the client the model/thinking options changed (after `/model`
    /// or `/thinking` ran as a slash command).
    pub(crate) async fn publish_config_options(&self) {
        let options = self.config_options().await;
        self.notify_update(json!({
            "sessionUpdate": "config_option_update",
            "configOptions": options,
        }))
        .await;
    }

    /// `session/set_config_option`. Changes apply to this session's lane only.
    pub(crate) async fn set_config_option(
        &self,
        config_id: &str,
        value: &str,
    ) -> Result<(), String> {
        match config_id {
            "model" => {
                let (provider, model) = value
                    .split_once('/')
                    .ok_or_else(|| format!("unknown model value `{value}`"))?;
                self.command(json!({"type": "set_model", "provider": provider, "modelId": model}))
                    .await
                    .map(drop)
            }
            "thinking" => self
                .command(json!({"type": "set_thinking_level", "level": value}))
                .await
                .map(drop),
            other => Err(format!("unknown config option `{other}`")),
        }
    }

    /// Stop the child: a graceful `stop`, then kill if it lingers.
    pub(crate) async fn shutdown(&self) {
        let _ = self.send(&json!({"type": "stop"})).await;
        let child = self.process.lock().unwrap().take();
        if let Some(mut child) = child {
            let exited = tokio::time::timeout(Duration::from_secs(3), child.wait()).await;
            if exited.is_err() {
                let _ = child.kill().await;
            }
        }
        self.closed.cancel();
    }
}

#[cfg(test)]
mod turn_tests {
    use super::*;
    use tokio::io::AsyncBufReadExt;

    #[tokio::test]
    async fn prompt_is_accepted_before_queued_abort() {
        let (child_stdin, child_input) = tokio::io::duplex(1024);
        let (client_out, _client_input) = tokio::io::duplex(1024);
        let session = Arc::new(Session {
            id: "test".into(),
            writer: Arc::new(Writer::new(Box::new(client_out))),
            elicitation_form: false,
            stdin: tokio::sync::Mutex::new(Box::new(child_stdin)),
            process: Mutex::new(None),
            next_id: AtomicU64::new(1),
            pending: Mutex::new(HashMap::new()),
            settled: Mutex::new(None),
            translator: Mutex::new(Translator::new(PathBuf::from("."))),
            turn: Mutex::new(None),
            turn_gate: tokio::sync::Mutex::new(()),
            closed: CancellationToken::new(),
        });
        let held_stdin = session.stdin.lock().await;
        let mut prompt = Box::pin(session.prompt("hello".into(), Vec::new()));
        assert!(matches!(
            futures::poll!(prompt.as_mut()),
            std::task::Poll::Pending
        ));
        let mut cancel = Box::pin(session.cancel());
        assert!(matches!(
            futures::poll!(cancel.as_mut()),
            std::task::Poll::Pending
        ));
        drop(held_stdin);
        assert!(matches!(
            futures::poll!(prompt.as_mut()),
            std::task::Poll::Pending
        ));
        let mut lines = tokio::io::BufReader::new(child_input).lines();
        let frame: Value =
            serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        assert_eq!(frame["type"], "prompt");
        assert!(matches!(
            futures::poll!(cancel.as_mut()),
            std::task::Poll::Pending
        ));
        let id = frame["id"].as_u64().unwrap();
        session
            .pending
            .lock()
            .unwrap()
            .remove(&id)
            .unwrap()
            .send(json!({"success": true}))
            .unwrap();
        assert!(matches!(
            futures::poll!(prompt.as_mut()),
            std::task::Poll::Pending
        ));
        tokio::time::timeout(Duration::from_secs(5), cancel)
            .await
            .unwrap();
        let abort: Value =
            serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        assert_eq!(abort["type"], "abort");
        session
            .settled
            .lock()
            .unwrap()
            .take()
            .unwrap()
            .send(json!({"outcome": "aborted"}))
            .unwrap();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(5), prompt)
                .await
                .unwrap()
                .unwrap(),
            "cancelled"
        );
    }

    #[tokio::test]
    async fn queued_cancel_does_not_abort_replacement_turn() {
        let (child_stdin, child_input) = tokio::io::duplex(1024);
        let (client_out, _client_input) = tokio::io::duplex(1024);
        let old = Arc::new(CancellationToken::new());
        let new = Arc::new(CancellationToken::new());
        let session = Arc::new(Session {
            id: "test".into(),
            writer: Arc::new(Writer::new(Box::new(client_out))),
            elicitation_form: false,
            stdin: tokio::sync::Mutex::new(Box::new(child_stdin)),
            process: Mutex::new(None),
            next_id: AtomicU64::new(1),
            pending: Mutex::new(HashMap::new()),
            settled: Mutex::new(None),
            translator: Mutex::new(Translator::new(PathBuf::from("."))),
            turn: Mutex::new(Some(old.clone())),
            turn_gate: tokio::sync::Mutex::new(()),
            closed: CancellationToken::new(),
        });
        let gate = session.turn_gate.lock().await;
        let mut cancel = Box::pin(session.cancel());
        assert!(matches!(
            futures::poll!(cancel.as_mut()),
            std::task::Poll::Pending
        ));
        *session.turn.lock().unwrap() = Some(new.clone());
        drop(gate);
        tokio::time::timeout(Duration::from_secs(5), cancel)
            .await
            .unwrap();
        assert!(!new.is_cancelled());
        assert!(!old.is_cancelled());
        let mut lines = tokio::io::BufReader::new(child_input).lines();
        assert!(
            tokio::time::timeout(Duration::from_millis(50), lines.next_line())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn queued_abort_finishes_before_old_turn_can_be_retired() {
        let (child_stdin, child_input) = tokio::io::duplex(1024);
        let (client_out, _client_input) = tokio::io::duplex(1024);
        let token = Arc::new(CancellationToken::new());
        let session = Arc::new(Session {
            id: "test".into(),
            writer: Arc::new(Writer::new(Box::new(client_out))),
            elicitation_form: false,
            stdin: tokio::sync::Mutex::new(Box::new(child_stdin)),
            process: Mutex::new(None),
            next_id: AtomicU64::new(1),
            pending: Mutex::new(HashMap::new()),
            settled: Mutex::new(None),
            translator: Mutex::new(Translator::new(PathBuf::from("."))),
            turn: Mutex::new(Some(token.clone())),
            turn_gate: tokio::sync::Mutex::new(()),
            closed: CancellationToken::new(),
        });
        let held_stdin = session.stdin.lock().await;
        let abort = tokio::spawn({
            let session = session.clone();
            async move { session.cancel().await }
        });
        tokio::time::timeout(Duration::from_secs(5), token.cancelled())
            .await
            .unwrap();
        let retirement = tokio::spawn({
            let session = session.clone();
            async move {
                let _gate = session.turn_gate.lock().await;
                session.turn.lock().unwrap().take();
            }
        });
        tokio::task::yield_now().await;
        assert!(
            !retirement.is_finished(),
            "turn must stay active while abort is queued"
        );
        drop(held_stdin);
        let mut lines = tokio::io::BufReader::new(child_input).lines();
        let line = tokio::time::timeout(Duration::from_secs(5), lines.next_line())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&line).unwrap()["type"],
            "abort"
        );
        tokio::time::timeout(Duration::from_secs(5), abort)
            .await
            .unwrap()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), retirement)
            .await
            .unwrap()
            .unwrap();
        assert!(session.turn.lock().unwrap().is_none());
    }
}

/// Config value for a model: `provider/id` (ids alone are not unique across
/// providers). The first `/` splits, so model ids may contain slashes.
fn model_value(provider: &str, id: &str) -> String {
    format!("{provider}/{id}")
}
