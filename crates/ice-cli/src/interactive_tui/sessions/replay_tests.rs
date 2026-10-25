use super::*;
use crate::transcript_view::UiEvent;
use ice_ai::types::{ToolCall, ToolCallType, ToolResultMessage, ToolResultRole};
use ice_harness::session::{SessionValueWriter, SessionValues};
use serde_json::json;

fn replay_output(marker: &str) -> String {
    (0..24)
        .map(|index| format!("output line {index}"))
        .chain(std::iter::once(marker.to_owned()))
        .collect::<Vec<_>>()
        .join("\n")
}

fn assert_replay_expansion(state: &TuiState, expanded: bool) {
    let output = strip_ansi(&state.chat_container.render(100).join("\n"));
    for marker in ["TOOL_REPLAY_END", "READ_REPLAY_END", "SEARCH_REPLAY_END"] {
        assert_eq!(output.contains(marker), expanded, "{marker}: {output}");
    }
    let children = state.chat_container.get_children();
    assert_eq!(
        children
            .iter()
            .filter(|child| child.as_any().is::<ice_tui::ToolGroupComponent>())
            .count(),
        1,
        "restored exploration tools must stay in one group"
    );
    let mut standalone = 0;
    for child in children {
        if let Some(panel) = child.as_any().downcast_ref::<ToolExecutionComponent>() {
            assert_eq!(panel.is_expanded(), expanded);
            standalone += 1;
        }
    }
    assert_eq!(
        standalone, 2,
        "the ordinary tool and plan panel are restored"
    );
    assert_eq!(state.transcript_view().expanded(), expanded);
}

#[test]
fn replay_preserves_saved_expansion_live_members_and_first_toggle() {
    with_isolated_config_dir(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let directory = tempfile::tempdir().unwrap();
                let cwd = directory.path().to_string_lossy().to_string();
                let harness = AgentHarness::create(ice_harness::types::AgentHarnessOptions {
                    session: crate::session::create_jsonl_session(directory.path(), &cwd)
                        .await
                        .unwrap(),
                    ..Default::default()
                })
                .await
                .unwrap();
                harness
                    .session()
                    .append_custom_entry(
                        "plan-mode",
                        Some(json!({"active": false, "plan": "Saved plan"})),
                    )
                    .await
                    .unwrap();
                let calls = [
                    (
                        "extension_tool",
                        json!({"path": "notes.txt"}),
                        "TOOL_REPLAY_END",
                    ),
                    ("read", json!({"path": "src/lib.rs"}), "READ_REPLAY_END"),
                    (
                        "bash",
                        json!({"command": "rg symbol src"}),
                        "SEARCH_REPLAY_END",
                    ),
                ];
                let mut assistant = AssistantMessage::empty(ice_ai::Api::Faux, "faux", "m", 0);
                assistant.content = calls
                    .iter()
                    .map(|(name, arguments, _)| {
                        Content::ToolCall(ToolCall {
                            kind: ToolCallType,
                            id: (*name).into(),
                            name: (*name).into(),
                            arguments: arguments.clone(),
                            thought_signature: None,
                            namespace: None,
                        })
                    })
                    .collect();
                harness
                    .session()
                    .append_message(AgentMessage::Assistant(Box::new(assistant)))
                    .await
                    .unwrap();
                for (name, _, marker) in calls {
                    harness
                        .session()
                        .append_message(AgentMessage::ToolResult(Box::new(ToolResultMessage {
                            role: ToolResultRole,
                            tool_call_id: name.into(),
                            tool_name: name.into(),
                            content: vec![Content::Text(ice_ai::types::TextContent {
                                kind: ice_ai::types::TextContentType,
                                text: replay_output(marker),
                                text_signature: None,
                            })],
                            details: None,
                            usage: None,
                            added_tool_names: vec![],
                            is_error: false,
                            timestamp: 0,
                        })))
                        .await
                        .unwrap();
                }
                for saved in [false, true] {
                    SessionValueWriter::new(harness.session().clone())
                        .set("display.tool_outputs_expanded", json!(saved))
                        .await
                        .unwrap();
                    let values = SessionValues::load(harness.session()).await.unwrap();
                    let expanded = values
                        .get_as::<bool>("display.tool_outputs_expanded")
                        .unwrap();
                    let state = test_tui_state();
                    *state.tool_outputs_expanded.lock().unwrap() =
                        crate::transcript_view::ToolExpansion::new(expanded);
                    // Startup resolves the saved value before creating the panels.
                    render_session_history(
                        &harness,
                        &state.chat_container,
                        None,
                        None,
                        true,
                        expanded,
                    )
                    .await;
                    assert_replay_expansion(&state, saved);

                    let view = state.transcript_view();
                    view.apply(
                        &UiEvent::ToolStart {
                            id: "live-read".into(),
                            name: "read".into(),
                            args: json!({"path": "src/new.rs"}),
                        },
                        100,
                    );
                    view.apply(&UiEvent::ToolEnd {
                        id: "live-read".into(), name: "read".into(),
                        result: json!({"content": [{"type": "text", "text": "LIVE_REPLAY_END"}]}),
                        is_error: false,
                    }, 100);
                    assert_replay_expansion(&state, saved);
                    assert_eq!(
                        state
                            .chat_container
                            .render(100)
                            .join("\n")
                            .contains("LIVE_REPLAY_END"),
                        saved
                    );

                    // The very first toggle must change both the flag and the display.
                    assert_eq!(state.transcript_view().toggle_expanded(), !saved);
                    assert_replay_expansion(&state, !saved);
                    assert_eq!(
                        state
                            .chat_container
                            .render(100)
                            .join("\n")
                            .contains("LIVE_REPLAY_END"),
                        !saved
                    );

                    // Replacement keeps the current choice, even when it differs
                    // from the value that this session originally restored.
                    replace_session_ui(&harness, &state.chat_container, &state).await;
                    assert_replay_expansion(&state, !saved);
                    assert_eq!(state.transcript_view().toggle_expanded(), saved);
                    assert_replay_expansion(&state, saved);

                    // Tree navigation rebuilds the same history with the current flag.
                    state.chat_container.clear();
                    render_session_history(
                        &harness,
                        &state.chat_container,
                        None,
                        None,
                        true,
                        view.expanded(),
                    )
                    .await;
                    assert_replay_expansion(&state, saved);
                }
                harness.close().await.unwrap();
            });
    });
}
