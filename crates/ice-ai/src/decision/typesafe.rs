//! Offline DTOs for TypeSafe's native System One (`/v1/systemone`) contract.
//!
//! This module is intentionally distinct from the independent JevTypeSafeAI
//! gateway and from OpenAI's `/v1/decisions` array protocol.

use std::collections::{BTreeMap, BTreeSet};

use super::{
    decode_response_value, encode_bounded_json, nonblank, parse_unique_json_with_depth,
    probability_distribution_sums_to_one, same_string_keys as same_keys, score_index_keys,
    score_matches_expected_value, selected_choice_has_max_probability, Confidence,
    DecisionCodecError, DecisionValidationError, NativeContent, Probability,
    MAX_DECISION_JSON_DEPTH, MAX_DECISION_RESPONSE_JSON_DEPTH,
};
use serde::{Deserialize, Serialize};

/// A TypeSafe Noul clarification. Either side may be omitted.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NoulCriteria {
    #[serde(rename = "true", skip_serializing_if = "Option::is_none")]
    pub yes: Option<NativeContent>,
    #[serde(rename = "false", skip_serializing_if = "Option::is_none")]
    pub no: Option<NativeContent>,
}

/// A native TypeSafe question, serialized under its caller-chosen map key.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Question {
    Noul {
        #[serde(skip_serializing_if = "Option::is_none")]
        instructions: Option<NativeContent>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        criteria: Option<Option<NoulCriteria>>,
    },
    Choice {
        #[serde(skip_serializing_if = "Option::is_none")]
        instructions: Option<NativeContent>,
        criteria: BTreeMap<String, Option<NativeContent>>,
    },
    Score {
        #[serde(skip_serializing_if = "Option::is_none")]
        instructions: Option<NativeContent>,
        criteria: Vec<NativeContent>,
    },
}

/// Request for TypeSafe native System One.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    model: String,
    state: NativeContent,
    questions: BTreeMap<String, Question>,
}

impl Request {
    pub fn new(
        model: impl Into<String>,
        state: NativeContent,
        questions: BTreeMap<String, Question>,
    ) -> Self {
        Self {
            model: model.into(),
            state,
            questions,
        }
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn state(&self) -> &NativeContent {
        &self.state
    }

    pub fn questions(&self) -> &BTreeMap<String, Question> {
        &self.questions
    }

    pub fn validate(&self) -> Result<(), DecisionValidationError> {
        if !nonblank(&self.model) {
            return Err(DecisionValidationError::EmptyModelId);
        }
        validate_content(&self.state)?;
        if self.questions.is_empty() {
            return Err(DecisionValidationError::EmptyQuestions);
        }

        for (name, question) in &self.questions {
            if !nonblank(name) {
                return Err(DecisionValidationError::EmptyQuestionName);
            }
            match question {
                Question::Noul {
                    instructions,
                    criteria,
                } => {
                    validate_optional_instructions(instructions.as_ref())?;
                    if let Some(Some(criteria)) = criteria {
                        validate_noul_criteria(criteria)?;
                    }
                }
                Question::Choice {
                    instructions,
                    criteria,
                } => {
                    validate_optional_instructions(instructions.as_ref())?;
                    if criteria.is_empty() || criteria.len() > 255 {
                        return Err(DecisionValidationError::ChoiceCountOutOfRange);
                    }
                    for (choice, description) in criteria {
                        if !nonblank(choice) {
                            return Err(DecisionValidationError::EmptyCriteria);
                        }
                        if let Some(description) = description {
                            validate_criterion(description)?;
                        }
                    }
                }
                Question::Score {
                    instructions,
                    criteria,
                } => {
                    validate_optional_instructions(instructions.as_ref())?;
                    if !(2..=10).contains(&criteria.len()) {
                        return Err(DecisionValidationError::ScoreLevelCountOutOfRange);
                    }
                    validate_distinct_criteria(criteria)?;
                }
            }
        }
        Ok(())
    }

    /// Encode a validated TypeSafe request. TypeSafe documents a 32 MiB body
    /// maximum and 64 nested container levels for state, instructions, and
    /// criteria; the codec enforces both.
    pub fn encode_json(&self) -> Result<Vec<u8>, DecisionCodecError> {
        self.validate()
            .map_err(DecisionCodecError::InvalidRequest)?;

        #[derive(Serialize)]
        struct WireRequest<'a> {
            model: &'a str,
            state: &'a NativeContent,
            questions: &'a BTreeMap<String, Question>,
        }

        encode_bounded_json(
            &WireRequest {
                model: &self.model,
                state: &self.state,
                questions: &self.questions,
            },
            32 * 1024 * 1024,
        )
    }
}

fn validate_content(content: &NativeContent) -> Result<(), DecisionValidationError> {
    if content.max_container_depth() > MAX_DECISION_JSON_DEPTH {
        return Err(DecisionValidationError::JsonDepthOutOfRange);
    }
    Ok(())
}

fn validate_criterion(content: &NativeContent) -> Result<(), DecisionValidationError> {
    // The enclosing criteria map/array is itself one container.
    if content.max_container_depth() >= MAX_DECISION_JSON_DEPTH {
        return Err(DecisionValidationError::JsonDepthOutOfRange);
    }
    Ok(())
}

fn validate_optional_instructions(
    instructions: Option<&NativeContent>,
) -> Result<(), DecisionValidationError> {
    if let Some(instructions) = instructions {
        validate_content(instructions)?;
    }
    Ok(())
}

fn validate_noul_criteria(criteria: &NoulCriteria) -> Result<(), DecisionValidationError> {
    let mut has_criterion = false;
    for criterion in [&criteria.yes, &criteria.no].into_iter().flatten() {
        validate_criterion(criterion)?;
        has_criterion = true;
    }
    if !has_criterion {
        return Err(DecisionValidationError::EmptyCriteria);
    }
    Ok(())
}

fn validate_distinct_criteria(criteria: &[NativeContent]) -> Result<(), DecisionValidationError> {
    for criterion in criteria {
        validate_criterion(criterion)?;
    }
    for (index, criterion) in criteria.iter().enumerate() {
        if criteria[index + 1..].contains(criterion) {
            return Err(DecisionValidationError::DuplicateScoreCriteria);
        }
    }
    Ok(())
}

/// TypeSafe's answer discriminant and protocol-native fields.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Answer {
    Noul {
        noul: Probability,
    },
    Choice {
        choice: String,
        probabilities: BTreeMap<String, Probability>,
        confidence: Confidence,
    },
    Score {
        score: f64,
        probabilities: BTreeMap<String, Probability>,
        legend: BTreeMap<String, NativeContent>,
        confidence: Confidence,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireResponse {
    model: String,
    answers: BTreeMap<String, Answer>,
    usage: Usage,
}

/// A TypeSafe response decoded and validated against its named question map.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Response {
    model: String,
    answers: BTreeMap<String, Answer>,
    usage: Usage,
}

impl Response {
    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn answers(&self) -> &BTreeMap<String, Answer> {
        &self.answers
    }

    pub fn usage(&self) -> &Usage {
        &self.usage
    }

    /// Parse strict JSON and validate exact answer keys, answer types, allowed
    /// choices, score indices, and score range against `request`.
    pub fn decode_json(bytes: &[u8], request: &Request) -> Result<Self, DecisionCodecError> {
        request
            .validate()
            .map_err(DecisionCodecError::InvalidRequest)?;
        let value = parse_unique_json_with_depth(bytes, MAX_DECISION_RESPONSE_JSON_DEPTH)?;
        let raw: WireResponse = decode_response_value(value)?;
        validate_response(raw, request)
    }
}

fn validate_response(raw: WireResponse, request: &Request) -> Result<Response, DecisionCodecError> {
    let invalid = |error| DecisionCodecError::InvalidResponse(error);
    if !nonblank(&raw.model) {
        return Err(invalid(DecisionValidationError::EmptyResponseModelId));
    }
    if !same_keys(&raw.answers, &request.questions) {
        return Err(invalid(DecisionValidationError::AnswerNamesMismatch));
    }

    for (name, question) in &request.questions {
        let answer = raw
            .answers
            .get(name)
            .ok_or_else(|| invalid(DecisionValidationError::AnswerNamesMismatch))?;
        match (question, answer) {
            (Question::Noul { .. }, Answer::Noul { .. }) => {}
            (
                Question::Choice { criteria, .. },
                Answer::Choice {
                    choice,
                    probabilities,
                    ..
                },
            ) => {
                if !criteria.contains_key(choice) {
                    return Err(invalid(DecisionValidationError::ChoiceNotRequested));
                }
                if !same_keys(probabilities, criteria) {
                    return Err(invalid(DecisionValidationError::DistributionMismatch));
                }
                if !probability_distribution_sums_to_one(probabilities.values().copied()) {
                    return Err(invalid(
                        DecisionValidationError::ProbabilityDistributionMismatch,
                    ));
                }
                if !selected_choice_has_max_probability(
                    probabilities[choice],
                    probabilities.values().copied(),
                ) {
                    return Err(invalid(DecisionValidationError::ChoiceProbabilityMismatch));
                }
            }
            (
                Question::Score { criteria, .. },
                Answer::Score {
                    score,
                    probabilities,
                    legend,
                    ..
                },
            ) => {
                if !score.is_finite() || *score < 0.0 || *score > (criteria.len() - 1) as f64 {
                    return Err(invalid(DecisionValidationError::ScoreOutOfRange));
                }
                let expected = score_index_keys(criteria.len());
                if probabilities.keys().cloned().collect::<BTreeSet<_>>() != expected
                    || legend.keys().cloned().collect::<BTreeSet<_>>() != expected
                {
                    return Err(invalid(DecisionValidationError::DistributionMismatch));
                }
                for (index, criterion) in criteria.iter().enumerate() {
                    if legend.get(&index.to_string()) != Some(criterion) {
                        return Err(invalid(DecisionValidationError::ScoreLegendMismatch));
                    }
                }
                if !probability_distribution_sums_to_one(probabilities.values().copied()) {
                    return Err(invalid(
                        DecisionValidationError::ProbabilityDistributionMismatch,
                    ));
                }
                if !score_matches_expected_value(
                    *score,
                    (0..criteria.len()).map(|index| (index, probabilities[&index.to_string()])),
                ) {
                    return Err(invalid(DecisionValidationError::ScoreDistributionMismatch));
                }
            }
            _ => return Err(invalid(DecisionValidationError::AnswerTypeMismatch)),
        }
    }

    Ok(Response {
        model: raw.model,
        answers: raw.answers,
        usage: raw.usage,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn questions() -> BTreeMap<String, Question> {
        BTreeMap::from([
            (
                "billing".into(),
                Question::Noul {
                    instructions: Some(NativeContent::text("Is this about billing?")),
                    criteria: None,
                },
            ),
            (
                "team".into(),
                Question::Choice {
                    instructions: Some(NativeContent::text("Which team should handle it?")),
                    criteria: BTreeMap::from([
                        (
                            "billing".into(),
                            Some(NativeContent::text("Payments and refunds")),
                        ),
                        ("support".into(), None),
                    ]),
                },
            ),
            (
                "urgency".into(),
                Question::Score {
                    instructions: Some(NativeContent::text("How urgent is it?")),
                    criteria: vec![
                        NativeContent::text("Can wait"),
                        NativeContent::text("Needs attention soon"),
                        NativeContent::text("Needs immediate attention"),
                    ],
                },
            ),
        ])
    }

    fn request() -> Request {
        Request::new(
            "jev-latest",
            NativeContent::Object(serde_json::Map::from_iter([(
                "message".into(),
                json!("Please refund the duplicate charge on my account."),
            )])),
            questions(),
        )
    }

    const RESPONSE: &str = r#"{
      "model":"jev-1.14.0",
      "answers":{
        "billing":{"type":"noul","noul":0.97},
        "team":{"type":"choice","choice":"billing","probabilities":{"billing":0.96,"support":0.04},"confidence":0.96},
        "urgency":{"type":"score","score":1.1,"legend":{"0":"Can wait","1":"Needs attention soon","2":"Needs immediate attention"},"probabilities":{"0":0.1,"1":0.7,"2":0.2},"confidence":0.7}
      },
      "usage":{"input_tokens":120,"output_tokens":3}
    }"#;

    #[test]
    fn rejects_inconsistent_distributions() {
        let request = request();
        for (answer, error) in [
            (
                json!({"type":"choice", "choice":"billing", "probabilities":{"billing":0.0,"support":0.0}, "confidence":0.5}),
                DecisionValidationError::ProbabilityDistributionMismatch,
            ),
            (
                json!({"type":"choice", "choice":"billing", "probabilities":{"billing":0.0,"support":1.0}, "confidence":0.5}),
                DecisionValidationError::ChoiceProbabilityMismatch,
            ),
        ] {
            let mut response: Value = serde_json::from_str(RESPONSE).unwrap();
            response["answers"]["team"] = answer;
            assert_eq!(
                Response::decode_json(&serde_json::to_vec(&response).unwrap(), &request),
                Err(DecisionCodecError::InvalidResponse(error))
            );
        }
        let mut response: Value = serde_json::from_str(RESPONSE).unwrap();
        response["answers"]["urgency"]["score"] = json!(0.0);
        assert_eq!(
            Response::decode_json(&serde_json::to_vec(&response).unwrap(), &request),
            Err(DecisionCodecError::InvalidResponse(
                DecisionValidationError::ScoreDistributionMismatch
            ))
        );
    }

    #[test]
    fn criteria_depth_includes_container_and_legend_round_trips() {
        for depth in [63, 64] {
            let mut value = json!("leaf");
            for _ in 0..depth {
                value = json!([value]);
            }
            let criterion: NativeContent = serde_json::from_value(value).unwrap();
            let request = Request::new(
                "jev-latest",
                NativeContent::text("state"),
                BTreeMap::from([(
                    "q".into(),
                    Question::Score {
                        instructions: None,
                        criteria: vec![criterion.clone(), NativeContent::text("other")],
                    },
                )]),
            );
            if depth == 64 {
                assert_eq!(
                    request.validate(),
                    Err(DecisionValidationError::JsonDepthOutOfRange)
                );
            } else {
                request.encode_json().unwrap();
                let response = json!({"model":"jev-resolved", "answers":{"q":{"type":"score", "score":0.5, "probabilities":{"0":0.5,"1":0.5}, "legend":{"0":criterion,"1":"other"}, "confidence":0.5}}, "usage":{"input_tokens":1,"output_tokens":1}});
                Response::decode_json(&serde_json::to_vec(&response).unwrap(), &request).unwrap();
            }
        }
    }

    #[test]
    fn native_map_contract_round_trips_and_preserves_alias_resolution_and_usage() {
        let request = request();
        let encoded: Value = serde_json::from_slice(&request.encode_json().unwrap()).unwrap();
        assert_eq!(encoded["model"], "jev-latest");
        assert_eq!(
            encoded["state"]["message"],
            "Please refund the duplicate charge on my account."
        );
        assert_eq!(
            encoded["questions"]["team"]["criteria"]["support"],
            Value::Null
        );
        assert!(encoded["questions"].is_object());

        let response = Response::decode_json(RESPONSE.as_bytes(), &request).unwrap();
        assert_eq!(response.model(), "jev-1.14.0");
        assert_eq!(response.answers().len(), 3);
        assert_eq!(response.usage().input_tokens, 120);
        assert!(matches!(response.answers()["billing"], Answer::Noul { .. }));
    }

    #[test]
    fn structured_native_instructions_and_null_criteria_are_preserved() {
        let request = Request::new(
            "model-explicit",
            NativeContent::text("shared state"),
            BTreeMap::from([(
                "structured".into(),
                Question::Noul {
                    instructions: Some(NativeContent::Object(serde_json::Map::from_iter([(
                        "question".into(),
                        json!("Is this true?"),
                    )]))),
                    criteria: Some(None),
                },
            )]),
        );
        let encoded: Value = serde_json::from_slice(&request.encode_json().unwrap()).unwrap();
        assert_eq!(
            encoded["questions"]["structured"]["instructions"]["question"],
            "Is this true?"
        );
        assert_eq!(encoded["questions"]["structured"]["criteria"], Value::Null);
    }

    #[test]
    fn missing_extra_keys_wrong_type_choices_distributions_and_scores_are_rejected() {
        let request = request();
        let missing = RESPONSE.replace("\"billing\":{\"type\":\"noul\",\"noul\":0.97},", "");
        assert!(matches!(
            Response::decode_json(missing.as_bytes(), &request),
            Err(DecisionCodecError::InvalidResponse(
                DecisionValidationError::AnswerNamesMismatch
            ))
        ));

        let extra = RESPONSE.replace(
            "\"answers\":{",
            "\"answers\":{\"other\":{\"type\":\"noul\",\"noul\":0.5},",
        );
        assert!(matches!(
            Response::decode_json(extra.as_bytes(), &request),
            Err(DecisionCodecError::InvalidResponse(
                DecisionValidationError::AnswerNamesMismatch
            ))
        ));

        let wrong_type = RESPONSE.replace(
            r#""billing":{"type":"noul","noul":0.97}"#,
            r#""billing":{"type":"choice","choice":"billing","probabilities":{"billing":1.0},"confidence":1.0}"#,
        );
        assert!(matches!(
            Response::decode_json(wrong_type.as_bytes(), &request),
            Err(DecisionCodecError::InvalidResponse(
                DecisionValidationError::AnswerTypeMismatch
            ))
        ));

        let wrong_choice = RESPONSE.replace("\"choice\":\"billing\"", "\"choice\":\"other\"");
        assert!(matches!(
            Response::decode_json(wrong_choice.as_bytes(), &request),
            Err(DecisionCodecError::InvalidResponse(
                DecisionValidationError::ChoiceNotRequested
            ))
        ));

        let extra_distribution =
            RESPONSE.replace("\"support\":0.04", "\"support\":0.04,\"other\":0.0");
        assert!(matches!(
            Response::decode_json(extra_distribution.as_bytes(), &request),
            Err(DecisionCodecError::InvalidResponse(
                DecisionValidationError::DistributionMismatch
            ))
        ));

        let wrong_legend = RESPONSE.replace("\"1\":\"Needs attention soon\"", "\"1\":\"wrong\"");
        assert!(matches!(
            Response::decode_json(wrong_legend.as_bytes(), &request),
            Err(DecisionCodecError::InvalidResponse(
                DecisionValidationError::ScoreLegendMismatch
            ))
        ));

        let out_of_range = RESPONSE.replace("\"score\":1.1", "\"score\":3.0");
        assert!(matches!(
            Response::decode_json(out_of_range.as_bytes(), &request),
            Err(DecisionCodecError::InvalidResponse(
                DecisionValidationError::ScoreOutOfRange
            ))
        ));

        let invalid_probability = RESPONSE.replace("\"noul\":0.97", "\"noul\":1.01");
        assert!(matches!(
            Response::decode_json(invalid_probability.as_bytes(), &request),
            Err(DecisionCodecError::InvalidResponse(
                DecisionValidationError::InvalidWireShape
            ))
        ));

        let invalid_confidence = RESPONSE.replace("\"confidence\":0.96", "\"confidence\":-0.01");
        assert!(matches!(
            Response::decode_json(invalid_confidence.as_bytes(), &request),
            Err(DecisionCodecError::InvalidResponse(
                DecisionValidationError::InvalidWireShape
            ))
        ));
    }

    #[test]
    fn native_request_limits_and_invalid_content_are_rejected_before_encoding() {
        let no_model = Request::new("  ", NativeContent::text("state"), questions());
        assert_eq!(
            no_model.validate(),
            Err(DecisionValidationError::EmptyModelId)
        );

        let one_level = Request::new(
            "model",
            NativeContent::text("state"),
            BTreeMap::from([(
                "score".into(),
                Question::Score {
                    instructions: None,
                    criteria: vec![NativeContent::text("only level")],
                },
            )]),
        );
        assert_eq!(
            one_level.validate(),
            Err(DecisionValidationError::ScoreLevelCountOutOfRange)
        );

        for count in [1, 255] {
            let criteria = (0..count)
                .map(|index| (format!("option_{index}"), None))
                .collect();
            let request = Request::new(
                "model",
                NativeContent::text("state"),
                BTreeMap::from([(
                    "choice".into(),
                    Question::Choice {
                        instructions: None,
                        criteria,
                    },
                )]),
            );
            assert!(request.validate().is_ok());
        }
        let criteria = (0..256)
            .map(|index| (format!("option_{index}"), None))
            .collect();
        let request = Request::new(
            "model",
            NativeContent::text("state"),
            BTreeMap::from([(
                "choice".into(),
                Question::Choice {
                    instructions: None,
                    criteria,
                },
            )]),
        );
        assert_eq!(
            request.validate(),
            Err(DecisionValidationError::ChoiceCountOutOfRange)
        );

        for count in [2, 10] {
            let criteria = (0..count)
                .map(|index| NativeContent::text(format!("level_{index}")))
                .collect();
            let request = Request::new(
                "model",
                NativeContent::text("state"),
                BTreeMap::from([(
                    "score".into(),
                    Question::Score {
                        instructions: None,
                        criteria,
                    },
                )]),
            );
            assert!(request.validate().is_ok());
        }

        let many_questions = (0..401)
            .map(|index| {
                (
                    format!("question_{index}"),
                    Question::Noul {
                        instructions: None,
                        criteria: None,
                    },
                )
            })
            .collect();
        let request = Request::new("model", NativeContent::text("state"), many_questions);
        assert!(request.validate().is_ok());
    }
}
