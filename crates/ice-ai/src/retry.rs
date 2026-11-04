//! Shared assistant-message retry classification and lifecycle policy.
//!
//! This is deliberately separate from provider HTTP retries: it classifies a
//! completed `AssistantMessage` and can retry the whole assistant operation.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use regex::Regex;
use tokio_util::sync::CancellationToken;

use crate::types::{AssistantMessage, StopReason};

/// Maximum timer-safe backoff supported by browser/JavaScript-compatible timers.
pub const MAX_RETRY_DELAY_MS: u64 = 2_147_483_647;
const DEFAULT_MAX_RETRY_DELAY_MS: u64 = 60_000;

/// Assistant-level retry settings. `max_retries` excludes the initial call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssistantRetryPolicy {
    pub enabled: bool,
    pub max_retries: u32,
    pub base_delay_ms: u64,
    /// Maximum individual backoff delay. `None` uses 60 seconds.
    pub max_delay_ms: Option<u64>,
}

impl Default for AssistantRetryPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            max_retries: 0,
            base_delay_ms: 1000,
            max_delay_ms: None,
        }
    }
}

/// Event passed to `on_retry_scheduled`, with one-based retry attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetryScheduled {
    pub attempt: u32,
    pub max_attempts: u32,
    pub delay_ms: u64,
    pub error_message: String,
}

/// Event passed to `on_retry_finished` after at least one retry was scheduled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetryFinished {
    pub success: bool,
    pub attempt: u32,
    pub final_error: Option<String>,
}

/// Error category identifying which awaited retry callback failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryCallbackKind {
    Scheduled,
    AttemptStart,
    Finished,
}

impl RetryCallbackKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Scheduled => "on_retry_scheduled",
            Self::AttemptStart => "on_retry_attempt_start",
            Self::Finished => "on_retry_finished",
        }
    }
}

/// Failure propagated from an awaited retry lifecycle callback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetryCallbackFailure {
    pub callback: RetryCallbackKind,
    pub message: String,
}

impl std::fmt::Display for RetryCallbackFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} failed: {}", self.callback.as_str(), self.message)
    }
}

impl std::error::Error for RetryCallbackFailure {}

/// Future type used by retry lifecycle callback closures.
pub type RetryCallbackFuture = Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'static>>;

type ScheduledCallback = Arc<dyn Fn(RetryScheduled) -> RetryCallbackFuture + Send + Sync>;
type AttemptStartCallback = Arc<dyn Fn() -> RetryCallbackFuture + Send + Sync>;
type FinishedCallback = Arc<dyn Fn(RetryFinished) -> RetryCallbackFuture + Send + Sync>;

/// Awaited callbacks emitted around each whole-assistant retry.
#[derive(Clone, Default)]
pub struct AssistantRetryCallbacks {
    on_retry_scheduled: Option<ScheduledCallback>,
    on_retry_attempt_start: Option<AttemptStartCallback>,
    on_retry_finished: Option<FinishedCallback>,
}

impl AssistantRetryCallbacks {
    pub fn on_retry_scheduled<F, Fut>(mut self, callback: F) -> Self
    where
        F: Fn(RetryScheduled) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), String>> + Send + 'static,
    {
        self.on_retry_scheduled = Some(Arc::new(move |event| Box::pin(callback(event))));
        self
    }

    pub fn on_retry_attempt_start<F, Fut>(mut self, callback: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), String>> + Send + 'static,
    {
        self.on_retry_attempt_start = Some(Arc::new(move || Box::pin(callback())));
        self
    }

    pub fn on_retry_finished<F, Fut>(mut self, callback: F) -> Self
    where
        F: Fn(RetryFinished) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), String>> + Send + 'static,
    {
        self.on_retry_finished = Some(Arc::new(move |event| Box::pin(callback(event))));
        self
    }
}

/// All controls for [`retry_assistant_call_with_options`]. This additive type
/// avoids changing existing harness `RetryPolicy` struct literals.
#[derive(Clone)]
pub struct AssistantRetryOptions {
    pub policy: Option<AssistantRetryPolicy>,
    pub signal: CancellationToken,
    pub callbacks: AssistantRetryCallbacks,
}

impl Default for AssistantRetryOptions {
    fn default() -> Self {
        Self {
            policy: None,
            signal: CancellationToken::new(),
            callbacks: AssistantRetryCallbacks::default(),
        }
    }
}

impl AssistantRetryOptions {
    pub fn new(policy: Option<AssistantRetryPolicy>, signal: CancellationToken) -> Self {
        Self {
            policy,
            signal,
            callbacks: AssistantRetryCallbacks::default(),
        }
    }

    pub fn with_callbacks(mut self, callbacks: AssistantRetryCallbacks) -> Self {
        self.callbacks = callbacks;
        self
    }
}

/// Compute a bounded exponential delay for a one-based retry attempt.
///
/// The exponent is tested/capped before shifting, so even attempt 2048 cannot
/// overflow. Delays are also capped at the maximum timer-safe millisecond value.
pub fn get_retry_delay_ms(base_delay_ms: u64, attempt: u32, max_delay_ms: Option<u64>) -> u64 {
    let cap = max_delay_ms
        .unwrap_or(DEFAULT_MAX_RETRY_DELAY_MS)
        .min(MAX_RETRY_DELAY_MS);
    if base_delay_ms == 0 || cap == 0 {
        return 0;
    }
    let exponent = attempt.saturating_sub(1);
    if exponent >= u64::BITS {
        return cap;
    }
    // Compare against the cap before shifting. `checked_shl` only guards the
    // shift amount, not value bits discarded by an in-range shift.
    if base_delay_ms > (cap >> exponent) {
        return cap;
    }
    base_delay_ms << exponent
}

/// Classify transient assistant errors using the source-reviewed AI-001
/// expressions. Account/quota/billing matches take precedence over transient
/// status text, so e.g. `429 quota exceeded` is not retried.
pub fn is_retryable_assistant_error(message: &AssistantMessage) -> bool {
    if message.stop_reason != StopReason::Error {
        return false;
    }
    let Some(error) = message.error_message.as_deref() else {
        return false;
    };

    static RETRYABLE: OnceLock<Regex> = OnceLock::new();
    if is_permanent_account_error(error) {
        return false;
    }
    let retryable = RETRYABLE.get_or_init(|| {
        Regex::new(concat!(
            "(?i)(overloaded|rate.?limit|too many requests|(?:^|\\D)(?:429|500|502|503|504|524)(?:\\D|$)|",
            "service.?unavailable|server.?error|internal.?error|provider.?returned.?error|",
            "network.?error|connection.?error|connection.?refused|connection.?lost|",
            "other side closed|fetch failed|getaddrinfo|ENOTFOUND|EAI_AGAIN|",
            "upstream.?connect|reset before headers|socket hang up|socket connection was closed|",
            "timed? out|timeout|terminated|websocket.?closed|websocket.?error|ended without|",
            "stream ended before message_stop|stream ended before a terminal response event|",
            "http2 request did not get a response|retry delay|you can retry your request|",
            "try your request again|please retry your request|ResourceExhausted)"
        ))
        .expect("static retryable assistant error pattern is valid")
    });
    retryable.is_match(error)
}

/// Conservative text fallback shared with transport retry. Numeric provider
/// codes alone are not globally meaningful and deliberately are not matched.
///
/// Usage-limit/quota-threshold errors are not retried when they state no reset
/// or a reset beyond the default backoff cap: an hours-long reset cannot be
/// reached by bounded retries, so retrying only delays the actionable error.
pub(crate) fn is_permanent_account_error(error: &str) -> bool {
    static NON_RETRYABLE: OnceLock<Regex> = OnceLock::new();
    let permanent = NON_RETRYABLE
        .get_or_init(|| {
            Regex::new(concat!(
                "(?i)(GoUsageLimitError|FreeUsageLimitError|Monthly usage limit reached|",
                "available balance|insufficient[ _-](?:quota|balance|credits?)|",
                "no resource package|out of budget|quota exceeded|billing)"
            ))
            .expect("static non-retryable account error pattern is valid")
        })
        .is_match(error);
    permanent
        || usage_limit_reset_ms(error)
            .is_some_and(|reset| reset.is_none_or(|ms| ms > DEFAULT_MAX_RETRY_DELAY_MS))
}

/// `None` when `error` is not a usage-limit/quota-threshold error; otherwise its
/// stated `reset after|in <N>h <N>m <N>s` delay, if one can be parsed.
fn usage_limit_reset_ms(error: &str) -> Option<Option<u64>> {
    static LIMIT: OnceLock<Regex> = OnceLock::new();
    static RESET: OnceLock<Regex> = OnceLock::new();
    static PART: OnceLock<Regex> = OnceLock::new();
    let limit = LIMIT.get_or_init(|| {
        Regex::new(
            r"(?i)usage[ _-]limit(?:[ _-]has[ _-]been|[ _-]was)?[ _-]reached|quota[ _-]threshold",
        )
        .expect("static usage-limit pattern is valid")
    });
    if !limit.is_match(error) {
        return None;
    }
    let reset = RESET.get_or_init(|| {
        Regex::new(r"(?i)\bresets?[ _-](?:after|in)[ _-]((?:\d+[ ]*[hms][ ]*)+)")
            .expect("static reset pattern is valid")
    });
    let part = PART
        .get_or_init(|| Regex::new(r"(?i)(\d+)[ ]*([hms])").expect("static reset part is valid"));
    Some(reset.captures(error).map(|caps| {
        part.captures_iter(&caps[1])
            .map(|unit| {
                let value: u64 = unit[1].parse().unwrap_or(u64::MAX);
                let scale = match unit[2].to_ascii_lowercase().as_str() {
                    "h" => 3_600_000,
                    "m" => 60_000,
                    _ => 1_000,
                };
                value.saturating_mul(scale)
            })
            .fold(0u64, u64::saturating_add)
    }))
}

/// Run an assistant-producing operation with transient-error retry and awaited
/// lifecycle hooks. Provider/request errors are represented by the producer's
/// returned assistant message; callback failures are propagated in order.
pub async fn retry_assistant_call_with_options<F, Fut>(
    produce: F,
    options: AssistantRetryOptions,
) -> Result<AssistantMessage, RetryCallbackFailure>
where
    F: Fn() -> Fut,
    Fut: Future<Output = AssistantMessage>,
{
    let policy = options.policy.filter(|policy| policy.enabled);
    let max_retries = policy.map(|policy| policy.max_retries).unwrap_or(0);
    let mut attempt = 0u32;
    let mut last_retry: Option<(u32, String)> = None;

    loop {
        let response = produce().await;
        if response.stop_reason == StopReason::Aborted {
            if let Some((attempt, _)) = last_retry {
                invoke_finished(
                    &options.callbacks,
                    RetryFinished {
                        success: false,
                        attempt,
                        final_error: None,
                    },
                )
                .await?;
            }
            return Ok(response);
        }
        if response.stop_reason != StopReason::Error {
            if let Some((attempt, _)) = last_retry {
                invoke_finished(
                    &options.callbacks,
                    RetryFinished {
                        success: true,
                        attempt,
                        final_error: None,
                    },
                )
                .await?;
            }
            return Ok(response);
        }
        if attempt >= max_retries || !is_retryable_assistant_error(&response) {
            if let Some((attempt, _)) = last_retry {
                invoke_finished(
                    &options.callbacks,
                    RetryFinished {
                        success: false,
                        attempt,
                        final_error: response.error_message.clone(),
                    },
                )
                .await?;
            }
            return Ok(response);
        }

        attempt += 1;
        let error_message = response
            .error_message
            .as_deref()
            .filter(|message| !message.is_empty())
            .unwrap_or("Unknown error")
            .to_string();
        let delay_ms = get_retry_delay_ms(
            policy.map(|policy| policy.base_delay_ms).unwrap_or(1000),
            attempt,
            policy.and_then(|policy| policy.max_delay_ms),
        );
        invoke_scheduled(
            &options.callbacks,
            RetryScheduled {
                attempt,
                max_attempts: max_retries,
                delay_ms,
                error_message: error_message.clone(),
            },
        )
        .await?;
        last_retry = Some((attempt, error_message));

        let delay = Duration::from_millis(delay_ms);
        let cancelled = tokio::select! {
            biased;
            _ = options.signal.cancelled() => true,
            _ = tokio::time::sleep(delay) => false,
        };
        if cancelled {
            let (last_attempt, last_error) = last_retry
                .as_ref()
                .map(|(attempt, error)| (*attempt, error.clone()))
                .expect("retry sleep follows a scheduled retry");
            invoke_finished(
                &options.callbacks,
                RetryFinished {
                    success: false,
                    attempt: last_attempt,
                    final_error: Some(last_error),
                },
            )
            .await?;
            let mut aborted = response;
            aborted.stop_reason = StopReason::Aborted;
            aborted.error_message = None;
            return Ok(aborted);
        }

        invoke_attempt_start(&options.callbacks).await?;
    }
}

async fn invoke_scheduled(
    callbacks: &AssistantRetryCallbacks,
    event: RetryScheduled,
) -> Result<(), RetryCallbackFailure> {
    if let Some(callback) = &callbacks.on_retry_scheduled {
        callback(event)
            .await
            .map_err(|message| RetryCallbackFailure {
                callback: RetryCallbackKind::Scheduled,
                message,
            })?;
    }
    Ok(())
}

async fn invoke_attempt_start(
    callbacks: &AssistantRetryCallbacks,
) -> Result<(), RetryCallbackFailure> {
    if let Some(callback) = &callbacks.on_retry_attempt_start {
        callback().await.map_err(|message| RetryCallbackFailure {
            callback: RetryCallbackKind::AttemptStart,
            message,
        })?;
    }
    Ok(())
}

async fn invoke_finished(
    callbacks: &AssistantRetryCallbacks,
    event: RetryFinished,
) -> Result<(), RetryCallbackFailure> {
    if let Some(callback) = &callbacks.on_retry_finished {
        callback(event)
            .await
            .map_err(|message| RetryCallbackFailure {
                callback: RetryCallbackKind::Finished,
                message,
            })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Api, AssistantMessage, Content, Usage};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::Mutex;

    fn error_message(text: &str) -> AssistantMessage {
        AssistantMessage::terminal(Api::Faux, "faux", "faux-1", StopReason::Error, text, 1)
    }

    #[test]
    fn classifier_matches_separator_variants_terminated_and_quota_precedence() {
        for text in [
            "rate_limit",
            "service-unavailable",
            "provider_returned_error",
            "terminated",
        ] {
            assert!(is_retryable_assistant_error(&error_message(text)), "{text}");
        }
        assert!(!is_retryable_assistant_error(&error_message(
            "429 quota exceeded"
        )));
        assert!(!is_retryable_assistant_error(&error_message(
            "billing server error 500"
        )));
    }

    #[tokio::test]
    async fn billing_failure_does_not_schedule_or_execute_retries() {
        let calls = Arc::new(AtomicUsize::new(0));
        let produce_calls = calls.clone();
        let result = retry_assistant_call_with_options(
            move || {
                produce_calls.fetch_add(1, Ordering::SeqCst);
                async {
                    error_message(
                        "HTTP 429: Insufficient balance or no resource package. Please recharge.",
                    )
                }
            },
            AssistantRetryOptions {
                policy: Some(AssistantRetryPolicy {
                    enabled: true,
                    max_retries: 5,
                    base_delay_ms: 0,
                    max_delay_ms: None,
                }),
                signal: CancellationToken::new(),
                callbacks: AssistantRetryCallbacks::default()
                    .on_retry_scheduled(|_| async { panic!("billing must not schedule a retry") }),
            },
        )
        .await
        .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(result.stop_reason, StopReason::Error);
        assert!(result.error_message.unwrap().contains("Please recharge"));
    }

    #[test]
    fn billing_balance_errors_are_not_transient_rate_limits() {
        for text in [
            r#"http error 429: {"error":{"code":"1113","message":"Insufficient balance or no resource package. Please recharge."}}"#,
            "429 insufficient balance",
            "HTTP 429 insufficient credits",
            "429 no resource package. Please recharge.",
        ] {
            assert!(
                !is_retryable_assistant_error(&error_message(text)),
                "{text}"
            );
        }
        assert!(is_retryable_assistant_error(&error_message(
            "HTTP 429 rate limit; retry after 2 seconds"
        )));
        assert!(is_retryable_assistant_error(&error_message(
            r#"HTTP 429 {"code":"1113","message":"rate limited"}"#
        )));
    }

    /// Shapes observed in local telemetry on 2026-11-04: a proxy and Codex
    /// both report quota exhaustion as HTTP 429 with an hours-long reset.
    #[test]
    fn usage_limits_with_distant_reset_are_not_retried() {
        for text in [
            "[codex/gpt-6-astra] [429]: The usage limit has been reached (reset after 2h 14s)",
            r#"http error 429: {"error":{"message":"[codex/gpt-6-astra] All codex accounts reached configured quota threshold (reset after 2h 12s)"}}"#,
            r#"http error 429: {"error":{"message":"[codex/gpt-6.1-sol] All codex accounts reached configured quota threshold (reset after 1h 59m 44s)"}}"#,
            "429: The usage limit has been reached (reset after 2h)",
            "429 usage limit has been reached",
            "429 quota threshold reached (resets in 5m)",
        ] {
            assert!(
                !is_retryable_assistant_error(&error_message(text)),
                "{text}"
            );
        }
        // A reset within the backoff cap is still worth one bounded retry.
        for text in [
            "[codex/gpt-6-astra] [429]: The usage limit has been reached (reset after 45s)",
            "429 quota threshold reached (reset after 1m)",
            // Unrelated reset wording is not a quota hint.
            "HTTP 503: connection reset after 2h idle",
        ] {
            assert!(is_retryable_assistant_error(&error_message(text)), "{text}");
        }
    }

    #[tokio::test]
    async fn distant_usage_limit_reset_executes_once() {
        let calls = Arc::new(AtomicUsize::new(0));
        let produce_calls = calls.clone();
        let result = retry_assistant_call_with_options(
            move || {
                produce_calls.fetch_add(1, Ordering::SeqCst);
                async {
                    error_message(
                        "[codex/gpt-6-astra] [429]: The usage limit has been reached (reset after 2h 14s)",
                    )
                }
            },
            AssistantRetryOptions {
                policy: Some(AssistantRetryPolicy {
                    enabled: true,
                    max_retries: 10,
                    base_delay_ms: 0,
                    max_delay_ms: None,
                }),
                signal: CancellationToken::new(),
                callbacks: AssistantRetryCallbacks::default().on_retry_scheduled(|_| async {
                    panic!("a distant quota reset must not schedule a retry")
                }),
            },
        )
        .await
        .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(result.error_message.unwrap().contains("reset after 2h 14s"));
    }

    #[test]
    fn status_codes_match_only_as_whole_numbers() {
        for text in ["HTTP 503", "status=502", "503"] {
            assert!(is_retryable_assistant_error(&error_message(text)), "{text}");
        }
        for text in ["invalid id 15003", "port 5000 refused value"] {
            assert!(
                !is_retryable_assistant_error(&error_message(text)),
                "{text}"
            );
        }
    }

    #[test]
    fn retry_delay_caps_before_large_shifts_and_timer_limit() {
        assert_eq!(get_retry_delay_ms(2000, 1, None), 2000);
        assert_eq!(get_retry_delay_ms(2000, 65, None), 60_000);
        assert_eq!(
            get_retry_delay_ms(2, 64, Some(u64::MAX)),
            MAX_RETRY_DELAY_MS
        );
        assert_eq!(
            get_retry_delay_ms(1, 2048, Some(u64::MAX)),
            MAX_RETRY_DELAY_MS
        );
        assert_eq!(get_retry_delay_ms(0, 2048, None), 0);
    }

    #[tokio::test]
    async fn awaited_callbacks_keep_lifecycle_order_and_propagate_failures() {
        let events = Arc::new(Mutex::new(Vec::<String>::new()));
        let callback_events = Arc::clone(&events);
        let callbacks = AssistantRetryCallbacks::default()
            .on_retry_scheduled(move |event| {
                let events = Arc::clone(&callback_events);
                async move {
                    events
                        .lock()
                        .await
                        .push(format!("scheduled:{}", event.attempt));
                    Ok(())
                }
            })
            .on_retry_attempt_start({
                let events = Arc::clone(&events);
                move || {
                    let events = Arc::clone(&events);
                    async move {
                        events.lock().await.push("attempt-start".into());
                        Ok(())
                    }
                }
            })
            .on_retry_finished({
                let events = Arc::clone(&events);
                move |event| {
                    let events = Arc::clone(&events);
                    async move {
                        events
                            .lock()
                            .await
                            .push(format!("finished:{}", event.success));
                        Ok(())
                    }
                }
            });
        let calls = Arc::new(AtomicUsize::new(0));
        let produce_calls = Arc::clone(&calls);
        let produce_events = Arc::clone(&events);
        let response = retry_assistant_call_with_options(
            move || {
                let calls = Arc::clone(&produce_calls);
                let events = Arc::clone(&produce_events);
                async move {
                    events.lock().await.push("produce".into());
                    if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                        error_message("terminated")
                    } else {
                        crate::providers::faux::faux_assistant_message("done", StopReason::Stop)
                    }
                }
            },
            AssistantRetryOptions {
                policy: Some(AssistantRetryPolicy {
                    enabled: true,
                    max_retries: 1,
                    base_delay_ms: 0,
                    max_delay_ms: None,
                }),
                signal: CancellationToken::new(),
                callbacks,
            },
        )
        .await
        .unwrap();
        assert_eq!(response.stop_reason, StopReason::Stop);
        assert_eq!(
            *events.lock().await,
            [
                "produce",
                "scheduled:1",
                "attempt-start",
                "produce",
                "finished:true"
            ]
        );

        let calls = Arc::new(AtomicUsize::new(0));
        let produce_calls = Arc::clone(&calls);
        let callbacks = AssistantRetryCallbacks::default()
            .on_retry_scheduled(|_| async { Err("scheduled callback failed".into()) });
        let error = retry_assistant_call_with_options(
            move || {
                let calls = Arc::clone(&produce_calls);
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    error_message("terminated")
                }
            },
            AssistantRetryOptions {
                policy: Some(AssistantRetryPolicy {
                    enabled: true,
                    max_retries: 1,
                    base_delay_ms: 0,
                    max_delay_ms: None,
                }),
                signal: CancellationToken::new(),
                callbacks,
            },
        )
        .await
        .unwrap_err();
        assert_eq!(error.callback, RetryCallbackKind::Scheduled);
        assert_eq!(error.message, "scheduled callback failed");
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        let calls = Arc::new(AtomicUsize::new(0));
        let produce_calls = Arc::clone(&calls);
        let callbacks = AssistantRetryCallbacks::default()
            .on_retry_attempt_start(|| async { Err("attempt callback failed".into()) });
        let error = retry_assistant_call_with_options(
            move || {
                let calls = Arc::clone(&produce_calls);
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    error_message("terminated")
                }
            },
            AssistantRetryOptions {
                policy: Some(AssistantRetryPolicy {
                    enabled: true,
                    max_retries: 1,
                    base_delay_ms: 0,
                    max_delay_ms: None,
                }),
                signal: CancellationToken::new(),
                callbacks,
            },
        )
        .await
        .unwrap_err();
        assert_eq!(error.callback, RetryCallbackKind::AttemptStart);
        assert_eq!(error.message, "attempt callback failed");
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        let calls = Arc::new(AtomicUsize::new(0));
        let produce_calls = Arc::clone(&calls);
        let callbacks = AssistantRetryCallbacks::default()
            .on_retry_finished(|_| async { Err("finished callback failed".into()) });
        let error = retry_assistant_call_with_options(
            move || {
                let calls = Arc::clone(&produce_calls);
                async move {
                    if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                        error_message("terminated")
                    } else {
                        crate::providers::faux::faux_assistant_message("done", StopReason::Stop)
                    }
                }
            },
            AssistantRetryOptions {
                policy: Some(AssistantRetryPolicy {
                    enabled: true,
                    max_retries: 1,
                    base_delay_ms: 0,
                    max_delay_ms: None,
                }),
                signal: CancellationToken::new(),
                callbacks,
            },
        )
        .await
        .unwrap_err();
        assert_eq!(error.callback, RetryCallbackKind::Finished);
        assert_eq!(error.message, "finished callback failed");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn cancelled_backoff_preserves_partial_content_and_usage_but_clears_error() {
        let signal = CancellationToken::new();
        signal.cancel();
        let mut partial = error_message("terminated");
        partial.content.push(Content::text("partial"));
        partial.usage = Usage {
            input: 17,
            ..Usage::zero()
        };
        let response = retry_assistant_call_with_options(
            move || {
                let partial = partial.clone();
                async move { partial }
            },
            AssistantRetryOptions {
                policy: Some(AssistantRetryPolicy {
                    enabled: true,
                    max_retries: 2,
                    base_delay_ms: 60_000,
                    max_delay_ms: None,
                }),
                signal,
                callbacks: AssistantRetryCallbacks::default(),
            },
        )
        .await
        .unwrap();
        assert_eq!(response.stop_reason, StopReason::Aborted);
        assert_eq!(response.error_message, None);
        assert_eq!(response.content, vec![Content::text("partial")]);
        assert_eq!(response.usage.input, 17);
    }
}
