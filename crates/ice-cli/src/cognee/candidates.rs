//! Durable, attributable memory candidates for selective writes.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use fs2::FileExt;

use ice_agent::AgentMessage;
use serde::{Deserialize, Serialize};

use super::client::RecallResult;
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
pub struct SourceScope {
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    pub conversation_branch: Option<Vec<String>>,
    pub git_branch: Option<String>,
    pub git_revision: Option<String>,
    pub worktree_id: Option<String>,
    pub source_text_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub admission_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_scope: Option<SourceScope>,
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
            // A successful tool call is not evidence for an arbitrary claim.
            // Require both the assistant source and this verifier output to
            // contain the exact normalized claim.
            let verifier_text = message_text(&AgentMessage::ToolResult(result.clone()));
            if normalize_whitespace(&message_text(source)).contains(&needle)
                && normalize_whitespace(&verifier_text).contains(&needle)
            {
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

fn candidate_lock(directory: &Path) -> std::io::Result<File> {
    create_private_dir(directory)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(directory.join(".candidates.lock"))?;
    lock.lock_exclusive()?;
    Ok(lock)
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
    create_private_dir(directory)?;
    let lock = candidate_lock(directory)?;
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
    if candidate.state == CandidateState::Pending
        && candidates
            .iter()
            .filter(|item| item.state == CandidateState::Pending)
            .count()
            >= MAX_PENDING_CANDIDATES
    {
        return Ok(false);
    }
    create_private_dir(directory)?;
    let path = candidate_path(directory, &candidate.id)?;
    let result = if path.exists() {
        let stored = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str::<Candidate>(&text).ok());
        if stored.is_none() {
            let _ = FileExt::unlock(&lock);
            return Ok(false);
        }
        match stored {
            // A proposal/helper may not replace the admitted record with the
            // stale Pending snapshot it carried across an await.
            Some(stored) if stored.admission_id.is_some() && candidate.admission_id.is_none() => {
                Ok(false)
            }
            Some(stored) if stored.state != CandidateState::Pending => Ok(false),
            _ => write_private_file_atomic(
                &path,
                &serde_json::to_string(candidate).expect("candidate serializes"),
            )
            .map(|()| true),
        }
    } else {
        write_private_file_atomic(
            &path,
            &serde_json::to_string(candidate).expect("candidate serializes"),
        )
        .map(|()| true)
    };
    let _ = FileExt::unlock(&lock);
    result
}

/// Read accepted local curated evidence without a network request. Call at
/// admission/lifecycle boundaries and retain the returned results in a cache.
pub fn query_accepted(
    directory: &Path,
    dataset: &str,
    branch: Option<&str>,
    query: &str,
    now_ms: u64,
    limit: usize,
) -> Vec<RecallResult> {
    let needle = query.to_lowercase();
    let mut records = read_candidates(directory)
        .into_iter()
        .filter(|candidate| {
            candidate.state == CandidateState::Accepted && candidate.admission_id.is_none()
        })
        .collect::<Vec<_>>();
    let receipts = super::admission::read_receipts(&super::admission::receipt_dir(directory));
    records.extend(receipts.iter().map(|receipt| receipt.candidate.clone()));
    let mut superseded = records
        .iter()
        .filter(|correction| {
            correction.admission_id.is_none()
                && correction.dataset == dataset
                && correction
                    .valid_until_ms
                    .is_none_or(|expiry| expiry > now_ms)
                && correction
                    .branch
                    .as_deref()
                    .is_none_or(|scope| Some(scope) == branch)
                && correction.source.role == Role::User
                && correction.category == Category::Constraint
                && correction.supersedes.is_some()
        })
        .filter_map(|correction| correction.supersedes.clone())
        .collect::<std::collections::HashSet<_>>();
    superseded.extend(receipts.iter().filter_map(|receipt| {
        let correction = &receipt.candidate;
        let target = receipt.superseded_target.as_ref()?;
        (correction.dataset == dataset
            && correction
                .valid_until_ms
                .is_none_or(|expiry| expiry > now_ms)
            && correction
                .branch
                .as_deref()
                .is_none_or(|scope| Some(scope) == branch)
            && correction.source.role == Role::User
            && correction.category == Category::Constraint)
            .then(|| correction.supersedes.clone())
            .flatten()
            .filter(|_| target.hash.len() == 64)
    }));
    records
        .into_iter()
        .filter(|candidate| {
            candidate.dataset == dataset
                && candidate
                    .valid_until_ms
                    .is_none_or(|expiry| expiry > now_ms)
                && !superseded.contains(&candidate.id)
                && candidate
                    .branch
                    .as_deref()
                    .is_none_or(|scope| Some(scope) == branch)
                && (needle.is_empty() || candidate.text.to_lowercase().contains(&needle))
        })
        .take(limit.min(MAX_PENDING_CANDIDATES * 4))
        .map(|candidate| RecallResult {
            text: candidate.text,
            score: None,
            metadata: Some(serde_json::json!({
                "source": "local_curated_candidate",
                "candidateId": candidate.id,
                "candidate_id": candidate.id,
                "sourceEntryId": candidate.source.entry_id,
                "source_entry_id": candidate.source.entry_id,
                "source_id": candidate.source.entry_id,
                "sourceRole": candidate.source.role,
                "source_role": candidate.source.role,
                "category": candidate.category,
                "dataset": candidate.dataset,
                "branch": candidate.branch,
                "admission_id": candidate.admission_id,
                "source_scope": candidate.source_scope.as_ref().map(|scope| serde_json::json!({
                    "session_id": scope.session_id,
                    "conversation_branch": scope.conversation_branch,
                    "git_branch": scope.git_branch,
                    "git_revision": scope.git_revision,
                    "worktree_id": scope.worktree_id,
                    "source_text_hash": scope.source_text_hash,
                })),
                "validUntilMs": candidate.valid_until_ms,
                "valid_until_ms": candidate.valid_until_ms,
                "supersedes": candidate.supersedes,
                "createdAt": candidate.created_at,
                "created_at": candidate.created_at,
            })),
        })
        .collect()
}

pub fn set_state(
    directory: &Path,
    candidate: &mut Candidate,
    state: CandidateState,
) -> std::io::Result<()> {
    let lock = candidate_lock(directory)?;
    let path = candidate_path(directory, &candidate.id)?;
    let mut stored: Candidate =
        serde_json::from_str(&std::fs::read_to_string(&path)?).map_err(std::io::Error::other)?;
    if stored.id != candidate.id {
        let _ = FileExt::unlock(&lock);
        return Err(std::io::Error::other("candidate identity mismatch"));
    }
    // A delayed helper result or stale proposal cannot reopen a terminal
    // record. The caller's source/text snapshot is never written back.
    if stored.state != CandidateState::Pending {
        let _ = FileExt::unlock(&lock);
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "candidate is already terminal",
        ));
    }
    stored.state = state.clone();
    let result = write_private_file_atomic(
        &path,
        &serde_json::to_string(&stored).expect("candidate serializes"),
    );
    if result.is_ok() {
        candidate.state = state;
    }
    let _ = FileExt::unlock(&lock);
    result
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
            admission_id: None,
            source_scope: None,
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
            content: vec![ice_ai::types::Content::text("export completed")],
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
        outcome.text = "unrelated claim".into();
        assert_eq!(
            admit(&outcome, &branch),
            Admission::Reject(Reason::NotInSource)
        );
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
    fn stale_helper_result_cannot_overwrite_admitted_candidate() {
        let directory = tempfile::tempdir().unwrap();
        let mut candidate = candidate(Category::Lesson, "fact", Role::Assistant, "assistant");
        candidate.id = "00000000-0000-4000-8000-000000000001".into();
        store_pending(directory.path(), &candidate, 1).unwrap();
        let mut admitted = candidate.clone();
        admitted.admission_id = Some("admission-1".into());
        admitted.state = CandidateState::Accepted;
        write_private_file_atomic(
            &candidate_path(directory.path(), &candidate.id).unwrap(),
            &serde_json::to_string(&admitted).unwrap(),
        )
        .unwrap();

        assert!(!store_pending(directory.path(), &candidate, 2).unwrap());
        let mut stale = candidate;
        assert_eq!(
            set_state(
                directory.path(),
                &mut stale,
                CandidateState::Rejected("late_helper_result".into()),
            )
            .unwrap_err()
            .kind(),
            std::io::ErrorKind::AlreadyExists
        );
        let stored = read_candidates(directory.path()).pop().unwrap();
        assert_eq!(stored.state, CandidateState::Accepted);
        assert_eq!(stored.admission_id.as_deref(), Some("admission-1"));
    }

    #[test]
    fn terminal_state_update_reports_conflict_without_mutating_snapshot() {
        let directory = tempfile::tempdir().unwrap();
        let mut stored = candidate(
            Category::Lesson,
            "committed fact",
            Role::Assistant,
            "assistant",
        );
        stored.state = CandidateState::Accepted;
        store_pending(directory.path(), &stored, 1).unwrap();
        let mut stale = stored.clone();
        stale.state = CandidateState::Pending;
        let result = set_state(
            directory.path(),
            &mut stale,
            CandidateState::Rejected("late_rejection".into()),
        );
        assert_eq!(
            result.unwrap_err().kind(),
            std::io::ErrorKind::AlreadyExists
        );
        assert_eq!(stale.state, CandidateState::Pending);
        assert_eq!(
            read_candidates(directory.path())[0].state,
            CandidateState::Accepted
        );
    }

    // Invoked only by the exact-filtered parent test below. The sentinel keeps
    // normal test runs from doing any child-process work.
    #[test]
    #[ignore]
    fn candidate_lock_child() {
        let Ok(directory) = std::env::var("ICERS_CANDIDATE_LOCK_CHILD") else {
            return;
        };
        let mut candidate = candidate(
            Category::Lesson,
            "child candidate",
            Role::Assistant,
            "assistant",
        );
        candidate.id = "ffffffffffffffffffffffffffffffff".into();
        assert!(!store_pending(Path::new(&directory), &candidate, 10).unwrap());
    }

    #[test]
    fn candidate_capacity_lock_blocks_another_process() {
        use std::process::{Command, Stdio};
        use std::time::{Duration, Instant};

        let directory = tempfile::tempdir().unwrap();
        for index in 0..MAX_PENDING_CANDIDATES {
            let mut item = candidate(
                Category::Lesson,
                "capacity occupant",
                Role::Assistant,
                "assistant",
            );
            item.id = format!("{index:032x}");
            assert!(store_pending(directory.path(), &item, 10).unwrap());
        }
        let held = candidate_lock(directory.path()).unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "cognee::candidates::tests::candidate_lock_child",
                "--ignored",
                "--nocapture",
            ])
            .env("ICERS_CANDIDATE_LOCK_CHILD", directory.path())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        std::thread::sleep(Duration::from_millis(100));
        assert!(
            child.try_wait().unwrap().is_none(),
            "child bypassed held cross-process lock"
        );
        FileExt::unlock(&held).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success(), "child test exited with {status}");
                break;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                panic!("child test exceeded bounded wait");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(
            read_candidates(directory.path()).len(),
            MAX_PENDING_CANDIDATES
        );
    }

    #[test]
    fn malformed_candidate_file_is_not_overwritten() {
        let directory = tempfile::tempdir().unwrap();
        let candidate = candidate(Category::Lesson, "fact", Role::Assistant, "assistant");
        let path = candidate_path(directory.path(), &candidate.id).unwrap();
        std::fs::write(&path, "{ malformed user data").unwrap();
        assert!(!store_pending(directory.path(), &candidate, 1).unwrap());
        assert_eq!(
            std::fs::read_to_string(path).unwrap(),
            "{ malformed user data"
        );
    }

    #[test]
    fn accepted_curated_evidence_is_locally_queryable_with_provenance() {
        let directory = tempfile::tempdir().unwrap();
        let mut old = candidate(
            Category::Constraint,
            "Use the stable cache",
            Role::User,
            "user-1",
        );
        old.id = "00000000-0000-4000-8000-000000000001".into();
        old.state = CandidateState::Accepted;
        old.dataset = "repo".into();
        let mut current = candidate(
            Category::Constraint,
            "Use the revised cache",
            Role::User,
            "user-2",
        );
        current.id = "00000000-0000-4000-8000-000000000002".into();
        current.state = CandidateState::Accepted;
        current.supersedes = Some(old.id.clone());
        current.dataset = "repo".into();
        current.valid_until_ms = Some(500);
        let mut pending_correction = candidate(
            Category::Constraint,
            "pending correction",
            Role::User,
            "user-3",
        );
        pending_correction.id = "00000000-0000-4000-8000-000000000003".into();
        pending_correction.dataset = "repo".into();
        pending_correction.supersedes = Some(old.id.clone());
        let mut rejected_correction = pending_correction.clone();
        rejected_correction.id = "00000000-0000-4000-8000-000000000004".into();
        rejected_correction.state = CandidateState::Rejected("reviewed".into());
        let mut foreign_correction = pending_correction.clone();
        foreign_correction.id = "00000000-0000-4000-8000-000000000005".into();
        foreign_correction.state = CandidateState::Accepted;
        foreign_correction.dataset = "other".into();
        let mut expired_correction = pending_correction.clone();
        expired_correction.id = "00000000-0000-4000-8000-000000000006".into();
        expired_correction.state = CandidateState::Accepted;
        expired_correction.valid_until_ms = Some(99);
        let mut assistant_correction = pending_correction.clone();
        assistant_correction.id = "00000000-0000-4000-8000-000000000007".into();
        assistant_correction.state = CandidateState::Accepted;
        assistant_correction.source.role = Role::Assistant;
        for item in [
            &old,
            &current,
            &pending_correction,
            &rejected_correction,
            &foreign_correction,
            &expired_correction,
            &assistant_correction,
        ] {
            assert!(store_pending(directory.path(), item, 10).unwrap());
        }
        let results = query_accepted(directory.path(), "repo", None, "revised", 100, 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].text, "Use the revised cache");
        let metadata = results[0].metadata.as_ref().unwrap();
        assert_eq!(metadata["sourceEntryId"], "user-2");
        assert_eq!(metadata["source_entry_id"], "user-2");
        assert_eq!(metadata["source_id"], "user-2");
        assert_eq!(metadata["valid_until_ms"], 500);
        assert_eq!(metadata["dataset"], "repo");
        assert_eq!(
            metadata["supersedes"],
            "00000000-0000-4000-8000-000000000001"
        );
        assert!(query_accepted(directory.path(), "repo", None, "stable", 100, 10).is_empty());
        let other_dataset = query_accepted(directory.path(), "other", None, "", 100, 10);
        assert_eq!(other_dataset.len(), 1);
        assert_eq!(other_dataset[0].text, "pending correction");
    }

    #[test]
    fn accepted_receipt_projection_is_ignored_and_expiry_supersession_apply() {
        let directory = tempfile::tempdir().unwrap();
        let receipts = super::super::admission::receipt_dir(directory.path());
        let mut accepted = super::super::admission::tests::receipt();
        accepted.candidate.category = Category::Lesson;
        accepted.candidate.text = "durable accepted knowledge".into();
        accepted.candidate.dataset = "d".into();
        accepted.operation.dataset = "d".into();
        accepted.candidate.valid_until_ms = Some(500);
        accepted.id = "accepted-receipt".into();
        accepted.candidate.admission_id = Some(accepted.id.clone());
        accepted.operation.admission_id = Some(accepted.id.clone());
        super::super::admission::commit(&receipts, &accepted).unwrap();

        let mut expired = accepted.clone();
        expired.id = "expired-receipt".into();
        expired.candidate.id = "00000000-0000-4000-8000-000000000002".into();
        expired.candidate.admission_id = Some(expired.id.clone());
        expired.candidate.text = "expired knowledge".into();
        expired.candidate.valid_until_ms = Some(100);
        expired.operation.admission_id = Some(expired.id.clone());
        super::super::admission::commit(&receipts, &expired).unwrap();

        let mut correction = accepted.clone();
        correction.id = "correction-receipt".into();
        correction.candidate.id = "00000000-0000-4000-8000-000000000003".into();
        correction.candidate.admission_id = Some(correction.id.clone());
        correction.candidate.category = Category::Constraint;
        correction.operation.node_set = "user_context".into();
        correction.candidate.text = "corrected durable knowledge".into();
        correction.candidate.source.role = Role::User;
        correction.candidate.branch = Some("main".into());
        correction.candidate.supersedes = Some(accepted.candidate.id.clone());
        correction.candidate.valid_until_ms = Some(500);
        correction.operation.admission_id = Some(correction.id.clone());
        correction.superseded_target = Some(super::super::admission::SupersededTarget {
            source: "e".into(),
            scope: Some("main".into()),
            hash: "a".repeat(64),
        });
        super::super::admission::commit(&receipts, &correction).unwrap();

        let projection = candidate_path(directory.path(), &accepted.candidate.id).unwrap();
        std::fs::write(&projection, "{corrupt projection").unwrap();
        assert_eq!(
            query_accepted(directory.path(), "d", None, "durable", 200, 10)
                .iter()
                .map(|item| item.text.as_str())
                .collect::<Vec<_>>(),
            ["durable accepted knowledge"]
        );
        assert!(query_accepted(directory.path(), "d", None, "expired", 200, 10).is_empty());
        assert!(query_accepted(
            directory.path(),
            "d",
            Some("main"),
            "durable accepted",
            200,
            10
        )
        .is_empty());
        assert_eq!(
            query_accepted(directory.path(), "d", None, "durable accepted", 200, 10).len(),
            1,
            "correction only supersedes within its matching branch"
        );
        std::fs::remove_file(projection).unwrap();
        assert_eq!(
            query_accepted(directory.path(), "d", None, "durable", 200, 10).len(),
            1
        );
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
