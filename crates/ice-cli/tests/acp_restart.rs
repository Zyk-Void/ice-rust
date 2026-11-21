//! Real ACP process restart regression, using only a temporary profile and a
//! loopback mock provider. No real credentials or user sessions are accessed.

use serde_json::{json, Value};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout};

struct Client {
    child: Child,
    input: Option<ChildStdin>,
    output: Lines<BufReader<ChildStdout>>,
    id: u64,
}

impl Client {
    fn start(temp: &tempfile::TempDir) -> Self {
        let program = std::env::var_os("ICERS_ACP_TEST_BINARY")
            .unwrap_or_else(|| env!("CARGO_BIN_EXE_icers").into());
        let mut child = tokio::process::Command::new(program)
            .args([
                "--acp",
                "--offline",
                "--model",
                "restart/default",
                "--no-extensions",
                "--no-skills",
                "--no-prompt-templates",
                "--no-context-files",
            ])
            .current_dir(temp.path())
            .env("ICERS_CODING_AGENT_DIR", temp.path().join("agent"))
            .env("ICERS_OFFLINE", "1")
            .env_remove("ICERS_MODEL")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        Self {
            input: child.stdin.take(),
            output: BufReader::new(child.stdout.take().unwrap()).lines(),
            child,
            id: 0,
        }
    }

    async fn call(&mut self, method: &str, params: Value) -> (Vec<Value>, Value) {
        self.id += 1;
        let mut line = serde_json::to_vec(
            &json!({"jsonrpc": "2.0", "id": self.id, "method": method, "params": params}),
        )
        .unwrap();
        line.push(b'\n');
        self.input.as_mut().unwrap().write_all(&line).await.unwrap();
        let mut updates = Vec::new();
        loop {
            let line = tokio::time::timeout(Duration::from_secs(60), self.output.next_line())
                .await
                .expect("ACP response timeout")
                .unwrap()
                .expect("ACP stream closed");
            let frame: Value = serde_json::from_str(&line).expect("stdout must be protocol-only");
            if frame["id"] == self.id && frame.get("method").is_none() {
                assert!(frame.get("error").is_none(), "{frame}");
                return (updates, frame["result"].clone());
            }
            updates.push(frame);
        }
    }

    async fn stop(mut self) {
        self.input.take();
        let status = tokio::time::timeout(Duration::from_secs(15), self.child.wait())
            .await
            .unwrap()
            .unwrap();
        assert!(status.success(), "{status}");
    }
}

async fn mock_provider(listener: tokio::net::TcpListener) -> Vec<Value> {
    let mut requests = Vec::new();
    for answer in ["persisted answer", "continued answer"] {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = Vec::new();
        let request = loop {
            let mut buf = [0u8; 8192];
            let count = socket.read(&mut buf).await.unwrap();
            assert!(count > 0);
            bytes.extend_from_slice(&buf[..count]);
            if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
                let length: usize = headers
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length:"))
                    .expect("content length")
                    .trim()
                    .parse()
                    .unwrap();
                if bytes.len() >= end + 4 + length {
                    break serde_json::from_slice::<Value>(&bytes[end + 4..end + 4 + length])
                        .unwrap();
                }
            }
        };
        requests.push(request);
        let body = format!("data: {{\"id\":\"mock\",\"choices\":[{{\"index\":0,\"delta\":{{\"content\":\"{answer}\"}},\"finish_reason\":\"stop\"}}]}}\n\ndata: [DONE]\n\n");
        let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        socket.write_all(response.as_bytes()).await.unwrap();
    }
    requests
}

fn replay_text(frames: &[Value], kind: &str) -> String {
    frames
        .iter()
        .filter(|frame| frame["params"]["update"]["sessionUpdate"] == kind)
        .filter_map(|frame| frame["params"]["update"]["content"]["text"].as_str())
        .collect()
}

#[tokio::test]
async fn acp_load_after_restart_preserves_transcript_model_and_provider_context() {
    restart_roundtrip(false, "session/load").await;
}

#[tokio::test]
async fn acp_load_legacy_session_recovers_model_from_assistant() {
    restart_roundtrip(true, "session/load").await;
}

#[tokio::test]
async fn acp_resume_after_restart_preserves_config_and_context_without_replay() {
    restart_roundtrip(false, "session/resume").await;
}

#[tokio::test]
async fn acp_resume_legacy_session_recovers_model_without_replay() {
    restart_roundtrip(true, "session/resume").await;
}

#[tokio::test]
async fn acp_fork_keeps_the_live_source_and_copies_context_without_rewriting_it() {
    let temp = tempfile::tempdir().unwrap();
    let agent = temp.path().join("agent");
    std::fs::create_dir(&agent).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    std::fs::write(agent.join("models.json"), json!({"providers": {"restart": {
        "api": "openai-completions", "baseUrl": format!("http://{address}/v1"), "apiKey": "test-only",
        "models": [{"id": "default", "contextWindow": 32000, "maxTokens": 1024},
                   {"id": "selected", "contextWindow": 32000, "maxTokens": 1024}]
    }}}).to_string()).unwrap();
    std::fs::write(
        agent.join("settings.json"),
        json!({"sessionDir": "saved-sessions"}).to_string(),
    )
    .unwrap();
    let provider = tokio::spawn(mock_provider(listener));
    let mut client = Client::start(&temp);
    let (_, init) = client
        .call(
            "initialize",
            json!({"protocolVersion": 1, "clientCapabilities": {}}),
        )
        .await;
    assert!(init["agentCapabilities"]["sessionCapabilities"]["fork"].is_object());
    let (_, source) = client
        .call("session/new", json!({"cwd": temp.path()}))
        .await;
    let source_id = source["sessionId"].as_str().unwrap();
    client
        .call(
            "session/set_config_option",
            json!({"sessionId": source_id, "configId": "model", "value": "restart/selected"}),
        )
        .await;
    client
        .call(
            "session/set_config_option",
            json!({"sessionId": source_id, "configId": "thinking", "value": "low"}),
        )
        .await;
    client.call("session/prompt", json!({"sessionId": source_id, "prompt": [{"type": "text", "text": "/name Fork source"}]})).await;
    client.call("session/prompt", json!({"sessionId": source_id, "prompt": [{"type": "text", "text": "remember fork-marker-721"}]})).await;

    fn path_for_id(root: &std::path::Path, id: &str) -> Option<std::path::PathBuf> {
        for entry in std::fs::read_dir(root).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                if let Some(found) = path_for_id(&path, id) {
                    return Some(found);
                }
            } else if path
                .extension()
                .is_some_and(|extension| extension == "jsonl")
            {
                let text = std::fs::read_to_string(&path).unwrap();
                let header: Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
                if header["id"] == id {
                    return Some(path);
                }
            }
        }
        None
    }
    let directory = temp.path().join("saved-sessions");
    let source_path = path_for_id(&directory, source_id).unwrap();
    let source_bytes = std::fs::read(&source_path).unwrap();
    let (frames, fork) = client
        .call(
            "session/fork",
            json!({"sessionId": source_id, "cwd": temp.path()}),
        )
        .await;
    let fork_id = fork["sessionId"].as_str().unwrap();
    assert_ne!(fork_id, source_id);
    assert!(uuid::Uuid::parse_str(fork_id).is_ok());
    assert_eq!(std::fs::read(&source_path).unwrap(), source_bytes);
    assert_eq!(
        replay_text(&frames, "user_message_chunk"),
        "remember fork-marker-721"
    );
    assert_eq!(
        replay_text(&frames, "agent_message_chunk"),
        "persisted answer"
    );
    for (id, value) in [("model", "restart/selected"), ("thinking", "low")] {
        let option = fork["configOptions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|option| option["id"] == id)
            .unwrap();
        assert_eq!(option["currentValue"], value);
    }
    let fork_path = path_for_id(&directory, fork_id).unwrap();
    let fork_text = std::fs::read_to_string(&fork_path).unwrap();
    let header: Value = serde_json::from_str(fork_text.lines().next().unwrap()).unwrap();
    assert_eq!(header["parentSessionId"], source_id);
    let (_, result) = client.call("session/prompt", json!({"sessionId": source_id, "prompt": [{"type": "text", "text": "/name Source still alive"}]})).await;
    assert_eq!(result["stopReason"], "end_turn");
    assert_eq!(
        std::fs::read_to_string(&fork_path).unwrap(),
        fork_text,
        "source writes cannot mutate the fork"
    );
    let (_, result) = client.call("session/prompt", json!({"sessionId": fork_id, "prompt": [{"type": "text", "text": "continue on the fork"}]})).await;
    assert_eq!(result["stopReason"], "end_turn");
    client.stop().await;
    let requests = tokio::time::timeout(Duration::from_secs(10), provider)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(requests[1]["model"], "selected");
    let messages = requests[1]["messages"].as_array().unwrap();
    assert!(messages
        .iter()
        .any(|message| message["content"].to_string().contains("fork-marker-721")));
    assert!(messages.iter().any(|message| message["role"] == "assistant"
        && message["content"].to_string().contains("persisted answer")));
}

async fn restart_roundtrip(legacy: bool, method: &str) {
    let temp = tempfile::tempdir().unwrap();
    let agent = temp.path().join("agent");
    std::fs::create_dir(&agent).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    std::fs::write(
        agent.join("models.json"),
        json!({"providers": {"restart": {
            "api": "openai-completions", "baseUrl": format!("http://{address}/v1"),
            "apiKey": "test-only", "models": [
                {"id": "default", "contextWindow": 32000, "maxTokens": 1024},
                {"id": "selected", "contextWindow": 32000, "maxTokens": 1024}
            ]
        }}})
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        agent.join("settings.json"),
        json!({"sessionDir": "saved-sessions"}).to_string(),
    )
    .unwrap();
    let provider = tokio::spawn(mock_provider(listener));
    let mut first = Client::start(&temp);
    let (_, init) = first
        .call(
            "initialize",
            json!({"protocolVersion": 1, "clientCapabilities": {}}),
        )
        .await;
    assert_eq!(init["agentCapabilities"]["loadSession"], true);
    let (_, session) = first
        .call("session/new", json!({"cwd": temp.path(), "mcpServers": []}))
        .await;
    let id = session["sessionId"].as_str().unwrap().to_string();
    let (named, _) = first
        .call(
            "session/prompt",
            json!({"sessionId": id, "prompt": [{"type": "text", "text": "/name Restart title"}]}),
        )
        .await;
    assert!(
        named
            .iter()
            .any(
                |frame| frame["params"]["update"]["sessionUpdate"] == "session_info_update"
                    && frame["params"]["update"]["title"] == "Restart title"
            ),
        "{named:?}"
    );
    first
        .call(
            "session/set_config_option",
            json!({"sessionId": id, "configId": "model", "value": "restart/selected"}),
        )
        .await;
    first
        .call(
            "session/set_config_option",
            json!({"sessionId": id, "configId": "thinking", "value": "low"}),
        )
        .await;
    first.call("session/prompt", json!({"sessionId": id, "prompt": [{"type": "text", "text": "remember restart-marker-721"}]})).await;
    first.stop().await;
    fn find_jsonl(dir: &std::path::Path) -> Option<std::path::PathBuf> {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                if let Some(found) = find_jsonl(&path) {
                    return Some(found);
                }
            } else if path
                .extension()
                .is_some_and(|extension| extension == "jsonl")
            {
                return Some(path);
            }
        }
        None
    }
    let saved_path =
        find_jsonl(&temp.path().join("saved-sessions")).expect("persisted native session");
    if legacy {
        let text = std::fs::read_to_string(&saved_path).unwrap();
        // Reproduce files written before RPC selections were durable, without
        // changing entry IDs or the branch's parent chain.
        let lines: Vec<String> = text
            .lines()
            .map(|line| {
                let mut entry: Value = serde_json::from_str(line).unwrap();
                if entry["type"] == "model_change" {
                    entry["type"] = json!("custom");
                    entry["customType"] = json!("legacy-selection");
                    entry["data"] = json!({});
                }
                entry.to_string()
            })
            .collect();
        std::fs::write(&saved_path, lines.join("\n") + "\n").unwrap();
    }
    let saved_bytes = std::fs::read(&saved_path).unwrap();

    let mut second = Client::start(&temp);
    second
        .call(
            "initialize",
            json!({"protocolVersion": 1, "clientCapabilities": {}}),
        )
        .await;
    let (frames, loaded) = second
        .call(
            method,
            json!({"sessionId": id, "cwd": temp.path(), "mcpServers": []}),
        )
        .await;
    assert!(
        frames
            .iter()
            .any(
                |frame| frame["params"]["update"]["sessionUpdate"] == "session_info_update"
                    && frame["params"]["update"]["title"] == "Restart title"
            ),
        "{frames:?}"
    );
    if method == "session/resume" {
        assert!(loaded.get("sessionId").is_none());
        assert!(replay_text(&frames, "user_message_chunk").is_empty());
        assert!(replay_text(&frames, "agent_message_chunk").is_empty());
        assert!(frames.iter().all(|frame| !matches!(
            frame["params"]["update"]["sessionUpdate"].as_str(),
            Some("agent_thought_chunk" | "tool_call" | "tool_call_update")
        )));
    } else {
        assert_eq!(
            replay_text(&frames, "user_message_chunk"),
            "remember restart-marker-721"
        );
        assert_eq!(
            replay_text(&frames, "agent_message_chunk"),
            "persisted answer"
        );
    }
    let model = loaded["configOptions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|option| option["id"] == "model")
        .unwrap();
    assert_eq!(model["currentValue"], "restart/selected");
    let thinking = loaded["configOptions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|option| option["id"] == "thinking")
        .unwrap();
    assert_eq!(thinking["currentValue"], "low");
    assert_eq!(
        std::fs::read(&saved_path).unwrap(),
        saved_bytes,
        "loading must not rewrite the saved conversation"
    );
    let (_, result) = second.call("session/prompt", json!({"sessionId": id, "prompt": [{"type": "text", "text": "continue after restart"}]})).await;
    assert_eq!(result["stopReason"], "end_turn");
    second.stop().await;
    let requests = tokio::time::timeout(Duration::from_secs(10), provider)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(requests[1]["model"], "selected");
    let messages = requests[1]["messages"].as_array().unwrap();
    assert!(messages.iter().any(|message| message["role"] == "user"
        && message["content"]
            .to_string()
            .contains("restart-marker-721")));
    assert!(messages.iter().any(|message| message["role"] == "assistant"
        && message["content"].to_string().contains("persisted answer")));
    assert!(messages.iter().any(|message| message["role"] == "user"
        && message["content"]
            .to_string()
            .contains("continue after restart")));
}
