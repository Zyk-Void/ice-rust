//! Offline DTOs for Inception's `/v1/decisions` contract.
//!
//! This module follows the published OpenAPI schema for `noul`, `choice`, and
//! `score`. The guide-only `extract` extension is intentionally deferred.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::{
    decode_response_value, encode_bounded_json, nonblank, parse_unique_json_with_depth,
    probability_distribution_sums_to_one, same_string_keys as same_keys, score_index_keys,
    score_matches_expected_value, selected_choice_has_max_probability, Confidence,
    DecisionCodecError, DecisionValidationError, NativeContent, Probability,
    MAX_DECISION_JSON_DEPTH, MAX_DECISION_RESPONSE_JSON_DEPTH,
};

/// Inception Noul criteria. At least one polarity description is required if
/// the criteria object is supplied.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NoulCriteria {
    #[serde(rename = "true", skip_serializing_if = "Option::is_none")]
    pub yes: Option<NativeContent>,
    #[serde(rename = "false", skip_serializing_if = "Option::is_none")]
    pub no: Option<NativeContent>,
}

/// An Inception-native typed question, keyed by caller-chosen name.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Question {
    Noul {
        /// Outer `None` omits the field; `Some(None)` emits JSON null.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        instructions: Option<Option<NativeContent>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        criteria: Option<Option<NoulCriteria>>,
    },
    Choice {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        instructions: Option<Option<NativeContent>>,
        criteria: BTreeMap<String, Option<NativeContent>>,
    },
    Score {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        instructions: Option<Option<NativeContent>>,
        criteria: Vec<NativeContent>,
    },
}

/// Request for Inception Decisions.
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
                    if !(1..=10).contains(&criteria.len()) {
                        return Err(DecisionValidationError::ScoreLevelCountOutOfRange);
                    }
                    validate_distinct_criteria(criteria)?;
                }
            }
        }
        Ok(())
    }

    /// Encode a validated request. Inception documents a 20 MiB default whole
    /// request limit. D1 does not model image parts, so image markers are
    /// rejected instead of passing through as an undocumented modality.
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
            20 * 1024 * 1024,
        )
    }
}

fn validate_content(content: &NativeContent) -> Result<(), DecisionValidationError> {
    if content.max_container_depth() > MAX_DECISION_JSON_DEPTH {
        return Err(DecisionValidationError::JsonDepthOutOfRange);
    }
    if content.contains_inception_image_part() {
        return Err(DecisionValidationError::UnsupportedImagePart);
    }
    Ok(())
}

fn validate_criterion(content: &NativeContent) -> Result<(), DecisionValidationError> {
    // Include the criteria map/array in the local content depth ceiling.
    if content.max_container_depth() >= MAX_DECISION_JSON_DEPTH {
        return Err(DecisionValidationError::JsonDepthOutOfRange);
    }
    validate_content(content)
}

fn validate_optional_instructions(
    instructions: Option<&Option<NativeContent>>,
) -> Result<(), DecisionValidationError> {
    if let Some(Some(instructions)) = instructions {
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

/// An Inception answer discriminated by the original question type.
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
        legend: BTreeMap<String, NativeContent>,
        probabilities: BTreeMap<String, Probability>,
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

/// An Inception response decoded and validated against the named question map.
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
    /// choices, score indices, score labels, and score range against `request`.
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
                    legend,
                    probabilities,
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

    fn request() -> Request {
        Request::new(
            "mercury-decide",
            NativeContent::Object(serde_json::Map::from_iter([(
                "message".into(),
                json!("Please refund the duplicate charge on my account."),
            )])),
            BTreeMap::from([
                (
                    "billing".into(),
                    Question::Noul {
                        instructions: Some(Some(NativeContent::text(
                            "Is this request about billing?",
                        ))),
                        criteria: None,
                    },
                ),
                (
                    "team".into(),
                    Question::Choice {
                        instructions: None,
                        criteria: BTreeMap::from([
                            (
                                "billing".into(),
                                Some(NativeContent::text("Payments, charges, and refunds")),
                            ),
                            (
                                "support".into(),
                                Some(NativeContent::text("Technical support and other requests")),
                            ),
                        ]),
                    },
                ),
                (
                    "urgency".into(),
                    Question::Score {
                        instructions: Some(Some(NativeContent::text(
                            "How urgent is this request?",
                        ))),
                        criteria: vec![
                            NativeContent::text("Can wait"),
                            NativeContent::text("Needs attention soon"),
                            NativeContent::text("Needs immediate attention"),
                        ],
                    },
                ),
            ]),
        )
    }

    const RESPONSE: &str = r#"{
      "model":"mercury-decide",
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
        for (choice, probabilities, error) in [
            (
                "billing",
                json!({"billing":0.0,"support":0.0}),
                DecisionValidationError::ProbabilityDistributionMismatch,
            ),
            (
                "billing",
                json!({"billing":0.0,"support":1.0}),
                DecisionValidationError::ChoiceProbabilityMismatch,
            ),
        ] {
            let mut response: Value = serde_json::from_str(RESPONSE).unwrap();
            response["answers"]["team"]["choice"] = json!(choice);
            response["answers"]["team"]["probabilities"] = probabilities;
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
    fn published_openapi_map_request_and_response_round_trip() {
        let request = request();
        let encoded: Value = serde_json::from_slice(&request.encode_json().unwrap()).unwrap();
        assert_eq!(encoded["model"], "mercury-decide");
        assert_eq!(encoded["questions"]["billing"]["type"], "noul");
        assert_eq!(
            encoded["questions"]["team"]["criteria"]["billing"],
            "Payments, charges, and refunds"
        );
        assert_eq!(
            encoded["questions"]["urgency"]["criteria"][1],
            "Needs attention soon"
        );
        assert!(encoded["questions"].is_object());

        let response = Response::decode_json(RESPONSE.as_bytes(), &request).unwrap();
        assert_eq!(response.model(), "mercury-decide");
        assert_eq!(response.answers().len(), 3);
        assert_eq!(response.usage().input_tokens, 120);
        assert!(matches!(response.answers()["billing"], Answer::Noul { .. }));
        if let Answer::Score {
            score, confidence, ..
        } = &response.answers()["urgency"]
        {
            assert_eq!(*score, 1.1);
            assert_eq!(confidence.get(), 0.7);
        } else {
            panic!("score answer should remain typed");
        }
    }

    #[test]
    fn instructions_preserve_omitted_null_and_json_values() {
        let request = Request::new(
            "explicit-model",
            NativeContent::text("state"),
            BTreeMap::from([
                (
                    "omitted".into(),
                    Question::Choice {
                        instructions: None,
                        criteria: BTreeMap::from([("one".into(), None)]),
                    },
                ),
                (
                    "null".into(),
                    Question::Noul {
                        instructions: Some(None),
                        criteria: Some(Some(NoulCriteria {
                            yes: Some(NativeContent::text("yes")),
                            no: None,
                        })),
                    },
                ),
            ]),
        );
        let encoded: Value = serde_json::from_slice(&request.encode_json().unwrap()).unwrap();
        assert!(encoded["questions"]["omitted"]
            .get("instructions")
            .is_none());
        assert_eq!(encoded["questions"]["null"]["instructions"], Value::Null);
        assert_eq!(
            encoded["questions"]["omitted"]["criteria"]["one"],
            Value::Null
        );
    }

    #[test]
    fn rejects_missing_extra_names_wrong_types_choices_distributions_and_scores() {
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

        let missing_probability = RESPONSE.replace(",\"support\":0.04", "");
        assert!(matches!(
            Response::decode_json(missing_probability.as_bytes(), &request),
            Err(DecisionCodecError::InvalidResponse(
                DecisionValidationError::DistributionMismatch
            ))
        ));

        let missing_legend = RESPONSE.replace(",\"2\":\"Needs immediate attention\"", "");
        assert!(matches!(
            Response::decode_json(missing_legend.as_bytes(), &request),
            Err(DecisionCodecError::InvalidResponse(
                DecisionValidationError::DistributionMismatch
            ))
        ));

        let out_of_range = RESPONSE.replace("\"score\":1.1", "\"score\":3.0");
        assert!(matches!(
            Response::decode_json(out_of_range.as_bytes(), &request),
            Err(DecisionCodecError::InvalidResponse(
                DecisionValidationError::ScoreOutOfRange
            ))
        ));

        let wrong_legend = RESPONSE.replace("\"1\":\"Needs attention soon\"", "\"1\":\"wrong\"");
        assert!(matches!(
            Response::decode_json(wrong_legend.as_bytes(), &request),
            Err(DecisionCodecError::InvalidResponse(
                DecisionValidationError::ScoreLegendMismatch
            ))
        ));

        let invalid_probability = RESPONSE.replace("\"noul\":0.97", "\"noul\":1.01");
        assert!(matches!(
            Response::decode_json(invalid_probability.as_bytes(), &request),
            Err(DecisionCodecError::InvalidResponse(
                DecisionValidationError::InvalidWireShape
            ))
        ));
    }

    #[test]
    fn image_parts_are_rejected_and_one_score_criterion_matches_openapi() {
        let image = NativeContent::Object(serde_json::Map::from_iter([
            ("type".into(), json!("image_url")),
            (
                "image_url".into(),
                json!({"url": "data:image/png;base64,AA=="}),
            ),
        ]));
        let request = Request::new(
            "model",
            image,
            BTreeMap::from([(
                "one_level".into(),
                Question::Score {
                    instructions: None,
                    criteria: vec![NativeContent::text("only documented level")],
                },
            )]),
        );
        assert_eq!(
            request.validate(),
            Err(DecisionValidationError::UnsupportedImagePart)
        );

        let single_score = Request::new(
            "model",
            NativeContent::text("state"),
            BTreeMap::from([(
                "one_level".into(),
                Question::Score {
                    instructions: None,
                    criteria: vec![NativeContent::text("only documented level")],
                },
            )]),
        );
        assert!(single_score.validate().is_ok());

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

        for count in [1, 10] {
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
        let criteria = (0..11)
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
        assert_eq!(
            request.validate(),
            Err(DecisionValidationError::ScoreLevelCountOutOfRange)
        );
    }
}
