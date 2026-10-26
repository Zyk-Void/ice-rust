//! What Cognee did for each prompt, shown on that prompt's message in the
//! transcript: a memory logo `◉` in the message's bottom-right corner, coloured
//! by how the turn's memory work went, and on a click, a panel on the right
//! side of the message box with what was recalled, saved, traced and improved. Fed by the live, already-redacted observation stream, so it
//! shows exactly what `observations.jsonl` records.

use super::*;
use crate::cognee::observer::{subscribe, Observation, Subscription};
use ice_tui::utils::{truncate_to_width, wrap_text_with_ansi};
use ice_tui::MessageAnnotation;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// Link scheme of a turn's badge and detail rows; the rest is the turn id.
const LINK_PREFIX: &str = "ice://cognee-memory/";
/// Turns kept for the transcript; older prompts lose their badge.
const MAX_TURNS: usize = 64;
/// Leading characters of the recall query used to find its prompt message.
const MATCH_CHARS: usize = 60;
/// The memory logo leading the badge.
const LOGO: &str = "◉";
/// The open panel takes the right three fifths of the message box, but never
/// less than this many columns (all of a narrower box).
const PANEL_MIN_WIDTH: usize = 48;
/// Improve attempts kept per turn (idle and dream passes can repeat).
const MAX_IMPROVES: usize = 10;
/// Recalls kept per turn (compaction checkpoint recalls can repeat).
const MAX_RECALLS: usize = 10;
/// Tool traces kept per turn; older ones are only counted.
const MAX_TRACES: usize = 200;
/// Detail rows start under the step names.
const INDENT: &str = "              ";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Status {
    Pending,
    Done,
    Failed,
    Skipped,
}

#[derive(Clone, Debug)]
struct Recall {
    id: String,
    checkpoint: bool,
    status: Status,
    query: String,
    hits: u64,
    memories: Vec<String>,
    note: String,
    latency_ms: Option<u64>,
}

#[derive(Clone, Debug)]
struct Item {
    id: String,
    status: Status,
    /// `prompt`, `answer` or `note` for saved entries; the tool for traces.
    kind: String,
    text: String,
    error: Option<String>,
}

#[derive(Clone, Debug)]
struct Improve {
    id: String,
    status: Status,
    reason: Option<String>,
    error: Option<String>,
    latency_ms: Option<u64>,
}

#[derive(Default)]
struct Model {
    recalls: Vec<Recall>,
    saved: Vec<Item>,
    traces: Vec<Item>,
    improves: Vec<Improve>,
    /// Traces dropped from the front of `traces` by `MAX_TRACES`.
    traces_dropped: usize,
    expanded: bool,
}

/// One line of plain text: no ANSI, no control characters, no newlines.
fn flat(text: &str) -> String {
    ice_tui::strip_ansi(text)
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// The string value of `"key":` in JSON text. Previews are capped at a few
/// hundred characters, so the JSON is often cut off; a value that runs into
/// the cut is returned as far as it goes.
fn json_field(raw: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\":");
    let rest = raw[raw.find(&needle)? + needle.len()..].trim_start();
    let mut chars = rest.strip_prefix('"')?.chars();
    let mut out = String::new();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => match chars.next() {
                Some('n') => out.push('\n'),
                Some('t' | 'r') => out.push(' '),
                Some('u') => {
                    let hex: String = chars.by_ref().take(4).collect();
                    if let Some(c) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                        out.push(c);
                    }
                }
                Some(other) => out.push(other),
                None => break,
            },
            c => out.push(c),
        }
    }
    (!out.is_empty()).then_some(out)
}

fn home_relative(path: &str) -> String {
    match std::env::var("HOME") {
        Ok(home) => home_relative_to(path, &home),
        Err(_) => path.to_string(),
    }
}

/// `~` for `home` itself or a path under it; a sibling such as `/home/al2`
/// for home `/home/al` is left alone.
fn home_relative_to(path: &str, home: &str) -> String {
    let home = home.trim_end_matches('/');
    match path.strip_prefix(home) {
        Some(rest) if !home.is_empty() && (rest.is_empty() || rest.starts_with('/')) => {
            format!("~{rest}")
        }
        _ => path.to_string(),
    }
}

/// `(tool, argument)` of a captured tool trace.
fn trace_summary(raw: &str) -> (String, String) {
    let tool = json_field(raw, "origin_function").unwrap_or_else(|| "tool".into());
    let params = raw
        .find("\"method_params\"")
        .map_or(raw, |index| &raw[index..]);
    let argument = [
        "command",
        "path",
        "file_path",
        "pattern",
        "query",
        "url",
        "name",
    ]
    .iter()
    .find_map(|key| json_field(params, key))
    .map(|value| home_relative(&flat(&value)))
    .unwrap_or_default();
    (flat(&tool), argument)
}

/// `(kind, text)` of a saved entry.
fn saved_summary(operation: &str, meta_kind: Option<&str>, raw: &str) -> (String, String) {
    if operation == "remember" {
        return ("note".into(), raw.to_string());
    }
    if let Some(answer) = json_field(raw, "answer").filter(|text| !text.trim().is_empty()) {
        return ("answer".into(), answer);
    }
    if let Some(question) = json_field(raw, "question").filter(|text| !text.trim().is_empty()) {
        return ("prompt".into(), question);
    }
    (meta_kind.unwrap_or("entry").to_string(), raw.to_string())
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

fn seconds(latency_ms: Option<u64>) -> String {
    latency_ms
        .map(|ms| format!(" · {:.1}s", ms as f64 / 1000.0))
        .unwrap_or_default()
}

/// One status for a group: pending wins, then failure.
fn overall(statuses: impl Iterator<Item = Status>) -> Status {
    let mut result = Status::Done;
    for status in statuses {
        match status {
            Status::Pending => return Status::Pending,
            Status::Failed => result = Status::Failed,
            _ => {}
        }
    }
    result
}

impl Model {
    /// Whether a request with this id started in this turn.
    fn has_request(&self, id: &str) -> bool {
        self.recalls.iter().any(|recall| recall.id == id)
            || self.saved.iter().any(|item| item.id == id)
            || self.traces.iter().any(|item| item.id == id)
            || self.improves.iter().any(|improve| improve.id == id)
    }

    fn is_empty(&self) -> bool {
        self.recalls.is_empty()
            && self.saved.is_empty()
            && self.traces.is_empty()
            && self.improves.is_empty()
    }

    fn upsert_item(list: &mut Vec<Item>, item: Item) {
        match list.iter_mut().find(|existing| existing.id == item.id) {
            Some(existing) => *existing = item,
            None => list.push(item),
        }
    }

    fn finish_item(list: &mut [Item], id: &str, status: Status, error: Option<String>) -> bool {
        match list.iter_mut().find(|item| item.id == id) {
            Some(item) => {
                item.status = status;
                item.error = error;
                true
            }
            None => false,
        }
    }

    /// Returns true when the panel changed.
    fn apply(&mut self, obs: &Observation) -> bool {
        let id = obs.request_id.clone().unwrap_or_else(|| obs.id.clone());
        let meta = |key: &str| obs.meta.as_ref().and_then(|meta| meta.get(key));
        let error = obs.error.as_deref().map(flat);
        // The full redacted text when the feed has it; the log's capped preview
        // otherwise.
        let preview = obs
            .detail
            .as_deref()
            .or(obs.preview.as_deref())
            .unwrap_or("");
        let phase = obs.phase.as_str();
        match obs.operation.as_str() {
            "recall" => self.apply_recall(obs, id, phase, preview, error),
            operation @ ("remember_entry" | "remember" | "trace") => {
                let list = if operation == "trace" {
                    &mut self.traces
                } else {
                    &mut self.saved
                };
                match phase {
                    "started" | "queued" => {
                        let (kind, text) = if operation == "trace" {
                            trace_summary(preview)
                        } else {
                            saved_summary(
                                operation,
                                meta("kind").and_then(|value| value.as_str()),
                                preview,
                            )
                        };
                        Self::upsert_item(
                            list,
                            Item {
                                id,
                                status: Status::Pending,
                                kind,
                                text,
                                error: None,
                            },
                        );
                        // A long tool loop under one prompt traces every call.
                        if operation == "trace" && self.traces.len() > MAX_TRACES {
                            self.traces.remove(0);
                            self.traces_dropped += 1;
                        }
                        true
                    }
                    "succeeded" => Self::finish_item(list, &id, Status::Done, None),
                    "failed" => Self::finish_item(list, &id, Status::Failed, error),
                    _ => false,
                }
            }
            "improve" => {
                match phase {
                    "started" => {
                        self.improves.push(Improve {
                            id,
                            status: Status::Pending,
                            reason: meta("reason")
                                .and_then(|value| value.as_str())
                                .map(String::from),
                            error: None,
                            latency_ms: None,
                        });
                        if self.improves.len() > MAX_IMPROVES {
                            self.improves.remove(0);
                        }
                    }
                    "succeeded" | "failed" => {
                        let Some(improve) = self.improves.iter_mut().find(|i| i.id == id) else {
                            return false;
                        };
                        improve.latency_ms = obs.latency_ms;
                        if phase == "failed" {
                            improve.status = Status::Failed;
                            improve.error = error;
                        } else {
                            improve.status = Status::Done;
                        }
                    }
                    _ => return false,
                }
                true
            }
            _ => false,
        }
    }

    fn apply_recall(
        &mut self,
        obs: &Observation,
        id: String,
        phase: &str,
        preview: &str,
        error: Option<String>,
    ) -> bool {
        let meta = |key: &str| obs.meta.as_ref().and_then(|meta| meta.get(key));
        let flag = |key: &str| meta(key).and_then(|value| value.as_bool()) == Some(true);
        if phase == "started" {
            let checkpoint = flag("checkpoint");
            self.recalls.push(Recall {
                id,
                checkpoint,
                status: Status::Pending,
                query: flat(preview),
                hits: 0,
                memories: Vec::new(),
                note: String::new(),
                latency_ms: None,
            });
            // Compaction checkpoint recalls join the current turn, so repeated
            // compactions would otherwise grow it without bound.
            if self.recalls.len() > MAX_RECALLS {
                self.recalls.remove(0);
            }
            return true;
        }
        let Some(index) = self.recalls.iter().position(|recall| recall.id == id) else {
            return false;
        };
        if phase == "succeeded" && flag("disabled") {
            self.recalls.remove(index);
            return true;
        }
        let recall = &mut self.recalls[index];
        recall.latency_ms = obs.latency_ms;
        match phase {
            "succeeded" if flag("skipped") || flag("duplicate") => {
                recall.status = Status::Skipped;
                recall.note = match meta("reason").and_then(|value| value.as_str()) {
                    Some("short_prompt") => "skipped · prompt too short".into(),
                    _ if flag("duplicate") => "skipped · same prompt as last time".into(),
                    _ => "skipped".into(),
                };
            }
            "succeeded" => {
                recall.status = Status::Done;
                let hits = meta("hits").and_then(|value| value.as_u64()).unwrap_or(0);
                recall.hits = hits;
                recall.memories = preview
                    .lines()
                    .map(flat)
                    .filter(|line| !line.is_empty())
                    .collect();
                recall.note = if hits == 0 {
                    "nothing relevant found".into()
                } else {
                    plural(hits as usize, "memory", "memories")
                };
            }
            "failed" => {
                recall.status = Status::Failed;
                recall.note = format!("failed · {}", error.unwrap_or_default());
            }
            _ => return false,
        }
        true
    }

    /// The whole turn as one state, shown as the logo's colour: working while
    /// any step runs, failed when a step's latest attempt failed, done when
    /// something succeeded, skipped when every step was skipped.
    fn status(&self) -> Status {
        let all: Vec<Status> = self
            .recalls
            .iter()
            .map(|recall| recall.status)
            .chain(
                self.saved
                    .iter()
                    .chain(&self.traces)
                    .map(|item| item.status),
            )
            .chain(self.improves.last().map(|improve| improve.status))
            .collect();
        [Status::Pending, Status::Failed, Status::Done]
            .into_iter()
            .find(|status| all.contains(status))
            .unwrap_or(Status::Skipped)
    }
}

fn plain_glyph(status: Status) -> &'static str {
    match status {
        Status::Pending => "◌",
        Status::Done => "✓",
        Status::Failed => "✗",
        Status::Skipped => "–",
    }
}

fn glyph(status: Status) -> String {
    let c = current_theme().colors;
    let paint = match status {
        Status::Pending => c.accent,
        Status::Done => c.success,
        Status::Failed => c.error,
        Status::Skipped => c.muted,
    };
    paint.fg(plain_glyph(status))
}

/// Non-empty lines of `text`, each flattened to plain single-line text.
fn text_lines(text: &str) -> Vec<String> {
    ice_tui::strip_ansi(text)
        .lines()
        .map(flat)
        .filter(|line| !line.is_empty())
        .collect()
}

/// Paragraphs wrapped to `width` (nothing is cut off): the first row starts
/// with `first`, every later row with `rest`.
fn wrapped(
    first: &str,
    rest: &str,
    paragraphs: &[String],
    paint: impl Fn(&str) -> String,
    width: usize,
) -> Vec<String> {
    // Never wider than the panel: a forced minimum would be cut off by
    // `linked`, losing the text wrapping exists to keep.
    let room = width.saturating_sub(ice_tui::visible_width(first)).max(1);
    let mut rows = Vec::new();
    for paragraph in paragraphs {
        for piece in wrap_text_with_ansi(paragraph, room) {
            let prefix = if rows.is_empty() { first } else { rest };
            rows.push(format!("{prefix}{}", paint(&piece)));
        }
    }
    if rows.is_empty() {
        rows.push(first.to_string());
    }
    rows
}

/// `  ✓ recall     <detail>`, continuation rows aligned under the detail.
fn step_rows(status: Status, name: &str, paragraphs: &[String], width: usize) -> Vec<String> {
    let c = current_theme().colors;
    let first = format!(
        "  {} {} ",
        glyph(status),
        c.accent.fg(&format!("{name:<10}"))
    );
    let rest = format!("{INDENT} ");
    let paint = |text: &str| {
        if status == Status::Failed {
            c.error.fg(text)
        } else {
            c.dim.fg(text)
        }
    };
    wrapped(&first, &rest, paragraphs, paint, width)
}

/// `│ <text>` rows under a step.
fn detail_rows(text: &str, width: usize) -> Vec<String> {
    let c = current_theme().colors;
    let prefix = format!("{INDENT}{} ", c.muted.fg("│"));
    wrapped(
        &prefix,
        &prefix,
        &[text.to_string()],
        |t| c.dim.fg(t),
        width,
    )
}

impl Model {
    /// The memory logo, coloured by [`Model::status`]; `None` while Cognee
    /// did nothing. A hovered badge is drawn as a highlighter mark instead.
    fn badge_text(&self, hovered: bool) -> Option<String> {
        if self.is_empty() {
            return None;
        }
        let c = current_theme().colors;
        if hovered {
            // Marker-pen look, as on hovered tool hints: accent fill with true
            // black text (palette black is often grey).
            return Some(
                c.accent
                    .bg(&ice_tui::theme::Color::Rgb(0, 0, 0).fg(&format!(" {LOGO} "))),
            );
        }
        let paint = match self.status() {
            Status::Pending => c.accent,
            Status::Done => c.success,
            Status::Failed => c.error,
            Status::Skipped => c.muted,
        };
        Some(format!(" {} ", paint.fg(LOGO)))
    }

    /// The dropdown rows wrapped to `width`, in full; empty while collapsed.
    fn detail_lines(&self, width: usize) -> Vec<String> {
        let mut lines = Vec::new();
        if !self.expanded {
            return lines;
        }

        for recall in &self.recalls {
            let name = if recall.checkpoint {
                "recall ⟲"
            } else {
                "recall"
            };
            let detail = match recall.status {
                Status::Pending => format!("“{}”", recall.query),
                _ => format!(
                    "“{}” → {}{}",
                    recall.query,
                    recall.note,
                    seconds(recall.latency_ms)
                ),
            };
            lines.extend(step_rows(recall.status, name, &[detail], width));
            for memory in &recall.memories {
                lines.extend(detail_rows(memory, width));
            }
        }

        for item in &self.saved {
            let paragraphs = match &item.error {
                Some(error) => vec![format!("{} · failed · {error}", item.kind)],
                None => {
                    let mut paragraphs = text_lines(&item.text);
                    let head = paragraphs.first().cloned().unwrap_or_default();
                    match paragraphs.first_mut() {
                        Some(first) => *first = format!("{} · {head}", item.kind),
                        None => paragraphs.push(item.kind.clone()),
                    }
                    paragraphs
                }
            };
            lines.extend(step_rows(item.status, "saved", &paragraphs, width));
        }

        if !self.traces.is_empty() {
            let mut counts: Vec<(String, usize)> = Vec::new();
            for trace in &self.traces {
                match counts.iter_mut().find(|(tool, _)| *tool == trace.kind) {
                    Some((_, count)) => *count += 1,
                    None => counts.push((trace.kind.clone(), 1)),
                }
            }
            let mut tally = counts
                .iter()
                .map(|(tool, count)| format!("{tool} ×{count}"))
                .collect::<Vec<_>>()
                .join(" · ");
            if self.traces_dropped > 0 {
                tally.push_str(&format!(" · +{} earlier", self.traces_dropped));
            }
            let status = overall(self.traces.iter().map(|item| item.status));
            lines.extend(step_rows(status, "traces", &[tally], width));
            let tool_width = self.traces.iter().map(|t| t.kind.len()).max().unwrap_or(0);
            for trace in &self.traces {
                let mark = if trace.status == Status::Failed {
                    " ✗"
                } else {
                    ""
                };
                lines.extend(detail_rows(
                    &format!("{:<tool_width$}  {}{mark}", trace.kind, trace.text),
                    width,
                ));
            }
        }

        for improve in &self.improves {
            let reason = improve.reason.as_deref().unwrap_or("turn");
            let detail = match (improve.status, &improve.error) {
                (Status::Pending, _) => format!("{reason} · running"),
                (Status::Failed, Some(error)) => format!("{reason} · failed · {error}"),
                _ => format!("{reason}{}", seconds(improve.latency_ms)),
            };
            lines.extend(step_rows(improve.status, "improve", &[detail], width));
        }
        lines
    }
}

/// One prompt's memory activity, attached to that prompt's message.
struct Turn {
    id: usize,
    /// The prompt as Cognee saw it (redacted, capped), to find its message.
    query: String,
    model: Mutex<Model>,
    /// Pointer is over the badge.
    hovered: AtomicBool,
}

fn linked(id: usize, row: &str, width: usize, pad: bool) -> String {
    let row = truncate_to_width(row, width, "…");
    let fill = if pad {
        " ".repeat(width.saturating_sub(ice_tui::visible_width(&row)))
    } else {
        String::new()
    };
    format!("\x1b]8;;{LINK_PREFIX}{id}\x07{row}{fill}\x1b]8;;\x07")
}

impl MessageAnnotation for Turn {
    fn badge(&self, width: usize) -> Option<String> {
        let hovered = self.hovered.load(Ordering::Acquire);
        let badge = self.model.lock().ok()?.badge_text(hovered)?;
        Some(linked(self.id, &badge, width, false))
    }

    fn details(&self, width: usize) -> Vec<String> {
        let Ok(model) = self.model.lock() else {
            return Vec::new();
        };
        // A panel on the right side, under the badge. Rows are padded to the
        // panel width so a click anywhere on the panel closes it.
        let panel = if width <= PANEL_MIN_WIDTH {
            width
        } else {
            (width * 3 / 5).max(PANEL_MIN_WIDTH)
        };
        let indent = " ".repeat(width - panel);
        model
            .detail_lines(panel)
            .iter()
            .map(|row| format!("{indent}{}", linked(self.id, row, panel, true)))
            .collect()
    }
}

/// Turn whose badge or detail row covers `column` of a painted screen row.
fn turn_link_at(row: &str, column: usize) -> Option<usize> {
    ice_tui::get_osc8_link_at_column(row, column)?
        .strip_prefix(LINK_PREFIX)?
        .parse()
        .ok()
}

/// Whether a prompt message is the one a recall query was made from.
fn prompt_matches(text: &str, query: &str) -> bool {
    let prefix: String = query.chars().take(MATCH_CHARS).collect();
    !prefix.is_empty() && flat(text).starts_with(&prefix)
}

/// Turns and their routing, apart from the global feed so tests can drive it.
#[derive(Default)]
struct Router {
    turns: Mutex<Vec<Arc<Turn>>>,
    /// Turns dropped by `MAX_TURNS` whose badge is still on a message; the
    /// next `attach` takes it off, since a click could no longer find them.
    evicted: Mutex<Vec<Arc<Turn>>>,
    next_id: AtomicUsize,
    dirty: AtomicBool,
}

impl Router {
    fn ingest(&self, obs: &Observation) {
        let Ok(mut turns) = self.turns.lock() else {
            return;
        };
        let opens_turn = obs.operation == "recall"
            && obs.phase == "started"
            && !obs
                .meta
                .as_ref()
                .and_then(|meta| meta.get("checkpoint"))
                .is_some_and(|value| value.as_bool() == Some(true));
        // A prompt's recall opens its turn; everything else (saves, traces,
        // improves, a compaction checkpoint recall) belongs to the latest one.
        if opens_turn || turns.is_empty() {
            turns.push(Arc::new(Turn {
                id: self.next_id.fetch_add(1, Ordering::Relaxed) + 1,
                query: if opens_turn {
                    flat(
                        obs.detail
                            .as_deref()
                            .or(obs.preview.as_deref())
                            .unwrap_or(""),
                    )
                } else {
                    String::new()
                },
                model: Mutex::new(Model::default()),
                hovered: AtomicBool::new(false),
            }));
            if turns.len() > MAX_TURNS {
                let old = turns.remove(0);
                if let Ok(mut evicted) = self.evicted.lock() {
                    evicted.push(old);
                }
            }
        }
        // A completion belongs to the turn that saw its start, even when a
        // newer prompt has opened a turn since (a slow save or improve).
        let owner = (!matches!(obs.phase.as_str(), "started" | "queued"))
            .then_some(obs.request_id.as_deref())
            .flatten()
            .and_then(|id| {
                turns
                    .iter()
                    .rev()
                    .find(|turn| turn.model.lock().is_ok_and(|model| model.has_request(id)))
            });
        let turn = owner.unwrap_or_else(|| turns.last().expect("a turn was just ensured"));
        if turn.model.lock().is_ok_and(|mut model| model.apply(obs)) {
            self.dirty.store(true, Ordering::SeqCst);
        }
    }

    fn sync(&self, chat: &Container) -> bool {
        let attached = self.attach(chat);
        self.dirty.swap(false, Ordering::SeqCst) || attached
    }

    fn attach(&self, chat: &Container) -> bool {
        let children = chat.get_children();
        let users: Vec<&UserMessageComponent> = children
            .iter()
            .filter_map(|child| child.as_any().downcast_ref::<UserMessageComponent>())
            .collect();
        let mut changed = self.detach_evicted(&users);
        let turns = match self.turns.lock() {
            Ok(turns) if !turns.is_empty() => turns.clone(),
            _ => return changed,
        };
        let held: HashSet<*const ()> = users
            .iter()
            .filter_map(|user| user.annotation())
            .map(|annotation| Arc::as_ptr(&annotation) as *const ())
            .collect();
        let newest = turns.len() - 1;
        // Newest first, so a repeated prompt's latest turn takes the latest
        // matching message.
        for (index, turn) in turns.iter().enumerate().rev() {
            if held.contains(&(Arc::as_ptr(turn) as *const ())) {
                continue;
            }
            let free = |user: &&&UserMessageComponent| user.annotation().is_none();
            let found = users
                .iter()
                .rev()
                .filter(free)
                .find(|user| prompt_matches(user.text(), &turn.query))
                // The prompt as sent can differ from the redacted query; the
                // newest turn then belongs to the newest message.
                .or_else(|| {
                    (index == newest)
                        .then(|| users.last().filter(free))
                        .flatten()
                });
            if let Some(user) = found {
                let annotation: Arc<dyn MessageAnnotation> = turn.clone();
                user.set_annotation(Some(annotation));
                changed = true;
            }
        }
        changed
    }

    fn detach_evicted(&self, users: &[&UserMessageComponent]) -> bool {
        let evicted = match self.evicted.lock() {
            Ok(mut evicted) if !evicted.is_empty() => std::mem::take(&mut *evicted),
            _ => return false,
        };
        let gone: HashSet<*const ()> = evicted
            .iter()
            .map(|turn| Arc::as_ptr(turn) as *const ())
            .collect();
        let mut changed = false;
        for user in users {
            if user
                .annotation()
                .is_some_and(|annotation| gone.contains(&(Arc::as_ptr(&annotation) as *const ())))
            {
                user.set_annotation(None);
                changed = true;
            }
        }
        changed
    }

    fn hover(&self, row: Option<&str>, column: usize) -> bool {
        let id = row.and_then(|row| turn_link_at(row, column));
        let Ok(turns) = self.turns.lock() else {
            return false;
        };
        let mut changed = false;
        for turn in turns.iter() {
            let hovered = id == Some(turn.id);
            changed |= turn.hovered.swap(hovered, Ordering::AcqRel) != hovered;
        }
        changed
    }

    fn click(&self, row: &str, column: usize) -> bool {
        let Some(id) = turn_link_at(row, column) else {
            return false;
        };
        let turn = match self.turns.lock() {
            Ok(turns) => turns.iter().find(|turn| turn.id == id).cloned(),
            Err(_) => None,
        };
        let Some(turn) = turn else {
            return false;
        };
        let Ok(mut model) = turn.model.lock() else {
            return false;
        };
        model.expanded = !model.expanded;
        true
    }
}

pub(super) struct CogneeActivity {
    router: Arc<Router>,
    _subscription: Subscription,
}

impl CogneeActivity {
    pub(super) fn new() -> Self {
        let router = Arc::new(Router::default());
        let feed = router.clone();
        let subscription = subscribe(move |obs| feed.ingest(obs));
        Self {
            router,
            _subscription: subscription,
        }
    }

    /// Tick hook: attach new turns to their prompt messages (again after a
    /// transcript rebuild). True when the transcript needs a repaint.
    pub(super) fn sync(&self, chat: &Container) -> bool {
        self.router.sync(chat)
    }

    /// Open or close a turn's dropdown when a click lands on its badge or
    /// detail rows. Returns true when it toggled.
    pub(super) fn click(&self, row: &str, column: usize) -> bool {
        self.router.click(row, column)
    }

    /// Highlight the badge under the pointer (`row` is the painted screen
    /// row, `None` off-screen). Returns true when a badge changed.
    pub(super) fn hover(&self, row: Option<&str>, column: usize) -> bool {
        self.router.hover(row, column)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Map, Value};

    fn obs(op: &str, phase: &str, id: &str, preview: Option<&str>, meta: Value) -> Observation {
        Observation {
            id: format!("o-{id}-{phase}"),
            operation: op.into(),
            phase: phase.into(),
            request_id: Some(id.into()),
            preview: preview.map(String::from),
            meta: meta.as_object().cloned().or_else(|| Some(Map::new())),
            ..Observation::default()
        }
    }

    fn plain(lines: &[String]) -> Vec<String> {
        lines
            .iter()
            .map(|line| ice_tui::strip_ansi(line).trim_end().to_string())
            .collect()
    }

    /// Badge then dropdown rows, as plain text.
    fn rows(model: &Model) -> Vec<String> {
        let mut rows: Vec<String> = model.badge_text(false).into_iter().collect();
        rows.extend(model.detail_lines(200));
        plain(&rows)
    }

    /// Open a message's panel, read it, and close it again.
    fn opened(router: &Router, message: &UserMessageComponent) -> String {
        let annotation = message.annotation().expect("attached");
        let badge = annotation.badge(200).unwrap();
        assert!(router.click(&badge, 1));
        let text = plain(&annotation.details(200)).join("\n");
        router.click(&badge, 1);
        text
    }

    #[test]
    fn recall_shows_memories() {
        let mut model = Model::default();
        model.apply(&obs(
            "recall",
            "started",
            "r1",
            Some("fix the\nbuild"),
            json!({}),
        ));
        assert_eq!(model.status(), Status::Pending);
        assert_eq!(rows(&model)[0].trim(), "◉", "collapsed it is just the logo");
        model.apply(&obs(
            "recall",
            "succeeded",
            "r1",
            Some("use cargo\ncheck\n\nrun fmt"),
            json!({"hits": 2}),
        ));
        assert_eq!(model.status(), Status::Done);
        model.expanded = true;
        let lines = rows(&model);
        assert!(lines[1].contains("recall"), "{lines:?}");
        assert!(
            lines[1].contains("“fix the build” → 2 memories"),
            "{lines:?}"
        );
        assert!(lines[2].ends_with("│ use cargo"), "{lines:?}");
        assert!(lines[4].ends_with("│ run fmt"), "{lines:?}");
    }

    #[test]
    fn skipped_recall_says_why_and_disabled_recall_leaves_nothing() {
        let mut model = Model::default();
        model.apply(&obs("recall", "started", "r1", Some("hey"), json!({})));
        model.apply(&obs(
            "recall",
            "succeeded",
            "r1",
            None,
            json!({"skipped": true, "reason": "short_prompt"}),
        ));
        assert_eq!(model.status(), Status::Skipped);
        model.expanded = true;
        assert!(rows(&model)[1].contains("skipped · prompt too short"));

        let mut off = Model::default();
        off.apply(&obs("recall", "started", "r1", Some("hey"), json!({})));
        off.apply(&obs(
            "recall",
            "succeeded",
            "r1",
            None,
            json!({"disabled": true}),
        ));
        assert!(off.is_empty());
        assert!(!off.apply(&obs("session", "succeeded", "s", None, json!({}))));
    }

    /// Traces arrive as JSON cut at the preview cap; they still show the tool
    /// and its argument instead of the raw object.
    #[test]
    fn traces_are_readable_even_when_the_preview_is_cut() {
        let mut model = Model::default();
        let home = std::env::var("HOME").unwrap_or_default();
        let read = format!(
            r#"{{"type":"trace","origin_function":"read","status":"success","method_params":{{"path":"{home}/.icers/agent/skills/x/SKILL.md"}},"return_value":"lots of te"#
        );
        let bash = r#"{"type":"trace","origin_function":"bash","status":"success","method_params":{"command":"pwd; git status --short --bra"#;
        model.apply(&obs("trace", "started", "a", Some(&read), json!({})));
        model.apply(&obs("trace", "started", "b", Some(&read), json!({})));
        model.apply(&obs("trace", "started", "c", Some(bash), json!({})));
        model.apply(&obs("trace", "succeeded", "a", None, json!({})));
        assert_eq!(model.status(), Status::Pending);
        for id in ["b", "c"] {
            model.apply(&obs("trace", "succeeded", id, None, json!({})));
        }
        assert_eq!(model.status(), Status::Done);
        model.expanded = true;
        let lines = rows(&model);
        assert!(lines[1].contains("traces") && lines[1].contains("read ×2 · bash ×1"));
        let expected_path = if home.is_empty() {
            "/.icers/agent/skills/x/SKILL.md".to_string()
        } else {
            "~/.icers/agent/skills/x/SKILL.md".to_string()
        };
        assert!(
            lines[2].ends_with(&format!("│ read  {expected_path}")),
            "{lines:?}"
        );
        assert!(
            lines[4].ends_with("│ bash  pwd; git status --short --bra"),
            "{lines:?}"
        );
        assert!(!lines.iter().any(|line| line.contains('{')), "{lines:?}");
    }

    #[test]
    fn saved_entries_show_what_they_are() {
        let mut model = Model::default();
        let qa = r#"{"type":"qa","question":"fix it","answer":"done\nok","context":""}"#;
        let prompt = r#"{"type":"qa","question":"fix it","answer":"","context":""}"#;
        let cut = r#"{"type":"qa","question":"cut off here"#;
        model.apply(&obs("remember_entry", "started", "a", Some(qa), json!({})));
        model.apply(&obs(
            "remember_entry",
            "started",
            "b",
            Some(prompt),
            json!({}),
        ));
        model.apply(&obs("remember_entry", "started", "c", Some(cut), json!({})));
        let mut failed = obs("remember_entry", "failed", "c", None, json!({}));
        failed.error = Some("timeout".into());
        model.apply(&failed);
        model.apply(&obs("remember_entry", "succeeded", "a", None, json!({})));
        model.apply(&obs("remember_entry", "succeeded", "b", None, json!({})));
        assert_eq!(model.status(), Status::Failed);
        model.expanded = true;
        let lines = rows(&model);
        assert!(lines[1].contains("✓ saved") && lines[1].ends_with("answer · done"));
        assert_eq!(lines[2], format!("{INDENT} ok"), "answer lines are kept");
        assert!(lines[3].contains("prompt · fix it"), "{lines:?}");
        assert!(lines[4].contains("✗ saved") && lines[4].contains("prompt · failed · timeout"));
    }

    /// The dropdown shows everything: long text wraps instead of being cut
    /// with "…", every memory and trace is listed, and the full text (not the
    /// log's 900-character preview) is used when the feed carries it.
    #[test]
    fn the_dropdown_shows_everything_wrapped() {
        let mut model = Model::default();
        let long_answer = format!("start {} end", "word ".repeat(400));
        let mut saved = obs(
            "remember_entry",
            "started",
            "a",
            Some("{\"answer\":\"cut"),
            json!({}),
        );
        saved.detail = Some(json!({"type": "qa", "answer": long_answer}).to_string());
        model.apply(&saved);
        model.apply(&obs("recall", "started", "r", Some("q"), json!({})));
        let memories = (1..=6)
            .map(|i| format!("memory number {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        model.apply(&obs(
            "recall",
            "succeeded",
            "r",
            Some(&memories),
            json!({"hits": 6}),
        ));
        for i in 0..9 {
            model.apply(&obs(
                "trace",
                "started",
                &format!("t{i}"),
                Some(&format!(
                    r#"{{"origin_function":"read","method_params":{{"path":"/f{i}"}}}}"#
                )),
                json!({}),
            ));
        }
        model.expanded = true;
        let width = 60;
        let lines = plain(&model.detail_lines(width));
        assert!(
            lines
                .iter()
                .all(|line| ice_tui::visible_width(line) <= width),
            "{lines:?}"
        );
        let text = lines.join("\n");
        assert!(!text.contains('…') && !text.contains("more"), "{text}");
        assert!(
            text.contains("answer · start") && text.contains("word end"),
            "{text}"
        );
        assert!(lines.len() > 30, "the long answer wraps over many rows");
        for i in 1..=6 {
            assert!(text.contains(&format!("│ memory number {i}")), "{text}");
        }
        for i in 0..9 {
            assert!(text.contains(&format!("read  /f{i}")), "{text}");
        }
    }

    #[test]
    fn improve_attempts_are_listed_with_reason_and_outcome() {
        let mut model = Model::default();
        model.apply(&obs(
            "improve",
            "started",
            "i1",
            None,
            json!({"reason": "dream"}),
        ));
        let mut failed = obs("improve", "failed", "i1", None, json!({}));
        failed.error = Some("timeout".into());
        model.apply(&failed);
        assert_eq!(model.status(), Status::Failed);
        model.apply(&obs(
            "improve",
            "started",
            "i2",
            None,
            json!({"reason": "shutdown"}),
        ));
        let mut done = obs("improve", "succeeded", "i2", None, json!({}));
        done.latency_ms = Some(470);
        model.apply(&done);
        assert_eq!(
            model.status(),
            Status::Done,
            "a successful retry clears the failure"
        );
        model.expanded = true;
        let lines = rows(&model);
        assert!(lines[1].contains("✗ improve") && lines[1].contains("dream · failed · timeout"));
        assert!(lines[2].contains("✓ improve") && lines[2].ends_with("shutdown · 0.5s"));
    }

    fn user(chat: &Container, text: &str) -> Arc<UserMessageComponent> {
        let message = Arc::new(UserMessageComponent::new(text));
        chat.add_child(message.clone());
        message
    }

    fn badge(message: &UserMessageComponent) -> String {
        message
            .annotation()
            .and_then(|annotation| annotation.badge(200))
            .map(|badge| ice_tui::strip_ansi(&badge))
            .unwrap_or_default()
    }

    /// Each prompt's activity lands on its own message, also when the earlier
    /// prompt's improve finishes after the next prompt was sent.
    #[test]
    fn each_turn_is_attached_to_its_own_prompt_message() {
        let router = Router::default();
        let chat = Container::new();
        let old = user(&chat, "an old prompt from a resumed session");
        let first = user(&chat, "explore the codebase");
        router.ingest(&obs(
            "recall",
            "started",
            "r1",
            Some("explore the codebase"),
            json!({}),
        ));
        router.ingest(&obs(
            "improve",
            "started",
            "i1",
            None,
            json!({"reason": "idle"}),
        ));
        assert!(router.sync(&chat));
        assert_eq!(badge(&first).trim(), "◉");
        assert!(opened(&router, &first).contains("idle · running"));
        assert!(old.annotation().is_none());

        let second = user(&chat, "now fix the build");
        router.ingest(&obs(
            "recall",
            "started",
            "r2",
            Some("now fix the build"),
            json!({}),
        ));
        router.ingest(&obs(
            "recall",
            "succeeded",
            "r2",
            Some("cargo check first"),
            json!({"hits": 1}),
        ));
        router.sync(&chat);
        let second_panel = opened(&router, &second);
        assert!(
            second_panel.contains("“now fix the build” → 1 memory"),
            "{second_panel}"
        );
        let first_panel = opened(&router, &first);
        assert!(!first_panel.contains("now fix the build"), "{first_panel}");
        assert!(!router.sync(&chat), "nothing new to attach or repaint");

        // The first prompt's improve finishes now; it updates that turn.
        router.ingest(&obs("improve", "succeeded", "i1", None, json!({})));
        assert!(router.sync(&chat));
        let first_panel = opened(&router, &first);
        assert!(!first_panel.contains("running"), "{first_panel}");
        assert!(first_panel.contains("idle"), "{first_panel}");
    }

    /// An evicted turn's badge comes off its message instead of staying as a
    /// dead click target.
    #[test]
    fn evicted_turns_lose_their_badge() {
        let router = Router::default();
        let chat = Container::new();
        let first = user(&chat, "prompt 0");
        router.ingest(&obs("recall", "started", "r0", Some("prompt 0"), json!({})));
        router.sync(&chat);
        assert!(first.annotation().is_some());
        for i in 1..=MAX_TURNS {
            let text = format!("prompt {i}");
            user(&chat, &text);
            router.ingest(&obs(
                "recall",
                "started",
                &format!("r{i}"),
                Some(&text),
                json!({}),
            ));
        }
        assert!(router.sync(&chat));
        assert!(first.annotation().is_none());
    }

    #[test]
    fn per_turn_recalls_and_traces_are_bounded() {
        let mut model = Model::default();
        for i in 0..MAX_RECALLS + 3 {
            model.apply(&obs(
                "recall",
                "started",
                &format!("c{i}"),
                Some("checkpoint"),
                json!({"checkpoint": true}),
            ));
        }
        assert_eq!(model.recalls.len(), MAX_RECALLS);
        for i in 0..MAX_TRACES + 5 {
            model.apply(&obs(
                "trace",
                "started",
                &format!("t{i}"),
                Some(r#"{"origin_function":"bash","method_params":{"command":"ls"}}"#),
                json!({}),
            ));
        }
        assert_eq!(model.traces.len(), MAX_TRACES);
        model.expanded = true;
        let text = rows(&model).join("\n");
        assert!(
            text.contains(&format!("bash ×{MAX_TRACES} · +5 earlier")),
            "{text}"
        );
    }

    #[test]
    fn home_prefix_is_matched_by_path_component() {
        assert_eq!(
            home_relative_to("/home/al/src/a.rs", "/home/al"),
            "~/src/a.rs"
        );
        assert_eq!(home_relative_to("/home/al", "/home/al/"), "~");
        assert_eq!(
            home_relative_to("/home/al2/a.rs", "/home/al"),
            "/home/al2/a.rs"
        );
        assert_eq!(home_relative_to("/x", ""), "/x");
    }

    /// A rebuilt transcript has fresh message components; turns find their
    /// prompts again by text.
    #[test]
    fn turns_reattach_after_the_transcript_is_rebuilt() {
        let router = Router::default();
        let chat = Container::new();
        user(&chat, "first prompt");
        router.ingest(&obs(
            "recall",
            "started",
            "r1",
            Some("first prompt"),
            json!({}),
        ));
        user(&chat, "second prompt");
        router.ingest(&obs(
            "recall",
            "started",
            "r2",
            Some("second prompt"),
            json!({}),
        ));
        router.sync(&chat);

        chat.clear();
        let first = user(&chat, "first prompt");
        let second = user(&chat, "second prompt");
        assert!(router.sync(&chat));
        assert!(opened(&router, &first).contains("“first prompt”"));
        assert!(opened(&router, &second).contains("“second prompt”"));
    }

    #[test]
    fn a_click_on_the_badge_or_a_detail_row_toggles_the_dropdown() {
        let router = Router::default();
        let chat = Container::new();
        let message = user(&chat, "hey");
        router.sync(&chat);
        assert!(message.annotation().is_none(), "nothing until Cognee acts");
        router.ingest(&obs("recall", "started", "r1", Some("hey"), json!({})));
        router.ingest(&obs(
            "trace",
            "started",
            "t",
            Some(r#"{"origin_function":"bash","method_params":{"command":"ls"}}"#),
            json!({}),
        ));
        router.sync(&chat);
        let annotation = message.annotation().expect("attached");
        let collapsed = annotation.badge(80).unwrap();
        assert!(annotation.details(80).is_empty());
        assert!(!router.click("plain row", 0));
        assert!(router.click(&collapsed, 0));
        let details = annotation.details(80);
        assert!(details.len() >= 2, "{details:?}");
        assert!(
            plain(&details)
                .iter()
                .any(|row| row.ends_with("│ bash  ls")),
            "{details:?}"
        );
        assert!(
            router.click(&details[0], 79),
            "detail rows are padded click targets"
        );
        assert!(annotation.details(80).is_empty());
    }

    #[test]
    fn hovering_the_badge_highlights_it_and_leaving_clears_it() {
        let router = Router::default();
        let chat = Container::new();
        let message = user(&chat, "hey");
        router.ingest(&obs("recall", "started", "r1", Some("hey"), json!({})));
        router.sync(&chat);
        let annotation = message.annotation().expect("attached");
        let normal = annotation.badge(80).unwrap();

        assert!(router.hover(Some(&normal), 1));
        let highlighted = annotation.badge(80).unwrap();
        assert_ne!(highlighted, normal);
        let fill = current_theme().colors.accent.bg("");
        let fill = fill.split("\x1b[0m").next().unwrap_or(&fill);
        assert!(highlighted.contains(fill), "accent fill: {highlighted:?}");
        assert_eq!(
            ice_tui::strip_ansi(&highlighted).trim(),
            ice_tui::strip_ansi(&normal).trim(),
            "same text, only the fill changes"
        );
        assert!(
            !router.hover(Some(&normal), 1),
            "no repaint while still over it"
        );
        assert!(router.hover(Some("elsewhere"), 0));
        assert_eq!(annotation.badge(80).unwrap(), normal);
        assert!(!router.hover(None, 0));
    }

    /// The open panel sits on the right side of the message, under the badge,
    /// and its rows end at the box's right edge.
    #[test]
    fn the_open_panel_sits_on_the_right_side() {
        let router = Router::default();
        let chat = Container::new();
        let message = user(&chat, "hey there friend");
        router.ingest(&obs(
            "recall",
            "started",
            "r1",
            Some("hey there friend"),
            json!({}),
        ));
        router.sync(&chat);
        let annotation = message.annotation().expect("attached");
        assert_eq!(ice_tui::strip_ansi(&annotation.badge(100).unwrap()), " ◉ ");
        router.click(&annotation.badge(100).unwrap(), 1);
        let rows = annotation.details(100);
        assert!(!rows.is_empty());
        for row in &rows {
            let text = ice_tui::strip_ansi(row);
            assert_eq!(ice_tui::visible_width(&text), 100, "{text:?}");
            assert!(
                text.starts_with(&" ".repeat(40)),
                "left side stays empty: {text:?}"
            );
        }
        assert!(router.click(&rows[0], 99), "the panel is clickable");
        // A narrow box gives the panel its whole width.
        router.click(&annotation.badge(40).unwrap(), 1);
        assert!(annotation
            .details(40)
            .iter()
            .all(|row| ice_tui::strip_ansi(row).starts_with("  ")));
    }
}
