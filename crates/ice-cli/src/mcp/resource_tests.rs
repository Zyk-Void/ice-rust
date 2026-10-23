use super::*;
#[path = "../../tests/support/mcp_fixture.rs"]
mod mcp_fixture;

fn fixture(resources: bool) -> ServerConfig {
    let mut config: ServerConfig = serde_json::from_value(json!({
        "command":mcp_fixture::server_executable(),
        "env":{"ICERS_MCP_TEST_CHILD":"1"},
        "approveTools":["read_project_notes"]
    }))
    .unwrap();
    if resources {
        config
            .env
            .insert("ICERS_MCP_TEST_RESOURCES".into(), "1".into());
    }
    config
}

async fn execute(tool: &dyn AgentTool, params: Value) -> AgentToolResult {
    tool.execute(
        "resource-test",
        params,
        CancellationToken::new(),
        Arc::new(|_| {}),
    )
    .await
    .unwrap()
}

fn text(result: &AgentToolResult) -> &str {
    match &result.content[0] {
        TextContentOrImage::Text(text) => &text.text,
        _ => panic!("expected text"),
    }
}

#[test]
fn resource_names_match_pma() {
    for (name, expected) in [
        ("Project Notes", "project_notes"),
        ("__a--B!!!", "a_b"),
        ("9 image!", "resource_9_image"),
        ("💧", "resource"),
        ("", "resource"),
    ] {
        assert_eq!(resource_tool_name(name), expected);
    }
}

#[tokio::test]
async fn resources_discover_two_pages_and_use_the_output_guard() {
    let dir = tempfile::tempdir().unwrap();
    let config = fixture(true);
    let client = Client::spawn("fake", &config, dir.path(), Handlers::default()).unwrap();
    let guard = OutputGuard {
        limits: output_guard::resolve(None),
        ..Default::default()
    };
    let tools = client.discover("fake", &config, &guard).await.unwrap();
    assert_eq!(
        tools
            .iter()
            .map(|t| t.schema.name.as_str())
            .collect::<Vec<_>>(),
        [
            "fake_echo",
            "fake_other",
            "fake_read_project_notes",
            "fake_read_resource_9_image"
        ]
    );
    let notes = &tools[2];
    assert!(notes.approval);
    assert!(!tools[3].approval);
    assert!(!notes.schema.description.contains('\u{001b}'));
    let schema = serde_json::to_value(&notes.schema.parameters).unwrap();
    assert_eq!(schema["properties"], json!({}));
    assert_eq!(schema["additionalProperties"], false);
    let result = execute(notes, json!({})).await;
    let preview = text(&result);
    assert!(preview.contains("MCP text output truncated"), "{preview}");
    assert!(preview.len() < 50 * 1024);
    let details = &result.details;
    let raw_path = details["fullResultPath"].as_str().unwrap();
    let raw: Value = serde_json::from_str(&std::fs::read_to_string(raw_path).unwrap()).unwrap();
    assert!(raw.get("contents").is_none());
    let blob_text = raw["content"][1]["resource"]["text"].as_str().unwrap();
    let path = blob_text
        .split("Binary content saved to ")
        .nth(1)
        .unwrap()
        .lines()
        .next()
        .unwrap();
    assert_eq!(std::fs::read(path).unwrap(), b"hello");
    assert!(!raw.to_string().contains("aGVsbG8="));
    let image = execute(&tools[3], json!({})).await;
    assert!(
        matches!(&image.content[0], TextContentOrImage::Image(i) if i.mime_type == "image/png" && i.data == "YWJj")
    );
    drop(tools);
    drop(guard);
    assert!(!Path::new(path).exists());
}

#[tokio::test]
async fn resources_are_never_probed_without_capability_or_when_hidden() {
    for (capability, exposed) in [(false, true), (true, false)] {
        let dir = tempfile::tempdir().unwrap();
        let mut config = fixture(capability);
        config.expose_resources = exposed;
        let probe = dir.path().join("resource-probe");
        config.env.insert(
            "ICERS_MCP_TEST_RESOURCE_PROBE".into(),
            probe.display().to_string(),
        );
        let client = Client::spawn("fake", &config, dir.path(), Handlers::default()).unwrap();
        let tools = client
            .discover("fake", &config, &OutputGuard::default())
            .await
            .unwrap();
        assert_eq!(tools.len(), 2);
        assert!(!probe.exists());
    }
}

#[tokio::test]
async fn resource_pagination_is_bounded() {
    for (mode, pages, error) in [
        ("repeat", 2, "repeated resource pagination cursor"),
        ("endless", 100, "exceeded 100 pages"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let mut config = fixture(true);
        let probe = dir.path().join("resource-probe");
        config.env.insert(
            "ICERS_MCP_TEST_RESOURCE_PROBE".into(),
            probe.display().to_string(),
        );
        config
            .env
            .insert("ICERS_MCP_TEST_RESOURCE_CURSOR".into(), mode.into());
        let client = Client::spawn("fake", &config, dir.path(), Handlers::default()).unwrap();
        let result = client
            .discover("fake", &config, &OutputGuard::default())
            .await;
        assert!(result.err().unwrap().contains(error));
        assert_eq!(std::fs::read_to_string(probe).unwrap(), pages.to_string());
    }
}

async fn manager(
    dir: &Path,
    config: &ServerConfig,
    args: crate::args::Args,
) -> (AgentHarness, McpManager, Arc<Mutex<HashSet<String>>>) {
    use ice_ai::providers::faux::{FauxProvider, FauxScript};
    use ice_harness::types::AgentHarnessOptions;
    std::fs::write(dir.join("mcp.json"), serde_json::to_vec(&json!({"mcpServers":{"fake":{
        "command":config.command,"env":config.env,"exposeResources":config.expose_resources,"approveTools":["read_project_notes"]
    }}})).unwrap()).unwrap();
    let provider = FauxProvider::new(FauxScript::new());
    let harness = AgentHarness::create(AgentHarnessOptions {
        model: provider.default_model().clone(),
        models: vec![provider],
        ..Default::default()
    })
    .await
    .unwrap();
    let approvals = Arc::new(Mutex::new(HashSet::new()));
    let manager = McpManager::new(&harness, &args, dir, dir.into(), false, approvals.clone());
    manager.reconnect(None).await.unwrap();
    (harness, manager, approvals)
}

async fn call(manager: &McpManager, name: &str, mode: &str) -> AgentToolResult {
    let tool = manager
        .registered_tools()
        .into_iter()
        .find(|t| t.tool.schema().name == name)
        .unwrap();
    execute(tool.tool.as_ref(), json!({"mode":mode})).await
}

async fn wait_tool(manager: &McpManager, name: &str) {
    for _ in 0..200 {
        if manager
            .registered_tools()
            .iter()
            .any(|t| t.tool.schema().name == name)
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("tool {name} was not registered");
}

#[tokio::test]
async fn resources_share_filters_and_approval_registration() {
    let dir = tempfile::tempdir().unwrap();
    let args = crate::args::Args {
        tools: Some(vec!["fake_read_project_notes".into()]),
        ..Default::default()
    };
    let (_harness, manager, approvals) = manager(dir.path(), &fixture(true), args).await;
    assert_eq!(
        manager
            .registered_tools()
            .iter()
            .map(|t| t.tool.schema().name.clone())
            .collect::<Vec<_>>(),
        ["fake_read_project_notes"]
    );
    assert_eq!(
        *approvals.lock().unwrap(),
        HashSet::from(["fake_read_project_notes".into()])
    );
}

#[tokio::test]
async fn resource_refresh_adds_removes_and_preserves_real_tool_priority() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = fixture(true);
    config
        .env
        .insert("ICERS_MCP_TEST_COLLISION".into(), "1".into());
    let (harness, manager, approvals) = manager(dir.path(), &config, Default::default()).await;
    let pid = call(&manager, "fake_echo", "pid").await;
    assert_eq!(
        text(&call(&manager, "fake_read_echo", "pid").await),
        text(&pid)
    );
    let counts = call(&manager, "fake_echo", "catalog_counts").await;
    assert_eq!(
        serde_json::from_str::<Value>(text(&counts)).unwrap(),
        json!({"tools":2,"resources":2})
    );
    call(&manager, "fake_echo", "change_resources").await;
    wait_tool(&manager, "fake_read_new_notes").await;
    assert!(!manager
        .registered_tools()
        .iter()
        .any(|t| t.tool.schema().name == "fake_read_project_notes"));
    assert!(!approvals
        .lock()
        .unwrap()
        .contains("fake_read_project_notes"));
    let counts = call(&manager, "fake_echo", "catalog_counts").await;
    assert_eq!(
        serde_json::from_str::<Value>(text(&counts)).unwrap(),
        json!({"tools":2,"resources":4})
    );
    call(&manager, "fake_echo", "change_resources").await;
    wait_tool(&manager, "fake_read_project_notes").await;
    assert!(!manager
        .registered_tools()
        .iter()
        .any(|t| t.tool.schema().name == "fake_read_new_notes"));
    assert!(approvals
        .lock()
        .unwrap()
        .contains("fake_read_project_notes"));
    call(&manager, "fake_echo", "add_tool").await;
    wait_tool(&manager, "fake_added").await;
    let counts = call(&manager, "fake_echo", "catalog_counts").await;
    assert_eq!(
        serde_json::from_str::<Value>(text(&counts)).unwrap(),
        json!({"tools":4,"resources":6})
    );
    assert_eq!(text(&call(&manager, "fake_echo", "pid").await), text(&pid));
    assert_eq!(
        text(&call(&manager, "fake_read_echo", "pid").await),
        text(&pid)
    );
    let names: Vec<_> = harness
        .get_tools()
        .await
        .unwrap()
        .iter()
        .map(|t| t.tool.schema().name.clone())
        .collect();
    assert_eq!(names.iter().filter(|n| *n == "fake_read_echo").count(), 1);
}

#[tokio::test]
async fn a_resource_only_server_does_not_need_tools_capability() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = fixture(true);
    config
        .env
        .insert("ICERS_MCP_TEST_RESOURCES_ONLY".into(), "1".into());
    let client = Client::spawn("fake", &config, dir.path(), Handlers::default()).unwrap();
    let tools = client
        .discover("fake", &config, &OutputGuard::default())
        .await
        .unwrap();
    assert_eq!(tools.len(), 2);
    assert!(tools.iter().all(|tool| tool.resource_uri.is_some()));
}

#[tokio::test]
async fn a_new_real_tool_replaces_a_colliding_resource() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = fixture(true);
    config
        .env
        .insert("ICERS_MCP_TEST_COLLISION".into(), "later".into());
    let (_harness, manager, _) = manager(dir.path(), &config, Default::default()).await;
    assert_eq!(
        text(&call(&manager, "fake_read_echo", "pid").await),
        "resource echo"
    );
    let pid = call(&manager, "fake_echo", "pid").await;
    call(&manager, "fake_echo", "add_tool").await;
    wait_tool(&manager, "fake_added").await;
    assert_eq!(
        text(&call(&manager, "fake_read_echo", "pid").await),
        text(&pid)
    );
}
