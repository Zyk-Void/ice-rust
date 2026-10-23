use super::*;
use ice_ai::types::{AssistantMessage, Content, StopReason, UserMessage};
use ice_harness::session::types::{CompactionEntry, EntryBase, MessageEntry};

fn user(text: &str) -> AgentMessage {
    AgentMessage::User(UserMessage::new(text, 0))
}

fn base(kind: &str, id: &str, parent: Option<&str>, seq: u64) -> EntryBase {
    EntryBase {
        entry_type: kind.into(),
        id: id.into(),
        parent_id: parent.map(str::to_string),
        seq,
        timestamp: 0,
    }
}

fn message(id: &str, parent: Option<&str>, seq: u64, text: &str) -> Entry {
    Entry::Message(MessageEntry {
        base: base("message", id, parent, seq),
        message: user(text),
        terminate: None,
    })
}

fn assistant(input: i64, cache: i64, total: i64, reason: StopReason) -> AgentMessage {
    let mut a = AssistantMessage::empty(ice_ai::Api::AnthropicMessages, "test", "model", 0);
    a.content = vec![Content::text("four")];
    a.usage.input = input;
    a.usage.cache_read = cache;
    a.usage.cache_write = 5;
    a.usage.output = 700;
    a.usage.total_tokens = total;
    a.stop_reason = reason;
    AgentMessage::Assistant(Box::new(a))
}

#[test]
fn bounded_largest_remainder_allocation() {
    assert_eq!(
        allocate(&[30_000, 10_000, 0], 40_000, 100_000, 100),
        [30, 10, 0]
    );
    assert_eq!(allocate(&[200; 10], 2000, 100_000, 200), [1; 10]);
    assert_eq!(allocate(&[1, 2, 3, 4, 5], 1, 1000, 3), [0, 0, 1, 1, 1]);
    for weights in [
        vec![],
        vec![0; 9],
        vec![1; 9],
        vec![u64::MAX; 9],
        vec![1, 10000, 0, 1],
    ] {
        for used in [0, 1, 200, u64::MAX] {
            for window in [0, 1, 100_000, u64::MAX] {
                for budget in [0, 1, 3, 200] {
                    let counts = allocate(&weights, used, window, budget);
                    assert_eq!(counts.len(), weights.len());
                    assert!(counts.iter().sum::<usize>() <= budget);
                    for (weight, cells) in weights.iter().zip(&counts) {
                        if *weight == 0 {
                            assert_eq!(*cells, 0);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn compaction_projects_summary_and_retained_tail_not_obsolete_history() {
    let entries = vec![
        message("old", None, 1, &"old".repeat(1000)),
        Entry::Compaction(CompactionEntry {
            base: base("compaction", "compact", Some("old"), 2),
            summary: "summary".into(),
            retained_tail: vec![user("tail")],
            tokens_before: 12345,
            details: None,
            usage: None,
        }),
        message("new", Some("compact"), 3, "next"),
    ];
    let mut snapshot = Snapshot::default();
    snapshot.messages(&entries);
    assert_eq!(snapshot.categories[4], 2);
    assert_eq!(snapshot.categories[7], 2);
    assert_eq!(snapshot.used, 4);
    assert!(snapshot
        .items
        .iter()
        .any(|item| item.0 == "compaction summary"));
}

#[test]
fn model_conversion_omits_excluded_shell_and_unknown_custom_roles() {
    let shell = |exclude| {
        roles::bash_execution_message("ls", "four", Some(0), false, false, None, Some(exclude), 0)
    };
    let mut unknown =
        match roles::create_custom_message("test", "private body".into(), true, None, 0) {
            AgentMessage::Custom(c) => c,
            _ => unreachable!(),
        };
    unknown.role = "not-registered".into();
    let mut snapshot = Snapshot::default();
    snapshot.projected_messages(vec![
        shell(false),
        shell(true),
        AgentMessage::Custom(unknown),
        roles::create_custom_message("test", "four".into(), true, None, 0),
    ]);
    assert_eq!(snapshot.categories[8], 2);
    assert_eq!(snapshot.categories[6], 1);
    assert_eq!(snapshot.used, 3);
    assert_eq!(snapshot.items.len(), 2);
    assert!(!snapshot.render(160).join("\n").contains("private body"));
}

#[test]
fn input_cache_baseline_and_estimated_output_tail_filter_invalid_usage() {
    let mut snapshot = Snapshot::default();
    snapshot.add(0, "system", 9000);
    snapshot.projected_messages(vec![
        assistant(2000, 50, 99999, StopReason::Stop),
        assistant(100, 20, 99999, StopReason::Stop),
        assistant(7000, 30, 99999, StopReason::Error),
        assistant(8000, 40, 99999, StopReason::Aborted),
        user("tail"),
    ]);
    assert_eq!(snapshot.provider, Some(125));
    assert_eq!(snapshot.tail, 4); // measured response, two invalid responses, user tail
    assert_eq!(snapshot.used, 129); // not 99999, not cumulative, not prompt added twice
    assert_eq!(snapshot.categories[0], 9000); // retain heuristic category totals
    let mut no_usage = Snapshot::default();
    no_usage.add(0, "system", 10);
    no_usage.projected_messages(vec![
        assistant(10, 20, 99999, StopReason::Error),
        user("tail"),
    ]);
    assert_eq!(no_usage.provider, None);
    assert_eq!(no_usage.used, 12);
}

#[test]
fn largest_entries_are_stable_and_labels_are_safe() {
    let mut snapshot = Snapshot::default();
    for (label, size) in [
        ("first", 10),
        ("small", 1),
        ("second", 10),
        ("big", 20),
        ("third", 10),
        ("fourth", 10),
        ("last", 0),
    ] {
        snapshot.add(4, label, size);
    }
    let labels: Vec<_> = snapshot
        .largest()
        .iter()
        .map(|item| item.0.as_str())
        .collect();
    assert_eq!(labels, ["big", "first", "second", "third", "fourth"]);
    assert_eq!(safe_label("bash\n\u{1b}[31m"), "bash31m");
    assert_eq!(safe_label(&"x".repeat(100)).len(), 64);
}

#[test]
fn responsive_grid_and_all_lines_fit_width_zero_through_160() {
    let mut snapshot = Snapshot {
        model: "provider/model".into(),
        window: 10000,
        ..Default::default()
    };
    snapshot.prompt(Some("base"), &[]);
    snapshot.projected_messages(vec![user("four")]);
    for width in 0..=160 {
        let lines = snapshot.render(width);
        assert!(
            lines
                .iter()
                .all(|line| ice_tui::visible_width(line) <= width),
            "width={width}"
        );
        if width == 0 {
            assert!(lines.iter().all(String::is_empty));
        }
        if width >= 39 {
            let grid: Vec<_> = lines
                .iter()
                .filter(|line| {
                    let head = line.chars().take(39).collect::<String>();
                    head.len() == 39
                        && head
                            .chars()
                            .all(|ch| SYMBOLS.contains(&ch) || ch == '.' || ch == ' ')
                })
                .collect();
            assert_eq!(grid.len(), 10);
            assert!(grid
                .iter()
                .all(|line| line.chars().take(39).filter(|ch| *ch != ' ').count() == 20));
            if width >= 92 {
                assert!(grid[0].contains("System prompt"));
            } else {
                assert!(!grid[0].contains("System prompt"));
            }
        }
    }
    let text = snapshot.render(160).join("\n");
    for label in [
        "Largest entries",
        "Free space",
        "Category estimates",
        "Not an exact wire payload",
        "provider/model",
    ] {
        assert!(text.contains(label), "{label}");
    }
}

#[tokio::test]
async fn context_command_appends_responsive_panel_without_usage_event_or_session_mutation() {
    let dir = tempfile::tempdir().unwrap();
    let harness = AgentHarness::create(Default::default()).await.unwrap();
    let lane = harness.lane("main");
    lane.append_message(user("private contents must not be printed"))
        .await
        .unwrap();
    let session = Arc::new(crate::agent_session::AgentSession::new(
        harness,
        vec![],
        vec![],
        dir.path(),
    ));
    let tip = session.harness().get_tip_id().await.unwrap();
    let before = session
        .harness()
        .find_entries(&EntryQuery::default())
        .await
        .unwrap();
    let state = crate::interactive_tui::tests::test_tui_state();
    let tui = Arc::new(TuiAltScreen::new(
        Box::new(ProcessTerminal::new()),
        true,
        None,
    ));
    let chat = Arc::new(Container::new());
    let catalog = Arc::new(std::sync::RwLock::new(vec![]));
    let reload_context = Arc::new(crate::session::ReloadContext {
        mcp_tools: crate::mcp::McpManager::new(
            session.harness(),
            &crate::args::parse_args(&[]),
            dir.path(),
            dir.path().join("agent"),
            false,
            Arc::new(Mutex::new(Default::default())),
        ),
        extension_session: state.extension_session.clone(),
        tool_context: ice_extensions::ToolCallContext::new(dir.path().display().to_string()),
        action_bridge: Arc::new(Mutex::new(None)),
        ext_status: state.ext_status.clone(),
        editor_text: state.editor_text.clone(),
        catalog: catalog.clone(),
        providers: Arc::new(std::sync::RwLock::new(vec![])),
        runtime: tokio::runtime::Handle::current(),
        cwd: dir.path().into(),
        project_trusted: false,
        args: crate::args::parse_args(&[]),
        resolved_model: session.model().await.unwrap(),
        broadcast: Arc::new(ice_agent::events::CollectorEmitter::default()),
        mailbox: Default::default(),
        ui_dialog: Default::default(),
        dev_extension: None,
    });
    let (tx, mut rx) = mpsc::unbounded_channel();
    let ctx = CommandContext {
        chat: chat.clone(),
        tui,
        tx,
        state: state.clone(),
        editor: state.editor.clone(),
        editor_container: Arc::new(Container::new()),
        lane,
        model_catalog: catalog,
        cwd: dir.path().into(),
        resources: Arc::new(Mutex::new(Default::default())),
        reload_context,
        session: session.clone(),
    };
    state
        .event_drain
        .session_changing
        .store(true, std::sync::atomic::Ordering::Release);
    ContextCommand.execute(&ctx, "");
    tokio::task::yield_now().await;
    assert_eq!(
        chat.child_count(),
        0,
        "commands during transitions are discarded"
    );
    state
        .event_drain
        .session_changing
        .store(false, std::sync::atomic::Ordering::Release);
    ContextCommand.execute(&ctx, "");
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while chat.child_count() == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(
        rx.try_recv().is_err(),
        "must not queue cumulative ShowUsage"
    );
    assert_eq!(chat.child_count(), 1);
    let text = chat.render(160).join("\n");
    assert!(text.contains("Largest entries"));
    assert!(!text.contains("private contents"));
    for width in 0..=160 {
        assert!(chat
            .render(width)
            .iter()
            .all(|line| ice_tui::visible_width(line) <= width));
    }
    assert_eq!(session.harness().get_tip_id().await.unwrap(), tip);
    assert_eq!(
        session
            .harness()
            .find_entries(&EntryQuery::default())
            .await
            .unwrap(),
        before
    );
}

#[tokio::test]
async fn changed_leaf_or_replaced_backing_rejects_snapshot_even_with_same_session_id() {
    let dir = tempfile::tempdir().unwrap();
    let harness = AgentHarness::create(Default::default()).await.unwrap();
    let lane = harness.lane("main");
    let root = lane.append_message(user("root")).await.unwrap();
    let session = crate::agent_session::AgentSession::new(harness, vec![], vec![], dir.path());
    let fence = Fence::capture(&session, &lane).await.unwrap();
    let prepared = collect_at(&session, &lane, &fence).await.unwrap();
    assert_eq!(prepared.categories[4], 1);
    lane.append_message(user("next")).await.unwrap();
    assert!(
        !fence.current(&session, &lane).await,
        "changed leaf rejects publication"
    );
    assert!(collect_at(&session, &lane, &fence).await.is_err());
    lane.navigate_tree(Some(&root), false, None, None)
        .await
        .unwrap();
    assert!(
        fence.current(&session, &lane).await,
        "same active branch can be recollected"
    );
    let replacement = AgentHarness::create(Default::default()).await.unwrap();
    assert_eq!(
        replacement.session().storage().metadata().id,
        fence.storage.metadata().id
    );
    session
        .harness()
        .session()
        .set_storage(replacement.session().storage());
    assert!(
        !fence.current(&session, &lane).await,
        "new backing rejects publication"
    );
    assert!(collect_at(&session, &lane, &fence).await.is_err());
}

#[test]
fn loaded_prompt_attribution_and_skill_listing_preserve_estimates() {
    let loaded = "base\n<project_context><project_instructions path=\"missing.md\">loaded</project_instructions></project_context>\nappend";
    let mut snapshot = Snapshot::default();
    snapshot.prompt(Some(loaded), &[skill()]);
    assert_eq!(snapshot.categories[0], byte_tokens("base\n\nappend".len()));
    let mut hidden = skill();
    hidden.disable_model_invocation = Some(true);
    let mut without_skills = Snapshot::default();
    without_skills.prompt(Some(loaded), &[hidden]);
    assert_eq!(without_skills.categories[2], 0);
    assert!(snapshot.categories[1] > 0);
    assert!(snapshot.categories[2] > 0);
    let text = snapshot.render(160).join("\n");
    assert!(!text.contains("missing.md"));
    assert!(!text.contains("description"));
    assert!(!text.contains("bodybody"));
}

fn skill() -> ice_harness::types::Skill {
    ice_harness::types::Skill {
        name: "sample".into(),
        description: "description".into(),
        content: "body".repeat(10000),
        file_path: "/not-read/SKILL.md".into(),
        disable_model_invocation: None,
    }
}

#[tokio::test]
async fn public_collection_tracks_active_fork_tools_skills_and_loaded_prompt_without_mutating() {
    use ice_harness::types::{AgentHarnessOptions, AgentHarnessResources, HarnessTool};
    let dir = tempfile::tempdir().unwrap();
    let instruction_path = dir.path().join("AGENTS.md");
    std::fs::write(&instruction_path, "changed on disk".repeat(1000)).unwrap();
    let loaded = format!("base\n<project_context>\n<project_instructions path=\"{}\">loaded</project_instructions>\n</project_context>", instruction_path.display());
    let env = Arc::new(ice_tools::OsExecutionEnv::with_cwd(dir.path().into()));
    let context = ice_tools::ExecutionToolContext::new(env.clone(), Some(env));
    let read = HarnessTool::new(ice_tools::create_read_tool(&context, None));
    let write = HarnessTool::new(ice_tools::create_write_tool(&context));
    let harness = AgentHarness::create(AgentHarnessOptions {
        system_prompt: Some(loaded.clone()),
        tools: vec![read.clone(), write],
        active_tool_names: vec!["read".into()],
        resources: AgentHarnessResources {
            skills: Some(vec![skill()]),
            ..Default::default()
        },
        ..Default::default()
    })
    .await
    .unwrap();
    let lane = harness.lane("main");
    let root = lane.append_message(user("root")).await.unwrap();
    lane.append_message(user(&"unrelated fork".repeat(1000)))
        .await
        .unwrap();
    lane.navigate_tree(Some(&root), false, None, None)
        .await
        .unwrap();
    lane.append_message(user("next")).await.unwrap();
    let session = crate::agent_session::AgentSession::new(harness, vec![], vec![], dir.path());
    let tip = session.harness().get_tip_id().await.unwrap();
    let entries = session
        .harness()
        .find_entries(&EntryQuery::default())
        .await
        .unwrap();
    let snapshot = collect(&session, &lane).await.unwrap();
    assert_eq!(snapshot.categories[4], 2, "unrelated fork excluded");
    assert_eq!(
        snapshot.categories[2],
        byte_tokens(
            (ice_harness::skills::format_skills_for_system_prompt(&[skill()]).len() + 2) as usize
        )
    );
    assert!(snapshot.categories[2] < 1000, "skill body not counted");
    assert_eq!(
        snapshot.categories[3],
        byte_tokens(serde_json::to_string(&read.tool.schema()).unwrap().len())
    );
    assert!(
        snapshot.categories[1] < 200,
        "loaded instructions, not changed disk contents"
    );
    assert!(snapshot
        .items
        .iter()
        .any(|item| item.0 == "loaded project instructions #1"));
    assert_eq!(session.harness().get_tip_id().await.unwrap(), tip);
    assert_eq!(
        session
            .harness()
            .find_entries(&EntryQuery::default())
            .await
            .unwrap(),
        entries
    );
    let before = snapshot.categories[2];
    lane.set_active_tools(vec!["write".into()]).await.unwrap();
    let without_read = collect(&session, &lane).await.unwrap();
    assert!(before > 0);
    assert_eq!(without_read.categories[2], 0);
    assert!(!without_read
        .items
        .iter()
        .any(|item| item.0 == "read schema"));
    lane.set_active_tools(vec![]).await.unwrap();
    let all_tools = collect(&session, &lane).await.unwrap();
    assert!(all_tools.items.iter().any(|item| item.0 == "read schema"));
    assert!(all_tools.items.iter().any(|item| item.0 == "write schema"));
    // Keep the harness's current empty-selection skill-gate semantics.
    assert_eq!(all_tools.categories[2], 0);
}
