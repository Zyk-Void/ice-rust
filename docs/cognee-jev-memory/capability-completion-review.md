# Capability routing completion review

## Implementation

- Added `CapabilityKind::NativeTool`; active native descriptors are generated from actual registered harness tools with exact serialized schema fingerprints and filtered by active tool policy. No native Cognee entry is fabricated.
- Added a deterministic 4,096-descriptor catalog cap. Inventory search validates queries at the service API (2,048 bytes), supports arbitrary reachable offsets with `next_offset`, reports page/catalog truncation, filters unusable entries, and scopes skill/tool listings.
- Hardened capability hint bookkeeping: coverage refresh hashes active messages plus revision, hints are whole-or-none, acknowledgment is fenced by typed session/task/generation plus registry revision, and pending/delivered maps are bounded with deterministic eviction.
- Installed the same `Arc<CapabilityService>` as `HarnessExtension` and `capability_search` at startup; reload refreshes descriptors from current active native tools and local resource/MCP snapshots. Request-context hints use `TaskCursor` per bounded session/lane scope and observe the actual original prompt and compaction timestamp. Hints respect cancellation and remaining context budget, use normal persistence acknowledgment, and never execute suggestions or make model/network calls.
- MCP snapshots include stable schema fingerprints when cached/discovered schema exists; empty/unknown schemas remain explicitly un-hashed. Snapshots require no reconnect or network access.

## Exact integration accessor

`CapabilityService::doctor_cognee_contract() -> serde_json::Value` is available for the Cognee doctor binding. It reports resident Cognee-named descriptors and `probed: false`; native and known MCP schema fingerprints are exact. The accessor is read-only and does not reconnect.

## Verification on isolated worktree target

`CARGO_TARGET_DIR=/home/mewtwo/Zykairotis/ice-rust/.worktrees/capability-catalog-final/target/validation`

- `cargo check -p ice-cli --locked` — passed.
- After `CARGO_TARGET_DIR=/home/mewtwo/Zykairotis/ice-rust/.worktrees/capability-catalog-final/target/validation cargo clean -p ice-cli`:
  - `cargo test -p ice-cli --locked --lib capability_catalog -- --test-threads=1` — 16 passed.
  - `cargo test -p ice-cli --locked --lib capability_service::tests:: -- --test-threads=1` — 8 passed, including caller cancellation/no-scope/next-boundary/10-follow-up coverage and same-generation loaded-skill compaction regression.
  - `cargo test -p ice-cli --locked --lib capability_suggest -- --test-threads=1` — 18 passed; capability-related total 42 passed.
  - `cargo test -p ice-cli --locked --lib skill_coverage::tests:: -- --test-threads=1` — 21 passed; `cargo test -p ice-cli --locked --lib cognee::task::tests:: -- --test-threads=1` — 4 passed.
  - `cargo test -p ice-cli --locked --lib session::tests:: -- --test-threads=1` — 66 passed.
  - `cargo test -p ice-cli --locked --lib cognee::runtime::tests::request_boundary_ -- --test-threads=1` — 8 passed; two additional selected context/compaction tests passed.
  - Final rerun after the query-ranking fix: capability/catalog 16 + service 8 + suggestion 18 (42 capability tests total), skill coverage 21, task cursor 4, all session tests 66, and selected Cognee request-boundary tests 8 — passed. `cargo check -p ice-cli --locked` and `git diff --check` — passed.

## Closure

Both requested closure items are implemented and regression-tested. `capability_search` accepts optional `context_event: "capability_miss"`; it records only against a unique, live request-hook scope, honors caller cancellation, reports `event_recorded` (not retrieval), and consumes its typed pending event at the next existing request boundary. It makes no model/network call and does not execute capabilities. The request-hook regression proves full loaded skill body suppresses a suggestion, then becomes eligible once after compaction removes that body while branch generation remains zero and task/run remain unchanged. Metadata and assistant-forged content are not used as loaded-body evidence.
