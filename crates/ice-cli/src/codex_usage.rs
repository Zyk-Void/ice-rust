//! Real ChatGPT subscription usage for `/usage`.
//!
//! `GET {base}/wham/usage` reports the signed-in account's plan, rate-limit
//! windows (percent used and time until reset), extra metered limits and credit
//! balance. Shape reference: the official Codex client's
//! `RateLimitStatusPayload`.

use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::codex_backend;

/// One rate-limit window, e.g. the 5-hour or weekly allowance.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct UsageWindow {
    pub(crate) used_percent: f64,
    pub(crate) window_seconds: u64,
    pub(crate) resets_in_seconds: u64,
}

/// A named limit: the main Codex limit or an additional metered feature.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct UsageLimit {
    pub(crate) name: String,
    pub(crate) limit_reached: bool,
    pub(crate) windows: Vec<UsageWindow>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct UsageCredits {
    pub(crate) unlimited: bool,
    pub(crate) balance: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ChatGptUsage {
    pub(crate) plan: String,
    pub(crate) limits: Vec<UsageLimit>,
    pub(crate) credits: Option<UsageCredits>,
}

/// A JSON number, or a numeric string, as `f64`. The backend is inconsistent
/// about both, and an explicit `null` must read as absent.
fn number(value: &Value, key: &str) -> Option<f64> {
    match value.get(key)? {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

fn seconds(value: &Value, key: &str) -> u64 {
    number(value, key).map_or(0, |seconds| seconds.max(0.0) as u64)
}

fn window(value: &Value) -> Option<UsageWindow> {
    Some(UsageWindow {
        used_percent: number(value, "used_percent")?.clamp(0.0, 100.0),
        window_seconds: seconds(value, "limit_window_seconds"),
        resets_in_seconds: seconds(value, "reset_after_seconds"),
    })
}

fn limit(details: &Value, name: &str) -> Option<UsageLimit> {
    details.is_object().then(|| UsageLimit {
        name: name.to_string(),
        limit_reached: details
            .get("limit_reached")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        windows: ["primary_window", "secondary_window"]
            .iter()
            .filter_map(|key| window(details.get(key)?))
            .collect(),
    })
}

fn credits(value: &Value) -> Option<UsageCredits> {
    let flag = |key| value.get(key).and_then(Value::as_bool).unwrap_or(false);
    let unlimited = flag("unlimited");
    (flag("has_credits") || unlimited).then(|| UsageCredits {
        unlimited,
        balance: match value.get("balance") {
            Some(Value::String(text)) => Some(text.clone()),
            Some(Value::Number(number)) => Some(number.to_string()),
            _ => None,
        },
    })
}

fn parse(body: &[u8]) -> Result<ChatGptUsage, String> {
    let payload: Value = serde_json::from_slice(body)
        .map_err(|_| "Unrecognized usage response (not JSON)".to_string())?;
    if !payload.is_object() {
        let kind = match payload {
            Value::Array(_) => "an array",
            Value::String(_) => "a string",
            Value::Number(_) => "a number",
            Value::Bool(_) => "a boolean",
            _ => "null",
        };
        return Err(format!("Unrecognized usage response (JSON is {kind})"));
    }
    let mut limits: Vec<UsageLimit> = payload
        .get("rate_limit")
        .and_then(|details| limit(details, "Codex"))
        .into_iter()
        .collect();
    for extra in payload
        .get("additional_rate_limits")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let name = extra
            .get("limit_name")
            .and_then(Value::as_str)
            .unwrap_or("Additional");
        limits.extend(
            extra
                .get("rate_limit")
                .and_then(|details| limit(details, name)),
        );
    }
    Ok(ChatGptUsage {
        plan: payload
            .get("plan_type")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string(),
        limits,
        credits: payload.get("credits").and_then(credits),
    })
}

/// Fetch the signed-in account's usage. `Ok(None)` means no ChatGPT login.
pub(crate) async fn fetch(
    base_url: &str,
    cancellation: &CancellationToken,
) -> Result<Option<ChatGptUsage>, String> {
    let Some(credential) = codex_backend::signed_in(cancellation).await? else {
        return Ok(None);
    };
    let url = codex_backend::codex_url(base_url, "wham/usage")?;
    let body = codex_backend::get(url, &credential, cancellation).await?;
    parse(&body).map(Some)
}

#[cfg(test)]
mod tests;
