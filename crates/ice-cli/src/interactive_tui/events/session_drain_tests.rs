use super::*;

#[test]
fn buffered_old_run_events_do_not_repopulate_a_new_session() {
    with_isolated_config_dir(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                use ice_ai::providers::faux::{
                    faux_assistant_message, FauxProvider, FauxScript, FauxStep,
                };
                let (emitter, events) = ice_agent::events::BroadcastEmitter::new(256);
                let script = FauxScript::new();
                script.set_responses(vec![FauxStep::factory(|_| {
                    faux_assistant_message("old response", ice_ai::types::StopReason::Stop)
                })]);
                let provider = FauxProvider::new(script);
                let harness = AgentHarness::create(ice_harness::types::AgentHarnessOptions {
                    model: provider.default_model().clone(),
                    models: vec![provider],
                    agent_emitter: Some(Arc::new(emitter)),
                    ..Default::default()
                })
                .await
                .unwrap();
                harness.prompt_text("old prompt", vec![]).await.unwrap();
                assert!(harness.snapshot().await.unwrap().active.is_none());
                assert!(!events.is_empty());

                let state = test_tui_state();
                let lane = harness.lane("main");
                let cwd = tempfile::tempdir().unwrap();
                let tool_context =
                    ice_extensions::ToolCallContext::new(cwd.path().to_string_lossy().to_string());
                let requests = state.event_drain.install();
                let tui = Arc::new(TuiAltScreen::new(
                    Box::new(ProcessTerminal::new()),
                    true,
                    None,
                ));
                let change = start_new_session(
                    &harness,
                    &lane,
                    cwd.path(),
                    &state.chat_container,
                    &state,
                    &tool_context,
                );
                tokio::pin!(change);
                // No consumer has run yet. Lane-idle alone must NOT permit
                // replacement while real old-provider events remain buffered.
                assert!(
                    tokio::time::timeout(std::time::Duration::from_millis(20), &mut change)
                        .await
                        .is_err()
                );
                assert!(!events.is_empty());
                assert!(
                    state.begin_user_bash().is_none(),
                    "bash admission must stay closed during the drain await"
                );
                let drain = tokio::spawn(drain_agent_events(
                    events,
                    tui,
                    state.clone(),
                    state.chat_container.clone(),
                    harness.clone(),
                    requests,
                ));
                assert!(change.await);
                assert!(state.begin_user_bash().is_some());
                state.finish_user_bash();
                state.event_drain.flush().await.unwrap();
                assert!(
                    state.last_assistant_text.lock().unwrap().is_empty(),
                    "the prior run was painted into the newly selected session"
                );
                let text = strip_ansi(&state.chat_container.render(120).join("\n"));
                assert!(!text.contains("old response"), "{text}");
                drain.abort();
                harness.close().await.unwrap();
            })
    })
}

fn isolated_async(body: impl std::future::Future<Output = ()>) {
    with_isolated_config_dir(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap()
            .block_on(body)
    });
}

async fn scripted_harness(
    script: ice_ai::providers::faux::FauxScript,
) -> (AgentHarness, broadcast::Receiver<AgentEvent>) {
    let (emitter, events) = ice_agent::events::BroadcastEmitter::new(256);
    let provider = ice_ai::providers::faux::FauxProvider::new(script);
    let harness = AgentHarness::create(ice_harness::types::AgentHarnessOptions {
        model: provider.default_model().clone(),
        models: vec![provider],
        agent_emitter: Some(Arc::new(emitter)),
        ..Default::default()
    })
    .await
    .unwrap();
    (harness, events)
}

pub(super) fn test_terminal() -> Arc<TuiAltScreen> {
    Arc::new(TuiAltScreen::new(
        Box::new(ProcessTerminal::new()),
        true,
        None,
    ))
}

fn display(state: &Arc<TuiState>) -> String {
    strip_ansi(&state.chat_container.render(120).join("\n"))
}

#[test]
fn context_footer_updates_during_message_and_tool_events() {
    isolated_async(async {
        let (emitter, events) = ice_agent::events::BroadcastEmitter::new(16);
        let mut model = ice_ai::Model::new("m", "m", ice_ai::Api::Faux, "faux", "");
        model.context_window = 100_000;
        let harness = AgentHarness::create(ice_harness::types::AgentHarnessOptions {
            model,
            ..Default::default()
        })
        .await
        .unwrap();
        let state = test_tui_state();
        state.set_status(RunStatus::Working);
        let requests = state.event_drain.install();
        let drain = tokio::spawn(drain_agent_events(
            events,
            test_terminal(),
            state.clone(),
            state.chat_container.clone(),
            AgentHarness::clone(&harness),
            requests,
        ));
        let mut assistant = AssistantMessage::empty(ice_ai::Api::Faux, "faux", "m", 0);
        assistant.usage.input = 25_000;
        let assistant = AgentMessage::Assistant(Box::new(assistant));
        let tool = AgentMessage::ToolResult(Box::new(ice_ai::ToolResultMessage {
            role: ice_ai::types::ToolResultRole,
            tool_call_id: "call".into(),
            tool_name: "read".into(),
            content: vec![ice_ai::Content::text("x".repeat(20_000))],
            details: None,
            usage: None,
            added_tool_names: Vec::new(),
            is_error: false,
            timestamp: 0,
        }));
        for (message, expected) in [(assistant, "~25.0%/100k"), (tool, "~30.0%/100k")] {
            harness
                .session()
                .append_message(message.clone())
                .await
                .unwrap();
            ice_agent::events::AgentEmitter::emit(&emitter, AgentEvent::MessageEnd { message })
                .await;
            tokio::time::timeout(std::time::Duration::from_secs(2), async {
                loop {
                    let footer = strip_ansi(&state.footer.render(120).join("\n"));
                    if footer.contains(expected) {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                }
            })
            .await
            .expect("context footer stayed stale during an active run");
            assert!(matches!(*state.status.lock().unwrap(), RunStatus::Working));
        }
        state.event_drain.flush().await.unwrap();
        drain.abort();
        harness.close().await.unwrap();
    });
}

#[test]
fn session_drain_stopped_consumer_preserves_session_draft_and_queue() {
    isolated_async(async {
        let (harness, _events) =
            scripted_harness(ice_ai::providers::faux::FauxScript::new().with_text("old reply"))
                .await;
        harness.prompt_text("old prompt", vec![]).await.unwrap();
        harness
            .steer(user_message_with_images("keep queued", vec![]))
            .await
            .unwrap();
        let expected_queue = harness.queued_messages().await.unwrap();
        let old_id = harness.session().storage().metadata().id.clone();
        let state = test_tui_state();
        state.editor.set_text("keep draft");
        add_note_message(&state.chat_container, "keep transcript");
        state.set_pending_queue(expected_queue.clone());
        // A configured but stopped consumer must fail closed, unlike the
        // deliberately absent drain on the blocking/non-streaming path.
        drop(state.event_drain.install());
        let cwd = tempfile::tempdir().unwrap();
        let context =
            ice_extensions::ToolCallContext::new(cwd.path().to_string_lossy().into_owned());
        let lane = harness.lane("main");
        assert!(
            !start_new_session(
                &harness,
                &lane,
                cwd.path(),
                &state.chat_container,
                &state,
                &context
            )
            .await
        );
        assert_eq!(harness.session().storage().metadata().id, old_id);
        assert_eq!(harness.queued_messages().await.unwrap(), expected_queue);
        assert_eq!(*state.pending_snapshot.lock().unwrap(), expected_queue);
        assert_eq!(state.editor.get_text(), "keep draft");
        let text = display(&state);
        assert!(
            text.contains("keep transcript") && text.contains("response updates are unavailable"),
            "{text}"
        );
        harness.close().await.unwrap();
    });
}

#[test]
fn session_drain_dropped_acknowledgement_fails_without_replacement() {
    isolated_async(async {
        let (harness, _events) = scripted_harness(ice_ai::providers::faux::FauxScript::new()).await;
        let state = test_tui_state();
        let mut requests = state.event_drain.install();
        let original = harness.session().storage().metadata().id.clone();
        let cwd = tempfile::tempdir().unwrap();
        let context =
            ice_extensions::ToolCallContext::new(cwd.path().to_string_lossy().into_owned());
        let lane = harness.lane("main");
        let fail = async move {
            let AgentEventDrainRequest::Flush(ack) = requests.recv().await.unwrap();
            drop(ack);
        };
        let (changed, ()) = tokio::join!(
            start_new_session(
                &harness,
                &lane,
                cwd.path(),
                &state.chat_container,
                &state,
                &context
            ),
            fail
        );
        assert!(!changed);
        assert_eq!(harness.session().storage().metadata().id, original);
        harness.close().await.unwrap();
    });
}

#[test]
fn session_drain_shared_switch_import_fork_paths_consume_real_old_events() {
    isolated_async(async {
        for action in ["switch", "import", "fork"] {
            let (harness, events) =
                scripted_harness(ice_ai::providers::faux::FauxScript::new().with_text("old reply"))
                    .await;
            let cwd = tempfile::tempdir().unwrap();
            let source_dir = tempfile::tempdir().unwrap();
            let source = crate::session::create_jsonl_session(
                source_dir.path(),
                &cwd.path().to_string_lossy(),
            )
            .await
            .unwrap();
            harness.set_session(source).await.unwrap();
            harness.prompt_text("old prompt", vec![]).await.unwrap();
            assert!(!events.is_empty());
            let state = test_tui_state();
            let requests = state.event_drain.install();
            let target_dir = tempfile::tempdir().unwrap();
            let target = crate::session::create_jsonl_session(
                target_dir.path(),
                &cwd.path().to_string_lossy(),
            )
            .await
            .unwrap();
            let target_path = target.storage().persistence_path().unwrap();
            // The target is reopened by the switch; release its writer lock first.
            drop(target);
            let context =
                ice_extensions::ToolCallContext::new(cwd.path().to_string_lossy().into_owned());
            let lane = harness.lane("main");
            let old_id = harness.session().storage().metadata().id.clone();
            let change = async {
                match action {
                    // The picker and /resume activate through this exact-file
                    // helper; import also funnels through it after copying.
                    "switch" => assert!(
                        switch_to_session(
                            &harness,
                            &lane,
                            &target_path,
                            cwd.path(),
                            &state.chat_container,
                            &state,
                            &context
                        )
                        .await
                    ),
                    "import" => {
                        import_session(
                            &harness,
                            &lane,
                            &target_path,
                            cwd.path(),
                            &state.chat_container,
                            &state,
                            &context,
                        )
                        .await
                    }
                    "fork" => assert!(
                        fork_session(
                            &harness,
                            cwd.path(),
                            &state.chat_container,
                            &state,
                            &context,
                            &Default::default()
                        )
                        .await
                    ),
                    _ => unreachable!(),
                }
            };
            tokio::pin!(change);
            assert!(
                tokio::time::timeout(std::time::Duration::from_millis(20), &mut change)
                    .await
                    .is_err()
            );
            assert_eq!(harness.session().storage().metadata().id, old_id);
            let drain = tokio::spawn(drain_agent_events(
                events,
                test_terminal(),
                state.clone(),
                state.chat_container.clone(),
                harness.clone(),
                requests,
            ));
            change.await;
            state.event_drain.flush().await.unwrap();
            assert_ne!(
                harness.session().storage().metadata().id,
                old_id,
                "{action}: {}",
                display(&state)
            );
            if action != "fork" {
                assert!(state.last_assistant_text.lock().unwrap().is_empty());
                assert!(!display(&state).contains("old reply"));
            } else {
                // Fork legitimately replays retained history exactly once.
                assert_eq!(display(&state).matches("old reply").count(), 1);
            }
            drain.abort();
            harness.close().await.unwrap();
        }
    });
}

#[test]
fn session_drain_active_user_bash_blocks_all_replacements_and_keeps_queue() {
    isolated_async(async {
        let (harness, _events) = scripted_harness(ice_ai::providers::faux::FauxScript::new()).await;
        harness
            .steer(user_message_with_images("keep queued", vec![]))
            .await
            .unwrap();
        let queue = harness.queued_messages().await.unwrap();
        let old_id = harness.session().storage().metadata().id.clone();
        let state = test_tui_state();
        let cancel = state.begin_user_bash().unwrap();
        let cwd = tempfile::tempdir().unwrap();
        let context =
            ice_extensions::ToolCallContext::new(cwd.path().to_string_lossy().into_owned());
        let lane = harness.lane("main");
        assert!(
            !start_new_session(
                &harness,
                &lane,
                cwd.path(),
                &state.chat_container,
                &state,
                &context
            )
            .await
        );
        assert!(
            !switch_to_session(
                &harness,
                &lane,
                "not-opened",
                cwd.path(),
                &state.chat_container,
                &state,
                &context
            )
            .await
        );
        assert!(
            !fork_session(
                &harness,
                cwd.path(),
                &state.chat_container,
                &state,
                &context,
                &Default::default()
            )
            .await
        );
        let source_dir = tempfile::tempdir().unwrap();
        let source =
            crate::session::create_jsonl_session(source_dir.path(), &cwd.path().to_string_lossy())
                .await
                .unwrap();
        let source_path = source.storage().persistence_path().unwrap();
        let destination = crate::session::default_session_dir(cwd.path())
            .join(std::path::Path::new(&source_path).file_name().unwrap());
        assert!(!destination.exists());
        import_session(
            &harness,
            &lane,
            &source_path,
            cwd.path(),
            &state.chat_container,
            &state,
            &context,
        )
        .await;
        assert!(
            !destination.exists(),
            "blocked import must not copy/overwrite session files"
        );
        assert_eq!(harness.session().storage().metadata().id, old_id);
        assert_eq!(harness.queued_messages().await.unwrap(), queue);
        assert!(!cancel.is_cancelled());
        assert!(state.user_bash_running());
        state.finish_user_bash();
        harness.close().await.unwrap();
    });
}

#[test]
fn session_drain_deferred_bash_is_persisted_to_old_session_before_new() {
    isolated_async(async {
        let (harness, events) =
            scripted_harness(ice_ai::providers::faux::FauxScript::new().with_text("old reply"))
                .await;
        harness.prompt_text("old prompt", vec![]).await.unwrap();
        let old = ice_harness::session::Session::new(harness.session().storage(), None);
        let state = test_tui_state();
        state.pending_bash_messages.lock().unwrap().push(
            crate::interactive_tui::BufferedBashMessage {
                storage: harness.session().storage(),
                message: ice_harness::messages::bash_execution_message(
                    "printf old-bash",
                    "old-bash",
                    Some(0),
                    false,
                    false,
                    None,
                    Some(false),
                    now_ms(),
                ),
            },
        );
        let requests = state.event_drain.install();
        let lane = harness.lane("main");
        let drain = tokio::spawn(drain_agent_events(
            events,
            test_terminal(),
            state.clone(),
            state.chat_container.clone(),
            AgentHarness::clone(&harness),
            requests,
        ));
        let cwd = tempfile::tempdir().unwrap();
        let context =
            ice_extensions::ToolCallContext::new(cwd.path().to_string_lossy().into_owned());
        assert!(
            start_new_session(
                &harness,
                &lane,
                cwd.path(),
                &state.chat_container,
                &state,
                &context
            )
            .await
        );
        assert!(state.pending_bash_messages.lock().unwrap().is_empty());
        let old_entries = old
            .view("main")
            .find_entries(&Default::default())
            .await
            .unwrap();
        assert!(format!("{old_entries:?}").contains("old-bash"));
        let new_entries = harness
            .session()
            .view("main")
            .find_entries(&Default::default())
            .await
            .unwrap();
        assert!(!format!("{new_entries:?}").contains("old-bash"));
        state.event_drain.flush().await.unwrap();
        assert!(!display(&state).contains("old-bash"));
        drain.abort();
        harness.close().await.unwrap();
    });
}

#[test]
fn run_summary_authoritative_outcomes_and_result_before_broadcast_drain() {
    isolated_async(async {
        use ice_ai::providers::faux::{faux_assistant_message, FauxScript, FauxStep};
        use ice_ai::types::{DeferredHandle, StopReason};
        for stop in [
            StopReason::Stop,
            StopReason::ToolUse,
            StopReason::Error,
            StopReason::Aborted,
            StopReason::Deferred,
        ] {
            let script = FauxScript::new();
            let mut reply = faux_assistant_message("terminal response", stop.clone());
            if stop == StopReason::Deferred {
                reply.deferred = Some(DeferredHandle {
                    provider: "faux".into(),
                    model_id: "faux-1".into(),
                    api: "faux".into(),
                    id: "parked-summary-test".into(),
                    expires_at: None,
                    poll_after_ms: None,
                    data: None,
                });
            }
            script.set_responses(vec![FauxStep::message(reply)]);
            let (harness, events) = scripted_harness(script).await;
            let state = test_tui_state();
            let requests = state.event_drain.install();
            let _watcher = watch_run_summaries(&harness, &state);
            // Like an extension, call the lane directly, without the TUI prompt
            // wrapper and without setting Aborting/Working on TuiState.
            let lane = harness.lane("main");
            let result = lane.prompt_text("extension prompt", vec![]).await.unwrap();
            assert!(!events.is_empty());
            assert!(!display(&state).contains("✻ "));
            let completed = matches!(result.outcome, HarnessRunOutcome::Completed { .. });
            assert_eq!(
                completed,
                matches!(stop, StopReason::Stop | StopReason::ToolUse),
                "{result:?}"
            );
            if stop == StopReason::Deferred {
                assert!(matches!(
                    result.outcome,
                    HarnessRunOutcome::Suspended { .. }
                ));
            }
            let drain = tokio::spawn(drain_agent_events(
                events,
                test_terminal(),
                state.clone(),
                state.chat_container.clone(),
                harness.clone(),
                requests,
            ));
            state.event_drain.flush().await.unwrap();
            let text = display(&state);
            assert_eq!(
                text.matches("✻ ").count(),
                usize::from(completed),
                "{stop:?}: {text}"
            );
            if completed {
                assert!(
                    text.find("terminal response").unwrap() < text.find("✻ ").unwrap(),
                    "{text}"
                );
                assert!(state.current_assistant.lock().unwrap().is_none());
            }
            drain.abort();
            harness.close().await.unwrap();
        }
    });
}

struct AbortDuringTool {
    schema: ice_ai::types::Tool,
    started: Arc<tokio::sync::Notify>,
}

#[async_trait::async_trait]
impl ice_agent::AgentTool for AbortDuringTool {
    fn schema(&self) -> &ice_ai::types::Tool {
        &self.schema
    }
    fn label(&self) -> &str {
        "abort-tool"
    }
    async fn execute(
        &self,
        _: &str,
        _: serde_json::Value,
        signal: tokio_util::sync::CancellationToken,
        _: Arc<dyn Fn(ice_agent::ToolResultPartial) + Send + Sync>,
    ) -> Result<ice_agent::AgentToolResult, ice_agent::AgentError> {
        self.started.notify_one();
        signal.cancelled().await;
        Err(ice_agent::AgentError::Abort)
    }
}

#[test]
fn run_summary_direct_extension_abort_retaining_tooluse_is_not_success() {
    isolated_async(async {
        use ice_ai::providers::faux::{FauxProvider, FauxScript};
        let started = Arc::new(tokio::sync::Notify::new());
        let tool = AbortDuringTool {
            schema: ice_ai::types::Tool {
                name: "abort-tool".into(),
                description: "wait until aborted".into(),
                parameters: ice_ai::types::Schema::new(
                    serde_json::json!({"type":"object", "properties":{}}),
                ),
                constrained_sampling: None,
            },
            started: started.clone(),
        };
        let script = FauxScript::new().with_tool_call("abort-tool", serde_json::json!({}));
        let provider = FauxProvider::new(script);
        let (emitter, events) = ice_agent::events::BroadcastEmitter::new(256);
        let harness = AgentHarness::create(ice_harness::types::AgentHarnessOptions {
            model: provider.default_model().clone(),
            models: vec![provider],
            tools: vec![ice_harness::types::HarnessTool::new(Arc::new(tool))],
            active_tool_names: vec!["abort-tool".into()],
            agent_emitter: Some(Arc::new(emitter)),
            ..Default::default()
        })
        .await
        .unwrap();
        let state = test_tui_state();
        let requests = state.event_drain.install();
        let _watcher = watch_run_summaries(&harness, &state);
        let lane = harness.lane("main");
        let abort = async {
            tokio::time::timeout(std::time::Duration::from_secs(3), started.notified())
                .await
                .unwrap();
            // No keyboard/session cancellation status change is involved.
            assert!(*state.status.lock().unwrap() == RunStatus::Idle);
            lane.abort().await.unwrap();
        };
        let (result, ()) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            tokio::join!(lane.prompt_text("extension tool run", vec![]), abort)
        })
        .await
        .unwrap();
        match result.unwrap().outcome {
            HarnessRunOutcome::Aborted { final_message, .. } => assert_eq!(
                final_message.stop_reason,
                ice_ai::types::StopReason::ToolUse
            ),
            other => panic!("expected authoritative abort: {other:?}"),
        }
        let drain = tokio::spawn(drain_agent_events(
            events,
            test_terminal(),
            state.clone(),
            state.chat_container.clone(),
            AgentHarness::clone(&harness),
            requests,
        ));
        state.event_drain.flush().await.unwrap();
        assert!(!display(&state).contains("✻ "), "{}", display(&state));
        assert!(state
            .tool_components
            .lock()
            .unwrap()
            .values()
            .all(|tool| tool.status() != ToolStatus::Running));
        drain.abort();
        harness.close().await.unwrap();
    });
}

#[test]
fn run_summary_failed_run_cannot_reuse_retained_success_message() {
    isolated_async(async {
        use ice_ai::providers::faux::FauxScript;
        use ice_harness::events::{HarnessEvent, RunEndEvent, RunEndOutcome, RunStartEvent};
        let (harness, _unused) = scripted_harness(FauxScript::new()).await;
        let state = test_tui_state();
        *state.last_assistant_text.lock().unwrap() = "earlier successful reply".into();
        add_note_message(&state.chat_container, "earlier successful reply");
        let requests = state.event_drain.install();
        let _watcher = watch_run_summaries(&harness, &state);
        let (emitter, events) = ice_agent::events::BroadcastEmitter::new(256);
        // Deliberately retain a successful assistant in AgentEnd, while the
        // authoritative lifecycle reports failure. The old heuristic emitted
        // success here even though this operation did not complete.
        use ice_agent::events::AgentEmitter;
        harness
            .events()
            .emit(&HarnessEvent::RunStart(RunStartEvent {
                lane: "main".into(),
                run_id: "failed".into(),
            }));
        emitter.emit(AgentEvent::AgentStart).await;
        emitter
            .emit(AgentEvent::AgentEnd {
                messages: vec![AgentMessage::Assistant(Box::new(
                    ice_ai::providers::faux::faux_assistant_message(
                        "retained success",
                        ice_ai::types::StopReason::Stop,
                    ),
                ))],
            })
            .await;
        harness.events().emit(&HarnessEvent::RunEnd(RunEndEvent {
            lane: "main".into(),
            run_id: "failed".into(),
            outcome: RunEndOutcome::Failed,
            leaf_id: String::new(),
        }));
        emitter
            .emit(AgentEvent::RunSettled {
                run_id: "failed".into(),
            })
            .await;
        let drain = tokio::spawn(drain_agent_events(
            events,
            test_terminal(),
            state.clone(),
            state.chat_container.clone(),
            AgentHarness::clone(&harness),
            requests,
        ));
        state.event_drain.flush().await.unwrap();
        let text = display(&state);
        assert!(text.contains("earlier successful reply"));
        assert!(!text.contains("✻ "), "{text}");
        drain.abort();
        harness.close().await.unwrap();
    });
}

#[test]
fn session_history_and_copy_follow_selected_branch_without_erasing_paid_usage() {
    isolated_async(async {
        use ice_ai::providers::faux::{faux_assistant_message, FauxScript};
        let (harness, _events) = scripted_harness(FauxScript::new()).await;
        let session = harness.session();
        let tree = session.view("main");
        let reply = |text: &str, input: i64| {
            let mut message = faux_assistant_message(text, ice_ai::types::StopReason::Stop);
            message.usage.input = input;
            message.usage.cost.total = input as f64 / 100.0;
            AgentMessage::Assistant(Box::new(message))
        };
        let root = tree
            .append_message(reply("ROOT_SELECTED_REPLY", 10))
            .await
            .unwrap();
        tree.append_message(reply("ABANDONED_FUTURE_REPLY", 20))
            .await
            .unwrap();
        session
            .storage()
            .move_lane("main", Some(&root))
            .await
            .unwrap();
        let alternate = tree
            .append_message(reply("ALTERNATE_BRANCH_REPLY", 30))
            .await
            .unwrap();
        let mut summary_usage = ice_ai::types::Usage::zero();
        summary_usage.input = 5;
        summary_usage.cost.total = 0.05;
        session
            .storage()
            .append_entry(
                ice_harness::session::types::ProvisionedEntry {
                    id: "paid-orphan-summary".into(),
                    kind: ice_harness::session::types::ProvisionedKind::BranchSummary {
                        from_id: alternate.clone(),
                        summary: "PAID_ORPHAN_SUMMARY".into(),
                        details: None,
                        usage: Some(summary_usage),
                    },
                },
                "main",
            )
            .await
            .unwrap();
        session
            .storage()
            .move_lane("main", Some(&root))
            .await
            .unwrap();
        let state = test_tui_state();
        replace_session_ui(&harness, &state.chat_container, &state).await;
        let text = display(&state);
        assert!(text.contains("ROOT_SELECTED_REPLY"), "{text}");
        assert!(!text.contains("ABANDONED_FUTURE_REPLY"), "{text}");
        assert!(!text.contains("ALTERNATE_BRANCH_REPLY"), "{text}");
        assert!(!text.contains("PAID_ORPHAN_SUMMARY"), "{text}");
        assert_eq!(
            *state.last_assistant_text.lock().unwrap(),
            "ROOT_SELECTED_REPLY"
        );
        assert_eq!(state.footer.usage().input, 65);
        assert!((state.footer.usage().cost - 0.65).abs() < 0.00001);
        session
            .storage()
            .move_lane("main", Some(&alternate))
            .await
            .unwrap();
        replace_session_ui(&harness, &state.chat_container, &state).await;
        let text = display(&state);
        assert!(
            text.find("ROOT_SELECTED_REPLY").unwrap()
                < text.find("ALTERNATE_BRANCH_REPLY").unwrap(),
            "{text}"
        );
        assert!(!text.contains("ABANDONED_FUTURE_REPLY"), "{text}");
        assert_eq!(
            *state.last_assistant_text.lock().unwrap(),
            "ALTERNATE_BRANCH_REPLY"
        );
        assert_eq!(state.footer.usage().input, 65);
        harness.close().await.unwrap();
    });
}

#[test]
fn run_summary_waits_for_context_accepted_by_run_end_listener() {
    isolated_async(async {
        use ice_ai::providers::faux::FauxScript;
        use ice_harness::events::HarnessEvent;
        let (harness, events) =
            scripted_harness(FauxScript::new().with_text("terminal reply")).await;
        let harness = Arc::new(harness);
        let state = test_tui_state();
        let requests = state.event_drain.install();
        let _summaries = watch_run_summaries(&harness, &state);
        let accepted = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let weak = Arc::downgrade(&harness);
        let listener_accepted = accepted.clone();
        let listener_release = release.clone();
        // Registered after the summary watcher: metadata already exists, but
        // RunSettled must not be emitted until these accepted writes finish.
        let _listener = harness.watch(move |event| {
            if !matches!(event, HarnessEvent::RunEnd(_)) {
                return;
            }
            let harness = weak.upgrade().unwrap();
            tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async {
                    harness
                        .append_message(ice_harness::messages::create_custom_message(
                            "late-context",
                            ice_ai::types::UserContent::Text("AFTER_RUN_END_CONTEXT".into()),
                            true,
                            None,
                            now_ms(),
                        ))
                        .await
                        .unwrap();
                    listener_accepted.notify_one();
                    listener_release.notified().await;
                })
            });
        });
        let drain = tokio::spawn(drain_agent_events(
            events,
            test_terminal(),
            state.clone(),
            state.chat_container.clone(),
            AgentHarness::clone(&harness),
            requests,
        ));
        let prompt = {
            let harness = harness.clone();
            tokio::spawn(async move { harness.prompt_text("prompt", vec![]).await })
        };
        tokio::time::timeout(std::time::Duration::from_secs(3), accepted.notified())
            .await
            .unwrap();
        // The consumer has CPU time while the run still owns its finishing
        // lease. Draining only currently available broadcasts would print here.
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        assert!(!display(&state).contains("✻ "), "{}", display(&state));
        release.notify_one();
        tokio::time::timeout(std::time::Duration::from_secs(3), prompt)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        state.event_drain.flush().await.unwrap();
        let text = display(&state);
        assert!(
            text.find("AFTER_RUN_END_CONTEXT").unwrap() < text.find("✻ ").unwrap(),
            "{text}"
        );
        assert_eq!(text.matches("✻ ").count(), 1);
        drain.abort();
        harness.close().await.unwrap();
    });
}

#[test]
fn navigation_interrupt_captures_ownership_before_spawned_abort_is_polled() {
    with_isolated_config_dir(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let (harness, _events) =
                    scripted_harness(ice_ai::providers::faux::FauxScript::new()).await;
                harness
                    .steer(user_message_with_images("keep source queue", vec![]))
                    .await
                    .unwrap();
                let queue = harness.queued_messages().await.unwrap();
                let leaf = harness.lane("main").get_leaf_id().await.unwrap();
                let state = test_tui_state();
                state.editor.set_text("keep source draft");
                state
                    .tree_navigation_active
                    .store(true, std::sync::atomic::Ordering::Release);
                state.set_status(RunStatus::Working);
                assert!(dispatch_run_interrupt(
                    &KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
                    &ice_tui::Keybindings::new(),
                    harness.lane("main"),
                    state.editor.clone(),
                    state.clone(),
                    test_terminal(),
                ));
                // Current-thread runtime: no spawned abort can run until we yield.
                // Model the navigation's RAII completion before that first poll.
                state
                    .tree_navigation_active
                    .store(false, std::sync::atomic::Ordering::Release);
                state.set_status(RunStatus::Idle);
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                assert_eq!(state.editor.get_text(), "keep source draft");
                assert_eq!(harness.queued_messages().await.unwrap(), queue);
                assert_eq!(harness.lane("main").get_leaf_id().await.unwrap(), leaf);
                harness.close().await.unwrap();
            });
    });
}

#[test]
fn run_summary_stopped_consumer_does_not_hide_authoritative_provider_error() {
    isolated_async(async {
        use ice_ai::providers::faux::{faux_assistant_message, FauxScript, FauxStep};
        let script = FauxScript::new();
        let mut failed = faux_assistant_message("", ice_ai::types::StopReason::Error);
        failed.error_message = Some("provider rejected the request".into());
        script.set_responses(vec![FauxStep::message(failed)]);
        let (harness, _events) = scripted_harness(script).await;
        let state = test_tui_state();
        drop(state.event_drain.install());
        run_prompt_streaming(
            &harness,
            "prompt",
            false,
            &test_terminal(),
            &state,
            true,
            vec![],
        )
        .await;
        let text = display(&state);
        assert!(text.contains("response updates are unavailable"), "{text}");
        assert!(text.contains("provider rejected the request"), "{text}");
        assert!(!text.contains("✻ "), "{text}");
        harness.close().await.unwrap();
    });
}

#[test]
fn run_summary_blocking_completion_follows_final_response() {
    isolated_async(async {
        let (harness, _events) = scripted_harness(
            ice_ai::providers::faux::FauxScript::new().with_text("blocking final response"),
        )
        .await;
        let state = test_tui_state();
        run_prompt_streaming(
            &harness,
            "prompt",
            false,
            &test_terminal(),
            &state,
            false,
            vec![],
        )
        .await;
        let text = display(&state);
        assert!(
            text.find("blocking final response").unwrap() < text.find("✻ ").unwrap(),
            "{text}"
        );
        assert_eq!(text.matches("✻ ").count(), 1);
        harness.close().await.unwrap();
    });
}

#[test]
fn session_drain_new_aborts_extension_run_before_agent_start_is_delivered() {
    isolated_async(async {
        use ice_ai::providers::faux::{faux_assistant_message, FauxScript, FauxStep};
        let entered = Arc::new(tokio::sync::Notify::new());
        let signal = entered.clone();
        let script = FauxScript::new().with_tokens_per_second(50.0);
        script.set_responses(vec![FauxStep::factory(move |_| {
            signal.notify_one();
            faux_assistant_message(
                "slow old response ".repeat(100),
                ice_ai::types::StopReason::Stop,
            )
        })]);
        let (harness, events) = scripted_harness(script).await;
        let harness = Arc::new(harness);
        let state = test_tui_state();
        let requests = state.event_drain.install();
        let prompt = {
            let harness = harness.clone();
            tokio::spawn(async move { harness.prompt_text("extension prompt", vec![]).await })
        };
        tokio::time::timeout(std::time::Duration::from_secs(3), entered.notified())
            .await
            .unwrap();
        assert!(*state.status.lock().unwrap() == RunStatus::Idle);
        assert!(harness.snapshot().await.unwrap().active.is_some());
        let lane = harness.lane("main");
        let cwd = tempfile::tempdir().unwrap();
        let context =
            ice_extensions::ToolCallContext::new(cwd.path().to_string_lossy().into_owned());
        let change = start_new_session(
            &harness,
            &lane,
            cwd.path(),
            &state.chat_container,
            &state,
            &context,
        );
        tokio::pin!(change);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(30), &mut change)
                .await
                .is_err()
        );
        // /new must abort the authoritative active lane even though the UI has
        // not yet seen AgentStart. It then remains blocked on the UI barrier.
        let old = tokio::time::timeout(std::time::Duration::from_secs(3), prompt)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(matches!(old.outcome, HarnessRunOutcome::Aborted { .. }));
        let drain = tokio::spawn(drain_agent_events(
            events,
            test_terminal(),
            state.clone(),
            state.chat_container.clone(),
            AgentHarness::clone(&harness),
            requests,
        ));
        assert!(change.await);
        state.event_drain.flush().await.unwrap();
        assert!(state.last_assistant_text.lock().unwrap().is_empty());
        assert!(!display(&state).contains("slow old response"));
        drain.abort();
        harness.close().await.unwrap();
    });
}

#[test]
fn run_summary_settled_a_precedes_streaming_extension_b_without_waiting_for_b() {
    isolated_async(async {
        use ice_ai::providers::faux::{faux_assistant_message, FauxScript, FauxStep};
        let entered_b = Arc::new(tokio::sync::Notify::new());
        let notify_b = entered_b.clone();
        let script = FauxScript::new().with_tokens_per_second(50.0);
        script.set_responses(vec![
            FauxStep::text("A_FINAL"),
            FauxStep::factory(move |_| {
                notify_b.notify_one();
                faux_assistant_message("B_STREAM ".repeat(200), ice_ai::types::StopReason::Stop)
            }),
        ]);
        let (harness, events) = scripted_harness(script).await;
        let harness = Arc::new(harness);
        let state = test_tui_state();
        let requests = state.event_drain.install();
        let _summaries = watch_run_summaries(&harness, &state);
        assert!(matches!(
            harness.prompt_text("A", vec![]).await.unwrap().outcome,
            HarnessRunOutcome::Completed { .. }
        ));
        // A is fully buffered, and the extension immediately starts B before
        // the UI consumer is scheduled. Waiting for global lane-idle now would
        // freeze B's stream and move A's summary behind B's output.
        let b = {
            let harness = harness.clone();
            tokio::spawn(async move { harness.prompt_text("B", vec![]).await })
        };
        tokio::time::timeout(std::time::Duration::from_secs(3), entered_b.notified())
            .await
            .unwrap();
        let lane = harness.lane("main");
        assert!(lane.snapshot().await.unwrap().active.is_some());
        let drain = tokio::spawn(drain_agent_events(
            events,
            test_terminal(),
            state.clone(),
            state.chat_container.clone(),
            AgentHarness::clone(&harness),
            requests,
        ));
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while !display(&state).contains("B_STREAM") {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert!(!b.is_finished());
        let text = display(&state);
        assert!(
            text.find("A_FINAL").unwrap() < text.find("✻ ").unwrap(),
            "{text}"
        );
        assert!(
            text.find("✻ ").unwrap() < text.find("B_STREAM").unwrap(),
            "{text}"
        );
        assert_eq!(text.matches("✻ ").count(), 1);
        lane.abort().await.unwrap();
        let result = tokio::time::timeout(std::time::Duration::from_secs(3), b)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(matches!(result.outcome, HarnessRunOutcome::Aborted { .. }));
        state.event_drain.flush().await.unwrap();
        assert_eq!(display(&state).matches("✻ ").count(), 1);
        drain.abort();
        harness.close().await.unwrap();
    });
}

#[test]
fn run_summary_metadata_is_bounded_and_settled_markers_are_consumed_once() {
    isolated_async(async {
        use ice_harness::events::{HarnessEvent, RunEndEvent, RunEndOutcome, RunStartEvent};
        let (harness, _events) = scripted_harness(ice_ai::providers::faux::FauxScript::new()).await;
        let state = test_tui_state();
        let _summaries = watch_run_summaries(&harness, &state);
        for index in 0..300 {
            let run_id = format!("run-{index}");
            harness
                .events()
                .emit(&HarnessEvent::RunStart(RunStartEvent {
                    lane: "main".into(),
                    run_id: run_id.clone(),
                }));
            harness.events().emit(&HarnessEvent::RunEnd(RunEndEvent {
                lane: "main".into(),
                run_id,
                outcome: RunEndOutcome::Completed,
                leaf_id: String::new(),
            }));
        }
        let tui = test_terminal();
        handle_agent_event(
            AgentEvent::RunSettled {
                run_id: "run-0".into(),
            },
            &tui,
            &state,
            &state.chat_container,
            &harness,
        )
        .await;
        assert!(!display(&state).contains("✻ "));
        for _ in 0..2 {
            handle_agent_event(
                AgentEvent::RunSettled {
                    run_id: "run-299".into(),
                },
                &tui,
                &state,
                &state.chat_container,
                &harness,
            )
            .await;
        }
        assert_eq!(display(&state).matches("✻ ").count(), 1);
        harness.close().await.unwrap();
    });
}

/// Esc can land while navigation is still waiting to be admitted as a run, when
/// the lane reports no active run. The abort must keep trying until navigation
/// is admitted or ends, not be discarded.
#[test]
fn navigation_interrupt_retries_while_no_run_is_admitted_yet() {
    isolated_async(async {
        let (harness, _events) = scripted_harness(ice_ai::providers::faux::FauxScript::new()).await;
        let state = test_tui_state();
        *state.status.lock().unwrap() = RunStatus::Aborting;
        state
            .tree_navigation_active
            .store(true, std::sync::atomic::Ordering::Release);
        let task = tokio::spawn(abort_interrupted_run(
            harness.lane("main"),
            state.editor.clone(),
            state.clone(),
            test_terminal(),
            true,
        ));
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(!task.is_finished(), "must keep retrying, not give up");
        // Navigation ended (TreeNavigationActivity::drop).
        state
            .tree_navigation_active
            .store(false, std::sync::atomic::Ordering::Release);
        *state.status.lock().unwrap() = RunStatus::Idle;
        tokio::time::timeout(std::time::Duration::from_secs(3), task)
            .await
            .unwrap()
            .unwrap();
        harness.close().await.unwrap();
    });
}

#[test]
fn navigation_completion_wins_status_lock_before_interrupt_dispatch() {
    isolated_async(async {
        let (harness, _events) = scripted_harness(ice_ai::providers::faux::FauxScript::new()).await;
        harness
            .steer(user_message_with_images("keep source queue", vec![]))
            .await
            .unwrap();
        let queued = harness.queued_messages().await.unwrap();
        let state = test_tui_state();
        state.editor.set_text("keep source draft");
        let mut status = state.status.lock().unwrap();
        *status = RunStatus::Working;
        state
            .tree_navigation_active
            .store(true, std::sync::atomic::Ordering::Release);
        let thread_state = state.clone();
        let lane = harness.lane("main");
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            let routed = dispatch_run_interrupt(
                &KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
                &ice_tui::Keybindings::new(),
                lane,
                thread_state.editor.clone(),
                thread_state,
                test_terminal(),
            );
            done_tx.send(routed).unwrap();
        });
        started_rx.recv().unwrap();
        assert!(done_rx
            .recv_timeout(std::time::Duration::from_millis(20))
            .is_err());
        // Model TreeNavigationActivity::drop's exact ownership/status update
        // while dispatch waits on the same mutex. Completion must win wholly.
        state
            .tree_navigation_active
            .store(false, std::sync::atomic::Ordering::Release);
        *status = RunStatus::Idle;
        drop(status);
        assert!(!done_rx
            .recv_timeout(std::time::Duration::from_secs(3))
            .unwrap());
        thread.join().unwrap();
        assert!(*state.status.lock().unwrap() == RunStatus::Idle);
        assert_eq!(state.editor.get_text(), "keep source draft");
        assert_eq!(harness.queued_messages().await.unwrap(), queued);
        harness.close().await.unwrap();
    });
}
