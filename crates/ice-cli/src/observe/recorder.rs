//! The observation recorder. Mirrors TS `observe-recorder.ts`
//! (`startObserveRecorder`, OBS-004..OBS-010): a read-only projection of the
//! live session event stream into one bounded activity timeline with stable
//! run/item/thread identities, an optional synchronous durable sink and a
//! live fan-out for the dashboard.
//!
//! Rust differences, deliberately:
//!
//! - The recorder never touches the agent loop. The CLI feeds it from a
//!   `broadcast` subscription on a dedicated thread ([`super::attach`]), so a
//!   slow SQLite write cannot stall the agent, and a panic or error in a
//!   projection cannot reach it either.
//! - Live listeners are a `tokio::sync::broadcast` channel; a lagging
//!   listener loses events instead of blocking the recorder.
//! - Strings are bounded by Unicode scalar values, not UTF-16 code units.
//! - Snapshots return owned copies; TS shares item objects with the ring.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use ice_agent::types::{AgentToolResult, TextContentOrImage};
use ice_agent::{AgentEvent, AgentMessage};
use ice_ai::types::{AssistantMessage, Content, StopReason, UserContent};
use indexmap::IndexMap;
use tokio::sync::broadcast;

use super::types::*;
use crate::cognee::observer::Observation;

/// Maximum retained activity items. Mirrors TS `MAX_ITEMS`.
pub const MAX_ITEMS: usize = 500;
/// Per-string retention ceiling. Mirrors TS `MAX_PAYLOAD_CHARS` (counted in
/// chars here, UTF-16 code units in TS).
pub const MAX_PAYLOAD_CHARS: usize = 1_048_576;
/// Default header poll interval. Mirrors TS `pollMs` default.
pub const DEFAULT_POLL_MS: u64 = 1_000;
/// Fixed review-attempt denominator in goal summaries (TS `MAX_REVIEWS`).
const GOAL_REVIEW_LIMIT: u32 = 3;
/// Criteria hash prefix length in goal summaries.
const CRITERIA_HASH_PREFIX: usize = 12;
/// Live listener channel capacity.
const LISTENER_CAPACITY: usize = 1_024;

/// Durable sink for the projected run history. Mirrors TS
/// `ObservePersistence`: implementations are synchronous, and the recorder
/// isolates their failures from live delivery.
pub trait ObservePersistence: Send + Sync {
    fn record_run(&self, header: &ObserveHeader) -> Result<(), String>;
    fn record_event(
        &self,
        event: &ObserveRecorderEvent,
        header: &ObserveHeader,
    ) -> Result<(), String>;
    fn finish_run(&self, run_id: &str, ended_at: i64) -> Result<(), String>;
    fn close(&self) -> Result<(), String>;
}

/// Session facts the CLI samples for the header. Mirrors the
/// `ObserveSessionLike` getters the TS recorder polls.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ObserveSessionInfo {
    pub session_name: Option<String>,
    /// `provider/id`.
    pub model: Option<String>,
    pub thinking_level: Option<String>,
    /// The model's context window; `0` or `None` means unknown.
    pub context_window: Option<u64>,
}

/// Options for [`ObserveRecorder::new`].
pub struct ObserveRecorderOptions {
    pub session_id: String,
    pub info: ObserveSessionInfo,
    pub persistence: Option<Arc<dyn ObservePersistence>>,
}

struct Inner {
    header: ObserveHeader,
    main: ObserveThreadIdentity,
    items: VecDeque<ObserveItem>,
    threads: IndexMap<String, ObserveThread>,
    lanes: IndexMap<String, ObserveLane>,
    next_item_id: u64,
    next_child_color: usize,
    /// Exact dedup summaries for goal/verifier/child transitions.
    transitions: HashMap<String, String>,
    /// In-flight Cognee requests: request id to the started item's id, so
    /// the terminal phase updates that item.
    pending_memory: IndexMap<String, u64>,
    /// Tokens of the latest assistant usage, for context occupancy.
    last_context_tokens: Option<i64>,
    context_window: Option<u64>,
    last_persisted_header: Option<String>,
    persistence: Option<Arc<dyn ObservePersistence>>,
    stopped: bool,
}

/// The recorder handle. Cheap to clone; clones share one projection.
#[derive(Clone)]
pub struct ObserveRecorder {
    inner: Arc<Mutex<Inner>>,
    tx: broadcast::Sender<ObserveRecorderEvent>,
}

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Bound one retained string. Mirrors TS `limitPayload`: a string over the
/// ceiling keeps its head plus an omitted-count suffix.
pub fn limit_payload(value: String) -> String {
    let count = value.chars().count();
    if count <= MAX_PAYLOAD_CHARS {
        return value;
    }
    let head: String = value.chars().take(MAX_PAYLOAD_CHARS).collect();
    format!("{head}\n… [{} chars omitted]", count - MAX_PAYLOAD_CHARS)
}

/// Two-space JSON, bounded. Mirrors TS `safeJson` (serde cannot fail on a
/// `Value`, so there is no `String(value)` fallback).
fn safe_json(value: &serde_json::Value) -> String {
    limit_payload(serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string()))
}

fn prefix_chars(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

fn main_thread_name(session_id: &str, session_name: Option<&str>) -> String {
    match session_name.map(str::trim).filter(|s| !s.is_empty()) {
        Some(name) => name.to_string(),
        None => format!("main · {}", prefix_chars(session_id, 10)),
    }
}

/// Parse a Rust skill invocation envelope
/// (`ice_harness::skills::format_skill_invocation`): returns the skill name
/// and the trailing user message, if any.
pub fn parse_skill_block(text: &str) -> Option<(String, String)> {
    let rest = text.strip_prefix("<skill name=\"")?;
    let name_end = rest.find('"')?;
    let name = &rest[..name_end];
    if name.is_empty() {
        return None;
    }
    let close = rest.find("</skill>")?;
    let user = rest[close + "</skill>".len()..].trim().to_string();
    Some((name.to_string(), user))
}

fn content_text(blocks: &[Content]) -> String {
    blocks
        .iter()
        .filter_map(|block| match block {
            Content::Text(t) => Some(t.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn user_text(content: &UserContent) -> String {
    match content {
        UserContent::Text(s) => s.clone(),
        UserContent::Blocks(blocks) => content_text(blocks),
    }
}

/// Split a tool result into visible text and forensic raw metadata. Mirrors
/// TS `resultText` + `resultRaw`: text blocks become the result, everything
/// else (details, images) the raw copy. Image bytes are not retained, only
/// their type and size.
fn tool_result_parts(result: &AgentToolResult) -> (String, Option<String>) {
    let mut texts = Vec::new();
    let mut others = Vec::new();
    for block in &result.content {
        match block {
            TextContentOrImage::Text(t) => texts.push(t.text.as_str()),
            TextContentOrImage::Image(image) => others.push(serde_json::json!({
                "type": "image",
                "mimeType": image.mime_type,
                "dataLength": image.data.len(),
            })),
        }
    }
    let has_details = !result.details.is_null();
    let text = if texts.is_empty() {
        safe_json(&serde_json::json!({ "content": others, "details": result.details }))
    } else {
        limit_payload(texts.join("\n"))
    };
    let raw = if others.is_empty() && !has_details {
        None
    } else {
        let mut raw = serde_json::Map::new();
        if !others.is_empty() {
            raw.insert("content".into(), serde_json::Value::Array(others));
        }
        if has_details {
            raw.insert("details".into(), result.details.clone());
        }
        Some(safe_json(&serde_json::Value::Object(raw)))
    };
    (text, raw)
}

fn json_str<'a>(value: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(serde_json::Value::as_str)
}

fn json_num(value: &serde_json::Value, key: &str) -> Option<f64> {
    value
        .get(key)
        .and_then(serde_json::Value::as_f64)
        .filter(|n| n.is_finite())
}

fn json_id(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Null => None,
        other => Some(other.to_string()),
    }
}

fn fmt_num(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

/// Count `(complete, total)` across `tasks` and nested `subtasks`. Mirrors TS
/// `countTasks`.
fn count_tasks(tasks: Option<&serde_json::Value>) -> (u64, u64) {
    let mut complete = 0;
    let mut total = 0;
    if let Some(serde_json::Value::Array(list)) = tasks {
        for task in list {
            if task.is_object() {
                total += 1;
                if json_str(task, "status") == Some("complete") {
                    complete += 1;
                }
                let (c, t) = count_tasks(task.get("subtasks"));
                complete += c;
                total += t;
            }
        }
    }
    (complete, total)
}

fn goal_summary(goal: &serde_json::Value) -> String {
    let status = json_str(goal, "status").unwrap_or("unknown");
    let mut parts = vec![format!("goal {status}")];
    if goal.get("approved") == Some(&serde_json::Value::Bool(true)) {
        parts.push("approved".into());
    }
    let (complete, total) = count_tasks(goal.get("tasks"));
    parts.push(format!("tasks {complete}/{total}"));
    let runs_used = json_num(goal, "runsUsed")
        .map(fmt_num)
        .unwrap_or("0".into());
    let max_runs = json_num(goal, "maxRuns").map(fmt_num).unwrap_or("?".into());
    parts.push(format!("runs {runs_used}/{max_runs}"));
    let reviews = json_num(goal, "reviewAttempts")
        .map(fmt_num)
        .unwrap_or("0".into());
    parts.push(format!("reviews {reviews}/{GOAL_REVIEW_LIMIT}"));
    if let Some(hash) = json_str(goal, "criteriaHash") {
        parts.push(format!(
            "criteria {}",
            prefix_chars(hash, CRITERIA_HASH_PREFIX)
        ));
    }
    if let Some(reason) = json_str(goal, "pauseReason") {
        parts.push(format!("paused: {reason}"));
    }
    parts.join(" · ")
}

fn goal_state(status: Option<&str>) -> Option<ObserveItemState> {
    match status {
        Some("complete") => Some(ObserveItemState::Done),
        Some("active") => Some(ObserveItemState::Running),
        Some("blocked") | Some("budget_limited") => Some(ObserveItemState::Error),
        _ => None,
    }
}

/// Blank item fields; [`Inner::push`] assigns identity, id and timestamp.
fn item(kind: ObserveItemKind, event_type: &str) -> ObserveItem {
    ObserveItem {
        identity: ObserveThreadIdentity {
            thread_key: String::new(),
            thread_id: String::new(),
            parent_thread_id: None,
            main_thread_id: String::new(),
            thread_kind: ObserveThreadKind::Main,
            thread_name: String::new(),
            thread_color: ObserveThreadColor::Blue,
        },
        run_id: String::new(),
        id: 0,
        ts: 0,
        kind,
        event_type: Some(event_type.to_string()),
        role: None,
        text: None,
        raw: None,
        tool_call_id: None,
        tool_name: None,
        state: None,
        args: None,
        result: None,
        duration_ms: None,
        updates: None,
        delegation_tool_name: None,
    }
}

impl Inner {
    fn persist(&mut self, event: &ObserveRecorderEvent) {
        let Some(persistence) = self.persistence.clone() else {
            return;
        };
        if let ObserveRecorderEvent::Header { header } = event {
            let serialized = serde_json::to_string(header).unwrap_or_default();
            if self.last_persisted_header.as_deref() == Some(serialized.as_str()) {
                return;
            }
            self.last_persisted_header = Some(serialized);
        }
        if let Err(error) = persistence.record_event(event, &self.header) {
            tracing::debug!("observe: persist {} failed: {error}", event.type_tag());
        }
    }

    fn header_event(&self) -> ObserveRecorderEvent {
        ObserveRecorderEvent::Header {
            header: self.header.clone(),
        }
    }

    fn push(&mut self, mut fields: ObserveItem, identity: ObserveThreadIdentity) -> ObserveItem {
        self.next_item_id += 1;
        fields.identity = identity;
        fields.run_id = self.header.run_id.clone();
        fields.id = self.next_item_id;
        fields.ts = now_ms();
        self.items.push_back(fields.clone());
        while self.items.len() > MAX_ITEMS {
            self.items.pop_front();
        }
        fields
    }

    fn refresh_context(&mut self) {
        self.header.context = match self.context_window {
            Some(window) if window > 0 => Some(ObserveContextUsage {
                tokens: self.last_context_tokens,
                context_window: window,
                percent: self
                    .last_context_tokens
                    .map(|t| t as f64 / window as f64 * 100.0),
            }),
            _ => None,
        };
    }

    /// Resolve (or create) a child thread. Mirrors TS `childThread`: an
    /// existing key keeps its first name, role and color.
    fn child_thread(
        &mut self,
        child_id: &str,
        name: Option<&str>,
        role: Option<&str>,
    ) -> (ObserveThreadIdentity, Option<ObserveThread>) {
        let key = format!("{}::{child_id}", self.main.thread_key);
        if let Some(existing) = self.threads.get(&key) {
            return (existing.identity.clone(), None);
        }
        let color = CHILD_THREAD_COLORS[self.next_child_color % CHILD_THREAD_COLORS.len()];
        self.next_child_color += 1;
        let identity = ObserveThreadIdentity {
            thread_key: key.clone(),
            thread_id: child_id.to_string(),
            parent_thread_id: Some(self.main.thread_id.clone()),
            main_thread_id: self.main.thread_id.clone(),
            thread_kind: ObserveThreadKind::Subagent,
            thread_name: name
                .filter(|n| !n.trim().is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| prefix_chars(child_id, 10)),
            thread_color: color,
        };
        let thread = ObserveThread {
            identity: identity.clone(),
            run_id: Some(self.header.run_id.clone()),
            role: role.map(str::to_string),
            profile_color: None,
        };
        self.threads.insert(key, thread.clone());
        (identity, Some(thread))
    }
}

impl ObserveRecorder {
    /// Start a recorder. Mirrors TS `startObserveRecorder` minus the session
    /// subscription, which the CLI performs (see [`super::attach`]).
    pub fn new(options: ObserveRecorderOptions) -> Self {
        let session_id = options.session_id;
        let started_at = now_ms();
        let run_id = format!("{session_id}-{started_at}-{}", uuid::Uuid::new_v4());
        let name = main_thread_name(&session_id, options.info.session_name.as_deref());
        let main = ObserveThreadIdentity {
            thread_key: session_id.clone(),
            thread_id: session_id.clone(),
            parent_thread_id: None,
            main_thread_id: session_id.clone(),
            thread_kind: ObserveThreadKind::Main,
            thread_name: name.clone(),
            thread_color: ObserveThreadColor::Blue,
        };
        let header = ObserveHeader {
            run_id: run_id.clone(),
            session_id: session_id.clone(),
            main_thread_id: session_id.clone(),
            main_thread_name: name,
            session_name: options.info.session_name.clone(),
            model: options.info.model.clone(),
            thinking_level: options.info.thinking_level.clone(),
            context: None,
            usage: ObserveUsage::default(),
            started_at,
        };
        let main_thread = ObserveThread {
            identity: main.clone(),
            run_id: Some(run_id),
            role: None,
            profile_color: None,
        };
        let mut threads = IndexMap::new();
        threads.insert(main.thread_key.clone(), main_thread.clone());
        let mut inner = Inner {
            header,
            main,
            items: VecDeque::new(),
            threads,
            lanes: IndexMap::new(),
            next_item_id: 0,
            next_child_color: 0,
            transitions: HashMap::new(),
            pending_memory: IndexMap::new(),
            last_context_tokens: None,
            context_window: options.info.context_window,
            last_persisted_header: None,
            persistence: options.persistence,
            stopped: false,
        };
        inner.refresh_context();
        if let Some(persistence) = inner.persistence.clone() {
            if let Err(error) = persistence.record_run(&inner.header) {
                tracing::debug!("observe: record_run failed: {error}");
            }
            inner.persist(&ObserveRecorderEvent::Thread {
                thread: main_thread,
            });
        }
        let (tx, _) = broadcast::channel(LISTENER_CAPACITY);
        Self {
            inner: Arc::new(Mutex::new(inner)),
            tx,
        }
    }

    /// Subscribe to live projection changes. Mirrors TS `subscribe`.
    pub fn subscribe(&self) -> broadcast::Receiver<ObserveRecorderEvent> {
        self.tx.subscribe()
    }

    pub fn get_header(&self) -> ObserveHeader {
        self.lock().header.clone()
    }

    pub fn get_state(&self) -> ObserveState {
        let inner = self.lock();
        ObserveState {
            header: inner.header.clone(),
            items: inner.items.iter().cloned().collect(),
            lanes: inner.lanes.values().cloned().collect(),
            threads: inner.threads.values().cloned().collect(),
        }
    }

    pub fn run_id(&self) -> String {
        self.lock().header.run_id.clone()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Persist (optionally) and then fan out, mirroring TS `emit`: durable
    /// writes are attempted before live delivery and their failures swallowed.
    fn emit(&self, inner: &mut Inner, event: ObserveRecorderEvent, persist: bool) {
        if persist {
            inner.persist(&event);
        }
        let _ = self.tx.send(event);
    }

    fn emit_header(&self, inner: &mut Inner) {
        let event = inner.header_event();
        self.emit(inner, event, true);
    }

    fn push_main(&self, inner: &mut Inner, fields: ObserveItem) -> ObserveItem {
        let identity = inner.main.clone();
        let item = inner.push(fields, identity);
        self.emit(
            inner,
            ObserveRecorderEvent::Activity { item: item.clone() },
            true,
        );
        item
    }

    /// Update sampled session facts; emits a header only when it changed.
    /// Mirrors the TS 1 s header poll.
    pub fn update_session_info(&self, info: ObserveSessionInfo) {
        let mut inner = self.lock();
        if inner.stopped {
            return;
        }
        let before = inner.header.clone();
        inner.header.session_name = info.session_name.clone();
        inner.header.main_thread_name =
            main_thread_name(&inner.header.session_id, info.session_name.as_deref());
        inner.header.model = info.model;
        inner.header.thinking_level = info.thinking_level;
        inner.context_window = info.context_window;
        inner.refresh_context();
        if inner.header != before {
            if let Some(persistence) = inner.persistence.clone() {
                let _ = persistence.record_run(&inner.header);
            }
            self.emit_header(&mut inner);
        }
    }

    /// Project one agent event. Mirrors the TS session-event dispatcher.
    pub fn handle_agent_event(&self, event: &AgentEvent) {
        let mut inner = self.lock();
        if inner.stopped {
            return;
        }
        match event {
            AgentEvent::MessageStart { message } => match message {
                AgentMessage::User(user) => {
                    let text = user_text(&user.content);
                    let mut fields = match parse_skill_block(&text) {
                        Some((name, rest)) => {
                            let mut f = item(ObserveItemKind::Skill, "message_start");
                            f.text = Some(if rest.is_empty() {
                                format!("/{name}")
                            } else {
                                limit_payload(format!("/{name} {rest}"))
                            });
                            f
                        }
                        None => {
                            let mut f = item(ObserveItemKind::Message, "message_start");
                            f.text = Some(limit_payload(text));
                            f
                        }
                    };
                    fields.role = Some(ObserveRole::User);
                    fields.raw = serde_json::to_value(user).ok().map(|v| safe_json(&v));
                    self.push_main(&mut inner, fields);
                }
                AgentMessage::Custom(custom) => {
                    let kind = if custom.role.starts_with("ice-goals") {
                        ObserveItemKind::Goal
                    } else {
                        ObserveItemKind::Message
                    };
                    let mut fields = item(kind, "message_start");
                    fields.role = Some(ObserveRole::Custom);
                    fields.tool_name = Some(custom.role.clone());
                    fields.text = Some(limit_payload(content_text(&custom.content)));
                    fields.raw = Some(safe_json(&serde_json::json!({
                        "customType": custom.role,
                        "details": custom.data,
                    })));
                    self.push_main(&mut inner, fields);
                }
                _ => {}
            },
            AgentEvent::MessageEnd {
                message: AgentMessage::Assistant(assistant),
            } => self.assistant_end(&mut inner, assistant),
            AgentEvent::ToolExecutionStart {
                tool_call_id,
                tool_name,
                args,
            } => {
                let mut fields = item(ObserveItemKind::Tool, "tool_execution_start");
                fields.tool_call_id = Some(tool_call_id.clone());
                fields.tool_name = Some(tool_name.clone());
                fields.args = Some(safe_json(args));
                fields.state = Some(ObserveItemState::Running);
                fields.updates = Some(0);
                self.push_main(&mut inner, fields);
            }
            AgentEvent::ToolExecutionUpdate { tool_call_id, .. } => {
                if let Some(found) = find_main_tool(&mut inner, tool_call_id) {
                    found.updates = Some(found.updates.unwrap_or(0) + 1);
                    let item = found.clone();
                    // Progress is broadcast but not persisted (TS parity).
                    self.emit(
                        &mut inner,
                        ObserveRecorderEvent::ActivityUpdate { item },
                        false,
                    );
                }
            }
            AgentEvent::ToolExecutionEnd {
                tool_call_id,
                tool_name,
                result,
                is_error,
            } => {
                let (text, raw) = tool_result_parts(result);
                let state = if *is_error {
                    ObserveItemState::Error
                } else {
                    ObserveItemState::Done
                };
                let now = now_ms();
                if let Some(found) = find_main_tool(&mut inner, tool_call_id) {
                    found.state = Some(state);
                    found.event_type = Some("tool_execution_end".into());
                    found.result = Some(text);
                    if raw.is_some() {
                        found.raw = raw;
                    }
                    found.duration_ms = Some((now - found.ts).max(0));
                    let item = found.clone();
                    self.emit(
                        &mut inner,
                        ObserveRecorderEvent::ActivityUpdate { item },
                        true,
                    );
                } else {
                    let mut fields = item(ObserveItemKind::Tool, "tool_execution_end");
                    fields.tool_call_id = Some(tool_call_id.clone());
                    fields.tool_name = Some(tool_name.clone());
                    fields.state = Some(state);
                    fields.result = Some(text);
                    fields.raw = raw;
                    fields.duration_ms = Some(0);
                    self.push_main(&mut inner, fields);
                }
                self.emit_header(&mut inner);
            }
            AgentEvent::RetryScheduled {
                attempt,
                max_retries,
                delay_ms,
                error,
            } => {
                let mut fields = item(ObserveItemKind::Retry, "auto_retry_start");
                fields.text = Some(format!("attempt {attempt}/{max_retries} in {delay_ms}ms"));
                fields.state = Some(ObserveItemState::Running);
                fields.raw = Some(safe_json(&serde_json::json!({
                    "attempt": attempt,
                    "maxAttempts": max_retries,
                    "delayMs": delay_ms,
                    "errorMessage": error,
                })));
                self.push_main(&mut inner, fields);
            }
            AgentEvent::RetryRefused { error } => {
                // Rust-only: the retry was refused because replay could repeat
                // a tool effect. No success row is ever synthesized.
                let mut fields = item(ObserveItemKind::Retry, "auto_retry_refused");
                fields.text = Some(limit_payload(format!("retry refused: {error}")));
                fields.state = Some(ObserveItemState::Error);
                fields.raw = Some(safe_json(&serde_json::json!({ "errorMessage": error })));
                self.push_main(&mut inner, fields);
            }
            _ => {}
        }
    }

    fn assistant_end(&self, inner: &mut Inner, message: &AssistantMessage) {
        let usage = &message.usage;
        inner.header.usage.input += usage.input;
        inner.header.usage.output += usage.output;
        inner.header.usage.cache_read += usage.cache_read;
        inner.header.usage.cache_write += usage.cache_write;
        inner.header.usage.cost += usage.cost.total;
        let context_tokens = usage.input + usage.output + usage.cache_read + usage.cache_write;
        if context_tokens > 0 {
            inner.last_context_tokens = Some(context_tokens);
            inner.refresh_context();
        }

        let text = content_text(&message.content).trim().to_string();
        let failed = matches!(message.stop_reason, StopReason::Aborted | StopReason::Error);
        if !text.is_empty() || failed {
            let mut fields = item(ObserveItemKind::Message, "message_end");
            fields.role = Some(ObserveRole::Assistant);
            fields.text = Some(limit_payload(if failed {
                let mut prefix = format!("[{}]", message.stop_reason.as_str());
                if let Some(error) = &message.error_message {
                    prefix.push(' ');
                    prefix.push_str(error);
                }
                if text.is_empty() {
                    prefix
                } else {
                    format!("{prefix}\n{text}")
                }
            } else {
                text
            }));
            if failed {
                fields.state = Some(ObserveItemState::Error);
            }
            fields.raw = Some(safe_json(&ice_agent::message::assistant_json(message)));
            self.push_main(inner, fields);
        }
        self.emit_header(inner);
    }

    /// Project a compaction lifecycle edge. Rust's harness reports only
    /// start/finish (`CompactionProgressEvent`), so the end row carries no
    /// summary or retry/abort outcome.
    pub fn handle_compaction(&self, manual: bool, active: bool) {
        let mut inner = self.lock();
        if inner.stopped {
            return;
        }
        let reason = if manual { "manual" } else { "threshold" };
        let (event_type, text, state) = if active {
            (
                "compaction_start",
                format!("started ({reason})"),
                ObserveItemState::Running,
            )
        } else {
            (
                "compaction_end",
                format!("finished ({reason})"),
                ObserveItemState::Done,
            )
        };
        let mut fields = item(ObserveItemKind::Compaction, event_type);
        fields.text = Some(text);
        fields.state = Some(state);
        fields.raw = Some(safe_json(&serde_json::json!({ "reason": reason })));
        self.push_main(&mut inner, fields);
    }

    /// Project a queue snapshot. Mirrors TS `queue_update`.
    pub fn handle_queue_update(&self, steering: usize, follow_up: usize) {
        let mut inner = self.lock();
        if inner.stopped {
            return;
        }
        let mut fields = item(ObserveItemKind::Queue, "queue_update");
        fields.text = Some(format!("{steering} steering, {follow_up} follow-up"));
        fields.raw = Some(safe_json(
            &serde_json::json!({ "steering": steering, "followUp": follow_up }),
        ));
        self.push_main(&mut inner, fields);
    }

    /// Project an appended custom session entry. Mirrors the TS
    /// session-manager `entry_appended` dispatcher (OBS-009, OBS-010):
    /// `ice-goals-state`, `ice-safe-verify-state`, `ice-subagent-v2-child`
    /// and `ice-subagent-v2-completion`. Other custom types are ignored.
    pub fn handle_custom_entry(&self, custom_type: &str, data: &serde_json::Value) {
        if !data.is_object() {
            return;
        }
        let mut inner = self.lock();
        if inner.stopped {
            return;
        }
        match custom_type {
            "ice-goals-state" => self.goals_state(&mut inner, custom_type, data),
            "ice-safe-verify-state" => self.verifier_state(&mut inner, custom_type, data),
            "ice-subagent-v2-child" => self.child_record(&mut inner, custom_type, data),
            "ice-subagent-v2-completion" => self.child_completion(&mut inner, custom_type, data),
            _ => {}
        }
    }

    fn goals_state(&self, inner: &mut Inner, custom_type: &str, data: &serde_json::Value) {
        let goals: Vec<&serde_json::Value> = match data.get("goals") {
            Some(serde_json::Value::Array(list)) => list.iter().filter(|g| g.is_object()).collect(),
            _ => Vec::new(),
        };
        let mut present = std::collections::HashSet::new();
        for goal in goals {
            let Some(id) = goal.get("id").and_then(json_id) else {
                continue;
            };
            let key = format!("goal:{id}");
            present.insert(key.clone());
            let summary = goal_summary(goal);
            if inner.transitions.get(&key) == Some(&summary) {
                continue;
            }
            inner.transitions.insert(key, summary.clone());
            let mut fields = item(ObserveItemKind::Goal, custom_type);
            fields.tool_name = Some("goal".into());
            fields.text = Some(format!("{summary} · {id}"));
            fields.result = json_str(goal, "objective").map(|s| limit_payload(s.to_string()));
            fields.raw = Some(safe_json(goal));
            fields.state = goal_state(json_str(goal, "status"));
            self.push_main(inner, fields);
        }
        let cleared: Vec<String> = inner
            .transitions
            .keys()
            .filter(|k| k.starts_with("goal:") && !present.contains(*k))
            .cloned()
            .collect();
        for key in cleared {
            inner.transitions.remove(&key);
            let mut fields = item(ObserveItemKind::Goal, custom_type);
            fields.tool_name = Some("goal".into());
            fields.text = Some(format!("goal cleared · {}", &key["goal:".len()..]));
            self.push_main(inner, fields);
        }
    }

    fn verifier_state(&self, inner: &mut Inner, custom_type: &str, data: &serde_json::Value) {
        let Some(verifier) = data.get("verifier").filter(|v| v.is_object()) else {
            return;
        };
        let status = json_str(verifier, "status").unwrap_or("unknown");
        let mut parts = vec![format!("verifier {status}")];
        if let Some(code) = json_num(verifier, "exitCode") {
            parts.push(format!("exit {}", fmt_num(code)));
        }
        if let Some(mode) = data.get("mode").filter(|m| !m.is_null()) {
            let mode = mode
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| mode.to_string());
            parts.push(format!("mode {mode}"));
        }
        let summary = parts.join(" · ");
        if inner.transitions.get("verifier") == Some(&summary) {
            return;
        }
        inner.transitions.insert("verifier".into(), summary.clone());
        let output: Vec<&str> = ["failureMessage", "stdout", "stderr"]
            .iter()
            .filter_map(|k| json_str(verifier, k))
            .filter(|s| !s.trim().is_empty())
            .collect();
        let mut fields = item(ObserveItemKind::Goal, custom_type);
        fields.tool_name = Some("verify".into());
        fields.text = Some(summary);
        if !output.is_empty() {
            fields.result = Some(limit_payload(output.join("\n")));
        }
        fields.raw = Some(safe_json(verifier));
        fields.duration_ms = json_num(verifier, "durationMs").map(|n| n as i64);
        fields.state = match status {
            "passed" => Some(ObserveItemState::Done),
            "failed" => Some(ObserveItemState::Error),
            "running" => Some(ObserveItemState::Running),
            _ => None,
        };
        self.push_main(inner, fields);
    }

    fn push_child(
        &self,
        inner: &mut Inner,
        child_id: &str,
        name: Option<&str>,
        agent: Option<&str>,
        fields: ObserveItem,
    ) {
        let (identity, new_thread) = inner.child_thread(child_id, name, agent);
        if let Some(thread) = new_thread {
            self.emit(inner, ObserveRecorderEvent::Thread { thread }, true);
        }
        let item = inner.push(fields, identity);
        self.emit(inner, ObserveRecorderEvent::Activity { item }, true);
    }

    fn child_record(&self, inner: &mut Inner, custom_type: &str, record: &serde_json::Value) {
        let Some(child_id) = record
            .get("id")
            .or_else(|| record.get("childId"))
            .and_then(json_id)
        else {
            return;
        };
        let state = json_str(record, "state").unwrap_or("unknown");
        let outcome = json_str(record, "lastTurnOutcome");
        let mut parts = vec![state.to_string()];
        if let Some(outcome) = outcome {
            parts.push(outcome.to_string());
        }
        if let Some(patch) = json_str(record, "patchState") {
            parts.push(format!("patch {patch}"));
        }
        let summary = parts.join(" · ");
        let key = format!("child:{child_id}");
        if inner.transitions.get(&key) == Some(&summary) {
            return;
        }
        inner.transitions.insert(key, summary.clone());
        let agent = json_str(record, "agent");
        let mut fields = item(ObserveItemKind::Subagent, custom_type);
        fields.tool_name = Some(agent.unwrap_or("agent").to_string());
        fields.text = Some(match json_str(record, "description") {
            Some(description) => limit_payload(format!("{summary} — {description}")),
            None => summary,
        });
        fields.raw = Some(safe_json(record));
        fields.state = Some(
            if outcome.is_some_and(|o| o != "completed") || state == "blocked" {
                ObserveItemState::Error
            } else if matches!(state, "running" | "starting") {
                ObserveItemState::Running
            } else {
                ObserveItemState::Done
            },
        );
        let name = json_str(record, "name");
        self.push_child(inner, &child_id, name, agent, fields);
    }

    fn child_completion(&self, inner: &mut Inner, custom_type: &str, data: &serde_json::Value) {
        let Some(child_id) = data.get("childId").and_then(json_id) else {
            return;
        };
        let outcome = json_str(data, "outcome");
        let agent = json_str(data, "agent");
        let mut fields = item(ObserveItemKind::Subagent, custom_type);
        fields.tool_name = Some(agent.unwrap_or("agent").to_string());
        fields.text = Some(format!("completion {}", outcome.unwrap_or("unknown")));
        fields.result = json_str(data, "summary").map(|s| limit_payload(s.to_string()));
        fields.raw = Some(safe_json(data));
        fields.state = Some(if outcome == Some("completed") {
            ObserveItemState::Done
        } else {
            ObserveItemState::Error
        });
        self.push_child(inner, &child_id, None, agent, fields);
    }

    /// Project one Cognee memory operation. Mirrors TS `handleMemory`
    /// (OBS-008): a started request is a running item that its terminal
    /// phase (succeeded/failed) updates in place.
    pub fn handle_memory(&self, observation: &Observation) {
        let mut inner = self.lock();
        if inner.stopped {
            return;
        }
        let terminal = matches!(observation.phase.as_str(), "succeeded" | "failed");
        let state = match observation.phase.as_str() {
            "failed" => ObserveItemState::Error,
            "succeeded" => ObserveItemState::Done,
            _ => ObserveItemState::Running,
        };
        let raw = serde_json::to_value(observation)
            .map(|value| safe_json(&value))
            .unwrap_or_default();
        let request_id = observation.request_id.as_deref();
        if terminal {
            let pending = request_id.and_then(|id| inner.pending_memory.shift_remove(id));
            if let Some(found) = pending.and_then(|id| inner.items.iter_mut().find(|i| i.id == id))
            {
                found.state = Some(state);
                found.text = Some(memory_title(observation));
                found.result = Some(memory_result(observation));
                found.raw = Some(raw);
                found.duration_ms = Some(
                    observation
                        .latency_ms
                        .map(|ms| ms as i64)
                        .unwrap_or_else(|| (now_ms() - found.ts).max(0)),
                );
                let item = found.clone();
                self.emit(
                    &mut inner,
                    ObserveRecorderEvent::ActivityUpdate { item },
                    true,
                );
                return;
            }
        }
        let mut fields = item(ObserveItemKind::Memory, "cognee_observation");
        fields.tool_name = Some(observation.operation.clone());
        fields.state = Some(state);
        fields.text = Some(memory_title(observation));
        fields.result = Some(memory_result(observation));
        fields.raw = Some(raw);
        // A started request's preview is its input (the recall query).
        if !terminal {
            fields.args = observation.preview.clone().map(limit_payload);
        }
        fields.duration_ms = observation.latency_ms.map(|ms| ms as i64);
        let pushed = self.push_main(&mut inner, fields);
        if let (Some(id), false) = (request_id, terminal) {
            inner.pending_memory.insert(id.to_string(), pushed.id);
            // Requests that never finish must not grow this map forever.
            if inner.pending_memory.len() > MAX_ITEMS {
                inner.pending_memory.shift_remove_index(0);
            }
        }
    }

    /// Record a non-fatal observer note (for example a lagged event feed).
    pub fn note(&self, text: &str) {
        let mut inner = self.lock();
        if inner.stopped {
            return;
        }
        let mut fields = item(ObserveItemKind::Message, "observe_note");
        fields.role = Some(ObserveRole::Custom);
        fields.tool_name = Some("observe".into());
        fields.text = Some(text.to_string());
        fields.state = Some(ObserveItemState::Error);
        self.push_main(&mut inner, fields);
    }

    /// Finish the run and close persistence. Idempotent. Mirrors TS `stop`.
    pub fn stop(&self) {
        let mut inner = self.lock();
        if inner.stopped {
            return;
        }
        inner.stopped = true;
        if let Some(persistence) = inner.persistence.take() {
            let run_id = inner.header.run_id.clone();
            if let Err(error) = persistence.finish_run(&run_id, now_ms()) {
                tracing::debug!("observe: finish_run failed: {error}");
            }
            let _ = persistence.close();
        }
    }
}

/// `Cognee <operation>[ (compaction)] · dataset · model`. Mirrors TS
/// `memoryTitle`.
fn memory_title(observation: &Observation) -> String {
    let checkpoint = observation
        .meta
        .as_ref()
        .and_then(|meta| meta.get("checkpoint"))
        .and_then(serde_json::Value::as_bool)
        == Some(true);
    let outcome = observation.meta.as_ref().and_then(|meta| {
        (meta
            .get("outcomeVersion")
            .and_then(serde_json::Value::as_u64)
            == Some(1))
        .then(|| meta.get("outcome").and_then(serde_json::Value::as_str))
        .flatten()
    });
    let suffix = match (observation.operation.as_str(), outcome) {
        ("recall", Some("skipped")) => " · skipped (no request)",
        ("recall", Some("stale")) => " · discarded (session changed)",
        ("recall", Some("retrieved_empty")) => " · retrieved empty",
        ("recall", Some("retrieved_nonempty")) => " · retrieved nonempty",
        ("improve", Some("accepted")) => " · accepted (completion unknown)",
        _ => "",
    };
    let head = format!(
        "Cognee {}{}{}",
        observation.operation,
        if checkpoint { " (compaction)" } else { "" },
        suffix
    );
    [
        Some(head.as_str()),
        observation.dataset.as_deref(),
        observation.model.as_deref(),
    ]
    .into_iter()
    .flatten()
    .filter(|part| !part.is_empty())
    .collect::<Vec<_>>()
    .join(" · ")
}

/// Error, `key=value` meta and preview lines. Mirrors TS `memoryResult`.
fn memory_result(observation: &Observation) -> String {
    let mut lines = Vec::new();
    if let Some(error) = observation.error.as_deref().filter(|e| !e.is_empty()) {
        lines.push(format!("error: {error}"));
    }
    if let Some(meta) = observation.meta.as_ref().filter(|m| !m.is_empty()) {
        let pairs: Vec<String> = meta
            .iter()
            .map(|(key, value)| match value {
                serde_json::Value::String(s) => format!("{key}={s}"),
                other => format!("{key}={other}"),
            })
            .collect();
        lines.push(pairs.join(", "));
    }
    if let Some(preview) = observation.preview.as_deref().filter(|p| !p.is_empty()) {
        lines.push(preview.to_string());
    }
    limit_payload(lines.join("\n"))
}

/// Newest retained main-thread tool item for `tool_call_id`. Mirrors the TS
/// reverse search; an evicted start is not found.
fn find_main_tool<'a>(inner: &'a mut Inner, tool_call_id: &str) -> Option<&'a mut ObserveItem> {
    inner.items.iter_mut().rev().find(|item| {
        item.kind == ObserveItemKind::Tool
            && item.identity.thread_kind == ObserveThreadKind::Main
            && item.tool_call_id.as_deref() == Some(tool_call_id)
    })
}

#[cfg(test)]
#[path = "recorder_tests.rs"]
mod tests;
