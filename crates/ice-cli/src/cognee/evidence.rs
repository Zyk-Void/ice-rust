//! Deterministic filters for automatic recall: drop routine tool-success
//! lines and lines already injected into the active context. Backend scores
//! are ranking signals, not probabilities, so nothing here thresholds on them.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashSet;
use std::hash::{Hash, Hasher};

use super::client::RecallResult;
use super::text::RecallLine;
use crate::blackhole::regex;

/// Lines remembered per context generation; the set resets when full, which
/// at worst re-injects a line.
const MAX_SEEN_LINES: usize = 512;
/// A "successful tool operations" summary is only routine when short.
const ROUTINE_SUMMARY_MAX_CHARS: usize = 200;

/// Content hashes of the lines already placed in the active context.
#[derive(Debug, Default)]
pub struct SeenEvidence(HashSet<u64>);

impl SeenEvidence {
    /// The context no longer holds earlier injections (compaction, another
    /// session), so they may be injected again.
    pub fn clear(&mut self) {
        self.0.clear();
    }

    /// True when `key` is new; records it.
    fn insert(&mut self, key: &str) -> bool {
        if self.0.len() >= MAX_SEEN_LINES {
            self.0.clear();
        }
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        self.0.insert(hasher.finish())
    }
}

fn normalize(line: &str) -> String {
    let line = line.trim();
    let line = line.strip_prefix("- ").unwrap_or(line);
    line.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// `bash succeeded`, `read succeeded` and "thirty-one successful tool
/// operations": traces of what ran, not facts worth remembering.
pub fn is_routine_success_line(line: &str) -> bool {
    let line = normalize(line);
    regex!(r"^(?:bash|read|write|edit|grep|find|ls|glob|agent) succeeded\W*$").is_match(&line)
        || (line.chars().count() <= ROUTINE_SUMMARY_MAX_CHARS
            && regex!(r"\bsuccessful tool (?:operations?|calls?|executions?)\b").is_match(&line))
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Provenance {
    pub source_id: Option<String>,
    pub session_id: Option<String>,
    pub created_at: Option<String>,
}

pub fn provenance(result: &RecallResult) -> Provenance {
    let field = |keys: &[&str]| {
        result.metadata.as_ref().and_then(|metadata| {
            keys.iter().find_map(|key| {
                metadata
                    .get(*key)
                    .and_then(|value| value.as_str())
                    .map(str::to_owned)
            })
        })
    };
    Provenance {
        source_id: field(&["id", "source_id", "entry_id"]),
        session_id: field(&["session_id", "sessionId"]),
        created_at: field(&["created_at", "createdAt", "timestamp"]),
    }
}

pub fn provenance_tag(result: &RecallResult) -> Option<String> {
    let provenance = provenance(result);
    let parts: Vec<String> = [
        ("source", provenance.source_id),
        ("session", provenance.session_id),
        ("at", provenance.created_at),
    ]
    .into_iter()
    .filter_map(|(key, value)| {
        value.map(|value| format!("{key}={}", value.chars().take(64).collect::<String>()))
    })
    .collect();
    (!parts.is_empty()).then(|| parts.join(" "))
}

/// The lines of `results` worth injecting: not routine, not already in
/// context, not repeated within this batch. A result with nothing left is
/// dropped.
pub fn select_new(results: &[RecallResult], seen: &mut SeenEvidence) -> Vec<RecallLine> {
    results
        .iter()
        .filter_map(|result| {
            let kept: Vec<&str> = result
                .text
                .lines()
                .filter(|line| !line.trim().is_empty())
                .filter(|line| !is_routine_success_line(line))
                .filter(|line| seen.insert(&normalize(line)))
                .collect();
            (!kept.is_empty()).then(|| RecallLine {
                text: kept.join("\n"),
                score: result.score,
                provenance: provenance_tag(result),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(text: &str) -> RecallResult {
        RecallResult {
            text: text.into(),
            score: Some(0.5),
            metadata: None,
        }
    }

    #[test]
    fn provenance_reads_only_known_string_fields() {
        for key in ["id", "source_id", "entry_id"] {
            let mut hit = result("fact");
            hit.metadata = Some(serde_json::json!({key: "source"}));
            assert_eq!(provenance(&hit).source_id.as_deref(), Some("source"));
        }
        for key in ["session_id", "sessionId"] {
            let mut hit = result("fact");
            hit.metadata = Some(serde_json::json!({key: "session"}));
            assert_eq!(provenance(&hit).session_id.as_deref(), Some("session"));
        }
        for key in ["created_at", "createdAt", "timestamp"] {
            let mut hit = result("fact");
            hit.metadata = Some(serde_json::json!({key: "time"}));
            assert_eq!(provenance(&hit).created_at.as_deref(), Some("time"));
        }
        let mut hit = result("fact");
        assert_eq!(provenance(&hit), Provenance::default());
        hit.metadata = Some(
            serde_json::json!({"id": 1, "sessionId": false, "timestamp": null, "other": "value"}),
        );
        assert_eq!(provenance(&hit), Provenance::default());
        assert_eq!(provenance_tag(&hit), None);
    }

    #[test]
    fn routine_tool_success_lines_are_dropped() {
        for line in [
            "bash succeeded",
            "- Read succeeded.",
            "thirty-one successful tool operations were run in this session",
        ] {
            assert!(is_routine_success_line(line), "{line}");
        }
        for line in [
            "Decision: use LRU eviction for the cache",
            "The migration succeeded after pinning serde",
            "bash succeeded in reproducing the failing test under Rust 1.88 only",
        ] {
            assert!(!is_routine_success_line(line), "{line}");
        }
    }

    #[test]
    fn only_new_lines_survive_and_clearing_allows_reinjection() {
        let mut seen = SeenEvidence::default();
        let first = select_new(
            &[
                result("bash succeeded\n- Use LRU eviction\nbash succeeded"),
                result("bash succeeded"),
            ],
            &mut seen,
        );
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].text, "- Use LRU eviction");
        // The same fact under a different query adds nothing.
        let again = select_new(&[result("  - use  LRU eviction ")], &mut seen);
        assert!(again.is_empty());
        let mixed = select_new(
            &[result("- Use LRU eviction\n- TTL is 5 minutes")],
            &mut seen,
        );
        assert_eq!(mixed[0].text, "- TTL is 5 minutes");
        seen.clear();
        assert_eq!(
            select_new(&[result("- Use LRU eviction")], &mut seen).len(),
            1
        );
    }
}
