//! Session switching, import, fork, sharing, and transcript history rendering.

use super::*;

/// `/share`: mirror the TS intent (share the session). With the `gh` CLI on
/// PATH, create a gist of the exported markdown; otherwise fall back to the
/// clipboard (best-effort) and note the local path.
pub(super) async fn share_session(harness: &AgentHarness, chat: &Arc<Container>) {
    share_session_with_transports(harness, chat, create_session_gist, copy_to_clipboard).await;
}

// Keep process/clipboard effects at the boundary so scope and fallback tests
// never depend on gh credentials, PATH, a display server or a real clipboard.
async fn share_session_with_transports(
    harness: &AgentHarness,
    chat: &Arc<Container>,
    gist: impl FnOnce(&str) -> Result<String, &'static str>,
    clipboard: impl FnOnce(&str) -> bool,
) {
    let md = match crate::export::active_branch_markdown(harness).await {
        Ok(markdown) => markdown,
        Err(error) => {
            add_error_message(chat, &error);
            return;
        }
    };

    match gist(&md) {
        Ok(url) => {
            add_note_message(chat, &format!("Shared session: {url}"));
            return;
        }
        Err(note) => add_note_message(chat, note),
    }
    if clipboard(&md) {
        add_note_message(chat, "Session transcript copied to the clipboard.");
    } else {
        add_note_message(
            chat,
            "Clipboard unavailable — use /export to write the transcript to a file.",
        );
    }
}

fn create_session_gist(md: &str) -> Result<String, &'static str> {
    use std::process::Stdio;

    // `gh gist create` — stdin-piped, best-effort; only when gh exists.
    let gh = std::process::Command::new("gh")
        .arg("gist")
        .arg("create")
        .arg("--filename")
        .arg("session.md")
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    if let Ok(mut child) = gh {
        use std::io::Write;
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(md.as_bytes());
            let _ = stdin.flush();
        }
        let out = child.wait_with_output().ok();
        if let Some(out) = out {
            if out.status.success() {
                return Ok(String::from_utf8_lossy(&out.stdout).trim().to_string());
            }
        }
        Err("gh gist failed — falling back to the clipboard.")
    } else {
        Err("gh CLI not found — falling back to the clipboard.")
    }
}

// Session export is driven through the product-layer `AgentSession`
// (`crate::agent_session`), which owns format selection + default filenames —
// the TUI loop calls `AgentSession::export*` directly, so there is no separate
// helper here.

/// Hot-switch the harness to another saved session: abort any in-flight run,
/// open the target session file, swap the durable backing, and re-render the
/// transcript from the new history (mirrors pi's `/session` resume-in-place).
/// Shared by the `/session` selector and `/import`. The current model stays
/// put (v1 doesn't replay the session's ModelChange entries).
pub(super) async fn switch_to_session(
    harness: &AgentHarness,
    lane: &Arc<dyn AgentLane>,
    id: &str,
    cwd: &std::path::Path,
    chat: &Arc<Container>,
    state: &Arc<TuiState>,
    tool_context: &ice_extensions::ToolCallContext,
) -> bool {
    let Some(_transition) = settle_active_run(harness, lane, chat, state, "switch session").await
    else {
        return false;
    };
    activate_saved_session(harness, id, cwd, chat, state, tool_context).await
}

async fn activate_saved_session(
    harness: &AgentHarness,
    id: &str,
    cwd: &std::path::Path,
    chat: &Arc<Container>,
    state: &Arc<TuiState>,
    tool_context: &ice_extensions::ToolCallContext,
) -> bool {
    let cwd_str = cwd.to_string_lossy().to_string();
    match crate::session::open_session_by_id(id, &cwd_str).await {
        Ok(new_session) => {
            // Route through the funnel so the plugin channels follow the swap;
            // a bare `set_session` would leave every plugin naming the session
            // it was serving before this call.
            if let Err(error) =
                crate::session::set_active_session(harness, new_session, Some(tool_context)).await
            {
                add_error_message(chat, &format!("Could not switch session: {error}"));
                state.set_status(RunStatus::Idle);
                return false;
            }
            replace_session_ui(harness, chat, state).await;
            add_note_message(chat, &format!("Switched to session {id}."));
            if let Some(note) = other_directory_note(harness, cwd) {
                add_note_message(chat, &note);
            }
            true
        }
        Err(e) => {
            state.set_status(RunStatus::Idle);
            add_error_message(chat, &format!("Could not open session {id}: {e}"));
            false
        }
    }
}

/// Tools keep running in the directory icers was started in, not the one a
/// resumed session was created in; say so rather than let it surprise anyone.
fn other_directory_note(harness: &AgentHarness, cwd: &std::path::Path) -> Option<String> {
    let path = harness.session().storage().persistence_path()?;
    let created_in = crate::session_catalog::header_cwd(std::path::Path::new(&path))?;
    let here = crate::session_catalog::canonical(cwd);
    (created_in != here).then(|| {
        format!(
            "This session was created in {}. Tools still run in {}.",
            created_in.display(),
            here.display()
        )
    })
}

/// `/import <path>`: copy a JSONL session file into the default session dir,
/// then hot-switch to it (the file name becomes its id — matching the
/// selector/`open_session_by_id` containment rules).
pub(super) async fn import_session(
    harness: &AgentHarness,
    lane: &Arc<dyn AgentLane>,
    path: &str,
    cwd: &std::path::Path,
    chat: &Arc<Container>,
    state: &Arc<TuiState>,
    tool_context: &ice_extensions::ToolCallContext,
) {
    use std::path::Path as FsPath;

    let src = FsPath::new(path);
    if !src.is_file() {
        add_error_message(chat, &format!("Import source not found: {path}"));
        return;
    }
    let Some(fname) = src.file_name().and_then(|f| f.to_str()) else {
        add_error_message(chat, "Import source has no file name.");
        return;
    };
    if !fname.ends_with(".jsonl") {
        add_error_message(chat, "Import source must be a .jsonl session file.");
        return;
    }
    // Settle before importing: a failed drain or active bash must not switch
    // away from a still-running session.
    let Some(_transition) = settle_active_run(harness, lane, chat, state, "import session").await
    else {
        return;
    };
    match crate::session_import::import_session_file(src, cwd).await {
        Ok(id) => {
            if activate_saved_session(harness, &id, cwd, chat, state, tool_context).await {
                add_note_message(chat, &format!("Imported session from {path}"));
            }
        }
        Err(error) => add_error_message(chat, &format!("Could not import session: {error}")),
    }
}

/// Fork the current session into a new one per `options` and switch to it.
pub(super) async fn fork_session(
    harness: &AgentHarness,
    cwd: &std::path::Path,
    chat: &Arc<Container>,
    state: &Arc<TuiState>,
    tool_context: &ice_extensions::ToolCallContext,
    options: &ice_harness::session::types::ForkOptions,
) -> bool {
    if *state.status.lock().unwrap() != RunStatus::Idle {
        add_note_message(chat, "Wait for the current run to finish before forking.");
        return false;
    }
    let Some(_transition) = begin_session_change(chat, state, "fork session") else {
        return false;
    };
    match harness.snapshot().await {
        Ok(snapshot) if snapshot.active.is_none() => {}
        Ok(_) => {
            add_note_message(chat, "Wait for the current run to finish before forking.");
            return false;
        }
        Err(error) => {
            add_error_message(chat, &format!("Could not fork session: {error}"));
            return false;
        }
    }
    if !settle_agent_events(harness, chat, state, "fork session").await {
        return false;
    }
    let new_session = match crate::session::fork_session_storage_with_options(
        harness,
        &cwd.to_string_lossy(),
        options,
    )
    .await
    {
        Ok(session) => session,
        Err(error) => {
            add_error_message(chat, &format!("Could not fork session: {error}"));
            return false;
        }
    };
    if let Err(error) =
        crate::session::set_active_session(harness, new_session, Some(tool_context)).await
    {
        add_error_message(chat, &format!("Could not fork session: {error}"));
        return false;
    }
    replace_session_ui(harness, chat, state).await;
    add_note_message(chat, "Forked into a new session.");
    true
}

/// Validate a picker result against the active file before creating
/// a fork. Return the chosen prompt only after the session switch succeeds.
pub(super) async fn fork_before_message(
    harness: &AgentHarness,
    cwd: &std::path::Path,
    chat: &Arc<Container>,
    state: &Arc<TuiState>,
    tool_context: &ice_extensions::ToolCallContext,
    selection: &ForkSelection,
) -> Option<String> {
    let active = harness.session();
    if active.storage().metadata().id != selection.session_id
        || active.storage().persistence_path() != selection.session_path
    {
        add_note_message(
            chat,
            "Session changed; open /fork again to choose a message.",
        );
        return None;
    }
    let Some(text) = fork_message_text(harness, &selection.entry_id).await else {
        add_note_message(chat, "Message is no longer available; open /fork again.");
        return None;
    };
    fork_session(
        harness,
        cwd,
        chat,
        state,
        tool_context,
        &ice_harness::session::types::ForkOptions::Branch {
            entry_id: Some(selection.entry_id.clone()),
            position: Some(ice_harness::session::types::ForkPosition::Before),
        },
    )
    .await
    .then_some(text)
}

/// Restore all prompt text, including newlines, and place the byte-based editor
/// cursor after its final character.
pub(super) fn restore_fork_prompt(editor: &Arc<Editor>, text: &str) {
    editor.set_text(text);
    editor.set_cursor(
        text.matches('\n').count(),
        text.rsplit('\n').next().map_or(0, str::len),
    );
}

/// `/new`: start an empty session in the current session directory (ice
/// `AgentSessionRuntime.newSession`). Like ice's `teardownCurrent`, an
/// in-flight run is aborted and settled before the backing is replaced, so
/// the old session keeps every entry it had and the agent starts with no
/// history.
pub(super) async fn start_new_session(
    harness: &AgentHarness,
    lane: &Arc<dyn AgentLane>,
    cwd: &std::path::Path,
    chat: &Arc<Container>,
    state: &Arc<TuiState>,
    tool_context: &ice_extensions::ToolCallContext,
) -> bool {
    let Some(_transition) =
        settle_active_run(harness, lane, chat, state, "start a new session").await
    else {
        return false;
    };
    let active_dir = harness
        .session()
        .storage()
        .persistence_path()
        .and_then(|path| {
            std::path::Path::new(&path)
                .parent()
                .map(std::path::Path::to_path_buf)
        });
    let dir = active_dir
        .clone()
        .unwrap_or_else(|| crate::session::default_session_dir(cwd));
    if let Err(error) = std::fs::create_dir_all(&dir) {
        add_error_message(chat, &format!("Could not create session dir: {error}"));
        state.set_status(RunStatus::Idle);
        return false;
    }
    let cwd_str = cwd.to_string_lossy().to_string();
    let session = if active_dir.is_some() {
        crate::session::create_jsonl_session_in_directory(&dir, &cwd_str).await
    } else {
        crate::session::create_jsonl_session(&dir, &cwd_str).await
    };
    let created = match session {
        Ok(session) => {
            crate::session::set_active_session(harness, session, Some(tool_context)).await
        }
        Err(error) => Err(error),
    };
    if let Err(error) = created {
        add_error_message(chat, &format!("Could not start a new session: {error}"));
        state.set_status(RunStatus::Idle);
        return false;
    }
    replace_session_ui(harness, chat, state).await;
    add_note_message(chat, "✓ New session started");
    true
}

/// A failed abort leaves the run going, so the status must not stay on
/// "Aborting". Events may have moved the status on since; only undo ours.
fn restore_working_after_failed_abort(state: &Arc<TuiState>) {
    let mut status = state.status.lock().unwrap();
    if *status == RunStatus::Aborting {
        *status = RunStatus::Working;
    }
}

/// Abort an in-flight run and wait for the lane to go idle before the session
/// backing is replaced. Reports the failure as "Could not {action}: …".
async fn settle_active_run(
    harness: &AgentHarness,
    lane: &Arc<dyn AgentLane>,
    chat: &Arc<Container>,
    state: &Arc<TuiState>,
    action: &str,
) -> Option<SessionTransition> {
    let transition = begin_session_change(chat, state, action)?;
    // Extensions can start a run before the AgentStart broadcast reaches the
    // UI. Consult the authoritative lane rather than waiting forever on a run
    // the display still believes is idle.
    let active = match lane.snapshot().await {
        Ok(snapshot) => snapshot.active.is_some(),
        Err(error) => {
            add_error_message(chat, &format!("Could not {action}: {error}"));
            return None;
        }
    };
    if active {
        state.set_status(RunStatus::Aborting);
        match lane.abort().await {
            Ok(_) | Err(ice_harness::result::HarnessError::NoActiveRun { .. }) => {}
            Err(error) => {
                restore_working_after_failed_abort(state);
                add_error_message(chat, &format!("Could not {action}: {error}"));
                return None;
            }
        }
    }
    if let Err(error) = lane.wait_for_idle().await {
        if active {
            restore_working_after_failed_abort(state);
        }
        add_error_message(chat, &format!("Could not {action}: {error}"));
        return None;
    }
    if !settle_agent_events(harness, chat, state, action).await {
        return None;
    }
    Some(transition)
}

/// Owns admission until both the backing and its UI have been replaced. The
/// key thread cannot start a user bash in the await gap between guard and swap.
pub(super) struct SessionTransition(Arc<TuiState>);

impl Drop for SessionTransition {
    fn drop(&mut self) {
        self.0
            .event_drain
            .session_changing
            .store(false, std::sync::atomic::Ordering::Release);
    }
}

pub(super) fn begin_session_change(
    chat: &Arc<Container>,
    state: &Arc<TuiState>,
    action: &str,
) -> Option<SessionTransition> {
    // Match begin_user_bash's lock order so checking bash and claiming the
    // transition is atomic with respect to key-thread command admission.
    let bash = state.user_bash_cancel.lock().unwrap();
    if bash.is_some() {
        add_note_message(
            chat,
            &format!("Wait for the bash command to finish or cancel it before you {action}."),
        );
        return None;
    }
    if state
        .event_drain
        .session_changing
        .swap(true, std::sync::atomic::Ordering::AcqRel)
    {
        add_note_message(chat, "Wait for the current session change to finish.");
        return None;
    }
    Some(SessionTransition(state.clone()))
}

async fn settle_agent_events(
    harness: &AgentHarness,
    chat: &Arc<Container>,
    state: &Arc<TuiState>,
    action: &str,
) -> bool {
    if let Err(error) = state.event_drain.flush().await {
        add_error_message(chat, &format!("Could not {action}: {error}"));
        return false;
    }
    // Shell results buffered during the old run belong to the still-selected
    // session; write them now, before the backing can change.
    let session_id = harness.session().storage().metadata().id;
    super::run::flush_pending_bash_messages(harness, &harness.lane("main"), state, &session_id)
        .await;
    true
}

/// Rebuild everything the screen shows about the session after its backing
/// was replaced (`/new`, `/resume`, `/import`, `/fork`): the transcript, the
/// footer's usage totals, and the per-run caches that pointed at the old
/// transcript. Without the reset the footer kept counting the previous
/// session's tokens and cost.
pub(super) async fn replace_session_ui(
    harness: &AgentHarness,
    chat: &Arc<Container>,
    state: &Arc<TuiState>,
) {
    state.current_assistant.lock().unwrap().take();
    state.cache_tracker.lock().unwrap().reset();
    state.clear_auto_send();
    state.new_message_count.reset();
    *state.selection_gesture.lock().unwrap() = SelectionGesture::Idle;
    if let Some(tui) = &state.tui {
        tui.set_selection(None);
        tui.clear_screen_selection();
    }
    // The startup banner reads live data, so it is kept as is (in ice it lives
    // in a header container that a session switch does not clear).
    let header = chat
        .get_children()
        .into_iter()
        .find(|child| child.as_any().is::<SessionHeader>());
    chat.clear();
    if let Some(header) = header {
        add_welcome_message(chat, header);
    }
    render_session_history(
        harness,
        chat,
        state.markdown_transformer(),
        Some(state.extension_session.clone()),
        state.images_visible(),
        state.transcript_view().expanded(),
    )
    .await;
    state.tool_components.lock().unwrap().clear();
    state.bash_components.lock().unwrap().clear();
    let (totals, last_text) = session_usage_and_last_text(harness).await;
    state.footer.set_usage(totals);
    state.footer.set_cache_hit_rate(None);
    state.footer.set_context_tokens(None);
    record_context_usage(&state.footer, harness).await;
    *state.last_assistant_text.lock().unwrap() = last_text;
    // The harness rebuilds queued prompts from the selected session's durable
    // records. Reconcile the dock with that queue, including an empty /new.
    state.set_pending_queue(harness.queued_messages().await.unwrap_or_default());
    state.set_status(RunStatus::Idle);
}

/// Usage summed over session responses and paid summaries, plus the last assistant
/// text on the active branch (for `/copy`).
async fn session_usage_and_last_text(
    harness: &AgentHarness,
) -> (ice_tui::footer::UsageTotals, String) {
    let mut totals = ice_tui::footer::UsageTotals::default();
    let entries = harness
        .session()
        .view("main")
        .find_entries(&EntryQuery {
            entry_type: None,
            custom_type: None,
            order: Some(EntryOrder::OldestFirst),
            limit: None,
            cursor: None,
        })
        .await
        .unwrap_or_default();
    for entry in entries {
        let usage = match &entry {
            Entry::Message(message) => match &message.message {
                AgentMessage::Assistant(assistant) => Some(&assistant.usage),
                _ => None,
            },
            Entry::BranchSummary(summary) => summary.usage.as_ref(),
            Entry::Compaction(summary) => summary.usage.as_ref(),
            _ => None,
        };
        if let Some(u) = usage {
            totals.add(u.input, u.output, u.cache_read, u.cache_write, u.cost.total);
        }
    }
    // Billing is session-wide: changing branches must not erase paid usage.
    // /copy, however, describes the selected conversation, not an orphaned future.
    let last_text = harness
        .session()
        .view("main")
        .find_entries_on_branch(
            &EntryQuery {
                order: Some(EntryOrder::NewestFirst),
                ..Default::default()
            },
            &Default::default(),
        )
        .await
        .unwrap_or_default()
        .into_iter()
        .find_map(|entry| match entry {
            Entry::Message(message) => match message.message {
                AgentMessage::Assistant(assistant) => {
                    let text = assistant_text(&assistant);
                    (!text.is_empty()).then_some(text)
                }
                _ => None,
            },
            _ => None,
        })
        .unwrap_or_default();
    (totals, last_text)
}

/// Render the restored session's prior transcript (user + assistant messages)
/// into the chat container. Called at TUI startup for `--continue`/`--resume`/
/// `--session` launches; a no-op for fresh sessions (no entries). Best-effort:
/// any session read failure just starts with an empty transcript.
///
/// `transformer` is the live assistant-markdown transformer (B5e); `None` is
/// the identity path. Each restored assistant component installs it so replayed
/// history renders through the same `register_markdown_transformer` handlers
/// the live stream does.
pub(super) async fn render_session_history(
    harness: &AgentHarness,
    chat: &Arc<Container>,
    transformer: Option<MarkdownTransformer>,
    extension_session: Option<crate::session::ExtensionSessionCell>,
    show_images: bool,
    tool_outputs_expanded: bool,
) {
    let tree = harness.session().view("main");
    let entries = match tree
        .find_entries_on_branch(
            &EntryQuery {
                entry_type: None,
                custom_type: None,
                // Session queries default to newest-first for selectors and
                // pagination. The transcript appends children top-to-bottom, so
                // restored history must explicitly be chronological.
                order: Some(EntryOrder::OldestFirst),
                limit: None,
                cursor: None,
            },
            &Default::default(),
        )
        .await
    {
        Ok(e) => e,
        Err(_) => return,
    };
    let mut rendered_any = false;
    let mut tool_arguments = std::collections::HashMap::new();
    for e in entries {
        match e {
            Entry::Message(me) => match &me.message {
                AgentMessage::User(u) => {
                    add_user_message(chat, &user_message_text(u));
                    add_user_images(chat, u, show_images);
                    rendered_any = true;
                }
                AgentMessage::Assistant(a) => {
                    for block in &a.content {
                        if let ice_ai::types::Content::ToolCall(call) = block {
                            tool_arguments.insert(call.id.clone(), call.arguments.clone());
                        }
                    }
                    let comp = Arc::new(AssistantMessageComponent::new(
                        AssistantMessageOptions::default(),
                    ));
                    comp.set_show_images(show_images);
                    comp.set_trailing_spacing(1);
                    if let Some(t) = &transformer {
                        comp.set_markdown_transformer(Some(t.clone()));
                    }
                    comp.update_blocks(&assistant_blocks(a));
                    chat.add_child(comp);
                    // The component adds spacing only when it has visible content.
                    if let Some(text) = extension_usage_text(extension_session.as_ref(), &a.usage) {
                        add_note_message(chat, &text);
                    }
                    if let Some(error) = assistant_error_text(a) {
                        add_error_message(chat, &error);
                    }
                    rendered_any = true;
                }
                AgentMessage::ToolResult(result) => {
                    if matches!(
                        result.tool_name.as_str(),
                        "plan_mode_start" | "plan_mode_complete"
                    ) && !result.is_error
                        && crate::transcript_view::update_plan_panel(
                            chat,
                            &serde_json::json!({"details":result.details}),
                        )
                    {
                        rendered_any = true;
                        continue;
                    }
                    if result.tool_name == "todo"
                        && !result.is_error
                        && crate::transcript_view::update_todo_list(
                            chat,
                            &serde_json::json!({
                                "content": result.content, "details": result.details,
                            }),
                        )
                    {
                        rendered_any = true;
                        continue;
                    }
                    // Tool results are persisted as separate message entries,
                    // not as part of the assistant text. Restore them as
                    // completed tool panels so resumed sessions show the
                    // command output as well as the user's prompts.
                    let args = tool_arguments
                        .remove(&result.tool_call_id)
                        .map(|args| args.to_string())
                        .unwrap_or_default();
                    let comp = Arc::new(ToolExecutionComponent::new(&result.tool_name, &args));
                    comp.set_result(&tool_result_message_text(result), result.is_error);
                    if tool_result_requests_markdown(result.details.as_ref()) {
                        comp.set_result_markdown(true);
                    }
                    crate::transcript_view::append_tool_panel(chat, comp, tool_outputs_expanded);
                    for block in &result.content {
                        if let ice_ai::types::Content::Image(image) = block {
                            add_image_preview(chat, image, show_images);
                        }
                    }
                    chat.add_child(Arc::new(Spacer::new(1)));
                    rendered_any = true;
                }
                AgentMessage::Custom(custom) if is_hidden_custom(custom) => {}
                AgentMessage::Custom(custom) => {
                    if let Some(session) = &extension_session {
                        if let Some(component) = extension_message_component(
                            session,
                            &custom.role,
                            &serde_json::json!({
                                "customType": custom.role,
                                "content": custom.content,
                                "details": custom.data,
                            }),
                            transformer.clone(),
                        ) {
                            chat.add_child(component);
                            chat.add_child(Arc::new(Spacer::new(1)));
                            rendered_any = true;
                            continue;
                        }
                    }
                    add_note_message(chat, &custom_message_fallback(&custom));
                    rendered_any = true;
                }
            },
            Entry::Compaction(compaction) => {
                add_note_message(
                    chat,
                    &format!(
                        "Compacted {} tokens: {}",
                        compaction.tokens_before, compaction.summary
                    ),
                );
                rendered_any = true;
            }
            Entry::BranchSummary(summary) => {
                add_note_message(chat, &format!("Branch summary: {}", summary.summary));
                rendered_any = true;
            }
            Entry::Custom(custom) => {
                if custom.custom_type == "plan-mode" {
                    let mut details = custom.data.clone().unwrap_or(serde_json::Value::Null);
                    if let Some(fields) = details.as_object_mut() {
                        fields.insert("kind".into(), serde_json::json!("plan"));
                    }
                    if crate::transcript_view::update_plan_panel(
                        chat,
                        &serde_json::json!({"details":details}),
                    ) {
                        rendered_any = true;
                        continue;
                    }
                }
                let rendered = extension_session.as_ref().and_then(|session| {
                    extension_entry_component(session, &custom.custom_type, custom.data.clone())
                });
                if let Some(component) = rendered {
                    chat.add_child(component);
                    chat.add_child(Arc::new(Spacer::new(1)));
                    rendered_any = true;
                } else if let Some(text) =
                    custom_entry_display_text(&custom.custom_type, custom.data.as_ref())
                {
                    add_note_message(chat, &text);
                    rendered_any = true;
                }
            }
            Entry::ModelChange(change) => {
                add_note_message(
                    chat,
                    &format!("Model changed to {}:{}", change.provider, change.model_id),
                );
                rendered_any = true;
            }
            Entry::ThinkingLevel(change) => {
                add_note_message(
                    chat,
                    &format!("Thinking level: {:?}", change.thinking_level),
                );
                rendered_any = true;
            }
            Entry::ActiveTools(change) => {
                add_note_message(
                    chat,
                    &format!("Active tools: {}", change.active_tool_names.join(", ")),
                );
                rendered_any = true;
            }
        }
    }
    // Replay includes panels (such as plan/todo) created outside the live tool
    // maps. Keep their initial display in sync with the saved/current toggle.
    crate::transcript_view::set_tool_panels_expanded(chat, tool_outputs_expanded);
    if rendered_any {
        // No trailing spacer here — each entry already adds its own trailing
        // Spacer(1), so an extra would double the bottom gap.
    }
}

pub(super) fn invoke_extension_renderer(
    session: &crate::session::ExtensionSessionCell,
    kind: ice_extensions::RegisteredRendererKind,
    payload: &serde_json::Value,
) -> Option<serde_json::Value> {
    let snapshot = session.lock().ok()?.snapshot_arc()?;
    let input = serde_json::to_string(payload).ok()?;
    for renderer in snapshot.renderers_of(kind) {
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut out = ice_plugin_sdk::StbString::empty();
            let rc = (renderer.render_fn)(
                ice_plugin_sdk::StbStringRef::from_str(&input),
                &mut out as *mut ice_plugin_sdk::StbString,
                renderer.user_data,
            );
            // SAFETY: the callback transfers a live output to its producer.
            let out = unsafe {
                ice_plugin_sdk::callbacks::OwnedString::from_raw(
                    out,
                    Some(renderer.plugin_free_string),
                )
            };
            let text = if rc == 0 {
                Some(out.to_string_lossy())
            } else {
                None
            };
            text
        }))
        .ok()
        .flatten();
        let Some(text) = outcome else { continue };
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
            return Some(value);
        }
    }
    None
}

pub(super) fn extension_text_component(
    value: &serde_json::Value,
) -> Option<Arc<dyn ice_tui::Component>> {
    if let Some(lines) = value.get("lines").and_then(|v| v.as_array()) {
        let text = lines
            .iter()
            .filter_map(|line| line.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        return Some(Arc::new(Text::new(text, 0, 0)));
    }
    let text = value.get("text").and_then(|v| v.as_str())?;
    if value.get("markdown").and_then(|v| v.as_bool()) == Some(true) {
        let component = Arc::new(AssistantMessageComponent::new(
            AssistantMessageOptions::default(),
        ));
        component.update_blocks(&[AssistantBlock::Text(text.to_string())]);
        Some(component)
    } else {
        Some(Arc::new(Text::new(text, 0, 0)))
    }
}

pub(super) fn extension_message_component(
    session: &crate::session::ExtensionSessionCell,
    custom_type: &str,
    payload: &serde_json::Value,
    transformer: Option<MarkdownTransformer>,
) -> Option<Arc<dyn ice_tui::Component>> {
    let value = invoke_extension_renderer(
        session,
        ice_extensions::RegisteredRendererKind::Message,
        payload,
    )?;
    if value.get("markdown").and_then(|v| v.as_bool()) == Some(true) {
        let text = value.get("text").and_then(|v| v.as_str())?;
        let component = Arc::new(AssistantMessageComponent::new(
            AssistantMessageOptions::default(),
        ));
        if let Some(transformer) = transformer {
            component.set_markdown_transformer(Some(transformer));
        }
        component.update_blocks(&[AssistantBlock::Text(text.to_string())]);
        return Some(component);
    }
    extension_text_component(&value)
        .or_else(|| Some(Arc::new(Text::new(format!("[{custom_type}]"), 0, 0))))
}

/// Render usage from a completed assistant message through the registered
/// message renderers. Hosts without a token-usage renderer return `None`.
pub(super) fn extension_usage_text(
    session: Option<&crate::session::ExtensionSessionCell>,
    usage: &ice_ai::types::Usage,
) -> Option<String> {
    let session = session?;
    let payload = serde_json::json!({
        "customType": "token-usage",
        "usage": usage,
    });
    let value = invoke_extension_renderer(
        session,
        ice_extensions::RegisteredRendererKind::Message,
        &payload,
    )?;
    value
        .get("text")
        .and_then(|value| value.as_str())
        .filter(|text| !text.trim().is_empty())
        .map(ToOwned::to_owned)
}

pub(super) fn extension_entry_component(
    session: &crate::session::ExtensionSessionCell,
    custom_type: &str,
    data: Option<serde_json::Value>,
) -> Option<Arc<dyn ice_tui::Component>> {
    let payload = serde_json::json!({
        "customType": custom_type,
        "data": data,
    });
    let value = invoke_extension_renderer(
        session,
        ice_extensions::RegisteredRendererKind::Entry,
        &payload,
    )?;
    extension_text_component(&value)
}

/// Project an assistant message's content into the provider-free
/// [`AssistantBlock`] list (text, thinking, and decoded image blocks, in
/// document order) the `AssistantMessageComponent` renders. Tool-call blocks
/// are rendered by their own components in the transcript.
/// Whether startup intentionally opened a session that already has history.
pub(super) fn launch_restores_history(args: &Args) -> bool {
    args.continue_session
        || args.resume
        || args.session.is_some()
        || args.session_id.is_some()
        || args.fork.is_some()
}

/// A custom message stored with `display: false` (extension context such as
/// recalled memory) is model input only, never a transcript row.
pub(super) fn is_hidden_custom(custom: &ice_agent::CustomMessage) -> bool {
    custom.role == ice_harness::messages::CUSTOM_ROLE
        && custom
            .data
            .get("display")
            .and_then(|display| display.as_bool())
            == Some(false)
}

pub(super) fn custom_message_fallback(custom: &ice_agent::CustomMessage) -> String {
    let content = custom
        .content
        .iter()
        .filter_map(|item| match item {
            Content::Text(text) => Some(text.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    if content.is_empty() {
        format!("{}: {}", custom.role, custom.data)
    } else {
        format!("{}: {}", custom.role, content)
    }
}

/// Path to the `HEAD` file of the repository or worktree enclosing `cwd`.
///
/// `<dir>/.git` is a directory in a normal checkout and a `gitdir: <path>`
/// file in a linked worktree or submodule; the path is relative to `<dir>`
/// unless absolute. Following the file matters: walking past it would find an
/// outer repository's HEAD and show that repository's branch.
pub(super) fn find_git_head_path(cwd: &std::path::Path) -> Option<std::path::PathBuf> {
    let mut dir = Some(cwd);
    while let Some(current) = dir {
        let dot_git = current.join(".git");
        if dot_git.is_dir() {
            let head = dot_git.join("HEAD");
            return head.is_file().then_some(head);
        } else if dot_git.is_file() {
            let contents = std::fs::read_to_string(&dot_git).ok()?;
            let gitdir = contents.trim().strip_prefix("gitdir:")?.trim();
            if gitdir.is_empty() {
                return None;
            }
            let head = current.join(gitdir).join("HEAD");
            return head.is_file().then_some(head);
        }
        dir = current.parent();
    }
    None
}

/// Current git branch for `cwd`, or `None` outside a repository / on error.
/// Reads `HEAD` directly (no `git` subprocess) so the footer can call it
/// cheaply at startup. A branch keeps its full name (`feat/x`); a detached
/// HEAD shows the first 7 characters of the commit.
pub(super) fn git_branch_for(cwd: &std::path::Path) -> Option<String> {
    let contents = std::fs::read_to_string(find_git_head_path(cwd)?).ok()?;
    let trimmed = contents.trim();
    if let Some(reference) = trimmed.strip_prefix("ref:") {
        let reference = reference.trim();
        let branch = reference.strip_prefix("refs/heads/").unwrap_or(reference);
        return (!branch.is_empty()).then(|| branch.to_string());
    }
    (!trimmed.is_empty()).then(|| trimmed.chars().take(7).collect())
}

/// The name displayed for a model id (last path segment / after the final
/// `:`), to keep the footer compact.
pub(super) fn short_model_name(id: &str) -> String {
    id.rsplit([':', '/'])
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(id)
        .to_string()
}

#[cfg(test)]
mod share_tests {
    use super::*;
    use ice_harness::session::types::{ProvisionedEntry, ProvisionedKind};

    async fn branched_harness() -> AgentHarness {
        let harness = AgentHarness::create(Default::default()).await.unwrap();
        let session = harness.session();
        let root = session
            .append_message(ice_agent::AgentMessage::User(
                ice_ai::types::UserMessage::new("SHARED_ROOT", 1),
            ))
            .await
            .unwrap();
        session
            .append_message(ice_agent::AgentMessage::User(
                ice_ai::types::UserMessage::new("ABANDONED_SECRET_SENTINEL", 2),
            ))
            .await
            .unwrap();
        session.move_lane("main", Some(&root)).await.unwrap();
        session
            .append_entry(
                ProvisionedEntry {
                    id: "checkpoint".into(),
                    kind: ProvisionedKind::Compaction {
                        summary: "ACTIVE_CHECKPOINT".into(),
                        retained_tail: Vec::new(),
                        tokens_before: 100,
                        details: None,
                        usage: None,
                    },
                },
                "main",
            )
            .await
            .unwrap();
        session
            .append_message(ice_agent::AgentMessage::User(
                ice_ai::types::UserMessage::new("ACTIVE_TAIL", 3),
            ))
            .await
            .unwrap();
        harness
    }

    #[tokio::test]
    async fn gist_receives_only_active_branch_and_success_skips_clipboard() {
        let harness = branched_harness().await;
        let chat = Arc::new(Container::new());
        let before = harness
            .session()
            .find_entries(&Default::default())
            .await
            .unwrap();
        let expected = crate::export::active_branch_markdown(&harness)
            .await
            .unwrap();
        share_session_with_transports(
            &harness,
            &chat,
            |payload| {
                assert_eq!(payload, expected);
                for marker in ["SHARED_ROOT", "ACTIVE_CHECKPOINT", "ACTIVE_TAIL"] {
                    assert!(payload.contains(marker), "missing {marker}");
                }
                assert!(!payload.contains("ABANDONED_SECRET_SENTINEL"));
                Ok("https://example.invalid/stub-gist".into())
            },
            |_| panic!("clipboard must not run after successful gist"),
        )
        .await;
        let rendered = chat.render(160).join("\n");
        assert!(
            rendered.contains("Shared session: https://example.invalid/stub-gist"),
            "{rendered}"
        );
        assert_eq!(
            harness
                .session()
                .find_entries(&Default::default())
                .await
                .unwrap(),
            before
        );
    }

    #[tokio::test]
    async fn gist_failure_copies_identical_scoped_payload_or_reports_unavailable() {
        let harness = branched_harness().await;
        let expected = crate::export::active_branch_markdown(&harness)
            .await
            .unwrap();
        for available in [true, false] {
            let chat = Arc::new(Container::new());
            let gist_called = std::cell::Cell::new(false);
            let clipboard_called = std::cell::Cell::new(false);
            share_session_with_transports(
                &harness,
                &chat,
                |payload| {
                    assert_eq!(payload, expected);
                    assert!(!payload.contains("ABANDONED_SECRET_SENTINEL"));
                    gist_called.set(true);
                    Err("stub gist failed — falling back to the clipboard.")
                },
                |payload| {
                    assert!(gist_called.get());
                    assert_eq!(payload, expected);
                    clipboard_called.set(true);
                    available
                },
            )
            .await;
            assert!(gist_called.get() && clipboard_called.get());
            let rendered = chat.render(160).join("\n");
            assert!(rendered.contains("stub gist failed"), "{rendered}");
            assert!(!rendered.contains("Shared session:"), "{rendered}");
            if available {
                assert!(rendered.contains("Session transcript copied"), "{rendered}");
            } else {
                assert!(rendered.contains("Clipboard unavailable"), "{rendered}");
                assert!(
                    !rendered.contains("Session transcript copied"),
                    "{rendered}"
                );
            }
        }
    }
}
