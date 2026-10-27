//! In-process extension seam: transient run prompts, compaction overrides,
//! checkpoint observation, boundary compaction and context rewrites.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ice_agent::message::AgentMessage;
use ice_ai::providers::faux::{faux_assistant_message, FauxProvider, FauxScript, FauxStep};
use ice_ai::types::{AssistantMessage, Context, StopReason, UserContent, UserMessage};
use ice_ai::{AssistantMessageEventStream, Model, Provider, SimpleStreamOptions};
use ice_harness::agent_harness::{AgentHarness, AgentLane, CompactionOutcome};
use ice_harness::compaction::settings::CodingAgentCompactionPolicy;
use ice_harness::extension::{
    AfterCompactEvent, BeforeCompactDecision, BeforeCompactEvent, BeforeRunEvent, BoundaryDecision,
    BoundaryEvent, CompactionOverride, CompactionTrigger, HarnessExtension, PromptContext,
};
use ice_harness::session::memory::{InMemorySessionStorage, SystemClock};
use ice_harness::session::types::{
    BranchBounds, Entry, EntryOrder, EntryQuery, LaneRecord, OperationOutcome, RecordQuery,
    SessionMetadata,
};
use ice_harness::session::{DefaultIdGenerator, Session};
use ice_harness::types::{AgentHarnessOptions, CompactionSettings, HarnessTool};
use ice_tools::{ExecutionToolContext, InMemoryExecutionEnv};

struct RecordingProvider {
    inner: Arc<FauxProvider>,
    system_prompts: Mutex<Vec<String>>,
    contexts: Mutex<Vec<Context>>,
}

#[async_trait]
impl Provider for RecordingProvider {
    fn id(&self) -> &str {
        self.inner.id()
    }
    fn models(&self) -> &[Model] {
        self.inner.models()
    }
    async fn stream_simple(
        &self,
        model: &Model,
        context: &Context,
        options: &SimpleStreamOptions,
    ) -> AssistantMessageEventStream {
        self.system_prompts
            .lock()
            .unwrap()
            .push(context.system_prompt.clone().unwrap_or_default());
        self.contexts.lock().unwrap().push(context.clone());
        self.inner.stream_simple(model, context, options).await
    }
}

#[derive(Default)]
struct Probe {
    decision: Mutex<Option<BeforeCompactDecision>>,
    boundary: Mutex<Option<BoundaryDecision>>,
    seen_triggers: Mutex<Vec<(CompactionTrigger, bool, String)>>,
    before_compact_branch_len: Mutex<Vec<usize>>,
    run_starts: Mutex<usize>,
    transform_times: Mutex<Vec<Option<i64>>>,
}

#[async_trait]
impl HarnessExtension for Probe {
    async fn run_start(&self) {
        *self.run_starts.lock().unwrap() += 1;
    }

    async fn before_run(&self, event: &BeforeRunEvent<'_>) -> Option<String> {
        Some(format!(
            "{}\n\nRUN_MEMORY for: {}",
            event.system_prompt.unwrap_or_default(),
            event.prompt
        ))
    }

    async fn before_compact(&self, event: &BeforeCompactEvent) -> Option<BeforeCompactDecision> {
        self.before_compact_branch_len
            .lock()
            .unwrap()
            .push(event.branch_entries.len());
        self.decision.lock().unwrap().clone()
    }

    async fn after_compact(&self, event: &AfterCompactEvent<'_>) {
        self.seen_triggers.lock().unwrap().push((
            event.trigger,
            event.from_extension,
            event.entry.summary.clone(),
        ));
    }

    async fn boundary(&self, _event: &BoundaryEvent) -> BoundaryDecision {
        self.boundary.lock().unwrap().take().unwrap_or_default()
    }

    fn transform_context(
        &self,
        _messages: &[AgentMessage],
        latest_compaction_at: Option<i64>,
    ) -> Option<Vec<AgentMessage>> {
        self.transform_times
            .lock()
            .unwrap()
            .push(latest_compaction_at);
        None
    }
}

fn session() -> Session {
    let storage = Arc::new(InMemorySessionStorage::new(
        SessionMetadata {
            id: "extensions".into(),
            created_at: 0,
            parent_session_id: None,
        },
        Arc::new(SystemClock),
        Arc::new(DefaultIdGenerator::new()),
    ));
    Session::new(storage, None)
}

async fn append_history(session: &Session) {
    let view = session.view("main");
    view.append_message(AgentMessage::User(UserMessage::new(
        UserContent::Text(format!("OLD {}", "x".repeat(3000))),
        1,
    )))
    .await
    .unwrap();
    let mut answer = AssistantMessage::empty(ice_ai::types::Api::Faux, "faux", "faux", 1);
    answer.content = vec![ice_ai::types::Content::text("older answer")];
    answer.stop_reason = StopReason::Stop;
    view.append_message(AgentMessage::Assistant(Box::new(answer)))
        .await
        .unwrap();
    view.append_message(AgentMessage::User(UserMessage::new(
        UserContent::Text(format!("recent request {}", "y".repeat(400))),
        1,
    )))
    .await
    .unwrap();
    let mut recent = AssistantMessage::empty(ice_ai::types::Api::Faux, "faux", "faux", 1);
    recent.content = vec![ice_ai::types::Content::text("recent answer")];
    recent.stop_reason = StopReason::Stop;
    view.append_message(AgentMessage::Assistant(Box::new(recent)))
        .await
        .unwrap();
}

async fn harness(steps: Vec<FauxStep>) -> (AgentHarness, Arc<RecordingProvider>, Arc<Probe>) {
    harness_with_compaction_settings(steps, CompactionSettings::default()).await
}

async fn harness_with_compaction_settings(
    steps: Vec<FauxStep>,
    compaction: CompactionSettings,
) -> (AgentHarness, Arc<RecordingProvider>, Arc<Probe>) {
    let session = session();
    append_history(&session).await;
    let script = FauxScript::new();
    script.set_responses(steps);
    let faux = FauxProvider::new(script);
    let provider = Arc::new(RecordingProvider {
        inner: faux.clone(),
        system_prompts: Mutex::new(Vec::new()),
        contexts: Mutex::new(Vec::new()),
    });
    let mut model = faux.default_model().clone();
    model.context_window = 2000;
    model.max_tokens = 1200;
    let harness = AgentHarness::create(AgentHarnessOptions {
        model,
        session,
        models: vec![provider.clone() as Arc<dyn Provider>],
        compaction,
        allow_existing_session: true,
        system_prompt: Some("BASE".into()),
        ..AgentHarnessOptions::default()
    })
    .await
    .unwrap();
    harness
        .set_coding_compaction_policy(Some(CodingAgentCompactionPolicy {
            keep_recent_tokens: 50,
            ..CodingAgentCompactionPolicy::default()
        }))
        .await
        .unwrap();
    let probe = Arc::new(Probe::default());
    harness.set_extensions(vec![probe.clone() as Arc<dyn HarnessExtension>]);
    (harness, provider, probe)
}

async fn compaction_entries(
    harness: &AgentHarness,
) -> Vec<ice_harness::session::types::CompactionEntry> {
    let view = harness.session().view("main");
    let leaf = view.get_leaf_id().await.unwrap().unwrap();
    view.find_entries_on_branch(
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
    .unwrap()
    .into_iter()
    .filter_map(|entry| match entry {
        Entry::Compaction(compaction) => Some(compaction),
        _ => None,
    })
    .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn manual_compaction_persists_the_extension_checkpoint_without_a_model_call() {
    let (harness, provider, probe) = harness(Vec::new()).await;
    *probe.decision.lock().unwrap() = Some(BeforeCompactDecision::Replace(CompactionOverride {
        summary: "DETERMINISTIC CHECKPOINT".into(),
        retained_tail: Some(Vec::new()),
        details: Some(serde_json::json!({"engine": "test"})),
    }));
    let result = AgentLane::compact(&harness, Some("__ice_vcc__"))
        .await
        .unwrap();
    assert!(
        matches!(result.outcome, CompactionOutcome::Completed { .. }),
        "{:?} consulted={:?}",
        result.outcome,
        probe.before_compact_branch_len.lock().unwrap()
    );
    assert!(provider.system_prompts.lock().unwrap().is_empty());
    let entries = compaction_entries(&harness).await;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].summary, "DETERMINISTIC CHECKPOINT");
    assert!(entries[0].retained_tail.is_empty());
    assert_eq!(
        entries[0].details,
        Some(serde_json::json!({"engine": "test"}))
    );
    assert_eq!(
        probe.seen_triggers.lock().unwrap().as_slice(),
        &[(
            CompactionTrigger::Manual,
            true,
            "DETERMINISTIC CHECKPOINT".to_string()
        )]
    );
    assert_eq!(probe.before_compact_branch_len.lock().unwrap()[0], 4);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_manual_compaction_is_declined() {
    let (harness, provider, probe) = harness(Vec::new()).await;
    *probe.decision.lock().unwrap() = Some(BeforeCompactDecision::Cancel);
    let result = AgentLane::compact(&harness, None).await.unwrap();
    assert!(matches!(result.outcome, CompactionOutcome::Declined { .. }));
    // The operation ends exactly once, as declined.
    let finished = harness
        .session()
        .find_records(&RecordQuery {
            run_id: Some(result.run_id.clone()),
            record_type: Some("operation_finished"),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(
        matches!(
            finished.as_slice(),
            [LaneRecord::OperationFinished(record)] if record.outcome == OperationOutcome::Declined
        ),
        "{finished:?}"
    );
    assert!(provider.system_prompts.lock().unwrap().is_empty());
    assert!(compaction_entries(&harness).await.is_empty());
    assert!(probe.seen_triggers.lock().unwrap().is_empty());
    assert_eq!(probe.before_compact_branch_len.lock().unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_threshold_checkpoint_stops_before_the_provider_request() {
    let (harness, provider, probe) = harness(vec![FauxStep::text("must not run")]).await;
    harness
        .set_coding_compaction_policy(Some(CodingAgentCompactionPolicy {
            threshold_percent: 1.0,
            keep_recent_tokens: 50,
            ..CodingAgentCompactionPolicy::default()
        }))
        .await
        .unwrap();
    *probe.decision.lock().unwrap() = Some(BeforeCompactDecision::Cancel);

    let result = harness
        .prompt_text("threshold trigger", Vec::new())
        .await
        .unwrap();

    assert!(
        matches!(
            result.outcome,
            ice_harness::agent_harness::HarnessRunOutcome::Failed { ref error, .. }
                if error.code == "compaction_failed"
        ),
        "unexpected run outcome: {:?}",
        result.outcome
    );
    assert!(provider.system_prompts.lock().unwrap().is_empty());
    assert!(compaction_entries(&harness).await.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn before_run_adds_transient_prompt_text_and_transform_sees_compaction_time() {
    let (harness, provider, probe) = harness(vec![FauxStep::text("done")]).await;
    *probe.decision.lock().unwrap() = Some(BeforeCompactDecision::Replace(CompactionOverride {
        summary: "CHECKPOINT".into(),
        retained_tail: None,
        details: None,
    }));
    AgentLane::compact(&harness, None).await.unwrap();
    harness
        .prompt_text("hello there", Vec::new())
        .await
        .unwrap();
    let prompts = provider.system_prompts.lock().unwrap().clone();
    assert_eq!(prompts.len(), 1);
    assert!(prompts[0].starts_with("BASE"));
    assert!(prompts[0].ends_with("RUN_MEMORY for: hello there"));
    // The stored system prompt is unchanged.
    assert_eq!(
        harness.get_system_prompt().await.unwrap().as_deref(),
        Some("BASE")
    );
    assert_eq!(*probe.run_starts.lock().unwrap(), 1);
    let times = probe.transform_times.lock().unwrap().clone();
    assert!(!times.is_empty());
    assert!(times.iter().all(|time| time.is_some()));
}

#[derive(Default)]
struct MemoryProbe {
    persisted: Mutex<Vec<bool>>,
}

#[async_trait]
impl HarnessExtension for MemoryProbe {
    async fn prompt_context(&self, event: &BeforeRunEvent<'_>) -> Option<PromptContext> {
        Some(PromptContext {
            custom_type: "test-memory".into(),
            text: format!("MEMORY for: {}", event.prompt),
        })
    }

    async fn prompt_context_persisted(&self, _context: &PromptContext, persisted: bool) {
        self.persisted.lock().unwrap().push(persisted);
    }
}

fn user_texts(context: &Context) -> Vec<String> {
    context
        .messages
        .iter()
        .filter_map(|message| match message {
            ice_ai::types::Message::User(user) => Some(match &user.content {
                UserContent::Text(text) => text.clone(),
                UserContent::Blocks(blocks) => blocks
                    .iter()
                    .filter_map(|block| match block {
                        ice_ai::types::Content::Text(text) => Some(text.text.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
            }),
            _ => None,
        })
        .collect()
}

/// Prompt context is stored after the prompt as hidden history: the system
/// prompt never changes, and each request starts with the previous request
/// unchanged, so the provider's prompt cache keeps covering the history.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prompt_context_is_persisted_after_the_prompt_and_keeps_the_prefix_stable() {
    let (harness, provider, _) = harness(vec![FauxStep::text("one"), FauxStep::text("two")]).await;
    let memory = Arc::new(MemoryProbe::default());
    harness.set_extensions(vec![memory.clone() as Arc<dyn HarnessExtension>]);
    harness
        .prompt_text("first question", Vec::new())
        .await
        .unwrap();
    harness
        .prompt_text("second question", Vec::new())
        .await
        .unwrap();

    assert_eq!(memory.persisted.lock().unwrap().as_slice(), &[true, true]);
    let prompts = provider.system_prompts.lock().unwrap().clone();
    assert_eq!(prompts.len(), 2);
    assert_eq!(prompts[0], prompts[1], "the system prompt is stable");
    assert!(!prompts[0].contains("MEMORY"), "{}", prompts[0]);

    let contexts = provider.contexts.lock().unwrap().clone();
    let first = user_texts(&contexts[0]);
    assert_eq!(
        first[first.len() - 2..],
        ["first question", "MEMORY for: first question"]
    );
    let second = user_texts(&contexts[1]);
    assert_eq!(
        second[second.len() - 2..],
        ["second question", "MEMORY for: second question"]
    );
    assert_eq!(
        contexts[1].messages[..contexts[0].messages.len()],
        contexts[0].messages[..],
        "the first request is an unchanged prefix of the second"
    );

    let hidden: Vec<_> = harness
        .session()
        .view("main")
        .find_entries(&EntryQuery {
            order: Some(EntryOrder::OldestFirst),
            ..Default::default()
        })
        .await
        .unwrap()
        .into_iter()
        .filter_map(|entry| match entry {
            Entry::Message(message) => ice_harness::messages::custom_data(&message.message),
            _ => None,
        })
        .collect();
    assert_eq!(hidden.len(), 2);
    assert!(hidden
        .iter()
        .all(|data| data.custom_type == "test-memory" && !data.display));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn boundary_request_compacts_mid_run_and_pauses() {
    let steps = vec![
        FauxStep::tool_call("read", serde_json::json!({"path": "missing.txt"})),
        FauxStep::Message(faux_assistant_message("unreachable", StopReason::Stop)),
    ];
    let (harness, provider, probe) = harness(steps).await;
    let execution = Arc::new(InMemoryExecutionEnv::new());
    let env: Arc<dyn ice_tools::ExecutionEnv> = execution.clone();
    let mut_env: Arc<dyn ice_tools::MutatingEnv> = execution;
    let tool_context = ExecutionToolContext::new(env, Some(mut_env));
    harness
        .set_tools(
            vec![HarnessTool::new(ice_tools::create_read_tool(
                &tool_context,
                None,
            ))],
            Some(vec!["read".to_string()]),
        )
        .await
        .unwrap();
    *probe.boundary.lock().unwrap() = Some(BoundaryDecision::Compact { pause: true });
    *probe.decision.lock().unwrap() = Some(BeforeCompactDecision::Replace(CompactionOverride {
        summary: "MIDRUN CHECKPOINT".into(),
        retained_tail: Some(Vec::new()),
        details: None,
    }));
    harness.prompt_text("go", Vec::new()).await.unwrap();
    assert_eq!(
        provider.system_prompts.lock().unwrap().len(),
        1,
        "paused after checkpoint"
    );
    let triggers = probe.seen_triggers.lock().unwrap().clone();
    assert!(triggers
        .iter()
        .any(
            |(trigger, from_extension, summary)| *trigger == CompactionTrigger::MidRun
                && *from_extension
                && summary == "MIDRUN CHECKPOINT"
        ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_extension_midrun_checkpoint_stops_before_the_next_request() {
    let steps = vec![
        FauxStep::tool_call("read", serde_json::json!({"path": "missing.txt"})),
        FauxStep::text("must not run after cancelled checkpoint"),
    ];
    let (harness, provider, probe) = harness(steps).await;
    let execution = Arc::new(InMemoryExecutionEnv::new());
    let env: Arc<dyn ice_tools::ExecutionEnv> = execution.clone();
    let mut_env: Arc<dyn ice_tools::MutatingEnv> = execution;
    let tool_context = ExecutionToolContext::new(env, Some(mut_env));
    harness
        .set_tools(
            vec![HarnessTool::new(ice_tools::create_read_tool(
                &tool_context,
                None,
            ))],
            Some(vec!["read".to_string()]),
        )
        .await
        .unwrap();
    *probe.boundary.lock().unwrap() = Some(BoundaryDecision::Compact { pause: false });
    *probe.decision.lock().unwrap() = Some(BeforeCompactDecision::Cancel);

    let result = harness.prompt_text("go", Vec::new()).await.unwrap();

    assert!(
        matches!(
            result.outcome,
            ice_harness::agent_harness::HarnessRunOutcome::Failed { ref error, .. }
                if error.code == "compaction_failed"
        ),
        "unexpected run outcome: {:?}",
        result.outcome
    );
    assert_eq!(provider.system_prompts.lock().unwrap().len(), 1);
    assert!(compaction_entries(&harness).await.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn extension_boundary_can_compact_when_generic_compaction_is_disabled() {
    let steps = vec![
        FauxStep::tool_call("read", serde_json::json!({"path": "missing.txt"})),
        FauxStep::text("must not run after extension pause"),
    ];
    let (harness, provider, probe) = harness_with_compaction_settings(
        steps,
        CompactionSettings {
            enabled: false,
            ..CompactionSettings::default()
        },
    )
    .await;
    let execution = Arc::new(InMemoryExecutionEnv::new());
    let env: Arc<dyn ice_tools::ExecutionEnv> = execution.clone();
    let mut_env: Arc<dyn ice_tools::MutatingEnv> = execution;
    let tool_context = ExecutionToolContext::new(env, Some(mut_env));
    harness
        .set_tools(
            vec![HarnessTool::new(ice_tools::create_read_tool(
                &tool_context,
                None,
            ))],
            Some(vec!["read".to_string()]),
        )
        .await
        .unwrap();
    *probe.boundary.lock().unwrap() = Some(BoundaryDecision::Compact { pause: true });
    *probe.decision.lock().unwrap() = Some(BeforeCompactDecision::Replace(CompactionOverride {
        summary: "EXTENSION CHECKPOINT".into(),
        retained_tail: Some(Vec::new()),
        details: None,
    }));

    let result = harness.prompt_text("go", Vec::new()).await.unwrap();

    assert!(matches!(
        result.outcome,
        ice_harness::agent_harness::HarnessRunOutcome::Failed { ref error, .. }
            if error.code == "compaction_paused"
    ));
    assert_eq!(provider.system_prompts.lock().unwrap().len(), 1);
    let entries = compaction_entries(&harness).await;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].summary, "EXTENSION CHECKPOINT");
}
