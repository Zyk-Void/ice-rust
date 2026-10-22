use super::*;
use crate::session::memory::{CounterIdGenerator, FakeClock};
use crate::session::types::{EntryOrder, EntryQuery, ForkOptions, ProvisionedKind};
use ice_agent::message::AgentMessage;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;

#[derive(Default)]
struct ManualClock(AtomicI64);

impl ManualClock {
    fn set(&self, now: i64) {
        self.0.store(now, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

fn entry(id: &str, text: &str, time: i64) -> ProvisionedEntry {
    ProvisionedEntry {
        id: id.into(),
        kind: ProvisionedKind::Message {
            message: AgentMessage::from(ice_ai::types::UserMessage::new(text, time)),
            terminate: None,
        },
    }
}

fn create_options(cwd: &str) -> SqliteSessionCreateOptions {
    SqliteSessionCreateOptions {
        cwd: cwd.to_string(),
        ..Default::default()
    }
}

fn repo_with_lease(
    path: &Path,
    clock: Arc<dyn Clock>,
    lease: SqliteWriterLeaseOptions,
) -> SqliteSessionRepo {
    SqliteSessionRepo::with_writer_lease(path, clock, Arc::new(CounterIdGenerator::new()), lease)
        .unwrap()
}

fn assert_open_rejection_preserves_database(path: &Path) {
    fn snapshot(path: &Path) -> (Vec<(String, String, Option<String>)>, String, i64) {
        let connection = Connection::open(path).unwrap();
        let mut statement = connection
            .prepare("SELECT type, name, sql FROM sqlite_master ORDER BY type, name")
            .unwrap();
        let schema = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        drop(statement);
        let journal: String = connection
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();
        let schema_version: i64 = connection
            .query_row("PRAGMA schema_version", [], |row| row.get(0))
            .unwrap();
        (schema, journal, schema_version)
    }

    let bytes_before = std::fs::read(path).unwrap();
    let state_before = snapshot(path);
    assert!(SqliteSessionRepo::open(path).is_err());
    assert_eq!(std::fs::read(path).unwrap(), bytes_before);
    assert_eq!(snapshot(path), state_before);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn same_repo_open_shares_handle_and_serializes_concurrent_writes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sessions.db");
    let repo = SqliteSessionRepo::open(&path).unwrap();
    let storage = repo
        .create_with_metadata(&create_options("/work"))
        .await
        .unwrap();
    let opened = repo
        .open_with_metadata(&storage.sqlite_metadata())
        .await
        .unwrap();
    assert!(Arc::ptr_eq(&storage.inner, &opened.inner));

    repo.inner
        .with_connection(|connection| {
            let journal: String = connection
                .query_row("PRAGMA journal_mode", [], |row| row.get(0))
                .map_err(|error| schema::storage_error("read journal mode", error))?;
            let synchronous: i64 = connection
                .query_row("PRAGMA synchronous", [], |row| row.get(0))
                .map_err(|error| schema::storage_error("read sync mode", error))?;
            let busy: i64 = connection
                .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
                .map_err(|error| schema::storage_error("read busy timeout", error))?;
            assert_eq!(journal.to_ascii_lowercase(), "wal");
            assert_eq!(synchronous, 2);
            assert_eq!(busy, 5_000);
            Ok(())
        })
        .unwrap();

    let other = SqliteSessionRepo::open(&path).unwrap();
    let conflict = match other.open_with_metadata(&storage.sqlite_metadata()).await {
        Ok(_) => panic!("a second repository acquired an active lease"),
        Err(error) => error,
    };
    assert!(conflict.message.contains("already has an active writer"));

    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let left_storage = storage.clone();
    let left_barrier = barrier.clone();
    let left = tokio::spawn(async move {
        left_barrier.wait().await;
        left_storage
            .append_entry(entry("left", "left", 1), "main")
            .await
    });
    let right_storage = opened.clone();
    let right = tokio::spawn(async move {
        barrier.wait().await;
        right_storage
            .append_entry(entry("right", "right", 2), "main")
            .await
    });
    left.await.unwrap().unwrap();
    right.await.unwrap().unwrap();
    assert_eq!(
        storage
            .find_entries(&EntryQuery::default())
            .await
            .unwrap()
            .len(),
        2
    );

    repo.close().await.unwrap();
    repo.close().await.unwrap();
    assert!(storage.get_lanes().await.is_err());
    let reopened = repo
        .open_with_metadata(&storage.sqlite_metadata())
        .await
        .unwrap();
    let fence: i64 = repo
        .inner
        .with_connection(|connection| {
            connection
                .query_row(
                    "SELECT fence FROM session_writer_leases WHERE session_id = ?1",
                    [storage.metadata().id],
                    |row| row.get(0),
                )
                .map_err(|error| schema::storage_error("read fence", error))
        })
        .unwrap();
    assert_eq!(fence, 2);
    assert_eq!(
        reopened
            .find_entries(&EntryQuery::default())
            .await
            .unwrap()
            .len(),
        2
    );
    repo.close().await.unwrap();
    other.close().await.unwrap();
}

#[tokio::test]
async fn dropping_last_storage_clone_releases_lease() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sessions.db");
    let first_repo = SqliteSessionRepo::open(&path).unwrap();
    let storage = first_repo
        .create_with_metadata(&create_options("/work"))
        .await
        .unwrap();
    let metadata = storage.sqlite_metadata();
    drop(storage);
    let second_repo = SqliteSessionRepo::open(&path).unwrap();
    let opened = second_repo.open_with_metadata(&metadata).await.unwrap();
    opened
        .append_entry(entry("after-drop", "RAII release", 1), "main")
        .await
        .unwrap();
    second_repo.close().await.unwrap();
    first_repo.close().await.unwrap();
}

#[tokio::test]
async fn expired_takeover_fences_stale_write_and_stale_release() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sessions.db");
    let clock = Arc::new(ManualClock::default());
    let lease = SqliteWriterLeaseOptions {
        ttl_ms: 1_000,
        heartbeat_interval_ms: 900,
    };
    let first_repo = repo_with_lease(&path, clock.clone(), lease.clone());
    let first = first_repo
        .create_with_metadata(&create_options("/work"))
        .await
        .unwrap();
    clock.set(2_000);
    let second_repo = repo_with_lease(&path, clock.clone(), lease.clone());
    let second = second_repo
        .open_with_metadata(&first.sqlite_metadata())
        .await
        .unwrap();
    assert!(first
        .append_entry(entry("stale", "rejected", 1), "main")
        .await
        .unwrap_err()
        .message
        .contains("writer lease was lost"));
    first.close().await.unwrap();

    let third_repo = repo_with_lease(&path, clock.clone(), lease);
    assert!(third_repo
        .open_with_metadata(&second.sqlite_metadata())
        .await
        .is_err());
    second
        .append_entry(entry("live", "accepted", 2), "main")
        .await
        .unwrap();
    second_repo.close().await.unwrap();
    third_repo.close().await.unwrap();
    first_repo.close().await.unwrap();
}

#[tokio::test]
async fn delete_honors_foreign_writer_then_succeeds_after_close() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sessions.db");
    let first_repo = SqliteSessionRepo::open(&path).unwrap();
    let storage = first_repo
        .create_with_metadata(&create_options("/work"))
        .await
        .unwrap();
    let second_repo = SqliteSessionRepo::open(&path).unwrap();
    let error = second_repo
        .delete_with_metadata(&storage.sqlite_metadata())
        .await
        .unwrap_err();
    assert!(error.message.contains("active writer"));
    storage
        .append_entry(entry("kept", "still here", 1), "main")
        .await
        .unwrap();
    first_repo.close().await.unwrap();
    second_repo
        .delete_with_metadata(&storage.sqlite_metadata())
        .await
        .unwrap();
    assert!(second_repo
        .list_with_options(&SqliteSessionListOptions::default())
        .await
        .unwrap()
        .is_empty());
    second_repo.close().await.unwrap();
}

#[tokio::test]
async fn failed_snapshot_commit_rolls_back_state_sequence_stats_and_recovers() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sessions.db");
    let clock = Arc::new(ManualClock::default());
    let repo = repo_with_lease(
        &path,
        clock.clone(),
        SqliteWriterLeaseOptions {
            ttl_ms: 50_000,
            heartbeat_interval_ms: 40_000,
        },
    );
    let storage = repo
        .create_with_metadata(&create_options("/work"))
        .await
        .unwrap();
    storage
        .append_entry(entry("before", "before", 1), "main")
        .await
        .unwrap();
    let lease_expiry = || {
        repo.inner
            .with_connection(|connection| {
                connection
                    .query_row(
                        "SELECT expires_at_ms FROM session_writer_leases WHERE session_id = ?1",
                        [storage.metadata().id],
                        |row| row.get::<_, i64>(0),
                    )
                    .map_err(|error| schema::storage_error("read test lease expiry", error))
            })
            .unwrap()
    };
    let expiry_before_rejection = lease_expiry();
    clock.set(1_000);
    repo.inner.with_connection(|connection| {
        connection.execute_batch("CREATE TRIGGER reject_snapshot BEFORE UPDATE OF state_json ON sessions BEGIN SELECT RAISE(ABORT, 'rejected'); END;")
            .map_err(|error| schema::storage_error("install trigger", error))
    }).unwrap();
    assert!(storage
        .append_entry(entry("rejected", "rejected", 2), "main")
        .await
        .is_err());
    assert_eq!(
        storage
            .find_entries(&EntryQuery::default())
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(storage.get_stats().await.unwrap().message_count, 1);
    assert_eq!(lease_expiry(), expiry_before_rejection);
    repo.inner
        .with_connection(|connection| {
            connection
                .execute_batch("DROP TRIGGER reject_snapshot;")
                .map_err(|error| schema::storage_error("drop trigger", error))
        })
        .unwrap();
    storage
        .append_entry(entry("after", "after", 3), "main")
        .await
        .unwrap();
    let entries = storage
        .find_entries(&EntryQuery {
            order: Some(EntryOrder::OldestFirst),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(
        entries.iter().map(Entry::id).collect::<Vec<_>>(),
        vec!["before", "after"]
    );
    assert_eq!(entries.last().unwrap().seq(), 2);
    repo.close().await.unwrap();
}

#[tokio::test]
async fn sqlite_metadata_cwd_fork_overrides_and_legacy_trait_api_work() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sessions.db");
    let repo = SqliteSessionRepo::open(&path).unwrap();
    let app_metadata =
        serde_json::Map::from_iter([("app".into(), serde_json::json!({"version": 1}))]);
    let source = repo
        .create_with_metadata(&SqliteSessionCreateOptions {
            cwd: String::new(),
            metadata: Some(app_metadata.clone()),
            ..Default::default()
        })
        .await
        .unwrap();
    source
        .append_entry(entry("root", "fork me", 1), "main")
        .await
        .unwrap();
    let inherited = repo
        .fork_with_metadata(
            &source.sqlite_metadata(),
            &SqliteSessionCreateOptions {
                parent_session_id: Some("override-parent".into()),
                ..Default::default()
            },
            &ForkOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(inherited.sqlite_metadata().cwd, "");
    assert_eq!(
        inherited.sqlite_metadata().metadata,
        Some(app_metadata.clone())
    );
    assert_eq!(
        inherited.metadata().parent_session_id.as_deref(),
        Some("override-parent")
    );
    let replacement = serde_json::Map::from_iter([("replace".into(), serde_json::json!(true))]);
    let child = repo
        .fork_with_metadata(
            &source.sqlite_metadata(),
            &SqliteSessionCreateOptions {
                metadata: Some(replacement.clone()),
                ..Default::default()
            },
            &ForkOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(child.sqlite_metadata().metadata, Some(replacement));

    let empty_cwd = repo
        .list_with_options(&SqliteSessionListOptions {
            cwd: Some(String::new()),
        })
        .await
        .unwrap();
    assert_eq!(empty_cwd.len(), 3);
    assert!(repo
        .list_with_options(&SqliteSessionListOptions {
            cwd: Some("/work".into())
        })
        .await
        .unwrap()
        .is_empty());
    let absent = dir.path().join("absent.db");
    assert!(
        SqliteSessionRepo::list_existing(&absent, &SqliteSessionListOptions::default())
            .unwrap()
            .is_empty()
    );
    assert!(!absent.exists());

    let legacy = SessionRepo::create(
        &repo,
        &SessionCreateOptions {
            id: Some("trait-api".into()),
            parent_session_id: None,
            metadata: Some(serde_json::Map::from_iter([(
                "client".into(),
                serde_json::json!("test"),
            )])),
        },
    )
    .await
    .unwrap();
    SessionStorage::append_entry(&legacy, entry("trait-entry", "legacy API", 1), "main")
        .await
        .unwrap();
    let reopened = SessionRepo::open(&repo, &legacy.metadata()).await.unwrap();
    assert_eq!(
        SessionStorage::find_entries(&reopened, &EntryQuery::default())
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(SessionRepo::list(&repo).await.unwrap().len(), 4);
    SessionRepo::delete(&repo, &legacy.metadata())
        .await
        .unwrap();
    repo.close().await.unwrap();
}

#[tokio::test]
async fn original_snapshot_schema_migrates_and_foreign_layout_is_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let legacy_path = dir.path().join("legacy.db");
    let connection = Connection::open(&legacy_path).unwrap();
    let state_json = serde_json::to_string(&SessionState::new()).unwrap();
    connection.execute_batch("CREATE TABLE sessions(id TEXT PRIMARY KEY, created_at INTEGER NOT NULL, parent_session_id TEXT, state_json TEXT NOT NULL);").unwrap();
    connection
        .execute(
            "INSERT INTO sessions(id, created_at, state_json) VALUES ('legacy', 1, ?1)",
            [&state_json],
        )
        .unwrap();
    drop(connection);
    let repo = SqliteSessionRepo::open(&legacy_path).unwrap();
    let rows = repo
        .list_with_options(&SqliteSessionListOptions::default())
        .await
        .unwrap();
    assert_eq!(rows[0].id, "legacy");
    assert_eq!(rows[0].cwd, "");
    repo.close().await.unwrap();

    let discovery_path = dir.path().join("legacy-discovery.db");
    let connection = Connection::open(&discovery_path).unwrap();
    connection.execute_batch("CREATE TABLE sessions(id TEXT PRIMARY KEY, created_at INTEGER NOT NULL, parent_session_id TEXT, state_json TEXT NOT NULL);").unwrap();
    connection
        .execute(
            "INSERT INTO sessions(id, created_at, state_json) VALUES ('discovery', 2, ?1)",
            [&state_json],
        )
        .unwrap();
    drop(connection);
    let bytes_before = std::fs::read(&discovery_path).unwrap();
    let list_none =
        SqliteSessionRepo::list_existing(&discovery_path, &SqliteSessionListOptions { cwd: None })
            .unwrap();
    let list_empty = SqliteSessionRepo::list_existing(
        &discovery_path,
        &SqliteSessionListOptions {
            cwd: Some(String::new()),
        },
    )
    .unwrap();
    let list_other = SqliteSessionRepo::list_existing(
        &discovery_path,
        &SqliteSessionListOptions {
            cwd: Some("/work".into()),
        },
    )
    .unwrap();
    assert_eq!(list_none.len(), 1);
    assert_eq!(list_empty.len(), 1);
    assert!(list_other.is_empty());
    assert_eq!(list_none[0].cwd, "");
    assert_eq!(std::fs::read(&discovery_path).unwrap(), bytes_before);

    let foreign_path = dir.path().join("foreign.db");
    let connection = Connection::open(&foreign_path).unwrap();
    connection.execute_batch("CREATE TABLE sessions(id TEXT PRIMARY KEY, created_at TEXT, cwd TEXT, parent_session_id TEXT, metadata TEXT);").unwrap();
    drop(connection);
    assert!(SqliteSessionRepo::open(&foreign_path).is_err());
    let connection = Connection::open(&foreign_path).unwrap();
    let journal: String = connection
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .unwrap();
    let lease_table: i64 = connection.query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name='session_writer_leases'", [], |row| row.get(0)).unwrap();
    assert_eq!(journal, "delete");
    assert_eq!(lease_table, 0);
}

#[test]
fn sessions_schema_preflight_rejects_noncanonical_keys_and_nullability_without_changes() {
    let dir = tempfile::tempdir().unwrap();
    let cases = [
        (
            "missing-primary-key",
            "CREATE TABLE sessions(id TEXT, created_at INTEGER NOT NULL, parent_session_id TEXT, state_json TEXT NOT NULL);",
        ),
        (
            "composite-primary-key",
            "CREATE TABLE sessions(id TEXT, created_at INTEGER NOT NULL, parent_session_id TEXT, state_json TEXT NOT NULL, PRIMARY KEY(id, created_at));",
        ),
        (
            "wrong-primary-key",
            "CREATE TABLE sessions(id TEXT, created_at INTEGER NOT NULL, parent_session_id TEXT, state_json TEXT PRIMARY KEY);",
        ),
        (
            "nullable-created-at",
            "CREATE TABLE sessions(id TEXT PRIMARY KEY, created_at INTEGER, parent_session_id TEXT, state_json TEXT NOT NULL);",
        ),
        (
            "nullable-state-json",
            "CREATE TABLE sessions(id TEXT PRIMARY KEY, created_at INTEGER NOT NULL, parent_session_id TEXT, state_json TEXT);",
        ),
        (
            "nullable-cwd",
            "CREATE TABLE sessions(id TEXT PRIMARY KEY, created_at INTEGER NOT NULL, parent_session_id TEXT, state_json TEXT NOT NULL, cwd TEXT, metadata_json TEXT);",
        ),
    ];

    for (name, ddl) in cases {
        let path = dir.path().join(format!("{name}.db"));
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch(ddl).unwrap();
        drop(connection);
        assert_open_rejection_preserves_database(&path);
    }
}

#[test]
fn writer_lease_views_are_rejected_before_migration_without_changes() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["session_writer_leases", "SESSION_WRITER_LEASES"] {
        let path = dir.path().join(format!("{name}.db"));
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch(
            "CREATE TABLE sessions(id TEXT PRIMARY KEY, created_at INTEGER NOT NULL, parent_session_id TEXT, state_json TEXT NOT NULL);"
        ).unwrap();
        connection.execute_batch(&format!(
            "CREATE VIEW {name} AS SELECT id AS session_id, id AS owner_id, 1 AS fence, 0 AS expires_at_ms FROM sessions;"
        )).unwrap();
        drop(connection);
        assert_open_rejection_preserves_database(&path);
    }
}

#[test]
fn search_schema_preflight_rejects_incompatible_occupied_objects_without_changes() {
    const LEGACY_SESSIONS: &str = "CREATE TABLE sessions(id TEXT PRIMARY KEY, created_at INTEGER NOT NULL, parent_session_id TEXT, state_json TEXT NOT NULL);";
    const BAD_PROJECTION_UNIQUE: &str = "CREATE TABLE session_search_entries(rowid INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT NOT NULL, cwd TEXT NOT NULL, entry_id TEXT NOT NULL, payload TEXT NOT NULL, timestamp_ms INTEGER NOT NULL, timestamp TEXT NOT NULL, UNIQUE(payload));";
    let cases = [
        (
            "wrong-projection-unique-key",
            BAD_PROJECTION_UNIQUE,
        ),
        (
            "projection-view",
            "CREATE VIEW session_search_entries AS SELECT id AS rowid, id AS session_id, id AS cwd, id AS entry_id, id AS payload, created_at AS timestamp_ms, id AS timestamp FROM sessions;",
        ),
        (
            "wrong-fts-table",
            "CREATE TABLE session_search_fts(payload TEXT);",
        ),
        (
            "contentless-fts",
            "CREATE VIRTUAL TABLE session_search_fts USING fts5(payload, tokenize = 'trigram', content = '');",
        ),
        (
            "external-content-fts",
            "CREATE VIRTUAL TABLE session_search_fts USING fts5(payload, tokenize = 'trigram', content = 'sessions');",
        ),
        (
            "fts-extra-column",
            "CREATE VIRTUAL TABLE session_search_fts USING fts5(payload, extra, tokenize = 'trigram');",
        ),
    ];
    let dir = tempfile::tempdir().unwrap();

    for (name, search_ddl) in cases {
        let path = dir.path().join(format!("{name}.db"));
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch(LEGACY_SESSIONS).unwrap();
        connection.execute_batch(search_ddl).unwrap();
        drop(connection);
        assert_open_rejection_preserves_database(&path);
    }
}

#[tokio::test]
async fn valid_incomplete_search_schemas_open_for_explicit_rebuild() {
    const LEGACY_SESSIONS: &str = "CREATE TABLE sessions(id TEXT PRIMARY KEY, created_at INTEGER NOT NULL, parent_session_id TEXT, state_json TEXT NOT NULL);";
    const PROJECTION: &str = "CREATE TABLE session_search_entries(rowid INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT NOT NULL, cwd TEXT NOT NULL, entry_id TEXT NOT NULL, payload TEXT NOT NULL, timestamp_ms INTEGER NOT NULL, timestamp TEXT NOT NULL, UNIQUE(session_id, entry_id));";
    const FTS: &str =
        "CREATE VIRTUAL TABLE session_search_fts USING fts5(payload, tokenize = 'trigram');";
    let dir = tempfile::tempdir().unwrap();

    for (name, derived_ddl) in [("projection-only", PROJECTION), ("fts-only", FTS)] {
        let path = dir.path().join(format!("{name}.db"));
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch(LEGACY_SESSIONS).unwrap();
        connection.execute_batch(derived_ddl).unwrap();
        drop(connection);

        let repo = SqliteSessionRepo::open(&path).unwrap();
        repo.rebuild_search_index().await.unwrap();
        repo.close().await.unwrap();
    }
}

#[tokio::test]
async fn fts_backfills_appends_forks_deletes_and_filters_exact_cwd() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sessions.db");
    let repo = SqliteSessionRepo::open(&path).unwrap();
    let source = repo
        .create_with_metadata(&create_options("/alpha"))
        .await
        .unwrap();
    source
        .append_entry(entry("historical", "preexisting needle", 10), "main")
        .await
        .unwrap();
    let empty = repo
        .create_with_metadata(&create_options(""))
        .await
        .unwrap();
    empty
        .append_entry(entry("empty-cwd", "needle empty cwd", 11), "main")
        .await
        .unwrap();
    let search = SqliteSessionSearch::new(&path);
    let hits = search
        .search(SqliteSessionSearchOptions {
            text: "needle".into(),
            cwd: Some("/alpha".into()),
        })
        .await
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].entry_id, "historical");
    assert!(hits[0].timestamp.ends_with('Z'));
    assert!(hits[0].score.is_finite());
    assert_eq!(
        search
            .search(SqliteSessionSearchOptions {
                text: "needle".into(),
                cwd: Some(String::new())
            })
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(search
        .search(SqliteSessionSearchOptions {
            text: "needle\" OR *".into(),
            cwd: None,
        })
        .await
        .unwrap()
        .is_empty());
    let rowid: i64 = repo
        .inner
        .with_connection(|connection| {
            connection
                .query_row(
                    "SELECT rowid FROM session_search_entries WHERE entry_id='historical'",
                    [],
                    |row| row.get(0),
                )
                .map_err(|error| schema::storage_error("read rowid", error))
        })
        .unwrap();
    source
        .append_entry(entry("later", "another needle", 12), "main")
        .await
        .unwrap();
    let fork = repo
        .fork_with_metadata(
            &source.sqlite_metadata(),
            &SqliteSessionCreateOptions::default(),
            &ForkOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        search
            .search(SqliteSessionSearchOptions {
                text: "another needle".into(),
                cwd: Some("/alpha".into())
            })
            .await
            .unwrap()
            .len(),
        2
    );
    let stable_rowid: i64 = repo
        .inner
        .with_connection(|connection| {
            connection
                .query_row(
                    "SELECT rowid FROM session_search_entries WHERE entry_id='historical'",
                    [],
                    |row| row.get(0),
                )
                .map_err(|error| schema::storage_error("read rowid", error))
        })
        .unwrap();
    assert_eq!(rowid, stable_rowid);
    repo.delete_with_metadata(&fork.sqlite_metadata())
        .await
        .unwrap();
    assert_eq!(
        search
            .search(SqliteSessionSearchOptions {
                text: "another needle".into(),
                cwd: Some("/alpha".into())
            })
            .await
            .unwrap()
            .len(),
        1
    );
    repo.close().await.unwrap();
}

#[tokio::test]
async fn rebuild_removes_orphaned_search_rows_before_session_id_reuse() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sessions.db");
    let repo = SqliteSessionRepo::open(&path).unwrap();
    let reused_id = "reused-session";
    let original = repo
        .create_with_metadata(&SqliteSessionCreateOptions {
            id: Some(reused_id.into()),
            cwd: "/work".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    original
        .append_entry(entry("old-entry", "orphaned searchable phrase", 1), "main")
        .await
        .unwrap();
    assert_eq!(
        repo.search_sessions(SqliteSessionSearchOptions {
            text: "orphaned searchable".into(),
            cwd: None,
        })
        .await
        .unwrap()
        .len(),
        1
    );

    repo.inner
        .with_connection(|connection| {
            connection
                .execute("DELETE FROM sessions WHERE id = ?1", [reused_id])
                .map_err(|error| {
                    schema::storage_error("simulate external session deletion", error)
                })?;
            Ok(())
        })
        .unwrap();
    original.close().await.unwrap();
    repo.rebuild_search_index().await.unwrap();

    let derived_row_counts = repo
        .inner
        .with_connection(|connection| {
            let projection: i64 = connection
                .query_row("SELECT count(*) FROM session_search_entries", [], |row| {
                    row.get(0)
                })
                .map_err(|error| schema::storage_error("count rebuilt projection", error))?;
            let fts: i64 = connection
                .query_row("SELECT count(*) FROM session_search_fts", [], |row| {
                    row.get(0)
                })
                .map_err(|error| schema::storage_error("count rebuilt FTS rows", error))?;
            Ok((projection, fts))
        })
        .unwrap();
    assert_eq!(derived_row_counts, (0, 0));

    let replacement = repo
        .create_with_metadata(&SqliteSessionCreateOptions {
            id: Some("unrelated-source".into()),
            cwd: "/work".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    replacement
        .append_entry(
            entry("new-entry", "completely unrelated replacement", 2),
            "main",
        )
        .await
        .unwrap();
    let replacement_state_json: String = repo
        .inner
        .with_connection(|connection| {
            connection
                .query_row(
                    "SELECT state_json FROM sessions WHERE id = 'unrelated-source'",
                    [],
                    |row| row.get(0),
                )
                .map_err(|error| schema::storage_error("read replacement canonical state", error))
        })
        .unwrap();
    repo.inner
        .with_connection(|connection| {
            connection
                .execute(
                    "INSERT INTO sessions(id, created_at, parent_session_id, state_json, cwd) VALUES (?1, 3, NULL, ?2, '/work')",
                    rusqlite::params![reused_id, replacement_state_json],
                )
                .map_err(|error| schema::storage_error("simulate session id reuse", error))?;
            Ok(())
        })
        .unwrap();
    assert!(repo
        .search_sessions(SqliteSessionSearchOptions {
            text: "orphaned searchable".into(),
            cwd: None,
        })
        .await
        .unwrap()
        .is_empty());
    replacement.close().await.unwrap();
    repo.close().await.unwrap();
}

#[tokio::test]
async fn blank_search_is_noncreating_and_fts_failure_rolls_back_then_rebuilds() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("not-created.db");
    let search = SqliteSessionSearch::new(&path);
    assert!(search
        .search(SqliteSessionSearchOptions {
            text: " \t\n ".into(),
            cwd: None
        })
        .await
        .unwrap()
        .is_empty());
    assert!(!path.exists());

    let clock = Arc::new(ManualClock::default());
    let repo = repo_with_lease(
        &path,
        clock.clone(),
        SqliteWriterLeaseOptions {
            ttl_ms: 50_000,
            heartbeat_interval_ms: 40_000,
        },
    );
    let storage = repo
        .create_with_metadata(&create_options("/work"))
        .await
        .unwrap();
    storage
        .append_entry(entry("before", "index before", 1), "main")
        .await
        .unwrap();
    repo.search_sessions(SqliteSessionSearchOptions {
        text: "before".into(),
        cwd: None,
    })
    .await
    .unwrap();
    let lease_expiry = || {
        repo.inner
            .with_connection(|connection| {
                connection
                    .query_row(
                        "SELECT expires_at_ms FROM session_writer_leases WHERE session_id = ?1",
                        [storage.metadata().id],
                        |row| row.get::<_, i64>(0),
                    )
                    .map_err(|error| schema::storage_error("read test lease expiry", error))
            })
            .unwrap()
    };
    let expiry_before_failure = lease_expiry();
    clock.set(1_000);
    repo.inner
        .with_connection(|connection| {
            connection
                .execute_batch("DROP TABLE session_search_fts;")
                .map_err(|error| schema::storage_error("drop FTS", error))
        })
        .unwrap();
    assert!(storage
        .append_entry(entry("rejected", "cannot index", 2), "main")
        .await
        .is_err());
    assert_eq!(
        storage
            .find_entries(&EntryQuery::default())
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(lease_expiry(), expiry_before_failure);
    let projection_rows: i64 = repo
        .inner
        .with_connection(|connection| {
            connection
                .query_row(
                    "SELECT count(*) FROM session_search_entries WHERE session_id = ?1",
                    [storage.metadata().id],
                    |row| row.get(0),
                )
                .map_err(|error| schema::storage_error("count FTS projection rows", error))
        })
        .unwrap();
    assert_eq!(projection_rows, 1);
    repo.rebuild_search_index().await.unwrap();
    assert_eq!(
        repo.search_sessions(SqliteSessionSearchOptions {
            text: "before".into(),
            cwd: None
        })
        .await
        .unwrap()
        .len(),
        1
    );
    storage
        .append_entry(entry("after", "index repaired", 3), "main")
        .await
        .unwrap();
    repo.close().await.unwrap();
}

#[test]
fn lease_options_are_positive_and_heartbeat_is_shorter_than_ttl() {
    let dir = tempfile::tempdir().unwrap();
    for options in [
        SqliteWriterLeaseOptions {
            ttl_ms: 0,
            heartbeat_interval_ms: 0,
        },
        SqliteWriterLeaseOptions {
            ttl_ms: 10,
            heartbeat_interval_ms: 10,
        },
    ] {
        assert!(
            SqliteSessionRepo::open_with_lease_options(dir.path().join("invalid.db"), options)
                .is_err()
        );
    }
    assert!(!dir.path().join("invalid.db").exists());
}

#[tokio::test]
async fn idle_writer_heartbeat_renews_lease() {
    let dir = tempfile::tempdir().unwrap();
    let repo = SqliteSessionRepo::with_writer_lease(
        dir.path().join("sessions.db"),
        Arc::new(FakeClock::new()),
        Arc::new(CounterIdGenerator::new()),
        SqliteWriterLeaseOptions {
            ttl_ms: 300,
            heartbeat_interval_ms: 50,
        },
    )
    .unwrap();
    let storage = repo
        .create_with_metadata(&create_options("/work"))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(120)).await;
    storage
        .append_entry(entry("still-owned", "heartbeat", 1), "main")
        .await
        .unwrap();
    repo.close().await.unwrap();
}
