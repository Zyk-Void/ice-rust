//! Cached local repository applicability, independent of memory dataset naming.
//!
//! Linked worktrees share a canonical common-directory identity, but retain a
//! separate worktree, branch and revision. No Git process, index walk or network
//! operation is needed. A small set of metadata stamps detects external Git
//! changes; content is reread only when those stamps change.

use std::fs::{self, File, Metadata};
use std::io::{BufRead, BufReader, Read};
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

use sha2::{Digest, Sha256};

use super::config::find_git_root;

const MAX_CONTROL_BYTES: u64 = 16 * 1024;
const MAX_PACKED_REF_BYTES: u64 = 4 * 1024 * 1024;

/// A snapshot, not proof that working-tree files match the commit. Volatile
/// facts must still be verified with authoritative tools before acting.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RepositoryIdentity {
    pub root: PathBuf,
    pub git_dir: Option<PathBuf>,
    pub common_dir: Option<PathBuf>,
    pub repository_id: String,
    pub worktree_id: String,
    pub branch: Option<String>,
    pub revision: Option<String>,
}

impl RepositoryIdentity {
    /// Resolve bounded Git control files. Unknown or malformed revision state
    /// remains unknown; it is never replaced with the current branch's guess.
    pub fn read(cwd: &Path) -> Self {
        let lexical_root = find_git_root(cwd);
        let root = fs::canonicalize(&lexical_root).unwrap_or(lexical_root);
        let git_dir = git_directory(&root);
        let common_dir = git_dir.as_ref().and_then(|git_dir| {
            let common = match read_control(&git_dir.join("commondir")) {
                Some(common) => join_control_path(git_dir, common.trim())?,
                None if !git_dir.join("commondir").exists() => git_dir.clone(),
                None => return None,
            };
            fs::canonicalize(common).ok().filter(|path| path.is_dir())
        });
        let mut identity = Self {
            repository_id: stable_id(common_dir.as_deref().unwrap_or(&root)),
            worktree_id: stable_id(&root),
            root,
            git_dir,
            common_dir,
            branch: None,
            revision: None,
        };
        let Some(git_dir) = identity.git_dir.as_ref() else {
            return identity;
        };
        let Some(head) = read_control(&git_dir.join("HEAD")) else {
            return identity;
        };
        let head = head.trim();
        if let Some(reference) = head.strip_prefix("ref: ") {
            if !valid_reference(reference) {
                return identity;
            }
            identity.branch = reference.strip_prefix("refs/heads/").map(str::to_owned);
            let Some(common) = identity.common_dir.as_ref() else {
                // A branch name without a usable common directory is not a
                // complete identity; do not admit branch-restricted facts.
                identity.branch = None;
                return identity;
            };
            identity.revision = read_reference(common, reference);
        } else if valid_object_id(head) {
            identity.revision = Some(head.to_ascii_lowercase());
        }
        identity
    }

    /// Identity used to fence optional results. Revision changes invalidate
    /// in-flight work even when the repository's dataset remains the same.
    pub fn applicability_key(&self) -> String {
        format!(
            "{}:{}:{}:{}",
            self.repository_id,
            self.worktree_id,
            self.branch.as_deref().unwrap_or("<detached-or-unknown>"),
            self.revision.as_deref().unwrap_or("<unknown>"),
        )
    }

    fn watched_paths(&self) -> Vec<PathBuf> {
        let mut paths = vec![self.root.join(".git")];
        if let Some(git_dir) = &self.git_dir {
            paths.push(git_dir.join("HEAD"));
            paths.push(git_dir.join("commondir"));
        }
        if let Some(common) = &self.common_dir {
            paths.push(common.join("packed-refs"));
            if let Some(branch) = &self.branch {
                paths.push(common.join("refs/heads").join(branch));
            }
        }
        paths
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Stamp {
    len: u64,
    modified: Option<SystemTime>,
    created: Option<SystemTime>,
    file_identity: u64,
}

impl From<Metadata> for Stamp {
    fn from(value: Metadata) -> Self {
        #[cfg(unix)]
        let file_identity = {
            use std::os::unix::fs::MetadataExt;
            value.ino()
        };
        #[cfg(not(unix))]
        let file_identity = 0;
        Self {
            len: value.len(),
            modified: value.modified().ok(),
            created: value.created().ok(),
            file_identity,
        }
    }
}

/// The initialized fast path does only a fixed, small number of metadata
/// checks. It never scans directories or shells out. A caller can request a
/// forced refresh after an authoritative repository mutation/tool result.
#[derive(Clone, Debug, Default)]
pub struct IdentityTracker {
    input_cwd: Option<PathBuf>,
    identity: Option<RepositoryIdentity>,
    watched: Vec<(PathBuf, Option<Stamp>)>,
}

impl IdentityTracker {
    pub fn snapshot(&self) -> Option<&RepositoryIdentity> {
        self.identity.as_ref()
    }

    pub fn refresh(&mut self, cwd: &Path, force: bool) -> bool {
        let same_root = self.input_cwd.as_deref() == Some(cwd);
        if !force
            && same_root
            && self.identity.is_some()
            && self
                .watched
                .iter()
                .all(|(path, stamp)| metadata_stamp(path) == *stamp)
        {
            return false;
        }
        let next = RepositoryIdentity::read(cwd);
        let changed = self.identity.as_ref() != Some(&next);
        self.watched = next
            .watched_paths()
            .into_iter()
            .map(|path| {
                let stamp = metadata_stamp(&path);
                (path, stamp)
            })
            .collect();
        self.identity = Some(next);
        self.input_cwd = Some(cwd.to_path_buf());
        changed
    }
}

fn metadata_stamp(path: &Path) -> Option<Stamp> {
    fs::metadata(path).ok().map(Into::into)
}

fn stable_id(path: &Path) -> String {
    format!("{:x}", Sha256::digest(path.to_string_lossy().as_bytes()))
}

fn read_control(path: &Path) -> Option<String> {
    let file = File::open(path).ok()?;
    if !file.metadata().ok()?.is_file() || file.metadata().ok()?.len() > MAX_CONTROL_BYTES {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(MAX_CONTROL_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_CONTROL_BYTES {
        return None;
    }
    String::from_utf8(bytes).ok()
}

fn join_control_path(base: &Path, value: &str) -> Option<PathBuf> {
    if value.is_empty() || value.contains(['\n', '\r', '\0']) {
        return None;
    }
    let path = Path::new(value);
    Some(if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    })
}

fn git_directory(root: &Path) -> Option<PathBuf> {
    let path = root.join(".git");
    let git_dir = if path.is_dir() {
        path
    } else {
        let text = read_control(&path)?;
        join_control_path(root, text.trim().strip_prefix("gitdir:")?.trim())?
    };
    fs::canonicalize(git_dir).ok().filter(|path| path.is_dir())
}

fn valid_reference(value: &str) -> bool {
    value.starts_with("refs/")
        && value.len() <= 1024
        && !value.ends_with('/')
        && !value.ends_with('.')
        && !value.ends_with(".lock")
        && !value.contains("..")
        && !value.contains("@{")
        && !value.contains("//")
        && !value
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || "~^:?*[\\".contains(c))
        && Path::new(value)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
        && value.split('/').all(|part| !part.starts_with('.'))
}

fn valid_object_id(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|b| b.is_ascii_hexdigit())
}

fn read_reference(common: &Path, reference: &str) -> Option<String> {
    let path = common.join(reference);
    if let Some(value) = read_control(&path) {
        let value = value.trim();
        // Symbolic references beyond HEAD are conservatively unknown.
        return valid_object_id(value).then(|| value.to_ascii_lowercase());
    }
    // An existing but unreadable/malformed loose ref must not expose an older
    // packed ref for the same name.
    if path.exists() {
        return None;
    }
    let file = File::open(common.join("packed-refs")).ok()?;
    if !file.metadata().ok()?.is_file() || file.metadata().ok()?.len() > MAX_PACKED_REF_BYTES {
        return None;
    }
    let reader = BufReader::new(file.take(MAX_PACKED_REF_BYTES));
    for line in reader.lines() {
        let line = line.ok()?;
        if line.starts_with(['#', '^']) {
            continue;
        }
        let Some((object, name)) = line.split_once(' ') else {
            continue;
        };
        if name == reference {
            return valid_object_id(object).then(|| object.to_ascii_lowercase());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn main_repo(root: &Path) {
        fs::create_dir_all(root.join(".git/refs/heads")).unwrap();
        fs::write(root.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::write(root.join(".git/refs/heads/main"), "a".repeat(40)).unwrap();
    }

    #[test]
    fn linked_worktrees_share_repo_but_not_branch_or_worktree() {
        let dir = tempfile::tempdir().unwrap();
        let main = dir.path().join("main");
        main_repo(&main);
        let linked = dir.path().join("linked");
        fs::create_dir_all(&linked).unwrap();
        let administrative = main.join(".git/worktrees/linked");
        fs::create_dir_all(&administrative).unwrap();
        fs::write(
            linked.join(".git"),
            format!("gitdir: {}", administrative.display()),
        )
        .unwrap();
        fs::write(administrative.join("commondir"), "../..\n").unwrap();
        fs::write(administrative.join("HEAD"), "ref: refs/heads/feature\n").unwrap();
        fs::write(main.join(".git/refs/heads/feature"), "b".repeat(40)).unwrap();
        let a = RepositoryIdentity::read(&main);
        let b = RepositoryIdentity::read(&linked);
        assert_eq!(a.repository_id, b.repository_id);
        assert_ne!(a.worktree_id, b.worktree_id);
        assert_ne!(a.applicability_key(), b.applicability_key());
        assert_eq!(a.branch.as_deref(), Some("main"));
        assert_eq!(b.branch.as_deref(), Some("feature"));
        assert_eq!(b.revision.as_deref(), Some("b".repeat(40).as_str()));
    }

    #[test]
    fn detached_unborn_and_malformed_refs_remain_distinct() {
        let dir = tempfile::tempdir().unwrap();
        main_repo(dir.path());
        let head = dir.path().join(".git/HEAD");
        fs::write(&head, "c".repeat(64)).unwrap();
        let detached = RepositoryIdentity::read(dir.path());
        assert!(detached.branch.is_none());
        assert_eq!(detached.revision, Some("c".repeat(64)));
        fs::write(&head, "ref: refs/heads/unborn\n").unwrap();
        let unborn = RepositoryIdentity::read(dir.path());
        assert_eq!(unborn.branch.as_deref(), Some("unborn"));
        assert!(unborn.revision.is_none());
        fs::write(&head, "ref: refs/heads/../../outside\n").unwrap();
        let malformed = RepositoryIdentity::read(dir.path());
        assert!(malformed.branch.is_none() && malformed.revision.is_none());
    }

    #[test]
    fn packed_refs_supported_but_invalid_loose_ref_does_not_fall_back() {
        let dir = tempfile::tempdir().unwrap();
        main_repo(dir.path());
        fs::write(
            dir.path().join(".git/packed-refs"),
            format!("# pack-refs\n{} refs/heads/main\n", "d".repeat(40)),
        )
        .unwrap();
        fs::remove_file(dir.path().join(".git/refs/heads/main")).unwrap();
        assert_eq!(
            RepositoryIdentity::read(dir.path()).revision,
            Some("d".repeat(40))
        );
        fs::write(dir.path().join(".git/refs/heads/main"), "invalid").unwrap();
        assert!(RepositoryIdentity::read(dir.path()).revision.is_none());
    }

    #[test]
    fn tracker_detects_external_branch_and_revision_change_without_process() {
        let dir = tempfile::tempdir().unwrap();
        main_repo(dir.path());
        let mut tracker = IdentityTracker::default();
        assert!(tracker.refresh(dir.path(), false));
        assert!(!tracker.refresh(dir.path(), false));
        fs::write(dir.path().join(".git/HEAD.new"), "ref: refs/heads/other\n").unwrap();
        fs::rename(
            dir.path().join(".git/HEAD.new"),
            dir.path().join(".git/HEAD"),
        )
        .unwrap();
        fs::write(dir.path().join(".git/refs/heads/other"), "e".repeat(40)).unwrap();
        assert!(tracker.refresh(dir.path(), false));
        assert_eq!(tracker.snapshot().unwrap().branch.as_deref(), Some("other"));
        fs::write(dir.path().join(".git/refs/heads/other.new"), "f".repeat(40)).unwrap();
        fs::rename(
            dir.path().join(".git/refs/heads/other.new"),
            dir.path().join(".git/refs/heads/other"),
        )
        .unwrap();
        assert!(tracker.refresh(dir.path(), false));
        assert_eq!(tracker.snapshot().unwrap().revision, Some("f".repeat(40)));
    }

    #[test]
    fn non_git_and_unrelated_repositories_never_share_identity() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        fs::create_dir_all(&a).unwrap();
        main_repo(&b);
        let a = RepositoryIdentity::read(&a);
        let b = RepositoryIdentity::read(&b);
        assert_ne!(a.repository_id, b.repository_id);
        assert!(a.branch.is_none() && a.revision.is_none());
    }

    #[test]
    fn oversized_head_is_unknown_not_a_large_read() {
        let dir = tempfile::tempdir().unwrap();
        main_repo(dir.path());
        fs::write(
            dir.path().join(".git/HEAD"),
            "a".repeat(MAX_CONTROL_BYTES as usize + 1),
        )
        .unwrap();
        assert!(RepositoryIdentity::read(dir.path()).revision.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn nested_path_and_symlink_resolve_to_the_same_canonical_repository() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repository");
        let nested = root.join("src/nested");
        fs::create_dir_all(&nested).unwrap();
        main_repo(&root);
        let alias = dir.path().join("repository-link");
        std::os::unix::fs::symlink(&root, &alias).unwrap();

        let from_nested = RepositoryIdentity::read(&nested);
        let from_symlink = RepositoryIdentity::read(&alias.join("src"));
        assert_eq!(from_nested.root, root.canonicalize().unwrap());
        assert_eq!(from_nested.repository_id, from_symlink.repository_id);
        assert_eq!(from_nested.worktree_id, from_symlink.worktree_id);
    }
}
