use super::*;
#[path = "../../tests/support/mcp_fixture.rs"]
mod mcp_fixture;

fn fixture(prompts: bool) -> ServerConfig {
    let mut config: ServerConfig = serde_json::from_value(json!({
        "command":mcp_fixture::server_executable(),
        "env":{"ICERS_MCP_TEST_CHILD":"1"}
    }))
    .unwrap();
    if prompts {
        config
            .env
            .insert("ICERS_MCP_TEST_PROMPTS".into(), "1".into());
    }
    config
}

#[tokio::test]
async fn paginated_prompts_validate_before_get_and_skip_absent_capability() {
    let dir = tempfile::tempdir().unwrap();
    for enabled in [false, true] {
        let mut config = fixture(enabled);
        let probe = dir.path().join("get.json");
        config.env.insert(
            "ICERS_MCP_TEST_PROMPT_GET".into(),
            probe.display().to_string(),
        );
        let client = Client::spawn("fake", &config, dir.path(), Handlers::default()).unwrap();
        client
            .discover("fake", &config, &OutputGuard::default())
            .await
            .unwrap();
        let prompts = client.list_prompts().await.unwrap();
        if !enabled {
            assert!(prompts.is_empty());
            continue;
        }
        assert_eq!(
            prompts
                .iter()
                .map(|p| p.command_name("fake"))
                .collect::<Vec<_>>(),
            ["/mcp__fake__Brief", "/mcp__fake__Review-Code"]
        );
        assert_eq!(prompts[0].description, "Daily brief");
        assert!(prompts::get_prompt(&client, "fake", &prompts[0], "")
            .await
            .unwrap_err()
            .contains("Usage:"));
        assert!(!probe.exists());
        let result = prompts::get_prompt(&client, "fake", &prompts[0], "today topic='two  words'")
            .await
            .unwrap();
        assert!(result.contains("[user]"));
        assert!(result.ends_with("[assistant] Ready"));
        let sent: Value = serde_json::from_slice(&std::fs::read(probe).unwrap()).unwrap();
        assert_eq!(
            sent,
            json!({"name":"Brief","arguments":{"day":"today","topic":"two  words"}})
        );
    }
}

#[tokio::test]
async fn prompt_pagination_is_bounded() {
    let dir = tempfile::tempdir().unwrap();
    for (mode, expected) in [("repeat", "repeated prompt"), ("endless", "exceeded 100")] {
        let mut config = fixture(true);
        config
            .env
            .insert("ICERS_MCP_TEST_PROMPT_CURSOR".into(), mode.into());
        let client = Client::spawn("fake", &config, dir.path(), Handlers::default()).unwrap();
        client
            .discover("fake", &config, &OutputGuard::default())
            .await
            .unwrap();
        assert!(client.list_prompts().await.unwrap_err().contains(expected));
    }
}

#[tokio::test]
async fn prompt_catalog_notifications_reconnect_disable_and_remove() {
    use ice_ai::providers::faux::{FauxProvider, FauxScript};
    let dir = tempfile::tempdir().unwrap();
    let config = fixture(true);
    std::fs::write(
        dir.path().join("mcp.json"),
        serde_json::to_vec(
            &json!({"mcpServers":{"fake":{"command":config.command,"env":config.env}}}),
        )
        .unwrap(),
    )
    .unwrap();
    let provider = FauxProvider::new(FauxScript::new());
    let harness = AgentHarness::create(ice_harness::types::AgentHarnessOptions {
        model: provider.default_model().clone(),
        models: vec![provider],
        ..Default::default()
    })
    .await
    .unwrap();
    let manager = McpManager::new(
        &harness,
        &Default::default(),
        dir.path(),
        dir.path().into(),
        false,
        Default::default(),
    );
    manager.reconnect(None).await.unwrap();
    let mut changes = manager.prompt_changes();
    let tool = manager
        .registered_tools()
        .into_iter()
        .find(|t| t.tool.schema().name == "fake_echo")
        .unwrap();
    tool.tool
        .execute(
            "change",
            json!({"mode":"change_prompts"}),
            CancellationToken::new(),
            Arc::new(|_| {}),
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), changes.changed())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        manager.prompts()[0].1.command_name("fake"),
        "/mcp__fake__New_Brief"
    );
    assert!(manager
        .get_prompt("/mcp__fake__Brief", "today")
        .await
        .is_err());
    manager.reconnect(Some("fake")).await.unwrap();
    assert_eq!(manager.prompts()[0].1.name, "Brief");
    manager.set_disabled("fake", true).await.unwrap();
    assert!(manager.prompts().is_empty());
    manager.set_disabled("fake", false).await.unwrap();
    assert_eq!(manager.prompts().len(), 2);
    manager.remove("fake").await.unwrap();
    assert!(manager.prompts().is_empty());
}
