//! The Blackhole harness extension: the before-compaction engine, the
//! context-only trim and the boundary trigger. Ports
//! `hooks/before-compact.ts`, `hooks/trim-kept-tail.ts` and
//! `om/compaction-trigger.ts` (BLACK-039..041).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use ice_agent::AgentMessage;
use ice_ai::types::{Content, UserContent};
use ice_harness::extension::{
    AfterCompactEvent, BeforeCompactDecision, BeforeCompactEvent, BoundaryDecision, BoundaryEvent,
    CompactionOverride, CompactionTrigger, HarnessExtension,
};
use serde_json::json;

use super::checkpoint_recall::consume_checkpoint_recall_section;
use super::config::{
    load_config_with, resolve_compact_after_tokens, BlackholeConfig, CompactionEngine,
    CompactionMode, MidRunCompaction, TailBehavior,
};
use super::live_state::{
    extract_subagents, format_active_goal_section, format_background_shells_section,
    format_external_tools_section, format_mode_and_plan_section, format_subagents_section,
};
use super::normalize::FileOps;
use super::summarize::{compile, CompileInput};
use super::tail::resolve_blackhole_tail;
use super::trim::trim_kept_tool_results;
use crate::builtin_extensions::{NoticeLevel, NoticeMailbox};

/// `customInstructions` that admit a manual-mode compaction.
pub const ICE_VCC_COMPACT_INSTRUCTION: &str = "__ice_vcc__";

/// Environment lookups, injectable for tests.
pub type EnvFn = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// Transient trigger state (`TriggerState`).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct TriggerState {
    /// A boundary compaction was requested and has not produced an entry yet.
    requested: bool,
    /// A requested compaction did not happen; skip requests until this run ends.
    suspended: bool,
}

pub struct BlackholeEngine {
    agent_dir: PathBuf,
    env: EnvFn,
    notices: NoticeMailbox,
    now: Arc<dyn Fn() -> i64 + Send + Sync>,
    trigger: Mutex<TriggerState>,
}

impl BlackholeEngine {
    pub fn new(agent_dir: PathBuf, notices: NoticeMailbox) -> Self {
        Self {
            agent_dir,
            env: Arc::new(|name| std::env::var(name).ok()),
            notices,
            now: Arc::new(super::now_ms),
            trigger: Mutex::new(TriggerState::default()),
        }
    }

    /// Replace the environment and clock (tests).
    pub fn with_env_and_clock(
        mut self,
        env: EnvFn,
        now: Arc<dyn Fn() -> i64 + Send + Sync>,
    ) -> Self {
        self.env = env;
        self.now = now;
        self
    }

    fn load(&self, warn: bool) -> BlackholeConfig {
        let env = Arc::clone(&self.env);
        let lookup = move |name: &str| env(name);
        let notices = self.notices.clone();
        load_config_with(&self.agent_dir, &lookup, &mut |message| {
            if warn {
                notices.notify(NoticeLevel::Warning, message);
            }
        })
    }

    /// Blackhole is opt-in: it runs once its config file exists (written by
    /// `/blackhole` or `/settings`) or a compaction variable selects it.
    pub fn installed(&self) -> bool {
        super::config::config_path(&self.agent_dir).exists()
            || [
                super::config::ENV_COMPACTION,
                super::config::ENV_COMPACTION_ENGINE,
                super::config::ENV_MID_RUN_COMPACTION,
            ]
            .iter()
            .any(|name| (self.env)(name).is_some())
    }

    /// The `/compact` instruction that admits a manual-mode Blackhole
    /// compaction, when manual mode is configured.
    pub fn manual_compaction_instruction(&self) -> Option<&'static str> {
        if !self.installed() {
            return None;
        }
        let config = self.load(false);
        (config.compaction == CompactionMode::Manual
            && config.compaction_engine == CompactionEngine::Blackhole)
            .then_some(ICE_VCC_COMPACT_INSTRUCTION)
    }

    fn compaction_active(config: &BlackholeConfig) -> bool {
        config.compaction != CompactionMode::Off
            && config.compaction_engine == CompactionEngine::Blackhole
    }

    async fn compose(&self, event: &BeforeCompactEvent) -> Option<BeforeCompactDecision> {
        if !self.installed() {
            return None;
        }
        let config = self.load(true);
        if !Self::compaction_active(&config) {
            return None;
        }
        if config.compaction == CompactionMode::Manual
            && event.custom_instructions.as_deref() != Some(ICE_VCC_COMPACT_INSTRUCTION)
        {
            return None;
        }
        let preparation = &event.preparation;
        // Overflow recovery keeps the latest user turn the host preserved; a
        // minimal tail there would summarize away the request being retried.
        let tail_behavior = if event.trigger == CompactionTrigger::Overflow {
            TailBehavior::IceDefault
        } else {
            config.tail_behavior
        };
        let tail = resolve_blackhole_tail(
            tail_behavior,
            &preparation.messages_to_summarize,
            &preparation.turn_prefix_messages,
            &preparation.retained_tail,
        );
        let user_requests: Vec<String> = tail
            .source_messages
            .iter()
            .filter_map(user_request_text)
            .filter(|text| !text.trim().is_empty())
            .collect();
        let summary = compile(&CompileInput {
            messages: ice_harness::messages::convert_to_llm(tail.source_messages.clone()),
            previous_summary: preparation.previous_summary.clone(),
            file_ops: Some(FileOps {
                read_files: preparation.file_ops.read.iter().cloned().collect(),
                modified_files: preparation.file_ops.edited.iter().cloned().collect(),
                created_files: preparation.file_ops.written.iter().cloned().collect(),
            }),
            user_requests: Some(user_requests),
        });

        // Volatile working state, rebuilt from durable entries at every
        // compaction and never merged from the previous checkpoint.
        let subagents = extract_subagents(&event.branch_entries);
        let state_sections: Vec<String> = [
            format_active_goal_section(&event.branch_entries),
            format_mode_and_plan_section(&event.branch_entries),
            format_subagents_section(&subagents),
            format_background_shells_section(&event.branch_entries),
            format_external_tools_section(&event.branch_entries),
        ]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect();

        let mut dedup_source = state_sections.clone();
        dedup_source.push(summary.clone());
        let memory_section = consume_checkpoint_recall_section(
            &self.agent_dir,
            (self.now)(),
            Some(&event.session_id),
            &dedup_source.join("\n"),
        );
        if state_sections.is_empty() && summary.is_empty() && memory_section.is_empty() {
            return Some(BeforeCompactDecision::Cancel);
        }

        let mut state_of_work = String::new();
        if config.refine {
            let mut refine_input = state_sections.clone();
            refine_input.push(summary.clone());
            match self
                .refine(
                    event,
                    config.refine_model.as_deref(),
                    &refine_input.join("\n\n"),
                    &tail.source_messages,
                )
                .await
            {
                Ok(section) => state_of_work = section,
                Err(error) => {
                    if !event.signal.is_cancelled() {
                        self.notices.notify(
                            NoticeLevel::Warning,
                            format!("Blackhole refine skipped: {error}"),
                        );
                    }
                }
            }
        }

        let mut parts = state_sections.clone();
        parts.push(summary);
        parts.push(state_of_work.clone());
        parts.push(memory_section.clone());
        let final_summary = parts
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");
        let section_names: Vec<String> = state_sections
            .iter()
            .map(|part| {
                let end = part.find(']').unwrap_or(part.len());
                part.get(1..end).unwrap_or_default().to_string()
            })
            .collect();
        Some(BeforeCompactDecision::Replace(CompactionOverride {
            summary: final_summary,
            retained_tail: tail.retained_tail,
            details: Some(json!({
                "engine": "blackhole",
                "memory": false,
                "tailBehavior": tail_behavior.as_str(),
                "summarizedKeptTail": tail.summarized_kept_tail,
                "subagents": subagents.len(),
                "relevantMemory": !memory_section.is_empty(),
                "stateSections": section_names,
                "refined": !state_of_work.is_empty(),
            })),
        }))
    }

    async fn refine(
        &self,
        event: &BeforeCompactEvent,
        refine_model: Option<&str>,
        summary: &str,
        recent: &[AgentMessage],
    ) -> Result<String, String> {
        let (provider, model) = match refine_model {
            Some(spec) => {
                let (provider_id, model_id) = spec.split_once('/').unwrap_or((spec, ""));
                event
                    .providers
                    .iter()
                    .find(|provider| provider.id() == provider_id)
                    .and_then(|provider| {
                        provider
                            .models()
                            .iter()
                            .find(|model| model.id == model_id)
                            .map(|model| (Arc::clone(provider), model.clone()))
                    })
                    .ok_or_else(|| format!("refine model {spec} is not available"))?
            }
            None => (
                event
                    .provider
                    .clone()
                    .ok_or_else(|| "refine model (active) is not available".to_string())?,
                event.model.clone(),
            ),
        };
        super::refine::refine_summary(provider, model, None, summary, recent, event.signal.clone())
            .await
    }
}

fn user_request_text(message: &AgentMessage) -> Option<String> {
    let AgentMessage::User(user) = message else {
        return None;
    };
    Some(match &user.content {
        UserContent::Text(text) => text.clone(),
        UserContent::Blocks(blocks) => blocks
            .iter()
            .filter_map(|block| match block {
                Content::Text(text) => Some(text.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
    })
}

#[async_trait::async_trait]
impl HarnessExtension for BlackholeEngine {
    async fn run_start(&self) {
        *self.trigger.lock().unwrap() = TriggerState::default();
    }

    async fn before_compact(&self, event: &BeforeCompactEvent) -> Option<BeforeCompactDecision> {
        self.compose(event).await
    }

    async fn after_compact(&self, _event: &AfterCompactEvent<'_>) {
        self.trigger.lock().unwrap().requested = false;
    }

    async fn boundary(&self, event: &BoundaryEvent) -> BoundaryDecision {
        if !self.installed() {
            return BoundaryDecision::Default;
        }
        let mut state = self.trigger.lock().unwrap();
        if state.requested {
            // The previous boundary compaction failed or was cancelled.
            // Retrying every tool turn would repeat the failure.
            state.requested = false;
            state.suspended = true;
        }
        let config = self.load(true);
        if config.compaction != CompactionMode::Auto
            || config.compaction_engine != CompactionEngine::Blackhole
            || config.mid_run_compaction == MidRunCompaction::Off
        {
            return BoundaryDecision::Default;
        }
        let window = (event.context_window > 0).then_some(event.context_window as f64);
        let notices = self.notices.clone();
        let Some(threshold) = resolve_compact_after_tokens(&config, window, &mut |message| {
            notices.notify(NoticeLevel::Warning, message)
        }) else {
            return BoundaryDecision::Default;
        };
        if event.input_tokens <= threshold {
            state.suspended = false;
            return BoundaryDecision::Default;
        }
        if state.suspended {
            return BoundaryDecision::Default;
        }
        state.requested = true;
        BoundaryDecision::Compact {
            pause: config.mid_run_compaction == MidRunCompaction::Pause,
        }
    }

    fn transform_context(
        &self,
        messages: &[AgentMessage],
        latest_compaction_at: Option<i64>,
    ) -> Option<Vec<AgentMessage>> {
        if !self.installed() {
            return None;
        }
        let config = self.load(false);
        if !Self::compaction_active(&config) || !config.trim_kept_tool_results {
            return None;
        }
        trim_kept_tool_results(messages, latest_compaction_at?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blackhole::config::{config_path, ENV_MID_RUN_COMPACTION};
    use crate::blackhole::live_state::tests::{custom, message};
    use ice_ai::types::UserMessage;
    use ice_harness::compaction::{CompactionPreparation, FileOperations};
    use ice_harness::types::CompactionSettings;
    use tokio_util::sync::CancellationToken;

    fn engine(dir: &std::path::Path, env: &[(&'static str, &'static str)]) -> BlackholeEngine {
        let path = config_path(dir);
        if !path.exists() {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "{}").unwrap();
        }
        let env: Vec<(String, String)> = env
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        BlackholeEngine::new(dir.to_path_buf(), NoticeMailbox::new()).with_env_and_clock(
            Arc::new(move |name| {
                env.iter()
                    .find(|(key, _)| key == name)
                    .map(|(_, value)| value.clone())
            }),
            Arc::new(|| 1_800_000_000_000),
        )
    }

    fn user(text: &str) -> AgentMessage {
        AgentMessage::User(UserMessage::new(text.to_string(), 0))
    }

    fn event(trigger: CompactionTrigger, custom_instructions: Option<&str>) -> BeforeCompactEvent {
        let mut file_ops = FileOperations::default();
        file_ops.read.insert("src/lib.rs".into());
        BeforeCompactEvent {
            preparation: CompactionPreparation {
                messages_to_summarize: vec![user("Implement the cache layer for the API")],
                turn_prefix_messages: Vec::new(),
                retained_tail: vec![user("Tail request")],
                is_split_turn: false,
                tokens_before: 1234,
                previous_summary: None,
                file_ops,
                settings: CompactionSettings::default(),
            },
            branch_entries: vec![
                message(user("Implement the cache layer for the API"), 0),
                custom(
                    "ice-goals-state",
                    json!({"goals":[{"id":"g","objective":"Ship cache"}]}),
                    1,
                ),
            ],
            custom_instructions: custom_instructions.map(str::to_string),
            trigger,
            will_retry: false,
            signal: CancellationToken::new(),
            session_id: "s1".into(),
            model: ice_ai::Model::new("m", "m", ice_ai::Api::Faux, "faux", "https://example.test"),
            provider: None,
            providers: Vec::new(),
        }
    }

    #[tokio::test]
    async fn default_engine_replaces_the_checkpoint_with_minimal_tail() {
        let dir = tempfile::tempdir().unwrap();
        let engine = engine(dir.path(), &[]);
        let Some(BeforeCompactDecision::Replace(checkpoint)) = engine
            .before_compact(&event(CompactionTrigger::Threshold, None))
            .await
        else {
            panic!("expected a checkpoint");
        };
        assert!(checkpoint.summary.starts_with("[Active Goal]"));
        assert!(checkpoint
            .summary
            .contains("[User Requests]\n- Implement the cache layer for the API\n- Tail request"));
        assert!(checkpoint.summary.contains("- Read: src/lib.rs"));
        assert_eq!(checkpoint.retained_tail, Some(Vec::new()));
        let details = checkpoint.details.unwrap();
        assert_eq!(details["engine"], "blackhole");
        assert_eq!(details["memory"], false);
        assert_eq!(details["summarizedKeptTail"], 1);
        assert_eq!(details["stateSections"], json!(["Active Goal"]));
    }

    #[tokio::test]
    async fn off_engine_and_manual_mode_defer_to_the_host() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(config_path(dir.path()).parent().unwrap()).unwrap();
        std::fs::write(
            config_path(dir.path()),
            r#"{"compactionEngine":"ice-default"}"#,
        )
        .unwrap();
        let engine1 = engine(dir.path(), &[]);
        assert!(engine1
            .before_compact(&event(CompactionTrigger::Manual, None))
            .await
            .is_none());
        std::fs::write(config_path(dir.path()), r#"{"compaction":"manual"}"#).unwrap();
        assert!(engine1
            .before_compact(&event(CompactionTrigger::Manual, None))
            .await
            .is_none());
        assert!(engine1
            .before_compact(&event(
                CompactionTrigger::Manual,
                Some(ICE_VCC_COMPACT_INSTRUCTION)
            ))
            .await
            .is_some());
    }

    #[tokio::test]
    async fn overflow_keeps_the_host_tail() {
        let dir = tempfile::tempdir().unwrap();
        let engine = engine(dir.path(), &[]);
        let Some(BeforeCompactDecision::Replace(checkpoint)) = engine
            .before_compact(&event(CompactionTrigger::Overflow, None))
            .await
        else {
            panic!("expected a checkpoint");
        };
        assert_eq!(checkpoint.retained_tail, None);
    }

    #[tokio::test]
    async fn empty_input_cancels_the_compaction() {
        let dir = tempfile::tempdir().unwrap();
        let engine = engine(dir.path(), &[]);
        let mut event = event(CompactionTrigger::Threshold, None);
        event.preparation.messages_to_summarize.clear();
        event.preparation.retained_tail.clear();
        event.preparation.file_ops = FileOperations::default();
        event.branch_entries.clear();
        assert_eq!(
            engine.before_compact(&event).await,
            Some(BeforeCompactDecision::Cancel)
        );
    }

    #[tokio::test]
    async fn trigger_requests_once_then_suspends_after_a_missed_request() {
        let dir = tempfile::tempdir().unwrap();
        let engine = engine(dir.path(), &[(ENV_MID_RUN_COMPACTION, "pause")]);
        let over = BoundaryEvent {
            input_tokens: 900,
            context_window: 1000,
        };
        let under = BoundaryEvent {
            input_tokens: 100,
            context_window: 1000,
        };
        assert_eq!(
            engine.boundary(&over).await,
            BoundaryDecision::Compact { pause: true }
        );
        // No session_compact acknowledged the request: suspend.
        assert_eq!(engine.boundary(&over).await, BoundaryDecision::Default);
        assert_eq!(engine.boundary(&over).await, BoundaryDecision::Default);
        // Falling below the threshold clears the suspension.
        assert_eq!(engine.boundary(&under).await, BoundaryDecision::Default);
        assert_eq!(
            engine.boundary(&over).await,
            BoundaryDecision::Compact { pause: true }
        );
        // A new run resets the state.
        engine.run_start().await;
        assert_eq!(
            engine.boundary(&over).await,
            BoundaryDecision::Compact { pause: true }
        );
    }

    #[tokio::test]
    async fn engine_is_inactive_until_configured() {
        let dir = tempfile::tempdir().unwrap();
        let engine = BlackholeEngine::new(dir.path().to_path_buf(), NoticeMailbox::new())
            .with_env_and_clock(Arc::new(|_| None), Arc::new(|| 0));
        assert!(!engine.installed());
        assert!(engine
            .before_compact(&event(CompactionTrigger::Threshold, None))
            .await
            .is_none());
    }

    #[tokio::test]
    async fn trigger_is_off_by_default() {
        let dir = tempfile::tempdir().unwrap();
        let engine = engine(dir.path(), &[]);
        let over = BoundaryEvent {
            input_tokens: 900,
            context_window: 1000,
        };
        assert_eq!(engine.boundary(&over).await, BoundaryDecision::Default);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn harness_manual_compaction_uses_the_deterministic_engine() {
        use ice_ai::providers::faux::{FauxProvider, FauxScript};
        use ice_ai::types::{Api, AssistantMessage, StopReason};
        use ice_harness::agent_harness::{AgentHarness, AgentLane, CompactionOutcome};
        use ice_harness::session::memory::{InMemorySessionStorage, SystemClock};
        use ice_harness::session::types::{
            BranchBounds, Entry, EntryOrder, EntryQuery, SessionMetadata,
        };
        use ice_harness::session::{DefaultIdGenerator, Session};

        let dir = tempfile::tempdir().unwrap();
        let engine = Arc::new(engine(dir.path(), &[]));
        let session = Session::new(
            Arc::new(InMemorySessionStorage::new(
                SessionMetadata {
                    id: "bh".into(),
                    created_at: 0,
                    parent_session_id: None,
                },
                Arc::new(SystemClock),
                Arc::new(DefaultIdGenerator::new()),
            )),
            None,
        );
        let view = session.view("main");
        for (index, text) in ["Implement the parser for config files", "Add tests for it"]
            .iter()
            .enumerate()
        {
            view.append_message(user(&format!("{text} {}", "x".repeat(400 * (2 - index)))))
                .await
                .unwrap();
            let mut answer = AssistantMessage::empty(Api::Faux, "faux", "faux", 1);
            answer.content = vec![Content::text("Done with that step.")];
            answer.stop_reason = StopReason::Stop;
            view.append_message(AgentMessage::Assistant(Box::new(answer)))
                .await
                .unwrap();
        }
        let faux = FauxProvider::new(FauxScript::new());
        let harness = AgentHarness::create(ice_harness::types::AgentHarnessOptions {
            model: faux.default_model().clone(),
            session,
            models: vec![faux.clone() as Arc<dyn ice_ai::Provider>],
            allow_existing_session: true,
            ..ice_harness::types::AgentHarnessOptions::default()
        })
        .await
        .unwrap();
        harness
            .set_coding_compaction_policy(Some(
                ice_harness::compaction::settings::CodingAgentCompactionPolicy {
                    keep_recent_tokens: 20,
                    ..Default::default()
                },
            ))
            .await
            .unwrap();
        harness.set_extensions(vec![engine.clone() as Arc<dyn HarnessExtension>]);
        let result = AgentLane::compact(&harness, None).await.unwrap();
        assert!(
            matches!(result.outcome, CompactionOutcome::Completed { .. }),
            "{:?}",
            result.outcome
        );
        let leaf = view.get_leaf_id().await.unwrap().unwrap();
        let entries = view
            .find_entries_on_branch(
                &EntryQuery {
                    order: Some(EntryOrder::OldestFirst),
                    ..Default::default()
                },
                &BranchBounds {
                    start: Some(leaf),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let Some(Entry::Compaction(compaction)) = entries.last() else {
            panic!("expected a compaction entry");
        };
        assert!(compaction
            .summary
            .contains("[Session Goal]\n- Implement the parser for config files"));
        assert!(compaction.retained_tail.is_empty(), "minimal tail");
        assert_eq!(compaction.details.as_ref().unwrap()["engine"], "blackhole");
    }
}
