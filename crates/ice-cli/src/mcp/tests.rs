use super::*;
#[path = "../../tests/support/mcp_fixture.rs"]
mod mcp_fixture;

fn fixture(dir: &Path) -> ServerConfig {
    let binary = mcp_fixture::server_executable();
    serde_json::from_value(json!({"command":binary,"env":{"ICERS_MCP_TEST_CHILD":"1","ICERS_MCP_TEST_CANCEL":dir.join("cancel")},"approveTools":true})).unwrap()
}

#[tokio::test]
async fn instructions_follow_connections_without_overwriting_base_prompt() {
    use ice_ai::providers::faux::{FauxProvider, FauxScript};
    let dir = tempfile::tempdir().unwrap();
    let config = fixture(dir.path());
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
        system_prompt: Some("BASE".into()),
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
    assert_eq!(
        manager.instructions("fake").as_deref(),
        Some("Call echo first.")
    );
    assert!(harness
        .context_snapshot()
        .await
        .unwrap()
        .system_prompt
        .contains("Call echo first."));
    manager.reconnect(Some("fake")).await.unwrap();
    assert_eq!(
        harness
            .context_snapshot()
            .await
            .unwrap()
            .system_prompt
            .matches("Call echo first.")
            .count(),
        1
    );
    manager.set_disabled("fake", true).await.unwrap();
    assert!(manager.instructions("fake").is_none());
    assert_eq!(
        harness.context_snapshot().await.unwrap().system_prompt,
        "BASE"
    );
    manager.set_disabled("fake", false).await.unwrap();
    let tool = manager
        .registered_tools()
        .into_iter()
        .find(|tool| tool.tool.schema().name == "fake_echo")
        .unwrap();
    assert!(tool
        .tool
        .execute(
            "disconnect",
            json!({"mode":"disconnect"}),
            CancellationToken::new(),
            Arc::new(|_| {})
        )
        .await
        .is_err());
    tokio::time::timeout(Duration::from_secs(3), async {
        while harness.context_snapshot().await.unwrap().system_prompt != "BASE" {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(manager.instructions("fake").is_none());
    manager.reconnect(Some("fake")).await.unwrap();
    manager.remove("fake").await.unwrap();
    assert_eq!(
        harness.context_snapshot().await.unwrap().system_prompt,
        "BASE"
    );
}

#[tokio::test]
async fn call_answers_server_ping_and_unknown_requests() {
    let dir = tempfile::tempdir().unwrap();
    let config = fixture(dir.path());
    let client = Client::spawn("fake", &config, dir.path(), Handlers::default()).unwrap();
    client
        .discover("fake", &config, &OutputGuard::default())
        .await
        .unwrap();
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        client.request(
            "tools/call",
            json!({"name":"echo","arguments":{"mode":"server_requests"}}),
            CancellationToken::new(),
            Duration::from_secs(2),
        ),
    )
    .await
    .expect("server requests must not block the tool call")
    .unwrap();
    assert_eq!(result["content"][0]["text"], "server requests answered");
}

async fn call_text(client: &Client, arguments: Value) -> String {
    let result = client
        .request(
            "tools/call",
            json!({"name":"echo","arguments":arguments}),
            CancellationToken::new(),
            Duration::from_secs(5),
        )
        .await
        .unwrap();
    result["content"][0]["text"].as_str().unwrap().to_owned()
}

/// Client replies the fixture has received, polled until `done` accepts them.
async fn wait_responses(client: &Client, done: impl Fn(&[Value]) -> bool) -> Vec<Value> {
    for _ in 0..200 {
        let responses: Vec<Value> =
            serde_json::from_str(&call_text(client, json!({"mode":"responses"})).await).unwrap();
        if done(&responses) {
            return responses;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("client never answered the server requests");
}

/// `test/slow` answers only after `release` is cancelled.
fn slow_handlers(release: CancellationToken) -> Handlers {
    Handlers::default().request(
        "test/slow",
        json!({}),
        Arc::new(move |params, _| {
            let release = release.clone();
            Box::pin(async move {
                release.cancelled().await;
                Ok(json!({ "echo": params }))
            })
        }),
    )
}

#[tokio::test]
async fn pending_server_request_does_not_block_ping_tool_calls_or_cancellation() {
    let dir = tempfile::tempdir().unwrap();
    let config = fixture(dir.path());
    let release = CancellationToken::new();
    let client =
        Client::spawn("fake", &config, dir.path(), slow_handlers(release.clone())).unwrap();
    client
        .discover("fake", &config, &OutputGuard::default())
        .await
        .unwrap();
    let delayed = json!({"mode":"delayed_request","id":"slow"});
    assert_eq!(call_text(&client, delayed).await, "ping answered");
    let cancelled = json!({"mode":"cancelled_request","id":"gone"});
    assert_eq!(call_text(&client, cancelled).await, "ping answered");
    assert_eq!(call_text(&client, json!({"mode":"responses"})).await, "[]");
    release.cancel();
    let responses = wait_responses(&client, |r| !r.is_empty()).await;
    // The cancelled request must never be answered.
    assert_eq!(
        responses,
        vec![json!({"jsonrpc":"2.0","id":"slow","result":{"echo":null}})]
    );
}

#[tokio::test]
async fn inbound_requests_past_the_cap_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let config = fixture(dir.path());
    let release = CancellationToken::new();
    let client =
        Client::spawn("fake", &config, dir.path(), slow_handlers(release.clone())).unwrap();
    client
        .discover("fake", &config, &OutputGuard::default())
        .await
        .unwrap();
    let flood = json!({"mode":"flood","count":MAX_INBOUND + 1});
    let refused: Value = serde_json::from_str(&call_text(&client, flood).await).unwrap();
    assert_eq!(refused["id"], MAX_INBOUND);
    assert_eq!(refused["error"]["code"], -32603);
    release.cancel();
    let responses = wait_responses(&client, |r| r.len() == MAX_INBOUND).await;
    assert!(responses.iter().all(|r| r["result"].is_object()));
}

#[tokio::test]
async fn capabilities_are_advertised_only_for_registered_handlers() {
    let dir = tempfile::tempdir().unwrap();
    let config = fixture(dir.path());
    let sampling = Handlers::default().request(
        "sampling/createMessage",
        json!({}),
        Arc::new(|_, _| Box::pin(async { Ok(json!({})) })),
    );
    for (handlers, expected) in [
        (Handlers::default(), json!({})),
        (sampling, json!({"sampling":{}})),
    ] {
        let client = Client::spawn("fake", &config, dir.path(), handlers).unwrap();
        client
            .discover("fake", &config, &OutputGuard::default())
            .await
            .unwrap();
        let sent = call_text(&client, json!({"mode":"client_capabilities"})).await;
        assert_eq!(serde_json::from_str::<Value>(&sent).unwrap(), expected);
        assert_eq!(client.instructions(), Some("Call echo first."));
        assert!(client.declares("tools") && !client.declares("prompts"));
    }
}

#[tokio::test]
async fn tools_list_changed_relists_without_restarting_the_server() {
    use ice_ai::providers::faux::{FauxProvider, FauxScript};
    use ice_harness::types::AgentHarnessOptions;
    let dir = tempfile::tempdir().unwrap();
    let config = fixture(dir.path());
    std::fs::write(
        dir.path().join("mcp.json"),
        serde_json::to_vec(
            &json!({"mcpServers":{"fake":{"command":config.command,"env":config.env}}}),
        )
        .unwrap(),
    )
    .unwrap();
    let provider = FauxProvider::new(FauxScript::new());
    let harness = AgentHarness::create(AgentHarnessOptions {
        model: provider.default_model().clone(),
        models: vec![provider],
        ..Default::default()
    })
    .await
    .unwrap();
    let manager = McpManager::new(
        &harness,
        &crate::args::Args::default(),
        dir.path(),
        dir.path().into(),
        false,
        Arc::new(Mutex::new(HashSet::new())),
    );
    manager.reconnect(None).await.unwrap();
    assert_eq!(
        manager.instructions("fake").as_deref(),
        Some("Call echo first.")
    );
    let call = |mode: &'static str| {
        let echo = manager
            .registered_tools()
            .into_iter()
            .find(|t| t.tool.schema().name == "fake_echo")
            .unwrap();
        async move {
            let result = echo
                .tool
                .execute(
                    "1",
                    json!({ "mode": mode }),
                    CancellationToken::new(),
                    Arc::new(|_| {}),
                )
                .await
                .unwrap();
            result.content[0].clone()
        }
    };
    let pid = call("pid").await;
    call("add_tool").await;
    for _ in 0..200 {
        if manager.tools("fake").iter().any(|(n, _)| n == "fake_added") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let names: Vec<String> = harness
        .get_tools()
        .await
        .unwrap()
        .iter()
        .map(|t| t.tool.schema().name.clone())
        .collect();
    for tool in ["fake_echo", "fake_other", "fake_added"] {
        assert_eq!(names.iter().filter(|n| *n == tool).count(), 1, "{names:?}");
    }
    assert_eq!(
        call("pid").await,
        pid,
        "refresh must reuse the server process"
    );
}

#[tokio::test]
async fn handshake_pagination_call_content_error_timeout_crash_and_cancel() {
    let dir = tempfile::tempdir().unwrap();
    let config = fixture(dir.path());
    let client = Client::spawn("fake", &config, dir.path(), Handlers::default()).unwrap();
    let tools = client
        .discover("fake", &config, &OutputGuard::default())
        .await
        .unwrap();
    assert_eq!(tools.len(), 2);
    assert_eq!(tools[0].schema.name, "fake_echo");
    assert!(tools[0].approval);
    let result = tools[0]
        .execute("1", json!({}), CancellationToken::new(), Arc::new(|_| {}))
        .await
        .unwrap();
    assert_eq!(result.content.len(), 4);
    assert_eq!(result.content[0], TextContentOrImage::text("hello"));
    assert!(
        matches!(&result.content[1], TextContentOrImage::Image(i) if i.mime_type == "image/png" && i.data == "YWJj")
    );
    assert_eq!(result.content[2], TextContentOrImage::text("embedded"));
    assert!(tools[0]
        .execute(
            "2",
            json!({"mode":"error"}),
            CancellationToken::new(),
            Arc::new(|_| {})
        )
        .await
        .is_err());
    let timeout = client
        .request(
            "tools/call",
            json!({"name":"echo","arguments":{"mode":"hang"}}),
            CancellationToken::new(),
            Duration::from_millis(50),
        )
        .await
        .unwrap_err();
    assert!(timeout.contains("timed out"));
    let signal = CancellationToken::new();
    let cancel = signal.clone();
    let future = client.request(
        "tools/call",
        json!({"name":"echo","arguments":{"mode":"hang"}}),
        signal,
        CALL_TIMEOUT,
    );
    let (result, _) = tokio::join!(future, async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        cancel.cancel();
    });
    assert!(result.unwrap_err().contains("cancelled"));
    for _ in 0..100 {
        if std::fs::read_to_string(dir.path().join("cancel"))
            .unwrap_or_default()
            .contains("cancelled")
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(std::fs::read_to_string(dir.path().join("cancel"))
        .unwrap()
        .contains("requestId"));
    assert!(client.pending.lock().unwrap().is_empty());
    let first = client.request(
        "tools/call",
        json!({"name":"echo","arguments":{"mode":"error"}}),
        CancellationToken::new(),
        CALL_TIMEOUT,
    );
    let second = client.request(
        "tools/call",
        json!({"name":"echo","arguments":{}}),
        CancellationToken::new(),
        CALL_TIMEOUT,
    );
    let (first, second) = tokio::join!(first, second);
    assert_eq!(first.unwrap()["isError"], true);
    assert_eq!(second.unwrap()["content"][0]["text"], "hello");
    let crashed = client
        .request(
            "tools/call",
            json!({"name":"echo","arguments":{"mode":"crash"}}),
            CancellationToken::new(),
            Duration::from_secs(2),
        )
        .await
        .unwrap_err();
    assert!(
        crashed.contains("closed") || crashed.contains("disconnected"),
        "{crashed}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn first_prompt_includes_delayed_mcp_tools() {
    use ice_ai::providers::faux::{faux_assistant_message, FauxProvider, FauxScript, FauxStep};
    use ice_ai::types::StopReason;
    use ice_harness::types::AgentHarnessOptions;

    let dir = tempfile::tempdir().unwrap();
    let mut config = fixture(dir.path());
    config.env.insert("MCP_TEST_DELAY_MS".into(), "500".into());
    let script = FauxScript::new();
    script.set_responses(vec![FauxStep::factory(|ctx| {
        assert!(ctx.tools.iter().any(|tool| tool.name == "fake_echo"));
        faux_assistant_message("ready", StopReason::Stop)
    })]);
    let provider = FauxProvider::new(script);
    let harness = AgentHarness::create(AgentHarnessOptions {
        model: provider.default_model().clone(),
        models: vec![provider],
        ..Default::default()
    })
    .await
    .unwrap();
    std::fs::write(dir.path().join("mcp.json"), serde_json::to_vec(&json!({"mcpServers":{"fake":{"command":config.command,"env":config.env,"approveTools":true}}})).unwrap()).unwrap();
    let args = crate::args::Args::default();
    let manager = McpManager::new(
        &harness,
        &args,
        dir.path(),
        dir.path().into(),
        false,
        Arc::new(Mutex::new(HashSet::new())),
    );
    let ready = start(manager);
    harness.set_lifecycle_hooks(Arc::new(McpReadyGate { inner: None, ready }));
    harness
        .lane("main")
        .prompt_text("go", vec![])
        .await
        .unwrap();
}

#[tokio::test]
async fn readiness_gate_bounds_hanging_discovery_and_accepts_closed_sender() {
    let (tx, ready) = watch::channel(false);
    let gate = McpReadyGate { inner: None, ready };
    tokio::time::timeout(
        Duration::from_secs(1),
        gate.wait_ready(Duration::from_millis(20)),
    )
    .await
    .unwrap();
    drop(tx);
    tokio::time::timeout(Duration::from_millis(100), gate.wait_ready(START_TIMEOUT))
        .await
        .unwrap();
}

#[test]
fn project_trust_and_server_override() {
    let dir = tempfile::tempdir().unwrap();
    let agent = dir.path().join("agent");
    let project = dir.path().join("project");
    std::fs::create_dir_all(&agent).unwrap();
    std::fs::create_dir_all(project.join(".icers")).unwrap();
    std::fs::write(
        agent.join("mcp.json"),
        r#"{"mcpServers":{"a":{"command":"global","approveTools":["one"]}}}"#,
    )
    .unwrap();
    std::fs::write(
        project.join(".icers/mcp.json"),
        r#"{"mcpServers":{"a":{"disabled":true}}}"#,
    )
    .unwrap();
    let global = load_config(&agent, &project, false).unwrap();
    assert_eq!(global["a"].command, "global");
    assert!(global["a"].approve_tools.requires("one"));
    assert!(!global["a"].approve_tools.requires("two"));
    assert!(load_config(&agent, &project, true).unwrap()["a"].disabled);
    std::fs::write(project.join(".icers/mcp.json"), "invalid").unwrap();
    assert!(load_config(&agent, &project, false).is_ok());
    assert!(load_config(&agent, &project, true).is_err());
}

#[tokio::test]
async fn approvals_are_interactive_per_call_and_fail_closed() {
    let ui = UiDialogMailbox::new();
    assert!(
        approve(&ui, "headless", "fake_echo", &CancellationToken::new())
            .await
            .unwrap_err()
            .contains("interactive")
    );
    ui.attach();
    for allow in [true, false] {
        let signal = CancellationToken::new();
        let approval = approve(&ui, "call", "fake_echo", &signal);
        let answer = async {
            loop {
                if let Some(request) = ui.take_pending() {
                    ui.respond(
                        &request.request_id,
                        json!({"values":[if allow {"Allow"} else {"Deny"}]}),
                    )
                    .unwrap();
                    break;
                }
                tokio::task::yield_now().await;
            }
        };
        let (result, _) = tokio::join!(approval, answer);
        assert_eq!(result.is_ok(), allow);
    }
    let signal = CancellationToken::new();
    signal.cancel();
    assert!(approve(&ui, "cancel", "fake_echo", &signal).await.is_err());
    assert_eq!(ui.pending_len(), 0);
}

#[tokio::test]
async fn manager_lifecycle_preserves_config_and_replaces_tools() {
    use ice_ai::providers::faux::{FauxProvider, FauxScript};
    use ice_harness::types::AgentHarnessOptions;
    let dir = tempfile::tempdir().unwrap();
    let agent = dir.path().join("agent");
    std::fs::create_dir_all(&agent).unwrap();
    std::fs::create_dir_all(dir.path().join(".icers")).unwrap();
    let config = fixture(dir.path());
    let entry =
        json!({"command":config.command,"env":config.env,"approveTools":true,"extra":"keep"});
    let global = agent.join("mcp.json");
    let project = dir.path().join(".icers/mcp.json");
    std::fs::write(&global, serde_json::to_vec(&json!({"other":42,"mcpServers":{"fake":entry,"bad":{"command":"icers-nonexistent-mcp-fixture"},"off":{"disabled":true}}})).unwrap()).unwrap();
    std::fs::write(
        &project,
        serde_json::to_vec(&json!({"projectKey":7,"mcpServers":{"fake":entry}})).unwrap(),
    )
    .unwrap();
    let provider = FauxProvider::new(FauxScript::new());
    let harness = AgentHarness::create(AgentHarnessOptions {
        model: provider.default_model().clone(),
        models: vec![provider],
        ..Default::default()
    })
    .await
    .unwrap();
    let approvals = Arc::new(Mutex::new(HashSet::new()));
    let manager = McpManager::new(
        &harness,
        &crate::args::Args::default(),
        dir.path(),
        agent,
        true,
        approvals.clone(),
    );
    manager.reconnect(None).await.unwrap();
    let snapshot = manager.snapshot();
    assert!(matches!(
        snapshot.iter().find(|s| s.name == "fake").unwrap().status,
        ServerStatus::Connected { .. }
    ));
    assert!(matches!(
        snapshot.iter().find(|s| s.name == "bad").unwrap().status,
        ServerStatus::Failed(_)
    ));
    assert!(matches!(
        snapshot.iter().find(|s| s.name == "off").unwrap().status,
        ServerStatus::Disabled
    ));
    assert!(approvals.lock().unwrap().contains("fake_echo"));
    manager.reconnect(Some("fake")).await.unwrap();
    assert_eq!(
        harness
            .get_tools()
            .await
            .unwrap()
            .iter()
            .filter(|t| t.tool.schema().name == "fake_echo")
            .count(),
        1
    );
    manager.set_disabled("fake", true).await.unwrap();
    assert!(manager.tools("fake").is_empty());
    assert!(!approvals.lock().unwrap().contains("fake_echo"));
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(&project).unwrap()).unwrap();
    assert_eq!(saved["mcpServers"]["fake"]["disabled"], true);
    assert_eq!(saved["mcpServers"]["fake"]["extra"], "keep");
    assert_eq!(saved["projectKey"], 7);
    assert!(!std::fs::read_to_string(&global)
        .unwrap()
        .contains("\"disabled\": true"));
    manager.set_disabled("fake", false).await.unwrap();
    assert!(approvals.lock().unwrap().contains("fake_echo"));
    manager
        .add("new", "icers-missing-mcp-fixture", &["arg".into()])
        .await
        .unwrap();
    assert!(manager.add("bad/name", "cmd", &[]).await.is_err());
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(&global).unwrap()).unwrap();
    assert_eq!(saved["mcpServers"]["new"]["args"], json!(["arg"]));
    assert_eq!(saved["other"], 42);
    manager.remove("new").await.unwrap();
    std::fs::write(&project, "{\"mcpServers\":{}}").unwrap();
    manager.remove("fake").await.unwrap();
    assert!(!manager.snapshot().iter().any(|s| s.name == "fake"));
    assert!(!harness
        .get_tools()
        .await
        .unwrap()
        .iter()
        .any(|t| t.tool.schema().name == "fake_echo"));
    assert!(!approvals.lock().unwrap().contains("fake_echo"));
    assert!(manager.registered_tools().is_empty());
}

#[tokio::test]
async fn panel_save_writes_direct_tools_and_registers_only_selected() {
    use ice_ai::providers::faux::{FauxProvider, FauxScript};
    use ice_harness::types::AgentHarnessOptions;
    let dir = tempfile::tempdir().unwrap();
    let config = fixture(dir.path());
    let path = dir.path().join("mcp.json");
    std::fs::write(
        &path,
        serde_json::to_vec(
            &json!({"mcpServers":{"fake":{"command":config.command,"env":config.env,"extra":1}}}),
        )
        .unwrap(),
    )
    .unwrap();
    let provider = FauxProvider::new(FauxScript::new());
    let harness = AgentHarness::create(AgentHarnessOptions {
        model: provider.default_model().clone(),
        models: vec![provider],
        ..Default::default()
    })
    .await
    .unwrap();
    let manager = McpManager::new(
        &harness,
        &crate::args::Args::default(),
        dir.path(),
        dir.path().into(),
        false,
        Arc::new(Mutex::new(HashSet::new())),
    );
    let registered = || async {
        let mut names: Vec<_> = harness
            .get_tools()
            .await
            .unwrap()
            .iter()
            .map(|t| t.tool.schema().name.clone())
            .filter(|n| n.starts_with("fake_"))
            .collect();
        names.sort();
        names
    };
    let catalog = || {
        let view = manager.snapshot().remove(0);
        let mut tools: Vec<_> = view
            .catalog
            .unwrap()
            .into_iter()
            .map(|t| (t.name, t.direct))
            .collect();
        tools.sort();
        (view.status, tools)
    };
    manager.reconnect(None).await.unwrap();
    assert_eq!(registered().await, ["fake_echo", "fake_other"]);
    let echo = manager.snapshot()[0].catalog.clone().unwrap();
    let echo = echo.iter().find(|t| t.name == "echo").unwrap();
    assert_eq!(echo.description, "Echo");
    assert!(echo.tokens > 10);

    let save = |disabled, direct_tools| PanelChange {
        server: "fake".into(),
        disabled,
        direct_tools,
    };
    manager
        .save_panel(&[save(None, Some(DirectTools::Tools(vec!["echo".into()])))])
        .await
        .unwrap();
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(saved["mcpServers"]["fake"]["directTools"], json!(["echo"]));
    assert_eq!(saved["mcpServers"]["fake"]["extra"], 1);
    assert_eq!(registered().await, ["fake_echo"]);
    assert_eq!(manager.registered_tools().len(), 1);
    assert_eq!(
        catalog().1,
        [("echo".into(), true), ("other".into(), false)]
    );

    manager
        .save_panel(&[save(None, Some(DirectTools::All(false)))])
        .await
        .unwrap();
    assert!(registered().await.is_empty());
    assert_eq!(catalog().1.len(), 2);

    // Disabling keeps the catalog; flags follow the saved directTools.
    manager
        .save_panel(&[save(Some(true), Some(DirectTools::All(true)))])
        .await
        .unwrap();
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(saved["mcpServers"]["fake"]["directTools"], true);
    assert_eq!(saved["mcpServers"]["fake"]["disabled"], true);
    let (status, tools) = catalog();
    assert!(matches!(status, ServerStatus::Disabled));
    assert_eq!(tools, [("echo".into(), true), ("other".into(), true)]);
    assert!(registered().await.is_empty());

    manager
        .save_panel(&[save(Some(false), None)])
        .await
        .unwrap();
    assert_eq!(registered().await, ["fake_echo", "fake_other"]);
    assert!(manager
        .save_panel(&[PanelChange {
            server: "missing".into(),
            disabled: Some(true),
            direct_tools: None,
        }])
        .await
        .is_err());
}

#[tokio::test]
async fn output_guard_follows_mcp_json_settings() {
    use ice_ai::providers::faux::{FauxProvider, FauxScript};
    use ice_harness::types::AgentHarnessOptions;
    let texts = |content: &[TextContentOrImage]| -> Vec<String> {
        content
            .iter()
            .filter_map(|block| match block {
                TextContentOrImage::Text(text) => Some(text.text.clone()),
                TextContentOrImage::Image(_) => None,
            })
            .collect()
    };
    for (settings, guarded) in [(json!({}), true), (json!({"outputGuard":false}), false)] {
        let dir = tempfile::tempdir().unwrap();
        let config = fixture(dir.path());
        std::fs::write(
            dir.path().join("mcp.json"),
            serde_json::to_vec(&json!({"settings":settings,"mcpServers":{"fake":{"command":config.command,"env":config.env}}}))
                .unwrap(),
        )
        .unwrap();
        let provider = FauxProvider::new(FauxScript::new());
        let harness = AgentHarness::create(AgentHarnessOptions {
            model: provider.default_model().clone(),
            models: vec![provider],
            ..Default::default()
        })
        .await
        .unwrap();
        let manager = McpManager::new(
            &harness,
            &crate::args::Args::default(),
            dir.path(),
            dir.path().into(),
            false,
            Arc::new(Mutex::new(HashSet::new())),
        );
        manager.reconnect(None).await.unwrap();
        let echo = manager
            .registered_tools()
            .into_iter()
            .find(|t| t.tool.schema().name == "fake_echo")
            .unwrap();
        let result = echo
            .tool
            .execute(
                "1",
                json!({"mode":"big"}),
                CancellationToken::new(),
                Arc::new(|_| {}),
            )
            .await
            .unwrap();
        let images = result
            .content
            .iter()
            .filter(|b| matches!(b, TextContentOrImage::Image(i) if i.data == "YWJj"))
            .count();
        assert_eq!(images, 1, "images pass through either way");
        let texts = texts(&result.content);
        // The blob is materialized even with the guard off, and never kept as base64.
        assert!(!result.details.to_string().contains("aGVsbG8="));
        if guarded {
            assert_eq!(texts.len(), 1);
            // 3,000 rows, the join separator, and the 3-line blob reference.
            assert!(texts[0].contains("[MCP text output truncated: original 3004 lines"));
            let start = texts[0].find("Full text saved to: ").unwrap() + 20;
            let end = texts[0][start..].find(" — ").unwrap();
            let spilled = std::fs::read_to_string(&texts[0][start..start + end]).unwrap();
            assert!(spilled.ends_with("MIME type: application/octet-stream"));
            assert!(texts[0].len() <= ice_tools::DEFAULT_MAX_BYTES);
            assert_eq!(result.details["omitted"], true);
        } else {
            assert_eq!(texts.len(), 2);
            assert_eq!(texts[0].lines().count(), 3000);
            assert!(texts[1].contains("Binary content saved to "));
            assert_eq!(result.details["content"][0]["text"], texts[0]);
        }
    }
}
