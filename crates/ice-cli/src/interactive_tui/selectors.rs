//! Editor-swapping selectors for models, sessions, themes, thinking, tools, and images.

use super::*;

pub(super) async fn apply_thinking_selection(
    lane: &Arc<dyn AgentLane>,
    state: &Arc<TuiState>,
    chat: &Arc<Container>,
    level: ice_ai::types::ThinkingLevel,
    save_default: bool,
) {
    if let Err(error) = lane.set_thinking_level(level).await {
        add_error_message(chat, &format!("Could not change thinking level: {error}"));
        return;
    }
    let name = thinking_level_name(level);
    state.footer.set_thinking_level(Some(name));
    let saved = if save_default {
        let result = crate::settings::load_settings()
            .map_err(|error| error.to_string())
            .and_then(|mut settings| {
                settings.default_thinking_level = Some(name.to_string());
                crate::settings::save_settings(&settings)
            });
        match result {
            Ok(_) => " Default saved.".to_string(),
            Err(error) => format!(" Changed for this session, but could not save default: {error}"),
        }
    } else {
        String::new()
    };
    add_note_message(chat, &format!("Thinking set to {name}.{saved}"));
}

// ===========================================================================
// Selectors — editor-container swap (TS showSelector pattern)
// ===========================================================================

/// Swap the `editor_container`'s child (the editor) for a `SelectList`,
/// hiding the editor while the selector is open. Records the selector in
/// `state.active_selector` so the key loop routes to it.
pub(super) fn open_selector(
    state: &Arc<TuiState>,
    editor_container: &Arc<Container>,
    editor: &Arc<Editor>,
    tui: &Arc<TuiAltScreen>,
    list: Arc<SelectList>,
    kind: SelectorKind,
) {
    open_selector_with_view(
        state,
        editor_container,
        editor,
        tui,
        SelectorView::List(list.clone()),
        list,
        kind,
    );
}

/// Open a searchable selector: the list plus a fuzzy-filter search box.
///
/// Returns the wrapper so callers can prefill the query (upstream's
/// `initialSearchInput`). Mirrors `SelectSubmenu` with `searchable: true`.
pub(super) fn open_searchable_selector(
    state: &Arc<TuiState>,
    editor_container: &Arc<Container>,
    editor: &Arc<Editor>,
    tui: &Arc<TuiAltScreen>,
    title: Option<&str>,
    description: Option<&str>,
    list: Arc<SelectList>,
    kind: SelectorKind,
) -> Arc<SearchableSelectList> {
    let searchable = Arc::new(SearchableSelectList::new(title, description, list));
    open_selector_with_view(
        state,
        editor_container,
        editor,
        tui,
        SelectorView::Searchable(searchable.clone()),
        searchable.clone(),
        kind,
    );
    // `TuiAltScreen::set_focus` only records the focused component; components
    // manage their own focus flag. Without this the search caret never renders
    // (and IME candidate positioning stays on the editor's old caret).
    searchable.set_focused(true);
    searchable
}

/// Open a selector with an optional framed view. Native extension selectors
/// wrap the list with a title and hint while built-in selectors keep the list
/// as the complete view.
///
/// `view_router` receives keys (a bare list, or a searchable wrapper) while
/// `view` is what renders — the two are the same object for every built-in
/// selector, but extension selectors frame a plain list.
pub(super) fn open_selector_with_view<V: Component + 'static>(
    state: &Arc<TuiState>,
    editor_container: &Arc<Container>,
    editor: &Arc<Editor>,
    tui: &Arc<TuiAltScreen>,
    view_router: SelectorView,
    view: Arc<V>,
    kind: SelectorKind,
) {
    // Dispatch ui_prompt_start event
    dispatch_ui_prompt_event(state);

    // Unfocus the editor so its cursor marker doesn't render behind the list.
    editor.set_focused(false);
    // A selector replaces the editor slot. Drop stale slash/@file
    // suggestions so they cannot reappear after the selector closes.
    state.autocomplete_container.clear();
    // Swap: clear the container and add the selector view.
    editor_container.clear();
    editor_container.add_child(view.clone());
    *state.active_selector.lock().unwrap() = Some((view_router, kind));
    let focused: Arc<dyn Component> = view;
    tui.set_focus(Some(focused));
    tui.request_render(false);
}

/// Show a capturing modal overlay over the chat and route keys to
/// `view_router`. The editor stays in its slot, unfocused.
pub(super) fn open_overlay_selector<V: Component + 'static>(
    state: &Arc<TuiState>,
    editor: &Arc<Editor>,
    tui: &Arc<TuiAltScreen>,
    view_router: SelectorView,
    view: Arc<V>,
    options: ice_tui::OverlayOptions,
    kind: SelectorKind,
) {
    dispatch_ui_prompt_event(state);
    editor.set_focused(false);
    state.autocomplete_container.clear();
    let handle = tui.show_overlay(view.clone(), Some(options));
    if let Some(old) = state.active_overlay.lock().unwrap().replace(handle) {
        old.hide();
    }
    *state.active_selector.lock().unwrap() = Some((view_router, kind));
    let focused: Arc<dyn Component> = view;
    tui.set_focus(Some(focused));
    tui.request_render(false);
}

/// Restore the editor into the `editor_container` and clear the active
/// selector (hiding its overlay, if any). Called by selector `on_cancel` and
/// the Esc handler.
pub(super) fn close_selector(
    state: &Arc<TuiState>,
    editor_container: &Arc<Container>,
    editor: &Arc<Editor>,
    tui: &Arc<TuiAltScreen>,
) {
    // Dispatch ui_prompt_end event
    dispatch_ui_prompt_event_end(state);

    if let Some(overlay) = state.active_overlay.lock().unwrap().take() {
        overlay.hide();
    }
    editor_container.clear();
    editor_container.add_child(editor.clone());
    state.autocomplete_container.clear();
    editor.set_focused(true);
    *state.active_selector.lock().unwrap() = None;
    *state.active_extension_cancel.lock().unwrap() = None;
    tui.set_focus(Some(editor.clone()));
    tui.request_render(false);
}

/// Build + open the `/model` selector. Items are the resolved catalog (display
/// label = model name; description = id), with the current model marked.
/// Selecting applies the model **live** via `lane.set_model` (takes effect on
/// the next user message — the in-flight run's config is already snapshotted),
/// updates the footer, and notes the next-prompt effect.
pub(super) fn open_model_selector(
    state: &Arc<TuiState>,
    editor_container: &Arc<Container>,
    editor: &Arc<Editor>,
    tui: &Arc<TuiAltScreen>,
    catalog: &[ice_ai::Model],
    lane: &Arc<dyn AgentLane>,
    lane_model_id: &str,
    chat: &Arc<Container>,
    initial_search: Option<&str>,
) -> Option<Arc<SearchableSelectList>> {
    let favorites = favorite_models();
    let items = model_selector_items(catalog, lane_model_id, &favorites);
    if items.is_empty() {
        add_note_message(
            chat,
            "No models in the catalog. Use --model at startup to select one.",
        );
        tui.request_render(false);
        return None;
    }
    let start = model_selector_start(&items, &favorites);
    let list = Arc::new(SelectList::new(items, 10));
    if let Some(index) = start {
        list.set_selected_index(index);
    }

    // Capture the catalog + lane so the on_select closure can resolve the
    // chosen Model and apply it. `on_select` fires on the blocking key thread,
    // so the async `set_model` runs on a spawned task (matches Ctrl+P).
    let catalog_arc = catalog.to_vec();
    let state_sel = state.clone();
    let ec_sel = editor_container.clone();
    let editor_sel = editor.clone();
    let tui_sel = tui.clone();
    let chat_sel = chat.clone();
    let lane_sel = lane.clone();
    list.on_select(Arc::new(move |item| {
        let Some(model) = find_model_selector_match(&catalog_arc, &item.value) else {
            add_note_message(
                &chat_sel,
                &format!("Model {} not found in catalog.", item.label),
            );
            close_selector(&state_sel, &ec_sel, &editor_sel, &tui_sel);
            return;
        };

        // The harness now registers every authenticated provider runtime, so
        // picking a model from ANOTHER provider hot-switches it (no restart).
        // Persist the choice as the saved default so the next launch keeps it,
        // whichever provider it belongs to.
        let lane = lane_sel.clone();
        let state = state_sel.clone();
        let tui = tui_sel.clone();
        tokio::spawn(async move {
            report_model_selection(&lane, &state, &tui, model, true).await;
        });
        close_selector(&state_sel, &ec_sel, &editor_sel, &tui_sel);
    }));
    let state_cancel = state.clone();
    let ec_cancel = editor_container.clone();
    let editor_cancel = editor.clone();
    let tui_cancel = tui.clone();
    list.on_cancel(Arc::new(move || {
        close_selector(&state_cancel, &ec_cancel, &editor_cancel, &tui_cancel);
    }));

    let searchable = open_searchable_selector(
        state,
        editor_container,
        editor,
        tui,
        Some("Select model"),
        Some("Type to filter by name, provider, or id · shift+f toggle favorite"),
        list,
        SelectorKind::Model,
    );
    if let Some(term) = initial_search {
        searchable.set_search_text(term);
    }
    Some(searchable)
}

/// Shift+F in `/model` (upstream `toggleFavorite`): save the highlighted
/// model's new favorite status, then re-pin the list with the same search
/// text and the highlight kept on that model.
pub(super) fn toggle_selected_favorite(
    searchable: &SearchableSelectList,
    catalog: &[ice_ai::Model],
    current_model_id: &str,
) -> Result<(), String> {
    let Some(item) = searchable.list().get_selected_item() else {
        return Ok(());
    };
    let favorites = toggle_favorite_model(&item.value)?;
    let items = model_selector_items(catalog, current_model_id, &favorites);
    searchable
        .list()
        .replace_items(items, &searchable.input().get_value(), Some(&item.value));
    Ok(())
}

/// Index of the item `model_selector_items` marked as the current model, so
/// `/model` opens on it.
pub(super) fn current_model_index(items: &[SelectItem]) -> Option<usize> {
    items.iter().position(|item| {
        item.description
            .as_deref()
            .is_some_and(|d| d.contains(" (current)"))
    })
}

/// Where `/model` opens. Favorites are pinned at the top, and opening on the
/// current model would scroll them out of view, so with any favorite listed the
/// picker opens at the top; otherwise it opens on the current model.
pub(super) fn model_selector_start(items: &[SelectItem], favorites: &[String]) -> Option<usize> {
    if items.iter().any(|item| favorites.contains(&item.value)) {
        None
    } else {
        current_model_index(items)
    }
}

/// The caller's requested behavior after a model switch has succeeded.
#[derive(Clone, Copy)]
pub(super) enum ModelSwitchMode {
    Cycle,
    Session,
    PersistDefault,
}

/// Report only the accepted result. In particular a pending or rejected switch
/// must not change the footer, context-window badge, or saved default. Keeping
/// the previous state also works after the harness closes and get_model fails.
pub(super) fn spawn_set_model(
    state: &Arc<TuiState>,
    lane: &Arc<dyn AgentLane>,
    model: ice_ai::Model,
    chat: &Arc<Container>,
    tui: &Arc<TuiAltScreen>,
    mode: ModelSwitchMode,
) -> tokio::task::JoinHandle<()> {
    let state = state.clone();
    let lane = lane.clone();
    let chat = chat.clone();
    let tui = tui.clone();
    tokio::spawn(async move {
        match lane.set_model(model.clone()).await {
            Ok(()) => {
                state.set_current_model(&model);
                if !matches!(mode, ModelSwitchMode::Cycle) {
                    add_note_message(
                        &chat,
                        &format!(
                            "Model set to {}/{} — applies to the next message.",
                            model.provider, model.id
                        ),
                    );
                }
                if matches!(mode, ModelSwitchMode::PersistDefault) {
                    match set_default_model(&model.provider, &model.id) {
                        Ok(_) => add_note_message(&chat, "Model saved as the default."),
                        Err(error) => add_error_message(
                            &chat,
                            &format!("Model is active, but could not save its default: {error}"),
                        ),
                    }
                }
            }
            Err(error) => add_error_message(
                &chat,
                &format!(
                    "Could not switch to {}/{}: {error}",
                    model.provider, model.id
                ),
            ),
        }
        tui.request_render(false);
    })
}

/// Exercise live-lane cycle anchoring with the persisted scope in integration tests.
/// Interactive hotkeys use the session's scope rather than reloading defaults.
#[cfg(test)]
pub(super) async fn cycle_model(
    lane: &Arc<dyn AgentLane>,
    catalog: &[ice_ai::Model],
    backwards: bool,
) -> Result<Option<ice_ai::Model>, String> {
    if catalog.is_empty() {
        return Ok(None);
    }
    let current = lane.get_model().await.map_err(|error| error.to_string())?;
    let settings = crate::settings::load_settings().map_err(|error| error.to_string())?;
    let scope = scoped_catalog(catalog, settings.scoped_models.as_deref());
    Ok(if backwards {
        cycle_prev_model(&scope, &current)
    } else {
        cycle_next_model(&scope, &current)
    })
}

/// Commit UI/default changes only after the live lane accepts the model.
pub(super) async fn apply_model_selection(
    lane: &Arc<dyn AgentLane>,
    state: &Arc<TuiState>,
    model: ice_ai::Model,
    save_default: bool,
) -> Result<String, String> {
    let switch_error = |error| {
        format!(
            "Could not switch to {}/{}: {error}",
            model.provider, model.id
        )
    };
    let current = lane.get_model().await.map_err(&switch_error)?;
    lane.set_model(model.clone()).await.map_err(switch_error)?;
    state.set_current_model(&model);
    let mut note = format!(
        "Model set to {}/{} — applies to the next message.",
        model.provider, model.id
    );
    if save_default || current.provider != model.provider {
        match set_default_model(&model.provider, &model.id) {
            Ok(_) => note.push_str(" Default saved."),
            Err(error) => note.push_str(&format!(" Could not save the default: {error}")),
        }
    }
    Ok(note)
}

pub(super) async fn report_model_selection(
    lane: &Arc<dyn AgentLane>,
    state: &Arc<TuiState>,
    tui: &Arc<TuiAltScreen>,
    model: ice_ai::Model,
    save_default: bool,
) {
    match apply_model_selection(lane, state, model, save_default).await {
        Ok(note) => add_note_message(&state.chat_container, &note),
        Err(error) => add_error_message(&state.chat_container, &error),
    }
    tui.request_render(false);
}

/// Cycle after the current provider/API/model identity, wrapping to the first.
/// An empty catalog returns `None`; a missing anchor returns the first entry.
pub(super) fn cycle_next_model(
    catalog: &[ice_ai::Model],
    current: &ice_ai::Model,
) -> Option<ice_ai::Model> {
    if catalog.is_empty() {
        return None;
    }
    let idx = catalog.iter().position(|m| {
        m.provider == current.provider
            && m.api == current.api
            && m.id.eq_ignore_ascii_case(&current.id)
    });
    match idx {
        Some(i) => {
            let next = (i + 1) % catalog.len();
            Some(catalog[next].clone())
        }
        None => Some(catalog[0].clone()),
    }
}

/// Cycle before the current provider/API/model identity, wrapping to the last —
/// the mirror of [`cycle_next_model`] for the previous-model hotkey.
pub(super) fn cycle_prev_model(
    catalog: &[ice_ai::Model],
    current: &ice_ai::Model,
) -> Option<ice_ai::Model> {
    if catalog.is_empty() {
        return None;
    }
    let idx = catalog.iter().position(|m| {
        m.provider == current.provider
            && m.api == current.api
            && m.id.eq_ignore_ascii_case(&current.id)
    });
    match idx {
        Some(i) => {
            let prev = (i + catalog.len() - 1) % catalog.len();
            Some(catalog[prev].clone())
        }
        None => Some(catalog[0].clone()),
    }
}

/// Browse current-project or indexed cross-project sessions without blocking input.
pub(super) fn open_session_selector(
    state: &Arc<TuiState>,
    editor_container: &Arc<Container>,
    editor: &Arc<Editor>,
    tui: &Arc<TuiAltScreen>,
    cwd: &std::path::Path,
    tx: &mpsc::UnboundedSender<TuiMessage>,
    active: Option<Arc<dyn ice_harness::session::types::SessionStorage>>,
) {
    session_picker::open(state, editor_container, editor, tui, cwd, tx, active);
}

pub(super) fn custom_entry_display_text(
    custom_type: &str,
    data: Option<&serde_json::Value>,
) -> Option<String> {
    let data = data?;
    let text = data
        .get("summary")
        .or_else(|| data.get("text"))
        .or_else(|| data.get("output"))
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())?;
    let label = match custom_type {
        "compactionSummary" => "Compaction summary",
        "branchSummary" => "Branch summary",
        "bashExecution" => "Command output",
        other => other,
    };
    Some(format!("{label}: {text}"))
}

/// Text-bearing user messages across the session, oldest first, as
/// `(entry id, text)`: the fork points `/fork` offers (ice
/// `getUserMessagesForForking`).
pub(super) async fn user_messages_for_forking(harness: &AgentHarness) -> Vec<(String, String)> {
    harness
        .session()
        .view("main")
        .find_entries(&EntryQuery {
            order: Some(EntryOrder::OldestFirst),
            ..Default::default()
        })
        .await
        .unwrap_or_default()
        .into_iter()
        .filter_map(|entry| match &entry {
            Entry::Message(me) => match &me.message {
                AgentMessage::User(u) => {
                    let text = user_message_text(u);
                    (!text.is_empty()).then(|| (entry.id().to_string(), text))
                }
                _ => None,
            },
            _ => None,
        })
        .collect()
}

/// Text of the user message `entry_id`, to put back in the editor after
/// forking before it.
pub(super) async fn fork_message_text(harness: &AgentHarness, entry_id: &str) -> Option<String> {
    user_messages_for_forking(harness)
        .await
        .into_iter()
        .find(|(id, _)| id == entry_id)
        .map(|(_, text)| text)
}

/// The file identity shown by the picker, so a queued selection cannot fork a
/// different session after a resume, import, or extension-triggered switch.
#[derive(Clone, Debug)]
pub(super) struct ForkSelection {
    pub(super) session_id: String,
    pub(super) session_path: Option<String>,
    pub(super) entry_id: String,
}

/// `/fork`: choose a prior user message (newest preselected). Selecting one
/// forks the session before it and puts its text in the editor, so the
/// prompt can be revised and sent again (ice `showUserMessageSelector`).
pub(super) async fn open_fork_selector(
    harness: &AgentHarness,
    state: &Arc<TuiState>,
    editor_container: &Arc<Container>,
    editor: &Arc<Editor>,
    tui: &Arc<TuiAltScreen>,
    chat: &Arc<Container>,
    tx: &mpsc::UnboundedSender<TuiMessage>,
) {
    if *state.status.lock().unwrap() != RunStatus::Idle {
        add_note_message(chat, "Wait for the current run to finish before forking.");
        tui.request_render(false);
        return;
    }
    let session = harness.session();
    let session_id = session.storage().metadata().id.clone();
    let session_path = session.storage().persistence_path();
    let messages = user_messages_for_forking(harness).await;
    if messages.is_empty() {
        add_note_message(chat, "No messages to fork from.");
        tui.request_render(false);
        return;
    }
    let items: Vec<SelectItem> = messages
        .iter()
        .enumerate()
        .map(|(index, (id, text))| {
            let label = text.split_whitespace().collect::<Vec<_>>().join(" ");
            SelectItem::new(
                id,
                if label.is_empty() {
                    "(empty message)"
                } else {
                    &label
                },
            )
            .with_description(&format!("#{}", index + 1))
            .with_search_text(text)
        })
        .collect();
    let newest = items.len() - 1;
    let list = Arc::new(SelectList::new(items, 10));
    list.set_selected_index(newest);
    let state_sel = state.clone();
    let ec_sel = editor_container.clone();
    let editor_sel = editor.clone();
    let tui_sel = tui.clone();
    let tx_sel = tx.clone();
    list.on_select(Arc::new(move |item| {
        close_selector(&state_sel, &ec_sel, &editor_sel, &tui_sel);
        let _ = tx_sel.send(TuiMessage::ForkBefore(ForkSelection {
            session_id: session_id.clone(),
            session_path: session_path.clone(),
            entry_id: item.value.clone(),
        }));
    }));
    let state_cancel = state.clone();
    let ec_cancel = editor_container.clone();
    let editor_cancel = editor.clone();
    let tui_cancel = tui.clone();
    list.on_cancel(Arc::new(move || {
        close_selector(&state_cancel, &ec_cancel, &editor_cancel, &tui_cancel);
    }));
    open_searchable_selector(
        state,
        editor_container,
        editor,
        tui,
        Some("Fork from message"),
        Some("Forks before the chosen message and puts it in the editor"),
        list,
        SelectorKind::Fork,
    );
}

/// Content and role used by tree search and preview formatting. Keep this full:
/// a clipped row must not make words later in a message impossible to find.
pub(super) fn tree_entry_text(entry: &Entry) -> String {
    match entry {
        Entry::Message(me) => match &me.message {
            AgentMessage::User(user) => format!("user: {}", user_message_text(user)),
            AgentMessage::Assistant(assistant) => {
                let text = assistant_text(assistant);
                let text = if !text.trim().is_empty() {
                    text
                } else if assistant.stop_reason == ice_ai::types::StopReason::Aborted {
                    "(aborted)".into()
                } else if let Some(error) = &assistant.error_message {
                    error.clone()
                } else {
                    let tools = assistant
                        .content
                        .iter()
                        .filter_map(|content| match content {
                            Content::ToolCall(call) => Some(call.name.as_str()),
                            _ => None,
                        })
                        .collect::<Vec<_>>();
                    if tools.is_empty() {
                        "(no content)".into()
                    } else {
                        format!("(tool calls: {})", tools.join(", "))
                    }
                };
                format!("assistant: {text}")
            }
            AgentMessage::ToolResult(result) => {
                let text = result
                    .content
                    .iter()
                    .filter_map(|content| match content {
                        Content::Text(text) => Some(text.text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                format!("tool result: {} {text}", result.tool_name)
            }
            AgentMessage::Custom(custom) => custom_message_fallback(custom),
        },
        Entry::ModelChange(model) => format!("model → {}/{}", model.provider, model.model_id),
        Entry::ThinkingLevel(thinking) => format!("thinking: {}", thinking.thinking_level),
        Entry::ActiveTools(tools) => {
            format!("active tools: {}", tools.active_tool_names.join(", "))
        }
        Entry::Compaction(compaction) => format!("compaction: {}", compaction.summary),
        Entry::BranchSummary(branch) => format!("branch summary: {}", branch.summary),
        Entry::Custom(custom) => {
            custom_entry_display_text(&custom.custom_type, custom.data.as_ref())
                .unwrap_or_else(|| format!("custom: {}", custom.custom_type))
        }
    }
}

/// A bounded one-row preview that does not split wide graphemes or emit control
/// characters from stored content. The selectable row retains the full entry ID.
pub(super) fn tree_entry_summary(entry: &Entry) -> String {
    let text = ice_tui::strip_ansi(&tree_entry_text(entry));
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let line: String = line
        .chars()
        .filter(|character| !character.is_control())
        .collect();
    ice_tui::strip_ansi(&ice_tui::truncate_to_width(&line, 80, "…"))
}

pub(super) fn tree_entry_item(entry: &Entry, current: Option<&str>) -> SelectItem {
    let summary = tree_entry_summary(entry);
    let label = if current == Some(entry.id()) {
        format!("{summary} (current)")
    } else {
        summary.clone()
    };
    let short_id: String = entry.id().chars().take(12).collect();
    SelectItem::new(entry.id(), &label)
        .with_description(&format!("#{} {short_id}", entry.seq()))
        .with_search_text(&format!(
            "{} {} {} {}",
            tree_entry_text(entry),
            entry.entry_type(),
            entry.seq(),
            entry.id()
        ))
}

/// Open the persisted tree with branch hierarchy and filtering. The selected
/// full entry ID is sent to the main loop only when the user confirms.
pub(super) async fn open_tree_selector(
    harness: &AgentHarness,
    state: &Arc<TuiState>,
    editor_container: &Arc<Container>,
    editor: &Arc<Editor>,
    tui: &Arc<TuiAltScreen>,
    chat: &Arc<Container>,
    tx: &mpsc::UnboundedSender<TuiMessage>,
) {
    let entries = match harness
        .session()
        .view("main")
        .find_entries(&EntryQuery {
            order: Some(EntryOrder::OldestFirst),
            ..Default::default()
        })
        .await
    {
        Ok(entries) => entries,
        Err(error) => {
            add_error_message(chat, &format!("Could not read session tree: {error}"));
            tui.request_render(false);
            return;
        }
    };
    if entries.is_empty() {
        add_note_message(chat, "The current session has no entries to navigate.");
        tui.request_render(false);
        return;
    }
    let snapshot = match harness.lane("main").snapshot().await {
        Ok(snapshot) => snapshot,
        Err(error) => {
            add_error_message(
                chat,
                &format!("Could not read current tree position: {error}"),
            );
            tui.request_render(false);
            return;
        }
    };
    let state_sel = state.clone();
    let ec_sel = editor_container.clone();
    let editor_sel = editor.clone();
    let tui_sel = tui.clone();
    let tx_sel = tx.clone();
    let state_cancel = state.clone();
    let ec_cancel = editor_container.clone();
    let editor_cancel = editor.clone();
    let tui_cancel = tui.clone();
    // Keep the exact view (filter, query, folding and selected row) available
    // when the summary chooser or operation returns to the tree. The callback
    // holds only a weak reference so the selector cannot retain itself.
    let return_view = Arc::new(Mutex::new(std::sync::Weak::<TreeSelector>::new()));
    let view_sel = return_view.clone();
    let selector = Arc::new(TreeSelector::new(
        entries,
        snapshot.leaf_id.as_deref(),
        snapshot.labels,
        ice_tui::get_keybindings().clone(),
        Arc::new(move |entry_id| {
            let view = view_sel.lock().unwrap().upgrade();
            close_selector(&state_sel, &ec_sel, &editor_sel, &tui_sel);
            if let Some(view) = view {
                let _ = tx_sel.send(TuiMessage::NavigateTree(entry_id, view));
            }
        }),
        Arc::new(move || {
            close_selector(&state_cancel, &ec_cancel, &editor_cancel, &tui_cancel);
        }),
    ));
    *return_view.lock().unwrap() = Arc::downgrade(&selector);
    reopen_tree_selector(selector, state, editor_container, editor, tui);
}

/// Reuse the tree itself rather than reconstructing it at the current leaf:
/// cancelling a chooser must not discard the user's search or folded branches.
pub(super) fn reopen_tree_selector(
    selector: Arc<TreeSelector>,
    state: &Arc<TuiState>,
    editor_container: &Arc<Container>,
    editor: &Arc<Editor>,
    tui: &Arc<TuiAltScreen>,
) {
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

/// Build + open the `/theme` selector. Built-in presets and enabled package
/// themes are shown; selecting applies the theme live and re-renders.
pub(super) fn open_theme_selector(
    state: &Arc<TuiState>,
    editor_container: &Arc<Container>,
    editor: &Arc<Editor>,
    tui: &Arc<TuiAltScreen>,
) {
    let items = vec![
        SelectItem::new("dark", "Dark").with_description("Default dark theme"),
        SelectItem::new("light", "Light").with_description("Light background"),
        SelectItem::new("monochrome", "Monochrome").with_description("No color accents"),
    ];
    let list = Arc::new(SelectList::new(items, 10));

    let state_sel = state.clone();
    let ec_sel = editor_container.clone();
    let editor_sel = editor.clone();
    let tui_sel = tui.clone();
    let chat_sel = state.chat_container.clone();
    list.on_select(Arc::new(move |item| {
        let preset = match item.value.as_str() {
            "light" => Some(ThemePreset::Light),
            "monochrome" => Some(ThemePreset::Monochrome),
            "dark" => Some(ThemePreset::Dark),
            name => {
                add_error_message(&chat_sel, &format!("Unknown theme `{name}`."));
                close_selector(&state_sel, &ec_sel, &editor_sel, &tui_sel);
                tui_sel.render_now(true);
                return;
            }
        };
        let Some(preset) = preset else { return };
        apply_theme_preset(preset);
        state_sel.theme_manager.apply_preset(preset);
        // A quick accent note so the user sees the change registered even if
        // the terminal's own colors mask the preset difference.
        add_note_message(&chat_sel, &format!("Theme set to {}.", item.label));
        close_selector(&state_sel, &ec_sel, &editor_sel, &tui_sel);
        tui_sel.render_now(true);
    }));
    let state_cancel = state.clone();
    let ec_cancel = editor_container.clone();
    let editor_cancel = editor.clone();
    let tui_cancel = tui.clone();
    list.on_cancel(Arc::new(move || {
        close_selector(&state_cancel, &ec_cancel, &editor_cancel, &tui_cancel);
    }));

    open_selector(
        state,
        editor_container,
        editor,
        tui,
        list,
        SelectorKind::Theme,
    );
}

/// Sentinel value for the "Add new provider" row in the `/provider` picker.
const ADD_PROVIDER_VALUE: &str = "__add_provider__";
/// Sentinel value for the "Remove provider" row in the `/provider` picker.
const REMOVE_PROVIDER_VALUE: &str = "__remove_provider__";

/// Pick a provider for the current session; the async loop applies and saves it.
#[derive(Debug, PartialEq, Eq)]
enum ProviderPickerKey {
    Ignore,
    Forward,
    ExitSearch,
    Refresh(String),
    Reauthenticate(String),
}

fn provider_picker_key(
    searching: &mut bool,
    key: KeyEvent,
    selected: Option<&str>,
) -> ProviderPickerKey {
    if key.kind == KeyEventKind::Release {
        return ProviderPickerKey::Ignore;
    }
    if *searching {
        if key.code == KeyCode::Esc {
            *searching = false;
            return ProviderPickerKey::ExitSearch;
        }
        return ProviderPickerKey::Forward;
    }
    if key.modifiers == KeyModifiers::NONE {
        match key.code {
            KeyCode::Char('/') => {
                *searching = true;
                return ProviderPickerKey::Ignore;
            }
            KeyCode::Char(action @ ('r' | 'a')) => {
                return match selected.filter(|id| {
                    !id.is_empty() && *id != ADD_PROVIDER_VALUE && *id != REMOVE_PROVIDER_VALUE
                }) {
                    Some(id) if action == 'r' => ProviderPickerKey::Refresh(id.to_owned()),
                    Some(id) => ProviderPickerKey::Reauthenticate(id.to_owned()),
                    None => ProviderPickerKey::Ignore,
                };
            }
            KeyCode::Char(_) => return ProviderPickerKey::Ignore,
            _ => {}
        }
    }
    ProviderPickerKey::Forward
}

struct ProviderPickerView {
    searchable: Arc<SearchableSelectList>,
    search_mode: Arc<Mutex<bool>>,
}

impl Component for ProviderPickerView {
    fn render(&self, width: usize) -> Vec<String> {
        let searching = *self.search_mode.lock().unwrap();
        let hint = if searching {
            "SEARCH MODE · Esc: keep filter and exit · Enter: select"
        } else {
            "BROWSE MODE · /: search · r: refresh · a: reauthenticate · Esc: cancel"
        };
        let mut lines = vec![ice_tui::truncate_to_width(hint, width, "…")];
        lines.extend(self.searchable.render(width));
        lines
    }

    fn invalidate(&self) {
        self.searchable.invalidate();
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod provider_picker_key_tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn shortcuts_require_a_provider_selection() {
        for id in [
            None,
            Some(""),
            Some(ADD_PROVIDER_VALUE),
            Some(REMOVE_PROVIDER_VALUE),
        ] {
            for action in ['r', 'a'] {
                assert_eq!(
                    provider_picker_key(&mut false, key(KeyCode::Char(action)), id),
                    ProviderPickerKey::Ignore
                );
            }
        }
        assert_eq!(
            provider_picker_key(&mut false, key(KeyCode::Char('r')), Some("openai")),
            ProviderPickerKey::Refresh("openai".into())
        );
        assert_eq!(
            provider_picker_key(&mut false, key(KeyCode::Char('a')), Some("openai")),
            ProviderPickerKey::Reauthenticate("openai".into())
        );
    }

    #[test]
    fn search_mode_routes_text_and_escape_then_restores_shortcuts() {
        let mut searching = false;
        assert_eq!(
            provider_picker_key(&mut searching, key(KeyCode::Char('/')), None),
            ProviderPickerKey::Ignore
        );
        assert!(searching);
        for code in [
            KeyCode::Char('r'),
            KeyCode::Char('a'),
            KeyCode::Down,
            KeyCode::Enter,
        ] {
            assert_eq!(
                provider_picker_key(&mut searching, key(code), Some("openai")),
                ProviderPickerKey::Forward
            );
        }
        assert_eq!(
            provider_picker_key(&mut searching, key(KeyCode::Esc), None),
            ProviderPickerKey::ExitSearch
        );
        assert!(!searching);
        assert_eq!(
            provider_picker_key(&mut searching, key(KeyCode::Char('r')), Some("openai")),
            ProviderPickerKey::Refresh("openai".into())
        );
    }

    #[test]
    fn exiting_search_preserves_filter_and_selection() {
        let list = Arc::new(SelectList::new(
            vec![
                SelectItem::new("openai", "OpenAI"),
                SelectItem::new("other", "Other"),
            ],
            10,
        ));
        let searchable = SearchableSelectList::new(None, None, list.clone());
        searchable.set_search_text("openai");
        let selected = list.get_selected_item().unwrap().value;
        let mut searching = true;
        assert_eq!(
            provider_picker_key(&mut searching, key(KeyCode::Esc), Some(&selected)),
            ProviderPickerKey::ExitSearch
        );
        assert_eq!(list.get_selected_item().unwrap().value, selected);
        assert_eq!(searchable.input().get_value(), "openai");
    }

    #[test]
    fn navigation_and_cancel_are_forwarded_without_typing() {
        for code in [KeyCode::Up, KeyCode::Down, KeyCode::Enter, KeyCode::Esc] {
            assert_eq!(
                provider_picker_key(&mut false, key(code), None),
                ProviderPickerKey::Forward
            );
        }
        assert_eq!(
            provider_picker_key(&mut false, key(KeyCode::Char('x')), None),
            ProviderPickerKey::Ignore
        );
        let release = KeyEvent::new_with_kind(
            KeyCode::Char('r'),
            KeyModifiers::NONE,
            KeyEventKind::Release,
        );
        assert_eq!(
            provider_picker_key(&mut false, release, Some("openai")),
            ProviderPickerKey::Ignore
        );
    }
}

pub(super) fn open_provider_selector(
    state: &Arc<TuiState>,
    editor_container: &Arc<Container>,
    editor: &Arc<Editor>,
    tui: &Arc<TuiAltScreen>,
    extension_session: &crate::session::ExtensionSessionCell,
    tx: &mpsc::UnboundedSender<TuiMessage>,
    current_provider: Option<&str>,
) {
    let mut items = provider_selector_items(
        &current_provider_manifests(extension_session),
        current_provider,
    );
    if items.is_empty() {
        add_note_message(
            &state.chat_container,
            "No providers available. Use \"Add new provider\" to configure one in ~/.icers/agent/models.json.",
        );
        tui.request_render(false);
        return;
    }
    // The trailing actions turn the picker into a small management surface.
    items.push(
        SelectItem::new(ADD_PROVIDER_VALUE, "＋ Add new provider")
            .with_description("Interactive form → writes ~/.icers/agent/models.json")
            .with_search_text("add new provider"),
    );
    items.push(
        SelectItem::new(REMOVE_PROVIDER_VALUE, "− Remove provider configuration")
            .with_description("Delete a models.json provider")
            .with_search_text("remove delete provider"),
    );
    let list = Arc::new(SelectList::new(items, 10));

    let state_sel = state.clone();
    let ec_sel = editor_container.clone();
    let editor_sel = editor.clone();
    let tui_sel = tui.clone();
    let tx_sel = tx.clone();
    list.on_select(Arc::new(move |item| {
        if item.value == ADD_PROVIDER_VALUE {
            close_selector(&state_sel, &ec_sel, &editor_sel, &tui_sel);
            begin_provider_form(&state_sel, &ec_sel, &editor_sel, &tui_sel);
            return;
        }
        if item.value == REMOVE_PROVIDER_VALUE {
            close_selector(&state_sel, &ec_sel, &editor_sel, &tui_sel);
            open_provider_remove_selector(&state_sel, &ec_sel, &editor_sel, &tui_sel);
            return;
        }
        let _ = tx_sel.send(TuiMessage::SwitchProvider(item.value.clone()));
        close_selector(&state_sel, &ec_sel, &editor_sel, &tui_sel);
    }));
    let state_cancel = state.clone();
    let ec_cancel = editor_container.clone();
    let editor_cancel = editor.clone();
    let tui_cancel = tui.clone();
    list.on_cancel(Arc::new(move || {
        close_selector(&state_cancel, &ec_cancel, &editor_cancel, &tui_cancel);
    }));

    let searchable = Arc::new(SearchableSelectList::new(
        Some("Select provider"),
        Some("↑/↓ navigate · Enter selects"),
        list.clone(),
    ));
    let search_mode = Arc::new(Mutex::new(false));
    let router = searchable.clone();
    let view = Arc::new(ProviderPickerView {
        searchable: searchable.clone(),
        search_mode: search_mode.clone(),
    });
    let state_action = state.clone();
    let ec_action = editor_container.clone();
    let editor_action = editor.clone();
    let tui_action = tui.clone();
    let tx_action = tx.clone();
    let extension_action = extension_session.clone();
    open_selector_with_view(
        state,
        editor_container,
        editor,
        tui,
        SelectorView::Custom(Arc::new(move |key| {
            let selected = list.get_selected_item();
            let mut searching = search_mode.lock().unwrap();
            let action = provider_picker_key(
                &mut searching,
                key,
                selected.as_ref().map(|item| item.value.as_str()),
            );
            router.set_focused(*searching);
            drop(searching);
            match action {
                ProviderPickerKey::Ignore | ProviderPickerKey::ExitSearch => {}
                ProviderPickerKey::Forward => {
                    SelectorView::Searchable(router.clone()).handle_key(key)
                }
                ProviderPickerKey::Refresh(id) => {
                    let _ = tx_action.send(TuiMessage::RefreshProviderModels(id));
                }
                ProviderPickerKey::Reauthenticate(id) => {
                    let manifests = current_provider_manifests(&extension_action);
                    let oauth = current_oauth_providers(&extension_action);
                    if !login_provider_items_with_oauth(&manifests, &oauth)
                        .iter()
                        .any(|provider| provider.value == id)
                    {
                        return;
                    }
                    close_selector(&state_action, &ec_action, &editor_action, &tui_action);
                    begin_provider_login(
                        &state_action,
                        &ec_action,
                        &editor_action,
                        &tui_action,
                        &tx_action,
                        &extension_action,
                        &id,
                    );
                }
            }
        })),
        view,
        SelectorKind::Provider,
    );
    searchable.set_focused(false);
}

/// Pick a provider for `/login`, then run its OAuth or API-key flow.
pub(super) fn open_login_provider_selector(
    state: &Arc<TuiState>,
    editor_container: &Arc<Container>,
    editor: &Arc<Editor>,
    tui: &Arc<TuiAltScreen>,
    extension_session: &crate::session::ExtensionSessionCell,
    tx: &mpsc::UnboundedSender<TuiMessage>,
) {
    let manifests = current_provider_manifests(extension_session);
    let oauth_providers = current_oauth_providers(extension_session);
    let items = login_provider_items_with_oauth(&manifests, &oauth_providers);
    if items.is_empty() {
        add_note_message(
            &state.chat_container,
            "No login-capable providers are loaded. Configure a provider or install an extension first.",
        );
        tui.request_render(false);
        return;
    }
    let list = Arc::new(SelectList::new(items, 10));
    let state_sel = state.clone();
    let ec_sel = editor_container.clone();
    let editor_sel = editor.clone();
    let tui_sel = tui.clone();
    let tx_sel = tx.clone();
    let extension_session_sel = extension_session.clone();
    list.on_select(Arc::new(move |item| {
        close_selector(&state_sel, &ec_sel, &editor_sel, &tui_sel);
        begin_provider_login(
            &state_sel,
            &ec_sel,
            &editor_sel,
            &tui_sel,
            &tx_sel,
            &extension_session_sel,
            &item.value,
        );
    }));
    let state_cancel = state.clone();
    let ec_cancel = editor_container.clone();
    let editor_cancel = editor.clone();
    let tui_cancel = tui.clone();
    list.on_cancel(Arc::new(move || {
        close_selector(&state_cancel, &ec_cancel, &editor_cancel, &tui_cancel);
    }));
    open_searchable_selector(
        state,
        editor_container,
        editor,
        tui,
        Some("Log in to provider"),
        Some("Type to filter providers · Enter selects · Esc cancels"),
        list,
        SelectorKind::Provider,
    );
}
pub(super) fn open_logout_provider_selector(
    state: &Arc<TuiState>,
    editor_container: &Arc<Container>,
    editor: &Arc<Editor>,
    tui: &Arc<TuiAltScreen>,
    tx: &mpsc::UnboundedSender<TuiMessage>,
) {
    let items = stored_credential_items();
    if items.is_empty() {
        add_note_message(
            &state.chat_container,
            "No saved provider credentials found.",
        );
        tui.request_render(false);
        return;
    }
    let list = Arc::new(SelectList::new(items, 10));
    let state_sel = state.clone();
    let ec_sel = editor_container.clone();
    let editor_sel = editor.clone();
    let tui_sel = tui.clone();
    let tx_sel = tx.clone();
    list.on_select(Arc::new(move |item| {
        let _ = tx_sel.send(TuiMessage::LogoutProvider(item.value.clone()));
        close_selector(&state_sel, &ec_sel, &editor_sel, &tui_sel);
    }));
    let state_cancel = state.clone();
    let ec_cancel = editor_container.clone();
    let editor_cancel = editor.clone();
    let tui_cancel = tui.clone();
    list.on_cancel(Arc::new(move || {
        close_selector(&state_cancel, &ec_cancel, &editor_cancel, &tui_cancel);
    }));
    open_searchable_selector(
        state,
        editor_container,
        editor,
        tui,
        Some("Log out provider"),
        Some("Type to filter providers · Enter removes · Esc cancels"),
        list,
        SelectorKind::Provider,
    );
}
/// Open the "remove provider" picker: lists only the providers defined in
/// `models.json` (built-ins cannot be removed), and deletes the chosen one.
pub(super) fn open_provider_remove_selector(
    state: &Arc<TuiState>,
    editor_container: &Arc<Container>,
    editor: &Arc<Editor>,
    tui: &Arc<TuiAltScreen>,
) {
    let items = removable_provider_items();
    if items.is_empty() {
        add_note_message(
            &state.chat_container,
            "No removable providers. Only providers defined in ~/.icers/agent/models.json can be removed.",
        );
        tui.request_render(false);
        return;
    }
    let list = Arc::new(SelectList::new(items, 10));

    let state_sel = state.clone();
    let ec_sel = editor_container.clone();
    let editor_sel = editor.clone();
    let tui_sel = tui.clone();
    let chat_sel = state.chat_container.clone();
    list.on_select(Arc::new(move |item| {
        match remove_provider(&item.value) {
            Ok(note) => add_note_message(&chat_sel, &note),
            Err(error) => add_error_message(&chat_sel, &error),
        }
        close_selector(&state_sel, &ec_sel, &editor_sel, &tui_sel);
    }));
    let state_cancel = state.clone();
    let ec_cancel = editor_container.clone();
    let editor_cancel = editor.clone();
    let tui_cancel = tui.clone();
    list.on_cancel(Arc::new(move || {
        close_selector(&state_cancel, &ec_cancel, &editor_cancel, &tui_cancel);
    }));

    open_searchable_selector(
        state,
        editor_container,
        editor,
        tui,
        Some("Remove provider"),
        Some("Type to filter · Enter deletes · Esc cancels"),
        list,
        SelectorKind::Provider,
    );
}

// ===========================================================================
// Feasible selectors — /thinking, /tools, /images
// ===========================================================================

/// One-line descriptions for each thinking level, ported from
/// thinking-selector.ts (the TS `getThinkingLevelDescription` table).
pub(super) fn thinking_level_description(level: ice_ai::types::ThinkingLevel) -> &'static str {
    use ice_ai::types::ThinkingLevel::*;
    match level {
        Off => "Off — No reasoning",
        Minimal => "Minimal — Brief reasoning (~1k tokens)",
        Low => "Low — Light reasoning (~1k tokens)",
        Medium => "Medium — Moderate reasoning (~80% of max)",
        High => "High — Extensive reasoning (~95% of max)",
        Xhigh => "Xhigh — Near-maximal reasoning",
        Max => "Max — Maximum reasoning",
    }
}

/// The lowercase serialized name of a [`ThinkingLevel`] (matches its
/// `#[serde(rename_all = "lowercase")]` form): "off", "minimal", … "max".
pub(super) fn thinking_level_name(level: ice_ai::types::ThinkingLevel) -> &'static str {
    use ice_ai::types::ThinkingLevel::*;
    match level {
        Off => "off",
        Minimal => "minimal",
        Low => "low",
        Medium => "medium",
        High => "high",
        Xhigh => "xhigh",
        Max => "max",
    }
}

/// Parse a thinking-level name back to the enum (case-insensitive). Returns
/// `None` for an unknown name; used by the `/thinking` selector callback.
pub(super) fn thinking_level_from_name(name: &str) -> Option<ice_ai::types::ThinkingLevel> {
    use ice_ai::types::ThinkingLevel::*;
    match name.to_ascii_lowercase().as_str() {
        "off" => Some(Off),
        "minimal" => Some(Minimal),
        "low" => Some(Low),
        "medium" => Some(Medium),
        "high" => Some(High),
        "xhigh" => Some(Xhigh),
        "max" => Some(Max),
        _ => None,
    }
}

/// Build + open the `/thinking` selector. Items are the levels the current
/// model supports (`Model::supported_thinking_levels`), each with a
/// description; the current level (read beforehand via `lane.get_thinking_level`)
/// is preselected. Selecting applies it live via `lane.set_thinking_level`.
///
/// `on_select` fires on the blocking key thread, so it can't await
/// `lane.get_thinking_level()` to know the current level — the opener resolves
/// it first (best-effort) and preselects; the toggle on_select just applies
/// whatever was picked.
pub(super) fn open_thinking_selector(
    state: &Arc<TuiState>,
    editor_container: &Arc<Container>,
    editor: &Arc<Editor>,
    tui: &Arc<TuiAltScreen>,
    lane: &Arc<dyn AgentLane>,
    catalog: &[ice_ai::Model],
    lane_model_id: &str,
    chat: &Arc<Container>,
) {
    // Find the current model in the catalog to read its supported levels. If
    // absent, fall back to all levels so the selector still opens.
    let model = catalog
        .iter()
        .find(|m| m.id.eq_ignore_ascii_case(lane_model_id));
    let levels: Vec<ice_ai::types::ThinkingLevel> = model
        .map(|m| m.supported_thinking_levels())
        .unwrap_or_else(|| {
            use ice_ai::types::ThinkingLevel::*;
            vec![Off, Minimal, Low, Medium, High]
        });
    let mut items: Vec<SelectItem> = Vec::new();
    for lvl in &levels {
        let name = thinking_level_name(*lvl);
        items.push(SelectItem::new(name, name).with_description(thinking_level_description(*lvl)));
    }
    if items.is_empty() {
        add_note_message(chat, "This model has no supported thinking levels.");
        tui.request_render(false);
        return;
    }
    let list = Arc::new(SelectList::new(items, 10));

    let state_sel = state.clone();
    let ec_sel = editor_container.clone();
    let editor_sel = editor.clone();
    let tui_sel = tui.clone();
    let chat_sel = chat.clone();
    let lane_sel = lane.clone();
    list.on_select(Arc::new(move |item| {
        let Some(level) = thinking_level_from_name(&item.value) else {
            add_note_message(
                &chat_sel,
                &format!("Unknown thinking level: {}.", item.label),
            );
            close_selector(&state_sel, &ec_sel, &editor_sel, &tui_sel);
            return;
        };
        let lane = lane_sel.clone();
        let state = state_sel.clone();
        let chat = chat_sel.clone();
        let tui = tui_sel.clone();
        tokio::spawn(async move {
            apply_thinking_selection(&lane, &state, &chat, level, false).await;
            tui.request_render(false);
        });
        close_selector(&state_sel, &ec_sel, &editor_sel, &tui_sel);
    }));
    let state_cancel = state.clone();
    let ec_cancel = editor_container.clone();
    let editor_cancel = editor.clone();
    let tui_cancel = tui.clone();
    list.on_cancel(Arc::new(move || {
        close_selector(&state_cancel, &ec_cancel, &editor_cancel, &tui_cancel);
    }));

    open_selector(
        state,
        editor_container,
        editor,
        tui,
        list,
        SelectorKind::Thinking,
    );
}

/// Build + open the `/tools` selector. Lists the 7 builtin tool names; each
/// visit reads the live active set via `lane.get_active_tools()` (best-effort,
/// resolved synchronously by the opener using `tokio::runtime::Handle` block_on
/// — the blocking key thread can't await) and selecting a tool **toggles** it
/// on/off via `lane.set_active_tools`. Active tools are marked `(on)`.
pub(super) fn open_tools_selector(
    state: &Arc<TuiState>,
    editor_container: &Arc<Container>,
    editor: &Arc<Editor>,
    tui: &Arc<TuiAltScreen>,
    lane: &Arc<dyn AgentLane>,
    chat: &Arc<Container>,
) {
    // Best-effort read of the current active set. The opener runs on the async
    // runtime (it's called from the main loop's channel dispatch or the submit
    // closure that lives on the blocking thread — but `handle.block_on` is safe
    // because `get_active_tools` is std-Mutex-backed and finishes quickly).
    let mut active = match tokio::runtime::Handle::try_current() {
        Ok(h) => h
            .block_on(async { lane.get_active_tools().await })
            .unwrap_or_default(),
        Err(_) => Vec::new(),
    };
    // An empty active set is the harness sentinel for "all registered tools"
    // (the selector only exposes built-ins). Expand it before rendering and
    // toggling so the first `/tools` visit does not show every tool as off or
    // accidentally reduce the active set to the one item selected.
    if active.is_empty() {
        active = crate::session::BUILTIN_TOOL_NAMES
            .iter()
            .map(|name| (*name).to_string())
            .collect();
    }
    let mut items: Vec<SelectItem> = Vec::new();
    for name in crate::session::BUILTIN_TOOL_NAMES {
        let on = active.iter().any(|a| a == name);
        let label = if on {
            format!("{name} (on)")
        } else {
            (*name).to_string()
        };
        items.push(SelectItem::new(name, &label).with_description("Toggle tool on/off"));
    }
    let list = Arc::new(SelectList::new(items, 10));

    // Capture the active set so on_select can toggle without re-reading.
    let active_captured = active.clone();
    let state_sel = state.clone();
    let ec_sel = editor_container.clone();
    let editor_sel = editor.clone();
    let tui_sel = tui.clone();
    let chat_sel = chat.clone();
    let lane_sel = lane.clone();
    list.on_select(Arc::new(move |item| {
        let mut next = active_captured.clone();
        if let Some(pos) = next.iter().position(|a| a == &item.value) {
            next.remove(pos);
        } else {
            next.push(item.value.clone());
        }
        let on = next.iter().any(|a| a == &item.value);
        let lane = lane_sel.clone();
        let next_clone = next.clone();
        tokio::spawn(async move {
            let _ = lane.set_active_tools(next_clone).await;
        });
        let list_str = if next.is_empty() {
            "(none)".to_string()
        } else {
            next.join(", ")
        };
        add_note_message(
            &chat_sel,
            &format!(
                "{} {} — active tools: {}",
                item.value,
                if on { "enabled" } else { "disabled" },
                list_str
            ),
        );
        close_selector(&state_sel, &ec_sel, &editor_sel, &tui_sel);
    }));
    let state_cancel = state.clone();
    let ec_cancel = editor_container.clone();
    let editor_cancel = editor.clone();
    let tui_cancel = tui.clone();
    list.on_cancel(Arc::new(move || {
        close_selector(&state_cancel, &ec_cancel, &editor_cancel, &tui_cancel);
    }));

    open_selector(
        state,
        editor_container,
        editor,
        tui,
        list,
        SelectorKind::Tools,
    );
}

/// Build + open the `/images` selector (Yes/No). Stores the choice in
/// `state.show_images` and notes it. Image wiring is minimal this pass — the
/// flag is consulted where images would be shown and echoed back here.
pub(super) fn open_images_selector(
    state: &Arc<TuiState>,
    editor_container: &Arc<Container>,
    editor: &Arc<Editor>,
    tui: &Arc<TuiAltScreen>,
    chat: &Arc<Container>,
) {
    let current = *state.show_images.lock().unwrap();
    let items = vec![
        SelectItem::new("yes", "Yes").with_description(if current {
            "Inline images (current)"
        } else {
            "Inline images"
        }),
        SelectItem::new("no", "No").with_description(if current {
            "Placeholder only"
        } else {
            "Placeholder only (current)"
        }),
    ];
    let list = Arc::new(SelectList::new(items, 5));

    let state_sel = state.clone();
    let ec_sel = editor_container.clone();
    let editor_sel = editor.clone();
    let tui_sel = tui.clone();
    let chat_sel = chat.clone();
    list.on_select(Arc::new(move |item| {
        let on = item.value == "yes";
        state_sel.set_show_images(on);
        add_note_message(
            &chat_sel,
            &format!("Inline images {}.", if on { "enabled" } else { "disabled" }),
        );
        close_selector(&state_sel, &ec_sel, &editor_sel, &tui_sel);
    }));
    let state_cancel = state.clone();
    let ec_cancel = editor_container.clone();
    let editor_cancel = editor.clone();
    let tui_cancel = tui.clone();
    list.on_cancel(Arc::new(move || {
        close_selector(&state_cancel, &ec_cancel, &editor_cancel, &tui_cancel);
    }));

    open_selector(
        state,
        editor_container,
        editor,
        tui,
        list,
        SelectorKind::Images,
    );
}
