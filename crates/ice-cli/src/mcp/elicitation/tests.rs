use super::*;
#[path = "../../../tests/support/mcp_fixture.rs"]
pub(super) mod mcp_fixture;

fn state() -> Arc<Elicitation> {
    let ui = UiDialogMailbox::new();
    ui.attach();
    Arc::new(Elicitation {
        server: "fake".into(),
        ui,
        url: url::UrlElicitation::new(tokio::sync::broadcast::channel(8).0),
    })
}
pub(crate) async fn next(ui: &UiDialogMailbox) -> ice_extensions::UiDialogRequest {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Some(request) = ui.take_pending() {
                return request;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap()
}
async fn answer(ui: &UiDialogMailbox, value: Value) -> ice_extensions::UiDialogRequest {
    let request = next(ui).await;
    ui.respond(&request.request_id, value).unwrap();
    request
}
fn form() -> Value {
    json!({"message":"Input?","requestedSchema":{"type":"object","properties":{
        "age":{"type":"integer","minimum":1,"maximum":10},
        "choice":{"type":"string","enum":["a","b"],"enumNames":["Same\u{001b}","Same"]},
        "tags":{"type":"array","items":{"type":"string","enum":["x","y"]}}
    },"required":["age","choice"]}})
}
#[tokio::test]
async fn form_validates_reasks_edits_and_submits_raw_values() {
    let state = state();
    let child = state.clone();
    let task = tokio::spawn(async move {
        child
            .handle(form(), &CancellationToken::new())
            .await
            .unwrap()
    });
    answer(&state.ui, json!({"values":["Continue"]})).await;
    answer(&state.ui, json!({"values":["Enter value"]})).await;
    answer(&state.ui, json!({"text":"0"})).await;
    let retry = answer(&state.ui, json!({"values":["Enter value"]})).await;
    assert!(retry.ui["question"]
        .as_str()
        .unwrap()
        .contains("Invalid value"));
    answer(&state.ui, json!({"text":"3"})).await;
    answer(&state.ui, json!({"values":["Enter value"]})).await;
    let choice = answer(&state.ui, json!({"values":["2. Same"]})).await;
    assert_eq!(choice.ui["options"][0]["title"], "1. Same");
    answer(&state.ui, json!({"values":["Enter value"]})).await;
    let multi = answer(&state.ui, json!({"values":["1. x","2. y"]})).await;
    assert_eq!(multi.ui["allowMultiple"], true);
    answer(&state.ui, json!({"values":["Edit"]})).await;
    answer(&state.ui, json!({"values":["1. age"]})).await;
    answer(&state.ui, json!({"values":["Enter value"]})).await;
    answer(&state.ui, json!({"text":"4"})).await;
    answer(&state.ui, json!({"values":["Submit"]})).await;
    assert_eq!(
        task.await.unwrap(),
        json!({"action":"accept","content":{"age":4,"choice":"b","tags":["x","y"]}})
    );
}
#[tokio::test]
async fn declines_cancels_and_omits_without_fabricating_content() {
    for choice in ["Decline", "Cancel"] {
        let state = state();
        let child = state.clone();
        let task = tokio::spawn(async move {
            child
                .handle(form(), &CancellationToken::new())
                .await
                .unwrap()
        });
        answer(&state.ui, json!({"values":[choice]})).await;
        assert_eq!(task.await.unwrap()["action"], choice.to_lowercase());
    }
    let state = state();
    let child = state.clone();
    let task = tokio::spawn(async move {
        child.handle(json!({"message":"Input","requestedSchema":{"type":"object","properties":{"optional":{"type":"boolean"},"value":{"type":"integer","default":2}},"required":["value"]}}),&CancellationToken::new()).await.unwrap()
    });
    for choice in ["Continue", "Omit", "Use default", "Submit"] {
        answer(&state.ui, json!({"values":[choice]})).await;
    }
    assert_eq!(
        task.await.unwrap(),
        json!({"action":"accept","content":{"value":2}})
    );
}
#[tokio::test]
async fn dialog_is_reclaimed_on_cancellation_and_future_drop() {
    let state = state();
    for drop_future in [false, true] {
        let child = state.clone();
        let signal = CancellationToken::new();
        let token = signal.clone();
        let task = tokio::spawn(async move { child.handle(form(), &token).await });
        let request = next(&state.ui).await;
        if drop_future {
            task.abort();
            let _ = task.await;
        } else {
            signal.cancel();
            assert_eq!(task.await.unwrap().unwrap()["action"], "cancel");
        }
        assert!(!state.ui.is_pending(&request.request_id));
        assert_eq!(state.ui.pending_len(), 0);
    }
}
#[tokio::test]
async fn capability_supports_both_modes_and_invalid_requests_never_open_dialogs() {
    let state = state();
    let handlers = register(
        Handlers::default(),
        "fake",
        state.ui.clone(),
        tokio::sync::broadcast::channel(8).0,
    );
    assert_eq!(
        handlers.capabilities(),
        json!({"elicitation":{"form":{},"url":{}}})
    );
    assert!(state
        .handle(
            json!({"mode":"url","message":"open?"}),
            &CancellationToken::new()
        )
        .await
        .is_err());
    assert!(state.handle(json!({"message":"bad","requestedSchema":{"type":"object","properties":{"x":{"type":"object"}}}}),&CancellationToken::new()).await.is_err());
    assert_eq!(state.ui.pending_len(), 0);
    state.ui.detach();
    assert_eq!(
        state
            .handle(form(), &CancellationToken::new())
            .await
            .unwrap()["action"],
        "cancel"
    );
}
#[tokio::test]
async fn transport_tool_abort_reclaims_uncooperative_server_dialog() {
    let dir = tempfile::tempdir().unwrap();
    let binary = mcp_fixture::server_executable();
    let config: ServerConfig =
        serde_json::from_value(json!({"command":binary,"env":{"ICERS_MCP_TEST_CHILD":"1"}}))
            .unwrap();
    let ui = UiDialogMailbox::new();
    ui.attach();
    let handlers = register(
        Handlers::default(),
        "fake",
        ui.clone(),
        tokio::sync::broadcast::channel(8).0,
    );
    let client = Client::spawn("fake", &config, dir.path(), handlers).unwrap();
    let tools = client
        .discover("fake", &config, &OutputGuard::default())
        .await
        .unwrap();
    let tool = tools
        .into_iter()
        .find(|tool| tool.remote_name == "echo")
        .unwrap();
    let token = CancellationToken::new();
    let signal = token.clone();
    let task = tokio::spawn(async move {
        tool.execute("one",json!({"mode":"elicit_hang","request":{"message":"Input?","requestedSchema":{"type":"object","properties":{}}}}),signal,Arc::new(|_|{})).await
    });
    let request = next(&ui).await;
    token.cancel();
    assert!(task.await.unwrap().is_err());
    tokio::time::timeout(Duration::from_secs(3), async {
        while ui.is_pending(&request.request_id) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(ui.pending_len(), 0);
}

#[tokio::test]
async fn transport_pre_cancelled_call_preserves_another_calls_form() {
    let dir = tempfile::tempdir().unwrap();
    let config: ServerConfig = serde_json::from_value(
        json!({"command":mcp_fixture::server_executable(),"env":{"ICERS_MCP_TEST_CHILD":"1"}}),
    )
    .unwrap();
    let state = state();
    let client = Client::spawn(
        "fake",
        &config,
        dir.path(),
        register(
            Handlers::default(),
            "fake",
            state.ui.clone(),
            tokio::sync::broadcast::channel(8).0,
        ),
    )
    .unwrap();
    client
        .discover("fake", &config, &OutputGuard::default())
        .await
        .unwrap();
    let c = client.clone();
    let task = tokio::spawn(async move {
        c.request("tools/call",json!({"name":"echo","arguments":{"mode":"elicit","request":{"message":"Keep this form","requestedSchema":{"type":"object","properties":{}}}}}),CancellationToken::new(),Duration::from_secs(3)).await
    });
    let request = next(&state.ui).await;
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert_eq!(
        client
            .request(
                "tools/call",
                json!({"name":"echo"}),
                cancelled,
                CALL_TIMEOUT
            )
            .await
            .unwrap_err(),
        "MCP call cancelled"
    );
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(state.ui.is_pending(&request.request_id));
    state
        .ui
        .respond(&request.request_id, json!({"values":["Continue"]}))
        .unwrap();
    answer(&state.ui, json!({"values":["Submit"]})).await;
    let result = task.await.unwrap().unwrap();
    let response: Value =
        serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(response["result"], json!({"action":"accept","content":{}}));
}

#[tokio::test]
async fn manager_does_not_advertise_without_tui_or_when_disabled() {
    let dir = tempfile::tempdir().unwrap();
    let harness = AgentHarness::create(Default::default()).await.unwrap();
    for (host, enabled, expected) in [
        (false, true, false),
        (true, false, false),
        (true, true, true),
    ] {
        std::fs::write(
            dir.path().join("mcp.json"),
            json!({"settings":{"elicitation":enabled}}).to_string(),
        )
        .unwrap();
        let manager = McpManager::new(
            &harness,
            &Default::default(),
            dir.path(),
            dir.path().into(),
            false,
            Default::default(),
        );
        let manager = if host {
            manager.with_elicitation_ui(UiDialogMailbox::new())
        } else {
            manager
        };
        assert_eq!(
            manager
                .handlers("fake")
                .capabilities()
                .get("elicitation")
                .is_some(),
            expected
        );
    }
}
#[tokio::test]
async fn transport_returns_form_response_and_handles_server_cancellation() {
    let dir = tempfile::tempdir().unwrap();
    let config: ServerConfig = serde_json::from_value(
        json!({"command":mcp_fixture::server_executable(),"env":{"ICERS_MCP_TEST_CHILD":"1"}}),
    )
    .unwrap();
    let state = state();
    let client = Client::spawn(
        "fake",
        &config,
        dir.path(),
        register(
            Handlers::default(),
            "fake",
            state.ui.clone(),
            tokio::sync::broadcast::channel(8).0,
        ),
    )
    .unwrap();
    client
        .discover("fake", &config, &OutputGuard::default())
        .await
        .unwrap();
    let c = client.clone();
    let task = tokio::spawn(async move {
        c.request("tools/call",json!({"name":"echo","arguments":{"mode":"elicit","request":{"message":"Empty form","requestedSchema":{"type":"object","properties":{}}}}}),CancellationToken::new(),CALL_TIMEOUT).await.unwrap()
    });
    for choice in ["Continue", "Submit"] {
        answer(&state.ui, json!({"values":[choice]})).await;
    }
    let result = task.await.unwrap();
    let response: Value =
        serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(response["result"], json!({"action":"accept","content":{}}));
    let c = client.clone();
    let task = tokio::spawn(async move {
        c.request("tools/call",json!({"name":"echo","arguments":{"mode":"elicit_cancel","request":{"message":"Cancel","requestedSchema":{"type":"object","properties":{}}}}}),CancellationToken::new(),Duration::from_millis(150)).await
    });
    assert!(task.await.unwrap().is_err());
    assert_eq!(state.ui.pending_len(), 0);
}
