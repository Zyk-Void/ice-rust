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
