//! ACP-only observation of automatic compaction at native extension boundaries.
//!
//! The native RPC and plugin-action surfaces drive the root session's main lane.
//! Child agents have separate session IDs. This observer is scoped to that root
//! session and is not a general observer for arbitrary concurrent named lanes.

use std::sync::{Arc, Mutex};

use ice_agent::AgentMessage;
use ice_harness::agent_harness::AgentHarness;
use ice_harness::events::HarnessEvent;
use ice_harness::extension::{
    AfterCompactEvent, BeforeCompactDecision, BeforeCompactEvent, BeforeRunEvent, BoundaryDecision,
    BoundaryEvent, CompactionTrigger, HarnessExtension, HarnessExtensions, PromptContext,
    PromptContextOutcome, RequestContextEvent,
};
use ice_harness::watcher::HarnessWatcher;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::remote::protocol::{CompactionStatus, RemoteEvent};

type Emitter = Arc<dyn Fn(Value) + Send + Sync>;

struct Attempt {
    id: String,
    signal: CancellationToken,
}

pub(crate) struct CompactionObserver {
    session_id: String,
    pending: Mutex<Option<Attempt>>,
    emit: Emitter,
}

impl CompactionObserver {
    fn send(
        &self,
        id: String,
        status: CompactionStatus,
        summary: Option<String>,
        error: Option<String>,
    ) {
        let event = RemoteEvent::CompactionStatus {
            compaction_id: id,
            status,
            summary,
            error,
        };
        (self.emit)(serde_json::to_value(event).expect("compaction event is JSON"));
    }

    fn unfinished(&self, attempt: Attempt, error: &str, cancelled: bool) {
        self.send(
            attempt.id,
            if cancelled || attempt.signal.is_cancelled() {
                CompactionStatus::Cancelled
            } else {
                CompactionStatus::Failed
            },
            None,
            Some(error.to_string()),
        );
    }

    pub(crate) fn settle(&self, error: Option<&str>) {
        let attempt = self.pending.lock().unwrap().take();
        if let Some(attempt) = attempt {
            self.unfinished(
                attempt,
                error.unwrap_or("Compaction ended without a persisted checkpoint."),
                false,
            );
        }
    }

    fn declined(&self, event: &BeforeCompactEvent) {
        if event.session_id == self.session_id && event.trigger != CompactionTrigger::Manual {
            let attempt = self.pending.lock().unwrap().take();
            if let Some(attempt) = attempt {
                self.unfinished(attempt, "Compaction was declined by an extension.", true);
            }
        }
    }
}

#[async_trait::async_trait]
impl HarnessExtension for CompactionObserver {
    async fn before_compact(&self, event: &BeforeCompactEvent) -> Option<BeforeCompactDecision> {
        if event.session_id != self.session_id || event.trigger == CompactionTrigger::Manual {
            return None;
        }
        let id = uuid::Uuid::new_v4().to_string();
        let previous = self.pending.lock().unwrap().replace(Attempt {
            id: id.clone(),
            signal: event.signal.clone(),
        });
        if let Some(previous) = previous {
            self.unfinished(
                previous,
                "Compaction ended without a persisted checkpoint.",
                false,
            );
        }
        self.send(id, CompactionStatus::InProgress, None, None);
        None
    }

    async fn after_compact(&self, event: &AfterCompactEvent<'_>) {
        if event.session_id != self.session_id || event.trigger == CompactionTrigger::Manual {
            return;
        }
        let attempt = self.pending.lock().unwrap().take();
        if let Some(attempt) = attempt {
            // The host calls this only after both the checkpoint and its
            // durable step-attempt record have been appended.
            self.send(
                attempt.id,
                CompactionStatus::Completed,
                Some(event.entry.summary.clone()),
                None,
            );
        }
    }

    async fn request_context(&self, event: &RequestContextEvent<'_>) -> Option<PromptContext> {
        if event.session_id == self.session_id && event.lane_id == "main" {
            self.settle(None);
        }
        None
    }
}

// A cancelled extension decision has no later after_compact callback. Forward
// every hook unchanged while observing that one otherwise invisible outcome.
struct ObservedExtension {
    inner: Arc<dyn HarnessExtension>,
    observer: Arc<CompactionObserver>,
}

#[async_trait::async_trait]
impl HarnessExtension for ObservedExtension {
    async fn run_start(&self) {
        self.inner.run_start().await;
    }
    async fn before_run(&self, event: &BeforeRunEvent<'_>) -> Option<String> {
        self.inner.before_run(event).await
    }
    async fn prompt_context(&self, event: &BeforeRunEvent<'_>) -> Option<PromptContext> {
        self.inner.prompt_context(event).await
    }
    async fn request_context(&self, event: &RequestContextEvent<'_>) -> Option<PromptContext> {
        self.inner.request_context(event).await
    }
    async fn prompt_context_persisted(&self, context: &PromptContext, persisted: bool) {
        self.inner
            .prompt_context_persisted(context, persisted)
            .await;
    }
    async fn prompt_context_acknowledged(
        &self,
        context: &PromptContext,
        outcome: PromptContextOutcome,
    ) {
        self.inner
            .prompt_context_acknowledged(context, outcome)
            .await;
    }
    async fn before_compact(&self, event: &BeforeCompactEvent) -> Option<BeforeCompactDecision> {
        let decision = self.inner.before_compact(event).await;
        if decision == Some(BeforeCompactDecision::Cancel) {
            self.observer.declined(event);
        }
        decision
    }
    async fn after_compact(&self, event: &AfterCompactEvent<'_>) {
        self.inner.after_compact(event).await;
    }
    async fn boundary(&self, event: &BoundaryEvent) -> BoundaryDecision {
        self.inner.boundary(event).await
    }
    fn transform_context(
        &self,
        messages: &[AgentMessage],
        latest_compaction_at: Option<i64>,
    ) -> Option<Vec<AgentMessage>> {
        self.inner.transform_context(messages, latest_compaction_at)
    }
}

pub(crate) struct CompactionGuard {
    observer: Arc<CompactionObserver>,
    harness: AgentHarness,
    original: HarnessExtensions,
    installed: HarnessExtensions,
    _watcher: HarnessWatcher,
}

impl CompactionGuard {
    pub(crate) fn attach(harness: &AgentHarness, emit: Emitter) -> Self {
        let observer = Arc::new(CompactionObserver {
            session_id: harness.session().storage().metadata().id,
            pending: Mutex::new(None),
            emit,
        });
        let original = harness.extensions();
        let installed: HarnessExtensions = original
            .iter()
            .map(|inner| {
                Arc::new(ObservedExtension {
                    inner: Arc::clone(inner),
                    observer: Arc::clone(&observer),
                }) as Arc<dyn HarnessExtension>
            })
            .collect();
        let mut extensions: HarnessExtensions = vec![observer.clone()];
        extensions.extend(installed.iter().cloned());
        harness.set_extensions(extensions);
        let watching = Arc::clone(&observer);
        let watcher = harness.watch_session(move |event| {
            if matches!(event, HarnessEvent::RunEnd(event) if event.lane == "main") {
                watching.settle(None);
            }
        });
        Self {
            observer,
            harness: harness.clone(),
            original,
            installed,
            _watcher: watcher,
        }
    }

    pub(crate) fn observer(&self) -> Arc<CompactionObserver> {
        Arc::clone(&self.observer)
    }
}

impl Drop for CompactionGuard {
    fn drop(&mut self) {
        self.observer
            .settle(Some("Native RPC stopped before compaction completed."));
        let observer: Arc<dyn HarnessExtension> = self.observer.clone();
        let extensions = self
            .harness
            .extensions()
            .into_iter()
            .filter_map(|extension| {
                if Arc::ptr_eq(&extension, &observer) {
                    return None;
                }
                match self
                    .installed
                    .iter()
                    .position(|wrapped| Arc::ptr_eq(&extension, wrapped))
                {
                    Some(index) => Some(Arc::clone(&self.original[index])),
                    None => Some(extension),
                }
            })
            .collect();
        self.harness.set_extensions(extensions);
    }
}

#[cfg(test)]
mod tests;
