//! Prompt execution, completion reconciliation, compaction, and clipboard operations.

use super::*;

pub(super) async fn maybe_add_startup_changelog(
    harness: &AgentHarness,
    chat: &Arc<Container>,
    settings: &crate::settings::Settings,
    collapse: bool,
    existing_profile: bool,
) {
    let has_messages = harness
        .session()
        .find_entry_on_branch(
            &EntryQuery {
                entry_type: Some("message"),
                ..Default::default()
            },
            &Default::default(),
        )
        .await
        .map(|entry| entry.is_some())
        .unwrap_or(true);
    let entries = crate::changelog::bundled_entries();
    let baseline = crate::changelog::ice_baseline(
        &entries,
        settings.last_ice_changelog_version.as_deref(),
        settings.last_changelog_version.as_deref(),
    );
    let mut notice = crate::changelog::startup_changelog_for_profile(
        &entries,
        baseline,
        crate::VERSION,
        has_messages,
        existing_profile,
    );
    // Copy an identified icers legacy baseline even when there is no new notice.
    // On resume this copies only the old version, so unseen updates stay pending.
    if notice.version_to_record.is_none() && settings.last_ice_changelog_version.is_none() {
        notice.version_to_record = baseline.map(str::to_owned);
    }
    if let Some(markdown) = &notice.markdown {
        add_startup_changelog_message(chat, markdown, crate::VERSION, collapse);
    }
    if let Some(version) = notice.version_to_record {
        let saved = crate::settings::load_settings()
            .map_err(|error| error.to_string())
            .and_then(|mut current| {
                let candidate =
                    semver::Version::parse(&version).map_err(|error| error.to_string())?;
                if current
                    .last_ice_changelog_version
                    .as_deref()
                    .and_then(|seen| semver::Version::parse(seen).ok())
                    .is_some_and(|seen| seen >= candidate)
                {
                    return Ok(());
                }
                current.last_ice_changelog_version = Some(version);
                crate::settings::save_settings(&current)
            });
        if let Err(error) = saved {
            tracing::warn!("Could not record changelog version: {error}");
        }
    }
}

// ===========================================================================
// Run a single prompt (streaming or blocking)
// ===========================================================================

/// Ctrl+G: hand the terminal to the user's editor for the current draft, then
/// take it back (ice `handleOpenExternalEditor`: `ui.stop()`, edit,
/// `ui.start()`). Runs on the key-dispatch thread, like Ctrl+Z, so nothing
/// reads stdin or paints while the editor owns the terminal.
pub(super) fn run_external_editor(
    tui: &Arc<TuiAltScreen>,
    draft: String,
    tx: &mpsc::UnboundedSender<TuiMessage>,
) {
    let command = external_editor_command();
    tui.suspend();
    let _ = crossterm::execute!(std::io::stdout(), crossterm::cursor::Show);
    let result = edit_in_external_editor(&command, &draft);
    tui.resume();
    let _ = tx.send(TuiMessage::ExternalEditorResult(result));
}

/// `ICERS_EXTERNAL_EDITOR` (nonblank), then `VISUAL`, `EDITOR`, then the
/// platform default.
fn external_editor_command() -> String {
    external_editor_from(|name| std::env::var(name).ok())
}

pub(super) fn external_editor_from(get: impl Fn(&str) -> Option<String>) -> String {
    ["ICERS_EXTERNAL_EDITOR", "VISUAL", "EDITOR"]
        .into_iter()
        .filter_map(get)
        .find(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            if cfg!(windows) {
                "notepad".to_string()
            } else {
                "nano".to_string()
            }
        })
}

/// Edit `content` in `command` and return the saved text (ice
/// `editInExternalEditor`). An existing executable path is kept whole, including
/// spaces. Otherwise split on whitespace into a program and its arguments
/// (`code --wait`), then append the draft path. Remove one trailing newline,
/// which most editors add on save.
pub(super) fn edit_in_external_editor(
    command: &str,
    content: &str,
) -> Result<Option<String>, String> {
    // Editors commonly replace the file atomically. Keep no open file handle
    // and protect the draft with its own private directory (also on Windows).
    let directory = tempfile::Builder::new()
        .prefix("icers-editor-")
        .tempdir()
        .map_err(|error| format!("Could not create editor directory: {error}"))?;
    let path = directory.path().join("prompt.md");
    std::fs::write(&path, content.as_bytes())
        .map_err(|error| format!("Could not write editor file: {error}"))?;
    if !edit_file(command, &path)? {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)
        .map_err(|error| format!("Could not read editor file: {error}"))?;
    let text = match text.strip_suffix('\n') {
        Some(rest) => rest.strip_suffix('\r').unwrap_or(rest).to_string(),
        None => text,
    };
    Ok(Some(text))
}

pub(super) fn edit_mcp_config(
    tui: &Arc<TuiAltScreen>,
    path: &std::path::Path,
) -> Result<(), String> {
    crate::mcp::ensure_config(path)?;
    tui.suspend();
    let _ = crossterm::execute!(std::io::stdout(), crossterm::cursor::Show);
    let result = edit_file(&external_editor_command(), path);
    tui.resume();
    if !result? {
        return Err("MCP edit cancelled".into());
    }
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let _: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("Invalid MCP JSON: {e}"))?;
    Ok(())
}

fn edit_file(command: &str, path: &std::path::Path) -> Result<bool, String> {
    let (program, args) = if std::path::Path::new(command).is_file() {
        (command, Vec::new())
    } else {
        let mut parts = command.split_whitespace();
        let program = parts
            .next()
            .ok_or_else(|| "No external editor is configured.".to_string())?;
        (program, parts.collect::<Vec<_>>())
    };
    // ice spawns with `shell: true` on Windows so `.cmd`/`.bat` editors resolve.
    let mut child = if cfg!(windows) {
        let mut cmd = std::process::Command::new("cmd");
        cmd.arg("/C").arg(program);
        cmd
    } else {
        std::process::Command::new(program)
    };
    let status = child
        .args(args)
        .arg(&path)
        .status()
        .map_err(|error| format!("Could not launch external editor `{command}`: {error}"))?;
    Ok(status.success())
}

/// Editor caret at the end of `text`: the last line and its byte length.
pub(super) fn end_of_text_cursor(text: &str) -> (usize, usize) {
    let row = text.matches('\n').count();
    let col = text.rsplit('\n').next().map_or(0, str::len);
    (row, col)
}

/// Apply the authoritative run result when it wins the race with the async
/// event drain, then detach the live component from further partial updates.
pub(super) fn reconcile_streamed_assistant_completion(
    current_assistant: &Arc<std::sync::Mutex<Option<Arc<AssistantMessageComponent>>>>,
    last_assistant_text: &Mutex<String>,
    final_message: Option<&AssistantMessage>,
) {
    let final_blocks = final_message.map(assistant_blocks);
    let final_text = final_message.map(assistant_text);
    let component = current_assistant.lock().unwrap().take();

    if let Some(component) = component {
        if let Some(blocks) = final_blocks.as_deref() {
            component.update_blocks(blocks);
        }
        component.set_streaming(false);
    }

    if let Some(text) = final_text.filter(|text| !text.is_empty()) {
        *last_assistant_text.lock().unwrap() = text;
    }
}

/// Persist buffered shell results before prompt settlement releases the UI.
/// Event rendering must never perform this write against a later session.
pub(super) async fn flush_pending_bash_messages(
    harness: &AgentHarness,
    lane: &Arc<dyn AgentLane>,
    state: &Arc<TuiState>,
    session_id: &str,
) {
    let pending = std::mem::take(&mut *state.pending_bash_messages.lock().unwrap());
    if harness.session().storage().metadata().id != session_id {
        if !pending.is_empty() {
            add_note_message(
                &state.chat_container,
                "Discarded buffered bash results because the session changed.",
            );
        }
        return;
    }
    for item in pending {
        if !Arc::ptr_eq(&item.storage, &harness.session().storage()) {
            add_note_message(
                &state.chat_container,
                "Discarded a buffered shell result because its session was replaced.",
            );
            continue;
        }
        if let Err(error) = lane.append_message(item.message).await {
            add_error_message(
                &state.chat_container,
                &format!("Could not record bash result: {error}"),
            );
        }
    }
}

/// Record an out-of-band shell result at a turn boundary.
pub(super) async fn finish_user_bash(
    harness: &AgentHarness,
    state: &Arc<TuiState>,
    report: UserBashReport,
) {
    state.finish_user_bash();
    if !Arc::ptr_eq(&report.storage, &harness.session().storage()) {
        add_note_message(
            &state.chat_container,
            "Discarded a shell result because its session was replaced.",
        );
        return;
    }
    let message = ice_harness::messages::bash_execution_message(
        report.command,
        report.output,
        report.exit_code,
        report.cancelled,
        report.truncated,
        report.full_output_path,
        Some(report.exclude_from_context),
        now_ms(),
    );
    // Defer during a run to preserve complete tool-call/result sequences.
    // The prompt owner flushes these before releasing the UI to another run.
    if *state.status.lock().unwrap() == RunStatus::Idle {
        if let Err(error) = harness.lane("main").append_message(message).await {
            add_error_message(
                &state.chat_container,
                &format!("Could not record bash result: {error}"),
            );
        }
    } else {
        state
            .pending_bash_messages
            .lock()
            .unwrap()
            .push(BufferedBashMessage {
                storage: report.storage,
                message,
            });
    }
}

/// Drive the startup batch through the same prompt owner as interactive input.
pub(super) async fn run_initial_prompts(
    harness: &AgentHarness,
    prompts: Vec<String>,
    mut images: Vec<ice_ai::types::ImageContent>,
    tui: &Arc<TuiAltScreen>,
    state: &Arc<TuiState>,
    streaming: bool,
    running: &Arc<Mutex<bool>>,
) {
    let resources = harness.get_resources().await.unwrap_or_default();
    for prompt in prompts {
        if !*running.lock().unwrap() {
            break;
        }
        // Direct prompts do not receive a user MessageStart event.
        add_user_message(
            &state.chat_container,
            &display_user_prompt(&resources, &prompt),
        );
        add_image_previews(&state.chat_container, &images, state.images_visible());
        tui.request_render(false);
        if !run_prompt_streaming(
            harness,
            &prompt,
            false,
            tui,
            state,
            streaming,
            std::mem::take(&mut images),
        )
        .await
        {
            break;
        }
    }
}

/// Characters a `$name` skill mention may contain.
pub(super) fn is_skill_name_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | ':')
}

/// Expand explicit skill invocations into their full instruction blocks.
///
/// A prompt starting with `/skill:name args` expands like upstream
/// `AgentSession._expandSkillCommand` (block, blank line, args). Otherwise
/// each `$name` mention that starts a word and names a loaded skill is
/// replaced in place; trailing `.`/`:` is read as punctuation. Unknown names
/// (`$PATH`, `$5`, `/skill:missing`) are left as typed.
pub(super) fn expand_skill_references(
    resources: &ice_harness::types::AgentHarnessResources,
    prompt: &str,
) -> String {
    let skills = resources.skills.as_deref().unwrap_or_default();
    let find = |name: &str| skills.iter().find(|s| s.name == name);
    if let Some(rest) = prompt.strip_prefix("/skill:") {
        let (name, args) = rest.split_once(' ').unwrap_or((rest, ""));
        return match find(name) {
            Some(skill) => ice_harness::skills::format_skill_invocation(skill, Some(args.trim())),
            None => prompt.to_string(),
        };
    }
    let mut expanded = String::new();
    let mut copied = 0;
    for (start, _) in prompt.match_indices('$') {
        if start < copied
            || prompt[..start]
                .chars()
                .next_back()
                .is_some_and(|c| !c.is_whitespace())
        {
            continue;
        }
        let rest = &prompt[start + 1..];
        let len = rest.find(|c| !is_skill_name_char(c)).unwrap_or(rest.len());
        let name = rest[..len].trim_end_matches(['.', ':']);
        if let Some(skill) = find(name) {
            expanded.push_str(&prompt[copied..start]);
            expanded.push_str(&ice_harness::skills::format_skill_invocation(skill, None));
            copied = start + 1 + name.len();
        }
    }
    expanded.push_str(&prompt[copied..]);
    expanded
}

/// Replace each skill invocation block (and the blank line before any text
/// that follows it) with `render(name)` plus a space separator.
fn replace_skill_blocks(text: &str, render: impl Fn(&str) -> String) -> String {
    static BLOCK: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r#"<skill name="([^"]+)" location="[^"]*">\n(?s:.*?)\n</skill>(\n\n)?"#)
            .unwrap()
    });
    BLOCK
        .replace_all(text, |captures: &regex::Captures<'_>| {
            let separator = if captures.get(2).is_some() { " " } else { "" };
            format!("{}{separator}", render(&captures[1]))
        })
        .into_owned()
}

/// Inverse of [`expand_skill_references`] for draft restoration and plain
/// listings: each skill invocation block becomes its `$name` mention again.
pub(super) fn collapse_skill_invocations(text: &str) -> String {
    replace_skill_blocks(text, |name| format!("${name}"))
}

/// Transcript text for a user prompt: each invoked skill shows as its bare
/// name in the accent color instead of the skill body or the `$` marker.
pub(super) fn skill_display_text(text: &str) -> String {
    let accent = current_theme().colors.accent;
    replace_skill_blocks(text, |name| accent.fg(name))
}

/// Transcript text for a prompt the user is sending directly.
pub(super) fn display_user_prompt(
    resources: &ice_harness::types::AgentHarnessResources,
    prompt: &str,
) -> String {
    skill_display_text(&expand_skill_references(resources, prompt))
}

/// The message and images to queue for a prompt submitted mid-run.
pub(super) fn queued_user_message(
    state: &TuiState,
    resources: &ice_harness::types::AgentHarnessResources,
    prompt: &str,
) -> (AgentMessage, Vec<ice_ai::types::ImageContent>) {
    let images = state.take_prompt_images(prompt);
    let expanded = expand_skill_references(resources, prompt);
    (user_message_with_images(&expanded, images.clone()), images)
}

/// Drive one prompt, reconcile its final response, and settle buffered writes.
pub(super) async fn run_prompt_streaming(
    harness: &AgentHarness,
    prompt: &str,
    resume: bool,
    tui: &Arc<TuiAltScreen>,
    state: &Arc<TuiState>,
    streaming: bool,
    images: Vec<ice_ai::types::ImageContent>,
) -> bool {
    let lane = harness.lane("main");
    let session_id = harness.session().storage().metadata().id;
    state.set_status(RunStatus::Working);
    tui.request_render(false);

    let started = std::time::Instant::now();
    let outcome = if resume {
        // Continuing an interrupted run: no new user turn, just the provider call
        // the interrupted run never reached. upstream behaves the same way.
        match lane.resume_pending().await {
            Ok(Some(result)) => Ok(result),
            // Nothing to resume after all: leave the status alone and let the
            // normal prompt path take over.
            Ok(None) => {
                flush_pending_bash_messages(harness, &lane, state, &session_id).await;
                state.set_status(RunStatus::Idle);
                tui.request_render(false);
                return true;
            }
            Err(error) => Err(error),
        }
    } else {
        let resources = harness.get_resources().await.unwrap_or_default();
        let expanded = expand_skill_references(&resources, prompt);
        lane.prompt_text(&expanded, images).await
    };

    // Settle final tool/message delivery before completion reconciliation. This
    // also consumes the ordered RunSettled marker and its authoritative summary.
    // A stopped consumer must not turn into a successful-looking completion.
    let updates_settled = if streaming {
        match state.event_drain.flush().await {
            Ok(()) => true,
            Err(error) => {
                add_error_message(
                    &state.chat_container,
                    &format!("Could not finish response updates: {error}"),
                );
                false
            }
        }
    } else {
        false
    };
    flush_pending_bash_messages(harness, &lane, state, &session_id).await;

    if streaming {
        // Broadcast delivery is asynchronous: the harness result can resolve
        // before the drain task processes MessageEnd. Reconcile from the
        // authoritative outcome before detaching the component so the final
        // streamed tail cannot be left at an earlier partial snapshot.
        let final_message = match &outcome {
            Ok(result) => match &result.outcome {
                HarnessRunOutcome::Completed { final_message, .. }
                | HarnessRunOutcome::Aborted { final_message, .. } => Some(final_message),
                HarnessRunOutcome::Failed { final_message, .. } => final_message.as_ref(),
                HarnessRunOutcome::Suspended { .. } => None,
            },
            Err(_) => None,
        };
        reconcile_streamed_assistant_completion(
            &state.current_assistant,
            &state.last_assistant_text,
            final_message,
        );
    }

    state.set_status(RunStatus::Idle);
    // Guarantee the pending-messages display reconciles at the end of every
    // run/prompt — covers aborts and the non-streaming path, where no
    // `AgentEnd` event necessarily reaches the drain task.
    refresh_pending_messages(state, &lane).await;

    record_context_usage(&state.footer, harness).await;
    let completed = matches!(
        &outcome,
        Ok(result) if matches!(result.outcome, HarnessRunOutcome::Completed { .. })
    );

    match outcome {
        Ok(result) => match &result.outcome {
            HarnessRunOutcome::Failed {
                error,
                final_message,
                ..
            } => {
                // Only add an error line if the stream did NOT already render
                // an assistant message for it (drain task leaves
                // current_assistant Some only on an abrupt end).
                // A final assistant error is emitted by the event drain only
                // in streaming mode. In regular mode there is no drain task,
                // so suppressing this branch merely hides 405/auth/network
                // diagnostics from the user.
                let already_rendered = updates_settled && final_message.is_some();
                if !already_rendered {
                    let msg = final_message
                        .as_ref()
                        .and_then(|m| m.error_message.clone())
                        .unwrap_or_else(|| format!("{error:?}"));
                    add_error_message(&state.chat_container, &msg);
                }
            }
            HarnessRunOutcome::Suspended { .. } => {
                add_error_message(
                    &state.chat_container,
                    "Run suspended (deferred) — resume is not supported in v1.",
                );
            }
            HarnessRunOutcome::Aborted { final_message, .. } => {
                // Aborted runs render their own partial/final message via the
                // stream; only add a note on the blocking fallback path.
                if !streaming {
                    add_error_message(&state.chat_container, "Request aborted.");
                    let _ = final_message; // (rendered by the stream in streaming mode)
                }
            }
            HarnessRunOutcome::Completed { final_message, .. } => {
                if !streaming {
                    let text = assistant_text(final_message);
                    if !text.is_empty() {
                        add_assistant_message_blocking(
                            &state.chat_container,
                            &text,
                            state.markdown_transformer(),
                        );
                        *state.last_assistant_text.lock().unwrap() = text;
                    }
                    add_run_summary(&state.chat_container, started.elapsed());
                }
            }
        },
        Err(e) => {
            add_error_message(&state.chat_container, &e.to_string());
        }
    }

    tui.request_render(false);
    completed
}

pub(super) fn compact_outcome_message(outcome: CompactionOutcome) -> Result<(), String> {
    match outcome {
        CompactionOutcome::Completed { .. } => Ok(()),
        CompactionOutcome::Declined { .. } => Err("Nothing to compact.".into()),
        CompactionOutcome::Aborted { .. } => Err("Compaction cancelled.".into()),
        CompactionOutcome::Failed { error, .. } => {
            Err(format!("Compact failed: {}", error.message))
        }
    }
}

/// `/compact`: drive a compaction on the lane (mirrors TS `app.compact`).
/// Reports the outcome as a transcript note; the harness progress watcher
/// displays the compaction indicator while the summary is generated.
pub(super) async fn run_compact(
    harness: &AgentHarness,
    tui: &Arc<TuiAltScreen>,
    state: &Arc<TuiState>,
    instructions: Option<&str>,
) {
    state.set_status(RunStatus::Working);
    tui.request_render(false);
    match harness.lane("main").compact(instructions).await {
        Ok(result) => match compact_outcome_message(result.outcome) {
            Ok(()) => {
                add_note_message(&state.chat_container, "Conversation compacted.");
                // Refresh below also handles cancellation after a saved checkpoint.
            }
            Err(message) => add_error_message(&state.chat_container, &message),
        },
        Err(error) => {
            add_error_message(&state.chat_container, &format!("Compact failed: {error}"));
        }
    }
    record_context_usage(&state.footer, harness).await;
    state.set_status(RunStatus::Idle);
    tui.request_render(false);
}

/// `/copy`: copy the last assistant reply to the clipboard. Best-effort —
/// when no clipboard is available (or the `clipboard` feature is off), prints a
/// hint instead. Mirrors the TS `/copy` (copies `this.messages.at(-1)` text).
pub(super) fn copy_last_assistant(state: &Arc<TuiState>, chat: &Arc<Container>) {
    let text = state.last_assistant_text.lock().unwrap().clone();
    if text.is_empty() {
        add_note_message(chat, "Nothing to copy yet — no assistant reply captured.");
        return;
    }
    if copy_to_clipboard(&text) {
        add_note_message(chat, "Copied last reply to the clipboard.");
    } else {
        // Clipboard unavailable — print the text to the transcript so the user
        // can select/copy it manually (degrades gracefully in headless envs).
        let preview: String = text.chars().take(200).collect();
        add_note_message(
            chat,
            &format!(
                "Clipboard unavailable. Last reply: {preview}{}",
                if text.chars().count() > 200 {
                    "…"
                } else {
                    ""
                }
            ),
        );
    }
}

/// Finish a drag using the current viewport and the original logical anchor.
/// A changed wrapping width invalidates row coordinates, so cancel that drag.
pub(super) fn extract_transcript_selection(
    snapshot: &ice_tui::TranscriptSnapshot,
    start: (usize, usize, usize),
    end: (u16, u16),
) -> Option<String> {
    if snapshot.content_width != start.2 {
        return None;
    }
    let end = snapshot.content_position(end.0 as usize, end.1 as usize, true)?;
    extract_selected_text(&snapshot.lines, (start.0, start.1), end)
}

pub(super) fn transcript_rows_unchanged(original: &[String], current: &[String]) -> bool {
    current.len() >= original.len() && current[..original.len()] == *original
}

/// Begin a click candidate. Nothing is highlighted or copied until a Drag
/// event confirms that the pointer moved with the left button held.
/// Tool cell whose collapsed "… +N lines" row is under a screen cell.
pub(super) fn expand_link_at(
    snapshot: &ice_tui::ScreenSnapshot,
    (column, row): (usize, usize),
) -> Option<usize> {
    if snapshot.has_capturing_overlay {
        return None;
    }
    ice_tui::expand_link_id(snapshot.rows.get(row)?, column)
}

/// Tool cell a click at a screen cell toggles: the collapsed hint under the
/// pointer, or any row of an expanded cell.
pub(super) fn click_link_at(
    snapshot: &ice_tui::ScreenSnapshot,
    cell: (usize, usize),
) -> Option<usize> {
    expand_link_at(snapshot, cell).or_else(|| {
        if snapshot.has_capturing_overlay {
            return None;
        }
        ice_tui::collapse_link_id(snapshot.rows.get(cell.1)?)
    })
}

pub(super) fn begin_mouse_selection(
    tui: &TuiAltScreen,
    origin: (usize, usize),
) -> SelectionGesture {
    let generation = tui.screen_generation();
    let transcript_anchor = tui.transcript_snapshot().and_then(|snapshot| {
        snapshot
            .content_position(origin.0, origin.1, false)
            .map(|logical| (logical, snapshot.content_width, snapshot.lines.clone()))
    });
    SelectionGesture::PendingClick {
        origin,
        generation,
        transcript_anchor,
    }
}

pub(super) fn mouse_selection_cancels_for_wheel(gesture: &SelectionGesture) -> bool {
    matches!(
        gesture,
        SelectionGesture::PendingClick { .. }
            | SelectionGesture::Dragging {
                source: SelectionSource::Screen { .. },
            }
    )
}

/// Extend the current mouse gesture. Transcript drags retain their logical
/// anchor across scrolling; a drag leaving that viewport switches to the
/// captured composed screen only while the anchor remains visible.
pub(super) fn update_mouse_selection(
    gesture: &mut SelectionGesture,
    tui: &TuiAltScreen,
    endpoint: (usize, usize),
) -> bool {
    let current = gesture.clone();
    let (source, generation, first_drag) = match current {
        SelectionGesture::Idle => return false,
        SelectionGesture::PendingClick {
            origin,
            generation,
            transcript_anchor,
        } => {
            if generation != tui.screen_generation() {
                *gesture = SelectionGesture::Idle;
                return false;
            }
            let Some(snapshot) = tui
                .screen_snapshot()
                .filter(|snapshot| snapshot.generation == generation)
            else {
                *gesture = SelectionGesture::Idle;
                return false;
            };
            if let Some((logical_anchor, content_width, original_lines)) = transcript_anchor {
                let Some(transcript) = tui.transcript_snapshot() else {
                    *gesture = SelectionGesture::Idle;
                    return false;
                };
                if transcript.content_width != content_width
                    || !transcript_rows_unchanged(&original_lines, &transcript.lines)
                {
                    *gesture = SelectionGesture::Idle;
                    return false;
                }
                let endpoint_inside = transcript
                    .content_position(endpoint.0, endpoint.1, false)
                    .is_some();
                if endpoint_inside {
                    (
                        SelectionSource::Transcript {
                            logical_anchor,
                            content_width,
                            original_lines,
                            generation,
                        },
                        generation,
                        true,
                    )
                } else {
                    let Some(screen_anchor) =
                        transcript.screen_position(logical_anchor.0, logical_anchor.1)
                    else {
                        *gesture = SelectionGesture::Idle;
                        return false;
                    };
                    (
                        SelectionSource::Screen {
                            snapshot,
                            screen_anchor,
                        },
                        generation,
                        true,
                    )
                }
            } else {
                (
                    SelectionSource::Screen {
                        snapshot,
                        screen_anchor: origin,
                    },
                    generation,
                    true,
                )
            }
        }
        SelectionGesture::Dragging { source, .. } => {
            let generation = match &source {
                SelectionSource::Transcript { generation, .. } => *generation,
                SelectionSource::Screen { snapshot, .. } => snapshot.generation,
            };
            (source, generation, false)
        }
    };

    if generation != tui.screen_generation() {
        *gesture = SelectionGesture::Idle;
        tui.set_selection(None);
        return false;
    }

    let source = match source {
        SelectionSource::Transcript {
            logical_anchor,
            content_width,
            original_lines,
            generation,
        } => {
            let Some(snapshot) = tui.transcript_snapshot() else {
                *gesture = SelectionGesture::Idle;
                tui.set_selection(None);
                return false;
            };
            if snapshot.content_width != content_width
                || !transcript_rows_unchanged(&original_lines, &snapshot.lines)
            {
                *gesture = SelectionGesture::Idle;
                tui.set_selection(None);
                return false;
            }
            if let Some(logical_end) = snapshot.content_position(endpoint.0, endpoint.1, false) {
                tui.clear_screen_selection();
                tui.set_selection(Some((logical_anchor, logical_end)));
                SelectionSource::Transcript {
                    logical_anchor,
                    content_width,
                    original_lines,
                    generation,
                }
            } else {
                let Some(screen_anchor) =
                    snapshot.screen_position(logical_anchor.0, logical_anchor.1)
                else {
                    *gesture = SelectionGesture::Idle;
                    tui.set_selection(None);
                    return false;
                };
                let Some(screen_snapshot) = tui
                    .screen_snapshot()
                    .filter(|snapshot| snapshot.generation == generation)
                else {
                    *gesture = SelectionGesture::Idle;
                    tui.set_selection(None);
                    return false;
                };
                tui.set_selection(None);
                if !tui.set_screen_selection(screen_snapshot.clone(), (screen_anchor, endpoint)) {
                    *gesture = SelectionGesture::Idle;
                    return false;
                }
                SelectionSource::Screen {
                    snapshot: screen_snapshot,
                    screen_anchor,
                }
            }
        }
        SelectionSource::Screen {
            snapshot,
            screen_anchor,
        } => {
            if !first_drag && !tui.has_screen_selection(snapshot.generation) {
                *gesture = SelectionGesture::Idle;
                return false;
            }
            if !tui.set_screen_selection(snapshot.clone(), (screen_anchor, endpoint)) {
                *gesture = SelectionGesture::Idle;
                return false;
            }
            SelectionSource::Screen {
                snapshot,
                screen_anchor,
            }
        }
    };
    *gesture = SelectionGesture::Dragging { source };
    true
}

/// Resolve a completed drag to plain text from its retained source frame.
pub(super) fn finish_mouse_selection(
    gesture: &SelectionGesture,
    tui: &TuiAltScreen,
    endpoint: (usize, usize),
) -> Option<String> {
    let SelectionGesture::Dragging { source } = gesture else {
        return None;
    };
    match source {
        SelectionSource::Transcript {
            logical_anchor,
            content_width,
            original_lines,
            generation,
        } => {
            if *generation != tui.screen_generation() {
                return None;
            }
            let endpoint = (
                endpoint.0.min(u16::MAX as usize) as u16,
                endpoint.1.min(u16::MAX as usize) as u16,
            );
            tui.transcript_snapshot().and_then(|snapshot| {
                (snapshot.content_width == *content_width
                    && transcript_rows_unchanged(original_lines, &snapshot.lines))
                .then(|| {
                    extract_transcript_selection(
                        &snapshot,
                        (logical_anchor.0, logical_anchor.1, *content_width),
                        endpoint,
                    )
                })?
            })
        }
        SelectionSource::Screen {
            snapshot,
            screen_anchor,
        } => {
            if !tui.has_screen_selection(snapshot.generation) {
                return None;
            }
            extract_selected_text(&snapshot.rows, *screen_anchor, endpoint)
        }
    }
}

pub(super) fn mouse_selection_text_for_copy(
    auto_copy: bool,
    gesture: &SelectionGesture,
    tui: &TuiAltScreen,
    endpoint: (usize, usize),
) -> Option<String> {
    auto_copy
        .then(|| finish_mouse_selection(gesture, tui, endpoint))
        .flatten()
}

pub(super) fn cancel_mouse_selection(state: &TuiState, tui: &TuiAltScreen) {
    *state.selection_gesture.lock().unwrap() = SelectionGesture::Idle;
    tui.set_selection(None);
    tui.clear_screen_selection();
}

pub(super) fn place_editor_caret_from_screen_click(
    tui: &TuiAltScreen,
    editor: &Arc<Editor>,
    column: usize,
    row: usize,
) -> bool {
    let Some(rect) = tui.component_rect(editor.as_ref()) else {
        return false;
    };
    if column < rect.x
        || column >= rect.x.saturating_add(rect.width)
        || row < rect.y
        || row >= rect.y.saturating_add(rect.height)
    {
        return false;
    }
    let visible_rows = rect.height.saturating_sub(2).max(1);
    let rendered_row = row
        .saturating_sub(rect.y)
        .saturating_sub(1)
        .min(visible_rows - 1);
    editor.set_cursor_at_visual_position(rendered_row, column.saturating_sub(rect.x), rect.width);
    editor.set_focused(true);
    tui.set_focus(Some(editor.clone()));
    tui.request_render_reusing_scroll_content();
    true
}

/// Plain text of a mouse selection over cached logical transcript rows.
///
/// `start` and `end` are `(column, row)` content cells, inclusive, in either
/// order. The first row is taken from the start column, the last row up to
/// the end column, and rows between in full. Escape sequences are dropped and
/// trailing padding is trimmed. This used to re-render the chat at a fixed
/// width of 80 and index it by screen row, which ignored the real width, the
/// scroll position and the columns, and kept escape codes.
pub(super) fn extract_selected_text(
    rows: &[String],
    start: (usize, usize),
    end: (usize, usize),
) -> Option<String> {
    let key = |(col, row): (usize, usize)| (row, col);
    let ((first_row, first_col), (last_row, last_col)) = if key(start) <= key(end) {
        (key(start), key(end))
    } else {
        (key(end), key(start))
    };
    if first_row >= rows.len() {
        return None;
    }
    // A drag that ends below the frame takes the last row in full.
    let (last_row, last_col) = if last_row >= rows.len() {
        (rows.len() - 1, usize::MAX)
    } else {
        (last_row, last_col)
    };
    let selected: Vec<String> = (first_row..=last_row)
        .map(|index| {
            let from = if index == first_row { first_col } else { 0 };
            let line = &rows[index];
            let text = if index == last_row {
                let width = last_col.saturating_add(1).saturating_sub(from);
                ice_tui::slice_with_width(line, from, width, false).0
            } else {
                ice_tui::slice_with_width(line, from, usize::MAX, false).0
            };
            ice_tui::strip_ansi(&text).trim_end().to_string()
        })
        .collect();
    Some(selected.join("\n"))
}

#[cfg(all(feature = "clipboard", not(target_os = "android")))]
thread_local! {
    // X11 clipboard ownership must outlive set_text so other clients can read
    // the selection. Keep one owner per calling UI/runtime thread; Clipboard
    // does not need to be Send, and successful copies reuse the same owner.
    static SYSTEM_CLIPBOARD: std::cell::RefCell<Option<arboard::Clipboard>> =
        std::cell::RefCell::new(None);
}

/// Lazily retain a successful backend and retire a failed one so a later copy
/// can retry. The generic seam tests ownership without accessing an OS clipboard.
#[cfg(any(test, all(feature = "clipboard", not(target_os = "android"))))]
pub(super) fn write_retained_resource<T, E>(
    slot: &std::cell::RefCell<Option<T>>,
    initialize: impl FnOnce() -> Result<T, E>,
    write: impl FnOnce(&mut T) -> Result<(), E>,
) -> bool {
    let mut retained = slot.borrow_mut();
    if retained.is_none() {
        match initialize() {
            Ok(resource) => *retained = Some(resource),
            Err(_) => return false,
        }
    }
    let succeeded = write(retained.as_mut().expect("initialized resource")).is_ok();
    if !succeeded {
        retained.take();
    }
    succeeded
}

/// Best-effort clipboard write. Enabled only with the `clipboard` feature
/// (`arboard`) on non-Android platforms; otherwise returns `false` so the
/// caller degrades to a hint.
#[cfg(all(feature = "clipboard", not(target_os = "android")))]
fn copy_to_system_clipboard(text: &str) -> bool {
    SYSTEM_CLIPBOARD.with(|slot| {
        write_retained_resource(slot, arboard::Clipboard::new, |clipboard| {
            clipboard.set_text(text)
        })
    })
}

#[cfg(any(not(feature = "clipboard"), target_os = "android"))]
fn copy_to_system_clipboard(_text: &str) -> bool {
    false
}

/// Linux clipboard helpers tried in ice's order: `wl-copy` on Wayland, then
/// `xclip`/`xsel`. arboard has no native-Wayland backend in this build, so
/// these keep copy working there without another dependency.
#[cfg(target_os = "linux")]
pub(super) fn clipboard_commands(wayland: bool) -> Vec<(&'static str, &'static [&'static str])> {
    let mut commands: Vec<(&'static str, &'static [&'static str])> = Vec::new();
    if wayland {
        commands.push(("wl-copy", &[]));
    }
    commands.push(("xclip", &["-selection", "clipboard"]));
    commands.push(("xsel", &["--clipboard", "--input"]));
    commands
}

#[cfg(target_os = "linux")]
fn copy_via_clipboard_command(text: &str) -> bool {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty());
    clipboard_commands(wayland)
        .into_iter()
        .any(|(program, args)| {
            let Ok(mut child) = Command::new(program)
                .args(args)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            else {
                return false;
            };
            let wrote = child
                .stdin
                .take()
                .is_some_and(|mut stdin| stdin.write_all(text.as_bytes()).is_ok());
            wrote && child.wait().is_ok_and(|status| status.success())
        })
}

#[cfg(not(target_os = "linux"))]
fn copy_via_clipboard_command(_text: &str) -> bool {
    false
}

/// Write `text` to the local clipboard. On Linux the platform tools
/// (`wl-copy`, `xclip`, `xsel`) go first, as in ice: arboard is X11-only here,
/// and on Wayland compositors such as Hyprland its write reports success
/// without reaching Wayland clients, which used to skip the tools entirely.
pub(super) fn copy_locally(
    text: &str,
    prefer_commands: bool,
    native: impl FnOnce(&str) -> bool,
    commands: impl FnOnce(&str) -> bool,
) -> bool {
    if prefer_commands {
        commands(text) || native(text)
    } else {
        native(text) || commands(text)
    }
}

/// Copy `text` like ice `copyToClipboard`: the system clipboard first, then,
/// over SSH or mosh (where that clipboard is the remote machine's) or when it
/// failed, also an OSC 52 request so the user's own terminal stores it.
pub(super) fn copy_to_clipboard(text: &str) -> bool {
    let copied = copy_locally(
        text,
        cfg!(target_os = "linux"),
        copy_to_system_clipboard,
        copy_via_clipboard_command,
    );
    if copied && !is_remote_session(|key| std::env::var_os(key)) {
        return true;
    }
    match osc52_sequence(text) {
        Some(sequence) => {
            use std::io::Write;
            let mut out = std::io::stdout().lock();
            out.write_all(sequence.as_bytes()).is_ok() && out.flush().is_ok() || copied
        }
        None => copied,
    }
}

/// ice `isRemoteSession`.
pub(super) fn is_remote_session(env: impl Fn(&str) -> Option<std::ffi::OsString>) -> bool {
    ["SSH_CONNECTION", "SSH_CLIENT", "MOSH_CONNECTION"]
        .iter()
        .any(|key| env(key).is_some_and(|v| !v.is_empty()))
}

/// The OSC 52 clipboard write for `text`, or `None` when the payload is too
/// large for terminals to take reliably (ice `MAX_OSC52_ENCODED_LENGTH`).
pub(super) fn osc52_sequence(text: &str) -> Option<String> {
    const MAX_ENCODED: usize = 100_000;
    // Reject by the known encoded length before allocating it.
    if text.len().div_ceil(3).saturating_mul(4) > MAX_ENCODED {
        return None;
    }
    let encoded = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
    (encoded.len() <= MAX_ENCODED).then(|| format!("\x1b]52;c;{encoded}\x07"))
}

/// Best-effort clipboard text read for Ctrl+V paste. The optional `clipboard`
/// feature keeps headless builds free of platform clipboard dependencies;
/// on Android the OS owns clipboard access so this degrades to `None`.
#[cfg(all(feature = "clipboard", not(target_os = "android")))]
pub(super) fn read_clipboard_text() -> Option<String> {
    let mut clipboard = arboard::Clipboard::new().ok()?;
    clipboard.get_text().ok()
}

#[cfg(any(not(feature = "clipboard"), target_os = "android"))]
pub(super) fn read_clipboard_text() -> Option<String> {
    None
}

/// Read a clipboard bitmap and normalize it to PNG for the provider-neutral
/// `ImageContent` contract. The optional clipboard feature keeps headless
/// builds free of platform clipboard dependencies.
#[cfg(all(feature = "clipboard", not(target_os = "android")))]
pub(super) fn read_clipboard_image() -> Result<Option<ice_ai::types::ImageContent>, String> {
    #[cfg(target_os = "linux")]
    if let Some(image) = read_linux_clipboard_image() {
        return Ok(Some(image));
    }
    let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    let image = match clipboard.get_image() {
        Ok(image) => image,
        Err(_) => return Ok(None),
    };
    let width =
        u32::try_from(image.width).map_err(|_| "clipboard image is too wide".to_string())?;
    let height =
        u32::try_from(image.height).map_err(|_| "clipboard image is too tall".to_string())?;
    if width == 0 || height == 0 || width > 16_384 || height > 16_384 {
        return Err("clipboard image dimensions are outside the supported range".into());
    }
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        writer
            .write_image_data(&image.bytes)
            .map_err(|e| e.to_string())?;
    }
    Ok(Some(ice_ai::types::ImageContent {
        kind: ice_ai::types::ImageContentType,
        data: base64::engine::general_purpose::STANDARD.encode(bytes),
        mime_type: "image/png".into(),
    }))
}

/// Prefer native Wayland clipboard access, with X11 as a fallback.
#[cfg(all(feature = "clipboard", target_os = "linux"))]
fn read_linux_clipboard_image() -> Option<ice_ai::types::ImageContent> {
    let mut commands: Vec<(&str, &[&str])> = Vec::new();
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        commands.push(("wl-paste", &["--no-newline", "--type", "image/png"]));
    }
    commands.push((
        "xclip",
        &["-selection", "clipboard", "-o", "-t", "image/png"],
    ));
    for (command, args) in commands {
        if let Some(stdout) = clipboard_command_output(command, args) {
            if stdout.starts_with(b"\x89PNG\r\n\x1a\n") {
                return Some(ice_ai::types::ImageContent {
                    kind: ice_ai::types::ImageContentType,
                    data: base64::engine::general_purpose::STANDARD.encode(stdout),
                    mime_type: "image/png".into(),
                });
            }
        }
    }
    None
}

/// Stdout of a successful clipboard tool run, or `None` on failure or after
/// 2 s. Runs on the key worker, so an unresponsive clipboard owner must not
/// freeze input. Stdout is drained on a thread: an image larger than the pipe
/// buffer would otherwise block the child before it exits.
#[cfg(all(feature = "clipboard", target_os = "linux"))]
fn clipboard_command_output(command: &str, args: &[&str]) -> Option<Vec<u8>> {
    let mut child = std::process::Command::new(command)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = tx.send(std::io::Read::read_to_end(&mut stdout, &mut bytes).map(|_| bytes));
    });
    match rx.recv_timeout(std::time::Duration::from_secs(2)) {
        Ok(Ok(bytes)) if child.wait().is_ok_and(|status| status.success()) => Some(bytes),
        Ok(_) => None,
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            None
        }
    }
}

/// Finder copied files arrive as aliases, rather than clipboard bitmaps.
#[cfg(all(target_os = "macos", feature = "clipboard"))]
pub(super) fn read_clipboard_file_paths() -> Option<Vec<String>> {
    let script = "try\nset items to the clipboard as alias list\nset paths to {}\nrepeat with itemPath in items\nset end of paths to POSIX path of itemPath\nend repeat\nset AppleScript's text item delimiters to linefeed\nreturn paths as text\non error\nreturn \"\"\nend try";
    let output = std::process::Command::new("osascript")
        .args(["-e", script])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let paths: Vec<String> = text
        .lines()
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect();
    if paths.is_empty() {
        None
    } else {
        Some(paths)
    }
}

/// Explorer's copied files use CF_HDROP rather than a bitmap or plain text.
#[cfg(all(windows, feature = "clipboard"))]
pub(super) fn read_clipboard_file_paths() -> Option<Vec<String>> {
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, GetClipboardData, OpenClipboard,
    };
    use windows_sys::Win32::UI::Shell::DragQueryFileW;
    const CF_HDROP: u32 = 15;

    struct ClipboardRead;
    impl Drop for ClipboardRead {
        fn drop(&mut self) {
            // SAFETY: this guard exists only after OpenClipboard succeeds.
            unsafe {
                CloseClipboard();
            }
        }
    }

    // SAFETY: null HWND is supported; the clipboard is opened only for reading.
    if unsafe { OpenClipboard(std::ptr::null_mut()) } == 0 {
        return None;
    }
    let _guard = ClipboardRead;
    // SAFETY: the clipboard remains open while its borrowed HDROP is queried.
    let drop = unsafe { GetClipboardData(CF_HDROP) };
    if drop.is_null() {
        return None;
    }
    let count = unsafe { DragQueryFileW(drop, u32::MAX, std::ptr::null_mut(), 0) };
    let mut paths = Vec::new();
    for index in 0..count {
        // SAFETY: query the length first, then provide that many UTF-16 units
        // plus the terminator. No DragFinish: the handle belongs to the clipboard.
        let length = unsafe { DragQueryFileW(drop, index, std::ptr::null_mut(), 0) };
        if length == 0 {
            continue;
        }
        let mut buffer = vec![0u16; length as usize + 1];
        let copied =
            unsafe { DragQueryFileW(drop, index, buffer.as_mut_ptr(), buffer.len() as u32) };
        if copied == 0 {
            continue;
        }
        let path = String::from_utf16(&buffer[..copied as usize]).ok()?;
        paths.push(path);
    }
    if paths.is_empty() {
        None
    } else {
        Some(paths)
    }
}

#[cfg(not(all(any(target_os = "macos", windows), feature = "clipboard")))]
pub(super) fn read_clipboard_file_paths() -> Option<Vec<String>> {
    None
}

pub(super) fn attach_pasted_images(state: &Arc<TuiState>, text: &str) -> bool {
    let Some(images) = images_from_pasted_paths(text) else {
        return false;
    };
    attach_images_to_draft(state, images);
    true
}

pub(super) fn attach_images_to_draft(
    state: &Arc<TuiState>,
    images: Vec<ice_ai::types::ImageContent>,
) {
    for image in images {
        let number = state.queue_image(image);
        state.editor.insert(&format!("{} ", image_marker(number)));
    }
}

/// Draft marker for the `number`th (1-based) pending image.
pub(super) fn image_marker(number: usize) -> String {
    format!("[Image #{number}]")
}

/// Consume a paste as attachments only when every path is an actual image.
/// Otherwise leave the pasted text intact. Handles Finder escaped spaces,
/// quoted multiple paths and clipboard file paths separated by newlines.
pub(super) fn images_from_pasted_paths(text: &str) -> Option<Vec<ice_ai::types::ImageContent>> {
    let candidate = text.trim().trim_matches(['\"', '\'']);
    let single = std::path::Path::new(candidate);
    if single.is_file() {
        return crate::app::image_content_from_path(single)
            .ok()
            .flatten()
            .map(|image| vec![image]);
    }
    let mut paths = Vec::new();
    if text.contains('\n') {
        paths.extend(
            text.lines()
                .filter(|line| !line.trim().is_empty())
                .map(|line| line.trim().trim_matches(['\"', '\'']).to_owned()),
        );
    } else {
        let mut path = String::new();
        let mut quote = None;
        let mut chars = text.chars().peekable();
        while let Some(ch) = chars.next() {
            match ch {
                '\'' | '"' if quote == Some(ch) => quote = None,
                '\'' | '"' if quote.is_none() => quote = Some(ch),
                '\\' if quote != Some('\'')
                    && chars.peek().is_some_and(|ch| {
                        ch.is_whitespace() || matches!(ch, '\'' | '"' | '\\')
                    }) =>
                {
                    path.push(chars.next().unwrap());
                }
                ch if ch.is_whitespace() && quote.is_none() => {
                    if !path.is_empty() {
                        paths.push(std::mem::take(&mut path));
                    }
                }
                _ => path.push(ch),
            }
        }
        if quote.is_some() {
            return None;
        }
        if !path.is_empty() {
            paths.push(path);
        }
    }
    if paths.is_empty() {
        return None;
    }
    paths
        .iter()
        .map(|path| {
            crate::app::image_content_from_path(std::path::Path::new(path))
                .ok()
                .flatten()
        })
        .collect()
}

pub(super) fn user_message_with_images(
    text: &str,
    images: Vec<ice_ai::types::ImageContent>,
) -> AgentMessage {
    let content = if images.is_empty() {
        ice_ai::types::UserContent::Text(text.into())
    } else {
        let mut blocks = Vec::with_capacity(images.len() + 1);
        if !text.is_empty() {
            blocks.push(ice_ai::types::Content::text(text));
        }
        blocks.extend(images.into_iter().map(ice_ai::types::Content::Image));
        ice_ai::types::UserContent::Blocks(blocks)
    };
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default();
    AgentMessage::User(UserMessage::new(content, timestamp))
}

pub(super) fn add_image_previews(
    chat: &Arc<Container>,
    images: &[ice_ai::types::ImageContent],
    show: bool,
) {
    for image in images {
        add_image_preview(chat, image, show);
    }
}

pub(super) fn add_user_images(
    chat: &Arc<Container>,
    user: &ice_ai::types::UserMessage,
    show: bool,
) {
    if let ice_ai::types::UserContent::Blocks(blocks) = &user.content {
        for block in blocks {
            if let ice_ai::types::Content::Image(image) = block {
                add_image_preview(chat, image, show);
            }
        }
    }
}

pub(super) fn add_tool_images(
    chat: &Arc<Container>,
    content: &[ice_agent::types::TextContentOrImage],
    show: bool,
) {
    for block in content {
        if let ice_agent::types::TextContentOrImage::Image(image) = block {
            add_image_preview(
                chat,
                &ice_ai::types::ImageContent {
                    kind: ice_ai::types::ImageContentType,
                    data: image.data.clone(),
                    mime_type: image.mime_type.clone(),
                },
                show,
            );
        }
    }
}

pub(super) fn add_image_preview(
    chat: &Arc<Container>,
    image: &ice_ai::types::ImageContent,
    show: bool,
) {
    if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&image.data) {
        // Kitty's f=100 payload must be PNG, regardless of the model MIME type.
        let bytes = if image.mime_type == "image/png" {
            bytes
        } else {
            match ice_tools::image_processing::process_image(&bytes, Default::default()) {
                Ok(png) => png,
                Err(_) => {
                    add_note_message(chat, "[Image preview unavailable]");
                    return;
                }
            }
        };
        let mut options = ImageOptions::default();
        options.width = Some(60);
        options.alt_text = Some("[Attached image]".into());
        let preview = Arc::new(Image::from_data(bytes, options));
        preview.set_inline_visible(show);
        chat.add_child(preview);
        chat.add_child(Arc::new(Spacer::new(1)));
    }
}

#[cfg(any(not(feature = "clipboard"), target_os = "android"))]
pub(super) fn read_clipboard_image() -> Result<Option<ice_ai::types::ImageContent>, String> {
    Ok(None)
}

/// Blocking fallback (no `event_rx`): render the final assistant text as a
/// single `AssistantMessageComponent`, mirroring the pre-streaming behavior.
/// `transformer` is the live assistant-markdown transformer (B5e); `None` is
/// the identity path. The blocking path only fires when `event_rx` is absent,
/// so it shares the same transformer the streaming path installs on its
/// components.
pub(super) fn add_assistant_message_blocking(
    container: &Arc<Container>,
    text: &str,
    transformer: Option<MarkdownTransformer>,
) {
    if text.is_empty() {
        return;
    }
    let msg = Arc::new(AssistantMessageComponent::new(
        AssistantMessageOptions::default(),
    ));
    if let Some(t) = &transformer {
        msg.set_markdown_transformer(Some(t.clone()));
    }
    msg.update_text(text);
    container.add_child(msg);
    container.add_child(Arc::new(Spacer::new(1)));
}
