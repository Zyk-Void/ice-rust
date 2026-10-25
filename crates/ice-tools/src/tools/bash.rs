//! Mirrors `packages/agent/src/harness/tools/bash.ts` — the built-in `bash`
//! `AgentTool`. Drives `execute_shell_with_capture`, streams throttled
//! progress, and converts ALL failures (non-zero exit, timeout, abort,
//! executionError) into `Err(AgentError)` — the agent loop encodes those into an
//! error `ToolResultMessage`.
//!
//! Throttle mirrors the coding-agent `BASH_UPDATE_THROTTLE_MS`: the first
//! output is emitted immediately, later output at most once per 100 ms through
//! one pending timer, and the final update always follows capture resolution.

use futures::future::BoxFuture;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use ice_agent::agent_tool::AgentTool;
use ice_agent::error::AgentError;
use ice_agent::types::{AgentToolResult, TextContentOrImage, ToolResultPartial};
use ice_ai::types::{ConstrainedSamplingConfig, ConstrainedStrictness, Tool};
use schemars::JsonSchema;
use serde::Deserialize;
use tokio_util::sync::CancellationToken;

use crate::env::ExecutionEnv;
use crate::error::{ExecutionError, ExecutionErrorCode};
use crate::shell_output::{
    execute_shell_with_capture, ChunkCallback, LogFileIssue, ShellCaptureOptions,
    ShellCaptureProgress, ShellCaptureResult,
};
use crate::tools::tool_context::ExecutionToolContext;
use crate::truncate::{format_size, TruncationResult, DEFAULT_MAX_BYTES, DEFAULT_MAX_LINES};

/// Throttle window for bash `on_update` flushes. Mirrors `BASH_UPDATE_THROTTLE_MS`.
const BASH_UPDATE_THROTTLE_MS: u64 = 100;

/// Input for the bash tool. Mirrors TS `BashToolInput`. `timeout` is in seconds.
#[derive(Debug, Clone, JsonSchema, Deserialize)]
pub struct BashInput {
    /// Bash command to execute.
    pub command: String,
    /// Timeout in seconds. Omit it or pass 0 for no timeout.
    #[serde(default)]
    pub timeout: Option<f64>,
    /// Short description of what the command does (5-10 words), shown to the user.
    #[serde(default)]
    pub description: Option<String>,
    /// Start the command as a background shell and return its id immediately.
    /// Read its output with bash_output and stop it with bash_kill.
    #[serde(default)]
    pub run_in_background: Option<bool>,
    /// Keep the background shell's stdin open so bash_input can write to it.
    /// Requires run_in_background: true.
    #[serde(default)]
    pub stdin: Option<bool>,
}

/// Largest accepted timeout: the 32-bit signed millisecond timer limit.
const MAX_TIMEOUT_MS: f64 = 2_147_483_647.0;

/// Structured details for the bash tool. Mirrors TS `BashToolDetails`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct BashToolDetails {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncation: Option<TruncationResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full_output_path: Option<String>,
    /// Present when the log at `full_output_path` is incomplete or missing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_issue: Option<LogFileIssue>,
}

/// Mutable execution description passed to the optional prepare hook.
#[derive(Debug, Clone)]
pub struct BashExecution {
    pub command: String,
    pub cwd: PathBuf,
    pub env: HashMap<String, String>,
    pub inherit_env: bool,
}

/// Async hook called immediately before shell execution, matching native Pi's
/// `BashPrepare` extension point.
pub type BashPrepare =
    Arc<dyn Fn(&mut BashExecution) -> BoxFuture<'static, Result<(), String>> + Send + Sync>;

/// Options for `create_bash_tool`. Mirrors TS `BashToolOptions`.
#[derive(Clone, Default)]
pub struct BashToolOptions {
    /// Prepended to the command (separated by `\n`) before execution.
    pub command_prefix: Option<String>,
    /// Fallback timeout in seconds applied when the model doesn't pass one.
    /// ice has no default, but a model that forgets `timeout` can hang the run
    /// forever, so icers injects this at the harness build site. A
    /// model-supplied timeout still wins; use `max_timeout` for a limit the
    /// model cannot raise.
    pub default_timeout: Option<f64>,
    /// Hard ceiling in seconds. Mirrors TS `maxTimeoutMs`: it replaces an
    /// omitted or longer requested timeout and leaves a shorter one unchanged,
    /// so a child agent cannot defeat its host's execution deadline.
    pub max_timeout: Option<f64>,
    /// Optional async hook that can adjust command, cwd, and environment.
    pub prepare: Option<BashPrepare>,
}

/// The built-in `bash` tool. Holds an `Arc<dyn ExecutionEnv>` (read-only view
/// suffices — bash does not touch the mutation queue).
pub struct BashTool {
    schema: Tool,
    env: Arc<dyn ExecutionEnv>,
    options: BashToolOptions,
}

impl BashTool {
    fn schema() -> Tool {
        let params = schemars::schema_for!(BashInput);
        Tool {
            name: "bash".to_string(),
            description: format!(
                "Execute a Bash command in the current working directory using Bash/POSIX syntax, even on Windows. PowerShell cmdlets (Get-ChildItem, Select-Object, Select-String) are not valid here; use ls, find, rg, and POSIX pipelines instead. Returns stdout and \
                 stderr. Output is truncated to last {DEFAULT_MAX_LINES} lines or {kb}KB \
                 (whichever is hit first). If truncated, full output is saved to a temp file. \
                 Optionally provide a timeout in seconds.",
                kb = DEFAULT_MAX_BYTES / 1024
            ),
            parameters: ice_ai::types::Schema::new(
                serde_json::to_value(params).unwrap_or_default(),
            ),
            constrained_sampling: Some(ConstrainedSamplingConfig::JsonSchema {
                strict: ConstrainedStrictness::Prefer,
            }),
        }
    }
}

/// Build the `bash` tool from a context. Mirrors TS `createBashTool`.
pub fn create_bash_tool(
    context: &ExecutionToolContext,
    options: Option<BashToolOptions>,
) -> Arc<dyn AgentTool> {
    Arc::new(BashTool {
        schema: BashTool::schema(),
        env: context.env().clone(),
        options: options.unwrap_or_default(),
    })
}

#[async_trait]
impl AgentTool for BashTool {
    fn schema(&self) -> &Tool {
        &self.schema
    }
    fn label(&self) -> &str {
        "bash"
    }

    async fn execute(
        &self,
        _tool_call_id: &str,
        params: serde_json::Value,
        signal: CancellationToken,
        on_update: Arc<dyn Fn(ToolResultPartial) + Send + Sync>,
    ) -> Result<AgentToolResult, AgentError> {
        let input: BashInput = serde_json::from_value(params)
            .map_err(|e| AgentError::Validation(format!("bash input invalid: {e}")))?;
        let requested = resolve_timeout(input.timeout)?;
        if input.stdin == Some(true) && input.run_in_background != Some(true) {
            return Err(AgentError::Tool(
                "stdin: true requires run_in_background: true.".into(),
            ));
        }
        if input.run_in_background == Some(true) {
            return Err(AgentError::Tool(
                "Background shells are not available here; run the command without run_in_background."
                    .into(),
            ));
        }
        let mut execution = BashExecution {
            command: match &self.options.command_prefix {
                Some(p) => format!("{p}\n{}", input.command),
                None => input.command,
            },
            cwd: self.env.cwd().to_path_buf(),
            env: HashMap::new(),
            inherit_env: true,
        };
        if let Some(prepare) = &self.options.prepare {
            prepare(&mut execution).await.map_err(AgentError::Tool)?;
        }
        let command = execution.command;
        let timeout = apply_timeout_limits(
            requested,
            self.options.default_timeout,
            self.options.max_timeout,
        );

        // Initial empty update (mirrors TS `onUpdate?.({ content: [], details: undefined })`).
        on_update(AgentToolResult {
            content: Vec::new(),
            details: serde_json::Value::Null,
            ..Default::default()
        });

        let throttle = Arc::new(UpdateThrottle::new(on_update.clone()));
        let throttle_for_cb = throttle.clone();
        let on_chunk: ChunkCallback<'_> = Box::new(
            move |_chunk, get_progress: &dyn Fn() -> ShellCaptureProgress| {
                throttle_for_cb.offer(Snapshot::from_progress(get_progress()));
            },
        );

        let cwd = execution.cwd;
        let env = self.env.clone();
        let capture_opts = ShellCaptureOptions {
            shell: None,
            cwd: Some(cwd),
            env: Some(execution.env),
            inherit_env: execution.inherit_env,
            timeout,
            cancel: Some(&signal),
            on_chunk: Some(on_chunk),
            return_execution_errors: true,
            max_log_bytes: None,
        };

        // execute_shell_with_capture wants `&Arc<dyn ExecutionEnv>`.
        let capture = execute_shell_with_capture(&env, &command, capture_opts).await;
        // Cancel any pending timer first so no stale partial update can follow
        // the final one.
        throttle.finish();
        let capture = capture.map_err(exec_err_to_agent)?;
        throttle.emit(Snapshot {
            output: capture.output.clone(),
            truncation: capture.truncated.then(|| capture.truncation.clone()),
            full_output_path: capture
                .full_output_path
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
        });

        // Build the terminal output text + details, applying truncation notices.
        let (output_text, details) = build_bash_output(&capture);
        let details_value = serde_json::to_value(details).unwrap_or(serde_json::Value::Null);

        // Resolution: cancelled → aborted; timeout → timeout; other exec error;
        // non-zero exit → Tool error. All become Err(AgentError).
        let append_status = |status: String| -> String {
            if output_text.is_empty() {
                status
            } else {
                format!("{output_text}\n\n{status}")
            }
        };
        if capture.cancelled {
            return Err(AgentError::Tool(append_status("Command aborted".into())));
        }
        if let Some(ee) = &capture.execution_error {
            if ee.code == ExecutionErrorCode::Timeout {
                // `timeout` is `Option<f64>` in seconds; `{timeout:?}` used to render
                // `Some(2.0)`, so the message read "timed out after Some(2.0) seconds".
                let status = match timeout {
                    Some(secs) => format!("Command timed out after {secs} seconds"),
                    None => "Command timed out".to_string(),
                };
                return Err(AgentError::Tool(append_status(status)));
            }
            return Err(AgentError::Tool(ee.to_string()));
        }
        if let Some(code) = capture.exit_code {
            if code != 0 {
                return Err(AgentError::Tool(append_status(format!(
                    "Command exited with code {code}"
                ))));
            }
        }
        let text = if output_text.is_empty() {
            "(no output)".to_string()
        } else {
            output_text
        };
        Ok(AgentToolResult {
            content: vec![TextContentOrImage::text(text)],
            details: details_value,
            ..Default::default()
        })
    }
}

/// One model-visible progress snapshot.
struct Snapshot {
    output: String,
    truncation: Option<TruncationResult>,
    full_output_path: Option<String>,
}

impl Snapshot {
    fn from_progress(progress: ShellCaptureProgress) -> Self {
        Self {
            truncation: progress
                .truncation
                .truncated
                .then(|| progress.truncation.clone()),
            full_output_path: progress
                .full_output_path
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
            output: progress.output,
        }
    }
}

/// Coalesces streaming updates to at most one per
/// [`BASH_UPDATE_THROTTLE_MS`], with an immediate first update and a single
/// pending timer. Updates are emitted while holding the lock, so a timer that
/// fires during `finish` cannot deliver a stale update after the final one.
struct UpdateThrottle {
    on_update: Arc<dyn Fn(ToolResultPartial) + Send + Sync>,
    inner: std::sync::Mutex<ThrottleInner>,
}

#[derive(Default)]
struct ThrottleInner {
    last_emit: Option<Instant>,
    pending: Option<Snapshot>,
    timer: Option<tokio::task::JoinHandle<()>>,
    finished: bool,
}

impl UpdateThrottle {
    fn new(on_update: Arc<dyn Fn(ToolResultPartial) + Send + Sync>) -> Self {
        Self {
            on_update,
            inner: std::sync::Mutex::new(ThrottleInner::default()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ThrottleInner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Record a new snapshot: emit now if the window has passed, otherwise
    /// keep it for the one pending timer.
    fn offer(self: &Arc<Self>, snapshot: Snapshot) {
        let window = Duration::from_millis(BASH_UPDATE_THROTTLE_MS);
        let mut inner = self.lock();
        if inner.finished {
            return;
        }
        let now = Instant::now();
        let due = inner
            .last_emit
            .map_or(true, |last| now.duration_since(last) >= window);
        if due && inner.timer.is_none() {
            inner.last_emit = Some(now);
            inner.pending = None;
            self.deliver(snapshot);
            return;
        }
        inner.pending = Some(snapshot);
        if inner.timer.is_none() {
            let wait = inner.last_emit.map_or(Duration::ZERO, |last| {
                window.saturating_sub(now.duration_since(last))
            });
            let this = self.clone();
            inner.timer = Some(tokio::spawn(async move {
                tokio::time::sleep(wait).await;
                let mut inner = this.lock();
                inner.timer = None;
                if inner.finished {
                    return;
                }
                if let Some(snapshot) = inner.pending.take() {
                    inner.last_emit = Some(Instant::now());
                    this.deliver(snapshot);
                }
            }));
        }
    }

    /// Stop streaming: drop the pending snapshot and cancel the timer.
    fn finish(&self) {
        let mut inner = self.lock();
        inner.finished = true;
        inner.pending = None;
        if let Some(timer) = inner.timer.take() {
            timer.abort();
        }
    }

    /// Emit the final snapshot (after `finish`).
    fn emit(&self, snapshot: Snapshot) {
        let _inner = self.lock();
        self.deliver(snapshot);
    }

    fn deliver(&self, snapshot: Snapshot) {
        let details = match (&snapshot.truncation, &snapshot.full_output_path) {
            (None, None) => serde_json::Value::Null,
            _ => serde_json::to_value(BashToolDetails {
                truncation: snapshot.truncation,
                full_output_path: snapshot.full_output_path,
                log_issue: None,
            })
            .unwrap_or(serde_json::Value::Null),
        };
        (self.on_update)(AgentToolResult {
            content: vec![TextContentOrImage::text(snapshot.output)],
            details,
            ..Default::default()
        });
    }
}

/// Build the terminal `(output_text, details)` from a capture, appending the
/// truncation continuation notice (mirrors the TS block).
fn build_bash_output(capture: &ShellCaptureResult) -> (String, BashToolDetails) {
    let mut output_text = capture.output.clone();
    let details = if capture.truncated {
        let t = &capture.truncation;
        let start_line = t.total_lines.saturating_sub(t.output_lines) + 1;
        let end_line = t.total_lines;
        let path_str = capture
            .full_output_path
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned());
        let full = path_str.as_deref().unwrap_or("unavailable");
        if t.last_line_partial {
            let last_line_size = format_size(capture.last_line_bytes);
            output_text.push_str(&format!(
                "\n\n[Showing last {} of line {end_line} (line is {last_line_size}). Full output: {full}]",
                format_size(t.output_bytes),
            ));
        } else if matches!(
            t.truncated_by,
            Some(crate::truncate::TruncationLimit::Lines)
        ) {
            output_text.push_str(&format!(
                "\n\n[Showing lines {start_line}-{end_line} of {}. Full output: {full}]",
                t.total_lines,
            ));
        } else {
            output_text.push_str(&format!(
                "\n\n[Showing lines {start_line}-{end_line} of {} ({} limit). Full output: {full}]",
                t.total_lines,
                format_size(DEFAULT_MAX_BYTES),
            ));
        }
        BashToolDetails {
            truncation: Some(capture.truncation.clone()),
            full_output_path: path_str,
            log_issue: capture.log_issue.clone(),
        }
    } else {
        BashToolDetails {
            truncation: None,
            full_output_path: None,
            log_issue: capture.log_issue.clone(),
        }
    };
    if let Some(issue) = &capture.log_issue {
        output_text.push_str(&format!("\n[Warning: full output {}]", issue.describe()));
    }
    (output_text, details)
}

/// Validate the model's timeout (seconds). Mirrors `resolveTimeoutMs`: an
/// omitted or zero timeout means "no requested timeout".
pub(crate) fn resolve_timeout(timeout: Option<f64>) -> Result<Option<f64>, AgentError> {
    let Some(secs) = timeout else {
        return Ok(None);
    };
    if secs == 0.0 {
        return Ok(None);
    }
    if !secs.is_finite() || secs < 0.0 {
        return Err(AgentError::Tool(
            "Invalid timeout: must be a finite number of seconds".into(),
        ));
    }
    if secs * 1000.0 > MAX_TIMEOUT_MS {
        return Err(AgentError::Tool(format!(
            "Invalid timeout: maximum is {} seconds",
            MAX_TIMEOUT_MS / 1000.0
        )));
    }
    Ok(Some(secs))
}

/// Combine the validated request with the host fallback and hard ceiling.
/// The fallback only fills an omitted request; the ceiling caps everything.
pub(crate) fn apply_timeout_limits(
    requested: Option<f64>,
    default_timeout: Option<f64>,
    max_timeout: Option<f64>,
) -> Option<f64> {
    let timeout = requested.or(default_timeout.filter(|t| *t > 0.0));
    match max_timeout.filter(|m| *m > 0.0) {
        Some(max) => Some(timeout.map_or(max, |t| t.min(max))),
        None => timeout,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_or_omitted_timeout_means_none() {
        assert_eq!(resolve_timeout(None).unwrap(), None);
        assert_eq!(resolve_timeout(Some(0.0)).unwrap(), None);
        assert_eq!(resolve_timeout(Some(1.5)).unwrap(), Some(1.5));
    }

    #[test]
    fn invalid_timeouts_are_rejected_with_ice_messages() {
        for bad in [-1.0, f64::NAN, f64::INFINITY] {
            let err = resolve_timeout(Some(bad)).unwrap_err().to_string();
            assert!(err.contains("must be a finite number of seconds"), "{err}");
        }
        let err = resolve_timeout(Some(2_147_484.0)).unwrap_err().to_string();
        assert!(err.contains("maximum is 2147483.647 seconds"), "{err}");
        assert!(resolve_timeout(Some(2_147_483.647)).is_ok());
    }

    #[test]
    fn max_timeout_is_a_ceiling_the_model_cannot_raise() {
        // Omitted request takes the ceiling; longer requests are capped;
        // shorter requests are kept.
        assert_eq!(apply_timeout_limits(None, None, Some(30.0)), Some(30.0));
        assert_eq!(
            apply_timeout_limits(Some(600.0), None, Some(30.0)),
            Some(30.0)
        );
        assert_eq!(apply_timeout_limits(Some(5.0), None, Some(30.0)), Some(5.0));
        // The fallback fills an omitted request but is still capped.
        assert_eq!(
            apply_timeout_limits(None, Some(120.0), Some(30.0)),
            Some(30.0)
        );
        assert_eq!(apply_timeout_limits(None, Some(120.0), None), Some(120.0));
        assert_eq!(
            apply_timeout_limits(Some(600.0), Some(120.0), None),
            Some(600.0)
        );
        // Zero host values mean unset, like TS `maxTimeoutMs || undefined`.
        assert_eq!(apply_timeout_limits(None, Some(0.0), Some(0.0)), None);
    }
}

fn exec_err_to_agent(e: ExecutionError) -> AgentError {
    AgentError::Tool(e.to_string())
}
