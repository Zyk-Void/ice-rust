//! Close owns only this test's child and shell group, in a temporary profile.
#![cfg(unix)]

use nix::sys::signal::{kill, killpg, Signal};
use nix::unistd::{getpgid, Pid};
use serde_json::{json, Value};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

struct OwnedShell(Option<std::path::PathBuf>);

impl Drop for OwnedShell {
    fn drop(&mut self) {
        // The PID was written by the shell launched by our mock tool call.
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

async fn reply(
    socket: &mut tokio::net::TcpStream,
    delta: Value,
    reason: &str,
) -> std::io::Result<()> {
    let mut bytes = Vec::new();
    loop {
        let mut buf = [0; 8192];
        let count = socket.read(&mut buf).await?;
        if count == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        bytes.extend_from_slice(&buf[..count]);
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
                break;
            }
        }
    }
    let chunk = json!({"id":"mock", "choices":[{"index":0,"delta":delta,"finish_reason":reason}]});
    let body = format!("data: {chunk}\n\ndata: [DONE]\n\n");
    let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    socket.write_all(response.as_bytes()).await
}

async fn call(
    input: &mut tokio::process::ChildStdin,
    output: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    id: u64,
    method: &str,
    params: Value,
) -> Value {
    let frame = json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params});
    input
        .write_all(format!("{frame}\n").as_bytes())
        .await
        .unwrap();
    loop {
        let line = tokio::time::timeout(Duration::from_secs(30), output.next_line())
            .await
            .expect("ACP response timeout")
            .unwrap()
            .expect("ACP exited");
        let frame: Value = serde_json::from_str(&line).expect("protocol-only stdout");
        if frame["id"] == id && frame.get("method").is_none() {
            assert!(frame.get("error").is_none(), "{frame}");
            return frame["result"].clone();
        }
    }
}

#[tokio::test]
async fn close_stops_term_resistant_owned_shell_and_releases_saved_session() {
    let temp = tempfile::tempdir().unwrap();
    // Register cleanup before making any request which could launch a shell.
    let mut owned = OwnedShell(Some(temp.path().join("owned-shell.pid")));
    let agent = temp.path().join("agent");
    std::fs::create_dir(&agent).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    std::fs::write(
        agent.join("models.json"),
        json!({"providers":{"close":{
            "api":"openai-completions","baseUrl":format!("http://{address}/v1"),
            "apiKey":"test-only","models":[{"id":"mock","contextWindow":32000,"maxTokens":1024}]
        }}})
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        agent.join("settings.json"),
        json!({"sessionDir":"saved-sessions"}).to_string(),
    )
    .unwrap();
    let (provider_stop, mut provider_stopped) = tokio::sync::oneshot::channel::<()>();
    let provider = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let args = json!({
            "command":"sh -c 'trap \"\" TERM; echo $$ > owned-shell.pid; while :; do sleep 1; done' </dev/null >/dev/null 2>&1 & echo started",
            "run_in_background":true
        });
        reply(
            &mut socket,
            json!({"tool_calls":[{
                "index":0,"id":"owned-shell","type":"function",
                "function":{"name":"bash","arguments":args.to_string()}
            }]}),
            "tool_calls",
        )
        .await
        .expect("initial tool request remains connected");
        // The outer shell can complete while its owned descendant lingers.
        // Its notice may need a late foreground reply or an ACP wake reply;
        // this cleanup test must keep the mock alive through either ordering.
        loop {
            let (mut socket, _) = tokio::select! {
                _ = &mut provider_stopped => break,
                next = listener.accept() => next.unwrap(),
            };
            if let Err(error) =
                reply(&mut socket, json!({"content":"Shell is running."}), "stop").await
            {
                // Close can cancel a late wake before its HTTP request or
                // response completes. The initial tool exchange stays strict.
                assert!(
                    matches!(
                        error.kind(),
                        std::io::ErrorKind::UnexpectedEof
                            | std::io::ErrorKind::ConnectionReset
                            | std::io::ErrorKind::BrokenPipe
                    ),
                    "late mock provider reply failed: {error}"
                );
            }
        }
    });
    let program = std::env::var_os("ICERS_ACP_TEST_BINARY")
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_icers").into());
    let mut child = tokio::process::Command::new(program)
        .args([
            "--acp",
            "--offline",
            "--model",
            "close/mock",
            "--no-extensions",
            "--no-skills",
            "--no-prompt-templates",
            "--no-context-files",
        ])
        .current_dir(temp.path())
        .env("ICERS_CODING_AGENT_DIR", &agent)
        .env("ICERS_OFFLINE", "1")
        .env_remove("ICERS_MODEL")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap()).lines();
    call(&mut input, &mut output, 1, "initialize", json!({})).await;
    let created = call(
        &mut input,
        &mut output,
        2,
        "session/new",
        json!({"cwd":temp.path()}),
    )
    .await;
    let id = created["sessionId"].as_str().unwrap();
    let turn = call(
        &mut input,
        &mut output,
        3,
        "session/prompt",
        json!({"sessionId":id,"prompt":[{"type":"text","text":"Start the test shell."}]}),
    )
    .await;
    assert_eq!(turn["stopReason"], "end_turn");
    let pid = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(text) = std::fs::read_to_string(temp.path().join("owned-shell.pid")) {
                if let Ok(pid) = text.trim().parse::<i32>() {
                    break Pid::from_raw(pid);
                }
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("mock tool started shell");
    assert!(
        kill(pid, None).is_ok(),
        "owned shell is running before close"
    );
    assert_eq!(
        call(
            &mut input,
            &mut output,
            4,
            "session/close",
            json!({"sessionId":id})
        )
        .await,
        json!({})
    );
    assert_eq!(
        kill(pid, None),
        Err(nix::errno::Errno::ESRCH),
        "close must wait for shell cleanup"
    );
    owned.0 = None;
    let loaded = call(
        &mut input,
        &mut output,
        5,
        "session/load",
        json!({"sessionId":id,"cwd":temp.path()}),
    )
    .await;
    assert_eq!(loaded["sessionId"], id);
    call(
        &mut input,
        &mut output,
        6,
        "session/close",
        json!({"sessionId":id}),
    )
    .await;
    drop(input);
    let status = tokio::time::timeout(Duration::from_secs(15), child.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(status.success(), "{status}");
    let _ = provider_stop.send(());
    provider.await.unwrap();
}
