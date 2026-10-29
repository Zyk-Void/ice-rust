//! Recorder behavior tests. Mirror `ice:packages/coding-agent/test/observe-recorder.test.ts`
//! as described by `docs/ice-gap/observability.md` OBS-004..OBS-010.

use super::*;
use ice_agent::message::CustomMessage;
use ice_ai::types::{
    Api, ImageContent, ImageContentType, TextContent, TextContentType, UserMessage,
};

#[derive(Default)]
struct MemorySink {
    events: Mutex<Vec<ObserveRecorderEvent>>,
    runs: Mutex<Vec<ObserveHeader>>,
    finished: Mutex<Vec<String>>,
    fail: bool,
}

impl ObservePersistence for MemorySink {
    fn record_run(&self, header: &ObserveHeader) -> Result<(), String> {
        self.runs.lock().unwrap().push(header.clone());
        Ok(())
    }
    fn record_event(&self, event: &ObserveRecorderEvent, _: &ObserveHeader) -> Result<(), String> {
        if self.fail {
            return Err("disk full".into());
        }
        self.events.lock().unwrap().push(event.clone());
        Ok(())
    }
    fn finish_run(&self, run_id: &str, _: i64) -> Result<(), String> {
        self.finished.lock().unwrap().push(run_id.into());
        Ok(())
    }
    fn close(&self) -> Result<(), String> {
        Ok(())
    }
}

fn recorder_with(sink: Option<Arc<MemorySink>>) -> ObserveRecorder {
    ObserveRecorder::new(ObserveRecorderOptions {
        session_id: "0123456789abcdef".into(),
        info: ObserveSessionInfo {
            model: Some("faux/faux-1".into()),
            thinking_level: Some("off".into()),
            context_window: Some(1000),
            ..Default::default()
        },
        persistence: sink.map(|s| s as Arc<dyn ObservePersistence>),
    })
}

fn user(text: &str) -> AgentEvent {
    AgentEvent::MessageStart {
        message: AgentMessage::User(UserMessage::new(text.to_string(), 0)),
    }
}

fn assistant(text: &str, stop: StopReason, error: Option<&str>) -> AgentEvent {
    let mut message = AssistantMessage::empty(Api::AnthropicMessages, "faux", "faux-1", 0);
    if !text.is_empty() {
        message.content.push(Content::text(text));
    }
    message.stop_reason = stop;
    message.error_message = error.map(str::to_string);
    message.usage.input = 100;
    message.usage.output = 20;
    message.usage.cache_read = 5;
    message.usage.cache_write = 1;
    message.usage.cost.total = 0.25;
    AgentEvent::MessageEnd {
        message: AgentMessage::Assistant(Box::new(message)),
    }
}

fn text_result(text: &str) -> AgentToolResult {
    AgentToolResult {
        content: vec![TextContentOrImage::Text(TextContent {
            kind: TextContentType,
            text: text.into(),
            text_signature: None,
        })],
        details: serde_json::Value::Null,
        usage: None,
        added_tool_names: Vec::new(),
        terminate: false,
    }
}

fn tool_start(id: &str) -> AgentEvent {
    AgentEvent::ToolExecutionStart {
        tool_call_id: id.into(),
        tool_name: "bash".into(),
        args: serde_json::json!({ "command": "ls" }),
    }
}

fn tool_update(id: &str) -> AgentEvent {
    AgentEvent::ToolExecutionUpdate {
        tool_call_id: id.into(),
        tool_name: "bash".into(),
        args: serde_json::json!({}),
        partial_result: Arc::new(text_result("partial")),
    }
}

fn tool_end(id: &str, result: AgentToolResult, is_error: bool) -> AgentEvent {
    AgentEvent::ToolExecutionEnd {
        tool_call_id: id.into(),
        tool_name: "bash".into(),
        result,
        is_error,
    }
}

#[test]
fn identity_header_and_main_thread() {
    let sink = Arc::new(MemorySink::default());
    let recorder = recorder_with(Some(sink.clone()));
    let state = recorder.get_state();
    assert!(state.header.run_id.starts_with("0123456789abcdef-"));
    assert_eq!(state.header.main_thread_name, "main · 0123456789");
    assert_eq!(state.header.model.as_deref(), Some("faux/faux-1"));
    assert_eq!(state.threads.len(), 1);
    assert_eq!(
        state.threads[0].identity.thread_color,
        ObserveThreadColor::Blue
    );
    // Initial persistence: the run plus the main-thread event.
    assert_eq!(sink.runs.lock().unwrap().len(), 1);
    assert!(matches!(
        sink.events.lock().unwrap()[0],
        ObserveRecorderEvent::Thread { .. }
    ));
    // Two recorders for one session never share a run id.
    assert_ne!(recorder_with(None).run_id(), recorder.run_id());
}

#[test]
fn ring_keeps_the_newest_500_items_with_increasing_ids() {
    let recorder = recorder_with(None);
    for i in 0..(MAX_ITEMS + 7) {
        recorder.handle_agent_event(&user(&format!("m{i}")));
    }
    let items = recorder.get_state().items;
    assert_eq!(items.len(), MAX_ITEMS);
    assert_eq!(items[0].id, 8);
    assert_eq!(items.last().unwrap().id, (MAX_ITEMS + 7) as u64);
    assert_eq!(items.last().unwrap().text.as_deref(), Some("m506"));
}

#[test]
fn long_strings_are_bounded_with_an_omitted_suffix() {
    let long = "x".repeat(MAX_PAYLOAD_CHARS + 10);
    let bounded = limit_payload(long);
    assert!(bounded.ends_with("[10 chars omitted]"));
    assert_eq!(limit_payload("short".into()), "short");
}

#[test]
fn user_messages_and_skill_invocations() {
    let recorder = recorder_with(None);
    recorder.handle_agent_event(&user("hello"));
    recorder.handle_agent_event(&user(
        "<skill name=\"deploy\" location=\"/s/SKILL.md\">\nReferences are relative to /s.\n\nbody\n</skill>\n\nship it",
    ));
    recorder.handle_agent_event(&user(
        "<skill name=\"lint\" location=\"/l/SKILL.md\">\nbody\n</skill>",
    ));
    let items = recorder.get_state().items;
    assert_eq!(items[0].kind, ObserveItemKind::Message);
    assert_eq!(items[0].role, Some(ObserveRole::User));
    assert_eq!(items[0].text.as_deref(), Some("hello"));
    assert_eq!(items[1].kind, ObserveItemKind::Skill);
    assert_eq!(items[1].text.as_deref(), Some("/deploy ship it"));
    assert_eq!(items[2].text.as_deref(), Some("/lint"));
    assert!(items[0].raw.is_some());
}

#[test]
fn assistant_usage_accumulates_and_failures_get_a_row() {
    let recorder = recorder_with(None);
    let mut live = recorder.subscribe();
    recorder.handle_agent_event(&assistant("answer", StopReason::Stop, None));
    recorder.handle_agent_event(&assistant("", StopReason::Stop, None));
    recorder.handle_agent_event(&assistant("", StopReason::Error, Some("boom")));
    recorder.handle_agent_event(&assistant("partial", StopReason::Aborted, None));
    let state = recorder.get_state();
    assert_eq!(state.header.usage.input, 400);
    assert_eq!(state.header.usage.output, 80);
    assert_eq!(state.header.usage.cache_read, 20);
    assert_eq!(state.header.usage.cache_write, 4);
    assert!((state.header.usage.cost - 1.0).abs() < 1e-9);
    let context = state.header.context.unwrap();
    assert_eq!(context.tokens, Some(126));
    assert_eq!(context.context_window, 1000);
    // Empty successful text adds no row; error/aborted always do.
    let texts: Vec<_> = state
        .items
        .iter()
        .map(|i| i.text.clone().unwrap())
        .collect();
    assert_eq!(texts, ["answer", "[error] boom", "[aborted]\npartial"]);
    assert_eq!(state.items[0].state, None);
    assert_eq!(state.items[1].state, Some(ObserveItemState::Error));
    // A header follows each assistant end.
    let mut headers = 0;
    while let Ok(event) = live.try_recv() {
        if matches!(event, ObserveRecorderEvent::Header { .. }) {
            headers += 1;
        }
    }
    assert_eq!(headers, 4);
}

#[test]
fn tool_lifecycle_is_one_row_and_progress_is_not_persisted() {
    let sink = Arc::new(MemorySink::default());
    let recorder = recorder_with(Some(sink.clone()));
    recorder.handle_agent_event(&tool_start("t1"));
    recorder.handle_agent_event(&tool_update("t1"));
    recorder.handle_agent_event(&tool_update("t1"));
    recorder.handle_agent_event(&tool_end("t1", text_result("out\nput"), false));
    let items = recorder.get_state().items;
    assert_eq!(items.len(), 1);
    let tool = &items[0];
    assert_eq!(tool.kind, ObserveItemKind::Tool);
    assert_eq!(tool.state, Some(ObserveItemState::Done));
    assert_eq!(tool.updates, Some(2));
    assert_eq!(tool.result.as_deref(), Some("out\nput"));
    assert!(tool
        .args
        .as_deref()
        .unwrap()
        .contains("\"command\": \"ls\""));
    assert!(tool.duration_ms.unwrap() >= 0);
    // Text-only result keeps no redundant raw copy.
    assert!(tool.raw.is_none());

    let events = sink.events.lock().unwrap();
    let kinds: Vec<_> = events.iter().map(|e| e.type_tag()).collect();
    // thread, start activity, final update, header — no progress ticks.
    assert_eq!(kinds, ["thread", "activity", "activity_update", "header"]);
}

#[test]
fn unmatched_tool_end_creates_a_terminal_row_and_errors_are_marked() {
    let recorder = recorder_with(None);
    recorder.handle_agent_event(&tool_update("ghost"));
    let mut result = text_result("");
    result.content = vec![TextContentOrImage::Image(ImageContent {
        kind: ImageContentType,
        data: "AAAA".into(),
        mime_type: "image/png".into(),
    })];
    result.details = serde_json::json!({ "exit": 1 });
    recorder.handle_agent_event(&tool_end("ghost", result, true));
    let items = recorder.get_state().items;
    assert_eq!(items.len(), 1, "progress without a start is ignored");
    assert_eq!(items[0].state, Some(ObserveItemState::Error));
    assert_eq!(items[0].duration_ms, Some(0));
    let raw = items[0].raw.as_deref().unwrap();
    assert!(raw.contains("image/png") && raw.contains("\"exit\": 1"));
    assert!(!raw.contains("AAAA"), "image bytes are not retained");
}

#[test]
fn retry_compaction_and_queue_rows() {
    let recorder = recorder_with(None);
    recorder.handle_agent_event(&AgentEvent::RetryScheduled {
        attempt: 1,
        max_retries: 3,
        delay_ms: 2000,
        error: "overloaded".into(),
    });
    recorder.handle_agent_event(&AgentEvent::RetryRefused {
        error: "tool ran".into(),
    });
    recorder.handle_compaction(false, true);
    recorder.handle_compaction(false, false);
    recorder.handle_queue_update(2, 1);
    let items = recorder.get_state().items;
    let texts: Vec<_> = items.iter().map(|i| i.text.clone().unwrap()).collect();
    assert_eq!(
        texts,
        [
            "attempt 1/3 in 2000ms",
            "retry refused: tool ran",
            "started (threshold)",
            "finished (threshold)",
            "2 steering, 1 follow-up",
        ]
    );
    assert!(items[0].raw.as_deref().unwrap().contains("overloaded"));
    assert_eq!(items[2].kind, ObserveItemKind::Compaction);
    assert_eq!(items[4].kind, ObserveItemKind::Queue);
}

fn observation(request_id: &str, phase: &str) -> Observation {
    Observation {
        operation: "recall".into(),
        phase: phase.into(),
        request_id: Some(request_id.into()),
        dataset: Some("ice".into()),
        ..Default::default()
    }
}

#[test]
fn memory_title_reads_outcomes_from_every_supported_version() {
    for version in [1, 2] {
        let mut done = observation("r", "succeeded");
        done.meta = serde_json::json!({"outcomeVersion": version, "outcome": "retrieved_empty"})
            .as_object()
            .cloned();
        assert!(
            memory_title(&done).contains("retrieved empty"),
            "v{version}"
        );
    }
    let mut unknown = observation("r", "succeeded");
    unknown.meta = serde_json::json!({"outcomeVersion": 9, "outcome": "retrieved_empty"})
        .as_object()
        .cloned();
    assert!(!memory_title(&unknown).contains("retrieved empty"));
}

#[test]
fn memory_terminal_phase_updates_its_started_item() {
    let sink = Arc::new(MemorySink::default());
    let recorder = recorder_with(Some(sink.clone()));
    let mut started = observation("r1", "started");
    started.preview = Some("what changed?".into());
    recorder.handle_memory(&started);
    let mut done = observation("r1", "succeeded");
    done.latency_ms = Some(42);
    done.meta = Some(serde_json::Map::from_iter([(
        "checkpoint".into(),
        serde_json::Value::Bool(true),
    )]));
    recorder.handle_memory(&done);
    let mut failed = observation("r2", "failed");
    failed.error = Some("timeout".into());
    recorder.handle_memory(&failed);

    let items = recorder.get_state().items;
    assert_eq!(items.len(), 2, "terminal phase reuses the started item");
    assert_eq!(items[0].kind, ObserveItemKind::Memory);
    assert_eq!(items[0].tool_name.as_deref(), Some("recall"));
    assert_eq!(items[0].state, Some(ObserveItemState::Done));
    assert_eq!(items[0].args.as_deref(), Some("what changed?"));
    assert_eq!(
        items[0].text.as_deref(),
        Some("Cognee recall (compaction) · ice")
    );
    assert_eq!(items[0].duration_ms, Some(42));
    assert_eq!(items[1].state, Some(ObserveItemState::Error));
    assert_eq!(items[1].result.as_deref(), Some("error: timeout"));
    assert!(sink.events.lock().unwrap().iter().any(
        |e| matches!(e, ObserveRecorderEvent::ActivityUpdate { item } if item.id == items[0].id)
    ));
}

#[test]
fn memory_recorder_preserves_timeout_abort_and_late_outcomes() {
    let recorder = recorder_with(None);
    for (id, phase, outcome) in [
        ("timeout", "failed", "timeout"),
        ("abort", "failed", "aborted"),
        ("late", "succeeded", "stale"),
    ] {
        recorder.handle_memory(&observation(id, "started"));
        let mut terminal = observation(id, phase);
        terminal.meta = serde_json::json!({"outcomeVersion": 1, "outcome": outcome})
            .as_object()
            .cloned();
        if phase == "failed" {
            terminal.error = Some(outcome.into());
        }
        recorder.handle_memory(&terminal);
    }
    let items = recorder.get_state().items;
    assert_eq!(items.len(), 3);
    assert_eq!(items[0].state, Some(ObserveItemState::Error));
    assert!(items[0]
        .result
        .as_deref()
        .unwrap()
        .starts_with("error: timeout"));
    assert_eq!(items[1].state, Some(ObserveItemState::Error));
    assert!(items[1]
        .result
        .as_deref()
        .unwrap()
        .starts_with("error: aborted"));
    assert!(items[2]
        .text
        .as_deref()
        .unwrap()
        .contains("discarded (session changed)"));
}

#[test]
fn memory_operations_with_shared_correlation_do_not_finish_each_other() {
    let recorder = recorder_with(None);
    recorder.handle_memory(&observation("shared", "started"));
    let mut injection = observation("shared", "succeeded");
    injection.operation = "injection".into();
    recorder.handle_memory(&injection);
    recorder.handle_memory(&observation("shared", "succeeded"));
    let items = recorder.get_state().items;
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].tool_name.as_deref(), Some("recall"));
    assert_eq!(items[0].state, Some(ObserveItemState::Done));
    assert_eq!(items[1].tool_name.as_deref(), Some("injection"));
}

#[test]
fn memory_titles_distinguish_versioned_outcomes_without_reinterpreting_legacy() {
    let recorder = recorder_with(None);
    for (id, operation, outcome) in [
        ("skip", "recall", "skipped"),
        ("empty", "recall", "retrieved_empty"),
        ("hit", "recall", "retrieved_nonempty"),
        ("job", "improve", "accepted"),
        ("stored", "injection", "persisted"),
        ("failed", "injection", "append_failed"),
    ] {
        let mut started = observation(id, "started");
        started.operation = operation.into();
        recorder.handle_memory(&started);
        let mut terminal = observation(id, "succeeded");
        terminal.operation = operation.into();
        terminal.meta = serde_json::json!({"outcomeVersion": 1, "outcome": outcome})
            .as_object()
            .cloned();
        recorder.handle_memory(&terminal);
    }
    recorder.handle_memory(&observation("legacy", "succeeded"));
    let items = recorder.get_state().items;
    assert!(items[0]
        .text
        .as_deref()
        .unwrap()
        .contains("skipped (no request)"));
    assert!(items[1]
        .text
        .as_deref()
        .unwrap()
        .contains("retrieved empty"));
    assert!(items[2]
        .text
        .as_deref()
        .unwrap()
        .contains("retrieved nonempty"));
    assert!(items[3]
        .text
        .as_deref()
        .unwrap()
        .contains("accepted (completion unknown)"));
    assert!(items[4]
        .text
        .as_deref()
        .unwrap()
        .contains("persisted (delivery unknown)"));
    assert!(items[5].text.as_deref().unwrap().contains("append failed"));
    assert_eq!(items[6].text.as_deref(), Some("Cognee recall · ice"));
}

#[test]
fn goal_transitions_are_deduplicated_and_cleared() {
    let recorder = recorder_with(None);
    let goal = |runs: u64, input: u64| {
        serde_json::json!({ "goals": [{
            "id": 7, "status": "active", "objective": "ship",
            "runsUsed": runs, "maxRuns": 5, "criteriaHash": "abcdef0123456789",
            "usage": { "input": input },
            "tasks": [{ "status": "complete", "subtasks": [{ "status": "open" }] }],
        }]})
    };
    recorder.handle_custom_entry("ice-goals-state", &goal(1, 10));
    // Usage-only rewrite: same summary, no row.
    recorder.handle_custom_entry("ice-goals-state", &goal(1, 99));
    recorder.handle_custom_entry("ice-goals-state", &goal(2, 99));
    recorder.handle_custom_entry("ice-goals-state", &serde_json::json!({}));
    let items = recorder.get_state().items;
    assert_eq!(items.len(), 3);
    assert_eq!(
        items[0].text.as_deref(),
        Some("goal active · tasks 1/2 · runs 1/5 · reviews 0/3 · criteria abcdef012345 · 7")
    );
    assert_eq!(items[0].result.as_deref(), Some("ship"));
    assert_eq!(items[0].state, Some(ObserveItemState::Running));
    assert_eq!(items[2].text.as_deref(), Some("goal cleared · 7"));
    // Unknown custom types are ignored.
    recorder.handle_custom_entry("other", &serde_json::json!({ "x": 1 }));
    assert_eq!(recorder.get_state().items.len(), 3);
}

#[test]
fn verifier_transitions_dedupe_on_status_exit_and_mode() {
    let recorder = recorder_with(None);
    let verifier = |status: &str, stdout: &str| {
        serde_json::json!({ "mode": "build", "verifier": {
            "status": status, "exitCode": 1, "stdout": stdout, "durationMs": 42,
        }})
    };
    recorder.handle_custom_entry("ice-safe-verify-state", &verifier("failed", "a"));
    recorder.handle_custom_entry("ice-safe-verify-state", &verifier("failed", "b"));
    recorder.handle_custom_entry("ice-safe-verify-state", &verifier("passed", "c"));
    let items = recorder.get_state().items;
    assert_eq!(items.len(), 2);
    assert_eq!(
        items[0].text.as_deref(),
        Some("verifier failed · exit 1 · mode build")
    );
    assert_eq!(items[0].state, Some(ObserveItemState::Error));
    assert_eq!(items[0].duration_ms, Some(42));
    assert_eq!(items[1].state, Some(ObserveItemState::Done));
}

#[test]
fn subagent_children_get_their_own_threads() {
    let recorder = recorder_with(None);
    let mut live = recorder.subscribe();
    let child = |state: &str| {
        serde_json::json!({ "id": "child-1", "name": "scout", "agent": "explore",
            "state": state, "description": "look around" })
    };
    recorder.handle_custom_entry("ice-subagent-v2-child", &child("running"));
    recorder.handle_custom_entry("ice-subagent-v2-child", &child("running"));
    recorder.handle_custom_entry(
        "ice-subagent-v2-completion",
        &serde_json::json!({ "childId": "child-1", "agent": "explore", "outcome": "failed", "summary": "no" }),
    );
    recorder.handle_custom_entry(
        "ice-subagent-v2-completion",
        &serde_json::json!({ "childId": "child-2", "outcome": "completed" }),
    );
    let state = recorder.get_state();
    assert_eq!(
        state.items.len(),
        3,
        "child records dedupe; completions do not"
    );
    let first = &state.items[0];
    assert_eq!(first.identity.thread_key, "0123456789abcdef::child-1");
    assert_eq!(first.identity.thread_kind, ObserveThreadKind::Subagent);
    assert_eq!(
        first.identity.parent_thread_id.as_deref(),
        Some("0123456789abcdef")
    );
    assert_eq!(first.text.as_deref(), Some("running — look around"));
    assert_eq!(first.state, Some(ObserveItemState::Running));
    assert_eq!(state.items[1].state, Some(ObserveItemState::Error));
    assert_eq!(state.items[2].identity.thread_name, "child-2");
    assert_eq!(state.threads.len(), 3);
    assert_eq!(
        state.threads[1].identity.thread_color,
        ObserveThreadColor::Violet
    );
    assert_eq!(
        state.threads[2].identity.thread_color,
        ObserveThreadColor::Cyan
    );
    assert_eq!(state.threads[1].role.as_deref(), Some("explore"));
    let thread_events = std::iter::from_fn(|| live.try_recv().ok())
        .filter(|e| matches!(e, ObserveRecorderEvent::Thread { .. }))
        .count();
    assert_eq!(thread_events, 2);
}

#[test]
fn custom_messages_are_projected_regardless_of_display() {
    let recorder = recorder_with(None);
    recorder.handle_agent_event(&AgentEvent::MessageStart {
        message: AgentMessage::Custom(CustomMessage::new(
            "ice-goals-note",
            vec![Content::text("goal note")],
            serde_json::json!({ "display": false }),
            0,
        )),
    });
    let items = recorder.get_state().items;
    assert_eq!(items[0].kind, ObserveItemKind::Goal);
    assert_eq!(items[0].role, Some(ObserveRole::Custom));
    assert_eq!(items[0].tool_name.as_deref(), Some("ice-goals-note"));
    assert_eq!(items[0].text.as_deref(), Some("goal note"));
}

#[test]
fn persistence_failures_do_not_block_live_delivery() {
    let sink = Arc::new(MemorySink {
        fail: true,
        ..Default::default()
    });
    let recorder = recorder_with(Some(sink));
    let mut live = recorder.subscribe();
    recorder.handle_agent_event(&user("still delivered"));
    assert!(matches!(
        live.try_recv(),
        Ok(ObserveRecorderEvent::Activity { .. })
    ));
}

#[test]
fn unchanged_session_info_emits_no_header_and_stop_is_idempotent() {
    let sink = Arc::new(MemorySink::default());
    let recorder = recorder_with(Some(sink.clone()));
    let mut live = recorder.subscribe();
    let info = ObserveSessionInfo {
        model: Some("faux/faux-1".into()),
        thinking_level: Some("off".into()),
        context_window: Some(1000),
        ..Default::default()
    };
    recorder.update_session_info(info.clone());
    assert!(live.try_recv().is_err());
    recorder.update_session_info(ObserveSessionInfo {
        session_name: Some("  named  ".into()),
        ..info
    });
    match live.try_recv() {
        Ok(ObserveRecorderEvent::Header { header }) => {
            assert_eq!(header.main_thread_name, "named")
        }
        other => panic!("expected header, got {other:?}"),
    }
    recorder.stop();
    recorder.stop();
    assert_eq!(sink.finished.lock().unwrap().len(), 1);
    recorder.handle_agent_event(&user("after stop"));
    assert!(recorder.get_state().items.is_empty());
}
