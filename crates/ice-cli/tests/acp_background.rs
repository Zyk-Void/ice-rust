//! Scripted localhost provider and temporary profile; no live model or user state.
#![cfg(unix)]

use nix::sys::signal::{killpg, Signal};
use nix::sys::stat::Mode;
use nix::unistd::{getpgid, mkfifo, Pid};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

struct OwnedShell(Option<PathBuf>);

impl Drop for OwnedShell {
    fn drop(&mut self) {
        // Only this mock tool writes the captured owned process ID.
        if let Some(pid) = self
            .0
            .as_ref()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| text.trim().parse::<i32>().ok())
        {
            if let Ok(group) = getpgid(Some(Pid::from_raw(pid))) {
                let _ = killpg(group, Signal::SIGKILL);
            }
        }
    }
}

async fn request(socket: &mut tokio::net::TcpStream) -> Value {
    let mut bytes = Vec::new();
    loop {
        let mut chunk = [0; 8192];
        let count = socket.read(&mut chunk).await.unwrap();
        assert!(count > 0, "provider request ended before its body");
        bytes.extend_from_slice(&chunk[..count]);
        if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
            let length: usize = headers
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .unwrap()
                .trim()
                .parse()
                .unwrap();
            if bytes.len() >= end + 4 + length {
                return serde_json::from_slice(&bytes[end + 4..end + 4 + length]).unwrap();
            }
        }
    }
}

fn event(delta: Value, reason: Option<&str>) -> String {
    let chunk = json!({"id":"mock", "choices":[{"index":0,"delta":delta,"finish_reason":reason}]});
    format!("data: {chunk}\n\n")
}

async fn reply(socket: &mut tokio::net::TcpStream, delta: Value, reason: &str) {
    let body = format!("{}data: [DONE]\n\n", event(delta, Some(reason)));
    let head = format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
    socket.write_all(head.as_bytes()).await.unwrap();
    socket.write_all(body.as_bytes()).await.unwrap();
}

async fn launch_shell(listener: &tokio::net::TcpListener, count: &AtomicUsize) {
    let (mut socket, _) = listener.accept().await.unwrap();
    request(&mut socket).await;
    count.fetch_add(1, Ordering::SeqCst);
    let args = json!({
        "command":"printf '%s' \"$$\" > owned-shell.pid; read -r release < release-fifo; printf 'shell finished\\n'",
        "run_in_background":true
    });
    reply(
        &mut socket,
        json!({"tool_calls":[{
            "index":0,"id":"background-call","type":"function",
            "function":{"name":"bash","arguments":args.to_string()}
        }]}),
        "tool_calls",
    )
    .await;
    let (mut socket, _) = listener.accept().await.unwrap();
    request(&mut socket).await;
    count.fetch_add(1, Ordering::SeqCst);
    reply(&mut socket, json!({"content":"Started the shell."}), "stop").await;
}

struct Rpc {
    child: tokio::process::Child,
    input: tokio::process::ChildStdin,
    output: tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    frames: Vec<Value>,
}

impl Rpc {
    async fn spawn(temp: &std::path::Path, address: std::net::SocketAddr) -> Self {
        Self::spawn_mode(temp, address, false).await
    }

    async fn spawn_mode(temp: &std::path::Path, address: std::net::SocketAddr, acp: bool) -> Self {
        let profile = temp.join("agent");
        std::fs::create_dir(&profile).unwrap();
        std::fs::write(
            profile.join("models.json"),
            json!({"providers":{"background":{
                "api":"openai-completions","baseUrl":format!("http://{address}/v1"),
                "apiKey":"test-only","models":[{"id":"mock","contextWindow":32000,"maxTokens":1024}]
            }}})
            .to_string(),
        )
        .unwrap();
        std::fs::write(
            profile.join("settings.json"),
            json!({"sessionDir":"sessions"}).to_string(),
        )
        .unwrap();
        mkfifo(&temp.join("release-fifo"), Mode::S_IRUSR | Mode::S_IWUSR).unwrap();
        let program = std::env::var_os("ICERS_ACP_TEST_BINARY")
            .unwrap_or_else(|| env!("CARGO_BIN_EXE_icers").into());
        let mut command = tokio::process::Command::new(program);
        command.env_remove("ICERS_ACP_RPC");
        if acp {
            command.arg("--acp");
        } else {
            command.args(["--mode", "rpc"]).env("ICERS_ACP_RPC", "1");
        }
        let mut child = command
            .args([
                "--offline",
                "--model",
                "background/mock",
                "--no-extensions",
                "--no-skills",
                "--no-prompt-templates",
                "--no-context-files",
            ])
            .current_dir(temp)
            .env("ICERS_CODING_AGENT_DIR", &profile)
            .env("ICERS_OFFLINE", "1")
            .env_remove("ICERS_MODEL")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let output = BufReader::new(child.stdout.take().unwrap()).lines();
        let mut rpc = Self {
            child,
            input,
            output,
            frames: Vec::new(),
        };
        if !acp {
            rpc.until(|frame| frame["type"] == "ready").await;
        }
        rpc
    }

    async fn send(&mut self, frame: Value) {
        self.input
            .write_all(format!("{frame}\n").as_bytes())
            .await
            .unwrap();
        self.input.flush().await.unwrap();
    }

    async fn until(&mut self, predicate: impl Fn(&Value) -> bool) -> Value {
        tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                let line = self
                    .output
                    .next_line()
                    .await
                    .unwrap()
                    .expect("native RPC exited");
                let frame: Value = serde_json::from_str(&line).expect("protocol-only stdout");
                self.frames.push(frame.clone());
                if predicate(&frame) {
                    return frame;
                }
            }
        })
        .await
        .unwrap_or_else(|_| {
            panic!(
                "native RPC frame timeout; last frames: {:?}",
                &self.frames[self.frames.len().saturating_sub(8)..]
            )
        })
    }

    async fn response(&mut self, id: u64) -> Value {
        let response = self
            .until(|frame| frame["type"] == "response" && frame["id"] == id)
            .await;
        assert_eq!(response["success"], true, "{response}");
        response["data"].clone()
    }

    async fn prompt(&mut self, id: u64, text: &str) {
        self.send(json!({"type":"prompt","id":id,"message":text}))
            .await;
        assert_eq!(self.response(id).await["accepted"], true);
    }

    async fn settled(&mut self) -> Value {
        let frame = self
            .until(|frame| frame["type"] == "agent_settled" && frame["origin"] != "wake")
            .await;
        self.ack_client(&frame).await;
        frame
    }

    async fn ack_client(&mut self, frame: &Value) {
        assert!(frame["settlementId"].is_string(), "{frame}");
        self.send(json!({"type":"client_settled_ack","settlementId":frame["settlementId"]}))
            .await;
    }

    async fn acp_call(&mut self, id: u64, method: &str, params: Value) -> Value {
        self.send(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))
            .await;
        let frame = self
            .until(|frame| frame["id"] == id && frame.get("method").is_none())
            .await;
        assert!(frame.get("error").is_none(), "{frame}");
        frame["result"].clone()
    }

    fn trace_acp(&self, label: &str) {
        if std::env::var_os("ICERS_ACP_TEST_TRACE").is_none() {
            return;
        }
        for frame in &self.frames {
            let update = &frame["params"]["update"];
            if frame["result"].get("stopReason").is_some()
                || matches!(
                    update["sessionUpdate"].as_str(),
                    Some("tool_call" | "tool_call_update" | "agent_message_chunk" | "usage_update")
                )
            {
                println!("ACP_TRACE {label} {frame}");
            }
        }
    }

    async fn finish(mut self) {
        drop(self.input);
        let status = tokio::time::timeout(Duration::from_secs(15), self.child.wait())
            .await
            .unwrap()
            .unwrap();
        assert!(status.success(), "{status}");
    }

    async fn stop(mut self) {
        self.send(json!({"type":"stop","id":99})).await;
        self.response(99).await;
        drop(self.input);
        let status = tokio::time::timeout(Duration::from_secs(15), self.child.wait())
            .await
            .unwrap()
            .unwrap();
        assert!(status.success(), "{status}");
    }
}

async fn acp_bridge_background_case(stop: bool) {
    let temp = tempfile::tempdir().unwrap();
    let mut owned = OwnedShell(Some(temp.path().join("owned-shell.pid")));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let provider_calls = calls.clone();
    let provider = tokio::spawn(async move {
        launch_shell(&listener, &provider_calls).await;
        let (mut socket, _) = listener.accept().await.unwrap();
        let client = request(&mut socket).await;
        provider_calls.fetch_add(1, Ordering::SeqCst);
        if stop {
            assert!(client.to_string().contains("after Stop"), "{client}");
            assert!(
                client.to_string().contains("was stopped by the user"),
                "{client}"
            );
            reply(
                &mut socket,
                json!({"content":"Stopped shell acknowledged."}),
                "stop",
            )
            .await;
        } else {
            assert!(
                client.to_string().contains("while shell is pending"),
                "{client}"
            );
            reply(
                &mut socket,
                json!({"content":"Other prompt answered."}),
                "stop",
            )
            .await;
            let (mut socket, _) = listener.accept().await.unwrap();
            let wake = request(&mut socket).await;
            provider_calls.fetch_add(1, Ordering::SeqCst);
            assert!(wake.to_string().contains("shell finished"), "{wake}");
            reply(
                &mut socket,
                json!({"content":"Background finished."}),
                "stop",
            )
            .await;
        }
    });
    let mut rpc = Rpc::spawn_mode(temp.path(), address, true).await;
    let initialized = rpc
        .acp_call(
            1,
            "initialize",
            json!({"protocolVersion":1,"clientCapabilities":{}}),
        )
        .await;
    assert_eq!(initialized["agentInfo"]["name"], "icers");
    let session = rpc
        .acp_call(2, "session/new", json!({"cwd":temp.path()}))
        .await["sessionId"]
        .as_str()
        .unwrap()
        .to_string();
    let first = rpc
        .acp_call(
            3,
            "session/prompt",
            json!({"sessionId":session,"prompt":[{"type":"text","text":"Launch the shell."}]}),
        )
        .await;
    assert_eq!(first["stopReason"], "end_turn");
    let start = rpc
        .frames
        .iter()
        .map(|frame| &frame["params"]["update"])
        .find(|update| update["rawOutput"]["type"] == "BackgroundTaskStarted")
        .expect("native bash result is an ACP background task");
    assert_eq!(start["status"], "in_progress");
    let tool_call = start["toolCallId"].clone();
    let task_id = start["rawOutput"]["task_id"].clone();
    assert_eq!(start["rawOutput"]["icers"]["backgroundTask"], true);
    assert_eq!(calls.load(Ordering::SeqCst), 2);

    if stop {
        let cancel =
            json!({"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":session}});
        if std::env::var_os("ICERS_ACP_TEST_TRACE").is_some() {
            println!("ACP_TRACE idle_stop sent {cancel}");
        }
        rpc.send(cancel).await;
    } else {
        let next = rpc.acp_call(4, "session/prompt", json!({"sessionId":session,"prompt":[{"type":"text","text":"while shell is pending"}]})).await;
        assert_eq!(next["stopReason"], "end_turn");
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        tokio::time::timeout(
            Duration::from_secs(5),
            tokio::fs::write(temp.path().join("release-fifo"), "finish\n"),
        )
        .await
        .unwrap()
        .unwrap();
    }
    let before_terminal = rpc.frames.len();
    let terminal = rpc
        .until(|frame| {
            let update = &frame["params"]["update"];
            update["toolCallId"] == tool_call && update["rawOutput"]["type"] == "TaskOutput"
        })
        .await;
    let update = &terminal["params"]["update"];
    assert_eq!(update["rawOutput"]["Result"]["task_id"], task_id);
    assert_eq!(update["rawOutput"]["icers"]["backgroundTask"], true);
    assert_eq!(update["status"], if stop { "failed" } else { "completed" });
    if stop {
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "idle Stop must not wake the model"
        );
        let next = rpc
            .acp_call(
                4,
                "session/prompt",
                json!({"sessionId":session,"prompt":[{"type":"text","text":"after Stop"}]}),
            )
            .await;
        assert_eq!(next["stopReason"], "end_turn");
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    } else {
        let usage = rpc
            .until(|frame| frame["params"]["update"]["sessionUpdate"] == "usage_update")
            .await;
        assert!(usage["params"]["update"]["size"].as_u64().unwrap() > 0);
        let wake_text = rpc.frames[before_terminal..]
            .iter()
            .filter(|frame| frame["params"]["update"]["sessionUpdate"] == "agent_message_chunk")
            .filter_map(|frame| frame["params"]["update"]["content"]["text"].as_str())
            .collect::<String>();
        assert_eq!(wake_text, "Background finished.");
        assert_eq!(calls.load(Ordering::SeqCst), 4);
    }
    rpc.trace_acp(if stop { "idle_stop" } else { "background_wake" });
    rpc.finish().await;
    owned.0 = None;
    provider.await.unwrap();
}

#[tokio::test]
async fn acp_bridge_reports_background_completion_and_wake_usage_after_foreground_reply() {
    acp_bridge_background_case(false).await;
}

#[tokio::test]
async fn acp_bridge_idle_cancel_updates_original_tool_and_keeps_stop_notice() {
    acp_bridge_background_case(true).await;
}

#[tokio::test]
async fn prompt_during_wake_waits_until_usage_ack_without_settling_early() {
    let temp = tempfile::tempdir().unwrap();
    let mut owned = OwnedShell(Some(temp.path().join("owned-shell.pid")));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let provider_calls = calls.clone();
    let (finish_wake, finish_rx) = tokio::sync::oneshot::channel::<()>();
    let provider = tokio::spawn(async move {
        launch_shell(&listener, &provider_calls).await;
        let (mut socket, _) = listener.accept().await.unwrap();
        let wake_request = request(&mut socket).await;
        provider_calls.fetch_add(1, Ordering::SeqCst);
        assert!(wake_request.to_string().contains("shell finished"));
        socket
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n",
            )
            .await
            .unwrap();
        socket
            .write_all(event(json!({"content":"Wake began."}), None).as_bytes())
            .await
            .unwrap();
        finish_rx.await.unwrap();
        socket
            .write_all(
                format!(
                    "{}data: [DONE]\n\n",
                    event(json!({"content":" Wake finished."}), Some("stop"))
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        drop(socket);
        let (mut socket, _) = listener.accept().await.unwrap();
        let queued = request(&mut socket).await;
        provider_calls.fetch_add(1, Ordering::SeqCst);
        assert!(queued.to_string().contains("queued client prompt"));
        reply(
            &mut socket,
            json!({"content":"Queued client reply."}),
            "stop",
        )
        .await;
    });

    let mut rpc = Rpc::spawn(temp.path(), address).await;
    rpc.prompt(1, "Launch the shell.").await;
    assert_eq!(rpc.settled().await["outcome"], "completed");
    tokio::time::timeout(
        Duration::from_secs(5),
        tokio::fs::write(temp.path().join("release-fifo"), "finish\n"),
    )
    .await
    .unwrap()
    .unwrap();
    rpc.until(|frame| frame["type"] == "message_update" && frame["eventType"] == "text_start")
        .await;
    let before_client = rpc.frames.len();
    rpc.prompt(2, "queued client prompt").await;
    rpc.send(json!({"type":"get_state","id":3})).await;
    rpc.response(3).await;
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    finish_wake.send(()).unwrap();
    let wake = rpc
        .until(|frame| frame["type"] == "agent_settled" && frame["origin"] == "wake")
        .await;
    assert_eq!(wake["outcome"], "completed");
    rpc.send(json!({"type":"get_state","id":4})).await;
    rpc.response(4).await;
    assert_eq!(
        calls.load(Ordering::SeqCst),
        3,
        "the queued client waits for wake usage acknowledgement"
    );
    assert!(!rpc.frames[before_client..]
        .iter()
        .any(|frame| frame["type"] == "agent_settled" && frame["origin"] != "wake"));
    rpc.send(json!({"type":"wake_settled_ack","wakeId":wake["wakeId"],"id":5}))
        .await;
    rpc.response(5).await;
    assert_eq!(rpc.settled().await["outcome"], "completed");
    assert_eq!(calls.load(Ordering::SeqCst), 4);
    assert!(rpc
        .frames
        .iter()
        .any(|frame| frame["type"] == "background_shell" && frame["status"] == "completed"));
    rpc.stop().await;
    owned.0 = None;
    provider.await.unwrap();
}

#[tokio::test]
async fn cancelling_a_client_queued_behind_wake_does_not_submit_it_to_provider() {
    let temp = tempfile::tempdir().unwrap();
    let mut owned = OwnedShell(Some(temp.path().join("owned-shell.pid")));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let provider_calls = calls.clone();
    let (close_wake, close_rx) = tokio::sync::oneshot::channel::<()>();
    let provider = tokio::spawn(async move {
        launch_shell(&listener, &provider_calls).await;
        let (mut socket, _) = listener.accept().await.unwrap();
        request(&mut socket).await;
        provider_calls.fetch_add(1, Ordering::SeqCst);
        socket
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n",
            )
            .await
            .unwrap();
        socket
            .write_all(event(json!({"content":"Wake began."}), None).as_bytes())
            .await
            .unwrap();
        close_rx.await.unwrap();
        drop(socket);
        let (mut socket, _) = listener.accept().await.unwrap();
        let next = request(&mut socket).await;
        provider_calls.fetch_add(1, Ordering::SeqCst);
        assert!(next.to_string().contains("after cancellation"), "{next}");
        assert!(
            !next.to_string().contains("must-never-enter-provider"),
            "{next}"
        );
        reply(
            &mut socket,
            json!({"content":"The next prompt works."}),
            "stop",
        )
        .await;
    });

    let mut rpc = Rpc::spawn(temp.path(), address).await;
    rpc.prompt(1, "Launch the shell.").await;
    assert_eq!(rpc.settled().await["outcome"], "completed");
    tokio::time::timeout(
        Duration::from_secs(5),
        tokio::fs::write(temp.path().join("release-fifo"), "finish\n"),
    )
    .await
    .unwrap()
    .unwrap();
    rpc.until(|frame| frame["type"] == "message_update" && frame["eventType"] == "text_start")
        .await;
    rpc.prompt(2, "must-never-enter-provider").await;
    let before_cancel = rpc.frames.len();
    rpc.send(json!({"type":"kill_background_shells","id":3}))
        .await;
    assert_eq!(rpc.response(3).await["stopped"], true);
    let client = if let Some(frame) = rpc.frames[before_cancel..]
        .iter()
        .find(|frame| frame["type"] == "agent_settled" && frame["origin"] != "wake")
    {
        let frame = frame.clone();
        rpc.ack_client(&frame).await;
        frame
    } else {
        rpc.settled().await
    };
    assert_eq!(client["outcome"], "aborted", "{client}");
    let wake = if let Some(frame) = rpc.frames[before_cancel..]
        .iter()
        .find(|frame| frame["type"] == "agent_settled" && frame["origin"] == "wake")
    {
        frame.clone()
    } else {
        rpc.until(|frame| frame["type"] == "agent_settled" && frame["origin"] == "wake")
            .await
    };
    assert_eq!(wake["outcome"], "aborted", "{wake}");
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    rpc.send(json!({"type":"wake_settled_ack","wakeId":wake["wakeId"],"id":4}))
        .await;
    rpc.response(4).await;
    close_wake.send(()).unwrap();
    rpc.prompt(5, "after cancellation").await;
    assert_eq!(rpc.settled().await["outcome"], "completed");
    assert_eq!(calls.load(Ordering::SeqCst), 4);
    rpc.stop().await;
    owned.0 = None;
    provider.await.unwrap();
}

#[tokio::test]
async fn idle_stop_kills_owned_shell_and_preserves_notice_for_next_turn() {
    let temp = tempfile::tempdir().unwrap();
    let mut owned = OwnedShell(Some(temp.path().join("owned-shell.pid")));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let provider_calls = calls.clone();
    let provider = tokio::spawn(async move {
        launch_shell(&listener, &provider_calls).await;
        let (mut socket, _) = listener.accept().await.unwrap();
        let next = request(&mut socket).await;
        provider_calls.fetch_add(1, Ordering::SeqCst);
        assert!(
            next.to_string().contains("was stopped by the user"),
            "{next}"
        );
        assert!(next.to_string().contains("next explicit prompt"), "{next}");
        reply(
            &mut socket,
            json!({"content":"Acknowledged the stop."}),
            "stop",
        )
        .await;
    });
    let mut rpc = Rpc::spawn(temp.path(), address).await;
    rpc.prompt(1, "Launch the shell.").await;
    assert_eq!(rpc.settled().await["outcome"], "completed");
    let before_stop = rpc.frames.len();
    rpc.send(json!({"type":"kill_background_shells","id":2}))
        .await;
    assert_eq!(rpc.response(2).await["stopped"], true);
    let terminal = if let Some(frame) = rpc.frames[before_stop..]
        .iter()
        .find(|frame| frame["type"] == "background_shell")
    {
        frame.clone()
    } else {
        rpc.until(|frame| frame["type"] == "background_shell").await
    };
    assert_eq!(terminal["stoppedByUser"], true);
    assert_eq!(terminal["status"], "failed");
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    rpc.prompt(3, "next explicit prompt").await;
    assert_eq!(rpc.settled().await["outcome"], "completed");
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    assert!(!rpc.frames[before_stop..]
        .iter()
        .any(|frame| frame["type"] == "agent_settled" && frame["origin"] == "wake"));
    rpc.stop().await;
    owned.0 = None;
    provider.await.unwrap();
}
