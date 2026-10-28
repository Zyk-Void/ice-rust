# Cognee + JEV completion audit

## Scope and immutable baseline

Baseline commit: `d912a00ebce86d019c6934b7c56042f402eee8e5` (`feat/cognee-helper`). Implementation worktree: `.worktrees/cognee-memory-completion`, branch `feat/cognee-memory-completion`.

Authoritative local plan: `.worktrees/extra-fx-bug-fix/docs/cognee-jev-memory-plan.md`, SHA-256 `0760aed89a52107e7b109f030fbd7608011292bbdcd3068271ca4cb8fe619366`. Its 25 task IDs and acceptance requirements are the frozen denominator. Private historical reports and evidence remain untracked in the original worktree; they are not copied into commits. This audit contains no transcript excerpts or credentials.

The user explicitly requested completion of all remaining plan work. This overrides the completion skill's usual >85% admission threshold; it does not waive tests or acceptance requirements. With strict whole-task scoring, the baseline is 1 complete, 19 partial, and 5 missing: `(1 + 19 * 0.5) / 25 = 42%`. This measures acceptance coverage, not lines of code or effort. Substantial implementation is present, but several libraries are unconsumed and important host contracts are missing. Later status must retain this denominator.

## Repository/worktree reconciliation

`main` (`dd0e2405eca8f09648882d387bbe19a543db9c27`) is an ancestor of the integration baseline. The integration source is clean, with untracked original planning/audit evidence and tool-cache files. Existing source and worktrees are preserved.

Patch-equivalence checks identify exactly one additional commit on each of two related clean worktrees:

- `t3/cognee-helper-gate`, `4b86227f48d020dbe53c2fd102901b6016951ac4`: optional JEV candidate admission, cache, concurrency and tests.
- `t3/cognee-suppression-ledger`, `bcc6e71112c6f4adc054e08c89c1d8364260d712`: local expiry/supersession suppression and tests.

Other related milestone branch work is already represented in the integration history. Do not replay it merely because original commit IDs differ. Unrelated worktrees, including a prunable old review worktree, must not be modified or pruned.

## Verified baseline

`cargo test -p ice-cli --lib --locked cognee::` selected 154 tests, all passed, 0 ignored, 1,253 filtered out; test execution 1.25 s. Build plus test command 48.9 s. This is a fresh fake-transport test result, not real backend latency or effectiveness. No live model, Cognee, or JEV request was made.

## Integration update (2026-10-28)

The completion branch now integrates request-boundary packet persistence/acknowledgment, receipt-backed admission/recovery, optional helper admission limits and fresh source ownership checks, capability catalog event/compaction coverage, and synthetic chronological replay. The current-source admission regression validates strict source attribution and task ownership before helper IO and again before receipt-backed acceptance. The offline replay uses 56 authored synthetic cases across five deterministic arms; it is not a held-out user-quality study and its mock-JEV arm does not test model judgments.

Final integrated verification passed: format check; warning-denied workspace all-target check; warning-denied workspace tests (3,395 passed, 7 ignored); embedded-doc synchronization and no-diff check; prebuilt release tests (4 passed); and `git diff --check`. Focused tests passed: 12 helper, 46 capability, 1 chronological replay. The ignored replay-report generator also passed. A stale Codex tool-list expectation caught in an earlier workspace run was corrected before the final pass. No live service/model was contacted. No production usefulness, latency, searchability, or release claim is made.

The 25-task denominator and baseline score remain unchanged; see `completion-evidence-matrix.md` and `merge-readiness-report.md`. Code review readiness is distinct from production or release readiness.

## Principal gaps confirmed at baseline

The following are historical baseline findings, not claims about current source. Subsequent fixes are recorded above and in `implementation-progress.md`; outstanding verification remains in the task ledger.

1. `BeforeRunEvent` has only expanded prompt/system/session/cancellation fields. It cannot convey original intent, active messages, model window, task identity, conversation branch or request-boundary identity.
2. `coordinator.rs::take` marks results delivered before persistence, discards structured provenance by rebuilding a result with `metadata: None`, and is consumed only at prompt admission. Its prefetch selector does not apply contextual expiry/branch filters.
3. `evidence.rs` records novelty before budget fitting/persistence, allows a branch-specific result when the current branch is unknown, and treats malformed expiry as usable unknown validity. Actual current context is not consulted.
4. Repository identity shares datasets but does not carry Git branch/revision/worktree applicability into candidates or retrieval.
5. Candidate acceptance trusts a successful unrelated tool as verification, persists candidate attribution after queue dispatch, and ignores a false candidate-store capacity return. Accepted candidates are not yet a reliable resident evidence source.
6. The queue has durable intent and uncertainty but no cross-process drain ownership. Startup recovery may race another live process.
7. JEV helper transport exists but is not wired at baseline. The extra gate branch adds wiring, but its fixed model, reset behavior, shared quotas and cancellation must be reviewed after integration.
8. `capability_catalog`, `skill_coverage`, and `capability_suggest` have tests but no production consumer of the coverage/suggestion APIs. They do not yet satisfy task-event delivery requirements.
9. Rollout/evaluation artifacts and full final repository checks are missing. No local selective utility or remote JEV quality is established.

## Verification policy

Use synthetic/faux transports and isolated temporary agent directories. Any live inference would require the exact authorized `cx/gpt-6-luna`; JEV itself must not be called under that restriction. Implement and verify its integration offline, keep it off by default, and report its quality/latency gate as unverified rather than manufacturing measurements. No writes to the real agent directory, no dataset deletion, no protected-branch merge, no release or tag.

See `completion-evidence-matrix.md` for frozen requirements and `completion-remaining-fixes.md` for execution order. Final readiness requires code wiring, failure-path tests, a reviewable committed tree, and explicit separation of implementation correctness from production effectiveness.
