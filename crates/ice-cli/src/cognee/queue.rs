//! Circuit breakers, the durable pending-remember queue and the session
//! cache warmup buffer. Ports `ice-cognee.ts` (COG-031..036).
//!
//! The queue is not exactly-once. A kernel advisory dispatch lease prevents
//! simultaneous processes from sending the same durable intent. Records marked
//! `uncertain` (a timed-out or aborted write that may have been accepted) are
//! only replayed by an explicit `/cognee flush`.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use fs2::FileExt;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use super::config::{create_private_dir, write_private_file_atomic};

pub const CIRCUIT_FAILURE_LIMIT: u32 = 3;
pub const CIRCUIT_COOLDOWN_MS: u64 = 30_000;
pub const MAX_WARMUP_ENTRIES: usize = 256;
pub const WARMUP_DRAIN_BATCH: usize = 16;
pub const PENDING_DRAIN_BATCH: usize = 4;

/// Exclusive process-wide ownership for dispatch and stale inflight recovery.
pub struct DispatchLease(File);

impl DispatchLease {
    pub fn try_acquire(directory: &Path) -> std::io::Result<Option<Self>> {
        create_private_dir(directory)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(directory.join(".dispatch.lock"))?;
        match file.try_lock_exclusive() {
            Ok(()) => Ok(Some(Self(file))),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
            Err(error) => Err(error),
        }
    }
}

impl Drop for DispatchLease {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}

/// `CircuitState`. No half-open state: after the cooldown every eligible
/// request may proceed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CircuitState {
    pub failures: u32,
    pub open_until: u64,
    pub last_error: Option<String>,
}

impl CircuitState {
    pub fn can_attempt(&self, now_ms: u64) -> bool {
        self.open_until <= now_ms
    }

    pub fn note_failure(&mut self, error: &str, now_ms: u64) {
        self.failures += 1;
        if self.failures >= CIRCUIT_FAILURE_LIMIT {
            self.open_until = now_ms + CIRCUIT_COOLDOWN_MS;
        }
        self.last_error = Some(error.into());
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PendingState {
    Pending,
    Uncertain,
    InFlight,
    Prepared,
}

/// One replayable permanent-memory write (`PendingRemember`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingRemember {
    pub operation_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub admission_id: Option<String>,
    pub content_hash: String,
    pub dataset: String,
    pub node_set: String,
    /// Kept as a JSON number so an integer stays an integer on rewrite.
    pub created_at: serde_json::Number,
    pub state: PendingState,
    pub text: String,
}

pub fn sha256_hex(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn pending_record_path(directory: &Path, operation_id: &str) -> std::io::Result<PathBuf> {
    let Some((created_at, hash)) = operation_id.split_once('-') else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Invalid pending remember operation ID",
        ));
    };
    if created_at.is_empty()
        || !created_at.bytes().all(|byte| byte.is_ascii_digit())
        || created_at.parse::<u64>().is_err()
        || hash.len() != 16
        || !hash.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Invalid pending remember operation ID",
        ));
    }
    Ok(directory.join(format!("{operation_id}.json")))
}

/// `createPendingRemember`. The operation id is `<createdAt>-<hash16>`;
/// the hash includes the original dataset and node set.
pub fn create_pending_remember(
    text: &str,
    dataset: &str,
    node_set: &str,
    created_at_ms: u64,
) -> PendingRemember {
    let content_hash = sha256_hex(text);
    let semantic_hash = sha256_hex(&format!("{dataset}\0{node_set}\0{text}"));
    PendingRemember {
        operation_id: format!("{created_at_ms}-{}", &semantic_hash[..16]),
        admission_id: None,
        content_hash,
        dataset: dataset.into(),
        node_set: node_set.into(),
        created_at: created_at_ms.into(),
        state: PendingState::Pending,
        text: text.into(),
    }
}

fn json_files(directory: &Path) -> std::io::Result<Vec<String>> {
    let mut names = Vec::new();
    for entry in std::fs::read_dir(directory)? {
        let name = entry?.file_name().to_string_lossy().into_owned();
        if name.ends_with(".json") {
            names.push(name);
        }
    }
    Ok(names)
}

fn record_line(record: &PendingRemember) -> String {
    format!(
        "{}\n",
        serde_json::to_string(record).expect("pending record serializes")
    )
}

/// `enqueuePendingRemember`: true when written or already present, false
/// when the queue already holds `limit` JSON files.
pub fn enqueue_pending_remember(
    directory: &Path,
    record: &PendingRemember,
    limit: usize,
) -> std::io::Result<bool> {
    let path = pending_record_path(directory, &record.operation_id)?;
    create_private_dir(directory)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(directory.join(".enqueue.lock"))?;
    lock.lock_exclusive()?;
    let result = (|| {
        let files = json_files(directory)?;
        let filename = format!("{}.json", record.operation_id);
        if path.exists()
            || read_pending_remember(directory).iter().any(|existing| {
                existing.dataset == record.dataset
                    && existing.node_set == record.node_set
                    && existing.text == record.text
            })
        {
            return Ok(true);
        }
        if files.iter().filter(|name| !name.starts_with('.')).count() >= limit {
            return Ok(false);
        }
        write_private_file_atomic(&directory.join(filename), &record_line(record))?;
        Ok(true)
    })();
    let _ = FileExt::unlock(&lock);
    result
}

/// Reserve a non-dispatchable admission operation under the queue capacity lock.
pub fn reserve_prepared(
    directory: &Path,
    record: &PendingRemember,
    limit: usize,
) -> std::io::Result<bool> {
    let path = pending_record_path(directory, &record.operation_id)?;
    create_private_dir(directory)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(directory.join(".enqueue.lock"))?;
    lock.lock_exclusive()?;
    let result = (|| {
        if path.exists() {
            return Ok(true);
        }
        let files = json_files(directory)?;
        if files.iter().filter(|name| !name.starts_with('.')).count() >= limit {
            return Ok(false);
        }
        let mut prepared = record.clone();
        prepared.state = PendingState::Prepared;
        write_private_file_atomic(&path, &record_line(&prepared))?;
        Ok(true)
    })();
    let _ = FileExt::unlock(&lock);
    result
}
/// Valid records in filename order; unreadable or malformed files are
/// skipped and left in place.
pub fn read_pending_remember(directory: &Path) -> Vec<PendingRemember> {
    let Ok(mut files) = json_files(directory) else {
        return Vec::new();
    };
    files.sort();
    files
        .into_iter()
        .filter_map(|file| {
            let text = std::fs::read_to_string(directory.join(&file)).ok()?;
            let value: Value = serde_json::from_str(&text).ok()?;
            // Shallow type checks only; extra keys are allowed.
            let object = value.as_object()?;
            let string = |key: &str| object.get(key).is_some_and(Value::is_string);
            if !(string("operationId")
                && string("contentHash")
                && string("dataset")
                && string("nodeSet")
                && string("text")
                && object.get("createdAt").is_some_and(Value::is_number))
            {
                return None;
            }
            let record: PendingRemember = serde_json::from_value(value).ok()?;
            let expected_file = format!("{}.json", record.operation_id);
            (pending_record_path(directory, &record.operation_id).is_ok() && file == expected_file)
                .then_some(record)
        })
        .collect()
}

/// Rewrite the first matching record with `state`; missing is a no-op.
pub fn update_pending_remember_state(
    directory: &Path,
    operation_id: &str,
    state: PendingState,
) -> std::io::Result<()> {
    let path = pending_record_path(directory, operation_id)?;
    let Some(mut record) = read_pending_remember(directory)
        .into_iter()
        .find(|record| record.operation_id == operation_id)
    else {
        return Ok(());
    };
    record.state = state;
    write_private_file_atomic(&path, &record_line(&record))
}

/// Delete one record; a missing file is not an error.
pub fn remove_pending_remember(directory: &Path, operation_id: &str) -> std::io::Result<()> {
    let path = pending_record_path(directory, operation_id)?;
    match std::fs::remove_file(path) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

/// Mark interrupted writes as uncertain so they are never replayed automatically.
pub fn recover_inflight(directory: &Path) -> usize {
    let Ok(Some(_lease)) = DispatchLease::try_acquire(directory) else {
        return 0;
    };
    let records = read_pending_remember(directory);
    let mut recovered = 0;
    for record in records {
        if record.state == PendingState::InFlight
            && update_pending_remember_state(
                directory,
                &record.operation_id,
                PendingState::Uncertain,
            )
            .is_ok()
        {
            recovered += 1;
        }
    }
    recovered
}

/// The records a drain would send: pending ones (and uncertain ones when
/// allowed), first `PENDING_DRAIN_BATCH` in filename order.
pub fn select_drain_batch(directory: &Path, allow_uncertain: bool) -> Vec<PendingRemember> {
    read_pending_remember(directory)
        .into_iter()
        .filter(|record| {
            record.state == PendingState::Pending
                || (allow_uncertain
                    && matches!(
                        record.state,
                        PendingState::Uncertain | PendingState::InFlight
                    ))
        })
        .take(PENDING_DRAIN_BATCH)
        .collect()
}

/// One buffered session-cache write.
#[derive(Debug, Clone, PartialEq)]
pub struct WarmupRecord {
    pub path: PathBuf,
    pub session_id: String,
    pub dataset: Option<String>,
    pub entry: Map<String, Value>,
}

/// Buffer a failed cache write; silently dropped past `MAX_WARMUP_ENTRIES`
/// or on any filesystem error.
pub fn buffer_warmup_entry(
    directory: &Path,
    session_id: &str,
    dataset: &str,
    entry: &Map<String, Value>,
    now_ms: u64,
) {
    let write = || -> std::io::Result<()> {
        std::fs::create_dir_all(directory)?;
        if json_files(directory)?.len() >= MAX_WARMUP_ENTRIES {
            return Ok(());
        }
        let id = uuid::Uuid::new_v4().to_string();
        let path = directory.join(format!("{now_ms}-{}.json", &id[..8]));
        let line = format!(
            "{}\n",
            serde_json::json!({"sessionId": session_id, "dataset": dataset, "entry": entry})
        );
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        use std::io::Write;
        options.open(path)?.write_all(line.as_bytes())
    };
    let _ = write();
}

/// Up to `WARMUP_DRAIN_BATCH` buffered records in directory order. Files
/// that do not parse or lack a session id or entry are skipped and kept.
pub fn read_warmup_batch(directory: &Path) -> Vec<WarmupRecord> {
    let Ok(files) = json_files(directory) else {
        return Vec::new();
    };
    files
        .into_iter()
        .take(WARMUP_DRAIN_BATCH)
        .filter_map(|file| {
            let path = directory.join(file);
            let value: Value = serde_json::from_str(&std::fs::read_to_string(&path).ok()?).ok()?;
            let session_id = value
                .get("sessionId")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())?
                .to_string();
            let entry = value.get("entry").and_then(Value::as_object)?.clone();
            Some(WarmupRecord {
                path,
                session_id,
                dataset: value
                    .get("dataset")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                entry,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn circuit_opens_after_three_failures_and_resets() {
        let mut circuit = CircuitState::default();
        circuit.note_failure("timeout", 1_000);
        circuit.note_failure("timeout", 1_000);
        assert!(circuit.can_attempt(1_000));
        circuit.note_failure("unreachable", 2_000);
        assert!(!circuit.can_attempt(31_999));
        assert!(circuit.can_attempt(32_000));
        assert_eq!(circuit.last_error.as_deref(), Some("unreachable"));
        circuit.note_failure("x", 40_000);
        assert_eq!(circuit.open_until, 70_000);
        circuit.reset();
        assert_eq!(circuit, CircuitState::default());
    }

    #[test]
    fn pending_records_have_stable_ids_and_capacity() {
        let dir = tempfile::tempdir().unwrap();
        let pending = dir.path().join("pending");
        let first = create_pending_remember("fact", "ice", "user_context", 42);
        let semantic_hash = sha256_hex("ice\0user_context\0fact");
        assert_eq!(first.content_hash, sha256_hex("fact"));
        assert_eq!(first.operation_id, format!("42-{}", &semantic_hash[..16]));
        assert!(enqueue_pending_remember(&pending, &first, 1).unwrap());
        assert!(enqueue_pending_remember(&pending, &first, 1).unwrap());
        update_pending_remember_state(&pending, &first.operation_id, PendingState::Uncertain)
            .unwrap();
        let second = create_pending_remember("other", "ice", "user_context", 43);
        assert!(!enqueue_pending_remember(&pending, &second, 1).unwrap());
        let same_scope_new_time = create_pending_remember("fact", "ice", "user_context", 43);
        assert!(enqueue_pending_remember(&pending, &same_scope_new_time, 1).unwrap());
        assert_eq!(read_pending_remember(&pending).len(), 1);
        let other_dataset = create_pending_remember("fact", "other", "user_context", 43);
        assert!(enqueue_pending_remember(&pending, &other_dataset, 2).unwrap());
        let other_node_set = create_pending_remember("fact", "ice", "project_docs", 43);
        assert!(enqueue_pending_remember(&pending, &other_node_set, 3).unwrap());
        assert_eq!(read_pending_remember(&pending).len(), 3);
        let text =
            std::fs::read_to_string(pending.join(format!("{}.json", first.operation_id))).unwrap();
        assert!(text.starts_with("{\"operationId\":\"42-"));
        assert!(text.ends_with("\"state\":\"uncertain\",\"text\":\"fact\"}\n"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&pending).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700);
        }
    }

    #[test]
    fn old_pending_and_uncertain_records_still_load() {
        let dir = tempfile::tempdir().unwrap();
        let pending = dir.path();
        let legacy_pending = r#"{"operationId":"1-2d711642b726b044","contentHash":"2d711642b726b04401627ca9fbac32f5c8530fb1903cc4db02258717921a4881","dataset":"ice","nodeSet":"user_context","createdAt":1,"state":"pending","text":"x"}"#;
        let legacy_uncertain = r#"{"operationId":"2-2d711642b726b044","contentHash":"2d711642b726b04401627ca9fbac32f5c8530fb1903cc4db02258717921a4881","dataset":"ice","nodeSet":"user_context","createdAt":2,"state":"uncertain","text":"x"}"#;
        std::fs::write(pending.join("1-2d711642b726b044.json"), legacy_pending).unwrap();
        std::fs::write(pending.join("2-2d711642b726b044.json"), legacy_uncertain).unwrap();

        let records = read_pending_remember(pending);
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].state, PendingState::Pending);
        assert_eq!(records[1].state, PendingState::Uncertain);
        assert_eq!(select_drain_batch(pending, false).len(), 1);
        assert_eq!(select_drain_batch(pending, true).len(), 2);
    }

    #[test]
    fn live_dispatch_owner_blocks_second_owner_and_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let record = create_pending_remember("fact", "ice", "user_context", 1);
        let mut inflight = record.clone();
        inflight.state = PendingState::InFlight;
        assert!(enqueue_pending_remember(dir.path(), &inflight, 4).unwrap());
        let owner = DispatchLease::try_acquire(dir.path()).unwrap().unwrap();
        assert!(DispatchLease::try_acquire(dir.path()).unwrap().is_none());
        assert_eq!(recover_inflight(dir.path()), 0);
        assert_eq!(
            read_pending_remember(dir.path())[0].state,
            PendingState::InFlight
        );
        drop(owner);
        assert_eq!(recover_inflight(dir.path()), 1);
        assert_eq!(
            read_pending_remember(dir.path())[0].state,
            PendingState::Uncertain
        );
    }

    #[test]
    fn inflight_recovery_and_flush_selection() {
        let dir = tempfile::tempdir().unwrap();
        let record = create_pending_remember("fact", "ice", "user_context", 1);
        let mut inflight = record.clone();
        inflight.state = PendingState::InFlight;
        assert!(enqueue_pending_remember(dir.path(), &inflight, 4).unwrap());
        assert!(select_drain_batch(dir.path(), false).is_empty());
        assert_eq!(recover_inflight(dir.path()), 1);
        assert_eq!(
            read_pending_remember(dir.path())[0].state,
            PendingState::Uncertain
        );
        assert!(select_drain_batch(dir.path(), false).is_empty());

        update_pending_remember_state(dir.path(), &record.operation_id, PendingState::InFlight)
            .unwrap();
        assert!(select_drain_batch(dir.path(), true)
            .iter()
            .any(|entry| entry.state == PendingState::InFlight));
    }

    #[test]
    fn load_update_remove_and_drain_selection() {
        let dir = tempfile::tempdir().unwrap();
        let pending = dir.path();
        for (text, at) in [("a", 3), ("b", 1), ("c", 2), ("d", 4), ("e", 5)] {
            enqueue_pending_remember(pending, &create_pending_remember(text, "ice", "n", at), 64)
                .unwrap();
        }
        std::fs::write(pending.join("0-bad.json"), "{\"operationId\":1}").unwrap();
        std::fs::write(pending.join("note.txt"), "x").unwrap();
        let records = read_pending_remember(pending);
        assert_eq!(
            records.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(),
            vec!["b", "c", "a", "d", "e"]
        );
        update_pending_remember_state(pending, &records[0].operation_id, PendingState::Uncertain)
            .unwrap();
        let batch = select_drain_batch(pending, false);
        assert_eq!(
            batch.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(),
            vec!["c", "a", "d", "e"]
        );
        assert_eq!(select_drain_batch(pending, true)[0].text, "b");
        remove_pending_remember(pending, &records[0].operation_id).unwrap();
        remove_pending_remember(pending, &records[0].operation_id).unwrap();
        assert_eq!(read_pending_remember(pending).len(), 4);
        assert!(pending.join("0-bad.json").exists());
        assert!(read_pending_remember(&pending.join("missing")).is_empty());
    }

    #[test]
    fn pending_record_ids_cannot_escape_the_queue_directory() {
        let root = tempfile::tempdir().unwrap();
        let pending = root.path().join("pending");
        std::fs::create_dir_all(&pending).unwrap();
        let victim = root.path().join("victim.json");
        std::fs::write(&victim, "keep").unwrap();
        std::fs::write(
            pending.join("record.json"),
            json!({
                "operationId": "../victim",
                "contentHash": "hash",
                "dataset": "ice",
                "nodeSet": "user_context",
                "createdAt": 1,
                "state": "pending",
                "text": "fact"
            })
            .to_string(),
        )
        .unwrap();

        assert!(read_pending_remember(&pending).is_empty());
        assert!(
            update_pending_remember_state(&pending, "../victim", PendingState::Uncertain).is_err()
        );
        assert!(remove_pending_remember(&pending, "../victim").is_err());
        assert_eq!(std::fs::read_to_string(victim).unwrap(), "keep");
    }

    #[test]
    fn warmup_buffer_caps_and_reads_valid_records() {
        let dir = tempfile::tempdir().unwrap();
        let warmup = dir.path().join("warmup");
        let entry = json!({"type": "qa", "question": "q"});
        buffer_warmup_entry(&warmup, "ice_s", "ice", entry.as_object().unwrap(), 7);
        std::fs::write(warmup.join("1-bad.json"), "{\"sessionId\":\"\"}").unwrap();
        let batch = read_warmup_batch(&warmup);
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].session_id, "ice_s");
        assert_eq!(batch[0].dataset.as_deref(), Some("ice"));
        assert_eq!(Value::Object(batch[0].entry.clone()), entry);
        for index in 0..MAX_WARMUP_ENTRIES {
            std::fs::write(warmup.join(format!("fill-{index}.json")), "{}").unwrap();
        }
        let before = std::fs::read_dir(&warmup).unwrap().count();
        buffer_warmup_entry(&warmup, "ice_s", "ice", entry.as_object().unwrap(), 8);
        assert_eq!(std::fs::read_dir(&warmup).unwrap().count(), before);
    }
}
