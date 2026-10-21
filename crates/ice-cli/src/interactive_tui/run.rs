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
    if !status.success() {
        // A nonzero editor exit is cancellation: keep the original draft.
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

/// Drive a single prompt through the lane. When `streaming` is true, the
/// `AgentEvent` drain task renders the response live and the completed outcome
/// reconciles its final snapshot. When false (no `event_rx`), this falls back
/// to the blocking await-final-text path.

/// Drive a single prompt through the lane. When `streaming` is true, the
/// `AgentEvent` drain task renders the response live and the completed outcome
/// reconciles its final snapshot. When false (no `event_rx`), this falls back
/// to the blocking await-final-text path.
pub(super) async fn run_prompt_streaming(
    lane: &Arc<dyn AgentLane>,
    prompt: &str,
    resume: bool,
    tui: &Arc<TuiAltScreen>,
    state: &Arc<TuiState>,
    streaming: bool,
    images: Vec<ice_ai::types::ImageContent>,
) {
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
                state.set_status(RunStatus::Idle);
                tui.request_render(false);
                return;
            }
            Err(error) => Err(error),
        }
    } else {
        lane.prompt_text(prompt, images).await
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
    refresh_pending_messages(state, lane).await;

    // Authoritative context-badge fallback. The streaming drain normally
    // updates it from `MessageEnd`, but a `Lagged` broadcast or the blocking
    // path can skip that event; reconcile from the run outcome so the badge
    // always reflects the newest real response.
    if let Ok(result) = &outcome {
        let final_message: Option<&AssistantMessage> = match &result.outcome {
            HarnessRunOutcome::Completed { final_message, .. }
            | HarnessRunOutcome::Aborted { final_message, .. } => Some(final_message),
            HarnessRunOutcome::Failed { final_message, .. } => final_message.as_ref(),
            HarnessRunOutcome::Suspended { .. } => None,
        };
        if let Some(a) = final_message {
            record_context_usage(&state.footer, a);
        }
    }

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
}

/// `/compact`: drive a compaction on the lane (mirrors TS `app.compact`).
/// Reports the outcome as a transcript note; the harness progress watcher
/// displays the compaction indicator while the summary is generated.
pub(super) async fn run_compact(
    lane: &Arc<dyn AgentLane>,
    tui: &Arc<TuiAltScreen>,
    state: &Arc<TuiState>,
) {
    state.set_status(RunStatus::Working);
    tui.request_render(false);
    match lane.compact(None).await {
        Ok(_) => {
            add_note_message(&state.chat_container, "Conversation compacted.");
            // The last assistant usage describes the pre-compaction context;
            // reset the badge to `?` until the next response reports the new
            // (much smaller) prompt size — mirrors pi's `percent: null`.
            state.footer.set_context_tokens(None);
        }
        Err(e) => {
            add_error_message(&state.chat_container, &format!("Compact failed: {e}"));
        }
    }
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

/// Copy `text` like ice `copyToClipboard`: the system clipboard first, then,
/// over SSH or mosh (where that clipboard is the remote machine's) or when it
/// failed, also an OSC 52 request so the user's own terminal stores it.
pub(super) fn copy_to_clipboard(text: &str) -> bool {
    let copied = copy_to_system_clipboard(text);
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
    let count = images.len();
    for image in images {
        state.queue_image(image);
    }
    add_note_message(
        &state.chat_container,
        &format!("Attached {count} image(s) to the next prompt."),
    );
    true
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
