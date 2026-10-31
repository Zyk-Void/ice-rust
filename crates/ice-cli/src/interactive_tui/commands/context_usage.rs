//! Read-only public-API context projection. Never rereads instruction files.
use super::*;
use ice_agent::message::AgentMessage;
use ice_harness::compaction::{build_session_context, estimate_context_tokens, estimate_tokens};
use ice_harness::messages as roles;
use ice_harness::session::types::{BranchBounds, Entry, EntryOrder, EntryQuery};

const CATEGORIES: [&str; 9] = [
    "System prompt",
    "Project instructions",
    "Skills listing",
    "Tool schemas",
    "Conversation",
    "Tool results",
    "Extension messages",
    "Summaries",
    "Shell output",
];
const SYMBOLS: [char; 9] = ['S', 'P', 'K', 'T', 'C', 'R', 'E', 'M', 'B'];

#[derive(Default)]
struct Snapshot {
    model: String,
    window: u64,
    used: u64,
    provider: Option<u64>,
    tail: u64,
    categories: [u64; 9],
    items: Vec<(String, u64, usize)>,
}

fn byte_tokens(bytes: usize) -> u64 {
    (bytes as u64 + 3) / 4
}

impl Snapshot {
    fn add(&mut self, category: usize, label: impl Into<String>, tokens: u64) {
        self.categories[category] = self.categories[category].saturating_add(tokens);
        if tokens > 0 {
            self.items.push((label.into(), tokens, category));
        }
    }

    fn prompt(&mut self, base: Option<&str>, skills: &[ice_harness::types::Skill]) {
        let composed = ice_harness::system_prompt::compose_system_prompt(base, skills, None, None);
        let base = base.unwrap_or("").trim();
        let mut rest = base;
        let mut project_bytes = 0;
        // Attribute the loaded block, not files currently present on disk.
        // Show loaded paths, never instruction contents.
        while let Some(start) = rest.find("<project_context>") {
            let after = &rest[start..];
            let Some(end) = after.find("</project_context>") else {
                break;
            };
            let end = end + "</project_context>".len();
            project_bytes += end;
            let block = &after[..end];
            let mut instructions = block;
            let mut attributed = 0;
            let mut number = 0;
            while let Some(start) = instructions.find("<project_instructions ") {
                let part = &instructions[start..];
                let Some(close) = part.find("</project_instructions>") else {
                    break;
                };
                let length = close + "</project_instructions>".len();
                number += 1;
                attributed += length;
                let label = part
                    .split_once('>')
                    .and_then(|(header, _)| header.split_once("path=\""))
                    .and_then(|(_, path)| path.split_once('"'))
                    .map(|(path, _)| display_label(path))
                    .filter(|path| !path.is_empty())
                    .unwrap_or_else(|| format!("loaded project instructions #{number}"));
                self.add(1, label, byte_tokens(length));
                instructions = &part[length..];
            }
            self.add(
                1,
                "loaded project context",
                byte_tokens(end.saturating_sub(attributed)),
            );
            rest = &after[end..];
        }
        self.add(
            0,
            "system prompt",
            byte_tokens(base.len().saturating_sub(project_bytes)),
        );
        // Attribute the exact loaded listing, not skill bodies or current files.
        let listing = ice_harness::skills::format_skills_for_system_prompt(skills);
        let mut attributed = 0;
        for (skill, block) in skills
            .iter()
            .filter(|skill| !skill.disable_model_invocation.unwrap_or(false))
            .zip(listing.split("  <skill>\n").skip(1))
        {
            if let Some(end) = block.find("  </skill>") {
                let size = "  <skill>\n".len() + end + "  </skill>\n".len();
                attributed += size;
                self.add(
                    2,
                    format!("{} listing", display_label(&skill.name)),
                    byte_tokens(size),
                );
            }
        }
        // Preserve aggregate rounding; per-item token estimates may round up.
        let listing_tokens = byte_tokens(composed.len().saturating_sub(base.len()));
        self.add(
            2,
            "skills listing framing",
            byte_tokens(
                composed
                    .len()
                    .saturating_sub(base.len())
                    .saturating_sub(attributed),
            ),
        );
        self.categories[2] = listing_tokens;
    }

    fn tools(
        &mut self,
        tools: &[ice_harness::types::HarnessTool],
        active: &[String],
    ) -> Result<(), String> {
        for tool in tools {
            let schema = tool.tool.schema();
            // Harness semantics: an empty selection enables all registered tools.
            if active.is_empty() || active.contains(&schema.name) {
                let encoded = serde_json::to_string(&schema).map_err(|e| e.to_string())?;
                self.add(
                    3,
                    format!("{} schema", display_label(&schema.name)),
                    byte_tokens(encoded.len()),
                );
            }
        }
        Ok(())
    }

    fn messages(&mut self, entries: &[Entry]) {
        self.projected_messages(build_session_context(entries).messages);
    }

    fn projected_messages(&mut self, projected: Vec<AgentMessage>) {
        let mut visible = Vec::new();
        for message in projected {
            // Public conversion is authoritative for custom-role inclusion,
            // including excluded shell records and malformed/unknown roles.
            if matches!(message, AgentMessage::Custom(_))
                && roles::convert_to_llm(vec![message.clone()]).is_empty()
            {
                continue;
            }
            let (category, label) = match &message {
                AgentMessage::User(_) => (4, "user".to_string()),
                AgentMessage::Assistant(_) => (4, "assistant".to_string()),
                AgentMessage::ToolResult(t) => (5, format!("{} result", safe_label(&t.tool_name))),
                AgentMessage::Custom(c) => match c.role.as_str() {
                    roles::BASH_EXECUTION_ROLE => (8, "shell output".to_string()),
                    roles::COMPACTION_SUMMARY_ROLE => (7, "compaction summary".to_string()),
                    roles::BRANCH_SUMMARY_ROLE => (7, "branch summary".to_string()),
                    _ => (6, "extension message".to_string()),
                },
            };
            self.add(category, label, estimate_tokens(&message).max(0) as u64);
            visible.push(message);
        }
        // Normalize to INPUT + cache only, never cumulative billable totals.
        // The measured assistant's estimated output belongs in the tail, since
        // input usage describes the request immediately BEFORE that response.
        for message in &mut visible {
            if let AgentMessage::Assistant(a) = message {
                let input = a
                    .usage
                    .input
                    .max(0)
                    .saturating_add(a.usage.cache_read.max(0))
                    .saturating_add(a.usage.cache_write.max(0));
                a.usage.total_tokens = input;
                a.usage.input = input;
                a.usage.output = 0;
                a.usage.cache_read = 0;
                a.usage.cache_write = 0;
            }
        }
        let estimate = estimate_context_tokens(&visible);
        self.provider = estimate
            .last_usage_index
            .map(|_| estimate.usage_tokens.max(0) as u64);
        self.tail = estimate.trailing_tokens.max(0) as u64;
        if let Some(index) = estimate.last_usage_index {
            self.tail = self
                .tail
                .saturating_add(estimate_tokens(&visible[index]).max(0) as u64);
        }
        let static_tokens: u64 = self.categories[..4].iter().sum();
        self.used = self
            .provider
            .unwrap_or(static_tokens)
            .saturating_add(self.tail);
    }

    fn largest(&self) -> Vec<&(String, u64, usize)> {
        let mut largest: Vec<_> = self.items.iter().collect();
        largest.sort_by(|a, b| b.1.cmp(&a.1)); // stable original-position tie break
        largest
    }
}

impl Component for Snapshot {
    fn invalidate(&self) {}

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn render(&self, width: usize) -> Vec<String> {
        render_report(self, width)
    }
}

fn render_report(snapshot: &Snapshot, width: usize) -> Vec<String> {
    use ice_tui::theme::{theme, Color};
    let colors = theme().colors;
    let content_width = width.saturating_sub(4).min(100);
    if content_width == 0 {
        return vec![String::new()];
    }
    // Three stable hues plus letter markers: identity never depends on color.
    // Long-tail categories share the neutral mark instead of cycling hues.
    let palette = if colors.surface == Color::Ansi256(254) {
        [
            Color::Rgb(42, 120, 214),
            Color::Rgb(235, 104, 52),
            Color::Rgb(27, 175, 122),
        ]
    } else if colors.accent == Color::Ansi256(15) {
        [colors.text; 3]
    } else {
        [
            Color::Rgb(57, 135, 229),
            Color::Rgb(217, 89, 38),
            Color::Rgb(25, 158, 112),
        ]
    };
    let category_color = |category: usize| match category {
        0..=2 => palette[0],
        3 => palette[1],
        4 => palette[2],
        _ => colors.muted,
    };
    let mark = |category: usize| category_color(category).fg(&SYMBOLS[category].to_string());
    let percent = |tokens: u64| {
        if snapshot.window == 0 {
            "—".into()
        } else {
            format!("{:.1}%", tokens as f64 / snapshot.window as f64 * 100.0)
        }
    };
    let mut lines = vec![
        colors
            .accent
            .fg(&format!("Context Usage · {}", safe_label(&snapshot.model))),
        colors.text.fg(&format!(
            "~{} / {} tokens  ({})",
            token_count(snapshot.used),
            if snapshot.window == 0 {
                "unknown".into()
            } else {
                token_count(snapshot.window)
            },
            percent(snapshot.used)
        )),
        colors
            .muted
            .fg("Estimated active context · not cumulative billing"),
    ];
    // Keep the grid beside the legend on ordinary terminal widths too.
    let side_by_side = content_width >= 64;
    let grid_width = if side_by_side {
        content_width.saturating_sub(48)
    } else {
        content_width
    };
    let columns = ((grid_width + 1) / 2).clamp(1, 20);
    let budget = columns * 10;
    let counts = allocate(&snapshot.categories, snapshot.used, snapshot.window, budget);
    let mut cells = Vec::with_capacity(budget);
    for (category, count) in counts.iter().enumerate() {
        cells.extend(std::iter::repeat(mark(category)).take(*count));
    }
    cells.resize(budget, colors.muted.fg("·"));
    let grid: Vec<_> = cells.chunks(columns).map(|row| row.join(" ")).collect();
    let mut legend: Vec<_> = CATEGORIES
        .iter()
        .enumerate()
        .map(|(i, name)| {
            format!(
                "{} {:<21} {:>9}  {:>6}",
                mark(i),
                name,
                format!("~{}", token_count(snapshot.categories[i])),
                percent(snapshot.categories[i])
            )
        })
        .collect();
    legend.push(format!(
        "{} {:<21} {:>9}  {:>6}",
        colors.muted.fg("·"),
        "Free space",
        token_count(snapshot.window.saturating_sub(snapshot.used)),
        percent(snapshot.window.saturating_sub(snapshot.used))
    ));
    if side_by_side {
        for (row, label) in grid.iter().zip(&legend) {
            lines.push(format!("{row}  {}", colors.text.fg(label)));
        }
    } else {
        lines.extend(grid);
        lines.push(String::new());
        lines.extend(legend.into_iter().map(|label| colors.text.fg(&label)));
    }
    lines.push(String::new());
    lines.push(colors.accent.fg(&format!(
        "Context consumers · {} entries",
        snapshot.items.len()
    )));
    lines.push(
        colors
            .muted
            .fg("Category estimates · every loaded contributor · Tokens per item"),
    );
    let sorted = snapshot.largest();
    for (category, name) in CATEGORIES.iter().enumerate() {
        let items: Vec<_> = sorted
            .iter()
            .copied()
            .filter(|item| item.2 == category)
            .collect();
        if items.is_empty() {
            continue;
        }
        lines.push(colors.text.fg(&format!(
            "{} {name}  ·  ~{} tokens",
            mark(category),
            token_count(snapshot.categories[category])
        )));
        let name_width = content_width.saturating_sub(16).max(1);
        for (label, tokens, _) in items {
            let wrapped = ice_tui::wrap_text_with_ansi(label, name_width);
            for (index, part) in wrapped.iter().enumerate() {
                let padding = name_width.saturating_sub(ice_tui::visible_width(part));
                lines.push(colors.text.fg(&format!(
                    "  {part}{} {:>10}",
                    " ".repeat(padding),
                    if index == 0 {
                        format!("~{}", token_count(*tokens))
                    } else {
                        String::new()
                    }
                )));
            }
        }
    }
    lines.push(String::new());
    lines.push(colors.muted.fg("Composition estimates use ~4 bytes/token. Grid shares are scaled to the used total; category estimates may differ from provider usage."));
    if let Some(input) = snapshot.provider {
        lines.push(colors.muted.fg(&format!(
            "Provider input/cache: {}; estimated output + tail: {}.",
            token_count(input),
            token_count(snapshot.tail)
        )));
    } else {
        lines.push(
            colors
                .muted
                .fg("No valid provider input usage; prompt, tools and messages estimated."),
        );
    }
    lines.push(colors.muted.fg("Public projection: active branch + standard compaction. Not an exact wire payload; private request transforms, custom projectors and prompt overrides are unavailable."));
    lines.push(colors.muted.fg("Provider input may reflect earlier prompt/tools; /usage retains billing totals. Skill bodies and custom agents count only when loaded into the prompt or messages."));
    let mut output = vec![String::new()];
    for line in lines {
        for wrapped in ice_tui::wrap_text_with_ansi(&line, content_width) {
            output.push(format!("  {wrapped}"));
        }
    }
    output.push(String::new());
    output
}

fn token_count(tokens: u64) -> String {
    let digits = tokens.to_string();
    let mut formatted = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            formatted.push(',');
        }
        formatted.push(ch);
    }
    formatted
}

fn display_label(label: &str) -> String {
    label
        .chars()
        .filter(|ch| !ch.is_control() && *ch != '\u{1b}')
        .collect()
}

fn safe_label(label: &str) -> String {
    label
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '/' | '.' | ':'))
        .take(64)
        .collect()
}

/// Bounded largest-remainder apportionment. Preserve one cell per nonempty
/// category when possible; reclaim excess minima before adding remainders.
fn allocate(weights: &[u64], used: u64, window: u64, budget: usize) -> Vec<usize> {
    let sum: u128 = weights.iter().map(|v| *v as u128).sum();
    let mut cells = vec![0; weights.len()];
    if sum == 0 || used == 0 || budget == 0 {
        return cells;
    }
    let nonempty = weights.iter().filter(|v| **v > 0).count();
    let target = if window == 0 {
        budget
    } else {
        (((used as u128 * budget as u128 + window as u128 / 2) / window as u128).min(budget as u128)
            as usize)
            .max(nonempty.min(budget))
    };
    let mut order: Vec<_> = weights
        .iter()
        .enumerate()
        .filter(|(_, v)| **v > 0)
        .map(|(i, v)| (i, *v as u128 * target as u128 % sum))
        .collect();
    for &(i, _) in &order {
        cells[i] = ((weights[i] as u128 * target as u128 / sum) as usize).max(1);
    }
    let mut total: usize = cells.iter().sum();
    while total > target {
        let i = if cells.iter().any(|count| *count > 1) {
            (0..cells.len())
                .filter(|i| cells[*i] > 1)
                .max_by_key(|i| (cells[*i], weights[*i], std::cmp::Reverse(*i)))
                .unwrap()
        } else {
            (0..cells.len())
                .filter(|i| cells[*i] > 0)
                .min_by_key(|i| (weights[*i], std::cmp::Reverse(*i)))
                .unwrap()
        };
        cells[i] -= 1;
        total -= 1;
    }
    order.sort_by(|a, b| b.1.cmp(&a.1));
    for &(i, _) in order.iter().cycle().take(target - total) {
        cells[i] += 1;
    }
    cells
}

/// Storage identity also rejects replacement sessions with reused metadata IDs.
struct Fence {
    storage: Arc<dyn ice_harness::session::types::SessionStorage>,
    tip: Option<String>,
}

impl Fence {
    async fn capture(
        session: &crate::agent_session::AgentSession,
        lane: &Arc<dyn AgentLane>,
    ) -> Result<Self, String> {
        let storage = session.harness().session().storage();
        let tip = lane.get_tip_id().await.map_err(|e| e.to_string())?;
        let fence = Self { storage, tip };
        if !fence.current(session, lane).await {
            return Err("Context changed".into());
        }
        Ok(fence)
    }

    async fn current(
        &self,
        session: &crate::agent_session::AgentSession,
        lane: &Arc<dyn AgentLane>,
    ) -> bool {
        let tip = match lane.get_tip_id().await {
            Ok(tip) => tip,
            Err(_) => return false,
        };
        tip == self.tip && Arc::ptr_eq(&self.storage, &session.harness().session().storage())
    }
}

#[cfg(test)]
async fn collect(
    session: &crate::agent_session::AgentSession,
    lane: &Arc<dyn AgentLane>,
) -> Result<Snapshot, String> {
    let fence = Fence::capture(session, lane).await?;
    collect_at(session, lane, &fence).await
}

async fn collect_at(
    session: &crate::agent_session::AgentSession,
    lane: &Arc<dyn AgentLane>,
    fence: &Fence,
) -> Result<Snapshot, String> {
    if !fence.current(session, lane).await {
        return Err("Context changed".into());
    }
    let model = session.model().await.map_err(|e| e.to_string())?;
    let snapshot = collect_from(session.harness(), lane, &model, fence.tip.clone()).await?;
    if !fence.current(session, lane).await {
        return Err("Context changed".into());
    }
    Ok(snapshot)
}

/// Build the report for `model` from the branch ending at `tip`.
async fn collect_from(
    harness: &ice_harness::agent_harness::AgentHarness,
    lane: &Arc<dyn AgentLane>,
    model: &ice_ai::Model,
    tip: Option<String>,
) -> Result<Snapshot, String> {
    let active = lane.get_active_tools().await.map_err(|e| e.to_string())?;
    let base = harness
        .get_system_prompt()
        .await
        .map_err(|e| e.to_string())?;
    let resources = harness.get_resources().await.map_err(|e| e.to_string())?;
    let tools = harness.get_tools().await.map_err(|e| e.to_string())?;
    let entries = if tip.is_some() {
        harness
            .find_entries_on_branch(
                &EntryQuery {
                    order: Some(EntryOrder::OldestFirst),
                    ..Default::default()
                },
                &BranchBounds {
                    start: tip,
                    ..Default::default()
                },
            )
            .await
            .map_err(|e| e.to_string())?
    } else {
        Vec::new()
    };
    let mut snapshot = Snapshot {
        model: format!("{}/{}", model.provider, model.id),
        window: model.context_window as u64,
        ..Default::default()
    };
    let skills = if active.iter().any(|name| name == "read") {
        resources.skills.as_deref().unwrap_or(&[])
    } else {
        &[]
    };
    snapshot.prompt(base.as_deref(), skills);
    snapshot.tools(&tools, &active)?;
    snapshot.messages(&entries);
    Ok(snapshot)
}

/// The `/context` report as plain text, for headless sessions (`--mode rpc`,
/// `--acp`): the same snapshot and layout the TUI renders, without ANSI styling.
pub(crate) async fn headless_report(
    harness: &ice_harness::agent_harness::AgentHarness,
    lane: &Arc<dyn AgentLane>,
    width: usize,
) -> Result<String, String> {
    let model = lane.get_model().await.map_err(|e| e.to_string())?;
    let tip = lane.get_tip_id().await.map_err(|e| e.to_string())?;
    let snapshot = collect_from(harness, lane, &model, tip).await?;
    Ok(render_report(&snapshot, width)
        .iter()
        .map(|line| ice_tui::utils::strip_ansi(line).trim_end().to_string())
        .collect::<Vec<_>>()
        .join("\n"))
}

pub(super) fn show(ctx: &CommandContext) {
    use std::sync::atomic::Ordering;
    if ctx
        .state
        .event_drain
        .session_changing
        .load(Ordering::Acquire)
    {
        return;
    }
    // Capture backing identity at command admission, not when the task happens
    // to start. No live transcript state or message contents are captured here.
    let storage = ctx.session.harness().session().storage();
    let session = ctx.session.clone();
    let lane = ctx.lane.clone();
    let chat = ctx.chat.clone();
    let tui = ctx.tui.clone();
    let state = ctx.state.clone();
    tokio::spawn(async move {
        let report = |error: String| {
            add_error_message(&chat, &format!("Could not read context usage: {error}"));
            tui.request_render(false);
        };
        let fence = match Fence::capture(&session, &lane).await {
            Ok(fence) => fence,
            Err(error) => return report(error.to_string()),
        };
        if !Arc::ptr_eq(&storage, &fence.storage) {
            return;
        }
        let snapshot = match collect_at(&session, &lane, &fence).await {
            Ok(snapshot) => snapshot,
            Err(error) => return report(error.to_string()),
        };
        if !fence.current(&session, &lane).await {
            return;
        }
        // Session transitions claim admission under this same mutex. Keep the
        // final flag/backing check and publication synchronous under it, so a
        // /new or tree transition cannot begin in the check-to-append gap.
        let _admission = state.user_bash_cancel.lock().unwrap();
        if state.event_drain.session_changing.load(Ordering::Acquire)
            || !Arc::ptr_eq(&storage, &session.harness().session().storage())
        {
            return;
        }
        chat.add_child(Arc::new(snapshot));
        tui.request_render(false);
    });
}

#[cfg(test)]
#[path = "context_usage_tests.rs"]
mod tests;
