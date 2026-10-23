//! Offline MCP transport fixture, inert unless explicitly enabled by tests.
use serde_json::{json, Value};
use std::io::{BufRead, Write};
fn main() {
    if std::env::var_os("ICERS_MCP_TEST_CHILD").is_none() {
        return;
    }
    let mut initialized = false;
    for line in std::io::stdin().lock().lines() {
        let value: Value = serde_json::from_str(&line.unwrap()).unwrap();
        let method = value["method"].as_str().unwrap();
        if method == "notifications/initialized" {
            initialized = true;
            continue;
        }
        if method == "notifications/cancelled" {
            if let Some(path) = std::env::var_os("ICERS_MCP_TEST_CANCEL") {
                std::fs::write(path, value.to_string()).unwrap();
            }
            continue;
        }
        let result = match method {
            "initialize" => {
                if let Ok(delay) = std::env::var("MCP_TEST_DELAY_MS") {
                    std::thread::sleep(std::time::Duration::from_millis(delay.parse().unwrap()));
                }
                assert_eq!(value["params"]["clientInfo"]["name"], "icers");
                json!({"protocolVersion":"2025-06-18","capabilities":{"tools":{}},"serverInfo":{"name":"fake","version":"1"}})
            }
            "tools/list" => {
                assert!(initialized);
                if value["params"]["cursor"] == "second" {
                    json!({"tools":[{"name":"other","inputSchema":{"type":"object"}}]})
                } else {
                    json!({"tools":[{"name":"echo","description":"Echo","inputSchema":{"type":"object","properties":{"mode":{"type":"string"}}}}],"nextCursor":"second"})
                }
            }
            "tools/call" => match value["params"]["arguments"]["mode"].as_str() {
                Some("crash") => std::process::exit(0),
                Some("hang") => continue,
                Some("error") => {
                    json!({"isError":true,"content":[{"type":"text","text":"failure"}]})
                }
                _ => {
                    json!({"content":[{"type":"text","text":"hello"},{"type":"image","data":"YWJj","mimeType":"image/png"},{"type":"resource","resource":{"text":"embedded"}},{"type":"audio","data":"abc"}]})
                }
            },
            _ => panic!("unexpected method: {method}"),
        };
        println!(
            "{}",
            json!({"jsonrpc":"2.0","id":value["id"],"result":result})
        );
        std::io::stdout().flush().unwrap();
    }
}
