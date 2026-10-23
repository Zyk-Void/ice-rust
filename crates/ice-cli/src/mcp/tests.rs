use super::*;

fn fixture(dir: &Path) -> ServerConfig {
    let exe = std::env::current_exe().unwrap();
    let binary = exe
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(format!("mcp-test-server{}", std::env::consts::EXE_SUFFIX));
    serde_json::from_value(json!({"command":binary,"env":{"ICERS_MCP_TEST_CHILD":"1","ICERS_MCP_TEST_CANCEL":dir.join("cancel")},"approveTools":true})).unwrap()
}

#[tokio::test]
async fn handshake_pagination_call_content_error_timeout_crash_and_cancel() {
    let dir = tempfile::tempdir().unwrap();
    let config = fixture(dir.path());
    let client = Client::spawn("fake", &config, dir.path()).unwrap();
    let tools = client.discover("fake", &config).await.unwrap();
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
    let (tx, ready) = watch::channel(false);
    publish_servers(
        &harness,
        &crate::args::Args::default(),
        dir.path(),
        Arc::new(Mutex::new(HashSet::new())),
        Arc::new(Mutex::new(Vec::new())),
        BTreeMap::from([("fake".into(), config)]),
        tx,
    );
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
