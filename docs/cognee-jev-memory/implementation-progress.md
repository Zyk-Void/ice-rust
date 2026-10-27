# Cognee + JEV implementation progress

Baseline: `09a728cdc5b98e270b8f01e38d2575ca047d45ff` on `feat/cognee-helper`.

## Baseline verification

- `cargo fmt --all -- --check`: passed.
- `cargo test -p ice-cli --locked cognee --lib`: passed, 76 tests; 0 failed; 1.25 s test execution (cold build 1m 17s). These are not end-to-end latency measurements.
- Historical measured recall and evaluation limitations are recorded in the existing observability audit and evaluation protocol. No live model or remote probe was run.
- The historical evidence JSON and some reports contain private session excerpts or descriptions. They remain untracked locally and must not be committed or published without a privacy-safe version. Original files have not been altered.

## Task ledger

| Tasks | Status | Evidence / blockers |
|---|---|---|
| M0.1 | In progress | Versioned recall skip/nonempty/empty and improvement-acceptance metadata added in `crates/ice-cli/src/cognee/runtime.rs`; skipped recall distinguishes unavailable, disabled, circuit-open, short-prompt and duplicate reasons, and legacy phases are preserved. Manual improvement is observed and the TUI labels improvement acceptance. Recorder titles now distinguish versioned skips, retrievals and accepted jobs while preserving legacy titles. Recall terminal metadata records actual remote wait and original source/session context, including errors. A session/dataset fence rejects late recall after identity changes; fake delayed transport verifies this path. A harness callback records whether context was appended, its byte count and a clearly labeled harness token estimate, not provider delivery or actual tokenizer counts; recall and injection share a request ID; a storage-failure harness fixture verifies that failed appends are not sent; full source/context attribution remains. Same-session task-generation fencing, complete late-completion attribution and full fixtures remain. |
| M0.2 | In progress | Doctor distinguishes the native recall endpoint and locally sent recall/improve fields from unverified server support, background improvement acceptance and an unprobed MCP catalog; endpoint credentials are stripped. It explicitly says completion status is not exposed by this adapter. Existing wire-shape fixtures cover recall and writes; new 404/422 fixtures verify no fallback retries. Per-operation observed outcomes and their fixtures remain. |
| M1.1 | In progress | `config::repository_root` reads the linked-worktree `.git` file and `commondir` (no subprocess), so a linked worktree resolves `$project` to its main checkout's dataset; a main checkout keeps its existing name. Fixture covers linked, nested, same-basename and unreadable cases. Remaining: `RepositoryIdentity` with worktree/branch/revision fields and branch-restricted facts. M1.2 adds bounded read-through for a linked worktree's older lexical dataset under non-legacy read policies. |
| M1.2 | In progress | Non-legacy automatic recall and tool search try the linked worktree's lexical dataset once after an empty successful current-dataset response, subject to the remaining budget; explicit datasets and legacy policy do not fall back. The linked-worktree fake transport fixture covers successful fallback and matching session/scope; a second fixture covers first-hit, first-error, explicit-dataset and legacy-policy bypass. Remaining: dedicated tool-search fallback and budget/cancellation fixtures, and fallback-error observation coverage. |
| M2.1 | In progress | `readPolicy` (`legacy`/`on_demand`/`event`) and `writePolicy` (`legacy`/`manual`) in config, env, `/cognee read\|write`, settings rows, status and doctor. `cognee/policy.rs` is a pure admission check; `event` searches only when the prompt asks about earlier work and otherwise sends no request, with a skip reason in the observation. `manual` disables session capture, tool traces, idle/shutdown improve, dream and compaction remember. Defaults stay legacy. Remaining: task-event producers (resume, compaction, explicit marker), original-intent metadata before skill expansion, `selective` write policy. |
| M2.2, M2.3 | In progress | Failed background captures retain their original session/dataset in warmup and terminal observations; warmup replay is single-flight and reports replay/failure counts. Remaining: resident cache, prefetch, context generation and generation-bound delivery, comprehensive concurrency tests; mid-task context append, persist-before-send writes, provenance/scope filters and JEV are out of scope. |
| M3.1 | In progress | Recall carries known string source, session and timestamp metadata into bounded tags in both recall paths. Evidence tests cover key variants and absent/non-string fields; a fake-transport prompt fixture checks mixed provenance and observation counts. Remaining: richer provenance and attribution beyond these fields and validation against a live backend. |
| M3.2 | In progress | `cognee/evidence.rs` (non-legacy read policies only): routine `bash succeeded`-style lines and lines already injected are dropped; the seen set resets on compaction and session change. Remaining: scope/expiry/supersession filters, which need M3.1 metadata. |
| M3.3 | In progress | Checkpoint and recall share one `recallMaxChars` budget; `format_recall` keeps the closing envelope marker when the budget allows. Applies to all policies. Remaining: token (not character) budget and a safe mid-task append boundary. |
| M4.1 | In progress | `manual_remember` now queues a timeout or abort as `uncertain` (never auto-replayed) instead of `pending`. The uncertain path is covered only through the shared `drain_failure_state` unit test: the 30 s remember timeout cannot be shortened in tests. Remaining: persist intent before sending, scope in the dedupe key, warmup ownership. |
| M4.2, M4.3, M4.4 | Not started | Candidates, supersession and accepted-versus-searchable reconciliation. `writePolicy=manual` is only the off switch, not the selective outbox. |
| M5.1, M5.2, M5.3, M5.4 | Not started | Optional JEV; no live JEV evaluation under the Luna-only restriction. |
| M6.1 | In progress | Local catalog prototype ranks supplied skill and MCP snapshots. Remaining: connect pre-deduplication skill registry and source classification, rebuild on resource/MCP refresh, and complete acceptance tests. Existing `skillDirs` setting supports explicit extra paths. |
| M6.2, M6.3 | Not started | Capability discovery and skill suggestions. |
| M6.4 | In progress | Native search propagates disabled and transport failures as tool errors; fake transport verifies failure, empty success, 404, cancellation and offline paths. MCP proxy fixtures cover an isError HTTP 404 result, JSON-RPC error and request timeout; the existing MCP mapper propagates them as tool errors. |
| M7.1, M7.2, M7.3 | Not started | Evaluation, rollout and release gates. |

## Stage 0 verification notes

- Initial runtime suite: 14 passed, 1 failed because manual improvement had no observations; after instrumenting that path, 15 passed. A broader run exposed a test race on asynchronous observation persistence; the test now waits for background writes.
- After fixing the test race, `cargo test -p ice-cli --locked cognee --lib`: 76 passed; the focused improvement activity display test also passed. Formatting and `git diff --check` passed.
- `cargo clippy -p ice-cli --lib --locked`: failed on the existing `clippy::not_unsafe_ptr_arg_deref` error in `crates/ice-extensions/src/actions.rs:1013`, with 168 warnings. This is outside the changed files.
- Native tool error fix `3b91ca7`: 77 Cognee tests passed. Doctor diagnostics and credential-redaction fixture passed; after correcting a test assumption about asynchronous log ordering, 78 Cognee tests passed.
- Persistence observations: `ad1d549` adds append outcomes, `084a4a8` correlates recall and injection, `c0e7f74` adds a storage append-failure fixture, and `5c17178` adds estimated token counts. `cargo test -p ice-harness --locked --test harness_extensions` passed 9 tests; `cargo test -p ice-cli --locked cognee --lib` passed 81 tests. Formatting and `git diff --check` passed.
- `6e35b63` separates recorder pending keys by operation and request ID; focused tests for shared correlation and terminal updates passed. `04cba56` and `8039595` add and test skip reasons; `0c62a47` exposes request fields in doctor. These changes do not complete M0.
- Latest regression run after `494dec4` and `7c719c8`: Cognee library 82 passed, recorder 18 passed, harness extensions 9 passed; formatting and `git diff --check` passed. No live service or model was used.
- No performance measurements have been made for these metadata changes. The historical baseline is not a candidate benchmark.

## Verification of this round

- Current provenance/fallback changes: `cargo fmt --all` passed; Cognee library 97 passed; workspace all-targets check with `-D warnings` passed; full ice-cli crate passed 1279 library tests with 3 ignored and 19 integration tests. No live service or provider was used. Full workspace tests, live backend validation, dedicated tool-search fallback and timing/cancellation fixtures were not run.

- `cargo fmt --all -- --check`, `RUSTFLAGS="-D warnings" cargo check --workspace --all-targets --locked`, `cargo test -p ice-cli --locked` (1276 passed, 0 failed, 3 ignored) and `cargo test -p ice-harness --locked --test harness_extensions` (9 passed) all passed. Cognee library tests: 94 passed. No live service or model was used; none of this measures latency or retrieval quality.
- Not run: the full workspace test suite, `sync-embedded-docs`, and `scripts/test_prebuilt_release.py`.

No milestone is complete. No performance improvement or release gate is claimed.
