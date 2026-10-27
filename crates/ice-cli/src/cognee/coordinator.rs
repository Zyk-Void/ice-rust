//! Resident lifecycle-prefetch evidence. All methods are called under the runtime state lock.

use std::collections::{HashMap, HashSet};

use super::{recall_with_fallback, wall_ms, CogneeRuntime, ObserveFields, AUTO_RECALL_SCOPE};
use crate::cognee::client::{RecallOptions, RecallResult};
use crate::cognee::config::ReadPolicy;
use crate::cognee::evidence::{select_new, SeenEvidence};
use crate::cognee::text::RecallLine;
use serde_json::json;
use tokio_util::sync::CancellationToken;

pub(super) const PREFETCH_ORIENTATION_QUERY: &str = "project decisions, constraints and open work";
const MAX_ENTRIES: usize = 32;
const POSITIVE_TTL_MS: u64 = 300_000;
const NEGATIVE_TTL_MS: u64 = 60_000;
type Key = (String, String, u64);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct OperationContext {
    pub(super) host_session_id: Option<String>,
    pub(super) session_id: Option<String>,
    pub(super) dataset: String,
    pub(super) generation: u64,
}

pub(super) struct Entry {
    pub(super) lines: Vec<RecallLine>,
    fetched_at_ms: u64,
    delivered: bool,
    sequence: u64,
    generation: u64,
}

#[derive(Default)]
pub(super) struct EvidenceCache {
    entries: HashMap<Key, Entry>,
    in_flight: HashSet<Key>,
    sequence: u64,
    cancel: CancellationToken,
}

impl EvidenceCache {
    pub(super) fn key(dataset: &str, query: &str, generation: u64) -> Key {
        (
            dataset.into(),
            query
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase(),
            generation,
        )
    }

    fn fresh(entry: &Entry, now: u64) -> bool {
        now.saturating_sub(entry.fetched_at_ms)
            < if entry.lines.is_empty() {
                NEGATIVE_TTL_MS
            } else {
                POSITIVE_TTL_MS
            }
    }

    pub(super) fn schedule(&mut self, key: &Key, now: u64) -> bool {
        if self.in_flight.contains(key)
            || self
                .entries
                .get(key)
                .is_some_and(|entry| Self::fresh(entry, now))
        {
            return false;
        }
        self.in_flight.insert(key.clone());
        true
    }

    pub(super) fn finish(&mut self, key: Key, lines: Vec<RecallLine>, now: u64) {
        let generation = key.2;
        if !self.in_flight.remove(&key) {
            return;
        }
        self.sequence += 1;
        if !self.entries.contains_key(&key) && self.entries.len() >= MAX_ENTRIES {
            if let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.sequence)
                .map(|(key, _)| key.clone())
            {
                self.entries.remove(&oldest);
            }
        }
        self.entries.insert(
            key,
            Entry {
                lines,
                fetched_at_ms: now,
                delivered: false,
                sequence: self.sequence,
                generation,
            },
        );
    }

    pub(super) fn clear(&mut self) {
        self.cancel.cancel();
        *self = Self::default();
    }

    pub(super) fn take(
        &mut self,
        dataset: &str,
        generation: u64,
        now: u64,
        seen: &mut SeenEvidence,
    ) -> Vec<RecallLine> {
        let mut entries: Vec<_> = self
            .entries
            .iter_mut()
            .filter(|((ds, _, key_generation), entry)| {
                ds == dataset
                    && *key_generation == generation
                    && entry.generation == generation
                    && !entry.delivered
                    && Self::fresh(entry, now)
            })
            .map(|(_, entry)| entry)
            .collect();
        entries.sort_by_key(|entry| entry.sequence);
        let mut selected = Vec::new();
        for entry in entries {
            entry.delivered = true;
            for line in &entry.lines {
                let result = RecallResult {
                    text: line.text.clone(),
                    score: line.score,
                    metadata: None,
                };
                for mut item in select_new(&[result], seen) {
                    item.provenance = line.provenance.clone();
                    selected.push(item);
                }
            }
        }
        selected
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
    fn coalesces_normalized_keys_and_expires_without_fetching() {
        let mut cache = EvidenceCache::default();
        let key = EvidenceCache::key("dataset", "  A  Query ", 0);
        assert_eq!(key, EvidenceCache::key("dataset", "a query", 0));
        assert!(cache.schedule(&key, 10));
        assert!(!cache.schedule(&key, 10));
        cache.finish(key.clone(), vec![line("decision")], 10);
        assert!(!cache.schedule(&key, 300_009));
        assert!(cache
            .take("dataset", 0, 300_010, &mut SeenEvidence::default())
            .is_empty());
        assert!(cache.schedule(&key, 300_010));
        cache.finish(key.clone(), Vec::new(), 300_010);
        assert!(!cache.schedule(&key, 360_009));
        assert!(cache
            .take("dataset", 0, 360_010, &mut SeenEvidence::default())
            .is_empty());
        assert!(cache.schedule(&key, 360_010));
    }

    #[test]
    fn bounded_and_delivered_once() {
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
        assert_eq!(cache.take("dataset", 0, 1, &mut seen).len(), 32);
        assert!(cache.take("dataset", 0, 1, &mut seen).is_empty());
    }
}

impl CogneeRuntime {
    pub(super) fn schedule_prefetch(&self, query: String, session_id: String) {
        if query.trim().is_empty() {
            return;
        }
        let (client, dataset, context, fallback, top_k, budget, token, key) = {
            let mut state = self.state();
            let context = state.operation_context();
            if state.config.read_policy != ReadPolicy::Event
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
            (
                client,
                dataset,
                context,
                state.fallback_dataset(),
                state.config.top_k,
                state.config.recall_budget_ms.min(6000),
                state.evidence_cache.cancel.clone(),
                key,
            )
        };
        let this = self.arc();
        self.track(async move {
            if token.is_cancelled() {
                return;
            }
            let request_id = uuid::Uuid::new_v4().to_string();
            let started = std::time::Instant::now();
            let session_id = context.session_id.clone().unwrap_or(session_id);
            this.observe("recall", "started", ObserveFields { request_id: Some(request_id.clone()), meta: super::meta(json!({"source":"prefetch"})), ..ObserveFields::default() });
            let result = recall_with_fallback(&client, &query, RecallOptions {
                top_k: Some(top_k), dataset: Some(dataset.clone()), session_id: Some(session_id.clone()),
                scope: AUTO_RECALL_SCOPE.iter().map(|s| s.to_string()).collect(), signal: Some(token.clone()), timeout_ms: Some(budget),
            }, fallback, budget);
            let (result, _) = tokio::select! {
                _ = token.cancelled() => {
                    let current = this.operation_context();
                    let stale_generation = current.generation != context.generation;
                    let stale_dataset = current.dataset != dataset;
                    let reason = if stale_generation {
                        "stale_generation"
                    } else if stale_dataset {
                        "stale_dataset"
                    } else {
                        "shutdown"
                    };
                    this.observe_for("recall", if stale_generation || stale_dataset { "succeeded" } else { "failed" }, ObserveFields { request_id: Some(request_id), meta: super::meta(json!({"source":"prefetch", "outcomeVersion":1, "outcome":if stale_generation || stale_dataset { "stale" } else { "aborted" }, "reason":reason, "remoteWaitMs":started.elapsed().as_millis() as u64, "hits":0})), ..ObserveFields::default() }, Some((&session_id, &dataset)));
                    return;
                },
                result = result => result,
            };
            let elapsed = started.elapsed().as_millis() as u64;
            let current = this.operation_context();
            let stale_generation = current.generation != context.generation;
            let stale_dataset = current.dataset != dataset;
            let mut state = this.state();
            let stale = stale_generation || stale_dataset || state.shutting_down || token.is_cancelled();
            let reason = if stale_generation {
                Some("stale_generation")
            } else if stale_dataset {
                Some("stale_dataset")
            } else if stale {
                Some("shutdown")
            } else {
                None
            };
            let (phase, outcome, hits, error) = match result {
                Ok(results) => {
                    let hits = results.len();
                    if stale {
                        state.evidence_cache.in_flight.remove(&key);
                        ("succeeded", "stale".to_string(), 0, None)
                    } else {
                        let lines = select_new(&results, &mut SeenEvidence::default());
                        state.recall_circuit.reset();
                        state.evidence_cache.finish(key, lines, this.now());
                        ("succeeded", if hits == 0 { "retrieved_empty".to_string() } else { "retrieved_nonempty".to_string() }, hits, None)
                    }
                }
                Err(error) => {
                    let kind = super::error_kind(&error);
                    state.evidence_cache.in_flight.remove(&key);
                    if !stale {
                        state.last_error = Some(kind.clone());
                        state.recall_circuit.note_failure(&kind, wall_ms());
                    }
                    ("failed", kind.clone(), 0, Some(kind))
                }
            };
            drop(state);
            let outcome = if stale && error.is_none() {
                "stale"
            } else {
                outcome.as_str()
            };
            this.observe_for("recall", phase, ObserveFields { request_id: Some(request_id), latency_ms: Some(elapsed), error, meta: super::meta(json!({"source":"prefetch", "outcomeVersion":1, "outcome":outcome, "remoteWaitMs":elapsed, "hits":if stale { 0 } else { hits }, "reason":reason})), ..ObserveFields::default() }, Some((&session_id, &dataset)));
        });
    }
}
