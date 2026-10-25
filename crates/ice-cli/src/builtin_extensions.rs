//! Built-in native extensions (Cognee memory and Blackhole compaction) and
//! the small host surface they share: user notices and the footer status.
//!
//! A notice is shown only while an interactive UI is attached, mirroring the
//! TypeScript `ctx.hasUI` guard; headless modes drop them.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Severity of a user notice (`ui.notify` levels).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeLevel {
    Info,
    Warning,
    Error,
}

/// Queue of notices from background extension work to the TUI.
#[derive(Clone, Default)]
pub struct NoticeMailbox {
    attached: Arc<AtomicBool>,
    pending: Arc<Mutex<VecDeque<(NoticeLevel, String)>>>,
}

impl NoticeMailbox {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mark an interactive consumer as available.
    pub fn attach(&self) {
        self.attached.store(true, Ordering::SeqCst);
    }

    pub fn detach(&self) {
        self.attached.store(false, Ordering::SeqCst);
        if let Ok(mut pending) = self.pending.lock() {
            pending.clear();
        }
    }

    /// `ctx.hasUI`.
    pub fn has_ui(&self) -> bool {
        self.attached.load(Ordering::SeqCst)
    }

    /// Queue a notice; dropped when no UI is attached.
    pub fn notify(&self, level: NoticeLevel, text: impl Into<String>) {
        if !self.has_ui() {
            return;
        }
        if let Ok(mut pending) = self.pending.lock() {
            pending.push_back((level, text.into()));
        }
    }

    /// Take every queued notice in order.
    pub fn drain(&self) -> Vec<(NoticeLevel, String)> {
        self.pending
            .lock()
            .map(|mut pending| pending.drain(..).collect())
            .unwrap_or_default()
    }
}

/// One namespaced `/settings` row contributed by a built-in extension.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingRow {
    /// `<namespace>.<id>`, for example `blackhole.threshold`.
    pub key: String,
    pub label: String,
    pub description: String,
    pub current: String,
    pub values: Vec<String>,
}

impl SettingRow {
    pub fn new(key: &str, label: &str, description: &str, current: &str, values: &[&str]) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            description: description.into(),
            current: current.into(),
            values: values.iter().map(|value| value.to_string()).collect(),
        }
    }
}

/// The built-in extensions installed into one session's harness.
#[derive(Clone)]
pub struct BuiltinExtensions {
    pub notices: NoticeMailbox,
    pub agent_dir: Option<PathBuf>,
    pub blackhole: Option<Arc<crate::blackhole::engine::BlackholeEngine>>,
}

impl BuiltinExtensions {
    /// No built-ins (tests and embedded hosts).
    pub fn disabled() -> Self {
        Self {
            notices: NoticeMailbox::new(),
            agent_dir: None,
            blackhole: None,
        }
    }

    /// Built-ins for the effective icers agent dir.
    pub fn for_agent_dir(agent_dir: PathBuf) -> Self {
        let notices = NoticeMailbox::new();
        let blackhole = Arc::new(crate::blackhole::engine::BlackholeEngine::new(
            agent_dir.clone(),
            notices.clone(),
        ));
        Self {
            notices,
            agent_dir: Some(agent_dir),
            blackhole: Some(blackhole),
        }
    }

    /// Harness extensions in dispatch order. Cognee runs before Blackhole so
    /// its checkpoint recall is written before Blackhole consumes it.
    pub fn harness_extensions(&self) -> ice_harness::extension::HarnessExtensions {
        let mut extensions: ice_harness::extension::HarnessExtensions = Vec::new();
        if let Some(blackhole) = &self.blackhole {
            extensions.push(blackhole.clone());
        }
        extensions
    }

    /// Namespaced `/settings` rows.
    pub fn settings_rows(&self) -> Vec<SettingRow> {
        let mut rows = Vec::new();
        if let (Some(dir), Some(_)) = (&self.agent_dir, &self.blackhole) {
            rows.extend(crate::blackhole::command::settings_rows(
                dir,
                &crate::blackhole::config::process_env,
            ));
        }
        rows
    }

    /// Apply a namespaced settings change; `None` when no built-in owns `key`.
    pub fn apply_setting(&self, key: &str, value: &str) -> Option<Result<String, String>> {
        let dir = self.agent_dir.as_ref()?;
        if self.blackhole.is_some() {
            if let Some(result) = crate::blackhole::command::apply_setting(
                dir,
                &crate::blackhole::config::process_env,
                key,
                value,
            ) {
                return Some(result);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notices_need_an_attached_ui() {
        let mailbox = NoticeMailbox::new();
        mailbox.notify(NoticeLevel::Info, "dropped");
        assert!(mailbox.drain().is_empty());
        mailbox.attach();
        mailbox.notify(NoticeLevel::Warning, "kept");
        assert_eq!(
            mailbox.drain(),
            vec![(NoticeLevel::Warning, "kept".to_string())]
        );
        mailbox.notify(NoticeLevel::Info, "x");
        mailbox.detach();
        assert!(mailbox.drain().is_empty());
    }
}
