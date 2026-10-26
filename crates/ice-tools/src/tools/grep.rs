//! Mirrors `packages/coding-agent/src/core/tools/grep.ts` — the built-in `grep`
//! `AgentTool`. Searches file contents for a regex/literal pattern and returns
//! matching lines with `file:line: text` (and optional `-line-` context lines).
//! Read-only — goes through the `FileSystem` trait so it runs against both
//! `OsExecutionEnv` and `InMemoryExecutionEnv`.
//!
//! Divergence from TS (documented): the TS tool **shells out to `rg`**
//! (ripgrep), parsing its `--json` stream, and re-reads files only for context
//! blocks. The Rust port implements search **in-process** via the `FileSystem`
//! trait + the `regex` crate, so the tool works against any `ExecutionEnv`
//! (including `InMemoryExecutionEnv`, which has no shell). It keeps rg's
//! observable rules: hidden files are searched, `.gitignore`/`.ignore`/
//! `.rgignore` apply hierarchically (see `ignore_rules`), `.git/` and
//! symlinks are not walked, binary files are skipped during the walk, and
//! `--glob` uses gitignore-style matching with `!` negation. Regex syntax is
//! Rust's `regex` crate, not JavaScript's or PCRE. Traversal order is a
//! deterministic sorted BFS rather than rg's walk order. Output shape,
//! match/context line formats, per-line + byte truncation, and the
//! match-limit semantics match TS.

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

use crate::env::ExecutionEnv;
use crate::env::FileKind;
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
    /// Directory or file to search (default: current directory).
    #[serde(default)]
    pub path: Option<String>,
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
    /// Maximum number of matches to return (default 100).
    #[serde(default)]
    pub limit: Option<u32>,
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

impl GrepTool {
    fn schema() -> Tool {
        let params = schemars::schema_for!(GrepInput);
        Tool {
            name: "grep".to_string(),
            description: format!(
                "Search file contents for a pattern. Returns matching lines with file paths and \
                 line numbers. Respects .gitignore and skips `.git/`. Output is truncated to {DEFAULT_LIMIT} \
                 matches or {kb}KB (whichever is hit first). Long lines are truncated to \
                 {GREP_MAX_LINE_LENGTH} chars.",
                kb = DEFAULT_MAX_BYTES / 1024
            ),
            parameters: ice_ai::types::Schema::new(
                serde_json::to_value(params).unwrap_or_default(),
            ),
            constrained_sampling: None,
        }
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

        let cancel = Some(&signal);
        let effective_limit = input.limit.unwrap_or(DEFAULT_LIMIT).max(1) as usize;
        let context_val = input.context.unwrap_or(0).min(u32::MAX) as usize;

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

        // Resolve the search path. A `.`/absent path resolves to the env cwd.
        let search_path =
            resolve_tool_path(&*self.env, input.path.as_deref().unwrap_or("."), cancel)
                .await
                .map_err(file_err_to_agent)?;

        // Determine dir vs file (following a symlinked root via canonical_path).
        // Any failure to stat the root is "Path not found", as in ice.
        let info = match self.env.file_info(&search_path, cancel).await {
            Ok(info) => info,
            Err(e) if e.code == FileErrorCode::Aborted => return Err(file_err_to_agent(e)),
            Err(_) => return Err(AgentError::Tool(format!("Path not found: {search_path}"))),
        };
        let root_kind = if info.kind == FileKind::Symlink {
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

        let mut state = SearchState {
            re,
            context: context_val,
            limit: effective_limit,
            matches: 0,
            match_limit_reached: false,
            lines_truncated: false,
            output_lines: Vec::new(),
        };

        if root_kind == FileKind::Directory {
            let root = PathBuf::from(&search_path);
            let root_stack = IgnoreStack::for_root(&*self.env, &root, IgnoreTool::Grep, cancel)
                .await
                .map_err(file_err_to_agent)?;
            // Iterative BFS walk (avoids async-recursion boxing). Sorted entries
            // for deterministic output order.
            let mut queue: VecDeque<(PathBuf, String, IgnoreStack)> = VecDeque::new();
            queue.push_back((root, String::new(), root_stack));
            'walk: while let Some((abs_dir, rel_dir, parent_stack)) = queue.pop_front() {
                check_cancelled(&signal)?;
                let mut entries = match self.env.list_dir(&abs_dir.to_string_lossy(), cancel).await
                {
                    Ok(e) => e,
                    Err(e) if e.code == FileErrorCode::Aborted => return Err(file_err_to_agent(e)),
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
                    let is_dir = entry.kind == FileKind::Directory;
                    if stack.is_ignored(&entry.path, is_dir) {
                        continue;
                    }
                    let rel = if rel_dir.is_empty() {
                        entry.name.clone()
                    } else {
                        format!("{rel_dir}/{}", entry.name)
                    };
                    match entry.kind {
                        FileKind::Directory => {
                            queue.push_back((entry.path.clone(), rel, stack.clone()));
                        }
                        FileKind::File => {
                            if let Some(g) = &glob {
                                if !g.includes_file(&rel, &entry.name) {
                                    continue;
                                }
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
                                break 'walk;
                            }
                        }
                        // rg does not follow symlinks while walking.
                        FileKind::Symlink => {}
                    }
                }
            }
        } else if root_kind == FileKind::File {
            // An explicit file is searched even if binary or ignored, as rg does.
            let rel = basename(&search_path);
            state
                .search_file(&*self.env, cancel, &search_path, &rel, false)
                .await?;
        } else {
            // Symlink root that didn't resolve to a file/dir (or not_supported).
            return Err(AgentError::Tool(format!(
                "Path is neither a file nor a directory: {search_path}"
            )));
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
    context: usize,
    limit: usize,
    matches: usize,
    match_limit_reached: bool,
    lines_truncated: bool,
    output_lines: Vec<String>,
}

/// Bytes rg inspects for a NUL to classify a file as binary.
const BINARY_PROBE_BYTES: usize = 8 * 1024;

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
            if self.context == 0 {
                self.push_line(rel, line_number, line, true);
            } else {
                // Context block: match line uses `:`, context lines use `-`.
                // Overlapping windows repeat lines, as in ice.
                let start = line_number.saturating_sub(self.context).max(1);
                let end = (line_number + self.context).min(lines.len());
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
/// matches the path below the search root, and a leading `!` excludes.
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
        // A leading slash anchors a glob to the search root. Strip exactly
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

/// Filename component of a path string (POSIX-agnostic: splits on `/` and `\`).
fn basename(p: &str) -> String {
    Path::new(p)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.to_string())
}

/// Map a `FileError` into an `AgentError::Tool` (same shape as `read.rs`).
fn file_err_to_agent(e: crate::error::FileError) -> AgentError {
    AgentError::Tool(e.to_string())
}
