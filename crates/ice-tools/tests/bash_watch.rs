//! Mirrors `packages/coding-agent/test/bash-watch.test.ts`: pattern watches
//! across output chunks, fire budgets and ids, rearming, silence/prompt/
//! interval timers, automatic defaults, removal when a shell stops, and the
//! bash_watch tool's validation.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use ice_agent::agent_tool::AgentTool;
use ice_agent::error::AgentError;
use ice_agent::types::{AgentToolResult, TextContentOrImage};
use ice_tools::bash_background::BashBackgroundRegistry;
use ice_tools::bash_watch::{
    create_bash_watch_tool, BashWatchDefaults, BashWatchFire, BashWatchKind, BashWatchRearm,
    BashWatchRegistry, BashWatchSpec,
};
use ice_tools::{BashToolOptions, ExecutionToolContext, InMemoryExecutionEnv, ShellScript};
use tokio_util::sync::CancellationToken;

struct Harness {
    env: Arc<InMemoryExecutionEnv>,
    shells: Arc<BashBackgroundRegistry>,
    watches: Arc<BashWatchRegistry>,
    bash: Arc<dyn AgentTool>,
    tool: Arc<dyn AgentTool>,
    fires: Arc<Mutex<Vec<BashWatchFire>>>,
}

fn harness(defaults: Option<BashWatchDefaults>) -> Harness {
    let env = Arc::new(InMemoryExecutionEnv::with_cwd("/w".into()));
    let env_dyn: Arc<dyn ice_tools::ExecutionEnv> = env.clone();
    let shells = BashBackgroundRegistry::new();
    let watches = BashWatchRegistry::new(
        shells.clone(),
        defaults.map(|d| Arc::new(move || d) as Arc<dyn Fn() -> BashWatchDefaults + Send + Sync>),
    );
    let fires = Arc::new(Mutex::new(Vec::new()));
    let sink = fires.clone();
    watches.on_fire(move |f| sink.lock().unwrap().push(f.clone()));
    let bash = ice_tools::create_bash_tool(
        &ExecutionToolContext::new(env_dyn, None),
        Some(BashToolOptions {
            background: Some(shells.clone()),
            ..Default::default()
        }),
    );
    let tool = create_bash_watch_tool(watches.clone());
    Harness {
        env,
        shells,
        watches,
        bash,
        tool,
        fires,
    }
}

async fn start(h: &Harness, command: &str) -> String {
    let r = h
        .bash
        .execute(
            "c",
            serde_json::json!({ "command": command, "run_in_background": true }),
            CancellationToken::new(),
            Arc::new(|_| ()),
        )
        .await
        .expect("launch");
    r.details["background_task_id"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn watch_tool(h: &Harness, params: serde_json::Value) -> Result<AgentToolResult, AgentError> {
    h.tool
        .execute("w", params, CancellationToken::new(), Arc::new(|_| ()))
        .await
}

fn text(r: &AgentToolResult) -> String {
    match &r.content[0] {
        TextContentOrImage::Text(t) => t.text.clone(),
        _ => String::new(),
    }
}

fn err(e: AgentError) -> String {
    match e {
        AgentError::Tool(m) => m,
        other => format!("{other:?}"),
    }
}

#[tokio::test]
async fn pattern_matches_across_chunks_once_and_spends_its_budget() {
    let h = harness(None);
    // "ready" is split between the first and the second output chunk.
    h.env
        .register_shell(
            "dev",
            ShellScript {
                stdout: "dy on :3000\nready again\n".into(),
                ..ShellScript::long_running("rea", 150)
            },
        )
        .await;
    let id = start(&h, "dev").await;
    let r = watch_tool(
        &h,
        serde_json::json!({ "action": "create", "shell": id, "kind": "pattern", "pattern": "ready" }),
    )
    .await
    .expect("created");
    assert!(text(&r).starts_with("Created watch-1 on bash-1: pattern \"ready\", fired 0/1"));
    tokio::time::sleep(Duration::from_millis(400)).await;
    let fires = h.fires.lock().unwrap().clone();
    // Two occurrences in one chunk still produce one alert; the budget of 1
    // removes the watch.
    assert_eq!(fires.len(), 1, "{fires:?}");
    assert_eq!(fires[0].fire_id, "watch-1#1");
    assert_eq!(fires[0].kind, BashWatchKind::Pattern);
    assert!(!fires[0].auto);
    assert!(fires[0].reason.contains("ready"), "{}", fires[0].reason);
    assert!(fires[0].tail.contains("on :3000"), "{}", fires[0].tail);
    assert!(fires[0].log_path.is_some());
    assert!(h.watches.list(None).is_empty());
}

#[tokio::test]
async fn watches_end_when_their_shell_stops() {
    let h = harness(None);
    h.env
        .register_shell("job", ShellScript::long_running("", 100))
        .await;
    let id = start(&h, "job").await;
    watch_tool(
        &h,
        serde_json::json!({ "action": "create", "shell": id, "kind": "pattern", "pattern": "x" }),
    )
    .await
    .unwrap();
    assert_eq!(h.watches.list(None).len(), 1);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(h.watches.list(None).is_empty());
    let e = watch_tool(
        &h,
        serde_json::json!({ "action": "create", "shell": id, "kind": "pattern", "pattern": "x" }),
    )
    .await
    .unwrap_err();
    assert!(err(e).contains("watches only apply to running shells"));
}

#[tokio::test]
async fn interval_fires_repeatedly_until_its_budget_is_used() {
    let h = harness(None);
    h.env
        .register_shell("job", ShellScript::long_running("", 5_000))
        .await;
    let id = start(&h, "job").await;
    let watch = h
        .watches
        .create(BashWatchSpec {
            target: id,
            kind: BashWatchKind::Interval,
            pattern: None,
            period: Some(Duration::from_millis(40)),
            max_fires: 3,
            rearm: BashWatchRearm::Manual,
        })
        .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let fires = h.fires.lock().unwrap().clone();
    let ids: Vec<&str> = fires.iter().map(|f| f.fire_id.as_str()).collect();
    assert_eq!(
        ids,
        vec![
            format!("{}#1", watch.id),
            format!("{}#2", watch.id),
            format!("{}#3", watch.id)
        ]
    );
    assert!(h.watches.list(None).is_empty());
    h.shells.dispose().await;
}

#[tokio::test]
async fn automatic_prompt_and_silence_defaults() {
    let h = harness(Some(BashWatchDefaults {
        silence_alert: Duration::from_millis(80),
        prompt_alert: Duration::from_millis(60),
    }));
    h.env
        .register_shell(
            "ask",
            ShellScript::long_running("Overwrite file? [y/N] ", 5_000),
        )
        .await;
    start(&h, "ask").await;
    let auto: Vec<_> = h.watches.list(None);
    assert_eq!(auto.len(), 2);
    assert_eq!(auto[0].kind, BashWatchKind::Prompt);
    assert_eq!(auto[1].kind, BashWatchKind::Silence);
    assert!(auto.iter().all(|w| w.auto && w.max_fires == 3));
    tokio::time::sleep(Duration::from_millis(300)).await;
    let fires = h.fires.lock().unwrap().clone();
    let kinds: Vec<BashWatchKind> = fires.iter().map(|f| f.kind).collect();
    assert_eq!(
        kinds,
        vec![BashWatchKind::Prompt, BashWatchKind::Silence],
        "{fires:?}"
    );
    assert!(
        fires[0].reason.contains("waiting for input"),
        "{}",
        fires[0].reason
    );
    assert!(
        fires[1].reason.starts_with("No output from bash-1 for"),
        "{}",
        fires[1].reason
    );
    assert!(fires.iter().all(|f| f.auto));
    // Both disarmed after firing; automatic watches do not count toward the
    // explicit limit and wait to rearm on activity.
    assert!(h.watches.list(None).iter().all(|w| !w.armed));
    h.shells.dispose().await;
}

#[tokio::test]
async fn a_non_prompt_last_line_keeps_polling_without_spending_budget() {
    let h = harness(Some(BashWatchDefaults {
        silence_alert: Duration::ZERO,
        prompt_alert: Duration::from_millis(40),
    }));
    h.env
        .register_shell("build", ShellScript::long_running("Compiling foo\n", 5_000))
        .await;
    start(&h, "build").await;
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert!(h.fires.lock().unwrap().is_empty());
    let watches = h.watches.list(None);
    assert_eq!(watches.len(), 1);
    assert_eq!(watches[0].fires, 0);
    assert!(watches[0].armed);
    h.shells.dispose().await;
}

#[tokio::test]
async fn manual_rearm_keeps_the_lifetime_count() {
    let h = harness(None);
    h.env
        .register_shell("job", ShellScript::long_running("", 5_000))
        .await;
    let id = start(&h, "job").await;
    let watch = h
        .watches
        .create(BashWatchSpec {
            target: id,
            kind: BashWatchKind::Silence,
            pattern: None,
            period: Some(Duration::from_millis(40)),
            max_fires: 3,
            rearm: BashWatchRearm::Manual,
        })
        .unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(h.fires.lock().unwrap().len(), 1);
    // Disarmed (manual): no further alerts until rearmed.
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(h.fires.lock().unwrap().len(), 1);
    let r = watch_tool(
        &h,
        serde_json::json!({ "action": "rearm", "watch_id": watch.id }),
    )
    .await
    .unwrap();
    assert!(text(&r).contains("fired 1/3, armed"), "{}", text(&r));
    // Already overdue: fires at once.
    tokio::time::sleep(Duration::from_millis(50)).await;
    let fires = h.fires.lock().unwrap().clone();
    assert_eq!(fires.len(), 2);
    assert_eq!(fires[1].fire_id, format!("{}#2", watch.id));
    h.shells.dispose().await;
}

#[tokio::test]
async fn bash_watch_tool_validation_and_listing() {
    let h = harness(None);
    h.env
        .register_shell("job", ShellScript::long_running("", 5_000))
        .await;
    let id = start(&h, "job").await;
    let r = watch_tool(&h, serde_json::json!({ "action": "list" }))
        .await
        .unwrap();
    assert_eq!(text(&r), "No watches.");
    let cases = [
        (
            serde_json::json!({ "action": "create", "shell": id, "kind": "silence", "silence_seconds": 1 }),
            "silence_seconds must be at least 5.",
        ),
        (
            serde_json::json!({ "action": "create", "shell": id, "kind": "interval", "interval_seconds": 10 }),
            "interval_seconds must be at least 30.",
        ),
        (
            serde_json::json!({ "action": "create", "shell": id, "kind": "pattern", "pattern": "" }),
            "pattern must be a non-empty string.",
        ),
        (
            serde_json::json!({ "action": "create", "shell": id, "kind": "pattern", "pattern": "x", "max_fires": 11 }),
            "max_fires must be an integer from 1 to 10.",
        ),
        (
            serde_json::json!({ "action": "rearm", "watch_id": "watch-9" }),
            "Unknown watch \"watch-9\". A watch that used all its fires is removed.",
        ),
    ];
    for (params, expected) in cases {
        let e = watch_tool(&h, params).await.unwrap_err();
        assert_eq!(err(e), expected);
    }
    watch_tool(
        &h,
        serde_json::json!({ "action": "create", "shell": id, "kind": "silence", "silence_seconds": 60 }),
    )
    .await
    .unwrap();
    let r = watch_tool(&h, serde_json::json!({ "action": "list", "shell": id }))
        .await
        .unwrap();
    assert_eq!(
        text(&r),
        "watch-1 on bash-1: silence 60s, fired 0/3, armed, rearms on activity"
    );
    let r = watch_tool(
        &h,
        serde_json::json!({ "action": "cancel", "watch_id": "watch-1" }),
    )
    .await
    .unwrap();
    assert_eq!(text(&r), "Cancelled watch-1.");
    assert_eq!(r.details["watches"], serde_json::json!([]));
    h.shells.dispose().await;
}
