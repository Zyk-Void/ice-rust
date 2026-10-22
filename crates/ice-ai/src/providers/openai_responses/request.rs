//! Private request DTOs shared by Responses-compatible providers.

use crate::strict_schema::strict_tool_parameters;
use crate::types::Tool;
use serde::ser::Serializer;
use serde::Serialize;
use serde_json::{Map, Value};

/// A request field whose strictness can be omitted, sent as `null`, or sent as
/// a boolean. The explicit null case preserves the original schema semantics
/// for protocols whose default is `strict: null`.
#[derive(Debug, Clone, Copy)]
pub(crate) enum FunctionToolStrictness {
    Boolean(bool),
    Null,
}

impl Serialize for FunctionToolStrictness {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Boolean(value) => serializer.serialize_bool(*value),
            Self::Null => serializer.serialize_none(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub(crate) enum ResponsesTool {
    #[serde(rename = "function")]
    Function {
        name: String,
        description: String,
        parameters: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        strict: Option<FunctionToolStrictness>,
    },
    #[serde(rename = "custom")]
    Custom {
        name: String,
        description: String,
        format: ResponsesToolFormat,
    },
}

impl ResponsesTool {
    /// Prepare the function schema before constructing its wire representation.
    /// If a supported constrained strict schema cannot be transformed, retain
    /// the original schema and omit `strict`, matching the OpenAI Responses
    /// contract.
    pub(crate) fn function(
        tool: &Tool,
        supports_strict_mode: bool,
        default_strict: Option<FunctionToolStrictness>,
    ) -> Self {
        let strict_parameters = supports_strict_mode
            .then(|| prepare_strict_tool_parameters(tool))
            .flatten();
        let strict = if strict_parameters.is_some() {
            Some(FunctionToolStrictness::Boolean(true))
        } else if supports_strict_mode {
            default_strict
        } else {
            None
        };
        let parameters = strict_parameters.unwrap_or_else(|| tool.parameters.as_value().clone());

        Self::Function {
            name: tool.name.clone(),
            description: tool.description.clone(),
            parameters,
            strict,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ResponsesToolFormat {
    #[serde(rename = "type")]
    kind: &'static str,
    syntax: String,
    definition: String,
}

impl ResponsesToolFormat {
    pub(crate) fn grammar(syntax: String, definition: String) -> Self {
        Self {
            kind: "grammar",
            syntax,
            definition,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ResponsesTextOptions {
    pub(crate) verbosity: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ResponsesReasoning {
    pub(crate) effort: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) summary: Option<String>,
}

/// Protocol-owned fields in a Responses request. Transcript items and tool
/// schemas stay dynamic because their shapes are defined by the API and the
/// caller's schemas, while the surrounding request shape is explicit.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct ResponsesRequest {
    pub(crate) model: String,
    pub(crate) input: Vec<Value>,
    pub(crate) stream: bool,
    pub(crate) store: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) instructions: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) max_output_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) reasoning: Option<ResponsesReasoning>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tools: Option<Vec<ResponsesTool>>,
    /// Session-scoped key that routes requests to the same cached prefix.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) prompt_cache_key: Option<String>,
    /// `"24h"` extends the cache lifetime for `CacheRetention::Long`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) prompt_cache_retention: Option<&'static str>,
    /// Extra output the API must return. Reasoning requests ask for
    /// `reasoning.encrypted_content`, which is what lets a `store:false`
    /// transcript replay earlier reasoning items exactly.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) include: Option<Vec<String>>,
}

/// OpenAI rejects prompt cache keys longer than this many characters.
pub(crate) const MAX_CACHE_KEY_CHARS: usize = 64;

pub(crate) fn clamp_cache_key(key: &str) -> String {
    key.chars().take(MAX_CACHE_KEY_CHARS).collect()
}

/// Codex-specific fields layered over the shared Responses request fields.
/// Keeping these fields in a separate DTO lets the Codex request be assembled
/// directly without patching an ordinary Responses JSON value.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct CodexResponsesRequest {
    #[serde(flatten)]
    pub(crate) responses: ResponsesRequest,
    pub(crate) text: ResponsesTextOptions,
    pub(crate) include: Vec<String>,
    pub(crate) tool_choice: String,
    pub(crate) parallel_tool_calls: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) prompt_cache_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) temperature: Option<f64>,
}

impl ResponsesRequest {
    /// Serialize the typed request, then apply custom sampling parameters last.
    /// This preserves the provider's established override precedence,
    /// including custom parameters that replace named request fields.
    pub(crate) fn into_value_with_sampling_params(
        self,
        sampling_params: Option<&Map<String, Value>>,
    ) -> Value {
        serialize_with_sampling_params(self, sampling_params)
            .expect("Responses request DTO fields are JSON serializable")
    }
}

pub(crate) fn serialize_with_sampling_params<T: Serialize>(
    request: T,
    sampling_params: Option<&Map<String, Value>>,
) -> Result<Value, String> {
    let mut request = serde_json::to_value(request)
        .map_err(|error| format!("Responses request serialization failed: {error}"))?;
    if let (Some(parameters), Some(request)) = (sampling_params, request.as_object_mut()) {
        request.extend(parameters.clone());
    }
    Ok(request)
}

/// Return a strict-compatible schema when the tool explicitly requests JSON
/// Schema constrained sampling and the schema can be represented by the strict
/// subset. Kept separate from tool serialization so other Responses-compatible
/// protocols can retain the untransformed optional-property semantics.
pub(crate) fn prepare_strict_tool_parameters(tool: &Tool) -> Option<Value> {
    strict_tool_parameters(tool)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Schema;
    use serde_json::json;

    #[test]
    fn explicit_null_strictness_keeps_the_original_optional_schema() {
        let tool = Tool {
            name: "read".into(),
            description: "Read a file".into(),
            parameters: Schema::new(json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "offset": {"type": ["integer", "null"]}
                },
                "required": ["path"]
            })),
            constrained_sampling: None,
        };
        let original_schema = tool.parameters.as_value().clone();

        let request_tool = ResponsesTool::function(&tool, true, Some(FunctionToolStrictness::Null));
        let serialized = serde_json::to_value(request_tool).unwrap();

        assert_eq!(serialized["strict"], Value::Null);
        assert_eq!(serialized["parameters"], original_schema);
        assert_eq!(serialized["parameters"]["required"], json!(["path"]));
        assert!(serialized["parameters"]
            .get("additionalProperties")
            .is_none());
    }
}
