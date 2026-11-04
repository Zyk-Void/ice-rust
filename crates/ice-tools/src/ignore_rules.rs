//! Hierarchical ignore rules for the in-process `find` and `grep` walks.
//!
//! ice runs `fd` and `rg`, which honor `.gitignore` files (and their own
//! `.ignore`/`.fdignore`/`.rgignore`) at every directory level. icers walks
//! through the `FileSystem` trait instead, so this module reproduces the
//! observable rules: a stack of ignore files from the repository root down to
//! the current directory, gitignore pattern syntax (comments, `!` negation,
//! trailing-`/` directory rules, anchoring by an inner or leading `/`, `**`),
//! last-match-wins precedence, and a nested repository starting its own
//! `.gitignore` scope. Global git excludes are not read.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use globset::{GlobBuilder, GlobMatcher};
use tokio_util::sync::CancellationToken;

use crate::env::{ExecutionEnv, FileKind};
use crate::error::{FileError, FileErrorCode};

/// Which tool is walking; decides the tool-specific ignore file name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IgnoreTool {
    /// `fd`: also reads `.fdignore`.
    Find,
    /// `rg`: also reads `.rgignore`.
    Grep,
}

impl IgnoreTool {
    fn file_names(self) -> [&'static str; 3] {
        match self {
            IgnoreTool::Find => [".gitignore", ".ignore", ".fdignore"],
            IgnoreTool::Grep => [".gitignore", ".ignore", ".rgignore"],
        }
    }
}

struct Rule {
    matcher: GlobMatcher,
    negate: bool,
    dir_only: bool,
}

struct IgnoreFile {
    base: PathBuf,
    /// `.gitignore`-family rules end at a nested repository boundary.
    git: bool,
    rules: Vec<Rule>,
}

/// An immutable stack of ignore files; pushing returns a new stack so each
/// directory of a walk can carry its own.
#[derive(Clone, Default)]
pub(crate) struct IgnoreStack {
    files: Vec<Arc<IgnoreFile>>,
    /// The walk root. rg and fd search an explicitly named root even when
    /// it sits inside an ignored directory, so only ancestors below it count.
    root: PathBuf,
}

impl IgnoreStack {
    /// Whether `path` (absolute) is ignored. The last matching rule wins.
    pub(crate) fn is_ignored(&self, path: &Path, is_dir: bool) -> bool {
        if path
            .ancestors()
            .skip(1)
            .take_while(|ancestor| *ancestor != self.root && ancestor.starts_with(&self.root))
            .any(|ancestor| self.matches_rules(ancestor, true))
        {
            return true;
        }
        self.matches_rules(path, is_dir)
    }

    fn matches_rules(&self, path: &Path, is_dir: bool) -> bool {
        let mut ignored = false;
        for file in &self.files {
            let Ok(rel) = path.strip_prefix(&file.base) else {
                continue;
            };
            let rel = to_posix(rel);
            if rel.is_empty() {
                continue;
            }
            for rule in &file.rules {
                if rule.dir_only && !is_dir {
                    continue;
                }
                if rule.matcher.is_match(&rel) {
                    ignored = !rule.negate;
                }
            }
        }
        ignored
    }

    /// The stack for the children of `dir`, given its directory entries.
    /// A directory that contains `.git` starts a new repository, so outer
    /// `.gitignore` rules stop applying inside it.
    pub(crate) async fn enter(
        &self,
        env: &dyn ExecutionEnv,
        dir: &Path,
        entry_names: &[String],
        tool: IgnoreTool,
        cancel: Option<&CancellationToken>,
    ) -> Result<Self, FileError> {
        let mut next = self.clone();
        if entry_names.iter().any(|n| n == ".git") {
            next.files.retain(|f| !f.git);
        }
        for name in tool.file_names() {
            if entry_names.iter().any(|n| n == name) {
                if let Some(file) = load(env, dir, name, cancel).await? {
                    next.files.push(Arc::new(file));
                }
            }
        }
        Ok(next)
    }

    /// The stack in effect at the walk root: the ignore files of every
    /// ancestor up to the enclosing repository root (and its
    /// `.git/info/exclude`). Outside a repository, ancestor ignore files
    /// apply up to the filesystem root.
    pub(crate) async fn for_root(
        env: &dyn ExecutionEnv,
        root: &Path,
        tool: IgnoreTool,
        cancel: Option<&CancellationToken>,
    ) -> Result<Self, FileError> {
        let mut ancestors: Vec<PathBuf> = Vec::new();
        let mut repo_root: Option<PathBuf> = None;
        let mut current = root.parent();
        // The root itself is entered by the walk; check whether it is a repo.
        if exists(env, &root.join(".git"), cancel).await? {
            repo_root = Some(root.to_path_buf());
        }
        while repo_root.is_none() {
            let Some(dir) = current else { break };
            ancestors.push(dir.to_path_buf());
            if exists(env, &dir.join(".git"), cancel).await? {
                repo_root = Some(dir.to_path_buf());
                break;
            }
            current = dir.parent();
        }
        let mut stack = IgnoreStack {
            root: root.to_path_buf(),
            ..IgnoreStack::default()
        };
        if let Some(repo_root) = repo_root {
            if let Some(file) =
                load(env, &repo_root.join(".git").join("info"), "exclude", cancel).await?
            {
                stack.files.push(Arc::new(IgnoreFile {
                    base: repo_root,
                    ..file
                }));
            }
        }
        for dir in ancestors.iter().rev() {
            for name in tool.file_names() {
                if let Some(file) = load(env, dir, name, cancel).await? {
                    stack.files.push(Arc::new(file));
                }
            }
        }
        Ok(stack)
    }
}

async fn exists(
    env: &dyn ExecutionEnv,
    path: &Path,
    cancel: Option<&CancellationToken>,
) -> Result<bool, FileError> {
    match env.exists(&path.to_string_lossy(), cancel).await {
        Ok(found) => Ok(found),
        Err(e) if e.code == FileErrorCode::Aborted => Err(e),
        Err(_) => Ok(false),
    }
}

async fn load(
    env: &dyn ExecutionEnv,
    dir: &Path,
    name: &str,
    cancel: Option<&CancellationToken>,
) -> Result<Option<IgnoreFile>, FileError> {
    let path = dir.join(name);
    let text = match env.read_text_file(&path.to_string_lossy(), cancel).await {
        Ok(text) => text,
        Err(e) if e.code == FileErrorCode::Aborted => return Err(e),
        // Missing or unreadable ignore files are skipped, as fd/rg do.
        Err(_) => return Ok(None),
    };
    Ok(Some(IgnoreFile {
        base: dir.to_path_buf(),
        git: name == ".gitignore" || name == "exclude",
        rules: parse(&text),
    }))
}

/// Parse gitignore syntax into glob rules relative to the file's directory.
fn parse(text: &str) -> Vec<Rule> {
    text.lines().filter_map(parse_line).collect()
}

fn parse_line(line: &str) -> Option<Rule> {
    let line = line.strip_suffix('\r').unwrap_or(line);
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let line = trim_unescaped_trailing_spaces(line);
    let (negate, line) = match line.strip_prefix('!') {
        Some(rest) => (true, rest),
        None => (false, line.strip_prefix("\\!").map_or(line, |_| &line[1..])),
    };
    let line = line.strip_prefix("\\#").map_or(line, |_| &line[1..]);
    let (dir_only, line) = match line.strip_suffix('/') {
        Some(rest) => (true, rest),
        None => (false, line),
    };
    if line.is_empty() {
        return None;
    }
    // A slash anywhere but the end anchors the pattern to this directory.
    let anchored = line.contains('/');
    let pattern = line.trim_start_matches('/');
    let glob = if anchored {
        pattern.to_string()
    } else {
        format!("**/{pattern}")
    };
    let matcher = GlobBuilder::new(&glob)
        .literal_separator(true)
        .backslash_escape(true)
        .build()
        .ok()?
        .compile_matcher();
    Some(Rule {
        matcher,
        negate,
        dir_only,
    })
}

fn trim_unescaped_trailing_spaces(line: &str) -> &str {
    let mut end = line.len();
    while end > 0 && line.as_bytes()[end - 1] == b' ' {
        if end >= 2 && line.as_bytes()[end - 2] == b'\\' {
            break;
        }
        end -= 1;
    }
    &line[..end]
}

fn to_posix(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// True for directory entries a walk must never enter.
pub(crate) fn is_vcs_dir(name: &str, kind: FileKind) -> bool {
    kind == FileKind::Directory && name == ".git"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stack(base: &str, text: &str) -> IgnoreStack {
        IgnoreStack {
            files: vec![Arc::new(IgnoreFile {
                base: PathBuf::from(base),
                git: true,
                rules: parse(text),
            })],
            root: PathBuf::new(),
        }
    }

    #[test]
    fn gitignore_syntax() {
        let s = stack(
            "/r",
            "# comment\n*.log\n!keep.log\nbuild/\n/top.txt\ndocs/*.md\n**/deep/x\n",
        );
        let ig = |p: &str, dir: bool| s.is_ignored(Path::new(p), dir);
        assert!(ig("/r/a.log", false));
        assert!(ig("/r/sub/a.log", false));
        assert!(!ig("/r/keep.log", false));
        assert!(ig("/r/build", true));
        assert!(
            !ig("/r/build", false),
            "trailing / matches directories only"
        );
        assert!(ig("/r/top.txt", false));
        assert!(!ig("/r/sub/top.txt", false), "leading / anchors");
        assert!(ig("/r/docs/a.md", false));
        assert!(!ig("/r/docs/sub/a.md", false), "* does not cross /");
        assert!(ig("/r/a/b/deep/x", false));
        assert!(
            !ig("/other/a.log", false),
            "rules apply below their base only"
        );
    }

    #[test]
    fn deeper_files_override_and_escapes_are_literal() {
        let mut s = stack("/r", "*.tmp\n");
        s.files.push(Arc::new(IgnoreFile {
            base: PathBuf::from("/r/sub"),
            git: true,
            rules: parse("!*.tmp\n\\#hash\nspace\\ \n"),
        }));
        assert!(s.is_ignored(Path::new("/r/a.tmp"), false));
        assert!(!s.is_ignored(Path::new("/r/sub/a.tmp"), false));
        assert!(s.is_ignored(Path::new("/r/sub/#hash"), false));
        assert!(s.is_ignored(Path::new("/r/sub/space "), false));
    }

    #[test]
    fn negation_cannot_reinclude_a_file_under_an_ignored_directory() {
        let s = stack("/r", "build/\n!build/keep.txt\n");
        assert!(s.is_ignored(Path::new("/r/build"), true));
        assert!(s.is_ignored(Path::new("/r/build/keep.txt"), false));
    }

    #[test]
    fn an_explicit_root_inside_an_ignored_directory_is_searched() {
        let mut s = stack("/r", "build/\n");
        s.root = PathBuf::from("/r/build");
        assert!(!s.is_ignored(Path::new("/r/build/keep.txt"), false));
        assert!(s.is_ignored(Path::new("/r/build/build/x.txt"), false));
    }
}
