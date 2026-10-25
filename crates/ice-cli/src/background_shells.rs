//! A session's background shells: the registry `bash` and the control tools
//! share, the watch registry, delivery of completions and watch alerts into
//! the conversation, and shutdown. Mirrors the session integration of
//! ice's `bash-background.ts`/`bash-watch.ts` (SESSION-023/025).
//!
//! Completions and alerts are sent to the main lane as custom messages
//! delivered as follow-ups with `triggerTurn`: while the agent is running
//! they are read at the next turn boundary; when it is idle they start a
//! turn so the model can react.

use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::task::JoinHandle;

use ice_harness::agent_harness::AgentHarness;
use ice_harness::types::HarnessTool;
use ice_harness::types::{MessageDelivery, SendMessageOptions};
use ice_tools::bash_background::{
    BashBackgroundCompletion, BashBackgroundRegistry, BASH_BACKGROUND_COMPLETED_CUSTOM_TYPE,
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
        })
    }

    /// The registry handed to the `bash` tool.
    pub fn registry(&self) -> Arc<BashBackgroundRegistry> {
        self.registry.clone()
    }

    pub fn auto_background_secs(&self) -> Option<f64> {
        self.auto_background_secs
    }

    /// `bash_output`, `bash_kill`, `bash_input` and `bash_watch`, named for
    /// the session's tool filters.
    pub fn tools(&self) -> Vec<(&'static str, HarnessTool)> {
        let mut tools: Vec<(&'static str, HarnessTool)> = create_bash_shell_tools(
            self.registry.clone(),
            BashShellToolOptions {
                input_approval: self.approval,
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
        let weak = harness.downgrade();
        let deliveries = self.deliveries.clone();
        self.registry.on_complete(move |completion| {
            if let Some(harness) = weak.upgrade() {
                let message = completion_message(completion);
                deliveries.spawn(deliver(harness, message));
            }
        });
        let weak = harness.downgrade();
        let deliveries = self.deliveries.clone();
        self.watches.on_fire(move |fire| {
            if let Some(harness) = weak.upgrade() {
                let message = fire_message(fire);
                deliveries.spawn(deliver(harness, message));
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

async fn deliver(harness: AgentHarness, message: ice_agent::AgentMessage) {
    let options = SendMessageOptions {
        trigger_turn: Some(true),
        deliver_as: MessageDelivery::FollowUp,
    };
    if let Err(error) = harness.lane("main").send_message(message, options).await {
        tracing::warn!("background shell notice was not delivered: {error}");
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
    let mut text = format!(
        "Background shell {} {status} after {}s: {}",
        c.task_id,
        c.duration_ms / 1000,
        ice_tools::bash_background::summarize_shell_command(&c.command)
    );
    if let Some(description) = &c.description {
        text.push_str(&format!(" ({description})"));
    }
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
        "Watch {} on background shell {} ({}{}): {}",
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
            task_id: "bash-2".into(),
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
                "Background shell bash-2 completed after 4s: npm test (+1 more lines) (tests)"
            ),
            "{text}"
        );
        assert!(text.contains("left processes running"), "{text}");
        assert!(text.ends_with("\n\n3 passed"), "{text}");
        let fire = BashWatchFire {
            watch_id: "watch-1".into(),
            target: "bash-2".into(),
            kind: ice_tools::bash_watch::BashWatchKind::Pattern,
            reason: "Output matched \"ready\": ready on :3000".into(),
            tail: "ready on :3000".into(),
            log_path: None,
            auto: false,
            fire_id: "watch-1#1".into(),
        };
        let text = text_of(&fire_message(&fire));
        assert!(
            text.starts_with("Watch watch-1 on background shell bash-2 (pattern): Output matched"),
            "{text}"
        );
        assert!(text.ends_with("Recent output:\nready on :3000"), "{text}");
    }
}
