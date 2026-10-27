# Helper completion review

## Implemented in this worktree

- Helper scheduling now exits before candidate-directory access when Cognee is offline/disabled, the write policy is not selective, shutdown has started, or no helper is bound. This avoids optional helper scans and candidate expiry mutations when helper work cannot run.
- Candidate promotion now requires both `durable == Yes` and `applicable == Yes` for fresh and cached helper judgments. Uncertain applicability remains pending and does not invoke persistence.
- The runtime fake-helper regression covers explicit unknown applicability and confirms no write occurs. The helper-off regression confirms an expired pending candidate is not mutated by disabled scheduling.
- Baseline already includes commits `5459227a` (helper gate integration) and `19944017` (local expiry/correction suppression).

## Configuration and behavior

Helper configuration currently selects an explicit decision provider (`helperProvider` / `ICERS_COGNEE_HELPER_PROVIDER`). The helper remains absent unless that provider is configured and the runtime is in selective-write mode. No chat-model fallback is used. However, the typed request currently hardcodes the TypeSafe model ID, so explicit model selection has **not** been completed by this change. Provider/model controls, revision-sensitive binding, provenance-aware cache keys, robust per-provider shared budgets, immutable flight ownership, comprehensive host revalidation, precise effective-setting source reporting, and immediate cancellation guarantees still require integration work.

## Verification

- `cargo test -p ice-cli --lib --locked cognee:: --quiet`: 172 passed, 0 failed, 0 ignored; 1,253 filtered out. Tests use synthetic/fake transports and decision responses; no live model, Cognee, TypeSafe/JEV, or external inference request was made.
- Formatting was applied only to `crates/ice-cli/src/cognee/runtime.rs`.

## Integration needs / remaining gates

- Thread explicit helper model configuration from config/env and `/cognee` + `/settings` through TypeSafe request construction; retain no-fallback semantics and test invalid IDs/round trips.
- Avoid rebuilding or resetting quotas on unrelated config saves; use explicit flight IDs across reset/cancel and do not erase shared provider request history.
- Bound/coalesce candidate event work and cache size; add process/provider shared rolling budgets and deterministic isolated gate injection.
- Revalidate candidate revision/source/scope/expiry and current config after await, including cache hits; tighten helper source excerpts and stable observation request ownership.
- Validate selective-mode usability without helper provider and synchronously prevent optional writes after off/manual transitions.
- JEV quality and latency remain **UNVERIFIED**; do not enable by default or make effectiveness claims. The permitted live model restriction prevented any inference-quality evaluation.

## Commits

This review and the runtime helper safety fixes are committed together as a local implementation commit; no publication or push is performed.
