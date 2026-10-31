//! Analytics aggregation tests, over a fake fact source and over facts the
//! SQLite store derives from real recorder events.

use super::*;
use crate::observe::recorder::{
    ObservePersistence, ObserveRecorder, ObserveRecorderOptions, ObserveSessionInfo,
};
use crate::observe::store::ObserveRunStore;
use ice_agent::types::{AgentToolResult, TextContentOrImage};
use ice_agent::{AgentEvent, AgentMessage};
use ice_ai::types::{Api, AssistantMessage, Content, StopReason, TextContent, TextContentType};
use std::sync::Arc;

/// Facts held in memory; window filtering matches the real backends.
struct FakeSource {
    runs: Vec<RunFact>,
    items: Vec<ItemFact>,
}

impl AnalyticsSource for FakeSource {
    fn name(&self) -> &'static str {
        "fake"
    }
    fn runs_between(&self, since: i64, until: i64) -> Result<Vec<RunFact>, String> {
        let mut runs: Vec<RunFact> = self
            .runs
            .iter()
            .filter(|r| r.summary.started_at >= since && r.summary.started_at < until)
            .cloned()
            .collect();
        runs.sort_by(|a, b| b.summary.started_at.cmp(&a.summary.started_at));
        Ok(runs)
    }
    fn items_between(&self, since: i64, until: i64) -> Result<Vec<ItemFact>, String> {
        Ok(self
            .items
            .iter()
            .filter(|i| i.ts >= since && i.ts < until)
            .cloned()
            .collect())
    }
    fn run_state(&self, _: &str) -> Result<Option<ObserveState>, String> {
        Ok(None)
    }
}

fn run(id: &str, started: i64, ended: Option<i64>, model: &str) -> RunFact {
    RunFact {
        summary: ObserveRunSummary {
            run_id: id.into(),
            session_id: format!("s-{id}"),
            main_thread_id: format!("t-{id}"),
            session_name: None,
            model: Some(model.into()),
            thinking_level: None,
            started_at: started,
            ended_at: ended,
            status: if ended.is_some() {
                ObserveRunStatus::Finished
            } else {
                ObserveRunStatus::Running
            },
        },
        last_activity: ended,
    }
}

fn tool(run: &str, ts: i64, name: &str, ms: i64, error: bool) -> ItemFact {
    ItemFact {
        run_id: run.into(),
        ts,
        kind: ObserveItemKind::Tool,
        tool_name: Some(name.into()),
        state: Some(if error {
            ObserveItemState::Error
        } else {
            ObserveItemState::Done
        }),
        duration_ms: Some(ms),
        subagent: false,
        path: None,
        turn: None,
        prompt: None,
    }
}

fn edit(run: &str, ts: i64, name: &str, path: &str) -> ItemFact {
    ItemFact {
        path: Some(path.into()),
        ..tool(run, ts, name, 5, false)
    }
}

fn turn(run: &str, ts: i64, model: &str, input: i64, cache_read: i64, stop: &str) -> ItemFact {
    ItemFact {
        run_id: run.into(),
        ts,
        kind: ObserveItemKind::Message,
        tool_name: None,
        state: None,
        duration_ms: None,
        subagent: false,
        path: None,
        turn: Some(TurnFact {
            model: Some(model.into()),
            input,
            output: 10,
            cache_read,
            cache_write: 0,
            cost: 0.5,
            stop_reason: Some(stop.into()),
        }),
        prompt: None,
    }
}

/// Twenty minutes past an hour, so hour buckets start before it.
const T0: i64 = 1_800_000_000_000 + 20 * 60_000;

fn fixture() -> FakeSource {
    FakeSource {
        runs: vec![
            run("a", T0, Some(T0 + 60_000), "p/m1"),
            run(
                "b",
                T0 + 2 * HOUR_MS,
                Some(T0 + 2 * HOUR_MS + 180_000),
                "p/m2",
            ),
            // Before the window: only in `previous`.
            run("old", T0 - 2 * HOUR_MS, Some(T0 - HOUR_MS), "p/m1"),
        ],
        items: vec![
            ItemFact {
                kind: ObserveItemKind::Message,
                prompt: Some("fix the   flaky\n test".into()),
                ..tool("a", T0 + 500, "-", 0, false)
            },
            turn("a", T0 + 1_000, "p/m1", 100, 300, "toolUse"),
            tool("a", T0 + 2_000, "bash", 10, false),
            tool("a", T0 + 3_000, "bash", 30, true),
            tool("a", T0 + 4_000, "read", 20, false),
            edit("a", T0 + 5_000, "edit", "src/lib.rs"),
            edit("a", T0 + 6_000, "edit", "src/lib.rs"),
            turn("b", T0 + 2 * HOUR_MS + 1_000, "p/m2", 50, 0, "error"),
            edit("b", T0 + 2 * HOUR_MS + 2_000, "write", "src/lib.rs"),
            ItemFact {
                subagent: true,
                ..tool("b", T0 + 2 * HOUR_MS + 3_000, "bash", 40, false)
            },
            tool("old", T0 - 2 * HOUR_MS + 1_000, "bash", 99, true),
        ],
    }
}

#[test]
fn overview_totals_compare_with_the_previous_window() {
    let source = fixture();
    let overview = overview(
        &source,
        &OverviewQuery {
            since: T0,
            until: T0 + 4 * HOUR_MS,
            bucket_ms: None,
            tz_offset_min: 0,
        },
    )
    .unwrap();
    assert_eq!(overview.source, "fake");
    assert_eq!(overview.bucket_ms, HOUR_MS);
    let totals = &overview.totals;
    assert_eq!(
        (totals.runs, totals.turns, totals.turn_errors),
        (2, 2, 1),
        "stopReason error counts as a failed turn"
    );
    assert_eq!((totals.tool_calls, totals.tool_errors), (7, 1));
    assert_eq!(totals.subagent_items, 1);
    assert_eq!(
        (
            totals.tokens.input,
            totals.tokens.cache_read,
            totals.tokens.output
        ),
        (150, 300, 20)
    );
    assert!((totals.tokens.cost - 1.0).abs() < 1e-9);
    // (a, src/lib.rs) and (b, src/lib.rs) are separate changes.
    assert_eq!(totals.files_changed, 2);
    assert_eq!(totals.median_run_ms, Some(60_000));
    assert_eq!(totals.p95_tool_ms, Some(40));
    assert_eq!(
        (overview.previous.runs, overview.previous.tool_errors),
        (1, 1)
    );
}

#[test]
fn overview_breaks_down_series_models_tools_and_files() {
    let source = fixture();
    let overview = overview(
        &source,
        &OverviewQuery {
            since: T0,
            until: T0 + 4 * HOUR_MS,
            bucket_ms: Some(HOUR_MS),
            tz_offset_min: 0,
        },
    )
    .unwrap();
    // The series starts at the hour boundary before T0.
    assert_eq!(
        overview.series.first().unwrap().t,
        T0 - T0.rem_euclid(HOUR_MS)
    );
    assert_eq!(overview.series.len(), 5);
    let busy: Vec<(u64, u64, u64)> = overview
        .series
        .iter()
        .map(|p| (p.runs, p.turns, p.tool_calls))
        .collect();
    assert_eq!(busy.iter().map(|b| b.0).sum::<u64>(), 2);
    assert_eq!(busy.iter().map(|b| b.2).sum::<u64>(), 7);

    let models: Vec<(&str, u64, u64)> = overview
        .models
        .iter()
        .map(|m| (m.model.as_str(), m.turns, m.errors))
        .collect();
    assert_eq!(models, [("p/m1", 1, 0), ("p/m2", 1, 1)]);

    let bash = overview.tools.iter().find(|t| t.tool == "bash").unwrap();
    assert_eq!((bash.calls, bash.errors, bash.runs), (3, 1, 2));
    assert_eq!(
        (bash.p50_ms, bash.p95_ms, bash.max_ms),
        (Some(30), Some(40), Some(40))
    );
    assert_eq!(bash.total_ms, 80);
    assert_eq!(overview.tools[0].tool, "bash", "sorted by calls");

    assert_eq!(overview.files.len(), 1);
    let file = &overview.files[0];
    assert_eq!(
        (file.path.as_str(), file.edits, file.writes, file.runs),
        ("src/lib.rs", 2, 1, 2)
    );
    assert_eq!(overview.kinds.get("tool"), Some(&7));
    assert_eq!(overview.kinds.get("message"), Some(&3));
    assert_eq!(overview.stop_reasons.get("error"), Some(&1));
    assert_eq!(overview.heatmap.iter().flatten().sum::<u64>(), 9);
}

#[test]
fn heatmap_and_day_buckets_use_the_viewer_time_zone() {
    // 2027-01-04 is a Monday; 23:30 UTC is Tuesday 01:30 at UTC+2.
    let monday = 1_799_020_800_000;
    let ts = monday + 23 * HOUR_MS + 30 * 60_000;
    let source = FakeSource {
        runs: vec![],
        items: vec![tool("a", ts, "bash", 1, false)],
    };
    let query = |tz| OverviewQuery {
        since: monday,
        until: monday + 2 * DAY_MS,
        bucket_ms: Some(DAY_MS),
        tz_offset_min: tz,
    };
    let utc = overview(&source, &query(0)).unwrap();
    assert_eq!(utc.heatmap[0][23], 1);
    assert_eq!(utc.series[0].tool_calls, 1);
    let plus2 = overview(&source, &query(120)).unwrap();
    assert_eq!(plus2.heatmap[1][1], 1);
    // Local midnight at UTC+2 is 22:00 UTC the day before.
    assert_eq!(plus2.series[0].t, monday - 2 * HOUR_MS);
    assert_eq!(plus2.series[1].tool_calls, 1);
}

#[test]
fn bucket_width_is_bounded() {
    assert_eq!(bucket_for(DAY_MS, None), HOUR_MS);
    assert_eq!(bucket_for(7 * DAY_MS, None), 6 * HOUR_MS);
    assert_eq!(bucket_for(30 * DAY_MS, None), DAY_MS);
    // A tiny requested bucket over a long span is widened to MAX_BUCKETS.
    let span = 365 * DAY_MS;
    let bucket = bucket_for(span, Some(60_000));
    assert!(span / bucket <= MAX_BUCKETS);
}

#[test]
fn overview_rejects_extreme_windows_and_bounds_aligned_series() {
    let source = FakeSource {
        runs: Vec::new(),
        items: Vec::new(),
    };
    for (since, until) in [(0, i64::MAX), (i64::MAX, i64::MAX), (-1, 1), (10, 5)] {
        assert!(overview(
            &source,
            &OverviewQuery {
                since,
                until,
                bucket_ms: None,
                tz_offset_min: 0,
            }
        )
        .is_err());
    }
    for (since, until, bucket) in [(1, 400 * 60_000 + 1, 60_000), (0, MAX_TIMESTAMP, i64::MAX)] {
        let result = overview(
            &source,
            &OverviewQuery {
                since,
                until,
                bucket_ms: Some(bucket),
                tz_offset_min: -1440,
            },
        )
        .unwrap();
        assert!(result.series.len() <= MAX_BUCKETS as usize);
        assert!(!result.series.is_empty());
    }
}

#[test]
fn run_list_rolls_up_each_run() {
    let source = fixture();
    let list = run_list(&source, T0, T0 + DAY_MS).unwrap();
    let ids: Vec<&str> = list
        .runs
        .iter()
        .map(|r| r.summary.run_id.as_str())
        .collect();
    assert_eq!(ids, ["b", "a"], "newest first");
    let a = &list.runs[1];
    assert_eq!(a.duration_ms, Some(60_000));
    assert_eq!(
        (a.turns, a.tool_calls, a.tool_errors, a.files),
        (1, 5, 1, 1)
    );
    assert_eq!(a.models, ["p/m1"]);
    assert_eq!(a.prompt.as_deref(), Some("fix the   flaky\n test"));
    assert_eq!(list.runs[0].prompt, None);
    assert_eq!(
        a.top_tools
            .iter()
            .map(|t| (t.name.as_str(), t.count))
            .collect::<Vec<_>>(),
        [("bash", 2), ("edit", 2), ("read", 1)]
    );
    assert_eq!(list.runs[0].turn_errors, 1);
    assert_eq!(list.runs[0].subagent_items, 1);
}

#[test]
fn prompt_labels_are_one_bounded_line() {
    assert_eq!(
        prompt_label("  fix the   flaky\n test "),
        Some("fix the flaky test".into())
    );
    assert_eq!(prompt_label(" \n\t "), None);
    let long = prompt_label(&"é".repeat(PROMPT_LABEL_CHARS + 5)).unwrap();
    assert_eq!(long.chars().count(), PROMPT_LABEL_CHARS + 1);
    assert!(long.ends_with('…'));
}

#[test]
fn turn_usage_tolerates_missing_or_truncated_messages() {
    assert_eq!(TurnFact::from_raw(None), TurnFact::default());
    assert_eq!(
        TurnFact::from_raw(Some("{\"usage\": {\"input\": 3")),
        TurnFact::default()
    );
    let turn = TurnFact::from_raw(Some(
        r#"{"provider":"omni","model":"cx/m","stopReason":"stop",
            "usage":{"input":1,"output":2,"cacheRead":3,"cacheWrite":4,"cost":{"total":0.5}}}"#,
    ));
    assert_eq!(turn.model.as_deref(), Some("omni/cx/m"));
    assert_eq!(
        (turn.input, turn.output, turn.cache_read, turn.cache_write),
        (1, 2, 3, 4)
    );
    assert_eq!(turn.stop_reason.as_deref(), Some("stop"));
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

#[test]
fn sqlite_facts_fold_each_activity_to_its_final_state() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(ObserveRunStore::open(&dir.path().join("o.sqlite")).unwrap());
    let recorder = ObserveRecorder::new(ObserveRecorderOptions {
        session_id: "sess".into(),
        info: ObserveSessionInfo {
            model: Some("faux/faux-1".into()),
            ..Default::default()
        },
        persistence: Some(Arc::clone(&store) as Arc<dyn ObservePersistence>),
    });
    recorder.handle_agent_event(&AgentEvent::ToolExecutionStart {
        tool_call_id: "c1".into(),
        tool_name: "edit".into(),
        args: serde_json::json!({
            "path": "src/main.rs",
            "edits": [{ "oldText": "a", "newText": "b" }],
        }),
    });
    recorder.handle_agent_event(&AgentEvent::ToolExecutionEnd {
        tool_call_id: "c1".into(),
        tool_name: "edit".into(),
        result: text_result("ok"),
        is_error: false,
    });
    let mut message = AssistantMessage::empty(Api::Faux, "faux", "faux-1", 0);
    message.content.push(Content::text("done"));
    message.stop_reason = StopReason::Stop;
    message.usage.input = 7;
    message.usage.cache_read = 70;
    message.usage.cost.total = 0.01;
    recorder.handle_agent_event(&AgentEvent::MessageEnd {
        message: AgentMessage::Assistant(Box::new(message)),
    });
    let run_id = recorder.run_id();
    recorder.stop();

    let items = store.items_between(0, i64::MAX).unwrap();
    let tools: Vec<&ItemFact> = items.iter().filter(|i| i.is_tool()).collect();
    assert_eq!(
        tools.len(),
        1,
        "start and end fold into one fact: {items:?}"
    );
    assert_eq!(tools[0].state, Some(ObserveItemState::Done));
    assert!(tools[0].duration_ms.is_some());
    assert_eq!(tools[0].path.as_deref(), Some("src/main.rs"));
    let turn = items.iter().find_map(|i| i.turn.as_ref()).unwrap();
    assert_eq!(turn.model.as_deref(), Some("faux/faux-1"));
    assert_eq!((turn.input, turn.cache_read), (7, 70));
    assert_eq!(turn.stop_reason.as_deref(), Some("stop"));

    let runs = store.runs_between(0, i64::MAX).unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].summary.run_id, run_id);
    assert!(runs[0].last_activity.is_some());
    let list = run_list(store.as_ref(), 0, i64::MAX).unwrap();
    assert_eq!(list.source, "sqlite");
    assert_eq!((list.runs[0].tool_calls, list.runs[0].files), (1, 1));
    assert!(store
        .items_between(i64::MAX - 1, i64::MAX)
        .unwrap()
        .is_empty());
}
