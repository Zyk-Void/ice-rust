//! Mirrors `packages/coding-agent/src/core/tools/bash-shells.ts` — the
//! model-facing controls for background shells: `bash_output` (list, read
//! new output, wait, tail or grep the log), `bash_kill` and `bash_input`.
//! All three share the session's [`BashBackgroundRegistry`] with `bash`.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::future::BoxFuture;
use ice_agent::agent_tool::AgentTool;
use ice_agent::error::AgentError;
use ice_agent::types::{AgentToolResult, TextContentOrImage, ToolResultPartial};
use ice_ai::types::Tool;
use schemars::JsonSchema;
use serde::Deserialize;
use tokio_util::sync::CancellationToken;

use crate::bash_background::{
    format_runtime, summarize_shell_command, BackgroundShellSnapshot, BackgroundShellStatus,
    BashBackgroundRegistry, KillOutcome, SteerProbe, WaitUntil,
};
use crate::truncate::{
    format_size, truncate_tail, TruncationOptions, DEFAULT_MAX_BYTES, DEFAULT_MAX_LINES,
};

/// Names of the background-shell control tools. Mirrors `BASH_SHELL_TOOL_NAMES`.
pub const BASH_SHELL_TOOL_NAMES: [&str; 3] = ["bash_output", "bash_kill", "bash_input"];
/// Longest accepted wait. Mirrors `MAX_WAIT_SECONDS`.
pub const MAX_WAIT_SECONDS: f64 = 300.0;
/// Default number of grep matches returned. Mirrors the TS default of 100.
const DEFAULT_GREP_LINES: usize = 100;
/// Longest accepted grep pattern (characters). Mirrors the 500-unit limit.
const MAX_GREP_CHARS: usize = 500;
/// Characters of input text quoted in the approval dialog.
const APPROVAL_PREVIEW_CHARS: usize = 400;

fn approval_preview(text: &str) -> (String, bool) {
    let mut chars = text.chars();
    let clipped: String = chars.by_ref().take(APPROVAL_PREVIEW_CHARS).collect();
    let truncated = chars.next().is_some();
    (clipped, truncated)
}

/// Input for `bash_output`. Mirrors `BashOutputInput`.
#[derive(Debug, Clone, JsonSchema, Deserialize)]
pub struct BashOutputInput {
    /// Background shell id (for example b4k2x9m1q). Omit to list all shells.
    #[serde(default)]
    pub id: Option<String>,
    /// Seconds to wait for the shell to exit (or, with until "output", for new
    /// output) before returning. 0 returns at once; at most 300.
    #[serde(default)]
    pub wait_seconds: Option<f64>,
    /// What the wait ends on: "exit" (default) or "output".
    #[serde(default)]
    pub until: Option<UntilArg>,
    /// Show the last N lines of the full log instead of new output.
    #[serde(default)]
    pub tail_lines: Option<f64>,
    /// Show only full-log lines matching this regular expression (newest
    /// first kept; 100 lines unless tail_lines is set).
    #[serde(default)]
    pub grep: Option<String>,
}

#[derive(Debug, Clone, Copy, JsonSchema, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum UntilArg {
    Exit,
    Output,
}

/// Input for `bash_kill`. Mirrors `BashKillInput`.
#[derive(Debug, Clone, JsonSchema, Deserialize)]
pub struct BashKillInput {
    /// Background shell id to stop (for example b4k2x9m1q).
    pub id: String,
}

/// Input for `bash_input`. Mirrors `BashInputInput`.
#[derive(Debug, Clone, JsonSchema, Deserialize)]
pub struct BashInputInput {
    /// Background shell id (started with stdin: true).
    pub id: String,
    /// Text to send exactly as given; no newline is added.
    #[serde(default)]
    pub text: Option<String>,
    /// Close stdin after sending (EOF).
    #[serde(default)]
    pub eof: Option<bool>,
}

/// Whether `bash_input` needs the user's confirmation. Mirrors
/// `BashInputApproval` (`bashInputApproval` setting).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BashInputApproval {
    #[default]
    Ask,
    Allow,
}

/// Asks the user to confirm; `(title, body) -> approved`. `None` means no
/// interactive UI is available.
pub type ConfirmFn = Arc<dyn Fn(String, String) -> BoxFuture<'static, bool> + Send + Sync>;

/// Options shared by the control tools.
#[derive(Clone, Default)]
pub struct BashShellToolOptions {
    /// Reports a queued steering message so waits return early.
    pub steer_probe: Option<SteerProbe>,
    /// Approval policy for `bash_input`.
    pub input_approval: BashInputApproval,
    /// UI confirmation for `bash_input` when approval is `Ask`.
    pub confirm: Option<ConfirmFn>,
}

fn tool(name: &str, description: &str, schema: schemars::schema::RootSchema) -> Tool {
    Tool {
        name: name.to_string(),
        description: description.to_string(),
        parameters: ice_ai::types::Schema::new(serde_json::to_value(schema).unwrap_or_default()),
        constrained_sampling: None,
    }
}

fn text_result(text: String, details: serde_json::Value) -> AgentToolResult {
    AgentToolResult {
        content: vec![TextContentOrImage::text(text)],
        details,
        ..Default::default()
    }
}

/// Build `bash_output`, `bash_kill` and `bash_input` for one registry.
pub fn create_bash_shell_tools(
    registry: Arc<BashBackgroundRegistry>,
    options: BashShellToolOptions,
) -> Vec<Arc<dyn AgentTool>> {
    vec![
        Arc::new(BashOutputTool {
            schema: tool(
                "bash_output",
                "Read a background shell started by bash (run_in_background, or moved to the background). With no id, list all shells. With an id, return only output not shown before; set wait_seconds to wait for the shell to exit (or, with until \"output\", for new output); a new user message ends the wait early. Use tail_lines or grep to inspect the full log without consuming new output.",
                schemars::schema_for!(BashOutputInput),
            ),
            registry: registry.clone(),
            options: options.clone(),
        }),
        Arc::new(BashKillTool {
            schema: tool(
                "bash_kill",
                "Stop a background shell (and any processes it left running) by id.",
                schemars::schema_for!(BashKillInput),
            ),
            registry: registry.clone(),
        }),
        Arc::new(BashInputTool {
            schema: tool(
                "bash_input",
                "Send text to the stdin of a background shell started with bash run_in_background: true and stdin: true. Text is sent exactly as given (add \\n yourself); eof: true closes stdin. Do not resend input reported as pending.",
                schemars::schema_for!(BashInputInput),
            ),
            registry,
            options,
        }),
    ]
}

/// One status line for a shell: `<id>: <status> (<runtime>)` plus warnings
/// and the first command line.
fn status_line(shell: &BackgroundShellSnapshot) -> String {
    let mut line = format!(
        "{}: {} ({})",
        shell.task_id,
        shell.status.as_str(),
        format_runtime(shell.runtime)
    );
    if shell.lingering {
        line.push_str(" [processes still running; stop them with bash_kill]");
    }
    if shell.stdin_open {
        line.push_str(" [stdin open]");
    }
    line.push_str(&format!(" {}", summarize_shell_command(&shell.command)));
    if let Some(description) = &shell.description {
        line.push_str(&format!(" — {description}"));
    }
    line
}

fn shells_details(shells: &[BackgroundShellSnapshot]) -> serde_json::Value {
    serde_json::json!({
        "shells": shells.iter().map(|s| s.status.as_str()).collect::<Vec<_>>(),
        "shell_ids": shells.iter().map(|s| s.task_id.clone()).collect::<Vec<_>>(),
    })
}

fn tail(text: &str) -> String {
    let t = truncate_tail(
        text,
        TruncationOptions {
            max_lines: Some(DEFAULT_MAX_LINES),
            max_bytes: Some(DEFAULT_MAX_BYTES),
        },
    );
    if t.truncated {
        format!(
            "[Showing the last {} lines ({}).]\n{}",
            t.output_lines,
            format_size(t.output_bytes),
            t.content
        )
    } else {
        t.content
    }
}

/// Rejects a quantified group followed by another quantifier, e.g.
/// `(a+)+`. Mirrors ice's `NESTED_QUANTIFIER` heuristic. Rust's regex engine
/// cannot backtrack catastrophically, so this only keeps ice's error.
fn has_nested_quantifier(pattern: &str) -> bool {
    let re =
        regex::Regex::new(r"\((?:[^()\\]|\\.)*[+*}](?:[^()\\]|\\.)*\)\s*(?:[+*]|\{\d*,?\d*\})")
            .expect("valid heuristic");
    re.is_match(pattern)
}

struct BashOutputTool {
    schema: Tool,
    registry: Arc<BashBackgroundRegistry>,
    options: BashShellToolOptions,
}

#[async_trait]
impl AgentTool for BashOutputTool {
    fn schema(&self) -> &Tool {
        &self.schema
    }
    fn label(&self) -> &str {
        "bash_output"
    }

    async fn execute(
        &self,
        _tool_call_id: &str,
        params: serde_json::Value,
        signal: CancellationToken,
        _on_update: Arc<dyn Fn(ToolResultPartial) + Send + Sync>,
    ) -> Result<AgentToolResult, AgentError> {
        let input: BashOutputInput = serde_json::from_value(params)
            .map_err(|e| AgentError::Validation(format!("bash_output input invalid: {e}")))?;
        // Listing ignores the other fields, as in ice.
        let Some(id) = input.id.filter(|id| !id.is_empty()) else {
            let shells = self.registry.snapshots();
            if shells.is_empty() {
                return Ok(text_result(
                    "No background shells.".into(),
                    shells_details(&[]),
                ));
            }
            let text = shells
                .iter()
                .map(status_line)
                .collect::<Vec<_>>()
                .join("\n");
            return Ok(text_result(text, shells_details(&shells)));
        };
        let wait = match input.wait_seconds {
            None => 0.0,
            Some(w) if w.is_finite() && w >= 0.0 => w.min(MAX_WAIT_SECONDS),
            Some(_) => {
                return Err(AgentError::Tool(
                    "wait_seconds must be a non-negative number".into(),
                ))
            }
        };
        let tail_lines = match input.tail_lines {
            None => None,
            Some(n) if n.is_finite() && n >= 1.0 && n.fract() == 0.0 => Some(n as usize),
            Some(_) => {
                return Err(AgentError::Tool(
                    "tail_lines must be a positive integer".into(),
                ))
            }
        };
        let grep = match input.grep.filter(|g| !g.is_empty()) {
            None => None,
            Some(pattern) => {
                let chars = pattern.chars().count();
                if chars > MAX_GREP_CHARS {
                    return Err(AgentError::Tool(format!(
                        "grep is {chars} characters; at most {MAX_GREP_CHARS}."
                    )));
                }
                if has_nested_quantifier(&pattern) {
                    return Err(AgentError::Tool(
                        "grep has a quantified group followed by another quantifier; simplify the pattern.".into(),
                    ));
                }
                Some(regex::Regex::new(&pattern).map_err(|e| {
                    AgentError::Tool(format!("grep is not a valid regular expression: {e}"))
                })?)
            }
        };
        let until = match input.until {
            Some(UntilArg::Output) => WaitUntil::Output,
            _ => WaitUntil::Exit,
        };
        // Counted until the result is built, so a completion notice for a
        // shell that ends meanwhile waits to see whether this call returns
        // its final result.
        let _reading = self.registry.reading(&id);

        // Wait (and consume new output) only for the incremental mode; log
        // queries leave the read cursor alone.
        let log_query = tail_lines.is_some() || grep.is_some();
        let read = if log_query {
            // Waiting still applies, but the read cursor is left alone.
            self.registry
                .wait(
                    &id,
                    Duration::from_secs_f64(wait),
                    until,
                    &signal,
                    self.options.steer_probe.as_ref(),
                )
                .await
                .map_err(AgentError::Tool)?;
            None
        } else {
            Some(
                self.registry
                    .read(
                        &id,
                        Duration::from_secs_f64(wait),
                        until,
                        &signal,
                        self.options.steer_probe.as_ref(),
                    )
                    .await
                    .map_err(AgentError::Tool)?,
            )
        };

        let mut parts: Vec<String> = Vec::new();
        let shell = match &read {
            Some(read) => read.shell.clone(),
            None => self
                .registry
                .get(&id)
                .ok_or_else(|| AgentError::Tool(self.registry.unknown_shell_error(&id)))?,
        };
        parts.push(status_line(&shell));
        let still_running = shell.status == BackgroundShellStatus::Running;
        if still_running
            && read.is_some()
            && self
                .options
                .steer_probe
                .as_ref()
                .is_some_and(|probe| probe())
        {
            parts.push(
                "[Stopped waiting because a message arrived (from the user, a finished background shell, or an alert); read it now. This shell keeps running, and you are told when it finishes.]"
                    .into(),
            );
        }
        if let Some(path) = &shell.log_path {
            parts.push(format!("Full log: {path}"));
        }
        if let Some(issue) = &shell.log_issue {
            parts.push(format!("[Warning: full output {issue}]"));
        }

        if log_query {
            let log = self
                .registry
                .read_log(
                    &id,
                    tail_lines.unwrap_or(if grep.is_some() {
                        DEFAULT_GREP_LINES
                    } else {
                        1
                    }),
                    grep.as_ref(),
                )
                .await
                .map_err(AgentError::Tool)?;
            let source = if shell.log_path.is_some() {
                "log"
            } else {
                "retained output"
            };
            match log.matched {
                Some(matched) => parts.push(format!(
                    "{} of {matched} matching lines from the {source}:",
                    log.lines.len()
                )),
                None => parts.push(format!("Last {} lines from the {source}:", log.lines.len())),
            }
            if log.skipped_bytes > 0 {
                parts.push(format!(
                    "[{:.1} MiB of older output was not searched.]",
                    log.skipped_bytes as f64 / (1024.0 * 1024.0)
                ));
            }
            parts.push(tail(&log.lines.join("\n")));
        } else if let Some(read) = &read {
            if read.omitted_chars > 0 {
                parts.push(format!(
                    "[{} earlier characters of new output were dropped from memory; see the full log.]",
                    read.omitted_chars
                ));
            }
            if read.new_output.is_empty() {
                parts.push("(no new output)".into());
            } else {
                parts.push(tail(&read.new_output));
            }
        }
        if !still_running {
            if let Some(final_text) = &shell.final_text {
                parts.push(format!("Final result:\n{}", tail(final_text)));
                self.registry.mark_result_delivered(&id);
            }
        }
        Ok(text_result(parts.join("\n"), shells_details(&[shell])))
    }
}

struct BashKillTool {
    schema: Tool,
    registry: Arc<BashBackgroundRegistry>,
}

#[async_trait]
impl AgentTool for BashKillTool {
    fn schema(&self) -> &Tool {
        &self.schema
    }
    fn label(&self) -> &str {
        "bash_kill"
    }

    async fn execute(
        &self,
        _tool_call_id: &str,
        params: serde_json::Value,
        _signal: CancellationToken,
        _on_update: Arc<dyn Fn(ToolResultPartial) + Send + Sync>,
    ) -> Result<AgentToolResult, AgentError> {
        let input: BashKillInput = serde_json::from_value(params)
            .map_err(|e| AgentError::Validation(format!("bash_kill input invalid: {e}")))?;
        let text = match self
            .registry
            .kill(&input.id, false)
            .await
            .map_err(AgentError::Tool)?
        {
            KillOutcome::Stopped => format!("Stopped {}.", input.id),
            KillOutcome::StoppedLingering => {
                format!("Stopped the processes {} left running.", input.id)
            }
            KillOutcome::AlreadyFinished(status) => {
                format!("{} already {}.", input.id, status.as_str())
            }
        };
        Ok(text_result(text, serde_json::Value::Null))
    }
}

struct BashInputTool {
    schema: Tool,
    registry: Arc<BashBackgroundRegistry>,
    options: BashShellToolOptions,
}

#[async_trait]
impl AgentTool for BashInputTool {
    fn schema(&self) -> &Tool {
        &self.schema
    }
    fn label(&self) -> &str {
        "bash_input"
    }

    async fn execute(
        &self,
        _tool_call_id: &str,
        params: serde_json::Value,
        _signal: CancellationToken,
        _on_update: Arc<dyn Fn(ToolResultPartial) + Send + Sync>,
    ) -> Result<AgentToolResult, AgentError> {
        let input: BashInputInput = serde_json::from_value(params)
            .map_err(|e| AgentError::Validation(format!("bash_input input invalid: {e}")))?;
        let text = input.text.filter(|t| !t.is_empty());
        let eof = input.eof == Some(true);
        if text.is_none() && !eof {
            return Err(AgentError::Tool(
                "bash_input needs text to send or eof: true.".into(),
            ));
        }
        // Resolve the shell before asking for approval.
        let shell = self
            .registry
            .get(&input.id)
            .ok_or_else(|| AgentError::Tool(self.registry.unknown_shell_error(&input.id)))?;
        if self.options.input_approval != BashInputApproval::Allow {
            let Some(confirm) = &self.options.confirm else {
                return Err(AgentError::Tool(
                    "bash_input needs the user's approval and no interactive UI is available. Set bashInputApproval to allow, or re-run the command non-interactively.".into(),
                ));
            };
            let preview = text.as_deref().map_or_else(String::new, |t| {
                let (clipped, truncated) = approval_preview(t);
                let ellipsis = if truncated { "…" } else { "" };
                format!(
                    "Text: {}{ellipsis}",
                    serde_json::to_string(&clipped).unwrap_or_default()
                )
            });
            let body = format!(
                "Command: {}\n{preview}\nClose stdin (EOF): {}",
                summarize_shell_command(&shell.command),
                if eof { "yes" } else { "no" }
            );
            if !confirm(format!("Send input to {}?", input.id), body).await {
                return Err(AgentError::Tool(
                    "The user declined to send this input.".into(),
                ));
            }
        }
        let written = self
            .registry
            .write_input(&input.id, text.as_deref(), eof)
            .await
            .map_err(AgentError::Tool)?;
        let text = if written.pending {
            format!(
                "Sent {} bytes to {}, but its stdin stayed full for 5 seconds; the input is still pending. Do not resend it. Check progress with bash_output.",
                written.bytes, input.id
            )
        } else if eof {
            format!(
                "Sent {} bytes to {} and closed stdin.",
                written.bytes, input.id
            )
        } else {
            format!("Sent {} bytes to {}.", written.bytes, input.id)
        };
        Ok(text_result(
            text,
            serde_json::json!({ "bytes": written.bytes }),
        ))
    }
}

#[cfg(test)]
mod preview_tests {
    use super::*;

    #[test]
    fn approval_preview_measures_characters_not_utf8_bytes() {
        let included = "é".repeat(APPROVAL_PREVIEW_CHARS);
        let (preview, truncated) = approval_preview(&included);
        assert_eq!(preview.chars().count(), APPROVAL_PREVIEW_CHARS);
        assert!(!truncated);

        let beyond = format!("{included}x");
        let (_, truncated) = approval_preview(&beyond);
        assert!(truncated);
    }
}
