# Cognee durable closeout (local implementation)

## Implemented

- `crates/ice-cli/src/cognee/candidates.rs`: serialized candidate-store mutations with a stable advisory lock; pending capacity accounting and expiry now run under that process-wide lock.
- A stale Pending candidate cannot replace an existing terminal candidate or a candidate carrying an admission receipt. A malformed file at the same candidate ID fails closed and is preserved.
- Candidate state updates reload and mutate the persisted record under the lock, rather than serializing a stale caller snapshot. Terminal records cannot be reopened by delayed helper outcomes; unrelated persisted provenance/admission fields are preserved.
- Added regressions for a late helper result against an admitted candidate and for preserving malformed candidate data.

## M4 acceptance mapping

- **M4.1 — NOT fully verified here.** Admission receipt durability/reconciliation, prepared reservation, dispatch lock, and explicit uncertain selection are present in owned queue/admission modules and focused tests pass. Full runtime dispatch-intent integration, timeout-after-acceptance proof, compatibility/migration, and complete cross-process drain behavior are not verified by this subtask.
- **M4.2 — PARTIALLY verified.** Source-validation and candidate focused suites pass; bounded pending candidates and accepted local evidence are exercised. Candidate capacity is now cross-process serialized. End-to-end attribution and all capture/improve switches are runtime-owned and remain to integrate/verify.
- **M4.3 — PARTIALLY verified.** Existing scoped correction/freshness tests remain unchanged and pass as part of candidate tests. No new exhaustive branch/role/revision correction matrix was added in this implementation.
- **M4.4 — NOT fully verified.** Dream ledger unit tests pass and runtime currently emits accepted/searchable-unknown outcome metadata. Supported job-handle persistence and reconciliation are owned by excluded client/runtime modules and remain unimplemented/unverified here. No live Cognee or inference proof was attempted.

## Local verification

All test commands used the inherited per-worktree `CARGO_TARGET_DIR`, `CARGO_BUILD_JOBS=2`, `RUSTFLAGS=-Dwarnings`, and `--locked`.

- `cargo test -p ice-cli --lib --locked cognee::candidates -- --nocapture` — PASS, 7 tests (final run).
- `cargo test -p ice-cli --lib --locked cognee::admission -- --nocapture` — PASS, 7 tests.
- `cargo test -p ice-cli --lib --locked cognee::queue -- --nocapture` — PASS, 8 tests.
- `cargo test -p ice-cli --lib --locked cognee::source_validation -- --nocapture` — PASS, 5 tests.
- `cargo test -p ice-cli --lib --locked cognee::dream -- --nocapture` — PASS, 8 tests.
- `cargo test -p ice-cli --lib --locked cognee::suppress -- --nocapture` — PASS, 4 tests.
- `cargo test -p ice-cli --lib --locked cognee::propose -- --nocapture` — PASS, no dedicated matching tests.
- `cargo test -p ice-cli --lib --locked cognee::validation -- --nocapture` — PASS (successful filtered build/test invocation).
- `rustfmt --edition 2021 --config skip_children=true` on the eight owned files — PASS; same command with `--check` — PASS.
- `git diff --check` — PASS.
- One initial test build failed because state was moved before reuse; corrected and rerun successfully. No failed-build artifacts are tracked.

## Exact lead-only integration remaining

1. In `crates/ice-cli/src/cognee/runtime.rs`, ensure manual remember and candidate dispatch commit the durable admission receipt and Prepared operation before any backend request; mark InFlight immediately before dispatch. On transport uncertainty, persist Uncertain, never auto-select it, and require explicit flush for uncertain replay. Hold the existing process-wide dispatch lease through recovery/dispatch; do not allow two drainers to send the same intent. Reconcile receipt projections without reactivating InFlight/Uncertain operations.
2. In `runtime.rs`, route explicit remember/correction and attributable source/outcome candidates through the same deterministic source validation/admission path regardless of optional helper/JEV switches. Revalidate source ID, role, exact span/hash, scope/revision, expiry, and policy after every await and before durable commitment. Keep rejected helper outcomes terminal and ensure no stale snapshot can be stored over admitted candidate state.
3. In `crates/ice-cli/src/cognee/client.rs` and `runtime.rs`, distinguish accepted-unverified from searchable. Persist backend job handles only where supported; reconcile by revision, coalesce same-revision work, and bound retry/backoff. Do not infer searchability from HTTP 2xx. Keep locally curated accepted facts usable with provenance independently of indexing verification.
4. Add runtime fake-backend tests for crash/abort after backend acceptance, explicit uncertain replay, restart projection repair, concurrent drains, same text across distinct category/scope, correction scope/role, helper/capture/improve switch matrix, and accepted/delayed/unsupported searchability. Keep all tests offline and fixture-backed.

No live inference, Cognee/JEV, external user data/config, or `~/.ice` access was used.
