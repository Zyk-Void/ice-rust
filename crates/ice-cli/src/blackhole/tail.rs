//! Retained-tail selection. Ports `core/tail.ts` (BLACK-031).
//!
//! TypeScript sessions keep a tail by naming a cut entry and, in minimal mode,
//! name a sentinel id that matches no entry. Rust compaction entries store the
//! retained tail itself, so minimal mode is expressed as an empty
//! `retained_tail`; durable history is never deleted.

use ice_agent::AgentMessage;
use ice_harness::messages::custom_data;

use super::config::TailBehavior;

/// Custom message types that are recall echoes, not conversation.
pub const RECALL_CUSTOM_TYPES: [&str; 1] = ["ice-cognee-recall"];

/// The resolved tail (`BlackholeTailPlan`).
#[derive(Debug, Clone, PartialEq)]
pub struct BlackholeTailPlan {
    /// `None` keeps the host's retained tail; `Some(vec![])` keeps no old tail.
    pub retained_tail: Option<Vec<AgentMessage>>,
    pub source_messages: Vec<AgentMessage>,
    pub dropped_recall_count: usize,
    pub summarized_kept_tail: usize,
}

/// Recall echoes and hidden custom messages read as user goals when
/// summarized, so they are dropped from the compile source.
pub fn is_recall_noise(message: &AgentMessage) -> bool {
    match custom_data(message) {
        Some(data) => RECALL_CUSTOM_TYPES.contains(&data.custom_type.as_str()) || !data.display,
        None => false,
    }
}

pub fn filter_compact_source_messages(messages: Vec<AgentMessage>) -> (Vec<AgentMessage>, usize) {
    let total = messages.len();
    let kept: Vec<AgentMessage> = messages
        .into_iter()
        .filter(|message| !is_recall_noise(message))
        .collect();
    let dropped = total - kept.len();
    (kept, dropped)
}

/// Decide which session tail survives compaction. `ice-default` keeps the
/// host cut; `minimal` summarizes the retained tail too and keeps none of it.
pub fn resolve_blackhole_tail(
    tail_behavior: TailBehavior,
    messages_to_summarize: &[AgentMessage],
    turn_prefix_messages: &[AgentMessage],
    retained_tail: &[AgentMessage],
) -> BlackholeTailPlan {
    let mut prepared: Vec<AgentMessage> = messages_to_summarize.to_vec();
    prepared.extend_from_slice(turn_prefix_messages);
    if tail_behavior != TailBehavior::Minimal {
        let (source_messages, dropped_recall_count) = filter_compact_source_messages(prepared);
        return BlackholeTailPlan {
            retained_tail: None,
            source_messages,
            dropped_recall_count,
            summarized_kept_tail: 0,
        };
    }
    prepared.extend_from_slice(retained_tail);
    let (source_messages, dropped_recall_count) = filter_compact_source_messages(prepared);
    BlackholeTailPlan {
        retained_tail: Some(Vec::new()),
        source_messages,
        dropped_recall_count,
        summarized_kept_tail: retained_tail.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ice_ai::types::{UserContent, UserMessage};
    use ice_harness::messages::create_custom_message;

    fn user(text: &str) -> AgentMessage {
        AgentMessage::User(UserMessage::new(text.to_string(), 0))
    }

    #[test]
    fn default_tail_keeps_host_cut_and_drops_hidden_custom_messages() {
        let hidden = create_custom_message("agents", UserContent::Text("x".into()), false, None, 0);
        let recall = create_custom_message(
            "ice-cognee-recall",
            UserContent::Text("m".into()),
            true,
            None,
            0,
        );
        let visible = create_custom_message("note", UserContent::Text("n".into()), true, None, 0);
        let plan = resolve_blackhole_tail(
            TailBehavior::IceDefault,
            &[user("a"), hidden, recall, visible.clone()],
            &[],
            &[user("tail")],
        );
        assert_eq!(plan.retained_tail, None);
        assert_eq!(plan.source_messages, vec![user("a"), visible]);
        assert_eq!(plan.dropped_recall_count, 2);
        assert_eq!(plan.summarized_kept_tail, 0);
    }

    #[test]
    fn minimal_tail_summarizes_the_retained_messages() {
        let plan = resolve_blackhole_tail(
            TailBehavior::Minimal,
            &[user("a")],
            &[user("prefix")],
            &[user("t1"), user("t2")],
        );
        assert_eq!(plan.retained_tail, Some(Vec::new()));
        assert_eq!(
            plan.source_messages,
            vec![user("a"), user("prefix"), user("t1"), user("t2")]
        );
        assert_eq!(plan.summarized_kept_tail, 2);
    }
}
