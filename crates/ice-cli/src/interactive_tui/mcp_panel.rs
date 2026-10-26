//! `/mcp` server panel: a centered modal overlay that stages server and tool
//! toggles until saved. Ports pi-mcp-adapter's `mcp-panel.ts`. The state
//! machine is pure (`handle_key` returns an action, `render` draws a width), so
//! the TUI wiring in `commands/mcp.rs` stays thin.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ice_tui::{theme as current_theme, truncate_to_width, truncate_to_width_padded, visible_width};

use crate::mcp::{DirectTools, PanelChange, ServerStatus, ServerView};

pub(super) const WIDTH: usize = 82;
const MAX_VISIBLE: usize = 12;

#[derive(Clone, Debug, PartialEq)]
pub(super) enum PanelAction {
    None,
    Close,
    /// Close and persist these edits (possibly none).
    Save(Vec<PanelChange>),
    Reconnect(String),
    CopyError(String),
}

struct Tool {
    name: String,
    description: String,
    tokens: usize,
    direct: bool,
    was_direct: bool,
}
struct Server {
    name: String,
    expanded: bool,
    disabled: bool,
    was_disabled: bool,
    status: ServerStatus,
    tools: Option<Vec<Tool>>,
}
impl Server {
    fn direct(&self) -> (usize, usize) {
        self.tools
            .iter()
            .flatten()
            .filter(|t| t.direct)
            .fold((0, 0), |(n, tokens), t| (n + 1, tokens + t.tokens))
    }
    fn failure(&self) -> Option<&str> {
        match &self.status {
            ServerStatus::Failed(error) => Some(error),
            _ => None,
        }
    }
}

pub(super) struct McpPanel {
    servers: Vec<Server>,
    cursor: usize,
    name_query: String,
    /// `Some` while `?` description search is active.
    desc_query: Option<String>,
    /// `Some(selected)` while confirming discard; 0 = Discard, 1 = Keep & Close.
    discard: Option<usize>,
    notice: Option<String>,
    visible: Vec<(usize, Option<usize>)>,
}

/// Server text reaches the terminal; replace control characters (including
/// ESC, so no escape sequence survives) and collapse whitespace.
pub(super) fn clean(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn tools_from(view: &ServerView, staged: Option<&[Tool]>) -> Option<Vec<Tool>> {
    let catalog = view.catalog.as_ref()?;
    Some(
        catalog
            .iter()
            .map(|t| Tool {
                name: t.name.clone(),
                description: clean(&t.description),
                tokens: t.tokens,
                direct: staged
                    .and_then(|s| s.iter().find(|s| s.name == t.name))
                    .map_or(t.direct, |s| s.direct),
                was_direct: t.direct,
            })
            .collect(),
    )
}

impl McpPanel {
    pub(super) fn new(views: &[ServerView]) -> Self {
        let mut panel = Self {
            servers: views
                .iter()
                .map(|view| Server {
                    name: view.name.clone(),
                    expanded: false,
                    disabled: view.disabled,
                    was_disabled: view.disabled,
                    status: view.status.clone(),
                    tools: tools_from(view, None),
                })
                .collect(),
            cursor: 0,
            name_query: String::new(),
            desc_query: None,
            discard: None,
            notice: None,
            visible: Vec::new(),
        };
        panel.rebuild();
        panel
    }

    /// Adopt fresh manager state (after a reconnect), keeping staged edits.
    pub(super) fn refresh(&mut self, views: &[ServerView]) {
        for view in views {
            let Some(server) = self.servers.iter_mut().find(|s| s.name == view.name) else {
                continue;
            };
            server.status = view.status.clone();
            server.was_disabled = view.disabled;
            server.tools = tools_from(view, server.tools.as_deref());
        }
        self.rebuild();
    }

    pub(super) fn set_notice(&mut self, notice: String) {
        self.notice = Some(notice);
    }

    pub(super) fn dirty(&self) -> bool {
        self.servers.iter().any(|s| {
            s.disabled != s.was_disabled
                || s.tools.iter().flatten().any(|t| t.direct != t.was_direct)
        })
    }

    pub(super) fn changes(&self) -> Vec<PanelChange> {
        self.servers
            .iter()
            .filter_map(|s| {
                let tools = s.tools.as_deref().unwrap_or_default();
                let direct_tools = tools.iter().any(|t| t.direct != t.was_direct).then(|| {
                    let direct: Vec<_> = tools.iter().filter(|t| t.direct).collect();
                    if direct.len() == tools.len() {
                        DirectTools::All(true)
                    } else if direct.is_empty() {
                        DirectTools::All(false)
                    } else {
                        DirectTools::Tools(direct.iter().map(|t| t.name.clone()).collect())
                    }
                });
                let disabled = (s.disabled != s.was_disabled).then_some(s.disabled);
                (disabled.is_some() || direct_tools.is_some()).then(|| PanelChange {
                    server: s.name.clone(),
                    disabled,
                    direct_tools,
                })
            })
            .collect()
    }

    fn query(&self) -> &str {
        self.desc_query.as_deref().unwrap_or(&self.name_query)
    }

    fn rebuild(&mut self) {
        let query = self.query().to_owned();
        let matches = |text: &str| ice_tui::fuzzy_match(&query, text).matches;
        self.visible.clear();
        for (si, server) in self.servers.iter().enumerate() {
            let tools: Vec<_> = server
                .tools
                .iter()
                .flatten()
                .enumerate()
                .filter(|(_, t)| match (&self.desc_query, query.is_empty()) {
                    (_, true) => server.expanded,
                    (Some(_), false) => matches(&t.description),
                    (None, false) => matches(&clean(&t.name)) || matches(&clean(&server.name)),
                })
                .map(|(ti, _)| (si, Some(ti)))
                .collect();
            // While searching, a server row only shows with matching tools.
            if query.is_empty() || !tools.is_empty() {
                self.visible.push((si, None));
                self.visible.extend(tools);
            }
        }
        self.cursor = self.cursor.min(self.visible.len().saturating_sub(1));
    }

    fn selected(&self) -> Option<(usize, Option<usize>)> {
        self.visible.get(self.cursor).copied()
    }

    fn move_cursor(&mut self, up: bool) {
        self.cursor = if up {
            self.cursor.saturating_sub(1)
        } else {
            (self.cursor + 1).min(self.visible.len().saturating_sub(1))
        };
    }

    fn toggle(&mut self, (si, ti): (usize, Option<usize>)) {
        let Some(tools) = self.servers[si].tools.as_mut() else {
            return;
        };
        match ti {
            Some(ti) => tools[ti].direct = !tools[ti].direct,
            None => {
                let all = !tools.iter().all(|t| t.direct);
                tools.iter_mut().for_each(|t| t.direct = all);
            }
        }
    }

    pub(super) fn handle_key(&mut self, key: KeyEvent) -> PanelAction {
        self.notice = None;
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let plain = !ctrl && !key.modifiers.contains(KeyModifiers::ALT);
        let code = ice_tui::get_keybindings()
            .selection_key(&key)
            .map_or(key.code, |k| k.code);
        if let Some(selected) = self.discard {
            return match code {
                KeyCode::Esc => {
                    self.discard = None;
                    PanelAction::None
                }
                KeyCode::Char('n' | 'N') if plain => {
                    self.discard = None;
                    PanelAction::None
                }
                KeyCode::Char('y' | 'Y') if plain => PanelAction::Close,
                KeyCode::Enter if selected == 0 => PanelAction::Close,
                KeyCode::Enter => PanelAction::Save(self.changes()),
                KeyCode::Left | KeyCode::Right | KeyCode::Tab => {
                    self.discard = Some(1 - selected);
                    PanelAction::None
                }
                _ => PanelAction::None,
            };
        }
        if ctrl && key.code == KeyCode::Char('s') {
            return PanelAction::Save(self.changes());
        }
        if let Some(desc) = self.desc_query.as_mut() {
            match code {
                KeyCode::Esc | KeyCode::Enter => self.desc_query = None,
                KeyCode::Backspace => {
                    desc.pop();
                }
                KeyCode::Up | KeyCode::Down => self.move_cursor(code == KeyCode::Up),
                KeyCode::Char(' ') if plain => {
                    if let Some(item) = self.selected() {
                        self.toggle(item);
                    }
                }
                KeyCode::Char(c) if plain => desc.push(c),
                _ => {}
            }
            self.rebuild();
            return PanelAction::None;
        }
        let item = self.selected();
        match code {
            KeyCode::Esc if !self.name_query.is_empty() => self.name_query.clear(),
            KeyCode::Esc if self.dirty() => self.discard = Some(1),
            KeyCode::Esc => return PanelAction::Close,
            KeyCode::Up | KeyCode::Down => self.move_cursor(code == KeyCode::Up),
            KeyCode::Char(' ') if plain => {
                if let Some(item) = item {
                    self.toggle(item);
                }
            }
            KeyCode::Enter => match item {
                Some((si, None)) if self.servers[si].tools.is_some() => {
                    self.servers[si].expanded = !self.servers[si].expanded
                }
                Some(item @ (_, Some(_))) => self.toggle(item),
                _ => {}
            },
            KeyCode::Char('r') if ctrl => {
                let Some((si, _)) = item else {
                    return PanelAction::None;
                };
                let server = &mut self.servers[si];
                if matches!(
                    server.status,
                    ServerStatus::Connecting | ServerStatus::Disabled
                ) {
                    return PanelAction::None;
                }
                server.status = ServerStatus::Connecting;
                return PanelAction::Reconnect(server.name.clone());
            }
            KeyCode::Char('d') if ctrl => {
                if let Some((si, None)) = item {
                    self.servers[si].disabled = !self.servers[si].disabled;
                }
            }
            KeyCode::Char('y') if ctrl => {
                if let Some(error) = item.and_then(|(si, _)| self.servers[si].failure()) {
                    return PanelAction::CopyError(clean(error));
                }
            }
            KeyCode::Char('?') if plain => self.desc_query = Some(String::new()),
            KeyCode::Backspace => {
                self.name_query.pop();
            }
            KeyCode::Char(c) if plain => self.name_query.push(c),
            _ => {}
        }
        self.rebuild();
        PanelAction::None
    }

    pub(super) fn render(&self, width: usize) -> Vec<String> {
        let colors = current_theme().colors;
        let frame = Frame::new(width);
        let inner = frame.inner;
        let row = |content: &str| frame.row(content);
        let empty = || frame.row("");
        let divider = || frame.divider();
        let mut lines = vec![frame.top("MCP Servers")];
        lines.push(empty());
        let caret = colors.accent.fg("│");
        lines.push(row(&match &self.desc_query {
            Some(desc) => format!("◎  {} {desc}{caret}", colors.warning.fg("desc:")),
            None if !self.name_query.is_empty() => format!("◎  {}{caret}", self.name_query),
            None => format!("◎  {}", colors.dim.fg("search...")),
        }));
        lines.push(empty());
        lines.push(divider());
        lines.push(empty());
        if self.servers.is_empty() {
            lines.push(row(&colors.dim.fg("No MCP servers configured.")));
        }
        let total = self.visible.len();
        let start = self
            .cursor
            .saturating_sub(MAX_VISIBLE / 2)
            .min(total.saturating_sub(MAX_VISIBLE));
        for (i, &(si, ti)) in self
            .visible
            .iter()
            .enumerate()
            .skip(start)
            .take(MAX_VISIBLE)
        {
            let selected = i == self.cursor;
            let server = &self.servers[si];
            match ti {
                None => {
                    lines.push(row(&self.server_row(server, selected)));
                    if let (true, Some(error)) = (selected, server.failure()) {
                        for line in ice_tui::wrap_text_with_ansi(
                            &clean(error),
                            inner.saturating_sub(6).max(8),
                        ) {
                            lines.push(row(&format!("    {}", colors.error.fg(&line))));
                        }
                    }
                }
                Some(ti) => {
                    let tool = &server.tools.as_ref().unwrap()[ti];
                    lines.push(row(&tool_row(tool, selected, inner)));
                }
            }
        }
        lines.push(empty());
        if total > MAX_VISIBLE {
            let filled = ((self.cursor + 1) * 10 + total / 2) / total;
            let dots: String = (0..10)
                .map(|i| if i < filled { '●' } else { '○' })
                .collect();
            lines.push(row(&format!(
                "{}  {}",
                colors.accent.fg(&dots),
                colors.dim.fg(&format!("{}/{total}", self.cursor + 1))
            )));
            lines.push(empty());
        }
        if let Some(notice) = &self.notice {
            lines.push(row(&colors.warning.fg(&clean(notice))));
            lines.push(empty());
        }
        lines.push(divider());
        lines.push(empty());
        if let Some(selected) = self.discard {
            let button = |label: &str, on: bool, color: &ice_tui::Color| {
                if on {
                    format!("\x1b[7m{}\x1b[27m", ice_tui::bold(&color.fg(label)))
                } else {
                    colors.dim.fg(label)
                }
            };
            lines.push(row(&format!(
                "Discard unsaved changes?  {}   {}",
                button("  Discard  ", selected == 0, &colors.error),
                button("  Keep & Close  ", selected == 1, &colors.success)
            )));
        } else {
            let (count, tokens) = self
                .servers
                .iter()
                .map(Server::direct)
                .fold((0, 0), |(a, b), (c, d)| (a + c, b + d));
            let stats = if count > 0 {
                format!("{count} direct  ~{} tokens", thousands(tokens))
            } else {
                "no direct tools".into()
            };
            let unsaved = if self.dirty() {
                colors.warning.fg("  (unsaved)")
            } else {
                String::new()
            };
            lines.push(row(&format!("{}{unsaved}", colors.dim.fg(&stats))));
        }
        lines.push(empty());
        let failed = self
            .selected()
            .is_some_and(|(si, _)| self.servers[si].failure().is_some());
        let hints = [
            Some("↑↓ navigate"),
            Some("space toggle"),
            Some("⏎ expand"),
            Some("ctrl+r reconnect"),
            Some("ctrl+d disable/enable"),
            failed.then_some("ctrl+y copy error"),
            Some("? desc search"),
            Some("ctrl+s save"),
            Some("esc clear/close"),
        ];
        lines.extend(frame.hints(&hints.into_iter().flatten().collect::<Vec<_>>()));
        lines.push(frame.bottom());
        lines
    }

    fn server_row(&self, server: &Server, selected: bool) -> String {
        let colors = current_theme().colors;
        let arrow = if server.expanded { "▾" } else { "▸" };
        let prefix = if selected {
            colors.accent.fg(arrow)
        } else {
            colors.dim.fg(if server.expanded { arrow } else { "·" })
        };
        let name = clean(&server.name);
        let name = if selected {
            ice_tui::bold(&colors.accent.fg(&name))
        } else {
            name
        };
        let status = if server.disabled {
            format!("  {}", colors.dim.fg("disabled"))
        } else {
            match server.status {
                ServerStatus::Connecting => format!("  {}", colors.warning.fg("connecting")),
                ServerStatus::Failed(_) => format!("  {}", colors.error.fg("failed")),
                _ => String::new(),
            }
        };
        let Some(tools) = &server.tools else {
            return format!(
                "{prefix}   {name}  {}{status}",
                colors.dim.fg("(not cached)")
            );
        };
        let (count, tokens) = server.direct();
        let icon = if count == tools.len() && count > 0 {
            colors.success.fg("●")
        } else if count > 0 {
            colors.warning.fg("◐")
        } else {
            colors.dim.fg("○")
        };
        let mut info = String::new();
        if !tools.is_empty() {
            info = format!("{count}/{}", tools.len());
            if count > 0 {
                info.push_str(&format!("  ~{}", thousands(tokens)));
            }
        }
        format!("{prefix} {icon} {name}  {}{status}", colors.dim.fg(&info))
    }
}

/// The rounded box both `/mcp` overlays draw in; `inner` excludes the borders.
pub(super) struct Frame {
    pub(super) inner: usize,
}
impl Frame {
    pub(super) fn new(width: usize) -> Self {
        Self {
            inner: width.saturating_sub(2),
        }
    }
    fn border(s: &str) -> String {
        current_theme().colors.border_muted.fg(s)
    }
    pub(super) fn top(&self, title: &str) -> String {
        let title = format!(" {title} ");
        let fill = self.inner.saturating_sub(visible_width(&title));
        format!(
            "{}{}{}",
            Self::border(&format!("╭{}", "─".repeat(fill / 2))),
            ice_tui::bold(&title),
            Self::border(&format!("{}╮", "─".repeat(fill - fill / 2)))
        )
    }
    pub(super) fn row(&self, content: &str) -> String {
        format!(
            "{}{}{}",
            Self::border("│"),
            truncate_to_width_padded(&format!(" {content}"), self.inner, "…"),
            Self::border("│")
        )
    }
    pub(super) fn divider(&self) -> String {
        Self::border(&format!("├{}┤", "─".repeat(self.inner)))
    }
    pub(super) fn bottom(&self) -> String {
        Self::border(&format!("╰{}╯", "─".repeat(self.inner)))
    }
    /// Key hints joined by two spaces, wrapped to the box width.
    pub(super) fn hints(&self, hints: &[&str]) -> Vec<String> {
        let dim = |s: &str| current_theme().colors.dim.fg(s);
        let mut lines = Vec::new();
        let mut line = String::new();
        for hint in hints {
            if !line.is_empty()
                && visible_width(&line) + 2 + visible_width(hint) > self.inner.saturating_sub(2)
            {
                lines.push(self.row(&dim(&line)));
                line.clear();
            }
            if !line.is_empty() {
                line.push_str("  ");
            }
            line.push_str(hint);
        }
        if !line.is_empty() {
            lines.push(self.row(&dim(&line)));
        }
        lines
    }
}

fn tool_row(tool: &Tool, selected: bool, inner: usize) -> String {
    let colors = current_theme().colors;
    let icon = if tool.direct {
        colors.success.fg("●")
    } else {
        colors.dim.fg("○")
    };
    let cursor = if selected {
        colors.accent.fg("▸")
    } else {
        " ".into()
    };
    let name = clean(&tool.name);
    let room = inner.saturating_sub(7 + visible_width(&name) + 8);
    let name = if selected {
        ice_tui::bold(&colors.accent.fg(&name))
    } else {
        name
    };
    let description = if room > 5 && !tool.description.is_empty() {
        colors.dim.fg(&format!(
            "— {}",
            truncate_to_width(&tool.description, room, "…")
        ))
    } else {
        String::new()
    };
    format!("  {cursor} {icon} {name} {description}")
}

fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Overlay component: the panel behind a mutex shared with the key router.
pub(super) struct McpPanelView(pub(super) std::sync::Mutex<McpPanel>);
impl ice_tui::Component for McpPanelView {
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
    use crate::mcp::ToolInfo;
    use ice_tui::strip_ansi;

    fn view(name: &str, status: ServerStatus, tools: Option<&[(&str, &str, bool)]>) -> ServerView {
        ServerView {
            name: name.into(),
            source: "mcp.json".into(),
            disabled: matches!(status, ServerStatus::Disabled),
            status,
            catalog: tools.map(|tools| {
                tools
                    .iter()
                    .map(|&(name, description, direct)| ToolInfo {
                        name: name.into(),
                        description: description.into(),
                        tokens: 100,
                        direct,
                    })
                    .collect()
            }),
        }
    }
    fn connected() -> ServerStatus {
        ServerStatus::Connected { tools: vec![] }
    }
    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }
    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }
    fn typed(panel: &mut McpPanel, text: &str) {
        for c in text.chars() {
            assert_eq!(panel.handle_key(key(KeyCode::Char(c))), PanelAction::None);
        }
    }
    fn sample() -> McpPanel {
        McpPanel::new(&[
            view(
                "alpha",
                connected(),
                Some(&[
                    ("read", "Read a file", true),
                    ("write", "Write a file", true),
                ]),
            ),
            view("beta", connected(), Some(&[("list", "List records", true)])),
        ])
    }
    fn text(panel: &McpPanel) -> Vec<String> {
        panel.render(WIDTH).iter().map(|l| strip_ansi(l)).collect()
    }

    #[test]
    fn search_filters_tools_and_escape_clears_then_closes() {
        let mut panel = sample();
        assert_eq!(panel.visible, vec![(0, None), (1, None)]);
        typed(&mut panel, "wri");
        assert_eq!(panel.visible, vec![(0, None), (0, Some(1))]);
        assert!(text(&panel).iter().any(|l| l.contains("◎  wri│")));
        typed(&mut panel, "?");
        typed(&mut panel, "records");
        assert_eq!(panel.visible, vec![(1, None), (1, Some(0))]);
        assert!(text(&panel).iter().any(|l| l.contains("desc: records")));
        panel.handle_key(key(KeyCode::Enter));
        assert_eq!(panel.query(), "wri");
        panel.handle_key(key(KeyCode::Esc));
        assert_eq!(panel.visible.len(), 2);
        assert_eq!(panel.handle_key(key(KeyCode::Esc)), PanelAction::Close);
    }

    #[test]
    fn toggles_stage_direct_tools_and_disabled() {
        let mut panel = sample();
        panel.handle_key(key(KeyCode::Enter));
        assert_eq!(panel.visible.len(), 4);
        panel.handle_key(key(KeyCode::Char(' ')));
        assert!(panel.dirty());
        assert_eq!(
            panel.changes(),
            vec![PanelChange {
                server: "alpha".into(),
                disabled: None,
                direct_tools: Some(DirectTools::All(false)),
            }]
        );
        panel.handle_key(key(KeyCode::Down));
        panel.handle_key(key(KeyCode::Enter));
        assert_eq!(
            panel.changes()[0].direct_tools,
            Some(DirectTools::Tools(vec!["read".into()]))
        );
        assert!(text(&panel)
            .iter()
            .any(|l| l.contains("▾ ◐ alpha  1/2  ~100")));
        panel.handle_key(key(KeyCode::Down));
        panel.handle_key(key(KeyCode::Char(' ')));
        assert!(!panel.dirty());
        // Collapsing hides tool rows again.
        panel.handle_key(key(KeyCode::Up));
        panel.handle_key(key(KeyCode::Up));
        panel.handle_key(key(KeyCode::Enter));
        assert_eq!(panel.visible.len(), 2);
        panel.handle_key(key(KeyCode::Down));
        panel.handle_key(ctrl('d'));
        assert_eq!(
            panel.changes(),
            vec![PanelChange {
                server: "beta".into(),
                disabled: Some(true),
                direct_tools: None,
            }]
        );
        assert!(text(&panel)
            .iter()
            .any(|l| l.contains("beta  1/1  ~100  disabled")));
        let footer = text(&panel).join("\n");
        assert!(
            footer.contains("3 direct  ~300 tokens  (unsaved)"),
            "{footer}"
        );
        assert_eq!(
            panel.handle_key(ctrl('s')),
            PanelAction::Save(panel.changes())
        );
    }

    #[test]
    fn tool_identifiers_survive_save_and_refresh_but_render_cleanly() {
        let raw = "read\t\x1b]0;tool\x07  file";
        let views = [view(
            "alpha",
            connected(),
            Some(&[(raw, "Read a file", false), ("other", "", true)]),
        )];
        let mut panel = McpPanel::new(&views);
        panel.handle_key(key(KeyCode::Enter));
        panel.handle_key(key(KeyCode::Down));
        panel.handle_key(key(KeyCode::Char(' ')));
        panel.handle_key(key(KeyCode::Down));
        panel.handle_key(key(KeyCode::Char(' ')));
        let changes = vec![PanelChange {
            server: "alpha".into(),
            disabled: None,
            direct_tools: Some(DirectTools::Tools(vec![raw.into()])),
        }];
        assert_eq!(panel.changes(), changes);
        panel.refresh(&views);
        assert_eq!(panel.changes(), changes);
        assert_eq!(panel.handle_key(ctrl('s')), PanelAction::Save(changes));
        let rendered = panel.render(WIDTH).join("\n");
        assert!(!rendered.contains("\x1b]") && !rendered.contains('\x07'));
        assert!(strip_ansi(&rendered).contains(&clean(raw)));
    }

    #[test]
    fn server_identifiers_survive_actions_and_refresh_but_render_cleanly() {
        let raw = "  alpha\t\x1b]0;server\x07  ";
        let mut panel = McpPanel::new(&[view(raw, connected(), None)]);
        assert_eq!(
            panel.handle_key(ctrl('r')),
            PanelAction::Reconnect(raw.into())
        );
        panel.refresh(&[view(raw, connected(), Some(&[("read", "", true)]))]);
        assert!(panel.servers[0].tools.is_some());
        panel.handle_key(ctrl('d'));
        assert_eq!(
            panel.changes(),
            vec![PanelChange {
                server: raw.into(),
                disabled: Some(true),
                direct_tools: None,
            }]
        );
        let rendered = panel.render(WIDTH).join("\n");
        assert!(!rendered.contains("\x1b]") && !rendered.contains('\x07'));
        assert!(strip_ansi(&rendered).contains(&clean(raw)));
    }

    #[test]
    fn discard_confirm_keeps_or_discards() {
        let mut panel = sample();
        panel.handle_key(key(KeyCode::Char(' ')));
        assert_eq!(panel.handle_key(key(KeyCode::Esc)), PanelAction::None);
        assert!(text(&panel)
            .iter()
            .any(|l| l.contains("Discard unsaved changes?")));
        panel.handle_key(key(KeyCode::Char('n')));
        assert_eq!(panel.discard, None);
        panel.handle_key(key(KeyCode::Esc));
        assert_eq!(
            panel.handle_key(key(KeyCode::Enter)),
            PanelAction::Save(panel.changes())
        );
        panel.handle_key(key(KeyCode::Tab));
        assert_eq!(panel.discard, Some(0));
        assert_eq!(panel.handle_key(key(KeyCode::Enter)), PanelAction::Close);
        panel.discard = Some(1);
        assert_eq!(
            panel.handle_key(key(KeyCode::Char('y'))),
            PanelAction::Close
        );
    }

    #[test]
    fn reconnect_copy_error_and_refresh_keep_staged_edits() {
        let failed = ServerStatus::Failed("boom\x1b[31m\nline two".into());
        let mut panel = McpPanel::new(&[
            view("ok", connected(), Some(&[("a", "", true), ("b", "", true)])),
            view("bad", failed.clone(), None),
        ]);
        panel.handle_key(key(KeyCode::Enter));
        panel.handle_key(key(KeyCode::Down));
        panel.handle_key(key(KeyCode::Char(' ')));
        panel.handle_key(key(KeyCode::Down));
        panel.handle_key(key(KeyCode::Down));
        // Uncached servers cannot expand.
        panel.handle_key(key(KeyCode::Enter));
        assert_eq!(panel.visible.len(), 4);
        let lines = text(&panel);
        assert!(lines
            .iter()
            .any(|l| l.contains("bad  (not cached)  failed")));
        assert!(lines.iter().any(|l| l.contains("boom [31m line two")));
        assert!(lines.iter().any(|l| l.contains("ctrl+y copy error")));
        assert_eq!(
            panel.handle_key(ctrl('y')),
            PanelAction::CopyError("boom [31m line two".into())
        );
        assert_eq!(
            panel.handle_key(ctrl('r')),
            PanelAction::Reconnect("bad".into())
        );
        assert!(text(&panel).iter().any(|l| l.contains("connecting")));
        assert_eq!(panel.handle_key(ctrl('r')), PanelAction::None);
        panel.refresh(&[
            view(
                "ok",
                connected(),
                Some(&[("a", "", true), ("b", "", true), ("c", "", true)]),
            ),
            view("bad", connected(), Some(&[("x", "", true)])),
        ]);
        let ok = &panel.servers[0];
        let direct: Vec<_> = ok.tools.iter().flatten().map(|t| t.direct).collect();
        assert_eq!(direct, [false, true, true]);
        assert!(panel.servers[1].tools.is_some());
        assert_eq!(panel.changes().len(), 1);
    }

    #[test]
    fn render_is_centered_box_with_truncation_scrolling_and_clean_text() {
        let long = "word ".repeat(40);
        let tools: Vec<(String, String)> = (0..20)
            .map(|i| {
                (
                    format!("tool{i:02}"),
                    if i == 0 {
                        long.clone()
                    } else {
                        "\x1b]0;pwn\x07ok".into()
                    },
                )
            })
            .collect();
        let tools: Vec<_> = tools
            .iter()
            .map(|(n, d)| (n.as_str(), d.as_str(), true))
            .collect();
        let mut panel = McpPanel::new(&[view("many", connected(), Some(&tools))]);
        panel.handle_key(key(KeyCode::Enter));
        let rendered = panel.render(WIDTH);
        assert!(rendered.iter().all(|l| visible_width(l) == WIDTH));
        assert!(rendered
            .iter()
            .all(|l| !l.contains("\x1b]") && !l.contains('\x07')));
        let lines = text(&panel);
        assert!(
            lines[0].starts_with("╭")
                && lines[0].contains(" MCP Servers ")
                && lines[0].ends_with("╮")
        );
        assert!(lines.last().unwrap().starts_with("╰"));
        let rows = lines
            .iter()
            .filter(|l| l.contains("tool") || l.contains("many"))
            .count();
        assert_eq!(rows, MAX_VISIBLE);
        assert!(lines
            .iter()
            .any(|l| l.contains("tool00 — word") && l.contains('…')));
        assert!(lines.iter().any(|l| l.contains("1/21")));
        assert!(lines.iter().any(|l| l.contains("20 direct  ~2,000 tokens")));
        for _ in 0..30 {
            panel.handle_key(key(KeyCode::Down));
        }
        let lines = text(&panel);
        assert!(lines.iter().any(|l| l.contains("21/21")));
        assert!(lines.iter().any(|l| l.contains("tool19")));
        assert!(!lines.iter().any(|l| l.contains("many")));
        // Narrow terminals shrink the overlay; rows still fit exactly.
        assert!(panel.render(40).iter().all(|l| visible_width(l) == 40));
        assert_eq!(thousands(1234567), "1,234,567");
        assert_eq!(clean("a\tb\u{9b}c\n"), "a b c");
    }
}
