//! Session-tree hierarchy, filters, and branch folding.
//! Ports tree-selector.ts and its ancestor-selection and folding regressions.

use super::*;
use std::collections::{BTreeMap, HashSet};

const TREE_PAGE: usize = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TreeFilter {
    Default,
    NoTools,
    UserOnly,
    LabeledOnly,
    All,
}

impl TreeFilter {
    const ALL: [Self; 5] = [
        Self::Default,
        Self::NoTools,
        Self::UserOnly,
        Self::LabeledOnly,
        Self::All,
    ];
    fn label(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::NoTools => "no tools",
            Self::UserOnly => "user messages",
            Self::LabeledOnly => "labeled",
            Self::All => "all entries",
        }
    }
}

#[derive(Default)]
struct TreeRow {
    entry: usize,
    parent: Option<usize>,
    children: Vec<usize>,
    depth: usize,
    branch_start: bool,
    connector: Option<bool>,
    gutters: Vec<(usize, bool)>,
}

struct TreeState {
    entries: Vec<Entry>,
    items: Vec<SelectItem>,
    search_text: Vec<String>,
    parents: Vec<Option<usize>>,
    order: Vec<usize>,
    active: HashSet<usize>,
    current: Option<usize>,
    labeled: HashSet<usize>,
    folded: HashSet<usize>,
    filter: TreeFilter,
    query: String,
    rows: Vec<TreeRow>,
    selected: usize,
    remembered: Option<usize>,
    multiple_roots: bool,
}

impl TreeState {
    fn new(
        mut entries: Vec<Entry>,
        current: Option<&str>,
        labels: BTreeMap<String, String>,
    ) -> Self {
        entries.sort_by_key(Entry::seq);
        let by_id: HashMap<_, _> = entries
            .iter()
            .enumerate()
            .map(|(i, entry)| (entry.id().to_string(), i))
            .collect();
        let current_index = current.and_then(|id| by_id.get(id).copied());
        let parents: Vec<_> = entries
            .iter()
            .enumerate()
            .map(|(i, entry)| {
                entry
                    .parent_id()
                    .and_then(|id| by_id.get(id).copied())
                    .filter(|parent| *parent != i)
            })
            .collect();
        let mut active = HashSet::new();
        let mut cursor = current_index;
        while let Some(index) = cursor {
            if !active.insert(index) {
                break;
            }
            cursor = parents[index];
        }
        let mut children = vec![Vec::new(); entries.len()];
        let mut roots = Vec::new();
        for (index, parent) in parents.iter().enumerate() {
            if let Some(parent) = parent {
                children[*parent].push(index);
            } else {
                roots.push(index);
            }
        }
        roots.sort_by_key(|index| !active.contains(index));
        for children in &mut children {
            children.sort_by_key(|index| !active.contains(index));
        }
        let mut stack: Vec<_> = roots.into_iter().rev().collect();
        let mut seen = HashSet::new();
        let mut order = Vec::new();
        // Traverse iteratively. Malformed imported components render once,
        // instead of causing recursion overflow or an infinite parent walk.
        loop {
            while let Some(index) = stack.pop() {
                if !seen.insert(index) {
                    continue;
                }
                order.push(index);
                stack.extend(children[index].iter().rev().copied());
            }
            match (0..entries.len()).find(|index| !seen.contains(index)) {
                Some(index) => stack.push(index),
                None => break,
            }
        }
        let mut labeled = HashSet::new();
        let mut search_text = Vec::new();
        let items = entries
            .iter()
            .enumerate()
            .map(|(index, entry)| {
                let mut item = tree_entry_item(entry, current);
                let label = labels.get(entry.id());
                if let Some(label) = label {
                    labeled.insert(index);
                    let label = ice_tui::strip_ansi(label);
                    let label: String = label
                        .chars()
                        .filter(|character| !character.is_control())
                        .collect();
                    item.label = format!("[{label}] {}", item.label);
                }
                search_text.push(
                    format!("{} {}", label.map_or("", String::as_str), item.search_key())
                        .to_lowercase(),
                );
                item
            })
            .collect();
        let mut state = Self {
            entries,
            items,
            search_text,
            parents,
            order,
            active,
            current: current_index,
            labeled,
            folded: HashSet::new(),
            filter: TreeFilter::Default,
            query: String::new(),
            rows: Vec::new(),
            selected: 0,
            remembered: current_index,
            multiple_roots: false,
        };
        state.rebuild();
        state
    }

    fn matches(&self, index: usize, tokens: &[&str]) -> bool {
        let entry = &self.entries[index];
        if let Entry::Message(message) = entry {
            if let AgentMessage::Assistant(assistant) = &message.message {
                let terminal_error = matches!(
                    assistant.stop_reason,
                    ice_ai::types::StopReason::Error | ice_ai::types::StopReason::Aborted
                );
                if Some(index) != self.current
                    && assistant_text(assistant).trim().is_empty()
                    && !terminal_error
                {
                    return false;
                }
            }
        }
        let metadata = matches!(
            entry,
            Entry::ModelChange(_)
                | Entry::ThinkingLevel(_)
                | Entry::ActiveTools(_)
                | Entry::Custom(_)
        );
        let tool_result = matches!(entry, Entry::Message(message) if matches!(&message.message, AgentMessage::ToolResult(_)));
        let passes = match self.filter {
            TreeFilter::Default => !metadata,
            TreeFilter::NoTools => !metadata && !tool_result,
            TreeFilter::UserOnly => {
                matches!(entry, Entry::Message(message) if matches!(&message.message, AgentMessage::User(_)))
            }
            TreeFilter::LabeledOnly => self.labeled.contains(&index),
            TreeFilter::All => true,
        };
        passes
            && tokens
                .iter()
                .all(|token| self.search_text[index].contains(token))
    }

    fn rebuild(&mut self) {
        let query = self.query.to_lowercase();
        let tokens: Vec<_> = query.split_whitespace().collect();
        let mut rows: Vec<TreeRow> = Vec::new();
        let mut nearest: Vec<Option<usize>> = vec![None; self.entries.len()];
        let mut hidden = HashSet::new();
        for &index in &self.order {
            let parent = self.parents[index];
            let visible_parent = parent.and_then(|parent| nearest[parent]);
            if parent
                .is_some_and(|parent| self.folded.contains(&parent) || hidden.contains(&parent))
            {
                hidden.insert(index);
            }
            if !hidden.contains(&index) && self.matches(index, &tokens) {
                let row = rows.len();
                rows.push(TreeRow {
                    entry: index,
                    parent: visible_parent,
                    ..Default::default()
                });
                if let Some(parent) = visible_parent {
                    rows[parent].children.push(row);
                }
                nearest[index] = Some(row);
            } else {
                nearest[index] = visible_parent;
            }
        }
        self.multiple_roots = rows.iter().filter(|row| row.parent.is_none()).count() > 1;
        for index in 0..rows.len() {
            if let Some(parent) = rows[index].parent {
                let branches = rows[parent].children.len() > 1;
                rows[index].depth = rows[parent].depth
                    + usize::from(
                        branches || (rows[parent].branch_start && rows[parent].depth > 0),
                    );
                rows[index].branch_start = branches;
                rows[index].connector =
                    branches.then(|| rows[parent].children.last() == Some(&index));
                let mut gutters = rows[parent].gutters.clone();
                if let Some(last) = rows[parent].connector {
                    let position = rows[parent]
                        .depth
                        .saturating_sub(usize::from(self.multiple_roots))
                        .saturating_sub(1);
                    gutters.push((position, !last));
                }
                rows[index].gutters = gutters;
            } else {
                rows[index].depth = usize::from(self.multiple_roots);
                rows[index].branch_start = self.multiple_roots;
            }
        }
        self.rows = rows;
        let mut cursor = self.remembered;
        let mut selected = None;
        for _ in 0..self.entries.len() {
            let Some(index) = cursor else {
                break;
            };
            if let Some(found) = self.rows.iter().position(|row| row.entry == index) {
                selected = Some(found);
                break;
            }
            cursor = self.parents[index];
        }
        self.select(selected.unwrap_or_else(|| self.rows.len().saturating_sub(1)));
    }

    fn select(&mut self, index: usize) {
        self.selected = index.min(self.rows.len().saturating_sub(1));
        if let Some(row) = self.rows.get(self.selected) {
            self.remembered = Some(row.entry);
        }
    }

    fn selected_id(&self) -> Option<String> {
        self.rows
            .get(self.selected)
            .map(|row| self.entries[row.entry].id().to_string())
    }

    fn move_rows(&mut self, delta: isize, wrap: bool) {
        if self.rows.is_empty() {
            return;
        }
        let next = if wrap {
            (self.selected as isize + delta).rem_euclid(self.rows.len() as isize) as usize
        } else {
            (self.selected as isize + delta).clamp(0, self.rows.len() as isize - 1) as usize
        };
        self.select(next);
    }

    fn foldable(&self, index: usize) -> bool {
        let row = &self.rows[index];
        !row.children.is_empty()
            && row
                .parent
                .map_or(true, |parent| self.rows[parent].children.len() > 1)
    }

    fn fold_or_up(&mut self) {
        let Some(row) = self.rows.get(self.selected) else {
            return;
        };
        let entry = row.entry;
        if self.foldable(self.selected) && self.folded.insert(entry) {
            self.rebuild();
            return;
        }
        let mut cursor = self.selected;
        while let Some(parent) = self.rows[cursor].parent {
            if self.rows[parent].children.len() > 1 && cursor < self.selected {
                break;
            }
            cursor = parent;
        }
        self.select(cursor);
    }

    fn unfold_or_down(&mut self) {
        let Some(row) = self.rows.get(self.selected) else {
            return;
        };
        let entry = row.entry;
        if self.folded.remove(&entry) {
            self.rebuild();
            return;
        }
        let mut cursor = self.selected;
        while let Some(&child) = self.rows[cursor].children.first() {
            let branch = self.rows[cursor].children.len() > 1;
            cursor = child;
            if branch {
                break;
            }
        }
        self.select(cursor);
    }

    fn set_filter(&mut self, filter: TreeFilter) {
        self.filter = if self.filter == filter {
            TreeFilter::Default
        } else {
            filter
        };
        self.folded.clear();
        self.rebuild();
    }

    fn cycle_filter(&mut self, delta: isize) {
        let index = TreeFilter::ALL
            .iter()
            .position(|filter| *filter == self.filter)
            .unwrap();
        self.filter = TreeFilter::ALL
            [(index as isize + delta).rem_euclid(TreeFilter::ALL.len() as isize) as usize];
        self.folded.clear();
        self.rebuild();
    }

    fn set_query(&mut self, query: String) {
        self.query = query;
        self.folded.clear();
        self.rebuild();
    }

    fn row_body(&self, index: usize) -> (String, usize) {
        let row = &self.rows[index];
        let depth = row.depth.saturating_sub(usize::from(self.multiple_roots));
        let folded = self.folded.contains(&row.entry);
        let mut prefix = String::new();
        for level in 0..depth {
            if let Some((_, show)) = row.gutters.iter().find(|(position, _)| *position == level) {
                prefix.push_str(if *show { "│  " } else { "   " });
            } else if level + 1 == depth && row.connector.is_some() {
                prefix.push(if row.connector == Some(true) {
                    '└'
                } else {
                    '├'
                });
                prefix.push(if folded {
                    '⊞'
                } else if self.foldable(index) {
                    '⊟'
                } else {
                    '─'
                });
                prefix.push(' ');
            } else {
                prefix.push_str("   ");
            }
        }
        if folded && row.connector.is_none() {
            prefix.push_str("⊞ ");
        }
        if self.active.contains(&row.entry) {
            prefix.push_str("• ");
        }
        let anchor = ice_tui::visible_width(&prefix);
        let item = &self.items[row.entry];
        (
            format!(
                "{prefix}{} · {}",
                item.label,
                item.description.as_deref().unwrap_or("")
            ),
            anchor,
        )
    }
}

pub(super) struct TreeSelector {
    state: Mutex<TreeState>,
    input: Input,
    bindings: ice_tui::Keybindings,
    on_select: Arc<dyn Fn(String) + Send + Sync>,
    on_cancel: Arc<dyn Fn() + Send + Sync>,
}

impl TreeSelector {
    pub(super) fn new(
        entries: Vec<Entry>,
        current: Option<&str>,
        labels: BTreeMap<String, String>,
        bindings: ice_tui::Keybindings,
        on_select: Arc<dyn Fn(String) + Send + Sync>,
        on_cancel: Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        let input = Input::with_placeholder("Search tree entries…");
        input.set_focused(true);
        Self {
            state: Mutex::new(TreeState::new(entries, current, labels)),
            input,
            bindings,
            on_select,
            on_cancel,
        }
    }

    pub(super) fn handle_key(&self, key: KeyEvent) {
        let matches = |id| keybinding_matches(&self.bindings, &key, id);
        if matches("tui.select.confirm") {
            let selected = self.state.lock().unwrap().selected_id();
            if let Some(id) = selected {
                (self.on_select)(id);
            }
            return;
        }
        if matches("tui.select.cancel") {
            if self.input.get_value().is_empty() {
                (self.on_cancel)();
            } else {
                self.input.set_value("");
                self.state.lock().unwrap().set_query(String::new());
            }
            return;
        }
        let mut state = self.state.lock().unwrap();
        if matches("tui.select.up") {
            state.move_rows(-1, true);
        } else if matches("tui.select.down") {
            state.move_rows(1, true);
        } else if matches("app.tree.foldOrUp") {
            state.fold_or_up();
        } else if matches("app.tree.unfoldOrDown") {
            state.unfold_or_down();
        } else if matches("tui.select.pageUp") || matches("tui.editor.cursorLeft") {
            state.move_rows(-(TREE_PAGE as isize), false);
        } else if matches("tui.select.pageDown") || matches("tui.editor.cursorRight") {
            state.move_rows(TREE_PAGE as isize, false);
        } else if matches("app.tree.filter.default") {
            state.set_filter(TreeFilter::Default);
        } else if matches("app.tree.filter.noTools") {
            state.set_filter(TreeFilter::NoTools);
        } else if matches("app.tree.filter.userOnly") {
            state.set_filter(TreeFilter::UserOnly);
        } else if matches("app.tree.filter.labeledOnly") {
            state.set_filter(TreeFilter::LabeledOnly);
        } else if matches("app.tree.filter.all") {
            state.set_filter(TreeFilter::All);
        } else if matches("app.tree.filter.cycleForward") {
            state.cycle_filter(1);
        } else if matches("app.tree.filter.cycleBackward") {
            state.cycle_filter(-1);
        } else {
            drop(state);
            let previous = self.input.get_value();
            if matches("tui.editor.deleteCharBackward") {
                self.input
                    .handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
            } else {
                self.input.handle_key(key);
            }
            let query = self.input.get_value();
            if previous != query {
                self.state.lock().unwrap().set_query(query);
            }
        }
    }
}

impl Component for TreeSelector {
    fn render(&self, width: usize) -> Vec<String> {
        let state = self.state.lock().unwrap();
        let mut lines =
            Text::new(format!("Session tree — {}", state.filter.label()), 0, 0).render(width);
        lines.extend(self.input.render(width));
        if state.rows.is_empty() {
            lines.extend(Text::new("No matching entries", 0, 0).render(width));
        } else {
            let start = state
                .selected
                .saturating_sub(TREE_PAGE / 2)
                .min(state.rows.len().saturating_sub(TREE_PAGE));
            let viewport = width.saturating_sub(2);
            let selected_anchor = state.row_body(state.selected).1;
            let min_content = (viewport / 3).clamp(4, 20);
            let offset = if selected_anchor > viewport.saturating_sub(min_content) {
                selected_anchor.saturating_sub((viewport / 4).clamp(2, 12))
            } else {
                0
            };
            for index in start..(start + TREE_PAGE).min(state.rows.len()) {
                let (body, _) = state.row_body(index);
                let body = ice_tui::slice_by_column(&body, offset, viewport, false);
                let row = format!(
                    "{}{body}",
                    if index == state.selected {
                        "› "
                    } else {
                        "  "
                    }
                );
                let row = ice_tui::truncate_to_width(&row, width, "");
                lines.push(if index == state.selected {
                    current_theme().colors.accent.fg(&row)
                } else {
                    row
                });
            }
        }
        lines.extend(
            Text::new(
                format!(
                    "({}/{}) · • active branch",
                    if state.rows.is_empty() {
                        0
                    } else {
                        state.selected + 1
                    },
                    state.rows.len()
                ),
                0,
                0,
            )
            .render(width),
        );
        let hint = |id| {
            let keys = self.bindings.get_keys(id);
            if keys.is_empty() {
                "unbound".into()
            } else {
                keys.iter()
                    .map(|key| key.display())
                    .collect::<Vec<_>>()
                    .join("/")
            }
        };
        for help in [
            format!(
                "{} / {} move · {} choose",
                hint("tui.select.up"),
                hint("tui.select.down"),
                hint("tui.select.confirm")
            ),
            format!(
                "{} fold/up · {} unfold/down",
                hint("app.tree.foldOrUp"),
                hint("app.tree.unfoldOrDown")
            ),
            format!(
                "{} / {} filters · {} clear search / close",
                hint("app.tree.filter.cycleForward"),
                hint("app.tree.filter.cycleBackward"),
                hint("tui.select.cancel")
            ),
        ] {
            lines.extend(Text::new(help, 0, 0).render(width));
        }
        // Input's prompt prefix can exceed an extremely narrow viewport.
        // Bound the complete composite, including title, search and hints.
        lines
            .into_iter()
            .map(|line| ice_tui::truncate_to_width(&line, width, ""))
            .collect()
    }
    fn invalidate(&self) {
        self.input.invalidate();
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ice_harness::session::types::{EntryBase, MessageEntry, ModelChangeEntry};

    fn message(id: &str, parent: Option<&str>, text: &str) -> Entry {
        Entry::Message(MessageEntry {
            base: EntryBase {
                entry_type: "message".into(),
                id: id.into(),
                seq: 0,
                parent_id: parent.map(str::to_string),
                timestamp: 0,
            },
            message: user_message_with_images(text, Vec::new()),
            terminate: None,
        })
    }

    fn branch_tree() -> Vec<Entry> {
        vec![
            message("root", None, "root"),
            message("branch", Some("root"), "branch point"),
            message("b-start", Some("branch"), "other branch"),
            message("b-leaf", Some("b-start"), "other leaf"),
            message("a-start", Some("branch"), "active branch"),
            message("a-mid", Some("a-start"), "active middle"),
            message("a-leaf", Some("a-mid"), "active leaf"),
        ]
    }

    fn view(entries: Vec<Entry>, current: &str) -> TreeSelector {
        TreeSelector::new(
            entries,
            Some(current),
            BTreeMap::new(),
            ice_tui::Keybindings::new(),
            Arc::new(|_| {}),
            Arc::new(|| {}),
        )
    }

    fn key(view: &TreeSelector, code: KeyCode, modifiers: KeyModifiers) {
        view.handle_key(KeyEvent::new(code, modifiers));
    }

    fn selected(view: &TreeSelector) -> Option<String> {
        view.state.lock().unwrap().selected_id()
    }

    #[test]
    fn tree_navigator_folds_unfolds_and_jumps_segments_on_both_branches() {
        let widget = view(branch_tree(), "a-leaf");
        assert_eq!(selected(&widget).as_deref(), Some("a-leaf"));
        key(&widget, KeyCode::Left, KeyModifiers::CONTROL);
        assert_eq!(selected(&widget).as_deref(), Some("a-start"));
        key(&widget, KeyCode::Left, KeyModifiers::CONTROL);
        key(&widget, KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(selected(&widget).as_deref(), Some("b-start"));
        key(&widget, KeyCode::Right, KeyModifiers::ALT);
        assert_eq!(selected(&widget).as_deref(), Some("b-leaf"));
        key(&widget, KeyCode::Left, KeyModifiers::ALT);
        assert_eq!(selected(&widget).as_deref(), Some("b-start"));
        key(&widget, KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(selected(&widget).as_deref(), Some("a-start"));
        key(&widget, KeyCode::Right, KeyModifiers::CONTROL);
        assert_eq!(selected(&widget).as_deref(), Some("a-start"));
        key(&widget, KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(selected(&widget).as_deref(), Some("a-mid"));
        key(&widget, KeyCode::Left, KeyModifiers::CONTROL);
        key(&widget, KeyCode::Right, KeyModifiers::CONTROL);
        assert_eq!(selected(&widget).as_deref(), Some("a-leaf"));
    }

    #[test]
    fn tree_navigator_root_fold_preserves_nested_folds_and_multiple_roots() {
        let mut entries = branch_tree();
        entries.push(message("second-root", None, "second root"));
        entries.push(message("second-child", Some("second-root"), "second child"));
        let widget = view(entries, "a-leaf");
        for _ in 0..3 {
            key(&widget, KeyCode::Left, KeyModifiers::CONTROL);
        }
        assert_eq!(selected(&widget).as_deref(), Some("root"));
        key(&widget, KeyCode::Left, KeyModifiers::CONTROL);
        key(&widget, KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(selected(&widget).as_deref(), Some("second-root"));
        key(&widget, KeyCode::Up, KeyModifiers::NONE);
        key(&widget, KeyCode::Right, KeyModifiers::CONTROL);
        key(&widget, KeyCode::Right, KeyModifiers::CONTROL);
        assert_eq!(selected(&widget).as_deref(), Some("a-start"));
        key(&widget, KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(selected(&widget).as_deref(), Some("b-start"));
    }

    #[test]
    fn tree_navigator_hidden_metadata_focuses_ancestor_and_root_fold_hides_descendants() {
        let mut entries = branch_tree();
        entries.push(Entry::ModelChange(ModelChangeEntry {
            base: EntryBase {
                entry_type: "model_change".into(),
                id: "model".into(),
                seq: 0,
                parent_id: Some("a-leaf".into()),
                timestamp: 0,
            },
            provider: "faux".into(),
            model_id: "m".into(),
        }));
        let widget = view(entries, "model");
        assert_eq!(selected(&widget).as_deref(), Some("a-leaf"));
        key(&widget, KeyCode::Char('a'), KeyModifiers::CONTROL);
        widget.state.lock().unwrap().select(5);
        assert_eq!(selected(&widget).as_deref(), Some("model"));
        key(&widget, KeyCode::Char('d'), KeyModifiers::CONTROL);
        assert_eq!(selected(&widget).as_deref(), Some("a-leaf"));
        let mut filtered = vec![message("root", None, "root")];
        let mut assistant = AssistantMessage::empty(ice_ai::Api::Faux, "faux", "m", 0);
        assistant.stop_reason = ice_ai::types::StopReason::ToolUse;
        filtered.push(Entry::Message(MessageEntry {
            base: EntryBase {
                entry_type: "message".into(),
                id: "hidden".into(),
                seq: 0,
                parent_id: Some("root".into()),
                timestamp: 0,
            },
            message: AgentMessage::Assistant(Box::new(assistant)),
            terminate: None,
        }));
        filtered.push(message("leaf", Some("hidden"), "leaf"));
        let widget = view(filtered, "leaf");
        key(&widget, KeyCode::Left, KeyModifiers::CONTROL);
        key(&widget, KeyCode::Left, KeyModifiers::CONTROL);
        assert_eq!(widget.state.lock().unwrap().rows.len(), 1);
    }

    #[test]
    fn tree_navigator_empty_filters_preserve_cursor_and_search_clears_folds() {
        let widget = view(branch_tree(), "a-leaf");
        key(&widget, KeyCode::Char('l'), KeyModifiers::CONTROL);
        assert!(selected(&widget).is_none());
        key(&widget, KeyCode::Char('d'), KeyModifiers::CONTROL);
        assert_eq!(selected(&widget).as_deref(), Some("a-leaf"));
        key(&widget, KeyCode::Left, KeyModifiers::CONTROL);
        key(&widget, KeyCode::Left, KeyModifiers::CONTROL);
        assert!(!widget.state.lock().unwrap().folded.is_empty());
        key(&widget, KeyCode::Char('a'), KeyModifiers::NONE);
        assert!(widget.state.lock().unwrap().folded.is_empty());
        key(&widget, KeyCode::Esc, KeyModifiers::NONE);
        assert!(widget.input.get_value().is_empty());
        key(&widget, KeyCode::Char('u'), KeyModifiers::CONTROL);
        assert_eq!(widget.state.lock().unwrap().filter, TreeFilter::UserOnly);
        let mut labels = BTreeMap::new();
        labels.insert("a-start".into(), "checkpoint".into());
        let mut labeled = TreeState::new(branch_tree(), Some("a-leaf"), labels);
        labeled.set_filter(TreeFilter::LabeledOnly);
        assert_eq!(labeled.selected_id().as_deref(), Some("a-start"));
        labeled.set_query("checkpoint".into());
        assert_eq!(labeled.rows.len(), 1);
    }

    #[test]
    fn tree_navigator_selection_cancel_and_remapped_controls_use_full_identity() {
        let selected_id = Arc::new(Mutex::new(None));
        let selected_cb = selected_id.clone();
        let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let cancelled_cb = cancelled.clone();
        let mut bindings = ice_tui::Keybindings::new();
        bindings.set(
            "tui.select.confirm",
            vec![ice_tui::KeyCombo::new(KeyCode::F(6), KeyModifiers::NONE)],
        );
        let widget = TreeSelector::new(
            branch_tree(),
            Some("a-leaf"),
            BTreeMap::new(),
            bindings,
            Arc::new(move |id| *selected_cb.lock().unwrap() = Some(id)),
            Arc::new(move || {
                cancelled_cb.store(true, std::sync::atomic::Ordering::SeqCst);
            }),
        );
        key(&widget, KeyCode::Enter, KeyModifiers::NONE);
        assert!(selected_id.lock().unwrap().is_none());
        key(&widget, KeyCode::F(6), KeyModifiers::NONE);
        assert_eq!(selected_id.lock().unwrap().as_deref(), Some("a-leaf"));
        key(&widget, KeyCode::Esc, KeyModifiers::NONE);
        assert!(cancelled.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn tree_navigator_renders_branch_connectors_and_keeps_narrow_rows_bounded() {
        let widget = view(branch_tree(), "a-leaf");
        let rendered = ice_tui::strip_ansi(&widget.render(100).join("\n"));
        assert!(
            rendered.contains("├⊟") && rendered.contains("└⊟"),
            "{rendered}"
        );
        assert!(rendered.find("active branch").unwrap() < rendered.find("other branch").unwrap());
        for width in [1, 10, 20, 40, 80] {
            for row in widget.render(width) {
                assert!(ice_tui::visible_width(&row) <= width, "{width}: {row}");
            }
        }
        let mut entries = Vec::new();
        for index in 0..2000 {
            entries.push(message(
                &index.to_string(),
                (index > 0).then(|| (index - 1).to_string()).as_deref(),
                "deep chain",
            ));
        }
        let deep = view(entries, "1999");
        assert_eq!(selected(&deep).as_deref(), Some("1999"));
        assert!(deep
            .state
            .lock()
            .unwrap()
            .rows
            .iter()
            .all(|row| row.depth == 0));
    }
}
