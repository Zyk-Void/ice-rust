//! Production request-boundary memory coordinator. Ordinary prompts inspect
//! ready resident state only. Optional retrieval is event-driven, and evidence
//! becomes seen only after the host acknowledges its durable insertion.

use std::collections::VecDeque;
use std::path::Path;

use ice_harness::extension::{
    BeforeRunEvent, HarnessExtension, PromptContext, RequestContextEvent,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

use super::coordinator::{EvidenceCache, OperationContext};
use super::{
    meta, recall_with_fallback, CogneeRuntime, ObserveFields, ReadPolicy, RecallOptions,
    AUTO_RECALL_SCOPE, MEMORY_CUSTOM_TYPE,
};
use crate::cognee::client::RecallResult;
use crate::cognee::delivery::{
    automatic_token_budget, fit_packet, has_query_anchor, prepare_selection, PreparedMemory,
};
use crate::cognee::evidence::FilterContext;
use crate::cognee::identity::{IdentityTracker, RepositoryIdentity};
use crate::cognee::policy::asks_for_history;
use crate::cognee::task::{TaskCursor, TaskEvent, TaskScope};
use crate::cognee::text::redact_memory_text;

const MAX_PENDING_PACKETS: usize = 8;
const PACKET_TTL_MS: u64 = 60_000;
const MAX_RESIDENT_FACTS: usize = 128;
const RESIDENT_REFRESH_MS: u64 = 60_000;

struct PendingPacket {
    id: String,
    body_hash: String,
    operation: OperationContext,
    task_id: String,
    run_id: String,
    prepared: PreparedMemory,
    created_at: u64,
    signal: CancellationToken,
}

#[derive(Default)]
pub(super) struct ReadCoordinator {
    repository: IdentityTracker,
    pub(in crate::cognee) task: TaskCursor,
    pending: VecDeque<PendingPacket>,
    last_run_id: Option<String>,
    last_compaction_at: Option<i64>,
    resident: Vec<RecallResult>,
    resident_dirty: bool,
    resident_loading: bool,
    resident_revision: u64,
    resident_refreshed_at: Option<u64>,
}

impl ReadCoordinator {
    fn prune(&mut self, now: u64) {
        self.pending.retain(|packet| {
            !packet.signal.is_cancelled() && now.saturating_sub(packet.created_at) < PACKET_TTL_MS
        });
        while self.pending.len() >= MAX_PENDING_PACKETS {
            self.pending.pop_front();
        }
    }
}

fn body_hash(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

impl CogneeRuntime {
    /// Current applicability snapshot for shared candidate admission. It is
    /// initialized by a host lifecycle/request event, never by a remote judge.
    /// Refresh bounded Git metadata for an explicit operation. Changes fence
    /// in-flight optional work before returning the new applicability snapshot.
    pub(crate) fn refresh_repository_identity(&self) -> Option<RepositoryIdentity> {
        let cwd = self.cwd.lock().unwrap().clone();
        let (changed, identity) = {
            let mut coordinator = self.read_coordinator.lock().unwrap();
            let changed = coordinator
                .repository
                .refresh(cwd.as_deref().unwrap_or_else(|| Path::new(".")), false);
            let identity = coordinator.repository.snapshot().cloned();
            if changed {
                coordinator.pending.clear();
                coordinator.resident.clear();
                coordinator.resident_dirty = true;
                coordinator.resident_revision = coordinator.resident_revision.wrapping_add(1);
            }
            (changed, identity)
        };
        if changed {
            let mut state = self.state();
            state.context_generation = state.context_generation.saturating_add(1);
            state.evidence_cache.clear();
            state.seen_evidence.clear();
            state.helper_cancel.cancel();
            state.helper_cancel = CancellationToken::new();
        }
        identity
    }

    pub(crate) fn repository_identity(&self) -> Option<RepositoryIdentity> {
        self.read_coordinator
            .lock()
            .unwrap()
            .repository
            .snapshot()
            .cloned()
    }

    /// Invalidate a resident view after a committed local admission/correction.
    /// The subsequent refresh runs off the prompt's foreground path.
    pub(crate) fn invalidate_curated_memory(&self) {
        let mut coordinator = self.read_coordinator.lock().unwrap();
        coordinator.resident_dirty = true;
        coordinator.resident_revision = coordinator.resident_revision.wrapping_add(1);
        drop(coordinator);
        self.refresh_curated_memory();
    }

    fn refresh_curated_memory(&self) {
        let (operation, enabled) = {
            let state = self.state();
            (
                state.operation_context(),
                Self::enabled(&state, self.offline) && !state.shutting_down,
            )
        };
        if !enabled {
            return;
        }
        let (branch, revision) = {
            let mut coordinator = self.read_coordinator.lock().unwrap();
            if !coordinator.resident_dirty || coordinator.resident_loading {
                return;
            }
            coordinator.resident_loading = true;
            let branch = coordinator
                .repository
                .snapshot()
                .and_then(|identity| identity.branch.clone());
            (branch, coordinator.resident_revision)
        };
        let this = self.arc();
        self.track(async move {
            let directory = this.candidates_dir();
            let dataset = operation.dataset.clone();
            let now = this.now();
            let results = tokio::task::spawn_blocking(move || {
                crate::cognee::candidates::query_accepted(
                    &directory,
                    &dataset,
                    branch.as_deref(),
                    "",
                    now,
                    MAX_RESIDENT_FACTS,
                )
            })
            .await
            .unwrap_or_default();
            let current = this.operation_context();
            let retry = {
                let mut coordinator = this.read_coordinator.lock().unwrap();
                coordinator.resident_loading = false;
                if current == operation && revision == coordinator.resident_revision {
                    coordinator.resident = results;
                    coordinator.resident_dirty = false;
                    coordinator.resident_refreshed_at = Some(this.now());
                }
                coordinator.resident_dirty
            };
            // A later request/lifecycle can refresh an invalidated revision;
            // do not recursively spawn on repeated filesystem changes.
            let _ = retry;
        });
    }

    /// A main-agent task/gap signal uses its already-needed generation. It does
    /// not grant write permission, call a classifier or execute a capability.
    pub(crate) fn signal_memory_task(
        &self,
        event: TaskEvent,
        query: &str,
        signal: &CancellationToken,
    ) -> bool {
        if signal.is_cancelled() || !self.is_enabled() {
            return false;
        }
        let changed = event == TaskEvent::Changed;
        let accepted = self
            .read_coordinator
            .lock()
            .unwrap()
            .task
            .signal(event, query);
        if accepted && changed {
            let mut state = self.state();
            state.context_generation = state.context_generation.saturating_add(1);
            state.evidence_cache.clear();
            state.seen_evidence.clear();
            state.helper_cancel.cancel();
            state.helper_cancel = CancellationToken::new();
        }
        accepted
    }

    pub(super) async fn context_for_request(
        &self,
        event: &RequestContextEvent<'_>,
    ) -> Option<PromptContext> {
        if event.signal.is_cancelled() || !self.is_installed() || !self.is_enabled() {
            return None;
        }
        let session_id = self.ensure_session_id(event.session_id);
        let policy = self.state().config.read_policy;
        if policy == ReadPolicy::Legacy {
            if event.request_index != 0 {
                return None;
            }
            let original = event.original_prompt?;
            return self
                .prompt_context(&BeforeRunEvent {
                    prompt: original,
                    system_prompt: Some(event.system_prompt),
                    session_id: event.session_id,
                    signal: event.signal.clone(),
                })
                .await;
        }
        let cwd = self.cwd.lock().unwrap().clone();
        let now = self.now();
        let (
            owner_changed,
            compaction_changed,
            task_event,
            task_id,
            task_query,
            repository,
            new_run,
        ) = {
            let mut coordinator = self.read_coordinator.lock().unwrap();
            let repository_changed = coordinator
                .repository
                .refresh(cwd.as_deref().unwrap_or_else(|| Path::new(".")), false);
            let repository = coordinator
                .repository
                .snapshot()
                .cloned()
                .expect("identity refreshed");
            let previous_scope = coordinator.task.scope().cloned();
            let resumed = previous_scope.as_ref().is_some_and(|scope| {
                scope.session_id == event.session_id
                    && scope.branch_generation != event.branch_generation
            });
            let owner_changed = coordinator.task.observe(
                TaskScope {
                    session_id: event.session_id.into(),
                    lane_id: event.lane_id.into(),
                    branch_generation: event.branch_generation,
                    repository_key: repository.applicability_key(),
                },
                event.original_prompt,
                event.latest_compaction_at,
                resumed,
            );
            let compaction_changed = coordinator.last_compaction_at != event.latest_compaction_at;
            coordinator.last_compaction_at = event.latest_compaction_at;
            let new_run = coordinator.last_run_id.as_deref() != Some(event.run_id);
            coordinator.last_run_id = Some(event.run_id.into());
            if owner_changed || repository_changed {
                coordinator.pending.clear();
                coordinator.resident.clear();
                coordinator.resident_dirty = true;
                coordinator.resident_revision = coordinator.resident_revision.wrapping_add(1);
            }
            // Another process may have committed a curated fact. Refresh the
            // bounded local snapshot at a later run boundary, never await disk
            // or authorize network work merely because its age expired.
            if new_run
                && coordinator
                    .resident_refreshed_at
                    .is_some_and(|at| now.saturating_sub(at) >= RESIDENT_REFRESH_MS)
                && !coordinator.resident_dirty
            {
                coordinator.resident.clear();
                coordinator.resident_dirty = true;
                coordinator.resident_revision = coordinator.resident_revision.wrapping_add(1);
            }
            coordinator.prune(now);
            let task_event = coordinator.task.take_event();
            (
                owner_changed || repository_changed,
                compaction_changed,
                task_event,
                coordinator.task.id(),
                coordinator.task.query().to_owned(),
                repository,
                new_run,
            )
        };
        if owner_changed {
            let mut state = self.state();
            state.context_generation = state.context_generation.saturating_add(1);
            state.evidence_cache.clear();
            state.seen_evidence.clear();
            state.helper_cancel.cancel();
            state.helper_cancel = CancellationToken::new();
        } else if compaction_changed {
            self.state().seen_evidence.clear();
        }
        if policy == ReadPolicy::OnDemand {
            return None;
        }
        self.refresh_curated_memory();
        let original = event.original_prompt.unwrap_or_default();
        let explicit = new_run
            && event.request_index == 0
            && event.original_prompt.is_some()
            && asks_for_history(original);
        let query = if explicit && has_query_anchor(original) {
            redact_memory_text(original, 400)
        } else if !task_query.is_empty() {
            task_query
        } else if explicit {
            redact_memory_text(original, 400)
        } else {
            String::new()
        };
        let operation = self.operation_context();
        let key = EvidenceCache::key(&operation.dataset, &query, operation.generation);
        let (client, top_k, budget_ms, legacy, auto_recall, circuit_ready, fresh_query) = {
            let state = self.state();
            (
                state.client.clone(),
                state.config.top_k,
                state.config.recall_budget_ms,
                state.fallback_dataset(),
                state.config.auto_recall,
                state.recall_circuit.can_attempt(super::wall_ms()),
                state.evidence_cache.has_fresh(&key, now),
            )
        };
        if !auto_recall || query.is_empty() {
            return None;
        }
        if explicit && !fresh_query && circuit_ready {
            if let Some(client) = client {
                let request_id = uuid::Uuid::new_v4().to_string();
                let started = std::time::Instant::now();
                self.observe_for("recall", "started", ObserveFields {
                    request_id: Some(request_id.clone()),
                    meta: meta(json!({"outcomeVersion":2,"outcome":"requested","reason":"explicit_history","taskId":task_id,"runId":event.run_id})),
                    ..ObserveFields::default()
                }, Some((&session_id, &operation.dataset)));
                let (result, _) = recall_with_fallback(
                    &client,
                    &query,
                    RecallOptions {
                        top_k: Some(top_k),
                        dataset: Some(operation.dataset.clone()),
                        session_id: Some(session_id.clone()),
                        scope: AUTO_RECALL_SCOPE
                            .iter()
                            .map(|scope| scope.to_string())
                            .collect(),
                        signal: Some(event.signal.clone()),
                        timeout_ms: Some(budget_ms),
                    },
                    legacy,
                    budget_ms,
                )
                .await;
                let backend_count = result
                    .as_ref()
                    .ok()
                    .and_then(|results| crate::cognee::client::backend_object_count(results));
                let stale = self.operation_context() != operation || event.signal.is_cancelled();
                let (phase, outcome, count, error) = match result {
                    Ok(results) if !stale => {
                        let count = results.len();
                        let mut state = self.state();
                        state.recall_circuit.reset();
                        state.evidence_cache.put(key, results, self.now());
                        (
                            "succeeded",
                            if count == 0 {
                                "retrieved_empty"
                            } else {
                                "retrieved_nonempty"
                            },
                            count,
                            None,
                        )
                    }
                    Ok(_) => ("succeeded", "stale", 0, None),
                    Err(error) => {
                        let kind = super::error_kind(&error);
                        if !stale && kind != "aborted" {
                            self.state()
                                .recall_circuit
                                .note_failure(&kind, super::wall_ms());
                        }
                        ("failed", "retrieval_failed", 0, Some(kind))
                    }
                };
                self.observe_for("recall", phase, ObserveFields {
                    request_id: Some(request_id), latency_ms: Some(started.elapsed().as_millis() as u64), error,
                    meta: meta(json!({"outcomeVersion":2,"outcome":outcome,"backendResultObjects":backend_count,"normalizedRecords":count,"hits":count,"foregroundWaitMs":started.elapsed().as_millis() as u64,"taskId":task_id,"runId":event.run_id})),
                    ..ObserveFields::default()
                }, Some((&session_id, &operation.dataset)));
                if stale {
                    return None;
                }
            }
        } else if !explicit
            && matches!(
                task_event,
                Some(TaskEvent::Resumed | TaskEvent::Changed | TaskEvent::HistoryGap)
            )
            && has_query_anchor(&query)
        {
            self.schedule_prefetch(query.clone(), session_id.clone());
        }
        let resident = self.read_coordinator.lock().unwrap().resident.clone();
        let (results, selection, checkpoint, char_budget) = {
            let mut state = self.state();
            if state.operation_context() != operation
                || state.shutting_down
                || !Self::enabled(&state, self.offline)
            {
                return None;
            }
            let mut results =
                state
                    .evidence_cache
                    .peek(&operation.dataset, operation.generation, self.now());
            results.extend(resident);
            let filter = FilterContext {
                now_ms: self.now(),
                dataset: operation.dataset.clone(),
                legacy_dataset: state.fallback_dataset(),
                branch: repository.branch.clone(),
                revision: repository.revision.clone(),
                worktree_id: Some(repository.worktree_id.clone()),
                suppressions: Some(&state.suppressions),
            };
            let selection = prepare_selection(
                &results,
                &state.seen_evidence,
                &filter,
                event.messages,
                &query,
                !explicit,
            );
            (
                results.len(),
                selection,
                state.last_compact_summary.clone(),
                state.config.recall_max_chars,
            )
        };
        let budget = automatic_token_budget(event.context_window, event.remaining_context_tokens);
        let prepared = fit_packet(&selection.lines, checkpoint.as_deref(), budget, char_budget);
        if prepared.text.is_empty() {
            return None;
        }
        if event.signal.is_cancelled() || self.operation_context() != operation {
            return None;
        }
        let id = format!("memory-v2:{}", uuid::Uuid::new_v4());
        let text = prepared.text.clone();
        let retained = prepared.lines.len();
        {
            let mut coordinator = self.read_coordinator.lock().unwrap();
            if coordinator.task.id() != task_id {
                return None;
            }
            coordinator.prune(self.now());
            coordinator.pending.push_back(PendingPacket {
                id: id.clone(),
                body_hash: body_hash(&text),
                operation: operation.clone(),
                task_id: task_id.clone(),
                run_id: event.run_id.into(),
                prepared,
                created_at: self.now(),
                signal: event.signal.clone(),
            });
        }
        self.observe_for("selection", "succeeded", ObserveFields {
            request_id: Some(id.clone()),
            meta: meta(json!({"outcomeVersion":2,"outcome":"prepared","candidateRecords":results,"retainedRecords":retained,"rejected":selection.rejected,"taskId":task_id,"runId":event.run_id,"foregroundWaitMs":0})),
            ..ObserveFields::default()
        }, Some((&session_id, &operation.dataset)));
        Some(PromptContext {
            custom_type: MEMORY_CUSTOM_TYPE.into(),
            text,
            correlation_id: Some(id),
        })
    }

    /// Acknowledgment is not a usefulness claim. A false/stale acknowledgment
    /// leaves the cache, checkpoint and novelty ledger unchanged.
    pub(super) fn acknowledge_request_memory(
        &self,
        context: &PromptContext,
        persisted: bool,
    ) -> bool {
        let Some(id) = context.correlation_id.as_deref() else {
            return false;
        };
        let packet = {
            let mut coordinator = self.read_coordinator.lock().unwrap();
            let Some(index) = coordinator
                .pending
                .iter()
                .position(|packet| packet.id == id)
            else {
                // Expired/evicted ownership cannot be reclassified as a
                // legacy successful injection into the new current session.
                return id.starts_with("memory-v2:");
            };
            coordinator.pending.remove(index).expect("known packet")
        };
        let (current_task, current_run) = {
            let coordinator = self.read_coordinator.lock().unwrap();
            (coordinator.task.id(), coordinator.last_run_id.clone())
        };
        let mut state = self.state();
        let valid = state.operation_context() == packet.operation
            && packet.task_id == current_task
            && current_run.as_deref() == Some(packet.run_id.as_str())
            && body_hash(&context.text) == packet.body_hash
            && !packet.signal.is_cancelled()
            && self.now().saturating_sub(packet.created_at) < PACKET_TTL_MS
            && Self::enabled(&state, self.offline)
            && !state.shutting_down;
        let committed = persisted && valid;
        if committed {
            state.seen_evidence.remember_lines(&packet.prepared.lines);
            if packet.prepared.checkpoint.is_some()
                && state.last_compact_summary == packet.prepared.checkpoint
            {
                state.last_compact_summary = None;
            }
        }
        drop(state);
        self.observe_for("injection", if committed {"succeeded"} else {"failed"}, ObserveFields {
            request_id: Some(id.into()),
            meta: meta(json!({"outcomeVersion":2,"outcome":if committed {"persisted"} else if valid {"append_failed"} else {"stale"},
                "persistedBytes":if committed {context.text.len()} else {0},"estimatedPersistedTokens":if committed {packet.prepared.estimated_tokens} else {0},
                "retainedRecords":if committed {packet.prepared.lines.len()} else {0},"taskId":packet.task_id,"runId":packet.run_id})),
            ..ObserveFields::default()
        }, Some((packet.operation.session_id.as_deref().unwrap_or_default(), &packet.operation.dataset)));
        true
    }
}
