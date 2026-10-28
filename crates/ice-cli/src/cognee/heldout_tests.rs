//! Opt-in, entirely local replay of scrubbed proposal streams. No runtime,
//! transport, provider, queue or Cognee service is constructed.
use super::candidates::{admit, Admission, Candidate, CandidateState, Category, Role, Source};
use super::source_validation::validate_source;
use ice_agent::AgentMessage;
use ice_ai::types::{
    Api, AssistantMessage, Content, ToolResultMessage, ToolResultRole, UserMessage,
};
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;
use std::path::Path;

#[derive(Deserialize)]
struct Session {
    key: String,
    turns: Vec<Turn>,
}
#[derive(Deserialize)]
struct Turn {
    role: String,
    timestamp: Option<i64>,
    text: String,
    entry_id: String,
    #[serde(default)]
    is_error: bool,
}
#[derive(Deserialize)]
struct Proposal {
    id: String,
    session_key: String,
    session_timestamp: Option<u64>,
    kind_hint: String,
    text: String,
    source_entry_id: String,
    verified_by: Option<String>,
    turn_index: Option<usize>,
    proposal_turn_index: Option<usize>,
}

fn message(turn: &Turn) -> AgentMessage {
    let at = turn.timestamp.unwrap_or_default();
    match turn.role.as_str() {
        "user" => AgentMessage::User(UserMessage::new(turn.text.clone(), at)),
        "assistant" => {
            let mut message = AssistantMessage::empty(Api::Faux, "faux", "local", at);
            message.content = vec![Content::text(&turn.text)];
            AgentMessage::Assistant(Box::new(message))
        }
        "tool" => AgentMessage::ToolResult(Box::new(ToolResultMessage {
            role: ToolResultRole,
            tool_call_id: turn.entry_id.clone(),
            tool_name: "recorded".into(),
            content: vec![Content::text(&turn.text)],
            details: None,
            usage: None,
            added_tool_names: vec![],
            is_error: turn.is_error,
            timestamp: at,
        })),
        _ => panic!("Normalizer emitted an unknown role"),
    }
}

fn decision(proposal: &Proposal, session: &Session) -> (String, String) {
    let Some(category) = Category::parse(&proposal.kind_hint) else {
        return ("reject".into(), "invalid_category".into());
    };
    let end = proposal
        .proposal_turn_index
        .or(proposal.turn_index)
        .map(|i| i.saturating_add(1))
        .unwrap_or(session.turns.len())
        .min(session.turns.len());
    // Match validation.rs's current bounded conversation, never future turns.
    let branch: Vec<_> = session.turns[end.saturating_sub(256)..end]
        .iter()
        .map(|turn| (turn.entry_id.clone(), message(turn)))
        .collect();
    let role = branch
        .iter()
        .find(|(id, _)| id == &proposal.source_entry_id)
        .map(|(_, message)| match message {
            AgentMessage::User(_) => Role::User,
            AgentMessage::Assistant(_) => Role::Assistant,
            _ => Role::Tool,
        })
        .unwrap_or(Role::User);
    let candidate = Candidate {
        id: proposal.id.clone(),
        admission_id: None,
        source_scope: None,
        category,
        text: proposal.text.clone(),
        source: Source {
            entry_id: proposal.source_entry_id.clone(),
            role,
        },
        verified_by: proposal.verified_by.clone(),
        supersedes: None,
        dataset: "heldout-local-only".into(),
        branch: None,
        valid_until_ms: None,
        created_at: proposal.session_timestamp.unwrap_or_default(),
        state: CandidateState::Pending,
    };
    // These are the unmodified production source gate and local admission
    // predicate. Pending is not an admission; no helper adjudication is run.
    if let Err(reason) = validate_source(&candidate, &branch) {
        return ("reject".into(), reason.as_str().into());
    }
    match admit(&candidate, &branch) {
        Admission::Accept => ("admit".into(), "source_validated_local_accept".into()),
        Admission::Reject(reason) => ("reject".into(), reason.as_str().into()),
        Admission::Pending => ("reject".into(), "pending_not_admitted_without_judge".into()),
    }
}

#[test]
#[ignore = "private local corpus; requires ICERS_HELDOUT_MANIFEST; never runs in CI"]
fn replay_heldout_native_admission() {
    let input =
        std::env::var_os("ICERS_HELDOUT_MANIFEST").expect("explicit manifest path required");
    let input = std::fs::canonicalize(input).unwrap();
    let expected = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/cognee-remaining-validation/heldout/protocol.json")
        .canonicalize()
        .unwrap();
    assert_eq!(
        input, expected,
        "only the private heldout directory is allowed"
    );
    let directory = input.parent().unwrap();
    let sessions: Vec<Session> =
        serde_json::from_slice(&std::fs::read(directory.join("sessions.json")).unwrap()).unwrap();
    let proposals: Vec<Proposal> =
        serde_json::from_slice(&std::fs::read(directory.join("candidates.json")).unwrap()).unwrap();
    let sessions: HashMap<_, _> = sessions.into_iter().map(|s| (s.key.clone(), s)).collect();
    let decisions: Vec<_> = proposals
        .iter()
        .map(|p| {
            let (outcome, reason) = decision(p, sessions.get(&p.session_key).unwrap());
            json!({"id":p.id,"decision":outcome,"reason":reason})
        })
        .collect();
    let path = directory.join("native-decisions.json");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path).unwrap();
    serde_json::to_writer_pretty(file, &decisions).unwrap();
    println!("local native decisions: {}", decisions.len());
}

#[test]
fn synthetic_replay_preserves_complete_source_gate() {
    let session = Session {
        key: "synthetic".into(),
        turns: vec![Turn {
            role: "user".into(),
            timestamp: Some(1),
            text: "Never disable source validation.".into(),
            entry_id: "user-1".into(),
            is_error: false,
        }],
    };
    let mut proposal = Proposal {
        id: "synthetic".into(),
        session_key: session.key.clone(),
        session_timestamp: Some(1),
        kind_hint: "constraint".into(),
        text: "Never disable source validation.".into(),
        source_entry_id: "user-1".into(),
        verified_by: None,
        turn_index: Some(0),
        proposal_turn_index: None,
    };
    assert_eq!(decision(&proposal, &session).0, "admit");
    proposal.text = "disable source validation".into();
    assert_eq!(decision(&proposal, &session).0, "reject");
    proposal.source_entry_id = "missing".into();
    assert_eq!(decision(&proposal, &session).1, "missing_source");
}
