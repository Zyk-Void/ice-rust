/// Canary and rollback against a real, isolated Cognee service: queued intents,
/// ambiguous records and explicit writes, observed on the service itself through
/// inference-free dataset reads. Ignored by default; it needs
/// `ICERS_LIVE_COGNEE_URL` (loopback, never the default 8211 port) and
/// `ICERS_LIVE_COGNEE_KEY_FILE` for a throwaway instance.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs an isolated live Cognee service"]
async fn live_isolated_canary_and_rollback() {
    use crate::cognee::config::{config_json, CogneeConfig};
    use crate::cognee::queue::{
        create_pending_remember, enqueue_pending_remember, update_pending_remember_state,
        PendingState,
    };
    let url = std::env::var("ICERS_LIVE_COGNEE_URL").expect("ICERS_LIVE_COGNEE_URL");
    let parsed = reqwest::Url::parse(&url).unwrap();
    assert_eq!(parsed.host_str(), Some("127.0.0.1"), "loopback only");
    assert_ne!(parsed.port(), Some(8211), "never the live service port");
    let key_file = std::env::var("ICERS_LIVE_COGNEE_KEY_FILE").expect("ICERS_LIVE_COGNEE_KEY_FILE");
    let key = std::fs::read_to_string(key_file).unwrap().trim().to_owned();
    let run = wall_ms();
    let dataset = format!("iso_canary_{run}");

    let dir = tempfile::tempdir().unwrap();
    let mut config = CogneeConfig {
        enabled: false,
        auto_recall: false,
        capture_session: false,
        capture_tools: false,
        auto_improve: false,
        helper_enabled: false,
        base_url: url.clone(),
        dataset: dataset.clone(),
        ..CogneeConfig::default()
    };
    let path = dir.path().join("ice-cognee").join("config.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, config_json(&config).to_string()).unwrap();
    let mut env = CogneeEnv::new();
    env.insert("COGNEE_API_KEY".into(), key.clone());
    let notices = NoticeMailbox::new();
    notices.attach();
    let runtime = CogneeRuntime::new(
        RuntimeOptions {
            agent_dir: dir.path().to_path_buf(),
            storage_dir: None,
            process_env: env,
            home_dir: None,
            transport: Arc::new(crate::cognee::client::ReqwestTransport::new()),
            now: Arc::new(wall_ms),
            offline: false,
            cwd: Some(dir.path().to_path_buf()),
        },
        notices,
    );
    runtime.ensure_loaded();
    let loaded = runtime.config();
    // Never fall through to a default (live) base URL.
    assert_eq!(loaded.base_url, url, "runtime did not adopt the isolated URL");
    assert!(
        !runtime.is_enabled(),
        "stored disabled config was not adopted: enabled={} base_url={} dataset={}",
        loaded.enabled,
        loaded.base_url,
        loaded.dataset
    );

    let http = reqwest::Client::new();
    let texts = || async {
        let datasets: Vec<Value> = http
            .get(format!("{url}/api/v1/datasets"))
            .header("x-api-key", &key)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let Some(id) = datasets
            .iter()
            .find(|d| d["name"] == dataset.as_str())
            .map(|d| d["id"].as_str().unwrap().to_owned())
        else {
            return Vec::new();
        };
        let items: Vec<Value> = http
            .get(format!("{url}/api/v1/datasets/{id}/data"))
            .header("x-api-key", &key)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let mut texts = Vec::new();
        for item in items {
            let raw = http
                .get(format!("{url}/api/v1/datasets/{id}/data/{}/raw", item["id"].as_str().unwrap()))
                .header("x-api-key", &key)
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap();
            texts.push(raw);
        }
        texts.sort();
        texts
    };
    let marker = |name: &str| format!("ISO-CANARY-{run}-{name}: synthetic fact {name}.");
    let pending = runtime.pending_dir();
    let seed = |name: &str, state: PendingState, at: u64| {
        let mut record = create_pending_remember(&marker(name), &dataset, "canary", at);
        record.state = state;
        assert!(enqueue_pending_remember(&pending, &record, 64).unwrap());
        update_pending_remember_state(&pending, &record.operation_id, state).unwrap();
    };
    seed("pending", PendingState::Pending, 1);
    seed("uncertain", PendingState::Uncertain, 2);
    seed("inflight", PendingState::InFlight, 3);

    // 1. Disabled: nothing reaches the service; the queue is untouched.
    let before = queue_snapshot(&pending);
    runtime.drain_pending(false).await;
    runtime.wait_for_background().await;
    assert_eq!(queue_snapshot(&pending), before);
    assert!(texts().await.is_empty(), "disabled runtime wrote to the service");
    eprintln!("canary step 1 (disabled): 0 service items, queue hash unchanged");

    // 2. Canary on: only the unambiguous pending intent is delivered.
    config.enabled = true;
    assert!(runtime.save_and_adopt(config.clone()));
    runtime.drain_pending(false).await;
    runtime.wait_for_background().await;
    // Accepted writes complete in the background on the service.
    let mut delivered = texts().await;
    for _ in 0..90 {
        if !delivered.is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        delivered = texts().await;
    }
    assert_eq!(delivered.len(), 1, "{delivered:?}");
    assert!(delivered[0].contains(&marker("pending")), "{delivered:?}");
    let remaining = crate::cognee::queue::select_drain_batch(&pending, true);
    let mut states: Vec<_> = remaining.iter().map(|r| r.state).collect();
    states.sort_by_key(|s| format!("{s:?}"));
    assert_eq!(states, [PendingState::InFlight, PendingState::Uncertain]);
    eprintln!("canary step 2 (enabled): delivered pending only; uncertain+inflight preserved");

    // 3. Explicit native remember still writes.
    runtime.manual_remember("canary", &marker("manual")).await;
    runtime.wait_for_background().await;
    let mut after_manual = texts().await;
    for _ in 0..90 {
        if after_manual.len() >= 2 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        after_manual = texts().await;
    }
    assert_eq!(after_manual.len(), 2, "{after_manual:?}");
    assert!(after_manual.iter().any(|t| t.contains(&marker("manual"))));
    eprintln!("canary step 3 (explicit remember): delivered");

    // 4. Rollback (disable): a new intent is kept locally, not sent.
    config.enabled = false;
    assert!(runtime.save_and_adopt(config.clone()));
    seed("after-rollback", PendingState::Pending, 4);
    let before = queue_snapshot(&pending);
    runtime.drain_pending(false).await;
    runtime.wait_for_background().await;
    assert_eq!(queue_snapshot(&pending), before);
    // Leave time for any background write to land before asserting none did.
    tokio::time::sleep(std::time::Duration::from_secs(20)).await;
    assert_eq!(texts().await, after_manual, "rolled-back runtime wrote");
    eprintln!("canary step 4 (rolled back): no new service items, queue hash unchanged");
}

/// Every file in the queue directory, by name, so a failure shows what changed.
fn queue_snapshot(dir: &std::path::Path) -> std::collections::BTreeMap<String, String> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(Result::unwrap)
        .filter(|entry| entry.path().is_file())
        .map(|entry| {
            (
                entry.file_name().to_string_lossy().into_owned(),
                std::fs::read_to_string(entry.path()).unwrap_or_default(),
            )
        })
        .collect()
}
