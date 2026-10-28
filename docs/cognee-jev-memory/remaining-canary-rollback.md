# M7.3 isolated canary and rollback procedure

- Date: 2026-10-28
- Status: **EXECUTED isolated synthetic mixed-version compatibility and backup/restore proof; BLOCKED only for production rollout acceptance**.
- No real stores, `~/.icers`, `~/.ice`, TypeScript ICE, remote service, live MCP, real model or credential was read or contacted. All fixture profiles and sources lived under the private `target/cognee-remaining-validation/` directory; the test runner used a temporary `ICERS_CODING_AGENT_DIR`.
- Isolated proof establishes reader compatibility and local restore mechanics only. It is not a production canary, remote-write, model-quality, or deployment rollback acceptance.

## Provenance and existing local exercise coverage

The test fixture `runtime()` uses `tempfile::tempdir()` as its agent directory, stores Cognee state at `<temp>/ice-cognee`, supplies an explicit local `CogneeEnv` config and fake transport, and uses a fixed clock. The chronological runner uses one new temporary runtime/profile per case. Local regression coverage already includes:

| Exercise | Existing proof |
|---|---|
| Off / helper default off / zero service calls | `helper::tests::binding_off_does_not_construct_a_client`; `runtime::tests::helper_off_leaves_pending_and_prompt_hook_never_judges`; `offline_replay_tests::chronological_replay_regressions_temporal_boundary_zero_calls_and_denominators` (1,000 ordinary event turns, no fake requests) |
| Manual and event reads | `runtime::tests::on_demand_read_policy_never_searches_automatically`; `runtime::tests::event_read_policy_searches_only_for_history_requests`; `runtime::tests::event_recall_injects_only_new_evidence_and_reinjects_after_compaction` |
| Self-contained prompts | `request_tests::request_boundary_plain_prompts_and_unknown_skill_origin_have_no_network_work`; chronological synthetic self-contained cohort |
| Explicit history, same-task continuation and evidence acknowledgement | `request_tests::request_boundary_uses_raw_intent_for_explicit_history_and_shared_small_model_budget`; `request_boundary_prefetch_is_same_task_and_consumed_only_after_ack`; `request_boundary_cancellation_keeps_evidence_unconsumed` |
| Compaction, session change and stale prefetch | `runtime::tests::compaction_does_not_use_summary_as_an_unconditional_query`; `context_generation_tracks_identity_but_not_compaction_or_repeated_start`; `session_change_discards_stale_prefetch_generation`; compaction reinjection test above |
| Shutdown and cancellation | `runtime::tests::slow_helper_is_background_and_shutdown_cancels_it`; `closeout_runtime_tests::switching_off_cancels_inflight_optional_capture_and_preserves_uncertainty`; `request_tests::request_boundary_cancellation_keeps_evidence_unconsumed` |
| Manual writes versus automatic writes | `runtime::tests::manual_remember_still_sends_under_selective_policy`; `runtime::tests::selective_policy_disables_all_legacy_writes_even_with_switches_on`; `manual_write_policy_disables_every_automatic_write` |
| Disable/rollback and uncertainty | `offline_replay_tests::isolated_mode_disable_and_rollback_preserve_uncertain_records`; `closeout_runtime_tests::queued_improve_cannot_revive_after_disable_and_reenable_before_first_poll`; `ambiguous_cache_capture_is_preserved_but_never_replayed_automatically` |
| Acceptance vs searchability | `closeout_runtime_tests::manual_accepted_job_handle_is_retained_with_unknown_searchability`; accepted receipt tests retain `searchability: unverified` |

These are unit/integration tests with synthetic profiles and do not prove live user-facing benefit, production rollback or service behavior. All fixture cases are not independently labeled real conversations. The frozen 56-case synthetic policy replay in `evidence/offline-replay-cases.json` is not production evidence.

## Isolated profile setup and diagnostics

The following commands are examples for a new disposable local drill. Choose a new absolute `DRILL_ROOT` outside any user profile, do not reuse or export an active `~/.icers` path, and do not put real credentials or user data in it. These commands configure no Cognee endpoint and make no remote request; the default helper remains off. Run a drill binary only after verifying its build revision and environment. Use a disposable project copy too, not the working tree containing user work.

```sh
set -eu
export DRILL_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/icers-cognee-drill.XXXXXX")"
chmod 700 "$DRILL_ROOT"
export ICERS_CODING_AGENT_DIR="$DRILL_ROOT/profile"
mkdir -m 700 "$ICERS_CODING_AGENT_DIR"
mkdir -p "$DRILL_ROOT/project" "$DRILL_ROOT/logs"
printf '%s\n' "$DRILL_ROOT" > "$DRILL_ROOT/drill-root.txt"
```

The explicit override points at the agent directory itself, not its parent. Runtime Cognee state defaults to `$ICERS_CODING_AGENT_DIR/ice-cognee/`; the Cognee runtime test fixture also uses a separate `<temp>/ice-cognee/config.json`. Do not set Cognee URL or API-key variables for a zero-request synthetic drill. For any later service-enabled drill, stop here until the endpoint/model/data/budget approvals are written, then point only at a separately authorized isolated endpoint and dataset.

Before enabling tools, record the effective mode and directory without printing secrets:

```sh
printf 'profile=%s\nproject=%s\n' "$ICERS_CODING_AGENT_DIR" "$DRILL_ROOT/project"
printf 'agent_dir_override=%s\n' "$ICERS_CODING_AGENT_DIR"
find "$ICERS_CODING_AGENT_DIR" -maxdepth 3 -type f -printf '%P\n' | sort
```

At startup and after each mode transition, inspect the CLI's read-only `/cognee status`, `/cognee mode status`, `/cognee helper status`, and `/cognee doctor` output. Redact endpoint credentials before retaining logs. The doctor reports capability inventory as unprobed/last observed; do not treat that as a live service check. Assert helper enabled is false and mode is the intended manual/canary setting before ordinary prompts. Check the fake transport/request counter or an inference-disabled adapter; do not infer zero requests from absence of a UI badge.

To run the temporary-profile behavior regressions with isolated process state and complete logs, from repository root:

```sh
set -euo pipefail
export CARGO_TARGET_DIR="$PWD/target/cognee-remaining-validation"
mkdir -p "$CARGO_TARGET_DIR"
export CARGO_BUILD_JOBS=2
export RUSTFLAGS='-D warnings'
export RUSTDOCFLAGS='-D warnings'
export ICERS_CODING_AGENT_DIR="$(mktemp -d "${TMPDIR:-/tmp}/icers-cognee-test-profile.XXXXXX")"
chmod 700 "$ICERS_CODING_AGENT_DIR"
cargo test -p ice-cli --locked cognee::runtime::tests::slow_helper_is_background_and_shutdown_cancels_it -- --nocapture 2>&1 | tee "$CARGO_TARGET_DIR/m7-shutdown.log"
cargo test -p ice-cli --locked cognee::runtime::tests::event_read_policy_searches_only_for_history_requests -- --nocapture 2>&1 | tee "$CARGO_TARGET_DIR/m7-event-read.log"
cargo test -p ice-cli --locked cognee::runtime::tests::on_demand_read_policy_never_searches_automatically -- --nocapture 2>&1 | tee "$CARGO_TARGET_DIR/m7-manual-read.log"
cargo test -p ice-cli --locked cognee::runtime::tests::manual_remember_still_sends_under_selective_policy -- --nocapture 2>&1 | tee "$CARGO_TARGET_DIR/m7-manual-write.log"
cargo test -p ice-cli --locked cognee::offline_replay_tests::isolated_mode_disable_and_rollback_preserve_uncertain_records -- --nocapture 2>&1 | tee "$CARGO_TARGET_DIR/m7-rollback.log"
```

The command block uses Bash `pipefail` so a failed test fails the block while `tee` retains full output; each command's complete log includes its test result and exit status. Do not remove test profile state until logs and evidence are preserved. The complete per-file state is under the newly created profile/fixture; these are not the results of executing the commands in this document.

## Backup and restore: exact commands for an isolated profile only

First stop the test process and every process sharing the drill profile. A filesystem archive is a snapshot, not reconciliation with an external service. Do not delete an old Cognee dataset to roll back. Use a destination that does not already exist, keep both paths below the disposable drill root, and verify the archive before any restore.

```sh
set -eu
# Set DRILL_ROOT to the recorded new temporary directory from setup.
DRILL_ROOT="${DRILL_ROOT:?set DRILL_ROOT to the isolated drill root}"
PROFILE="$DRILL_ROOT/profile"
BACKUP="$DRILL_ROOT/backup-before-mode-change.tar"
RESTORE="$DRILL_ROOT/restore-check"
test -d "$PROFILE"
test ! -e "$BACKUP"
tar --create --file "$BACKUP" --directory "$DRILL_ROOT" profile
sha256sum "$BACKUP" | tee "$DRILL_ROOT/backup-before-mode-change.sha256"
tar --list --file "$BACKUP"
```

For a non-destructive restore check, extract to a fresh sibling copy, never over the running profile:

```sh
set -eu
DRILL_ROOT="${DRILL_ROOT:?set DRILL_ROOT to the isolated drill root}"
BACKUP="$DRILL_ROOT/backup-before-mode-change.tar"
RESTORE="$DRILL_ROOT/restore-check"
test -f "$BACKUP"
test ! -e "$RESTORE"
mkdir -m 700 "$RESTORE"
tar --extract --file "$BACKUP" --directory "$RESTORE"
test -d "$RESTORE/profile/ice-cognee" || test ! -d "$DRILL_ROOT/profile/ice-cognee"
diff -qr "$DRILL_ROOT/profile" "$RESTORE/profile"
```

To restore an isolated profile after stopping all writers, preserve the changed profile under a new name before restoring into a fresh expected path. Never use `tar` extraction over a populated directory:

```sh
set -eu
DRILL_ROOT="${DRILL_ROOT:?set DRILL_ROOT to the isolated drill root}"
BACKUP="$DRILL_ROOT/backup-before-mode-change.tar"
PROFILE="$DRILL_ROOT/profile"
CHANGED="$DRILL_ROOT/profile-after-drill"
RESTORE_TMP="$DRILL_ROOT/restore-tmp"
test -f "$BACKUP"
test -d "$PROFILE"
test ! -e "$CHANGED"
test ! -e "$RESTORE_TMP"
mv "$PROFILE" "$CHANGED"
mkdir -m 700 "$RESTORE_TMP"
tar --extract --file "$BACKUP" --directory "$RESTORE_TMP"
test -d "$RESTORE_TMP/profile"
mv "$RESTORE_TMP/profile" "$PROFILE"
rmdir "$RESTORE_TMP"
DIFF_STATUS=0
diff -qr "$PROFILE" "$CHANGED" > "$DRILL_ROOT/restore-vs-drill.diff" || DIFF_STATUS=$?
test "$DIFF_STATUS" -le 1
printf 'restore comparison status=%s (0=same, 1=expected differences)\n' "$DIFF_STATUS"
```

The final diff is expected to show drill changes versus the restored snapshot and is diagnostic, not a success criterion; inspect `$DRILL_ROOT/restore-vs-drill.diff` before proceeding. A `diff` status above 1 is an operational error and stops the procedure. Re-run read-only status/doctor and verify pending, warmup, candidates/admissions, suppression records, accepted-job records, uncertainty states and dataset configuration are present. Do not remove `$CHANGED`, backup archive, logs, or uncertain records until the owner explicitly decides their retention. Never apply these commands to a non-disposable user profile.

## Executed isolated mixed-version compatibility proof

The earlier gate incorrectly described isolated mixed-version acceptance as needing new authorization. The isolated copy and temporary profiles were already explicitly authorized. This section records the completed proof; production rollout remains a different gate.

### Exact source and build provenance

- Comparison base: `b87d10c073af0dffaed1f3cf4fd54ce641fb27a7` (ancestor of tested checkout `0658512a7221daceb62e98e5d5c4a689486252d3`).
- Historical `crates/ice-cli/src/cognee/queue.rs` SHA-256: `bc4385850069fe1cfe4931ea4ce9a07243739701e799aba04ba0f44fa2deaaa7`.
- Current `crates/ice-cli/src/cognee/queue.rs` SHA-256: `de6257c1b69183e6a9e08da1ccf65213b9e8cec915d1e4caf6430b7c6334d03b`.
- Historical source was copied byte-for-byte from `git show <base>:crates/ice-cli/src/cognee/queue.rs`; the harness supplied only the historical module's two local filesystem helpers and called the actual `read_pending_remember`, `select_drain_batch`, and `read_warmup_batch` functions. Current harness used the exact current queue module and current `read_replayable_warmup_batch` function. Both built with `RUSTFLAGS='-D warnings'`, `--locked --offline`, jobs=2, under the private target. Harness logs, source snapshots, checksums, and exit records are retained in `target/cognee-remaining-validation/`.
- Historical `crates/ice-cli/src/cognee/runtime.rs` SHA-256 at the selected base: `04efc6af61e32b86ff5462ae89c70dbb6da4f8ee1d10f73832ce80112c71753f`.

### Synthetic fixture results

The disposable fixture contained permanent queue JSON records in `uncertain`, `inflight`, and `pending` states and matching warmup records, with synthetic IDs/content only. Before and after each reader execution, SHA-256 was recorded for every queue file; the files were byte-identical. Both readers decoded all three permanent states. For `select_drain_batch(false)`, both returned only the `pending` record (1 selected); `select_drain_batch(true)` returned all 3, including `uncertain` and `inflight`. The historical runtime calls its automatic `spawn_drain_pending()` with `false`; `true` is the explicit `/cognee flush uncertain` path. Thus the tested older permanent-queue automatic reader did **not** replay uncertainty in this source revision. The warning is specifically that explicit uncertain flush is deliberately replayable and an operator should not invoke it during rollback without reconciliation/authorization.

The older warmup reader decoded all 3 records without filtering state; its runtime passed the batch to a write loop. The current warmup reader returned only the 1 `pending` record; both `uncertain` and `inflight` are excluded. Therefore rolling back to the older runtime with a nonempty ambiguous warmup queue is an actual replay hazard: its next eligible warmup drain could resend those entries. Keep writes disabled and do not start the older runtime against a store containing uncertain/inflight warmup records; archive and reconcile first. This finding comes from execution of the historical source and its runtime call paths, not inferred unknown-enum behavior.

Run output is preserved as `old-reader-run.log` and `current-reader-run.log`; exact fixture file hashes are in `mixed-fixture.before.sha256`, `mixed-fixture.after.sha256`, `current-reader-after.sha256`, and the corresponding `*-unchanged.log` comparisons. No network-capable code or HTTP client was included in either harness.

### Executed backup / restore state-hash drill

A separate synthetic store contained pending and warmup `uncertain`/`inflight` records, candidate and admission-receipt files, an accepted-job receipt, history/observation evidence, and a config file. The disposable copy was backed up, altered only in its drill copy, then restored from backup. A deterministic recursive hash over relative paths and full file contents matched before/backup/restored: `0dcdf5395c5b5623fa77a57e86248344d4f708cc0c3678441fce1b3c2745ea27`; 9 files restored. Records were not normalized, replayed, edited, or deleted. `backup-restore-compare.log` records the successful byte/hash equality.

The new focused current-binary regression tests also prove that (a) the automatic selectors omit uncertain/inflight records, (b) a full current runtime's automatic pending and warmup drain makes zero fake-transport requests and preserves queue file hashes while retaining synthetic history/candidate/receipt/job records, and (c) a stored `enabled=false`, `writePolicy=manual` configuration disables the runtime and leaves a full synthetic store hash unchanged. Test stdout and exit records are retained in the private validation target; the final workspace gates below are the authoritative combined-source verification.

## Mixed-version rollback boundary

The drill is scoped to an isolated synthetic profile; no new authorization was needed or requested for this proof. Never point an old executable at the actual store or a production profile. A production rollback is not proven by this fixture. The older warmup hazard above means the safe rollback choice is to stop the old runtime from dispatching, keep the current compatible binary/configuration available, retain a verified backup, and reconcile ambiguous warmup operations before any older replay path is enabled.

## Stop, disable and preserve uncertainty

1. Stop issuing prompts; cancel/shut down the runtime and wait for its tasks to finish. For service-enabled work, stop the isolated service work queue if independently authorized.
2. Using the compatible current binary, issue `/cognee off` to disable all reads and writes; `/cognee write manual` is the safer write-only pause when reads must continue. Check `/cognee status` and the stored `enabled`/`writePolicy` values. Do not use `/cognee improve now` or `/cognee flush uncertain` as rollback shortcuts; do not switch to Legacy.
3. Preserve the complete `$ICERS_CODING_AGENT_DIR/ice-cognee/` tree, Cognee config, profile, project copy, effective config (redacted), logs, accepted-job handles, pending/warmup state and every uncertain record. Copy/archive before any repair. The isolated regression verifies `enabled=false` plus `writePolicy=manual` blocks runtime availability and preserves store hashes.
4. Treat HTTP accepted, locally acknowledged, indexed/searchable and used as distinct states. Keep ambiguous writes uncertain; no automatic retry, hand-edit to Pending, or claim of exactly-once delivery.
5. If any external service was authorized, reconcile its operation/job status only through a documented endpoint and owner approval. Unknown status is retained as unknown.

A real canary additionally requires an owner-designated isolated project/profile/dataset, live model and service authorization, privacy and budget approvals, representative independently labeled tasks, and an operator with a tested stop/restore plan. Mixed-version proof remains isolated-copy-only. None is established by synthetic tests or this runbook.