#[test]
fn current_replay_readers_skip_ambiguous_records() {
    let root = tempfile::tempdir().unwrap();
    let pending = root.path().join("pending");
    let mut uncertain = super::super::queue::create_pending_remember("uncertain", "ice", "n", 1);
    uncertain.state = super::super::queue::PendingState::Uncertain;
    let mut inflight = super::super::queue::create_pending_remember("inflight", "ice", "n", 2);
    inflight.state = super::super::queue::PendingState::InFlight;
    let queued = super::super::queue::create_pending_remember("pending", "ice", "n", 3);
    for record in [&uncertain, &inflight, &queued] {
        assert!(super::super::queue::enqueue_pending_remember(&pending, record, 64).unwrap());
    }
    super::super::queue::update_pending_remember_state(
        &pending,
        &uncertain.operation_id,
        super::super::queue::PendingState::Uncertain,
    )
    .unwrap();
    let selected = super::super::queue::select_drain_batch(&pending, false);
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].operation_id, queued.operation_id);

    let warmup = root.path().join("warmup");
    let entry = serde_json::json!({"type": "answer", "text": "synthetic"});
    let entry = entry.as_object().unwrap();
    for (at, state) in [
        (1, super::super::queue::PendingState::Uncertain),
        (2, super::super::queue::PendingState::InFlight),
        (3, super::super::queue::PendingState::Pending),
    ] {
        super::super::queue::buffer_warmup_entry_with_state(
            &warmup,
            "synthetic-session",
            "synthetic-dataset",
            entry,
            at,
            state,
        );
    }
    let replayable = super::super::queue::read_replayable_warmup_batch(&warmup);
    assert_eq!(replayable.len(), 1);
    assert_eq!(replayable[0].state, super::super::queue::PendingState::Pending);
}

#[tokio::test]
async fn current_runtime_does_not_dispatch_isolated_ambiguous_records() {
    let dir = tempfile::tempdir().unwrap();
    let transport = Arc::new(FakeTransport::default());
    let mut enabled = super::super::config::CogneeConfig::default();
    enabled.helper_provider = Some("fixture-provider".into());
    enabled.helper_model = Some("fixture-model".into());
    let runtime = runtime(
        dir.path(),
        &transport,
        Some(super::super::config::config_json(&enabled)),
        false,
    );
    let pending = runtime.pending_dir();
    let mut uncertain = super::super::queue::create_pending_remember("uncertain", "ice", "n", 1);
    uncertain.state = super::super::queue::PendingState::Uncertain;
    let mut inflight = super::super::queue::create_pending_remember("inflight", "ice", "n", 2);
    inflight.state = super::super::queue::PendingState::InFlight;
    for record in [&uncertain, &inflight] {
        assert!(super::super::queue::enqueue_pending_remember(&pending, record, 64).unwrap());
        super::super::queue::update_pending_remember_state(&pending, &record.operation_id, record.state).unwrap();
    }
    let warmup = runtime.warmup_dir();
    let entry = serde_json::json!({"type": "answer", "text": "synthetic"});
    let entry = entry.as_object().unwrap();
    for (at, state) in [
        (1, super::super::queue::PendingState::Uncertain),
        (2, super::super::queue::PendingState::InFlight),
    ] {
        super::super::queue::buffer_warmup_entry_with_state(
            &warmup,
            "synthetic-session",
            "synthetic-dataset",
            entry,
            at,
            state,
        );
    }
    assert!(super::super::queue::select_drain_batch(&pending, false).is_empty());
    assert!(super::super::queue::read_replayable_warmup_batch(&warmup).is_empty());
    assert_eq!(runtime.config().write_policy, super::super::config::WritePolicy::Legacy);
    let pending_before = rollback_directory_hash(&pending);
    let warmup_before = rollback_directory_hash(&warmup);
    let history = runtime.storage_dir().join("history/synthetic.jsonl");
    std::fs::create_dir_all(history.parent().unwrap()).unwrap();
    std::fs::write(&history, "{\"role\":\"user\",\"content\":\"synthetic rollback history\"}\n").unwrap();
    let candidate = runtime.storage_dir().join("candidates/synthetic-candidate.json");
    std::fs::create_dir_all(candidate.parent().unwrap()).unwrap();
    std::fs::write(&candidate, "{\"id\":\"synthetic-candidate\",\"state\":\"pending\"}\n").unwrap();
    let receipt = runtime.storage_dir().join("candidates/admissions/synthetic-receipt.json");
    std::fs::create_dir_all(receipt.parent().unwrap()).unwrap();
    std::fs::write(&receipt, "{\"id\":\"synthetic-receipt\",\"searchability\":\"unverified\"}\n").unwrap();
    let accepted = runtime.storage_dir().join("accepted-jobs/synthetic-job.json");
    std::fs::create_dir_all(accepted.parent().unwrap()).unwrap();
    std::fs::write(&accepted, "{\"operationId\":\"synthetic-job\",\"searchability\":\"unverified\"}\n").unwrap();

    runtime.ensure_loaded();
    runtime.drain_pending(false).await;
    runtime.drain_warmup();
    runtime.wait_for_background().await;

    assert!(transport.sent().is_empty(), "unexpected requests: {:?}", transport.sent().iter().map(|request| (&request.method, &request.url, &request.body)).collect::<Vec<_>>());
    assert_eq!(rollback_directory_hash(&pending), pending_before);
    assert_eq!(rollback_directory_hash(&warmup), warmup_before);
    assert!(history.exists());
    assert!(candidate.exists());
    assert!(receipt.exists());
    assert!(accepted.exists());
}

#[test]
fn disabled_current_runtime_preserves_isolated_queue_without_requests() {
    let dir = tempfile::tempdir().unwrap();
    let transport = Arc::new(FakeTransport::default());
    let pending = dir.path().join("ice-cognee/pending");
    let mut uncertain = super::super::queue::create_pending_remember("uncertain", "ice", "n", 1);
    uncertain.state = super::super::queue::PendingState::Uncertain;
    let mut inflight = super::super::queue::create_pending_remember("inflight", "ice", "n", 2);
    inflight.state = super::super::queue::PendingState::InFlight;
    for record in [&uncertain, &inflight] {
        assert!(super::super::queue::enqueue_pending_remember(&pending, record, 64).unwrap());
    }
    let warmup = dir.path().join("ice-cognee/warmup");
    let warmup_entry = serde_json::json!({"type": "answer", "text": "synthetic"});
    let warmup_entry = warmup_entry.as_object().unwrap();
    for (at, state) in [
        (1, super::super::queue::PendingState::Uncertain),
        (2, super::super::queue::PendingState::InFlight),
    ] {
        super::super::queue::buffer_warmup_entry_with_state(
            &warmup,
            "synthetic-session",
            "synthetic-dataset",
            warmup_entry,
            at,
            state,
        );
    }
    let storage = dir.path().join("ice-cognee");
    for (relative, body) in [
        ("candidates/candidate-1.json", r#"{"id":"candidate-1","state":"pending"}"#),
        ("candidates/admissions/receipt-1.json", r#"{"id":"receipt-1","searchability":"unverified"}"#),
        ("accepted-jobs/job-1.json", r#"{"operationId":"job-1","jobId":"synthetic-job","searchability":"unverified"}"#),
        ("observations.jsonl", r#"{"operation":"remember","phase":"failed","error":"synthetic ambiguous result"}"#),
    ] {
        let path = storage.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, format!("{body}\n")).unwrap();
    }

    let storage_dir = dir.path().join("ice-cognee");
    std::fs::create_dir_all(&storage_dir).unwrap();
    std::fs::write(
        storage_dir.join("config.json"),
        r#"{"enabled":false,"writePolicy":"manual"}"#,
    )
    .unwrap();
    let notices = NoticeMailbox::new();
    notices.attach();
    let runtime = CogneeRuntime::new(
        RuntimeOptions {
            agent_dir: dir.path().to_path_buf(),
            storage_dir: Some(storage_dir.clone()),
            process_env: CogneeEnv::new(),
            home_dir: None,
            transport: transport.clone(),
            now: Arc::new(|| NOW),
            offline: false,
            cwd: Some(dir.path().to_path_buf()),
        },
        notices,
    );
    let before = rollback_tree_hash(&storage_dir);
    runtime.ensure_loaded();
    let stored_config = std::fs::read(storage_dir.join("config.json")).unwrap();
    let stored_config: serde_json::Value = serde_json::from_slice(&stored_config).unwrap();
    assert_eq!(stored_config["enabled"], false);
    assert_eq!(stored_config["writePolicy"], "manual");
    assert!(!runtime.config().enabled);
    assert!(!runtime.is_enabled());
    assert_eq!(transport.sent().len(), 0);
    assert_eq!(rollback_tree_hash(&storage_dir), before);
}

fn rollback_directory_hash(root: &std::path::Path) -> String {
    let mut files = std::fs::read_dir(root)
        .unwrap()
        .map(Result::unwrap)
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && path.extension().is_some_and(|extension| extension == "json"))
        .collect::<Vec<_>>();
    files.sort();
    let mut hash = sha2::Sha256::new();
    use sha2::Digest;
    for path in files {
        hash.update(path.file_name().unwrap().to_string_lossy().as_bytes());
        hash.update(std::fs::read(path).unwrap());
    }
    hash.finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn rollback_tree_hash(root: &std::path::Path) -> String {
    fn visit(root: &std::path::Path, path: &std::path::Path, hash: &mut sha2::Sha256) {
        let mut entries = std::fs::read_dir(path)
            .unwrap()
            .map(Result::unwrap)
            .collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            let relative = path.strip_prefix(root).unwrap().to_string_lossy();
            hash.update(relative.as_bytes());
            if path.is_dir() {
                hash.update([b'd']);
                visit(root, &path, hash);
            } else {
                hash.update([b'f']);
                hash.update(std::fs::read(path).unwrap());
            }
        }
    }

    use sha2::Digest;
    let mut hash = sha2::Sha256::new();
    visit(root, root, &mut hash);
    hash.finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
