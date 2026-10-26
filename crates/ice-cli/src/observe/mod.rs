//! Observability: an always-on forensic run history and an opt-in loopback
//! dashboard. Mirrors TS `packages/coding-agent/src/observe/*` and
//! `main.ts` wiring (ICE_PORTING.md §4, `docs/ice-gap/observability.md`
//! OBS-003..OBS-016).
//!
//! - [`recorder`] projects the live agent event stream into a bounded
//!   timeline (read-only; it never drives the agent).
//! - [`store`] persists that projection to `<agent dir>/observability.sqlite`,
//!   separate from session storage.
//! - [`server`] serves the dashboard, JSON state/replay and SSE on
//!   `127.0.0.1` behind a per-user token (`icers --observe[=port]`).
//!
//! Cognee memory operations reach the timeline as `memory` items (OBS-008).
//! The Cognee knowledge views (OBS-017..019, OBS-024..029) are not ported.

pub mod recorder;
pub mod server;
pub mod store;
pub mod types;

use std::path::{Path, PathBuf};
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
    Stop,
}

/// A running observation pipeline. Dropping it without [`Self::stop`] leaves
/// the run marked `running` in history, as a crashed process would.
pub struct ObserveRuntime {
    pub recorder: ObserveRecorder,
    pub store: Option<Arc<ObserveRunStore>>,
    tx: std::sync::mpsc::Sender<Command>,
    worker: Option<std::thread::JoinHandle<()>>,
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
async fn forward_new_entries(
    harness: &AgentHarness,
    watermark: &mut u64,
    tx: &std::sync::mpsc::Sender<Command>,
) {
    for entry in branch_entries(harness).await {
        let seq = entry.base().seq;
        if seq <= *watermark {
            continue;
        }
        *watermark = seq;
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
/// cannot be opened yields a warning and a live-only recorder.
pub async fn start(
    harness: &AgentHarness,
    events: broadcast::Receiver<AgentEvent>,
    agent_dir: Option<&Path>,
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
    (spawn(harness, events, recorder, store, watermark), warning)
}

fn spawn(
    harness: &AgentHarness,
    mut events: broadcast::Receiver<AgentEvent>,
    recorder: ObserveRecorder,
    store: Option<Arc<ObserveRunStore>>,
    mut watermark: u64,
) -> ObserveRuntime {
    let (tx, rx) = std::sync::mpsc::channel::<Command>();
    let worker_recorder = recorder.clone();
    let worker = std::thread::Builder::new()
        .name("icers-observe".into())
        .spawn(move || {
            while let Ok(command) = rx.recv() {
                let recorder = &worker_recorder;
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
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
                        Command::Stop => return false,
                    }
                    true
                }));
                if !matches!(outcome, Ok(true)) {
                    break;
                }
            }
        })
        .ok();

    let forward_tx = tx.clone();
    let drain = Arc::new(tokio::sync::Notify::new());
    let forward_drain = Arc::clone(&drain);
    let forward_harness = harness.clone();
    let forward = tokio::spawn(async move {
        let harness = forward_harness;
        let tx = forward_tx;
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
                forward_new_entries(&harness, &mut watermark, &tx).await;
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
        tx,
        worker,
        forward: Some(forward),
        drain,
        poll: Some(poll),
        _watch: watch,
        _memory: memory,
    }
}

impl ObserveRuntime {
    /// Drain pending events, finish the run and close the store.
    pub async fn stop(mut self) {
        if let Some(poll) = self.poll.take() {
            poll.abort();
        }
        self.drain.notify_one();
        if let Some(forward) = self.forward.take() {
            let _ = forward.await;
        }
        let _ = self.tx.send(Command::Stop);
        if let Some(worker) = self.worker.take() {
            let _ = tokio::task::spawn_blocking(move || worker.join()).await;
        }
        self.recorder.stop();
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
