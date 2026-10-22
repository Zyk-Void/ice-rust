//! Mirrors the token-estimation + file-operation + conversation-serialization
//! pieces of `packages/agent/src/harness/compaction/utils.ts`, plus
//! `estimate_tokens`/`estimate_context_tokens` (from `compaction.ts`).
//!
//! The context-builder pieces (`build_session_context`/
//! `default_context_entry_transform`/`SessionContext`) are re-exported from
//! the canonical `session::context` module (the authoritative
//! `session/context.ts` port) — compaction's `tokens_before` path delegates to
//! that module's no-options wrapper so the two never diverge.

use std::collections::BTreeSet;

use ice_agent::message::AgentMessage;
use ice_ai::types::{AssistantMessage, Content, Message, Usage};

use crate::messages::{bash_execution_data, branch_summary_data, compaction_summary_data};
use crate::session::types::Entry;

// ---------------------------------------------------------------------------
// File operations — mirrors utils.ts FileOperations / extractFileOpsFromMessage
// / computeFileLists / formatFileOperations
// ---------------------------------------------------------------------------

/// Files touched by a session branch or compaction range. Mirrors TS
/// `FileOperations`. `BTreeSet` (not `HashSet`) so derived lists are sorted and
/// deterministic — the TS code sorts at `computeFileLists` time; we keep the
/// sets unsorted and sort once at list-computation, matching that.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileOperations {
    pub read: BTreeSet<String>,
    pub written: BTreeSet<String>,
    pub edited: BTreeSet<String>,
}

/// `createFileOps()` — an empty accumulator.
pub fn create_file_ops() -> FileOperations {
    FileOperations::default()
}

/// Add file operations from an assistant message's tool calls. Mirrors
/// `extractFileOpsFromMessage`: only `assistant` messages, only `toolCall`
/// blocks, classified by tool `name` (`read`/`write`/`edit`) using
/// `arguments.path`.
pub fn extract_file_ops_from_message(message: &AgentMessage, file_ops: &mut FileOperations) {
    let assistant = match message {
        AgentMessage::Assistant(a) => a,
        _ => return,
    };
    for block in &assistant.content {
        if let Content::ToolCall(call) = block {
            let path = match call.arguments.get("path") {
                Some(v) => match v.as_str() {
                    Some(s) if !s.is_empty() => s,
                    _ => continue,
                },
                None => continue,
            };
            match call.name.as_str() {
                "read" => {
                    file_ops.read.insert(path.to_string());
                }
                "write" => {
                    file_ops.written.insert(path.to_string());
                }
                "edit" => {
                    file_ops.edited.insert(path.to_string());
                }
                _ => {}
            }
        }
    }
}

/// Compute sorted read-only and modified file lists from accumulated operations.
/// Mirrors `computeFileLists`: `modified = edited ∪ written`; `readOnly = read
/// − modified`.
pub fn compute_file_lists(file_ops: &FileOperations) -> (Vec<String>, Vec<String>) {
    let modified: BTreeSet<String> = file_ops.edited.union(&file_ops.written).cloned().collect();
    let read_only: Vec<String> = file_ops
        .read
        .iter()
        .filter(|f| !modified.contains(*f))
        .cloned()
        .collect();
    let modified_files: Vec<String> = modified.into_iter().collect();
    (read_only, modified_files)
}

/// Format file lists as summary metadata XML tags. Mirrors
/// `formatFileOperations`: empty input → `""`; non-empty → `"\n\n"` + sections
/// joined `"\n\n"`.
pub fn format_file_operations(read_files: &[String], modified_files: &[String]) -> String {
    let mut sections: Vec<String> = Vec::new();
    if !read_files.is_empty() {
        sections.push(format!(
            "<read-files>\n{}\n</read-files>",
            read_files.join("\n")
        ));
    }
    if !modified_files.is_empty() {
        sections.push(format!(
            "<modified-files>\n{}\n</modified-files>",
            modified_files.join("\n")
        ));
    }
    if sections.is_empty() {
        String::new()
    } else {
        format!("\n\n{}", sections.join("\n\n"))
    }
}

// ---------------------------------------------------------------------------
// serializeConversation — mirrors utils.ts (operates on provider Message[])
// ---------------------------------------------------------------------------

const TOOL_RESULT_MAX_UTF16_UNITS: usize = 2000;

/// `safeJsonStringify` — `JSON.stringify` with a fallback. Rust's
/// `serde_json::to_string` can fail on non-finite floats / cycles (cycles
/// impossible for `Value`); mirror the fallback string.
fn safe_json_stringify(value: &serde_json::Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "[unserializable]".to_string())
}

/// Count JavaScript-compatible UTF-16 code units. Astral Unicode scalars count
/// as two units; BMP scalars count as one.
pub fn utf16_code_units(text: &str) -> usize {
    text.encode_utf16().count()
}

fn truncate_for_summary(text: &str, max_units: usize) -> String {
    let total_units = utf16_code_units(text);
    if total_units <= max_units {
        return text.to_string();
    }

    // JS String.slice can leave an isolated surrogate when the boundary falls
    // between an astral scalar's UTF-16 pair. Rust strings cannot represent that,
    // so back up to the preceding Unicode scalar boundary instead. The omitted
    // count is the exact number of original UTF-16 units not retained.
    let mut retained_units = 0usize;
    let mut end = 0usize;
    for (byte_index, ch) in text.char_indices() {
        let char_units = ch.len_utf16();
        if retained_units.saturating_add(char_units) > max_units {
            break;
        }
        retained_units += char_units;
        end = byte_index + ch.len_utf8();
    }
    let omitted_units = total_units.saturating_sub(retained_units);
    format!(
        "{}\n\n[... {omitted_units} more characters truncated]",
        &text[..end]
    )
}

/// Serialize LLM messages to plain text for summarization prompts. Mirrors
/// `serializeConversation`: `[User]`/`[Assistant]`/`[Assistant thinking]`/
/// `[Assistant tool calls]`/`[Tool result]` blocks joined `\n\n`.
pub fn serialize_conversation(messages: &[Message]) -> String {
    let mut parts: Vec<String> = Vec::new();
    for msg in messages {
        match msg {
            Message::User(u) => {
                let content = user_content_text(&u.content);
                if !content.is_empty() {
                    parts.push(format!("[User]: {content}"));
                }
            }
            Message::Assistant(a) => {
                let mut thinking_parts: Vec<String> = Vec::new();
                let mut tool_calls: Vec<String> = Vec::new();
                for block in &a.content {
                    match block {
                        Content::Thinking(t) => thinking_parts.push(t.thinking.clone()),
                        Content::ToolCall(call) => {
                            let args_str = match &call.arguments {
                                serde_json::Value::Object(map) => map
                                    .iter()
                                    .map(|(k, v)| format!("{k}={}", safe_json_stringify(v)))
                                    .collect::<Vec<_>>()
                                    .join(", "),
                                other => safe_json_stringify(other),
                            };
                            tool_calls.push(format!("{}({})", call.name, args_str));
                        }
                        _ => {}
                    }
                }
                if !thinking_parts.is_empty() {
                    parts.push(format!(
                        "[Assistant thinking]: {}",
                        thinking_parts.join("\n")
                    ));
                }
                if a.content.iter().any(|b| matches!(b, Content::Text(_))) {
                    parts.push(format!(
                        "[Assistant]: {}",
                        Content::text_only(&a.content, "\n")
                    ));
                }
                if !tool_calls.is_empty() {
                    parts.push(format!("[Assistant tool calls]: {}", tool_calls.join("; ")));
                }
            }
            Message::ToolResult(t) => {
                let content = Content::text_only(&t.content, "");
                if !content.is_empty() {
                    parts.push(format!(
                        "[Tool result]: {}",
                        truncate_for_summary(&content, TOOL_RESULT_MAX_UTF16_UNITS)
                    ));
                }
            }
        }
    }
    parts.join("\n\n")
}

fn user_content_text(content: &ice_ai::types::UserContent) -> String {
    match content {
        ice_ai::types::UserContent::Text(s) => s.clone(),
        ice_ai::types::UserContent::Blocks(blocks) => Content::text_only(blocks, ""),
    }
}

// ---------------------------------------------------------------------------
// estimateTokens / estimateContextTokens — mirrors compaction.ts
// ---------------------------------------------------------------------------

const ESTIMATED_IMAGE_UNITS: usize = 4800;

/// UTF-16 code-unit count for text+image content. Images retain the fixed 4800
/// estimate used by the TypeScript implementation.
fn estimate_text_and_image_units_content(content: &[Content]) -> usize {
    let mut units = 0usize;
    for block in content {
        match block {
            Content::Text(t) => units = units.saturating_add(utf16_code_units(&t.text)),
            Content::Image(_) => units = units.saturating_add(ESTIMATED_IMAGE_UNITS),
            _ => {}
        }
    }
    units
}

/// Estimate token count for one message using `ceil(UTF-16 units / 4)`.
/// Mirrors `estimateTokens(message)` per role.
pub fn estimate_tokens(message: &AgentMessage) -> i64 {
    let units = estimate_message_units(message);
    let tokens = units.saturating_add(3) / 4;
    i64::try_from(tokens).unwrap_or(i64::MAX)
}

fn estimate_message_units(message: &AgentMessage) -> usize {
    match message {
        AgentMessage::User(u) => match &u.content {
            ice_ai::types::UserContent::Text(s) => utf16_code_units(s),
            ice_ai::types::UserContent::Blocks(blocks) => {
                estimate_text_and_image_units_content(blocks)
            }
        },
        AgentMessage::Assistant(a) => estimate_assistant_units(a),
        AgentMessage::ToolResult(t) => estimate_text_and_image_units_content(&t.content),
        AgentMessage::Custom(c) => estimate_custom_units(c),
    }
}

fn estimate_assistant_units(a: &AssistantMessage) -> usize {
    let mut units = 0usize;
    for block in &a.content {
        match block {
            Content::Text(t) => units = units.saturating_add(utf16_code_units(&t.text)),
            Content::Thinking(t) => units = units.saturating_add(utf16_code_units(&t.thinking)),
            Content::ToolCall(call) => {
                units = units
                    .saturating_add(utf16_code_units(&call.name))
                    .saturating_add(utf16_code_units(&safe_json_stringify(&call.arguments)));
            }
            _ => {}
        }
    }
    units
}

fn estimate_custom_units(c: &ice_agent::message::CustomMessage) -> usize {
    match c.role.as_str() {
        crate::messages::BASH_EXECUTION_ROLE => {
            if let Some(d) = bash_execution_data(&AgentMessage::Custom(c.clone())) {
                utf16_code_units(&d.command).saturating_add(utf16_code_units(&d.output))
            } else {
                0
            }
        }
        crate::messages::BRANCH_SUMMARY_ROLE => {
            branch_summary_data(&AgentMessage::Custom(c.clone()))
                .map(|d| utf16_code_units(&d.summary))
                .unwrap_or(0)
        }
        crate::messages::COMPACTION_SUMMARY_ROLE => {
            compaction_summary_data(&AgentMessage::Custom(c.clone()))
                .map(|d| utf16_code_units(&d.summary))
                .unwrap_or(0)
        }
        crate::messages::CUSTOM_ROLE => estimate_text_and_image_units_content(&c.content),
        // The registered harness roles are a closed projection. Unknown custom
        // roles have no guaranteed rendering contract and estimate to zero.
        _ => 0,
    }
}

/// Return the last valid assistant usage (stop_reason not aborted/error,
/// `context_tokens > 0`) along with its index, or `None`. Mirrors
/// `getLastAssistantUsageInfo`.
fn last_assistant_usage_info(messages: &[AgentMessage]) -> Option<(Usage, usize)> {
    for (i, msg) in messages.iter().enumerate().rev() {
        if let AgentMessage::Assistant(a) = msg {
            if a.stop_reason != ice_ai::types::StopReason::Aborted
                && a.stop_reason != ice_ai::types::StopReason::Error
                && a.usage.context_tokens() > 0
            {
                return Some((a.usage.clone(), i));
            }
        }
    }
    None
}

/// Estimated context-token usage for a message list. Mirrors
/// `estimateContextTokens` + `ContextUsageEstimate`.
pub struct ContextUsageEstimate {
    pub tokens: i64,
    pub usage_tokens: i64,
    pub trailing_tokens: i64,
    pub last_usage_index: Option<usize>,
}

pub fn estimate_context_tokens(messages: &[AgentMessage]) -> ContextUsageEstimate {
    match last_assistant_usage_info(messages) {
        None => {
            let mut estimated = 0i64;
            for m in messages {
                estimated = estimated.saturating_add(estimate_tokens(m));
            }
            ContextUsageEstimate {
                tokens: estimated,
                usage_tokens: 0,
                trailing_tokens: estimated,
                last_usage_index: None,
            }
        }
        Some((usage, index)) => {
            let usage_tokens = usage.context_tokens();
            let mut trailing_tokens = 0i64;
            for m in messages.iter().skip(index + 1) {
                trailing_tokens = trailing_tokens.saturating_add(estimate_tokens(m));
            }
            ContextUsageEstimate {
                tokens: usage_tokens.saturating_add(trailing_tokens),
                usage_tokens,
                trailing_tokens,
                last_usage_index: Some(index),
            }
        }
    }
}

/// Estimate a coding request using the known composed prompt and tool schemas.
/// Provider usage can already include both, so take the larger of fresh input
/// estimation and historical usage instead of adding overhead to usage twice.
/// This does not execute asynchronous request hooks or claim tokenizer accuracy.
pub(crate) fn estimate_request_tokens(
    messages: &[AgentMessage],
    system_prompt: &str,
    tools: &[ice_ai::types::Tool],
) -> i64 {
    let text_tokens = |text: &str| {
        i64::try_from(utf16_code_units(text).saturating_add(3) / 4).unwrap_or(i64::MAX)
    };
    let mut estimated = messages.iter().fold(0i64, |sum, message| {
        sum.saturating_add(estimate_tokens(message))
    });
    estimated = estimated.saturating_add(text_tokens(system_prompt));
    if !tools.is_empty() {
        // Tool definitions contain only serializable schema data. Fail closed
        // on unexpected serialization errors rather than estimating zero.
        let schema_tokens = serde_json::to_string(tools)
            .map(|json| text_tokens(&json))
            .unwrap_or(i64::MAX);
        estimated = estimated.saturating_add(schema_tokens);
    }
    estimated.max(estimate_context_tokens(messages).tokens)
}

// ---------------------------------------------------------------------------
// build_session_context — compaction's tokensBefore path delegates to the
// canonical `session::context` module (the authoritative `session/context.ts`
// port). The no-options shape compaction needs is `build_session_context_default`.
// ---------------------------------------------------------------------------

/// `defaultContextEntryTransform` — re-exported from the canonical context
/// module so compaction callers reach it without a second path. Mirrors
/// `defaultContextEntryTransform`.
pub use crate::session::context::default_context_entry_transform;

/// `SessionContext` — re-exported from the canonical context module. Mirrors TS
/// `SessionContext`. Carried in `compaction::tokens` for tokens-path callers.
pub use crate::session::context::SessionContext;

/// `buildSessionContext(pathEntries)` (no options) — the shape compaction uses
/// for `tokensBefore`. Delegates to the canonical context builder. Mirrors TS
/// `buildSessionContext`.
pub fn build_session_context(path_entries: &[Entry]) -> SessionContext {
    crate::session::context::build_session_context(path_entries, &Default::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ice_agent::message::AgentMessage;
    use ice_ai::types::{UserContent, UserMessage};

    fn user(text: &str) -> AgentMessage {
        AgentMessage::User(UserMessage::new(UserContent::Text(text.into()), 1))
    }

    #[test]
    fn estimate_tokens_user_chars_div_4_ceil() {
        // 8 chars → ceil(8/4)=2.
        assert_eq!(estimate_tokens(&user("abcdefgh")), 2);
        // 9 chars → ceil(9/4)=3.
        assert_eq!(estimate_tokens(&user("abcdefghi")), 3);
        assert_eq!(estimate_tokens(&user("")), 0);
    }

    #[test]
    fn request_estimate_counts_unicode_prompt_and_tool_schema() {
        let messages = vec![user("😀😀😀")];
        let tools = vec![ice_ai::types::Tool {
            name: "read".into(),
            description: "Read 日本語 😀".into(),
            parameters:
                serde_json::json!({"type": "object", "properties": {"path": {"type": "string"}}})
                    .into(),
            constrained_sampling: None,
        }];
        let schema = serde_json::to_string(&tools).unwrap();
        let expected_schema = ((schema.encode_utf16().count() + 3) / 4) as i64;
        assert_eq!(estimate_request_tokens(&messages, "😀😀😀", &[]), 4);
        assert_eq!(
            estimate_request_tokens(&messages, "😀😀😀", &tools),
            4 + expected_schema
        );
        assert_eq!(estimate_request_tokens(&[], "", &[]), 0);
    }

    #[test]
    fn request_estimate_does_not_charge_known_overhead_twice() {
        let mut assistant = AssistantMessage::empty(ice_ai::types::Api::Faux, "faux", "faux", 1);
        assistant.stop_reason = ice_ai::types::StopReason::Stop;
        assistant.content = vec![Content::text("done")];
        assistant.usage.input = 90;
        assistant.usage.output = 10;
        let messages = vec![AgentMessage::Assistant(Box::new(assistant)), user("next")];
        // The provider's 100-token measurement already includes its prompt.
        assert_eq!(estimate_request_tokens(&messages, "known prompt", &[]), 101);
        // A much larger live prompt must not be hidden by older usage either.
        assert_eq!(
            estimate_request_tokens(&messages, &"s".repeat(800), &[]),
            202
        );
        assert_eq!(estimate_context_tokens(&messages).tokens, 101);
    }

    #[test]
    fn extract_file_ops_classifies_read_write_edit() {
        // Build a real assistant with three tool calls.
        let mut a =
            ice_ai::types::AssistantMessage::empty(ice_ai::types::Api::Faux, "faux", "faux", 1);
        a.content.push(Content::tool_call(
            "c1",
            "read",
            serde_json::json!({"path": "/a"}),
        ));
        a.content.push(Content::tool_call(
            "c2",
            "write",
            serde_json::json!({"path": "/b"}),
        ));
        a.content.push(Content::tool_call(
            "c3",
            "edit",
            serde_json::json!({"path": "/c"}),
        ));
        a.content.push(Content::tool_call(
            "c4",
            "read",
            serde_json::json!({"path": ""}),
        ));
        let msg = AgentMessage::Assistant(Box::new(a));
        let mut ops = create_file_ops();
        extract_file_ops_from_message(&msg, &mut ops);
        assert!(ops.read.contains("/a"));
        assert!(ops.written.contains("/b"));
        assert!(ops.edited.contains("/c"));
        assert!(!ops.read.contains(""));
        let (read, modified) = compute_file_lists(&ops);
        assert_eq!(read, vec!["/a".to_string()]);
        assert_eq!(modified, vec!["/b".to_string(), "/c".to_string()]);
    }

    #[test]
    fn format_file_operations_empty_and_nonempty() {
        assert_eq!(format_file_operations(&[], &[]), "");
        let s = format_file_operations(&["/a".into(), "/b".into()], &["/c".into()]);
        assert!(s.starts_with("\n\n"));
        assert!(s.contains("<read-files>"));
        assert!(s.contains("/a"));
        assert!(s.contains("<modified-files>"));
        assert!(s.contains("/c"));
    }

    #[test]
    fn serialize_conversation_renders_roles() {
        let msgs = vec![Message::User(UserMessage::new(
            UserContent::Text("hello".into()),
            1,
        ))];
        let s = serialize_conversation(&msgs);
        assert_eq!(s, "[User]: hello");
    }

    #[test]
    fn estimates_utf16_units_for_astral_text_and_tool_arguments() {
        // Two astral characters are four UTF-16 units (one estimated token),
        // despite occupying eight bytes in UTF-8.
        assert_eq!(estimate_tokens(&user("😀😀")), 1);

        let mut assistant =
            ice_ai::types::AssistantMessage::empty(ice_ai::types::Api::Faux, "faux", "faux", 1);
        assistant.content.push(Content::tool_call(
            "call",
            "read",
            serde_json::json!({"value": "😀😀"}),
        ));
        let message = AgentMessage::Assistant(Box::new(assistant));
        let serialized_units = "read".encode_utf16().count()
            + serde_json::to_string(&serde_json::json!({"value": "😀😀"}))
                .unwrap()
                .encode_utf16()
                .count();
        assert_eq!(estimate_tokens(&message), (serialized_units as i64 + 3) / 4);
    }

    #[test]
    fn unknown_custom_roles_count_zero_and_bash_estimate_ignores_full_output_path() {
        let unknown = AgentMessage::Custom(ice_agent::message::CustomMessage::new(
            "applicationRole",
            vec![Content::text("visible but unknown")],
            serde_json::Value::Null,
            1,
        ));
        assert_eq!(estimate_tokens(&unknown), 0);
        let custom = AgentMessage::Custom(ice_agent::message::CustomMessage::new(
            crate::messages::CUSTOM_ROLE,
            vec![Content::text("abcd")],
            serde_json::Value::Null,
            1,
        ));
        assert_eq!(estimate_tokens(&custom), 1);

        let bash = crate::messages::bash_execution_message(
            "cmd",
            "out",
            None,
            false,
            true,
            Some("/a/very/long/full-output.txt".into()),
            None,
            1,
        );
        assert_eq!(estimate_tokens(&bash), 2); // command+output: 6 UTF-16 units.
    }

    #[test]
    fn serialization_truncates_at_utf16_boundary_and_counts_omitted_units() {
        let make_tool_result = |text: String| {
            Message::ToolResult(Box::new(ice_ai::types::ToolResultMessage {
                role: ice_ai::types::ToolResultRole,
                tool_call_id: "call".into(),
                tool_name: "read".into(),
                content: vec![Content::text(text)],
                details: None,
                usage: None,
                added_tool_names: Vec::new(),
                is_error: false,
                timestamp: 1,
            }))
        };
        let text = format!("{}😀z", "a".repeat(1999));
        let serialized = serialize_conversation(&[make_tool_result(text.clone())]);
        assert!(serialized.starts_with(&format!("[Tool result]: {}", "a".repeat(1999))));
        // Rust never emits an isolated surrogate: this boundary backs up before
        // the astral scalar, so it retains 1999 units and counts all 3 omitted.
        assert!(serialized.ends_with("[... 3 more characters truncated]"));
        assert!(!serialized.contains('😀'));

        let bmp = "界".repeat(2001);
        let serialized = serialize_conversation(&[make_tool_result(bmp)]);
        assert!(serialized.ends_with("[... 1 more characters truncated]"));
    }

    #[test]
    fn estimate_context_tokens_no_usage_sums_all() {
        let msgs = vec![user("abcdefgh"), user("abcdefgh")]; // 2 + 2 = 4.
        let est = estimate_context_tokens(&msgs);
        assert_eq!(est.tokens, 4);
        assert_eq!(est.usage_tokens, 0);
        assert!(est.last_usage_index.is_none());
    }

    #[test]
    fn estimate_context_tokens_uses_last_assistant_usage_plus_trailing() {
        let mut a =
            ice_ai::types::AssistantMessage::empty(ice_ai::types::Api::Faux, "faux", "faux", 1);
        a.usage = Usage {
            input: 100,
            output: 10,
            cache_read: 0,
            cache_write: 0,
            cache_write_1h: None,
            reasoning: None,
            total_tokens: 110,
            cost: ice_ai::types::UsageCost::default(),
        };
        a.stop_reason = ice_ai::types::StopReason::Stop;
        let msgs = vec![
            user("abcdefgh"),                     // 2
            AgentMessage::Assistant(Box::new(a)), // usage 110, index 1
            user("abcdefghi"),                    // 3 trailing
        ];
        let est = estimate_context_tokens(&msgs);
        assert_eq!(est.usage_tokens, 110);
        assert_eq!(est.trailing_tokens, 3);
        assert_eq!(est.tokens, 113);
        assert_eq!(est.last_usage_index, Some(1));
    }
}
