//! Store behavior tests. Mirror `ice:packages/coding-agent/test/observe-store.test.ts`
//! as described by `docs/ice-gap/observability.md` OBS-011..OBS-013.

use super::*;
use crate::observe::recorder::{ObserveRecorder, ObserveRecorderOptions, ObserveSessionInfo};
use ice_agent::{AgentEvent, AgentMessage};
use ice_ai::types::UserMessage;
use std::sync::Arc;

fn open(dir: &tempfile::TempDir) -> Arc<ObserveRunStore> {
    Arc::new(ObserveRunStore::open(&dir.path().join("nested/observability.sqlite")).unwrap())
}

fn recorder(store: &Arc<ObserveRunStore>, session: &str) -> ObserveRecorder {
    ObserveRecorder::new(ObserveRecorderOptions {
        session_id: session.into(),
        info: ObserveSessionInfo {
            model: Some("faux/faux-1".into()),
            ..Default::default()
        },
        persistence: Some(Arc::clone(store) as Arc<dyn ObservePersistence>),
    })
}

fn user(text: &str) -> AgentEvent {
    AgentEvent::MessageStart {
        message: AgentMessage::User(UserMessage::new(text.to_string(), 0)),
    }
}

#[test]
fn persists_and_replays_after_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested/observability.sqlite");
    let run_id = {
        let store = Arc::new(ObserveRunStore::open(&path).unwrap());
        let rec = recorder(&store, "sess-a");
        rec.handle_agent_event(&user("one"));
        rec.handle_agent_event(&user("two"));
        let run_id = rec.run_id();
        rec.stop();
        run_id
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "history is private");
    }
    let store = ObserveRunStore::open(&path).unwrap();
    let run = store.get_run(&run_id).unwrap().unwrap();
    assert_eq!(run.status, ObserveRunStatus::Finished);
    assert!(run.ended_at.is_some());
    assert_eq!(run.model.as_deref(), Some("faux/faux-1"));

    let events = store.get_events(&run_id, None, None).unwrap();
    let sequences: Vec<_> = events.iter().map(|e| e.sequence).collect();
    assert_eq!(sequences, [1, 2, 3]);
    assert_eq!(events[0].event_type, "thread");
    assert_eq!(store.get_latest_sequence(&run_id).unwrap(), 3);
    assert_eq!(store.get_events(&run_id, Some(2.0), None).unwrap().len(), 1);

    let state = store.get_state(&run_id, None).unwrap().unwrap();
    let texts: Vec<_> = state
        .items
        .iter()
        .map(|i| i.text.clone().unwrap())
        .collect();
    assert_eq!(texts, ["one", "two"]);
    assert_eq!(state.threads.len(), 1);
    assert_eq!(state.header.main_thread_name, "main · sess-a");
    assert!(store.get_state("missing", None).unwrap().is_none());
}

#[test]
fn updates_replace_items_in_first_insertion_order() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir);
    let rec = recorder(&store, "s");
    rec.handle_agent_event(&AgentEvent::ToolExecutionStart {
        tool_call_id: "t".into(),
        tool_name: "read".into(),
        args: serde_json::json!({}),
    });
    rec.handle_agent_event(&user("between"));
    rec.handle_agent_event(&AgentEvent::ToolExecutionEnd {
        tool_call_id: "t".into(),
        tool_name: "read".into(),
        result: ice_agent::types::AgentToolResult {
            content: Vec::new(),
            details: serde_json::json!({ "ok": true }),
            usage: None,
            added_tool_names: Vec::new(),
            terminate: false,
        },
        is_error: false,
    });
    let state = store.get_state(&rec.run_id(), None).unwrap().unwrap();
    assert_eq!(state.items.len(), 2);
    assert_eq!(state.items[0].kind, ObserveItemKind::Tool);
    assert_eq!(state.items[0].state, Some(ObserveItemState::Done));
    // An explicit limit folds only the first page.
    let partial = store.get_state(&rec.run_id(), Some(2.0)).unwrap().unwrap();
    assert_eq!(partial.items[0].state, Some(ObserveItemState::Running));
}

#[test]
fn durable_payloads_are_redacted_but_live_state_is_not() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir);
    let rec = recorder(&store, "s");
    rec.handle_agent_event(&AgentEvent::ToolExecutionStart {
        tool_call_id: "t".into(),
        tool_name: "bash".into(),
        args: serde_json::json!({ "command": "curl -H 'Authorization: Bearer sk-live-123' x", "api_key": "abc" }),
    });
    let live = rec.get_state().items[0].args.clone().unwrap();
    assert!(live.contains("sk-live-123"));
    let stored = store.get_state(&rec.run_id(), None).unwrap().unwrap();
    let args = stored.items[0].args.clone().unwrap();
    assert!(!args.contains("sk-live-123"), "{args}");
    assert!(!args.contains("\"abc\""), "{args}");
    assert!(args.contains("[REDACTED]"));
}

#[test]
fn redaction_rules() {
    assert_eq!(
        redact_credential_text("Authorization: Bearer abc.def"),
        "Authorization: Bearer [REDACTED]"
    );
    assert_eq!(
        redact_credential_text("api_key=sk-1 next"),
        "api_key=[REDACTED] next"
    );
    assert_eq!(
        redact_credential_text(r#"{"password": "hunter2"}"#),
        r#"{"password": "[REDACTED]"}"#
    );
    assert_eq!(
        redact_credential_text("tokenizer output"),
        "tokenizer output"
    );
    let value = redact_value(&serde_json::json!({
        "token": "t", "refresh_token": "r", "nested": [{ "secret": "s", "count": 3 }], "name": "ok"
    }));
    assert_eq!(
        value,
        serde_json::json!({
            "token": "[REDACTED]", "refresh_token": "[REDACTED]",
            "nested": [{ "secret": "[REDACTED]", "count": 3 }], "name": "ok"
        })
    );
}

#[test]
fn retention_prunes_old_runs_on_open() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("o.sqlite");
    let run_id = {
        let store = Arc::new(ObserveRunStore::open(&path).unwrap());
        let rec = recorder(&store, "old");
        rec.handle_agent_event(&user("x"));
        rec.run_id()
    };
    // Still within the default retention.
    assert!(ObserveRunStore::open(&path)
        .unwrap()
        .get_run(&run_id)
        .unwrap()
        .is_some());
    // A negative retention puts the cutoff in the future.
    let store = ObserveRunStore::open_with_retention(&path, -60_000).unwrap();
    assert!(store.get_run(&run_id).unwrap().is_none());
    assert_eq!(store.get_latest_sequence(&run_id).unwrap(), 0);
}

#[test]
fn listing_live_and_latest_runs() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir);
    let a = recorder(&store, "sess");
    std::thread::sleep(std::time::Duration::from_millis(5));
    let b = recorder(&store, "sess");
    b.handle_agent_event(&user("recent"));
    let runs = store.list_runs(None).unwrap();
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].run_id, b.run_id(), "newest first");
    assert_eq!(
        store
            .get_latest_run_for_session("sess")
            .unwrap()
            .unwrap()
            .run_id,
        b.run_id()
    );
    let live = store.list_live_runs(now_ms() - 60_000, None).unwrap();
    assert_eq!(live.len(), 2);
    a.stop();
    let live = store.list_live_runs(now_ms() - 60_000, None).unwrap();
    assert_eq!(live.len(), 1, "finished runs are not live");
    assert!(store
        .list_live_runs(now_ms() + 60_000, None)
        .unwrap()
        .is_empty());
}

#[test]
fn reopening_a_run_continues_its_sequence() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("o.sqlite");
    let store = ObserveRunStore::open(&path).unwrap();
    let header = ObserveHeader {
        run_id: "r".into(),
        session_id: "s".into(),
        main_thread_id: "s".into(),
        main_thread_name: "main · s".into(),
        session_name: None,
        model: None,
        thinking_level: None,
        context: None,
        usage: ObserveUsage::default(),
        started_at: now_ms(),
    };
    let event = ObserveRecorderEvent::Header {
        header: header.clone(),
    };
    store.record_run(&header).unwrap();
    store.record_event(&event, &header).unwrap();
    drop(store);
    let store = ObserveRunStore::open(&path).unwrap();
    store.record_run(&header).unwrap();
    store.record_event(&event, &header).unwrap();
    assert_eq!(store.get_latest_sequence("r").unwrap(), 2);
}

#[test]
fn closed_store_rejects_use_and_limits_clamp() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir);
    store.close().unwrap();
    store.close().unwrap();
    assert_eq!(
        store.list_runs(None).unwrap_err(),
        "Observe run store is closed"
    );
    assert!(
        store.finish_run("x", 1).is_ok(),
        "finish after close is a no-op"
    );

    assert_eq!(normalize_limit(None), 100);
    assert_eq!(normalize_limit(Some(f64::NAN)), 100);
    assert_eq!(normalize_limit(Some(0.0)), 1);
    assert_eq!(normalize_limit(Some(2.9)), 2);
    assert_eq!(normalize_limit(Some(1e9)), 10_000);
    assert_eq!(normalize_cursor(Some(-1.0)), 0);
    assert_eq!(normalize_cursor(Some(1.5)), 0);
    assert_eq!(normalize_cursor(Some(7.0)), 7);
}

#[test]
fn corrupt_payload_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("o.sqlite");
    let store = Arc::new(ObserveRunStore::open(&path).unwrap());
    let rec = recorder(&store, "s");
    let run_id = rec.run_id();
    let conn = Connection::open(&path).unwrap();
    conn.execute(
        "UPDATE observe_events SET payload_json = '{' WHERE run_id = ?1",
        params![run_id],
    )
    .unwrap();
    let error = store.get_events(&run_id, None, None).unwrap_err();
    assert!(error.contains("corrupt observe event"), "{error}");
}
