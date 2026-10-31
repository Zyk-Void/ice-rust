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
    restart_roundtrip(false).await;
}

#[tokio::test]
async fn acp_load_legacy_session_recovers_model_from_assistant() {
    restart_roundtrip(true).await;
}

async fn restart_roundtrip(legacy: bool) {
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
            "session/load",
            json!({"sessionId": id, "cwd": temp.path(), "mcpServers": []}),
        )
        .await;
    assert_eq!(
        replay_text(&frames, "user_message_chunk"),
        "remember restart-marker-721"
    );
    assert_eq!(
        replay_text(&frames, "agent_message_chunk"),
        "persisted answer"
    );
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
