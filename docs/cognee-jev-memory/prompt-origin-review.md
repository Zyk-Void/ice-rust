# Prompt origin implementation review

## API and behavior

- `ice_ai::types::UserMessage` now has `origin: Option<PromptOrigin>` with `#[serde(skip)]`; normal `UserMessage` and `ice_ai::Message` serialization omit it.
- `UserMessage::with_origin(raw_text, expansion_kind)` attaches bounded (16 KiB) provenance and fingerprints exact `UserContent` plus timestamp with SHA-256. `UserMessage::original_prompt()` returns the raw prompt only if the version, size, content, and timestamp still validate.
- `ice_agent::AgentMessage::User` has a variant serde adapter that emits validated provenance as `icersPromptOrigin`, retaining it in durable agent/session JSON while excluding it from provider message serialization. Legacy agent JSON loads without provenance; invalidated origin is discarded.
- Lead bridge accessor: `ice_agent::message::original_user_prompt(&AgentMessage) -> Option<&str>`.
- TUI direct and queued skill-expansion prompts attach the raw pre-expansion input to the actual expanded user content, preserving its images and timestamp.

## Current status

This review is complete for the bounded request-context origin cases, together
with [context-boundary-review.md](context-boundary-review.md). The request
callback now falls back to ordinary unexpanded plain user text when there is no
origin metadata; validated expansion metadata remains authoritative, and invalid
attached metadata does not fall back to the changed body. Legacy skill/template
expansions without valid origin are excluded.

The previously reported selective-proposal Cognee failure is resolved by that
fallback. `cargo test -p ice-harness --locked --no-fail-fast` (268 passed),
`cargo test -p ice-agent --locked --no-fail-fast` (28 passed), and
`cargo test -p ice-cli --locked cognee:: --no-fail-fast` (202 passed, 1
ignored) all pass.

## Verification

Final-slice checks passed on the request-fix worktree:

- `cargo test -p ice-harness --locked --no-fail-fast` — 268 passed.
- `cargo test -p ice-agent --locked --no-fail-fast` — 28 passed.
- `cargo test -p ice-cli --locked cognee:: --no-fail-fast` — 202 passed,
  1 ignored. The selective-proposal full prompt/tool/answer regression passes.

The origin implementation's focused tests cover provider serialization
omission, direct-user deserialization, fingerprint invalidation for changed
content/timestamp, validated tagged `AgentMessage` round-trip, legacy JSON
loading, and TUI queue raw prompt/image preservation.

## Scope notes

- No changes were made to the separately owned before-request callback.
- No Cognee configuration, persistence, capability, or helper behavior was changed.
- This focused patch does not modify the harness prompt-template expansion implementation.

Feature commit SHA: `73e8ee83b47bba18b7b23db244cecca292655c38`.
