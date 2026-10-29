## Superseded component-stage evidence

This file records an earlier component-stage run. The final integrated source, verification and acceptance status are in [offline-closeout-report.md](offline-closeout-report.md). Do not use earlier counts or mock-JEV metrics as current results.

# Capability closeout: M6.1–M6.3

This closeout records the repository-side implementation and offline evidence. It does not claim live host UX, live Cognee/MCP/provider behavior, or real JEV ranking quality.

## M6.1 — Local capability catalog and lifecycle

**Acceptance mapping.** The resident catalog is assembled from the skill loader's winners and collision-identified losers plus the MCP manager's in-memory `ServerView` snapshot. `crates/ice-cli/src/capability_catalog.rs` provides namespaced skill/MCP/native IDs, content/schema fingerprints, availability and model-invocation flags; `from_live` does not perform filesystem discovery, and MCP state comes from `snapshot()`. Startup in `crates/ice-cli/src/session.rs` enriches this with native tool definitions selected by `active_tool_names` and `filter_active_tool_names`. The `/reload` path rebuilds local registries and, after rebuilding tools, refreshes the service with the then-active native whitelist. No reconnect is performed by catalog construction. Hidden/non-invocable skills and unavailable or disabled MCP tools remain visible for inventory where known but are excluded from shortlist/auto-suggestion.

**Offline evidence.** `capability_catalog_tests.rs` covers stable IDs/hash, source classification, collision winner/loser availability, hidden skills, disabled/disconnected MCP state, schema changes and add/remove refresh. `capability_service.rs` tests refresh/revision, inventory pagination/filtering and registered tool access to the refreshed snapshot. `session.rs` supplies the actual startup/reload consumer path; its reload tests exercise local resource reload behavior. `resource_dirs.rs`/`settings.rs` include opt-in configured path expansion; `562dd769` already contains the leading-tilde fix and tests, so it is not duplicated here.

**Caveat.** MCP snapshots describe resident/last-observed state and may have unknown schema hashes; they are not a live probe. No network reconnect, Cognee/JEV call, real-user home scan or live MCP lifecycle was run. Local tests do not prove external server schema stability.

## M6.2 — Skill instruction coverage and invalidation

**Acceptance mapping.** `crates/ice-cli/src/skill_coverage.rs` separates a skill descriptor from evidence that its normalized full body is present: full untruncated `read` results and valid explicit user skill invocations can establish `Loaded`; partial reads and advertised metadata cannot. A full body with a changed hash is `Stale`; compacted-away messages no longer count because coverage is derived from the active message branch. Coverage keys are stable descriptor IDs, preventing same-name source collisions. The capability request-context consumer in `capability_service.rs` derives coverage from the event's actual active messages; restored session messages therefore use the same derivation rather than trusting persisted metadata. Compaction, branch changes and revised skill hashes are reflected when active messages/registry are refreshed.

**Offline evidence.** `skill_coverage.rs` tests full versus partial body, hash revision/staleness, branch/compaction and restored-message coverage, plus hidden skill explicit invocation. Capability-service request-context tests cover full-body suppression followed by a suggestion after same-generation compaction. Session reload tests exercise resource replacement; the catalog/service revision fence prevents stale acknowledgments from authorizing a changed registry.

**Caveat.** Coverage means matching text exists in supplied active messages; it cannot establish model attention or use. The tests model host messages and local reloads, not every real host event ordering or provider context transformation.

## M6.3 — Bounded advisory suggestions and capability misses

**Acceptance mapping.** `capability_suggest.rs` keeps inventory local to the exact catalog, caps listing/ambiguity output, rejects unavailable/non-invocable entries, avoids helper-style suggestions for unrelated requests, suppresses loaded skills, and records repeat suggestions by task state. Hints are advisory text only. `capability_service.rs` is installed as both the harness extension and `capability_search` tool in `session.rs`; request-boundary hints observe the actual original prompt, active messages, task cursor, cancellation and remaining-token budget, persist through the host acknowledgment contract, and do not execute capabilities. The tool accepts `context_event: "capability_miss"` only in a unique live request scope; the typed pending signal is consumed at the next request boundary. It reports recording, not retrieval.

**Offline evidence.** `capability_suggest_tests.rs` covers exact inventory output/caps, name hints, irrelevant requests, disabled/hidden filtering, ambiguity, and within-task repeat suppression. `capability_service.rs` covers cancellation, absent/ambiguous scopes, event consumption at the next boundary, bounded request state, persistence acknowledgment and loaded-skill/compaction behavior. Session startup and tool registration are inspected in `session.rs` rather than inferred from pure suggestion tests.

**Caveat.** Tests verify deterministic matching and bounded advisory mechanics, not whether a real user finds a hint useful. No real JEV ranker or live UX was evaluated. A restored skill can be suggested only when current active messages do not establish its full body and the task query meets the local matching criteria; there is no claim of semantic ranking quality.

## Verification status

Focused verification used `CARGO_TARGET_DIR=$PWD/target/cognee-closeout`, `CARGO_BUILD_JOBS=2`, and `RUSTFLAGS=-Dwarnings`:

- `cargo test -p ice-cli --lib --locked capability_catalog -- --test-threads=1` — 16 passed.
- `cargo test -p ice-cli --lib --locked capability_suggest -- --test-threads=1` — 18 passed.
- `cargo test -p ice-cli --lib --locked capability_service::tests:: -- --test-threads=1` — 8 passed.
- `cargo test -p ice-cli --lib --locked skill_coverage::tests:: -- --test-threads=1` — 21 passed.
- `cargo test -p ice-cli --lib --locked settings::tests:: -- --test-threads=1` — 9 passed.
- `cargo test -p ice-cli --lib --locked session::tests:: -- --test-threads=1` — 66 passed.
- `git diff --check` — passed.

An initial broader substring filter, `cargo test -p ice-cli --lib --locked capability -- --test-threads=1`, also selected unrelated MCP prompt/resource fixture tests: 43 passed, 3 failed at `crates/ice-cli/src/mcp/../../tests/support/mcp_fixture.rs:25` during fixture startup. Those tests were not part of the capability modules above; no MCP production behavior was modified. No broad workspace suite or live external service was run.
