# Request-context host boundary review

Status: **complete for the bounded request-context packet and prompt-origin cases.**

Implementation commits: `f4e6c7e3` (trusted request context) and this follow-up.

- The `RequestContextEvent` callback runs only at an already-needed provider
  request boundary. The loop does not create a model turn for memory. The
  post-tool request test proves no extra provider request is introduced.
- A validated origin is authoritative. A plain user message without origin
  uses its unexpanded text as raw intent, which restores explicit history
  prompts. If provenance is attached but invalid, it is not silently replaced
  by the expanded body. Legacy recognized `<skill ...>` and
  `<prompt-template ...>` expansions without valid provenance have no raw
  intent; unknown markup remains conservatively usable as plain text.
- Template invocations preserve the expanded provider content while exposing
  their distinct raw invocation. Provider serialization omits provenance;
  durable agent-message serialization retains validated origin metadata.
- Prompt-origin validation is cached per exact `AgentMessage` within the run
  (bounded to 256 entries). Invalid serialized metadata is discarded without
  rejecting the user message.
- `branch_generation` is a monotonic per-harness navigation epoch captured for
  the run. Successful session replacement and completed tree navigation
  increment it; ordinary run activity does not. Request packet persistence
  checks the captured epoch plus run/lane owner immediately before append.
- `latest_compaction_at` is refreshed at each request callback. The new
  mid-run test verifies a later callback observes the compaction timestamp,
  bounded active context, and exactly one transform/provider request per pass.
- Optional packets are persisted before injection. A successful append means
  delivered and acknowledges true; failed, cancelled-before-append, duplicate,
  empty, and oversized packets acknowledge false. A failed append leaves no
  packet in provider context and does not corrupt seen-correlation state.
- One shared request token estimate is decremented across extensions and
  includes role/envelope framing. Oversized packets are rejected intact. The
  mandatory external queue path is unchanged.
- Correlation dedup uses custom type plus correlation ID; a duplicate packet
  is rejected and acknowledged false. The regression test checks one durable
  entry and one provider-context copy across initial/tool-follow-up requests.

## Verification

`crates/ice-harness/tests/harness_extensions.rs` now has 17 tests, including
shared-budget rejection, duplicate correlation, optional-packet persistence
failure, mid-run compaction refresh, plain/legacy/tampered origins, and prompt-template expansion/raw invocation separation. Checks passed:

- `cargo test -p ice-harness --locked --no-fail-fast` — 268 passed.
- `cargo test -p ice-agent --locked --no-fail-fast` — 28 passed.
- `cargo test -p ice-cli --locked cognee:: --no-fail-fast` — 202 passed,
  1 ignored; the selective-proposal full prompt/tool/answer test passes.

Workspace-wide `cargo fmt --all -- --check` reports existing formatting
changes in unrelated `ice-agent` files. The two touched Rust files were
formatted directly.
