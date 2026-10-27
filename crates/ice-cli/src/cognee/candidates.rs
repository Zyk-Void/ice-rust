//! Durable, attributable memory candidates for selective writes.

use std::path::{Path, PathBuf};

use ice_agent::AgentMessage;
use serde::{Deserialize, Serialize};

use super::config::{create_private_dir, write_private_file_atomic};
use super::evidence::is_routine_success_line;

pub const MAX_PENDING_CANDIDATES: usize = 32;
pub const CANDIDATE_TTL_MS: u64 = 24 * 60 * 60 * 1000;
pub const MAX_CANDIDATE_CHARS: usize = 600;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Constraint,
    Decision,
    Preference,
    Lesson,
    Outcome,
}

impl Category {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Constraint => "constraint",
            Self::Decision => "decision",
            Self::Preference => "preference",
            Self::Lesson => "lesson",
            Self::Outcome => "outcome",
        }
    }

    pub fn node_set(self) -> &'static str {
        match self {
            Self::Constraint | Self::Preference => "user_context",
            Self::Decision => "project_docs",
            Self::Lesson | Self::Outcome => "agent_actions",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "constraint" => Some(Self::Constraint),
            "decision" => Some(Self::Decision),
            "preference" => Some(Self::Preference),
            "lesson" => Some(Self::Lesson),
            "outcome" => Some(Self::Outcome),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateState {
    Pending,
    Accepted,
    Rejected(String),
    Expired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub entry_id: String,
    pub role: Role,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub id: String,
    pub category: Category,
    pub text: String,
    pub source: Source,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verified_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersedes: Option<String>,
    pub dataset: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_until_ms: Option<u64>,
    pub created_at: u64,
    pub state: CandidateState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    Invalid,
    UnknownSource,
    NotInSource,
    Unverified,
    WrongRole,
    Routine,
    QueueFull,
    Unavailable,
}

impl Reason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Invalid => "invalid",
            Self::UnknownSource => "unknown_source",
            Self::NotInSource => "not_in_source",
            Self::Unverified => "unverified",
            Self::WrongRole => "wrong_role",
            Self::Routine => "routine",
            Self::QueueFull => "queue_full",
            Self::Unavailable => "unavailable",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission {
    Accept,
    Reject(Reason),
    Pending,
}

pub fn admit(candidate: &Candidate, branch: &[(String, AgentMessage)]) -> Admission {
    if candidate.text.trim().is_empty() || candidate.text.chars().count() > MAX_CANDIDATE_CHARS {
        return Admission::Reject(Reason::Invalid);
    }
    let Some((_, source)) = branch
        .iter()
        .find(|(entry_id, _)| entry_id == &candidate.source.entry_id)
    else {
        return Admission::Reject(Reason::UnknownSource);
    };
    if candidate.category == Category::Decision && candidate.source.role != Role::User {
        return Admission::Reject(Reason::WrongRole);
    }
    let source_role = match source {
        AgentMessage::User(_) => Role::User,
        AgentMessage::Assistant(_) => Role::Assistant,
        AgentMessage::ToolResult(_) => Role::Tool,
        AgentMessage::Custom(_) => return Admission::Reject(Reason::WrongRole),
    };
    if source_role != candidate.source.role {
        return Admission::Reject(Reason::WrongRole);
    }
    if is_routine_success_line(&candidate.text) {
        return Admission::Reject(Reason::Routine);
    }

    match candidate.category {
        Category::Constraint | Category::Preference | Category::Decision => {
            if source_role != Role::User {
                return Admission::Reject(Reason::WrongRole);
            }
            let text = message_text(source);
            let needle = normalize_whitespace(&candidate.text);
            if normalize_whitespace(&text).contains(&needle) {
                Admission::Accept
            } else if candidate.text.chars().count() <= 300 && {
                use std::collections::HashSet;
                let tokens = |value: &str| {
                    value
                        .to_lowercase()
                        .split(|character: char| !character.is_alphanumeric())
                        .filter(|token| token.chars().count() >= 3)
                        .map(str::to_owned)
                        .collect::<HashSet<_>>()
                };
                let candidate_tokens = tokens(&candidate.text);
                !candidate_tokens.is_empty()
                    && candidate_tokens.intersection(&tokens(&text)).count() * 100
                        >= candidate_tokens.len() * 70
            } {
                Admission::Pending
            } else {
                Admission::Reject(Reason::NotInSource)
            }
        }
        Category::Lesson | Category::Outcome => {
            if source_role != Role::Assistant {
                return Admission::Reject(Reason::WrongRole);
            }
            let Some(verified_by) = candidate.verified_by.as_deref() else {
                return Admission::Reject(Reason::Unverified);
            };
            let Some((_, AgentMessage::ToolResult(result))) =
                branch.iter().find(|(entry_id, _)| entry_id == verified_by)
            else {
                return Admission::Reject(Reason::Unverified);
            };
            if result.is_error {
                return Admission::Reject(Reason::Unverified);
            }
            let needle = normalize_whitespace(&candidate.text);
            if normalize_whitespace(&message_text(source)).contains(&needle) {
                Admission::Accept
            } else {
                Admission::Reject(Reason::NotInSource)
            }
        }
    }
}

fn normalize_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn message_text(message: &AgentMessage) -> String {
    match message {
        AgentMessage::User(message) => match &message.content {
            ice_ai::types::UserContent::Text(text) => text.clone(),
            ice_ai::types::UserContent::Blocks(blocks) => blocks
                .iter()
                .filter_map(|block| match block {
                    ice_ai::types::Content::Text(text) => Some(text.text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join(" "),
        },
        AgentMessage::Assistant(message) => message
            .content
            .iter()
            .filter_map(|block| match block {
                ice_ai::types::Content::Text(text) => Some(text.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(" "),
        AgentMessage::ToolResult(message) => message
            .content
            .iter()
            .filter_map(|block| match block {
                ice_ai::types::Content::Text(text) => Some(text.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(" "),
        AgentMessage::Custom(_) => String::new(),
    }
}

fn candidate_path(directory: &Path, id: &str) -> std::io::Result<PathBuf> {
    if id.is_empty()
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() || byte == b'-')
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Invalid candidate ID",
        ));
    }
    Ok(directory.join(format!("{id}.json")))
}

pub fn read_candidates(directory: &Path) -> Vec<Candidate> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut candidates = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
        .filter_map(|text| serde_json::from_str::<Candidate>(&text).ok())
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| left.created_at.cmp(&right.created_at));
    candidates
}

pub fn store_pending(
    directory: &Path,
    candidate: &Candidate,
    now_ms: u64,
) -> std::io::Result<bool> {
    let mut candidates = read_candidates(directory);
    for existing in candidates.iter_mut() {
        if existing.state == CandidateState::Pending
            && now_ms.saturating_sub(existing.created_at) >= CANDIDATE_TTL_MS
        {
            existing.state = CandidateState::Expired;
            let path = candidate_path(directory, &existing.id)?;
            write_private_file_atomic(
                &path,
                &serde_json::to_string(existing).expect("candidate serializes"),
            )?;
        }
    }
    if candidates
        .iter()
        .filter(|item| item.state == CandidateState::Pending)
        .count()
        >= MAX_PENDING_CANDIDATES
    {
        return Ok(false);
    }
    create_private_dir(directory)?;
    let path = candidate_path(directory, &candidate.id)?;
    write_private_file_atomic(
        &path,
        &serde_json::to_string(candidate).expect("candidate serializes"),
    )?;
    Ok(true)
}

pub fn set_state(
    directory: &Path,
    candidate: &mut Candidate,
    state: CandidateState,
) -> std::io::Result<()> {
    candidate.state = state;
    let path = candidate_path(directory, &candidate.id)?;
    write_private_file_atomic(
        &path,
        &serde_json::to_string(candidate).expect("candidate serializes"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ice_ai::types::{Api, AssistantMessage, ToolResultMessage, UserMessage};

    fn assistant(text: &str) -> AgentMessage {
        let mut message = AssistantMessage::empty(Api::AnthropicMessages, "test", "test", 1);
        message.content.push(ice_ai::types::Content::text(text));
        AgentMessage::Assistant(Box::new(message))
    }

    fn candidate(category: Category, text: &str, role: Role, source: &str) -> Candidate {
        Candidate {
            id: uuid::Uuid::new_v4().to_string(),
            category,
            text: text.into(),
            source: Source {
                entry_id: source.into(),
                role,
            },
            verified_by: None,
            supersedes: None,
            dataset: "test".into(),
            branch: None,
            valid_until_ms: None,
            created_at: 1,
            state: CandidateState::Pending,
        }
    }

    #[test]
    fn user_quotes_are_required_and_proposals_are_not_decisions() {
        let user = AgentMessage::User(UserMessage::new(
            "Please keep generated files out of source control.",
            1,
        ));
        let branch = vec![("user-1".into(), user)];
        let quoted = candidate(
            Category::Constraint,
            "keep generated files out of source control.",
            Role::User,
            "user-1",
        );
        assert_eq!(admit(&quoted, &branch), Admission::Accept);
        let fabricated = candidate(Category::Constraint, "anything", Role::User, "no-such-id");
        assert_eq!(
            admit(&fabricated, &branch),
            Admission::Reject(Reason::UnknownSource)
        );
        let paraphrase = candidate(
            Category::Preference,
            "keep generated files outside source control",
            Role::User,
            "user-1",
        );
        assert_eq!(admit(&paraphrase, &branch), Admission::Pending);
        let unrelated = candidate(
            Category::Preference,
            "Use a different language for database work.",
            Role::User,
            "user-1",
        );
        assert_eq!(
            admit(&unrelated, &branch),
            Admission::Reject(Reason::NotInSource)
        );

        let assistant_proposal = assistant("We should use SQLite.");
        let branch = vec![("assistant-1".into(), assistant_proposal)];
        let decision = candidate(
            Category::Decision,
            "We should use SQLite.",
            Role::Assistant,
            "assistant-1",
        );
        assert_eq!(
            admit(&decision, &branch),
            Admission::Reject(Reason::WrongRole)
        );
    }

    #[test]
    fn outcomes_require_successful_verification_and_text_source() {
        let assistant = assistant("The export completed and can be reused.");
        let success = AgentMessage::ToolResult(Box::new(ToolResultMessage {
            role: Default::default(),
            tool_call_id: "call-1".into(),
            tool_name: "bash".into(),
            content: vec![ice_ai::types::Content::text("done")],
            details: None,
            usage: None,
            added_tool_names: Vec::new(),
            is_error: false,
            timestamp: 2,
        }));
        let branch = vec![
            ("assistant-1".into(), assistant.clone()),
            ("tool-1".into(), success),
        ];
        let mut outcome = candidate(
            Category::Outcome,
            "export completed",
            Role::Assistant,
            "assistant-1",
        );
        assert_eq!(
            admit(&outcome, &branch),
            Admission::Reject(Reason::Unverified)
        );
        outcome.verified_by = Some("tool-1".into());
        assert_eq!(admit(&outcome, &branch), Admission::Accept);
        outcome.text = "made up outcome".into();
        assert_eq!(
            admit(&outcome, &branch),
            Admission::Reject(Reason::NotInSource)
        );

        if let AgentMessage::ToolResult(result) = &branch[1].1 {
            let mut failure = (**result).clone();
            failure.is_error = true;
            let error_branch = vec![
                ("assistant-1".into(), assistant),
                ("tool-1".into(), AgentMessage::ToolResult(Box::new(failure))),
            ];
            outcome.text = "export completed".into();
            assert_eq!(
                admit(&outcome, &error_branch),
                Admission::Reject(Reason::Unverified)
            );
        }
    }

    #[test]
    fn rejects_routine_and_invalid_candidates() {
        let branch = vec![(
            "user-1".into(),
            AgentMessage::User(UserMessage::new("bash succeeded", 1)),
        )];
        let routine = candidate(Category::Constraint, "bash succeeded", Role::User, "user-1");
        assert_eq!(admit(&routine, &branch), Admission::Reject(Reason::Routine));
        let empty = candidate(Category::Constraint, " ", Role::User, "user-1");
        assert_eq!(admit(&empty, &branch), Admission::Reject(Reason::Invalid));
        let long = candidate(Category::Constraint, &"x".repeat(601), Role::User, "user-1");
        assert_eq!(admit(&long, &branch), Admission::Reject(Reason::Invalid));
    }

    #[test]
    fn pending_candidates_expire_and_are_bounded() {
        let directory = tempfile::tempdir().unwrap();
        for index in 0..MAX_PENDING_CANDIDATES {
            let mut item = candidate(Category::Lesson, "candidate", Role::Assistant, "assistant");
            item.id = format!("{index:032x}");
            item.created_at = 100;
            assert!(store_pending(directory.path(), &item, 100).unwrap());
        }
        let mut overflow = candidate(Category::Lesson, "overflow", Role::Assistant, "assistant");
        overflow.id = "ffffffffffffffffffffffffffffffff".into();
        assert!(!store_pending(directory.path(), &overflow, 100).unwrap());
        assert!(store_pending(directory.path(), &overflow, 100 + CANDIDATE_TTL_MS).unwrap());
        let records = read_candidates(directory.path());
        assert_eq!(
            records
                .iter()
                .filter(|item| item.state == CandidateState::Pending)
                .count(),
            1
        );
        assert_eq!(
            records
                .iter()
                .filter(|item| item.state == CandidateState::Expired)
                .count(),
            MAX_PENDING_CANDIDATES
        );
    }
}
