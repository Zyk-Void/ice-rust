//! Instruction-file discovery: the `AGENTS.md` family pi and icers inject into
//! the system prompt as a `<project_context>` block. Mirrors `packages/
//! coding-agent/src/core/resource-loader.ts` (`loadContextFileFromDir`,
//! `loadProjectContextFiles`, `findShadowedContextFile`), extended with the
//! loading rules of Claude Code's `CLAUDE.md` memory.
//!
//! Unlike skills (a named registry) and prompt templates (on-demand `/expand`
//! targets), instruction files are always injected when present.
//!
//! Load order, lowest to highest priority (later files are read last and
//! win in practice):
//!
//! 1. Global, under the agent dir (`~/.icers/agent`): the first
//!    [`CONTEXT_FILE_CANDIDATES`] match, then `rules/*.md`.
//! 2. For each directory from the filesystem root down to `cwd`: the first
//!    candidate match and `.icers/rules/*.md` (checked in), then
//!    `AGENTS.local.md` (the user's private notes, normally git-ignored).
//!
//! Each file is processed the same way:
//! - YAML frontmatter is removed. A rule file whose frontmatter has `paths:`
//!   applies only to matching files, so it is not loaded here.
//! - Block-level HTML comments (`<!-- ... -->` on their own lines, outside
//!   code fences) are removed, so authors can leave notes for humans.
//! - `@path` references (`@file.md`, `@./file.md`, `@/abs/file.md`) outside
//!   code are loaded as separate entries right after the file, up to
//!   [`MAX_INCLUDE_DEPTH`] levels. Only text files are followed. A global
//!   file may include any path; a project or local file may include only
//!   paths inside its repository (or its own directory outside one), after
//!   resolving symlinks, so a cloned repository cannot pull `~/.ssh` into the
//!   prompt.
//! - Content past [`MAX_CONTEXT_FILE_BYTES`] is cut with a notice.
//! - Each resolved path is loaded at most once.
//!
//! In a git worktree nested inside its main checkout (`.worktrees/x`), the
//! walk passes both the worktree and the main checkout. Checked-in files from
//! directories inside the main checkout but outside the worktree are skipped,
//! since the worktree has its own copy; `AGENTS.local.md` is still loaded.

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use ice_tools::env::{ExecutionEnv, FileKind};
use tokio_util::sync::CancellationToken;

/// Per-directory candidate filenames, **in priority order**. First existing
/// match wins. Mirrors `loadContextFileFromDir`'s `candidates` array
/// (`resource-loader.ts:71`) — the AGENTS family takes precedence over the
/// CLAUDE family, and `.override.md` wins over the plain `AGENTS.md`.
pub const CONTEXT_FILE_CANDIDATES: &[&str] = &[
    "AGENTS.override.md",
    "AGENTS.md",
    "AGENTS.MD",
    "CLAUDE.md",
    "CLAUDE.MD",
];

/// Private, per-directory instructions that are not checked in.
pub const LOCAL_CONTEXT_FILE: &str = "AGENTS.local.md";

/// How many `@path` levels are followed below a top-level file.
pub const MAX_INCLUDE_DEPTH: usize = 5;

/// Per-file content limit in bytes; longer files are cut with a notice.
pub const MAX_CONTEXT_FILE_BYTES: usize = 64 * 1024;

/// Extensions an `@path` reference may point at. A path without an extension
/// (`Makefile`) is allowed too; anything else (images, archives) is skipped.
const INCLUDE_EXTENSIONS: &[&str] = &[
    "md", "markdown", "mdx", "txt", "text", "rst", "adoc", "json", "jsonc", "yaml", "yml", "toml",
    "ini", "cfg", "conf", "xml", "csv", "html", "htm", "css", "scss", "js", "mjs", "cjs", "ts",
    "tsx", "jsx", "py", "rb", "go", "rs", "java", "kt", "swift", "c", "h", "cc", "cpp", "hpp",
    "cs", "php", "sh", "bash", "zsh", "fish", "ps1", "sql", "graphql", "proto", "env", "example",
];

/// Who owns an instruction file, shown to the model as `scope="..."`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextScope {
    /// The user's own instructions for every project (agent dir).
    Global,
    /// Checked into the project.
    Project,
    /// The user's private notes for this project (`AGENTS.local.md`).
    Local,
}

impl ContextScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Project => "project",
            Self::Local => "local",
        }
    }
}

/// One loaded instruction file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextFile {
    pub path: PathBuf,
    /// Content as injected (frontmatter and comments removed, length capped).
    pub content: String,
    pub scope: ContextScope,
    /// The file whose `@path` reference loaded this one.
    pub included_from: Option<PathBuf>,
}

impl ContextFile {
    /// A top-level project file with `content` as given.
    pub fn project(path: impl Into<PathBuf>, content: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            content: content.into(),
            scope: ContextScope::Project,
            included_from: None,
        }
    }
}

/// Load the first context file present in `dir`, or `None`. Mirrors
/// `loadContextFileFromDir` (`resource-loader.ts:70-89`): candidates are tried
/// in order and the first readable regular file (or symlink to one) wins. The
/// content is returned raw; [`load_project_context_files`] processes it.
pub async fn load_context_file_from_dir(
    env: &Arc<dyn ExecutionEnv>,
    dir: impl AsRef<Path>,
) -> Option<ContextFile> {
    let cancel = CancellationToken::new();
    for name in CONTEXT_FILE_CANDIDATES {
        let candidate = dir.as_ref().join(name);
        if let Some(content) = read_regular_file(env, &candidate, &cancel).await {
            return Some(ContextFile::project(candidate, content));
        }
    }
    None
}

/// Discover and process the full instruction-file set for `cwd`, in load
/// order (see the module docs).
pub async fn load_project_context_files(
    env: &Arc<dyn ExecutionEnv>,
    cwd: &Path,
    agent_dir: &Path,
) -> Vec<ContextFile> {
    let mut loader = Loader {
        env,
        cancel: CancellationToken::new(),
        seen: HashSet::new(),
        out: Vec::new(),
    };

    // 1. Global.
    loader.load_global(agent_dir).await;

    // 2. Ancestors, root first. Paths shown to the model keep the spelling of
    // `cwd`; the symlink-resolved forms are only used for the comparisons.
    let repo = loader.repo_layout(&loader.resolve(cwd).await).await;
    let mut dirs: Vec<PathBuf> = normalize(cwd).ancestors().map(Path::to_path_buf).collect();
    dirs.reverse();
    for dir in dirs {
        let resolved = loader.resolve(&dir).await;
        let boundary = match &repo {
            Some(repo) if resolved.starts_with(&repo.root) => repo.root.clone(),
            _ => resolved.clone(),
        };
        let shadowed = repo.as_ref().is_some_and(|repo| repo.shadows(&resolved));
        if !shadowed {
            if let Some(file) = load_context_file_from_dir(env, &dir).await {
                loader
                    .load(file.path, ContextScope::Project, Some(&boundary), false)
                    .await;
            }
            for rule in loader.rule_files(&dir.join(".icers").join("rules")).await {
                loader
                    .load(rule, ContextScope::Project, Some(&boundary), true)
                    .await;
            }
        }
        loader
            .load(
                dir.join(LOCAL_CONTEXT_FILE),
                ContextScope::Local,
                Some(&boundary),
                false,
            )
            .await;
    }
    loader.out
}

/// Only the user's own global instruction files (step 1 of the load order),
/// for a project whose own files must not be read.
pub async fn load_global_context_files(
    env: &Arc<dyn ExecutionEnv>,
    agent_dir: &Path,
) -> Vec<ContextFile> {
    let mut loader = Loader {
        env,
        cancel: CancellationToken::new(),
        seen: HashSet::new(),
        out: Vec::new(),
    };
    loader.load_global(agent_dir).await;
    loader.out
}

/// Render the context-file list as a `<project_context>` block for the system
/// prompt. Mirrors `createCodingSystemPrompt`'s context section
/// (`system-prompt.ts:54-60,144-151`): a wrapper tag, a lead, then one
/// `<project_instructions path="..." scope="...">` child per file. Returns
/// `""` when there are no context files, so a caller can interpolate the
/// result without leaving an empty block.
pub fn format_project_context(files: &[ContextFile]) -> String {
    if files.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    out.push_str("\n\n<project_context>\n\n");
    out.push_str(
        "Instructions from the user and the project follow. Follow them; where they conflict \
with the default guidance above, they take precedence. Later files are more specific than \
earlier ones. scope=\"global\" is the user's own instructions for every project, \
scope=\"project\" is checked into the repository, and scope=\"local\" is the user's private \
notes for this project.\n\n",
    );
    for file in files {
        let path = escape_path(&file.path.to_string_lossy());
        let included = file
            .included_from
            .as_ref()
            .map(|parent| {
                format!(
                    " included_from=\"{}\"",
                    escape_path(&parent.to_string_lossy())
                )
            })
            .unwrap_or_default();
        out.push_str(&format!(
            "<project_instructions path=\"{path}\" scope=\"{}\"{included}>\n{}\n</project_instructions>\n\n",
            file.scope.as_str(),
            file.content
        ));
    }
    out.push_str("</project_context>\n");
    out
}

struct Loader<'a> {
    env: &'a Arc<dyn ExecutionEnv>,
    cancel: CancellationToken,
    seen: HashSet<String>,
    out: Vec<ContextFile>,
}

/// Where the repository containing `cwd` lives.
struct RepoLayout {
    /// The work tree holding `cwd` (main checkout or linked worktree).
    root: PathBuf,
    /// The main checkout, when `root` is a linked worktree nested inside it.
    nesting_main: Option<PathBuf>,
}

impl RepoLayout {
    /// Checked-in files in `dir` duplicate the worktree's own copies.
    fn shadows(&self, dir: &Path) -> bool {
        self.nesting_main
            .as_ref()
            .is_some_and(|main| dir.starts_with(main) && !dir.starts_with(&self.root))
    }
}

impl Loader<'_> {
    async fn load_global(&mut self, agent_dir: &Path) {
        if let Some(file) = load_context_file_from_dir(self.env, agent_dir).await {
            self.load(file.path, ContextScope::Global, None, false)
                .await;
        }
        for rule in self.rule_files(&agent_dir.join("rules")).await {
            self.load(rule, ContextScope::Global, None, true).await;
        }
    }

    /// Load `path` and, depth first, the files it references with `@path`.
    async fn load(
        &mut self,
        path: PathBuf,
        scope: ContextScope,
        boundary: Option<&Path>,
        is_rule: bool,
    ) {
        let boundary = match boundary {
            Some(boundary) => Some(self.resolve(boundary).await),
            None => None,
        };
        let mut stack: Vec<(PathBuf, Option<PathBuf>, usize)> = vec![(path, None, 0)];
        while let Some((path, parent, depth)) = stack.pop() {
            let key = self.resolve(&path).await;
            if !self.seen.insert(key.to_string_lossy().into_owned()) {
                continue;
            }
            let Some(raw) = read_regular_file(self.env, &path, &self.cancel).await else {
                continue;
            };
            let Some(body) = strip_frontmatter(&raw, is_rule && parent.is_none()) else {
                continue;
            };
            let body = strip_html_comments(&body);
            let base_dir = path.parent().unwrap_or(Path::new("/")).to_path_buf();
            let mut includes = Vec::new();
            if depth < MAX_INCLUDE_DEPTH {
                for target in extract_includes(&body) {
                    let target = base_dir.join(target);
                    if !has_text_extension(&target) {
                        continue;
                    }
                    if let Some(boundary) = &boundary {
                        // Resolving symlinks first stops `@link` escaping the
                        // repository through a symlink inside it.
                        if !self.resolve(&target).await.starts_with(boundary) {
                            continue;
                        }
                    }
                    includes.push(target);
                }
            }
            let content = cap_length(body.trim());
            let path = normalize(&path);
            if !content.is_empty() {
                self.out.push(ContextFile {
                    path: path.clone(),
                    content,
                    scope,
                    included_from: parent,
                });
            }
            for target in includes.into_iter().rev() {
                stack.push((target, Some(path.clone()), depth + 1));
            }
        }
    }

    /// `*.md` files directly in `dir`, sorted by name.
    async fn rule_files(&self, dir: &Path) -> Vec<PathBuf> {
        let Ok(entries) = self
            .env
            .list_dir(&dir.to_string_lossy(), Some(&self.cancel))
            .await
        else {
            return Vec::new();
        };
        let mut files: Vec<PathBuf> = entries
            .into_iter()
            .filter(|entry| entry.kind != FileKind::Directory)
            .filter(|entry| entry.name.to_ascii_lowercase().ends_with(".md"))
            .map(|entry| dir.join(entry.name))
            .collect();
        files.sort();
        files
    }

    /// Symlink-resolved path when it exists, else the lexically normalized
    /// path, so dedupe and boundary checks compare like with like.
    async fn resolve(&self, path: &Path) -> PathBuf {
        match self
            .env
            .canonical_path(&path.to_string_lossy(), Some(&self.cancel))
            .await
        {
            Ok(path) => PathBuf::from(path.to_string_lossy().replace('\\', "/")),
            Err(_) => PathBuf::from(normalize(path).to_string_lossy().replace('\\', "/")),
        }
    }

    /// Find the git work tree containing `cwd` by reading `.git` entries, like
    /// TS `findGitPaths`; no git process is spawned.
    async fn repo_layout(&self, cwd: &Path) -> Option<RepoLayout> {
        for dir in cwd.ancestors() {
            let dot_git = dir.join(".git");
            let Ok(info) = self
                .env
                .file_info(&dot_git.to_string_lossy(), Some(&self.cancel))
                .await
            else {
                continue;
            };
            let root = dir.to_path_buf();
            if info.kind == FileKind::Directory {
                return Some(RepoLayout {
                    root,
                    nesting_main: None,
                });
            }
            // A linked worktree's `.git` file: `gitdir: <main>/.git/worktrees/<name>`.
            let text = self
                .env
                .read_text_file(&dot_git.to_string_lossy(), Some(&self.cancel))
                .await
                .ok()?;
            let git_dir = root.join(text.trim().strip_prefix("gitdir:")?.trim());
            let common = self
                .env
                .read_text_file(
                    &git_dir.join("commondir").to_string_lossy(),
                    Some(&self.cancel),
                )
                .await
                .ok()
                .map(|common| normalize(&git_dir.join(common.trim())));
            let nesting_main = match common {
                Some(common) if common.file_name().is_some_and(|name| name == ".git") => {
                    let main = common.parent()?.to_path_buf();
                    let main = self.resolve(&main).await;
                    (root != main && root.starts_with(&main)).then_some(main)
                }
                _ => None,
            };
            return Some(RepoLayout { root, nesting_main });
        }
        None
    }
}

/// Read `path` when it is a regular file or a symlink to one.
async fn read_regular_file(
    env: &Arc<dyn ExecutionEnv>,
    path: &Path,
    cancel: &CancellationToken,
) -> Option<String> {
    let path_str = path.to_string_lossy();
    let info = env.file_info(&path_str, Some(cancel)).await.ok()?;
    match info.kind {
        FileKind::File => {}
        FileKind::Symlink => {
            let target = env.canonical_path(&path_str, Some(cancel)).await.ok()?;
            let target = env
                .file_info(&target.to_string_lossy(), Some(cancel))
                .await
                .ok()?;
            if target.kind != FileKind::File {
                return None;
            }
        }
        FileKind::Directory => return None,
    }
    env.read_text_file(&path_str, Some(cancel)).await.ok()
}

/// Drop YAML frontmatter. Returns `None` for a rule whose frontmatter scopes it
/// to `paths:`; such rules are loaded only for matching files. Malformed
/// frontmatter leaves the content untouched.
fn strip_frontmatter(raw: &str, is_rule: bool) -> Option<String> {
    if !raw.starts_with("---") {
        return Some(raw.to_string());
    }
    match crate::frontmatter::parse_frontmatter(raw) {
        Ok((frontmatter, body)) => {
            let scoped = frontmatter
                .get("paths")
                .is_some_and(|paths| !paths.is_null());
            (!(is_rule && scoped)).then_some(body)
        }
        Err(_) => Some(raw.to_string()),
    }
}

/// Remove `<!-- ... -->` comments that start a line outside fenced code. An
/// unclosed comment is kept, so a typo does not swallow the rest of the file.
fn strip_html_comments(content: &str) -> String {
    if !content.contains("<!--") {
        return content.to_string();
    }
    let lines: Vec<&str> = content.lines().collect();
    let mut out: Vec<&str> = Vec::with_capacity(lines.len());
    let mut fence: Option<&str> = None;
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        let trimmed = line.trim_start();
        if let Some(marker) = fence {
            if trimmed.starts_with(marker) {
                fence = None;
            }
            out.push(line);
            index += 1;
            continue;
        }
        if let Some(marker) = fence_marker(trimmed) {
            fence = Some(marker);
            out.push(line);
            index += 1;
            continue;
        }
        if let Some(after_open) = trimmed.strip_prefix("<!--") {
            // The closing `-->` may sit on this line or a later one.
            let mut end = index;
            let mut rest = after_open;
            let tail = loop {
                if let Some((_, tail)) = rest.split_once("-->") {
                    break Some(tail);
                }
                end += 1;
                match lines.get(end) {
                    Some(next) => rest = next,
                    None => break None,
                }
            };
            if let Some(tail) = tail {
                if !tail.trim().is_empty() {
                    out.push(tail);
                }
                index = end + 1;
                continue;
            }
        }
        out.push(line);
        index += 1;
    }
    out.join("\n")
}

fn fence_marker(trimmed: &str) -> Option<&'static str> {
    if trimmed.starts_with("```") {
        Some("```")
    } else if trimmed.starts_with("~~~") {
        Some("~~~")
    } else {
        None
    }
}

/// `@path` references outside fenced code and inline code spans, in order of
/// appearance. `#fragment` suffixes are dropped and `\ ` unescapes to a space.
fn extract_includes(content: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    let mut fence: Option<&str> = None;
    for line in content.lines() {
        let trimmed = line.trim_start();
        if let Some(marker) = fence {
            if trimmed.starts_with(marker) {
                fence = None;
            }
            continue;
        }
        if let Some(marker) = fence_marker(trimmed) {
            fence = Some(marker);
            continue;
        }
        // Blank out inline code spans so `@x` inside backticks is ignored.
        let mut prose = String::with_capacity(line.len());
        let mut in_code = false;
        for ch in line.chars() {
            if ch == '`' {
                in_code = !in_code;
                prose.push(' ');
            } else {
                prose.push(if in_code { ' ' } else { ch });
            }
        }
        let chars: Vec<char> = prose.chars().collect();
        let mut at = 0;
        while at < chars.len() {
            let starts_token = chars[at] == '@' && (at == 0 || chars[at - 1].is_whitespace());
            if !starts_token {
                at += 1;
                continue;
            }
            let mut end = at + 1;
            let mut path = String::new();
            while end < chars.len() {
                match chars[end] {
                    '\\' if chars.get(end + 1) == Some(&' ') => {
                        path.push(' ');
                        end += 2;
                    }
                    ch if ch.is_whitespace() => break,
                    ch => {
                        path.push(ch);
                        end += 1;
                    }
                }
            }
            at = end;
            if let Some(hash) = path.find('#') {
                path.truncate(hash);
            }
            if is_include_path(&path) && !found.contains(&path) {
                found.push(path);
            }
        }
    }
    found
}

fn is_include_path(path: &str) -> bool {
    if path.starts_with("./") || path.starts_with("../") {
        return true;
    }
    if let Some(rest) = path.strip_prefix('/') {
        return !rest.is_empty();
    }
    path.chars()
        .next()
        .is_some_and(|first| first.is_ascii_alphanumeric() || matches!(first, '.' | '_' | '-'))
}

fn has_text_extension(path: &Path) -> bool {
    match path.extension() {
        None => true,
        Some(ext) => {
            let ext = ext.to_string_lossy().to_ascii_lowercase();
            INCLUDE_EXTENSIONS.contains(&ext.as_str())
        }
    }
}

fn cap_length(content: &str) -> String {
    if content.len() <= MAX_CONTEXT_FILE_BYTES {
        return content.to_string();
    }
    let mut end = MAX_CONTEXT_FILE_BYTES;
    while !content.is_char_boundary(end) {
        end -= 1;
    }
    format!(
        "{}\n\n[Truncated: this file is longer than {} KiB. Read the file for the rest.]",
        &content[..end],
        MAX_CONTEXT_FILE_BYTES / 1024
    )
}

/// Collapse `.` and `..` lexically, keeping the platform's separators.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Escape a path for safe interpolation into an XML attribute. The directory
/// component is user-controlled, so escape the quote and markup characters.
fn escape_path(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '"' => "&quot;".to_string(),
            '&' => "&amp;".to_string(),
            '<' => "&lt;".to_string(),
            '>' => "&gt;".to_string(),
            other => other.to_string(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ice_tools::env::FileSystem;
    use ice_tools::in_memory::InMemoryExecutionEnv;

    /// Build a fresh in-memory env with cwd `/proj` and seed the given files
    /// (relative paths resolve against `/proj`). Returns the env as a trait
    /// object so it passes directly to the loader signatures.
    async fn env_with(files: &[(&str, &str)]) -> Arc<dyn ExecutionEnv> {
        let env = Arc::new(InMemoryExecutionEnv::with_cwd(PathBuf::from("/proj")));
        for (rel, content) in files {
            // `absolute_path` normalizes `.` / `/`-relative against cwd.
            let abs = env.absolute_path(rel, None).await.expect("absolute_path");
            env.seed_file(&abs.to_string_lossy(), content.as_bytes().to_vec())
                .await;
        }
        env
    }

    /// Join path segments with `/` and seed a file. The in-memory env stores
    /// `BTreeMap` keys verbatim and looks them up via `resolve_key` (which
    /// normalizes to forward slashes), so tests must seed with forward-slash
    /// absolute strings — `Path::join` on Windows would insert backslashes and
    /// the lookup would miss.
    async fn seed(env: &InMemoryExecutionEnv, abs_slash_path: &str, content: &str) {
        env.seed_file(abs_slash_path, content.as_bytes().to_vec())
            .await;
    }

    #[tokio::test]
    async fn agents_md_beats_claude_md_in_same_dir() {
        let env = env_with(&[
            ("AGENTS.md", "agents-content"),
            ("CLAUDE.md", "claude-content"),
        ])
        .await;
        let dir = Path::new("/proj");
        let got = load_context_file_from_dir(&env, dir).await.expect("found");
        assert_eq!(got.path.file_name().unwrap(), "AGENTS.md");
        assert_eq!(got.content, "agents-content");
    }

    #[tokio::test]
    async fn override_beats_plain_agents() {
        let env = env_with(&[("AGENTS.md", "plain"), ("AGENTS.override.md", "override")]).await;
        let dir = Path::new("/proj");
        let got = load_context_file_from_dir(&env, dir).await.expect("found");
        assert_eq!(got.path.file_name().unwrap(), "AGENTS.override.md");
        assert_eq!(got.content, "override");
    }

    #[tokio::test]
    async fn claude_fallback_when_no_agents() {
        let env = env_with(&[("CLAUDE.md", "claude-only")]).await;
        let dir = Path::new("/proj");
        let got = load_context_file_from_dir(&env, dir).await.expect("found");
        assert_eq!(got.path.file_name().unwrap(), "CLAUDE.md");
    }

    #[tokio::test]
    async fn none_when_no_candidates() {
        let env = env_with(&[("README.md", "unrelated")]).await;
        let dir = Path::new("/proj");
        assert!(load_context_file_from_dir(&env, dir).await.is_none());
    }

    #[tokio::test]
    async fn project_walk_global_then_deep_last() {
        // Global under /home/icers/agent, project under /proj + /proj/sub.
        let concrete = InMemoryExecutionEnv::with_cwd(PathBuf::from("/proj/sub"));
        let agent_dir = Path::new("/home/icers/agent");
        let cwd = Path::new("/proj/sub");
        // Seed with forward-slash absolute paths: `InMemoryExecutionEnv` stores
        // `seed_file` keys verbatim, while `file_info`'s `resolve_key` normalizes
        // `\`→`/` (Windows `Path::join` inserts `\`, which would miss the lookup).
        seed(&concrete, "/home/icers/agent/AGENTS.md", "GLOBAL").await;
        seed(&concrete, "/proj/sub/AGENTS.md", "SUB").await;
        seed(&concrete, "/proj/AGENTS.md", "ROOT").await;
        let env: Arc<dyn ExecutionEnv> = Arc::new(concrete);

        let files = load_project_context_files(&env, cwd, agent_dir).await;
        let contents: Vec<&str> = files.iter().map(|f| f.content.as_str()).collect();
        // Global first, then root (shallow), then sub (deepest = cwd) last.
        assert_eq!(contents, vec!["GLOBAL", "ROOT", "SUB"]);
    }

    #[tokio::test]
    async fn dedups_when_global_equals_cwd_dir() {
        // agent_dir == cwd dir → the global load + the cwd step hit the same
        // file; the cwd step must skip it (no duplicate entry).
        let concrete = InMemoryExecutionEnv::with_cwd(PathBuf::from("/x/y"));
        seed(&concrete, "/x/y/AGENTS.md", "only").await;
        let env: Arc<dyn ExecutionEnv> = Arc::new(concrete);
        let dir = Path::new("/x/y");
        let files = load_project_context_files(&env, dir, dir).await;
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].content, "only");
    }

    async fn mem_env(cwd: &str, dirs: &[&str], files: &[(&str, &str)]) -> Arc<dyn ExecutionEnv> {
        let env = InMemoryExecutionEnv::with_cwd(PathBuf::from(cwd));
        for dir in dirs {
            env.create_dir(dir, true, None).await.unwrap();
        }
        for (path, content) in files {
            seed(&env, path, content).await;
        }
        Arc::new(env)
    }

    fn summary(files: &[ContextFile]) -> Vec<(String, &'static str, String)> {
        files
            .iter()
            .map(|file| {
                (
                    file.path.to_string_lossy().replace('\\', "/"),
                    file.scope.as_str(),
                    file.content.clone(),
                )
            })
            .collect()
    }

    #[tokio::test]
    async fn rules_and_local_files_load_in_order() {
        let env = mem_env(
            "/proj",
            &["/agent/rules", "/proj/.icers/rules"],
            &[
                ("/agent/AGENTS.md", "GLOBAL"),
                ("/agent/rules/style.md", "GLOBAL_RULE"),
                ("/proj/AGENTS.md", "PROJECT"),
                (
                    "/proj/.icers/rules/b.md",
                    "---\ndescription: x\n---\nRULE_B",
                ),
                ("/proj/.icers/rules/a.md", "RULE_A"),
                (
                    "/proj/.icers/rules/scoped.md",
                    "---\npaths: src/**\n---\nSCOPED",
                ),
                ("/proj/.icers/rules/notes.txt", "NOT_MARKDOWN"),
                ("/proj/AGENTS.local.md", "LOCAL"),
            ],
        )
        .await;
        let files = load_project_context_files(&env, Path::new("/proj"), Path::new("/agent")).await;
        let got: Vec<String> = summary(&files)
            .into_iter()
            .map(|(_, scope, content)| format!("{scope}:{content}"))
            .collect();
        assert_eq!(
            got,
            [
                "global:GLOBAL",
                "global:GLOBAL_RULE",
                "project:PROJECT",
                "project:RULE_A",
                "project:RULE_B",
                "local:LOCAL",
            ]
        );
    }

    #[tokio::test]
    async fn includes_follow_parent_order_and_stop_at_cycles_and_depth() {
        let mut files = vec![
            (
                "/proj/AGENTS.md",
                "Top @docs/a.md and @./b.md#section\n`@code.md` ```@also-code.md```\n```\n@fenced.md\n```\nmail me@example.com",
            ),
            ("/proj/docs/a.md", "A includes @../b.md and @c.md"),
            ("/proj/docs/c.md", "C back to @../AGENTS.md"),
            ("/proj/b.md", "B"),
            ("/proj/code.md", "CODE"),
            ("/proj/fenced.md", "FENCED"),
        ];
        // A chain deeper than the include limit.
        let chain: Vec<(String, String)> = (0..=MAX_INCLUDE_DEPTH + 1)
            .map(|level| {
                (
                    format!("/proj/chain{level}.md"),
                    format!("L{level} @chain{}.md", level + 1),
                )
            })
            .collect();
        files.push(("/proj/AGENTS.local.md", "LOCAL @chain0.md"));
        let mut all: Vec<(&str, &str)> = files.clone();
        for (path, content) in &chain {
            all.push((path.as_str(), content.as_str()));
        }
        let env = mem_env("/proj", &[], &all).await;
        let loaded =
            load_project_context_files(&env, Path::new("/proj"), Path::new("/agent")).await;
        let paths: Vec<String> = summary(&loaded)
            .into_iter()
            .map(|(path, ..)| path)
            .collect();
        let mut expected = vec![
            "/proj/AGENTS.md",
            "/proj/docs/a.md",
            "/proj/b.md",
            "/proj/docs/c.md",
            "/proj/AGENTS.local.md",
        ];
        let chain_paths: Vec<String> = (0..MAX_INCLUDE_DEPTH)
            .map(|level| format!("/proj/chain{level}.md"))
            .collect();
        expected.extend(chain_paths.iter().map(String::as_str));
        assert_eq!(paths, expected);
        let a = loaded
            .iter()
            .find(|f| f.path.ends_with("docs/a.md"))
            .unwrap();
        assert_eq!(
            a.included_from.as_deref(),
            Some(Path::new("/proj/AGENTS.md"))
        );
        assert_eq!(a.scope, ContextScope::Project);
        let chain0 = loaded
            .iter()
            .find(|f| f.path.ends_with("chain0.md"))
            .unwrap();
        assert_eq!(chain0.scope, ContextScope::Local);
    }

    #[tokio::test]
    async fn project_includes_stay_inside_the_repository() {
        let env = mem_env(
            "/repo/sub",
            &["/repo/.git"],
            &[
                (
                    "/repo/sub/AGENTS.md",
                    "SUB @../shared.md @/home/me/secret.md @../../outside.md @logo.png",
                ),
                ("/repo/shared.md", "SHARED"),
                ("/repo/sub/logo.png", "PNG"),
                ("/home/me/secret.md", "SECRET"),
                ("/outside.md", "OUTSIDE"),
                ("/agent/AGENTS.md", "GLOBAL @/home/me/secret.md"),
            ],
        )
        .await;
        let loaded =
            load_project_context_files(&env, Path::new("/repo/sub"), Path::new("/agent")).await;
        let got: Vec<String> = summary(&loaded).into_iter().map(|(_, _, c)| c).collect();
        // The global file may include anything; the project file only paths
        // inside /repo, and never binary files.
        assert_eq!(
            got,
            vec![
                "GLOBAL @/home/me/secret.md",
                "SECRET",
                "SUB @../shared.md @/home/me/secret.md @../../outside.md @logo.png",
                "SHARED"
            ]
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_symlink_cannot_carry_an_include_out_of_the_repository() {
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("repo");
        let outside = root.path().join("outside");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret.md"), "SECRET").unwrap();
        std::os::unix::fs::symlink(outside.join("secret.md"), repo.join("link.md")).unwrap();
        // A symlinked instruction file inside the repo still loads.
        std::fs::write(repo.join("real.md"), "REAL @link.md").unwrap();
        std::os::unix::fs::symlink(repo.join("real.md"), repo.join("AGENTS.md")).unwrap();
        let env: Arc<dyn ExecutionEnv> =
            Arc::new(ice_tools::OsExecutionEnv::with_cwd(repo.clone()));
        let loaded = load_project_context_files(&env, &repo, &root.path().join("agent")).await;
        let got: Vec<String> = summary(&loaded).into_iter().map(|(_, _, c)| c).collect();
        assert_eq!(got, vec!["REAL @link.md"]);
    }

    #[tokio::test]
    async fn nested_worktree_skips_the_main_checkouts_copies() {
        let env = mem_env(
            "/main/.worktrees/wt",
            &["/main/.git/worktrees/wt", "/main/.worktrees/wt"],
            &[
                ("/main/AGENTS.md", "MAIN_COPY"),
                ("/main/AGENTS.local.md", "MAIN_LOCAL"),
                ("/main/.git/worktrees/wt/commondir", "../..\n"),
                (
                    "/main/.worktrees/wt/.git",
                    "gitdir: /main/.git/worktrees/wt\n",
                ),
                ("/main/.worktrees/wt/AGENTS.md", "WORKTREE_COPY"),
                ("/AGENTS.md", "ABOVE_REPO"),
            ],
        )
        .await;
        let loaded =
            load_project_context_files(&env, Path::new("/main/.worktrees/wt"), Path::new("/agent"))
                .await;
        let got: Vec<String> = summary(&loaded).into_iter().map(|(_, _, c)| c).collect();
        assert_eq!(got, vec!["ABOVE_REPO", "MAIN_LOCAL", "WORKTREE_COPY"]);
    }

    #[tokio::test]
    async fn global_only_load_skips_every_project_file() {
        let env = mem_env(
            "/proj",
            &["/agent/rules"],
            &[
                ("/agent/AGENTS.md", "GLOBAL"),
                ("/agent/rules/r.md", "GLOBAL_RULE"),
                ("/proj/AGENTS.md", "PROJECT"),
                ("/proj/AGENTS.local.md", "LOCAL"),
            ],
        )
        .await;
        let files = load_global_context_files(&env, Path::new("/agent")).await;
        let got: Vec<String> = summary(&files).into_iter().map(|(_, _, c)| c).collect();
        assert_eq!(got, ["GLOBAL", "GLOBAL_RULE"]);
    }

    #[test]
    fn html_comments_are_removed_outside_code() {
        let text = "keep\n<!-- note -->\n<!-- multi\nline --> tail\n```\n<!-- in code -->\n```\nmid <!-- inline --> stays\n<!-- unclosed\nrest";
        assert_eq!(
            strip_html_comments(text),
            "keep\n tail\n```\n<!-- in code -->\n```\nmid <!-- inline --> stays\n<!-- unclosed\nrest"
        );
    }

    #[test]
    fn long_files_are_capped_with_a_notice() {
        let long = "é".repeat(MAX_CONTEXT_FILE_BYTES);
        let capped = cap_length(&long);
        assert!(capped.len() < MAX_CONTEXT_FILE_BYTES + 100);
        assert!(capped.ends_with("Read the file for the rest.]"));
        assert_eq!(cap_length("short"), "short");
    }

    #[test]
    fn format_marks_scope_and_inclusion() {
        let mut child = ContextFile::project("/p/docs/x.md", "x");
        child.included_from = Some(PathBuf::from("/p/AGENTS.md"));
        child.scope = ContextScope::Local;
        let out = format_project_context(&[child]);
        assert!(out.contains(
            "<project_instructions path=\"/p/docs/x.md\" scope=\"local\" included_from=\"/p/AGENTS.md\">"
        ));
        assert!(out.contains("they take precedence"));
    }

    #[test]
    fn format_empty_when_no_files() {
        assert_eq!(format_project_context(&[]), "");
    }

    #[test]
    fn format_wraps_each_file() {
        let files = vec![ContextFile::project("/p/AGENTS.md", "do x")];
        let out = format_project_context(&files);
        assert!(out.starts_with("\n\n<project_context>\n\n"));
        assert!(out.contains("<project_instructions path=\"/p/AGENTS.md\" scope=\"project\">"));
        assert!(out.contains("do x"));
        assert!(out.ends_with("</project_context>\n"));
    }
}
