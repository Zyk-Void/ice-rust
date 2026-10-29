//! Explicit memory lookup: one contract shared by the native tool and command.
//! It preserves immutable request ownership, validates complete source records,
//! and does not consume novelty before the tool result is actually persisted.

use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use super::coordinator::EvidenceCache;
use super::{
    error_kind, meta, recall_with_fallback, wall_ms, CogneeRuntime, NoticeLevel, ObserveFields,
    RecallOptions,
};
use crate::cognee::client::backend_object_count;
use crate::cognee::delivery::fit_packet;
use crate::cognee::evidence::{select_atomic_with_context, FilterContext, SeenEvidence};
use crate::cognee::text::redact_memory_text;

impl CogneeRuntime {
    pub(super) async fn explicit_lookup(
        &self,
        query: &str,
        signal: CancellationToken,
    ) -> (String, Value) {
        self.ensure_loaded();
        let dataset = self.active_dataset();
        if signal.is_cancelled() {
            return (
                "Cognee search aborted (cancelled).".into(),
                json!({"enabled":self.is_enabled(),"count":0,"dataset":dataset,"error":"aborted","outcome":"cancelled"}),
            );
        }
        if query.trim().is_empty() || query.chars().count() > 4000 {
            return (
                "Cognee search needs a nonempty query of at most 4000 characters.".into(),
                json!({"enabled":self.is_enabled(),"count":0,"dataset":dataset,"error":"invalid_query"}),
            );
        }
        if !self.is_enabled() {
            return (
                self.disabled_message().into(),
                json!({"enabled":false,"count":0,"dataset":dataset,"outcome":"disabled"}),
            );
        }
        let repository = self.refresh_repository_identity();
        let operation = self.operation_context();
        let (client, top_k, max_chars, legacy_dataset, budget_ms) = {
            let state = self.state();
            (
                state.client.clone(),
                state.config.top_k,
                state.config.recall_max_chars,
                state.fallback_dataset(),
                state.config.recall_budget_ms,
            )
        };
        let Some(client) = client else {
            return (
                "Cognee client is unavailable.".into(),
                json!({"enabled":true,"count":0,"dataset":dataset,"error":"unavailable"}),
            );
        };
        let query = redact_memory_text(query.trim(), 4000);
        let request_id = uuid::Uuid::new_v4().to_string();
        let started = std::time::Instant::now();
        let session = operation.session_id.as_deref().unwrap_or_default();
        self.observe_for(
            "recall",
            "started",
            ObserveFields {
                request_id: Some(request_id.clone()),
                meta: meta(
                    json!({"outcomeVersion":2,"source":"explicit_lookup","outcome":"requested"}),
                ),
                ..ObserveFields::default()
            },
            Some((session, &operation.dataset)),
        );
        // Explicit requests do not depend on the optional admission classifier
        // or its circuit. The shared deadline still bounds canonical+alias work.
        let (result, legacy_attempted) = recall_with_fallback(
            &client,
            &query,
            RecallOptions {
                top_k: Some(top_k),
                dataset: Some(operation.dataset.clone()),
                session_id: operation.session_id.clone(),
                signal: Some(signal.clone()),
                timeout_ms: Some(budget_ms),
                ..RecallOptions::default()
            },
            legacy_dataset,
            budget_ms,
        )
        .await;
        let current_repository = self.refresh_repository_identity();
        let elapsed = started.elapsed().as_millis() as u64;
        let stale = signal.is_cancelled()
            || self.operation_context() != operation
            || current_repository != repository
            || !self.is_enabled();
        if stale {
            self.observe_for("recall","failed",ObserveFields {
                request_id:Some(request_id),latency_ms:Some(elapsed),error:Some(if signal.is_cancelled(){"aborted"}else{"stale"}.into()),
                meta:meta(json!({"outcomeVersion":2,"source":"explicit_lookup","outcome":"stale","foregroundWaitMs":elapsed})),..ObserveFields::default()
            },Some((session,&operation.dataset)));
            return ("Cognee result was discarded because its request was cancelled or its session, configuration, or repository scope changed.".into(),json!({"enabled":self.is_enabled(),"count":0,"dataset":operation.dataset,"error":if signal.is_cancelled(){"aborted"}else{"stale"},"outcome":"discarded"}));
        }
        match result {
            Ok(results) => {
                let backend_count = backend_object_count(&results);
                let normalized_count = results.len();
                let (selection, prepared) = {
                    let mut state = self.state();
                    state.recall_circuit.reset();
                    state.last_error = None;
                    let filter = FilterContext {
                        now_ms: self.now(),
                        dataset: operation.dataset.clone(),
                        legacy_dataset: state.fallback_dataset(),
                        branch: repository
                            .as_ref()
                            .and_then(|identity| identity.branch.clone()),
                        revision: repository
                            .as_ref()
                            .and_then(|identity| identity.revision.clone()),
                        worktree_id: repository
                            .as_ref()
                            .map(|identity| identity.worktree_id.clone()),
                        suppressions: Some(&state.suppressions),
                    };
                    // Explicit searches can repeat. Do not falsely consume a
                    // fact merely because a tool was invoked or a result fit.
                    let selection =
                        select_atomic_with_context(&results, &mut SeenEvidence::default(), &filter);
                    let prepared = fit_packet(&selection.lines, None, 800, max_chars);
                    if EvidenceCache::keeps_distinct_key(&query) {
                        let key =
                            EvidenceCache::key(&operation.dataset, &query, operation.generation);
                        state.evidence_cache.put(key, results, self.now());
                    }
                    (selection, prepared)
                };
                let count = prepared.lines.len();
                let outcome = if count > 0 {
                    "retained_evidence"
                } else if normalized_count > 0 {
                    "filtered_empty"
                } else {
                    "retrieved_empty"
                };
                self.observe_for("recall","succeeded",ObserveFields {
                    request_id:Some(request_id),latency_ms:Some(elapsed),meta:meta(json!({
                        "outcomeVersion":2,"source":"explicit_lookup","outcome":outcome,"backendResultObjects":backend_count,"normalizedRecords":normalized_count,
                        "retainedRecords":count,"rejected":selection.rejected,"foregroundWaitMs":elapsed,"estimatedResultTokens":prepared.estimated_tokens
                    })),..ObserveFields::default()
                },Some((session,&operation.dataset)));
                let text = if count == 0 {
                    "No applicable Cognee memory found.".into()
                } else {
                    prepared.text
                };
                let mut details = json!({"enabled":true,"count":count,"dataset":operation.dataset,"outcome":outcome,"backendResultObjects":backend_count,"normalizedRecords":normalized_count,
                    "retainedRecords":count,"rejected":selection.rejected,"estimatedResultTokens":prepared.estimated_tokens,
                    "sources":prepared.lines.iter().filter_map(|line|line.provenance.as_deref()).collect::<Vec<_>>()});
                if let Some(legacy) = legacy_attempted {
                    details["legacyDataset"] = json!(legacy);
                    details["legacyHits"] = json!(normalized_count);
                }
                (text, details)
            }
            Err(error) => {
                let kind = error_kind(&error);
                if kind != "aborted" {
                    let mut state = self.state();
                    state.last_error = Some(kind.clone());
                    state.recall_circuit.note_failure(&kind, wall_ms());
                }
                self.observe_for("recall","failed",ObserveFields {request_id:Some(request_id),latency_ms:Some(elapsed),error:Some(kind.clone()),
                    meta:meta(json!({"outcomeVersion":2,"source":"explicit_lookup","outcome":kind,"foregroundWaitMs":elapsed})),..ObserveFields::default()
                },Some((session,&operation.dataset)));
                (
                    format!("Cognee search failed: {kind}"),
                    json!({"enabled":true,"count":0,"dataset":operation.dataset,"error":kind,"outcome":"retrieval_failed"}),
                )
            }
        }
    }

    pub(super) async fn explicit_search_notice(&self, query: &str) {
        let (text, details) = self.explicit_lookup(query, CancellationToken::new()).await;
        let level = if details.get("error").is_some() {
            NoticeLevel::Error
        } else if details.get("enabled") == Some(&Value::Bool(false)) {
            NoticeLevel::Warning
        } else {
            NoticeLevel::Info
        };
        self.notify(level, text);
    }
}
