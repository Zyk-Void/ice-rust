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

pub(super) struct CurrentSources {
    pub(super) operation: OperationContext,
    pub(super) repository: Option<RepositoryIdentity>,
    pub(super) leaf: Option<String>,
    pub(super) messages: Vec<(String, AgentMessage)>,
}

/// Only host code can construct this proof; a tool's JSON cannot claim a
/// source has been checked. It is short-lived and rechecked before commitment.
pub(super) struct CandidateSourceProof {
    pub(super) sources: CurrentSources,
    pub(super) evidence: SourceEvidence,
}

impl CogneeRuntime {
    pub(crate) async fn candidate_source_messages(
        &self,
        signal: &CancellationToken,
    ) -> Result<Vec<(String, AgentMessage)>, String> {
        Ok(self.current_candidate_sources(signal).await?.messages)
    }

    /// This is the single host application gate for both direct proposals and
    /// cached/fresh helper verdicts. A helper must name a still-pending record.
    pub(crate) async fn accept_validated_candidate(
        &self,
        candidate: &Candidate,
        signal: &CancellationToken,
        require_pending_record: bool,
    ) -> Result<bool, String> {
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
        self.revalidate_candidate_proof(&proof, signal).await?;
        if signal.is_cancelled() {
            return Err("Candidate operation was cancelled".into());
        }
        let accepted = crate::cognee::propose::accept_candidate(self, candidate)?;
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
        Ok(accepted)
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
        {
            return Err("Candidate source scope changed; no memory was accepted".into());
        }
        let messages = entries
            .into_iter()
            .filter_map(|entry| match entry {
                Entry::Message(message) => Some((message.base.id, message.message)),
                _ => None,
            })
            .collect();
        Ok(CurrentSources {
            operation,
            repository,
            leaf,
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
