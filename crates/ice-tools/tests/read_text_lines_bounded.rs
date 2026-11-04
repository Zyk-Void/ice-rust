//! `read_text_lines(path, Some(n))` must stop after `n` lines instead of
//! loading the whole file: session discovery asks for the header line of every
//! saved session.

use ice_tools::{FileSystem, OsExecutionEnv};

#[tokio::test]
async fn bounded_read_does_not_touch_bytes_past_the_requested_lines() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.jsonl");
    // Line 3 is invalid UTF-8; a whole-file read would fail on it.
    let mut bytes = b"header\nsecond\n".to_vec();
    bytes.extend_from_slice(&[0xff, 0xfe, b'\n']);
    std::fs::write(&path, bytes).unwrap();
    let env = OsExecutionEnv::with_cwd(dir.path().to_path_buf());

    let lines = env
        .read_text_lines(&path.to_string_lossy(), Some(2), None)
        .await
        .unwrap();
    assert_eq!(lines, vec!["header".to_string(), "second".to_string()]);

    let error = env
        .read_text_lines(&path.to_string_lossy(), Some(3), None)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("utf-8"), "{error}");
}

#[tokio::test]
async fn bounded_read_rejects_a_single_oversized_line() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("huge.jsonl");
    std::fs::write(&path, vec![b'a'; 2 * 1024 * 1024]).unwrap();
    let env = OsExecutionEnv::with_cwd(dir.path().to_path_buf());

    let error = env
        .read_text_lines(&path.to_string_lossy(), Some(1), None)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("limit"), "{error}");
}

/// An empty file has no lines on every reader. The unbounded readers used to
/// return one empty line, so session discovery reported 0-byte files as
/// malformed while the bounded reader skipped them.
#[tokio::test]
async fn empty_file_has_no_lines_on_every_reader() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("empty.jsonl");
    std::fs::write(&path, b"").unwrap();
    let path = path.to_string_lossy().into_owned();
    let os = OsExecutionEnv::with_cwd(dir.path().to_path_buf());
    assert!(os
        .read_text_lines(&path, Some(1), None)
        .await
        .unwrap()
        .is_empty());
    assert!(os
        .read_text_lines(&path, None, None)
        .await
        .unwrap()
        .is_empty());

    let memory = ice_tools::InMemoryExecutionEnv::with_cwd("/proj".into());
    memory
        .write_file(
            "/proj/empty.jsonl",
            ice_tools::FileContent::Text(String::new()),
            None,
        )
        .await
        .unwrap();
    assert!(memory
        .read_text_lines("/proj/empty.jsonl", None, None)
        .await
        .unwrap()
        .is_empty());
}
