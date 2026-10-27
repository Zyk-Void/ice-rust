# Request-boundary implementation review

Status: **host request-context bridge integrated; extension consumer/origin plumbing remains separate.**

## Implemented guarantees in this slice

- `before_request` runs only for a real provider request, after the request-local context transform and before provider conversion. The transform is evaluated once for that request; the hook sees the same transformed messages the provider uses.
- Persisted optional request packets are added to subsequent loop request contexts, including after tool results, but are not placed in `new_messages` and therefore are not appended again by final-pass persistence.
- Request hooks capture the lane, session/run identity, and a pinned `SessionTree` at operation start. Before each append they require the same active run and lane and a non-cancelled signal. They never resolve a mutable current global view after an extension await.
- `set_session` and branch navigation use exclusive active-run admission. They cannot replace the active branch during a run; `branch_generation = 0` is the stable run-local epoch under that enforced invariant, not a fabricated mutable counter. Navigation after the operation starts is rejected by admission.
- Persisted custom packet details contain source, correlation ID, extension registration ordinal, run, lane, and session. A bounded run-local set is initialized from the active context and updated only after successful append. Duplicate correlation packets are not appended again. Failed writes do not consume the key.
- The context-token budget is shared in extension order; oversized packets are rejected intact. Optional append failures fail open and acknowledge false; successful append/insertion acknowledges true. No mandatory external queued delivery path was changed.
- `original_prompt` is currently the run prompt where known; raw origin/expansion provenance remains with the separate worker/lead work.

## Verification in this slice

- `cargo test -p ice-agent --locked --quiet`: passed.
- `cargo test -p ice-harness --locked --quiet`: passed (latest run).
- `cargo test -p ice-cli --lib --locked cognee:: --quiet`: passed (172 passed, 1,253 filtered out).
- Faux/local tests only; no live provider, Cognee, JEV, or network calls.

## Remaining limitations

This slice adds one faux-host regression to the existing active-run fixture proving session replacement is rejected while the run is active. It does not add dedicated tests for all five requested scenarios (request-index-1 provider capture, final-response no-extra-request, two-extension over-budget rejection, cancellation/stale-navigation awaits, and cross-request/retry correlation persistence). Existing agent/harness suites pass but do not establish those new guarantees. The context-transform behavior is changed and covered only by existing tests so far. Original-prompt provenance and Cognee/JEV consumers remain separate work. Do not treat this review as sign-off for the full consumer implementation.
