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
fn analysis_pages_preserve_boundaries_mark_bad_rows_and_sanitize_legacy_payloads() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir);
    let rec = recorder(&store, "analysis");
    rec.handle_agent_event(&user("ordinary diagnostic"));
    let id = rec.run_id();
    let first = store.analysis_page(&id, 0, None, 1).unwrap().unwrap();
    assert_eq!(first.schema_version, 1);
    assert_eq!(first.source, "local-observe");
    assert_eq!(first.timestamp_unit, "unix_ms");
    assert!(first.has_more);
    let boundary = first.through_sequence;
    rec.handle_agent_event(&user("later append excluded"));
    store.with_conn(|conn, _| {
        conn.execute("UPDATE observe_events SET payload_json = replace(payload_json, 'ordinary diagnostic', 'EXAMPLE_API_KEY=DUMMY_ANALYSIS') WHERE run_id = ?1", params![id]).map_err(sql_err)?;
        Ok(())
    }).unwrap();
    let second = store
        .analysis_page(&id, first.next_after, Some(boundary), 100)
        .unwrap()
        .unwrap();
    assert!(!second.has_more);
    assert_eq!(second.next_after, boundary);
    assert_eq!(second.events.len(), 1);
    assert_eq!(second.events[0].item_id, Some(1));
    let encoded = serde_json::to_string(&second).unwrap();
    assert!(!encoded.contains("DUMMY_ANALYSIS"));
    assert!(!encoded.contains("later append excluded"));
    store.with_conn(|conn, _| {
        let raw: String = conn.query_row("SELECT payload_json FROM observe_events WHERE run_id = ?1 AND sequence = ?2", params![id, boundary], |row| row.get(0)).map_err(sql_err)?;
        assert!(raw.contains("DUMMY_ANALYSIS"));
        conn.execute("UPDATE observe_events SET event_type = 'header' WHERE run_id = ?1 AND sequence = ?2", params![id, boundary]).map_err(sql_err)?;
        conn.execute("INSERT INTO observe_events VALUES (?1, 100, 123, 'activity', '{DUMMY_MALFORMED')", params![id]).map_err(sql_err)?;
        conn.execute("INSERT INTO observe_events VALUES (?1, 101, 124, 'activity', ?2)", params![id, "DUMMY_OVERSIZE".repeat(30_000)]).map_err(sql_err)?;
        Ok(())
    }).unwrap();
    let page = store
        .analysis_page(&id, boundary - 1, None, 100)
        .unwrap()
        .unwrap();
    let reasons: Vec<_> = page
        .events
        .iter()
        .filter_map(|event| event.unavailable.as_deref())
        .collect();
    assert_eq!(reasons, ["type_mismatch", "malformed", "payload_too_large"]);
    assert!(page
        .events
        .iter()
        .any(|event| event.sequence == 100 && event.ts_ms == 123));
    let encoded = serde_json::to_string(&page).unwrap();
    assert!(!encoded.contains("DUMMY_"));
    assert!(!page.has_more);
    assert_eq!(page.next_after, 101);
    assert!(store
        .analysis_page("missing", 0, None, 10)
        .unwrap()
        .is_none());
}

#[test]
fn analysis_page_byte_budget_and_update_identity_are_explicit() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir);
    let rec = recorder(&store, "analysis-budget");
    rec.handle_agent_event(&AgentEvent::ToolExecutionStart {
        tool_call_id: "call".into(),
        tool_name: "synthetic".into(),
        args: serde_json::json!({}),
    });
    // Persist an update for the same activity without deriving identity from text.
    let mut event = store
        .get_events(&rec.run_id(), Some(1.0), Some(1.0))
        .unwrap()
        .remove(0)
        .payload;
    if let ObserveRecorderEvent::Activity { item } = event {
        event = ObserveRecorderEvent::ActivityUpdate { item };
    }
    store.record_event(&event, &rec.get_header()).unwrap();
    let page = store
        .analysis_page(&rec.run_id(), 0, None, 100)
        .unwrap()
        .unwrap();
    assert_eq!(page.events[1].item_id, page.events[2].item_id);
    assert_ne!(page.events[1].sequence, page.events[2].sequence);
    let id = rec.run_id();
    store
        .with_conn(|conn, _| {
            for sequence in 100..112 {
                conn.execute(
                    "INSERT INTO observe_events VALUES (?1, ?2, 123, 'activity', ?3)",
                    params![id, sequence, "x".repeat(250_000)],
                )
                .map_err(sql_err)?;
            }
            Ok(())
        })
        .unwrap();
    let page = store.analysis_page(&id, 99, None, 1000).unwrap().unwrap();
    assert!(page.has_more);
    assert_eq!(page.events.len(), 8);
    assert!(page
        .events
        .iter()
        .all(|event| event.unavailable.as_deref() == Some("malformed")));
    let rest = store
        .analysis_page(&id, page.next_after, Some(page.through_sequence), 1000)
        .unwrap()
        .unwrap();
    assert_eq!(rest.events.len(), 4);
    assert!(!rest.has_more);
    store
        .with_conn(|conn, _| {
            conn.execute(
                "DELETE FROM observe_events WHERE run_id = ?1 AND sequence = 111",
                params![id],
            )
            .map_err(sql_err)?;
            Ok(())
        })
        .unwrap();
    let last = store
        .analysis_page(&id, 110, Some(111), 1000)
        .unwrap()
        .unwrap();
    assert_eq!(last.through_sequence, 111);
    assert!(last.events.is_empty());
    assert!(!last.has_more);
}

#[test]
fn redaction_covers_prefixed_keys_cookies_and_url_credentials_idempotently() {
    for input in [
        "EXAMPLE_API_KEY=DUMMY_SECRET",
        "Cookie: session=DUMMY_SECRET; theme=dark",
        "Set-Cookie: session=DUMMY_SECRET; HttpOnly",
        "https://user:DUMMY_SECRET@example.invalid/path",
        r#"{\"EXAMPLE_API_KEY\":\"DUMMY_SECRET\"}"#,
        r#"password="prefix DUMMY_SECRET""#,
        "password='prefix DUMMY_SECRET'",
        r#"{"password":"prefix DUMMY_SECRET"}"#,
        r#"{\"password\":\"prefix DUMMY_SECRET\"}"#,
        r#"Cookie: session="DUMMY_SECRET"; theme=dark"#,
    ] {
        let redacted = redact_credential_text(input);
        assert!(!redacted.contains("DUMMY_SECRET"), "{input} -> {redacted}");
        assert_eq!(redact_credential_text(&redacted), redacted);
    }
    let value = redact_value(
        &serde_json::json!({"EXAMPLE_API_KEY": "DUMMY_SECRET", "Cookie": "DUMMY_COOKIE"}),
    );
    assert!(!value.to_string().contains("DUMMY_"));
    assert_eq!(
        redact_credential_text("ordinary diagnostic: timeout 30s"),
        "ordinary diagnostic: timeout 30s"
    );
}

/// Provider-shaped keys pasted into prose/commands carry no `key=` label.
/// Legacy local telemetry contained such `sk-` values (2026-11-04 audit).
/// Synthetic tokens are assembled at runtime so no key-shaped literal exists.
#[test]
fn redaction_masks_unlabelled_provider_shaped_tokens() {
    let body = "A1b2C3d4E5f6G7h8".repeat(3);
    for token in [
        format!("sk-{body}"),
        format!("sk-proj-{body}"),
        format!("sk-ant-api03-{body}-{body}"),
        format!("sk-{}-{}-x9", &body[..16], &body[..6]),
        format!("ghp_{body}"),
        format!("github_pat_{body}_{body}"),
        format!("AKIA{}", "ABCDEFGH23456789"),
        format!("xoxb-{}-{}", "123456789012", &body[..24]),
        format!("AIza{}", &body[..35]),
        format!("glpat-{}", &body[..20]),
    ] {
        for input in [
            format!("export OPENAI={token} && run"),
            format!("curl -H 'x: {token}' https://example.invalid"),
            format!(r#"{{"args":"use {token} please"}}"#),
        ] {
            let redacted = redact_credential_text(&input);
            assert!(!redacted.contains(&token), "{input} -> {redacted}");
            assert_eq!(redact_credential_text(&redacted), redacted);
        }
    }
    for ordinary in [
        "pip install sk-learn-compatible-estimator-wrapper",
        "task-0123456789abcdefghij0123 is done",
        "desk-ABCDEFGHIJ0123456789xyz",
        "the AKIA prefix and ghp_ prefix are documented",
    ] {
        assert_eq!(redact_credential_text(ordinary), ordinary);
    }
}

#[test]
fn legacy_reads_and_mirror_exports_are_sanitized_without_rewriting_history() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir);
    let rec = recorder(&store, "legacy");
    rec.handle_agent_event(&user("ordinary diagnostic"));
    let run_id = rec.run_id();
    // Simulate old data written before redaction covered prefixed credentials.
    store.with_conn(|conn, _| {
        conn.execute("UPDATE observe_events SET payload_json = replace(payload_json, 'ordinary diagnostic', 'EXAMPLE_API_KEY=DUMMY_LEGACY') WHERE run_id = ?1", params![run_id]).map_err(sql_err)?;
        Ok(())
    }).unwrap();
    assert!(
        !serde_json::to_string(&store.get_events(&run_id, None, None).unwrap())
            .unwrap()
            .contains("DUMMY_LEGACY")
    );
    assert!(
        !serde_json::to_string(&store.get_state(&run_id, None).unwrap())
            .unwrap()
            .contains("DUMMY_LEGACY")
    );
    assert!(store
        .get_stored_events(&run_id, 0, 100)
        .unwrap()
        .iter()
        .all(|event| !event.payload.contains("DUMMY_LEGACY")));
    store
        .with_conn(|conn, _| {
            let count: i64 = conn
                .query_row(
                    "SELECT count(*) FROM observe_events WHERE payload_json LIKE '%DUMMY_LEGACY%'",
                    [],
                    |row| row.get(0),
                )
                .map_err(sql_err)?;
            assert!(count > 0, "read sanitization must not mutate history");
            Ok(())
        })
        .unwrap();
}

#[test]
fn completed_memory_survives_restart_as_terminal() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir);
    let rec = recorder(&store, "memory");
    let run_id = rec.run_id();
    let mut observation = crate::cognee::observer::Observation {
        operation: "warmup_replay".into(),
        request_id: Some("one".into()),
        phase: "started".into(),
        ..Default::default()
    };
    rec.handle_memory(&observation);
    observation.phase = "completed".into();
    rec.handle_memory(&observation);
    rec.handle_memory(&observation);
    rec.stop();
    drop(rec);
    drop(store);
    let store = open(&dir);
    let state = store.get_state(&run_id, None).unwrap().unwrap();
    assert_eq!(state.items.len(), 1);
    assert_eq!(state.items[0].state, Some(ObserveItemState::Done));
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

#[cfg(unix)]
#[test]
fn reopening_an_existing_store_restricts_database_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("observability.sqlite");
    let store = ObserveRunStore::open(&path).unwrap();
    store.close().unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

    let store = ObserveRunStore::open(&path).unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    store.close().unwrap();
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
fn durable_payloads_and_live_state_are_equally_redacted() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir);
    let rec = recorder(&store, "s");
    rec.handle_agent_event(&AgentEvent::ToolExecutionStart {
        tool_call_id: "t".into(),
        tool_name: "bash".into(),
        args: serde_json::json!({ "command": "curl -H 'Authorization: Bearer sk-live-123' x", "api_key": "abc" }),
    });
    let live = rec.get_state().items[0].args.clone().unwrap();
    assert!(!live.contains("sk-live-123"));
    let stored = store.get_state(&rec.run_id(), None).unwrap().unwrap();
    let args = stored.items[0].args.clone().unwrap();
    assert!(!args.contains("sk-live-123"), "{args}");
    assert!(!args.contains("\"abc\""), "{args}");
    assert!(args.contains("[REDACTED]"));
    assert_eq!(live, args);
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
