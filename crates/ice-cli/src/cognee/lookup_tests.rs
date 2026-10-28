// Included inside runtime::tests to exercise the actual command/tool lookup.

#[tokio::test]
async fn explicit_lookup_retains_only_applicable_whole_records_and_truthful_counts() {
    let dir = tempfile::tempdir().unwrap();
    let transport = Arc::new(FakeTransport::default());
    transport.push(FakeResponse::json(200,json!({"results":[
        {"text":"Cache uses LRU\nonly with revision validation.","metadata":{"source_id":"decision-1"}},
        {"text":"Cache stale branch rule","metadata":{"branch":"definitely-other-branch"}},
        {"text":"Cache invalid validity","metadata":{"valid_until_ms":"tomorrow"}},
        {"text":"Cache deleted rule","metadata":{"deleted":true}},
        {"text":"bash succeeded"}
    ]})));
    let rt = runtime(
        dir.path(),
        &transport,
        Some(json!({"readPolicy":"on_demand","writePolicy":"manual","topK":8})),
        false,
    );
    let (text, details) = rt.search_for_tool("cache", CancellationToken::new()).await;
    assert_eq!(details["count"], 1);
    assert_eq!(details["backendResultObjects"], 5);
    assert_eq!(details["normalizedRecords"], 5);
    assert!(text.contains("only with revision validation."));
    assert!(text.contains("source=decision-1"));
    assert!(
        !text.contains("stale branch")
            && !text.contains("deleted rule")
            && !text.contains("bash succeeded")
    );
    assert!(text.ends_with("</ice-void-cognee-memory>"));
    assert_eq!(json_body(&transport.sent()[0])["onlyContext"], true);
    rt.wait_for_background().await;
}

#[tokio::test]
async fn explicit_lookup_can_repeat_and_never_marks_automatic_evidence_seen() {
    let dir = tempfile::tempdir().unwrap();
    let transport = Arc::new(FakeTransport::default());
    for _ in 0..2 {
        transport.push(FakeResponse::json(
            200,
            json!({"results":[{"text":"Cache uses LRU","metadata":{"source_id":"s1"}}]}),
        ));
    }
    let rt = runtime(
        dir.path(),
        &transport,
        Some(json!({"readPolicy":"on_demand","writePolicy":"manual"})),
        false,
    );
    let first = rt.search_for_tool("cache", CancellationToken::new()).await;
    let second = rt.search_for_tool("cache", CancellationToken::new()).await;
    assert_eq!(first.0, second.0);
    assert_eq!(first.1["count"], 1);
    assert_eq!(second.1["count"], 1);
    assert_eq!(transport.sent().len(), 2);
    assert_eq!(rt.state().helper_requests, 0);
    rt.wait_for_background().await;
}

#[tokio::test]
async fn explicit_lookup_configuration_change_discards_late_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let transport = Arc::new(FakeTransport::default());
    transport.push(FakeResponse::Ok {
        status: 200,
        headers: vec![],
        chunks: vec![json!({"results":[{"text":"Cache old result"}]})
            .to_string()
            .into_bytes()],
        delay_ms: 40,
    });
    let rt = runtime(
        dir.path(),
        &transport,
        Some(json!({"readPolicy":"event","writePolicy":"manual"})),
        false,
    );
    let lookup = rt.search_for_tool("cache", CancellationToken::new());
    let disable = async {
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        let mut next = rt.config();
        next.enabled = false;
        rt.persist_config_now(next).unwrap();
    };
    let (result, ()) = tokio::join!(lookup, disable);
    assert!(result.1.get("error").is_some());
    assert_eq!(result.1["count"], 0);
    assert!(!result.0.contains("Cache old result"));
    assert_eq!(transport.sent().len(), 1);
    rt.wait_for_background().await;
}

#[tokio::test]
async fn explicit_lookup_cancelled_or_oversized_query_makes_no_request() {
    let dir = tempfile::tempdir().unwrap();
    let transport = Arc::new(FakeTransport::default());
    let rt = runtime(dir.path(), &transport, Some(json!({})), false);
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert_eq!(
        rt.search_for_tool("cache", cancelled).await.1["error"],
        "aborted"
    );
    assert_eq!(
        rt.search_for_tool(&"x".repeat(4001), CancellationToken::new())
            .await
            .1["error"],
        "invalid_query"
    );
    assert!(transport.sent().is_empty());
}
