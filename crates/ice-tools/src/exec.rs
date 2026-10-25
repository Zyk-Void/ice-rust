//! Mirrors `packages/coding-agent/src/core/exec.ts` — run an executable with
//! an exact argument list (no shell), capture stdout and stderr separately,
//! and always return a result, even on spawn failure, timeout or abort.
//!
//! Differences from ice, on purpose:
//! - abort and timeout kill the whole process tree (ice signals only the
//!   direct child and its SIGKILL escalation depends on a flag that does not
//!   prove the process exited);
//! - a spawn or wait failure is reported in `stderr` instead of being dropped;
//! - each stream decodes UTF-8 incrementally, so a character split across
//!   reads stays intact.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio_util::sync::CancellationToken;

use crate::os_env::{kill_process_tree, Utf8StreamDecoder};

/// Pipe inactivity after exit before reading stops. Mirrors
/// `waitForChildProcess`'s 100 ms release.
const POST_EXIT_IDLE: Duration = Duration::from_millis(100);
/// Hard cap for draining inherited pipes after the direct child exits. A
/// detached descendant can keep writing forever, so idle-only draining is
/// not sufficient to bound the command.
const POST_EXIT_DRAIN_LIMIT: Duration = Duration::from_secs(2);

/// Options for [`exec_command`]. Mirrors `ExecOptions` (timeout in
/// milliseconds there; a `Duration` here).
#[derive(Debug, Clone, Default)]
pub struct ExecOptions {
    pub signal: Option<CancellationToken>,
    pub timeout: Option<Duration>,
}

/// Mirrors `ExecResult`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExecResult {
    pub stdout: String,
    pub stderr: String,
    /// The exit code; 0 when the process ended without one (e.g. by a
    /// signal it handled), 1 when it could not be run.
    pub code: i32,
    /// True when abort or the timeout stopped the process.
    pub killed: bool,
}

/// Run `command` with `args` in `cwd`. Stdin is closed. Mirrors `execCommand`.
pub async fn exec_command(
    command: &str,
    args: &[String],
    cwd: &Path,
    options: ExecOptions,
) -> ExecResult {
    let mut cmd = tokio::process::Command::new(command);
    cmd.args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(error) => {
            return ExecResult {
                stderr: format!("failed to run {command}: {error}"),
                code: 1,
                ..Default::default()
            }
        }
    };
    let pid = child.id();
    let mut stdout_pipe = child.stdout.take();
    let mut stderr_pipe = child.stderr.take();
    let mut stdout = String::new();
    let mut stderr = String::new();
    let mut out_decoder = Utf8StreamDecoder::default();
    let mut err_decoder = Utf8StreamDecoder::default();
    let mut out_buf = vec![0u8; 8192];
    let mut err_buf = vec![0u8; 8192];
    let signal = options.signal.unwrap_or_default();
    let deadline = async {
        match options.timeout {
            Some(timeout) => tokio::time::sleep(timeout).await,
            None => std::future::pending().await,
        }
    };
    tokio::pin!(deadline);

    let mut killed = false;
    let mut status = None;
    let mut post_exit_deadline = None;
    // Read both pipes while waiting, so a chatty child cannot block on a
    // full pipe; once it exits, keep reading until the pipes go quiet.
    loop {
        let reading = stdout_pipe.is_some() || stderr_pipe.is_some();
        let idle = async {
            if status.is_some() {
                tokio::time::sleep(POST_EXIT_IDLE).await
            } else {
                std::future::pending().await
            }
        };
        let post_exit_limit = async {
            if let Some(deadline) = post_exit_deadline {
                tokio::time::sleep_until(deadline).await;
            } else {
                std::future::pending().await
            }
        };
        tokio::select! {
            read = read_some(&mut stdout_pipe, &mut out_buf), if stdout_pipe.is_some() => {
                match read {
                    Some(bytes) => stdout.push_str(&out_decoder.decode(&bytes)),
                    None => stdout_pipe = None,
                }
            }
            read = read_some(&mut stderr_pipe, &mut err_buf), if stderr_pipe.is_some() => {
                match read {
                    Some(bytes) => stderr.push_str(&err_decoder.decode(&bytes)),
                    None => stderr_pipe = None,
                }
            }
            result = child.wait(), if status.is_none() => {
                status = Some(result);
                post_exit_deadline = Some(tokio::time::Instant::now() + POST_EXIT_DRAIN_LIMIT);
                if !reading {
                    break;
                }
            }
            _ = signal.cancelled(), if status.is_none() && !killed => {
                killed = true;
                if let Some(pid) = pid {
                    kill_process_tree(pid).await;
                }
            }
            _ = &mut deadline, if status.is_none() && !killed => {
                killed = true;
                if let Some(pid) = pid {
                    kill_process_tree(pid).await;
                }
            }
            _ = idle => break,
            _ = post_exit_limit => break,
        }
        if status.is_some() && stdout_pipe.is_none() && stderr_pipe.is_none() {
            break;
        }
    }
    stdout.push_str(&out_decoder.finish());
    stderr.push_str(&err_decoder.finish());
    let code = match status {
        Some(Ok(status)) => status.code().unwrap_or(0),
        Some(Err(error)) => {
            stderr.push_str(&format!("\nfailed to wait for {command}: {error}"));
            1
        }
        None => 1,
    };
    ExecResult {
        stdout,
        stderr,
        code,
        killed,
    }
}

/// One read; `None` at EOF or on a read error.
async fn read_some<R: tokio::io::AsyncRead + Unpin>(
    pipe: &mut Option<R>,
    buf: &mut [u8],
) -> Option<Vec<u8>> {
    let reader = pipe.as_mut()?;
    match reader.read(buf).await {
        Ok(0) | Err(_) => None,
        Ok(n) => Some(buf[..n].to_vec()),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[tokio::test]
    async fn runs_argv_without_a_shell_and_separates_streams() {
        let dir = std::env::temp_dir();
        let r = exec_command(
            "sh",
            &args(&[
                "-c",
                "printf 'out %s' \"$1\"; printf err >&2; exit 3",
                "sh",
                "a b;c",
            ]),
            &dir,
            ExecOptions::default(),
        )
        .await;
        // The argument reaches the program verbatim; no shell expands it.
        assert_eq!(r.stdout, "out a b;c");
        assert_eq!(r.stderr, "err");
        assert_eq!(r.code, 3);
        assert!(!r.killed);
    }

    #[tokio::test]
    async fn timeout_and_abort_kill_and_report() {
        let dir = std::env::temp_dir();
        let started = std::time::Instant::now();
        let r = exec_command(
            "sleep",
            &args(&["30"]),
            &dir,
            ExecOptions {
                timeout: Some(Duration::from_millis(100)),
                ..Default::default()
            },
        )
        .await;
        assert!(r.killed);
        assert!(started.elapsed() < Duration::from_secs(5));
        let token = CancellationToken::new();
        token.cancel();
        let r = exec_command(
            "sleep",
            &args(&["30"]),
            &dir,
            ExecOptions {
                signal: Some(token),
                ..Default::default()
            },
        )
        .await;
        assert!(r.killed);
    }

    #[tokio::test]
    async fn spawn_failure_still_returns_a_result() {
        let r = exec_command(
            "definitely-not-a-real-binary-xyz",
            &[],
            &std::env::temp_dir(),
            ExecOptions::default(),
        )
        .await;
        assert_eq!(r.code, 1);
        assert!(r
            .stderr
            .starts_with("failed to run definitely-not-a-real-binary-xyz"));
    }

    #[tokio::test]
    async fn multibyte_output_split_across_reads_is_intact() {
        // 9000 × "é" (2 bytes each) spans several 8 KiB reads.
        let r = exec_command(
            "sh",
            &args(&[
                "-c",
                "i=0; while [ $i -lt 9000 ]; do printf 'é'; i=$((i+1)); done",
            ]),
            &std::env::temp_dir(),
            ExecOptions::default(),
        )
        .await;
        assert_eq!(r.code, 0);
        assert_eq!(r.stdout.chars().count(), 9000);
        assert!(!r.stdout.contains('\u{FFFD}'));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn detached_writer_cannot_hold_exec_open_after_parent_exit() {
        let started = std::time::Instant::now();
        let result = exec_command(
            "sh",
            &args(&[
                "-c",
                "setsid sh -c 'echo $$; while :; do printf x; sleep 0.01; done' & exit 0",
            ]),
            &std::env::temp_dir(),
            ExecOptions::default(),
        )
        .await;

        let pid = result
            .stdout
            .lines()
            .next()
            .and_then(|line| line.parse::<i32>().ok())
            .expect("detached writer should report its pid");
        let _ = nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(pid),
            nix::sys::signal::Signal::SIGKILL,
        );
        assert!(started.elapsed() < POST_EXIT_DRAIN_LIMIT + Duration::from_secs(1));
        assert_eq!(result.code, 0);
    }
}
