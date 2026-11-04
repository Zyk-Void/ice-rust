//! Recovery must remain retryable after a second crash or an ambiguous write.
//! Synthetic storage injects errors before/after individual durable appends.

use std::sync::{Arc, Mutex};

use ice_agent::AgentMessage;
use ice_ai::frames::AssistantMessageFrame;
use ice_ai::providers::faux::{FauxProvider, FauxScript};
use ice_ai::types::{Api, AssistantMessage, Content, ToolCall, UserMessage};
use ice_ai::Provider;
use ice_harness::agent_harness::AgentHarness;
use ice_harness::error::{SessionError, SessionResult};
use ice_harness::session::jsonl::storage::JsonlSessionStorage;
use ice_harness::session::jsonl::types::JsonlV4Header;
use ice_harness::session::memory::{Clock, InMemorySessionStorage, SystemClock};
use ice_harness::session::sqlite::{
    SqliteSessionCreateOptions, SqliteSessionRepo, SqliteWriterLeaseOptions,
};
use ice_harness::session::types::*;
use ice_harness::session::{DefaultIdGenerator, Session};
use ice_harness::types::AgentHarnessOptions;
use ice_tools::env::FileSystem;
use ice_tools::OsExecutionEnv;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Boundary {
    Intent,
    Assistant,
    ResultOne,
    ResultTwo,
    Terminal,
}

struct FailingStorage {
    inner: Arc<dyn SessionStorage>,
    failure: Mutex<Option<(Boundary, bool)>>,
}

impl FailingStorage {
    fn take(&self, boundary: Boundary) -> Option<bool> {
        let mut failure = self.failure.lock().unwrap();
        if failure.is_some_and(|(target, _)| target == boundary) {
            failure.take().map(|(_, after)| after)
        } else {
            None
        }
    }
}

async fn injected() -> SessionError {
    if let Some(dir) = std::env::var_os("ICERS_RECOVERY_CRASH_DIR") {
        // Test-child-only pause exactly at the selected recovery boundary.
        std::fs::write(
            std::path::PathBuf::from(dir).join("ready"),
            b"recovery boundary reached",
        )
        .unwrap();
        std::future::pending::<()>().await;
    }
    SessionError::storage("injected recovery write failure")
}

#[async_trait::async_trait]
impl SessionStorage for FailingStorage {
    fn metadata(&self) -> SessionMetadata {
        self.inner.metadata()
    }
    async fn get_lanes(&self) -> SessionResult<Vec<LanePointer>> {
        self.inner.get_lanes().await
    }
    async fn create_lane(&self, lane: &str, at: Option<&str>) -> SessionResult<()> {
        self.inner.create_lane(lane, at).await
    }
    async fn move_lane(&self, lane: &str, to: Option<&str>) -> SessionResult<()> {
        self.inner.move_lane(lane, to).await
    }
    async fn append_entry(&self, entry: ProvisionedEntry, lane: &str) -> SessionResult<Entry> {
        let boundary = match &entry.kind {
            ProvisionedKind::Message {
                message: AgentMessage::Assistant(_),
                ..
            } => Some(Boundary::Assistant),
            ProvisionedKind::Message {
                message: AgentMessage::ToolResult(result),
                ..
            } => Some(if result.tool_call_id == "call-1" {
                Boundary::ResultOne
            } else {
                Boundary::ResultTwo
            }),
            _ => None,
        };
        let failure = boundary.and_then(|boundary| self.take(boundary));
        if failure == Some(false) {
            return Err(injected().await);
        }
        let result = self.inner.append_entry(entry, lane).await?;
        if failure == Some(true) {
            return Err(injected().await);
        }
        Ok(result)
    }
    async fn append_record(&self, record: LaneRecord) -> SessionResult<LaneRecord> {
        let boundary = match &record {
            LaneRecord::OperationFinished(_) => Some(Boundary::Terminal),
            LaneRecord::AssistantFrame(frame) if frame.op == AssistantFrameOp::CommitStream => {
                Some(Boundary::Intent)
            }
            _ => None,
        };
        let failure = boundary.and_then(|boundary| self.take(boundary));
        if failure == Some(false) {
            return Err(injected().await);
        }
        // Force concurrent openers to interleave at the commit-intent boundary.
        tokio::task::yield_now().await;
        let result = self.inner.append_record(record).await?;
        if failure == Some(true) {
            return Err(injected().await);
        }
        Ok(result)
    }
    async fn get_entry(&self, id: &str) -> SessionResult<Option<Entry>> {
        self.inner.get_entry(id).await
    }
    async fn find_entries(&self, query: &EntryQuery) -> SessionResult<Vec<Entry>> {
        self.inner.find_entries(query).await
    }
    async fn find_entries_on_branch(
        &self,
        query: &EntryQuery,
        bounds: &BranchBounds,
        start: &str,
    ) -> SessionResult<Vec<Entry>> {
        self.inner
            .find_entries_on_branch(query, bounds, start)
            .await
    }
    async fn find_records(&self, query: &RecordQuery) -> SessionResult<Vec<LaneRecord>> {
        self.inner.find_records(query).await
    }
    async fn find_open_operations(
        &self,
        lane: &str,
        limit: Option<usize>,
    ) -> SessionResult<Vec<OperationStartedRecord>> {
        self.inner.find_open_operations(lane, limit).await
    }
    async fn get_log(&self, options: &LogOptions) -> SessionResult<Vec<LogItem>> {
        self.inner.get_log(options).await
    }
    async fn get_name(&self) -> SessionResult<Option<String>> {
        self.inner.get_name().await
    }
    async fn set_name(&self, name: Option<&str>) -> SessionResult<()> {
        self.inner.set_name(name).await
    }
    async fn get_label(&self, id: &str) -> SessionResult<Option<String>> {
        self.inner.get_label(id).await
    }
    async fn set_label(&self, id: &str, label: Option<&str>) -> SessionResult<()> {
        self.inner.set_label(id, label).await
    }
    async fn get_stats(&self) -> SessionResult<SessionStats> {
        self.inner.get_stats().await
    }
}

fn base(id: String) -> RecordBase {
    RecordBase {
        id,
        seq: 0,
        lane: "main".into(),
        timestamp: 0,
    }
}

async fn seeded() -> (Session, Arc<FailingStorage>) {
    let storage = Arc::new(FailingStorage {
        inner: Arc::new(InMemorySessionStorage::new(
            SessionMetadata {
                id: "recovery-faults".into(),
                created_at: 0,
                parent_session_id: None,
            },
            Arc::new(SystemClock),
            Arc::new(DefaultIdGenerator::new()),
        )),
        failure: Mutex::new(None),
    });
    let session = Session::new(storage.clone(), None);
    seed_history(&session).await;
    (session, storage)
}

async fn seed_history(session: &Session) {
    session
        .append_message(AgentMessage::User(UserMessage::new("keep my work", 0)))
        .await
        .unwrap();
    session
        .append_record(LaneRecord::OperationStarted(OperationStartedRecord {
            base: base("crashed-run".into()),
            source_leaf_id: None,
            intent: OperationIntent::Run {
                original_prompt: vec![],
                initial_messages: vec![],
                system_prompt_override: None,
                resume_data: None,
            },
        }))
        .await
        .unwrap();
    let mut frames = vec![
        AssistantMessageFrame::Start {
            partial: AssistantMessage::empty(Api::Faux, "faux", "faux", 0),
        },
        AssistantMessageFrame::TextStart {
            content_index: 0,
            content: serde_json::from_value(
                serde_json::json!({"type":"text", "text":"recoverable work"}),
            )
            .unwrap(),
        },
    ];
    for (index, id) in ["call-1", "call-2"].into_iter().enumerate() {
        frames.push(AssistantMessageFrame::ToolcallStart {
            content_index: index + 1,
            tool_call: ToolCall {
                kind: Default::default(),
                id: id.into(),
                name: "unsafe_external_action".into(),
                arguments: serde_json::json!({}),
                thought_signature: None,
                namespace: None,
            },
        });
    }
    for (index, frame) in frames.into_iter().enumerate() {
        session
            .append_record(LaneRecord::AssistantFrame(AssistantFrameRecord {
                base: base(format!("frame-{index}")),
                run_id: "crashed-run".into(),
                stream_index: 0,
                op: AssistantFrameOp::Append,
                frame: Some(serde_json::to_value(frame).unwrap()),
            }))
            .await
            .unwrap();
    }
}

async fn reopen(session: Session) -> ice_harness::result::HarnessResult<AgentHarness> {
    let provider = FauxProvider::new(FauxScript::new().with_text("must not execute"));
    AgentHarness::create(AgentHarnessOptions {
        model: provider.default_model().clone(),
        thinking_level: Default::default(),
        active_tool_names: vec![],
        tools: vec![],
        system_prompt: None,
        resources: Default::default(),
        stream_options: Default::default(),
        retry: Default::default(),
        compaction: Default::default(),
        steering_mode: Default::default(),
        follow_up_mode: Default::default(),
        tool_execution: Default::default(),
        drive: Default::default(),
        session,
        models: vec![provider as Arc<dyn Provider>],
        to_provider_messages: None,
        entry_projectors: Default::default(),
        agent_emitter: None,
        before_tool_call: None,
        after_tool_call: None,
        transform_context: None,
        entry_transforms: vec![],
        provider_hooks: None,
        allow_existing_session: true,
    })
    .await
}

#[tokio::test]
async fn recovery_is_retryable_at_every_write_boundary() {
    for boundary in [
        Boundary::Intent,
        Boundary::Assistant,
        Boundary::ResultOne,
        Boundary::ResultTwo,
        Boundary::Terminal,
    ] {
        for after in [false, true] {
            let (session, storage) = seeded().await;
            *storage.failure.lock().unwrap() = Some((boundary, after));
            assert!(
                reopen(session.clone()).await.is_err(),
                "must report {boundary:?} after={after}"
            );
            assert!(
                storage.failure.lock().unwrap().is_none(),
                "boundary exercised"
            );
            for _ in 0..2 {
                drop(
                    reopen(session.clone())
                        .await
                        .expect("recovery is resumable"),
                );
            }
            assert!(session
                .find_open_operations("main", None)
                .await
                .unwrap()
                .is_empty());
            let entries = session.find_entries(&EntryQuery::default()).await.unwrap();
            let messages: Vec<_> = entries
                .iter()
                .filter_map(|entry| match entry {
                    Entry::Message(entry) => Some(&entry.message),
                    _ => None,
                })
                .collect();
            assert_eq!(
                messages.len(),
                4,
                "one user, one assistant, two results: {boundary:?} after={after}"
            );
            let assistant = messages
                .iter()
                .find_map(|message| match message {
                    AgentMessage::Assistant(message) => Some(message),
                    _ => None,
                })
                .unwrap();
            assert!(assistant.content.iter().any(
                |block| matches!(block, Content::Text(text) if text.text == "recoverable work")
            ));
            for id in ["call-1", "call-2"] {
                let results: Vec<_> = messages
                    .iter()
                    .filter_map(|message| match message {
                        AgentMessage::ToolResult(result) if result.tool_call_id == id => {
                            Some(result)
                        }
                        _ => None,
                    })
                    .collect();
                assert_eq!(results.len(), 1, "no duplicate or omitted results");
                assert!(results[0].is_error, "unsafe actions are not re-executed");
                assert!(results[0].content.iter().any(|block| matches!(block, Content::Text(text) if text.text.contains("external outcome is unknown"))));
            }
        }
    }
}

#[tokio::test]
async fn concurrent_reopen_of_cloned_session_does_not_duplicate_salvage() {
    let (session, _) = seeded().await;
    let (first, second) = tokio::join!(reopen(session.clone()), reopen(session.clone()));
    drop(first.expect("first repair"));
    drop(second.expect("second repair"));
    let entries = session.find_entries(&EntryQuery::default()).await.unwrap();
    assert_eq!(entries.len(), 4, "one user, one assistant, two results");
    let terminals = session
        .find_records(&RecordQuery {
            record_type: Some("operation_finished"),
            run_id: Some("crashed-run".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(terminals.len(), 1);
}

fn disk_fixture() -> tempfile::TempDir {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/harness-hardening-checks");
    std::fs::create_dir_all(&root).unwrap();
    tempfile::Builder::new()
        .prefix("recovery-disk-")
        .tempdir_in(root)
        .unwrap()
}

async fn disk_storage(path: &std::path::Path, create: bool) -> SessionResult<JsonlSessionStorage> {
    let fs: Arc<dyn FileSystem> = Arc::new(OsExecutionEnv::new());
    let clock = Arc::new(SystemClock);
    let ids = Arc::new(DefaultIdGenerator::new());
    let path = path.to_str().unwrap();
    if create {
        JsonlSessionStorage::create(
            fs,
            path,
            JsonlV4Header::new(
                "recovery-disk".into(),
                0,
                "/synthetic".into(),
                None,
                None,
                None,
            ),
            clock,
            ids,
        )
        .await
    } else {
        JsonlSessionStorage::load(fs, path, clock, ids).await
    }
}

async fn assert_repaired(session: &Session) {
    assert!(session
        .find_open_operations("main", None)
        .await
        .unwrap()
        .is_empty());
    let entries = session.find_entries(&EntryQuery::default()).await.unwrap();
    assert_eq!(entries.len(), 4, "one user, one assistant, two results");
    let assistant = entries
        .iter()
        .find_map(|entry| match entry {
            Entry::Message(entry) => match &entry.message {
                AgentMessage::Assistant(message) => Some(message),
                _ => None,
            },
            _ => None,
        })
        .expect("salvaged assistant");
    assert!(assistant.content.iter().any(|block| matches!(block,
        Content::Text(text) if text.text == "recoverable work")));
    for id in ["call-1", "call-2"] {
        assert_eq!(
            assistant
                .content
                .iter()
                .filter(|block| matches!(block,
            Content::ToolCall(call) if call.id == id))
                .count(),
            1
        );
        let results: Vec<_> = entries
            .iter()
            .filter_map(|entry| match entry {
                Entry::Message(entry) => match &entry.message {
                    AgentMessage::ToolResult(result) if result.tool_call_id == id => Some(result),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        assert_eq!(results.len(), 1, "one result per recovered call");
        assert!(results[0].is_error);
        assert!(results[0].content.iter().any(|block| matches!(block,
            Content::Text(text) if text.text.contains("external outcome is unknown"))));
    }
    let terminals = session
        .find_records(&RecordQuery {
            record_type: Some("operation_finished"),
            run_id: Some("crashed-run".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(terminals.len(), 1, "one durable terminal record");
}

#[derive(Default)]
struct LeaseClock(std::sync::atomic::AtomicI64);

impl Clock for LeaseClock {
    fn now_ms(&self) -> i64 {
        self.0.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[tokio::test]
async fn sqlite_lease_takeover_repairs_every_write_failure_and_survives_reopen() {
    for boundary in [
        Boundary::Intent,
        Boundary::Assistant,
        Boundary::ResultOne,
        Boundary::ResultTwo,
        Boundary::Terminal,
    ] {
        for after in [false, true] {
            let dir = disk_fixture();
            let path = dir.path().join("sessions.db");
            let clock = Arc::new(LeaseClock::default());
            let repository = || {
                SqliteSessionRepo::with_writer_lease(
                    &path,
                    clock.clone(),
                    Arc::new(DefaultIdGenerator::new()),
                    SqliteWriterLeaseOptions {
                        ttl_ms: 1_000_000,
                        heartbeat_interval_ms: 900_000,
                    },
                )
                .unwrap()
            };
            let old_repo = repository();
            let old_storage = Arc::new(
                old_repo
                    .create_with_metadata(&SqliteSessionCreateOptions {
                        cwd: "/synthetic/recovery".into(),
                        ..Default::default()
                    })
                    .await
                    .unwrap(),
            );
            let metadata = old_storage.sqlite_metadata();
            let stale = Session::new(old_storage.clone(), None);
            seed_history(&stale).await;
            let new_repo = repository();
            assert!(
                new_repo.open_with_metadata(&metadata).await.is_err(),
                "live lease excludes takeover"
            );
            // Deterministic expired ownership, not an actual process crash.
            clock
                .0
                .store(2_000_000, std::sync::atomic::Ordering::SeqCst);
            let storage = Arc::new(FailingStorage {
                inner: Arc::new(new_repo.open_with_metadata(&metadata).await.unwrap()),
                failure: Mutex::new(Some((boundary, after))),
            });
            let session = Session::new(storage.clone(), None);
            assert!(
                reopen(stale.clone()).await.is_err(),
                "expired owner cannot repair"
            );
            assert!(
                reopen(session.clone()).await.is_err(),
                "{boundary:?} after={after}"
            );
            assert!(
                storage.failure.lock().unwrap().is_none(),
                "boundary exercised"
            );
            drop(
                reopen(session.clone())
                    .await
                    .expect("retry repair under new fence"),
            );
            assert_repaired(&session).await;
            old_storage.close().await.unwrap();
            let contender = repository();
            assert!(
                contender.open_with_metadata(&metadata).await.is_err(),
                "stale close cannot release new fence"
            );
            contender.close().await.unwrap();
            drop(stale);
            drop(old_storage);
            old_repo.close().await.unwrap();
            drop(session);
            drop(storage);
            new_repo.close().await.unwrap();
            for _ in 0..2 {
                let repo = repository();
                let session = Session::new(
                    Arc::new(repo.open_with_metadata(&metadata).await.unwrap()),
                    None,
                );
                drop(
                    reopen(session.clone())
                        .await
                        .expect("repair replay from SQLite"),
                );
                assert_repaired(&session).await;
                drop(session);
                repo.close().await.unwrap();
            }
        }
    }
}

#[tokio::test]
async fn jsonl_recovery_survives_every_write_failure_and_two_disk_reopens() {
    for boundary in [
        Boundary::Intent,
        Boundary::Assistant,
        Boundary::ResultOne,
        Boundary::ResultTwo,
        Boundary::Terminal,
    ] {
        for after in [false, true] {
            let dir = disk_fixture();
            let path = dir.path().join("session.jsonl");
            let storage = Arc::new(FailingStorage {
                inner: Arc::new(disk_storage(&path, true).await.unwrap()),
                failure: Mutex::new(None),
            });
            let session = Session::new(storage.clone(), None);
            seed_history(&session).await;
            *storage.failure.lock().unwrap() = Some((boundary, after));
            assert!(
                reopen(session.clone()).await.is_err(),
                "{boundary:?} after={after}"
            );
            assert!(
                storage.failure.lock().unwrap().is_none(),
                "boundary exercised"
            );
            drop(session);
            drop(storage);
            for _ in 0..2 {
                let session =
                    Session::new(Arc::new(disk_storage(&path, false).await.unwrap()), None);
                drop(
                    reopen(session.clone())
                        .await
                        .expect("repair from durable history"),
                );
                assert_repaired(&session).await;
                // All storage handles drop here: the next pass must replay disk.
            }
        }
    }
}

#[tokio::test]
async fn recovery_crash_child() {
    let Some(dir) = std::env::var_os("ICERS_RECOVERY_CRASH_DIR") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    let storage = Arc::new(FailingStorage {
        inner: Arc::new(
            disk_storage(&dir.join("session.jsonl"), true)
                .await
                .unwrap(),
        ),
        failure: Mutex::new(None),
    });
    let session = Session::new(storage.clone(), None);
    seed_history(&session).await;
    if let Ok(target) = std::env::var("ICERS_RECOVERY_CRASH_BOUNDARY") {
        let boundaries = [
            Boundary::Intent,
            Boundary::Assistant,
            Boundary::ResultOne,
            Boundary::ResultTwo,
            Boundary::Terminal,
        ];
        let index: usize = target.parse().unwrap();
        *storage.failure.lock().unwrap() = Some((
            boundaries[index],
            std::env::var_os("ICERS_RECOVERY_CRASH_AFTER").is_some(),
        ));
        let _ = reopen(session).await;
        panic!("selected recovery boundary was not reached");
    }
    std::fs::write(dir.join("ready"), b"progress acknowledged").unwrap();
    // Parent terminates us without running Rust destructors.
    std::future::pending::<()>().await;
}

struct CrashChild(std::process::Child);

impl Drop for CrashChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[tokio::test]
async fn killed_jsonl_owner_preserves_progress_and_recovers_once() {
    kill_and_recover(None).await;
}

#[tokio::test]
async fn killed_jsonl_recovery_resumes_at_every_write_boundary() {
    for boundary in 0..5 {
        for after in [false, true] {
            kill_and_recover(Some((boundary, after))).await;
        }
    }
}

async fn kill_and_recover(boundary: Option<(usize, bool)>) {
    let dir = disk_fixture();
    let log = std::fs::File::create(dir.path().join("child.log")).unwrap();
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "recovery_crash_child", "--nocapture"])
        .env("ICERS_RECOVERY_CRASH_DIR", dir.path())
        .env_remove("ICERS_RECOVERY_CRASH_BOUNDARY")
        .env_remove("ICERS_RECOVERY_CRASH_AFTER")
        .stdout(log.try_clone().unwrap())
        .stderr(log);
    if let Some((index, after)) = boundary {
        command.env("ICERS_RECOVERY_CRASH_BOUNDARY", index.to_string());
        if after {
            command.env("ICERS_RECOVERY_CRASH_AFTER", "1");
        }
    }
    let mut child = CrashChild(command.spawn().unwrap());
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    while !dir.path().join("ready").exists() {
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "child exited before acknowledging progress"
        );
        assert!(
            tokio::time::Instant::now() < deadline,
            "child never acknowledged progress"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let path = dir.path().join("session.jsonl");
    assert!(
        disk_storage(&path, false).await.is_err(),
        "child owns storage before kill"
    );
    child.0.kill().unwrap();
    let status = child.0.wait().unwrap();
    assert!(!status.success(), "child was forcibly terminated");
    for _ in 0..2 {
        let session = Session::new(Arc::new(disk_storage(&path, false).await.unwrap()), None);
        drop(reopen(session.clone()).await.expect("recover killed owner"));
        assert_repaired(&session).await;
    }
}

#[tokio::test]
async fn jsonl_lock_probe_child() {
    let Some(path) = std::env::var_os("ICERS_RECOVERY_LOCK_TEST_PATH") else {
        return;
    };
    let result = disk_storage(std::path::Path::new(&path), false).await;
    if std::env::var_os("ICERS_RECOVERY_LOCK_TEST_HELD").is_some() {
        let error = result.err().expect("another process owns this session");
        assert_eq!(error.code, ice_harness::error::SessionErrorCode::Storage);
        assert!(error.message.contains("already open or cannot be locked"));
    } else {
        drop(result.expect("lock is released after owner drop"));
    }
}

fn probe_process(path: &std::path::Path, held: bool) {
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "jsonl_lock_probe_child", "--nocapture"])
        .env("ICERS_RECOVERY_LOCK_TEST_PATH", path)
        .env_remove("ICERS_RECOVERY_LOCK_TEST_HELD");
    if held {
        command.env("ICERS_RECOVERY_LOCK_TEST_HELD", "1");
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "child lock probe failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn jsonl_ownership_excludes_independent_process_during_and_after_tail_repair() {
    let dir = disk_fixture();
    let path = dir.path().join("session.jsonl");
    let session = Session::new(Arc::new(disk_storage(&path, true).await.unwrap()), None);
    seed_history(&session).await;
    probe_process(&path, true);
    drop(session);
    let valid = std::fs::read(&path).unwrap();
    let mut torn = valid.clone();
    torn.extend_from_slice(b"{\"kind\":");
    std::fs::write(&path, torn).unwrap();
    let session = Session::new(Arc::new(disk_storage(&path, false).await.unwrap()), None);
    assert_eq!(
        std::fs::read(&path).unwrap(),
        valid,
        "only torn suffix removed"
    );
    probe_process(&path, true); // ownership survives atomic replacement of JSONL
    drop(reopen(session.clone()).await.unwrap());
    assert_repaired(&session).await;
    drop(session);
    probe_process(&path, false);
    let session = Session::new(Arc::new(disk_storage(&path, false).await.unwrap()), None);
    drop(reopen(session.clone()).await.unwrap());
    assert_repaired(&session).await;
}

#[tokio::test]
async fn jsonl_hard_corruption_is_not_rewritten_and_failed_load_releases_ownership() {
    for suffix in [
        b"{broken\n{}\n".as_slice(),
        b"{\"kind\":\"not-a-mutation\"}\n".as_slice(),
    ] {
        let dir = disk_fixture();
        let path = dir.path().join("session.jsonl");
        let session = Session::new(Arc::new(disk_storage(&path, true).await.unwrap()), None);
        seed_history(&session).await;
        drop(session);
        let valid = std::fs::read(&path).unwrap();
        let mut corrupt = valid.clone();
        corrupt.extend_from_slice(suffix);
        std::fs::write(&path, &corrupt).unwrap();
        for _ in 0..2 {
            let error = disk_storage(&path, false)
                .await
                .err()
                .expect("hard corruption");
            assert_eq!(
                error.code,
                ice_harness::error::SessionErrorCode::InvalidEntry
            );
            assert_eq!(std::fs::read(&path).unwrap(), corrupt, "preserve evidence");
        }
        // Restore synthetic fixture only; a successful load proves failed loads
        // released ownership. This is not an application data-repair policy.
        std::fs::write(&path, valid).unwrap();
        let session = Session::new(Arc::new(disk_storage(&path, false).await.unwrap()), None);
        drop(reopen(session.clone()).await.unwrap());
        assert_repaired(&session).await;
    }
}

#[tokio::test]
async fn independently_constructed_facades_share_recovery_ownership() {
    let (session, storage) = seeded().await;
    let independent = Session::new(storage, None);
    let (first, second) = tokio::join!(reopen(session.clone()), reopen(independent));
    drop(first.expect("first repair"));
    drop(second.expect("second repair"));
    let entries = session.find_entries(&EntryQuery::default()).await.unwrap();
    assert_eq!(
        entries.len(),
        4,
        "shared storage must not duplicate salvage"
    );
    assert_repaired(&session).await;
}

#[tokio::test]
async fn results_on_another_branch_do_not_suppress_repair() {
    let (session, _) = seeded().await;
    session.create_lane("other", None).await.unwrap();
    let unrelated = ice_harness::frame_progress::interrupted_tool_result(
        &ice_harness::frame_progress::UnknownToolOutcome {
            tool_call_id: "call-1".into(),
            tool_name: "unsafe_external_action".into(),
            arguments: serde_json::json!({}),
        },
        0,
    );
    session
        .view("other")
        .append_message(unrelated)
        .await
        .unwrap();
    drop(reopen(session.clone()).await.expect("repair main"));
    let entries = session
        .view("main")
        .find_entries_on_branch(&EntryQuery::default(), &BranchBounds::default())
        .await
        .unwrap();
    assert_eq!(entries.len(), 4, "other branch cannot answer main's call");
}
