use super::*;
#[path = "../../tests/support/mcp_fixture.rs"]
mod mcp_fixture;

fn write(dir: &Path, server: Value) {
    std::fs::write(
        dir.join("mcp.json"),
        serde_json::to_vec(&json!({"mcpServers":{"fake":server}})).unwrap(),
    )
    .unwrap();
}
fn fixture(extra: Value) -> Value {
    let mut server =
        json!({"command":mcp_fixture::server_executable(),"env":{"ICERS_MCP_TEST_CHILD":"1"}});
    server
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    server
}
async fn session(dir: &Path) -> (AgentHarness, McpManager) {
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
async fn call(harness: &AgentHarness, name: &str, mode: &str) -> Result<String, AgentError> {
    let tool = harness
        .get_tools()
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.tool.schema().name == name)
        .unwrap_or_else(|| panic!("{name} registered"));
    let result = tool
        .tool
        .execute(
            "t",
            json!({"mode":mode}),
            CancellationToken::new(),
            Arc::new(|_| {}),
        )
        .await?;
    match &result.content[0] {
        TextContentOrImage::Text(text) => Ok(text.text.clone()),
        _ => panic!("expected text"),
    }
}
fn status(manager: &McpManager) -> ServerStatus {
    manager.snapshot().remove(0).status
}

#[tokio::test]
async fn lazy_servers_start_on_first_call_and_stop_when_idle() {
    let dir = tempfile::tempdir().unwrap();
    // Without a cache entry, a lazy server starts once to fill it.
    write(dir.path(), fixture(json!({"lifecycle":"lazy"})));
    let (_first, manager) = session(dir.path()).await;
    assert!(matches!(status(&manager), ServerStatus::Connected { .. }));
    drop(manager);

    // A new session registers the cached tools without starting the server.
    write(
        dir.path(),
        fixture(json!({"lifecycle":"lazy","idleTimeout":0.0001})),
    );
    let (harness, manager) = session(dir.path()).await;
    assert!(matches!(status(&manager), ServerStatus::Idle { ref tools } if tools.len() == 2));
    let pid = call(&harness, "fake_echo", "pid").await.unwrap();
    assert!(pid.parse::<u32>().is_ok(), "{pid}");
    assert!(matches!(status(&manager), ServerStatus::Connected { .. }));

    tokio::time::sleep(Duration::from_millis(50)).await;
    manager.check_lifecycle().await;
    assert!(matches!(status(&manager), ServerStatus::Idle { .. }));
    // The proxy starts it again, too.
    let again = manager
        .proxy_view()
        .into_iter()
        .find(|s| s.name == "fake")
        .unwrap();
    assert_eq!(again.entries.len(), 2);
    assert_ne!(call(&harness, "fake_echo", "pid").await.unwrap(), pid);
}

#[tokio::test]
async fn keep_alive_servers_restart_with_backoff() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), fixture(json!({"lifecycle":"keep-alive"})));
    let (harness, manager) = session(dir.path()).await;
    let pid = call(&harness, "fake_echo", "pid").await.unwrap();
    assert!(call(&harness, "fake_echo", "disconnect").await.is_err());
    manager.check_lifecycle().await;
    assert!(matches!(status(&manager), ServerStatus::Connected { .. }));
    assert_ne!(call(&harness, "fake_echo", "pid").await.unwrap(), pid);

    // A server that cannot start is retried after 30 s, then 60 s.
    write(
        dir.path(),
        json!({"command":"icers-missing-mcp-fixture","lifecycle":"keep-alive"}),
    );
    manager.reconnect(None).await.unwrap();
    assert!(matches!(status(&manager), ServerStatus::Failed(_)));
    manager.check_lifecycle().await;
    let backoff = |manager: &McpManager| manager.0.retries.lock().unwrap()["fake"].1;
    assert_eq!(backoff(&manager), RETRY_FIRST);
    // Not due yet: no new attempt, so the backoff is unchanged.
    manager.check_lifecycle().await;
    assert_eq!(backoff(&manager), RETRY_FIRST);
    manager.0.retries.lock().unwrap().get_mut("fake").unwrap().0 = std::time::Instant::now();
    manager.check_lifecycle().await;
    assert_eq!(backoff(&manager), RETRY_FIRST * 2);
}

#[test]
fn request_timeout_and_lifecycle_parse_from_config() {
    let config: ServerConfig = serde_json::from_value(
        json!({"command":"c","lifecycle":"lazy-keep-alive","requestTimeoutMs":1500}),
    )
    .unwrap();
    assert_eq!(config.lifecycle, Lifecycle::LazyKeepAlive);
    assert!(config.lifecycle.lazy() && config.lifecycle.keep_alive());
    assert_eq!(config.request_timeout(), Duration::from_millis(1500));
    let default: ServerConfig = serde_json::from_value(json!({"command":"c"})).unwrap();
    assert_eq!(default.lifecycle, Lifecycle::Eager);
    assert_eq!(default.request_timeout(), CALL_TIMEOUT);
}
