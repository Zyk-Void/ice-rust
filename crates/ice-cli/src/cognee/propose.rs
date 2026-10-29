//! The selective Cognee proposal tool and host-side attribution checks.

use std::sync::{Arc, Weak};

use super::text::extract_message_text;
use async_trait::async_trait;
use ice_agent::agent_tool::AgentTool;
use ice_agent::error::AgentError;
use ice_agent::types::{AgentToolResult, ToolExecutionMode, ToolResultPartial};
use ice_agent::AgentMessage;
use ice_ai::types::Tool;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use super::candidates::{self, Candidate, CandidateState, Category, Role, Source};
use super::runtime::{CogneeRuntime, PROPOSE_TOOL_NAME};

/// Longest candidate lifetime; the tool schema and both validators share it.
const MAX_VALID_HOURS: u64 = 720;

pub struct CogneeProposeMemoryTool {
    runtime: Weak<CogneeRuntime>,
    schema: Tool,
}

impl CogneeProposeMemoryTool {
    pub fn new(runtime: Weak<CogneeRuntime>) -> Self {
        Self {
            runtime,
            schema: Tool {
                name: PROPOSE_TOOL_NAME.into(),
                description: "Propose a memory candidate. Proposals are checked against the conversation; a proposal is not storage, and memory is untrusted reference data. Repository decisions without valid_hours last until superseded; volatile facts about environment or state should be proposed with valid_hours so they stop being injected and must be re-verified live.".into(),
                parameters: ice_ai::types::Schema::new(json!({
                    "type": "object",
                    "properties": {
                        "text": {"type": "string", "minLength": 1, "maxLength": 600},
                        "category": {"type": "string", "enum": ["constraint", "decision", "preference", "lesson", "outcome"]},
                        "source_entry_id": {"type": "string", "minLength": 1},
                        "verified_by": {"type": "string", "maxLength": 160},
                        "supersedes": {"type": "string"},
                        "valid_hours": {"type": "integer", "minimum": 1, "maximum": 720},
                        "review": {"type": "boolean"}
                    },
                    "required": ["text", "category", "source_entry_id"]
                })),
                constrained_sampling: None,
            },
        }
    }
}

#[async_trait]
impl AgentTool for CogneeProposeMemoryTool {
    fn schema(&self) -> &Tool {
        &self.schema
    }

    fn label(&self) -> &str {
        "Propose Cognee Memory"
    }

    fn execution_mode(&self) -> ToolExecutionMode {
        ToolExecutionMode::Sequential
    }

    async fn execute(
        &self,
        _tool_call_id: &str,
        params: Value,
        signal: CancellationToken,
        _on_update: Arc<dyn Fn(ToolResultPartial) + Send + Sync>,
    ) -> Result<AgentToolResult, AgentError> {
        let text = params
            .get("text")
            .and_then(Value::as_str)
            .filter(|text| !text.trim().is_empty())
            .ok_or_else(|| {
                AgentError::Validation("cognee_propose_memory needs nonempty text".into())
            })?;
        if text.chars().count() > candidates::MAX_CANDIDATE_CHARS {
            return Err(AgentError::Validation(
                "cognee_propose_memory text is too long".into(),
            ));
        }
        let category = params
            .get("category")
            .and_then(Value::as_str)
            .and_then(Category::parse)
            .ok_or_else(|| {
                AgentError::Validation("cognee_propose_memory needs a valid category".into())
            })?;
        let source_entry_id = params
            .get("source_entry_id")
            .and_then(Value::as_str)
            .filter(|id| !id.trim().is_empty())
            .ok_or_else(|| {
                AgentError::Validation("cognee_propose_memory needs source_entry_id".into())
            })?;
        let verified_by = optional_string(&params, "verified_by")?;
        let supersedes = optional_string(&params, "supersedes")?;
        let valid_hours = optional_valid_hours(&params)?;
        if signal.is_cancelled() {
            return Err(AgentError::Tool("Cognee proposal cancelled".into()));
        }
        let runtime = self
            .runtime
            .upgrade()
            .ok_or_else(|| AgentError::Tool("Cognee is unavailable".into()))?;
        propose_memory(
            &runtime,
            text,
            category,
            source_entry_id,
            verified_by.as_deref(),
            supersedes.as_deref(),
            valid_hours,
            ProposalControl {
                signal,
                defer_review: params
                    .get("review")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            },
        )
        .await
        .map(AgentToolResult::text)
        .map_err(AgentError::Tool)
    }
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct AdmissionOutcome {
    pub(crate) candidate_id: String,
    pub(crate) operation_id: String,
    pub(crate) receipt_id: Option<String>,
    pub(crate) duplicate: bool,
}

pub(crate) fn accept_candidate(
    runtime: &CogneeRuntime,
    candidate: &Candidate,
) -> Result<Option<AdmissionOutcome>, String> {
    use super::admission::{self, AdmissionReceipt, SupersededTarget, RECEIPT_VERSION};
    use super::queue::PendingState;
    let record_text = format!("[{}] {}", candidate.category.as_str(), candidate.text);
    let config = runtime.config();
    if record_text.chars().count() > config.remember_max_chars {
        return Ok(None);
    }
    let candidate_dir = runtime.candidates_dir();
    let receipt_dir = admission::receipt_dir(&candidate_dir);
    let _admission_lock = admission::admission_lock(&receipt_dir)
        .map_err(|e| format!("Could not lock Cognee admission: {e}"))?;
    let mut committed = admission::read_receipts(&receipt_dir)
        .into_iter()
        .map(|receipt| receipt.candidate)
        .collect::<Vec<_>>();
    committed.extend(
        candidates::read_candidates(&candidate_dir)
            .into_iter()
            .filter(|stored| {
                stored.state == CandidateState::Accepted && stored.admission_id.is_none()
            }),
    );
    let superseded: std::collections::HashSet<_> = committed
        .iter()
        .filter(|record| {
            record.dataset == candidate.dataset
                && record.branch == candidate.branch
                && record
                    .valid_until_ms
                    .is_none_or(|expiry| expiry > runtime.now())
        })
        .filter_map(|record| record.supersedes.as_deref())
        .collect();
    let applicability = |record: &Candidate| {
        record.source_scope.as_ref().map(|scope| {
            (
                scope.git_branch.clone(),
                scope.git_revision.clone(),
                scope.worktree_id.clone(),
            )
        })
    };
    if let Some(stored) = committed.iter().find(|stored| {
        stored.dataset == candidate.dataset
            && stored.category == candidate.category
            && stored.branch == candidate.branch
            && stored.text == candidate.text
            && applicability(stored) == applicability(candidate)
            && stored
                .valid_until_ms
                .is_none_or(|expiry| expiry > runtime.now())
            && !superseded.contains(stored.id.as_str())
    }) {
        let operation_id = admission::read_receipts(&receipt_dir)
            .into_iter()
            .find(|receipt| receipt.candidate.id == stored.id)
            .map(|receipt| receipt.operation.operation_id)
            .unwrap_or_else(|| {
                super::queue::create_pending_remember(
                    &record_text,
                    &stored.dataset,
                    stored.category.node_set(),
                    stored.created_at,
                )
                .operation_id
            });
        return Ok(Some(AdmissionOutcome {
            candidate_id: stored.id.clone(),
            operation_id,
            receipt_id: stored.admission_id.clone(),
            duplicate: true,
        }));
    }
    let target = match candidate.supersedes.as_deref() {
        None => None,
        Some(id) => {
            let old = committed
                .iter()
                .find(|record| record.id == id)
                .ok_or("Supersession target is not committed")?;
            if old.state != CandidateState::Accepted
                || old.dataset != candidate.dataset
                || old.category != candidate.category
                || old.branch != candidate.branch
                || applicability(old) != applicability(candidate)
                || candidate.created_at < old.created_at
                || superseded.contains(id)
                || old.source.role != Role::User
                || candidate.source.role != Role::User
            {
                return Err("Supersession is not a newer user correction in the same scope".into());
            }
            Some(old.clone())
        }
    };
    let mut record = super::queue::create_pending_remember(
        &record_text,
        &candidate.dataset,
        candidate.category.node_set(),
        candidate.created_at,
    );
    let id = uuid::Uuid::new_v4().to_string();
    record.admission_id = Some(id.clone());
    record.state = PendingState::Prepared;
    let mut attributed = candidate.clone();
    attributed.id = candidate.id.clone();
    attributed.admission_id = Some(id.clone());
    attributed.state = CandidateState::Accepted;
    let superseded_target = target.map(|old| SupersededTarget {
        source: old.source.entry_id,
        scope: old.branch,
        hash: super::suppress::content_hash(&old.text),
    });
    let receipt = AdmissionReceipt {
        version: RECEIPT_VERSION,
        id: id.clone(),
        candidate: attributed.clone(),
        operation: record.clone(),
        superseded_target,
        committed_at: runtime.now(),
    };
    // A full queue is a hard failure; do not evict durable intent.
    match super::queue::reserve_prepared(&runtime.pending_dir(), &record, config.queue_limit) {
        Ok(true) => {}
        Ok(false) => return Ok(None),
        Err(error) => return Err(format!("Could not reserve Cognee operation: {error}")),
    }
    if let Err(error) = admission::commit(&receipt_dir, &receipt) {
        // Without a receipt nothing owns the reservation, and reconciliation
        // leaves a Prepared operation without one blocked and unreplayable.
        if !admission::read_receipts(&receipt_dir)
            .iter()
            .any(|existing| existing.id == id)
        {
            let _ =
                super::queue::remove_pending_remember(&runtime.pending_dir(), &record.operation_id);
        }
        return Err(format!("Admission receipt not committed: {error}"));
    }
    runtime.refresh_receipt_suppressions();
    // Everything after the atomic receipt commit is a recoverable projection.
    let _ = candidates::store_pending(&candidate_dir, &attributed, runtime.now());
    if super::queue::update_pending_remember_state(
        &runtime.pending_dir(),
        &record.operation_id,
        PendingState::Pending,
    )
    .is_err()
    {
        let _ = admission::reconcile(
            &candidate_dir,
            &runtime.pending_dir(),
            config.queue_limit,
            runtime.now(),
        );
    }
    runtime.observe_candidate_with_supersession(
        &attributed,
        "accepted",
        candidate.supersedes.as_ref().map(|_| "recorded"),
        None,
    );
    runtime.spawn_drain_pending();
    Ok(Some(AdmissionOutcome {
        candidate_id: attributed.id,
        operation_id: record.operation_id,
        receipt_id: Some(id),
        duplicate: false,
    }))
}

#[derive(Default)]
pub(crate) struct ProposalControl {
    pub(crate) signal: CancellationToken,
    pub(crate) defer_review: bool,
}

pub(crate) async fn propose_memory(
    runtime: &CogneeRuntime,
    text: &str,
    category: Category,
    source_entry_id: &str,
    verified_by: Option<&str>,
    supersedes: Option<&str>,
    valid_hours: Option<u64>,
    control: ProposalControl,
) -> Result<String, String> {
    runtime.ensure_loaded();
    if control.signal.is_cancelled() {
        return Err("Candidate operation was cancelled".into());
    }
    if text.trim().is_empty() || text.chars().count() > super::candidates::MAX_CANDIDATE_CHARS {
        return Err("Candidate fact is empty or exceeds the complete-record budget".into());
    }
    if super::text::redact_credential_text(text) != text {
        return Err("Sensitive candidate rejected without persistence".into());
    }
    let config = runtime.config();
    if !runtime.is_enabled() {
        return Err("Cognee is unavailable".into());
    }
    if !config.selective_writes() {
        return Err("Selective memory admission is unavailable".into());
    }
    let sources = runtime.candidate_source_snapshot(&control.signal).await?;
    let source = sources
        .messages
        .iter()
        .find(|(id, _)| id == source_entry_id)
        .map(|(_, message)| message)
        .ok_or_else(|| {
            "Source entry is not on the current bounded conversation branch".to_string()
        })?;
    let source_role = match source {
        AgentMessage::User(_) => Role::User,
        AgentMessage::Assistant(_) => Role::Assistant,
        AgentMessage::ToolResult(_) => Role::Tool,
        AgentMessage::Custom(_) => {
            return Err("Custom context is not an attributable source".into())
        }
    };
    let dataset = runtime.active_dataset();
    let now = runtime.now();
    let repository = sources.repository.clone();
    let volatile = matches!(category, Category::Outcome | Category::Lesson);
    let expiry = valid_hours
        .filter(|hours| (1..=MAX_VALID_HOURS).contains(hours))
        .map(|hours| now.saturating_add(hours.saturating_mul(3_600_000)))
        .or_else(|| volatile.then(|| now.saturating_add(86_400_000)));
    if valid_hours.is_some_and(|hours| !(1..=MAX_VALID_HOURS).contains(&hours)) {
        return Err(format!(
            "Candidate expiry must be 1..{MAX_VALID_HOURS} hours"
        ));
    }
    let mut candidate = Candidate {
        id: uuid::Uuid::new_v4().to_string(),
        admission_id: None,
        source_scope: None,
        category,
        text: text.trim().into(),
        source: Source {
            entry_id: source_entry_id.into(),
            role: source_role,
        },
        verified_by: verified_by.map(str::to_owned),
        supersedes: supersedes.map(str::to_owned),
        dataset,
        branch: if volatile {
            repository
                .as_ref()
                .and_then(|identity| identity.branch.clone())
        } else {
            None
        },
        valid_until_ms: expiry,
        created_at: now,
        state: CandidateState::Pending,
    };
    let evidence = match super::source_validation::validate_source(&candidate, &sources.messages) {
        Ok(evidence) => evidence,
        Err(reason) => {
            candidate.state = CandidateState::Rejected(reason.as_str().into());
            runtime.observe_candidate_with_supersession(&candidate, "rejected", None, None);
            return Err(format!("Candidate rejected: {}", reason.as_str()));
        }
    };
    candidate.source_scope = Some(super::candidates::SourceScope {
        session_id: sources.operation.host_session_id.clone(),
        task_id: sources.task_id.clone(),
        conversation_branch: Some(sources.messages.iter().map(|(id, _)| id.clone()).collect()),
        git_branch: if volatile {
            repository
                .as_ref()
                .and_then(|identity| identity.branch.clone())
        } else {
            None
        },
        git_revision: if volatile {
            repository
                .as_ref()
                .and_then(|identity| identity.revision.clone())
        } else {
            None
        },
        worktree_id: if volatile {
            repository
                .as_ref()
                .map(|identity| identity.worktree_id.clone())
        } else {
            None
        },
        source_text_hash: evidence.source_text_hash,
    });
    let original = ice_agent::message::original_user_prompt(source)
        .map(str::to_owned)
        .unwrap_or_else(|| extract_message_text(source));
    let explicit = original
        .trim_start()
        .to_ascii_lowercase()
        .starts_with("remember")
        || original
            .trim_start()
            .to_ascii_lowercase()
            .starts_with("please remember");
    if control.defer_review && !explicit {
        if !super::candidates::store_pending(&runtime.candidates_dir(), &candidate, now)
            .map_err(|_| "Candidate review storage unavailable".to_string())?
        {
            return Err("Candidate review queue is full; no fact was accepted".into());
        }
        runtime.observe_candidate_with_supersession(&candidate, "pending", None, None);
        return Ok(format!("pending:{}", candidate.id));
    }
    let outcome = runtime
        .accept_validated_candidate(&candidate, &control.signal, false)
        .await?
        .ok_or("Durable memory queue is full; no fact was accepted")?;
    Ok(format!(
        "accepted_queued:{}:{}",
        outcome.candidate_id,
        if outcome.duplicate {
            "duplicate"
        } else {
            "new"
        }
    ))
}

fn optional_string(params: &Value, name: &str) -> Result<Option<String>, AgentError> {
    match params.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value))
            if !value.trim().is_empty()
                && value.chars().count() <= 160
                && !value.chars().any(char::is_control)
                && (name != "supersedes"
                    || value
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || b"-_.:".contains(&byte))) =>
        {
            Ok(Some(value.clone()))
        }
        _ => Err(AgentError::Validation(format!(
            "{name} must be a valid nonempty string"
        ))),
    }
}

fn optional_valid_hours(params: &Value) -> Result<Option<u64>, AgentError> {
    match params.get("valid_hours") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(value)) => value
            .as_u64()
            .filter(|hours| (1..=MAX_VALID_HOURS).contains(hours))
            .map(Some)
            .ok_or_else(|| {
                AgentError::Validation(format!(
                    "valid_hours must be an integer from 1 to {MAX_VALID_HOURS}"
                ))
            }),
        Some(_) => Err(AgentError::Validation(format!(
            "valid_hours must be an integer from 1 to {MAX_VALID_HOURS}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_and_validator_share_the_expiry_bound() {
        let tool = CogneeProposeMemoryTool::new(Weak::new());
        let properties = &tool.schema().parameters.as_value()["properties"];
        assert_eq!(properties["valid_hours"]["maximum"], MAX_VALID_HOURS);
        assert_eq!(properties["verified_by"]["maxLength"], 160);
        assert_eq!(properties["review"]["type"], "boolean");
        assert_eq!(
            optional_valid_hours(&json!({"valid_hours": MAX_VALID_HOURS})).unwrap(),
            Some(MAX_VALID_HOURS)
        );
        assert!(optional_valid_hours(&json!({"valid_hours": MAX_VALID_HOURS + 1})).is_err());
    }
}
