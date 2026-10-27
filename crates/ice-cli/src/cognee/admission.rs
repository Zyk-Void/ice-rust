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
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "json") {
            if let Ok(text) = std::fs::read_to_string(path) {
                if let Ok(r) = serde_json::from_str::<AdmissionReceipt>(&text) {
                    if r.version == RECEIPT_VERSION && receipt_path(dir, &r.id).is_ok() {
                        out.push(r);
                    }
                }
            }
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
    let path = receipt_path(dir, &receipt.id)?;
    create_private_dir(dir)?;
    if !path.exists() {
        write_private_file_atomic(
            &path,
            &serde_json::to_string(receipt).map_err(std::io::Error::other)?,
        )?;
    }
    Ok(())
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
    dir.join(format!("completed-{operation_id}.json")).is_file()
}
/// Restore the candidate/outbox projections from committed receipts. Prepared
/// operations with no valid receipt are deliberately left blocked.
pub fn reconcile(
    candidates_dir: &Path,
    pending_dir: &Path,
    queue_limit: usize,
    now: u64,
) -> std::io::Result<usize> {
    let mut projected = 0;
    let receipts_dir = receipt_dir(candidates_dir);
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
mod tests {
    use super::*;
    use crate::cognee::candidates::{Category, Role, Source};

    fn receipt() -> AdmissionReceipt {
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
