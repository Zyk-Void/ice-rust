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
    let resources_enabled = std::env::var_os("ICERS_MCP_TEST_RESOURCES").is_some();
    let mut changed_resources = false;
    let prompts_enabled = std::env::var_os("ICERS_MCP_TEST_PROMPTS").is_some();
    let mut changed_prompts = false;
    let mut prompt_lists = 0;
    let mut resource_lists = 0;
    let mut tool_lists = 0;
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
                let mut result = json!({"protocolVersion":"2025-06-18","capabilities":{"tools":{"listChanged":true}},"serverInfo":{"name":"fake","version":"1"},"instructions":"Call echo first."});
                if prompts_enabled {
                    result["capabilities"]["prompts"] = json!({"listChanged":true});
                }
                if resources_enabled {
                    result["capabilities"]["resources"] = json!({"listChanged":true});
                }
                if std::env::var_os("ICERS_MCP_TEST_RESOURCES_ONLY").is_some() {
                    result["capabilities"]
                        .as_object_mut()
                        .unwrap()
                        .remove("tools");
                }
                result
            }
            "prompts/list" => {
                assert!(
                    initialized && prompts_enabled,
                    "prompts probed without capability"
                );
                prompt_lists += 1;
                match std::env::var("ICERS_MCP_TEST_PROMPT_CURSOR").as_deref() {
                    Ok("repeat") => json!({"prompts":[],"nextCursor":"same"}),
                    Ok("endless") => json!({"prompts":[],"nextCursor":prompt_lists.to_string()}),
                    _ if value["params"]["cursor"] == "second" => {
                        json!({"prompts":[{"name":"Review-Code"}]})
                    }
                    _ => {
                        json!({"prompts":[{"name":if changed_prompts {"New Brief"} else {"Brief"},"description":"Daily\u{001b} brief","arguments":[{"name":"day","required":true},{"name":"topic"}]}],"nextCursor":"second"})
                    }
                }
            }
            "prompts/get" => {
                assert!(prompts_enabled);
                if let Some(path) = std::env::var_os("ICERS_MCP_TEST_PROMPT_GET") {
                    std::fs::write(path, value["params"].to_string()).unwrap();
                }
                json!({"messages":[{"role":"user","content":{"type":"text","text":value["params"]["arguments"].to_string()}},{"role":"assistant","content":{"type":"text","text":"Ready"}}]})
            }
            "resources/list" => {
                assert!(
                    initialized && resources_enabled,
                    "resources must not be probed without capability"
                );
                resource_lists += 1;
                if let Some(path) = std::env::var_os("ICERS_MCP_TEST_RESOURCE_PROBE") {
                    std::fs::write(path, resource_lists.to_string()).unwrap();
                }
                match std::env::var("ICERS_MCP_TEST_RESOURCE_CURSOR").as_deref() {
                    Ok("repeat") => json!({"resources":[],"nextCursor":"same"}),
                    Ok("endless") => {
                        json!({"resources":[],"nextCursor":resource_lists.to_string()})
                    }
                    _ if value["params"]["cursor"] == "second" => {
                        let mut resources = vec![json!({"name":"9 image!","uri":"test:///image"})];
                        if std::env::var_os("ICERS_MCP_TEST_COLLISION").is_some() {
                            resources.push(json!({"name":"Echo","uri":"test:///collision"}));
                        }
                        json!({"resources":resources})
                    }
                    _ => {
                        json!({"resources":[{"name":if changed_resources { "New Notes" } else { "Project Notes" },"description":"Read\u{001b} notes","uri":"test:///notes"}],"nextCursor":"second"})
                    }
                }
            }
            "resources/read" => {
                assert!(resources_enabled);
                assert_eq!(value["params"].as_object().unwrap().len(), 1);
                match value["params"]["uri"].as_str().unwrap() {
                    "test:///image" => {
                        json!({"contents":[{"uri":"test:///image","mimeType":"image/png","blob":"YWJj"}]})
                    }
                    "test:///collision" => {
                        json!({"contents":[{"uri":"test:///collision","text":"resource echo"}]})
                    }
                    "test:///notes" => json!({"contents":[
                        {"uri":"test:///notes","text":"resource line\n".repeat(3000)},
                        {"uri":"test:///blob","mimeType":"application/octet-stream","blob":"aGVsbG8="}
                    ]}),
                    _ => panic!("unexpected resource URI"),
                }
            }
            "tools/list" => {
                assert!(initialized);
                assert!(std::env::var_os("ICERS_MCP_TEST_RESOURCES_ONLY").is_none());
                tool_lists += 1;
                if value["params"]["cursor"] == "second" {
                    let mut tools = vec![json!({"name":"other","inputSchema":{"type":"object"}})];
                    if added_tool {
                        tools.push(json!({"name":"added","inputSchema":{"type":"object"}}));
                    }
                    if std::env::var("ICERS_MCP_TEST_COLLISION").as_deref() == Ok("1") || added_tool
                    {
                        tools.push(json!({"name":"read_echo","inputSchema":{"type":"object"}}));
                    }
                    json!({ "tools": tools })
                } else {
                    json!({"tools":[{"name":"echo","description":"Echo","inputSchema":{"type":"object","properties":{"mode":{"type":"string"}}}}],"nextCursor":"second"})
                }
            }
            "tools/call" => match value["params"]["arguments"]["mode"].as_str() {
                Some("elicit") | Some("elicit_hang") | Some("elicit_cancel") => {
                    send(
                        json!({"jsonrpc":"2.0","id":"elicitation","method":"elicitation/create","params":value["params"]["arguments"]["request"]}),
                    );
                    if value["params"]["arguments"]["mode"] == "elicit_cancel" {
                        send(
                            json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":"elicitation"}}),
                        );
                    }
                    if value["params"]["arguments"]["mode"] != "elicit" {
                        continue;
                    }
                    let response: Value =
                        serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
                    json!({"content":[{"type":"text","text":response.to_string()}]})
                }
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
                Some("change_prompts") => {
                    changed_prompts = !changed_prompts;
                    send(json!({"jsonrpc":"2.0","method":"notifications/prompts/list_changed"}));
                    json!({"content":[{"type":"text","text":"changed"}]})
                }
                Some("catalog_counts") => {
                    json!({"content":[{"type":"text","text":json!({"tools":tool_lists,"resources":resource_lists}).to_string()}]})
                }
                Some("change_resources") => {
                    changed_resources = !changed_resources;
                    send(json!({"jsonrpc":"2.0","method":"notifications/resources/list_changed"}));
                    json!({"content":[{"type":"text","text":"changed"}]})
                }
                Some("disconnect") => return,
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
                // Larger than the default output guard (2,000 lines) and details limit.
                Some("big") => {
                    let text = (0..3000).map(|i| format!("row {i}\n")).collect::<String>();
                    json!({"content":[
                        {"type":"text","text":text},
                        {"type":"image","data":"YWJj","mimeType":"image/png"},
                        {"type":"resource","resource":{"uri":"file:///b.bin","blob":"aGVsbG8="}}
                    ]})
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
