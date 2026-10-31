//! Mirrors ice `test/shells-panel.test.ts`, plus the description-led labels,
//! the transcript completion line and output cleaning.

use std::sync::atomic::AtomicUsize;

use crossterm::event::{KeyCode, KeyModifiers};
use ice_agent::agent_tool::AgentTool;
use ice_tools::bash_background::BashBackgroundCompletion;
use ice_tools::{BashToolOptions, ExecutionToolContext, InMemoryExecutionEnv, ShellScript};
use tokio_util::sync::CancellationToken;

use super::*;

struct Shells {
    env: Arc<InMemoryExecutionEnv>,
    registry: Arc<BashBackgroundRegistry>,
    bash: Arc<dyn AgentTool>,
    completions: Arc<Mutex<Vec<BashBackgroundCompletion>>>,
}

impl Shells {
    fn new() -> Self {
        let env = Arc::new(InMemoryExecutionEnv::with_cwd("/w".into()));
        let env_dyn: Arc<dyn ice_tools::ExecutionEnv> = env.clone();
        let registry = BashBackgroundRegistry::new();
        let completions = Arc::new(Mutex::new(Vec::new()));
        let sink = completions.clone();
        registry.on_complete(move |c| sink.lock().unwrap().push(c.clone()));
        let bash = ice_tools::create_bash_tool(
            &ExecutionToolContext::new(env_dyn, None),
            Some(BashToolOptions {
                background: Some(registry.clone()),
                ..Default::default()
            }),
        );
        Self {
            env,
            registry,
            bash,
            completions,
        }
    }

    /// Start `command` in the background; it prints `output` and keeps running.
    async fn start(&self, command: &str, output: &str, description: Option<&str>) -> String {
        self.start_script(
            command,
            ShellScript::long_running(output, 60_000),
            description,
        )
        .await
    }

    async fn start_script(
        &self,
        command: &str,
        script: ShellScript,
        description: Option<&str>,
    ) -> String {
        self.env.register_shell(command, script).await;
        let mut params = serde_json::json!({ "command": command, "run_in_background": true });
        if let Some(description) = description {
            params["description"] = description.into();
        }
        let result = self
            .bash
            .execute("c", params, CancellationToken::new(), Arc::new(|_| ()))
            .await
            .expect("background launch");
        result.details["background_task_id"]
            .as_str()
            .unwrap()
            .to_string()
    }

    /// Wait for the shell's initial output to land in the registry.
    async fn wait_output(&self, id: &str, text: &str) {
        for _ in 0..200 {
            if self
                .registry
                .get(id)
                .is_some_and(|shell| shell.output.contains(text))
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("no output {text:?} from {id}");
    }
}

struct Panel {
    panel: Arc<ShellsPanel>,
    closed: Arc<AtomicUsize>,
    renders: Arc<AtomicUsize>,
}

fn panel(registry: &Arc<BashBackgroundRegistry>, now: Option<i64>) -> Panel {
    let closed = Arc::new(AtomicUsize::new(0));
    let renders = Arc::new(AtomicUsize::new(0));
    let (close, render) = (closed.clone(), renders.clone());
    let panel = ShellsPanel::new(
        registry.clone(),
        Arc::new(Keybindings::new()),
        Arc::new(move || {
            close.fetch_add(1, Ordering::Relaxed);
        }),
        Arc::new(move || {
            render.fetch_add(1, Ordering::Relaxed);
        }),
        ShellsPanelOptions {
            now: now.map(|now| Arc::new(move || now) as Arc<dyn Fn() -> i64 + Send + Sync>),
            no_tick: true,
        },
    );
    Panel {
        panel,
        closed,
        renders,
    }
}

fn press(panel: &ShellsPanel, code: KeyCode) {
    panel.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
}

fn strip(lines: Vec<String>) -> Vec<String> {
    lines
        .iter()
        .map(|line| ice_tui::strip_ansi(line).trim_end().to_string())
        .collect()
}

#[tokio::test]
async fn lists_running_shells_and_opens_details_with_a_clean_output_tail() {
    let shells = Shells::new();
    let dev = shells
        .start(
            "npm run dev",
            "\x1b[32mready\x1b[0m on http://localhost:5173\nGET / 200\n",
            None,
        )
        .await;
    let test = shells.start("./test.sh", "", None).await;
    shells.wait_output(&dev, "GET / 200").await;
    let started = shells.registry.get(&dev).unwrap().started_at_ms;
    let p = panel(&shells.registry, Some(started + 65_000));

    let list = strip(p.panel.render(100));
    assert_eq!(list[1], "Shells  2 running");
    // The newest running shell starts selected; rows lead with the command
    // and show the id dimmed at the end.
    let selected = list.iter().find(|line| line.starts_with('›')).unwrap();
    assert!(
        selected.contains("./test.sh") && selected.ends_with(&test),
        "{list:?}"
    );
    assert!(
        list.iter()
            .any(|line| line.contains("1m 5s  bash  npm run dev  ") && line.ends_with(&dev)),
        "{list:?}"
    );

    press(&p.panel, KeyCode::Up);
    press(&p.panel, KeyCode::Enter);
    let details = strip(p.panel.render(100));
    assert_eq!(details[1], format!("bash  npm run dev  {dev}"));
    assert!(details.contains(&"Status:   running · started by the agent".to_string()));
    assert!(details.contains(&"Runtime:  1m 5s".to_string()));
    assert!(details.contains(&"Command:  npm run dev".to_string()));
    assert!(details.contains(&format!("Id:       {dev}")));
    // Escape sequences are stripped. (Carriage-return redraws are covered by
    // the `plain_output` test: the registry's capture already drops CRs.)
    assert!(
        details.contains(&"  ready on http://localhost:5173".to_string()),
        "{details:?}"
    );
    assert!(details.contains(&"  GET / 200".to_string()), "{details:?}");

    press(&p.panel, KeyCode::Left);
    assert_eq!(strip(p.panel.render(100))[1], "Shells  2 running");
    press(&p.panel, KeyCode::Esc);
    assert_eq!(p.closed.load(Ordering::Relaxed), 1);
    shells.registry.dispose().await;
}

#[tokio::test]
async fn stops_the_selected_shell_telling_the_model_and_follows_a_shell() {
    let shells = Shells::new();
    let dev = shells.start("npm run dev", "ready\n", None).await;
    let test = shells.start("./test.sh", "", None).await;
    let p = panel(&shells.registry, None);

    press(&p.panel, KeyCode::Char('f'));
    let follow = strip(p.panel.render(100));
    assert!(
        follow[1].starts_with(&format!("Following ./test.sh  live  {test}")),
        "{follow:?}"
    );
    assert!(follow.contains(&"No output yet.".to_string()));

    press(&p.panel, KeyCode::Char('x'));
    let mut stopped = Vec::new();
    for _ in 0..200 {
        stopped = strip(p.panel.render(100));
        if stopped.contains(&format!("Stopped {test}.")) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(stopped.contains(&format!("Stopped {test}.")), "{stopped:?}");
    let completions = shells.completions.lock().unwrap().clone();
    assert_eq!(completions.len(), 1);
    assert_eq!(completions[0].task_id, test);
    assert!(completions[0].stopped_by_user);
    assert_eq!(completions[0].text, "Stopped by the user.");
    // A stopped shell has no stop hint, and stays in view until the user
    // goes back to the list, which only shows running shells.
    assert!(!stopped.last().unwrap().contains("x stop"), "{stopped:?}");
    assert!(
        stopped[1].starts_with("Following ./test.sh  killed"),
        "{stopped:?}"
    );
    press(&p.panel, KeyCode::Left);
    press(&p.panel, KeyCode::Left);
    let list = strip(p.panel.render(100));
    assert_eq!(list[1], "Shells  1 running");
    assert!(!list.join("\n").contains(&test));
    press(&p.panel, KeyCode::Down);
    press(&p.panel, KeyCode::Right);
    assert_eq!(
        strip(p.panel.render(100))[1],
        format!("bash  npm run dev  {dev}")
    );
    shells.registry.dispose().await;
}

#[tokio::test]
async fn stop_does_not_target_another_shell_when_selection_disappears() {
    let shells = Shells::new();
    let first = shells.start("first", "", None).await;
    let selected = shells.start("second", "", None).await;
    let p = panel(&shells.registry, None);
    p.panel.render(100);
    assert_eq!(
        p.panel.state.lock().unwrap().selected.as_deref(),
        Some(selected.as_str())
    );
    shells.registry.kill(&selected, true).await.unwrap();

    press(&p.panel, KeyCode::Char('x'));
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        shells.registry.get(&first).unwrap().status,
        BackgroundShellStatus::Running
    );
    shells.registry.dispose().await;
}

#[tokio::test]
async fn untrusted_metadata_cannot_emit_terminal_controls() {
    let shells = Shells::new();
    let malicious = "hello\x1b]52;c;SECRET\x07world";
    shells.start("command", "", Some(malicious)).await;
    let p = panel(&shells.registry, None);
    let lines = p.panel.render(100).join("\n");
    assert!(!lines.contains("\x1b]52;"), "{lines:?}");
    assert!(!lines.contains('\x07'), "{lines:?}");
    let completion = completion_lines(&serde_json::json!({"description": malicious})).join("\n");
    assert!(!completion.contains("\x1b]52;"), "{completion:?}");
    assert!(!completion.contains('\x07'), "{completion:?}");
    shells.registry.dispose().await;
}

#[tokio::test]
async fn opens_straight_on_a_single_shell_and_stays_within_the_width() {
    let shells = Shells::new();
    let command = format!("printf '%s' {}", "x".repeat(300));
    let id = shells.start(&command, "", None).await;
    let p = panel(&shells.registry, None);
    let lines = p.panel.render(40);
    assert!(strip(lines.clone())[1].starts_with("bash  "), "{lines:?}");
    assert!(lines.iter().any(|line| line.contains(&id)));
    for line in &lines {
        assert!(visible_width(line) <= 40, "{line:?}");
    }
    shells.registry.dispose().await;
}

#[tokio::test]
async fn leaves_finished_shells_out_of_the_list() {
    let shells = Shells::new();
    let id = shells
        .start_script("true", ShellScript::success("done\n"), None)
        .await;
    for _ in 0..200 {
        if shells.registry.get(&id).unwrap().status != BackgroundShellStatus::Running {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let p = panel(&shells.registry, None);
    let lines = strip(p.panel.render(100));
    assert_eq!(lines[1], "Shells  no running shells");
    assert!(!lines.join("\n").contains(&id));
    shells.registry.dispose().await;
}

#[tokio::test]
async fn repaints_on_shell_changes_until_disposed() {
    let shells = Shells::new();
    let p = panel(&shells.registry, None);
    shells.start("tail -f log", "line\n", None).await;
    assert!(p.renders.load(Ordering::Relaxed) > 0);
    // Closing unsubscribes: later changes no longer repaint the panel.
    press(&p.panel, KeyCode::Esc);
    let before = p.renders.load(Ordering::Relaxed);
    shells.start("sleep 100", "", None).await;
    assert_eq!(p.renders.load(Ordering::Relaxed), before);
    shells.registry.dispose().await;
}

#[tokio::test]
async fn a_description_leads_the_row_with_the_command_and_id_after_it() {
    let shells = Shells::new();
    let id = shells
        .start("npm run dev -- --port 5173", "", Some("Dev server"))
        .await;
    shells.start("./other.sh", "", None).await;
    let p = panel(&shells.registry, None);
    let list = strip(p.panel.render(120));
    let row = list
        .iter()
        .find(|line| line.contains("Dev server"))
        .unwrap();
    assert!(
        row.contains("Dev server · npm run dev -- --port 5173") && row.ends_with(&id),
        "{row}"
    );
    press(&p.panel, KeyCode::Up);
    press(&p.panel, KeyCode::Enter);
    let details = strip(p.panel.render(120));
    assert_eq!(details[1], format!("bash  Dev server  {id}"));
    assert!(details.contains(&"About:    Dev server".to_string()));
    shells.registry.dispose().await;
}

#[test]
fn completion_line_names_the_command_not_the_id() {
    let strip_one = |lines: Vec<String>| strip(lines);
    let lines = strip_one(completion_lines(&serde_json::json!({
        "task_id": "b2k7q0xyz",
        "command": "npm test\nnpm run lint",
        "description": "Run tests",
        "text": "a\nb\nc\nd\n3 passed",
        "is_error": false,
        "exit_code": 0,
        "duration_ms": 4_200,
    })));
    assert_eq!(
        lines[0],
        "✓ Background bash \"Run tests\" completed (exit code 0) · 4s"
    );
    assert_eq!(
        lines[1..],
        ["  … 2 earlier lines", "  c", "  d", "  3 passed"]
    );
    assert!(!lines.join("\n").contains("b2k7q0xyz"));

    // ice (camelCase) details, no description: the command's first line.
    let failed = strip_one(completion_lines(&serde_json::json!({
        "taskId": "b3m8r1abc",
        "command": "make\nmake install",
        "isError": true,
        "exitCode": 2,
    })));
    assert_eq!(failed, ["✗ Background bash \"make\" failed (exit code 2)"]);
    let stopped = strip_one(completion_lines(&serde_json::json!({
        "task_id": "b4n9s2def",
        "command": "sleep 100",
        "stopped_by_user": true,
        "is_error": true,
        "text": "Stopped by the user.",
    })));
    assert_eq!(
        stopped,
        [
            "· Background bash \"sleep 100\" was stopped",
            "  Stopped by the user."
        ]
    );
}

#[test]
fn completion_messages_render_as_a_status_line() {
    let chat = Arc::new(ice_tui::Container::new());
    let completion = ice_harness::messages::create_custom_message(
        ice_tools::bash_background::BASH_BACKGROUND_COMPLETED_CUSTOM_TYPE,
        ice_ai::types::UserContent::Text("Background bash b1j6p9uvw completed".into()),
        true,
        Some(serde_json::json!({ "task_id": "b1j6p9uvw", "command": "ls" })),
        0,
    );
    let ice_agent::AgentMessage::Custom(custom) = completion else {
        panic!("custom message");
    };
    assert!(add_completion_message(&chat, &custom));
    let rendered = strip(chat.render(80)).join("\n");
    assert!(
        rendered.contains("✓ Background bash \"ls\" completed"),
        "{rendered}"
    );
    assert!(!rendered.contains("b1j6p9uvw"), "{rendered}");

    let other = ice_harness::messages::create_custom_message(
        "something_else",
        ice_ai::types::UserContent::Text("x".into()),
        true,
        None,
        0,
    );
    let ice_agent::AgentMessage::Custom(other) = other else {
        panic!("custom message");
    };
    assert!(!add_completion_message(&chat, &other));
}

#[test]
fn plain_output_drops_escapes_and_redraws_but_keeps_crlf_lines() {
    assert_eq!(
        plain_output(
            "\x1b[1mbold\x1b[0m\r\nnext\x1b]8;;https://x\x07link\x1b]8;;\x1b\\\nbar 1\rbar 2\x07\n"
        ),
        "bold\nnextlink\nbar 2\n"
    );
}

#[test]
fn stopping_without_a_runtime_does_not_deadlock_a_same_thread_repaint() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (shells, id) = runtime.block_on(async {
        let shells = Shells::new();
        let id = shells.start("sleep 100", "", None).await;
        (shells, id)
    });
    // Outside the runtime the stop runs inline, and this repaint renders the
    // panel on the same thread, as a synchronous host would.
    let slot: Arc<std::sync::OnceLock<std::sync::Weak<ShellsPanel>>> = Arc::default();
    let repaint = slot.clone();
    let panel = ShellsPanel::new(
        shells.registry.clone(),
        Arc::new(Keybindings::new()),
        Arc::new(|| {}),
        Arc::new(move || {
            if let Some(panel) = repaint.get().and_then(std::sync::Weak::upgrade) {
                panel.render(80);
            }
        }),
        ShellsPanelOptions {
            now: None,
            no_tick: true,
        },
    );
    slot.set(Arc::downgrade(&panel)).ok().unwrap();
    press(&panel, KeyCode::Char('x'));
    assert!(strip(panel.render(80)).contains(&format!("Stopped {id}.")));
    runtime.block_on(shells.registry.dispose());
}

#[test]
fn tasks_is_an_alias_of_shells() {
    let registry = super::super::build_builtin_registry();
    assert_eq!(
        registry
            .find("/tasks")
            .map(|command| command.name().to_string())
            .as_deref(),
        Some("/shells")
    );
}
