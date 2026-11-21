//! In-memory ACP server tests against a fake native `--mode rpc` child.

use super::session::{ChildIo, Spawner};
use super::*;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, Lines};

#[tokio::test]
async fn load_saved_session_replays_all_pages_before_response_and_can_prompt() {
    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path().to_str().unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    let saved = crate::session::create_jsonl_session_with_id(
        &crate::session::default_session_dir(temp.path()),
        cwd,
        Some(id.clone()),
    )
    .await
    .unwrap();
    drop(saved);
    let mut client = Client::start();
    client
        .call(
            "initialize",
            json!({"protocolVersion": 1, "clientCapabilities": {"elicitation": {"form": {}}}}),
        )
        .await;
    let (frames, response) = client
        .call(
            "session/load",
            json!({"sessionId": id, "cwd": cwd, "mcpServers": []}),
        )
        .await;
    assert!(response.get("error").is_none(), "{response}");
    assert_eq!(response["result"]["sessionId"], id);
    assert_eq!(
        update_kinds(&frames),
        ["user_message_chunk", "agent_message_chunk", "usage_update"]
    );
    assert_eq!(
        frames[0]["params"]["update"]["content"]["text"],
        "saved question"
    );
    assert_eq!(
        frames[1]["params"]["update"]["content"]["text"],
        "saved answer"
    );
    assert_eq!(
        usage_after_content(&frames),
        &json!({"sessionUpdate": "usage_update", "used": 0, "size": 8192})
    );
    let (_, duplicate) = client
        .call("session/load", json!({"sessionId": id, "cwd": cwd}))
        .await;
    assert_eq!(duplicate["error"]["code"], INVALID_PARAMS);
    let (_, response) = client
        .call(
            "session/prompt",
            json!({"sessionId": id, "prompt": [{"type": "text", "text": "continue"}]}),
        )
        .await;
    assert_eq!(response["result"]["stopReason"], "end_turn");
    let (requests, native) = question_in_session(
        &mut client,
        &id,
        json!({"question": "Continue after loading?", "options": ["Yes", "No"]}),
        accepted_question(json!({"answer": "Yes"})),
    )
    .await;
    assert_eq!(
        requests.len(),
        1,
        "loaded sessions keep the negotiated form capability"
    );
    assert_eq!(
        native["answer"],
        json!({"values": ["Yes"], "kind": "selector"})
    );
}

#[tokio::test]
async fn load_rejects_missing_invalid_and_relative_sessions_without_creating() {
    let temp = tempfile::tempdir().unwrap();
    let mut client = Client::start();
    let id = uuid::Uuid::new_v4().to_string();
    let (_, missing) = client
        .call("session/load", json!({"sessionId": id, "cwd": temp.path()}))
        .await;
    assert_eq!(missing["error"]["code"], RESOURCE_NOT_FOUND);
    let (_, invalid) = client
        .call(
            "session/load",
            json!({"sessionId": "../oops", "cwd": temp.path()}),
        )
        .await;
    assert_eq!(invalid["error"]["code"], INVALID_PARAMS);
    let (_, relative) = client
        .call("session/load", json!({"sessionId": id, "cwd": "."}))
        .await;
    assert_eq!(relative["error"]["code"], INVALID_PARAMS);
    assert!(
        crate::session::list_session_metadata(temp.path().to_str().unwrap())
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn load_rejects_corrupt_and_wrong_project_sessions() {
    let temp = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let dir = crate::session::default_session_dir(temp.path());
    let id = uuid::Uuid::new_v4().to_string();
    let saved = crate::session::create_jsonl_session_with_id(
        &dir,
        other.path().to_str().unwrap(),
        Some(id.clone()),
    )
    .await
    .unwrap();
    let mut client = Client::start();
    let (_, wrong) = client
        .call("session/load", json!({"sessionId": id, "cwd": temp.path()}))
        .await;
    assert_eq!(wrong["error"]["code"], INVALID_PARAMS);
    drop(saved);
    let discovery =
        crate::session::discover_session_metadata_in_dir(temp.path().to_str().unwrap(), &dir)
            .await
            .unwrap();
    std::fs::write(&discovery.sessions[0].path, "not a session header\n").unwrap();
    let (_, corrupt) = client
        .call("session/load", json!({"sessionId": id, "cwd": temp.path()}))
        .await;
    assert_eq!(corrupt["error"]["code"], RESOURCE_NOT_FOUND);
}

#[test]
fn replay_preserves_images_thoughts_and_tool_results() {
    let mut translator = translate::Translator::new(PathBuf::from("/tmp"));
    let image = translator.replay_message(&json!({"kind": "user", "content": [{"type": "image", "data": "abcd", "mimeType": "image/png"}]}));
    assert_eq!(image[0]["content"]["type"], "image");
    let assistant = translator.replay_message(&json!({"kind": "assistant", "content": [
        {"type": "thinking", "thinking": "reason"}, {"type": "text", "text": "answer"},
        {"type": "toolCall", "id": "call-1", "name": "read", "arguments": {"path": "a.txt"}}
    ]}));
    assert_eq!(assistant[0]["sessionUpdate"], "agent_thought_chunk");
    assert_eq!(assistant[1]["sessionUpdate"], "agent_message_chunk");
    assert_eq!(assistant[2]["sessionUpdate"], "tool_call");
    assert_eq!(assistant[2]["toolCallId"], "call-1");
    let result = translator.replay_message(&json!({"kind": "tool_result", "toolCallId": "call-1", "toolName": "read", "isError": true, "content": [{"type": "text", "text": "not found"}]}));
    assert_eq!(result[0]["toolCallId"], "call-1");
    assert_eq!(result[0]["status"], "failed");
    assert!(translator.take_stop_reason().is_none());
}

/// A fake child: answers state queries, streams a canned turn for `prompt`,
/// waits for `abort` when the prompt text is `wait`, and asks a permission
/// question when it is `ask`, or a serialized question after `question `.
async fn fake_child(stdin: DuplexStream, mut stdout: DuplexStream) {
    async fn emit(out: &mut DuplexStream, value: Value) {
        let mut line = serde_json::to_vec(&value).unwrap();
        line.push(b'\n');
        let _ = out.write_all(&line).await;
    }
    emit(&mut stdout, json!({"type": "ready"})).await;
    let mut lines = BufReader::new(stdin).lines();
    let mut waiting = false;
    let mut asked = false;
    let mut asked_question = false;
    let mut context_tokens = 0;
    let mut context_window = 8192;
    let mut cost_total = 0.0;
    let mut state_error = false;
    let mut compacting = false;
    while let Ok(Some(line)) = lines.next_line().await {
        let msg: Value = serde_json::from_str(&line).unwrap();
        let id = msg["id"].clone();
        let ok = |data: Value| json!({"type": "response", "id": id, "success": true, "data": data});
        match msg["type"].as_str().unwrap() {
            "get_state" => {
                let response = if state_error {
                    json!({"type": "response", "id": id, "success": false, "error": "state unavailable"})
                } else {
                    ok(json!({"modelId": "m1", "provider": "p", "thinkingLevel": "low",
                        "context": {"tokens": context_tokens, "window": context_window},
                        "stats": {"costTotal": cost_total}}))
                };
                emit(&mut stdout, response).await
            }
            "get_messages" => {
                let (messages, more, first) = if msg["beforeId"].is_null() {
                    (json!([{"kind": "assistant", "content": [{"type": "text", "text": "saved answer"}]}]), true, "answer")
                } else {
                    assert_eq!(msg["beforeId"], "answer");
                    (json!([{"kind": "user", "content": "saved question"}]), false, "question")
                };
                emit(&mut stdout, ok(json!({"messages": messages, "hasMore": more, "firstId": first}))).await;
            }
            "get_available_models" => {
                let mut models = vec![
                    json!({"id": "a/b", "name": "", "provider": "q"}),
                    json!({"id": "m1", "name": "M1", "provider": "p"}),
                ];
                models.extend((0..701).map(|index| {
                    json!({"id": format!("model-{index}"), "name": format!("Model {index}"), "provider": "omni"})
                }));
                emit(&mut stdout, ok(json!({"models": models}))).await
            }
            "get_commands" => {
                emit(
                    &mut stdout,
                    ok(json!({"commands": [
                        {"name": "compact", "description": "Compact", "source": "builtin", "hint": "focus"},
                        {"name": "skill:review", "description": "Review code", "source": "skill"},
                    ]})),
                )
                .await
            }
            "get_available_thinking_levels" => {
                emit(&mut stdout, ok(json!({"levels": ["off", "low", "high"]}))).await
            }
            "set_model" => {
                if msg["provider"] == "omni" {
                    assert_eq!(msg["modelId"], "model-700");
                } else {
                    assert_eq!(msg["provider"], "q");
                    assert_eq!(msg["modelId"], "a/b");
                }
                emit(&mut stdout, ok(json!({}))).await
            }
            "prompt" => {
                context_tokens = 1234;
                context_window = if msg["message"] == "no-window" { 0 } else { 8192 };
                cost_total = 0.125;
                state_error = msg["message"] == "state-error";
                if msg["message"].as_str().unwrap().trim() == "/compact reject" {
                    emit(&mut stdout, json!({"type": "response", "id": id, "success": false, "error": "cannot compact now"})).await;
                    continue;
                }
                if msg["message"] == "image regression" || msg["message"] == "" {
                    assert_eq!(msg["images"].as_array().unwrap().len(), 1);
                    assert_eq!(msg["images"][0]["mimeType"], "image/png");
                    assert_eq!(msg["images"][0]["data"], "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a9ZkAAAAASUVORK5CYII=");
                }
                emit(&mut stdout, ok(json!({"accepted": true}))).await;
                match msg["message"].as_str().unwrap() {
                    text if text.split_whitespace().next() == Some("/compact") => {
                        match text.split_whitespace().nth(1) {
                            Some("wait") => {
                                waiting = true;
                                compacting = true;
                                emit(&mut stdout, json!({"type": "command_output", "text": "compacting"})).await;
                            }
                            Some("exit") => break,
                            Some("fail") => emit(&mut stdout, json!({"type": "agent_settled", "outcome": "failed", "error": "compactor failed", "summary": "Compaction failed: compactor failed"})).await,
                            Some("decline") => emit(&mut stdout, json!({"type": "agent_settled", "outcome": "declined", "summary": "Nothing to compact yet."})).await,
                            _ => emit(&mut stdout, json!({"type": "agent_settled", "outcome": "compacted", "summary": "Conversation compacted."})).await,
                        }
                    }
                    "wait" => waiting = true,
                    "fail" => {
                        emit(&mut stdout, json!({"type": "message_update", "eventType": "text_delta",
                            "delta": "partial reply", "message": {"kind": "assistant"}})).await;
                        emit(&mut stdout, json!({"type": "agent_settled", "outcome": "failed", "error": "provider failed"})).await;
                    }
                    "/version" => {
                        emit(&mut stdout, json!({"type": "command_output", "text": "icers 0.0.0"})).await;
                        emit(&mut stdout, json!({"type": "agent_settled", "outcome": "completed"})).await;
                    }
                    "ask" => {
                        asked = true;
                        emit(&mut stdout, json!({"type": "tool_execution_start", "toolCallId": "t1", "toolName": "edit",
                            "args": {"path": "a.rs", "edits": []}})).await;
                        emit(&mut stdout, json!({"type": "ui_dialog_request", "requestId": "r1", "toolCallId": "t1",
                            "ui": {"purpose": "permission", "kind": "confirm", "summary": "Allow?",
                                "options": [{"title": "Deny", "permission": "reject_once"}, {"title": "Allow", "permission": "allow_once"}]}})).await;
                    }
                    question if question.starts_with("question ") => {
                        asked_question = true;
                        let ui: Value = serde_json::from_str(&question["question ".len()..]).unwrap();
                        emit(&mut stdout, json!({"type": "tool_execution_start", "toolCallId": "question-tool",
                            "toolName": "ask_user", "args": {}})).await;
                        emit(&mut stdout, json!({"type": "ui_dialog_request", "requestId": "question-1",
                            "toolCallId": "question-tool", "ui": ui})).await;
                    }
                    _ => {
                        emit(&mut stdout, json!({"type": "message_start", "message": {"kind": "assistant"}})).await;
                        emit(&mut stdout, json!({"type": "message_update", "eventType": "text_delta", "delta": "hel", "message": {"kind": "assistant"}})).await;
                        emit(&mut stdout, json!({"type": "message_end", "message": {"kind": "assistant", "stopReason": "stop",
                            "content": [{"type": "text", "text": "hello"}]}})).await;
                        emit(&mut stdout, json!({"type": "agent_settled", "outcome": "completed"})).await;
                    }
                }
            }
            "ui_dialog_response" if asked_question => {
                asked_question = false;
                emit(&mut stdout, json!({"type": "message_update", "eventType": "text_delta",
                    "delta": msg.to_string(), "message": {"kind": "assistant"}})).await;
                emit(&mut stdout, json!({"type": "agent_settled", "outcome": "completed"})).await;
            }
            "ui_dialog_response" if asked => {
                let answer = msg["answer"]["values"][0].as_str().unwrap_or("cancelled").to_string();
                let answer = if msg["cancelled"] == true { "cancelled".to_string() } else { answer };
                emit(&mut stdout, json!({"type": "message_update", "eventType": "text_delta", "delta": answer, "message": {"kind": "assistant"}})).await;
                emit(&mut stdout, json!({"type": "agent_settled", "outcome": "completed"})).await;
            }
            "abort" if waiting => {
                let mut settled = json!({"type": "agent_settled", "outcome": "aborted", "aborted": true});
                if compacting {
                    settled["summary"] = json!("Compaction aborted.");
                }
                emit(&mut stdout, settled).await;
            }
            "stop" => break,
            _ => {}
        }
    }
}

fn fake_spawner() -> Spawner {
    Arc::new(|_cwd: &Path, _id: &str, _saved_path: Option<&Path>| {
        let (child_stdin, fake_in) = tokio::io::duplex(1 << 16);
        let (fake_out, child_stdout) = tokio::io::duplex(1 << 16);
        tokio::spawn(fake_child(fake_in, fake_out));
        Ok(ChildIo {
            stdin: Box::new(child_stdin),
            stdout: Box::new(BufReader::new(child_stdout)),
            process: None,
        })
    })
}

struct Client {
    input: DuplexStream,
    output: Lines<BufReader<DuplexStream>>,
    next_id: u64,
}

impl Client {
    fn start() -> Self {
        Self::with_spawner(fake_spawner())
    }

    fn with_spawner(spawner: Spawner) -> Self {
        let (input, server_in) = tokio::io::duplex(1 << 16);
        let (server_out, output) = tokio::io::duplex(1 << 16);
        tokio::spawn(serve(BufReader::new(server_in), server_out, spawner, None));
        Self {
            input,
            output: BufReader::new(output).lines(),
            next_id: 1,
        }
    }

    async fn send(&mut self, frame: Value) {
        let mut line = serde_json::to_vec(&frame).unwrap();
        line.push(b'\n');
        self.input.write_all(&line).await.unwrap();
    }

    async fn next(&mut self) -> Value {
        let line = tokio::time::timeout(std::time::Duration::from_secs(5), self.output.next_line())
            .await
            .expect("frame in time")
            .unwrap()
            .expect("open stream");
        serde_json::from_str(&line).unwrap()
    }

    async fn request_id(&mut self, method: &str, params: Value) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
            .await;
        id
    }

    /// Send a request; return (notifications before the response, response).
    async fn call(&mut self, method: &str, params: Value) -> (Vec<Value>, Value) {
        let id = self.request_id(method, params).await;
        self.until_response(id).await
    }

    async fn until_response(&mut self, id: u64) -> (Vec<Value>, Value) {
        let mut seen = Vec::new();
        loop {
            let frame = self.next().await;
            if frame["id"] == id && frame.get("method").is_none() {
                return (seen, frame);
            }
            seen.push(frame);
        }
    }

    async fn new_session(&mut self) -> (String, Value) {
        let cwd = std::env::temp_dir();
        let (_, response) = self
            .call("session/new", json!({"cwd": cwd, "mcpServers": [{"name": "t3-code", "command": "x", "args": [], "env": []}]}))
            .await;
        let result = response["result"].clone();
        (
            result["sessionId"]
                .as_str()
                .expect("session id")
                .to_string(),
            result,
        )
    }
}

fn update_kinds(frames: &[Value]) -> Vec<String> {
    frames
        .iter()
        .filter(|f| f["method"] == "session/update")
        .map(|f| {
            f["params"]["update"]["sessionUpdate"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect()
}

fn usage_after_content(frames: &[Value]) -> &Value {
    let usage_indices: Vec<_> = frames
        .iter()
        .enumerate()
        .filter(|(_, frame)| frame["params"]["update"]["sessionUpdate"] == "usage_update")
        .map(|(index, _)| index)
        .collect();
    assert_eq!(
        usage_indices.len(),
        1,
        "one usage update before the response"
    );
    if let Some(last_content) = frames.iter().rposition(|frame| {
        matches!(
            frame["params"]["update"]["sessionUpdate"].as_str(),
            Some("user_message_chunk" | "agent_message_chunk" | "agent_thought_chunk")
        )
    }) {
        assert!(
            usage_indices[0] > last_content,
            "usage follows the last content update"
        );
    }
    &frames[usage_indices[0]]["params"]["update"]
}

async fn compaction_client(capability: Value) -> (Client, String) {
    let mut client = Client::start();
    client
        .call(
            "initialize",
            json!({"clientCapabilities": {"session": {"compaction": capability}}}),
        )
        .await;
    let (id, _) = client.new_session().await;
    (client, id)
}

fn compaction_updates(frames: &[Value]) -> Vec<&Value> {
    frames
        .iter()
        .filter_map(|frame| {
            let update = &frame["params"]["update"];
            (update["sessionUpdate"] == "compaction_update").then_some(update)
        })
        .collect()
}

#[tokio::test]
async fn compaction_reports_started_and_completed_before_prompt_response() {
    let (mut client, id) = compaction_client(json!({})).await;
    let mut previous_id = None;
    for (prompt, summary) in [
        ("/compact focus on tests", "Conversation compacted."),
        ("  /compact decline  ", "Nothing to compact yet."),
    ] {
        let (frames, response) = client
            .call(
                "session/prompt",
                json!({"sessionId": id, "prompt": [{"type": "text", "text": prompt}]}),
            )
            .await;
        assert_eq!(response["result"]["stopReason"], "end_turn", "{response}");
        let updates = compaction_updates(&frames);
        assert_eq!(updates.len(), 2, "{frames:?}");
        assert_eq!(updates[0]["status"], "in_progress");
        assert_eq!(updates[1]["status"], "completed");
        assert_eq!(updates[0]["compactionId"], updates[1]["compactionId"]);
        let compact_id = updates[0]["compactionId"].as_str().unwrap();
        assert!(uuid::Uuid::parse_str(compact_id).is_ok());
        assert_ne!(previous_id.as_deref(), Some(compact_id));
        previous_id = Some(compact_id.to_owned());
        assert_eq!(
            updates[1]["summary"],
            json!([{"type": "text", "text": summary}])
        );
        assert!(!update_kinds(&frames).contains(&"agent_message_chunk".to_string()));
    }
}

#[tokio::test]
async fn compaction_failure_and_child_exit_always_close_the_progress_row() {
    for (prompt, error) in [
        ("/compact fail", "compactor failed"),
        ("/compact reject", "cannot compact now"),
        ("/compact exit", "icers process exited during the turn"),
    ] {
        let (mut client, id) = compaction_client(json!({})).await;
        let (frames, response) = client
            .call(
                "session/prompt",
                json!({"sessionId": id, "prompt": [{"type": "text", "text": prompt}]}),
            )
            .await;
        assert_eq!(response["error"]["message"], error, "{response}");
        let updates = compaction_updates(&frames);
        assert_eq!(updates.len(), 2, "{frames:?}");
        assert_eq!(updates[0]["status"], "in_progress");
        assert_eq!(updates[1]["status"], "failed");
        assert_eq!(updates[0]["compactionId"], updates[1]["compactionId"]);
        assert_eq!(updates[1]["error"], error);
    }
}

#[tokio::test]
async fn compaction_cancellation_closes_the_same_progress_row() {
    let (mut client, id) = compaction_client(json!({})).await;
    let request = client
        .request_id(
            "session/prompt",
            json!({"sessionId": id, "prompt": [{"type": "text", "text": "/compact wait"}]}),
        )
        .await;
    let mut frames = Vec::new();
    loop {
        let frame = client.next().await;
        let ready = frame["params"]["update"]["content"]["text"] == "compacting";
        frames.push(frame);
        if ready {
            break;
        }
    }
    client
        .send(json!({"jsonrpc": "2.0", "method": "session/cancel", "params": {"sessionId": id}}))
        .await;
    let (terminal, response) = client.until_response(request).await;
    frames.extend(terminal);
    assert_eq!(response["result"]["stopReason"], "cancelled");
    let updates = compaction_updates(&frames);
    assert_eq!(updates.len(), 2);
    assert_eq!(updates[1]["status"], "cancelled");
    assert_eq!(updates[0]["compactionId"], updates[1]["compactionId"]);
    assert_eq!(updates[1]["summary"][0]["text"], "Compaction aborted.");
}

#[tokio::test]
async fn compaction_without_negotiation_keeps_the_text_summary() {
    for capability in [Value::Null, json!(true)] {
        let (mut client, id) = compaction_client(capability).await;
        let (frames, response) = client
            .call(
                "session/prompt",
                json!({"sessionId": id, "prompt": [{"type": "text", "text": "/compact"}]}),
            )
            .await;
        assert_eq!(response["result"]["stopReason"], "end_turn");
        assert!(compaction_updates(&frames).is_empty());
        assert!(
            frames
                .iter()
                .any(|frame| frame["params"]["update"]["content"]["text"]
                    == "Conversation compacted.")
        );
    }
}

#[tokio::test]
async fn compaction_requires_the_exact_slash_command_token() {
    let (mut client, id) = compaction_client(json!({})).await;
    for prompt in ["/compaction", "please /compact", "/compact-extra"] {
        let (frames, response) = client
            .call(
                "session/prompt",
                json!({"sessionId": id, "prompt": [{"type": "text", "text": prompt}]}),
            )
            .await;
        assert_eq!(response["result"]["stopReason"], "end_turn");
        assert!(compaction_updates(&frames).is_empty());
    }
}

#[test]
fn child_arguments_allow_observe_but_reject_bridge_owned_flags() {
    let args = ["--observe", "--offline", "--model", "provider/model"].map(str::to_owned);
    assert_eq!(check_child_args(&args), Ok(()));
    for flag in RESERVED_CHILD_FLAGS {
        for argument in [flag.to_string(), format!("{flag}=value")] {
            assert_eq!(check_child_args(&[argument.clone()]), Err(argument));
        }
    }
}

#[tokio::test]
async fn initialize_advertises_v1_without_unimplemented_capabilities() {
    let mut client = Client::start();
    let (_, response) = client
        .call(
            "initialize",
            json!({"protocolVersion": 2, "clientCapabilities": {}}),
        )
        .await;
    let result = &response["result"];
    assert_eq!(result["protocolVersion"], 1);
    assert_eq!(result["agentInfo"]["name"], "icers");
    assert_eq!(result["agentCapabilities"]["loadSession"], true);
    assert_eq!(
        result["agentCapabilities"]["promptCapabilities"]["image"],
        true
    );
    assert!(
        result.get("info").is_none(),
        "a v2-shaped response would switch T3 to v2"
    );
}

#[tokio::test]
async fn unknown_methods_and_sessions_are_errors() {
    let mut client = Client::start();
    let (_, response) = client.call("session/fork", json!({})).await;
    assert_eq!(response["error"]["code"], METHOD_NOT_FOUND);
    let (_, response) = client
        .call("session/prompt", json!({"sessionId": "nope", "prompt": []}))
        .await;
    assert_eq!(response["error"]["code"], RESOURCE_NOT_FOUND);
    let (_, response) = client.call("session/new", json!({"cwd": "relative"})).await;
    assert_eq!(response["error"]["code"], INVALID_PARAMS);
}

#[tokio::test]
async fn full_model_catalog_is_one_selector_without_pages() {
    let mut client = Client::start();
    let (session_id, result) = client.new_session().await;
    let options = result["configOptions"].as_array().unwrap();
    assert_eq!(options.len(), 2, "only model and thinking selectors");
    assert!(options.iter().all(|option| option["id"] != "model_catalog"));
    let model = options
        .iter()
        .find(|option| option["category"] == "model")
        .unwrap();
    let choices = model["options"].as_array().unwrap();
    assert_eq!(choices.len(), 703);
    assert_eq!(choices[0]["value"], "p/m1");
    assert_eq!(choices.last().unwrap()["value"], "omni/model-700");
    let (_, response) = client
        .call(
            "session/set_config_option",
            json!({
                "sessionId": session_id, "configId": "model", "value": "omni/model-700",
            }),
        )
        .await;
    assert!(response.get("error").is_none(), "{response}");
    let updated = response["result"]["configOptions"].as_array().unwrap();
    assert!(updated.iter().all(|option| option["id"] != "model_catalog"));
    let model = updated
        .iter()
        .find(|option| option["category"] == "model")
        .unwrap();
    assert_eq!(model["options"].as_array().unwrap().len(), 703);
}

#[tokio::test]
async fn session_new_accepts_client_mcp_servers_and_reports_config() {
    let mut client = Client::start();
    let (_, result) = client.new_session().await;
    let options = result["configOptions"].as_array().unwrap();
    let model = options.iter().find(|o| o["category"] == "model").unwrap();
    assert_eq!(model["currentValue"], "p/m1");
    assert_eq!(
        model["options"][0]["value"], "p/m1",
        "current model listed first"
    );
    assert_eq!(model["options"][1]["value"], "q/a/b");
    assert_eq!(model["options"][1]["name"], "a/b");
    let thinking = options
        .iter()
        .find(|o| o["category"] == "thought_level")
        .unwrap();
    assert_eq!(thinking["currentValue"], "low");
}

#[tokio::test]
async fn prompt_streams_then_resolves_on_settlement() {
    let mut client = Client::start();
    let (session, _) = client.new_session().await;
    let (frames, response) = client
        .call(
            "session/prompt",
            json!({"sessionId": session, "prompt": [{"type": "text", "text": "hi"}]}),
        )
        .await;
    assert_eq!(response["result"]["stopReason"], "end_turn");
    let text: String = frames
        .iter()
        .filter(|f| f["params"]["update"]["sessionUpdate"] == "agent_message_chunk")
        .map(|f| f["params"]["update"]["content"]["text"].as_str().unwrap())
        .collect();
    assert_eq!(text, "hello", "final text reconciled exactly once");
    assert!(frames.iter().all(|f| f["params"]["sessionId"] == session));
    assert_eq!(
        usage_after_content(&frames),
        &json!({"sessionUpdate": "usage_update", "used": 1234, "size": 8192,
            "cost": {"amount": 0.125, "currency": "USD"}})
    );
}

#[tokio::test]
async fn usage_query_failure_or_unknown_window_does_not_fail_the_prompt() {
    for message in ["state-error", "no-window"] {
        let mut client = Client::start();
        let (session, _) = client.new_session().await;
        let (frames, response) = client
            .call(
                "session/prompt",
                json!({
                    "sessionId": session, "prompt": [{"type": "text", "text": message}],
                }),
            )
            .await;
        assert_eq!(response["result"]["stopReason"], "end_turn", "{response}");
        assert!(!update_kinds(&frames).contains(&"usage_update".to_string()));
    }
}

#[tokio::test]
async fn usage_is_published_before_a_failed_prompt_response() {
    let mut client = Client::start();
    let (session, _) = client.new_session().await;
    let (frames, response) = client
        .call(
            "session/prompt",
            json!({
                "sessionId": session, "prompt": [{"type": "text", "text": "fail"}],
            }),
        )
        .await;
    assert_eq!(response["error"]["message"], "provider failed");
    assert_eq!(usage_after_content(&frames)["used"], 1234);
}

#[tokio::test]
async fn image_prompts_reach_the_native_child_and_invalid_images_are_recoverable() {
    let mut client = Client::start();
    let (session, _) = client.new_session().await;
    let (_, rejected) = client
        .call(
            "session/prompt",
            json!({"sessionId": session,
        "prompt": [{"type": "image", "mimeType": "image/png", "data": "invalid"}]}),
        )
        .await;
    assert_eq!(rejected["error"]["code"], INVALID_PARAMS);
    for text in [Some("image regression"), None] {
        let mut blocks = vec![json!({"type": "image", "mimeType": "image/png", "data":
            "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a9ZkAAAAASUVORK5CYII="})];
        if let Some(text) = text {
            blocks.insert(0, json!({"type": "text", "text": text}));
        }
        let (_, response) = client
            .call(
                "session/prompt",
                json!({"sessionId": session, "prompt": blocks}),
            )
            .await;
        assert_eq!(response["result"]["stopReason"], "end_turn", "{response}");
    }
    let (_, response) = client
        .call(
            "session/set_model",
            json!({"sessionId": session, "modelId": "q/a/b"}),
        )
        .await;
    assert_eq!(response["result"], json!({}));
}

#[tokio::test]
async fn cancel_resolves_prompt_as_cancelled() {
    let mut client = Client::start();
    let (session, _) = client.new_session().await;
    let id = client
        .request_id(
            "session/prompt",
            json!({"sessionId": session, "prompt": [{"type": "text", "text": "wait"}]}),
        )
        .await;
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    client
        .send(
            json!({"jsonrpc": "2.0", "method": "session/cancel", "params": {"sessionId": session}}),
        )
        .await;
    let (frames, response) = client.until_response(id).await;
    assert_eq!(response["result"]["stopReason"], "cancelled");
    assert_eq!(usage_after_content(&frames)["used"], 1234);
}

async fn permission_round_trip(answer: Value) -> String {
    let mut client = Client::start();
    let (session, _) = client.new_session().await;
    let id = client
        .request_id(
            "session/prompt",
            json!({"sessionId": session, "prompt": [{"type": "text", "text": "ask"}]}),
        )
        .await;
    let mut chunks = String::new();
    let mut frames = Vec::new();
    loop {
        let frame = client.next().await;
        frames.push(frame.clone());
        if frame["method"] == "session/request_permission" {
            assert!(
                update_kinds(&frames).contains(&"tool_call".to_string()),
                "the tool call is announced before its permission request"
            );
            let params = &frame["params"];
            assert_eq!(params["sessionId"], session);
            assert_eq!(params["toolCall"]["toolCallId"], "t1");
            assert_eq!(params["toolCall"]["kind"], "edit");
            assert!(params["toolCall"]["locations"][0]["path"]
                .as_str()
                .unwrap()
                .ends_with("a.rs"));
            let mut reply = json!({"jsonrpc": "2.0", "id": frame["id"]});
            match answer {
                Value::String(ref error) => reply["error"] = json!({"code": -1, "message": error}),
                ref result => reply["result"] = result.clone(),
            }
            client.send(reply).await;
            continue;
        }
        if frame["params"]["update"]["sessionUpdate"] == "agent_message_chunk" {
            chunks.push_str(
                frame["params"]["update"]["content"]["text"]
                    .as_str()
                    .unwrap(),
            );
        }
        if frame["id"] == id {
            assert_eq!(frame["result"]["stopReason"], "end_turn");
            return chunks;
        }
    }
}

#[tokio::test]
async fn permission_selection_answers_the_native_dialog() {
    let allowed =
        permission_round_trip(json!({"outcome": {"outcome": "selected", "optionId": "1"}})).await;
    assert_eq!(allowed, "Allow");
}

#[tokio::test]
async fn permission_cancel_or_client_error_denies() {
    let cancelled = permission_round_trip(json!({"outcome": {"outcome": "cancelled"}})).await;
    assert_eq!(cancelled, "cancelled");
    let failed = permission_round_trip(json!("boom")).await;
    assert_eq!(failed, "cancelled");
}

async fn question_round_trip(ui: Value, capabilities: Value, answer: Value) -> (Vec<Value>, Value) {
    let mut client = Client::start();
    let (_, initialized) = client
        .call(
            "initialize",
            json!({"protocolVersion": 1, "clientCapabilities": capabilities}),
        )
        .await;
    assert!(initialized.get("error").is_none(), "{initialized}");
    let (session, _) = client.new_session().await;
    question_in_session(&mut client, &session, ui, answer).await
}

async fn question_in_session(
    client: &mut Client,
    session: &str,
    mut ui: Value,
    answer: Value,
) -> (Vec<Value>, Value) {
    ui["purpose"] = json!("question");
    let id = client
        .request_id(
            "session/prompt",
            json!({"sessionId": session,
                "prompt": [{"type": "text", "text": format!("question {ui}")}]}),
        )
        .await;
    let mut requests = Vec::new();
    let mut chunks = String::new();
    loop {
        let frame = client.next().await;
        if frame["method"] == "session/elicitation" {
            assert_eq!(frame["params"]["sessionId"], session);
            assert_eq!(frame["params"]["toolCallId"], "question-tool");
            assert_eq!(frame["params"]["mode"], "form");
            requests.push(frame["params"].clone());
            let mut reply = json!({"jsonrpc": "2.0", "id": frame["id"]});
            match &answer {
                Value::String(error) => reply["error"] = json!({"code": -1, "message": error}),
                result => reply["result"] = result.clone(),
            }
            client.send(reply).await;
        } else if frame["params"]["update"]["sessionUpdate"] == "agent_message_chunk" {
            chunks.push_str(
                frame["params"]["update"]["content"]["text"]
                    .as_str()
                    .unwrap(),
            );
        } else if frame["id"] == id && frame.get("method").is_none() {
            assert_eq!(frame["result"]["stopReason"], "end_turn", "{frame}");
            return (requests, serde_json::from_str(&chunks).unwrap());
        }
    }
}

fn accepted_question(content: Value) -> Value {
    json!({"action": {"action": "accept", "content": content}})
}

#[tokio::test]
async fn question_form_single_choice_round_trips_t3_response() {
    let (requests, native) = question_round_trip(
        json!({"question": "Continue?", "header": "Next step", "context": "The build passed.",
            "options": [{"title": "Yes", "description": "Continue with the work"}, "No"]}),
        json!({"elicitation": {"form": {}}}),
        accepted_question(json!({"answer": "Yes"})),
    )
    .await;
    assert_eq!(requests.len(), 1, "a question must issue form elicitation");
    assert_eq!(requests[0]["requestedSchema"]["type"], "object");
    assert_eq!(requests[0]["requestedSchema"]["title"], "Next step");
    assert_eq!(
        requests[0]["requestedSchema"]["properties"]["answer"]["type"],
        "string"
    );
    let message = requests[0]["message"].as_str().unwrap();
    assert!(message.contains("Continue?") && message.contains("The build passed."));
    assert_eq!(
        native,
        json!({"type": "ui_dialog_response", "requestId": "question-1",
        "answer": {"values": ["Yes"], "kind": "selector"}})
    );
}

#[tokio::test]
async fn question_form_multi_choice_round_trips_tui_answer() {
    let (requests, native) = question_round_trip(
        json!({"question": "Choose targets", "allowMultiple": true,
            "options": ["Linux", "Windows", "macOS"]}),
        json!({"elicitation": {"form": {}}}),
        accepted_question(json!({"answer": ["Linux", "macOS"]})),
    )
    .await;
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0]["requestedSchema"]["properties"]["answer"]["type"],
        "array"
    );
    assert_eq!(
        native["answer"],
        json!({"values": ["Linux", "macOS"], "kind": "selector"})
    );
}

#[tokio::test]
async fn question_form_freeform_only_round_trips_tui_answer() {
    let (requests, native) = question_round_trip(
        json!({"questions": [{"question": "Name the branch", "kind": "input",
            "suggest": "Use a feat/ prefix"}]}),
        json!({"elicitation": {"form": {}}}),
        accepted_question(json!({"answer": "feat/questions"})),
    )
    .await;
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0]["requestedSchema"]["properties"]["answer"]["type"],
        "string"
    );
    let description = requests[0]["requestedSchema"]["properties"]["answer"]["description"]
        .as_str()
        .unwrap();
    assert!(description.contains("Name the branch"));
    assert!(description.contains("Use a feat/ prefix"));
    assert_eq!(
        native["answer"],
        json!({"values": ["feat/questions"], "value": "feat/questions",
        "text": "feat/questions", "kind": "input"})
    );
}

#[tokio::test]
async fn question_form_freeform_companion_can_replace_the_selection() {
    for content in [
        json!({"answer__other": "Custom target"}),
        json!({"answer": "Linux", "answer__other": "Custom target"}),
    ] {
        let (requests, native) = question_round_trip(
            json!({"question": "Choose a target", "allowFreeform": true, "options": ["Linux", "Windows"]}),
            json!({"elicitation": {"form": {}}}),
            accepted_question(content),
        )
        .await;
        assert_eq!(requests.len(), 1);
        let schema = &requests[0]["requestedSchema"];
        assert_eq!(schema["properties"]["answer__other"]["type"], "string");
        assert!(!schema["required"]
            .as_array()
            .is_some_and(|fields| fields.iter().any(|field| field == "answer")));
        assert_eq!(
            native["answer"],
            json!({"values": ["Custom target"], "value": "Custom target",
            "text": "Custom target", "kind": "input"})
        );
    }
}

#[tokio::test]
async fn question_form_confirm_round_trips_yes_and_no() {
    for choice in ["Yes", "No"] {
        let (requests, native) = question_round_trip(
            json!({"question": "Continue?", "kind": "confirm"}),
            json!({"elicitation": {"form": {}}}),
            accepted_question(json!({"answer": choice})),
        )
        .await;
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0]["requestedSchema"]["properties"]["answer"]["type"],
            "string"
        );
        assert_eq!(
            native["answer"],
            json!({"values": [choice], "kind": "selector"})
        );
    }
}

#[tokio::test]
async fn question_form_decline_cancel_error_and_invalid_accept_cancel_the_dialog() {
    for answer in [
        json!({"action": {"action": "decline"}}),
        json!({"action": {"action": "cancel"}}),
        json!("client failed"),
        accepted_question(json!({})),
        accepted_question(json!({"answer": "unoffered choice"})),
        accepted_question(json!({"answer": ["Yes"]})),
    ] {
        let (requests, native) = question_round_trip(
            json!({"question": "Continue?", "options": ["Yes", "No"]}),
            json!({"elicitation": {"form": {}}}),
            answer,
        )
        .await;
        assert_eq!(requests.len(), 1);
        assert_eq!(
            native,
            json!({"type": "ui_dialog_response", "requestId": "question-1", "cancelled": true})
        );
    }
}

#[tokio::test]
async fn question_form_without_capability_cancels_without_a_request() {
    for capabilities in [
        json!({}),
        json!({"elicitation": {}}),
        json!({"elicitation": {"form": null}}),
        json!({"elicitation": {"form": false}}),
        json!({"elicitation": {"form": true}}),
    ] {
        let (requests, native) = question_round_trip(
            json!({"question": "Continue?", "options": ["Yes", "No"]}),
            capabilities,
            accepted_question(json!({"answer": "Yes"})),
        )
        .await;
        assert!(requests.is_empty());
        assert_eq!(
            native,
            json!({"type": "ui_dialog_response", "requestId": "question-1", "cancelled": true})
        );
    }
}

#[tokio::test]
async fn question_form_child_exit_while_waiting_does_not_hang() {
    let (exit_tx, exit_rx) = tokio::sync::oneshot::channel::<()>();
    let exit_rx = Arc::new(std::sync::Mutex::new(Some(exit_rx)));
    let spawner: Spawner = Arc::new(move |_cwd: &Path, _id: &str, _saved_path: Option<&Path>| {
        let (child_stdin, fake_in) = tokio::io::duplex(1 << 16);
        let (fake_out, child_stdout) = tokio::io::duplex(1 << 16);
        let exit_rx = exit_rx.lock().unwrap().take().unwrap();
        tokio::spawn(async move {
            tokio::select! {
                _ = fake_child(fake_in, fake_out) => {},
                _ = exit_rx => {},
            }
        });
        Ok(ChildIo {
            stdin: Box::new(child_stdin),
            stdout: Box::new(BufReader::new(child_stdout)),
            process: None,
        })
    });
    let mut client = Client::with_spawner(spawner);
    client
        .call(
            "initialize",
            json!({"protocolVersion": 1,
        "clientCapabilities": {"elicitation": {"form": {}}}}),
        )
        .await;
    let (session, _) = client.new_session().await;
    let ui = json!({"purpose": "question", "question": "Continue?", "options": ["Yes", "No"]});
    let id = client
        .request_id(
            "session/prompt",
            json!({"sessionId": session,
        "prompt": [{"type": "text", "text": format!("question {ui}")}]}),
        )
        .await;
    loop {
        let frame = client.next().await;
        if frame["method"] == "session/elicitation" {
            break;
        }
        assert!(
            frame["id"] != id || frame.get("method").is_some(),
            "the question must remain pending until the client answers: {frame}"
        );
    }
    exit_tx.send(()).unwrap();
    let (_, response) = client.until_response(id).await;
    assert!(response.get("error").is_some(), "{response}");
    let (_, initialized) = client
        .call("initialize", json!({"protocolVersion": 1}))
        .await;
    assert_eq!(initialized["result"]["protocolVersion"], 1);
}

#[tokio::test]
async fn question_request_drop_removes_the_pending_waiter() {
    let (out, input) = tokio::io::duplex(1 << 16);
    let writer = Writer::new(Box::new(out));
    let mut output = BufReader::new(input).lines();
    let mut request = Box::pin(writer.request("session/elicitation", json!({"sessionId": "test"})));
    let frame = tokio::select! {
        result = request.as_mut() => panic!("request completed before a response: {result:?}"),
        line = output.next_line() => serde_json::from_str::<Value>(&line.unwrap().unwrap()).unwrap(),
    };
    assert_eq!(frame["method"], "session/elicitation");
    assert_eq!(writer.pending.lock().unwrap().len(), 1);
    drop(request);
    assert!(
        writer.pending.lock().unwrap().is_empty(),
        "closing a session must release its unanswered form request"
    );
}

#[tokio::test]
async fn set_config_option_switches_model_in_session() {
    let mut client = Client::start();
    let (session, _) = client.new_session().await;
    let (_, response) = client
        .call(
            "session/set_config_option",
            json!({"sessionId": session, "configId": "model", "value": "q/a/b"}),
        )
        .await;
    assert!(response["result"]["configOptions"].is_array(), "{response}");
    let (_, response) = client
        .call(
            "session/set_config_option",
            json!({"sessionId": session, "configId": "nope", "value": "x"}),
        )
        .await;
    assert_eq!(response["error"]["code"], INVALID_PARAMS);
}

#[tokio::test]
async fn session_new_advertises_slash_commands_after_responding() {
    let mut client = Client::start();
    let (session, _) = client.new_session().await;
    let frame = client.next().await;
    assert_eq!(frame["method"], "session/update");
    assert_eq!(frame["params"]["sessionId"], session);
    let update = &frame["params"]["update"];
    assert_eq!(update["sessionUpdate"], "available_commands_update");
    assert_eq!(update["availableCommands"][0]["name"], "compact");
    assert_eq!(update["availableCommands"][0]["input"]["hint"], "focus");
    assert_eq!(update["availableCommands"][1]["name"], "skill:review");
}

#[tokio::test]
async fn slash_command_output_is_streamed_as_agent_text() {
    let mut client = Client::start();
    let (session, _) = client.new_session().await;
    let (frames, response) = client
        .call(
            "session/prompt",
            json!({"sessionId": session, "prompt": [{"type": "text", "text": "/version"}]}),
        )
        .await;
    assert_eq!(response["result"]["stopReason"], "end_turn");
    let text: String = frames
        .iter()
        .filter(|f| f["params"]["update"]["sessionUpdate"] == "agent_message_chunk")
        .map(|f| f["params"]["update"]["content"]["text"].as_str().unwrap())
        .collect();
    assert_eq!(text, "icers 0.0.0");
}
