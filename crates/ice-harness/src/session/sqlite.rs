//! SQLite-backed session repository.
//!
//! `state_json` remains the canonical Rust reducer snapshot. SQLite writer
//! operations load a candidate, apply one mutation, and persist the snapshot
//! under a single `BEGIN IMMEDIATE` transaction and fenced writer lease. The
//! per-entry FTS5 projection is derived state maintained by that same
//! transaction; it is intentionally not the TypeScript relational schema.

mod lease;
mod schema;
mod search;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::Duration;

use async_trait::async_trait;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use tokio::runtime::Handle;
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::error::{SessionError, SessionResult};
use crate::session::memory::{stamp_record, Clock, SystemClock};
use crate::session::state::SessionState;
use crate::session::types::{
    provisioned_into_entry, BranchBounds, Entry, EntryQuery, ForkOptions, IdGenerator, LanePointer,
    LaneRecord, LogItem, LogOptions, OperationStartedRecord, ProvisionedEntry, RecordQuery,
    SessionCreateOptions, SessionMetadata, SessionMutation, SessionRepo, SessionStats,
    SessionStorage,
};

/// Fenced SQLite writer timing. Values are milliseconds; both durations must be
/// positive and the heartbeat interval must be shorter than the TTL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqliteWriterLeaseOptions {
    pub ttl_ms: u64,
    pub heartbeat_interval_ms: u64,
}

impl Default for SqliteWriterLeaseOptions {
    fn default() -> Self {
        Self {
            ttl_ms: 30_000,
            heartbeat_interval_ms: 10_000,
        }
    }
}

impl SqliteWriterLeaseOptions {
    fn validate(&self) -> SessionResult<(i64, i64)> {
        if self.ttl_ms == 0 || self.heartbeat_interval_ms == 0 {
            return Err(SessionError::storage(
                "SQLite writer lease durations must be positive",
            ));
        }
        if self.heartbeat_interval_ms >= self.ttl_ms {
            return Err(SessionError::storage(
                "SQLite writer lease heartbeat interval must be less than its TTL",
            ));
        }
        let ttl = i64::try_from(self.ttl_ms)
            .map_err(|_| SessionError::storage("SQLite writer lease TTL is too large"))?;
        let heartbeat = i64::try_from(self.heartbeat_interval_ms)
            .map_err(|_| SessionError::storage("SQLite writer heartbeat interval is too large"))?;
        Ok((ttl, heartbeat))
    }
}

/// SQLite-specific metadata carried alongside the shared SessionRepo metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct SqliteSessionMetadata {
    pub id: String,
    pub created_at: i64,
    pub parent_session_id: Option<String>,
    pub cwd: String,
    /// Resolved canonical snapshot database path.
    pub path: PathBuf,
    pub metadata: Option<serde_json::Map<String, serde_json::Value>>,
}

impl SqliteSessionMetadata {
    pub fn base_metadata(&self) -> SessionMetadata {
        SessionMetadata {
            id: self.id.clone(),
            created_at: self.created_at,
            parent_session_id: self.parent_session_id.clone(),
        }
    }
}

/// Create/fork fields that are not present on the shared `SessionRepo` trait.
#[derive(Debug, Clone, Default)]
pub struct SqliteSessionCreateOptions {
    pub id: Option<String>,
    pub parent_session_id: Option<String>,
    pub cwd: String,
    pub metadata: Option<serde_json::Map<String, serde_json::Value>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SqliteSessionListOptions {
    /// `Some("")` is an exact empty-string filter, not an omitted filter.
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SqliteSessionSearchOptions {
    pub text: String,
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SqliteSessionSearchHit {
    pub metadata: SqliteSessionMetadata,
    pub entry_id: String,
    pub timestamp: String,
    pub score: f64,
}

#[derive(Clone)]
pub struct SqliteSessionStorage {
    inner: Arc<StorageInner>,
}

struct StorageInner {
    repository: Arc<RepoInner>,
    metadata: SqliteSessionMetadata,
    lease: lease::LeaseToken,
    closed: AtomicBool,
    lease_lost: AtomicBool,
    lease_released: AtomicBool,
    heartbeat: Mutex<Option<JoinHandle<()>>>,
}

impl SqliteSessionStorage {
    fn new(
        repository: Arc<RepoInner>,
        metadata: SqliteSessionMetadata,
        lease: lease::LeaseToken,
        runtime: Handle,
    ) -> Self {
        let inner = Arc::new(StorageInner {
            repository: repository.clone(),
            metadata,
            lease,
            closed: AtomicBool::new(false),
            lease_lost: AtomicBool::new(false),
            lease_released: AtomicBool::new(false),
            heartbeat: Mutex::new(None),
        });
        let weak = Arc::downgrade(&inner);
        let interval = repository.heartbeat_interval;
        let task = runtime.spawn(async move {
            heartbeat_loop(weak, interval).await;
        });
        *inner
            .heartbeat
            .lock()
            .expect("heartbeat task mutex not poisoned") = Some(task);
        Self { inner }
    }

    /// Base metadata required by the shared `SessionStorage` API.
    pub fn metadata(&self) -> SessionMetadata {
        self.inner.metadata.base_metadata()
    }

    pub fn sqlite_metadata(&self) -> SqliteSessionMetadata {
        self.inner.metadata.clone()
    }

    /// Idempotently stop heartbeats and release this handle's exact lease fence.
    pub async fn close(&self) -> SessionResult<()> {
        self.inner.close()
    }
}

impl StorageInner {
    fn ensure_writable(&self) -> SessionResult<()> {
        if self.lease_lost.load(Ordering::SeqCst) {
            return Err(self.lost_lease_error());
        }
        if self.closed.load(Ordering::SeqCst) {
            return Err(SessionError::storage(format!(
                "SQLite session {} storage is closed",
                self.metadata.id
            )));
        }
        Ok(())
    }

    fn lost_lease_error(&self) -> SessionError {
        SessionError::storage(format!(
            "SQLite session {} writer lease was lost",
            self.metadata.id
        ))
    }

    fn write<R>(
        &self,
        mutate: impl FnOnce(&mut SessionState) -> SessionResult<R>,
    ) -> SessionResult<R> {
        self.ensure_writable()?;
        let ttl_ms = self.repository.ttl_ms;
        self.repository.with_connection(|connection| {
            self.ensure_writable()?;
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(|error| schema::storage_error("begin SQLite session write", error))?;
            let now_ms = self.repository.clock.now_ms();
            if let Err(error) = lease::renew(&transaction, &self.lease, now_ms, ttl_ms) {
                if error.message.ends_with("writer lease was lost") {
                    self.lease_lost.store(true, Ordering::SeqCst);
                }
                return Err(error);
            }
            let mut candidate = load_state(&transaction, &self.metadata.id)?;
            let value = mutate(&mut candidate)?;
            let state_json = serde_json::to_string(&candidate).map_err(|error| {
                SessionError::storage(format!("serialize SQLite session: {error}"))
            })?;
            let changed = transaction
                .execute(
                    "UPDATE sessions SET state_json = ?1 WHERE id = ?2",
                    params![state_json, self.metadata.id],
                )
                .map_err(|error| schema::storage_error("persist SQLite session snapshot", error))?;
            if changed != 1 {
                return Err(SessionError::not_found(format!(
                    "Session not found: {}",
                    self.metadata.id
                )));
            }
            search::sync_session(
                &transaction,
                &self.metadata.id,
                &self.metadata.cwd,
                &candidate,
            )?;
            transaction
                .commit()
                .map_err(|error| schema::storage_error("commit SQLite session write", error))?;
            Ok(value)
        })
    }

    fn read<R>(&self, query: impl FnOnce(&SessionState) -> SessionResult<R>) -> SessionResult<R> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(SessionError::storage(format!(
                "SQLite session {} storage is closed",
                self.metadata.id
            )));
        }
        self.repository.with_connection(|connection| {
            if self.closed.load(Ordering::SeqCst) {
                return Err(SessionError::storage(format!(
                    "SQLite session {} storage is closed",
                    self.metadata.id
                )));
            }
            let state = load_state(connection, &self.metadata.id)?;
            query(&state)
        })
    }

    fn close(&self) -> SessionResult<()> {
        self.closed.store(true, Ordering::SeqCst);
        if let Ok(mut heartbeat) = self.heartbeat.lock() {
            if let Some(task) = heartbeat.take() {
                task.abort();
            }
        }
        self.release_lease()
    }

    fn release_lease(&self) -> SessionResult<()> {
        if self.lease_released.load(Ordering::SeqCst) {
            return Ok(());
        }
        self.repository.with_connection(|connection| {
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(|error| schema::storage_error("begin SQLite lease release", error))?;
            lease::release(&transaction, &self.lease)?;
            transaction
                .commit()
                .map_err(|error| schema::storage_error("commit SQLite lease release", error))?;
            self.lease_released.store(true, Ordering::SeqCst);
            Ok(())
        })
    }

    fn heartbeat(&self) -> SessionResult<()> {
        if self.closed.load(Ordering::SeqCst) || self.lease_lost.load(Ordering::SeqCst) {
            return Ok(());
        }
        let result = self.repository.with_connection(|connection| {
            if self.closed.load(Ordering::SeqCst) {
                return Ok(());
            }
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(|error| schema::storage_error("begin SQLite lease heartbeat", error))?;
            let now_ms = self.repository.clock.now_ms();
            lease::renew(&transaction, &self.lease, now_ms, self.repository.ttl_ms)?;
            transaction
                .commit()
                .map_err(|error| schema::storage_error("commit SQLite lease heartbeat", error))
        });
        if result
            .as_ref()
            .err()
            .map(|error| error.message.ends_with("writer lease was lost"))
            .unwrap_or(false)
        {
            self.lease_lost.store(true, Ordering::SeqCst);
        }
        result
    }
}

impl Drop for StorageInner {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::SeqCst);
        if let Ok(heartbeat) = self.heartbeat.get_mut() {
            if let Some(task) = heartbeat.take() {
                task.abort();
            }
        }
        // RAII release is best effort; explicit close reports database errors.
        if !self.lease_released.load(Ordering::SeqCst) {
            let _ = self.release_lease();
        }
    }
}

async fn heartbeat_loop(weak: Weak<StorageInner>, interval_ms: u64) {
    loop {
        tokio::time::sleep(Duration::from_millis(interval_ms)).await;
        let Some(storage) = weak.upgrade() else {
            return;
        };
        if storage.closed.load(Ordering::SeqCst) || storage.lease_lost.load(Ordering::SeqCst) {
            return;
        }
        // Transient database errors are retried next interval. Writes always
        // validate the lease transactionally, so a missed renewal is not silent.
        let _ = storage.heartbeat();
        drop(storage);
    }
}

#[async_trait]
impl SessionStorage for SqliteSessionStorage {
    fn metadata(&self) -> SessionMetadata {
        SqliteSessionStorage::metadata(self)
    }

    async fn get_lanes(&self) -> SessionResult<Vec<LanePointer>> {
        self.inner.read(|state| Ok(state.get_lanes()))
    }

    async fn create_lane(&self, lane: &str, at: Option<&str>) -> SessionResult<()> {
        self.inner.write(|state| {
            state.validate_new_lane(lane)?;
            state.validate_target(at)?;
            let seq = state.next_sequence();
            state.apply_mutation(SessionMutation::Lane {
                seq,
                lane: lane.to_string(),
                leaf_id: at.map(str::to_string),
            })?;
            Ok(())
        })
    }

    async fn move_lane(&self, lane: &str, to: Option<&str>) -> SessionResult<()> {
        self.inner.write(|state| {
            state.require_lane(lane)?;
            state.validate_target(to)?;
            let seq = state.next_sequence();
            state.apply_mutation(SessionMutation::Lane {
                seq,
                lane: lane.to_string(),
                leaf_id: to.map(str::to_string),
            })?;
            Ok(())
        })
    }

    async fn append_entry(&self, entry: ProvisionedEntry, lane: &str) -> SessionResult<Entry> {
        self.inner.write(|state| {
            let parent_id = state.require_lane(lane)?;
            state.validate_unused_id(&entry.id)?;
            let seq = state.next_sequence();
            let timestamp = self.inner.repository.clock.now_ms();
            let full = provisioned_into_entry(entry, seq, parent_id.clone(), timestamp);
            state.apply_mutation(SessionMutation::Entry {
                seq,
                timestamp,
                lane: Some(lane.to_string()),
                entry: full.clone(),
            })?;
            Ok(full)
        })
    }

    async fn append_record(&self, record: LaneRecord) -> SessionResult<LaneRecord> {
        self.inner.write(|state| {
            let lane = record.lane().to_string();
            state.require_lane(&lane)?;
            state.validate_unused_id(record.id())?;
            if matches!(record, LaneRecord::OperationStarted(_)) {
                if let Some(current) = state.find_open_operations(&lane, Some(1))?.first() {
                    return Err(SessionError::storage(format!(
                        "Lane {lane} already has an open operation {}",
                        current.base.id
                    )));
                }
            }
            let seq = state.next_sequence();
            let timestamp = self.inner.repository.clock.now_ms();
            let stamped = stamp_record(record, seq, lane, timestamp);
            state.apply_mutation(SessionMutation::Record {
                record: stamped.clone(),
            })?;
            Ok(stamped)
        })
    }

    async fn get_entry(&self, id: &str) -> SessionResult<Option<Entry>> {
        self.inner.read(|state| Ok(state.get_entry(id).cloned()))
    }

    async fn find_entries(&self, query: &EntryQuery) -> SessionResult<Vec<Entry>> {
        self.inner.read(|state| state.find_entries(query))
    }

    async fn find_entries_on_branch(
        &self,
        query: &EntryQuery,
        bounds: &BranchBounds,
        start: &str,
    ) -> SessionResult<Vec<Entry>> {
        self.inner
            .read(|state| state.find_entries_on_branch(query, bounds, start))
    }

    async fn find_records(&self, query: &RecordQuery) -> SessionResult<Vec<LaneRecord>> {
        self.inner.read(|state| state.find_records(query))
    }

    async fn find_open_operations(
        &self,
        lane: &str,
        limit: Option<usize>,
    ) -> SessionResult<Vec<OperationStartedRecord>> {
        self.inner
            .read(|state| state.find_open_operations(lane, limit))
    }

    async fn get_log(&self, options: &LogOptions) -> SessionResult<Vec<LogItem>> {
        self.inner.read(|state| state.get_log(options))
    }

    async fn get_name(&self) -> SessionResult<Option<String>> {
        self.inner
            .read(|state| Ok(state.get_name().map(str::to_string)))
    }

    async fn set_name(&self, name: Option<&str>) -> SessionResult<()> {
        self.inner.write(|state| {
            let seq = state.next_sequence();
            state.apply_mutation(SessionMutation::FactName {
                seq,
                name: name.map(str::to_string),
            })?;
            Ok(())
        })
    }

    async fn get_label(&self, id: &str) -> SessionResult<Option<String>> {
        self.inner
            .read(|state| Ok(state.get_label(id).map(str::to_string)))
    }

    async fn set_label(&self, id: &str, label: Option<&str>) -> SessionResult<()> {
        self.inner.write(|state| {
            state.validate_target(Some(id))?;
            let seq = state.next_sequence();
            state.apply_mutation(SessionMutation::FactLabel {
                seq,
                target_id: id.to_string(),
                label: label.map(str::to_string),
            })?;
            Ok(())
        })
    }

    async fn get_stats(&self) -> SessionResult<SessionStats> {
        self.inner.read(|state| Ok(state.get_stats().clone()))
    }
}

pub struct SqliteSessionRepo {
    inner: Arc<RepoInner>,
}

struct RepoInner {
    path: PathBuf,
    connection: Mutex<Option<Connection>>,
    active: Mutex<HashMap<String, Weak<StorageInner>>>,
    lifecycle: Mutex<()>,
    clock: Arc<dyn Clock>,
    ids: Arc<dyn IdGenerator>,
    closing: AtomicBool,
    ttl_ms: i64,
    heartbeat_interval: u64,
}

impl SqliteSessionRepo {
    pub fn open(path: impl AsRef<Path>) -> SessionResult<Self> {
        Self::with_clock_ids(
            path,
            Arc::new(SystemClock),
            Arc::new(crate::session::DefaultIdGenerator::new()),
        )
    }

    pub fn with_clock_ids(
        path: impl AsRef<Path>,
        clock: Arc<dyn Clock>,
        ids: Arc<dyn IdGenerator>,
    ) -> SessionResult<Self> {
        Self::with_writer_lease(path, clock, ids, SqliteWriterLeaseOptions::default())
    }

    pub fn open_with_lease_options(
        path: impl AsRef<Path>,
        writer_lease: SqliteWriterLeaseOptions,
    ) -> SessionResult<Self> {
        Self::with_writer_lease(
            path,
            Arc::new(SystemClock),
            Arc::new(crate::session::DefaultIdGenerator::new()),
            writer_lease,
        )
    }

    pub fn with_writer_lease(
        path: impl AsRef<Path>,
        clock: Arc<dyn Clock>,
        ids: Arc<dyn IdGenerator>,
        writer_lease: SqliteWriterLeaseOptions,
    ) -> SessionResult<Self> {
        let (ttl_ms, heartbeat_interval) = writer_lease.validate()?;
        let path = resolve_path(path.as_ref())?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| SessionError::storage(format!("create SQLite parent: {error}")))?;
        }
        let connection = open_connection(&path)?;
        Ok(Self {
            inner: Arc::new(RepoInner {
                path,
                connection: Mutex::new(Some(connection)),
                active: Mutex::new(HashMap::new()),
                lifecycle: Mutex::new(()),
                closing: AtomicBool::new(false),
                clock,
                ids,
                ttl_ms,
                heartbeat_interval: heartbeat_interval as u64,
            }),
        })
    }

    pub fn path(&self) -> &Path {
        &self.inner.path
    }

    /// Rich create API, preserving cwd and application-owned metadata.
    pub async fn create_with_metadata(
        &self,
        options: &SqliteSessionCreateOptions,
    ) -> SessionResult<SqliteSessionStorage> {
        let _lifecycle = self.lifecycle_guard()?;
        self.create_storage(options, SessionState::new())
    }

    /// Open an existing session and claim its writer lease. Repeated opens on
    /// this repository return another clone of the exact same storage handle.
    pub async fn open_with_metadata(
        &self,
        metadata: &SqliteSessionMetadata,
    ) -> SessionResult<SqliteSessionStorage> {
        self.validate_metadata_path(metadata)?;
        self.open_session(&metadata.id)
    }

    pub async fn list_with_options(
        &self,
        options: &SqliteSessionListOptions,
    ) -> SessionResult<Vec<SqliteSessionMetadata>> {
        let _lifecycle = self.lifecycle_guard()?;
        self.inner.with_connection(|connection| {
            let mut statement = connection
                .prepare(
                    "SELECT id, created_at, parent_session_id, cwd, metadata_json
                     FROM sessions WHERE (?1 IS NULL OR cwd = ?1) ORDER BY created_at DESC",
                )
                .map_err(|error| schema::storage_error("prepare SQLite session list", error))?;
            let rows = statement
                .query_map([options.cwd.as_deref()], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, Option<String>>(4)?,
                    ))
                })
                .map_err(|error| schema::storage_error("query SQLite session list", error))?;
            let raw_rows: Vec<_> = rows
                .map(|row| {
                    row.map_err(|error| {
                        schema::storage_error("read SQLite session metadata", error)
                    })
                })
                .collect::<SessionResult<_>>()?;
            raw_rows
                .into_iter()
                .map(|(id, created_at, parent, cwd, metadata)| {
                    decode_metadata(id, created_at, parent, cwd, metadata, &self.inner.path)
                })
                .collect()
        })
    }

    /// Read-only list helper for discovery. A missing database returns an empty
    /// list and is never created; cwd uses exact matching, including "".
    pub fn list_existing(
        path: impl AsRef<Path>,
        options: &SqliteSessionListOptions,
    ) -> SessionResult<Vec<SqliteSessionMetadata>> {
        let path = resolve_path(path.as_ref())?;
        if !path.exists() {
            return Ok(Vec::new());
        }
        let connection =
            Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .map_err(|error| {
                    schema::storage_error("open SQLite database for discovery", error)
                })?;
        if !schema::inspect_schema(&connection)? {
            return Ok(Vec::new());
        }
        let has_cwd = has_column(&connection, "cwd")?;
        let has_metadata = has_column(&connection, "metadata_json")?;
        if !has_cwd
            && options
                .cwd
                .as_deref()
                .map(|cwd| !cwd.is_empty())
                .unwrap_or(false)
        {
            return Ok(Vec::new());
        }
        let sql = match (has_cwd, has_metadata) {
            (true, true) => "SELECT id, created_at, parent_session_id, cwd, metadata_json FROM sessions WHERE (?1 IS NULL OR cwd = ?1) ORDER BY created_at DESC",
            (true, false) => "SELECT id, created_at, parent_session_id, cwd, NULL AS metadata_json FROM sessions WHERE (?1 IS NULL OR cwd = ?1) ORDER BY created_at DESC",
            (false, true) => "SELECT id, created_at, parent_session_id, '' AS cwd, metadata_json FROM sessions ORDER BY created_at DESC",
            (false, false) => "SELECT id, created_at, parent_session_id, '' AS cwd, NULL AS metadata_json FROM sessions ORDER BY created_at DESC",
        };
        let raw_rows = if has_cwd {
            let mut statement = connection
                .prepare(sql)
                .map_err(|error| schema::storage_error("prepare read-only SQLite list", error))?;
            let rows = statement
                .query_map([options.cwd.as_deref()], read_session_list_row)
                .map_err(|error| schema::storage_error("query read-only SQLite list", error))?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|error| schema::storage_error("read SQLite discovery row", error))?
        } else {
            // The legacy query has no placeholders. Do not bind an unused NULL
            // parameter: rusqlite rejects a parameter-count mismatch even when
            // the value would otherwise be ignored.
            let mut statement = connection
                .prepare(sql)
                .map_err(|error| schema::storage_error("prepare read-only SQLite list", error))?;
            let rows = statement
                .query_map([], read_session_list_row)
                .map_err(|error| schema::storage_error("query read-only SQLite list", error))?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|error| schema::storage_error("read SQLite discovery row", error))?
        };
        raw_rows
            .into_iter()
            .map(|(id, created_at, parent, cwd, metadata)| {
                decode_metadata(id, created_at, parent, cwd, metadata, &path)
            })
            .collect()
    }

    pub async fn fork_with_metadata(
        &self,
        source: &SqliteSessionMetadata,
        options: &SqliteSessionCreateOptions,
        fork: &ForkOptions,
    ) -> SessionResult<SqliteSessionStorage> {
        self.validate_metadata_path(source)?;
        // Do not retain any connection guard across this await.
        let source_storage = self.open_with_metadata(source).await?;
        let source_state = source_storage.inner.read(|state| Ok(state.clone()))?;
        let metadata = SqliteSessionCreateOptions {
            id: options.id.clone(),
            parent_session_id: options
                .parent_session_id
                .clone()
                .or_else(|| Some(source.id.clone())),
            cwd: source.cwd.clone(),
            metadata: options.metadata.clone().or_else(|| source.metadata.clone()),
        };
        let candidate = source_state.create_fork_state(fork)?;
        let _lifecycle = self.lifecycle_guard()?;
        self.create_storage(&metadata, candidate)
    }

    pub async fn delete_with_metadata(
        &self,
        metadata: &SqliteSessionMetadata,
    ) -> SessionResult<()> {
        self.validate_metadata_path(metadata)?;
        self.delete_id(&metadata.id)
    }

    /// Drain and release this repository's live handles, close its connection,
    /// and permit a later call to reopen it. Repeated close is harmless.
    pub async fn close(&self) -> SessionResult<()> {
        let _lifecycle = self.lifecycle_guard()?;
        self.inner.closing.store(true, Ordering::SeqCst);
        let active = {
            let mut handles = self
                .inner
                .active
                .lock()
                .map_err(|_| SessionError::storage("SQLite active-handle registry poisoned"))?;
            let active = handles
                .values()
                .filter_map(Weak::upgrade)
                .collect::<Vec<_>>();
            handles.clear();
            active
        };
        let mut first_error = None;
        for handle in active {
            if let Err(error) = handle.close() {
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
        let mut connection = self
            .inner
            .connection
            .lock()
            .map_err(|_| SessionError::storage("SQLite connection mutex poisoned"))?;
        connection.take();
        if let Some(error) = first_error {
            Err(error)
        } else {
            Ok(())
        }
    }

    /// Indexed search over complete serialized Entry values. The first
    /// nonblank search installs/backfills the co-located FTS5 projection.
    pub async fn search_sessions(
        &self,
        options: SqliteSessionSearchOptions,
    ) -> SessionResult<Vec<SqliteSessionSearchHit>> {
        if options.text.trim().is_empty() {
            return Ok(Vec::new());
        }
        let _lifecycle = self.lifecycle_guard()?;
        self.inner.with_connection(|connection| {
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(|error| schema::storage_error("begin SQLite FTS search", error))?;
            let hits = search::search(&transaction, &self.inner.path, &options)?;
            transaction.commit().map_err(|error| {
                schema::storage_error("commit SQLite FTS initialization", error)
            })?;
            Ok(hits)
        })
    }

    /// Explicitly repair/rebuild the derived FTS projection without replacing
    /// canonical state or changing existing projection rowids.
    pub async fn rebuild_search_index(&self) -> SessionResult<()> {
        let _lifecycle = self.lifecycle_guard()?;
        self.inner.with_connection(|connection| {
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(|error| schema::storage_error("begin SQLite FTS rebuild", error))?;
            search::rebuild(&transaction)?;
            transaction
                .commit()
                .map_err(|error| schema::storage_error("commit SQLite FTS rebuild", error))
        })
    }

    fn create_storage(
        &self,
        options: &SqliteSessionCreateOptions,
        state: SessionState,
    ) -> SessionResult<SqliteSessionStorage> {
        let runtime = current_runtime()?;
        let id = options.id.clone().unwrap_or_else(|| self.inner.ids.next());
        let metadata = SqliteSessionMetadata {
            id: id.clone(),
            created_at: self.inner.clock.now_ms(),
            parent_session_id: options.parent_session_id.clone(),
            cwd: options.cwd.clone(),
            path: self.inner.path.clone(),
            metadata: options.metadata.clone(),
        };
        let state_json = serde_json::to_string(&state)
            .map_err(|error| SessionError::storage(format!("serialize SQLite state: {error}")))?;
        let metadata_json = metadata
            .metadata
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|error| {
                SessionError::storage(format!("serialize SQLite metadata: {error}"))
            })?;
        let owner_id = Uuid::new_v4().to_string();
        let lease_token = self.inner.with_connection(|connection| {
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(|error| schema::storage_error("begin SQLite session create", error))?;
            transaction
                .execute(
                    "INSERT INTO sessions(id, created_at, parent_session_id, state_json, cwd, metadata_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![id, metadata.created_at, metadata.parent_session_id, state_json, metadata.cwd, metadata_json],
                )
                .map_err(|error| map_insert_error(&id, error))?;
            search::sync_session(&transaction, &id, &metadata.cwd, &state)?;
            let lease = lease::acquire(
                &transaction,
                &id,
                &owner_id,
                self.inner.clock.now_ms(),
                self.inner.ttl_ms,
            )?;
            transaction
                .commit()
                .map_err(|error| schema::storage_error("commit SQLite session create", error))?;
            Ok(lease)
        })?;
        let storage = SqliteSessionStorage::new(self.inner.clone(), metadata, lease_token, runtime);
        self.insert_active(&id, &storage.inner)?;
        Ok(storage)
    }

    fn open_session(&self, id: &str) -> SessionResult<SqliteSessionStorage> {
        let _lifecycle = self.lifecycle_guard()?;
        let runtime = current_runtime()?;
        let mut active = self
            .inner
            .active
            .lock()
            .map_err(|_| SessionError::storage("SQLite active-handle registry poisoned"))?;
        if let Some(storage) = active.get(id).and_then(Weak::upgrade) {
            if !storage.closed.load(Ordering::SeqCst) {
                return Ok(SqliteSessionStorage { inner: storage });
            }
        }
        active.remove(id);
        let owner_id = Uuid::new_v4().to_string();
        let (metadata, lease_token) = self.inner.with_connection(|connection| {
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(|error| schema::storage_error("begin SQLite session open", error))?;
            let metadata = load_metadata(&transaction, id, &self.inner.path)?
                .ok_or_else(|| SessionError::not_found(format!("Session not found: {id}")))?;
            // Decode canonical state before claiming a writer handle, so a
            // corrupt row never leaves an orphaned active lease.
            let _ = load_state(&transaction, id)?;
            let lease_token = lease::acquire(
                &transaction,
                id,
                &owner_id,
                self.inner.clock.now_ms(),
                self.inner.ttl_ms,
            )?;
            transaction
                .commit()
                .map_err(|error| schema::storage_error("commit SQLite session open", error))?;
            Ok((metadata, lease_token))
        })?;
        let storage = SqliteSessionStorage::new(self.inner.clone(), metadata, lease_token, runtime);
        active.insert(id.to_string(), Arc::downgrade(&storage.inner));
        Ok(storage)
    }

    fn delete_id(&self, id: &str) -> SessionResult<()> {
        let _lifecycle = self.lifecycle_guard()?;
        let local = {
            let mut active = self
                .inner
                .active
                .lock()
                .map_err(|_| SessionError::storage("SQLite active-handle registry poisoned"))?;
            active.remove(id).and_then(|handle| handle.upgrade())
        };
        if let Some(local) = local {
            local.close()?;
        }
        self.inner.with_connection(|connection| {
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(|error| schema::storage_error("begin SQLite session delete", error))?;
            let now_ms = self.inner.clock.now_ms();
            let expires_at: Option<i64> = transaction
                .query_row(
                    "SELECT expires_at_ms FROM session_writer_leases WHERE session_id = ?1",
                    [id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|error| schema::storage_error("check SQLite delete lease", error))?;
            if expires_at.map(|expires| expires > now_ms).unwrap_or(false) {
                return Err(SessionError::storage(format!(
                    "SQLite session {id} already has an active writer"
                )));
            }
            search::delete_session(&transaction, id)?;
            transaction
                .execute("DELETE FROM sessions WHERE id = ?1", [id])
                .map_err(|error| schema::storage_error("delete SQLite session", error))?;
            transaction
                .execute(
                    "DELETE FROM session_writer_leases WHERE session_id = ?1",
                    [id],
                )
                .map_err(|error| schema::storage_error("delete SQLite writer lease", error))?;
            transaction
                .commit()
                .map_err(|error| schema::storage_error("commit SQLite session delete", error))
        })
    }

    fn validate_metadata_path(&self, metadata: &SqliteSessionMetadata) -> SessionResult<()> {
        if resolve_path(&metadata.path)? != self.inner.path {
            return Err(SessionError::storage(format!(
                "SQLite session metadata path does not match this repository: {}",
                metadata.path.display()
            )));
        }
        Ok(())
    }

    fn insert_active(&self, id: &str, storage: &Arc<StorageInner>) -> SessionResult<()> {
        self.inner
            .active
            .lock()
            .map_err(|_| SessionError::storage("SQLite active-handle registry poisoned"))?
            .insert(id.to_string(), Arc::downgrade(storage));
        Ok(())
    }

    fn lifecycle_guard(&self) -> SessionResult<MutexGuard<'_, ()>> {
        let guard = self
            .inner
            .lifecycle
            .lock()
            .map_err(|_| SessionError::storage("SQLite repository lifecycle mutex poisoned"))?;
        // An explicit close is reversible; the next repository-level use
        // reopens the connection. Previously closed storage handles remain shut.
        self.inner.closing.store(false, Ordering::SeqCst);
        Ok(guard)
    }
}

#[async_trait]
impl SessionRepo for SqliteSessionRepo {
    type Storage = SqliteSessionStorage;

    async fn create(&self, options: &SessionCreateOptions) -> SessionResult<Self::Storage> {
        let _lifecycle = self.lifecycle_guard()?;
        self.create_storage(
            &SqliteSessionCreateOptions {
                id: options.id.clone(),
                parent_session_id: options.parent_session_id.clone(),
                cwd: String::new(),
                metadata: options.metadata.clone(),
            },
            SessionState::new(),
        )
    }

    async fn open(&self, metadata: &SessionMetadata) -> SessionResult<Self::Storage> {
        self.open_session(&metadata.id)
    }

    async fn list(&self) -> SessionResult<Vec<SessionMetadata>> {
        Ok(self
            .list_with_options(&SqliteSessionListOptions::default())
            .await?
            .iter()
            .map(SqliteSessionMetadata::base_metadata)
            .collect())
    }

    async fn delete(&self, metadata: &SessionMetadata) -> SessionResult<()> {
        self.delete_id(&metadata.id)
    }

    async fn fork(
        &self,
        source: &SessionMetadata,
        options: &SessionCreateOptions,
        fork: &ForkOptions,
    ) -> SessionResult<Self::Storage> {
        let source_storage = self.open_session(&source.id)?;
        let source_metadata = source_storage.sqlite_metadata();
        self.fork_with_metadata(
            &source_metadata,
            &SqliteSessionCreateOptions {
                id: options.id.clone(),
                parent_session_id: options.parent_session_id.clone(),
                cwd: source_metadata.cwd.clone(),
                metadata: options.metadata.clone(),
            },
            fork,
        )
        .await
    }
}

impl RepoInner {
    fn with_connection<R>(
        &self,
        operation: impl FnOnce(&mut Connection) -> SessionResult<R>,
    ) -> SessionResult<R> {
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| SessionError::storage("SQLite connection mutex poisoned"))?;
        if connection.is_none() {
            if self.closing.load(Ordering::SeqCst) {
                return Err(SessionError::storage(
                    "SQLite repository is closing or closed",
                ));
            }
            *connection = Some(open_connection(&self.path)?);
        }
        operation(connection.as_mut().expect("connection initialized"))
    }
}

/// A reusable standalone FTS search object. Blank search returns before path
/// resolution, directory creation or SQLite open.
pub struct SqliteSessionSearch {
    path: PathBuf,
}

impl SqliteSessionSearch {
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
        }
    }

    pub async fn search(
        &self,
        options: SqliteSessionSearchOptions,
    ) -> SessionResult<Vec<SqliteSessionSearchHit>> {
        if options.text.trim().is_empty() {
            return Ok(Vec::new());
        }
        let repo = SqliteSessionRepo::open(&self.path)?;
        let result = repo.search_sessions(options).await;
        let close_result = repo.close().await;
        match (result, close_result) {
            (Ok(hits), Ok(())) => Ok(hits),
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(error),
        }
    }
}

fn current_runtime() -> SessionResult<Handle> {
    Handle::try_current().map_err(|_| {
        SessionError::storage(
            "opening a writable SQLite session requires a Tokio runtime for lease heartbeats",
        )
    })
}

fn open_connection(path: &Path) -> SessionResult<Connection> {
    let mut connection = Connection::open(path)
        .map_err(|error| schema::storage_error("open SQLite database", error))?;
    schema::initialize(&mut connection)?;
    Ok(connection)
}

fn resolve_path(path: &Path) -> SessionResult<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        std::env::current_dir()
            .map(|current| current.join(path))
            .map_err(|error| {
                SessionError::storage(format!("resolve SQLite database path: {error}"))
            })
    }
}

fn load_state(connection: &Connection, id: &str) -> SessionResult<SessionState> {
    let state_json: String = connection
        .query_row(
            "SELECT state_json FROM sessions WHERE id = ?1",
            [id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| schema::storage_error("read SQLite session snapshot", error))?
        .ok_or_else(|| SessionError::not_found(format!("Session not found: {id}")))?;
    serde_json::from_str(&state_json)
        .map_err(|error| SessionError::storage(format!("decode SQLite session {id}: {error}")))
}

fn load_metadata(
    connection: &Connection,
    id: &str,
    path: &Path,
) -> SessionResult<Option<SqliteSessionMetadata>> {
    let row = connection
        .query_row(
            "SELECT id, created_at, parent_session_id, cwd, metadata_json FROM sessions WHERE id = ?1",
            [id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            },
        )
        .optional()
        .map_err(|error| schema::storage_error("read SQLite session metadata", error))?;
    row.map(|(id, created_at, parent, cwd, metadata)| {
        decode_metadata(id, created_at, parent, cwd, metadata, path)
    })
    .transpose()
}

fn decode_metadata(
    id: String,
    created_at: i64,
    parent_session_id: Option<String>,
    cwd: String,
    metadata_json: Option<String>,
    path: &Path,
) -> SessionResult<SqliteSessionMetadata> {
    let metadata = metadata_json
        .map(|json| {
            let value: serde_json::Value = serde_json::from_str(&json).map_err(|error| {
                SessionError::storage(format!("decode SQLite session metadata: {error}"))
            })?;
            value.as_object().cloned().ok_or_else(|| {
                SessionError::storage("SQLite session metadata must be a JSON object")
            })
        })
        .transpose()?;
    Ok(SqliteSessionMetadata {
        id,
        created_at,
        parent_session_id,
        cwd,
        path: path.to_path_buf(),
        metadata,
    })
}

fn read_session_list_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<(String, i64, Option<String>, String, Option<String>)> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
    ))
}

fn has_column(connection: &Connection, column_name: &str) -> SessionResult<bool> {
    let mut statement = connection
        .prepare("PRAGMA table_info(sessions)")
        .map_err(|error| schema::storage_error("inspect SQLite session columns", error))?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| schema::storage_error("inspect SQLite session columns", error))?;
    for row in rows {
        if row.map_err(|error| schema::storage_error("read SQLite session column", error))?
            == column_name
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn map_insert_error(id: &str, error: rusqlite::Error) -> SessionError {
    match error {
        rusqlite::Error::SqliteFailure(ref code, _)
            if code.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            SessionError::already_exists(format!("Session already exists: {id}"))
        }
        other => schema::storage_error("insert SQLite session", other),
    }
}
