//! Saved session selection with a persistent current/all-project scope.
use super::*;
use crate::session_actions::SessionAction;
use crate::session_catalog::{canonical, display_text, SessionRow};
use ice_harness::session::types::SessionStorage;
use std::any::Any;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

struct SearchRequest {
    generation: u64,
    rows: Arc<Vec<SessionRow>>,
    cwd: std::path::PathBuf,
    all: bool,
    query: String,
}

#[derive(Clone)]
enum PickerMode {
    Browse,
    Rename(SessionRow),
    ConfirmDelete(SessionRow),
    Busy,
}

pub(super) struct SessionPicker {
    rows: Mutex<Arc<Vec<SessionRow>>>,
    generation: Arc<AtomicU64>,
    requests: std::sync::mpsc::Sender<SearchRequest>,
    searching: AtomicBool,
    error: Mutex<Option<String>>,
    mode: Mutex<PickerMode>,
    name_input: Arc<Input>,
    notice: Mutex<Option<String>>,
    active: Option<Arc<dyn SessionStorage>>,
    bindings: Arc<ice_tui::Keybindings>,
    redraw: Arc<dyn Fn() + Send + Sync>,
    cwd: std::path::PathBuf,
    all: AtomicBool,
    loading: AtomicBool,
    input: Arc<Input>,
    list: Mutex<Arc<SelectList>>,
    choose: Arc<dyn Fn(String) + Send + Sync>,
    cancel: Arc<dyn Fn() + Send + Sync>,
}

impl SessionPicker {
    fn new(
        cwd: &Path,
        choose: Arc<dyn Fn(String) + Send + Sync>,
        cancel: Arc<dyn Fn() + Send + Sync>,
        redraw: Arc<dyn Fn() + Send + Sync>,
        active: Option<Arc<dyn SessionStorage>>,
        bindings: Arc<ice_tui::Keybindings>,
    ) -> Arc<Self> {
        let (requests, receiver) = std::sync::mpsc::channel::<SearchRequest>();
        let picker = Arc::new(Self {
            rows: Mutex::new(Arc::new(Vec::new())),
            generation: Arc::new(AtomicU64::new(0)),
            requests,
            searching: AtomicBool::new(false),
            error: Mutex::new(None),
            mode: Mutex::new(PickerMode::Browse),
            name_input: Arc::new(Input::with_placeholder("Session name (blank clears it)")),
            notice: Mutex::new(None),
            active,
            bindings,
            redraw,
            cwd: canonical(cwd),
            all: AtomicBool::new(false),
            loading: AtomicBool::new(true),
            input: Arc::new(Input::with_placeholder("Find a saved session…")),
            list: Mutex::new(Arc::new(SelectList::new(Vec::new(), 10))),
            choose,
            cancel,
        });
        let weak = Arc::downgrade(&picker);
        picker.input.on_change(Arc::new(move |_| {
            if let Some(picker) = weak.upgrade() {
                picker.rebuild();
            }
        }));
        let weak = Arc::downgrade(&picker);
        let generation = picker.generation.clone();
        std::thread::spawn(move || {
            let mut cache = crate::session_search::ContentCache::default();
            while let Ok(mut request) = receiver.recv() {
                while let Ok(latest) = receiver.try_recv() {
                    request = latest;
                }
                let cancelled = || {
                    weak.strong_count() == 0
                        || generation.load(Ordering::Acquire) != request.generation
                };
                let result = crate::session_search::search_rows(
                    &request.rows,
                    &request.cwd,
                    request.all,
                    &request.query,
                    &mut cache,
                    &cancelled,
                );
                if let Some(picker) = weak.upgrade() {
                    picker.install(request, result);
                }
            }
        });
        picker.input.set_focused(true);
        picker.name_input.set_focused(true);
        picker.rebuild();
        picker
    }

    fn rebuild(&self) {
        let rows = self.rows.lock().unwrap().clone();
        // Publish the new generation and clear selection under the same lock as
        // result installation. Enter cannot act on an obsolete query result.
        let mut list = self.list.lock().unwrap();
        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        *list = self.make_list(Vec::new());
        self.searching.store(true, Ordering::Release);
        *self.error.lock().unwrap() = None;
        // Send while still holding the lock: requests must reach the worker in
        // generation order, or it can keep only an obsolete one and the picker
        // stays on "Searching saved messages…".
        let _ = self.requests.send(SearchRequest {
            generation,
            rows,
            cwd: self.cwd.clone(),
            all: self.all.load(Ordering::Relaxed),
            query: self.input.get_value(),
        });
        drop(list);
        (self.redraw)();
    }

    fn make_list(&self, items: Vec<SelectItem>) -> Arc<SelectList> {
        let list = Arc::new(SelectList::new(items, 10));
        let choose = self.choose.clone();
        list.on_select(Arc::new(move |item| choose(item.value.clone())));
        list.on_cancel(self.cancel.clone());
        list
    }

    fn install(
        &self,
        request: SearchRequest,
        result: Result<crate::session_search::SearchResult, String>,
    ) {
        let mut list = self.list.lock().unwrap();
        if self.generation.load(Ordering::Acquire) != request.generation {
            return;
        }
        let items = match result {
            Ok(result) => {
                *self.error.lock().unwrap() = (result.unreadable > 0).then(|| {
                    format!(
                        "Could not search {} unreadable session(s)",
                        result.unreadable
                    )
                });
                result
                    .indices
                    .into_iter()
                    .map(|index| {
                        let row = &request.rows[index];
                        let description = format!(
                            "{} · {} · {}",
                            display_text(&row.cwd.display().to_string()),
                            crate::session::format_session_bytes(row.bytes),
                            row.id.chars().take(8).collect::<String>()
                        );
                        SelectItem::new(&row.path.to_string_lossy(), &row.label)
                            .with_description(&description)
                    })
                    .collect()
            }
            Err(error) => {
                *self.error.lock().unwrap() = Some(error);
                Vec::new()
            }
        };
        *list = self.make_list(items);
        self.searching.store(false, Ordering::Release);
        drop(list);
        (self.redraw)();
    }

    fn selected_row(&self) -> Option<SessionRow> {
        let path = self.list.lock().unwrap().get_selected_item()?.value;
        self.rows
            .lock()
            .unwrap()
            .iter()
            .find(|row| row.path == std::path::PathBuf::from(&path))
            .cloned()
    }

    fn start_action(self: &Arc<Self>, row: SessionRow, action: SessionAction) {
        *self.mode.lock().unwrap() = PickerMode::Busy;
        *self.notice.lock().unwrap() = None;
        (self.redraw)();
        let weak = Arc::downgrade(self);
        let active = self.active.clone();
        std::thread::spawn(move || {
            let result = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| error.to_string())
                .and_then(|runtime| {
                    runtime.block_on(crate::session_actions::perform(
                        &row,
                        &action,
                        active.as_ref(),
                    ))
                });
            if let Some(picker) = weak.upgrade() {
                if result.is_ok() {
                    let mut rows: Vec<_> = picker.rows.lock().unwrap().iter().cloned().collect();
                    rows.retain(|candidate| candidate.path != row.path);
                    if matches!(action, SessionAction::Rename(_)) {
                        if let Some(updated) = crate::session_catalog::load_rows(&[row
                            .path
                            .parent()
                            .unwrap()
                            .to_path_buf()])
                        .into_iter()
                        .find(|candidate| candidate.path == row.path)
                        {
                            rows.push(updated);
                        }
                    }
                    rows.sort_by(|left, right| {
                        right
                            .modified
                            .cmp(&left.modified)
                            .then_with(|| left.path.cmp(&right.path))
                    });
                    *picker.rows.lock().unwrap() = Arc::new(rows);
                }
                *picker.mode.lock().unwrap() = PickerMode::Browse;
                picker.rebuild();
                *picker.notice.lock().unwrap() = Some(match result {
                    Ok(()) => match action {
                        SessionAction::Rename(_) => "Session renamed.".into(),
                        SessionAction::Delete => "Session deleted.".into(),
                    },
                    Err(error) => error,
                });
                (picker.redraw)();
            }
        });
    }

    fn hint(&self, id: ice_tui::KeybindingId) -> String {
        let keys = self
            .bindings
            .get_keys(id)
            .iter()
            .map(|key| key.display())
            .collect::<Vec<_>>();
        if keys.is_empty() {
            "unbound".into()
        } else {
            keys.join("/")
        }
    }

    pub(super) fn handle_key(self: &Arc<Self>, key: KeyEvent) {
        let mode = self.mode.lock().unwrap().clone();
        match mode {
            PickerMode::Rename(row) => {
                if self.bindings.matches(&key, "tui.select.cancel") {
                    *self.mode.lock().unwrap() = PickerMode::Browse;
                } else if self.bindings.matches(&key, "tui.select.confirm") {
                    let name = self.name_input.get_value();
                    if name.trim() == row.label.trim() {
                        // Unedited: the prefill may be the first-message
                        // preview, which must not be saved as a name.
                        *self.mode.lock().unwrap() = PickerMode::Browse;
                    } else {
                        self.start_action(row, SessionAction::Rename(name));
                    }
                } else {
                    self.name_input.handle_key(key);
                }
                (self.redraw)();
                return;
            }
            PickerMode::ConfirmDelete(row) => {
                if self.bindings.matches(&key, "tui.select.cancel") {
                    *self.mode.lock().unwrap() = PickerMode::Browse;
                } else if self.bindings.matches(&key, "tui.select.confirm") {
                    self.start_action(row, SessionAction::Delete);
                }
                (self.redraw)();
                return;
            }
            PickerMode::Busy => return,
            PickerMode::Browse => {}
        }
        if self.bindings.matches(&key, "app.session.rename") {
            if let Some(row) = self.selected_row() {
                self.name_input.set_value(&row.label);
                *self.mode.lock().unwrap() = PickerMode::Rename(row);
                *self.notice.lock().unwrap() = None;
                (self.redraw)();
            }
            return;
        }
        if self.bindings.matches(&key, "app.session.delete") {
            if let Some(row) = self.selected_row() {
                *self.mode.lock().unwrap() = PickerMode::ConfirmDelete(row);
                *self.notice.lock().unwrap() = None;
                (self.redraw)();
            }
            return;
        }
        if self.bindings.matches(&key, "app.session.scope") {
            self.all.fetch_xor(true, Ordering::Relaxed);
            self.rebuild();
            return;
        }
        if let Some(selection_key) = self.bindings.selection_key(&key) {
            let list = self.list.lock().unwrap().clone();
            list.handle_key(selection_key);
        } else {
            self.input.handle_key(key);
        }
    }
}

impl Component for SessionPicker {
    fn render(&self, width: usize) -> Vec<String> {
        let mode = self.mode.lock().unwrap().clone();
        let prompt = match mode {
            PickerMode::Rename(row) => {
                let mut lines = vec![
                    "Rename session".into(),
                    row.label,
                    display_text(&row.path.display().to_string()),
                    format!(
                        "{}: save · {}: cancel",
                        self.hint("tui.select.confirm"),
                        self.hint("tui.select.cancel")
                    ),
                ];
                lines.extend(self.name_input.render(width));
                Some(lines)
            }
            PickerMode::ConfirmDelete(row) => Some(vec![
                "Delete this saved session?".into(),
                row.label,
                display_text(&row.path.display().to_string()),
                "This removes its saved conversation.".into(),
                format!(
                    "{}: delete · {}: cancel",
                    self.hint("tui.select.confirm"),
                    self.hint("tui.select.cancel")
                ),
            ]),
            PickerMode::Busy => Some(vec!["Updating saved session…".into()]),
            PickerMode::Browse => None,
        };
        if let Some(lines) = prompt {
            return lines
                .into_iter()
                .map(|line| ice_tui::truncate_to_width(&line, width, ""))
                .collect();
        }
        let scope = if self.all.load(Ordering::Relaxed) {
            "All projects"
        } else {
            "Current folder"
        };
        let mut lines = vec![
            ice_tui::truncate_to_width(&format!("Resume session · {scope}"), width, "…"),
            ice_tui::truncate_to_width(
                &format!(
                    "{}: scope · {}: rename · {}: delete · {}: resume · {}: cancel",
                    self.hint("app.session.scope"),
                    self.hint("app.session.rename"),
                    self.hint("app.session.delete"),
                    self.hint("tui.select.confirm"),
                    self.hint("tui.select.cancel")
                ),
                width,
                "…",
            ),
            ice_tui::truncate_to_width("Find: text · \"phrase\" · re:pattern", width, "…"),
        ];
        lines.extend(self.input.render(width));
        if self.loading.load(Ordering::Relaxed) {
            lines.push(ice_tui::truncate_to_width(
                "Loading saved sessions…",
                width,
                "…",
            ));
        } else if self.searching.load(Ordering::Acquire) {
            lines.push("Searching saved messages…".to_string());
        } else {
            let list = self.list.lock().unwrap().clone();
            lines.extend(list.render(width));
        }
        if let Some(error) = self.error.lock().unwrap().as_ref() {
            lines.push(display_text(error));
        }
        if let Some(notice) = self.notice.lock().unwrap().as_ref() {
            lines.push(display_text(notice));
        }
        lines
            .into_iter()
            .map(|line| ice_tui::truncate_to_width(&line, width, ""))
            .collect()
    }
    fn invalidate(&self) {
        self.input.invalidate();
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub(super) fn open(
    state: &Arc<TuiState>,
    editor_container: &Arc<Container>,
    editor: &Arc<Editor>,
    tui: &Arc<TuiAltScreen>,
    cwd: &Path,
    tx: &mpsc::UnboundedSender<TuiMessage>,
    active: Option<Arc<dyn SessionStorage>>,
) {
    let close = {
        let (state, container, editor, tui) = (
            state.clone(),
            editor_container.clone(),
            editor.clone(),
            tui.clone(),
        );
        Arc::new(move || close_selector(&state, &container, &editor, &tui))
            as Arc<dyn Fn() + Send + Sync>
    };
    let choose = {
        let (close, tx) = (close.clone(), tx.clone());
        Arc::new(move |path| {
            close();
            let _ = tx.send(TuiMessage::SwitchSession(path));
        })
    };
    let redraw = {
        let tui = tui.clone();
        Arc::new(move || tui.request_render(false))
    };
    let picker = SessionPicker::new(
        cwd,
        choose,
        close,
        redraw,
        active,
        Arc::new(ice_tui::get_keybindings().clone()),
    );
    let router = picker.clone();
    open_selector_with_view(
        state,
        editor_container,
        editor,
        tui,
        SelectorView::Custom(Arc::new(move |key| router.handle_key(key))),
        picker.clone(),
        SelectorKind::Session,
    );
    let current = crate::session::default_session_dir(cwd);
    let directories = crate::config::agent_dir()
        .map(|agent| crate::session_catalog::known_directories(&agent, &current))
        .unwrap_or_else(|_| vec![current]);
    let weak = Arc::downgrade(&picker);
    let tui = tui.clone();
    std::thread::spawn(move || {
        let rows = crate::session_catalog::load_rows(&directories);
        if let Some(picker) = weak.upgrade() {
            *picker.rows.lock().unwrap() = Arc::new(rows);
            picker.loading.store(false, Ordering::Relaxed);
            picker.rebuild();
            tui.request_render(false);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    fn picker() -> Arc<SessionPicker> {
        let picker = SessionPicker::new(
            Path::new("/project-a"),
            Arc::new(|_| {}),
            Arc::new(|| {}),
            Arc::new(|| {}),
            None,
            Arc::new(ice_tui::Keybindings::new()),
        );
        *picker.rows.lock().unwrap() = Arc::new(vec![
            SessionRow {
                path: "/a.jsonl".into(),
                cwd: "/project-a".into(),
                id: "same".into(),
                label: "first".into(),
                bytes: 1,
                modified: std::time::UNIX_EPOCH,
            },
            SessionRow {
                path: "/b.jsonl".into(),
                cwd: "/project-b".into(),
                id: "same".into(),
                label: "second".into(),
                bytes: 1,
                modified: std::time::UNIX_EPOCH,
            },
        ]);
        picker.loading.store(false, Ordering::Relaxed);
        picker.rebuild();
        settle(&picker);
        picker
    }

    fn settle(picker: &SessionPicker) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while picker.searching.load(Ordering::Acquire) {
            assert!(
                std::time::Instant::now() < deadline,
                "search worker did not finish"
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    #[test]
    fn toggling_scope_preserves_query_and_uses_unambiguous_paths() {
        let picker = picker();
        for ch in "second".chars() {
            picker.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
        }
        settle(&picker);
        assert!(picker.list.lock().unwrap().get_selected_item().is_none());
        picker.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        settle(&picker);
        assert_eq!(picker.input.get_value(), "second");
        assert_eq!(
            picker
                .list
                .lock()
                .unwrap()
                .get_selected_item()
                .unwrap()
                .value,
            "/b.jsonl"
        );
        picker.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        settle(&picker);
        assert!(picker.list.lock().unwrap().get_selected_item().is_none());
    }
    #[test]
    fn picker_can_cancel_while_loading_and_renders_narrowly() {
        let cancelled = Arc::new(AtomicBool::new(false));
        let flag = cancelled.clone();
        let picker = SessionPicker::new(
            Path::new("/a"),
            Arc::new(|_| {}),
            Arc::new(move || {
                flag.store(true, Ordering::Relaxed);
            }),
            Arc::new(|| {}),
            None,
            Arc::new(ice_tui::Keybindings::new()),
        );
        picker.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(cancelled.load(Ordering::Relaxed));
        for width in 1..80 {
            assert!(picker
                .render(width)
                .iter()
                .all(|line| ice_tui::visible_width(line) <= width));
        }
    }
    #[test]
    fn picker_uses_effective_keys_for_navigation_actions_and_cancellation() {
        let mut bindings = ice_tui::Keybindings::new();
        for (action, code) in [
            ("tui.select.up", KeyCode::F(7)),
            ("tui.select.down", KeyCode::F(8)),
            ("tui.select.confirm", KeyCode::F(10)),
            ("tui.select.cancel", KeyCode::F(12)),
            ("app.session.scope", KeyCode::F(6)),
        ] {
            bindings.set(
                action,
                vec![ice_tui::KeyCombo::new(code, KeyModifiers::NONE)],
            );
        }
        let selected = Arc::new(Mutex::new(None));
        let chosen = selected.clone();
        let cancellations = Arc::new(AtomicU64::new(0));
        let cancelled = cancellations.clone();
        let rows = picker().rows.lock().unwrap().clone();
        let picker = SessionPicker::new(
            Path::new("/project-a"),
            Arc::new(move |path| *chosen.lock().unwrap() = Some(path)),
            Arc::new(move || {
                cancelled.fetch_add(1, Ordering::Relaxed);
            }),
            Arc::new(|| {}),
            None,
            Arc::new(bindings),
        );
        *picker.rows.lock().unwrap() = rows;
        picker.loading.store(false, Ordering::Relaxed);
        picker.rebuild();
        settle(&picker);
        picker.handle_key(KeyEvent::new(KeyCode::F(6), KeyModifiers::NONE));
        settle(&picker);
        assert!(picker.all.load(Ordering::Relaxed));
        let current = || {
            picker
                .list
                .lock()
                .unwrap()
                .get_selected_item()
                .unwrap()
                .value
        };
        assert_eq!(current(), "/a.jsonl");
        picker.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(current(), "/a.jsonl", "released default must not navigate");
        picker.handle_key(KeyEvent::new(KeyCode::F(8), KeyModifiers::NONE));
        assert_eq!(current(), "/b.jsonl");
        picker.handle_key(KeyEvent::new(KeyCode::F(7), KeyModifiers::NONE));
        assert_eq!(current(), "/a.jsonl");
        picker.handle_key(KeyEvent::new(KeyCode::F(8), KeyModifiers::NONE));
        for key in [KeyCode::Enter, KeyCode::Esc] {
            picker.handle_key(KeyEvent::new(key, KeyModifiers::NONE));
        }
        assert!(selected.lock().unwrap().is_none());
        assert_eq!(cancellations.load(Ordering::Relaxed), 0);
        picker.handle_key(KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE));
        assert!(matches!(
            *picker.mode.lock().unwrap(),
            PickerMode::Rename(_)
        ));
        picker.handle_key(KeyEvent::new(KeyCode::F(12), KeyModifiers::NONE));
        assert!(matches!(*picker.mode.lock().unwrap(), PickerMode::Browse));
        picker.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));
        assert!(matches!(
            *picker.mode.lock().unwrap(),
            PickerMode::ConfirmDelete(_)
        ));
        picker.handle_key(KeyEvent::new(KeyCode::F(12), KeyModifiers::NONE));
        assert!(matches!(*picker.mode.lock().unwrap(), PickerMode::Browse));
        assert_eq!(cancellations.load(Ordering::Relaxed), 0);
        picker.handle_key(KeyEvent::new(KeyCode::F(10), KeyModifiers::NONE));
        assert_eq!(selected.lock().unwrap().as_deref(), Some("/b.jsonl"));
        picker.handle_key(KeyEvent::new(KeyCode::F(12), KeyModifiers::NONE));
        assert_eq!(cancellations.load(Ordering::Relaxed), 1);
        let hints = ice_tui::strip_ansi(&picker.render(240).join("\n"));
        assert!(
            hints.contains("F6: scope")
                && hints.contains("F10: resume")
                && hints.contains("F12: cancel")
        );
    }

    #[test]
    fn obsolete_results_cannot_replace_the_latest_query() {
        let picker = picker();
        let old = picker.generation.load(Ordering::Acquire);
        picker.input.set_value("re:[");
        picker.rebuild();
        settle(&picker);
        let rows = picker.rows.lock().unwrap().clone();
        picker.install(
            SearchRequest {
                generation: old,
                rows,
                cwd: picker.cwd.clone(),
                all: true,
                query: String::new(),
            },
            Ok(crate::session_search::SearchResult {
                indices: vec![0],
                unreadable: 0,
            }),
        );
        assert!(picker.list.lock().unwrap().get_selected_item().is_none());
        assert!(picker
            .error
            .lock()
            .unwrap()
            .as_deref()
            .unwrap()
            .contains("Invalid regular expression"));
    }
    #[test]
    fn confirming_an_unedited_rename_changes_nothing() {
        let picker = picker();
        picker.handle_key(KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE));
        picker.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(matches!(*picker.mode.lock().unwrap(), PickerMode::Browse));
    }
    #[test]
    fn cancelling_rename_or_delete_returns_to_the_same_picker() {
        let picker = picker();
        let original = picker
            .list
            .lock()
            .unwrap()
            .get_selected_item()
            .unwrap()
            .value;
        picker.handle_key(KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE));
        assert!(matches!(
            *picker.mode.lock().unwrap(),
            PickerMode::Rename(_)
        ));
        picker.name_input.set_value("discarded name");
        picker.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(matches!(*picker.mode.lock().unwrap(), PickerMode::Browse));
        picker.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));
        assert!(matches!(
            *picker.mode.lock().unwrap(),
            PickerMode::ConfirmDelete(_)
        ));
        for width in 1..40 {
            assert!(picker
                .render(width)
                .iter()
                .all(|line| ice_tui::visible_width(line) <= width));
        }
        picker.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(matches!(*picker.mode.lock().unwrap(), PickerMode::Browse));
        assert_eq!(
            picker
                .list
                .lock()
                .unwrap()
                .get_selected_item()
                .unwrap()
                .value,
            original
        );
        assert!(picker.notice.lock().unwrap().is_none());
    }
}
