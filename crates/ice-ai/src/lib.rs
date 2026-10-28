//! Mirrors `packages/ai/src` — the unified, multi-provider LLM layer.
//!
//! Provider-agnostic types (`Content`, `Message`, `AssistantMessage`,
//! `AssistantMessageEvent`, `Context`, `Tool`, `Usage`) live in [`types`];
//! model-layer configuration (`Model`, compat structs) lives in [`model`].
//! Schema validation + LLM-sloppy-JSON coercion (mirroring `validation.ts`) is
//! in [`schema`]; the SPSC streaming queue mirroring `event-stream.ts` is in
//! [`event_stream`]; the `Provider` trait seam is in [`provider`]; concrete
//! providers live under [`providers`] (faux now, anthropic in M3).
//!
//! Every module's doc comment names the TS file it mirrors.

#![forbid(unsafe_code)]

pub mod constrained;
pub mod decision;
pub mod error;
pub mod event_stream;
pub mod frames;
#[cfg(feature = "providers")]
pub mod http;
#[cfg(feature = "providers")]
pub mod images;
pub mod json_parse;
pub mod model;
pub mod provider;
pub mod providers;
pub mod retry;
pub mod schema;
pub mod simple_options;
// Strict wire-schema conversion is used only by the HTTP provider adapters.
#[cfg(feature = "providers")]
pub(crate) mod strict_schema;
pub mod types;

// Flat re-exports — consumers do `use ice_ai::Message` not `ice_ai::types::Message`.
pub use error::AiError;
pub use event_stream::{AssistantMessageEventStream, AssistantMessageEventStreamProducer};
pub use model::{Model, StreamingProtocolCompat, WireProtocol};
pub use provider::{
    CacheRetention, ContextPatch, NoopProviderHooks, Provider, ProviderHooks, SimpleStreamOptions,
    SimpleStreamOptionsPatch, DEFAULT_LLM_API_TIMEOUT,
};
pub use retry::{
    get_retry_delay_ms, is_retryable_assistant_error, retry_assistant_call_with_options,
    AssistantRetryCallbacks, AssistantRetryOptions, AssistantRetryPolicy, RetryCallbackFailure,
    RetryCallbackFuture, RetryCallbackKind, RetryFinished, RetryScheduled, MAX_RETRY_DELAY_MS,
};
pub use schema::validate_tool_arguments;
pub use types::{
    Api, AssistantMessage, AssistantMessageEvent, Content, Context, DeferredHandle, DoneReason,
    ErrorReason, ImageContent, InputModality, Message, ModelCost, ModelCostRates, ModelCostTier,
    ProviderId, Schema, StopReason, TextContent, ThinkingBudgets, ThinkingContent, ThinkingLevel,
    ThinkingLevelMap, Tool, ToolCall, ToolCallType, ToolResultMessage, Usage, UsageCost,
    UserContent, UserMessage,
};
