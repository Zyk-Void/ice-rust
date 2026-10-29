// Included inside client::tests. These contracts are source-backed native
// requests and synthetic responses; they do not probe a deployed service.

#[tokio::test]
async fn native_contract_inventory_matches_exact_recall_and_write_shapes() {
    let transport = Arc::new(FakeTransport::default());
    transport.push(FakeResponse::json(200, json!({"results": []})));
    transport.push(FakeResponse::json(202, json!({"job_id": "remember-job"})));
    transport.push(FakeResponse::json(202, json!({"task_id": "improve-task"})));
    let client = client(transport.clone());

    client
        .recall(
            "missing decision",
            RecallOptions {
                dataset: Some("repo".into()),
                session_id: Some("session-1".into()),
                top_k: Some(3),
                scope: vec!["graph".into()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let remember = client
        .remember_with_receipt(
            "fact",
            "user_context",
            Some("repo"),
            Some("session-1"),
            CallOptions::default(),
        )
        .await
        .unwrap();
    let improve = client
        .improve_with_receipt(
            Some("repo"),
            &["session-1".into()],
            CallOptions::default(),
        )
        .await
        .unwrap();

    let sent = transport.sent();
    assert_eq!(sent.len(), 3);
    assert_eq!(sent[0].url, "http://cognee.test/base/api/v1/recall");
    assert_eq!(sent[0].method, Method::Post);
    assert_eq!(
        body_json(&sent[0]),
        json!({
            "query": "missing decision",
            "topK": 3,
            "onlyContext": true,
            "scope": ["graph"],
            "sessionId": "session-1",
            "datasets": ["repo"]
        })
    );

    assert_eq!(sent[1].url, "http://cognee.test/base/api/v1/remember");
    assert_eq!(sent[1].method, Method::Post);
    let Body::Multipart(parts) = &sent[1].body else {
        panic!("native remember uses multipart form data")
    };
    let fields: Vec<_> = parts
        .iter()
        .map(|part| (part.name.as_str(), part.value.as_str()))
        .collect();
    assert_eq!(
        fields,
        vec![
            ("datasetName", "repo"),
            ("node_set", "user_context"),
            ("run_in_background", "true"),
            ("session_id", "session-1"),
            ("data", "fact")
        ]
    );
    assert_eq!(parts[4].filename.as_deref(), Some("ice-cognee.txt"));
    assert_eq!(remember.job_id.as_deref(), Some("remember-job"));

    assert_eq!(sent[2].url, "http://cognee.test/base/api/v1/improve");
    assert_eq!(sent[2].method, Method::Post);
    assert_eq!(
        body_json(&sent[2]),
        json!({
            "datasetName": "repo",
            "sessionIds": ["session-1"],
            "runInBackground": true
        })
    );
    assert_eq!(improve.job_id.as_deref(), Some("improve-task"));
}

#[test]
fn absent_or_malformed_recall_provenance_stays_unknown() {
    let results = normalize_recall(&json!({
        "results": [{"text": "unattributed fact", "score": 0.4}]
    }))
    .unwrap();
    let metadata = results[0].metadata.as_ref().unwrap();
    assert!(metadata.get("source_id").is_none());
    assert!(metadata.get("dataset").is_none());
    assert!(metadata.get("branch").is_none());
    assert!(metadata.get("revision").is_none());
    assert!(metadata.get("expires_at").is_none());

    let malformed = normalize_recall(&json!({"results": "not-an-array"})).unwrap_err();
    assert_eq!(malformed.kind, CogneeErrorKind::Malformed);
}

#[tokio::test]
async fn recall_contract_failures_are_terminal_and_never_guess_another_endpoint() {
    for (status, expected_kind) in [
        (404, CogneeErrorKind::NotFound),
        (422, CogneeErrorKind::Malformed),
    ] {
        let transport = Arc::new(FakeTransport::default());
        transport.push(FakeResponse::json(status, json!({"detail": "fixture"})));
        let error = client(transport.clone())
            .recall("query", RecallOptions::default())
            .await
            .unwrap_err();
        assert_eq!(error.kind, expected_kind);
        assert_eq!(error.status, Some(status));
        let sent = transport.sent();
        assert_eq!(sent.len(), 1, "HTTP {status} is not retried");
        assert_eq!(sent[0].url, "http://cognee.test/base/api/v1/recall");
    }
}

#[tokio::test]
async fn accepted_write_response_without_known_handle_remains_unverified() {
    for response in [
        FakeResponse::json(202, json!({"status": "completed"})),
        FakeResponse::json(202, json!({"status": "accepted", "id": "unknown-shape"})),
    ] {
        let transport = Arc::new(FakeTransport::default());
        transport.push(response);
        let receipt = client(transport.clone())
            .improve_with_receipt(None, &[], CallOptions::default())
            .await
            .unwrap();
        assert_eq!(receipt.job_id, None);
        assert_eq!(transport.sent().len(), 1);
    }
}
