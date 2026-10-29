//! Bounded, ownership-fenced resident evidence. A cache read does not acknowledge
//! delivery; only a successful host persistence callback commits novelty.

use std::collections::HashMap;

use super::{recall_with_fallback, wall_ms, CogneeRuntime, ObserveFields, AUTO_RECALL_SCOPE};
use crate::cognee::client::{RecallOptions, RecallResult};
use crate::cognee::config::ReadPolicy;
use crate::cognee::evidence::{select_new, SeenEvidence};
use crate::cognee::text::{redact_memory_text, RecallLine};
use serde_json::json;
use tokio_util::sync::CancellationToken;

const MAX_ENTRIES: usize = 32;
const MAX_IN_FLIGHT: usize = 2;
const MAX_RECORDS_PER_ENTRY: usize = 32;
const MAX_ENTRY_CHARS: usize = 32_000;
const MAX_METADATA_BYTES: usize = 4096;
const POSITIVE_TTL_MS: u64 = 300_000;
const NEGATIVE_TTL_MS: u64 = 60_000;
type Key = (String, String, u64);
/// Longest normalized query that keeps a distinct cache key.
const KEY_QUERY_CHARS: usize = 400;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::cognee) struct OperationContext {
    pub(in crate::cognee) host_session_id: Option<String>,
    pub(in crate::cognee) session_id: Option<String>,
    pub(in crate::cognee) dataset: String,
    pub(in crate::cognee) generation: u64,
}

struct Entry {
    results: Vec<RecallResult>,
    fetched_at_ms: u64,
    sequence: u64,
}

#[derive(Default)]
pub(super) struct EvidenceCache {
    entries: HashMap<Key, Entry>,
    in_flight: HashMap<Key, u64>,
    sequence: u64,
    cancel: CancellationToken,
}

impl EvidenceCache {
    pub(super) fn key(dataset: &str, query: &str, generation: u64) -> Key {
        // Preserve code/path identifier case; only whitespace is normalized.
        (
            dataset.into(),
            query
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(KEY_QUERY_CHARS)
                .collect(),
            generation,
        )
    }

    /// A longer query shares its key with every query that has the same
    /// prefix, so its results must not be cached for another request.
    pub(super) fn keeps_distinct_key(query: &str) -> bool {
        query
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .count()
            <= KEY_QUERY_CHARS
    }

    fn fresh(entry: &Entry, now: u64) -> bool {
        now.saturating_sub(entry.fetched_at_ms)
            < if entry.results.is_empty() {
                NEGATIVE_TTL_MS
            } else {
                POSITIVE_TTL_MS
            }
    }

    pub(super) fn has_fresh(&self, key: &Key, now: u64) -> bool {
        self.entries
            .get(key)
            .is_some_and(|entry| Self::fresh(entry, now))
    }

    pub(super) fn schedule(&mut self, key: &Key, now: u64) -> bool {
        self.entries.retain(|_, entry| Self::fresh(entry, now));
        if self.in_flight.contains_key(key)
            || self.in_flight.len() >= MAX_IN_FLIGHT
            || self.has_fresh(key, now)
        {
            return false;
        }
        self.sequence = self.sequence.wrapping_add(1);
        self.in_flight.insert(key.clone(), self.sequence);
        true
    }

    fn ticket(&self, key: &Key) -> Option<u64> {
        self.in_flight.get(key).copied()
    }

    fn abort_flight(&mut self, key: &Key, ticket: u64) {
        if self.ticket(key) == Some(ticket) {
            self.in_flight.remove(key);
        }
    }

    fn finish_results(
        &mut self,
        key: Key,
        ticket: u64,
        results: Vec<RecallResult>,
        now: u64,
    ) -> bool {
        if self.ticket(&key) != Some(ticket) {
            return false;
        }
        self.in_flight.remove(&key);
        self.put(key, results, now);
        true
    }

    /// Retain raw provenance, not pre-rendered lines with missing restrictions.
    /// Over-budget records are dropped intact instead of changing their claims.
    pub(super) fn put(&mut self, key: Key, results: Vec<RecallResult>, now: u64) {
        self.entries.retain(|_, entry| Self::fresh(entry, now));
        self.sequence = self.sequence.wrapping_add(1);
        if self.entries.len() >= MAX_ENTRIES && !self.entries.contains_key(&key) {
            if let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.sequence)
                .map(|(key, _)| key.clone())
            {
                self.entries.remove(&oldest);
            }
        }
        let mut remaining = MAX_ENTRY_CHARS;
        let results = results
            .into_iter()
            .take(MAX_RECORDS_PER_ENTRY)
            .filter(|result| {
                let chars = result.text.chars().count();
                if chars > remaining
                    || result
                        .metadata
                        .as_ref()
                        .is_some_and(|metadata| metadata.to_string().len() > MAX_METADATA_BYTES)
                {
                    return false;
                }
                remaining -= chars;
                true
            })
            .collect();
        self.entries.insert(
            key,
            Entry {
                results,
                fetched_at_ms: now,
                sequence: self.sequence,
            },
        );
    }

    pub(super) fn clear(&mut self) {
        self.cancel.cancel();
        self.cancel = CancellationToken::new();
        self.entries.clear();
        self.in_flight.clear();
        // A stale completion must not own a later flight for the same key.
        self.sequence = self.sequence.wrapping_add(1);
    }

    pub(super) fn peek(&mut self, dataset: &str, generation: u64, now: u64) -> Vec<RecallResult> {
        self.entries.retain(|_, entry| Self::fresh(entry, now));
        let mut entries: Vec<_> = self
            .entries
            .iter()
            .filter(|(key, _)| key.0 == dataset && key.2 == generation)
            .collect();
        entries.sort_by_key(|(_, entry)| std::cmp::Reverse(entry.sequence));
        entries
            .into_iter()
            .flat_map(|(_, entry)| entry.results.iter().cloned())
            .take(128)
            .collect()
    }

    /// Legacy adapter. Selection is speculative until the host acknowledges
    /// persistence; request-boundary delivery uses raw `peek` and full filters.
    pub(super) fn take(
        &mut self,
        dataset: &str,
        generation: u64,
        now: u64,
        seen: &mut SeenEvidence,
    ) -> Vec<RecallLine> {
        select_new(&self.peek(dataset, generation, now), &mut seen.clone())
    }

    #[cfg(test)]
    pub(super) fn finish(&mut self, key: Key, lines: Vec<RecallLine>, now: u64) {
        let Some(ticket) = self.ticket(&key) else {
            return;
        };
        let results = lines
            .into_iter()
            .map(|line| RecallResult {
                text: line.text,
                score: line.score,
                metadata: None,
            })
            .collect();
        assert!(self.finish_results(key, ticket, results, now));
    }
}

impl CogneeRuntime {
    pub(super) fn schedule_prefetch(&self, query: String, session_id: String) {
        if query.trim().is_empty() {
            return;
        }
        let query = redact_memory_text(&query, 400);
        let (client, dataset, context, fallback, top_k, budget, token, key, ticket) = {
            let mut state = self.state();
            let context = state.operation_context();
            if state.config.read_policy != ReadPolicy::Event
                || !state.config.auto_recall
                || !Self::enabled(&state, self.offline)
                || state.shutting_down
                || !state.recall_circuit.can_attempt(wall_ms())
            {
                return;
            }
            let Some(client) = state.client.clone() else {
                return;
            };
            let dataset = context.dataset.clone();
            let key = EvidenceCache::key(&dataset, &query, context.generation);
            if !state.evidence_cache.schedule(&key, self.now()) {
                return;
            }
            let ticket = state.evidence_cache.ticket(&key).expect("scheduled flight");
            (
                client,
                dataset,
                context,
                state.fallback_dataset(),
                state.config.top_k,
                state.config.recall_budget_ms.min(6000),
                state.evidence_cache.cancel.clone(),
                key,
                ticket,
            )
        };
        let this = self.arc();
        self.track(async move {
            if token.is_cancelled() {
                this.state().evidence_cache.abort_flight(&key, ticket);
                return;
            }
            let request_id = uuid::Uuid::new_v4().to_string();
            let started = std::time::Instant::now();
            let session_id = context.session_id.clone().unwrap_or(session_id);
            this.observe_for("recall", "started", ObserveFields {
                request_id: Some(request_id.clone()),
                meta: super::meta(json!({"source":"prefetch", "outcomeVersion":2, "outcome":"requested", "foregroundWaitMs":0})),
                ..ObserveFields::default()
            }, Some((&session_id, &dataset)));
            let work = recall_with_fallback(&client, &query, RecallOptions {
                top_k: Some(top_k), dataset: Some(dataset.clone()), session_id: Some(session_id.clone()),
                scope: AUTO_RECALL_SCOPE.iter().map(|s| s.to_string()).collect(),
                signal: Some(token.clone()), timeout_ms: Some(budget),
            }, fallback, budget);
            let (result, _) = tokio::select! {
                _ = token.cancelled() => {
                    let current = this.operation_context();
                    let stale_generation = current.generation != context.generation;
                    let stale_dataset = current.dataset != dataset;
                    this.state().evidence_cache.abort_flight(&key, ticket);
                    let reason = if stale_generation { "stale_generation" } else if stale_dataset { "stale_dataset" } else { "shutdown" };
                    this.observe_for("recall", if stale_generation || stale_dataset { "succeeded" } else { "failed" }, ObserveFields {
                        request_id: Some(request_id),
                        meta: super::meta(json!({"source":"prefetch", "outcomeVersion":2, "outcome":if stale_generation || stale_dataset {"stale"} else {"aborted"}, "reason":reason, "remoteWaitMs":started.elapsed().as_millis() as u64, "foregroundWaitMs":0,"hits":0})),
                        ..ObserveFields::default()
                    }, Some((&session_id, &dataset)));
                    return;
                },
                result = work => result,
            };
            let backend_count = result.as_ref().ok().and_then(|results| crate::cognee::client::backend_object_count(results));
            let elapsed = started.elapsed().as_millis() as u64;
            let current = this.operation_context();
            let stale_generation = current.generation != context.generation;
            let stale_dataset = current.dataset != dataset;
            let mut state = this.state();
            let stale = current != context || state.shutting_down || token.is_cancelled();
            let reason = if stale_generation {Some("stale_generation")} else if stale_dataset {Some("stale_dataset")} else if stale {Some("shutdown")} else {None};
            let (phase, outcome, hits, error) = match result {
                Ok(results) if !stale => {
                    let hits = results.len();
                    state.recall_circuit.reset();
                    let stored = state.evidence_cache.finish_results(key.clone(), ticket, results, this.now());
                    ("succeeded", if !stored { "stale".into() } else if hits == 0 { "retrieved_empty".into() } else { "retrieved_nonempty".into() }, hits, None)
                }
                Ok(_) => {
                    state.evidence_cache.abort_flight(&key, ticket);
                    ("succeeded", "stale".into(), 0, None)
                }
                Err(error) => {
                    let kind = super::error_kind(&error);
                    state.evidence_cache.abort_flight(&key, ticket);
                    if !stale && kind != "aborted" {
                        state.last_error = Some(kind.clone());
                        state.recall_circuit.note_failure(&kind, wall_ms());
                    }
                    ("failed", kind.clone(), 0, Some(kind))
                }
            };
            drop(state);
            this.observe_for("recall", phase, ObserveFields {
                request_id: Some(request_id), latency_ms: Some(elapsed), error,
                meta: super::meta(json!({"source":"prefetch", "outcomeVersion":2, "outcome":outcome, "remoteWaitMs":elapsed, "foregroundWaitMs":0, "hits":if stale {0} else {hits}, "backendResultObjects":if stale {None} else {backend_count}, "normalizedRecords":if stale {0} else {hits}, "reason":reason})),
                ..ObserveFields::default()
            }, Some((&session_id, &dataset)));
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn line(text: &str) -> RecallLine {
        RecallLine {
            text: text.into(),
            score: None,
            provenance: None,
        }
    }

    #[test]
    fn queries_longer_than_the_key_limit_are_not_cacheable() {
        assert!(EvidenceCache::keeps_distinct_key(
            &"a ".repeat(200).trim().to_owned()
        ));
        assert!(!EvidenceCache::keeps_distinct_key(&"a".repeat(401)));
        assert_eq!(
            EvidenceCache::key("d", &format!("{}x", "a".repeat(400)), 0),
            EvidenceCache::key("d", &format!("{}y", "a".repeat(400)), 0)
        );
    }

    #[test]
    fn coalesces_normalized_keys_and_expires_without_fetching() {
        let mut cache = EvidenceCache::default();
        let key = EvidenceCache::key("dataset", "  A  Query ", 0);
        assert_eq!(key, EvidenceCache::key("dataset", "A Query", 0));
        assert_ne!(key, EvidenceCache::key("dataset", "a query", 0));
        assert!(cache.schedule(&key, 10));
        assert!(!cache.schedule(&key, 10));
        cache.finish(key.clone(), vec![line("decision")], 10);
        assert!(!cache.schedule(&key, 300_009));
        assert!(cache.peek("dataset", 0, 300_010).is_empty());
        assert!(cache.schedule(&key, 300_010));
        cache.finish(key.clone(), vec![], 300_010);
        assert!(!cache.schedule(&key, 360_009));
        assert!(cache.peek("dataset", 0, 360_010).is_empty());
        assert!(cache.schedule(&key, 360_010));
    }

    #[test]
    fn bounded_entries_and_acknowledged_delivery() {
        let mut cache = EvidenceCache::default();
        for i in 0..33 {
            let key = EvidenceCache::key("dataset", &i.to_string(), 0);
            assert!(cache.schedule(&key, 0));
            cache.finish(key, vec![line(&i.to_string())], 0);
        }
        assert_eq!(cache.entries.len(), 32);
        assert!(!cache
            .entries
            .contains_key(&EvidenceCache::key("dataset", "0", 0)));
        let mut seen = SeenEvidence::default();
        let lines = cache.take("dataset", 0, 1, &mut seen);
        assert_eq!(lines.len(), 32);
        assert_eq!(cache.take("dataset", 0, 1, &mut seen).len(), 32);
        seen.remember_lines(&lines);
        assert!(cache.take("dataset", 0, 1, &mut seen).is_empty());
    }

    #[test]
    fn raw_provenance_survives_multiple_speculative_reads() {
        let mut cache = EvidenceCache::default();
        let result = RecallResult {
            text: "fact".into(),
            score: None,
            metadata: Some(json!({"branch":"main","source_id":"s1","expires_at_ms":1000})),
        };
        cache.put(
            EvidenceCache::key("dataset", "query", 1),
            vec![result.clone()],
            1,
        );
        assert_eq!(cache.peek("dataset", 1, 2), vec![result.clone()]);
        assert_eq!(cache.peek("dataset", 1, 3), vec![result]);
        assert!(cache.peek("dataset", 2, 3).is_empty());
    }

    #[test]
    fn stale_flight_never_finishes_or_cancels_replacement_flight() {
        let mut cache = EvidenceCache::default();
        let key = EvidenceCache::key("dataset", "query", 1);
        assert!(cache.schedule(&key, 1));
        let old = cache.ticket(&key).unwrap();
        cache.clear();
        assert!(cache.schedule(&key, 2));
        let new = cache.ticket(&key).unwrap();
        cache.abort_flight(&key, old);
        assert_eq!(cache.ticket(&key), Some(new));
        assert!(!cache.finish_results(key.clone(), old, vec![], 3));
        assert_eq!(cache.ticket(&key), Some(new));
    }

    #[test]
    fn in_flight_work_is_bounded_before_spawn() {
        let mut cache = EvidenceCache::default();
        assert!(cache.schedule(&EvidenceCache::key("d", "one", 1), 1));
        assert!(cache.schedule(&EvidenceCache::key("d", "two", 1), 1));
        assert!(!cache.schedule(&EvidenceCache::key("d", "three", 1), 1));
    }
}
