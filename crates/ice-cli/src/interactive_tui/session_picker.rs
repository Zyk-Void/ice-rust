//! Saved session selection with a persistent current/all-project scope.
use super::*;
use crate::session_catalog::{canonical, display_text, SessionRow};
use std::any::Any;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

pub(super) struct SessionPicker {
    rows: Mutex<Vec<SessionRow>>,
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
    ) -> Arc<Self> {
        let picker = Arc::new(Self {
            rows: Mutex::new(Vec::new()),
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
        picker.input.set_focused(true);
        picker.rebuild();
        picker
    }

    fn rebuild(&self) {
        let query = self.input.get_value();
        let all = self.all.load(Ordering::Relaxed);
        let items = self
            .rows
            .lock()
            .unwrap()
            .iter()
            .filter(|row| all || row.cwd == self.cwd)
            .map(|row| {
                let description = format!(
                    "{} · {} · {}",
                    display_text(&row.cwd.display().to_string()),
                    crate::session::format_session_bytes(row.bytes),
                    row.id.chars().take(8).collect::<String>()
                );
                SelectItem::new(&row.path.to_string_lossy(), &row.label)
                    .with_description(&description)
                    .with_search_text(&format!("{} {} {}", row.label, row.id, row.cwd.display()))
            })
            .collect();
        let list = Arc::new(SelectList::new(items, 10));
        list.set_filter(&query);
        let choose = self.choose.clone();
        list.on_select(Arc::new(move |item| choose(item.value.clone())));
        list.on_cancel(self.cancel.clone());
        *self.list.lock().unwrap() = list;
    }

    pub(super) fn handle_key(&self, key: KeyEvent) {
        if key.code == KeyCode::Tab {
            self.all.fetch_xor(true, Ordering::Relaxed);
            self.rebuild();
            return;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if matches!(
            key.code,
            KeyCode::Esc
                | KeyCode::Enter
                | KeyCode::Up
                | KeyCode::Down
                | KeyCode::PageUp
                | KeyCode::PageDown
        ) || (ctrl && matches!(key.code, KeyCode::Char('j' | 'k' | 'n' | 'p')))
        {
            let list = self.list.lock().unwrap().clone();
            list.handle_key(key);
        } else {
            self.input.handle_key(key);
        }
    }
}

impl Component for SessionPicker {
    fn render(&self, width: usize) -> Vec<String> {
        let scope = if self.all.load(Ordering::Relaxed) {
            "All projects"
        } else {
            "Current folder"
        };
        let mut lines = vec![
            ice_tui::truncate_to_width(&format!("Resume session · {scope}"), width, "…"),
            ice_tui::truncate_to_width(
                "Tab: scope · type to filter · Enter: resume · Esc: cancel",
                width,
                "…",
            ),
        ];
        lines.extend(self.input.render(width));
        if self.loading.load(Ordering::Relaxed) {
            lines.push(ice_tui::truncate_to_width(
                "Loading saved sessions…",
                width,
                "…",
            ));
        } else {
            let list = self.list.lock().unwrap().clone();
            lines.extend(list.render(width));
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
    let picker = SessionPicker::new(cwd, choose, close);
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
            *picker.rows.lock().unwrap() = rows;
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
        let picker = SessionPicker::new(Path::new("/project-a"), Arc::new(|_| {}), Arc::new(|| {}));
        *picker.rows.lock().unwrap() = vec![
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
        ];
        picker.loading.store(false, Ordering::Relaxed);
        picker.rebuild();
        picker
    }
    #[test]
    fn toggling_scope_preserves_query_and_uses_unambiguous_paths() {
        let picker = picker();
        for ch in "second".chars() {
            picker.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
        }
        assert!(picker.list.lock().unwrap().get_selected_item().is_none());
        picker.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
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
}
