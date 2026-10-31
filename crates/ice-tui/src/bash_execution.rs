//! Bash execution component — streaming display port of `bash-execution.ts`.
//!
//! This is a **display-only** consumer of the bash tool's existing `on_update`
//! stream: it renders a command header, a spinner while running, a tailed
//! preview of captured output, and a status/truncation line on completion.
//! There is no async/signal plumbing — the caller drives lifecycle via
//! `append_output` / `set_complete`.
//!
//! It backs both the `bash` tool panels and the user-initiated `!command` /
//! `!!command` shell mode. Commands submitted with `!!` are excluded from the
//! model context; those panels render with the dim border/header color so the
//! distinction is visible, matching `bash-execution.ts`.

use std::any::Any;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::component::Component;
use crate::ansi::bold;
use crate::ansi::strip_ansi;
use crate::spacer::Spacer;
use crate::theme::{theme, theme_revision, Theme, ThemeColors};
use crate::tool_execution::{render_compact, tool_style, CompactTool, ToolStatus, ToolStyle};
use crate::tool_panel::ToolPanel;
use crate::utils::{truncate_to_width, wrap_text_with_ansi};
use crate::visual_truncate::truncate_to_visual_lines;

/// Preview line limit when collapsed (matches the TS `PREVIEW_LINES`).
const PREVIEW_LINES: usize = 20;

/// After this long without a result, a running bash panel shows the
/// "Esc / Ctrl+C to abort" hint (the tool has no default timeout — the model must
/// pass one, and a stuck command otherwise looks frozen).
const LONG_RUNNING_HINT_AFTER: Duration = Duration::from_secs(60);

/// Bash execution status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BashStatus {
    Running,
    Complete,
    Cancelled,
    Error,
}

/// Truncation info extracted from `BashToolDetails` at completion.
#[derive(Debug, Clone, Default)]
pub struct BashTruncation {
    /// Whether the captured output was truncated for context limits.
    pub truncated: bool,
    /// Path to the full output file, if truncation wrote one.
    pub full_output_path: Option<String>,
}

/// A finalized bash panel is immutable, so its rendered lines can be memoized
/// by width. Mirrors upstream `bash-execution.ts` `cachedWidth/cachedLines`.
#[derive(Clone)]
struct BashRenderCache {
    width: usize,
    revision: u64,
    theme_revision: u64,
    style: ToolStyle,
    lines: Vec<String>,
}

/// Component that displays a bash command execution with streaming output.
pub struct BashExecutionComponent {
    command: Mutex<String>,
    /// The agent's description of the command, shown first when set.
    description: Mutex<Option<String>>,
    output_lines: Mutex<Vec<String>>,
    status: Mutex<BashStatus>,
    exit_code: Mutex<Option<i32>>,
    expanded: Mutex<bool>,
    truncation: Mutex<BashTruncation>,
    /// `true` for a `!!`-prefixed user command: render dim and keep out of the
    /// model context (`bash-execution.ts` `excludeFromContext`).
    exclude_from_context: Mutex<bool>,
    /// `true` for a user-typed `!command`: always the boxed shell panel. Agent
    /// bash calls follow the tool style (ice only boxes user shell commands).
    user_shell: Mutex<bool>,
    /// When the command started / finished, for the live elapsed readout while
    /// running and the total elapsed after completion.
    started_at: Mutex<Option<Instant>>,
    finished_at: Mutex<Option<Instant>>,
    /// Bumped by every mutator so a completed panel's cached render is dropped
    /// when the user expands it or a late update lands.
    revision: AtomicU64,
    /// Pointer is over the collapsed hint row (see `set_hovered`).
    hovered: AtomicBool,
    cache: Mutex<Option<BashRenderCache>>,
}

impl BashExecutionComponent {
    /// Create a new component for the given command (without the leading `$`).
    pub fn new(command: impl Into<String>) -> Self {
        Self::new_with_context(command, false)
    }

    /// Create a component whose command is excluded from the model context
    /// (the `!!` prefix). Renders with the dim border/header color.
    pub fn new_excluded(command: impl Into<String>) -> Self {
        Self::new_with_context(command, true)
    }

    /// Shared constructor. `exclude_from_context` selects the dim color key.
    pub fn new_with_context(command: impl Into<String>, exclude_from_context: bool) -> Self {
        Self {
            command: Mutex::new(command.into()),
            description: Mutex::new(None),
            output_lines: Mutex::new(Vec::new()),
            status: Mutex::new(BashStatus::Running),
            exit_code: Mutex::new(None),
            expanded: Mutex::new(false),
            truncation: Mutex::new(BashTruncation::default()),
            exclude_from_context: Mutex::new(exclude_from_context),
            user_shell: Mutex::new(false),
            started_at: Mutex::new(Some(Instant::now())),
            finished_at: Mutex::new(None),
            revision: AtomicU64::new(0),
            hovered: AtomicBool::new(false),
            cache: Mutex::new(None),
        }
    }

    /// Mark this as a user-typed `!command`, which keeps the boxed panel.
    pub fn set_user_shell(&self) {
        *self.user_shell.lock().unwrap() = true;
        self.bump();
    }

    /// Highlight the collapsed hint row while the pointer is over it. Returns
    /// whether the state changed, so callers repaint only then.
    pub fn set_hovered(&self, hovered: bool) -> bool {
        let changed = self.hovered.swap(hovered, Ordering::AcqRel) != hovered;
        if changed {
            self.bump();
        }
        changed
    }

    /// Invalidate the memoized render (state changed).
    fn bump(&self) {
        self.revision.fetch_add(1, Ordering::AcqRel);
    }

    /// Total elapsed time, or the elapsed time so far while still running.
    fn elapsed(&self) -> Duration {
        let started = self.started_at.lock().ok().and_then(|g| *g);
        let Some(started) = started else {
            return Duration::default();
        };
        match self.finished_at.lock().ok().and_then(|g| *g) {
            Some(finished) => finished.saturating_duration_since(started),
            None => started.elapsed(),
        }
    }

    /// Set whether this command is excluded from the model context. The border
    /// and header recolor on the next render.
    pub fn set_exclude_from_context(&self, exclude: bool) {
        if let Ok(mut e) = self.exclude_from_context.lock() {
            *e = exclude;
        }
        self.bump();
    }

    /// Whether this command is excluded from the model context.
    pub fn is_excluded_from_context(&self) -> bool {
        *self.exclude_from_context.lock().unwrap()
    }

    /// Name the command by what it does (the bash tool's `description`).
    /// Model text: kept to one line without control characters.
    pub fn set_description(&self, description: &str) {
        let clean: String = description
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .filter(|c| !c.is_control())
            .collect();
        let clean = Some(clean).filter(|d| !d.is_empty());
        let mut current = self.description.lock().unwrap();
        if *current != clean {
            *current = clean;
            drop(current);
            self.bump();
        }
    }

    /// Apply the authoritative command from `ToolExecutionStart`. A panel may
    /// have been created from an earlier streaming snapshot, so this must
    /// replace a partial command as well as an empty one.
    pub fn set_command(&self, command: &str) {
        if !command.trim().is_empty() {
            let mut c = self.command.lock().unwrap();
            *c = command.to_string();
            drop(c);
            self.bump();
        }
    }

    /// Replace the displayed output with the latest streamed snapshot.
    ///
    /// The bash tool's `on_update` contract sends the complete captured output
    /// so far, not a delta. Appending each update duplicated all prior lines
    /// (`a`, then `a\nb` became `aa\nb`). Keep the latest snapshot instead.
    pub fn append_output(&self, chunk: &str) {
        let clean = strip_ansi(chunk).replace("\r\n", "\n").replace('\r', "\n");
        if let Ok(mut out) = self.output_lines.lock() {
            *out = if clean.is_empty() {
                Vec::new()
            } else {
                clean
                    .split('\n')
                    .map(|line| line.replace('\t', "    "))
                    .collect()
            };
        }
        self.bump();
    }

    /// Mark execution complete.
    pub fn set_complete(
        &self,
        exit_code: Option<i32>,
        cancelled: bool,
        truncation: BashTruncation,
    ) {
        if let Ok(mut e) = self.exit_code.lock() {
            *e = exit_code;
        }
        let new_status = if cancelled {
            BashStatus::Cancelled
        } else if exit_code.map(|c| c != 0).unwrap_or(false) {
            BashStatus::Error
        } else {
            BashStatus::Complete
        };
        if let Ok(mut s) = self.status.lock() {
            *s = new_status;
        }
        if let Ok(mut t) = self.truncation.lock() {
            *t = truncation;
        }
        if let Ok(mut f) = self.finished_at.lock() {
            *f = Some(Instant::now());
        }
        self.bump();
    }

    /// Toggle expanded (full output) vs collapsed (preview) display.
    pub fn set_expanded(&self, expanded: bool) {
        if let Ok(mut e) = self.expanded.lock() {
            *e = expanded;
        }
        self.bump();
    }

    /// Whether currently expanded.
    pub fn is_expanded(&self) -> bool {
        *self.expanded.lock().unwrap()
    }

    /// Current command and status used by compact exploration groups.
    pub fn command(&self) -> String {
        self.command.lock().unwrap().clone()
    }

    pub fn status(&self) -> BashStatus {
        *self.status.lock().unwrap()
    }

    /// Get the raw joined output.
    pub fn get_output(&self) -> String {
        self.output_lines
            .lock()
            .map(|o| o.join("\n"))
            .unwrap_or_default()
    }

    /// Build the trailing status/truncation line(s), if any.
    fn status_line(&self, hidden: usize, colors: &ThemeColors) -> Vec<String> {
        let status = *self.status.lock().unwrap();
        let truncation = self.truncation.lock().unwrap().clone();
        let exit_code = *self.exit_code.lock().unwrap();
        let expanded = *self.expanded.lock().unwrap();

        let mut parts: Vec<String> = Vec::new();
        // Total elapsed up front: it is the first thing a reader looks for on a
        // finished command.
        let elapsed = self.elapsed();
        if elapsed > Duration::from_millis(100) {
            parts.push(
                colors
                    .muted
                    .fg(&crate::tool_execution::format_elapsed(elapsed)),
            );
        }
        if hidden > 0 {
            if expanded {
                parts.push(
                    colors
                        .muted
                        .fg(crate::keybindings::tools_expand_paren("collapse").trim_start()),
                );
            } else {
                parts.push(colors.muted.fg(&format!(
                    "... {} more lines{}",
                    hidden,
                    crate::keybindings::tools_expand_paren("expand")
                )));
            }
        }
        match status {
            BashStatus::Cancelled => parts.push(colors.warning.fg("(cancelled)")),
            BashStatus::Error => {
                parts.push(
                    colors
                        .error
                        .fg(&format!("(exit {})", exit_code.unwrap_or(-1))),
                );
            }
            _ => {}
        }
        if truncation.truncated {
            let path = truncation.full_output_path.as_deref().unwrap_or("");
            parts.push(
                colors
                    .warning
                    .fg(&format!("Output truncated. Full output: {}", path)),
            );
        }
        if parts.is_empty() {
            Vec::new()
        } else {
            vec![parts.join(" ")]
        }
    }

    /// Compact style for an agent bash call: the shared bullet row and gutter
    /// preview, with the exit/truncation outcome appended to the output.
    fn render_compact_panel(&self, width: usize, status: BashStatus, theme: &Theme) -> Vec<String> {
        let command = self.command.lock().unwrap().clone();
        let description = self.description.lock().unwrap().clone();
        let mut text = self.output_lines.lock().unwrap().join("\n");
        let hidden = 0;
        for line in self.status_line(hidden, &theme.colors) {
            text.push('\n');
            text.push_str(&strip_ansi(&line));
        }
        let tool_status = match status {
            BashStatus::Running => ToolStatus::Running,
            BashStatus::Complete => ToolStatus::Completed,
            BashStatus::Cancelled | BashStatus::Error => ToolStatus::Failed,
        };
        render_compact(
            &theme.colors,
            &CompactTool {
                name: "bash",
                label: "bash".to_string(),
                summary: command,
                description: description.as_deref(),
                status: tool_status,
                result: Some(text.as_str()).filter(|t| !t.trim().is_empty()),
                diff: None,
                markdown: false,
                expanded: *self.expanded.lock().unwrap(),
                leading_spacer: true,
                expand_id: Some(self as *const Self as usize),
                hovered: self.hovered.load(Ordering::Acquire),
            },
            width,
        )
    }

    /// Render against one theme snapshot; keep all insets inside the background.
    fn render_panel(&self, width: usize, status: BashStatus, theme: &Theme) -> Vec<String> {
        let colors = &theme.colors;
        let bg = match status {
            BashStatus::Running => colors.tool_pending_bg,
            BashStatus::Error | BashStatus::Cancelled => colors.tool_error_bg,
            BashStatus::Complete => colors.tool_success_bg,
        };
        let Some(panel) = ToolPanel::new(width, theme.spacing, bg) else {
            return Vec::new();
        };
        let content_width = panel.content_width;
        let panel_line = |content: &str| panel.line(content);
        let blank = panel.blank();
        let mut lines = Spacer::new(1).render(width);
        lines.extend(std::iter::repeat(blank.clone()).take(panel.padding_y));

        let excluded = *self.exclude_from_context.lock().unwrap();
        let color_key = if excluded {
            colors.dim
        } else {
            colors.bash_mode
        };
        if let Some(description) = self.description.lock().unwrap().clone() {
            for part in wrap_text_with_ansi(&description, content_width) {
                lines.push(panel_line(&colors.tool_title.fg(&bold(&part))));
            }
        }
        let command = self.command.lock().unwrap().clone();
        for part in wrap_text_with_ansi(&format!("$ {command}"), content_width) {
            lines.push(panel_line(&color_key.fg(&bold(&part))));
        }

        let (display_lines, hidden) = {
            let out = self.output_lines.lock().unwrap();
            let expanded = *self.expanded.lock().unwrap();
            if out.is_empty() {
                (Vec::new(), 0usize)
            } else {
                let styled = format!(
                    "\n{}",
                    out.iter()
                        .map(|l| colors.muted.fg(l))
                        .collect::<Vec<_>>()
                        .join("\n")
                );
                if expanded {
                    (
                        crate::text::Text::new(&styled, 0, 0).render(content_width),
                        0,
                    )
                } else {
                    let r = truncate_to_visual_lines(&styled, PREVIEW_LINES, content_width, 0);
                    (r.visual_lines, r.skipped_count)
                }
            }
        };
        for line in &display_lines {
            lines.push(panel_line(line));
        }

        if status == BashStatus::Running {
            let elapsed = self.elapsed();
            let row = format!(
                "{} {}",
                colors.accent.fg("●"),
                colors
                    .muted
                    .fg(&crate::tool_execution::format_elapsed(elapsed)),
            );
            lines.push(panel_line(&truncate_to_width(&row, content_width, "…")));
            if elapsed > LONG_RUNNING_HINT_AFTER {
                let hint = format!("{} Esc / Ctrl+C to abort", colors.muted.fg("⏸"));
                for part in wrap_text_with_ansi(&hint, content_width) {
                    lines.push(panel_line(&part));
                }
            }
        } else {
            let status_lines = self.status_line(hidden, colors);
            if !status_lines.is_empty() {
                lines.push(blank.clone());
                for line in &status_lines {
                    for part in wrap_text_with_ansi(line, content_width) {
                        lines.push(panel_line(&part));
                    }
                }
            }
        }
        lines.extend(std::iter::repeat(blank).take(panel.padding_y));
        lines
    }
}

impl Component for BashExecutionComponent {
    fn render(&self, width: usize) -> Vec<String> {
        let status = *self.status.lock().unwrap();
        let revision = self.revision.load(Ordering::Acquire);
        let theme_revision = theme_revision();
        let style = tool_style();

        // A finished panel never changes (no live elapsed timer), so serve it
        // from the cache. A running panel is skipped — its elapsed timer must
        // keep advancing.
        if status != BashStatus::Running {
            if let Ok(cache) = self.cache.lock() {
                if let Some(cached) = cache.as_ref() {
                    if cached.width == width
                        && cached.revision == revision
                        && cached.theme_revision == theme_revision
                        && cached.style == style
                    {
                        return cached.lines.clone();
                    }
                }
            }
        }

        let lines = if style == ToolStyle::Compact && !*self.user_shell.lock().unwrap() {
            self.render_compact_panel(width, status, &theme())
        } else {
            self.render_panel(width, status, &theme())
        };

        if status != BashStatus::Running {
            if let Ok(mut cache) = self.cache.lock() {
                *cache = Some(BashRenderCache {
                    width,
                    revision,
                    theme_revision,
                    style,
                    lines: lines.clone(),
                });
            }
        } else if let Ok(mut cache) = self.cache.lock() {
            *cache = None;
        }

        lines
    }

    fn invalidate(&self) {
        self.bump();
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Agent bash calls render compact: one `Ran` row, a 3-row head under the
    /// gutter and a hint; a failure keeps its exit code visible.
    #[test]
    fn compact_agent_bash_is_one_row_with_head_preview() {
        let theme = Theme::default();
        let c = BashExecutionComponent::new("ls -la && find crates -name Cargo.toml");
        c.append_output(
            &(1..=33)
                .map(|i| format!("row {i}"))
                .collect::<Vec<_>>()
                .join("\n"),
        );
        c.set_complete(Some(0), false, BashTruncation::default());
        let plain = |c: &BashExecutionComponent| {
            let status = *c.status.lock().unwrap();
            strip_ansi(&c.render_compact_panel(100, status, &theme).join("\n"))
        };
        let done = plain(&c);
        assert!(
            done.contains("✓ Ran ls -la && find crates -name Cargo.toml"),
            "{done}"
        );
        assert!(done.contains("⎿ row 1") && done.contains("row 3") && !done.contains("row 4"));
        assert!(done.contains("… +30 lines"));
        assert!(!done.contains("$ "));

        let failed = BashExecutionComponent::new("make");
        failed.append_output("boom");
        failed.set_complete(Some(2), false, BashTruncation::default());
        let text = plain(&failed);
        assert!(
            text.contains("✗ Ran make") && text.contains("(exit 2)"),
            "{text}"
        );
    }

    /// Several commands run at once: each row leads with its description,
    /// then the command, dimmed. Model text cannot break the row.
    #[test]
    fn compact_agent_bash_leads_with_its_description() {
        let theme = Theme::default();
        let c = BashExecutionComponent::new("cargo test -p ice-cli\ncargo fmt");
        c.set_description("Run CLI tests\n\x1b[31mnow");
        let status = *c.status.lock().unwrap();
        let rows = c.render_compact_panel(100, status, &theme);
        let title = strip_ansi(&rows[1]);
        assert_eq!(
            title.trim_end(),
            "› Running Run CLI tests [31mnow · cargo test -p ice-cli …"
        );
        c.set_description("  ");
        let rows = c.render_compact_panel(100, status, &theme);
        assert!(strip_ansi(&rows[1]).starts_with("› Running cargo test -p ice-cli"));
    }

    #[test]
    fn compact_bash_hint_is_clickable_and_highlights_on_hover() {
        let theme = Theme::default();
        let c = BashExecutionComponent::new("ls");
        c.append_output(
            &(1..=33)
                .map(|i| format!("row {i}"))
                .collect::<Vec<_>>()
                .join("\n"),
        );
        c.set_complete(Some(0), false, BashTruncation::default());
        let render = |c: &BashExecutionComponent| {
            let status = *c.status.lock().unwrap();
            c.render_compact_panel(100, status, &theme)
        };
        let find_hint = |lines: &[String]| {
            lines
                .iter()
                .find(|line| strip_ansi(line).contains("+30 lines"))
                .cloned()
                .unwrap()
        };
        let idle = find_hint(&render(&c));
        let column = strip_ansi(&idle).find("+30").unwrap();
        assert_eq!(
            crate::expand_link_id(&idle, column),
            Some(&c as *const BashExecutionComponent as usize)
        );
        let fill = theme.colors.accent.to_bg();
        assert!(!idle.contains(&fill));

        assert!(c.set_hovered(true));
        assert!(!c.set_hovered(true));
        let hovered = find_hint(&render(&c));
        assert!(hovered.contains(&fill), "{hovered:?}");
        assert!(c.set_hovered(false));
        assert_eq!(find_hint(&render(&c)), idle);
    }

    #[test]
    fn panel_has_themed_insets_in_every_execution_state() {
        let theme = Theme::default();
        let width = 40;
        for (exit_code, cancelled) in [
            (None, false),
            (Some(0), false),
            (Some(1), false),
            (Some(0), true),
        ] {
            let c = BashExecutionComponent::new("echo hi");
            c.append_output("hi");
            if exit_code.is_some() {
                c.set_complete(exit_code, cancelled, BashTruncation::default());
            }
            let status = *c.status.lock().unwrap();
            let bg = match status {
                BashStatus::Running => theme.colors.tool_pending_bg,
                BashStatus::Complete => theme.colors.tool_success_bg,
                BashStatus::Error | BashStatus::Cancelled => theme.colors.tool_error_bg,
            };
            let lines = c.render_panel(width, status, &theme);
            assert_eq!(lines[0], "", "outer spacer must remain uncolored");
            assert_eq!(lines[1], bg.bg(&" ".repeat(width)));
            assert_eq!(lines.last().unwrap(), &lines[1]);
            let plain: Vec<_> = lines[1..].iter().map(|line| strip_ansi(line)).collect();
            assert_eq!(plain[1].trim_end(), "  $ echo hi");
            assert!(plain.iter().any(|line| line.trim_end() == "  hi"));
            for (styled, text) in lines[1..].iter().zip(&plain) {
                assert!(styled.starts_with(&bg.to_bg()));
                assert_eq!(crate::utils::visible_width(text), width);
                assert!(text.starts_with("  ") && text.ends_with("  "), "{text:?}");
            }
        }
    }

    #[test]
    fn custom_theme_spacing_controls_panel_without_global_mutation() {
        let mut theme = Theme::default();
        theme.spacing.tool_panel_padding_x = 4;
        theme.spacing.tool_panel_padding_y = 2;
        let c = BashExecutionComponent::new("echo hi");
        // Keep this layout-only test independent of scheduler delays.
        *c.started_at.lock().unwrap() = None;
        c.append_output("hi");
        c.set_complete(Some(0), false, BashTruncation::default());
        let lines = c.render_panel(40, BashStatus::Complete, &theme);
        let blank = theme.colors.tool_success_bg.bg(&" ".repeat(40));
        assert_eq!(lines[1..3], [blank.clone(), blank.clone()]);
        assert_eq!(lines[lines.len() - 2..], [blank.clone(), blank]);
        assert_eq!(strip_ansi(&lines[3]).trim_end(), "    $ echo hi");
        assert!(lines
            .iter()
            .any(|line| strip_ansi(line).trim_end() == "    hi"));

        theme.spacing.tool_panel_padding_x = 0;
        theme.spacing.tool_panel_padding_y = 0;
        let lines = c.render_panel(40, BashStatus::Complete, &theme);
        assert_eq!(strip_ansi(&lines[1]).trim_end(), "$ echo hi");
        assert_eq!(strip_ansi(lines.last().unwrap()).trim_end(), "hi");
    }

    #[test]
    fn output_wraps_inside_both_insets_collapsed_and_expanded() {
        let theme = Theme::default();
        let output = "abcdefghijklmnop你好世界qrstuvwxyz";
        let c = BashExecutionComponent::new("echo");
        *c.started_at.lock().unwrap() = None;
        c.append_output(output);
        c.set_complete(Some(0), false, BashTruncation::default());
        for expanded in [false, true] {
            c.set_expanded(expanded);
            let lines = c.render_panel(12, BashStatus::Complete, &theme);
            let output_rows = &lines[3..lines.len() - 1];
            let reconstructed: String = output_rows
                .iter()
                .map(|line| strip_ansi(line).trim().to_string())
                .collect();
            assert_eq!(reconstructed, output);
            for line in output_rows {
                let plain = strip_ansi(line);
                assert_eq!(crate::utils::visible_width(&plain), 12);
                assert!(plain.starts_with("  ") && plain.ends_with("  "));
            }
        }
    }

    #[test]
    fn long_status_paths_wrap_inside_panel() {
        let path = "/tmp/very-long-directory/full-command-output.log";
        let c = BashExecutionComponent::new("false");
        c.set_complete(
            Some(1),
            false,
            BashTruncation {
                truncated: true,
                full_output_path: Some(path.to_string()),
            },
        );
        let lines = c.render_panel(16, BashStatus::Error, &Theme::default());
        let compact: String = strip_ansi(&lines.join("\n"))
            .chars()
            .filter(|ch| !ch.is_whitespace())
            .collect();
        assert!(compact.contains(path));
        assert!(lines
            .iter()
            .all(|line| crate::utils::visible_width(line) <= 16));
    }

    #[test]
    fn narrow_panels_reduce_padding_without_overflow() {
        let c = BashExecutionComponent::new("echo 你好");
        c.append_output("你好世界");
        let mut theme = Theme::default();
        theme.spacing.tool_panel_padding_x = usize::MAX;
        for width in 0..=8 {
            let lines = c.render_panel(width, BashStatus::Running, &theme);
            assert!(
                lines
                    .iter()
                    .all(|line| crate::utils::visible_width(line) <= width),
                "width {width}: {lines:?}"
            );
            if width == 0 {
                assert!(lines.is_empty());
            }
        }
    }

    #[test]
    fn test_new_renders_header_and_live_elapsed() {
        let c = BashExecutionComponent::new("ls -la");
        let lines = c.render(40);
        let joined = lines.join("\n");
        assert!(joined.contains("$ ls -la"));
        // A running panel reports a live elapsed timer and a static status
        // glyph — no spinner, matching tool panels.
        let plain = strip_ansi(&joined);
        assert!(
            plain.contains('●'),
            "running panel needs the status glyph: {plain}"
        );
        assert!(
            plain.contains('s'),
            "running panel needs an elapsed readout: {plain}"
        );
        assert!(
            !plain.contains("Running"),
            "the `Running...` label was replaced by the elapsed timer: {plain}"
        );
    }

    #[test]
    fn completed_panel_reports_total_elapsed() {
        let c = BashExecutionComponent::new("sleep 1");
        c.append_output("done\n");
        std::thread::sleep(std::time::Duration::from_millis(120));
        c.set_complete(Some(0), false, BashTruncation::default());
        let plain = strip_ansi(&c.render(60).join("\n"));
        // The completion line leads with the total elapsed time.
        assert!(
            plain.contains("0.1s") || plain.contains("0.2s"),
            "total elapsed missing: {plain}"
        );
        // And it stops advancing once the command is finished.
        let first = c.render(60).join("\n");
        std::thread::sleep(std::time::Duration::from_millis(60));
        assert_eq!(
            first,
            c.render(60).join("\n"),
            "elapsed kept counting after completion"
        );
    }

    #[test]
    fn test_append_and_complete() {
        let c = BashExecutionComponent::new("echo hi");
        c.append_output("hi\n");
        c.set_complete(Some(0), false, BashTruncation::default());
        let lines = c.render(40);
        let joined = lines.join("\n");
        // After completion the spinner is gone; output "hi" is visible.
        assert!(strip_ansi(&joined).contains("hi"), "joined: {joined}");
    }

    #[test]
    fn excluded_from_context_uses_dim_border_and_header() {
        // `!!cmd` renders dim rather than bashMode so the exclusion is visible
        // (mirrors `bash-execution.ts` `excludeFromContext`).
        let normal = BashExecutionComponent::new("echo hi");
        let excluded = BashExecutionComponent::new_excluded("echo hi");
        assert!(!normal.is_excluded_from_context());
        assert!(excluded.is_excluded_from_context());

        let colors = theme().colors;
        // The command header uses the bash accent (or dim when excluded), so
        // compare the foreground escape each color produces.
        let bash_start = colors
            .bash_mode
            .fg("X")
            .split('X')
            .next()
            .unwrap()
            .to_string();
        let dim_start = colors.dim.fg("X").split('X').next().unwrap().to_string();

        let normal_out = normal.render(40).join("\n");
        let excluded_out = excluded.render(40).join("\n");
        assert!(normal_out.contains(&bash_start), "normal: {normal_out:?}");
        assert!(
            excluded_out.contains(&dim_start),
            "excluded: {excluded_out:?}"
        );

        // Both still show the command itself.
        assert!(strip_ansi(&excluded_out).contains("$ echo hi"));

        // Flipping the flag in place recolors without recreating the component.
        excluded.set_exclude_from_context(false);
        assert!(!excluded.is_excluded_from_context());
        assert!(excluded.render(40).join("\n").contains(&bash_start));
    }

    #[test]
    fn streamed_output_updates_replace_snapshots_instead_of_duplicating() {
        let c = BashExecutionComponent::new("printf test");
        c.append_output("one\n");
        c.append_output("one\ntwo\n");
        assert_eq!(c.get_output(), "one\ntwo\n");
    }

    #[test]
    fn authoritative_start_command_replaces_partial_streamed_command() {
        let c = BashExecutionComponent::new("printf");
        c.set_command("printf '\\n--- agent skills ---\\n' && find skills");
        let rendered = strip_ansi(&c.render(120).join("\n"));
        assert!(rendered.contains("printf '\\n--- agent skills ---\\n' && find skills"));
        assert!(!rendered.contains("$ printf\n"));
    }

    #[test]
    fn long_running_command_wraps_instead_of_truncating() {
        let command = "printf 'this command must remain fully visible while it runs'";
        let c = BashExecutionComponent::new(command);
        let lines = c.render(20);
        let rendered = strip_ansi(&lines.join("\n"));
        let compact: String = rendered.chars().filter(|ch| !ch.is_whitespace()).collect();
        let expected: String = command.chars().filter(|ch| !ch.is_whitespace()).collect();
        assert!(
            compact.contains(&expected),
            "command was truncated: {rendered}"
        );
        assert!(lines
            .iter()
            .all(|line| crate::utils::visible_width(line) <= 20));
    }

    #[test]
    fn test_error_exit_status() {
        let c = BashExecutionComponent::new("false");
        c.set_complete(Some(1), false, BashTruncation::default());
        let lines = c.render(40);
        let joined = lines.join("\n");
        assert!(joined.contains("exit 1"));
    }

    #[test]
    fn test_truncation_path_shown() {
        let c = BashExecutionComponent::new("big-output");
        c.set_complete(
            Some(0),
            false,
            BashTruncation {
                truncated: true,
                full_output_path: Some("/tmp/out.log".to_string()),
            },
        );
        let lines = c.render(40);
        // Paths may wrap now that status rows reserve both panel insets.
        let joined: String = lines
            .iter()
            .map(|line| strip_ansi(line).trim().to_string())
            .collect();
        assert!(joined.contains("/tmp/out.log"), "joined: {joined}");
    }
}
