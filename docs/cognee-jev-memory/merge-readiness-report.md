# Cognee + JEV merge-readiness report

**Recommendation: NOT MERGE-READY against the requested definition of done.** The feature is reviewable, but the 25-task acceptance ledger still contains partial or blocked criteria. Synthetic evaluation is not live-model or production evidence. Do not enable JEV based on this report.

## Branch and scope

- Integration branch: `feat/cognee-memory-completion`. Tested implementation HEAD: `b3c71e883b780a15bdc57b512d70be62657ba67f`. This report and the measured replay are documentation/evidence commits on top of that implementation; use `git rev-parse HEAD` for the exact containing commit, avoiding a self-referential commit hash.
- Baseline: `main` at `dd0e2405eca8f09648882d387bbe19a543db9c27`, the merge base. The feature diff before these final evidence edits spans approximately 99 files, 22,157 additions and 832 deletions; it is substantially larger than the repository's normal review-size guideline and deserves staged human review.
- The immutable plan is in the original planning worktree at `.worktrees/extra-fx-bug-fix/docs/cognee-jev-memory-plan.md`, SHA-256 `0760aed89a52107e7b109f030fbd7608011292bbdcd3068271ca4cb8fe619366`. The 25-task ledger with source, test, integrated SHA and unresolved acceptance gaps is [`completion-evidence-matrix.md`](completion-evidence-matrix.md). No claim is made that M0–M7 are all complete.

## Integrated work

- Request-boundary provenance and persistence: `7a789f4c`, `0a61e217`. Original user intent remains distinct from skill/template expansion; packets are acknowledged after persistence and stale/duplicate/oversized packets fail closed.
- Source-backed selective admission: `440edddc`, `6297a800`; valid candidate source, ownership and fresh evidence are checked before helper inference and acceptance. Helper errors are nonfatal and cached verdicts remain advisory.
- Receipt-backed admission and recovery: `5e289a87`, `03bb097a`, `8d479121` (durable completion branch `e0abd5e0`); immutable receipt is the commit point, queue reservations are not dispatched before receipt, completed projections are recoverable, and uncertain wire writes are not automatically replayed. Legacy accepted records remain readable. Dispatch and admission have cross-process locks.
- Bounded optional helper: `ec4bcee9` and integrated changes from the helper branch. Its remaining code differences are formatting/test visibility or overlap older runtime code; replacing current runtime with that branch would discard later source and receipt safety.
- Live capability catalog, miss and compaction coverage: `17db78cc` (capability branch `5700d08f`); synthetic chronological replay: `3173b8b4`, `44e5ac4e` (replay branch `68c556c6`, `9ce948d9`). Suggestions are advisory and do not execute tools.
- Replay accounting correction: `82dc9ed`. Negated-history admission fix and regression: `b3c71e8`. The latter closes the observed false positive for the authored self-contained cohort; it does not establish general semantic negation understanding.

## Verification

All commands used isolated `CARGO_TARGET_DIR=$PWD/.target-completion`, `CARGO_BUILD_JOBS=2`, and `ICERS_CODING_AGENT_DIR=/tmp/ice-cognee-final-review-20261028/agent`. Full local logs are under `/tmp/ice-cognee-final-review-20261028/`. No live model, JEV, Cognee service or user agent data was used.

| Command | Result on tested implementation |
|---|---|
| `cargo fmt --all -- --check` | Pass |
| `RUSTFLAGS='-D warnings' cargo check --workspace --all-targets --locked` | Pass, `final-check.log` |
| `RUSTFLAGS='-D warnings' RUSTDOCFLAGS='-D warnings' cargo test --workspace --locked` | Pass: 3,396 passed, 0 failed, 7 ignored, 92 suites; `final-workspace.log` |
| `cargo test -p ice-cli --lib cognee:: --locked` with `RUSTFLAGS='-D warnings'` | 226 passed, 1 ignored; `final-cognee.log` |
| `cargo test -p ice-cli --lib capability --locked` with `RUSTFLAGS='-D warnings'` | 46 passed; `final-capability.log` |
| `cargo test -p ice-cli --lib cognee::policy --locked` | 4 passed |
| `cargo test -p ice-harness --test harness_extensions --locked` with `RUSTFLAGS='-D warnings'` | **Failed before running tests**: seven dead-code errors in `ice-ai::strict_schema`; `final-harness.log` |
| `cargo test -p ice-harness --test harness_extensions --features ice-ai/providers --locked` with `RUSTFLAGS='-D warnings'` | 17 passed, exit 0; `final-harness-providers.log`. An initial invocation's wrapper exited 1 after Cargo passed because `status` is a readonly zsh variable; the corrected invocation passed. |
| `cargo test -p ice-cli --lib chronological_offline_replay_report --locked -- --ignored --nocapture` with `ICERS_MEMORY_REPLAY_REPORT` | Pass, 56 authored cases/arm; `evidence-replay.log` |
| `sh scripts/sync-embedded-docs.sh` and `git diff --exit-code -- crates/ice-cli/embedded-docs` | Pass |
| `python3 scripts/test_prebuilt_release.py` | 4 passed, `prebuilt.log` |
| `git diff --check` | Pass before this documentation commit; rerun after staging |

The earlier replay accounting attempt failed an overbroad equality assertion (21 vs 5 misses); it was corrected to assert the intended on-demand historical cohort and rerun successfully. Seven workspace tests remain ignored, including the separately executed opt-in report generator. Full workspace tests include the helper, capability and receipt regressions; the earlier focused helper run selected 12 tests.

The harness-only failure uses the narrower default `ice-ai` feature set: `strict_schema` is compiled without its provider consumers, and warning denial makes its unused symbols errors. The module, its declaration and feature definitions are unchanged from `main`; the integration branch adds an unrelated `sha2` dependency to that manifest. This is evidence of a pre-existing feature-configuration issue, not a successful standalone warning-denied gate. The provider-enabled targeted run and the full workspace run both pass.

## Offline evaluation

[`offline-evaluation.md`](offline-evaluation.md) and [`evidence/offline-replay.json`](evidence/offline-replay.json) contain the exact corpus hash, denominators, counts and local fake-transport timings. For 56 cases each: memory-off made 0 fake requests and missed 33 required facts; legacy made 56 requests and delivered 20 forbidden synthetic facts; selective with JEV off made 33 requests, missed 5 required facts, and delivered no forbidden synthetic facts. On-demand made 12 explicit lookups and retained 12 facts; it was invoked only in its historical cohort, so its 21 total misses are not a prediction of real agent use. The mock-JEV arm does **not** invoke a judge and is not evidence of JEV quality. Warm local admission (1,000 samples) had zero fake requests and p50 30 µs, p95 38 µs, max 106 µs; no TTFT, production backend latency, background latency, task usefulness, automatic-admission false-positive rate or real-model quality was measured. These authored cases are not held-out or representative user evidence.

## Defaults, privacy, recovery and rollback

The default read and write policies are legacy. JEV is off by default and requires explicit helper enablement, provider and model. Canary mode uses event reads and manual writes; helper dispatch does not run under manual writes, but the mode change can preserve the helper-enabled setting. Legacy, manual, canary and selective modes remain selectable. Environment policy and helper overrides take precedence over saved settings and must be checked when changing modes. Current-source and task checks fail closed before optional judgment; helper quotas, cancellation and receipt-backed recovery are exercised by deterministic tests. Results do not establish production filesystem durability, real service searchability or mixed-version deployment correctness.

For a local rollback, disable the helper first in Cognee settings, then choose `/cognee mode manual` for on-demand reads and explicit writes, or disable Cognee entirely; inspect effective environment overrides and cancel/shut down pending optional work. Legacy mode restores historical broader automatic behavior and is not equivalent to manual safety. Preserve datasets, receipt files and uncertain queue entries; do not delete or automatically resend uncertain writes. Reverting a deployed binary or schema requires a separately reviewed backup and compatibility plan. No real shadow/canary or rollback drill has been performed.

## Outstanding acceptance blockers

1. The 25-task ledger lists multiple partial requirements; current tests do not prove all host lifecycle, cross-repository, metadata, cancellation and writer-policy boundaries.
2. JEV judgment, held-out utility, real service/provider latency, accepted-to-searchable status and end-to-end task usefulness remain unverified; the restriction prohibits use of any other model and this evaluation made no live requests.
3. A local-only shadow/canary and rollback drill, operational monitoring and mixed-version rollout verification remain outstanding. Preserve existing data during any such exercise.
4. The local build directory `.target-completion/` is untracked and intentionally preserved rather than deleted. A clean-worktree claim must account for it. No PR was opened or linked.

No protected branch was merged, no release was published, and no production deployment is recommended from the current evidence.
