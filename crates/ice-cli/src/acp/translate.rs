//! Pure mapping from native `--mode rpc` events to ACP v1 `session/update`
//! payloads. No I/O: the session pump feeds events in and writes what comes out.
//!
//! Behaviour reference: upstream `rpi-acp` (`pi-rust/rpi-package`, MIT),
//! `packages/rpi-acp/src/session.rs` at `9906990d`.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Per-session translation state.
///
/// Native RPC may drop events when its broadcast lags, so text deltas already
/// streamed for the current assistant message are tracked; `message_end`
/// sends whatever suffix is still missing instead of duplicating chunks.
pub(crate) struct Translator {
    cwd: PathBuf,
    text_sent: String,
    thought_sent: String,
    last_stop_reason: Option<String>,
    /// `tool_call` updates of running tools, so a permission request for the
    /// call can carry the same title, kind and locations.
    tools: HashMap<String, Value>,
}

impl Translator {
    pub(crate) fn new(cwd: PathBuf) -> Self {
        Self {
            cwd,
            text_sent: String::new(),
            thought_sent: String::new(),
            last_stop_reason: None,
            tools: HashMap::new(),
        }
    }

    pub(crate) fn clone_for_replay(&self) -> Self {
        Self::new(self.cwd.clone())
    }

    /// Map persisted branch messages without executing any historical tools.
    pub(crate) fn replay_message(&mut self, message: &Value) -> Vec<Value> {
        let mut updates = Vec::new();
        match message["kind"]
            .as_str()
            .or_else(|| message["role"].as_str())
        {
            Some("user") => {
                if let Some(text) = message["content"].as_str() {
                    updates.push(chunk("user_message_chunk", text));
                } else {
                    for block in message["content"].as_array().into_iter().flatten() {
                        let content = match block["type"].as_str() {
                            Some("text") => Some(json!({"type": "text", "text": block["text"]})),
                            Some("image") => Some(
                                json!({"type": "image", "data": block["data"], "mimeType": block["mimeType"]}),
                            ),
                            _ => None,
                        };
                        if let Some(content) = content {
                            updates.push(
                                json!({"sessionUpdate": "user_message_chunk", "content": content}),
                            );
                        }
                    }
                }
            }
            Some("assistant") => {
                for block in message["content"].as_array().into_iter().flatten() {
                    match block["type"].as_str() {
                        Some("text") => updates.push(chunk(
                            "agent_message_chunk",
                            block["text"].as_str().unwrap_or_default(),
                        )),
                        Some("thinking") => updates.push(chunk(
                            "agent_thought_chunk",
                            block["thinking"].as_str().unwrap_or_default(),
                        )),
                        Some("toolCall") => updates.extend(self.translate(&json!({
                            "type": "tool_execution_start", "toolCallId": block["id"],
                            "toolName": block["name"], "args": block["arguments"],
                        }))),
                        _ => {}
                    }
                }
            }
            Some("tool_result" | "toolResult") => updates.extend(self.translate(&json!({
                "type": "tool_execution_end", "toolCallId": message["toolCallId"],
                "toolName": message["toolName"], "isError": message["isError"],
                "result": {"content": message["content"], "details": message["details"]},
            }))),
            _ => {}
        }
        updates
    }

    /// The ACP `ToolCallUpdate` describing a running tool call, for
    /// `session/request_permission`.
    pub(crate) fn tool_call(&self, tool_call_id: &str) -> Option<Value> {
        let call = self.tools.get(tool_call_id)?;
        let mut update = json!({"toolCallId": tool_call_id});
        for key in ["title", "kind", "locations", "rawInput"] {
            if let Some(value) = call.get(key) {
                update[key] = value.clone();
            }
        }
        Some(update)
    }

    /// The final assistant `stopReason` seen since the last call, if any.
    pub(crate) fn take_stop_reason(&mut self) -> Option<String> {
        self.last_stop_reason.take()
    }

    /// Translate one native event into zero or more ACP `session/update`
    /// `update` objects (the value under `params.update`).
    pub(crate) fn translate(&mut self, event: &Value) -> Vec<Value> {
        match event.get("type").and_then(Value::as_str) {
            Some("message_start") if is_assistant(event) => {
                self.text_sent.clear();
                self.thought_sent.clear();
                Vec::new()
            }
            Some("message_update") => self.message_update(event),
            // Output of a text-only slash command (no model turn).
            Some("command_output") => match event["text"].as_str() {
                Some(text) if !text.is_empty() => vec![chunk("agent_message_chunk", text)],
                _ => Vec::new(),
            },
            Some("message_end") if is_assistant(event) => self.message_end(event),
            Some("tool_execution_start") => {
                let update = self.tool_start(event);
                if let Some(id) = event["toolCallId"].as_str() {
                    self.tools.insert(id.to_string(), update.clone());
                }
                vec![update]
            }
            Some("tool_execution_update") => tool_update(event).into_iter().collect(),
            Some("tool_execution_end") => {
                if let Some(id) = event["toolCallId"].as_str() {
                    self.tools.remove(id);
                }
                vec![tool_end(event)]
            }
            _ => Vec::new(),
        }
    }

    fn message_update(&mut self, event: &Value) -> Vec<Value> {
        let Some(delta) = event.get("delta").and_then(Value::as_str) else {
            return Vec::new();
        };
        if delta.is_empty() {
            return Vec::new();
        }
        match event.get("eventType").and_then(Value::as_str) {
            Some("text_delta") => {
                self.text_sent.push_str(delta);
                vec![chunk("agent_message_chunk", delta)]
            }
            Some("thinking_delta") => {
                self.thought_sent.push_str(delta);
                vec![chunk("agent_thought_chunk", delta)]
            }
            _ => Vec::new(),
        }
    }

    fn message_end(&mut self, event: &Value) -> Vec<Value> {
        let message = &event["message"];
        self.last_stop_reason = message
            .get("stopReason")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let (text, thought) = message_text(message);
        let mut out = Vec::new();
        if let Some(rest) = missing_suffix(&self.thought_sent, &thought) {
            out.push(chunk("agent_thought_chunk", rest));
        }
        if let Some(rest) = missing_suffix(&self.text_sent, &text) {
            out.push(chunk("agent_message_chunk", rest));
        }
        self.text_sent.clear();
        self.thought_sent.clear();
        out
    }

    fn tool_start(&self, event: &Value) -> Value {
        let name = event["toolName"].as_str().unwrap_or("tool");
        let args = &event["args"];
        let mut update = json!({
            "sessionUpdate": "tool_call",
            "toolCallId": event["toolCallId"],
            "title": tool_title(name, args),
            "kind": tool_kind(name),
            "status": "in_progress",
            "rawInput": args,
        });
        let locations = tool_locations(name, args, &self.cwd);
        if !locations.is_empty() {
            update["locations"] = Value::Array(locations);
        }
        if let Some(diff) = tool_diff(name, args, &self.cwd) {
            update["content"] = json!([diff]);
        }
        update
    }
}

fn is_assistant(event: &Value) -> bool {
    event["message"].get("kind").and_then(Value::as_str) == Some("assistant")
}

fn chunk(kind: &str, text: &str) -> Value {
    json!({"sessionUpdate": kind, "content": {"type": "text", "text": text}})
}

/// Concatenated text and thinking blocks of a serialized assistant message.
fn message_text(message: &Value) -> (String, String) {
    let mut text = String::new();
    let mut thought = String::new();
    for block in message["content"].as_array().into_iter().flatten() {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => text.push_str(block["text"].as_str().unwrap_or_default()),
            Some("thinking") => thought.push_str(block["thinking"].as_str().unwrap_or_default()),
            _ => {}
        }
    }
    (text, thought)
}

/// The part of `full` not yet streamed, when what was streamed is a prefix of
/// it. A non-prefix means deltas went missing mid-stream; re-sending the whole
/// text would duplicate, so nothing is added.
fn missing_suffix<'a>(sent: &str, full: &'a str) -> Option<&'a str> {
    let rest = full.strip_prefix(sent)?;
    (!rest.is_empty()).then_some(rest)
}

fn tool_update(event: &Value) -> Option<Value> {
    let text = result_text(&event["partialResult"]);
    if text.is_empty() {
        return None;
    }
    Some(json!({
        "sessionUpdate": "tool_call_update",
        "toolCallId": event["toolCallId"],
        "status": "in_progress",
        "content": [text_content(&text)],
    }))
}

fn tool_end(event: &Value) -> Value {
    let failed = event["isError"].as_bool().unwrap_or(false);
    let result = &event["result"];
    let mut update = json!({
        "sessionUpdate": "tool_call_update",
        "toolCallId": event["toolCallId"],
        "status": if failed { "failed" } else { "completed" },
        "rawOutput": result,
    });
    let text = result_text(result);
    // Keep the edit diff sent at start; replacing it with the summary text
    // would hide the change from the client.
    let is_edit = matches!(event["toolName"].as_str(), Some("edit" | "write"));
    if !text.is_empty() && (failed || !is_edit) {
        update["content"] = json!([text_content(&text)]);
    }
    update
}

fn text_content(text: &str) -> Value {
    json!({"type": "content", "content": {"type": "text", "text": text}})
}

fn result_text(result: &Value) -> String {
    result["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|block| block["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// ACP `ToolKind` for a built-in tool. Clients such as T3 Code decide
/// auto-approval from this, so mutating tools must never map to `read`.
pub(crate) fn tool_kind(name: &str) -> &'static str {
    match name {
        "read" => "read",
        "write" | "edit" => "edit",
        "bash" => "execute",
        "grep" | "find" | "ls" => "search",
        "webfetch" | "websearch" => "fetch",
        _ => "other",
    }
}

fn tool_title(name: &str, args: &Value) -> String {
    let detail = match name {
        "bash" => args["command"].as_str(),
        "grep" | "find" => args["pattern"].as_str(),
        _ => args["path"].as_str(),
    };
    match detail {
        Some(detail) if !detail.is_empty() => {
            let line = detail.lines().next().unwrap_or(detail);
            let short: String = line.chars().take(120).collect();
            format!("{name} {short}")
        }
        _ => name.to_string(),
    }
}

/// Absolute file locations a built-in tool touches. T3 Code's auto-accept
/// policy denies edits without locations, so file tools always carry one.
pub(crate) fn tool_locations(name: &str, args: &Value, cwd: &Path) -> Vec<Value> {
    let path = match name {
        "read" | "write" | "edit" | "ls" | "grep" | "find" => args["path"].as_str(),
        _ => None,
    };
    let Some(path) = path.filter(|path| !path.is_empty()) else {
        return Vec::new();
    };
    let absolute = resolve(cwd, path);
    let mut location = json!({"path": absolute.to_string_lossy()});
    if name == "read" {
        if let Some(line) = args["offset"].as_u64() {
            location["line"] = json!(line);
        }
    }
    vec![location]
}

fn tool_diff(name: &str, args: &Value, cwd: &Path) -> Option<Value> {
    let path = resolve(cwd, args["path"].as_str()?);
    let path = path.to_string_lossy();
    match name {
        "write" => Some(json!({
            "type": "diff",
            "path": path,
            "oldText": Value::Null,
            "newText": args["content"].as_str()?,
        })),
        "edit" => {
            let edits = args["edits"].as_array()?;
            let pieces = |key: &str| {
                edits
                    .iter()
                    .filter_map(|edit| edit[key].as_str())
                    .collect::<Vec<_>>()
                    .join("\n…\n")
            };
            Some(json!({
                "type": "diff",
                "path": path,
                "oldText": pieces("oldText"),
                "newText": pieces("newText"),
            }))
        }
        _ => None,
    }
}

fn resolve(cwd: &Path, path: &str) -> PathBuf {
    let path = path.strip_prefix('@').unwrap_or(path);
    let expanded = match path.strip_prefix("~/") {
        Some(rest) => dirs::home_dir().map(|home| home.join(rest)),
        None => None,
    };
    let path = expanded.unwrap_or_else(|| PathBuf::from(path));
    if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    }
}

/// ACP `availableCommands` from the native `get_commands` list.
pub(crate) fn available_commands(native: &Value) -> Vec<Value> {
    native["commands"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|command| {
            let name = command["name"].as_str()?;
            let mut entry = json!({
                "name": name,
                "description": command["description"].as_str().unwrap_or_default(),
            });
            if let Some(hint) = command["hint"].as_str() {
                entry["input"] = json!({"hint": hint});
            }
            Some(entry)
        })
        .collect()
}

/// Map a native `agent_settled` outcome to an ACP `stopReason`, or `Err` with
/// the failure message. `stop_reason` is the last assistant message's native
/// stop reason, used to report `max_tokens` only when it actually happened.
pub(crate) fn stop_reason(
    settled: &Value,
    cancelled: bool,
    stop_reason: Option<&str>,
) -> Result<&'static str, String> {
    match settled["outcome"].as_str().unwrap_or("completed") {
        "aborted" => Ok("cancelled"),
        "completed" if stop_reason == Some("length") => Ok("max_tokens"),
        "completed" => Ok("end_turn"),
        _ if cancelled => Ok("cancelled"),
        _ => Err(settled["error"]
            .as_str()
            .unwrap_or("the agent run failed")
            .to_string()),
    }
}

/// Option titles of a native permission dialog, when every option declares a
/// standard ACP `PermissionOptionKind`. Other dialogs (free-form questions)
/// return `None`; they have no `session/request_permission` equivalent.
pub(crate) fn permission_options(ui: &Value) -> Option<Vec<(String, String)>> {
    if ui["purpose"].as_str() != Some("permission") {
        return None;
    }
    let options = ui["options"].as_array()?;
    let mapped: Vec<(String, String)> = options
        .iter()
        .map(|option| {
            let title = option["title"].as_str()?.to_string();
            let kind = option["permission"].as_str()?;
            matches!(
                kind,
                "allow_once" | "allow_always" | "reject_once" | "reject_always"
            )
            .then(|| (title, kind.to_string()))
        })
        .collect::<Option<_>>()?;
    (!mapped.is_empty()).then_some(mapped)
}

/// `session/request_permission` params (minus `sessionId`) for a dialog.
pub(crate) fn permission_params(
    ui: &Value,
    options: &[(String, String)],
    tool_call: Option<Value>,
    tool_call_id: &str,
) -> Value {
    let summary = ui["summary"]
        .as_str()
        .or_else(|| ui["question"].as_str())
        .unwrap_or("Allow this tool call?");
    let mut tool_call = tool_call.unwrap_or_else(|| json!({"toolCallId": tool_call_id}));
    if tool_call.get("title").is_none() {
        tool_call["title"] = json!(summary);
    }
    let options: Vec<Value> = options
        .iter()
        .enumerate()
        .map(|(index, (title, kind))| {
            json!({"optionId": index.to_string(), "name": title, "kind": kind})
        })
        .collect();
    json!({"toolCall": tool_call, "options": options})
}

/// The native `ui_dialog_response` for a client's permission outcome. Only an
/// explicit selection of an offered option answers the dialog; a cancelled,
/// malformed or failed response cancels it, which the gate treats as a denial.
pub(crate) fn permission_answer(
    request_id: &str,
    options: &[(String, String)],
    response: Result<Value, String>,
) -> Value {
    let selected = response.ok().and_then(|result| {
        let outcome = &result["outcome"];
        if outcome["outcome"].as_str() != Some("selected") {
            return None;
        }
        let index: usize = outcome["optionId"].as_str()?.parse().ok()?;
        options.get(index).map(|(title, _)| title.clone())
    });
    match selected {
        Some(title) => json!({
            "type": "ui_dialog_response",
            "requestId": request_id,
            "answer": {"values": [title]},
        }),
        None => json!({"type": "ui_dialog_response", "requestId": request_id, "cancelled": true}),
    }
}

/// Preserve inline images separately from the flattened text/editor context.
/// Audio and URL-only images are not supported.
pub(crate) fn prompt_content(
    blocks: &[Value],
) -> Result<(String, Vec<ice_ai::types::ImageContent>), String> {
    let mut parts = Vec::new();
    let mut images = Vec::new();
    for block in blocks {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => parts.push(block["text"].as_str().unwrap_or_default().to_string()),
            Some("image") => {
                images.extend(crate::rpc::protocol::prompt_images(Some(&json!([block])))?);
            }
            Some("resource_link") => {
                let uri = block["uri"].as_str().unwrap_or_default();
                let name = block["name"].as_str().unwrap_or(uri);
                parts.push(format!("[{name}]({uri})"));
            }
            Some("resource") => {
                let resource = &block["resource"];
                let uri = resource["uri"].as_str().unwrap_or_default();
                match resource["text"].as_str() {
                    Some(text) => {
                        parts.push(format!("<context uri=\"{uri}\">\n{text}\n</context>"))
                    }
                    None => parts.push(format!("[binary resource]({uri})")),
                }
            }
            Some(other) => return Err(format!("unsupported prompt content type `{other}`")),
            None => return Err("prompt content block without a type".to_string()),
        }
    }
    Ok((parts.join("\n\n"), images))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn translator() -> Translator {
        Translator::new(PathBuf::from("/work"))
    }

    fn delta(kind: &str, text: &str) -> Value {
        json!({"type": "message_update", "eventType": kind, "delta": text, "message": {"kind": "assistant"}})
    }

    fn assistant_end(text: &str, thinking: &str, stop: &str) -> Value {
        json!({"type": "message_end", "message": {
            "kind": "assistant",
            "stopReason": stop,
            "content": [{"type": "thinking", "thinking": thinking}, {"type": "text", "text": text}],
        }})
    }

    #[test]
    fn streams_text_and_thinking_chunks() {
        let mut t = translator();
        let text = t.translate(&delta("text_delta", "hi"));
        assert_eq!(text[0]["sessionUpdate"], "agent_message_chunk");
        assert_eq!(text[0]["content"]["text"], "hi");
        let thought = t.translate(&delta("thinking_delta", "hmm"));
        assert_eq!(thought[0]["sessionUpdate"], "agent_thought_chunk");
    }

    #[test]
    fn message_end_fills_only_missing_suffix() {
        let mut t = translator();
        t.translate(&json!({"type": "message_start", "message": {"kind": "assistant"}}));
        t.translate(&delta("text_delta", "hel"));
        let out = t.translate(&assistant_end("hello", "", "stop"));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["content"]["text"], "lo");
        assert_eq!(t.take_stop_reason().as_deref(), Some("stop"));
    }

    #[test]
    fn message_end_never_duplicates_complete_or_gapped_text() {
        let mut t = translator();
        t.translate(&delta("text_delta", "hello"));
        assert!(t.translate(&assistant_end("hello", "", "stop")).is_empty());
        // A gap mid-stream: "ho" is not a prefix of "hello", so nothing is resent.
        t.translate(&delta("text_delta", "ho"));
        assert!(t.translate(&assistant_end("hello", "", "stop")).is_empty());
    }

    #[test]
    fn user_message_end_is_ignored() {
        let mut t = translator();
        let event = json!({"type": "message_end", "message": {"kind": "user", "content": [{"type": "text", "text": "q"}]}});
        assert!(t.translate(&event).is_empty());
    }

    #[test]
    fn edit_tool_call_has_kind_absolute_location_and_diff() {
        let mut t = translator();
        let out = t.translate(&json!({
            "type": "tool_execution_start",
            "toolCallId": "c1",
            "toolName": "edit",
            "args": {"path": "src/a.rs", "edits": [{"oldText": "a", "newText": "b"}]},
        }));
        let call = &out[0];
        assert_eq!(call["sessionUpdate"], "tool_call");
        assert_eq!(call["kind"], "edit");
        assert_eq!(call["status"], "in_progress");
        assert_eq!(
            call["locations"][0]["path"],
            Path::new("/work/src/a.rs").to_string_lossy().as_ref()
        );
        assert_eq!(call["content"][0]["type"], "diff");
        assert_eq!(call["content"][0]["oldText"], "a");
        assert_eq!(call["content"][0]["newText"], "b");
    }

    #[test]
    fn bash_is_execute_without_locations() {
        let mut t = translator();
        let out = t.translate(&json!({
            "type": "tool_execution_start", "toolCallId": "c2", "toolName": "bash",
            "args": {"command": "cargo test\n--all"},
        }));
        assert_eq!(out[0]["kind"], "execute");
        assert_eq!(out[0]["title"], "bash cargo test");
        assert!(out[0].get("locations").is_none());
    }

    #[test]
    fn tool_end_reports_status_and_text() {
        let mut t = translator();
        let ok = t.translate(&json!({
            "type": "tool_execution_end", "toolCallId": "c3", "toolName": "read",
            "isError": false, "result": {"content": [{"type": "text", "text": "body"}]},
        }));
        assert_eq!(ok[0]["status"], "completed");
        assert_eq!(ok[0]["content"][0]["content"]["text"], "body");
        let failed = t.translate(&json!({
            "type": "tool_execution_end", "toolCallId": "c4", "toolName": "edit",
            "isError": true, "result": {"content": [{"type": "text", "text": "no match"}]},
        }));
        assert_eq!(failed[0]["status"], "failed");
        assert_eq!(failed[0]["content"][0]["content"]["text"], "no match");
    }

    #[test]
    fn command_output_becomes_agent_text() {
        let mut t = translator();
        let out = t.translate(&json!({"type": "command_output", "text": "Model set."}));
        assert_eq!(out[0]["sessionUpdate"], "agent_message_chunk");
        assert_eq!(out[0]["content"]["text"], "Model set.");
        assert!(t
            .translate(&json!({"type": "command_output", "text": ""}))
            .is_empty());
    }

    #[test]
    fn maps_native_commands_to_acp() {
        let commands = available_commands(&json!({"commands": [
            {"name": "compact", "description": "Compact", "source": "builtin", "hint": "focus"},
            {"name": "skill:review", "description": "Review", "source": "skill"},
        ]}));
        assert_eq!(
            commands[0],
            json!({"name": "compact", "description": "Compact", "input": {"hint": "focus"}})
        );
        assert!(commands[1].get("input").is_none());
    }

    #[test]
    fn maps_settlement_to_stop_reason() {
        let done = json!({"outcome": "completed"});
        assert_eq!(stop_reason(&done, false, Some("stop")), Ok("end_turn"));
        assert_eq!(stop_reason(&done, false, Some("length")), Ok("max_tokens"));
        assert_eq!(
            stop_reason(&json!({"outcome": "aborted"}), true, None),
            Ok("cancelled")
        );
        let failed = json!({"outcome": "failed", "error": "boom"});
        assert_eq!(stop_reason(&failed, false, None), Err("boom".to_string()));
        assert_eq!(stop_reason(&failed, true, None), Ok("cancelled"));
    }

    fn mcp_permission_ui() -> Value {
        json!({"purpose": "permission", "kind": "confirm", "summary": "Allow MCP tool x?",
            "options": [{"title": "Deny", "permission": "reject_once"}, {"title": "Allow", "permission": "allow_once"}]})
    }

    #[test]
    fn permission_request_reuses_tool_call_details() {
        let mut t = translator();
        t.translate(&json!({
            "type": "tool_execution_start", "toolCallId": "c5", "toolName": "write",
            "args": {"path": "/work/out.txt", "content": "x"},
        }));
        let ui = mcp_permission_ui();
        let options = permission_options(&ui).unwrap();
        let params = permission_params(&ui, &options, t.tool_call("c5"), "c5");
        assert_eq!(params["toolCall"]["kind"], "edit");
        assert_eq!(params["toolCall"]["locations"][0]["path"], "/work/out.txt");
        assert_eq!(params["options"][1]["kind"], "allow_once");
        assert_eq!(params["options"][1]["optionId"], "1");
    }

    #[test]
    fn non_permission_dialogs_are_not_mapped() {
        assert!(permission_options(&json!({"purpose": "question", "options": []})).is_none());
        let bad =
            json!({"purpose": "permission", "options": [{"title": "Yes", "permission": "maybe"}]});
        assert!(permission_options(&bad).is_none());
    }

    #[test]
    fn permission_answer_fails_closed() {
        let options = permission_options(&mcp_permission_ui()).unwrap();
        let allow = json!({"outcome": {"outcome": "selected", "optionId": "1"}});
        assert_eq!(
            permission_answer("r", &options, Ok(allow))["answer"]["values"][0],
            "Allow"
        );
        for response in [
            Ok(json!({"outcome": {"outcome": "cancelled"}})),
            Ok(json!({"outcome": {"outcome": "selected", "optionId": "9"}})),
            Ok(json!({})),
            Err("client gone".to_string()),
        ] {
            assert_eq!(
                permission_answer("r", &options, response)["cancelled"],
                true
            );
        }
    }

    #[test]
    fn preserves_mixed_and_image_only_prompts() {
        // A real one-pixel PNG; ACP supplies raw base64, not a data URL.
        let image = json!({"type": "image", "mimeType": "image/png", "data":
            "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a9ZkAAAAASUVORK5CYII="});
        for text in [None, Some("describe this")] {
            let mut blocks = Vec::new();
            if let Some(text) = text {
                blocks.push(json!({"type": "text", "text": text}));
            }
            blocks.push(image.clone());
            let (actual, images) = prompt_content(&blocks).unwrap();
            assert_eq!(actual, text.unwrap_or_default());
            assert_eq!(images.len(), 1);
            assert_eq!(images[0].data, image["data"].as_str().unwrap());
            assert_eq!(images[0].mime_type, "image/png");
        }
        let mut mismatch = image.clone();
        mismatch["mimeType"] = json!("image/jpeg");
        assert!(prompt_content(&[mismatch]).is_err());
        assert!(prompt_content(&[
            json!({"type": "image", "mimeType": "image/png", "uri": "https://example.com/a.png"})
        ])
        .is_err());
    }

    #[test]
    fn flattens_prompt_blocks_and_rejects_invalid_images() {
        let (text, images) = prompt_content(&[
            json!({"type": "text", "text": "fix it"}),
            json!({"type": "resource", "resource": {"uri": "file:///a.rs", "text": "fn a() {}"}}),
            json!({"type": "resource_link", "uri": "file:///b.rs", "name": "b.rs"}),
        ])
        .unwrap();
        assert!(text.starts_with("fix it"));
        assert!(text.contains("<context uri=\"file:///a.rs\">\nfn a() {}\n</context>"));
        assert!(text.ends_with("[b.rs](file:///b.rs)"));
        assert!(images.is_empty());
        assert!(prompt_content(&[json!({"type": "image", "data": "x"})]).is_err());
        assert!(prompt_content(&[json!({"type": "audio", "data": "x"})]).is_err());
    }
}
