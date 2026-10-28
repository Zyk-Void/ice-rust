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

#[path = "coordinator.rs"]
mod coordinator;
#[path = "lookup.rs"]
mod lookup;
#[path = "request.rs"]
mod request;
#[path = "validation.rs"]
mod validation;

use coordinator::EvidenceCache;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use sha2::{Digest, Sha256};

use ice_agent::{AgentEvent, AgentMessage};
use ice_harness::extension::{
    AfterCompactEvent, BeforeCompactDecision, BeforeCompactEvent, BeforeRunEvent,
    CompactionOverride, CompactionTrigger, HarnessExtension, PromptContext,
};
use serde_json::{json, Map, Value};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

#[cfg(test)]
use super::candidates::Category;
use super::candidates::{Candidate, CandidateState, Role};
use super::client::{
    CallOptions, ClientConfig, CogneeClient, CogneeError, CogneeTransport,
    OperationOutcomesSnapshot, RecallOptions, RecallResult, ReqwestTransport,
};
use super::config::{
    find_git_root, load_connection, process_env, repository_root, resolve_runtime_dataset,
    save_config, write_private_file_atomic, AutoRememberMode, CogneeConfig, CogneeEnv,
    CompactionSummaryMode, ConnectionInputs, KeySource, ReadPolicy, WritePolicy,
    PROJECT_DATASET_SENTINEL,
};
use super::dream::{
    claim_dream_candidate, mark_dream_accepted, mark_dream_captured, mark_dream_improved,
    read_dream_candidates, read_dream_record_for_dataset, release_dream_claim, DreamClaim,
};
use super::evidence::{
    select_new_with_context, FilterContext, RejectCounts, SeenEvidence, Selection,
};
use super::helper::{
    self, AbstainReason, Candidate as HelperCandidate, CandidateBatch, Helper, HelperOutcome,
    Verdict,
};
use super::helper_gate::{Admission as HelperAdmission, HelperGate};
use super::observer::{persist_observation, publish_observation, sanitize_endpoint, Observation};
use super::policy::{admit_prompt, Admission};
#[cfg(test)]
use super::queue::read_warmup_batch;
use super::queue::{
    buffer_warmup_entry, create_pending_remember, enqueue_pending_remember, read_pending_remember,
    recover_inflight, remove_pending_remember, select_drain_batch, update_pending_remember_state,
    CircuitState, DispatchLease, PendingState,
};
#[cfg(test)]
use super::suppress::Suppression;
use super::suppress::SuppressionSet;
use super::text::{
    extract_message_text, fit_lines, format_bounded_last_compact_injection,
    format_last_compact_injection, format_recall, redact_memory_text, truncate_for_capture,
    truncate_value_for_capture, RecallLine,
};
use crate::blackhole::js::{iso_from_ms, len16, prefix16};
use crate::builtin_extensions::{NoticeLevel, NoticeMailbox};

pub const STATUS_KEY: &str = "ice-cognee";
pub const SESSION_ENTRY_TYPE: &str = "ice-cognee";
pub const SEARCH_TOOL_NAME: &str = "cognee_search";
pub const PROPOSE_TOOL_NAME: &str = "cognee_propose_memory";
const RECALL_MIN_WORDS: usize = 4;
/// Non-legacy automatic memory is capped at 800 estimated tokens. BeforeRunEvent
/// has no model-window information, so a model-window-relative cap is unavailable.
const AUTO_INJECT_TOKEN_CAP: usize = 800;
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

fn read_helper_candidates_bounded(directory: &Path, limit: usize) -> Vec<Candidate> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .take(limit)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
        .filter_map(|text| serde_json::from_str::<Candidate>(&text).ok())
        .collect()
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
    pub operation_outcomes: OperationOutcomesSnapshot,
    pub capability_inventory: Value,
    pub capture_session: bool,
    pub capture_tools: bool,
    pub auto_recall: bool,
    pub read_policy: ReadPolicy,
    pub write_policy: WritePolicy,
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
    pub helper_status: String,
    pub helper_requests: u64,
    pub helper_abstains: u64,
    pub helper_skips: u64,
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
            "Cognee {}; health {}; recall {} ({}); remember {}; write {}; capture {}; tools {}; improve {}; compact={}{}{}; endpoint {}; dataset {}; session {}; key {}; saves p/t/a={}/{}/{}; lastRecall={}; queue pending={} uncertain={}; breaker recall={} remember={}; lastError={}",
            on_off(self.enabled),
            self.health(),
            on_off(self.auto_recall),
            self.read_policy.as_str(),
            self.auto_remember.as_str(),
            self.write_policy.as_str(),
            on_off(self.capture_session),
            on_off(self.capture_tools),
            on_off(self.auto_improve),
            self.compaction_summary_mode.as_str(),
            if self.blackhole_active {
                "+blackhole"
            } else {
                ""
            },
            if self.own_compaction_summary {
                "(own-summary)"
            } else {
                "(defer-summary)"
            },
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
            format!(
                "readPolicy={} writePolicy={}",
                self.read_policy.as_str(),
                self.write_policy.as_str()
            ),
            "nativeRecall=POST:/api/v1/recall (support unverified; sends query, topK, onlyContext, scope, sessionId, datasets)".to_string(),
            "nativeRemember=POST:/api/v1/remember (multipart datasetName, node_set, run_in_background, optional session_id, data; support unverified)".to_string(),
            "nativeRememberEntry=POST:/api/v1/remember/entry (JSON entry, dataset_name, session_id; support unverified)".to_string(),
            "nativeImprove=POST:/api/v1/improve (sends datasetName, sessionIds, runInBackground; background acceptance only; completion status: not exposed by this adapter)".to_string(),
            format!("mcpCognee=separate catalog (not probed); inventory={}", self.capability_inventory),
            format!("operations={}", self.operation_outcomes),
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
            format!("helper={} requests={} abstains={} skips={}", self.helper_status, self.helper_requests, self.helper_abstains, self.helper_skips),
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

pub(crate) struct State {
    pub(crate) config: CogneeConfig,
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
    /// Recall lines already in the active context (non-legacy read policies).
    seen_evidence: SeenEvidence,
    suppressions: SuppressionSet,
    evidence_cache: EvidenceCache,
    resolved_dataset: String,
    context_generation: u64,
    legacy_dataset: Option<String>,
    improve_in_flight: bool,
    improve_backoff: HashMap<(String, String), (f64, u64, u32)>,
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
    helper: Option<Arc<Helper>>,
    helper_error: Option<String>,
    helper_signature: Option<(bool, WritePolicy, bool, Option<String>, Option<String>)>,
    helper_cancel: CancellationToken,
    automatic_write_cancel: CancellationToken,
    helper_gate: HelperGate,
    helper_requests: u64,
    helper_abstains: u64,
    helper_skips: u64,
    candidate_revision: u64,
    helper_scheduled_revision: u64,
}

impl State {
    fn new() -> Self {
        let config = CogneeConfig::default();
        Self {
            resolved_dataset: config.dataset.clone(),
            context_generation: 0,
            legacy_dataset: None,
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
            seen_evidence: SeenEvidence::default(),
            suppressions: SuppressionSet::default(),
            evidence_cache: EvidenceCache::default(),
            improve_in_flight: false,
            improve_backoff: HashMap::new(),
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
            helper: None,
            helper_error: None,
            helper_signature: None,
            helper_cancel: CancellationToken::new(),
            automatic_write_cancel: CancellationToken::new(),
            helper_gate: HelperGate::default(),
            helper_requests: 0,
            helper_abstains: 0,
            helper_skips: 0,
            candidate_revision: 0,
            helper_scheduled_revision: 0,
        }
    }

    /// Resolve `$project` for `cwd` and, for a linked worktree, the dataset
    /// it used before sharing its repository's. An explicit dataset has none.
    fn refresh_dataset(&mut self, cwd: Option<&Path>, storage_dir: &Path) {
        let previous = self.active_dataset();
        self.resolved_dataset = resolve_runtime_dataset(&self.config.dataset, cwd);
        self.suppressions =
            SuppressionSet::load(&storage_dir.join("suppress"), &self.active_dataset());
        self.suppressions
            .include_receipts(&storage_dir.join("candidates"), &self.active_dataset());
        if previous != self.active_dataset() {
            self.context_generation = self.context_generation.saturating_add(1);
            self.helper_cancel.cancel();
            self.helper_cancel = CancellationToken::new();
            self.evidence_cache.clear();
        }
        self.legacy_dataset = (self.config.dataset == PROJECT_DATASET_SENTINEL)
            .then(|| {
                let cwd = cwd
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
                super::config::legacy_project_dataset(&cwd)
            })
            .filter(|legacy| *legacy != self.resolved_dataset);
    }

    /// The one extra dataset an empty automatic recall may try.
    fn fallback_dataset(&self) -> Option<String> {
        (self.config.read_policy != ReadPolicy::Legacy)
            .then(|| self.legacy_dataset.clone())
            .flatten()
    }

    pub(crate) fn active_dataset(&self) -> String {
        if self.resolved_dataset.is_empty() {
            self.config.dataset.clone()
        } else {
            self.resolved_dataset.clone()
        }
    }

    fn operation_context(&self) -> coordinator::OperationContext {
        coordinator::OperationContext {
            host_session_id: self.host_session_id.clone(),
            session_id: self.session_id.clone(),
            dataset: self.active_dataset(),
            generation: self.context_generation,
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

struct HelperFlightGuard {
    runtime: Weak<CogneeRuntime>,
    key: String,
    flight_id: u64,
    finished: bool,
    _permit: Option<super::helper_gate::BudgetPermit>,
}

impl HelperFlightGuard {
    fn finish(&mut self, outcome: &HelperOutcome) {
        if let Some(runtime) = self.runtime.upgrade() {
            runtime
                .state()
                .helper_gate
                .finish(self.flight_id, &self.key, runtime.now(), outcome);
        }
        self._permit.take();
        self.finished = true;
    }
}

impl Drop for HelperFlightGuard {
    fn drop(&mut self) {
        if !self.finished {
            if let Some(runtime) = self.runtime.upgrade() {
                runtime.state().helper_gate.finish(
                    self.flight_id,
                    &self.key,
                    runtime.now(),
                    &HelperOutcome::Abstained(AbstainReason::Cancelled),
                );
            }
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
    capability_service: Mutex<Weak<crate::capability_service::CapabilityService>>,
    state: Mutex<State>,
    read_coordinator: Mutex<request::ReadCoordinator>,
    background: Mutex<Vec<JoinHandle<()>>>,
    helper_tasks: Mutex<Vec<JoinHandle<()>>>,
    settle: Mutex<Option<JoinHandle<()>>>,
    tool_args: Mutex<HashMap<String, Value>>,
    tools_allowed: Mutex<Option<bool>>,
    /// Dream ledger writes run one at a time.
    dream_writes: tokio::sync::Mutex<()>,
    /// Single-flight pending drain.
    drain: tokio::sync::Mutex<()>,
    /// Single-flight warmup replay.
    warmup_drain: tokio::sync::Mutex<()>,
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
            capability_service: Mutex::new(Weak::new()),
            state: Mutex::new(State::new()),
            read_coordinator: Mutex::new(request::ReadCoordinator::default()),
            background: Mutex::new(Vec::new()),
            helper_tasks: Mutex::new(Vec::new()),
            settle: Mutex::new(None),
            tool_args: Mutex::new(HashMap::new()),
            tools_allowed: Mutex::new(None),
            dream_writes: tokio::sync::Mutex::new(()),
            drain: tokio::sync::Mutex::new(()),
            warmup_drain: tokio::sync::Mutex::new(()),
        })
    }

    /// Share the host's resident catalog without probing MCP or retaining a cycle.
    pub fn bind_capability_service(
        &self,
        service: &Arc<crate::capability_service::CapabilityService>,
    ) {
        *self.capability_service.lock().unwrap() = Arc::downgrade(service);
    }

    fn capability_inventory(&self) -> Value {
        let service = self.capability_service.lock().unwrap().upgrade();
        service.map_or_else(
            || json!({"tools":[], "probed":false, "status":"catalog_unavailable"}),
            |service| service.doctor_cognee_contract(),
        )
    }

    fn arc(&self) -> Arc<Self> {
        self.me.upgrade().expect("Cognee runtime is alive")
    }

    pub(crate) fn now(&self) -> u64 {
        (self.now)()
    }

    pub(crate) fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }

    pub fn storage_dir(&self) -> &Path {
        &self.storage_dir
    }

    fn config_path(&self) -> PathBuf {
        self.storage_dir.join("config.json")
    }

    pub(crate) fn pending_dir(&self) -> PathBuf {
        self.storage_dir.join("pending")
    }

    pub(crate) fn candidates_dir(&self) -> PathBuf {
        self.storage_dir.join("candidates")
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
        if let Some(live) = harness.upgrade() {
            self.ensure_session_id(&live.session().storage().metadata().id);
        }
        *self.harness.lock().unwrap() = Some(harness);
        *self.status.lock().unwrap() = Some(status);
        *self.cwd.lock().unwrap() = Some(cwd);
        self.refresh_repository_identity();
    }

    pub(crate) fn harness(&self) -> Option<ice_harness::agent_harness::AgentHarness> {
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
        state.refresh_dataset(cwd.as_deref(), &self.storage_dir);
        state.api_key = connection.api_key;
        state.key_source = connection.key_source;
        Self::rebuild_helper(&mut state);
        let helper_error = state.helper_error.clone();
        state.client = Some(CogneeClient::new(
            Self::client_config(&state),
            self.transport.clone(),
        ));
        state.loaded = true;
        drop(state);
        if let Some(error) = helper_error {
            self.notify(
                NoticeLevel::Warning,
                format!("Cognee helper binding failed: {error}"),
            );
            self.observe_helper(None, "skipped", "bind_failed", 0, 0, 0, None);
        }
    }

    fn rebuild_client(state: &mut State, transport: Arc<dyn CogneeTransport>) {
        state.client = Some(CogneeClient::new(Self::client_config(state), transport));
    }

    fn rebuild_helper(state: &mut State) {
        let signature = (
            state.config.enabled,
            state.config.write_policy,
            state.config.helper_enabled,
            state.config.helper_provider.clone(),
            state.config.helper_model.clone(),
        );
        if state.helper_signature.as_ref() == Some(&signature) {
            return;
        }
        state.helper_signature = Some(signature);
        state.helper_cancel.cancel();
        state.helper_cancel = CancellationToken::new();
        state.helper_error = None;
        let should_bind = state.config.enabled
            && state.config.selective_writes()
            && state.config.helper_enabled
            && state.config.helper_provider.is_some()
            && state.config.helper_model.is_some();
        if !should_bind {
            state.helper = None;
            return;
        }
        state.helper = match Helper::bind(&state.config) {
            Ok(helper) => helper.map(Arc::new),
            Err(error) => {
                state.helper_error = Some(match error {
                    helper::HelperError::UnsupportedProtocol => "unsupported_protocol".into(),
                    _ => "binding_failed".into(),
                });
                None
            }
        };
    }

    /// The client when Cognee may contact the server.
    pub(crate) fn live_client(&self) -> Option<CogneeClient> {
        let state = self.state();
        if Self::enabled(&state, self.offline) {
            state.client.clone()
        } else {
            None
        }
    }

    fn persist_backend_acceptance(
        &self,
        id: &str,
        operation: &str,
        session: Option<&str>,
        dataset: &str,
        receipt: &super::client::AcceptedReceipt,
    ) -> bool {
        let record = super::accepted_jobs::AcceptedJob {
            version: 1,
            operation_id: id.to_owned(),
            operation: operation.to_owned(),
            dataset: dataset.to_owned(),
            session_id: session.map(str::to_owned),
            accepted_at_ms: self.now(),
            receipt: receipt.clone(),
            searchability: super::accepted_jobs::Searchability::Unverified,
        };
        if super::accepted_jobs::persist(&self.storage_dir.join("accepted-jobs"), &record).is_err()
        {
            self.notify(NoticeLevel::Warning, "Cognee accepted the request but its local acknowledgment could not be retained; searchability remains unverified");
            false
        } else {
            true
        }
    }

    fn automatic_capture_allowed(&self, kind: &str) -> bool {
        let state = self.state();
        !state.shutting_down
            && Self::enabled(&state, self.offline)
            && if kind == "trace" {
                state.config.captures_tools()
            } else {
                state.config.captures_session()
            }
    }

    fn automatic_improve_allowed(&self) -> bool {
        let state = self.state();
        Self::enabled(&state, self.offline) && state.config.improves()
    }

    fn ensure_session_id(&self, host_id: &str) -> String {
        let id = super::config::cognee_session_id(host_id);
        let mut state = self.state();
        if state.session_id.as_deref() != Some(id.as_str())
            || state.host_session_id.as_deref() != Some(host_id)
        {
            state.context_generation = state.context_generation.saturating_add(1);
            state.dream_dirty = false;
            state.seen_evidence.clear();
        }
        if state.session_id.as_deref() != Some(id.as_str())
            || state.host_session_id.as_deref() != Some(host_id)
        {
            state.helper_cancel.cancel();
            state.helper_cancel = CancellationToken::new();
        }
        state.session_id = Some(id.clone());
        state.host_session_id = Some(host_id.into());
        id
    }

    // ── background work ──

    fn track_helper(&self, work: impl std::future::Future<Output = ()> + Send + 'static) {
        let handle = tokio::spawn(work);
        let mut tasks = self.helper_tasks.lock().unwrap();
        tasks.retain(|task| !task.is_finished());
        tasks.push(handle);
    }

    fn observe_helper(
        &self,
        request_id: Option<&str>,
        outcome: &str,
        reason: &str,
        candidates: usize,
        accepted: usize,
        rejected: usize,
        latency_ms: Option<u64>,
    ) {
        self.observe_helper_for(
            request_id, outcome, reason, candidates, accepted, rejected, latency_ms, None,
        );
    }

    fn observe_helper_for(
        &self,
        request_id: Option<&str>,
        outcome: &str,
        reason: &str,
        candidates: usize,
        accepted: usize,
        rejected: usize,
        latency_ms: Option<u64>,
        identity: Option<(&str, &str)>,
    ) {
        // A helper invocation is not proof of an HTTP send. Keep attempt counts
        // separate from unknown wire counts; skips and cached results are local.
        let invoked = latency_ms.is_some() && reason != "batch_cached";
        self.observe_for(
            "helper",
            outcome,
            ObserveFields {
                request_id: Some(
                    request_id
                        .map(str::to_owned)
                        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                ),
                latency_ms,
                meta: meta(json!({
                    "reason": reason,
                    "candidates": candidates,
                    "accepted": accepted,
                    "rejected": rejected,
                    "requested": if invoked { candidates } else { 0 },
                    "invocationCount": usize::from(invoked),
                    "networkRequestCount": Value::Null,
                    "candidateCount": candidates,
                    "judged": if outcome == "judged" { candidates } else { 0 },
                    "abstained": if outcome == "abstained" { candidates } else { 0 },
                    "cached": reason == "batch_cached",
                    "busy": reason == "busy",
                    "rateLimited": reason == "rate_limited",
                    "stale": reason == "stale",
                    "applicationFailed": reason == "application_failed",
                    "persisted": accepted,
                    "outcomeVersion": 3
                })),
                ..ObserveFields::default()
            },
            identity,
        );
    }

    fn notify_helper_candidates_changed(&self) {
        {
            let mut state = self.state();
            state.candidate_revision = state.candidate_revision.saturating_add(1);
        }
        self.schedule_helper();
    }

    fn schedule_helper(&self) {
        self.ensure_loaded();
        let (eligible, helper, cancel, dataset, provider, endpoint, shutting_down) = {
            let state = self.state();
            (
                state.config.selective_writes()
                    && state.config.enabled
                    && Self::enabled(&state, self.offline),
                state.helper.clone(),
                state.helper_cancel.child_token(),
                state.active_dataset(),
                state.config.helper_provider.clone().unwrap_or_default(),
                state.config.base_url.clone(),
                state.shutting_down,
            )
        };
        if shutting_down || !eligible {
            return;
        }
        let Some(helper) = helper else {
            if let Some(error) = self.state().helper_error.clone() {
                self.notify(
                    NoticeLevel::Warning,
                    format!("Cognee helper binding failed: {error}"),
                );
                self.observe_helper(None, "skipped", "bind_failed", 0, 0, 0, None);
            }
            return;
        };
        {
            let mut state = self.state();
            if state.helper_gate.flight_id().is_some()
                || state.candidate_revision == state.helper_scheduled_revision
            {
                return;
            }
            state.helper_scheduled_revision = state.candidate_revision;
        };
        let candidates_dir = self.candidates_dir();
        let receipt_candidates: std::collections::HashSet<_> =
            super::admission::read_receipts(&super::admission::receipt_dir(&candidates_dir))
                .into_iter()
                .map(|receipt| receipt.candidate.id)
                .collect();
        let now = self.now();
        let mut pending = Vec::new();
        for mut candidate in read_helper_candidates_bounded(&candidates_dir, 32) {
            if receipt_candidates.contains(&candidate.id)
                || candidate.dataset != dataset
                || candidate.state != CandidateState::Pending
            {
                continue;
            }
            if now.saturating_sub(candidate.created_at) >= super::candidates::CANDIDATE_TTL_MS
                || candidate.valid_until_ms.is_some_and(|expiry| expiry <= now)
            {
                let _ = super::candidates::set_state(
                    &candidates_dir,
                    &mut candidate,
                    CandidateState::Expired,
                );
            } else {
                pending.push(candidate);
            }
        }
        let (helper, cancel, dataset, provider, endpoint) =
            (helper, cancel, dataset, provider, endpoint);
        let pending = pending
            .into_iter()
            .filter(|candidate| candidate.dataset == dataset)
            .collect::<Vec<_>>();
        if pending.is_empty() {
            self.observe_helper(None, "skipped", "no_pending", 0, 0, 0, None);
            return;
        }
        let this = self.arc();
        let operation_id = uuid::Uuid::new_v4().to_string();
        let work_operation_id = operation_id.clone();
        let source_identity = (
            self.state().session_id.clone().unwrap_or_default(),
            dataset.clone(),
        );
        let work = async move {
            let sources = match this.candidate_source_snapshot(&cancel).await {
                Ok(sources) => sources,
                Err(_) => {
                    this.observe_helper_for(
                        Some(&work_operation_id),
                        "skipped",
                        "no_pending",
                        0,
                        0,
                        0,
                        None,
                        Some((&source_identity.0, &source_identity.1)),
                    );
                    return;
                }
            };
            if sources.task_id.is_none() {
                this.observe_helper_for(
                    Some(&work_operation_id),
                    "skipped",
                    "no_pending",
                    0,
                    0,
                    0,
                    None,
                    Some((&source_identity.0, &source_identity.1)),
                );
                return;
            }
            let mut candidates = Vec::new();
            for mut candidate in pending {
                if this.now().saturating_sub(candidate.created_at)
                    >= super::candidates::CANDIDATE_TTL_MS
                    || candidate
                        .valid_until_ms
                        .is_some_and(|expiry| expiry <= this.now())
                {
                    let _ = super::candidates::set_state(
                        &candidates_dir,
                        &mut candidate,
                        CandidateState::Expired,
                    );
                    continue;
                }
                let evidence = match super::source_validation::validate_source(
                    &candidate,
                    &sources.messages,
                ) {
                    Ok(evidence) => evidence,
                    Err(reason) => {
                        let _ = super::candidates::set_state(
                            &candidates_dir,
                            &mut candidate,
                            CandidateState::Rejected(reason.as_str().into()),
                        );
                        continue;
                    }
                };
                if validation::validate_candidate_scope(&candidate, &sources, &evidence, true)
                    .is_err()
                {
                    let _ = super::candidates::set_state(
                        &candidates_dir,
                        &mut candidate,
                        CandidateState::Rejected("stale_source_scope".into()),
                    );
                    continue;
                }
                candidates.push(candidate);
                if candidates.len() == 8 {
                    break;
                }
            }
            if candidates.is_empty() {
                this.observe_helper_for(
                    Some(&work_operation_id),
                    "skipped",
                    "no_pending",
                    0,
                    0,
                    0,
                    None,
                    Some((&source_identity.0, &source_identity.1)),
                );
                return;
            }
            let helper_candidates = candidates
                .iter()
                .map(|candidate| HelperCandidate {
                    id: candidate.id.clone(),
                    text: candidate.text.clone(),
                    kind_hint: Some(candidate.category.as_str().to_owned()),
                })
                .collect::<Vec<_>>();
            let source_excerpt = candidates
                .iter()
                .filter_map(|candidate| {
                    sources
                        .messages
                        .iter()
                        .find(|(id, _)| id == &candidate.source.entry_id)
                        .map(|(_, message)| extract_message_text(message))
                })
                .collect::<Vec<_>>()
                .join("\n");
            let excerpt = source_excerpt.chars().take(800).collect::<String>();
            let batch = match CandidateBatch::new(helper_candidates, Some(excerpt)) {
                Ok(batch) => batch,
                Err(_) => {
                    this.observe_helper_for(
                        Some(&work_operation_id),
                        "abstained",
                        "malformed",
                        candidates.len(),
                        0,
                        0,
                        None,
                        Some((&source_identity.0, &source_identity.1)),
                    );
                    return;
                }
            };
            let mut ordered_candidates = candidates.iter().collect::<Vec<_>>();
            ordered_candidates.sort_by(|left, right| left.id.cmp(&right.id));
            let mut hasher = Sha256::new();
            hasher.update(provider.as_bytes());
            let origin = super::observer::sanitize_endpoint(&endpoint);
            hasher.update(Sha256::digest(origin.as_bytes()));
            let (model, dataset, generation) = {
                let state = this.state();
                (
                    state
                        .config
                        .helper_model
                        .as_deref()
                        .unwrap_or_default()
                        .to_owned(),
                    state.active_dataset(),
                    state.context_generation,
                )
            };
            hasher.update(model.as_bytes());
            hasher.update(dataset.as_bytes());
            hasher.update(generation.to_le_bytes());
            for candidate in ordered_candidates {
                hasher.update(candidate.id.as_bytes());
                hasher.update(Sha256::digest(candidate.text.as_bytes()));
                hasher.update(candidate.source.entry_id.as_bytes());
                hasher.update(candidate.category.as_str().as_bytes());
                hasher.update(candidate.dataset.as_bytes());
                hasher.update(candidate.branch.as_deref().unwrap_or_default().as_bytes());
                hasher.update(
                    candidate
                        .supersedes
                        .as_deref()
                        .unwrap_or_default()
                        .as_bytes(),
                );
                hasher.update(candidate.valid_until_ms.unwrap_or_default().to_le_bytes());
            }
            let key = hasher
                .finalize()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            let (admission, permit, flight_id) = {
                let mut state = this.state();
                // The Cognee dataset endpoint is not the decision provider.
                // Changing it must not grant a fresh provider quota.
                let provider_key = Sha256::digest(provider.as_bytes())
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>();
                let (admission, permit) = state.helper_gate.admit(&key, &provider_key, this.now());
                (admission, permit, state.helper_gate.flight_id())
            };
            let HelperAdmission::Start = admission else {
                if admission.reason() == Some(super::helper_gate::SkipReason::BatchCached) {
                    let cached = this.state().helper_gate.cached(&key, this.now());
                    if let Some(HelperOutcome::Judged(judgments)) = cached {
                        let mut accepted = 0;
                        let mut rejected = 0;
                        for mut candidate in candidates {
                            let Some(judgment) = judgments
                                .iter()
                                .find(|judgment| judgment.id == candidate.id)
                            else {
                                continue;
                            };
                            if judgment.durable == Verdict::No {
                                if super::candidates::set_state(
                                    &candidates_dir,
                                    &mut candidate,
                                    CandidateState::Rejected("helper_not_durable".into()),
                                )
                                .is_ok()
                                {
                                    rejected += 1;
                                }
                            } else if judgment.durable == Verdict::Yes
                                && judgment.applicable == Verdict::Yes
                                && judgment.category != Some(helper::Category::None)
                            {
                                if this
                                    .accept_validated_candidate(&candidate, &cancel, true)
                                    .await
                                    .map(|outcome| outcome.is_some())
                                    .unwrap_or(false)
                                {
                                    accepted += 1;
                                }
                            }
                        }
                        this.observe_helper_for(
                            Some(&work_operation_id),
                            "judged",
                            "batch_cached",
                            accepted + rejected,
                            accepted,
                            rejected,
                            None,
                            Some((&source_identity.0, &source_identity.1)),
                        );
                    } else {
                        this.observe_helper_for(
                            Some(&work_operation_id),
                            "skipped",
                            "batch_cached",
                            candidates.len(),
                            0,
                            0,
                            None,
                            Some((&source_identity.0, &source_identity.1)),
                        );
                    }
                } else if let Some(reason) = admission.reason() {
                    if reason != super::helper_gate::SkipReason::BatchCached {
                        let mut state = this.state();
                        state.helper_skips = state.helper_skips.saturating_add(1);
                    }
                    this.observe_helper_for(
                        Some(&work_operation_id),
                        "skipped",
                        reason.as_str(),
                        candidates.len(),
                        0,
                        0,
                        None,
                        Some((&source_identity.0, &source_identity.1)),
                    );
                }
                return;
            };
            let mut guard = HelperFlightGuard {
                runtime: Arc::downgrade(&this),
                key: key.clone(),
                flight_id: flight_id.expect("started helper flight has an id"),
                finished: false,
                _permit: permit,
            };
            {
                let mut state = this.state();
                state.helper_requests = state.helper_requests.saturating_add(1);
            }
            let started = std::time::Instant::now();
            let outcome = helper.judge(&batch, cancel.clone()).await;
            let latency = started.elapsed().as_millis() as u64;
            if cancel.is_cancelled() {
                guard.finish(&HelperOutcome::Abstained(AbstainReason::Cancelled));
                this.observe_helper_for(
                    Some(&work_operation_id),
                    "skipped",
                    "cancelled",
                    candidates.len(),
                    0,
                    0,
                    Some(latency),
                    Some((&source_identity.0, &source_identity.1)),
                );
                return;
            }
            let judgments = match outcome {
                HelperOutcome::Judged(judgments) => {
                    guard.finish(&HelperOutcome::Judged(judgments.clone()));
                    judgments
                }
                HelperOutcome::Abstained(AbstainReason::Cancelled) if cancel.is_cancelled() => {
                    guard.finish(&HelperOutcome::Abstained(AbstainReason::Cancelled));
                    this.observe_helper_for(
                        Some(&work_operation_id),
                        "skipped",
                        "cancelled",
                        candidates.len(),
                        0,
                        0,
                        Some(latency),
                        Some((&source_identity.0, &source_identity.1)),
                    );
                    return;
                }
                HelperOutcome::Abstained(reason) => {
                    guard.finish(&HelperOutcome::Abstained(reason));
                    let label = match reason {
                        AbstainReason::Timeout => "timeout",
                        AbstainReason::Cancelled => "cancelled",
                        AbstainReason::Unavailable => "unavailable",
                        AbstainReason::Malformed | AbstainReason::TooLarge => "malformed",
                        AbstainReason::UnknownCandidateId => "unknown_candidate_id",
                        AbstainReason::Disabled => "unavailable",
                    };
                    {
                        let mut state = this.state();
                        state.helper_abstains = state.helper_abstains.saturating_add(1);
                    }
                    this.observe_helper_for(
                        Some(&work_operation_id),
                        "abstained",
                        label,
                        candidates.len(),
                        0,
                        0,
                        Some(latency),
                        Some((&source_identity.0, &source_identity.1)),
                    );
                    return;
                }
            };
            let mut accepted = 0;
            let mut rejected = 0;
            for mut candidate in candidates {
                if candidate.state != CandidateState::Pending {
                    continue;
                }
                let Some(judgment) = judgments
                    .iter()
                    .find(|judgment| judgment.id == candidate.id)
                else {
                    continue;
                };
                if judgment.durable == Verdict::No {
                    if super::candidates::set_state(
                        &candidates_dir,
                        &mut candidate,
                        CandidateState::Rejected("helper_not_durable".into()),
                    )
                    .is_ok()
                    {
                        rejected += 1;
                    }
                } else if judgment.durable == Verdict::Yes
                    && judgment.applicable == Verdict::Yes
                    && judgment.category != Some(helper::Category::None)
                {
                    match this
                        .accept_validated_candidate(&candidate, &cancel, true)
                        .await
                    {
                        Ok(Some(_)) => {
                            accepted += 1;
                        }
                        Ok(None) => {
                            if super::candidates::set_state(
                                &candidates_dir,
                                &mut candidate,
                                CandidateState::Rejected(
                                    super::candidates::Reason::QueueFull.as_str().into(),
                                ),
                            )
                            .is_ok()
                            {
                                rejected += 1;
                            }
                        }
                        Err(_) => {}
                    }
                }
            }
            this.observe_helper_for(
                Some(&work_operation_id),
                "judged",
                "judged",
                accepted + rejected,
                accepted,
                rejected,
                Some(latency),
                Some((&source_identity.0, &source_identity.1)),
            );
        };
        self.track_helper(work);
    }

    #[cfg(test)]
    fn use_helper_for_test(&self, helper: Arc<Helper>) {
        let mut state = self.state();
        state.config.write_policy = WritePolicy::Selective;
        state.config.enabled = true;
        state.installed = true;
        state.helper = Some(helper);
        state.helper_error = None;
    }

    #[cfg(test)]
    fn store_pending_candidate_for_test(&self, candidate: &Candidate) {
        super::candidates::store_pending(&self.candidates_dir(), candidate, self.now()).unwrap();
    }

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
            let helper_handles: Vec<JoinHandle<()>> =
                std::mem::take(&mut *self.helper_tasks.lock().unwrap());
            if handles.is_empty() && helper_handles.is_empty() {
                break;
            }
            for handle in handles.into_iter().chain(helper_handles) {
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

    pub(crate) fn observe_candidate_with_supersession(
        &self,
        candidate: &Candidate,
        outcome: &str,
        supersession: Option<&str>,
        supersession_reason: Option<&str>,
    ) {
        let reason = match &candidate.state {
            CandidateState::Rejected(reason) => Some(reason.clone()),
            _ => None,
        };
        let mut metadata = json!({
            "category": candidate.category.as_str(),
            "outcome": outcome,
            "reason": reason,
            "sourceRole": match candidate.source.role {
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::Tool => "tool",
            },
            "supersedes": candidate.supersedes,
            "outcomeVersion": 1
        });
        if let Some(status) = supersession {
            metadata["supersession"] = json!(status);
            if let Some(reason) = supersession_reason {
                metadata["supersessionReason"] = json!(reason);
            }
        }
        self.observe(
            "candidate",
            outcome,
            ObserveFields {
                request_id: Some(candidate.id.clone()),
                meta: meta(metadata),
                ..ObserveFields::default()
            },
        );
    }

    pub(crate) fn refresh_receipt_suppressions(&self) {
        let dataset = self.active_dataset();
        let mut suppressions = SuppressionSet::load(&self.storage_dir.join("suppress"), &dataset);
        suppressions.include_receipts(&self.candidates_dir(), &dataset);
        self.state().suppressions = suppressions;
    }
    #[cfg(test)]
    pub(crate) fn record_suppression(&self, suppression: Suppression) -> Result<(), String> {
        let directory = self.storage_dir.join("suppress");
        super::suppress::record(&directory, &suppression)
            .map_err(|error| format!("Could not record Cognee suppression: {error}"))?;
        self.state().suppressions.insert(suppression);
        Ok(())
    }

    fn observe(&self, operation: &str, phase: &str, fields: ObserveFields) {
        self.observe_for(operation, phase, fields, None);
    }

    fn observe_for(
        &self,
        operation: &str,
        phase: &str,
        fields: ObserveFields,
        identity: Option<(&str, &str)>,
    ) {
        let (session_id, dataset, base_url, model) = {
            let state = self.state();
            if !state.installed {
                return;
            }
            (
                match identity {
                    Some((session, _)) => (!session.is_empty()).then(|| session.to_owned()),
                    None => state.session_id.clone(),
                },
                identity
                    .map(|(_, dataset)| dataset.to_owned())
                    .unwrap_or_else(|| state.active_dataset()),
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
        let (client, session_id, dataset, signal) = {
            let state = self.state();
            if state.shutting_down
                || !Self::enabled(&state, self.offline)
                || !(if kind == "trace" {
                    state.config.captures_tools()
                } else {
                    state.config.captures_session()
                })
            {
                return None;
            }
            (
                state.client.clone()?,
                state.session_id.clone()?,
                state.active_dataset(),
                state.automatic_write_cancel.child_token(),
            )
        };
        let request_id = uuid::Uuid::new_v4().to_string();
        let started = std::time::Instant::now();
        let operation = if kind == "trace" {
            "trace"
        } else {
            "remember_entry"
        };
        let this = self.arc();
        self.track(async move {
            if signal.is_cancelled() || !this.automatic_capture_allowed(kind) {
                return;
            }
            this.observe_for(
                operation,
                "started",
                ObserveFields {
                    request_id: Some(request_id.clone()),
                    preview: Some(Value::Object(entry.clone())),
                    meta: meta(json!({"kind": kind})),
                    ..ObserveFields::default()
                },
                Some((&session_id, &dataset)),
            );
            let entry_value = Value::Object(entry.clone());
            let result = client
                .remember_entry(
                    &entry_value,
                    Some(&dataset),
                    &session_id,
                    CallOptions {
                        signal: Some(signal),
                        ..CallOptions::default()
                    },
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
                    this.observe_for(
                        operation,
                        "succeeded",
                        ObserveFields {
                            request_id: Some(request_id),
                            latency_ms: latency,
                            meta: meta(json!({"kind": kind})),
                            ..ObserveFields::default()
                        },
                        Some((&session_id, &dataset)),
                    );
                }
                Err(error) => {
                    let kind_name = error_kind(&error);
                    this.state().last_error = Some(kind_name.clone());
                    this.observe_for(
                        operation,
                        "failed",
                        ObserveFields {
                            request_id: Some(request_id),
                            latency_ms: latency,
                            error: Some(kind_name),
                            meta: meta(json!({"kind": kind})),
                            ..ObserveFields::default()
                        },
                        Some((&session_id, &dataset)),
                    );
                    if drain_failure_state(&error_kind(&error)) == PendingState::Uncertain {
                        super::queue::buffer_warmup_entry_with_state(
                            &this.warmup_dir(),
                            &session_id,
                            &dataset,
                            &entry,
                            this.now(),
                            PendingState::Uncertain,
                        );
                    } else {
                        this.buffer_warmup_for(&session_id, &dataset, &entry);
                    }
                }
            }
        });
        Some(())
    }

    fn buffer_warmup_for(&self, session_id: &str, dataset: &str, entry: &Map<String, Value>) {
        buffer_warmup_entry(&self.warmup_dir(), session_id, dataset, entry, self.now());
    }

    /// Replay up to 16 buffered cache writes.
    fn drain_warmup(&self) {
        let Some(client) = self.live_client() else {
            return;
        };
        let (dir, dataset) = (self.warmup_dir(), self.state().active_dataset());
        let signal = self.state().automatic_write_cancel.child_token();
        let this = self.arc();
        self.track(async move {
            let Ok(_guard) = this.warmup_drain.try_lock() else {
                return;
            };
            let lease_dir = dir.clone();
            let Ok(Some(_lease)) =
                tokio::task::spawn_blocking(move || DispatchLease::try_acquire(&lease_dir))
                    .await
                    .unwrap_or(Ok(None))
            else {
                return;
            };
            let batch = tokio::task::spawn_blocking(move || {
                super::queue::read_replayable_warmup_batch(&dir)
            })
            .await
            .unwrap_or_default();
            let (mut replayed, mut failed) = (0, 0);
            for record in batch {
                if signal.is_cancelled() || !this.automatic_capture_allowed("answer") {
                    break;
                }
                if record.entry.get("type").and_then(Value::as_str) == Some("trace")
                    && !this.automatic_capture_allowed("trace")
                {
                    // Preserve traces that their original capture policy no
                    // longer authorizes; other eligible answers may proceed.
                    continue;
                }
                let entry = Value::Object(record.entry.clone());
                let dataset = record.dataset.clone().unwrap_or_else(|| dataset.clone());
                if super::queue::set_warmup_state(&record, PendingState::InFlight).is_err() {
                    failed += 1;
                    continue;
                }
                let result = client
                    .remember_entry(
                        &entry,
                        Some(&dataset),
                        &record.session_id,
                        CallOptions {
                            signal: Some(signal.clone()),
                            ..CallOptions::default()
                        },
                    )
                    .await;
                match result {
                    Ok(_) => {
                        if std::fs::remove_file(&record.path).is_ok() {
                            replayed += 1;
                        } else {
                            failed += 1;
                        } // keep InFlight if local acknowledgment fails
                    }
                    Err(error) => {
                        let state = drain_failure_state(&error_kind(&error));
                        let _ = super::queue::set_warmup_state(&record, state);
                        failed += 1;
                    }
                }
            }
            this.observe(
                "warmup_replay",
                "completed",
                ObserveFields {
                    meta: meta(json!({"replayed": replayed, "failed": failed})),
                    ..ObserveFields::default()
                },
            );
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
        let dispatch_session = self.state().session_id.clone().unwrap_or_default();
        let dir = self.pending_dir();
        let lease_dir = dir.clone();
        let Ok(Some(_dispatch_lease)) =
            tokio::task::spawn_blocking(move || DispatchLease::try_acquire(&lease_dir))
                .await
                .unwrap_or(Ok(None))
        else {
            return;
        };
        let scan_dir = dir.clone();
        let records =
            tokio::task::spawn_blocking(move || select_drain_batch(&scan_dir, allow_uncertain))
                .await
                .unwrap_or_default();
        for record in records {
            // Preserve durable authorized intents while disabled, but do not
            // dispatch them until Cognee is enabled again.
            if self.live_client().is_none() {
                return;
            }
            if record.admission_id.is_some()
                && super::admission::is_completed(
                    &super::admission::receipt_dir(&self.candidates_dir()),
                    &record.operation_id,
                )
            {
                let _ = remove_pending_remember(&dir, &record.operation_id);
                continue;
            }
            if !self.state().remember_circuit.can_attempt(wall_ms()) {
                return;
            }
            let request_id = uuid::Uuid::new_v4().to_string();
            let started = std::time::Instant::now();
            let state_name = match record.state {
                PendingState::Pending => "pending",
                PendingState::Uncertain => "uncertain",
                PendingState::InFlight => "inflight",
                PendingState::Prepared => continue,
            };
            if update_pending_remember_state(&dir, &record.operation_id, PendingState::InFlight)
                .is_err()
            {
                continue;
            }
            self.observe_for(
                "remember",
                "started",
                ObserveFields {
                    request_id: Some(request_id.clone()),
                    preview: Some(Value::String(record.text.clone())),
                    meta: meta(json!({"operationId": record.operation_id, "state": state_name})),
                    ..ObserveFields::default()
                },
                Some((&dispatch_session, &record.dataset)),
            );
            let sent = client
                .remember_with_receipt(
                    &record.text,
                    &record.node_set,
                    Some(&record.dataset),
                    None,
                    CallOptions::default(),
                )
                .await;
            let latency = Some(started.elapsed().as_millis() as u64);
            let outcome = match sent {
                Ok(receipt) => {
                    if !self.persist_backend_acceptance(
                        &request_id,
                        "remember",
                        (!dispatch_session.is_empty()).then_some(dispatch_session.as_str()),
                        &record.dataset,
                        &receipt,
                    ) {
                        Err("unknown".to_string())
                    } else if record.admission_id.is_some()
                        && super::admission::mark_completed(
                            &super::admission::receipt_dir(&self.candidates_dir()),
                            &record.operation_id,
                        )
                        .is_err()
                    {
                        // The remote accepted the write, but local acknowledgement
                        // could not be made durable. Keep it ambiguous; never resend.
                        Err("unknown".to_string())
                    } else {
                        remove_pending_remember(&dir, &record.operation_id)
                            .map_err(|_| "unknown".to_string())
                    }
                }
                Err(error) => Err(error_kind(&error)),
            };
            match outcome {
                Ok(()) => {
                    self.state().remember_circuit.reset();
                    self.observe_for(
                        "remember",
                        "succeeded",
                        ObserveFields {
                            request_id: Some(request_id),
                            latency_ms: latency,
                            meta: meta(json!({"operationId": record.operation_id})),
                            ..ObserveFields::default()
                        },
                        Some((&dispatch_session, &record.dataset)),
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
                    self.observe_for(
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
                        Some((&dispatch_session, &record.dataset)),
                    );
                    let open = {
                        let mut state = self.state();
                        state.last_error = Some(kind.clone());
                        state.remember_circuit.note_failure(&kind, wall_ms());
                        !state.remember_circuit.can_attempt(wall_ms())
                    };
                    let failure_state = if record.admission_id.is_some() && kind == "unknown" {
                        PendingState::Uncertain
                    } else {
                        drain_failure_state(&kind)
                    };
                    let ambiguous = failure_state == PendingState::Uncertain;
                    let _ =
                        update_pending_remember_state(&dir, &record.operation_id, failure_state);
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

    pub(crate) fn spawn_drain_pending(&self) {
        let this = self.arc();
        self.track(async move { this.drain_pending(false).await });
    }

    // ── improvement ──

    fn suppress_improve(
        &self,
        session: &str,
        dataset: &str,
        revision: f64,
        accepted: Option<f64>,
    ) -> bool {
        let now = self.now();
        let mut state = self.state();
        let key = (session.to_string(), dataset.to_string());
        if state
            .improve_backoff
            .get(&key)
            .is_some_and(|entry| entry.0 != revision)
        {
            state.improve_backoff.remove(&key);
        }
        let suppression = if accepted.is_some_and(|value| value >= revision) {
            Some(("already_accepted_revision", None))
        } else {
            state
                .improve_backoff
                .get(&key)
                .and_then(|entry| (entry.1 > now).then_some(("backoff", Some(entry.1 - now))))
        };
        drop(state);
        if let Some((reason, retry)) = suppression {
            let mut value = json!({"reason": reason, "outcomeVersion": 1, "outcome": "skipped", "target": session});
            if let Some(retry) = retry {
                value["retryAfterMs"] = json!(retry);
            }
            self.observe(
                "improve",
                "skipped",
                ObserveFields {
                    meta: meta(value),
                    ..ObserveFields::default()
                },
            );
            true
        } else {
            false
        }
    }

    fn improve_result(&self, session: &str, dataset: &str, revision: f64, accepted: bool) {
        let now = self.now();
        let mut state = self.state();
        let key = (session.to_string(), dataset.to_string());
        if accepted {
            state.improve_backoff.remove(&key);
        } else {
            let failures = state
                .improve_backoff
                .get(&key)
                .filter(|entry| entry.0 == revision)
                .map_or(1, |entry| entry.2.saturating_add(1));
            let delay = if failures == 1 { 60_000 } else { 300_000 };
            state
                .improve_backoff
                .insert(key, (revision, now.saturating_add(delay), failures));
        }
    }

    /// `runImprove`: one current-session improve, claim-fenced.
    pub fn run_improve(&self, reason: &'static str) -> Option<JoinHandle<()>> {
        let now = self.now();
        let (client, session_id, dataset, signal) = {
            let mut state = self.state();
            if !Self::enabled(&state, self.offline)
                || !state.config.improves()
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
            (
                client,
                session_id,
                state.active_dataset(),
                state.automatic_write_cancel.child_token(),
            )
        };
        let this = self.arc();
        let handle = tokio::spawn(async move {
            this.improve_current_guarded(client, session_id, dataset, reason, signal)
                .await;
            let mut state = this.state();
            state.improve_in_flight = false;
            state.last_improve_at = this.now();
        });
        Some(handle)
    }

    #[cfg(test)]
    async fn improve_current(
        &self,
        client: CogneeClient,
        session_id: String,
        dataset: String,
        reason: &'static str,
    ) {
        let signal = self.state().automatic_write_cancel.child_token();
        self.improve_current_guarded(client, session_id, dataset, reason, signal)
            .await;
    }

    async fn improve_current_guarded(
        &self,
        client: CogneeClient,
        session_id: String,
        dataset: String,
        reason: &'static str,
        signal: CancellationToken,
    ) {
        drop(self.dream_writes.lock().await);
        let token = uuid::Uuid::new_v4().to_string();
        let dir = self.dream_dir();
        let claim_now = self.now() as f64;
        let snapshot = read_dream_record_for_dataset(&dir, &session_id, &dataset);
        let revision = snapshot
            .as_ref()
            .map_or(0.0, |record| record.capture_revision);
        if self.suppress_improve(
            &session_id,
            &dataset,
            revision,
            snapshot
                .as_ref()
                .and_then(|record| record.accepted_revision),
        ) {
            return;
        }
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
        if let Some(record) = &claim {
            if self.suppress_improve(
                &session_id,
                &dataset,
                record.capture_revision,
                record.accepted_revision,
            ) {
                let file = record.ledger_file.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    release_dream_claim(&dir, &session_id, &token, file.as_deref())
                })
                .await;
                return;
            }
        }
        if signal.is_cancelled() || !self.automatic_improve_allowed() {
            if let Some(claim) = &claim {
                let file = claim.ledger_file.clone();
                let (dir, session) = (dir.clone(), session_id.clone());
                let _ = tokio::task::spawn_blocking(move || {
                    release_dream_claim(&dir, &session, &token, file.as_deref())
                })
                .await;
            }
            return;
        }
        let request_id = uuid::Uuid::new_v4().to_string();
        let started = std::time::Instant::now();
        let improved_at = self.now() as f64;
        self.observe_for(
            "improve",
            "started",
            ObserveFields {
                request_id: Some(request_id.clone()),
                meta: meta(json!({"reason": reason})),
                ..ObserveFields::default()
            },
            Some((&session_id, &dataset)),
        );
        let result = client
            .improve_with_receipt(
                Some(&dataset),
                std::slice::from_ref(&session_id),
                CallOptions {
                    timeout_ms: Some(DREAM_IMPROVE_TIMEOUT_MS),
                    signal: Some(signal.clone()),
                    ..CallOptions::default()
                },
            )
            .await;
        let latency = Some(started.elapsed().as_millis() as u64);
        match result {
            Ok(receipt) => {
                self.persist_backend_acceptance(
                    &request_id,
                    "improve",
                    Some(&session_id),
                    &dataset,
                    &receipt,
                );
                let revision = claim
                    .as_ref()
                    .map_or(revision, |record| record.capture_revision);
                self.improve_result(&session_id, &dataset, revision, true);
                let _ = mark_dream_accepted(&dir, &session_id, &dataset, revision);
                self.observe_for(
                    "improve",
                    "succeeded",
                    ObserveFields {
                        request_id: Some(request_id),
                        latency_ms: latency,
                        meta: meta(json!({"outcomeVersion": 1, "outcome": "accepted", "accepted": true, "searchable": "unknown"})),
                        ..ObserveFields::default()
                    },
                    Some((&session_id, &dataset)),
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
                let revision = claim
                    .as_ref()
                    .map_or(revision, |record| record.capture_revision);
                self.improve_result(&session_id, &dataset, revision, false);
                let kind = error_kind(&error);
                self.state().last_error = Some(kind.clone());
                self.observe_for(
                    "improve",
                    "failed",
                    ObserveFields {
                        request_id: Some(request_id),
                        latency_ms: latency,
                        error: Some(kind),
                        ..ObserveFields::default()
                    },
                    Some((&session_id, &dataset)),
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
        let (client, dataset, signal) = {
            let mut state = self.state();
            if !Self::enabled(&state, self.offline)
                || !state.config.improves()
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
            (
                client,
                state.active_dataset(),
                state.automatic_write_cancel.child_token(),
            )
        };
        let this = self.arc();
        Some(tokio::spawn(async move {
            this.dream_pass(client, dataset, signal).await;
            this.state().dream_in_flight = false;
        }))
    }

    async fn dream_pass(&self, client: CogneeClient, dataset: String, signal: CancellationToken) {
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
                if state.shutting_down
                    || !Self::enabled(&state, self.offline)
                    || !state.config.improves()
                {
                    return;
                }
                if state.improve_in_flight
                    && state.session_id.as_deref() == Some(record.session_id.as_str())
                {
                    continue;
                }
            }
            if self.suppress_improve(
                &record.session_id,
                &record.dataset,
                record.capture_revision,
                record.accepted_revision,
            ) {
                continue;
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
            if self.suppress_improve(
                &claim.session_id,
                &claim.dataset,
                claim.capture_revision,
                claim.accepted_revision,
            ) {
                let _ = release(
                    dir.clone(),
                    claim.session_id.clone(),
                    token,
                    claim.ledger_file.clone(),
                )
                .await;
                continue;
            }
            if signal.is_cancelled() || !self.automatic_improve_allowed() {
                let _ = release(
                    dir.clone(),
                    record.session_id.clone(),
                    token,
                    claim.ledger_file.clone(),
                )
                .await;
                return;
            }
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
            self.observe_for(
                "improve",
                "started",
                ObserveFields {
                    request_id: Some(request_id.clone()),
                    meta: meta(meta_value.clone()),
                    ..ObserveFields::default()
                },
                Some((&record.session_id, &record.dataset)),
            );
            let result = client
                .improve_with_receipt(
                    Some(&claim.dataset),
                    std::slice::from_ref(&claim.session_id),
                    CallOptions {
                        timeout_ms: Some(DREAM_IMPROVE_TIMEOUT_MS),
                        signal: Some(signal.clone()),
                        ..CallOptions::default()
                    },
                )
                .await;
            let latency = Some(started.elapsed().as_millis() as u64);
            let failed = match result {
                Ok(receipt) => {
                    self.persist_backend_acceptance(
                        &request_id,
                        "improve",
                        Some(&claim.session_id),
                        &claim.dataset,
                        &receipt,
                    );
                    self.improve_result(
                        &claim.session_id,
                        &claim.dataset,
                        claim.capture_revision,
                        true,
                    );
                    let _ = mark_dream_accepted(
                        &dir,
                        &claim.session_id,
                        &claim.dataset,
                        claim.capture_revision,
                    );
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
                    self.observe_for(
                        "improve",
                        "succeeded",
                        ObserveFields {
                            request_id: Some(request_id),
                            latency_ms: latency,
                            meta: meta(json!({"reason": "dream", "target": record.session_id, "outcomeVersion": 1, "outcome": "accepted", "accepted": true, "searchable": "unknown"})),
                            ..ObserveFields::default()
                        },
                        Some((&record.session_id, &record.dataset)),
                    );
                    false
                }
                Err(error) => {
                    self.improve_result(
                        &claim.session_id,
                        &claim.dataset,
                        claim.capture_revision,
                        false,
                    );
                    let kind = error_kind(&error);
                    self.state().last_error = Some(kind.clone());
                    self.observe_for(
                        "improve",
                        "failed",
                        ObserveFields {
                            request_id: Some(request_id),
                            latency_ms: latency,
                            error: Some(kind),
                            meta: meta(meta_value),
                            ..ObserveFields::default()
                        },
                        Some((&record.session_id, &record.dataset)),
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
            capability_inventory: self.capability_inventory(),
            operation_outcomes: state
                .client
                .as_ref()
                .map(CogneeClient::operation_outcomes)
                .unwrap_or_default(),
            capture_session: state.config.capture_session,
            capture_tools: state.config.capture_tools,
            auto_recall: state.config.auto_recall,
            read_policy: state.config.read_policy,
            write_policy: state.config.write_policy,
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
                .filter(|record| {
                    matches!(
                        record.state,
                        PendingState::Uncertain | PendingState::InFlight
                    )
                })
                .count(),
            helper_status: if !state.config.enabled
                || !state.config.selective_writes()
                || !state.config.helper_enabled
            {
                "off".into()
            } else if state.config.helper_provider.is_none() || state.config.helper_model.is_none()
            {
                "unconfigured".into()
            } else if let Some(error) = &state.helper_error {
                format!("error({error})")
            } else if state.helper.is_some() {
                format!(
                    "bound({})",
                    state.config.helper_provider.as_deref().unwrap_or("unknown")
                )
            } else {
                "off".into()
            },
            helper_requests: state.helper_requests,
            helper_abstains: state.helper_abstains,
            helper_skips: state.helper_skips,
        }
    }

    // ── tools and config ──

    /// Register Cognee tools under their respective read/write policies.
    pub async fn register_search_tool(&self, allowed: bool) {
        *self.tools_allowed.lock().unwrap() = Some(allowed);
        if !allowed {
            return;
        }
        let Some(harness) = self.harness() else {
            return;
        };
        let Ok(mut tools) = harness.get_tools().await else {
            return;
        };
        let config = self.config();
        let enabled = self.is_enabled();
        let has_search = tools
            .iter()
            .any(|tool| tool.tool.schema().name == SEARCH_TOOL_NAME);
        let has_propose = tools
            .iter()
            .any(|tool| tool.tool.schema().name == PROPOSE_TOOL_NAME);
        let mut changed = false;
        if !has_search {
            tools.push(ice_harness::types::HarnessTool::new(Arc::new(
                super::command::CogneeSearchTool::new(self.me.clone()),
            )));
            changed = true;
        }
        if config.selective_writes() && enabled && !has_propose {
            tools.push(ice_harness::types::HarnessTool::new(Arc::new(
                super::propose::CogneeProposeMemoryTool::new(self.me.clone()),
            )));
            changed = true;
        }
        if changed {
            let Ok(active) = harness.lane("main").get_active_tools().await else {
                return;
            };
            let _ = harness.set_tools(tools, Some(active)).await;
        }
    }

    /// Toggle Cognee tools according to availability and the active write policy.
    pub async fn apply_tool_state(&self) {
        if self.tools_allowed.lock().unwrap().unwrap_or(false) {
            self.register_search_tool(true).await;
        }
        let Some(harness) = self.harness() else {
            return;
        };
        let (enabled, selective) = {
            let state = self.state();
            (
                Self::enabled(&state, self.offline),
                state.config.selective_writes(),
            )
        };
        let Ok(mut tools) = harness.get_tools().await else {
            return;
        };
        let lane = harness.lane("main");
        let Ok(mut active) = lane.get_active_tools().await else {
            return;
        };
        let mut changed = false;
        if !enabled || !selective {
            let before = tools.len();
            tools.retain(|tool| tool.tool.schema().name != PROPOSE_TOOL_NAME);
            changed |= tools.len() != before;
            let active_before = active.len();
            active.retain(|name| name != PROPOSE_TOOL_NAME);
            changed |= active.len() != active_before;
        }
        for (name, should_enable) in [
            (SEARCH_TOOL_NAME, enabled),
            (PROPOSE_TOOL_NAME, enabled && selective),
        ] {
            if !tools.iter().any(|tool| tool.tool.schema().name == name) {
                continue;
            }
            let present = active.iter().any(|current| current == name);
            if should_enable && !present {
                active.push(name.into());
                changed = true;
            } else if !should_enable && present {
                active.retain(|current| current != name);
                changed = true;
            }
        }
        if changed && harness.set_tools(tools, Some(active)).await.is_err() {
            return;
        }
        // Tool availability may change without a resource reload. Publish the
        // actual active host registry, not names inferred from settings.
        let service = self.capability_service.lock().unwrap().upgrade();
        if let Some(service) = service {
            if let (Ok(tools), Ok(active)) = (
                harness.get_tools().await,
                harness.lane("main").get_active_tools().await,
            ) {
                let active_tools: Vec<_> = tools
                    .into_iter()
                    .filter(|tool| active.contains(&tool.tool.schema().name))
                    .collect();
                service.refresh_native_tools(&active_tools);
            }
        }
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

    pub(crate) fn save_and_adopt(&self, next: CogneeConfig) -> bool {
        self.ensure_loaded();
        if save_config(&self.config_path(), &next).is_err() {
            return false;
        }
        let mut state = self.state();
        let previous_dataset = state.active_dataset();
        if state.config.enabled != next.enabled
            || state.config.write_policy != next.write_policy
            || state.config.capture_session != next.capture_session
            || state.config.capture_tools != next.capture_tools
            || state.config.auto_improve != next.auto_improve
            || state.config.base_url != next.base_url
        {
            state.automatic_write_cancel.cancel();
            state.automatic_write_cancel = CancellationToken::new();
        }
        state.config = next;
        state.installed = true;
        Self::rebuild_helper(&mut state);
        let cwd = self.cwd.lock().unwrap().clone();
        state.refresh_dataset(cwd.as_deref(), &self.storage_dir);
        state.context_generation = state.context_generation.saturating_add(1);
        if previous_dataset != state.active_dataset() {
            state.evidence_cache.clear();
        }
        Self::rebuild_client(&mut state, self.transport.clone());
        let helper_error = state.helper_error.clone();
        drop(state);
        if let Some(error) = helper_error {
            self.notify(
                NoticeLevel::Warning,
                format!("Cognee helper binding failed: {error}"),
            );
            self.observe_helper(None, "skipped", "bind_failed", 0, 0, 0, None);
        }
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

    pub(super) fn generation(&self) -> u64 {
        self.state().context_generation
    }

    fn operation_context(&self) -> coordinator::OperationContext {
        self.state().operation_context()
    }

    pub fn offline(&self) -> bool {
        self.offline
    }

    // ── recall paths shared by the tool and commands ──

    /// Explicit searches share one scoped, context-only contract with the UI.
    pub async fn search_for_tool(&self, query: &str, signal: CancellationToken) -> (String, Value) {
        self.explicit_lookup(query, signal).await
    }

    /// `/cognee search` uses the same deadline, aliases and source filters.
    pub async fn manual_search(&self, query: &str) {
        self.explicit_search_notice(query).await;
    }

    fn disabled_message(&self) -> &'static str {
        if self.offline {
            "Cognee is disabled while icers runs offline"
        } else {
            "Cognee is disabled"
        }
    }

    /// Admit and queue one model-proposed memory against current branch evidence.
    #[cfg(test)]
    pub(crate) async fn propose_memory(
        &self,
        text: &str,
        category: Category,
        source_entry_id: &str,
        verified_by: Option<&str>,
        supersedes: Option<&str>,
    ) -> Result<String, String> {
        self.propose_memory_with_expiry(
            text,
            category,
            source_entry_id,
            verified_by,
            supersedes,
            None,
        )
        .await
    }

    #[cfg(test)]
    pub(crate) async fn propose_memory_with_expiry(
        &self,
        text: &str,
        category: Category,
        source_entry_id: &str,
        verified_by: Option<&str>,
        supersedes: Option<&str>,
        valid_hours: Option<u64>,
    ) -> Result<String, String> {
        let result = super::propose::propose_memory(
            self,
            text,
            category,
            source_entry_id,
            verified_by,
            supersedes,
            valid_hours,
            super::propose::ProposalControl::default(),
        )
        .await;
        result
    }

    /// `/cognee remember`: persist intent before sending.
    pub async fn manual_remember(&self, node_set: &str, text: &str) {
        self.manual_remember_with(node_set, text, CallOptions::default())
            .await;
    }

    async fn manual_remember_with(&self, node_set: &str, text: &str, options: CallOptions) {
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

        let dir = self.pending_dir();
        if read_pending_remember(&dir).iter().any(|existing| {
            existing.dataset == dataset
                && existing.node_set == node_set
                && existing.text == safe_text
        }) {
            self.notify(
                NoticeLevel::Warning,
                "Cognee remember already exists in the durable queue",
            );
            return;
        }
        let mut record = create_pending_remember(&safe_text, &dataset, node_set, self.now());
        record.state = PendingState::InFlight;
        match enqueue_pending_remember(&dir, &record, limit) {
            Ok(true) => {}
            Ok(false) => {
                self.state().last_error = Some("queue_full".into());
                self.notify(
                    NoticeLevel::Error,
                    "Cognee remember queue is full; no memory was sent",
                );
                return;
            }
            Err(error) => {
                self.notify(
                    NoticeLevel::Error,
                    format!("Cognee remember was not sent; queue error: {error}"),
                );
                return;
            }
        }
        // Dedupe may have returned a prior record with a different timestamp.
        let Some(record) = read_pending_remember(&dir).into_iter().find(|existing| {
            existing.dataset == dataset
                && existing.node_set == node_set
                && existing.text == safe_text
        }) else {
            self.notify(
                NoticeLevel::Error,
                "Cognee remember was not sent; queued record disappeared",
            );
            return;
        };
        let dispatch_session = self.state().session_id.clone();
        let acceptance_id = uuid::Uuid::new_v4().to_string();
        let lease_dir = dir.clone();
        let Ok(Some(_dispatch_lease)) =
            tokio::task::spawn_blocking(move || DispatchLease::try_acquire(&lease_dir))
                .await
                .unwrap_or(Ok(None))
        else {
            // The operation may belong to the process holding the lease. Do
            // not mutate its durable state when this runtime did not acquire
            // dispatch ownership.
            self.notify(
                NoticeLevel::Warning,
                "Cognee remember queued: another process owns dispatch",
            );
            return;
        };
        if self.live_client().is_none() {
            self.notify(
                NoticeLevel::Warning,
                "Cognee disabled before dispatch; durable intent retained",
            );
            return;
        }
        if update_pending_remember_state(&dir, &record.operation_id, PendingState::InFlight)
            .is_err()
        {
            self.notify(
                NoticeLevel::Error,
                "Cognee remember was not sent; queue update failed",
            );
            return;
        }

        self.append_operation(json!({
            "action": "remember",
            "operationId": record.operation_id,
            "contentHash": record.content_hash,
            "dataset": record.dataset,
            "nodeSet": record.node_set,
            "state": "inflight",
        }));
        match client
            .remember_with_receipt(
                &record.text,
                &record.node_set,
                Some(&record.dataset),
                None,
                options,
            )
            .await
        {
            Ok(receipt) => {
                if !self.persist_backend_acceptance(
                    &acceptance_id,
                    "remember",
                    dispatch_session.as_deref(),
                    &record.dataset,
                    &receipt,
                ) || remove_pending_remember(&dir, &record.operation_id).is_err()
                {
                    let _ = update_pending_remember_state(
                        &dir,
                        &record.operation_id,
                        PendingState::Uncertain,
                    );
                    self.notify(NoticeLevel::Warning, "Cognee accepted the write but local completion is uncertain; it will not be replayed automatically");
                    return;
                }
                {
                    let mut state = self.state();
                    state.remember_circuit.reset();
                    state.last_error = None;
                }
                self.notify(
                    NoticeLevel::Info,
                    "Cognee memory accepted; searchability unverified",
                );
                self.append_operation(json!({
                    "action": "remember",
                    "operationId": record.operation_id,
                    "contentHash": record.content_hash,
                    "dataset": record.dataset,
                    "nodeSet": record.node_set,
                    "state": "confirmed",
                    "outcomeVersion": 2,
                    "outcome": "accepted",
                    "searchable": "unknown",
                }));
            }
            Err(error) => {
                let kind = error_kind(&error);
                let failure_state = drain_failure_state(&kind);
                let _ = update_pending_remember_state(&dir, &record.operation_id, failure_state);
                {
                    let mut state = self.state();
                    state.last_error = Some(kind.clone());
                    state.remember_circuit.note_failure(&kind, wall_ms());
                }
                self.append_operation(json!({
                    "action": "remember",
                    "operationId": record.operation_id,
                    "contentHash": record.content_hash,
                    "dataset": record.dataset,
                    "nodeSet": record.node_set,
                    "state": if failure_state == PendingState::Uncertain { "uncertain" } else { "pending" },
                    "error": kind,
                }));
                if failure_state == PendingState::Uncertain {
                    self.notify(
                        NoticeLevel::Warning,
                        format!("Cognee remember {kind}; it may have been stored. Queued as uncertain; /cognee flush uncertain retries it"),
                    );
                } else {
                    self.notify(
                        NoticeLevel::Warning,
                        format!("Cognee unavailable ({kind}); memory queued"),
                    );
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
        let dir = self.dream_dir();
        let revision = read_dream_record_for_dataset(&dir, &session_id, &dataset)
            .map_or(0.0, |record| record.capture_revision);
        let request_id = uuid::Uuid::new_v4().to_string();
        let started = std::time::Instant::now();
        self.observe_for(
            "improve",
            "started",
            ObserveFields {
                request_id: Some(request_id.clone()),
                meta: meta(json!({"reason": "manual"})),
                ..ObserveFields::default()
            },
            Some((&session_id, &dataset)),
        );
        match client
            .improve_with_receipt(
                Some(&dataset),
                std::slice::from_ref(&session_id),
                CallOptions::default(),
            )
            .await
        {
            Ok(receipt) => {
                self.persist_backend_acceptance(
                    &request_id,
                    "improve",
                    Some(&session_id),
                    &dataset,
                    &receipt,
                );
                self.state().last_improve_at = self.now();
                self.improve_result(&session_id, &dataset, revision, true);
                let _ = mark_dream_accepted(&dir, &session_id, &dataset, revision);
                self.observe_for(
                    "improve",
                    "succeeded",
                    ObserveFields {
                        request_id: Some(request_id),
                        latency_ms: Some(started.elapsed().as_millis() as u64),
                        meta: meta(
                            json!({"reason": "manual", "outcomeVersion": 1, "outcome": "accepted", "accepted": true, "searchable": "unknown"}),
                        ),
                        ..ObserveFields::default()
                    },
                    Some((&session_id, &dataset)),
                );
                self.notify(NoticeLevel::Info, "Cognee improve dispatched");
            }
            Err(error) => {
                self.improve_result(&session_id, &dataset, revision, false);
                let kind = error_kind(&error);
                self.state().last_error = Some(kind.clone());
                self.observe_for(
                    "improve",
                    "failed",
                    ObserveFields {
                        request_id: Some(request_id),
                        latency_ms: Some(started.elapsed().as_millis() as u64),
                        error: Some(kind.clone()),
                        meta: meta(json!({"reason": "manual"})),
                        ..ObserveFields::default()
                    },
                    Some((&session_id, &dataset)),
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
        let session_id = self.ensure_session_id(host_id);
        {
            let mut state = self.state();
            if state.started_host_session.as_deref() == Some(host_id) {
                return;
            }
            state.started_host_session = Some(host_id.into());
            let cwd = self.cwd.lock().unwrap().clone();
            state.refresh_dataset(cwd.as_deref(), &self.storage_dir);
        }
        self.apply_tool_state().await;
        // Ready local facts load without a blind remote orientation request.
        self.invalidate_curated_memory();
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
                let root = repository_root(&find_git_root(&cwd))
                    .to_string_lossy()
                    .into_owned();
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
        let _ = super::admission::reconcile(
            &self.candidates_dir(),
            &self.pending_dir(),
            self.config().queue_limit,
            self.now(),
        );
        recover_inflight(&self.pending_dir());
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
            state.helper_cancel.cancel();
            state.evidence_cache.clear();
            (state.client.clone(), state.session_id.clone())
        };
        let helper_tasks = std::mem::take(&mut *self.helper_tasks.lock().unwrap());
        for mut task in helper_tasks {
            if tokio::time::timeout(Duration::from_secs(1), &mut task)
                .await
                .is_err()
            {
                task.abort();
            }
        }
        let settle = self.settle.lock().unwrap().take();
        if let Some(settle) = settle {
            let _ = settle.await;
        }
        self.wait_for_background().await;
        let (enabled, auto_improve) = {
            let state = self.state();
            (Self::enabled(&state, self.offline), state.config.improves())
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
                || !state.config.captures_tools()
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
            if !Self::enabled(&state, self.offline) || !state.config.captures_session() {
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
        self.schedule_helper();
        let ready = {
            let state = self.state();
            state.loaded && Self::enabled(&state, self.offline) && state.config.improves()
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
                let proposal = tool_name == PROPOSE_TOOL_NAME && !*is_error;
                self.on_tool_result(tool_name, input, &result.content, *is_error);
                if proposal {
                    self.notify_helper_candidates_changed();
                }
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
    /// recall, if any, within one character and (for non-legacy policies) token
    /// budget. The harness stores it after the prompt in the history.
    fn turn_scoped_memory(&self, recall: Option<Vec<RecallLine>>) -> Option<String> {
        let (summary, max_chars, read_policy) = {
            let mut state = self.state();
            (
                state.last_compact_summary.take(),
                state.config.recall_max_chars,
                state.config.read_policy,
            )
        };
        let token_limited = read_policy != ReadPolicy::Legacy;
        let budget_chars = if token_limited {
            max_chars.min(AUTO_INJECT_TOKEN_CAP * 4)
        } else {
            max_chars
        };
        let recall = recall.filter(|lines| !lines.is_empty());
        // The checkpoint yields a third of the budget to fresh recall.
        let summary_budget = if recall.is_some() {
            let target = budget_chars - budget_chars / 3;
            if token_limited
                && budget_chars
                    < super::text::last_compact_envelope_overhead()
                        + super::text::recall_empty_len()
                        + 2
            {
                0
            } else {
                target
            }
        } else {
            budget_chars
        };
        let summary = summary
            .map(|summary| {
                if token_limited {
                    format_bounded_last_compact_injection(&summary, summary_budget)
                } else {
                    format_last_compact_injection(&summary, summary_budget)
                }
            })
            .filter(|part| !part.is_empty());
        let used = summary.as_ref().map_or(0, |part| len16(part) + 2);
        let recall_budget = budget_chars.saturating_sub(used);
        let recall = recall.and_then(|lines| {
            if token_limited {
                let lines = fit_lines(&lines, recall_budget);
                (!lines.is_empty()).then(|| format_recall(&lines, recall_budget))
            } else {
                Some(format_recall(&lines, recall_budget))
            }
        });
        let parts: Vec<String> = summary
            .into_iter()
            .chain(recall)
            .filter(|part| !part.is_empty())
            .collect();
        let memory = (!parts.is_empty()).then(|| parts.join("\n\n"))?;
        if token_limited && len16(&memory) > budget_chars {
            // The recall envelope is retained by format_recall; this branch is
            // a final boundary for the combined checkpoint/header overhead.
            Some(prefix16(&memory, budget_chars.saturating_sub(1)).to_string() + "…")
        } else {
            Some(memory)
        }
    }

    /// Recall lines worth injecting. Legacy preserves all results byte-for-byte;
    /// non-legacy policies also apply metadata-based scope and freshness filters.
    fn recall_evidence(&self, results: &[RecallResult]) -> Selection {
        let repository = self.repository_identity();
        let mut state = self.state();
        if state.config.read_policy == ReadPolicy::Legacy {
            let lines = recall_lines(results);
            let kept = lines.len() as u32;
            return Selection {
                lines,
                rejected: RejectCounts::default(),
                kept,
            };
        }
        let dataset = state.active_dataset();
        let legacy_dataset = state.fallback_dataset();
        let State {
            seen_evidence,
            suppressions,
            ..
        } = &mut *state;
        let context = FilterContext {
            now_ms: self.now(),
            dataset,
            legacy_dataset,
            branch: repository
                .as_ref()
                .and_then(|identity| identity.branch.clone()),
            revision: repository
                .as_ref()
                .and_then(|identity| identity.revision.clone()),
            worktree_id: repository
                .as_ref()
                .map(|identity| identity.worktree_id.clone()),
            suppressions: Some(suppressions),
        };
        select_new_with_context(results, seen_evidence, &context)
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
        let (auto_recall, read_policy, can_attempt, top_k, dataset, legacy_dataset, budget_ms) = {
            let mut state = self.state();
            if state.config.captures_session() && !event.prompt.trim().is_empty() {
                let max = state.config.capture_max_chars;
                state.pending_prompt = Some(redact_memory_text(
                    &truncate_for_capture(event.prompt, max),
                    max,
                ));
            }
            (
                state.config.auto_recall,
                state.config.read_policy,
                state.recall_circuit.can_attempt(wall_ms()),
                state.config.top_k,
                state.active_dataset(),
                state.fallback_dataset(),
                state.config.recall_budget_ms,
            )
        };
        let mut cached = if read_policy == ReadPolicy::Event {
            let generation = self.generation();
            let mut state = self.state();
            let State {
                evidence_cache,
                seen_evidence,
                suppressions,
                ..
            } = &mut *state;
            let mut entries = evidence_cache.take(&dataset, generation, self.now(), seen_evidence);
            for entry in &mut entries {
                entry.text = entry
                    .text
                    .lines()
                    .filter(|line| suppressions.is_suppressed(line, self.now(), None).is_none())
                    .collect::<Vec<_>>()
                    .join("\n");
            }
            entries.retain(|entry| !entry.text.is_empty());
            entries
        } else {
            Vec::new()
        };
        if !auto_recall || !can_attempt {
            self.observe(
                "recall",
                "succeeded",
                succeeded(json!({"skipped": true, "reason": if !auto_recall { "auto_recall_disabled" } else { "circuit_open" }, "outcomeVersion": 1, "outcome": "skipped"})),
            );
            return self.turn_scoped_memory(Some(cached));
        }
        if let Admission::Skip(reason) = admit_prompt(read_policy, event.prompt) {
            self.observe(
                "recall",
                "succeeded",
                succeeded(if read_policy == ReadPolicy::Event {
                    json!({"skipped": true, "reason": reason.as_str(), "cacheDelivered": cached.len(), "cacheReason": if cached.is_empty() { "cache_empty" } else { "cache_ready" }, "outcomeVersion": 1, "outcome": "skipped"})
                } else {
                    json!({"skipped": true, "reason": reason.as_str(), "outcomeVersion": 1, "outcome": "skipped"})
                }),
            );
            return self.turn_scoped_memory(Some(cached));
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
                return self.turn_scoped_memory(Some(cached));
            }
        }
        let operation_context = self.operation_context();
        let network_started = std::time::Instant::now();
        let (result, legacy_attempted) = recall_with_fallback(
            &client,
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
            legacy_dataset,
            budget_ms,
        )
        .await;
        let remote_wait_ms = network_started.elapsed().as_millis() as u64;
        let still_current = self.generation() == operation_context.generation;
        if !still_current {
            self.observe(
                "recall",
                "succeeded",
                succeeded(json!({"reason": "stale_session", "outcomeVersion": 1, "outcome": "stale", "remoteWaitMs": remote_wait_ms, "sourceDataset": dataset, "sourceSessionId": session_id, "hostSessionId": event.session_id, "staleGeneration": true})),
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
                let selection = self.recall_evidence(&results);
                let mut recall_meta = json!({"hits": results.len(), "withProvenance": results.iter().filter(|result| super::evidence::provenance_tag(result).is_some()).count(), "selected": selection.lines.len(), "legacyFallback": legacy_attempted.is_some(), "outcomeVersion": 1, "outcome": if results.is_empty() { "retrieved_empty" } else { "retrieved_nonempty" }, "remoteWaitMs": remote_wait_ms, "sourceDataset": dataset, "sourceSessionId": session_id, "hostSessionId": event.session_id});
                if read_policy != ReadPolicy::Legacy {
                    recall_meta["rejected"] = json!({
                        "routine": selection.rejected.routine,
                        "duplicate": selection.rejected.duplicate,
                        "expired": selection.rejected.expired,
                        "superseded": selection.rejected.superseded,
                        "wrong_scope": selection.rejected.wrong_scope,
                        "wrong_branch": selection.rejected.wrong_branch,
                        "unknown_validity": selection.rejected.unknown_validity,
                    });
                }
                if let Some(legacy) = &legacy_attempted {
                    recall_meta["legacyDataset"] = json!(legacy);
                    recall_meta["legacyHits"] = json!(results.len());
                }
                cached.extend(selection.lines);
                let memory = self.turn_scoped_memory(Some(cached));
                if read_policy != ReadPolicy::Legacy {
                    let budget_tokens =
                        AUTO_INJECT_TOKEN_CAP.min(self.config().recall_max_chars.div_ceil(4));
                    let injected_tokens = memory.as_deref().map_or(0, super::text::estimate_tokens);
                    recall_meta["injectedTokens"] = json!(injected_tokens);
                    recall_meta["budgetTokens"] = json!(budget_tokens);
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
                        meta: meta(recall_meta),
                        ..ObserveFields::default()
                    },
                );
                memory
            }
            Err(error) => {
                let kind = error_kind(&error);
                {
                    let mut state = self.state();
                    state.last_error = Some(kind.clone());
                    state.recall_circuit.note_failure(&kind, wall_ms());
                }
                let mut failure_meta = json!({"outcomeVersion": 1, "outcome": kind, "remoteWaitMs": remote_wait_ms, "sourceDataset": dataset, "sourceSessionId": session_id, "hostSessionId": event.session_id, "legacyFallback": legacy_attempted.is_some()});
                if let Some(legacy) = &legacy_attempted {
                    failure_meta["legacyDataset"] = json!(legacy);
                }
                self.observe(
                    "recall",
                    "failed",
                    ObserveFields {
                        request_id: Some(request_id.into()),
                        latency_ms: latency(),
                        error: Some(kind.clone()),
                        meta: meta(failure_meta),
                        ..ObserveFields::default()
                    },
                );
                self.turn_scoped_memory(Some(cached))
            }
        }
    }
}

/// Only an explicit rejection is safe to retry automatically. A generic
/// transport error may occur after the server accepted the non-idempotent write.
fn drain_failure_state(kind: &str) -> PendingState {
    match kind {
        "auth_failed" | "not_found" | "invalid_request" => PendingState::Pending,
        _ => PendingState::Uncertain,
    }
}

async fn recall_with_fallback(
    client: &CogneeClient,
    query: &str,
    mut options: RecallOptions,
    legacy_dataset: Option<String>,
    budget_ms: u64,
) -> (Result<Vec<RecallResult>, CogneeError>, Option<String>) {
    let started = std::time::Instant::now();
    options.timeout_ms = Some(options.timeout_ms.unwrap_or(budget_ms).min(budget_ms));
    let result = client.recall(query, options.clone()).await;
    let remaining =
        budget_ms.saturating_sub(started.elapsed().as_millis().min(u64::MAX as u128) as u64);
    if matches!(&result, Ok(results) if results.is_empty()) && remaining >= 250 {
        if let Some(dataset) = legacy_dataset {
            let mut fallback = options;
            fallback.dataset = Some(dataset.clone());
            fallback.timeout_ms = Some(remaining);
            return (client.recall(query, fallback).await, Some(dataset));
        }
    }
    (result, None)
}

fn recall_lines(results: &[RecallResult]) -> Vec<RecallLine> {
    results
        .iter()
        .map(|result| RecallLine {
            text: result.text.clone(),
            score: result.score,
            provenance: super::evidence::provenance_tag(result),
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
    async fn request_context(
        &self,
        event: &ice_harness::extension::RequestContextEvent<'_>,
    ) -> Option<PromptContext> {
        self.context_for_request(event).await
    }

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
        if self.acknowledge_request_memory(context, persisted) {
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
        let session_id = self.ensure_session_id(event.session_id);
        self.schedule_helper();
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
        // Compaction is observed by the request coordinator; a summary is not
        // a search query and a checkpoint alone does not authorize a fetch.
        let (capture, capture_max, remember, remember_max, dataset, limit) = {
            let mut state = self.state();
            state.last_compact_summary =
                Some(redact_memory_text(summary, state.config.recall_max_chars));
            // Compaction removed earlier injections from the context.
            state.seen_evidence.clear();
            (
                state.config.captures_session(),
                state.config.capture_max_chars,
                state.config.remembers_compaction(),
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
        if !remember {
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
        let mut record = create_pending_remember(&text, &dataset, "agent_actions", self.now());
        record.state = PendingState::Pending;
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
    use async_trait::async_trait;
    use ice_agent::types::AgentToolResult;
    use ice_ai::decision::client::DecisionError;
    use ice_ai::decision::typesafe::{self, Question};
    use ice_ai::providers::faux::{FauxProvider, FauxScript, FauxStep};
    use ice_ai::types::{
        Api, AssistantMessage, Content, TextContent, TextContentType, UserMessage,
    };
    use ice_harness::agent_harness::{AgentHarness, AgentLane};
    use ice_harness::compaction::{CompactionPreparation, FileOperations};
    use ice_harness::session::types::{CompactionEntry, Entry, EntryQuery};
    use ice_harness::types::{AgentHarnessOptions, CompactionSettings};
    use std::sync::atomic::{AtomicUsize, Ordering};

    const NOW: u64 = 1_800_000_000_000;

    include!("request_tests.rs");
    include!("lookup_tests.rs");
    include!("closeout_runtime_tests.rs");
    include!("offline_replay_tests.rs");

    #[derive(Clone, Copy)]
    enum TestVerdict {
        Durable,
        NotDurable,
        Uncertain,
        ApplicableUnknown,
    }

    struct RuntimeDecider {
        calls: Arc<AtomicUsize>,
        verdict: TestVerdict,
        slow: bool,
    }

    #[async_trait]
    impl helper::DecisionDecider for RuntimeDecider {
        async fn decide(
            &self,
            request: &typesafe::Request,
            _timeout: Duration,
            _signal: &CancellationToken,
        ) -> Result<typesafe::Response, DecisionError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.slow {
                std::future::pending::<()>().await;
            }
            let answers = request
                .questions()
                .iter()
                .map(|(id, question)| {
                    let value = match question {
                        Question::Noul { .. } if id.ends_with(".applicable") => {
                            let probability =
                                if matches!(self.verdict, TestVerdict::ApplicableUnknown) {
                                    0.5
                                } else {
                                    0.9
                                };
                            json!({"type":"noul", "noul":probability})
                        }
                        Question::Noul { .. } => {
                            let probability = match self.verdict {
                                TestVerdict::Durable | TestVerdict::ApplicableUnknown => 0.9,
                                TestVerdict::NotDurable => 0.1,
                                TestVerdict::Uncertain => 0.5,
                            };
                            json!({"type":"noul", "noul":probability})
                        }
                        Question::Choice { .. } => json!({
                            "type":"choice", "choice":"constraint", "probabilities":{
                                "decision":0.0,"constraint":1.0,"preference":0.0,
                                "lesson":0.0,"none":0.0
                            }, "confidence":0.9
                        }),
                        Question::Score { .. } => unreachable!(),
                    };
                    (id.clone(), value)
                })
                .collect::<serde_json::Map<String, Value>>();
            let bytes = serde_json::to_vec(&json!({
                "model": "jev-1.14.0",
                "answers": answers,
                "usage": {"input_tokens": 1, "output_tokens": 1}
            }))
            .unwrap();
            typesafe::Response::decode_json(&bytes, request)
                .map_err(|error| DecisionError::Codec(error))
        }
    }

    fn fake_helper(verdict: TestVerdict, calls: Arc<AtomicUsize>, slow: bool) -> Arc<Helper> {
        Arc::new(Helper::with_decider(
            Arc::new(RuntimeDecider {
                calls,
                verdict,
                slow,
            }),
            20,
        ))
    }

    fn runtime(
        dir: &Path,
        transport: &Arc<FakeTransport>,
        config: Option<Value>,
        offline: bool,
    ) -> Arc<CogneeRuntime> {
        runtime_with_clock(dir, transport, config, offline, Arc::new(|| NOW))
    }

    fn runtime_with_clock(
        dir: &Path,
        transport: &Arc<FakeTransport>,
        config: Option<Value>,
        offline: bool,
        now: Arc<dyn Fn() -> u64 + Send + Sync>,
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
                now,
                offline,
                cwd: Some(dir.to_path_buf()),
            },
            notices,
        )
    }

    async fn harness_with_messages(messages: Vec<AgentMessage>) -> AgentHarness {
        let mut options = AgentHarnessOptions::default();
        options.allow_existing_session = true;
        let harness = AgentHarness::create(options).await.unwrap();
        for message in messages {
            harness.append_message(message).await.unwrap();
        }
        harness
    }

    fn clock_runtime(
        dir: &Path,
        transport: &Arc<FakeTransport>,
        clock: Arc<std::sync::atomic::AtomicU64>,
    ) -> Arc<CogneeRuntime> {
        let path = dir.join("ice-cognee").join("config.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "{}").unwrap();
        let notices = NoticeMailbox::new();
        notices.attach();
        let rt = CogneeRuntime::new(
            RuntimeOptions {
                agent_dir: dir.to_path_buf(),
                storage_dir: None,
                process_env: CogneeEnv::new(),
                home_dir: None,
                transport: transport.clone(),
                now: Arc::new(move || clock.load(std::sync::atomic::Ordering::SeqCst)),
                offline: false,
                cwd: Some(dir.to_path_buf()),
            },
            notices,
        );
        rt.ensure_loaded();
        rt
    }

    #[tokio::test]
    async fn selective_proposals_are_attributed_queued_and_category_deduplicated() {
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        transport.push(FakeResponse::json(200, json!({})));
        let rt = runtime(
            dir.path(),
            &transport,
            Some(json!({"writePolicy":"selective", "readPolicy":"on_demand"})),
            false,
        );
        let source_text = "Always keep generated files out of source control.";
        let candidate_text = "Always keep generated files out of source control";
        let harness =
            harness_with_messages(vec![AgentMessage::User(UserMessage::new(source_text, 1))]).await;
        let entries = harness
            .session()
            .find_entries(&ice_harness::session::types::EntryQuery::default())
            .await
            .unwrap();
        let source_id = entries
            .iter()
            .find_map(|entry| match entry {
                Entry::Message(message) if matches!(message.message, AgentMessage::User(_)) => {
                    Some(entry.id().to_string())
                }
                _ => None,
            })
            .unwrap();
        rt.bind(
            harness.downgrade(),
            ice_extensions::ExtensionStatusMailbox::new(),
            dir.path().to_path_buf(),
        );
        rt.propose_memory(candidate_text, Category::Constraint, &source_id, None, None)
            .await
            .unwrap();
        rt.wait_for_background().await;
        assert_eq!(transport.sent().len(), 1);
        let receipts_dir = super::super::admission::receipt_dir(&rt.candidates_dir());
        assert_eq!(
            super::super::admission::read_receipts(&receipts_dir).len(),
            1
        );
        let accepted = super::super::candidates::read_candidates(&rt.candidates_dir())
            .into_iter()
            .find(|candidate| candidate.state == CandidateState::Accepted)
            .unwrap();
        std::fs::remove_file(rt.candidates_dir().join(format!("{}.json", accepted.id))).unwrap();
        super::super::admission::reconcile(
            &rt.candidates_dir(),
            &rt.pending_dir(),
            rt.config().queue_limit,
            NOW,
        )
        .unwrap();
        assert!(
            super::super::candidates::read_candidates(&rt.candidates_dir())
                .iter()
                .any(|candidate| candidate.id == accepted.id
                    && candidate.state == CandidateState::Accepted)
        );
        let request = &transport.sent()[0];
        let Body::Multipart(parts) = &request.body else {
            panic!("remember is multipart")
        };
        assert_eq!(parts[1].value, "user_context");
        assert!(parts.last().unwrap().value.starts_with("[constraint] "));

        rt.propose_memory(candidate_text, Category::Constraint, &source_id, None, None)
            .await
            .unwrap();
        rt.wait_for_background().await;
        assert_eq!(transport.sent().len(), 1);
        rt.propose_memory(
            "Always keep generated files out of source control",
            Category::Preference,
            &source_id,
            None,
            None,
        )
        .await
        .unwrap();
        rt.wait_for_background().await;
        assert_eq!(transport.sent().len(), 2);
        let invalid = rt
            .propose_memory(
                "Always keep generated files out of source control",
                Category::Lesson,
                &source_id,
                None,
                Some("missing-target"),
            )
            .await;
        assert!(
            invalid.is_err(),
            "a user source is not a verified assistant lesson"
        );
        rt.wait_for_background().await;
        assert_eq!(transport.sent().len(), 2);
        let events = super::super::observer::read_observations(&rt.storage_dir);
        assert!(
            events
                .iter()
                .filter(|event| event.operation == "candidate")
                .all(|event| event.preview.is_none()),
            "candidate observations do not dump source text"
        );
    }

    #[tokio::test]
    async fn accepted_valid_hours_expires_at_injected_time_and_legacy_is_unchanged() {
        use std::sync::atomic::{AtomicU64, Ordering};
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        transport.push(FakeResponse::json(200, json!({})));
        let clock = Arc::new(AtomicU64::new(NOW));
        let clock_read = clock.clone();
        let rt = runtime_with_clock(
            dir.path(),
            &transport,
            Some(json!({"writePolicy":"selective", "readPolicy":"on_demand"})),
            false,
            Arc::new(move || clock_read.load(Ordering::Relaxed)),
        );
        let harness = harness_with_messages(vec![AgentMessage::User(UserMessage::new(
            "volatile environment state",
            1,
        ))])
        .await;
        let entries = harness
            .session()
            .find_entries(&ice_harness::session::types::EntryQuery::default())
            .await
            .unwrap();
        let source_id = entries
            .iter()
            .find_map(|entry| match entry {
                Entry::Message(message) if matches!(message.message, AgentMessage::User(_)) => {
                    Some(entry.id().to_string())
                }
                _ => None,
            })
            .unwrap();
        rt.bind(
            harness.downgrade(),
            ice_extensions::ExtensionStatusMailbox::new(),
            dir.path().to_path_buf(),
        );
        rt.propose_memory_with_expiry(
            "volatile environment state",
            Category::Preference,
            &source_id,
            None,
            None,
            Some(1),
        )
        .await
        .unwrap();
        rt.wait_for_background().await;
        let candidate = super::super::candidates::read_candidates(&rt.candidates_dir())
            .into_iter()
            .find(|candidate| candidate.text == "volatile environment state")
            .unwrap();
        assert_eq!(candidate.valid_until_ms, Some(NOW + 60 * 60 * 1000));
        let before_expiry = rt.recall_evidence(&[RecallResult {
            text: "volatile environment state".into(),
            score: Some(1.0),
            metadata: None,
        }]);
        assert_eq!(before_expiry.lines.len(), 1);
        clock.store(NOW + 60 * 60 * 1000, Ordering::Relaxed);
        let after_expiry = rt.recall_evidence(&[RecallResult {
            text: "volatile environment state".into(),
            score: Some(1.0),
            metadata: None,
        }]);
        assert!(after_expiry.lines.is_empty());
        assert_eq!(after_expiry.rejected.expired, 1);
        let receipt = super::super::admission::read_receipts(
            &super::super::admission::receipt_dir(&rt.candidates_dir()),
        )
        .into_iter()
        .find(|receipt| receipt.candidate.id == candidate.id)
        .unwrap();
        assert_eq!(receipt.candidate.valid_until_ms, Some(NOW + 60 * 60 * 1000));
        let mut reloaded = super::super::suppress::SuppressionSet::load(
            &rt.storage_dir.join("suppress"),
            &candidate.dataset,
        );
        reloaded.include_receipts(&rt.candidates_dir(), &candidate.dataset);
        assert_eq!(
            reloaded.is_suppressed("volatile environment state", NOW + 60 * 60 * 1000, None),
            Some(super::super::suppress::Reason::Expired)
        );

        rt.state().config.read_policy = ReadPolicy::Legacy;
        let legacy = rt.recall_evidence(&[RecallResult {
            text: "volatile environment state".into(),
            score: Some(1.0),
            metadata: None,
        }]);
        assert_eq!(legacy.lines.len(), 1);
        assert_eq!(legacy.lines[0].text, "volatile environment state");
    }

    #[tokio::test]
    async fn accepted_user_correction_persists_and_filters_old_fact_after_seen_clear() {
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        transport.push(FakeResponse::json(200, json!({})));
        transport.push(FakeResponse::json(200, json!({})));
        let rt = runtime(
            dir.path(),
            &transport,
            Some(json!({"writePolicy":"selective", "readPolicy":"on_demand"})),
            false,
        );
        let harness = harness_with_messages(vec![AgentMessage::User(UserMessage::new(
            "- old durable fact
- new durable fact",
            1,
        ))])
        .await;
        let entries = harness
            .session()
            .find_entries(&ice_harness::session::types::EntryQuery::default())
            .await
            .unwrap();
        let source_id = entries
            .iter()
            .find_map(|entry| match entry {
                Entry::Message(message) if matches!(message.message, AgentMessage::User(_)) => {
                    Some(entry.id().to_string())
                }
                _ => None,
            })
            .unwrap();
        rt.bind(
            harness.downgrade(),
            ice_extensions::ExtensionStatusMailbox::new(),
            dir.path().to_path_buf(),
        );
        rt.propose_memory(
            "old durable fact",
            Category::Decision,
            &source_id,
            None,
            None,
        )
        .await
        .unwrap();
        rt.wait_for_background().await;
        let old = super::super::candidates::read_candidates(&rt.candidates_dir())
            .into_iter()
            .find(|candidate| candidate.text == "old durable fact")
            .unwrap();
        rt.propose_memory(
            "new durable fact",
            Category::Decision,
            &source_id,
            None,
            Some(&old.id),
        )
        .await
        .unwrap();
        rt.wait_for_background().await;

        let candidates = super::super::candidates::read_candidates(&rt.candidates_dir());
        assert!(candidates.iter().any(|candidate| candidate.id == old.id));
        assert!(candidates
            .iter()
            .any(|candidate| candidate.text == "new durable fact"));
        let correction_observation = super::super::observer::read_observations(&rt.storage_dir)
            .into_iter()
            .find(|event| {
                event.operation == "candidate"
                    && event.meta.as_ref().is_some_and(|meta| {
                        meta.get("supersession").and_then(Value::as_str) == Some("recorded")
                            && !meta.contains_key("supersessionReason")
                    })
            })
            .unwrap();
        assert_eq!(correction_observation.phase, "accepted");
        let correction_receipt = super::super::admission::read_receipts(
            &super::super::admission::receipt_dir(&rt.candidates_dir()),
        )
        .into_iter()
        .find(|receipt| receipt.candidate.text == "new durable fact")
        .unwrap();
        assert_eq!(
            correction_receipt
                .superseded_target
                .as_ref()
                .map(|target| target.hash.as_str()),
            Some(super::super::suppress::content_hash("old durable fact").as_str())
        );

        rt.state().seen_evidence.clear();
        let selection = rt.recall_evidence(&[RecallResult {
            text: "old durable fact\nnew durable fact".into(),
            score: Some(1.0),
            metadata: None,
        }]);
        assert_eq!(selection.lines.len(), 1);
        assert_eq!(selection.lines[0].text, "new durable fact");
        assert_eq!(selection.rejected.superseded, 1);
        let same_fact_under_another_query = rt.recall_evidence(&[RecallResult {
            text: "old durable fact".into(),
            score: Some(1.0),
            metadata: None,
        }]);
        assert!(same_fact_under_another_query.lines.is_empty());
        assert_eq!(same_fact_under_another_query.rejected.superseded, 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn selective_proposal_runs_in_a_full_prompt_tool_answer_turn() {
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        transport.push(FakeResponse::json(
            200,
            json!({"results": [{"text": "Use the agreed local constraint"}]}),
        ));
        transport.push(FakeResponse::json(200, json!({})));
        let rt = runtime(
            dir.path(),
            &transport,
            Some(json!({"writePolicy":"selective", "readPolicy":"legacy", "autoRecall":true})),
            false,
        );
        let candidate_text = "always keep generated files out of source control";
        let script = FauxScript::new()
            .with_tool_call(
                PROPOSE_TOOL_NAME,
                json!({
                    "text": candidate_text,
                    "category": "constraint",
                    "source_entry_id": "source-entry"
                }),
            )
            .with_text("I recorded the verified constraint.");
        let provider = FauxProvider::new(script.clone());
        let mut options = AgentHarnessOptions::default();
        options.allow_existing_session = true;
        options.model = provider.default_model().clone();
        options.models.push(provider.clone());
        let harness = AgentHarness::create(options).await.unwrap();
        let source_id = harness
            .append_message(AgentMessage::User(UserMessage::new(
                "Please always keep generated files out of source control.",
                1,
            )))
            .await
            .unwrap();
        rt.bind(
            harness.downgrade(),
            ice_extensions::ExtensionStatusMailbox::new(),
            dir.path().to_path_buf(),
        );
        rt.register_search_tool(true).await;
        rt.apply_tool_state().await;
        let mut proposal = json!({
            "text": candidate_text,
            "category": "constraint",
            "source_entry_id": source_id
        });
        script.set_responses(vec![
            FauxStep::tool_call(PROPOSE_TOOL_NAME, proposal.take()),
            FauxStep::text("I recorded the verified constraint."),
        ]);
        harness.set_extensions(vec![rt.clone()]);

        harness
            .prompt_text("Record the durable constraint from earlier.", Vec::new())
            .await
            .unwrap();
        rt.wait_for_background().await;

        let entries = harness
            .session()
            .find_entries(&ice_harness::session::types::EntryQuery::default())
            .await
            .unwrap();
        assert!(
            entries.iter().any(|entry| matches!(entry,
                Entry::Message(message)
                    if matches!(&message.message, AgentMessage::Custom(custom)
                        if custom.data["customType"] == MEMORY_CUSTOM_TYPE)
            )),
            "entries={entries:#?}"
        );
        assert!(entries.iter().any(|entry| matches!(entry,
            Entry::Message(message)
                if matches!(&message.message, AgentMessage::ToolResult(result) if !result.is_error)
        )));
        assert!(entries.iter().any(|entry| matches!(entry,
            Entry::Message(message)
                if matches!(&message.message, AgentMessage::Assistant(assistant)
                    if assistant.content.iter().any(|block| matches!(block,
                        Content::Text(text) if text.text == "I recorded the verified constraint.")))
        )));
        assert_eq!(transport.sent().len(), 2);
        let requests = transport.sent();
        let request = requests
            .iter()
            .find(|request| request.url.ends_with("/api/v1/remember"))
            .unwrap();
        let Body::Multipart(parts) = &request.body else {
            panic!("remember is multipart")
        };
        assert_eq!(parts[1].value, "user_context");
        assert!(parts.last().unwrap().value.starts_with("[constraint] "));
        let observations = super::super::observer::read_observations(&rt.storage_dir);
        assert!(observations
            .iter()
            .any(|event| event.operation == "injection" && event.phase == "succeeded"));
        assert!(observations.iter().any(|event| event.operation == "recall"
            && event.phase == "succeeded"
            && event.meta.as_ref().is_some_and(|meta| {
                meta["hits"] == 1 && meta["outcome"] == "retrieved_nonempty"
            })));
        let active = harness.lane("main").get_active_tools().await.unwrap();
        assert!(active.contains(&PROPOSE_TOOL_NAME.to_string()));
    }

    #[tokio::test]
    async fn manual_remember_still_sends_under_selective_policy() {
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        transport.push(FakeResponse::json(200, json!({})));
        let rt = runtime(
            dir.path(),
            &transport,
            Some(json!({"writePolicy":"selective", "readPolicy":"on_demand"})),
            false,
        );

        rt.manual_remember("user_context", "explicitly remember this")
            .await;
        rt.wait_for_background().await;

        let requests = transport.sent();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].url.ends_with("/api/v1/remember"));
        let records = read_pending_remember(&rt.pending_dir());
        assert!(records.is_empty());
    }

    #[tokio::test]
    async fn selective_candidate_tool_is_only_registered_when_available_and_selective() {
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        let rt = runtime(
            dir.path(),
            &transport,
            Some(json!({"writePolicy":"manual"})),
            false,
        );
        let harness = harness_with_messages(Vec::new()).await;
        rt.bind(
            harness.downgrade(),
            ice_extensions::ExtensionStatusMailbox::new(),
            dir.path().to_path_buf(),
        );
        rt.register_search_tool(true).await;
        assert!(!harness
            .get_tools()
            .await
            .unwrap()
            .iter()
            .any(|tool| { tool.tool.schema().name == PROPOSE_TOOL_NAME }));
        let mut config = rt.config();
        config.write_policy = WritePolicy::Selective;
        assert!(rt.save_and_adopt(config));
        rt.apply_tool_state().await;
        let proposal = harness
            .get_tools()
            .await
            .unwrap()
            .into_iter()
            .find(|tool| tool.tool.schema().name == PROPOSE_TOOL_NAME)
            .unwrap();
        let error = proposal
            .tool
            .execute(
                "invalid",
                json!({"text": "candidate", "category": "unknown", "source_entry_id": "missing"}),
                CancellationToken::new(),
                Arc::new(|_| {}),
            )
            .await
            .unwrap_err();
        assert!(matches!(error, ice_agent::AgentError::Validation(_)));

        for valid_hours in [json!(0), json!(8761), json!(-1), json!(1.5), json!("2")] {
            let invalid_validity = proposal
                .tool
                .execute(
                    "invalid-validity",
                    json!({
                        "text": "candidate",
                        "category": "constraint",
                        "source_entry_id": "missing",
                        "valid_hours": valid_hours
                    }),
                    CancellationToken::new(),
                    Arc::new(|_| {}),
                )
                .await
                .unwrap_err();
            assert!(matches!(
                invalid_validity,
                ice_agent::AgentError::Validation(_)
            ));
        }

        let mut config = rt.config();
        config.enabled = false;
        assert!(rt.save_and_adopt(config));
        let unavailable = proposal
            .tool
            .execute(
                "unavailable",
                json!({
                    "text": "candidate",
                    "category": "constraint",
                    "source_entry_id": "missing"
                }),
                CancellationToken::new(),
                Arc::new(|_| {}),
            )
            .await
            .unwrap_err();
        assert!(matches!(unavailable, ice_agent::AgentError::Tool(message)
            if message == "Cognee is unavailable"));
    }

    #[tokio::test]
    async fn helper_judges_only_pending_candidates_offloads_acceptance_and_abstains_safely() {
        for (verdict, expected_state, expected_requests) in [
            (TestVerdict::Durable, CandidateState::Accepted, 1),
            (
                TestVerdict::NotDurable,
                CandidateState::Rejected("helper_not_durable".into()),
                0,
            ),
            (TestVerdict::Uncertain, CandidateState::Pending, 0),
            (TestVerdict::ApplicableUnknown, CandidateState::Pending, 0),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let transport = Arc::new(FakeTransport::default());
            let rt = runtime(
                dir.path(),
                &transport,
                Some(
                    json!({"readPolicy":"event", "writePolicy":"selective", "helperProvider":"fake"}),
                ),
                false,
            );
            let harness = harness_with_messages(vec![AgentMessage::User(UserMessage::new(
                "Please always keep generated assets outside the source repository.",
                1,
            ))])
            .await;
            let source_id = harness
                .session()
                .find_entries(&EntryQuery::default())
                .await
                .unwrap()
                .into_iter()
                .find_map(|entry| match &entry {
                    Entry::Message(message) if matches!(message.message, AgentMessage::User(_)) => {
                        Some(entry.id().to_string())
                    }
                    _ => None,
                })
                .unwrap();
            rt.bind(
                harness.downgrade(),
                ice_extensions::ExtensionStatusMailbox::new(),
                dir.path().to_path_buf(),
            );
            let host_session_id = harness.session().storage().metadata().id.clone();
            rt.context_for_request(&ice_harness::extension::RequestContextEvent {
                prompt: "Please always keep generated assets outside the source repository.",
                original_prompt: Some(
                    "Please always keep generated assets outside the source repository.",
                ),
                messages: &[],
                system_prompt: "test system",
                session_id: &host_session_id,
                run_id: "helper-task",
                lane_id: "main",
                branch_generation: 0,
                request_index: 0,
                context_window: 128_000,
                remaining_context_tokens: 800,
                latest_compaction_at: None,
                signal: CancellationToken::new(),
            })
            .await;
            let sources = rt
                .candidate_source_snapshot(&CancellationToken::new())
                .await
                .unwrap();
            let mut candidate = Candidate {
                id: uuid::Uuid::new_v4().to_string(),
                admission_id: None,
                source_scope: None,
                category: Category::Constraint,
                text: "always keep generated assets outside the source repository".into(),
                branch: None,
                valid_until_ms: None,
                source: super::super::candidates::Source {
                    entry_id: source_id.clone(),
                    role: Role::User,
                },
                verified_by: None,
                supersedes: None,
                dataset: rt.active_dataset(),
                created_at: NOW,
                state: CandidateState::Pending,
            };
            let evidence =
                super::super::source_validation::validate_source(&candidate, &sources.messages)
                    .unwrap();
            candidate.source_scope = Some(super::super::candidates::SourceScope {
                session_id: sources.operation.host_session_id.clone(),
                task_id: sources.task_id.clone(),
                conversation_branch: Some(
                    sources.messages.iter().map(|(id, _)| id.clone()).collect(),
                ),
                git_branch: None,
                git_revision: None,
                worktree_id: None,
                source_text_hash: evidence.source_text_hash,
            });
            rt.store_pending_candidate_for_test(&candidate);
            let calls = Arc::new(AtomicUsize::new(0));
            rt.use_helper_for_test(fake_helper(verdict, calls.clone(), false));
            rt.notify_helper_candidates_changed();
            rt.wait_for_background().await;
            let records = super::super::candidates::read_candidates(&rt.candidates_dir());
            assert_eq!(
                calls.load(Ordering::SeqCst),
                1,
                "helper calls with candidate records {records:?}"
            );
            assert_eq!(transport.sent().len(), expected_requests);
            candidate = records
                .into_iter()
                .find(|item| item.source.entry_id == source_id)
                .unwrap();
            assert_eq!(candidate.state, expected_state);
            if expected_requests == 1 {
                let Body::Multipart(parts) = &transport.sent()[0].body else {
                    panic!("remember multipart")
                };
                assert_eq!(parts[1].value, "user_context");
                assert!(parts.last().unwrap().value.starts_with("[constraint] "));
            }
        }
    }

    #[tokio::test]
    async fn helper_rejects_unsupported_source_before_remote_judgment() {
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        let rt = runtime(
            dir.path(),
            &transport,
            Some(json!({
                "readPolicy":"event",
                "writePolicy":"selective",
                "helperProvider":"fake"
            })),
            false,
        );
        let harness = harness_with_messages(vec![AgentMessage::User(UserMessage::new(
            "Never enable automatic deployment without review.",
            1,
        ))])
        .await;
        let source_id = harness
            .session()
            .find_entries(&EntryQuery::default())
            .await
            .unwrap()
            .into_iter()
            .find_map(|entry| match &entry {
                Entry::Message(message) if matches!(message.message, AgentMessage::User(_)) => {
                    Some(entry.id().to_string())
                }
                _ => None,
            })
            .unwrap();
        rt.bind(
            harness.downgrade(),
            ice_extensions::ExtensionStatusMailbox::new(),
            dir.path().to_path_buf(),
        );
        let host_session_id = harness.session().storage().metadata().id.clone();
        rt.context_for_request(&ice_harness::extension::RequestContextEvent {
            prompt: "Never enable automatic deployment without review.",
            original_prompt: Some("Never enable automatic deployment without review."),
            messages: &[],
            system_prompt: "test system",
            session_id: &host_session_id,
            run_id: "invalid-source-task",
            lane_id: "main",
            branch_generation: 0,
            request_index: 0,
            context_window: 128_000,
            remaining_context_tokens: 800,
            latest_compaction_at: None,
            signal: CancellationToken::new(),
        })
        .await;
        let sources = rt
            .candidate_source_snapshot(&CancellationToken::new())
            .await
            .unwrap();
        let mut candidate = Candidate {
            id: uuid::Uuid::new_v4().to_string(),
            admission_id: None,
            source_scope: None,
            category: Category::Constraint,
            text: "Enable automatic deployment without review".into(),
            source: super::super::candidates::Source {
                entry_id: source_id.clone(),
                role: Role::User,
            },
            verified_by: None,
            supersedes: None,
            dataset: rt.active_dataset(),
            branch: None,
            valid_until_ms: None,
            created_at: NOW,
            state: CandidateState::Pending,
        };
        let evidence =
            super::super::source_validation::validate_source(&candidate, &sources.messages)
                .unwrap_err();
        assert_eq!(
            evidence,
            super::super::source_validation::SourceRejection::UnsupportedSpan
        );
        candidate.source_scope = Some(super::super::candidates::SourceScope {
            session_id: sources.operation.host_session_id.clone(),
            task_id: sources.task_id.clone(),
            conversation_branch: Some(sources.messages.iter().map(|(id, _)| id.clone()).collect()),
            git_branch: None,
            git_revision: None,
            worktree_id: None,
            source_text_hash: String::new(),
        });
        rt.store_pending_candidate_for_test(&candidate);
        let mut stale_candidate = candidate.clone();
        stale_candidate.id = uuid::Uuid::new_v4().to_string();
        stale_candidate.text = "Never enable automatic deployment without review".into();
        let evidence =
            super::super::source_validation::validate_source(&stale_candidate, &sources.messages)
                .unwrap();
        let scope = stale_candidate.source_scope.as_mut().unwrap();
        scope.task_id = Some("different-task".into());
        scope.source_text_hash = evidence.source_text_hash;
        rt.store_pending_candidate_for_test(&stale_candidate);
        let calls = Arc::new(AtomicUsize::new(0));
        rt.use_helper_for_test(fake_helper(TestVerdict::Durable, calls.clone(), false));
        rt.notify_helper_candidates_changed();
        rt.wait_for_background().await;

        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(transport.sent().is_empty());
        let records = super::super::candidates::read_candidates(&rt.candidates_dir());
        candidate = records
            .iter()
            .find(|stored| stored.id == candidate.id)
            .unwrap()
            .clone();
        assert_eq!(
            candidate.state,
            CandidateState::Rejected("unsupported_source_span".into())
        );
        assert_eq!(
            records
                .iter()
                .find(|stored| stored.id == stale_candidate.id)
                .unwrap()
                .state,
            CandidateState::Rejected("stale_source_scope".into())
        );
    }

    #[tokio::test]
    async fn repeated_settles_without_candidate_revision_do_no_helper_io_or_jobs() {
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        let rt = runtime(
            dir.path(),
            &transport,
            Some(json!({"writePolicy":"selective"})),
            true,
        );
        for _ in 0..100 {
            rt.on_run_settled();
        }
        assert!(rt.helper_tasks.lock().unwrap().is_empty());
        assert!(!rt.candidates_dir().exists());
        assert!(transport.sent().is_empty());
    }

    #[tokio::test]
    async fn helper_off_leaves_pending_and_prompt_hook_never_judges() {
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        let rt = runtime(
            dir.path(),
            &transport,
            Some(json!({"writePolicy":"selective"})),
            false,
        );
        let harness = harness_with_messages(vec![AgentMessage::User(UserMessage::new(
            "I prefer compact reports for status updates.",
            1,
        ))])
        .await;
        let source_id = harness
            .session()
            .find_entries(&EntryQuery::default())
            .await
            .unwrap()
            .into_iter()
            .find_map(|entry| match &entry {
                Entry::Message(message) if matches!(message.message, AgentMessage::User(_)) => {
                    Some(entry.id().to_string())
                }
                _ => None,
            })
            .unwrap();
        rt.bind(
            harness.downgrade(),
            ice_extensions::ExtensionStatusMailbox::new(),
            dir.path().to_path_buf(),
        );
        let mut candidate = Candidate {
            id: uuid::Uuid::new_v4().to_string(),
            admission_id: None,
            source_scope: None,
            category: Category::Preference,
            text: "Prefer compact reports".into(),
            branch: None,
            valid_until_ms: None,
            source: super::super::candidates::Source {
                entry_id: source_id,
                role: Role::User,
            },
            verified_by: None,
            supersedes: None,
            dataset: rt.active_dataset(),
            created_at: NOW,
            state: CandidateState::Pending,
        };
        rt.store_pending_candidate_for_test(&candidate);
        for _ in 0..20 {
            let event = BeforeRunEvent {
                prompt: "An ordinary prompt",
                system_prompt: None,
                session_id: "host1",
                signal: CancellationToken::new(),
            };
            let _ = rt.prompt_context(&event).await;
        }
        rt.on_run_settled();
        rt.wait_for_background().await;
        let records = super::super::candidates::read_candidates(&rt.candidates_dir());
        candidate = records
            .into_iter()
            .find(|item| item.id == candidate.id)
            .unwrap();
        assert_eq!(candidate.state, CandidateState::Pending);
        assert!(transport.sent().is_empty());
        candidate.created_at = NOW - super::super::candidates::CANDIDATE_TTL_MS;
        super::super::candidates::set_state(
            &rt.candidates_dir(),
            &mut candidate,
            CandidateState::Pending,
        )
        .unwrap();
        rt.on_run_settled();
        assert_eq!(
            super::super::candidates::read_candidates(&rt.candidates_dir())[0].state,
            CandidateState::Pending,
            "disabled helper scheduling must not mutate candidate storage"
        );
    }

    #[tokio::test]
    async fn slow_helper_is_background_and_shutdown_cancels_it() {
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        let rt = runtime(
            dir.path(),
            &transport,
            Some(json!({"readPolicy":"event", "writePolicy":"selective", "helperProvider":"fake"})),
            false,
        );
        let harness = harness_with_messages(vec![AgentMessage::User(UserMessage::new(
            "Always retain source provenance for generated artifacts.",
            1,
        ))])
        .await;
        let source_id = harness
            .session()
            .find_entries(&EntryQuery::default())
            .await
            .unwrap()
            .into_iter()
            .find_map(|entry| match &entry {
                Entry::Message(message) if matches!(message.message, AgentMessage::User(_)) => {
                    Some(entry.id().to_string())
                }
                _ => None,
            })
            .unwrap();
        rt.bind(
            harness.downgrade(),
            ice_extensions::ExtensionStatusMailbox::new(),
            dir.path().to_path_buf(),
        );
        let host_session_id = harness.session().storage().metadata().id.clone();
        rt.context_for_request(&ice_harness::extension::RequestContextEvent {
            prompt: "Always retain source provenance for generated artifacts.",
            original_prompt: Some("Always retain source provenance for generated artifacts."),
            messages: &[],
            system_prompt: "test system",
            session_id: &host_session_id,
            run_id: "slow-helper-task",
            lane_id: "main",
            branch_generation: 0,
            request_index: 0,
            context_window: 128_000,
            remaining_context_tokens: 800,
            latest_compaction_at: None,
            signal: CancellationToken::new(),
        })
        .await;
        let sources = rt
            .candidate_source_snapshot(&CancellationToken::new())
            .await
            .unwrap();
        let mut candidate = Candidate {
            id: uuid::Uuid::new_v4().to_string(),
            admission_id: None,
            source_scope: None,
            category: Category::Constraint,
            text: "Always retain source provenance for generated artifacts".into(),
            branch: None,
            valid_until_ms: None,
            source: super::super::candidates::Source {
                entry_id: source_id,
                role: Role::User,
            },
            verified_by: None,
            supersedes: None,
            dataset: rt.active_dataset(),
            created_at: NOW,
            state: CandidateState::Pending,
        };
        let evidence =
            super::super::source_validation::validate_source(&candidate, &sources.messages)
                .unwrap();
        candidate.source_scope = Some(super::super::candidates::SourceScope {
            session_id: sources.operation.host_session_id.clone(),
            task_id: sources.task_id,
            conversation_branch: Some(sources.messages.iter().map(|(id, _)| id.clone()).collect()),
            git_branch: None,
            git_revision: None,
            worktree_id: None,
            source_text_hash: evidence.source_text_hash,
        });
        rt.store_pending_candidate_for_test(&candidate);
        let calls = Arc::new(AtomicUsize::new(0));
        rt.use_helper_for_test(fake_helper(TestVerdict::Durable, calls.clone(), true));
        let started = std::time::Instant::now();
        rt.notify_helper_candidates_changed();
        assert!(started.elapsed() < Duration::from_millis(100));
        tokio::task::yield_now().await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        tokio::time::timeout(Duration::from_secs(1), rt.session_shutdown())
            .await
            .unwrap();
        let after_cancel = super::super::candidates::read_candidates(&rt.candidates_dir());
        assert_eq!(
            after_cancel
                .iter()
                .find(|item| item.id == candidate.id)
                .unwrap()
                .state,
            CandidateState::Pending
        );
    }

    #[tokio::test]
    async fn selective_policy_disables_all_legacy_writes_even_with_switches_on() {
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        let rt = runtime(
            dir.path(),
            &transport,
            Some(json!({
                "writePolicy":"selective", "readPolicy":"on_demand", "captureSession":true,
                "captureTools":true, "autoImprove":true, "autoRemember":"compaction"
            })),
            false,
        );
        rt.on_agent_event(&AgentEvent::ToolExecutionEnd {
            tool_call_id: "c1".into(),
            tool_name: "bash".into(),
            result: AgentToolResult::text("bash succeeded"),
            is_error: false,
        });
        rt.on_agent_event(&AgentEvent::AgentEnd {
            messages: vec![assistant("Done.")],
        });
        rt.on_run_settled();
        rt.wait_for_background().await;
        let entry = CompactionEntry {
            base: base("compaction", 1),
            summary: "not retained".into(),
            retained_tail: Vec::new(),
            tokens_before: 1,
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
        assert!(rt.run_improve("idle").is_none());
        assert!(rt.run_dream().is_none());
        assert!(transport.sent().is_empty());
    }

    #[tokio::test]
    async fn rejected_selective_candidate_is_recorded_without_network_promotion() {
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        let rt = runtime(
            dir.path(),
            &transport,
            Some(json!({"writePolicy":"selective", "readPolicy":"on_demand"})),
            false,
        );
        let harness = harness_with_messages(vec![AgentMessage::User(UserMessage::new(
            "That is the only requested detail.",
            1,
        ))])
        .await;
        let entry = harness
            .session()
            .find_entries(&ice_harness::session::types::EntryQuery::default())
            .await
            .unwrap()
            .into_iter()
            .find(|entry| matches!(entry, Entry::Message(message) if matches!(message.message, AgentMessage::User(_))))
            .unwrap();
        rt.bind(
            harness.downgrade(),
            ice_extensions::ExtensionStatusMailbox::new(),
            dir.path().to_path_buf(),
        );
        let status = rt
            .propose_memory(
                "not present in the user source",
                Category::Constraint,
                entry.id(),
                None,
                None,
            )
            .await;
        assert!(status.is_err());
        rt.wait_for_background().await;
        assert!(transport.sent().is_empty());
        let observations =
            super::super::observer::read_observations(&dir.path().join("ice-cognee"));
        assert!(
            observations.iter().any(|event| {
                event.operation == "candidate"
                    && event.meta.as_ref().is_some_and(|meta| {
                        meta["category"] == "constraint"
                            && meta["outcome"] == "rejected"
                            && matches!(
                                meta["reason"].as_str(),
                                Some("unsupported_source_span" | "routine_activity")
                            )
                            && meta["outcomeVersion"] == 1
                    })
            }),
            "status={status:?}; observations={observations:?}"
        );
    }

    #[tokio::test]
    async fn improve_acceptance_coalesces_and_manual_bypasses() {
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        for _ in 0..3 {
            transport.push(FakeResponse::json(200, json!({})));
        }
        let rt = runtime(
            dir.path(),
            &transport,
            Some(json!({"enabled": true})),
            false,
        );
        rt.ensure_loaded();
        rt.ensure_session_id("host1");
        let dataset = rt.state().active_dataset();
        let session = "ice_host1";
        mark_dream_captured(&rt.dream_dir(), session, &dataset, NOW as f64).unwrap();
        let client = rt.live_client().unwrap();
        rt.improve_current(client.clone(), session.into(), dataset.clone(), "idle")
            .await;
        rt.improve_current(client.clone(), session.into(), dataset.clone(), "idle")
            .await;
        assert_eq!(transport.sent().len(), 1);
        rt.manual_improve().await;
        assert_eq!(transport.sent().len(), 2);
        mark_dream_captured(&rt.dream_dir(), session, &dataset, NOW as f64).unwrap();
        rt.improve_current(client, session.into(), dataset, "idle")
            .await;
        assert_eq!(transport.sent().len(), 3);
        rt.wait_for_background().await;
        let events = super::super::observer::read_observations(&rt.storage_dir);
        assert!(events.iter().any(|e| e.operation == "improve"
            && e.phase == "skipped"
            && e.meta.as_ref().unwrap()["reason"] == "already_accepted_revision"));
        assert!(events.iter().any(|e| e.phase == "succeeded"
            && e.operation == "improve"
            && e.meta.as_ref().unwrap()["accepted"] == true
            && e.meta.as_ref().unwrap()["searchable"] == "unknown"));
    }

    #[tokio::test]
    async fn improve_failure_backoff_steps_and_new_revision_resets() {
        use std::sync::atomic::{AtomicU64, Ordering};
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        for _ in 0..4 {
            transport.push(FakeResponse::Fail);
        }
        let clock = Arc::new(AtomicU64::new(NOW));
        let rt = clock_runtime(dir.path(), &transport, clock.clone());
        let dataset = "dataset";
        let session = "session";
        let client = super::super::client::tests::client(transport.clone());
        mark_dream_captured(&rt.dream_dir(), session, dataset, NOW as f64).unwrap();
        for (advance, count) in [
            (0, 1),
            (0, 1),
            (60_000, 2),
            (299_999, 2),
            (1, 3),
            (299_999, 3),
        ] {
            clock.fetch_add(advance, Ordering::SeqCst);
            rt.improve_current(client.clone(), session.into(), dataset.into(), "idle")
                .await;
            assert_eq!(transport.sent().len(), count);
        }
        mark_dream_captured(
            &rt.dream_dir(),
            session,
            dataset,
            clock.load(Ordering::SeqCst) as f64,
        )
        .unwrap();
        rt.improve_current(client, session.into(), dataset.into(), "idle")
            .await;
        assert_eq!(transport.sent().len(), 4);
        rt.wait_for_background().await;
        let events = super::super::observer::read_observations(&rt.storage_dir);
        assert!(events.iter().any(|e| e.phase == "skipped"
            && e.meta.as_ref().unwrap()["reason"] == "backoff"
            && e.meta.as_ref().unwrap()["retryAfterMs"] == 60_000));
        assert!(events.iter().any(|e| e.phase == "skipped"
            && e.meta.as_ref().unwrap()["reason"] == "backoff"
            && e.meta.as_ref().unwrap()["retryAfterMs"] == 1));
    }

    #[tokio::test]
    async fn manual_improve_bypasses_failure_backoff() {
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        transport.push(FakeResponse::Fail);
        transport.push(FakeResponse::json(200, json!({})));
        let rt = runtime(
            dir.path(),
            &transport,
            Some(json!({"enabled": true})),
            false,
        );
        rt.ensure_loaded();
        rt.ensure_session_id("host1");
        let dataset = rt.state().active_dataset();
        mark_dream_captured(&rt.dream_dir(), "ice_host1", &dataset, NOW as f64).unwrap();
        let client = rt.live_client().unwrap();
        rt.improve_current(client.clone(), "ice_host1".into(), dataset.clone(), "idle")
            .await;
        rt.improve_current(client, "ice_host1".into(), dataset.clone(), "idle")
            .await;
        assert_eq!(transport.sent().len(), 1);
        rt.manual_improve().await;
        assert_eq!(transport.sent().len(), 2);
        assert_eq!(
            read_dream_record_for_dataset(&rt.dream_dir(), "ice_host1", &dataset)
                .unwrap()
                .accepted_revision,
            Some(1.0)
        );
    }

    #[tokio::test]
    async fn dream_skips_previously_accepted_older_revision() {
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        let rt = runtime(dir.path(), &transport, Some(json!({})), false);
        rt.ensure_loaded();
        let ledger = rt.dream_dir();
        mark_dream_captured(&ledger, "older", "dataset", (NOW - 1_000_000) as f64).unwrap();
        mark_dream_accepted(&ledger, "older", "dataset", 1.0).unwrap();
        rt.dream_pass(
            super::super::client::tests::client(transport.clone()),
            "other".into(),
            CancellationToken::new(),
        )
        .await;
        rt.wait_for_background().await;
        assert!(transport.sent().is_empty());
        let events = super::super::observer::read_observations(&rt.storage_dir);
        assert!(events.iter().any(|e| e.phase == "skipped"
            && e.meta.as_ref().unwrap()["reason"] == "already_accepted_revision"));
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
    async fn speculative_cache_reads_are_not_delivery_acknowledgments() {
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        transport.push(FakeResponse::json(
            200,
            json!({"results":[{"text":"Use the agreed cache design"}]}),
        ));
        let rt = runtime(
            dir.path(),
            &transport,
            Some(json!({"readPolicy":"event","writePolicy":"manual"})),
            false,
        );
        rt.ensure_loaded();
        let session = rt.ensure_session_id("host1");
        rt.schedule_prefetch("cache design".into(), session.clone());
        rt.schedule_prefetch("cache design".into(), session);
        rt.wait_for_background().await;
        assert_eq!(transport.sent().len(), 1);
        let dataset = rt.active_dataset();
        let generation = rt.generation();
        let mut seen = SeenEvidence::default();
        let first = rt
            .state()
            .evidence_cache
            .take(&dataset, generation, NOW, &mut seen);
        let second = rt
            .state()
            .evidence_cache
            .take(&dataset, generation, NOW, &mut seen);
        assert_eq!(first, second);
        assert_eq!(first.len(), 1);
        // The actual host acknowledgment behavior is exercised by the
        // request_boundary_* tests, including failed writes and ten followups.
        assert_eq!(transport.sent().len(), 1);
    }

    #[tokio::test]
    async fn suppression_recorded_after_prefetch_filters_cached_delivery() {
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        transport.push(FakeResponse::json(
            200,
            json!({"results":[{"text":"Use the old cache design"}]}),
        ));
        let rt = runtime(
            dir.path(),
            &transport,
            Some(json!({"readPolicy":"event", "writePolicy":"manual"})),
            false,
        );
        rt.ensure_loaded();
        let session = rt.ensure_session_id("host1");
        rt.schedule_prefetch("cache design".into(), session);
        rt.wait_for_background().await;
        rt.record_suppression(Suppression {
            id: uuid::Uuid::new_v4().to_string(),
            hash: super::super::suppress::content_hash("Use the old cache design"),
            kind: super::super::suppress::SuppressionKind::Superseded {
                by: "replacement".into(),
            },
            dataset: rt.active_dataset(),
            branch: None,
            recorded_at_ms: NOW,
        })
        .unwrap();
        assert!(memory(&rt, "hello there", None).await.is_none());
    }

    #[tokio::test]
    async fn prefetch_respects_policy_circuit_and_shutdown() {
        for policy in ["legacy", "on_demand", "event"] {
            let dir = tempfile::tempdir().unwrap();
            let transport = Arc::new(FakeTransport::default());
            let rt = runtime(
                dir.path(),
                &transport,
                Some(json!({"readPolicy":policy, "writePolicy":"manual"})),
                false,
            );
            rt.ensure_loaded();
            if policy == "event" {
                rt.state().recall_circuit.open_until = wall_ms() + 60_000;
            }
            rt.schedule_prefetch("query".into(), rt.ensure_session_id("host1"));
            rt.wait_for_background().await;
            assert!(transport.sent().is_empty());
        }
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        transport.push(FakeResponse::Ok {
            status: 200,
            headers: vec![],
            chunks: vec![b"[]".to_vec()],
            delay_ms: 30_000,
        });
        let rt = runtime(
            dir.path(),
            &transport,
            Some(json!({"readPolicy":"event", "writePolicy":"manual"})),
            false,
        );
        rt.ensure_loaded();
        rt.schedule_prefetch("query".into(), rt.ensure_session_id("host1"));
        while transport.sent().is_empty() {
            tokio::task::yield_now().await;
        }
        tokio::time::timeout(std::time::Duration::from_secs(1), rt.session_shutdown())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn injected_clock_expires_cached_results_without_requests() {
        use std::sync::atomic::{AtomicU64, Ordering};
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        transport.push(FakeResponse::json(
            200,
            json!({"results":[{"text":"expired decision"}]}),
        ));
        transport.push(FakeResponse::json(200, json!({"results":[]})));
        let clock = Arc::new(AtomicU64::new(NOW));
        let clock_read = clock.clone();
        let rt = runtime_with_clock(
            dir.path(),
            &transport,
            Some(json!({"readPolicy":"event", "writePolicy":"manual"})),
            false,
            Arc::new(move || clock_read.load(Ordering::Relaxed)),
        );
        rt.ensure_loaded();
        let session = rt.ensure_session_id("host1");
        rt.schedule_prefetch("positive".into(), session.clone());
        rt.wait_for_background().await;
        clock.store(NOW + 300_000, Ordering::Relaxed);
        assert!(memory(&rt, "ordinary prompt", None).await.is_none());
        assert_eq!(transport.sent().len(), 1);
        rt.schedule_prefetch("negative".into(), session);
        rt.wait_for_background().await;
        clock.store(NOW + 360_000, Ordering::Relaxed);
        assert!(memory(&rt, "another ordinary prompt", None).await.is_none());
        assert_eq!(transport.sent().len(), 2);
    }

    #[tokio::test]
    async fn context_generation_tracks_identity_but_not_compaction_or_repeated_start() {
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        let rt = runtime(
            dir.path(),
            &transport,
            Some(json!({"readPolicy":"event", "writePolicy":"manual", "dataset":"old"})),
            false,
        );
        rt.ensure_loaded();
        let initial = rt.generation();
        rt.session_start_for("host1").await;
        let first = rt.generation();
        assert!(first > initial);
        rt.session_start_for("host1").await;
        assert_eq!(rt.generation(), first);
        rt.ensure_session_id("host2");
        assert!(rt.generation() > first);
        let resumed = rt.generation();
        rt.session_start_for("host3").await;
        assert!(rt.generation() > resumed);
        let before_compaction = rt.generation();
        {
            let mut state = rt.state();
            let key = coordinator::EvidenceCache::key("old", "cached", before_compaction);
            assert!(state.evidence_cache.schedule(&key, NOW));
            state.evidence_cache.finish(
                key,
                vec![RecallLine {
                    text: "cached".into(),
                    score: None,
                    provenance: None,
                }],
                NOW,
            );
        }
        let entry = CompactionEntry {
            base: base("compaction", 1),
            summary: "A compacted summary".into(),
            retained_tail: Vec::new(),
            tokens_before: 10,
            details: None,
            usage: None,
        };
        rt.after_compact(&AfterCompactEvent {
            entry: &entry,
            trigger: CompactionTrigger::Threshold,
            from_extension: false,
            session_id: "host3",
        })
        .await;
        assert_eq!(rt.generation(), before_compaction);
        assert_eq!(
            rt.state()
                .evidence_cache
                .take("old", before_compaction, NOW, &mut SeenEvidence::default(),)
                .len(),
            1
        );
        let mut config = rt.config();
        config.dataset = "new".into();
        assert!(rt.persist_config(config).await);
        assert!(rt.generation() > before_compaction);
        let before_reload = rt.generation();
        let config = rt.config();
        assert!(rt.persist_config(config).await);
        assert!(rt.generation() > before_reload);
        rt.wait_for_background().await;
    }

    #[tokio::test]
    async fn repeated_session_start_does_not_make_a_blind_recall() {
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        let rt = runtime(
            dir.path(),
            &transport,
            Some(json!({"readPolicy":"event", "writePolicy":"manual"})),
            false,
        );
        rt.session_start_for("host1").await;
        rt.session_start_for("host1").await;
        rt.wait_for_background().await;
        assert_eq!(
            transport
                .sent()
                .iter()
                .filter(|request| request.url.ends_with("/api/v1/recall"))
                .count(),
            0
        );
    }

    #[tokio::test]
    async fn compaction_does_not_use_summary_as_an_unconditional_query() {
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        transport.push(FakeResponse::json(200, json!({"results":[]})));
        let rt = runtime(
            dir.path(),
            &transport,
            Some(json!({"readPolicy":"event", "writePolicy":"manual"})),
            false,
        );
        let mut entry = CompactionEntry {
            base: base("compaction", 1),
            summary: "  Project constraints and decisions  ".into(),
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
        assert!(transport.sent().is_empty());
        assert_eq!(
            rt.state().last_compact_summary.as_deref(),
            Some("Project constraints and decisions")
        );
        entry.summary = "  ".into();
        rt.after_compact(&AfterCompactEvent {
            entry: &entry,
            trigger: CompactionTrigger::Threshold,
            from_extension: false,
            session_id: "host1",
        })
        .await;
        rt.wait_for_background().await;
        assert!(transport.sent().is_empty());
    }

    #[tokio::test]
    async fn session_change_discards_stale_prefetch_generation() {
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        transport.push(FakeResponse::Ok {
            status: 200,
            headers: vec![],
            chunks: vec![br#"{"results":[{"text":"stale decision"}]}"#.to_vec()],
            delay_ms: 100,
        });
        let rt = runtime(
            dir.path(),
            &transport,
            Some(json!({"readPolicy":"event", "writePolicy":"manual", "dataset":"old"})),
            false,
        );
        rt.ensure_loaded();
        rt.schedule_prefetch("query".into(), rt.ensure_session_id("host1"));
        while transport.sent().is_empty() {
            tokio::task::yield_now().await;
        }
        let scheduled_generation = rt.generation();
        rt.ensure_session_id("host2");
        assert!(rt.generation() > scheduled_generation);
        rt.wait_for_background().await;
        assert!(memory(&rt, "ordinary prompt", None).await.is_none());
        let generation = rt.generation();
        assert_eq!(
            rt.state()
                .evidence_cache
                .take("old", generation, NOW, &mut SeenEvidence::default())
                .len(),
            0
        );
        let observations = super::super::observer::read_observations(&rt.storage_dir);
        assert!(observations.iter().any(|observation| {
            observation.operation == "recall"
                && observation
                    .meta
                    .as_ref()
                    .and_then(|meta| meta.get("reason"))
                    == Some(&json!("stale_generation"))
        }));
        assert_eq!(transport.sent().len(), 1);
    }

    #[tokio::test]
    async fn event_cache_and_network_share_budget() {
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        transport.push(FakeResponse::json(
            200,
            json!({"results":[{"text":"cached design decision"}]}),
        ));
        transport.push(FakeResponse::json(
            200,
            json!({"results":[{"text":"other historical decision"}]}),
        ));
        let rt = runtime(
            dir.path(),
            &transport,
            Some(json!({"readPolicy":"event", "writePolicy":"manual", "recallMaxChars":300})),
            false,
        );
        rt.ensure_loaded();
        rt.schedule_prefetch("query".into(), rt.ensure_session_id("host1"));
        rt.wait_for_background().await;
        let text = memory(&rt, "What did we decide earlier?", None)
            .await
            .unwrap();
        assert!(text.contains("cached design decision"));
        assert!(text.contains("other historical decision"));
        assert!(len16(&text) <= rt.config().recall_max_chars);
        assert!(text.contains("</ice-void-cognee-memory>"));
    }

    #[tokio::test]
    async fn automatic_budget_caps_large_checkpoint_and_recall() {
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        transport.push(FakeResponse::json(
            200,
            json!({"results":[{"text":"recall fact ".repeat(900)}]}),
        ));
        let rt = runtime(
            dir.path(),
            &transport,
            Some(json!({"readPolicy":"event", "writePolicy":"manual", "recallMaxChars":100_000})),
            false,
        );
        rt.state().last_compact_summary = Some("checkpoint ".repeat(900));
        let text = memory(&rt, "What did we decide earlier?", None)
            .await
            .unwrap();
        assert!(len16(&text) <= 3200);
        assert!(text.ends_with("</ice-void-cognee-memory>"));
        assert!(text.contains("</ice-void-last-compact>"));
        assert!(text.contains("…"));
    }

    #[tokio::test]
    async fn failed_capture_keeps_original_identity() {
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        transport.push(FakeResponse::Ok {
            status: 500,
            headers: Default::default(),
            chunks: vec![b"failure".to_vec()],
            delay_ms: 50,
        });
        let rt = runtime(dir.path(), &transport, Some(json!({})), false);
        rt.ensure_loaded();
        {
            let mut state = rt.state();
            state.session_id = Some(super::super::config::cognee_session_id("original-host"));
            state.host_session_id = Some("original-host".into());
            state.resolved_dataset = "original-dataset".into();
        }
        rt.store_entry(Map::new(), "answer").unwrap();
        tokio::task::yield_now().await;
        rt.ensure_session_id("new-host");
        rt.wait_for_background().await;
        let records = read_warmup_batch(&rt.warmup_dir());
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].dataset.as_deref(), Some("original-dataset"));
        let observations = super::super::observer::read_observations(&rt.storage_dir);
        let failed = observations
            .iter()
            .find(|o| o.operation == "remember_entry" && o.phase == "failed")
            .unwrap();
        let original_session = super::super::config::cognee_session_id("original-host");
        assert_eq!(records[0].session_id, original_session);
        assert_eq!(
            failed.session_id.as_deref(),
            Some(original_session.as_str())
        );
        assert_eq!(failed.dataset.as_deref(), Some("original-dataset"));
        assert_eq!(failed.agent_id.as_deref(), Some(original_session.as_str()));
    }

    #[tokio::test]
    async fn warmup_drain_is_single_flight() {
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        transport.push(FakeResponse::Ok {
            status: 200,
            headers: Vec::new(),
            chunks: vec![b"{}".to_vec()],
            delay_ms: 50,
        });
        let rt = runtime(dir.path(), &transport, Some(json!({})), false);
        rt.ensure_loaded();
        rt.buffer_warmup_for("original", "dataset", &Map::new());
        rt.drain_warmup();
        rt.drain_warmup();
        rt.wait_for_background().await;
        assert_eq!(transport.requests.lock().unwrap().len(), 1);
        assert!(read_warmup_batch(&rt.warmup_dir()).is_empty());
        let observations = super::super::observer::read_observations(&rt.storage_dir);
        let replay = observations
            .iter()
            .find(|o| o.operation == "warmup_replay")
            .unwrap();
        assert_eq!(replay.meta.as_ref().unwrap()["replayed"], 1);
        assert_eq!(replay.meta.as_ref().unwrap()["failed"], 0);
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
    async fn fallback_recall_keeps_provenance_and_scope() {
        let dir = tempfile::tempdir().unwrap();
        let main = dir.path().join("repo");
        let linked = main.join("worktree");
        let admin = main.join(".git/worktrees/feature");
        std::fs::create_dir_all(&admin).unwrap();
        std::fs::create_dir_all(&linked).unwrap();
        std::fs::write(admin.join("commondir"), "../..\n").unwrap();
        std::fs::write(
            linked.join(".git"),
            format!("gitdir: {}\n", admin.display()),
        )
        .unwrap();
        let transport = Arc::new(FakeTransport::default());
        transport.push(FakeResponse::json(200, json!({"results": []})));
        transport.push(FakeResponse::json(200, json!({"results": [
            {"text": "first fact", "metadata": {"id": "src", "sessionId": "sess", "createdAt": "now"}},
            {"text": "second fact"}
        ]})));
        let rt = runtime(
            &linked,
            &transport,
            Some(json!({"readPolicy": "event"})),
            false,
        );
        let injected = memory(&rt, "What did we decide about cache eviction?", None)
            .await
            .unwrap();
        assert!(
            injected.contains("first fact [source=src session=sess at=now]"),
            "{injected}"
        );
        assert!(injected.contains("second fact"));
        let sent = transport.sent();
        assert_eq!(sent.len(), 2);
        let first = json_body(&sent[0]);
        let second = json_body(&sent[1]);
        assert_eq!(
            second["datasets"],
            json!([super::super::config::legacy_project_dataset(&linked)])
        );
        assert_eq!(first["sessionId"], second["sessionId"]);
        assert_eq!(first["scope"], second["scope"]);
        rt.wait_for_background().await;
        let observations = super::super::observer::read_observations(&linked.join("ice-cognee"));
        let event = observations
            .iter()
            .find(|event| event.operation == "recall" && event.phase == "succeeded")
            .unwrap();
        let meta = event.meta.as_ref().unwrap();
        assert_eq!(meta["legacyFallback"], true);
        assert_eq!(meta["legacyHits"], 2);
        assert_eq!(meta["withProvenance"], 1);
        assert_eq!(meta["selected"], 2);
    }

    #[tokio::test]
    async fn fallback_is_bypassed_for_hits_errors_explicit_dataset_and_legacy_policy() {
        for (config, response) in [
            (
                json!({"readPolicy":"event"}),
                FakeResponse::json(200, json!({"results":[{"text":"fact"}]})),
            ),
            (
                json!({"readPolicy":"event"}),
                FakeResponse::json(500, json!({"error":"failed"})),
            ),
            (
                json!({"readPolicy":"event", "dataset":"explicit"}),
                FakeResponse::json(200, json!({"results":[]})),
            ),
            (
                json!({"readPolicy":"legacy"}),
                FakeResponse::json(200, json!({"results":[]})),
            ),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let main = dir.path().join("repo");
            let linked = main.join("linked");
            let admin = main.join(".git/worktrees/test");
            std::fs::create_dir_all(&admin).unwrap();
            std::fs::create_dir_all(&linked).unwrap();
            std::fs::write(admin.join("commondir"), "../..").unwrap();
            std::fs::write(linked.join(".git"), format!("gitdir: {}", admin.display())).unwrap();
            let transport = Arc::new(FakeTransport::default());
            transport.push(response);
            let rt = runtime(&linked, &transport, Some(config), false);
            let _ = memory(&rt, "What did we decide about cache eviction?", None).await;
            assert_eq!(transport.sent().len(), 1);
        }
    }

    #[tokio::test]
    async fn prompt_recall_injects_memory_and_deduplicates() {
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        transport.push(FakeResponse::json(
            200,
            json!({"results": [{"text": "Cache uses LRU eviction", "score": 0.9}]}),
        ));
        let rt = runtime(
            dir.path(),
            &transport,
            Some(json!({"readPolicy":"event", "writePolicy":"manual"})),
            false,
        );
        let prompt = "What did we decide earlier about cache eviction?";
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
        assert_eq!(reasons, ["duplicate", "no_history_request"]);
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
        assert_eq!(metadata["rejected"]["routine"], 0);
        assert_eq!(metadata["rejected"]["duplicate"], 0);
        assert!(metadata["injectedTokens"].as_u64().is_some());
        assert_eq!(metadata["budgetTokens"], 800);
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
        assert!(observations.iter().any(|event| {
            event
                .meta
                .as_ref()
                .and_then(|m| m.get("outcome"))
                .and_then(Value::as_str)
                == Some("stale")
        }));
    }

    #[tokio::test]
    async fn doctor_reports_unverified_contract_without_endpoint_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        let rt = runtime(dir.path(), &transport, Some(json!({})), true);
        rt.ensure_loaded();
        let mut client = rt.state().client.clone().unwrap();
        client.config.api_key = Some("private-api-key".into());
        transport.push(FakeResponse::json(200, json!({"results": []})));
        client
            .recall("query", RecallOptions::default())
            .await
            .unwrap();
        transport.push(FakeResponse::json(404, json!({})));
        client
            .improve(None, &[], CallOptions::default())
            .await
            .unwrap_err();
        let mut report = rt.collect_doctor().await;
        report.base_url = "http://user:secret@localhost:8211/path?token=secret".into();
        let doctor = report.doctor_line();
        assert!(doctor.contains("helper=off requests=0 abstains=0 skips=0"));
        assert!(doctor.contains("nativeRecall=POST:/api/v1/recall (support unverified;"));
        assert!(doctor.contains("sends query, topK, onlyContext, scope, sessionId, datasets"));
        assert!(doctor.contains("nativeRemember=POST:/api/v1/remember (multipart"));
        assert!(doctor.contains("nativeRememberEntry=POST:/api/v1/remember/entry"));
        assert!(doctor.contains("sends datasetName, sessionIds, runInBackground"));
        assert!(doctor.contains("completion status: not exposed by this adapter"));
        assert!(doctor.contains("mcpCognee=separate catalog (not probed)"));
        assert!(doctor.contains("operations=recall:ok(1),remember:never_called,remember_entry:never_called,improve:not_found(1)"));
        assert!(!doctor.contains("secret"));
        assert!(!doctor.contains("private-api-key"));
        assert!(!report.status_line().contains("secret"));
        assert_eq!(transport.sent().len(), 2);
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
    async fn proposal_tool_rejects_invalid_valid_hours() {
        use ice_agent::agent_tool::AgentTool;

        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        let rt = runtime(
            dir.path(),
            &transport,
            Some(json!({"writePolicy":"selective"})),
            false,
        );
        let tool = super::super::propose::CogneeProposeMemoryTool::new(Arc::downgrade(&rt));
        for valid_hours in [json!(0), json!(8761), json!(-1), json!(1.5), json!("2")] {
            let error = tool
                .execute(
                    "call",
                    json!({
                        "text": "temporary fact",
                        "category": "preference",
                        "source_entry_id": "source",
                        "valid_hours": valid_hours
                    }),
                    CancellationToken::new(),
                    Arc::new(|_| {}),
                )
                .await
                .unwrap_err();
            assert!(matches!(error, ice_agent::AgentError::Validation(_)));
        }
        assert!(transport.sent().is_empty());
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
    async fn event_read_policy_searches_only_for_history_requests() {
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        transport.push(FakeResponse::json(
            200,
            json!({"results": [{"text": "Cache uses LRU eviction", "score": 0.9}]}),
        ));
        let rt = runtime(
            dir.path(),
            &transport,
            Some(json!({"readPolicy": "event"})),
            false,
        );
        for prompt in [
            "Build the release binary and run the whole test suite",
            "How many commits are on this branch right now?",
            "continue",
        ] {
            assert_eq!(memory(&rt, prompt, None).await, None, "{prompt}");
        }
        assert!(transport.sent().is_empty(), "ordinary turns send nothing");
        let injected = memory(&rt, "What did we decide about cache eviction?", None)
            .await
            .unwrap();
        assert!(injected.contains("Cache uses LRU eviction"), "{injected}");
        assert_eq!(transport.sent().len(), 1);
    }

    #[tokio::test]
    async fn recall_result_is_dropped_when_session_switches_mid_flight() {
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        transport.push(FakeResponse::Ok {
            status: 200,
            headers: vec![],
            chunks: vec![br#"{"results":[{"text":"old session decision"}]}"#.to_vec()],
            delay_ms: 100,
        });
        let rt = runtime(
            dir.path(),
            &transport,
            Some(json!({"readPolicy":"event", "writePolicy":"manual"})),
            false,
        );
        let task = {
            let rt = rt.clone();
            tokio::spawn(async move { memory(&rt, "What did we decide earlier?", None).await })
        };
        while transport.sent().is_empty() {
            tokio::task::yield_now().await;
        }
        rt.ensure_session_id("host2");
        assert!(task.await.unwrap().is_none());
        rt.wait_for_background().await;
        let observations = super::super::observer::read_observations(&rt.storage_dir);
        assert!(observations.iter().any(|observation| {
            observation.operation == "recall"
                && observation.meta.as_ref().is_some_and(|meta| {
                    meta.get("reason") == Some(&json!("stale_session"))
                        && meta.get("staleGeneration") == Some(&json!(true))
                })
        }));
    }

    #[tokio::test]
    async fn event_recall_injects_only_new_evidence_and_reinjects_after_compaction() {
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        let hit = |text: &str| json!({"results": [{"text": text, "score": 0.9}]});
        transport.push(FakeResponse::json(
            200,
            hit("bash succeeded\n- Cache uses LRU eviction\nread succeeded"),
        ));
        transport.push(FakeResponse::json(
            200,
            hit("- cache uses  LRU eviction\nbash succeeded"),
        ));
        transport.push(FakeResponse::json(
            200,
            hit("- Cache uses LRU eviction\n- TTL is five minutes"),
        ));
        transport.push(FakeResponse::json(200, hit("- Cache uses LRU eviction")));
        let rt = runtime(
            dir.path(),
            &transport,
            Some(json!({"readPolicy": "event"})),
            false,
        );
        let first = memory(&rt, "What did we decide about eviction?", None)
            .await
            .unwrap();
        assert!(first.contains("1. - Cache uses LRU eviction"), "{first}");
        assert!(!first.contains("succeeded"), "{first}");
        // The same fact under a different query is already in context.
        assert_eq!(
            memory(&rt, "Remind me what we decided on caching", None).await,
            None
        );
        let third = memory(&rt, "What else did we decide about the cache?", None)
            .await
            .unwrap();
        assert!(third.contains("TTL is five minutes"), "{third}");
        assert!(!third.contains("LRU"), "{third}");
        // After compaction the earlier injection may be gone: inject again.
        rt.state().seen_evidence.clear();
        let fourth = memory(&rt, "Did we choose LRU for the cache?", None)
            .await
            .unwrap();
        assert!(fourth.contains("LRU"), "{fourth}");
    }

    #[tokio::test]
    async fn checkpoint_and_recall_share_one_budget_with_a_complete_envelope() {
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        transport.push(FakeResponse::json(
            200,
            json!({"results": [{"text": "x".repeat(5000), "score": 0.9}]}),
        ));
        let rt = runtime(
            dir.path(),
            &transport,
            Some(json!({"recallMaxChars": 1000})),
            false,
        );
        rt.ensure_loaded();
        rt.state().last_compact_summary = Some("y".repeat(5000));
        let text = memory(&rt, "How does the cache layer evict entries?", None)
            .await
            .unwrap();
        assert!(len16(&text) <= 1000, "{}", len16(&text));
        assert!(text.contains("<ice-void-last-compact>"));
        assert!(text.ends_with("</ice-void-cognee-memory>"), "{text}");
    }

    #[tokio::test]
    async fn on_demand_read_policy_never_searches_automatically() {
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        let rt = runtime(
            dir.path(),
            &transport,
            Some(json!({"readPolicy": "on_demand"})),
            false,
        );
        assert_eq!(
            memory(&rt, "What did we decide about cache eviction?", None).await,
            None
        );
        assert!(transport.sent().is_empty());
    }

    #[tokio::test]
    async fn manual_write_policy_disables_every_automatic_write() {
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        let rt = runtime(
            dir.path(),
            &transport,
            Some(json!({"writePolicy": "manual", "readPolicy": "on_demand"})),
            false,
        );
        memory(&rt, "hi there", None).await;
        rt.on_agent_event(&AgentEvent::ToolExecutionEnd {
            tool_call_id: "c1".into(),
            tool_name: "bash".into(),
            result: AgentToolResult::text("out"),
            is_error: false,
        });
        rt.on_agent_event(&AgentEvent::AgentEnd {
            messages: vec![assistant("Done answer")],
        });
        rt.on_run_settled();
        rt.wait_for_background().await;
        assert!(rt.run_improve("idle").is_none());
        assert!(rt.run_dream().is_none());
        assert!(transport.sent().is_empty(), "{:?}", transport.sent());
        // The explicit command still writes.
        transport.push(FakeResponse::json(200, json!({})));
        rt.manual_remember("user_context", "Prefer tabs").await;
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
    async fn explicitly_rejected_capture_is_buffered_and_replayed() {
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        let rt = runtime(dir.path(), &transport, Some(json!({})), false);
        memory(&rt, "hi there", None).await;
        // Explicit rejection can be replayed; ambiguous failures stay uncertain.
        transport.push(FakeResponse::json(401, json!({})));
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
    async fn inflight_records_recover_to_uncertain_and_require_explicit_flush() {
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        let rt = runtime(dir.path(), &transport, Some(json!({})), false);
        rt.ensure_loaded();
        let record = create_pending_remember("fact", "ice", "user_context", 1);
        let mut inflight = record.clone();
        inflight.state = PendingState::InFlight;
        enqueue_pending_remember(&rt.pending_dir(), &inflight, 64).unwrap();

        // Simulate a previous process stopping after writing the intent.
        assert_eq!(recover_inflight(&rt.pending_dir()), 1);
        assert_eq!(
            read_pending_remember(&rt.pending_dir())[0].state,
            PendingState::Uncertain
        );
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
        assert_eq!(drain_failure_state("server_error"), PendingState::Uncertain);
        assert_eq!(drain_failure_state("unreachable"), PendingState::Uncertain);
        assert_eq!(drain_failure_state("unknown"), PendingState::Uncertain);
        assert_eq!(drain_failure_state("auth_failed"), PendingState::Pending);
    }

    #[tokio::test]
    async fn manual_remember_persists_inflight_before_request_and_removes_on_success() {
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        transport.push(FakeResponse::Ok {
            status: 200,
            headers: Vec::new(),
            chunks: vec![b"{}".to_vec()],
            delay_ms: 100,
        });
        let rt = runtime(dir.path(), &transport, Some(json!({})), false);
        let request = {
            let rt = rt.clone();
            tokio::spawn(async move {
                rt.manual_remember_with("user_context", "persist first", CallOptions::default())
                    .await;
            })
        };
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while transport.sent().is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("transport observed the request");
        let records = read_pending_remember(&rt.pending_dir());
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].state, PendingState::InFlight);
        request.await.unwrap();
        assert!(read_pending_remember(&rt.pending_dir()).is_empty());
    }

    #[tokio::test]
    async fn manual_remember_failure_states_and_queue_full_are_safe() {
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        transport.push(FakeResponse::Fail);
        let rt = runtime(dir.path(), &transport, Some(json!({})), false);
        rt.manual_remember("user_context", "connection failure")
            .await;
        let records = read_pending_remember(&rt.pending_dir());
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].state, PendingState::Uncertain);

        let timeout_dir = tempfile::tempdir().unwrap();
        let timeout_transport = Arc::new(FakeTransport::default());
        timeout_transport.push(FakeResponse::Ok {
            status: 200,
            headers: Vec::new(),
            chunks: vec![b"{}".to_vec()],
            delay_ms: 100,
        });
        let timeout_rt = runtime(
            timeout_dir.path(),
            &timeout_transport,
            Some(json!({})),
            false,
        );
        timeout_rt
            .manual_remember_with(
                "user_context",
                "slow response",
                CallOptions {
                    timeout_ms: Some(10),
                    ..CallOptions::default()
                },
            )
            .await;
        let records = read_pending_remember(&timeout_rt.pending_dir());
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].state, PendingState::Uncertain);
        timeout_rt.drain_pending(false).await;
        assert_eq!(timeout_transport.sent().len(), 1);

        let full_dir = tempfile::tempdir().unwrap();
        let full_transport = Arc::new(FakeTransport::default());
        let full_rt = runtime(
            full_dir.path(),
            &full_transport,
            Some(json!({"queueLimit": 1})),
            false,
        );
        enqueue_pending_remember(
            &full_rt.pending_dir(),
            &create_pending_remember("existing", "ice", "user_context", 1),
            1,
        )
        .unwrap();
        full_rt
            .manual_remember("user_context", "cannot enqueue")
            .await;
        assert!(full_transport.sent().is_empty());
        assert!(full_rt
            .notices()
            .drain()
            .iter()
            .any(|(_, text)| text.contains("queue is full")));
    }

    #[tokio::test]
    async fn busy_manual_duplicate_does_not_rewrite_live_inflight_record() {
        let dir = tempfile::tempdir().unwrap();
        let holder_transport = Arc::new(FakeTransport::default());
        let busy_transport = Arc::new(FakeTransport::default());
        let holder = runtime(dir.path(), &holder_transport, Some(json!({})), false);
        let busy = runtime(dir.path(), &busy_transport, Some(json!({})), false);
        holder.ensure_loaded();
        let mut record = create_pending_remember("active work", "ice", "user_context", 1);
        record.state = PendingState::InFlight;
        enqueue_pending_remember(&holder.pending_dir(), &record, 64).unwrap();
        let _lease = DispatchLease::try_acquire(&holder.pending_dir())
            .unwrap()
            .expect("holder owns dispatch");

        busy.manual_remember("user_context", "new manual intent")
            .await;

        let records = read_pending_remember(&holder.pending_dir());
        assert_eq!(records.len(), 2);
        assert_eq!(
            records
                .iter()
                .find(|item| item.text == "active work")
                .unwrap()
                .state,
            PendingState::InFlight
        );
        assert!(holder_transport.sent().is_empty());
        assert!(busy_transport.sent().is_empty());
    }

    #[tokio::test]
    async fn pending_drain_is_single_flight() {
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        transport.push(FakeResponse::Ok {
            status: 200,
            headers: Vec::new(),
            chunks: vec![b"{}".to_vec()],
            delay_ms: 100,
        });
        let rt = runtime(dir.path(), &transport, Some(json!({})), false);
        rt.ensure_loaded();
        enqueue_pending_remember(
            &rt.pending_dir(),
            &create_pending_remember("single drain", "ice", "user_context", 1),
            64,
        )
        .unwrap();
        let first = {
            let rt = rt.clone();
            tokio::spawn(async move { rt.drain_pending(false).await })
        };
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while transport.sent().is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("first drain reached transport");
        rt.drain_pending(false).await;
        first.await.unwrap();
        assert_eq!(transport.sent().len(), 1);
        assert!(read_pending_remember(&rt.pending_dir()).is_empty());
    }

    #[tokio::test]
    async fn doctor_counts_inflight_with_uncertain_queue_records() {
        let dir = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::default());
        let rt = runtime(dir.path(), &transport, Some(json!({})), false);
        let mut inflight = create_pending_remember("inflight", "ice", "user_context", 1);
        inflight.state = PendingState::InFlight;
        let mut uncertain = create_pending_remember("uncertain", "ice", "user_context", 2);
        uncertain.state = PendingState::Uncertain;
        enqueue_pending_remember(&rt.pending_dir(), &inflight, 64).unwrap();
        enqueue_pending_remember(&rt.pending_dir(), &uncertain, 64).unwrap();
        assert_eq!(rt.collect_doctor().await.queue_uncertain, 2);
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
        super::super::command::run_command(&rt, "write selective").await;
        assert_eq!(rt.config().write_policy, WritePolicy::Selective);
        assert!(!rt.config().automatic_writes());
        super::super::command::run_command(&rt, "remember on extra").await;
        // `remember on extra` is literal text, so it was sent (and failed).
        let sent = transport.sent();
        assert_eq!(sent.len(), 1);
        let Body::Multipart(parts) = &sent[0].body else {
            panic!("multipart")
        };
        assert_eq!(parts[1].value, "user_context");
        assert_eq!(parts.last().unwrap().value, "on extra");
        let queued = read_pending_remember(&rt.pending_dir());
        assert_eq!(queued.len(), 1);
        // A generic transport error does not prove the server rejected the write.
        assert_eq!(queued[0].state, PendingState::Uncertain);
        super::super::command::run_command(&rt, "bogus").await;
        let notices = rt.notices().drain();
        assert!(notices
            .iter()
            .any(|(_, text)| text == "Cognee session capture: off"));
        assert!(notices
            .iter()
            .any(|(_, text)| text.contains("Cognee remember unreachable")
                && text.contains("uncertain")));
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
