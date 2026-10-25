//! Mirrors `packages/agent/src/harness/env/nodejs.ts` — `NodeExecutionEnv` →
//! `OsExecutionEnv`. Real-filesystem + real-process backend via `tokio::fs` +
//! `tokio::process`.
//!
//! Path resolution (\[`resolvePath`\]): tilde (`~`/`~/`, `~\` on Windows),
//! `file://` URL (parse failure silently keeps the literal string), absolute
//! → `canonicalize`-style normalize, relative → `cwd.join`. No env-var or
//! `~user` expansion. No symlink resolution (that's `canonical_path`).
//!
//! Shell detection (\[`getShellConfig`\]): explicit override → Windows
//! git-bash probe (`ProgramFiles\Git\bin\bash.exe`, `ProgramFiles(x86)\…`) →
//! `where bash.exe` → POSIX `/bin/bash` → `which bash` → `/bin/sh` fallback
//! (POSIX only). **No cmd.exe fallback** on Windows — failure is
//! `ExecutionError::ShellUnavailable`. Legacy WSL bash (`System32\bash.exe`)
//! uses `bash -s` stdin transport to avoid argv parse bugs.
//!
//! Kill tree: Windows `taskkill /F /T /PID`; POSIX `kill -PG` (process group,
//! since the child is spawned as its own group leader) with fallback to
//! `kill -PID`. Best-effort, errors swallowed.
//!
//! `exec` abort+timeout bracketing (plan §5.4): the spawn is `tokio::select!`-ed
//! against the cancellation token and `tokio::time::timeout`. Resolution
//! priority: callbackError → timeout → aborted → exit-code-from-success.
//! `cleanup()` kills tracked child pids best-effort.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::env::{
    check_cancel_exec, check_cancel_file, ExecutionEnv, FileContent, FileInfo, FileKind,
    FileSystem, Shell, ShellExecOptions, ShellOutput, StdinRequest, STDIN_TRANSPORT_CONFLICT,
};
use crate::error::{
    io_to_file_error, ExecutionError, ExecutionErrorCode, FileError, FileErrorCode,
};
use crate::file_mutation_queue::{MutatingEnv, MutationQueueRegistry};

/// Max timeout in ms (Int32 max). Mirrors `MAX_TIMEOUT_MS`.
const MAX_TIMEOUT_MS: u64 = 2_147_483_647;
/// Max timeout in seconds. Mirrors `MAX_TIMEOUT_SECONDS`.
const MAX_TIMEOUT_SECONDS: f64 = 2_147_483.647;
/// Grace period after child exit for trailing stdio. Mirrors
/// `EXIT_STDIO_GRACE_MS`.
const EXIT_STDIO_GRACE_MS: u64 = 100;

/// How the command is fed to the shell. Mirrors `commandTransport`.
#[derive(Clone, Copy)]
enum CommandTransport {
    /// Append the command as the last argv element (`bash -c <cmd>`).
    Argv,
    /// Pipe the command to the shell's stdin (`bash -s`).
    Stdin,
}

/// Resolved shell config. Mirrors `ShellConfig`.
struct ShellConfig {
    shell: PathBuf,
    args: Vec<String>,
    transport: CommandTransport,
}

/// Environment variables the **host** sets for its own runtime plumbing, which
/// must not be handed to a shell it spawns.
///
/// A spawned shell is a child process, so anything inherited reaches whatever
/// that shell runs — including another `icers`. `ICERS_SESSION_ID` names the session
/// *this process* serves, which is meaningless (and actively misleading) in a
/// descendant: the child would either report the parent's session or, seeing the
/// variable already present, refuse to publish its own. Both are wrong answers,
/// and neither is visible from the child.
///
/// This is only the host's own half. An *extension* can publish variables with
/// the same hazard, and it declares them itself; [`is_host_private_env`] is the
/// function that answers the full question. Use it rather than reading this
/// slice, or an extension's channel will be missed — which is precisely the bug
/// that let `LANGFUSE_PI_PARENT_*` through.
pub const HOST_PRIVATE_ENV: &[&str] = &[
    ice_plugin_sdk::SESSION_ID_ENV,
    // The rest of the per-call session identity bash sets explicitly (see
    // `tools::bash::SESSION_ENV_NAMES`); an inherited copy would be stale.
    "ICERS_SESSION_FILE",
    "ICERS_PROVIDER",
    "ICERS_MODEL",
    "ICERS_REASONING_LEVEL",
];

/// Whether `name` must not be inherited by a spawned shell.
///
/// Two sources, because the variables come from two places:
///
/// - [`HOST_PRIVATE_ENV`] — the host's own plumbing.
/// - [`ice_plugin_sdk::SUBAGENT_PARENT_ENV`] — the family an *extension*
///   declares for subagent nesting. An inherited copy makes a plain `icers` started
///   from inside `bash` adopt its ancestor's trace, hang its root off the
///   ancestor's root span, and call itself `Subagent Turn` instead of opening its
///   own trace. A launcher that sets these on the child *it* starts is the
///   supported nesting path, so this is about not *inheriting* them, not about
///   forbidding the names.
///
/// The extension family is consulted **as the SDK's slice**, never as names
/// restated here. Restating them is what the previous version did, and it is
/// how the channel re-leaked when the plugin renamed them: the list went on
/// matching a name nobody set. Going through the constant means a rename is a
/// compile error, and a newly added family member is excluded automatically.
pub fn is_host_private_env(name: &str) -> bool {
    HOST_PRIVATE_ENV.contains(&name) || ice_plugin_sdk::SUBAGENT_PARENT_ENV.contains(&name)
}

/// Real-OS `ExecutionEnv`. Clone shares the tracked-pid set + registry.
pub struct OsExecutionEnv {
    cwd: PathBuf,
    shell_path: Option<PathBuf>,
    shell_env: Option<HashMap<String, String>>,
    active_pids: Arc<Mutex<HashSet<u32>>>,
    registry: Arc<MutationQueueRegistry>,
}

impl OsExecutionEnv {
    pub fn new() -> Self {
        Self {
            cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")),
            shell_path: None,
            shell_env: None,
            active_pids: Arc::new(Mutex::new(HashSet::new())),
            registry: Arc::new(MutationQueueRegistry::new()),
        }
    }

    pub fn with_cwd(cwd: PathBuf) -> Self {
        Self {
            cwd,
            shell_path: None,
            shell_env: None,
            active_pids: Arc::new(Mutex::new(HashSet::new())),
            registry: Arc::new(MutationQueueRegistry::new()),
        }
    }

    pub fn with_shell_path(mut self, shell_path: PathBuf) -> Self {
        self.shell_path = Some(shell_path);
        self
    }

    pub fn with_shell_env(mut self, env: HashMap<String, String>) -> Self {
        self.shell_env = Some(env);
        self
    }

    /// Resolve a path (tilde, file://, absolute/relative) against `cwd`. Pure.
    fn resolve_path(cwd: &Path, path: &str) -> PathBuf {
        let mut p = path.to_string();
        // Tilde expansion (literal only).
        if p == "~" {
            p = home_dir_string();
        } else if let Some(rest) = p.strip_prefix("~/") {
            p = format!("{}/{}", home_dir_string(), rest);
        } else if cfg!(windows) {
            if let Some(rest) = p.strip_prefix("~\\") {
                p = format!("{}\\{}", home_dir_string(), rest);
            }
        }
        // file:// URL → path. `absolute_path` rejects an invalid URL first;
        // other callers keep the literal.
        if p.starts_with("file://") {
            if let Some(local) = file_url_to_path(&p) {
                p = local;
            }
        }
        let pb = PathBuf::from(&p);
        if pb.is_absolute() {
            normalize_absolute(&pb)
        } else {
            normalize_absolute(&cwd.join(pb))
        }
    }

    /// Detect + return the shell config, or `ShellUnavailable`. Mirrors
    /// `getShellConfig`.
    async fn get_shell_config(&self) -> Result<ShellConfig, ExecutionError> {
        // Explicit override. Shell discovery happens on the hot path immediately
        // before spawning a command. Keep this probe synchronous: putting a
        // `tokio::fs` metadata future here can queue work on Tokio's blocking
        // pool and leave a tool run waiting before it has even created a child.
        if let Some(sp) = &self.shell_path {
            if sp.is_file() {
                return Ok(bash_shell_config(sp.clone()));
            }
            return Err(ExecutionError::new(
                ExecutionErrorCode::ShellUnavailable,
                format!("Custom shell path not found: {}", sp.display()),
            ));
        }
        // Windows probe.
        #[cfg(windows)]
        {
            if let Some(cfg) = windows_probe_bash().await {
                return Ok(cfg);
            }
            return Err(ExecutionError::new(
                ExecutionErrorCode::ShellUnavailable,
                "bash not found. Install Git for Windows / MSYS2 / Cygwin, or set shellPath.",
            ));
        }
        // POSIX.
        #[cfg(not(windows))]
        {
            if std::path::Path::new("/bin/bash").is_file() {
                return Ok(ShellConfig {
                    shell: PathBuf::from("/bin/bash"),
                    args: vec!["-c".to_string()],
                    transport: CommandTransport::Argv,
                });
            }
            // `which bash` fallback.
            if let Some(found) =
                run_command("which", &["bash"], std::time::Duration::from_millis(5000)).await
            {
                let trimmed = found.trim();
                if !trimmed.is_empty() && std::path::Path::new(trimmed).is_file() {
                    return Ok(ShellConfig {
                        shell: PathBuf::from(trimmed),
                        args: vec!["-c".to_string()],
                        transport: CommandTransport::Argv,
                    });
                }
            }
            // sh fallback.
            return Ok(ShellConfig {
                shell: PathBuf::from("sh"),
                args: vec!["-c".to_string()],
                transport: CommandTransport::Argv,
            });
        }
    }
}

impl Default for OsExecutionEnv {
    fn default() -> Self {
        Self::new()
    }
}

fn home_dir_string() -> String {
    dirs::home_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn normalize_absolute(p: &Path) -> PathBuf {
    // std::path doesn't expose ".."/"." collapse without canonicalize (which
    // hits the FS). Rebuild component-by-component.
    let mut out = PathBuf::new();
    for comp in p.components() {
        match comp {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// Incremental UTF-8 decoding for pipe reads. A multi-byte character split
/// across two reads is held back instead of becoming two U+FFFD, like a
/// streaming `TextDecoder`. Invalid bytes still decode lossily.
#[derive(Default)]
struct Utf8StreamDecoder {
    pending: Vec<u8>,
}

impl Utf8StreamDecoder {
    fn decode(&mut self, bytes: &[u8]) -> String {
        self.pending.extend_from_slice(bytes);
        let keep = incomplete_utf8_suffix(&self.pending);
        let split = self.pending.len() - keep;
        let text = String::from_utf8_lossy(&self.pending[..split]).into_owned();
        self.pending.drain(..split);
        text
    }

    /// Flush a dangling partial character at EOF.
    fn finish(&mut self) -> String {
        let text = String::from_utf8_lossy(&self.pending).into_owned();
        self.pending.clear();
        text
    }
}

/// Length of a trailing, possibly incomplete UTF-8 sequence (0..=3).
fn incomplete_utf8_suffix(bytes: &[u8]) -> usize {
    for back in 1..=bytes.len().min(3) {
        let byte = bytes[bytes.len() - back];
        if byte & 0xC0 == 0x80 {
            continue; // continuation byte; keep looking for the lead byte
        }
        let needed = if byte >= 0xF0 {
            4
        } else if byte >= 0xE0 {
            3
        } else if byte >= 0xC0 {
            2
        } else {
            1
        };
        return if needed > back { back } else { 0 };
    }
    0
}

/// `file://` URL → local path, following Node's `fileURLToPath`: the host must
/// be empty or `localhost`, percent-escapes are decoded, and an encoded `/`
/// or an invalid escape is rejected. `None` means the URL is invalid.
fn file_url_to_path(url: &str) -> Option<String> {
    let rest = url.strip_prefix("file://")?;
    let (host, path) = match rest.find('/') {
        Some(slash) => (&rest[..slash], &rest[slash..]),
        None => (rest, ""),
    };
    if !(host.is_empty() || host.eq_ignore_ascii_case("localhost")) || path.is_empty() {
        return None;
    }
    let path = path.split(['?', '#']).next().unwrap_or("");
    let decoded = percent_decode_path(path)?;
    // `/C:/...` is a Windows drive path.
    let bytes = decoded.as_bytes();
    if bytes.len() >= 3 && bytes[2] == b':' && bytes[1].is_ascii_alphabetic() {
        return Some(decoded[1..].to_string());
    }
    Some(decoded)
}

fn percent_decode_path(path: &str) -> Option<String> {
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            let byte = u8::from_str_radix(hex, 16).ok()?;
            if byte == b'/' || (cfg!(windows) && byte == b'\\') {
                return None;
            }
            out.push(byte);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn bash_shell_config(shell: PathBuf) -> ShellConfig {
    if is_legacy_wsl_bash_path(&shell) {
        ShellConfig {
            shell,
            args: vec!["-s".to_string()],
            transport: CommandTransport::Stdin,
        }
    } else {
        ShellConfig {
            shell,
            args: vec!["-c".to_string()],
            transport: CommandTransport::Argv,
        }
    }
}

fn is_legacy_wsl_bash_path(p: &Path) -> bool {
    let s = p.to_string_lossy().replace('/', "\\").to_lowercase();
    let re = regex::Regex::new(r"^[a-z]:\\windows\\(?:system32|sysnative)\\bash\.exe$").unwrap();
    re.is_match(&s)
}

#[cfg(windows)]
async fn windows_probe_bash() -> Option<ShellConfig> {
    let pf = std::env::var("ProgramFiles").ok();
    let pf86 = std::env::var("ProgramFiles(x86)").ok();
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(p) = pf {
        candidates.push(PathBuf::from(p).join("Git").join("bin").join("bash.exe"));
    }
    if let Some(p) = pf86 {
        candidates.push(PathBuf::from(p).join("Git").join("bin").join("bash.exe"));
    }
    for c in &candidates {
        if tokio::fs::metadata(c).await.is_ok() {
            return Some(bash_shell_config(c.clone()));
        }
    }
    // `where bash.exe`.
    if let Some(found) = run_command(
        "where",
        &["bash.exe"],
        std::time::Duration::from_millis(5000),
    )
    .await
    {
        let first = found.lines().next().unwrap_or("").trim().to_string();
        if !first.is_empty() && tokio::fs::metadata(&first).await.is_ok() {
            return Some(bash_shell_config(PathBuf::from(first)));
        }
    }
    None
}

/// Run a quick command, capture stdout, with a timeout. Mirrors `runCommand`
/// (used only by shell detection). Returns `None` on spawn failure / timeout /
/// empty stdout.
async fn run_command(cmd: &str, args: &[&str], timeout: std::time::Duration) -> Option<String> {
    let mut command = tokio::process::Command::new(cmd);
    command.args(args);
    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::null());
    command.stdin(std::process::Stdio::null());
    #[cfg(windows)]
    {
        // Hide the console window for the probe.
        use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let child = command.spawn().ok()?;
    let out = match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(o)) => o,
        _ => return None,
    };
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).into_owned();
    if s.trim().is_empty() {
        None
    } else {
        Some(s)
    }
}

/// Kill a process tree. Windows: `taskkill /F /T /PID`. POSIX: `kill -PG` with
/// fallback to `kill -PID`. Best-effort, errors swallowed.
async fn kill_process_tree(pid: u32) {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;
        let mut cmd = tokio::process::Command::new("taskkill");
        cmd.args(&["/F", "/T", "/PID", &pid.to_string()]);
        cmd.creation_flags(CREATE_NO_WINDOW);
        cmd.stdin(std::process::Stdio::null());
        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::null());
        // Wait for taskkill itself to finish. Spawning it fire-and-forget lets
        // the shell's `child.wait()` race the tree termination, and a listener
        // can keep its inherited stdout/stderr handles open long enough to
        // deadlock the capture join after Ctrl+C.
        let _ = cmd.status().await;
    }
    #[cfg(unix)]
    {
        use nix::sys::signal::{kill, Signal};
        use nix::unistd::Pid;
        // Process group: negative pid.
        let pg = Pid::from_raw(-(pid as i32));
        if kill(pg, Signal::SIGKILL).is_err() {
            let _ = kill(Pid::from_raw(pid as i32), Signal::SIGKILL);
        }
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = pid;
    }
}

#[async_trait]
impl FileSystem for OsExecutionEnv {
    fn cwd(&self) -> &Path {
        &self.cwd
    }

    fn native_path(&self, path: &str) -> Option<PathBuf> {
        Some(Self::resolve_path(&self.cwd, path))
    }

    async fn absolute_path(
        &self,
        path: &str,
        cancel: Option<&CancellationToken>,
    ) -> Result<PathBuf, FileError> {
        check_cancel_file(cancel, Some(path))?;
        if path.starts_with("file://") && file_url_to_path(path).is_none() {
            return Err(FileError::new(
                FileErrorCode::Invalid,
                format!("Invalid file URL: {path}"),
            )
            .with_path(path));
        }
        Ok(Self::resolve_path(&self.cwd, path))
    }

    async fn join_path(
        &self,
        parts: &[&str],
        cancel: Option<&CancellationToken>,
    ) -> Result<PathBuf, FileError> {
        check_cancel_file(cancel, None)?;
        let mut out = PathBuf::new();
        for p in parts {
            out.push(p);
        }
        Ok(out)
    }

    async fn read_text_file(
        &self,
        path: &str,
        cancel: Option<&CancellationToken>,
    ) -> Result<String, FileError> {
        check_cancel_file(cancel, Some(path))?;
        let abs = Self::resolve_path(&self.cwd, path);
        let bytes = read_cancellable(&abs, cancel).await?;
        String::from_utf8(bytes).map_err(|e| {
            FileError::new(FileErrorCode::Invalid, format!("invalid utf-8: {e}"))
                .with_path(abs.to_string_lossy())
        })
    }

    async fn read_text_lines(
        &self,
        path: &str,
        max_lines: Option<usize>,
        cancel: Option<&CancellationToken>,
    ) -> Result<Vec<String>, FileError> {
        check_cancel_file(cancel, Some(path))?;
        if max_lines.map(|m| m == 0).unwrap_or(false) {
            return Ok(Vec::new());
        }
        if let Some(max) = max_lines {
            // Bounded: never read past the requested lines (session discovery
            // asks for the header line of every saved session).
            let abs = Self::resolve_path(&self.cwd, path);
            return read_lines_bounded(&abs, max, cancel).await;
        }
        let text = self.read_text_file(path, cancel).await?;
        let mut lines: Vec<String> = text.split('\n').map(|s| s.to_string()).collect();
        if text.ends_with('\n') {
            if lines.last().map(|s| s.is_empty()).unwrap_or(false) {
                lines.pop();
            }
        }
        if let Some(m) = max_lines {
            lines.truncate(m);
        }
        Ok(lines)
    }

    async fn read_binary_file(
        &self,
        path: &str,
        cancel: Option<&CancellationToken>,
    ) -> Result<Vec<u8>, FileError> {
        check_cancel_file(cancel, Some(path))?;
        let abs = Self::resolve_path(&self.cwd, path);
        read_cancellable(&abs, cancel).await
    }

    async fn write_file(
        &self,
        path: &str,
        content: FileContent,
        cancel: Option<&CancellationToken>,
    ) -> Result<(), FileError> {
        check_cancel_file(cancel, Some(path))?;
        let abs = Self::resolve_path(&self.cwd, path);
        // mkdir parent (recursive).
        if let Some(parent) = abs.parent() {
            if !parent.as_os_str().is_empty() {
                if let Err(e) = tokio::fs::create_dir_all(parent).await {
                    return Err(io_to_file_error(e, Some(&abs.to_string_lossy())));
                }
            }
        }
        check_cancel_file(cancel, Some(&abs.to_string_lossy()))?;
        write_cancellable(&abs, content.as_bytes(), cancel).await
    }

    async fn append_file(
        &self,
        path: &str,
        content: FileContent,
        cancel: Option<&CancellationToken>,
    ) -> Result<(), FileError> {
        check_cancel_file(cancel, Some(path))?;
        let abs = Self::resolve_path(&self.cwd, path);
        if let Some(parent) = abs.parent() {
            if !parent.as_os_str().is_empty() {
                let _ = tokio::fs::create_dir_all(parent).await;
            }
        }
        use tokio::io::AsyncWriteExt;
        let res = match tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&abs)
            .await
        {
            Ok(mut f) => {
                // Tokio may acknowledge a buffered write before the blocking
                // filesystem operation completes. Await it before returning so
                // an immediate read/metadata call observes the appended bytes.
                match f.write_all(content.as_bytes()).await {
                    Ok(()) => f.flush().await,
                    Err(error) => Err(error),
                }
            }
            Err(e) => return Err(io_to_file_error(e, Some(&abs.to_string_lossy()))),
        };
        res.map_err(|e| io_to_file_error(e, Some(&abs.to_string_lossy())))
    }

    async fn rename_file(
        &self,
        source: &str,
        dest: &str,
        cancel: Option<&CancellationToken>,
    ) -> Result<(), FileError> {
        check_cancel_file(cancel, Some(source))?;
        let src = Self::resolve_path(&self.cwd, source);
        let dst = Self::resolve_path(&self.cwd, dest);
        tokio::fs::rename(&src, &dst)
            .await
            .map_err(|e| io_to_file_error(e, Some(&src.to_string_lossy())))
    }

    async fn file_info(
        &self,
        path: &str,
        cancel: Option<&CancellationToken>,
    ) -> Result<FileInfo, FileError> {
        check_cancel_file(cancel, Some(path))?;
        let abs = Self::resolve_path(&self.cwd, path);
        let meta = tokio::fs::symlink_metadata(&abs)
            .await
            .map_err(|e| io_to_file_error(e, Some(&abs.to_string_lossy())))?;
        let ft = meta.file_type();
        let kind = if ft.is_file() {
            FileKind::File
        } else if ft.is_dir() {
            FileKind::Directory
        } else if ft.is_symlink() {
            FileKind::Symlink
        } else {
            return Err(
                FileError::new(FileErrorCode::Invalid, "Unsupported file type")
                    .with_path(abs.to_string_lossy()),
            );
        };
        let mtime_ms = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        Ok(FileInfo {
            name: abs
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| abs.to_string_lossy().into_owned()),
            path: abs.clone(),
            kind,
            size: meta.len(),
            mtime_ms,
        })
    }

    async fn list_dir(
        &self,
        path: &str,
        cancel: Option<&CancellationToken>,
    ) -> Result<Vec<FileInfo>, FileError> {
        check_cancel_file(cancel, Some(path))?;
        let abs = Self::resolve_path(&self.cwd, path);
        let mut entries = tokio::fs::read_dir(&abs)
            .await
            .map_err(|e| io_to_file_error(e, Some(&abs.to_string_lossy())))?;
        let mut out = Vec::new();
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| io_to_file_error(e, Some(&abs.to_string_lossy())))?
        {
            if let Some(t) = cancel {
                if t.is_cancelled() {
                    return Err(FileError::aborted().with_path(abs.to_string_lossy()));
                }
            }
            let ep = entry.path();
            let meta = match tokio::fs::symlink_metadata(&ep).await {
                Ok(m) => m,
                // An entry that vanished or cannot be statted is skipped
                // rather than failing the whole listing, as ice's ls/fd/rg do.
                Err(_) => continue,
            };
            let ft = meta.file_type();
            let kind = if ft.is_file() {
                FileKind::File
            } else if ft.is_dir() {
                FileKind::Directory
            } else if ft.is_symlink() {
                FileKind::Symlink
            } else {
                continue; // silently skip sockets/fifos/etc.
            };
            let mtime_ms = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            out.push(FileInfo {
                name: entry.file_name().to_string_lossy().into_owned(),
                path: ep,
                kind,
                size: meta.len(),
                mtime_ms,
            });
        }
        Ok(out)
    }

    async fn canonical_path(
        &self,
        path: &str,
        cancel: Option<&CancellationToken>,
    ) -> Result<PathBuf, FileError> {
        check_cancel_file(cancel, Some(path))?;
        let abs = Self::resolve_path(&self.cwd, path);
        tokio::fs::canonicalize(&abs)
            .await
            .map_err(|e| io_to_file_error(e, Some(&abs.to_string_lossy())))
    }

    async fn read_link(
        &self,
        path: &str,
        cancel: Option<&CancellationToken>,
    ) -> Result<PathBuf, FileError> {
        check_cancel_file(cancel, Some(path))?;
        let abs = Self::resolve_path(&self.cwd, path);
        tokio::fs::read_link(&abs)
            .await
            .map_err(|e| io_to_file_error(e, Some(&abs.to_string_lossy())))
    }

    async fn exists(
        &self,
        path: &str,
        cancel: Option<&CancellationToken>,
    ) -> Result<bool, FileError> {
        check_cancel_file(cancel, Some(path))?;
        let abs = Self::resolve_path(&self.cwd, path);
        match tokio::fs::symlink_metadata(&abs).await {
            Ok(_) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(io_to_file_error(e, Some(&abs.to_string_lossy()))),
        }
    }

    async fn create_dir(
        &self,
        path: &str,
        recursive: bool,
        cancel: Option<&CancellationToken>,
    ) -> Result<(), FileError> {
        check_cancel_file(cancel, Some(path))?;
        let abs = Self::resolve_path(&self.cwd, path);
        let res = if recursive {
            tokio::fs::create_dir_all(&abs).await
        } else {
            tokio::fs::create_dir(&abs).await
        };
        res.map_err(|e| io_to_file_error(e, Some(&abs.to_string_lossy())))
    }

    async fn remove(
        &self,
        path: &str,
        recursive: bool,
        force: bool,
        cancel: Option<&CancellationToken>,
    ) -> Result<(), FileError> {
        check_cancel_file(cancel, Some(path))?;
        let abs = Self::resolve_path(&self.cwd, path);
        let res = if recursive {
            tokio::fs::remove_dir_all(&abs).await
        } else {
            // Try file first, then empty dir.
            match tokio::fs::remove_file(&abs).await {
                Ok(()) => Ok(()),
                Err(_) => tokio::fs::remove_dir(&abs).await,
            }
        };
        match res {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && force => Ok(()),
            Err(e) => Err(io_to_file_error(e, Some(&abs.to_string_lossy()))),
        }
    }

    async fn create_temp_dir(
        &self,
        prefix: Option<&str>,
        cancel: Option<&CancellationToken>,
    ) -> Result<PathBuf, FileError> {
        check_cancel_file(cancel, None)?;
        let prefix = prefix.unwrap_or("tmp-");
        // Use tempfile's NamedTempFile-less builder for a dir.
        let tmp =
            tempfile::tempdir_in(std::env::temp_dir()).map_err(|e| io_to_file_error(e, None))?;
        // tempfile picks a random name; rename to include the prefix.
        let path = tmp.path().to_path_buf();
        std::mem::forget(tmp); // keep the dir alive; caller manages cleanup.
                               // Build a prefixed sibling.
        let final_path = std::env::temp_dir().join(format!("{}{}", prefix, random_suffix()));
        // Rename the tempdir into the prefixed path.
        if let Err(e) = tokio::fs::rename(&path, &final_path).await {
            // Fallback: just return the original random path.
            let _ = e;
            return Ok(path);
        }
        Ok(final_path)
    }

    async fn create_temp_file(
        &self,
        prefix: &str,
        suffix: &str,
        cancel: Option<&CancellationToken>,
    ) -> Result<PathBuf, FileError> {
        check_cancel_file(cancel, None)?;
        // Create an isolated temp dir (matching the TS pattern of one dir per
        // file), then write an empty file named `<prefix><random><suffix>`.
        let dir = self.create_temp_dir(Some("tmp-"), cancel).await?;
        let name = format!("{}{}{}", prefix, random_suffix(), suffix);
        let path = dir.join(name);
        tokio::fs::File::create(&path)
            .await
            .map_err(|e| io_to_file_error(e, Some(&path.to_string_lossy())))?;
        Ok(path)
    }

    async fn cleanup(&self) {
        let pids: Vec<u32> = self.active_pids.lock().await.iter().copied().collect();
        for pid in pids {
            kill_process_tree(pid).await;
        }
        self.active_pids.lock().await.clear();
    }
}

/// Read a file to bytes, honoring cooperative cancellation.
async fn read_cancellable(
    path: &Path,
    cancel: Option<&CancellationToken>,
) -> Result<Vec<u8>, FileError> {
    let read = tokio::fs::read(path);
    let res = match cancel {
        Some(t) => tokio::select! {
            r = read => r,
            _ = t.cancelled() => return Err(FileError::aborted().with_path(path.to_string_lossy())),
        },
        None => read.await,
    };
    res.map_err(|e| io_to_file_error(e, Some(&path.to_string_lossy())))
}

/// Read at most `max` newline-terminated lines without loading the whole file.
/// Longest single line `read_lines_bounded` will buffer.
const MAX_BOUNDED_LINE_BYTES: u64 = 1024 * 1024;

async fn read_lines_bounded(
    path: &Path,
    max: usize,
    cancel: Option<&CancellationToken>,
) -> Result<Vec<String>, FileError> {
    use tokio::io::{AsyncBufReadExt, AsyncReadExt};
    let display = path.to_string_lossy();
    let read = async {
        let file = tokio::fs::File::open(path).await?;
        let mut reader = tokio::io::BufReader::new(file);
        let mut lines = Vec::new();
        while lines.len() < max {
            let mut buf = Vec::new();
            // Cap each line so one huge or newline-free line cannot exhaust memory.
            let read = (&mut reader)
                .take(MAX_BOUNDED_LINE_BYTES + 1)
                .read_until(b'\n', &mut buf)
                .await?;
            if read == 0 {
                break;
            }
            if buf.len() as u64 > MAX_BOUNDED_LINE_BYTES {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "line exceeds the bounded read limit",
                ));
            }
            if buf.last() == Some(&b'\n') {
                buf.pop();
            }
            lines.push(buf);
        }
        Ok::<_, std::io::Error>(lines)
    };
    let res = match cancel {
        Some(t) => tokio::select! {
            r = read => r,
            _ = t.cancelled() => return Err(FileError::aborted().with_path(display.as_ref())),
        },
        None => read.await,
    };
    res.map_err(|e| io_to_file_error(e, Some(&display)))?
        .into_iter()
        .map(|bytes| {
            String::from_utf8(bytes).map_err(|e| {
                FileError::new(FileErrorCode::Invalid, format!("invalid utf-8: {e}"))
                    .with_path(display.as_ref())
            })
        })
        .collect()
}

/// Write bytes to a file (truncate), honoring cooperative cancellation.
async fn write_cancellable(
    path: &Path,
    bytes: &[u8],
    cancel: Option<&CancellationToken>,
) -> Result<(), FileError> {
    use tokio::io::AsyncWriteExt;
    let f = tokio::fs::File::create(path)
        .await
        .map_err(|e| io_to_file_error(e, Some(&path.to_string_lossy())))?;
    let write = async {
        let mut f = f;
        f.write_all(bytes).await?;
        f.flush().await?;
        Ok::<(), std::io::Error>(())
    };
    let res = match cancel {
        Some(t) => tokio::select! {
            r = write => r,
            _ = t.cancelled() => return Err(FileError::aborted().with_path(path.to_string_lossy())),
        },
        None => write.await,
    };
    res.map_err(|e| io_to_file_error(e, Some(&path.to_string_lossy())))
}

fn random_suffix() -> String {
    // Short random suffix via uuid v4 (no Time—it's a UUID, not a timestamp).
    uuid::Uuid::new_v4().simple().to_string()
}

#[async_trait]
impl Shell for OsExecutionEnv {
    async fn exec<'a>(
        &'a self,
        command: &str,
        options: ShellExecOptions<'a>,
    ) -> Result<ShellOutput, ExecutionError> {
        // Pre-flight abort.
        check_cancel_exec(options.cancel)?;
        // Resolve timeout.
        let timeout = match resolve_timeout(options.timeout) {
            Ok(t) => t,
            Err(e) => return Err(e),
        };
        // Resolve cwd.
        let cwd = match &options.cwd {
            Some(c) => Self::resolve_path(&self.cwd, &c.to_string_lossy()),
            None => self.cwd.clone(),
        };
        // Shell config. An explicit override is used by native shell tools
        // such as PowerShell; otherwise preserve the default Bash behavior.
        let shell_cfg = match &options.shell {
            Some(override_shell) => ShellConfig {
                shell: override_shell.program.clone(),
                args: override_shell.args.clone(),
                transport: CommandTransport::Argv,
            },
            None => self.get_shell_config().await?,
        };
        if options.stdin.is_some() && matches!(shell_cfg.transport, CommandTransport::Stdin) {
            return Err(ExecutionError::new(
                ExecutionErrorCode::SpawnError,
                STDIN_TRANSPORT_CONFLICT,
            ));
        }
        // Verify cwd exists.
        if !cwd.exists() {
            return Err(ExecutionError::new(
                ExecutionErrorCode::SpawnError,
                format!(
                    "Working directory does not exist: {}\nCannot execute bash commands.",
                    cwd.display()
                ),
            ));
        }

        // Build the spawn.
        let mut cmd = tokio::process::Command::new(&shell_cfg.shell);
        cmd.current_dir(&cwd);
        cmd.env_clear();
        // Environment: inherit + shell_env + per-call. The host's own private
        // plumbing (`HOST_PRIVATE_ENV`) is skipped: it describes *this* process,
        // not the shell or anything the shell runs, and inheriting it makes a
        // nested `icers` misreport its session. A caller that genuinely wants a
        // value passed down can still set it explicitly through `env`/`shell_env`
        // below (those are applied after this loop and win).
        if options.inherit_env {
            for (k, v) in std::env::vars() {
                if is_host_private_env(&k) {
                    continue;
                }
                cmd.env(k, v);
            }
        }
        if let Some(base) = &self.shell_env {
            for (k, v) in base {
                cmd.env(k, v);
            }
        }
        if let Some(extra) = &options.env {
            for (k, v) in extra {
                cmd.env(k, v);
            }
        }
        // stdio.
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        #[cfg(unix)]
        {
            cmd.process_group(0); // child becomes its own group leader.
        }
        #[cfg(windows)]
        {
            use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        // Args: stdin transport → `bash -s` (no command argv); argv → append.
        let transport = shell_cfg.transport;
        cmd.args(&shell_cfg.args);
        if matches!(transport, CommandTransport::Argv) {
            cmd.arg(command);
        }

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                return Err(ExecutionError::new(
                    ExecutionErrorCode::SpawnError,
                    format!("failed to spawn shell: {e}"),
                ));
            }
        };
        let pid = child.id();
        if let Some(pid) = pid {
            self.active_pids.lock().await.insert(pid);
            if let Some(on_spawn) = options.on_spawn {
                on_spawn(pid);
            }
        }

        // Stdin transport: write command then close. Closing happens on BOTH
        // transports, and deliberately: anything that prompts (`npx` asking to
        // install, `npm init`, a `read` in a script) must see EOF and fail fast,
        // not block on a pipe that nothing will ever write to or close. tokio's
        // `Child::wait` also drops stdin as a deadlock guard, but relying on that
        // would make the guarantee a side effect of an implementation detail.
        // The one exception is an explicit interactive stdin (bash_input),
        // which keeps the pipe open and feeds it from `options.stdin`.
        let mut stdin = child.stdin.take();
        if matches!(transport, CommandTransport::Stdin) {
            if let Some(stdin) = stdin.as_mut() {
                use tokio::io::AsyncWriteExt;
                let _ = stdin.write_all(command.as_bytes()).await;
                let _ = stdin.flush().await;
            }
        }
        let mut stdin_requests = options.stdin;
        // Without interactive stdin the pipe is dropped here (EOF).
        let mut interactive_stdin = if stdin_requests.is_some() {
            stdin
        } else {
            drop(stdin);
            None
        };
        let stdin_task = async {
            use tokio::io::AsyncWriteExt;
            let Some(requests) = stdin_requests.as_mut() else {
                return;
            };
            while let Some(request) = requests.recv().await {
                match request {
                    StdinRequest::Write { bytes, done } => {
                        let result = match interactive_stdin.as_mut() {
                            Some(pipe) => match pipe.write_all(&bytes).await {
                                Ok(()) => pipe.flush().await.map_err(|e| e.to_string()),
                                Err(e) => Err(e.to_string()),
                            },
                            None => Err("stdin is closed".to_string()),
                        };
                        if result.is_err() {
                            interactive_stdin = None;
                        }
                        let _ = done.send(result);
                    }
                    StdinRequest::Close => interactive_stdin = None,
                }
            }
            // The sender is gone: close the pipe so the child sees EOF.
            interactive_stdin = None;
        };

        // Collect stdout/stderr via separate read tasks, firing callbacks.
        let mut stdout_buf = String::new();
        let mut stderr_buf = String::new();
        let cancel = options.cancel;
        let mut on_stdout = options.on_stdout;
        let mut on_stderr = options.on_stderr;

        let mut stdout = child.stdout.take();
        let mut stderr = child.stderr.take();

        let stdout_task = async {
            if let Some(s) = stdout.as_mut() {
                let mut buf = vec![0u8; 8192];
                let mut decoder = Utf8StreamDecoder::default();
                use tokio::io::AsyncReadExt;
                loop {
                    let read = s.read(&mut buf);
                    let read_result = match cancel {
                        Some(token) => tokio::select! {
                            result = read => result,
                            _ = token.cancelled() => break,
                        },
                        None => read.await,
                    };
                    let n = match read_result {
                        Ok(0) => break,
                        Ok(n) => n,
                        Err(_) => break,
                    };
                    let chunk = decoder.decode(&buf[..n]);
                    if chunk.is_empty() {
                        continue;
                    }
                    if let Some(cb) = on_stdout.as_mut() {
                        // Callback errors are swallowed (TS sets callbackError;
                        // v1 simplifies — the callback contract is best-effort).
                        cb(&chunk);
                    } else {
                        stdout_buf.push_str(&chunk);
                    }
                }
                let tail = decoder.finish();
                if !tail.is_empty() {
                    match on_stdout.as_mut() {
                        Some(cb) => cb(&tail),
                        None => stdout_buf.push_str(&tail),
                    }
                }
            }
        };
        let stderr_task = async {
            if let Some(s) = stderr.as_mut() {
                let mut buf = vec![0u8; 8192];
                let mut decoder = Utf8StreamDecoder::default();
                use tokio::io::AsyncReadExt;
                loop {
                    let read = s.read(&mut buf);
                    let read_result = match cancel {
                        Some(token) => tokio::select! {
                            result = read => result,
                            _ = token.cancelled() => break,
                        },
                        None => read.await,
                    };
                    let n = match read_result {
                        Ok(0) => break,
                        Ok(n) => n,
                        Err(_) => break,
                    };
                    let chunk = decoder.decode(&buf[..n]);
                    if chunk.is_empty() {
                        continue;
                    }
                    if let Some(cb) = on_stderr.as_mut() {
                        cb(&chunk);
                    } else {
                        stderr_buf.push_str(&chunk);
                    }
                }
                let tail = decoder.finish();
                if !tail.is_empty() {
                    match on_stderr.as_mut() {
                        Some(cb) => cb(&tail),
                        None => stderr_buf.push_str(&tail),
                    }
                }
            }
        };

        // Wait for the child, with timeout + cancellation.
        let mut timed_out = false;
        let exit_status = {
            // We pin stdout/stderr take handles + the wait future, then drive
            // them concurrently inside a single select! block. `child` is only
            // touched again AFTER the join completes (kill + reap), so the
            // borrow ends cleanly.
            let status_fut = async {
                match (timeout, cancel) {
                    (Some(to), Some(token)) => {
                        tokio::select! {
                            status = child.wait() => status,
                            _ = tokio::time::sleep(to) => {
                                timed_out = true;
                                if let Some(pid) = child.id() {
                                    kill_process_tree(pid).await;
                                }
                                child.wait().await
                            }
                            _ = token.cancelled() => {
                                if let Some(pid) = child.id() {
                                    kill_process_tree(pid).await;
                                }
                                child.wait().await
                            }
                        }
                    }
                    (Some(to), None) => match tokio::time::timeout(to, child.wait()).await {
                        Ok(status) => status,
                        Err(_) => {
                            timed_out = true;
                            if let Some(pid) = child.id() {
                                kill_process_tree(pid).await;
                            }
                            child.wait().await
                        }
                    },
                    (None, Some(token)) => {
                        tokio::select! {
                            status = child.wait() => status,
                            _ = token.cancelled() => {
                                if let Some(pid) = child.id() {
                                    kill_process_tree(pid).await;
                                }
                                child.wait().await
                            }
                        }
                    }
                    (None, None) => child.wait().await,
                }
            };
            // Run stdio + status concurrently — but the CHILD is the driver:
            // once it is reaped the command is over, and the call has to end even
            // if something else still holds the stdout/stderr pipes open.
            //
            // This is the "`npm run` gets stuck" shape. A script that starts a
            // watcher, a dev-server or a background daemon leaves a grandchild
            // running with the inherited pipes, so reading to EOF waits for a
            // process the command no longer owns. No timeout rescues that: the
            // timeout wraps `child.wait()` and the child already exited
            // successfully.
            //
            // The readers must keep running while we wait, though: a command
            // that writes more than the OS pipe buffer (~64 KiB) blocks on its
            // write until someone reads, so awaiting `child.wait()` alone
            // deadlocks it until the timeout.
            tokio::pin!(status_fut, stdout_task, stderr_task, stdin_task);
            let mut stdout_done = false;
            let mut stderr_done = false;
            let mut stdin_done = false;
            let status_res = loop {
                tokio::select! {
                    status = &mut status_fut => break status,
                    _ = &mut stdout_task, if !stdout_done => stdout_done = true,
                    _ = &mut stderr_task, if !stderr_done => stderr_done = true,
                    _ = &mut stdin_task, if !stdin_done => stdin_done = true,
                }
            };
            // Drain what is already in the pipe — a completed write is already in
            // the pipe buffer, so this is not a race — then stop, without waiting
            // for an EOF that may never come. A command whose own process is gone
            // should not be able to hold the call open.
            let _ = tokio::time::timeout(
                std::time::Duration::from_millis(EXIT_STDIO_GRACE_MS),
                async {
                    // A finished task must not be polled again.
                    let out = async {
                        if !stdout_done {
                            (&mut stdout_task).await;
                        }
                    };
                    let err = async {
                        if !stderr_done {
                            (&mut stderr_task).await;
                        }
                    };
                    tokio::join!(out, err);
                },
            )
            .await;
            status_res
        };

        // Give trailing stdio a brief grace period. Only the timed-out and
        // cancelled paths still owe a wait: on the normal path the drain above
        // already capped it.
        if timed_out {
            let _ = tokio::time::sleep(std::time::Duration::from_millis(EXIT_STDIO_GRACE_MS)).await;
        }

        // Remove from tracked pids.
        if let Some(pid) = pid {
            self.active_pids.lock().await.remove(&pid);
        }

        // Resolution priority: timedOut → cancelled → status.
        let status = match exit_status {
            Ok(s) => s,
            Err(e) => {
                return Err(ExecutionError::new(
                    ExecutionErrorCode::SpawnError,
                    format!("child wait failed: {e}"),
                ));
            }
        };
        if timed_out {
            return Err(ExecutionError::timeout(
                options.timeout.map(|f| f as u64).unwrap_or(0),
            ));
        }
        if cancel.map(|t| t.is_cancelled()).unwrap_or(false) {
            return Err(ExecutionError::aborted());
        }
        let exit_code = status.code().unwrap_or(0);
        Ok(ShellOutput {
            stdout: stdout_buf,
            stderr: stderr_buf,
            exit_code,
        })
    }

    async fn cleanup(&self) {
        FileSystem::cleanup(self).await
    }
}

#[async_trait]
impl ExecutionEnv for OsExecutionEnv {}

#[async_trait]
impl MutatingEnv for OsExecutionEnv {
    fn as_env(&self) -> &dyn ExecutionEnv {
        self
    }
    fn mutation_registry(&self) -> &MutationQueueRegistry {
        &self.registry
    }
}

/// Validate a timeout (seconds). Mirrors `resolveTimeoutMs` semantics.
fn resolve_timeout(timeout: Option<f64>) -> Result<Option<std::time::Duration>, ExecutionError> {
    match timeout {
        None => Ok(None),
        Some(secs) => {
            if !secs.is_finite() || secs <= 0.0 {
                return Err(ExecutionError::new(
                    ExecutionErrorCode::Timeout,
                    "Invalid timeout: must be a finite number of seconds",
                ));
            }
            let ms = secs * 1000.0;
            if ms > MAX_TIMEOUT_MS as f64 {
                return Err(ExecutionError::new(
                    ExecutionErrorCode::Timeout,
                    format!(
                        "Invalid timeout: maximum is {} seconds",
                        MAX_TIMEOUT_SECONDS
                    ),
                ));
            }
            Ok(Some(std::time::Duration::from_secs_f64(secs)))
        }
    }
}

#[cfg(test)]
mod file_url_tests {
    use super::*;

    #[test]
    fn file_urls_follow_node_file_url_to_path() {
        assert_eq!(
            file_url_to_path("file:///a/b%20c.txt").as_deref(),
            Some("/a/b c.txt")
        );
        assert_eq!(
            file_url_to_path("file://localhost/x").as_deref(),
            Some("/x")
        );
        assert_eq!(
            file_url_to_path("file:///C:/dir/f").as_deref(),
            Some("C:/dir/f")
        );
        assert_eq!(
            file_url_to_path("file:///q?x=1#frag").as_deref(),
            Some("/q")
        );
        // Remote hosts, encoded slashes and bad escapes are invalid.
        assert_eq!(file_url_to_path("file://server/share"), None);
        assert_eq!(file_url_to_path("file:///a%2Fb"), None);
        assert_eq!(file_url_to_path("file:///a%zz"), None);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn absolute_path_rejects_an_invalid_file_url() {
        let env = OsExecutionEnv::with_cwd(std::env::temp_dir());
        let err = env
            .absolute_path("file://server/x", None)
            .await
            .unwrap_err();
        assert_eq!(err.code, FileErrorCode::Invalid);
        let ok = env.absolute_path("file:///tmp/a%20b", None).await.unwrap();
        assert_eq!(ok, PathBuf::from("/tmp/a b"));
    }
}
