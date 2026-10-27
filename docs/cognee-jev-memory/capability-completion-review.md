# Capability service completion review

## Commit

- `5b32191` — `feat(capabilities): add resident search and hint service`
- Includes `Co-Authored-By: OpenAI <noreply@openai.com>`.

## Implemented slice

- Registered read-only `capability_search` during session preparation when tool policy permits it; reload refreshes the existing `Arc<CapabilityService>` used by the registered weak-reference tool.
- Catalog uses loaded skill snapshots and resident MCP snapshots only. It does not scan directories, connect to MCP, invoke a capability, or access Cognee/JEV.
- Search is bounded and paginated, with stable IDs, source, skill-body/MCP metadata fingerprints, availability/readiness, validated skill load paths and current skill-context coverage.
- Suggestions are limited to task-start/capability-miss event types, filter loaded entries before the shortlist cap, and only suppress repeats after a persisted acknowledgment fenced by task, branch generation and catalog revision.
- Local Cognee contract inventory includes native `cognee_search` as `unprobed`, not healthy; it makes no calls.
- Added the interface binding example and test fixture updates for the `ReloadContext` field.

## Verification

- `cargo check -p ice-cli` — passed.
- `cargo fmt --check` — passed.
- `git diff --check` — passed before commit.
- `cargo test -p ice-cli capability --no-fail-fast` — passed, 39 tests.
- `cargo test -p ice-cli cognee --no-fail-fast` — passed, 187 tests (Cognee regression baseline).
- `cargo test -p ice-cli session::tests::build_applies_the_name_flag_to_the_session -- --exact` — passed.
- `cargo test -p ice-cli session:: --no-fail-fast` — 63 passed, 4 failed; serial rerun was 64 passed, 3 failed. Failures are existing session fixture/environment issues (global config test lock poisoned in three reload tests; one expected skill resource absent in a reload fixture), not assertions in capability code. The focused build test and capability/session registration wiring compile and pass.

## One remaining lead hook

The session consumer currently has no safe `RequestContextEvent`/task-cursor callback. Lead should attach `ReloadContext::capability_service.prepare_hint(session_id, task_id, branch_generation, query, messages, event, max_chars)` to the new task-start/capability-miss event callback, append the returned opaque prompt context through the existing persistence path, then call `acknowledge_hint(&context, persisted)` with the actual persistence result. Do not call it for ordinary follow-ups. The exact example is in `capability-service-interface.md`.
