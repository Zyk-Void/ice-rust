//! Mirrors `packages/agent/src/harness/compaction/compaction.ts` — the core
//! compaction pipeline: `prepare_compaction` (cut-point selection + virtual
//! retained-tail re-injection + file-op extraction), `compact` (split-turn TWO
//! LLM calls, invariant §10), `complete_simple_with_retries` (standalone
//! summary request, `CacheRetention::None` + fresh `session_id`), the
//! summarization prompts, `combine_usage`, and the `CompactionError` type.
//!
//! Implementation notes:
//! - The generic harness summarizes through its configured `Provider` because
//!   this Rust API has no `Models.completeSimple` runtime abstraction. The
//!   callback-aware `complete_simple_with_retry_options` is additive; existing
//!   generic helpers remain permissive about non-error stop reasons.
//! - Assistant-level classifier/backoff/callback behavior is shared with
//!   `ice-ai::retry`, distinct from provider HTTP retries.
//! - `complete_validated_coding_summary` supplies the stricter coding-agent
//!   validation boundary. `SimpleStreamOptions` currently has no `tool_choice`
//!   or per-call `env` field, so this adapter can only clear context tools and
//!   forward the options the Rust provider API actually supports.
//! - **`build_session_context`** is a minimal port (in `tokens.rs`) pulled
//!   forward from M5f; M5f's Session facade will own/expand it.

use std::sync::Arc;

use futures::future::BoxFuture;
use ice_agent::message::AgentMessage;
pub use ice_ai::is_retryable_assistant_error;
use ice_ai::types::{
    AssistantMessage, Content, Context, Message, StopReason, Usage, UserContent, UserMessage,
};
use ice_ai::{
    retry_assistant_call_with_options, AssistantRetryCallbacks, AssistantRetryOptions,
    AssistantRetryPolicy, CacheRetention, Model, Provider, RetryCallbackFailure,
    SimpleStreamOptions, ThinkingLevel,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::compaction::cut_point::find_cut_point;
use crate::compaction::tokens::{
    build_session_context, compute_file_lists, create_file_ops, estimate_context_tokens,
    extract_file_ops_from_message, format_file_operations, serialize_conversation, FileOperations,
};
use crate::messages::convert_to_llm;
use crate::messages::{create_branch_summary_message, create_compaction_summary_message};
use crate::session::types::{Entry, EntryBase, MessageEntry};
use crate::types::{CompactionSettings, RetryPolicy};

// ---------------------------------------------------------------------------
// Prompts — verbatim from compaction.ts
// ---------------------------------------------------------------------------

pub const SUMMARIZATION_SYSTEM_PROMPT: &str = "You are a context summarization assistant. Your task is to read a conversation between a user and an AI assistant, then produce a structured summary following the exact format specified.\n\nDo NOT continue the conversation. Do NOT respond to any questions in the conversation. ONLY output the structured summary.";

const SUMMARIZATION_PROMPT: &str = "The messages above are a conversation to summarize. Create a structured context checkpoint summary that another LLM will use to continue the work.\n\nUse this EXACT format:\n\n## Goal\n[What is the user trying to accomplish? Can be multiple items if the session covers different tasks.]\n\n## Constraints & Preferences\n- [Any constraints, preferences, or requirements mentioned by user]\n- [Or \"(none)\" if none were mentioned]\n\n## Progress\n### Done\n- [x] [Completed tasks/changes]\n\n### In Progress\n- [ ] [Current work]\n\n### Blocked\n- [Issues preventing progress, if any]\n\n## Key Decisions\n- **[Decision]**: [Brief rationale]\n\n## Next Steps\n1. [Ordered list of what should happen next]\n\n## Critical Context\n- [Any data, examples, or references needed to continue]\n- [Or \"(none)\" if not applicable]\n\nKeep each section concise. Preserve exact file paths, function names, and error messages.";

const UPDATE_SUMMARIZATION_PROMPT: &str = "The messages above are NEW conversation messages to incorporate into the existing summary provided in <previous-summary> tags.\n\nUpdate the existing structured summary with new information. RULES:\n- PRESERVE all existing information from the previous summary\n- ADD new progress, decisions, and context from the new messages\n- UPDATE the Progress section: move items from \"In Progress\" to \"Done\" when completed\n- UPDATE \"Next Steps\" based on what was accomplished\n- PRESERVE exact file paths, function names, and error messages\n- If something is no longer relevant, you may remove it\n\nUse this EXACT format:\n\n## Goal\n[Preserve existing goals, add new ones if the task expanded]\n\n## Constraints & Preferences\n- [Preserve existing, add new ones discovered]\n\n## Progress\n### Done\n- [x] [Include previously done items AND newly completed items]\n\n### In Progress\n- [ ] [Current work - update based on progress]\n\n### Blocked\n- [Current blockers - remove if resolved]\n\n## Key Decisions\n- **[Decision]**: [Brief rationale] (preserve all previous, add new)\n\n## Next Steps\n1. [Update based on current state]\n\n## Critical Context\n- [Preserve important context, add new if needed]\n\nKeep each section concise. Preserve exact file paths, function names, and error messages.";

const TURN_PREFIX_SUMMARIZATION_PROMPT: &str = "This is the PREFIX of a turn that was too large to keep. The SUFFIX (recent work) is retained.\n\nSummarize the prefix to provide context for the retained suffix:\n\n## Original Request\n[What did the user ask for in this turn?]\n\n## Early Progress\n- [Key decisions and work done in the prefix]\n\n## Context for Suffix\n- [Information needed to understand the retained recent work]\n\nBe concise. Focus on what's needed to understand the kept suffix.";

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// `CompactionError` code. Mirrors TS `"aborted" | "summarization_failed"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactionErrorCode {
    Aborted,
    SummarizationFailed,
}

impl CompactionErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            CompactionErrorCode::Aborted => "aborted",
            CompactionErrorCode::SummarizationFailed => "summarization_failed",
        }
    }
}

/// Mirrors TS `CompactionError`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactionError {
    pub code: CompactionErrorCode,
    pub message: String,
}

impl CompactionError {
    pub fn aborted(message: impl Into<String>) -> Self {
        Self {
            code: CompactionErrorCode::Aborted,
            message: message.into(),
        }
    }
    pub fn summarization_failed(message: impl Into<String>) -> Self {
        Self {
            code: CompactionErrorCode::SummarizationFailed,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for CompactionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for CompactionError {}

// ---------------------------------------------------------------------------
// Results / preparation
// ---------------------------------------------------------------------------

/// File-operation details stored on generated compaction entries. Mirrors TS
/// `CompactionDetails`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactionDetails {
    pub read_files: Vec<String>,
    pub modified_files: Vec<String>,
}

/// Generated compaction data ready to be persisted as a compaction entry.
/// Mirrors TS `CompactResult<T>`.
#[derive(Debug, Clone, PartialEq)]
pub struct CompactResult {
    pub summary: String,
    pub tokens_before: i64,
    pub usage: Option<Usage>,
    pub retained_tail: Vec<AgentMessage>,
    pub details: Option<CompactionDetails>,
}

/// Prepared inputs for a compaction run. Mirrors TS `CompactionPreparation`.
#[derive(Debug, Clone, PartialEq)]
pub struct CompactionPreparation {
    pub messages_to_summarize: Vec<AgentMessage>,
    pub turn_prefix_messages: Vec<AgentMessage>,
    pub retained_tail: Vec<AgentMessage>,
    pub is_split_turn: bool,
    pub tokens_before: i64,
    pub previous_summary: Option<String>,
    pub file_ops: FileOperations,
    pub settings: CompactionSettings,
}

/// Provider/model/etc. needed for the standalone summarization LLM call(s).
/// Consolidates the `models, model, signal, customInstructions, thinkingLevel,
/// retry` TS args into one struct so `compact`/`generate_*` stay readable.
#[derive(Clone)]
pub struct CompactionLlmOptions {
    pub provider: Arc<dyn Provider>,
    pub model: Model,
    pub api_key: Option<String>,
    pub signal: CancellationToken,
    pub thinking_level: Option<ThinkingLevel>,
    pub retry: Option<RetryPolicy>,
    pub custom_instructions: Option<String>,
}

// ---------------------------------------------------------------------------
// Entry → message helpers — mirrors getMessageFromEntry{,ForCompaction}
// ---------------------------------------------------------------------------

/// `getMessageFromEntry`. `message` → its message; `branch_summary` → a
/// branch-summary message; `compaction` → a compaction-summary message; else
/// `None`.
pub fn get_message_from_entry(entry: &Entry) -> Option<AgentMessage> {
    match entry {
        Entry::Message(m) => Some(m.message.clone()),
        Entry::BranchSummary(b) => Some(create_branch_summary_message(
            &b.summary,
            &b.from_id,
            b.base.timestamp,
        )),
        Entry::Compaction(c) => Some(create_compaction_summary_message(
            &c.summary,
            c.tokens_before,
            c.base.timestamp,
        )),
        _ => None,
    }
}

/// `getMessageFromEntryForCompaction` — compaction entries are not summarized
/// (they ARE the summary), so skip them.
pub fn get_message_from_entry_for_compaction(entry: &Entry) -> Option<AgentMessage> {
    if matches!(entry, Entry::Compaction(_)) {
        return None;
    }
    get_message_from_entry(entry)
}

/// The timestamp carried by an `AgentMessage` (each variant stores one).
fn agent_message_timestamp(message: &AgentMessage) -> i64 {
    match message {
        AgentMessage::User(u) => u.timestamp,
        AgentMessage::Assistant(a) => a.timestamp,
        AgentMessage::ToolResult(t) => t.timestamp,
        AgentMessage::Custom(c) => c.timestamp,
    }
}

// ---------------------------------------------------------------------------
// extractFileOperations — seeds from prev compaction details + summarized msgs
// ---------------------------------------------------------------------------

/// `extractFileOperations`. Seeds `read`/`edited` from a previous compaction's
/// `details` (so file lists accumulate across compactions), then extracts from
/// each summarized assistant message.
pub fn extract_file_operations(
    messages: &[AgentMessage],
    entries: &[Entry],
    prev_compaction_index: Option<usize>,
) -> FileOperations {
    let mut ops = create_file_ops();
    if let Some(i) = prev_compaction_index {
        if let Some(Entry::Compaction(c)) = entries.get(i) {
            if let Some(details) = &c.details {
                if let Some(arr) = details.get("readFiles").and_then(|v| v.as_array()) {
                    for f in arr {
                        if let Some(s) = f.as_str() {
                            ops.read.insert(s.to_string());
                        }
                    }
                }
                if let Some(arr) = details.get("modifiedFiles").and_then(|v| v.as_array()) {
                    for f in arr {
                        if let Some(s) = f.as_str() {
                            ops.edited.insert(s.to_string());
                        }
                    }
                }
            }
        }
    }
    for msg in messages {
        extract_file_ops_from_message(msg, &mut ops);
    }
    ops
}

// ---------------------------------------------------------------------------
// prepareCompaction
// ---------------------------------------------------------------------------

/// Prepare session entries for compaction, or `None` when compaction is not
/// applicable (empty path, or last entry is already a compaction). Mirrors
/// `prepareCompaction`.
///
/// The returned `Result` is always `Ok` (TS never errors here), but carries the
/// `CompactionError` type for parity with the TS signature.
pub fn prepare_compaction(
    path_entries: &[Entry],
    settings: CompactionSettings,
) -> Result<Option<CompactionPreparation>, CompactionError> {
    if path_entries.is_empty() {
        return Ok(None);
    }
    if matches!(path_entries.last(), Some(Entry::Compaction(_))) {
        return Ok(None);
    }

    let prev_compaction_index = path_entries
        .iter()
        .enumerate()
        .rev()
        .find(|(_, e)| matches!(e, Entry::Compaction(_)))
        .map(|(i, _)| i);

    let (previous_summary, compactable_entries): (Option<String>, Vec<Entry>) =
        match prev_compaction_index {
            Some(i) => {
                let prev = match &path_entries[i] {
                    Entry::Compaction(c) => c.clone(),
                    _ => unreachable!("prev_compaction_index points at a compaction"),
                };
                let prev_summary = prev.summary.clone();
                let prev_id = prev.base.id.clone();
                let prev_seq = prev.base.seq;
                let mut virtual_entries: Vec<Entry> = Vec::with_capacity(prev.retained_tail.len());
                for (idx, message) in prev.retained_tail.iter().enumerate() {
                    let id = format!("{prev_id}:retained:{idx}");
                    let parent_id = if idx == 0 {
                        Some(prev_id.clone())
                    } else {
                        Some(format!("{prev_id}:retained:{}", idx - 1))
                    };
                    let timestamp = agent_message_timestamp(message);
                    virtual_entries.push(Entry::Message(MessageEntry {
                        base: EntryBase {
                            entry_type: "message".into(),
                            id,
                            seq: prev_seq,
                            parent_id,
                            timestamp,
                        },
                        message: message.clone(),
                        terminate: None,
                    }));
                }
                // `…virtualRetainedEntries, ...pathEntries.slice(prevCompactionIndex + 1)`.
                let rest: Vec<Entry> = path_entries[i + 1..].to_vec();
                virtual_entries.extend(rest);
                (Some(prev_summary), virtual_entries)
            }
            None => (None, path_entries.to_vec()),
        };

    let boundary_end = compactable_entries.len();

    // `tokensBefore` uses the ORIGINAL path entries (not compactable), matching
    // TS `buildSessionContext(pathEntries).messages`.
    let tokens_before =
        estimate_context_tokens(&build_session_context(path_entries).messages).tokens;

    let cut_point = find_cut_point(
        &compactable_entries,
        0,
        boundary_end,
        settings.keep_recent_tokens,
    );
    let history_end = if cut_point.is_split_turn {
        cut_point
            .turn_start_index
            .expect("is_split_turn ⇒ turn_start_index is Some")
    } else {
        cut_point.first_kept_entry_index
    };

    let mut messages_to_summarize: Vec<AgentMessage> = Vec::new();
    for entry in compactable_entries.iter().take(history_end) {
        if let Some(m) = get_message_from_entry_for_compaction(entry) {
            messages_to_summarize.push(m);
        }
    }

    let mut turn_prefix_messages: Vec<AgentMessage> = Vec::new();
    if cut_point.is_split_turn {
        let turn_start = cut_point
            .turn_start_index
            .expect("is_split_turn ⇒ turn_start_index is Some");
        for entry in compactable_entries
            .iter()
            .take(cut_point.first_kept_entry_index)
            .skip(turn_start)
        {
            if let Some(m) = get_message_from_entry_for_compaction(entry) {
                turn_prefix_messages.push(m);
            }
        }
    }

    let mut retained_tail: Vec<AgentMessage> = Vec::new();
    for entry in compactable_entries
        .iter()
        .skip(cut_point.first_kept_entry_index)
    {
        if let Some(m) = get_message_from_entry_for_compaction(entry) {
            retained_tail.push(m);
        }
    }

    let mut file_ops =
        extract_file_operations(&messages_to_summarize, path_entries, prev_compaction_index);
    if cut_point.is_split_turn {
        for msg in &turn_prefix_messages {
            extract_file_ops_from_message(msg, &mut file_ops);
        }
    }

    Ok(Some(CompactionPreparation {
        messages_to_summarize,
        turn_prefix_messages,
        retained_tail,
        is_split_turn: cut_point.is_split_turn,
        tokens_before,
        previous_summary,
        file_ops,
        settings,
    }))
}

// ---------------------------------------------------------------------------
// completeSimpleWithRetries + shared retry integration
// ---------------------------------------------------------------------------

/// `completeSimpleWithRetries`. Summaries are standalone requests: force
/// `CacheRetention::None` and a fresh `session_id`, then retry the complete
/// assistant operation using the shared `ice-ai` retry policy.
pub async fn complete_simple_with_retries(
    provider: Arc<dyn Provider>,
    model: Model,
    context: Context,
    options: SimpleStreamOptions,
    retry: Option<RetryPolicy>,
) -> AssistantMessage {
    let retry_policy = retry.map(|policy| AssistantRetryPolicy {
        enabled: policy.enabled,
        max_retries: policy.max_retries,
        base_delay_ms: policy.base_delay_ms,
        max_delay_ms: Some(policy.max_agent_delay_ms),
    });
    let retry_options = AssistantRetryOptions::new(retry_policy, options.signal.clone());
    complete_simple_with_retry_options(provider, model, context, options, retry_options)
        .await
        .expect("the legacy summary helper installs no fallible retry callbacks")
}

/// Callback-aware additive counterpart to [`complete_simple_with_retries`].
/// Callback failures are returned in lifecycle order instead of being hidden in
/// an assistant error message.
pub async fn complete_simple_with_retry_options(
    provider: Arc<dyn Provider>,
    model: Model,
    context: Context,
    mut options: SimpleStreamOptions,
    mut retry: AssistantRetryOptions,
) -> Result<AssistantMessage, RetryCallbackFailure> {
    options.cache_retention = CacheRetention::None;
    options.session_id = Some(Uuid::now_v7().to_string());
    retry.signal = options.signal.clone();
    retry_assistant_call_with_options(
        || async {
            let stream = provider.stream_simple(&model, &context, &options).await;
            match stream.result().await {
                Ok(message) => message,
                Err(_) => AssistantMessage::terminal(
                    model.api.clone(),
                    provider.id().to_string(),
                    model.id.clone(),
                    StopReason::Error,
                    "Summarization stream ended without a terminal event",
                    0,
                ),
            }
        },
        retry,
    )
    .await
}

/// Injected coding-agent stream seam. The callback receives immutable request
/// values; it must return an owned future so it cannot mutate or borrow them.
pub type SummarizationStreamFn = Arc<
    dyn Fn(&Model, &Context, &SimpleStreamOptions) -> BoxFuture<'static, AssistantMessage>
        + Send
        + Sync,
>;

/// Optional coding-summary request injection and retry lifecycle callbacks.
///
/// The generic compaction APIs do not use these options; callers opt into the
/// stricter coding-agent completion contract through [`compact_coding`].
#[derive(Clone, Default)]
pub struct CodingSummaryCompletionOptions {
    /// Inject a stream response for testing or caller-managed completion.
    pub stream_fn: Option<SummarizationStreamFn>,
    /// Awaited callbacks for whole-assistant retry lifecycle events.
    pub callbacks: AssistantRetryCallbacks,
}

/// Coding-agent completion contract layered beside the intentionally
/// permissive generic compaction helpers. It strips context tools, disables
/// cache retention and assigns one UUID per request (reused for every retry),
/// then retries provider failures before applying terminal validation.
///
/// This Rust version can pass an explicit Model/Provider and the available
/// `SimpleStreamOptions`; it has no `tool_choice` or per-request `env` field.
pub async fn complete_validated_coding_summary(
    provider: Arc<dyn Provider>,
    model: &Model,
    context: &Context,
    options: &SimpleStreamOptions,
    stream_fn: Option<SummarizationStreamFn>,
    mut retry: AssistantRetryOptions,
) -> Result<AssistantMessage, RetryCallbackFailure> {
    let model = model.clone();
    let mut request_context = context.clone();
    request_context.tools.clear();
    let mut request_options = options.clone();
    request_options.cache_retention = CacheRetention::None;
    request_options.session_id = Some(Uuid::now_v7().to_string());
    retry.signal = request_options.signal.clone();

    let response = retry_assistant_call_with_options(
        || async {
            if let Some(stream_fn) = &stream_fn {
                stream_fn(&model, &request_context, &request_options).await
            } else {
                let stream = provider
                    .stream_simple(&model, &request_context, &request_options)
                    .await;
                match stream.result().await {
                    Ok(message) => message,
                    Err(_) => AssistantMessage::terminal(
                        model.api.clone(),
                        provider.id().to_string(),
                        model.id.clone(),
                        StopReason::Error,
                        "Summarization stream ended without a terminal event",
                        0,
                    ),
                }
            }
        },
        retry,
    )
    .await?;

    Ok(validate_coding_summary_response(
        response,
        request_options.signal.is_cancelled(),
    ))
}

fn validate_coding_summary_response(
    mut response: AssistantMessage,
    signal_cancelled: bool,
) -> AssistantMessage {
    if signal_cancelled || response.stop_reason == StopReason::Aborted {
        response.stop_reason = StopReason::Aborted;
        response.error_message = Some("Summarization aborted".to_string());
        return response;
    }
    if response.stop_reason == StopReason::Error {
        return response;
    }
    if response.stop_reason != StopReason::Stop {
        let stop_reason = response.stop_reason.as_str();
        response.stop_reason = StopReason::Error;
        response.error_message = Some(format!(
            "Incomplete summarization (stop reason: {stop_reason})"
        ));
        return response;
    }
    if response
        .content
        .iter()
        .any(|block| matches!(block, Content::ToolCall(_)))
    {
        response.stop_reason = StopReason::Error;
        response.error_message = Some("Summarization returned tool calls".to_string());
        return response;
    }
    let text: String = response
        .content
        .iter()
        .filter_map(|block| match block {
            Content::Text(text) => Some(text.text.as_str()),
            _ => None,
        })
        .collect();
    if text.trim().is_empty() {
        response.stop_reason = StopReason::Error;
        response.error_message = Some("Summarization returned no text".to_string());
    }
    response
}

/// `combineUsage(first, second)`. `Usage::add` already sums every field
/// (including the optional `cache_write_1h`/`reasoning` with None-as-0
/// semantics + costs); this is the functional wrapper the TS code uses.
pub fn combine_usage(first: &Usage, second: &Usage) -> Usage {
    let mut u = first.clone();
    u.add(second);
    u
}

// ---------------------------------------------------------------------------
// generateSummaryWithUsage / generateTurnPrefixSummary
// ---------------------------------------------------------------------------

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// `floor(0.8 * reserveTokens)`, capped by `model.max_tokens` when > 0.
fn summary_max_tokens(reserve_tokens: i64, model_max_tokens: u64) -> u64 {
    let reserve = reserve_tokens.max(0) as u64;
    let budget = ((reserve as f64) * 0.8).floor() as u64;
    if model_max_tokens > 0 {
        budget.min(model_max_tokens)
    } else {
        budget
    }
}

/// `floor(0.5 * reserveTokens)`, capped by `model.max_tokens` when > 0.
fn turn_prefix_max_tokens(reserve_tokens: i64, model_max_tokens: u64) -> u64 {
    let reserve = reserve_tokens.max(0) as u64;
    let budget = ((reserve as f64) * 0.5).floor() as u64;
    if model_max_tokens > 0 {
        budget.min(model_max_tokens)
    } else {
        budget
    }
}

/// Build the per-call `SimpleStreamOptions` from the LLM options + a resolved
/// `max_tokens`. Sets `api_key`/`signal`/`max_tokens`/`reasoning` (reasoning
/// only when the model reasons AND a non-`Off` level is set — mirrors TS).
fn build_summary_options(opts: &CompactionLlmOptions, max_tokens: u64) -> SimpleStreamOptions {
    let mut o = SimpleStreamOptions::default();
    o.api_key = opts.api_key.clone();
    o.signal = opts.signal.clone();
    o.max_tokens = Some(max_tokens);
    if opts.model.reasoning {
        if let Some(tl) = opts.thinking_level {
            if tl != ThinkingLevel::Off {
                o.reasoning = Some(tl);
            }
        }
    }
    o
}

async fn complete_summary_request(
    opts: &CompactionLlmOptions,
    context: &Context,
    stream_options: &SimpleStreamOptions,
    coding_options: Option<&CodingSummaryCompletionOptions>,
) -> Result<AssistantMessage, CompactionError> {
    let Some(coding_options) = coding_options else {
        return Ok(complete_simple_with_retries(
            opts.provider.clone(),
            opts.model.clone(),
            context.clone(),
            stream_options.clone(),
            opts.retry.clone(),
        )
        .await);
    };

    let retry_policy = opts.retry.as_ref().map(|policy| AssistantRetryPolicy {
        enabled: policy.enabled,
        max_retries: policy.max_retries,
        base_delay_ms: policy.base_delay_ms,
        max_delay_ms: Some(policy.max_agent_delay_ms),
    });
    let retry_options = AssistantRetryOptions::new(retry_policy, stream_options.signal.clone())
        .with_callbacks(coding_options.callbacks.clone());
    complete_validated_coding_summary(
        opts.provider.clone(),
        &opts.model,
        context,
        stream_options,
        coding_options.stream_fn.clone(),
        retry_options,
    )
    .await
    .map_err(|error| {
        CompactionError::summarization_failed(format!(
            "Summarization retry callback failed: {error}"
        ))
    })
}

/// `generateSummaryWithUsage`. Builds the conversation+prompt user message,
/// calls `complete_simple_with_retries`, maps `aborted`/`error` stop reasons to
/// `CompactionError`. Returns `(text, usage)`.
pub async fn generate_summary_with_usage(
    current_messages: &[AgentMessage],
    reserve_tokens: i64,
    previous_summary: Option<&str>,
    opts: &CompactionLlmOptions,
) -> Result<(String, Usage), CompactionError> {
    generate_summary_with_usage_impl(
        current_messages,
        reserve_tokens,
        previous_summary,
        opts,
        None,
    )
    .await
}

async fn generate_summary_with_usage_impl(
    current_messages: &[AgentMessage],
    reserve_tokens: i64,
    previous_summary: Option<&str>,
    opts: &CompactionLlmOptions,
    coding_options: Option<&CodingSummaryCompletionOptions>,
) -> Result<(String, Usage), CompactionError> {
    let max_tokens = summary_max_tokens(reserve_tokens, opts.model.max_tokens);

    let previous_summary = previous_summary.filter(|summary| !summary.is_empty());
    let custom_instructions = opts
        .custom_instructions
        .as_deref()
        .filter(|instructions| !instructions.is_empty());
    let mut base_prompt = if previous_summary.is_some() {
        UPDATE_SUMMARIZATION_PROMPT
    } else {
        SUMMARIZATION_PROMPT
    }
    .to_string();
    if let Some(instructions) = custom_instructions {
        base_prompt += &format!("\n\nAdditional focus: {instructions}");
    }

    let llm_messages = convert_to_llm(current_messages.to_vec());
    let conversation_text = serialize_conversation(&llm_messages);
    let mut prompt_text = format!("<conversation>\n{conversation_text}\n</conversation>\n\n");
    if let Some(prev) = previous_summary {
        prompt_text += &format!("<previous-summary>\n{prev}\n</previous-summary>\n\n");
    }
    prompt_text += &base_prompt;

    let summarization_messages = vec![Message::User(UserMessage::new(
        UserContent::Blocks(vec![Content::text(prompt_text)]),
        now_ms(),
    ))];
    let context = Context {
        system_prompt: Some(SUMMARIZATION_SYSTEM_PROMPT.to_string()),
        messages: summarization_messages,
        tools: Vec::new(),
    };
    let options = build_summary_options(opts, max_tokens);

    let response = complete_summary_request(opts, &context, &options, coding_options).await?;

    if response.stop_reason == StopReason::Aborted {
        return Err(CompactionError::aborted(
            response
                .error_message
                .as_deref()
                .filter(|message| !message.is_empty())
                .unwrap_or("Summarization aborted"),
        ));
    }
    if response.stop_reason == StopReason::Error {
        return Err(CompactionError::summarization_failed(format!(
            "Summarization failed: {}",
            response
                .error_message
                .as_deref()
                .filter(|message| !message.is_empty())
                .unwrap_or("Unknown error")
        )));
    }

    let text = if coding_options.is_some() {
        response
            .content
            .iter()
            .filter_map(|block| match block {
                Content::Text(text) => Some(text.text.as_str()),
                _ => None,
            })
            .collect()
    } else {
        Content::text_only(&response.content, "\n")
    };
    Ok((text, response.usage.clone()))
}

/// `generateTurnPrefixSummary`. Same shape as the history summary but with the
/// turn-prefix prompt, `0.5×reserve` max-tokens, no previous-summary, no custom
/// instructions.
pub async fn generate_turn_prefix_summary(
    messages: &[AgentMessage],
    reserve_tokens: i64,
    opts: &CompactionLlmOptions,
) -> Result<(String, Usage), CompactionError> {
    generate_turn_prefix_summary_impl(messages, reserve_tokens, opts, None).await
}

async fn generate_turn_prefix_summary_impl(
    messages: &[AgentMessage],
    reserve_tokens: i64,
    opts: &CompactionLlmOptions,
    coding_options: Option<&CodingSummaryCompletionOptions>,
) -> Result<(String, Usage), CompactionError> {
    let max_tokens = turn_prefix_max_tokens(reserve_tokens, opts.model.max_tokens);

    let llm_messages = convert_to_llm(messages.to_vec());
    let conversation_text = serialize_conversation(&llm_messages);
    let prompt_text = format!(
        "<conversation>\n{conversation_text}\n</conversation>\n\n{TURN_PREFIX_SUMMARIZATION_PROMPT}"
    );

    let summarization_messages = vec![Message::User(UserMessage::new(
        UserContent::Blocks(vec![Content::text(prompt_text)]),
        now_ms(),
    ))];
    let context = Context {
        system_prompt: Some(SUMMARIZATION_SYSTEM_PROMPT.to_string()),
        messages: summarization_messages,
        tools: Vec::new(),
    };
    let options = build_summary_options(opts, max_tokens);

    let response = complete_summary_request(opts, &context, &options, coding_options).await?;

    if response.stop_reason == StopReason::Aborted {
        return Err(CompactionError::aborted(
            response
                .error_message
                .as_deref()
                .filter(|message| !message.is_empty())
                .unwrap_or("Turn prefix summarization aborted"),
        ));
    }
    if response.stop_reason == StopReason::Error {
        return Err(CompactionError::summarization_failed(format!(
            "Turn prefix summarization failed: {}",
            response
                .error_message
                .as_deref()
                .filter(|message| !message.is_empty())
                .unwrap_or("Unknown error")
        )));
    }

    let text = if coding_options.is_some() {
        response
            .content
            .iter()
            .filter_map(|block| match block {
                Content::Text(text) => Some(text.text.as_str()),
                _ => None,
            })
            .collect()
    } else {
        Content::text_only(&response.content, "\n")
    };
    Ok((text, response.usage.clone()))
}

// ---------------------------------------------------------------------------
// compact — split-turn TWO LLM calls (invariant §10)
// ---------------------------------------------------------------------------

/// `compact`. Generates the compaction summary from a preparation. Split-turn
/// with a non-empty turn prefix → TWO LLM calls (history @ 0.8×reserve, turn
/// prefix @ 0.5×reserve) concatenated as `"{history}\n\n---\n\n**Turn Context
/// (split turn):**\n\n{turnPrefix}"`; otherwise a single history call. Appends
/// `format_file_operations`. Returns `CompactResult`.
pub async fn compact(
    preparation: &CompactionPreparation,
    opts: &CompactionLlmOptions,
) -> Result<CompactResult, CompactionError> {
    compact_impl(preparation, opts, None).await
}

/// Coding-agent compaction with strict summary validation and retry callbacks.
/// Unlike [`compact`], this rejects a preparation with no history or turn-prefix
/// messages before making a completion request.
pub async fn compact_coding(
    preparation: &CompactionPreparation,
    opts: &CompactionLlmOptions,
    completion_options: &CodingSummaryCompletionOptions,
) -> Result<CompactResult, CompactionError> {
    if preparation.messages_to_summarize.is_empty() && preparation.turn_prefix_messages.is_empty() {
        return Err(CompactionError::summarization_failed(
            "No messages available to summarize",
        ));
    }
    compact_impl(preparation, opts, Some(completion_options)).await
}

async fn compact_impl(
    preparation: &CompactionPreparation,
    opts: &CompactionLlmOptions,
    coding_options: Option<&CodingSummaryCompletionOptions>,
) -> Result<CompactResult, CompactionError> {
    let (summary, usage) =
        if preparation.is_split_turn && !preparation.turn_prefix_messages.is_empty() {
            let (history_text, history_usage) = if preparation.messages_to_summarize.is_empty() {
                ("No prior history.".to_string(), None)
            } else {
                let (t, u) = generate_summary_with_usage_impl(
                    &preparation.messages_to_summarize,
                    preparation.settings.reserve_tokens,
                    preparation.previous_summary.as_deref(),
                    opts,
                    coding_options,
                )
                .await?;
                (t, Some(u))
            };
            let (turn_text, turn_usage) = generate_turn_prefix_summary_impl(
                &preparation.turn_prefix_messages,
                preparation.settings.reserve_tokens,
                opts,
                coding_options,
            )
            .await?;
            let combined =
                format!("{history_text}\n\n---\n\n**Turn Context (split turn):**\n\n{turn_text}");
            let combined_usage = match history_usage {
                Some(hu) => Some(combine_usage(&hu, &turn_usage)),
                None => Some(turn_usage),
            };
            (combined, combined_usage)
        } else {
            let (t, u) = generate_summary_with_usage_impl(
                &preparation.messages_to_summarize,
                preparation.settings.reserve_tokens,
                preparation.previous_summary.as_deref(),
                opts,
                coding_options,
            )
            .await?;
            (t, Some(u))
        };

    let (read_files, modified_files) = compute_file_lists(&preparation.file_ops);
    let file_ops_text = format_file_operations(&read_files, &modified_files);
    let summary = format!("{summary}{file_ops_text}");

    Ok(CompactResult {
        summary,
        tokens_before: preparation.tokens_before,
        usage,
        retained_tail: preparation.retained_tail.clone(),
        details: Some(CompactionDetails {
            read_files,
            modified_files,
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::types::EntryBase;

    fn user_msg(text: &str, seq: u64) -> Entry {
        Entry::Message(MessageEntry {
            base: EntryBase {
                entry_type: "message".into(),
                id: format!("e{seq}"),
                seq,
                parent_id: None,
                timestamp: seq as i64,
            },
            message: AgentMessage::User(UserMessage::new(
                UserContent::Text(text.into()),
                seq as i64,
            )),
            terminate: None,
        })
    }

    fn agent_user_msg(text: &str) -> AgentMessage {
        AgentMessage::User(UserMessage::new(text, 1))
    }

    fn test_preparation(
        messages_to_summarize: Vec<AgentMessage>,
        turn_prefix_messages: Vec<AgentMessage>,
        is_split_turn: bool,
    ) -> CompactionPreparation {
        CompactionPreparation {
            messages_to_summarize,
            turn_prefix_messages,
            retained_tail: Vec::new(),
            is_split_turn,
            tokens_before: 77,
            previous_summary: None,
            file_ops: create_file_ops(),
            settings: CompactionSettings {
                enabled: true,
                reserve_tokens: 1000,
                keep_recent_tokens: 0,
            },
        }
    }

    #[tokio::test]
    async fn empty_summary_instructions_and_previous_summary_use_falsy_semantics() {
        use ice_ai::providers::faux::{FauxProvider, FauxScript, FauxStep};
        use std::sync::Mutex;

        let observed_prompt = Arc::new(Mutex::new(None::<String>));
        let prompt = Arc::clone(&observed_prompt);
        let script = FauxScript::new();
        script.set_responses(vec![FauxStep::factory(move |context| {
            let text = match context.messages.first() {
                Some(Message::User(user)) => match &user.content {
                    UserContent::Text(text) => text.clone(),
                    UserContent::Blocks(blocks) => Content::text_only(blocks, ""),
                },
                _ => String::new(),
            };
            *prompt.lock().unwrap() = Some(text);
            ice_ai::providers::faux::faux_assistant_message("summary", StopReason::Stop)
        })]);
        let provider = FauxProvider::new(script);
        let model = provider.default_model().clone();
        let options = CompactionLlmOptions {
            provider,
            model,
            api_key: None,
            signal: CancellationToken::new(),
            thinking_level: None,
            retry: None,
            custom_instructions: Some(String::new()),
        };
        let messages = [AgentMessage::User(UserMessage::new("history", 1))];
        generate_summary_with_usage(&messages, 2048, Some(""), &options)
            .await
            .unwrap();
        let prompt = observed_prompt.lock().unwrap().clone().unwrap();
        assert!(prompt.contains(SUMMARIZATION_PROMPT));
        assert!(!prompt.contains(UPDATE_SUMMARIZATION_PROMPT));
        assert!(!prompt.contains("<previous-summary>"));
        assert!(!prompt.contains("Additional focus:"));
    }

    #[tokio::test]
    async fn empty_error_message_uses_generic_summary_fallback() {
        use ice_ai::providers::faux::{FauxProvider, FauxScript, FauxStep};

        let script = FauxScript::new();
        script.set_responses(vec![FauxStep::Message(AssistantMessage::terminal(
            ice_ai::types::Api::Faux,
            "faux",
            "faux-1",
            StopReason::Error,
            "",
            1,
        ))]);
        let provider = FauxProvider::new(script);
        let options = CompactionLlmOptions {
            provider: provider.clone(),
            model: provider.default_model().clone(),
            api_key: None,
            signal: CancellationToken::new(),
            thinking_level: None,
            retry: None,
            custom_instructions: None,
        };
        let messages = [AgentMessage::User(UserMessage::new("history", 1))];
        let error = generate_summary_with_usage(&messages, 2048, None, &options)
            .await
            .unwrap_err();
        assert_eq!(error.message, "Summarization failed: Unknown error");
    }

    #[test]
    fn prepare_compaction_empty_is_none() {
        let r = prepare_compaction(&[], CompactionSettings::default()).unwrap();
        assert!(r.is_none());
    }

    #[test]
    fn prepare_compaction_last_is_compaction_is_none() {
        let entries = vec![user_msg("hi", 1)];
        let compaction = Entry::Compaction(crate::session::types::CompactionEntry {
            base: EntryBase {
                entry_type: "compaction".into(),
                id: "c1".into(),
                seq: 2,
                parent_id: Some("e1".into()),
                timestamp: 2,
            },
            summary: "s".into(),
            retained_tail: Vec::new(),
            tokens_before: 10,
            details: None,
            usage: None,
        });
        let r = prepare_compaction(
            &[entries[0].clone(), compaction],
            CompactionSettings::default(),
        )
        .unwrap();
        assert!(r.is_none());
    }

    #[test]
    fn prepare_compaction_single_user_returns_some_with_empty_summarize() {
        // One user message, small keep_recent — no history to summarize.
        let entries = vec![user_msg("hi", 1)];
        let settings = CompactionSettings {
            enabled: true,
            reserve_tokens: 1000,
            keep_recent_tokens: 10,
        };
        let p = prepare_compaction(&entries, settings).unwrap().unwrap();
        assert!(p.messages_to_summarize.is_empty());
        assert!(p
            .retained_tail
            .iter()
            .any(|m| matches!(m, AgentMessage::User(u) if u.content.as_text()==Some("hi"))));
        assert!(!p.is_split_turn);
    }

    #[test]
    fn combine_usage_sums_fields() {
        let a = Usage {
            input: 10,
            output: 5,
            cache_read: 1,
            cache_write: 2,
            cache_write_1h: Some(3),
            reasoning: None,
            total_tokens: 18,
            cost: ice_ai::types::UsageCost {
                input: 1.0,
                output: 2.0,
                cache_read: 0.5,
                cache_write: 0.25,
                total: 3.75,
            },
        };
        let b = Usage {
            input: 1,
            output: 1,
            cache_read: 0,
            cache_write: 0,
            cache_write_1h: None,
            reasoning: Some(4),
            total_tokens: 2,
            cost: ice_ai::types::UsageCost {
                input: 0.1,
                output: 0.2,
                cache_read: 0.0,
                cache_write: 0.0,
                total: 0.3,
            },
        };
        let c = combine_usage(&a, &b);
        assert_eq!(c.input, 11);
        assert_eq!(c.output, 6);
        assert_eq!(c.cache_write_1h, Some(3));
        assert_eq!(c.reasoning, Some(4));
        assert_eq!(c.total_tokens, 20);
        assert!((c.cost.total - 4.05).abs() < 1e-9);
    }

    #[tokio::test]
    async fn coding_compact_validates_response_and_preserves_text_and_usage() {
        use ice_ai::providers::faux::{FauxProvider, FauxScript};
        use std::sync::Mutex;

        let provider = FauxProvider::new(FauxScript::new());
        let model = provider.default_model().clone();
        let observations = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&observations);
        let stream_fn: SummarizationStreamFn = Arc::new(move |model, context, options| {
            let prompt = match context.messages.first() {
                Some(Message::User(user)) => match &user.content {
                    UserContent::Text(text) => text.clone(),
                    UserContent::Blocks(blocks) => Content::text_only(blocks, ""),
                },
                _ => String::new(),
            };
            recorded.lock().unwrap().push((
                model.id.clone(),
                context.system_prompt.clone().unwrap_or_default(),
                context.tools.len(),
                prompt,
                options.api_key.clone(),
                options.max_tokens,
                options.cache_retention,
                options.session_id.clone(),
            ));
            let mut response =
                AssistantMessage::empty(model.api.clone(), "faux", model.id.clone(), 1);
            response.stop_reason = StopReason::Stop;
            response.content = vec![Content::text("  First "), Content::text("Second\n ")];
            response.usage.input = 19;
            response.usage.output = 3;
            response.usage.total_tokens = 22;
            Box::pin(async move { response }) as BoxFuture<'static, AssistantMessage>
        });
        let llm_options = CompactionLlmOptions {
            provider,
            model: model.clone(),
            api_key: Some("test-api-key".into()),
            signal: CancellationToken::new(),
            thinking_level: None,
            retry: None,
            custom_instructions: Some("focus decisions".into()),
        };
        let mut preparation =
            test_preparation(vec![agent_user_msg("user request")], Vec::new(), false);
        preparation.previous_summary = Some("prior checkpoint".into());
        let completion_options = CodingSummaryCompletionOptions {
            stream_fn: Some(stream_fn),
            ..CodingSummaryCompletionOptions::default()
        };

        let result = compact_coding(&preparation, &llm_options, &completion_options)
            .await
            .unwrap();

        assert_eq!(result.summary, "  First Second\n ");
        assert_eq!(result.usage.as_ref().unwrap().input, 19);
        assert_eq!(result.usage.as_ref().unwrap().output, 3);
        assert_eq!(result.tokens_before, 77);
        let observations = observations.lock().unwrap();
        assert_eq!(observations.len(), 1);
        let (
            model_id,
            system_prompt,
            tool_count,
            prompt,
            api_key,
            max_tokens,
            cache_retention,
            session_id,
        ) = &observations[0];
        assert_eq!(model_id, &model.id);
        assert_eq!(system_prompt, SUMMARIZATION_SYSTEM_PROMPT);
        assert_eq!(*tool_count, 0);
        assert!(prompt.contains("user request"));
        assert!(prompt.contains("prior checkpoint"));
        assert!(prompt.contains("Additional focus: focus decisions"));
        assert_eq!(api_key.as_deref(), Some("test-api-key"));
        assert_eq!(
            *max_tokens,
            Some(summary_max_tokens(1000, model.max_tokens))
        );
        assert_eq!(*cache_retention, CacheRetention::None);
        assert!(Uuid::parse_str(session_id.as_deref().unwrap()).is_ok());
    }

    #[tokio::test]
    async fn coding_compact_rejects_invalid_responses_without_retrying_validation_failures() {
        use ice_ai::providers::faux::{FauxProvider, FauxScript};
        use std::sync::atomic::{AtomicUsize, Ordering};

        let mut length = AssistantMessage::empty(ice_ai::types::Api::Faux, "faux", "faux-1", 1);
        length.stop_reason = StopReason::Length;
        length.content.push(Content::text("partial"));
        let mut tool_use = AssistantMessage::empty(ice_ai::types::Api::Faux, "faux", "faux-1", 1);
        tool_use.stop_reason = StopReason::ToolUse;
        let mut tool_call = AssistantMessage::empty(ice_ai::types::Api::Faux, "faux", "faux-1", 1);
        tool_call.stop_reason = StopReason::Stop;
        tool_call
            .content
            .push(Content::tool_call("call", "read", serde_json::json!({})));
        let mut blank = AssistantMessage::empty(ice_ai::types::Api::Faux, "faux", "faux-1", 1);
        blank.stop_reason = StopReason::Stop;
        blank.content.push(Content::text("  \n\t"));
        let invalid_responses = [
            (length, "Incomplete summarization (stop reason: length)"),
            (tool_use, "Incomplete summarization (stop reason: toolUse)"),
            (tool_call, "Summarization returned tool calls"),
            (blank, "Summarization returned no text"),
        ];

        for (response, expected_message) in invalid_responses {
            let provider = FauxProvider::new(FauxScript::new());
            let model = provider.default_model().clone();
            let calls = Arc::new(AtomicUsize::new(0));
            let stream_calls = Arc::clone(&calls);
            let stream_fn: SummarizationStreamFn = Arc::new(move |_model, _context, _options| {
                stream_calls.fetch_add(1, Ordering::SeqCst);
                let response = response.clone();
                Box::pin(async move { response }) as BoxFuture<'static, AssistantMessage>
            });
            let llm_options = CompactionLlmOptions {
                provider,
                model,
                api_key: None,
                signal: CancellationToken::new(),
                thinking_level: None,
                retry: Some(RetryPolicy {
                    enabled: true,
                    max_retries: 3,
                    base_delay_ms: 0,
                    max_agent_delay_ms: 0,
                }),
                custom_instructions: None,
            };
            let completion_options = CodingSummaryCompletionOptions {
                stream_fn: Some(stream_fn),
                ..CodingSummaryCompletionOptions::default()
            };
            let preparation =
                test_preparation(vec![agent_user_msg("summarize me")], Vec::new(), false);

            let error = compact_coding(&preparation, &llm_options, &completion_options)
                .await
                .unwrap_err();
            assert_eq!(error.code, CompactionErrorCode::SummarizationFailed);
            assert!(error.message.contains(expected_message), "{error}");
            assert_eq!(calls.load(Ordering::SeqCst), 1);
        }
    }

    #[tokio::test]
    async fn coding_compact_rejects_empty_summary_spans_before_request() {
        use ice_ai::providers::faux::{FauxProvider, FauxScript};
        use std::sync::atomic::{AtomicUsize, Ordering};

        let provider = FauxProvider::new(FauxScript::new());
        let model = provider.default_model().clone();
        let calls = Arc::new(AtomicUsize::new(0));
        let stream_calls = Arc::clone(&calls);
        let stream_fn: SummarizationStreamFn = Arc::new(move |_model, _context, _options| {
            stream_calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(async {
                ice_ai::providers::faux::faux_assistant_message("unexpected", StopReason::Stop)
            }) as BoxFuture<'static, AssistantMessage>
        });
        let llm_options = CompactionLlmOptions {
            provider,
            model,
            api_key: None,
            signal: CancellationToken::new(),
            thinking_level: None,
            retry: None,
            custom_instructions: None,
        };
        let completion_options = CodingSummaryCompletionOptions {
            stream_fn: Some(stream_fn),
            ..CodingSummaryCompletionOptions::default()
        };
        let preparation = test_preparation(Vec::new(), Vec::new(), false);

        let error = compact_coding(&preparation, &llm_options, &completion_options)
            .await
            .unwrap_err();
        assert!(error.message.contains("No messages available to summarize"));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn coding_compact_assigns_fresh_ids_per_split_request_and_reuses_each_on_retry() {
        use ice_ai::providers::faux::{FauxProvider, FauxScript};
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Mutex;

        let provider = FauxProvider::new(FauxScript::new());
        let model = provider.default_model().clone();
        let ids = Arc::new(Mutex::new(Vec::new()));
        let observed_ids = Arc::clone(&ids);
        let attempts = Arc::new(AtomicUsize::new(0));
        let stream_attempts = Arc::clone(&attempts);
        let stream_fn: SummarizationStreamFn = Arc::new(move |_model, _context, options| {
            observed_ids
                .lock()
                .unwrap()
                .push(options.session_id.clone().unwrap());
            let attempt = stream_attempts.fetch_add(1, Ordering::SeqCst);
            let response = if attempt == 0 {
                AssistantMessage::terminal(
                    ice_ai::types::Api::Faux,
                    "faux",
                    "faux-1",
                    StopReason::Error,
                    "terminated",
                    1,
                )
            } else {
                let mut response =
                    AssistantMessage::empty(ice_ai::types::Api::Faux, "faux", "faux-1", 1);
                response.stop_reason = StopReason::Stop;
                response.content.push(Content::text("summary"));
                response
            };
            Box::pin(async move { response }) as BoxFuture<'static, AssistantMessage>
        });
        let llm_options = CompactionLlmOptions {
            provider,
            model,
            api_key: None,
            signal: CancellationToken::new(),
            thinking_level: None,
            retry: Some(RetryPolicy {
                enabled: true,
                max_retries: 1,
                base_delay_ms: 0,
                max_agent_delay_ms: 0,
            }),
            custom_instructions: None,
        };
        let completion_options = CodingSummaryCompletionOptions {
            stream_fn: Some(stream_fn),
            ..CodingSummaryCompletionOptions::default()
        };
        let preparation = test_preparation(
            vec![agent_user_msg("history")],
            vec![agent_user_msg("large turn prefix")],
            true,
        );

        compact_coding(&preparation, &llm_options, &completion_options)
            .await
            .unwrap();

        let ids = ids.lock().unwrap();
        assert_eq!(ids.len(), 3);
        assert_eq!(ids[0], ids[1]);
        assert_ne!(ids[1], ids[2]);
        assert!(ids.iter().all(|id| Uuid::parse_str(id).is_ok()));
    }

    #[tokio::test]
    async fn coding_compact_surfaces_retry_callback_failures_as_compaction_errors() {
        use ice_ai::providers::faux::{FauxProvider, FauxScript};

        let provider = FauxProvider::new(FauxScript::new());
        let model = provider.default_model().clone();
        let stream_fn: SummarizationStreamFn = Arc::new(|_model, _context, _options| {
            let response = AssistantMessage::terminal(
                ice_ai::types::Api::Faux,
                "faux",
                "faux-1",
                StopReason::Error,
                "terminated",
                1,
            );
            Box::pin(async move { response }) as BoxFuture<'static, AssistantMessage>
        });
        let llm_options = CompactionLlmOptions {
            provider,
            model,
            api_key: None,
            signal: CancellationToken::new(),
            thinking_level: None,
            retry: Some(RetryPolicy {
                enabled: true,
                max_retries: 1,
                base_delay_ms: 0,
                max_agent_delay_ms: 0,
            }),
            custom_instructions: None,
        };
        let callbacks = AssistantRetryCallbacks::default()
            .on_retry_scheduled(|_| async { Err("observer failed".to_string()) });
        let completion_options = CodingSummaryCompletionOptions {
            stream_fn: Some(stream_fn),
            callbacks,
        };
        let preparation = test_preparation(vec![agent_user_msg("summarize me")], Vec::new(), false);

        let error = compact_coding(&preparation, &llm_options, &completion_options)
            .await
            .unwrap_err();
        assert_eq!(error.code, CompactionErrorCode::SummarizationFailed);
        assert!(error
            .message
            .contains("on_retry_scheduled failed: observer failed"));
    }

    #[tokio::test]
    async fn generic_compact_remains_permissive_for_empty_spans_and_length_responses() {
        use ice_ai::providers::faux::{FauxProvider, FauxScript, FauxStep};

        let script = FauxScript::new();
        let mut response = AssistantMessage::empty(ice_ai::types::Api::Faux, "faux", "faux-1", 1);
        response.stop_reason = StopReason::Length;
        response.content.push(Content::text("partial summary"));
        response.usage.input = 17;
        script.set_responses(vec![FauxStep::Message(response)]);
        let provider = FauxProvider::new(script);
        let llm_options = CompactionLlmOptions {
            provider: provider.clone(),
            model: provider.default_model().clone(),
            api_key: None,
            signal: CancellationToken::new(),
            thinking_level: None,
            retry: Some(RetryPolicy {
                enabled: true,
                max_retries: 3,
                base_delay_ms: 0,
                max_agent_delay_ms: 0,
            }),
            custom_instructions: None,
        };
        let preparation = test_preparation(Vec::new(), Vec::new(), false);

        let result = compact(&preparation, &llm_options).await.unwrap();
        assert_eq!(result.summary, "partial summary");
        assert!(result.usage.unwrap().input > 0);
    }

    #[test]
    fn coding_summary_validation_preserves_ordered_failure_precedence_and_payload() {
        let mut length = AssistantMessage::empty(ice_ai::types::Api::Faux, "faux", "faux-1", 1);
        length.stop_reason = StopReason::Length;
        length.content.push(Content::text("partial"));
        let length = validate_coding_summary_response(length, false);
        assert_eq!(length.stop_reason, StopReason::Error);
        assert_eq!(
            length.error_message.as_deref(),
            Some("Incomplete summarization (stop reason: length)")
        );

        let mut tool_use = AssistantMessage::empty(ice_ai::types::Api::Faux, "faux", "faux-1", 1);
        tool_use.stop_reason = StopReason::ToolUse;
        tool_use.content.push(Content::text("partial"));
        assert_eq!(
            validate_coding_summary_response(tool_use, false)
                .error_message
                .as_deref(),
            Some("Incomplete summarization (stop reason: toolUse)")
        );

        let mut tool_call = AssistantMessage::empty(ice_ai::types::Api::Faux, "faux", "faux-1", 1);
        tool_call.stop_reason = StopReason::Stop;
        tool_call
            .content
            .push(Content::tool_call("call", "read", serde_json::json!({})));
        let tool_call = validate_coding_summary_response(tool_call, false);
        assert_eq!(tool_call.stop_reason, StopReason::Error);
        assert_eq!(
            tool_call.error_message.as_deref(),
            Some("Summarization returned tool calls")
        );

        let mut blank = AssistantMessage::empty(ice_ai::types::Api::Faux, "faux", "faux-1", 1);
        blank.stop_reason = StopReason::Stop;
        blank.content.push(Content::text("  \n\t"));
        assert_eq!(
            validate_coding_summary_response(blank, false)
                .error_message
                .as_deref(),
            Some("Summarization returned no text")
        );

        let mut provider_error =
            AssistantMessage::empty(ice_ai::types::Api::Faux, "faux", "faux-1", 1);
        provider_error.stop_reason = StopReason::Error;
        provider_error.error_message = Some("provider error".into());
        provider_error
            .content
            .push(Content::tool_call("call", "read", serde_json::json!({})));
        assert_eq!(
            validate_coding_summary_response(provider_error.clone(), false),
            provider_error
        );

        let aborted = validate_coding_summary_response(provider_error, true);
        assert_eq!(aborted.stop_reason, StopReason::Aborted);
        assert_eq!(
            aborted.error_message.as_deref(),
            Some("Summarization aborted")
        );

        let mut success = AssistantMessage::empty(ice_ai::types::Api::Faux, "faux", "faux-1", 1);
        success.stop_reason = StopReason::Stop;
        success.content.push(Content::text("  summary \n"));
        success.usage.input = 29;
        let original = success.clone();
        assert_eq!(validate_coding_summary_response(success, false), original);
    }

    #[tokio::test]
    async fn coding_summary_validation_failure_is_not_retried_and_post_response_cancel_wins() {
        use ice_ai::providers::faux::{FauxProvider, FauxScript};
        use std::sync::atomic::{AtomicUsize, Ordering};

        let provider = FauxProvider::new(FauxScript::new());
        let model = provider.default_model().clone();
        let calls = Arc::new(AtomicUsize::new(0));
        let stream_calls = Arc::clone(&calls);
        let invalid_stream: SummarizationStreamFn = Arc::new(move |_model, _context, _options| {
            stream_calls.fetch_add(1, Ordering::SeqCst);
            let mut response =
                AssistantMessage::empty(ice_ai::types::Api::Faux, "faux", "faux-1", 1);
            response.stop_reason = StopReason::Length;
            response.content.push(Content::text("partial"));
            Box::pin(async move { response }) as BoxFuture<'static, AssistantMessage>
        });
        let mut retry_options = AssistantRetryOptions::new(
            Some(AssistantRetryPolicy {
                enabled: true,
                max_retries: 3,
                base_delay_ms: 0,
                max_delay_ms: None,
            }),
            CancellationToken::new(),
        );
        let response = complete_validated_coding_summary(
            provider.clone(),
            &model,
            &Context::default(),
            &SimpleStreamOptions::default(),
            Some(invalid_stream),
            retry_options.clone(),
        )
        .await
        .unwrap();
        assert_eq!(response.stop_reason, StopReason::Error);
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        let signal = CancellationToken::new();
        let stream_signal = signal.clone();
        let success_stream: SummarizationStreamFn = Arc::new(move |_model, _context, _options| {
            stream_signal.cancel();
            let mut response =
                AssistantMessage::empty(ice_ai::types::Api::Faux, "faux", "faux-1", 1);
            response.stop_reason = StopReason::Stop;
            response.content.push(Content::text("completed"));
            Box::pin(async move { response }) as BoxFuture<'static, AssistantMessage>
        });
        retry_options.signal = signal;
        let response = complete_validated_coding_summary(
            provider,
            &model,
            &Context::default(),
            &SimpleStreamOptions {
                signal: retry_options.signal.clone(),
                ..SimpleStreamOptions::default()
            },
            Some(success_stream),
            retry_options,
        )
        .await
        .unwrap();
        assert_eq!(response.stop_reason, StopReason::Aborted);
        assert_eq!(
            response.error_message.as_deref(),
            Some("Summarization aborted")
        );
    }

    #[tokio::test]
    async fn injected_coding_summary_stream_is_immutable_and_reuses_request_id_on_retry() {
        use ice_ai::providers::faux::{FauxProvider, FauxScript};
        use ice_ai::types::{Schema, Tool};
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Mutex;

        let provider = FauxProvider::new(FauxScript::new());
        let model = provider.default_model().clone();
        let mut context = Context::default();
        context.tools.push(Tool {
            name: "read".into(),
            description: "read a file".into(),
            parameters: Schema::empty_object(),
            constrained_sampling: None,
        });
        let mut options = SimpleStreamOptions::default();
        options.cache_retention = CacheRetention::Long;
        options.session_id = Some("caller-session".into());
        let observations = Arc::new(Mutex::new(
            Vec::<(usize, CacheRetention, Option<String>)>::new(),
        ));
        let seen = Arc::clone(&observations);
        let attempts = Arc::new(AtomicUsize::new(0));
        let produce_attempts = Arc::clone(&attempts);
        let stream_fn: SummarizationStreamFn =
            Arc::new(move |_model, request_context, request_options| {
                seen.lock().unwrap().push((
                    request_context.tools.len(),
                    request_options.cache_retention,
                    request_options.session_id.clone(),
                ));
                let attempt = produce_attempts.fetch_add(1, Ordering::SeqCst);
                let response = if attempt == 0 {
                    AssistantMessage::terminal(
                        ice_ai::types::Api::Faux,
                        "faux",
                        "faux-1",
                        StopReason::Error,
                        "terminated",
                        1,
                    )
                } else {
                    let mut response =
                        AssistantMessage::empty(ice_ai::types::Api::Faux, "faux", "faux-1", 2);
                    response.stop_reason = StopReason::Stop;
                    response.content.push(Content::text("  summary \n"));
                    response.usage.input = 29;
                    response
                };
                Box::pin(async move { response }) as BoxFuture<'static, AssistantMessage>
            });
        let retry = AssistantRetryOptions::new(
            Some(AssistantRetryPolicy {
                enabled: true,
                max_retries: 1,
                base_delay_ms: 0,
                max_delay_ms: None,
            }),
            options.signal.clone(),
        );

        let response = complete_validated_coding_summary(
            provider,
            &model,
            &context,
            &options,
            Some(stream_fn),
            retry,
        )
        .await
        .unwrap();
        assert_eq!(response.stop_reason, StopReason::Stop);
        assert_eq!(response.content, vec![Content::text("  summary \n")]);
        assert_eq!(response.usage.input, 29);
        let observations = observations.lock().unwrap();
        assert_eq!(observations.len(), 2);
        assert_eq!(observations[0].0, 0);
        assert_eq!(observations[1].0, 0);
        assert_eq!(observations[0].1, CacheRetention::None);
        assert_eq!(observations[1].1, CacheRetention::None);
        assert_eq!(observations[0].2, observations[1].2);
        assert_ne!(observations[0].2.as_deref(), Some("caller-session"));
        assert!(Uuid::parse_str(observations[0].2.as_deref().unwrap()).is_ok());
        assert_eq!(context.tools.len(), 1);
        assert_eq!(options.cache_retention, CacheRetention::Long);
        assert_eq!(options.session_id.as_deref(), Some("caller-session"));
    }

    #[test]
    fn extract_file_operations_seeds_from_prev_compaction_details() {
        let details = serde_json::json!({"readFiles": ["/r"], "modifiedFiles": ["/m"]});
        let compaction = Entry::Compaction(crate::session::types::CompactionEntry {
            base: EntryBase {
                entry_type: "compaction".into(),
                id: "c1".into(),
                seq: 1,
                parent_id: None,
                timestamp: 1,
            },
            summary: "s".into(),
            retained_tail: Vec::new(),
            tokens_before: 0,
            details: Some(details),
            usage: None,
        });
        let ops = extract_file_operations(&[], &[compaction], Some(0));
        assert!(ops.read.contains("/r"));
        assert!(ops.edited.contains("/m"));
    }
}
