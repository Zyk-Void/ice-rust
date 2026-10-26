//! Integration coverage for the opt-in coding-agent compaction policy.
//!
//! The product policy runs before a new prompt is persisted, while a harness
//! with no product policy keeps the generic after-prompt compaction order.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ice_agent::message::AgentMessage;
use ice_ai::providers::faux::{faux_assistant_message, FauxProvider, FauxScript, FauxStep};
use ice_ai::types::{
    AssistantMessage, AssistantMessageEvent, Context, DoneReason, Message, StopReason, UserContent,
    UserMessage,
};
use ice_ai::{AssistantMessageEventStream, Model, Provider, SimpleStreamOptions};
use ice_harness::agent_harness::{AgentHarness, AgentLane, CompactionOutcome};
use ice_harness::compaction::settings::{CodingAgentCompactionPolicy, MidRunCompactionMode};
use ice_harness::session::memory::{InMemorySessionStorage, SystemClock};
use ice_harness::session::types::{
    BranchBounds, CompactionReason, Entry, EntryOrder, EntryQuery, LanePointer, LaneRecord,
    LogItem, LogOptions, OperationStartedRecord, ProvisionedEntry, RecordQuery, SessionMetadata,
    SessionStats, SessionStorage, StepKind,
};
use ice_harness::session::{DefaultIdGenerator, Session};
use ice_harness::types::{AgentHarnessOptions, CompactionSettings, HarnessTool};
use ice_tools::{ExecutionToolContext, InMemoryExecutionEnv};

#[derive(Debug, Clone)]
struct RecordedRequest {
    context: String,
    max_tokens: Option<u64>,
}

struct RecordingProvider {
    inner: Arc<FauxProvider>,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    cancel_after_response: AtomicBool,
    cancel_before_response: AtomicBool,
    request_started: Arc<tokio::sync::Notify>,
    pause_next_response: AtomicBool,
    response_paused: Arc<tokio::sync::Notify>,
    release_response: Arc<tokio::sync::Notify>,
}

impl RecordingProvider {
    fn new(inner: Arc<FauxProvider>) -> Arc<Self> {
        Arc::new(Self {
            inner,
            requests: Arc::new(Mutex::new(Vec::new())),
            cancel_after_response: AtomicBool::new(false),
            cancel_before_response: AtomicBool::new(false),
            request_started: Arc::new(tokio::sync::Notify::new()),
            pause_next_response: AtomicBool::new(false),
            response_paused: Arc::new(tokio::sync::Notify::new()),
            release_response: Arc::new(tokio::sync::Notify::new()),
        })
    }

    fn requests(&self) -> Vec<RecordedRequest> {
        self.requests.lock().unwrap().clone()
    }

    fn cancel_after_next_response(&self) {
        self.cancel_after_response.store(true, Ordering::SeqCst);
    }

    fn cancel_before_next_response(&self) {
        self.cancel_before_response.store(true, Ordering::SeqCst);
    }

    fn pause_next_response(&self) {
        self.pause_next_response.store(true, Ordering::SeqCst);
    }
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
        self.requests.lock().unwrap().push(RecordedRequest {
            context: context_text(context),
            max_tokens: options.max_tokens,
        });
        if self.cancel_before_response.swap(false, Ordering::SeqCst) {
            self.request_started.notify_one();
            options.signal.cancelled().await;
            let response = AssistantMessage::terminal(
                model.api.clone(),
                self.id(),
                model.id.clone(),
                StopReason::Aborted,
                "summary request cancelled before response",
                0,
            );
            let (mut producer, stream) =
                ice_ai::event_stream::create_assistant_message_event_stream();
            producer.push(AssistantMessageEvent::Start {
                partial: Arc::new(response.clone()),
            });
            producer.push(AssistantMessageEvent::Error {
                reason: ice_ai::types::ErrorReason::Aborted,
                error: response,
            });
            stream
        } else if self.cancel_after_response.swap(false, Ordering::SeqCst) {
            let mut response = faux_assistant_message(
                "## Goal\nContinue the existing task.\n\n## Constraints & Preferences\n- None\n\n## Progress\n### Done\n- [x] Existing history reviewed\n\n### In Progress\n- [ ] Continue the task\n\n### Blocked\n- None\n\n## Key Decisions\n- **Keep the existing context**: preserve the prior work.\n\n## Next Steps\n1. Continue from the retained context.\n\n## Critical Context\n- Preserve the old history marker.",
                StopReason::Stop,
            );
            response.model = model.id.clone();
            let (mut producer, stream) =
                ice_ai::event_stream::create_assistant_message_event_stream();
            producer.push(AssistantMessageEvent::Start {
                partial: Arc::new(response.clone()),
            });
            producer.push(AssistantMessageEvent::Done {
                reason: DoneReason::Stop,
                message: response,
            });
            // A terminal success arrives first; this cancellation is deliberately
            // late, between completion and checkpoint persistence.
            options.signal.cancel();
            stream
        } else if self.pause_next_response.swap(false, Ordering::SeqCst) {
            self.request_started.notify_one();
            self.response_paused.notify_one();
            self.release_response.notified().await;
            self.inner.stream_simple(model, context, options).await
        } else {
            self.inner.stream_simple(model, context, options).await
        }
    }
}

fn context_text(context: &Context) -> String {
    let mut text = context.system_prompt.clone().unwrap_or_default();
    for message in &context.messages {
        match message {
            Message::User(message) => match &message.content {
                UserContent::Text(value) => text.push_str(value),
                UserContent::Blocks(blocks) => {
                    text.push_str(&ice_ai::types::Content::text_only(blocks, "\n"));
                }
            },
            Message::Assistant(message) => {
                text.push_str(&ice_ai::types::Content::text_only(&message.content, "\n"));
            }
            Message::ToolResult(message) => {
                text.push_str(&ice_ai::types::Content::text_only(&message.content, "\n"));
            }
        }
    }
    text
}

fn script() -> FauxScript {
    let script = FauxScript::new();
    // Every call gets a valid structured response, whether it is a summary
    // request or the ordinary agent turn. This also makes a surprise provider
    // continuation observable as an extra recorded request rather than a
    // queue-exhaustion artifact.
    let step = FauxStep::factory(|_| {
        faux_assistant_message(
            "## Goal\nContinue the existing task.\n\n## Constraints & Preferences\n- None\n\n## Progress\n### Done\n- [x] Existing history reviewed\n\n### In Progress\n- [ ] Continue the task\n\n### Blocked\n- None\n\n## Key Decisions\n- **Keep the existing context**: preserve the prior work.\n\n## Next Steps\n1. Continue from the retained context.\n\n## Critical Context\n- Preserve the old history marker.",
            StopReason::Stop,
        )
    });
    script.set_responses(vec![step; 20]);
    script
}

fn new_session() -> Session {
    let metadata = SessionMetadata {
        id: "coding-compaction-policy".into(),
        created_at: 0,
        parent_session_id: None,
    };
    let storage = Arc::new(InMemorySessionStorage::new(
        metadata,
        Arc::new(SystemClock),
        Arc::new(DefaultIdGenerator::new()),
    ));
    Session::new(storage, None)
}

async fn harness_with_history(
    user_one_len: usize,
    assistant_one_len: usize,
    user_two_len: usize,
    assistant_two_len: usize,
    context_window: u64,
    max_tokens: u64,
    compaction: CompactionSettings,
) -> (AgentHarness, Arc<RecordingProvider>) {
    let session = new_session();
    let first_user = padded_text("OLD_PREFIX_SENTINEL", user_one_len);
    let first_assistant = padded_text("older answer", assistant_one_len);
    let second_user = padded_text("recent request", user_two_len);
    let second_assistant = padded_text("recent answer", assistant_two_len);

    append_user(&session, first_user).await;
    append_assistant(&session, first_assistant).await;
    append_user(&session, second_user).await;
    append_assistant(&session, second_assistant).await;

    let faux = FauxProvider::new(script());
    let provider = RecordingProvider::new(faux.clone());
    let mut model = faux.default_model().clone();
    model.context_window = context_window;
    model.max_tokens = max_tokens;
    let options = AgentHarnessOptions {
        model,
        thinking_level: Default::default(),
        active_tool_names: Vec::new(),
        tools: Vec::new(),
        system_prompt: None,
        resources: Default::default(),
        stream_options: Default::default(),
        retry: Default::default(),
        compaction,
        steering_mode: Default::default(),
        follow_up_mode: Default::default(),
        tool_execution: Default::default(),
        drive: Default::default(),
        session,
        models: vec![provider.clone() as Arc<dyn Provider>],
        to_provider_messages: None,
        entry_projectors: Default::default(),
        agent_emitter: None,
        before_tool_call: None,
        after_tool_call: None,
        transform_context: None,
        entry_transforms: Vec::new(),
        provider_hooks: None,
        allow_existing_session: true,
    };
    let harness = AgentHarness::create(options).await.expect("create harness");
    (harness, provider)
}

fn padded_text(prefix: &str, len: usize) -> String {
    assert!(len >= prefix.len());
    format!("{prefix}{}", "x".repeat(len - prefix.len()))
}

async fn append_user(session: &Session, text: String) {
    session
        .view("main")
        .append_message(AgentMessage::User(UserMessage::new(
            UserContent::Text(text),
            1,
        )))
        .await
        .expect("append user history");
}

async fn append_assistant(session: &Session, text: String) {
    let mut message = AssistantMessage::empty(ice_ai::types::Api::Faux, "faux", "faux", 1);
    message.content = vec![ice_ai::types::Content::text(text)];
    message.stop_reason = StopReason::Stop;
    session
        .view("main")
        .append_message(AgentMessage::Assistant(Box::new(message)))
        .await
        .expect("append assistant history");
}

fn small_recent_policy() -> CodingAgentCompactionPolicy {
    CodingAgentCompactionPolicy {
        keep_recent_tokens: 50,
        ..CodingAgentCompactionPolicy::default()
    }
}

#[derive(Default)]
struct AddingRequestHook {
    calls: std::sync::atomic::AtomicUsize,
}

impl ice_ai::ProviderHooks for AddingRequestHook {
    fn transform_context(&self, _model: &Model, context: &Context) -> Option<ice_ai::ContextPatch> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Some(ice_ai::ContextPatch {
            system_prompt: Some(format!(
                "{}\nHOOK_BUDGET_SENTINEL {}",
                context.system_prompt.as_deref().unwrap_or_default(),
                "extra request context ".repeat(12)
            )),
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn request_hook_safety_reserve_compacts_before_new_prompt() {
    let session = new_session();
    append_user(&session, padded_text("OLD_PREFIX_SENTINEL", 4000)).await;
    append_assistant(&session, "older answer ".repeat(100)).await;
    append_user(&session, "recent request".into()).await;
    append_assistant(&session, "recent answer".into()).await;
    let faux = FauxProvider::new(script());
    let provider = RecordingProvider::new(faux.clone());
    let hook = Arc::new(AddingRequestHook::default());
    let mut model = faux.default_model().clone();
    model.max_tokens = 100;
    let harness = AgentHarness::create(AgentHarnessOptions {
        model: model.clone(),
        session,
        models: vec![provider.clone()],
        provider_hooks: Some(hook.clone()),
        allow_existing_session: true,
        ..Default::default()
    })
    .await
    .unwrap();
    let snapshot = harness.context_snapshot().await.unwrap();
    let estimate =
        ice_harness::compaction::estimate_context_tokens(&snapshot.context.messages).tokens;
    // Raw context is 80% of the window: below the 85% trigger, but above
    // that trigger once its bounded 10% request-transformation reserve is held.
    model.context_window = ((estimate * 5 + 3) / 4) as u64;
    harness.set_model(model).await.unwrap();
    harness
        .set_coding_compaction_policy(Some(small_recent_policy()))
        .await
        .unwrap();
    harness
        .prompt_text("NEW_HOOK_INTENT", vec![])
        .await
        .unwrap();

    let requests = provider.requests();
    assert_eq!(
        requests.len(),
        2,
        "hook overhead must reserve space before sending"
    );
    assert!(is_summary_request(&requests[0]));
    assert!(requests[0].context.contains("OLD_PREFIX_SENTINEL"));
    assert!(!requests[0].context.contains("NEW_HOOK_INTENT"));
    assert!(requests[1].context.contains("NEW_HOOK_INTENT"));
    assert!(requests
        .iter()
        .all(|request| request.context.contains("HOOK_BUDGET_SENTINEL")));
    assert_eq!(
        hook.calls.load(Ordering::SeqCst),
        requests.len(),
        "budgeting must not replay hooks"
    );
}

fn assert_complete_tool_pair(context: &Context) {
    let calls: Vec<_> = context
        .messages
        .iter()
        .flat_map(|message| match message {
            Message::Assistant(assistant) => assistant
                .content
                .iter()
                .filter_map(|block| match block {
                    ice_ai::types::Content::ToolCall(call) => Some(call.id.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        })
        .collect();
    let results: Vec<_> = context
        .messages
        .iter()
        .filter_map(|message| match message {
            Message::ToolResult(result) => {
                assert!(!result.is_error, "the faux tool must execute successfully");
                Some(result.tool_call_id.clone())
            }
            _ => None,
        })
        .collect();
    assert_eq!(calls.len(), 1, "one retained call, including its identity");
    assert_eq!(
        calls, results,
        "every retained call must have its matching result"
    );
    assert!(context
        .system_prompt
        .as_deref()
        .unwrap_or_default()
        .contains("HOOK_BUDGET_SENTINEL"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tool_call_result_pair_survives_compaction_with_request_hooks() {
    let session = new_session();
    append_user(&session, padded_text("OLD_PREFIX_SENTINEL", 6000)).await;
    append_assistant(&session, "old answer ".repeat(200)).await;
    let script = FauxScript::new();
    script.set_responses(vec![
        FauxStep::tool_call(
            "write",
            serde_json::json!({"path":"evidence.txt","content":"TOOL_PAIR_SENTINEL"}),
        ),
        FauxStep::factory(|context| {
            assert_complete_tool_pair(context);
            faux_assistant_message("Tool completed.", StopReason::Stop)
        }),
        FauxStep::text("Older work summarized."),
        FauxStep::factory(|context| {
            assert_complete_tool_pair(context);
            faux_assistant_message("Continued with the retained pair.", StopReason::Stop)
        }),
    ]);
    let faux = FauxProvider::new(script);
    let provider = RecordingProvider::new(faux.clone());
    let execution = Arc::new(InMemoryExecutionEnv::new());
    let env: Arc<dyn ice_tools::ExecutionEnv> = execution.clone();
    let mut_env: Arc<dyn ice_tools::MutatingEnv> = execution;
    let tool_context = ExecutionToolContext::new(env, Some(mut_env));
    let hook = Arc::new(AddingRequestHook::default());
    let mut model = faux.default_model().clone();
    model.context_window = 100_000;
    let harness = AgentHarness::create(AgentHarnessOptions {
        model,
        session,
        models: vec![provider.clone()],
        tools: vec![HarnessTool::new(ice_tools::create_write_tool(
            &tool_context,
        ))],
        active_tool_names: vec!["write".into()],
        provider_hooks: Some(hook.clone()),
        allow_existing_session: true,
        ..Default::default()
    })
    .await
    .unwrap();
    harness
        .set_coding_compaction_policy(Some(CodingAgentCompactionPolicy {
            keep_recent_tokens: 200,
            ..Default::default()
        }))
        .await
        .unwrap();
    harness
        .prompt_text("Write the evidence file.", vec![])
        .await
        .unwrap();
    let compacted = AgentLane::compact(&harness, None).await.unwrap();
    assert!(matches!(
        compacted.outcome,
        CompactionOutcome::Completed { .. }
    ));
    harness
        .prompt_text("Continue after compaction.", vec![])
        .await
        .unwrap();
    assert_eq!(provider.requests().len(), 4);
    assert_eq!(hook.calls.load(Ordering::SeqCst), 4);
    assert!(branch_entries(&harness)
        .await
        .iter()
        .any(|entry| matches!(entry, Entry::Compaction(_))));
}

struct CancelAfterCheckpointStorage {
    inner: Arc<dyn SessionStorage>,
    armed: AtomicBool,
    committed: tokio::sync::Notify,
    release: tokio::sync::Notify,
}

#[async_trait]
impl SessionStorage for CancelAfterCheckpointStorage {
    fn metadata(&self) -> SessionMetadata {
        self.inner.metadata()
    }
    async fn get_lanes(&self) -> ice_harness::error::SessionResult<Vec<LanePointer>> {
        self.inner.get_lanes().await
    }
    async fn create_lane(
        &self,
        lane: &str,
        at: Option<&str>,
    ) -> ice_harness::error::SessionResult<()> {
        self.inner.create_lane(lane, at).await
    }
    async fn move_lane(
        &self,
        lane: &str,
        to: Option<&str>,
    ) -> ice_harness::error::SessionResult<()> {
        self.inner.move_lane(lane, to).await
    }
    async fn append_entry(
        &self,
        entry: ProvisionedEntry,
        lane: &str,
    ) -> ice_harness::error::SessionResult<Entry> {
        let is_checkpoint = matches!(
            entry.kind,
            ice_harness::session::types::ProvisionedKind::Compaction { .. }
        );
        let committed = self.inner.append_entry(entry, lane).await?;
        if is_checkpoint && self.armed.swap(false, Ordering::SeqCst) {
            self.committed.notify_one();
            self.release.notified().await;
        }
        Ok(committed)
    }
    async fn append_record(
        &self,
        record: LaneRecord,
    ) -> ice_harness::error::SessionResult<LaneRecord> {
        self.inner.append_record(record).await
    }
    async fn get_entry(&self, id: &str) -> ice_harness::error::SessionResult<Option<Entry>> {
        self.inner.get_entry(id).await
    }
    async fn find_entries(
        &self,
        query: &EntryQuery,
    ) -> ice_harness::error::SessionResult<Vec<Entry>> {
        self.inner.find_entries(query).await
    }
    async fn find_entries_on_branch(
        &self,
        query: &EntryQuery,
        bounds: &BranchBounds,
        start: &str,
    ) -> ice_harness::error::SessionResult<Vec<Entry>> {
        self.inner
            .find_entries_on_branch(query, bounds, start)
            .await
    }
    async fn find_records(
        &self,
        query: &RecordQuery,
    ) -> ice_harness::error::SessionResult<Vec<LaneRecord>> {
        self.inner.find_records(query).await
    }
    async fn find_open_operations(
        &self,
        lane: &str,
        limit: Option<usize>,
    ) -> ice_harness::error::SessionResult<Vec<OperationStartedRecord>> {
        self.inner.find_open_operations(lane, limit).await
    }
    async fn get_log(
        &self,
        options: &LogOptions,
    ) -> ice_harness::error::SessionResult<Vec<LogItem>> {
        self.inner.get_log(options).await
    }
    async fn get_name(&self) -> ice_harness::error::SessionResult<Option<String>> {
        self.inner.get_name().await
    }
    async fn set_name(&self, name: Option<&str>) -> ice_harness::error::SessionResult<()> {
        self.inner.set_name(name).await
    }
    async fn get_label(&self, id: &str) -> ice_harness::error::SessionResult<Option<String>> {
        self.inner.get_label(id).await
    }
    async fn set_label(
        &self,
        id: &str,
        label: Option<&str>,
    ) -> ice_harness::error::SessionResult<()> {
        self.inner.set_label(id, label).await
    }
    async fn get_stats(&self) -> ice_harness::error::SessionResult<SessionStats> {
        self.inner.get_stats().await
    }
}

async fn branch_entries(harness: &AgentHarness) -> Vec<Entry> {
    let view = harness.session().view("main");
    let leaf = view
        .get_leaf_id()
        .await
        .expect("read branch leaf")
        .expect("branch has a leaf");
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
    .expect("read branch entries")
}

fn is_summary_request(request: &RecordedRequest) -> bool {
    let text = request.context.to_ascii_lowercase();
    text.contains("summariz") || text.contains("checkpoint summary")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn coding_policy_does_not_trigger_at_exactly_85_percent() {
    // 750 + 50 + 25 + 25 = 850 tokens against a 1,000-token window.
    let (harness, provider) = harness_with_history(
        3000,
        200,
        100,
        100,
        1000,
        800,
        CompactionSettings {
            enabled: false,
            ..CompactionSettings::default()
        },
    )
    .await;
    harness
        .set_coding_compaction_policy(Some(small_recent_policy()))
        .await
        .expect("set policy");

    harness
        .prompt_text("NEW_PROMPT_SENTINEL", Vec::new())
        .await
        .expect("prompt completes");

    let entries = branch_entries(&harness).await;
    assert!(!entries
        .iter()
        .any(|entry| matches!(entry, Entry::Compaction(_))));
    let requests = provider.requests();
    assert_eq!(requests.len(), 1, "only the ordinary agent turn should run");
    assert!(!requests.iter().any(is_summary_request));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn follow_up_with_image_queued_during_compaction_is_delivered_once() {
    use ice_ai::types::{ImageContent, ImageContentType};

    let (harness, provider) = harness_with_history(
        3000,
        200,
        100,
        101,
        1000,
        800,
        CompactionSettings::default(),
    )
    .await;
    harness
        .set_coding_compaction_policy(Some(small_recent_policy()))
        .await
        .unwrap();
    provider.pause_next_response();
    let received_follow_up = Arc::new(AtomicBool::new(false));
    let received_follow_up_from_factory = Arc::clone(&received_follow_up);
    fn is_queued_follow_up(message: &AgentMessage) -> bool {
        let AgentMessage::User(user) = message else {
            return false;
        };
        let UserContent::Blocks(blocks) = &user.content else {
            return false;
        };
        ice_ai::types::Content::text_only(blocks, " ").contains("FOLLOW_UP_DURING_COMPACTION")
            && blocks.iter().any(|block| {
                matches!(block,
                ice_ai::types::Content::Image(image) if image.mime_type == "image/png")
            })
    }
    let follow_up_response = FauxStep::factory(move |context| {
        let found = context.messages.iter().any(|message| {
            matches!(
                message,
                Message::User(user) if is_queued_follow_up(&AgentMessage::User(user.clone()))
            )
        });
        received_follow_up_from_factory.store(found, Ordering::SeqCst);
        faux_assistant_message("follow-up consumed", StopReason::Stop)
    });
    provider.inner.script().set_responses(vec![
        FauxStep::text("Old history checkpoint."),
        FauxStep::text("Initial prompt completed."),
        follow_up_response,
    ]);

    let lane = harness.lane("main");
    let first_turn = {
        let lane = Arc::clone(&lane);
        tokio::spawn(async move {
            lane.prompt_text("INITIAL_DURING_COMPACTION", Vec::new())
                .await
        })
    };
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        provider.response_paused.notified(),
    )
    .await
    .expect("automatic compaction reached the deterministic provider barrier");

    let queued = lane
        .follow_up(AgentMessage::User(UserMessage::new(
            UserContent::Blocks(vec![
                ice_ai::types::Content::text("FOLLOW_UP_DURING_COMPACTION"),
                ice_ai::types::Content::Image(ImageContent {
                    kind: ImageContentType,
                    data: "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4z8AAAAMBAQDJ/pLvAAAAAElFTkSuQmCC".into(),
                    mime_type: "image/png".into(),
                }),
            ]),
            999,
        )))
        .await
        .expect("TUI-style follow-up is accepted while compaction is active");
    assert!(!queued.entry_id.is_empty());
    provider.release_response.notify_one();

    let result = tokio::time::timeout(std::time::Duration::from_secs(5), first_turn)
        .await
        .expect("run with queued follow-up settles")
        .unwrap()
        .unwrap();
    assert!(matches!(
        result.outcome,
        ice_harness::agent_harness::HarnessRunOutcome::Completed { .. }
    ));
    assert!(received_follow_up.load(Ordering::SeqCst));
    let entries = branch_entries(&harness).await;
    let follow_up_entries: Vec<_> = entries
        .iter()
        .filter(|entry| {
            matches!(entry,
                Entry::Message(message) if is_queued_follow_up(&message.message)
            )
        })
        .collect();
    assert_eq!(follow_up_entries.len(), 1, "queued follow-up commits once");
    let Entry::Message(message) = follow_up_entries[0] else {
        unreachable!()
    };
    let AgentMessage::User(user) = &message.message else {
        unreachable!()
    };
    assert!(matches!(&user.content, UserContent::Blocks(blocks)
    if blocks.iter().any(|block| matches!(block,
        ice_ai::types::Content::Image(image) if image.mime_type == "image/png"
    ))));
    assert_eq!(
        provider.requests().len(),
        3,
        "summary, first turn, follow-up"
    );
    assert!(is_summary_request(&provider.requests()[0]));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn coding_policy_summarizes_old_history_before_persisting_new_prompt() {
    // The last assistant is 26 tokens, so the old branch totals 851 tokens.
    let (harness, provider) = harness_with_history(
        3000,
        200,
        100,
        101,
        1000,
        800,
        CompactionSettings {
            enabled: false,
            ..CompactionSettings::default()
        },
    )
    .await;
    harness
        .set_coding_compaction_policy(Some(small_recent_policy()))
        .await
        .expect("set policy");

    harness
        .prompt_text("NEW_PROMPT_SENTINEL", Vec::new())
        .await
        .expect("prompt completes");

    let requests = provider.requests();
    let summary = requests
        .iter()
        .find(|request| is_summary_request(request))
        .expect("threshold crossing summarizes the old branch");
    assert!(summary.context.contains("OLD_PREFIX_SENTINEL"));
    assert!(
        !summary.context.contains("NEW_PROMPT_SENTINEL"),
        "the incoming user prompt must not enter the old-history summary"
    );
    assert_eq!(
        requests.len(),
        2,
        "one summary and one normal provider turn"
    );

    let entries = branch_entries(&harness).await;
    let checkpoint_index = entries
        .iter()
        .position(|entry| matches!(entry, Entry::Compaction(_)))
        .expect("persisted coding compaction checkpoint");
    let incoming_index = entries
        .iter()
        .position(|entry| match entry {
            Entry::Message(message) => match &message.message {
                AgentMessage::User(user) => match &user.content {
                    UserContent::Text(text) => text == "NEW_PROMPT_SENTINEL",
                    UserContent::Blocks(_) => false,
                },
                _ => false,
            },
            _ => false,
        })
        .expect("incoming prompt is persisted");
    assert!(checkpoint_index < incoming_index);

    let records = harness
        .session()
        .find_records(&RecordQuery::default())
        .await
        .expect("read durable operation records");
    assert!(records.iter().any(|record| matches!(
        record,
        LaneRecord::StepAttempt(step)
            if step.step == StepKind::Compaction
                && step.compaction_reason == Some(CompactionReason::Threshold)
    )));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn generic_none_policy_keeps_after_prompt_compaction_order() {
    // 850 existing tokens are exactly at the generic 850-token threshold; the
    // new prompt pushes the branch over it. `None` keeps legacy ordering.
    let (harness, provider) = harness_with_history(
        3000,
        200,
        100,
        100,
        1000,
        800,
        CompactionSettings {
            enabled: true,
            reserve_tokens: 150,
            keep_recent_tokens: 50,
        },
    )
    .await;

    harness
        .prompt_text(&"GENERIC_NEW_PROMPT ".repeat(20), Vec::new())
        .await
        .expect("prompt completes");

    let entries = branch_entries(&harness).await;
    let checkpoint_index = entries
        .iter()
        .position(|entry| matches!(entry, Entry::Compaction(_)))
        .expect("generic threshold compacts after the prompt is appended");
    let prompt_index = entries
        .iter()
        .position(|entry| match entry {
            Entry::Message(message) => match &message.message {
                AgentMessage::User(user) => match &user.content {
                    UserContent::Text(text) => text.starts_with("GENERIC_NEW_PROMPT"),
                    UserContent::Blocks(_) => false,
                },
                _ => false,
            },
            _ => false,
        })
        .expect("incoming generic prompt is persisted");
    assert!(prompt_index < checkpoint_index);
    assert!(provider.requests().iter().any(is_summary_request));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn coding_policy_reserve_is_recomputed_after_model_switch() {
    let (harness, provider) = harness_with_history(
        3000,
        200,
        100,
        101,
        1000,
        800,
        CompactionSettings::default(),
    )
    .await;
    let policy = small_recent_policy();
    harness
        .set_coding_compaction_policy(Some(policy))
        .await
        .expect("set policy");
    assert_eq!(
        harness
            .get_coding_compaction_policy()
            .await
            .expect("get policy"),
        Some(policy)
    );

    harness
        .prompt_text("first model run", Vec::new())
        .await
        .expect("first model run completes");
    let first_summary_max = provider
        .requests()
        .into_iter()
        .find(|request| is_summary_request(request))
        .and_then(|request| request.max_tokens)
        .expect("summary call has an output-token budget");

    // Add enough durable history to exceed the product threshold on the larger
    // context model, then change the model on the same live harness.
    append_user(harness.session(), padded_text("SECOND_MODEL_HISTORY", 6800)).await;
    let mut next_model = AgentLane::get_model(&harness)
        .await
        .expect("read current model");
    next_model.context_window = 2000;
    next_model.max_tokens = 1200;
    AgentLane::set_model(&harness, next_model)
        .await
        .expect("switch selected model");

    provider.inner.script().set_responses(vec![
        FauxStep::factory(|_| {
            faux_assistant_message(
                "## Goal\nContinue the existing task.\n\n## Constraints & Preferences\n- None\n\n## Progress\n### Done\n- [x] Existing history reviewed\n\n### In Progress\n- [ ] Continue the task\n\n### Blocked\n- None\n\n## Key Decisions\n- **Keep the existing context**: preserve the prior work.\n\n## Next Steps\n1. Continue from the retained context.\n\n## Critical Context\n- Preserve the old history marker.",
                StopReason::Stop,
            )
        });
        10
    ]);
    let second_start = provider.requests().len();
    harness
        .prompt_text("second model run", Vec::new())
        .await
        .expect("second model run completes");
    let second_summary_max = provider.requests()[second_start..]
        .iter()
        .find(|request| is_summary_request(request))
        .and_then(|request| request.max_tokens)
        .expect("second summary call has an output-token budget");

    assert!(
        second_summary_max > first_summary_max,
        "implicit reserve must be recalculated from the selected model: first={first_summary_max}, second={second_summary_max}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn manual_compaction_cancellation_after_commit_keeps_checkpoint_and_reports_abort() {
    let (harness, provider) =
        harness_with_history(300, 100, 100, 100, 1000, 200, CompactionSettings::default()).await;
    harness
        .set_coding_compaction_policy(Some(small_recent_policy()))
        .await
        .unwrap();
    let storage = Arc::new(CancelAfterCheckpointStorage {
        inner: harness.session().storage(),
        armed: AtomicBool::new(true),
        committed: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
    });
    harness.session().set_storage(storage.clone());
    provider
        .inner
        .script()
        .set_responses(vec![FauxStep::text("Manual checkpoint committed.")]);
    let run = {
        let harness = harness.clone();
        tokio::spawn(async move { AgentLane::compact(&harness, None).await })
    };
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        storage.committed.notified(),
    )
    .await
    .unwrap();
    assert!(branch_entries(&harness)
        .await
        .iter()
        .any(|entry| matches!(entry, Entry::Compaction(_))));
    harness.lane("main").abort().await.unwrap();
    storage.release.notify_one();
    let result = tokio::time::timeout(std::time::Duration::from_secs(5), run)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        matches!(result.outcome, CompactionOutcome::Aborted { .. }),
        "manual compaction after acknowledged checkpoint must not report success"
    );
    assert!(branch_entries(&harness)
        .await
        .iter()
        .any(|entry| matches!(entry,
        Entry::Compaction(checkpoint) if checkpoint.summary == "Manual checkpoint committed.")));
    let records = harness
        .session()
        .find_records(&RecordQuery::default())
        .await
        .unwrap();
    assert!(
        records.iter().any(|record| matches!(record,
            LaneRecord::OperationFinished(finished)
                if finished.outcome == ice_harness::session::types::OperationOutcome::Aborted
        )),
        "manual operation settlement must agree with the public outcome"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn manual_compaction_cancelled_after_summary_response_settles_as_aborted() {
    let (harness, provider) =
        harness_with_history(300, 100, 100, 100, 1000, 200, CompactionSettings::default()).await;
    harness
        .set_coding_compaction_policy(Some(small_recent_policy()))
        .await
        .unwrap();
    provider
        .inner
        .script()
        .set_responses(vec![FauxStep::text("Summary cancelled before commit.")]);
    provider.cancel_after_next_response();

    let result = AgentLane::compact(&harness, None)
        .await
        .expect("cancellation is a settled manual-compaction outcome");

    assert!(
        matches!(result.outcome, CompactionOutcome::Aborted { .. }),
        "a cancelled summary must not be reported as failed or completed"
    );
    assert!(!branch_entries(&harness)
        .await
        .iter()
        .any(|entry| matches!(entry, Entry::Compaction(_))));
    assert_eq!(provider.requests().len(), 1);
    let records = harness
        .session()
        .find_records(&RecordQuery::default())
        .await
        .unwrap();
    assert!(records.iter().any(|record| matches!(record,
        LaneRecord::OperationFinished(finished)
            if finished.outcome == ice_harness::session::types::OperationOutcome::Aborted
    )));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn manual_compaction_cancelled_before_summary_response_settles_as_aborted() {
    let (harness, provider) =
        harness_with_history(300, 100, 100, 100, 1000, 200, CompactionSettings::default()).await;
    harness
        .set_coding_compaction_policy(Some(small_recent_policy()))
        .await
        .expect("install coding summary policy");
    provider.cancel_before_next_response();
    let request_started = provider.request_started.notified();
    let run = {
        let harness = harness.clone();
        tokio::spawn(async move { AgentLane::compact(&harness, None).await })
    };
    tokio::time::timeout(std::time::Duration::from_secs(5), request_started)
        .await
        .expect("summary request reached the provider");

    harness.lane("main").abort().await.unwrap();
    let result = tokio::time::timeout(std::time::Duration::from_secs(5), run)
        .await
        .expect("cancelled compaction settled")
        .unwrap()
        .expect("cancellation is a settled compaction outcome");

    assert!(matches!(result.outcome, CompactionOutcome::Aborted { .. }));
    assert!(!branch_entries(&harness)
        .await
        .iter()
        .any(|entry| matches!(entry, Entry::Compaction(_))));
    assert_eq!(provider.requests().len(), 1);
    let records = harness
        .session()
        .find_records(&RecordQuery::default())
        .await
        .unwrap();
    assert!(records.iter().any(|record| matches!(record,
        LaneRecord::OperationFinished(finished)
            if finished.outcome == ice_harness::session::types::OperationOutcome::Aborted
    )));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn manual_coding_compaction_uses_the_product_summary_path() {
    let (harness, provider) = harness_with_history(
        3000,
        200,
        100,
        101,
        1000,
        800,
        CompactionSettings::default(),
    )
    .await;
    harness
        .set_coding_compaction_policy(Some(small_recent_policy()))
        .await
        .expect("set policy");

    let result = AgentLane::compact(&harness, None)
        .await
        .expect("manual compaction operation completes");
    assert!(matches!(
        result.outcome,
        CompactionOutcome::Completed { .. }
    ));
    assert!(provider.requests().iter().any(is_summary_request));
    assert_eq!(provider.requests().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn coding_summary_failure_settles_run_without_checkpoint_or_prompt() {
    let (harness, provider) = harness_with_history(
        3000,
        200,
        100,
        101,
        1000,
        800,
        CompactionSettings::default(),
    )
    .await;
    harness
        .set_coding_compaction_policy(Some(small_recent_policy()))
        .await
        .expect("set policy");
    let failure = AssistantMessage::terminal(
        ice_ai::types::Api::Faux,
        "faux",
        "faux",
        StopReason::Length,
        "summary response was truncated",
        1,
    );
    provider
        .inner
        .script()
        .set_responses(vec![FauxStep::Message(failure)]);

    let result = harness
        .prompt_text("NEW_PROMPT_SENTINEL", Vec::new())
        .await
        .expect("summary failure is a settled run outcome");

    assert!(matches!(
        result.outcome,
        ice_harness::agent_harness::HarnessRunOutcome::Failed { ref error, .. }
            if error.code == "compaction_failed"
    ));
    assert_eq!(provider.requests().len(), 1, "the agent must not continue");
    let entries = branch_entries(&harness).await;
    assert!(!entries
        .iter()
        .any(|entry| matches!(entry, Entry::Compaction(_))));
    assert!(entries.iter().any(|entry| match entry {
        Entry::Message(message) => match &message.message {
            AgentMessage::User(user) => match &user.content {
                UserContent::Text(text) => text == "NEW_PROMPT_SENTINEL",
                UserContent::Blocks(_) => false,
            },
            _ => false,
        },
        _ => false,
    }));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn late_cancellation_after_coding_summary_prevents_checkpoint() {
    let (harness, provider) = harness_with_history(
        3000,
        200,
        100,
        101,
        1000,
        800,
        CompactionSettings::default(),
    )
    .await;
    harness
        .set_coding_compaction_policy(Some(small_recent_policy()))
        .await
        .expect("set policy");
    provider.cancel_after_next_response();

    let result = harness
        .prompt_text("NEW_PROMPT_SENTINEL", Vec::new())
        .await
        .expect("late cancellation is a settled aborted run");

    assert!(matches!(
        result.outcome,
        ice_harness::agent_harness::HarnessRunOutcome::Aborted { .. }
    ));
    assert_eq!(provider.requests().len(), 1, "the agent must not continue");
    let entries = branch_entries(&harness).await;
    assert!(!entries
        .iter()
        .any(|entry| matches!(entry, Entry::Compaction(_))));
    assert!(entries.iter().any(|entry| match entry {
        Entry::Message(message) => match &message.message {
            AgentMessage::User(user) => match &user.content {
                UserContent::Text(text) => text == "NEW_PROMPT_SENTINEL",
                UserContent::Blocks(_) => false,
            },
            _ => false,
        },
        _ => false,
    }));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn coding_midrun_summary_failure_stops_the_turn_and_fails_the_run() {
    let (harness, provider) = harness_with_history(
        3000,
        200,
        100,
        100,
        2000,
        1200,
        CompactionSettings::default(),
    )
    .await;
    let execution = Arc::new(InMemoryExecutionEnv::new());
    let env: Arc<dyn ice_tools::ExecutionEnv> = execution.clone();
    let mut_env: Arc<dyn ice_tools::MutatingEnv> = execution;
    let tool_context = ExecutionToolContext::new(env, Some(mut_env));
    let read = ice_tools::create_read_tool(&tool_context, None);
    harness
        .set_tools(vec![HarnessTool::new(read)], Some(vec!["read".to_string()]))
        .await
        .expect("install faux read tool");

    let policy = CodingAgentCompactionPolicy {
        mid_run_compaction: MidRunCompactionMode::Resume,
        keep_recent_tokens: 50,
        ..CodingAgentCompactionPolicy::default()
    };
    harness
        .set_coding_compaction_policy(Some(policy))
        .await
        .expect("set policy");
    let failure = AssistantMessage::terminal(
        ice_ai::types::Api::Faux,
        "faux",
        "faux",
        StopReason::Length,
        "mid-run summary response was truncated",
        1,
    );
    provider.inner.script().set_responses(vec![
        FauxStep::tool_call("read", serde_json::json!({"path": "missing.txt"})),
        FauxStep::Message(failure),
    ]);

    let result = harness
        .prompt_text(&padded_text("MIDRUN_TRIGGER_PROMPT", 4000), Vec::new())
        .await
        .expect("mid-run compaction failure settles the run");

    assert!(matches!(
        result.outcome,
        ice_harness::agent_harness::HarnessRunOutcome::Failed { ref error, .. }
            if error.code == "compaction_failed"
    ));
    assert_eq!(provider.requests().len(), 2, "no post-failure continuation");
    let entries = branch_entries(&harness).await;
    assert!(!entries
        .iter()
        .any(|entry| matches!(entry, Entry::Compaction(_))));
}

fn overflow_step() -> FauxStep {
    let mut message = faux_assistant_message("", StopReason::Error);
    message.error_message = Some("context_length_exceeded: prompt is too long".into());
    FauxStep::Message(message)
}

async fn overflow_harness() -> (AgentHarness, Arc<RecordingProvider>) {
    let (harness, provider) = harness_with_history(
        300,
        100,
        100,
        100,
        1000,
        200,
        CompactionSettings {
            enabled: false,
            ..Default::default()
        },
    )
    .await;
    harness
        .set_coding_compaction_policy(Some(small_recent_policy()))
        .await
        .unwrap();
    (harness, provider)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn coding_overflow_compacts_once_and_retries_with_original_prompt() {
    let (harness, provider) = overflow_harness().await;
    provider.inner.script().set_responses(vec![
        overflow_step(),
        FauxStep::text("Useful old-history checkpoint."),
        FauxStep::text("recovered answer"),
    ]);
    let result = harness
        .prompt_text("OVERFLOW_NEW_PROMPT", vec![])
        .await
        .unwrap();
    assert!(
        matches!(
            result.outcome,
            ice_harness::agent_harness::HarnessRunOutcome::Completed { .. }
        ),
        "{:?}",
        result.outcome
    );
    let requests = provider.requests();
    assert_eq!(requests.len(), 3, "failed turn, summary, one retry");
    assert!(is_summary_request(&requests[1]));
    assert!(!requests[1].context.contains("OVERFLOW_NEW_PROMPT"));
    assert!(requests[2].context.contains("OVERFLOW_NEW_PROMPT"));
    assert!(requests[2]
        .context
        .contains("Useful old-history checkpoint."));
    let entries = branch_entries(&harness).await;
    let checkpoint = entries
        .iter()
        .find_map(|entry| match entry {
            Entry::Compaction(checkpoint) => Some(checkpoint),
            _ => None,
        })
        .unwrap();
    assert!(checkpoint
        .retained_tail
        .iter()
        .any(|message| match message {
            AgentMessage::User(user) =>
                matches!(&user.content, UserContent::Text(text) if text == "OVERFLOW_NEW_PROMPT"),
            _ => false,
        }));
    let reasons = harness
        .session()
        .find_records(&RecordQuery::default())
        .await
        .unwrap();
    assert!(reasons.iter().any(|record| matches!(record, LaneRecord::StepAttempt(step) if step.compaction_reason == Some(CompactionReason::Overflow))));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancellation_after_checkpoint_commit_keeps_checkpoint_and_reports_abort() {
    let (harness, provider) = overflow_harness().await;
    let storage = Arc::new(CancelAfterCheckpointStorage {
        inner: harness.session().storage(),
        armed: AtomicBool::new(true),
        committed: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
    });
    harness.session().set_storage(storage.clone());
    provider.inner.script().set_responses(vec![
        overflow_step(),
        FauxStep::text("Committed checkpoint before cancellation."),
        FauxStep::text("must not run"),
    ]);
    let run = {
        let harness = harness.clone();
        tokio::spawn(async move { harness.prompt_text("POST_COMMIT_CANCEL", vec![]).await })
    };
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        storage.committed.notified(),
    )
    .await
    .unwrap();
    // Storage accepted the Compaction entry; the append has not returned to
    // persist_compaction_entry yet, so operation settlement is still pending.
    assert!(branch_entries(&harness)
        .await
        .iter()
        .any(|entry| matches!(entry, Entry::Compaction(_))));
    harness.lane("main").abort().await.unwrap();
    storage.release.notify_one();
    let result = tokio::time::timeout(std::time::Duration::from_secs(5), run)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        matches!(
            result.outcome,
            ice_harness::agent_harness::HarnessRunOutcome::Aborted { .. }
        ),
        "{:?}",
        result.outcome
    );
    assert!(
        !storage.armed.load(Ordering::SeqCst),
        "checkpoint write boundary was reached"
    );
    let requests = provider.requests();
    assert_eq!(
        requests.len(),
        2,
        "cancellation after commit must not start another provider request"
    );
    let entries = branch_entries(&harness).await;
    let checkpoint = entries
        .iter()
        .find_map(|entry| match entry {
            Entry::Compaction(checkpoint) => Some(checkpoint),
            _ => None,
        })
        .expect("acknowledged checkpoint remains durable after cancellation");
    assert_eq!(
        checkpoint.summary,
        "Committed checkpoint before cancellation."
    );
    let records = harness
        .session()
        .find_records(&RecordQuery::default())
        .await
        .unwrap();
    assert!(
        records.iter().any(|record| matches!(record,
            LaneRecord::OperationFinished(finished)
                if finished.outcome == ice_harness::session::types::OperationOutcome::Aborted
        )),
        "the run settles as aborted rather than completed"
    );
    assert!(
        records.iter().any(|record| matches!(record,
            LaneRecord::StepAttempt(step)
                if step.step == StepKind::Compaction
                    && step.compaction_reason == Some(CompactionReason::Overflow)
        )),
        "the committed checkpoint has its settlement record"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancellation_after_midrun_checkpoint_commit_keeps_checkpoint_and_stops() {
    let (harness, provider) =
        harness_with_history(300, 100, 100, 100, 2000, 400, CompactionSettings::default()).await;
    harness
        .set_coding_compaction_policy(Some(CodingAgentCompactionPolicy {
            mid_run_compaction: MidRunCompactionMode::Resume,
            ..small_recent_policy()
        }))
        .await
        .unwrap();
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
    let storage = Arc::new(CancelAfterCheckpointStorage {
        inner: harness.session().storage(),
        armed: AtomicBool::new(true),
        committed: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
    });
    harness.session().set_storage(storage.clone());
    provider.inner.script().set_responses(vec![
        FauxStep::tool_call("read", serde_json::json!({"path":"missing.txt"})),
        FauxStep::text("Committed mid-run checkpoint."),
        FauxStep::text("must not run"),
    ]);
    let run = {
        let harness = harness.clone();
        tokio::spawn(async move {
            harness
                .prompt_text(&padded_text("MIDRUN_POST_COMMIT_CANCEL", 8000), vec![])
                .await
        })
    };
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        storage.committed.notified(),
    )
    .await
    .unwrap();
    assert!(branch_entries(&harness)
        .await
        .iter()
        .any(|entry| matches!(entry, Entry::Compaction(_))));
    harness.lane("main").abort().await.unwrap();
    storage.release.notify_one();
    let result = tokio::time::timeout(std::time::Duration::from_secs(5), run)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        matches!(
            result.outcome,
            ice_harness::agent_harness::HarnessRunOutcome::Aborted { .. }
        ),
        "{:?}",
        result.outcome
    );
    assert_eq!(
        provider.requests().len(),
        2,
        "post-commit cancellation must not continue to another provider request"
    );
    assert!(branch_entries(&harness)
        .await
        .iter()
        .any(|entry| matches!(entry,
        Entry::Compaction(checkpoint) if checkpoint.summary == "Committed mid-run checkpoint.")));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn repeated_coding_overflow_stops_after_one_recovery() {
    let (harness, provider) = overflow_harness().await;
    provider.inner.script().set_responses(vec![
        overflow_step(),
        FauxStep::text("Useful checkpoint."),
        overflow_step(),
        FauxStep::text("must not run"),
    ]);
    let result = harness
        .prompt_text("retain my input", vec![])
        .await
        .unwrap();
    assert!(
        matches!(result.outcome, ice_harness::agent_harness::HarnessRunOutcome::Failed { ref error, .. } if error.message.contains("Shorten") || error.message.contains("shorten")),
        "{:?}",
        result.outcome
    );
    assert_eq!(provider.requests().len(), 3);
    assert_eq!(
        branch_entries(&harness)
            .await
            .iter()
            .filter(|entry| matches!(entry, Entry::Compaction(_)))
            .count(),
        1
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn coding_overflow_failed_summary_preserves_prompt_without_checkpoint() {
    let (harness, provider) = overflow_harness().await;
    provider.inner.script().set_responses(vec![
        overflow_step(),
        FauxStep::Message(faux_assistant_message("", StopReason::Length)),
        FauxStep::text("must not run"),
    ]);
    let result = harness
        .prompt_text("RETAIN_FAILED_OVERFLOW_PROMPT", vec![])
        .await
        .unwrap();
    assert!(matches!(
        result.outcome,
        ice_harness::agent_harness::HarnessRunOutcome::Failed { .. }
    ));
    assert_eq!(provider.requests().len(), 2);
    let entries = branch_entries(&harness).await;
    assert!(!entries
        .iter()
        .any(|entry| matches!(entry, Entry::Compaction(_))));
    assert!(entries.iter().any(|entry| match entry {
        Entry::Message(message) => matches!(&message.message, AgentMessage::User(user) if matches!(&user.content, UserContent::Text(text) if text == "RETAIN_FAILED_OVERFLOW_PROMPT")),
        _ => false,
    }));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn oversized_incoming_prompt_is_not_summarized_away_or_retried() {
    let (harness, provider) = overflow_harness().await;
    provider
        .inner
        .script()
        .set_responses(vec![overflow_step(), FauxStep::text("must not run")]);
    let prompt = padded_text("OVERSIZED_INPUT", 5000);
    let result = harness.prompt_text(&prompt, vec![]).await.unwrap();
    assert!(
        matches!(result.outcome, ice_harness::agent_harness::HarnessRunOutcome::Failed { ref error, .. } if error.message.to_ascii_lowercase().contains("shorten"))
    );
    assert_eq!(provider.requests().len(), 1);
    assert!(!branch_entries(&harness)
        .await
        .iter()
        .any(|entry| matches!(entry, Entry::Compaction(_))));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn coding_overflow_late_summary_cancellation_does_not_publish_checkpoint() {
    let (harness, provider) = overflow_harness().await;
    let cancel_provider = provider.clone();
    provider.inner.script().set_responses(vec![
        FauxStep::factory(move |_| {
            // The wrapper captures its flag at request entry, so arm from
            // request one to cancel after request two's completed summary.
            cancel_provider.cancel_after_next_response();
            let mut failed = faux_assistant_message("", StopReason::Error);
            failed.error_message = Some("context_length_exceeded".into());
            failed
        }),
        FauxStep::text("Valid but cancelled summary"),
        FauxStep::text("must not run"),
    ]);
    let result = harness
        .prompt_text("CANCELLED_OVERFLOW_INPUT", vec![])
        .await
        .unwrap();
    assert!(matches!(
        result.outcome,
        ice_harness::agent_harness::HarnessRunOutcome::Aborted { .. }
    ));
    assert_eq!(provider.requests().len(), 2);
    let entries = branch_entries(&harness).await;
    assert!(!entries
        .iter()
        .any(|entry| matches!(entry, Entry::Compaction(_))));
    assert!(entries.iter().any(|entry| match entry {
        Entry::Message(message) => matches!(&message.message, AgentMessage::User(user) if matches!(&user.content, UserContent::Text(text) if text == "CANCELLED_OVERFLOW_INPUT")),
        _ => false,
    }));
}

async fn midrun_checkpoint(
    mode: MidRunCompactionMode,
) -> (
    ice_harness::agent_harness::RunResult,
    Arc<RecordingProvider>,
    AgentHarness,
) {
    let (harness, provider) =
        harness_with_history(300, 100, 100, 100, 2000, 400, CompactionSettings::default()).await;
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
    harness
        .set_coding_compaction_policy(Some(CodingAgentCompactionPolicy {
            mid_run_compaction: mode,
            keep_recent_tokens: 50,
            ..Default::default()
        }))
        .await
        .unwrap();
    provider.inner.script().set_responses(vec![
        FauxStep::tool_call("read", serde_json::json!({"path":"missing.txt"})),
        FauxStep::text("Summary of the old history."),
        FauxStep::text("actual final answer"),
    ]);
    let result = harness
        .prompt_text(&padded_text("MIDRUN_CHECKPOINT", 8000), vec![])
        .await
        .unwrap();
    (result, provider, harness)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn default_midrun_safety_net_continues_after_checkpoint() {
    let (result, provider, _) = midrun_checkpoint(MidRunCompactionMode::Off).await;
    assert!(
        matches!(
            result.outcome,
            ice_harness::agent_harness::HarnessRunOutcome::Completed { .. }
        ),
        "{:?}",
        result.outcome
    );
    assert_eq!(provider.requests().len(), 3, "{:?}", result.outcome);
    let ice_harness::agent_harness::HarnessRunOutcome::Completed { final_message, .. } =
        result.outcome
    else {
        unreachable!()
    };
    assert!(final_message.content.iter().any(|block| matches!(block, ice_ai::types::Content::Text(text) if text.text == "actual final answer")));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn explicit_midrun_pause_is_not_reported_as_task_completion() {
    let (result, provider, harness) = midrun_checkpoint(MidRunCompactionMode::Pause).await;
    assert!(
        matches!(result.outcome, ice_harness::agent_harness::HarnessRunOutcome::Failed { ref error, .. } if error.code == "compaction_paused"),
        "{:?}",
        result.outcome
    );
    assert_eq!(provider.requests().len(), 2, "{:?}", result.outcome);
    assert!(branch_entries(&harness)
        .await
        .iter()
        .any(|entry| matches!(entry, Entry::Compaction(_))));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn coding_budget_reserves_selected_model_output_and_recomputes_on_switch() {
    let (harness, _) = harness_with_history(
        300,
        100,
        100,
        100,
        10000,
        4000,
        CompactionSettings::default(),
    )
    .await;
    harness
        .set_coding_compaction_policy(Some(small_recent_policy()))
        .await
        .unwrap();
    assert_eq!(
        harness.context_snapshot().await.unwrap().compact_at_tokens,
        Some(6000)
    );

    let mut model = AgentLane::get_model(&harness).await.unwrap();
    model.max_tokens = 1000;
    AgentLane::set_model(&harness, model.clone()).await.unwrap();
    assert_eq!(
        harness.context_snapshot().await.unwrap().compact_at_tokens,
        Some(8500)
    );

    model.context_window = 8000;
    model.max_tokens = 4000;
    AgentLane::set_model(&harness, model.clone()).await.unwrap();
    // The reserve is the configured request fitted to the window alone
    // (8,000 - 4,096 = 3,904), never to current usage.
    assert_eq!(
        harness.context_snapshot().await.unwrap().compact_at_tokens,
        Some(4096)
    );

    // A catalog whose output maximum equals its window still leaves an
    // input budget.
    model.max_tokens = 8000;
    AgentLane::set_model(&harness, model.clone()).await.unwrap();
    assert_eq!(
        harness.context_snapshot().await.unwrap().compact_at_tokens,
        Some(4096)
    );

    model.context_window = 0;
    AgentLane::set_model(&harness, model).await.unwrap();
    assert_eq!(
        harness.context_snapshot().await.unwrap().compact_at_tokens,
        Some(0)
    );
}

/// Regression: when the output maximum equals the window, fitting the reserve
/// to current usage made the ceiling track the input, so with a request hook
/// every turn boundary compacted (and compacted again after each compaction).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn coding_ceiling_does_not_track_usage_when_output_max_equals_window() {
    let small = {
        let (harness, _) = harness_with_history(
            300,
            100,
            100,
            100,
            272_000,
            272_000,
            CompactionSettings::default(),
        )
        .await;
        harness
            .set_coding_compaction_policy(Some(small_recent_policy()))
            .await
            .unwrap();
        harness.context_snapshot().await.unwrap()
    };
    let large = {
        let (harness, _) = harness_with_history(
            120_000,
            100,
            100,
            100,
            272_000,
            272_000,
            CompactionSettings::default(),
        )
        .await;
        harness
            .set_coding_compaction_policy(Some(small_recent_policy()))
            .await
            .unwrap();
        harness.context_snapshot().await.unwrap()
    };
    assert_eq!(small.compact_at_tokens, Some(136_000));
    assert_eq!(large.compact_at_tokens, small.compact_at_tokens);
    let used = ice_harness::compaction::estimate_context_tokens(&large.context.messages).tokens;
    assert!(
        used < 136_000,
        "history of {used} tokens must sit below the ceiling"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn coding_budget_includes_system_prompt_and_active_tool_schemas() {
    let (harness, provider) =
        harness_with_history(300, 100, 100, 100, 2000, 200, CompactionSettings::default()).await;
    harness
        .set_coding_compaction_policy(Some(small_recent_policy()))
        .await
        .unwrap();
    harness
        .set_system_prompt(Some("SYSTEM_OVERHEAD ".repeat(700)))
        .await
        .unwrap();
    let execution = Arc::new(InMemoryExecutionEnv::new());
    let env: Arc<dyn ice_tools::ExecutionEnv> = execution.clone();
    let mut_env: Arc<dyn ice_tools::MutatingEnv> = execution;
    let tools = ExecutionToolContext::new(env, Some(mut_env));
    harness
        .set_tools(
            vec![HarnessTool::new(ice_tools::create_read_tool(&tools, None))],
            Some(vec!["read".into()]),
        )
        .await
        .unwrap();
    provider.inner.script().set_responses(vec![
        FauxStep::text("older history summary"),
        FauxStep::text("split history summary"),
        FauxStep::text("answer"),
    ]);
    harness.prompt_text("KEEP_NEW_INPUT", vec![]).await.unwrap();
    let requests = provider.requests();
    assert!(
        is_summary_request(&requests[0]),
        "known request overhead must trigger before the new prompt"
    );
    assert!(!requests[0].context.contains("KEEP_NEW_INPUT"));
    assert!(branch_entries(&harness)
        .await
        .iter()
        .any(|entry| matches!(entry, Entry::Compaction(_))));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn overflow_budget_refuses_irreducible_system_overhead_before_summary() {
    let (harness, provider) = overflow_harness().await;
    harness
        .set_coding_compaction_policy(Some(CodingAgentCompactionPolicy {
            threshold_percent: 99.0,
            ..small_recent_policy()
        }))
        .await
        .unwrap();
    // Below the proactive threshold but impossible together with the latest
    // user turn. Recovery must not bill a summary that cannot solve this.
    harness
        .set_system_prompt(Some("s".repeat(2500)))
        .await
        .unwrap();
    provider
        .inner
        .script()
        .set_responses(vec![overflow_step(), FauxStep::text("unhelpful summary")]);
    let result = harness
        .prompt_text(&"new input ".repeat(160), vec![])
        .await
        .unwrap();
    assert!(matches!(
        result.outcome,
        ice_harness::agent_harness::HarnessRunOutcome::Failed { .. }
    ));
    assert!(
        !provider.requests().iter().any(is_summary_request),
        "irreducible request overhead cannot be compacted away"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn disabled_coding_compaction_does_not_recover_overflow_implicitly() {
    let (harness, provider) = overflow_harness().await;
    harness
        .set_coding_compaction_policy(Some(CodingAgentCompactionPolicy {
            enabled: false,
            ..small_recent_policy()
        }))
        .await
        .unwrap();
    provider
        .inner
        .script()
        .set_responses(vec![overflow_step(), FauxStep::text("must not run")]);
    let result = harness.prompt_text("keep the input", vec![]).await.unwrap();
    assert!(matches!(
        result.outcome,
        ice_harness::agent_harness::HarnessRunOutcome::Failed { .. }
    ));
    assert_eq!(provider.requests().len(), 1);
    assert!(!branch_entries(&harness)
        .await
        .iter()
        .any(|entry| matches!(entry, Entry::Compaction(_))));
}
