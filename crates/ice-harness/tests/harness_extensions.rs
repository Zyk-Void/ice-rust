//! In-process extension seam: transient run prompts, compaction overrides,
//! checkpoint observation, boundary compaction and context rewrites.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ice_agent::message::AgentMessage;
use ice_ai::providers::faux::{FauxProvider, FauxScript, FauxStep, faux_assistant_message};
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
    BranchBounds, Entry, EntryOrder, EntryQuery, LanePointer, LaneRecord, LogItem, LogOptions,
    OperationOutcome, OperationStartedRecord, ProvisionedEntry, ProvisionedKind, RecordQuery,
    SessionMetadata, SessionStats, SessionStorage,
};
use ice_harness::session::{DefaultIdGenerator, Session};
use ice_harness::types::{
    AgentHarnessOptions, AgentHarnessResources, CompactionSettings, HarnessTool, PromptTemplate,
};
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

struct RejectMemoryStorage(Arc<dyn SessionStorage>);

#[async_trait]
impl SessionStorage for RejectMemoryStorage {
    fn metadata(&self) -> SessionMetadata {
        self.0.metadata()
    }
    async fn get_lanes(&self) -> ice_harness::error::SessionResult<Vec<LanePointer>> {
        self.0.get_lanes().await
    }
    async fn create_lane(
        &self,
        lane: &str,
        at: Option<&str>,
    ) -> ice_harness::error::SessionResult<()> {
        self.0.create_lane(lane, at).await
    }
    async fn move_lane(
        &self,
        lane: &str,
        to: Option<&str>,
    ) -> ice_harness::error::SessionResult<()> {
        self.0.move_lane(lane, to).await
    }
    async fn append_entry(
        &self,
        entry: ProvisionedEntry,
        lane: &str,
    ) -> ice_harness::error::SessionResult<Entry> {
        if let ProvisionedKind::Message { message, .. } = &entry.kind {
            if ice_harness::messages::custom_data(message).is_some_and(|data| {
                matches!(data.custom_type.as_str(), "test-memory" | "request-memory")
            }) {
                return Err(ice_harness::error::SessionError::storage(
                    "injected failure",
                ));
            }
        }
        self.0.append_entry(entry, lane).await
    }
    async fn append_record(
        &self,
        record: LaneRecord,
    ) -> ice_harness::error::SessionResult<LaneRecord> {
        self.0.append_record(record).await
    }
    async fn get_entry(&self, id: &str) -> ice_harness::error::SessionResult<Option<Entry>> {
        self.0.get_entry(id).await
    }
    async fn find_entries(
        &self,
        query: &EntryQuery,
    ) -> ice_harness::error::SessionResult<Vec<Entry>> {
        self.0.find_entries(query).await
    }
    async fn find_entries_on_branch(
        &self,
        query: &EntryQuery,
        bounds: &BranchBounds,
        start: &str,
    ) -> ice_harness::error::SessionResult<Vec<Entry>> {
        self.0.find_entries_on_branch(query, bounds, start).await
    }
    async fn find_records(
        &self,
        query: &RecordQuery,
    ) -> ice_harness::error::SessionResult<Vec<LaneRecord>> {
        self.0.find_records(query).await
    }
    async fn find_open_operations(
        &self,
        lane: &str,
        limit: Option<usize>,
    ) -> ice_harness::error::SessionResult<Vec<OperationStartedRecord>> {
        self.0.find_open_operations(lane, limit).await
    }
    async fn get_log(
        &self,
        options: &LogOptions,
    ) -> ice_harness::error::SessionResult<Vec<LogItem>> {
        self.0.get_log(options).await
    }
    async fn get_name(&self) -> ice_harness::error::SessionResult<Option<String>> {
        self.0.get_name().await
    }
    async fn set_name(&self, name: Option<&str>) -> ice_harness::error::SessionResult<()> {
        self.0.set_name(name).await
    }
    async fn get_label(&self, id: &str) -> ice_harness::error::SessionResult<Option<String>> {
        self.0.get_label(id).await
    }
    async fn set_label(
        &self,
        id: &str,
        label: Option<&str>,
    ) -> ice_harness::error::SessionResult<()> {
        self.0.set_label(id, label).await
    }
    async fn get_stats(&self) -> ice_harness::error::SessionResult<SessionStats> {
        self.0.get_stats().await
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

#[derive(Default)]
struct RequestProbe {
    events: Mutex<Vec<(String, Option<String>, u64, u64, Option<i64>, usize)>>,
    acknowledgments: Mutex<Vec<bool>>,
}

#[async_trait]
impl HarnessExtension for RequestProbe {
    async fn request_context(
        &self,
        event: &ice_harness::extension::RequestContextEvent<'_>,
    ) -> Option<PromptContext> {
        let texts = event
            .messages
            .iter()
            .filter_map(|message| match message {
                AgentMessage::Custom(custom)
                    if custom
                        .data
                        .get("customType")
                        .and_then(serde_json::Value::as_str)
                        == Some("test-memory") =>
                {
                    Some(())
                }
                _ => None,
            })
            .count();
        self.events.lock().unwrap().push((
            event.prompt.to_string(),
            event.original_prompt.map(str::to_string),
            event.request_index,
            event.branch_generation,
            event.latest_compaction_at,
            texts,
        ));
        (event.request_index == 1).then(|| PromptContext {
            custom_type: "test-memory".into(),
            text: "READY MEMORY".into(),
            correlation_id: Some("stable-correlation".into()),
        })
    }

    async fn prompt_context_persisted(&self, _context: &PromptContext, persisted: bool) {
        self.acknowledgments.lock().unwrap().push(persisted);
    }
}

#[async_trait]
impl HarnessExtension for MemoryProbe {
    async fn prompt_context(&self, event: &BeforeRunEvent<'_>) -> Option<PromptContext> {
        Some(PromptContext {
            custom_type: "test-memory".into(),
            text: format!("MEMORY for: {}", event.prompt),
            correlation_id: None,
        })
    }

    async fn prompt_context_persisted(&self, _context: &PromptContext, persisted: bool) {
        self.persisted.lock().unwrap().push(persisted);
    }
}

struct PacketExtension {
    packet: PromptContext,
    acknowledgments: Mutex<Vec<bool>>,
    every_request: bool,
}

#[async_trait]
impl HarnessExtension for PacketExtension {
    async fn request_context(
        &self,
        event: &ice_harness::extension::RequestContextEvent<'_>,
    ) -> Option<PromptContext> {
        (self.every_request || event.request_index == 0).then(|| self.packet.clone())
    }

    async fn prompt_context_persisted(&self, _context: &PromptContext, persisted: bool) {
        self.acknowledgments.lock().unwrap().push(persisted);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shared_request_budget_rejects_second_packet_intact() {
    let (agent_harness, provider, _) = harness(vec![FauxStep::text("done")]).await;
    let first = Arc::new(PacketExtension {
        packet: PromptContext {
            custom_type: "request-memory".into(),
            text: "a".repeat(40),
            correlation_id: None,
        },
        acknowledgments: Mutex::new(Vec::new()),
        every_request: false,
    });
    let second = Arc::new(PacketExtension {
        packet: PromptContext {
            custom_type: "request-memory".into(),
            text: "b".repeat(40),
            correlation_id: None,
        },
        acknowledgments: Mutex::new(Vec::new()),
        every_request: false,
    });
    agent_harness.set_extensions(vec![
        first.clone() as Arc<dyn HarnessExtension>,
        second.clone() as Arc<dyn HarnessExtension>,
    ]);
    agent_harness
        .prompt_text("budget", Vec::new())
        .await
        .unwrap();
    let contexts = provider.contexts.lock().unwrap();
    let texts = user_texts(&contexts[0]);
    assert!(texts.iter().any(|text| text == &"a".repeat(40)));
    assert!(!texts.iter().any(|text| text == &"b".repeat(40)));
    assert_eq!(first.acknowledgments.lock().unwrap().as_slice(), &[true]);
    assert_eq!(second.acknowledgments.lock().unwrap().as_slice(), &[false]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn duplicate_correlation_is_acknowledged_false_and_persisted_once() {
    let (harness, provider, _) = harness(vec![
        FauxStep::tool_call("read", serde_json::json!({"path":"missing"})),
        FauxStep::text("done"),
    ])
    .await;
    let execution = Arc::new(InMemoryExecutionEnv::new());
    let env: Arc<dyn ice_tools::ExecutionEnv> = execution.clone();
    let mut_env: Arc<dyn ice_tools::MutatingEnv> = execution;
    harness
        .set_tools(
            vec![HarnessTool::new(ice_tools::create_read_tool(
                &ExecutionToolContext::new(env, Some(mut_env)),
                None,
            ))],
            Some(vec!["read".into()]),
        )
        .await
        .unwrap();
    let packet = Arc::new(PacketExtension {
        packet: PromptContext {
            custom_type: "request-memory".into(),
            text: "once".into(),
            correlation_id: Some("same-id".into()),
        },
        acknowledgments: Mutex::new(Vec::new()),
        every_request: true,
    });
    harness.set_extensions(vec![packet.clone() as Arc<dyn HarnessExtension>]);
    harness.prompt_text("duplicate", Vec::new()).await.unwrap();
    assert_eq!(
        packet.acknowledgments.lock().unwrap().as_slice(),
        &[true, false]
    );
    assert_eq!(provider.contexts.lock().unwrap().len(), 2);
    let context_count = user_texts(&provider.contexts.lock().unwrap()[0])
        .iter()
        .filter(|text| text.contains("once"))
        .count();
    assert_eq!(context_count, 1);
    let entries = harness
        .session()
        .view("main")
        .find_entries(&EntryQuery {
            order: Some(EntryOrder::OldestFirst),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(entries.iter().filter(|entry| matches!(entry, Entry::Message(message) if ice_harness::messages::custom_data(&message.message).is_some_and(|d| d.custom_type == "request-memory"))).count(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn request_context_append_failure_does_not_inject_or_acknowledge() {
    let storage = Arc::new(InMemorySessionStorage::new(
        SessionMetadata {
            id: "failed-request".into(),
            created_at: 0,
            parent_session_id: None,
        },
        Arc::new(SystemClock),
        Arc::new(DefaultIdGenerator::new()),
    ));
    let session = Session::new(Arc::new(RejectMemoryStorage(storage)), None);
    let faux = FauxProvider::new(FauxScript::new());
    faux.script()
        .set_responses(vec![FauxStep::text("main answer")]);
    let provider = Arc::new(RecordingProvider {
        inner: faux.clone(),
        system_prompts: Mutex::new(Vec::new()),
        contexts: Mutex::new(Vec::new()),
    });
    let harness = AgentHarness::create(AgentHarnessOptions {
        model: faux.default_model().clone(),
        session,
        models: vec![provider.clone() as Arc<dyn Provider>],
        ..AgentHarnessOptions::default()
    })
    .await
    .unwrap();
    let packet = Arc::new(PacketExtension {
        packet: PromptContext {
            custom_type: "request-memory".into(),
            text: "do not inject".into(),
            correlation_id: None,
        },
        acknowledgments: Mutex::new(Vec::new()),
        every_request: false,
    });
    harness.set_extensions(vec![packet.clone() as Arc<dyn HarnessExtension>]);
    let result = harness.prompt_text("continues", Vec::new()).await.unwrap();
    assert!(matches!(
        result.outcome,
        ice_harness::agent_harness::HarnessRunOutcome::Completed { .. }
    ));
    assert_eq!(packet.acknowledgments.lock().unwrap().as_slice(), &[false]);
    assert!(
        !user_texts(&provider.contexts.lock().unwrap()[0])
            .iter()
            .any(|text| text == "do not inject")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn same_session_tree_navigation_increments_request_epoch() {
    let (harness, _provider, _) = harness(vec![FauxStep::text("one"), FauxStep::text("two")]).await;
    let probe = Arc::new(RequestProbe::default());
    harness.set_extensions(vec![probe.clone() as Arc<dyn HarnessExtension>]);
    let session_id = harness.session().storage().metadata().id;
    harness.prompt_text("first", Vec::new()).await.unwrap();
    let before = probe.events.lock().unwrap()[0].3;
    let view = harness.session().view("main");
    let path = view
        .find_entries_on_branch(
            &EntryQuery {
                order: Some(EntryOrder::OldestFirst),
                ..Default::default()
            },
            &BranchBounds::default(),
        )
        .await
        .unwrap();
    let target = path.first().unwrap().id().to_string();
    let navigation = harness
        .navigate_tree(Some(&target), false, None, None)
        .await
        .unwrap();
    assert!(matches!(
        navigation.outcome,
        ice_harness::agent_harness::NavigationOutcome::Completed { .. }
    ));
    harness.prompt_text("second", Vec::new()).await.unwrap();
    let events = probe.events.lock().unwrap();
    assert_eq!(harness.session().storage().metadata().id, session_id);
    assert!(events[1].3 > before);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn plain_prompt_is_raw_intent_but_expanded_or_tampered_messages_are_not() {
    let (agent_harness, _, _) = harness(vec![FauxStep::text("done")]).await;
    let probe = Arc::new(RequestProbe::default());
    agent_harness.set_extensions(vec![probe.clone() as Arc<dyn HarnessExtension>]);
    agent_harness
        .prompt_message(AgentMessage::User(UserMessage::new(
            "ordinary history query",
            123,
        )))
        .await
        .unwrap();
    assert_eq!(
        probe.events.lock().unwrap()[0].1.as_deref(),
        Some("ordinary history query")
    );

    let expanded = AgentMessage::User(UserMessage::new(
        "<skill name=\"x\">instructions</skill>",
        124,
    ));
    assert_eq!(
        ice_agent::message::original_user_prompt(&expanded),
        None,
        "legacy expanded payload must not be treated as raw invocation"
    );
    let mut tampered = UserMessage::new("changed expanded content", 125)
        .with_origin("$review", "skill-expansion-v1");
    tampered.content = UserContent::Text("changed after provenance was attached".into());
    let tampered = AgentMessage::User(tampered);
    assert_eq!(ice_agent::message::original_user_prompt(&tampered), None);
    assert_eq!(
        match &tampered {
            AgentMessage::User(user) => user.origin.as_ref().and_then(|_| user.original_prompt()),
            _ => None,
        },
        None,
        "tampered metadata must not recover the previously claimed raw prompt"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prompt_template_keeps_expansion_and_raw_invocation_distinct() {
    let (harness, provider, _) = harness(vec![FauxStep::text("done")]).await;
    harness
        .set_resources(AgentHarnessResources {
            prompt_templates: Some(vec![PromptTemplate {
                name: "summarize".into(),
                description: None,
                content: "Summarize $1".into(),
            }]),
            ..AgentHarnessResources::default()
        })
        .await
        .unwrap();
    let probe = Arc::new(RequestProbe::default());
    harness.set_extensions(vec![probe.clone() as Arc<dyn HarnessExtension>]);
    harness
        .prompt_from_template("summarize", &["the report".into()])
        .await
        .unwrap();
    assert_eq!(
        probe.events.lock().unwrap()[0].1.as_deref(),
        Some("summarize the report")
    );
    assert!(
        user_texts(&provider.contexts.lock().unwrap()[0])
            .iter()
            .any(|text| text == "Summarize the report")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn request_context_waits_for_existing_tool_turn_and_uses_raw_origin() {
    let steps = vec![
        FauxStep::tool_call("read", serde_json::json!({"path": "missing.txt"})),
        FauxStep::text("done"),
    ];
    let (harness, provider, _) = harness(steps).await;
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
            Some(vec!["read".into()]),
        )
        .await
        .unwrap();
    let probe = Arc::new(RequestProbe::default());
    harness.set_extensions(vec![probe.clone() as Arc<dyn HarnessExtension>]);
    let prompt = AgentMessage::User(
        UserMessage::new("expanded skill instructions", 123)
            .with_origin("$review", "skill-expansion-v1"),
    );
    harness.prompt_message(prompt).await.unwrap();
    let events = probe.events.lock().unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].1.as_deref(), Some("$review"));
    assert_eq!(events[1].1.as_deref(), Some("$review"));
    assert_eq!(events[1].5, 0);
    assert_eq!(events[0].3, events[1].3);
    assert_eq!(provider.contexts.lock().unwrap().len(), 2);
    assert!(
        user_texts(&provider.contexts.lock().unwrap()[0])
            .iter()
            .all(|text| !text.contains("READY MEMORY"))
    );
    assert!(
        user_texts(&provider.contexts.lock().unwrap()[1])
            .iter()
            .any(|text| text.contains("READY MEMORY"))
    );
    assert_eq!(probe.acknowledgments.lock().unwrap().as_slice(), &[true]);
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
    assert!(
        hidden
            .iter()
            .all(|data| data.custom_type == "test-memory" && !data.display)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_prompt_context_append_is_not_sent_to_provider() {
    let storage = Arc::new(InMemorySessionStorage::new(
        SessionMetadata {
            id: "failed-memory".into(),
            created_at: 0,
            parent_session_id: None,
        },
        Arc::new(SystemClock),
        Arc::new(DefaultIdGenerator::new()),
    ));
    let session = Session::new(Arc::new(RejectMemoryStorage(storage)), None);
    let script = FauxScript::new();
    script.set_responses(vec![FauxStep::text("done")]);
    let faux = FauxProvider::new(script);
    let provider = Arc::new(RecordingProvider {
        inner: faux.clone(),
        system_prompts: Mutex::new(Vec::new()),
        contexts: Mutex::new(Vec::new()),
    });
    let harness = AgentHarness::create(AgentHarnessOptions {
        model: faux.default_model().clone(),
        session,
        models: vec![provider.clone() as Arc<dyn Provider>],
        system_prompt: Some("BASE".into()),
        ..AgentHarnessOptions::default()
    })
    .await
    .unwrap();
    let memory = Arc::new(MemoryProbe::default());
    harness.set_extensions(vec![memory.clone() as Arc<dyn HarnessExtension>]);
    harness.prompt_text("question", Vec::new()).await.unwrap();
    assert_eq!(memory.persisted.lock().unwrap().as_slice(), &[false]);
    let contexts = provider.contexts.lock().unwrap();
    assert_eq!(contexts.len(), 1);
    assert!(
        !user_texts(&contexts[0])
            .iter()
            .any(|text| text.contains("MEMORY for:"))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn request_context_observes_midrun_compaction_once_with_bounded_context() {
    let steps = vec![
        FauxStep::tool_call("read", serde_json::json!({"path": "missing.txt"})),
        FauxStep::text("done"),
    ];
    let (harness, provider, boundary_probe) = harness(steps).await;
    *boundary_probe.boundary.lock().unwrap() = Some(BoundaryDecision::Compact { pause: false });
    *boundary_probe.decision.lock().unwrap() =
        Some(BeforeCompactDecision::Replace(CompactionOverride {
            summary: "REQUEST CONTEXT CHECKPOINT".into(),
            retained_tail: Some(Vec::new()),
            details: None,
        }));
    let execution = Arc::new(InMemoryExecutionEnv::new());
    let env: Arc<dyn ice_tools::ExecutionEnv> = execution.clone();
    let mut_env: Arc<dyn ice_tools::MutatingEnv> = execution;
    harness
        .set_tools(
            vec![HarnessTool::new(ice_tools::create_read_tool(
                &ExecutionToolContext::new(env, Some(mut_env)),
                None,
            ))],
            Some(vec!["read".into()]),
        )
        .await
        .unwrap();
    let probe = Arc::new(RequestProbe::default());
    harness.set_extensions(vec![
        probe.clone() as Arc<dyn HarnessExtension>,
        boundary_probe.clone() as Arc<dyn HarnessExtension>,
    ]);
    harness
        .prompt_text("compact boundary", Vec::new())
        .await
        .unwrap();
    assert_eq!(boundary_probe.transform_times.lock().unwrap().len(), 2);
    let events = probe.events.lock().unwrap();
    assert_eq!(events.len(), 2);
    assert!(
        events[1].4.is_some(),
        "latest compaction timestamp was not refreshed: {:?}",
        events.iter().map(|event| event.4).collect::<Vec<_>>()
    );
    let contexts = provider.contexts.lock().unwrap();
    assert_eq!(contexts.len(), 2);
    assert!(
        user_texts(&contexts[1])
            .iter()
            .all(|text| !text.contains("OLD "))
    );
    assert_eq!(
        events.len(),
        2,
        "request callback is once per provider request"
    );
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
    assert!(
        triggers
            .iter()
            .any(
                |(trigger, from_extension, summary)| *trigger == CompactionTrigger::MidRun
                    && *from_extension
                    && summary == "MIDRUN CHECKPOINT"
            )
    );
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
