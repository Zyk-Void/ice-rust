//! Optional, bounded TypeSafe judgments over supplied Cognee candidates.
//!
//! This module only constructs requests and reports judgments or abstentions; it
//! does not measure JEV quality or latency. Unit-test mocks prove control flow
//! only. Credential redaction is a heuristic for common shapes, not a guarantee
//! that every secret or personal datum is removed.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use ice_ai::decision::client::{DecisionClient, DecisionError, DecisionProtocol};
use ice_ai::decision::typesafe::{self, Answer, Question};
use ice_ai::decision::{DecisionCodecError, DecisionValidationError, NativeContent};
use serde_json::{json, Map, Value};
use thiserror::Error;
use tokio_util::sync::CancellationToken;

use super::config::CogneeConfig;
use super::text::redact_memory_text;

const DEFAULT_DEADLINE_MS: u64 = 1_500;
const MAX_CANDIDATES: usize = 8;
const MAX_CANDIDATE_TEXT_UTF16: usize = 600;
const MAX_CONTEXT_UTF16: usize = 800;
const MAX_INPUT_TOKENS: usize = 2_000;
const TEST_MODEL_ID: &str = "fixture/helper-model";
const QUESTION_PREFIX: &str = "candidate.";

/// A candidate fact and an optional hint for its memory category.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub id: String,
    pub text: String,
    pub kind_hint: Option<String>,
}

/// A bounded batch of redacted candidate text and current-context excerpt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateBatch {
    candidates: Vec<Candidate>,
    context_excerpt: Option<String>,
}

impl CandidateBatch {
    /// Validate IDs, reject oversized facts, redact bounded text, and enforce
    /// the approximate request token limit.
    pub fn new(
        mut candidates: Vec<Candidate>,
        context_excerpt: Option<String>,
    ) -> Result<Self, HelperError> {
        if candidates.is_empty() {
            return Err(HelperError::EmptyBatch);
        }
        if candidates.len() > MAX_CANDIDATES {
            return Err(HelperError::TooManyCandidates);
        }

        let mut ids = BTreeSet::new();
        for candidate in &mut candidates {
            if !valid_candidate_id(&candidate.id) {
                return Err(HelperError::InvalidCandidateId);
            }
            if !ids.insert(candidate.id.clone()) {
                return Err(HelperError::DuplicateCandidateId);
            }
            if candidate.text.encode_utf16().count() > MAX_CANDIDATE_TEXT_UTF16
                || candidate
                    .kind_hint
                    .as_ref()
                    .is_some_and(|hint| hint.encode_utf16().count() > MAX_CANDIDATE_TEXT_UTF16)
            {
                return Err(HelperError::TooLarge);
            }
            candidate.text = redact_memory_text(&candidate.text, MAX_CANDIDATE_TEXT_UTF16);
            candidate.kind_hint = candidate
                .kind_hint
                .as_deref()
                .map(|hint| redact_memory_text(hint, MAX_CANDIDATE_TEXT_UTF16));
        }
        if context_excerpt
            .as_ref()
            .is_some_and(|excerpt| excerpt.encode_utf16().count() > MAX_CONTEXT_UTF16)
        {
            return Err(HelperError::TooLarge);
        }
        let context_excerpt = context_excerpt
            .as_deref()
            .map(|excerpt| redact_memory_text(excerpt, MAX_CONTEXT_UTF16));
        let batch = Self {
            candidates,
            context_excerpt,
        };
        build_request(&batch)?;
        Ok(batch)
    }

    pub fn candidates(&self) -> &[Candidate] {
        &self.candidates
    }

    pub fn context_excerpt(&self) -> Option<&str> {
        self.context_excerpt.as_deref()
    }
}

/// A yes/no answer with a middle band for inconclusive probability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Yes,
    No,
    Uncertain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Decision,
    Constraint,
    Preference,
    Lesson,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Judgment {
    pub id: String,
    pub applicable: Verdict,
    pub durable: Verdict,
    /// `None` means this batch did not request a category; `Some(Category::None)`
    /// is the explicit `none` choice from the category question.
    pub category: Option<Category>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbstainReason {
    Disabled,
    Unavailable,
    Timeout,
    Cancelled,
    Malformed,
    UnknownCandidateId,
    TooLarge,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HelperOutcome {
    Judged(Vec<Judgment>),
    Abstained(AbstainReason),
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum HelperError {
    #[error("candidate batch must contain at least one candidate")]
    EmptyBatch,
    #[error("candidate batch exceeds the maximum of eight candidates")]
    TooManyCandidates,
    #[error("candidate ID must contain 1-64 letters, numbers, dots, underscores, or hyphens")]
    InvalidCandidateId,
    #[error("candidate IDs must be unique within a batch")]
    DuplicateCandidateId,
    #[error("candidate batch exceeds the estimated input limit")]
    TooLarge,
    #[error("helper decision provider could not be bound")]
    Binding,
    #[error("helper decision provider must use TypeSafe System One")]
    UnsupportedProtocol,
}

#[async_trait]
pub(crate) trait DecisionDecider: Send + Sync {
    async fn decide(
        &self,
        request: &typesafe::Request,
        timeout: Duration,
        signal: &CancellationToken,
    ) -> Result<typesafe::Response, DecisionError>;
}

struct ClientDecider(DecisionClient);

#[async_trait]
impl DecisionDecider for ClientDecider {
    async fn decide(
        &self,
        request: &typesafe::Request,
        timeout: Duration,
        signal: &CancellationToken,
    ) -> Result<typesafe::Response, DecisionError> {
        self.0.decide(request, timeout, signal).await
    }
}

/// A cached optional decision client with a short, single-call deadline.
pub struct Helper {
    decider: Arc<dyn DecisionDecider>,
    deadline_ms: u64,
    model_id: String,
}

impl Helper {
    #[cfg(test)]
    pub(crate) fn with_decider(decider: Arc<dyn DecisionDecider>, deadline_ms: u64) -> Self {
        Self {
            decider,
            deadline_ms,
            model_id: TEST_MODEL_ID.into(),
        }
    }

    /// Bind only the explicitly configured decision provider. No chat provider
    /// or fallback is consulted. `Ok(None)` constructs no decision client.
    pub fn bind(config: &CogneeConfig) -> Result<Option<Self>, HelperError> {
        Self::bind_using(config, crate::decisions::decision_client)
    }

    fn bind_using(
        config: &CogneeConfig,
        bind_client: impl FnOnce(&str) -> Result<DecisionClient, crate::decisions::DecisionConfigError>,
    ) -> Result<Option<Self>, HelperError> {
        if !config.helper_enabled || !config.enabled || !config.selective_writes() {
            return Ok(None);
        }
        let (Some(provider_id), Some(model_id)) = (
            config.helper_provider.as_deref(),
            config.helper_model.as_deref(),
        ) else {
            return Ok(None);
        };
        let client = bind_client(provider_id).map_err(|error| match error {
            crate::decisions::DecisionConfigError::UnsupportedProtocol { .. } => {
                HelperError::UnsupportedProtocol
            }
            _ => HelperError::Binding,
        })?;
        if client.protocol() != DecisionProtocol::TypeSafeSystemOne {
            return Err(HelperError::UnsupportedProtocol);
        }
        Ok(Some(Self {
            decider: Arc::new(ClientDecider(client)),
            deadline_ms: DEFAULT_DEADLINE_MS,
            model_id: model_id.to_owned(),
        }))
    }

    pub async fn judge(&self, batch: &CandidateBatch, signal: CancellationToken) -> HelperOutcome {
        if batch.candidates.is_empty() || batch.candidates.len() > MAX_CANDIDATES {
            return HelperOutcome::Abstained(AbstainReason::Malformed);
        }
        if signal.is_cancelled() {
            return HelperOutcome::Abstained(AbstainReason::Cancelled);
        }
        let request = match build_request_for_model(batch, &self.model_id) {
            Ok(request) => request,
            Err(HelperError::TooLarge) => {
                return HelperOutcome::Abstained(AbstainReason::TooLarge);
            }
            Err(_) => return HelperOutcome::Abstained(AbstainReason::Malformed),
        };
        let deadline = Duration::from_millis(self.deadline_ms);
        let call = self.decider.decide(&request, deadline, &signal);
        let response = tokio::select! {
            biased;
            _ = signal.cancelled() => return HelperOutcome::Abstained(AbstainReason::Cancelled),
            result = tokio::time::timeout(deadline, call) => match result {
                Ok(Ok(response)) => response,
                Ok(Err(error)) => return HelperOutcome::Abstained(map_decision_error(error)),
                Err(_) => return HelperOutcome::Abstained(AbstainReason::Timeout),
            },
        };
        match decode_response(batch, &request, &response) {
            Ok(judgments) => HelperOutcome::Judged(judgments),
            Err(reason) => HelperOutcome::Abstained(reason),
        }
    }
}

fn valid_candidate_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn question_id(id: &str, field: &str) -> String {
    format!("{QUESTION_PREFIX}{id}.{field}")
}

/// Build one independent set of questions for each candidate.
pub fn build_request(batch: &CandidateBatch) -> Result<typesafe::Request, HelperError> {
    build_request_for_model(batch, TEST_MODEL_ID)
}

fn build_request_for_model(
    batch: &CandidateBatch,
    model_id: &str,
) -> Result<typesafe::Request, HelperError> {
    if batch.candidates.is_empty() {
        return Err(HelperError::EmptyBatch);
    }
    if batch.candidates.len() > MAX_CANDIDATES {
        return Err(HelperError::TooManyCandidates);
    }
    let mut questions = BTreeMap::new();
    for candidate in &batch.candidates {
        questions.insert(
            question_id(&candidate.id, "applicable"),
            Question::Noul {
                instructions: Some(NativeContent::text(format!(
                    "Does candidate {} affect the current task and add information absent from the supplied context? Judge only this candidate. Answer yes only when both conditions hold.",
                    candidate.id
                ))),
                criteria: None,
            },
        );
        questions.insert(
            question_id(&candidate.id, "durable"),
            Question::Noul {
                instructions: Some(NativeContent::text(format!(
                    "Is candidate {} a durable project constraint or decision rather than a temporary status? Judge only this candidate.",
                    candidate.id
                ))),
                criteria: None,
            },
        );
        if candidate.kind_hint.is_none() {
            questions.insert(
                question_id(&candidate.id, "category"),
                Question::Choice {
                    instructions: Some(NativeContent::text(format!(
                        "Which category best describes candidate {}? Choose none if no category fits. Judge only this candidate.",
                        candidate.id
                    ))),
                    criteria: BTreeMap::from([
                        ("decision".into(), None),
                        ("constraint".into(), None),
                        ("preference".into(), None),
                        ("lesson".into(), None),
                        ("none".into(), None),
                    ]),
                },
            );
        }
    }

    let mut state = Map::new();
    if let Some(context) = &batch.context_excerpt {
        state.insert("context_excerpt".into(), Value::String(context.clone()));
    }
    let candidates = batch
        .candidates
        .iter()
        .map(|candidate| {
            (
                candidate.id.clone(),
                json!({ "text": candidate.text, "kind_hint": candidate.kind_hint }),
            )
        })
        .collect::<Map<String, Value>>();
    state.insert("candidates".into(), Value::Object(candidates));
    let request = typesafe::Request::new(model_id, NativeContent::Object(state), questions);
    request.validate().map_err(|_| HelperError::TooLarge)?;
    if request_estimated_tokens(&request) > MAX_INPUT_TOKENS {
        return Err(HelperError::TooLarge);
    }
    Ok(request)
}

fn request_estimated_tokens(request: &typesafe::Request) -> usize {
    request
        .encode_json()
        .map(|encoded| utf16_len(&String::from_utf8_lossy(&encoded)).div_ceil(4))
        .unwrap_or(usize::MAX)
}

fn utf16_len(value: &str) -> usize {
    value.encode_utf16().count()
}

fn decode_response(
    batch: &CandidateBatch,
    request: &typesafe::Request,
    response: &typesafe::Response,
) -> Result<Vec<Judgment>, AbstainReason> {
    decode_answers(batch, request, response.answers())
}

fn decode_answers(
    batch: &CandidateBatch,
    request: &typesafe::Request,
    answers: &BTreeMap<String, Answer>,
) -> Result<Vec<Judgment>, AbstainReason> {
    let expected = request.questions().keys().cloned().collect::<BTreeSet<_>>();
    let candidate_ids = batch
        .candidates
        .iter()
        .map(|candidate| candidate.id.as_str())
        .collect::<BTreeSet<_>>();
    for key in answers.keys() {
        if expected.contains(key) {
            continue;
        }
        let Some(rest) = key.strip_prefix(QUESTION_PREFIX) else {
            return Err(AbstainReason::Malformed);
        };
        let candidate_id = rest
            .strip_suffix(".applicable")
            .or_else(|| rest.strip_suffix(".durable"))
            .or_else(|| rest.strip_suffix(".category"));
        let Some(candidate_id) = candidate_id else {
            return Err(AbstainReason::Malformed);
        };
        if !candidate_ids.contains(candidate_id) {
            return Err(AbstainReason::UnknownCandidateId);
        }
        return Err(AbstainReason::Malformed);
    }
    if answers.keys().cloned().collect::<BTreeSet<_>>() != expected {
        return Err(AbstainReason::Malformed);
    }

    batch
        .candidates
        .iter()
        .map(|candidate| {
            let applicable = match answers.get(&question_id(&candidate.id, "applicable")) {
                Some(Answer::Noul { noul }) => probability_verdict(noul.get()),
                _ => return Err(AbstainReason::Malformed),
            };
            let durable = match answers.get(&question_id(&candidate.id, "durable")) {
                Some(Answer::Noul { noul }) => probability_verdict(noul.get()),
                _ => return Err(AbstainReason::Malformed),
            };
            let category = if candidate.kind_hint.is_some() {
                None
            } else {
                match answers.get(&question_id(&candidate.id, "category")) {
                    Some(Answer::Choice { choice, .. }) => {
                        Some(parse_category(choice).ok_or(AbstainReason::Malformed)?)
                    }
                    _ => return Err(AbstainReason::Malformed),
                }
            };
            Ok(Judgment {
                id: candidate.id.clone(),
                applicable,
                durable,
                category,
            })
        })
        .collect()
}

fn probability_verdict(probability: f64) -> Verdict {
    if probability >= 0.75 {
        Verdict::Yes
    } else if probability <= 0.25 {
        Verdict::No
    } else {
        Verdict::Uncertain
    }
}

fn parse_category(value: &str) -> Option<Category> {
    match value {
        "decision" => Some(Category::Decision),
        "constraint" => Some(Category::Constraint),
        "preference" => Some(Category::Preference),
        "lesson" => Some(Category::Lesson),
        "none" => Some(Category::None),
        _ => None,
    }
}

fn map_decision_error(error: DecisionError) -> AbstainReason {
    match error {
        DecisionError::Cancelled => AbstainReason::Cancelled,
        DecisionError::Timeout => AbstainReason::Timeout,
        DecisionError::Codec(DecisionCodecError::InvalidResponse(
            DecisionValidationError::AnswerNamesMismatch,
        )) => AbstainReason::UnknownCandidateId,
        DecisionError::Codec(_) => AbstainReason::Malformed,
        _ => AbstainReason::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decisions::DecisionConfigError;
    use ice_ai::decision::client::{DecisionEndpoint, DecisionProtocol};
    use ice_ai::decision::typesafe::Question;
    use ice_ai::decision::Probability;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    include!("live_jev_tests.rs");

    fn batch(ids: &[&str]) -> CandidateBatch {
        CandidateBatch::new(
            ids.iter()
                .map(|id| Candidate {
                    id: (*id).into(),
                    text: format!("{id} is a project constraint."),
                    kind_hint: None,
                })
                .collect(),
            Some("current task context".into()),
        )
        .unwrap()
    }

    fn response_for(request: &typesafe::Request) -> typesafe::Response {
        let answers = request
            .questions()
            .iter()
            .map(|(id, question)| {
                let answer = match question {
                    Question::Noul { .. } if id.ends_with(".applicable") => {
                        json!({"type":"noul", "noul":0.9})
                    }
                    Question::Noul { .. } => json!({"type":"noul", "noul":0.1}),
                    Question::Choice { .. } => json!({
                        "type":"choice",
                        "choice":"none",
                        "probabilities":{
                            "decision":0.0,
                            "constraint":0.0,
                            "preference":0.0,
                            "lesson":0.0,
                            "none":1.0
                        },
                        "confidence":0.9
                    }),
                    Question::Score { .. } => unreachable!(),
                };
                (id.clone(), answer)
            })
            .collect::<Map<String, Value>>();
        let bytes = serde_json::to_vec(&json!({
            "model": TEST_MODEL_ID,
            "answers": answers,
            "usage": {"input_tokens": 1, "output_tokens": 1}
        }))
        .unwrap();
        typesafe::Response::decode_json(&bytes, request).unwrap()
    }

    fn client(protocol: DecisionProtocol) -> DecisionClient {
        DecisionClient::new(
            DecisionEndpoint {
                protocol,
                base_url: "https://decision.invalid/v1".into(),
                api_key: "not-a-real-key".into(),
                headers: BTreeMap::new(),
            },
            None,
        )
        .unwrap()
    }

    async fn serve_typesafe_once() -> (String, tokio::task::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buffer = [0_u8; 4096];
            loop {
                let read = socket.read(&mut buffer).await.unwrap();
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);
                let text = String::from_utf8_lossy(&request);
                if let Some(header_end) = text.find("\r\n\r\n") {
                    let length = text[..header_end]
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|value| value.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if request.len() >= header_end + 4 + length {
                        break;
                    }
                }
            }
            let text = String::from_utf8_lossy(&request);
            let body_offset = text.find("\r\n\r\n").unwrap() + 4;
            let body: Value = serde_json::from_slice(&request[body_offset..]).unwrap();
            assert_eq!(body["model"], TEST_MODEL_ID);
            let answers = body["questions"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(id, question)| {
                    let answer = if question["type"] == "choice" {
                        json!({
                            "type": "choice",
                            "choice": "constraint",
                            "probabilities": {
                                "decision": 0.0,
                                "constraint": 1.0,
                                "preference": 0.0,
                                "lesson": 0.0,
                                "none": 0.0
                            },
                            "confidence": 0.9
                        })
                    } else {
                        json!({"type": "noul", "noul": 0.9})
                    };
                    (id.clone(), answer)
                })
                .collect::<Map<String, Value>>();
            let response = serde_json::to_vec(&json!({
                "model": TEST_MODEL_ID,
                "answers": answers,
                "usage": {"input_tokens": 12, "output_tokens": 4}
            }))
            .unwrap();
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                response.len()
            );
            socket.write_all(head.as_bytes()).await.unwrap();
            socket.write_all(&response).await.unwrap();
            socket.shutdown().await.ok();
            String::from_utf8(request).unwrap()
        });
        (format!("http://{address}/v1/"), server)
    }

    #[test]
    fn batch_rejects_more_than_eight_candidates() {
        let candidates = (0..9)
            .map(|index| Candidate {
                id: format!("id{index}"),
                text: "candidate".into(),
                kind_hint: None,
            })
            .collect();
        assert_eq!(
            CandidateBatch::new(candidates, None).unwrap_err(),
            HelperError::TooManyCandidates
        );
    }

    #[test]
    fn batch_rejects_duplicate_candidate_ids() {
        assert_eq!(
            CandidateBatch::new(
                vec![
                    Candidate {
                        id: "same".into(),
                        text: "a".into(),
                        kind_hint: None
                    },
                    Candidate {
                        id: "same".into(),
                        text: "b".into(),
                        kind_hint: None
                    },
                ],
                None,
            )
            .unwrap_err(),
            HelperError::DuplicateCandidateId
        );
    }

    #[test]
    fn batch_rejects_invalid_candidate_ids() {
        for id in ["", "has space", "slash/id", &"x".repeat(65)] {
            assert_eq!(
                CandidateBatch::new(
                    vec![Candidate {
                        id: id.into(),
                        text: "text".into(),
                        kind_hint: None
                    }],
                    None,
                )
                .unwrap_err(),
                HelperError::InvalidCandidateId
            );
        }
    }

    #[test]
    fn batch_rejects_oversized_candidate_instead_of_judging_truncation() {
        assert_eq!(
            CandidateBatch::new(
                vec![Candidate {
                    id: "unicode".into(),
                    text: format!("{}tail", "😀".repeat(400)),
                    kind_hint: None
                }],
                None
            )
            .unwrap_err(),
            HelperError::TooLarge
        );
        let batch = CandidateBatch::new(
            vec![Candidate {
                id: "within".into(),
                text: "😀".repeat(200),
                kind_hint: None,
            }],
            None,
        )
        .unwrap();
        assert_eq!(utf16_len(&batch.candidates()[0].text), 400);
    }

    #[test]
    fn batch_rejects_oversized_context_excerpt() {
        assert_eq!(
            CandidateBatch::new(
                vec![Candidate {
                    id: "one".into(),
                    text: "text".into(),
                    kind_hint: None
                }],
                Some("context".repeat(200))
            )
            .unwrap_err(),
            HelperError::TooLarge
        );
    }

    #[test]
    fn batch_rejects_estimated_input_over_two_thousand_tokens() {
        let candidates = (0..8)
            .map(|index| Candidate {
                id: format!("id{index}"),
                text: "fact".repeat(150),
                kind_hint: Some("hint".repeat(150)),
            })
            .collect();
        assert_eq!(
            CandidateBatch::new(candidates, None).unwrap_err(),
            HelperError::TooLarge
        );
    }

    #[test]
    fn configured_model_is_used_exactly_in_the_decision_request() {
        let batch = batch(&["alpha"]);
        let request = build_request_for_model(&batch, "vendor/custom-model-v9").unwrap();
        assert_eq!(request.model(), "vendor/custom-model-v9");
    }

    #[test]
    fn batch_redacts_candidate_and_context_before_building_request() {
        let batch = CandidateBatch::new(
            vec![Candidate {
                id: "one".into(),
                text: "Use Bearer candidate-secret-123 in production".into(),
                kind_hint: None,
            }],
            Some("Authorization: Bearer context-secret-456".into()),
        )
        .unwrap();
        let request = build_request(&batch).unwrap();
        let serialized = String::from_utf8(request.encode_json().unwrap()).unwrap();
        assert!(!serialized.contains("candidate-secret-123"));
        assert!(!serialized.contains("context-secret-456"));
        assert!(serialized.contains("[REDACTED]"));
    }

    #[test]
    fn built_request_questions_are_independent_and_include_none_choice() {
        let batch = batch(&["alpha", "beta"]);
        let request = build_request(&batch).unwrap();
        assert_eq!(request.questions().len(), 6);
        for (key, question) in request.questions() {
            let instructions = match question {
                Question::Noul { instructions, .. } | Question::Choice { instructions, .. } => {
                    instructions
                }
                Question::Score { .. } => panic!("score questions are not built"),
            };
            let text = match instructions.as_ref().unwrap() {
                NativeContent::Text(text) => text,
                _ => panic!("question instructions are text"),
            };
            let other_candidate = if key.contains("alpha") {
                "beta"
            } else {
                "alpha"
            };
            assert!(!text.contains(other_candidate));
            assert!(!text.contains("question 1"));
            assert!(!text.contains("answer to"));
            if let Question::Choice { criteria, .. } = question {
                assert!(criteria.contains_key("none"));
            }
        }
    }

    #[test]
    fn question_ids_encode_candidate_ids_and_context_is_in_state() {
        let batch = batch(&["alpha-1"]);
        let request = build_request(&batch).unwrap();
        assert!(request
            .questions()
            .contains_key("candidate.alpha-1.applicable"));
        assert!(request
            .questions()
            .contains_key("candidate.alpha-1.durable"));
        assert!(request
            .questions()
            .contains_key("candidate.alpha-1.category"));
        let state = serde_json::to_string(request.state()).unwrap();
        assert!(state.contains("current task context"));
    }

    #[test]
    fn kind_hint_skips_category_question() {
        let batch = CandidateBatch::new(
            vec![Candidate {
                id: "hinted".into(),
                text: "text".into(),
                kind_hint: Some("decision".into()),
            }],
            None,
        )
        .unwrap();
        let request = build_request(&batch).unwrap();
        assert_eq!(request.questions().len(), 2);
        assert!(!request
            .questions()
            .contains_key("candidate.hinted.category"));
    }

    #[test]
    fn yes_no_and_uncertain_answers_decode_to_judgments() {
        let batch = batch(&["alpha"]);
        let request = build_request(&batch).unwrap();
        let mut answers = response_for(&request).answers().clone();
        if let Some(Answer::Noul { noul }) = answers.get_mut("candidate.alpha.durable") {
            *noul = Probability::new(0.5).unwrap();
        }
        if let Some(Answer::Choice { choice, .. }) = answers.get_mut("candidate.alpha.category") {
            *choice = "decision".into();
        }
        let judgments = decode_answers(&batch, &request, &answers).unwrap();
        assert_eq!(
            judgments,
            vec![Judgment {
                id: "alpha".into(),
                applicable: Verdict::Yes,
                durable: Verdict::Uncertain,
                category: Some(Category::Decision),
            }]
        );
        assert_eq!(probability_verdict(0.1), Verdict::No);
    }

    #[test]
    fn unknown_returned_candidate_id_is_rejected() {
        let batch = batch(&["alpha"]);
        let request = build_request(&batch).unwrap();
        let mut answers = response_for(&request).answers().clone();
        answers.insert(
            "candidate.ghost.applicable".into(),
            Answer::Noul {
                noul: Probability::new(0.9).unwrap(),
            },
        );
        assert_eq!(
            decode_answers(&batch, &request, &answers),
            Err(AbstainReason::UnknownCandidateId)
        );
    }

    #[test]
    fn unknown_response_candidate_id_becomes_abstention() {
        let candidate_batch = batch(&["alpha"]);
        let request = build_request(&candidate_batch).unwrap();
        let foreign_request = build_request(&batch(&["ghost"])).unwrap();
        let response = response_for(&foreign_request);
        assert_eq!(
            decode_response(&candidate_batch, &request, &response),
            Err(AbstainReason::UnknownCandidateId)
        );
    }

    #[test]
    fn malformed_answer_shape_abstains() {
        let batch = batch(&["alpha"]);
        let request = build_request(&batch).unwrap();
        let mut answers = response_for(&request).answers().clone();
        answers.insert(
            "candidate.alpha.applicable".into(),
            Answer::Choice {
                choice: "none".into(),
                probabilities: BTreeMap::new(),
                confidence: ice_ai::decision::Confidence::new(1.0).unwrap(),
            },
        );
        assert_eq!(
            decode_answers(&batch, &request, &answers),
            Err(AbstainReason::Malformed)
        );
    }

    #[test]
    fn missing_answers_are_malformed() {
        let batch = batch(&["alpha"]);
        let request = build_request(&batch).unwrap();
        let mut answers = response_for(&request).answers().clone();
        answers.remove("candidate.alpha.durable");
        assert_eq!(
            decode_answers(&batch, &request, &answers),
            Err(AbstainReason::Malformed)
        );
    }

    #[test]
    fn explicit_helper_off_does_not_construct_a_client() {
        let mut config = CogneeConfig {
            helper_enabled: false,
            write_policy: super::super::config::WritePolicy::Selective,
            ..CogneeConfig::default()
        };
        let calls = AtomicUsize::new(0);
        let result = Helper::bind_using(&config, |_| {
            calls.fetch_add(1, Ordering::SeqCst);
            Err(DecisionConfigError::UnknownProvider(
                "must-not-be-read".into(),
            ))
        })
        .unwrap();
        assert!(result.is_none());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        config.helper_provider = None;
    }

    #[test]
    fn enabled_default_with_missing_binding_is_nonfatal_and_does_not_construct_a_client() {
        let config = CogneeConfig {
            write_policy: super::super::config::WritePolicy::Selective,
            ..CogneeConfig::default()
        };
        assert!(config.helper_enabled);
        let calls = AtomicUsize::new(0);
        let result = Helper::bind_using(&config, |_| {
            calls.fetch_add(1, Ordering::SeqCst);
            Err(DecisionConfigError::UnknownProvider(
                "must-not-be-read".into(),
            ))
        })
        .unwrap();
        assert!(result.is_none());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn manual_mode_suppresses_enabled_helper_binding() {
        let config = CogneeConfig {
            write_policy: super::super::config::WritePolicy::Manual,
            helper_provider: Some("decision-provider".into()),
            helper_model: Some("test/model".into()),
            ..CogneeConfig::default()
        };
        assert!(config.helper_enabled);
        let calls = AtomicUsize::new(0);
        let result = Helper::bind_using(&config, |_| {
            calls.fetch_add(1, Ordering::SeqCst);
            Err(DecisionConfigError::UnknownProvider(
                "must-not-be-read".into(),
            ))
        })
        .unwrap();
        assert!(result.is_none());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn binding_errors_are_nonfatal_and_never_include_provider_secrets() {
        let mut config = CogneeConfig::default();
        config.write_policy = super::super::config::WritePolicy::Selective;
        config.helper_enabled = true;
        config.helper_model = Some("test/model".into());
        config.helper_provider = Some("decision-provider".into());
        for source in [
            DecisionConfigError::UnknownProvider("decision-provider".into()),
            DecisionConfigError::MissingCredential("decision-provider".into()),
        ] {
            let result = Helper::bind_using(&config, |_| Err(source));
            let Err(error) = result else {
                panic!("provider binding should fail");
            };
            assert_eq!(error, HelperError::Binding);
        }
        let source = DecisionConfigError::UnsupportedProtocol {
            provider: "decision-provider".into(),
            protocol: "credential-like-secret".into(),
        };
        let Err(error) = Helper::bind_using(&config, |_| Err(source)) else {
            panic!("protocol binding should fail");
        };
        assert_eq!(error, HelperError::UnsupportedProtocol);
        assert!(!error.to_string().contains("credential-like-secret"));
    }

    #[test]
    fn binding_rejects_non_typesafe_protocol_without_fallback() {
        let mut config = CogneeConfig::default();
        config.write_policy = super::super::config::WritePolicy::Selective;
        config.helper_enabled = true;
        config.helper_model = Some("test/model".into());
        config.helper_provider = Some("decision-provider".into());
        let result = Helper::bind_using(&config, |_| Ok(client(DecisionProtocol::OpenAiDecisions)));
        assert!(matches!(result, Err(HelperError::UnsupportedProtocol)));

        let result = Helper::bind_using(&config, |_| {
            Err(DecisionConfigError::UnsupportedProtocol {
                provider: "decision-provider".into(),
                protocol: "openai-decisions".into(),
            })
        });
        assert!(matches!(result, Err(HelperError::UnsupportedProtocol)));
    }

    #[derive(Clone, Copy)]
    enum FakeBehavior {
        Success,
        Error,
        Slow,
    }

    struct FakeDecider {
        calls: AtomicUsize,
        behavior: FakeBehavior,
    }

    #[async_trait]
    impl DecisionDecider for FakeDecider {
        async fn decide(
            &self,
            request: &typesafe::Request,
            _timeout: Duration,
            _signal: &CancellationToken,
        ) -> Result<typesafe::Response, DecisionError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            match self.behavior {
                FakeBehavior::Success => Ok(response_for(request)),
                FakeBehavior::Error => Err(DecisionError::Status { status: 500 }),
                FakeBehavior::Slow => {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    Ok(response_for(request))
                }
            }
        }
    }

    fn fake_helper(behavior: FakeBehavior, deadline_ms: u64) -> (Helper, Arc<FakeDecider>) {
        let decider = Arc::new(FakeDecider {
            calls: AtomicUsize::new(0),
            behavior,
        });
        (
            Helper {
                decider: decider.clone(),
                deadline_ms,
                model_id: TEST_MODEL_ID.into(),
            },
            decider,
        )
    }

    #[tokio::test]
    async fn helper_judges_through_the_real_typesafe_http_client() {
        let (base_url, server) = serve_typesafe_once().await;
        let client = DecisionClient::new(
            DecisionEndpoint {
                protocol: DecisionProtocol::TypeSafeSystemOne,
                base_url,
                api_key: "local-fixture-key".into(),
                headers: BTreeMap::new(),
            },
            None,
        )
        .unwrap();
        let helper = Helper {
            decider: Arc::new(ClientDecider(client)),
            deadline_ms: 1_000,
            model_id: TEST_MODEL_ID.into(),
        };

        let outcome = helper
            .judge(&batch(&["alpha"]), CancellationToken::new())
            .await;
        let raw_request = tokio::time::timeout(Duration::from_secs(5), server)
            .await
            .expect("loopback TypeSafe fixture must finish within its deadline")
            .unwrap();

        let HelperOutcome::Judged(judgments) = outcome else {
            panic!("local TypeSafe response should produce a judgment");
        };
        assert_eq!(judgments.len(), 1);
        assert_eq!(judgments[0].id, "alpha");
        assert_eq!(judgments[0].applicable, Verdict::Yes);
        assert_eq!(judgments[0].durable, Verdict::Yes);
        assert_eq!(judgments[0].category, Some(Category::Constraint));
        assert!(raw_request.starts_with("POST /v1/systemone HTTP/1.1\r\n"));
        assert!(raw_request
            .to_ascii_lowercase()
            .contains("authorization: bearer local-fixture-key\r\n"));
    }

    #[tokio::test]
    async fn slow_decider_times_out_within_the_feature_deadline() {
        let (helper, fake) = fake_helper(FakeBehavior::Slow, 25);
        let started = tokio::time::Instant::now();
        let result = helper
            .judge(&batch(&["alpha"]), CancellationToken::new())
            .await;
        assert_eq!(result, HelperOutcome::Abstained(AbstainReason::Timeout));
        assert!(started.elapsed() < Duration::from_millis(150));
        assert_eq!(fake.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn cancellation_abstains_and_does_not_retry() {
        let (helper, fake) = fake_helper(FakeBehavior::Slow, 1_000);
        let signal = CancellationToken::new();
        let trigger = signal.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            trigger.cancel();
        });
        let result = helper.judge(&batch(&["alpha"]), signal).await;
        assert_eq!(result, HelperOutcome::Abstained(AbstainReason::Cancelled));
        assert_eq!(fake.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn cancelled_before_judging_skips_the_decider() {
        let (helper, fake) = fake_helper(FakeBehavior::Success, 100);
        let signal = CancellationToken::new();
        signal.cancel();
        assert_eq!(
            helper.judge(&batch(&["alpha"]), signal).await,
            HelperOutcome::Abstained(AbstainReason::Cancelled)
        );
        assert_eq!(fake.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn decision_errors_abstain_unavailable_with_one_call() {
        let (helper, fake) = fake_helper(FakeBehavior::Error, 100);
        assert_eq!(
            helper
                .judge(&batch(&["alpha"]), CancellationToken::new())
                .await,
            HelperOutcome::Abstained(AbstainReason::Unavailable)
        );
        assert_eq!(fake.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn successful_call_returns_judgments_with_one_decision_call() {
        let (helper, fake) = fake_helper(FakeBehavior::Success, 100);
        assert!(matches!(
            helper
                .judge(&batch(&["alpha"]), CancellationToken::new())
                .await,
            HelperOutcome::Judged(_)
        ));
        assert_eq!(fake.calls.load(Ordering::SeqCst), 1);
    }
}
