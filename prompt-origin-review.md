# Prompt origin implementation review

## API and behavior

- `ice_ai::types::UserMessage` now has `origin: Option<PromptOrigin>` with `#[serde(skip)]`; normal `UserMessage` and `ice_ai::Message` serialization omit it.
- `UserMessage::with_origin(raw_text, expansion_kind)` attaches bounded (16 KiB) provenance and fingerprints exact `UserContent` plus timestamp with SHA-256. `UserMessage::original_prompt()` returns the raw prompt only if the version, size, content, and timestamp still validate.
- `ice_agent::AgentMessage::User` has a variant serde adapter that emits validated provenance as `icersPromptOrigin`, retaining it in durable agent/session JSON while excluding it from provider message serialization. Legacy agent JSON loads without provenance; invalidated origin is discarded.
- Lead bridge accessor: `ice_agent::message::original_user_prompt(&AgentMessage) -> Option<&str>`.
- TUI direct and queued skill-expansion prompts attach the raw pre-expansion input to the actual expanded user content, preserving its images and timestamp.

## Tests and checks

- `cargo test -p ice-ai --locked --offline`: passed (92 unit tests plus integration suites; new prompt-origin integration tests included).
- `cargo test -p ice-agent --locked --offline`: passed (26 unit tests and integration suites; new serde tests included).
- `cargo test -p ice-cli --locked --offline skill -- --nocapture`: passed (44 matching tests).
- `cargo check --workspace --locked --offline`: passed.
- Focused tests cover provider serialization omission, direct-user deserialization behavior, fingerprint invalidation for changed content/timestamp, validated tagged `AgentMessage` round-trip, and legacy JSON loading. The TUI queue skill test verifies raw prompt recovery and image retention.

## Scope notes

- No changes were made to the separately owned before-request callback.
- No Cognee configuration, persistence, capability, or helper behavior was changed.
- This focused patch does not modify the harness prompt-template expansion implementation.

Commit SHA: to be filled after commit.
