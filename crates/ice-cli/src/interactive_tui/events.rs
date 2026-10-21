//! Agent event rendering and queued-message recovery.

use super::*;

/// A session barrier is acknowledged only after prior ordered broadcasts,
/// including the harness's RunSettled completion markers, have been handled.
pub(super) enum AgentEventDrainRequest {
    Flush(tokio::sync::oneshot::Sender<()>),
}

/// RunEnd is derived from HarnessRunOutcome, unlike AgentEnd's message-only
/// payload. Observe the bus synchronously (including extension-started runs),
/// then render on the same consumer that owns final message/tool delivery.
pub(super) fn watch_run_summaries(
    harness: &AgentHarness,
    state: &Arc<TuiState>,
) -> ice_harness::watcher::HarnessWatcher {
    let state = Arc::downgrade(state);
    let started = Mutex::new(None);
    harness.watch(move |event| {
        use ice_harness::events::HarnessEvent;
        match event {
            HarnessEvent::RunStart(event) if event.lane == "main" => {
                *started.lock().unwrap() = Some((event.run_id.clone(), std::time::Instant::now()));
            }
            HarnessEvent::RunEnd(event) if event.lane == "main" => {
                let at = started
                    .lock()
                    .unwrap()
                    .take()
                    .filter(|(id, _)| id == &event.run_id)
                    .map(|(_, at)| at);
                if let (Some(at), Some(state)) = (at, state.upgrade()) {
                    state.event_drain.record_run_summary(
                        event.run_id.clone(),
                        event.outcome,
                        at.elapsed(),
                    );
                }
            }
            _ => {}
        }
    })
}

/// A completed lane can still have display events buffered in its broadcast
/// receiver. Session changes wait until the UI has consumed those events.
#[derive(Default)]
pub(super) struct AgentEventDrain {
    pub(super) session_changing: std::sync::atomic::AtomicBool,
    sender: Mutex<Option<mpsc::UnboundedSender<AgentEventDrainRequest>>>,
    // A lost marker must not retain metadata forever. Dropping the oldest
    // metadata can suppress a lagged summary, never invent success for a run.
    summaries: Mutex<
        std::collections::VecDeque<(
            String,
            ice_harness::events::RunEndOutcome,
            std::time::Duration,
        )>,
    >,
}

impl AgentEventDrain {
    const MAX_SUMMARIES: usize = 256;

    fn record_run_summary(
        &self,
        run_id: String,
        outcome: ice_harness::events::RunEndOutcome,
        elapsed: std::time::Duration,
    ) {
        let mut summaries = self.summaries.lock().unwrap();
        if summaries.len() == Self::MAX_SUMMARIES {
            summaries.pop_front();
        }
        summaries.push_back((run_id, outcome, elapsed));
    }

    fn take_run_summary(
        &self,
        run_id: &str,
    ) -> Option<(ice_harness::events::RunEndOutcome, std::time::Duration)> {
        let mut summaries = self.summaries.lock().unwrap();
        let index = summaries.iter().position(|(id, _, _)| id == run_id)?;
        summaries
            .remove(index)
            .map(|(_, outcome, elapsed)| (outcome, elapsed))
    }

    pub(super) fn install(&self) -> mpsc::UnboundedReceiver<AgentEventDrainRequest> {
        let (sender, receiver) = mpsc::unbounded_channel();
        *self.sender.lock().unwrap() = Some(sender);
        receiver
    }

    pub(super) async fn flush(&self) -> Result<(), &'static str> {
        let sender = self.sender.lock().unwrap().clone();
        let Some(sender) = sender else {
            return Ok(());
        };
        let (acknowledge, completed) = tokio::sync::oneshot::channel();
        sender
            .send(AgentEventDrainRequest::Flush(acknowledge))
            .map_err(|_| "response updates are unavailable")?;
        tokio::time::timeout(std::time::Duration::from_secs(5), completed)
            .await
            .map_err(|_| "the previous response updates did not finish; try again")?
            .map_err(|_| "response updates are unavailable")
    }
}

// ===========================================================================
// AgentEvent drain task — the streaming core
// ===========================================================================

/// Drain `AgentEvent`s from the broadcast receiver and apply the TS
/// `handleEvent` event→UI mapping. Runs on a `tokio::spawn`'d task for the
/// lifetime of the TUI.
/// Re-read the lane's queue and update the pending-messages dock slot.
/// Cheap (two short mutex locks) and a no-op when the snapshot is unchanged,
/// so it is safe to call on every relevant event.
pub(super) async fn refresh_pending_messages(state: &Arc<TuiState>, lane: &Arc<dyn AgentLane>) {
    if let Ok(queued) = lane.queued_messages().await {
        state.set_pending_queue(queued);
    }
}

/// Whether the transcript holds a panel that repaints its elapsed readout from
/// the clock on every frame: a running bash command, or a tool call still in
/// [`ToolStatus::Running`]. Such a panel needs a real transcript rebuild each
/// render tick — a frame that reuses the cached scroll content would leave its
/// timer frozen at the second of the last full rebuild.
pub(super) fn transcript_has_live_panel(
    bash_components_present: bool,
    tool_components: &HashMap<String, Arc<ToolExecutionComponent>>,
) -> bool {
    bash_components_present
        || tool_components
            .values()
            .any(|component| component.status() == ToolStatus::Running)
}

/// Prepend restored queued messages to the draft the user may already have
/// typed (upstream's `restoreQueuedMessagesToEditor` places them first).
pub(super) fn merge_queued_into_draft(queued_text: &str, current: &str) -> String {
    if current.trim().is_empty() {
        queued_text.to_string()
    } else {
        format!("{queued_text}\n\n{current}")
    }
}

/// Place the caret at the END of `text` after `set_text`. `set_text` resets the
/// caret to `(0, 0)`, and the last line's **byte** length is the right column:
/// a char count was wrong for multibyte drafts, and row 0 was wrong for a
/// multi-line one (upstream's `setTextInternal(text, "end")`).
pub(super) fn set_editor_text_caret_at_end(editor: &Arc<Editor>, text: &str) {
    editor.set_text(text);
    let last_row = text.split('\n').count().saturating_sub(1);
    let last_len = text.split('\n').next_back().map(str::len).unwrap_or(0);
    editor.set_cursor(last_row, last_len);
}

/// Drain every queued steering/follow-up message out of the lane and restore it
/// to the editor (upstream's `restoreQueuedMessagesToEditor`). Returns how many
/// were restored; the queue is consumed either way, so the dock drops its rows.
///
/// Runs on a spawned task rather than through the [`TuiMessage`] mailbox: the
/// async main loop owns `run_prompt_streaming(..).await` for the whole run, and
/// a queue only exists *while* a run is in flight — so a `TuiMessage::Dequeue`
/// was never serviced until the run ended, after the lane had already consumed
/// the queue. That made Alt+Q a no-op while it was most needed.
pub(super) async fn restore_queued_messages_to_editor(
    lane: Arc<dyn AgentLane>,
    editor: Arc<Editor>,
    state: Arc<TuiState>,
    tui: Arc<TuiAltScreen>,
) -> usize {
    match lane.clear_queue().await {
        Ok(queued) => {
            state.restore_pending_images(queued.images.clone());
            let all: Vec<String> = queued
                .steering
                .iter()
                .chain(queued.follow_up.iter())
                .cloned()
                .collect();
            if !all.is_empty() {
                let queued_text = all.join("\n\n");
                // Read the EXPANDED draft: `set_text` below clears the folded
                // paste store, so any `[paste #id …]` marker still in the
                // editor must be materialized first or it would be left behind
                // as literal text.
                let current = editor.get_expanded_text();
                let combined = merge_queued_into_draft(&queued_text, &current);
                set_editor_text_caret_at_end(&editor, &combined);
            }
            // The lane drained the queue; refresh so the dock drops the rows.
            refresh_pending_messages(&state, &lane).await;
            tui.request_render(false);
            all.len()
        }
        Err(error) => {
            add_error_message(
                &state.chat_container,
                &format!("Could not restore queued messages: {error}"),
            );
            tui.request_render(false);
            0
        }
    }
}

/// `app.message.dequeue`: restore the queued messages and report the count, like
/// upstream's `handleDequeue` (`Restored N queued message(s)` / `No queued
/// messages to restore`). The status is a dim transcript note (icers's analog of
/// native `showStatus`).
pub(super) async fn handle_dequeue(
    lane: Arc<dyn AgentLane>,
    editor: Arc<Editor>,
    state: Arc<TuiState>,
    tui: Arc<TuiAltScreen>,
) {
    let restored =
        restore_queued_messages_to_editor(lane, editor, state.clone(), tui.clone()).await;
    if restored == 0 {
        add_note_message(&state.chat_container, "No queued messages to restore");
    } else {
        let plural = if restored > 1 { "s" } else { "" };
        add_note_message(
            &state.chat_container,
            &format!("Restored {restored} queued message{plural} to editor"),
        );
    }
    tui.request_render(false);
}

/// Cancel the active run the native-pi way: pull every queued steering /
/// follow-up message back into the editor FIRST, then abort. upstream's
/// `onEscape` does exactly this during streaming
/// (`restoreQueuedMessagesToEditor({abort: true})`), so a cancel never silently
/// swallows messages the user staged while the run was in flight — they land in
/// the editor to be re-edited or resent.
/// Ownership is captured under the dispatch status lock and passed explicitly.
/// Never reclassify a completed navigation as an ordinary queued-message run.
pub(super) fn abort_interrupted_run(
    lane: Arc<dyn AgentLane>,
    editor: Arc<Editor>,
    state: Arc<TuiState>,
    tui: Arc<TuiAltScreen>,
    navigation: bool,
) -> impl std::future::Future<Output = ()> + Send {
    async move {
        if navigation {
            // The source operation may finish before this task gets CPU time.
            // Do not restore its queue or abort a subsequently admitted prompt.
            if !state
                .tree_navigation_active
                .load(std::sync::atomic::Ordering::Acquire)
                || *state.status.lock().unwrap() != RunStatus::Aborting
            {
                return;
            }
        } else {
            let _ =
                restore_queued_messages_to_editor(lane.clone(), editor, state.clone(), tui.clone())
                    .await;
        }
        loop {
            match lane.abort().await {
                // Navigation can still be waiting to be admitted as a run, so
                // there is nothing to abort yet. Keep trying until it is, or
                // until it ends; a lost Esc would let it commit anyway.
                Err(ice_harness::result::HarnessError::NoActiveRun { .. })
                    if navigation
                        && state
                            .tree_navigation_active
                            .load(std::sync::atomic::Ordering::Acquire)
                        && *state.status.lock().unwrap() == RunStatus::Aborting =>
                {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
                _ => break,
            }
        }
        tui.request_render(false);
    }
}

/// The actual key-loop interrupt seam, also used by navigation cancellation
/// regressions so effective key routing and queue ownership stay coupled.
pub(super) fn dispatch_run_interrupt(
    key: &KeyEvent,
    bindings: &ice_tui::Keybindings,
    lane: Arc<dyn AgentLane>,
    editor: Arc<Editor>,
    state: Arc<TuiState>,
    tui: Arc<TuiAltScreen>,
) -> bool {
    if !keybinding_matches(bindings, key, ice_tui::keybindings::keys::INTERRUPT) {
        return false;
    }
    // TreeNavigationActivity::drop uses this same lock to clear ownership and
    // publish Idle. Claim Working, capture ownership, and publish Aborting as
    // one transaction; completion cannot split those observations.
    let mut status = state.status.lock().unwrap();
    if *status != RunStatus::Working {
        return false;
    }
    let navigation = state
        .tree_navigation_active
        .load(std::sync::atomic::Ordering::Acquire);
    *status = RunStatus::Aborting;
    drop(status);
    state.apply_status(RunStatus::Aborting);
    tokio::spawn(abort_interrupted_run(lane, editor, state, tui, navigation));
    true
}

pub(super) async fn drain_agent_events(
    mut rx: broadcast::Receiver<AgentEvent>,
    tui: Arc<TuiAltScreen>,
    state: Arc<TuiState>,
    chat: Arc<Container>,
    lane: Arc<dyn AgentLane>,
    mut requests: mpsc::UnboundedReceiver<AgentEventDrainRequest>,
) {
    loop {
        tokio::select! {
            request = requests.recv() => {
                let Some(request) = request else { break };
                // The caller has settled the lane. Consume everything already
                // emitted, including AgentEnd's deferred bash writes, before
                // allowing the durable session backing to change.
                loop {
                    match rx.try_recv() {
                        Ok(event) => handle_agent_event(event, &tui, &state, &chat, &lane).await,
                        Err(broadcast::error::TryRecvError::Lagged(_)) => continue,
                        Err(broadcast::error::TryRecvError::Empty
                            | broadcast::error::TryRecvError::Closed) => break,
                    }
                }
                let AgentEventDrainRequest::Flush(acknowledge) = request;
                let _ = acknowledge.send(());
            }
            event = rx.recv() => match event {
                Ok(event) => handle_agent_event(event, &tui, &state, &chat, &lane).await,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    }
}

/// Apply a single `AgentEvent` to the UI. Mirrors the TS `handleEvent` switch
/// (`interactive-mode.ts:3068-3396`).
pub(super) async fn handle_agent_event(
    event: AgentEvent,
    tui: &Arc<TuiAltScreen>,
    state: &Arc<TuiState>,
    chat: &Arc<Container>,
    lane: &Arc<dyn AgentLane>,
) {
    match event {
        AgentEvent::RunSettled { run_id } => {
            if let Some((outcome, elapsed)) = state.event_drain.take_run_summary(&run_id) {
                if outcome == ice_harness::events::RunEndOutcome::Completed {
                    add_run_summary(chat, elapsed);
                    tui.request_render(false);
                }
            }
        }
        AgentEvent::AgentStart => {
            state.set_status(RunStatus::Working);
            tui.request_render(false);
        }

        AgentEvent::AgentEnd { .. } => {
            // Finalize any still-streaming assistant message.
            state.transcript_view().finalize_assistant();
            state.set_status(RunStatus::Idle);
            // Flush any `!command` results that completed mid-run: the run's
            // tool sequence is closed, so appending now keeps transcript order
            // intact (upstream's `flushPendingBashMessages`).
            let pending: Vec<AgentMessage> =
                std::mem::take(&mut *state.pending_bash_messages.lock().unwrap());
            for message in pending {
                if let Err(error) = lane.append_message(message).await {
                    add_error_message(chat, &format!("Could not record bash result: {error}"));
                }
            }
            // The run drained the queue; refresh so consumed entries drop out
            // of the pending-messages display.
            refresh_pending_messages(state, lane).await;
            tui.request_render(false);
        }

        AgentEvent::RetryScheduled {
            attempt,
            max_retries,
            delay_ms,
            ..
        } => {
            state.show_retry(attempt, max_retries, delay_ms);
            tui.request_render(false);
        }

        AgentEvent::TurnStart => {
            // A new turn: reset the streaming-assistant guard so the next
            // MessageStart creates a fresh component.
            state.transcript_view().finalize_assistant();
        }

        AgentEvent::TurnEnd {
            message,
            tool_results,
        } => {
            // Finalize the assistant message for this turn.
            let view = state.transcript_view();
            if let AgentMessage::Assistant(a) = &message {
                view.finalize_assistant_blocks(&assistant_blocks(a));
            } else {
                view.finalize_assistant();
            }
            // Tool results whose components were never ended by a
            // ToolExecutionEnd are handled by the normal path; there is nothing
            // extra to finalize here.
            let _ = tool_results;
            tui.request_render(false);
        }

        AgentEvent::MessageStart { message } => match message {
            AgentMessage::Assistant(a) => {
                state.new_message_count.begin_assistant_message();
                // The shared transcript view renders this through the same
                // component path as the remote client. It installs the live
                // markdown transformer + hide-thinking preference itself.
                state.transcript_view().apply(
                    &UiEvent::AssistantStart {
                        blocks: assistant_blocks(&a),
                    },
                    tui.width(),
                );
                tui.request_render(false);
            }
            AgentMessage::Custom(custom) => {
                state.new_message_count.custom_message();
                let payload = serde_json::json!({
                    "customType": custom.role,
                    "content": custom.content,
                    "details": custom.data,
                    "expanded": false,
                    "outputPad": 1,
                });
                if let Some(component) = extension_message_component(
                    &state.extension_session,
                    &custom.role,
                    &payload,
                    state.markdown_transformer(),
                ) {
                    chat.add_child(component);
                    chat.add_child(Arc::new(Spacer::new(1)));
                    tui.request_render(false);
                } else {
                    add_note_message(chat, &custom_message_fallback(&custom));
                    tui.request_render(false);
                }
            }
            // Queued (steering / follow-up) user messages are rendered here,
            // not at queue time, so a message that is still waiting never looks
            // delivered. The loop drains those itself and emits
            // `message_start` for them (`agent_loop.rs` pending-message drain).
            //
            // A DIRECTLY-sent prompt does NOT reach this arm: the harness
            // persists it up front and runs the loop with an empty prompts vec,
            // so no `message_start` is emitted for it — the submit handler
            // renders that bubble instead.
            AgentMessage::User(user) => {
                state.new_message_count.other_message();
                state.transcript_view().add_user(&user_message_text(&user));
                add_user_images(&state.chat_container, &user, state.images_visible());
                // A consumed entry must drop out of the pending display.
                refresh_pending_messages(state, lane).await;
                tui.request_render(false);
            }
            // Tool results break adjacent assistant continuations, but their
            // starts are echoed via tool-execution components.
            AgentMessage::ToolResult(_) => state.new_message_count.other_message(),
        },

        AgentEvent::MessageUpdate {
            message,
            assistant_message_event,
        } => {
            let _ = assistant_message_event; // the snapshot is authoritative
            let a: &ice_ai::types::AssistantMessage = &message;
            let text = assistant_text(a);
            state
                .new_message_count
                .assistant_message(!text.trim().is_empty());
            // The shared view creates/updates tool + bash panels from the
            // snapshot's finalized tool calls and streams the block list, so a
            // thinking block renders live exactly as the remote client does.
            state.transcript_view().apply(
                &UiEvent::AssistantUpdate {
                    blocks: assistant_blocks(a),
                    tool_calls: assistant_tool_calls(a),
                },
                tui.width(),
            );
            // A freshly-created bash panel hides the global `Working…` loader.
            state.sync_working_loader_with_bash();
            *state.last_assistant_text.lock().unwrap() = text;
            tui.request_render(false);
        }

        AgentEvent::MessageEnd { message } => {
            if let AgentMessage::Assistant(a) = &message {
                state
                    .new_message_count
                    .assistant_message(!assistant_text(a).trim().is_empty());
                let text = assistant_text(a);
                state
                    .transcript_view()
                    .finalize_assistant_blocks(&assistant_blocks(a));
                // Cache the finalized text for `/copy`.
                if !text.is_empty() {
                    *state.last_assistant_text.lock().unwrap() = text;
                }
                // Cache-miss notice (`maybeShowCacheMissNotice`): the previous
                // turn's input established a cacheable prefix; a prompt that
                // re-reads less than it should means the prefix was re-billed.
                // Uses the shared cache-stats logic (noise floor, idle gap,
                // model change) and prices the miss when known.
                let usage = &a.usage;
                // Footer usage totals + cache hit rate (pi footer.ts).
                state.footer.add_usage(
                    usage.input,
                    usage.output,
                    usage.cache_read,
                    usage.cache_write,
                    usage.cost.total,
                );
                let denom = usage.input + usage.cache_read + usage.cache_write;
                if denom > 0 && (usage.cache_read + usage.cache_write) > 0 {
                    state
                        .footer
                        .set_cache_hit_rate(Some(usage.cache_read as f64 / denom as f64 * 100.0));
                }
                // Context-window badge (`?/512k` → `63.2%/512k`): the provider
                // reports the prompt token count for this request, which is the
                // context size the footer displays.
                record_context_usage(&state.footer, a);
                let miss = state
                    .cache_tracker
                    .lock()
                    .unwrap()
                    .observe(a, &ice_harness::cache_stats::NoPrices);
                // upstream gates these behind `showCacheMissNotices`
                // (default `false`); the tracker still runs so the footer's
                // cache-hit rate stays accurate either way.
                let cache_miss_notices = *state.cache_miss_notices.lock().unwrap();
                if let Some(miss) = miss.filter(|_| cache_miss_notices) {
                    let cost = if miss.missed_cost > 0.0 {
                        format!(" (~${:.4})", miss.missed_cost)
                    } else {
                        String::new()
                    };
                    add_note_message(
                        &state.chat_container,
                        &format!(
                            "Cache miss: {} tokens re-billed{}",
                            format_tokens(miss.missed_tokens),
                            cost
                        ),
                    );
                }
                if let Some(text) = extension_usage_text(Some(&state.extension_session), usage) {
                    add_note_message(chat, &text);
                }
                // Error assistant messages carry the provider diagnostic in
                // `error_message`, not in text content. The assistant
                // component is empty for these messages, so surface the
                // diagnostic as a visible error row in the transcript.
                if let Some(error) = assistant_error_text(a) {
                    add_error_message(chat, &error);
                }
            }
            tui.request_render(false);
        }

        AgentEvent::ToolExecutionStart {
            tool_call_id,
            tool_name,
            args,
        } => {
            state.transcript_view().apply(
                &UiEvent::ToolStart {
                    id: tool_call_id,
                    name: tool_name,
                    args,
                },
                tui.width(),
            );
            state.sync_working_loader_with_bash();
            tui.request_render(false);
        }

        AgentEvent::ToolExecutionUpdate {
            tool_call_id,
            tool_name,
            args,
            partial_result,
        } => {
            state.transcript_view().apply(
                &UiEvent::ToolUpdate {
                    id: tool_call_id,
                    name: tool_name,
                    args,
                    result: crate::remote::protocol::tool_result_json(&partial_result),
                },
                tui.width(),
            );
            state.sync_working_loader_with_bash();
            tui.request_render(false);
        }

        AgentEvent::ToolExecutionEnd {
            tool_call_id,
            tool_name,
            result,
            is_error,
        } => {
            state.transcript_view().apply(
                &UiEvent::ToolEnd {
                    id: tool_call_id,
                    name: tool_name,
                    result: crate::remote::protocol::tool_result_json(&result),
                    is_error,
                },
                tui.width(),
            );
            add_tool_images(
                &state.chat_container,
                &result.content,
                state.images_visible(),
            );
            state.sync_working_loader_with_bash();
            // Bash tool completion is user-visible: immediate render ensures the
            // result is displayed even when the scheduler thread is busy or the
            // throttle window has not elapsed.
            tui.render_now(false);
        }
    }
}

/// Update the footer's context-window badge from one assistant response.
///
/// `Usage::context_tokens()` is the prompt size the provider reported for that
/// request — the value pi's footer renders as `{percent}%/{window}`. Aborted
/// and errored turns are skipped (their usage is not a real context
/// measurement, mirroring pi's `lastAssistantUsageInfo`) so the last valid
/// count stays on screen instead of flickering to `?`.
pub(super) fn record_context_usage(
    footer: &ice_tui::FooterComponent,
    message: &ice_ai::AssistantMessage,
) {
    if message.stop_reason == ice_ai::StopReason::Aborted
        || message.stop_reason == ice_ai::StopReason::Error
    {
        return;
    }
    let tokens = message.usage.context_tokens();
    if tokens > 0 {
        footer.set_context_tokens(Some(tokens));
    }
}

/// Return the diagnostic carried by a failed or aborted provider request.
/// Providers may omit `error_message`; keep a stable fallback so a terminal
/// request failure can never render as an empty transcript turn.
pub(super) fn assistant_error_text(message: &ice_ai::AssistantMessage) -> Option<String> {
    let fallback = match message.stop_reason {
        ice_ai::StopReason::Error => "Provider request failed.",
        ice_ai::StopReason::Aborted => "Request aborted.",
        _ => return None,
    };
    Some(
        message
            .error_message
            .as_deref()
            .filter(|text| !text.trim().is_empty())
            .unwrap_or(fallback)
            .to_string(),
    )
}

/// Whether a tool result opts into markdown rendering of its body.
///
/// A tool signals this by setting `details.markdown = true` on its result
/// (the `ice-todo` extension does this for its progress bar + table payload).
/// The host then hands the body to the markdown renderer instead of printing
/// it as plain text — see `ToolExecutionComponent::set_result_markdown`.
/// Absent or non-`true` details keep the existing plain-text rendering, so
/// every other tool is unaffected.
pub(super) fn tool_result_requests_markdown(details: Option<&serde_json::Value>) -> bool {
    details
        .and_then(|details| details.get("markdown"))
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

/// Verbs for the run-summary line, one picked per run (ice
/// `RUN_SUMMARY_VERBS`).
const RUN_SUMMARY_VERBS: [&str; 12] = [
    "Worked", "Sautéed", "Simmered", "Brewed", "Baked", "Churned", "Cooked", "Crunched", "Whisked",
    "Forged", "Pondered", "Tinkered",
];

/// A dim elapsed line, called only after an authoritative completed outcome.
pub(super) fn add_run_summary(chat: &Arc<Container>, elapsed: std::time::Duration) {
    let line = run_summary_line(elapsed);
    chat.add_child(Arc::new(Spacer::new(1)));
    chat.add_child(Arc::new(Text::new(
        current_theme().colors.dim.fg(&line),
        0,
        0,
    )));
}

pub(super) fn run_summary_line(elapsed: std::time::Duration) -> String {
    let verb = RUN_SUMMARY_VERBS[(elapsed.subsec_nanos() as usize) % RUN_SUMMARY_VERBS.len()];
    format!("✻ {verb} for {}", format_run_duration(elapsed))
}

/// `45s`, `3m 17s`, `1h 2m` (ice `formatShellRuntime`).
fn format_run_duration(elapsed: std::time::Duration) -> String {
    let seconds = elapsed.as_secs();
    if seconds < 60 {
        return format!("{seconds}s");
    }
    let minutes = seconds / 60;
    if minutes < 60 {
        return format!("{minutes}m {}s", seconds % 60);
    }
    format!("{}h {}m", minutes / 60, minutes % 60)
}
