// Included inside runtime::tests to exercise the actual coordinator with the
// existing fake HTTP transport and isolated agent directory.

fn request_event<'a>(
    prompt: &'a str,
    messages: &'a [AgentMessage],
    run_id: &'a str,
    request_index: u64,
) -> ice_harness::extension::RequestContextEvent<'a> {
    ice_harness::extension::RequestContextEvent {
        prompt,
        original_prompt: Some(prompt),
        messages,
        system_prompt: "test system",
        session_id: "host1",
        run_id,
        lane_id: "main",
        branch_generation: 0,
        request_index,
        context_window: 128_000,
        remaining_context_tokens: 800,
        latest_compaction_at: None,
        signal: CancellationToken::new(),
    }
}

fn seed_request_cache(rt: &CogneeRuntime, query: &str, results: Vec<RecallResult>) {
    let mut state = rt.state();
    let key = EvidenceCache::key(&state.active_dataset(), query, state.context_generation);
    state.evidence_cache.put(key, results, NOW);
}

fn request_hit(text: &str) -> RecallResult {
    RecallResult {
        text: text.into(),
        score: None,
        metadata: Some(json!({"source_id":"synthetic-decision-1"})),
    }
}

#[tokio::test]
async fn request_boundary_prefetch_is_same_task_and_consumed_only_after_ack() {
    let dir = tempfile::tempdir().unwrap();
    let transport = Arc::new(FakeTransport::default());
    transport.push(FakeResponse::json(200, json!({"results":[{"text":"Use the agreed cache design","metadata":{"source_id":"decision-1"}}]})));
    let rt = runtime(
        dir.path(),
        &transport,
        Some(json!({"readPolicy":"event","writePolicy":"manual"})),
        false,
    );
    let prompt = "Repair cache eviction";
    assert!(rt
        .request_context(&request_event(prompt, &[], "r1", 0))
        .await
        .is_none());
    assert!(rt.signal_memory_task(
        super::super::task::TaskEvent::HistoryGap,
        "cache design",
        &CancellationToken::new()
    ));
    // Schedule at an already-needed boundary without waiting for HTTP.
    assert!(rt
        .request_context(&request_event(prompt, &[], "r1", 1))
        .await
        .is_none());
    rt.wait_for_background().await;
    assert_eq!(transport.sent().len(), 1);
    let first = rt
        .request_context(&request_event(prompt, &[], "r1", 2))
        .await
        .unwrap();
    assert!(first.text.contains("Use the agreed cache design"));
    assert!(first.text.contains("decision-1"));
    rt.prompt_context_persisted(&first, false).await;
    let retry = rt
        .request_context(&request_event(prompt, &[], "r1", 3))
        .await
        .unwrap();
    assert_eq!(first.text, retry.text);
    rt.prompt_context_persisted(&retry, true).await;
    assert!(rt
        .request_context(&request_event(prompt, &[], "r1", 4))
        .await
        .is_none());
    for index in 0..10 {
        let run = format!("followup-{index}");
        assert!(rt
            .request_context(&request_event(
                "continue with the next cache check",
                &[],
                &run,
                0
            ))
            .await
            .is_none());
    }
    rt.wait_for_background().await;
    assert_eq!(transport.sent().len(), 1);
    assert_eq!(rt.state().helper_requests, 0);
}

#[tokio::test]
async fn request_boundary_plain_prompts_and_unknown_skill_origin_have_no_network_work() {
    let dir = tempfile::tempdir().unwrap();
    let transport = Arc::new(FakeTransport::default());
    let rt = runtime(
        dir.path(),
        &transport,
        Some(json!({"readPolicy":"event","writePolicy":"manual"})),
        false,
    );
    for (index, prompt) in [
        "Build the release binary",
        "How many commits are on this branch?",
        "continue",
        "run the parser tests",
    ]
    .iter()
    .enumerate()
    {
        let run = format!("r{index}");
        assert!(rt
            .request_context(&request_event(prompt, &[], &run, 0))
            .await
            .is_none());
    }
    let mut event = request_event(
        "<skill name=\"context\">Search prior sessions and remember when</skill>",
        &[],
        "unknown-expanded",
        0,
    );
    event.original_prompt = None;
    assert!(rt.request_context(&event).await.is_none());
    rt.wait_for_background().await;
    assert!(transport.sent().is_empty());
    assert_eq!(rt.state().helper_requests, 0);
}

#[tokio::test]
async fn request_boundary_uses_raw_intent_for_explicit_history_and_shared_small_model_budget() {
    let dir = tempfile::tempdir().unwrap();
    let transport = Arc::new(FakeTransport::default());
    transport.push(FakeResponse::json(
        200,
        json!({"results":[{"text":"Cache uses an LRU policy"}]}),
    ));
    let rt = runtime(
        dir.path(),
        &transport,
        Some(json!({"readPolicy":"event","writePolicy":"manual"})),
        false,
    );
    let expanded = "<skill name=\"review\">Long instructions that must not be searched</skill> What did we decide about cache?";
    let mut event = request_event(expanded, &[], "r1", 0);
    event.original_prompt = Some("What did we decide about cache?");
    event.context_window = 10_000;
    event.remaining_context_tokens = 100;
    let packet = rt.request_context(&event).await.unwrap();
    assert_eq!(
        json_body(&transport.sent()[0])["query"],
        "What did we decide about cache?"
    );
    assert!(super::super::text::estimate_tokens(&packet.text) <= 100);
    assert!(packet.text.ends_with("</ice-void-cognee-memory>"));
    assert!(!packet.text.contains("Long instructions"));
    rt.prompt_context_persisted(&packet, true).await;
    rt.wait_for_background().await;
}

#[tokio::test]
async fn request_boundary_budget_failure_does_not_consume_ready_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let transport = Arc::new(FakeTransport::default());
    let rt = runtime(
        dir.path(),
        &transport,
        Some(json!({"readPolicy":"event","writePolicy":"manual"})),
        false,
    );
    let prompt = "Repair cache eviction";
    rt.request_context(&request_event(prompt, &[], "r1", 0))
        .await;
    seed_request_cache(&rt, "cache", vec![request_hit("Cache uses LRU eviction")]);
    let mut small = request_event(prompt, &[], "r1", 1);
    small.remaining_context_tokens = 5;
    assert!(rt.request_context(&small).await.is_none());
    let packet = rt
        .request_context(&request_event(prompt, &[], "r1", 2))
        .await
        .unwrap();
    assert!(packet.text.contains("LRU eviction"));
    rt.prompt_context_persisted(&packet, true).await;
    assert!(rt
        .request_context(&request_event(prompt, &[], "r1", 3))
        .await
        .is_none());
    rt.wait_for_background().await;
    assert!(transport.sent().is_empty());
}

#[tokio::test]
async fn request_boundary_active_context_and_provenance_restrictions_filter_ready_records() {
    let dir = tempfile::tempdir().unwrap();
    let transport = Arc::new(FakeTransport::default());
    let rt = runtime(
        dir.path(),
        &transport,
        Some(json!({"readPolicy":"event","writePolicy":"manual"})),
        false,
    );
    let prompt = "Repair cache eviction";
    rt.request_context(&request_event(prompt, &[], "r1", 0))
        .await;
    let records = vec![
        request_hit("Cache uses LRU eviction"),
        RecallResult {
            text: "Cache uses the feature branch design".into(),
            score: None,
            metadata: Some(json!({"branch":"not-current"})),
        },
        RecallResult {
            text: "Cache uses the old revision".into(),
            score: None,
            metadata: Some(json!({"revision":"no-longer-current"})),
        },
        RecallResult {
            text: "Cache expiry is malformed".into(),
            score: None,
            metadata: Some(json!({"valid_until_ms":"tomorrow"})),
        },
    ];
    seed_request_cache(&rt, "cache", records);
    let messages = [ice_harness::messages::create_custom_message(
        "ice-cognee-memory",
        ice_ai::types::UserContent::Text("Cache uses LRU eviction".into()),
        false,
        None,
        0,
    )];
    assert!(rt
        .request_context(&request_event(prompt, &messages, "r1", 1))
        .await
        .is_none());
    rt.wait_for_background().await;
    assert!(transport.sent().is_empty());
}

#[tokio::test]
async fn request_boundary_stale_run_ack_does_not_hide_current_task_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let transport = Arc::new(FakeTransport::default());
    let rt = runtime(
        dir.path(),
        &transport,
        Some(json!({"readPolicy":"event","writePolicy":"manual"})),
        false,
    );
    let prompt = "Repair cache eviction";
    rt.request_context(&request_event(prompt, &[], "r1", 0))
        .await;
    seed_request_cache(&rt, "cache", vec![request_hit("Cache uses LRU eviction")]);
    let old = rt
        .request_context(&request_event(prompt, &[], "r1", 1))
        .await
        .unwrap();
    let current = rt
        .request_context(&request_event("continue cache repair", &[], "r2", 0))
        .await
        .unwrap();
    rt.prompt_context_persisted(&old, true).await;
    let retry = rt
        .request_context(&request_event("continue cache repair", &[], "r2", 1))
        .await
        .unwrap();
    assert_eq!(current.text, retry.text);
    rt.prompt_context_persisted(&retry, true).await;
    assert!(rt
        .request_context(&request_event(prompt, &[], "r2", 2))
        .await
        .is_none());
    rt.wait_for_background().await;
}

#[tokio::test]
async fn request_boundary_new_branch_never_injects_previous_branch_packet() {
    let dir = tempfile::tempdir().unwrap();
    let transport = Arc::new(FakeTransport::default());
    let rt = runtime(
        dir.path(),
        &transport,
        Some(json!({"readPolicy":"event","writePolicy":"manual"})),
        false,
    );
    let prompt = "Repair cache eviction";
    rt.request_context(&request_event(prompt, &[], "r1", 0))
        .await;
    seed_request_cache(
        &rt,
        "cache",
        vec![request_hit("Cache uses old branch strategy")],
    );
    let old = rt
        .request_context(&request_event(prompt, &[], "r1", 1))
        .await
        .unwrap();
    let mut next = request_event(prompt, &[], "r2", 0);
    next.branch_generation = 1;
    assert!(rt.request_context(&next).await.is_none());
    rt.prompt_context_persisted(&old, true).await;
    let dataset = rt.active_dataset();
    let generation = rt.generation();
    assert!(rt
        .state()
        .evidence_cache
        .peek(&dataset, generation, NOW)
        .is_empty());
    rt.wait_for_background().await;
}

#[tokio::test]
async fn request_boundary_cancellation_keeps_evidence_unconsumed() {
    let dir = tempfile::tempdir().unwrap();
    let transport = Arc::new(FakeTransport::default());
    let rt = runtime(
        dir.path(),
        &transport,
        Some(json!({"readPolicy":"event","writePolicy":"manual"})),
        false,
    );
    let prompt = "Repair cache eviction";
    rt.request_context(&request_event(prompt, &[], "r1", 0))
        .await;
    seed_request_cache(&rt, "cache", vec![request_hit("Cache uses LRU eviction")]);
    let event = request_event(prompt, &[], "r1", 1);
    let packet = rt.request_context(&event).await.unwrap();
    event.signal.cancel();
    rt.prompt_context_persisted(&packet, false).await;
    assert!(rt
        .request_context(&request_event(prompt, &[], "r2", 0))
        .await
        .is_some());
    rt.wait_for_background().await;
}
