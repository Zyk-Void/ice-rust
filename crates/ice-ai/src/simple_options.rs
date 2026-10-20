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
        // Native adapters use integer token fields. Match JSON.stringify's
        // integral-number encoding rather than sending e.g. 4096.0 to APIs
        // that require integer wire values; preserve fractional pure clamps.
        let value = if limit.fract() == 0.0 && limit <= u64::MAX as f64 {
            Value::from(limit as u64)
        } else {
            Value::from(limit)
        };
        payload.insert(field.into(), value);
    }
    Ok(())
}

/// Existing native context estimate (estimation parity is a separate gap).
pub fn estimate_context_tokens(messages: &[Message], system_prompt: Option<&str>) -> u64 {
    fn block_chars(blocks: &[Content], include_thinking: bool) -> u64 {
        blocks
            .iter()
            .map(|block| match block {
                Content::Text(t) => t.text.chars().count() as u64,
                Content::Thinking(t) if include_thinking => t.thinking.chars().count() as u64,
                Content::Image(_) => 4800,
                _ => 0,
            })
            .sum()
    }
    let mut chars = system_prompt.map_or(0, |s| s.chars().count() as u64);
    for message in messages {
        chars += match message {
            Message::User(user) => match &user.content {
                UserContent::Text(text) => text.chars().count() as u64,
                UserContent::Blocks(blocks) => block_chars(blocks, false),
            },
            Message::Assistant(assistant) => block_chars(&assistant.content, true),
            Message::ToolResult(result) => block_chars(&result.content, false),
        };
    }
    (chars + 3) / 4
}

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
    let requested = clamp_max_tokens_to_context(
        model,
        &context.messages,
        context.system_prompt.as_deref(),
        options.max_tokens.unwrap_or(model.max_tokens),
    );
    Ok(
        clamp_max_tokens_to_hard_limit(Some(requested as f64), options.hard_max_output_tokens)?
            .unwrap_or(requested as f64) as u64,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Api;
    use serde_json::json;

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
