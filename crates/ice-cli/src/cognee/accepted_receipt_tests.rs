#[tokio::test]
async fn write_acceptance_keeps_valid_optional_handles_without_an_extra_request() {
    let transport = Arc::new(FakeTransport::default());
    transport.push(FakeResponse::json(
        202,
        json!({"job_id":"job-42","status":"completed"}),
    ));
    transport.push(FakeResponse::json(202, json!({"taskId":"task-7"})));
    let client = client(transport.clone());
    assert_eq!(
        client
            .remember_with_receipt("fact", "nodes", None, None, CallOptions::default())
            .await
            .unwrap()
            .job_id
            .as_deref(),
        Some("job-42")
    );
    assert_eq!(
        client
            .improve_with_receipt(None, &[], CallOptions::default())
            .await
            .unwrap()
            .job_id
            .as_deref(),
        Some("task-7")
    );
    assert_eq!(transport.sent().len(), 2);
}

#[tokio::test]
async fn accepted_invalid_acknowledgments_never_become_replayable_errors() {
    for body in [
        b"{broken".to_vec(),
        vec![b'x'; 5000],
        b"{}".to_vec(),
        br#"{"job_id":"https://wrong.example/?key=secret"}"#.to_vec(),
    ] {
        let transport = Arc::new(FakeTransport::default());
        transport.push(FakeResponse::Ok {
            status: 202,
            headers: vec![],
            chunks: vec![body],
            delay_ms: 0,
        });
        let receipt = client(transport.clone())
            .improve_with_receipt(None, &[], CallOptions::default())
            .await
            .unwrap();
        assert_eq!(receipt.job_id, None);
        assert_eq!(transport.sent().len(), 1);
    }
    let transport = Arc::new(FakeTransport::default());
    transport.push(FakeResponse::json(404, json!({"job_id":"not-accepted"})));
    assert!(client(transport)
        .improve_with_receipt(None, &[], CallOptions::default())
        .await
        .is_err());
}

struct NeverEndingAcceptedBody;
#[async_trait::async_trait]
impl CogneeTransport for NeverEndingAcceptedBody {
    async fn send(&self, _request: HttpRequest) -> Result<HttpResponse, String> {
        Ok(HttpResponse {
            status: 202,
            headers: vec![],
            body: Box::pin(futures::stream::pending()),
        })
    }
}

#[tokio::test]
async fn timeout_after_acceptance_does_not_reclassify_the_write_as_failed() {
    let client = CogneeClient::new(
        ClientConfig {
            base_url: "http://fixture.test/".into(),
            dataset: "d".into(),
            api_key: None,
            recall_budget_ms: 1000,
            max_response_chars: 12000,
        },
        Arc::new(NeverEndingAcceptedBody),
    );
    let result = tokio::time::timeout(
        Duration::from_secs(1),
        client.improve_with_receipt(
            None,
            &[],
            CallOptions {
                timeout_ms: Some(20),
                ..Default::default()
            },
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(result.job_id, None);
}
