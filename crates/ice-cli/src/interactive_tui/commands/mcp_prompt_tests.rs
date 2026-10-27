use super::*;
#[path = "../../../tests/support/mcp_fixture.rs"]
mod mcp_fixture;

#[tokio::test]
async fn prompt_dispatch_submission_and_live_completion() {
    use ice_ai::providers::faux::{FauxProvider, FauxScript};
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("mcp.json"), serde_json::to_vec(&serde_json::json!({"mcpServers":{"fake":{
        "command":mcp_fixture::server_executable(),"env":{"ICERS_MCP_TEST_CHILD":"1","ICERS_MCP_TEST_PROMPTS":"1"}
    }}})).unwrap()).unwrap();
    let provider = FauxProvider::new(FauxScript::new());
    let harness = AgentHarness::create(ice_harness::types::AgentHarnessOptions {
        model: provider.default_model().clone(),
        models: vec![provider],
        ..Default::default()
    })
    .await
    .unwrap();
    let lane = harness.lane("main");
    let session = Arc::new(crate::agent_session::AgentSession::new(
        harness,
        vec![],
        vec![],
        dir.path(),
    ));
    let state = crate::interactive_tui::tests::test_tui_state();
    let catalog = Arc::new(std::sync::RwLock::new(vec![]));
    let manager = McpManager::new(
        session.harness(),
        &Default::default(),
        dir.path(),
        dir.path().into(),
        false,
        Default::default(),
    );
    manager.reconnect(None).await.unwrap();
    let reload_context = Arc::new(crate::session::ReloadContext {
        skill_registry: Default::default(),
        capability_service: None,
        mcp_tools: manager.clone(),
        shells: crate::background_shells::SessionShells::new(&Default::default()),
        extension_session: state.extension_session.clone(),
        tool_context: ice_extensions::ToolCallContext::new(dir.path().display().to_string()),
        action_bridge: Arc::new(Mutex::new(None)),
        ext_status: state.ext_status.clone(),
        editor_text: state.editor_text.clone(),
        catalog: catalog.clone(),
        providers: Default::default(),
        runtime: tokio::runtime::Handle::current(),
        cwd: dir.path().into(),
        project_trusted: false,
        args: Default::default(),
        resolved_model: session.model().await.unwrap(),
        broadcast: Arc::new(ice_agent::events::CollectorEmitter::default()),
        mailbox: Default::default(),
        ui_dialog: Default::default(),
        dev_extension: None,
        builtins: crate::builtin_extensions::BuiltinExtensions::disabled(),
    });
    let (tx, mut rx) = mpsc::unbounded_channel();
    let ctx = CommandContext {
        chat: Arc::new(Container::new()),
        tui: Arc::new(TuiAltScreen::new(
            Box::new(ProcessTerminal::new()),
            true,
            None,
        )),
        tx,
        state: state.clone(),
        editor: state.editor.clone(),
        editor_container: Arc::new(Container::new()),
        lane,
        model_catalog: catalog,
        cwd: dir.path().into(),
        resources: Default::default(),
        reload_context,
        session,
    };
    let mut registry = build_builtin_registry();
    register_prompts(&mut registry, &manager);
    let provider = autocomplete::build_autocomplete_with_mcp(
        &registry,
        &Default::default(),
        dir.path(),
        None,
        Some(manager.clone()),
    );
    let input = "/mcp__fake__B";
    assert_eq!(
        provider.get_suggestions(input, input.len()).unwrap().items[0].text,
        "/mcp__fake__Brief"
    );
    assert!(dispatch_slash(
        "/mcp__fake__Brief today topic='two  words'",
        &ctx,
        &registry
    ));
    let message = tokio::time::timeout(std::time::Duration::from_secs(3), rx.recv())
        .await
        .unwrap()
        .unwrap();
    let TuiMessage::UserInput(prompt, images) = message else {
        panic!("expected normal submission")
    };
    assert!(images.is_empty());
    assert!(prompt.contains("two  words"));
    assert!(prompt.ends_with("[assistant] Ready"));
    assert!(*state.status.lock().unwrap() == RunStatus::Working);
    assert!(ctx.chat.child_count() > 0);
    assert!(apply(&manager, "tools fake")
        .await
        .unwrap()
        .contains("Instructions:\nCall echo first."));
    assert!(!apply(&manager, "tools")
        .await
        .unwrap()
        .contains("Instructions:"));
    assert!(prompts_report(&manager, Some("fake")).contains("/mcp__fake__Brief"));
    assert_eq!(
        prompts_report(&manager, Some("missing")),
        "No MCP prompts available."
    );

    assert!(dispatch_slash(
        "/mcp__fake__Brief tomorrow",
        &ctx,
        &registry
    ));
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while ctx
            .lane
            .queued_messages()
            .await
            .unwrap()
            .steering
            .is_empty()
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(
        rx.try_recv().is_err(),
        "active runs must steer rather than start another run"
    );

    let mut changes = manager.prompt_changes();
    let tool = manager
        .registered_tools()
        .into_iter()
        .find(|t| t.tool.schema().name == "fake_echo")
        .unwrap();
    tool.tool
        .execute(
            "change",
            serde_json::json!({"mode":"change_prompts"}),
            tokio_util::sync::CancellationToken::new(),
            Arc::new(|_| {}),
        )
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(3), changes.changed())
        .await
        .unwrap()
        .unwrap();
    let mut fresh = build_builtin_registry();
    register_prompts(&mut fresh, &manager);
    registry.replace(fresh);
    assert!(registry.find("/mcp__fake__Brief").is_none());
    assert!(registry.find("/mcp__fake__New_Brief").is_some());
    let provider = autocomplete::build_autocomplete_with_mcp(
        &registry,
        &Default::default(),
        dir.path(),
        None,
        Some(manager),
    );
    let input = "/mcp__fake__New";
    assert_eq!(
        provider.get_suggestions(input, input.len()).unwrap().items[0].text,
        "/mcp__fake__New_Brief"
    );
}
