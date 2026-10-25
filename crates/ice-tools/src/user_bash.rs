//! Mirrors `packages/coding-agent/src/core/bash-executor.ts` — run a
//! user-typed shell command (`!cmd` / `!!cmd`) outside any model tool call.
//!
//! The command gets the same configuration as the model's bash tool
//! (command prefix, session variables, prepare hook) via
//! [`BashToolOptions::prepare_execution`], and the same capture (sanitizing,
//! spill to a temp log, 2000-line/50 KB tail) as [`execute_shell_with_capture`].
//! Like ice's helper it takes no timeout, and it does not persist anything:
//! recording the result and deciding whether it reaches model context is the
//! caller's job.
//!
//! Differences from ice, on purpose:
//! - callers choose between deltas (`on_data`, ice's `onData`) and snapshots
//!   of the whole retained tail (`on_snapshot`), or take both;
//! - the decoder is flushed at EOF, the log is closed before returning, and
//!   an incomplete log is reported through `log_issue`;
//! - a failure to run is a result with `error` set, never a panic or a lost
//!   message.

use std::path::PathBuf;
use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use crate::env::ExecutionEnv;
use crate::shell_output::{execute_shell_with_capture, LogFileIssue, ShellCaptureOptions};
use crate::tools::bash::BashToolOptions;

/// Callback receiving text: a delta for `on_data`, the whole tail for
/// `on_snapshot`.
pub type UserBashCallback<'a> = Box<dyn FnMut(&str) + Send + 'a>;

/// Mirrors `BashExecutorOptions`.
#[derive(Default)]
pub struct UserBashOptions<'a> {
    pub cancel: Option<&'a CancellationToken>,
    /// Each sanitized chunk as it arrives.
    pub on_data: Option<UserBashCallback<'a>>,
    /// The complete retained tail after each chunk.
    pub on_snapshot: Option<UserBashCallback<'a>>,
}

/// Mirrors `BashResult`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UserBashResult {
    pub output: String,
    /// `None` when cancelled, when the process had no exit code, or when it
    /// could not be run.
    pub exit_code: Option<i32>,
    pub cancelled: bool,
    pub truncated: bool,
    pub full_output_path: Option<PathBuf>,
    pub log_issue: Option<LogFileIssue>,
    /// Why the command could not be run (prepare hook, spawn, environment).
    pub error: Option<String>,
}

/// Run `command` in the environment's cwd. Mirrors `executeBashWithOperations`.
pub async fn execute_user_bash(
    env: &Arc<dyn ExecutionEnv>,
    command: &str,
    bash: &BashToolOptions,
    options: UserBashOptions<'_>,
) -> UserBashResult {
    let failed = |error: String| UserBashResult {
        error: Some(error),
        ..Default::default()
    };
    let execution = match bash
        .prepare_execution(command.to_string(), env.cwd().to_path_buf())
        .await
    {
        Ok(execution) => execution,
        Err(error) => return failed(error),
    };
    if options.cancel.is_some_and(|c| c.is_cancelled()) {
        return UserBashResult {
            cancelled: true,
            ..Default::default()
        };
    }
    let UserBashOptions {
        cancel,
        mut on_data,
        mut on_snapshot,
    } = options;
    let on_chunk: Option<crate::shell_output::ChunkCallback<'_>> =
        if on_data.is_some() || on_snapshot.is_some() {
            Some(Box::new(move |chunk, progress| {
                if let Some(on_data) = on_data.as_mut() {
                    on_data(chunk);
                }
                if let Some(on_snapshot) = on_snapshot.as_mut() {
                    on_snapshot(&progress().output);
                }
            }))
        } else {
            None
        };
    let capture = execute_shell_with_capture(
        env,
        &execution.command,
        ShellCaptureOptions {
            cwd: Some(execution.cwd),
            env: Some(execution.env),
            inherit_env: execution.inherit_env,
            cancel,
            on_chunk,
            return_execution_errors: true,
            ..Default::default()
        },
    )
    .await;
    match capture {
        Ok(capture) => UserBashResult {
            output: capture.output,
            exit_code: if capture.cancelled {
                None
            } else {
                capture.exit_code
            },
            cancelled: capture.cancelled,
            truncated: capture.truncated,
            full_output_path: capture.full_output_path,
            log_issue: capture.log_issue,
            error: capture.execution_error.map(|e| e.to_string()),
        },
        Err(error) => failed(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::in_memory::{InMemoryExecutionEnv, ShellScript};
    use std::sync::Mutex;

    async fn env_with(prefix: &str, script: ShellScript) -> Arc<dyn ExecutionEnv> {
        let env = InMemoryExecutionEnv::with_cwd("/w".into());
        env.register_shell(prefix, script).await;
        Arc::new(env)
    }

    #[tokio::test]
    async fn shares_the_bash_tool_configuration() {
        // Only the prefixed command matches, so the output proves the prefix
        // was applied.
        let env = env_with("set -e\necho hi", ShellScript::success("hi\n")).await;
        let bash = BashToolOptions {
            command_prefix: Some("set -e".into()),
            ..Default::default()
        };
        let result = execute_user_bash(&env, "echo hi", &bash, UserBashOptions::default()).await;
        assert_eq!(result.output, "hi\n");
        assert_eq!(result.exit_code, Some(0));
        assert_eq!(result.error, None);
    }

    #[tokio::test]
    async fn reports_deltas_and_snapshots() {
        let env = env_with("two", ShellScript::success("a\nb\n")).await;
        let deltas = Arc::new(Mutex::new(String::new()));
        let snapshots = Arc::new(Mutex::new(Vec::<String>::new()));
        let (d, s) = (deltas.clone(), snapshots.clone());
        let result = execute_user_bash(
            &env,
            "two",
            &BashToolOptions::default(),
            UserBashOptions {
                on_data: Some(Box::new(move |chunk| d.lock().unwrap().push_str(chunk))),
                on_snapshot: Some(Box::new(move |tail| s.lock().unwrap().push(tail.into()))),
                ..Default::default()
            },
        )
        .await;
        assert_eq!(&*deltas.lock().unwrap(), "a\nb\n");
        assert!(snapshots
            .lock()
            .unwrap()
            .last()
            .is_some_and(|tail| tail.starts_with("a\nb")));
        assert_eq!(result.output, "a\nb\n");
    }

    #[tokio::test]
    async fn a_failing_prepare_hook_is_a_result_not_a_panic() {
        let env = env_with("x", ShellScript::success("")).await;
        let bash = BashToolOptions {
            prepare: Some(Arc::new(|_| Box::pin(async { Err("denied".to_string()) }))),
            ..Default::default()
        };
        let result = execute_user_bash(&env, "x", &bash, UserBashOptions::default()).await;
        assert_eq!(result.error.as_deref(), Some("denied"));
        assert_eq!(result.exit_code, None);
    }

    #[tokio::test]
    async fn cancellation_clears_the_exit_code() {
        let env = env_with("slow", ShellScript::long_running("start\n", 10_000)).await;
        let cancel = CancellationToken::new();
        let trigger = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            trigger.cancel();
        });
        let result = execute_user_bash(
            &env,
            "slow",
            &BashToolOptions::default(),
            UserBashOptions {
                cancel: Some(&cancel),
                ..Default::default()
            },
        )
        .await;
        assert!(result.cancelled);
        assert_eq!(result.exit_code, None);
    }
}
