//! Crash-recoverable admission receipts. A receipt is the durable commit point;
//! candidate and queue files are projections and may be rebuilt from it.
use super::{
    candidates::{Candidate, CandidateState},
    config::{create_private_dir, write_private_file_atomic},
    queue::{self, PendingRemember, PendingState},
};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
};

pub const RECEIPT_VERSION: u32 = 1;
/// Bound untrusted receipt reads and directory scans.
const MAX_RECEIPT_BYTES: u64 = 64 * 1024;
const MAX_RECEIPTS: usize = 4096;
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdmissionReceipt {
    pub version: u32,
    pub id: String,
    pub candidate: Candidate,
    pub operation: PendingRemember,
    pub superseded_target: Option<SupersededTarget>,
    pub committed_at: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SupersededTarget {
    pub source: String,
    pub scope: Option<String>,
    pub hash: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdmissionReceiptResult {
    Accepted {
        candidate_id: String,
        operation_id: String,
        receipt_id: String,
    },
    Duplicate {
        candidate_id: String,
        operation_id: String,
        receipt_id: String,
    },
}

pub fn receipt_dir(candidates_dir: &Path) -> PathBuf {
    candidates_dir.join("admissions")
}
pub fn receipt_path(dir: &Path, id: &str) -> std::io::Result<PathBuf> {
    if id.is_empty() || id.len() > 64 || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "invalid admission id",
        ));
    }
    Ok(dir.join(format!("{id}.json")))
}
pub fn read_receipts(dir: &Path) -> Vec<AdmissionReceipt> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    // Completion markers are bookkeeping, not receipts; they must not use up
    // the cap and starve real receipts once many admissions have finished.
    let receipt_files = entries.flatten().filter(|entry| {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        name.ends_with(".json") && !name.starts_with("completed-")
    });
    for entry in receipt_files.take(MAX_RECEIPTS) {
        let path = entry.path();
        let Ok(metadata) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if !metadata.file_type().is_file() || metadata.len() > MAX_RECEIPT_BYTES {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(r) = serde_json::from_str::<AdmissionReceipt>(&text) else {
            continue;
        };
        if r.version == RECEIPT_VERSION
            && receipt_path(dir, &r.id).ok().as_deref() == Some(path.as_path())
            && valid_receipt(&r)
        {
            out.push(r);
        }
    }
    out
}
pub fn admission_lock(dir: &Path) -> std::io::Result<File> {
    create_private_dir(dir)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(dir.join(".admission.lock"))?;
    lock.lock_exclusive()?;
    Ok(lock)
}
pub fn commit(dir: &Path, receipt: &AdmissionReceipt) -> std::io::Result<()> {
    commit_with(dir, receipt, write_private_file_atomic)
}
pub fn commit_with(
    dir: &Path,
    receipt: &AdmissionReceipt,
    writer: impl FnOnce(&Path, &str) -> std::io::Result<()>,
) -> std::io::Result<()> {
    let path = receipt_path(dir, &receipt.id)?;
    if !valid_receipt(receipt) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "invalid admission receipt",
        ));
    }
    create_private_dir(dir)?;
    let serialized = serde_json::to_string(receipt).map_err(std::io::Error::other)?;
    if path.exists() {
        if std::fs::read(&path)? == serialized.as_bytes() {
            return Ok(());
        }
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "conflicting admission receipt",
        ));
    }
    writer(&path, &serialized)
}
pub fn mark_completed(dir: &Path, operation_id: &str) -> std::io::Result<()> {
    let Some((created, hash)) = operation_id.split_once('-') else {
        return Err(std::io::Error::other("invalid operation id"));
    };
    if created.is_empty()
        || !created.bytes().all(|b| b.is_ascii_digit())
        || hash.len() != 16
        || !hash.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(std::io::Error::other("invalid operation id"));
    }
    create_private_dir(dir)?;
    write_private_file_atomic(
        &dir.join(format!("completed-{operation_id}.json")),
        "{\"completed\":true}",
    )
}
pub fn is_completed(dir: &Path, operation_id: &str) -> bool {
    if !valid_operation_id(operation_id) {
        return false;
    }
    let path = dir.join(format!("completed-{operation_id}.json"));
    let Ok(meta) = std::fs::symlink_metadata(&path) else {
        return false;
    };
    meta.file_type().is_file()
        && meta.len() <= 1024
        && std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .is_some_and(|value| value == serde_json::json!({"completed": true}))
}
fn valid_operation_id(id: &str) -> bool {
    let Some((created, hash)) = id.split_once('-') else {
        return false;
    };
    !created.is_empty()
        && created.bytes().all(|b| b.is_ascii_digit())
        && created.parse::<u64>().is_ok()
        && hash.len() == 16
        && hash.bytes().all(|b| b.is_ascii_hexdigit())
}
fn valid_receipt(r: &AdmissionReceipt) -> bool {
    let c = &r.candidate;
    let o = &r.operation;
    r.version == RECEIPT_VERSION
        && !r.id.is_empty()
        && r.id.len() <= 64
        && r.id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        && c.id.len() == 36
        && uuid::Uuid::parse_str(&c.id).is_ok()
        && c.admission_id.as_deref() == Some(r.id.as_str())
        && o.admission_id.as_deref() == Some(r.id.as_str())
        && c.dataset == o.dataset
        && !c.dataset.is_empty()
        && c.category.node_set() == o.node_set
        && !c.text.trim().is_empty()
        && c.text.chars().count() <= super::candidates::MAX_CANDIDATE_CHARS
        && o.content_hash == queue::sha256_hex(&o.text)
        && valid_operation_id(&o.operation_id)
        && o.state == PendingState::Prepared
        && c.state == CandidateState::Accepted
        && c.source_scope.as_ref().is_none_or(|s| {
            s.source_text_hash.len() == 64
                && s.source_text_hash.bytes().all(|b| b.is_ascii_hexdigit())
        })
        && r.superseded_target.as_ref().is_none_or(|t| {
            t.hash.len() == 64
                && t.hash.bytes().all(|b| b.is_ascii_hexdigit())
                && t.scope == c.branch
        })
}
/// Restore the candidate/outbox projections from committed receipts. Prepared
/// operations with no valid receipt are deliberately left blocked.
pub fn reconcile(
    candidates_dir: &Path,
    pending_dir: &Path,
    queue_limit: usize,
    now: u64,
) -> std::io::Result<usize> {
    let receipts_dir = receipt_dir(candidates_dir);
    // Lock order is dispatch lease, then admission lock, then queue file operations.
    // The lease is held only during local reconciliation; no HTTP is performed here.
    let Some(_dispatch_lease) = queue::DispatchLease::try_acquire(pending_dir)? else {
        return Ok(0);
    };
    let _admission_guard = admission_lock(&receipts_dir)?;
    let mut projected = 0;
    for receipt in read_receipts(&receipts_dir) {
        let mut candidate = receipt.candidate.clone();
        candidate.state = CandidateState::Accepted;
        let path = candidates_dir.join(format!("{}.json", candidate.id));
        if !path.exists() {
            super::candidates::store_pending(candidates_dir, &candidate, now)?;
            projected += 1;
        }
        let mut operation = receipt.operation.clone();
        operation.state = PendingState::Pending;
        if is_completed(&receipts_dir, &operation.operation_id) {
            continue;
        }
        if !queue::read_pending_remember(pending_dir)
            .iter()
            .any(|r| r.operation_id == operation.operation_id)
        {
            if !queue::reserve_prepared(pending_dir, &operation, queue_limit)? {
                return Err(std::io::Error::other(
                    "queue full while recovering committed admission",
                ));
            }
            queue::update_pending_remember_state(
                pending_dir,
                &operation.operation_id,
                PendingState::Pending,
            )?;
            projected += 1;
        } else if let Some(existing) = queue::read_pending_remember(pending_dir)
            .into_iter()
            .find(|r| r.operation_id == operation.operation_id)
        {
            // Only a reservation known never to have reached dispatch may be
            // activated. InFlight/Uncertain means the wire may have occurred.
            if existing.state == PendingState::Prepared {
                queue::update_pending_remember_state(
                    pending_dir,
                    &operation.operation_id,
                    PendingState::Pending,
                )?;
            }
        }
    }
    Ok(projected)
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::cognee::candidates::{Category, Role, Source};

    pub(crate) fn receipt() -> AdmissionReceipt {
        let mut operation =
            queue::create_pending_remember("[lesson] fact", "d", "agent_actions", 7);
        operation.admission_id = Some("admission-1".into());
        operation.state = PendingState::Prepared;
        let candidate = Candidate {
            id: "00000000-0000-4000-8000-000000000001".into(),
            admission_id: Some("admission-1".into()),
            source_scope: None,
            category: Category::Lesson,
            text: "fact".into(),
            source: Source {
                entry_id: "e".into(),
                role: Role::User,
            },
            verified_by: None,
            supersedes: None,
            dataset: "d".into(),
            branch: None,
            valid_until_ms: None,
            created_at: 7,
            state: CandidateState::Accepted,
        };
        AdmissionReceipt {
            version: RECEIPT_VERSION,
            id: "admission-1".into(),
            candidate,
            operation,
            superseded_target: None,
            committed_at: 8,
        }
    }

    #[test]
    fn injected_receipt_write_failure_does_not_commit() {
        let root = tempfile::tempdir().unwrap();
        let r = receipt();
        let result = commit_with(&receipt_dir(root.path()), &r, |_path, _content| {
            Err(std::io::Error::other("injected receipt failure"))
        });
        assert!(result.is_err());
        assert!(read_receipts(&receipt_dir(root.path())).is_empty());
    }

    #[test]
    fn receipt_commit_rejects_conflicts_and_traversal_ids() {
        let root = tempfile::tempdir().unwrap();
        let dir = receipt_dir(root.path());
        let receipt = receipt();
        commit(&dir, &receipt).unwrap();
        let mut conflict = receipt.clone();
        conflict.committed_at += 1;
        assert!(commit(&dir, &conflict).is_err());
        assert!(receipt_path(&dir, "../victim").is_err());
        assert_eq!(read_receipts(&dir), vec![receipt]);
    }

    #[test]
    fn operation_ids_must_fit_the_queue_timestamp() {
        assert!(valid_operation_id("7-0123456789abcdef"));
        assert!(valid_operation_id("18446744073709551615-0123456789abcdef"));
        assert!(!valid_operation_id("18446744073709551616-0123456789abcdef"));
        assert!(!valid_operation_id("+7-0123456789abcdef"));
    }

    #[test]
    fn completion_markers_do_not_count_against_the_receipt_cap() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("receipts");
        let receipt = receipt();
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..(MAX_RECEIPTS + 8) {
            std::fs::write(
                dir.join(format!("completed-{i}-0123456789abcdef.json")),
                r#"{"completed":true}"#,
            )
            .unwrap();
        }
        commit(&dir, &receipt).unwrap();
        assert_eq!(read_receipts(&dir), vec![receipt]);
    }

    #[test]
    fn empty_completion_marker_is_not_authoritative() {
        let root = tempfile::tempdir().unwrap();
        let operation = "7-0123456789abcdef";
        std::fs::write(root.path().join(format!("completed-{operation}.json")), "").unwrap();
        assert!(!is_completed(root.path(), operation));
        assert!(!is_completed(root.path(), "../victim"));
    }

    #[test]
    fn uncommitted_prepared_is_blocked_and_receipt_recovery_projects_once() {
        let root = tempfile::tempdir().unwrap();
        let candidates = root.path().join("candidates");
        let pending = root.path().join("pending");
        let r = receipt();
        queue::enqueue_pending_remember(&pending, &r.operation, 4).unwrap();
        assert!(queue::select_drain_batch(&pending, false).is_empty());
        assert!(read_receipts(&receipt_dir(&candidates)).is_empty());
        commit(&receipt_dir(&candidates), &r).unwrap();
        assert_eq!(reconcile(&candidates, &pending, 4, 10).unwrap(), 1);
        assert_eq!(reconcile(&candidates, &pending, 4, 10).unwrap(), 0);
        assert_eq!(
            queue::read_pending_remember(&pending)[0].state,
            PendingState::Pending
        );
        assert_eq!(
            super::super::candidates::read_candidates(&candidates)[0].state,
            CandidateState::Accepted
        );
    }

    #[test]
    fn receipt_recovery_never_resets_inflight_or_uncertain() {
        let root = tempfile::tempdir().unwrap();
        let candidates = root.path().join("candidates");
        let pending = root.path().join("pending");
        let mut r = receipt();
        commit(&receipt_dir(&candidates), &r).unwrap();
        r.operation.state = PendingState::InFlight;
        queue::enqueue_pending_remember(&pending, &r.operation, 4).unwrap();
        queue::update_pending_remember_state(
            &pending,
            &r.operation.operation_id,
            PendingState::Uncertain,
        )
        .unwrap();
        reconcile(&candidates, &pending, 4, 10).unwrap();
        assert_eq!(
            queue::read_pending_remember(&pending)[0].state,
            PendingState::Uncertain
        );
    }

    #[test]
    fn completion_marker_prevents_queue_projection_after_restart() {
        let root = tempfile::tempdir().unwrap();
        let candidates = root.path().join("candidates");
        let pending = root.path().join("pending");
        let r = receipt();
        commit(&receipt_dir(&candidates), &r).unwrap();
        mark_completed(&receipt_dir(&candidates), &r.operation.operation_id).unwrap();
        reconcile(&candidates, &pending, 4, 10).unwrap();
        assert!(queue::read_pending_remember(&pending).is_empty());
    }

    #[test]
    fn legacy_candidate_json_without_receipt_fields_still_loads() {
        let value = serde_json::json!({"id":"legacy","category":"lesson","text":"x","source":{"entryId":"e","role":"user"},"dataset":"d","createdAt":1,"state":"accepted"});
        let parsed: Candidate = serde_json::from_value(value).unwrap();
        assert_eq!(parsed.admission_id, None);
        assert_eq!(parsed.source_scope, None);
    }
}
