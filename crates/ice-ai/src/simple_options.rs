//! Shared request bounds, mirroring `packages/ai/src/api/simple-options.ts`.

use crate::model::Model;
use crate::provider::SimpleStreamOptions;
use crate::types::{Content, Context, Message, UserContent};
use serde_json::{Map, Value};

/// Clamp without widening runtime authority. `None` also represents wire null.
/// Floating-point inputs preserve validation of NaN and infinity at this seam.
pub fn clamp_max_tokens_to_hard_limit(
    requested: Option<f64>,
    hard: Option<f64>,
) -> Result<Option<f64>, String> {
    let hard = match hard {
        None => return Ok(requested),
        Some(hard) => hard,
    };
    if !hard.is_finite() || hard <= 0.0 {
        return Err("hardMaxOutputTokens must be a positive finite number.".into());
    }
    match requested {
        Some(value) if !value.is_finite() || value <= 0.0 => Err(
            "maxTokens must be a positive finite number when a hard output authority is supplied."
                .into(),
        ),
        Some(value) => Ok(Some(value.min(hard))),
        None => Ok(Some(hard)),
    }
}

/// Reapply authority after all adapter payload transforms. Deletion and null
/// mean unset, not permission to remove the runtime ceiling.
pub fn enforce_hard_max_tokens_in_record(
    payload: &mut Map<String, Value>,
    field: &str,
    hard: Option<f64>,
) -> Result<(), String> {
    if hard.is_none() {
        return Ok(());
    }
    let requested = match payload.get(field) {
        None | Some(Value::Null) => None,
        Some(Value::Number(number)) => number.as_f64(),
        Some(_) => {
            return Err(format!(
                "{field} must be a number when a hard output authority is supplied."
            ));
        }
    };
    if let Some(limit) = clamp_max_tokens_to_hard_limit(requested, hard)? {
        // Provider token fields are integers. Round down so a fractional limit
        // never widens authority or reaches the wire as e.g. 4096.5.
        payload.insert(field.into(), Value::from(whole_tokens(limit)?));
    }
    Ok(())
}

/// Floor a validated clamp to whole tokens; a sub-token limit allows nothing.
fn whole_tokens(limit: f64) -> Result<u64, String> {
    let tokens = limit.floor() as u64;
    if tokens == 0 {
        return Err("hardMaxOutputTokens must allow at least one output token.".into());
    }
    Ok(tokens)
}

/// Conservative content estimate, not an exact provider tokenizer. Tool-call
/// arguments are input on subsequent requests just like ordinary text.
pub fn estimate_context_tokens(messages: &[Message], system_prompt: Option<&str>) -> u64 {
    fn block_chars(blocks: &[Content], include_thinking: bool) -> u64 {
        blocks
            .iter()
            .map(|block| match block {
                Content::Text(t) => t.text.len() as u64,
                Content::Thinking(t) if include_thinking => t.thinking.len() as u64,
                Content::Image(_) => 4800,
                Content::ToolCall(call) => {
                    call.name.len() as u64
                        + call.id.len() as u64
                        + call.arguments.to_string().len() as u64
                }
                _ => 0,
            })
            .sum()
    }
    let mut chars = system_prompt.map_or(0, |s| s.len() as u64);
    for message in messages {
        chars += match message {
            Message::User(user) => match &user.content {
                UserContent::Text(text) => text.len() as u64,
                UserContent::Blocks(blocks) => block_chars(blocks, false),
            },
            Message::Assistant(assistant) => block_chars(&assistant.content, true),
            Message::ToolResult(result) => block_chars(&result.content, false),
        };
    }
    (chars + 3) / 4
}

/// Estimate the complete request, including tools and per-message framing.
/// Serialized schema bytes deliberately err toward reserving too much headroom.
pub fn estimate_request_tokens(context: &Context) -> u64 {
    let schemas = context.tools.iter().fold(0u64, |total, tool| {
        total.saturating_add(serde_json::to_string(tool).map_or(0, |s| s.len() as u64))
    });
    estimate_context_tokens(&context.messages, context.system_prompt.as_deref())
        .saturating_add(schemas.div_ceil(4))
        .saturating_add((context.messages.len() as u64).saturating_mul(16))
}

/// Legacy message-only clamp. New provider requests should use
/// [`request_max_tokens`], which includes tool schemas and rejects exhausted input.
/// Reserve 4096 context tokens and keep at least one token of headroom.
pub fn clamp_max_tokens_to_context(
    model: &Model,
    messages: &[Message],
    system_prompt: Option<&str>,
    requested: u64,
) -> u64 {
    if model.context_window == 0 {
        return requested.max(1);
    }
    let available = model
        .context_window
        .saturating_sub(estimate_context_tokens(messages, system_prompt))
        .saturating_sub(4096);
    requested.min(available.max(1))
}

/// Output allowance when the conservative input estimate leaves no room at all.
const EXHAUSTED_OUTPUT_FLOOR: u64 = 4096;

/// Build the shared integer allowance. Validate explicit requests before context
/// fitting can hide a zero. Fractional hard authority is rounded down, never up.
/// Adapters must also enforce the original authority on the final wire record.
pub fn request_max_tokens(
    model: &Model,
    context: &Context,
    options: &SimpleStreamOptions,
) -> Result<u64, String> {
    clamp_max_tokens_to_hard_limit(
        options.max_tokens.map(|value| value as f64),
        options.hard_max_output_tokens,
    )?;
    let mut requested = options.max_tokens.unwrap_or(model.max_tokens).max(1);
    if model.context_window != 0 {
        // Remaining estimated room bounds the output. When the estimate says
        // nothing is left, do not refuse: it rounds up (bytes / 4) and has
        // rejected requests the provider measured well inside the window. The
        // provider decides; a real overflow reaches the harness's overflow
        // recovery. The floor keeps that request's output usable.
        let input = estimate_request_tokens(context);
        let available = model
            .context_window
            .saturating_sub(input)
            .saturating_sub(4096);
        if available == 0 {
            tracing::debug!(
                input,
                context_window = model.context_window,
                "estimated request input exhausts the window; deferring to the provider"
            );
            requested = requested.min(EXHAUSTED_OUTPUT_FLOOR);
        } else {
            requested = requested.min(available);
        }
    }
    match clamp_max_tokens_to_hard_limit(Some(requested as f64), options.hard_max_output_tokens)? {
        Some(limit) if options.hard_max_output_tokens.is_some() => whole_tokens(limit),
        _ => Ok(requested),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Api;
    use serde_json::json;

    #[test]
    fn request_estimates_are_monotonic_for_unicode_images_and_thinking() {
        let mut ctx = Context::default();
        ctx.system_prompt = Some("界".repeat(4000));
        assert!(estimate_request_tokens(&ctx) >= 3000);
        let mut model = Model::new("test", "test", Api::Faux, "faux", "http://localhost:0");
        model.context_window = 20000;
        model.max_tokens = 16000;
        let opts = SimpleStreamOptions::default();
        let mut previous = request_max_tokens(&model, &ctx, &opts).unwrap();
        for block in [
            json!({"type":"image", "data":"AA==", "mimeType":"image/png"}),
            json!({"type":"thinking", "thinking":"x".repeat(4000)}),
            json!({"type":"text", "text":"x".repeat(4000)}),
        ] {
            let mut assistant = crate::types::AssistantMessage::empty(Api::Faux, "faux", "test", 0);
            assistant
                .content
                .push(serde_json::from_value(block).unwrap());
            ctx.messages.push(Message::Assistant(Box::new(assistant)));
            let current = request_max_tokens(&model, &ctx, &opts).unwrap();
            assert!(current < previous);
            previous = current;
        }
        // An estimate alone never refuses: the floor applies instead.
        model.context_window = 4096;
        assert_eq!(
            request_max_tokens(&model, &Context::default(), &opts).unwrap(),
            4096
        );
        model.context_window = 0;
        assert_eq!(
            request_max_tokens(&model, &ctx, &opts).unwrap(),
            model.max_tokens
        );
    }

    /// H18 (2026-11-04): 30 parallel reads estimated at 71k tokens against a
    /// 48k window were refused locally, though the provider measured 49k and
    /// accepted the same request on the previous build. The rounded-up
    /// estimate must not refuse a request or shrink output below the floor;
    /// the provider decides, and a real overflow reaches overflow recovery.
    #[test]
    fn conservative_estimate_never_refuses_or_starves_the_request() {
        let mut model = Model::new("test", "test", Api::Faux, "faux", "http://localhost:0");
        model.context_window = 48_000;
        model.max_tokens = 8192;
        let mut context = Context::default();
        context
            .messages
            .push(Message::User(crate::types::UserMessage::new(
                "x".repeat(270_000),
                1,
            )));
        let opts = SimpleStreamOptions::default();
        assert!(estimate_request_tokens(&context) > model.context_window);
        assert_eq!(request_max_tokens(&model, &context, &opts).unwrap(), 4096);
        // A smaller explicit request is kept as is.
        let small = SimpleStreamOptions {
            max_tokens: Some(512),
            ..SimpleStreamOptions::default()
        };
        assert_eq!(request_max_tokens(&model, &context, &small).unwrap(), 512);
        // Room left above the floor is still used.
        context.messages.clear();
        assert_eq!(request_max_tokens(&model, &context, &opts).unwrap(), 8192);
    }

    #[test]
    fn request_budget_counts_tool_arguments_and_schemas() {
        let mut assistant = crate::types::AssistantMessage::empty(Api::Faux, "faux", "test", 0);
        assistant.content.push(
            serde_json::from_value(json!({
                "type": "toolCall", "id": "call", "name": "write",
                "arguments": {"content": "x".repeat(200_000)}
            }))
            .unwrap(),
        );
        assert!(
            estimate_context_tokens(&[Message::Assistant(Box::new(assistant))], None) >= 50_000
        );
        let mut model = Model::new("test", "test", Api::Faux, "faux", "http://localhost:0");
        model.context_window = 8192;
        model.max_tokens = 4096;
        let mut context = Context::default();
        let before = request_max_tokens(&model, &context, &SimpleStreamOptions::default()).unwrap();
        context.tools.push(
            serde_json::from_value(json!({
                "name": "write", "description": "x".repeat(4000),
                "parameters": {"type": "object"}
            }))
            .unwrap(),
        );
        let after = request_max_tokens(&model, &context, &SimpleStreamOptions::default()).unwrap();
        assert!(
            after + 900 < before,
            "schema must reduce headroom: {before} -> {after}"
        );
        context.system_prompt = Some("x".repeat(40_000));
        // Estimated exhaustion defers to the provider with the output floor (H18).
        assert_eq!(
            request_max_tokens(&model, &context, &SimpleStreamOptions::default()).unwrap(),
            4096
        );
    }

    #[test]
    fn hard_clamp_preserves_absent_and_explicit_requests_without_authority() {
        assert_eq!(clamp_max_tokens_to_hard_limit(None, None).unwrap(), None);
        for value in [0.0, -1.0, 2000.0, f64::INFINITY] {
            assert_eq!(
                clamp_max_tokens_to_hard_limit(Some(value), None).unwrap(),
                Some(value)
            );
        }
    }

    #[test]
    fn hard_clamp_caps_explicit_and_supplies_absent_requests() {
        for (requested, expected) in [
            (None, 4096.0),
            (Some(10000.0), 4096.0),
            (Some(2000.0), 2000.0),
        ] {
            assert_eq!(
                clamp_max_tokens_to_hard_limit(requested, Some(4096.0)).unwrap(),
                Some(expected)
            );
        }
    }

    #[test]
    fn invalid_authority_and_explicit_limits_name_the_limit() {
        for invalid in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(clamp_max_tokens_to_hard_limit(Some(100.0), Some(invalid))
                .unwrap_err()
                .contains("hardMaxOutputTokens"));
            assert!(clamp_max_tokens_to_hard_limit(Some(invalid), Some(4096.0))
                .unwrap_err()
                .contains("maxTokens"));
        }
    }

    #[test]
    fn payload_customization_cannot_widen_remove_or_mistype_authority() {
        for value in [Some(json!(50000)), Some(Value::Null), None] {
            let mut payload = Map::new();
            if let Some(value) = value {
                payload.insert("max_output_tokens".into(), value);
            }
            enforce_hard_max_tokens_in_record(&mut payload, "max_output_tokens", Some(4096.0))
                .unwrap();
            assert_eq!(payload["max_output_tokens"].as_f64(), Some(4096.0));
            assert_eq!(payload["max_output_tokens"].as_u64(), Some(4096));
        }
        for value in [json!("50000"), json!(true), json!({}), json!([])] {
            let mut payload = Map::new();
            payload.insert("max_output_tokens".into(), value);
            assert!(enforce_hard_max_tokens_in_record(
                &mut payload,
                "max_output_tokens",
                Some(4096.0)
            )
            .unwrap_err()
            .contains("max_output_tokens must be a number"));
        }
        let mut payload = Map::new();
        payload.insert("max_output_tokens".into(), json!(0));
        assert!(
            enforce_hard_max_tokens_in_record(&mut payload, "max_output_tokens", Some(4096.0))
                .is_err()
        );
        enforce_hard_max_tokens_in_record(&mut payload, "max_output_tokens", None).unwrap();
        assert_eq!(payload["max_output_tokens"], 0);
    }

    #[test]
    fn fractional_authority_reaches_the_wire_as_whole_tokens() {
        let mut payload = Map::new();
        payload.insert("max_tokens".into(), json!(50000));
        enforce_hard_max_tokens_in_record(&mut payload, "max_tokens", Some(4096.5)).unwrap();
        assert_eq!(payload["max_tokens"], json!(4096));

        let mut payload = Map::new();
        assert!(
            enforce_hard_max_tokens_in_record(&mut payload, "max_tokens", Some(0.5))
                .unwrap_err()
                .contains("at least one output token")
        );
    }

    #[test]
    fn context_clamp_reserves_safety_and_minimum_headroom() {
        let mut model = Model::new("test", "test", Api::Faux, "faux", "http://localhost:0");
        for (window, requested, expected) in [
            (0, 0, 1),
            (0, 2000, 2000),
            (8192, 10000, 4095),
            (4096, 10000, 1),
            (1, 10000, 1),
            (8192, 2000, 2000),
            (8192, 0, 0),
        ] {
            model.context_window = window;
            assert_eq!(
                clamp_max_tokens_to_context(&model, &[], Some("four"), requested),
                expected
            );
        }
    }
}
