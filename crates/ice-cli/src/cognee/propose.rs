//! The selective Cognee proposal tool and host-side attribution checks.

use std::sync::{Arc, Weak};

use async_trait::async_trait;
use ice_agent::agent_tool::AgentTool;
use ice_agent::error::AgentError;
use ice_agent::types::{AgentToolResult, ToolExecutionMode, ToolResultPartial};
use ice_ai::types::Tool;
use ice_harness::session::types::{BranchBounds, Entry, EntryQuery};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use super::candidates::{
    self, Admission, Candidate, CandidateState, Category, Reason, Role, Source,
};
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
        _signal: CancellationToken,
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
        let runtime = self
            .runtime
            .upgrade()
            .ok_or_else(|| AgentError::Tool("Cognee is unavailable".into()))?;
        runtime
            .propose_memory_with_expiry(
                text,
                category,
                source_entry_id,
                verified_by.as_deref(),
                supersedes.as_deref(),
                valid_hours,
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
    runtime: &CogneeRuntime,
    candidate: &Candidate,
    candidate_dir: &std::path::Path,
    accepted: bool,
) -> Result<(Option<String>, Option<String>), String> {
    use super::suppress::{content_hash, Suppression, SuppressionKind};

    let supersession = if candidate.supersedes.is_some() {
        let stored = candidates::read_candidates(candidate_dir);
        match resolve_supersession(candidate, &stored) {
            Ok(old) if accepted => {
                runtime.record_suppression(Suppression {
                    id: uuid::Uuid::new_v4().to_string(),
                    hash: content_hash(&old.text),
                    kind: SuppressionKind::Superseded {
                        by: candidate.id.clone(),
                    },
                    dataset: old.dataset.clone(),
                    branch: old.branch.clone(),
                    recorded_at_ms: candidate.created_at,
                })?;
                Some(("recorded".to_string(), None))
            }
            Ok(_) => Some(("unresolved".to_string(), Some("not_accepted".into()))),
            Err(reason) => Some(("unresolved".to_string(), Some(reason.into()))),
        }
    } else {
        None
    };
    if !accepted {
        return Ok(match supersession {
            Some((status, reason)) => (Some(status), reason),
            None => (None, None),
        });
    }

    if let Some(at_ms) = candidate.valid_until_ms {
        runtime.record_suppression(Suppression {
            id: uuid::Uuid::new_v4().to_string(),
            hash: content_hash(&candidate.text),
            kind: SuppressionKind::Expires { at_ms },
            dataset: candidate.dataset.clone(),
            branch: candidate.branch.clone(),
            recorded_at_ms: candidate.created_at,
        })?;
    }

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
    let mut record_text = format!("[{}] {}", candidate.category.as_str(), candidate.text);
    let config = runtime.config();
    if record_text.chars().count() > config.remember_max_chars {
        record_text = record_text
            .chars()
            .take(config.remember_max_chars)
            .collect();
    }
    let record = super::queue::create_pending_remember(
        &record_text,
        &candidate.dataset,
        candidate.category.node_set(),
        candidate.created_at,
    );
    let was_pending = super::queue::read_pending_remember(&runtime.pending_dir())
        .iter()
        .any(|item| item.operation_id == record.operation_id);
    match super::queue::enqueue_pending_remember(
        &runtime.pending_dir(),
        &record,
        config.queue_limit,
    ) {
        Ok(true) => {
            if !was_pending {
                runtime.spawn_drain_pending();
            }
            Ok(true)
        }
        Ok(false) => Ok(false),
        Err(error) => Err(format!("Could not queue candidate: {error}")),
    }
}

pub(crate) async fn propose_memory(
    runtime: &CogneeRuntime,
    text: &str,
    category: Category,
    source_entry_id: &str,
    verified_by: Option<&str>,
    supersedes: Option<&str>,
    valid_hours: Option<u64>,
) -> Result<String, String> {
    runtime.ensure_loaded();
    let (selective, dataset) = {
        let state = runtime.state();
        (state.config.selective_writes(), state.active_dataset())
    };
    if !selective || !runtime.is_enabled() || runtime.live_client().is_none() {
        return Err("Cognee is unavailable".into());
    }
    let harness = runtime
        .harness()
        .ok_or_else(|| "Cognee is unavailable".to_string())?;
    let entries = harness
        .session()
        .find_entries_on_branch(&EntryQuery::default(), &BranchBounds::default())
        .await
        .map_err(|_| "Could not read the current conversation".to_string())?;
    let branch = entries
        .iter()
        .filter_map(|entry| match entry {
            Entry::Message(message) => Some((entry.id().to_string(), message.message.clone())),
            _ => None,
        })
        .collect::<Vec<_>>();
    let created_at = runtime.now();
    let mut candidate = Candidate {
        id: uuid::Uuid::new_v4().to_string(),
        category,
        text: text.trim().to_string(),
        source: Source {
            entry_id: source_entry_id.into(),
            role: branch
                .iter()
                .find(|(entry_id, _)| entry_id == source_entry_id)
                .map(|(_, message)| match message {
                    ice_agent::AgentMessage::User(_) => Role::User,
                    ice_agent::AgentMessage::Assistant(_) => Role::Assistant,
                    ice_agent::AgentMessage::ToolResult(_) => Role::Tool,
                    ice_agent::AgentMessage::Custom(_) => Role::Assistant,
                })
                .unwrap_or(Role::User),
        },
        verified_by: verified_by.map(str::to_owned),
        supersedes: supersedes.map(str::to_owned),
        dataset: dataset.clone(),
        branch: None,
        valid_until_ms: valid_hours
            .map(|hours| created_at.saturating_add(hours.saturating_mul(60 * 60 * 1000))),
        created_at,
        state: CandidateState::Pending,
    };
    let admission = candidates::admit(&candidate, &branch);
    candidate.text =
        super::text::redact_memory_text(&candidate.text, candidates::MAX_CANDIDATE_CHARS);
    if admission == Admission::Accept
        && candidates::read_candidates(&runtime.candidates_dir())
            .iter()
            .any(|existing| {
                existing.state == CandidateState::Accepted
                    && existing.dataset == candidate.dataset
                    && existing.category == candidate.category
                    && existing.text == candidate.text
            })
    {
        candidate.state = CandidateState::Accepted;
        candidates::store_pending(&runtime.candidates_dir(), &candidate, runtime.now())
            .map_err(|error| format!("Could not store candidate: {error}"))?;
        let (supersession, supersession_reason) =
            record_accepted_suppressions(runtime, &candidate, &runtime.candidates_dir(), true)?;
        runtime.observe_candidate_with_supersession(
            &candidate,
            "accepted",
            supersession.as_deref(),
            supersession_reason.as_deref(),
        );
        return Ok("accepted_queued".into());
    }
    let outcome = match admission {
        Admission::Accept => match accept_candidate(runtime, &candidate) {
            Ok(true) => {
                candidate.state = CandidateState::Accepted;
                "accepted"
            }
            Ok(false) => {
                candidate.state = CandidateState::Rejected(Reason::QueueFull.as_str().into());
                "rejected"
            }
            Err(error) => return Err(error),
        },
        Admission::Reject(reason) => {
            candidate.state = CandidateState::Rejected(reason.as_str().into());
            "rejected"
        }
        Admission::Pending => "pending",
    };
    let candidate_dir = runtime.candidates_dir();
    if let Err(error) = candidates::store_pending(&candidate_dir, &candidate, runtime.now()) {
        if outcome == "accepted" {
            return Err(format!(
                "Candidate queued, but candidate audit record failed: {error}"
            ));
        }
        return Err(format!("Could not store candidate: {error}"));
    }
    let (supersession, supersession_reason) =
        record_accepted_suppressions(runtime, &candidate, &candidate_dir, outcome == "accepted")?;
    runtime.observe_candidate_with_supersession(
        &candidate,
        outcome,
        supersession.as_deref(),
        supersession_reason.as_deref(),
    );
    Ok(match outcome {
        "accepted" => "accepted_queued".into(),
        "rejected" => format!(
            "rejected:{}",
            match &candidate.state {
                CandidateState::Rejected(reason) => reason.as_str(),
                _ => "invalid",
            }
        ),
        _ => "pending".into(),
    })
}
