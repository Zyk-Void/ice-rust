#[test]
fn judged_config_roundtrip_and_bounded_typed_request() {
    use super::super::config::ReadPolicy;
    assert_eq!(ReadPolicy::parse("judged"), Some(ReadPolicy::Judged));
    assert_eq!(ReadPolicy::Judged.as_str(), "judged");
    assert_eq!(CogneeConfig::default().read_policy, ReadPolicy::Legacy);
    assert_eq!(admit_prompt(ReadPolicy::Judged, "fix it"), Admission::Recall(super::super::policy::RecallReason::Judged));
    let request = helper::build_read_gate_request("Build the current project", Some(&"a".repeat(10_000)), "fixture/read").unwrap();
    assert_eq!(request.questions().len(), 2);
    assert!(matches!(request.questions()["needs"], Question::Choice { .. }));
    assert!(matches!(request.questions()["incomplete"], Question::Noul { .. }));
    assert!(serde_json::to_string(request.state()).unwrap().len() < 1200);
    assert!(helper::build_read_gate_request(&"x".repeat(10_000), None, "fixture/read").is_err());
    let config = CogneeConfig { read_policy: ReadPolicy::Judged, write_policy: WritePolicy::Manual, ..CogneeConfig::default() };
    assert!(Helper::bind_read_gate(&config).unwrap().is_none());
}

struct ReadDecider {
    choice: &'static str,
    confidence: f64,
    delay_ms: u64,
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl helper::DecisionDecider for ReadDecider {
    async fn decide(
        &self,
        request: &typesafe::Request,
        _timeout: Duration,
        _signal: &CancellationToken,
    ) -> Result<typesafe::Response, DecisionError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.choice == "check_previous" {
            let state = serde_json::to_value(request.state()).unwrap();
            assert!(state.to_string().contains("previous assistant marker"));
            assert!(state.to_string().len() < 1500);
            return Err(DecisionError::Cancelled);
        }
        tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
        if self.choice == "error" {
            return Err(DecisionError::Cancelled);
        }
        if self.choice == "abstain" {
            return Err(DecisionError::Timeout);
        }
        let bytes = serde_json::to_vec(&json!({
            "model":"fixture/read", "answers": {
                "needs": {"type":"choice", "choice":self.choice,
                    "probabilities":{"none": if self.choice == "none" {1.0} else {0.0},
                        "session": if self.choice == "session" {1.0} else {0.0},
                        "memory": if self.choice == "memory" {1.0} else {0.0}},
                    "confidence": self.confidence},
                "incomplete":{"type":"noul","noul":0.8}
            }, "usage":{"input_tokens":1,"output_tokens":1}
        })).unwrap();
        typesafe::Response::decode_json(&bytes, request).map_err(DecisionError::Codec)
    }
}

#[tokio::test]
async fn judged_read_gate_choices_and_event_fallback() {
    for (choice, confidence, delay, prompt, expected) in [
        ("none", 0.9, 0, "What did we decide last session?", false),
        ("session", 0.9, 0, "What did we decide last session?", false),
        ("memory", 0.9, 0, "Explain cache eviction for this project", true),
        ("none", 0.4, 0, "What did we decide last session?", true),
        ("none", 0.4, 0, "Build the current project", false),
        ("error", 0.9, 0, "What did we decide last session?", true),
        ("error", 0.9, 0, "Build the current project", false),
        ("abstain", 0.9, 0, "What did we decide last session?", true),
        ("abstain", 0.9, 0, "Build the current project", false),
        ("none", 0.9, 100, "What did we decide last session?", true),
        ("none", 0.9, 100, "Build the current project", false),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        transport.push(FakeResponse::Ok {
            status: 200, headers: vec![],
            chunks: vec![json!({"results":[{"text":"Cache uses LRU eviction","score":0.9}]}).to_string().into_bytes()],
            delay_ms: 50,
        });
        let rt = runtime(dir.path(), &transport,
            Some(json!({"readPolicy":"judged","writePolicy":"manual"})), false);
        rt.ensure_loaded();
        rt.ensure_session_id("host1");
        let calls = Arc::new(AtomicUsize::new(0));
        rt.state().read_helper = Some(Arc::new(Helper::with_decider(Arc::new(ReadDecider {
            choice, confidence, delay_ms: delay, calls: calls.clone(),
        }), 20)));
        let result = memory(&rt, prompt, None).await;
        assert_eq!(result.is_some(), expected, "{choice} {confidence} {delay} {prompt}");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(transport.sent().len(), 1, "recall must begin alongside the gate");
        if expected {
            assert!(result.unwrap().contains("Cache uses LRU eviction"));
        } else {
            assert!(rt.state().last_read_gate.contains("recallCancelled"));
        }
        assert!(rt.collect_doctor().await.doctor_line().contains("readGate="));
    }
}

#[tokio::test]
async fn judged_request_boundary_supplies_previous_assistant_only_once() {
    let dir = tempfile::tempdir().unwrap();
    let transport = Arc::new(FakeTransport::default());
    transport.push(FakeResponse::json(200, json!({"results":[]})));
    let rt = runtime(dir.path(), &transport, Some(json!({"readPolicy":"judged", "writePolicy":"manual"})), false);
    rt.ensure_loaded();
    rt.ensure_session_id("host1");
    let calls = Arc::new(AtomicUsize::new(0));
    rt.state().read_helper = Some(Arc::new(Helper::with_decider(Arc::new(ReadDecider {
        choice: "check_previous", confidence: 0.9, delay_ms: 0, calls: calls.clone(),
    }), 20)));
    let messages = vec![assistant(&format!("previous assistant marker {}", "x".repeat(2000)))];
    assert!(rt.request_context(&request_event("Build the current project", &messages, "judged-run", 0)).await.is_none());
    assert!(rt.request_context(&request_event("Build the current project", &messages, "judged-run", 1)).await.is_none());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn judged_unbound_uses_event_rule() {
    for (prompt, expected) in [("What did we decide last session?", true), ("Build the current project", false)] {
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        transport.push(FakeResponse::json(200, json!({"results":[{"text":"Cache uses LRU eviction","score":0.9}]})));
        let rt = runtime(dir.path(), &transport,
            Some(json!({"readPolicy":"judged","writePolicy":"manual"})), false);
        assert_eq!(memory(&rt, prompt, None).await.is_some(), expected);
        assert!(rt.state().last_read_gate.contains("unbound"));
    }
}

#[tokio::test]
async fn judged_abort_discards_late_gate_and_recall() {
    let dir = tempfile::tempdir().unwrap();
    let transport = Arc::new(FakeTransport::default());
    transport.push(FakeResponse::Ok {
        status: 200, headers: vec![], chunks: vec![json!({"results":[{"text":"late memory"}]}).to_string().into_bytes()], delay_ms: 100,
    });
    let rt = runtime(dir.path(), &transport,
        Some(json!({"readPolicy":"judged","writePolicy":"manual"})), false);
    rt.ensure_loaded();
    rt.ensure_session_id("host1");
    let calls = Arc::new(AtomicUsize::new(0));
    rt.state().read_helper = Some(Arc::new(Helper::with_decider(Arc::new(ReadDecider {
        choice: "memory", confidence: 0.9, delay_ms: 100, calls: calls.clone(),
    }), 1500)));
    let signal = CancellationToken::new();
    let cancel = signal.clone();
    let event = BeforeRunEvent { prompt: "Use our previous project decisions", system_prompt: None,
        session_id: "host1", signal };
    let (result, ()) = tokio::join!(rt.prompt_context(&event), async {
        tokio::time::sleep(Duration::from_millis(5)).await;
        cancel.cancel();
    });
    assert!(result.is_none());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(rt.state().last_recall_hits, 0);
    assert_eq!(rt.state().last_read_gate, "not_run");
}
