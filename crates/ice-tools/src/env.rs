//! Mirrors `packages/agent/src/harness/types.ts::ExecutionEnv` — the trait every
//! tool and test-double implements. `ExecutionEnv = FileSystem + Shell`.
//!
//! Contract (plan §5.13): **every method must never panic.** All failures are
//! encoded in `Result<_, FileError>` / `Result<_, ExecutionError>`. Paths may be
//! absolute or relative to [`FileSystem::cwd`]; returned paths are "addressed"
//! (symlink-canonicalized only by [`FileSystem::canonical_path`]).
//!
//! The TS `AbortSignal` becomes `&tokio_util::sync::CancellationToken` on every
//! method. Backends SHOULD check `is_cancelled()` at entry and honor cooperative
//! cancellation during blocking ops via `tokio::select!`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use crate::error::{ExecutionError, FileError, FileErrorCode};

/// `file` | `directory` | `symlink`. Mirrors TS `FileKind`.
///
/// Backends detect via `symlink_metadata` (lstat) — symlinks are NOT followed
/// automatically by any `FileSystem` method except `canonical_path`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    File,
    Directory,
    Symlink,
}

/// Mirrors TS `FileInfo`. `path` is the absolute addressed (resolved) path —
/// NOT symlink-canonicalized. `mtime_ms` is ms since Unix epoch.
#[derive(Debug, Clone)]
pub struct FileInfo {
    pub name: String,
    pub path: PathBuf,
    pub kind: FileKind,
    pub size: u64,
    pub mtime_ms: i64,
}

/// Write/append payload. Mirrors the TS `string | Uint8Array` union.
#[derive(Debug, Clone)]
pub enum FileContent {
    Text(String),
    Bytes(Vec<u8>),
}

impl FileContent {
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            FileContent::Text(s) => s.as_bytes(),
            FileContent::Bytes(b) => b,
        }
    }
}

impl From<String> for FileContent {
    fn from(s: String) -> Self {
        FileContent::Text(s)
    }
}

impl From<&str> for FileContent {
    fn from(s: &str) -> Self {
        FileContent::Text(s.to_string())
    }
}

impl From<Vec<u8>> for FileContent {
    fn from(b: Vec<u8>) -> Self {
        FileContent::Bytes(b)
    }
}

/// Result of `Shell::exec`. Mirrors the TS inline return type
/// `{ stdout, stderr, exitCode }`. Signal kills collapse to `exit_code: 0`
/// (matching the TS `code ?? 0` rule).
#[derive(Debug, Clone)]
pub struct ShellOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

/// Mirrors TS `ShellExecOptions` (note: `timeout` is in **seconds**, matching
/// the TS contract). `on_stdout` / `on_stderr` fire per chunk, interleaved in
/// arrival order; the bash-tool wraps these with throttling + capture.
#[derive(Clone, Debug)]
pub struct ShellOverride {
    pub program: PathBuf,
    pub args: Vec<String>,
}

pub struct ShellExecOptions<'a> {
    /// Optional executable and fixed arguments. `None` keeps the environment's
    /// default shell (Bash on POSIX/Windows), while tools such as PowerShell
    /// can select their native interpreter without going through Bash.
    pub shell: Option<ShellOverride>,
    pub cwd: Option<PathBuf>,
    pub env: Option<HashMap<String, String>>,
    /// Default `true`. When `false`, ONLY `env` is used (no process env, no
    /// shell base env).
    pub inherit_env: bool,
    /// Seconds. `None` = no timeout.
    pub timeout: Option<f64>,
    pub cancel: Option<&'a CancellationToken>,
    /// Per-chunk output callbacks. A stream with a callback is not also
    /// buffered into [`ShellOutput`], so long-running commands stay bounded.
    pub on_stdout: Option<Box<dyn FnMut(&str) + Send + 'a>>,
    pub on_stderr: Option<Box<dyn FnMut(&str) + Send + 'a>>,
    /// Called once with the child's process id (its process-group id on
    /// POSIX) right after it starts. Mirrors TS `onSpawn`.
    pub on_spawn: Option<Box<dyn FnOnce(u32) + Send + 'a>>,
    /// Keep the child's stdin open and feed it from this channel (bash_input).
    /// Without it, stdin is closed at once so prompts see EOF. Backends that
    /// deliver the command itself over stdin reject this.
    pub stdin: Option<ShellStdin>,
}

/// One request to a running shell's stdin. Mirrors TS `onStdin` writes.
#[derive(Debug)]
pub enum StdinRequest {
    /// Write these bytes; `done` reports when they were accepted by the pipe.
    Write {
        bytes: Vec<u8>,
        done: tokio::sync::oneshot::Sender<Result<(), String>>,
    },
    /// Close stdin (EOF).
    Close,
}

/// Receiving end of a shell's interactive stdin.
pub type ShellStdin = tokio::sync::mpsc::UnboundedReceiver<StdinRequest>;

/// Error for a shell that receives its command over stdin.
pub const STDIN_TRANSPORT_CONFLICT: &str =
    "This shell reads its command from stdin, so the command's stdin cannot be kept open.";

impl<'a> Default for ShellExecOptions<'a> {
    fn default() -> Self {
        Self {
            shell: None,
            cwd: None,
            env: None,
            inherit_env: true,
            timeout: None,
            cancel: None,
            on_stdout: None,
            on_stderr: None,
            on_spawn: None,
            stdin: None,
        }
    }
}

impl<'a> ShellExecOptions<'a> {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Mirrors TS `FileSystem`. All methods async, never panic.
///
/// `cancel: Option<&CancellationToken>` is the Rust analogue of the optional
/// `abortSignal`; backends SHOULD check it at entry (returning
/// `FileError::aborted()`) and honor cooperative cancel during long ops.
#[async_trait]
pub trait FileSystem: Send + Sync {
    /// The environment's working directory (absolute).
    fn cwd(&self) -> &Path;

    /// Local OS path when this backend directly addresses the host filesystem.
    /// Used for native session advisory ownership, not for tool I/O. Virtual or
    /// remote backends return `None` and must supply their own writer policy.
    fn native_path(&self, _path: &str) -> Option<PathBuf> {
        None
    }

    /// Resolve `path` (relative-to-cwd, `~`, `file://`, absolute) to an absolute
    /// addressed path. Pure — no FS access, no abort. Mirrors TS `absolutePath`.
    async fn absolute_path(
        &self,
        path: &str,
        cancel: Option<&CancellationToken>,
    ) -> Result<PathBuf, FileError>;

    /// Join path components. Pure. Mirrors TS `joinPath`.
    async fn join_path(
        &self,
        parts: &[&str],
        cancel: Option<&CancellationToken>,
    ) -> Result<PathBuf, FileError>;

    async fn read_text_file(
        &self,
        path: &str,
        cancel: Option<&CancellationToken>,
    ) -> Result<String, FileError>;

    /// Stream-read up to `max_lines` lines (split on `\n`; `max_lines <= 0`
    /// → empty). Mirrors TS `readTextLines{maxLines}`.
    async fn read_text_lines(
        &self,
        path: &str,
        max_lines: Option<usize>,
        cancel: Option<&CancellationToken>,
    ) -> Result<Vec<String>, FileError>;

    async fn read_binary_file(
        &self,
        path: &str,
        cancel: Option<&CancellationToken>,
    ) -> Result<Vec<u8>, FileError>;

    /// Read at most the newest `max_bytes` bytes and return them with the
    /// number of earlier bytes skipped. Backends may override this to seek
    /// without loading the whole file; the default preserves compatibility.
    async fn read_binary_tail(
        &self,
        path: &str,
        max_bytes: u64,
        cancel: Option<&CancellationToken>,
    ) -> Result<(Vec<u8>, u64), FileError> {
        let bytes = self.read_binary_file(path, cancel).await?;
        let skipped = (bytes.len() as u64).saturating_sub(max_bytes);
        let start = usize::try_from(skipped).unwrap_or(bytes.len());
        Ok((bytes[start..].to_vec(), skipped))
    }

    /// Creates parent directories (recursive) automatically. Mirrors TS
    /// `writeFile`.
    async fn write_file(
        &self,
        path: &str,
        content: FileContent,
        cancel: Option<&CancellationToken>,
    ) -> Result<(), FileError>;

    async fn append_file(
        &self,
        path: &str,
        content: FileContent,
        cancel: Option<&CancellationToken>,
    ) -> Result<(), FileError>;

    async fn rename_file(
        &self,
        source: &str,
        dest: &str,
        cancel: Option<&CancellationToken>,
    ) -> Result<(), FileError>;

    /// Uses `symlink_metadata` (lstat — symlinks not followed). Mirrors TS
    /// `fileInfo`.
    async fn file_info(
        &self,
        path: &str,
        cancel: Option<&CancellationToken>,
    ) -> Result<FileInfo, FileError>;

    async fn list_dir(
        &self,
        path: &str,
        cancel: Option<&CancellationToken>,
    ) -> Result<Vec<FileInfo>, FileError>;

    /// Follows symlinks (`canonicalize`). Mirrors TS `canonicalPath`. Backends
    /// may return `not_found` or `not_supported` for missing/unsupported paths;
    /// callers (mutation queue) fall back to absolute path on those codes.
    async fn canonical_path(
        &self,
        path: &str,
        cancel: Option<&CancellationToken>,
    ) -> Result<PathBuf, FileError>;

    /// The target of the symlink at `path`, unresolved. Backends without
    /// symlinks keep the default `not_supported`.
    async fn read_link(
        &self,
        path: &str,
        cancel: Option<&CancellationToken>,
    ) -> Result<PathBuf, FileError> {
        let _ = cancel;
        Err(
            FileError::new(FileErrorCode::NotSupported, "symlinks are not supported")
                .with_path(path),
        )
    }

    /// `false` ONLY for `not_found`; other errors propagate. Mirrors TS
    /// `exists` (which delegates to `fileInfo`, NOT `access(F_OK)`).
    async fn exists(
        &self,
        path: &str,
        cancel: Option<&CancellationToken>,
    ) -> Result<bool, FileError>;

    /// Default `recursive=true`. Mirrors TS `createDir{recursive}`.
    async fn create_dir(
        &self,
        path: &str,
        recursive: bool,
        cancel: Option<&CancellationToken>,
    ) -> Result<(), FileError>;

    /// Defaults `recursive=false`, `force=false`. Mirrors TS `remove`.
    async fn remove(
        &self,
        path: &str,
        recursive: bool,
        force: bool,
        cancel: Option<&CancellationToken>,
    ) -> Result<(), FileError>;

    /// Default prefix `"tmp-"`. Mirrors TS `createTempDir{prefix}`.
    async fn create_temp_dir(
        &self,
        prefix: Option<&str>,
        cancel: Option<&CancellationToken>,
    ) -> Result<PathBuf, FileError>;

    /// Mirrors TS `createTempFile{prefix,suffix}` (defaults empty). Note: the
    /// TS impl creates an isolated temp **dir** per file; backends may simplify
    /// to a single file in the OS temp dir (document any divergence).
    async fn create_temp_file(
        &self,
        prefix: &str,
        suffix: &str,
        cancel: Option<&CancellationToken>,
    ) -> Result<PathBuf, FileError>;

    /// Best-effort, never returns `Err`. Mirrors TS `cleanup` (kills tracked
    /// child processes, etc.).
    async fn cleanup(&self);
}

/// Mirrors TS `Shell`.
#[async_trait]
pub trait Shell: Send + Sync {
    /// Execute `command` in a bash shell. The TS env resolves a bash binary
    /// (git-bash on Windows, `/bin/bash` on POSIX, `/bin/sh` fallback on POSIX).
    /// There is NO cmd.exe fallback on Windows — failure to find bash is
    /// `ExecutionError::ShellUnavailable`.
    async fn exec<'a>(
        &'a self,
        command: &str,
        options: ShellExecOptions<'a>,
    ) -> Result<ShellOutput, ExecutionError>;

    /// Whether this environment can execute a program with separate arguments.
    fn supports_exec_program(&self) -> bool {
        false
    }

    /// Execute a program without interpreting its arguments as shell source.
    async fn exec_program<'a>(
        &'a self,
        _program: &str,
        _args: &[String],
        _options: ShellExecOptions<'a>,
    ) -> Result<ShellOutput, ExecutionError> {
        Err(ExecutionError::new(
            crate::error::ExecutionErrorCode::ShellUnavailable,
            "direct program execution is not supported by this environment",
        ))
    }

    /// Best-effort, never returns `Err`. Mirrors TS `Shell.cleanup`.
    async fn cleanup(&self);
}

/// Mirrors TS `ExecutionEnv extends FileSystem, Shell`. A marker combining both
/// capabilities — tools receive `Arc<dyn ExecutionEnv>` (or a concrete env) and
/// use `FileSystem` + `Shell` methods interchangeably.
#[async_trait]
pub trait ExecutionEnv: FileSystem + Shell {}

/// Check `cancel` at a method entry point, returning `FileError::aborted()`
/// when set. Mirrors the TS `abortResult<T>(signal, path?)` helper used at the
/// top of every `FileSystem` method.
pub fn check_cancel_file(
    cancel: Option<&CancellationToken>,
    path: Option<&str>,
) -> Result<(), FileError> {
    if let Some(t) = cancel {
        if t.is_cancelled() {
            let mut err = FileError::aborted();
            if let Some(p) = path {
                err = err.with_path(p);
            }
            return Err(err);
        }
    }
    Ok(())
}

/// Same as [`check_cancel_file`] but for `Shell`-side (returns
/// `ExecutionError::aborted()`).
pub fn check_cancel_exec(cancel: Option<&CancellationToken>) -> Result<(), ExecutionError> {
    if let Some(t) = cancel {
        if t.is_cancelled() {
            return Err(ExecutionError::aborted());
        }
    }
    Ok(())
}
