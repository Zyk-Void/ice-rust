//! Observation DTOs. Mirrors the exported types of TS
//! `packages/coding-agent/src/observe/observe-recorder.ts` (OBS-004) and
//! `observe-store.ts` (OBS-013). Field names serialize in camelCase so the
//! stored payloads and the HTTP/SSE wire match the TS dashboard contract.

use serde::{Deserialize, Serialize};

/// Activity kind on the tape. Mirrors TS `ObserveItemKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ObserveItemKind {
    Message,
    Tool,
    Subagent,
    Compaction,
    Retry,
    Skill,
    Queue,
    Memory,
    Goal,
}

/// Mirrors TS `ObserveThreadKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ObserveThreadKind {
    Main,
    Subagent,
}

/// Mirrors TS `ObserveThreadColor`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ObserveThreadColor {
    Blue,
    Violet,
    Cyan,
    Green,
    Amber,
    Rose,
    Orange,
    Indigo,
    Slate,
}

/// Colors handed to child threads, in allocation order.
pub const CHILD_THREAD_COLORS: [ObserveThreadColor; 7] = [
    ObserveThreadColor::Violet,
    ObserveThreadColor::Cyan,
    ObserveThreadColor::Green,
    ObserveThreadColor::Amber,
    ObserveThreadColor::Rose,
    ObserveThreadColor::Orange,
    ObserveThreadColor::Indigo,
];

/// Mirrors TS `ObserveThreadIdentity`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObserveThreadIdentity {
    /// Stable path: `mainThreadId` for the root, `parentKey::childId` below it.
    pub thread_key: String,
    pub thread_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_thread_id: Option<String>,
    pub main_thread_id: String,
    pub thread_kind: ObserveThreadKind,
    pub thread_name: String,
    pub thread_color: ObserveThreadColor,
}

/// Mirrors TS `ObserveThread`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObserveThread {
    #[serde(flatten)]
    pub identity: ObserveThreadIdentity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_color: Option<String>,
}

/// Tool/goal state chip. Mirrors TS `ObserveItem["state"]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ObserveItemState {
    Running,
    Done,
    Error,
}

/// Mirrors TS `ObserveItem["role"]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ObserveRole {
    User,
    Assistant,
    Custom,
}

/// One activity on the tape. Mirrors TS `ObserveItem`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObserveItem {
    #[serde(flatten)]
    pub identity: ObserveThreadIdentity,
    #[serde(default)]
    pub run_id: String,
    pub id: u64,
    pub ts: i64,
    pub kind: ObserveItemKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<ObserveRole>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<ObserveItemState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updates: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delegation_tool_name: Option<String>,
}

/// Compatibility lane record. Mirrors TS `ObserveLane`. The recorder never
/// creates lanes from normal session events (neither does TS); the type exists
/// so snapshots, replay and the dashboard keep the same shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObserveLane {
    #[serde(flatten)]
    pub identity: ObserveThreadIdentity,
    pub stream_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    pub tool_name: String,
    pub status: String,
    pub phase: String,
    pub terminal: bool,
    pub started_at_ms: i64,
    pub elapsed_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_tool: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

/// Mirrors TS `ContextUsage`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObserveContextUsage {
    pub tokens: Option<i64>,
    pub context_window: u64,
    pub percent: Option<f64>,
}

/// Run-wide token and cost totals. Mirrors TS `ObserveHeader["usage"]`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObserveUsage {
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_write: i64,
    pub cost: f64,
}

/// Mirrors TS `ObserveHeader`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObserveHeader {
    pub run_id: String,
    pub session_id: String,
    pub main_thread_id: String,
    pub main_thread_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_name: Option<String>,
    pub model: Option<String>,
    pub thinking_level: Option<String>,
    #[serde(default)]
    pub context: Option<ObserveContextUsage>,
    pub usage: ObserveUsage,
    pub started_at: i64,
}

/// Mirrors TS `ObserveState`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObserveState {
    #[serde(flatten)]
    pub header: ObserveHeader,
    pub items: Vec<ObserveItem>,
    pub lanes: Vec<ObserveLane>,
    pub threads: Vec<ObserveThread>,
}

/// Recorder fan-out event. Mirrors TS `ObserveRecorderEvent`, except that the
/// `header` variant always carries the header: TS sends a bare signal live and
/// embeds the header only when persisting, so carrying it here makes the live
/// and stored payloads identical.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ObserveRecorderEvent {
    Activity { item: ObserveItem },
    ActivityUpdate { item: ObserveItem },
    Lane { lane: ObserveLane },
    Thread { thread: ObserveThread },
    Header { header: ObserveHeader },
}

impl ObserveRecorderEvent {
    /// The stored `event_type` column / SSE event name.
    pub fn type_tag(&self) -> &'static str {
        match self {
            ObserveRecorderEvent::Activity { .. } => "activity",
            ObserveRecorderEvent::ActivityUpdate { .. } => "activity_update",
            ObserveRecorderEvent::Lane { .. } => "lane",
            ObserveRecorderEvent::Thread { .. } => "thread",
            ObserveRecorderEvent::Header { .. } => "header",
        }
    }
}

/// Run lifecycle status. Mirrors the `observe_runs.status` CHECK.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ObserveRunStatus {
    Running,
    Finished,
}

impl ObserveRunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            ObserveRunStatus::Running => "running",
            ObserveRunStatus::Finished => "finished",
        }
    }
}

/// Mirrors TS `ObserveRunSummary`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObserveRunSummary {
    pub run_id: String,
    pub session_id: String,
    pub main_thread_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_name: Option<String>,
    pub model: Option<String>,
    pub thinking_level: Option<String>,
    pub started_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<i64>,
    pub status: ObserveRunStatus,
}

/// One persisted event. Mirrors TS `ObserveReplayEvent`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObserveReplayEvent {
    pub sequence: i64,
    pub ts: i64,
    #[serde(rename = "type")]
    pub event_type: String,
    pub payload: ObserveRecorderEvent,
}

/// Mirrors TS `CombinedObserveState`: the current recorder header plus the
/// merged recent items of every live run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CombinedObserveState {
    #[serde(flatten)]
    pub state: ObserveState,
    pub view: String,
    pub live_run_id: String,
    pub runs: Vec<ObserveRunSummary>,
}
