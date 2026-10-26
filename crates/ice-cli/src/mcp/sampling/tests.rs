use super::*;
use ice_ai::providers::faux::{faux_assistant_message, FauxProvider, FauxScript, FauxStep};
use std::sync::atomic::Ordering;

fn setup(auto_approve: bool, steps: Vec<FauxStep>) -> (Arc<Sampling>, Arc<FauxProvider>) {
    let script = FauxScript::new();
    script.set_responses(steps);
    let faux = FauxProvider::new(script);
    let current = faux.default_model().clone();
    let ui = UiDialogMailbox::new();
    ui.attach();
    let state = Arc::new(Sampling {
        server: "fake".into(),
        ui,
        providers: Arc::new(RwLock::new(vec![faux.clone() as Arc<dyn Provider>])),
        current_model: Arc::new(move || {
            let current = current.clone();
            Box::pin(async move { Some(current) })
        }),
        auto_approve,
    });
    (state, faux)
}
fn calls(faux: &FauxProvider) -> usize {
    faux.state().call_count.load(Ordering::SeqCst)
}
fn request() -> Value {
    json!({
        "messages":[
            {"role":"user","content":{"type":"text","text":"Hello"}},
            {"role":"assistant","content":{"type":"text","text":"Hi"}},
            {"role":"user","content":[{"type":"text","text":"Again\u{001b}[31m"}]}
        ],
        "systemPrompt":"Be brief",
        "maxTokens":64
    })
}
async fn next(ui: &UiDialogMailbox) -> ice_extensions::UiDialogRequest {
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
async fn answer(ui: &UiDialogMailbox, choice: &str) -> ice_extensions::UiDialogRequest {
    let request = next(ui).await;
    ui.respond(&request.request_id, json!({"values":[choice]}))
        .unwrap();
    request
}

#[test]
fn rejects_unsupported_inputs_with_explicit_errors() {
    let base = request();
    for (key, value, expected) in [
        ("includeContext", json!("thisServer"), "includeContext"),
        ("includeContext", json!("allServers"), "includeContext"),
        ("tools", json!([{"name":"t"}]), "Tools"),
        ("toolChoice", json!({"mode":"auto"}), "Tools"),
        ("stopSequences", json!(["END"]), "stopSequences"),
        ("maxTokens", json!(0), "maxTokens"),
        ("maxTokens", json!("64"), "maxTokens"),
        ("messages", json!([]), "messages"),
        (
            "messages",
            json!([{"role":"user","content":{"type":"image","data":"x","mimeType":"image/png"}}]),
            "text",
        ),
        (
            "messages",
            json!([{"role":"user","content":{"type":"audio","data":"x","mimeType":"audio/wav"}}]),
            "text",
        ),
        (
            "messages",
            json!([{"role":"system","content":{"type":"text","text":"x"}}]),
            "role",
        ),
    ] {
        let mut params = base.clone();
        params[key] = value;
        let error = parse(&params)
            .err()
            .unwrap_or_else(|| panic!("{key} accepted"));
        assert_eq!(error.code, -32602);
        assert!(error.message.contains(expected), "{key}: {}", error.message);
    }
    let mut missing = base.clone();
    missing.as_object_mut().unwrap().remove("maxTokens");
    assert!(parse(&missing).is_err());
    // `none` and an explicit null are the spec's "no context" spellings.
    for accepted in [json!("none"), Value::Null] {
        let mut params = base.clone();
        params["includeContext"] = accepted;
        params["stopSequences"] = json!([]);
        assert!(parse(&params).is_ok());
    }
}

#[test]
fn parse_collects_messages_hints_and_options() {
    let mut params = request();
    params["modelPreferences"] = json!({"hints":[{"name":"Large"},{"name":"small"},{}]});
    params["temperature"] = json!(0.25);
    let parsed = parse(&params).unwrap();
    assert_eq!(parsed.max_tokens, 64);
    assert_eq!(parsed.system.as_deref(), Some("Be brief"));
    assert_eq!(parsed.hints, ["large", "small"]);
    assert_eq!(parsed.temperature, Some(0.25));
    assert_eq!(
        parsed.messages.iter().map(|m| m.0).collect::<Vec<_>>(),
        [false, true, false]
    );
}

#[test]
fn hint_wins_then_current_model_then_nothing() {
    let (_, faux) = setup(true, vec![]);
    let mut large = faux.default_model().clone();
    large.id = "faux-LARGE-2".into();
    let provider = FauxProvider::with_models(
        FauxScript::new(),
        vec![faux.default_model().clone(), large.clone()],
    ) as Arc<dyn Provider>;
    let providers = [provider];
    let current = Some(faux.default_model().clone());

    let hinted = |hints: &[&str]| {
        let hints = hints.iter().map(|h| h.to_string()).collect::<Vec<_>>();
        resolve(&providers, &hints, current.clone()).map(|(_, model)| model.id)
    };
    assert_eq!(hinted(&["large"]).as_deref(), Some("faux-LARGE-2"));
    // Hints are tried in order; an unmatched one falls through to the next.
    assert_eq!(hinted(&["nope", "large"]).as_deref(), Some("faux-LARGE-2"));
    assert_eq!(
        hinted(&["nope"]).as_deref(),
        Some(faux.default_model().id.as_str())
    );
    assert_eq!(
        hinted(&[]).as_deref(),
        Some(faux.default_model().id.as_str())
    );
    assert!(resolve(&providers, &[], None).is_none());

    let mut foreign = faux.default_model().clone();
    foreign.provider = "elsewhere".into();
    assert!(resolve(&providers, &[], Some(foreign)).is_none());
}

#[tokio::test]
async fn auto_approve_returns_text_and_maps_end_turn() {
    let (state, faux) = setup(true, vec![FauxStep::text("Reply")]);
    let result = state
        .handle(request(), &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(result["role"], "assistant");
    assert_eq!(result["content"], json!({"type":"text","text":"Reply"}));
    assert_eq!(result["stopReason"], "endTurn");
    assert_eq!(calls(&faux), 1);
    // Auto-approve never opens a dialog.
    assert!(state.ui.take_pending().is_none());
}

#[tokio::test]
async fn length_maps_to_max_tokens_and_errors_are_not_returned_as_text() {
    let (state, _) = setup(
        true,
        vec![FauxStep::message(faux_assistant_message(
            "Cut",
            ice_ai::types::StopReason::Length,
        ))],
    );
    let result = state
        .handle(request(), &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(result["stopReason"], "maxTokens");

    let (state, _) = setup(
        true,
        vec![FauxStep::message(faux_assistant_message(
            "Partial",
            ice_ai::types::StopReason::Error,
        ))],
    );
    let error = state
        .handle(request(), &CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(error.code, -32603);
}

#[tokio::test]
async fn denied_request_never_reaches_the_provider() {
    let (state, faux) = setup(false, vec![FauxStep::text("Reply")]);
    let child = state.clone();
    let task = tokio::spawn(async move {
        child
            .handle(request(), &CancellationToken::new())
            .await
            .unwrap_err()
    });
    let asked = answer(&state.ui, "Deny").await;
    let summary = asked.ui["summary"].as_str().unwrap();
    assert!(summary.contains("fake"), "{summary}");
    assert!(summary.contains("Hello"), "{summary}");
    // Untrusted request text has its control characters stripped.
    assert!(!summary.contains('\u{001b}'), "{summary:?}");
    let error = task.await.unwrap();
    assert_eq!(error.code, -1);
    assert_eq!(calls(&faux), 0);
}

#[tokio::test]
async fn approved_request_and_response_round_trips() {
    let (state, faux) = setup(false, vec![FauxStep::text("Reply")]);
    let child = state.clone();
    let task = tokio::spawn(async move {
        child
            .handle(request(), &CancellationToken::new())
            .await
            .unwrap()
    });
    answer(&state.ui, "Allow").await;
    let review = answer(&state.ui, "Allow").await;
    assert!(review.ui["summary"].as_str().unwrap().contains("Reply"));
    assert_eq!(task.await.unwrap()["content"]["text"], "Reply");
    assert_eq!(calls(&faux), 1);
}

#[tokio::test]
async fn rejected_response_is_withheld_from_the_server() {
    let (state, faux) = setup(false, vec![FauxStep::text("Secret")]);
    let child = state.clone();
    let task = tokio::spawn(async move {
        child
            .handle(request(), &CancellationToken::new())
            .await
            .unwrap_err()
    });
    answer(&state.ui, "Allow").await;
    answer(&state.ui, "Deny").await;
    let error = task.await.unwrap();
    assert_eq!(error.code, -1);
    assert!(!error.message.contains("Secret"));
    assert_eq!(calls(&faux), 1);
}

#[tokio::test]
async fn cancelling_while_awaiting_approval_denies_without_a_call() {
    let (state, faux) = setup(false, vec![FauxStep::text("Reply")]);
    let signal = CancellationToken::new();
    let child = state.clone();
    let token = signal.clone();
    let task = tokio::spawn(async move { child.handle(request(), &token).await.unwrap_err() });
    next(&state.ui).await;
    signal.cancel();
    assert_eq!(task.await.unwrap().code, -1);
    assert_eq!(calls(&faux), 0);
}

#[test]
fn only_registered_when_a_ui_host_installs_it() {
    let (state, _) = setup(false, vec![]);
    let handlers = register(
        Handlers::default(),
        "fake",
        state.ui.clone(),
        state.providers.clone(),
        state.current_model.clone(),
        false,
    );
    assert_eq!(handlers.capabilities()["sampling"], json!({}));
    assert!(Handlers::default().capabilities().get("sampling").is_none());
}

#[tokio::test]
async fn manager_advertises_sampling_only_for_a_tui_host_with_the_setting_on() {
    use ice_harness::agent_harness::AgentHarness;
    let dir = tempfile::tempdir().unwrap();
    let harness = AgentHarness::create(Default::default()).await.unwrap();
    let providers: Providers = Default::default();
    // (settings, dialog host, sampling providers, expected)
    for (settings, ui, sampling, expected) in [
        (json!({}), false, true, false),
        (json!({}), true, false, false),
        (json!({"sampling": false}), true, true, false),
        (json!({}), true, true, true),
        (
            json!({"sampling": true, "samplingAutoApprove": true}),
            true,
            true,
            true,
        ),
    ] {
        std::fs::write(
            dir.path().join("mcp.json"),
            json!({ "settings": settings }).to_string(),
        )
        .unwrap();
        let mut manager = McpManager::new(
            &harness,
            &Default::default(),
            dir.path(),
            dir.path().into(),
            false,
            Default::default(),
        );
        if ui {
            manager = manager.with_elicitation_ui(UiDialogMailbox::new());
        }
        if sampling {
            manager = manager.with_sampling(providers.clone());
        }
        assert_eq!(
            manager
                .handlers("fake")
                .capabilities()
                .get("sampling")
                .is_some(),
            expected,
            "{settings}"
        );
    }
}
