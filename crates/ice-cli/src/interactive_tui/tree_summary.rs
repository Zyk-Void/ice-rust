//! Session-tree summary choice, prompt editing, and accepted navigation effects.

use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct TreeNavigationRequest {
    pub(super) entry_id: String,
    pub(super) summarize: bool,
    pub(super) instructions: Option<String>,
}

pub(super) struct TreeSummarySelector {
    entry_id: String,
    list: SelectList,
    instructions: Editor,
    editing: Mutex<bool>,
    bindings: ice_tui::Keybindings,
    on_select: Arc<dyn Fn(TreeNavigationRequest) + Send + Sync>,
    on_cancel: Arc<dyn Fn() + Send + Sync>,
}

impl TreeSummarySelector {
    pub(super) fn new(
        entry_id: String,
        bindings: ice_tui::Keybindings,
        on_select: Arc<dyn Fn(TreeNavigationRequest) + Send + Sync>,
        on_cancel: Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        let instructions = Editor::new(
            EditorOptions::default(),
            EditorStyle::default(),
            Arc::new(bindings.clone()),
        );
        Self {
            entry_id,
            list: SelectList::new(
                vec![
                    SelectItem::new("none", "No summary"),
                    SelectItem::new("summary", "Summarize"),
                    SelectItem::new("custom", "Summarize with custom prompt"),
                ],
                3,
            ),
            instructions,
            editing: Mutex::new(false),
            bindings,
            on_select,
            on_cancel,
        }
    }

    fn submit(&self, summarize: bool, instructions: Option<String>) {
        (self.on_select)(TreeNavigationRequest {
            entry_id: self.entry_id.clone(),
            summarize,
            instructions: instructions.filter(|value| !value.trim().is_empty()),
        });
    }

    pub(super) fn handle_key(&self, key: KeyEvent) {
        let matches = |id| keybinding_matches(&self.bindings, &key, id);
        let editing = *self.editing.lock().unwrap();
        if matches("tui.select.cancel") {
            if editing {
                *self.editing.lock().unwrap() = false;
                self.instructions.set_focused(false);
            } else {
                (self.on_cancel)();
            }
            return;
        }
        if editing {
            if matches("tui.input.submit") {
                self.submit(true, Some(self.instructions.get_expanded_text()));
            } else {
                self.instructions.handle_key(key);
            }
            return;
        }
        if matches("tui.select.confirm") {
            if let Some(item) = self.list.get_selected_item() {
                match item.value.as_str() {
                    "custom" => {
                        *self.editing.lock().unwrap() = true;
                        self.instructions.set_focused(true);
                    }
                    "summary" => self.submit(true, None),
                    _ => self.submit(false, None),
                }
            }
            return;
        }
        for (id, code) in [
            ("tui.select.up", KeyCode::Up),
            ("tui.select.down", KeyCode::Down),
            ("tui.select.pageUp", KeyCode::PageUp),
            ("tui.select.pageDown", KeyCode::PageDown),
        ] {
            if matches(id) {
                self.list
                    .handle_key(KeyEvent::new(code, KeyModifiers::NONE));
                return;
            }
        }
    }

    fn hint(&self, id: &'static str) -> String {
        let keys = self.bindings.get_keys(id);
        if keys.is_empty() {
            "unbound".into()
        } else {
            keys.iter()
                .map(|key| key.display())
                .collect::<Vec<_>>()
                .join("/")
        }
    }
}

impl Component for TreeSummarySelector {
    fn render(&self, width: usize) -> Vec<String> {
        let editing = *self.editing.lock().unwrap();
        let mut lines = if editing {
            let mut lines = Text::new("Instructions for branch summary", 0, 0).render(width);
            lines.extend(self.instructions.render(width));
            lines.extend(
                Text::new(
                    format!(
                        "{} summarize · {} back",
                        self.hint("tui.input.submit"),
                        self.hint("tui.select.cancel")
                    ),
                    0,
                    0,
                )
                .render(width),
            );
            lines
        } else {
            let mut lines = Text::new("Summarize the branch before leaving?", 0, 0).render(width);
            lines.extend(self.list.render(width));
            lines.extend(
                Text::new(
                    format!(
                        "{} choose · {} return to tree",
                        self.hint("tui.select.confirm"),
                        self.hint("tui.select.cancel")
                    ),
                    0,
                    0,
                )
                .render(width),
            );
            lines
        };
        for line in &mut lines {
            *line = ice_tui::truncate_to_width(line, width, "");
        }
        lines
    }
    fn invalidate(&self) {
        self.instructions.invalidate();
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

pub(super) async fn choose_tree_navigation(
    entry_id: String,
    tree: Arc<TreeSelector>,
    harness: &AgentHarness,
    state: &Arc<TuiState>,
    editor_container: &Arc<Container>,
    editor: &Arc<Editor>,
    tui: &Arc<TuiAltScreen>,
    tx: &mpsc::UnboundedSender<TuiMessage>,
) {
    if *state.status.lock().unwrap() != RunStatus::Idle {
        add_note_message(
            &state.chat_container,
            "Wait for the current run to finish before navigating.",
        );
        tui.request_render(false);
        return;
    }
    match harness.lane("main").snapshot().await {
        Ok(snapshot) if snapshot.leaf_id.as_deref() == Some(entry_id.as_str()) => {
            // Resolve prompt entries before deciding that selection is a no-op:
            // replaying a user/custom prompt actually targets its parent and
            // restores that prompt as an editable draft.
            let replayable = harness
                .session()
                .view("main")
                .get_entry(&entry_id)
                .await
                .ok()
                .flatten()
                .is_some_and(|entry| tree_navigation_target(&entry).1.is_some());
            if !replayable {
                add_note_message(
                    &state.chat_container,
                    "Already at the selected session entry.",
                );
                tui.request_render(false);
                return;
            }
        }
        Err(error) => {
            add_error_message(
                &state.chat_container,
                &format!("Could not read current tree position: {error}"),
            );
            tui.request_render(false);
            return;
        }
        _ => {}
    }
    if crate::settings::load_settings()
        .unwrap_or_default()
        .branch_summary_skipped()
    {
        let _ = tx.send(TuiMessage::CommitTreeNavigation(
            TreeNavigationRequest {
                entry_id,
                summarize: false,
                instructions: None,
            },
            tree,
        ));
    } else {
        open_tree_summary_selector(entry_id, tree, state, editor_container, editor, tui, tx);
    }
}

pub(super) fn open_tree_summary_selector(
    entry_id: String,
    tree: Arc<TreeSelector>,
    state: &Arc<TuiState>,
    editor_container: &Arc<Container>,
    editor: &Arc<Editor>,
    tui: &Arc<TuiAltScreen>,
    tx: &mpsc::UnboundedSender<TuiMessage>,
) {
    let state_sel = state.clone();
    let ec_sel = editor_container.clone();
    let editor_sel = editor.clone();
    let tui_sel = tui.clone();
    let tx_sel = tx.clone();
    let state_cancel = state.clone();
    let ec_cancel = editor_container.clone();
    let editor_cancel = editor.clone();
    let tui_cancel = tui.clone();
    let tx_cancel = tx.clone();
    let tree_sel = tree.clone();
    let selector = Arc::new(TreeSummarySelector::new(
        entry_id,
        ice_tui::get_keybindings().clone(),
        Arc::new(move |request| {
            close_selector(&state_sel, &ec_sel, &editor_sel, &tui_sel);
            let _ = tx_sel.send(TuiMessage::CommitTreeNavigation(request, tree_sel.clone()));
        }),
        Arc::new(move || {
            close_selector(&state_cancel, &ec_cancel, &editor_cancel, &tui_cancel);
            let _ = tx_cancel.send(TuiMessage::OpenTreeAt(tree.clone()));
        }),
    ));
    let router = selector.clone();
    let render = tui.clone();
    open_selector_with_view(
        state,
        editor_container,
        editor,
        tui,
        SelectorView::Custom(Arc::new(move |key| {
            router.handle_key(key);
            render.request_render(false);
        })),
        selector,
        SelectorKind::Tree,
    );
}

/// Reopening a user/custom prompt starts immediately before it and restores its
/// text as a draft. The lane API continues to accept an exact leaf target.
pub(super) fn tree_navigation_target(entry: &Entry) -> (Option<String>, Option<String>) {
    let draft = match entry {
        Entry::Message(message) => match &message.message {
            AgentMessage::User(user) => Some(match &user.content {
                ice_ai::types::UserContent::Text(text) => text.clone(),
                ice_ai::types::UserContent::Blocks(blocks) => blocks
                    .iter()
                    .filter_map(|block| match block {
                        Content::Text(text) => Some(text.text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
            }),
            AgentMessage::Custom(custom) if custom.role == ice_harness::messages::CUSTOM_ROLE => {
                Some(
                    custom
                        .content
                        .iter()
                        .filter_map(|block| match block {
                            Content::Text(text) => Some(text.text.as_str()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("\n"),
                )
            }
            _ => None,
        },
        _ => None,
    };
    let target = if draft.is_some() {
        entry.parent_id().map(str::to_string)
    } else {
        Some(entry.id().to_string())
    };
    (target, draft)
}

/// The images attached to a replayed user prompt, in order.
pub(super) fn tree_prompt_images(entry: &Entry) -> Vec<ice_ai::types::ImageContent> {
    match entry {
        Entry::Message(message) => match &message.message {
            AgentMessage::User(user) => match &user.content {
                ice_ai::types::UserContent::Blocks(blocks) => blocks
                    .iter()
                    .filter_map(|block| match block {
                        Content::Image(image) => Some(image.clone()),
                        _ => None,
                    })
                    .collect(),
                _ => Vec::new(),
            },
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

/// Put a replayed prompt, and its images, back in the editor when the user has
/// not started a draft of their own.
pub(super) fn restore_tree_prompt(
    state: &TuiState,
    draft: Option<&str>,
    images: &[ice_ai::types::ImageContent],
) {
    if let Some(draft) = draft.filter(|_| {
        state.editor.get_text().trim().is_empty() && state.pending_images.lock().unwrap().is_empty()
    }) {
        set_editor_text_caret_at_end(&state.editor, draft);
        state
            .pending_images
            .lock()
            .unwrap()
            .extend(images.iter().cloned());
        state
            .programmatic_editor_write
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Owns both interrupt routing and the run slot. Publish navigation ownership
/// before Working while holding the status lock, so the key thread can never
/// observe a navigation as an ordinary queued-message run. Dropping a cancelled
/// or unwinding future releases both pieces of UI state.
struct TreeNavigationActivity<'a> {
    state: &'a TuiState,
}

impl<'a> TreeNavigationActivity<'a> {
    fn start(state: &'a TuiState) -> Option<Self> {
        let mut status = state.status.lock().unwrap();
        if *status != RunStatus::Idle
            || state
                .tree_navigation_active
                .compare_exchange(
                    false,
                    true,
                    std::sync::atomic::Ordering::AcqRel,
                    std::sync::atomic::Ordering::Acquire,
                )
                .is_err()
        {
            return None;
        }
        *status = RunStatus::Working;
        drop(status);
        let activity = Self { state };
        state.apply_status(RunStatus::Working);
        Some(activity)
    }
}

impl Drop for TreeNavigationActivity<'_> {
    fn drop(&mut self) {
        // Also recover during an unwind; a poisoned status lock must not leave
        // the atomic flag stuck and misroute the next ordinary interrupt.
        let mut status = self
            .state
            .status
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        self.state
            .tree_navigation_active
            .store(false, std::sync::atomic::Ordering::Release);
        *status = RunStatus::Idle;
        drop(status);
        self.state.apply_status(RunStatus::Idle);
    }
}

/// Apply transcript/draft changes only after the lane accepts navigation. The
/// real key interrupt routes directly to lane.abort while activity is held.
pub(super) async fn run_tree_navigation(
    request: &TreeNavigationRequest,
    harness: &AgentHarness,
    lane: &Arc<dyn AgentLane>,
    state: &Arc<TuiState>,
    tui: &Arc<TuiAltScreen>,
) -> bool {
    use ice_harness::agent_harness::NavigationOutcome;
    if *state.status.lock().unwrap() != RunStatus::Idle {
        add_note_message(
            &state.chat_container,
            "Wait for the current run to finish before navigating.",
        );
        tui.request_render(false);
        return false;
    }
    // Claim shared bash/session admission before the first await, and retain
    // it through both backend navigation and the accepted UI replacement.
    let _transition =
        match begin_session_change(&state.chat_container, state, "navigate session tree") {
            Some(transition) => transition,
            None => return true,
        };
    if let Err(error) = state.event_drain.flush().await {
        add_error_message(
            &state.chat_container,
            &format!("Could not settle current session events: {error}"),
        );
        return true;
    }
    let entry = match harness
        .session()
        .view("main")
        .get_entry(&request.entry_id)
        .await
    {
        Ok(Some(entry)) => entry,
        Ok(None) => {
            add_error_message(
                &state.chat_container,
                "The selected tree entry no longer exists.",
            );
            tui.request_render(false);
            return true;
        }
        Err(error) => {
            add_error_message(
                &state.chat_container,
                &format!("Could not read selected tree entry: {error}"),
            );
            tui.request_render(false);
            return true;
        }
    };
    let original_leaf = match lane.snapshot().await {
        Ok(snapshot) => snapshot.leaf_id,
        Err(error) => {
            add_error_message(
                &state.chat_container,
                &format!("Could not read current tree position before navigating: {error}"),
            );
            return true;
        }
    };
    let (target, draft) = tree_navigation_target(&entry);
    let images = tree_prompt_images(&entry);
    let activity = match TreeNavigationActivity::start(state) {
        Some(activity) => activity,
        None => return true,
    };
    if draft.is_some() && target == original_leaf {
        // Replaying a future prompt can already target the active parent (or
        // the empty root). This is a verified editor-only action, not a blanket
        // reinterpretation of backend Declined outcomes. Revalidate after
        // claiming activity, and never call the provider or mutate history.
        match lane.snapshot().await {
            Ok(snapshot) if snapshot.leaf_id == target && snapshot.active.is_none() => {
                restore_tree_prompt(state, draft.as_deref(), &images);
                add_note_message(&state.chat_container, "Selected prompt is ready to edit.");
                tui.request_render(false);
                return false;
            }
            Ok(_) => add_note_message(
                &state.chat_container,
                "The active tree position changed or a run started; select the prompt again after the run finishes.",
            ),
            Err(error) => add_error_message(
                &state.chat_container,
                &format!("Could not verify the prompt replay position: {error}"),
            ),
        }
        tui.request_render(false);
        return true;
    }
    if request.summarize && state.show_terminal_progress {
        state.editor.set_working(Some(WorkingState {
            frame: 0,
            started_at: std::time::Instant::now(),
            message: "Summarizing branch…".into(),
        }));
    }
    tui.request_render(false);
    let result = lane
        .navigate_tree(
            target.as_deref(),
            request.summarize,
            request.instructions.as_deref(),
            None,
        )
        .await;
    drop(activity);
    let reopen = match result {
        Ok(result) => match result.outcome {
            NavigationOutcome::Completed { summary_entry, .. } => {
                replace_session_ui(harness, &state.chat_container, state).await;
                restore_tree_prompt(state, draft.as_deref(), &images);
                add_note_message(
                    &state.chat_container,
                    if summary_entry.is_some() {
                        "Branch summarized. Moved to the selected session entry."
                    } else {
                        "Moved to the selected session entry."
                    },
                );
                false
            }
            NavigationOutcome::Declined { .. } => {
                add_note_message(&state.chat_container, "Tree navigation cancelled.");
                true
            }
            NavigationOutcome::Aborted { .. } => {
                add_note_message(&state.chat_container, "Branch summarization cancelled.");
                true
            }
            NavigationOutcome::Failed { error, .. } => {
                add_error_message(
                    &state.chat_container,
                    &format!("Could not navigate session tree: {}", error.message),
                );
                true
            }
        },
        Err(error) => {
            // A durable lane move/summary can precede a failed acceptance
            // append. Determine the actual branch, never classify by an error
            // string or paint the old transcript as though nothing moved.
            let reopen = match lane.snapshot().await {
                Ok(snapshot) if snapshot.leaf_id != original_leaf => {
                    replace_session_ui(harness, &state.chat_container, state).await;
                    add_note_message(
                        &state.chat_container,
                        "The active branch changed, but navigation finalization failed. The display now reflects the actual branch; retry or reopen the session to recover, and use /tree to inspect the current position.",
                    );
                    // Do not restore the selected prompt draft without an
                    // accepted outcome, or reopen its stale retained tree.
                    false
                }
                Ok(_) => true,
                Err(snapshot_error) => {
                    add_error_message(
                        &state.chat_container,
                        &format!("Could not verify the current tree position: {snapshot_error}. Navigation may have changed the active branch; reopen the session or inspect /tree before continuing."),
                    );
                    false
                }
            };
            add_error_message(
                &state.chat_container,
                &format!("Could not navigate session tree: {error}"),
            );
            reopen
        }
    };
    tui.request_render(false);
    reopen
}

#[cfg(test)]
mod tests {
    use super::super::tests::{test_tui_state, with_isolated_config_dir};
    use super::*;
    use ice_ai::providers::faux::{faux_assistant_message, FauxProvider, FauxScript, FauxStep};
    use ice_ai::types::StopReason;
    use ice_harness::session::types::{EntryBase, MessageEntry};
    use std::sync::atomic::{AtomicBool, Ordering};

    // Mirrors the backend JSONL writer seam, failing only after the durable
    // summary/move has landed. Other filesystem behavior stays in-memory.
    struct AcceptanceAppendFailure {
        inner: Arc<dyn ice_tools::FileSystem>,
        failures: std::sync::atomic::AtomicUsize,
    }

    #[async_trait::async_trait]
    impl ice_tools::FileSystem for AcceptanceAppendFailure {
        fn cwd(&self) -> &std::path::Path {
            self.inner.cwd()
        }
        async fn absolute_path(
            &self,
            path: &str,
            cancel: Option<&tokio_util::sync::CancellationToken>,
        ) -> Result<std::path::PathBuf, ice_tools::error::FileError> {
            self.inner.absolute_path(path, cancel).await
        }
        async fn join_path(
            &self,
            parts: &[&str],
            cancel: Option<&tokio_util::sync::CancellationToken>,
        ) -> Result<std::path::PathBuf, ice_tools::error::FileError> {
            self.inner.join_path(parts, cancel).await
        }
        async fn read_text_file(
            &self,
            path: &str,
            cancel: Option<&tokio_util::sync::CancellationToken>,
        ) -> Result<String, ice_tools::error::FileError> {
            self.inner.read_text_file(path, cancel).await
        }
        async fn read_text_lines(
            &self,
            path: &str,
            max_lines: Option<usize>,
            cancel: Option<&tokio_util::sync::CancellationToken>,
        ) -> Result<Vec<String>, ice_tools::error::FileError> {
            self.inner.read_text_lines(path, max_lines, cancel).await
        }
        async fn read_binary_file(
            &self,
            path: &str,
            cancel: Option<&tokio_util::sync::CancellationToken>,
        ) -> Result<Vec<u8>, ice_tools::error::FileError> {
            self.inner.read_binary_file(path, cancel).await
        }
        async fn write_file(
            &self,
            path: &str,
            content: ice_tools::env::FileContent,
            cancel: Option<&tokio_util::sync::CancellationToken>,
        ) -> Result<(), ice_tools::error::FileError> {
            self.inner.write_file(path, content, cancel).await
        }
        async fn append_file(
            &self,
            path: &str,
            content: ice_tools::env::FileContent,
            cancel: Option<&tokio_util::sync::CancellationToken>,
        ) -> Result<(), ice_tools::error::FileError> {
            if let ice_tools::env::FileContent::Text(text) = &content {
                use ice_harness::session::types::SessionMutation;
                let should_fail = matches!(
                    ice_harness::session::jsonl::parse_mutation(text),
                    Ok(SessionMutation::Record {
                        record: ice_harness::session::types::LaneRecord::OperationFinished(_),
                        ..
                    })
                );
                if should_fail && self.failures.fetch_add(1, Ordering::SeqCst) == 0 {
                    return Err(ice_tools::error::FileError::new(
                        ice_tools::error::FileErrorCode::Unknown,
                        "injected navigation acceptance failure",
                    ));
                }
            }
            self.inner.append_file(path, content, cancel).await
        }
        async fn rename_file(
            &self,
            source: &str,
            destination: &str,
            cancel: Option<&tokio_util::sync::CancellationToken>,
        ) -> Result<(), ice_tools::error::FileError> {
            self.inner.rename_file(source, destination, cancel).await
        }
        async fn file_info(
            &self,
            path: &str,
            cancel: Option<&tokio_util::sync::CancellationToken>,
        ) -> Result<ice_tools::env::FileInfo, ice_tools::error::FileError> {
            self.inner.file_info(path, cancel).await
        }
        async fn list_dir(
            &self,
            path: &str,
            cancel: Option<&tokio_util::sync::CancellationToken>,
        ) -> Result<Vec<ice_tools::env::FileInfo>, ice_tools::error::FileError> {
            self.inner.list_dir(path, cancel).await
        }
        async fn canonical_path(
            &self,
            path: &str,
            cancel: Option<&tokio_util::sync::CancellationToken>,
        ) -> Result<std::path::PathBuf, ice_tools::error::FileError> {
            self.inner.canonical_path(path, cancel).await
        }
        async fn exists(
            &self,
            path: &str,
            cancel: Option<&tokio_util::sync::CancellationToken>,
        ) -> Result<bool, ice_tools::error::FileError> {
            self.inner.exists(path, cancel).await
        }
        async fn create_dir(
            &self,
            path: &str,
            recursive: bool,
            cancel: Option<&tokio_util::sync::CancellationToken>,
        ) -> Result<(), ice_tools::error::FileError> {
            self.inner.create_dir(path, recursive, cancel).await
        }
        async fn remove(
            &self,
            path: &str,
            recursive: bool,
            force: bool,
            cancel: Option<&tokio_util::sync::CancellationToken>,
        ) -> Result<(), ice_tools::error::FileError> {
            self.inner.remove(path, recursive, force, cancel).await
        }
        async fn create_temp_dir(
            &self,
            prefix: Option<&str>,
            cancel: Option<&tokio_util::sync::CancellationToken>,
        ) -> Result<std::path::PathBuf, ice_tools::error::FileError> {
            self.inner.create_temp_dir(prefix, cancel).await
        }
        async fn create_temp_file(
            &self,
            prefix: &str,
            suffix: &str,
            cancel: Option<&tokio_util::sync::CancellationToken>,
        ) -> Result<std::path::PathBuf, ice_tools::error::FileError> {
            self.inner.create_temp_file(prefix, suffix, cancel).await
        }
        async fn cleanup(&self) {
            self.inner.cleanup().await
        }
    }

    fn terminal() -> Arc<TuiAltScreen> {
        Arc::new(TuiAltScreen::new(
            Box::new(ProcessTerminal::new()),
            true,
            None,
        ))
    }

    fn route(state: &TuiState, code: KeyCode) {
        let view = state
            .active_selector
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .0
            .clone();
        view.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    async fn navigation_fixture(script: FauxScript) -> (AgentHarness, String, String) {
        let provider = FauxProvider::new(script);
        let session = ice_harness::types::AgentHarnessOptions::default().session;
        let root = session
            .append_message(AgentMessage::Assistant(Box::new(faux_assistant_message(
                "common root",
                StopReason::Stop,
            ))))
            .await
            .unwrap();
        let source = session
            .append_message(user_message_with_images("abandoned work", vec![]))
            .await
            .unwrap();
        let mut options = ice_harness::types::AgentHarnessOptions {
            model: provider.default_model().clone(),
            models: vec![provider],
            session,
            allow_existing_session: true,
            ..Default::default()
        };
        options.retry.enabled = false;
        (AgentHarness::create(options).await.unwrap(), root, source)
    }

    #[test]
    fn tree_summary_back_restores_exact_tree_search_selection_and_draft() {
        with_isolated_config_dir(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async {
                    let (harness, root, _) = navigation_fixture(FauxScript::new()).await;
                    let state = test_tui_state();
                    state.editor.set_text("source draft 猫");
                    let tui = terminal();
                    let editor_container = Arc::new(Container::new());
                    let (tx, mut rx) = mpsc::unbounded_channel();
                    open_tree_selector(
                        &harness,
                        &state,
                        &editor_container,
                        &state.editor,
                        &tui,
                        &state.chat_container,
                        &tx,
                    )
                    .await;
                    for ch in "common".chars() {
                        route(&state, KeyCode::Char(ch));
                    }
                    route(&state, KeyCode::Enter);
                    let TuiMessage::NavigateTree(id, tree) = rx.try_recv().unwrap() else {
                        panic!("tree selection")
                    };
                    assert_eq!(id, root);
                    let before = tree.render(80);
                    choose_tree_navigation(
                        id,
                        tree.clone(),
                        &harness,
                        &state,
                        &editor_container,
                        &state.editor,
                        &tui,
                        &tx,
                    )
                    .await;
                    route(&state, KeyCode::Down);
                    route(&state, KeyCode::Down);
                    route(&state, KeyCode::Enter);
                    route(&state, KeyCode::Char('x'));
                    route(&state, KeyCode::Esc); // custom editor -> choices
                    assert!(rx.try_recv().is_err());
                    route(&state, KeyCode::Esc); // choices -> same tree view
                    let TuiMessage::OpenTreeAt(returned) = rx.try_recv().unwrap() else {
                        panic!("return to tree")
                    };
                    assert!(Arc::ptr_eq(&tree, &returned));
                    assert_eq!(returned.render(80), before);
                    reopen_tree_selector(returned, &state, &editor_container, &state.editor, &tui);
                    route(&state, KeyCode::Enter);
                    let TuiMessage::NavigateTree(id, _) = rx.try_recv().unwrap() else {
                        panic!("restored selection")
                    };
                    assert_eq!(id, root);
                    assert_eq!(state.editor.get_text(), "source draft 猫");
                    harness.close().await.unwrap();
                });
        });
    }

    #[test]
    fn tree_summary_skip_prompt_dispatches_plain_navigation_and_invalid_settings_do_not_panic() {
        with_isolated_config_dir(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async {
                    let (harness, root, _) = navigation_fixture(FauxScript::new()).await;
                    let state = test_tui_state();
                    let tui = terminal();
                    let editor_container = Arc::new(Container::new());
                    let (tx, mut rx) = mpsc::unbounded_channel();
                    open_tree_selector(
                        &harness,
                        &state,
                        &editor_container,
                        &state.editor,
                        &tui,
                        &state.chat_container,
                        &tx,
                    )
                    .await;
                    route(&state, KeyCode::Up);
                    route(&state, KeyCode::Enter);
                    let TuiMessage::NavigateTree(id, tree) = rx.try_recv().unwrap() else {
                        panic!("tree selection")
                    };
                    assert_eq!(id, root);
                    let path = crate::config::settings_path().unwrap();
                    std::fs::write(&path, r#"{"branchSummary":{"skipPrompt":true}}"#).unwrap();
                    choose_tree_navigation(
                        id.clone(),
                        tree.clone(),
                        &harness,
                        &state,
                        &editor_container,
                        &state.editor,
                        &tui,
                        &tx,
                    )
                    .await;
                    let TuiMessage::CommitTreeNavigation(request, _) = rx.try_recv().unwrap()
                    else {
                        panic!("skip chooser")
                    };
                    assert!(!request.summarize);
                    assert!(request.instructions.is_none());
                    std::fs::write(path, "invalid settings").unwrap();
                    choose_tree_navigation(
                        id,
                        tree,
                        &harness,
                        &state,
                        &editor_container,
                        &state.editor,
                        &tui,
                        &tx,
                    )
                    .await;
                    assert!(state.selector_open());
                    assert!(rx.try_recv().is_err());
                    harness.close().await.unwrap();
                });
        });
    }

    #[test]
    fn tree_summary_real_outcomes_preserve_draft_and_render_only_completed_summary() {
        with_isolated_config_dir(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async {
                    for reason in [StopReason::Error, StopReason::Aborted, StopReason::Stop] {
                        let contexts = Arc::new(Mutex::new(String::new()));
                        let captured = contexts.clone();
                        let script = FauxScript::new();
                        script.set_responses(vec![FauxStep::factory(move |context| {
                            *captured.lock().unwrap() = serde_json::to_string(context).unwrap();
                            let mut reply =
                                faux_assistant_message("Completed branch summary", reason);
                            if reason == StopReason::Error {
                                reply.error_message = Some("summary failure".into());
                            }
                            reply
                        })]);
                        let (harness, root, source) = navigation_fixture(script).await;
                        let state = test_tui_state();
                        state.editor.set_text("edited source draft 猫");
                        state
                            .pending_images
                            .lock()
                            .unwrap()
                            .push(ice_ai::types::ImageContent {
                                kind: ice_ai::types::ImageContentType,
                                data: "test".into(),
                                mime_type: "image/png".into(),
                            });
                        state.chat_container.add_child(Arc::new(Text::new(
                            "ORIGINAL TRANSCRIPT",
                            0,
                            0,
                        )));
                        let request = TreeNavigationRequest {
                            entry_id: root,
                            summarize: true,
                            instructions: Some("Keep test decisions".into()),
                        };
                        let reopen = run_tree_navigation(
                            &request,
                            &harness,
                            &harness.lane("main"),
                            &state,
                            &terminal(),
                        )
                        .await;
                        assert!(*state.status.lock().unwrap() == RunStatus::Idle);
                        assert!(!state.tree_navigation_active.load(Ordering::Acquire));
                        assert_eq!(state.editor.get_text(), "edited source draft 猫");
                        assert_eq!(state.pending_images.lock().unwrap().len(), 1);
                        let transcript = strip_ansi(&state.chat_container.render(100).join("\n"));
                        if reason == StopReason::Stop {
                            assert!(!reopen);
                            assert!(!transcript.contains("ORIGINAL TRANSCRIPT"));
                            assert!(
                                transcript.contains("Completed branch summary"),
                                "{transcript}"
                            );
                            assert!(transcript.contains("Branch summarized"));
                        } else {
                            assert!(reopen);
                            assert!(transcript.contains("ORIGINAL TRANSCRIPT"));
                            assert!(!transcript.contains("Branch summarized"));
                            assert_eq!(
                                harness.session().get_leaf_id().await.unwrap(),
                                Some(source)
                            );
                        }
                        assert!(contexts.lock().unwrap().contains("Keep test decisions"));
                        harness.close().await.unwrap();
                    }
                });
        });
    }

    #[test]
    fn tree_summary_key_interrupt_aborts_inflight_summary_without_dequeuing_source() {
        with_isolated_config_dir(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async {
                    let started = Arc::new(tokio::sync::Notify::new());
                    let notify = started.clone();
                    let script = FauxScript::new().with_tokens_per_second(1.0);
                    script.set_responses(vec![FauxStep::factory(move |_| {
                        notify.notify_one();
                        faux_assistant_message(
                            "genuinely delayed summary ".repeat(100),
                            StopReason::Stop,
                        )
                    })]);
                    let (harness, root, source) = navigation_fixture(script).await;
                    let harness = Arc::new(harness);
                    let lane = harness.lane("main");
                    let state = test_tui_state();
                    state.editor.set_text("original editable draft 猫");
                    let image = ice_ai::types::ImageContent {
                        kind: ice_ai::types::ImageContentType,
                        data: "original-image".into(),
                        mime_type: "image/png".into(),
                    };
                    state.pending_images.lock().unwrap().push(image.clone());
                    state
                        .chat_container
                        .add_child(Arc::new(Text::new("SOURCE TRANSCRIPT", 0, 0)));
                    let tui = terminal();
                    let editor_container = Arc::new(Container::new());
                    let (tx, mut rx) = mpsc::unbounded_channel();
                    open_tree_selector(
                        &harness,
                        &state,
                        &editor_container,
                        &state.editor,
                        &tui,
                        &state.chat_container,
                        &tx,
                    )
                    .await;
                    route(&state, KeyCode::Up);
                    route(&state, KeyCode::Enter);
                    let TuiMessage::NavigateTree(id, tree) = rx.try_recv().unwrap() else {
                        panic!("selected target")
                    };
                    assert_eq!(id, root);
                    let before_tree = tree.render(80);
                    let running = {
                        let harness = harness.clone();
                        let lane = lane.clone();
                        let state = state.clone();
                        let tui = tui.clone();
                        tokio::spawn(async move {
                            let request = TreeNavigationRequest {
                                entry_id: id,
                                summarize: true,
                                instructions: None,
                            };
                            run_tree_navigation(&request, &harness, &lane, &state, &tui).await
                        })
                    };
                    tokio::time::timeout(std::time::Duration::from_secs(5), started.notified())
                        .await
                        .unwrap();
                    assert!(state.tree_navigation_active.load(Ordering::Acquire));
                    assert!(*state.status.lock().unwrap() == RunStatus::Working);
                    // Queue while the summary is actually generating. A prompt-run
                    // interrupt would consume these and overwrite the source draft.
                    lane.steer(user_message_with_images("queued steering", vec![]))
                        .await
                        .unwrap();
                    lane.follow_up(user_message_with_images("queued follow-up", vec![]))
                        .await
                        .unwrap();
                    let queued = lane.queued_messages().await.unwrap();
                    assert!(!queued.is_empty());
                    let bindings = ice_tui::Keybindings::new();
                    assert!(!dispatch_run_interrupt(
                        &KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                        &bindings,
                        lane.clone(),
                        state.editor.clone(),
                        state.clone(),
                        tui.clone(),
                    ));
                    // This is the exact key handler called by the real TUI loop,
                    // not a helper-only direct lane.abort invocation.
                    assert!(dispatch_run_interrupt(
                        &KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
                        &bindings,
                        lane.clone(),
                        state.editor.clone(),
                        state.clone(),
                        tui.clone(),
                    ));
                    let reopen = tokio::time::timeout(std::time::Duration::from_secs(5), running)
                        .await
                        .unwrap()
                        .unwrap();
                    assert!(reopen);
                    assert!(!state.tree_navigation_active.load(Ordering::Acquire));
                    assert!(*state.status.lock().unwrap() == RunStatus::Idle);
                    assert_eq!(state.editor.get_text(), "original editable draft 猫");
                    assert_eq!(*state.pending_images.lock().unwrap(), vec![image]);
                    assert_eq!(lane.queued_messages().await.unwrap(), queued);
                    assert_eq!(harness.session().get_leaf_id().await.unwrap(), Some(source));
                    let transcript = strip_ansi(&state.chat_container.render(100).join("\n"));
                    assert!(transcript.contains("SOURCE TRANSCRIPT"));
                    assert!(
                        transcript.contains("Branch summarization cancelled"),
                        "{transcript}"
                    );
                    assert!(!transcript.contains("Branch summarized"));
                    let records = harness
                        .session()
                        .find_records(&ice_harness::session::types::RecordQuery::default())
                        .await
                        .unwrap();
                    assert!(records.iter().any(|record| matches!(record,
                    ice_harness::session::types::LaneRecord::OperationFinished(finished)
                    if finished.outcome == ice_harness::session::types::OperationOutcome::Aborted
                )));
                    assert_eq!(tree.render(80), before_tree);
                    reopen_tree_selector(tree, &state, &editor_container, &state.editor, &tui);
                    route(&state, KeyCode::Enter);
                    let TuiMessage::NavigateTree(id, _) = rx.try_recv().unwrap() else {
                        panic!("restored target")
                    };
                    assert_eq!(id, root);
                    harness.close().await.unwrap();
                });
        });
    }

    #[test]
    fn tree_summary_navigation_renders_only_active_branch_and_updates_copy_answer() {
        with_isolated_config_dir(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async {
                    for summarize in [false, true] {
                        let script = FauxScript::new().with_text("retained branch work");
                        let provider = FauxProvider::new(script);
                        let session = ice_harness::types::AgentHarnessOptions::default().session;
                        let root = session
                            .append_message(AgentMessage::Assistant(Box::new(
                                faux_assistant_message("kept copy answer", StopReason::Stop),
                            )))
                            .await
                            .unwrap();
                        session
                            .append_message(user_message_with_images(
                                "abandoned future question",
                                vec![],
                            ))
                            .await
                            .unwrap();
                        session
                            .append_message(AgentMessage::Assistant(Box::new(
                                faux_assistant_message("abandoned future answer", StopReason::Stop),
                            )))
                            .await
                            .unwrap();
                        let harness =
                            AgentHarness::create(ice_harness::types::AgentHarnessOptions {
                                model: provider.default_model().clone(),
                                models: vec![provider],
                                session,
                                allow_existing_session: true,
                                ..Default::default()
                            })
                            .await
                            .unwrap();
                        let state = test_tui_state();
                        *state.last_assistant_text.lock().unwrap() =
                            "abandoned future answer".into();
                        let request = TreeNavigationRequest {
                            entry_id: root,
                            summarize,
                            instructions: None,
                        };
                        assert!(
                            !run_tree_navigation(
                                &request,
                                &harness,
                                &harness.lane("main"),
                                &state,
                                &terminal()
                            )
                            .await
                        );
                        let transcript = strip_ansi(&state.chat_container.render(100).join("\n"));
                        assert!(transcript.contains("kept copy answer"), "{transcript}");
                        assert!(
                            !transcript.contains("abandoned future question"),
                            "{transcript}"
                        );
                        assert!(
                            !transcript.contains("abandoned future answer"),
                            "{transcript}"
                        );
                        assert_eq!(
                            *state.last_assistant_text.lock().unwrap(),
                            "kept copy answer"
                        );
                        if summarize {
                            assert!(transcript.contains("retained branch work"));
                        }
                        assert!(!state.tree_navigation_active.load(Ordering::Acquire));
                        harness.close().await.unwrap();
                    }
                });
        });
    }

    #[test]
    fn tree_summary_current_user_leaf_replays_parent_and_restores_editable_prompt() {
        with_isolated_config_dir(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async {
                    let (harness, root, source) = navigation_fixture(FauxScript::new()).await;
                    let state = test_tui_state();
                    let tui = terminal();
                    let editor_container = Arc::new(Container::new());
                    let (tx, mut rx) = mpsc::unbounded_channel();
                    open_tree_selector(
                        &harness,
                        &state,
                        &editor_container,
                        &state.editor,
                        &tui,
                        &state.chat_container,
                        &tx,
                    )
                    .await;
                    route(&state, KeyCode::Enter);
                    let TuiMessage::NavigateTree(id, tree) = rx.try_recv().unwrap() else {
                        panic!("selected user leaf")
                    };
                    assert_eq!(id, source);
                    choose_tree_navigation(
                        id,
                        tree,
                        &harness,
                        &state,
                        &editor_container,
                        &state.editor,
                        &tui,
                        &tx,
                    )
                    .await;
                    assert!(
                        state.selector_open(),
                        "a user leaf resolves to its parent, not a no-op"
                    );
                    route(&state, KeyCode::Enter); // no summary
                    let TuiMessage::CommitTreeNavigation(request, _) = rx.try_recv().unwrap()
                    else {
                        panic!("accepted replay")
                    };
                    assert!(
                        !run_tree_navigation(
                            &request,
                            &harness,
                            &harness.lane("main"),
                            &state,
                            &tui
                        )
                        .await
                    );
                    assert_eq!(harness.session().get_leaf_id().await.unwrap(), Some(root));
                    assert_eq!(state.editor.get_text(), "abandoned work");
                    let transcript = strip_ansi(&state.chat_container.render(100).join("\n"));
                    assert!(!transcript.contains("abandoned work"));
                    harness.close().await.unwrap();
                });
        });
    }

    async fn assert_already_at_parent_prompt_replay(root_prompt: bool) {
        for custom in [false, true] {
            let provider = FauxProvider::new(FauxScript::new().with_text("must not be called"));
            let session = ice_harness::types::AgentHarnessOptions::default().session;
            let parent = if root_prompt {
                None
            } else {
                Some(
                    session
                        .append_message(AgentMessage::Assistant(Box::new(faux_assistant_message(
                            "active parent answer",
                            StopReason::Stop,
                        ))))
                        .await
                        .unwrap(),
                )
            };
            let prompt = "replay selected prompt\n猫";
            let message = if custom {
                AgentMessage::Custom(ice_agent::CustomMessage::new(
                    ice_harness::messages::CUSTOM_ROLE,
                    vec![Content::text(prompt)],
                    serde_json::json!({}),
                    0,
                ))
            } else {
                user_message_with_images(prompt, vec![])
            };
            let selected = session.append_message(message).await.unwrap();
            let harness = AgentHarness::create(ice_harness::types::AgentHarnessOptions {
                model: provider.default_model().clone(),
                models: vec![provider.clone()],
                session,
                allow_existing_session: true,
                ..Default::default()
            })
            .await
            .unwrap();
            assert!(matches!(
                harness
                    .navigate_tree(parent.as_deref(), false, None, None)
                    .await
                    .unwrap()
                    .outcome,
                ice_harness::agent_harness::NavigationOutcome::Completed { .. }
            ));
            harness
                .steer(user_message_with_images("keep queued steering", vec![]))
                .await
                .unwrap();
            harness
                .follow_up(user_message_with_images("keep queued follow-up", vec![]))
                .await
                .unwrap();
            let queued = harness.queued_messages().await.unwrap();
            let entries_before = serde_json::to_value(
                harness
                    .session()
                    .find_entries(&EntryQuery::default())
                    .await
                    .unwrap(),
            )
            .unwrap();
            let records_before = serde_json::to_value(
                harness
                    .session()
                    .find_records(&ice_harness::session::types::RecordQuery::default())
                    .await
                    .unwrap(),
            )
            .unwrap();
            for draft_case in 0..4 {
                let state = test_tui_state();
                match draft_case {
                    1 => state.editor.set_text("keep edited source draft"),
                    2 => state
                        .editor
                        .handle_paste(&"preserve expanded folded draft\n".repeat(20)),
                    3 => state
                        .pending_images
                        .lock()
                        .unwrap()
                        .push(ice_ai::types::ImageContent {
                            kind: ice_ai::types::ImageContentType,
                            data: "pending-original".into(),
                            mime_type: "image/png".into(),
                        }),
                    _ => {}
                }
                let displayed_before = state.editor.get_text();
                let expanded_before = state.editor.get_expanded_text();
                if draft_case == 2 {
                    assert_ne!(
                        displayed_before, expanded_before,
                        "fixture must use an actual folded paste"
                    );
                }
                let images_before = state.pending_images.lock().unwrap().clone();
                state.chat_container.add_child(Arc::new(Text::new(
                    "UNCHANGED ACTIVE TRANSCRIPT",
                    0,
                    0,
                )));
                *state.last_assistant_text.lock().unwrap() = "keep current copy state".into();
                let tui = terminal();
                let editor_container = Arc::new(Container::new());
                let (tx, mut rx) = mpsc::unbounded_channel();
                open_tree_selector(
                    &harness,
                    &state,
                    &editor_container,
                    &state.editor,
                    &tui,
                    &state.chat_container,
                    &tx,
                )
                .await;
                if parent.is_some() {
                    route(&state, KeyCode::Down);
                }
                route(&state, KeyCode::Enter);
                let TuiMessage::NavigateTree(id, tree) = rx.try_recv().unwrap() else {
                    panic!("select future prompt")
                };
                assert_eq!(id, selected);
                choose_tree_navigation(
                    id,
                    tree,
                    &harness,
                    &state,
                    &editor_container,
                    &state.editor,
                    &tui,
                    &tx,
                )
                .await;
                route(&state, KeyCode::Enter); // No summary
                let TuiMessage::CommitTreeNavigation(request, _) = rx.try_recv().unwrap() else {
                    panic!("accept editor-only replay")
                };
                assert!(!request.summarize);
                assert!(
                    !run_tree_navigation(&request, &harness, &harness.lane("main"), &state, &tui)
                        .await,
                    "verified no-move replay should finish instead of reopening a cancelled tree"
                );
                assert_eq!(harness.snapshot().await.unwrap().leaf_id, parent);
                if draft_case == 0 {
                    assert_eq!(state.editor.get_expanded_text(), prompt);
                } else {
                    assert_eq!(state.editor.get_text(), displayed_before);
                    assert_eq!(state.editor.get_expanded_text(), expanded_before);
                }
                assert_eq!(*state.pending_images.lock().unwrap(), images_before);
                assert_eq!(
                    *state.last_assistant_text.lock().unwrap(),
                    "keep current copy state"
                );
                let transcript = strip_ansi(&state.chat_container.render(100).join("\n"));
                assert!(transcript.contains("UNCHANGED ACTIVE TRANSCRIPT"));
                assert!(transcript.contains("Selected prompt is ready to edit"));
                assert!(!transcript.contains("Tree navigation cancelled"));
                assert_eq!(harness.queued_messages().await.unwrap(), queued);
                assert_eq!(provider.state().call_count.load(Ordering::SeqCst), 0);
                assert_eq!(
                    serde_json::to_value(
                        harness
                            .session()
                            .find_entries(&EntryQuery::default())
                            .await
                            .unwrap()
                    )
                    .unwrap(),
                    entries_before
                );
                assert_eq!(
                    serde_json::to_value(
                        harness
                            .session()
                            .find_records(&ice_harness::session::types::RecordQuery::default())
                            .await
                            .unwrap()
                    )
                    .unwrap(),
                    records_before
                );
                assert!(!state.tree_navigation_active.load(Ordering::Acquire));
                assert!(*state.status.lock().unwrap() == RunStatus::Idle);
                assert!(
                    state.begin_user_bash().is_some(),
                    "editor-only replay releases transition admission"
                );
                state.finish_user_bash();
            }
            harness.close().await.unwrap();
        }
    }

    #[test]
    fn tree_summary_future_prompt_at_active_parent_restores_draft_without_navigation() {
        with_isolated_config_dir(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(assert_already_at_parent_prompt_replay(false));
        });
    }

    #[test]
    fn tree_summary_root_prompt_at_empty_leaf_restores_draft_without_navigation() {
        with_isolated_config_dir(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(assert_already_at_parent_prompt_replay(true));
        });
    }

    #[test]
    fn tree_summary_non_prompt_declined_is_still_cancelled_without_draft_changes() {
        with_isolated_config_dir(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async {
                    let (harness, parent, _) = navigation_fixture(FauxScript::new()).await;
                    harness
                        .navigate_tree(Some(&parent), false, None, None)
                        .await
                        .unwrap();
                    let state = test_tui_state();
                    state.editor.set_text("unchanged original draft");
                    let request = TreeNavigationRequest {
                        entry_id: parent,
                        summarize: false,
                        instructions: None,
                    };
                    assert!(
                        run_tree_navigation(
                            &request,
                            &harness,
                            &harness.lane("main"),
                            &state,
                            &terminal()
                        )
                        .await
                    );
                    assert_eq!(state.editor.get_text(), "unchanged original draft");
                    let transcript = strip_ansi(&state.chat_container.render(100).join("\n"));
                    assert!(transcript.contains("Tree navigation cancelled"));
                    assert!(!transcript.contains("ready to edit"));
                    assert!(!state.tree_navigation_active.load(Ordering::Acquire));
                    harness.close().await.unwrap();
                });
        });
    }

    #[test]
    fn tree_summary_rejects_bash_during_drain_provider_and_accepted_ui_replacement() {
        with_isolated_config_dir(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(4)
                .enable_all()
                .build()
                .unwrap()
                .block_on(async {
                    let started = Arc::new(tokio::sync::Notify::new());
                    let notify = started.clone();
                    let (release, wait) = std::sync::mpsc::channel();
                    let wait = Mutex::new(wait);
                    let script = FauxScript::new();
                    script.set_responses(vec![FauxStep::factory(move |_| {
                        notify.notify_one();
                        wait.lock()
                            .unwrap()
                            .recv_timeout(std::time::Duration::from_secs(5))
                            .unwrap();
                        faux_assistant_message("accepted branch summary", StopReason::Stop)
                    })]);
                    let (harness, root, source) = navigation_fixture(script).await;
                    let harness = Arc::new(harness);
                    let state = test_tui_state();
                    *state.last_assistant_text.lock().unwrap() = "stale copy answer".into();
                    let mut requests = state.event_drain.install();
                    let running = {
                        let harness = harness.clone();
                        let state = state.clone();
                        tokio::spawn(async move {
                            let request = TreeNavigationRequest {
                                entry_id: root,
                                summarize: true,
                                instructions: None,
                            };
                            run_tree_navigation(
                                &request,
                                &harness,
                                &harness.lane("main"),
                                &state,
                                &terminal(),
                            )
                            .await
                        })
                    };
                    // Deliberately withhold the real event-drain acknowledgement:
                    // bash admission must already be excluded before this await.
                    let flush =
                        tokio::time::timeout(std::time::Duration::from_secs(5), requests.recv())
                            .await
                            .unwrap()
                            .unwrap();
                    let AgentEventDrainRequest::Flush(acknowledge) = flush;
                    assert!(state.begin_user_bash().is_none());
                    assert_eq!(
                        harness.session().get_leaf_id().await.unwrap(),
                        Some(source.clone())
                    );
                    acknowledge.send(()).unwrap();
                    tokio::time::timeout(std::time::Duration::from_secs(5), started.notified())
                        .await
                        .unwrap();
                    assert!(state.begin_user_bash().is_none());
                    // Hold the actual /copy state lock: accepted UI replacement
                    // cannot finish its final assistant-text update until released.
                    let copy_block = state.last_assistant_text.lock().unwrap();
                    release.send(()).unwrap();
                    tokio::time::timeout(std::time::Duration::from_secs(5), async {
                        while harness.session().get_leaf_id().await.unwrap().as_deref()
                            == Some(source.as_str())
                        {
                            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                        }
                        while *state.status.lock().unwrap() != RunStatus::Idle {
                            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                        }
                    })
                    .await
                    .unwrap();
                    assert!(
                        !running.is_finished(),
                        "UI completion is deliberately blocked"
                    );
                    assert_eq!(copy_block.as_str(), "stale copy answer");
                    assert!(
                        state.begin_user_bash().is_none(),
                        "backend completion must not release UI-transition admission"
                    );
                    drop(copy_block);
                    assert!(
                        !tokio::time::timeout(std::time::Duration::from_secs(5), running)
                            .await
                            .unwrap()
                            .unwrap()
                    );
                    assert_eq!(*state.last_assistant_text.lock().unwrap(), "common root");
                    assert!(strip_ansi(&state.chat_container.render(100).join("\n"))
                        .contains("accepted branch summary"));
                    assert!(
                        state.begin_user_bash().is_some(),
                        "UI completion releases shared admission"
                    );
                    state.finish_user_bash();
                    harness.close().await.unwrap();
                });
        });
    }

    #[test]
    fn tree_summary_post_commit_acceptance_failure_repaints_actual_branch_without_draft_restore() {
        with_isolated_config_dir(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async {
                    use ice_harness::session::jsonl::{
                        JsonlSessionCreateOptions, JsonlSessionRepo, JsonlSessionRepoOptions,
                    };
                    use ice_harness::session::memory::{CounterIdGenerator, FakeClock};
                    use ice_harness::session::types::EntryQuery;
                    for summarize in [false, true] {
                        for original_draft in ["", "original source draft 猫"] {
                            let fs = Arc::new(AcceptanceAppendFailure {
                                inner: Arc::new(ice_tools::InMemoryExecutionEnv::new()),
                                failures: std::sync::atomic::AtomicUsize::new(0),
                            });
                            let repo = JsonlSessionRepo::new(
                                JsonlSessionRepoOptions {
                                    fs: fs.clone(),
                                    sessions_root: "/sessions".into(),
                                    clock: Arc::new(FakeClock::new()),
                                    ids: Arc::new(CounterIdGenerator::new()),
                                },
                                "/project".into(),
                            );
                            let storage = repo
                                .create_typed(&JsonlSessionCreateOptions {
                                    id: Some("cli-acceptance-failure".into()),
                                    cwd: "/project".into(),
                                    parent_session_id: None,
                                    metadata: None,
                                })
                                .await
                                .unwrap();
                            let session =
                                ice_harness::session::Session::new(Arc::new(storage), None);
                            session
                                .append_message(AgentMessage::Assistant(Box::new(
                                    faux_assistant_message(
                                        "actual branch copy answer",
                                        StopReason::Stop,
                                    ),
                                )))
                                .await
                                .unwrap();
                            let selected_prompt = session
                                .append_message(user_message_with_images(
                                    "selected prompt must not replace source draft",
                                    vec![],
                                ))
                                .await
                                .unwrap();
                            let source = session
                                .append_message(AgentMessage::Assistant(Box::new(
                                    faux_assistant_message(
                                        "abandoned source answer",
                                        StopReason::Stop,
                                    ),
                                )))
                                .await
                                .unwrap();
                            let provider = FauxProvider::new(
                                FauxScript::new().with_text("durable branch summary"),
                            );
                            let harness =
                                AgentHarness::create(ice_harness::types::AgentHarnessOptions {
                                    model: provider.default_model().clone(),
                                    models: vec![provider],
                                    session,
                                    allow_existing_session: true,
                                    ..Default::default()
                                })
                                .await
                                .unwrap();
                            let state = test_tui_state();
                            state.editor.set_text(original_draft);
                            if !original_draft.is_empty() {
                                state.pending_images.lock().unwrap().push(
                                    ice_ai::types::ImageContent {
                                        kind: ice_ai::types::ImageContentType,
                                        data: "original".into(),
                                        mime_type: "image/png".into(),
                                    },
                                );
                            }
                            let original_images = state.pending_images.lock().unwrap().clone();
                            state.chat_container.add_child(Arc::new(Text::new(
                                "OLD SOURCE TRANSCRIPT",
                                0,
                                0,
                            )));
                            *state.last_assistant_text.lock().unwrap() =
                                "abandoned source answer".into();
                            let request = TreeNavigationRequest {
                                entry_id: selected_prompt,
                                summarize,
                                instructions: None,
                            };
                            let reopen = run_tree_navigation(
                                &request,
                                &harness,
                                &harness.lane("main"),
                                &state,
                                &terminal(),
                            )
                            .await;
                            assert!(
                                !reopen,
                                "never reopen the stale retained tree after a durable move"
                            );
                            let actual = harness.snapshot().await.unwrap().leaf_id;
                            assert_ne!(actual, Some(source));
                            assert_eq!(state.editor.get_text(), original_draft);
                            assert_eq!(*state.pending_images.lock().unwrap(), original_images);
                            assert_eq!(
                                *state.last_assistant_text.lock().unwrap(),
                                "actual branch copy answer"
                            );
                            let transcript =
                                strip_ansi(&state.chat_container.render(100).join("\n"));
                            assert!(
                                transcript.contains("actual branch copy answer"),
                                "{transcript}"
                            );
                            assert!(
                                !transcript.contains("OLD SOURCE TRANSCRIPT"),
                                "{transcript}"
                            );
                            assert!(
                                !transcript.contains("abandoned source answer"),
                                "{transcript}"
                            );
                            assert!(transcript.contains("active branch changed"), "{transcript}");
                            assert!(transcript.contains("finalization failed"), "{transcript}");
                            assert!(
                                transcript
                                    .split_whitespace()
                                    .collect::<Vec<_>>()
                                    .join(" ")
                                    .contains("injected navigation acceptance failure"),
                                "{transcript}"
                            );
                            assert!(
                                !transcript.contains("Branch summarized."),
                                "must not announce accepted success: {transcript}"
                            );
                            if summarize {
                                assert!(transcript.contains("durable branch summary"));
                            }
                            assert_eq!(fs.failures.load(Ordering::SeqCst), 1);
                            assert_eq!(
                                harness
                                    .session()
                                    .find_open_operations("main", None)
                                    .await
                                    .unwrap()
                                    .len(),
                                1
                            );
                            assert!(!state.tree_navigation_active.load(Ordering::Acquire));
                            assert!(state.begin_user_bash().is_some());
                            state.finish_user_bash();
                            // The original records remain durable for recovery.
                            assert_eq!(
                                harness
                                    .session()
                                    .find_entries(&EntryQuery::default())
                                    .await
                                    .unwrap()
                                    .len(),
                                if summarize { 4 } else { 3 }
                            );
                            harness.close().await.unwrap();
                        }
                    }
                });
        });
    }

    #[test]
    fn tree_summary_activity_drop_recovers_on_future_cancellation_and_unwind() {
        let state = test_tui_state();
        let activity = TreeNavigationActivity::start(&state).unwrap();
        assert!(state.tree_navigation_active.load(Ordering::Acquire));
        assert!(*state.status.lock().unwrap() == RunStatus::Working);
        drop(activity);
        assert!(!state.tree_navigation_active.load(Ordering::Acquire));
        assert!(*state.status.lock().unwrap() == RunStatus::Idle);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _activity = TreeNavigationActivity::start(&state).unwrap();
            panic!("injected navigation unwind");
        }));
        assert!(result.is_err());
        assert!(!state.tree_navigation_active.load(Ordering::Acquire));
        assert!(*state.status.lock().unwrap() == RunStatus::Idle);
    }

    #[test]
    fn tree_summary_draft_restoration_respects_existing_text_and_pending_images() {
        let state = test_tui_state();
        restore_tree_prompt(&state, Some("old prompt\n猫"), &[]);
        assert_eq!(state.editor.get_text(), "old prompt\n猫");
        restore_tree_prompt(&state, Some("replacement"), &[]);
        assert_eq!(state.editor.get_text(), "old prompt\n猫");
        state.editor.set_text("");
        state
            .pending_images
            .lock()
            .unwrap()
            .push(ice_ai::types::ImageContent {
                kind: ice_ai::types::ImageContentType,
                data: "test".into(),
                mime_type: "image/png".into(),
            });
        restore_tree_prompt(&state, Some("replacement"), &[]);
        assert_eq!(state.editor.get_text(), "");
        assert_eq!(state.pending_images.lock().unwrap().len(), 1);
    }

    fn selector(
        bindings: ice_tui::Keybindings,
    ) -> (
        TreeSummarySelector,
        Arc<Mutex<Vec<TreeNavigationRequest>>>,
        Arc<AtomicBool>,
    ) {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let cancelled = Arc::new(AtomicBool::new(false));
        let out = requests.clone();
        let cancel = cancelled.clone();
        (
            TreeSummarySelector::new(
                "full-target-id".into(),
                bindings,
                Arc::new(move |request| out.lock().unwrap().push(request)),
                Arc::new(move || {
                    cancel.store(true, Ordering::SeqCst);
                }),
            ),
            requests,
            cancelled,
        )
    }

    fn press(selector: &TreeSummarySelector, code: KeyCode, modifiers: KeyModifiers) {
        selector.handle_key(KeyEvent::new(code, modifiers));
    }

    #[test]
    fn tree_summary_choices_dispatch_only_after_confirmation() {
        let (widget, requests, cancelled) = selector(ice_tui::Keybindings::new());
        assert!(requests.lock().unwrap().is_empty());
        press(&widget, KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(
            *requests.lock().unwrap(),
            vec![TreeNavigationRequest {
                entry_id: "full-target-id".into(),
                summarize: false,
                instructions: None,
            }]
        );
        press(&widget, KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(requests.lock().unwrap().len(), 1);
        press(&widget, KeyCode::Enter, KeyModifiers::NONE);
        assert!(requests.lock().unwrap()[1].summarize);
        press(&widget, KeyCode::Esc, KeyModifiers::NONE);
        assert!(cancelled.load(Ordering::SeqCst));
    }

    #[test]
    fn tree_summary_custom_prompt_cancel_returns_to_choice_without_navigation() {
        let (widget, requests, cancelled) = selector(ice_tui::Keybindings::new());
        widget.list.set_selected_index(2);
        press(&widget, KeyCode::Enter, KeyModifiers::NONE);
        for ch in "Keep decisions".chars() {
            press(&widget, KeyCode::Char(ch), KeyModifiers::NONE);
        }
        press(&widget, KeyCode::Esc, KeyModifiers::NONE);
        assert!(requests.lock().unwrap().is_empty());
        assert!(!cancelled.load(Ordering::SeqCst));
        assert!(!*widget.editing.lock().unwrap());
        press(&widget, KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(widget.instructions.get_text(), "Keep decisions");
        press(&widget, KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(
            *requests.lock().unwrap(),
            vec![TreeNavigationRequest {
                entry_id: "full-target-id".into(),
                summarize: true,
                instructions: Some("Keep decisions".into()),
            }]
        );
    }

    #[test]
    fn tree_summary_effective_confirm_submit_and_cancel_bindings() {
        let mut bindings = ice_tui::Keybindings::new();
        for (id, code) in [
            ("tui.select.confirm", KeyCode::F(6)),
            ("tui.input.submit", KeyCode::F(7)),
            ("tui.select.cancel", KeyCode::F(8)),
        ] {
            bindings.set(id, vec![ice_tui::KeyCombo::new(code, KeyModifiers::NONE)]);
        }
        let (widget, requests, cancelled) = selector(bindings);
        widget.list.set_selected_index(2);
        press(&widget, KeyCode::Enter, KeyModifiers::NONE);
        assert!(!*widget.editing.lock().unwrap());
        press(&widget, KeyCode::F(6), KeyModifiers::NONE);
        press(&widget, KeyCode::F(8), KeyModifiers::NONE);
        assert!(!*widget.editing.lock().unwrap());
        assert!(!cancelled.load(Ordering::SeqCst));
        press(&widget, KeyCode::F(6), KeyModifiers::NONE);
        widget.instructions.set_text("custom");
        press(&widget, KeyCode::F(7), KeyModifiers::NONE);
        assert_eq!(
            requests.lock().unwrap()[0].instructions.as_deref(),
            Some("custom")
        );
        for width in [1, 20, 80] {
            assert!(widget
                .render(width)
                .iter()
                .all(|line| ice_tui::visible_width(line) <= width));
        }
    }

    #[test]
    fn replaying_a_prompt_restores_its_image_attachments() {
        let image = ice_ai::types::ImageContent {
            kind: Default::default(),
            data: "aGk=".into(),
            mime_type: "image/png".into(),
        };
        let user = Entry::Message(MessageEntry {
            base: EntryBase {
                entry_type: "message".into(),
                id: "u".into(),
                seq: 1,
                parent_id: Some("p".into()),
                timestamp: 0,
            },
            message: user_message_with_images("look", vec![image.clone()]),
            terminate: None,
        });
        let images = tree_prompt_images(&user);
        assert_eq!(images.len(), 1);
        let state = test_tui_state();
        restore_tree_prompt(&state, Some("look"), &images);
        assert_eq!(state.editor.get_text(), "look");
        assert_eq!(state.pending_images.lock().unwrap().len(), 1);
        // A draft the user already started is never mixed with replayed images.
        let started = test_tui_state();
        started.editor.set_text("mine");
        restore_tree_prompt(&started, Some("look"), &images);
        assert!(started.pending_images.lock().unwrap().is_empty());
    }

    #[test]
    fn tree_summary_target_preserves_full_ids_and_draft_content() {
        let user = Entry::Message(MessageEntry {
            base: EntryBase {
                entry_type: "message".into(),
                id: "selected-user-full-id".into(),
                seq: 1,
                parent_id: Some("previous-full-id".into()),
                timestamp: 0,
            },
            message: user_message_with_images("old\nprompt", Vec::new()),
            terminate: None,
        });
        assert_eq!(
            tree_navigation_target(&user),
            (Some("previous-full-id".into()), Some("old\nprompt".into()))
        );
        let mut assistant = user.clone();
        if let Entry::Message(message) = &mut assistant {
            message.message = AgentMessage::Assistant(Box::new(AssistantMessage::empty(
                ice_ai::Api::Faux,
                "faux",
                "m",
                0,
            )));
        }
        assert_eq!(
            tree_navigation_target(&assistant),
            (Some("selected-user-full-id".into()), None)
        );
        let mut custom = user;
        if let Entry::Message(message) = &mut custom {
            message.base.parent_id = None;
            message.message = AgentMessage::Custom(ice_agent::CustomMessage::new(
                "custom",
                vec![Content::Text(ice_ai::types::TextContent {
                    kind: ice_ai::types::TextContentType,
                    text: "custom prompt".into(),
                    text_signature: None,
                })],
                serde_json::json!({}),
                0,
            ));
        }
        assert_eq!(
            tree_navigation_target(&custom),
            (None, Some("custom prompt".into()))
        );
        if let Entry::Message(message) = &mut custom {
            if let AgentMessage::Custom(custom) = &mut message.message {
                custom.role = ice_harness::messages::BASH_EXECUTION_ROLE.into();
            }
        }
        assert_eq!(
            tree_navigation_target(&custom),
            (Some("selected-user-full-id".into()), None)
        );
    }
}
