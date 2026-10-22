use std::collections::HashSet;
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, TransactionBehavior};

use crate::error::{SessionError, SessionResult};

const BASE_COLUMNS: &[&str] = &["id", "created_at", "parent_session_id", "state_json"];
const OWN_COLUMNS: &[&str] = &["cwd", "metadata_json"];

/// Set up only databases whose `sessions` table matches the original Rust
/// snapshot format. In particular, do not run DDL or switch journal mode on a
/// TypeScript/unknown sessions schema.
pub(super) fn initialize(connection: &mut Connection) -> SessionResult<()> {
    connection
        .busy_timeout(Duration::from_millis(5_000))
        .map_err(|error| storage_error("configure SQLite busy timeout", error))?;
    inspect_schema(connection)?;
    super::search::preflight(connection)?;

    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| storage_error("begin SQLite schema migration", error))?;
    let existing = inspect_schema(&transaction)?;
    super::search::preflight(&transaction)?;
    if !existing {
        transaction
            .execute_batch(
                "CREATE TABLE sessions (
                    id TEXT PRIMARY KEY,
                    created_at INTEGER NOT NULL,
                    parent_session_id TEXT,
                    state_json TEXT NOT NULL,
                    cwd TEXT NOT NULL DEFAULT '',
                    metadata_json TEXT
                );",
            )
            .map_err(|error| storage_error("create SQLite sessions table", error))?;
    } else {
        let columns = session_columns(&transaction)?;
        if !columns.contains("cwd") {
            transaction
                .execute(
                    "ALTER TABLE sessions ADD COLUMN cwd TEXT NOT NULL DEFAULT ''",
                    [],
                )
                .map_err(|error| storage_error("migrate SQLite session cwd", error))?;
        }
        if !columns.contains("metadata_json") {
            transaction
                .execute("ALTER TABLE sessions ADD COLUMN metadata_json TEXT", [])
                .map_err(|error| storage_error("migrate SQLite session metadata", error))?;
        }
    }
    transaction
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS session_writer_leases (
                session_id TEXT PRIMARY KEY,
                owner_id TEXT NOT NULL,
                fence INTEGER NOT NULL CHECK(fence > 0),
                expires_at_ms INTEGER NOT NULL
            );",
        )
        .map_err(|error| storage_error("create SQLite session support tables", error))?;
    transaction
        .commit()
        .map_err(|error| storage_error("commit SQLite schema migration", error))?;
    connection
        .execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = FULL;")
        .map_err(|error| storage_error("configure SQLite durability", error))?;
    Ok(())
}

/// Return true when a recognized sessions table exists; false means this is an
/// empty database. Any incompatible layout fails before this adapter changes it.
pub(super) fn inspect_schema(connection: &Connection) -> SessionResult<bool> {
    let has_sessions: bool = connection
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'sessions'",
            [],
            |_| Ok(true),
        )
        .optional()
        .map_err(|error| storage_error("inspect SQLite sessions table", error))?
        .unwrap_or(false);
    if !has_sessions {
        let other_table: Option<String> = connection
            .query_row(
                "SELECT name FROM sqlite_master WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite_%' LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| storage_error("inspect SQLite database layout", error))?;
        if let Some(name) = other_table {
            return Err(SessionError::storage(format!(
                "Unsupported SQLite database layout: table {name} exists without a Rust sessions table"
            )));
        }
        return Ok(false);
    }

    let columns = session_column_info(connection)?;
    let missing: Vec<_> = BASE_COLUMNS
        .iter()
        .filter(|column| !columns.contains_key(**column))
        .copied()
        .collect();
    let unknown: Vec<_> = columns
        .keys()
        .filter(|column| {
            !BASE_COLUMNS.contains(&column.as_str()) && !OWN_COLUMNS.contains(&column.as_str())
        })
        .cloned()
        .collect();
    let wrong_types: Vec<_> = columns
        .iter()
        .filter_map(|(column, info)| {
            let expected = match column.as_str() {
                "id" | "parent_session_id" | "state_json" | "cwd" | "metadata_json" => "TEXT",
                "created_at" => "INTEGER",
                _ => return None,
            };
            (!info.declared_type.eq_ignore_ascii_case(expected)).then(|| column.clone())
        })
        .collect();
    let wrong_primary_key = columns
        .get("id")
        .map(|column| column.primary_key_position != 1)
        .unwrap_or(true)
        || columns
            .iter()
            .any(|(name, column)| name != "id" && column.primary_key_position != 0);
    let missing_not_null: Vec<_> = ["created_at", "state_json", "cwd"]
        .iter()
        .filter_map(|name| {
            columns
                .get(*name)
                .filter(|column| !column.not_null)
                .map(|_| (*name).to_string())
        })
        .collect();
    if !missing.is_empty()
        || !unknown.is_empty()
        || !wrong_types.is_empty()
        || wrong_primary_key
        || !missing_not_null.is_empty()
    {
        let primary_key_problem = wrong_primary_key.then_some("id must be the sole primary key");
        let mut incompatible_constraints = missing_not_null;
        if let Some(problem) = primary_key_problem {
            incompatible_constraints.push(problem.to_string());
        }
        return Err(SessionError::storage(format!(
            "Unsupported SQLite sessions layout (missing: [{}], unknown: [{}], incompatible types: [{}], incompatible constraints: [{}]); expected the Rust state_json snapshot schema",
            missing.join(", "),
            unknown.join(", "),
            wrong_types.join(", "),
            incompatible_constraints.join(", ")
        )));
    }
    validate_lease_schema(connection)?;
    Ok(true)
}

fn validate_lease_schema(connection: &Connection) -> SessionResult<()> {
    let object_type = connection
        .query_row(
            "SELECT type FROM sqlite_master WHERE type IN ('table', 'view') AND name = 'session_writer_leases' COLLATE NOCASE",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| storage_error("inspect SQLite writer lease table", error))?;
    let Some(object_type) = object_type else {
        return Ok(());
    };
    if object_type != "table" {
        return Err(SessionError::storage(
            "Unsupported SQLite session writer lease object; expected a table",
        ));
    }
    let mut statement = connection
        .prepare("PRAGMA table_info(session_writer_leases)")
        .map_err(|error| storage_error("inspect SQLite writer lease columns", error))?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(5)?,
            ))
        })
        .map_err(|error| storage_error("inspect SQLite writer lease columns", error))?;
    let columns = rows
        .map(|row| row.map_err(|error| storage_error("read SQLite writer lease column", error)))
        .collect::<SessionResult<Vec<_>>>()?;
    let expected = [
        ("session_id", "TEXT", 1),
        ("owner_id", "TEXT", 0),
        ("fence", "INTEGER", 0),
        ("expires_at_ms", "INTEGER", 0),
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
            "Unsupported SQLite session writer lease layout",
        ));
    }
    Ok(())
}

pub(super) fn session_columns(connection: &Connection) -> SessionResult<HashSet<String>> {
    Ok(session_column_info(connection)?.into_keys().collect())
}

#[derive(Debug)]
struct SessionColumnInfo {
    declared_type: String,
    not_null: bool,
    primary_key_position: i64,
}

fn session_column_info(
    connection: &Connection,
) -> SessionResult<std::collections::HashMap<String, SessionColumnInfo>> {
    let mut statement = connection
        .prepare("PRAGMA table_info(sessions)")
        .map_err(|error| storage_error("inspect SQLite session columns", error))?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(1)?,
                SessionColumnInfo {
                    declared_type: row.get(2)?,
                    not_null: row.get::<_, i64>(3)? != 0,
                    primary_key_position: row.get(5)?,
                },
            ))
        })
        .map_err(|error| storage_error("inspect SQLite session columns", error))?;
    rows.map(|row| row.map_err(|error| storage_error("read SQLite session column", error)))
        .collect()
}

pub(super) fn storage_error(context: &str, error: impl std::fmt::Display) -> SessionError {
    SessionError::storage(format!("{context}: {error}"))
}
