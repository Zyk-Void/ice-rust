//! Shells panel: one live view over the background shells (bash commands
//! started with `run_in_background` or moved there with Ctrl+B). It replaces
//! the editor while open, so the transcript above stays visible. A list shows
//! the running shells; details show one shell's status, runtime, command and
//! latest output; follow mode brings a shell's output to the foreground as a
//! taller live tail. Shells can be stopped from any view, and a shell that
//! exits while its details or follow view is open stays there until the user
//! goes back to the list. Mirrors ice `shells-panel.ts`.
//!
//! Unlike ice, every shell is named `bash` and told apart by what it does
//! (its description, or the first command line); the random id is shown
//! dimmed, since it is for the model's `bash_output`/`bash_kill`, not a
//! label for a person.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossterm::event::KeyEvent;
use ice_tools::bash_background::{
    format_runtime, BackgroundShellSnapshot, BackgroundShellStatus, BashBackgroundRegistry,
    KillOutcome, ListenerId, ShellOrigin,
};
use ice_tui::component::Component;
use ice_tui::keybinding_hints::key_text;
use ice_tui::keybindings::{keys, Keybindings};
use ice_tui::TUI as _;
use ice_tui::{theme, truncate_to_width, visible_width, wrap_text_with_ansi};

/// List rows shown at once; the window follows the selection.
const LIST_WINDOW: usize = 10;
/// Output rows in the details view.
const DETAIL_OUTPUT_ROWS: usize = 8;
/// Output rows while following a shell.
const FOLLOW_OUTPUT_ROWS: usize = 24;
/// Command rows in the details view before it is cut.
const COMMAND_ROWS: usize = 3;
/// Width of the details labels (`Status:` …).
const LABEL_WIDTH: usize = 10;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum View {
    List,
    Details,
    Follow,
}

struct PanelState {
    view: View,
    /// Selected shell by id, so the selection stays put when shells start or
    /// finish.
    selected: Option<String>,
}

type Callback = Arc<dyn Fn() + Send + Sync>;

pub(super) struct ShellsPanel {
    registry: Arc<BashBackgroundRegistry>,
    keybindings: Arc<Keybindings>,
    on_close: Callback,
    request_render: Callback,
    now: Arc<dyn Fn() -> i64 + Send + Sync>,
    runtime: Option<tokio::runtime::Handle>,
    state: Mutex<PanelState>,
    /// Result of the last stop, shown until the next key. Shared with the
    /// task that performs the (async) stop.
    message: Arc<Mutex<Option<String>>>,
    listener: Mutex<Option<ListenerId>>,
    ticker_stop: Arc<AtomicBool>,
}

/// Test hooks; the host uses the defaults.
#[derive(Default)]
pub(super) struct ShellsPanelOptions {
    pub(super) now: Option<Arc<dyn Fn() -> i64 + Send + Sync>>,
    /// Skip the one-second runtime repaint.
    pub(super) no_tick: bool,
}

impl ShellsPanel {
    pub(super) fn new(
        registry: Arc<BashBackgroundRegistry>,
        keybindings: Arc<Keybindings>,
        on_close: Callback,
        request_render: Callback,
        options: ShellsPanelOptions,
    ) -> Arc<Self> {
        // One running shell: open straight on its details. Several: start on
        // the newest one.
        let running = registry.list();
        let view = if running.len() == 1 {
            View::Details
        } else {
            View::List
        };
        let selected = running.last().map(|shell| shell.task_id.clone());
        let render = request_render.clone();
        let listener = registry.on_change(move || render());
        let ticker_stop = Arc::new(AtomicBool::new(false));
        if !options.no_tick {
            // Runtimes advance even when a shell prints nothing.
            let (stop, render) = (ticker_stop.clone(), request_render.clone());
            std::thread::spawn(move || loop {
                std::thread::sleep(Duration::from_secs(1));
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                render();
            });
        }
        Arc::new(Self {
            registry,
            keybindings,
            on_close,
            request_render,
            now: options.now.unwrap_or_else(|| Arc::new(now_ms)),
            runtime: tokio::runtime::Handle::try_current().ok(),
            state: Mutex::new(PanelState { view, selected }),
            message: Arc::new(Mutex::new(None)),
            listener: Mutex::new(Some(listener)),
            ticker_stop,
        })
    }

    /// Stop the live updates. Safe to call more than once.
    fn dispose(&self) {
        self.ticker_stop.store(true, Ordering::Relaxed);
        if let Some(listener) = self.listener.lock().unwrap().take() {
            self.registry.unsubscribe(listener);
        }
    }

    fn close(&self) {
        self.dispose();
        (self.on_close)();
    }

    /// Running shells, oldest first; the shell open in details or follow
    /// stays after it exits.
    fn shells(&self, state: &PanelState) -> Vec<BackgroundShellSnapshot> {
        self.registry
            .snapshots()
            .into_iter()
            .filter(|shell| {
                shell.status == BackgroundShellStatus::Running
                    || (state.view != View::List
                        && state.selected.as_deref() == Some(shell.task_id.as_str()))
            })
            .collect()
    }

    fn selected_index(shells: &[BackgroundShellSnapshot], state: &PanelState) -> usize {
        shells
            .iter()
            .position(|shell| state.selected.as_deref() == Some(shell.task_id.as_str()))
            .unwrap_or(0)
    }

    pub(super) fn handle_key(&self, key: KeyEvent) {
        let kb = &self.keybindings;
        *self.message.lock().unwrap() = None;
        if kb.matches(&key, keys::SELECT_CANCEL) {
            self.close();
            return;
        }
        let mut state = self.state.lock().unwrap();
        let shells = self.shells(&state);
        let index = Self::selected_index(&shells, &state);
        let selected = shells.get(index);
        if kb.matches(&key, keys::SHELLS_STOP) {
            // Release the state first: stopping notifies listeners, and a
            // repaint reads this panel's state.
            let id = state.selected.as_deref().and_then(|selected_id| {
                shells
                    .iter()
                    .find(|shell| shell.task_id == selected_id)
                    .map(|shell| shell.task_id.clone())
            });
            drop(state);
            if let Some(id) = id {
                self.stop(id);
            }
            (self.request_render)();
            return;
        }
        if state.view == View::List {
            if kb.matches(&key, keys::SELECT_UP) {
                state.selected = shells
                    .get(index.saturating_sub(1))
                    .map(|s| s.task_id.clone());
            } else if kb.matches(&key, keys::SELECT_DOWN) {
                state.selected = shells
                    .get((index + 1).min(shells.len().saturating_sub(1)))
                    .map(|s| s.task_id.clone());
            } else if kb.matches(&key, keys::SELECT_CONFIRM) || kb.matches(&key, keys::SELECT_RIGHT)
            {
                if selected.is_some() {
                    state.view = View::Details;
                }
            } else if kb.matches(&key, keys::SHELLS_FOLLOW) {
                if selected.is_some() {
                    state.view = View::Follow;
                }
            } else {
                return;
            }
        } else if kb.matches(&key, keys::SELECT_LEFT) {
            // Back: follow -> details -> list.
            state.view = if state.view == View::Follow {
                View::Details
            } else {
                View::List
            };
        } else if state.view == View::Details && kb.matches(&key, keys::SHELLS_FOLLOW) {
            state.view = View::Follow;
        } else if state.view == View::Details && kb.matches(&key, keys::SELECT_CONFIRM) {
            drop(state);
            self.close();
            return;
        } else {
            return;
        }
        drop(state);
        (self.request_render)();
    }

    /// Stop a shell and tell the model, as `bash_kill` would with a user
    /// notification. The registry's stop is async; the result shows when it
    /// lands.
    fn stop(&self, id: String) {
        let (registry, message, render) = (
            self.registry.clone(),
            self.message.clone(),
            self.request_render.clone(),
        );
        let task = async move {
            let text = match registry.kill(&id, true).await {
                Ok(KillOutcome::Stopped) => format!("Stopped {id}."),
                Ok(KillOutcome::StoppedLingering) => {
                    format!("Stopped the processes {id} left running.")
                }
                Ok(KillOutcome::AlreadyFinished(status)) => {
                    format!("{id} is already {}.", status.as_str())
                }
                Err(error) => error,
            };
            *message.lock().unwrap() = Some(text);
            render();
        };
        match &self.runtime {
            Some(runtime) => {
                runtime.spawn(task);
            }
            None => futures::executor::block_on(task),
        }
    }

    fn header(&self, shells: &[BackgroundShellSnapshot]) -> String {
        let c = theme().colors;
        let count = if shells.is_empty() {
            c.muted.fg("no running shells")
        } else {
            c.accent.fg(&format!("{} running", shells.len()))
        };
        format!("{}  {count}", ice_tui::bold("Shells"))
    }

    fn list_lines(
        &self,
        shells: &[BackgroundShellSnapshot],
        state: &PanelState,
        width: usize,
    ) -> Vec<String> {
        let c = theme().colors;
        if shells.is_empty() {
            return vec![c.muted.fg(
                "No running background shells. Commands the agent starts with run_in_background, \
                 or that you move to the background, appear here while they run.",
            )];
        }
        let selected = Self::selected_index(shells, state);
        let start = selected
            .saturating_sub(LIST_WINDOW / 2)
            .min(shells.len().saturating_sub(LIST_WINDOW));
        let end = shells.len().min(start + LIST_WINDOW);
        let now = (self.now)();
        let mut lines = Vec::new();
        if start > 0 {
            lines.push(c.dim.fg(&format!("  ↑ {start} more")));
        }
        for (index, shell) in shells.iter().enumerate().take(end).skip(start) {
            let is_selected = index == selected;
            let pointer = if is_selected {
                c.accent.fg("›")
            } else {
                " ".to_string()
            };
            let runtime = format!("{:>7}", runtime(shell, now));
            let label = if is_selected {
                ice_tui::bold(&label(shell))
            } else {
                label(shell)
            };
            let mut row = format!(
                "{pointer} {} {}  {}  {label}",
                status_mark(shell),
                c.dim.fg(&runtime),
                c.muted.fg("bash")
            );
            if shell.description.is_some() {
                row.push_str(&c.dim.fg(&format!(" · {}", first_line(&shell.command))));
            }
            row.push_str(&c.dim.fg(&format!("  {}", shell.task_id)));
            lines.push(one_row(&row, width));
        }
        if end < shells.len() {
            lines.push(c.dim.fg(&format!("  ↓ {} more", shells.len() - end)));
        }
        lines
    }

    fn output_lines(
        &self,
        shell: &BackgroundShellSnapshot,
        rows: usize,
        width: usize,
    ) -> Vec<String> {
        let c = theme().colors;
        // A finished shell's final text holds its output and exit status; a
        // stopped one only has a note.
        let source = match (shell.status, &shell.final_text) {
            (BackgroundShellStatus::Killed, final_text) => format!(
                "{}\n[{}]",
                shell.output.trim_end(),
                final_text.as_deref().unwrap_or("Stopped.")
            ),
            (BackgroundShellStatus::Running, _) | (_, None) => shell.output.clone(),
            (_, Some(final_text)) => final_text.clone(),
        };
        let text = plain_output(&source);
        let text = text.trim();
        if text.is_empty() {
            let empty = if shell.status == BackgroundShellStatus::Running {
                "No output yet."
            } else {
                "No output."
            };
            return vec![c.muted.fg(empty)];
        }
        let wrapped: Vec<String> = text
            .split('\n')
            .flat_map(|line| wrap_text_with_ansi(line, width.saturating_sub(2).max(1)))
            .collect();
        let hidden = wrapped.len().saturating_sub(rows);
        let mut lines: Vec<String> = Vec::new();
        if hidden > 0 {
            lines.push(c.dim.fg(&format!("  … {hidden} earlier lines")));
        }
        lines.extend(
            wrapped[hidden..]
                .iter()
                .map(|line| format!("  {}", c.tool_output.fg(line))),
        );
        lines
    }

    fn detail_lines(
        &self,
        shell: &BackgroundShellSnapshot,
        width: usize,
        rows: usize,
    ) -> Vec<String> {
        let c = theme().colors;
        let label = |name: &str| c.muted.fg(&format!("{name:<LABEL_WIDTH$}"));
        let blank = " ".repeat(LABEL_WIDTH);
        let origin = match shell.origin {
            ShellOrigin::Model => "started by the agent",
            ShellOrigin::User => "moved to the background by you",
        };
        let mut lines = vec![
            format!(
                "{}{}{}",
                label("Status:"),
                status_text(shell),
                c.dim.fg(&format!(" · {origin}"))
            ),
            format!("{}{}", label("Runtime:"), runtime(shell, (self.now)())),
        ];
        if let Some(description) = &shell.description {
            lines.push(format!("{}{}", label("About:"), safe_metadata(description)));
        }
        let safe_command = plain_output(&shell.command);
        let command: Vec<String> = safe_command
            .split('\n')
            .flat_map(|line| wrap_text_with_ansi(line, width.saturating_sub(LABEL_WIDTH).max(1)))
            .collect();
        for (index, line) in command.iter().take(COMMAND_ROWS).enumerate() {
            let prefix = if index == 0 {
                label("Command:")
            } else {
                blank.clone()
            };
            lines.push(format!("{prefix}{line}"));
        }
        if command.len() > COMMAND_ROWS {
            lines.push(format!(
                "{blank}{}",
                c.dim
                    .fg(&format!("… {} more lines", command.len() - COMMAND_ROWS))
            ));
        }
        lines.push(format!("{}{}", label("Id:"), c.dim.fg(&shell.task_id)));
        if let Some(log) = &shell.log_path {
            lines.push(format!(
                "{}{}",
                label("Log:"),
                c.dim.fg(&safe_metadata(log))
            ));
        }
        lines.push(String::new());
        lines.push(label("Output:"));
        lines.extend(self.output_lines(shell, rows, width));
        lines
            .into_iter()
            .map(|line| one_row(&line, width))
            .collect()
    }

    fn hints(&self, view: View, selected: Option<&BackgroundShellSnapshot>) -> String {
        let running = selected.is_some_and(|s| s.status == BackgroundShellStatus::Running);
        let stop = running.then(|| format!("{} stop", key_text(keys::SHELLS_STOP)));
        let parts: Vec<String> = match view {
            View::List => [
                Some(format!(
                    "{}/{} select",
                    key_text(keys::SELECT_UP),
                    key_text(keys::SELECT_DOWN)
                )),
                Some(format!("{} details", key_text(keys::SELECT_CONFIRM))),
                Some(format!("{} follow", key_text(keys::SHELLS_FOLLOW))),
                stop,
                Some(format!("{} close", key_text(keys::SELECT_CANCEL))),
            ]
            .into_iter()
            .flatten()
            .collect(),
            View::Details => [
                Some(format!("{} back", key_text(keys::SELECT_LEFT))),
                Some(format!("{} follow", key_text(keys::SHELLS_FOLLOW))),
                stop,
                Some(format!(
                    "{}/{} close",
                    key_text(keys::SELECT_CANCEL),
                    key_text(keys::SELECT_CONFIRM)
                )),
            ]
            .into_iter()
            .flatten()
            .collect(),
            View::Follow => [
                Some(format!("{} back", key_text(keys::SELECT_LEFT))),
                stop,
                Some(format!("{} close", key_text(keys::SELECT_CANCEL))),
            ]
            .into_iter()
            .flatten()
            .collect(),
        };
        parts.join(" · ")
    }
}

impl Drop for ShellsPanel {
    fn drop(&mut self) {
        self.dispose();
    }
}

impl Component for ShellsPanel {
    fn invalidate(&self) {}

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn render(&self, width: usize) -> Vec<String> {
        let c = theme().colors;
        let mut state = self.state.lock().unwrap();
        let shells = self.shells(&state);
        let selected = shells.get(Self::selected_index(&shells, &state)).cloned();
        state.selected = selected.as_ref().map(|s| s.task_id.clone());
        if selected.is_none() {
            state.view = View::List;
        }
        let mut lines = vec![c.border.fg(&"─".repeat(width))];
        match (&selected, state.view) {
            (Some(shell), View::Details) => {
                lines.push(one_row(
                    &format!(
                        "{}  {}  {}",
                        ice_tui::bold("bash"),
                        label(shell),
                        c.dim.fg(&shell.task_id)
                    ),
                    width,
                ));
                lines.push(String::new());
                lines.extend(self.detail_lines(shell, width, DETAIL_OUTPUT_ROWS));
            }
            (Some(shell), View::Follow) => {
                let live = if shell.status == BackgroundShellStatus::Running {
                    c.accent.fg("live")
                } else {
                    status_text(shell)
                };
                lines.push(one_row(
                    &format!(
                        "{}  {live}  {}",
                        ice_tui::bold(&format!("Following {}", label(shell))),
                        c.dim.fg(&shell.task_id)
                    ),
                    width,
                ));
                lines.push(String::new());
                lines.extend(
                    self.output_lines(shell, FOLLOW_OUTPUT_ROWS, width)
                        .iter()
                        .map(|line| one_row(line, width)),
                );
            }
            _ => {
                lines.push(self.header(&shells));
                lines.push(String::new());
                lines.extend(self.list_lines(&shells, &state, width));
            }
        }
        if let Some(message) = self.message.lock().unwrap().clone() {
            lines.push(String::new());
            lines.push(c.muted.fg(&one_row(&message, width)));
        }
        lines.push(String::new());
        lines.push(
            c.dim
                .fg(&one_row(&self.hints(state.view, selected.as_ref()), width)),
        );
        lines
            .into_iter()
            .map(|line| {
                if visible_width(&line) > width {
                    truncate_to_width(&line, width, "…")
                } else {
                    line
                }
            })
            .collect()
    }
}

/// Open the Shells panel in the editor slot (Down on an empty editor, a click
/// on the footer's shell count, or `/shells`).
pub(super) fn open_shells_panel(ctx: &super::CommandContext) {
    let close = {
        let (state, container, editor, tui) = (
            ctx.state.clone(),
            ctx.editor_container.clone(),
            ctx.editor.clone(),
            ctx.tui.clone(),
        );
        Arc::new(move || super::close_selector(&state, &container, &editor, &tui)) as Callback
    };
    let render = {
        let tui = ctx.tui.clone();
        Arc::new(move || tui.request_render(false)) as Callback
    };
    let panel = ShellsPanel::new(
        ctx.reload_context.shells.registry(),
        Arc::new(ice_tui::get_keybindings().clone()),
        close,
        render,
        ShellsPanelOptions::default(),
    );
    let router = panel.clone();
    super::open_selector_with_view(
        &ctx.state,
        &ctx.editor_container,
        &ctx.editor,
        &ctx.tui,
        super::SelectorView::Custom(Arc::new(move |key| router.handle_key(key))),
        panel,
        super::SelectorKind::Shells,
    );
}

/// Output lines kept under a finished shell's status line in the transcript.
const COMPLETION_TAIL_ROWS: usize = 3;

/// A finished background shell as the transcript shows it: one status line,
/// e.g. `✓ Background bash "Run tests" completed (exit code 0) · 4s`
/// (ice `bashCompletionLine`), then the last lines of its result, dimmed.
/// The model still receives the full text with the shell's id. Reads both
/// the Rust (snake_case) and ice (camelCase) completion fields.
pub(super) fn completion_lines(details: &serde_json::Value) -> Vec<String> {
    let c = theme().colors;
    let field = |snake: &str, camel: &str| details.get(snake).or_else(|| details.get(camel));
    let text = |snake, camel| {
        field(snake, camel)
            .and_then(|v| v.as_str())
            .map(str::to_owned)
    };
    let flag = |snake, camel| field(snake, camel).and_then(|v| v.as_bool()) == Some(true);
    let label = text("description", "description")
        .or_else(|| text("command", "command").map(|command| first_line(&command)))
        .or_else(|| text("task_id", "taskId"))
        .unwrap_or_else(|| "command".into());
    let label = safe_metadata(&label);
    let exit = field("exit_code", "exitCode")
        .and_then(|v| v.as_i64())
        .map(|code| format!(" (exit code {code})"))
        .unwrap_or_default();
    let (mark, status) = if flag("stopped_by_user", "stoppedByUser") {
        (c.muted.fg("·"), "was stopped".to_string())
    } else if flag("is_error", "isError") {
        (c.error.fg("✗"), format!("failed{exit}"))
    } else {
        (c.success.fg("✓"), format!("completed{exit}"))
    };
    let duration = field("duration_ms", "durationMs")
        .and_then(|v| v.as_u64())
        .map(|ms| {
            c.dim
                .fg(&format!(" · {}", format_runtime(Duration::from_millis(ms))))
        })
        .unwrap_or_default();
    let mut lines = vec![format!(
        "{mark} {}{duration}",
        c.text.fg(&format!("Background bash \"{label}\" {status}"))
    )];
    let output = plain_output(&text("text", "text").unwrap_or_default());
    let output: Vec<&str> = output
        .trim()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    let hidden = output.len().saturating_sub(COMPLETION_TAIL_ROWS);
    if hidden > 0 {
        lines.push(c.dim.fg(&format!("  … {hidden} earlier lines")));
    }
    lines.extend(
        output[hidden..]
            .iter()
            .map(|line| format!("  {}", c.dim.fg(line))),
    );
    lines
}

/// Add a finished background shell to the transcript as its status line.
/// Returns false for any other custom message.
pub(super) fn add_completion_message(
    chat: &Arc<ice_tui::Container>,
    custom: &ice_agent::CustomMessage,
) -> bool {
    if custom.data.get("customType").and_then(|v| v.as_str())
        != Some(ice_tools::bash_background::BASH_BACKGROUND_COMPLETED_CUSTOM_TYPE)
    {
        return false;
    }
    let details = custom
        .data
        .get("details")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let text = completion_lines(&details)
        .iter()
        .map(|line| format!("  {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    chat.add_child(Arc::new(ice_tui::Text::new(text, 1, 0)));
    chat.add_child(Arc::new(ice_tui::Spacer::new(1)));
    true
}

/// What a person calls this shell: its description, else its first command
/// line.
pub(super) fn label(shell: &BackgroundShellSnapshot) -> String {
    shell
        .description
        .as_deref()
        .map(safe_metadata)
        .unwrap_or_else(|| first_line(&shell.command))
}

fn first_line(command: &str) -> String {
    safe_metadata(command.trim().split('\n').next().unwrap_or(""))
}

fn safe_metadata(text: &str) -> String {
    plain_output(text)
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect()
}

fn runtime(shell: &BackgroundShellSnapshot, now: i64) -> String {
    let end = shell.ended_at_ms.unwrap_or(now);
    format_runtime(Duration::from_millis(
        end.saturating_sub(shell.started_at_ms).max(0) as u64,
    ))
}

fn status_mark(shell: &BackgroundShellSnapshot) -> String {
    let c = theme().colors;
    match shell.status {
        BackgroundShellStatus::Running => c.accent.fg("●"),
        BackgroundShellStatus::Completed => c.success.fg("✓"),
        BackgroundShellStatus::Failed => c.error.fg("✗"),
        BackgroundShellStatus::Killed => c.muted.fg("·"),
    }
}

fn status_text(shell: &BackgroundShellSnapshot) -> String {
    let c = theme().colors;
    let color = match shell.status {
        BackgroundShellStatus::Running => c.accent,
        BackgroundShellStatus::Completed => c.success,
        BackgroundShellStatus::Failed => c.error,
        BackgroundShellStatus::Killed => c.muted,
    };
    color.fg(shell.status.as_str())
}

/// One terminal row: shell output and commands are untrusted and may hold
/// control characters.
fn one_row(text: &str, width: usize) -> String {
    let mut flat = String::with_capacity(text.len());
    let mut in_break = false;
    for ch in text.chars() {
        if matches!(ch, '\r' | '\n' | '\t') {
            if !in_break {
                flat.push(' ');
            }
            in_break = true;
        } else {
            flat.push(ch);
            in_break = false;
        }
    }
    truncate_to_width(&flat, width, "…")
}

/// Strip terminal control sequences from command output before it is drawn
/// inside the panel. A carriage return redraws the line (progress bars), so
/// only each line's final state is kept.
pub(super) fn plain_output(text: &str) -> String {
    let mut stripped = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\x1b' {
            stripped.push(ch);
            continue;
        }
        match chars.next() {
            // CSI: parameters and intermediates, then one final byte.
            Some('[') => {
                for next in chars.by_ref() {
                    if ('@'..='~').contains(&next) {
                        break;
                    }
                }
            }
            // OSC: until BEL or ST (ESC \).
            Some(']') => {
                while let Some(next) = chars.next() {
                    if next == '\x07' {
                        break;
                    }
                    if next == '\x1b' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    stripped
        .split('\n')
        // A CRLF line ending is not a redraw.
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .map(|line| line.rsplit('\r').next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n")
        .chars()
        .filter(|ch| *ch == '\n' || *ch == '\t' || !ch.is_control())
        .collect()
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
#[path = "shells_panel_tests.rs"]
mod tests;
