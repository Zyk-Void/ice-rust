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

/// Text an extension adds to the model context for one prompt.
#[derive(Debug, Clone, PartialEq)]
pub struct PromptContext {
    /// Custom message type the text is stored under.
    pub custom_type: String,
    pub text: String,
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
