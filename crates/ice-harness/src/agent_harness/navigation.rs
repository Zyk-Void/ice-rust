//! Branch navigation with a cancellable summary and a recoverable commit.
//! Reuses the branch summarizer and existing Navigation/WriteDeferred records.

use super::*;
use crate::compaction::branch_summary::{
    collect_entries_for_branch_summary, generate_branch_summary, BranchSummaryDetails,
    GenerateBranchSummaryOptions,
};

fn navigation_error(message: impl ToString) -> OperationError {
    OperationError {
        code: "navigation_failed".into(),
        message: message.to_string(),
    }
}

fn navigation_commit_pending(error: impl ToString) -> OperationError {
    OperationError {
        code: "navigation_commit_pending".into(),
        message: format!(
            "Navigation committed but finalization failed; retry/reopen to recover: {}",
            error.to_string()
        ),
    }
}

fn navigation_aborted() -> OperationError {
    OperationError {
        code: "aborted".into(),
        message: "Branch summarization cancelled".into(),
    }
}

impl AgentHarness {
    pub(super) async fn navigate_tree_core(
        &self,
        target_id: Option<&str>,
        summarize: bool,
        custom_instructions: Option<&str>,
        label: Option<&str>,
    ) -> HarnessResult<NavigationResult> {
        let (run_id, signal, _idle) = self.acquire_run(OperationKind::Navigation).await?;
        let _lease = ActiveRunLease {
            harness: self,
            run_id: run_id.clone(),
        };
        let target = target_id.filter(|id| !id.is_empty());
        // A caller may retry after finalization failed even though the result
        // landed. Recognize that exact accepted request before generating a
        // summary of the summary. Different requests proceed normally.
        let interrupted_summary = self
            .session
            .find_open_operations(&self.lane, Some(2))
            .await
            .map_err(session_to_harness_err)?
            .into_iter()
            .find_map(|operation| match operation.intent {
                OperationIntent::Navigation {
                    target_id,
                    summarize: previous_summarize,
                    custom_instructions: previous_instructions,
                    label: previous_label,
                    summary_entry_id,
                } if target_id.as_deref() == target
                    && previous_summarize == summarize
                    && previous_instructions.as_deref() == custom_instructions
                    && previous_label.as_deref() == label =>
                {
                    summary_entry_id
                }
                _ => None,
            });
        Self::finish_interrupted_operation(
            &self.session,
            &self.lane,
            &self.get_tools().await?,
            &signal,
            true,
        )
        .await?;
        let source = self
            .view()
            .get_leaf_id()
            .await
            .map_err(session_to_harness_err)?;
        if let Some(id) = interrupted_summary.filter(|id| source.as_deref() == Some(id.as_str())) {
            let summary_entry = self
                .view()
                .get_entry(&id)
                .await
                .map_err(session_to_harness_err)?;
            // Same tail as a fresh navigation: commit context queued meanwhile.
            self.finish_context_writes(&run_id, None).await?;
            return Ok(NavigationResult {
                run_id,
                outcome: NavigationOutcome::Completed {
                    new_leaf_id: source,
                    summary_entry,
                },
            });
        }
        if source.as_deref() == target {
            return Ok(NavigationResult {
                run_id,
                outcome: NavigationOutcome::Declined { leaf_id: source },
            });
        }
        if let Some(target) = target {
            if self
                .view()
                .get_entry(target)
                .await
                .map_err(session_to_harness_err)?
                .is_none()
            {
                return Ok(NavigationResult {
                    run_id,
                    outcome: NavigationOutcome::Failed {
                        leaf_id: source,
                        error: navigation_error(format!("Entry {target} not found")),
                    },
                });
            }
        }
        let entries = if summarize {
            match target {
                Some(target) => {
                    collect_entries_for_branch_summary(
                        self.view().as_ref(),
                        source.as_deref(),
                        target,
                    )
                    .await
                    .map_err(session_to_harness_err)?
                    .entries
                }
                None => self.branch_path_oldest_first().await?,
            }
        } else {
            Vec::new()
        };
        let summary_id = (!entries.is_empty()).then(|| self.session.id_generator().next());
        self.write_operation_started(
            &run_id,
            source.clone(),
            OperationIntent::Navigation {
                target_id: target.map(str::to_string),
                summarize,
                custom_instructions: custom_instructions.map(str::to_string),
                label: label.map(str::to_string),
                summary_entry_id: summary_id.clone(),
            },
        )
        .await?;

        let prepared = self
            .prepare_navigation_summary(
                &entries,
                summary_id,
                source.as_deref(),
                custom_instructions,
                &signal,
            )
            .await;
        let prepared = if signal.is_cancelled() {
            Err(navigation_aborted())
        } else {
            prepared
        };
        let (outcome, durable, error) = match prepared {
            Err(error) if error.code == "aborted" => (
                NavigationOutcome::Aborted {
                    leaf_id: source.clone(),
                },
                OperationOutcome::Aborted,
                Some(error),
            ),
            Err(error) => (
                NavigationOutcome::Failed {
                    leaf_id: source.clone(),
                    error: error.clone(),
                },
                OperationOutcome::Failed,
                Some(error),
            ),
            Ok(entry) => match self
                .commit_navigation(&run_id, target, entry, label, &signal)
                .await
            {
                Ok(summary_entry) => (
                    NavigationOutcome::Completed {
                        new_leaf_id: summary_entry
                            .as_ref()
                            .map(|entry| entry.id().to_string())
                            .or_else(|| target.map(str::to_string)),
                        summary_entry,
                    },
                    OperationOutcome::Completed,
                    None,
                ),
                Err(error) if error.code == "aborted" => (
                    NavigationOutcome::Aborted {
                        leaf_id: source.clone(),
                    },
                    OperationOutcome::Aborted,
                    Some(error),
                ),
                Err(error) if error.code == "navigation_commit_pending" => {
                    // The move/result is already durable. Do not report a
                    // rejected navigation or roll history back: leave the intent
                    // open for idempotent recovery on reopen/the next operation.
                    return Err(HarnessError::io(error.message));
                }
                Err(error) => {
                    // An append failure after moving to the target must not leave
                    // the live lane on a branch the user was told failed.
                    self.session
                        .move_lane(&self.lane, source.as_deref())
                        .await
                        .map_err(|restore| {
                            HarnessError::io(format!(
                                "{}; could not restore the previous branch: {restore}",
                                error.message
                            ))
                        })?;
                    (
                        NavigationOutcome::Failed {
                            leaf_id: source.clone(),
                            error: error.clone(),
                        },
                        OperationOutcome::Failed,
                        Some(error),
                    )
                }
            },
        };
        // A failed finish leaves an open durable intent for recovery; do not
        // claim success if that acceptance record did not reach storage.
        self.write_operation_finished(&run_id, durable, error)
            .await
            .map_err(|error| {
                if durable == OperationOutcome::Completed {
                    HarnessError::io(navigation_commit_pending(error).message)
                } else {
                    error
                }
            })?;
        self.finish_context_writes(&run_id, None).await?;
        Ok(NavigationResult { run_id, outcome })
    }

    async fn prepare_navigation_summary(
        &self,
        entries: &[Entry],
        summary_id: Option<String>,
        source: Option<&str>,
        custom_instructions: Option<&str>,
        signal: &CancellationToken,
    ) -> Result<Option<ProvisionedEntry>, OperationError> {
        let Some(id) = summary_id else {
            return Ok(None);
        };
        let snap = self.snapshot_config().map_err(navigation_error)?;
        let provider =
            Self::resolve_provider(&snap.models, &snap.model).map_err(navigation_error)?;
        let context_window = if snap.model.context_window > 0 {
            snap.model.context_window as i64
        } else {
            128_000
        };
        if context_window < 2 {
            return Err(navigation_error(
                "Model context window is too small for branch summarization",
            ));
        }
        let mut options = GenerateBranchSummaryOptions::new(CompactionLlmOptions {
            provider,
            model: snap.model,
            api_key: None,
            signal: signal.clone(),
            thinking_level: Some(snap.thinking_level),
            retry: snap.retry.enabled.then_some(snap.retry),
            custom_instructions: custom_instructions.map(str::to_string),
        });
        // The shared preparer interprets a nonpositive budget as unlimited.
        // Keep at least one input token so a large reserve cannot accidentally
        // disable budgeting and send the entire abandoned branch.
        options.reserve_tokens = snap.branch_summary_reserve_tokens.min(context_window - 1);
        let summary = tokio::select! {
            biased;
            _ = signal.cancelled() => return Err(navigation_aborted()),
            result = generate_branch_summary(entries, &options) => result.map_err(|error| {
                if error.code.as_str() == "aborted" { navigation_aborted() }
                else { navigation_error(error.message) }
            })?,
        };
        Ok(Some(ProvisionedEntry {
            id,
            kind: ProvisionedKind::BranchSummary {
                from_id: source.unwrap_or_default().to_string(),
                summary: summary.summary,
                details: Some(
                    serde_json::to_value(BranchSummaryDetails {
                        read_files: summary.read_files,
                        modified_files: summary.modified_files,
                    })
                    .map_err(navigation_error)?,
                ),
                usage: summary.usage,
            },
        }))
    }

    async fn commit_navigation(
        &self,
        run_id: &str,
        target: Option<&str>,
        summary: Option<ProvisionedEntry>,
        label: Option<&str>,
        signal: &CancellationToken,
    ) -> Result<Option<Entry>, OperationError> {
        let _gate = self.message_writes.lock().await;
        if signal.is_cancelled() {
            return Err(navigation_aborted());
        }
        // Commit starts here. A later Escape does not roll back an accepted
        // branch. Recovery uses the source, target and reserved summary ID.
        if let Some(active) = self.inner.lock().unwrap().active_run.as_mut() {
            active.finishing = true;
        }
        if let Some(summary) = &summary {
            self.write_write_deferred(run_id, summary)
                .await
                .map_err(navigation_error)?;
        }
        self.session
            .move_lane(&self.lane, target)
            .await
            .map_err(navigation_error)?;
        let summary = match summary {
            Some(summary) => {
                let id = summary.id.clone();
                let committed = self
                    .session
                    .append_entry(summary, &self.lane)
                    .await
                    .map_err(navigation_error)?;
                self.write_step_attempt(run_id, StepKind::BranchSummary, 1, &id, None)
                    .await
                    .map_err(navigation_commit_pending)?;
                Some(committed)
            }
            None => None,
        };
        if let (Some(id), Some(label)) = (
            summary.as_ref().map(|entry| entry.id()).or(target),
            label.filter(|label| !label.is_empty()),
        ) {
            self.view()
                .set_label(id, Some(label))
                .await
                .map_err(navigation_commit_pending)?;
        }
        Ok(summary)
    }

    /// A crash before the result entry commits restores the source. A fully
    /// committed result is completed in place, without another provider call.
    pub(super) async fn recover_navigation(
        session: &Session,
        lane: &str,
        operation: &OperationStartedRecord,
    ) -> HarnessResult<bool> {
        let OperationIntent::Navigation {
            target_id,
            summary_entry_id,
            label,
            ..
        } = &operation.intent
        else {
            return Ok(false);
        };
        let view = session.view(lane);
        let leaf = view.get_leaf_id().await.map_err(session_to_harness_err)?;
        let complete = if let Some(id) = summary_entry_id {
            match view.get_entry(id).await.map_err(session_to_harness_err)? {
                Some(Entry::BranchSummary(entry)) => {
                    leaf.as_deref() == Some(id.as_str())
                        && entry.base.parent_id == *target_id
                        && Some(entry.from_id.as_str()) == operation.source_leaf_id.as_deref()
                }
                _ => false,
            }
        } else {
            leaf == *target_id
        };
        if complete {
            if let Some(id) = summary_entry_id {
                let records = session
                    .find_records(&RecordQuery::default())
                    .await
                    .map_err(session_to_harness_err)?;
                if !records.iter().any(|record| {
                    matches!(record,
                        LaneRecord::StepAttempt(step)
                            if step.run_id == operation.base.id && step.result_entry_id == *id
                    )
                }) {
                    session
                        .append_record(LaneRecord::StepAttempt(StepAttemptRecord {
                            base: RecordBase {
                                id: session.id_generator().next(),
                                seq: 0,
                                lane: lane.to_string(),
                                timestamp: 0,
                            },
                            run_id: operation.base.id.clone(),
                            step: StepKind::BranchSummary,
                            attempt: 1,
                            result_entry_id: id.clone(),
                            compaction_reason: None,
                        }))
                        .await
                        .map_err(session_to_harness_err)?;
                }
            }
            if let (Some(id), Some(label)) = (
                summary_entry_id.as_deref().or(target_id.as_deref()),
                label.as_deref().filter(|label| !label.is_empty()),
            ) {
                view.set_label(id, Some(label))
                    .await
                    .map_err(session_to_harness_err)?;
            }
        } else {
            session
                .move_lane(lane, operation.source_leaf_id.as_deref())
                .await
                .map_err(session_to_harness_err)?;
        }
        session
            .append_record(LaneRecord::OperationFinished(OperationFinishedRecord {
                base: RecordBase {
                    id: session.id_generator().next(),
                    seq: 0,
                    lane: lane.to_string(),
                    timestamp: 0,
                },
                run_id: operation.base.id.clone(),
                outcome: if complete {
                    OperationOutcome::Completed
                } else {
                    OperationOutcome::Aborted
                },
                error: (!complete).then(|| OperationError {
                    code: "interrupted".into(),
                    message: "Interrupted branch navigation restored its previous position".into(),
                }),
            }))
            .await
            .map_err(session_to_harness_err)?;
        Ok(true)
    }
}
