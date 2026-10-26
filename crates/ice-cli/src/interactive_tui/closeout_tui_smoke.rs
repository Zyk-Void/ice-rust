//! Reproducible terminal exercise. Runs only with an explicit --ignored request.
//! All state, exports and the gh transport stub live under one temporary root.

use super::*;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::time::{Duration, Instant};

const TEST: &str = "interactive_tui::closeout_tui_smoke::faux_terminal_closeout";
const ROOT: &str = "ICERS_CLOSEOUT_TUI_ROOT";
const PHASE: &str = "ICERS_CLOSEOUT_TUI_PHASE";

#[test]
#[ignore = "explicit faux-provider PTY exercise; requires the local script utility"]
fn faux_terminal_closeout() {
    if let Ok(phase) = std::env::var(PHASE) {
        let root = PathBuf::from(std::env::var_os(ROOT).unwrap());
        assert!(Command::new("stty")
            .args(["rows", "40", "cols", "120"])
            .status()
            .unwrap()
            .success());
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap()
            .block_on(run_child(&root, &phase));
        return;
    }

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("work")).unwrap();
    std::fs::create_dir_all(root.join("bin")).unwrap();
    std::fs::create_dir_all(root.join("config")).unwrap();
    let stub = root.join("bin/gh");
    std::fs::write(
        &stub,
        "#!/bin/sh\ncat > \"$ICERS_CLOSEOUT_TUI_ROOT/share.md\"\nprintf '%s\\n' 'https://example.invalid/stub-gist'\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o700)).unwrap();
    }

    let mut terminal = TerminalExercise::spawn(root, "ui");
    terminal.wait_for("CLOSEOUT_ACTIVE_BRANCH");
    terminal.line("/share");
    terminal.wait_for("sharing does not redact anything");
    assert!(!root.join("share.md").exists());
    println!("OBSERVED /share: confirmation text; transport was not invoked.");

    terminal.line("/share confirm");
    terminal.wait_for("https://example.invalid/stub-gist");
    let shared = std::fs::read_to_string(root.join("share.md")).unwrap();
    assert!(shared.contains("CLOSEOUT_ACTIVE_BRANCH"));
    println!("OBSERVED /share confirm: temporary gh stub received the transcript.");

    terminal.line("/export native");
    terminal.wait_for("Exported active branch as a native session");
    let native = find_file(&root.join("work"), ".native.jsonl").unwrap();
    let native_text = std::fs::read_to_string(&native).unwrap();
    let header: serde_json::Value =
        serde_json::from_str(native_text.lines().next().unwrap()).unwrap();
    assert_eq!(header["version"], 4);
    assert!(native_text.contains("CLOSEOUT_ACTIVE_BRANCH"));
    println!("OBSERVED /export native: a v4 JSONL file was created.");

    terminal.line("/export md");
    terminal.wait_for("Exported session to");
    let markdown = find_file(&root.join("work"), ".md").unwrap();
    let before = std::fs::read(&markdown).unwrap();
    terminal.line("/export md");
    terminal.wait_for("Could not write export");
    assert_eq!(std::fs::read(&markdown).unwrap(), before);
    println!("OBSERVED repeated /export md: overwrite refused; bytes unchanged.");

    terminal.line("/usage");
    terminal.wait_for("All history, including abandoned branches");
    terminal.wait_for("active context (estimated)");
    println!("OBSERVED /usage: all-history billing and estimated active context labels.");

    terminal.line("SMOKE_SLOW_PROMPT");
    std::thread::sleep(Duration::from_millis(700));
    terminal.send("\x1b[Z");
    terminal.wait_for("Could not change thinking level");
    println!("OBSERVED thinking shortcut: busy-run failure was shown.");

    terminal.line("UNQUEUE_SENTINEL");
    std::thread::sleep(Duration::from_millis(150));
    terminal.line("/unqueue");
    terminal.wait_for("Moved queued message to editor");
    println!("OBSERVED /unqueue: selected pending message returned to the editor.");
    terminal.send("\x15");
    terminal.send("\x1b");
    std::thread::sleep(Duration::from_millis(500));
    terminal.line("/exit");
    terminal.finish();

    let session_path = find_file(&root.join("sessions"), ".jsonl").unwrap();
    std::fs::write(
        session_path.parent().unwrap().join("malformed.jsonl"),
        "{bad header}\n",
    )
    .unwrap();
    let mut picker = TerminalExercise::spawn(root, "picker");
    picker.wait_for("Resume session");
    picker.wait_for("malformed.jsonl");
    picker.wait_for("CLOSEOUT_ACTIVE_BRANCH");
    picker.send("\x1b");
    picker.finish();
    println!("OBSERVED --session-dir with -r: custom-root session offered; malformed path reported; Esc cancelled.");
}

fn find_file(directory: &Path, suffix: &str) -> Option<PathBuf> {
    for entry in std::fs::read_dir(directory).ok()? {
        let path = entry.ok()?.path();
        if path.is_dir() {
            if let Some(found) = find_file(&path, suffix) {
                return Some(found);
            }
        } else if path.to_string_lossy().ends_with(suffix) {
            return Some(path);
        }
    }
    None
}

struct TerminalExercise {
    child: Child,
    input: ChildStdin,
    output: Arc<Mutex<Vec<u8>>>,
    reader: Option<std::thread::JoinHandle<()>>,
    cursor_queries: usize,
}

impl TerminalExercise {
    fn spawn(root: &Path, phase: &str) -> Self {
        let executable = std::env::current_exe().unwrap();
        let quoted = format!(
            "'{}'",
            executable.display().to_string().replace('\'', "'\\''")
        );
        let command = format!("{quoted} --exact {TEST} --ignored --nocapture");
        let mut child = Command::new("script")
            .args(["-qefc", &command, "/dev/null"])
            .env(ROOT, root)
            .env(PHASE, phase)
            .env(crate::config::CONFIG_DIR_ENV, root.join("config"))
            .env("ICERS_OFFLINE", "1")
            .env("TERM", "xterm-256color")
            .env("COLUMNS", "120")
            .env("LINES", "40")
            .env(
                "PATH",
                format!("{}:/usr/bin:/bin", root.join("bin").display()),
            )
            .current_dir(root.join("work"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("script utility must be installed for the terminal exercise");
        let input = child.stdin.take().unwrap();
        let mut stdout = child.stdout.take().unwrap();
        let output = Arc::new(Mutex::new(Vec::new()));
        let captured = output.clone();
        let reader = std::thread::spawn(move || {
            let mut buffer = [0u8; 8192];
            while let Ok(count) = stdout.read(&mut buffer) {
                if count == 0 {
                    break;
                }
                captured.lock().unwrap().extend_from_slice(&buffer[..count]);
            }
        });
        Self {
            child,
            input,
            output,
            reader: Some(reader),
            cursor_queries: 0,
        }
    }

    fn text(&self) -> String {
        let output = self.output.lock().unwrap();
        ice_tui::strip_ansi(&String::from_utf8_lossy(&output))
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn wait_for(&mut self, expected: &str) {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let raw = String::from_utf8_lossy(&self.output.lock().unwrap()).into_owned();
            let queries = raw.matches("\x1b[6n").count();
            if queries > self.cursor_queries {
                self.cursor_queries = queries;
                self.send("\x1b[1;1R");
            }
            let text = self.text();
            let compact: String = text.chars().filter(|ch| !ch.is_whitespace()).collect();
            let expected_compact: String =
                expected.chars().filter(|ch| !ch.is_whitespace()).collect();
            if compact.contains(&expected_compact) {
                return;
            }
            assert!(
                Instant::now() < deadline && self.child.try_wait().unwrap().is_none(),
                "terminal did not show {expected:?}; output: {text}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn send(&mut self, input: &str) {
        self.input.write_all(input.as_bytes()).unwrap();
        self.input.flush().unwrap();
    }

    fn line(&mut self, input: &str) {
        self.send(&format!("\x1b[200~{input}\x1b[201~"));
        std::thread::sleep(Duration::from_millis(30));
        self.send("\r");
    }

    fn finish(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                if let Some(reader) = self.reader.take() {
                    reader.join().unwrap();
                }
                assert!(status.success(), "terminal child failed: {}", self.text());
                return;
            }
            assert!(
                Instant::now() < deadline,
                "terminal child did not exit: {}",
                self.text()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for TerminalExercise {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

async fn run_child(root: &Path, phase: &str) {
    let cwd = root.join("work");
    let mut args = crate::args::parse_args(&[
        "--session-dir".into(),
        root.join("sessions").to_string_lossy().into_owned(),
        "-r".into(),
    ]);
    assert!(args.errors.is_empty());
    crate::session::configure_session_dir(&args);
    if phase == "picker" {
        assert!(crate::resume_picker::select(&cwd).await.unwrap().is_none());
        return;
    }

    let provider = ice_ai::providers::faux::FauxProvider::new(
        ice_ai::providers::faux::FauxScript::new()
            .with_text("SMOKE_STREAMING still working through a long terminal response")
            .with_tokens_per_second(1.0),
    );
    let mut model = provider.default_model().clone();
    model.context_window = 100_000;
    model.reasoning = true;
    let (broadcast, event_rx) = ice_agent::events::BroadcastEmitter::new(256);
    let broadcast: Arc<dyn ice_agent::AgentEmitter> = Arc::new(broadcast);
    let harness = AgentHarness::create(ice_harness::types::AgentHarnessOptions {
        model: model.clone(),
        models: vec![provider.clone()],
        agent_emitter: Some(broadcast.clone()),
        ..Default::default()
    })
    .await
    .unwrap();
    harness
        .session()
        .append_message(AgentMessage::User(UserMessage::new(
            "CLOSEOUT_ACTIVE_BRANCH",
            1,
        )))
        .await
        .unwrap();
    let session = crate::session::fork_session_storage(&harness, &cwd.to_string_lossy())
        .await
        .unwrap();
    harness.set_session(session).await.unwrap();
    let id = harness.session().storage().metadata().id;
    args.resume = false;
    args.session = Some(id.clone());
    args.no_extensions = true;
    args.offline = true;
    let context = ice_extensions::ToolCallContext::new(cwd.to_string_lossy().into_owned());
    context.set_session_id(id);
    let runtime = tokio::runtime::Handle::current();
    let (host, cell) = crate::extensions_actions::HarnessActionHost::new_empty(
        vec![model.clone()],
        cwd.clone(),
        runtime.clone(),
        Default::default(),
        context.clone(),
    );
    crate::extensions_actions::HarnessActionHost::set_harness(&cell, Arc::new(harness.clone()));
    let mailbox = ice_extensions::ReloadMailbox::new();
    let ui_dialog = ice_extensions::UiDialogMailbox::new();
    let ext_status = ice_extensions::ExtensionStatusMailbox::new();
    let editor_text = ice_extensions::EditorTextMailbox::new();
    let bridge = ice_extensions::ActionBridge::with_reload_and_ui(
        runtime.clone(),
        Arc::new(host),
        ice_extensions::reload_callback_from_mailbox(mailbox.clone()),
        ui_dialog.clone(),
        ext_status.clone(),
        editor_text.clone(),
    );
    let reload = crate::session::ReloadContext {
        shells: crate::background_shells::SessionShells::new(&Default::default()),
        mcp_tools: crate::mcp::McpManager::new(
            &harness,
            &args,
            &cwd,
            cwd.join("agent"),
            false,
            Arc::new(Mutex::new(Default::default())),
        ),
        extension_session: Arc::new(Mutex::new(ice_extensions::ExtensionSession::none())),
        tool_context: context,
        action_bridge: Arc::new(Mutex::new(Some(bridge))),
        ext_status,
        editor_text,
        catalog: Arc::new(std::sync::RwLock::new(vec![model.clone()])),
        providers: Arc::new(std::sync::RwLock::new(vec![provider])),
        runtime,
        cwd,
        project_trusted: false,
        args: args.clone(),
        resolved_model: model.clone(),
        broadcast,
        mailbox,
        ui_dialog,
        dev_extension: None,
    };
    assert_eq!(
        interactive_tui(
            &harness,
            Some(event_rx),
            &args,
            vec![model],
            None,
            &[],
            Vec::new(),
            None,
            true,
            &reload
        )
        .await,
        0
    );
}
