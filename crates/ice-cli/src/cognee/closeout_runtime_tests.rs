// Offline regressions for policy changes between scheduling and dispatch.
#[tokio::test]
async fn queued_improve_rechecks_manual_and_selective_modes_without_losing_capture() {
    for policy in [WritePolicy::Manual, WritePolicy::Selective] {
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        let rt = runtime(dir.path(), &transport, Some(json!({})), false);
        rt.ensure_loaded();
        let session = rt.ensure_session_id("old-session");
        let dataset = rt.active_dataset();
        mark_dream_captured(&rt.dream_dir(), &session, &dataset, NOW as f64).unwrap();
        let before = read_dream_record_for_dataset(&rt.dream_dir(), &session, &dataset).unwrap();
        let lock = rt.dream_writes.lock().await;
        let work = rt.run_improve("policy_test").unwrap();
        tokio::task::yield_now().await;
        let mut config = rt.config();
        config.write_policy = policy;
        assert!(rt.save_and_adopt(config));
        drop(lock);
        work.await.unwrap();
        rt.wait_for_background().await;
        assert!(transport.sent().is_empty());
        let capture = read_dream_record_for_dataset(&rt.dream_dir(), &session, &dataset).unwrap();
        assert_eq!(capture.capture_revision, before.capture_revision);
        assert!(capture.accepted_revision.is_none());
    }
}

#[tokio::test]
async fn queued_capture_cannot_escape_a_manual_policy_change() {
    let dir = tempfile::tempdir().unwrap();
    let transport = Arc::new(FakeTransport::default());
    let rt = runtime(dir.path(), &transport, Some(json!({})), false);
    rt.ensure_loaded();
    rt.ensure_session_id("session");
    // This test uses the current-thread runtime: the queued future cannot run
    // before the effective policy changes unless this task yields.
    rt.store_entry(Map::new(), "answer").unwrap();
    let mut config = rt.config();
    config.write_policy = WritePolicy::Manual;
    assert!(rt.save_and_adopt(config));
    rt.wait_for_background().await;
    assert!(transport.sent().is_empty());
    let observations = super::super::observer::read_observations(&rt.storage_dir);
    assert!(!observations
        .iter()
        .any(|e| e.operation == "remember_entry" && e.phase == "started"));
}

#[tokio::test]
async fn manual_and_selective_warmup_replay_preserve_buffer_without_transport() {
    for policy in ["manual", "selective"] {
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        let rt = runtime(
            dir.path(),
            &transport,
            Some(json!({"writePolicy":policy})),
            false,
        );
        rt.ensure_loaded();
        rt.buffer_warmup_for("original-session", "original-dataset", &Map::new());
        rt.drain_warmup();
        rt.wait_for_background().await;
        assert!(transport.sent().is_empty());
        let buffered = read_warmup_batch(&rt.warmup_dir());
        assert_eq!(buffered.len(), 1);
        assert_eq!(buffered[0].session_id, "original-session");
        assert_eq!(buffered[0].dataset.as_deref(), Some("original-dataset"));
    }
}

#[tokio::test]
async fn capture_dispatch_respects_session_and_trace_switches() {
    for (session_enabled, trace_enabled, expected_calls) in
        [(false, true, 0), (true, false, 1), (true, true, 2)]
    {
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        for _ in 0..expected_calls {
            transport.push(FakeResponse::json(200, json!({})));
        }
        let rt = runtime(
            dir.path(),
            &transport,
            Some(json!({"captureSession":session_enabled,"captureTools":trace_enabled})),
            false,
        );
        rt.ensure_loaded();
        rt.ensure_session_id("session");
        assert_eq!(
            rt.store_entry(Map::new(), "answer").is_some(),
            session_enabled
        );
        assert_eq!(
            rt.store_entry(Map::new(), "trace").is_some(),
            session_enabled && trace_enabled
        );
        rt.wait_for_background().await;
        assert_eq!(transport.sent().len(), expected_calls);
    }
}

#[tokio::test]
async fn disabled_trace_warmup_is_preserved_while_answers_can_replay() {
    let dir = tempfile::tempdir().unwrap();
    let transport = Arc::new(FakeTransport::default());
    transport.push(FakeResponse::json(200, json!({})));
    let rt = runtime(
        dir.path(),
        &transport,
        Some(json!({"captureSession":true,"captureTools":false})),
        false,
    );
    rt.ensure_loaded();
    let trace = json!({"type":"trace","origin_function":"read"})
        .as_object()
        .unwrap()
        .clone();
    rt.buffer_warmup_for("trace-session", "dataset", &trace);
    rt.buffer_warmup_for("answer-session", "dataset", &Map::new());
    assert_eq!(read_warmup_batch(&rt.warmup_dir()).len(), 2);
    rt.drain_warmup();
    rt.wait_for_background().await;
    assert_eq!(transport.sent().len(), 1);
    let buffered = read_warmup_batch(&rt.warmup_dir());
    assert_eq!(buffered.len(), 1);
    assert_eq!(
        buffered[0].entry.get("type").and_then(Value::as_str),
        Some("trace")
    );
}

#[tokio::test]
async fn improve_terminal_observation_keeps_original_dispatch_ownership() {
    let dir = tempfile::tempdir().unwrap();
    let transport = Arc::new(FakeTransport::default());
    transport.push(FakeResponse::Ok {
        status: 200,
        headers: vec![],
        chunks: vec![b"{}".to_vec()],
        delay_ms: 30,
    });
    let rt = runtime(
        dir.path(),
        &transport,
        Some(json!({"dataset":"original-dataset"})),
        false,
    );
    rt.ensure_loaded();
    let session = rt.ensure_session_id("original-session");
    let dataset = rt.active_dataset();
    mark_dream_captured(&rt.dream_dir(), &session, &dataset, NOW as f64).unwrap();
    let handle = rt.run_improve("identity_test").unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while transport.sent().is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    rt.ensure_session_id("new-session");
    rt.state().resolved_dataset = "new-dataset".into();
    handle.await.unwrap();
    rt.wait_for_background().await;
    let observations = super::super::observer::read_observations(&rt.storage_dir);
    let events: Vec<_> = observations
        .iter()
        .filter(|e| e.operation == "improve" && matches!(e.phase.as_str(), "started" | "succeeded"))
        .collect();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].request_id, events[1].request_id);
    for event in events {
        assert_eq!(event.session_id.as_deref(), Some(session.as_str()));
        assert_eq!(event.dataset.as_deref(), Some(dataset.as_str()));
    }
}

#[tokio::test]
async fn mode_change_refreshes_actual_catalog_and_doctor_without_mcp_probe() {
    let dir = tempfile::tempdir().unwrap();
    let transport = Arc::new(FakeTransport::default());
    let rt = runtime(
        dir.path(),
        &transport,
        Some(json!({"writePolicy":"selective"})),
        false,
    );
    let harness = harness_with_messages(vec![]).await;
    rt.bind(
        harness.downgrade(),
        ice_extensions::ExtensionStatusMailbox::new(),
        dir.path().to_path_buf(),
    );
    let service = crate::capability_service::CapabilityService::new(Default::default());
    rt.bind_capability_service(&service);
    rt.register_search_tool(true).await;
    rt.apply_tool_state().await;
    let names: Vec<_> = service
        .snapshot()
        .entries
        .into_iter()
        .map(|entry| entry.name)
        .collect();
    assert!(names.iter().any(|name| name == SEARCH_TOOL_NAME));
    assert!(names.iter().any(|name| name == PROPOSE_TOOL_NAME));
    let mut config = rt.config();
    config.enabled = false;
    assert!(rt.persist_config(config).await);
    assert!(!service
        .snapshot()
        .entries
        .iter()
        .any(|entry| entry.name == SEARCH_TOOL_NAME || entry.name == PROPOSE_TOOL_NAME));
    let report = rt.collect_doctor().await;
    assert_eq!(report.capability_inventory["probed"], false);
    assert_eq!(report.capability_inventory["total"], 0);
    assert!(report.doctor_line().contains("inventory="));
    assert!(transport.sent().is_empty());
}

#[tokio::test]
async fn ambiguous_cache_capture_is_preserved_but_never_replayed_automatically() {
    let dir = tempfile::tempdir().unwrap();
    let transport = Arc::new(FakeTransport::default());
    transport.push(FakeResponse::Fail);
    let rt = runtime(dir.path(), &transport, Some(json!({})), false);
    rt.ensure_loaded();
    rt.ensure_session_id("original");
    rt.store_entry(Map::new(), "answer").unwrap();
    rt.wait_for_background().await;
    assert_eq!(transport.sent().len(), 1);
    let buffered = read_warmup_batch(&rt.warmup_dir());
    assert_eq!(buffered.len(), 1);
    assert_eq!(buffered[0].state, PendingState::Uncertain);
    rt.drain_warmup();
    rt.wait_for_background().await;
    assert_eq!(transport.sent().len(), 1);
    assert_eq!(read_warmup_batch(&rt.warmup_dir()).len(), 1);
}

#[tokio::test]
async fn inflight_warmup_is_not_reactivated_and_does_not_starve_pending_records() {
    let dir = tempfile::tempdir().unwrap();
    let transport = Arc::new(FakeTransport::default());
    let rt = runtime(dir.path(), &transport, Some(json!({})), false);
    rt.ensure_loaded();
    for i in 0..20 {
        crate::cognee::queue::buffer_warmup_entry_with_state(
            &rt.warmup_dir(),
            "original",
            "dataset",
            &Map::new(),
            i,
            PendingState::InFlight,
        );
    }
    rt.buffer_warmup_for("pending", "dataset", &Map::new());
    transport.push(FakeResponse::json(200, json!({})));
    rt.drain_warmup();
    rt.wait_for_background().await;
    assert_eq!(transport.sent().len(), 1);
    assert!(crate::cognee::queue::read_replayable_warmup_batch(&rt.warmup_dir()).is_empty());
    assert!(read_warmup_batch(&rt.warmup_dir())
        .iter()
        .all(|r| r.state == PendingState::InFlight));
}

#[tokio::test]
async fn switching_off_cancels_inflight_optional_capture_and_preserves_uncertainty() {
    let dir = tempfile::tempdir().unwrap();
    let transport = Arc::new(FakeTransport::default());
    transport.push(FakeResponse::Ok {
        status: 200,
        headers: vec![],
        chunks: vec![b"{}".to_vec()],
        delay_ms: 5_000,
    });
    let rt = runtime(dir.path(), &transport, Some(json!({})), false);
    rt.ensure_loaded();
    rt.ensure_session_id("original");
    rt.store_entry(Map::new(), "answer").unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while transport.sent().is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let mut config = rt.config();
    config.enabled = false;
    assert!(rt.save_and_adopt(config));
    tokio::time::timeout(Duration::from_secs(1), rt.wait_for_background())
        .await
        .unwrap();
    assert_eq!(transport.sent().len(), 1);
    let buffered = read_warmup_batch(&rt.warmup_dir());
    assert_eq!(buffered.len(), 1);
    assert_eq!(buffered[0].state, PendingState::Uncertain);
}

#[tokio::test]
async fn manual_accepted_job_handle_is_retained_with_unknown_searchability() {
    let dir = tempfile::tempdir().unwrap();
    let transport = Arc::new(FakeTransport::default());
    transport.push(FakeResponse::json(202, json!({"job_id":"write-job-1"})));
    let rt = runtime(
        dir.path(),
        &transport,
        Some(json!({"writePolicy":"manual","dataset":"original"})),
        false,
    );
    rt.ensure_loaded();
    rt.ensure_session_id("source-session");
    rt.manual_remember("user_context", "retain this explicitly")
        .await;
    rt.wait_for_background().await;
    assert_eq!(transport.sent().len(), 1);
    assert!(read_pending_remember(&rt.pending_dir()).is_empty());
    let files: Vec<_> = std::fs::read_dir(rt.storage_dir.join("accepted-jobs"))
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .collect();
    assert_eq!(files.len(), 1);
    let saved: Value = serde_json::from_slice(&std::fs::read(files[0].path()).unwrap()).unwrap();
    assert_eq!(saved["dataset"], "original");
    assert_eq!(saved["receipt"]["jobId"], "write-job-1");
    assert_eq!(saved["searchability"], "unverified");
}

#[test]
fn legacy_warmup_outcomes_are_readable_but_not_assumed_replayable() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("legacy.json"),
        json!({"sessionId":"legacy","dataset":"d","entry":{}}).to_string(),
    )
    .unwrap();
    let records = crate::cognee::queue::read_warmup_batch(dir.path());
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].state, PendingState::Uncertain);
    assert!(crate::cognee::queue::read_replayable_warmup_batch(dir.path()).is_empty());
}

#[tokio::test]
async fn helper_skips_and_cached_verdicts_do_not_count_as_new_requests() {
    let dir = tempfile::tempdir().unwrap();
    let transport = Arc::new(FakeTransport::default());
    let rt = runtime(dir.path(), &transport, Some(json!({})), false);
    rt.ensure_loaded();
    rt.observe_helper(Some("skip"), "skipped", "rate_limited", 8, 0, 0, None);
    rt.observe_helper(Some("cached"), "judged", "batch_cached", 3, 1, 1, None);
    rt.observe_helper(Some("called"), "abstained", "timeout", 4, 0, 0, Some(1500));
    rt.wait_for_background().await;
    let events = crate::cognee::observer::read_observations(&rt.storage_dir);
    for id in ["skip", "cached"] {
        let meta = events
            .iter()
            .find(|e| e.request_id.as_deref() == Some(id))
            .unwrap()
            .meta
            .as_ref()
            .unwrap();
        assert_eq!(meta["invocationCount"], 0);
        assert_eq!(meta["requested"], 0);
    }
    let meta = events
        .iter()
        .find(|e| e.request_id.as_deref() == Some("called"))
        .unwrap()
        .meta
        .as_ref()
        .unwrap();
    assert_eq!(meta["invocationCount"], 1);
    assert_eq!(meta["requested"], 4);
    assert_eq!(meta["outcomeVersion"], 3);
    assert!(transport.sent().is_empty());
}

#[tokio::test]
async fn helper_late_observation_preserves_source_scope_instead_of_current_session() {
    let dir = tempfile::tempdir().unwrap();
    let transport = Arc::new(FakeTransport::default());
    let rt = runtime(
        dir.path(),
        &transport,
        Some(json!({"dataset":"new-dataset"})),
        false,
    );
    rt.ensure_loaded();
    rt.ensure_session_id("new-session");
    rt.observe_helper_for(
        Some("old-operation"),
        "skipped",
        "cancelled",
        1,
        0,
        0,
        Some(5),
        Some(("old-session", "old-dataset")),
    );
    rt.wait_for_background().await;
    let event = crate::cognee::observer::read_observations(&rt.storage_dir)
        .into_iter()
        .find(|e| e.request_id.as_deref() == Some("old-operation"))
        .unwrap();
    assert_eq!(event.session_id.as_deref(), Some("old-session"));
    assert_eq!(event.dataset.as_deref(), Some("old-dataset"));
    assert_eq!(event.meta.as_ref().unwrap()["invocationCount"], 1);
    assert_eq!(
        event.meta.as_ref().unwrap()["networkRequestCount"],
        Value::Null
    );
}
