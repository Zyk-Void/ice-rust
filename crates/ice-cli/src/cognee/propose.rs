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
                        "verified_by": {"type": "string"},
                        "supersedes": {"type": "string"},
                        "valid_hours": {"type": "integer", "minimum": 1, "maximum": 8760}
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

fn resolve_supersession<'a>(
    candidate: &Candidate,
    stored: &'a [Candidate],
) -> Result<&'a Candidate, &'static str> {
    let old_id = candidate.supersedes.as_deref().ok_or("unknown_target")?;
    let old = stored
        .iter()
        .find(|existing| existing.id == old_id)
        .ok_or("unknown_target")?;
    if old.state != CandidateState::Accepted {
        return Err("not_accepted");
    }
    if old.id == candidate.id || old.dataset != candidate.dataset || old.branch != candidate.branch
    {
        return Err("scope_mismatch");
    }
    if candidate.source.role != Role::User {
        return Err("role_not_allowed");
    }
    Ok(old)
}

fn record_accepted_suppressions(
    _runtime: &CogneeRuntime,
    candidate: &Candidate,
    candidate_dir: &std::path::Path,
    accepted: bool,
) -> Result<(Option<String>, Option<String>), String> {
    // Accepted correction/expiry effects are persisted in the admission receipt.
    // This helper now only reports whether a requested correction target resolves.
    let supersession = if candidate.supersedes.is_some() {
        let stored = candidates::read_candidates(candidate_dir);
        match resolve_supersession(candidate, &stored) {
            Ok(_) if accepted => Some(("recorded".to_string(), None)),
            Ok(_) => Some(("unresolved".to_string(), Some("not_accepted".into()))),
            Err(reason) => Some(("unresolved".to_string(), Some(reason.into()))),
        }
    } else {
        None
    };
    Ok(match supersession {
        Some((status, reason)) => (Some(status), reason),
        None => (None, None),
    })
}

fn optional_valid_hours(params: &Value) -> Result<Option<u64>, AgentError> {
    match params.get("valid_hours") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(value)) => value
            .as_u64()
            .filter(|hours| (1..=8760).contains(hours))
            .map(Some)
            .ok_or_else(|| {
                AgentError::Validation("valid_hours must be an integer from 1 to 8760".into())
            }),
        Some(_) => Err(AgentError::Validation(
            "valid_hours must be an integer from 1 to 8760".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(id: &str, state: CandidateState, dataset: &str, role: Role) -> Candidate {
        Candidate {
            id: id.into(),
            admission_id: None,
            source_scope: None,
            category: Category::Decision,
            text: id.into(),
            source: Source {
                entry_id: "source".into(),
                role,
            },
            verified_by: None,
            supersedes: None,
            dataset: dataset.into(),
            branch: None,
            valid_until_ms: None,
            created_at: 1,
            state,
        }
    }

    #[test]
    fn supersession_requires_accepted_same_scope_target_and_user_source() {
        let old = candidate("old", CandidateState::Accepted, "project-a", Role::User);
        let mut new = candidate("new", CandidateState::Accepted, "project-a", Role::User);
        new.supersedes = Some("old".into());
        assert!(resolve_supersession(&new, std::slice::from_ref(&old)).is_ok());
        assert_eq!(resolve_supersession(&new, &[]), Err("unknown_target"));

        let pending = candidate("old", CandidateState::Pending, "project-a", Role::User);
        assert_eq!(resolve_supersession(&new, &[pending]), Err("not_accepted"));
        let wrong_dataset = candidate("old", CandidateState::Accepted, "project-b", Role::User);
        assert_eq!(
            resolve_supersession(&new, &[wrong_dataset]),
            Err("scope_mismatch")
        );
        let mut wrong_branch = old.clone();
        wrong_branch.branch = Some("release".into());
        assert_eq!(
            resolve_supersession(&new, &[wrong_branch]),
            Err("scope_mismatch")
        );
        let rejected = candidate(
            "old",
            CandidateState::Rejected("wrong_role".into()),
            "project-a",
            Role::User,
        );
        assert_eq!(resolve_supersession(&new, &[rejected]), Err("not_accepted"));

        let mut old_constraint = old.clone();
        old_constraint.category = Category::Constraint;
        let mut assistant_outcome = new;
        assistant_outcome.category = Category::Outcome;
        assistant_outcome.source.role = Role::Assistant;
        assert_eq!(
            resolve_supersession(&assistant_outcome, std::slice::from_ref(&old_constraint)),
            Err("role_not_allowed")
        );
    }

    #[test]
    fn valid_hours_requires_an_integer_in_the_supported_range() {
        for value in [json!(0), json!(8761), json!(-1), json!(1.5), json!("2")] {
            assert!(optional_valid_hours(&json!({"valid_hours": value})).is_err());
        }
        assert_eq!(
            optional_valid_hours(&json!({"valid_hours": 1})).unwrap(),
            Some(1)
        );
        assert_eq!(optional_valid_hours(&json!({})).unwrap(), None);
    }
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

pub(crate) fn accept_candidate(
    runtime: &CogneeRuntime,
    candidate: &Candidate,
) -> Result<bool, String> {
    use super::admission::{self, AdmissionReceipt, SupersededTarget, RECEIPT_VERSION};
    use super::queue::PendingState;
    let record_text = format!("[{}] {}", candidate.category.as_str(), candidate.text);
    let config = runtime.config();
    if record_text.chars().count() > config.remember_max_chars {
        return Ok(false);
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
    if committed.into_iter().any(|stored| {
        stored.dataset == candidate.dataset
            && stored.category == candidate.category
            && stored.branch == candidate.branch
            && stored.text == candidate.text
            && stored
                .valid_until_ms
                .is_none_or(|expiry| expiry > runtime.now())
    }) {
        // Existing durable intent is canonical; do not manufacture a new write.
        return Ok(true);
    }
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
    let target = candidate.supersedes.as_ref().and_then(|target_id| {
        candidates::read_candidates(&candidate_dir)
            .into_iter()
            .find(|c| &c.id == target_id)
    });
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
        Ok(false) => return Ok(false),
        Err(error) => return Err(format!("Could not reserve Cognee operation: {error}")),
    }
    admission::commit(&receipt_dir, &receipt)
        .map_err(|e| format!("Admission receipt not committed: {e}"))?;
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
    runtime.spawn_drain_pending();
    Ok(true)
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
    let sources = runtime.candidate_source_messages(&control.signal).await?;
    let source = sources
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
    let repository = runtime.repository_identity();
    let volatile = matches!(category, Category::Outcome | Category::Lesson);
    let expiry = valid_hours
        .filter(|hours| (1..=720).contains(hours))
        .map(|hours| now.saturating_add(hours.saturating_mul(3_600_000)))
        .or_else(|| volatile.then(|| now.saturating_add(86_400_000)));
    if valid_hours.is_some_and(|hours| !(1..=720).contains(&hours)) {
        return Err("Candidate expiry must be 1..720 hours".into());
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
    if let Err(reason) = super::source_validation::validate_source(&candidate, &sources) {
        candidate.state = CandidateState::Rejected(reason.as_str().into());
        runtime.observe_candidate_with_supersession(&candidate, "rejected", None, None);
        return Err(format!("Candidate rejected: {}", reason.as_str()));
    }
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
    let admitted = runtime
        .accept_validated_candidate(&candidate, &control.signal, false)
        .await?;
    if !admitted {
        return Err("Durable memory queue is full; no fact was accepted".into());
    }
    candidate.state = CandidateState::Accepted;

    Ok("accepted_queued".into())
}
