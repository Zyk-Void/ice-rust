//! In-memory ACP server tests against a fake native `--mode rpc` child.

use super::session::{ChildIo, Spawner};
use super::*;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, Lines};

fn list_fixture(
    dir: &Path,
    cwd: &Path,
    index: usize,
    modified_ms: u64,
    body: &str,
) -> (String, PathBuf) {
    let id = uuid::Uuid::from_u128(index as u128 + 1).to_string();
    std::fs::create_dir_all(dir).unwrap();
    let path = dir.join(format!("{index:04}.jsonl"));
    let header = json!({"kind": "header", "version": 4, "id": id, "createdAt": 0, "cwd": cwd});
    std::fs::write(&path, format!("{header}\n{body}")).unwrap();
    let time = std::time::UNIX_EPOCH + std::time::Duration::from_millis(modified_ms);
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(time))
        .unwrap();
    (id, path)
}

#[tokio::test]
async fn list_filters_shared_directories_and_reads_live_names_without_writes() {
    let temp = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let shared = tempfile::tempdir().unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    let saved = crate::session::create_jsonl_session_with_id(
        shared.path(),
        temp.path().to_str().unwrap(),
        Some(id.clone()),
    )
    .await
    .unwrap();
    saved.storage().set_name(Some("First name")).await.unwrap();
    saved
        .storage()
        .set_name(Some("Current name"))
        .await
        .unwrap();
    let foreign = list_fixture(shared.path(), other.path(), 500, 1_700_000_000_000, "");
    let meta = crate::session::discover_session_metadata_in_dir(
        temp.path().to_str().unwrap(),
        shared.path(),
    )
    .await
    .unwrap()
    .sessions
    .into_iter()
    .find(|m| m.id == id)
    .unwrap();
    let before = std::fs::read(&meta.path).unwrap();
    let mut client = Client::with_session_dir(Some(shared.path().to_path_buf()));
    let (_, response) = client
        .call("session/list", json!({"cwd": temp.path()}))
        .await;
    assert!(response.get("error").is_none(), "{response}");
    let sessions = response["result"]["sessions"].as_array().unwrap();
    assert_eq!(sessions.len(), 1, "{response}");
    assert_eq!(sessions[0]["sessionId"], id);
    assert_eq!(sessions[0]["title"], "Current name");
    assert_eq!(
        sessions[0]["cwd"],
        json!(temp.path().canonicalize().unwrap())
    );
    assert!(sessions[0]["updatedAt"].as_str().unwrap().ends_with('Z'));
    assert!(response["result"].get("nextCursor").is_none());
    assert_eq!(std::fs::read(&meta.path).unwrap(), before);
    assert!(foreign.1.exists());
    saved.storage().set_name(Some("Still owned")).await.unwrap();
}

#[tokio::test]
async fn list_pages_stably_and_binds_cursors_to_the_requested_project() {
    let temp = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let dir = crate::session::default_session_dir(temp.path());
    let mut expected = Vec::new();
    for index in 0..51 {
        let modified = if index < 25 {
            1_700_000_001_000
        } else {
            1_700_000_000_000
        };
        expected.push(list_fixture(&dir, temp.path(), index, modified, "").0);
    }
    let mut client = Client::start();
    let (_, first) = client
        .call("session/list", json!({"cwd": temp.path()}))
        .await;
    assert!(first.get("error").is_none(), "{first}");
    let page = first["result"]["sessions"].as_array().unwrap();
    assert_eq!(page.len(), 50);
    assert_eq!(page[0]["updatedAt"], "2023-11-14T22:13:21.000Z");
    let cursor = first["result"]["nextCursor"].as_str().unwrap();
    // Updating a not-yet-returned file must not move it ahead of the cursor.
    let path = dir.join("0050.jsonl");
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(
            std::time::UNIX_EPOCH + std::time::Duration::from_millis(1_800_000_000_000),
        ))
        .unwrap();
    let (_, second) = client
        .call(
            "session/list",
            json!({"cwd": temp.path(), "cursor": cursor}),
        )
        .await;
    let mut actual: Vec<_> = page
        .iter()
        .map(|session| session["sessionId"].as_str().unwrap().to_owned())
        .collect();
    actual.extend(
        second["result"]["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|session| session["sessionId"].as_str().unwrap().to_owned()),
    );
    assert_eq!(actual, expected);
    assert!(second["result"].get("nextCursor").is_none());
    let (_, wrong_project) = client
        .call(
            "session/list",
            json!({"cwd": other.path(), "cursor": cursor}),
        )
        .await;
    assert_eq!(wrong_project["error"]["code"], INVALID_PARAMS);
    std::fs::remove_file(dir.join("0049.jsonl")).unwrap();
    let (_, after_delete) = client
        .call(
            "session/list",
            json!({"cwd": temp.path(), "cursor": cursor}),
        )
        .await;
    assert_eq!(
        after_delete["result"]["sessions"],
        second["result"]["sessions"]
    );
}

#[tokio::test]
async fn list_rejects_invalid_scope_and_cursor_without_creating_sessions() {
    let temp = tempfile::tempdir().unwrap();
    let mut client = Client::start();
    for params in [
        json!({}),
        json!({"cwd": "."}),
        json!({"cwd": temp.path().join("missing")}),
        json!({"cwd": temp.path(), "cursor": 3}),
        json!({"cwd": temp.path(), "cursor": "not-a-cursor"}),
    ] {
        let (_, response) = client.call("session/list", params).await;
        assert_eq!(response["error"]["code"], INVALID_PARAMS, "{response}");
    }
    let (_, empty) = client
        .call("session/list", json!({"cwd": temp.path(), "cursor": null}))
        .await;
    assert_eq!(empty["result"], json!({"sessions": []}));
    assert!(!crate::session::default_session_dir(temp.path()).exists());
}

#[tokio::test]
async fn list_names_scan_the_complete_snapshot_and_ignore_partial_tail_facts() {
    let temp = tempfile::tempdir().unwrap();
    let dir = crate::session::default_session_dir(temp.path());
    let filler =
        json!({"kind": "lane", "seq": 1, "lane": "padding".repeat(40_000), "leafId": null});
    let mut tail = filler.clone();
    tail["seq"] = json!(3);
    let body = format!("{filler}\n{{\"kind\":\"fact\",\"fact\":\"name\",\"seq\":2,\"name\":\"Middle name\"}}\n{tail}\n{{\"kind\":\"fact\",\"fact\":\"name\",\"seq\":4,\"name\":\"Partial name\"}}");
    let (id, path) = list_fixture(&dir, temp.path(), 0, 1_700_000_000_000, &body);
    let before = std::fs::read(&path).unwrap();
    let mut client = Client::start();
    let (_, response) = client
        .call("session/list", json!({"cwd": temp.path()}))
        .await;
    assert_eq!(
        response["result"]["sessions"][0]["sessionId"], id,
        "{response}"
    );
    assert_eq!(response["result"]["sessions"][0]["title"], "Middle name");
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let clear = "{\"kind\":\"fact\",\"fact\":\"name\",\"seq\":1,\"name\":\"Old\"}\n{\"kind\":\"fact\",\"fact\":\"name\",\"seq\":2,\"name\":null}\n";
    list_fixture(&dir, temp.path(), 0, 1_700_000_000_000, clear);
    let (_, cleared) = client
        .call("session/list", json!({"cwd": temp.path()}))
        .await;
    assert!(cleared["result"]["sessions"][0].get("title").is_none());
    let large = format!(
        "{{\"kind\":\"fact\",\"fact\":\"name\",\"seq\":1,\"name\":\"Old\"}}\n{}",
        "x".repeat(1024 * 1024)
    );
    list_fixture(&dir, temp.path(), 0, 1_700_000_000_000, &large);
    let (_, oversized) = client
        .call("session/list", json!({"cwd": temp.path()}))
        .await;
    assert!(oversized["result"]["sessions"][0].get("title").is_none());
}

#[tokio::test]
async fn resume_restores_config_without_history_and_publishes_commands_after_response() {
    let temp = tempfile::tempdir().unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    let saved = crate::session::create_jsonl_session_with_id(
        &crate::session::default_session_dir(temp.path()),
        temp.path().to_str().unwrap(),
        Some(id.clone()),
    )
    .await
    .unwrap();
    drop(saved);
    let mut client = Client::start();
    let (frames, response) = client
        .call(
            "session/resume",
            json!({"sessionId": id, "cwd": temp.path()}),
        )
        .await;
    assert!(response.get("error").is_none(), "{response}");
    assert!(response["result"].get("sessionId").is_none());
    assert!(
        frames.iter().all(|frame| matches!(
            frame["params"]["update"]["sessionUpdate"].as_str(),
            Some("usage_update" | "session_info_update")
        )),
        "resume cannot replay historical updates: {frames:?}"
    );
    let options = response["result"]["configOptions"].as_array().unwrap();
    assert_eq!(
        options
            .iter()
            .find(|option| option["id"] == "model")
            .unwrap()["currentValue"],
        "q/a/b"
    );
    assert_eq!(
        options
            .iter()
            .find(|option| option["id"] == "thinking")
            .unwrap()["currentValue"],
        "high"
    );
    let commands = client.next().await;
    assert_eq!(commands["params"]["sessionId"], id);
    assert_eq!(
        commands["params"]["update"]["sessionUpdate"],
        "available_commands_update"
    );
    let (frames, prompt) = client
        .call(
            "session/prompt",
            json!({"sessionId": id, "prompt": [{"type": "text", "text": "history pages"}]}),
        )
        .await;
    assert_eq!(prompt["result"]["stopReason"], "end_turn");
    assert_eq!(
        frames[0]["params"]["update"]["content"]["text"], "1",
        "resume reads only the config page"
    );
}

#[tokio::test]
async fn resume_and_load_of_one_session_cannot_create_two_owners() {
    let temp = tempfile::tempdir().unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    let saved = crate::session::create_jsonl_session_with_id(
        &crate::session::default_session_dir(temp.path()),
        temp.path().to_str().unwrap(),
        Some(id.clone()),
    )
    .await
    .unwrap();
    drop(saved);
    let mut client = Client::start();
    let load = client
        .request_id("session/load", json!({"sessionId": id, "cwd": temp.path()}))
        .await;
    let resume = client
        .request_id(
            "session/resume",
            json!({"sessionId": id, "cwd": temp.path()}),
        )
        .await;
    let mut responses = Vec::new();
    while responses.len() < 2 {
        let frame = client.next().await;
        if frame.get("method").is_none() && (frame["id"] == load || frame["id"] == resume) {
            responses.push(frame);
        }
    }
    assert_eq!(
        responses
            .iter()
            .filter(|response| response.get("result").is_some())
            .count(),
        1,
        "{responses:?}"
    );
    let error = responses
        .iter()
        .find(|response| response.get("error").is_some())
        .unwrap();
    assert_eq!(error["error"]["code"], INVALID_PARAMS);
    assert!(error["error"]["message"]
        .as_str()
        .unwrap()
        .contains("already open"));
}

#[tokio::test]
async fn resume_validates_saved_identity_and_cwd_without_creating_replacements() {
    let temp = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let dir = crate::session::default_session_dir(temp.path());
    let id = uuid::Uuid::new_v4().to_string();
    let mut client = Client::start();
    for (params, code) in [
        (
            json!({"sessionId": id, "cwd": temp.path()}),
            RESOURCE_NOT_FOUND,
        ),
        (
            json!({"sessionId": "../escape", "cwd": temp.path()}),
            INVALID_PARAMS,
        ),
        (json!({"sessionId": id, "cwd": "."}), INVALID_PARAMS),
        (
            json!({"sessionId": id, "cwd": temp.path(), "mcpServers": {}}),
            INVALID_PARAMS,
        ),
    ] {
        let (_, response) = client.call("session/resume", params).await;
        assert_eq!(response["error"]["code"], code, "{response}");
    }
    assert!(!dir.exists());
    let saved = crate::session::create_jsonl_session_with_id(
        &dir,
        other.path().to_str().unwrap(),
        Some(id.clone()),
    )
    .await
    .unwrap();
    let path = saved.storage().persistence_path().unwrap();
    drop(saved);
    let (_, wrong) = client
        .call(
            "session/resume",
            json!({"sessionId": id, "cwd": temp.path()}),
        )
        .await;
    assert_eq!(wrong["error"]["code"], INVALID_PARAMS);
    std::fs::write(&path, "invalid header\n").unwrap();
    let (_, corrupt) = client
        .call(
            "session/resume",
            json!({"sessionId": id, "cwd": temp.path()}),
        )
        .await;
    assert_eq!(corrupt["error"]["code"], RESOURCE_NOT_FOUND);
    assert_eq!(std::fs::read_to_string(path).unwrap(), "invalid header\n");
}

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
        [
            "user_message_chunk",
            "agent_message_chunk",
            "session_info_update"
        ]
    );
    assert_eq!(
        frames[0]["params"]["update"]["content"]["text"],
        "saved question"
    );
    assert_eq!(
        frames[1]["params"]["update"]["content"]["text"],
        "saved answer"
    );
    assert_eq!(frames[2]["params"]["update"]["title"], "saved title");
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
async fn fake_child(stdin: DuplexStream, mut stdout: DuplexStream, cwd: PathBuf) {
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
    let mut provider = "p".to_string();
    let mut model_id = "m1".to_string();
    let mut thinking_level = "low".to_string();
    let mut history_pages = 0;
    let mut session_name = None::<String>;
    let mut state_unavailable = false;
    let mut compacting = false;
    while let Ok(Some(line)) = lines.next_line().await {
        let msg: Value = serde_json::from_str(&line).unwrap();
        let id = msg["id"].clone();
        let ok = |data: Value| json!({"type": "response", "id": id, "success": true, "data": data});
        match msg["type"].as_str().unwrap() {
            "fork_session" => {
                emit(&mut stdout, ok(json!({"sessionId": uuid::Uuid::new_v4().to_string(), "path": cwd.join("fake-fork.jsonl")}))).await;
            }
            "get_state" => {
                if state_unavailable {
                    emit(&mut stdout, json!({"type": "response", "id": id, "success": false, "error": "state unavailable"})).await;
                } else {
                    emit(&mut stdout, ok(json!({"modelId": model_id, "provider": provider, "thinkingLevel": thinking_level, "sessionName": session_name}))).await;
                }
            }
            "get_messages" => {
                history_pages += 1;
                session_name = Some("saved title".to_string());
                let (messages, more, first) = if msg["beforeId"].is_null() {
                    (json!([{"kind": "assistant", "content": [{"type": "text", "text": "saved answer"}]}]), true, "answer")
                } else {
                    assert_eq!(msg["beforeId"], "answer");
                    (json!([{"kind": "user", "content": "saved question"}]), false, "question")
                };
                emit(&mut stdout, ok(json!({"messages": messages, "hasMore": more, "firstId": first,
                    "savedConfig": {"provider": "q", "modelId": "a/b", "thinkingLevel": "high"}}))).await;
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
                provider = msg["provider"].as_str().unwrap().to_string();
                model_id = msg["modelId"].as_str().unwrap().to_string();
                if msg.get("persist").is_some() {
                    assert_eq!(msg["persist"], false);
                }
                emit(&mut stdout, ok(json!({}))).await
            }
            "set_thinking_level" => {
                thinking_level = msg["level"].as_str().unwrap().to_string();
                if msg.get("persist").is_some() {
                    assert_eq!(msg["persist"], false);
                }
                emit(&mut stdout, ok(json!({}))).await
            }
            "prompt" => {
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
                    "history pages" => {
                        emit(&mut stdout, json!({"type": "command_output", "text": history_pages.to_string()})).await;
                        emit(&mut stdout, json!({"type": "agent_settled", "outcome": "completed"})).await;
                    }
                    "wait-fork" => {
                        waiting = true;
                        emit(&mut stdout, json!({"type": "command_output", "text": "fork blocked"})).await;
                    }
                    text if text.split_whitespace().next() == Some("/name") => {
                        if let Some((_, args)) = text.trim().split_once(char::is_whitespace) {
                            let args = args.trim();
                            match args {
                                "refused" => {},
                                "unavailable" => state_unavailable = true,
                                "" => {},
                                _ => session_name = Some(args.to_string()),
                            }
                        }
                        emit(&mut stdout, json!({"type": "command_output", "text": "name command finished"})).await;
                        emit(&mut stdout, json!({"type": "agent_settled", "outcome": "completed"})).await;
                    }
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
            "abort" if waiting || asked_question => {
                asked_question = false;
                waiting = false;
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
        tokio::spawn(fake_child(fake_in, fake_out, _cwd.to_path_buf()));
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
        Self::with_options(spawner, None)
    }

    fn with_session_dir(session_dir: Option<PathBuf>) -> Self {
        Self::with_options(fake_spawner(), session_dir)
    }

    fn with_options(spawner: Spawner, session_dir: Option<PathBuf>) -> Self {
        let (input, server_in) = tokio::io::duplex(1 << 16);
        let (server_out, output) = tokio::io::duplex(1 << 16);
        tokio::spawn(serve(
            BufReader::new(server_in),
            server_out,
            spawner,
            session_dir,
        ));
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

#[tokio::test]
async fn session_title_uses_native_name_before_prompt_response() {
    let mut client = Client::start();
    let (id, _) = client.new_session().await;
    let (frames, response) = client.call("session/prompt", json!({"sessionId": id, "prompt": [{"type": "text", "text": "  /name  Build release  "}]})).await;
    assert_eq!(response["result"]["stopReason"], "end_turn");
    let titles: Vec<_> = frames
        .iter()
        .filter(|frame| frame["params"]["update"]["sessionUpdate"] == "session_info_update")
        .collect();
    assert_eq!(titles.len(), 1, "{frames:?}");
    assert_eq!(titles[0]["params"]["update"]["title"], "Build release");
    assert!(titles[0]["params"]["update"].get("updatedAt").is_none());
}

#[tokio::test]
async fn session_title_skips_ordinary_and_incomplete_name_prompts() {
    let mut client = Client::start();
    let (id, _) = client.new_session().await;
    for prompt in [
        "hello",
        "/name",
        "  /name   ",
        "/names release",
        "please /name release",
    ] {
        let (frames, response) = client
            .call(
                "session/prompt",
                json!({"sessionId": id, "prompt": [{"type": "text", "text": prompt}]}),
            )
            .await;
        assert_eq!(response["result"]["stopReason"], "end_turn");
        assert!(
            !update_kinds(&frames).contains(&"session_info_update".to_string()),
            "{frames:?}"
        );
    }
}

#[tokio::test]
async fn session_title_does_not_invent_requested_name_after_failure() {
    let mut client = Client::start();
    let (id, _) = client.new_session().await;
    client
        .call(
            "session/prompt",
            json!({"sessionId": id, "prompt": [{"type": "text", "text": "/name Original title"}]}),
        )
        .await;
    for prompt in ["/name refused", "/name unavailable"] {
        let (frames, response) = client
            .call(
                "session/prompt",
                json!({"sessionId": id, "prompt": [{"type": "text", "text": prompt}]}),
            )
            .await;
        assert_eq!(response["result"]["stopReason"], "end_turn");
        assert!(
            !update_kinds(&frames).contains(&"session_info_update".to_string()),
            "{frames:?}"
        );
    }
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
        result["agentCapabilities"]["sessionCapabilities"]["list"],
        json!({})
    );
    assert_eq!(
        result["agentCapabilities"]["sessionCapabilities"]["resume"],
        json!({})
    );
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
    let (_, response) = client.call("session/not-implemented", json!({})).await;
    assert_eq!(response["error"]["code"], METHOD_NOT_FOUND);
    let (_, response) = client
        .call("session/prompt", json!({"sessionId": "nope", "prompt": []}))
        .await;
    assert_eq!(response["error"]["code"], RESOURCE_NOT_FOUND);
    let (_, response) = client.call("session/new", json!({"cwd": "relative"})).await;
    assert_eq!(response["error"]["code"], INVALID_PARAMS);
}

#[tokio::test]
async fn close_removes_the_session_and_rejects_unknown_ids() {
    let mut client = Client::start();
    let (_, init) = client.call("initialize", json!({})).await;
    assert_eq!(
        init["result"]["agentCapabilities"]["sessionCapabilities"]["close"],
        json!({})
    );
    let (id, _) = client.new_session().await;
    let (_, closed) = client.call("session/close", json!({"sessionId": id})).await;
    assert_eq!(closed["result"], json!({}));
    let (_, absent) = client
        .call("session/prompt", json!({"sessionId": id, "prompt": []}))
        .await;
    assert_eq!(absent["error"]["code"], RESOURCE_NOT_FOUND);
    let (_, repeated) = client.call("session/close", json!({"sessionId": id})).await;
    assert_eq!(repeated["error"]["code"], RESOURCE_NOT_FOUND);
    let (_, invalid) = client.call("session/close", json!({})).await;
    assert_eq!(invalid["error"]["code"], INVALID_PARAMS);
}

#[tokio::test]
async fn close_allows_the_saved_session_to_be_loaded_again() {
    let temp = tempfile::tempdir().unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    let saved = crate::session::create_jsonl_session_with_id(
        &crate::session::default_session_dir(temp.path()),
        temp.path().to_str().unwrap(),
        Some(id.clone()),
    )
    .await
    .unwrap();
    drop(saved);
    let mut client = Client::start();
    let (_, loaded) = client
        .call("session/load", json!({"sessionId": id, "cwd": temp.path()}))
        .await;
    assert!(loaded.get("error").is_none(), "{loaded}");
    let (_, closed) = client.call("session/close", json!({"sessionId": id})).await;
    assert!(closed.get("error").is_none(), "{closed}");
    let (_, reloaded) = client
        .call("session/load", json!({"sessionId": id, "cwd": temp.path()}))
        .await;
    assert!(reloaded.get("error").is_none(), "{reloaded}");
}

#[tokio::test]
async fn close_releases_an_in_flight_prompt() {
    let mut client = Client::start();
    let (id, _) = client.new_session().await;
    let prompt = client
        .request_id(
            "session/prompt",
            json!({"sessionId": id, "prompt": [{"type":"text", "text":"wait"}]}),
        )
        .await;
    let close = client
        .request_id("session/close", json!({"sessionId": id}))
        .await;
    let mut seen = std::collections::HashSet::new();
    while seen.len() < 2 {
        let frame = client.next().await;
        if frame.get("method").is_none() && frame["id"] == close {
            assert_eq!(frame["result"], json!({}), "{frame}");
            seen.insert(close);
        } else if frame.get("method").is_none() && frame["id"] == prompt {
            assert!(frame.get("error").is_some(), "{frame}");
            seen.insert(prompt);
        }
    }
}

#[tokio::test]
async fn fork_creates_an_independent_session_and_replays_before_response() {
    let mut client = Client::start();
    let (_, init) = client.call("initialize", json!({})).await;
    assert!(init["result"]["agentCapabilities"]["sessionCapabilities"]["fork"].is_object());
    let (source, _) = client.new_session().await;
    let (frames, response) = client
        .call(
            "session/fork",
            json!({"sessionId": source, "cwd": std::env::temp_dir()}),
        )
        .await;
    assert!(response.get("error").is_none(), "{response}");
    let fork = response["result"]["sessionId"].as_str().unwrap();
    assert_ne!(fork, source);
    assert!(uuid::Uuid::parse_str(fork).is_ok());
    let kinds = update_kinds(&frames);
    assert!(kinds.contains(&"user_message_chunk".to_string()));
    assert!(kinds.contains(&"agent_message_chunk".to_string()));
    for id in [source.as_str(), fork] {
        let (_, response) = client
            .call(
                "session/prompt",
                json!({"sessionId": id, "prompt": [{"type": "text", "text": "continue"}]}),
            )
            .await;
        assert_eq!(response["result"]["stopReason"], "end_turn");
    }
}

#[tokio::test]
async fn fork_rejects_unknown_sessions_and_wrong_working_directories() {
    let mut client = Client::start();
    let (source, _) = client.new_session().await;
    let (_, response) = client
        .call(
            "session/fork",
            json!({"sessionId": uuid::Uuid::new_v4().to_string(), "cwd": std::env::temp_dir()}),
        )
        .await;
    assert_eq!(response["error"]["code"], RESOURCE_NOT_FOUND);
    let other = tempfile::tempdir().unwrap();
    for cwd in [json!(other.path()), json!("relative"), Value::Null] {
        let (_, response) = client
            .call("session/fork", json!({"sessionId": source, "cwd": cwd}))
            .await;
        assert_eq!(response["error"]["code"], INVALID_PARAMS, "{response}");
    }
}

#[tokio::test]
async fn fork_rejects_an_active_prompt_without_stopping_it() {
    let mut client = Client::start();
    let (source, _) = client.new_session().await;
    let prompt = client
        .request_id(
            "session/prompt",
            json!({"sessionId": source, "prompt": [{"type": "text", "text": "wait-fork"}]}),
        )
        .await;
    loop {
        if client.next().await["params"]["update"]["content"]["text"] == "fork blocked" {
            break;
        }
    }
    let (_, response) = client
        .call(
            "session/fork",
            json!({"sessionId": source, "cwd": std::env::temp_dir()}),
        )
        .await;
    assert_eq!(response["error"]["code"], INVALID_PARAMS);
    client
        .send(
            json!({"jsonrpc": "2.0", "method": "session/cancel", "params": {"sessionId": source}}),
        )
        .await;
    let (_, response) = client.until_response(prompt).await;
    assert_eq!(response["result"]["stopReason"], "cancelled");
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
    let (_, response) = client.until_response(id).await;
    assert_eq!(response["result"]["stopReason"], "cancelled");
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
    ui: Value,
    answer: Value,
) -> (Vec<Value>, Value) {
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
        json!({"purpose": "question", "question": "Choose targets", "allowMultiple": true,
            "allowFreeform": false, "options": ["Linux", "Windows", "macOS"]}),
        json!({"elicitation": {"form": {}}}),
        accepted_question(json!({"answer": ["Linux", "macOS"]})),
    )
    .await;
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0]["requestedSchema"]["properties"]["answer"]["type"],
        "array"
    );
    assert!(requests[0]["requestedSchema"]["properties"]
        .get("answer__other")
        .is_none());
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
async fn nested_input_kind_takes_precedence_over_options() {
    let (requests, native) = question_round_trip(
        json!({"kind": "selector", "questions": [{"question": "Name", "kind": "input", "options": ["suggestion"]}]}),
        json!({"elicitation": {"form": {}}}),
        accepted_question(json!({"answer": "custom"})),
    ).await;
    assert!(requests[0]["requestedSchema"]["properties"]["answer"]
        .get("enum")
        .is_none());
    assert_eq!(native["answer"]["kind"], "input");
    assert_eq!(native["answer"]["value"], "custom");
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
                _ = fake_child(fake_in, fake_out, PathBuf::from(".")) => {},
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
    let cancel = CancellationToken::new();
    let mut request =
        Box::pin(writer.request("session/elicitation", json!({"sessionId": "test"}), &cancel));
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
async fn closed_client_output_fails_dialog_request() {
    let (out, input) = tokio::io::duplex(1024);
    drop(input);
    let writer = Writer::new(Box::new(out));
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        writer.request("session/elicitation", json!({}), &CancellationToken::new()),
    )
    .await
    .unwrap();
    assert_eq!(result, Err("client stdout closed".into()));
    assert!(writer.pending.lock().unwrap().is_empty());
}

#[tokio::test]
async fn output_failure_drains_other_pending_requests() {
    let (out, input) = tokio::io::duplex(1024);
    let writer = Writer::new(Box::new(out));
    let (tx, rx) = tokio::sync::oneshot::channel();
    writer.pending.lock().unwrap().insert(999, tx);
    drop(input);
    assert!(writer
        .request("session/elicitation", json!({}), &CancellationToken::new())
        .await
        .is_err());
    assert!(rx.await.is_err());
    assert!(writer.closed.load(Ordering::Acquire));
    assert!(writer.pending.lock().unwrap().is_empty());
}

#[test]
fn unsupported_question_combinations_are_rejected() {
    for ui in [
        json!({"questions": [{"question":"one"}, {"question":"two"}]}),
        json!({"question":"pick", "allowMultiple":true, "allowFreeform":true, "options":["one"]}),
        json!({"question":"pick", "options":["one", "one"]}),
    ] {
        assert!(translate::question_form(&json!({"requestId":"test", "ui":ui})).is_none());
    }
}

#[tokio::test]
async fn cancelling_form_queued_for_writer_does_not_emit_frame() {
    let (out, input) = tokio::io::duplex(1024);
    let writer = Arc::new(Writer::new(Box::new(out)));
    let guard = writer.out.lock().await;
    let cancel = CancellationToken::new();
    let task = tokio::spawn({
        let writer = Arc::clone(&writer);
        let cancel = cancel.clone();
        async move {
            writer
                .request("session/elicitation", json!({}), &cancel)
                .await
        }
    });
    tokio::task::yield_now().await;
    cancel.cancel();
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap(),
        Err("dialog cancelled".into())
    );
    drop(guard);
    assert!(writer.pending.lock().unwrap().is_empty());
    let mut input = BufReader::new(input).lines();
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(50), input.next_line())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn cancelling_form_wait_finishes_the_in_flight_frame() {
    use tokio::io::AsyncReadExt;

    let (out, mut input) = tokio::io::duplex(8);
    let writer = Arc::new(Writer::new(Box::new(out)));
    let cancel = CancellationToken::new();
    let task = tokio::spawn({
        let writer = Arc::clone(&writer);
        let cancel = cancel.clone();
        async move {
            writer
                .request("session/elicitation", json!({"sessionId": "test"}), &cancel)
                .await
        }
    });
    let deadline = std::time::Duration::from_secs(5);
    let first = tokio::time::timeout(deadline, input.read_u8())
        .await
        .expect("the request starts writing")
        .unwrap();
    cancel.cancel();
    let mut line = vec![first];
    let mut input = BufReader::new(input);
    tokio::time::timeout(deadline, input.read_until(b'\n', &mut line))
        .await
        .expect("cancellation must not truncate the frame")
        .unwrap();
    let frame: Value = serde_json::from_slice(&line).unwrap();
    assert_eq!(frame["method"], "session/elicitation");
    let mut cancellation = Vec::new();
    tokio::time::timeout(deadline, input.read_until(b'\n', &mut cancellation))
        .await
        .unwrap()
        .unwrap();
    let cancellation: Value = serde_json::from_slice(&cancellation).unwrap();
    assert_eq!(cancellation["method"], "$/cancel_request");
    assert_eq!(cancellation["params"]["requestId"], frame["id"]);
    assert_eq!(
        tokio::time::timeout(deadline, task)
            .await
            .expect("cancellation releases the response wait")
            .unwrap(),
        Err("dialog cancelled".into())
    );
    assert!(writer.pending.lock().unwrap().is_empty());
}

#[tokio::test]
async fn cancelling_prompt_with_open_form_allows_the_next_form() {
    let mut client = Client::start();
    client
        .call(
            "initialize",
            json!({"protocolVersion": 1,
        "clientCapabilities": {"elicitation": {"form": {}}}}),
        )
        .await;
    let (session, _) = client.new_session().await;
    let prompt = client.request_id("session/prompt", json!({"sessionId": session,
        "prompt": [{"type": "text", "text": "question {\"question\":\"Continue?\",\"options\":[\"Yes\",\"No\"]}"}]})).await;
    let old_form = loop {
        let frame = client.next().await;
        if frame["method"] == "session/elicitation" {
            break frame["id"].clone();
        }
        assert_ne!(
            frame["id"], prompt,
            "prompt settled before elicitation: {frame}"
        );
    };
    client
        .send(
            json!({"jsonrpc": "2.0", "method": "session/cancel", "params": {"sessionId": session}}),
        )
        .await;
    let (_, response) = client.until_response(prompt).await;
    assert_eq!(response["result"]["stopReason"], "cancelled");
    client.send(json!({"jsonrpc": "2.0", "id": old_form, "result": accepted_question(json!({"answer": "No"}))})).await;
    let (requests, native) = question_in_session(
        &mut client,
        &session,
        json!({"question": "Try again?", "options": ["Yes", "No"]}),
        accepted_question(json!({"answer": "Yes"})),
    )
    .await;
    assert_eq!(requests.len(), 1);
    assert_eq!(native["answer"]["values"], json!(["Yes"]));
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
