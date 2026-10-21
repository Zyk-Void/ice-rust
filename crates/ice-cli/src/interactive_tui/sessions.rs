//! Session switching, import, fork, sharing, and transcript history rendering.

use super::*;

/// `/share`: mirror the TS intent (share the session). With the `gh` CLI on
/// PATH, create a gist of the exported markdown; otherwise fall back to the
/// clipboard (best-effort) and note the local path.
pub(super) async fn share_session(harness: &AgentHarness, chat: &Arc<Container>) {
    use std::process::Stdio;

    // Reuse the export builder for the transcript text.
    let tree = harness.session().view("main");
    let entries = match tree
        .find_entries(&EntryQuery {
            entry_type: None,
            custom_type: None,
            // Exports append entries top-to-bottom, so use chronological order
            // instead of the session query default (newest-first).
            order: Some(EntryOrder::OldestFirst),
            limit: None,
            cursor: None,
        })
        .await
    {
        Ok(e) => e,
        Err(e) => {
            add_error_message(chat, &format!("Could not read session: {e}"));
            return;
        }
    };
    let mut md = String::from("# Session\n\n");
    for e in entries {
        let Entry::Message(me) = e else { continue };
        match &me.message {
            AgentMessage::User(u) => {
                md.push_str(&format!("## User\n\n{}\n\n", user_message_text(u)));
            }
            AgentMessage::Assistant(a) => {
                let text = assistant_text(a);
                if !text.is_empty() {
                    md.push_str(&format!("## Assistant\n\n{}\n\n", text));
                }
            }
            _ => {}
        }
    }

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
                let url = String::from_utf8_lossy(&out.stdout).trim().to_string();
                add_note_message(chat, &format!("Shared session: {url}"));
                return;
            }
        }
        add_note_message(chat, "gh gist failed — falling back to the clipboard.");
    } else {
        add_note_message(chat, "gh CLI not found — falling back to the clipboard.");
    }
    // Clipboard fallback (or transcript echo when the clipboard feature is off).
    if copy_to_clipboard(&md) {
        add_note_message(chat, "Session transcript copied to the clipboard.");
    } else {
        add_note_message(
            chat,
            "Clipboard unavailable — use /export to write the transcript to a file.",
        );
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
    if !settle_active_run(lane, chat, state, "switch session").await {
        return false;
    }
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
            true
        }
        Err(e) => {
            state.set_status(RunStatus::Idle);
            add_error_message(chat, &format!("Could not open session {id}: {e}"));
            false
        }
    }
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
    let dir = crate::session::default_session_dir(cwd);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        add_error_message(chat, &format!("Could not create session dir: {e}"));
        return;
    }
    let dest = dir.join(fname);
    match std::fs::copy(src, &dest) {
        Ok(_) => {
            let id = fname.strip_suffix(".jsonl").unwrap_or(fname).to_string();
            if switch_to_session(harness, lane, &id, cwd, chat, state, tool_context).await {
                add_note_message(chat, &format!("Imported session from {path}"));
            }
        }
        Err(e) => add_error_message(chat, &format!("Could not copy import: {e}")),
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
    if !settle_active_run(lane, chat, state, "start a new session").await {
        return false;
    }
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

/// Abort an in-flight run and wait for the lane to go idle before the session
/// backing is replaced. Reports the failure as "Could not {action}: …".
async fn settle_active_run(
    lane: &Arc<dyn AgentLane>,
    chat: &Arc<Container>,
    state: &Arc<TuiState>,
    action: &str,
) -> bool {
    if *state.status.lock().unwrap() == RunStatus::Working {
        state.set_status(RunStatus::Aborting);
        let _ = lane.abort().await;
    }
    if let Err(error) = lane.wait_for_idle().await {
        add_error_message(chat, &format!("Could not {action}: {error}"));
        return false;
    }
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
    state.new_message_count.reset();
    state.run_started_at.lock().unwrap().take();
    *state.selection_start.lock().unwrap() = None;
    *state.selection_end.lock().unwrap() = None;
    chat.clear();
    add_welcome_message(chat);
    render_session_history(
        harness,
        chat,
        state.markdown_transformer(),
        Some(state.extension_session.clone()),
        state.images_visible(),
    )
    .await;
    state.tool_components.lock().unwrap().clear();
    state.bash_components.lock().unwrap().clear();
    let (totals, last_text) = session_usage_and_last_text(harness).await;
    state.footer.set_usage(totals);
    state.footer.set_cache_hit_rate(None);
    state.footer.set_context_tokens(None);
    *state.last_assistant_text.lock().unwrap() = last_text;
    state.set_status(RunStatus::Idle);
}

/// Usage summed over the session's assistant messages, plus the last
/// assistant text (for `/copy`).
async fn session_usage_and_last_text(
    harness: &AgentHarness,
) -> (ice_tui::footer::UsageTotals, String) {
    let mut totals = ice_tui::footer::UsageTotals::default();
    let mut last_text = String::new();
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
        if let Entry::Message(me) = entry {
            if let AgentMessage::Assistant(a) = &me.message {
                let u = &a.usage;
                totals.add(u.input, u.output, u.cache_read, u.cache_write, u.cost.total);
                let text = assistant_text(a);
                if !text.is_empty() {
                    last_text = text;
                }
            }
        }
    }
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
) {
    let tree = harness.session().view("main");
    let entries = match tree
        .find_entries(&EntryQuery {
            entry_type: None,
            custom_type: None,
            // Session queries default to newest-first for selectors and
            // pagination. The transcript appends children top-to-bottom, so
            // restored history must explicitly be chronological.
            order: Some(EntryOrder::OldestFirst),
            limit: None,
            cursor: None,
        })
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
                    crate::transcript_view::append_tool_panel(chat, comp, false);
                    for block in &result.content {
                        if let ice_ai::types::Content::Image(image) = block {
                            add_image_preview(chat, image, show_images);
                        }
                    }
                    chat.add_child(Arc::new(Spacer::new(1)));
                    rendered_any = true;
                }
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
