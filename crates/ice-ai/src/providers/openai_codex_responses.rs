//! ChatGPT subscription Responses API provider.
//!
//! Mirrors the SSE transport in `packages/ai/src/api/openai-codex-responses.ts`.
//! Authentication is supplied as a short-lived bearer token by the caller;
//! credential refresh and persistence belong to `ice-cli`.

use crate::event_stream::{
    create_assistant_message_event_stream, AssistantMessageEventStream,
    AssistantMessageEventStreamProducer,
};
use crate::model::{Model, StreamingProtocolCompat};
use crate::provider::{
    enforce_hard_max_tokens_in_record, request_max_tokens, CacheRetention, Provider,
    SimpleStreamOptions,
};
use crate::providers::openai_responses::request::{
    clamp_cache_key, CodexResponsesRequest, FunctionToolStrictness, ResponsesReasoning,
    ResponsesRequest, ResponsesTextOptions,
};
use crate::providers::openai_responses::{
    build_responses_input, build_responses_tools, process_responses_sse,
};
use crate::types::{AssistantMessage, Context, ThinkingLevel};
use async_trait::async_trait;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

const DEFAULT_CODEX_BASE_URL: &str = "https://chatgpt.com/backend-api";
const ACCOUNT_CLAIM: &str = "https://api.openai.com/auth";
const DEFAULT_INSTRUCTIONS: &str = "You are a helpful assistant.";

pub struct OpenAiCodexResponsesProvider {
    id: String,
    api_key: Option<String>,
    http: reqwest::Client,
    models: Vec<Model>,
}

impl OpenAiCodexResponsesProvider {
    /// Construct a Codex Responses provider with an optional access token.
    /// The token may instead be provided per call in `SimpleStreamOptions`.
    pub fn with_models(
        id: impl Into<String>,
        api_key: Option<String>,
        http: reqwest::Client,
        models: Vec<Model>,
    ) -> Self {
        Self {
            id: id.into(),
            api_key,
            http,
            models,
        }
    }

    /// Kept parallel with the OpenAI Responses constructor for provider
    /// factories that explicitly avoid process-wide credential inheritance.
    pub fn with_models_without_env_api_key(
        id: impl Into<String>,
        api_key: Option<String>,
        http: reqwest::Client,
        models: Vec<Model>,
    ) -> Self {
        Self::with_models(id, api_key, http, models)
    }
}

#[async_trait]
impl Provider for OpenAiCodexResponsesProvider {
    fn id(&self) -> &str {
        &self.id
    }

    fn models(&self) -> &[Model] {
        &self.models
    }

    async fn stream_simple(
        &self,
        model: &Model,
        ctx: &Context,
        opts: &SimpleStreamOptions,
    ) -> AssistantMessageEventStream {
        let (mut producer, stream) = create_assistant_message_event_stream();
        let http = self.http.clone();
        let provider_key = self.api_key.clone();
        let model = model.clone();
        let ctx = Arc::new(ctx.clone());
        let opts = opts.clone();
        tokio::spawn(async move {
            run_stream(&mut producer, http, provider_key, &model, &ctx, &opts).await;
        });
        stream
    }
}

async fn run_stream(
    producer: &mut AssistantMessageEventStreamProducer,
    http: reqwest::Client,
    provider_key: Option<String>,
    model: &Model,
    ctx: &Context,
    opts: &SimpleStreamOptions,
) {
    let mut output = AssistantMessage::empty(
        model.api.clone(),
        model.provider.clone(),
        model.id.clone(),
        now_ms(),
    );
    let token = opts
        .api_key
        .clone()
        .or(provider_key)
        .filter(|token| !token.is_empty());
    let Some(token) = token else {
        emit_error(
            producer,
            &mut output,
            format!("No API key for provider: {}", model.provider),
            false,
        );
        return;
    };
    let account_id = match extract_account_id(&token) {
        Ok(account_id) => account_id,
        Err(message) => {
            emit_error(producer, &mut output, message, false);
            return;
        }
    };
    let body = match build_codex_request(model, ctx, opts) {
        Ok(body) => body,
        Err(message) => {
            emit_error(producer, &mut output, message, false);
            return;
        }
    };
    let cache_session_id = codex_session_id(opts);
    let headers = build_codex_headers(
        model,
        opts,
        &token,
        &account_id,
        cache_session_id.as_deref(),
    );
    let url = codex_responses_url(&model.base_url);
    let timeout = opts.request_timeout();
    // `.json()` would add its own `Content-Type`, and `.header()` appends, so the
    // backend would see two and reject the request as an unsupported content
    // type. The header map above is the single source of `Content-Type`.
    let mut request = http.post(url).body(body.to_string()).timeout(timeout);
    for (name, value) in headers {
        request = request.header(name, value);
    }

    // Codex's native provider defaults to zero retries. Keep the HTTP boundary
    // as one cancellable request so callers cannot accidentally retry a
    // subscription request by setting generic provider retry options.
    let response = tokio::select! {
        response = request.send() => match response {
            Ok(response) => response,
            Err(error) => {
                emit_error(producer, &mut output, format!("http error: {error}"), false);
                return;
            }
        },
        _ = opts.signal.cancelled() => {
            emit_error(producer, &mut output, "Request aborted", true);
            return;
        }
    };

    if !response.status().is_success() {
        let status = response.status().as_u16();
        let response_body = tokio::select! {
            body = response.text() => body.unwrap_or_default(),
            _ = opts.signal.cancelled() => {
                emit_error(producer, &mut output, "Request aborted", true);
                return;
            }
        };
        let message = format_codex_http_error(status, &response_body);
        emit_error(producer, &mut output, message, false);
        return;
    }

    process_responses_sse(
        producer,
        output,
        response,
        model,
        ctx,
        opts,
        "OpenAI Codex Responses",
    )
    .await;
}

fn build_codex_request(
    model: &Model,
    ctx: &Context,
    opts: &SimpleStreamOptions,
) -> Result<Value, String> {
    if opts
        .temperature
        .is_some_and(|temperature| !temperature.is_finite())
    {
        return Err("temperature must be a finite number".into());
    }

    let max_output_tokens = if opts.max_tokens.is_some() || opts.hard_max_output_tokens.is_some() {
        Some(request_max_tokens(model, ctx, opts)?)
    } else {
        None
    };
    if max_output_tokens == Some(0) {
        return Err("maxTokens must be a positive integer".into());
    }

    let prompt_cache_key = (opts.cache_retention != CacheRetention::None)
        .then(|| opts.session_id.as_deref())
        .flatten()
        .map(clamp_cache_key);
    let reasoning = if model.reasoning {
        opts.reasoning
            .and_then(|level| codex_reasoning_effort(model, level))
            .map(|effort| ResponsesReasoning {
                effort,
                summary: Some("auto".into()),
            })
    } else {
        None
    };
    let tools = build_responses_tools(
        model,
        &ctx.tools,
        codex_supports_strict_mode(model),
        Some(FunctionToolStrictness::Null),
    );
    let input = build_codex_responses_input(model, ctx);
    let instructions = ctx
        .system_prompt
        .as_deref()
        .filter(|prompt| !prompt.is_empty())
        .unwrap_or(DEFAULT_INSTRUCTIONS)
        .to_string();

    let request = CodexResponsesRequest {
        responses: ResponsesRequest {
            model: model.id.clone(),
            input,
            stream: true,
            store: false,
            instructions: Some(instructions),
            max_output_tokens,
            reasoning,
            tools,
            // Codex carries its own `prompt_cache_key` below.
            prompt_cache_key: None,
            prompt_cache_retention: None,
            include: None,
        },
        text: ResponsesTextOptions {
            verbosity: "low".into(),
        },
        include: vec!["reasoning.encrypted_content".into()],
        tool_choice: "auto".into(),
        parallel_tool_calls: true,
        prompt_cache_key,
        temperature: opts.temperature,
    };
    // The Codex reference builds from its direct options and intentionally
    // does not merge Model.samplingParams. Ordinary Responses retains its
    // established sampling-parameter overlay in the shared builder.
    let mut body = serde_json::to_value(request)
        .map_err(|error| format!("Codex Responses request serialization failed: {error}"))?;
    let Some(payload) = body.as_object_mut() else {
        return Err("Codex Responses request payload must be an object".into());
    };
    if let Some(hard_max_output_tokens) = opts.hard_max_output_tokens {
        enforce_hard_max_tokens_in_record(
            payload,
            "max_output_tokens",
            Some(hard_max_output_tokens),
        )?;
    }
    validate_codex_request(payload)?;
    Ok(body)
}

/// Codex expects every user message's text content as typed input parts. The
/// shared Responses builder keeps plain text as a string, which regular
/// Responses accepts, but the Codex endpoint rejects with "Unsupported content
/// type". Block content is already converted to typed `input_text`/`input_image`
/// parts by the shared builder.
fn build_codex_responses_input(model: &Model, ctx: &Context) -> Vec<Value> {
    let mut input = build_responses_input(model, ctx);
    for item in &mut input {
        if item.get("role").and_then(Value::as_str) != Some("user") {
            continue;
        }
        let Some(text) = item
            .get("content")
            .and_then(Value::as_str)
            .map(str::to_string)
        else {
            continue;
        };
        item["content"] = serde_json::json!([{"type": "input_text", "text": text}]);
    }
    input
}

fn validate_codex_request(payload: &mut Map<String, Value>) -> Result<(), String> {
    if payload.get("model").and_then(Value::as_str).is_none() {
        return Err("model sampling parameter must be a string".into());
    }
    if payload.get("stream") != Some(&Value::Bool(true)) {
        return Err("Codex Responses requires stream:true".into());
    }
    if payload.get("store") != Some(&Value::Bool(false)) {
        return Err("Codex Responses requires store:false".into());
    }
    if !payload.get("input").is_some_and(Value::is_array) {
        return Err("input sampling parameter must be an array".into());
    }
    if payload.get("max_output_tokens").is_some_and(|value| {
        !value.is_number() || value.as_u64().map_or(true, |tokens| tokens == 0)
    }) {
        return Err("max_output_tokens must be a positive integer".into());
    }
    if payload
        .get("temperature")
        .is_some_and(|value| !value.as_f64().is_some_and(f64::is_finite))
    {
        return Err("temperature sampling parameter must be a finite number".into());
    }
    if payload
        .get("prompt_cache_key")
        .is_some_and(|value| !value.is_string())
    {
        return Err("prompt_cache_key must be a string".into());
    }
    Ok(())
}

fn codex_supports_strict_mode(model: &Model) -> bool {
    match model.compat.as_ref() {
        Some(StreamingProtocolCompat::OpenaiResponses(compat)) => {
            compat.supports_strict_mode.unwrap_or(true)
        }
        _ => true,
    }
}

fn codex_reasoning_effort(model: &Model, level: ThinkingLevel) -> Option<String> {
    let mapped = model
        .thinking_level_map
        .as_ref()
        .and_then(|map| map.get(&level));
    if level == ThinkingLevel::Off {
        // The TS implementation uses `off ?? "none"`, so an absent or null
        // mapping still explicitly disables reasoning for this request.
        return Some(
            mapped
                .and_then(Clone::clone)
                .unwrap_or_else(|| "none".into()),
        );
    }
    match mapped {
        Some(Some(effort)) => Some(effort.clone()),
        Some(None) => None,
        None => Some(
            match level {
                ThinkingLevel::Off => "none",
                ThinkingLevel::Minimal => "minimal",
                ThinkingLevel::Low => "low",
                ThinkingLevel::Medium => "medium",
                ThinkingLevel::High => "high",
                ThinkingLevel::Xhigh => "xhigh",
                ThinkingLevel::Max => "max",
            }
            .into(),
        ),
    }
}

fn codex_session_id(opts: &SimpleStreamOptions) -> Option<String> {
    (opts.cache_retention != CacheRetention::None)
        .then(|| opts.session_id.as_deref())
        .flatten()
        .map(clamp_cache_key)
}

fn build_codex_headers(
    model: &Model,
    opts: &SimpleStreamOptions,
    token: &str,
    account_id: &str,
    session_id: Option<&str>,
) -> BTreeMap<String, String> {
    let mut headers = model.headers.clone().unwrap_or_default();
    if let Some(additional) = &opts.headers {
        for (name, value) in additional {
            set_header(&mut headers, name, value);
        }
    }
    headers.retain(|name, _| !name.eq_ignore_ascii_case("x-api-key"));
    set_header(&mut headers, "authorization", &format!("Bearer {token}"));
    set_header(&mut headers, "chatgpt-account-id", account_id);
    set_header(&mut headers, "originator", "ice");
    set_header(&mut headers, "user-agent", &codex_user_agent());
    set_header(&mut headers, "openai-beta", "responses=experimental");
    set_header(&mut headers, "accept", "text/event-stream");
    set_header(&mut headers, "content-type", "application/json");
    if let Some(session_id) = session_id {
        set_header(&mut headers, "session-id", session_id);
        set_header(&mut headers, "x-client-request-id", session_id);
    }
    headers
}

fn set_header(headers: &mut BTreeMap<String, String>, name: &str, value: &str) {
    headers.retain(|existing, _| !existing.eq_ignore_ascii_case(name));
    headers.insert(name.into(), value.into());
}

fn codex_user_agent() -> String {
    let platform = match std::env::consts::OS {
        "windows" => "win32",
        "macos" => "darwin",
        other => other,
    };
    let architecture = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "x86" => "ia32",
        "aarch64" => "arm64",
        other => other,
    };
    format!("ice ({platform}; {architecture})")
}

/// Stored subscription credentials may only be used with the official backend.
/// Explicitly supplied tokens can still be used with caller-controlled transports.
pub fn validate_oauth_base_url(base_url: &str) -> Result<(), &'static str> {
    let url = reqwest::Url::parse(&codex_responses_url(base_url))
        .map_err(|_| "Invalid Codex OAuth endpoint")?;
    if url.scheme() != "https"
        || url.host_str() != Some("chatgpt.com")
        || url.port_or_known_default() != Some(443)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/backend-api/codex/responses"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("Stored Codex OAuth credentials require https://chatgpt.com/backend-api; custom endpoints require an explicit token");
    }
    Ok(())
}

fn codex_responses_url(base_url: &str) -> String {
    let trimmed = base_url.trim();
    let base = if trimmed.is_empty() {
        DEFAULT_CODEX_BASE_URL
    } else {
        trimmed
    };
    let normalized = base.trim_end_matches('/');
    if normalized.ends_with("/codex/responses") {
        normalized.to_string()
    } else if normalized.ends_with("/codex") {
        format!("{normalized}/responses")
    } else {
        format!("{normalized}/codex/responses")
    }
}

fn extract_account_id(token: &str) -> Result<String, &'static str> {
    let mut segments = token.split('.');
    let (Some(_header), Some(payload), Some(_signature), None) = (
        segments.next(),
        segments.next(),
        segments.next(),
        segments.next(),
    ) else {
        return Err("Failed to extract accountId from token");
    };
    let payload = URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .map_err(|_| "Failed to extract accountId from token")?;
    let payload: Value =
        serde_json::from_slice(&payload).map_err(|_| "Failed to extract accountId from token")?;
    payload
        .get(ACCOUNT_CLAIM)
        .and_then(|claim| claim.get("chatgpt_account_id"))
        .and_then(Value::as_str)
        .filter(|account_id| !account_id.is_empty())
        .map(str::to_string)
        .ok_or("Failed to extract accountId from token")
}

fn format_codex_http_error(status: u16, response_body: &str) -> String {
    let payload: Value = serde_json::from_str(response_body).unwrap_or(Value::Null);
    let error = payload.get("error");
    let code = error
        .and_then(|error| error.get("code").or_else(|| error.get("type")))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let recognized_limit = status == 429
        || [
            "usage_limit_reached",
            "usage_not_included",
            "rate_limit_exceeded",
        ]
        .iter()
        .any(|known| code.to_ascii_lowercase().contains(known));
    if recognized_limit {
        let plan = error
            .and_then(|error| error.get("plan_type"))
            .and_then(Value::as_str)
            .filter(|plan| !plan.is_empty())
            .map(|plan| format!(" ({} plan)", plan.to_lowercase()))
            .unwrap_or_default();
        let reset = error
            .and_then(|error| error.get("resets_at"))
            .and_then(Value::as_f64)
            .filter(|timestamp| timestamp.is_finite() && *timestamp > 0.0)
            .map(|timestamp| {
                let now_seconds = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|duration| duration.as_secs_f64())
                    .unwrap_or(0.0);
                ((timestamp - now_seconds) / 60.0).round().max(0.0) as u64
            });
        let retry = reset
            .map(|minutes| format!(" Try again in ~{minutes} min."))
            .unwrap_or_default();
        return format!("You have hit your ChatGPT usage limit{plan}.{retry}");
    }

    error
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
        .filter(|message| !message.is_empty())
        .map(str::to_string)
        .or_else(|| (!response_body.is_empty()).then(|| response_body.to_string()))
        .unwrap_or_else(|| format!("Request failed with status {status}"))
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

fn emit_error(
    producer: &mut AssistantMessageEventStreamProducer,
    output: &mut AssistantMessage,
    message: impl Into<String>,
    aborted: bool,
) {
    crate::providers::openai_responses::emit_error(producer, output, message, aborted);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{OpenaiResponsesCompat, StreamingProtocolCompat};
    use crate::providers::openai_responses::request::MAX_CACHE_KEY_CHARS;
    use crate::types::{
        Api, AssistantMessage, ConstrainedSamplingConfig, ConstrainedStrictness, Content,
        InputModality, Message, Schema, StopReason, TextContent, TextContentType, ThinkingContent,
        ThinkingContentType, Tool, ToolCall, ToolCallType, ToolResultMessage, ToolResultRole,
        UserMessage,
    };
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;
    use serde_json::json;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn model() -> Model {
        let mut model = Model::new(
            "gpt-5.4",
            "GPT-5.4",
            Api::OpenaiCodexResponses,
            "openai-codex",
            "https://chatgpt.com/backend-api",
        );
        model.reasoning = true;
        model.input = vec![InputModality::Text, InputModality::Image];
        model.max_tokens = 128_000;
        model.context_window = 272_000;
        model.compat = Some(StreamingProtocolCompat::OpenaiResponses(
            OpenaiResponsesCompat {
                supports_strict_mode: Some(true),
                supports_openai_grammar_tools: Some(true),
                ..Default::default()
            },
        ));
        model
    }

    fn token(account_id: &str) -> String {
        let payload = URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&json!({
                "https://api.openai.com/auth": { "chatgpt_account_id": account_id }
            }))
            .unwrap(),
        );
        format!("header.{payload}.signature")
    }

    fn tool() -> Tool {
        Tool {
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
        }
    }

    #[test]
    fn codex_url_normalizes_all_supported_base_shapes() {
        for (base, expected) in [
            (
                "https://chatgpt.com/backend-api",
                "https://chatgpt.com/backend-api/codex/responses",
            ),
            (
                "https://chatgpt.com/backend-api/codex/",
                "https://chatgpt.com/backend-api/codex/responses",
            ),
            (
                "https://chatgpt.com/backend-api/codex/responses///",
                "https://chatgpt.com/backend-api/codex/responses",
            ),
            (" ", "https://chatgpt.com/backend-api/codex/responses"),
        ] {
            assert_eq!(codex_responses_url(base), expected);
        }
    }

    #[test]
    fn codex_request_has_typed_defaults_and_omits_unsupplied_limits() {
        let request = build_codex_request(
            &model(),
            &Context::new(Vec::new()),
            &SimpleStreamOptions::default(),
        )
        .unwrap();

        assert_eq!(request["model"], "gpt-5.4");
        assert_eq!(request["input"], json!([]));
        assert_eq!(request["stream"], true);
        assert_eq!(request["store"], false);
        assert_eq!(request["instructions"], DEFAULT_INSTRUCTIONS);
        assert_eq!(request["text"]["verbosity"], "low");
        assert_eq!(request["include"], json!(["reasoning.encrypted_content"]));
        assert_eq!(request["tool_choice"], "auto");
        assert_eq!(request["parallel_tool_calls"], true);
        assert!(request.get("max_output_tokens").is_none());
        assert!(request.get("temperature").is_none());
        assert!(request.get("reasoning").is_none());
    }

    #[test]
    fn empty_system_prompt_uses_the_codex_default() {
        let mut context = Context::new(Vec::new());
        context.system_prompt = Some(String::new());
        let request =
            build_codex_request(&model(), &context, &SimpleStreamOptions::default()).unwrap();
        assert_eq!(request["instructions"], DEFAULT_INSTRUCTIONS);
    }

    #[test]
    fn codex_function_tools_default_to_null_without_rewriting_optional_fields() {
        let request = build_codex_request(
            &model(),
            &Context {
                system_prompt: None,
                messages: Vec::new(),
                tools: vec![tool()],
            },
            &SimpleStreamOptions::default(),
        )
        .unwrap();
        assert_eq!(request["tools"][0]["strict"], Value::Null);
        assert_eq!(
            request["tools"][0]["parameters"]["required"],
            json!(["path"])
        );
        assert!(request["tools"][0]["parameters"]
            .get("additionalProperties")
            .is_none());
    }

    #[test]
    fn explicit_constrained_sampling_prepares_a_strict_schema_before_serializing() {
        let mut strict_tool = tool();
        strict_tool.constrained_sampling = Some(ConstrainedSamplingConfig::JsonSchema {
            strict: ConstrainedStrictness::Prefer,
        });
        let request = build_codex_request(
            &model(),
            &Context {
                system_prompt: None,
                messages: Vec::new(),
                tools: vec![strict_tool],
            },
            &SimpleStreamOptions::default(),
        )
        .unwrap();
        assert_eq!(request["tools"][0]["strict"], true);
        assert_eq!(
            request["tools"][0]["parameters"]["additionalProperties"],
            false
        );
        let mut required: Vec<&str> = request["tools"][0]["parameters"]["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap())
            .collect();
        required.sort_unstable();
        assert_eq!(required, vec!["offset", "path"]);
    }

    #[test]
    fn reasoning_uses_typed_maps_and_explicit_off_policy() {
        use crate::types::ThinkingLevelMap;

        let mut model = model();
        model.thinking_level_map = Some(ThinkingLevelMap::from([
            (ThinkingLevel::Off, None),
            (ThinkingLevel::Minimal, Some("low".into())),
            (ThinkingLevel::High, Some("high".into())),
            (ThinkingLevel::Xhigh, None),
        ]));
        assert_eq!(
            codex_reasoning_effort(&model, ThinkingLevel::Off).as_deref(),
            Some("none")
        );
        assert_eq!(
            codex_reasoning_effort(&model, ThinkingLevel::Minimal).as_deref(),
            Some("low")
        );
        assert_eq!(
            codex_reasoning_effort(&model, ThinkingLevel::High).as_deref(),
            Some("high")
        );
        assert_eq!(codex_reasoning_effort(&model, ThinkingLevel::Xhigh), None);

        let mut opts = SimpleStreamOptions::default();
        opts.reasoning = Some(ThinkingLevel::High);
        let request = build_codex_request(&model, &Context::new(Vec::new()), &opts).unwrap();
        assert_eq!(request["reasoning"]["effort"], "high");
        assert_eq!(request["reasoning"]["summary"], "auto");

        opts.reasoning = Some(ThinkingLevel::Off);
        let request = build_codex_request(&model, &Context::new(Vec::new()), &opts).unwrap();
        assert_eq!(request["reasoning"]["effort"], "none");
    }

    #[test]
    fn codex_matches_reference_and_does_not_merge_model_sampling_parameters() {
        let mut model = model();
        model.sampling_params = Some(
            json!({
                "top_p": 0.4,
                "store": true,
                "model": "sampling-model",
                "custom_sampling_parameter": {"enabled": true}
            })
            .as_object()
            .unwrap()
            .clone(),
        );
        let request = build_codex_request(
            &model,
            &Context::new(Vec::new()),
            &SimpleStreamOptions::default(),
        )
        .unwrap();
        assert_eq!(request["model"], model.id);
        assert_eq!(request["store"], false);
        assert_eq!(request["stream"], true);
        assert!(request.get("top_p").is_none());
        assert!(request.get("custom_sampling_parameter").is_none());
    }

    #[test]
    fn hard_output_limit_has_no_ordinary_responses_floor() {
        let model = model();
        let mut opts = SimpleStreamOptions::default();
        opts.max_tokens = Some(8000);
        opts.hard_max_output_tokens = Some(2048.5);
        let request = build_codex_request(&model, &Context::new(Vec::new()), &opts).unwrap();
        assert_eq!(request["max_output_tokens"], 2048);

        opts.max_tokens = None;
        opts.hard_max_output_tokens = Some(8.5);
        let request = build_codex_request(&model, &Context::new(Vec::new()), &opts).unwrap();
        assert_eq!(request["max_output_tokens"], 8);
    }

    #[test]
    fn cache_none_suppresses_generated_session_and_cache_identity() {
        let mut model = model();
        model.sampling_params = Some(
            json!({"prompt_cache_key": "sampling-key", "top_p": 0.2})
                .as_object()
                .unwrap()
                .clone(),
        );
        let mut opts = SimpleStreamOptions::default();
        opts.cache_retention = CacheRetention::None;
        opts.session_id = Some("session".into());
        let request = build_codex_request(&model, &Context::new(Vec::new()), &opts).unwrap();
        assert!(request.get("prompt_cache_key").is_none());
        assert!(request.get("top_p").is_none());
        assert!(codex_session_id(&opts).is_none());
    }

    #[test]
    fn explicit_temperature_and_output_options_are_serialized() {
        let mut opts = SimpleStreamOptions::default();
        opts.temperature = Some(0.25);
        opts.max_tokens = Some(4096);
        let request = build_codex_request(&model(), &Context::new(Vec::new()), &opts).unwrap();
        assert_eq!(request["temperature"], 0.25);
        assert_eq!(request["max_output_tokens"], 4096);
    }

    #[test]
    fn session_and_prompt_cache_keys_are_bounded_by_unicode_characters() {
        let mut opts = SimpleStreamOptions::default();
        opts.session_id = Some("界".repeat(70));
        let session_id = codex_session_id(&opts).unwrap();
        assert_eq!(session_id.chars().count(), MAX_CACHE_KEY_CHARS);
        let request = build_codex_request(&model(), &Context::new(Vec::new()), &opts).unwrap();
        assert_eq!(
            request["prompt_cache_key"]
                .as_str()
                .unwrap()
                .chars()
                .count(),
            MAX_CACHE_KEY_CHARS
        );
    }

    #[test]
    fn account_id_is_extracted_without_exposing_invalid_tokens() {
        assert_eq!(
            extract_account_id(&token("acct_test")).unwrap(),
            "acct_test"
        );
        assert_eq!(
            extract_account_id("not-a-jwt").unwrap_err(),
            "Failed to extract accountId from token"
        );
    }

    #[test]
    fn quota_error_uses_available_plan_and_reset_metadata_only() {
        let reset = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 120;
        let with_metadata = format_codex_http_error(
            429,
            &json!({
                "error": {
                    "code": "rate_limit_exceeded",
                    "message": "rate limit",
                    "plan_type": "Plus",
                    "resets_at": reset
                }
            })
            .to_string(),
        );
        assert!(with_metadata.contains("ChatGPT usage limit (plus plan)"));
        assert!(with_metadata.contains("Try again in ~2 min."));

        let without_metadata = format_codex_http_error(
            429,
            r#"{"error":{"code":"usage_limit_reached","message":"limit"}}"#,
        );
        assert_eq!(without_metadata, "You have hit your ChatGPT usage limit.");

        let invalid_token = format_codex_http_error(
            401,
            r#"{"error":{"code":"invalid_api_key","message":"invalid token"}}"#,
        );
        assert_eq!(invalid_token, "invalid token");
    }

    #[tokio::test]
    async fn fake_http_request_has_codex_url_headers_and_typed_body() {
        let (address, request_rx, server) =
            fake_server(200, "text/event-stream", completed_event("resp_test")).await;
        let mut model = model();
        model.id = "gpt-5.6-luna".into();
        model.base_url = format!("http://{address}/codex");
        model.headers = Some(BTreeMap::from([
            ("authorization".into(), "Bearer stale".into()),
            ("x-api-key".into(), "stale-api-key".into()),
            ("originator".into(), "wrong".into()),
        ]));
        let provider = OpenAiCodexResponsesProvider::with_models(
            "openai-codex",
            Some(token("acct_fake")),
            reqwest::Client::new(),
            vec![model.clone()],
        );
        let mut opts = SimpleStreamOptions::default();
        opts.session_id = Some("session-α".into());
        opts.headers = Some(BTreeMap::from([("X-Custom".into(), "caller".into())]));
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            provider
                .stream_simple(
                    &model,
                    &Context::new(vec![Message::User(UserMessage::new("hey there", 1))]),
                    &opts,
                )
                .await
                .result(),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(result.stop_reason, StopReason::Stop);

        let captured = request_rx.await.unwrap();
        assert_eq!(captured.path, "/codex/responses");
        assert_eq!(
            captured.headers["authorization"],
            format!("Bearer {}", token("acct_fake"))
        );
        assert_eq!(captured.headers["chatgpt-account-id"], "acct_fake");
        assert_eq!(captured.headers["originator"], "ice");
        assert_eq!(captured.headers["openai-beta"], "responses=experimental");
        assert_eq!(captured.headers["accept"], "text/event-stream");
        assert_eq!(captured.headers["content-type"], "application/json");
        assert!(!captured.headers.contains_key("x-api-key"));
        assert_eq!(captured.headers["x-custom"], "caller");
        assert_eq!(captured.headers["session-id"], "session-α");
        assert_eq!(captured.headers["x-client-request-id"], "session-α");
        assert!(captured.headers["user-agent"].starts_with("ice ("));
        assert_eq!(captured.body["store"], false);
        assert_eq!(captured.body["stream"], true);
        assert_eq!(captured.body["model"], "gpt-5.6-luna");
        assert_eq!(
            captured.body["input"][0]["content"],
            json!([{"type": "input_text", "text": "hey there"}])
        );
        assert_eq!(captured.body["prompt_cache_key"], "session-α");
        server.await.unwrap();
    }

    #[tokio::test]
    async fn cache_none_omits_session_headers_and_prompt_cache_key() {
        let (address, request_rx, server) =
            fake_server(200, "text/event-stream", completed_event("resp_no_cache")).await;
        let mut model = model();
        model.base_url = format!("http://{address}");
        let provider = OpenAiCodexResponsesProvider::with_models(
            "openai-codex",
            Some(token("acct_fake")),
            reqwest::Client::new(),
            vec![model.clone()],
        );
        let mut opts = SimpleStreamOptions::default();
        opts.cache_retention = CacheRetention::None;
        opts.session_id = Some("must-not-be-sent".into());
        let result = provider
            .stream_simple(&model, &Context::new(Vec::new()), &opts)
            .await
            .result()
            .await
            .unwrap();
        assert_eq!(result.stop_reason, StopReason::Stop);
        let captured = request_rx.await.unwrap();
        assert!(!captured.headers.contains_key("session-id"));
        assert!(!captured.headers.contains_key("x-client-request-id"));
        assert!(captured.body.get("prompt_cache_key").is_none());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn completed_event_finishes_while_http_body_remains_open() {
        let (address, _request_rx, server) =
            fake_open_body_server(completed_event("resp_open")).await;
        let mut model = model();
        model.base_url = format!("http://{address}");
        let provider = OpenAiCodexResponsesProvider::with_models(
            "openai-codex",
            Some(token("acct_fake")),
            reqwest::Client::new(),
            vec![model.clone()],
        );
        let result = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            provider
                .stream_simple(
                    &model,
                    &Context::new(Vec::new()),
                    &SimpleStreamOptions::default(),
                )
                .await
                .result(),
        )
        .await
        .expect("terminal response event should stop the reader")
        .unwrap();
        assert_eq!(result.stop_reason, StopReason::Stop);
        server.abort();
    }

    #[tokio::test]
    async fn incomplete_and_premature_eof_keep_responses_terminal_semantics() {
        let incomplete = concat!(
            "event: response.incomplete\r\n",
            "data: {\"type\":\"response.incomplete\",\"response\":{\"status\":\"incomplete\",\"incomplete_details\":{\"reason\":\"max_output_tokens\"},\"output\":[]}}\r\n",
            "\r\n",
        );
        let (address, _, server) = fake_server(200, "text/event-stream", incomplete).await;
        let mut model = model();
        model.base_url = format!("http://{address}");
        let provider = OpenAiCodexResponsesProvider::with_models(
            "openai-codex",
            Some(token("acct_fake")),
            reqwest::Client::new(),
            vec![model.clone()],
        );
        let result = provider
            .stream_simple(
                &model,
                &Context::new(Vec::new()),
                &SimpleStreamOptions::default(),
            )
            .await
            .result()
            .await
            .unwrap();
        assert_eq!(result.stop_reason, StopReason::Length);
        server.await.unwrap();

        let partial = concat!(
            "event: response.output_text.delta\r\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"partial\",\"output_index\":0,\"content_index\":0}\r\n",
            "\r\n",
        );
        let (address, _, server) = fake_server(200, "text/event-stream", partial).await;
        model.base_url = format!("http://{address}");
        let provider = OpenAiCodexResponsesProvider::with_models(
            "openai-codex",
            Some(token("acct_fake")),
            reqwest::Client::new(),
            vec![model.clone()],
        );
        let result = provider
            .stream_simple(
                &model,
                &Context::new(Vec::new()),
                &SimpleStreamOptions::default(),
            )
            .await
            .result()
            .await
            .unwrap();
        assert_eq!(result.stop_reason, StopReason::Error);
        assert!(result
            .error_message
            .as_deref()
            .is_some_and(|message| message.contains("ended before a terminal response event")));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn final_sse_frame_is_flushed_at_fragmented_eof() {
        let terminal_frame_without_blank_line = concat!(
            "event: response.completed\r\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_eof\",\"status\":\"completed\",\"output\":[]}}"
        );
        let (address, _request_rx, server) =
            fake_fragmented_eof_server(terminal_frame_without_blank_line).await;
        let mut model = model();
        model.base_url = format!("http://{address}");
        let provider = OpenAiCodexResponsesProvider::with_models(
            "openai-codex",
            Some(token("acct_fake")),
            reqwest::Client::new(),
            vec![model.clone()],
        );
        let result = provider
            .stream_simple(
                &model,
                &Context::new(Vec::new()),
                &SimpleStreamOptions::default(),
            )
            .await
            .result()
            .await
            .unwrap();
        assert_eq!(result.stop_reason, StopReason::Stop);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn second_request_replays_tool_results_and_reasoning_signatures() {
        let (address, request_rx, server) =
            fake_server(200, "text/event-stream", completed_event("resp_replay")).await;
        let mut model = model();
        model.base_url = format!("http://{address}");
        let provider = OpenAiCodexResponsesProvider::with_models(
            "openai-codex",
            Some(token("acct_fake")),
            reqwest::Client::new(),
            vec![model.clone()],
        );
        let assistant = assistant_with_tool_and_signature(&model);
        let context = Context::new(vec![
            Message::User(UserMessage::new("run read", 1)),
            Message::Assistant(Box::new(assistant)),
            tool_result("call_1"),
        ]);
        let result = provider
            .stream_simple(&model, &context, &SimpleStreamOptions::default())
            .await
            .result()
            .await
            .unwrap();
        assert_eq!(result.stop_reason, StopReason::Stop);

        let captured = request_rx.await.unwrap();
        let input = captured.body["input"].as_array().unwrap();
        let reasoning = input
            .iter()
            .find(|item| item["type"] == "reasoning")
            .expect("same-model encrypted reasoning must replay");
        assert_eq!(reasoning["encrypted_content"], "opaque-signature");
        let call_index = input
            .iter()
            .position(|item| item["type"] == "function_call")
            .expect("function call must replay");
        let result_index = input
            .iter()
            .position(|item| item["type"] == "function_call_output")
            .expect("tool result must replay");
        assert!(call_index < result_index);
        assert_eq!(input[call_index]["call_id"], "call_1");
        assert_eq!(input[result_index]["call_id"], "call_1");
        server.await.unwrap();
    }

    #[tokio::test]
    async fn quota_and_non_quota_http_errors_are_formatted_without_retry() {
        let (address, request_rx, server) = fake_server(
            429,
            "application/json",
            r#"{"error":{"code":"usage_limit_reached","plan_type":"plus","message":"quota reached"}}"#,
        )
        .await;
        let mut model = model();
        model.base_url = format!("http://{address}");
        let provider = OpenAiCodexResponsesProvider::with_models(
            "openai-codex",
            Some(token("acct_fake")),
            reqwest::Client::new(),
            vec![model.clone()],
        );
        let mut opts = SimpleStreamOptions::default();
        opts.max_retries = Some(3);
        let result = provider
            .stream_simple(&model, &Context::new(Vec::new()), &opts)
            .await
            .result()
            .await
            .unwrap();
        assert_eq!(result.stop_reason, StopReason::Error);
        assert_eq!(
            result.error_message.as_deref(),
            Some("You have hit your ChatGPT usage limit (plus plan).")
        );
        let _ = request_rx.await.unwrap();
        server.await.unwrap();

        let (address, _, server) = fake_server(
            401,
            "application/json",
            r#"{"error":{"code":"invalid_api_key","message":"invalid token"}}"#,
        )
        .await;
        model.base_url = format!("http://{address}");
        let provider = OpenAiCodexResponsesProvider::with_models(
            "openai-codex",
            Some(token("acct_fake")),
            reqwest::Client::new(),
            vec![model.clone()],
        );
        let result = provider
            .stream_simple(
                &model,
                &Context::new(Vec::new()),
                &SimpleStreamOptions::default(),
            )
            .await
            .result()
            .await
            .unwrap();
        assert_eq!(result.stop_reason, StopReason::Error);
        assert_eq!(result.error_message.as_deref(), Some("invalid token"));
        server.await.unwrap();
    }

    fn assistant_with_tool_and_signature(model: &Model) -> AssistantMessage {
        let mut assistant = AssistantMessage::empty(
            model.api.clone(),
            model.provider.clone(),
            model.id.clone(),
            2,
        );
        assistant.stop_reason = StopReason::ToolUse;
        assistant.content = vec![
            Content::Thinking(ThinkingContent {
                kind: ThinkingContentType,
                thinking: String::new(),
                thinking_signature: Some(
                    json!({
                        "type": "reasoning",
                        "id": "rs_1",
                        "encrypted_content": "opaque-signature"
                    })
                    .to_string(),
                ),
                redacted: false,
            }),
            Content::ToolCall(ToolCall {
                kind: ToolCallType,
                id: "call_1".into(),
                name: "read".into(),
                arguments: json!({"path": "README.md"}),
                thought_signature: None,
                namespace: None,
            }),
        ];
        assistant
    }

    fn tool_result(id: &str) -> Message {
        ToolResultMessage {
            role: ToolResultRole,
            tool_call_id: id.into(),
            tool_name: "read".into(),
            content: vec![Content::Text(TextContent {
                kind: TextContentType,
                text: "file content".into(),
                text_signature: None,
            })],
            details: None,
            usage: Some(crate::types::Usage::zero()),
            added_tool_names: Vec::new(),
            is_error: false,
            timestamp: 3,
        }
        .into()
    }

    fn completed_event(response_id: &str) -> String {
        format!(
            "event: response.completed\r\ndata: {{\"type\":\"response.completed\",\"response\":{{\"id\":\"{response_id}\",\"status\":\"completed\",\"output\":[]}}}}\r\n\r\n"
        )
    }

    #[derive(Debug)]
    struct CapturedRequest {
        path: String,
        headers: BTreeMap<String, String>,
        body: Value,
    }

    async fn fake_server(
        status: u16,
        content_type: &str,
        response_body: impl Into<String>,
    ) -> (
        std::net::SocketAddr,
        tokio::sync::oneshot::Receiver<CapturedRequest>,
        tokio::task::JoinHandle<()>,
    ) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (request_tx, request_rx) = tokio::sync::oneshot::channel();
        let content_type = content_type.to_string();
        let response_body = response_body.into();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_request(&mut socket).await;
            let (status, reason, content_type, response_body) =
                if request.headers.get("content-type").map(String::as_str)
                    != Some("application/json")
                {
                    (
                        415,
                        "Unsupported Media Type",
                        "application/json",
                        r#"{"detail":"Unsupported content type"}"#.to_string(),
                    )
                } else {
                    (
                        status,
                        if status == 200 { "OK" } else { "Error" },
                        content_type.as_str(),
                        response_body,
                    )
                };
            let _ = request_tx.send(request);
            let response = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
                response_body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        });
        (address, request_rx, server)
    }

    async fn fake_open_body_server(
        response_body: impl Into<String>,
    ) -> (
        std::net::SocketAddr,
        tokio::sync::oneshot::Receiver<CapturedRequest>,
        tokio::task::JoinHandle<()>,
    ) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (request_tx, request_rx) = tokio::sync::oneshot::channel();
        let response_body = response_body.into();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_request(&mut socket).await;
            let _ = request_tx.send(request);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n{response_body}",
                response_body.len() + 1024
            );
            socket.write_all(response.as_bytes()).await.unwrap();
            std::future::pending::<()>().await;
        });
        (address, request_rx, server)
    }

    async fn fake_fragmented_eof_server(
        response_body: impl Into<String>,
    ) -> (
        std::net::SocketAddr,
        tokio::sync::oneshot::Receiver<CapturedRequest>,
        tokio::task::JoinHandle<()>,
    ) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (request_tx, request_rx) = tokio::sync::oneshot::channel();
        let response_body = response_body.into();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_request(&mut socket).await;
            let _ = request_tx.send(request);
            let headers = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                response_body.len()
            );
            socket.write_all(headers.as_bytes()).await.unwrap();
            for byte in response_body.as_bytes().chunks(3) {
                socket.write_all(byte).await.unwrap();
                tokio::task::yield_now().await;
            }
        });
        (address, request_rx, server)
    }

    async fn read_request(socket: &mut tokio::net::TcpStream) -> CapturedRequest {
        let mut request = Vec::new();
        let mut chunk = [0u8; 4096];
        let body_start;
        let content_length;
        loop {
            let read = socket.read(&mut chunk).await.unwrap();
            assert!(read > 0, "client closed before sending an HTTP request");
            request.extend_from_slice(&chunk[..read]);
            if let Some(header_end) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                body_start = header_end + 4;
                let raw_headers = String::from_utf8_lossy(&request[..header_end]);
                content_length = raw_headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().unwrap())
                    })
                    .expect("reqwest JSON body must have Content-Length");
                break;
            }
        }
        while request.len() < body_start + content_length {
            let read = socket.read(&mut chunk).await.unwrap();
            assert!(read > 0, "client closed before sending the full body");
            request.extend_from_slice(&chunk[..read]);
        }

        let head = String::from_utf8_lossy(&request[..body_start - 4]);
        let mut lines = head.lines();
        let path = lines
            .next()
            .and_then(|request_line| request_line.split_whitespace().nth(1))
            .unwrap()
            .to_string();
        // Repeated header lines are joined as HTTP does, so a duplicated
        // `Content-Type` is visible to assertions (the real backend rejects it).
        let mut headers = BTreeMap::<String, String>::new();
        for line in lines {
            let Some((name, value)) = line.split_once(':') else {
                continue;
            };
            let value = value.trim();
            headers
                .entry(name.trim().to_ascii_lowercase())
                .and_modify(|existing| {
                    existing.push_str(", ");
                    existing.push_str(value);
                })
                .or_insert_with(|| value.to_string());
        }
        let body =
            serde_json::from_slice(&request[body_start..body_start + content_length]).unwrap();
        CapturedRequest {
            path,
            headers,
            body,
        }
    }
}
