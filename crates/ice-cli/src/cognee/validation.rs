//! Fresh, lane-scoped source checks for all candidate admissions. A helper
//! verdict is advisory; this host gate remains authoritative after any await.

use ice_agent::AgentMessage;
use ice_harness::session::types::{BranchBounds, Entry, EntryOrder, EntryQuery};
use tokio_util::sync::CancellationToken;

use super::coordinator::OperationContext;
use super::CogneeRuntime;
use crate::cognee::candidates::{Candidate, CandidateState};
use crate::cognee::config::WritePolicy;
use crate::cognee::identity::RepositoryIdentity;
use crate::cognee::source_validation::{validate_source, SourceEvidence};

const MAX_SOURCE_ENTRIES: usize = 256;

pub(in crate::cognee) struct CurrentSources {
    pub(in crate::cognee) operation: OperationContext,
    pub(in crate::cognee) repository: Option<RepositoryIdentity>,
    pub(in crate::cognee) leaf: Option<String>,
    pub(in crate::cognee) task_id: Option<String>,
    pub(in crate::cognee) messages: Vec<(String, AgentMessage)>,
}

/// Only host code can construct this proof; a tool's JSON cannot claim a
/// source has been checked. It is short-lived and rechecked before commitment.
pub(super) struct CandidateSourceProof {
    pub(super) sources: CurrentSources,
    pub(super) evidence: SourceEvidence,
}

pub(in crate::cognee) fn validate_candidate_scope(
    candidate: &Candidate,
    sources: &CurrentSources,
    evidence: &SourceEvidence,
    require_scope: bool,
) -> Result<(), &'static str> {
    let Some(scope) = candidate.source_scope.as_ref() else {
        return if require_scope || sources.task_id.is_some() {
            Err("candidate task ownership is unknown")
        } else {
            Ok(())
        };
    };
    if scope.session_id != sources.operation.host_session_id
        || scope.task_id != sources.task_id
        || scope.source_text_hash != evidence.source_text_hash
        || !scope
            .conversation_branch
            .as_ref()
            .is_some_and(|branch| branch.contains(&candidate.source.entry_id))
    {
        return Err("candidate source ownership changed");
    }
    let volatile = matches!(
        candidate.category,
        crate::cognee::candidates::Category::Outcome | crate::cognee::candidates::Category::Lesson
    );
    if volatile
        && sources.repository.as_ref().is_none_or(|repository| {
            scope.git_branch != repository.branch
                || scope.git_revision != repository.revision
                || scope.worktree_id.as_deref() != Some(repository.worktree_id.as_str())
        })
    {
        return Err("candidate repository ownership changed");
    }
    Ok(())
}

impl CogneeRuntime {
    pub(in crate::cognee) async fn candidate_source_snapshot(
        &self,
        signal: &CancellationToken,
    ) -> Result<CurrentSources, String> {
        self.current_candidate_sources(signal).await
    }

    /// This is the single host application gate for both direct proposals and
    /// cached/fresh helper verdicts. A helper must name a still-pending record.
    pub(crate) async fn accept_validated_candidate(
        &self,
        candidate: &Candidate,
        signal: &CancellationToken,
        require_pending_record: bool,
    ) -> Result<Option<crate::cognee::propose::AdmissionOutcome>, String> {
        let stored = crate::cognee::candidates::read_candidates(&self.candidates_dir());
        match stored.iter().find(|stored| stored.id == candidate.id) {
            Some(current)
                if serde_json::to_value(current).ok() != serde_json::to_value(candidate).ok()
                    || current.state != CandidateState::Pending =>
            {
                return Err("Candidate changed or was already resolved".into())
            }
            None if require_pending_record => {
                return Err("Pending candidate no longer exists".into())
            }
            _ => {}
        }
        let proof = self.validate_current_candidate(candidate, signal).await?;
        validate_candidate_scope(
            candidate,
            &proof.sources,
            &proof.evidence,
            require_pending_record,
        )
        .map_err(str::to_owned)?;
        self.revalidate_candidate_proof(&proof, signal).await?;
        if signal.is_cancelled() {
            return Err("Candidate operation was cancelled".into());
        }
        let mut attributed = candidate.clone();
        let volatile = matches!(
            candidate.category,
            crate::cognee::candidates::Category::Outcome
                | crate::cognee::candidates::Category::Lesson
        );
        attributed.source_scope = Some(crate::cognee::candidates::SourceScope {
            session_id: proof.sources.operation.host_session_id.clone(),
            task_id: proof.sources.task_id.clone(),
            conversation_branch: Some(
                proof
                    .sources
                    .messages
                    .iter()
                    .map(|(id, _)| id.clone())
                    .collect(),
            ),
            git_branch: if volatile {
                proof
                    .sources
                    .repository
                    .as_ref()
                    .and_then(|repo| repo.branch.clone())
            } else {
                None
            },
            git_revision: if volatile {
                proof
                    .sources
                    .repository
                    .as_ref()
                    .and_then(|repo| repo.revision.clone())
            } else {
                None
            },
            worktree_id: if volatile {
                proof
                    .sources
                    .repository
                    .as_ref()
                    .map(|repo| repo.worktree_id.clone())
            } else {
                None
            },
            source_text_hash: proof.evidence.source_text_hash.clone(),
        });
        let outcome = crate::cognee::propose::accept_candidate(self, &attributed)?;
        let accepted = outcome.is_some();
        self.observe_for("candidate_validation",if accepted {"succeeded"} else {"failed"},super::ObserveFields {
            request_id:Some(candidate.id.clone()),
            meta:super::meta(serde_json::json!({"outcomeVersion":2,"outcome":if accepted {"source_validated_and_queued"}else{"queue_full"},
                "sourceEntryId":proof.evidence.source_entry_id,"sourceTextHash":proof.evidence.source_text_hash,
                "verifierEntryId":proof.evidence.verifier_entry_id,"verifierTextHash":proof.evidence.verifier_text_hash})),
            ..super::ObserveFields::default()
        },Some((proof.sources.operation.session_id.as_deref().unwrap_or_default(),&proof.sources.operation.dataset)));
        if accepted {
            self.invalidate_curated_memory();
        }
        Ok(outcome)
    }

    pub(super) async fn current_candidate_sources(
        &self,
        signal: &CancellationToken,
    ) -> Result<CurrentSources, String> {
        if signal.is_cancelled() {
            return Err("Candidate operation was cancelled".into());
        }
        self.ensure_loaded();
        let repository = self.refresh_repository_identity();
        let operation = self.operation_context();
        let task_id = self.read_coordinator.lock().unwrap().task.current_id();
        {
            let state = self.state();
            if !Self::enabled(&state, self.offline)
                || state.shutting_down
                || state.config.write_policy != WritePolicy::Selective
            {
                return Err("Selective memory admission is unavailable".into());
            }
        }
        let harness = self
            .harness()
            .ok_or_else(|| "Current conversation is unavailable".to_string())?;
        if operation.host_session_id.as_deref()
            != Some(harness.session().storage().metadata().id.as_str())
        {
            return Err("Candidate belongs to a different session".into());
        }
        let view = harness.view();
        let leaf = view
            .get_leaf_id()
            .await
            .map_err(|_| "Could not identify the current conversation branch".to_string())?;
        let query = EntryQuery {
            entry_type: Some("message"),
            order: Some(EntryOrder::NewestFirst),
            limit: Some(MAX_SOURCE_ENTRIES),
            ..Default::default()
        };
        let bounds = BranchBounds {
            start: leaf.clone(),
            ..Default::default()
        };
        let entries = tokio::select! {
            _ = signal.cancelled() => return Err("Candidate operation was cancelled".into()),
            entries = view.find_entries_on_branch(&query,&bounds) => entries.map_err(|_| "Could not read the current conversation branch".to_string())?,
        };
        let now_leaf = view
            .get_leaf_id()
            .await
            .map_err(|_| "Could not revalidate the conversation branch".to_string())?;
        if leaf != now_leaf
            || signal.is_cancelled()
            || self.operation_context() != operation
            || self.refresh_repository_identity() != repository
            || self.read_coordinator.lock().unwrap().task.current_id() != task_id
        {
            return Err("Candidate source scope changed; no memory was accepted".into());
        }
        let messages = entries
            .into_iter()
            .filter_map(|entry| {
                let id = entry.id().to_string();
                match entry {
                    Entry::Message(message) => Some((id, message.message)),
                    _ => None,
                }
            })
            .collect();
        Ok(CurrentSources {
            operation,
            repository,
            leaf,
            task_id,
            messages,
        })
    }

    pub(super) async fn validate_current_candidate(
        &self,
        candidate: &Candidate,
        signal: &CancellationToken,
    ) -> Result<CandidateSourceProof, String> {
        if candidate.state != CandidateState::Pending
            || candidate.created_at > self.now()
            || candidate
                .valid_until_ms
                .is_some_and(|expiry| expiry <= self.now())
        {
            return Err("Candidate is not current pending evidence".into());
        }
        let sources = self.current_candidate_sources(signal).await?;
        if candidate.dataset != sources.operation.dataset {
            return Err("Candidate dataset is not the active project".into());
        }
        if candidate.branch.as_deref().is_some_and(|branch| {
            sources
                .repository
                .as_ref()
                .and_then(|identity| identity.branch.as_deref())
                != Some(branch)
        }) {
            return Err("Candidate Git branch is not current".into());
        }
        let evidence = validate_source(candidate, &sources.messages)
            .map_err(|reason| format!("Candidate source validation failed: {}", reason.as_str()))?;
        Ok(CandidateSourceProof { sources, evidence })
    }

    pub(super) async fn revalidate_candidate_proof(
        &self,
        proof: &CandidateSourceProof,
        signal: &CancellationToken,
    ) -> Result<(), String> {
        if signal.is_cancelled()
            || self.operation_context() != proof.sources.operation
            || self.refresh_repository_identity() != proof.sources.repository
        {
            return Err("Candidate source proof became stale".into());
        }
        let harness = self
            .harness()
            .ok_or_else(|| "Current conversation is unavailable".to_string())?;
        if harness.session().storage().metadata().id
            != proof
                .sources
                .operation
                .host_session_id
                .as_deref()
                .unwrap_or_default()
        {
            return Err("Candidate source proof belongs to a previous session".into());
        }
        let leaf = harness
            .view()
            .get_leaf_id()
            .await
            .map_err(|_| "Could not revalidate candidate source ownership".to_string())?;
        if leaf != proof.sources.leaf
            || signal.is_cancelled()
            || self.operation_context() != proof.sources.operation
            || self.read_coordinator.lock().unwrap().task.current_id() != proof.sources.task_id
        {
            return Err("Candidate conversation changed before commitment".into());
        }
        let state = self.state();
        if !Self::enabled(&state, self.offline)
            || state.shutting_down
            || state.config.write_policy != WritePolicy::Selective
        {
            return Err("Selective admission was disabled before commitment".into());
        }
        Ok(())
    }
}
