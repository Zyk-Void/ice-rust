use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde_json::{json, Value};

fn read_request(stream: &mut TcpStream) -> (String, HashMap<String, String>, Value) {
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 8192];
    let header_end = loop {
        let count = stream.read(&mut chunk).unwrap();
        assert_ne!(count, 0, "server received an incomplete HTTP request");
        bytes.extend_from_slice(&chunk[..count]);
        if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break end;
        }
    };

    let header_text = String::from_utf8_lossy(&bytes[..header_end]);
    let mut lines = header_text.split("\r\n");
    let request_line = lines.next().unwrap_or_default().to_string();
    let headers: HashMap<String, String> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_string()))
        .collect();
    let content_length = headers
        .get("content-length")
        .expect("JSON request should include Content-Length")
        .parse::<usize>()
        .unwrap();
    let body_start = header_end + 4;
    let body_end = body_start + content_length;
    while bytes.len() < body_end {
        let count = stream.read(&mut chunk).unwrap();
        assert_ne!(count, 0, "server received a truncated HTTP body");
        bytes.extend_from_slice(&chunk[..count]);
    }
    let body = serde_json::from_slice(&bytes[body_start..body_end]).unwrap();
    (request_line, headers, body)
}

fn respond(stream: &mut TcpStream, status: &str, content_type: &str, body: &[u8]) {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .unwrap();
    stream.write_all(body).unwrap();
}

#[test]
fn plain_prompt_uses_codex_input_text_items_through_the_cli() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        let (mut stream, _) = loop {
            match listener.accept() {
                Ok(connection) => break connection,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        Instant::now() < deadline,
                        "icers did not contact the fake server"
                    );
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("fake server accept failed: {error}"),
            }
        };
        let request = read_request(&mut stream);
        let typed_input = request.2["input"]
            .as_array()
            .and_then(|items| items.first())
            .and_then(|item| item.get("content"))
            .and_then(Value::as_array)
            .is_some_and(|content| {
                content == &[json!({"type": "input_text", "text": "hey there"})]
            });
        if typed_input {
            let response = concat!(
                "event: response.created\r\n",
                "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_fixture\",\"status\":\"in_progress\",\"model\":\"gpt-5.6-luna\"}}\r\n\r\n",
                "event: response.output_text.delta\r\n",
                "data: {\"type\":\"response.output_text.delta\",\"output_index\":0,\"content_index\":0,\"delta\":\"Hello from the local fixture.\"}\r\n\r\n",
                "event: response.completed\r\n",
                "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_fixture\",\"status\":\"completed\",\"model\":\"gpt-5.6-luna\",\"output\":[],\"usage\":{\"input_tokens\":4,\"output_tokens\":5,\"total_tokens\":9}}}\r\n\r\n",
            )
            .as_bytes();
            respond(&mut stream, "200 OK", "text/event-stream", response);
        } else {
            respond(
                &mut stream,
                "415 Unsupported Media Type",
                "application/json",
                br#"{"detail":"Unsupported content type"}"#,
            );
        }
        request
    });

    let temp = tempfile::tempdir().unwrap();
    let config_dir = temp.path().join("config");
    let project_dir = temp.path().join("project");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::create_dir_all(&project_dir).unwrap();
    let claim = json!({
        "https://api.openai.com/auth": {"chatgpt_account_id": "acct_fixture"}
    });
    let access = format!(
        "header.{}.signature",
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claim).unwrap())
    );
    let auth = json!({
        "openai-codex": {
            "type": "oauth",
            "access": access,
            "refresh": "synthetic-refresh-token",
            "expires": 4_102_444_800_i64,
            "accountId": "acct_fixture"
        }
    });
    let auth_path = config_dir.join("auth.json");
    std::fs::write(&auth_path, serde_json::to_vec(&auth).unwrap()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&auth_path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }

    let output = Command::new(env!("CARGO_BIN_EXE_icers"))
        .current_dir(&project_dir)
        .env("ICERS_CODING_AGENT_DIR", &config_dir)
        .env("ICERS_OFFLINE", "1")
        .env_remove("OPENAI_API_KEY")
        .args([
            "--provider",
            "openai-codex",
            "--model",
            "openai-codex/gpt-5.6-luna",
            "--base-url",
        ])
        .arg(format!("http://{address}"))
        // Custom endpoints require an explicitly supplied token; stored OAuth
        // credentials must never be forwarded to this local fixture.
        .arg("--api-key")
        .arg(&access)
        .args([
            "--offline",
            "--no-extensions",
            "--no-skills",
            "--no-prompt-templates",
            "--no-context-files",
            "--no-themes",
            "--print",
            "hey there",
        ])
        .output()
        .unwrap();
    let (request_line, headers, body) = server.join().unwrap();

    assert_eq!(request_line, "POST /codex/responses HTTP/1.1");
    assert_eq!(
        headers.get("content-type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(
        headers.get("accept").map(String::as_str),
        Some("text/event-stream")
    );
    assert!(headers.contains_key("authorization"));
    assert!(headers.contains_key("chatgpt-account-id"));
    assert_eq!(body["model"], "gpt-5.6-luna");
    assert_eq!(
        body["input"],
        json!([{
            "role": "user",
            "content": [{"type": "input_text", "text": "hey there"}]
        }])
    );
    assert_eq!(body["tools"].as_array().map(Vec::len), Some(5));
    assert!(
        output.status.success(),
        "icers failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("Hello from the local fixture."));
}
