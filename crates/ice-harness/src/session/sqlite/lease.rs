use rusqlite::{params, OptionalExtension, Transaction};

use crate::error::{SessionError, SessionResult};

use super::schema::storage_error;

#[derive(Debug, Clone)]
pub(super) struct LeaseToken {
    pub session_id: String,
    pub owner_id: String,
    pub fence: i64,
}

pub(super) fn acquire(
    transaction: &Transaction<'_>,
    session_id: &str,
    owner_id: &str,
    now_ms: i64,
    ttl_ms: i64,
) -> SessionResult<LeaseToken> {
    let current: Option<(String, i64, i64)> = transaction
        .query_row(
            "SELECT owner_id, fence, expires_at_ms FROM session_writer_leases WHERE session_id = ?1",
            [session_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|error| storage_error("read SQLite writer lease", error))?;

    let fence = match current {
        None => {
            transaction
                .execute(
                    "INSERT INTO session_writer_leases(session_id, owner_id, fence, expires_at_ms) VALUES (?1, ?2, 1, ?3)",
                    params![session_id, owner_id, expiry(now_ms, ttl_ms)?],
                )
                .map_err(|error| storage_error("acquire SQLite writer lease", error))?;
            1
        }
        Some((_, fence, expires_at_ms)) if expires_at_ms <= now_ms => {
            let next_fence = fence
                .checked_add(1)
                .filter(|value| *value > 0)
                .ok_or_else(|| {
                    SessionError::storage(format!(
                        "SQLite session {session_id} writer fence exhausted"
                    ))
                })?;
            let changed = transaction
                .execute(
                    "UPDATE session_writer_leases SET owner_id = ?1, fence = ?2, expires_at_ms = ?3 WHERE session_id = ?4 AND fence = ?5 AND expires_at_ms <= ?6",
                    params![owner_id, next_fence, expiry(now_ms, ttl_ms)?, session_id, fence, now_ms],
                )
                .map_err(|error| storage_error("take over expired SQLite writer lease", error))?;
            if changed != 1 {
                return Err(SessionError::storage(format!(
                    "SQLite session {session_id} already has an active writer"
                )));
            }
            next_fence
        }
        Some(_) => {
            return Err(SessionError::storage(format!(
                "SQLite session {session_id} already has an active writer"
            )))
        }
    };

    Ok(LeaseToken {
        session_id: session_id.to_string(),
        owner_id: owner_id.to_string(),
        fence,
    })
}

/// Validate the exact owner/fence and renew the lease. Expired leases cannot
/// be revived, even if a takeover has not happened yet.
pub(super) fn renew(
    transaction: &Transaction<'_>,
    lease: &LeaseToken,
    now_ms: i64,
    ttl_ms: i64,
) -> SessionResult<()> {
    let changed = transaction
        .execute(
            "UPDATE session_writer_leases SET expires_at_ms = ?1 WHERE session_id = ?2 AND owner_id = ?3 AND fence = ?4 AND expires_at_ms > ?5",
            params![expiry(now_ms, ttl_ms)?, lease.session_id, lease.owner_id, lease.fence, now_ms],
        )
        .map_err(|error| storage_error("renew SQLite writer lease", error))?;
    if changed != 1 {
        return Err(SessionError::storage(format!(
            "SQLite session {} writer lease was lost",
            lease.session_id
        )));
    }
    Ok(())
}

pub(super) fn release(transaction: &Transaction<'_>, lease: &LeaseToken) -> SessionResult<()> {
    transaction
        .execute(
            "UPDATE session_writer_leases SET expires_at_ms = ?1 WHERE session_id = ?2 AND owner_id = ?3 AND fence = ?4",
            params![i64::MIN, lease.session_id, lease.owner_id, lease.fence],
        )
        .map_err(|error| storage_error("release SQLite writer lease", error))?;
    Ok(())
}

pub(super) fn expiry(now_ms: i64, ttl_ms: i64) -> SessionResult<i64> {
    now_ms
        .checked_add(ttl_ms)
        .ok_or_else(|| SessionError::storage("SQLite writer lease expiry overflow"))
}
