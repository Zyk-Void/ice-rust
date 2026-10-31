//! Optional ClickHouse mirror of the observation history.
//!
//! With `ICERS_OBSERVE_CLICKHOUSE_URL` set, every icers process copies the
//! SQLite history into ClickHouse over its HTTP interface, and the dashboard
//! reads analytics (and runs older than the local retention) from it. SQLite
//! stays the local source of truth; ClickHouse is a mirror that can keep
//! history longer and collect it from several machines.
//!
//! - Delivery is at least once. At startup a process backfills every run that
//!   ClickHouse is behind on, then tails its own run. Rows are keyed by
//!   `(run_id, sequence)` in `ReplacingMergeTree` tables, so repeats collapse;
//!   reads use `argMax` or `FINAL` and never count a repeat twice.
//! - Payloads are copied as stored, so ClickHouse only receives redacted text.
//! - Failures never block the agent. They are retried with backoff and shown
//!   by `/api/meta`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use super::analytics::{model_label, prompt_label, AnalyticsSource, ItemFact, RunFact, TurnFact};
use super::recorder::now_ms;
use super::store::{ObserveRunStore, StateFold, StoredEvent};
use super::types::*;

pub const ENV_URL: &str = "ICERS_OBSERVE_CLICKHOUSE_URL";
pub const ENV_DATABASE: &str = "ICERS_OBSERVE_CLICKHOUSE_DATABASE";
pub const ENV_USER: &str = "ICERS_OBSERVE_CLICKHOUSE_USER";
pub const ENV_PASSWORD: &str = "ICERS_OBSERVE_CLICKHOUSE_PASSWORD";
pub const DEFAULT_DATABASE: &str = "icers";

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const SYNC_INTERVAL: Duration = Duration::from_secs(2);
const MAX_BACKOFF: Duration = Duration::from_secs(60);
const FINAL_FLUSH: Duration = Duration::from_secs(3);
const PAGE: i64 = 2_000;
const MAX_STATE_EVENTS: i64 = 100_000;
/// Finished run rows always outrank running ones in `ReplacingMergeTree`.
const FINISHED_VERSION_BASE: u64 = 1 << 62;

/// Connection settings, read from the environment.
#[derive(Clone, PartialEq, Eq)]
pub struct ClickHouseConfig {
    /// HTTP interface URL without credentials, e.g. `http://127.0.0.1:8123`.
    pub url: String,
    pub database: String,
    pub user: Option<String>,
    pub password: Option<String>,
}

impl std::fmt::Debug for ClickHouseConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClickHouseConfig")
            .field("url", &self.url)
            .field("database", &self.database)
            .field("user", &self.user)
            .field("password", &self.password.as_ref().map(|_| "[REDACTED]"))
            .finish()
    }
}

impl ClickHouseConfig {
    /// `None` when the mirror is not configured.
    pub fn from_env() -> Option<Result<Self, String>> {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Option<Result<Self, String>> {
        let get = |key: &str| {
            get(key)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        let url = get(ENV_URL)?;
        Some(Self::parse(
            url,
            get(ENV_DATABASE),
            get(ENV_USER),
            get(ENV_PASSWORD),
        ))
    }

    fn parse(
        url: String,
        database: Option<String>,
        user: Option<String>,
        password: Option<String>,
    ) -> Result<Self, String> {
        let parsed = reqwest::Url::parse(&url).map_err(|e| format!("{ENV_URL}: {e}"))?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(format!("{ENV_URL} must be an http or https URL"));
        }
        if !parsed.username().is_empty() || parsed.password().is_some() {
            return Err(format!(
                "{ENV_URL} must not contain credentials; use {ENV_USER} and {ENV_PASSWORD}"
            ));
        }
        if parsed.query().is_some() || parsed.fragment().is_some() {
            return Err(format!("{ENV_URL} must not have a query or fragment"));
        }
        let database = database.unwrap_or_else(|| DEFAULT_DATABASE.into());
        let valid = database
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && database
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_');
        if !valid {
            return Err(format!(
                "{ENV_DATABASE} must be a plain identifier (letters, digits, _)"
            ));
        }
        Ok(Self {
            url: url.trim_end_matches('/').to_string(),
            database,
            user,
            password,
        })
    }
}

/// Mirror health, served by `/api/meta`.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClickHouseStatus {
    pub url: String,
    pub database: String,
    /// `None` until the first request finishes.
    pub connected: Option<bool>,
    pub last_error: Option<String>,
    pub last_sync_at: Option<i64>,
    pub events_sent: u64,
}

/// A ClickHouse HTTP client for the mirror and the dashboard.
pub struct ClickHouse {
    config: ClickHouseConfig,
    http: reqwest::Client,
    handle: tokio::runtime::Handle,
    schema_ready: tokio::sync::OnceCell<()>,
    status: Mutex<ClickHouseStatus>,
}

/// One `observe_runs` row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct RunRow {
    run_id: String,
    session_id: String,
    main_thread_id: String,
    session_name: Option<String>,
    model: Option<String>,
    thinking_level: Option<String>,
    started_at_ms: i64,
    ended_at_ms: Option<i64>,
    status: String,
    #[serde(default)]
    version: u64,
}

impl RunRow {
    fn from_summary(run: &ObserveRunSummary) -> Self {
        let version = match (run.status, run.ended_at) {
            (ObserveRunStatus::Finished, Some(ended)) => {
                FINISHED_VERSION_BASE + ended.max(0) as u64
            }
            (ObserveRunStatus::Finished, None) => FINISHED_VERSION_BASE,
            (ObserveRunStatus::Running, _) => now_ms().max(0) as u64,
        };
        Self {
            run_id: run.run_id.clone(),
            session_id: run.session_id.clone(),
            main_thread_id: run.main_thread_id.clone(),
            session_name: run.session_name.clone(),
            model: run.model.clone(),
            thinking_level: run.thinking_level.clone(),
            started_at_ms: run.started_at,
            ended_at_ms: run.ended_at,
            status: run.status.as_str().into(),
            version,
        }
    }

    fn into_summary(self) -> ObserveRunSummary {
        ObserveRunSummary {
            run_id: self.run_id,
            session_id: self.session_id,
            main_thread_id: self.main_thread_id,
            session_name: self.session_name,
            model: self.model,
            thinking_level: self.thinking_level,
            started_at: self.started_at_ms,
            ended_at: self.ended_at_ms,
            status: if self.status == "finished" {
                ObserveRunStatus::Finished
            } else {
                ObserveRunStatus::Running
            },
        }
    }
}

/// One `observe_events` insert row.
#[derive(Serialize)]
struct EventRow<'a> {
    run_id: &'a str,
    sequence: i64,
    ts_ms: i64,
    event_type: &'a str,
    payload: &'a str,
}

/// The final state of one activity, as [`ITEMS_SQL`] returns it.
#[derive(Debug, Deserialize)]
struct ItemRow {
    run_id: String,
    ts: i64,
    kind: String,
    tool_name: String,
    state: String,
    duration_ms: Option<i64>,
    thread_kind: String,
    role: String,
    path: String,
    input: i64,
    output: i64,
    cache_read: i64,
    cache_write: i64,
    cost: f64,
    provider: String,
    model: String,
    stop_reason: String,
    #[serde(default)]
    prompt: String,
}

fn non_empty(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}

fn enum_from<T: serde::de::DeserializeOwned>(value: &str) -> Option<T> {
    serde_json::from_value(serde_json::Value::String(value.to_string())).ok()
}

impl ItemRow {
    fn into_fact(self) -> Option<ItemFact> {
        let kind: ObserveItemKind = enum_from(&self.kind)?;
        let assistant = kind == ObserveItemKind::Message && self.role == "assistant";
        Some(ItemFact {
            ts: self.ts,
            kind,
            state: enum_from(&self.state),
            duration_ms: self.duration_ms,
            subagent: self.thread_kind == "subagent",
            path: non_empty(self.path),
            turn: assistant.then(|| TurnFact {
                model: model_label(
                    Some(self.provider.as_str()).filter(|s| !s.is_empty()),
                    Some(self.model.as_str()).filter(|s| !s.is_empty()),
                ),
                input: self.input,
                output: self.output,
                cache_read: self.cache_read,
                cache_write: self.cache_write,
                cost: self.cost,
                stop_reason: non_empty(self.stop_reason),
            }),
            prompt: prompt_label(&self.prompt),
            tool_name: non_empty(self.tool_name),
            run_id: self.run_id,
        })
    }
}

/// DDL for the mirror's database and tables. Derived columns are computed
/// by ClickHouse at insert time, so analytics never parse payloads.
pub fn schema_statements(db: &str) -> Vec<String> {
    let raw = "JSONExtractString(payload, 'item', 'raw')";
    vec![
        format!("CREATE DATABASE IF NOT EXISTS {db}"),
        format!(
            "CREATE TABLE IF NOT EXISTS {db}.observe_runs (
  run_id String,
  session_id String,
  main_thread_id String,
  session_name Nullable(String),
  model Nullable(String),
  thinking_level Nullable(String),
  started_at_ms Int64,
  ended_at_ms Nullable(Int64),
  status LowCardinality(String),
  version UInt64
) ENGINE = ReplacingMergeTree(version) ORDER BY run_id"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS {db}.observe_events (
  run_id String,
  sequence Int64,
  ts_ms Int64,
  event_type LowCardinality(String),
  payload String,
  item_id UInt64 MATERIALIZED JSONExtractUInt(payload, 'item', 'id'),
  kind LowCardinality(String) MATERIALIZED JSONExtractString(payload, 'item', 'kind'),
  tool_name LowCardinality(String) MATERIALIZED JSONExtractString(payload, 'item', 'toolName'),
  state LowCardinality(String) MATERIALIZED JSONExtractString(payload, 'item', 'state'),
  duration_ms Nullable(Int64) MATERIALIZED JSONExtract(payload, 'item', 'durationMs', 'Nullable(Int64)'),
  thread_kind LowCardinality(String) MATERIALIZED JSONExtractString(payload, 'item', 'threadKind'),
  role LowCardinality(String) MATERIALIZED JSONExtractString(payload, 'item', 'role'),
  path String MATERIALIZED if(JSONExtractString(payload, 'item', 'toolName') IN ('edit', 'write'), JSONExtractString(JSONExtractString(payload, 'item', 'args'), 'path'), ''),
  usage_input Int64 MATERIALIZED JSONExtractInt({raw}, 'usage', 'input'),
  usage_output Int64 MATERIALIZED JSONExtractInt({raw}, 'usage', 'output'),
  usage_cache_read Int64 MATERIALIZED JSONExtractInt({raw}, 'usage', 'cacheRead'),
  usage_cache_write Int64 MATERIALIZED JSONExtractInt({raw}, 'usage', 'cacheWrite'),
  usage_cost Float64 MATERIALIZED JSONExtractFloat({raw}, 'usage', 'cost', 'total'),
  turn_provider LowCardinality(String) MATERIALIZED JSONExtractString({raw}, 'provider'),
  turn_model LowCardinality(String) MATERIALIZED JSONExtractString({raw}, 'model'),
  stop_reason LowCardinality(String) MATERIALIZED JSONExtractString({raw}, 'stopReason')
) ENGINE = ReplacingMergeTree
PARTITION BY toYYYYMM(fromUnixTimestamp64Milli(ts_ms))
ORDER BY (run_id, sequence)"
        ),
    ]
}

/// Final activity states in a window. `argMax` by sequence picks each
/// item's latest version and ignores repeated rows.
const ITEMS_SQL: &str = "SELECT run_id, item_id,
  argMax(ts_ms, sequence) AS ts, argMax(kind, sequence) AS kind,
  argMax(tool_name, sequence) AS tool_name, argMax(state, sequence) AS state,
  argMax(duration_ms, sequence) AS duration_ms, argMax(thread_kind, sequence) AS thread_kind,
  argMax(role, sequence) AS role, argMax(path, sequence) AS path,
  argMax(usage_input, sequence) AS input, argMax(usage_output, sequence) AS output,
  argMax(usage_cache_read, sequence) AS cache_read, argMax(usage_cache_write, sequence) AS cache_write,
  argMax(usage_cost, sequence) AS cost, argMax(turn_provider, sequence) AS provider,
  argMax(turn_model, sequence) AS model, argMax(stop_reason, sequence) AS stop_reason,
  argMax(if(JSONExtractString(payload, 'item', 'role') = 'user', substringUTF8(JSONExtractString(payload, 'item', 'text'), 1, 400), ''), sequence) AS prompt
FROM {db}.observe_events
WHERE event_type IN ('activity', 'activity_update')
  AND ts_ms >= {since:Int64} AND ts_ms < {until:Int64}
GROUP BY run_id, item_id
ORDER BY run_id, ts, item_id";

const RUN_SELECT: &str = "SELECT run_id, session_id, main_thread_id, session_name, model, \
     thinking_level, started_at_ms, ended_at_ms, status, version FROM {db}.observe_runs FINAL";

fn first_line(text: &str) -> String {
    let line = text.lines().next().unwrap_or("").trim();
    line.chars().take(300).collect()
}

impl ClickHouse {
    /// Build a client on the current Tokio runtime.
    pub fn new(config: ClickHouseConfig) -> Result<Self, String> {
        let handle = tokio::runtime::Handle::try_current().map_err(|e| e.to_string())?;
        let http = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            status: Mutex::new(ClickHouseStatus {
                url: config.url.clone(),
                database: config.database.clone(),
                ..ClickHouseStatus::default()
            }),
            config,
            http,
            handle,
            schema_ready: tokio::sync::OnceCell::new(),
        })
    }

    pub fn status(&self) -> ClickHouseStatus {
        self.status
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    fn record(&self, outcome: &Result<(), String>, sent: u64) {
        let mut status = self.status.lock().unwrap_or_else(|p| p.into_inner());
        status.events_sent += sent;
        match outcome {
            Ok(()) => {
                status.connected = Some(true);
                status.last_error = None;
                status.last_sync_at = Some(now_ms());
            }
            Err(error) => {
                status.connected = Some(false);
                status.last_error = Some(error.clone());
            }
        }
    }

    fn sql(&self, template: &str) -> String {
        template.replace("{db}", &self.config.database)
    }

    /// POST one statement. `query` goes in the URL (used for inserts whose
    /// body is data); otherwise the body is the statement.
    async fn post(
        &self,
        query: Option<&str>,
        params: &[(&str, String)],
        body: String,
    ) -> Result<String, String> {
        let mut url = reqwest::Url::parse(&self.config.url).map_err(|e| e.to_string())?;
        {
            // No `database` parameter: ClickHouse rejects it while the
            // database does not exist yet, so statements name `db.table`.
            let mut pairs = url.query_pairs_mut();
            pairs.append_pair("output_format_json_quote_64bit_integers", "0");
            if let Some(query) = query {
                pairs.append_pair("query", query);
            }
            for (name, value) in params {
                pairs.append_pair(&format!("param_{name}"), value);
            }
        }
        let mut request = self.http.post(url).body(body);
        if let Some(user) = &self.config.user {
            request = request.header("X-ClickHouse-User", user);
        }
        if let Some(password) = &self.config.password {
            request = request.header("X-ClickHouse-Key", password);
        }
        let response = request
            .send()
            .await
            .map_err(|e| format!("ClickHouse request failed: {e}"))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|e| format!("ClickHouse response failed: {e}"))?;
        if !status.is_success() {
            return Err(format!(
                "ClickHouse HTTP {}: {}",
                status.as_u16(),
                first_line(&text)
            ));
        }
        Ok(text)
    }

    async fn select<T: serde::de::DeserializeOwned>(
        &self,
        sql: &str,
        params: &[(&str, String)],
    ) -> Result<Vec<T>, String> {
        let body = self
            .post(None, params, format!("{sql}\nFORMAT JSONEachRow"))
            .await?;
        body.lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| serde_json::from_str(line).map_err(|e| format!("ClickHouse row: {e}")))
            .collect()
    }

    async fn insert<T: Serialize>(&self, table: &str, rows: &[T]) -> Result<(), String> {
        if rows.is_empty() {
            return Ok(());
        }
        let mut body = String::new();
        for row in rows {
            body.push_str(&serde_json::to_string(row).map_err(|e| e.to_string())?);
            body.push('\n');
        }
        let query = format!(
            "INSERT INTO {}.{table} FORMAT JSONEachRow",
            self.config.database
        );
        self.post(Some(&query), &[], body).await.map(drop)
    }

    pub async fn ensure_schema(&self) -> Result<(), String> {
        self.schema_ready
            .get_or_try_init(|| async {
                for statement in schema_statements(&self.config.database) {
                    self.post(None, &[], statement).await?;
                }
                Ok::<(), String>(())
            })
            .await
            .map(drop)
    }

    async fn push_runs(&self, runs: &[ObserveRunSummary]) -> Result<(), String> {
        let rows: Vec<RunRow> = runs.iter().map(RunRow::from_summary).collect();
        self.insert("observe_runs", &rows).await
    }

    /// Copy a run's events with `sequence > after`; returns the new cursor
    /// and the number of events sent.
    async fn push_events(
        &self,
        store: &Arc<ObserveRunStore>,
        run_id: &str,
        mut after: i64,
    ) -> Result<(i64, u64), String> {
        let mut sent = 0;
        loop {
            let page = blocking(store, {
                let run_id = run_id.to_string();
                move |s| s.get_stored_events(&run_id, after, PAGE)
            })
            .await?;
            let Some(last) = page.last().map(|e: &StoredEvent| e.sequence) else {
                break;
            };
            let rows: Vec<EventRow<'_>> = page
                .iter()
                .map(|e| EventRow {
                    run_id,
                    sequence: e.sequence,
                    ts_ms: e.ts,
                    event_type: &e.event_type,
                    payload: &e.payload,
                })
                .collect();
            self.insert("observe_events", &rows).await?;
            sent += rows.len() as u64;
            after = last;
            if (page.len() as i64) < PAGE {
                break;
            }
        }
        Ok((after, sent))
    }

    /// Copy every run ClickHouse is behind on, plus all local run rows.
    /// Returns, per local run, the sequence ClickHouse now has through.
    async fn backfill(&self, store: &Arc<ObserveRunStore>) -> Result<HashMap<String, i64>, String> {
        #[derive(Deserialize)]
        struct Latest {
            run_id: String,
            sequence: i64,
        }
        let local = blocking(store, |s| s.latest_sequences()).await?;
        let remote: HashMap<String, i64> = self
            .select::<Latest>(
                &self.sql(
                    "SELECT run_id, max(sequence) AS sequence FROM {db}.observe_events GROUP BY run_id",
                ),
                &[],
            )
            .await?
            .into_iter()
            .map(|l| (l.run_id, l.sequence))
            .collect();
        let runs = blocking(store, |s| s.list_runs(Some(f64::MAX))).await?;
        self.push_runs(&runs).await?;
        let mut sent = 0;
        let mut through = HashMap::new();
        for (run_id, latest) in local {
            let from = remote.get(&run_id).copied().unwrap_or(0);
            // Events appended after `latest_sequences` are sent too, so the
            // cursor is what was pushed, not the earlier snapshot.
            let cursor = if latest > from {
                let (cursor, count) = self.push_events(store, &run_id, from).await?;
                sent += count;
                cursor
            } else {
                latest
            };
            through.insert(run_id, cursor);
        }
        self.record(&Ok(()), sent);
        Ok(through)
    }

    fn block_on<T>(&self, future: impl std::future::Future<Output = T>) -> T {
        // Only called from blocking threads (dashboard routes run under
        // `spawn_blocking`), never from inside the runtime.
        self.handle.block_on(future)
    }

    async fn runs_between_async(&self, since: i64, until: i64) -> Result<Vec<RunFact>, String> {
        #[derive(Deserialize)]
        struct Last {
            run_id: String,
            last_ts: i64,
        }
        let window = [("since", since.to_string()), ("until", until.to_string())];
        let runs: Vec<RunRow> = self
            .select(
                &self.sql(&format!(
                    "{RUN_SELECT} WHERE started_at_ms >= {{since:Int64}} AND started_at_ms < {{until:Int64}} \
                     ORDER BY started_at_ms DESC"
                )),
                &window,
            )
            .await?;
        let last: HashMap<String, i64> = self
            .select::<Last>(
                &self.sql(
                    "SELECT run_id, max(ts_ms) AS last_ts FROM {db}.observe_events \
                     WHERE ts_ms >= {since:Int64} GROUP BY run_id",
                ),
                &window[..1],
            )
            .await?
            .into_iter()
            .map(|l| (l.run_id, l.last_ts))
            .collect();
        Ok(runs
            .into_iter()
            .map(|row| RunFact {
                last_activity: last.get(&row.run_id).copied(),
                summary: row.into_summary(),
            })
            .collect())
    }

    async fn run_state_async(&self, run_id: &str) -> Result<Option<ObserveState>, String> {
        #[derive(Deserialize)]
        struct EventPayload {
            sequence: i64,
            payload: String,
        }
        let run = [("run", run_id.to_string())];
        let Some(row) = self
            .select::<RunRow>(
                &self.sql(&format!("{RUN_SELECT} WHERE run_id = {{run:String}}")),
                &run,
            )
            .await?
            .into_iter()
            .next()
        else {
            return Ok(None);
        };
        let limit = MAX_STATE_EVENTS + 1;
        let events: Vec<EventPayload> = self
            .select(
                &self.sql(&format!(
                    "SELECT sequence, payload FROM {{db}}.observe_events FINAL \
                     WHERE run_id = {{run:String}} ORDER BY sequence LIMIT {limit}"
                )),
                &run,
            )
            .await?;
        if events.len() > MAX_STATE_EVENTS as usize {
            return Err(format!(
                "run {run_id} exceeds the replay limit of {MAX_STATE_EVENTS} events"
            ));
        }
        let mut fold = StateFold::new(&row.into_summary());
        for event in events {
            let payload = serde_json::from_str(&event.payload)
                .map_err(|e| format!("corrupt observe event {run_id}#{}: {e}", event.sequence))?;
            fold.apply(payload);
        }
        Ok(Some(fold.finish()))
    }
}

impl AnalyticsSource for ClickHouse {
    fn name(&self) -> &'static str {
        "clickhouse"
    }

    fn runs_between(&self, since: i64, until: i64) -> Result<Vec<RunFact>, String> {
        self.block_on(async {
            self.ensure_schema().await?;
            self.runs_between_async(since, until).await
        })
    }

    fn items_between(&self, since: i64, until: i64) -> Result<Vec<ItemFact>, String> {
        self.block_on(async {
            self.ensure_schema().await?;
            let rows: Vec<ItemRow> = self
                .select(
                    &self.sql(ITEMS_SQL),
                    &[("since", since.to_string()), ("until", until.to_string())],
                )
                .await?;
            Ok(rows.into_iter().filter_map(ItemRow::into_fact).collect())
        })
    }

    fn run_state(&self, run_id: &str) -> Result<Option<ObserveState>, String> {
        self.block_on(async {
            self.ensure_schema().await?;
            self.run_state_async(run_id).await
        })
    }
}

async fn blocking<T: Send + 'static>(
    store: &Arc<ObserveRunStore>,
    f: impl FnOnce(&ObserveRunStore) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let store = Arc::clone(store);
    tokio::task::spawn_blocking(move || f(&store))
        .await
        .map_err(|e| e.to_string())?
}

/// The background task copying history into ClickHouse.
pub struct ClickHouseMirror {
    cancel: CancellationToken,
    task: tokio::task::JoinHandle<()>,
}

impl ClickHouseMirror {
    /// Backfill, then tail `run_id` until [`Self::finish`].
    pub fn spawn(clickhouse: Arc<ClickHouse>, store: Arc<ObserveRunStore>, run_id: String) -> Self {
        let cancel = CancellationToken::new();
        let task = tokio::spawn(mirror_loop(clickhouse, store, run_id, cancel.clone()));
        Self { cancel, task }
    }

    /// Stop syncing and send what is left, waiting at most `FINAL_FLUSH`.
    pub async fn finish(self) {
        self.cancel.cancel();
        let _ = self.task.await;
    }
}

struct RunCursor {
    run_id: String,
    sequence: i64,
    signature: Option<String>,
}

impl RunCursor {
    async fn sync(&mut self, ch: &ClickHouse, store: &Arc<ObserveRunStore>) -> Result<(), String> {
        let (sequence, sent) = ch.push_events(store, &self.run_id, self.sequence).await?;
        self.sequence = sequence;
        let run_id = self.run_id.clone();
        if let Some(run) = blocking(store, move |s| s.get_run(&run_id)).await? {
            let signature = serde_json::to_string(&run).map_err(|e| e.to_string())?;
            if self.signature.as_ref() != Some(&signature) {
                ch.push_runs(std::slice::from_ref(&run)).await?;
                self.signature = Some(signature);
            }
        }
        ch.record(&Ok(()), sent);
        Ok(())
    }
}

async fn mirror_loop(
    ch: Arc<ClickHouse>,
    store: Arc<ObserveRunStore>,
    run_id: String,
    cancel: CancellationToken,
) {
    let mut cursor: Option<RunCursor> = None;
    let mut backoff = SYNC_INTERVAL;
    loop {
        let attempt = async {
            ch.ensure_schema().await?;
            let cursor = match &mut cursor {
                Some(cursor) => cursor,
                None => {
                    let local = ch.backfill(&store).await?;
                    cursor.insert(RunCursor {
                        sequence: local.get(&run_id).copied().unwrap_or(0),
                        run_id: run_id.clone(),
                        signature: None,
                    })
                }
            };
            cursor.sync(&ch, &store).await
        };
        // Stopping abandons a slow attempt (a long backfill, a hung server):
        // delivery is at least once, so the next process resumes from what
        // ClickHouse has, and exit stays bounded by the final flush.
        let outcome = tokio::select! {
            _ = cancel.cancelled() => break,
            outcome = attempt => outcome,
        };
        let wait = match &outcome {
            Ok(()) => {
                backoff = SYNC_INTERVAL;
                SYNC_INTERVAL
            }
            Err(_) => {
                ch.record(&outcome, 0);
                let wait = backoff;
                backoff = (backoff * 2).min(MAX_BACKOFF);
                wait
            }
        };
        tokio::select! {
            _ = cancel.cancelled() => break,
            _ = tokio::time::sleep(wait) => {}
        }
    }
    let final_sync = async {
        ch.ensure_schema().await?;
        let mut cursor = match cursor {
            Some(cursor) => cursor,
            None => {
                let local = ch.backfill(&store).await?;
                RunCursor {
                    sequence: local.get(&run_id).copied().unwrap_or(0),
                    run_id: run_id.clone(),
                    signature: None,
                }
            }
        };
        cursor.sync(&ch, &store).await
    };
    match tokio::time::timeout(FINAL_FLUSH, final_sync).await {
        Ok(outcome) => {
            if outcome.is_err() {
                ch.record(&outcome, 0);
            }
        }
        Err(_) => ch.record(&Err("final ClickHouse sync timed out".into()), 0),
    }
}

#[cfg(test)]
#[path = "clickhouse_tests.rs"]
mod tests;
