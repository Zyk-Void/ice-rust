//! Mirrors `packages/coding-agent/src/core/tools/bash-background.ts` — the
//! session-owned registry of background shells.
//!
//! A shell enters the registry either at launch (`bash run_in_background`)
//! or when a running foreground call is released (by the user, a queued
//! steering message, or the auto-background timer). The registry then owns
//! the process: its cancellation, its output (a bounded tail plus the full
//! log), its stdin when requested, and its single terminal transition. The
//! registry is in memory only; logs are its only persistent payload and are
//! removed on disposal.
//!
//! Counts that ice measures in UTF-16 code units (output retention, read
//! cursors) are measured in Unicode scalar values here, so no limit can split
//! a character.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tokio::sync::watch;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::env::{ExecutionEnv, StdinRequest};
use crate::shell_output::{LiveShell, LogFileIssue, OutputCapture};

/// Running shells allowed at explicit launch. Mirrors `MAX_RUNNING_SHELLS`.
pub const MAX_RUNNING_SHELLS: usize = 16;
/// Finished shells retained for inspection. Mirrors `MAX_FINISHED_SHELLS`.
pub const MAX_FINISHED_SHELLS: usize = 20;
/// Retained output per shell (characters). Mirrors `MAX_OUTPUT_CHARS`.
pub const MAX_OUTPUT_CHARS: usize = 256 * 1024;
/// Stdin bytes per `bash_input` call. Mirrors `MAX_INPUT_BYTES_PER_CALL`.
pub const MAX_INPUT_BYTES_PER_CALL: usize = 8192;
/// Stdin bytes per shell lifetime. Mirrors `MAX_INPUT_BYTES_PER_SHELL`.
pub const MAX_INPUT_BYTES_PER_SHELL: usize = 65536;
/// Custom message type for completion delivery.
pub const BASH_BACKGROUND_COMPLETED_CUSTOM_TYPE: &str = "bash_background_completed";

/// How long a stdin write may wait for the pipe to drain.
const STDIN_DRAIN_WAIT: Duration = Duration::from_secs(5);
/// Queued-steer polling interval during waits. Mirrors `USER_MESSAGE_POLL_MS`.
pub const USER_MESSAGE_POLL: Duration = Duration::from_millis(250);

/// Lifecycle state. Mirrors `BackgroundShellStatus`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BackgroundShellStatus {
    Running,
    Completed,
    Failed,
    Killed,
}

impl BackgroundShellStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Killed => "killed",
        }
    }
}

/// Who put the shell in the background.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ShellOrigin {
    Model,
    User,
}

/// A read-only view of one shell. Mirrors `BackgroundShellSnapshot`.
#[derive(Debug, Clone, Serialize)]
pub struct BackgroundShellSnapshot {
    pub task_id: String,
    pub command: String,
    pub origin: ShellOrigin,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub status: BackgroundShellStatus,
    pub started_at_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at_ms: Option<i64>,
    pub last_output_at_ms: i64,
    pub output: String,
    pub total_chars: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub final_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_issue: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub stdin_open: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub lingering: bool,
    /// Wall-clock runtime so far (or until it ended).
    #[serde(skip)]
    pub runtime: Duration,
    /// Time since the last output (or since start, before any output).
    #[serde(skip)]
    pub since_output: Duration,
}

/// Delivered once when a shell finishes. Mirrors `BashBackgroundCompletion`.
#[derive(Debug, Clone, Serialize)]
pub struct BashBackgroundCompletion {
    pub task_id: String,
    pub command: String,
    pub text: String,
    pub is_error: bool,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_issue: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub lingering: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub stopped_by_user: bool,
}

/// The normalized end of an execution. Mirrors `settledOutcome`.
#[derive(Debug, Clone)]
pub struct SettledOutcome {
    pub text: String,
    pub is_error: bool,
    pub exit_code: Option<i32>,
}

/// Formats an execution result into its settled outcome.
pub type SettleFormatter = Box<
    dyn FnOnce(
            Result<crate::shell_output::ShellCaptureResult, crate::error::ExecutionError>,
        ) -> SettledOutcome
        + Send,
>;

/// A shell handed to the registry.
pub struct Adoption {
    pub live: LiveShell,
    pub capture: OutputCapture,
    /// The original command (without any host prefix).
    pub command: String,
    pub origin: ShellOrigin,
    pub description: Option<String>,
    /// When the command started (foreground releases keep their start).
    pub started: Instant,
    pub started_at_ms: i64,
    /// Output the caller already returned to the model; it is retained but
    /// not repeated by the next incremental read.
    pub initial_output: String,
    pub env: Arc<dyn ExecutionEnv>,
    pub settle: SettleFormatter,
}

/// Result of [`BashBackgroundRegistry::read`]. Mirrors `BackgroundShellRead`.
#[derive(Debug, Clone)]
pub struct BackgroundShellRead {
    pub shell: BackgroundShellSnapshot,
    pub new_output: String,
    pub omitted_chars: usize,
}

/// Result of [`BashBackgroundRegistry::read_log`].
#[derive(Debug, Clone)]
pub struct BackgroundLogRead {
    pub shell: BackgroundShellSnapshot,
    pub lines: Vec<String>,
    pub matched: Option<usize>,
    pub skipped_bytes: u64,
}

/// What a wait on a shell ends on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitUntil {
    Exit,
    Output,
}

/// Outcome of [`BashBackgroundRegistry::kill`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KillOutcome {
    Stopped,
    StoppedLingering,
    AlreadyFinished(BackgroundShellStatus),
}

/// Outcome of [`BashBackgroundRegistry::write_input`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputWrite {
    pub bytes: usize,
    pub pending: bool,
}

/// Why a foreground call is moving to the background.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReleaseReason {
    /// The user asked (ctrl+b).
    User,
    /// A steering message is waiting.
    Message,
    /// The auto-background threshold elapsed.
    Timer,
}

/// Steering probe supplied by the session: true when a steering message is
/// queued (follow-ups do not count).
pub type SteerProbe = Arc<dyn Fn() -> bool + Send + Sync>;

type StartListener = Arc<dyn Fn(&str) + Send + Sync>;
type OutputListener = Arc<dyn Fn(&str, &str) + Send + Sync>;
type ChangeListener = Arc<dyn Fn() + Send + Sync>;
type CompleteListener = Arc<dyn Fn(&BashBackgroundCompletion) + Send + Sync>;

#[derive(Clone)]
enum Listener {
    Start(StartListener),
    Output(OutputListener),
    Change(ChangeListener),
    Complete(CompleteListener),
}

struct ShellRecord {
    id: String,
    command: String,
    origin: ShellOrigin,
    description: Option<String>,
    status: BackgroundShellStatus,
    started: Instant,
    started_at_ms: i64,
    ended: Option<Instant>,
    ended_at_ms: Option<i64>,
    last_output: Instant,
    last_output_at_ms: i64,
    chunks: VecDeque<String>,
    retained_chars: usize,
    total_chars: usize,
    read_cursor: usize,
    final_text: Option<String>,
    log_path: Option<PathBuf>,
    log_issue: Option<LogFileIssue>,
    abort: CancellationToken,
    /// Bumped on every output or state change; waiters watch it.
    tick: watch::Sender<u64>,
    /// Completes after the capture task has flushed its final log bytes.
    ingest_done: Option<tokio::sync::oneshot::Receiver<()>>,
    stdin: Option<tokio::sync::mpsc::UnboundedSender<StdinRequest>>,
    input_bytes: usize,
    pid: Arc<std::sync::Mutex<Option<u32>>>,
    env: Arc<dyn ExecutionEnv>,
}

impl ShellRecord {
    fn running(&self) -> bool {
        self.status == BackgroundShellStatus::Running
    }

    fn pgid(&self) -> Option<u32> {
        *self.pid.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn lingering(&self) -> bool {
        !self.running() && self.pgid().is_some_and(process_group_alive)
    }

    fn bump(&self) {
        self.tick.send_modify(|t| *t += 1);
    }

    fn retained_text(&self) -> String {
        self.chunks.iter().map(String::as_str).collect()
    }

    fn snapshot(&self) -> BackgroundShellSnapshot {
        let end = self.ended.unwrap_or_else(Instant::now);
        BackgroundShellSnapshot {
            task_id: self.id.clone(),
            command: self.command.clone(),
            origin: self.origin,
            description: self.description.clone(),
            status: self.status,
            started_at_ms: self.started_at_ms,
            ended_at_ms: self.ended_at_ms,
            last_output_at_ms: self.last_output_at_ms,
            output: self.retained_text(),
            total_chars: self.total_chars,
            final_text: self.final_text.clone(),
            log_path: self
                .log_path
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
            log_issue: self.log_issue.as_ref().map(LogFileIssue::describe),
            stdin_open: self.running() && self.stdin.is_some(),
            lingering: self.lingering(),
            runtime: end.saturating_duration_since(self.started),
            since_output: Instant::now().saturating_duration_since(self.last_output),
        }
    }
}

struct Foreground {
    release: Box<dyn FnOnce(ReleaseReason) + Send>,
}

#[derive(Default)]
struct Inner {
    shells: Vec<ShellRecord>,
    next_id: u64,
    next_listener: u64,
    listeners: Vec<(u64, Listener)>,
    foreground: HashMap<String, (u64, Foreground)>,
    next_foreground: u64,
    tracked_logs: Vec<(PathBuf, Arc<dyn ExecutionEnv>)>,
    disposed: bool,
}

/// The per-session background shell registry. Mirrors `BashBackgroundRegistry`.
#[derive(Default)]
pub struct BashBackgroundRegistry {
    inner: Mutex<Inner>,
}

/// Unsubscribes its listener when dropped... only if kept; call
/// [`BashBackgroundRegistry::unsubscribe`] for explicit removal.
pub type ListenerId = u64;

impl BashBackgroundRegistry {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn listeners(&self) -> Vec<Listener> {
        self.lock()
            .listeners
            .iter()
            .map(|(_, l)| l.clone())
            .collect()
    }

    fn subscribe(&self, listener: Listener) -> ListenerId {
        let mut inner = self.lock();
        inner.next_listener += 1;
        let id = inner.next_listener;
        inner.listeners.push((id, listener));
        id
    }

    pub fn on_start(&self, f: impl Fn(&str) + Send + Sync + 'static) -> ListenerId {
        self.subscribe(Listener::Start(Arc::new(f)))
    }
    pub fn on_output(&self, f: impl Fn(&str, &str) + Send + Sync + 'static) -> ListenerId {
        self.subscribe(Listener::Output(Arc::new(f)))
    }
    pub fn on_change(&self, f: impl Fn() + Send + Sync + 'static) -> ListenerId {
        self.subscribe(Listener::Change(Arc::new(f)))
    }
    pub fn on_complete(
        &self,
        f: impl Fn(&BashBackgroundCompletion) + Send + Sync + 'static,
    ) -> ListenerId {
        self.subscribe(Listener::Complete(Arc::new(f)))
    }
    pub fn unsubscribe(&self, id: ListenerId) {
        self.lock().listeners.retain(|(lid, _)| *lid != id);
    }

    /// Listener failures are isolated: one panicking sink cannot stop the
    /// lifecycle (ice leaves change-listener errors uncaught; icers catches
    /// all of them).
    fn emit(&self, f: impl Fn(&Listener)) {
        for listener in self.listeners() {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(&listener)));
        }
    }

    fn emit_change(&self) {
        self.emit(|l| {
            if let Listener::Change(f) = l {
                f()
            }
        });
    }

    pub fn running_count(&self) -> usize {
        self.lock().shells.iter().filter(|s| s.running()).count()
    }

    /// Take ownership of a running shell and return its id (`bash-N`). If
    /// disposal already started, cancel the shell and remove its log after it
    /// exits instead of adding an un-settleable record.
    pub fn adopt(self: &Arc<Self>, adoption: Adoption) -> Result<String, String> {
        let Adoption {
            live,
            capture,
            command,
            origin,
            description,
            started,
            started_at_ms,
            initial_output,
            env,
            settle,
        } = adoption;
        let LiveShell {
            chunks,
            done,
            cancel,
            pid,
            stdin,
        } = live;
        let (tick, _) = watch::channel(0u64);
        let (ingest_done_tx, ingest_done_rx) = tokio::sync::oneshot::channel();
        let id = {
            let mut inner = self.lock();
            if inner.disposed {
                None
            } else {
                inner.next_id += 1;
                let id = format!("bash-{}", inner.next_id);
                let mut record = ShellRecord {
                    id: id.clone(),
                    command,
                    origin,
                    description: description.filter(|d| !d.is_empty()),
                    status: BackgroundShellStatus::Running,
                    started,
                    started_at_ms,
                    ended: None,
                    ended_at_ms: None,
                    last_output: started,
                    last_output_at_ms: started_at_ms,
                    chunks: VecDeque::new(),
                    retained_chars: 0,
                    total_chars: 0,
                    read_cursor: 0,
                    final_text: None,
                    log_path: capture.log_path(),
                    log_issue: capture.log_issue(),
                    abort: cancel.clone(),
                    tick,
                    ingest_done: Some(ingest_done_rx),
                    stdin,
                    input_bytes: 0,
                    pid,
                    env: env.clone(),
                };
                append_chunk(&mut record, &initial_output);
                record.read_cursor = record.total_chars;
                inner.shells.push(record);
                Some(id)
            }
        };
        let Some(id) = id else {
            cancel.cancel();
            let log_path = capture.log_path();
            tokio::spawn(async move {
                drop(chunks);
                let _ = done.await;
                if let Some(path) = log_path {
                    remove_log(env, path).await;
                }
            });
            return Err("The background shell registry is shutting down.".into());
        };
        self.emit(|l| {
            if let Listener::Start(f) = l {
                f(&id)
            }
        });
        self.emit_change();
        let registry = Arc::clone(self);
        let task_id = id.clone();
        tokio::spawn(async move {
            registry
                .ingest(task_id, chunks, done, cancel, capture, settle)
                .await;
            let _ = ingest_done_tx.send(());
        });
        Ok(id)
    }

    async fn ingest(
        self: Arc<Self>,
        id: String,
        mut chunks: tokio::sync::mpsc::UnboundedReceiver<String>,
        mut done: tokio::sync::oneshot::Receiver<
            Result<crate::env::ShellOutput, crate::error::ExecutionError>,
        >,
        cancel: CancellationToken,
        mut capture: OutputCapture,
        settle: SettleFormatter,
    ) {
        let exec = loop {
            tokio::select! {
                biased;
                Some(raw) = chunks.recv() => {
                    let text = capture.push(&raw).await;
                    self.append_output(&id, &text, capture.log_issue());
                }
                result = &mut done => break result.unwrap_or_else(|_| {
                    Err(crate::error::ExecutionError::aborted())
                }),
            }
        };
        while let Ok(raw) = chunks.try_recv() {
            let text = capture.push(&raw).await;
            self.append_output(&id, &text, capture.log_issue());
        }
        let outcome = settle(capture.finish(exec, cancel.is_cancelled()));
        self.settle(&id, outcome, capture.log_issue());
    }

    fn append_output(&self, id: &str, text: &str, log_issue: Option<LogFileIssue>) {
        if text.is_empty() {
            return;
        }
        {
            let mut inner = self.lock();
            let Some(record) = inner.shells.iter_mut().find(|s| s.id == id) else {
                return;
            };
            if !record.running() {
                return;
            }
            append_chunk(record, text);
            if log_issue.is_some() {
                record.log_issue = log_issue;
            }
            record.bump();
        }
        self.emit(|l| {
            if let Listener::Output(f) = l {
                f(id, text)
            }
        });
        self.emit_change();
    }

    /// Natural settlement: ignored once the shell is no longer running (a
    /// kill already recorded its single terminal transition).
    fn settle(&self, id: &str, outcome: SettledOutcome, log_issue: Option<LogFileIssue>) {
        let completion = {
            let mut inner = self.lock();
            if inner.disposed {
                return;
            }
            let Some(record) = inner.shells.iter_mut().find(|s| s.id == id) else {
                return;
            };
            if !record.running() {
                return;
            }
            record.status = if outcome.is_error {
                BackgroundShellStatus::Failed
            } else {
                BackgroundShellStatus::Completed
            };
            record.ended = Some(Instant::now());
            record.ended_at_ms = Some(now_ms());
            record.final_text = Some(outcome.text.clone());
            if log_issue.is_some() {
                record.log_issue = log_issue;
            }
            record.stdin = None;
            record.bump();
            let completion = BashBackgroundCompletion {
                task_id: record.id.clone(),
                command: record.command.clone(),
                text: outcome.text,
                is_error: outcome.is_error,
                duration_ms: record.started.elapsed().as_millis() as u64,
                exit_code: outcome.exit_code,
                description: record.description.clone(),
                log_path: record
                    .log_path
                    .as_ref()
                    .map(|p| p.to_string_lossy().into_owned()),
                log_issue: record.log_issue.as_ref().map(LogFileIssue::describe),
                lingering: record.lingering(),
                stopped_by_user: false,
            };
            prune_finished(&mut inner);
            completion
        };
        self.emit_change();
        self.emit(|l| {
            if let Listener::Complete(f) = l {
                f(&completion)
            }
        });
    }

    /// Running shells in launch order. Mirrors `list`.
    pub fn list(&self) -> Vec<BackgroundShellSnapshot> {
        self.lock()
            .shells
            .iter()
            .filter(|s| s.running())
            .map(ShellRecord::snapshot)
            .collect()
    }

    /// Every retained shell in launch order. Mirrors `snapshots`.
    pub fn snapshots(&self) -> Vec<BackgroundShellSnapshot> {
        self.lock()
            .shells
            .iter()
            .map(ShellRecord::snapshot)
            .collect()
    }

    pub fn get(&self, id: &str) -> Option<BackgroundShellSnapshot> {
        self.lock()
            .shells
            .iter()
            .find(|s| s.id == id)
            .map(ShellRecord::snapshot)
    }

    /// The error for an unknown id. Mirrors the TS message.
    pub fn unknown_shell_error(&self, id: &str) -> String {
        let ids: Vec<String> = self.lock().shells.iter().map(|s| s.id.clone()).collect();
        if ids.is_empty() {
            format!("Unknown background shell \"{id}\".")
        } else {
            format!(
                "Unknown background shell \"{id}\". Known: {}.",
                ids.join(", ")
            )
        }
    }

    fn tick_for(&self, id: &str) -> Option<watch::Receiver<u64>> {
        self.lock()
            .shells
            .iter()
            .find(|s| s.id == id)
            .map(|s| s.tick.subscribe())
    }

    fn state(&self, id: &str) -> Option<(bool, usize, usize)> {
        self.lock()
            .shells
            .iter()
            .find(|s| s.id == id)
            .map(|s| (s.running(), s.total_chars, s.read_cursor))
    }

    /// Wait up to `wait` for the shell to exit or (with `until=Output`) to
    /// produce unread output, then return only the output not previously
    /// returned and advance the read cursor. The wait ends early on `signal`
    /// or a queued steering message and never kills the shell. Mirrors
    /// `bash_output`'s wait plus `read`.
    pub async fn read(
        &self,
        id: &str,
        wait: Duration,
        until: WaitUntil,
        signal: &CancellationToken,
        steer: Option<&SteerProbe>,
    ) -> Result<BackgroundShellRead, String> {
        self.wait(id, wait, until, signal, steer).await?;
        self.take_new_output(id)
    }

    /// The wait half of [`Self::read`]; it leaves the read cursor alone, so
    /// log queries can wait without consuming new output.
    pub async fn wait(
        &self,
        id: &str,
        wait: Duration,
        until: WaitUntil,
        signal: &CancellationToken,
        steer: Option<&SteerProbe>,
    ) -> Result<(), String> {
        let Some(mut tick) = self.tick_for(id) else {
            return Err(self.unknown_shell_error(id));
        };
        let satisfied = |state: Option<(bool, usize, usize)>| match state {
            None => true,
            Some((running, total, cursor)) => {
                !running || (until == WaitUntil::Output && total > cursor)
            }
        };
        if !wait.is_zero() && !satisfied(self.state(id)) {
            let deadline = tokio::time::sleep(wait);
            tokio::pin!(deadline);
            let mut poll = tokio::time::interval(USER_MESSAGE_POLL);
            let steered = || steer.is_some_and(|probe| probe());
            if !steered() {
                loop {
                    tokio::select! {
                        _ = &mut deadline => break,
                        _ = signal.cancelled() => break,
                        changed = tick.changed() => {
                            if changed.is_err() || satisfied(self.state(id)) {
                                break;
                            }
                        }
                        _ = poll.tick() => {
                            if steered() {
                                break;
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Return the output not previously returned and advance the cursor.
    pub fn take_new_output(&self, id: &str) -> Result<BackgroundShellRead, String> {
        let mut inner = self.lock();
        let Some(record) = inner.shells.iter_mut().find(|s| s.id == id) else {
            drop(inner);
            return Err(self.unknown_shell_error(id));
        };
        let unread = record.total_chars - record.read_cursor;
        let retained = record.retained_text();
        let take = unread.min(record.retained_chars);
        let new_output: String = retained
            .chars()
            .skip(record.retained_chars - take)
            .collect();
        record.read_cursor = record.total_chars;
        Ok(BackgroundShellRead {
            shell: record.snapshot(),
            new_output,
            omitted_chars: unread - take,
        })
    }

    /// Stop a shell. A running shell is killed and marked `killed` at once;
    /// a finished one whose process group is still alive has that group
    /// terminated without changing its status. Mirrors `kill`.
    pub async fn kill(&self, id: &str, notify: bool) -> Result<KillOutcome, String> {
        enum Action {
            Killed(Option<BashBackgroundCompletion>),
            Linger(u32),
            Done(BackgroundShellStatus),
        }
        let action = {
            let mut inner = self.lock();
            let Some(record) = inner.shells.iter_mut().find(|s| s.id == id) else {
                drop(inner);
                return Err(self.unknown_shell_error(id));
            };
            if record.running() {
                record.status = BackgroundShellStatus::Killed;
                record.ended = Some(Instant::now());
                record.ended_at_ms = Some(now_ms());
                record.final_text = Some(if notify {
                    "Stopped by the user.".to_string()
                } else {
                    "Stopped with bash_kill.".to_string()
                });
                record.stdin = None;
                record.abort.cancel();
                record.bump();
                let completion = notify.then(|| BashBackgroundCompletion {
                    task_id: record.id.clone(),
                    command: record.command.clone(),
                    text: record.final_text.clone().unwrap_or_default(),
                    is_error: true,
                    duration_ms: record.started.elapsed().as_millis() as u64,
                    exit_code: None,
                    description: record.description.clone(),
                    log_path: None,
                    log_issue: None,
                    lingering: false,
                    stopped_by_user: true,
                });
                prune_finished(&mut inner);
                Action::Killed(completion)
            } else if let Some(pgid) = record.pgid().filter(|p| process_group_alive(*p)) {
                Action::Linger(pgid)
            } else {
                Action::Done(record.status)
            }
        };
        match action {
            Action::Killed(completion) => {
                self.emit_change();
                if let Some(completion) = completion {
                    self.emit(|l| {
                        if let Listener::Complete(f) = l {
                            f(&completion)
                        }
                    });
                }
                Ok(KillOutcome::Stopped)
            }
            Action::Linger(pgid) => {
                terminate_process_group(pgid).await;
                Ok(KillOutcome::StoppedLingering)
            }
            Action::Done(status) => Ok(KillOutcome::AlreadyFinished(status)),
        }
    }

    /// Write to a running shell's stdin. Mirrors `writeInput`.
    pub async fn write_input(
        &self,
        id: &str,
        text: Option<&str>,
        eof: bool,
    ) -> Result<InputWrite, String> {
        let bytes = text.unwrap_or("").as_bytes().to_vec();
        let sender = {
            let mut inner = self.lock();
            let Some(record) = inner.shells.iter_mut().find(|s| s.id == id) else {
                drop(inner);
                return Err(self.unknown_shell_error(id));
            };
            let Some(sender) = record.stdin.clone().filter(|_| record.running()) else {
                return Err(format!(
                    "{id} has no open stdin. Start the command with bash using run_in_background: true and stdin: true to send it input."
                ));
            };
            if bytes.len() > MAX_INPUT_BYTES_PER_CALL {
                return Err(format!(
                    "Input is {} bytes; at most {MAX_INPUT_BYTES_PER_CALL} bytes can be sent per call.",
                    bytes.len()
                ));
            }
            if record.input_bytes + bytes.len() > MAX_INPUT_BYTES_PER_SHELL {
                return Err(format!(
                    "{id} already received {} bytes of input; at most {MAX_INPUT_BYTES_PER_SHELL} bytes can be sent to one shell.",
                    record.input_bytes
                ));
            }
            // The budget is charged before the write is attempted.
            record.input_bytes += bytes.len();
            if eof {
                record.stdin = None;
            }
            sender
        };
        let written = bytes.len();
        let mut pending = false;
        if !bytes.is_empty() {
            let (done, accepted) = tokio::sync::oneshot::channel();
            if sender.send(StdinRequest::Write { bytes, done }).is_err() {
                return Err(format!("{id} has no open stdin."));
            }
            match tokio::time::timeout(STDIN_DRAIN_WAIT, accepted).await {
                Ok(Ok(Ok(()))) => {}
                Ok(Ok(Err(e))) => return Err(format!("Writing to {id} failed: {e}")),
                Ok(Err(_)) => return Err(format!("{id} closed its stdin.")),
                Err(_) => pending = true,
            }
        }
        if eof {
            let _ = sender.send(StdinRequest::Close);
        }
        if let Some(record) = self.lock().shells.iter().find(|s| s.id == id) {
            record.bump();
        }
        self.emit_change();
        Ok(InputWrite {
            bytes: written,
            pending,
        })
    }

    /// Read the newest lines of a shell's stored log, optionally only lines
    /// matching `grep`, without moving the incremental read cursor. Mirrors
    /// `readLog`.
    pub async fn read_log(
        &self,
        id: &str,
        tail_lines: usize,
        grep: Option<&regex::Regex>,
    ) -> Result<BackgroundLogRead, String> {
        let (snapshot, env) = {
            let inner = self.lock();
            let Some(record) = inner.shells.iter().find(|s| s.id == id) else {
                drop(inner);
                return Err(self.unknown_shell_error(id));
            };
            (record.snapshot(), record.env.clone())
        };
        let keep = tail_lines.max(1);
        let (text, skipped_bytes) = match &snapshot.log_path {
            Some(path) => {
                let limit = if grep.is_some() {
                    LOG_GREP_SCAN_BYTES
                } else {
                    LOG_TAIL_BYTES
                };
                match read_log_suffix(&*env, path, limit).await {
                    Some(result) => result,
                    None => (snapshot.output.clone(), 0),
                }
            }
            None => (snapshot.output.clone(), 0),
        };
        let mut lines: Vec<&str> = text.split('\n').collect();
        if lines.last().is_some_and(|l| l.is_empty()) {
            lines.pop();
        }
        let (lines, matched) = match grep {
            None => {
                let start = lines.len().saturating_sub(keep);
                (lines[start..].iter().map(|l| l.to_string()).collect(), None)
            }
            Some(re) => {
                let mut kept: VecDeque<String> = VecDeque::new();
                let mut matched = 0usize;
                for line in lines {
                    let probe: String = line.chars().take(LOG_LINE_PROBE_CHARS).collect();
                    if !re.is_match(&probe) {
                        continue;
                    }
                    matched += 1;
                    let shown = if probe.len() < line.len() {
                        format!("{probe} [line cut]")
                    } else {
                        probe
                    };
                    kept.push_back(shown);
                    if kept.len() > keep {
                        kept.pop_front();
                    }
                }
                (kept.into_iter().collect(), Some(matched))
            }
        };
        Ok(BackgroundLogRead {
            shell: snapshot,
            lines,
            matched,
            skipped_bytes,
        })
    }

    /// Register a running foreground call that can be moved to the
    /// background. The returned guard unregisters it (only if it is still
    /// this registration). Mirrors `registerForeground`.
    pub fn register_foreground(
        self: &Arc<Self>,
        tool_call_id: &str,
        release: impl FnOnce(ReleaseReason) + Send + 'static,
    ) -> ForegroundGuard {
        let mut inner = self.lock();
        inner.next_foreground += 1;
        let token = inner.next_foreground;
        inner.foreground.insert(
            tool_call_id.to_string(),
            (
                token,
                Foreground {
                    release: Box::new(release),
                },
            ),
        );
        ForegroundGuard {
            registry: Arc::downgrade(self),
            tool_call_id: tool_call_id.to_string(),
            token,
        }
    }

    pub fn has_releasable(&self) -> bool {
        !self.lock().foreground.is_empty()
    }

    /// Move every registered foreground call to the background (ctrl+b).
    /// Returns how many were released. Mirrors `releaseAll`.
    pub fn release_all(&self) -> usize {
        let released: Vec<Foreground> = {
            let mut inner = self.lock();
            inner.foreground.drain().map(|(_, (_, f))| f).collect()
        };
        let count = released.len();
        for foreground in released {
            (foreground.release)(ReleaseReason::User);
        }
        count
    }

    /// Keep a foreground log until disposal (or remove it at once if the
    /// registry is already disposed). Mirrors `trackLog`.
    pub fn track_log(&self, path: PathBuf, env: Arc<dyn ExecutionEnv>) {
        let disposed = {
            let mut inner = self.lock();
            if !inner.disposed {
                inner.tracked_logs.push((path.clone(), env.clone()));
            }
            inner.disposed
        };
        if disposed {
            tokio::spawn(remove_log(env, path));
        }
    }

    /// Stop every running shell and lingering process group, resolve
    /// waiters, remove owned logs and drop listeners. Mirrors `dispose`.
    pub async fn dispose(&self) {
        let (shells, logs) = {
            let mut inner = self.lock();
            inner.disposed = true;
            inner.foreground.clear();
            inner.listeners.clear();
            (
                std::mem::take(&mut inner.shells),
                std::mem::take(&mut inner.tracked_logs),
            )
        };
        for record in &shells {
            if record.running() {
                record.abort.cancel();
            } else if let Some(pgid) = record.pgid().filter(|p| process_group_alive(*p)) {
                terminate_process_group(pgid).await;
            }
            record.bump();
        }
        let mut shells = shells;
        for record in &mut shells {
            if let Some(done) = record.ingest_done.take() {
                let _ = done.await;
            }
        }
        for record in shells {
            if let Some(path) = record.log_path {
                remove_log(record.env, path).await;
            }
        }
        for (path, env) in logs {
            remove_log(env, path).await;
        }
    }
}

/// Unregisters a foreground call on drop. Mirrors the `registerForeground`
/// unregister closure: it only removes the matching registration.
pub struct ForegroundGuard {
    registry: std::sync::Weak<BashBackgroundRegistry>,
    tool_call_id: String,
    token: u64,
}

impl Drop for ForegroundGuard {
    fn drop(&mut self) {
        if let Some(registry) = self.registry.upgrade() {
            let mut inner = registry.lock();
            if inner
                .foreground
                .get(&self.tool_call_id)
                .is_some_and(|(token, _)| *token == self.token)
            {
                inner.foreground.remove(&self.tool_call_id);
            }
        }
    }
}

/// Newest bytes scanned by a log grep. Mirrors the 32 MiB scan window.
const LOG_GREP_SCAN_BYTES: u64 = 32 * 1024 * 1024;
/// Newest bytes read for a plain log tail. Mirrors the 1 MiB tail.
const LOG_TAIL_BYTES: u64 = 1024 * 1024;
/// Characters of each log line tested by grep. Mirrors 8192 code units.
const LOG_LINE_PROBE_CHARS: usize = 8 * 1024;

/// Read at most the last `limit` bytes of a log; drop the first partial line
/// when bytes were skipped. `None` falls back to the retained tail.
async fn read_log_suffix(env: &dyn ExecutionEnv, path: &str, limit: u64) -> Option<(String, u64)> {
    let (bytes, skipped) = env.read_binary_tail(path, limit, None).await.ok()?;
    let tail = if skipped == 0 {
        bytes.as_slice()
    } else {
        match bytes.iter().position(|b| *b == b'\n') {
            Some(nl) => &bytes[nl + 1..],
            None => &bytes[bytes.len()..],
        }
    };
    Some((String::from_utf8_lossy(tail).into_owned(), skipped))
}

async fn remove_log(env: Arc<dyn ExecutionEnv>, path: PathBuf) {
    let _ = env.remove(&path.to_string_lossy(), false, true, None).await;
}

fn append_chunk(record: &mut ShellRecord, text: &str) {
    if text.is_empty() {
        return;
    }
    let chars = text.chars().count();
    record.total_chars += chars;
    record.retained_chars += chars;
    record.chunks.push_back(text.to_string());
    record.last_output = Instant::now();
    record.last_output_at_ms = now_ms();
    // Drop exactly the excess over the cap from the oldest chunks.
    while record.retained_chars > MAX_OUTPUT_CHARS {
        let excess = record.retained_chars - MAX_OUTPUT_CHARS;
        let Some(front) = record.chunks.front_mut() else {
            break;
        };
        let front_chars = front.chars().count();
        if front_chars <= excess {
            record.retained_chars -= front_chars;
            record.chunks.pop_front();
        } else {
            let cut = front
                .char_indices()
                .nth(excess)
                .map_or(front.len(), |(i, _)| i);
            front.drain(..cut);
            record.retained_chars -= excess;
        }
    }
}

/// Keep the newest finished shells; running and lingering ones are exempt.
/// Mirrors `pruneFinished` (adoption order, not completion order).
fn prune_finished(inner: &mut Inner) {
    let finished: Vec<usize> = inner
        .shells
        .iter()
        .enumerate()
        .filter(|(_, s)| !s.running() && !s.lingering())
        .map(|(i, _)| i)
        .collect();
    if finished.len() <= MAX_FINISHED_SHELLS {
        return;
    }
    let drop_count = finished.len() - MAX_FINISHED_SHELLS;
    let to_drop: Vec<usize> = finished.into_iter().take(drop_count).collect();
    let mut index = 0usize;
    let mut removed = Vec::new();
    inner.shells.retain(|record| {
        let keep = !to_drop.contains(&index);
        index += 1;
        if !keep {
            if let Some(path) = &record.log_path {
                removed.push((record.env.clone(), path.clone()));
            }
        }
        keep
    });
    for (env, path) in removed {
        tokio::spawn(remove_log(env, path));
    }
}

/// First line of a command, plus how many more lines it has. Mirrors
/// `summarizeShellCommand`.
pub fn summarize_shell_command(command: &str) -> String {
    let trimmed = command.trim_end();
    let mut lines = trimmed.split('\n');
    let first = lines.next().unwrap_or("").to_string();
    let more = lines.count();
    if more == 0 {
        first
    } else {
        format!("{first} (+{more} more lines)")
    }
}

/// `Ns`, `Nm Ns` or `Nh Nm`, with seconds floored. Mirrors the bash_output
/// runtime display.
pub fn format_runtime(runtime: Duration) -> String {
    let secs = runtime.as_secs();
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    }
}

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Whether a process group still has members. `EPERM` counts as alive.
/// Always false on platforms without process groups. Mirrors
/// `isProcessGroupAlive`.
pub fn process_group_alive(pgid: u32) -> bool {
    #[cfg(unix)]
    {
        use nix::errno::Errno;
        use nix::sys::signal::kill;
        use nix::unistd::Pid;
        if pgid <= 1 {
            return false;
        }
        match kill(Pid::from_raw(-(pgid as i32)), None) {
            Ok(()) => true,
            Err(Errno::EPERM) => true,
            Err(_) => false,
        }
    }
    #[cfg(not(unix))]
    {
        let _ = pgid;
        false
    }
}

/// SIGTERM the group, poll every 100 ms, SIGKILL after 3 s if it is still
/// alive. Mirrors `terminateProcessGroup`.
pub async fn terminate_process_group(pgid: u32) {
    #[cfg(unix)]
    {
        use nix::sys::signal::{kill, Signal};
        use nix::unistd::Pid;
        if pgid <= 1 {
            return;
        }
        let group = Pid::from_raw(-(pgid as i32));
        let _ = kill(group, Signal::SIGTERM);
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(100)).await;
            if !process_group_alive(pgid) {
                return;
            }
        }
        let _ = kill(group, Signal::SIGKILL);
    }
    #[cfg(not(unix))]
    {
        let _ = pgid;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarize_shell_command_matches_ice() {
        assert_eq!(summarize_shell_command("npm run dev"), "npm run dev");
        assert_eq!(
            summarize_shell_command("a\nb\nc\n\n  \n"),
            "a (+2 more lines)"
        );
        assert_eq!(summarize_shell_command("  x  \n"), "  x");
        assert_eq!(summarize_shell_command("   "), "");
    }

    #[test]
    fn runtime_display() {
        assert_eq!(format_runtime(Duration::from_millis(59_999)), "59s");
        assert_eq!(format_runtime(Duration::from_secs(61)), "1m 1s");
        assert_eq!(format_runtime(Duration::from_secs(3_661)), "1h 1m");
    }

    #[tokio::test]
    async fn adoption_after_dispose_cancels_shell_and_removes_its_log() {
        use crate::env::{ExecutionEnv, ShellOutput};
        use crate::in_memory::InMemoryExecutionEnv;

        let registry = BashBackgroundRegistry::new();
        registry.dispose().await;
        let env: Arc<dyn ExecutionEnv> = Arc::new(InMemoryExecutionEnv::with_cwd("/w".into()));
        let mut capture = crate::shell_output::OutputCapture::new(env.clone(), None);
        capture.persist().await;
        let log_path = capture.log_path().unwrap();
        let (chunk_tx, chunks) = tokio::sync::mpsc::unbounded_channel();
        let (done_tx, done) = tokio::sync::oneshot::channel();
        let cancel = CancellationToken::new();
        let result = registry.adopt(Adoption {
            live: LiveShell {
                chunks,
                done,
                cancel: cancel.clone(),
                pid: Arc::new(std::sync::Mutex::new(None)),
                stdin: None,
            },
            capture,
            command: "sleep".into(),
            origin: ShellOrigin::Model,
            description: None,
            started: Instant::now(),
            started_at_ms: now_ms(),
            initial_output: String::new(),
            env: env.clone(),
            settle: Box::new(|_| SettledOutcome {
                text: String::new(),
                is_error: false,
                exit_code: Some(0),
            }),
        });

        assert!(result.is_err());
        assert!(cancel.is_cancelled());
        drop(chunk_tx);
        done_tx
            .send(Ok(ShellOutput {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: 0,
            }))
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if env
                    .file_info(&log_path.to_string_lossy(), None)
                    .await
                    .is_err()
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the rejected adoption's log should be removed");
        assert!(registry.snapshots().is_empty());
    }
}
