//! Mirrors `packages/coding-agent/src/core/tools/grep.ts` — the built-in `grep`
//! `AgentTool`. Searches file contents for a regex/literal pattern and returns
//! matching lines with `file:line: text` (and optional `-line-` context lines).
//! Read-only — goes through the `FileSystem` trait so it runs against both
//! `OsExecutionEnv` and `InMemoryExecutionEnv`.
//!
//! Text search uses ripgrep on hosts supporting direct program execution;
//! archive and structural modes use ugrep and ast-grep respectively. The
//! in-process implementation remains available for virtual environments and
//! when ripgrep is not installed.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use ice_agent::agent_tool::AgentTool;
use ice_agent::error::AgentError;
use ice_agent::types::{AgentToolResult, TextContentOrImage, ToolResultPartial};
use ice_ai::types::Tool;
use schemars::JsonSchema;
use serde::Deserialize;
use tokio_util::sync::CancellationToken;

use crate::env::FileKind;
use crate::env::{ExecutionEnv, ShellExecOptions};
use crate::error::FileErrorCode;
use crate::ignore_rules::{is_vcs_dir, IgnoreStack, IgnoreTool};
use crate::path_utils::resolve_tool_path;
use crate::tools::tool_context::ExecutionToolContext;
use crate::truncate::{
    format_size, truncate_head, truncate_line_default, TruncationOptions, TruncationResult,
    DEFAULT_MAX_BYTES, GREP_MAX_LINE_LENGTH,
};

/// Default max matches. Mirrors TS `grep.ts::DEFAULT_LIMIT`.
const DEFAULT_LIMIT: u32 = 100;

/// Input for the grep tool. Mirrors TS `GrepToolInput`. `camelCase` field names
/// match the TS wire schema (so `ignoreCase`/`ignore_case` deserialization lines
/// up with what the model emits).
#[derive(Debug, Clone, JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrepInput {
    /// Search pattern (regex or literal string).
    pub pattern: String,
    /// Directory or file to search, or a list of them (default: current directory).
    #[serde(default)]
    pub path: Option<SearchPaths>,
    /// Filter files by glob pattern, e.g. `*.ts` or `**/*.spec.ts`.
    #[serde(default)]
    pub glob: Option<String>,
    /// Case-insensitive search (default false).
    #[serde(default)]
    pub ignore_case: Option<bool>,
    /// Treat pattern as a literal string instead of regex (default false).
    #[serde(default)]
    pub literal: Option<bool>,
    /// Number of lines to show before and after each match (default 0).
    #[serde(default)]
    pub context: Option<u32>,
    /// Lines to show before each match; overrides `context` on that side.
    #[serde(default)]
    pub before: Option<u32>,
    /// Lines to show after each match; overrides `context` on that side.
    #[serde(default)]
    pub after: Option<u32>,
    /// Maximum number of matches to return (default 100).
    #[serde(default)]
    pub limit: Option<u32>,
    /// Search mode: text (ripgrep), archive (ugrep), or ast (ast-grep).
    #[serde(default)]
    pub mode: GrepMode,
    /// Required language for ast mode, e.g. rust or typescript.
    #[serde(default)]
    pub language: Option<String>,
}

/// One search path or several; agents commonly search a few files at once.
#[derive(Debug, Clone, JsonSchema, Deserialize)]
#[serde(untagged)]
pub enum SearchPaths {
    One(String),
    Many(Vec<String>),
}

impl GrepInput {
    fn paths(&self) -> Result<Vec<&str>, AgentError> {
        match &self.path {
            None => Ok(vec!["."]),
            Some(SearchPaths::One(path)) => Ok(vec![path.as_str()]),
            Some(SearchPaths::Many(paths)) if paths.is_empty() => {
                Err(AgentError::Validation("path list must not be empty".into()))
            }
            Some(SearchPaths::Many(paths)) => Ok(paths.iter().map(String::as_str).collect()),
        }
    }

    /// Context lines `(before, after)`; `before`/`after` override `context`.
    fn context_window(&self) -> (usize, usize) {
        let context = self.context.unwrap_or(0);
        (
            self.before.unwrap_or(context) as usize,
            self.after.unwrap_or(context) as usize,
        )
    }
}

/// Search engine selection is explicit because text and syntax searches differ.
#[derive(Debug, Clone, Copy, Default, JsonSchema, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum GrepMode {
    #[default]
    Text,
    Archive,
    Ast,
}

/// Structured details for the grep tool. Mirrors TS `GrepToolDetails`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct GrepToolDetails {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncation: Option<TruncationResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub match_limit_reached: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lines_truncated: Option<bool>,
}

/// Options for `create_grep_tool`. The TS tool carries an optional `operations`
/// bag for remote backends; the Rust port always uses the `FileSystem` trait, so
/// this is an empty forward-compat seam (parity with `ReadToolOptions`).
#[derive(Debug, Clone, Default)]
pub struct GrepToolOptions {}

/// The built-in `grep` tool. Holds an `Arc<dyn ExecutionEnv>` (read-only view).
pub struct GrepTool {
    schema: Tool,
    env: Arc<dyn ExecutionEnv>,
}

enum ExternalError {
    Unavailable(&'static str),
    Failed(AgentError),
}

impl GrepTool {
    fn schema() -> Tool {
        let params = schemars::schema_for!(GrepInput);
        Tool {
            name: "grep".to_string(),
            description: format!(
                "Search files with mode text (default, ripgrep), archive (ugrep, searches \
                 compressed files and archives), or ast (ast-grep, structural pattern and \
                 required language). Use ast only for syntax patterns, not arbitrary text. \
                 Respects ignore files and skips `.git/`. Output is limited to {DEFAULT_LIMIT} \
                 matches by default or {kb}KB; long lines are truncated to \
                 {GREP_MAX_LINE_LENGTH} chars. External engines must be installed.",
                kb = DEFAULT_MAX_BYTES / 1024
            ),
            parameters: ice_ai::types::Schema::new(
                serde_json::to_value(params).unwrap_or_default(),
            ),
            constrained_sampling: None,
        }
    }

    /// Resolve every search path before any engine runs, so a missing path
    /// fails the call the same way in every engine.
    async fn resolve_roots(
        &self,
        input: &GrepInput,
        signal: &CancellationToken,
    ) -> Result<Vec<(String, Option<FileKind>)>, AgentError> {
        let cancel = Some(signal);
        let mut roots = Vec::new();
        for path in input.paths()? {
            // A `.` path resolves to the env cwd.
            let search_path = resolve_tool_path(&*self.env, path, cancel)
                .await
                .map_err(file_err_to_agent)?;
            // Follow a symlinked root via canonical_path. A special file (a
            // FIFO, a device) has no kind; any other failure to stat the root
            // is "Path not found", as in ice.
            let info = match self.env.file_info(&search_path, cancel).await {
                Ok(info) => info,
                Err(e) if e.code == FileErrorCode::Aborted => return Err(file_err_to_agent(e)),
                Err(e) if e.code == FileErrorCode::Invalid => {
                    roots.push((search_path, None));
                    continue;
                }
                Err(_) => return Err(AgentError::Tool(format!("Path not found: {search_path}"))),
            };
            let kind = if info.kind == FileKind::Symlink {
                match self.env.canonical_path(&search_path, cancel).await {
                    Ok(canon) => match self.env.file_info(&canon.to_string_lossy(), cancel).await {
                        Ok(i) => i.kind,
                        Err(_) => info.kind, // treat as whatever lstat said
                    },
                    Err(_) => info.kind,
                }
            } else {
                info.kind
            };
            roots.push((search_path, Some(kind)));
        }
        Ok(roots)
    }

    async fn search_external(
        &self,
        input: &GrepInput,
        roots: &[(String, Option<FileKind>)],
        signal: &CancellationToken,
    ) -> Result<AgentToolResult, ExternalError> {
        let program = match input.mode {
            GrepMode::Text => "rg",
            GrepMode::Archive => "ugrep",
            GrepMode::Ast => "ast-grep",
        };
        if !self.env.supports_exec_program() {
            return Err(ExternalError::Unavailable(program));
        }
        if input.mode != GrepMode::Ast && input.language.is_some() {
            return Err(ExternalError::Failed(AgentError::Validation(
                "language is only supported in ast mode".into(),
            )));
        }
        let (before, after) = input.context_window();
        if input.mode == GrepMode::Ast
            && (input.literal.unwrap_or(false)
                || input.ignore_case.unwrap_or(false)
                || before != 0
                || after != 0)
        {
            return Err(ExternalError::Failed(AgentError::Validation(
                "literal, ignoreCase, and context are not supported in ast mode".into(),
            )));
        }

        let cwd = self.env.cwd();
        let mut args: Vec<String> = match input.mode {
            GrepMode::Text => [
                "--no-config",
                "--hidden",
                "--glob",
                "!.git",
                "--color",
                "never",
                "--no-heading",
                "--line-number",
                "--with-filename",
                // NUL after the path makes match and context lines unambiguous.
                "--null",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
            GrepMode::Archive => [
                "--no-config",
                "-r",
                "-z",
                "-n",
                "-H",
                "-I",
                "--hidden",
                "--ignore-files=.gitignore",
                "--ignore-files=.ignore",
                "--ignore-files=.rgignore",
                "--exclude-dir=.git",
                "--color=never",
                "--null",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
            GrepMode::Ast => {
                let language = input
                    .language
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| {
                        ExternalError::Failed(AgentError::Validation(
                            "ast mode requires language".into(),
                        ))
                    })?;
                vec![
                    "run".into(),
                    "--pattern".into(),
                    input.pattern.clone(),
                    "--lang".into(),
                    language.into(),
                    "--json=stream".into(),
                    "--no-ignore".into(),
                    "hidden".into(),
                ]
            }
        };
        if let Some(glob) = &input.glob {
            args.push(match input.mode {
                GrepMode::Text => "--glob".into(),
                GrepMode::Archive => "--glob".into(),
                GrepMode::Ast => "--globs".into(),
            });
            args.push(glob.clone());
        }
        if input.mode != GrepMode::Ast {
            if input.literal.unwrap_or(false) {
                args.push("--fixed-strings".into());
            }
            if input.ignore_case.unwrap_or(false) {
                args.push("--ignore-case".into());
            }
            for (flag, lines) in [("--before-context", before), ("--after-context", after)] {
                if lines > 0 {
                    args.push(flag.into());
                    args.push(lines.to_string());
                }
            }
            args.push("--regexp".into());
            args.push(input.pattern.clone());
        }
        args.push("--".into());
        args.extend(roots.iter().map(|(path, _)| path.clone()));

        // Bound memory, and stop the engine once the result is decided: after
        // the match past the limit (which proves the limit was reached), or
        // once the capture is full. Like ice, the engine is killed rather than
        // left to scan the rest of the tree.
        const CAPTURE_LIMIT: usize = 256 * 1024;
        let limit = input.limit.unwrap_or(DEFAULT_LIMIT).max(1) as usize;
        let stop = signal.child_token();
        let stdout = Arc::new(std::sync::Mutex::new(String::new()));
        let stderr = Arc::new(std::sync::Mutex::new(String::new()));
        let stdout_full = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let capture = |buffer: Arc<std::sync::Mutex<String>>,
                       full: Arc<std::sync::atomic::AtomicBool>,
                       stop: Option<CancellationToken>| {
            let ast = input.mode == GrepMode::Ast;
            let (mut counted, mut matches) = (0usize, 0usize);
            Box::new(move |chunk: &str| {
                let mut buffer = buffer.lock().unwrap_or_else(|poison| poison.into_inner());
                let remaining = CAPTURE_LIMIT.saturating_sub(buffer.len());
                let mut end = remaining.min(chunk.len());
                while !chunk.is_char_boundary(end) {
                    end -= 1;
                }
                buffer.push_str(&chunk[..end]);
                let Some(stop) = &stop else { return };
                if end < chunk.len() {
                    full.store(true, std::sync::atomic::Ordering::Relaxed);
                    stop.cancel();
                    return;
                }
                while let Some(newline) = buffer[counted..].find('\n') {
                    let line = &buffer[counted..counted + newline];
                    let is_match = if ast {
                        !line.is_empty()
                    } else {
                        parse_engine_line(line).is_some_and(|line| line.is_match)
                    };
                    counted += newline + 1;
                    if is_match {
                        matches += 1;
                        if matches > limit {
                            stop.cancel();
                            return;
                        }
                    }
                }
            }) as Box<dyn FnMut(&str) + Send>
        };
        let options = ShellExecOptions {
            cwd: Some(self.env.cwd().to_path_buf()),
            timeout: Some(30.0),
            cancel: Some(&stop),
            on_stdout: Some(capture(
                stdout.clone(),
                stdout_full.clone(),
                Some(stop.clone()),
            )),
            on_stderr: Some(capture(
                stderr.clone(),
                Arc::new(std::sync::atomic::AtomicBool::new(false)),
                None,
            )),
            ..Default::default()
        };
        let result = self.env.exec_program(program, &args, options).await;
        // Our own stop is a normal end; the caller's cancellation is not.
        let exit_code = match result {
            Ok(output) => Some(output.exit_code),
            Err(_) if stop.is_cancelled() && !signal.is_cancelled() => None,
            Err(error) => return Err(map_exec_error(program, error)),
        };
        let stdout = stdout
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone();
        let stderr = stderr
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone();
        // Unreadable paths (permission denied, broken links) do not invalidate
        // what was found; rg and ugrep report them with exit 2 and keep going.
        let (skipped, errors): (Vec<&str>, Vec<&str>) = stderr
            .lines()
            .filter(|line| !line.trim().is_empty())
            .partition(|line| {
                line.contains("(os error ") || line.starts_with(&format!("{program}: warning:"))
            });
        let failed = exit_code.is_some_and(|code| code > 2 || code == 2 && skipped.is_empty());
        if failed || !errors.is_empty() {
            let code = exit_code.unwrap_or(0);
            return Err(ExternalError::Failed(AgentError::Tool(if code > 1 {
                format!("{program} failed (exit {code}): {}", stderr.trim())
            } else {
                format!("{program}: {}", stderr.trim())
            })));
        }
        let capture_truncated = stdout_full.load(std::sync::atomic::Ordering::Relaxed);
        let mut lines = Vec::new();
        let mut lines_truncated = false;
        let mut reached = false;
        if input.mode == GrepMode::Ast {
            for line in stdout.lines().filter(|line| !line.is_empty()) {
                if capture_truncated
                    && !stdout.ends_with('\n')
                    && stdout.rsplit('\n').next() == Some(line)
                {
                    break;
                }
                if lines.len() == limit {
                    reached = true;
                    break;
                }
                let value: serde_json::Value = serde_json::from_str(line).map_err(|e| {
                    ExternalError::Failed(AgentError::Tool(format!("invalid ast-grep output: {e}")))
                })?;
                let file = value["file"].as_str().unwrap_or("?");
                let file = Path::new(file)
                    .strip_prefix(cwd)
                    .unwrap_or(Path::new(file))
                    .to_string_lossy();
                let line = value["range"]["start"]["line"].as_u64().unwrap_or(0) + 1;
                let text = value["text"].as_str().unwrap_or("").replace('\n', "\\n");
                let text = truncate_line_default(&text);
                lines_truncated |= text.was_truncated;
                lines.push(format!("{file}:{line}: {}", text.text));
            }
        } else {
            let prefix = format!("{}{}", cwd.display(), std::path::MAIN_SEPARATOR);
            let mut matches = 0usize;
            for raw in stdout.lines() {
                let line = match parse_engine_line(raw) {
                    Some(line) => {
                        if line.is_match {
                            matches += 1;
                            if matches > limit {
                                reached = true;
                                break;
                            }
                        }
                        // Only the line text counts toward the length cap, as in
                        // the in-process engine.
                        let path = line.path.strip_prefix(&prefix).unwrap_or(line.path);
                        let sep = if line.is_match { ':' } else { '-' };
                        let text = truncate_line_default(line.text);
                        lines_truncated |= text.was_truncated;
                        format!(
                            "{path}{}{sep}{}{sep} {}",
                            line.member, line.number, text.text
                        )
                    }
                    None => raw.strip_prefix(&prefix).unwrap_or(raw).to_owned(),
                };
                lines.push(line);
            }
        }
        let text = if lines.is_empty() {
            "No matches found".to_owned()
        } else {
            lines.join("\n")
        };
        let truncation = truncate_head(
            &text,
            TruncationOptions {
                max_lines: None,
                max_bytes: None,
            },
        );
        let mut text = truncation.content.clone();
        let truncated = reached || truncation.truncated || capture_truncated || lines_truncated;
        if reached {
            text.push_str(&format!(
                "\n\n[{limit} matches limit reached. Use limit={} for more, or refine the pattern or path.]",
                limit * 2
            ));
        } else if truncated {
            text.push_str("\n\n[Search output truncated; refine the pattern or path.]");
        }
        if let Some(first) = skipped.first() {
            let first = first.strip_prefix(&format!("{program}: ")).unwrap_or(first);
            text.push_str(&format!(
                "\n\n[Skipped {} unreadable path(s), e.g. {}]",
                skipped.len(),
                first.replace(
                    &format!("{}{}", cwd.display(), std::path::MAIN_SEPARATOR),
                    ""
                )
            ));
        }
        Ok(AgentToolResult {
            content: vec![TextContentOrImage::text(text)],
            details: serde_json::json!({"engine": program, "truncated": truncated, "skippedPaths": skipped.len()}),
            ..Default::default()
        })
    }
}

/// One `path\0N:text` (match) or `path\0N-text` (context) line from
/// `--null` output. ugrep names an archive member as `path\0{member}:N:text`.
struct EngineLine<'a> {
    path: &'a str,
    member: &'a str,
    number: &'a str,
    is_match: bool,
    text: &'a str,
}

fn parse_engine_line(line: &str) -> Option<EngineLine<'_>> {
    let (path, rest) = line.split_once('\0')?;
    let member_len = match rest.strip_prefix('{') {
        Some(member) => member.find('}')? + 2,
        None => 0,
    };
    let (member, rest) = rest.split_at(member_len);
    // ugrep repeats the separator after a member: `path\0{member}:N:text`.
    let rest = match member.is_empty() {
        true => rest,
        false => rest.strip_prefix([':', '-'])?,
    };
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    let sep = rest[digits..].chars().next()?;
    if digits == 0 || sep != ':' && sep != '-' {
        return None;
    }
    Some(EngineLine {
        path,
        member,
        number: &rest[..digits],
        is_match: sep == ':',
        text: &rest[digits + 1..],
    })
}

fn map_exec_error(program: &'static str, error: crate::error::ExecutionError) -> ExternalError {
    if error.code == crate::error::ExecutionErrorCode::ShellUnavailable
        || (error.code == crate::error::ExecutionErrorCode::SpawnError
            && error
                .source_message
                .as_deref()
                .is_some_and(|s| s.contains("not found")))
        || (error.code == crate::error::ExecutionErrorCode::SpawnError
            && (error.message.contains("No such file or directory")
                || error.message.contains("os error 2")))
    {
        ExternalError::Unavailable(program)
    } else {
        ExternalError::Failed(AgentError::Tool(error.to_string()))
    }
}

/// Build the `grep` tool from a context. Mirrors TS `createGrepTool`.
pub fn create_grep_tool(
    context: &ExecutionToolContext,
    options: Option<GrepToolOptions>,
) -> Arc<dyn AgentTool> {
    let _ = options; // forward-compat seam, no surface in v1
    Arc::new(GrepTool {
        schema: GrepTool::schema(),
        env: context.env().clone(),
    })
}

#[async_trait]
impl AgentTool for GrepTool {
    fn schema(&self) -> &Tool {
        &self.schema
    }
    fn label(&self) -> &str {
        "grep"
    }

    async fn execute(
        &self,
        _tool_call_id: &str,
        params: serde_json::Value,
        signal: CancellationToken,
        _on_update: Arc<dyn Fn(ToolResultPartial) + Send + Sync>,
    ) -> Result<AgentToolResult, AgentError> {
        let input: GrepInput = serde_json::from_value(params)
            .map_err(|e| AgentError::Validation(format!("grep input invalid: {e}")))?;

        if input.mode == GrepMode::Text && input.language.is_some() {
            return Err(AgentError::Validation(
                "language is only supported in ast mode".into(),
            ));
        }
        let roots = self.resolve_roots(&input, &signal).await?;
        if input.mode != GrepMode::Text || self.env.supports_exec_program() {
            match self.search_external(&input, &roots, &signal).await {
                Ok(result) => return Ok(result),
                Err(ExternalError::Unavailable(_)) if input.mode == GrepMode::Text => {}
                Err(ExternalError::Unavailable(program)) => {
                    return Err(AgentError::Tool(format!(
                        "{program} is unavailable; install it to use this grep mode"
                    )));
                }
                Err(ExternalError::Failed(error)) => return Err(error),
            }
        }

        let cancel = Some(&signal);
        let effective_limit = input.limit.unwrap_or(DEFAULT_LIMIT).max(1) as usize;
        let (before, after) = input.context_window();

        // Compile the regex. `literal` ⇒ escape; `ignore_case` ⇒ case-insensitive.
        let pattern = if input.literal.unwrap_or(false) {
            regex::escape(&input.pattern)
        } else {
            input.pattern.clone()
        };
        let re = regex::RegexBuilder::new(&pattern)
            .case_insensitive(input.ignore_case.unwrap_or(false))
            .build()
            .map_err(|e| AgentError::Validation(format!("grep pattern invalid: {e}")))?;

        let glob = match input.glob.as_deref() {
            Some(g) if !g.is_empty() => Some(GrepGlob::new(g)?),
            _ => None,
        };

        let mut state = SearchState {
            re,
            before,
            after,
            limit: effective_limit,
            matches: 0,
            match_limit_reached: false,
            lines_truncated: false,
            output_lines: Vec::new(),
        };

        'paths: for (search_path, root_kind) in roots {
            // Paths are shown relative to the working directory, as with rg.
            let display_root = match Path::new(&search_path).strip_prefix(self.env.cwd()) {
                Ok(rel) => rel.to_string_lossy().into_owned(),
                Err(_) => search_path.clone(),
            };

            if root_kind == Some(FileKind::Directory) {
                let root = PathBuf::from(&search_path);
                let root_stack = IgnoreStack::for_root(&*self.env, &root, IgnoreTool::Grep, cancel)
                    .await
                    .map_err(file_err_to_agent)?;
                // Iterative BFS walk (avoids async-recursion boxing). Sorted entries
                // for deterministic output order.
                let mut queue: VecDeque<(PathBuf, String, IgnoreStack)> = VecDeque::new();
                // Walk paths are relative to the working directory: that is what is
                // displayed and what rg matches globs against.
                queue.push_back((root, display_root.clone(), root_stack));
                while let Some((abs_dir, rel_dir, parent_stack)) = queue.pop_front() {
                    check_cancelled(&signal)?;
                    let mut entries =
                        match self.env.list_dir(&abs_dir.to_string_lossy(), cancel).await {
                            Ok(e) => e,
                            Err(e) if e.code == FileErrorCode::Aborted => {
                                return Err(file_err_to_agent(e))
                            }
                            // An unreadable directory is skipped, as rg does.
                            Err(_) => continue,
                        };
                    entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
                    let names: Vec<String> = entries.iter().map(|e| e.name.clone()).collect();
                    let stack = parent_stack
                        .enter(&*self.env, &abs_dir, &names, IgnoreTool::Grep, cancel)
                        .await
                        .map_err(file_err_to_agent)?;
                    for entry in entries {
                        if is_vcs_dir(&entry.name, entry.kind) {
                            continue;
                        }
                        let rel = if rel_dir.is_empty() {
                            entry.name.clone()
                        } else {
                            format!("{rel_dir}/{}", entry.name)
                        };
                        match entry.kind {
                            FileKind::Directory => {
                                if stack.is_ignored(&entry.path, true) {
                                    continue;
                                }
                                queue.push_back((entry.path.clone(), rel, stack.clone()));
                            }
                            FileKind::File => {
                                // As in rg, a file a positive glob selects is
                                // searched even when ignore rules exclude it.
                                let selected =
                                    glob.as_ref().map(|g| g.includes_file(&rel, &entry.name));
                                if selected == Some(false) {
                                    continue;
                                }
                                let whitelisted = selected == Some(true)
                                    && glob.as_ref().is_some_and(|g| !g.negate);
                                if !whitelisted && stack.is_ignored(&entry.path, false) {
                                    continue;
                                }
                                check_cancelled(&signal)?;
                                state
                                    .search_file(
                                        &*self.env,
                                        cancel,
                                        &entry.path.to_string_lossy(),
                                        &rel,
                                        true,
                                    )
                                    .await?;
                                if state.match_limit_reached {
                                    break 'paths;
                                }
                            }
                            // rg does not follow symlinks while walking.
                            FileKind::Symlink => {}
                        }
                    }
                }
            } else if root_kind == Some(FileKind::File) {
                // An explicit file is searched even if binary or ignored, as rg does.
                state
                    .search_file(&*self.env, cancel, &search_path, &display_root, false)
                    .await?;
                if state.match_limit_reached {
                    break 'paths;
                }
            } else {
                // Symlink root that didn't resolve to a file/dir (or not_supported).
                return Err(AgentError::Tool(format!(
                    "Path is neither a file nor a directory: {search_path}"
                )));
            }
        }
        let SearchState {
            match_limit_reached,
            lines_truncated,
            output_lines,
            ..
        } = state;

        if output_lines.is_empty() {
            return Ok(AgentToolResult {
                content: vec![TextContentOrImage::text("No matches found")],
                details: serde_json::Value::Null,
                ..Default::default()
            });
        }

        let raw_output = output_lines.join("\n");
        // Byte cap only — line cap disabled (match count already capped).
        let truncation = truncate_head(
            &raw_output,
            TruncationOptions {
                max_lines: Some(usize::MAX),
                max_bytes: None,
            },
        );
        let was_truncated = truncation.truncated;
        let mut output = truncation.content.clone();
        let mut details = GrepToolDetails {
            truncation: None,
            match_limit_reached: None,
            lines_truncated: None,
        };
        let mut notices: Vec<String> = Vec::new();
        if match_limit_reached {
            notices.push(format!(
                "{effective_limit} matches limit reached. Use limit={} for more, or refine pattern",
                effective_limit * 2
            ));
            details.match_limit_reached = Some(effective_limit as u32);
        }
        if was_truncated {
            notices.push(format!("{} limit reached", format_size(DEFAULT_MAX_BYTES)));
            details.truncation = Some(truncation);
        }
        if lines_truncated {
            notices.push(format!(
                "Some lines truncated to {GREP_MAX_LINE_LENGTH} chars. Use read tool to see full lines"
            ));
            details.lines_truncated = Some(true);
        }
        if !notices.is_empty() {
            output.push_str(&format!("\n\n[{}]", notices.join(". ")));
        }

        let details_value = serde_json::to_value(&details).unwrap_or(serde_json::Value::Null);
        Ok(AgentToolResult {
            content: vec![TextContentOrImage::text(output)],
            details: details_value,
            ..Default::default()
        })
    }
}

/// Match state shared across the files of one search.
struct SearchState {
    re: regex::Regex,
    before: usize,
    after: usize,
    limit: usize,
    matches: usize,
    match_limit_reached: bool,
    lines_truncated: bool,
    output_lines: Vec<String>,
}

/// rg reads 64 KiB buffers and skips a walked file once a buffer holds a
/// NUL, so a NUL anywhere in the first 64 KiB hides the whole file.
const BINARY_PROBE_BYTES: usize = 64 * 1024;

impl SearchState {
    /// Search one file, appending formatted match/context lines. Lines follow
    /// rg: split on LF only (no extra empty line after a final LF, and a lone
    /// CR does not start a new line); displayed text drops CR. Invalid UTF-8 is
    /// decoded lossily rather than skipping the file. A walked file with a NUL
    /// byte near its start is treated as binary and skipped.
    async fn search_file(
        &mut self,
        env: &dyn ExecutionEnv,
        cancel: Option<&CancellationToken>,
        abs: &str,
        rel: &str,
        skip_binary: bool,
    ) -> Result<(), AgentError> {
        let bytes = match env.read_binary_file(abs, cancel).await {
            Ok(b) => b,
            Err(e) if e.code == FileErrorCode::Aborted => return Err(file_err_to_agent(e)),
            // Unreadable files are skipped, as rg does.
            Err(_) => return Ok(()),
        };
        if skip_binary && bytes[..bytes.len().min(BINARY_PROBE_BYTES)].contains(&0) {
            return Ok(());
        }
        let text = String::from_utf8_lossy(&bytes);
        let mut lines: Vec<&str> = if text.is_empty() {
            Vec::new()
        } else {
            text.split('\n').collect()
        };
        if !text.is_empty() && text.ends_with('\n') {
            lines.pop();
        }

        for (idx, line) in lines.iter().enumerate() {
            if !self.re.is_match(line) {
                continue;
            }
            let line_number = idx + 1; // 1-indexed.
            self.matches += 1;
            if self.before == 0 && self.after == 0 {
                self.push_line(rel, line_number, line, true);
            } else {
                // Context block: match line uses `:`, context lines use `-`.
                // Overlapping windows repeat lines, as in ice.
                let start = line_number.saturating_sub(self.before).max(1);
                let end = (line_number + self.after).min(lines.len());
                for current in start..=end {
                    let text_line = lines.get(current - 1).copied().unwrap_or("");
                    self.push_line(rel, current, text_line, current == line_number);
                }
            }
            // Stop as soon as the cap is reached (ice kills rg here).
            if self.matches >= self.limit {
                self.match_limit_reached = true;
                return Ok(());
            }
        }
        Ok(())
    }

    fn push_line(&mut self, rel: &str, number: usize, line: &str, is_match: bool) {
        let display = line.replace('\r', "");
        let out = truncate_line_default(&display);
        if out.was_truncated {
            self.lines_truncated = true;
        }
        if is_match {
            self.output_lines
                .push(format!("{rel}:{number}: {}", out.text));
        } else {
            self.output_lines
                .push(format!("{rel}-{number}- {}", out.text));
        }
    }
}

/// An rg `--glob` filter. Like rg, it uses gitignore-style matching: a
/// pattern without `/` matches the basename at any depth, one with `/`
/// matches the path relative to the working directory, and a leading `!`
/// excludes.
struct GrepGlob {
    matcher: globset::GlobMatcher,
    negate: bool,
    basename: bool,
}

impl GrepGlob {
    fn new(glob: &str) -> Result<Self, AgentError> {
        let (negate, pattern) = match glob.strip_prefix('!') {
            Some(rest) => (true, rest),
            None => (false, glob),
        };
        let pattern = pattern.replace('\\', "/");
        // A leading slash anchors a glob to the working directory. Strip exactly
        // that anchor, and keep it in path mode even for a basename pattern.
        let anchored = pattern.starts_with('/');
        let pattern = if anchored { &pattern[1..] } else { &pattern };
        let basename = !anchored && !pattern.contains('/');
        let matcher = globset::GlobBuilder::new(pattern)
            .literal_separator(true)
            .build()
            .map_err(|e| AgentError::Validation(format!("grep glob invalid: {e}")))?
            .compile_matcher();
        Ok(Self {
            matcher,
            negate,
            basename,
        })
    }

    fn matches(&self, rel: &str, name: &str) -> bool {
        if self.basename {
            self.matcher.is_match(name)
        } else {
            self.matcher.is_match(rel)
        }
    }

    /// A positive glob selects files; a negated one removes them.
    fn includes_file(&self, rel: &str, name: &str) -> bool {
        self.matches(rel, name) != self.negate
    }
}

fn check_cancelled(signal: &CancellationToken) -> Result<(), AgentError> {
    if signal.is_cancelled() {
        return Err(AgentError::Tool("Operation aborted".into()));
    }
    Ok(())
}

/// Map a `FileError` into an `AgentError::Tool` (same shape as `read.rs`).
fn file_err_to_agent(e: crate::error::FileError) -> AgentError {
    AgentError::Tool(e.to_string())
}
