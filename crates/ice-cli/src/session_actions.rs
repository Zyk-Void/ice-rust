//! Exact-file operations for the session picker.
use crate::session_catalog::{canonical, SessionRow};
use ice_harness::session::jsonl::{parse_header, parse_mutation, JsonlSessionStorage};
use ice_harness::session::memory::SystemClock;
use ice_harness::session::types::SessionStorage;
use ice_harness::session::DefaultIdGenerator;
use ice_tools::OsExecutionEnv;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::sync::Arc;

#[derive(Clone)]
pub(crate) enum SessionAction {
    Rename(String),
    Delete,
}

/// Apply an already-confirmed action. Renaming the active session uses its live
/// storage; other files need the same sidecar lock used by CLI session writers.
pub(crate) async fn perform(
    row: &SessionRow,
    action: &SessionAction,
    active: Option<&Arc<dyn SessionStorage>>,
) -> Result<(), String> {
    let path = row.path.canonicalize().map_err(|error| error.to_string())?;
    if path != row.path || !path.is_file() {
        return Err("The selected session path changed; reopen the picker.".into());
    }
    let is_active = active
        .and_then(|storage| storage.persistence_path())
        .map(|active_path| canonical(Path::new(&active_path)) == path)
        .unwrap_or(false);
    if is_active {
        let storage = active.unwrap();
        if storage.metadata().id != row.id {
            return Err("The active session changed; reopen the picker.".into());
        }
        return match action {
            SessionAction::Delete => Err(
                "The active session cannot be deleted. Start or resume another session first."
                    .into(),
            ),
            SessionAction::Rename(name) => storage
                .set_name(valid_name(name)?)
                .await
                .map_err(|error| error.to_string()),
        };
    }

    match action {
        SessionAction::Delete => {
            // Rename opens the session through the storage layer, which takes
            // this same lock; only a delete needs the sidecar lock directly.
            let mut lock_name = path.as_os_str().to_os_string();
            lock_name.push(".lock");
            let lock = std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .open(std::path::PathBuf::from(lock_name))
                .map_err(|error| format!("Could not open the session lock: {error}"))?;
            fs2::FileExt::try_lock_exclusive(&lock)
            .map_err(|_| "This session is open in an icers process. Close that session's process before changing it here.".to_string())?;
            validate_snapshot(row)?;
            std::fs::remove_file(&path).map_err(|error| error.to_string())?;
            // Keep the sidecar inode: removing it could let a new writer lock a
            // second inode while another process still uses the original.
            drop(lock);
        }
        SessionAction::Rename(name) => {
            let name = valid_name(name)?;
            validate_snapshot(row)?;
            // The normal loader may repair torn or duplicate records. A name
            // edit must not silently rewrite history, so reject such files first.
            validate_log_for_rename(&path)?;
            let storage = JsonlSessionStorage::load(
                Arc::new(OsExecutionEnv::with_cwd(row.cwd.clone())),
                path.to_str()
                    .ok_or("The session path is not valid UTF-8.")?,
                Arc::new(SystemClock),
                Arc::new(DefaultIdGenerator::new()),
            )
            .await
            .map_err(|error| error.to_string())?;
            if storage.jsonl_metadata().id != row.id {
                return Err("The selected session identity changed; reopen the picker.".into());
            }
            storage
                .set_name(name)
                .await
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

fn valid_name(name: &str) -> Result<Option<&str>, String> {
    let name = name.trim();
    if name.chars().any(char::is_control) {
        return Err("Use a single-line session name without control characters.".into());
    }
    Ok((!name.is_empty()).then_some(name))
}

fn validate_snapshot(row: &SessionRow) -> Result<(), String> {
    let metadata = std::fs::metadata(&row.path).map_err(|error| error.to_string())?;
    if metadata.len() != row.bytes
        || metadata.modified().ok() != Some(row.modified)
        || row.path.canonicalize().ok().as_ref() != Some(&row.path)
    {
        return Err("The selected session changed; reopen the picker before changing it.".into());
    }
    let file = std::fs::File::open(&row.path).map_err(|error| error.to_string())?;
    let mut header = String::new();
    BufReader::new(std::io::Read::take(file, 64 * 1024))
        .read_line(&mut header)
        .map_err(|error| error.to_string())?;
    let header: serde_json::Value =
        serde_json::from_str(&header).map_err(|error| error.to_string())?;
    if header["id"].as_str() != Some(&row.id)
        || header["cwd"].as_str().map(|cwd| canonical(Path::new(cwd))) != Some(row.cwd.clone())
    {
        return Err("The selected session identity changed; reopen the picker.".into());
    }
    Ok(())
}

fn validate_log_for_rename(path: &Path) -> Result<(), String> {
    let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .map_err(|error| error.to_string())?;
    parse_header(line.trim_end())
        .map_err(|error| format!("Cannot rename this session: {error}"))?;
    let mut sequence = 0;
    loop {
        line.clear();
        if reader
            .read_line(&mut line)
            .map_err(|error| error.to_string())?
            == 0
        {
            return Ok(());
        }
        if !line.ends_with('\n') {
            return Err("Resume and recover this incomplete session before renaming it.".into());
        }
        let mutation = parse_mutation(line.trim_end())
            .map_err(|error| format!("Cannot rename a damaged session: {error}"))?;
        if mutation.seq() <= sequence {
            return Err(
                "Resume and recover this session's duplicate records before renaming it.".into(),
            );
        }
        sequence = mutation.seq();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ice_harness::session::jsonl::{
        JsonlSessionCreateOptions, JsonlSessionRepo, JsonlSessionRepoOptions,
    };
    use std::io::Write;

    async fn fixture(root: &Path) -> (Arc<dyn SessionStorage>, SessionRow) {
        std::fs::create_dir_all(root).unwrap();
        let root = canonical(root);
        let repo = JsonlSessionRepo::with_env_cwd(JsonlSessionRepoOptions {
            fs: Arc::new(OsExecutionEnv::with_cwd(root.clone())),
            sessions_root: root.join("sessions").to_string_lossy().into_owned(),
            clock: Arc::new(SystemClock),
            ids: Arc::new(DefaultIdGenerator::new()),
        });
        let storage = Arc::new(
            repo.create_typed(&JsonlSessionCreateOptions {
                id: Some("same-id".into()),
                parent_session_id: None,
                cwd: root.to_string_lossy().into_owned(),
                metadata: None,
            })
            .await
            .unwrap(),
        );
        storage.set_name(Some("original")).await.unwrap();
        let path = storage.jsonl_metadata().path;
        let row = crate::session_catalog::load_rows(&[Path::new(&path).parent().unwrap().into()])
            .pop()
            .unwrap();
        (storage, row)
    }

    fn refresh(row: &SessionRow) -> SessionRow {
        crate::session_catalog::load_rows(&[row.path.parent().unwrap().into()])
            .into_iter()
            .find(|candidate| candidate.path == row.path)
            .unwrap()
    }

    #[tokio::test]
    async fn rename_appends_a_name_fact_and_delete_uses_exact_path_with_duplicate_ids() {
        let temp = tempfile::tempdir().unwrap();
        let (_, first) = fixture(&temp.path().join("first")).await;
        let (_, second) = fixture(&temp.path().join("second")).await;
        let first_bytes = std::fs::read(&first.path).unwrap();
        let second_bytes = std::fs::read(&second.path).unwrap();
        perform(&first, &SessionAction::Rename("東京 project".into()), None)
            .await
            .unwrap();
        let renamed = std::fs::read(&first.path).unwrap();
        assert!(renamed.starts_with(&first_bytes));
        assert_eq!(
            crate::session::summarize_session_file(&first.path)
                .name
                .as_deref(),
            Some("東京 project")
        );
        assert_eq!(std::fs::read(&second.path).unwrap(), second_bytes);
        perform(&refresh(&first), &SessionAction::Delete, None)
            .await
            .unwrap();
        assert!(!first.path.exists());
        assert_eq!(std::fs::read(&second.path).unwrap(), second_bytes);
    }

    #[tokio::test]
    async fn active_rename_uses_live_sequence_and_active_delete_is_refused() {
        let temp = tempfile::tempdir().unwrap();
        let (active, row) = fixture(temp.path()).await;
        perform(
            &row,
            &SessionAction::Rename("changed".into()),
            Some(&active),
        )
        .await
        .unwrap();
        assert_eq!(active.get_name().await.unwrap().as_deref(), Some("changed"));
        active.set_name(Some("after picker")).await.unwrap();
        assert_eq!(
            crate::session::summarize_session_file(&row.path)
                .name
                .as_deref(),
            Some("after picker")
        );
        let bytes = std::fs::read(&row.path).unwrap();
        assert!(perform(&row, &SessionAction::Delete, Some(&active))
            .await
            .is_err());
        assert_eq!(std::fs::read(&row.path).unwrap(), bytes);
    }

    #[tokio::test]
    async fn changed_or_locked_files_are_not_modified() {
        let temp = tempfile::tempdir().unwrap();
        let (storage, row) = fixture(temp.path()).await;
        // The fixture's live storage handle owns the session's writer lock.
        let bytes = std::fs::read(&row.path).unwrap();
        assert!(perform(&row, &SessionAction::Delete, None)
            .await
            .unwrap_err()
            .contains("open in"));
        assert_eq!(std::fs::read(&row.path).unwrap(), bytes);
        storage.set_name(Some("external change")).await.unwrap();
        drop(storage);
        assert!(perform(&row, &SessionAction::Delete, None)
            .await
            .unwrap_err()
            .contains("changed"));
        assert!(row.path.exists());
    }

    #[tokio::test]
    async fn damaged_logs_are_not_repaired_by_a_failed_rename() {
        let temp = tempfile::tempdir().unwrap();
        let (_, row) = fixture(temp.path()).await;
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&row.path)
            .unwrap();
        file.write_all(b"{unfinished").unwrap();
        drop(file);
        let row = refresh(&row);
        let bytes = std::fs::read(&row.path).unwrap();
        assert!(perform(&row, &SessionAction::Rename("new".into()), None)
            .await
            .is_err());
        assert_eq!(std::fs::read(&row.path).unwrap(), bytes);
    }
}
