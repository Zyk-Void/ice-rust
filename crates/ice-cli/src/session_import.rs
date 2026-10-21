//! Safe import of native v4 JSONL sessions.
//!
//! The interactive TUI currently owns activation and import orchestration. This
//! module is the persistence boundary: it validates a staged copy before
//! making a new session file visible, and never replaces an existing path.

use std::path::{Path, PathBuf};

use ice_harness::session::jsonl::{parse_header, parse_mutation};
use ice_harness::session::state::SessionState;
use thiserror::Error;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader};
use uuid::Uuid;

const MAX_HEADER_LINE_BYTES: usize = 64 * 1024;
const MAX_MUTATION_LINE_BYTES: usize = 16 * 1024 * 1024;
// Leaves room for the prefix and extension on filesystems with the common
// 255-byte component limit.
const MAX_ENCODED_ID_BYTES: usize = 225;
const DESTINATION_PREFIX: &str = "import-";
const DESTINATION_SUFFIX: &str = ".jsonl";

#[derive(Debug, Error)]
enum ImportError {
    #[error("import source is not a regular file: {0}")]
    NotAFile(PathBuf),
    #[error("could not {action}: {source}")]
    Io {
        action: &'static str,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid native session at line {line}: {reason}")]
    InvalidSession { line: u64, reason: String },
    #[error(
        "session version 3 is not supported by native import; export or migrate it to v4 first"
    )]
    UnsupportedV3,
    #[error("session id is too long to safely encode as a session filename")]
    IdTooLong,
    #[error("session id {id:?} is already present in {directory}")]
    DuplicateId { id: String, directory: PathBuf },
    #[error(
        "filesystem does not support atomic hard-link publication; session was not installed: {0}"
    )]
    HardLinksUnsupported(PathBuf),
    #[error("session directory is not a real directory (symlinks are not accepted): {0}")]
    UnsafeDirectory(PathBuf),
}

fn io_error(action: &'static str, source: std::io::Error) -> ImportError {
    ImportError::Io { action, source }
}

/// Validate and install a native v4 JSONL file in the configured session
/// directory. Returns the session ID from the JSONL header; installation does
/// not activate or switch the current session.
///
/// Validation is performed against a private staging copy using the native
/// codec and state reducer. Unlike `JsonlSessionStorage::load`, this import
/// path does not repair torn tails, discard duplicate sequences, or rewrite
/// bytes while validating. It bounds each line while streaming so a large log
/// need not be read into one string.
pub(crate) async fn import_session_file(source: &Path, cwd: &Path) -> Result<String, String> {
    import_session_file_inner(source, cwd)
        .await
        .map_err(|error| error.to_string())
}

async fn import_session_file_inner(source: &Path, cwd: &Path) -> Result<String, ImportError> {
    let source_metadata = tokio::fs::metadata(source)
        .await
        .map_err(|error| io_error("inspect the import source", error))?;
    if !source_metadata.is_file() {
        return Err(ImportError::NotAFile(source.to_path_buf()));
    }

    let directory = crate::session::session_dir(cwd);
    tokio::fs::create_dir_all(&directory)
        .await
        .map_err(|error| io_error("create the session directory", error))?;
    let directory_metadata = tokio::fs::symlink_metadata(&directory)
        .await
        .map_err(|error| io_error("inspect the session directory", error))?;
    if !directory_metadata.file_type().is_dir() {
        return Err(ImportError::UnsafeDirectory(directory));
    }

    let staging_path = stage_source(source, &directory).await?;
    // Every ordinary failure after staging removes only this invocation's
    // unique staging file. The final hard link is exclusive and never clobbers
    // a file, directory, or symlink already at its destination.
    let result = validate_and_publish(&staging_path, &directory).await;
    if let Err(cleanup_error) = tokio::fs::remove_file(&staging_path).await {
        if result.is_ok() {
            // The destination is already safely installed. Do not report a
            // failed import (and encourage a retry) after the publish point.
            tracing::warn!(path = %staging_path.display(), error = %cleanup_error, "could not remove session import staging link");
        }
    }
    result
}

async fn stage_source(source: &Path, directory: &Path) -> Result<PathBuf, ImportError> {
    let (path, mut target) = loop {
        let path = directory.join(format!(".icers-import-{}.tmp", Uuid::new_v4()));
        match tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .await
        {
            Ok(file) => break (path, file),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(io_error("create a private staging file", error)),
        }
    };

    let copy_result = async {
        let mut input = tokio::fs::File::open(source)
            .await
            .map_err(|error| io_error("open the import source", error))?;
        tokio::io::copy(&mut input, &mut target)
            .await
            .map_err(|error| io_error("copy the import source to staging", error))?;
        target
            .flush()
            .await
            .map_err(|error| io_error("flush the staged session", error))?;
        target
            .sync_all()
            .await
            .map_err(|error| io_error("sync the staged session", error))?;
        Ok::<(), ImportError>(())
    }
    .await;
    drop(target);

    if let Err(error) = copy_result {
        let _ = tokio::fs::remove_file(&path).await;
        return Err(error);
    }
    Ok(path)
}

async fn validate_and_publish(
    staging_path: &Path,
    directory: &Path,
) -> Result<String, ImportError> {
    let header = validate_staged_session(staging_path).await?;
    let encoded_id = encode_session_id(&header.id)?;
    if encoded_id.len() > MAX_ENCODED_ID_BYTES {
        return Err(ImportError::IdTooLong);
    }

    // Deterministic names make duplicate imports collide even when callers use
    // different source basenames. Scan existing JSONL headers too, so older or
    // externally-created filenames cannot introduce the same session ID.
    if has_session_id(directory, &header.id).await? {
        return Err(ImportError::DuplicateId {
            id: header.id,
            directory: directory.to_path_buf(),
        });
    }

    let destination = directory.join(format!(
        "{DESTINATION_PREFIX}{encoded_id}{DESTINATION_SUFFIX}"
    ));
    match tokio::fs::hard_link(staging_path, &destination).await {
        Ok(()) => Ok(header.id),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            Err(ImportError::DuplicateId {
                id: header.id,
                directory: directory.to_path_buf(),
            })
        }
        Err(error) if error.kind() == std::io::ErrorKind::Unsupported => {
            Err(ImportError::HardLinksUnsupported(destination))
        }
        Err(error) => Err(io_error("publish the validated session exclusively", error)),
    }
}

async fn validate_staged_session(
    staging_path: &Path,
) -> Result<ice_harness::session::jsonl::JsonlV4Header, ImportError> {
    let file = tokio::fs::File::open(staging_path)
        .await
        .map_err(|error| io_error("open the staged session for validation", error))?;
    let mut reader = BufReader::new(file);
    let mut line_number = 0_u64;
    let header_line = read_bounded_line(&mut reader, MAX_HEADER_LINE_BYTES, line_number + 1)
        .await?
        .ok_or_else(|| ImportError::InvalidSession {
            line: 1,
            reason: "session is empty and has no header".to_string(),
        })?;
    line_number += 1;
    let header_text = decode_line(header_line, line_number)?;

    // Give the unsupported legacy format a migration-oriented message rather
    // than the codec's generic unsupported-version schema diagnostic.
    if serde_json::from_str::<serde_json::Value>(&header_text)
        .ok()
        .and_then(|value| value.get("version").and_then(serde_json::Value::as_i64))
        == Some(3)
    {
        return Err(ImportError::UnsupportedV3);
    }
    let header = parse_header(&header_text).map_err(|error| ImportError::InvalidSession {
        line: line_number,
        reason: error.to_string(),
    })?;

    let mut state = SessionState::new();
    while let Some(bytes) =
        read_bounded_line(&mut reader, MAX_MUTATION_LINE_BYTES, line_number + 1).await?
    {
        line_number += 1;
        let text = decode_line(bytes, line_number)?;
        let mutation = parse_mutation(&text).map_err(|error| ImportError::InvalidSession {
            line: line_number,
            reason: error.to_string(),
        })?;
        state
            .apply_mutation(mutation)
            .map_err(|error| ImportError::InvalidSession {
                line: line_number,
                reason: error.to_string(),
            })?;
    }

    Ok(header)
}

fn decode_line(bytes: Vec<u8>, line: u64) -> Result<String, ImportError> {
    String::from_utf8(bytes).map_err(|error| ImportError::InvalidSession {
        line,
        reason: format!("line is not UTF-8: {error}"),
    })
}

/// Read one physical line without allowing an unbounded allocation. A final
/// unterminated line is returned as-is; its JSON still has to pass native
/// parsing and the staged/source bytes are never rewritten.
async fn read_bounded_line<R>(
    reader: &mut R,
    limit: usize,
    line_number: u64,
) -> Result<Option<Vec<u8>>, ImportError>
where
    R: AsyncBufRead + Unpin,
{
    let mut line = Vec::new();
    loop {
        let available = reader
            .fill_buf()
            .await
            .map_err(|error| io_error("read the staged session", error))?;
        if available.is_empty() {
            return if line.is_empty() {
                Ok(None)
            } else {
                Ok(Some(line))
            };
        }

        let newline = available.iter().position(|byte| *byte == b'\n');
        let bytes_to_take = newline.unwrap_or(available.len());
        if line.len().saturating_add(bytes_to_take) > limit {
            return Err(ImportError::InvalidSession {
                line: line_number,
                reason: format!("line exceeds the {limit}-byte import limit"),
            });
        }
        line.extend_from_slice(&available[..bytes_to_take]);
        let consumed = bytes_to_take + if newline.is_some() { 1 } else { 0 };
        reader.consume(consumed);
        if newline.is_some() {
            return Ok(Some(line));
        }
    }
}

async fn has_session_id(directory: &Path, id: &str) -> Result<bool, ImportError> {
    let mut entries = tokio::fs::read_dir(directory)
        .await
        .map_err(|error| io_error("scan existing session IDs", error))?;
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|error| io_error("scan existing session IDs", error))?
    {
        if entry
            .path()
            .extension()
            .and_then(|extension| extension.to_str())
            != Some("jsonl")
        {
            continue;
        }
        let file_type = entry
            .file_type()
            .await
            .map_err(|error| io_error("inspect an existing session", error))?;
        if !file_type.is_file() {
            continue;
        }
        let file = match tokio::fs::File::open(entry.path()).await {
            Ok(file) => file,
            // A file can disappear between readdir and open; that does not
            // create a duplicate. Other errors are surfaced rather than
            // silently weakening the uniqueness check.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(io_error("read an existing session header", error)),
        };
        let mut reader = BufReader::new(file);
        let Some(bytes) = read_bounded_line(&mut reader, MAX_HEADER_LINE_BYTES, 1).await? else {
            continue;
        };
        let Ok(line) = String::from_utf8(bytes) else {
            continue;
        };
        if parse_header(&line)
            .map(|header| header.id == id)
            .unwrap_or(false)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Percent-encode UTF-8 bytes rather than Unicode scalar values. Only filename
/// characters safe on Windows and POSIX are left literal; encoding `%` itself
/// makes this mapping injective.
fn encode_session_id(id: &str) -> Result<String, ImportError> {
    let mut encoded = String::with_capacity(id.len());
    for byte in id.bytes() {
        match byte {
            // Escape uppercase ASCII too: paths are case-insensitive on
            // Windows, so leaving it literal would alias a lowercase ID.
            b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => encoded.push(byte as char),
            _ => {
                use std::fmt::Write as _;
                write!(&mut encoded, "%{byte:02X}").expect("writing to String cannot fail");
            }
        }
    }
    if encoded.len() > MAX_ENCODED_ID_BYTES {
        return Err(ImportError::IdTooLong);
    }
    Ok(encoded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ice_harness::session::jsonl::{encode_header, encode_mutation, JsonlSessionStorage};
    use ice_harness::session::memory::SystemClock;
    use ice_harness::session::session::DefaultIdGenerator;
    use ice_harness::session::types::{
        CustomEntry, Entry, EntryBase, EntryQuery, SessionMutation, SessionStorage,
    };
    use ice_tools::{FileSystem, OsExecutionEnv};
    use std::sync::Arc;

    fn v4_header(id: &str) -> String {
        encode_header(&ice_harness::session::jsonl::JsonlV4Header::new(
            id.to_string(),
            1_700_000_000_000,
            "/test".to_string(),
            None,
            None,
            None,
        ))
    }

    fn destination(cwd: &Path, id: &str) -> PathBuf {
        crate::session::default_session_dir(cwd).join(format!(
            "{DESTINATION_PREFIX}{}{DESTINATION_SUFFIX}",
            encode_session_id(id).unwrap()
        ))
    }

    #[test]
    fn filename_encoding_is_safe_in_case_insensitive_directories() {
        assert_eq!(encode_session_id("A").unwrap(), "%41");
        assert_eq!(encode_session_id("a").unwrap(), "a");
        assert_ne!(
            encode_session_id("A").unwrap(),
            encode_session_id("a").unwrap()
        );
        assert_eq!(encode_session_id("../x").unwrap(), "..%2Fx");
    }

    #[tokio::test]
    async fn imports_header_id_not_source_basename_and_preserves_source_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let cwd = temp.path().join("project");
        tokio::fs::create_dir_all(&cwd).await.unwrap();
        let source = temp.path().join("arbitrary-name.jsonl");
        let original = v4_header("actual-session-id").into_bytes();
        tokio::fs::write(&source, &original).await.unwrap();

        let imported = import_session_file(&source, &cwd).await.unwrap();

        assert_eq!(imported, "actual-session-id");
        assert_eq!(tokio::fs::read(&source).await.unwrap(), original);
        assert_eq!(
            tokio::fs::read(destination(&cwd, &imported)).await.unwrap(),
            original
        );
    }

    #[tokio::test]
    async fn valid_mutations_import_and_reopen_through_native_storage() {
        let temp = tempfile::tempdir().unwrap();
        let cwd = temp.path().join("project");
        tokio::fs::create_dir_all(&cwd).await.unwrap();
        let source = temp.path().join("valid-source.jsonl");
        let mutation = SessionMutation::Entry {
            seq: 1,
            timestamp: 1_700_000_000_001,
            lane: Some("main".to_string()),
            entry: Entry::Custom(CustomEntry {
                base: EntryBase {
                    entry_type: "custom".to_string(),
                    id: "entry-1".to_string(),
                    seq: 1,
                    parent_id: None,
                    timestamp: 1_700_000_000_001,
                },
                custom_type: "test".to_string(),
                data: None,
            }),
        };
        let original = format!(
            "{}{}",
            v4_header("valid-session"),
            encode_mutation(&mutation)
        );
        tokio::fs::write(&source, &original).await.unwrap();

        assert_eq!(
            import_session_file(&source, &cwd).await.unwrap(),
            "valid-session"
        );
        assert_eq!(tokio::fs::read(&source).await.unwrap(), original.as_bytes());

        let imported_path = destination(&cwd, "valid-session");
        let env = Arc::new(OsExecutionEnv::with_cwd(cwd));
        let fs: Arc<dyn FileSystem> = env;
        let storage = JsonlSessionStorage::load(
            fs,
            &imported_path.to_string_lossy(),
            Arc::new(SystemClock),
            Arc::new(DefaultIdGenerator::new()),
        )
        .await
        .unwrap();
        assert_eq!(storage.metadata().id, "valid-session");
        let entries = storage.find_entries(&EntryQuery::default()).await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id(), "entry-1");
    }

    #[tokio::test]
    async fn duplicate_id_from_different_source_names_does_not_change_destination() {
        let temp = tempfile::tempdir().unwrap();
        let cwd = temp.path().join("project");
        tokio::fs::create_dir_all(&cwd).await.unwrap();
        let first_source = temp.path().join("first.jsonl");
        let second_source = temp.path().join("second.jsonl");
        tokio::fs::write(&first_source, v4_header("shared-id"))
            .await
            .unwrap();
        tokio::fs::write(&second_source, v4_header("shared-id"))
            .await
            .unwrap();
        import_session_file(&first_source, &cwd).await.unwrap();
        let target = destination(&cwd, "shared-id");
        let before = tokio::fs::read(&target).await.unwrap();

        let result = import_session_file(&second_source, &cwd).await;

        assert!(result.unwrap_err().contains("already present"));
        assert_eq!(tokio::fs::read(&target).await.unwrap(), before);
        assert_eq!(
            tokio::fs::read(&second_source).await.unwrap(),
            v4_header("shared-id").as_bytes()
        );
    }

    #[tokio::test]
    async fn malformed_mutation_is_rejected_without_installing_a_session() {
        let temp = tempfile::tempdir().unwrap();
        let cwd = temp.path().join("project");
        tokio::fs::create_dir_all(&cwd).await.unwrap();
        let source = temp.path().join("malformed.jsonl");
        let bytes = format!("{}{{not-json}}\n", v4_header("malformed-id"));
        tokio::fs::write(&source, bytes).await.unwrap();

        assert!(import_session_file(&source, &cwd).await.is_err());
        let mut entries = tokio::fs::read_dir(crate::session::default_session_dir(&cwd))
            .await
            .unwrap();
        assert!(entries.next_entry().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn invalid_mutation_sequence_is_rejected_before_publish() {
        let temp = tempfile::tempdir().unwrap();
        let cwd = temp.path().join("project");
        tokio::fs::create_dir_all(&cwd).await.unwrap();
        let source = temp.path().join("bad-sequence.jsonl");
        let bytes = format!(
            "{}{{\"kind\":\"lane\",\"seq\":2,\"lane\":\"main\",\"leafId\":null}}\n",
            v4_header("bad-sequence")
        );
        tokio::fs::write(&source, bytes).await.unwrap();

        assert!(import_session_file(&source, &cwd).await.is_err());
        let mut entries = tokio::fs::read_dir(crate::session::default_session_dir(&cwd))
            .await
            .unwrap();
        assert!(entries.next_entry().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn pathlike_header_id_cannot_escape_the_session_directory() {
        let temp = tempfile::tempdir().unwrap();
        let cwd = temp.path().join("project");
        tokio::fs::create_dir_all(&cwd).await.unwrap();
        let source = temp.path().join("pathlike.jsonl");
        let id = "../../outside/unsafe";
        tokio::fs::write(&source, v4_header(id)).await.unwrap();

        assert_eq!(import_session_file(&source, &cwd).await.unwrap(), id);
        let target = destination(&cwd, id);
        assert!(target.starts_with(crate::session::default_session_dir(&cwd)));
        assert!(tokio::fs::metadata(target).await.unwrap().is_file());
        assert!(!temp.path().join("outside/unsafe.jsonl").exists());
    }

    #[tokio::test]
    async fn concurrent_duplicate_imports_only_publish_once() {
        let temp = tempfile::tempdir().unwrap();
        let cwd = temp.path().join("project");
        tokio::fs::create_dir_all(&cwd).await.unwrap();
        let first = temp.path().join("first.jsonl");
        let second = temp.path().join("second.jsonl");
        tokio::fs::write(&first, v4_header("racing-id"))
            .await
            .unwrap();
        tokio::fs::write(&second, v4_header("racing-id"))
            .await
            .unwrap();

        let (a, b) = tokio::join!(
            import_session_file(&first, &cwd),
            import_session_file(&second, &cwd)
        );

        assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
        assert!(tokio::fs::metadata(destination(&cwd, "racing-id"))
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn source_equal_to_destination_is_never_truncated() {
        let temp = tempfile::tempdir().unwrap();
        let cwd = temp.path().join("project");
        let directory = crate::session::default_session_dir(&cwd);
        tokio::fs::create_dir_all(&directory).await.unwrap();
        let source = destination(&cwd, "same-path");
        let original = v4_header("same-path").into_bytes();
        tokio::fs::write(&source, &original).await.unwrap();

        assert!(import_session_file(&source, &cwd).await.is_err());
        assert_eq!(tokio::fs::read(&source).await.unwrap(), original);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn destination_symlink_is_not_replaced() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let cwd = temp.path().join("project");
        let directory = crate::session::default_session_dir(&cwd);
        tokio::fs::create_dir_all(&directory).await.unwrap();
        let source = temp.path().join("symlink-source.jsonl");
        tokio::fs::write(&source, v4_header("symlink-id"))
            .await
            .unwrap();
        let outside = temp.path().join("outside.jsonl");
        tokio::fs::write(&outside, "must not be modified")
            .await
            .unwrap();
        let target = destination(&cwd, "symlink-id");
        symlink(&outside, &target).unwrap();

        assert!(import_session_file(&source, &cwd).await.is_err());
        assert!(tokio::fs::symlink_metadata(&target)
            .await
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(
            tokio::fs::read(&outside).await.unwrap(),
            b"must not be modified"
        );
    }

    #[tokio::test]
    async fn v3_file_has_an_actionable_rejection() {
        let temp = tempfile::tempdir().unwrap();
        let cwd = temp.path().join("project");
        tokio::fs::create_dir_all(&cwd).await.unwrap();
        let source = temp.path().join("legacy.jsonl");
        tokio::fs::write(
            &source,
            "{\"kind\":\"header\",\"version\":3,\"id\":\"legacy\",\"createdAt\":1,\"cwd\":\"/test\"}\n",
        )
        .await
        .unwrap();

        assert!(import_session_file(&source, &cwd)
            .await
            .unwrap_err()
            .contains("migrate it to v4"));
    }
}
