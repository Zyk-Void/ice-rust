//! Deterministic filters for automatic recall: drop routine tool-success
//! lines and lines already injected into the active context. Backend scores
//! are ranking signals, not probabilities, so nothing here thresholds on them.

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashSet, VecDeque};
use std::hash::{Hash, Hasher};

use super::client::RecallResult;
use super::suppress::{Reason as SuppressionReason, SuppressionSet};
use super::text::RecallLine;
use crate::blackhole::js::parse_iso_ms;
use crate::blackhole::regex;

/// Bounded FIFO of persisted evidence identities. Selection never commits them.
const MAX_SEEN_LINES: usize = 512;
/// A "successful tool operations" summary is only routine when short.
const ROUTINE_SUMMARY_MAX_CHARS: usize = 200;

/// Content hashes of the lines already placed in the active context.
#[derive(Clone, Debug, Default)]
pub struct SeenEvidence {
    keys: HashSet<u64>,
    order: VecDeque<u64>,
}

impl SeenEvidence {
    /// The context no longer holds earlier injections (compaction, another
    /// session), so they may be injected again.
    pub fn clear(&mut self) {
        self.keys.clear();
        self.order.clear();
    }

    /// True when `key` is new; records it.
    fn insert(&mut self, key: &str) -> bool {
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        let key = hasher.finish();
        if self.keys.contains(&key) {
            return false;
        }
        if self.keys.len() >= MAX_SEEN_LINES {
            if let Some(oldest) = self.order.pop_front() {
                self.keys.remove(&oldest);
            }
        }
        self.order.push_back(key);
        self.keys.insert(key)
    }

    /// Commit only complete facts actually persisted in a context packet.
    pub fn remember_lines(&mut self, lines: &[RecallLine]) {
        for line in lines {
            self.insert(&normalize(&line.text));
        }
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
    pub dataset: Option<String>,
    pub expires_at: Option<String>,
    pub superseded_by: Option<String>,
    pub branch: Option<String>,
    pub revision: Option<String>,
    pub worktree_id: Option<String>,
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
        dataset: field(&["dataset", "dataset_name", "datasetName"]),
        expires_at: field(&["expires_at", "expiresAt", "valid_until", "validUntil"]),
        superseded_by: field(&["superseded_by", "supersededBy"]),
        branch: field(&["branch", "git_branch"]),
        revision: field(&["revision", "git_revision", "commit_sha"]),
        worktree_id: field(&["worktree_id", "worktreeId"]),
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

/// Eligibility context for metadata-driven automatic recall filters.
#[derive(Debug)]
pub struct FilterContext<'a> {
    pub now_ms: u64,
    pub dataset: String,
    pub legacy_dataset: Option<String>,
    pub branch: Option<String>,
    pub revision: Option<String>,
    pub worktree_id: Option<String>,
    pub suppressions: Option<&'a SuppressionSet>,
}

#[derive(Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RejectCounts {
    pub routine: u32,
    pub duplicate: u32,
    pub expired: u32,
    pub superseded: u32,
    pub wrong_scope: u32,
    pub wrong_branch: u32,
    pub unknown_validity: u32,
    pub wrong_revision: u32,
    pub wrong_worktree: u32,
    pub active_context: u32,
    pub irrelevant: u32,
}

#[derive(Debug, Default, PartialEq)]
pub struct Selection {
    pub lines: Vec<RecallLine>,
    pub rejected: RejectCounts,
    pub kept: u32,
}

/// The lines of `results` worth injecting: not routine, not already in
/// context, not repeated within this batch. A result with nothing left is
/// dropped.
pub fn select_new(results: &[RecallResult], seen: &mut SeenEvidence) -> Vec<RecallLine> {
    select_results(results, seen, None, false).lines
}

pub fn select_new_with_context(
    results: &[RecallResult],
    seen: &mut SeenEvidence,
    ctx: &FilterContext<'_>,
) -> Selection {
    select_results(results, seen, Some(ctx), false)
}

fn parse_rfc3339_ms(value: &str) -> Option<i64> {
    let captures = regex!(r"^([0-9]{4})-([0-9]{2})-([0-9]{2})T([0-9]{2}):([0-9]{2}):([0-9]{2})(?:\.[0-9]+)?(?:Z|[+-][0-9]{2}:[0-9]{2})$")
        .captures(value)?;
    let number = |index| captures.get(index)?.as_str().parse::<u32>().ok();
    let (year, month, day) = (number(1)?, number(2)?, number(3)?);
    let (hour, minute, second) = (number(4)?, number(5)?, number(6)?);
    let month_days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 400 == 0 || (year % 4 == 0 && year % 100 != 0) => 29,
        2 => 28,
        _ => return None,
    };
    if day == 0 || day > month_days || hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    parse_iso_ms(value)
}

/// Apply validity and novelty to whole source records, preserving conditions
/// spanning multiple lines. This is the selective-delivery path.
pub fn select_atomic_with_context(
    results: &[RecallResult],
    seen: &mut SeenEvidence,
    context: &FilterContext<'_>,
) -> Selection {
    select_results(results, seen, Some(context), true)
}

fn select_results(
    results: &[RecallResult],
    seen: &mut SeenEvidence,
    context: Option<&FilterContext<'_>>,
    atomic: bool,
) -> Selection {
    let mut selection = Selection::default();
    for result in results {
        let provenance = provenance(result);
        let line_count = if atomic {
            u32::from(!result.text.trim().is_empty())
        } else {
            result
                .text
                .lines()
                .filter(|line| !line.trim().is_empty())
                .count() as u32
        };
        if let Some(ctx) = context {
            if let Some(expires_at) = provenance.expires_at.as_deref() {
                match parse_rfc3339_ms(expires_at) {
                    Some(expires_at) if expires_at <= ctx.now_ms.min(i64::MAX as u64) as i64 => {
                        selection.rejected.expired += line_count;
                        continue;
                    }
                    None => {
                        selection.rejected.unknown_validity += line_count;
                        continue;
                    }
                    _ => {}
                }
            }
            if provenance.superseded_by.is_some() {
                selection.rejected.superseded += line_count;
                continue;
            }
            if provenance.dataset.as_deref().is_some_and(|dataset| {
                dataset != ctx.dataset && ctx.legacy_dataset.as_deref() != Some(dataset)
            }) {
                selection.rejected.wrong_scope += line_count;
                continue;
            }
            if provenance
                .branch
                .as_deref()
                .is_some_and(|branch| ctx.branch.as_deref() != Some(branch))
            {
                selection.rejected.wrong_branch += line_count;
                continue;
            }
            if provenance
                .revision
                .as_deref()
                .is_some_and(|revision| ctx.revision.as_deref() != Some(revision))
            {
                selection.rejected.wrong_revision += line_count;
                continue;
            }
            if provenance
                .worktree_id
                .as_deref()
                .is_some_and(|worktree| ctx.worktree_id.as_deref() != Some(worktree))
            {
                selection.rejected.wrong_worktree += line_count;
                continue;
            }
            if let Some(metadata) = result.metadata.as_ref() {
                // Explicitly malformed restrictions must not become unscoped
                // facts merely because a string accessor returned None.
                let malformed = [
                    "branch",
                    "git_branch",
                    "revision",
                    "git_revision",
                    "commit_sha",
                    "worktree_id",
                    "worktreeId",
                    "dataset",
                    "dataset_name",
                    "datasetName",
                ]
                .iter()
                .any(|key| {
                    metadata.get(*key).is_some_and(|value| {
                        !value.is_null() && value.as_str().is_none_or(|text| text.trim().is_empty())
                    })
                });
                if malformed {
                    selection.rejected.unknown_validity += line_count;
                    continue;
                }
                if let Some(expiry) = metadata
                    .get("valid_until_ms")
                    .or_else(|| metadata.get("expires_at_ms"))
                {
                    if !expiry.is_null() {
                        match expiry.as_u64() {
                            Some(expiry) if expiry > ctx.now_ms => {}
                            Some(_) => {
                                selection.rejected.expired += line_count;
                                continue;
                            }
                            None => {
                                selection.rejected.unknown_validity += line_count;
                                continue;
                            }
                        }
                    }
                }
            }
        }
        let mut kept = Vec::new();
        let facts: Vec<&str> = if atomic {
            vec![result.text.as_str()]
        } else {
            result.text.lines().collect()
        };
        for line in facts.into_iter().filter(|line| !line.trim().is_empty()) {
            if let Some(reason) = context.and_then(|ctx| {
                ctx.suppressions
                    .and_then(|set| set.is_suppressed(line, ctx.now_ms, ctx.branch.as_deref()))
            }) {
                match reason {
                    SuppressionReason::Superseded => selection.rejected.superseded += 1,
                    SuppressionReason::Expired => selection.rejected.expired += 1,
                }
            } else if is_routine_success_line(line) {
                selection.rejected.routine += 1;
            } else if !seen.insert(&normalize(line)) {
                selection.rejected.duplicate += 1;
            } else {
                kept.push(line);
            }
        }
        selection.kept += kept.len() as u32;
        if kept.is_empty() {
            continue;
        }
        selection.lines.push(RecallLine {
            text: kept.join("\n"),
            score: result.score,
            provenance: provenance_tag(result),
        });
    }
    selection
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
        for key in ["dataset", "dataset_name", "datasetName"] {
            let mut hit = result("fact");
            hit.metadata = Some(serde_json::json!({key: "project"}));
            assert_eq!(provenance(&hit).dataset.as_deref(), Some("project"));
        }
        for key in ["expires_at", "expiresAt", "valid_until", "validUntil"] {
            let mut hit = result("fact");
            hit.metadata = Some(serde_json::json!({key: "2030-01-01T00:00:00Z"}));
            assert_eq!(
                provenance(&hit).expires_at.as_deref(),
                Some("2030-01-01T00:00:00Z")
            );
        }
        for key in ["superseded_by", "supersededBy"] {
            let mut hit = result("fact");
            hit.metadata = Some(serde_json::json!({key: "new-entry"}));
            assert_eq!(provenance(&hit).superseded_by.as_deref(), Some("new-entry"));
        }
        for key in ["branch", "git_branch"] {
            let mut hit = result("fact");
            hit.metadata = Some(serde_json::json!({key: "main"}));
            assert_eq!(provenance(&hit).branch.as_deref(), Some("main"));
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
    fn local_suppressions_filter_lines_without_changing_legacy_selection() {
        use super::super::suppress::{content_hash, Suppression, SuppressionKind, SuppressionSet};

        let mut suppressions = SuppressionSet::default();
        suppressions.insert(Suppression {
            id: uuid::Uuid::new_v4().to_string(),
            hash: content_hash("old decision"),
            kind: SuppressionKind::Superseded {
                by: "new-id".into(),
            },
            dataset: "current".into(),
            branch: None,
            recorded_at_ms: 1,
        });
        suppressions.insert(Suppression {
            id: uuid::Uuid::new_v4().to_string(),
            hash: content_hash("temporary state"),
            kind: SuppressionKind::Expires { at_ms: 100 },
            dataset: "current".into(),
            branch: None,
            recorded_at_ms: 1,
        });
        let context = FilterContext {
            now_ms: 100,
            dataset: "current".into(),
            legacy_dataset: None,
            branch: None,
            revision: None,
            worktree_id: None,
            suppressions: Some(&suppressions),
        };
        let results = [result(
            "[decision] Old decision\n- Temporary   state\nKeep this decision",
        )];
        let selection = select_new_with_context(&results, &mut SeenEvidence::default(), &context);
        assert_eq!(selection.lines.len(), 1);
        assert_eq!(selection.lines[0].text, "Keep this decision");
        assert_eq!(selection.rejected.superseded, 1);
        assert_eq!(selection.rejected.expired, 1);

        let legacy = select_new(&results, &mut SeenEvidence::default());
        assert_eq!(legacy[0].text, results[0].text);
    }

    #[test]
    fn contextual_filters_count_metadata_and_line_rejections() {
        let mut seen = SeenEvidence::default();
        let context = FilterContext {
            now_ms: 1_800_000_000_000,
            dataset: "current".into(),
            legacy_dataset: Some("legacy".into()),
            branch: Some("main".into()),
            revision: None,
            worktree_id: None,
            suppressions: None,
        };
        let with_metadata = |text, metadata| {
            let mut hit = result(text);
            hit.metadata = Some(metadata);
            hit
        };
        seen.insert(&normalize("duplicate"));
        let results = vec![
            with_metadata(
                "expired",
                serde_json::json!({"expiresAt":"2020-01-01T00:00:00Z"}),
            ),
            with_metadata(
                "superseded",
                serde_json::json!({"supersededBy":"replacement"}),
            ),
            with_metadata("wrong scope", serde_json::json!({"dataset":"other"})),
            with_metadata("wrong branch", serde_json::json!({"branch":"release"})),
            with_metadata(
                "current scope",
                serde_json::json!({"datasetName":"current"}),
            ),
            with_metadata("legacy scope", serde_json::json!({"dataset_name":"legacy"})),
            with_metadata(
                "unknown expiry",
                serde_json::json!({"validUntil":"not a timestamp"}),
            ),
            result("bash succeeded\nnew fact"),
            result("duplicate"),
        ];
        let selection = select_new_with_context(&results, &mut seen, &context);
        assert_eq!(selection.lines.len(), 3);
        assert_eq!(selection.kept, 3);
        assert_eq!(selection.rejected.expired, 1);
        assert_eq!(selection.rejected.superseded, 1);
        assert_eq!(selection.rejected.wrong_scope, 1);
        assert_eq!(selection.rejected.wrong_branch, 1);
        assert_eq!(selection.rejected.unknown_validity, 1);
        assert_eq!(selection.rejected.routine, 1);
        assert_eq!(selection.rejected.duplicate, 1);
        assert_eq!(
            selection.kept
                + selection.rejected.routine
                + selection.rejected.duplicate
                + selection.rejected.expired
                + selection.rejected.superseded
                + selection.rejected.wrong_scope
                + selection.rejected.wrong_branch
                + selection.rejected.unknown_validity,
            results
                .iter()
                .map(|result| result.text.lines().count() as u32)
                .sum::<u32>()
        );

        let no_branch = FilterContext {
            branch: None,
            ..context
        };
        let branch_result = with_metadata(
            "branch restriction requires known current branch",
            serde_json::json!({"branch":"other"}),
        );
        assert_eq!(
            select_new_with_context(&[branch_result], &mut SeenEvidence::default(), &no_branch)
                .lines
                .len(),
            0
        );
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
