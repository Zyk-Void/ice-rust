//! `AgentSession` — the coding-agent product layer over the harness.
//!
//! Mirrors the role of `packages/coding-agent/src/core/agent-session.ts`: one
//! object the CLI/TUI holds that unifies prompt/continue, queueing,
//! model/thinking mutation, scoped models, compaction + retry, export,
//! session switching / fork, usage + context stats, and name/label facts.
//!
//! Design notes (deliberate, vs the TS original):
//!
//! - **Composition, not inheritance.** TS `AgentSession` *is* the harness plus
//!   a pile of product concerns. Here [`AgentSession`] owns an `AgentHarness`
//!   and adds the product surface; the harness stays usable on its own for
//!   SDK consumers who do not want the product layer. This keeps the
//!   `ice-harness` crate free of CLI concerns (provider catalogs, settings
//!   paths, export formats).
//! - **Sync-friendly handles.** The TUI key loop is a blocking thread, so the
//!   mutating surface that the loop touches (`set_thinking_level`,
//!   `scoped_models`) works off cached state or spawns onto the runtime; the
//!   async surface (`prompt`, `compact`, `export`) is `async fn`.
//! - **Retry/compaction policy lives in the harness.** The session exposes
//!   `retry()`/`compact()` as intent, and the harness applies its own
//!   `RetryPolicy`/`CompactionSettings`. TS keeps the auto-retry driver in the
//!   session; we keep it in the harness so non-CLI callers get it for free.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ice_agent::AgentMessage;
use ice_ai::types::{AssistantMessage, Content, ImageContent, Usage};
use ice_ai::{Model, ThinkingLevel};
use ice_harness::agent_harness::{
    AgentHarness, AgentLane, CompactionOutcome, HarnessRunOutcome, LaneSnapshot, RunResult,
};
use ice_harness::compaction::estimate_context_tokens;
use ice_harness::result::{HarnessError, HarnessResult};
use ice_harness::session::types::{BranchBounds, Entry, EntryOrder, EntryQuery};

use crate::export::{export_session, ExportFormat};

/// Where a session content lives — used by export naming and `--continue`.
#[derive(Debug, Clone)]
pub struct SessionDescriptor {
    pub id: String,
    pub name: Option<String>,
    pub path: Option<PathBuf>,
    pub cwd: PathBuf,
}

/// A snapshot of usage + context pressure for the footer/status line.
#[derive(Debug, Clone, Default)]
pub struct SessionUsageStats {
    pub message_count: u64,
    pub total_tokens: i64,
    pub cached_tokens: i64,
    pub uncached_tokens: i64,
    pub cost_total: f64,
}

impl SessionUsageStats {
    /// Fraction of the rolled-up input tokens that came from cache. `None` when
    /// no input tokens have been recorded yet (avoids a bogus `0 %`).
    pub fn cache_hit_ratio(&self) -> Option<f64> {
        let input = self.cached_tokens + self.uncached_tokens;
        if input <= 0 {
            return None;
        }
        Some(self.cached_tokens as f64 / input as f64)
    }
}

/// Token components and billed cost projected from embedded coding-session entries.
/// Unlike [`SessionUsageStats`], this is an all-history report and derives the
/// token total from the four components instead of trusting `Usage.total_tokens`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CodingUsageTotals {
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_write: i64,
    pub total_tokens: i64,
    pub cost: f64,
}

/// Cost attribution bucket in a coding-session usage report.
#[derive(Debug, Clone, PartialEq)]
pub struct CodingUsageCostBreakdown {
    /// `provider/response-model`, `Tools/summaries`, or `Subagents`.
    pub key: String,
    pub cost: f64,
    pub tokens: i64,
}

/// Active-branch context occupancy. `tokens` and `percent` are unknown directly
/// after compaction until a later non-error, non-aborted assistant reports usage.
#[derive(Debug, Clone, PartialEq)]
pub struct CodingContextUsage {
    pub tokens: Option<i64>,
    pub context_window: u64,
    pub percent: Option<f64>,
}

/// Coding-agent statistics, deliberately separate from native session accounting.
/// Usage covers every stored entry (including abandoned branches); context is
/// calculated only from the active main-lane context.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CodingSessionStats {
    pub user_messages: u64,
    pub assistant_messages: u64,
    pub tool_calls: u64,
    pub tool_results: u64,
    pub total_messages: u64,
    pub usage: CodingUsageTotals,
    pub cost_breakdown: Vec<CodingUsageCostBreakdown>,
    pub context_usage: Option<CodingContextUsage>,
}

/// One scoped-model entry: the routing id plus its human label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopedModel {
    pub id: String,
    pub label: String,
}

impl ScopedModel {
    /// Parse the `provider/model[:thinking]` form used by `--model` and
    /// `settings.json#scopedModels`.
    pub fn parse(raw: &str) -> Self {
        let id = raw.trim().to_string();
        let label = id
            .rsplit('/')
            .next()
            .unwrap_or(&id)
            .split(':')
            .next()
            .unwrap_or(&id)
            .to_string();
        Self { id, label }
    }
}

/// The product-level session. Cheap to clone — the harness is behind an `Arc`.
#[derive(Clone)]
pub struct AgentSession {
    harness: Arc<AgentHarness>,
    catalog: Arc<Vec<Model>>,
    scoped_models: Arc<Vec<ScopedModel>>,
    cwd: PathBuf,
}

impl AgentSession {
    /// Wrap an existing harness. `catalog` is the resolved model catalog the
    /// session may switch within; `scoped_models` is the (possibly empty)
    /// settings-driven subset used by Ctrl+P / `/model`.
    pub fn new(
        harness: AgentHarness,
        catalog: Vec<Model>,
        scoped_models: Vec<ScopedModel>,
        cwd: impl Into<PathBuf>,
    ) -> Self {
        Self {
            harness: Arc::new(harness),
            catalog: Arc::new(catalog),
            scoped_models: Arc::new(scoped_models),
            cwd: cwd.into(),
        }
    }

    /// The wrapped harness, for the call sites (TUI drain task, extension
    /// bridge) that already speak the harness API.
    pub fn harness(&self) -> &AgentHarness {
        &self.harness
    }

    /// The `Arc` handle, for spawning work onto the runtime.
    pub fn harness_arc(&self) -> Arc<AgentHarness> {
        Arc::clone(&self.harness)
    }

    /// The main-lane runner.
    pub fn lane(&self) -> Arc<dyn AgentLane> {
        self.harness.lane("main")
    }

    /// The working directory the session was opened in.
    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    /// The resolved model catalog.
    pub fn catalog(&self) -> &[Model] {
        &self.catalog
    }

    /// The active scoped-model set (`[]` → the full catalog is in play).
    pub fn scoped_models(&self) -> &[ScopedModel] {
        &self.scoped_models
    }

    // -- prompt / continue / queue -------------------------------------------

    /// Send a prompt with optional image attachments. Mirrors TS
    /// `AgentSession.prompt`.
    pub async fn prompt(&self, text: &str, images: Vec<ImageContent>) -> HarnessResult<RunResult> {
        self.harness.prompt_text(text, images).await
    }

    /// Send an already-built user message. Mirrors TS
    /// `AgentSession.sendUserMessage`.
    pub async fn send_message(&self, message: AgentMessage) -> HarnessResult<RunResult> {
        self.harness.prompt_message(message).await
    }

    /// Queue a steering message for the in-flight run. Mirrors TS
    /// `AgentSession.steer`.
    pub async fn steer(&self, message: AgentMessage) -> HarnessResult<()> {
        self.harness.steer(message).await?;
        Ok(())
    }

    /// Queue a follow-up message for after the in-flight run. Mirrors TS
    /// `AgentSession.followUp`.
    pub async fn follow_up(&self, message: AgentMessage) -> HarnessResult<()> {
        self.harness.follow_up(message).await?;
        Ok(())
    }

    /// Abort the in-flight run. Mirrors TS `AgentSession.abort`.
    pub async fn abort(&self) -> HarnessResult<()> {
        self.harness.abort().await?;
        Ok(())
    }

    // -- model / thinking ----------------------------------------------------

    /// The current model.
    pub async fn model(&self) -> HarnessResult<Model> {
        self.harness.get_model().await
    }

    /// Switch the model. Mirrors TS `AgentSession.setModel`.
    pub async fn set_model(&self, model: Model) -> HarnessResult<()> {
        self.harness.set_model(model).await
    }

    /// The current thinking level.
    pub async fn thinking_level(&self) -> HarnessResult<ThinkingLevel> {
        self.harness.get_thinking_level().await
    }

    /// Set the thinking level. Mirrors TS `AgentSession.setThinkingLevel`.
    pub async fn set_thinking_level(&self, level: ThinkingLevel) -> HarnessResult<()> {
        self.harness.set_thinking_level(level).await
    }

    /// The effective model cycle order: the scoped set when configured, else
    /// the full catalog. Mirrors TS `AgentSession.getScopedModels`.
    pub fn cycle_catalog(&self) -> Vec<Model> {
        if self.scoped_models.is_empty() {
            return self.catalog.to_vec();
        }
        let mut out = Vec::new();
        for scoped in self.scoped_models.iter() {
            let bare = scoped.id.split(':').next().unwrap_or(&scoped.id);
            if let Some(model) = self.catalog.iter().find(|m| {
                m.id == bare
                    || format!("{}/{}", m.provider, m.id) == bare
                    || format!("{}/{}", m.provider, m.id) == scoped.id
            }) {
                out.push(model.clone());
            }
        }
        out
    }

    // -- compaction / retry --------------------------------------------------

    /// Explicit compaction. A declined, aborted, or failed compaction is not a
    /// successful save. Use [`Self::compact_entry`] to inspect the saved entry.
    pub async fn compact(&self, instructions: Option<&str>) -> HarnessResult<()> {
        self.compact_entry(instructions).await.map(|_| ())
    }

    /// Compact and return the actual durable checkpoint, including its entry ID.
    /// This never searches by summary text, which may be shared by checkpoints.
    pub async fn compact_entry(&self, instructions: Option<&str>) -> HarnessResult<Entry> {
        match self.harness.compact(instructions).await?.outcome {
            CompactionOutcome::Completed { entry, .. } => Ok(entry),
            CompactionOutcome::Declined { .. } => Err(HarnessError::NothingToCompact {
                lane: "main".to_string(),
                message: "There is no history available to compact".to_string(),
            }),
            CompactionOutcome::Aborted { .. } => Err(HarnessError::Compaction {
                message: "Compaction was cancelled before saving a checkpoint".to_string(),
            }),
            CompactionOutcome::Failed { error, .. } => Err(HarnessError::Compaction {
                message: error.message,
            }),
        }
    }

    /// Explicitly resubmit the last user message as a new turn, when one exists.
    /// This is a user-requested rerun, not automatic provider recovery: it may
    /// repeat tools or external side effects and is not a safe/exactly-once replay.
    pub async fn retry(&self) -> HarnessResult<Option<RunResult>> {
        let Some(last_user) = self.last_user_message().await? else {
            return Ok(None);
        };
        self.harness.prompt_message(last_user).await.map(Some)
    }

    // -- durable reads -------------------------------------------------------

    /// The current leaf entry id.
    pub async fn tip_id(&self) -> HarnessResult<Option<String>> {
        self.harness.get_tip_id().await
    }

    /// All entries on the session, oldest first.
    pub async fn entries(&self) -> HarnessResult<Vec<Entry>> {
        self.harness
            .find_entries(&EntryQuery {
                order: Some(EntryOrder::OldestFirst),
                ..Default::default()
            })
            .await
    }

    /// The lane read model. Mirrors the TS runtime lane snapshot.
    pub async fn snapshot(&self) -> HarnessResult<LaneSnapshot> {
        self.harness.snapshot().await
    }

    /// Name/label facts. Mirrors TS `AgentSession.setName` / `setLabel`.
    pub async fn name(&self) -> HarnessResult<Option<String>> {
        self.harness.get_name().await
    }

    /// Set (or clear) the session name.
    pub async fn set_name(&self, name: Option<&str>) -> HarnessResult<()> {
        self.harness.set_name(name).await
    }

    /// Attach (or clear) a label on an entry.
    pub async fn set_label(&self, target_id: &str, label: Option<&str>) -> HarnessResult<()> {
        self.harness.set_label(target_id, label).await
    }

    /// Read a durable session value (see `ice_harness::session::values`).
    pub async fn value(&self, key: &str) -> HarnessResult<Option<serde_json::Value>> {
        let values = ice_harness::session::SessionValues::load(self.harness.session())
            .await
            .map_err(|e| HarnessError::io(e.to_string()))?;
        Ok(values.get(key).cloned())
    }

    /// Persist a durable session value (latest-wins). Used for per-session
    /// display preferences so they survive resume.
    pub async fn set_value(&self, key: &str, value: serde_json::Value) -> HarnessResult<()> {
        let writer = ice_harness::session::SessionValueWriter::new(self.harness.session().clone());
        writer
            .set(key, value)
            .await
            .map_err(|e| HarnessError::io(e.to_string()))?;
        Ok(())
    }

    /// Rolled-up usage + context stats for the footer.
    pub async fn usage_stats(&self) -> HarnessResult<SessionUsageStats> {
        let stats = self.harness.get_stats().await?;
        Ok(SessionUsageStats {
            message_count: stats.message_count,
            total_tokens: stats.total_tokens,
            cached_tokens: stats.cached_tokens,
            uncached_tokens: stats.uncached_tokens,
            cost_total: stats.cost_total,
        })
    }

    /// All-history coding-session usage for `/context` and billing display.
    ///
    /// This additive coding projection reads usage embedded in assistant and
    /// tool-result messages, summaries, and validated `subagent_usage` custom
    /// entries. It intentionally does not also add native `UsageRecord`s: those
    /// records remain the authority for [`Self::usage_stats`], while embedded
    /// records are the coding-session history projection. Token total is derived
    /// from input/output/cache components; costs sum each usage's `cost.total`.
    pub async fn total_usage(&self) -> HarnessResult<Usage> {
        let projection = project_coding_history(&self.entries().await?)?;
        Ok(projection.as_usage()?)
    }

    /// The all-history coding-agent usage and active-branch context report.
    ///
    /// Unlike [`Self::usage_stats`], this includes embedded assistant, tool,
    /// summary and subagent usage from abandoned as well as active branches.
    pub async fn coding_session_stats(&self) -> HarnessResult<CodingSessionStats> {
        let projection = project_coding_history(&self.entries().await?)?;
        let context_usage = self.current_context_usage().await?;
        Ok(CodingSessionStats {
            user_messages: projection.user_messages,
            assistant_messages: projection.assistant_messages,
            tool_calls: projection.tool_calls,
            tool_results: projection.tool_results,
            total_messages: projection.total_messages,
            usage: projection.usage_totals()?,
            cost_breakdown: projection.cost_breakdown()?,
            context_usage,
        })
    }

    /// Estimated occupancy of the active main-lane provider context.
    ///
    /// A positive-window model yields unknown (`None` tokens/percent) after
    /// compaction until at least one later non-error/non-aborted assistant has
    /// reported positive usage. A subsequent zero-usage assistant does not
    /// invalidate that measurement gate. Percentages are deliberately not
    /// clamped to the model window.
    pub async fn current_context_usage(&self) -> HarnessResult<Option<CodingContextUsage>> {
        let model = self.harness.get_model().await?;
        if model.context_window == 0 {
            return Ok(None);
        }

        let entries = self.active_branch_entries().await?;
        let latest_compaction = entries
            .iter()
            .rposition(|entry| matches!(entry, Entry::Compaction(_)));
        if let Some(compaction_index) = latest_compaction {
            let has_post_compaction_usage = entries
                .iter()
                .skip(compaction_index + 1)
                .any(entry_has_positive_assistant_usage);
            if !has_post_compaction_usage {
                return Ok(Some(CodingContextUsage {
                    tokens: None,
                    context_window: model.context_window,
                    percent: None,
                }));
            }
        }

        // Use the canonical compaction-aware context projection, never the
        // session-wide entry list, so abandoned branches cannot affect occupancy.
        let context = self.active_context().await?;
        let tokens = estimate_context_tokens(&context.messages).tokens;
        Ok(Some(CodingContextUsage {
            tokens: Some(tokens),
            context_window: model.context_window,
            percent: Some(tokens as f64 / model.context_window as f64 * 100.0),
        }))
    }

    /// Explain the active configured context before per-request async hooks.
    /// Provider usage is kept separate from estimated category composition, and
    /// remains unknown immediately after compaction. The configured prompt is
    /// counted as a whole: this read does not re-open resource files or invent
    /// tool provenance. Hosts with captured resource/source snapshots can use
    /// [`crate::context_breakdown::compute_context_breakdown`] for finer splits.
    pub async fn context_breakdown(
        &self,
    ) -> HarnessResult<Option<crate::context_breakdown::ContextBreakdown>> {
        let snapshot = self.harness.context_snapshot().await?;
        if snapshot.model.context_window == 0 {
            return Ok(None);
        }
        let known = self
            .current_context_usage()
            .await?
            .and_then(|usage| usage.tokens)
            .is_some();
        let estimate = estimate_context_tokens(&snapshot.context.messages);
        let measured_tokens = if known && estimate.usage_tokens > 0 {
            u64::try_from(estimate.tokens).ok()
        } else {
            None
        };
        let mut breakdown = crate::context_breakdown::compute_context_breakdown(
            &crate::context_breakdown::ContextBreakdownInput {
                context: &snapshot.context.messages,
                context_window: snapshot.model.context_window,
                system_prompt: Some(&snapshot.system_prompt),
                base_system_prompt: Some(&snapshot.system_prompt),
                tools: Some(&snapshot.tools),
                measured_tokens,
                compaction: Some(&snapshot.compaction),
                ..Default::default()
            },
        );
        breakdown.compact_at_tokens = snapshot.compact_at_tokens;
        Ok(Some(breakdown))
    }

    /// The last assistant's text in the active compacted context (for `/copy`).
    /// Only an aborted assistant with no content is skipped. Once an assistant
    /// is selected, its text blocks are concatenated without separators and
    /// trimmed; a selected non-text-only message does not fall back to older text.
    pub async fn last_assistant_text(&self) -> HarnessResult<Option<String>> {
        let context = self.active_context().await?;
        for message in context.messages.iter().rev() {
            let AgentMessage::Assistant(assistant) = message else {
                continue;
            };
            if assistant.stop_reason == ice_ai::types::StopReason::Aborted
                && assistant.content.is_empty()
            {
                continue;
            }
            let mut text = String::new();
            for block in &assistant.content {
                if let Content::Text(text_block) = block {
                    text.push_str(&text_block.text);
                }
            }
            let text = text.trim();
            return Ok((!text.is_empty()).then(|| text.to_string()));
        }
        Ok(None)
    }

    // -- export --------------------------------------------------------------

    /// `exportToHtml()`. Mirrors TS `AgentSession.exportToHtml`.
    pub async fn export_to_html(&self, path: &Path) -> HarnessResult<()> {
        self.export(ExportFormat::Html, path).await
    }

    /// `exportToJsonl()`. Mirrors TS `AgentSession.exportToJsonl`.
    pub async fn export_to_jsonl(&self, path: &Path) -> HarnessResult<()> {
        self.export(ExportFormat::Jsonl, path).await
    }

    /// `exportToMarkdown()` — the local `/export` default.
    pub async fn export_to_markdown(&self, path: &Path) -> HarnessResult<()> {
        self.export(ExportFormat::Markdown, path).await
    }

    /// Export to `format`, deriving the default filename from the session
    /// name/leaf when `path` is a directory.
    pub async fn export(&self, format: ExportFormat, path: &Path) -> HarnessResult<()> {
        let target = if path.is_dir() {
            path.join(self.default_export_file_name(format.clone()).await)
        } else {
            path.to_path_buf()
        };
        export_session(&self.harness, format, &target)
            .await
            .map_err(HarnessError::io)
    }

    /// The filename `/export` would write for `format`.
    pub async fn default_export_file_name(&self, format: ExportFormat) -> String {
        let extension = match format {
            ExportFormat::Markdown => "md",
            ExportFormat::Html => "html",
            ExportFormat::Jsonl => "jsonl",
        };
        let name = self.name().await.ok().flatten().unwrap_or_default();
        if !name.is_empty() {
            return format!("{name}.{extension}");
        }
        let id = self
            .tip_id()
            .await
            .ok()
            .flatten()
            .unwrap_or_else(|| "session".to_string());
        format!("{id}.{extension}")
    }

    // -- fork / tree ---------------------------------------------------------

    /// Mark the current leaf as a fork point: the label (when given) tags the
    /// leaf and the visible conversation is unchanged, so the next message
    /// continues from it. Navigate elsewhere with `navigate_tree` to branch off.
    pub async fn fork(&self, label: Option<&str>) -> HarnessResult<()> {
        if let (Some(label), Some(leaf)) = (label, self.tip_id().await?) {
            self.harness.set_label(&leaf, Some(label)).await?;
        }
        Ok(())
    }

    /// Read back the terminal outcome of a run by id. Mirrors TS
    /// `AgentSession.getResult`.
    pub async fn result(&self, run_id: &str) -> HarnessResult<Option<RunResult>> {
        self.harness.get_result(run_id).await
    }

    // -- internals -----------------------------------------------------------

    async fn active_branch_entries(&self) -> HarnessResult<Vec<Entry>> {
        self.harness
            .find_entries_on_branch(
                &EntryQuery {
                    order: Some(EntryOrder::OldestFirst),
                    ..Default::default()
                },
                &BranchBounds::default(),
            )
            .await
    }

    async fn active_context(&self) -> HarnessResult<ice_harness::session::context::SessionContext> {
        Ok(self.harness.context_snapshot().await?.context)
    }

    async fn last_user_message(&self) -> HarnessResult<Option<AgentMessage>> {
        let context = self.active_context().await?;
        Ok(context
            .messages
            .into_iter()
            .rev()
            .find(|message| matches!(message, AgentMessage::User(_))))
    }
}

const MAX_SAFE_SUBAGENT_INTEGER: i64 = 9_007_199_254_740_991;

#[derive(Debug, Clone, Default)]
struct UsageAccumulator {
    input: i64,
    output: i64,
    cache_read: i64,
    cache_write: i64,
    cost_input: f64,
    cost_output: f64,
    cost_cache_read: f64,
    cost_cache_write: f64,
    cost_total: f64,
}

impl UsageAccumulator {
    fn add(&mut self, usage: &Usage) -> HarnessResult<()> {
        self.input = checked_add_tokens(self.input, usage.input, "input")?;
        self.output = checked_add_tokens(self.output, usage.output, "output")?;
        self.cache_read = checked_add_tokens(self.cache_read, usage.cache_read, "cacheRead")?;
        self.cache_write = checked_add_tokens(self.cache_write, usage.cache_write, "cacheWrite")?;
        self.cost_input = checked_add_cost(self.cost_input, usage.cost.input, "input")?;
        self.cost_output = checked_add_cost(self.cost_output, usage.cost.output, "output")?;
        self.cost_cache_read =
            checked_add_cost(self.cost_cache_read, usage.cost.cache_read, "cacheRead")?;
        self.cost_cache_write =
            checked_add_cost(self.cost_cache_write, usage.cost.cache_write, "cacheWrite")?;
        self.cost_total = checked_add_cost(self.cost_total, usage.cost.total, "total")?;
        Ok(())
    }

    fn total_tokens(&self) -> HarnessResult<i64> {
        checked_add_tokens(
            checked_add_tokens(
                checked_add_tokens(self.input, self.output, "total")?,
                self.cache_read,
                "total",
            )?,
            self.cache_write,
            "total",
        )
    }

    fn coding_totals(&self) -> HarnessResult<CodingUsageTotals> {
        Ok(CodingUsageTotals {
            input: self.input,
            output: self.output,
            cache_read: self.cache_read,
            cache_write: self.cache_write,
            total_tokens: self.total_tokens()?,
            cost: self.cost_total,
        })
    }

    fn as_usage(&self) -> HarnessResult<Usage> {
        Ok(Usage {
            input: self.input,
            output: self.output,
            cache_read: self.cache_read,
            cache_write: self.cache_write,
            cache_write_1h: None,
            reasoning: None,
            total_tokens: self.total_tokens()?,
            cost: ice_ai::types::UsageCost {
                input: self.cost_input,
                output: self.cost_output,
                cache_read: self.cost_cache_read,
                cache_write: self.cost_cache_write,
                total: self.cost_total,
            },
        })
    }
}

fn checked_add_tokens(left: i64, right: i64, component: &str) -> HarnessResult<i64> {
    left.checked_add(right).ok_or_else(|| {
        HarnessError::io(format!(
            "coding-session {component} token total overflowed i64"
        ))
    })
}

fn checked_add_cost(left: f64, right: f64, component: &str) -> HarnessResult<f64> {
    if !right.is_finite() {
        return Err(HarnessError::io(format!(
            "coding-session {component} cost must be finite"
        )));
    }
    let sum = left + right;
    if !sum.is_finite() {
        return Err(HarnessError::io(format!(
            "coding-session {component} cost total overflowed"
        )));
    }
    Ok(sum)
}

#[derive(Debug, Default)]
struct CodingHistoryProjection {
    user_messages: u64,
    assistant_messages: u64,
    tool_calls: u64,
    tool_results: u64,
    total_messages: u64,
    totals: UsageAccumulator,
    groups: Vec<(String, UsageAccumulator)>,
}

impl CodingHistoryProjection {
    fn add_usage(&mut self, group: Option<&str>, usage: &Usage) -> HarnessResult<()> {
        self.totals.add(usage)?;
        if let Some(group) = group {
            let index = self
                .groups
                .iter()
                .position(|(key, _)| key == group)
                .unwrap_or_else(|| {
                    self.groups
                        .push((group.to_string(), UsageAccumulator::default()));
                    self.groups.len() - 1
                });
            self.groups[index].1.add(usage)?;
        }
        Ok(())
    }

    fn usage_totals(&self) -> HarnessResult<CodingUsageTotals> {
        self.totals.coding_totals()
    }

    fn as_usage(&self) -> HarnessResult<Usage> {
        self.totals.as_usage()
    }

    fn cost_breakdown(&self) -> HarnessResult<Vec<CodingUsageCostBreakdown>> {
        let mut entries = Vec::with_capacity(self.groups.len());
        for (key, totals) in &self.groups {
            let tokens = totals.total_tokens()?;
            if totals.cost_total > 0.0 || tokens > 0 {
                entries.push(CodingUsageCostBreakdown {
                    key: key.clone(),
                    cost: totals.cost_total,
                    tokens,
                });
            }
        }
        // Slice sorting is stable: equal-cost groups retain their first-seen
        // order, matching the source Map's insertion order.
        entries.sort_by(|left, right| {
            right
                .cost
                .partial_cmp(&left.cost)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(entries)
    }
}

fn project_coding_history(entries: &[Entry]) -> HarnessResult<CodingHistoryProjection> {
    let mut projection = CodingHistoryProjection::default();
    for entry in entries {
        match entry {
            Entry::Message(message_entry) => {
                projection.total_messages += 1;
                match &message_entry.message {
                    AgentMessage::User(_) => projection.user_messages += 1,
                    AgentMessage::Assistant(assistant) => {
                        projection.assistant_messages += 1;
                        projection.tool_calls += assistant
                            .content
                            .iter()
                            .filter(|block| matches!(block, Content::ToolCall(_)))
                            .count() as u64;
                        projection.add_usage(
                            Some(&format!(
                                "{}/{}",
                                assistant.provider.as_str(),
                                assistant
                                    .response_model
                                    .as_deref()
                                    .unwrap_or(&assistant.model)
                            )),
                            &assistant.usage,
                        )?;
                    }
                    AgentMessage::ToolResult(tool_result) => {
                        projection.tool_results += 1;
                        if let Some(usage) = &tool_result.usage {
                            projection.add_usage(Some("Tools/summaries"), usage)?;
                        }
                    }
                    AgentMessage::Custom(_) => {}
                }
            }
            Entry::Compaction(compaction) => {
                if let Some(usage) = &compaction.usage {
                    projection.add_usage(Some("Tools/summaries"), usage)?;
                }
            }
            Entry::BranchSummary(summary) => {
                if let Some(usage) = &summary.usage {
                    projection.add_usage(Some("Tools/summaries"), usage)?;
                }
            }
            Entry::Custom(custom) if custom.custom_type == "subagent_usage" => {
                if let Some(usage) = subagent_usage_from_entry(custom) {
                    projection.add_usage(Some("Subagents"), &usage)?;
                }
            }
            _ => {}
        }
    }
    Ok(projection)
}

fn subagent_usage_from_entry(custom: &ice_harness::session::types::CustomEntry) -> Option<Usage> {
    if custom.custom_type != "subagent_usage" {
        return None;
    }
    let data = custom.data.as_ref()?.as_object()?;
    let raw_usage = data.get("usage")?.as_object()?;
    let input = safe_nonnegative_integer(raw_usage.get("input")?)?;
    let output = safe_nonnegative_integer(raw_usage.get("output")?)?;
    let cache_read = safe_nonnegative_integer(raw_usage.get("cacheRead")?)?;
    let cache_write = safe_nonnegative_integer(raw_usage.get("cacheWrite")?)?;
    let cost = raw_usage.get("cost")?.as_f64()?;
    if !cost.is_finite() || cost < 0.0 {
        return None;
    }
    let total_tokens = input
        .checked_add(output)?
        .checked_add(cache_read)?
        .checked_add(cache_write)?;
    if total_tokens > MAX_SAFE_SUBAGENT_INTEGER {
        return None;
    }
    Some(Usage {
        input,
        output,
        cache_read,
        cache_write,
        cache_write_1h: None,
        reasoning: None,
        total_tokens,
        cost: ice_ai::types::UsageCost {
            total: cost,
            ..Default::default()
        },
    })
}

fn safe_nonnegative_integer(value: &serde_json::Value) -> Option<i64> {
    let number = value.as_f64()?;
    if !number.is_finite()
        || number < 0.0
        || number > MAX_SAFE_SUBAGENT_INTEGER as f64
        || number.fract() != 0.0
    {
        return None;
    }
    Some(number as i64)
}

fn entry_has_positive_assistant_usage(entry: &Entry) -> bool {
    let Entry::Message(message_entry) = entry else {
        return false;
    };
    let AgentMessage::Assistant(assistant) = &message_entry.message else {
        return false;
    };
    !matches!(
        assistant.stop_reason,
        ice_ai::types::StopReason::Error | ice_ai::types::StopReason::Aborted
    ) && assistant.usage.context_tokens() > 0
}

/// The finalized text of an assistant message (text blocks only), joined with
/// blank lines. Mirrors the TS print-mode/export concat.
pub fn assistant_text(message: &AssistantMessage) -> String {
    message
        .content
        .iter()
        .filter_map(|c| match c {
            Content::Text(t) => Some(t.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Whether a run outcome is terminal (i.e. produced a final assistant message).
pub fn outcome_is_completed(outcome: &HarnessRunOutcome) -> bool {
    matches!(outcome, HarnessRunOutcome::Completed { .. })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ice_ai::Api;
    use ice_harness::agent_harness::AgentHarness;
    use ice_harness::session::memory::{InMemorySessionStorage, SystemClock};
    use ice_harness::session::session::{DefaultIdGenerator, Session};
    use ice_harness::session::types::SessionMetadata;
    use ice_harness::types::{
        AgentHarnessOptions, AgentHarnessResources, DrivingMode, HarnessToolExecution, RetryPolicy,
    };

    fn test_session() -> Session {
        Session::new(
            std::sync::Arc::new(InMemorySessionStorage::new(
                SessionMetadata {
                    id: "s-agent-session".to_string(),
                    created_at: 0,
                    parent_session_id: None,
                },
                std::sync::Arc::new(SystemClock),
                std::sync::Arc::new(DefaultIdGenerator::new()),
            )),
            None,
        )
    }

    async fn test_harness() -> AgentHarness {
        test_harness_with_models(vec![ice_ai::providers::faux::FauxProvider::new(
            ice_ai::providers::faux::FauxScript::new(),
        )])
        .await
    }

    async fn test_harness_with_models(models: Vec<Arc<dyn ice_ai::Provider>>) -> AgentHarness {
        AgentHarness::create(test_harness_options(models))
            .await
            .unwrap()
    }

    fn test_harness_options(models: Vec<Arc<dyn ice_ai::Provider>>) -> AgentHarnessOptions {
        AgentHarnessOptions {
            model: Model::new("m", "m", Api::Faux, "faux", "http://x"),
            thinking_level: Default::default(),
            active_tool_names: Vec::new(),
            tools: Vec::new(),
            system_prompt: None,
            resources: AgentHarnessResources::default(),
            stream_options: Default::default(),
            retry: RetryPolicy::default(),
            compaction: Default::default(),
            steering_mode: Default::default(),
            follow_up_mode: Default::default(),
            tool_execution: HarnessToolExecution::default(),
            drive: DrivingMode::default(),
            session: test_session(),
            allow_existing_session: true,
            models,
            to_provider_messages: None,
            entry_projectors: Default::default(),
            agent_emitter: None,
            before_tool_call: None,
            after_tool_call: None,
            transform_context: None,
            entry_transforms: Vec::new(),
            provider_hooks: None,
        }
    }

    fn usage(input: i64, output: i64, cache_read: i64, cache_write: i64, cost: f64) -> Usage {
        let mut usage = Usage::zero();
        usage.input = input;
        usage.output = output;
        usage.cache_read = cache_read;
        usage.cache_write = cache_write;
        // Deliberately unrelated to the components: coding-session totals must
        // derive this field rather than trust the provider-reported total.
        usage.total_tokens = 999;
        usage.cost.total = cost;
        usage
    }

    fn assistant_message(content: Vec<Content>, usage: Usage) -> AssistantMessage {
        let mut assistant = AssistantMessage::empty(Api::Faux, "provider", "model", 1);
        assistant.content = content;
        assistant.usage = usage;
        assistant.stop_reason = ice_ai::types::StopReason::Stop;
        assistant
    }

    async fn append_assistant(harness: &AgentHarness, message: AssistantMessage) -> String {
        harness
            .session()
            .append_message(AgentMessage::Assistant(Box::new(message)))
            .await
            .unwrap()
    }

    async fn append_usage_entry(
        harness: &AgentHarness,
        id: &str,
        kind: ice_harness::session::types::ProvisionedKind,
    ) {
        harness
            .session()
            .append_entry(
                ice_harness::session::types::ProvisionedEntry {
                    id: id.to_string(),
                    kind,
                },
                "main",
            )
            .await
            .unwrap();
    }

    fn text_block(text: &str) -> Content {
        Content::Text(ice_ai::types::TextContent {
            kind: ice_ai::types::TextContentType,
            text: text.to_string(),
            text_signature: None,
        })
    }

    fn custom_subagent_usage(
        data: serde_json::Value,
    ) -> ice_harness::session::types::ProvisionedKind {
        ice_harness::session::types::ProvisionedKind::Custom {
            custom_type: "subagent_usage".to_string(),
            data: Some(data),
        }
    }

    #[test]
    fn scoped_model_parses_provider_and_thinking_forms() {
        let m = ScopedModel::parse("anthropic/claude-opus-4-8:high");
        assert_eq!(m.id, "anthropic/claude-opus-4-8:high");
        assert_eq!(m.label, "claude-opus-4-8");

        let bare = ScopedModel::parse("gpt-5");
        assert_eq!(bare.label, "gpt-5");
    }

    #[test]
    fn cache_hit_ratio_is_none_without_input() {
        let stats = SessionUsageStats::default();
        assert!(stats.cache_hit_ratio().is_none());

        let stats = SessionUsageStats {
            cached_tokens: 75,
            uncached_tokens: 25,
            ..Default::default()
        };
        assert_eq!(stats.cache_hit_ratio(), Some(0.75));
    }

    #[test]
    fn assistant_text_joins_text_blocks_only() {
        let mut msg =
            ice_ai::types::AssistantMessage::empty(ice_ai::Api::AnthropicMessages, "p", "m", 0);
        msg.content = vec![
            Content::Thinking(ice_ai::types::ThinkingContent {
                kind: ice_ai::types::ThinkingContentType,
                thinking: "hidden".into(),
                thinking_signature: None,
                redacted: false,
            }),
            Content::Text(ice_ai::types::TextContent {
                kind: ice_ai::types::TextContentType,
                text: "hello".into(),
                text_signature: None,
            }),
            Content::Text(ice_ai::types::TextContent {
                kind: ice_ai::types::TextContentType,
                text: "world".into(),
                text_signature: None,
            }),
        ];
        assert_eq!(assistant_text(&msg), "hello\nworld");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn compact_entry_returns_the_actual_durable_checkpoint() {
        use ice_ai::providers::faux::{FauxProvider, FauxScript};
        let provider = FauxProvider::new(FauxScript::new().with_text("A saved summary"));
        let harness = test_harness_with_models(vec![provider]).await;
        let session = AgentSession::new(harness, Vec::new(), Vec::new(), ".");
        session
            .harness()
            .session()
            .append_message(AgentMessage::User(ice_ai::types::UserMessage::new(
                "A conversation to compact",
                1,
            )))
            .await
            .unwrap();
        let entry = session.compact_entry(None).await.unwrap();
        assert!(
            matches!(&entry, Entry::Compaction(checkpoint) if checkpoint.summary == "A saved summary")
        );
        assert_eq!(session.tip_id().await.unwrap().as_deref(), Some(entry.id()));
        assert_eq!(
            session
                .harness()
                .session()
                .get_entry(entry.id())
                .await
                .unwrap(),
            Some(entry)
        );
        assert!(session.snapshot().await.unwrap().active.is_none());
    }

    #[tokio::test]
    async fn compact_does_not_report_an_empty_session_as_success() {
        let session = AgentSession::new(test_harness().await, Vec::new(), Vec::new(), ".");
        let error = session
            .compact(None)
            .await
            .expect_err("empty session declined");
        assert!(matches!(error, HarnessError::NothingToCompact { .. }));
    }

    #[tokio::test]
    async fn compact_propagates_a_failed_summary_instead_of_reporting_success() {
        let session = AgentSession::new(
            test_harness_with_models(Vec::new()).await,
            Vec::new(),
            Vec::new(),
            ".",
        );
        session
            .harness()
            .session()
            .append_message(AgentMessage::User(ice_ai::types::UserMessage::new(
                "A conversation to compact",
                1,
            )))
            .await
            .unwrap();
        let error = session
            .compact(None)
            .await
            .expect_err("provider is unavailable");
        assert!(matches!(error, HarnessError::Compaction { .. }));
        assert!(error.to_string().contains("No provider for compaction"));
        assert!(session.snapshot().await.unwrap().active.is_none());
    }

    #[tokio::test]
    async fn coding_usage_includes_every_embedded_billing_source_and_groups_it() {
        let harness = test_harness().await;
        let mut model = Model::new("m", "m", Api::Faux, "faux", "http://x");
        model.context_window = 100;
        harness.set_model(model).await.unwrap();
        let session = AgentSession::new(harness, Vec::new(), Vec::new(), ".");

        let mut assistant = assistant_message(
            vec![
                text_block("answer"),
                Content::ToolCall(ice_ai::types::ToolCall {
                    kind: Default::default(),
                    id: "call-1".to_string(),
                    name: "test".to_string(),
                    arguments: serde_json::json!({}),
                    thought_signature: None,
                    namespace: None,
                }),
            ],
            usage(3, 2, 1, 1, 0.5),
        );
        assistant.response_model = Some("served-model".to_string());
        append_assistant(session.harness(), assistant).await;
        append_assistant(
            session.harness(),
            assistant_message(vec![text_block("fallback model")], usage(1, 0, 0, 0, 0.2)),
        )
        .await;
        session
            .harness()
            .session()
            .append_message(AgentMessage::ToolResult(Box::new(
                ice_ai::types::ToolResultMessage {
                    role: Default::default(),
                    tool_call_id: "tool-call".to_string(),
                    tool_name: "tool".to_string(),
                    content: Vec::new(),
                    details: None,
                    usage: Some(usage(1, 2, 3, 4, 1.0)),
                    added_tool_names: Vec::new(),
                    is_error: false,
                    timestamp: 2,
                },
            )))
            .await
            .unwrap();
        append_usage_entry(
            session.harness(),
            "compact-usage",
            ice_harness::session::types::ProvisionedKind::Compaction {
                summary: "summary".to_string(),
                retained_tail: Vec::new(),
                tokens_before: 20,
                details: None,
                usage: Some(usage(2, 0, 1, 0, 0.25)),
            },
        )
        .await;
        append_usage_entry(
            session.harness(),
            "branch-usage",
            ice_harness::session::types::ProvisionedKind::BranchSummary {
                from_id: "root".to_string(),
                summary: "branch summary".to_string(),
                details: None,
                usage: Some(usage(0, 1, 1, 0, 0.75)),
            },
        )
        .await;
        append_usage_entry(
            session.harness(),
            "subagent-usage",
            custom_subagent_usage(serde_json::json!({"usage": {
                "input": 2, "output": 1, "cacheRead": 2, "cacheWrite": 0, "cost": 3.0
            }})),
        )
        .await;
        // Native records are intentionally a separate accounting authority.
        session
            .harness()
            .record_usage(usage(500, 0, 0, 0, 100.0), None, None)
            .await
            .unwrap();

        let totals = session.total_usage().await.unwrap();
        assert_eq!(
            (
                totals.input,
                totals.output,
                totals.cache_read,
                totals.cache_write
            ),
            (9, 6, 8, 5)
        );
        assert_eq!(totals.total_tokens, 28);
        assert_eq!(totals.cost.total, 5.7);
        assert_eq!(session.usage_stats().await.unwrap().total_tokens, 999);

        let stats = session.coding_session_stats().await.unwrap();
        assert_eq!(stats.usage.total_tokens, 28);
        assert_eq!(stats.usage.cost, 5.7);
        assert_eq!(stats.total_messages, 3);
        assert_eq!(stats.assistant_messages, 2);
        assert_eq!(stats.tool_results, 1);
        assert_eq!(stats.tool_calls, 1);
        assert_eq!(
            stats.cost_breakdown,
            vec![
                CodingUsageCostBreakdown {
                    key: "Subagents".to_string(),
                    cost: 3.0,
                    tokens: 5,
                },
                CodingUsageCostBreakdown {
                    key: "Tools/summaries".to_string(),
                    cost: 2.0,
                    tokens: 15,
                },
                CodingUsageCostBreakdown {
                    key: "provider/served-model".to_string(),
                    cost: 0.5,
                    tokens: 7,
                },
                CodingUsageCostBreakdown {
                    key: "provider/model".to_string(),
                    cost: 0.2,
                    tokens: 1,
                },
            ]
        );
    }

    #[tokio::test]
    async fn coding_usage_skips_malformed_subagent_payloads() {
        let harness = test_harness().await;
        let session = AgentSession::new(harness, Vec::new(), Vec::new(), ".");
        for (id, usage_payload) in [
            (
                "fractional",
                serde_json::json!({"input": 1.5, "output": 0, "cacheRead": 0, "cacheWrite": 0, "cost": 1.0}),
            ),
            (
                "negative",
                serde_json::json!({"input": -1, "output": 0, "cacheRead": 0, "cacheWrite": 0, "cost": 1.0}),
            ),
            (
                "unsafe",
                serde_json::json!({"input": 9007199254740992u64, "output": 0, "cacheRead": 0, "cacheWrite": 0, "cost": 1.0}),
            ),
            (
                "unsafe-sum",
                serde_json::json!({"input": 9007199254740991u64, "output": 1, "cacheRead": 0, "cacheWrite": 0, "cost": 1.0}),
            ),
            (
                "negative-cost",
                serde_json::json!({"input": 1, "output": 0, "cacheRead": 0, "cacheWrite": 0, "cost": -0.1}),
            ),
            (
                "bad-cost",
                serde_json::json!({"input": 1, "output": 0, "cacheRead": 0, "cacheWrite": 0, "cost": "NaN"}),
            ),
        ] {
            append_usage_entry(
                session.harness(),
                id,
                custom_subagent_usage(serde_json::json!({"usage": usage_payload})),
            )
            .await;
        }

        let totals = session.total_usage().await.unwrap();
        assert_eq!(totals.total_tokens, 0);
        assert_eq!(totals.cost.total, 0.0);
        assert!(session
            .coding_session_stats()
            .await
            .unwrap()
            .cost_breakdown
            .is_empty());
    }

    #[tokio::test]
    async fn coding_usage_counts_abandoned_branches_but_context_uses_active_branch() {
        let harness = test_harness().await;
        let mut model = Model::new("m", "m", Api::Faux, "faux", "http://x");
        model.context_window = 100;
        harness.set_model(model).await.unwrap();
        let session = AgentSession::new(harness, Vec::new(), Vec::new(), ".");
        let root = session
            .harness()
            .session()
            .append_message(AgentMessage::User(ice_ai::types::UserMessage::new(
                "root", 1,
            )))
            .await
            .unwrap();
        let mut abandoned_usage = usage(80, 0, 0, 0, 1.0);
        abandoned_usage.total_tokens = 80;
        append_assistant(
            session.harness(),
            assistant_message(vec![text_block("abandoned")], abandoned_usage),
        )
        .await;
        session
            .harness()
            .session()
            .move_lane("main", Some(&root))
            .await
            .unwrap();
        session
            .harness()
            .session()
            .append_message(AgentMessage::User(ice_ai::types::UserMessage::new(
                "active", 3,
            )))
            .await
            .unwrap();
        let mut active_usage = usage(120, 3, 0, 0, 2.0);
        active_usage.total_tokens = 123;
        append_assistant(
            session.harness(),
            assistant_message(vec![text_block("active answer")], active_usage),
        )
        .await;

        let stats = session.coding_session_stats().await.unwrap();
        assert_eq!(stats.usage.total_tokens, 203);
        assert_eq!(stats.usage.cost, 3.0);
        let context = stats.context_usage.unwrap();
        assert_eq!(context.tokens, Some(123));
        assert_eq!(context.percent, Some(123.0));
    }

    #[tokio::test]
    async fn context_reads_use_configured_projectors_and_entry_transforms() {
        let mut options = test_harness_options(Vec::new());
        options.model.context_window = 100;
        options.entry_transforms.push(Arc::new(|entries| {
            entries
                .iter()
                .filter(|entry| !matches!(entry, Entry::Message(_)))
                .cloned()
                .collect()
        }));
        options.entry_projectors.insert(
            "visible".to_string(),
            Arc::new(|_, _, _| {
                let mut measured_usage = usage(16, 0, 0, 0, 0.0);
                measured_usage.total_tokens = 16;
                vec![AgentMessage::Assistant(Box::new(assistant_message(
                    vec![text_block("projected answer")],
                    measured_usage,
                )))]
            }),
        );
        let harness = AgentHarness::create(options).await.unwrap();
        let session = AgentSession::new(harness, Vec::new(), Vec::new(), ".");
        session
            .harness()
            .session()
            .append_message(AgentMessage::User(ice_ai::types::UserMessage::new(
                "ignored".repeat(100),
                1,
            )))
            .await
            .unwrap();
        session
            .harness()
            .session()
            .append_custom_entry("visible", Some(serde_json::json!({})))
            .await
            .unwrap();
        assert_eq!(
            session
                .current_context_usage()
                .await
                .unwrap()
                .unwrap()
                .tokens,
            Some(16)
        );
        assert_eq!(
            session.last_assistant_text().await.unwrap().as_deref(),
            Some("projected answer")
        );
        let breakdown = session.context_breakdown().await.unwrap().unwrap();
        assert_eq!(breakdown.measured_tokens, Some(16));
        assert!(breakdown.estimated_tokens < 16);
        assert_eq!(breakdown.used_tokens, 16);
        assert_eq!(session.total_usage().await.unwrap().total_tokens, 0);
    }

    #[tokio::test]
    async fn context_breakdown_reads_live_configuration_and_distinguishes_estimates() {
        use crate::context_breakdown::ContextCategoryKey;
        let harness = test_harness().await;
        let mut model = Model::new("m", "m", Api::Faux, "faux", "http://x");
        model.context_window = 0;
        harness.set_model(model.clone()).await.unwrap();
        let session = AgentSession::new(harness, Vec::new(), Vec::new(), ".");
        assert!(session.context_breakdown().await.unwrap().is_none());
        model.context_window = 100;
        session.set_model(model).await.unwrap();
        session
            .harness()
            .set_system_prompt(Some("12345678".to_string()))
            .await
            .unwrap();
        session
            .harness()
            .session()
            .append_message(AgentMessage::User(ice_ai::types::UserMessage::new(
                "中文😀",
                1,
            )))
            .await
            .unwrap();
        let breakdown = session.context_breakdown().await.unwrap().unwrap();
        assert_eq!(breakdown.context_window, 100);
        assert_eq!(breakdown.measured_tokens, None);
        assert_eq!(breakdown.estimated_tokens, 3);
        assert_eq!(breakdown.free_tokens, 97);
        assert_eq!(
            breakdown
                .categories
                .iter()
                .find(|row| row.key == ContextCategoryKey::SystemPrompt)
                .unwrap()
                .tokens,
            2
        );
    }

    #[tokio::test]
    async fn context_is_unknown_after_compaction_until_positive_usage_and_zero_does_not_clear_gate()
    {
        let harness = test_harness().await;
        let mut model = Model::new("m", "m", Api::Faux, "faux", "http://x");
        model.context_window = 100;
        harness.set_model(model).await.unwrap();
        let session = AgentSession::new(harness, Vec::new(), Vec::new(), ".");
        let mut before_compaction = usage(80, 0, 0, 0, 1.0);
        before_compaction.total_tokens = 80;
        append_assistant(
            session.harness(),
            assistant_message(vec![text_block("before")], before_compaction),
        )
        .await;
        append_usage_entry(
            session.harness(),
            "compact-boundary",
            ice_harness::session::types::ProvisionedKind::Compaction {
                summary: "compact".to_string(),
                retained_tail: Vec::new(),
                tokens_before: 80,
                details: None,
                usage: None,
            },
        )
        .await;
        session
            .harness()
            .session()
            .append_message(AgentMessage::User(ice_ai::types::UserMessage::new(
                "after", 3,
            )))
            .await
            .unwrap();
        for stop_reason in [
            ice_ai::types::StopReason::Aborted,
            ice_ai::types::StopReason::Error,
        ] {
            let mut invalid_gate = assistant_message(
                vec![text_block("not a valid measurement")],
                usage(20, 0, 0, 0, 0.0),
            );
            invalid_gate.stop_reason = stop_reason;
            append_assistant(session.harness(), invalid_gate).await;
        }
        append_assistant(
            session.harness(),
            assistant_message(vec![text_block("zero")], Usage::zero()),
        )
        .await;

        let unknown = session.current_context_usage().await.unwrap().unwrap();
        assert_eq!(unknown.tokens, None);
        assert_eq!(unknown.percent, None);
        assert_eq!(
            session
                .context_breakdown()
                .await
                .unwrap()
                .unwrap()
                .measured_tokens,
            None
        );

        let mut measured = usage(20, 5, 0, 0, 0.0);
        measured.total_tokens = 25;
        append_assistant(
            session.harness(),
            assistant_message(vec![text_block("measured")], measured),
        )
        .await;
        append_assistant(
            session.harness(),
            assistant_message(vec![text_block("later zero")], Usage::zero()),
        )
        .await;
        let known = session.current_context_usage().await.unwrap().unwrap();
        assert!(known.tokens.is_some());
        assert!(known.percent.is_some());
        assert!(session
            .context_breakdown()
            .await
            .unwrap()
            .unwrap()
            .measured_tokens
            .is_some());
    }

    #[tokio::test]
    async fn last_assistant_text_selects_active_context_message_before_extracting_text() {
        let harness = test_harness().await;
        let session = AgentSession::new(harness, Vec::new(), Vec::new(), ".");
        append_assistant(
            session.harness(),
            assistant_message(vec![text_block("older answer")], Usage::zero()),
        )
        .await;
        append_usage_entry(
            session.harness(),
            "text-compaction",
            ice_harness::session::types::ProvisionedKind::Compaction {
                summary: "old answer is out of context".to_string(),
                retained_tail: Vec::new(),
                tokens_before: 5,
                details: None,
                usage: None,
            },
        )
        .await;
        let thinking_only = Content::Thinking(ice_ai::types::ThinkingContent {
            kind: ice_ai::types::ThinkingContentType,
            thinking: "private".to_string(),
            thinking_signature: None,
            redacted: false,
        });
        append_assistant(
            session.harness(),
            assistant_message(vec![thinking_only], Usage::zero()),
        )
        .await;

        assert_eq!(session.last_assistant_text().await.unwrap(), None);
    }

    #[tokio::test]
    async fn last_assistant_text_ignores_assistants_on_abandoned_branches() {
        let harness = test_harness().await;
        let session = AgentSession::new(harness, Vec::new(), Vec::new(), ".");
        let root = session
            .harness()
            .session()
            .append_message(AgentMessage::User(ice_ai::types::UserMessage::new(
                "root", 1,
            )))
            .await
            .unwrap();
        append_assistant(
            session.harness(),
            assistant_message(vec![text_block("abandoned answer")], Usage::zero()),
        )
        .await;
        session
            .harness()
            .session()
            .move_lane("main", Some(&root))
            .await
            .unwrap();
        session
            .harness()
            .session()
            .append_message(AgentMessage::User(ice_ai::types::UserMessage::new(
                "active", 3,
            )))
            .await
            .unwrap();

        assert_eq!(session.last_assistant_text().await.unwrap(), None);
    }

    #[tokio::test]
    async fn last_assistant_text_skips_only_empty_aborted_messages_and_trims_concatenation() {
        let harness = test_harness().await;
        let session = AgentSession::new(harness, Vec::new(), Vec::new(), ".");
        append_assistant(
            session.harness(),
            assistant_message(vec![text_block("  older answer  ")], Usage::zero()),
        )
        .await;
        let mut empty_aborted = assistant_message(Vec::new(), Usage::zero());
        empty_aborted.stop_reason = ice_ai::types::StopReason::Aborted;
        append_assistant(session.harness(), empty_aborted).await;
        assert_eq!(
            session.last_assistant_text().await.unwrap().as_deref(),
            Some("older answer")
        );

        let mut partial = assistant_message(
            vec![text_block("  partial "), text_block("reply  ")],
            Usage::zero(),
        );
        partial.stop_reason = ice_ai::types::StopReason::Aborted;
        append_assistant(session.harness(), partial).await;
        assert_eq!(
            session.last_assistant_text().await.unwrap().as_deref(),
            Some("partial reply")
        );
    }

    #[tokio::test]
    async fn retry_user_lookup_uses_only_active_branch_context() {
        let harness = test_harness().await;
        let session = AgentSession::new(harness, Vec::new(), Vec::new(), ".");
        let root = session
            .harness()
            .session()
            .append_message(AgentMessage::User(ice_ai::types::UserMessage::new(
                "root", 1,
            )))
            .await
            .unwrap();
        session
            .harness()
            .session()
            .append_message(AgentMessage::User(ice_ai::types::UserMessage::new(
                "abandoned",
                2,
            )))
            .await
            .unwrap();
        session
            .harness()
            .session()
            .move_lane("main", Some(&root))
            .await
            .unwrap();
        session
            .harness()
            .session()
            .append_message(AgentMessage::User(ice_ai::types::UserMessage::new(
                "active", 3,
            )))
            .await
            .unwrap();

        let AgentMessage::User(last_user) = session.last_user_message().await.unwrap().unwrap()
        else {
            panic!("expected a user message");
        };
        assert_eq!(
            last_user.content,
            ice_ai::types::UserContent::Text("active".to_string())
        );
    }

    fn user_texts(context: &ice_harness::session::context::SessionContext) -> Vec<String> {
        context
            .messages
            .iter()
            .filter_map(|message| match message {
                AgentMessage::User(user) => match &user.content {
                    ice_ai::types::UserContent::Text(text) => Some(text.clone()),
                    _ => None,
                },
                _ => None,
            })
            .collect()
    }

    #[tokio::test]
    async fn active_context_follows_fork_and_tree_navigation() {
        let harness = test_harness().await;
        let session = AgentSession::new(harness, Vec::new(), Vec::new(), ".");
        let append = |text: &'static str, ts: i64| {
            let storage = session.harness().session();
            async move {
                storage
                    .append_message(AgentMessage::User(ice_ai::types::UserMessage::new(
                        text, ts,
                    )))
                    .await
                    .unwrap()
            }
        };
        let root = append("root", 1).await;
        append("implementation A", 2).await;

        // Forking at the current leaf keeps the visible conversation intact.
        session.fork(Some("checkpoint")).await.unwrap();
        assert_eq!(
            user_texts(&session.active_context().await.unwrap()),
            vec!["root", "implementation A"]
        );

        // Navigating back to the root and continuing hides the A-only message.
        session
            .harness()
            .navigate_tree(Some(&root), false, None, None)
            .await
            .unwrap();
        append("implementation B", 3).await;
        assert_eq!(
            user_texts(&session.active_context().await.unwrap()),
            vec!["root", "implementation B"]
        );
    }

    #[tokio::test]
    async fn session_values_round_trip_through_the_product_layer() {
        let session = AgentSession::new(test_harness().await, Vec::new(), Vec::new(), ".");
        assert!(session
            .value("display.hide_thinking")
            .await
            .unwrap()
            .is_none());

        session
            .set_value("display.hide_thinking", serde_json::json!(true))
            .await
            .unwrap();
        assert_eq!(
            session.value("display.hide_thinking").await.unwrap(),
            Some(serde_json::json!(true))
        );

        // Latest-wins.
        session
            .set_value("display.hide_thinking", serde_json::json!(false))
            .await
            .unwrap();
        assert_eq!(
            session.value("display.hide_thinking").await.unwrap(),
            Some(serde_json::json!(false))
        );
    }

    #[tokio::test]
    async fn usage_stats_and_snapshot_read_from_the_empty_session() {
        let session = AgentSession::new(test_harness().await, Vec::new(), Vec::new(), ".");
        let stats = session.usage_stats().await.unwrap();
        assert_eq!(stats.message_count, 0);
        assert_eq!(stats.total_tokens, 0);
        assert!(stats.cache_hit_ratio().is_none());

        let snapshot = session.snapshot().await.unwrap();
        assert_eq!(snapshot.lane, "main");
        assert!(snapshot.active.is_none());
    }

    #[tokio::test]
    async fn export_writes_every_supported_format() {
        let session = AgentSession::new(test_harness().await, Vec::new(), Vec::new(), ".");
        let dir = tempfile::tempdir().unwrap();

        for (format, ext) in [
            (ExportFormat::Markdown, "md"),
            (ExportFormat::Html, "html"),
            (ExportFormat::Jsonl, "jsonl"),
        ] {
            let name = session.default_export_file_name(format.clone()).await;
            assert!(name.ends_with(&format!(".{ext}")), "unexpected name {name}");
            let path = dir.path().join(&name);
            session.export(format, &path).await.unwrap();
            assert!(path.exists(), "{ext} export missing at {}", path.display());
        }
    }
}
