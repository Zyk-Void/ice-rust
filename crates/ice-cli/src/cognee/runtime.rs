//! The Cognee extension runtime: one memory state machine per icers
//! session. Ports `createIceCogneeExtension` from `ice-cognee.ts`
//! (COG-043..054, COG-061, COG-062).
//!
//! Host events arrive three ways: harness extension hooks (prompt recall and
//! compaction), agent events through [`CogneeEmitter`] (tool traces, answers,
//! settle-time improvement), and session start/shutdown from the CLI
//! lifecycle dispatch. Network work runs on tracked background tasks that
//! shutdown drains; nothing here can fail a run.
//!
//! icers installs Cognee only when `<agentDir>/ice-cognee/config.json`
//! exists or an `ICERS_COGNEE_*` variable is set, and never contacts the
//! server while running offline (`ICERS_OFFLINE` / `--offline`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};

use ice_agent::{AgentEvent, AgentMessage};
use ice_harness::extension::{
    AfterCompactEvent, BeforeCompactDecision, BeforeCompactEvent, BeforeRunEvent,
    CompactionOverride, CompactionTrigger, HarnessExtension, PromptContext,
};
use serde_json::{json, Map, Value};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::client::{
    CallOptions, ClientConfig, CogneeClient, CogneeError, CogneeTransport, RecallOptions,
    RecallResult, ReqwestTransport,
};
use super::config::{
    find_git_root, load_connection, process_env, resolve_runtime_dataset, save_config,
    write_private_file_atomic, AutoRememberMode, CogneeConfig, CogneeEnv, CompactionSummaryMode,
    ConnectionInputs, KeySource, PROJECT_DATASET_SENTINEL,
};
use super::dream::{
    claim_dream_candidate, mark_dream_captured, mark_dream_improved, read_dream_candidates,
    read_dream_record_for_dataset, release_dream_claim, DreamClaim,
};
use super::observer::{persist_observation, publish_observation, sanitize_endpoint, Observation};
use super::queue::{
    buffer_warmup_entry, create_pending_remember, enqueue_pending_remember, read_pending_remember,
    read_warmup_batch, remove_pending_remember, select_drain_batch, update_pending_remember_state,
    CircuitState, PendingState,
};
use super::text::{
    extract_message_text, format_last_compact_injection, format_recall, redact_memory_text,
    truncate_for_capture, truncate_value_for_capture, RecallLine,
};
use crate::blackhole::js::{iso_from_ms, len16, prefix16};
use crate::builtin_extensions::{NoticeLevel, NoticeMailbox};

pub const STATUS_KEY: &str = "ice-cognee";
pub const SESSION_ENTRY_TYPE: &str = "ice-cognee";
pub const SEARCH_TOOL_NAME: &str = "cognee_search";
const RECALL_MIN_WORDS: usize = 4;
const RECALL_DEDUP_TTL_MS: u64 = 15_000;
const MAX_TRACES_PER_TURN: u32 = 24;
const IDLE_IMPROVE_COOLDOWN_MS: u64 = 300_000;
const DREAM_IMPROVE_TIMEOUT_MS: u64 = 30_000;
const DREAM_MAX_SESSIONS: usize = 2;
/// Custom message type of the memory stored after each prompt.
pub const MEMORY_CUSTOM_TYPE: &str = "ice-cognee-memory";

/// Tools whose results are captured as traces.
pub const CAPTURE_TOOL_ALLOWLIST: [&str; 18] = [
    "bash", "Bash", "read", "Read", "write", "Write", "edit", "Edit", "grep", "Grep", "find",
    "Find", "ls", "Ls", "glob", "Glob", "agent", "Agent",
];

/// Tools whose output is file or listing contents. Their traces record what
/// was looked at, not what was read: the repository is the source of truth,
/// and stored contents (docs, skill files) were turned into misleading "facts"
/// in the memory graph.
const CONTENT_TOOLS: [&str; 5] = ["read", "grep", "find", "ls", "glob"];
/// Output kept from another tool's successful trace (a command's result).
const TRACE_RESULT_CHARS: usize = 600;
/// Prompt recall searches session memory and the graph. Raw tool traces are
/// left out: they matched on incidental words and crowded out real memories.
const AUTO_RECALL_SCOPE: [&str; 2] = ["session", "graph"];

/// Fewer than four words containing a letter or digit.
pub fn is_low_signal_recall_prompt(prompt: &str) -> bool {
    prompt
        .split_whitespace()
        .filter(|word| word.chars().any(char::is_alphanumeric))
        .count()
        < RECALL_MIN_WORDS
}

/// The newest user message in the summarized span with enough signal,
/// trimmed and capped at 400 UTF-16 units.
pub fn checkpoint_recall_query(messages_to_summarize: &[AgentMessage]) -> String {
    for message in messages_to_summarize.iter().rev() {
        if !matches!(message, AgentMessage::User(_)) {
            continue;
        }
        let text = extract_message_text(message);
        let text = text.trim();
        if !text.is_empty() && !is_low_signal_recall_prompt(text) {
            return prefix16(text, 400).to_string();
        }
    }
    String::new()
}

/// Blackhole counts as active when its config file is an object whose
/// compaction is not `off` and whose engine is `blackhole` or unset.
pub fn is_blackhole_compaction_active(agent_dir: &Path) -> bool {
    let path = crate::blackhole::config::config_path(agent_dir);
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    let Ok(Value::Object(raw)) = serde_json::from_str::<Value>(&text) else {
        return false;
    };
    if raw.get("compaction").and_then(Value::as_str) == Some("off") {
        return false;
    }
    match raw.get("compactionEngine") {
        None => true,
        Some(engine) => engine.as_str() == Some("blackhole"),
    }
}

/// Cognee returns a compaction summary only in `own` mode.
pub fn should_own_compaction_summary(mode: CompactionSummaryMode) -> bool {
    mode == CompactionSummaryMode::Own
}

/// The preparation fields the local anchor reads.
pub struct AnchorInput<'a> {
    pub previous_summary: Option<&'a str>,
    pub messages_to_summarize: &'a [AgentMessage],
    pub turn_prefix_messages: &'a [AgentMessage],
    pub read: Vec<String>,
    pub edited: Vec<String>,
    pub written: Vec<String>,
    pub tokens_before: Option<i64>,
}

/// `buildLocalPrecompactAnchor`.
pub fn build_local_precompact_anchor(input: &AnchorInput<'_>, reason: &str) -> String {
    let mut lines = vec![
        format!("Pre-compact local anchor ({reason})"),
        format!("messagesToSummarize={}", input.messages_to_summarize.len()),
        format!("turnPrefix={}", input.turn_prefix_messages.len()),
    ];
    if let Some(tokens) = input.tokens_before {
        lines.push(format!("tokensBefore≈{tokens}"));
    }
    for (label, paths) in [
        ("read", &input.read),
        ("edited", &input.edited),
        ("written", &input.written),
    ] {
        let paths: Vec<&str> = paths.iter().take(12).map(String::as_str).collect();
        if !paths.is_empty() {
            lines.push(format!("{label}: {}", paths.join(", ")));
        }
    }
    if let Some(previous) = input
        .previous_summary
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        lines.push("previousSummary:".into());
        lines.push(prefix16(previous, 2000).to_string());
    }
    lines.join("\n")
}

/// `buildOwnedCompactionSummary`: the anchor plus the first 40 visible
/// message texts, redacted and capped.
pub fn build_owned_compaction_summary(
    input: &AnchorInput<'_>,
    reason: &str,
    max_chars: usize,
) -> String {
    let mut parts = vec![build_local_precompact_anchor(input, reason)];
    parts.extend(
        input
            .messages_to_summarize
            .iter()
            .chain(input.turn_prefix_messages)
            .map(extract_message_text)
            .filter(|text| !text.is_empty())
            .take(40),
    );
    redact_memory_text(&parts.join("\n"), max_chars)
}

fn error_kind(error: &CogneeError) -> String {
    error.kind.as_str().to_string()
}

fn on_off(value: bool) -> &'static str {
    if value {
        "on"
    } else {
        "off"
    }
}

/// Session-local capture counters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Saves {
    pub prompt: u64,
    pub trace: u64,
    pub answer: u64,
}

/// `DoctorReport`.
#[derive(Debug, Clone, PartialEq)]
pub struct DoctorReport {
    pub enabled: bool,
    pub base_url: String,
    pub dataset: String,
    pub session_id: Option<String>,
    pub api_key_source: KeySource,
    pub healthy: bool,
    pub health_detail: String,
    pub capture_session: bool,
    pub capture_tools: bool,
    pub auto_recall: bool,
    pub auto_remember: AutoRememberMode,
    pub auto_improve: bool,
    pub compaction_summary_mode: CompactionSummaryMode,
    pub blackhole_active: bool,
    pub own_compaction_summary: bool,
    pub recall_circuit_open: bool,
    pub remember_circuit_open: bool,
    pub last_error: Option<String>,
    pub saves: Saves,
    pub last_recall_hits: usize,
    pub queue_pending: usize,
    pub queue_uncertain: usize,
}

impl DoctorReport {
    fn health(&self) -> &'static str {
        if self.healthy {
            "healthy"
        } else {
            "unreachable"
        }
    }

    /// The `/cognee status` line.
    pub fn status_line(&self) -> String {
        let circuit = |open: bool| if open { "open" } else { "closed" };
        format!(
            "Cognee {}; health {}; recall {}; remember {}; capture {}; tools {}; improve {}; compact={}{}{}; endpoint {}; dataset {}; session {}; key {}; saves p/t/a={}/{}/{}; lastRecall={}; queue pending={} uncertain={}; breaker recall={} remember={}; lastError={}",
            on_off(self.enabled),
            self.health(),
            on_off(self.auto_recall),
            self.auto_remember.as_str(),
            on_off(self.capture_session),
            on_off(self.capture_tools),
            on_off(self.auto_improve),
            self.compaction_summary_mode.as_str(),
            if self.blackhole_active { "+blackhole" } else { "" },
            if self.own_compaction_summary { "(own-summary)" } else { "(defer-summary)" },
            sanitize_endpoint(&self.base_url),
            self.dataset,
            self.session_id.as_deref().unwrap_or("none"),
            self.api_key_source.as_str(),
            self.saves.prompt,
            self.saves.trace,
            self.saves.answer,
            self.last_recall_hits,
            self.queue_pending,
            self.queue_uncertain,
            circuit(self.recall_circuit_open),
            circuit(self.remember_circuit_open),
            self.last_error.as_deref().unwrap_or("none"),
        )
    }

    /// The `/cognee doctor` line.
    pub fn doctor_line(&self) -> String {
        let circuit = |open: bool| if open { "open" } else { "closed" };
        [
            "mode=http".to_string(),
            "nativeRecall=POST:/api/v1/recall (support unverified; sends query, topK, onlyContext, scope, sessionId, datasets)".to_string(),
            "nativeImprove=POST:/api/v1/improve (sends datasetName, sessionIds, runInBackground; background acceptance only; completion status unsupported)".to_string(),
            "mcpCognee=separate catalog (not probed)".to_string(),
            format!("health={}", self.health()),
            format!("endpoint={}", sanitize_endpoint(&self.base_url)),
            format!("dataset={}", self.dataset),
            format!("session={}", self.session_id.as_deref().unwrap_or("none")),
            format!("apiKey={}", self.api_key_source.as_str()),
            format!("capture={}", self.capture_session),
            format!("tools={}", self.capture_tools),
            format!("improve={}", self.auto_improve),
            format!("compactMode={}", self.compaction_summary_mode.as_str()),
            format!("blackhole={}", self.blackhole_active),
            format!("ownSummary={}", self.own_compaction_summary),
            format!(
                "breaker=recall:{},remember:{}",
                circuit(self.recall_circuit_open),
                circuit(self.remember_circuit_open)
            ),
            format!(
                "saves={{\"prompt\":{},\"trace\":{},\"answer\":{}}}",
                self.saves.prompt, self.saves.trace, self.saves.answer
            ),
            format!("queue={}/{}", self.queue_pending, self.queue_uncertain),
            format!("detail={}", self.health_detail),
        ]
        .join(" · ")
    }
}

#[derive(Default)]
struct ObserveFields {
    request_id: Option<String>,
    latency_ms: Option<u64>,
    preview: Option<Value>,
    error: Option<String>,
    meta: Option<Map<String, Value>>,
}

fn meta(value: Value) -> Option<Map<String, Value>> {
    match value {
        Value::Object(map) => Some(map),
        _ => None,
    }
}

struct State {
    config: CogneeConfig,
    installed: bool,
    api_key: Option<String>,
    client: Option<CogneeClient>,
    loaded: bool,
    recall_circuit: CircuitState,
    remember_circuit: CircuitState,
    last_error: Option<String>,
    last_recall_key: Option<String>,
    last_recall_at: u64,
    last_compact_summary: Option<String>,
    resolved_dataset: String,
    improve_in_flight: bool,
    pending_prompt: Option<String>,
    session_id: Option<String>,
    host_session_id: Option<String>,
    started_host_session: Option<String>,
    agent_registered: bool,
    connected: bool,
    server_model: Option<String>,
    last_recall_hits: usize,
    saves: Saves,
    last_improve_at: u64,
    dream_dirty: bool,
    dream_in_flight: bool,
    last_dream_at: u64,
    key_source: KeySource,
    shutting_down: bool,
    turn_traces: u32,
}

impl State {
    fn new() -> Self {
        let config = CogneeConfig::default();
        Self {
            resolved_dataset: config.dataset.clone(),
            config,
            installed: false,
            api_key: None,
            client: None,
            loaded: false,
            recall_circuit: CircuitState::default(),
            remember_circuit: CircuitState::default(),
            last_error: None,
            last_recall_key: None,
            last_recall_at: 0,
            last_compact_summary: None,
            improve_in_flight: false,
            pending_prompt: None,
            session_id: None,
            host_session_id: None,
            started_host_session: None,
            agent_registered: false,
            connected: false,
            server_model: None,
            last_recall_hits: 0,
            saves: Saves::default(),
            last_improve_at: 0,
            dream_dirty: false,
            dream_in_flight: false,
            last_dream_at: 0,
            key_source: KeySource::None,
            shutting_down: false,
            turn_traces: 0,
        }
    }

    fn active_dataset(&self) -> String {
        if self.resolved_dataset.is_empty() {
            self.config.dataset.clone()
        } else {
            self.resolved_dataset.clone()
        }
    }
}

/// Construction inputs; tests replace the environment, transport and clock.
pub struct RuntimeOptions {
    pub agent_dir: PathBuf,
    pub storage_dir: Option<PathBuf>,
    pub process_env: CogneeEnv,
    pub home_dir: Option<PathBuf>,
    pub transport: Arc<dyn CogneeTransport>,
    pub now: Arc<dyn Fn() -> u64 + Send + Sync>,
    pub offline: bool,
    pub cwd: Option<PathBuf>,
}

impl RuntimeOptions {
    /// The live process environment, reqwest transport and wall clock.
    pub fn for_agent_dir(agent_dir: PathBuf) -> Self {
        Self {
            agent_dir,
            storage_dir: None,
            process_env: process_env(),
            home_dir: dirs::home_dir(),
            transport: Arc::new(ReqwestTransport::new()),
            now: Arc::new(|| crate::blackhole::now_ms().max(0) as u64),
            offline: crate::args::offline_env_enabled(),
            cwd: std::env::current_dir().ok(),
        }
    }
}

pub struct CogneeRuntime {
    me: Weak<CogneeRuntime>,
    agent_dir: PathBuf,
    storage_dir: PathBuf,
    process_env: CogneeEnv,
    home_dir: Option<PathBuf>,
    transport: Arc<dyn CogneeTransport>,
    now: Arc<dyn Fn() -> u64 + Send + Sync>,
    offline: bool,
    cwd: Mutex<Option<PathBuf>>,
    notices: NoticeMailbox,
    status: Mutex<Option<ice_extensions::ExtensionStatusMailbox>>,
    harness: Mutex<Option<ice_harness::agent_harness::WeakAgentHarness>>,
    state: Mutex<State>,
    background: Mutex<Vec<JoinHandle<()>>>,
    settle: Mutex<Option<JoinHandle<()>>>,
    tool_args: Mutex<HashMap<String, Value>>,
    /// Dream ledger writes run one at a time.
    dream_writes: tokio::sync::Mutex<()>,
    /// Single-flight pending drain.
    drain: tokio::sync::Mutex<()>,
}

impl CogneeRuntime {
    pub fn new(options: RuntimeOptions, notices: NoticeMailbox) -> Arc<Self> {
        let storage_dir = options
            .storage_dir
            .unwrap_or_else(|| options.agent_dir.join("ice-cognee"));
        Arc::new_cyclic(|me| Self {
            me: me.clone(),
            agent_dir: options.agent_dir,
            storage_dir,
            process_env: options.process_env,
            home_dir: options.home_dir,
            transport: options.transport,
            now: options.now,
            offline: options.offline,
            cwd: Mutex::new(options.cwd),
            notices,
            status: Mutex::new(None),
            harness: Mutex::new(None),
            state: Mutex::new(State::new()),
            background: Mutex::new(Vec::new()),
            settle: Mutex::new(None),
            tool_args: Mutex::new(HashMap::new()),
            dream_writes: tokio::sync::Mutex::new(()),
            drain: tokio::sync::Mutex::new(()),
        })
    }

    fn arc(&self) -> Arc<Self> {
        self.me.upgrade().expect("Cognee runtime is alive")
    }

    fn now(&self) -> u64 {
        (self.now)()
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }

    pub fn storage_dir(&self) -> &Path {
        &self.storage_dir
    }

    fn config_path(&self) -> PathBuf {
        self.storage_dir.join("config.json")
    }

    fn pending_dir(&self) -> PathBuf {
        self.storage_dir.join("pending")
    }

    fn warmup_dir(&self) -> PathBuf {
        self.storage_dir.join("warmup")
    }

    fn dream_dir(&self) -> PathBuf {
        self.storage_dir.join("dream")
    }

    fn checkpoint_recall_path(&self) -> PathBuf {
        self.storage_dir.join("compaction-recall.json")
    }

    /// Bind the live harness (for tools and session entries), the footer
    /// status registry and the working directory.
    pub fn bind(
        &self,
        harness: ice_harness::agent_harness::WeakAgentHarness,
        status: ice_extensions::ExtensionStatusMailbox,
        cwd: PathBuf,
    ) {
        *self.harness.lock().unwrap() = Some(harness);
        *self.status.lock().unwrap() = Some(status);
        *self.cwd.lock().unwrap() = Some(cwd);
    }

    fn harness(&self) -> Option<ice_harness::agent_harness::AgentHarness> {
        self.harness.lock().unwrap().as_ref()?.upgrade()
    }

    fn installed_now(&self) -> bool {
        self.config_path().exists()
            || self
                .process_env
                .keys()
                .any(|key| key.starts_with("ICERS_COGNEE_"))
    }

    /// Cognee may contact the server: installed, enabled and online.
    fn enabled(state: &State, offline: bool) -> bool {
        state.installed && state.config.enabled && !offline
    }

    /// Cognee is configured for this agent dir (config file or env).
    pub fn is_installed(&self) -> bool {
        self.ensure_loaded();
        self.state().installed
    }

    /// Whether automatic and manual memory operations run now.
    pub fn is_enabled(&self) -> bool {
        self.ensure_loaded();
        Self::enabled(&self.state(), self.offline)
    }

    fn client_config(state: &State) -> ClientConfig {
        ClientConfig {
            base_url: state.config.base_url.clone(),
            dataset: state.config.dataset.clone(),
            api_key: state.api_key.clone(),
            recall_budget_ms: state.config.recall_budget_ms,
            max_response_chars: state
                .config
                .recall_max_chars
                .max(state.config.max_response_chars),
        }
    }

    /// `ensureLoaded`: read connection and config once.
    pub fn ensure_loaded(&self) {
        if self.state().loaded {
            return;
        }
        let connection = load_connection(&ConnectionInputs {
            config_path: self.config_path(),
            storage_dir: self.storage_dir.clone(),
            home_dir: self.home_dir.clone(),
            env: None,
            process_env: self.process_env.clone(),
            api_key: None,
        });
        let installed = self.installed_now();
        let cwd = self.cwd.lock().unwrap().clone();
        let mut state = self.state();
        if state.loaded {
            return;
        }
        if connection.invalid_config {
            state.last_error = Some("invalid_config".into());
        }
        state.config = connection.config;
        state.installed = installed;
        state.resolved_dataset = resolve_runtime_dataset(&state.config.dataset, cwd.as_deref());
        state.api_key = connection.api_key;
        state.key_source = connection.key_source;
        state.client = Some(CogneeClient::new(
            Self::client_config(&state),
            self.transport.clone(),
        ));
        state.loaded = true;
    }

    fn rebuild_client(state: &mut State, transport: Arc<dyn CogneeTransport>) {
        state.client = Some(CogneeClient::new(Self::client_config(state), transport));
    }

    /// The client when Cognee may contact the server.
    fn live_client(&self) -> Option<CogneeClient> {
        let state = self.state();
        if Self::enabled(&state, self.offline) {
            state.client.clone()
        } else {
            None
        }
    }

    fn ensure_session_id(&self, host_id: &str) -> String {
        let id = super::config::cognee_session_id(host_id);
        let mut state = self.state();
        if state.session_id.as_deref() != Some(id.as_str()) {
            state.dream_dirty = false;
        }
        state.session_id = Some(id.clone());
        state.host_session_id = Some(host_id.into());
        id
    }

    // ── background work ──

    fn track(&self, work: impl std::future::Future<Output = ()> + Send + 'static) {
        let handle = tokio::spawn(work);
        let mut background = self.background.lock().unwrap();
        background.retain(|handle| !handle.is_finished());
        background.push(handle);
    }

    /// Await tracked work until none remains, including work added while
    /// waiting.
    pub async fn wait_for_background(&self) {
        loop {
            let handles: Vec<JoinHandle<()>> =
                std::mem::take(&mut *self.background.lock().unwrap());
            if handles.is_empty() {
                break;
            }
            for handle in handles {
                let _ = handle.await;
            }
        }
    }

    fn queue_dream_write(&self, write: impl FnOnce() + Send + 'static) {
        let this = self.arc();
        self.track(async move {
            let _guard = this.dream_writes.lock().await;
            let _ = tokio::task::spawn_blocking(write).await;
        });
    }

    fn notify(&self, level: NoticeLevel, text: impl Into<String>) {
        self.notices.notify(level, text);
    }

    fn observe(&self, operation: &str, phase: &str, fields: ObserveFields) {
        let (session_id, dataset, base_url, model) = {
            let state = self.state();
            if !state.installed {
                return;
            }
            (
                state.session_id.clone(),
                state.active_dataset(),
                state.config.base_url.clone(),
                state.server_model.clone(),
            )
        };
        let detail = fields.preview.as_ref().map(|value| {
            let text = match value {
                Value::String(text) => text.clone(),
                other => other.to_string(),
            };
            redact_memory_text(&text, super::observer::MAX_DETAIL_CHARS)
        });
        let preview = fields
            .preview
            .map(|value| redact_memory_text(&truncate_value_for_capture(&value, 900), 900));
        let observation = Observation {
            id: uuid::Uuid::new_v4().to_string(),
            at: iso_from_ms(chrono_now_ms()),
            agent_id: session_id.clone(),
            session_id,
            dataset: Some(dataset),
            endpoint: Some(sanitize_endpoint(&base_url)),
            operation: operation.into(),
            phase: phase.into(),
            request_id: fields.request_id,
            latency_ms: fields.latency_ms,
            status_code: None,
            preview,
            error: fields.error.map(|error| prefix16(&error, 160).to_string()),
            model,
            meta: fields.meta,
            detail,
        };
        // Listeners (the TUI panel) are notified here, in emission order; only
        // the file append runs off-thread. Notifying from the blocking task let
        // a "succeeded" overtake its "started" and leave a row stuck pending.
        let safe = publish_observation(observation);
        let storage = self.storage_dir.clone();
        let handle = tokio::task::spawn_blocking(move || persist_observation(&storage, &safe));
        self.background.lock().unwrap().push(handle);
    }

    fn append_operation(&self, data: Value) {
        let Some(harness) = self.harness() else {
            return;
        };
        self.track(async move {
            let _ = harness
                .append_custom_entry(SESSION_ENTRY_TYPE, Some(data))
                .await;
        });
    }

    // ── status line ──

    fn set_status(&self, value: Option<&str>) {
        if let Some(status) = self.status.lock().unwrap().as_ref() {
            status.set(STATUS_KEY, value.unwrap_or(""));
        }
    }

    /// The footer shows only whether Cognee is on; what it did is shown by
    /// the activity panel above the prompt.
    pub fn update_status_line(&self) {
        self.set_status(Some(if self.is_enabled() {
            "cognee"
        } else {
            "disabled"
        }));
    }

    // ── capture ──

    /// `storeEntry`: send one cache entry in the background.
    fn store_entry(&self, entry: Map<String, Value>, kind: &'static str) -> Option<()> {
        let (client, session_id, dataset) = {
            let state = self.state();
            if state.shutting_down || !Self::enabled(&state, self.offline) {
                return None;
            }
            (
                state.client.clone()?,
                state.session_id.clone()?,
                state.active_dataset(),
            )
        };
        let request_id = uuid::Uuid::new_v4().to_string();
        let started = std::time::Instant::now();
        let operation = if kind == "trace" {
            "trace"
        } else {
            "remember_entry"
        };
        self.observe(
            operation,
            "started",
            ObserveFields {
                request_id: Some(request_id.clone()),
                preview: Some(Value::Object(entry.clone())),
                meta: meta(json!({"kind": kind})),
                ..ObserveFields::default()
            },
        );
        let this = self.arc();
        self.track(async move {
            let entry_value = Value::Object(entry.clone());
            let result = client
                .remember_entry(
                    &entry_value,
                    Some(&dataset),
                    &session_id,
                    CallOptions::default(),
                )
                .await;
            let latency = Some(started.elapsed().as_millis() as u64);
            match result {
                Ok(_) => {
                    {
                        let mut state = this.state();
                        match kind {
                            "trace" => state.saves.trace += 1,
                            "answer" => {
                                state.saves.answer += 1;
                                state.saves.prompt += 1;
                            }
                            _ => state.saves.prompt += 1,
                        }
                    }
                    let captured_at = this.now();
                    let dream_dir = this.dream_dir();
                    let marker = this.clone();
                    let (session, data) = (session_id.clone(), dataset.clone());
                    this.queue_dream_write(move || {
                        if mark_dream_captured(&dream_dir, &session, &data, captured_at as f64)
                            .is_ok()
                        {
                            let mut state = marker.state();
                            if state.session_id.as_deref() == Some(session.as_str()) {
                                state.dream_dirty = true;
                            }
                        }
                    });
                    this.observe(
                        operation,
                        "succeeded",
                        ObserveFields {
                            request_id: Some(request_id),
                            latency_ms: latency,
                            meta: meta(json!({"kind": kind})),
                            ..ObserveFields::default()
                        },
                    );
                }
                Err(error) => {
                    let kind_name = error_kind(&error);
                    this.state().last_error = Some(kind_name.clone());
                    this.observe(
                        operation,
                        "failed",
                        ObserveFields {
                            request_id: Some(request_id),
                            latency_ms: latency,
                            error: Some(kind_name),
                            meta: meta(json!({"kind": kind})),
                            ..ObserveFields::default()
                        },
                    );
                    this.buffer_warmup(&entry);
                }
            }
        });
        Some(())
    }

    /// Buffer under the current session identity (as the TypeScript does).
    fn buffer_warmup(&self, entry: &Map<String, Value>) {
        let (session_id, dataset) = {
            let state = self.state();
            (state.session_id.clone(), state.active_dataset())
        };
        if let Some(session_id) = session_id {
            buffer_warmup_entry(&self.warmup_dir(), &session_id, &dataset, entry, self.now());
        }
    }

    /// Replay up to 16 buffered cache writes.
    fn drain_warmup(&self) {
        let Some(client) = self.live_client() else {
            return;
        };
        let (dir, dataset) = (self.warmup_dir(), self.state().active_dataset());
        self.track(async move {
            let batch = tokio::task::spawn_blocking(move || read_warmup_batch(&dir))
                .await
                .unwrap_or_default();
            for record in batch {
                let entry = Value::Object(record.entry.clone());
                let dataset = record.dataset.clone().unwrap_or_else(|| dataset.clone());
                if client
                    .remember_entry(
                        &entry,
                        Some(&dataset),
                        &record.session_id,
                        CallOptions::default(),
                    )
                    .await
                    .is_ok()
                {
                    let _ = std::fs::remove_file(&record.path);
                }
            }
        });
    }

    // ── permanent memory queue ──

    /// `drainPending`: single-flight replay of up to four queued writes.
    pub async fn drain_pending(&self, allow_uncertain: bool) {
        let Ok(_guard) = self.drain.try_lock() else {
            // Reuse the running drain.
            let _ = self.drain.lock().await;
            return;
        };
        let client = {
            let state = self.state();
            if state.shutting_down {
                return;
            }
            drop(state);
            match self.live_client() {
                Some(client) => client,
                None => return,
            }
        };
        let dir = self.pending_dir();
        let scan_dir = dir.clone();
        let records =
            tokio::task::spawn_blocking(move || select_drain_batch(&scan_dir, allow_uncertain))
                .await
                .unwrap_or_default();
        for record in records {
            if !self.state().remember_circuit.can_attempt(wall_ms()) {
                return;
            }
            let request_id = uuid::Uuid::new_v4().to_string();
            let started = std::time::Instant::now();
            let state_name = match record.state {
                PendingState::Pending => "pending",
                PendingState::Uncertain => "uncertain",
            };
            self.observe(
                "remember",
                "started",
                ObserveFields {
                    request_id: Some(request_id.clone()),
                    preview: Some(Value::String(record.text.clone())),
                    meta: meta(json!({"operationId": record.operation_id, "state": state_name})),
                    ..ObserveFields::default()
                },
            );
            let sent = client
                .remember(
                    &record.text,
                    &record.node_set,
                    Some(&record.dataset),
                    None,
                    CallOptions::default(),
                )
                .await;
            let latency = Some(started.elapsed().as_millis() as u64);
            let outcome = match sent {
                Ok(()) => remove_pending_remember(&dir, &record.operation_id)
                    .map_err(|_| "unknown".to_string()),
                Err(error) => Err(error_kind(&error)),
            };
            match outcome {
                Ok(()) => {
                    self.state().remember_circuit.reset();
                    self.observe(
                        "remember",
                        "succeeded",
                        ObserveFields {
                            request_id: Some(request_id),
                            latency_ms: latency,
                            meta: meta(json!({"operationId": record.operation_id})),
                            ..ObserveFields::default()
                        },
                    );
                    self.append_operation(json!({
                        "action": "remember",
                        "operationId": record.operation_id,
                        "contentHash": record.content_hash,
                        "dataset": record.dataset,
                        "nodeSet": record.node_set,
                        "state": "confirmed",
                    }));
                }
                Err(kind) => {
                    self.observe(
                        "remember",
                        "failed",
                        ObserveFields {
                            request_id: Some(request_id),
                            latency_ms: latency,
                            error: Some(kind.clone()),
                            meta: meta(
                                json!({"operationId": record.operation_id, "state": state_name}),
                            ),
                            ..ObserveFields::default()
                        },
                    );
                    let open = {
                        let mut state = self.state();
                        state.last_error = Some(kind.clone());
                        state.remember_circuit.note_failure(&kind, wall_ms());
                        !state.remember_circuit.can_attempt(wall_ms())
                    };
                    let ambiguous = drain_failure_state(&kind) == PendingState::Uncertain;
                    if ambiguous {
                        let _ = update_pending_remember_state(
                            &dir,
                            &record.operation_id,
                            PendingState::Uncertain,
                        );
                    }
                    self.append_operation(json!({
                        "action": "remember",
                        "operationId": record.operation_id,
                        "contentHash": record.content_hash,
                        "dataset": record.dataset,
                        "nodeSet": record.node_set,
                        "state": if ambiguous { "uncertain" } else { "pending" },
                        "error": kind,
                    }));
                    if open {
                        self.notify(
                            NoticeLevel::Warning,
                            "Cognee remember paused after repeated failures",
                        );
                    }
                }
            }
        }
    }

    fn spawn_drain_pending(&self) {
        let this = self.arc();
        self.track(async move { this.drain_pending(false).await });
    }

    // ── improvement ──

    /// `runImprove`: one current-session improve, claim-fenced.
    pub fn run_improve(&self, reason: &'static str) -> Option<JoinHandle<()>> {
        let now = self.now();
        let (client, session_id, dataset) = {
            let mut state = self.state();
            if !Self::enabled(&state, self.offline)
                || !state.config.auto_improve
                || state.improve_in_flight
            {
                return None;
            }
            let client = state.client.clone()?;
            let session_id = state.session_id.clone()?;
            if reason == "idle"
                && now.saturating_sub(state.last_improve_at) < IDLE_IMPROVE_COOLDOWN_MS
            {
                return None;
            }
            state.improve_in_flight = true;
            state.dream_dirty = false;
            (client, session_id, state.active_dataset())
        };
        let this = self.arc();
        let handle = tokio::spawn(async move {
            this.improve_current(client, session_id, dataset, reason)
                .await;
            let mut state = this.state();
            state.improve_in_flight = false;
            state.last_improve_at = this.now();
        });
        Some(handle)
    }

    async fn improve_current(
        &self,
        client: CogneeClient,
        session_id: String,
        dataset: String,
        reason: &'static str,
    ) {
        drop(self.dream_writes.lock().await);
        let token = uuid::Uuid::new_v4().to_string();
        let dir = self.dream_dir();
        let claim_now = self.now() as f64;
        let claim = {
            let (dir, session_id, dataset, token) = (
                dir.clone(),
                session_id.clone(),
                dataset.clone(),
                token.clone(),
            );
            tokio::task::spawn_blocking(move || {
                let snapshot = read_dream_record_for_dataset(&dir, &session_id, &dataset)?;
                Some(
                    claim_dream_candidate(&dir, &snapshot, claim_now, &token, false)
                        .ok()
                        .flatten(),
                )
            })
            .await
            .ok()
            .flatten()
        };
        // A record that exists but cannot be claimed belongs to another pass.
        let claim = match claim {
            Some(None) => return,
            Some(Some(record)) => Some(record),
            None => None,
        };
        let request_id = uuid::Uuid::new_v4().to_string();
        let started = std::time::Instant::now();
        let improved_at = self.now() as f64;
        self.observe(
            "improve",
            "started",
            ObserveFields {
                request_id: Some(request_id.clone()),
                meta: meta(json!({"reason": reason})),
                ..ObserveFields::default()
            },
        );
        let result = client
            .improve(
                Some(&dataset),
                std::slice::from_ref(&session_id),
                CallOptions {
                    timeout_ms: Some(DREAM_IMPROVE_TIMEOUT_MS),
                    ..CallOptions::default()
                },
            )
            .await;
        let latency = Some(started.elapsed().as_millis() as u64);
        match result {
            Ok(()) => {
                self.observe(
                    "improve",
                    "succeeded",
                    ObserveFields {
                        request_id: Some(request_id),
                        latency_ms: latency,
                        meta: meta(json!({"outcomeVersion": 1, "outcome": "accepted"})),
                        ..ObserveFields::default()
                    },
                );
                if let Some(claim) = &claim {
                    let dream_claim = DreamClaim {
                        token: token.clone(),
                        capture_revision: claim.capture_revision,
                        ledger_file: claim.ledger_file.clone(),
                    };
                    let (dir, session_id) = (dir.clone(), session_id.clone());
                    let _ = tokio::task::spawn_blocking(move || {
                        mark_dream_improved(&dir, &session_id, improved_at, &dream_claim)
                    })
                    .await;
                }
            }
            Err(error) => {
                let kind = error_kind(&error);
                self.state().last_error = Some(kind.clone());
                self.observe(
                    "improve",
                    "failed",
                    ObserveFields {
                        request_id: Some(request_id),
                        latency_ms: latency,
                        error: Some(kind),
                        ..ObserveFields::default()
                    },
                );
            }
        }
        if let Some(claim) = claim {
            let _ = tokio::task::spawn_blocking(move || {
                release_dream_claim(&dir, &session_id, &token, claim.ledger_file.as_deref())
            })
            .await;
        }
    }

    /// `runDream`: improve up to two older sessions.
    pub fn run_dream(&self) -> Option<JoinHandle<()>> {
        let now = self.now();
        let (client, dataset) = {
            let mut state = self.state();
            if !Self::enabled(&state, self.offline)
                || !state.config.auto_improve
                || state.dream_in_flight
                || state.shutting_down
            {
                return None;
            }
            let client = state.client.clone()?;
            if now.saturating_sub(state.last_dream_at) < IDLE_IMPROVE_COOLDOWN_MS {
                return None;
            }
            state.dream_in_flight = true;
            state.last_dream_at = now;
            (client, state.active_dataset())
        };
        let this = self.arc();
        Some(tokio::spawn(async move {
            this.dream_pass(client, dataset).await;
            this.state().dream_in_flight = false;
        }))
    }

    async fn dream_pass(&self, client: CogneeClient, dataset: String) {
        drop(self.dream_writes.lock().await);
        let dir = self.dream_dir();
        let scan_now = self.now() as f64;
        let current = self.state().session_id.clone();
        let candidates = {
            let dir = dir.clone();
            tokio::task::spawn_blocking(move || {
                read_dream_candidates(
                    &dir,
                    scan_now,
                    current.as_deref(),
                    DREAM_MAX_SESSIONS,
                    Some(&dataset),
                )
            })
            .await
            .unwrap_or_default()
        };
        for record in candidates {
            {
                let state = self.state();
                if state.shutting_down {
                    return;
                }
                if state.improve_in_flight
                    && state.session_id.as_deref() == Some(record.session_id.as_str())
                {
                    continue;
                }
            }
            let token = uuid::Uuid::new_v4().to_string();
            let claim_now = self.now() as f64;
            let claim = {
                let (dir, record, token) = (dir.clone(), record.clone(), token.clone());
                tokio::task::spawn_blocking(move || {
                    claim_dream_candidate(&dir, &record, claim_now, &token, true)
                        .ok()
                        .flatten()
                })
                .await
                .ok()
                .flatten()
            };
            let Some(claim) = claim else {
                continue;
            };
            let release = |dir: PathBuf, session: String, token: String, file: Option<String>| {
                tokio::task::spawn_blocking(move || {
                    release_dream_claim(&dir, &session, &token, file.as_deref())
                })
            };
            if self.state().shutting_down {
                let _ = release(
                    dir.clone(),
                    record.session_id.clone(),
                    token,
                    claim.ledger_file.clone(),
                )
                .await;
                return;
            }
            let request_id = uuid::Uuid::new_v4().to_string();
            let started = std::time::Instant::now();
            let improved_at = self.now() as f64;
            let meta_value = json!({"reason": "dream", "target": record.session_id});
            self.observe(
                "improve",
                "started",
                ObserveFields {
                    request_id: Some(request_id.clone()),
                    meta: meta(meta_value.clone()),
                    ..ObserveFields::default()
                },
            );
            let result = client
                .improve(
                    Some(&claim.dataset),
                    std::slice::from_ref(&claim.session_id),
                    CallOptions {
                        timeout_ms: Some(DREAM_IMPROVE_TIMEOUT_MS),
                        ..CallOptions::default()
                    },
                )
                .await;
            let latency = Some(started.elapsed().as_millis() as u64);
            let failed = match result {
                Ok(()) => {
                    let dream_claim = DreamClaim {
                        token: token.clone(),
                        capture_revision: claim.capture_revision,
                        ledger_file: claim.ledger_file.clone(),
                    };
                    let (dir, session) = (dir.clone(), claim.session_id.clone());
                    let _ = tokio::task::spawn_blocking(move || {
                        mark_dream_improved(&dir, &session, improved_at, &dream_claim)
                    })
                    .await;
                    self.observe(
                        "improve",
                        "succeeded",
                        ObserveFields {
                            request_id: Some(request_id),
                            latency_ms: latency,
                            meta: meta(json!({"reason": "dream", "target": record.session_id, "outcomeVersion": 1, "outcome": "accepted"})),
                            ..ObserveFields::default()
                        },
                    );
                    false
                }
                Err(error) => {
                    let kind = error_kind(&error);
                    self.state().last_error = Some(kind.clone());
                    self.observe(
                        "improve",
                        "failed",
                        ObserveFields {
                            request_id: Some(request_id),
                            latency_ms: latency,
                            error: Some(kind),
                            meta: meta(meta_value),
                            ..ObserveFields::default()
                        },
                    );
                    true
                }
            };
            let _ = release(
                dir.clone(),
                claim.session_id.clone(),
                token,
                claim.ledger_file.clone(),
            )
            .await;
            // The server is likely busy or down; leave the rest for later.
            if failed {
                return;
            }
        }
    }

    // ── health and doctor ──

    async fn probe_health(&self) -> (bool, String) {
        let client = self.state().client.clone();
        match client {
            Some(client) => client.probe_health().await,
            None => (false, "unreachable".into()),
        }
    }

    /// `collectDoctor`.
    pub async fn collect_doctor(&self) -> DoctorReport {
        self.ensure_loaded();
        let pending_dir = self.pending_dir();
        let records = tokio::task::spawn_blocking(move || read_pending_remember(&pending_dir))
            .await
            .unwrap_or_default();
        let enabled = Self::enabled(&self.state(), self.offline);
        let (healthy, detail) = if enabled {
            self.probe_health().await
        } else if self.offline {
            (false, "offline".to_string())
        } else {
            (false, "disabled".to_string())
        };
        let blackhole_active = is_blackhole_compaction_active(&self.agent_dir);
        let mut state = self.state();
        state.connected = healthy;
        let now = wall_ms();
        DoctorReport {
            enabled,
            base_url: state.config.base_url.clone(),
            dataset: state.active_dataset(),
            session_id: state.session_id.clone(),
            api_key_source: state.key_source,
            healthy,
            health_detail: detail,
            capture_session: state.config.capture_session,
            capture_tools: state.config.capture_tools,
            auto_recall: state.config.auto_recall,
            auto_remember: state.config.auto_remember,
            auto_improve: state.config.auto_improve,
            compaction_summary_mode: state.config.compaction_summary_mode,
            blackhole_active,
            own_compaction_summary: should_own_compaction_summary(
                state.config.compaction_summary_mode,
            ),
            recall_circuit_open: !state.recall_circuit.can_attempt(now),
            remember_circuit_open: !state.remember_circuit.can_attempt(now),
            last_error: state.last_error.clone(),
            saves: state.saves,
            last_recall_hits: state.last_recall_hits,
            queue_pending: records
                .iter()
                .filter(|record| record.state == PendingState::Pending)
                .count(),
            queue_uncertain: records
                .iter()
                .filter(|record| record.state == PendingState::Uncertain)
                .count(),
        }
    }

    // ── tools and config ──

    /// Register `cognee_search` once (when the tool policy allows it).
    pub async fn register_search_tool(&self, allowed: bool) {
        if !allowed {
            return;
        }
        let Some(harness) = self.harness() else {
            return;
        };
        let Ok(mut tools) = harness.get_tools().await else {
            return;
        };
        if tools
            .iter()
            .any(|tool| tool.tool.schema().name == SEARCH_TOOL_NAME)
        {
            return;
        }
        let Ok(active) = harness.lane("main").get_active_tools().await else {
            return;
        };
        tools.push(ice_harness::types::HarnessTool::new(Arc::new(
            super::command::CogneeSearchTool::new(self.me.clone()),
        )));
        let _ = harness.set_tools(tools, Some(active)).await;
    }

    /// `applyToolState`: toggle only `cognee_search` in the active set.
    pub async fn apply_tool_state(&self) {
        let Some(harness) = self.harness() else {
            return;
        };
        let enabled = Self::enabled(&self.state(), self.offline);
        let Ok(tools) = harness.get_tools().await else {
            return;
        };
        if !tools
            .iter()
            .any(|tool| tool.tool.schema().name == SEARCH_TOOL_NAME)
        {
            return;
        }
        let lane = harness.lane("main");
        let Ok(mut active) = lane.get_active_tools().await else {
            return;
        };
        let present = active.iter().any(|name| name == SEARCH_TOOL_NAME);
        if enabled && !present {
            active.push(SEARCH_TOOL_NAME.into());
        } else if !enabled && present {
            active.retain(|name| name != SEARCH_TOOL_NAME);
        } else {
            return;
        }
        let _ = lane.set_active_tools(active).await;
    }

    /// `persistConfig`: save, adopt, rebuild the client and re-apply the
    /// tool state. A failed save notifies and keeps the old config.
    pub async fn persist_config(&self, next: CogneeConfig) -> bool {
        if !self.save_and_adopt(next) {
            self.notify(NoticeLevel::Error, "Could not save Cognee configuration");
            return false;
        }
        self.apply_tool_state().await;
        self.update_status_line();
        true
    }

    /// Save and adopt `next` without touching the harness; the tool state
    /// is re-applied in the background.
    pub fn persist_config_now(&self, next: CogneeConfig) -> Result<(), String> {
        if !self.save_and_adopt(next) {
            return Err("Could not save Cognee configuration".into());
        }
        let this = self.arc();
        if tokio::runtime::Handle::try_current().is_ok() {
            tokio::spawn(async move {
                this.apply_tool_state().await;
                this.update_status_line();
            });
        }
        Ok(())
    }

    fn save_and_adopt(&self, next: CogneeConfig) -> bool {
        self.ensure_loaded();
        if save_config(&self.config_path(), &next).is_err() {
            return false;
        }
        let mut state = self.state();
        state.config = next;
        state.installed = true;
        let cwd = self.cwd.lock().unwrap().clone();
        state.resolved_dataset = resolve_runtime_dataset(&state.config.dataset, cwd.as_deref());
        Self::rebuild_client(&mut state, self.transport.clone());
        true
    }

    pub fn notices(&self) -> &NoticeMailbox {
        &self.notices
    }

    pub fn config(&self) -> CogneeConfig {
        self.ensure_loaded();
        self.state().config.clone()
    }

    pub fn active_dataset(&self) -> String {
        self.ensure_loaded();
        self.state().active_dataset()
    }

    pub fn offline(&self) -> bool {
        self.offline
    }

    // ── recall paths shared by the tool and commands ──

    /// Model-facing search: no session scope, no circuit admission check.
    pub async fn search_for_tool(&self, query: &str, signal: CancellationToken) -> (String, Value) {
        self.ensure_loaded();
        let dataset = self.active_dataset();
        let (client, top_k, max_chars) = {
            let state = self.state();
            (
                state.client.clone(),
                state.config.top_k,
                state.config.recall_max_chars,
            )
        };
        let Some(client) = client.filter(|_| self.is_enabled()) else {
            return (
                "Cognee is disabled.".into(),
                json!({"enabled": false, "count": 0, "dataset": dataset}),
            );
        };
        let result = client
            .recall(
                query,
                RecallOptions {
                    top_k: Some(top_k),
                    dataset: Some(dataset.clone()),
                    signal: Some(signal),
                    ..RecallOptions::default()
                },
            )
            .await;
        match result {
            Ok(results) => {
                self.state().recall_circuit.reset();
                let text = if results.is_empty() {
                    "No Cognee memory found.".to_string()
                } else {
                    format_recall(&recall_lines(&results), max_chars)
                };
                (
                    text,
                    json!({"enabled": true, "count": results.len(), "dataset": dataset}),
                )
            }
            Err(error) => {
                let kind = error_kind(&error);
                {
                    let mut state = self.state();
                    state.last_error = Some(kind.clone());
                    state.recall_circuit.note_failure(&kind, wall_ms());
                }
                (
                    format!("Cognee search failed: {kind}"),
                    json!({"enabled": true, "count": 0, "dataset": dataset, "error": kind}),
                )
            }
        }
    }

    /// `/cognee search`.
    pub async fn manual_search(&self, query: &str) {
        self.ensure_loaded();
        let Some(client) = self.live_client() else {
            self.notify(NoticeLevel::Warning, self.disabled_message());
            return;
        };
        let (top_k, max_chars, dataset) = {
            let state = self.state();
            (
                state.config.top_k,
                state.config.recall_max_chars,
                state.active_dataset(),
            )
        };
        let result = client
            .recall(
                query,
                RecallOptions {
                    top_k: Some(top_k),
                    dataset: Some(dataset),
                    ..RecallOptions::default()
                },
            )
            .await;
        match result {
            Ok(results) => {
                {
                    let mut state = self.state();
                    state.recall_circuit.reset();
                    state.last_error = None;
                }
                let text = if results.is_empty() {
                    "No Cognee memory found".to_string()
                } else {
                    format_recall(&recall_lines(&results), max_chars)
                };
                self.notify(NoticeLevel::Info, text);
            }
            Err(error) => {
                let kind = error_kind(&error);
                {
                    let mut state = self.state();
                    state.last_error = Some(kind.clone());
                    state.recall_circuit.note_failure(&kind, wall_ms());
                }
                self.notify(NoticeLevel::Error, format!("Cognee search failed: {kind}"));
            }
        }
    }

    fn disabled_message(&self) -> &'static str {
        if self.offline {
            "Cognee is disabled while icers runs offline"
        } else {
            "Cognee is disabled"
        }
    }

    /// `/cognee remember`: send now; queue as pending on any failure.
    pub async fn manual_remember(&self, node_set: &str, text: &str) {
        self.ensure_loaded();
        let Some(client) = self.live_client() else {
            self.notify(NoticeLevel::Warning, self.disabled_message());
            return;
        };
        let (max_chars, dataset, limit) = {
            let state = self.state();
            (
                state.config.remember_max_chars,
                state.active_dataset(),
                state.config.queue_limit,
            )
        };
        let safe_text = redact_memory_text(text.trim(), max_chars);
        if safe_text.is_empty() {
            self.notify(
                NoticeLevel::Warning,
                "Usage: /cognee remember [user_context|project_docs|agent_actions] <text>",
            );
            return;
        }
        match client
            .remember(
                &safe_text,
                node_set,
                Some(&dataset),
                None,
                CallOptions::default(),
            )
            .await
        {
            Ok(()) => {
                {
                    let mut state = self.state();
                    state.remember_circuit.reset();
                    state.last_error = None;
                }
                self.notify(NoticeLevel::Info, "Cognee memory stored");
            }
            Err(error) => {
                let kind = error_kind(&error);
                {
                    let mut state = self.state();
                    state.last_error = Some(kind.clone());
                    state.remember_circuit.note_failure(&kind, wall_ms());
                }
                let record = create_pending_remember(&safe_text, &dataset, node_set, self.now());
                match enqueue_pending_remember(&self.pending_dir(), &record, limit) {
                    Ok(true) => self.notify(
                        NoticeLevel::Warning,
                        format!("Cognee unavailable ({kind}); memory queued"),
                    ),
                    Ok(false) => self.notify(
                        NoticeLevel::Error,
                        format!("Cognee remember failed: {kind}"),
                    ),
                    Err(error) => self.notify(
                        NoticeLevel::Error,
                        format!("Cognee remember failed: {kind}; queue error: {error}"),
                    ),
                }
            }
        }
    }

    /// `/cognee improve [now]`: bypasses automatic scheduling while enabled.
    pub async fn manual_improve(&self) {
        self.ensure_loaded();
        let Some(client) = self.live_client() else {
            self.notify(NoticeLevel::Warning, self.disabled_message());
            return;
        };
        let (session_id, dataset) = {
            let state = self.state();
            (state.session_id.clone(), state.active_dataset())
        };
        let Some(session_id) = session_id else {
            self.notify(NoticeLevel::Warning, "Cognee improve: no session");
            return;
        };
        self.state().last_improve_at = 0;
        let request_id = uuid::Uuid::new_v4().to_string();
        let started = std::time::Instant::now();
        self.observe(
            "improve",
            "started",
            ObserveFields {
                request_id: Some(request_id.clone()),
                meta: meta(json!({"reason": "manual"})),
                ..ObserveFields::default()
            },
        );
        match client
            .improve(Some(&dataset), &[session_id], CallOptions::default())
            .await
        {
            Ok(()) => {
                self.state().last_improve_at = self.now();
                self.observe(
                    "improve",
                    "succeeded",
                    ObserveFields {
                        request_id: Some(request_id),
                        latency_ms: Some(started.elapsed().as_millis() as u64),
                        meta: meta(
                            json!({"reason": "manual", "outcomeVersion": 1, "outcome": "accepted"}),
                        ),
                        ..ObserveFields::default()
                    },
                );
                self.notify(NoticeLevel::Info, "Cognee improve dispatched");
            }
            Err(error) => {
                let kind = error_kind(&error);
                self.state().last_error = Some(kind.clone());
                self.observe(
                    "improve",
                    "failed",
                    ObserveFields {
                        request_id: Some(request_id),
                        latency_ms: Some(started.elapsed().as_millis() as u64),
                        error: Some(kind.clone()),
                        meta: meta(json!({"reason": "manual"})),
                        ..ObserveFields::default()
                    },
                );
                self.notify(NoticeLevel::Error, format!("Cognee improve failed: {kind}"));
            }
        }
    }

    // ── lifecycle ──

    /// `session_start`: bind identity, report connectivity in the background,
    /// register the server agent and replay queues. Runs once per host
    /// session.
    pub async fn session_start(&self) {
        let Some(harness) = self.harness() else {
            return;
        };
        let host_id = harness.session().storage().metadata().id;
        self.session_start_for(&host_id).await;
    }

    pub async fn session_start_for(&self, host_id: &str) {
        self.ensure_loaded();
        {
            let mut state = self.state();
            if state.started_host_session.as_deref() == Some(host_id) {
                return;
            }
            state.started_host_session = Some(host_id.into());
            let cwd = self.cwd.lock().unwrap().clone();
            state.resolved_dataset = resolve_runtime_dataset(&state.config.dataset, cwd.as_deref());
        }
        self.apply_tool_state().await;
        let session_id = self.ensure_session_id(host_id);
        let (enabled, label, base_url) = {
            let mut state = self.state();
            let mut label = format!("shared memory {}", state.active_dataset());
            if state.config.dataset == PROJECT_DATASET_SENTINEL {
                let cwd = self
                    .cwd
                    .lock()
                    .unwrap()
                    .clone()
                    .or_else(|| std::env::current_dir().ok())
                    .unwrap_or_default();
                let root = find_git_root(&cwd).to_string_lossy().into_owned();
                let shown = match self.home_dir.as_ref().map(|home| home.to_string_lossy()) {
                    Some(home) if root == home || root.starts_with(&format!("{home}/")) => {
                        format!("~{}", &root[home.len()..])
                    }
                    _ => root,
                };
                label = format!("project memory {} ({shown})", state.active_dataset());
            }
            label.push_str(&format!(" · session memory {session_id}"));
            state.connected = false;
            state.server_model = None;
            (
                Self::enabled(&state, self.offline),
                label,
                state.config.base_url.clone(),
            )
        };
        self.update_status_line();
        if !enabled {
            return;
        }
        let this = self.arc();
        let health_session = session_id.clone();
        self.track(async move {
            let (ok, detail) = this.probe_health().await;
            let stale = |this: &Self| {
                let state = this.state();
                state.shutting_down || state.session_id.as_deref() != Some(health_session.as_str())
            };
            if stale(&this) {
                return;
            }
            this.state().connected = ok;
            this.update_status_line();
            if ok {
                let client = this.state().client.clone();
                if let Some(client) = client {
                    let server = client.server_model(CallOptions::default()).await.ok();
                    if stale(&this) {
                        return;
                    }
                    this.state().server_model = server.as_ref().and_then(|s| s.model.clone());
                    this.observe(
                        "session",
                        "succeeded",
                        ObserveFields {
                            meta: meta(json!({
                                "llmProvider": server.as_ref().and_then(|s| s.provider.clone()),
                                "llmModel": server.as_ref().and_then(|s| s.model.clone()),
                            })),
                            ..ObserveFields::default()
                        },
                    );
                }
            }
            if ok {
                this.notify(
                    NoticeLevel::Info,
                    format!("Cognee Memory Connected · {label} · {base_url}"),
                );
            } else {
                this.notify(
                    NoticeLevel::Warning,
                    format!("Cognee Memory offline · {base_url} · {detail}"),
                );
            }
        });
        let register = {
            let state = self.state();
            (!state.agent_registered)
                .then(|| state.client.clone())
                .flatten()
                .map(|client| (client, state.active_dataset()))
        };
        if let Some((client, dataset)) = register {
            let this = self.arc();
            self.track(async move {
                match client
                    .register_agent(
                        &session_id,
                        Some(&session_id),
                        &[dataset],
                        CallOptions::default(),
                    )
                    .await
                {
                    Ok(()) => this.state().agent_registered = true,
                    Err(error) => this.state().last_error = Some(error_kind(&error)),
                }
            });
        }
        self.spawn_drain_pending();
        self.drain_warmup();
    }

    /// `session_shutdown`: drain, improve once more, then unregister.
    pub async fn session_shutdown(&self) {
        let (client, session_id) = {
            let mut state = self.state();
            state.shutting_down = true;
            (state.client.clone(), state.session_id.clone())
        };
        let settle = self.settle.lock().unwrap().take();
        if let Some(settle) = settle {
            let _ = settle.await;
        }
        self.wait_for_background().await;
        let (enabled, auto_improve) = {
            let state = self.state();
            (
                Self::enabled(&state, self.offline),
                state.config.auto_improve,
            )
        };
        if client.is_some() && session_id.is_some() && enabled && auto_improve {
            if let Some(handle) = self.run_improve("shutdown") {
                let _ = handle.await;
            }
        }
        self.wait_for_background().await;
        let registered = self.state().agent_registered;
        if let (Some(client), Some(session_id), true) = (client, session_id, registered) {
            if let Err(error) = client
                .unregister_agent(&session_id, CallOptions::default())
                .await
            {
                self.state().last_error = Some(error_kind(&error));
            }
        }
    }

    // ── agent events ──

    fn on_tool_result(
        &self,
        tool_name: &str,
        input: Value,
        content: &[ice_agent::types::TextContentOrImage],
        is_error: bool,
    ) {
        self.ensure_loaded();
        let max = {
            let mut state = self.state();
            if !Self::enabled(&state, self.offline)
                || !state.config.capture_session
                || !state.config.capture_tools
                || state.session_id.is_none()
            {
                return;
            }
            if tool_name == SEARCH_TOOL_NAME
                || (!CAPTURE_TOOL_ALLOWLIST.contains(&tool_name)
                    && !CAPTURE_TOOL_ALLOWLIST.contains(&tool_name.to_lowercase().as_str()))
            {
                return;
            }
            if state.turn_traces >= MAX_TRACES_PER_TURN {
                return;
            }
            state.turn_traces += 1;
            state.config.capture_max_chars
        };
        let limit = 4000.min(max);
        let mut params = Map::new();
        if let Value::Object(input) = input {
            for (key, value) in input {
                params.insert(
                    key,
                    Value::String(redact_memory_text(
                        &truncate_value_for_capture(&value, limit),
                        limit,
                    )),
                );
            }
        }
        let return_text = content
            .iter()
            .filter_map(|block| match block {
                ice_agent::types::TextContentOrImage::Text(text) if !text.text.is_empty() => {
                    Some(text.text.as_str())
                }
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        let content_tool = CONTENT_TOOLS.contains(&tool_name.to_lowercase().as_str());
        let safe_return = if content_tool && !is_error {
            String::new()
        } else {
            let return_limit = if is_error {
                limit
            } else {
                TRACE_RESULT_CHARS.min(max)
            };
            redact_memory_text(
                &truncate_for_capture(&return_text, return_limit),
                return_limit,
            )
        };
        let error_text = if return_text.is_empty() {
            "error"
        } else {
            return_text.as_str()
        };
        let safe_error = redact_memory_text(&truncate_for_capture(error_text, limit), limit);
        let entry = json!({
            "type": "trace",
            "origin_function": tool_name,
            "status": if is_error { "error" } else { "success" },
            "method_params": params,
            "method_return_value": safe_return,
            "error_message": if is_error { safe_error } else { String::new() },
            "generate_feedback_with_llm": false,
        });
        if let Value::Object(entry) = entry {
            self.store_entry(entry, "trace");
        }
    }

    fn on_agent_end(&self, messages: &[AgentMessage]) {
        self.ensure_loaded();
        let (question, host, max) = {
            let mut state = self.state();
            if !Self::enabled(&state, self.offline) || !state.config.capture_session {
                return;
            }
            let Some(question) = state.pending_prompt.take() else {
                return;
            };
            (
                question,
                state.host_session_id.clone(),
                state.config.capture_max_chars,
            )
        };
        if let Some(host) = host {
            self.ensure_session_id(&host);
        }
        let last = messages
            .iter()
            .rev()
            .find(|message| matches!(message, AgentMessage::Assistant(_)));
        let answer_text = last.map(extract_message_text).unwrap_or_default();
        let answer = redact_memory_text(&truncate_for_capture(&answer_text, max), max);
        if answer.is_empty() {
            return;
        }
        if let Value::Object(entry) = json!({
            "type": "qa",
            "question": question,
            "answer": answer,
            "context": "",
        }) {
            self.store_entry(entry, "answer");
        }
    }

    fn on_run_settled(&self) {
        let ready = {
            let state = self.state();
            state.loaded && Self::enabled(&state, self.offline) && state.config.auto_improve
        };
        if !ready {
            return;
        }
        let this = self.arc();
        let task = tokio::spawn(async move {
            this.wait_for_background().await;
            if let Some(handle) = this.run_improve("idle") {
                this.background.lock().unwrap().push(handle);
            }
            if let Some(handle) = this.run_dream() {
                this.background.lock().unwrap().push(handle);
            }
            this.update_status_line();
            this.drain_warmup();
        });
        *self.settle.lock().unwrap() = Some(task);
    }

    /// Feed one agent event.
    pub fn on_agent_event(&self, event: &AgentEvent) {
        match event {
            AgentEvent::ToolExecutionStart {
                tool_call_id, args, ..
            } => {
                self.tool_args
                    .lock()
                    .unwrap()
                    .insert(tool_call_id.clone(), args.clone());
            }
            AgentEvent::ToolExecutionEnd {
                tool_call_id,
                tool_name,
                result,
                is_error,
            } => {
                let input = self
                    .tool_args
                    .lock()
                    .unwrap()
                    .remove(tool_call_id)
                    .unwrap_or(Value::Object(Map::new()));
                self.on_tool_result(tool_name, input, &result.content, *is_error);
            }
            AgentEvent::AgentEnd { messages } => {
                // Calls aborted before their result never report an end.
                self.tool_args.lock().unwrap().clear();
                self.on_agent_end(messages);
            }
            AgentEvent::RunSettled { .. } => self.on_run_settled(),
            _ => {}
        }
    }

    // ── checkpoint recall for Blackhole ──

    fn clear_checkpoint_recall_for_session(&self, host_session_id: &str) {
        let path = self.checkpoint_recall_path();
        let Some(text) = crate::blackhole::checkpoint_recall::read_checkpoint_file(&path) else {
            return;
        };
        if let Ok(existing) = serde_json::from_str::<Value>(&text) {
            if let Some(owner) = existing.get("hostSessionId").and_then(Value::as_str) {
                if owner != host_session_id {
                    return;
                }
            }
        }
        let _ = std::fs::remove_file(path);
    }

    async fn maybe_write_checkpoint_recall(
        &self,
        event: &BeforeCompactEvent,
        session_id: &str,
        host_session_id: &str,
        blackhole_active: bool,
    ) {
        if !blackhole_active {
            return;
        }
        self.clear_checkpoint_recall_for_session(host_session_id);
        let (enabled, client, top_k, max_chars, dataset) = {
            let state = self.state();
            (
                state.config.checkpoint_recall,
                state.client.clone(),
                state.config.top_k,
                state.config.recall_max_chars,
                state.active_dataset(),
            )
        };
        if !enabled {
            return;
        }
        let Some(client) = client else {
            return;
        };
        if !self.state().recall_circuit.can_attempt(wall_ms()) {
            return;
        }
        let query = checkpoint_recall_query(&event.preparation.messages_to_summarize);
        if query.is_empty() {
            return;
        }
        let request_id = uuid::Uuid::new_v4().to_string();
        let started = std::time::Instant::now();
        self.observe(
            "recall",
            "started",
            ObserveFields {
                request_id: Some(request_id.clone()),
                preview: Some(Value::String(query.clone())),
                meta: meta(json!({"checkpoint": true})),
                ..ObserveFields::default()
            },
        );
        let result = client
            .recall(
                &query,
                RecallOptions {
                    top_k: Some(top_k),
                    dataset: Some(dataset),
                    session_id: Some(session_id.into()),
                    signal: Some(event.signal.clone()),
                    ..RecallOptions::default()
                },
            )
            .await;
        let latency = Some(started.elapsed().as_millis() as u64);
        let results = match result {
            Ok(results) => results,
            Err(error) => {
                let kind = error_kind(&error);
                {
                    let mut state = self.state();
                    state.last_error = Some(kind.clone());
                    state.recall_circuit.note_failure(&kind, wall_ms());
                }
                self.observe(
                    "recall",
                    "failed",
                    ObserveFields {
                        request_id: Some(request_id),
                        latency_ms: latency,
                        error: Some(kind),
                        meta: meta(json!({"checkpoint": true})),
                        ..ObserveFields::default()
                    },
                );
                return;
            }
        };
        self.state().recall_circuit.reset();
        self.observe(
            "recall",
            "succeeded",
            ObserveFields {
                request_id: Some(request_id),
                latency_ms: latency,
                preview: Some(Value::String(
                    results
                        .iter()
                        .map(|result| result.text.as_str())
                        .collect::<Vec<_>>()
                        .join("\n"),
                )),
                meta: meta(json!({"checkpoint": true, "hits": results.len()})),
                ..ObserveFields::default()
            },
        );
        let mut remaining = max_chars;
        let mut bounded = Vec::new();
        for result in &results {
            if remaining == 0 {
                break;
            }
            let text = redact_memory_text(&result.text, remaining)
                .trim()
                .to_string();
            if text.is_empty() {
                continue;
            }
            remaining = remaining.saturating_sub(len16(&text));
            let mut item = Map::new();
            item.insert("text".into(), Value::String(text));
            if let Some(score) = result.score.filter(|score| score.is_finite()) {
                item.insert("score".into(), json!(score));
            }
            bounded.push(Value::Object(item));
        }
        if bounded.is_empty() {
            return;
        }
        let payload = json!({
            "generatedAt": iso_from_ms(self.now() as i64),
            "hostSessionId": host_session_id,
            "query": query,
            "results": bounded,
        });
        let text = format!(
            "{}\n",
            serde_json::to_string_pretty(&payload).unwrap_or_default()
        );
        if std::fs::create_dir_all(&self.storage_dir).is_ok() {
            let _ = write_private_file_atomic(&self.checkpoint_recall_path(), &text);
        }
    }

    /// The memory text for this prompt: a pending checkpoint summary and the
    /// recall, if any. The harness stores it after the prompt in the history.
    fn turn_scoped_memory(&self, extra: Option<String>) -> Option<String> {
        let (summary, max_chars) = {
            let mut state = self.state();
            (
                state.last_compact_summary.take(),
                state.config.recall_max_chars,
            )
        };
        let parts: Vec<String> = summary
            .map(|summary| format_last_compact_injection(&summary, max_chars))
            .into_iter()
            .chain(extra)
            .filter(|part| !part.is_empty())
            .collect();
        (!parts.is_empty()).then(|| parts.join("\n\n"))
    }

    async fn recall_for_prompt(
        &self,
        event: &BeforeRunEvent<'_>,
        request_id: &str,
        started: std::time::Instant,
    ) -> Option<String> {
        let latency = || Some(started.elapsed().as_millis() as u64);
        let succeeded = |meta_value: Value| ObserveFields {
            request_id: Some(request_id.into()),
            latency_ms: latency(),
            meta: meta(meta_value),
            ..ObserveFields::default()
        };
        self.ensure_loaded();
        let Some(client) = self.live_client() else {
            self.observe(
                "recall",
                "succeeded",
                succeeded(json!({"disabled": true, "reason": "unavailable", "outcomeVersion": 1, "outcome": "skipped"})),
            );
            return None;
        };
        let session_id = self.ensure_session_id(event.session_id);
        let (auto_recall, can_attempt, top_k, max_chars, dataset) = {
            let mut state = self.state();
            if state.config.capture_session && !event.prompt.trim().is_empty() {
                let max = state.config.capture_max_chars;
                state.pending_prompt = Some(redact_memory_text(
                    &truncate_for_capture(event.prompt, max),
                    max,
                ));
            }
            (
                state.config.auto_recall,
                state.recall_circuit.can_attempt(wall_ms()),
                state.config.top_k,
                state.config.recall_max_chars,
                state.active_dataset(),
            )
        };
        if !auto_recall || !can_attempt {
            self.observe(
                "recall",
                "succeeded",
                succeeded(json!({"skipped": true, "reason": if !auto_recall { "auto_recall_disabled" } else { "circuit_open" }, "outcomeVersion": 1, "outcome": "skipped"})),
            );
            return self.turn_scoped_memory(None);
        }
        if is_low_signal_recall_prompt(event.prompt) {
            self.observe(
                "recall",
                "succeeded",
                succeeded(json!({"skipped": true, "reason": "short_prompt", "outcomeVersion": 1, "outcome": "skipped"})),
            );
            return self.turn_scoped_memory(None);
        }
        let recall_key = format!("{session_id}\n{}", event.prompt);
        let now = self.now();
        {
            let state = self.state();
            if state.last_recall_key.as_deref() == Some(recall_key.as_str())
                && now.saturating_sub(state.last_recall_at) < RECALL_DEDUP_TTL_MS
            {
                drop(state);
                self.observe(
                    "recall",
                    "succeeded",
                    succeeded(
                        json!({"duplicate": true, "reason": "duplicate", "outcomeVersion": 1, "outcome": "skipped"}),
                    ),
                );
                return self.turn_scoped_memory(None);
            }
        }
        let network_started = std::time::Instant::now();
        let result = client
            .recall(
                event.prompt,
                RecallOptions {
                    top_k: Some(top_k),
                    dataset: Some(dataset.clone()),
                    session_id: Some(session_id.clone()),
                    scope: AUTO_RECALL_SCOPE
                        .iter()
                        .map(|scope| scope.to_string())
                        .collect(),
                    signal: Some(event.signal.clone()),
                    ..RecallOptions::default()
                },
            )
            .await;
        let remote_wait_ms = network_started.elapsed().as_millis() as u64;
        let still_current = {
            let state = self.state();
            state.session_id.as_deref() == Some(session_id.as_str())
                && state.host_session_id.as_deref() == Some(event.session_id)
                && state.active_dataset() == dataset
        };
        if !still_current {
            self.observe(
                "recall",
                "succeeded",
                succeeded(json!({"reason": "stale_session", "outcomeVersion": 1, "outcome": "stale", "remoteWaitMs": remote_wait_ms, "sourceDataset": dataset, "sourceSessionId": session_id, "hostSessionId": event.session_id})),
            );
            return None;
        }
        match result {
            Ok(results) => {
                {
                    let mut state = self.state();
                    state.recall_circuit.reset();
                    state.last_recall_hits = results.len();
                    state.last_recall_key = Some(recall_key);
                    state.last_recall_at = now;
                }
                self.observe(
                    "recall",
                    "succeeded",
                    ObserveFields {
                        request_id: Some(request_id.into()),
                        latency_ms: latency(),
                        preview: Some(Value::String(
                            results
                                .iter()
                                .map(|result| result.text.as_str())
                                .collect::<Vec<_>>()
                                .join("\n"),
                        )),
                        meta: meta(json!({"hits": results.len(), "outcomeVersion": 1, "outcome": if results.is_empty() { "retrieved_empty" } else { "retrieved_nonempty" }, "remoteWaitMs": remote_wait_ms, "sourceDataset": dataset, "sourceSessionId": session_id, "hostSessionId": event.session_id})),
                        ..ObserveFields::default()
                    },
                );
                let extra = (!results.is_empty())
                    .then(|| format_recall(&recall_lines(&results), max_chars));
                self.turn_scoped_memory(extra)
            }
            Err(error) => {
                let kind = error_kind(&error);
                {
                    let mut state = self.state();
                    state.last_error = Some(kind.clone());
                    state.recall_circuit.note_failure(&kind, wall_ms());
                }
                self.observe(
                    "recall",
                    "failed",
                    ObserveFields {
                        request_id: Some(request_id.into()),
                        latency_ms: latency(),
                        error: Some(kind.clone()),
                        meta: meta(json!({"outcomeVersion": 1, "outcome": kind, "remoteWaitMs": remote_wait_ms, "sourceDataset": dataset, "sourceSessionId": session_id, "hostSessionId": event.session_id})),
                        ..ObserveFields::default()
                    },
                );
                self.turn_scoped_memory(None)
            }
        }
    }
}

/// A timed-out or aborted write may have been accepted, so it is not
/// replayed automatically.
fn drain_failure_state(kind: &str) -> PendingState {
    if kind == "timeout" || kind == "aborted" {
        PendingState::Uncertain
    } else {
        PendingState::Pending
    }
}

fn recall_lines(results: &[RecallResult]) -> Vec<RecallLine> {
    results
        .iter()
        .map(|result| RecallLine {
            text: result.text.clone(),
            score: result.score,
        })
        .collect()
}

/// Circuit breakers and observation timestamps use the wall clock, as the
/// TypeScript helpers default to `Date.now()`.
fn wall_ms() -> u64 {
    crate::blackhole::now_ms().max(0) as u64
}

fn chrono_now_ms() -> i64 {
    crate::blackhole::now_ms()
}

#[async_trait::async_trait]
impl HarnessExtension for CogneeRuntime {
    /// Recalled memory goes into the history after the prompt, not into the
    /// system prompt: a system prompt that changed every turn kept the
    /// provider from caching any of the conversation after it.
    async fn prompt_context(&self, event: &BeforeRunEvent<'_>) -> Option<PromptContext> {
        // An unconfigured Cognee leaves no trace: no spinner, no log.
        if !self.is_installed() {
            return None;
        }
        self.state().turn_traces = 0;
        let request_id = uuid::Uuid::new_v4().to_string();
        let started = std::time::Instant::now();
        self.observe(
            "recall",
            "started",
            ObserveFields {
                request_id: Some(request_id.clone()),
                preview: Some(Value::String(event.prompt.into())),
                ..ObserveFields::default()
            },
        );
        let text = self.recall_for_prompt(event, &request_id, started).await?;
        Some(PromptContext {
            custom_type: MEMORY_CUSTOM_TYPE.into(),
            text,
            correlation_id: Some(request_id),
        })
    }

    async fn prompt_context_persisted(&self, context: &PromptContext, persisted: bool) {
        if context.custom_type != MEMORY_CUSTOM_TYPE {
            return;
        }
        let estimated_tokens = if persisted {
            let message = ice_harness::messages::create_custom_message(
                &context.custom_type,
                ice_ai::types::UserContent::Text(context.text.clone()),
                false,
                None,
                0,
            );
            ice_harness::compaction::estimate_tokens(&message)
        } else {
            0
        };
        self.observe(
            "injection",
            if persisted { "succeeded" } else { "failed" },
            ObserveFields {
                request_id: context.correlation_id.clone(),
                meta: meta(json!({
                    "outcomeVersion": 1,
                    "outcome": if persisted { "persisted" } else { "append_failed" },
                    "persistedBytes": if persisted { context.text.len() } else { 0 },
                    "estimatedPersistedTokens": estimated_tokens,
                })),
                ..ObserveFields::default()
            },
        );
    }

    async fn before_compact(&self, event: &BeforeCompactEvent) -> Option<BeforeCompactDecision> {
        self.ensure_loaded();
        let host_id = event.session_id.clone();
        let blackhole_active = is_blackhole_compaction_active(&self.agent_dir);
        if self.live_client().is_none() {
            if blackhole_active {
                self.clear_checkpoint_recall_for_session(&host_id);
            }
            return None;
        }
        let session_id = self.ensure_session_id(&host_id);
        let (mode, capture_max) = {
            let state = self.state();
            (
                state.config.compaction_summary_mode,
                state.config.capture_max_chars,
            )
        };
        if !should_own_compaction_summary(mode) {
            self.maybe_write_checkpoint_recall(event, &session_id, &host_id, blackhole_active)
                .await;
            return None;
        }
        let reason = event.trigger.reason();
        let preparation = &event.preparation;
        let input = AnchorInput {
            previous_summary: preparation.previous_summary.as_deref(),
            messages_to_summarize: &preparation.messages_to_summarize,
            turn_prefix_messages: &preparation.turn_prefix_messages,
            read: preparation.file_ops.read.iter().cloned().collect(),
            edited: preparation.file_ops.edited.iter().cloned().collect(),
            written: preparation.file_ops.written.iter().cloned().collect(),
            tokens_before: Some(preparation.tokens_before),
        };
        let skip_network = event.trigger == CompactionTrigger::Overflow || event.will_retry;
        let summary = if skip_network {
            redact_memory_text(&build_local_precompact_anchor(&input, reason), capture_max)
        } else {
            build_owned_compaction_summary(&input, reason, capture_max)
        };
        Some(BeforeCompactDecision::Replace(CompactionOverride {
            summary,
            retained_tail: None,
            details: Some(json!({
                "engine": "ice-cognee",
                "reason": reason,
                "sessionId": session_id,
                "blackholeActive": blackhole_active,
                "compactionSummaryMode": mode.as_str(),
            })),
        }))
    }

    async fn after_compact(&self, event: &AfterCompactEvent<'_>) {
        self.ensure_loaded();
        if !Self::enabled(&self.state(), self.offline) {
            return;
        }
        let summary = event.entry.summary.trim();
        if summary.is_empty() {
            return;
        }
        let engine = event
            .entry
            .details
            .as_ref()
            .and_then(|details| details.get("engine"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| {
                if event.from_extension {
                    "extension".into()
                } else {
                    "native".into()
                }
            });
        let reason = event.trigger.reason();
        let session_id = self.ensure_session_id(event.session_id);
        let (capture, capture_max, remember, remember_max, dataset, limit) = {
            let mut state = self.state();
            state.last_compact_summary =
                Some(redact_memory_text(summary, state.config.recall_max_chars));
            (
                state.config.capture_session,
                state.config.capture_max_chars,
                state.config.auto_remember,
                state.config.remember_max_chars,
                state.active_dataset(),
                state.config.queue_limit,
            )
        };
        if capture {
            if let Value::Object(entry) = json!({
                "type": "qa",
                "question": format!("Compaction checkpoint ({engine})"),
                "answer": redact_memory_text(summary, capture_max),
                "context": format!("reason={reason}; engine={engine}; session={session_id}"),
            }) {
                self.store_entry(entry, "answer");
            }
        }
        if remember != AutoRememberMode::Compaction {
            return;
        }
        let text = redact_memory_text(
            &[
                "ICE compaction checkpoint".to_string(),
                format!("Session: {session_id}"),
                format!("Reason: {reason}"),
                format!("Engine: {engine}"),
                "Summary:".into(),
                summary.to_string(),
            ]
            .join("\n"),
            remember_max,
        );
        let record = create_pending_remember(&text, &dataset, "agent_actions", self.now());
        let accepted = enqueue_pending_remember(&self.pending_dir(), &record, limit);
        if !matches!(accepted, Ok(true)) {
            self.state().last_error = Some("queue_full".into());
            self.notify(
                NoticeLevel::Warning,
                "Cognee remember queue is full; no summary was dropped",
            );
            return;
        }
        self.append_operation(json!({
            "action": "queue",
            "operationId": record.operation_id,
            "contentHash": record.content_hash,
            "dataset": record.dataset,
            "nodeSet": record.node_set,
            "state": "pending",
            "engine": engine,
        }));
        self.notify(
            NoticeLevel::Info,
            format!("Cognee queued {engine} compaction summary for permanent memory"),
        );
        self.spawn_drain_pending();
    }
}

/// Feeds agent events to the runtime without delaying the loop.
pub struct CogneeEmitter(pub Arc<CogneeRuntime>);

impl ice_agent::AgentEmitter for CogneeEmitter {
    fn emit(&self, event: AgentEvent) -> futures::future::BoxFuture<'static, ()> {
        self.0.on_agent_event(&event);
        Box::pin(async {})
    }

    fn try_emit(&self, event: AgentEvent) {
        // `RunSettled` arrives on the non-blocking path.
        if matches!(event, AgentEvent::RunSettled { .. }) {
            self.0.on_agent_event(&event);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::client::tests::{FakeResponse, FakeTransport};
    use super::super::client::{Body, HttpRequest};
    use super::*;
    use crate::blackhole::live_state::tests::base;
    use ice_agent::types::AgentToolResult;
    use ice_ai::types::{
        Api, AssistantMessage, Content, TextContent, TextContentType, UserMessage,
    };
    use ice_harness::compaction::{CompactionPreparation, FileOperations};
    use ice_harness::session::types::CompactionEntry;
    use ice_harness::types::CompactionSettings;

    const NOW: u64 = 1_800_000_000_000;

    fn runtime(
        dir: &Path,
        transport: &Arc<FakeTransport>,
        config: Option<Value>,
        offline: bool,
    ) -> Arc<CogneeRuntime> {
        if let Some(config) = config {
            let path = dir.join("ice-cognee").join("config.json");
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, config.to_string()).unwrap();
        }
        let notices = NoticeMailbox::new();
        notices.attach();
        CogneeRuntime::new(
            RuntimeOptions {
                agent_dir: dir.to_path_buf(),
                storage_dir: None,
                process_env: CogneeEnv::new(),
                home_dir: None,
                transport: transport.clone(),
                now: Arc::new(|| NOW),
                offline,
                cwd: Some(dir.to_path_buf()),
            },
            notices,
        )
    }

    /// The memory text Cognee adds after `prompt`, checking its message type.
    async fn memory(
        rt: &CogneeRuntime,
        prompt: &str,
        system_prompt: Option<&str>,
    ) -> Option<String> {
        let context = rt.prompt_context(&run_event(prompt, system_prompt)).await?;
        assert_eq!(context.custom_type, MEMORY_CUSTOM_TYPE);
        Some(context.text)
    }

    #[tokio::test]
    async fn injection_observation_distinguishes_persistence_failure() {
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        let rt = runtime(dir.path(), &transport, Some(json!({})), false);
        rt.ensure_loaded();
        let context = PromptContext {
            custom_type: MEMORY_CUSTOM_TYPE.into(),
            text: "évidence".into(),
            correlation_id: Some("recall-id".into()),
        };
        rt.prompt_context_persisted(&context, true).await;
        rt.prompt_context_persisted(&context, false).await;
        rt.wait_for_background().await;
        let observations =
            super::super::observer::read_observations(&dir.path().join("ice-cognee"));
        let mut outcomes: Vec<_> = observations
            .iter()
            .filter(|event| event.operation == "injection")
            .map(|event| {
                assert_eq!(event.request_id.as_deref(), Some("recall-id"));
                let meta = event.meta.as_ref().unwrap();
                (
                    meta["outcome"].as_str().unwrap().to_string(),
                    meta["persistedBytes"].as_u64().unwrap(),
                    meta["estimatedPersistedTokens"].as_i64().unwrap(),
                )
            })
            .collect();
        outcomes.sort();
        assert_eq!(
            outcomes,
            vec![("append_failed".into(), 0, 0), ("persisted".into(), 9, 2)]
        );
    }

    fn run_event<'a>(prompt: &'a str, system_prompt: Option<&'a str>) -> BeforeRunEvent<'a> {
        BeforeRunEvent {
            prompt,
            system_prompt,
            session_id: "host1",
            signal: CancellationToken::new(),
        }
    }

    fn json_body(request: &HttpRequest) -> Value {
        match &request.body {
            Body::Json(text) => serde_json::from_str(text).unwrap(),
            other => panic!("expected JSON, got {other:?}"),
        }
    }

    fn assistant(text: &str) -> AgentMessage {
        let mut message = AssistantMessage::empty(Api::AnthropicMessages, "p", "m", 0);
        message.content = vec![Content::Text(TextContent {
            kind: TextContentType,
            text: text.into(),
            text_signature: None,
        })];
        AgentMessage::Assistant(Box::new(message))
    }

    fn compact_event(trigger: CompactionTrigger) -> BeforeCompactEvent {
        let mut file_ops = FileOperations::default();
        file_ops.read.insert("src/lib.rs".into());
        BeforeCompactEvent {
            preparation: CompactionPreparation {
                messages_to_summarize: vec![AgentMessage::User(UserMessage::new(
                    "Implement the cache layer for the API".to_string(),
                    0,
                ))],
                turn_prefix_messages: Vec::new(),
                retained_tail: Vec::new(),
                is_split_turn: false,
                tokens_before: 1234,
                previous_summary: None,
                file_ops,
                settings: CompactionSettings::default(),
            },
            branch_entries: Vec::new(),
            custom_instructions: None,
            trigger,
            will_retry: false,
            signal: CancellationToken::new(),
            session_id: "host1".into(),
            model: ice_ai::Model::new("m", "m", ice_ai::Api::Faux, "faux", "https://example.test"),
            provider: None,
            providers: Vec::new(),
        }
    }

    #[test]
    fn low_signal_and_checkpoint_query_rules() {
        assert!(is_low_signal_recall_prompt("fix it now"));
        assert!(!is_low_signal_recall_prompt("fix the cache layer now"));
        assert!(is_low_signal_recall_prompt("-- -- -- -- ok"));
        let messages = vec![
            AgentMessage::User(UserMessage::new(
                "explain the cache eviction policy".to_string(),
                0,
            )),
            assistant("sure"),
            AgentMessage::User(UserMessage::new("ok thanks".to_string(), 0)),
        ];
        assert_eq!(
            checkpoint_recall_query(&messages),
            "explain the cache eviction policy"
        );
    }

    /// Regression: listeners were notified from the blocking append task, so
    /// a "succeeded" could reach the TUI panel before its "started" and leave
    /// the row pending forever.
    #[tokio::test]
    async fn observation_listeners_see_phases_in_emission_order() {
        let transport = Arc::new(FakeTransport::default());
        let dir = tempfile::tempdir().unwrap();
        let rt = runtime(dir.path(), &transport, Some(json!({})), false);
        rt.ensure_loaded();
        let request_id = uuid::Uuid::new_v4().to_string();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        let wanted = request_id.clone();
        let _subscription = super::super::observer::subscribe(move |obs| {
            if obs.request_id.as_deref() == Some(wanted.as_str()) {
                sink.lock().unwrap().push(obs.phase.clone());
            }
        });
        for phase in ["started", "succeeded"] {
            rt.observe(
                "recall",
                phase,
                ObserveFields {
                    request_id: Some(request_id.clone()),
                    ..ObserveFields::default()
                },
            );
        }
        assert_eq!(*seen.lock().unwrap(), ["started", "succeeded"]);
    }

    #[tokio::test]
    async fn footer_status_is_only_cognee_or_disabled() {
        let transport = Arc::new(FakeTransport::default());
        let dir = tempfile::tempdir().unwrap();
        let on = runtime(dir.path(), &transport, Some(json!({})), false);
        let status = ice_extensions::ExtensionStatusMailbox::new();
        *on.status.lock().unwrap() = Some(status.clone());
        on.update_status_line();
        assert_eq!(status.get(STATUS_KEY).as_deref(), Some("cognee"));

        let offline_dir = tempfile::tempdir().unwrap();
        let off = runtime(offline_dir.path(), &transport, Some(json!({})), true);
        let status = ice_extensions::ExtensionStatusMailbox::new();
        *off.status.lock().unwrap() = Some(status.clone());
        off.update_status_line();
        assert_eq!(status.get(STATUS_KEY).as_deref(), Some("disabled"));
    }

    #[tokio::test]
    async fn uninstalled_or_offline_cognee_makes_no_requests() {
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        let rt = runtime(dir.path(), &transport, None, false);
        assert!(!rt.is_installed());
        assert_eq!(
            memory(&rt, "how does the cache layer work", Some("base")).await,
            None
        );
        rt.wait_for_background().await;
        assert!(!dir.path().join("ice-cognee").exists());
        let offline = tempfile::tempdir().unwrap();
        let rt = runtime(offline.path(), &transport, Some(json!({})), true);
        assert!(rt.is_installed());
        assert!(!rt.is_enabled());
        assert_eq!(
            memory(&rt, "how does the cache layer work", None).await,
            None
        );
        rt.wait_for_background().await;
        assert!(transport.sent().is_empty());
    }

    #[tokio::test]
    async fn manual_improve_requires_cognee_enabled_but_not_auto_improve() {
        let disabled_dir = tempfile::tempdir().unwrap();
        let disabled_transport = Arc::new(FakeTransport::default());
        let disabled = runtime(
            disabled_dir.path(),
            &disabled_transport,
            Some(json!({"enabled": false})),
            false,
        );
        disabled.ensure_session_id("host1");
        disabled.manual_improve().await;
        assert!(disabled_transport.sent().is_empty());

        let enabled_dir = tempfile::tempdir().unwrap();
        let enabled_transport = Arc::new(FakeTransport::default());
        enabled_transport.push(FakeResponse::json(200, json!({})));
        let enabled = runtime(
            enabled_dir.path(),
            &enabled_transport,
            Some(json!({"enabled": true, "autoImprove": false})),
            false,
        );
        enabled.ensure_session_id("host1");
        assert!(enabled.is_enabled());
        assert!(!enabled.config().auto_improve);
        enabled.manual_improve().await;
        enabled.wait_for_background().await;
        let requests = enabled_transport.sent();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].url.ends_with("/api/v1/improve"));
        assert_eq!(json_body(&requests[0])["sessionIds"], json!(["ice_host1"]));
        let observations =
            super::super::observer::read_observations(&enabled_dir.path().join("ice-cognee"));
        let accepted = observations
            .iter()
            .find(|event| event.operation == "improve" && event.phase == "succeeded")
            .unwrap();
        assert_eq!(accepted.meta.as_ref().unwrap()["outcome"], "accepted");
    }

    #[tokio::test]
    async fn prompt_recall_injects_memory_and_deduplicates() {
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        transport.push(FakeResponse::json(
            200,
            json!({"results": [{"text": "Cache uses LRU eviction", "score": 0.9}]}),
        ));
        let rt = runtime(dir.path(), &transport, Some(json!({})), false);
        let prompt = "How does the cache layer evict entries?";
        let injected = memory(&rt, prompt, Some("base")).await.unwrap();
        assert!(
            injected.starts_with("<ice-void-cognee-memory>\n"),
            "{injected}"
        );
        assert!(injected.contains("1. Cache uses LRU eviction (score 0.9)"));
        // Exact duplicate within 15 s: no request, nothing injected.
        assert_eq!(memory(&rt, prompt, Some("base")).await, None);
        // Low-signal prompt: no request.
        assert_eq!(memory(&rt, "fix it", None).await, None);
        rt.wait_for_background().await;
        let sent = transport.sent();
        assert_eq!(sent.len(), 1);
        let body = json_body(&sent[0]);
        assert_eq!(body["sessionId"], "ice_host1");
        // Each project gets its own dataset by default.
        assert!(
            rt.active_dataset().starts_with("ice-"),
            "{}",
            rt.active_dataset()
        );
        assert_eq!(body["datasets"], json!([rt.active_dataset()]));
        assert_eq!(
            body["scope"],
            json!(["session", "graph"]),
            "raw traces are not recalled"
        );
        let observations =
            super::super::observer::read_observations(&dir.path().join("ice-cognee"));
        let mut outcomes: Vec<_> = observations
            .iter()
            .filter(|event| event.operation == "recall" && event.phase == "succeeded")
            .map(|event| event.meta.as_ref().unwrap()["outcome"].as_str().unwrap())
            .collect();
        // Persistence runs on blocking threads and need not preserve emission order.
        outcomes.sort_unstable();
        assert_eq!(outcomes, ["retrieved_nonempty", "skipped", "skipped"]);
        let mut reasons: Vec<_> = observations
            .iter()
            .filter_map(|event| event.meta.as_ref()?.get("reason")?.as_str())
            .collect();
        reasons.sort_unstable();
        assert_eq!(reasons, ["duplicate", "short_prompt"]);
        let retrieved = observations
            .iter()
            .find(|event| {
                event
                    .meta
                    .as_ref()
                    .and_then(|m| m.get("outcome"))
                    .and_then(Value::as_str)
                    == Some("retrieved_nonempty")
            })
            .unwrap();
        let metadata = retrieved.meta.as_ref().unwrap();
        assert_eq!(metadata["sourceSessionId"], "ice_host1");
        assert_eq!(metadata["hostSessionId"], "host1");
        assert_eq!(metadata["sourceDataset"], rt.active_dataset());
        assert!(metadata["remoteWaitMs"].as_u64().is_some());
    }

    #[tokio::test]
    async fn late_recall_does_not_enter_a_new_session() {
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        transport.push(FakeResponse::Ok {
            status: 200,
            headers: vec![],
            chunks: vec![json!({"results": [{"text": "old session fact"}]})
                .to_string()
                .into_bytes()],
            delay_ms: 50,
        });
        let rt = runtime(dir.path(), &transport, Some(json!({})), false);
        let pending_rt = rt.clone();
        let task = tokio::spawn(async move {
            memory(&pending_rt, "find the previous cache decision", None).await
        });
        while transport.sent().is_empty() {
            tokio::task::yield_now().await;
        }
        rt.ensure_session_id("host2");
        assert_eq!(task.await.unwrap(), None);
        rt.wait_for_background().await;
        let observations =
            super::super::observer::read_observations(&dir.path().join("ice-cognee"));
        assert!(observations.iter().any(|event| event
            .meta
            .as_ref()
            .and_then(|m| m.get("outcome"))
            .and_then(Value::as_str)
            == Some("stale")));
    }

    #[tokio::test]
    async fn doctor_reports_unverified_contract_without_endpoint_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        let rt = runtime(dir.path(), &transport, Some(json!({})), true);
        let mut report = rt.collect_doctor().await;
        report.base_url = "http://user:secret@localhost:8211/path?token=secret".into();
        let doctor = report.doctor_line();
        assert!(doctor.contains("nativeRecall=POST:/api/v1/recall (support unverified;"));
        assert!(doctor.contains("sends query, topK, onlyContext, scope, sessionId, datasets"));
        assert!(doctor.contains("sends datasetName, sessionIds, runInBackground"));
        assert!(doctor.contains("completion status unsupported"));
        assert!(doctor.contains("mcpCognee=separate catalog (not probed)"));
        assert!(!doctor.contains("secret"));
        assert!(!report.status_line().contains("secret"));
        assert!(transport.sent().is_empty());
    }

    #[tokio::test]
    async fn search_tool_propagates_unavailable_and_transport_errors() {
        use ice_agent::agent_tool::AgentTool;

        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        let rt = runtime(dir.path(), &transport, Some(json!({})), false);
        let tool = super::super::command::CogneeSearchTool::new(Arc::downgrade(&rt));
        let execute = || {
            tool.execute(
                "call",
                json!({"query": "cache"}),
                CancellationToken::new(),
                Arc::new(|_| {}),
            )
        };
        let error = execute().await.unwrap_err();
        assert!(error.to_string().contains("Cognee search failed"));
        assert_eq!(transport.sent().len(), 1);

        transport.push(FakeResponse::json(200, json!({"results": []})));
        assert!(execute().await.is_ok());

        transport.push(FakeResponse::json(404, json!({"detail": "Not Found"})));
        assert!(execute()
            .await
            .unwrap_err()
            .to_string()
            .contains("not_found"));
        let cancelled = CancellationToken::new();
        cancelled.cancel();
        let error = tool
            .execute(
                "call",
                json!({"query": "cache"}),
                cancelled,
                Arc::new(|_| {}),
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("aborted"));

        let requests_before_offline = transport.sent().len();
        let offline_dir = tempfile::tempdir().unwrap();
        let offline = runtime(offline_dir.path(), &transport, Some(json!({})), true);
        let offline_tool = super::super::command::CogneeSearchTool::new(Arc::downgrade(&offline));
        let error = offline_tool
            .execute(
                "call",
                json!({"query": "cache"}),
                CancellationToken::new(),
                Arc::new(|_| {}),
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("disabled"));
        assert_eq!(transport.sent().len(), requests_before_offline);
    }

    #[tokio::test]
    async fn search_tool_reports_timeout_as_error() {
        use ice_agent::agent_tool::AgentTool;

        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        transport.push(FakeResponse::Ok {
            status: 200,
            headers: vec![],
            chunks: vec![json!({"results": []}).to_string().into_bytes()],
            delay_ms: 500,
        });
        let rt = runtime(
            dir.path(),
            &transport,
            Some(json!({"recallBudgetMs": 100})),
            false,
        );
        let tool = super::super::command::CogneeSearchTool::new(Arc::downgrade(&rt));
        let error = tool
            .execute(
                "call",
                json!({"query": "cache"}),
                CancellationToken::new(),
                Arc::new(|_| {}),
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("timeout"));
        assert_eq!(transport.sent().len(), 1);
    }

    #[tokio::test]
    async fn recall_failures_open_the_circuit() {
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        let rt = runtime(dir.path(), &transport, Some(json!({})), false);
        for index in 0..4 {
            let prompt = format!("question number {index} about the cache");
            assert_eq!(memory(&rt, &prompt, None).await, None);
        }
        // Three failures open the breaker; the fourth prompt sends nothing.
        assert_eq!(transport.sent().len(), 3);
        let report = rt.collect_doctor().await;
        assert!(report.recall_circuit_open);
        assert_eq!(report.last_error.as_deref(), Some("unreachable"));
        rt.wait_for_background().await;
        let observations =
            super::super::observer::read_observations(&dir.path().join("ice-cognee"));
        assert!(observations.iter().any(|event| {
            event.operation == "recall"
                && event.meta.as_ref().and_then(|m| m.get("reason")) == Some(&json!("circuit_open"))
        }));
    }

    /// Traces record actions, not contents: a successful read keeps its path
    /// but not the file, a command keeps a short result, and an error keeps
    /// its message.
    #[tokio::test]
    async fn traces_keep_actions_and_errors_but_not_file_contents() {
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        let rt = runtime(dir.path(), &transport, Some(json!({})), false);
        memory(&rt, "hi there", None).await;
        let long = "line of output ".repeat(200);
        let calls = [
            (
                "c1",
                "read",
                json!({"path": "/skills/x/SKILL.md"}),
                "SKILL BODY",
                false,
            ),
            (
                "c2",
                "bash",
                json!({"command": "cargo test"}),
                long.as_str(),
                false,
            ),
            (
                "c3",
                "read",
                json!({"path": "/missing"}),
                "ENOENT: no such file",
                true,
            ),
        ];
        for (id, tool, args, output, is_error) in calls {
            transport.push(FakeResponse::json(200, json!({"entry_id": id})));
            rt.on_agent_event(&AgentEvent::ToolExecutionStart {
                tool_call_id: id.into(),
                tool_name: tool.into(),
                args,
            });
            rt.on_agent_event(&AgentEvent::ToolExecutionEnd {
                tool_call_id: id.into(),
                tool_name: tool.into(),
                result: AgentToolResult::text(output),
                is_error,
            });
        }
        rt.wait_for_background().await;
        let traces: Vec<Value> = transport
            .sent()
            .iter()
            .filter(|request| request.url.ends_with("/api/v1/remember/entry"))
            .map(json_body)
            .filter(|body| body["entry"]["type"] == "trace")
            .map(|body| body["entry"].clone())
            .collect();
        assert_eq!(traces.len(), 3);
        assert_eq!(traces[0]["method_params"]["path"], "/skills/x/SKILL.md");
        assert_eq!(traces[0]["method_return_value"], "");
        let command = traces[1]["method_return_value"].as_str().unwrap();
        assert!(command.starts_with("line of output"), "{command}");
        assert!(
            command.chars().count() < TRACE_RESULT_CHARS + 20,
            "{command}"
        );
        assert_eq!(traces[2]["status"], "error");
        assert_eq!(traces[2]["method_return_value"], "ENOENT: no such file");
    }

    #[tokio::test]
    async fn tool_traces_and_answers_are_captured_with_dream_revisions() {
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        let rt = runtime(dir.path(), &transport, Some(json!({})), false);
        memory(&rt, "hi there", None).await;
        transport.push(FakeResponse::json(200, json!({"entry_id": "t"})));
        transport.push(FakeResponse::json(200, json!({"entry_id": "a"})));
        rt.on_agent_event(&AgentEvent::ToolExecutionStart {
            tool_call_id: "c1".into(),
            tool_name: "bash".into(),
            args: json!({"command": "ls", "token": "secret=1"}),
        });
        rt.on_agent_event(&AgentEvent::ToolExecutionEnd {
            tool_call_id: "c1".into(),
            tool_name: "bash".into(),
            result: AgentToolResult::text("out"),
            is_error: false,
        });
        // Not on the allowlist.
        rt.on_agent_event(&AgentEvent::ToolExecutionEnd {
            tool_call_id: "c2".into(),
            tool_name: "docs".into(),
            result: AgentToolResult::text("x"),
            is_error: false,
        });
        rt.on_agent_event(&AgentEvent::AgentEnd {
            messages: vec![assistant("Done answer")],
        });
        rt.wait_for_background().await;
        let entries: Vec<Value> = transport
            .sent()
            .iter()
            .filter(|request| request.url.ends_with("/api/v1/remember/entry"))
            .map(json_body)
            .collect();
        assert_eq!(entries.len(), 2);
        let trace = entries
            .iter()
            .find(|body| body["entry"]["type"] == "trace")
            .unwrap();
        assert_eq!(trace["entry"]["origin_function"], "bash");
        assert_eq!(trace["entry"]["method_params"]["command"], "ls");
        assert_eq!(
            trace["entry"]["method_params"]["token"],
            "secret=[REDACTED]"
        );
        assert_eq!(trace["entry"]["method_return_value"], "out");
        let qa = entries
            .iter()
            .find(|body| body["entry"]["type"] == "qa")
            .unwrap();
        assert_eq!(qa["entry"]["question"], "hi there");
        assert_eq!(qa["entry"]["answer"], "Done answer");
        assert_eq!(qa["session_id"], "ice_host1");
        let saves = rt.state().saves;
        assert_eq!(
            saves,
            Saves {
                prompt: 1,
                trace: 1,
                answer: 1
            }
        );
        let record =
            read_dream_record_for_dataset(&rt.dream_dir(), "ice_host1", &rt.active_dataset())
                .unwrap();
        assert_eq!(record.capture_revision, 2.0);
        assert!(rt.state().dream_dirty);
    }

    #[tokio::test]
    async fn failed_capture_is_buffered_and_replayed() {
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        let rt = runtime(dir.path(), &transport, Some(json!({})), false);
        memory(&rt, "hi there", None).await;
        rt.on_agent_event(&AgentEvent::AgentEnd {
            messages: vec![assistant("Answer")],
        });
        rt.wait_for_background().await;
        assert_eq!(read_warmup_batch(&rt.warmup_dir()).len(), 1);
        transport.push(FakeResponse::json(200, json!({})));
        rt.drain_warmup();
        rt.wait_for_background().await;
        assert!(read_warmup_batch(&rt.warmup_dir()).is_empty());
    }

    #[tokio::test]
    async fn compaction_checkpoint_is_queued_remembered_and_injected_once() {
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        transport.push(FakeResponse::json(200, json!({})));
        transport.push(FakeResponse::json(200, json!({})));
        let rt = runtime(dir.path(), &transport, Some(json!({})), false);
        let entry = CompactionEntry {
            base: base("compaction", 1),
            summary: "Checkpoint summary".into(),
            retained_tail: Vec::new(),
            tokens_before: 10,
            details: None,
            usage: None,
        };
        rt.after_compact(&AfterCompactEvent {
            entry: &entry,
            trigger: CompactionTrigger::Threshold,
            from_extension: false,
            session_id: "host1",
        })
        .await;
        rt.wait_for_background().await;
        let sent = transport.sent();
        let remember = sent
            .iter()
            .find(|request| request.url.ends_with("/api/v1/remember"))
            .unwrap();
        let Body::Multipart(parts) = &remember.body else {
            panic!("remember is multipart")
        };
        assert_eq!(parts[1].value, "agent_actions");
        assert_eq!(
            parts.last().unwrap().value,
            "ICE compaction checkpoint\nSession: ice_host1\nReason: threshold\nEngine: native\nSummary:\nCheckpoint summary"
        );
        assert!(read_pending_remember(&rt.pending_dir()).is_empty());
        let injected = memory(&rt, "ok", Some("base")).await.unwrap();
        assert!(injected.contains("<ice-void-last-compact>"));
        assert!(injected.contains("Checkpoint summary"));
        assert_eq!(memory(&rt, "ok", Some("base")).await, None);
    }

    #[tokio::test]
    async fn uncertain_records_need_an_explicit_flush() {
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        let rt = runtime(dir.path(), &transport, Some(json!({})), false);
        rt.ensure_loaded();
        let record = create_pending_remember("fact", "ice", "user_context", 1);
        enqueue_pending_remember(&rt.pending_dir(), &record, 64).unwrap();
        update_pending_remember_state(
            &rt.pending_dir(),
            &record.operation_id,
            PendingState::Uncertain,
        )
        .unwrap();
        // Normal drains skip uncertain records; `flush uncertain` sends them.
        rt.drain_pending(false).await;
        assert!(transport.sent().is_empty());
        transport.push(FakeResponse::json(200, json!({})));
        super::super::command::run_command(&rt, "flush uncertain").await;
        assert_eq!(transport.sent().len(), 1);
        assert!(read_pending_remember(&rt.pending_dir()).is_empty());
        assert!(rt
            .notices()
            .drain()
            .iter()
            .any(|(_, text)| text == "Cognee uncertain queue flushed"));
        assert_eq!(drain_failure_state("timeout"), PendingState::Uncertain);
        assert_eq!(drain_failure_state("aborted"), PendingState::Uncertain);
        assert_eq!(drain_failure_state("server_error"), PendingState::Pending);
    }

    #[tokio::test]
    async fn own_mode_returns_a_local_anchor_and_auto_writes_checkpoint_recall() {
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        let rt = runtime(
            dir.path(),
            &transport,
            Some(json!({"compactionSummaryMode": "own"})),
            false,
        );
        let Some(BeforeCompactDecision::Replace(own)) = rt
            .before_compact(&compact_event(CompactionTrigger::Threshold))
            .await
        else {
            panic!("own mode replaces the summary")
        };
        assert_eq!(
            own.summary,
            "Pre-compact local anchor (threshold)\nmessagesToSummarize=1\nturnPrefix=0\ntokensBefore≈1234\nread: src/lib.rs\nImplement the cache layer for the API"
        );
        assert_eq!(own.details.as_ref().unwrap()["engine"], "ice-cognee");
        let Some(BeforeCompactDecision::Replace(overflow)) = rt
            .before_compact(&compact_event(CompactionTrigger::Overflow))
            .await
        else {
            panic!("own mode replaces the summary")
        };
        assert!(!overflow.summary.contains("Implement the cache"));

        let auto = tempfile::tempdir().unwrap();
        let blackhole = crate::blackhole::config::config_path(auto.path());
        std::fs::create_dir_all(blackhole.parent().unwrap()).unwrap();
        std::fs::write(&blackhole, "{}").unwrap();
        transport.push(FakeResponse::json(
            200,
            json!({"results": [{"text": "The API cache uses LRU eviction", "score": 0.5}]}),
        ));
        let rt = runtime(
            auto.path(),
            &transport,
            Some(json!({"checkpointRecall": true})),
            false,
        );
        assert!(rt
            .before_compact(&compact_event(CompactionTrigger::Threshold))
            .await
            .is_none());
        let written: Value =
            serde_json::from_str(&std::fs::read_to_string(rt.checkpoint_recall_path()).unwrap())
                .unwrap();
        assert_eq!(written["hostSessionId"], "host1");
        assert_eq!(written["query"], "Implement the cache layer for the API");
        let section = crate::blackhole::checkpoint_recall::consume_checkpoint_recall_section(
            auto.path(),
            NOW as i64,
            Some("host1"),
            "",
        );
        assert!(section.contains("The API cache uses LRU eviction"));
        assert!(!rt.checkpoint_recall_path().exists());
    }

    #[tokio::test]
    async fn commands_toggle_policy_and_remember_with_queue_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        let rt = runtime(dir.path(), &transport, None, false);
        assert!(!rt.is_enabled());
        super::super::command::run_command(&rt, "on").await;
        assert!(rt.is_enabled());
        super::super::command::run_command(&rt, "capture off").await;
        assert!(!rt.config().capture_session);
        super::super::command::run_command(&rt, "compact own").await;
        assert_eq!(
            rt.config().compaction_summary_mode,
            CompactionSummaryMode::Own
        );
        super::super::command::run_command(&rt, "remember on extra").await;
        // `remember on extra` is literal text, so it was sent (and failed).
        let sent = transport.sent();
        assert_eq!(sent.len(), 1);
        let Body::Multipart(parts) = &sent[0].body else {
            panic!("multipart")
        };
        assert_eq!(parts[1].value, "user_context");
        assert_eq!(parts.last().unwrap().value, "on extra");
        assert_eq!(read_pending_remember(&rt.pending_dir()).len(), 1);
        super::super::command::run_command(&rt, "bogus").await;
        let notices = rt.notices().drain();
        assert!(notices
            .iter()
            .any(|(_, text)| text == "Cognee session capture: off"));
        assert!(notices
            .iter()
            .any(|(_, text)| text == "Cognee unavailable (unreachable); memory queued"));
        assert_eq!(
            notices.last().unwrap(),
            &(
                NoticeLevel::Warning,
                super::super::command::USAGE.to_string()
            )
        );
    }

    #[tokio::test]
    async fn shutdown_improves_then_unregisters() {
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        let rt = runtime(dir.path(), &transport, Some(json!({})), false);
        transport.push(FakeResponse::json(200, json!({"status": "ok"}))); // health
        transport.push(FakeResponse::json(200, json!({}))); // register
        transport.push(FakeResponse::json(200, json!({}))); // settings
        rt.session_start_for("host1").await;
        rt.wait_for_background().await;
        assert!(rt.state().agent_registered);
        transport.push(FakeResponse::json(202, json!({}))); // improve
        transport.push(FakeResponse::json(200, json!({}))); // unregister
        rt.session_shutdown().await;
        let urls: Vec<String> = transport.sent().iter().map(|r| r.url.clone()).collect();
        let improve = urls
            .iter()
            .position(|u| u.ends_with("/api/v1/improve"))
            .unwrap();
        let unregister = urls
            .iter()
            .position(|u| u.ends_with("/api/v1/agents/unregister"))
            .unwrap();
        assert!(improve < unregister);
        assert!(rt.state().shutting_down);
    }
}
