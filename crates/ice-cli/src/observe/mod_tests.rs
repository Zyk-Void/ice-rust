//! End-to-end pipeline test: a faux-provider harness run feeds the recorder
//! through the CLI wiring and lands in the history store.

use super::*;
use crate::observe::types::{ObserveItemKind, ObserveRole, ObserveRunStatus};
use ice_ai::providers::faux::{FauxProvider, FauxScript};
use ice_ai::{Api, Model};
use ice_harness::types::AgentHarnessOptions;

/// A lagging forwarder reads the session after later turns were persisted. A
/// boundary must not project entries beyond the next unforwarded assistant
/// entry, or a later prompt/custom entry appears before the earlier answer.
#[tokio::test(flavor = "multi_thread")]
async fn lagging_boundary_read_stops_at_the_next_assistant_entry() {
    let script = FauxScript::new()
        .with_text("first answer")
        .with_text("second answer");
    let mut model = Model::new("m", "m", Api::Faux, "faux", "http://x");
    model.context_window = 200_000;
    let harness = AgentHarness::create(AgentHarnessOptions {
        model,
        models: vec![FauxProvider::new(script)],
        ..Default::default()
    })
    .await
    .unwrap();
    let mut cursor = EntryCursor {
        watermark: entry_watermark(&harness).await,
        ..EntryCursor::default()
    };
    // Everything below is persisted before the "forwarder" reads anything.
    harness
        .prompt_text("hello there", Vec::new())
        .await
        .unwrap();
    harness
        .append_custom_entry("ice-goals-state", Some(serde_json::json!({ "goals": [] })))
        .await
        .unwrap();
    harness.prompt_text("again", Vec::new()).await.unwrap();

    async fn read(harness: &AgentHarness, cursor: &mut EntryCursor, flush: bool) -> Vec<String> {
        let (tx, rx) = queue::channel();
        forward_new_entries(harness, cursor, &tx, flush).await;
        tx.close();
        let mut seen = Vec::new();
        while let Some(queue::Received::Command(command)) = rx.recv() {
            seen.push(match command {
                Command::Agent(AgentEvent::MessageStart { message }) => {
                    let json = serde_json::to_string(&message).unwrap();
                    let text = ["hello there", "again"]
                        .into_iter()
                        .find(|text| json.contains(text))
                        .unwrap_or("?");
                    format!("user:{text}")
                }
                Command::CustomEntry { custom_type, .. } => format!("custom:{custom_type}"),
                _ => "other".into(),
            });
        }
        seen
    }
    // First run's AgentStart: no answer event yet, so only its own prompt.
    assert_eq!(
        read(&harness, &mut cursor, false).await,
        ["user:hello there"]
    );
    // Its answer's message_end is forwarded, then TurnEnd reads the later
    // custom entry and prompt, stopping before the second (unseen) answer.
    cursor.assistant_events += 1;
    assert_eq!(
        read(&harness, &mut cursor, false).await,
        ["custom:ice-goals-state", "user:again"]
    );
    assert!(read(&harness, &mut cursor, false).await.is_empty());
    // AgentEnd flushes past an answer whose event never arrived.
    assert!(read(&harness, &mut cursor, true).await.is_empty());
    assert_eq!(cursor.assistant_entries, 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_run_is_recorded_with_prompts_custom_entries_and_finish() {
    let dir = tempfile::tempdir().unwrap();
    let (emitter, events) = ice_agent::events::BroadcastEmitter::new(256);
    let script = FauxScript::new()
        .with_text("first answer")
        .with_text("second answer");
    let mut model = Model::new("m", "m", Api::Faux, "faux", "http://x");
    model.context_window = 200_000;
    let harness = AgentHarness::create(AgentHarnessOptions {
        model,
        models: vec![FauxProvider::new(script)],
        agent_emitter: Some(Arc::new(emitter)),
        ..Default::default()
    })
    .await
    .unwrap();
    // Entries that exist before the recorder starts are not projected.
    harness
        .append_custom_entry(
            "ice-goals-state",
            Some(serde_json::json!({ "goals": [{ "id": "old" }] })),
        )
        .await
        .unwrap();

    let (runtime, warning) = start(&harness, events, Some(dir.path()), None).await;
    assert!(warning.is_none(), "{warning:?}");
    let store = runtime.store.clone().unwrap();
    let run_id = runtime.recorder.run_id();

    harness
        .prompt_text("hello there", Vec::new())
        .await
        .unwrap();
    harness
        .append_custom_entry(
            "ice-goals-state",
            Some(serde_json::json!({ "goals": [{ "id": "g1", "status": "active" }] })),
        )
        .await
        .unwrap();
    harness.prompt_text("again", Vec::new()).await.unwrap();
    runtime.stop().await;

    let run = store.get_run(&run_id).unwrap().unwrap();
    assert_eq!(run.status, ObserveRunStatus::Finished);
    assert_eq!(run.model.as_deref(), Some("faux/m"));
    let state = store.get_state(&run_id, None).unwrap().unwrap();
    let rows: Vec<(ObserveItemKind, Option<ObserveRole>, String)> = state
        .items
        .iter()
        .map(|i| (i.kind, i.role, i.text.clone().unwrap_or_default()))
        .collect();
    assert_eq!(
        rows,
        [
            (
                ObserveItemKind::Message,
                Some(ObserveRole::User),
                "hello there".to_string()
            ),
            (
                ObserveItemKind::Message,
                Some(ObserveRole::Assistant),
                "first answer".to_string()
            ),
            (
                ObserveItemKind::Goal,
                None,
                "goal active · tasks 0/0 · runs 0/? · reviews 0/3 · g1".to_string()
            ),
            (
                ObserveItemKind::Message,
                Some(ObserveRole::User),
                "again".to_string()
            ),
            (
                ObserveItemKind::Message,
                Some(ObserveRole::Assistant),
                "second answer".to_string()
            ),
        ]
    );
    assert!(state.header.usage.output > 0);
    assert_eq!(state.header.context.unwrap().context_window, 200_000);
}

#[tokio::test]
async fn an_unopenable_store_only_warns() {
    let dir = tempfile::tempdir().unwrap();
    // A file where the agent directory should be makes the store unopenable.
    let blocker = dir.path().join("agent");
    std::fs::write(&blocker, "not a dir").unwrap();
    let (emitter, events) = ice_agent::events::BroadcastEmitter::new(16);
    let harness = AgentHarness::create(AgentHarnessOptions {
        model: Model::new("m", "m", Api::Faux, "faux", "http://x"),
        models: vec![FauxProvider::new(FauxScript::new())],
        agent_emitter: Some(Arc::new(emitter)),
        ..Default::default()
    })
    .await
    .unwrap();
    let (runtime, warning) = start(&harness, events, Some(&blocker), None).await;
    assert!(warning
        .unwrap()
        .starts_with("Warning: observability history unavailable: "));
    assert!(runtime.store.is_none());
    runtime.stop().await;
}

async fn synthetic_runtime(sink: Arc<dyn ObservePersistence>) -> ObserveRuntime {
    let (_, events) = ice_agent::events::BroadcastEmitter::new(16);
    let harness = AgentHarness::create(AgentHarnessOptions {
        model: Model::new("m", "m", Api::Faux, "faux", "http://x"),
        models: vec![FauxProvider::new(FauxScript::new())],
        ..Default::default()
    })
    .await
    .unwrap();
    let recorder = ObserveRecorder::new(ObserveRecorderOptions {
        session_id: "shutdown-fixture".into(),
        info: ObserveSessionInfo::default(),
        persistence: Some(sink),
    });
    spawn(&harness, events, recorder, None, 0)
}

#[derive(Default)]
struct ControlledSink {
    armed: AtomicBool,
    fail_terminal: AtomicBool,
    entered: tokio::sync::Notify,
    released: (std::sync::Mutex<bool>, std::sync::Condvar),
    finished: std::sync::atomic::AtomicUsize,
}

impl ObservePersistence for ControlledSink {
    fn record_run(&self, _: &types::ObserveHeader) -> Result<(), String> {
        Ok(())
    }
    fn record_event(
        &self,
        _: &types::ObserveRecorderEvent,
        _: &types::ObserveHeader,
    ) -> Result<(), String> {
        if self.armed.swap(false, Ordering::AcqRel) {
            self.entered.notify_one();
            let (lock, ready) = &self.released;
            let mut released = lock.lock().unwrap();
            while !*released {
                released = ready.wait(released).unwrap();
            }
        }
        Ok(())
    }
    fn finish_run(&self, _: &str, _: i64) -> Result<(), String> {
        self.finished.fetch_add(1, Ordering::Relaxed);
        if self.fail_terminal.load(Ordering::Relaxed) {
            Err("synthetic failure".into())
        } else {
            Ok(())
        }
    }
    fn close(&self) -> Result<(), String> {
        Ok(())
    }
}

struct ReleaseWrite(Arc<ControlledSink>);
impl Drop for ReleaseWrite {
    fn drop(&mut self) {
        *self.0.released.0.lock().unwrap() = true;
        self.0.released.1.notify_all();
    }
}

#[tokio::test]
async fn observer_stop_reports_terminal_failure_without_claiming_durability() {
    let sink = Arc::new(ControlledSink::default());
    sink.fail_terminal.store(true, Ordering::Relaxed);
    let runtime = synthetic_runtime(sink).await;
    let report = runtime
        .stop_until(tokio::time::Instant::now() + std::time::Duration::from_secs(1))
        .await;
    assert!(report.drained && report.worker_finished && report.mirror_finished);
    assert!(report.persistence.terminal_failed);
    assert!(!report.persistence.terminal_persisted);
}

#[tokio::test]
async fn observer_stop_deadline_does_not_wait_for_blocked_sync_writer() {
    let sink = Arc::new(ControlledSink::default());
    let release = ReleaseWrite(sink.clone());
    let runtime = synthetic_runtime(sink.clone()).await;
    let recorder = runtime.recorder.clone();
    sink.armed.store(true, Ordering::Release);
    runtime.tx.send(Command::Lagged(1)).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(1), sink.entered.notified())
        .await
        .unwrap();
    let report = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        runtime.stop_until(tokio::time::Instant::now() + std::time::Duration::from_millis(20)),
    )
    .await
    .unwrap();
    assert!(!report.worker_finished);
    assert!(!report.persistence.terminal_persisted);
    assert_eq!(sink.finished.load(Ordering::Relaxed), 0);
    // Health must not acquire the lock held by the blocked writer.
    assert_eq!(recorder.persistence_health().failed_writes, 0);
    drop(release);
}

#[tokio::test]
async fn observer_stop_deadline_aborts_hung_forwarder_and_leaves_run_unfinished() {
    let sink = Arc::new(ControlledSink::default());
    let mut runtime = synthetic_runtime(sink.clone()).await;
    runtime.forward.take().unwrap().abort();
    let task = tokio::spawn(std::future::pending::<()>());
    let abort = task.abort_handle();
    runtime.forward = Some(task);
    let report = runtime
        .stop_until(tokio::time::Instant::now() + std::time::Duration::from_millis(20))
        .await;
    assert!(!report.drained);
    assert!(!report.worker_finished);
    assert!(!report.persistence.terminal_persisted);
    assert_eq!(sink.finished.load(Ordering::Relaxed), 0);
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while !abort.is_finished() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn cancelling_observer_stop_does_not_detach_hung_forwarder() {
    let sink = Arc::new(ControlledSink::default());
    let mut runtime = synthetic_runtime(sink.clone()).await;
    runtime.forward.take().unwrap().abort();
    let forward = tokio::spawn(std::future::pending::<()>());
    let abort = forward.abort_handle();
    runtime.forward = Some(forward);
    let stop = runtime.stop_until(tokio::time::Instant::now() + std::time::Duration::from_secs(60));
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(20), stop)
            .await
            .is_err()
    );
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while !abort.is_finished() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(sink.finished.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn dropping_observer_aborts_forwarding_and_does_not_finish_history() {
    let sink = Arc::new(ControlledSink::default());
    let runtime = synthetic_runtime(sink.clone()).await;
    let abort = runtime.forward.as_ref().unwrap().abort_handle();
    drop(runtime);
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while !abort.is_finished() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(sink.finished.load(Ordering::Relaxed), 0);
}
