//! `/usage` panel: ChatGPT plan limits plus this session's token and cost
//! totals, drawn with theme colors, bars and a block-letter banner.

use super::*;
use crate::codex_usage::{ChatGptUsage, UsageWindow};

const BAR_CELLS: usize = 24;
const BANNER: [&str; 2] = ["█ █ █▀ ▄▀█ █▀▀ █▀▀", "█▄█ ▄█ █▀█ █▄█ ██▄"];

/// What the ChatGPT section shows.
pub(super) enum ChatGptSection {
    /// No ChatGPT login: point at `/login`.
    SignedOut,
    Usage(ChatGptUsage),
    Failed(String),
}

/// Session totals gathered from the product-layer `AgentSession`.
#[derive(Default)]
pub(super) struct SessionSection {
    pub(super) messages: u64,
    pub(super) cached_tokens: i64,
    pub(super) uncached_tokens: i64,
    pub(super) cost: f64,
    pub(super) all_history: Option<AllHistory>,
    pub(super) footer: Vec<String>,
}

pub(super) struct AllHistory {
    pub(super) input: i64,
    pub(super) output: i64,
    pub(super) cache_read: i64,
    pub(super) cache_write: i64,
    pub(super) total_tokens: i64,
    pub(super) cost: f64,
    /// (tokens, window, percent) of the active context, when known.
    pub(super) context: Option<ContextOccupancy>,
}

pub(super) struct ContextOccupancy {
    pub(super) tokens: Option<i64>,
    pub(super) window: u64,
    pub(super) percent: Option<f64>,
}

pub(super) struct UsagePanel {
    pub(super) chatgpt: ChatGptSection,
    pub(super) session: SessionSection,
}

/// `(filled, empty)` cell counts for a percentage; any nonzero use shows at
/// least one filled cell.
fn bar_cells(percent: f64, cells: usize) -> (usize, usize) {
    let percent = percent.clamp(0.0, 100.0);
    let mut filled = ((percent / 100.0) * cells as f64).round() as usize;
    if percent > 0.0 {
        filled = filled.max(1);
    }
    let filled = filled.min(cells);
    (filled, cells - filled)
}

fn format_duration(seconds: u64) -> String {
    let (days, hours, minutes) = (
        seconds / 86_400,
        seconds % 86_400 / 3_600,
        seconds % 3_600 / 60,
    );
    match (days, hours, minutes) {
        (0, 0, 0) => "<1m".into(),
        (0, 0, m) => format!("{m}m"),
        (0, h, m) => format!("{h}h {m}m"),
        (d, h, _) => format!("{d}d {h}h"),
    }
}

/// "5h", "weekly", "3d"; falls back to "window" when the server sends none.
fn window_label(seconds: u64) -> String {
    match seconds {
        0 => "window".into(),
        604_800 => "weekly".into(),
        s if s % 86_400 == 0 => format!("{}d", s / 86_400),
        s if s % 3_600 == 0 => format!("{}h", s / 3_600),
        s => format!("{}m", s / 60),
    }
}

fn plan_label(plan: &str) -> String {
    let mut chars = plan.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

fn thousands(value: i64) -> String {
    let digits = value.unsigned_abs().to_string();
    let mut out = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            out.push(',');
        }
        out.push(digit);
    }
    if value < 0 {
        out.insert(0, '-');
    }
    out
}

impl UsagePanel {
    fn bar(&self, percent: f64) -> String {
        let c = current_theme().colors;
        let color = if percent >= 85.0 {
            &c.error
        } else if percent >= 60.0 {
            &c.warning
        } else {
            &c.success
        };
        let (filled, empty) = bar_cells(percent, BAR_CELLS);
        format!(
            "{}{}",
            color.fg(&"█".repeat(filled)),
            c.dim.fg(&"░".repeat(empty))
        )
    }

    fn window_line(&self, label: &str, window: &UsageWindow) -> String {
        let c = current_theme().colors;
        format!(
            "  {} {} {:>3.0}%  {}",
            c.text.fg(&format!("{label:<8}")),
            self.bar(window.used_percent),
            window.used_percent,
            c.muted.fg(&format!(
                "resets in {}",
                format_duration(window.resets_in_seconds)
            ))
        )
    }

    fn heading(&self, title: &str, detail: &str) -> String {
        let c = current_theme().colors;
        format!(
            "{} {}",
            c.accent.fg(&format!("━━ {title}")),
            c.muted.fg(detail)
        )
    }

    fn chatgpt_lines(&self) -> Vec<String> {
        let c = current_theme().colors;
        match &self.chatgpt {
            ChatGptSection::SignedOut => vec![
                self.heading("ChatGPT", "openai-codex"),
                format!(
                    "  {}",
                    c.muted
                        .fg("Not signed in. Use /login to see your plan limits here.")
                ),
            ],
            ChatGptSection::Failed(error) => vec![
                self.heading("ChatGPT", "openai-codex"),
                format!(
                    "  {}",
                    c.error.fg(&format!("Could not fetch usage: {error}"))
                ),
            ],
            ChatGptSection::Usage(usage) => {
                let mut lines =
                    vec![self.heading("ChatGPT", &format!("{} plan", plan_label(&usage.plan)))];
                for limit in &usage.limits {
                    if usage.limits.len() > 1 || limit.limit_reached {
                        let reached = if limit.limit_reached {
                            format!("  {}", c.error.fg("LIMIT REACHED"))
                        } else {
                            String::new()
                        };
                        lines.push(format!("  {}{reached}", c.text.fg(&limit.name)));
                    }
                    for window in &limit.windows {
                        lines.push(self.window_line(&window_label(window.window_seconds), window));
                    }
                }
                if let Some(credits) = &usage.credits {
                    let text = match (&credits.balance, credits.unlimited) {
                        (_, true) => "unlimited".to_string(),
                        (Some(balance), false) => format!("{balance} remaining"),
                        (None, false) => "available".to_string(),
                    };
                    lines.push(format!("  {} {}", c.text.fg("Credits "), c.info.fg(&text)));
                }
                lines
            }
        }
    }

    fn session_lines(&self) -> Vec<String> {
        let c = current_theme().colors;
        let session = &self.session;
        let mut lines =
            vec![self.heading("This session", &format!("{} messages", session.messages))];
        let input = session.cached_tokens + session.uncached_tokens;
        if input > 0 {
            let ratio = session.cached_tokens as f64 / input as f64 * 100.0;
            lines.push(format!(
                "  {} {} {:>3.0}%  {}",
                c.text.fg("Cache   "),
                self.bar(ratio),
                ratio,
                c.muted.fg("of input tokens served from cache")
            ));
        } else {
            lines.push(format!(
                "  {}",
                c.muted.fg("Cache hit ratio: n/a (no input yet)")
            ));
        }
        lines.push(format!(
            "  {} {}",
            c.text.fg("Cost    "),
            c.info.fg(&format!("${:.4}", session.cost))
        ));
        if let Some(all) = &session.all_history {
            lines.push(String::new());
            lines.push(self.heading("All history, including abandoned branches", ""));
            lines.push(format!(
                "  {} tokens  {}",
                c.text.fg(&thousands(all.total_tokens)),
                c.muted.fg(&format!(
                    "in {} · out {} · cache read {} · cache write {}",
                    thousands(all.input),
                    thousands(all.output),
                    thousands(all.cache_read),
                    thousands(all.cache_write)
                ))
            ));
            lines.push(format!(
                "  {} {}",
                c.info.fg(&format!("${:.4}", all.cost)),
                c.muted.fg("provider-reported, not a local estimate")
            ));
            if let Some(context) = &all.context {
                match (context.tokens, context.percent) {
                    (Some(tokens), Some(percent)) => lines.push(format!(
                        "  {} {} {:>3.0}%  {}",
                        c.text.fg("Context "),
                        self.bar(percent),
                        percent,
                        c.muted.fg(&format!(
                            "active context (estimated): {} / {} tokens",
                            thousands(tokens),
                            thousands(context.window as i64)
                        ))
                    )),
                    _ => lines.push(format!(
                        "  {}",
                        c.muted.fg(&format!(
                            "active context: unknown until the model replies after compaction (window {})",
                            thousands(context.window as i64)
                        ))
                    )),
                }
            }
        }
        for line in &session.footer {
            lines.push(format!("  {}", c.dim.fg(line)));
        }
        lines
    }
}

impl Component for UsagePanel {
    fn invalidate(&self) {}

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn render(&self, width: usize) -> Vec<String> {
        let c = current_theme().colors;
        let mut lines = Vec::new();
        for (row, art) in BANNER.iter().enumerate() {
            let subtitle = match row {
                0 => c.muted.fg("  plan limits & session totals"),
                _ => String::new(),
            };
            lines.push(format!("  {}{subtitle}", c.accent.fg(art)));
        }
        lines.push(String::new());
        lines.extend(self.chatgpt_lines());
        lines.push(String::new());
        lines.extend(self.session_lines());
        lines.push(String::new());
        if width == 0 {
            return vec![String::new(); lines.len()];
        }
        lines
            .into_iter()
            .map(|line| ice_tui::truncate_to_width(&line, width, ""))
            .collect()
    }
}

#[cfg(test)]
#[path = "usage_panel_tests.rs"]
mod tests;

/// This session's totals for the panel, read through the product-layer
/// session. `None` when the usage stats cannot be read.
pub(super) async fn session_section(
    agent_session: &crate::agent_session::AgentSession,
) -> Option<SessionSection> {
    let stats = agent_session.usage_stats().await.ok()?;
    let snapshot = agent_session.snapshot().await.ok();
    let all_history = agent_session
        .coding_session_stats()
        .await
        .ok()
        .map(|coding| AllHistory {
            input: coding.usage.input,
            output: coding.usage.output,
            cache_read: coding.usage.cache_read,
            cache_write: coding.usage.cache_write,
            total_tokens: coding.usage.total_tokens,
            cost: coding.usage.cost,
            context: coding.context_usage.map(|usage| ContextOccupancy {
                tokens: usage.tokens,
                window: usage.context_window,
                percent: usage.percent,
            }),
        });
    let mut footer = Vec::new();
    if let Some(snapshot) = snapshot {
        footer.push(format!(
            "lane {} · leaf {}",
            snapshot.lane,
            snapshot.leaf_id.clone().unwrap_or_else(|| "(empty)".into())
        ));
        if let Some(active) = &snapshot.active {
            footer.push(format!(
                "active operation: {} ({})",
                active.kind.as_str(),
                active.run_id
            ));
        }
    }
    Some(SessionSection {
        messages: stats.message_count,
        cached_tokens: stats.cached_tokens,
        uncached_tokens: stats.uncached_tokens,
        cost: stats.cost_total,
        all_history,
        footer,
    })
}

/// ChatGPT plan limits from the backend (signed out when offline).
pub(super) async fn chatgpt_section() -> ChatGptSection {
    if crate::args::offline_env_enabled() {
        return ChatGptSection::SignedOut;
    }
    match crate::codex_usage::fetch(
        crate::provider::OPENAI_CODEX_BASE_URL,
        &tokio_util::sync::CancellationToken::new(),
    )
    .await
    {
        Ok(Some(usage)) => ChatGptSection::Usage(usage),
        Ok(None) => ChatGptSection::SignedOut,
        Err(error) => ChatGptSection::Failed(error),
    }
}

/// The `/usage` panel as plain text, for headless sessions.
pub(crate) async fn headless_report(
    agent_session: &crate::agent_session::AgentSession,
    width: usize,
) -> Result<String, String> {
    let session = session_section(agent_session)
        .await
        .ok_or("Could not read session usage stats.")?;
    let panel = UsagePanel {
        chatgpt: chatgpt_section().await,
        session,
    };
    Ok(super::plain_lines(&panel.render(width)))
}
