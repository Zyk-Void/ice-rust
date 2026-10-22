//! Mirrors `packages/agent/src/harness/compaction/branch-summarization.ts` —
//! summarize an abandoned branch tail into a `branchSummary` entry when the
//! conversation navigates away from it.
//!
//! `collect_entries_for_branch_summary` walks a [`SessionTree`] (the Rust
//! equivalent of TS `Session`); `prepare_branch_entries` + `generate_branch_sum
//! mary` are pure/IO-bounded over the collected entries. The generated summary
//! reuses [`complete_simple_with_ret  ries`] + the compaction prompts from
//! [`crate::compaction::compaction`].

use std::collections::BTreeSet;

use ice_agent::message::AgentMessage;
use ice_ai::types::{Content, Context, Message, StopReason, Usage, UserContent, UserMessage};
use serde::{Deserialize, Serialize};

use crate::compaction::compaction::{
    complete_simple_with_retries, now_ms, CompactionLlmOptions, SUMMARIZATION_SYSTEM_PROMPT,
};
use crate::compaction::tokens::{
    compute_file_lists, create_file_ops, estimate_tokens, extract_file_ops_from_message,
    format_file_operations, serialize_conversation, FileOperations,
};
use crate::error::{SessionError, SessionResult};
use crate::messages::convert_to_llm;
use crate::messages::{create_branch_summary_message, create_compaction_summary_message};
use crate::session::types::{BranchBounds, Entry, EntryQuery, SessionTree};

// Re-export so callers can reach the branch-summary error type via
// `compaction::branch_summary`.
pub use crate::compaction::compaction::CompactionError as BranchSummaryError;

/// Generated branch summary data ready to be persisted. Mirrors TS
/// `BranchSummaryResult`.
#[derive(Debug, Clone, PartialEq)]
pub struct BranchSummaryResult {
    pub summary: String,
    pub usage: Option<Usage>,
    pub read_files: Vec<String>,
    pub modified_files: Vec<String>,
}

/// File-operation details stored on generated branch summary entries. Mirrors
/// TS `BranchSummaryDetails`. Serialized so it can round-trip on a
/// `BranchSummaryEntry::details: Option<JsonValue>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchSummaryDetails {
    pub read_files: Vec<String>,
    pub modified_files: Vec<String>,
}

/// Prepared branch content for summarization. Mirrors `BranchPreparation`.
#[derive(Debug, Clone, PartialEq)]
pub struct BranchPreparation {
    pub messages: Vec<AgentMessage>,
    pub file_ops: FileOperations,
    pub total_tokens: i64,
}

/// Entries selected for branch summarization. Mirrors `CollectEntriesResult`.
#[derive(Debug, Clone, PartialEq)]
pub struct CollectEntriesResult {
    pub entries: Vec<Entry>,
    pub common_ancestor_id: Option<String>,
}

/// Options for generating a branch summary. Mirrors
/// `GenerateBranchSummaryOptions`; the `models`/`model`/`signal`/`retry` fields
/// consolidate into [`CompactionLlmOptions`], and `reserveTokens` defaults to
/// the compaction default. Use [`GenerateBranchSummaryOptions::new`] because a
/// valid provider/model routing configuration has no meaningful `Default`.
#[derive(Clone)]
pub struct GenerateBranchSummaryOptions {
    pub llm: CompactionLlmOptions,
    /// Replace the default prompt with `custom_instructions` instead of
    /// appending. Mirrors TS `replaceInstructions`.
    pub replace_instructions: bool,
    /// Tokens reserved for prompt + model output. Defaults to 16384.
    pub reserve_tokens: i64,
}

impl GenerateBranchSummaryOptions {
    /// Default reserve tokens (mirrors TS `reserveTokens = 16384`).
    pub const DEFAULT_RESERVE_TOKENS: i64 = 16384;
}

/// `getMessageFromEntry` for branch summarization. Mirrors the local helper in
/// `branch-summarization.ts`: drops `toolResult` messages (their context is
/// covered by the assistant tool call); renders `branch_summary`/`compaction`
/// entries as their summary messages; drops metadata-only entries.
fn get_message_from_entry(entry: &Entry) -> Option<AgentMessage> {
    match entry {
        Entry::Message(m) => {
            if matches!(
                m.message.role(),
                ice_agent::message::AgentMessageRole::ToolResult
            ) {
                return None;
            }
            Some(m.message.clone())
        }
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
        // thinking_level_change / model_change / active_tools_change / custom.
        _ => None,
    }
}

/// Collect entries that should be summarized before navigating to a different
/// session tree entry. Mirrors `collectEntriesForBranchSummary`.
///
/// Walks `old_leaf_id` back toward the common ancestor with `target_id`'s path
/// (exclusive of the ancestor), reversed into chronological order. Returns
/// empty when `old_leaf_id` is absent or the empty string.
pub async fn collect_entries_for_branch_summary<S: SessionTree + ?Sized>(
    session: &S,
    old_leaf_id: Option<&str>,
    target_id: &str,
) -> SessionResult<CollectEntriesResult> {
    let old_leaf_id = match old_leaf_id {
        Some(id) if !id.is_empty() => id,
        Some(_) | None => {
            return Ok(CollectEntriesResult {
                entries: Vec::new(),
                common_ancestor_id: None,
            })
        }
    };

    let old_path: BTreeSet<String> = session
        .find_entries_on_branch(
            &EntryQuery::default(),
            &BranchBounds {
                start: Some(old_leaf_id.to_string()),
                ..Default::default()
            },
        )
        .await?
        .iter()
        .map(|e| e.id().to_string())
        .collect();

    let target_path = session
        .find_entries_on_branch(
            &EntryQuery::default(),
            &BranchBounds {
                start: Some(target_id.to_string()),
                ..Default::default()
            },
        )
        .await?;

    let common_ancestor_id = target_path
        .iter()
        .find(|e| old_path.contains(e.id()))
        .map(|e| e.id().to_string());

    let mut entries: Vec<Entry> = Vec::new();
    let mut current: Option<String> = Some(old_leaf_id.to_string());
    while let Some(cur) = current {
        if Some(&cur) == common_ancestor_id.as_ref() {
            break;
        }
        let entry = session.get_entry(&cur).await?.ok_or_else(|| {
            SessionError::invalid_entry(format!(
                "Entry {cur} not found while collecting branch summary"
            ))
        })?;
        current = entry.parent_id().map(|s| s.to_string());
        entries.push(entry);
    }
    entries.reverse();
    Ok(CollectEntriesResult {
        entries,
        common_ancestor_id,
    })
}

/// Prepare branch entries for summarization within an optional token budget.
/// Mirrors `prepareBranchEntries`: seed file ops from prior branch_summary
/// details, then walk back accumulating messages until the budget is exceeded
/// (compaction/branch_summary entries get a 0.9×budget grace before dropping).
pub fn prepare_branch_entries(entries: &[Entry], token_budget: i64) -> BranchPreparation {
    let mut messages: Vec<AgentMessage> = Vec::new();
    let mut file_ops = create_file_ops();
    let mut total_tokens = 0i64;

    // Seed file ops from prior branch_summary details.
    for entry in entries {
        if let Entry::BranchSummary(b) = entry {
            if let Some(details_value) = &b.details {
                if let Some(arr) = details_value.get("readFiles").and_then(|v| v.as_array()) {
                    for f in arr {
                        if let Some(s) = f.as_str() {
                            file_ops.read.insert(s.to_string());
                        }
                    }
                }
                if let Some(arr) = details_value
                    .get("modifiedFiles")
                    .and_then(|v| v.as_array())
                {
                    for f in arr {
                        if let Some(s) = f.as_str() {
                            file_ops.edited.insert(s.to_string());
                        }
                    }
                }
            }
        }
    }

    // Walk back, unshifting into chronological order.
    if token_budget > 0 {
        for entry in entries.iter().rev() {
            let message = match get_message_from_entry(entry) {
                Some(m) => m,
                None => continue,
            };
            extract_file_ops_from_message(&message, &mut file_ops);
            let tokens = estimate_tokens(&message);
            let exceeds_budget = (total_tokens as i128) + (tokens as i128) > token_budget as i128;
            if exceeds_budget {
                let within_summary_grace = (total_tokens as i128) * 10 < (token_budget as i128) * 9;
                if matches!(entry, Entry::Compaction(_) | Entry::BranchSummary(_))
                    && within_summary_grace
                {
                    messages.insert(0, message);
                    total_tokens = total_tokens.saturating_add(tokens);
                }
                break;
            }
            messages.insert(0, message);
            total_tokens = total_tokens.saturating_add(tokens);
        }
    } else {
        // No budget → include everything in chronological order.
        for entry in entries {
            if let Some(message) = get_message_from_entry(entry) {
                extract_file_ops_from_message(&message, &mut file_ops);
                total_tokens = total_tokens.saturating_add(estimate_tokens(&message));
                messages.push(message);
            }
        }
    }

    BranchPreparation {
        messages,
        file_ops,
        total_tokens,
    }
}

// ---------------------------------------------------------------------------
// Prompts — verbatim from branch-summarization.ts
// ---------------------------------------------------------------------------

const BRANCH_SUMMARY_PREAMBLE: &str =
    "The user explored a different conversation branch before returning here.\nSummary of that exploration:\n\n";

const BRANCH_SUMMARY_PROMPT: &str = "Create a structured summary of this conversation branch for context when returning later.\n\nUse this EXACT format:\n\n## Goal\n[What was the user trying to accomplish in this branch?]\n\n## Constraints & Preferences\n- [Any constraints, preferences, or requirements mentioned]\n- [Or \"(none)\" if none were mentioned]\n\n## Progress\n### Done\n- [x] [Completed tasks/changes]\n\n### In Progress\n- [ ] [Work that was started but not finished]\n\n### Blocked\n- [Issues preventing progress, if any]\n\n## Key Decisions\n- **[Decision]**: [Brief rationale]\n\n## Next Steps\n1. [What should happen next to continue this work]\n\nKeep each section concise. Preserve exact file paths, function names, and error messages.";

fn branch_summary_instructions(custom_instructions: Option<&str>, replace: bool) -> String {
    match (
        custom_instructions.filter(|instructions| !instructions.is_empty()),
        replace,
    ) {
        (Some(instructions), true) => instructions.to_string(),
        (Some(instructions), false) => {
            format!("{BRANCH_SUMMARY_PROMPT}\n\nAdditional focus: {instructions}")
        }
        (None, _) => BRANCH_SUMMARY_PROMPT.to_string(),
    }
}

fn branch_summary_stream_options(llm: &CompactionLlmOptions) -> ice_ai::SimpleStreamOptions {
    let mut options = ice_ai::SimpleStreamOptions::default();
    options.api_key = llm.api_key.clone();
    options.signal = llm.signal.clone();
    options.max_tokens = Some(2048);
    // Branch summaries intentionally do not forward reasoning settings.
    options
}

/// `generateBranchSummary`. Mirrors TS: budget = `context_window − reserve`
/// (context_window 0 → 128000); empty messages → `"No content to summarize"`;
/// single user message with the conversation+instructions prompt; `maxTokens:
/// 2048`; prepend `BRANCH_SUMMARY_PREAMBLE`; append file-op tags; map
/// `aborted`/`error` to `BranchSummaryError`.
pub async fn generate_branch_summary(
    entries: &[Entry],
    options: &GenerateBranchSummaryOptions,
) -> Result<BranchSummaryResult, BranchSummaryError> {
    let context_window = if options.llm.model.context_window > 0 {
        options.llm.model.context_window as i64
    } else {
        128_000
    };
    let token_budget = context_window.saturating_sub(options.reserve_tokens);

    let prep = prepare_branch_entries(entries, token_budget);

    if prep.messages.is_empty() {
        return Ok(BranchSummaryResult {
            summary: "No content to summarize".to_string(),
            usage: None,
            read_files: Vec::new(),
            modified_files: Vec::new(),
        });
    }

    let llm_messages = convert_to_llm(prep.messages.clone());
    let conversation_text = serialize_conversation(&llm_messages);

    let instructions = branch_summary_instructions(
        options.llm.custom_instructions.as_deref(),
        options.replace_instructions,
    );
    let prompt_text =
        format!("<conversation>\n{conversation_text}\n</conversation>\n\n{instructions}");

    let summarization_messages = vec![Message::User(UserMessage::new(
        UserContent::Blocks(vec![Content::text(prompt_text)]),
        now_ms(),
    ))];
    let context = Context {
        system_prompt: Some(SUMMARIZATION_SYSTEM_PROMPT.to_string()),
        messages: summarization_messages,
        tools: Vec::new(),
    };

    let req_opts = branch_summary_stream_options(&options.llm);
    let response = complete_simple_with_retries(
        options.llm.provider.clone(),
        options.llm.model.clone(),
        context,
        req_opts,
        options.llm.retry.clone(),
    )
    .await;

    if response.stop_reason == StopReason::Aborted {
        return Err(BranchSummaryError::aborted(
            response
                .error_message
                .as_deref()
                .filter(|message| !message.is_empty())
                .unwrap_or("Branch summary aborted"),
        ));
    }
    if response.stop_reason == StopReason::Error {
        return Err(BranchSummaryError::summarization_failed(format!(
            "Branch summary failed: {}",
            response
                .error_message
                .as_deref()
                .filter(|message| !message.is_empty())
                .unwrap_or("Unknown error")
        )));
    }

    let mut summary = Content::text_only(&response.content, "\n");
    summary = format!("{BRANCH_SUMMARY_PREAMBLE}{summary}");
    let (read_files, modified_files) = compute_file_lists(&prep.file_ops);
    summary += &format_file_operations(&read_files, &modified_files);

    Ok(BranchSummaryResult {
        summary: if summary.is_empty() {
            "No summary generated".to_string()
        } else {
            summary
        },
        usage: Some(response.usage.clone()),
        read_files,
        modified_files,
    })
}

// Re-export the harness-level `CompactionLlmOptions` alias for callers building
// branch-summary options, and a tiny helper to construct the default reserve.
impl GenerateBranchSummaryOptions {
    /// Build options with a given LLM config and the default reserve.
    pub fn new(llm: CompactionLlmOptions) -> Self {
        Self {
            llm,
            replace_instructions: false,
            reserve_tokens: Self::DEFAULT_RESERVE_TOKENS,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::types::EntryBase;

    fn user_msg(text: &str, seq: u64) -> Entry {
        user_msg_with_parent(text, format!("e{seq}"), None, seq)
    }

    fn user_msg_with_parent(
        text: &str,
        id: impl Into<String>,
        parent_id: Option<String>,
        seq: u64,
    ) -> Entry {
        Entry::Message(crate::session::types::MessageEntry {
            base: EntryBase {
                entry_type: "message".into(),
                id: id.into(),
                seq,
                parent_id,
                timestamp: seq as i64,
            },
            message: AgentMessage::User(UserMessage::new(
                UserContent::Text(text.into()),
                seq as i64,
            )),
            terminate: None,
        })
    }

    #[derive(Default)]
    struct StubSession {
        entries: std::collections::BTreeMap<String, Entry>,
        paths: std::collections::BTreeMap<String, Vec<Entry>>,
    }

    #[async_trait::async_trait]
    impl SessionTree for StubSession {
        async fn get_leaf_id(&self) -> crate::error::SessionResult<Option<String>> {
            unreachable!()
        }
        async fn get_entry(&self, id: &str) -> crate::error::SessionResult<Option<Entry>> {
            Ok(self.entries.get(id).cloned())
        }
        async fn get_stats(
            &self,
        ) -> crate::error::SessionResult<crate::session::types::SessionStats> {
            unreachable!()
        }
        async fn get_name(&self) -> crate::error::SessionResult<Option<String>> {
            unreachable!()
        }
        async fn set_name(&self, _name: Option<&str>) -> crate::error::SessionResult<()> {
            unreachable!()
        }
        async fn get_label(&self, _target_id: &str) -> crate::error::SessionResult<Option<String>> {
            unreachable!()
        }
        async fn set_label(
            &self,
            _target_id: &str,
            _label: Option<&str>,
        ) -> crate::error::SessionResult<()> {
            unreachable!()
        }
        async fn find_entries(
            &self,
            _query: &EntryQuery,
        ) -> crate::error::SessionResult<Vec<Entry>> {
            unreachable!()
        }
        async fn find_entry(
            &self,
            _query: &EntryQuery,
        ) -> crate::error::SessionResult<Option<Entry>> {
            unreachable!()
        }
        async fn find_entries_on_branch(
            &self,
            _query: &EntryQuery,
            bounds: &BranchBounds,
        ) -> crate::error::SessionResult<Vec<Entry>> {
            Ok(bounds
                .start
                .as_ref()
                .and_then(|start| self.paths.get(start))
                .cloned()
                .unwrap_or_default())
        }
        async fn find_entry_on_branch(
            &self,
            _query: &EntryQuery,
            _bounds: &BranchBounds,
        ) -> crate::error::SessionResult<Option<Entry>> {
            unreachable!()
        }
        async fn append_message(
            &self,
            _message: AgentMessage,
        ) -> crate::error::SessionResult<String> {
            unreachable!()
        }
        async fn append_custom_entry(
            &self,
            _custom_type: &str,
            _data: Option<crate::session::types::JsonValue>,
        ) -> crate::error::SessionResult<String> {
            unreachable!()
        }
    }

    #[tokio::test]
    async fn collect_branch_treats_empty_old_leaf_as_empty_without_session_reads() {
        let result =
            collect_entries_for_branch_summary(&StubSession::default(), Some(""), "target")
                .await
                .unwrap();
        assert!(result.entries.is_empty());
        assert_eq!(result.common_ancestor_id, None);
    }

    #[tokio::test]
    async fn collect_branch_excludes_deepest_common_ancestor() {
        let root = user_msg_with_parent("root", "root", None, 1);
        let common = user_msg_with_parent("common", "common", Some("root".into()), 2);
        let old_leaf = user_msg_with_parent("old", "old", Some("common".into()), 3);
        let target_leaf = user_msg_with_parent("target", "target", Some("common".into()), 4);
        let entries = [&root, &common, &old_leaf, &target_leaf];
        let mut session = StubSession::default();
        for entry in entries {
            session
                .entries
                .insert(entry.id().to_string(), entry.clone());
        }
        session.paths.insert(
            "old".into(),
            vec![old_leaf.clone(), common.clone(), root.clone()],
        );
        session
            .paths
            .insert("target".into(), vec![target_leaf, common.clone(), root]);

        let result = collect_entries_for_branch_summary(&session, Some("old"), "target")
            .await
            .unwrap();
        assert_eq!(result.common_ancestor_id.as_deref(), Some("common"));
        assert_eq!(
            result.entries.iter().map(Entry::id).collect::<Vec<_>>(),
            ["old"]
        );
    }

    #[test]
    fn empty_branch_instructions_fall_back_and_reasoning_is_not_forwarded() {
        use ice_ai::providers::faux::{FauxProvider, FauxScript};
        use ice_ai::ThinkingLevel;

        assert_eq!(
            branch_summary_instructions(Some(""), true),
            BRANCH_SUMMARY_PROMPT
        );
        assert_eq!(
            branch_summary_instructions(Some(""), false),
            BRANCH_SUMMARY_PROMPT
        );

        let provider = FauxProvider::new(FauxScript::new());
        let mut model = provider.default_model().clone();
        model.reasoning = true;
        let llm = CompactionLlmOptions {
            provider,
            model,
            api_key: None,
            signal: tokio_util::sync::CancellationToken::new(),
            thinking_level: Some(ThinkingLevel::High),
            retry: None,
            custom_instructions: None,
        };
        let request = branch_summary_stream_options(&llm);
        assert_eq!(request.max_tokens, Some(2048));
        assert_eq!(request.reasoning, None);
        assert_eq!(
            GenerateBranchSummaryOptions::new(llm).reserve_tokens,
            GenerateBranchSummaryOptions::DEFAULT_RESERVE_TOKENS
        );
    }

    #[test]
    fn prepare_branch_entries_no_budget_includes_all_non_toolresult() {
        let entries = vec![user_msg("a", 1), user_msg("b", 2)];
        let p = prepare_branch_entries(&entries, 0);
        assert_eq!(p.messages.len(), 2);
        assert_eq!(prepare_branch_entries(&entries, -1).messages.len(), 2);
    }

    #[test]
    fn prepare_branch_entries_budget_drops_overflow_keeps_trailing_fit() {
        // Walk is back-to-front. Trailing "XYZ" (ceil(3/4)=1) fits in budget 3
        // and is unshifted; leading 26-char msg (ceil(26/4)=7) overflows
        // (1+7=8 > 3), is a plain user message (not compaction/branch_summary),
        // so it is dropped and the loop breaks. Result: only the trailing msg.
        let entries = vec![
            user_msg("abcdefghijklmnopqrstuvwxyz", 1),
            user_msg("XYZ", 2),
        ];
        let p = prepare_branch_entries(&entries, 3);
        assert_eq!(p.messages.len(), 1);
        assert!(matches!(
            &p.messages[0],
            AgentMessage::User(u) if matches!(u.content, UserContent::Text(ref t) if t == "XYZ")
        ));
    }

    #[test]
    fn fractional_ninety_percent_grace_includes_checkpoint_at_budget_three() {
        let checkpoint = Entry::BranchSummary(crate::session::types::BranchSummaryEntry {
            base: EntryBase {
                entry_type: "branch_summary".into(),
                id: "branch".into(),
                seq: 1,
                parent_id: None,
                timestamp: 1,
            },
            from_id: "leaf".into(),
            summary: "abcdefgh".into(), // two estimated tokens
            details: None,
            usage: None,
        });
        let preparation = prepare_branch_entries(&[checkpoint, user_msg("abcdefgh", 2)], 3);
        assert_eq!(preparation.total_tokens, 4);
        assert_eq!(preparation.messages.len(), 2);
    }

    #[test]
    fn rejected_entry_still_contributes_file_attribution_and_empty_paths_are_ignored() {
        let mut assistant =
            ice_ai::types::AssistantMessage::empty(ice_ai::types::Api::Faux, "faux", "faux", 1);
        assistant.content.push(Content::tool_call(
            "read-empty",
            "read",
            serde_json::json!({"path": ""}),
        ));
        assistant.content.push(Content::tool_call(
            "read-file",
            "read",
            serde_json::json!({"path": "/rejected.txt"}),
        ));
        let rejected = Entry::Message(crate::session::types::MessageEntry {
            base: EntryBase {
                entry_type: "message".into(),
                id: "rejected".into(),
                seq: 1,
                parent_id: None,
                timestamp: 1,
            },
            message: AgentMessage::Assistant(Box::new(assistant)),
            terminate: None,
        });
        let preparation = prepare_branch_entries(&[rejected, user_msg("ok", 2)], 1);
        assert_eq!(preparation.messages.len(), 1);
        assert!(preparation.file_ops.read.contains("/rejected.txt"));
        assert!(!preparation.file_ops.read.contains(""));
    }

    #[test]
    fn prepare_branch_entries_seeds_from_prior_branch_summary_details() {
        let details = serde_json::json!({"readFiles": ["/r"], "modifiedFiles": ["/m"]});
        let b = Entry::BranchSummary(crate::session::types::BranchSummaryEntry {
            base: EntryBase {
                entry_type: "branch_summary".into(),
                id: "b1".into(),
                seq: 1,
                parent_id: None,
                timestamp: 1,
            },
            from_id: "leaf".into(),
            summary: "s".into(),
            details: Some(details),
            usage: None,
        });
        let p = prepare_branch_entries(&[b], 0);
        assert!(p.file_ops.read.contains("/r"));
        assert!(p.file_ops.edited.contains("/m"));
    }
}
