//! Mirrors `packages/coding-agent/src/core/tools/find.ts` — the built-in `find`
//! `AgentTool`. Searches for files by glob pattern and returns matching paths
//! (relative to the search directory, posix separators). Read-only — goes
//! through the `FileSystem` trait so it runs against both `OsExecutionEnv` and
//! `InMemoryExecutionEnv`.
//!
//! Divergence from TS (documented): the TS tool **shells out to `fd`**. The
//! Rust port walks **in-process** via the `FileSystem` trait and `globset`, so
//! it works against any `ExecutionEnv` and needs no external binary. It keeps
//! fd's observable rules: hidden files are included, `.gitignore`/`.ignore`/
//! `.fdignore` apply hierarchically (see `ignore_rules`), `.git/` is skipped,
//! symlinks are listed but not followed, and basename versus full-path glob
//! modes follow ice's `fd --glob [--full-path]` arguments. The walk order is a
//! deterministic sorted BFS rather than fd's.

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
    format_size, truncate_head, TruncationOptions, TruncationResult, DEFAULT_MAX_BYTES,
};

/// Default max results. Mirrors TS `find.ts::DEFAULT_LIMIT`.
const DEFAULT_LIMIT: u32 = 1000;

/// Input for the find tool. Mirrors TS `FindToolInput`.
#[derive(Debug, Clone, JsonSchema, Deserialize)]
pub struct FindInput {
    /// Glob pattern to match files, e.g. `*.ts`, `**/*.json`, `src/**/*.spec.ts`.
    pub pattern: String,
    /// Directory to search in (default: current directory).
    #[serde(default)]
    pub path: Option<String>,
    /// Maximum number of results (default 1000).
    #[serde(default)]
    pub limit: Option<u32>,
}

/// Structured details for the find tool. Mirrors TS `FindToolDetails`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FindToolDetails {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncation: Option<TruncationResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_limit_reached: Option<u32>,
}

/// Options for `create_find_tool`. Forward-compat seam (the TS tool carries a
/// pluggable `operations` bag for remote backends; the Rust port always uses
/// the `FileSystem` trait).
#[derive(Debug, Clone, Default)]
pub struct FindToolOptions {}

/// The built-in `find` tool. Holds an `Arc<dyn ExecutionEnv>` (read-only view).
pub struct FindTool {
    schema: Tool,
    env: Arc<dyn ExecutionEnv>,
}

impl FindTool {
    fn schema() -> Tool {
        let params = schemars::schema_for!(FindInput);
        Tool {
            name: "find".to_string(),
            description: format!(
                "Search for files by glob pattern. Returns matching file paths relative to the \
                 search directory. Respects .gitignore and skips `.git/`. Output is truncated to {DEFAULT_LIMIT} \
                 results or {kb}KB (whichever is hit first).",
                kb = DEFAULT_MAX_BYTES / 1024
            ),
            parameters: ice_ai::types::Schema::new(serde_json::to_value(params).unwrap_or_default()),
            constrained_sampling: None,
        }
    }
}

/// Build the `find` tool from a context. Mirrors TS `createFindTool`.
pub fn create_find_tool(
    context: &ExecutionToolContext,
    options: Option<FindToolOptions>,
) -> Arc<dyn AgentTool> {
    let _ = options; // forward-compat seam, no surface in v1
    Arc::new(FindTool {
        schema: FindTool::schema(),
        env: context.env().clone(),
    })
}

#[async_trait]
impl AgentTool for FindTool {
    fn schema(&self) -> &Tool {
        &self.schema
    }
    fn label(&self) -> &str {
        "find"
    }

    async fn execute(
        &self,
        _tool_call_id: &str,
        params: serde_json::Value,
        signal: CancellationToken,
        _on_update: Arc<dyn Fn(ToolResultPartial) + Send + Sync>,
    ) -> Result<AgentToolResult, AgentError> {
        let input: FindInput = serde_json::from_value(params)
            .map_err(|e| AgentError::Validation(format!("find input invalid: {e}")))?;

        let cancel = Some(&signal);
        let effective_limit = input.limit.unwrap_or(DEFAULT_LIMIT).max(1) as usize;

        let search_path =
            resolve_tool_path(&*self.env, input.path.as_deref().unwrap_or("."), cancel)
                .await
                .map_err(file_err_to_agent)?;

        // Path-not-found (the TS custom-glob branch checks exists; the fd branch
        // lets fd error — we check up-front for a uniform in-process path).
        if !self
            .env
            .exists(&search_path, cancel)
            .await
            .map_err(file_err_to_agent)?
        {
            return Err(AgentError::Tool(format!("Path not found: {search_path}")));
        }

        let glob = FindGlob::new(&input.pattern)?;
        let root = PathBuf::from(&search_path);
        let canonical_root = if glob.is_absolute() {
            match self.env.canonical_path(&search_path, cancel).await {
                Ok(path) => Some(path),
                Err(e) if e.code == FileErrorCode::Aborted => return Err(file_err_to_agent(e)),
                // Some execution environments cannot canonicalize paths.
                // Keep their existing absolute-path spelling in that case.
                Err(_) => None,
            }
        } else {
            None
        };
        let root_stack = IgnoreStack::for_root(&*self.env, &root, IgnoreTool::Find, cancel)
            .await
            .map_err(file_err_to_agent)?;

        let mut collected: Vec<String> = Vec::new();
        // BFS walk from the search root, sorted per directory for a stable
        // order (fd's order is unspecified).
        let mut queue: VecDeque<(PathBuf, String, IgnoreStack)> = VecDeque::new();
        queue.push_back((root.clone(), String::new(), root_stack));
        'walk: while let Some((abs_dir, rel_dir, parent_stack)) = queue.pop_front() {
            check_cancelled(&signal)?;
            let mut entries = match self.env.list_dir(&abs_dir.to_string_lossy(), cancel).await {
                Ok(e) => e,
                Err(e) if e.code == FileErrorCode::Aborted => return Err(file_err_to_agent(e)),
                // An unreadable directory is skipped, as fd does.
                Err(_) => continue,
            };
            entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
            let names: Vec<String> = entries.iter().map(|e| e.name.clone()).collect();
            let stack = parent_stack
                .enter(&*self.env, &abs_dir, &names, IgnoreTool::Find, cancel)
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
                let match_path = canonical_root
                    .as_ref()
                    .map(|root| root.join(&rel))
                    .unwrap_or_else(|| entry.path.clone());
                if glob.is_match(&entry.name, &rel, &match_path) {
                    let mut shown = rel.clone();
                    if is_dir {
                        shown.push('/');
                    }
                    collected.push(shown);
                    if collected.len() >= effective_limit {
                        break 'walk;
                    }
                }
                // Symlinks are listed but not followed, as fd does by default.
                if is_dir {
                    queue.push_back((entry.path.clone(), rel, stack.clone()));
                }
            }
        }
        // Mirrors TS: the notice appears when the returned count reaches the
        // limit (fd --max-results), not only when more results existed.
        let result_limit_reached = collected.len() >= effective_limit;
        if collected.is_empty() {
            return Ok(AgentToolResult {
                content: vec![TextContentOrImage::text("No files found matching pattern")],
                details: serde_json::Value::Null,
                ..Default::default()
            });
        }

        let raw_output = collected.join("\n");
        // Byte cap only — line cap disabled (result count already capped).
        let truncation = truncate_head(
            &raw_output,
            TruncationOptions {
                max_lines: Some(usize::MAX),
                max_bytes: None,
            },
        );
        let was_truncated = truncation.truncated;
        let mut output = truncation.content.clone();
        let mut details = FindToolDetails {
            truncation: None,
            result_limit_reached: None,
        };
        let mut notices: Vec<String> = Vec::new();
        if result_limit_reached {
            notices.push(format!(
                "{effective_limit} results limit reached. Use limit={} for more, or refine pattern",
                effective_limit * 2
            ));
            details.result_limit_reached = Some(effective_limit as u32);
        }
        if was_truncated {
            notices.push(format!("{} limit reached", format_size(DEFAULT_MAX_BYTES)));
            details.truncation = Some(truncation);
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

/// The compiled find pattern. Mirrors how ice drives `fd --glob`: repeated
/// leading `./` is stripped; a pattern without `/` matches the entry's
/// basename; one with `/` matches the full path (`--full-path`), prefixed with
/// `**/` unless it is absolute or already `**/`-prefixed. An absolute pattern
/// matches the absolute path; a relative one the path below the search root.
struct FindGlob {
    matcher: globset::GlobMatcher,
    mode: GlobMode,
}

enum GlobMode {
    Basename,
    Relative,
    Absolute,
}

impl FindGlob {
    fn new(pattern: &str) -> Result<Self, AgentError> {
        let mut stripped = pattern;
        while let Some(rest) = stripped.strip_prefix("./") {
            stripped = rest;
        }
        if stripped.is_empty() {
            stripped = pattern;
        }
        let normalized = stripped.replace('\\', "/");
        let (glob, mode) = if !normalized.contains('/') {
            (normalized, GlobMode::Basename)
        } else if normalized.starts_with('/') || is_windows_absolute(&normalized) {
            (normalized, GlobMode::Absolute)
        } else if normalized.starts_with("**/") {
            (normalized, GlobMode::Relative)
        } else {
            (format!("**/{normalized}"), GlobMode::Relative)
        };
        let matcher = globset::GlobBuilder::new(&glob)
            .literal_separator(true)
            .build()
            .map_err(|e| AgentError::Validation(format!("find pattern invalid: {e}")))?
            .compile_matcher();
        Ok(Self { matcher, mode })
    }

    fn is_match(&self, name: &str, rel: &str, abs: &Path) -> bool {
        match self.mode {
            GlobMode::Basename => self.matcher.is_match(name),
            GlobMode::Relative => self.matcher.is_match(rel),
            GlobMode::Absolute => self
                .matcher
                .is_match(abs.to_string_lossy().replace('\\', "/")),
        }
    }

    fn is_absolute(&self) -> bool {
        matches!(self.mode, GlobMode::Absolute)
    }
}

fn is_windows_absolute(p: &str) -> bool {
    let b = p.as_bytes();
    b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'/'
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
