## Superseded component-stage evidence

This file records an earlier component-stage run. The final integrated source, verification and acceptance status are in [offline-closeout-report.md](offline-closeout-report.md). Do not use earlier counts or mock-JEV metrics as current results.

# Cognee durable closeout (local implementation)

## Owned durable implementation and regression proof

Changes in this closeout are limited to `crates/ice-cli/src/cognee/{candidates,admission,queue}.rs` and this note. No live service, model, or external user data was used.

- `candidates::store_pending` and `set_state` use the stable cross-process candidate advisory lock. `set_state` reloads and mutates the stored record, preserves receipt-backed/provenance data, and now returns `AlreadyExists` for a persisted terminal candidate instead of returning `Ok(())` while leaving the caller snapshot unchanged. The existing late-helper regression and a direct stale-snapshot test assert that conflict and verify accepted data remains stored.
- Cross-process candidate-capacity test: parent fills all 32 Pending slots and holds `.candidates.lock`; it starts only the exact ignored child test in the current test executable with a temp-directory sentinel. The child proposes a 33rd candidate and must observe capacity rejection after the parent releases the lock. Parent checks the child is blocked while the lock is held, enforces a five-second bound, checks successful child exit, and verifies capacity remains 32.
- Cross-process dispatch-lease test: parent owns `.dispatch.lock`; an exact-filtered ignored child with a temp-directory sentinel verifies another process cannot acquire the lease. Parent checks bounded successful exit and can acquire again after releasing its lease. No service or backend is started.
- Receipt-backed query test: accepted immutable receipt data remains queryable when its candidate projection is corrupt and when that projection is absent; an expired receipt is excluded. A receipt-backed user/Constraint correction with matching branch supersedes the target only in that branch. Existing candidate tests separately cover invalid role, dataset/scope and expiry correction cases.
- Existing receipt tests remain in place and pass: receipt writer failure does not commit; Prepared orphan without a receipt stays blocked; committed receipt reconciliation repairs projections idempotently; InFlight/Uncertain are not reset; completed receipt does not re-create pending queue work; conflicting receipts and traversal IDs are rejected. Existing queue tests cover admission capacity/dedup by dataset and node-set and Uncertain explicit-flush selection.

## M4 acceptance mapping / limits

- **M4.1 — locally verified subset:** candidate capacity and dispatch lease exclusion across processes; receipt-writer failure; Prepared/committed recovery, no InFlight/Uncertain reactivation, and completed-operation projection behavior as enumerated above. Cross-process child waits are bounded and exit status is checked. This does not constitute end-to-end runtime dispatch-intent, timeout-after-remote-acceptance, migration, or full cross-process drain proof.
- **M4.2 — locally verified subset:** pending capacity is bounded under the process-wide lock; immutable accepted receipt facts are available to local query despite corrupt/missing candidate projections and expired evidence is filtered. Attribution and runtime switches are not established by these storage tests.
- **M4.3 — locally verified subset:** correction role/category, dataset/branch scope and expiry gates are exercised by existing candidate tests and the receipt-backed branch-supersession regression. This is fixture-level local proof, not production-backend validation.
- **M4.4 — not verified here:** runtime job-handle persistence, reconciliation, retry/backoff and backend searchability are outside these owned changes.

## Local verification

Commands use the inherited private per-worktree `CARGO_TARGET_DIR` (`target/validation`), `CARGO_BUILD_JOBS=2`, `RUSTFLAGS=-Dwarnings`, and `--locked`; no shared `/tmp` target was used.

- `cargo test -p ice-cli --lib --locked cognee::candidates::tests -- --nocapture` — PASS, 10 passed, 1 ignored child helper. The passing parent regression launches the helper with the exact filter.
- `cargo test -p ice-cli --lib --locked cognee::queue::tests -- --nocapture` — PASS, 9 passed, 1 ignored child helper. The passing parent regression launches the helper with the exact filter.
- `cargo test -p ice-cli --lib --locked cognee::admission::tests -- --nocapture` — PASS, 7 passed.
- `rustfmt --edition 2021 --config skip_children=true` on the three owned Rust files — PASS.
- `git diff --check` — PASS.

No live inference, Cognee/JEV service, external user data/config, or `~/.ice` access was used. Runtime dispatch/admission gates remain with their runtime owner; this closeout does not assign completed durable code or these passing storage regressions back as missing lead implementation.
