//! Context-only trimming of tool results kept from before the latest
//! compaction. Ports `hooks/trim-kept-tail.ts` (BLACK-040). Only the model
//! context changes; session entries keep the full output.

use ice_agent::AgentMessage;
use ice_ai::types::Content;
use ice_harness::session::types::Entry;

use super::js::{last16, len16, prefix16};

pub const KEPT_TOOL_RESULT_MAX_CHARS: usize = 6000;
const HEAD_CHARS: usize = 3000;
const TAIL_CHARS: usize = 1500;

/// Timestamp of the newest compaction entry on the branch.
pub fn latest_compaction_time(entries: &[Entry]) -> Option<i64> {
    entries
        .iter()
        .filter_map(|entry| match entry {
            Entry::Compaction(compaction) => Some((compaction.base.seq, compaction.base.timestamp)),
            _ => None,
        })
        .max_by_key(|(seq, _)| *seq)
        .map(|(_, timestamp)| timestamp)
}

fn trim_text(text: &str, budget: usize, marker: &str) -> String {
    let marker_text = prefix16(marker, budget);
    let available = budget.saturating_sub(len16(marker_text));
    let head = HEAD_CHARS.min(available);
    let tail = TAIL_CHARS.min(available - head);
    let tail_text = if tail > 0 { last16(text, tail) } else { "" };
    format!("{}{marker_text}{tail_text}", prefix16(text, head))
}

/// Cap oversized text in tool results created before `compacted_at`.
/// Returns `None` when nothing changed. Image blocks are kept as they are.
pub fn trim_kept_tool_results(
    messages: &[AgentMessage],
    compacted_at: i64,
) -> Option<Vec<AgentMessage>> {
    let mut changed = false;
    let next = messages
        .iter()
        .map(|message| {
            let AgentMessage::ToolResult(result) = message else {
                return message.clone();
            };
            if result.timestamp >= compacted_at {
                return message.clone();
            }
            let total: usize = result
                .content
                .iter()
                .map(|block| match block {
                    Content::Text(text) => len16(&text.text),
                    _ => 0,
                })
                .sum();
            if total <= KEPT_TOOL_RESULT_MAX_CHARS {
                return message.clone();
            }
            let full_output_path = result
                .details
                .as_ref()
                .and_then(|details| details.get("fullOutputPath"))
                .and_then(|path| path.as_str())
                .map(|path| prefix16(path, 512).to_string());
            let marker = format!(
                "\n\n[pre-compaction tool output trimmed in model context; the session keeps the full output. Do not re-run the tool just to see it.{}]\n\n",
                full_output_path
                    .map(|path| format!(" Full output: {path}"))
                    .unwrap_or_default()
            );
            let mut remaining = KEPT_TOOL_RESULT_MAX_CHARS as i64;
            let mut notice = marker;
            changed = true;
            let mut trimmed = result.as_ref().clone();
            trimmed.content = result
                .content
                .iter()
                .map(|block| {
                    let Content::Text(text) = block else {
                        return block.clone();
                    };
                    let length = len16(&text.text) as i64;
                    if length <= remaining - len16(&notice) as i64 {
                        remaining -= length;
                        return block.clone();
                    }
                    let new_text = trim_text(&text.text, remaining.max(0) as usize, &notice);
                    remaining -= len16(&new_text) as i64;
                    notice.clear();
                    let mut text = text.clone();
                    text.text = new_text;
                    Content::Text(text)
                })
                .collect();
            AgentMessage::ToolResult(Box::new(trimmed))
        })
        .collect();
    changed.then_some(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ice_ai::types::{TextContent, TextContentType, ToolResultMessage};
    use ice_harness::session::types::{CompactionEntry, EntryBase};
    use serde_json::json;

    fn result(texts: &[String], timestamp: i64) -> AgentMessage {
        AgentMessage::ToolResult(Box::new(ToolResultMessage {
            role: Default::default(),
            tool_call_id: "c".into(),
            tool_name: "bash".into(),
            content: texts
                .iter()
                .map(|text| {
                    Content::Text(TextContent {
                        kind: TextContentType,
                        text: text.clone(),
                        text_signature: None,
                    })
                })
                .collect(),
            details: Some(json!({"fullOutputPath": "/tmp/out.log"})),
            usage: None,
            added_tool_names: Vec::new(),
            is_error: false,
            timestamp,
        }))
    }

    fn texts(message: &AgentMessage) -> Vec<String> {
        let AgentMessage::ToolResult(result) = message else {
            panic!()
        };
        result
            .content
            .iter()
            .map(|block| match block {
                Content::Text(text) => text.text.clone(),
                _ => String::new(),
            })
            .collect()
    }

    #[test]
    fn small_or_newer_results_are_unchanged() {
        let messages = vec![
            result(&["x".repeat(6000)], 1),
            result(&["y".repeat(9000)], 10),
        ];
        assert_eq!(trim_kept_tool_results(&messages, 10), None);
    }

    #[test]
    fn oversized_old_results_keep_head_marker_and_tail() {
        let text = format!("{}{}", "h".repeat(5000), "t".repeat(5000));
        let messages = vec![result(&[text, "second block".into()], 1)];
        let out = trim_kept_tool_results(&messages, 5).unwrap();
        let blocks = texts(&out[0]);
        assert!(blocks[0].starts_with(&"h".repeat(3000)));
        assert!(blocks[0].contains("Full output: /tmp/out.log]"));
        assert!(blocks[0].ends_with(&"t".repeat(1500)));
        // The second block fits in what is left of the shared budget.
        assert_eq!(blocks[1], "second block");
    }

    #[test]
    fn latest_compaction_time_reads_the_newest_entry() {
        assert_eq!(latest_compaction_time(&[]), None);
        let compaction = |seq, timestamp| {
            Entry::Compaction(CompactionEntry {
                base: EntryBase {
                    entry_type: "compaction".into(),
                    id: format!("c{seq}"),
                    seq,
                    parent_id: None,
                    timestamp,
                },
                summary: String::new(),
                retained_tail: Vec::new(),
                tokens_before: 0,
                details: None,
                usage: None,
            })
        };
        assert_eq!(
            latest_compaction_time(&[compaction(2, 200), compaction(3, 50), compaction(1, 100),]),
            Some(50)
        );
    }
}
