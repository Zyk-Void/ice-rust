//! Editor component for multi-line text editing.
//!
//! Provides a text editor with cursor positioning, selection, and keyboard input.

use std::any::Any;
use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::{Arc, Mutex};

use crossterm::event::{KeyCode, KeyModifiers};
use unicode_segmentation::UnicodeSegmentation;

use super::component::{Component, Focusable};
use super::keybindings::Keybindings;
use crate::ansi::CURSOR_MARKER;
use crate::kill_ring::{KillRing, PushOptions};
use crate::theme::Color;
use crate::undo_stack::UndoStack;

/// Editor style configuration.
#[derive(Debug, Clone)]
pub struct EditorStyle {
    pub prompt: String,
    pub placeholder: String,
}

impl Default for EditorStyle {
    fn default() -> Self {
        Self {
            prompt: "> ".to_string(),
            placeholder: String::new(),
        }
    }
}

/// Editor options.
#[derive(Debug, Clone, Default)]
pub struct EditorOptions {
    /// Padding on left and right.
    pub padding_x: usize,
    /// Maximum visible lines for autocomplete.
    pub autocomplete_max_visible: usize,
    /// Placeholder text when empty.
    pub placeholder: Option<String>,
    /// Initial text content.
    pub initial_text: Option<String>,
}

/// Editor state.
#[derive(Debug, Clone)]
struct EditorState {
    /// Lines of text.
    lines: Vec<String>,
    /// Cursor row (line index).
    cursor_row: usize,
    /// Cursor column (character index within line).
    cursor_col: usize,
    /// Selection anchor (row, col) if any.
    selection_anchor: Option<(usize, usize)>,
    /// Scroll offset for multi-line.
    #[allow(dead_code)]
    scroll_offset: usize,
    /// Whether the editor is focused.
    focused: bool,
    /// History of submitted lines.
    history: Vec<String>,
    /// History index for navigation.
    history_index: usize,
}

impl Default for EditorState {
    fn default() -> Self {
        Self {
            lines: vec![String::new()],
            cursor_row: 0,
            cursor_col: 0,
            selection_anchor: None,
            scroll_offset: 0,
            focused: false,
            history: Vec::new(),
            history_index: 0,
        }
    }
}

/// Greatest char-boundary byte index in `s` that is `<= idx` (clamped to
/// `s.len()`). The editor tracks `cursor_col` as a **byte** index and keeps it
/// on a char boundary; this helper restores that invariant when a caller
/// supplies a column that may be mid-character (e.g. clamping after moving
/// across lines of differing widths) and when converting external char counts.
///
/// Why byte indices: every `String` mutation the editor performs
/// (`insert`, `remove`, `truncate`, `drain`) and the `len()` comparisons used
/// for clamping are byte-indexed in `std`. Treating `cursor_col` as a byte
/// offset (advanced by `ch.len_utf8()`, never `+1`) keeps all of those sound
/// for multibyte input. Movement ops use this helper to step by *character*
/// while storing bytes.
fn snap_boundary(s: &str, idx: usize) -> usize {
    let idx = idx.min(s.len());
    // A valid char boundary (including the END of the string) is kept as-is —
    // the old `take_while(b <= idx)` returned the START of the char AT `idx`
    // for boundary positions, so a caret placed exactly after the last char
    // (e.g. autocomplete accept: `set_cursor(0, len)`) snapped one char EARLY
    // and the terminal showed "/mode|l" with the final char dangling past the
    // caret. Mid-character positions snap DOWN to the char start instead.
    if s.is_char_boundary(idx) {
        return idx;
    }
    s.char_indices()
        .take_while(|(b, _)| *b < idx)
        .last()
        .map(|(b, _)| b)
        .unwrap_or(0)
}

/// Apply a `word_navigation` helper, which works in char indices, to the
/// byte-offset caret: convert the caret to a char index, then the result back
/// to a byte offset on a char boundary.
fn word_target(s: &str, byte_col: usize, find: fn(&str, usize) -> usize) -> usize {
    let char_col = s[..snap_boundary(s, byte_col)].chars().count();
    let target = find(s, char_col);
    s.char_indices().nth(target).map_or(s.len(), |(b, _)| b)
}

/// A folded store of a large paste, keyed by the id rendered in its marker.
/// The document (`EditorState::lines`) only ever carries the short
/// `[paste #id …]` marker; the real text lives here until the draft is
/// submitted, at which point [`Editor::get_expanded_text`] splices it back in.
#[derive(Debug, Clone)]
struct PasteEntry {
    content: String,
}

/// Matches a whole `[paste #id …]` marker (group 1 = id).
fn paste_marker_regex() -> &'static regex::Regex {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"\[paste #(\d+)( (\+\d+ lines|\d+ chars))?\]").unwrap())
}

/// The byte length of the `[paste #id …]` marker ending exactly at `end`
/// (`end` is a byte offset in `line`), or `0` when no marker ends there.
fn paste_marker_len_ending_at(line: &str, end: usize) -> usize {
    for m in paste_marker_regex().find_iter(line) {
        if m.end() == end {
            return m.as_str().len();
        }
    }
    0
}

/// A paste folds when it exceeds either threshold (upstream's `handlePaste`).
const PASTE_FOLD_LINE_THRESHOLD: usize = 10;
const PASTE_FOLD_CHAR_THRESHOLD: usize = 1000;

/// Replace every `[paste #id …]` marker in `document` with its stashed body
/// (upstream's `expandPasteMarkers`). Markers whose id is absent from
/// `pastes` are left as-is so a stray marker never silently vanishes.
fn expand_paste_markers(document: &str, pastes: &HashMap<u32, PasteEntry>) -> String {
    if pastes.is_empty() || !document.contains("[paste #") {
        return document.to_string();
    }
    paste_marker_regex()
        .replace_all(document, |caps: &regex::Captures| {
            let id: u32 = caps[1].parse().unwrap_or(0);
            match pastes.get(&id) {
                Some(entry) => entry.content.clone(),
                None => caps[0].to_string(),
            }
        })
        .into_owned()
}

/// Decode the kitty CSI-u form a client may emit for `Ctrl+<letter>`
/// (`ESC[<code>;5u`), which upstream decodes before folding
/// (`handlePaste`). Scalars that are not a `Ctrl+letter` chord are left as-is.
fn decode_csi_u_ctrl(text: &str) -> String {
    if !text.contains("\x1b[") {
        return text.to_string();
    }
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| regex::Regex::new(r"\x1b\[(\d+);5u").unwrap());
    re.replace_all(text, |caps: &regex::Captures| {
        let code: u32 = caps[1].parse().unwrap_or(0);
        let lower = 0x61..=0x7a;
        let upper = 0x41..=0x5a;
        if lower.contains(&code) {
            char::from_u32(code - 96)
                .map(String::from)
                .unwrap_or_default()
        } else if upper.contains(&code) {
            char::from_u32(code - 64)
                .map(String::from)
                .unwrap_or_default()
        } else {
            caps[0].to_string()
        }
    })
    .into_owned()
}

/// A rendered segment of a logical editor line.
#[derive(Debug, Clone, Copy)]
struct VisualLine {
    logical_row: usize,
    start: usize,
    end: usize,
}

/// Split a logical editor line into display-width chunks, preferring word
/// boundaries and falling back to grapheme boundaries for overlong words.
/// Rendering and vertical navigation share this layout.
fn wrap_line_ranges(line: &str, max_width: usize) -> Vec<(usize, usize)> {
    if line.is_empty() {
        return vec![(0, 0)];
    }
    let max_width = max_width.max(1);
    let graphemes: Vec<(usize, &str, usize, bool)> = line
        .grapheme_indices(true)
        .map(|(index, grapheme)| {
            let width = crate::ansi::visible_width(grapheme);
            let whitespace = grapheme.chars().all(char::is_whitespace);
            (index, grapheme, width, whitespace)
        })
        .collect();
    let mut ranges = Vec::new();
    let mut chunk_start = 0;
    let mut current_width = 0usize;
    let mut wrap_opportunity: Option<(usize, usize)> = None;

    for (position, (index, grapheme, grapheme_width, is_whitespace)) in
        graphemes.iter().copied().enumerate()
    {
        if current_width.saturating_add(grapheme_width) > max_width {
            if let Some((break_at, width_at_break)) = wrap_opportunity {
                if current_width
                    .saturating_sub(width_at_break)
                    .saturating_add(grapheme_width)
                    <= max_width
                {
                    ranges.push((chunk_start, break_at));
                    chunk_start = break_at;
                    current_width = current_width.saturating_sub(width_at_break);
                } else if chunk_start < index {
                    ranges.push((chunk_start, index));
                    chunk_start = index;
                    current_width = 0;
                }
            } else if chunk_start < index {
                ranges.push((chunk_start, index));
                chunk_start = index;
                current_width = 0;
            }
            wrap_opportunity = None;
        }

        current_width = current_width.saturating_add(grapheme_width);
        if let Some((next_index, next_grapheme, _, next_whitespace)) = graphemes.get(position + 1) {
            if is_whitespace && !next_whitespace {
                wrap_opportunity = Some((*next_index, current_width));
            } else if !is_whitespace
                && !next_whitespace
                && (is_cjk_grapheme(grapheme) || is_cjk_grapheme(next_grapheme))
            {
                wrap_opportunity = Some((*next_index, current_width));
            }
        }
    }
    ranges.push((chunk_start, line.len()));
    ranges
}

fn is_cjk_grapheme(grapheme: &str) -> bool {
    grapheme.chars().any(|character| {
        matches!(
            character as u32,
            0x2E80..=0x2FFF
                | 0x3000..=0x30FF
                | 0x31F0..=0x31FF
                | 0x3200..=0x33FF
                | 0x3400..=0x4DBF
                | 0x4E00..=0x9FFF
                | 0xA960..=0xA97F
                | 0xAC00..=0xD7FF
                | 0xF900..=0xFAFF
                | 0xFE30..=0xFE4F
                | 0xFF00..=0xFFEF
                | 0x20000..=0x3134F
        )
    })
}

fn visual_lines(lines: &[String], width: usize) -> Vec<VisualLine> {
    lines
        .iter()
        .enumerate()
        .flat_map(|(logical_row, line)| {
            wrap_line_ranges(line, width)
                .into_iter()
                .map(move |(start, end)| VisualLine {
                    logical_row,
                    start,
                    end,
                })
        })
        .collect()
}

fn visual_column(line: &str, start: usize, end: usize, byte_col: usize) -> usize {
    crate::ansi::visible_width(&line[start..byte_col.min(end)])
}

/// Convert a display-cell column in one visual segment back to a valid byte
/// offset. If the target falls inside a wide grapheme, stop before it.
fn byte_column_at_visual_column(line: &str, start: usize, end: usize, column: usize) -> usize {
    let mut width = 0usize;
    for (idx, grapheme) in line[start..end].grapheme_indices(true) {
        let grapheme_width = crate::ansi::visible_width(grapheme);
        if width.saturating_add(grapheme_width) > column {
            return start + idx;
        }
        width = width.saturating_add(grapheme_width);
    }
    end
}

/// Border row naming rows hidden above or below the editor viewport (ice
/// `createScrollBorder`): `─── ↑ 3 more ─────`, cut to `width`.
fn scroll_border(direction: &str, hidden: usize, width: usize) -> String {
    let indicator = format!("─── {direction} {hidden} more ");
    let used = crate::ansi::visible_width(&indicator);
    if used <= width {
        indicator + &"─".repeat(width - used)
    } else {
        crate::utils::truncate_to_width(&indicator, width, "")
    }
}

/// Insert the zero-width cursor marker without slicing through ANSI styling.
fn insert_cursor_marker(text: &str, column: usize) -> String {
    let mut out = String::with_capacity(text.len() + CURSOR_MARKER.len());
    let mut visible = 0usize;
    let mut cursor = 0usize;
    let mut inserted = false;
    while cursor < text.len() {
        if text[cursor..].starts_with('\x1b') {
            let escape_len = crate::ansi::escape_len(&text[cursor..]);
            out.push_str(&text[cursor..cursor + escape_len]);
            cursor += escape_len;
            continue;
        }
        let grapheme = text[cursor..]
            .graphemes(true)
            .next()
            .expect("cursor is at a UTF-8 boundary");
        if !inserted && visible >= column {
            out.push_str(CURSOR_MARKER);
            inserted = true;
        }
        out.push_str(grapheme);
        visible = visible.saturating_add(crate::ansi::visible_width(grapheme));
        cursor += grapheme.len();
    }
    if !inserted {
        out.push_str(CURSOR_MARKER);
    }
    out
}

/// Editor - A multi-line text editor component.
pub struct Editor {
    state: Mutex<EditorState>,
    /// Display width used to lay out visual rows, updated by `render`.
    visual_width: Mutex<usize>,
    /// Sticky display column for consecutive vertical cursor movements.
    preferred_visual_column: Mutex<Option<usize>>,
    options: EditorOptions,
    #[allow(dead_code)]
    style: EditorStyle,
    keybindings: Arc<Keybindings>,
    on_submit: Mutex<Option<Arc<dyn Fn(&str) + Send + Sync>>>,
    on_change: Mutex<Option<Arc<dyn Fn(&str) + Send + Sync>>>,
    /// Undo history (text + caret snapshots). Pushed before each mutation;
    /// consecutive character insertions coalesce into one entry.
    undo_stack: Mutex<UndoStack<EditorSnapshot>>,
    /// Popped undo entries, for Ctrl+R redo.
    redo_stack: Mutex<Vec<EditorSnapshot>>,
    /// Emacs-style kill ring for Ctrl+K/U/W kills; Ctrl+Y yanks.
    kill_ring: Mutex<KillRing>,
    /// The last mutation kind, for undo coalescing + kill accumulation.
    last_action: Mutex<Option<&'static str>>,
    /// Character-jump mode (pi `jumpForward`/`jumpBackward`): `Some(1)` waits
    /// for the next printable char to jump forward to, `Some(-1)` backward.
    /// The next character input consumes it instead of inserting.
    jump_mode: Mutex<Option<i32>>,
    /// Explicit top/bottom border color. `None` ⇒ the theme's default border
    /// color. Upstream's `editor.borderColor` (used for bash mode and thinking
    /// levels via `updateEditorBorderColor`).
    border_color: Mutex<Option<Color>>,
    /// Gutter before the first line (upstream `prompt`, e.g. `"❯ "`).
    prompt: Mutex<String>,
    /// Working indicator text shown inside the top border. `None` means idle.
    /// When `Some`, the top border line renders the spinner + elapsed + message
    /// instead of plain `─` characters (pi's working-inside-input-border style).
    working: Mutex<Option<WorkingState>>,
    /// Folded pastes: `id -> content`, referenced by a `[paste #id …]` marker
    /// in `state.lines`. Mirrors upstream's `pastes` map.
    pastes: Mutex<HashMap<u32, PasteEntry>>,
    /// Monotonic id source for [`Self::pastes`] (upstream's `pasteCounter`).
    paste_counter: Mutex<u32>,
    /// Most text rows shown at once (ice: 30% of the terminal, at least 5);
    /// longer drafts scroll inside the box. `None` shows every row.
    max_visible_rows: Mutex<Option<usize>>,
}

/// State for the working indicator embedded in the editor's top border.
#[derive(Debug, Clone)]
pub struct WorkingState {
    /// Spinner animation frame index.
    pub frame: usize,
    /// When the working state started (for elapsed time display).
    pub started_at: std::time::Instant,
    /// The message to display (e.g. "Working…").
    pub message: String,
}

/// A restorable editor state snapshot (text + caret, plus the folded paste
/// store so undo can resurrect a marker's content). Selection/scroll/focus
/// are not part of undo (mirrors the TS `EditorSnapshot`).
#[derive(Clone)]
struct EditorSnapshot {
    lines: Vec<String>,
    cursor_row: usize,
    cursor_col: usize,
    pastes: HashMap<u32, PasteEntry>,
    paste_counter: u32,
}

impl Editor {
    /// Create a new editor with options.
    pub fn new(options: EditorOptions, style: EditorStyle, keybindings: Arc<Keybindings>) -> Self {
        let mut state = EditorState::default();
        if let Some(text) = &options.initial_text {
            state.lines = text.split('\n').map(|s| s.to_string()).collect();
            if state.lines.is_empty() {
                state.lines.push(String::new());
            }
        }

        Self {
            state: Mutex::new(state),
            visual_width: Mutex::new(79),
            preferred_visual_column: Mutex::new(None),
            options,
            style,
            keybindings,
            on_submit: Mutex::new(None),
            on_change: Mutex::new(None),
            undo_stack: Mutex::new(UndoStack::with_max_size(200)),
            redo_stack: Mutex::new(Vec::new()),
            kill_ring: Mutex::new(KillRing::new()),
            last_action: Mutex::new(None),
            jump_mode: Mutex::new(None),
            border_color: Mutex::new(None),
            prompt: Mutex::new(String::new()),
            working: Mutex::new(None),
            pastes: Mutex::new(HashMap::new()),
            paste_counter: Mutex::new(0),
            max_visible_rows: Mutex::new(None),
        }
    }

    /// Create a new editor with default options.
    pub fn simple() -> Self {
        Self::new(
            EditorOptions::default(),
            EditorStyle::default(),
            Arc::new(Keybindings::new()),
        )
    }

    /// Set the text content.
    pub fn set_text(&self, text: &str) {
        if let Ok(mut state) = self.state.lock() {
            state.lines = if text.is_empty() {
                vec![String::new()]
            } else {
                text.split('\n').map(|s| s.to_string()).collect()
            };
            if state.lines.is_empty() {
                state.lines.push(String::new());
            }
            state.cursor_row = 0;
            state.cursor_col = 0;
        }
        // A programmatic replacement discards any folded pastes: the new text
        // is the whole truth (mirrors upstream's `setText` clearing `pastes`).
        self.reset_pastes();
        // Mirrors native `setTextInternal`: programmatic replacement is a change.
        self.notify_change();
    }

    /// Get the text content **as shown**: folded large pastes appear as their
    /// `[paste #id …]` marker, not their full body. This is the right source
    /// for UI state that reasons about what is on screen (autocomplete,
    /// empty-checks). Use [`Self::get_expanded_text`] for anything that must
    /// see the user's real content.
    pub fn get_text(&self) -> String {
        self.state
            .lock()
            .map(|s| s.lines.join("\n"))
            .unwrap_or_default()
    }

    /// Get the text content **with folded pastes expanded** (upstream's
    /// `getExpandedText`): every `[paste #id …]` marker is replaced by the full
    /// pasted body. Submission, the external editor and extension reads all go
    /// through here so a folded paste is never sent truncated.
    pub fn get_expanded_text(&self) -> String {
        let document = self.get_text();
        let pastes = self.pastes.lock().map(|p| p.clone()).unwrap_or_default();
        expand_paste_markers(&document, &pastes)
    }

    /// Clear the folded-paste store and reset the id counter.
    fn reset_pastes(&self) {
        if let Ok(mut pastes) = self.pastes.lock() {
            pastes.clear();
        }
        if let Ok(mut counter) = self.paste_counter.lock() {
            *counter = 0;
        }
    }

    /// The folded-paste id whose marker starts exactly at byte offset
    /// `start` on line `row`, if any.
    fn paste_id_at_marker_start(&self, row: usize, start: usize) -> Option<u32> {
        let state = self.state.lock().ok()?;
        let line = state.lines.get(row)?;
        paste_marker_regex()
            .captures_iter(line)
            .find(|caps| caps.get(0).unwrap().start() == start)
            .and_then(|caps| caps[1].parse::<u32>().ok())
    }

    /// Drop a folded paste and renumber the higher ids down by one so the
    /// markers left in the document stay contiguous (upstream renumbers in
    /// `handleBackspace`). Also rewrites the surviving markers in place.
    fn remove_paste(&self, id: u32) {
        {
            let mut pastes = self.pastes.lock().unwrap();
            if pastes.remove(&id).is_none() {
                return;
            }
            // Ascending, so each id moves into the slot its predecessor just
            // vacated; HashMap order could overwrite a not-yet-moved entry.
            let mut higher: Vec<u32> = pastes.keys().copied().filter(|k| *k > id).collect();
            higher.sort_unstable();
            for old in higher {
                if let Some(entry) = pastes.remove(&old) {
                    pastes.insert(old - 1, entry);
                }
            }
        }
        {
            let mut counter = self.paste_counter.lock().unwrap();
            *counter = counter.saturating_sub(1);
        }
        // Rewrite the markers still in the document: ids above the removed one
        // shift down by one, matching the renumbered store.
        if let Ok(mut state) = self.state.lock() {
            for line in state.lines.iter_mut() {
                *line = paste_marker_regex()
                    .replace_all(line, |caps: &regex::Captures| {
                        let n: u32 = caps[1].parse().unwrap_or(0);
                        if n <= id {
                            caps[0].to_string()
                        } else {
                            // Group 2 already carries the leading space + suffix.
                            let suffix = caps.get(2).map(|m| m.as_str()).unwrap_or("");
                            format!("[paste #{}{suffix}]", n - 1)
                        }
                    })
                    .into_owned();
            }
        }
    }

    /// Fold a pasted block: short pastes are inserted verbatim; a block over
    /// the size threshold is stashed and replaced by a `[paste #id …]` marker
    /// so the input box stays readable. Mirrors upstream's `handlePaste`
    /// (fold when `>10` lines or `>1000` chars).
    ///
    /// A single-line pasted file path is special-cased by the caller (image
    /// attachment), so this only handles text.
    pub fn handle_paste(&self, text: &str) {
        // Decode the CSI-u `Ctrl+<letter>` form a client may hand us, and
        // normalize line endings + tabs (upstream's `normalizeText`).
        let decoded = decode_csi_u_ctrl(text)
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .replace('\t', "    ");
        // Drop control characters, keeping newlines (upstream filters
        // `char === "\n" || char.charCodeAt(0) >= 32`).
        let filtered: String = decoded
            .chars()
            .filter(|c| *c == '\n' || (*c as u32) >= 32)
            .collect();
        if filtered.is_empty() {
            return;
        }

        let line_count = filtered.split('\n').count();
        let char_count = filtered.chars().count();
        if line_count > PASTE_FOLD_LINE_THRESHOLD || char_count > PASTE_FOLD_CHAR_THRESHOLD {
            self.push_undo("insert");
            if self.has_selection() {
                self.delete_selection_no_undo();
            }
            let (id, by_lines, count) = {
                let mut counter = self.paste_counter.lock().unwrap();
                *counter += 1;
                let id = *counter;
                if line_count > PASTE_FOLD_LINE_THRESHOLD {
                    (id, true, line_count)
                } else {
                    (id, false, char_count)
                }
            };
            if let Ok(mut pastes) = self.pastes.lock() {
                pastes.insert(id, PasteEntry { content: filtered });
            }
            let marker = if by_lines {
                format!("[paste #{id} +{count} lines]")
            } else {
                format!("[paste #{id} {count} chars]")
            };
            self.insert_no_undo(&marker);
            *self.last_action.lock().unwrap() = Some("insert");
        } else {
            self.insert(&filtered);
        }
    }

    /// Clear the editor.
    pub fn clear(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.lines = vec![String::new()];
            state.cursor_row = 0;
            state.cursor_col = 0;
            state.selection_anchor = None;
        }
        self.reset_pastes();
        // Upstream's editor notifies change observers for programmatic clears
        // (`setTextInternal`), so derived UI state (bash-mode border color,
        // autocomplete) drops the stale draft. `submit()` relies on this to
        // reset the border after routing a `!command`.
        self.notify_change();
    }

    /// Set callback for when text is submitted (Enter pressed).
    pub fn on_submit(&self, callback: Arc<dyn Fn(&str) + Send + Sync>) {
        if let Ok(mut cb) = self.on_submit.lock() {
            *cb = Some(callback);
        }
    }

    /// Set callback for when text changes.
    pub fn on_change(&self, callback: Arc<dyn Fn(&str) + Send + Sync>) {
        if let Ok(mut cb) = self.on_change.lock() {
            *cb = Some(callback);
        }
    }

    /// Get cursor position.
    pub fn cursor_position(&self) -> (usize, usize) {
        self.state
            .lock()
            .map(|s| (s.cursor_row, s.cursor_col))
            .unwrap_or((0, 0))
    }

    /// Set the cursor position (clamped to text bounds), clearing any
    /// in-progress selection. Used by autocomplete to place the caret after
    /// accepting a suggestion (`set_text` resets the caret to the start).
    ///
    /// `col` is interpreted as a **byte** offset (callers in the autocomplete
    /// path pass byte indices from `String::len`/`char_indices`) and snapped to
    /// the nearest preceding char boundary so multibyte input never lands
    /// mid-character.
    pub fn set_cursor(&self, row: usize, col: usize) {
        if let Ok(mut state) = self.state.lock() {
            let row = row.min(state.lines.len().saturating_sub(1));
            let col = snap_boundary(&state.lines[row], col);
            state.cursor_row = row;
            state.cursor_col = col;
            state.selection_anchor = None;
        }
        self.reset_preferred_visual_column();
    }

    /// Place the caret from a zero-based rendered content row and display-cell
    /// column. `width` is the editor's actual render width; wrapping and wide
    /// graphemes use the same layout as rendering and vertical navigation.
    pub fn set_cursor_at_visual_position(
        &self,
        visual_row: usize,
        display_column: usize,
        width: usize,
    ) -> bool {
        let width = width.max(1);
        let prompt = self
            .prompt
            .lock()
            .map(|prompt| prompt.clone())
            .unwrap_or_default();
        let prompt_width = crate::ansi::visible_width(&prompt);
        let prompt_width = if prompt_width > 0
            && prompt_width + 2 <= width.saturating_sub(self.options.padding_x * 2)
        {
            prompt_width
        } else {
            0
        };
        let text_left = self.options.padding_x + prompt_width;
        let content_width = width.saturating_sub(text_left).max(1);
        let visual_width = content_width.saturating_sub(1).max(1);

        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        let visual = visual_lines(&state.lines, visual_width);
        let Some(line) = visual.get(
            state
                .scroll_offset
                .saturating_add(visual_row)
                .min(visual.len().saturating_sub(1)),
        ) else {
            return false;
        };
        let target_column = display_column.saturating_sub(text_left);
        let byte_column = byte_column_at_visual_column(
            &state.lines[line.logical_row],
            line.start,
            line.end,
            target_column,
        );
        state.cursor_row = line.logical_row;
        state.cursor_col = byte_column;
        state.selection_anchor = None;
        drop(state);
        self.reset_preferred_visual_column();
        true
    }

    /// Snapshot the current text + caret state (for undo/redo), including the
    /// folded-paste store so undo can bring a dropped paste back.
    fn snapshot_state(&self) -> EditorSnapshot {
        let state = self.state.lock().unwrap();
        EditorSnapshot {
            lines: state.lines.clone(),
            cursor_row: state.cursor_row,
            cursor_col: state.cursor_col,
            pastes: self.pastes.lock().map(|p| p.clone()).unwrap_or_default(),
            paste_counter: self.paste_counter.lock().map(|c| *c).unwrap_or(0),
        }
    }

    /// Restore a snapshot, notifying change observers.
    fn restore_state(&self, snap: &EditorSnapshot) {
        if let Ok(mut state) = self.state.lock() {
            state.lines = snap.lines.clone();
            state.cursor_row = snap.cursor_row;
            state.cursor_col = snap.cursor_col;
            // Snapshots don't carry the selection; undo/redo ends it.
            state.selection_anchor = None;
        }
        if let Ok(mut pastes) = self.pastes.lock() {
            *pastes = snap.pastes.clone();
        }
        if let Ok(mut counter) = self.paste_counter.lock() {
            *counter = snap.paste_counter;
        }
        self.notify_change();
    }

    /// Record an undo snapshot before a mutation. `action` names the mutation
    /// kind: consecutive `"insert"` actions coalesce into a single undo entry
    /// (typing a word undoes as one step, mirroring the TS coalescing); any
    /// other mutation starts a fresh entry. A new mutation clears the redo
    /// stack (the classic undo/redo semantics).
    fn push_undo(&self, action: &'static str) {
        let coalesce = action == "insert" && *self.last_action.lock().unwrap() == Some("insert");
        if !coalesce {
            let snap = self.snapshot_state();
            if let Ok(mut stack) = self.undo_stack.lock() {
                stack.push(snap);
            }
            if let Ok(mut redo) = self.redo_stack.lock() {
                redo.clear();
            }
        }
        *self.last_action.lock().unwrap() = Some(action);
    }

    /// Undo the last mutation (Ctrl+-). Restores the prior snapshot; the
    /// undone state moves to the redo stack for Ctrl+R.
    pub fn undo(&self) {
        let popped = self.undo_stack.lock().unwrap().pop();
        let Some(snap) = popped else { return };
        // The current state is the redo target.
        self.redo_stack.lock().unwrap().push(self.snapshot_state());
        self.restore_state(&snap);
        *self.last_action.lock().unwrap() = None;
    }

    /// Redo the last undone mutation (Ctrl+R).
    pub fn redo(&self) {
        let popped = self.redo_stack.lock().unwrap().pop();
        let Some(snap) = popped else { return };
        self.undo_stack.lock().unwrap().push(self.snapshot_state());
        self.restore_state(&snap);
        *self.last_action.lock().unwrap() = None;
    }

    /// Push killed (deleted) text onto the kill ring, accumulating consecutive
    /// kills into one entry (mirrors the TS kill-ring semantics).
    fn kill(&self, text: String, prepend: bool) {
        if text.is_empty() {
            return;
        }
        let accumulate = *self.last_action.lock().unwrap() == Some("kill");
        if let Ok(mut ring) = self.kill_ring.lock() {
            ring.push(
                &text,
                PushOptions {
                    prepend,
                    accumulate,
                },
            );
        }
        *self.last_action.lock().unwrap() = Some("kill");
    }

    /// Yank (paste) the most recent kill at the cursor (Ctrl+Y). Replaces an
    /// active selection.
    pub fn yank(&self) {
        let text = self.kill_ring.lock().unwrap().peek().map(str::to_string);
        let Some(text) = text else { return };
        self.push_undo("yank");
        if self.has_selection() {
            self.delete_selection_no_undo();
        }
        self.insert_no_undo(&text);
        *self.last_action.lock().unwrap() = Some("yank");
    }

    /// Yank-pop: rotate the kill ring and yank the next entry (Alt+Y).
    pub fn yank_pop(&self) {
        if self.kill_ring.lock().unwrap().len() <= 1 {
            return;
        }
        self.kill_ring.lock().unwrap().rotate();
        self.yank();
    }

    // -------------------------------------------------------------------
    // Selection (Shift+arrows select, Ctrl+X/C/V cut/copy/paste).
    // -------------------------------------------------------------------

    /// Whether a selection is active.
    pub fn has_selection(&self) -> bool {
        self.state.lock().unwrap().selection_anchor.is_some()
    }

    /// The ordered selection span `(start_row, start_col, end_row, end_col)`
    /// with start <= end (byte offsets, both on char boundaries).
    fn selection_ordered(&self) -> Option<(usize, usize, usize, usize)> {
        let state = self.state.lock().unwrap();
        let (ar, ac) = state.selection_anchor?;
        let (br, bc) = (state.cursor_row, state.cursor_col);
        Some(if (ar, ac) <= (br, bc) {
            (ar, ac, br, bc)
        } else {
            (br, bc, ar, ac)
        })
    }

    /// The selected text ("\n"-joined across rows), if any.
    pub fn selected_text(&self) -> Option<String> {
        let (sr, sc, er, ec) = self.selection_ordered()?;
        let lines = self.state.lock().unwrap().lines.clone();
        if sr == er {
            Some(lines[sr][sc..ec].to_string())
        } else {
            let mut out = String::new();
            out.push_str(&lines[sr][sc..]);
            for r in sr + 1..er {
                out.push('\n');
                out.push_str(&lines[r]);
            }
            out.push('\n');
            out.push_str(&lines[er][..ec]);
            Some(out)
        }
    }

    /// The selection span on `row`, as `(start_col, end_col)` byte offsets,
    /// for render-time highlighting. `None` when the row is outside the span.
    fn selection_span_on_row(&self, state: &EditorState, row: usize) -> Option<(usize, usize)> {
        let (ar, ac) = state.selection_anchor?;
        let (br, bc) = (state.cursor_row, state.cursor_col);
        let (sr, sc, er, ec) = if (ar, ac) <= (br, bc) {
            (ar, ac, br, bc)
        } else {
            (br, bc, ar, ac)
        };
        if row < sr || row > er {
            return None;
        }
        if sr == er {
            (sc != ec).then_some((sc, ec))
        } else if row == sr {
            (sc < state.lines[row].len()).then_some((sc, state.lines[row].len()))
        } else if row == er {
            (ec > 0).then_some((0, ec))
        } else {
            (!state.lines[row].is_empty()).then_some((0, state.lines[row].len()))
        }
    }

    /// Seed the selection anchor at the current caret (Shift+direction start).
    fn begin_selection(&self) {
        let mut state = self.state.lock().unwrap();
        if state.selection_anchor.is_none() {
            state.selection_anchor = Some((state.cursor_row, state.cursor_col));
        }
    }

    /// Clear the active selection (plain cursor movement / edits).
    fn clear_selection(&self) {
        self.state.lock().unwrap().selection_anchor = None;
    }

    /// Delete the selected span, leaving the caret at its start. Pushes an
    /// undo snapshot; no-op without a selection.
    fn delete_selection(&self) {
        if !self.has_selection() {
            return;
        }
        self.push_undo("edit");
        self.delete_selection_no_undo();
    }

    /// Delete the selected span WITHOUT an undo snapshot (caller pushed one,
    /// e.g. a replace-through-insert coalesced with the insertion).
    fn delete_selection_no_undo(&self) {
        let (sr, sc, er, ec) = match self.selection_ordered() {
            Some(s) => s,
            None => return,
        };
        let mut state = self.state.lock().unwrap();
        if sr == er {
            state.lines[sr].replace_range(sc..ec, "");
            state.cursor_row = sr;
            state.cursor_col = sc;
        } else {
            let mut joined = state.lines[sr][..sc].to_string();
            joined.push_str(&state.lines[er][ec..]);
            state.lines.drain(sr + 1..=er);
            state.lines[sr] = joined;
            state.cursor_row = sr;
            state.cursor_col = sc;
        }
        state.selection_anchor = None;
        drop(state);
        self.notify_change();
    }

    /// Copy the selection onto the kill ring (Ctrl+C). No-op without one.
    pub fn copy_selection(&self) -> bool {
        let Some(text) = self.selected_text() else {
            return false;
        };
        if text.is_empty() {
            return false;
        }
        if let Ok(mut ring) = self.kill_ring.lock() {
            ring.push(&text, PushOptions::default());
        }
        true
    }

    /// Cut the selection (copy + delete, Ctrl+X). Returns whether anything
    /// was cut.
    pub fn cut_selection(&self) -> bool {
        if !self.copy_selection() {
            return false;
        }
        self.delete_selection();
        true
    }

    /// Insert text at cursor position. Replaces an active selection (one undo
    /// step: the snapshot is taken before the selection is deleted).
    pub fn insert(&self, text: &str) {
        self.push_undo("insert");
        if self.has_selection() {
            self.delete_selection_no_undo();
        }
        self.insert_no_undo(text);
    }

    /// Insert without an undo snapshot (caller pushed one — yank/replace
    /// paths). Notifies change observers.
    ///
    /// Line endings are normalized: `\r\n` and a lone `\r` are treated as one
    /// newline. Pasted Windows text (and the `Event::Paste` payload on Unix,
    /// which preserves CRLF) would otherwise insert literal carriage returns
    /// into the line buffer — they render as garbage and break `cursor_col`
    /// arithmetic on the next edit.
    fn insert_no_undo(&self, text: &str) {
        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        if let Ok(mut state) = self.state.lock() {
            for ch in normalized.chars() {
                if ch == '\n' {
                    // Split line at cursor
                    let row = state.cursor_row;
                    let col = state.cursor_col;
                    let current_line = &mut state.lines[row];
                    let after = current_line.split_off(col);
                    state.lines.insert(row + 1, after);
                    state.cursor_row += 1;
                    state.cursor_col = 0;
                } else {
                    let row = state.cursor_row;
                    let col = state.cursor_col;
                    state.lines[row].insert(col, ch);
                    // Advance by the BYTE length of the inserted char so
                    // `cursor_col` stays a valid char boundary (and a sound
                    // byte index for the next `String` mutation). The old
                    // `+= 1` was a char count and panicked on multibyte input
                    // the moment a second keystroke landed mid-character.
                    state.cursor_col += ch.len_utf8();
                }
            }
        }
        self.notify_change();
    }

    /// Delete character before cursor (Backspace). With a selection, deletes
    /// the selection instead.
    fn backspace(&self) {
        if self.has_selection() {
            self.delete_selection();
            return;
        }
        // Backspacing over a `[paste #id …]` marker deletes the whole marker
        // (and drops / renumbers the folded pastes). The marker is one atomic
        // unit — the user cannot nibble it into an orphaned placeholder.
        let deleted_marker = {
            let state = self.state.lock().unwrap();
            let row = state.cursor_row;
            (state.cursor_col > 0)
                .then(|| paste_marker_len_ending_at(&state.lines[row], state.cursor_col))
                .filter(|len| *len > 0)
                .map(|len| (row, state.cursor_col - len, len))
        };
        if let Some((row, start, len)) = deleted_marker {
            // Snapshot FIRST: `remove_paste` mutates the paste store and
            // renumbers the surviving markers, so undo must capture the
            // pre-deletion state to be able to bring them all back.
            self.push_undo("edit");
            let id = self.paste_id_at_marker_start(row, start);
            // Delete the marker text BEFORE renumbering, so the renumber pass
            // (which can change a marker's byte length when an id crosses
            // `#10 -> #9`) never has to keep a stale `start..start+len` valid.
            if let Ok(mut state) = self.state.lock() {
                state.lines[row].replace_range(start..start + len, "");
                state.cursor_col = start;
            }
            if let Some(id) = id {
                self.remove_paste(id);
            }
            self.notify_change();
            return;
        }
        self.push_undo("edit");
        if let Ok(mut state) = self.state.lock() {
            if state.cursor_col > 0 {
                let row = state.cursor_row;
                // Step back by the byte length of the char immediately
                // before the cursor, then remove that char. (`remove` takes
                // a byte index and must land on a boundary.) Compute the
                // length through a shared borrow first so the mutable borrow
                // for `remove` is the only outstanding alias.
                let prev_len = state.lines[row][..state.cursor_col]
                    .chars()
                    .last()
                    .map(|c| c.len_utf8())
                    .unwrap_or(0);
                let col = state.cursor_col - prev_len;
                state.lines[row].remove(col);
                state.cursor_col = col;
            } else if state.cursor_row > 0 {
                // Join with previous line
                let row = state.cursor_row;
                let current = state.lines.remove(row);
                state.cursor_row -= 1;
                let prev_row = state.cursor_row;
                state.cursor_col = state.lines[prev_row].len();
                state.lines[prev_row].push_str(&current);
            }
        }
        self.notify_change();
    }

    /// Delete character at cursor (Delete). With a selection, deletes the
    /// selection instead.
    fn delete(&self) {
        if self.has_selection() {
            self.delete_selection();
            return;
        }
        self.push_undo("edit");
        if let Ok(mut state) = self.state.lock() {
            let row = state.cursor_row;
            let line_len = state.lines[row].len();
            if state.cursor_col < line_len {
                let col = state.cursor_col;
                state.lines[row].remove(col);
            } else if row < state.lines.len() - 1 {
                // Join with next line
                let next = state.lines.remove(row + 1);
                state.lines[row].push_str(&next);
            }
        }
        self.notify_change();
    }

    /// Move cursor left.
    fn cursor_left(&self) {
        self.reset_preferred_visual_column();
        if let Ok(mut state) = self.state.lock() {
            if state.cursor_col > 0 {
                let row = state.cursor_row;
                let line = &state.lines[row];
                // A folded paste marker is atomic: step the whole marker.
                let marker = paste_marker_len_ending_at(line, state.cursor_col);
                if marker > 0 {
                    state.cursor_col -= marker;
                    return;
                }
                // Retrear by the byte length of the preceding char so the
                // caret stays on a char boundary.
                let prev_len = line[..state.cursor_col]
                    .chars()
                    .last()
                    .map(|c| c.len_utf8())
                    .unwrap_or(0);
                state.cursor_col -= prev_len;
            } else if state.cursor_row > 0 {
                state.cursor_row -= 1;
                state.cursor_col = state.lines[state.cursor_row].len();
            }
        }
    }

    /// Move cursor right.
    fn cursor_right(&self) {
        self.reset_preferred_visual_column();
        if let Ok(mut state) = self.state.lock() {
            let row = state.cursor_row;
            let line = &state.lines[row];
            if state.cursor_col < line.len() {
                // A folded paste marker is atomic: step the whole marker.
                for m in paste_marker_regex().find_iter(line) {
                    if m.start() == state.cursor_col {
                        state.cursor_col = m.end();
                        return;
                    }
                }
                // Advance by the byte length of the char at the cursor.
                let ch_len = line[state.cursor_col..]
                    .chars()
                    .next()
                    .map(|c| c.len_utf8())
                    .unwrap_or(0);
                state.cursor_col += ch_len;
            } else if row < state.lines.len() - 1 {
                state.cursor_row += 1;
                state.cursor_col = 0;
            }
        }
    }

    fn move_cursor_vertical(&self, direction: isize) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        let width = self.visual_width.lock().map(|w| *w).unwrap_or(79).max(1);
        let visual = visual_lines(&state.lines, width);
        let Some(current) = visual.iter().position(|line| {
            line.logical_row == state.cursor_row
                && (state.cursor_col >= line.start
                    && (state.cursor_col < line.end
                        || (line.end == state.lines[state.cursor_row].len()
                            && state.cursor_col == line.end)))
        }) else {
            return;
        };
        let target = if direction < 0 {
            current.checked_sub(1)
        } else {
            (current + 1 < visual.len()).then_some(current + 1)
        };
        let Some(target) = target else {
            return;
        };
        let source = visual[current];
        let destination = visual[target];
        let source_text = &state.lines[source.logical_row];
        let destination_text = &state.lines[destination.logical_row];
        let current_column = visual_column(source_text, source.start, source.end, state.cursor_col);
        let mut preferred = self.preferred_visual_column.lock().unwrap();
        let preferred_column = *preferred.get_or_insert(current_column);
        let destination_width =
            crate::ansi::visible_width(&destination_text[destination.start..destination.end]);
        let continues_on_next_visual_row = visual
            .get(target + 1)
            .is_some_and(|next| next.logical_row == destination.logical_row);
        let max_column = if continues_on_next_visual_row {
            destination_width.saturating_sub(1)
        } else {
            destination_width
        };
        let target_byte_column = byte_column_at_visual_column(
            destination_text,
            destination.start,
            destination.end,
            preferred_column.min(max_column),
        );
        state.cursor_row = destination.logical_row;
        state.cursor_col = target_byte_column;
    }

    /// Move cursor to the previous rendered row.
    fn cursor_up(&self) {
        self.move_cursor_vertical(-1);
    }

    /// Move cursor to the next rendered row.
    fn cursor_down(&self) {
        self.move_cursor_vertical(1);
    }

    fn reset_preferred_visual_column(&self) {
        if let Ok(mut preferred) = self.preferred_visual_column.lock() {
            *preferred = None;
        }
    }

    /// Move cursor to start of line.
    fn cursor_home(&self) {
        self.reset_preferred_visual_column();
        if let Ok(mut state) = self.state.lock() {
            state.cursor_col = 0;
        }
    }

    /// Move cursor to end of line.
    fn cursor_end(&self) {
        self.reset_preferred_visual_column();
        if let Ok(mut state) = self.state.lock() {
            let row = state.cursor_row;
            state.cursor_col = state.lines[row].len();
        }
    }

    /// Kill the word before the cursor (Ctrl+W / Alt+Backspace): Emacs
    /// backward-kill-word — skip the current word, then the intervening
    /// delimiters, landing on the previous word's start.
    fn kill_word_backward(&self) {
        self.push_undo("kill");
        let killed = if let Ok(mut state) = self.state.lock() {
            let row = state.cursor_row;
            let col = state.cursor_col;
            let start = word_target(
                &state.lines[row],
                col,
                crate::word_navigation::find_word_backward,
            );
            let dead: String = state.lines[row].drain(start..col).collect();
            state.cursor_col = start;
            dead
        } else {
            String::new()
        };
        self.kill(killed, true);
        self.notify_change();
    }

    /// Kill the word after the cursor (Alt+D / Alt+Delete).
    fn kill_word_forward(&self) {
        self.push_undo("kill");
        let killed = if let Ok(mut state) = self.state.lock() {
            let row = state.cursor_row;
            let col = state.cursor_col;
            // Kill only the current word (not the trailing delimiter) —
            // `find_word_end` stops at the last word char.
            let end = word_target(
                &state.lines[row],
                col,
                crate::word_navigation::find_word_end,
            );
            let dead: String = state.lines[row].drain(col..end).collect();
            dead
        } else {
            String::new()
        };
        self.kill(killed, false);
        self.notify_change();
    }

    /// Jump to the previous word boundary (Alt+Left).
    fn cursor_word_left(&self) {
        if let Ok(mut state) = self.state.lock() {
            let row = state.cursor_row;
            state.cursor_col = word_target(
                &state.lines[row],
                state.cursor_col,
                crate::word_navigation::find_word_backward,
            );
        }
    }

    /// Jump to the next word boundary (Alt+Right).
    fn cursor_word_right(&self) {
        if let Ok(mut state) = self.state.lock() {
            let row = state.cursor_row;
            state.cursor_col = word_target(
                &state.lines[row],
                state.cursor_col,
                crate::word_navigation::find_word_forward,
            );
        }
    }

    /// Page-scroll the editor caret up/down by a page (pi `tui.editor.pageUp/
    /// pageDown`): move the cursor row by a page and clamp the column to the
    /// target line (page = 10 rows; a terminal-height-aware version would need
    /// the viewport, which the component doesn't own).
    fn page_scroll(&self, dir: i32) {
        const PAGE: usize = 10;
        if let Ok(mut state) = self.state.lock() {
            let target = if dir > 0 {
                (state.cursor_row + PAGE).min(state.lines.len().saturating_sub(1))
            } else {
                state.cursor_row.saturating_sub(PAGE)
            };
            state.cursor_row = target;
            let max = state.lines[target].len();
            state.cursor_col = snap_boundary(&state.lines[target], state.cursor_col.min(max));
        }
    }

    /// Jump the caret to the next/previous occurrence of `target` (pi
    /// `jumpToChar`): scan the current line from after/before the caret, then
    /// onward/backward line by line. Byte-safe: `find`/`rfind` return char
    /// boundaries and all search offsets start on one. On the caret's line a
    /// forward search starts after the char under the caret and a backward
    /// search accepts any match that starts before the caret.
    fn jump_to_char(&self, target: char, dir: i32) {
        let forward = dir > 0;
        let mut state = self.state.lock().unwrap();
        let start_row = state.cursor_row;
        let mut row = start_row;
        let step: isize = if forward { 1 } else { -1 };
        loop {
            let line = &state.lines[row];
            let search_from = if row == start_row {
                let caret = snap_boundary(line, state.cursor_col);
                if forward {
                    line[caret..]
                        .chars()
                        .next()
                        .map_or(line.len(), |c| caret + c.len_utf8())
                } else {
                    caret
                }
            } else if forward {
                0
            } else {
                line.len()
            };
            let found = if forward {
                line[search_from..].find(target).map(|i| search_from + i)
            } else {
                line[..search_from].rfind(target)
            };
            if let Some(idx) = found {
                state.cursor_row = row;
                state.cursor_col = idx;
                return;
            }
            let next = row as isize + step;
            if next < 0 || next >= state.lines.len() as isize {
                return;
            }
            row = next as usize;
        }
    }

    /// Size the text viewport for a terminal `rows` tall: at most 30% of the
    /// rows, never fewer than 5 (ice `editor.ts` `maxVisibleLines`). A draft
    /// with more rows scrolls inside the box, following the caret, so a long
    /// prompt no longer pushes the transcript off the screen.
    pub fn set_terminal_rows(&self, rows: usize) {
        if let Ok(mut max) = self.max_visible_rows.lock() {
            *max = Some((rows.saturating_mul(3) / 10).max(5));
        }
    }

    /// Set the gutter drawn before the first line in the border color (e.g.
    /// `"❯ "`); wrapped and later lines are indented to match. Empty removes
    /// it. Mirrors upstream `Editor.setPrompt`.
    pub fn set_prompt(&self, prompt: &str) {
        if let Ok(mut guard) = self.prompt.lock() {
            *guard = prompt.to_string();
        }
    }

    /// Set an explicit top/bottom border color. `None` restores the theme's
    /// default border color. Mirrors upstream's `editor.borderColor`.
    pub fn set_border_color(&self, color: Option<Color>) {
        if let Ok(mut guard) = self.border_color.lock() {
            *guard = color;
        }
    }

    /// The current explicit border color, if any.
    pub fn border_color(&self) -> Option<Color> {
        self.border_color.lock().ok().and_then(|guard| *guard)
    }

    /// Set the working state. When Some, shows a spinner + elapsed time + message
    /// inside the top border. When None, shows the normal border.
    pub fn set_working(&self, working: Option<WorkingState>) {
        if let Ok(mut guard) = self.working.lock() {
            *guard = working;
        }
    }

    /// Get the current working state, if any.
    pub fn working(&self) -> Option<WorkingState> {
        self.working.lock().ok().and_then(|guard| guard.clone())
    }

    /// Submit current text (Enter).
    fn submit(&self) {
        // Send the EXPANDED draft: a folded `[paste #id …]` marker must reach
        // the model as the user's real content, not the placeholder.
        let text = self.get_expanded_text();
        if let Ok(mut state) = self.state.lock() {
            if !text.is_empty() {
                state.history.push(text.clone());
                state.history_index = state.history.len();
            }
        }
        // Submitted text is committed — don't let Ctrl+- resurrect it.
        self.undo_stack.lock().unwrap().clear();
        self.redo_stack.lock().unwrap().clear();
        *self.last_action.lock().unwrap() = None;
        if let Ok(cb) = self.on_submit.lock() {
            if let Some(callback) = cb.as_ref() {
                callback(&text);
            }
        }
        self.clear();
    }

    /// Notify change callback.
    fn notify_change(&self) {
        self.reset_preferred_visual_column();
        if let Ok(cb) = self.on_change.lock() {
            if let Some(callback) = cb.as_ref() {
                let text = self.get_text();
                callback(&text);
            }
        }
    }

    /// Route configured editor actions before text input and selection handling.
    pub fn handle_key(&self, mut key: crossterm::event::KeyEvent) -> bool {
        use KeyCode::*;
        use KeyModifiers as M;

        // Terminal protocols can deliver Enter as a raw CR/LF codepoint.
        if matches!(key.code, Char('\n' | '\r')) {
            key.code = Enter;
        }
        let actions = [
            ("tui.editor.cursorLeft", Left, M::NONE),
            ("tui.editor.cursorRight", Right, M::NONE),
            ("tui.editor.cursorUp", Up, M::NONE),
            ("tui.editor.cursorDown", Down, M::NONE),
            ("tui.editor.cursorLineStart", Home, M::NONE),
            ("tui.editor.cursorLineEnd", End, M::NONE),
            ("tui.editor.cursorWordLeft", Left, M::ALT),
            ("tui.editor.cursorWordRight", Right, M::ALT),
            ("tui.editor.deleteCharBackward", Backspace, M::NONE),
            ("tui.editor.deleteCharForward", Delete, M::NONE),
            ("tui.editor.deleteWordBackward", Backspace, M::ALT),
            ("tui.editor.deleteWordForward", Delete, M::ALT),
            ("tui.editor.deleteToLineStart", Char('u'), M::CONTROL),
            ("tui.editor.deleteToLineEnd", Char('k'), M::CONTROL),
            ("tui.editor.yank", Char('y'), M::CONTROL),
            ("tui.editor.yankPop", Char('y'), M::ALT),
            ("tui.editor.undo", Char('-'), M::CONTROL),
            ("tui.editor.jumpForward", Char(']'), M::CONTROL),
            ("tui.editor.jumpBackward", Char(']'), M::CONTROL | M::ALT),
            ("tui.editor.pageUp", PageUp, M::CONTROL),
            ("tui.editor.pageDown", PageDown, M::CONTROL),
            ("tui.input.newLine", Enter, M::SHIFT),
            ("tui.input.submit", Enter, M::NONE),
            ("tui.input.copy", Char('c'), M::CONTROL),
        ];
        for &(id, code, modifiers) in &actions {
            if self.keybindings.matches(&key, id) {
                return self
                    .handle_key_unconfigured(crossterm::event::KeyEvent::new(code, modifiers));
            }
        }
        let is_default = |id, key: &crossterm::event::KeyEvent| {
            self.keybindings
                .get_definition(id)
                .is_some_and(|definition| {
                    definition
                        .default_keys
                        .iter()
                        .any(|combo| combo.code == key.code && combo.modifiers == key.modifiers)
                })
        };
        // Shift extends a configured cursor movement. A rebound movement must
        // also release its former shifted selection shortcut.
        if key.modifiers.contains(M::SHIFT) {
            let mut movement = key;
            movement.modifiers.remove(M::SHIFT);
            if let Char(character) = movement.code {
                movement.code = Char(character.to_ascii_lowercase());
            }
            for &(id, code, _) in &actions[..6] {
                if self.keybindings.matches(&movement, id) {
                    return self
                        .handle_key_unconfigured(crossterm::event::KeyEvent::new(code, M::SHIFT));
                }
            }
            if actions[..6]
                .iter()
                .any(|(id, _, _)| is_default(*id, &movement))
            {
                return false;
            }
        }
        // Do not keep a default key active after rebinding or explicitly
        // unbinding its action. Other keys retain the text/selection protocol.
        if actions.iter().any(|(id, _, _)| is_default(*id, &key)) {
            return false;
        }
        self.handle_key_unconfigured(key)
    }

    fn handle_key_unconfigured(&self, key: crossterm::event::KeyEvent) -> bool {
        match (key.modifiers, key.code) {
            // Navigation — plain movement clears any active selection;
            // Shift+movement extends it.
            (KeyModifiers::NONE, KeyCode::Left) => {
                self.clear_selection();
                self.cursor_left();
            }
            (KeyModifiers::NONE, KeyCode::Right) => {
                self.clear_selection();
                self.cursor_right();
            }
            (KeyModifiers::NONE, KeyCode::Up) => {
                self.clear_selection();
                self.cursor_up();
            }
            (KeyModifiers::NONE, KeyCode::Down) => {
                self.clear_selection();
                self.cursor_down();
            }
            (KeyModifiers::NONE, KeyCode::Home) => {
                self.clear_selection();
                self.cursor_home();
            }
            (KeyModifiers::NONE, KeyCode::End) => {
                self.clear_selection();
                self.cursor_end();
            }
            (KeyModifiers::SHIFT, KeyCode::Left) => {
                self.begin_selection();
                self.cursor_left();
            }
            (KeyModifiers::SHIFT, KeyCode::Right) => {
                self.begin_selection();
                self.cursor_right();
            }
            (KeyModifiers::SHIFT, KeyCode::Up) => {
                self.begin_selection();
                self.cursor_up();
            }
            (KeyModifiers::SHIFT, KeyCode::Down) => {
                self.begin_selection();
                self.cursor_down();
            }
            (KeyModifiers::SHIFT, KeyCode::Home) => {
                self.begin_selection();
                self.cursor_home();
            }
            (KeyModifiers::SHIFT, KeyCode::End) => {
                self.begin_selection();
                self.cursor_end();
            }

            // Editing
            (KeyModifiers::NONE, KeyCode::Backspace) => self.backspace(),
            (KeyModifiers::NONE, KeyCode::Delete) => self.delete(),
            (KeyModifiers::NONE, KeyCode::Enter)
            | (KeyModifiers::NONE, KeyCode::Char('\n'))
            | (KeyModifiers::NONE, KeyCode::Char('\r')) => {
                // Submit on Enter (mirrors TS `tui.input.submit`). The
                // `Char('\n')`/`Char('\r')` forms cover a client whose terminal
                // hands us the codepoint directly (e.g. the kitty keyboard
                // protocol's CSI-u path); they used to fall through to the
                // generic char arm and insert a literal newline.
                self.submit();
            }
            // Shift+Enter inserts a newline (mirrors `tui.input.newLine`).
            (KeyModifiers::SHIFT, KeyCode::Enter)
            | (KeyModifiers::SHIFT, KeyCode::Char('\r'))
            | (KeyModifiers::SHIFT, KeyCode::Char('\n')) => self.insert("\n"),

            // Ctrl shortcuts
            (KeyModifiers::CONTROL, KeyCode::Char('a')) => self.cursor_home(),
            (KeyModifiers::CONTROL, KeyCode::Char('e')) => self.cursor_end(),
            (KeyModifiers::CONTROL, KeyCode::Char('k')) => {
                // Kill to end of line (push the killed text onto the ring).
                self.push_undo("kill");
                let killed = if let Ok(mut state) = self.state.lock() {
                    let row = state.cursor_row;
                    let col = state.cursor_col;
                    let dead: String = state.lines[row].drain(col..).collect();
                    state.lines[row].truncate(col);
                    dead
                } else {
                    String::new()
                };
                self.kill(killed, false);
                self.notify_change();
            }
            (KeyModifiers::CONTROL, KeyCode::Char('u')) => {
                // Kill to start of line (killed text prepends to the ring entry
                // so Ctrl+Y pastes it back in the same order).
                self.push_undo("kill");
                let killed = if let Ok(mut state) = self.state.lock() {
                    let row = state.cursor_row;
                    let col = state.cursor_col;
                    let before: String = state.lines[row].drain(..col).collect();
                    state.cursor_col = 0;
                    before
                } else {
                    String::new()
                };
                self.kill(killed, true);
                self.notify_change();
            }
            // Undo / redo (pi binds undo to Ctrl+-; Ctrl+R redo is a Rust port
            // convenience since pi has no redo binding).
            (KeyModifiers::CONTROL, KeyCode::Char('-')) => self.undo(),
            (KeyModifiers::CONTROL, KeyCode::Char('r')) => self.redo(),
            // Selection cut/copy/paste (Ctrl+X / Ctrl+C / Ctrl+V).
            (KeyModifiers::CONTROL, KeyCode::Char('x')) => {
                self.cut_selection();
            }
            (KeyModifiers::CONTROL, KeyCode::Char('c')) => {
                self.copy_selection();
            }
            (KeyModifiers::CONTROL, KeyCode::Char('v')) => {
                self.yank();
            }
            // Kill-ring yank (Ctrl+Y) / yank-pop (Alt+Y).
            (KeyModifiers::CONTROL, KeyCode::Char('y')) => self.yank(),
            (KeyModifiers::ALT, KeyCode::Char('y')) => self.yank_pop(),
            // Delete the word before the cursor, killing it (Alt+Backspace).
            (KeyModifiers::CONTROL, KeyCode::Char('w')) => self.kill_word_backward(),
            (KeyModifiers::ALT, KeyCode::Backspace) => self.kill_word_backward(),
            (KeyModifiers::ALT, KeyCode::Char('d')) | (KeyModifiers::ALT, KeyCode::Delete) => {
                self.kill_word_forward();
            }
            // Word navigation (Alt+Left / Alt+Right — pi's word-jump bindings).
            (KeyModifiers::ALT, KeyCode::Left) => self.cursor_word_left(),
            (KeyModifiers::ALT, KeyCode::Right) => self.cursor_word_right(),
            // Ctrl+J inserts a newline (pi tui.input.newLine alongside
            // Shift+Enter). Ctrl+D deletes the char forward (deleteCharForward).
            (KeyModifiers::CONTROL, KeyCode::Char('j')) => self.insert_no_undo(
                "
",
            ),
            (KeyModifiers::CONTROL, KeyCode::Char('d')) => self.delete(),
            // Editor page scroll (pi tui.editor.pageUp/pageDown — the
            // unmodified PageUp/Down are the alt-screen transcript scroll).
            (KeyModifiers::CONTROL, KeyCode::PageUp) => self.page_scroll(-1),
            (KeyModifiers::CONTROL, KeyCode::PageDown) => self.page_scroll(1),
            // Character jump mode (pi jumpForward/jumpBackward): Ctrl+] jumps
            // forward to the next typed char, Ctrl+Alt+] backward. NB: a
            // CONTROL|ALT pattern in a match arm is an or-pattern (matches
            // EITHER modifier), so the backward arm uses a guard.
            (_, KeyCode::Char(']')) if key.modifiers.contains(KeyModifiers::CONTROL) => {
                // Ctrl+Alt+] jumps backward, Ctrl+] forward. A bare CONTROL
                // arm wouldn't match the CONTROL|ALT combination.
                if key.modifiers.contains(KeyModifiers::ALT) {
                    *self.jump_mode.lock().unwrap() = Some(-1);
                } else {
                    *self.jump_mode.lock().unwrap() = Some(1);
                }
            }

            // Regular character input — unless a character jump is pending
            // (Ctrl+] / Ctrl+Alt+] consumed the key as the jump target).
            (KeyModifiers::NONE | KeyModifiers::SHIFT, KeyCode::Char(c)) => {
                let jump = self.jump_mode.lock().unwrap().take();
                match jump {
                    Some(dir) => self.jump_to_char(c, dir),
                    None => self.insert(&c.to_string()),
                }
            }

            _ => return false,
        }
        true
    }
}

impl Component for Editor {
    fn render(&self, width: usize) -> Vec<String> {
        let state = self.state.lock().map(|s| s.clone()).unwrap_or_default();
        let mut content_lines = Vec::new();
        // Visual row holding the caret, for the scrolling viewport.
        let mut cursor_line: Option<usize> = None;

        let pad = " ".repeat(self.options.padding_x);
        // An explicit `border_color` (bash mode) wins over the theme default.
        let border_color = self
            .border_color
            .lock()
            .ok()
            .and_then(|guard| *guard)
            .unwrap_or(crate::theme::theme().colors.border);
        // The prompt gutter (`set_prompt`) is dropped when it would leave fewer
        // than 2 columns for text (upstream `editor.ts`). `EditorStyle.prompt`
        // is unrelated and unread here.
        let prompt = self.prompt.lock().map(|p| p.clone()).unwrap_or_default();
        let prompt_width = crate::ansi::visible_width(&prompt);
        let prompt_width = if prompt_width > 0
            && prompt_width + 2 <= width.saturating_sub(self.options.padding_x * 2)
        {
            prompt_width
        } else {
            0
        };
        let prompt_gutter = if prompt_width > 0 {
            border_color.fg(&prompt)
        } else {
            String::new()
        };
        let continuation_gutter = " ".repeat(prompt_width);
        // Where text starts: left padding plus the prompt gutter.
        let text_left = self.options.padding_x + prompt_width;
        let content_width = width.saturating_sub(text_left).max(1);
        // Reserve the final cell for the cursor on every visual row so wrap
        // boundaries stay stable while navigating between logical lines.
        let visual_width = content_width.saturating_sub(1).max(1);
        if let Ok(mut stored_width) = self.visual_width.lock() {
            *stored_width = visual_width;
        }
        for (row_idx, line) in state.lines.iter().enumerate() {
            let ranges = wrap_line_ranges(line, visual_width);
            for (range_idx, (chunk_start, chunk_end)) in ranges.iter().copied().enumerate() {
                let chunk = &line[chunk_start..chunk_end];
                // Placeholder (dim) only on the single empty line at the cursor —
                // honors callers that set `placeholder`; production drops it.
                let content =
                    if chunk.is_empty() && row_idx == state.cursor_row && state.lines.len() == 1 {
                        if let Some(placeholder) = &self.options.placeholder {
                            format!("\x1b[2m{}\x1b[22m", placeholder)
                        } else {
                            String::new()
                        }
                    } else {
                        chunk.to_string()
                    };

                // Selection highlight: inverse-video the span on this row (byte
                // offsets are char boundaries — both anchor and caret always sit
                // on one).
                let mut content = content;
                if let Some((sc, ec)) = self.selection_span_on_row(&state, row_idx) {
                    let local_start = sc.max(chunk_start).min(chunk_end);
                    let local_end = ec.max(chunk_start).min(chunk_end);
                    if local_start < local_end {
                        let before = &line[chunk_start..local_start];
                        let sel = &line[local_start..local_end];
                        let after = &line[local_end..chunk_end];
                        content = format!("{before}\x1b[7m{sel}\x1b[27m{after}");
                    }
                }

                let visible_w = crate::ansi::visible_width(&content);
                // Pad the line out to the full width (left padding + content +
                // right padding) so the border box has a uniform interior width
                // (mirrors editor.ts:522-578).
                let right_pad =
                    " ".repeat(width.saturating_sub(text_left).saturating_sub(visible_w));
                let right_pad_cursor = if state.focused && row_idx == state.cursor_row {
                    right_pad
                        .get(..right_pad.len().saturating_sub(1))
                        .unwrap_or("")
                        .to_string()
                } else {
                    right_pad
                };

                let gutter = if content_lines.is_empty() {
                    &prompt_gutter
                } else {
                    &continuation_gutter
                };
                let mut rendered = format!("{pad}{gutter}{content}{right_pad_cursor}");

                // Cursor marker if this focused row holds the cursor. `cursor_col`
                // is a byte offset into `line`; the rendered prefix up to the caret
                // is `pad` (ASCII, `padding_x` bytes) + `line[..cursor_col]`.
                // Because `content` may carry ANSI escapes (placeholder branch),
                // and the prefix length is computed in *bytes* of the source line,
                // snap the insertion point to the nearest preceding char boundary
                // in `rendered` before byte-slicing — never slice mid-character.
                let is_last_chunk = range_idx + 1 == ranges.len();
                if state.focused
                    && row_idx == state.cursor_row
                    && state.cursor_col >= chunk_start
                    && (state.cursor_col < chunk_end
                        || (is_last_chunk && state.cursor_col == chunk_end))
                {
                    let cursor_col = crate::ansi::visible_width(
                        &line[chunk_start..state.cursor_col.min(chunk_end)],
                    );
                    rendered = insert_cursor_marker(&rendered, text_left + cursor_col);
                }
                if row_idx == state.cursor_row
                    && state.cursor_col >= chunk_start
                    && (state.cursor_col < chunk_end
                        || (is_last_chunk && state.cursor_col == chunk_end))
                {
                    cursor_line.get_or_insert(content_lines.len());
                }

                content_lines.push(rendered);
            }
        }

        // Guard against an empty `lines` vector (the invariant is `[""]`, but
        // be defensive): emit one blank interior row so the box still renders.
        if content_lines.is_empty() {
            content_lines.push(format!(
                "{pad}{prompt_gutter}{}",
                " ".repeat(width.saturating_sub(text_left))
            ));
        }

        // Full-width top + bottom border, colored via the theme border color
        // (mirrors editor.ts:494,530,587). The global `theme()` is read-only
        // after OnceLock init; `Color::fg` wraps the `─` run in the escape.
        // Check if we have a working state to display in the top border.
        //
        // The frame advances once per render, mirroring `Loader::render`: the
        // spinner is therefore driven by *repaints*, so it visibly speeds up
        // while the model is streaming (each token schedules a frame) and falls
        // back to the render tick's cadence when nothing is being produced. A
        // timer-only animation looked constant-speed and unrelated to output.
        let working_state = self.working.lock().ok().and_then(|mut guard| {
            guard.as_mut().map(|state| {
                let snapshot = state.clone();
                state.frame = state.frame.wrapping_add(1);
                snapshot
            })
        });
        let top_border = if let Some(ref working) = working_state {
            // Working indicator embedded in the top border: spinner + message +
            // elapsed time, with the remaining width filled by border glyphs.
            // The frame set is shared with `Loader` so the two never drift.
            let frames = crate::loader::SPINNER_FRAMES;
            let spinner_char = frames[working.frame % frames.len()];
            let colors = crate::theme::theme().colors;

            let elapsed = working.started_at.elapsed();
            let elapsed_str = if elapsed.as_secs() >= 60 {
                format!("{}m{}s", elapsed.as_secs() / 60, elapsed.as_secs() % 60)
            } else {
                format!("{}s", elapsed.as_secs())
            };

            let plain = format!(" {} {} {} ", spinner_char, working.message, elapsed_str);
            let indicator_width = crate::ansi::visible_width(&plain);

            if width >= indicator_width + 1 {
                let colored = format!(
                    " {} {} {} ",
                    colors.accent.fg(&spinner_char.to_string()),
                    colors.muted.fg(&working.message),
                    colors.muted.fg(&elapsed_str),
                );
                let right = border_color.fg(&"─".repeat(width - indicator_width - 1));
                format!("{}{}{}", border_color.fg("─"), colored, right)
            } else {
                border_color.fg(&"─".repeat(width))
            }
        } else {
            border_color.fg(&"─".repeat(width))
        };

        let bottom_border = {
            let horizontal = "─".repeat(width);
            border_color.fg(&horizontal)
        };

        // Scroll the text rows so the caret stays visible (ice `editor.ts`).
        let max_rows = self
            .max_visible_rows
            .lock()
            .ok()
            .and_then(|max| *max)
            .unwrap_or(usize::MAX);
        let (top_border, bottom_border) = if content_lines.len() > max_rows {
            let cursor_line = cursor_line.unwrap_or(0);
            let mut offset = state.scroll_offset;
            if cursor_line < offset {
                offset = cursor_line;
            } else if cursor_line >= offset + max_rows {
                offset = cursor_line + 1 - max_rows;
            }
            offset = offset.min(content_lines.len() - max_rows);
            if let Ok(mut live) = self.state.lock() {
                live.scroll_offset = offset;
            }
            let below = content_lines.len() - offset - max_rows;
            content_lines = content_lines.split_off(offset);
            content_lines.truncate(max_rows);
            let top = if offset > 0 && working_state.is_none() {
                border_color.fg(&scroll_border("↑", offset, width))
            } else {
                top_border
            };
            let bottom = if below > 0 {
                border_color.fg(&scroll_border("↓", below, width))
            } else {
                bottom_border
            };
            (top, bottom)
        } else {
            // The draft fits again: start from the top next time it grows.
            if state.scroll_offset != 0 {
                if let Ok(mut live) = self.state.lock() {
                    live.scroll_offset = 0;
                }
            }
            (top_border, bottom_border)
        };

        let mut lines = Vec::with_capacity(content_lines.len() + 2);
        lines.push(top_border);
        lines.extend(content_lines);
        lines.push(bottom_border);
        lines
    }

    fn invalidate(&self) {
        // Editor doesn't have external cache
    }

    fn handle_input(&self, _data: &str) -> bool {
        // Parse key event from crossterm format
        // This is a simplified version - in real usage, you'd receive KeyEvent directly
        false
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Focusable for Editor {
    fn set_focused(&self, focused: bool) {
        if let Ok(mut state) = self.state.lock() {
            state.focused = focused;
        }
    }

    fn is_focused(&self) -> bool {
        self.state.lock().map(|s| s.focused).unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capped_viewport_keeps_caret_at_a_wrap_boundary_visible() {
        let editor = Editor::simple();
        editor.set_focused(true);
        editor.set_terminal_rows(17);
        editor.set_text(&"abcdef".repeat(10));
        // With padding and the reserved cursor cell, width 8 wraps at 6.
        // Byte 30 belongs to the sixth row, not the end of the fifth row.
        editor.set_cursor(0, 30);
        let rows = editor.render(8);
        assert_eq!(rows.len(), 7);
        assert_eq!(
            rows.iter()
                .filter(|row| row.contains(CURSOR_MARKER))
                .count(),
            1
        );
        assert!(rows[5].contains(CURSOR_MARKER));
    }

    #[test]
    fn capped_unicode_draft_keeps_caret_after_resize_and_visual_movement() {
        use crossterm::event::KeyEvent;
        let editor = Editor::simple();
        editor.set_focused(true);
        editor.set_terminal_rows(20);
        let text = "界 e\u{301} 👩‍💻 wide words ".repeat(20);
        editor.set_text(&text);
        editor.set_cursor(0, text.len());
        for (width, height) in [(22, 20), (38, 40), (18, 12)] {
            editor.set_terminal_rows(height);
            let rows = editor.render(width);
            assert!(rows.len() <= (height * 3 / 10).max(5) + 2);
            assert_eq!(
                rows.iter()
                    .filter(|row| row.contains(CURSOR_MARKER))
                    .count(),
                1
            );
            editor.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
            let rows = editor.render(width);
            assert_eq!(
                rows.iter()
                    .filter(|row| row.contains(CURSOR_MARKER))
                    .count(),
                1
            );
            assert!(text.is_char_boundary(editor.cursor_position().1));
        }
    }

    /// A long draft used to render every row, pushing the transcript off the
    /// screen. The editor now shows at most 30% of the terminal (at least 5
    /// rows) and scrolls to keep the caret visible, with "N more" borders.
    #[test]
    fn long_draft_scrolls_inside_a_capped_viewport() {
        use crate::component::Component;
        let editor = Editor::simple();
        editor.set_terminal_rows(20); // 30% of 20 = 6 rows
        let text: Vec<String> = (0..12).map(|i| format!("line {i}")).collect();
        editor.set_text(&text.join("\n"));
        editor.set_cursor(11, 0);
        let rows = editor.render(40);
        let plain: Vec<String> = rows.iter().map(|r| crate::strip_ansi(r)).collect();
        assert_eq!(plain.len(), 6 + 2, "{plain:?}");
        assert!(plain[0].starts_with("─── ↑ 6 more "), "{plain:?}");
        assert!(plain[6].contains("line 11"), "{plain:?}");
        assert!(plain[7].chars().all(|c| c == '─'), "{plain:?}");

        // Moving the caret to the top scrolls back up.
        editor.set_cursor(0, 0);
        let plain: Vec<String> = editor
            .render(40)
            .iter()
            .map(|r| crate::strip_ansi(r))
            .collect();
        assert!(plain[1].contains("line 0"), "{plain:?}");
        assert!(plain[7].starts_with("─── ↓ 6 more "), "{plain:?}");

        // Small terminals still get 5 rows; no cap shows everything.
        editor.set_terminal_rows(8);
        assert_eq!(editor.render(40).len(), 5 + 2);
        let uncapped = Editor::simple();
        uncapped.set_text(&text.join("\n"));
        assert_eq!(uncapped.render(40).len(), 12 + 2);
    }

    #[test]
    fn working_spinner_advances_per_render_not_per_timer() {
        use crate::component::Component;
        let editor = Editor::simple();
        editor.set_working(Some(WorkingState {
            frame: 0,
            started_at: std::time::Instant::now(),
            message: "Working…".to_string(),
        }));

        // The indicator lives in the top border and reports the message.
        let first = crate::strip_ansi(&editor.render(60)[0]);
        assert!(first.contains("Working"), "top border: {first}");

        // Each render steps the frame, so repaints driven by streamed output
        // visibly advance the spinner; no timer is required for that.
        let frame_after_first = editor.working().expect("working state").frame;
        assert_eq!(frame_after_first, 1, "render must advance the frame once");
        let _ = editor.render(60);
        assert_eq!(editor.working().expect("working state").frame, 2);

        // Two renders of the same state differ on screen (spinner char moves).
        let a = crate::strip_ansi(&editor.render(60)[0]);
        let b = crate::strip_ansi(&editor.render(60)[0]);
        let spinner_of = |line: &str| line.chars().find(|c| "⠋⠘⠹⠸⠼⠴⠦⠧⠇⠏".contains(*c));
        assert_ne!(
            spinner_of(&a),
            spinner_of(&b),
            "consecutive renders must show different spinner frames: {a:?} vs {b:?}"
        );

        // Idle editors have no indicator.
        editor.set_working(None);
        let idle = crate::strip_ansi(&editor.render(60)[0]);
        assert!(!idle.contains("Working"), "idle border: {idle}");
    }

    #[test]
    fn working_spinner_uses_native_pi_frames_and_cadence() {
        // The editor used to hand-copy the frame list and drifted (`⠘` where
        // upstream has `⠙`), so the two indicators animated differently. Both
        // now share `SPINNER_FRAMES`.
        assert_eq!(crate::loader::SPINNER_FRAMES.len(), 10);
        assert_eq!(
            crate::loader::SPINNER_FRAMES.iter().collect::<String>(),
            "⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏"
        );
        // 10 frames at 80ms = the 800ms full cycle upstream animates at.
        assert_eq!(crate::loader::SPINNER_FRAME_MS, 80);

        // The rendered indicator steps through exactly that list, in order.
        use crate::component::Component;
        let editor = Editor::simple();
        editor.set_working(Some(WorkingState {
            frame: 0,
            started_at: std::time::Instant::now(),
            message: "Working…".to_string(),
        }));
        let spinner_of = |line: &str| {
            let plain = crate::strip_ansi(line);
            crate::loader::SPINNER_FRAMES
                .iter()
                .copied()
                .find(|c| plain.contains(*c))
        };
        let mut seen = Vec::new();
        for _ in 0..10 {
            seen.push(spinner_of(&editor.render(60)[0]).expect("a spinner glyph"));
        }
        // The snapshot is taken before the advance, so the first paint shows
        // frame 0 — exactly upstream, whose `start()` paints frame 0 and then
        // steps on each interval. Ten paints therefore cover the whole cycle in
        // order and wrap back to the start.
        let expected: Vec<char> = crate::loader::SPINNER_FRAMES.to_vec();
        assert_eq!(seen, expected);
        assert_eq!(
            editor.working().expect("working").frame % crate::loader::SPINNER_FRAMES.len(),
            0,
            "ten paints must land back on the first frame"
        );
    }

    #[test]
    fn idle_cadence_matches_native_pi() {
        // One frame per `SPINNER_FRAME_MS` of wall clock when nothing else is
        // repainting. This is the property the tick task relies on: it only
        // requests a repaint, and the repaint advances exactly one frame. The
        // tick used to *also* advance the frame, which made the idle spinner
        // run at 40ms/frame — twice upstream's rate.
        use crate::component::Component;
        assert_eq!(crate::loader::SPINNER_FRAME_MS, 80);
        assert_eq!(
            crate::loader::SPINNER_FRAME_MS * crate::loader::SPINNER_FRAMES.len() as u64,
            800,
            "a full cycle should take 800ms, like upstream"
        );

        let editor = Editor::simple();
        editor.set_working(Some(WorkingState {
            frame: 0,
            started_at: std::time::Instant::now(),
            message: "Working…".to_string(),
        }));
        // One repaint == one frame, i.e. one tick advances the spinner once.
        let before = editor.working().expect("working").frame;
        let _ = editor.render(60);
        assert_eq!(editor.working().expect("working").frame, before + 1);
    }

    #[test]
    fn explicit_border_color_overrides_the_theme_border() {
        use crate::component::Component;
        let editor = Editor::simple();
        editor.set_focused(true);
        editor.insert("echo hi");

        let default_border = crate::theme::theme().colors.border.fg(&"─".repeat(20));
        assert!(editor.render(20).contains(&default_border));
        assert_eq!(editor.border_color(), None);

        // Bash mode sets an explicit accent; the border line changes and the
        // theme default is gone.
        editor.set_border_color(Some(crate::theme::theme().colors.bash_mode));
        let bash_border = crate::theme::theme().colors.bash_mode.fg(&"─".repeat(20));
        assert_eq!(
            editor.border_color(),
            Some(crate::theme::theme().colors.bash_mode)
        );
        let rendered = editor.render(20);
        assert!(rendered.contains(&bash_border), "{rendered:?}");
        assert!(!rendered.contains(&default_border));

        // Clearing restores the theme default (what `updateEditorBorderColor`
        // does once the draft no longer starts with `!`).
        editor.set_border_color(None);
        assert!(editor.render(20).contains(&default_border));
    }

    #[test]
    fn clear_and_set_text_notify_change_observers() {
        // Upstream's `setTextInternal` fires onChange for programmatic edits;
        // the bash-mode border and autocomplete rely on it to drop stale state.
        use std::sync::atomic::{AtomicUsize, Ordering};
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_cb = calls.clone();
        let editor = Editor::simple();
        editor.on_change(Arc::new(move |_text: &str| {
            calls_cb.fetch_add(1, Ordering::SeqCst);
        }));

        editor.set_text("!ls");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        editor.clear();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn snap_boundary_keeps_char_boundaries_including_end() {
        // The regression: a caret placed exactly at the END of the text (what
        // autocomplete accept does — `set_cursor(0, text.len())`) used to snap
        // one char EARLY, rendering "/mode|l" instead of "/model|".
        assert_eq!(snap_boundary("/mo", 3), 3);
        assert_eq!(snap_boundary("/model", 6), 6);
        assert_eq!(snap_boundary("hello", 5), 5);
        // Interior boundaries are kept as-is.
        assert_eq!(snap_boundary("/model", 0), 0);
        assert_eq!(snap_boundary("/model", 2), 2);
        // Out-of-range clamps to the end (a valid boundary).
        assert_eq!(snap_boundary("/mo", 99), 3);
    }

    #[test]
    fn snap_boundary_mid_multibyte_snaps_down_to_char_start() {
        // "你" occupies bytes 0..3: a position inside it must snap to 0.
        assert_eq!(snap_boundary("你", 1), 0);
        assert_eq!(snap_boundary("你", 2), 0);
        // Position after it (end of string) stays 3.
        assert_eq!(snap_boundary("你", 3), 3);
        // Mixed: "a你b" — 你 is bytes 1..4; end of string is 5.
        assert_eq!(snap_boundary("a你b", 5), 5);
        assert_eq!(snap_boundary("a你b", 2), 1);
    }

    #[test]
    fn left_arrow_keeps_text_in_render() {
        use crate::component::Component;
        use crossterm::event::{KeyCode, KeyModifiers};
        let mk = |m, c| crossterm::event::KeyEvent::new(c, m);
        let editor = Editor::simple();
        editor.set_focused(true);
        editor.insert("hello world");
        editor.handle_key(mk(KeyModifiers::NONE, KeyCode::Left));
        let rendered = editor.render(40).join(
            "
",
        );
        assert!(
            rendered.contains("hello"),
            "text must survive Left, got: {rendered:?}"
        );
    }
    #[test]
    fn test_char_jump_and_page_scroll() {
        use crossterm::event::{KeyCode, KeyModifiers};
        let mk = |m, c| crossterm::event::KeyEvent::new(c, m);
        let editor = Editor::simple();

        // Character jump: from the end, Ctrl+Alt+] then 'x' jumps backward to
        // the previous 'x' ("alpha xray xray" — the second "xray" starts at 11).
        editor.insert("alpha xray xray");
        editor.handle_key(mk(KeyModifiers::CONTROL | KeyModifiers::ALT, KeyCode::End)); // caret at end
        editor.handle_key(mk(
            KeyModifiers::CONTROL | KeyModifiers::ALT,
            KeyCode::Char(']'),
        ));
        // The next printable char is consumed as the jump target.
        editor.handle_key(mk(KeyModifiers::NONE, KeyCode::Char('x')));
        assert_eq!(editor.cursor_position(), (0, 11));

        // Page scroll on a multi-line buffer clamps to the last row.
        editor.set_text(
            "l0
l1
l2
l3
l4
l5
l6
l7
l8
l9
l10
l11
l12",
        );
        editor.set_cursor(0, 2);
        editor.handle_key(mk(KeyModifiers::CONTROL, KeyCode::PageDown));
        let (row, _) = editor.cursor_position();
        assert!(row >= 10, "page-down moves ~10 rows, got {row}");
        editor.handle_key(mk(KeyModifiers::CONTROL, KeyCode::PageUp));
        let (row2, _) = editor.cursor_position();
        assert!(row2 <= 2, "page-up returns near the start, got {row2}");
    }
    #[test]
    fn test_word_ops_and_new_keys() {
        use crossterm::event::{KeyCode, KeyModifiers};
        let mk = |m, c| crossterm::event::KeyEvent::new(c, m);
        let editor = Editor::simple();
        editor.insert("alpha beta gamma");

        // Ctrl+W kills the word before the cursor.
        editor.handle_key(mk(KeyModifiers::CONTROL, KeyCode::End));
        editor.handle_key(mk(KeyModifiers::CONTROL, KeyCode::Char('w')));
        assert_eq!(editor.get_text(), "alpha beta ");

        // Alt+Left jumps back a word; Alt+D kills the word after.
        editor.handle_key(mk(KeyModifiers::ALT, KeyCode::Left));
        assert_eq!(editor.cursor_position(), (0, 6));
        editor.handle_key(mk(KeyModifiers::ALT, KeyCode::Char('d')));
        assert_eq!(editor.get_text(), "alpha  ");

        // Ctrl+J inserts a newline.
        editor.set_text("a");
        editor.set_cursor(0, 1);
        editor.handle_key(mk(KeyModifiers::CONTROL, KeyCode::Char('j')));
        assert_eq!(
            editor.get_text(),
            "a
"
        );

        // Ctrl+D deletes the char forward (deleteCharForward).
        editor.set_text("abc");
        editor.set_cursor(0, 1);
        editor.handle_key(mk(KeyModifiers::CONTROL, KeyCode::Char('d')));
        assert_eq!(editor.get_text(), "ac");
    }

    #[test]
    fn word_ops_on_multibyte_text_use_byte_cursor() {
        // Regression: word helpers return char indices but `cursor_col` is a
        // byte offset; mixing them indexed past the char vector or drained
        // inside a UTF-8 sequence. "你好 世界" is 13 bytes, 5 chars.
        use crossterm::event::{KeyCode, KeyModifiers};
        let mk = |m, c| crossterm::event::KeyEvent::new(c, m);
        let editor = Editor::simple();

        editor.set_text("你好 世界");
        editor.set_cursor(0, 13);
        editor.handle_key(mk(KeyModifiers::ALT, KeyCode::Left));
        assert_eq!(editor.cursor_position(), (0, 7));
        editor.handle_key(mk(KeyModifiers::ALT, KeyCode::Left));
        assert_eq!(editor.cursor_position(), (0, 0));
        editor.handle_key(mk(KeyModifiers::ALT, KeyCode::Right));
        assert_eq!(editor.cursor_position(), (0, 7));

        editor.handle_key(mk(KeyModifiers::ALT, KeyCode::Char('d')));
        assert_eq!(editor.get_text(), "你好 ");
        assert_eq!(editor.cursor_position(), (0, 7));

        editor.set_text("你好 世界");
        editor.set_cursor(0, 13);
        editor.handle_key(mk(KeyModifiers::CONTROL, KeyCode::Char('w')));
        assert_eq!(editor.get_text(), "你好 ");
        assert_eq!(editor.cursor_position(), (0, 7));

        // Kill forward from the start removes the whole first word.
        editor.set_text("你好 世界");
        editor.set_cursor(0, 0);
        editor.handle_key(mk(KeyModifiers::ALT, KeyCode::Char('d')));
        assert_eq!(editor.get_text(), " 世界");
    }

    #[test]
    fn char_jump_steps_over_the_char_at_the_caret_not_the_target() {
        // Regression: the search offset used the *target's* UTF-8 length, so
        // a multibyte line sliced inside a char (panic) and a backward jump
        // skipped a match directly before the caret. Mirrors ice
        // `jumpToChar`: forward starts after the char under the caret,
        // backward finds a match that starts before the caret.
        use crossterm::event::{KeyCode, KeyModifiers};
        let mk = |m, c| crossterm::event::KeyEvent::new(c, m);
        let jump = |editor: &Editor, backward: bool, c: char| {
            let mods = if backward {
                KeyModifiers::CONTROL | KeyModifiers::ALT
            } else {
                KeyModifiers::CONTROL
            };
            editor.handle_key(mk(mods, KeyCode::Char(']')));
            editor.handle_key(mk(KeyModifiers::NONE, KeyCode::Char(c)));
        };
        let editor = Editor::simple();

        editor.set_text("你x");
        editor.set_cursor(0, 0);
        jump(&editor, false, 'x');
        assert_eq!(editor.cursor_position(), (0, 3));

        editor.set_text("x世");
        editor.set_cursor(0, 0);
        jump(&editor, false, '世');
        assert_eq!(editor.cursor_position(), (0, 1));

        editor.set_text("axb");
        editor.set_cursor(0, 2);
        jump(&editor, true, 'x');
        assert_eq!(editor.cursor_position(), (0, 1));

        editor.set_text("世x界");
        editor.set_cursor(0, 7);
        jump(&editor, true, '世');
        assert_eq!(editor.cursor_position(), (0, 0));
    }

    #[test]
    fn test_selection_cut_copy_paste() {
        let editor = Editor::simple();
        editor.insert("hello world");
        // Caret to the start, then Shift+Right ×5 selects "hello".
        editor.handle_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Home,
            crossterm::event::KeyModifiers::NONE,
        ));
        let shift_right = || {
            editor.handle_key(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Right,
                crossterm::event::KeyModifiers::SHIFT,
            ))
        };
        for _ in 0..5 {
            shift_right();
        }
        assert!(editor.has_selection());
        assert_eq!(editor.selected_text().as_deref(), Some("hello"));
        // Ctrl+C copies (selection intact), then plain Right clears it.
        editor.handle_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('c'),
            crossterm::event::KeyModifiers::CONTROL,
        ));
        assert_eq!(editor.get_text(), "hello world");
        editor.handle_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Right,
            crossterm::event::KeyModifiers::NONE,
        ));
        assert!(!editor.has_selection(), "plain movement clears selection");
        // Move to the end and yank the copied text.
        editor.handle_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::End,
            crossterm::event::KeyModifiers::NONE,
        ));
        editor.handle_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('v'),
            crossterm::event::KeyModifiers::CONTROL,
        ));
        assert_eq!(editor.get_text(), "hello worldhello");
    }

    #[test]
    fn test_selection_replace_and_cut() {
        let editor = Editor::simple();
        editor.insert("abcdef");
        // Caret to the start, then Shift+Right ×4 selects "abcd".
        editor.handle_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Home,
            crossterm::event::KeyModifiers::NONE,
        ));
        for _ in 0..4 {
            editor.handle_key(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Right,
                crossterm::event::KeyModifiers::SHIFT,
            ));
        }
        // Backspace deletes the selection ("abcd" → "ef").
        editor.handle_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Backspace,
            crossterm::event::KeyModifiers::NONE,
        ));
        assert_eq!(editor.get_text(), "ef", "backspace deletes the selection");
        // Undo restores it.
        editor.undo();
        assert_eq!(editor.get_text(), "abcdef");
        // Cut: return to the start, select "ab", Ctrl+X removes + kills it.
        editor.handle_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Home,
            crossterm::event::KeyModifiers::NONE,
        ));
        for _ in 0..2 {
            editor.handle_key(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Right,
                crossterm::event::KeyModifiers::SHIFT,
            ));
        }
        editor.handle_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('x'),
            crossterm::event::KeyModifiers::CONTROL,
        ));
        assert_eq!(editor.get_text(), "cdef");
        // Type over a selection replaces it.
        editor.handle_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Right,
            crossterm::event::KeyModifiers::SHIFT,
        ));
        editor.insert("Z");
        assert_eq!(editor.get_text(), "Zdef", "typing replaces the selection");
    }
    #[test]
    fn test_undo_redo_roundtrip() {
        let editor = Editor::simple();
        editor.insert("hello");
        editor.insert(" world");
        // Consecutive inserts coalesce: one undo restores the empty state.
        assert_eq!(editor.get_text(), "hello world");
        editor.undo();
        assert_eq!(editor.get_text(), "", "coalesced inserts undo as one step");
        // Redo restores everything.
        editor.redo();
        assert_eq!(editor.get_text(), "hello world");
        // A fresh edit after undo clears redo.
        editor.undo();
        editor.insert("hi");
        editor.redo();
        assert_eq!(editor.get_text(), "hi", "new edit clears the redo stack");
    }

    #[test]
    fn test_kill_ring_yank() {
        let editor = Editor::simple();
        editor.insert("alpha beta gamma");
        // Ctrl+K kills to end of line.
        editor.set_cursor(0, 6); // after "alpha "
        let key = |m, c| crossterm::event::KeyEvent::new(c, m);
        editor.handle_key(key(
            crossterm::event::KeyModifiers::CONTROL,
            crossterm::event::KeyCode::Char('k'),
        ));
        assert_eq!(editor.get_text(), "alpha ");
        // Ctrl+Y yanks it back.
        editor.handle_key(key(
            crossterm::event::KeyModifiers::CONTROL,
            crossterm::event::KeyCode::Char('y'),
        ));
        assert_eq!(editor.get_text(), "alpha beta gamma");
    }

    #[test]
    fn test_undo_after_kill() {
        let editor = Editor::simple();
        editor.insert("hello world");
        editor.set_cursor(0, 5);
        editor.handle_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('k'),
            crossterm::event::KeyModifiers::CONTROL,
        ));
        assert_eq!(editor.get_text(), "hello");
        editor.undo();
        assert_eq!(editor.get_text(), "hello world", "kill is undoable");
    }

    #[test]
    fn shift_enter_encodings_insert_newline_without_submitting() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        for code in [KeyCode::Enter, KeyCode::Char('\r'), KeyCode::Char('\n')] {
            let editor = Editor::simple();
            editor.on_submit(Arc::new(|_| panic!("Shift+Enter must not submit")));
            editor.insert("draft");
            editor.handle_key(KeyEvent::new(code, KeyModifiers::SHIFT));
            assert_eq!(editor.get_text(), "draft\n");
        }
    }

    #[test]
    fn test_submit_clears_undo() {
        let editor = Editor::simple();
        editor.insert("committed");
        editor.handle_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        ));
        editor.undo();
        assert_eq!(
            editor.get_text(),
            "",
            "submit clears undo so committed text stays committed"
        );
    }

    /// Regression: a remote/mobile client (SSH, soft keyboard) that encodes
    /// its Enter key as LF (0x0A) or CR (0x0D) sends it as a bare character
    /// event in raw mode. Those used to fall through to the generic char arm
    /// and insert a literal newline — "回车变成了换行，无法发送内容". They must
    /// submit just like `KeyCode::Enter`.
    #[test]
    fn test_remote_lf_cr_enter_submits() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        for code in [
            crossterm::event::KeyCode::Char('\n'),
            crossterm::event::KeyCode::Char('\r'),
        ] {
            let editor = Editor::simple();
            let submits = Arc::new(AtomicUsize::new(0));
            let submits_for_cb = submits.clone();
            editor.on_submit(Arc::new(move |_text: &str| {
                submits_for_cb.fetch_add(1, Ordering::SeqCst);
            }));
            editor.insert("hello");
            editor.handle_key(crossterm::event::KeyEvent::new(
                code,
                crossterm::event::KeyModifiers::NONE,
            ));
            assert_eq!(submits.load(Ordering::SeqCst), 1, "{code:?} must submit");
            assert_eq!(editor.get_text(), "", "editor cleared after submit");
        }
    }
    #[test]
    fn test_editor_text() {
        let editor = Editor::simple();
        editor.set_text("Hello");
        assert_eq!(editor.get_text(), "Hello");
    }

    #[test]
    fn test_editor_insert() {
        let editor = Editor::simple();
        editor.insert("Hello");
        assert_eq!(editor.get_text(), "Hello");
        editor.insert(" World");
        assert_eq!(editor.get_text(), "Hello World");
    }

    #[test]
    fn test_editor_clear() {
        let editor = Editor::simple();
        editor.set_text("Test");
        editor.clear();
        assert_eq!(editor.get_text(), "");
    }

    #[test]
    fn prompt_gutter_leads_the_first_line_and_indents_wrapped_rows() {
        let editor = Editor::new(
            EditorOptions {
                padding_x: 1,
                ..Default::default()
            },
            EditorStyle::default(),
            Arc::new(Keybindings::new()),
        );
        editor.set_prompt("❯ ");
        editor.set_focused(true);
        editor.set_text("abcdefghijklmnop");
        let lines = editor.render(12);
        let rows: Vec<String> = lines[1..lines.len() - 1]
            .iter()
            .map(|line| crate::ansi::strip_ansi(&line.replace(CURSOR_MARKER, "")))
            .collect();
        // 12 columns - 1 padding - 2 prompt - 1 caret cell = 8 per row.
        assert_eq!(rows[0].trim_end(), " ❯ abcdefgh");
        assert_eq!(rows[1].trim_end(), "   ijklmnop");
        for line in &lines {
            assert!(crate::ansi::visible_width(&line.replace(CURSOR_MARKER, "")) <= 12);
        }
        // The caret column counts the gutter: at the start of the text it sits
        // right after `❯ `, not on it.
        editor.set_text("ab");
        editor.handle_key(crossterm::event::KeyEvent::new(
            KeyCode::Home,
            KeyModifiers::NONE,
        ));
        let first = editor.render(12)[1].clone();
        let before = &first[..first.find(CURSOR_MARKER).expect("cursor marker")];
        assert_eq!(crate::ansi::strip_ansi(before), " ❯ ");

        // Too narrow for the prompt plus two text columns: it is dropped.
        editor.set_text("");
        let narrow = crate::ansi::strip_ansi(&editor.render(5)[1].replace(CURSOR_MARKER, ""));
        assert!(!narrow.contains('❯'), "{narrow:?}");
    }

    #[test]
    fn test_editor_render_has_borders() {
        // The editor renders a native-pi bordered box: a full-width `─` line
        // above and below the content, with no `> ` prompt prefix on the text
        // lines (mirrors TS editor.ts:525-588).
        let editor = Editor::new(
            EditorOptions {
                padding_x: 1,
                ..Default::default()
            },
            EditorStyle::default(),
            Arc::new(Keybindings::new()),
        );
        editor.set_text("hi");
        let lines = editor.render(20);
        assert_eq!(lines.len(), 3, "expected [border, content, border]");
        // Both borders are a 20-wide `─` run (visible width, ANSI stripped).
        let top = crate::ansi::strip_ansi(&lines[0]);
        let bottom = crate::ansi::strip_ansi(lines.last().unwrap());
        assert_eq!(
            top.chars().filter(|c| *c == '─').count(),
            20,
            "top border not full width: {top:?}"
        );
        assert_eq!(
            bottom.chars().filter(|c| *c == '─').count(),
            20,
            "bottom border not full width: {bottom:?}"
        );
        // The content line carries the text with no `> ` prompt.
        let content = crate::ansi::strip_ansi(&lines[1]);
        assert!(
            content.contains("hi"),
            "content line missing text: {content:?}"
        );
        assert!(
            !content.contains("> "),
            "content line should not have a prompt prefix: {content:?}"
        );
    }

    #[test]
    fn test_editor_multibyte_no_panic() {
        // Inserting a multibyte char (读 = 3 bytes) and then a second char
        // used to panic: `cursor_col += 1` left the caret at byte 1, inside
        // '读', so the next `String::insert(1, …)` hit a char boundary. This
        // exercises the byte-cursor fix across insert + render + a second
        // keystroke.
        let editor = Editor::simple();
        editor.set_focused(true);
        editor.insert("读");
        editor.insert("a");
        editor.insert("书");
        assert_eq!(editor.get_text(), "读a书");
        // cursor_col should be at the end (byte offset 7: 读=3 + a=1 + 书=3).
        assert_eq!(editor.cursor_position(), (0, 7));
        // Render must not panic when placing the cursor marker on the row.
        let lines = editor.render(20);
        assert_eq!(
            lines.len(),
            3,
            "bordered box keeps [border, content, border]"
        );

        // Backspace / left/right must also stay on boundaries.
        editor.backspace(); // remove 书
        assert_eq!(editor.get_text(), "读a");
        editor.cursor_left(); // left of 'a'
        assert_eq!(editor.cursor_position(), (0, 3));
        editor.insert("b"); // insert between 读 and a
        assert_eq!(editor.get_text(), "读ba");
    }

    #[test]
    fn test_editor_wraps_long_draft_inside_width() {
        let editor = Editor::new(
            EditorOptions {
                padding_x: 1,
                ..Default::default()
            },
            EditorStyle::default(),
            Arc::new(Keybindings::new()),
        );
        editor.set_focused(true);
        editor.set_text("12345678901234567890");
        let lines = editor.render(10);
        assert_eq!(lines.len(), 5, "two wrapped content rows plus borders");
        for line in &lines {
            assert!(
                crate::ansi::visible_width(line) <= 10,
                "line escaped editor width: {line:?}"
            );
        }
        let rendered = crate::ansi::strip_ansi(&lines.join("\n"));
        assert!(rendered.contains("1234567"));
        assert!(rendered.contains("90123456"));
    }

    #[test]
    fn word_wrap_prefers_word_boundaries_and_keeps_the_separator_space() {
        let line = "hello world test";
        let ranges = wrap_line_ranges(line, 11);
        let chunks: Vec<&str> = ranges
            .iter()
            .map(|(start, end)| &line[*start..*end])
            .collect();
        assert_eq!(chunks, ["hello ", "world test"]);
    }

    #[test]
    fn word_wrap_breaks_long_words_only_when_required() {
        let line = "abcde fghijklmnop q";
        let ranges = wrap_line_ranges(line, 8);
        let chunks: Vec<&str> = ranges
            .iter()
            .map(|(start, end)| &line[*start..*end])
            .collect();
        assert_eq!(chunks, ["abcde ", "fghijklm", "nop q"]);
        assert_eq!(chunks.concat(), line);
    }

    #[test]
    fn word_wrap_allows_breaks_between_cjk_graphemes() {
        let line = "你好世界和平";
        let ranges = wrap_line_ranges(line, 4);
        let chunks: Vec<&str> = ranges
            .iter()
            .map(|(start, end)| &line[*start..*end])
            .collect();
        assert_eq!(chunks, ["你好", "世界", "和平"]);
    }

    #[test]
    fn up_and_down_move_between_visual_rows_of_a_wrapped_line() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let editor = Editor::new(
            EditorOptions {
                padding_x: 1,
                ..Default::default()
            },
            EditorStyle::default(),
            Arc::new(Keybindings::new()),
        );
        editor.set_focused(true);
        editor.set_text("abcdefghijklmnopqrst");
        editor.render(10); // 8 text cells per visual line; active row reserves 1.
        editor.set_cursor(0, 13);

        editor.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(editor.cursor_position(), (0, 5));
        editor.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(editor.cursor_position(), (0, 13));
    }

    #[test]
    fn display_cell_click_uses_prompt_wrapping_and_grapheme_layout() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let editor = Editor::new(
            EditorOptions {
                padding_x: 1,
                ..Default::default()
            },
            EditorStyle::default(),
            Arc::new(Keybindings::new()),
        );
        editor.set_prompt("❯ ");
        editor.set_text("abcdefghijklmnop");
        editor.render(10); // prompt + padding leave six cells per visual row.
        editor.handle_key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
        editor.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::SHIFT));
        assert!(editor.has_selection());

        assert!(editor.set_cursor_at_visual_position(1, 1 + 2 + 2, 10));
        assert_eq!(editor.cursor_position(), (0, 8));
        assert!(!editor.has_selection());

        editor.set_text("e\u{301}👩‍💻x");
        editor.render(20);
        // The second cell of the joined emoji maps before the whole grapheme.
        assert!(editor.set_cursor_at_visual_position(0, 1 + 2 + 2, 20));
        assert_eq!(editor.cursor_position(), (0, 3));
        // The cell after the joined emoji maps to its byte end.
        assert!(editor.set_cursor_at_visual_position(0, 1 + 2 + 3, 20));
        assert_eq!(editor.cursor_position(), (0, 14));
    }

    #[test]
    fn vertical_visual_column_is_sticky_across_short_logical_lines() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let editor = Editor::new(
            EditorOptions {
                padding_x: 1,
                ..Default::default()
            },
            EditorStyle::default(),
            Arc::new(Keybindings::new()),
        );
        editor.set_focused(true);
        editor.set_text("abcdefghij\nxyz\nabcdefghij");
        editor.render(12);
        editor.set_cursor(0, 8);

        editor.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(editor.cursor_position(), (1, 3));
        editor.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(editor.cursor_position(), (2, 8));
    }

    #[test]
    fn vertical_visual_movement_uses_grapheme_cell_widths() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let editor = Editor::new(
            EditorOptions {
                padding_x: 1,
                ..Default::default()
            },
            EditorStyle::default(),
            Arc::new(Keybindings::new()),
        );
        editor.set_focused(true);
        editor.set_text("ab界defgh\n12345678");
        editor.render(12);
        editor.set_cursor(0, 5); // after `界`, at display column 4

        editor.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(editor.cursor_position(), (1, 4));
    }

    #[test]
    fn horizontal_movement_resets_visual_column_preference() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let editor = Editor::new(
            EditorOptions {
                padding_x: 1,
                ..Default::default()
            },
            EditorStyle::default(),
            Arc::new(Keybindings::new()),
        );
        editor.set_focused(true);
        editor.set_text("abcdefghij\nxyz\nabcdefghij");
        editor.render(12);
        editor.set_cursor(0, 8);
        editor.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        editor.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        editor.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(editor.cursor_position(), (2, 2));
    }

    #[test]
    fn test_editor_preserves_trailing_newline() {
        let editor = Editor::simple();
        editor.set_text("first\n");
        assert_eq!(editor.get_text(), "first\n");
        assert_eq!(editor.cursor_position(), (0, 0));
    }

    #[test]
    fn test_autocomplete_cursor_origin() {
        // Origin of the autocomplete panic (autocomplete.rs:279, before the
        // fix): the editor handed back a byte `cursor_col` for multibyte
        // text, and the slash-command provider sliced `&input[..cursor]`
        // mid-character. A multibyte editor line must still round-trip
        // through `get_suggestions` without panicking.
        use crate::autocomplete::AutocompleteManager;

        let editor = Editor::simple();
        editor.set_text("读");
        editor.set_cursor(0, 3); // byte end of line
        let text = editor.get_text();
        let (_r, col) = editor.cursor_position();
        let cursor = col.min(text.len());
        let mgr = AutocompleteManager::new();

        // No slash prefix → None; must not panic on the multibyte slice.
        assert!(mgr.get_suggestions(&text, cursor).is_none());
        // And the slash path with a char-count overshoot also must not panic.
        editor.set_text("/读");
        editor.set_cursor(0, 4);
        let text2 = editor.get_text();
        let mgr2 = AutocompleteManager::new();
        let _ = mgr2.get_suggestions(&text2, 9); // overshoot past end
    }

    #[test]
    fn insert_normalizes_crlf_and_lone_cr_to_newlines() {
        // Pasted Windows text (and the Unix `Event::Paste` payload, which keeps
        // CRLF) used to insert literal `\r` bytes into the line buffer: they
        // render as garbage and desync `cursor_col` from the string length.
        let editor = Editor::simple();
        editor.insert("line1\r\nline2\rline3");
        assert_eq!(editor.get_text(), "line1\nline2\nline3");

        // The buffer is 3 lines and the cursor sits at the end of the last.
        let (row, col) = editor.cursor_position();
        assert_eq!((row, col), (2, "line3".len()));

        // Editing after a normalized paste stays sound (no mid-char split).
        editor.insert("!");
        assert_eq!(editor.get_text(), "line1\nline2\nline3!");
    }

    #[test]
    fn insert_of_a_lone_crlf_makes_exactly_one_newline() {
        let editor = Editor::simple();
        editor.insert("a\r\nb");
        assert_eq!(editor.get_text().matches('\n').count(), 1);
        assert_eq!(editor.get_text(), "a\nb");
    }

    // ---- Folded pastes (upstream `handlePaste` / `expandPasteMarkers`) ----

    #[test]
    fn short_paste_is_inserted_verbatim() {
        let editor = Editor::simple();
        editor.handle_paste("hello\nworld\nthird");
        assert_eq!(editor.get_text(), "hello\nworld\nthird");
        assert_eq!(editor.get_expanded_text(), "hello\nworld\nthird");
    }

    #[test]
    fn many_line_paste_folds_to_a_marker() {
        let editor = Editor::simple();
        let payload = (1..=12)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        editor.handle_paste(&payload);

        // The document shows only the marker; the body is stashed.
        assert_eq!(editor.get_text(), "[paste #1 +12 lines]");
        // Submitting reveals the real content.
        assert_eq!(editor.get_expanded_text(), payload);
    }

    #[test]
    fn long_single_line_paste_folds_by_char_count() {
        let editor = Editor::simple();
        let payload = "x".repeat(2000);
        editor.handle_paste(&payload);
        assert_eq!(editor.get_text(), "[paste #1 2000 chars]");
        assert_eq!(editor.get_expanded_text(), payload);
    }

    #[test]
    fn pasted_crlf_and_tabs_are_normalized_before_folding() {
        let editor = Editor::simple();
        let payload = (1..=12)
            .map(|i| format!("a\tb{i}"))
            .collect::<Vec<_>>()
            .join("\r\n");
        editor.handle_paste(&payload);
        let expanded = editor.get_expanded_text();
        assert!(!expanded.contains('\r'), "CR must be normalized away");
        assert!(!expanded.contains('\t'), "tab must expand to spaces");
        assert!(expanded.contains("a    b1"), "tab becomes four spaces");
        assert_eq!(expanded.split('\n').count(), 12);
    }

    #[test]
    fn two_folded_pastes_get_distinct_ids_and_both_expand() {
        let editor = Editor::simple();
        let first = "1\n".repeat(11);
        let second = "2\n".repeat(11);
        editor.handle_paste(&first);
        editor.handle_paste(&second);
        assert_eq!(
            editor.get_text(),
            "[paste #1 +12 lines][paste #2 +12 lines]"
        );
        assert_eq!(editor.get_expanded_text(), format!("{first}{second}"));
    }

    #[test]
    fn backspace_over_a_folded_marker_drops_it_and_renumbers() {
        let editor = Editor::simple();
        let first = "a\n".repeat(11);
        let second = "b\n".repeat(11);
        editor.handle_paste(&first);
        editor.handle_paste(&second);
        assert_eq!(
            editor.get_text(),
            "[paste #1 +12 lines][paste #2 +12 lines]"
        );

        // Delete the FIRST marker: one step left lands the caret right after
        // marker #1 (markers are adjacent), and Backspace eats it whole.
        let _ = editor.handle_key(crossterm::event::KeyEvent::new(
            KeyCode::Left,
            KeyModifiers::NONE,
        ));
        let _ = editor.handle_key(crossterm::event::KeyEvent::new(
            KeyCode::Backspace,
            KeyModifiers::NONE,
        ));

        // Only the second survives, renumbered to #1, and it still expands.
        assert_eq!(editor.get_text(), "[paste #1 +12 lines]");
        assert_eq!(editor.get_expanded_text(), second);
    }

    #[test]
    fn deleting_a_marker_renumbers_many_higher_pastes_without_collisions() {
        // Regression: renumbering walked the HashMap's higher ids in
        // arbitrary order, so moving #3 to #2 before #2 had moved to #1
        // overwrote #2's payload. Six pastes make that order near-certain.
        let editor = Editor::simple();
        let payloads: Vec<String> = (1..=6).map(|n| format!("{n}\n").repeat(11)).collect();
        for p in &payloads {
            editor.handle_paste(p);
        }
        let marker_one = "[paste #1 +12 lines]";
        editor.set_cursor(0, marker_one.len());
        let _ = editor.handle_key(crossterm::event::KeyEvent::new(
            KeyCode::Backspace,
            KeyModifiers::NONE,
        ));

        let expected_text: String = (1..=5).map(|n| format!("[paste #{n} +12 lines]")).collect();
        assert_eq!(editor.get_text(), expected_text);
        assert_eq!(editor.get_expanded_text(), payloads[1..].concat());
    }

    #[test]
    fn undo_restores_a_backspaced_folded_paste() {
        let editor = Editor::simple();
        let payload = "line\n".repeat(11);
        editor.handle_paste(&payload);
        assert_eq!(editor.get_text(), "[paste #1 +12 lines]");

        let _ = editor.handle_key(crossterm::event::KeyEvent::new(
            KeyCode::Backspace,
            KeyModifiers::NONE,
        ));
        assert_eq!(editor.get_text(), "", "marker deleted");

        // Undo must resurrect both the marker AND its stashed body.
        editor.undo();
        assert_eq!(editor.get_text(), "[paste #1 +12 lines]");
        assert_eq!(editor.get_expanded_text(), payload);
    }

    #[test]
    fn cursor_arrows_step_over_a_marker_atomically() {
        let editor = Editor::simple();
        let payload = "z\n".repeat(11);
        editor.handle_paste(&payload);
        // Caret is after the marker; one step left must land BEFORE it.
        let _ = editor.handle_key(crossterm::event::KeyEvent::new(
            KeyCode::Left,
            KeyModifiers::NONE,
        ));
        assert_eq!(
            editor.cursor_position(),
            (0, 0),
            "left jumps the whole marker"
        );
        // And one step right must jump back over the whole marker.
        let _ = editor.handle_key(crossterm::event::KeyEvent::new(
            KeyCode::Right,
            KeyModifiers::NONE,
        ));
        assert_eq!(
            editor.cursor_position(),
            (0, "[paste #1 +12 lines]".len()),
            "right jumps the whole marker"
        );
    }

    #[test]
    fn clear_and_set_text_forget_folded_pastes() {
        let editor = Editor::simple();
        editor.handle_paste(&"m\n".repeat(11));
        editor.clear();
        assert_eq!(editor.get_text(), "");
        assert_eq!(editor.get_expanded_text(), "");

        editor.handle_paste(&"n\n".repeat(11));
        editor.set_text("fresh");
        // A programmatic replacement drops the store; a leftover marker would
        // otherwise expand against a stale body.
        assert_eq!(editor.get_text(), "fresh");
        assert_eq!(editor.get_expanded_text(), "fresh");
    }

    #[test]
    fn csi_u_ctrl_sequences_decode_then_drop_like_native_pi() {
        // `ESC[104;5u` is kitty CSI-u for Ctrl+H. Upstream decodes it to the
        // control char (104 - 96 = 8) and then its `charCode >= 32` filter
        // drops it, so a stream of them folds to nothing.
        let editor = Editor::simple();
        editor.handle_paste(&"\x1b[104;5u".repeat(1200));
        assert_eq!(editor.get_text(), "");
        assert_eq!(editor.get_expanded_text(), "");
    }
}
