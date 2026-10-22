//! Safety admission for ordinary assistant retries in a coding-session run.
//!
//! This guard runs before the shared transient-error classifier. It tracks
//! possible tool effects for only the currently streaming assistant generation,
//! so an already completed tool turn does not taint a later failed generation.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use futures::future::BoxFuture;
use ice_agent::{AgentEmitter, AgentEvent, AgentMessage};
use ice_ai::types::{AssistantMessage, Content, StopReason};

/// Tracks whether the current assistant generation may have caused a tool effect.
///
/// Agent events are delivered serially by the agent loop. An atomic is used so
/// the state can be shared between its emitter decorator and the retry decision
/// without introducing an operation lock or holding a guard across an await.
#[derive(Debug, Default)]
pub(crate) struct RetrySafetyState {
    possible_tool_effect: AtomicBool,
}

impl RetrySafetyState {
    pub(crate) fn possible_tool_effect(&self) -> bool {
        self.possible_tool_effect.load(Ordering::Acquire)
    }

    fn observe(&self, event: &AgentEvent) {
        match event {
            AgentEvent::AgentStart
            | AgentEvent::MessageStart {
                message: AgentMessage::Assistant(_),
            } => self.possible_tool_effect.store(false, Ordering::Release),
            AgentEvent::ToolExecutionStart { .. }
            | AgentEvent::ToolExecutionEnd { .. }
            | AgentEvent::MessageStart {
                message: AgentMessage::ToolResult(_),
            } => self.possible_tool_effect.store(true, Ordering::Release),
            _ => {}
        }
    }
}

/// Observe retry-safety events without changing their delivery or ordering.
///
/// This sits outside the existing settle/frame emitters. Its state update is
/// synchronous and it forwards each event exactly once to the established
/// durable observers and caller emitter.
pub(crate) struct RetrySafetyEmitter {
    inner: Arc<dyn AgentEmitter>,
    state: Arc<RetrySafetyState>,
}

impl RetrySafetyEmitter {
    pub(crate) fn new(inner: Arc<dyn AgentEmitter>, state: Arc<RetrySafetyState>) -> Self {
        Self { inner, state }
    }
}

impl AgentEmitter for RetrySafetyEmitter {
    fn emit(&self, event: AgentEvent) -> BoxFuture<'static, ()> {
        self.state.observe(&event);
        self.inner.emit(event)
    }

    fn try_emit(&self, event: AgentEvent) {
        self.state.observe(&event);
        self.inner.try_emit(event);
    }
}

/// Whether a failed assistant generation can be admitted to ordinary retry.
///
/// A tool call in the failed message is unsafe even if tool execution did not
/// start. Tool events/results also make the current generation unsafe because an
/// effect may have occurred before the response failed.
pub(crate) fn can_retry_failed_generation(
    assistant: &AssistantMessage,
    possible_tool_effect: bool,
) -> bool {
    if assistant.stop_reason != StopReason::Error || possible_tool_effect {
        return false;
    }
    if assistant
        .content
        .iter()
        .any(|block| matches!(block, Content::ToolCall(_)))
    {
        return false;
    }
    !is_context_overflow_error(assistant.error_message.as_deref().unwrap_or_default())
}

/// Conservative error-message context-overflow recognition for retry safety.
///
/// `docs/ice-gap/ai.md` §AI-007 documents the provider pattern families but not
/// their literal regexes. To avoid inventing a broader classifier, this guard
/// recognizes only these documented phrases (case-insensitively): `prompt is
/// too long`, `request too large`, `input too long`, `context window`,
/// `maximum length`, `context length exceeded`, `too many tokens`, and `token
/// limit`. Hyphens and underscores are normalized to spaces, covering the
/// source-named `prompt-is-too-long`, `request_too_large`,
/// `context_length_exceeded`, `too-many-tokens`, and `token-limit` spellings.
///
/// The documented exclusions take precedence: `throttling error`, `service
/// unavailable`, `rate limit`, and `too many requests`. This is intentionally
/// narrower than AI-007's complete table and does not infer silent overflow
/// from usage statistics; the ordinary retry path has no context-window input.
pub(crate) fn is_context_overflow_error(error: &str) -> bool {
    let normalized = error
        .to_ascii_lowercase()
        .replace('_', " ")
        .replace('-', " ");

    const EXCLUSIONS: &[&str] = &[
        "throttling error",
        "service unavailable",
        "rate limit",
        "too many requests",
    ];
    if EXCLUSIONS
        .iter()
        .any(|pattern| normalized.contains(pattern))
    {
        return false;
    }

    const OVERFLOW_PHRASES: &[&str] = &[
        "prompt is too long",
        "request too large",
        "input too long",
        "context window",
        "maximum length",
        "context length exceeded",
        "too many tokens",
        "token limit",
    ];
    OVERFLOW_PHRASES
        .iter()
        .any(|pattern| normalized.contains(pattern))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ice_agent::AgentToolResult;
    use ice_ai::providers::faux::{faux_assistant_message, FauxBlock};
    use ice_ai::types::StopReason;

    fn assistant(
        content: impl Into<ice_ai::providers::faux::FauxContent>,
        error: &str,
    ) -> AssistantMessage {
        let mut assistant = faux_assistant_message(content, StopReason::Error);
        assistant.error_message = Some(error.to_string());
        assistant
    }

    #[test]
    fn assistant_tool_call_and_context_overflow_are_not_retryable() {
        let partial_call = assistant(
            vec![FauxBlock::tool_call(
                "write",
                serde_json::json!({"path":"x"}),
            )],
            "503 service unavailable",
        );
        assert!(!can_retry_failed_generation(&partial_call, false));

        let overflow = assistant("", "503 request_too_large");
        assert!(!can_retry_failed_generation(&overflow, false));

        let excluded_transient = assistant("", "503 service unavailable; request_too_large");
        assert!(can_retry_failed_generation(&excluded_transient, false));
    }

    #[test]
    fn event_state_resets_per_assistant_generation_and_tracks_all_tool_signals() {
        let state = RetrySafetyState::default();
        state.observe(&AgentEvent::AgentStart);
        assert!(!state.possible_tool_effect());

        state.observe(&AgentEvent::ToolExecutionStart {
            tool_call_id: "call".into(),
            tool_name: "write".into(),
            args: serde_json::Value::Null,
        });
        assert!(state.possible_tool_effect());

        state.observe(&AgentEvent::MessageStart {
            message: AgentMessage::Assistant(Box::new(faux_assistant_message(
                "next generation",
                StopReason::Stop,
            ))),
        });
        assert!(!state.possible_tool_effect());

        state.observe(&AgentEvent::ToolExecutionEnd {
            tool_call_id: "call".into(),
            tool_name: "write".into(),
            result: AgentToolResult::text("written"),
            is_error: false,
        });
        assert!(state.possible_tool_effect());

        state.observe(&AgentEvent::MessageStart {
            message: AgentMessage::ToolResult(Box::new(ice_ai::types::ToolResultMessage {
                role: ice_ai::types::ToolResultRole,
                tool_call_id: "call".into(),
                tool_name: "write".into(),
                content: vec![],
                details: None,
                usage: None,
                added_tool_names: vec![],
                is_error: false,
                timestamp: 0,
            })),
        });
        assert!(state.possible_tool_effect());

        state.observe(&AgentEvent::AgentStart);
        assert!(!state.possible_tool_effect());
    }
}
