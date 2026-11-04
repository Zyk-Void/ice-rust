//! Observability: an always-on forensic run history and an opt-in loopback
//! dashboard. Mirrors TS `packages/coding-agent/src/observe/*` and
//! `main.ts` wiring (ICE_PORTING.md §4, `docs/ice-gap/observability.md`
//! OBS-003..OBS-016).
//!
//! - [`recorder`] projects the live agent event stream into a bounded
//!   timeline (read-only; it never drives the agent).
//! - [`store`] persists that projection to `<agent dir>/observability.sqlite`,
//!   separate from session storage.
//! - [`server`] serves the dashboard, JSON state/replay, analytics and SSE
//!   on `127.0.0.1` behind a per-user token (`icers --observe[=port]`).
//! - [`analytics`] aggregates run lists and overviews from history facts.
//! - [`clickhouse`] optionally mirrors the history into ClickHouse
//!   (`ICERS_OBSERVE_CLICKHOUSE_URL`) and serves analytics from it.
//!
//! Cognee memory operations reach the timeline as `memory` items (OBS-008).
//! The Cognee knowledge views (OBS-017..019, OBS-024..029) are not ported.

pub mod analytics;
pub mod clickhouse;
mod queue;
pub mod recorder;
mod redaction;
pub mod server;
pub mod store;
pub mod types;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::cognee::observer::Observation;
use ice_agent::{AgentEvent, AgentMessage};
use ice_harness::agent_harness::{AgentHarness, AgentLane};
use ice_harness::events::HarnessEvent;
use ice_harness::session::types::{BranchBounds, Entry, EntryOrder, EntryQuery};
use ice_harness::watcher::HarnessWatcher;
use tokio::sync::broadcast;

pub use recorder::{
    ObservePersistence, ObserveRecorder, ObserveRecorderOptions, ObserveSessionInfo,
};
pub use store::ObserveRunStore;

/// History database file name under the agent dir.
pub const OBSERVE_DB_FILE: &str = "observability.sqlite";
/// Dashboard token file name under the agent dir.
pub const OBSERVE_TOKEN_FILE: &str = "observe.token";

pub fn db_path(agent_dir: &Path) -> PathBuf {
    agent_dir.join(OBSERVE_DB_FILE)
}

pub fn token_path(agent_dir: &Path) -> PathBuf {
    agent_dir.join(OBSERVE_TOKEN_FILE)
}

static STARTUP_NOTICES: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// Queue a notice for the interactive TUI, which clears the terminal on start.
pub fn push_startup_notice(notice: String) {
    if let Ok(mut notices) = STARTUP_NOTICES.lock() {
        notices.push(notice);
    }
}

/// Take the queued startup notices (the TUI shows them once).
pub fn take_startup_notices() -> Vec<String> {
    STARTUP_NOTICES
        .lock()
        .map(|mut notices| std::mem::take(&mut *notices))
        .unwrap_or_default()
}

#[derive(Debug)]
enum Command {
    Agent(AgentEvent),
    CustomEntry {
        custom_type: String,
        data: serde_json::Value,
    },
    Harness(HarnessEvent),
    Info(ObserveSessionInfo),
    Memory(Observation),
    Lagged(u64),
}

/// Observer-only shutdown budget. The caller may supply a stricter shared
/// deadline with `stop_until`; this does not extend any outer transport grace.
pub const DEFAULT_STOP_BUDGET: std::time::Duration = std::time::Duration::from_secs(2);

#[derive(Debug, Clone, Copy)]
pub struct ObserveStopReport {
    pub drained: bool,
    pub worker_finished: bool,
    pub mirror_finished: bool,
    pub persistence: recorder::PersistenceHealth,
}

/// A running observation pipeline. Dropping it without [`Self::stop`] leaves
/// the run marked `running` in history, as a crashed process would.
pub struct ObserveRuntime {
    pub recorder: ObserveRecorder,
    pub store: Option<Arc<ObserveRunStore>>,
    pub clickhouse: Option<Arc<clickhouse::ClickHouse>>,
    mirror: Option<clickhouse::ClickHouseMirror>,
    tx: queue::Sender,
    worker: Option<tokio::sync::oneshot::Receiver<bool>>,
    finish_allowed: Arc<AtomicBool>,
    forward: Option<tokio::task::JoinHandle<()>>,
    drain: Arc<tokio::sync::Notify>,
    poll: Option<tokio::task::JoinHandle<()>>,
    _watch: HarnessWatcher,
    _memory: crate::cognee::observer::Subscription,
}

async fn sample_info(harness: &AgentHarness) -> ObserveSessionInfo {
    let model = harness.get_model().await.ok();
    let thinking = harness.get_thinking_level().await.ok();
    ObserveSessionInfo {
        session_name: harness.get_name().await.ok().flatten(),
        model: model.as_ref().map(|m| format!("{}/{}", m.provider, m.id)),
        thinking_level: thinking
            .and_then(|t| serde_json::to_value(t).ok())
            .and_then(|v| v.as_str().map(str::to_string)),
        context_window: model.map(|m| m.context_window).filter(|w| *w > 0),
    }
}

async fn branch_entries(harness: &AgentHarness) -> Vec<Entry> {
    harness
        .find_entries_on_branch(
            &EntryQuery {
                order: Some(EntryOrder::OldestFirst),
                ..Default::default()
            },
            &BranchBounds::default(),
        )
        .await
        .unwrap_or_default()
}

/// Highest session entry sequence on the active branch.
/// Projection position in the session, and how many assistant entries versus
/// assistant `message_end` events have been forwarded.
#[derive(Debug, Default)]
struct EntryCursor {
    watermark: u64,
    assistant_entries: u64,
    assistant_events: u64,
}

async fn entry_watermark(harness: &AgentHarness) -> u64 {
    branch_entries(harness)
        .await
        .iter()
        .map(|entry| entry.base().seq)
        .max()
        .unwrap_or(0)
}

/// Forward session entries appended since `watermark`. The harness persists
/// prompts directly (the loop never emits a user `message_start`), and custom
/// state entries (goals, verifier, Subagents V2) never reach the event
/// stream, so they are read from the session — the Rust stand-in for the TS
/// session-manager `entry_appended` subscription. Like TS, entries present
/// when the recorder starts are not projected.
///
/// A lagging forwarder reads entries persisted after later turns. Unless
/// `flush`, the read stops before an assistant entry whose `message_end` the
/// event stream has not reached, so a later prompt or custom entry is not
/// projected ahead of an earlier answer. `AgentEnd` flushes: an assistant
/// entry without a matching event can delay entries by one run at most.
async fn forward_new_entries(
    harness: &AgentHarness,
    cursor: &mut EntryCursor,
    tx: &queue::Sender,
    flush: bool,
) {
    for entry in branch_entries(harness).await {
        let seq = entry.base().seq;
        if seq <= cursor.watermark {
            continue;
        }
        if matches!(&entry, Entry::Message(message) if matches!(message.message, AgentMessage::Assistant(_)))
        {
            if !flush && cursor.assistant_entries >= cursor.assistant_events {
                break;
            }
            cursor.assistant_entries += 1;
        }
        cursor.watermark = seq;
        let command = match entry {
            Entry::Message(message) if matches!(message.message, AgentMessage::User(_)) => {
                Command::Agent(AgentEvent::MessageStart {
                    message: message.message,
                })
            }
            Entry::Custom(custom) => Command::CustomEntry {
                custom_type: custom.custom_type,
                data: custom.data.unwrap_or(serde_json::Value::Null),
            },
            _ => continue,
        };
        let _ = tx.send(command);
    }
}

/// Open the history store and attach a recorder to the live event stream.
/// Mirrors TS `main.ts` observability startup. Never fails: a store that
/// cannot be opened yields a warning and a live-only recorder. With
/// `clickhouse`, the store is mirrored there while the run lasts.
pub async fn start(
    harness: &AgentHarness,
    events: broadcast::Receiver<AgentEvent>,
    agent_dir: Option<&Path>,
    clickhouse: Option<Arc<clickhouse::ClickHouse>>,
) -> (ObserveRuntime, Option<String>) {
    let mut warning = None;
    let store = match agent_dir {
        Some(dir) => match ObserveRunStore::open(&db_path(dir)) {
            Ok(store) => Some(Arc::new(store)),
            Err(error) => {
                warning = Some(format!(
                    "Warning: observability history unavailable: {error}"
                ));
                None
            }
        },
        None => {
            warning = Some("Warning: observability history unavailable: no agent dir".into());
            None
        }
    };
    let session_id = harness
        .session()
        .get_metadata()
        .await
        .map(|m| m.id)
        .unwrap_or_else(|_| "session".into());
    let recorder = ObserveRecorder::new(ObserveRecorderOptions {
        session_id,
        info: sample_info(harness).await,
        persistence: store.clone().map(|s| s as Arc<dyn ObservePersistence>),
    });
    let watermark = entry_watermark(harness).await;
    let mut runtime = spawn(harness, events, recorder, store, watermark);
    if let (Some(clickhouse), Some(store)) = (&clickhouse, &runtime.store) {
        runtime.mirror = Some(clickhouse::ClickHouseMirror::spawn(
            Arc::clone(clickhouse),
            Arc::clone(store),
            runtime.recorder.run_id(),
        ));
    }
    runtime.clickhouse = clickhouse;
    (runtime, warning)
}

fn spawn(
    harness: &AgentHarness,
    mut events: broadcast::Receiver<AgentEvent>,
    recorder: ObserveRecorder,
    store: Option<Arc<ObserveRunStore>>,
    watermark: u64,
) -> ObserveRuntime {
    let (tx, rx) = queue::channel();
    let worker_recorder = recorder.clone();
    let finish_allowed = Arc::new(AtomicBool::new(false));
    let worker_finish_allowed = finish_allowed.clone();
    let (worker_done, worker_result) = tokio::sync::oneshot::channel();
    let worker = std::thread::Builder::new()
        .name("icers-observe".into())
        .spawn(move || {
            let mut healthy = true;
            while let Some(received) = rx.recv() {
                let recorder = &worker_recorder;
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let command = match received {
                        queue::Received::Command(command) => command,
                        queue::Received::Gap(n) => {
                            recorder.note(&format!("observer queue lost {n} events; telemetry is incomplete and affected outcomes are unknown"));
                            return;
                        }
                    };
                    match command {
                        Command::Agent(event) => recorder.handle_agent_event(&event),
                        Command::Harness(HarnessEvent::CompactionProgress(progress)) => {
                            recorder.handle_compaction(progress.manual, progress.active)
                        }
                        Command::CustomEntry { custom_type, data } => {
                            recorder.handle_custom_entry(&custom_type, &data)
                        }
                        Command::Harness(_) => {}
                        Command::Info(info) => recorder.update_session_info(info),
                        Command::Memory(observation) => recorder.handle_memory(&observation),
                        Command::Lagged(n) => recorder.note(&format!(
                            "observer fell behind; {n} agent events were not recorded"
                        )),
                    }
                }));
                if outcome.is_err() {
                    healthy = false;
                    break;
                }
            }
            if healthy && worker_finish_allowed.load(Ordering::Acquire) {
                healthy = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    worker_recorder.stop();
                })).is_ok();
            } else {
                healthy = false;
            }
            let _ = worker_done.send(healthy);
        })
        .ok()
        .map(|_thread| worker_result);

    let forward_tx = tx.clone();
    let drain = Arc::new(tokio::sync::Notify::new());
    let forward_drain = Arc::clone(&drain);
    let forward_harness = harness.clone();
    let forward = tokio::spawn(async move {
        let harness = forward_harness;
        let tx = forward_tx;
        let mut cursor = EntryCursor {
            watermark,
            ..EntryCursor::default()
        };
        let mut forward_one = async |event: AgentEvent| -> bool {
            // Streaming deltas are not projected (TS parity); skip the copy.
            if matches!(event, AgentEvent::MessageUpdate { .. }) {
                return true;
            }
            // New prompts and custom entries are read from the session at
            // run/turn boundaries, ahead of the boundary event itself.
            if matches!(
                event,
                AgentEvent::AgentStart | AgentEvent::TurnEnd { .. } | AgentEvent::AgentEnd { .. }
            ) {
                let flush = matches!(event, AgentEvent::AgentEnd { .. });
                forward_new_entries(&harness, &mut cursor, &tx, flush).await;
            }
            if matches!(
                event,
                AgentEvent::MessageEnd {
                    message: AgentMessage::Assistant(_)
                }
            ) {
                cursor.assistant_events += 1;
            }
            tx.send(Command::Agent(event)).is_ok()
        };
        loop {
            tokio::select! {
                _ = forward_drain.notified() => {
                    // Shutdown: record whatever is already buffered, then stop.
                    loop {
                        match events.try_recv() {
                            Ok(event) => {
                                if !forward_one(event).await {
                                    break;
                                }
                            }
                            Err(broadcast::error::TryRecvError::Lagged(n)) => {
                                let _ = tx.send(Command::Lagged(n));
                            }
                            Err(_) => break,
                        }
                    }
                    break;
                }
                received = events.recv() => match received {
                    Ok(event) => {
                        if !forward_one(event).await {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        let _ = tx.send(Command::Lagged(n));
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                },
            }
        }
    });

    let poll_tx = tx.clone();
    let poll_harness = harness.clone();
    let poll = tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(std::time::Duration::from_millis(recorder::DEFAULT_POLL_MS));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            let info = sample_info(&poll_harness).await;
            if poll_tx.send(Command::Info(info)).is_err() {
                break;
            }
        }
    });

    let watch_tx = std::sync::Mutex::new(tx.clone());
    let watch = harness.watch_session(move |event| {
        if matches!(event, HarnessEvent::CompactionProgress(_)) {
            if let Ok(tx) = watch_tx.lock() {
                let _ = tx.send(Command::Harness(event.clone()));
            }
        }
    });

    // Process-wide, like TS `main.ts`: every Cognee operation this process
    // runs is recorded on the main thread.
    let memory_tx = std::sync::Mutex::new(tx.clone());
    let memory = crate::cognee::observer::subscribe(move |observation| {
        if let Ok(tx) = memory_tx.lock() {
            let _ = tx.send(Command::Memory(observation.clone()));
        }
    });

    ObserveRuntime {
        recorder,
        store,
        clickhouse: None,
        mirror: None,
        tx,
        worker,
        finish_allowed,
        forward: Some(forward),
        drain,
        poll: Some(poll),
        _watch: watch,
        _memory: memory,
    }
}

impl ObserveRuntime {
    /// Close the optional dashboard and drain recording concurrently under one
    /// absolute deadline. The server outcome is separate from persistence health.
    pub async fn stop_with_server_until(
        self,
        server: Option<server::ObserveServer>,
        deadline: tokio::time::Instant,
    ) -> (bool, ObserveStopReport) {
        tokio::join!(
            async move {
                match server {
                    Some(server) => server.close_until(deadline).await,
                    None => true,
                }
            },
            self.stop_until(deadline),
        )
    }

    /// Drain/finalize within the observer budget. Timeout is not a durable
    /// finish: any in-flight synchronous write remains explicitly unresolved.
    pub async fn stop(self) {
        self.stop_until(tokio::time::Instant::now() + DEFAULT_STOP_BUDGET)
            .await;
    }

    pub async fn stop_until(mut self, deadline: tokio::time::Instant) -> ObserveStopReport {
        if let Some(poll) = self.poll.take() {
            poll.abort();
        }
        self.drain.notify_one();
        let mut drained = true;
        if let Some(forward) = self.forward.as_mut() {
            drained = matches!(
                tokio::time::timeout_at(deadline, &mut *forward).await,
                Ok(Ok(()))
            );
            if !drained {
                forward.abort();
            }
        }
        self.forward.take();
        self.finish_allowed.store(drained, Ordering::Release);
        if drained {
            self.tx.close();
        } else {
            self.tx.abandon();
        }
        let worker_finished = if let Some(worker) = self.worker.take() {
            matches!(
                tokio::time::timeout_at(deadline, worker).await,
                Ok(Ok(true))
            )
        } else {
            false
        };
        if !worker_finished {
            self.finish_allowed.store(false, Ordering::Release);
            self.tx.abandon();
        }
        let mirror_finished = if let Some(mirror) = self.mirror.take() {
            mirror.finish_until(deadline).await
        } else {
            true
        };
        let persistence = self.recorder.persistence_health();
        if !drained
            || !worker_finished
            || !mirror_finished
            || persistence.terminal_failed
            || persistence.close_failed
            || persistence.failed_writes != 0
        {
            // Independent of the failed store. Never include sink errors or payloads.
            eprintln!("Warning: observer shutdown incomplete: drained={drained}, worker_finished={worker_finished}, mirror_finished={mirror_finished}, persistence={persistence:?}; unresolved writes/outcomes are unknown");
        }
        ObserveStopReport {
            drained,
            worker_finished,
            mirror_finished,
            persistence,
        }
    }
}

impl Drop for ObserveRuntime {
    fn drop(&mut self) {
        self.finish_allowed.store(false, Ordering::Release);
        self.tx.abandon();
        if let Some(forward) = &self.forward {
            forward.abort();
        }
        if let Some(poll) = &self.poll {
            poll.abort();
        }
        // A synchronous write already in progress cannot be cancelled. The
        // worker observes the disabled finish flag once that write returns.
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
