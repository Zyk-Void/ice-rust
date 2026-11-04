//! In-process harness extensions.
//!
//! An extension observes and adjusts one run without a second agent loop. It
//! can add transient system-prompt text for a run, replace or cancel a
//! compaction before the summarizer runs, observe the persisted checkpoint,
//! request a compaction at a tool-turn boundary, and rewrite the model
//! context (never the session). Mirrors the subset of the TypeScript
//! extension events used by the ICE Cognee and Blackhole extensions:
//! `before_agent_start`, `session_before_compact`, `session_compact`,
//! `turn_end` + `compactAfterTurn`, `agent_start` and `context`.
//!
//! Extensions run in registration order. For `before_compact` the first
//! extension that returns a decision wins and later ones are not consulted.

use std::sync::Arc;

use ice_agent::AgentMessage;
use ice_ai::{Model, Provider};
use tokio_util::sync::CancellationToken;

use crate::compaction::CompactionPreparation;
use crate::session::types::{CompactionEntry, Entry};

/// Why a compaction is running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactionTrigger {
    /// An explicit `/compact` or `compact()` call.
    Manual,
    /// The pre-run usage threshold.
    Threshold,
    /// A tool-turn boundary inside a run.
    MidRun,
    /// Recovery after the provider rejected the request as too large.
    Overflow,
}

impl CompactionTrigger {
    /// The TypeScript `reason` string (`manual`, `threshold`, `overflow`).
    pub fn reason(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Threshold | Self::MidRun => "threshold",
            Self::Overflow => "overflow",
        }
    }
}

/// Inputs for a run's transient system prompt (`before_agent_start`).
pub struct BeforeRunEvent<'a> {
    /// Text of the newest user prompt in this run.
    pub prompt: &'a str,
    /// The composed system prompt the run would use.
    pub system_prompt: Option<&'a str>,
    pub session_id: &'a str,
    pub signal: CancellationToken,
}

/// Context at an already-needed provider request boundary.
pub struct RequestContextEvent<'a> {
    pub prompt: &'a str,
    pub original_prompt: Option<&'a str>,
    pub messages: &'a [AgentMessage],
    pub system_prompt: &'a str,
    pub session_id: &'a str,
    pub run_id: &'a str,
    pub lane_id: &'a str,
    pub branch_generation: u64,
    pub request_index: u64,
    pub context_window: u64,
    pub remaining_context_tokens: usize,
    pub latest_compaction_at: Option<i64>,
    pub signal: CancellationToken,
}

/// Text an extension adds to the model context for one prompt.
#[derive(Debug, Clone, PartialEq)]
pub struct PromptContext {
    /// Custom message type the text is stored under.
    pub custom_type: String,
    pub text: String,
    /// Opaque extension-owned correlation ID, never persisted with the text.
    pub correlation_id: Option<String>,
}

/// Result of admitting a context packet at a provider request boundary.
/// Persistence is not proof of provider delivery or useful completion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PromptContextOutcome {
    Persisted,
    Duplicate,
    Empty,
    BudgetRejected,
    Cancelled,
    Stale,
    AppendFailed,
    /// Legacy boolean rejection has no reliable cause attached.
    RejectedUnknown,
}

impl PromptContextOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Persisted => "persisted",
            Self::Duplicate => "duplicate",
            Self::Empty => "empty",
            Self::BudgetRejected => "budget_rejected",
            Self::Cancelled => "cancelled",
            Self::Stale => "stale",
            Self::AppendFailed => "append_failed",
            Self::RejectedUnknown => "rejected_unknown",
        }
    }
}

/// Everything known before the summarizer runs (`session_before_compact`).
#[derive(Clone)]
pub struct BeforeCompactEvent {
    pub preparation: CompactionPreparation,
    /// The durable branch, oldest first.
    pub branch_entries: Vec<Entry>,
    pub custom_instructions: Option<String>,
    pub trigger: CompactionTrigger,
    /// The run retries its provider request after this compaction.
    pub will_retry: bool,
    pub signal: CancellationToken,
    pub session_id: String,
    /// The active model and the provider that serves it, if resolvable.
    pub model: Model,
    pub provider: Option<Arc<dyn Provider>>,
    /// Every configured provider, for extensions that pick another model.
    pub providers: Vec<Arc<dyn Provider>>,
}

/// An extension-supplied checkpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct CompactionOverride {
    pub summary: String,
    /// `None` keeps the host's retained tail; `Some(vec![])` keeps none.
    pub retained_tail: Option<Vec<AgentMessage>>,
    /// Stored as the compaction entry's `details`.
    pub details: Option<serde_json::Value>,
}

/// An extension's answer to [`HarnessExtension::before_compact`].
#[derive(Debug, Clone, PartialEq)]
pub enum BeforeCompactDecision {
    /// Skip this compaction.
    Cancel,
    /// Persist this checkpoint instead of asking the summarizer.
    Replace(CompactionOverride),
}

/// A persisted checkpoint (`session_compact`).
pub struct AfterCompactEvent<'a> {
    pub entry: &'a CompactionEntry,
    pub trigger: CompactionTrigger,
    /// An extension supplied the summary.
    pub from_extension: bool,
    pub session_id: &'a str,
}

/// Usage at a completed tool turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoundaryEvent {
    /// Estimated input tokens of the next request.
    pub input_tokens: i64,
    /// The active model's context window (0 when unknown).
    pub context_window: u64,
}

/// An extension's answer at a tool-turn boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BoundaryDecision {
    /// Use the harness's own mid-run policy.
    #[default]
    Default,
    /// Compact now; `pause` stops the run after the checkpoint.
    Compact { pause: bool },
}

#[async_trait::async_trait]
pub trait HarnessExtension: Send + Sync {
    /// A run is starting (`agent_start`).
    async fn run_start(&self) {}

    /// Replacement system prompt for this run only. Returning `None` keeps the
    /// composed prompt. Nothing is persisted.
    async fn before_run(&self, _event: &BeforeRunEvent<'_>) -> Option<String> {
        None
    }

    /// Context for this run's prompt (e.g. recalled memory). It is stored
    /// right after the prompt as a hidden custom message, so later requests
    /// resend it unchanged: the history stays a stable prefix the provider can
    /// cache, which a per-run system prompt change would break every turn.
    async fn prompt_context(&self, _event: &BeforeRunEvent<'_>) -> Option<PromptContext> {
        None
    }

    /// Context for an already-needed provider request. By default, bridge only
    /// the first request to the legacy callback.
    async fn request_context(&self, event: &RequestContextEvent<'_>) -> Option<PromptContext> {
        if event.request_index != 0 {
            return None;
        }
        self.prompt_context(&BeforeRunEvent {
            prompt: event.prompt,
            system_prompt: Some(event.system_prompt),
            session_id: event.session_id,
            signal: event.signal.clone(),
        })
        .await
    }

    /// Called after the harness attempts to persist returned prompt context.
    /// `persisted` means it was appended to session history and the initial
    /// run context; it does not guarantee delivery to a model provider.
    async fn prompt_context_persisted(&self, _context: &PromptContext, _persisted: bool) {}

    /// Typed acknowledgment; the default preserves the legacy boolean callback.
    /// New extensions should use this hook to distinguish admission rejection
    /// from append I/O failure. Exactly one acknowledgment is made per packet.
    async fn prompt_context_acknowledged(
        &self,
        context: &PromptContext,
        outcome: PromptContextOutcome,
    ) {
        self.prompt_context_persisted(context, outcome == PromptContextOutcome::Persisted)
            .await;
    }

    async fn before_compact(&self, _event: &BeforeCompactEvent) -> Option<BeforeCompactDecision> {
        None
    }

    async fn after_compact(&self, _event: &AfterCompactEvent<'_>) {}

    async fn boundary(&self, _event: &BoundaryEvent) -> BoundaryDecision {
        BoundaryDecision::Default
    }

    /// Rewrite the model context before a request. `latest_compaction_at` is
    /// the timestamp of the newest compaction entry on the branch. Returning
    /// `None` leaves the messages unchanged; the session is never modified.
    fn transform_context(
        &self,
        _messages: &[AgentMessage],
        _latest_compaction_at: Option<i64>,
    ) -> Option<Vec<AgentMessage>> {
        None
    }
}

/// An ordered list of extensions.
pub type HarnessExtensions = Vec<Arc<dyn HarnessExtension>>;

#[cfg(test)]
mod acknowledgment_tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Legacy(Mutex<Vec<bool>>);
    #[async_trait::async_trait]
    impl HarnessExtension for Legacy {
        async fn prompt_context_persisted(&self, _: &PromptContext, persisted: bool) {
            self.0.lock().unwrap().push(persisted);
        }
    }

    #[tokio::test]
    async fn typed_outcomes_bridge_once_to_legacy_boolean_without_claiming_delivery() {
        let extension = Legacy::default();
        let context = PromptContext {
            custom_type: "fixture".into(),
            text: "safe".into(),
            correlation_id: None,
        };
        for outcome in [
            PromptContextOutcome::Persisted,
            PromptContextOutcome::Duplicate,
            PromptContextOutcome::Empty,
            PromptContextOutcome::BudgetRejected,
            PromptContextOutcome::Cancelled,
            PromptContextOutcome::Stale,
            PromptContextOutcome::AppendFailed,
            PromptContextOutcome::RejectedUnknown,
        ] {
            extension
                .prompt_context_acknowledged(&context, outcome)
                .await;
        }
        assert_eq!(
            *extension.0.lock().unwrap(),
            [true, false, false, false, false, false, false, false]
        );
    }
}
