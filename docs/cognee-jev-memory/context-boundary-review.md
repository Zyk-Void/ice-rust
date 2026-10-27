# Request-boundary implementation review

Status: **partial host contract only; not a complete Cognee context-delivery implementation.**

## Implemented

- Added `ice_agent::AgentLoopConfig::before_request`, invoked only immediately before a real provider stream and after pending steering/follow-up messages have been appended. Its returned messages affect that request context but are excluded from `new_messages` final persistence.
- Added `RequestContextEvent` and `HarnessExtension::request_context`. The default delegates to legacy `prompt_context` only at request index zero and never invokes both callbacks.
- Added the interface note at `context-boundary-interface.md`.

## Required bridge integration before consumers can use this

The current harness does **not** wire `before_request` to extensions. Integration must:

1. Track the expanded latest user prompt and trusted original intent from its exact persisted entry; untrusted skill/template expansions must have `original_prompt: None`.
2. Supply real run ID, lane ID, branch-generation counter tied to navigation, monotonic per-run request index including retries, context window and latest compaction timestamp.
3. Use the actual post-compaction/pruning request context in the event (not the whole session history); coordinate ordering after tool result and steering delivery.
4. Share `min(800, 1% of context_window)` across all extensions per request; reject an oversized packet intact.
5. Persist optional packets exactly once before injection, fail open on optional persistence errors, acknowledge `prompt_context_persisted` true only after persistence and inclusion, and keep them out of final-pass `new_messages` writes.
6. Add faux-harness tests for late second-request delivery, no extra call after final response, retry/restart idempotence, persistence failure, stale navigation/cancellation, compaction ordering, original prompt/skill/images/queued/RPC cases, and budget/envelope safety.

Original-prompt metadata plumbing in CLI/TUI/RPC and remote prompt paths is not implemented in this change. No compatibility fields were added to persisted/provider message formats. No unsafe persistence or fabricated identity was introduced as a substitute for the bridge.

## Verification performed

- `cargo test -p ice-agent --locked`: passed (39 passed, 1 ignored).
- `cargo test -p ice-harness --locked`: passed (all test binaries successful; includes existing harness boundary suites).
- `cargo test -p ice-cli --lib --locked cognee:: --quiet`: passed (172 passed, 1,253 filtered out).
- Faux/local tests only; no live provider/Cognee/JEV calls.

Exact commit SHA is recorded in repository history after commit. Integration should treat this commit as the low-level hook and type contract only, not completion of M2.1/M2.3/M3.3/M0.1 host behavior.
