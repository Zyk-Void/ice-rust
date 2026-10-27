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
| M0.1 | In progress | Versioned recall skip/nonempty/empty and improvement-acceptance metadata added in `crates/ice-cli/src/cognee/runtime.rs`; skipped recall distinguishes unavailable, disabled, circuit-open, short-prompt and duplicate reasons, and legacy phases are preserved. Manual improvement is observed and the TUI labels improvement acceptance. Recorder titles now distinguish versioned skips, retrievals and accepted jobs while preserving legacy titles. Recall terminal metadata records actual remote wait and original source/session context, including errors. A session/dataset fence rejects late recall after identity changes; fake delayed transport verifies this path. An in-progress harness callback records whether context was appended and its byte count, not provider delivery or token counts; recall and injection share a request ID; a storage-failure harness fixture verifies that failed appends are not sent; full source/context attribution remains. Same-session task-generation fencing, complete late-completion attribution and full fixtures remain. |
| M0.2 | In progress | Doctor distinguishes the native recall endpoint and locally sent recall/improve fields from unverified server support, background improvement acceptance and an unprobed MCP catalog; endpoint credentials are stripped. Existing wire-shape fixtures cover recall and writes; new 404/422 fixtures verify no fallback retries. Capability discovery and unsupported completion-status fixtures remain. |
| M1.1, M1.2 | Not started | Depends on M0.1; legacy read-through also depends on M3.1. |
| M2.1, M2.2, M2.3 | Not started | Selective recall; preserve legacy defaults. |
| M3.1, M3.2, M3.3 | Not started | Evidence scope and context coverage. |
| M4.1, M4.2, M4.3, M4.4 | Not started | Selective persistence and durability. |
| M5.1, M5.2, M5.3, M5.4 | Not started | Optional JEV; no live JEV evaluation under the Luna-only restriction. |
| M6.1, M6.2, M6.3 | Not started | Capability discovery and skill suggestions. |
| M6.4 | In progress | Native search propagates disabled and transport failures as tool errors; fake transport verifies failure, empty success, 404, cancellation and offline paths. Timeout and MCP fixtures remain. |
| M7.1, M7.2, M7.3 | Not started | Evaluation, rollout and release gates. |

## Stage 0 verification notes

- Initial runtime suite: 14 passed, 1 failed because manual improvement had no observations; after instrumenting that path, 15 passed. A broader run exposed a test race on asynchronous observation persistence; the test now waits for background writes.
- After fixing the test race, `cargo test -p ice-cli --locked cognee --lib`: 76 passed; the focused improvement activity display test also passed. Formatting and `git diff --check` passed.
- `cargo clippy -p ice-cli --lib --locked`: failed on the existing `clippy::not_unsafe_ptr_arg_deref` error in `crates/ice-extensions/src/actions.rs:1013`, with 168 warnings. This is outside the changed files.
- Native tool error fix `3b91ca7`: 77 Cognee tests passed. Doctor diagnostics and credential-redaction fixture passed; after correcting a test assumption about asynchronous log ordering, 78 Cognee tests passed.
- Persistence observation change: `cargo test -p ice-harness --locked --test harness_extensions` passed 8 tests; `cargo test -p ice-cli --locked cognee --lib` passed 81 tests. Formatting and `git diff --check` passed. The direct failure-outcome fixture does not simulate a storage append failure.
- No performance measurements have been made for these metadata changes. The historical baseline is not a candidate benchmark.

No milestone is complete. No performance improvement or release gate is claimed.
