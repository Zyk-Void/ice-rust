//! Blackhole: optional deterministic compaction for icers.
//!
//! A native port of the ICE `ice-blackhole` extension (MIT, © 2026 k0valik;
//! adapted from ice-blackhole 0.4.3 and sting8k/ice-vcc). It replaces the
//! LLM checkpoint with deterministic sections compiled from the conversation,
//! rebuilds volatile working state from durable session entries, and can add
//! a bounded Cognee recall section. It is selected by its own configuration
//! file and never starts a second agent loop. See `docs/blackhole.md`.

pub mod brief;
pub mod checkpoint_recall;
pub mod command;
pub mod config;
pub mod engine;
pub mod js;
pub mod live_state;
pub mod normalize;
pub mod refine;
pub mod sections;
pub mod summarize;
pub mod tail;
pub mod trim;

/// A lazily compiled, process-wide regex.
macro_rules! regex {
    ($pattern:expr) => {{
        static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        RE.get_or_init(|| regex::Regex::new($pattern).expect("valid regex"))
    }};
}
pub(crate) use regex;

/// Wall-clock milliseconds since the Unix epoch.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}
