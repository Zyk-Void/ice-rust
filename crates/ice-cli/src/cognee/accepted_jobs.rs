//! Bounded private acknowledgments of backend acceptance, never proof of indexing.
use super::client::AcceptedReceipt;
use super::config::{create_private_dir, write_private_file_atomic};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const MAX_RECORDS: usize = 4096;
const MAX_RECORD_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AcceptedJob {
    pub version: u8,
    pub operation_id: String,
    pub operation: String,
    pub dataset: String,
    pub session_id: Option<String>,
    pub accepted_at_ms: u64,
    pub receipt: AcceptedReceipt,
    pub searchability: Searchability,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Searchability {
    Unverified,
}

fn path(directory: &Path, id: &str) -> std::io::Result<PathBuf> {
    if id.is_empty()
        || id.len() > 100
        || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "invalid acceptance operation id",
        ));
    }
    Ok(directory.join(format!("{id}.json")))
}

pub(crate) fn persist(directory: &Path, record: &AcceptedJob) -> std::io::Result<()> {
    let file = path(directory, &record.operation_id)?;
    if record.version != 1 || !matches!(record.operation.as_str(), "remember" | "improve") {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "invalid acceptance record",
        ));
    }
    let body = serde_json::to_string(record).map_err(std::io::Error::other)?;
    if body.len() > MAX_RECORD_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "acceptance record too large",
        ));
    }
    create_private_dir(directory)?;
    // A stable advisory lock makes the cap and no-overwrite rule process-safe.
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(directory.join(".accepted.lock"))?;
    fs2::FileExt::lock_exclusive(&lock)?;
    if file.exists() {
        if std::fs::read(&file)? == body.as_bytes() {
            return Ok(());
        }
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "conflicting acceptance record",
        ));
    }
    if std::fs::read_dir(directory)?
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .take(MAX_RECORDS)
        .count()
        >= MAX_RECORDS
    {
        return Err(std::io::Error::other("acceptance ledger is full"));
    }
    write_private_file_atomic(&file, &body)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn acceptance_receipt_survives_restart_without_claiming_searchability() {
        let dir = tempfile::tempdir().unwrap();
        let record = AcceptedJob {
            version: 1,
            operation_id: "op-1".into(),
            operation: "improve".into(),
            dataset: "original".into(),
            session_id: Some("session".into()),
            accepted_at_ms: 7,
            receipt: AcceptedReceipt {
                job_id: Some("job-42".into()),
            },
            searchability: Searchability::Unverified,
        };
        persist(dir.path(), &record).unwrap();
        persist(dir.path(), &record).unwrap();
        let restored: AcceptedJob =
            serde_json::from_slice(&std::fs::read(path(dir.path(), "op-1").unwrap()).unwrap())
                .unwrap();
        assert_eq!(restored, record);
        let mut conflict = record.clone();
        conflict.dataset = "new".into();
        assert_eq!(
            persist(dir.path(), &conflict).unwrap_err().kind(),
            std::io::ErrorKind::AlreadyExists
        );
        let mut invalid = record;
        invalid.operation_id = "../escape".into();
        assert_eq!(
            persist(dir.path(), &invalid).unwrap_err().kind(),
            std::io::ErrorKind::InvalidInput
        );
    }
}
