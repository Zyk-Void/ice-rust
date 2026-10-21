//! Tree navigation and branch summary regressions, including JSONL recovery.
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use ice_agent::message::AgentMessage;
use ice_ai::providers::faux::{faux_assistant_message, FauxProvider, FauxScript, FauxStep};
use ice_ai::types::{StopReason, UserMessage};
use ice_harness::agent_harness::{AgentHarness, AgentLane, NavigationOutcome};
use ice_harness::session::jsonl::{
    JsonlSessionCreateOptions, JsonlSessionListOptions, JsonlSessionRepo, JsonlSessionRepoOptions,
};
use ice_harness::session::memory::{CounterIdGenerator, FakeClock};
use ice_harness::session::types::{
    Entry, EntryQuery, LaneRecord, OperationIntent, OperationOutcome, OperationStartedRecord,
    ProvisionedEntry, ProvisionedKind, RecordBase, RecordQuery,
};
use ice_harness::session::Session;
use ice_harness::types::AgentHarnessOptions;
use ice_tools::{FileSystem, InMemoryExecutionEnv};

fn user(text: &str) -> AgentMessage {
    AgentMessage::User(UserMessage::new(text, 0))
}

fn options(provider: &Arc<FauxProvider>, session: Session) -> AgentHarnessOptions {
    let mut options = AgentHarnessOptions {
        model: provider.default_model().clone(),
        models: vec![provider.clone()],
        session,
        allow_existing_session: true,
        ..Default::default()
    };
    options.retry.enabled = false;
    options
}

async fn fixture(script: FauxScript) -> (AgentHarness, Arc<FauxProvider>, String, String) {
    let provider = FauxProvider::new(script);
    let session = AgentHarnessOptions::default().session;
    let root = session.append_message(user("common root")).await.unwrap();
    let source = session
        .append_message(user("abandoned branch work"))
        .await
        .unwrap();
    let harness = AgentHarness::create(options(&provider, session))
        .await
        .unwrap();
    (harness, provider, root, source)
}

async fn assert_finished(session: &Session, expected: OperationOutcome) {
    assert!(session
        .find_open_operations("main", None)
        .await
        .unwrap()
        .is_empty());
    let records = session.find_records(&RecordQuery::default()).await.unwrap();
    let last = records
        .iter()
        .find_map(|record| match record {
            LaneRecord::OperationFinished(record) => Some(record.outcome),
            _ => None,
        })
        .unwrap();
    assert_eq!(last, expected);
}

#[tokio::test]
async fn tree_navigation_summary_uses_abandoned_tail_and_custom_instructions() {
    let prompt = Arc::new(Mutex::new(String::new()));
    let captured = prompt.clone();
    let script = FauxScript::new();
    script.set_responses(vec![FauxStep::factory(move |context| {
        *captured.lock().unwrap() = serde_json::to_string(context).unwrap();
        faux_assistant_message("Retain completed work", StopReason::Stop)
    })]);
    let (harness, provider, root, source) = fixture(script).await;
    let result = harness
        .navigate_tree(
            Some(&root),
            true,
            Some("Focus on tests"),
            Some("Checkpoint"),
        )
        .await
        .unwrap();
    let NavigationOutcome::Completed {
        new_leaf_id,
        summary_entry: Some(Entry::BranchSummary(entry)),
    } = result.outcome
    else {
        panic!("expected committed branch summary")
    };
    assert_eq!(entry.base.parent_id.as_deref(), Some(root.as_str()));
    assert_eq!(entry.from_id, source);
    assert!(entry.summary.contains("Retain completed work"));
    assert!(entry.usage.is_some());
    assert_eq!(entry.details.unwrap()["readFiles"], serde_json::json!([]));
    assert_eq!(new_leaf_id.as_deref(), Some(entry.base.id.as_str()));
    assert_eq!(
        harness
            .session()
            .get_label(&entry.base.id)
            .await
            .unwrap()
            .as_deref(),
        Some("Checkpoint")
    );
    assert!(harness
        .session()
        .get_entry(&source)
        .await
        .unwrap()
        .is_some());
    let prompt = prompt.lock().unwrap().clone();
    assert!(prompt.contains("abandoned branch work"));
    assert!(prompt.contains("Focus on tests"));
    assert!(!prompt.contains("common root"));
    assert_eq!(provider.state().call_count.load(Ordering::SeqCst), 1);
    assert_finished(harness.session(), OperationOutcome::Completed).await;
}

#[tokio::test]
async fn tree_navigation_plain_noop_invalid_and_forward_moves_do_not_call_provider() {
    let (harness, provider, root, source) = fixture(FauxScript::new()).await;
    assert!(matches!(
        harness
            .navigate_tree(Some(&source), true, None, None)
            .await
            .unwrap()
            .outcome,
        NavigationOutcome::Declined { .. }
    ));
    assert!(matches!(
        harness
            .navigate_tree(Some("missing"), true, None, None)
            .await
            .unwrap()
            .outcome,
        NavigationOutcome::Failed { .. }
    ));
    assert_eq!(
        harness.session().get_leaf_id().await.unwrap().as_deref(),
        Some(source.as_str())
    );
    let plain = harness
        .navigate_tree(Some(&root), false, None, Some("Root"))
        .await
        .unwrap();
    assert!(matches!(
        plain.outcome,
        NavigationOutcome::Completed {
            summary_entry: None,
            ..
        }
    ));
    assert_eq!(
        harness.session().get_label(&root).await.unwrap().as_deref(),
        Some("Root")
    );
    // The destination extends the current path: there is no abandoned tail.
    let forward = harness
        .navigate_tree(Some(&source), true, None, None)
        .await
        .unwrap();
    assert!(matches!(
        forward.outcome,
        NavigationOutcome::Completed {
            summary_entry: None,
            ..
        }
    ));
    assert_eq!(provider.state().call_count.load(Ordering::SeqCst), 0);
    assert_finished(harness.session(), OperationOutcome::Completed).await;
}

#[tokio::test]
async fn tree_navigation_error_and_aborted_provider_keep_the_source_branch() {
    for reason in [StopReason::Error, StopReason::Aborted] {
        let script = FauxScript::new();
        let mut reply = faux_assistant_message("", reason);
        reply.error_message = Some("injected summary outcome".into());
        script.set_responses(vec![FauxStep::message(reply)]);
        let (harness, _, root, source) = fixture(script).await;
        let result = harness
            .navigate_tree(Some(&root), true, None, None)
            .await
            .unwrap();
        let expected = if reason == StopReason::Aborted {
            assert!(matches!(result.outcome, NavigationOutcome::Aborted { .. }));
            OperationOutcome::Aborted
        } else {
            assert!(matches!(result.outcome, NavigationOutcome::Failed { .. }));
            OperationOutcome::Failed
        };
        assert_eq!(
            harness.session().get_leaf_id().await.unwrap().as_deref(),
            Some(source.as_str())
        );
        assert_eq!(
            harness
                .session()
                .find_entries(&EntryQuery::default())
                .await
                .unwrap()
                .len(),
            2
        );
        assert_finished(harness.session(), expected).await;
        // A failed summary must release admission for another operation.
        assert!(matches!(
            harness
                .navigate_tree(Some(&root), false, None, None)
                .await
                .unwrap()
                .outcome,
            NavigationOutcome::Completed { .. }
        ));
    }
}

#[tokio::test]
async fn tree_navigation_cancel_during_generation_is_atomic_and_releases_admission() {
    let started = Arc::new(tokio::sync::Notify::new());
    let notify = started.clone();
    let script = FauxScript::new().with_tokens_per_second(1.0);
    script.set_responses(vec![FauxStep::factory(move |_| {
        notify.notify_one();
        faux_assistant_message("delayed summary ".repeat(100), StopReason::Stop)
    })]);
    let (harness, _, root, source) = fixture(script).await;
    let harness = Arc::new(harness);
    let running = {
        let harness = harness.clone();
        let root = root.clone();
        tokio::spawn(async move { harness.navigate_tree(Some(&root), true, None, None).await })
    };
    tokio::time::timeout(std::time::Duration::from_secs(1), started.notified())
        .await
        .unwrap();
    assert_eq!(
        harness.session().get_leaf_id().await.unwrap().as_deref(),
        Some(source.as_str())
    );
    assert!(harness
        .navigate_tree(Some(&root), false, None, None)
        .await
        .is_err());
    harness.abort().await.unwrap();
    let result = tokio::time::timeout(std::time::Duration::from_secs(1), running)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(matches!(result.outcome, NavigationOutcome::Aborted { .. }));
    assert_eq!(
        harness.session().get_leaf_id().await.unwrap().as_deref(),
        Some(source.as_str())
    );
    assert_eq!(
        harness
            .session()
            .find_entries(&EntryQuery::default())
            .await
            .unwrap()
            .len(),
        2
    );
    assert_finished(harness.session(), OperationOutcome::Aborted).await;
    assert!(matches!(
        harness
            .navigate_tree(Some(&root), false, None, None)
            .await
            .unwrap()
            .outcome,
        NavigationOutcome::Completed { .. }
    ));
}

#[tokio::test]
async fn tree_navigation_summary_isolated_to_its_lane_and_can_target_root() {
    let script = FauxScript::new().with_text("Side branch summary");
    let (harness, _, root, source) = fixture(script).await;
    harness
        .session()
        .create_lane("side", Some(&root))
        .await
        .unwrap();
    let side_source = harness
        .session()
        .view("side")
        .append_message(user("side work"))
        .await
        .unwrap();
    let result = harness
        .lane("side")
        .navigate_tree(None, true, None, None)
        .await
        .unwrap();
    let NavigationOutcome::Completed {
        summary_entry: Some(Entry::BranchSummary(entry)),
        ..
    } = result.outcome
    else {
        panic!("expected side summary")
    };
    assert_eq!(entry.base.parent_id, None);
    assert_eq!(entry.from_id, side_source);
    assert_eq!(
        harness.session().get_leaf_id().await.unwrap().as_deref(),
        Some(source.as_str())
    );
    assert_eq!(
        harness.session().view("side").get_leaf_id().await.unwrap(),
        Some(entry.base.id)
    );
}

fn jsonl_repo(fs: Arc<dyn FileSystem>) -> JsonlSessionRepo {
    JsonlSessionRepo::new(
        JsonlSessionRepoOptions {
            fs,
            sessions_root: "/sessions".into(),
            clock: Arc::new(FakeClock::new()),
            ids: Arc::new(CounterIdGenerator::new()),
        },
        "/project".into(),
    )
}

#[tokio::test]
async fn tree_navigation_jsonl_recovery_rolls_back_partial_move_or_keeps_committed_summary() {
    for phase in 0..3 {
        let fs = Arc::new(InMemoryExecutionEnv::new());
        let repo = jsonl_repo(fs);
        let storage = repo
            .create_typed(&JsonlSessionCreateOptions {
                id: Some(format!("navigation-{phase}")),
                cwd: "/project".into(),
                parent_session_id: None,
                metadata: None,
            })
            .await
            .unwrap();
        let session = Session::new(Arc::new(storage), None);
        let root = session.append_message(user("root")).await.unwrap();
        let source = session.append_message(user("branch")).await.unwrap();
        session
            .append_record(LaneRecord::OperationStarted(OperationStartedRecord {
                base: RecordBase {
                    id: "navigation".into(),
                    seq: 0,
                    lane: "main".into(),
                    timestamp: 0,
                },
                source_leaf_id: Some(source.clone()),
                intent: OperationIntent::Navigation {
                    target_id: Some(root.clone()),
                    summarize: true,
                    custom_instructions: None,
                    label: Some("Recovered".into()),
                    summary_entry_id: Some("summary".into()),
                },
            }))
            .await
            .unwrap();
        if phase >= 1 {
            session.move_lane("main", Some(&root)).await.unwrap();
        }
        if phase == 2 {
            session
                .append_entry(
                    ProvisionedEntry {
                        id: "summary".into(),
                        kind: ProvisionedKind::BranchSummary {
                            from_id: source.clone(),
                            summary: "durable result".into(),
                            details: None,
                            usage: None,
                        },
                    },
                    "main",
                )
                .await
                .unwrap();
        }
        let metadata = repo
            .list_typed(&JsonlSessionListOptions {
                cwd: Some("/project".into()),
            })
            .await
            .unwrap()
            .pop()
            .unwrap();
        drop(session);
        let reopened = repo.open_by_jsonl_metadata(&metadata).await.unwrap();
        let provider = FauxProvider::new(FauxScript::new());
        let restored =
            AgentHarness::create(options(&provider, Session::new(Arc::new(reopened), None)))
                .await
                .unwrap();
        assert_eq!(
            restored.session().get_leaf_id().await.unwrap(),
            Some(if phase == 2 { "summary".into() } else { source })
        );
        assert_finished(
            restored.session(),
            if phase == 2 {
                OperationOutcome::Completed
            } else {
                OperationOutcome::Aborted
            },
        )
        .await;
        assert!(
            restored.pending_resume().is_none(),
            "navigation recovery must never auto-prompt"
        );
        assert_eq!(provider.state().call_count.load(Ordering::SeqCst), 0);
        if phase == 2 {
            assert_eq!(
                restored
                    .session()
                    .get_label("summary")
                    .await
                    .unwrap()
                    .as_deref(),
                Some("Recovered")
            );
        }
    }
}

/// Fail only the BranchSummary entry append, after the lane move has landed.
struct SummaryAppendFailure {
    inner: Arc<dyn FileSystem>,
    failures: AtomicUsize,
    phase: FailurePhase,
}

#[derive(Clone, Copy, Debug)]
enum FailurePhase {
    Summary,
    Step,
    Label,
    Acceptance,
}

#[async_trait::async_trait]
impl FileSystem for SummaryAppendFailure {
    fn cwd(&self) -> &std::path::Path {
        self.inner.cwd()
    }
    async fn absolute_path(
        &self,
        path: &str,
        cancel: Option<&tokio_util::sync::CancellationToken>,
    ) -> Result<std::path::PathBuf, ice_tools::error::FileError> {
        self.inner.absolute_path(path, cancel).await
    }
    async fn join_path(
        &self,
        parts: &[&str],
        cancel: Option<&tokio_util::sync::CancellationToken>,
    ) -> Result<std::path::PathBuf, ice_tools::error::FileError> {
        self.inner.join_path(parts, cancel).await
    }
    async fn read_text_file(
        &self,
        path: &str,
        cancel: Option<&tokio_util::sync::CancellationToken>,
    ) -> Result<String, ice_tools::error::FileError> {
        self.inner.read_text_file(path, cancel).await
    }
    async fn read_text_lines(
        &self,
        path: &str,
        max_lines: Option<usize>,
        cancel: Option<&tokio_util::sync::CancellationToken>,
    ) -> Result<Vec<String>, ice_tools::error::FileError> {
        self.inner.read_text_lines(path, max_lines, cancel).await
    }
    async fn read_binary_file(
        &self,
        path: &str,
        cancel: Option<&tokio_util::sync::CancellationToken>,
    ) -> Result<Vec<u8>, ice_tools::error::FileError> {
        self.inner.read_binary_file(path, cancel).await
    }
    async fn write_file(
        &self,
        path: &str,
        content: ice_tools::env::FileContent,
        cancel: Option<&tokio_util::sync::CancellationToken>,
    ) -> Result<(), ice_tools::error::FileError> {
        self.inner.write_file(path, content, cancel).await
    }
    async fn append_file(
        &self,
        path: &str,
        content: ice_tools::env::FileContent,
        cancel: Option<&tokio_util::sync::CancellationToken>,
    ) -> Result<(), ice_tools::error::FileError> {
        if let ice_tools::env::FileContent::Text(text) = &content {
            use ice_harness::session::types::SessionMutation;
            let should_fail = match (
                self.phase,
                ice_harness::session::jsonl::parse_mutation(text),
            ) {
                (
                    FailurePhase::Summary,
                    Ok(SessionMutation::Entry {
                        entry: Entry::BranchSummary(_),
                        ..
                    }),
                )
                | (
                    FailurePhase::Step,
                    Ok(SessionMutation::Record {
                        record: LaneRecord::StepAttempt(_),
                        ..
                    }),
                )
                | (FailurePhase::Label, Ok(SessionMutation::FactLabel { .. }))
                | (
                    FailurePhase::Acceptance,
                    Ok(SessionMutation::Record {
                        record: LaneRecord::OperationFinished(_),
                        ..
                    }),
                ) => true,
                _ => false,
            };
            if should_fail && self.failures.fetch_add(1, Ordering::SeqCst) == 0 {
                return Err(ice_tools::error::FileError::new(
                    ice_tools::error::FileErrorCode::Unknown,
                    "injected summary append failure",
                ));
            }
        }
        self.inner.append_file(path, content, cancel).await
    }
    async fn rename_file(
        &self,
        source: &str,
        destination: &str,
        cancel: Option<&tokio_util::sync::CancellationToken>,
    ) -> Result<(), ice_tools::error::FileError> {
        self.inner.rename_file(source, destination, cancel).await
    }
    async fn file_info(
        &self,
        path: &str,
        cancel: Option<&tokio_util::sync::CancellationToken>,
    ) -> Result<ice_tools::env::FileInfo, ice_tools::error::FileError> {
        self.inner.file_info(path, cancel).await
    }
    async fn list_dir(
        &self,
        path: &str,
        cancel: Option<&tokio_util::sync::CancellationToken>,
    ) -> Result<Vec<ice_tools::env::FileInfo>, ice_tools::error::FileError> {
        self.inner.list_dir(path, cancel).await
    }
    async fn canonical_path(
        &self,
        path: &str,
        cancel: Option<&tokio_util::sync::CancellationToken>,
    ) -> Result<std::path::PathBuf, ice_tools::error::FileError> {
        self.inner.canonical_path(path, cancel).await
    }
    async fn exists(
        &self,
        path: &str,
        cancel: Option<&tokio_util::sync::CancellationToken>,
    ) -> Result<bool, ice_tools::error::FileError> {
        self.inner.exists(path, cancel).await
    }
    async fn create_dir(
        &self,
        path: &str,
        recursive: bool,
        cancel: Option<&tokio_util::sync::CancellationToken>,
    ) -> Result<(), ice_tools::error::FileError> {
        self.inner.create_dir(path, recursive, cancel).await
    }
    async fn remove(
        &self,
        path: &str,
        recursive: bool,
        force: bool,
        cancel: Option<&tokio_util::sync::CancellationToken>,
    ) -> Result<(), ice_tools::error::FileError> {
        self.inner.remove(path, recursive, force, cancel).await
    }
    async fn create_temp_dir(
        &self,
        prefix: Option<&str>,
        cancel: Option<&tokio_util::sync::CancellationToken>,
    ) -> Result<std::path::PathBuf, ice_tools::error::FileError> {
        self.inner.create_temp_dir(prefix, cancel).await
    }
    async fn create_temp_file(
        &self,
        prefix: &str,
        suffix: &str,
        cancel: Option<&tokio_util::sync::CancellationToken>,
    ) -> Result<std::path::PathBuf, ice_tools::error::FileError> {
        self.inner.create_temp_file(prefix, suffix, cancel).await
    }
    async fn cleanup(&self) {
        self.inner.cleanup().await
    }
}

#[tokio::test]
async fn tree_navigation_jsonl_append_failure_restores_source_and_retry_succeeds() {
    let fs = Arc::new(SummaryAppendFailure {
        inner: Arc::new(InMemoryExecutionEnv::new()),
        failures: AtomicUsize::new(0),
        phase: FailurePhase::Summary,
    });
    let repo = jsonl_repo(fs.clone());
    let storage = repo
        .create_typed(&JsonlSessionCreateOptions {
            id: Some("failure".into()),
            cwd: "/project".into(),
            parent_session_id: None,
            metadata: None,
        })
        .await
        .unwrap();
    let session = Session::new(Arc::new(storage), None);
    let root = session.append_message(user("root")).await.unwrap();
    let source = session.append_message(user("branch")).await.unwrap();
    let provider = FauxProvider::new(FauxScript::new().with_text("first").with_text("retry"));
    let harness = AgentHarness::create(options(&provider, session))
        .await
        .unwrap();
    let result = harness
        .navigate_tree(Some(&root), true, None, None)
        .await
        .unwrap();
    let NavigationOutcome::Failed { error, .. } = result.outcome else {
        panic!("must report append failure")
    };
    assert!(error.message.contains("injected summary append failure"));
    assert_eq!(
        harness.session().get_leaf_id().await.unwrap().as_deref(),
        Some(source.as_str())
    );
    assert_eq!(
        harness
            .session()
            .find_entries(&EntryQuery::default())
            .await
            .unwrap()
            .len(),
        2
    );
    assert_finished(harness.session(), OperationOutcome::Failed).await;
    let retry = harness
        .navigate_tree(Some(&root), true, None, None)
        .await
        .unwrap();
    assert!(matches!(
        retry.outcome,
        NavigationOutcome::Completed {
            summary_entry: Some(_),
            ..
        }
    ));
    assert_finished(harness.session(), OperationOutcome::Completed).await;
    assert_eq!(fs.failures.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn tree_navigation_post_commit_failures_recover_without_regeneration() {
    for phase in [
        FailurePhase::Step,
        FailurePhase::Label,
        FailurePhase::Acceptance,
    ] {
        let fs = Arc::new(SummaryAppendFailure {
            inner: Arc::new(InMemoryExecutionEnv::new()),
            failures: AtomicUsize::new(0),
            phase,
        });
        let repo = jsonl_repo(fs);
        let storage = repo
            .create_typed(&JsonlSessionCreateOptions {
                id: Some(format!("post-commit-{phase:?}")),
                cwd: "/project".into(),
                parent_session_id: None,
                metadata: None,
            })
            .await
            .unwrap();
        let session = Session::new(Arc::new(storage), None);
        let root = session.append_message(user("root")).await.unwrap();
        let source = session.append_message(user("branch")).await.unwrap();
        let provider = FauxProvider::new(FauxScript::new().with_text("durable summary"));
        let harness = AgentHarness::create(options(&provider, session))
            .await
            .unwrap();
        let error = harness
            .navigate_tree(Some(&root), true, None, Some("Checkpoint"))
            .await
            .err()
            .expect("post-commit errors must not claim rejected navigation");
        assert!(
            error.to_string().contains("Navigation committed"),
            "{error}"
        );
        let leaf = harness.session().get_leaf_id().await.unwrap().unwrap();
        assert_ne!(leaf, source, "a durable summary must not be rolled back");
        assert!(matches!(
            harness.session().get_entry(&leaf).await.unwrap(),
            Some(Entry::BranchSummary(_))
        ));
        assert_eq!(
            harness
                .session()
                .find_open_operations("main", None)
                .await
                .unwrap()
                .len(),
            1
        );
        let metadata = repo
            .list_typed(&JsonlSessionListOptions {
                cwd: Some("/project".into()),
            })
            .await
            .unwrap()
            .pop()
            .unwrap();
        drop(harness);
        let reopened = repo.open_by_jsonl_metadata(&metadata).await.unwrap();
        let restored =
            AgentHarness::create(options(&provider, Session::new(Arc::new(reopened), None)))
                .await
                .unwrap();
        assert_eq!(
            restored.session().get_leaf_id().await.unwrap().as_deref(),
            Some(leaf.as_str())
        );
        assert_eq!(
            restored
                .session()
                .get_label(&leaf)
                .await
                .unwrap()
                .as_deref(),
            Some("Checkpoint")
        );
        assert!(restored
            .session()
            .get_entry(&source)
            .await
            .unwrap()
            .is_some());
        assert!(restored.pending_resume().is_none());
        assert_finished(restored.session(), OperationOutcome::Completed).await;
        let records = restored
            .session()
            .find_records(&RecordQuery::default())
            .await
            .unwrap();
        assert_eq!(records.iter().filter(|record| matches!(record, LaneRecord::StepAttempt(step) if step.result_entry_id == leaf)).count(), 1);
        assert_eq!(provider.state().call_count.load(Ordering::SeqCst), 1);
        assert!(matches!(
            restored
                .navigate_tree(Some(&leaf), true, None, None)
                .await
                .unwrap()
                .outcome,
            NavigationOutcome::Declined { .. }
        ));
    }
}

#[tokio::test]
async fn tree_navigation_reserve_tokens_changes_actual_summary_budget() {
    let prompts = Arc::new(Mutex::new(Vec::new()));
    let captured = prompts.clone();
    let script = FauxScript::new();
    let response = FauxStep::factory(move |context| {
        captured
            .lock()
            .unwrap()
            .push(serde_json::to_string(context).unwrap());
        faux_assistant_message("budgeted summary", StopReason::Stop)
    });
    script.set_responses(vec![response.clone(), response]);
    let provider = FauxProvider::new(script);
    let session = AgentHarnessOptions::default().session;
    let root = session.append_message(user("common root")).await.unwrap();
    session
        .append_message(user(&format!("older-marker {}", "x".repeat(800))))
        .await
        .unwrap();
    let source = session.append_message(user("recent-marker")).await.unwrap();
    let mut opts = options(&provider, session);
    opts.model.context_window = 1000;
    let harness = AgentHarness::create(opts).await.unwrap();
    assert_eq!(
        harness.branch_summary_reserve_tokens().await.unwrap(),
        16384
    );
    harness
        .set_branch_summary_reserve_tokens(900)
        .await
        .unwrap();
    assert!(matches!(
        harness
            .navigate_tree(Some(&root), true, None, None)
            .await
            .unwrap()
            .outcome,
        NavigationOutcome::Completed {
            summary_entry: Some(_),
            ..
        }
    ));
    {
        let prompts = prompts.lock().unwrap();
        assert_eq!(prompts.len(), 1);
        assert!(prompts[0].contains("recent-marker"));
        assert!(!prompts[0].contains("older-marker"));
    }
    harness
        .navigate_tree(Some(&source), false, None, None)
        .await
        .unwrap();
    harness.set_branch_summary_reserve_tokens(-1).await.unwrap();
    assert_eq!(harness.branch_summary_reserve_tokens().await.unwrap(), 1);
    harness
        .navigate_tree(Some(&root), true, None, None)
        .await
        .unwrap();
    {
        let prompts = prompts.lock().unwrap();
        assert_eq!(prompts.len(), 2);
        assert!(prompts[1].contains("recent-marker"));
        assert!(prompts[1].contains("older-marker"));
    }
    harness
        .navigate_tree(Some(&source), false, None, None)
        .await
        .unwrap();
    harness
        .set_branch_summary_reserve_tokens(i64::MAX)
        .await
        .unwrap();
    let result = harness
        .navigate_tree(Some(&root), true, None, None)
        .await
        .unwrap();
    let NavigationOutcome::Completed {
        summary_entry: Some(Entry::BranchSummary(summary)),
        ..
    } = result.outcome
    else {
        panic!("expected budget-empty summary: {:?}", result.outcome)
    };
    assert_eq!(summary.summary, "No content to summarize");
    assert_eq!(provider.state().call_count.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn tree_navigation_retry_after_acceptance_failure_recognizes_durable_summary() {
    let repo = jsonl_repo(Arc::new(SummaryAppendFailure {
        inner: Arc::new(InMemoryExecutionEnv::new()),
        failures: AtomicUsize::new(0),
        phase: FailurePhase::Acceptance,
    }));
    let storage = repo
        .create_typed(&JsonlSessionCreateOptions {
            id: Some("retry-acceptance".into()),
            cwd: "/project".into(),
            parent_session_id: None,
            metadata: None,
        })
        .await
        .unwrap();
    let session = Session::new(Arc::new(storage), None);
    let root = session.append_message(user("root")).await.unwrap();
    session.append_message(user("branch")).await.unwrap();
    let provider = FauxProvider::new(FauxScript::new().with_text("accepted summary"));
    let harness = AgentHarness::create(options(&provider, session))
        .await
        .unwrap();
    assert!(harness
        .navigate_tree(Some(&root), true, None, None)
        .await
        .is_err());
    let committed = harness.session().get_leaf_id().await.unwrap();
    let retry = harness
        .navigate_tree(Some(&root), true, None, None)
        .await
        .unwrap();
    let NavigationOutcome::Completed {
        new_leaf_id,
        summary_entry: Some(_),
    } = retry.outcome
    else {
        panic!("retry should recognize the accepted result")
    };
    assert_eq!(new_leaf_id, committed);
    assert_eq!(provider.state().call_count.load(Ordering::SeqCst), 1);
    assert_finished(harness.session(), OperationOutcome::Completed).await;
}

#[tokio::test]
async fn tree_navigation_reserve_survives_session_replacement_and_rejects_closed_harness() {
    let (harness, _, _, _) = fixture(FauxScript::new()).await;
    harness
        .set_branch_summary_reserve_tokens(1234)
        .await
        .unwrap();
    harness
        .set_session(AgentHarnessOptions::default().session)
        .await
        .unwrap();
    assert_eq!(harness.branch_summary_reserve_tokens().await.unwrap(), 1234);
    harness.set_branch_summary_reserve_tokens(0).await.unwrap();
    assert_eq!(harness.branch_summary_reserve_tokens().await.unwrap(), 1);
    harness.close().await.unwrap();
    assert!(harness
        .set_branch_summary_reserve_tokens(500)
        .await
        .is_err());
    assert!(harness.branch_summary_reserve_tokens().await.is_err());
}
