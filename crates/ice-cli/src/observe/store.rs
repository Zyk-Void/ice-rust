//! Durable observation history. Mirrors TS `observe-store.ts`
//! (`openObserveRunStore`, `ObserveRunStore`; OBS-011..OBS-013): a dedicated
//! SQLite file of runs and ordered projected events, separate from canonical
//! session storage and never mutating it.
//!
//! Rust differences, deliberately:
//!
//! - The first `record_run` for a run starts its sequence at
//!   `MAX(sequence) + 1` instead of 1, so reopening an existing run cannot
//!   collide with already stored rows.
//! - Pruning deletes events and runs in one transaction.
//! - A corrupt stored payload fails the read with an error naming the row
//!   (TS throws from `JSON.parse` without context).

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use indexmap::IndexMap;
use regex::Regex;
use rusqlite::{params, Connection, OptionalExtension};

use super::recorder::{now_ms, ObservePersistence};
use super::types::*;

/// Run retention: 30 days by start time. Mirrors TS `OBSERVE_RETENTION_MS`.
pub const OBSERVE_RETENTION_MS: i64 = 30 * 24 * 60 * 60 * 1000;
/// Default/maximum page size for run and event queries.
pub const DEFAULT_QUERY_LIMIT: i64 = 100;
pub const MAX_QUERY_LIMIT: i64 = 10_000;
const REDACTED: &str = "[REDACTED]";

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS observe_runs (
	run_id TEXT PRIMARY KEY,
	session_id TEXT NOT NULL,
	main_thread_id TEXT NOT NULL,
	session_name TEXT,
	model TEXT,
	thinking_level TEXT,
	started_at_ms INTEGER NOT NULL,
	ended_at_ms INTEGER,
	status TEXT NOT NULL CHECK (status IN ('running', 'finished'))
);
CREATE INDEX IF NOT EXISTS observe_runs_session_idx
	ON observe_runs (session_id, started_at_ms DESC);
CREATE TABLE IF NOT EXISTS observe_events (
	run_id TEXT NOT NULL REFERENCES observe_runs(run_id) ON DELETE CASCADE,
	sequence INTEGER NOT NULL,
	ts_ms INTEGER NOT NULL,
	event_type TEXT NOT NULL,
	payload_json TEXT NOT NULL,
	PRIMARY KEY (run_id, sequence)
);
CREATE INDEX IF NOT EXISTS observe_events_run_ts_idx
	ON observe_events (run_id, ts_ms, sequence);
";

fn sensitive_key() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)^(?:api[_-]?key|(?:access[_-]?|refresh[_-]?)?token|secret|password|authorization)$",
        )
        .expect("valid regex")
    })
}

fn bearer() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)\bBearer\s+[A-Za-z0-9._~+/=-]+").expect("valid regex"))
}

fn assignment() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r#"(?i)\b((?:api[_-]?key|(?:access[_-]?|refresh[_-]?)?token|secret|password|authorization)["']?\s*[:=]\s*["']?)([^\s"',;}]+)"#,
        )
        .expect("valid regex")
    })
}

/// Replace credential-shaped text. Mirrors TS `redactCredentialText`: Bearer
/// credentials and `key=value` / `"key": "value"` assignments for API keys,
/// tokens, secrets, passwords and authorization values. This is a best-effort
/// filter, not a guarantee that arbitrary secrets are removed.
pub fn redact_credential_text(text: &str) -> String {
    let text = bearer().replace_all(text, "Bearer [REDACTED]");
    assignment()
        .replace_all(&text, |caps: &regex::Captures<'_>| {
            if caps[2].eq_ignore_ascii_case("bearer") || &caps[2] == REDACTED {
                caps[0].to_string()
            } else {
                format!("{}{REDACTED}", &caps[1])
            }
        })
        .into_owned()
}

/// Redact a JSON value in place of TS's `JSON.stringify` replacer: string
/// values under a sensitive key become `[REDACTED]`, every other string is
/// passed through [`redact_credential_text`]. Structure is preserved.
pub fn redact_value(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::String(s) => serde_json::Value::String(redact_credential_text(s)),
        serde_json::Value::Array(list) => {
            serde_json::Value::Array(list.iter().map(redact_value).collect())
        }
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.iter()
                .map(|(k, v)| {
                    let v = if v.is_string() && sensitive_key().is_match(k) {
                        serde_json::Value::String(REDACTED.into())
                    } else {
                        redact_value(v)
                    };
                    (k.clone(), v)
                })
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Clamp an optional query limit. Mirrors TS `normalizeLimit`.
pub fn normalize_limit(limit: Option<f64>) -> i64 {
    match limit {
        Some(n) if n.is_finite() => (n.floor() as i64).clamp(1, MAX_QUERY_LIMIT),
        _ => DEFAULT_QUERY_LIMIT,
    }
}

/// Validate a replay cursor. Mirrors TS: only a nonnegative safe integer.
pub fn normalize_cursor(after: Option<f64>) -> i64 {
    match after {
        Some(n)
            if n.is_finite() && n >= 0.0 && n.fract() == 0.0 && n <= 9_007_199_254_740_991.0 =>
        {
            n as i64
        }
        _ => 0,
    }
}

struct StoreInner {
    conn: Option<Connection>,
    next_sequence: HashMap<String, i64>,
}

/// The observation store. Implements [`ObservePersistence`] for the recorder
/// and the read API used by the dashboard server.
pub struct ObserveRunStore {
    inner: Mutex<StoreInner>,
}

fn closed() -> String {
    "Observe run store is closed".to_string()
}

fn sql_err(e: rusqlite::Error) -> String {
    e.to_string()
}

fn row_summary(row: &rusqlite::Row<'_>) -> rusqlite::Result<ObserveRunSummary> {
    let status: String = row.get(8)?;
    Ok(ObserveRunSummary {
        run_id: row.get(0)?,
        session_id: row.get(1)?,
        main_thread_id: row.get(2)?,
        session_name: row.get(3)?,
        model: row.get(4)?,
        thinking_level: row.get(5)?,
        started_at: row.get(6)?,
        ended_at: row.get(7)?,
        status: if status == "finished" {
            ObserveRunStatus::Finished
        } else {
            ObserveRunStatus::Running
        },
    })
}

const RUN_COLUMNS: &str = "run_id, session_id, main_thread_id, session_name, model, \
     thinking_level, started_at_ms, ended_at_ms, status";

impl ObserveRunStore {
    /// Open (creating) the store. Mirrors TS `openObserveRunStore`: creates
    /// the parent directory, sets WAL / journal limit / FULL sync / busy
    /// timeout, creates the schema and prunes runs older than the retention.
    pub fn open(path: &Path) -> Result<Self, String> {
        Self::open_with_retention(path, OBSERVE_RETENTION_MS)
    }

    pub fn open_with_retention(path: &Path, retention_ms: i64) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        // History holds full prompts and tool output: create it private
        // (SQLite gives the -wal/-shm files the database's mode).
        #[cfg(unix)]
        if !path.exists() {
            use std::os::unix::fs::OpenOptionsExt;
            let _ = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(path);
        }
        let mut conn = Connection::open(path).map_err(sql_err)?;
        conn.busy_timeout(std::time::Duration::from_millis(5_000))
            .map_err(sql_err)?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(sql_err)?;
        conn.pragma_update(None, "journal_size_limit", 67_108_864i64)
            .map_err(sql_err)?;
        conn.pragma_update(None, "synchronous", "FULL")
            .map_err(sql_err)?;
        conn.execute_batch(SCHEMA).map_err(sql_err)?;
        let cutoff = now_ms() - retention_ms;
        let tx = conn.transaction().map_err(sql_err)?;
        tx.execute(
            "DELETE FROM observe_events WHERE run_id IN \
             (SELECT run_id FROM observe_runs WHERE started_at_ms < ?1)",
            params![cutoff],
        )
        .map_err(sql_err)?;
        tx.execute(
            "DELETE FROM observe_runs WHERE started_at_ms < ?1",
            params![cutoff],
        )
        .map_err(sql_err)?;
        tx.commit().map_err(sql_err)?;
        Ok(Self {
            inner: Mutex::new(StoreInner {
                conn: Some(conn),
                next_sequence: HashMap::new(),
            }),
        })
    }

    fn with_conn<T>(
        &self,
        f: impl FnOnce(&Connection, &mut HashMap<String, i64>) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let StoreInner {
            conn,
            next_sequence,
        } = &mut *inner;
        let conn = conn.as_ref().ok_or_else(closed)?;
        f(conn, next_sequence)
    }

    /// Close the connection. Idempotent; later calls other than
    /// `finish_run` fail with `Observe run store is closed`.
    pub fn close(&self) -> Result<(), String> {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(conn) = inner.conn.take() {
            conn.close().map_err(|(_, e)| e.to_string())?;
        }
        Ok(())
    }

    pub fn list_runs(&self, limit: Option<f64>) -> Result<Vec<ObserveRunSummary>, String> {
        let limit = normalize_limit(limit);
        self.with_conn(|conn, _| {
            let mut stmt = conn
                .prepare(&format!(
                    "SELECT {RUN_COLUMNS} FROM observe_runs ORDER BY started_at_ms DESC LIMIT ?1"
                ))
                .map_err(sql_err)?;
            let rows = stmt
                .query_map(params![limit], row_summary)
                .map_err(sql_err)?;
            rows.collect::<Result<_, _>>().map_err(sql_err)
        })
    }

    /// Running runs with at least one event at or after `active_since`.
    /// Recent-event evidence, not a process-liveness check (TS parity).
    pub fn list_live_runs(
        &self,
        active_since: i64,
        limit: Option<f64>,
    ) -> Result<Vec<ObserveRunSummary>, String> {
        let limit = normalize_limit(limit);
        self.with_conn(|conn, _| {
            let mut stmt = conn
                .prepare(&format!(
                    "SELECT {RUN_COLUMNS} FROM observe_runs r WHERE status = 'running' \
                     AND EXISTS (SELECT 1 FROM observe_events e WHERE e.run_id = r.run_id \
                     AND e.ts_ms >= ?1) ORDER BY started_at_ms DESC LIMIT ?2"
                ))
                .map_err(sql_err)?;
            let rows = stmt
                .query_map(params![active_since, limit], row_summary)
                .map_err(sql_err)?;
            rows.collect::<Result<_, _>>().map_err(sql_err)
        })
    }

    pub fn get_run(&self, run_id: &str) -> Result<Option<ObserveRunSummary>, String> {
        self.with_conn(|conn, _| {
            conn.query_row(
                &format!("SELECT {RUN_COLUMNS} FROM observe_runs WHERE run_id = ?1"),
                params![run_id],
                row_summary,
            )
            .optional()
            .map_err(sql_err)
        })
    }

    pub fn get_latest_run_for_session(
        &self,
        session_id: &str,
    ) -> Result<Option<ObserveRunSummary>, String> {
        self.with_conn(|conn, _| {
            conn.query_row(
                &format!(
                    "SELECT {RUN_COLUMNS} FROM observe_runs WHERE session_id = ?1 \
                     ORDER BY started_at_ms DESC LIMIT 1"
                ),
                params![session_id],
                row_summary,
            )
            .optional()
            .map_err(sql_err)
        })
    }

    /// Events with `sequence > after`, ascending.
    pub fn get_events(
        &self,
        run_id: &str,
        after: Option<f64>,
        limit: Option<f64>,
    ) -> Result<Vec<ObserveReplayEvent>, String> {
        let after = normalize_cursor(after);
        let limit = normalize_limit(limit);
        self.with_conn(|conn, _| {
            let mut stmt = conn
                .prepare(
                    "SELECT sequence, ts_ms, event_type, payload_json FROM observe_events \
                     WHERE run_id = ?1 AND sequence > ?2 ORDER BY sequence ASC LIMIT ?3",
                )
                .map_err(sql_err)?;
            let rows = stmt
                .query_map(params![run_id, after, limit], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                })
                .map_err(sql_err)?;
            let mut events = Vec::new();
            for row in rows {
                let (sequence, ts, event_type, payload) = row.map_err(sql_err)?;
                let payload = serde_json::from_str(&payload)
                    .map_err(|e| format!("corrupt observe event {run_id}#{sequence}: {e}"))?;
                events.push(ObserveReplayEvent {
                    sequence,
                    ts,
                    event_type,
                    payload,
                });
            }
            Ok(events)
        })
    }

    pub fn get_latest_sequence(&self, run_id: &str) -> Result<i64, String> {
        self.with_conn(|conn, _| {
            conn.query_row(
                "SELECT COALESCE(MAX(sequence), 0) FROM observe_events WHERE run_id = ?1",
                params![run_id],
                |row| row.get(0),
            )
            .map_err(sql_err)
        })
    }

    /// Reconstruct a run's state by folding its events in sequence order.
    /// Mirrors TS `getState`: without a limit every page is folded; with a
    /// limit only the first page is.
    pub fn get_state(
        &self,
        run_id: &str,
        limit: Option<f64>,
    ) -> Result<Option<ObserveState>, String> {
        let Some(run) = self.get_run(run_id)? else {
            return Ok(None);
        };
        let mut header = ObserveHeader {
            run_id: run.run_id.clone(),
            session_id: run.session_id.clone(),
            main_thread_id: run.main_thread_id.clone(),
            main_thread_name: match run.session_name.as_deref().filter(|s| !s.is_empty()) {
                Some(name) => name.to_string(),
                None => format!(
                    "main · {}",
                    run.session_id.chars().take(10).collect::<String>()
                ),
            },
            session_name: run.session_name.clone(),
            model: run.model.clone(),
            thinking_level: run.thinking_level.clone(),
            context: None,
            usage: ObserveUsage::default(),
            started_at: run.started_at,
        };
        let mut items: IndexMap<u64, ObserveItem> = IndexMap::new();
        let mut lanes: IndexMap<String, ObserveLane> = IndexMap::new();
        let mut threads: IndexMap<String, ObserveThread> = IndexMap::new();
        let mut after = 0.0;
        loop {
            let page = self.get_events(
                run_id,
                Some(after),
                Some(limit.unwrap_or(MAX_QUERY_LIMIT as f64)),
            )?;
            let Some(last) = page.last() else {
                break;
            };
            after = last.sequence as f64;
            for event in page {
                match event.payload {
                    ObserveRecorderEvent::Header { header: h } => header = h,
                    ObserveRecorderEvent::Activity { mut item }
                    | ObserveRecorderEvent::ActivityUpdate { mut item } => {
                        if item.run_id.is_empty() {
                            item.run_id = run_id.to_string();
                        }
                        items.insert(item.id, item);
                    }
                    ObserveRecorderEvent::Lane { lane } => {
                        lanes.insert(lane.stream_key.clone(), lane);
                    }
                    ObserveRecorderEvent::Thread { thread } => {
                        threads.insert(thread.identity.thread_key.clone(), thread);
                    }
                }
            }
            if limit.is_some() {
                break;
            }
        }
        Ok(Some(ObserveState {
            header,
            items: items.into_values().collect(),
            lanes: lanes.into_values().collect(),
            threads: threads.into_values().collect(),
        }))
    }
}

impl ObservePersistence for ObserveRunStore {
    fn record_run(&self, header: &ObserveHeader) -> Result<(), String> {
        self.with_conn(|conn, seq| {
            conn.execute(
                "INSERT INTO observe_runs (run_id, session_id, main_thread_id, session_name, \
                 model, thinking_level, started_at_ms, status) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'running') \
                 ON CONFLICT(run_id) DO UPDATE SET session_name = excluded.session_name, \
                 model = excluded.model, thinking_level = excluded.thinking_level",
                params![
                    header.run_id,
                    header.session_id,
                    header.main_thread_id,
                    header.session_name,
                    header.model,
                    header.thinking_level,
                    header.started_at,
                ],
            )
            .map_err(sql_err)?;
            if !seq.contains_key(&header.run_id) {
                let max: i64 = conn
                    .query_row(
                        "SELECT COALESCE(MAX(sequence), 0) FROM observe_events WHERE run_id = ?1",
                        params![header.run_id],
                        |row| row.get(0),
                    )
                    .map_err(sql_err)?;
                seq.insert(header.run_id.clone(), max + 1);
            }
            Ok(())
        })
    }

    fn record_event(
        &self,
        event: &ObserveRecorderEvent,
        header: &ObserveHeader,
    ) -> Result<(), String> {
        if !self.with_conn(|_, seq| Ok(seq.contains_key(&header.run_id)))? {
            self.record_run(header)?;
        }
        self.with_conn(|conn, seq| {
            let ts = match event {
                ObserveRecorderEvent::Activity { item }
                | ObserveRecorderEvent::ActivityUpdate { item } => item.ts,
                _ => now_ms(),
            };
            let value = serde_json::to_value(event).map_err(|e| e.to_string())?;
            let payload =
                serde_json::to_string(&redact_value(&value)).map_err(|e| e.to_string())?;
            let sequence = *seq.get(&header.run_id).unwrap_or(&1);
            conn.execute(
                "INSERT INTO observe_events (run_id, sequence, ts_ms, event_type, payload_json) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![header.run_id, sequence, ts, event.type_tag(), payload],
            )
            .map_err(sql_err)?;
            seq.insert(header.run_id.clone(), sequence + 1);
            Ok(())
        })
    }

    fn finish_run(&self, run_id: &str, ended_at: i64) -> Result<(), String> {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let Some(conn) = inner.conn.as_ref() else {
            return Ok(());
        };
        conn.execute(
            "UPDATE observe_runs SET ended_at_ms = ?2, status = 'finished' WHERE run_id = ?1",
            params![run_id, ended_at],
        )
        .map_err(sql_err)?;
        inner.next_sequence.remove(run_id);
        Ok(())
    }

    /// The store is shared (recorder, server, other recorders), so a
    /// recorder's stop does not close it; it closes when the last handle
    /// drops or through [`ObserveRunStore::close`].
    fn close(&self) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
