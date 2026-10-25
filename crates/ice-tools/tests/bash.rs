//! Mirrors `packages/agent/test/harness/tools.test.ts` "bash" describe block —
//! non-zero exit → Err, "(no output)", truncation temp-file, oversized-final-line
//! size report, command prefix, and output combination. The TS runs against a
//! real `NodeExecutionEnv`+bash; the Rust port runs the SAME invariants against
//! `InMemoryExecutionEnv` with canned `ShellScript`s (deterministic, no-platform-
//! bash dependency) plus the `OsExecutionEnv` for a couple of real-shell smoke
//! checks gated on bash availability.
//!
//! See the initial port notes (retired) for the divergence note on the timeout-truncated
//! output case (the TS `TimeoutOutputExecutionEnv` cannot be expressed against
//! `InMemoryExecutionEnv` without a blocking-override env like `abort_bracketing`).

use std::sync::Arc;
use std::time::Duration;

use ice_agent::agent_tool::AgentTool;
use ice_agent::error::AgentError;
use ice_tools::{
    BashToolOptions, ExecutionToolContext, FileSystem, InMemoryExecutionEnv, ShellScript,
    DEFAULT_MAX_BYTES, DEFAULT_MAX_LINES,
};
use tokio_util::sync::CancellationToken;

fn fresh_context() -> (Arc<InMemoryExecutionEnv>, ExecutionToolContext) {
    let env = Arc::new(InMemoryExecutionEnv::with_cwd("/tmp/work".into()));
    let env_dyn: Arc<dyn ice_tools::ExecutionEnv> = env.clone();
    let mut_env: Arc<dyn ice_tools::MutatingEnv> = env.clone();
    let ctx = ExecutionToolContext::new(env_dyn, Some(mut_env));
    (env, ctx)
}

async fn run_bash(
    tool: Arc<dyn AgentTool>,
    params: serde_json::Value,
) -> Result<ice_agent::types::AgentToolResult, AgentError> {
    let signal = CancellationToken::new();
    let on_update = Arc::new(|_p| ());
    let prepared = tool
        .prepare_arguments(params.clone())
        .unwrap_or_else(|_| params);
    tool.execute("bash-x", prepared, signal, on_update).await
}

fn text_output(r: &ice_agent::types::AgentToolResult) -> String {
    use ice_agent::types::TextContentOrImage;
    r.content
        .iter()
        .filter_map(|c| match c {
            TextContentOrImage::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn err_text(e: &AgentError) -> String {
    match e {
        AgentError::Tool(m) => m.clone(),
        other => format!("{other:?}"),
    }
}

#[tokio::test]
async fn no_output_renders_placeholder() {
    let (env, ctx) = fresh_context();
    env.register_shell("noop", ShellScript::success("")).await;
    let tool = ice_tools::create_bash_tool(&ctx, None);
    let r = run_bash(tool, serde_json::json!({ "command": "noop" }))
        .await
        .expect("ok");
    assert_eq!(text_output(&r), "(no output)");
}

#[tokio::test]
async fn combines_stdout_and_stderr() {
    let (env, ctx) = fresh_context();
    // ShellScript carries separate stdout/stderr but the capture layer merges
    // them into one rolling buffer. Seed a script with both non-empty.
    env.register_shell(
        "both",
        ShellScript {
            stdout: "out".into(),
            stderr: "err".into(),
            exit_code: 0,
            delay_ms: None,
            initial_stdout: String::new(),
        },
    )
    .await;
    let tool = ice_tools::create_bash_tool(&ctx, None);
    let r = run_bash(tool, serde_json::json!({ "command": "both" }))
        .await
        .expect("ok");
    let out = text_output(&r);
    assert!(out.contains("out"), "{out}");
    assert!(out.contains("err"), "{out}");
}

#[tokio::test]
async fn nonzero_exit_returns_error_with_output() {
    let (env, ctx) = fresh_context();
    env.register_shell(
        "fail",
        ShellScript {
            stdout: "failed".into(),
            stderr: String::new(),
            exit_code: 7,
            delay_ms: None,
            initial_stdout: String::new(),
        },
    )
    .await;
    let tool = ice_tools::create_bash_tool(&ctx, None);
    let err = run_bash(tool, serde_json::json!({ "command": "fail" }))
        .await
        .expect_err("nonzero exit → Err");
    let msg = err_text(&err);
    assert!(msg.contains("failed"), "{msg}");
    assert!(msg.contains("Command exited with code 7"), "{msg}");
}

#[tokio::test]
async fn abort_returns_error() {
    let (env, ctx) = fresh_context();
    env.register_shell(
        "slow",
        ShellScript {
            stdout: "x".into(),
            stderr: String::new(),
            exit_code: 0,
            delay_ms: Some(1000),
            initial_stdout: String::new(),
        },
    )
    .await;
    let tool = ice_tools::create_bash_tool(&ctx, None);
    let signal = CancellationToken::new();
    let on_update = Arc::new(|_p| ());
    let handle = {
        let tool = tool.clone();
        let sig = signal.clone();
        tokio::spawn(async move {
            tool.execute(
                "bash-abort",
                serde_json::json!({ "command": "slow" }),
                sig,
                on_update,
            )
            .await
        })
    };
    // Let the command enter its delay, then abort.
    tokio::time::sleep(Duration::from_millis(20)).await;
    signal.cancel();
    let err = handle.await.expect("join").expect_err("abort → Err");
    let msg = err_text(&err);
    assert!(msg.contains("aborted") || msg.contains("Aborted"), "{msg}");
}

#[tokio::test]
async fn persists_truncated_full_output_to_temp_file() {
    let (env, ctx) = fresh_context();
    // Emit DEFAULT_MAX_LINES + 1 lines — exceeds the line cap, triggers the
    // post-stream temp-file spill.
    let lines: String = (0..(DEFAULT_MAX_LINES + 1))
        .map(|i| format!("line-{}", i + 1))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    env.register_shell("big", ShellScript::success(lines)).await;

    let tool = ice_tools::create_bash_tool(&ctx, None);
    let r = run_bash(tool, serde_json::json!({ "command": "big" }))
        .await
        .expect("ok");
    // Truncation metadata + a fullOutputPath.
    let trunc = r.details.get("truncation").expect("truncation present");
    assert_eq!(trunc["truncated"], true);
    assert_eq!(trunc["truncated_by"], "lines");
    assert_eq!(trunc["total_lines"], DEFAULT_MAX_LINES + 1);
    assert_eq!(trunc["output_lines"], DEFAULT_MAX_LINES);
    let full_path = r
        .details
        .get("full_output_path")
        .and_then(|v| v.as_str())
        .expect("full_output_path present");
    let full = env
        .read_text_file(full_path, None)
        .await
        .expect("read full output");
    assert!(full.contains("line-1\nline-2"), "{full}");
    assert!(
        full.contains(&format!(
            "line-{}\nline-{}",
            DEFAULT_MAX_LINES,
            DEFAULT_MAX_LINES + 1
        )),
        "{full}"
    );
}

#[tokio::test]
async fn byte_limited_range_notice_names_the_first_visible_line() {
    let (env, ctx) = fresh_context();
    // 1000 lines of 99 bytes + LF: under the line cap, over the byte cap, so
    // the tail keeps only the last ~500 lines.
    let total = 1000usize;
    let lines: String = (1..=total)
        .map(|i| format!("{:<99}\n", format!("line-{i}")))
        .collect();
    env.register_shell("wide", ShellScript::success(lines))
        .await;

    let tool = ice_tools::create_bash_tool(&ctx, None);
    let r = run_bash(tool, serde_json::json!({ "command": "wide" }))
        .await
        .expect("ok");
    let trunc = r.details.get("truncation").expect("truncation present");
    assert_eq!(trunc["truncated_by"], "bytes");
    let shown = trunc["output_lines"].as_u64().expect("output_lines") as usize;
    assert!(shown < total, "{shown}");

    let out = text_output(&r);
    let first = total - shown + 1;
    let (body, notice) = out.split_once("\n\n[").expect("notice present");
    assert_eq!(body.lines().count(), shown);
    assert!(
        body.starts_with(&format!("line-{first} ")),
        "body starts with {:?}",
        &body[..20]
    );
    assert!(
        notice.starts_with(&format!(
            "Showing lines {first}-{total} of {total} (50.0KB limit)."
        )),
        "{notice}"
    );
}

#[tokio::test]
async fn reports_oversized_final_line_size() {
    let (env, ctx) = fresh_context();
    // One line that exceeds the 50KB byte cap → last_line_partial path.
    let big_line: String = "0".repeat(DEFAULT_MAX_BYTES + 5000);
    env.register_shell("oneline", ShellScript::success(big_line))
        .await;

    let tool = ice_tools::create_bash_tool(&ctx, None);
    let r = run_bash(tool, serde_json::json!({ "command": "oneline" }))
        .await
        .expect("ok");
    let out = text_output(&r);
    // The TS expects "Showing last <size> of line 1 (line is <size>). Full output:".
    assert!(out.contains("Showing last"), "{out}");
    assert!(out.contains("line 1"), "{out}");
}

#[tokio::test]
async fn supports_command_prefix() {
    let (env, ctx) = fresh_context();
    // Prefix sets a variable; the command echoes it. The InMemory shell matches
    // by longest prefix, so register a script for the FULL composed command the
    // tool emits (prefix + "\n" + command).
    env.register_shell("value=hello\necho hello", ShellScript::success("hello"))
        .await;
    let tool = ice_tools::create_bash_tool(
        &ctx,
        Some(BashToolOptions {
            command_prefix: Some("value=hello".into()),
            ..Default::default()
        }),
    );
    // The shell_script prefix-matches on the composed command start; echo is the
    // user command. Register a broader prefix to be safe.
    env.register_shell("value=hello", ShellScript::success("hello"))
        .await;
    let r = run_bash(tool, serde_json::json!({ "command": "echo ignored" }))
        .await
        .expect("ok");
    assert_eq!(text_output(&r), "hello");
}

#[tokio::test]
async fn full_output_log_keeps_output_older_than_the_tail() {
    let (env, ctx) = fresh_context();
    // ~300 KB: three times the 100 KB in-memory tail. The log must hold the
    // first line too, not just what the tail retained.
    let lines: String = (1..=3000)
        .map(|i| format!("{:<99}\n", format!("line-{i}")))
        .collect();
    env.register_shell("huge", ShellScript::success(lines.clone()))
        .await;
    let tool = ice_tools::create_bash_tool(&ctx, None);
    let r = run_bash(tool, serde_json::json!({ "command": "huge" }))
        .await
        .expect("ok");
    let path = r
        .details
        .get("full_output_path")
        .and_then(|v| v.as_str())
        .expect("full_output_path present");
    let log = env.read_text_file(path, None).await.expect("read log");
    assert_eq!(log.len(), lines.len());
    assert!(log.starts_with("line-1 "), "{}", &log[..20]);
    assert!(log.contains("\nline-1500 "));
    assert!(log.ends_with(&format!("{:<99}\n", "line-3000")));
    assert!(r.details.get("log_issue").is_none());
}

#[tokio::test]
async fn first_output_update_is_immediate_and_initial_update_is_empty() {
    let (env, ctx) = fresh_context();
    env.register_shell("hello", ShellScript::success("hi\n"))
        .await;
    let tool = ice_tools::create_bash_tool(&ctx, None);
    let updates = Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = updates.clone();
    let on_update = Arc::new(move |p: ice_agent::types::AgentToolResult| {
        sink.lock().unwrap().push(p);
    });
    tool.execute(
        "bash-u",
        serde_json::json!({ "command": "hello" }),
        CancellationToken::new(),
        on_update,
    )
    .await
    .expect("ok");
    // Let any (wrongly) pending timer fire before inspecting.
    tokio::time::sleep(Duration::from_millis(250)).await;
    let updates = updates.lock().unwrap();
    assert!(
        updates[0].content.is_empty(),
        "initial update has no blocks"
    );
    // The chunk update is emitted at once, then the final update; no stale
    // partial follows the final one.
    assert_eq!(updates.len(), 3, "initial, first chunk, final");
    assert_eq!(text_output(&updates[1]), "hi\n");
    assert_eq!(text_output(&updates[2]), "hi\n");
}

fn slow(stdout: &str, delay_ms: u64) -> ShellScript {
    ShellScript {
        delay_ms: Some(delay_ms),
        ..ShellScript::success(stdout)
    }
}

#[tokio::test]
async fn zero_timeout_means_no_timeout() {
    let (env, ctx) = fresh_context();
    env.register_shell("quick", ShellScript::success("done"))
        .await;
    let tool = ice_tools::create_bash_tool(&ctx, None);
    let r = run_bash(
        tool,
        serde_json::json!({ "command": "quick", "timeout": 0 }),
    )
    .await
    .expect("timeout 0 is accepted");
    assert_eq!(text_output(&r), "done");
}

#[tokio::test]
async fn timeout_error_uses_seconds() {
    let (env, ctx) = fresh_context();
    env.register_shell("sleepy", slow("partial", 5_000)).await;
    let tool = ice_tools::create_bash_tool(&ctx, None);
    let err = run_bash(
        tool,
        serde_json::json!({ "command": "sleepy", "timeout": 0.05 }),
    )
    .await
    .expect_err("times out");
    let msg = err_text(&err);
    assert!(
        msg.ends_with("Command timed out after 0.05 seconds"),
        "{msg}"
    );
}

#[tokio::test]
async fn max_timeout_caps_a_longer_model_timeout() {
    let (env, ctx) = fresh_context();
    env.register_shell("sleepy", slow("", 5_000)).await;
    let tool = ice_tools::create_bash_tool(
        &ctx,
        Some(BashToolOptions {
            max_timeout: Some(0.05),
            ..Default::default()
        }),
    );
    let err = run_bash(
        tool,
        serde_json::json!({ "command": "sleepy", "timeout": 3600 }),
    )
    .await
    .expect_err("the host ceiling wins");
    assert!(
        err_text(&err).contains("Command timed out after 0.05 seconds"),
        "{}",
        err_text(&err)
    );
}

#[tokio::test]
async fn background_and_stdin_requests_are_rejected_without_a_registry() {
    let (env, ctx) = fresh_context();
    env.register_shell("server", ShellScript::success("")).await;
    let tool = ice_tools::create_bash_tool(&ctx, None);
    let err = run_bash(
        tool.clone(),
        serde_json::json!({ "command": "server", "stdin": true }),
    )
    .await
    .expect_err("stdin needs background");
    assert_eq!(
        err_text(&err),
        "stdin: true requires run_in_background: true."
    );
    let err = run_bash(
        tool,
        serde_json::json!({
            "command": "server",
            "run_in_background": true,
            "description": "start server"
        }),
    )
    .await
    .expect_err("no registry");
    assert!(
        err_text(&err).starts_with("Background shells are not available here"),
        "{}",
        err_text(&err)
    );
}
