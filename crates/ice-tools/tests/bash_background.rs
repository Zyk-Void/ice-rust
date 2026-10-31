//! Mirrors `packages/coding-agent/test/bash-background.test.ts` and
//! `bash-input.test.ts`: explicit background launch, foreground release,
//! incremental bash_output reads, log tail/grep, bash_kill, bash_input with
//! approval and limits, retention and disposal. Runs against
//! `InMemoryExecutionEnv` (no real shell).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use ice_agent::agent_tool::AgentTool;
use ice_agent::error::AgentError;
use ice_agent::types::{AgentToolResult, TextContentOrImage};
use ice_tools::bash_background::{
    BackgroundShellStatus, BashBackgroundCompletion, BashBackgroundRegistry, MAX_RUNNING_SHELLS,
};
use ice_tools::tools::bash_shells::{
    create_bash_shell_tools, BashInputApproval, BashShellToolOptions, ConfirmFn,
};
use ice_tools::{
    BashToolOptions, ExecutionToolContext, FileSystem, InMemoryExecutionEnv, ShellScript,
};
use tokio_util::sync::CancellationToken;

struct Harness {
    env: Arc<InMemoryExecutionEnv>,
    registry: Arc<BashBackgroundRegistry>,
    bash: Arc<dyn AgentTool>,
    output: Arc<dyn AgentTool>,
    kill: Arc<dyn AgentTool>,
    input: Arc<dyn AgentTool>,
    completions: Arc<Mutex<Vec<BashBackgroundCompletion>>>,
}

fn harness_with(bash: BashToolOptions, shells: BashShellToolOptions) -> Harness {
    let env = Arc::new(InMemoryExecutionEnv::with_cwd("/w".into()));
    let env_dyn: Arc<dyn ice_tools::ExecutionEnv> = env.clone();
    let ctx = ExecutionToolContext::new(env_dyn, None);
    let registry = BashBackgroundRegistry::new();
    let completions = Arc::new(Mutex::new(Vec::new()));
    let sink = completions.clone();
    registry.on_complete(move |c| sink.lock().unwrap().push(c.clone()));
    let bash = ice_tools::create_bash_tool(
        &ctx,
        Some(BashToolOptions {
            background: Some(registry.clone()),
            ..bash
        }),
    );
    let mut tools = create_bash_shell_tools(registry.clone(), shells).into_iter();
    Harness {
        env,
        registry,
        bash,
        output: tools.next().unwrap(),
        kill: tools.next().unwrap(),
        input: tools.next().unwrap(),
        completions,
    }
}

fn harness() -> Harness {
    harness_with(BashToolOptions::default(), BashShellToolOptions::default())
}

async fn run(
    tool: &Arc<dyn AgentTool>,
    id: &str,
    params: serde_json::Value,
) -> Result<AgentToolResult, AgentError> {
    tool.execute(id, params, CancellationToken::new(), Arc::new(|_| ()))
        .await
}

fn text(r: &AgentToolResult) -> String {
    r.content
        .iter()
        .filter_map(|c| match c {
            TextContentOrImage::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn err(e: AgentError) -> String {
    match e {
        AgentError::Tool(m) => m,
        other => format!("{other:?}"),
    }
}

async fn start(h: &Harness, command: &str, extra: serde_json::Value) -> String {
    let mut params = serde_json::json!({ "command": command, "run_in_background": true });
    if let serde_json::Value::Object(extra) = extra {
        params.as_object_mut().unwrap().extend(extra);
    }
    let r = run(&h.bash, "call-1", params).await.expect("launch ok");
    r.details["background_task_id"]
        .as_str()
        .expect("task id")
        .to_string()
}

#[tokio::test]
async fn explicit_launch_returns_at_once_and_reports_one_completion() {
    let h = harness();
    h.env
        .register_shell(
            "server",
            ShellScript {
                stdout: "done\n".into(),
                ..ShellScript::long_running("listening\n", 200)
            },
        )
        .await;
    let started = std::time::Instant::now();
    let r = run(
        &h.bash,
        "c",
        serde_json::json!({ "command": "server", "run_in_background": true, "description": "dev server" }),
    )
    .await
    .expect("launch ok");
    assert!(
        started.elapsed() < Duration::from_millis(150),
        "did not wait"
    );
    let id = r.details["background_task_id"]
        .as_str()
        .unwrap()
        .to_string();
    // `b` and eight base-36 characters, not a per-session counter.
    assert_eq!(id.len(), 9, "{id}");
    assert!(id.starts_with('b'), "{id}");
    assert!(
        id[1..]
            .chars()
            .all(|c| c.is_ascii_digit() || c.is_ascii_lowercase()),
        "{id}"
    );
    assert_eq!(r.details["background_origin"], "model");
    let log = r.details["full_output_path"]
        .as_str()
        .expect("log path")
        .to_string();
    // Every shell is named "bash"; its description tells them apart.
    assert!(
        text(&r).starts_with(&format!("Started background bash {id}: dev server")),
        "{}",
        text(&r)
    );
    assert!(text(&r).contains("bash_output"));

    // Wait for exit: the final result is reported.
    let out = run(
        &h.output,
        "o",
        serde_json::json!({ "id": id, "wait_seconds": 5 }),
    )
    .await
    .expect("read ok");
    let out = text(&out);
    assert!(out.starts_with(&format!("{id}: completed")), "{out}");
    assert!(out.contains("listening"), "{out}");
    assert!(out.contains("Final result:"), "{out}");

    let completions = h.completions.lock().unwrap().clone();
    assert_eq!(completions.len(), 1);
    assert_eq!(completions[0].task_id, id);
    assert!(!completions[0].is_error);
    assert_eq!(completions[0].exit_code, Some(0));
    assert_eq!(completions[0].description.as_deref(), Some("dev server"));
    let logged = h.env.read_text_file(&log, None).await.unwrap();
    assert_eq!(logged, "listening\ndone\n");
}

#[tokio::test]
async fn incremental_reads_return_only_new_output() {
    let h = harness();
    h.env
        .register_shell(
            "watch",
            ShellScript {
                stdout: "second\n".into(),
                ..ShellScript::long_running("first\n", 300)
            },
        )
        .await;
    let id = start(&h, "watch", serde_json::json!({})).await;
    let r = run(
        &h.output,
        "o",
        serde_json::json!({ "id": id, "wait_seconds": 2, "until": "output" }),
    )
    .await
    .unwrap();
    assert!(text(&r).contains("first"), "{}", text(&r));
    let r = run(&h.output, "o", serde_json::json!({ "id": id }))
        .await
        .unwrap();
    assert!(text(&r).contains("(no new output)"), "{}", text(&r));
    let r = run(
        &h.output,
        "o",
        serde_json::json!({ "id": id, "wait_seconds": 5 }),
    )
    .await
    .unwrap();
    let out = text(&r);
    let (new_part, final_part) = out.split_once("Final result:").expect("final block");
    assert!(new_part.contains("second"), "{out}");
    assert!(
        !new_part.contains("first"),
        "already-read output is not repeated: {out}"
    );
    assert!(final_part.contains("first\nsecond"), "{out}");
}

#[tokio::test]
async fn listing_validation_and_unknown_ids() {
    let h = harness();
    let r = run(&h.output, "o", serde_json::json!({})).await.unwrap();
    assert_eq!(text(&r), "No background shells.");
    h.env
        .register_shell("job", ShellScript::long_running("", 500))
        .await;
    let id = start(&h, "job", serde_json::json!({})).await;
    // Listing ignores other (even invalid) fields.
    let r = run(&h.output, "o", serde_json::json!({ "wait_seconds": -1 }))
        .await
        .unwrap();
    assert!(
        text(&r).starts_with(&format!("{id}: running")),
        "{}",
        text(&r)
    );
    let e = run(
        &h.output,
        "o",
        serde_json::json!({ "id": id, "wait_seconds": -1 }),
    )
    .await
    .unwrap_err();
    assert_eq!(err(e), "wait_seconds must be a non-negative number");
    let e = run(
        &h.output,
        "o",
        serde_json::json!({ "id": id, "tail_lines": 1.5 }),
    )
    .await
    .unwrap_err();
    assert_eq!(err(e), "tail_lines must be a positive integer");
    let e = run(&h.output, "o", serde_json::json!({ "id": "bash-9" }))
        .await
        .unwrap_err();
    assert_eq!(
        err(e),
        format!("Unknown background shell \"bash-9\". Known: {id}.")
    );
    let e = run(
        &h.output,
        "o",
        serde_json::json!({ "id": id, "grep": "(a+)+" }),
    )
    .await
    .unwrap_err();
    assert!(err(e).contains("quantified group"));
}

#[tokio::test]
async fn log_tail_and_grep_do_not_consume_new_output() {
    let h = harness();
    h.env
        .register_shell(
            "build",
            ShellScript::long_running("one\nerror: a\ntwo\nerror: b\nthree\n", 300),
        )
        .await;
    let id = start(&h, "build", serde_json::json!({})).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let r = run(
        &h.output,
        "o",
        serde_json::json!({ "id": id, "tail_lines": 2 }),
    )
    .await
    .unwrap();
    assert!(text(&r).ends_with("error: b\nthree"), "{}", text(&r));
    let r = run(
        &h.output,
        "o",
        serde_json::json!({ "id": id, "grep": "^error" }),
    )
    .await
    .unwrap();
    let out = text(&r);
    assert!(out.contains("2 of 2 matching lines"), "{out}");
    assert!(out.ends_with("error: a\nerror: b"), "{out}");
    // The incremental cursor did not move.
    let r = run(&h.output, "o", serde_json::json!({ "id": id }))
        .await
        .unwrap();
    assert!(text(&r).contains("one\nerror: a"), "{}", text(&r));
}

#[tokio::test]
async fn kill_records_one_terminal_state_and_no_late_completion() {
    let h = harness();
    h.env
        .register_shell("forever", ShellScript::long_running("tick\n", 10_000))
        .await;
    let id = start(&h, "forever", serde_json::json!({})).await;
    let r = run(&h.kill, "k", serde_json::json!({ "id": id }))
        .await
        .unwrap();
    assert_eq!(text(&r), format!("Stopped {id}."));
    tokio::time::sleep(Duration::from_millis(50)).await;
    let snap = h.registry.get(&id).unwrap();
    assert_eq!(snap.status, BackgroundShellStatus::Killed);
    // A tool kill does not notify, and the late settlement is ignored.
    assert!(h.completions.lock().unwrap().is_empty());
    let r = run(&h.kill, "k", serde_json::json!({ "id": id }))
        .await
        .unwrap();
    assert_eq!(text(&r), format!("{id} already killed."));
}

#[tokio::test]
async fn user_stop_notifies_with_stopped_by_user() {
    let h = harness();
    h.env
        .register_shell("forever", ShellScript::long_running("", 10_000))
        .await;
    let id = start(&h, "forever", serde_json::json!({})).await;
    h.registry.kill(&id, true).await.unwrap();
    let completions = h.completions.lock().unwrap().clone();
    assert_eq!(completions.len(), 1);
    assert!(completions[0].stopped_by_user);
    assert!(completions[0].is_error);
    assert!(completions[0].log_path.is_none());
}

#[tokio::test]
async fn the_seventeenth_explicit_launch_is_rejected() {
    let h = harness();
    h.env
        .register_shell("job", ShellScript::long_running("", 10_000))
        .await;
    for _ in 0..MAX_RUNNING_SHELLS {
        start(&h, "job", serde_json::json!({})).await;
    }
    let e = run(
        &h.bash,
        "c",
        serde_json::json!({ "command": "job", "run_in_background": true }),
    )
    .await
    .unwrap_err();
    assert!(err(e).starts_with("16 background shells are already running."));
    h.registry.dispose().await;
}

#[tokio::test]
async fn bash_input_writes_exact_text_with_approval_and_limits() {
    let asked = Arc::new(Mutex::new(Vec::<String>::new()));
    let seen = asked.clone();
    let confirm: ConfirmFn = Arc::new(move |title, body| {
        seen.lock().unwrap().push(format!("{title}\n{body}"));
        Box::pin(async { true })
    });
    let h = harness_with(
        BashToolOptions::default(),
        BashShellToolOptions {
            confirm: Some(confirm),
            ..Default::default()
        },
    );
    h.env
        .register_shell("prompt", ShellScript::long_running("Continue? ", 10_000))
        .await;
    let id = start(&h, "prompt", serde_json::json!({ "stdin": true })).await;
    let r = run(
        &h.input,
        "i",
        serde_json::json!({ "id": id, "text": "y\n" }),
    )
    .await
    .unwrap();
    assert_eq!(text(&r), format!("Sent 2 bytes to {id}."));
    assert_eq!(r.details["bytes"], 2);
    assert_eq!(h.env.stdin_written().await, b"y\n");
    let asked = asked.lock().unwrap().clone();
    assert!(
        asked[0].starts_with(&format!("Send input to {id}?")),
        "{asked:?}"
    );
    assert!(asked[0].contains("Text: \"y\\n\""), "{asked:?}");

    let e = run(&h.input, "i", serde_json::json!({ "id": id }))
        .await
        .unwrap_err();
    assert_eq!(err(e), "bash_input needs text to send or eof: true.");
    let big = "x".repeat(8193);
    let e = run(&h.input, "i", serde_json::json!({ "id": id, "text": big }))
        .await
        .unwrap_err();
    assert!(err(e).contains("at most 8192 bytes"));
    let r = run(&h.input, "i", serde_json::json!({ "id": id, "eof": true }))
        .await
        .unwrap();
    assert!(text(&r).ends_with("and closed stdin."));
    let e = run(
        &h.input,
        "i",
        serde_json::json!({ "id": id, "text": "more" }),
    )
    .await
    .unwrap_err();
    assert!(err(e).contains("has no open stdin"));
    h.registry.dispose().await;
}

#[tokio::test]
async fn bash_input_without_ui_or_with_refusal_fails() {
    let h = harness();
    h.env
        .register_shell("prompt", ShellScript::long_running("", 10_000))
        .await;
    let id = start(&h, "prompt", serde_json::json!({ "stdin": true })).await;
    let e = run(&h.input, "i", serde_json::json!({ "id": id, "text": "y" }))
        .await
        .unwrap_err();
    assert!(err(e).contains("needs the user's approval and no interactive UI"));

    let refuse: ConfirmFn = Arc::new(|_, _| Box::pin(async { false }));
    let h2 = harness_with(
        BashToolOptions::default(),
        BashShellToolOptions {
            confirm: Some(refuse),
            ..Default::default()
        },
    );
    h2.env
        .register_shell("prompt", ShellScript::long_running("", 10_000))
        .await;
    let id2 = start(&h2, "prompt", serde_json::json!({ "stdin": true })).await;
    let e = run(
        &h2.input,
        "i",
        serde_json::json!({ "id": id2, "text": "y" }),
    )
    .await
    .unwrap_err();
    assert_eq!(err(e), "The user declined to send this input.");

    let allow = harness_with(
        BashToolOptions::default(),
        BashShellToolOptions {
            input_approval: BashInputApproval::Allow,
            ..Default::default()
        },
    );
    allow
        .env
        .register_shell("prompt", ShellScript::long_running("", 10_000))
        .await;
    let id3 = start(&allow, "prompt", serde_json::json!({ "stdin": true })).await;
    run(
        &allow.input,
        "i",
        serde_json::json!({ "id": id3, "text": "ok" }),
    )
    .await
    .expect("allowed without a UI");
    h.registry.dispose().await;
    h2.registry.dispose().await;
    allow.registry.dispose().await;
}

#[tokio::test]
async fn user_release_moves_a_running_foreground_call_without_restarting_it() {
    let h = harness();
    h.env
        .register_shell(
            "slow",
            ShellScript {
                stdout: "finished\n".into(),
                ..ShellScript::long_running("working\n", 400)
            },
        )
        .await;
    let bash = h.bash.clone();
    let call = tokio::spawn(async move {
        bash.execute(
            "fg-1",
            serde_json::json!({ "command": "slow" }),
            CancellationToken::new(),
            Arc::new(|_| ()),
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(h.registry.has_releasable());
    assert_eq!(h.registry.release_all(), 1);
    let r = call.await.unwrap().expect("released ok");
    let id = r.details["background_task_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(r.details["background_origin"], "user");
    let shown = text(&r);
    assert!(
        shown.starts_with("The user moved this command to the background."),
        "{shown}"
    );
    assert!(shown.contains("working"), "{shown}");
    // The same process finishes; output already returned is not repeated.
    let out = run(
        &h.output,
        "o",
        serde_json::json!({ "id": id, "wait_seconds": 5 }),
    )
    .await
    .unwrap();
    let out = text(&out);
    assert!(out.contains(&format!("{id}: completed")), "{out}");
    let (new_part, final_part) = out.split_once("Final result:").expect("final block");
    assert!(new_part.contains("finished"), "{out}");
    assert!(
        !new_part.contains("working"),
        "returned output is not repeated: {out}"
    );
    // One process: its full result spans before and after the release.
    assert!(final_part.contains("working\nfinished"), "{out}");
    assert!(!h.registry.has_releasable());
}

#[tokio::test]
async fn the_default_timeout_moves_a_foreground_call_to_the_background() {
    let h = harness_with(
        BashToolOptions {
            default_timeout: Some(0.2),
            ..Default::default()
        },
        BashShellToolOptions::default(),
    );
    h.env
        .register_shell(
            "slow",
            ShellScript {
                stdout: "finished\n".into(),
                ..ShellScript::long_running("working\n", 500)
            },
        )
        .await;
    // Where shells can run in the background, the default moves the call
    // there instead of stopping it.
    let r = run(&h.bash, "fg", serde_json::json!({ "command": "slow" }))
        .await
        .unwrap();
    assert_eq!(r.details["background_origin"], "model");
    assert!(
        text(&r).starts_with("Still running after 0s, so this command moved to the background."),
        "{}",
        text(&r)
    );
    let id = r.details["background_task_id"]
        .as_str()
        .unwrap()
        .to_string();
    let out = text(
        &run(
            &h.output,
            "o",
            serde_json::json!({ "id": id, "wait_seconds": 5 }),
        )
        .await
        .unwrap(),
    );
    assert!(out.contains(": completed"), "{out}");
    assert!(out.contains("finished"), "{out}");

    // Without background shells it stops the call, as a timeout.
    let env: Arc<dyn ice_tools::ExecutionEnv> = h.env.clone();
    let plain = ice_tools::create_bash_tool(
        &ExecutionToolContext::new(env, None),
        Some(BashToolOptions {
            default_timeout: Some(0.2),
            ..Default::default()
        }),
    );
    let e = err(run(&plain, "fg2", serde_json::json!({ "command": "slow" }))
        .await
        .unwrap_err());
    assert!(
        e.ends_with("working\n\n\nCommand timed out after 0.2 seconds"),
        "{e:?}"
    );

    // A background shell is not subject to the default, but its own
    // timeout stops it, and the launch says so.
    let id = start(&h, "slow", serde_json::json!({})).await;
    let out = text(
        &run(
            &h.output,
            "o2",
            serde_json::json!({ "id": id, "wait_seconds": 5 }),
        )
        .await
        .unwrap(),
    );
    assert!(out.contains(": completed"), "{out}");
    let r = run(
        &h.bash,
        "bg",
        serde_json::json!({ "command": "slow", "run_in_background": true, "timeout": 0.1 }),
    )
    .await
    .unwrap();
    assert!(
        text(&r).contains("Its timeout stops it 0.1 seconds after it started."),
        "{}",
        text(&r)
    );
    let id = r.details["background_task_id"]
        .as_str()
        .unwrap()
        .to_string();
    let out = text(
        &run(
            &h.output,
            "o3",
            serde_json::json!({ "id": id, "wait_seconds": 5 }),
        )
        .await
        .unwrap(),
    );
    assert!(out.contains(": failed"), "{out}");
    assert!(
        out.ends_with("Command timed out after 0.1 seconds"),
        "{out}"
    );
    h.registry.dispose().await;
}

#[tokio::test]
async fn a_call_moved_to_the_background_outlives_the_default_timeout() {
    let h = harness_with(
        BashToolOptions {
            default_timeout: Some(0.3),
            ..Default::default()
        },
        BashShellToolOptions::default(),
    );
    h.env
        .register_shell("slow", ShellScript::long_running("working\n", 600))
        .await;
    let bash = h.bash.clone();
    let call = tokio::spawn(async move {
        bash.execute(
            "fg",
            serde_json::json!({ "command": "slow" }),
            CancellationToken::new(),
            Arc::new(|_| ()),
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(h.registry.release_all(), 1);
    let released = call.await.unwrap().unwrap();
    assert!(
        !text(&released).contains("Its timeout"),
        "{}",
        text(&released)
    );
    let id = released.details["background_task_id"].as_str().unwrap();
    let out = run(
        &h.output,
        "o",
        serde_json::json!({ "id": id, "wait_seconds": 5 }),
    )
    .await
    .unwrap();
    assert!(
        text(&out).starts_with(&format!("{id}: completed")),
        "{}",
        text(&out)
    );
}

#[tokio::test]
async fn reading_the_final_result_makes_the_completion_notice_moot() {
    let h = harness();
    // Whether a host would still send each completion notice.
    let decisions = Arc::new(Mutex::new(Vec::new()));
    let (sink, registry) = (decisions.clone(), Arc::downgrade(&h.registry));
    h.registry.on_complete(move |c| {
        let (registry, id) = (registry.upgrade().unwrap(), c.task_id.clone());
        let decision =
            tokio::spawn(async move { (id.clone(), registry.completion_due(&id).await) });
        sink.lock().unwrap().push(decision);
    });
    let decided = |nth: usize| {
        let decision = decisions.lock().unwrap().remove(nth);
        async move { decision.await.unwrap() }
    };
    for (name, ms) in [("quick", 50), ("slow", 300), ("dropped", 50)] {
        h.env
            .register_shell(name, ShellScript::long_running("", ms))
            .await;
    }

    // Ended before anyone read it: a completion notice is still due, until
    // a read returns the final result.
    let quick = start(&h, "quick", serde_json::json!({})).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(decided(0).await, (quick.clone(), true));
    let out = run(&h.output, "o", serde_json::json!({ "id": quick }))
        .await
        .unwrap();
    assert!(text(&out).contains("Final result:"), "{}", text(&out));
    assert!(!h.registry.completion_due(&quick).await);

    // A read waiting when the shell ends hands the result over, so the
    // notice is not sent.
    let slow = start(&h, "slow", serde_json::json!({})).await;
    let out = run(
        &h.output,
        "w",
        serde_json::json!({ "id": slow, "wait_seconds": 5 }),
    )
    .await
    .unwrap();
    assert!(text(&out).contains("Final result:"), "{}", text(&out));
    assert_eq!(decided(0).await, (slow, false));

    // A read in flight when the shell ends that never returns the result
    // (the call was cancelled) leaves the notice due.
    let dropped = start(&h, "dropped", serde_json::json!({})).await;
    let reading = h.registry.reading(&dropped);
    tokio::time::sleep(Duration::from_millis(200)).await;
    drop(reading);
    assert_eq!(decided(0).await, (dropped.clone(), true));

    // Once the session is disposed, no notice is due.
    h.registry.dispose().await;
    assert!(!h.registry.completion_due(&dropped).await);
}

#[tokio::test]
async fn auto_background_releases_after_the_threshold_as_model_origin() {
    let h = harness_with(
        BashToolOptions {
            auto_background_secs: Some(0.1),
            ..Default::default()
        },
        BashShellToolOptions::default(),
    );
    h.env
        .register_shell("slow", ShellScript::long_running("", 2_000))
        .await;
    let r = run(&h.bash, "fg", serde_json::json!({ "command": "slow" }))
        .await
        .expect("released");
    assert_eq!(r.details["background_origin"], "model");
    assert!(
        text(&r).starts_with("Still running after 0s"),
        "{}",
        text(&r)
    );
    // A command with a shorter timeout is never auto-backgrounded.
    h.env
        .register_shell("quick", ShellScript::long_running("", 50))
        .await;
    let r = run(
        &h.bash,
        "fg2",
        serde_json::json!({ "command": "quick", "timeout": 0.05 }),
    )
    .await;
    assert!(r.is_err() || r.unwrap().details.get("background_task_id").is_none());
    h.registry.dispose().await;
}

#[tokio::test]
async fn dispose_stops_shells_and_removes_logs() {
    let h = harness();
    h.env
        .register_shell("forever", ShellScript::long_running("x\n", 10_000))
        .await;
    let id = start(&h, "forever", serde_json::json!({})).await;
    let log = h.registry.get(&id).unwrap().log_path.unwrap();
    assert!(h.env.exists(&log, None).await.unwrap());
    h.registry.dispose().await;
    assert!(!h.env.exists(&log, None).await.unwrap());
    assert!(h.registry.snapshots().is_empty());
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(
        !h.env.exists(&log, None).await.unwrap(),
        "ingestion must not recreate a removed log after disposal"
    );
}

#[tokio::test]
async fn finished_shells_are_pruned_to_twenty_in_launch_order() {
    let h = harness();
    h.env
        .register_shell("quick", ShellScript::success("ok\n"))
        .await;
    let mut launched = Vec::new();
    for _ in 0..22 {
        let id = start(&h, "quick", serde_json::json!({})).await;
        launched.push(id.clone());
        run(
            &h.output,
            "o",
            serde_json::json!({ "id": id, "wait_seconds": 5 }),
        )
        .await
        .unwrap();
    }
    let ids: Vec<String> = h
        .registry
        .snapshots()
        .into_iter()
        .map(|s| s.task_id)
        .collect();
    assert_eq!(ids, launched[2..]);
}

#[cfg(unix)]
fn os_harness(
    dir: &std::path::Path,
) -> (
    Arc<BashBackgroundRegistry>,
    Arc<dyn AgentTool>,
    Vec<Arc<dyn AgentTool>>,
) {
    let env: Arc<dyn ice_tools::ExecutionEnv> =
        Arc::new(ice_tools::OsExecutionEnv::with_cwd(dir.to_path_buf()));
    let registry = BashBackgroundRegistry::new();
    let bash = ice_tools::create_bash_tool(
        &ExecutionToolContext::new(env, None),
        Some(BashToolOptions {
            background: Some(registry.clone()),
            ..Default::default()
        }),
    );
    let tools = create_bash_shell_tools(
        registry.clone(),
        BashShellToolOptions {
            input_approval: BashInputApproval::Allow,
            ..Default::default()
        },
    );
    (registry, bash, tools)
}

#[cfg(unix)]
#[tokio::test]
async fn real_shell_stdin_reaches_the_process() {
    let dir = tempfile::tempdir().unwrap();
    let (registry, bash, tools) = os_harness(dir.path());
    let launched = run(
        &bash,
        "c",
        serde_json::json!({ "command": "read line; echo got:$line", "run_in_background": true, "stdin": true }),
    )
    .await;
    let Ok(launched) = launched else { return }; // no bash on this machine
    let id = launched.details["background_task_id"]
        .as_str()
        .unwrap()
        .to_string();
    run(
        &tools[2],
        "i",
        serde_json::json!({ "id": id, "text": "hello\n" }),
    )
    .await
    .expect("input ok");
    let out = run(
        &tools[0],
        "o",
        serde_json::json!({ "id": id, "wait_seconds": 10 }),
    )
    .await
    .unwrap();
    assert!(text(&out).contains("got:hello"), "{}", text(&out));
    registry.dispose().await;
}

#[cfg(unix)]
#[tokio::test]
async fn real_shell_lingering_group_is_stopped_by_bash_kill() {
    let dir = tempfile::tempdir().unwrap();
    let (registry, bash, tools) = os_harness(dir.path());
    // The shell exits at once but leaves `sleep` running in its group.
    let launched = run(
        &bash,
        "c",
        serde_json::json!({ "command": "sleep 30 >/dev/null 2>&1 & echo started", "run_in_background": true }),
    )
    .await;
    let Ok(launched) = launched else { return };
    let id = launched.details["background_task_id"]
        .as_str()
        .unwrap()
        .to_string();
    run(
        &tools[0],
        "o",
        serde_json::json!({ "id": id, "wait_seconds": 10 }),
    )
    .await
    .unwrap();
    let snap = registry.get(&id).unwrap();
    assert_eq!(snap.status, BackgroundShellStatus::Completed);
    assert!(snap.lingering, "sleep should still be running in the group");
    let r = run(&tools[1], "k", serde_json::json!({ "id": id }))
        .await
        .unwrap();
    assert_eq!(
        text(&r),
        format!("Stopped the processes {id} left running.")
    );
    assert!(!registry.get(&id).unwrap().lingering);
    registry.dispose().await;
}

/// Without a registry the release channel is closed from the start; output
/// that arrives in separate bursts must not poll it again.
#[cfg(unix)]
#[tokio::test]
async fn a_foreground_call_without_background_shells_survives_output_bursts() {
    let dir = tempfile::tempdir().unwrap();
    let env: Arc<dyn ice_tools::ExecutionEnv> = Arc::new(ice_tools::OsExecutionEnv::with_cwd(
        dir.path().to_path_buf(),
    ));
    let bash = ice_tools::create_bash_tool(&ExecutionToolContext::new(env, None), None);
    let r = run(
        &bash,
        "fg",
        serde_json::json!({ "command": "printf a; sleep 0.2; printf b; sleep 0.2; printf c" }),
    )
    .await
    .unwrap();
    assert_eq!(text(&r), "abc");
}
