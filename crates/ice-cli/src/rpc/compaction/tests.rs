use super::*;
use ice_ai::providers::faux::{faux_assistant_message, FauxProvider, FauxScript, FauxStep};
use ice_ai::types::{StopReason, UserMessage};
use ice_harness::compaction::settings::CodingAgentCompactionPolicy;
use ice_harness::error::{SessionError, SessionResult};
use ice_harness::extension::CompactionOverride;
use ice_harness::session::memory::{InMemorySessionStorage, SystemClock};
use ice_harness::session::types::*;
use ice_harness::session::{DefaultIdGenerator, Session};
use ice_harness::types::AgentHarnessOptions;
use serde_json::json;

#[derive(Default)]
struct Gate {
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}

struct FaultStorage {
    inner: InMemorySessionStorage,
    fault: u8,
    gate: Option<Arc<Gate>>,
}

#[async_trait::async_trait]
impl SessionStorage for FaultStorage {
    fn metadata(&self) -> SessionMetadata {
        self.inner.metadata()
    }
    async fn append_entry(&self, entry: ProvisionedEntry, lane: &str) -> SessionResult<Entry> {
        if matches!(&entry.kind, ProvisionedKind::Compaction { .. }) {
            if let Some(gate) = &self.gate {
                gate.entered.notify_one();
                gate.release.notified().await;
            }
            if self.fault == 1 {
                return Err(SessionError::storage("injected checkpoint append failure"));
            }
        }
        self.inner.append_entry(entry, lane).await
    }
    async fn append_record(&self, record: LaneRecord) -> SessionResult<LaneRecord> {
        if self.fault == 2
            && matches!(&record, LaneRecord::StepAttempt(step) if step.step == StepKind::Compaction)
        {
            return Err(SessionError::storage("injected checkpoint record failure"));
        }
        self.inner.append_record(record).await
    }
    async fn get_lanes(&self) -> SessionResult<Vec<LanePointer>> {
        self.inner.get_lanes().await
    }
    async fn create_lane(&self, lane: &str, at: Option<&str>) -> SessionResult<()> {
        self.inner.create_lane(lane, at).await
    }
    async fn move_lane(&self, lane: &str, to: Option<&str>) -> SessionResult<()> {
        self.inner.move_lane(lane, to).await
    }
    async fn get_entry(&self, id: &str) -> SessionResult<Option<Entry>> {
        self.inner.get_entry(id).await
    }
    async fn find_entries(&self, query: &EntryQuery) -> SessionResult<Vec<Entry>> {
        self.inner.find_entries(query).await
    }
    async fn find_entries_on_branch(
        &self,
        query: &EntryQuery,
        bounds: &BranchBounds,
        start: &str,
    ) -> SessionResult<Vec<Entry>> {
        self.inner
            .find_entries_on_branch(query, bounds, start)
            .await
    }
    async fn find_records(&self, query: &RecordQuery) -> SessionResult<Vec<LaneRecord>> {
        self.inner.find_records(query).await
    }
    async fn find_open_operations(
        &self,
        lane: &str,
        limit: Option<usize>,
    ) -> SessionResult<Vec<OperationStartedRecord>> {
        self.inner.find_open_operations(lane, limit).await
    }
    async fn get_log(&self, options: &LogOptions) -> SessionResult<Vec<LogItem>> {
        self.inner.get_log(options).await
    }
    async fn get_name(&self) -> SessionResult<Option<String>> {
        self.inner.get_name().await
    }
    async fn set_name(&self, name: Option<&str>) -> SessionResult<()> {
        self.inner.set_name(name).await
    }
    async fn get_label(&self, id: &str) -> SessionResult<Option<String>> {
        self.inner.get_label(id).await
    }
    async fn set_label(&self, id: &str, label: Option<&str>) -> SessionResult<()> {
        self.inner.set_label(id, label).await
    }
    async fn get_stats(&self) -> SessionResult<SessionStats> {
        self.inner.get_stats().await
    }
}

enum Decision {
    Replace,
    Decline,
    CancelSignal,
}

#[async_trait::async_trait]
impl HarnessExtension for Decision {
    async fn before_compact(&self, event: &BeforeCompactEvent) -> Option<BeforeCompactDecision> {
        match self {
            Self::Replace => Some(BeforeCompactDecision::Replace(CompactionOverride {
                summary: "The exact durable summary.".into(),
                retained_tail: Some(vec![]),
                details: None,
            })),
            Self::Decline => Some(BeforeCompactDecision::Cancel),
            Self::CancelSignal => {
                event.signal.cancel();
                None
            }
        }
    }
}

async fn fixture(
    fault: u8,
    gate: Option<Arc<Gate>>,
    provider_error: bool,
) -> (AgentHarness, Arc<FaultStorage>) {
    let storage = Arc::new(FaultStorage {
        inner: InMemorySessionStorage::new(
            SessionMetadata {
                id: uuid::Uuid::new_v4().to_string(),
                created_at: 0,
                parent_session_id: None,
            },
            Arc::new(SystemClock),
            Arc::new(DefaultIdGenerator::new()),
        ),
        fault,
        gate,
    });
    let session = Session::new(storage.clone(), None);
    // The recent turn must fill the retained budget; otherwise preparation
    // also retains the older turn and has no history left to summarize.
    for text in ["old history ".repeat(1000), "recent history ".repeat(30)] {
        session
            .view("main")
            .append_message(AgentMessage::User(UserMessage::new(text, 1)))
            .await
            .unwrap();
        session
            .view("main")
            .append_message(AgentMessage::Assistant(Box::new(faux_assistant_message(
                "earlier response",
                StopReason::Stop,
            ))))
            .await
            .unwrap();
    }
    let script = FauxScript::new()
        .with_text("continued")
        .with_tokens_per_second(0.0);
    if provider_error {
        let mut error = faux_assistant_message("", StopReason::Error);
        error.error_message = Some("summarizer provider failed".into());
        script.set_responses(vec![FauxStep::message(error)]);
    }
    let provider = FauxProvider::new(script);
    let mut model = provider.default_model().clone();
    model.context_window = 1000;
    model.max_tokens = 200;
    let mut options = AgentHarnessOptions {
        session,
        model,
        models: vec![provider],
        allow_existing_session: true,
        ..Default::default()
    };
    options.retry.enabled = false;
    let harness = AgentHarness::create(options).await.unwrap();
    harness
        .set_coding_compaction_policy(Some(CodingAgentCompactionPolicy {
            keep_recent_tokens: 50,
            ..Default::default()
        }))
        .await
        .unwrap();
    (harness, storage)
}

fn observed(harness: &AgentHarness) -> (CompactionGuard, Arc<Mutex<Vec<Value>>>) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let target = Arc::clone(&events);
    let guard = CompactionGuard::attach(
        harness,
        Arc::new(move |event| {
            target.lock().unwrap().push(event);
        }),
    );
    (guard, events)
}

fn assert_terminal(events: &[Value], status: &str) {
    assert_eq!(events.len(), 2, "{events:?}");
    assert_eq!(events[0]["status"], "in_progress");
    assert_eq!(events[1]["status"], status);
    assert_eq!(events[0]["compactionId"], events[1]["compactionId"]);
    assert!(uuid::Uuid::parse_str(events[0]["compactionId"].as_str().unwrap()).is_ok());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn completion_waits_for_the_durable_checkpoint() {
    let gate = Arc::new(Gate::default());
    let (harness, storage) = fixture(0, Some(gate.clone()), false).await;
    harness.set_extensions(vec![Arc::new(Decision::Replace)]);
    let (_guard, events) = observed(&harness);
    let running = harness.clone();
    let task =
        tokio::spawn(async move { running.lane("main").prompt_text("continue", vec![]).await });
    tokio::time::timeout(std::time::Duration::from_secs(5), gate.entered.notified())
        .await
        .unwrap();
    assert_eq!(events.lock().unwrap().len(), 1);
    assert!(!storage
        .find_entries(&EntryQuery::default())
        .await
        .unwrap()
        .iter()
        .any(|entry| matches!(entry, Entry::Compaction(_))));
    gate.release.notify_one();
    assert!(task.await.unwrap().is_ok());
    let events = events.lock().unwrap();
    assert_terminal(&events, "completed");
    assert_eq!(events[1]["summary"], "The exact durable summary.");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn checkpoint_and_step_record_failures_never_report_completion() {
    for fault in [1, 2] {
        let (harness, storage) = fixture(fault, None, false).await;
        harness.set_extensions(vec![Arc::new(Decision::Replace)]);
        let (guard, events) = observed(&harness);
        let result = harness.lane("main").prompt_text("continue", vec![]).await;
        guard.observer().settle(
            result
                .as_ref()
                .err()
                .map(|error| error.to_string())
                .as_deref(),
        );
        assert_terminal(&events.lock().unwrap(), "failed");
        let checkpoints = storage
            .find_entries(&EntryQuery::default())
            .await
            .unwrap()
            .into_iter()
            .filter(|entry| matches!(entry, Entry::Compaction(_)))
            .count();
        assert_eq!(checkpoints, usize::from(fault == 2));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn summarizer_provider_failure_closes_the_attempt() {
    let (harness, storage) = fixture(0, None, true).await;
    let (guard, events) = observed(&harness);
    let result = harness.lane("main").prompt_text("continue", vec![]).await;
    guard.observer().settle(
        result
            .as_ref()
            .err()
            .map(|error| error.to_string())
            .as_deref(),
    );
    assert_terminal(&events.lock().unwrap(), "failed");
    assert!(!storage
        .find_entries(&EntryQuery::default())
        .await
        .unwrap()
        .iter()
        .any(|entry| matches!(entry, Entry::Compaction(_))));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn extension_veto_and_native_cancellation_close_without_a_checkpoint() {
    for decision in [Decision::Decline, Decision::CancelSignal] {
        let (harness, storage) = fixture(0, None, false).await;
        harness.set_extensions(vec![Arc::new(decision)]);
        let (guard, events) = observed(&harness);
        let result = harness.lane("main").prompt_text("continue", vec![]).await;
        guard.observer().settle(
            result
                .as_ref()
                .err()
                .map(|error| error.to_string())
                .as_deref(),
        );
        assert_terminal(&events.lock().unwrap(), "cancelled");
        assert!(!storage
            .find_entries(&EntryQuery::default())
            .await
            .unwrap()
            .iter()
            .any(|entry| matches!(entry, Entry::Compaction(_))));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn manual_compaction_does_not_duplicate_bridge_progress() {
    let (harness, _) = fixture(0, None, false).await;
    harness.set_extensions(vec![Arc::new(Decision::Replace)]);
    let (_guard, events) = observed(&harness);
    assert!(harness.lane("main").compact(None).await.is_ok());
    assert!(events.lock().unwrap().is_empty());
}

struct Probe {
    name: &'static str,
    calls: Arc<Mutex<Vec<String>>>,
}

#[async_trait::async_trait]
impl HarnessExtension for Probe {
    async fn run_start(&self) {
        self.calls
            .lock()
            .unwrap()
            .push(format!("{}:start", self.name));
    }
    async fn prompt_context(&self, _event: &BeforeRunEvent<'_>) -> Option<PromptContext> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("{}:context", self.name));
        Some(PromptContext {
            custom_type: self.name.into(),
            text: "context".into(),
            correlation_id: None,
        })
    }
    async fn prompt_context_persisted(&self, _context: &PromptContext, persisted: bool) {
        self.calls
            .lock()
            .unwrap()
            .push(format!("{}:persisted:{persisted}", self.name));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn observer_preserves_extension_order_defaults_and_restores_identity() {
    let (harness, _) = fixture(0, None, false).await;
    let calls = Arc::new(Mutex::new(Vec::new()));
    let original: HarnessExtensions = ["first", "second"]
        .into_iter()
        .map(|name| {
            Arc::new(Probe {
                name,
                calls: calls.clone(),
            }) as Arc<dyn HarnessExtension>
        })
        .collect();
    harness.set_extensions(original.clone());
    let (guard, _) = observed(&harness);
    let event = RequestContextEvent {
        prompt: "p",
        original_prompt: None,
        messages: &[],
        system_prompt: "",
        session_id: "other",
        run_id: "r",
        lane_id: "main",
        branch_generation: 0,
        request_index: 0,
        context_window: 1000,
        remaining_context_tokens: 1000,
        latest_compaction_at: None,
        signal: CancellationToken::new(),
    };
    for extension in harness.extensions() {
        extension.run_start().await;
    }
    for extension in harness.extensions() {
        if let Some(context) = extension.request_context(&event).await {
            extension
                .prompt_context_acknowledged(&context, PromptContextOutcome::Persisted)
                .await;
        }
    }
    assert_eq!(
        *calls.lock().unwrap(),
        [
            "first:start",
            "second:start",
            "first:context",
            "first:persisted:true",
            "second:context",
            "second:persisted:true",
        ]
    );
    drop(guard);
    let restored = harness.extensions();
    assert_eq!(restored.len(), original.len());
    assert!(restored
        .iter()
        .zip(original.iter())
        .all(|(a, b)| Arc::ptr_eq(a, b)));
}

#[test]
fn new_native_event_round_trips_without_becoming_a_turn_terminal() {
    let event = RemoteEvent::CompactionStatus {
        compaction_id: "c".into(),
        status: CompactionStatus::Completed,
        summary: Some("summary".into()),
        error: None,
    };
    let wire = serde_json::to_value(&event).unwrap();
    assert_eq!(
        wire,
        json!({"type": "compaction_status", "compactionId": "c",
        "status": "completed", "summary": "summary"})
    );
    assert_eq!(serde_json::from_value::<RemoteEvent>(wire).unwrap(), event);
    assert!(!event.is_terminal());
}
