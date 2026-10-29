//! Local admission for automatic prompt recall. Pure: no I/O, so an ordinary
//! turn costs a string scan and never a Cognee or model request.
//!
//! `legacy` keeps the original rule (any prompt with enough words searches).
//! `event` searches only when the prompt itself asks about earlier work; the
//! model's `cognee_search` tool and `/cognee search` stay available for
//! everything else, so a missed admission costs one tool call, not a lost
//! memory.

use super::config::ReadPolicy;
use super::runtime::is_low_signal_recall_prompt;

/// Phrases that reference earlier work or earlier sessions. Deliberately
/// phrase-level: bare "continue", "again" or "previous" occur in ordinary
/// coding requests and must not trigger a search.
const HISTORY_PHRASES: [&str; 31] = [
    "last time",
    "last session",
    "earlier session",
    "previous session",
    "previous decision",
    "previous discussion",
    "earlier decision",
    "what did we",
    "did we ",
    "have we ",
    "we decided",
    "we discussed",
    "we agreed",
    "we chose",
    "we settled",
    "i told you",
    "i said earlier",
    "you suggested",
    "remember when",
    "do you remember",
    "our decision",
    "our earlier",
    "from before",
    "as before",
    "same setup",
    "same approach",
    "where did we leave",
    "where were we",
    "pick up where",
    "continue that",
    "resume that",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecallReason {
    /// `legacy`: any prompt with enough words.
    Legacy,
    /// The prompt asks about earlier work.
    ExplicitHistory,
    /// Recall starts speculatively; the typed gate decides whether it is used.
    Judged,
}

impl RecallReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Legacy => "legacy",
            Self::ExplicitHistory => "explicit_history",
            Self::Judged => "judged",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// `legacy`: fewer than four words.
    ShortPrompt,
    /// `on_demand`: automatic recall is off by policy.
    OnDemand,
    /// `event`: the prompt does not ask about earlier work.
    NoHistoryRequest,
}

impl SkipReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ShortPrompt => "short_prompt",
            Self::OnDemand => "on_demand",
            Self::NoHistoryRequest => "no_history_request",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission {
    Recall(RecallReason),
    Skip(SkipReason),
}

pub fn asks_for_history(prompt: &str) -> bool {
    let lower = prompt.to_lowercase();
    HISTORY_PHRASES.iter().any(|phrase| {
        lower.match_indices(phrase).any(|(start, _)| {
            let prefix = &lower[..start];
            let mut words = prefix.split_whitespace().rev();
            let previous = words
                .next()
                .unwrap_or("")
                .trim_start_matches(|c: char| !c.is_alphanumeric());
            let previous = if matches!(previous, "the" | "any") {
                words
                    .next()
                    .unwrap_or("")
                    .trim_start_matches(|c: char| !c.is_alphanumeric())
            } else {
                previous
            };
            !(matches!(previous, "no" | "not" | "never" | "without" | "dont")
                || previous.ends_with("n't")
                || previous.ends_with("n\u{2019}t"))
        })
    })
}

pub fn admit_prompt(policy: ReadPolicy, prompt: &str) -> Admission {
    match policy {
        ReadPolicy::Legacy if is_low_signal_recall_prompt(prompt) => {
            Admission::Skip(SkipReason::ShortPrompt)
        }
        ReadPolicy::Legacy => Admission::Recall(RecallReason::Legacy),
        ReadPolicy::OnDemand => Admission::Skip(SkipReason::OnDemand),
        ReadPolicy::Event if asks_for_history(prompt) => {
            Admission::Recall(RecallReason::ExplicitHistory)
        }
        ReadPolicy::Event => Admission::Skip(SkipReason::NoHistoryRequest),
        ReadPolicy::Judged => Admission::Recall(RecallReason::Judged),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_keeps_the_word_count_rule() {
        assert_eq!(
            admit_prompt(ReadPolicy::Legacy, "fix it"),
            Admission::Skip(SkipReason::ShortPrompt)
        );
        assert_eq!(
            admit_prompt(ReadPolicy::Legacy, "How does the cache evict entries?"),
            Admission::Recall(RecallReason::Legacy)
        );
    }

    #[test]
    fn on_demand_never_searches_automatically() {
        assert_eq!(
            admit_prompt(ReadPolicy::OnDemand, "What did we decide about the cache?"),
            Admission::Skip(SkipReason::OnDemand)
        );
    }

    #[test]
    fn negated_history_phrases_do_not_trigger_automatic_recall() {
        for prompt in [
            "Implement a standalone component with no earlier decisions",
            "Proceed without the previous decision",
            "Do not remember when the cache changed",
            "Don't remember when we changed this",
            "Please don\u{2019}t remember when it changed",
            "No previous decision applies. What did we decide about caching?",
        ] {
            let expected = if prompt.contains("What did we") {
                Admission::Recall(RecallReason::ExplicitHistory)
            } else {
                Admission::Skip(SkipReason::NoHistoryRequest)
            };
            assert_eq!(
                admit_prompt(ReadPolicy::Event, prompt),
                expected,
                "{prompt}"
            );
        }
    }

    #[test]
    fn event_searches_only_for_history_requests() {
        for prompt in [
            "What did we decide about the cache?",
            "Previous decision?",
            "Same setup?",
            "Continue that migration.",
            "Where were we on the parser?",
            "Last time you suggested a different approach",
        ] {
            assert_eq!(
                admit_prompt(ReadPolicy::Event, prompt),
                Admission::Recall(RecallReason::ExplicitHistory),
                "{prompt}"
            );
        }
        // Ordinary turns, including long ones and bare continuations, send
        // nothing.
        for prompt in [
            "Build the release binary and run the whole test suite",
            "How many commits are on this branch?",
            "continue",
            "Do it again with the previous element of the list",
            "ok thanks",
        ] {
            assert_eq!(
                admit_prompt(ReadPolicy::Event, prompt),
                Admission::Skip(SkipReason::NoHistoryRequest),
                "{prompt}"
            );
        }
    }
}
