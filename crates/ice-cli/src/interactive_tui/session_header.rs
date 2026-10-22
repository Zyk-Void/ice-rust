//! Compact startup banner, ported from ice `SessionHeaderComponent`
//! (`packages/coding-agent/src/modes/interactive/components/session-header.ts`).

use super::*;

/// What the banner shows. Read on every render so a model or thinking switch
/// and a `/reload` are reflected without rebuilding the transcript.
pub(super) struct SessionHeaderData {
    pub app_name: String,
    pub version: String,
    /// Model label, or `None` before a model is selected.
    pub model: Option<String>,
    /// Thinking level; `None` hides it.
    pub thinking: Option<String>,
    pub cwd: String,
    /// Loaded resource summary (e.g. "2 context files · 61 skills"); empty hides it.
    pub loaded: String,
    /// One-line dim hint shown under the banner.
    pub hint: String,
}

/// Pixel mascot, rendered in the accent color.
const BLOCK_ART: [&str; 5] = [
    "█▄  ▄▄▄  ▄█",
    "▀██▀███▀██▀",
    " ██▄▀█▀▄██",
    "  ▀█████▀",
    "   █▄▀▄█",
];
const ART_GAP: usize = 2;
/// Below this many columns for the text, the mascot is dropped.
const MIN_TEXT_WIDTH: usize = 24;
const SEPARATOR: &str = " · ";
const ELLIPSIS: &str = "…";

/// Truncate a path in the middle so both the root and the leaf stay visible.
pub(super) fn middle_truncate(text: &str, max_width: usize) -> String {
    if ice_tui::visible_width(text) <= max_width {
        return text.to_string();
    }
    let available = max_width.saturating_sub(ice_tui::visible_width(ELLIPSIS));
    if available <= 1 {
        return ice_tui::truncate_to_width(text, max_width, "");
    }
    let chars: Vec<char> = text.chars().collect();
    let tail = available.div_ceil(2).min(chars.len());
    let head = (available - tail).min(chars.len() - tail);
    let head: String = chars[..head].iter().collect();
    let tail: String = chars[chars.len() - tail..].iter().collect();
    format!("{head}{ELLIPSIS}{tail}")
}

/// A pixel mascot beside three lines: name and version; model and thinking
/// level; directory and loaded resources. The mascot drops on narrow
/// terminals.
pub(super) struct SessionHeader {
    data: Box<dyn Fn() -> SessionHeaderData + Send + Sync>,
}

impl SessionHeader {
    pub(super) fn new(data: impl Fn() -> SessionHeaderData + Send + Sync + 'static) -> Self {
        Self {
            data: Box::new(data),
        }
    }

    fn build_text(data: &SessionHeaderData, width: usize) -> Vec<String> {
        let c = current_theme().colors;
        let dim_separator = c.dim.fg(SEPARATOR);
        let title = format!(
            "{} {}",
            ice_tui::bold(&data.app_name.to_uppercase()),
            c.dim.fg(&format!("v{}", data.version))
        );

        let model = match (&data.model, &data.thinking) {
            (Some(model), Some(thinking)) => format!("{model}{SEPARATOR}{thinking}"),
            (Some(model), None) => model.clone(),
            (None, _) => "no model".to_string(),
        };
        let session = if ice_tui::visible_width(&model) <= width {
            c.muted.fg(&model)
        } else {
            c.muted
                .fg(&ice_tui::truncate_to_width(&model, width, ELLIPSIS))
        };

        let with_loaded = if data.loaded.is_empty() {
            data.cwd.clone()
        } else {
            format!("{}{SEPARATOR}{}", data.cwd, data.loaded)
        };
        let location = if ice_tui::visible_width(&with_loaded) <= width {
            if data.loaded.is_empty() {
                c.muted.fg(&data.cwd)
            } else {
                format!(
                    "{}{dim_separator}{}",
                    c.muted.fg(&data.cwd),
                    c.dim.fg(&data.loaded)
                )
            }
        } else {
            c.muted.fg(&middle_truncate(&data.cwd, width))
        };

        vec![
            ice_tui::truncate_to_width(&title, width, ELLIPSIS),
            session,
            location,
        ]
    }
}

impl Component for SessionHeader {
    fn render(&self, width: usize) -> Vec<String> {
        let data = (self.data)();
        let accent = current_theme().colors.accent;
        let art_width = BLOCK_ART
            .iter()
            .map(|l| ice_tui::visible_width(l))
            .max()
            .unwrap_or(0);
        let show_art = width.saturating_sub(art_width + ART_GAP) >= MIN_TEXT_WIDTH;
        let text_width = if show_art {
            width - art_width - ART_GAP
        } else {
            width
        }
        .max(1);
        let text = Self::build_text(&data, text_width);

        // The text block is vertically centered against the mascot.
        let height = text.len().max(if show_art { BLOCK_ART.len() } else { 0 });
        let text_top = (height - text.len()) / 2;
        let mut lines = Vec::with_capacity(height + 1);
        for row in 0..height {
            let art = if show_art {
                BLOCK_ART.get(row).copied().unwrap_or("")
            } else {
                ""
            };
            let Some(text_line) = row.checked_sub(text_top).and_then(|i| text.get(i)) else {
                lines.push(accent.fg(art));
                continue;
            };
            let left = if show_art {
                let pad = art_width - ice_tui::visible_width(art) + ART_GAP;
                format!("{}{}", accent.fg(art), " ".repeat(pad))
            } else {
                String::new()
            };
            lines.push(format!("{left}{text_line}"));
        }
        lines.push(ice_tui::truncate_to_width(
            &current_theme().colors.dim.fg(&data.hint),
            width,
            ELLIPSIS,
        ));
        lines
    }

    fn invalidate(&self) {}

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// One-line count of loaded context files, skills and prompt templates
/// (ice `getLoadedResourceSummary`; zero counts are omitted).
pub(super) fn loaded_resource_summary(
    resources: &ice_harness::types::AgentHarnessResources,
) -> String {
    let counts = [
        (resources.context_files.len(), "context file"),
        (resources.skills.as_ref().map_or(0, Vec::len), "skill"),
        (
            resources.prompt_templates.as_ref().map_or(0, Vec::len),
            "prompt",
        ),
    ];
    counts
        .iter()
        .filter(|(count, _)| *count > 0)
        .map(|(count, noun)| {
            let plural = if *count == 1 { "" } else { "s" };
            format!("{count} {noun}{plural}")
        })
        .collect::<Vec<_>>()
        .join(SEPARATOR)
}
