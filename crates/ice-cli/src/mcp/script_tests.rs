use super::*;
#[path = "../../tests/support/mcp_fixture.rs"]
mod mcp_fixture;

async fn session(dir: &Path, server: Value, settings: Value) -> (AgentHarness, McpManager) {
    std::fs::write(
        dir.join("mcp.json"),
        serde_json::to_vec(&json!({"settings":settings,"mcpServers":{"fake":server}})).unwrap(),
    )
    .unwrap();
    let harness = AgentHarness::create(Default::default()).await.unwrap();
    let manager = McpManager::new(
        &harness,
        &Default::default(),
        dir,
        dir.into(),
        false,
        Default::default(),
    );
    manager.reconnect(None).await.unwrap();
    (harness, manager)
}
fn fixture(extra: Value) -> Value {
    let mut server = json!({"command":mcp_fixture::server_executable(),"env":{"ICERS_MCP_TEST_CHILD":"1"},"directTools":false});
    server
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    server
}
async fn script(harness: &AgentHarness, code: &str) -> Result<AgentToolResult, AgentError> {
    let tool = harness
        .get_tools()
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.tool.schema().name == NAME)
        .expect("mcpScript registered");
    tool.tool
        .execute(
            "script",
            json!({"code":code}),
            CancellationToken::new(),
            Arc::new(|_| {}),
        )
        .await
}
fn body(result: &AgentToolResult) -> &str {
    match &result.content[0] {
        TextContentOrImage::Text(text) => &text.text,
        _ => panic!("expected text"),
    }
}

#[tokio::test]
async fn scripts_search_describe_and_chain_calls() {
    let dir = tempfile::tempdir().unwrap();
    let (harness, _manager) = session(dir.path(), fixture(json!({})), json!({})).await;
    let result = script(
        &harness,
        r#"
        const { items, total } = await tools.search({ query: "echo" });
        const details = await tools.describe({ path: items[0].path });
        const first = await tools.call(details.path, { mode: "pid" });
        const missing = await tools.call("fake_nope", {});
        console.log("found", total, details.server);
        emit({ step: "called" });
        return { path: details.path, pid: Number(first.data), ok: first.ok, missing: missing.ok, code: missing.error.code };
        "#,
    )
    .await
    .unwrap();
    let text = body(&result);
    assert!(
        text.starts_with("found 1 fake\n{\"step\":\"called\"}"),
        "{text}"
    );
    let value: Value = serde_json::from_str(&text[text.find("\n\n").unwrap() + 2..]).unwrap();
    assert_eq!(value["path"], "fake_echo");
    assert!(value["pid"].as_f64().unwrap() > 0.0, "{value}");
    assert_eq!(
        (value["ok"].as_bool(), value["missing"].as_bool()),
        (Some(true), Some(false))
    );
    assert_eq!(value["code"], "tool_not_found");
    let calls = result.details["calls"].as_array().unwrap();
    assert_eq!(calls.len(), 4);
    assert_eq!(calls[2]["target"], "fake_echo");
}

#[tokio::test]
async fn runaway_and_failing_scripts_stop_with_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let (harness, _manager) = session(
        dir.path(),
        fixture(json!({"approveTools":["echo"]})),
        json!({}),
    )
    .await;
    let looped = script(&harness, "while (true) {}")
        .await
        .unwrap_err()
        .to_string();
    assert!(
        looped.contains("mcpScript failed") && looped.to_lowercase().contains("loop"),
        "{looped}"
    );
    let deep = script(&harness, "const f = (n) => f(n + 1); f(0);")
        .await
        .unwrap_err()
        .to_string();
    assert!(deep.contains("mcpScript failed"), "{deep}");
    let thrown = script(&harness, "throw new Error('boom')")
        .await
        .unwrap_err()
        .to_string();
    assert!(thrown.contains("boom"), "{thrown}");
    // Approval applies inside scripts; without a dialog the call is refused.
    let refused = script(&harness, "return await tools.call('fake_echo', {});")
        .await
        .unwrap();
    assert!(
        body(&refused).contains("approval denied"),
        "{}",
        body(&refused)
    );
    assert!(script(&harness, &"x".repeat(MAX_CODE + 1)).await.is_err());
}

#[tokio::test]
async fn script_deadline_interrupts_a_pending_tool_call() {
    let dir = tempfile::tempdir().unwrap();
    let (harness, manager) = session(
        dir.path(),
        fixture(json!({"requestTimeoutMs":60000})),
        json!({}),
    )
    .await;
    let result = tokio::time::timeout(
        DEADLINE + Duration::from_secs(2),
        script(
            &harness,
            "return await tools.call('fake_echo', { mode: 'hang' });",
        ),
    )
    .await;
    let error = result
        .expect("script deadline must interrupt dispatch before the tool timeout")
        .unwrap_err()
        .to_string();
    assert!(error.contains("mcpScript timed out"), "{error}");
    assert!(!manager.0.servers.lock().unwrap()["fake"]
        .client
        .as_ref()
        .unwrap()
        .busy());
}

#[tokio::test]
async fn script_mode_false_hides_the_tool() {
    let dir = tempfile::tempdir().unwrap();
    let (harness, _manager) =
        session(dir.path(), fixture(json!({})), json!({"scriptMode":false})).await;
    let names: Vec<String> = harness
        .get_tools()
        .await
        .unwrap()
        .iter()
        .map(|t| t.tool.schema().name.clone())
        .collect();
    assert!(names.contains(&proxy::NAME.to_string()));
    assert!(!names.contains(&NAME.to_string()));
}
