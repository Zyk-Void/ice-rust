//! End-to-end pipeline test: a faux-provider harness run feeds the recorder
//! through the CLI wiring and lands in the history store.

use super::*;
use crate::observe::types::{ObserveItemKind, ObserveRole, ObserveRunStatus};
use ice_ai::providers::faux::{FauxProvider, FauxScript};
use ice_ai::{Api, Model};
use ice_harness::types::AgentHarnessOptions;

#[tokio::test(flavor = "multi_thread")]
async fn a_run_is_recorded_with_prompts_custom_entries_and_finish() {
    let dir = tempfile::tempdir().unwrap();
    let (emitter, events) = ice_agent::events::BroadcastEmitter::new(256);
    let script = FauxScript::new()
        .with_text("first answer")
        .with_text("second answer");
    let mut model = Model::new("m", "m", Api::Faux, "faux", "http://x");
    model.context_window = 200_000;
    let harness = AgentHarness::create(AgentHarnessOptions {
        model,
        models: vec![FauxProvider::new(script)],
        agent_emitter: Some(Arc::new(emitter)),
        ..Default::default()
    })
    .await
    .unwrap();
    // Entries that exist before the recorder starts are not projected.
    harness
        .append_custom_entry(
            "ice-goals-state",
            Some(serde_json::json!({ "goals": [{ "id": "old" }] })),
        )
        .await
        .unwrap();

    let (runtime, warning) = start(&harness, events, Some(dir.path()), None).await;
    assert!(warning.is_none(), "{warning:?}");
    let store = runtime.store.clone().unwrap();
    let run_id = runtime.recorder.run_id();

    harness
        .prompt_text("hello there", Vec::new())
        .await
        .unwrap();
    harness
        .append_custom_entry(
            "ice-goals-state",
            Some(serde_json::json!({ "goals": [{ "id": "g1", "status": "active" }] })),
        )
        .await
        .unwrap();
    harness.prompt_text("again", Vec::new()).await.unwrap();
    runtime.stop().await;

    let run = store.get_run(&run_id).unwrap().unwrap();
    assert_eq!(run.status, ObserveRunStatus::Finished);
    assert_eq!(run.model.as_deref(), Some("faux/m"));
    let state = store.get_state(&run_id, None).unwrap().unwrap();
    let rows: Vec<(ObserveItemKind, Option<ObserveRole>, String)> = state
        .items
        .iter()
        .map(|i| (i.kind, i.role, i.text.clone().unwrap_or_default()))
        .collect();
    assert_eq!(
        rows,
        [
            (
                ObserveItemKind::Message,
                Some(ObserveRole::User),
                "hello there".to_string()
            ),
            (
                ObserveItemKind::Message,
                Some(ObserveRole::Assistant),
                "first answer".to_string()
            ),
            (
                ObserveItemKind::Goal,
                None,
                "goal active · tasks 0/0 · runs 0/? · reviews 0/3 · g1".to_string()
            ),
            (
                ObserveItemKind::Message,
                Some(ObserveRole::User),
                "again".to_string()
            ),
            (
                ObserveItemKind::Message,
                Some(ObserveRole::Assistant),
                "second answer".to_string()
            ),
        ]
    );
    assert!(state.header.usage.output > 0);
    assert_eq!(state.header.context.unwrap().context_window, 200_000);
}

#[tokio::test]
async fn an_unopenable_store_only_warns() {
    let dir = tempfile::tempdir().unwrap();
    // A file where the agent directory should be makes the store unopenable.
    let blocker = dir.path().join("agent");
    std::fs::write(&blocker, "not a dir").unwrap();
    let (emitter, events) = ice_agent::events::BroadcastEmitter::new(16);
    let harness = AgentHarness::create(AgentHarnessOptions {
        model: Model::new("m", "m", Api::Faux, "faux", "http://x"),
        models: vec![FauxProvider::new(FauxScript::new())],
        agent_emitter: Some(Arc::new(emitter)),
        ..Default::default()
    })
    .await
    .unwrap();
    let (runtime, warning) = start(&harness, events, Some(&blocker), None).await;
    assert!(warning
        .unwrap()
        .starts_with("Warning: observability history unavailable: "));
    assert!(runtime.store.is_none());
    runtime.stop().await;
}
