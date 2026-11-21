//! Coordination for one dedicated ACP RPC child. Ordinary RPC never installs it.

use super::{DeliveryTasks, SessionShells};
use ice_agent::AgentEvent;
use ice_harness::agent_harness::{AgentHarness, HarnessRunOutcome, SendMessageResult};
use ice_harness::events::{HarnessEvent, RunEndOutcome};
use ice_harness::types::{MessageDelivery, SendMessageOptions};
use ice_tools::bash_background::{BackgroundShellStatus, BashBackgroundCompletion};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use tokio::sync::{mpsc, OwnedMutexGuard};
use tokio_util::sync::CancellationToken;

static RUNTIME: OnceLock<Arc<AcpRuntime>> = OnceLock::new();

#[cfg(test)]
mod tests;

pub fn acp_rpc_enabled() -> bool {
    std::env::var("ICERS_ACP_RPC").as_deref() == Ok("1")
}

pub fn acp_runtime() -> Option<Arc<AcpRuntime>> {
    RUNTIME.get().cloned()
}

#[derive(Clone)]
struct Wake {
    id: String,
    acknowledged: CancellationToken,
    released: CancellationToken,
    cancel: CancellationToken,
    admitted: Arc<AtomicBool>,
}

struct Run {
    wake: Option<Wake>,
    outcome: Option<RunEndOutcome>,
}

#[derive(Clone, Copy, Default)]
pub(super) struct DeliveryEpoch {
    generation: u64,
    suppressed: bool,
}

/// The operation guard spans admission, the run, and the bridge's final usage
/// or response publication. Waiting happens in a task so RPC can read cancel and ack.
pub struct AcpRuntime {
    operation: Arc<tokio::sync::Mutex<()>>,
    admitting: Mutex<Option<Wake>>,
    runs: Mutex<HashMap<String, Run>>,
    acknowledgements: Mutex<HashMap<String, (CancellationToken, bool)>>,
    epoch: AtomicU64,
    stopping: AtomicBool,
    stopped_pending: AtomicU64,
    stopped_recorded: tokio::sync::Notify,
    closed: CancellationToken,
    events: mpsc::UnboundedSender<Value>,
    receiver: Mutex<Option<mpsc::UnboundedReceiver<Value>>>,
    watcher: Mutex<Option<ice_harness::watcher::HarnessWatcher>>,
    registry: Arc<ice_tools::bash_background::BashBackgroundRegistry>,
    watches: Arc<ice_tools::bash_watch::BashWatchRegistry>,
    deliveries: Arc<DeliveryTasks>,
}

impl AcpRuntime {
    pub(super) fn install(shells: &SessionShells, harness: &AgentHarness) -> Arc<Self> {
        let runtime = Self::new(shells, harness);
        let _ = RUNTIME.set(runtime.clone());
        runtime
    }

    fn new(shells: &SessionShells, harness: &AgentHarness) -> Arc<Self> {
        let (events, receiver) = mpsc::unbounded_channel();
        let runtime = Arc::new(Self {
            operation: Arc::default(),
            admitting: Mutex::new(None),
            runs: Mutex::default(),
            acknowledgements: Mutex::default(),
            epoch: AtomicU64::new(0),
            stopping: AtomicBool::new(false),
            stopped_pending: AtomicU64::new(0),
            stopped_recorded: tokio::sync::Notify::new(),
            closed: CancellationToken::new(),
            events,
            receiver: Mutex::new(Some(receiver)),
            watcher: Mutex::new(None),
            registry: shells.registry.clone(),
            watches: shells.watches.clone(),
            deliveries: shells.deliveries.clone(),
        });
        let weak = Arc::downgrade(&runtime);
        *runtime.watcher.lock().unwrap() = Some(harness.watch(move |event| {
            let Some(runtime) = weak.upgrade() else {
                return;
            };
            match event {
                HarnessEvent::RunStart(event) if event.lane == "main" => {
                    let wake = runtime.admitting.lock().unwrap().clone();
                    if let Some(wake) = &wake {
                        wake.admitted.store(true, Ordering::Release);
                    }
                    runtime.runs.lock().unwrap().insert(
                        event.run_id.clone(),
                        Run {
                            wake,
                            outcome: None,
                        },
                    );
                }
                HarnessEvent::RunEnd(event) if event.lane == "main" => {
                    if let Some(run) = runtime.runs.lock().unwrap().get_mut(&event.run_id) {
                        run.outcome = Some(event.outcome);
                    }
                }
                _ => {}
            }
        }));
        runtime
    }

    pub fn operation(&self) -> Arc<tokio::sync::Mutex<()>> {
        self.operation.clone()
    }

    pub fn take_events(&self) -> Option<mpsc::UnboundedReceiver<Value>> {
        self.receiver.lock().unwrap().take()
    }

    pub fn emit(&self, event: Value) {
        let _ = self.events.send(event);
    }

    pub async fn reserve(
        &self,
        cancelled: &CancellationToken,
    ) -> Result<OwnedMutexGuard<()>, String> {
        tokio::select! {
            biased;
            _ = cancelled.cancelled() => Err("Operation aborted".to_string()),
            _ = self.closed.cancelled() => Err("Session is closing".to_string()),
            guard = self.operation.clone().lock_owned() => Ok(guard),
        }
    }

    pub(super) fn epoch(&self) -> DeliveryEpoch {
        let _admission = self.admitting.lock().unwrap();
        DeliveryEpoch {
            generation: self.epoch.load(Ordering::Acquire),
            suppressed: self.stopping.load(Ordering::Acquire),
        }
    }

    fn stale(&self, epoch: DeliveryEpoch) -> bool {
        epoch.suppressed
            || self.stopping.load(Ordering::Acquire)
            || epoch.generation != self.epoch.load(Ordering::Acquire)
    }

    pub(super) fn completion(&self, completion: &BashBackgroundCompletion) {
        let _ = self.events.send(json!({
            "type": "background_shell",
            "shellId": completion.task_id,
            "status": if completion.is_error { "failed" } else { "completed" },
            "exitCode": completion.exit_code,
            "output": completion.text,
            "stoppedByUser": completion.stopped_by_user,
        }));
    }

    pub(super) fn begin_completion(&self, completion: &BashBackgroundCompletion) {
        if completion.stopped_by_user {
            self.stopped_pending.fetch_add(1, Ordering::AcqRel);
        } else {
            self.completion(completion);
        }
    }

    pub(super) fn finish_completion(&self, completion: &BashBackgroundCompletion) {
        if completion.stopped_by_user {
            self.completion(completion);
            self.stopped_pending.fetch_sub(1, Ordering::AcqRel);
            self.stopped_recorded.notify_one();
        }
    }

    /// Called only by the ordered AgentEvent consumer. RunEnd metadata is
    /// observed synchronously, but publication waits for the durable barrier.
    pub fn ordered_settlement(&self, event: &AgentEvent) -> Option<Value> {
        let AgentEvent::RunSettled { run_id } = event else {
            return None;
        };
        let run = self.runs.lock().unwrap().remove(run_id)?;
        let wake = run.wake?;
        let outcome = run.outcome?;
        Some(self.settlement(&wake, Some(run_id), outcome.as_str(), None))
    }

    fn settlement(
        &self,
        wake: &Wake,
        run_id: Option<&str>,
        outcome: &str,
        error: Option<String>,
    ) -> Value {
        self.acknowledgements
            .lock()
            .unwrap()
            .insert(wake.id.clone(), (wake.acknowledged.clone(), false));
        json!({
            "type": "agent_settled", "origin": "wake", "wakeId": wake.id,
            "runId": run_id, "outcome": outcome, "aborted": outcome == "aborted",
            "error": error, "summary": null,
        })
    }

    pub fn acknowledge(&self, id: &str) {
        self.acknowledge_kind(id, false);
    }

    pub async fn acknowledge_wake(&self, id: &str) {
        let released = self
            .admitting
            .lock()
            .unwrap()
            .as_ref()
            .filter(|wake| wake.id == id)
            .map(|wake| wake.released.clone());
        self.acknowledge(id);
        if let Some(released) = released {
            tokio::select! {
                _ = released.cancelled() => {}
                _ = self.closed.cancelled() => {}
            }
        }
    }

    pub fn acknowledge_client(&self, id: &str) -> bool {
        self.acknowledge_kind(id, true)
    }

    fn acknowledge_kind(&self, id: &str, client: bool) -> bool {
        let mut acknowledgements = self.acknowledgements.lock().unwrap();
        if acknowledgements
            .get(id)
            .is_some_and(|(_, is_client)| *is_client == client)
        {
            let (done, _) = acknowledgements.remove(id).unwrap();
            done.cancel();
            true
        } else {
            false
        }
    }

    pub fn client_settlement(&self) -> (String, CancellationToken) {
        let id = uuid::Uuid::new_v4().to_string();
        let done = CancellationToken::new();
        self.acknowledgements
            .lock()
            .unwrap()
            .insert(id.clone(), (done.clone(), true));
        (id, done)
    }

    pub async fn wait_client(&self, id: &str, done: CancellationToken) {
        tokio::select! {
            _ = done.cancelled() => {}
            _ = self.closed.cancelled() => {}
        }
        self.acknowledgements.lock().unwrap().remove(id);
    }

    pub fn cancel_wake(&self) {
        let admitting = self.admitting.lock().unwrap();
        self.epoch.fetch_add(1, Ordering::AcqRel);
        if let Some(wake) = admitting.as_ref() {
            wake.cancel.cancel();
        }
    }

    pub(super) async fn deliver(
        &self,
        harness: AgentHarness,
        message: ice_agent::AgentMessage,
        mut options: SendMessageOptions,
        epoch: DeliveryEpoch,
    ) {
        if self.stale(epoch) {
            options.trigger_turn = None;
            options.deliver_as = MessageDelivery::NextTurn;
        }
        let lane = harness.lane("main");
        if options.trigger_turn != Some(true) {
            if let Err(error) = lane.send_message(message, options).await {
                tracing::warn!("background notice was not recorded: {error}");
            }
            return;
        }
        let queued = CancellationToken::new();
        let Ok(_operation) = self.reserve(&queued).await else {
            return;
        };
        let wake = {
            // Cancellation and wake admission share this lock, so Stop cannot
            // slip between the generation check and publishing the new wake.
            let mut admitting = self.admitting.lock().unwrap();
            if self.stale(epoch) {
                None
            } else {
                let wake = Wake {
                    id: uuid::Uuid::new_v4().to_string(),
                    acknowledged: CancellationToken::new(),
                    released: CancellationToken::new(),
                    cancel: CancellationToken::new(),
                    admitted: Arc::new(AtomicBool::new(false)),
                };
                *admitting = Some(wake.clone());
                Some(wake)
            }
        };
        let Some(wake) = wake else {
            options.trigger_turn = None;
            options.deliver_as = MessageDelivery::NextTurn;
            let _ = lane.send_message(message, options).await;
            return;
        };
        let mut retain_notice = false;
        let result = {
            let turn = lane.send_message(message.clone(), options);
            tokio::pin!(turn);
            tokio::select! {
                biased;
                _ = self.closed.cancelled() => {
                    if lane.abort().await.is_ok() || wake.admitted.load(Ordering::Acquire) {
                        Some(turn.await)
                    } else { None }
                }
                _ = wake.cancel.cancelled() => {
                    if lane.abort().await.is_ok() || wake.admitted.load(Ordering::Acquire) {
                        Some(turn.await)
                    } else {
                        retain_notice = true;
                        None
                    }
                }
                result = &mut turn => Some(result),
            }
        };
        if retain_notice {
            // The unadmitted send future is now dropped. Admission is still
            // reserved and no native run exists, so append the original custom
            // notice for the next prompt without invoking input hooks again.
            if let Err(error) = lane.append_message(message).await {
                tracing::warn!("cancelled background notice was not recorded: {error}");
            }
        }
        let result = match result {
            Some(Ok(result)) => Some(result),
            Some(Err(error)) => {
                let _ = self.events.send(self.settlement(
                    &wake,
                    None,
                    "failed",
                    Some(error.to_string()),
                ));
                None
            }
            None => None,
        };
        // A suspended run has no RunEnd/RunSettled. Its fallback is drained
        // after prior AgentEvents by the RPC consumer.
        if let Some(SendMessageResult::Run(result)) = &result {
            if matches!(result.outcome, HarnessRunOutcome::Suspended { .. }) {
                self.runs.lock().unwrap().remove(&result.run_id);
                let _ = self.events.send(self.settlement(
                    &wake,
                    Some(&result.run_id),
                    "suspended",
                    None,
                ));
            }
        }
        if wake.admitted.load(Ordering::Acquire)
            || self.acknowledgements.lock().unwrap().contains_key(&wake.id)
        {
            tokio::select! {
                _ = wake.acknowledged.cancelled() => {}
                _ = self.closed.cancelled() => {}
            }
        }
        self.acknowledgements.lock().unwrap().remove(&wake.id);
        {
            let mut admitting = self.admitting.lock().unwrap();
            *admitting = None;
            drop(_operation);
            wake.released.cancel();
        }
    }

    pub async fn stop_background(&self, harness: &AgentHarness) {
        self.stopping.store(true, Ordering::Release);
        self.cancel_wake();
        let _ = harness.lane("main").abort().await;
        self.stop_lingering().await;
        for shell in self.registry.list() {
            let _ = self.registry.kill(&shell.task_id, true).await;
        }
        loop {
            let recorded = self.stopped_recorded.notified();
            if self.stopped_pending.load(Ordering::Acquire) == 0 {
                break;
            }
            recorded.await;
        }
        self.cancel_wake();
        self.stopping.store(false, Ordering::Release);
    }

    async fn stop_lingering(&self) {
        let mut tasks = tokio::task::JoinSet::new();
        for shell in self
            .registry
            .snapshots()
            .into_iter()
            .filter(|shell| shell.status != BackgroundShellStatus::Running)
        {
            let registry = self.registry.clone();
            tasks.spawn(async move {
                let _ = registry.kill(&shell.task_id, false).await;
            });
        }
        while tasks.join_next().await.is_some() {}
    }

    pub async fn shutdown(&self, harness: &AgentHarness) {
        self.closed.cancel();
        self.deliveries.close();
        self.watches.dispose();
        self.cancel_wake();
        let _ = harness.lane("main").abort().await;
        self.stop_lingering().await;
        self.registry.dispose().await;
        self.deliveries.close_and_drain().await;
    }
}
