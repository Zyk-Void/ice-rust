// Included inside client::tests. Only local synthetic transport responses.

#[test]
fn graph_normalization_keeps_complete_records_and_separate_counts() {
    let first = format!(
        "Cache policy {}\nonly when the branch and revision match.",
        "detail ".repeat(100)
    );
    let graph = format!("Node: cache-decision\n__node_content_start__\n{first}\n__node_content_end__\nNode: verification\n__node_content_start__\nVerify current files before acting.\n__node_content_end__");
    let results = normalize_recall(&json!({"results":[{"text":graph,"score":0.8,"metadata":{"source_id":"source-42","dataset":"repo","branch":"main","revision":"abc","valid_until_ms":1000}}]})).unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(backend_object_count(&results), Some(1));
    assert_eq!(results[0].text, first);
    assert!(results[0]
        .text
        .ends_with("only when the branch and revision match."));
    assert_eq!(
        results[0].metadata.as_ref().unwrap()["source_id"],
        "source-42"
    );
    assert_eq!(results[1].metadata.as_ref().unwrap()["revision"], "abc");
    assert_eq!(results[0].metadata.as_ref().unwrap()["graph_node_index"], 0);
    assert_eq!(results[1].metadata.as_ref().unwrap()["graph_node_index"], 1);
}

#[test]
fn incomplete_graph_and_conflicting_metadata_do_not_become_clean_facts() {
    assert!(normalize_recall(
        &json!({"results":[{"text":"Node: x\n__node_content_start__\nunclosed"}]})
    )
    .is_err());
    let results = normalize_recall(
        &json!({"results":[{"text":"Cache decision","branch":"old","metadata":{"branch":"main"}}]}),
    )
    .unwrap();
    assert_eq!(
        results[0].metadata.as_ref().unwrap()["invalid_applicability_metadata"],
        true
    );
    let results =
        normalize_recall(&json!({"results":[{"text":"Cache decision","metadata":["invalid"]}]}))
            .unwrap();
    assert_eq!(
        results[0].metadata.as_ref().unwrap()["invalid_applicability_metadata"],
        true
    );
    assert_eq!(backend_object_count(&[]), None);
}

#[tokio::test]
async fn oversized_first_record_does_not_hide_a_later_complete_fact() {
    let transport = Arc::new(FakeTransport::default());
    transport.push(FakeResponse::json(200,json!({"results":[{"text":"x".repeat(500)}, {"text":"Use LRU","metadata":{"source_id":"s1"}}]})));
    let mut client = client(transport);
    client.config.max_response_chars = 40;
    let results = client
        .recall(
            "cache",
            RecallOptions {
                top_k: Some(1),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].text, "Use LRU");
    assert_eq!(
        results[0].metadata.as_ref().unwrap()["retrieved_dataset"],
        "ice"
    );
    assert_eq!(backend_object_count(&results), Some(2));
}

#[test]
fn root_scope_fields_are_not_discarded_during_normalization() {
    let results = normalize_recall(&json!([{"content":"Cache fact","source_entry_id":"entry-1","dataset":"repo","git_branch":"work","git_revision":"abc","valid_until_ms":99,"deleted":true}])).unwrap();
    let metadata = results[0].metadata.as_ref().unwrap();
    assert_eq!(metadata["source_entry_id"], "entry-1");
    assert_eq!(metadata["git_branch"], "work");
    assert_eq!(metadata["git_revision"], "abc");
    assert_eq!(metadata["valid_until_ms"], 99);
    assert_eq!(metadata["deleted"], true);
}
