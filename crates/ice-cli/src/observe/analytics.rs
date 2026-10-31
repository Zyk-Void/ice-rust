//! Cross-run analytics for the dashboard: an enriched run list and an
//! overview of tokens, cost, tool performance and activity over a window.
//!
//! A history backend (the SQLite store, or ClickHouse when configured) only
//! supplies *facts*: the runs started in a window and the final state of
//! every activity in it. All aggregation happens here, once, so both
//! backends report the same numbers.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::Serialize;

use super::types::*;

pub const HOUR_MS: i64 = 3_600_000;
pub const DAY_MS: i64 = 24 * HOUR_MS;
/// Upper bound on series points; wider windows get wider buckets.
pub const MAX_BUCKETS: i64 = 400;
/// Last epoch millisecond in year 9999; leaves room for bucket arithmetic.
pub const MAX_TIMESTAMP: i64 = 253_402_300_799_999;
const TOP_FILES: usize = 25;
const TOP_RUN_TOOLS: usize = 3;
/// Characters of a user prompt kept as a run label.
pub const PROMPT_LABEL_CHARS: usize = 160;

/// A history backend the analytics read from.
pub trait AnalyticsSource: Send + Sync {
    /// `sqlite` or `clickhouse`.
    fn name(&self) -> &'static str;
    /// Runs started in `[since, until)`, newest first.
    fn runs_between(&self, since: i64, until: i64) -> Result<Vec<RunFact>, String>;
    /// The final state of every activity that started in `[since, until)`.
    fn items_between(&self, since: i64, until: i64) -> Result<Vec<ItemFact>, String>;
    /// One run's state folded from its stored events.
    fn run_state(&self, run_id: &str) -> Result<Option<ObserveState>, String>;
}

/// A run plus the time of its latest stored event.
#[derive(Debug, Clone, PartialEq)]
pub struct RunFact {
    pub summary: ObserveRunSummary,
    pub last_activity: Option<i64>,
}

/// The final state of one activity, reduced to what analytics use.
#[derive(Debug, Clone, PartialEq)]
pub struct ItemFact {
    pub run_id: String,
    pub ts: i64,
    pub kind: ObserveItemKind,
    pub tool_name: Option<String>,
    pub state: Option<ObserveItemState>,
    pub duration_ms: Option<i64>,
    /// On a Subagents V2 child thread.
    pub subagent: bool,
    /// Target path of an `edit` or `write` call.
    pub path: Option<String>,
    /// Usage of an assistant reply; `None` for every other item.
    pub turn: Option<TurnFact>,
    /// The start of a user prompt, as a run label.
    pub prompt: Option<String>,
}

/// Token use and outcome of one assistant reply.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TurnFact {
    /// `provider/model`, the run header's format.
    pub model: Option<String>,
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_write: i64,
    pub cost: f64,
    pub stop_reason: Option<String>,
}

/// A prompt shortened to one line of at most [`PROMPT_LABEL_CHARS`].
pub fn prompt_label(text: &str) -> Option<String> {
    let line: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.is_empty() {
        return None;
    }
    let mut label: String = line.chars().take(PROMPT_LABEL_CHARS).collect();
    if label.len() < line.len() {
        label.push('…');
    }
    Some(label)
}

/// `provider/model`, the run header's model format.
pub(crate) fn model_label(provider: Option<&str>, model: Option<&str>) -> Option<String> {
    match (provider, model) {
        (Some(provider), Some(model)) => Some(format!("{provider}/{model}")),
        (None, Some(model)) => Some(model.to_string()),
        _ => None,
    }
}

impl TurnFact {
    /// Read usage from an assistant item's raw message JSON. A missing or
    /// truncated (invalid) message counts as a turn with no usage.
    pub fn from_raw(raw: Option<&str>) -> Self {
        let Some(message) = raw.and_then(|r| serde_json::from_str::<serde_json::Value>(r).ok())
        else {
            return Self::default();
        };
        let usage = &message["usage"];
        let int = |key: &str| usage[key].as_i64().unwrap_or(0);
        let text = |key: &str| message[key].as_str().filter(|s| !s.is_empty());
        Self {
            model: model_label(text("provider"), text("model")),
            input: int("input"),
            output: int("output"),
            cache_read: int("cacheRead"),
            cache_write: int("cacheWrite"),
            cost: usage["cost"]["total"].as_f64().unwrap_or(0.0),
            stop_reason: text("stopReason").map(str::to_string),
        }
    }

    fn failed(&self) -> bool {
        matches!(self.stop_reason.as_deref(), Some("error" | "aborted"))
    }
}

impl ItemFact {
    pub fn from_item(run_id: &str, item: &ObserveItem) -> Self {
        let path = match item.tool_name.as_deref() {
            Some("edit" | "write") => item
                .args
                .as_deref()
                .and_then(|args| serde_json::from_str::<serde_json::Value>(args).ok())
                .and_then(|args| args["path"].as_str().map(str::to_string)),
            _ => None,
        };
        let assistant =
            item.kind == ObserveItemKind::Message && item.role == Some(ObserveRole::Assistant);
        Self {
            run_id: run_id.to_string(),
            ts: item.ts,
            kind: item.kind,
            tool_name: item.tool_name.clone(),
            state: item.state,
            duration_ms: item.duration_ms,
            subagent: item.identity.thread_kind == ObserveThreadKind::Subagent,
            path,
            turn: assistant.then(|| TurnFact::from_raw(item.raw.as_deref())),
            prompt: match (item.kind, item.role) {
                (ObserveItemKind::Message, Some(ObserveRole::User)) => {
                    item.text.as_deref().and_then(prompt_label)
                }
                _ => None,
            },
        }
    }

    fn is_tool(&self) -> bool {
        self.kind == ObserveItemKind::Tool
    }

    fn failed(&self) -> bool {
        self.state == Some(ObserveItemState::Error)
            || self.turn.as_ref().is_some_and(TurnFact::failed)
    }
}

/// Token and cost sums.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenTotals {
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_write: i64,
    pub cost: f64,
}

impl TokenTotals {
    fn add(&mut self, turn: &TurnFact) {
        self.input += turn.input;
        self.output += turn.output;
        self.cache_read += turn.cache_read;
        self.cache_write += turn.cache_write;
        self.cost += turn.cost;
    }
}

/// Window-wide totals.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Totals {
    pub runs: u64,
    pub turns: u64,
    pub turn_errors: u64,
    pub tool_calls: u64,
    pub tool_errors: u64,
    pub subagent_items: u64,
    #[serde(flatten)]
    pub tokens: TokenTotals,
    pub files_changed: u64,
    pub median_run_ms: Option<i64>,
    pub p95_tool_ms: Option<i64>,
}

/// One series bucket starting at `t`.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SeriesPoint {
    pub t: i64,
    pub runs: u64,
    pub turns: u64,
    pub tool_calls: u64,
    pub tool_errors: u64,
    #[serde(flatten)]
    pub tokens: TokenTotals,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelStat {
    pub model: String,
    pub runs: u64,
    pub turns: u64,
    pub errors: u64,
    #[serde(flatten)]
    pub tokens: TokenTotals,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolStat {
    pub tool: String,
    pub calls: u64,
    pub errors: u64,
    pub runs: u64,
    pub p50_ms: Option<i64>,
    pub p95_ms: Option<i64>,
    pub max_ms: Option<i64>,
    pub total_ms: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileStat {
    pub path: String,
    pub edits: u64,
    pub writes: u64,
    pub runs: u64,
    pub last_ts: i64,
}

/// `GET /api/overview`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    pub source: String,
    pub since: i64,
    pub until: i64,
    pub bucket_ms: i64,
    pub totals: Totals,
    /// The same totals for the equally long window before `since`.
    pub previous: Totals,
    pub series: Vec<SeriesPoint>,
    pub models: Vec<ModelStat>,
    pub tools: Vec<ToolStat>,
    pub kinds: BTreeMap<String, u64>,
    pub stop_reasons: BTreeMap<String, u64>,
    /// Tool calls plus replies by local weekday (Monday first) and hour.
    pub heatmap: Vec<[u64; 24]>,
    pub files: Vec<FileStat>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OverviewQuery {
    pub since: i64,
    pub until: i64,
    pub bucket_ms: Option<i64>,
    /// Viewer's offset from UTC in minutes (east positive), for day buckets
    /// and the heatmap.
    pub tz_offset_min: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NamedCount {
    pub name: String,
    pub count: u64,
}

/// One row of `GET /api/runs`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunRow {
    #[serde(flatten)]
    pub summary: ObserveRunSummary,
    pub last_activity: Option<i64>,
    pub duration_ms: Option<i64>,
    pub turns: u64,
    pub turn_errors: u64,
    pub tool_calls: u64,
    pub tool_errors: u64,
    pub subagent_items: u64,
    #[serde(flatten)]
    pub tokens: TokenTotals,
    pub files: u64,
    /// Models that answered, in first-use order.
    pub models: Vec<String>,
    pub top_tools: Vec<NamedCount>,
    /// The start of the run's first user prompt.
    pub prompt: Option<String>,
}

/// `GET /api/runs`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunList {
    pub source: String,
    pub since: i64,
    pub until: i64,
    pub runs: Vec<RunRow>,
}

/// Nearest-rank percentile of an ascending slice.
fn percentile(sorted: &[i64], p: f64) -> Option<i64> {
    if sorted.is_empty() {
        return None;
    }
    let rank = (p * sorted.len() as f64).ceil() as usize;
    Some(sorted[rank.clamp(1, sorted.len()) - 1])
}

/// A bucket width that keeps the series at or under [`MAX_BUCKETS`] points.
pub fn bucket_for(span_ms: i64, requested: Option<i64>) -> i64 {
    let auto = match span_ms {
        s if s <= 2 * DAY_MS => HOUR_MS,
        s if s <= 14 * DAY_MS => 6 * HOUR_MS,
        s if s <= 120 * DAY_MS => DAY_MS,
        _ => 7 * DAY_MS,
    };
    let bucket = requested
        .filter(|b| *b >= 60_000)
        .unwrap_or(auto)
        .min(span_ms.max(60_000));
    // Reserve one point for alignment before the start of the window.
    let span = span_ms.max(1);
    let min = span / (MAX_BUCKETS - 1) + i64::from(span % (MAX_BUCKETS - 1) != 0);
    bucket.max(min)
}

/// Start of the bucket holding `ts`, aligned to the viewer's local midnight.
fn bucket_start(ts: i64, bucket: i64, tz_ms: i64) -> i64 {
    (ts + tz_ms).div_euclid(bucket) * bucket - tz_ms
}

/// Weekday (Monday = 0) and hour of `ts` in the viewer's time zone.
fn local_slot(ts: i64, tz_ms: i64) -> (usize, usize) {
    let local = ts + tz_ms;
    // 1970-01-01 was a Thursday.
    let weekday = (local.div_euclid(DAY_MS) + 3).rem_euclid(7) as usize;
    let hour = (local.rem_euclid(DAY_MS) / HOUR_MS) as usize;
    (weekday, hour)
}

fn run_duration(run: &RunFact) -> Option<i64> {
    let end = run.summary.ended_at.or(run.last_activity)?;
    Some((end - run.summary.started_at).max(0))
}

fn totals(runs: &[RunFact], items: &[ItemFact]) -> Totals {
    let mut out = Totals {
        runs: runs.len() as u64,
        ..Totals::default()
    };
    let mut files = HashSet::new();
    let mut tool_ms = Vec::new();
    for item in items {
        if item.subagent {
            out.subagent_items += 1;
        }
        if let Some(turn) = &item.turn {
            out.turns += 1;
            out.turn_errors += u64::from(item.failed());
            out.tokens.add(turn);
        }
        if item.is_tool() {
            out.tool_calls += 1;
            out.tool_errors += u64::from(item.failed());
            tool_ms.extend(item.duration_ms);
        }
        if let Some(path) = &item.path {
            files.insert((item.run_id.as_str(), path.as_str()));
        }
    }
    out.files_changed = files.len() as u64;
    let mut run_ms: Vec<i64> = runs.iter().filter_map(run_duration).collect();
    run_ms.sort_unstable();
    out.median_run_ms = percentile(&run_ms, 0.5);
    tool_ms.sort_unstable();
    out.p95_tool_ms = percentile(&tool_ms, 0.95);
    out
}

/// Aggregate an overview from the backend's facts.
pub fn overview(source: &dyn AnalyticsSource, query: &OverviewQuery) -> Result<Overview, String> {
    if query.since < 0 || query.since >= query.until || query.until > MAX_TIMESTAMP {
        return Err("invalid time window".into());
    }
    let until = query.until;
    let span = until - query.since;
    let runs = source.runs_between(query.since, until)?;
    let items = source.items_between(query.since, until)?;
    let previous_runs = source.runs_between(query.since - span, query.since)?;
    let previous_items = source.items_between(query.since - span, query.since)?;
    let tz_ms = query.tz_offset_min.clamp(-24 * 60, 24 * 60) * 60_000;
    let bucket = bucket_for(span, query.bucket_ms);

    let first = bucket_start(query.since, bucket, tz_ms);
    let count = ((until - first + bucket - 1) / bucket).max(1);
    let mut series: Vec<SeriesPoint> = (0..count)
        .map(|i| SeriesPoint {
            t: first + i * bucket,
            ..SeriesPoint::default()
        })
        .collect();
    let slot = |ts: i64| ((bucket_start(ts, bucket, tz_ms) - first) / bucket) as usize;
    for run in &runs {
        if let Some(point) = series.get_mut(slot(run.summary.started_at)) {
            point.runs += 1;
        }
    }

    let run_models: HashMap<&str, Option<&str>> = runs
        .iter()
        .map(|r| (r.summary.run_id.as_str(), r.summary.model.as_deref()))
        .collect();
    let mut models: BTreeMap<String, (ModelStat, HashSet<&str>)> = BTreeMap::new();
    let mut tools: BTreeMap<&str, (ToolStat, Vec<i64>, HashSet<&str>)> = BTreeMap::new();
    let mut files: HashMap<&str, (FileStat, HashSet<&str>)> = HashMap::new();
    let mut kinds = BTreeMap::new();
    let mut stop_reasons = BTreeMap::new();
    let mut heatmap = vec![[0u64; 24]; 7];
    for item in &items {
        *kinds.entry(item.kind.as_str().to_string()).or_insert(0) += 1;
        let point = series.get_mut(slot(item.ts));
        if let Some(turn) = &item.turn {
            if let Some(point) = point {
                point.turns += 1;
                point.tokens.add(turn);
            }
            let model = turn
                .model
                .as_deref()
                .or_else(|| run_models.get(item.run_id.as_str()).copied().flatten())
                .unwrap_or("unknown");
            let (stat, model_runs) = models.entry(model.to_string()).or_default();
            stat.turns += 1;
            stat.errors += u64::from(item.failed());
            stat.tokens.add(turn);
            model_runs.insert(&item.run_id);
            if let Some(reason) = &turn.stop_reason {
                *stop_reasons.entry(reason.clone()).or_insert(0) += 1;
            }
        } else if item.is_tool() {
            if let Some(point) = point {
                point.tool_calls += 1;
                point.tool_errors += u64::from(item.failed());
            }
            let name = item.tool_name.as_deref().unwrap_or("unknown");
            let (stat, durations, tool_runs) = tools.entry(name).or_default();
            stat.calls += 1;
            stat.errors += u64::from(item.failed());
            if let Some(ms) = item.duration_ms {
                durations.push(ms);
                stat.total_ms += ms;
            }
            tool_runs.insert(&item.run_id);
        } else {
            continue;
        }
        let (weekday, hour) = local_slot(item.ts, tz_ms);
        heatmap[weekday][hour] += 1;
        if let (Some(path), Some(tool)) = (&item.path, &item.tool_name) {
            let (stat, file_runs) = files.entry(path.as_str()).or_default();
            if tool == "write" {
                stat.writes += 1;
            } else {
                stat.edits += 1;
            }
            stat.last_ts = stat.last_ts.max(item.ts);
            file_runs.insert(&item.run_id);
        }
    }

    let mut models: Vec<ModelStat> = models
        .into_iter()
        .map(|(model, (mut stat, model_runs))| {
            stat.model = model;
            stat.runs = model_runs.len() as u64;
            stat
        })
        .collect();
    models.sort_by(|a, b| b.turns.cmp(&a.turns).then_with(|| a.model.cmp(&b.model)));
    let mut tools: Vec<ToolStat> = tools
        .into_iter()
        .map(|(name, (mut stat, mut durations, tool_runs))| {
            durations.sort_unstable();
            stat.tool = name.to_string();
            stat.runs = tool_runs.len() as u64;
            stat.p50_ms = percentile(&durations, 0.5);
            stat.p95_ms = percentile(&durations, 0.95);
            stat.max_ms = durations.last().copied();
            stat
        })
        .collect();
    tools.sort_by(|a, b| b.calls.cmp(&a.calls).then_with(|| a.tool.cmp(&b.tool)));
    let mut files: Vec<FileStat> = files
        .into_iter()
        .map(|(path, (mut stat, file_runs))| {
            stat.path = path.to_string();
            stat.runs = file_runs.len() as u64;
            stat
        })
        .collect();
    files.sort_by(|a, b| {
        (b.edits + b.writes)
            .cmp(&(a.edits + a.writes))
            .then_with(|| b.last_ts.cmp(&a.last_ts))
            .then_with(|| a.path.cmp(&b.path))
    });
    files.truncate(TOP_FILES);

    Ok(Overview {
        source: source.name().to_string(),
        since: query.since,
        until,
        bucket_ms: bucket,
        totals: totals(&runs, &items),
        previous: totals(&previous_runs, &previous_items),
        series,
        models,
        tools,
        kinds,
        stop_reasons,
        heatmap,
        files,
    })
}

/// Runs started in `[since, until)` with per-run totals, newest first.
pub fn run_list(source: &dyn AnalyticsSource, since: i64, until: i64) -> Result<RunList, String> {
    let runs = source.runs_between(since, until)?;
    // A run's activity can outlast the window it started in.
    let items = source.items_between(since, i64::MAX)?;
    let mut by_run: HashMap<&str, Vec<&ItemFact>> = HashMap::new();
    for item in &items {
        by_run.entry(&item.run_id).or_default().push(item);
    }
    let rows = runs
        .into_iter()
        .map(|run| {
            let run_items = by_run
                .get(run.summary.run_id.as_str())
                .map(Vec::as_slice)
                .unwrap_or_default();
            let mut row = RunRow {
                duration_ms: run_duration(&run),
                last_activity: run.last_activity,
                summary: run.summary,
                turns: 0,
                turn_errors: 0,
                tool_calls: 0,
                tool_errors: 0,
                subagent_items: 0,
                tokens: TokenTotals::default(),
                files: 0,
                models: Vec::new(),
                top_tools: Vec::new(),
                prompt: None,
            };
            let mut first_prompt: Option<(i64, &str)> = None;
            let mut files = HashSet::new();
            let mut tools: HashMap<&str, u64> = HashMap::new();
            for item in run_items {
                row.subagent_items += u64::from(item.subagent);
                if let Some(turn) = &item.turn {
                    row.turns += 1;
                    row.turn_errors += u64::from(item.failed());
                    row.tokens.add(turn);
                    if let Some(model) = &turn.model {
                        if !row.models.contains(model) {
                            row.models.push(model.clone());
                        }
                    }
                }
                if item.is_tool() {
                    row.tool_calls += 1;
                    row.tool_errors += u64::from(item.failed());
                    *tools
                        .entry(item.tool_name.as_deref().unwrap_or("unknown"))
                        .or_default() += 1;
                }
                if let Some(path) = &item.path {
                    files.insert(path.as_str());
                }
                if let Some(prompt) = &item.prompt {
                    if first_prompt.is_none_or(|(ts, _)| item.ts < ts) {
                        first_prompt = Some((item.ts, prompt));
                    }
                }
            }
            row.prompt = first_prompt.map(|(_, prompt)| prompt.to_string());
            row.files = files.len() as u64;
            let mut tools: Vec<NamedCount> = tools
                .into_iter()
                .map(|(name, count)| NamedCount {
                    name: name.to_string(),
                    count,
                })
                .collect();
            tools.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.name.cmp(&b.name)));
            tools.truncate(TOP_RUN_TOOLS);
            row.top_tools = tools;
            row
        })
        .collect();
    Ok(RunList {
        source: source.name().to_string(),
        since,
        until,
        runs: rows,
    })
}

#[cfg(test)]
#[path = "analytics_tests.rs"]
mod tests;
