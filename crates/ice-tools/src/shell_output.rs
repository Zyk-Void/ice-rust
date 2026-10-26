//! Mirrors `packages/agent/src/harness/utils/shell-output.ts` — the bash tool's
//! output-capture layer wrapping `Shell::exec`.
//!
//! Combines stdout+stderr into a single rolling tail buffer (capped at
//! `max_output_bytes = DEFAULT_MAX_BYTES * 2 = 100KB`), spills to a temp file
//! (`bash-*.log`) once cumulative output exceeds `DEFAULT_MAX_BYTES`/`DEFAULT_MAX_LINES`,
//! and produces a [`ShellCaptureResult`] carrying truncation metadata + the
//! temp-file path. `on_chunk` fires per sanitized/CR-stripped chunk with a
//! progress snapshot.
//!
//! `return_execution_errors: true` causes exec failures to come back as a
//! success-result with `execution_error` populated (so the bash tool — not the
//! env — owns error rendering via `append_status`). There is NO time-based
//! throttle here; throttling lives in the bash tool.
//!
//! The env's stdout/stderr callbacks are synchronous, so they only sanitize a
//! chunk and queue it on an unbounded channel. An async loop running beside
//! `exec` drains that channel in order: it updates the counters, creates the
//! log the moment output crosses a limit (writing every chunk held back so
//! far before the tail is trimmed), appends later chunks and calls
//! `on_chunk`. Nothing is dropped under contention, and the log holds the
//! whole sanitized output up to [`MAX_LOG_FILE_BYTES`].
//!
//! Policy note: ice's coding-agent bash logs raw bytes and the harness strips
//! only control characters. icers removes whole ANSI sequences from both the
//! model text and the log, because a leftover `[36m` body is noise to the
//! model.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use crate::env::{ExecutionEnv, FileContent, ShellExecOptions, ShellOutput};
use crate::error::{ExecutionError, ExecutionErrorCode};
use crate::truncate::{
    trim_to_last_utf8_bytes, truncate_tail, TruncationLimit, TruncationOptions, TruncationResult,
    DEFAULT_MAX_BYTES, DEFAULT_MAX_LINES,
};

/// The 100KB cap on the in-memory `tail_output` rolling buffer.
const MAX_OUTPUT_BYTES: usize = DEFAULT_MAX_BYTES * 2;

/// Cap on the full-output log. Mirrors `MAX_LOG_FILE_BYTES` (64 MiB).
pub const MAX_LOG_FILE_BYTES: usize = 64 * 1024 * 1024;

/// Bytes kept free at the end of a capped log for the stop marker.
const LOG_MARKER_RESERVE: usize = 256;

/// Why the full-output log is incomplete. Mirrors TS `LogFileIssue`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LogFileIssue {
    /// The log reached `max_bytes`; later output is only in the tail.
    Truncated { max_bytes: usize },
    /// Creating or appending to the log failed; logging stopped there.
    WriteFailed { error: String },
}

impl LogFileIssue {
    /// One-line description surfaced to the model. Mirrors
    /// `describeLogFileIssue`.
    pub fn describe(&self) -> String {
        match self {
            LogFileIssue::Truncated { max_bytes } => format!(
                "log stopped at {}; later output is only in the retained tail",
                crate::truncate::format_size(*max_bytes)
            ),
            LogFileIssue::WriteFailed { error } => format!(
                "log write failed ({error}); the log is incomplete and later output is only in the retained tail"
            ),
        }
    }
}

/// Mirrors `ShellCaptureProgress`.
#[derive(Clone)]
pub struct ShellCaptureProgress {
    pub output: String,
    pub truncation: TruncationResult,
    pub full_output_path: Option<PathBuf>,
    pub last_line_bytes: usize,
    pub log_issue: Option<LogFileIssue>,
}

/// Mirrors `ShellCaptureResult`.
#[derive(Clone)]
pub struct ShellCaptureResult {
    pub output: String,
    pub truncation: TruncationResult,
    pub full_output_path: Option<PathBuf>,
    pub last_line_bytes: usize,
    pub exit_code: Option<i32>,
    pub cancelled: bool,
    pub truncated: bool,
    pub execution_error: Option<ExecutionError>,
    /// Set when the log at `full_output_path` is incomplete (or was never
    /// created): it must not be presented as the complete record.
    pub log_issue: Option<LogFileIssue>,
}

/// Per-chunk callback: the sanitized chunk plus a progress getter.
pub type ChunkCallback<'a> = Box<dyn FnMut(&str, &dyn Fn() -> ShellCaptureProgress) + Send + 'a>;

/// Mirrors `ShellCaptureOptions`.
pub struct ShellCaptureOptions<'a> {
    pub shell: Option<crate::env::ShellOverride>,
    pub cwd: Option<PathBuf>,
    pub env: Option<HashMap<String, String>>,
    pub inherit_env: bool,
    pub timeout: Option<f64>,
    pub cancel: Option<&'a CancellationToken>,
    /// Called with each sanitized chunk, in order, after it is counted and
    /// logged.
    pub on_chunk: Option<ChunkCallback<'a>>,
    pub return_execution_errors: bool,
    /// Override for [`MAX_LOG_FILE_BYTES`] (tests use a small cap).
    pub max_log_bytes: Option<usize>,
}

impl<'a> Default for ShellCaptureOptions<'a> {
    fn default() -> Self {
        Self {
            shell: None,
            cwd: None,
            env: None,
            inherit_env: true,
            timeout: None,
            cancel: None,
            on_chunk: None,
            return_execution_errors: false,
            max_log_bytes: None,
        }
    }
}

/// Capture state, owned by the ingest loop.
#[derive(Default)]
struct CaptureState {
    tail_output: String,
    total_bytes: usize,
    completed_lines: usize,
    has_open_line: bool,
    current_line_bytes: usize,
    full_output_path: Option<PathBuf>,
    /// Output not yet written to the log. Holds everything until the log is
    /// created, so the prefix survives tail trimming.
    unlogged: String,
    /// Bytes already appended to the log.
    logged_bytes: usize,
    /// Logging stopped (capped or failed); `log_issue` says why.
    logging_stopped: bool,
    /// Create the log even below the display limits (background shells).
    force_log: bool,
    log_issue: Option<LogFileIssue>,
}

impl CaptureState {
    fn ingest_chunk(&mut self, text: &str) {
        self.count_chunk(text);
        if !self.logging_stopped {
            self.unlogged.push_str(text);
        }
        self.tail_output.push_str(text);
    }

    fn exceeds_display_limits(&self) -> bool {
        let total_lines = self.completed_lines + usize::from(self.has_open_line);
        total_lines > DEFAULT_MAX_LINES || self.total_bytes > DEFAULT_MAX_BYTES
    }

    /// Cap the in-memory tail at MAX_OUTPUT_BYTES. Called only after the
    /// pending log write, so trimming never loses unlogged output.
    fn trim_tail(&mut self) {
        let trimmed = trim_to_last_utf8_bytes(&self.tail_output, MAX_OUTPUT_BYTES);
        if trimmed.len() != self.tail_output.len() {
            self.tail_output = trimmed.to_string();
        }
    }

    fn count_chunk(&mut self, text: &str) {
        let text_bytes = text.len();
        self.total_bytes += text_bytes;
        let newline_count = text.matches('\n').count();
        self.completed_lines += newline_count;
        let last_newline = text.rfind('\n');
        match last_newline {
            Some(idx) => {
                let trailing = &text[idx + 1..];
                self.current_line_bytes = trailing.len();
                self.has_open_line = !trailing.is_empty();
            }
            None if !text.is_empty() => {
                self.current_line_bytes += text_bytes;
                self.has_open_line = true;
            }
            None => {}
        }
    }

    fn create_progress(&self) -> ShellCaptureProgress {
        let tail_truncation = truncate_tail(&self.tail_output, TruncationOptions::default());
        let total_lines = self.completed_lines + if self.has_open_line { 1 } else { 0 };
        let truncated = total_lines > DEFAULT_MAX_LINES || self.total_bytes > DEFAULT_MAX_BYTES;
        let truncated_by = if truncated {
            tail_truncation
                .truncated_by
                .or(if self.total_bytes > DEFAULT_MAX_BYTES {
                    Some(TruncationLimit::Bytes)
                } else {
                    Some(TruncationLimit::Lines)
                })
        } else {
            None
        };
        let truncation = TruncationResult {
            truncated,
            truncated_by,
            total_lines,
            total_bytes: self.total_bytes,
            ..tail_truncation
        };
        let output = if truncated {
            truncation.content.clone()
        } else {
            self.tail_output.clone()
        };
        ShellCaptureProgress {
            output,
            truncation,
            full_output_path: self.full_output_path.clone(),
            last_line_bytes: self.current_line_bytes,
            log_issue: self.log_issue.clone(),
        }
    }

    /// Count, log and report one sanitized chunk.
    async fn accept(
        &mut self,
        text: &str,
        env: &Arc<dyn ExecutionEnv>,
        max_log_bytes: usize,
        on_chunk: &mut Option<ChunkCallback<'_>>,
    ) {
        self.ingest_chunk(text);
        self.flush_log(env, max_log_bytes).await;
        self.trim_tail();
        if let Some(cb) = on_chunk.as_mut() {
            let snap = self.create_progress();
            cb(text, &|| snap.clone());
        }
    }

    fn stop_logging(&mut self, issue: LogFileIssue) {
        self.logging_stopped = true;
        self.unlogged.clear();
        if self.log_issue.is_none() {
            self.log_issue = Some(issue);
        }
    }

    /// Write pending output to the log, creating it on first need. A log is
    /// started once output crosses a display limit; after that every chunk
    /// is appended. Failures and the size cap stop logging and are recorded.
    /// Log I/O ignores the command's cancel token: a cancelled command still
    /// gets a complete log of what it printed.
    async fn flush_log(&mut self, env: &Arc<dyn ExecutionEnv>, max_log_bytes: usize) {
        let must_create = self.force_log && self.full_output_path.is_none();
        if self.logging_stopped || (self.unlogged.is_empty() && !must_create) {
            return;
        }
        if self.full_output_path.is_none() {
            if !self.force_log && !self.exceeds_display_limits() {
                return;
            }
            match env.create_temp_file("bash-", ".log", None).await {
                Ok(path) => self.full_output_path = Some(path),
                Err(e) => {
                    self.stop_logging(LogFileIssue::WriteFailed {
                        error: e.to_string(),
                    });
                    return;
                }
            }
        }
        let path = self
            .full_output_path
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let budget = max_log_bytes
            .saturating_sub(LOG_MARKER_RESERVE)
            .saturating_sub(self.logged_bytes);
        let pending = std::mem::take(&mut self.unlogged);
        let (data, capped) = if pending.len() > budget {
            (truncate_to_utf8_prefix(&pending, budget).to_string(), true)
        } else {
            (pending, false)
        };
        let mut payload = data;
        if capped {
            let marker = format!(
                "\n[log stopped at {}; later output was not written to this file]\n",
                crate::truncate::format_size(max_log_bytes)
            );
            let remaining = max_log_bytes.saturating_sub(self.logged_bytes);
            let marker = truncate_to_utf8_prefix(&marker, remaining.saturating_sub(payload.len()));
            payload.push_str(marker);
        }
        let written = payload.len();
        if payload.is_empty() {
            if capped {
                self.stop_logging(LogFileIssue::Truncated {
                    max_bytes: max_log_bytes,
                });
            }
            return;
        }
        if let Err(e) = env
            .append_file(&path, FileContent::Text(payload), None)
            .await
        {
            self.stop_logging(LogFileIssue::WriteFailed {
                error: e.to_string(),
            });
            return;
        }
        self.logged_bytes += written;
        if capped {
            self.stop_logging(LogFileIssue::Truncated {
                max_bytes: max_log_bytes,
            });
        }
    }
}

/// Longest prefix of `s` that fits in `max_bytes`, ending on a char boundary.
fn truncate_to_utf8_prefix(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Sanitize a chunk: drop C0 controls (except TAB/LF/CR) and interlinear
/// annotation chars (U+FFF9..U+FFFB), then strip all CR. Mirrors
/// `sanitizeBinaryOutput` + the per-chunk `.replace(/\r/g, "")`.
///
/// ANSI escape sequences are consumed whole rather than having just their
/// leading ESC byte (a C0 control) dropped: dropping only ESC left the
/// sequence body behind, so colored output (e.g. `vite` build logs) surfaced
/// as literal `[36m`-style garbage in the tool response.
fn sanitize_and_strip_cr(chunk: &str) -> String {
    let mut out = String::with_capacity(chunk.len());
    let mut chars = chunk.chars().peekable();
    while let Some(c) = chars.next() {
        let u = c as u32;
        if c == '\x1b' {
            skip_escape_sequence(&mut chars);
        } else if u == 0x09 || u == 0x0a || u == 0x0d {
            out.push(c);
        } else if u <= 0x1f {
            // other C0 controls → drop
        } else if (0xfff9..=0xfffb).contains(&u) {
            // interlinear annotation → drop
        } else {
            out.push(c);
        }
    }
    out.replace('\r', "")
}

/// Consume one ANSI escape sequence, starting just after the ESC byte the
/// caller already read. Handles CSI (`ESC [ … final`), OSC (`ESC ] … BEL |
/// `ESC \`), and charset designators (`ESC ( X`); any other escape swallows a
/// single following char.
fn skip_escape_sequence(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    match chars.peek() {
        Some('[') => {
            chars.next();
            // CSI: parameter/intermediate bytes, terminated by a final byte in
            // the 0x40..=0x7E range.
            while let Some(c) = chars.next() {
                if (0x40..=0x7e).contains(&(c as u32)) {
                    break;
                }
            }
        }
        Some(']') => {
            chars.next();
            // OSC: terminated by BEL (0x07) or ST (ESC \).
            while let Some(c) = chars.next() {
                if c == '\x07' {
                    break;
                }
                if c == '\x1b' {
                    if let Some('\\') = chars.peek() {
                        chars.next();
                    }
                    break;
                }
            }
        }
        Some('(') | Some(')') => {
            // Charset designator: ESC ( or ESC ) followed by one char.
            chars.next();
            chars.next();
        }
        Some(_) => {
            chars.next();
        }
        None => {}
    }
}

/// Mirrors `executeShellWithCapture`. Drives `env.exec` with on_stdout/on_stderr
/// callbacks that queue sanitized chunks, ingests them in order beside the
/// running command, and returns the terminal [`ShellCaptureResult`].
pub async fn execute_shell_with_capture(
    env: &Arc<dyn ExecutionEnv>,
    command: &str,
    options: ShellCaptureOptions<'_>,
) -> Result<ShellCaptureResult, ExecutionError> {
    let ShellCaptureOptions {
        shell,
        cwd,
        env: exec_env,
        inherit_env,
        timeout,
        cancel,
        mut on_chunk,
        return_execution_errors,
        max_log_bytes,
    } = options;
    let max_log_bytes = max_log_bytes.unwrap_or(MAX_LOG_FILE_BYTES);

    // Late callbacks (after `exec` settles) are ignored, as in the TS
    // `acceptingOutput` flag.
    let accepting = Arc::new(AtomicBool::new(true));
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let make_sink = |tx: tokio::sync::mpsc::UnboundedSender<String>, accepting: Arc<AtomicBool>| {
        Box::new(move |chunk: &str| {
            if !accepting.load(Ordering::Acquire) {
                return;
            }
            let text = sanitize_and_strip_cr(chunk);
            if !text.is_empty() {
                let _ = tx.send(text);
            }
        }) as Box<dyn FnMut(&str) + Send>
    };
    let exec_opts = ShellExecOptions {
        shell,
        cwd,
        env: exec_env,
        inherit_env,
        timeout,
        cancel,
        on_stdout: Some(make_sink(tx.clone(), accepting.clone())),
        on_stderr: Some(make_sink(tx, accepting.clone())),
        on_spawn: None,
        stdin: None,
    };

    let mut state = CaptureState::default();
    let exec_fut = env.exec(command, exec_opts);
    tokio::pin!(exec_fut);
    let exec_result = loop {
        tokio::select! {
            biased;
            Some(text) = rx.recv() => {
                state.accept(&text, env, max_log_bytes, &mut on_chunk).await;
            }
            result = &mut exec_fut => break result,
        }
    };
    accepting.store(false, Ordering::Release);
    // Drain chunks queued before `exec` settled.
    while let Ok(text) = rx.try_recv() {
        state.accept(&text, env, max_log_bytes, &mut on_chunk).await;
    }

    let ShellCaptureProgress {
        output,
        truncation,
        full_output_path,
        last_line_bytes,
        log_issue,
    } = state.create_progress();
    let truncated = truncation.truncated;
    let cancelled_token = cancel.map(|t| t.is_cancelled()).unwrap_or(false);
    let base = ShellCaptureResult {
        output,
        truncation,
        full_output_path,
        last_line_bytes,
        exit_code: None,
        cancelled: false,
        truncated,
        execution_error: None,
        log_issue,
    };
    match exec_result {
        Ok(out) => Ok(ShellCaptureResult {
            exit_code: if cancelled_token {
                None
            } else {
                Some(out.exit_code)
            },
            cancelled: cancelled_token,
            ..base
        }),
        Err(e) if e.code == ExecutionErrorCode::Aborted || cancelled_token => {
            Ok(ShellCaptureResult {
                cancelled: true,
                ..base
            })
        }
        Err(e) if return_execution_errors => Ok(ShellCaptureResult {
            execution_error: Some(e),
            ..base
        }),
        Err(e) => Err(e),
    }
}

/// A shell command running on its own task. Output arrives on `chunks` (raw,
/// in order) and the final result on `done`. Cancelling `cancel` kills the
/// process tree. Unlike [`execute_shell_with_capture`], the process is owned
/// by this handle rather than by an awaiting caller, so it can be handed from
/// a foreground tool call to the background registry without restarting.
pub struct LiveShell {
    pub chunks: tokio::sync::mpsc::UnboundedReceiver<String>,
    pub done: tokio::sync::oneshot::Receiver<Result<ShellOutput, ExecutionError>>,
    pub cancel: CancellationToken,
    /// The process (group) id once known. `None` for backends without one.
    pub pid: Arc<std::sync::Mutex<Option<u32>>>,
    /// Interactive stdin, when requested at spawn.
    pub stdin: Option<tokio::sync::mpsc::UnboundedSender<crate::env::StdinRequest>>,
}

/// What to run for [`spawn_live_shell`].
#[derive(Default)]
pub struct LiveShellSpec {
    pub command: String,
    pub cwd: Option<PathBuf>,
    pub env: Option<HashMap<String, String>>,
    pub inherit_env: bool,
    pub timeout: Option<f64>,
    /// Keep stdin open for `bash_input`.
    pub interactive_stdin: bool,
}

/// Start `spec` on a new task. Requires a Tokio runtime.
pub fn spawn_live_shell(env: Arc<dyn ExecutionEnv>, spec: LiveShellSpec) -> LiveShell {
    let (chunk_tx, chunks) = tokio::sync::mpsc::unbounded_channel::<String>();
    let (done_tx, done) = tokio::sync::oneshot::channel();
    let cancel = CancellationToken::new();
    let pid = Arc::new(std::sync::Mutex::new(None));
    let (stdin_tx, stdin_rx) = if spec.interactive_stdin {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        (Some(tx), Some(rx))
    } else {
        (None, None)
    };
    let task_cancel = cancel.clone();
    let task_pid = pid.clone();
    tokio::spawn(async move {
        let out_tx = chunk_tx.clone();
        let err_tx = chunk_tx;
        let options = ShellExecOptions {
            shell: None,
            cwd: spec.cwd,
            env: spec.env,
            inherit_env: spec.inherit_env,
            timeout: spec.timeout,
            cancel: Some(&task_cancel),
            on_stdout: Some(Box::new(move |chunk: &str| {
                let _ = out_tx.send(chunk.to_string());
            })),
            on_stderr: Some(Box::new(move |chunk: &str| {
                let _ = err_tx.send(chunk.to_string());
            })),
            on_spawn: Some(Box::new(move |id| {
                *task_pid.lock().unwrap_or_else(|e| e.into_inner()) = Some(id);
            })),
            stdin: stdin_rx,
        };
        let result = env.exec(&spec.command, options).await;
        let _ = done_tx.send(result);
    });
    LiveShell {
        chunks,
        done,
        cancel,
        pid,
        stdin: stdin_tx,
    }
}

/// Owned output accounting for a [`LiveShell`]: sanitization, counters, the
/// bounded tail and the full-output log, shared by the foreground bash tool
/// and the background registry so a released shell keeps one log.
pub struct OutputCapture {
    state: CaptureState,
    env: Arc<dyn ExecutionEnv>,
    max_log_bytes: usize,
}

impl OutputCapture {
    pub fn new(env: Arc<dyn ExecutionEnv>, max_log_bytes: Option<usize>) -> Self {
        Self {
            state: CaptureState::default(),
            env,
            max_log_bytes: max_log_bytes.unwrap_or(MAX_LOG_FILE_BYTES),
        }
    }

    /// Ingest one raw chunk; returns its sanitized text.
    pub async fn push(&mut self, raw: &str) -> String {
        let text = sanitize_and_strip_cr(raw);
        if !text.is_empty() {
            self.state
                .accept(&text, &self.env, self.max_log_bytes, &mut None)
                .await;
        }
        text
    }

    /// Start the log now, even below the display limits, and write all
    /// output so far. Mirrors `OutputAccumulator.persist()` for background
    /// adoption.
    pub async fn persist(&mut self) {
        self.state.force_log = true;
        self.state.flush_log(&self.env, self.max_log_bytes).await;
    }

    pub fn progress(&self) -> ShellCaptureProgress {
        self.state.create_progress()
    }

    pub fn log_path(&self) -> Option<PathBuf> {
        self.state.full_output_path.clone()
    }

    pub fn log_issue(&self) -> Option<LogFileIssue> {
        self.state.log_issue.clone()
    }

    /// The terminal result for an exec outcome, as
    /// [`execute_shell_with_capture`] would build it.
    pub fn finish(
        &self,
        exec: Result<ShellOutput, ExecutionError>,
        cancelled: bool,
    ) -> Result<ShellCaptureResult, ExecutionError> {
        let ShellCaptureProgress {
            output,
            truncation,
            full_output_path,
            last_line_bytes,
            log_issue,
        } = self.progress();
        let base = ShellCaptureResult {
            truncated: truncation.truncated,
            output,
            truncation,
            full_output_path,
            last_line_bytes,
            exit_code: None,
            cancelled: false,
            execution_error: None,
            log_issue,
        };
        match exec {
            Ok(out) => Ok(ShellCaptureResult {
                exit_code: if cancelled { None } else { Some(out.exit_code) },
                cancelled,
                ..base
            }),
            Err(e) if e.code == ExecutionErrorCode::Aborted || cancelled => {
                Ok(ShellCaptureResult {
                    cancelled: true,
                    ..base
                })
            }
            Err(e) => Ok(ShellCaptureResult {
                execution_error: Some(e),
                ..base
            }),
        }
    }
}

/// Re-export `format_size` for the bash tool's truncation footer messages.
pub use crate::truncate::format_size;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_controls_keeps_tab_lf() {
        let s = "a\x01b\tc\nd\x0be";
        let out = sanitize_and_strip_cr(&s);
        assert_eq!(out, "ab\tc\nde");
    }

    #[test]
    fn sanitize_strips_cr() {
        let s = "a\r\nb\rc";
        let out = sanitize_and_strip_cr(&s);
        assert_eq!(out, "a\nbc");
    }

    #[test]
    fn sanitize_strips_interlinear_annotation() {
        let s = "a\u{fff9}b\u{fffb}c";
        let out = sanitize_and_strip_cr(&s);
        assert_eq!(out, "abc");
    }

    #[test]
    fn sanitize_strips_full_ansi_sequences() {
        // Regression: dropping only the ESC byte used to leave `[36m`-style
        // literal garbage in the captured/output text.
        let s = "\x1b[36mvite v6.4.3\x1b[39m \x1b[32m\u{2713}\x1b[39m built\n";
        assert_eq!(sanitize_and_strip_cr(s), "vite v6.4.3 \u{2713} built\n");
        let s = "a\x1b[1m\x1b[2mX\x1b[22m\x1b[22mb";
        assert_eq!(sanitize_and_strip_cr(s), "aXb");
    }

    #[test]
    fn sanitize_strips_osc_sequences() {
        assert_eq!(sanitize_and_strip_cr("a\x1b]0;title\x07b"), "ab");
        assert_eq!(sanitize_and_strip_cr("a\x1b]8;;http://x\x1b\\b"), "ab");
    }

    fn in_memory(
        script: &str,
        stdout: String,
    ) -> (Arc<crate::InMemoryExecutionEnv>, Arc<dyn ExecutionEnv>) {
        let mem = Arc::new(crate::InMemoryExecutionEnv::with_cwd("/w".into()));
        let rt_mem = mem.clone();
        let script = script.to_string();
        futures::executor::block_on(async move {
            rt_mem
                .register_shell(&script, crate::ShellScript::success(stdout))
                .await;
        });
        let env: Arc<dyn ExecutionEnv> = mem.clone();
        (mem, env)
    }

    #[tokio::test]
    async fn capped_log_keeps_its_prefix_and_reports_the_issue() {
        let out: String = (1..=2000).map(|i| format!("{:<49}\n", i)).collect();
        let (mem, env) = in_memory("big", out.clone());
        let result = execute_shell_with_capture(
            &env,
            "big",
            ShellCaptureOptions {
                max_log_bytes: Some(4096),
                return_execution_errors: true,
                ..Default::default()
            },
        )
        .await
        .expect("capture");
        // 100 KB of output is over the 50 KB display limit, so a log exists.
        let path = result.full_output_path.expect("log path");
        let log = crate::FileSystem::read_text_file(&*mem, &path.to_string_lossy(), None)
            .await
            .expect("read");
        assert!(log.len() <= 4096, "{}", log.len());
        assert!(log.starts_with(&out[..100]));
        assert!(log.ends_with("later output was not written to this file]\n"));
        assert_eq!(
            result.log_issue,
            Some(LogFileIssue::Truncated { max_bytes: 4096 })
        );
        // The visible tail is unaffected by the log cap.
        assert!(result.output.ends_with(&format!("{:<49}", 2000)));
    }

    #[tokio::test]
    async fn capped_log_marker_never_exceeds_a_small_byte_limit() {
        let (mem, env) = in_memory("small-cap", "x".repeat(60_000));
        let result = execute_shell_with_capture(
            &env,
            "small-cap",
            ShellCaptureOptions {
                max_log_bytes: Some(16),
                return_execution_errors: true,
                ..Default::default()
            },
        )
        .await
        .expect("capture");
        let path = result.full_output_path.expect("log path");
        let log = crate::FileSystem::read_binary_file(&*mem, &path.to_string_lossy(), None)
            .await
            .expect("read");
        assert!(log.len() <= 16, "{}", log.len());
        assert_eq!(
            result.log_issue,
            Some(LogFileIssue::Truncated { max_bytes: 16 })
        );
    }

    #[tokio::test]
    async fn on_chunk_receives_the_sanitized_chunk() {
        let (_mem, env) = in_memory("color", "\x1b[32mok\x1b[0m\r\n".into());
        let seen = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let sink = seen.clone();
        let result = execute_shell_with_capture(
            &env,
            "color",
            ShellCaptureOptions {
                on_chunk: Some(Box::new(move |chunk, _| {
                    sink.lock().unwrap().push(chunk.to_string());
                })),
                ..Default::default()
            },
        )
        .await
        .expect("capture");
        assert_eq!(*seen.lock().unwrap(), vec!["ok\n".to_string()]);
        assert_eq!(result.output, "ok\n");
        assert!(result.full_output_path.is_none());
        assert!(result.log_issue.is_none());
    }

    #[test]
    fn log_issue_descriptions() {
        assert_eq!(
            LogFileIssue::Truncated {
                max_bytes: MAX_LOG_FILE_BYTES
            }
            .describe(),
            "log stopped at 64.0MB; later output is only in the retained tail"
        );
        assert!(LogFileIssue::WriteFailed {
            error: "disk full".into()
        }
        .describe()
        .starts_with("log write failed (disk full)"));
    }

    #[test]
    fn capture_state_ingest_and_progress() {
        let mut s = CaptureState::default();
        s.ingest_chunk("line1\nline2\n");
        s.ingest_chunk("line3");
        let p = s.create_progress();
        // 3 lines total (line1, line2, line3-open).
        assert_eq!(p.truncation.total_lines, 3);
        assert!(!p.truncation.truncated);
        assert_eq!(p.output, "line1\nline2\nline3");
    }
}
