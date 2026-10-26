//! `/mcp setup` overlay: detected import sources, quick-add presets, name
//! conflicts and skipped imports. Ports pi-mcp-adapter's `mcp-setup-panel.ts`
//! for the parts icers supports (stdio only, no shared `.mcp.json` scaffold).
//! Like `mcp_panel`, the state machine is pure; `commands/mcp.rs` runs actions.

use std::path::Path;

use crossterm::event::{KeyCode, KeyEvent};
use ice_tui::{theme as current_theme, truncate_to_width};

use super::mcp_panel::{clean, Frame};
use crate::mcp::setup::{PresetEntry, SetupView, PRESETS};

pub(super) const WIDTH: usize = 82;
/// Conflicts and notes can be long; the rest is a fixed, short list.
const MAX_LISTED: usize = 5;

#[derive(Clone, Debug, PartialEq)]
pub(super) enum SetupAction {
    None,
    Close,
    SetImport(&'static str, bool),
    AddPreset(&'static str),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Item {
    Import(usize),
    Preset(usize),
}

pub(super) struct SetupPanel {
    view: SetupView,
    items: Vec<Item>,
    cursor: usize,
    /// Set while an action runs; further actions wait for `refresh`.
    busy: bool,
    notice: Option<String>,
    home: Option<std::path::PathBuf>,
}

impl SetupPanel {
    pub(super) fn new(view: SetupView, home: Option<std::path::PathBuf>) -> Self {
        let mut panel = Self {
            view: SetupView::default(),
            items: Vec::new(),
            cursor: 0,
            busy: false,
            notice: None,
            home,
        };
        panel.refresh(view);
        panel
    }

    /// Adopt the state after an action, keeping the cursor on the same row kind.
    pub(super) fn refresh(&mut self, view: SetupView) {
        self.items = (0..view.imports.len())
            .map(Item::Import)
            .chain((0..PRESETS.len()).map(Item::Preset))
            .collect();
        self.view = view;
        self.busy = false;
        self.cursor = self.cursor.min(self.items.len().saturating_sub(1));
    }

    /// Apply an action's outcome and the reloaded state; always ends `busy`.
    pub(super) fn finish(
        &mut self,
        view: Result<SetupView, String>,
        outcome: Result<String, String>,
    ) {
        let mut notice = outcome.unwrap_or_else(|error| error);
        match view {
            Ok(view) => self.refresh(view),
            Err(error) => {
                self.busy = false;
                notice = format!("{notice} {error}");
            }
        }
        self.notice = Some(notice);
    }

    pub(super) fn handle_key(&mut self, key: KeyEvent) -> SetupAction {
        let code = ice_tui::get_keybindings()
            .selection_key(&key)
            .map_or(key.code, |k| k.code);
        match code {
            KeyCode::Esc => return SetupAction::Close,
            KeyCode::Up => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Down => {
                self.cursor = (self.cursor + 1).min(self.items.len().saturating_sub(1))
            }
            KeyCode::Enter | KeyCode::Char(' ') if !self.busy => {
                self.notice = None;
                let action = self.activate();
                self.busy = action != SetupAction::None;
                return action;
            }
            _ => {}
        }
        SetupAction::None
    }

    fn activate(&mut self) -> SetupAction {
        match self.items.get(self.cursor) {
            Some(&Item::Import(i)) => {
                let source = &self.view.imports[i];
                if source.detected.kind == "codex" {
                    self.notice = Some("Codex config.toml cannot be imported yet.".into());
                    return SetupAction::None;
                }
                SetupAction::SetImport(source.detected.kind, !source.enabled)
            }
            Some(&Item::Preset(i)) => {
                let preset = &PRESETS[i];
                if self.view.configured.iter().any(|name| name == preset.id) {
                    self.notice = Some(format!("{} is already configured.", preset.name));
                    return SetupAction::None;
                }
                SetupAction::AddPreset(preset.id)
            }
            None => SetupAction::None,
        }
    }

    fn path(&self, path: &Path) -> String {
        let shown = match self
            .home
            .as_deref()
            .and_then(|home| path.strip_prefix(home).ok())
        {
            Some(rest) => Path::new("~").join(rest).display().to_string(),
            None => path.display().to_string(),
        };
        clean(&shown)
    }

    pub(super) fn render(&self, width: usize) -> Vec<String> {
        let colors = current_theme().colors;
        let frame = Frame::new(width);
        let heading = |text: &str| frame.row(&ice_tui::bold(text));
        let mut lines = vec![frame.top("MCP Setup"), frame.row("")];
        let marker = |item: Item| {
            if self.items.get(self.cursor) == Some(&item) {
                colors.accent.fg("▸")
            } else {
                " ".into()
            }
        };

        lines.push(heading("Import from other tools"));
        if self.view.imports.is_empty() {
            lines.push(frame.row(&colors.dim.fg("  No other tools' MCP configs found.")));
        }
        for (i, source) in self.view.imports.iter().enumerate() {
            let detected = &source.detected;
            let (icon, info) = if detected.kind == "codex" {
                (colors.dim.fg("○"), "unsupported (config.toml)".to_string())
            } else {
                let n = detected.servers.len();
                (
                    if source.enabled {
                        colors.success.fg("●")
                    } else {
                        colors.dim.fg("○")
                    },
                    format!("{n} server{}", if n == 1 { "" } else { "s" }),
                )
            };
            let files: Vec<_> = detected.files.iter().map(|f| self.path(f)).collect();
            lines.push(frame.row(&format!(
                "{} {icon} {:<15}{:<14}{}",
                marker(Item::Import(i)),
                detected.kind,
                info,
                colors.dim.fg(&files.join(", "))
            )));
        }
        lines.push(frame.row(""));

        lines.push(heading("Quick add"));
        for (i, preset) in PRESETS.iter().enumerate() {
            let configured = self.view.configured.iter().any(|name| name == preset.id);
            let (icon, note) = match (&preset.entry, configured) {
                (_, true) => (colors.success.fg("✓"), "configured".to_string()),
                (PresetEntry::Http { oauth: true, .. }, _) => (
                    colors.accent.fg("+"),
                    format!("sign-in · {}", preset.summary),
                ),
                _ => (colors.accent.fg("+"), preset.summary.into()),
            };
            let room = frame.inner.saturating_sub(24);
            lines.push(frame.row(&format!(
                "{} {icon} {:<18}{}",
                marker(Item::Preset(i)),
                preset.name,
                colors.dim.fg(&truncate_to_width(&note, room, "…"))
            )));
        }

        if !self.view.conflicts.is_empty() {
            lines.push(frame.row(""));
            lines.push(heading("Conflicts (the last source wins)"));
            for conflict in self.view.conflicts.iter().take(MAX_LISTED) {
                let chain: Vec<_> = conflict
                    .shadowed
                    .iter()
                    .chain([&conflict.winner])
                    .map(|p| self.path(p))
                    .collect();
                lines.push(frame.row(&format!(
                    "  {}: {}",
                    clean(&conflict.server),
                    colors.dim.fg(&chain.join(" → "))
                )));
            }
            more(&mut lines, &frame, self.view.conflicts.len());
        }
        if !self.view.notes.is_empty() {
            lines.push(frame.row(""));
            lines.push(heading("Skipped imports"));
            for note in self.view.notes.iter().take(MAX_LISTED) {
                lines.push(frame.row(&colors.warning.fg(&format!("  {}", clean(note)))));
            }
            more(&mut lines, &frame, self.view.notes.len());
        }

        lines.push(frame.row(""));
        if self.busy {
            lines.push(frame.row(&colors.warning.fg("Updating MCP servers…")));
        } else if let Some(notice) = &self.notice {
            lines.push(frame.row(&colors.warning.fg(&clean(notice))));
        }
        lines.push(frame.divider());
        lines.extend(frame.hints(&["↑↓ navigate", "⏎ toggle import / add preset", "esc close"]));
        lines.push(frame.bottom());
        lines
    }
}

fn more(lines: &mut Vec<String>, frame: &Frame, total: usize) {
    if total > MAX_LISTED {
        let dim = current_theme().colors.dim;
        lines.push(frame.row(&dim.fg(&format!("  … {} more", total - MAX_LISTED))));
    }
}

/// Overlay component: the panel behind a mutex shared with the key router.
pub(super) struct SetupPanelView(pub(super) std::sync::Mutex<SetupPanel>);
impl ice_tui::Component for SetupPanelView {
    fn render(&self, width: usize) -> Vec<String> {
        self.0.lock().unwrap().render(width)
    }
    fn invalidate(&self) {}
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::setup::{Conflict, Detected, ImportSource};
    use crossterm::event::KeyModifiers;
    use ice_tui::{strip_ansi, visible_width};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }
    fn source(kind: &'static str, enabled: bool, servers: &[&str]) -> ImportSource {
        ImportSource {
            detected: Detected {
                kind,
                files: vec![format!("/home/u/.{kind}/mcp.json").into()],
                servers: servers.iter().map(|s| (*s).into()).collect(),
            },
            enabled,
        }
    }
    fn sample() -> SetupPanel {
        SetupPanel::new(
            SetupView {
                imports: vec![
                    source("cursor", true, &["a", "b"]),
                    source("codex", false, &[]),
                ],
                conflicts: vec![Conflict {
                    server: "a\x1b[31m".into(),
                    winner: "/home/u/.icers/agent/mcp.json".into(),
                    shadowed: vec!["/home/u/.cursor/mcp.json".into()],
                }],
                notes: (0..7)
                    .map(|i| format!("import:cursor: r{i}: unsupported (http)"))
                    .collect(),
                configured: vec!["a".into(), "b".into()],
            },
            Some("/home/u".into()),
        )
    }
    fn select(panel: &mut SetupPanel, item: Item) {
        panel.cursor = panel.items.iter().position(|i| *i == item).unwrap();
    }

    #[test]
    fn enter_toggles_imports_and_adds_only_stdio_presets() {
        let mut panel = sample();
        assert_eq!(
            panel.handle_key(key(KeyCode::Enter)),
            SetupAction::SetImport("cursor", false)
        );
        // Busy until the result is applied.
        assert_eq!(panel.handle_key(key(KeyCode::Enter)), SetupAction::None);
        let mut view = panel.view.clone();
        view.imports[0].enabled = false;
        panel.finish(Ok(view), Ok("Stopped importing cursor.".into()));
        assert_eq!(panel.notice.as_deref(), Some("Stopped importing cursor."));
        assert_eq!(
            panel.handle_key(key(KeyCode::Char(' '))),
            SetupAction::SetImport("cursor", true)
        );
        // A failed reload still ends the busy state.
        panel.finish(Err("bad mcp.json".into()), Err("write failed".into()));
        assert_eq!(panel.notice.as_deref(), Some("write failed bad mcp.json"));
        assert!(!panel.busy);

        assert_eq!(panel.handle_key(key(KeyCode::Down)), SetupAction::None);
        assert_eq!(panel.handle_key(key(KeyCode::Enter)), SetupAction::None);
        assert!(panel.notice.as_deref().unwrap().contains("Codex"));

        select(&mut panel, Item::Preset(1));
        assert_eq!(
            panel.handle_key(key(KeyCode::Enter)),
            SetupAction::AddPreset("context7")
        );
        panel.refresh(panel.view.clone());
        let chrome = PRESETS
            .iter()
            .position(|p| p.id == "chrome-devtools")
            .unwrap();
        select(&mut panel, Item::Preset(chrome));
        assert_eq!(
            panel.handle_key(key(KeyCode::Enter)),
            SetupAction::AddPreset("chrome-devtools")
        );
        let mut view = panel.view.clone();
        view.configured.push("chrome-devtools".into());
        panel.refresh(view);
        assert_eq!(panel.handle_key(key(KeyCode::Enter)), SetupAction::None);
        assert!(panel
            .notice
            .as_deref()
            .unwrap()
            .contains("already configured"));

        // The cursor stops at both ends.
        for _ in 0..20 {
            panel.handle_key(key(KeyCode::Down));
        }
        assert_eq!(panel.cursor, panel.items.len() - 1);
        assert_eq!(panel.handle_key(key(KeyCode::Esc)), SetupAction::Close);
    }

    #[test]
    fn render_is_a_bounded_box_with_clean_shortened_text() {
        let panel = sample();
        let lines: Vec<_> = panel.render(WIDTH).iter().map(|l| strip_ansi(l)).collect();
        assert!(
            lines.iter().all(|l| visible_width(l) == WIDTH),
            "{lines:#?}"
        );
        assert!(lines[0].contains(" MCP Setup "));
        let text = lines.join("\n");
        assert!(text.contains("● cursor"));
        assert!(text.contains("2 servers"));
        assert!(text.contains("~/.cursor/mcp.json"));
        assert!(text.contains("unsupported (config.toml)"));
        assert!(text.contains("+ Chrome DevTools"));
        assert!(text.contains("+ DeepWiki"));
        assert!(text.contains("Notion            sign-in · "));
        assert!(text.contains("a [31m: ~/.cursor/mcp.json → ~/.icers/agent/mcp.json"));
        assert!(text.contains("… 2 more"));
        let empty = SetupPanel::new(SetupView::default(), None);
        let text = strip_ansi(&empty.render(WIDTH).join("\n"));
        assert!(text.contains("No other tools' MCP configs found."));
    }
}
