//! Stable task-event ownership. A changed prompt or hash is not a new task.
//! The host provides lifecycle boundaries; explicit main-agent signals can
//! describe a task switch or a missing-history/capability dependency without
//! invoking a separate classifier.

use sha2::{Digest, Sha256};

use super::delivery::has_query_anchor;
use super::text::redact_memory_text;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TaskScope {
    pub session_id: String,
    pub lane_id: String,
    pub branch_generation: u64,
    pub repository_key: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskEvent {
    Started,
    Resumed,
    Changed,
    Compacted,
    HistoryGap,
    CapabilityMiss,
}

impl TaskEvent {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Started => "task_started",
            Self::Resumed => "task_resumed",
            Self::Changed => "task_changed",
            Self::Compacted => "context_compacted",
            Self::HistoryGap => "history_gap",
            Self::CapabilityMiss => "capability_miss",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct TaskCursor {
    scope: Option<TaskScope>,
    epoch: u64,
    query: String,
    last_compaction_at: Option<i64>,
    pending: Option<TaskEvent>,
    last_explicit_switch: Option<String>,
}

impl TaskCursor {
    pub fn id(&self) -> String {
        let scope = self.scope.as_ref();
        let value = format!(
            "{}:{}:{}:{}:{}",
            scope.map_or("", |scope| scope.session_id.as_str()),
            scope.map_or("", |scope| scope.lane_id.as_str()),
            scope.map_or(0, |scope| scope.branch_generation),
            scope.map_or("", |scope| scope.repository_key.as_str()),
            self.epoch,
        );
        format!("{:x}", Sha256::digest(value.as_bytes()))
    }

    pub fn query(&self) -> &str {
        &self.query
    }
    pub fn scope(&self) -> Option<&TaskScope> {
        self.scope.as_ref()
    }

    /// Return true only when optional result ownership changes. Compaction
    /// invalidates context coverage, not an otherwise valid retrieval result.
    pub fn observe(
        &mut self,
        scope: TaskScope,
        original_prompt: Option<&str>,
        latest_compaction_at: Option<i64>,
        resumed: bool,
    ) -> bool {
        let mut changed = self.scope.as_ref() != Some(&scope);
        if changed {
            self.scope = Some(scope);
            self.epoch = self.epoch.saturating_add(1);
            self.query.clear();
            self.last_explicit_switch = None;
            self.pending = Some(if resumed {
                TaskEvent::Resumed
            } else {
                TaskEvent::Started
            });
        }
        if let Some(prompt) = original_prompt {
            let prompt = prompt.trim();
            let lower = prompt.to_lowercase();
            // These are deliberate task-boundary requests, not guesses based
            // on semantic similarity or every new prompt's identity.
            let task_switch = [
                "new task:",
                "switch task:",
                "switch tasks:",
                "different task:",
            ]
            .iter()
            .any(|prefix| lower.starts_with(prefix));
            let key = format!("{:x}", Sha256::digest(prompt.as_bytes()));
            if task_switch && self.last_explicit_switch.as_deref() != Some(key.as_str()) {
                if !changed {
                    self.epoch = self.epoch.saturating_add(1);
                }
                self.query = redact_memory_text(prompt, 400);
                self.last_explicit_switch = Some(key);
                self.pending = Some(TaskEvent::Changed);
                changed = true;
            } else if self.query.is_empty() && has_query_anchor(prompt) {
                self.query = redact_memory_text(prompt, 400);
            }
        }
        if !changed
            && latest_compaction_at != self.last_compaction_at
            && latest_compaction_at.is_some()
        {
            self.pending = Some(TaskEvent::Compacted);
        }
        self.last_compaction_at = latest_compaction_at;
        changed
    }

    /// An explicit signal emitted by the already-running main model. It is
    /// advisory state, never a permission or an instruction to execute a tool.
    pub fn signal(&mut self, event: TaskEvent, query: &str) -> bool {
        if self.scope.is_none() {
            return false;
        }
        let query = redact_memory_text(query.trim(), 400);
        if query.is_empty() || !has_query_anchor(&query) {
            return false;
        }
        let ownership_changed = event == TaskEvent::Changed;
        if ownership_changed {
            self.epoch = self.epoch.saturating_add(1);
            self.last_explicit_switch = None;
        }
        self.query = query;
        self.pending = Some(event);
        true
    }

    pub fn take_event(&mut self) -> Option<TaskEvent> {
        self.pending.take()
    }
    pub fn pending_event(&self) -> Option<TaskEvent> {
        self.pending
    }
    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn scope() -> TaskScope {
        TaskScope {
            session_id: "s".into(),
            lane_id: "main".into(),
            branch_generation: 0,
            repository_key: "repo:work:main:rev".into(),
        }
    }

    #[test]
    fn ten_worded_followups_are_one_task_and_one_event() {
        let mut cursor = TaskCursor::default();
        assert!(cursor.observe(scope(), Some("fix cache eviction"), None, false));
        let id = cursor.id();
        assert_eq!(cursor.take_event(), Some(TaskEvent::Started));
        for prompt in [
            "continue",
            "ok",
            "do the next part",
            "inspect the parser",
            "run tests",
            "why is that",
            "show results",
            "try again",
            "yes please",
            "finish this",
        ] {
            assert!(!cursor.observe(scope(), Some(prompt), None, false));
            assert_eq!(cursor.id(), id);
            assert_eq!(cursor.take_event(), None);
        }
    }

    #[test]
    fn branches_and_explicit_task_changes_fence_old_results() {
        let mut cursor = TaskCursor::default();
        cursor.observe(scope(), Some("cache repair"), None, false);
        let first = cursor.id();
        assert!(cursor.observe(scope(), Some("new task: schema migration"), None, false));
        let second = cursor.id();
        assert_ne!(first, second);
        assert!(!cursor.observe(scope(), Some("new task: schema migration"), None, false));
        let mut branch = scope();
        branch.branch_generation = 1;
        assert!(cursor.observe(branch, Some("cache repair"), None, true));
        assert_ne!(cursor.id(), second);
        assert_eq!(cursor.pending_event(), Some(TaskEvent::Resumed));
    }

    #[test]
    fn compaction_is_a_context_event_not_a_new_task() {
        let mut cursor = TaskCursor::default();
        cursor.observe(scope(), Some("cache repair"), None, false);
        cursor.take_event();
        let id = cursor.id();
        assert!(!cursor.observe(scope(), Some("continue"), Some(10), false));
        assert_eq!(cursor.id(), id);
        assert_eq!(cursor.take_event(), Some(TaskEvent::Compacted));
        cursor.observe(scope(), Some("continue"), Some(10), false);
        assert_eq!(cursor.take_event(), None);
    }

    #[test]
    fn unknown_expansion_and_generic_phrases_do_not_become_queries() {
        let mut cursor = TaskCursor::default();
        cursor.observe(scope(), None, None, false);
        assert!(cursor.query().is_empty());
        assert!(!cursor.signal(TaskEvent::HistoryGap, "continue previous decision"));
        assert!(cursor.signal(TaskEvent::HistoryGap, "cache eviction decision"));
        assert_eq!(cursor.take_event(), Some(TaskEvent::HistoryGap));
    }
}
