//! Native ownership must cover load/repair and survive process crashes without
//! stale-lock cleanup. All files and child-process state live in a temp directory.
use std::sync::Arc;

use ice_harness::session::jsonl::{JsonlSessionStorage, JsonlV4Header};
use ice_harness::session::memory::{CounterIdGenerator, FakeClock};
use ice_harness::session::types::SessionStorage;
use ice_tools::{FileSystem, OsExecutionEnv};

fn fs(dir: &std::path::Path) -> Arc<dyn FileSystem> {
    Arc::new(OsExecutionEnv::with_cwd(dir.to_path_buf()))
}

async fn create(dir: &std::path::Path) -> JsonlSessionStorage {
    JsonlSessionStorage::create(
        fs(dir),
        "session.jsonl",
        JsonlV4Header::new("owned".into(), 1, "/test".into(), None, None, None),
        Arc::new(FakeClock::new()),
        Arc::new(CounterIdGenerator::new()),
    )
    .await
    .unwrap()
}

async fn load(
    dir: &std::path::Path,
) -> Result<JsonlSessionStorage, ice_harness::error::SessionError> {
    JsonlSessionStorage::load(
        fs(dir),
        "session.jsonl",
        Arc::new(FakeClock::new()),
        Arc::new(CounterIdGenerator::new()),
    )
    .await
}

#[tokio::test]
async fn independent_handles_refused_until_last_shared_handle_drops() {
    let temp = tempfile::tempdir().unwrap();
    let owner = Arc::new(create(temp.path()).await);
    owner.set_name(Some("acknowledged work")).await.unwrap();
    let shared = owner.clone();
    let before = std::fs::read(temp.path().join("session.jsonl")).unwrap();
    assert!(load(temp.path()).await.is_err());
    assert_eq!(
        std::fs::read(temp.path().join("session.jsonl")).unwrap(),
        before
    );
    drop(owner);
    assert!(load(temp.path()).await.is_err());
    drop(shared);
    let reopened = load(temp.path()).await.unwrap();
    assert_eq!(
        reopened.get_name().await.unwrap().as_deref(),
        Some("acknowledged work")
    );
}

#[tokio::test]
async fn create_and_fork_never_replace_existing_history() {
    let temp = tempfile::tempdir().unwrap();
    let owner = create(temp.path()).await;
    owner.set_name(Some("preserve me")).await.unwrap();
    drop(owner);
    let before = std::fs::read(temp.path().join("session.jsonl")).unwrap();
    let result = JsonlSessionStorage::create(
        fs(temp.path()),
        "session.jsonl",
        JsonlV4Header::new("other".into(), 2, "/test".into(), None, None, None),
        Arc::new(FakeClock::new()),
        Arc::new(CounterIdGenerator::new()),
    )
    .await;
    assert!(result.is_err());
    let source = load(temp.path()).await.unwrap();
    let result = source
        .fork(
            "session.jsonl",
            JsonlV4Header::new("fork".into(), 2, "/test".into(), None, None, None),
            &Default::default(),
            Arc::new(FakeClock::new()),
            Arc::new(CounterIdGenerator::new()),
        )
        .await;
    assert!(result.is_err());
    assert_eq!(
        std::fs::read(temp.path().join("session.jsonl")).unwrap(),
        before
    );
}

#[tokio::test]
async fn live_owned_source_can_fork_and_reopen_without_reclaiming_source() {
    use ice_harness::session::jsonl::{
        JsonlSessionCreateOptions, JsonlSessionRepo, JsonlSessionRepoOptions,
    };
    use ice_harness::session::types::{ProvisionedEntry, ProvisionedKind};
    let temp = tempfile::tempdir().unwrap();
    let source = create(temp.path()).await;
    source
        .append_entry(
            ProvisionedEntry {
                id: "message".into(),
                kind: ProvisionedKind::Message {
                    message: ice_agent::AgentMessage::User(ice_ai::types::UserMessage::new(
                        "keep this",
                        1,
                    )),
                    terminate: None,
                },
            },
            "main",
        )
        .await
        .unwrap();
    source.set_name(Some("named source")).await.unwrap();
    source.set_label("message", Some("label")).await.unwrap();
    let repo = JsonlSessionRepo::with_env_cwd(JsonlSessionRepoOptions {
        fs: fs(temp.path()),
        sessions_root: temp.path().join("sessions").to_string_lossy().into_owned(),
        clock: Arc::new(FakeClock::new()),
        ids: Arc::new(CounterIdGenerator::new()),
    });
    let fork = repo
        .fork_from_storage(
            &source,
            &JsonlSessionCreateOptions {
                id: Some("fork".into()),
                parent_session_id: None,
                cwd: temp.path().to_string_lossy().into_owned(),
                metadata: None,
            },
            &Default::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        fork.get_name().await.unwrap().as_deref(),
        Some("named source")
    );
    assert_eq!(
        fork.get_label("message").await.unwrap().as_deref(),
        Some("label")
    );
    assert_eq!(fork.get_stats().await.unwrap(), Default::default());
    let metadata = fork.jsonl_metadata();
    assert!(repo.open_by_jsonl_metadata(&metadata).await.is_err());
    drop(fork);
    let reopened = repo.open_by_jsonl_metadata(&metadata).await.unwrap();
    assert!(reopened.get_entry("message").await.unwrap().is_some());
    assert_eq!(
        reopened.jsonl_metadata().parent_session_id.as_deref(),
        Some("owned")
    );
    assert!(load(temp.path()).await.is_err(), "source retains ownership");
}

#[tokio::test]
async fn discovery_isolates_unreadable_headers_but_reports_root_failure() {
    use ice_harness::session::jsonl::{
        JsonlSessionCreateOptions, JsonlSessionRepo, JsonlSessionRepoOptions,
    };
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("sessions");
    let repo = JsonlSessionRepo::with_env_cwd(JsonlSessionRepoOptions {
        fs: fs(temp.path()),
        sessions_root: root.to_string_lossy().into_owned(),
        clock: Arc::new(FakeClock::new()),
        ids: Arc::new(CounterIdGenerator::new()),
    });
    let healthy = repo
        .create_typed(&JsonlSessionCreateOptions {
            id: Some("healthy".into()),
            parent_session_id: None,
            cwd: temp.path().to_string_lossy().into_owned(),
            metadata: None,
        })
        .await
        .unwrap();
    let metadata = healthy.jsonl_metadata();
    let directory = std::path::Path::new(&metadata.path).parent().unwrap();
    std::fs::write(directory.join("unreadable.jsonl"), [0xff, 0xfe]).unwrap();
    std::fs::write(directory.join("malformed.jsonl"), "not a header\n").unwrap();
    let listed = repo.list_typed(&Default::default()).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, "healthy");
    drop(healthy);
    std::fs::remove_dir_all(&root).unwrap();
    std::fs::write(&root, "not a directory").unwrap();
    assert!(repo.list_typed(&Default::default()).await.is_err());
}

#[tokio::test]
async fn native_constructor_and_delete_cannot_bypass_owned_history() {
    use ice_harness::session::jsonl::{JsonlSessionRepo, JsonlSessionRepoOptions};
    let temp = tempfile::tempdir().unwrap();
    let owner = create(temp.path()).await;
    owner.set_name(Some("saved")).await.unwrap();
    let metadata = owner.jsonl_metadata();
    let before = std::fs::read(temp.path().join("session.jsonl")).unwrap();
    let raw = JsonlSessionStorage::new(
        fs(temp.path()),
        metadata.clone(),
        Arc::new(FakeClock::new()),
        Arc::new(CounterIdGenerator::new()),
    );
    assert!(raw.set_name(Some("overwrite")).await.is_err());
    let repo = JsonlSessionRepo::with_env_cwd(JsonlSessionRepoOptions {
        fs: fs(temp.path()),
        sessions_root: temp.path().to_string_lossy().into_owned(),
        clock: Arc::new(FakeClock::new()),
        ids: Arc::new(CounterIdGenerator::new()),
    });
    assert!(repo.delete_by_jsonl_metadata(&metadata).await.is_err());
    assert_eq!(
        std::fs::read(temp.path().join("session.jsonl")).unwrap(),
        before
    );
    drop(owner);
    repo.delete_by_jsonl_metadata(&metadata).await.unwrap();
    assert!(!temp.path().join("session.jsonl").exists());
}

#[tokio::test]
async fn ownership_child() {
    let Some(dir) = std::env::var_os("ICERS_TEST_OWNERSHIP_DIR") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    if std::env::var_os("ICERS_TEST_EXPECT_BUSY").is_some() {
        assert!(load(&dir).await.is_err());
        return;
    }
    let owner = load(&dir).await.unwrap();
    owner
        .set_name(Some("child acknowledged work"))
        .await
        .unwrap();
    std::fs::write(dir.join("ready"), b"ready").unwrap();
    // Parent kills this process without running Rust destructors.
    tokio::time::sleep(std::time::Duration::from_secs(60)).await;
}

#[tokio::test]
async fn child_process_exclusion_and_crash_release_preserve_acknowledged_work() {
    let temp = tempfile::tempdir().unwrap();
    let owner = create(temp.path()).await;
    let exe = std::env::current_exe().unwrap();
    let status = std::process::Command::new(&exe)
        .args(["--exact", "ownership_child", "--nocapture"])
        .env("ICERS_TEST_OWNERSHIP_DIR", temp.path())
        .env("ICERS_TEST_EXPECT_BUSY", "1")
        .status()
        .unwrap();
    assert!(status.success());
    drop(owner);
    let mut child = std::process::Command::new(exe)
        .args(["--exact", "ownership_child", "--nocapture"])
        .env("ICERS_TEST_OWNERSHIP_DIR", temp.path())
        .env_remove("ICERS_TEST_EXPECT_BUSY")
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !temp.path().join("ready").exists() {
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("ownership child did not become ready");
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(load(temp.path()).await.is_err());
    child.kill().unwrap();
    child.wait().unwrap();
    let reopened = load(temp.path()).await.unwrap();
    assert_eq!(
        reopened.get_name().await.unwrap().as_deref(),
        Some("child acknowledged work")
    );
}
