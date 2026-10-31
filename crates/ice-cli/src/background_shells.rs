//! A session's background shells: the registry `bash` and the control tools
//! share, the watch registry, delivery of completions and watch alerts into
//! the conversation, and shutdown. Mirrors the session integration of
//! ice's `bash-background.ts`/`bash-watch.ts` (SESSION-023/025).
//!
//! Completions and alerts are sent to the main lane as custom messages that
//! steer with `triggerTurn`: while the agent is running they are read at the
//! next tool boundary, not only when the whole run ends; when it is idle they
//! start a turn so the model can react. A user stop is recorded for the next
//! turn instead of waking the model. A completion whose result the model
//! already read with `bash_output` is not sent.

use std::future::Future;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tokio::task::JoinHandle;

use ice_harness::agent_harness::{AgentHarness, WeakAgentHarness};
use ice_harness::types::HarnessTool;
use ice_harness::types::{MessageDelivery, SendMessageOptions};
use ice_tools::bash_background::{
    BashBackgroundCompletion, BashBackgroundRegistry, SteerProbe,
    BASH_BACKGROUND_COMPLETED_CUSTOM_TYPE,
};
use ice_tools::bash_watch::{
    create_bash_watch_tool, BashWatchDefaults, BashWatchFire, BashWatchRegistry,
    BASH_WATCH_FIRED_CUSTOM_TYPE,
};
use ice_tools::tools::bash_shells::{
    create_bash_shell_tools, BashInputApproval, BashShellToolOptions,
};

/// Default automatic prompt alert when the setting is absent.
const DEFAULT_PROMPT_ALERT_SECS: f64 = 30.0;

#[derive(Default)]
struct DeliveryState {
    closed: bool,
    tasks: Vec<JoinHandle<()>>,
}

#[derive(Default)]
struct DeliveryTasks(Mutex<DeliveryState>);

impl DeliveryTasks {
    fn spawn(&self, delivery: impl Future<Output = ()> + Send + 'static) {
        let mut state = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if !state.closed {
            state.tasks.push(tokio::spawn(delivery));
        }
    }

    async fn close_and_drain(&self) {
        let tasks = {
            let mut state = self.0.lock().unwrap_or_else(|error| error.into_inner());
            state.closed = true;
            std::mem::take(&mut state.tasks)
        };
        for task in tasks {
            let _ = task.await;
        }
    }
}

fn setting_duration(value: Option<f64>, default: f64) -> Duration {
    let secs = value.unwrap_or(default);
    Duration::try_from_secs_f64(secs)
        .ok()
        .filter(|duration| !duration.is_zero())
        .unwrap_or(Duration::ZERO)
}

/// One session's background shells and watches.
pub struct SessionShells {
    registry: Arc<BashBackgroundRegistry>,
    watches: Arc<BashWatchRegistry>,
    approval: BashInputApproval,
    auto_background_secs: Option<f64>,
    deliveries: Arc<DeliveryTasks>,
    /// Set by `connect`; the tools are built before the harness exists.
    harness: Arc<OnceLock<WeakAgentHarness>>,
}

impl SessionShells {
    pub fn new(settings: &crate::settings::Settings) -> Arc<Self> {
        let defaults = BashWatchDefaults {
            prompt_alert: setting_duration(
                settings.bash_prompt_alert_seconds,
                DEFAULT_PROMPT_ALERT_SECS,
            ),
            silence_alert: setting_duration(settings.bash_silence_alert_seconds, 0.0),
        };
        let registry = BashBackgroundRegistry::new();
        let watches = BashWatchRegistry::new(
            registry.clone(),
            Some(Arc::new(move || defaults) as Arc<dyn Fn() -> BashWatchDefaults + Send + Sync>),
        );
        let approval = match settings.bash_input_approval.as_deref() {
            Some("allow") => BashInputApproval::Allow,
            _ => BashInputApproval::Ask,
        };
        Arc::new(Self {
            registry,
            watches,
            approval,
            auto_background_secs: settings
                .bash_auto_background_seconds
                .filter(|s| *s > 0.0 && Duration::try_from_secs_f64(*s).is_ok()),
            deliveries: Arc::new(DeliveryTasks::default()),
            harness: Arc::default(),
        })
    }

    /// The registry handed to the `bash` tool.
    pub fn registry(&self) -> Arc<BashBackgroundRegistry> {
        self.registry.clone()
    }

    pub fn auto_background_secs(&self) -> Option<f64> {
        self.auto_background_secs
    }

    /// True while a steering message waits on the connected harness: one
    /// the user sent mid-run, or a background notice. `bash` then moves a
    /// slow foreground command to the background and `bash_output` stops
    /// waiting, so the model reads it at the next tool boundary.
    pub fn steer_probe(&self) -> SteerProbe {
        let harness = self.harness.clone();
        Arc::new(move || {
            harness
                .get()
                .and_then(WeakAgentHarness::upgrade)
                .is_some_and(|harness| harness.has_queued_steering())
        })
    }

    /// Call `f(running shells, watches)` now and whenever either changes, for
    /// the footer's shell count. Only watches the model created count; the
    /// automatic alerts every shell gets are not news. The listeners hold the
    /// registries weakly, so they do not keep them alive.
    pub fn watch_counts(&self, f: impl Fn(usize, usize) + Send + Sync + 'static) {
        let (registry, watches) = (
            Arc::downgrade(&self.registry),
            Arc::downgrade(&self.watches),
        );
        let report = Arc::new(move || {
            if let (Some(registry), Some(watches)) = (registry.upgrade(), watches.upgrade()) {
                let explicit = watches.list(None).iter().filter(|w| !w.auto).count();
                f(registry.running_count(), explicit);
            }
        });
        report();
        let on_shell = report.clone();
        self.registry.on_change(move || on_shell());
        self.watches.on_change(move || report());
    }

    /// `bash_output`, `bash_kill`, `bash_input` and `bash_watch`, named for
    /// the session's tool filters.
    pub fn tools(&self) -> Vec<(&'static str, HarnessTool)> {
        let mut tools: Vec<(&'static str, HarnessTool)> = create_bash_shell_tools(
            self.registry.clone(),
            BashShellToolOptions {
                input_approval: self.approval,
                steer_probe: Some(self.steer_probe()),
                ..Default::default()
            },
        )
        .into_iter()
        .zip(["bash_output", "bash_kill", "bash_input"])
        .map(|(tool, name)| (name, HarnessTool::new(tool)))
        .collect();
        tools.push((
            "bash_watch",
            HarnessTool::new(create_bash_watch_tool(self.watches.clone())),
        ));
        tools
    }

    /// Deliver completions and watch alerts to `harness`'s main lane. Holds
    /// only a weak reference, so the harness can still be dropped.
    pub fn connect(&self, harness: &AgentHarness) {
        let _ = self.harness.set(harness.downgrade());
        let weak = harness.downgrade();
        let deliveries = self.deliveries.clone();
        // Weak: the registry owns this listener.
        let registry = Arc::downgrade(&self.registry);
        self.registry.on_complete(move |completion| {
            if let Some(harness) = weak.upgrade() {
                let completion = completion.clone();
                let registry = registry.clone();
                deliveries.spawn(async move {
                    // Checked at send time, so a read between the shell's end
                    // and now still counts.
                    if registry
                        .upgrade()
                        .is_some_and(|r| r.result_delivered(&completion.task_id))
                    {
                        return;
                    }
                    let options = completion_delivery(&completion);
                    deliver(harness, completion_message(&completion), options).await;
                });
            }
        });
        let weak = harness.downgrade();
        let deliveries = self.deliveries.clone();
        self.watches.on_fire(move |fire| {
            if let Some(harness) = weak.upgrade() {
                let message = fire_message(fire);
                deliveries.spawn(deliver(harness, message, steer()));
            }
        });
    }

    /// Move every running foreground bash call to the background (ctrl+b).
    pub fn release_foreground(&self) -> usize {
        self.registry.release_all()
    }

    /// Stop every background shell and remove its log. Call at session end:
    /// background shells run in their own process groups and would otherwise
    /// outlive icers.
    pub async fn dispose(&self) {
        self.watches.dispose();
        self.registry.dispose().await;
        self.deliveries.close_and_drain().await;
    }
}

async fn deliver(
    harness: AgentHarness,
    message: ice_agent::AgentMessage,
    options: SendMessageOptions,
) {
    if let Err(error) = harness.lane("main").send_message(message, options).await {
        tracing::warn!("background shell notice was not delivered: {error}");
    }
}

/// Steer, not follow-up: a busy agent (often waiting on another shell) learns
/// at the next tool boundary instead of only when its whole run ends, after it
/// may have answered already. An idle agent starts a turn.
fn steer() -> SendMessageOptions {
    SendMessageOptions {
        trigger_turn: Some(true),
        deliver_as: MessageDelivery::Steer,
    }
}

/// A user stop is deliberate: record it for the next turn instead of waking
/// the model to react. Mirrors ice's `deliverAs: "nextTurn"`.
fn completion_delivery(c: &BashBackgroundCompletion) -> SendMessageOptions {
    if c.stopped_by_user {
        SendMessageOptions {
            trigger_turn: None,
            deliver_as: MessageDelivery::NextTurn,
        }
    } else {
        steer()
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn completion_message(c: &BashBackgroundCompletion) -> ice_agent::AgentMessage {
    let status = if c.stopped_by_user {
        "was stopped by the user".to_string()
    } else if c.is_error {
        "failed".to_string()
    } else {
        "completed".to_string()
    };
    // Named "bash" like every shell; the description says which one.
    let command = ice_tools::bash_background::summarize_shell_command(&c.command);
    let what = match c.description.as_deref().filter(|d| !d.is_empty()) {
        Some(description) => format!("{description} ({command})"),
        None => command,
    };
    let mut text = format!(
        "Background bash {} {status} after {}s: {what}",
        c.task_id,
        c.duration_ms / 1000,
    );
    if c.lingering {
        text.push_str(&format!(
            "\nIt left processes running; stop them with bash_kill (id \"{}\").",
            c.task_id
        ));
    }
    if let Some(issue) = &c.log_issue {
        text.push_str(&format!("\n[Warning: full output {issue}]"));
    }
    text.push_str(&format!("\n\n{}", c.text));
    ice_harness::messages::create_custom_message(
        BASH_BACKGROUND_COMPLETED_CUSTOM_TYPE,
        ice_ai::types::UserContent::Text(text),
        true,
        serde_json::to_value(c).ok(),
        now_ms(),
    )
}

fn fire_message(fire: &BashWatchFire) -> ice_agent::AgentMessage {
    let mut text = format!(
        "Watch {} on background bash {} ({}{}): {}",
        fire.watch_id,
        fire.target,
        fire.kind.as_str(),
        if fire.auto { ", automatic" } else { "" },
        fire.reason
    );
    if !fire.tail.is_empty() {
        text.push_str(&format!("\n\nRecent output:\n{}", fire.tail));
    }
    ice_harness::messages::create_custom_message(
        BASH_WATCH_FIRED_CUSTOM_TYPE,
        ice_ai::types::UserContent::Text(text),
        true,
        serde_json::to_value(fire).ok(),
        now_ms(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn closing_delivery_tasks_waits_for_started_notices() {
        let deliveries = Arc::new(DeliveryTasks::default());
        let (finish_tx, finish_rx) = tokio::sync::oneshot::channel::<()>();
        let (done_tx, done_rx) = tokio::sync::oneshot::channel::<()>();
        deliveries.spawn(async move {
            let _ = finish_rx.await;
            let _ = done_tx.send(());
        });

        let draining = {
            let deliveries = deliveries.clone();
            tokio::spawn(async move { deliveries.close_and_drain().await })
        };
        tokio::task::yield_now().await;
        assert!(!draining.is_finished());
        finish_tx.send(()).unwrap();
        draining.await.unwrap();
        assert!(done_rx.await.is_ok());

        deliveries.spawn(async {});
        assert!(deliveries.0.lock().unwrap().tasks.is_empty());
    }

    #[tokio::test]
    async fn watch_counts_reports_now_and_on_every_shell_change() {
        let shells = SessionShells::new(&Default::default());
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = seen.clone();
        shells.watch_counts(move |running, watches| sink.lock().unwrap().push((running, watches)));
        assert_eq!(*seen.lock().unwrap(), [(0, 0)]);

        let env = Arc::new(ice_tools::InMemoryExecutionEnv::with_cwd("/w".into()));
        env.register_shell("server", ice_tools::ShellScript::long_running("", 60_000))
            .await;
        let env_dyn: Arc<dyn ice_tools::ExecutionEnv> = env;
        let bash = ice_tools::create_bash_tool(
            &ice_tools::ExecutionToolContext::new(env_dyn, None),
            Some(ice_tools::BashToolOptions {
                background: Some(shells.registry()),
                ..Default::default()
            }),
        );
        let started = bash
            .execute(
                "c",
                serde_json::json!({ "command": "server", "run_in_background": true }),
                tokio_util::sync::CancellationToken::new(),
                Arc::new(|_| ()),
            )
            .await
            .unwrap();
        assert_eq!(seen.lock().unwrap().last(), Some(&(1, 0)));
        let id = started.details["background_task_id"].as_str().unwrap();
        shells.registry().kill(id, false).await.unwrap();
        assert_eq!(seen.lock().unwrap().last(), Some(&(0, 0)));
        shells.dispose().await;
    }

    #[test]
    fn tools_and_settings() {
        let settings: crate::settings::Settings = serde_json::from_str(
            r#"{"bashInputApproval":"allow","bashAutoBackgroundSeconds":90,"bashPromptAlertSeconds":0}"#,
        )
        .unwrap();
        let shells = SessionShells::new(&settings);
        let names: Vec<&str> = shells.tools().iter().map(|(name, _)| *name).collect();
        assert_eq!(
            names,
            vec!["bash_output", "bash_kill", "bash_input", "bash_watch"]
        );
        assert_eq!(shells.approval, BashInputApproval::Allow);
        assert_eq!(shells.auto_background_secs(), Some(90.0));
        let defaults = SessionShells::new(&Default::default());
        assert_eq!(defaults.approval, BashInputApproval::Ask);
        assert_eq!(defaults.auto_background_secs(), None);
    }

    #[test]
    fn unrepresentable_alert_and_auto_background_durations_are_ignored() {
        let settings: crate::settings::Settings = serde_json::from_str(
            r#"{"bashAutoBackgroundSeconds":1e300,"bashPromptAlertSeconds":1e300,"bashSilenceAlertSeconds":1e300}"#,
        )
        .unwrap();
        let shells = SessionShells::new(&settings);
        assert_eq!(shells.auto_background_secs(), None);
        assert_eq!(setting_duration(Some(1e300), 30.0), Duration::ZERO);
        assert_eq!(setting_duration(Some(1e300), 0.0), Duration::ZERO);
    }

    fn text_of(message: &ice_agent::AgentMessage) -> String {
        match message {
            ice_agent::AgentMessage::Custom(c) => c
                .content
                .iter()
                .filter_map(|b| match b {
                    ice_ai::types::Content::Text(t) => Some(t.text.clone()),
                    _ => None,
                })
                .collect(),
            _ => String::new(),
        }
    }

    #[test]
    fn completion_and_fire_messages() {
        let completion = BashBackgroundCompletion {
            task_id: "b0tests42".into(),
            command: "npm test\nnpm run lint".into(),
            text: "3 passed".into(),
            is_error: false,
            duration_ms: 4_200,
            exit_code: Some(0),
            description: Some("tests".into()),
            log_path: None,
            log_issue: None,
            lingering: true,
            stopped_by_user: false,
        };
        let text = text_of(&completion_message(&completion));
        assert!(
            text.starts_with(
                "Background bash b0tests42 completed after 4s: tests (npm test (+1 more lines))"
            ),
            "{text}"
        );
        assert!(text.contains("left processes running"), "{text}");
        assert!(text.ends_with("\n\n3 passed"), "{text}");
        let fire = BashWatchFire {
            watch_id: "watch-1".into(),
            target: "b0tests42".into(),
            kind: ice_tools::bash_watch::BashWatchKind::Pattern,
            reason: "Output matched \"ready\": ready on :3000".into(),
            tail: "ready on :3000".into(),
            log_path: None,
            auto: false,
            fire_id: "watch-1#1".into(),
        };
        let text = text_of(&fire_message(&fire));
        assert!(
            text.starts_with(
                "Watch watch-1 on background bash b0tests42 (pattern): Output matched"
            ),
            "{text}"
        );
        assert!(text.ends_with("Recent output:\nready on :3000"), "{text}");
    }
}

#[cfg(test)]
mod delivery_tests {
    use super::*;
    use ice_ai::providers::faux::{
        faux_assistant_message, FauxBlock, FauxProvider, FauxScript, FauxStep,
    };
    use ice_harness::agent_harness::AgentLane;
    use ice_harness::types::AgentHarnessOptions;
    use serde_json::json;
    use std::time::Instant;

    /// The model's turns, in order.
    fn script(steps: Vec<FauxStep>) -> FauxScript {
        let script = FauxScript::new();
        script.set_responses(steps);
        script
    }

    fn launch(command: &str) -> FauxStep {
        FauxStep::tool_call(
            "bash",
            json!({"command": command, "run_in_background": true}),
        )
    }

    /// `bash_output` waiting on the shell that the `nth` launch started; the
    /// id is random, so it is read from that launch's result.
    fn wait_on(nth: usize) -> FauxStep {
        FauxStep::factory(move |context| {
            let id = context
                .messages
                .iter()
                .filter_map(|message| match message {
                    ice_ai::types::Message::ToolResult(result) => {
                        result.content.iter().find_map(|content| match content {
                            ice_ai::types::Content::Text(text) => text
                                .text
                                .strip_prefix("Started background bash ")
                                .and_then(|rest| rest.split(':').next())
                                .map(str::to_string),
                            _ => None,
                        })
                    }
                    _ => None,
                })
                .nth(nth)
                .expect("the shell was launched");
            faux_assistant_message(
                vec![FauxBlock::tool_call(
                    "bash_output",
                    json!({"id": id, "wait_seconds": 30}),
                )],
                ice_ai::types::StopReason::ToolUse,
            )
        })
    }

    /// Run `script` with `bash` and the shell tools. Commands: `slow` runs
    /// 200 ms, `busy` 600 ms, `long` 3 s. Returns the conversation as
    /// `user`, `assistant`, `tool:<name>` or `notice`, the number of model
    /// calls, and how long the run took.
    async fn conversation(script: FauxScript) -> (Vec<String>, usize, Duration) {
        let provider = FauxProvider::new(script);
        let env = Arc::new(ice_tools::InMemoryExecutionEnv::with_cwd("/w".into()));
        for (name, ms) in [("slow", 200), ("busy", 600), ("long", 3_000)] {
            env.register_shell(name, ice_tools::ShellScript::long_running("working\n", ms))
                .await;
        }
        let shells = SessionShells::new(&Default::default());
        let context = ice_tools::ExecutionToolContext::new(env, None);
        let bash = ice_tools::create_bash_tool(
            &context,
            Some(ice_tools::BashToolOptions {
                background: Some(shells.registry()),
                steer_probe: Some(shells.steer_probe()),
                ..Default::default()
            }),
        );
        let mut tools = vec![HarnessTool::new(bash)];
        tools.extend(shells.tools().into_iter().map(|(_, tool)| tool));
        let options = AgentHarnessOptions {
            model: provider.default_model().clone(),
            models: vec![provider.clone()],
            active_tool_names: tools
                .iter()
                .map(|tool| tool.tool.schema().name.clone())
                .collect(),
            tools,
            ..Default::default()
        };
        let harness = AgentHarness::create(options).await.unwrap();
        shells.connect(&harness);
        let started = Instant::now();
        harness.prompt_text("go", vec![]).await.unwrap();
        let elapsed = started.elapsed();
        // Lets every notice finish sending, including one that would start
        // another run.
        shells.dispose().await;
        let messages = harness.context_snapshot().await.unwrap().context.messages;
        let labels = messages
            .iter()
            .map(|message| match message {
                ice_agent::AgentMessage::User(_) => "user".to_string(),
                ice_agent::AgentMessage::Assistant(_) => "assistant".to_string(),
                ice_agent::AgentMessage::ToolResult(result) => format!("tool:{}", result.tool_name),
                ice_agent::AgentMessage::Custom(_) => "notice".to_string(),
            })
            .collect();
        let calls = provider
            .state()
            .call_count
            .load(std::sync::atomic::Ordering::SeqCst);
        (labels, calls, elapsed)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_result_the_model_waited_for_is_not_sent_again() {
        let (labels, calls, _) = conversation(script(vec![
            launch("slow"),
            wait_on(0),
            FauxStep::text("done"),
        ]))
        .await;
        assert_eq!(
            labels,
            [
                "user",
                "assistant",
                "tool:bash",
                "assistant",
                "tool:bash_output",
                "assistant"
            ]
        );
        assert_eq!(calls, 3, "no turn was started for a repeated result");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_unread_result_arrives_at_the_next_tool_boundary() {
        let (labels, calls, _) = conversation(script(vec![
            launch("slow"),
            FauxStep::tool_call("bash", json!({"command": "busy"})),
            FauxStep::text("done"),
        ]))
        .await;
        assert_eq!(
            labels,
            [
                "user",
                "assistant",
                "tool:bash",
                "assistant",
                "tool:bash",
                "notice",
                "assistant"
            ]
        );
        assert_eq!(calls, 3, "the notice did not start another turn");
    }

    /// The steer probe is wired: another shell's notice ends a long wait, so
    /// the model reads it instead of blocking until the waited-on shell ends.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_finished_shell_ends_a_wait_on_another() {
        let (labels, calls, elapsed) = conversation(script(vec![
            launch("long"),
            launch("slow"),
            wait_on(0),
            FauxStep::text("done"),
        ]))
        .await;
        assert_eq!(
            labels,
            [
                "user",
                "assistant",
                "tool:bash",
                "assistant",
                "tool:bash",
                "assistant",
                "tool:bash_output",
                "notice",
                "assistant"
            ]
        );
        assert_eq!(calls, 4);
        assert!(elapsed < Duration::from_secs(2), "waited {elapsed:?}");
    }
}
