//! Typed, one-shot decision API contracts, separate from chat provider streams.
//!
//! This module contains offline request encoders and request-aware response
//! decoders for OpenAI Decisions, TypeSafe System One, and Inception Decisions.
//! With the `providers` feature, [`client`] adds a one-shot HTTP transport bound
//! to an explicit endpoint and key. Provider selection and credential storage
//! belong to the host (see `icers`' `decisionProviders` in `models.json`).

#[cfg(feature = "providers")]
pub mod client;
pub mod inception;
pub mod openai;
pub mod typesafe;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io::{self, Write};

use serde::de::{self, DeserializeOwned, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Number, Value};
use thiserror::Error;

/// Local ceiling for response JSON accepted by the offline decoders.
///
/// This is a library safety bound, not a claim about a provider's response
/// limit. Provider request body limits are enforced separately by each codec.
pub const MAX_DECISION_RESPONSE_JSON_BYTES: usize = 32 * 1024 * 1024;

/// Local maximum container nesting accepted by the decision codecs.
pub const MAX_DECISION_JSON_DEPTH: usize = 64;

/// TypeSafe and Inception score legends echo criteria inside the response
/// envelope. Their request field limit counts the criteria array/map itself;
/// the response adds the root, answers map, answer object, and legend map.
pub(crate) const MAX_DECISION_RESPONSE_JSON_DEPTH: usize = MAX_DECISION_JSON_DEPTH + 3;

/// Local consistency tolerance, allowing small floating-point/serialization
/// differences without accepting materially contradictory decision results.
/// This is a validation policy, not a provider precision guarantee.
const DISTRIBUTION_ARITHMETIC_TOLERANCE: f64 = 1e-6;

/// Errors from protocol-specific request and response validation.
///
/// Variants intentionally carry no question names or user-provided content so
/// they can be displayed or logged without exposing decision input or output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum DecisionValidationError {
    #[error("model ID must not be empty")]
    EmptyModelId,
    #[error("at least one question is required")]
    EmptyQuestions,
    #[error("the question count exceeds the protocol limit")]
    TooManyQuestions,
    #[error("question names must not be empty")]
    EmptyQuestionName,
    #[error("question names must be unique")]
    DuplicateQuestionName,
    #[error("question criteria must not be empty")]
    EmptyCriteria,
    #[error("choice count is outside the protocol limit")]
    ChoiceCountOutOfRange,
    #[error("choice names or values must be unique")]
    DuplicateChoice,
    #[error("score level count is outside the protocol limit")]
    ScoreLevelCountOutOfRange,
    #[error("score level labels must be non-empty and unique")]
    InvalidScoreLevelLabels,
    #[error("score criteria must be distinct")]
    DuplicateScoreCriteria,
    #[error("native JSON content exceeds the supported nesting limit")]
    JsonDepthOutOfRange,
    #[error("OpenAI JSON input is outside the documented limits")]
    OpenAiJsonInputOutOfRange,
    #[error("the request body exceeds the protocol limit")]
    RequestBodyTooLarge,
    #[error("this protocol input contains an unsupported image part")]
    UnsupportedImagePart,
    #[error("response model ID must not be empty")]
    EmptyResponseModelId,
    #[error("response answer count does not match the request")]
    AnswerCountMismatch,
    #[error("response question names do not match the request")]
    AnswerNamesMismatch,
    #[error("response answer type does not match the request")]
    AnswerTypeMismatch,
    #[error("response choice is not one of the requested choices")]
    ChoiceNotRequested,
    #[error("response distribution does not match the requested options")]
    DistributionMismatch,
    #[error("response probabilities do not form a normalized distribution")]
    ProbabilityDistributionMismatch,
    #[error("response choice is not among the highest-probability options")]
    ChoiceProbabilityMismatch,
    #[error("response score does not match its probability distribution")]
    ScoreDistributionMismatch,
    #[error("response score is outside the requested level range")]
    ScoreOutOfRange,
    #[error("response score labels do not match the requested levels")]
    ScoreLegendMismatch,
    #[error("response has an invalid wire shape")]
    InvalidWireShape,
    #[error("probability must be finite and between 0 and 1")]
    InvalidProbability,
    #[error("confidence must be finite and between 0 and 1")]
    InvalidConfidence,
}

/// Safe errors returned by request encoders and response decoders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum DecisionCodecError {
    #[error("invalid decision request: {0}")]
    InvalidRequest(DecisionValidationError),
    #[error("invalid decision response: {0}")]
    InvalidResponse(DecisionValidationError),
    #[error("decision JSON is malformed or has trailing content")]
    MalformedJson,
    #[error("decision JSON contains a duplicate object key")]
    DuplicateObjectKey,
    #[error("decision JSON exceeds the local 32 MiB response limit")]
    ResponseBodyTooLarge,
    #[error("decision JSON exceeds the local nesting limit")]
    JsonDepthOutOfRange,
    #[error("decision JSON encoding failed")]
    SerializationFailed,
}

/// A probability constrained to a finite value in the inclusive range 0–1.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Probability(f64);

impl Probability {
    pub fn new(value: f64) -> Result<Self, DecisionValidationError> {
        if value.is_finite() && (0.0..=1.0).contains(&value) {
            Ok(Self(value))
        } else {
            Err(DecisionValidationError::InvalidProbability)
        }
    }

    pub const fn get(self) -> f64 {
        self.0
    }
}

impl Serialize for Probability {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_f64(self.0)
    }
}

impl<'de> Deserialize<'de> for Probability {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = f64::deserialize(deserializer)?;
        Self::new(value).map_err(de::Error::custom)
    }
}

/// Provider-reported confidence, kept distinct from probability and
/// categorical distributions. No formula is implied by this type.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Confidence(f64);

impl Confidence {
    pub fn new(value: f64) -> Result<Self, DecisionValidationError> {
        if value.is_finite() && (0.0..=1.0).contains(&value) {
            Ok(Self(value))
        } else {
            Err(DecisionValidationError::InvalidConfidence)
        }
    }

    pub const fn get(self) -> f64 {
        self.0
    }
}

impl Serialize for Confidence {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_f64(self.0)
    }
}

impl<'de> Deserialize<'de> for Confidence {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = f64::deserialize(deserializer)?;
        Self::new(value).map_err(de::Error::custom)
    }
}

/// Native TypeSafe/Inception JSON content: text, an object, or an array.
/// Nested values retain ordinary JSON types.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum NativeContent {
    Text(String),
    Object(Map<String, Value>),
    Array(Vec<Value>),
}

impl NativeContent {
    pub fn text(value: impl Into<String>) -> Self {
        Self::Text(value.into())
    }

    pub(crate) fn max_container_depth(&self) -> usize {
        match self {
            Self::Text(_) => 0,
            Self::Object(values) => {
                1 + values
                    .values()
                    .map(value_max_container_depth)
                    .max()
                    .unwrap_or(0)
            }
            Self::Array(values) => {
                1 + values
                    .iter()
                    .map(value_max_container_depth)
                    .max()
                    .unwrap_or(0)
            }
        }
    }

    pub(crate) fn contains_inception_image_part(&self) -> bool {
        let mut stack = Vec::new();
        match self {
            Self::Text(_) => return false,
            Self::Object(values) => {
                if is_inception_image_part(values) {
                    return true;
                }
                stack.extend(values.values());
            }
            Self::Array(values) => stack.extend(values.iter()),
        }

        while let Some(value) = stack.pop() {
            match value {
                Value::Object(object) => {
                    if is_inception_image_part(object) {
                        return true;
                    }
                    stack.extend(object.values());
                }
                Value::Array(values) => stack.extend(values),
                _ => {}
            }
        }
        false
    }
}

/// Return true when text contains at least one non-whitespace character.
pub(crate) fn nonblank(value: &str) -> bool {
    !value.trim().is_empty()
}

/// Serialize JSON incrementally, stopping at `max_bytes` instead of first
/// allocating an unbounded `Vec` for an oversized request.
pub(crate) fn encode_bounded_json<T: Serialize>(
    value: &T,
    max_bytes: usize,
) -> Result<Vec<u8>, DecisionCodecError> {
    let mut writer = BoundedVecWriter {
        bytes: Vec::new(),
        limit: max_bytes,
    };
    match serde_json::to_writer(&mut writer, value) {
        Ok(()) => Ok(writer.bytes),
        Err(error) if error.io_error_kind() == Some(io::ErrorKind::WriteZero) => Err(
            DecisionCodecError::InvalidRequest(DecisionValidationError::RequestBodyTooLarge),
        ),
        Err(_) => Err(DecisionCodecError::SerializationFailed),
    }
}

struct BoundedVecWriter {
    bytes: Vec<u8>,
    limit: usize,
}

impl Write for BoundedVecWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        let remaining = self.limit.saturating_sub(self.bytes.len());
        if remaining == 0 {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "decision request JSON byte limit reached",
            ));
        }
        let written = remaining.min(buffer.len());
        self.bytes.extend_from_slice(&buffer[..written]);
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) fn probability_distribution_sums_to_one(
    probabilities: impl IntoIterator<Item = Probability>,
) -> bool {
    let sum = probabilities.into_iter().map(Probability::get).sum::<f64>();
    (sum - 1.0).abs() <= DISTRIBUTION_ARITHMETIC_TOLERANCE
}

pub(crate) fn selected_choice_has_max_probability(
    selected: Probability,
    probabilities: impl IntoIterator<Item = Probability>,
) -> bool {
    probabilities
        .into_iter()
        .all(|probability| probability.get() <= selected.get() + DISTRIBUTION_ARITHMETIC_TOLERANCE)
}

pub(crate) fn score_matches_expected_value(
    score: f64,
    probabilities: impl IntoIterator<Item = (usize, Probability)>,
) -> bool {
    let expected = probabilities
        .into_iter()
        .map(|(index, probability)| index as f64 * probability.get())
        .sum::<f64>();
    (score - expected).abs() <= DISTRIBUTION_ARITHMETIC_TOLERANCE * expected.abs().max(1.0)
}

pub(crate) fn same_string_keys<T, U>(
    left: &BTreeMap<String, T>,
    right: &BTreeMap<String, U>,
) -> bool {
    left.keys().eq(right.keys())
}

pub(crate) fn score_index_keys(count: usize) -> BTreeSet<String> {
    (0..count).map(|index| index.to_string()).collect()
}

/// Count every JSON value, including the root, without recursive traversal.
pub(crate) fn value_count(value: &Value) -> usize {
    let mut count = 0;
    let mut stack = vec![value];
    while let Some(value) = stack.pop() {
        count += 1;
        match value {
            Value::Array(values) => stack.extend(values),
            Value::Object(values) => stack.extend(values.values()),
            _ => {}
        }
    }
    count
}

/// Count nested object/array levels, with a top-level object or array at 1.
pub(crate) fn value_max_container_depth(value: &Value) -> usize {
    let mut max_depth = 0;
    let mut stack = vec![(value, 0usize)];
    while let Some((value, parent_depth)) = stack.pop() {
        match value {
            Value::Array(values) => {
                let depth = parent_depth + 1;
                max_depth = max_depth.max(depth);
                stack.extend(values.iter().map(|child| (child, depth)));
            }
            Value::Object(values) => {
                let depth = parent_depth + 1;
                max_depth = max_depth.max(depth);
                stack.extend(values.values().map(|child| (child, depth)));
            }
            _ => {}
        }
    }
    max_depth
}

fn is_inception_image_part(object: &Map<String, Value>) -> bool {
    object.get("type") == Some(&Value::String("image_url".into()))
        && object.contains_key("image_url")
}

/// Parse a bounded JSON response while rejecting duplicate object keys.
///
/// `serde_json` enforces its built-in recursion ceiling. We then apply the
/// stricter local depth bound before converting the value into a protocol DTO.
pub(crate) fn parse_unique_json(bytes: &[u8]) -> Result<Value, DecisionCodecError> {
    parse_unique_json_with_depth(bytes, MAX_DECISION_JSON_DEPTH)
}

pub(crate) fn parse_unique_json_with_depth(
    bytes: &[u8],
    max_depth: usize,
) -> Result<Value, DecisionCodecError> {
    if bytes.len() > MAX_DECISION_RESPONSE_JSON_BYTES {
        return Err(DecisionCodecError::ResponseBodyTooLarge);
    }

    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let value = UniqueJsonValue::deserialize(&mut deserializer).map_err(|error| {
        if error
            .to_string()
            .contains("duplicate decision JSON object key")
        {
            DecisionCodecError::DuplicateObjectKey
        } else {
            DecisionCodecError::MalformedJson
        }
    })?;
    deserializer
        .end()
        .map_err(|_| DecisionCodecError::MalformedJson)?;

    if value_max_container_depth(&value.0) > max_depth {
        return Err(DecisionCodecError::JsonDepthOutOfRange);
    }
    Ok(value.0)
}

pub(crate) fn decode_response_value<T: DeserializeOwned>(
    value: Value,
) -> Result<T, DecisionCodecError> {
    serde_json::from_value(value)
        .map_err(|_| DecisionCodecError::InvalidResponse(DecisionValidationError::InvalidWireShape))
}

#[derive(Debug)]
struct UniqueJsonValue(Value);

impl<'de> Deserialize<'de> for UniqueJsonValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(UniqueJsonVisitor)
    }
}

struct UniqueJsonVisitor;

impl<'de> Visitor<'de> for UniqueJsonVisitor {
    type Value = UniqueJsonValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value without duplicate object keys")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(UniqueJsonValue(Value::Bool(value)))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(UniqueJsonValue(Value::Number(Number::from(value))))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(UniqueJsonValue(Value::Number(Number::from(value))))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Number::from_f64(value)
            .map(Value::Number)
            .map(UniqueJsonValue)
            .ok_or_else(|| E::custom("non-finite JSON number"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(UniqueJsonValue(Value::String(value.to_owned())))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(UniqueJsonValue(Value::String(value)))
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(UniqueJsonValue(Value::Null))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.visit_unit()
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        UniqueJsonValue::deserialize(deserializer)
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element::<UniqueJsonValue>()? {
            values.push(value.0);
        }
        Ok(UniqueJsonValue(Value::Array(values)))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut values = Map::new();
        while let Some((key, value)) = map.next_entry::<String, UniqueJsonValue>()? {
            if values.contains_key(&key) {
                return Err(de::Error::custom("duplicate decision JSON object key"));
            }
            values.insert(key, value.0);
        }
        Ok(UniqueJsonValue(Value::Object(values)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probability_and_confidence_are_finite_unit_interval_types() {
        assert!(Probability::new(0.0).is_ok());
        assert!(Probability::new(1.0).is_ok());
        assert_eq!(
            Probability::new(f64::NAN),
            Err(DecisionValidationError::InvalidProbability)
        );
        assert!(serde_json::from_str::<Probability>("1.01").is_err());
        assert!(serde_json::from_str::<Confidence>("-0.01").is_err());
        assert_eq!(Confidence::new(0.5).unwrap().get(), 0.5);
    }

    #[test]
    fn strict_json_parser_rejects_duplicate_keys_trailing_data_and_deep_values() {
        assert_eq!(
            parse_unique_json(br#"{"answers":{"q":1,"q":2}}"#),
            Err(DecisionCodecError::DuplicateObjectKey)
        );
        assert_eq!(
            parse_unique_json(b"{} trailing"),
            Err(DecisionCodecError::MalformedJson)
        );
        let deep = format!("{}0{}", "[".repeat(65), "]".repeat(65));
        assert_eq!(
            parse_unique_json(deep.as_bytes()),
            Err(DecisionCodecError::JsonDepthOutOfRange)
        );
    }

    #[test]
    fn bounded_encoding_stops_before_serializing_later_elements() {
        use serde::ser::SerializeSeq;
        struct TooLarge;
        impl Serialize for TooLarge {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                let mut seq = serializer.serialize_seq(Some(2))?;
                seq.serialize_element(&"a".repeat(100))?;
                panic!("serialization must stop at the byte limit");
            }
        }
        assert_eq!(
            encode_bounded_json(&TooLarge, 10),
            Err(DecisionCodecError::InvalidRequest(
                DecisionValidationError::RequestBodyTooLarge
            ))
        );
        assert_eq!(encode_bounded_json(&"abc", 5).unwrap(), b"\"abc\"");
        assert!(encode_bounded_json(&"abc", 4).is_err());
    }

    #[test]
    fn numerical_consistency_allows_small_rounding_and_ties_not_contradictions() {
        let p = |v| Probability::new(v).unwrap();
        assert!(probability_distribution_sums_to_one([
            p(0.3333333),
            p(0.3333333),
            p(0.3333333)
        ]));
        assert!(!probability_distribution_sums_to_one([p(0.0), p(0.0)]));
        assert!(!probability_distribution_sums_to_one([p(1.0), p(1.0)]));
        assert!(selected_choice_has_max_probability(
            p(0.5),
            [p(0.5), p(0.5)]
        ));
        assert!(!selected_choice_has_max_probability(p(0.0), [p(1.0)]));
        assert!(score_matches_expected_value(
            1.1,
            [(0, p(0.1)), (1, p(0.7)), (2, p(0.2))]
        ));
        assert!(!score_matches_expected_value(0.0, [(1, p(1.0))]));
    }

    #[test]
    fn strict_json_parser_rejects_oversized_bodies_without_echoing_content() {
        let oversized = vec![b' '; MAX_DECISION_RESPONSE_JSON_BYTES + 1];
        assert_eq!(
            parse_unique_json(&oversized),
            Err(DecisionCodecError::ResponseBodyTooLarge)
        );
        let error = DecisionCodecError::DuplicateObjectKey.to_string();
        assert!(!error.contains("q"));
        assert!(!error.contains("secret"));
    }
}
