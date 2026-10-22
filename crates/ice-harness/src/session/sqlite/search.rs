use std::collections::HashMap;
use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension, Transaction};

use crate::error::{SessionError, SessionResult};
use crate::session::state::SessionState;
use crate::session::types::{EntryOrder, EntryQuery};

use super::schema::storage_error;
use super::{decode_metadata, SqliteSessionSearchHit, SqliteSessionSearchOptions};

const PROJECTION_TABLE: &str = "session_search_entries";
const FTS_TABLE: &str = "session_search_fts";

pub(super) fn ensure_index(transaction: &Transaction<'_>) -> SessionResult<()> {
    let projection_exists = table_exists(transaction, PROJECTION_TABLE)?;
    let fts_exists = table_exists(transaction, FTS_TABLE)?;
    match (projection_exists, fts_exists) {
        (false, false) => {
            transaction
                .execute_batch(
                    "CREATE TABLE session_search_entries (
                        rowid INTEGER PRIMARY KEY AUTOINCREMENT,
                        session_id TEXT NOT NULL,
                        cwd TEXT NOT NULL,
                        entry_id TEXT NOT NULL,
                        payload TEXT NOT NULL,
                        timestamp_ms INTEGER NOT NULL,
                        timestamp TEXT NOT NULL,
                        UNIQUE(session_id, entry_id)
                    );
                    CREATE INDEX session_search_entries_session
                        ON session_search_entries(session_id);
                    CREATE VIRTUAL TABLE session_search_fts USING fts5(payload, tokenize = 'trigram');",
                )
                .map_err(|error| storage_error("create SQLite FTS5 session index", error))?;
            backfill(transaction)?;
            Ok(())
        }
        (true, true) => validate_index_schema(transaction),
        _ => Err(SessionError::storage(
            "SQLite session search index is incomplete; use rebuild_search_index after repairing its schema",
        )),
    }
}

pub(super) fn rebuild(transaction: &Transaction<'_>) -> SessionResult<()> {
    let projection_exists = table_exists(transaction, PROJECTION_TABLE)?;
    let fts_exists = table_exists(transaction, FTS_TABLE)?;
    match (projection_exists, fts_exists) {
        (false, false) => ensure_index(transaction)?,
        (true, false) => {
            validate_projection_schema(transaction)?;
            transaction
                .execute_batch(
                    "CREATE VIRTUAL TABLE session_search_fts USING fts5(payload, tokenize = 'trigram');",
                )
                .map_err(|error| storage_error("repair SQLite FTS5 table", error))?;
        }
        (false, true) => {
            validate_fts_schema(transaction)?;
            transaction
                .execute_batch("DROP TABLE session_search_fts;")
                .map_err(|error| storage_error("replace incomplete SQLite FTS5 table", error))?;
            transaction
                .execute_batch(
                    "CREATE TABLE session_search_entries (
                        rowid INTEGER PRIMARY KEY AUTOINCREMENT,
                        session_id TEXT NOT NULL,
                        cwd TEXT NOT NULL,
                        entry_id TEXT NOT NULL,
                        payload TEXT NOT NULL,
                        timestamp_ms INTEGER NOT NULL,
                        timestamp TEXT NOT NULL,
                        UNIQUE(session_id, entry_id)
                    );
                    CREATE INDEX session_search_entries_session
                        ON session_search_entries(session_id);
                    CREATE VIRTUAL TABLE session_search_fts USING fts5(payload, tokenize = 'trigram');",
                )
                .map_err(|error| storage_error("repair SQLite FTS projection", error))?;
        }
        (true, true) => validate_index_schema(transaction)?,
    }

    // Remove rows for sessions deleted outside this adapter before rebuilding
    // FTS; otherwise an orphaned projection payload could remain searchable or
    // later be associated with a reused session id.
    transaction
        .execute(
            "DELETE FROM session_search_entries
             WHERE NOT EXISTS (SELECT 1 FROM sessions WHERE sessions.id = session_search_entries.session_id)",
            [],
        )
        .map_err(|error| storage_error("remove orphaned SQLite search entries", error))?;

    // Reconcile the derived projection against canonical snapshots, retaining
    // any existing projection rowid for unchanged entries.
    for (session_id, cwd, state_json) in read_snapshots(transaction)? {
        let state = decode_state(&session_id, &state_json)?;
        sync_session(transaction, &session_id, &cwd, &state)?;
    }
    transaction
        .execute("DELETE FROM session_search_fts", [])
        .map_err(|error| storage_error("clear SQLite FTS5 index for rebuild", error))?;
    let mut statement = transaction
        .prepare("SELECT rowid, payload FROM session_search_entries ORDER BY rowid")
        .map_err(|error| storage_error("read SQLite search projection for rebuild", error))?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| storage_error("read SQLite search projection for rebuild", error))?;
    let projection: Vec<_> = rows
        .map(|row| {
            row.map_err(|error| storage_error("read SQLite projection row for rebuild", error))
        })
        .collect::<SessionResult<_>>()?;
    drop(statement);
    for (rowid, payload) in projection {
        transaction
            .execute(
                "INSERT INTO session_search_fts(rowid, payload) VALUES (?1, ?2)",
                params![rowid, payload],
            )
            .map_err(|error| storage_error("rebuild SQLite FTS5 index", error))?;
    }
    Ok(())
}

/// Keep projection and full-text rows transactionally coupled to the canonical
/// snapshot. Existing rows retain their rowid when an append changes a state.
pub(super) fn sync_session(
    transaction: &Transaction<'_>,
    session_id: &str,
    cwd: &str,
    state: &SessionState,
) -> SessionResult<()> {
    if !table_exists(transaction, PROJECTION_TABLE)? {
        if table_exists(transaction, FTS_TABLE)? {
            return Err(SessionError::storage(
                "SQLite FTS table has no entry projection",
            ));
        }
        return Ok(());
    }
    if !table_exists(transaction, FTS_TABLE)? {
        return Err(SessionError::storage(
            "SQLite FTS entry projection has no FTS table",
        ));
    }
    validate_index_schema(transaction)?;

    let mut current = HashMap::<String, (i64, String, i64)>::new();
    {
        let mut statement = transaction
            .prepare("SELECT rowid, entry_id, payload, timestamp_ms FROM session_search_entries WHERE session_id = ?1")
            .map_err(|error| storage_error("read SQLite search projection", error))?;
        let rows = statement
            .query_map([session_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })
            .map_err(|error| storage_error("read SQLite search projection", error))?;
        for row in rows {
            let (rowid, entry_id, payload, timestamp_ms) =
                row.map_err(|error| storage_error("read SQLite search projection row", error))?;
            current.insert(entry_id, (rowid, payload, timestamp_ms));
        }
    }

    let entries = state.find_entries(&EntryQuery {
        order: Some(EntryOrder::OldestFirst),
        ..Default::default()
    })?;
    for entry in entries {
        let entry_id = entry.id().to_string();
        let timestamp_ms = entry.base().timestamp;
        let timestamp = super::super::jsonl::repo::js_iso_timestamp_from_ms(timestamp_ms)?;
        let payload = serde_json::to_string(&entry).map_err(|error| {
            SessionError::storage(format!("serialize indexed session entry: {error}"))
        })?;
        if let Some((rowid, old_payload, old_timestamp)) = current.remove(&entry_id) {
            transaction
                .execute(
                    "UPDATE session_search_entries SET cwd = ?1, payload = ?2, timestamp_ms = ?3, timestamp = ?4 WHERE rowid = ?5",
                    params![cwd, payload, timestamp_ms, timestamp, rowid],
                )
                .map_err(|error| storage_error("update SQLite search projection", error))?;
            if old_payload != payload || old_timestamp != timestamp_ms {
                transaction
                    .execute("DELETE FROM session_search_fts WHERE rowid = ?1", [rowid])
                    .map_err(|error| storage_error("remove stale SQLite FTS entry", error))?;
                transaction
                    .execute(
                        "INSERT INTO session_search_fts(rowid, payload) VALUES (?1, ?2)",
                        params![rowid, payload],
                    )
                    .map_err(|error| storage_error("update SQLite FTS entry", error))?;
            }
        } else {
            transaction
                .execute(
                    "INSERT INTO session_search_entries(session_id, entry_id, payload, timestamp_ms, timestamp, cwd) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![session_id, entry_id, payload, timestamp_ms, timestamp, cwd],
                )
                .map_err(|error| storage_error("insert SQLite search projection", error))?;
            let rowid = transaction.last_insert_rowid();
            transaction
                .execute(
                    "INSERT INTO session_search_fts(rowid, payload) VALUES (?1, ?2)",
                    params![rowid, payload],
                )
                .map_err(|error| storage_error("insert SQLite FTS entry", error))?;
        }
    }

    for (_, (rowid, _, _)) in current {
        transaction
            .execute("DELETE FROM session_search_fts WHERE rowid = ?1", [rowid])
            .map_err(|error| storage_error("remove deleted SQLite FTS entry", error))?;
        transaction
            .execute(
                "DELETE FROM session_search_entries WHERE rowid = ?1",
                [rowid],
            )
            .map_err(|error| storage_error("remove deleted SQLite search projection", error))?;
    }
    Ok(())
}

pub(super) fn delete_session(transaction: &Transaction<'_>, session_id: &str) -> SessionResult<()> {
    let projection_exists = table_exists(transaction, PROJECTION_TABLE)?;
    let fts_exists = table_exists(transaction, FTS_TABLE)?;
    if !projection_exists && !fts_exists {
        return Ok(());
    }
    if !projection_exists || !fts_exists {
        return Err(SessionError::storage(
            "SQLite session search index is incomplete",
        ));
    }
    validate_index_schema(transaction)?;
    let mut statement = transaction
        .prepare("SELECT rowid FROM session_search_entries WHERE session_id = ?1")
        .map_err(|error| storage_error("read SQLite entries for delete", error))?;
    let rowids = statement
        .query_map([session_id], |row| row.get::<_, i64>(0))
        .map_err(|error| storage_error("read SQLite entries for delete", error))?
        .map(|row| row.map_err(|error| storage_error("read SQLite entry rowid", error)))
        .collect::<SessionResult<Vec<_>>>()?;
    drop(statement);
    for rowid in rowids {
        transaction
            .execute("DELETE FROM session_search_fts WHERE rowid = ?1", [rowid])
            .map_err(|error| storage_error("delete SQLite FTS entry", error))?;
    }
    transaction
        .execute(
            "DELETE FROM session_search_entries WHERE session_id = ?1",
            [session_id],
        )
        .map_err(|error| storage_error("delete SQLite search projection", error))?;
    Ok(())
}

pub(super) fn search(
    transaction: &Transaction<'_>,
    path: &Path,
    options: &SqliteSessionSearchOptions,
) -> SessionResult<Vec<SqliteSessionSearchHit>> {
    let query = options.text.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    ensure_index(transaction)?;
    // Trigram FTS has no token for a query shorter than three Unicode scalar
    // values. Returning no hits avoids turning a normal short query into an FTS
    // syntax error while retaining substring matching for indexable phrases.
    if query.chars().count() < 3 {
        return Ok(Vec::new());
    }
    let quoted = format!("\"{}\"", query.replace('"', "\"\""));
    let mut statement = transaction
        .prepare(
            "SELECT s.id, s.created_at, s.parent_session_id, s.cwd, s.metadata_json,
                    p.entry_id, p.timestamp_ms, bm25(session_search_fts)
             FROM session_search_fts
             JOIN session_search_entries p ON p.rowid = session_search_fts.rowid
             JOIN sessions s ON s.id = p.session_id
             WHERE session_search_fts MATCH ?1 AND (?2 IS NULL OR s.cwd = ?2)
             ORDER BY bm25(session_search_fts) ASC",
        )
        .map_err(|error| storage_error("prepare SQLite session search", error))?;
    let cwd_filter = options.cwd.as_deref();
    let rows = statement
        .query_map(params![quoted, cwd_filter], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, f64>(7)?,
            ))
        })
        .map_err(|error| storage_error("execute SQLite session search", error))?;
    let mut hits = Vec::new();
    for row in rows {
        let (id, created_at, parent, session_cwd, metadata_json, hit_entry_id, timestamp_ms, score) =
            row.map_err(|error| storage_error("read SQLite session search hit", error))?;
        let metadata = decode_metadata(id, created_at, parent, session_cwd, metadata_json, path)?;
        let timestamp = super::super::jsonl::repo::js_iso_timestamp_from_ms(timestamp_ms)?;
        hits.push(SqliteSessionSearchHit {
            metadata,
            entry_id: hit_entry_id,
            timestamp,
            score,
        });
    }
    Ok(hits)
}

fn backfill(transaction: &Transaction<'_>) -> SessionResult<()> {
    for (session_id, cwd, state_json) in read_snapshots(transaction)? {
        let state = decode_state(&session_id, &state_json)?;
        sync_session(transaction, &session_id, &cwd, &state)?;
    }
    Ok(())
}

fn read_snapshots(transaction: &Transaction<'_>) -> SessionResult<Vec<(String, String, String)>> {
    let mut statement = transaction
        .prepare("SELECT id, cwd, state_json FROM sessions ORDER BY created_at DESC")
        .map_err(|error| storage_error("read SQLite snapshots for FTS", error))?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|error| storage_error("read SQLite snapshots for FTS", error))?;
    rows.map(|row| row.map_err(|error| storage_error("read SQLite snapshot for FTS", error)))
        .collect()
}

fn decode_state(session_id: &str, state_json: &str) -> SessionResult<SessionState> {
    serde_json::from_str(state_json).map_err(|error| {
        SessionError::storage(format!(
            "decode SQLite session {session_id} for FTS: {error}"
        ))
    })
}

pub(super) fn preflight(connection: &Connection) -> SessionResult<()> {
    for (name, description) in [
        (PROJECTION_TABLE, "SQLite session search projection"),
        (FTS_TABLE, "SQLite session FTS5 index"),
    ] {
        match search_object_type(connection, name)? {
            None => {}
            Some(object_type) if object_type == "table" => {
                if name == PROJECTION_TABLE {
                    validate_projection_schema(connection)?;
                } else {
                    validate_fts_schema(connection)?;
                }
            }
            Some(_) => {
                return Err(SessionError::storage(format!(
                    "Unsupported SQLite {description} object; expected a table"
                )))
            }
        }
    }
    Ok(())
}

fn validate_index_schema(transaction: &Transaction<'_>) -> SessionResult<()> {
    validate_projection_schema(transaction)?;
    validate_fts_schema(transaction)
}

fn validate_projection_schema(connection: &Connection) -> SessionResult<()> {
    let mut statement = connection
        .prepare("PRAGMA table_info(session_search_entries)")
        .map_err(|error| storage_error("inspect SQLite search projection schema", error))?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(5)?,
            ))
        })
        .map_err(|error| storage_error("inspect SQLite search projection schema", error))?;
    let columns = rows
        .map(|row| {
            row.map_err(|error| storage_error("read SQLite search projection schema", error))
        })
        .collect::<SessionResult<Vec<_>>>()?;
    let expected = [
        ("rowid", "INTEGER", 1),
        ("session_id", "TEXT", 0),
        ("cwd", "TEXT", 0),
        ("entry_id", "TEXT", 0),
        ("payload", "TEXT", 0),
        ("timestamp_ms", "INTEGER", 0),
        ("timestamp", "TEXT", 0),
    ];
    if columns.len() != expected.len()
        || expected.iter().any(|(name, declared_type, primary_key)| {
            !columns.iter().any(|(actual_name, actual_type, actual_pk)| {
                actual_name == name
                    && actual_type.eq_ignore_ascii_case(declared_type)
                    && actual_pk == primary_key
            })
        })
    {
        return Err(SessionError::storage(
            "Unsupported SQLite session search projection layout",
        ));
    }
    if !has_session_entry_unique_index(connection)? {
        return Err(SessionError::storage(
            "SQLite session search projection lacks its per-session entry uniqueness constraint",
        ));
    }
    Ok(())
}

fn has_session_entry_unique_index(connection: &Connection) -> SessionResult<bool> {
    let mut statement = connection
        .prepare("SELECT name, [unique], partial FROM pragma_index_list(?1)")
        .map_err(|error| storage_error("inspect SQLite search projection indexes", error))?;
    let rows = statement
        .query_map([PROJECTION_TABLE], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)? != 0,
                row.get::<_, i64>(2)? != 0,
            ))
        })
        .map_err(|error| storage_error("inspect SQLite search projection indexes", error))?;
    let indexes = rows
        .map(|row| row.map_err(|error| storage_error("read SQLite projection index", error)))
        .collect::<SessionResult<Vec<_>>>()?;
    drop(statement);

    for (name, unique, partial) in indexes {
        if !unique || partial {
            continue;
        }
        let mut statement = connection
            .prepare("SELECT name FROM pragma_index_info(?1) ORDER BY seqno")
            .map_err(|error| {
                storage_error("inspect SQLite search projection index columns", error)
            })?;
        let rows = statement
            .query_map([name], |row| row.get::<_, Option<String>>(0))
            .map_err(|error| {
                storage_error("inspect SQLite search projection index columns", error)
            })?;
        let columns = rows
            .map(|row| {
                row.map_err(|error| storage_error("read SQLite projection index column", error))
            })
            .collect::<SessionResult<Vec<_>>>()?;
        if columns == [Some("session_id".to_string()), Some("entry_id".to_string())] {
            return Ok(true);
        }
    }
    Ok(false)
}

fn validate_fts_schema(connection: &Connection) -> SessionResult<()> {
    let definition: Option<String> = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = ?1 COLLATE NOCASE",
            [FTS_TABLE],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| storage_error("inspect SQLite FTS5 schema", error))?;
    let definition =
        definition.ok_or_else(|| SessionError::storage("SQLite FTS5 table is missing"))?;
    let compact = definition
        .to_ascii_lowercase()
        .chars()
        .filter(|character| {
            !character.is_whitespace() && !matches!(character, '\"' | '`' | '[' | ']')
        })
        .collect::<String>();
    if compact != "createvirtualtablesession_search_ftsusingfts5(payload,tokenize='trigram')" {
        return Err(SessionError::storage(
            "Unsupported SQLite session FTS5 layout",
        ));
    }

    let mut statement = connection
        .prepare("PRAGMA table_info(session_search_fts)")
        .map_err(|error| storage_error("inspect SQLite FTS5 columns", error))?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| storage_error("inspect SQLite FTS5 columns", error))?;
    let columns = rows
        .map(|row| row.map_err(|error| storage_error("read SQLite FTS5 column", error)))
        .collect::<SessionResult<Vec<_>>>()?;
    if columns != ["payload"] {
        return Err(SessionError::storage(
            "Unsupported SQLite session FTS5 columns",
        ));
    }
    Ok(())
}

fn search_object_type(connection: &Connection, name: &str) -> SessionResult<Option<String>> {
    connection
        .query_row(
            "SELECT type FROM sqlite_master WHERE type IN ('table', 'view') AND name = ?1 COLLATE NOCASE",
            [name],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| storage_error("inspect SQLite search object", error))
}

fn table_exists(transaction: &Transaction<'_>, name: &str) -> SessionResult<bool> {
    transaction
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type IN ('table', 'view') AND name = ?1 COLLATE NOCASE",
            [name],
            |_| Ok(true),
        )
        .optional()
        .map(|value| value.unwrap_or(false))
        .map_err(|error| storage_error("inspect SQLite search schema", error))
}
