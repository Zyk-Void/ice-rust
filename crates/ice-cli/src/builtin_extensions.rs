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
    pub cognee: Option<Arc<crate::cognee::runtime::CogneeRuntime>>,
    pub blackhole: Option<Arc<crate::blackhole::engine::BlackholeEngine>>,
}

impl BuiltinExtensions {
    /// No built-ins (tests and embedded hosts).
    pub fn disabled() -> Self {
        Self {
            notices: NoticeMailbox::new(),
            agent_dir: None,
            cognee: None,
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
        let cognee = crate::cognee::runtime::CogneeRuntime::new(
            crate::cognee::runtime::RuntimeOptions::for_agent_dir(agent_dir.clone()),
            notices.clone(),
        );
        Self {
            notices,
            agent_dir: Some(agent_dir),
            cognee: Some(cognee),
            blackhole: Some(blackhole),
        }
    }

    /// Bind the live harness: register `cognee_search` (when the tool policy
    /// allows it) and give Cognee the footer status and working directory.
    pub async fn bind(
        &self,
        harness: &ice_harness::agent_harness::AgentHarness,
        status: ice_extensions::ExtensionStatusMailbox,
        cwd: PathBuf,
        search_tool_allowed: bool,
    ) {
        if let Some(cognee) = &self.cognee {
            cognee.bind(harness.downgrade(), status, cwd);
        }
        self.register_tools(search_tool_allowed).await;
    }

    /// Register built-in tools on the bound harness (again after a reload
    /// rebuilt its tool set).
    pub async fn register_tools(&self, search_tool_allowed: bool) {
        if let Some(cognee) = &self.cognee {
            cognee.register_search_tool(search_tool_allowed).await;
            cognee.apply_tool_state().await;
        }
    }

    /// The agent-event observer for built-ins, composed into the session
    /// emitter.
    pub fn agent_emitter(&self) -> Option<Arc<dyn ice_agent::AgentEmitter>> {
        self.cognee.as_ref().map(|cognee| {
            Arc::new(crate::cognee::runtime::CogneeEmitter(cognee.clone()))
                as Arc<dyn ice_agent::AgentEmitter>
        })
    }

    /// Skill files contributed by built-ins: Cognee's three skills once
    /// Cognee is configured.
    pub fn skill_paths(&self) -> Vec<PathBuf> {
        match &self.cognee {
            Some(cognee) if cognee.is_installed() => {
                crate::cognee::command::materialize_skills(cognee.storage_dir())
            }
            _ => Vec::new(),
        }
    }

    /// `session_start` for built-ins.
    pub async fn session_start(&self) {
        if let Some(cognee) = &self.cognee {
            cognee.session_start().await;
        }
    }

    /// `session_shutdown` for built-ins: drains Cognee's background work.
    pub async fn session_shutdown(&self) {
        if let Some(cognee) = &self.cognee {
            cognee.session_shutdown().await;
        }
    }

    /// Harness extensions in dispatch order. Cognee runs before Blackhole so
    /// its checkpoint recall is written before Blackhole consumes it.
    pub fn harness_extensions(&self) -> ice_harness::extension::HarnessExtensions {
        let mut extensions: ice_harness::extension::HarnessExtensions = Vec::new();
        if let Some(cognee) = &self.cognee {
            extensions.push(cognee.clone());
        }
        if let Some(blackhole) = &self.blackhole {
            extensions.push(blackhole.clone());
        }
        extensions
    }

    /// Namespaced `/settings` rows.
    pub fn settings_rows(&self) -> Vec<SettingRow> {
        let mut rows = Vec::new();
        if let Some(cognee) = &self.cognee {
            rows.extend(crate::cognee::command::settings_rows(&cognee.config()));
        }
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
        if let Some(cognee) = &self.cognee {
            if let Some(next) = crate::cognee::command::apply_setting(&cognee.config(), key, value)
            {
                return Some(
                    cognee
                        .persist_config_now(next)
                        .map(|()| format!("Cognee {}: {value}", &key["cognee.".len()..])),
                );
            }
        }
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
