//! Offline DTOs for OpenAI's `/v1/decisions` endpoint.
//!
//! D1 supports text input and one `input_json` part in a user message, plus
//! predicate, choice, and score questions. It intentionally does not model
//! message history, tools, or images.

use std::collections::HashSet;

use serde::ser::SerializeSeq;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    decode_response_value, encode_bounded_json, nonblank, parse_unique_json,
    probability_distribution_sums_to_one, score_matches_expected_value, value_count,
    value_max_container_depth, Confidence, DecisionCodecError, DecisionValidationError,
    Probability,
};

/// OpenAI input supported by this D1 slice.
#[derive(Debug, Clone, PartialEq)]
pub enum Input {
    /// The endpoint's direct text-string input form.
    Text(String),
    /// A JSON object or array, encoded as a user `input_json` part.
    Json(Value),
}

impl Serialize for Input {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::Text(text) => serializer.serialize_str(text),
            Self::Json(data) => {
                #[derive(Serialize)]
                struct JsonPart<'a> {
                    #[serde(rename = "type")]
                    kind: &'static str,
                    data: &'a Value,
                }

                #[derive(Serialize)]
                struct UserMessage<'a> {
                    role: &'static str,
                    content: [JsonPart<'a>; 1],
                }

                let message = UserMessage {
                    role: "user",
                    content: [JsonPart {
                        kind: "input_json",
                        data,
                    }],
                };
                let mut sequence = serializer.serialize_seq(Some(1))?;
                sequence.serialize_element(&message)?;
                sequence.end()
            }
        }
    }
}

/// A named typed question in OpenAI's ordered question array.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Question {
    Predicate {
        name: String,
        instructions: String,
    },
    Choice {
        name: String,
        instructions: String,
        choices: Vec<ChoiceOption>,
    },
    Score {
        name: String,
        instructions: String,
        levels: Vec<ScoreLevel>,
    },
}

impl Question {
    pub fn name(&self) -> &str {
        match self {
            Self::Predicate { name, .. } | Self::Choice { name, .. } | Self::Score { name, .. } => {
                name
            }
        }
    }
}

/// String or boolean choice value. A string such as `"true"` stays distinct
/// from the boolean `true`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ChoiceValue {
    Bool(bool),
    String(String),
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChoiceOption {
    pub value: ChoiceValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ScoreLevel {
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Validated request source for the OpenAI Decisions wire contract.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    model: String,
    input: Input,
    questions: Vec<Question>,
}

impl Request {
    /// Construct a request. Call [`Request::encode_json`] to validate and
    /// encode it before passing the bytes to a transport.
    pub fn new(model: impl Into<String>, input: Input, questions: Vec<Question>) -> Self {
        Self {
            model: model.into(),
            input,
            questions,
        }
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn input(&self) -> &Input {
        &self.input
    }

    pub fn questions(&self) -> &[Question] {
        &self.questions
    }

    pub fn validate(&self) -> Result<(), DecisionValidationError> {
        if !nonblank(&self.model) {
            return Err(DecisionValidationError::EmptyModelId);
        }

        match &self.input {
            Input::Json(value) => {
                let is_object_or_array = matches!(value, Value::Object(_) | Value::Array(_));
                if !is_object_or_array
                    || value_max_container_depth(value) > 32
                    || value_count(value) > 10_000
                {
                    return Err(DecisionValidationError::OpenAiJsonInputOutOfRange);
                }
                encode_bounded_json(value, 256 * 1024)
                    .map_err(|_| DecisionValidationError::OpenAiJsonInputOutOfRange)?;
            }
            Input::Text(_) => {}
        }

        if self.questions.is_empty() {
            return Err(DecisionValidationError::EmptyQuestions);
        }
        if self.questions.len() > 400 {
            return Err(DecisionValidationError::TooManyQuestions);
        }

        let mut names = HashSet::with_capacity(self.questions.len());
        for question in &self.questions {
            if !nonblank(question.name()) {
                return Err(DecisionValidationError::EmptyQuestionName);
            }
            if !names.insert(question.name()) {
                return Err(DecisionValidationError::DuplicateQuestionName);
            }

            match question {
                Question::Predicate { .. } => {}
                Question::Choice { choices, .. } => {
                    if !(2..=255).contains(&choices.len()) {
                        return Err(DecisionValidationError::ChoiceCountOutOfRange);
                    }
                    let mut values = HashSet::with_capacity(choices.len());
                    if choices.iter().any(|choice| !values.insert(&choice.value)) {
                        return Err(DecisionValidationError::DuplicateChoice);
                    }
                }
                Question::Score { levels, .. } => {
                    if !(2..=10).contains(&levels.len()) {
                        return Err(DecisionValidationError::ScoreLevelCountOutOfRange);
                    }
                    let mut labels = HashSet::with_capacity(levels.len());
                    if levels
                        .iter()
                        .any(|level| !nonblank(&level.label) || !labels.insert(&level.label))
                    {
                        return Err(DecisionValidationError::InvalidScoreLevelLabels);
                    }
                }
            }
        }
        Ok(())
    }

    /// Encode a validated request as compact JSON for an external transport.
    pub fn encode_json(&self) -> Result<Vec<u8>, DecisionCodecError> {
        self.validate()
            .map_err(DecisionCodecError::InvalidRequest)?;
        #[derive(Serialize)]
        struct WireRequest<'a> {
            model: &'a str,
            input: &'a Input,
            questions: &'a [Question],
        }

        serde_json::to_vec(&WireRequest {
            model: &self.model,
            input: &self.input,
            questions: &self.questions,
        })
        .map_err(|_| DecisionCodecError::SerializationFailed)
    }
}

/// A discriminated answer returned by OpenAI Decisions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Answer {
    Predicate {
        #[serde(default)]
        name: Option<String>,
        probability: Probability,
    },
    Choice {
        #[serde(default)]
        name: Option<String>,
        choice: ChoiceValue,
        probabilities: Vec<ChoiceProbability>,
        confidence: Confidence,
    },
    Score {
        #[serde(default)]
        name: Option<String>,
        score: f64,
        probabilities: Vec<ScoreProbability>,
        confidence: Confidence,
    },
    /// A refusal replaces this question's classification; other answers may
    /// still be present in the same response.
    Refusal {
        #[serde(default)]
        name: Option<String>,
    },
}

impl Answer {
    fn name(&self) -> Option<&str> {
        match self {
            Self::Predicate { name, .. }
            | Self::Choice { name, .. }
            | Self::Score { name, .. }
            | Self::Refusal { name } => name.as_deref(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChoiceProbability {
    pub value: ChoiceValue,
    pub probability: Probability,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScoreProbability {
    pub value: usize,
    pub label: String,
    pub probability: Probability,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputTokenDetails {
    pub cache_write_tokens: u64,
    pub cached_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputTokenDetails {
    pub reasoning_tokens: u64,
}

/// OpenAI's protocol-specific usage details.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Usage {
    pub input_tokens: u64,
    pub input_tokens_details: InputTokenDetails,
    pub output_tokens: u64,
    pub output_tokens_details: OutputTokenDetails,
    pub total_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireResponse {
    model: String,
    answers: Vec<Answer>,
    usage: Usage,
}

/// A response decoded and validated against its original request.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Response {
    model: String,
    answers: Vec<Answer>,
    usage: Usage,
}

impl Response {
    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn answers(&self) -> &[Answer] {
        &self.answers
    }

    pub fn usage(&self) -> &Usage {
        &self.usage
    }

    /// Parse strict JSON and validate answer count, order, names, types,
    /// choices, score labels, and score range against `request`.
    pub fn decode_json(bytes: &[u8], request: &Request) -> Result<Self, DecisionCodecError> {
        request
            .validate()
            .map_err(DecisionCodecError::InvalidRequest)?;
        let value = parse_unique_json(bytes)?;
        let raw: WireResponse = decode_response_value(value)?;
        validate_response(raw, request)
    }
}

fn validate_response(raw: WireResponse, request: &Request) -> Result<Response, DecisionCodecError> {
    let invalid = |error| DecisionCodecError::InvalidResponse(error);
    if !nonblank(&raw.model) {
        return Err(invalid(DecisionValidationError::EmptyResponseModelId));
    }
    if raw.answers.len() != request.questions.len() {
        return Err(invalid(DecisionValidationError::AnswerCountMismatch));
    }

    for (answer, question) in raw.answers.iter().zip(&request.questions) {
        if answer.name() != Some(question.name()) {
            return Err(invalid(DecisionValidationError::AnswerNamesMismatch));
        }

        match (question, answer) {
            (_, Answer::Refusal { .. }) => {}
            (Question::Predicate { .. }, Answer::Predicate { .. }) => {}
            (
                Question::Choice { choices, .. },
                Answer::Choice {
                    choice,
                    probabilities,
                    ..
                },
            ) => {
                let allowed = choices
                    .iter()
                    .map(|option| &option.value)
                    .collect::<HashSet<_>>();
                if !allowed.contains(choice) {
                    return Err(invalid(DecisionValidationError::ChoiceNotRequested));
                }
                if probabilities.len() != allowed.len() {
                    return Err(invalid(DecisionValidationError::DistributionMismatch));
                }
                let mut seen = HashSet::with_capacity(probabilities.len());
                for item in probabilities {
                    if !allowed.contains(&item.value) || !seen.insert(&item.value) {
                        return Err(invalid(DecisionValidationError::DistributionMismatch));
                    }
                }
                if !probability_distribution_sums_to_one(
                    probabilities.iter().map(|item| item.probability),
                ) {
                    return Err(invalid(
                        DecisionValidationError::ProbabilityDistributionMismatch,
                    ));
                }
            }
            (
                Question::Score { levels, .. },
                Answer::Score {
                    score,
                    probabilities,
                    ..
                },
            ) => {
                if !score.is_finite() || *score < 0.0 || *score > (levels.len() - 1) as f64 {
                    return Err(invalid(DecisionValidationError::ScoreOutOfRange));
                }
                if probabilities.len() != levels.len() {
                    return Err(invalid(DecisionValidationError::DistributionMismatch));
                }
                let mut seen = HashSet::with_capacity(probabilities.len());
                for item in probabilities {
                    let Some(level) = levels.get(item.value) else {
                        return Err(invalid(DecisionValidationError::DistributionMismatch));
                    };
                    if !seen.insert(item.value) || item.label != level.label {
                        return Err(invalid(DecisionValidationError::ScoreLegendMismatch));
                    }
                }
                if !probability_distribution_sums_to_one(
                    probabilities.iter().map(|item| item.probability),
                ) {
                    return Err(invalid(
                        DecisionValidationError::ProbabilityDistributionMismatch,
                    ));
                }
                if !score_matches_expected_value(
                    *score,
                    probabilities
                        .iter()
                        .map(|item| (item.value, item.probability)),
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
            "gpt-6-luna",
            Input::Text("A support ticket asks about a duplicate charge.".into()),
            vec![
                Question::Predicate {
                    name: "billing".into(),
                    instructions: "Is this about billing?".into(),
                },
                Question::Choice {
                    name: "team".into(),
                    instructions: "Which team should handle it?".into(),
                    choices: vec![
                        ChoiceOption {
                            value: ChoiceValue::String("billing".into()),
                            description: Some("Payments and refunds".into()),
                        },
                        ChoiceOption {
                            value: ChoiceValue::Bool(true),
                            description: None,
                        },
                    ],
                },
                Question::Score {
                    name: "urgency".into(),
                    instructions: "How urgent is it?".into(),
                    levels: vec![
                        ScoreLevel {
                            label: "can wait".into(),
                            description: None,
                        },
                        ScoreLevel {
                            label: "soon".into(),
                            description: None,
                        },
                        ScoreLevel {
                            label: "immediate".into(),
                            description: None,
                        },
                    ],
                },
            ],
        )
    }

    const RESPONSE: &str = r#"{
      "model":"gpt-6-luna-2026-10-01",
      "answers":[
        {"type":"predicate","name":"billing","probability":0.97},
        {"type":"choice","name":"team","choice":"billing","probabilities":[
          {"value":"billing","probability":0.96},{"value":true,"probability":0.04}
        ],"confidence":0.71},
        {"type":"score","name":"urgency","score":1.1,"probabilities":[
          {"value":0,"label":"can wait","probability":0.1},
          {"value":1,"label":"soon","probability":0.7},
          {"value":2,"label":"immediate","probability":0.2}
        ],"confidence":0.55}
      ],
      "usage":{"input_tokens":120,"input_tokens_details":{"cache_write_tokens":0,"cached_tokens":0},
        "output_tokens":3,"output_tokens_details":{"reasoning_tokens":1},"total_tokens":123}
    }"#;

    #[test]
    fn documented_question_array_and_response_round_trip_with_alias_and_usage() {
        let request = request();
        let encoded: Value = serde_json::from_slice(&request.encode_json().unwrap()).unwrap();
        assert_eq!(encoded["model"], "gpt-6-luna");
        assert_eq!(
            encoded["input"],
            "A support ticket asks about a duplicate charge."
        );
        assert_eq!(encoded["questions"][0]["type"], "predicate");
        assert_eq!(encoded["questions"][1]["choices"][1]["value"], true);
        assert_eq!(encoded["questions"][2]["levels"][1]["label"], "soon");

        let response = Response::decode_json(RESPONSE.as_bytes(), &request).unwrap();
        assert_eq!(response.model(), "gpt-6-luna-2026-10-01");
        assert_eq!(response.answers().len(), 3);
        assert!(matches!(response.answers()[1], Answer::Choice { .. }));
        assert_eq!(response.usage().input_tokens_details.cached_tokens, 0);
        assert_eq!(response.usage().output_tokens_details.reasoning_tokens, 1);
    }

    #[test]
    fn json_input_maps_to_input_json_data_part() {
        let request = Request::new(
            "explicit-model",
            Input::Json(json!({"ticket": {"body": "duplicate charge"}})),
            vec![Question::Predicate {
                name: "duplicate_charge".into(),
                instructions: "Does the ticket describe a duplicate charge?".into(),
            }],
        );
        let encoded: Value = serde_json::from_slice(&request.encode_json().unwrap()).unwrap();
        assert_eq!(encoded["input"][0]["role"], "user");
        assert_eq!(encoded["input"][0]["content"][0]["type"], "input_json");
        assert_eq!(
            encoded["input"][0]["content"][0]["data"]["ticket"]["body"],
            "duplicate charge"
        );
    }

    #[test]
    fn rejects_inconsistent_distributions() {
        let request = request();
        let mut response: Value = serde_json::from_str(RESPONSE).unwrap();
        response["answers"][1]["probabilities"][0]["probability"] = json!(0.0);
        response["answers"][1]["probabilities"][1]["probability"] = json!(0.0);
        assert_eq!(
            Response::decode_json(&serde_json::to_vec(&response).unwrap(), &request),
            Err(DecisionCodecError::InvalidResponse(
                DecisionValidationError::ProbabilityDistributionMismatch
            ))
        );
        let mut response: Value = serde_json::from_str(RESPONSE).unwrap();
        response["answers"][2]["score"] = json!(0.0);
        assert_eq!(
            Response::decode_json(&serde_json::to_vec(&response).unwrap(), &request),
            Err(DecisionCodecError::InvalidResponse(
                DecisionValidationError::ScoreDistributionMismatch
            ))
        );
    }

    #[test]
    fn refusal_is_a_per_answer_variant_and_keeps_other_results() {
        let request = request();
        let response = RESPONSE.replace(
            r#"{"type":"choice","name":"team","choice":"billing","probabilities":[
          {"value":"billing","probability":0.96},{"value":true,"probability":0.04}
        ],"confidence":0.71}"#,
            r#"{"type":"refusal","name":"team"}"#,
        );
        let response = Response::decode_json(response.as_bytes(), &request).unwrap();
        assert!(matches!(response.answers()[0], Answer::Predicate { .. }));
        assert!(matches!(response.answers()[1], Answer::Refusal { .. }));
        assert!(matches!(response.answers()[2], Answer::Score { .. }));
    }

    #[test]
    fn invalid_request_names_types_distributions_scores_and_bounds_fail() {
        let duplicate_names = Request::new(
            "model",
            Input::Text("input".into()),
            vec![
                Question::Predicate {
                    name: "same".into(),
                    instructions: "first".into(),
                },
                Question::Predicate {
                    name: "same".into(),
                    instructions: "second".into(),
                },
            ],
        );
        assert_eq!(
            duplicate_names.validate(),
            Err(DecisionValidationError::DuplicateQuestionName)
        );

        let request = request();
        let wrong_type = RESPONSE.replace(
            r#"{"type":"predicate","name":"billing","probability":0.97}"#,
            r#"{"type":"choice","name":"billing","choice":true,"probabilities":[{"value":true,"probability":1.0}],"confidence":1.0}"#,
        );
        assert!(matches!(
            Response::decode_json(wrong_type.as_bytes(), &request),
            Err(DecisionCodecError::InvalidResponse(
                DecisionValidationError::AnswerTypeMismatch
            ))
        ));

        let missing_name = RESPONSE.replace("\"name\":\"team\",", "");
        assert!(matches!(
            Response::decode_json(missing_name.as_bytes(), &request),
            Err(DecisionCodecError::InvalidResponse(
                DecisionValidationError::AnswerNamesMismatch
            ))
        ));

        let extra_answer = RESPONSE.replace(
            "\"answers\":[",
            "\"answers\":[{\"type\":\"refusal\",\"name\":\"extra\"},",
        );
        assert!(matches!(
            Response::decode_json(extra_answer.as_bytes(), &request),
            Err(DecisionCodecError::InvalidResponse(
                DecisionValidationError::AnswerCountMismatch
            ))
        ));

        let wrong_choice = RESPONSE.replace("\"choice\":\"billing\"", "\"choice\":\"unknown\"");
        assert!(matches!(
            Response::decode_json(wrong_choice.as_bytes(), &request),
            Err(DecisionCodecError::InvalidResponse(
                DecisionValidationError::ChoiceNotRequested
            ))
        ));

        let missing_probability = RESPONSE.replace(r#",{"value":true,"probability":0.04}"#, "");
        assert!(matches!(
            Response::decode_json(missing_probability.as_bytes(), &request),
            Err(DecisionCodecError::InvalidResponse(
                DecisionValidationError::DistributionMismatch
            ))
        ));

        let out_of_range_score = RESPONSE.replace("\"score\":1.1", "\"score\":3.0");
        assert!(matches!(
            Response::decode_json(out_of_range_score.as_bytes(), &request),
            Err(DecisionCodecError::InvalidResponse(
                DecisionValidationError::ScoreOutOfRange
            ))
        ));

        let wrong_label = RESPONSE.replace("\"label\":\"soon\"", "\"label\":\"later\"");
        assert!(matches!(
            Response::decode_json(wrong_label.as_bytes(), &request),
            Err(DecisionCodecError::InvalidResponse(
                DecisionValidationError::ScoreLegendMismatch
            ))
        ));

        let duplicate_choice_probability = RESPONSE.replace(
            r#"{"value":true,"probability":0.04}"#,
            r#"{"value":"billing","probability":0.04}"#,
        );
        assert!(matches!(
            Response::decode_json(duplicate_choice_probability.as_bytes(), &request),
            Err(DecisionCodecError::InvalidResponse(
                DecisionValidationError::DistributionMismatch
            ))
        ));
    }

    #[test]
    fn openai_choice_values_remain_typed_and_duplicate_values_are_rejected() {
        let parsed: ChoiceValue = serde_json::from_str("\"true\"").unwrap();
        assert_eq!(parsed, ChoiceValue::String("true".into()));
        let parsed: ChoiceValue = serde_json::from_str("true").unwrap();
        assert_eq!(parsed, ChoiceValue::Bool(true));

        let request = Request::new(
            "model",
            Input::Text("ticket".into()),
            vec![Question::Choice {
                name: "kind".into(),
                instructions: "Choose a kind".into(),
                choices: vec![
                    ChoiceOption {
                        value: ChoiceValue::String("same".into()),
                        description: None,
                    },
                    ChoiceOption {
                        value: ChoiceValue::String("same".into()),
                        description: None,
                    },
                ],
            }],
        );
        assert_eq!(
            request.validate(),
            Err(DecisionValidationError::DuplicateChoice)
        );
    }

    #[test]
    fn openai_choice_and_score_count_limits_match_the_current_guide() {
        for count in [2, 255] {
            let choices = (0..count)
                .map(|index| ChoiceOption {
                    value: ChoiceValue::String(format!("option_{index}")),
                    description: None,
                })
                .collect();
            let request = Request::new(
                "model",
                Input::Text("state".into()),
                vec![Question::Choice {
                    name: "choice".into(),
                    instructions: "Choose one".into(),
                    choices,
                }],
            );
            assert!(request.validate().is_ok());
        }
        for count in [1, 256] {
            let choices = (0..count)
                .map(|index| ChoiceOption {
                    value: ChoiceValue::String(format!("option_{index}")),
                    description: None,
                })
                .collect();
            let request = Request::new(
                "model",
                Input::Text("state".into()),
                vec![Question::Choice {
                    name: "choice".into(),
                    instructions: "Choose one".into(),
                    choices,
                }],
            );
            assert_eq!(
                request.validate(),
                Err(DecisionValidationError::ChoiceCountOutOfRange)
            );
        }

        for count in [2, 10] {
            let levels = (0..count)
                .map(|index| ScoreLevel {
                    label: format!("level_{index}"),
                    description: None,
                })
                .collect();
            let request = Request::new(
                "model",
                Input::Text("state".into()),
                vec![Question::Score {
                    name: "score".into(),
                    instructions: "Score this".into(),
                    levels,
                }],
            );
            assert!(request.validate().is_ok());
        }
        for count in [1, 11] {
            let levels = (0..count)
                .map(|index| ScoreLevel {
                    label: format!("level_{index}"),
                    description: None,
                })
                .collect();
            let request = Request::new(
                "model",
                Input::Text("state".into()),
                vec![Question::Score {
                    name: "score".into(),
                    instructions: "Score this".into(),
                    levels,
                }],
            );
            assert_eq!(
                request.validate(),
                Err(DecisionValidationError::ScoreLevelCountOutOfRange)
            );
        }
    }

    #[test]
    fn openai_documented_count_limits_and_response_numeric_bounds_are_enforced() {
        let questions = (0..400)
            .map(|index| Question::Predicate {
                name: format!("question_{index}"),
                instructions: "Is the statement true?".into(),
            })
            .collect();
        assert!(
            Request::new("model", Input::Text("state".into()), questions)
                .validate()
                .is_ok()
        );

        let questions = (0..401)
            .map(|index| Question::Predicate {
                name: format!("question_{index}"),
                instructions: "Is the statement true?".into(),
            })
            .collect();
        assert_eq!(
            Request::new("model", Input::Text("state".into()), questions).validate(),
            Err(DecisionValidationError::TooManyQuestions)
        );

        let request = request();
        let invalid_probability = RESPONSE.replace("\"probability\":0.97", "\"probability\":1.01");
        assert!(matches!(
            Response::decode_json(invalid_probability.as_bytes(), &request),
            Err(DecisionCodecError::InvalidResponse(
                DecisionValidationError::InvalidWireShape
            ))
        ));
        let invalid_confidence = RESPONSE.replace("\"confidence\":0.55", "\"confidence\":1.01");
        assert!(matches!(
            Response::decode_json(invalid_confidence.as_bytes(), &request),
            Err(DecisionCodecError::InvalidResponse(
                DecisionValidationError::InvalidWireShape
            ))
        ));
    }

    #[test]
    fn openai_json_input_rejects_non_container_and_overdeep_values() {
        let scalar = Request::new(
            "model",
            Input::Json(Value::Bool(true)),
            vec![Question::Predicate {
                name: "q".into(),
                instructions: "Is it true?".into(),
            }],
        );
        assert_eq!(
            scalar.validate(),
            Err(DecisionValidationError::OpenAiJsonInputOutOfRange)
        );

        let deep = format!("{}0{}", "[".repeat(33), "]".repeat(33));
        let deep_value = serde_json::from_str(&deep).unwrap();
        let request = Request::new(
            "model",
            Input::Json(deep_value),
            vec![Question::Predicate {
                name: "q".into(),
                instructions: "Is it true?".into(),
            }],
        );
        assert_eq!(
            request.validate(),
            Err(DecisionValidationError::OpenAiJsonInputOutOfRange)
        );

        let too_many_values = Value::Array(vec![Value::Null; 10_000]);
        let request = Request::new(
            "model",
            Input::Json(too_many_values),
            vec![Question::Predicate {
                name: "q".into(),
                instructions: "Is it true?".into(),
            }],
        );
        assert_eq!(
            request.validate(),
            Err(DecisionValidationError::OpenAiJsonInputOutOfRange)
        );

        let too_large = json!({"data": "x".repeat(256 * 1024)});
        let request = Request::new(
            "model",
            Input::Json(too_large),
            vec![Question::Predicate {
                name: "q".into(),
                instructions: "Is it true?".into(),
            }],
        );
        assert_eq!(
            request.validate(),
            Err(DecisionValidationError::OpenAiJsonInputOutOfRange)
        );
    }
}
