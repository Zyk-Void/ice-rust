//! Mirrors `packages/agent/src/harness/compaction/compaction.ts:246-250`
//! (`shouldCompact`) and the `CompactionSettings`/`DEFAULT_COMPACTION_SETTINGS`
//! re-export.
//!
//! The struct + constant themselves live in [`crate::types`] (declared there so
//! `AgentHarnessOptions`, which predates this module, can carry the field
//! without a forward dependency). This module owns only the threshold check and
//! re-exports the struct for a stable `ice_harness::compaction::settings` path.

// Re-export so `super::mod` can surface them as `compaction::CompactionSettings`.
pub use crate::types::{CompactionSettings, DEFAULT_COMPACTION_SETTINGS};

/// Default percentage trigger for the coding-agent product policy.
pub const DEFAULT_CODING_AGENT_COMPACTION_THRESHOLD_PERCENT: f64 = 85.0;
/// Default summary-response reserve when no model is available for sizing.
pub const DEFAULT_CODING_AGENT_RESERVE_TOKENS: i64 = 16_384;
/// Default retained context size for coding-agent compaction.
pub const DEFAULT_CODING_AGENT_KEEP_RECENT_TOKENS: i64 = 20_000;

/// Turn-boundary handling selected by the coding-agent product policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MidRunCompactionMode {
    /// Do not compact at a turn boundary unless the 95% safety net is crossed.
    Off,
    /// Compact at a turn boundary and pause before continuing the run.
    Pause,
    /// Compact at a turn boundary and resume the run after the checkpoint.
    Resume,
}

impl Default for MidRunCompactionMode {
    fn default() -> Self {
        Self::Off
    }
}

/// Coding-agent automatic compaction policy.
///
/// This policy is intentionally separate from [`CompactionSettings`], whose
/// generic harness trigger is `context_tokens > context_window - reserve_tokens`.
/// The coding-agent product trigger is percentage-based; `reserve_tokens` is
/// only the summary response budget. `None` means no explicit reserve was
/// configured, and allows [`Self::resolve_reserve_tokens`] to adapt it to each
/// model. In the absence of model information that resolver uses the documented
/// 16,384-token default.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CodingAgentCompactionPolicy {
    /// Whether product-level automatic compaction is enabled.
    pub enabled: bool,
    /// Percentage trigger; finite positive values are clamped to 1..=99 and
    /// invalid values use the 85% default when evaluating the policy.
    pub threshold_percent: f64,
    /// Explicit summary reserve, if set. `None` preserves model-aware sizing.
    pub reserve_tokens: Option<i64>,
    /// Amount of recent context to retain when preparing a product checkpoint.
    pub keep_recent_tokens: i64,
    /// Behavior for compaction between completed turns.
    pub mid_run_compaction: MidRunCompactionMode,
}

impl Default for CodingAgentCompactionPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            threshold_percent: DEFAULT_CODING_AGENT_COMPACTION_THRESHOLD_PERCENT,
            reserve_tokens: None,
            keep_recent_tokens: DEFAULT_CODING_AGENT_KEEP_RECENT_TOKENS,
            mid_run_compaction: MidRunCompactionMode::Off,
        }
    }
}

impl CodingAgentCompactionPolicy {
    /// Return the effective summary reserve for a selected model.
    ///
    /// An explicit setting is preserved verbatim, including an explicit value
    /// of 16,384. Otherwise the product uses `min(model.max_tokens,
    /// floor(model.context_window / 2))`. If model metadata is unavailable, it
    /// falls back to the documented 16,384-token default.
    pub fn resolve_reserve_tokens(
        &self,
        model_max_tokens: Option<u64>,
        context_window: Option<u64>,
    ) -> i64 {
        if let Some(reserve_tokens) = self.reserve_tokens {
            return reserve_tokens;
        }
        match (model_max_tokens, context_window) {
            (Some(max_tokens), Some(window)) => {
                max_tokens.min(window / 2).min(i64::MAX as u64) as i64
            }
            _ => DEFAULT_CODING_AGENT_RESERVE_TOKENS,
        }
    }

    /// Return the normalized percentage used by the product trigger.
    pub fn normalized_threshold_percent(&self) -> f64 {
        normalize_threshold_percent(self.threshold_percent)
    }
}

fn normalize_threshold_percent(threshold_percent: f64) -> f64 {
    if !threshold_percent.is_finite() || threshold_percent <= 0.0 {
        DEFAULT_CODING_AGENT_COMPACTION_THRESHOLD_PERCENT
    } else {
        threshold_percent.clamp(1.0, 99.0)
    }
}

/// Return whether coding-agent product compaction is due at the configured
/// percentage. This is separate from [`should_compact`]: the product policy
/// requires a positive context window and strictly exceeds the floored
/// percentage threshold.
pub fn coding_agent_should_compact(
    context_tokens: i64,
    context_window: i64,
    policy: &CodingAgentCompactionPolicy,
) -> bool {
    if !policy.enabled || context_window <= 0 {
        return false;
    }
    let threshold =
        ((context_window as f64) * policy.normalized_threshold_percent() / 100.0).floor() as i64;
    context_tokens > threshold
}

/// Return whether usage strictly exceeds the coding-agent 95% mid-run safety
/// ceiling. Disabled policy and non-positive windows never trigger it.
pub fn is_past_mid_run_safety_net(
    context_tokens: i64,
    context_window: i64,
    policy: &CodingAgentCompactionPolicy,
) -> bool {
    if !policy.enabled || context_window <= 0 {
        return false;
    }
    let threshold = ((context_window as f64) * 95.0 / 100.0).floor() as i64;
    context_tokens > threshold
}

/// Return whether context usage exceeds the configured compaction threshold.
/// Mirrors TS `shouldCompact(contextTokens, contextWindow, settings)`:
/// `if (!settings.enabled) return false; return contextTokens > contextWindow -
/// settings.reserveTokens;`.
///
/// `context_tokens` and `context_window` are `i64` (token counts). The
/// subtraction is plain (NOT saturating): when `reserve_tokens > context_window`
/// the threshold goes negative, so ANY `context_tokens >= 0` trips compaction —
/// exactly the TS behavior. Realistic token counts (millions) cannot overflow
/// `i64`, so plain subtraction is safe.
pub fn should_compact(
    context_tokens: i64,
    context_window: i64,
    settings: &CompactionSettings,
) -> bool {
    if !settings.enabled {
        return false;
    }
    let threshold = context_window - settings.reserve_tokens;
    context_tokens > threshold
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_never_compacts() {
        let s = CompactionSettings {
            enabled: false,
            reserve_tokens: 1000,
            keep_recent_tokens: 500,
        };
        assert!(!should_compact(9_999_999, 10_000, &s));
    }

    #[test]
    fn trips_above_threshold() {
        let s = CompactionSettings {
            enabled: true,
            reserve_tokens: 1000,
            keep_recent_tokens: 500,
        };
        // window 10000 - reserve 1000 = 9000 threshold.
        assert!(!should_compact(9000, 10000, &s));
        assert!(should_compact(9001, 10000, &s));
        assert!(should_compact(99999, 10000, &s));
    }

    #[test]
    fn reserve_exceeding_window_trips_on_any_usage() {
        // Mirrors TS: threshold = contextWindow - reserve = 10000 - 20000 = -10000,
        // and `contextTokens > -10000` is true for every non-negative context_tokens,
        // so compaction trips even at zero usage.
        let s = CompactionSettings {
            enabled: true,
            reserve_tokens: 20_000,
            keep_recent_tokens: 500,
        };
        assert!(should_compact(0, 10_000, &s));
        assert!(should_compact(1, 10_000, &s));
    }

    #[test]
    fn defaults_match_ts() {
        assert_eq!(DEFAULT_COMPACTION_SETTINGS.enabled, true);
        assert_eq!(DEFAULT_COMPACTION_SETTINGS.reserve_tokens, 16384);
        assert_eq!(DEFAULT_COMPACTION_SETTINGS.keep_recent_tokens, 20000);
    }

    #[test]
    fn coding_agent_policy_has_product_defaults() {
        let policy = CodingAgentCompactionPolicy::default();
        assert!(policy.enabled);
        assert_eq!(policy.threshold_percent, 85.0);
        assert_eq!(policy.reserve_tokens, None);
        assert_eq!(
            policy.resolve_reserve_tokens(None, None),
            DEFAULT_CODING_AGENT_RESERVE_TOKENS
        );
        assert_eq!(policy.keep_recent_tokens, 20_000);
        assert_eq!(policy.mid_run_compaction, MidRunCompactionMode::Off);
    }

    #[test]
    fn coding_agent_percentage_threshold_is_strictly_greater_than_floor() {
        let policy = CodingAgentCompactionPolicy::default();
        // floor(10,000 * 85 / 100) = 8,500.
        assert!(!coding_agent_should_compact(8_500, 10_000, &policy));
        assert!(coding_agent_should_compact(8_501, 10_000, &policy));
    }

    #[test]
    fn invalid_coding_agent_thresholds_use_default_and_valid_values_clamp() {
        for invalid in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let policy = CodingAgentCompactionPolicy {
                threshold_percent: invalid,
                ..CodingAgentCompactionPolicy::default()
            };
            assert_eq!(policy.normalized_threshold_percent(), 85.0);
            assert!(!coding_agent_should_compact(8_500, 10_000, &policy));
            assert!(coding_agent_should_compact(8_501, 10_000, &policy));
        }

        let below_range = CodingAgentCompactionPolicy {
            threshold_percent: 0.5,
            ..CodingAgentCompactionPolicy::default()
        };
        assert_eq!(below_range.normalized_threshold_percent(), 1.0);
        let above_range = CodingAgentCompactionPolicy {
            threshold_percent: 100.0,
            ..CodingAgentCompactionPolicy::default()
        };
        assert_eq!(above_range.normalized_threshold_percent(), 99.0);
    }

    #[test]
    fn coding_agent_safety_net_uses_strict_95_percent_boundary() {
        let policy = CodingAgentCompactionPolicy::default();
        // floor(10,000 * 95 / 100) = 9,500.
        assert!(!is_past_mid_run_safety_net(9_500, 10_000, &policy));
        assert!(is_past_mid_run_safety_net(9_501, 10_000, &policy));
    }

    #[test]
    fn disabled_or_zero_window_product_policy_never_triggers() {
        let disabled = CodingAgentCompactionPolicy {
            enabled: false,
            ..CodingAgentCompactionPolicy::default()
        };
        let enabled = CodingAgentCompactionPolicy::default();
        assert!(!coding_agent_should_compact(99_999, 100_000, &disabled));
        assert!(!is_past_mid_run_safety_net(99_999, 100_000, &disabled));
        assert!(!coding_agent_should_compact(1, 0, &enabled));
        assert!(!is_past_mid_run_safety_net(1, 0, &enabled));
        assert!(!coding_agent_should_compact(1, -1, &enabled));
        assert!(!is_past_mid_run_safety_net(1, -1, &enabled));
    }

    #[test]
    fn implicit_reserve_is_recalculated_for_the_selected_model() {
        let policy = CodingAgentCompactionPolicy::default();
        assert_eq!(policy.reserve_tokens, None);
        assert_eq!(
            policy.resolve_reserve_tokens(Some(8_000), Some(100_000)),
            8_000
        );
        assert_eq!(
            policy.resolve_reserve_tokens(Some(50_000), Some(20_000)),
            10_000
        );
    }

    #[test]
    fn explicit_default_reserve_is_not_model_adjusted() {
        let policy = CodingAgentCompactionPolicy {
            reserve_tokens: Some(16_384),
            ..CodingAgentCompactionPolicy::default()
        };
        assert_eq!(
            policy.resolve_reserve_tokens(Some(8_000), Some(10_000)),
            16_384
        );
    }
}
