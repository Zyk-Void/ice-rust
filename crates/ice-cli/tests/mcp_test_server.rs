//! Offline MCP transport fixture, inert unless explicitly enabled by tests.
use serde_json::{json, Value};
use std::io::{BufRead, Write};
fn main() {
    if std::env::var_os("ICERS_MCP_TEST_CHILD").is_none() {
        return;
    }
    let mut initialized = false;
    let mut client_capabilities = Value::Null;
    let mut added_tool = false;
    // Client replies to server-initiated requests, reported by the `responses` mode.
    let mut responses = Vec::new();
    let send = |value: Value| {
        println!("{value}");
        std::io::stdout().flush().unwrap();
    };
    let mut lines = std::io::stdin().lock().lines();
    while let Some(line) = lines.next() {
        let value: Value = serde_json::from_str(&line.unwrap()).unwrap();
        let Some(method) = value["method"].as_str() else {
            responses.push(value);
            continue;
        };
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
                client_capabilities = value["params"]["capabilities"].clone();
                json!({"protocolVersion":"2025-06-18","capabilities":{"tools":{"listChanged":true}},"serverInfo":{"name":"fake","version":"1"},"instructions":"Call echo first."})
            }
            "tools/list" => {
                assert!(initialized);
                if value["params"]["cursor"] == "second" {
                    let mut tools = vec![json!({"name":"other","inputSchema":{"type":"object"}})];
                    if added_tool {
                        tools.push(json!({"name":"added","inputSchema":{"type":"object"}}));
                    }
                    json!({ "tools": tools })
                } else {
                    json!({"tools":[{"name":"echo","description":"Echo","inputSchema":{"type":"object","properties":{"mode":{"type":"string"}}}}],"nextCursor":"second"})
                }
            }
            "tools/call" => match value["params"]["arguments"]["mode"].as_str() {
                Some("crash") => std::process::exit(0),
                Some("hang") => continue,
                Some("server_requests") => {
                    for (method, id, reply) in [
                        ("ping", json!("server-ping"), json!({"result":{}})),
                        (
                            "unknown/method",
                            json!(42),
                            json!({"error":{"code":-32601,"message":"Method not found"}}),
                        ),
                    ] {
                        println!("{}", json!({"jsonrpc":"2.0","method":method,"id":id}));
                        std::io::stdout().flush().unwrap();
                        let response: Value =
                            serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
                        let mut expected = reply;
                        expected["jsonrpc"] = json!("2.0");
                        expected["id"] = id;
                        assert_eq!(response, expected);
                    }
                    json!({"content":[{"type":"text","text":"server requests answered"}]})
                }
                // Leaves a request pending in the client, then checks ping still flows.
                Some("delayed_request") | Some("cancelled_request") => {
                    let id = value["params"]["arguments"]["id"].clone();
                    send(json!({"jsonrpc":"2.0","method":"test/slow","id":id}));
                    if value["params"]["arguments"]["mode"] == "cancelled_request" {
                        send(
                            json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":id}}),
                        );
                    }
                    send(json!({"jsonrpc":"2.0","method":"ping","id":"live"}));
                    let response: Value =
                        serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
                    assert_eq!(response, json!({"jsonrpc":"2.0","id":"live","result":{}}));
                    json!({"content":[{"type":"text","text":"ping answered"}]})
                }
                // One request past the client's cap must be refused immediately.
                Some("flood") => {
                    let count = value["params"]["arguments"]["count"].as_u64().unwrap();
                    for id in 0..count {
                        send(json!({"jsonrpc":"2.0","method":"test/slow","id":id}));
                    }
                    let response: Value =
                        serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
                    json!({"content":[{"type":"text","text":response.to_string()}]})
                }
                Some("responses") => {
                    json!({"content":[{"type":"text","text":Value::from(responses.clone()).to_string()}]})
                }
                Some("client_capabilities") => {
                    json!({"content":[{"type":"text","text":client_capabilities.to_string()}]})
                }
                Some("pid") => {
                    json!({"content":[{"type":"text","text":std::process::id().to_string()}]})
                }
                Some("add_tool") => {
                    added_tool = true;
                    send(json!({"jsonrpc":"2.0","method":"notifications/tools/list_changed"}));
                    json!({"content":[{"type":"text","text":"added"}]})
                }
                Some("error") => {
                    json!({"isError":true,"content":[{"type":"text","text":"failure"}]})
                }
                _ => {
                    json!({"content":[{"type":"text","text":"hello"},{"type":"image","data":"YWJj","mimeType":"image/png"},{"type":"resource","resource":{"text":"embedded"}},{"type":"audio","data":"abc"}]})
                }
            },
            _ => panic!("unexpected method: {method}"),
        };
        send(json!({"jsonrpc":"2.0","id":value["id"],"result":result}));
    }
}
