//! The dream ledger: which captured session revisions still need an
//! improve pass. Ports `ice-cognee.ts` (COG-037..041).
//!
//! Records live in `<storage>/dream/<sessionId>.json`, or
//! `<sessionId>.<sha256(dataset)>.json` when a session wrote to more than
//! one dataset. Mutations hold a `<file>.lock` directory lock, matching
//! `proper-lockfile` (5 retries, 50-500 ms backoff, 10 s stale), so ledger
//! updates from several icers processes do not lose captures.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde_json::{json, Map, Value};

use super::config::{create_private_dir, write_private_file_atomic};
use super::queue::sha256_hex;

pub const DREAM_QUIET_MS: f64 = 900_000.0;
pub const DREAM_MAX_AGE_MS: f64 = 604_800_000.0;
pub const DREAM_CLAIM_LEASE_MS: f64 = 60_000.0;
const LOCK_STALE: Duration = Duration::from_millis(10_000);
const LOCK_RETRIES: u32 = 5;
const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

#[derive(Debug, Clone, PartialEq)]
pub struct ImproveLease {
    pub token: String,
    pub expires_at: f64,
}

/// `DreamRecord`. `ledger_file` is the basename it was read from and is not
/// persisted.
#[derive(Debug, Clone, PartialEq)]
pub struct DreamRecord {
    pub session_id: String,
    pub dataset: String,
    pub captured_at: f64,
    pub capture_revision: f64,
    pub improved_revision: f64,
    pub improved_at: Option<f64>,
    pub improve_lease: Option<ImproveLease>,
    pub ledger_file: Option<String>,
}

/// `DreamImprovementClaim`.
#[derive(Debug, Clone, PartialEq)]
pub struct DreamClaim {
    pub token: String,
    pub capture_revision: f64,
    pub ledger_file: Option<String>,
}

/// A JSON number written the way JavaScript prints it (integers stay
/// integers).
fn number(value: f64) -> Value {
    if value.fract() == 0.0 && value.abs() <= MAX_SAFE_INTEGER {
        json!(value as i64)
    } else {
        json!(value)
    }
}

fn safe_integer(value: Option<&Value>) -> Option<f64> {
    let value = value?.as_f64()?;
    (value.fract() == 0.0 && value.abs() <= MAX_SAFE_INTEGER).then_some(value)
}

fn basename(path: &Path) -> Option<String> {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
}

fn is_plain_session_id(session_id: &str) -> bool {
    !session_id.is_empty()
        && !session_id.contains('/')
        && !session_id.contains('\\')
        && !session_id.contains('\0')
        && Path::new(session_id)
            .file_name()
            .is_some_and(|name| name.to_string_lossy() == session_id)
}

/// `readDreamRecord`.
pub fn read_dream_record(path: &Path) -> Option<DreamRecord> {
    let parsed: Value = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    let object = parsed.as_object()?;
    let session_id = object.get("sessionId")?.as_str()?.to_string();
    let dataset = object.get("dataset")?.as_str()?.to_string();
    let captured_at = object.get("capturedAt")?.as_f64()?;
    let improved_at = match object.get("improvedAt") {
        None => None,
        Some(value) => Some(value.as_f64()?),
    };
    let mut capture_revision = 1.0;
    let mut improved_revision = 0.0;
    if object.contains_key("captureRevision") || object.contains_key("improvedRevision") {
        let capture = safe_integer(object.get("captureRevision"))?;
        let improved = safe_integer(object.get("improvedRevision"))?;
        if capture < 1.0 || improved < 0.0 || improved > capture {
            return None;
        }
        capture_revision = capture;
        improved_revision = improved;
    } else if improved_at.is_some_and(|at| at > captured_at) {
        // Legacy records had timestamps only.
        improved_revision = capture_revision;
    }
    let improve_lease = object
        .get("improveLease")
        .and_then(Value::as_object)
        .and_then(|lease| {
            let token = lease.get("token")?.as_str().filter(|t| !t.is_empty())?;
            let expires_at = lease
                .get("expiresAt")?
                .as_f64()
                .filter(|at| at.is_finite())?;
            Some(ImproveLease {
                token: token.into(),
                expires_at,
            })
        });
    Some(DreamRecord {
        session_id,
        dataset,
        captured_at,
        capture_revision,
        improved_revision,
        improved_at,
        improve_lease,
        ledger_file: basename(path),
    })
}

fn scoped_dream_path(directory: &Path, session_id: &str, dataset: &str) -> Option<PathBuf> {
    is_plain_session_id(session_id)
        .then(|| directory.join(format!("{session_id}.{}.json", sha256_hex(dataset))))
}

/// `dreamLedgerPath`: `None` for a file name that is not a plain basename
/// ending in `.json`.
pub fn dream_ledger_path(
    directory: &Path,
    session_id: &str,
    ledger_file: Option<&str>,
) -> Option<PathBuf> {
    if !is_plain_session_id(session_id) {
        return None;
    }
    let file = ledger_file
        .map(str::to_string)
        .unwrap_or_else(|| format!("{session_id}.json"));
    let plain = Path::new(&file)
        .file_name()
        .is_some_and(|name| name.to_string_lossy() == file)
        && !file.contains('/')
        && !file.contains('\\');
    if !plain || !file.ends_with(".json") {
        return None;
    }
    Some(directory.join(file))
}

fn write_dream_record(path: &Path, record: &DreamRecord) -> std::io::Result<()> {
    let mut object = Map::new();
    object.insert("sessionId".into(), json!(record.session_id));
    object.insert("dataset".into(), json!(record.dataset));
    object.insert("capturedAt".into(), number(record.captured_at));
    object.insert("captureRevision".into(), number(record.capture_revision));
    object.insert("improvedRevision".into(), number(record.improved_revision));
    if let Some(at) = record.improved_at {
        object.insert("improvedAt".into(), number(at));
    }
    if let Some(lease) = &record.improve_lease {
        object.insert(
            "improveLease".into(),
            json!({"token": lease.token, "expiresAt": number(lease.expires_at)}),
        );
    }
    write_private_file_atomic(path, &format!("{}\n", Value::Object(object)))
}

/// `readDreamRecordForDataset`: the scoped record first, then the legacy
/// one, each only when its dataset matches.
pub fn read_dream_record_for_dataset(
    directory: &Path,
    session_id: &str,
    dataset: &str,
) -> Option<DreamRecord> {
    let scoped_path = scoped_dream_path(directory, session_id, dataset)?;
    if let Some(scoped) = read_dream_record(&scoped_path) {
        if scoped.dataset == dataset {
            return Some(scoped);
        }
    }
    let legacy_path = dream_ledger_path(directory, session_id, None)?;
    read_dream_record(&legacy_path).filter(|legacy| legacy.dataset == dataset)
}

/// A held `<path>.lock` directory; removed on drop.
pub struct DreamLock {
    path: PathBuf,
}

impl Drop for DreamLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir(&self.path);
    }
}

fn lock_is_stale(lock: &Path) -> bool {
    std::fs::metadata(lock)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .is_some_and(|age| age > LOCK_STALE)
}

/// `acquireDreamLock`.
pub fn acquire_dream_lock(path: &Path) -> std::io::Result<DreamLock> {
    if let Some(parent) = path.parent() {
        create_private_dir(parent)?;
    }
    let mut lock = path.as_os_str().to_owned();
    lock.push(".lock");
    let lock = PathBuf::from(lock);
    let mut delay = 50u64;
    let mut attempt = 0;
    loop {
        match std::fs::create_dir(&lock) {
            Ok(()) => return Ok(DreamLock { path: lock }),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                if lock_is_stale(&lock) {
                    let _ = std::fs::remove_dir(&lock);
                    continue;
                }
                if attempt >= LOCK_RETRIES {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::WouldBlock,
                        "Cognee dream ledger is locked",
                    ));
                }
                attempt += 1;
                std::thread::sleep(Duration::from_millis(delay));
                delay = (delay * 2).min(500);
            }
            Err(error) => return Err(error),
        }
    }
}

/// `markDreamCaptured`: bump the capture revision; a session id that is not
/// a plain file name is ignored.
pub fn mark_dream_captured(
    directory: &Path,
    session_id: &str,
    dataset: &str,
    captured_at: f64,
) -> std::io::Result<()> {
    let Some(legacy_path) = dream_ledger_path(directory, session_id, None) else {
        return Ok(());
    };
    let Some(scoped_path) = scoped_dream_path(directory, session_id, dataset) else {
        return Ok(());
    };
    let mut path = if scoped_path.exists() {
        scoped_path.clone()
    } else {
        legacy_path.clone()
    };
    loop {
        let lock = acquire_dream_lock(&path)?;
        let existing = read_dream_record(&path);
        if existing
            .as_ref()
            .is_some_and(|record| record.dataset != dataset)
        {
            if path != legacy_path {
                return Err(std::io::Error::other(
                    "Cognee dream dataset hash collision.",
                ));
            }
            drop(lock);
            path = scoped_path.clone();
            continue;
        }
        let same = existing;
        let record = DreamRecord {
            session_id: session_id.into(),
            dataset: dataset.into(),
            captured_at: same
                .as_ref()
                .map_or(captured_at, |record| record.captured_at.max(captured_at)),
            capture_revision: same.as_ref().map_or(0.0, |record| record.capture_revision) + 1.0,
            improved_revision: same.as_ref().map_or(0.0, |record| record.improved_revision),
            improved_at: same.as_ref().and_then(|record| record.improved_at),
            improve_lease: same
                .as_ref()
                .and_then(|record| record.improve_lease.clone()),
            ledger_file: basename(&path),
        };
        let result = write_dream_record(&path, &record);
        drop(lock);
        return result;
    }
}

/// `claimDreamCandidate`: lease the record when it still matches the
/// candidate snapshot and is due.
pub fn claim_dream_candidate(
    directory: &Path,
    candidate: &DreamRecord,
    now: f64,
    token: &str,
    require_quiet: bool,
) -> Result<Option<DreamRecord>, String> {
    if token.trim().is_empty() {
        return Err("Dream claim token must be nonempty.".into());
    }
    let Some(path) = dream_ledger_path(
        directory,
        &candidate.session_id,
        candidate.ledger_file.as_deref(),
    ) else {
        return Ok(None);
    };
    let Ok(_lock) = acquire_dream_lock(&path) else {
        return Ok(None);
    };
    let Some(current) = read_dream_record(&path) else {
        return Ok(None);
    };
    if current.session_id != candidate.session_id
        || current.dataset != candidate.dataset
        || current.captured_at != candidate.captured_at
        || current.capture_revision != candidate.capture_revision
        || current.capture_revision <= current.improved_revision
        || current.captured_at > now
        || now - current.captured_at > DREAM_MAX_AGE_MS
        || (require_quiet && now - current.captured_at < DREAM_QUIET_MS)
        || current
            .improve_lease
            .as_ref()
            .is_some_and(|lease| lease.expires_at > now)
    {
        return Ok(None);
    }
    let claimed = DreamRecord {
        improve_lease: Some(ImproveLease {
            token: token.into(),
            expires_at: now + DREAM_CLAIM_LEASE_MS,
        }),
        ..current
    };
    write_dream_record(&path, &claimed).map_err(|error| error.to_string())?;
    Ok(Some(claimed))
}

/// `markDreamImproved`: record coverage of the claimed revision. Captures
/// that arrived during the improve stay pending.
pub fn mark_dream_improved(
    directory: &Path,
    session_id: &str,
    improved_at: f64,
    claim: &DreamClaim,
) -> std::io::Result<()> {
    let Some(path) = dream_ledger_path(directory, session_id, claim.ledger_file.as_deref()) else {
        return Ok(());
    };
    let Ok(_lock) = acquire_dream_lock(&path) else {
        return Ok(());
    };
    let Some(record) = read_dream_record(&path) else {
        return Ok(());
    };
    if record
        .improve_lease
        .as_ref()
        .map(|lease| lease.token.as_str())
        != Some(claim.token.as_str())
        || claim.capture_revision > record.capture_revision
    {
        return Ok(());
    }
    if claim.capture_revision >= record.capture_revision {
        let _ = std::fs::remove_file(&path);
        return Ok(());
    }
    let updated = DreamRecord {
        improved_at: Some(record.improved_at.unwrap_or(improved_at).max(improved_at)),
        improved_revision: record.improved_revision.max(claim.capture_revision),
        improve_lease: None,
        ..record
    };
    write_dream_record(&path, &updated)
}

/// `releaseDreamClaim`: drop our lease only.
pub fn release_dream_claim(
    directory: &Path,
    session_id: &str,
    token: &str,
    ledger_file: Option<&str>,
) -> std::io::Result<()> {
    let Some(path) = dream_ledger_path(directory, session_id, ledger_file) else {
        return Ok(());
    };
    let Ok(_lock) = acquire_dream_lock(&path) else {
        return Ok(());
    };
    let Some(record) = read_dream_record(&path) else {
        return Ok(());
    };
    if record
        .improve_lease
        .as_ref()
        .map(|lease| lease.token.as_str())
        != Some(token)
    {
        return Ok(());
    }
    write_dream_record(
        &path,
        &DreamRecord {
            improve_lease: None,
            ..record
        },
    )
}

/// `readDreamCandidates`: due records oldest first, deleting malformed or
/// expired files under their lock.
pub fn read_dream_candidates(
    directory: &Path,
    now: f64,
    exclude_session_id: Option<&str>,
    limit: usize,
    exclude_dataset: Option<&str>,
) -> Vec<DreamRecord> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let expired = |record: &Option<DreamRecord>| {
        record
            .as_ref()
            .is_none_or(|record| now - record.captured_at > DREAM_MAX_AGE_MS)
    };
    let mut due = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.ends_with(".json") {
            continue;
        }
        let path = directory.join(&name);
        let record = read_dream_record(&path);
        if expired(&record) {
            if let Ok(_lock) = acquire_dream_lock(&path) {
                if expired(&read_dream_record(&path)) {
                    let _ = std::fs::remove_file(&path);
                }
            }
            continue;
        }
        let record = record.expect("checked above");
        if (exclude_session_id == Some(record.session_id.as_str())
            && exclude_dataset.is_none_or(|dataset| dataset == record.dataset))
            || record.capture_revision <= record.improved_revision
            || record
                .improve_lease
                .as_ref()
                .is_some_and(|lease| lease.expires_at > now)
            || now - record.captured_at < DREAM_QUIET_MS
        {
            continue;
        }
        due.push(record);
    }
    due.sort_by(|a, b| a.captured_at.total_cmp(&b.captured_at));
    due.truncate(limit);
    due
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: f64 = 3_600_000.0;

    fn claim_of(record: &DreamRecord, token: &str) -> DreamClaim {
        DreamClaim {
            token: token.into(),
            capture_revision: record.capture_revision,
            ledger_file: record.ledger_file.clone(),
        }
    }

    #[test]
    fn legacy_records_migrate_and_invalid_revisions_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.json");
        std::fs::write(
            &path,
            r#"{"sessionId":"s","dataset":"d","capturedAt":5,"improvedAt":6}"#,
        )
        .unwrap();
        let record = read_dream_record(&path).unwrap();
        assert_eq!(
            (record.capture_revision, record.improved_revision),
            (1.0, 1.0)
        );
        std::fs::write(
            &path,
            r#"{"sessionId":"s","dataset":"d","capturedAt":5,"improvedAt":5}"#,
        )
        .unwrap();
        assert_eq!(read_dream_record(&path).unwrap().improved_revision, 0.0);
        std::fs::write(
            &path,
            r#"{"sessionId":"s","dataset":"d","capturedAt":5,"captureRevision":1,"improvedRevision":2}"#,
        )
        .unwrap();
        assert!(read_dream_record(&path).is_none());
        std::fs::write(
            &path,
            r#"{"sessionId":"s","dataset":"d","capturedAt":5,"captureRevision":1.5,"improvedRevision":0}"#,
        )
        .unwrap();
        assert!(read_dream_record(&path).is_none());
        assert!(dream_ledger_path(dir.path(), "../x", None).is_none());
        assert!(dream_ledger_path(dir.path(), "s", Some("a/b.json")).is_none());
        assert!(dream_ledger_path(dir.path(), "s", Some("s.txt")).is_none());
    }

    #[test]
    fn captures_bump_revisions_and_scope_by_dataset() {
        let dir = tempfile::tempdir().unwrap();
        mark_dream_captured(dir.path(), "s", "a", 10.0).unwrap();
        mark_dream_captured(dir.path(), "s", "a", 5.0).unwrap();
        let record = read_dream_record_for_dataset(dir.path(), "s", "a").unwrap();
        assert_eq!((record.captured_at, record.capture_revision), (10.0, 2.0));
        assert_eq!(record.ledger_file.as_deref(), Some("s.json"));
        let text = std::fs::read_to_string(dir.path().join("s.json")).unwrap();
        assert_eq!(
            text,
            "{\"sessionId\":\"s\",\"dataset\":\"a\",\"capturedAt\":10,\"captureRevision\":2,\"improvedRevision\":0}\n"
        );
        mark_dream_captured(dir.path(), "s", "b", 20.0).unwrap();
        let scoped = read_dream_record_for_dataset(dir.path(), "s", "b").unwrap();
        assert_eq!(
            scoped.ledger_file,
            Some(format!("s.{}.json", sha256_hex("b")))
        );
        assert_eq!(
            read_dream_record_for_dataset(dir.path(), "s", "a")
                .unwrap()
                .capture_revision,
            2.0
        );
        assert!(!dir.path().join("s.json.lock").exists());
        mark_dream_captured(dir.path(), "../bad", "a", 1.0).unwrap();
    }

    #[test]
    fn dataset_lookup_rejects_session_ids_that_escape_the_ledger_directory() {
        let root = tempfile::tempdir().unwrap();
        let ledger_dir = root.path().join("dream");
        std::fs::create_dir_all(&ledger_dir).unwrap();
        let outside = root
            .path()
            .join(format!("outside.{}.json", sha256_hex("dataset")));
        std::fs::write(
            &outside,
            r#"{"sessionId":"../outside","dataset":"dataset","capturedAt":1}"#,
        )
        .unwrap();

        assert!(read_dream_record_for_dataset(&ledger_dir, "../outside", "dataset").is_none());
        assert!(outside.exists());
    }

    #[test]
    fn claims_are_revision_fenced_and_leased() {
        let dir = tempfile::tempdir().unwrap();
        mark_dream_captured(dir.path(), "s", "a", 0.0).unwrap();
        let candidate = read_dream_record_for_dataset(dir.path(), "s", "a").unwrap();
        let now = DREAM_QUIET_MS;
        assert!(
            claim_dream_candidate(dir.path(), &candidate, now - 1.0, "t", true)
                .unwrap()
                .is_none()
        );
        assert!(claim_dream_candidate(dir.path(), &candidate, now, " ", true).is_err());
        let claimed = claim_dream_candidate(dir.path(), &candidate, now, "t", true)
            .unwrap()
            .unwrap();
        assert_eq!(
            claimed.improve_lease.as_ref().unwrap().expires_at,
            now + DREAM_CLAIM_LEASE_MS
        );
        // An unexpired lease blocks a second claim; at expiry it is free.
        assert!(
            claim_dream_candidate(dir.path(), &candidate, now + 1.0, "u", true)
                .unwrap()
                .is_none()
        );
        let reclaimed = claim_dream_candidate(
            dir.path(),
            &candidate,
            now + DREAM_CLAIM_LEASE_MS,
            "u",
            true,
        )
        .unwrap()
        .unwrap();
        // The first owner can no longer release or complete.
        release_dream_claim(dir.path(), "s", "t", None).unwrap();
        mark_dream_improved(dir.path(), "s", now, &claim_of(&claimed, "t")).unwrap();
        assert_eq!(
            read_dream_record(&dir.path().join("s.json"))
                .unwrap()
                .improve_lease
                .unwrap()
                .token,
            "u"
        );
        // A capture during the improve keeps the record pending.
        mark_dream_captured(dir.path(), "s", "a", now).unwrap();
        mark_dream_improved(dir.path(), "s", now, &claim_of(&reclaimed, "u")).unwrap();
        let record = read_dream_record(&dir.path().join("s.json")).unwrap();
        assert_eq!(
            (record.capture_revision, record.improved_revision),
            (2.0, 1.0)
        );
        assert!(record.improve_lease.is_none());
        assert_eq!(record.improved_at, Some(now));
        // Covering the latest revision deletes the record.
        let claimed = claim_dream_candidate(dir.path(), &record, now, "v", false)
            .unwrap()
            .unwrap();
        mark_dream_improved(dir.path(), "s", now, &claim_of(&claimed, "v")).unwrap();
        assert!(!dir.path().join("s.json").exists());
    }

    #[test]
    fn candidate_scan_cleans_expired_and_excludes_active_pairs() {
        let dir = tempfile::tempdir().unwrap();
        let now = 10.0 * 24.0 * HOUR;
        mark_dream_captured(dir.path(), "old", "a", now - 8.0 * 24.0 * HOUR).unwrap();
        mark_dream_captured(dir.path(), "b", "a", now - 2.0 * HOUR).unwrap();
        mark_dream_captured(dir.path(), "c", "a", now - 3.0 * HOUR).unwrap();
        mark_dream_captured(dir.path(), "recent", "a", now - 1.0).unwrap();
        mark_dream_captured(dir.path(), "active", "a", now - 4.0 * HOUR).unwrap();
        mark_dream_captured(dir.path(), "active", "z", now - 5.0 * HOUR).unwrap();
        std::fs::write(dir.path().join("bad.json"), "{").unwrap();
        let due = read_dream_candidates(dir.path(), now, Some("active"), 10, Some("a"));
        let ids: Vec<(&str, &str)> = due
            .iter()
            .map(|r| (r.session_id.as_str(), r.dataset.as_str()))
            .collect();
        assert_eq!(ids, vec![("active", "z"), ("c", "a"), ("b", "a")]);
        assert!(!dir.path().join("old.json").exists());
        assert!(!dir.path().join("bad.json").exists());
        let due = read_dream_candidates(dir.path(), now, Some("active"), 2, None);
        assert_eq!(due.len(), 2);
        assert!(due.iter().all(|r| r.session_id != "active"));
    }

    #[cfg(unix)]
    #[test]
    fn stale_locks_are_broken_and_live_locks_time_out() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.json");
        std::fs::create_dir(dir.path().join("s.json.lock")).unwrap();
        assert!(acquire_dream_lock(&path).is_err());
        let old = SystemTime::now() - Duration::from_secs(60);
        let lock = std::fs::File::open(dir.path().join("s.json.lock")).unwrap();
        lock.set_modified(old).unwrap();
        drop(lock);
        let held = acquire_dream_lock(&path).unwrap();
        drop(held);
        assert!(!dir.path().join("s.json.lock").exists());
    }
}
