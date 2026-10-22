//! Session replacement must retire state that belongs to the old transcript.

use super::*;
use std::time::Duration;

fn cache_message(timestamp: i64, cache_write: i64) -> AssistantMessage {
    let mut message = AssistantMessage::empty(
        ice_ai::Api::AnthropicMessages,
        "anthropic",
        "session-reset-test",
        timestamp,
    );
    message.usage.input = 20_000 - cache_write;
    message.usage.cache_write = cache_write;
    message.usage.total_tokens = 20_000;
    message
}

#[test]
fn new_session_retires_stale_assistant_cache_queue_and_auto_send() {
    with_isolated_config_dir(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let cwd = tempfile::tempdir().unwrap();
                let harness = AgentHarness::create(Default::default()).await.unwrap();
                harness
                    .steer(user_message_with_images("old queued prompt", vec![]))
                    .await
                    .unwrap();
                let state = test_tui_state();
                state.set_pending_queue(harness.queued_messages().await.unwrap());
                assert!(state.pending_container.child_count() > 0);

                let assistant = Arc::new(AssistantMessageComponent::new(
                    AssistantMessageOptions::default(),
                ));
                assistant.set_streaming(true);
                assistant.update_blocks(&[AssistantBlock::Text("old partial reply".into())]);
                state.chat_container.add_child(assistant.clone());
                *state.current_assistant.lock().unwrap() = Some(assistant);

                let previous = cache_message(1_000, 10_000);
                let next = cache_message(2_000, 0);
                let mut control = ice_harness::cache_stats::CacheMissTracker::new();
                control.observe(&previous, &ice_harness::cache_stats::NoPrices);
                assert!(control
                    .observe(&next, &ice_harness::cache_stats::NoPrices)
                    .is_some());
                state
                    .cache_tracker
                    .lock()
                    .unwrap()
                    .observe(&previous, &ice_harness::cache_stats::NoPrices);

                state.editor.set_text("keep this editable draft");
                *state.auto_send.lock().unwrap() = Some(AutoSendPending {
                    deadline: std::time::Instant::now() + Duration::from_secs(60),
                    text: state.editor.get_text(),
                });
                let lane = harness.lane("main");
                let tool_context =
                    ice_extensions::ToolCallContext::new(cwd.path().to_string_lossy().to_string());
                assert!(
                    start_new_session(
                        &harness,
                        &lane,
                        cwd.path(),
                        &state.chat_container,
                        &state,
                        &tool_context,
                    )
                    .await
                );

                assert!(state.current_assistant.lock().unwrap().is_none());
                assert!(state.auto_send.lock().unwrap().is_none());
                assert_eq!(state.editor.get_text(), "keep this editable draft");
                assert!(state
                    .cache_tracker
                    .lock()
                    .unwrap()
                    .observe(&next, &ice_harness::cache_stats::NoPrices)
                    .is_none());
                assert!(harness.queued_messages().await.unwrap().is_empty());
                assert!(state.pending_snapshot.lock().unwrap().is_empty());
                assert_eq!(state.pending_container.child_count(), 0);
                let text = strip_ansi(&state.chat_container.render(120).join("\n"));
                assert!(!text.contains("old partial reply"));
                harness.close().await.unwrap();
            })
    })
}

#[test]
fn replacement_displays_the_target_sessions_persisted_queue() {
    with_isolated_config_dir(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let cwd = tempfile::tempdir().unwrap();
                let sessions = tempfile::tempdir().unwrap();
                let target = crate::session::create_jsonl_session(
                    sessions.path(),
                    &cwd.path().to_string_lossy(),
                )
                .await
                .unwrap();
                let target_storage = target.storage();
                let target_harness =
                    AgentHarness::create(ice_harness::types::AgentHarnessOptions {
                        session: target,
                        ..Default::default()
                    })
                    .await
                    .unwrap();
                target_harness
                    .steer(user_message_with_images("target steering prompt", vec![]))
                    .await
                    .unwrap();
                target_harness
                    .follow_up(user_message_with_images("target follow-up prompt", vec![]))
                    .await
                    .unwrap();
                let expected = target_harness.queued_messages().await.unwrap();
                target_harness.close().await.unwrap();
                let saved =
                    std::fs::read_to_string(target_storage.persistence_path().unwrap()).unwrap();
                assert!(saved.contains("target steering prompt"));
                assert!(saved.contains("target follow-up prompt"));

                let harness = AgentHarness::create(Default::default()).await.unwrap();
                harness
                    .steer(user_message_with_images("old queued prompt", vec![]))
                    .await
                    .unwrap();
                let state = test_tui_state();
                state.set_pending_queue(harness.queued_messages().await.unwrap());
                // A separate harness rebuilds the target queue from its durable
                // records; no live queue Arc is shared with target_harness.
                harness
                    .set_session(ice_harness::session::Session::new(target_storage, None))
                    .await
                    .unwrap();
                assert_eq!(harness.queued_messages().await.unwrap(), expected);
                replace_session_ui(&harness, &state.chat_container, &state).await;

                assert_eq!(*state.pending_snapshot.lock().unwrap(), expected);
                let shown = strip_ansi(&state.pending_container.render(120).join("\n"));
                assert!(shown.contains("target steering prompt"), "{shown}");
                assert!(shown.contains("target follow-up prompt"), "{shown}");
                assert!(!shown.contains("old queued prompt"), "{shown}");
                assert_eq!(harness.queued_messages().await.unwrap(), expected);
                harness.close().await.unwrap();
            })
    })
}
