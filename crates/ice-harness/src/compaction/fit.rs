//! Fit the latest turn's tool results into the context window after a
//! provider has reported a real context overflow.
//!
//! Overflow recovery cannot summarise the latest user turn (its prompt and
//! tool call/result pairs must stay intact), so a single turn of large tool
//! output (for example many parallel reads) could fail a run outright. This
//! shrinks the text of that turn's tool results for the request only: every
//! call keeps exactly one result, each result keeps its head and tail, and a
//! marker says how much was omitted and how to see it. Persisted session
//! entries are never changed. Callers must use it only after an authoritative
//! overflow, never on a token estimate alone (estimates overcount prose).

use ice_agent::AgentMessage;
use ice_ai::types::{Content, TextContent};

/// Smallest per-result text cap before fitting gives up.
const MIN_RESULT_CHARS: usize = 2_000;
/// Smallest share of a cap kept for any single text block.
const MIN_BLOCK_CHARS: usize = 200;

/// What fitting changed, for diagnostics (no content).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FitReport {
    pub results_shrunk: usize,
    pub chars_omitted: usize,
}

fn omitted_marker(omitted: usize) -> String {
    format!(
        "\n[… {omitted} characters omitted to fit the context window. Re-read the source with offset/limit, or narrow the command, to see this part.]\n"
    )
}

fn text_chars(message: &AgentMessage) -> usize {
    match message {
        AgentMessage::ToolResult(result) => result
            .content
            .iter()
            .map(|block| match block {
                Content::Text(text) => text.text.chars().count(),
                _ => 0,
            })
            .sum(),
        _ => 0,
    }
}

/// Keep `keep` characters split between head and tail, on char boundaries.
fn truncate_middle(text: &str, keep: usize) -> Option<(String, usize)> {
    let total = text.chars().count();
    if total <= keep {
        return None;
    }
    let head = keep / 2;
    let tail = keep - head;
    let head_end = text.char_indices().nth(head).map_or(text.len(), |(i, _)| i);
    let tail_start = text
        .char_indices()
        .nth(total - tail)
        .map_or(text.len(), |(i, _)| i);
    let omitted = total - keep;
    Some((
        format!(
            "{}{}{}",
            &text[..head_end],
            omitted_marker(omitted),
            &text[tail_start..]
        ),
        omitted,
    ))
}

/// Cap one tool result's text at `cap` characters, spread over its text
/// blocks in proportion to their length.
fn cap_result(message: &mut AgentMessage, cap: usize) -> usize {
    let total = text_chars(message);
    if total <= cap {
        return 0;
    }
    let AgentMessage::ToolResult(result) = message else {
        return 0;
    };
    let mut omitted = 0;
    for block in &mut result.content {
        if let Content::Text(TextContent { text, .. }) = block {
            let len = text.chars().count();
            let share = ((cap as u128 * len as u128) / total as u128) as usize;
            if let Some((shrunk, cut)) = truncate_middle(text, share.max(MIN_BLOCK_CHARS)) {
                *text = shrunk;
                omitted += cut;
            }
        }
    }
    omitted
}

fn latest_turn_results(messages: &[AgentMessage]) -> Vec<usize> {
    let start = messages
        .iter()
        .rposition(|message| matches!(message, AgentMessage::User(_)))
        .map_or(0, |index| index + 1);
    (start..messages.len())
        .filter(|&index| matches!(messages[index], AgentMessage::ToolResult(_)))
        .collect()
}

fn apply_cap(messages: &mut [AgentMessage], indices: &[usize], cap: usize) -> FitReport {
    let mut report = FitReport::default();
    for &index in indices {
        let cut = cap_result(&mut messages[index], cap);
        if cut > 0 {
            report.results_shrunk += 1;
            report.chars_omitted += cut;
        }
    }
    report
}

/// Shrink the latest turn's tool results until `fits` accepts the messages.
/// Uses the largest per-result cap that fits (binary search; `fits` must be
/// monotonic in the amount of text). Returns `None`, leaving `messages`
/// untouched, when there is nothing to shrink or even the minimum cap does
/// not fit. Deterministic for the same input.
pub fn fit_latest_turn_tool_results(
    messages: &mut [AgentMessage],
    fits: impl Fn(&[AgentMessage]) -> bool,
) -> Option<FitReport> {
    let indices = latest_turn_results(messages);
    let largest = indices.iter().map(|&i| text_chars(&messages[i])).max()?;
    if largest <= MIN_RESULT_CHARS {
        return None;
    }
    let attempt = |cap: usize| {
        let mut candidate = messages.to_vec();
        let report = apply_cap(&mut candidate, &indices, cap);
        (fits(&candidate), candidate, report)
    };
    if !attempt(MIN_RESULT_CHARS).0 {
        return None;
    }
    let (mut low, mut high) = (MIN_RESULT_CHARS, largest);
    while low < high {
        let mid = low + (high - low).div_ceil(2);
        if attempt(mid).0 {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    let (_, fitted, report) = attempt(low);
    messages.clone_from_slice(&fitted);
    Some(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ice_ai::types::{ToolResultMessage, UserMessage};

    fn result(id: &str, text: String) -> AgentMessage {
        AgentMessage::ToolResult(Box::new(ToolResultMessage {
            role: Default::default(),
            tool_call_id: id.into(),
            tool_name: "read".into(),
            content: vec![Content::text(text)],
            details: None,
            usage: None,
            added_tool_names: Vec::new(),
            is_error: false,
            timestamp: 1,
        }))
    }

    fn total_chars(messages: &[AgentMessage]) -> usize {
        messages.iter().map(text_chars).sum()
    }

    fn ids(messages: &[AgentMessage]) -> Vec<String> {
        messages
            .iter()
            .filter_map(|m| match m {
                AgentMessage::ToolResult(r) => Some(r.tool_call_id.clone()),
                _ => None,
            })
            .collect()
    }

    fn sample() -> Vec<AgentMessage> {
        vec![
            AgentMessage::User(UserMessage::new("older turn", 1)),
            result("old", "o".repeat(50_000)),
            AgentMessage::User(UserMessage::new("read all docs", 2)),
            result("a", "a".repeat(50_000)),
            result("b", "b".repeat(30_000)),
            result("c", "c".repeat(1_000)),
        ]
    }

    #[test]
    fn shrinks_only_the_latest_turn_and_keeps_every_result() {
        let mut messages = sample();
        let before = ids(&messages);
        let report =
            fit_latest_turn_tool_results(&mut messages, |m| total_chars(m) <= 80_000).unwrap();
        assert!(total_chars(&messages) <= 80_000);
        assert_eq!(ids(&messages), before, "no result dropped or reordered");
        assert_eq!(text_chars(&messages[1]), 50_000, "older turn untouched");
        assert_eq!(text_chars(&messages[5]), 1_000, "small result untouched");
        assert_eq!(report.results_shrunk, 2);
        let AgentMessage::ToolResult(a) = &messages[3] else {
            panic!()
        };
        let Content::Text(text) = &a.content[0] else {
            panic!()
        };
        assert!(text.text.starts_with('a') && text.text.ends_with('a'));
        assert!(text
            .text
            .contains("characters omitted to fit the context window"));
    }

    #[test]
    fn uses_the_largest_cap_that_fits_and_is_deterministic() {
        let fits = |m: &[AgentMessage]| total_chars(m) <= 70_000;
        let mut first = sample();
        let mut second = sample();
        fit_latest_turn_tool_results(&mut first, fits).unwrap();
        fit_latest_turn_tool_results(&mut second, fits).unwrap();
        assert_eq!(first, second);
        // One more character per shrunk result would no longer fit.
        assert!(total_chars(&first) > 69_000, "{}", total_chars(&first));
    }

    #[test]
    fn gives_up_untouched_when_even_the_minimum_does_not_fit() {
        let mut messages = sample();
        let original = messages.clone();
        assert!(fit_latest_turn_tool_results(&mut messages, |_| false).is_none());
        assert_eq!(messages, original);
        let mut no_results = vec![AgentMessage::User(UserMessage::new("x".repeat(90_000), 1))];
        assert!(fit_latest_turn_tool_results(&mut no_results, |_| true).is_none());
    }

    #[test]
    fn truncation_respects_multibyte_characters() {
        let mut messages = vec![
            AgentMessage::User(UserMessage::new("q", 1)),
            result("u", "界🙂".repeat(20_000)),
        ];
        fit_latest_turn_tool_results(&mut messages, |m| total_chars(m) <= 10_000).unwrap();
        let AgentMessage::ToolResult(r) = &messages[1] else {
            panic!()
        };
        let Content::Text(text) = &r.content[0] else {
            panic!()
        };
        assert!(text.text.starts_with('界') && text.text.ends_with('🙂'));
    }
}
