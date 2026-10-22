# Plan: typed ChatGPT subscription (Codex) support

Status: four implementation stages integrated with automated coverage. Live-service and Windows acceptance have not been run. Rust 1.78 verification is blocked by the existing locked `aligned 0.4.3` dependency requiring edition-2024 Cargo support; the current-toolchain workspace gates pass.

## Goal and scope

A user runs `icers auth login --provider openai-codex`, signs in with their
ChatGPT account, and uses the subscription models without an API key or
extension. CLI and TUI use the same built-in authentication implementation.

Follow `ICE_PORTING.md` §9.1: retain the existing native protocols and add
this endpoint. Do not introduce a general OAuth/provider framework.

Build on `refactor/ai-typed-endpoints` (`c758f45`), already present in this
worktree. Do not restart from `main` and lose that prerequisite. If work is
split into PRs, each layer must include its prerequisite or follow its merge.
This plan does not authorize commits, pushes, PR creation, or live login.

## Reference and compatibility baseline

Use the local TypeScript implementation at `~/Zks/ice/packages/ai/`:

- `src/api/openai-codex-responses.ts`
- `src/auth/oauth/openai-codex.ts`, `pkce.ts`, and `oauth-page.ts`
- `src/auth/resolve.ts`
- `src/providers/data/openai-codex.json`
- `test/openai-codex-oauth.test.ts`, `openai-codex-stream.test.ts`, and
  `codex-sse-eof.test.ts`

Before implementing each stage, inspect the relevant reference functions
and tests. Record the reference commit in the implementation notes; line
numbers and catalogs can move. Port the SSE behavior, not the WebSocket
implementation. Preserve existing OpenAI/Anthropic behavior and extension
precedence.

## Existing implementation to reuse

| Responsibility | Existing owner | Planned change |
|---|---|---|
| Closed native protocol dispatch | `ice-ai/src/model.rs`: `WireProtocol` | Add a Codex variant and update all dispatch/parse/list paths. |
| Typed compatibility flags | `OpenaiResponsesCompat`, `StreamingProtocolCompat` | Reuse Responses flags; do not duplicate the compatibility struct. |
| Responses transcript conversion | `ice-ai/src/providers/openai_responses.rs` | Extract shared typed request construction without copying tool/history logic. |
| Responses stream state | Same module | Reuse terminal handling, tool reconciliation, usage, reasoning signatures, and typed internal events. |
| SSE parser and EOF flush | `ice-ai/src/providers/anthropic/sse.rs` | Reuse as-is unless a new regression demonstrates a defect. |
| Provider-private authentication | `ice-cli/src/provider/binding.rs`: `ExecutionBinding`, `RequestAuth` | Add refreshable Codex authentication without exposing tokens in model snapshots. |
| Credential file operations | `ice-cli/src/config.rs` | Extend the credential shape and reuse atomic, owner-only writes. |
| CLI auth commands | `ice-cli/src/auth.rs` | Add built-in fallback after extension ownership checks. |
| TUI OAuth interactions/workers | `ice-cli/src/interactive_tui.rs` and `interactive_tui/settings.rs` | Add built-in routing while preserving terminal input ownership. |
| Credential precedence and expiry | `ice-cli/src/credentials.rs` | Keep refreshable Codex credentials available for request-time refresh. |

The existing Responses processor already stops on `response.completed`,
`response.done`, and `response.incomplete`, handles failed/error events,
and errors on premature EOF. The shared SSE parser already flushes its last
frame. These require reuse and integration tests, not parallel implementations.

The existing auth writer provides atomic replacement, not transactional
read-modify-write locking. Existing OAuth UI/commands dispatch to extensions;
they do not yet implement built-in OpenAI login.

## Type-safety boundaries and invariants

1. Native routing uses `WireProtocol`, never string comparisons scattered
   through factories. Add `Api::OpenaiCodexResponses` and the corresponding
   `WireProtocol` variant; update `ALL`, conversions, parsing, default URL,
   provider form, factory, and reasoning replay policy. Test round trips.
2. Introduce concrete, private serializable Responses request types shared
   by OpenAI and Codex. Represent protocol-owned fields, reasoning options,
   and function-tool strictness explicitly. Keep genuinely dynamic tool
   schemas and existing transcript items as JSON; do not rewrite all event
   and history representations just to add Codex.
3. Select ordinary Responses versus Codex behavior with the existing closed
   protocol type (or exhaustive matching inside the shared builder), not
   a new configurable service or a collection of unrelated booleans.
4. Build Codex fields directly. Do not build an ordinary request and patch
   string keys afterward: ordinary Responses defaults output tokens and
   transforms strict tool schemas. Setting `strict:null` later cannot undo
   that transformation.
5. Preserve the existing sampling-parameter and hard-output-limit contract
   for ordinary Responses. Codex follows the reference's absence of arbitrary
   model `samplingParams` overrides; document this protocol difference. Enforce
   required transport/privacy fields such as `stream:true` and `store:false`
   and test option/header precedence and hard output caps.
6. Deserialize token responses into concrete structs. Convert them into a
   validated private Codex credential with a nonempty account ID and an
   explicitly named expiry in epoch seconds. Convert TS milliseconds only
   at the storage boundary. Use checked arithmetic for expiry calculations.
7. Never put credentials into public `Model` descriptors, transcripts,
   telemetry, URLs, or debug output. Redact secret-bearing runtime types.
8. Resolve and validate the execution binding before refreshing or attaching
   credentials. Preserve the existing protection against caller-supplied
   endpoint/model substitution.
9. Do not claim full compile-time `Model.api`/`Model.compat` correspondence:
   the current model erases that relationship. Validate combinations at the
   configuration boundary and reuse typed compatibility access thereafter.

## Stage 1 — shared typed request construction

Files: `ice-ai/src/providers/openai_responses.rs` and a private sibling
module if extraction improves ownership.

- Extract request DTOs and shared construction around the existing transcript
  conversion. Keep the public provider API unchanged.
- Preserve ordinary Responses wire output, strict schema conversion,
  constrained sampling, reasoning, custom parameters, and hard token limits.
- Separate strict schema preparation from tool serialization so Codex's
  default `strict:null` retains the original optional-property semantics.
- Expose only the crate-private helpers needed by the Codex provider.
- Keep the existing dynamic stream decoder; introducing a complete typed
  Responses event enum is outside this task.

Proof: existing Responses tests plus serialization regressions for ordinary
requests, tool schemas, custom parameter precedence, and hard output limits.
This stage must pass before adding Codex behavior.

## Stage 2 — Codex wire protocol and built-in catalog

Files: `ice-ai/src/model.rs`, `types.rs`, `providers/`, and the existing CLI
built-in provider/catalog/configuration paths.

### Request contract

- Provider ID `openai-codex`; API label `openai-codex-responses`.
- Base `https://chatgpt.com/backend-api`. Normalize base shapes ending in
  neither suffix, `/codex`, or `/codex/responses` to one responses URL.
- Bearer access token and `chatgpt-account-id`; never `x-api-key`.
- Match TS `originator: ice`, User-Agent construction, experimental Responses
  beta header, and SSE accept header. Use an existing platform helper if one
  fits; avoid a new dependency solely for an OS-release string.
- Optional session headers and prompt cache key follow TS bounds and cache
  retention rules. Test session headers separately from cache-key omission.
- Always `store:false`, `stream:true`, and system instructions, falling back
  to `You are a helpful assistant.` as TS does for an absent/empty prompt.
- Include encrypted reasoning content and default text verbosity `low`.
- Default tool choice to `auto` and enable parallel tool calls. The current
  common Rust options do not expose a direct tool-choice override; that
  separate API extension is deferred rather than encoded as arbitrary JSON.
- Match reference reasoning mappings, including off/minimal and supported
  high levels, using the catalog's typed thinking map. Keep the host's
  existing default thinking-level policy.
- Emit output-token and temperature options only when supplied, while
  honoring an explicit hard output cap.
- Function tools default to `strict:null`; explicit strict constrained
  sampling follows the reference and existing capability checks.

### Stream and errors

Reuse the existing Responses stream state and SSE parser. Extract the
transport/processor boundary only as needed to supply the Codex request,
terminal-error wording, and structured provider error context.

- Completed/done stop immediately; incomplete maps to length.
- A missing terminal event is an error, not successful partial output.
- Preserve tool-call IDs, results, reasoning signatures, usage and images
  through a multi-turn transcript.
- Format recognized quota/rate-limit failures using plan/reset metadata
  when available. Preserve non-quota authentication, validation and server
  errors. Missing metadata must not produce invented reset estimates.
- No new retries: follow the reference Codex default of zero retries.

### Catalog

Import the reference catalog's IDs, limits, modalities, costs and thinking
maps through the existing built-in catalog mechanism. Current reference:
`gpt-5.3-codex-spark`, `gpt-5.4`, `gpt-5.4-mini`, `gpt-5.5`,
`gpt-5.6-luna`, `gpt-5.6-sol`, and `gpt-5.6-terra`. Reverify before copying;
do not infer limits or expose unsupported thinking levels.

Proof: local faux HTTP tests for URL normalization, headers, typed body,
no-prompt defaults, absent output-token option, strict/optional tool schemas,
reasoning maps, cache/session behavior, terminal events with an open body,
byte-fragmented EOF variants, premature EOF, quota/non-quota errors, and a
second request containing a tool result plus reasoning signature. No live
credentials or network calls.

## Stage 3 — built-in OAuth and credential lifecycle

Primary owner: new private `ice-cli/src/oauth_codex.rs`, with focused changes
to `config.rs`, `credentials.rs`, and provider binding/factories.

### Storage compatibility

- Add optional serde field `accountId` to stored OAuth credentials; preserve
  existing API-key and extension OAuth entries.
- Normalize TS epoch-millisecond expiry values on deserialization (the
  reference compatibility rule is values above `10^12`), keeping existing
  seconds and zero/unknown semantics for other OAuth credentials.
- Validate Codex account ID and known expiry before use; derive a missing
  account ID from the access token for compatible imported credentials.
- Audit every OAuth constructor/pattern and extension conversion so login,
  refresh, or TUI persistence cannot accidentally erase `accountId`.
- Reuse the existing mode-0600 atomic writer. Document the additive field
  and normalized expiry representation; no edits to the user's real store.

### Login implementation

- Use reference client ID, authorize/token URLs, redirect URI
  `http://localhost:1455/auth/callback`, scopes, and authorize parameters.
- PKCE: 32 random bytes, unpadded base64url verifier, SHA-256 challenge;
  state: 16 random bytes encoded as hex. Reuse installed hashing/base64
  dependencies; add a direct MSRV-compatible randomness dependency only
  where required. Verify Rust 1.78 compatibility.
- Bind the callback before opening the browser. Default to loopback; do not
  add a public-bind environment option for a fixed localhost redirect.
- Accept only the callback path; reject missing code and mismatched state.
  Bound HTTP request size/time, handle provider error callbacks, and never
  log authorization codes or tokens. Return static success/error pages.
- Print the authorization URL and use the platform browser launcher. Browser
  launch failure is recoverable through manual paste.
- Support full redirect URL, `code#state`, query-string and bare-code input
  as in TS. Validate state whenever supplied; automatic callbacks always
  require matching state.
- If the port is occupied, use manual paste without treating an unrelated
  existing listener as ours.
- Browser/manual completion shares one cancellable flow. Cancel/drop the
  losing branch, close the listener and restore prompts on completion,
  timeout, or interruption. Do not leave a blocking stdin reader behind.
  EOF disables manual input while a listener remains usable; if neither
  path remains available, return a clear error.
- Share protocol work between CLI and TUI, not stdin ownership. TUI manual
  input uses its existing interaction UI and background result routing.
- Exchange code with form fields from TS. Require nonempty access/refresh
  tokens and valid `expires_in`. Decode the JWT payload with padded or
  unpadded base64url to extract the account claim. This extracts metadata;
  it is not local cryptographic authentication of the JWT.

### Refresh and concurrency

- Add refreshable auth to the private binding. Preserve extension priority
  and explicit credential override behavior; never attach a stale account
  ID to an independently overridden access token.
- Refresh within five minutes of expiry, using a 15-second network timeout
  and request cancellation. Failed refresh/persistence returns a normal
  provider error without token leakage or unsolicited stderr output.
- Coordinate independently created token holders and separate processes
  sharing one auth store. Acquire an interprocess refresh/store lock,
  reread and recheck expiry, then refresh and persist rotated credentials.
  Update in-memory state only consistently with persistence.
- All credential mutations must participate in the same read-modify-write
  coordination so unrelated provider entries are not lost and logout
  cannot be undone by an in-flight stale refresh. Atomic rename alone is
  insufficient. Use an existing locking dependency if suitable; otherwise
  select one small Rust-1.78-compatible cross-platform file-lock solution.
  Do not implement an ad hoc stale PID lock protocol.
- Scope lock release to RAII and keep synchronous lock waits off async
  executor threads. Errors release locks and leave the prior file intact.

Proof: fake token server, injected time/endpoints, temporary auth directory,
PKCE/state/parser cases, callback validation/cleanup, manual EOF and
cancellation, malformed token responses/JWT claims, imported TS entries,
refresh-window boundaries, rotated credentials, two independent holders,
separate-process contention, persistence failure, and logout/update races.
Use the repository's environment-isolation test patterns, not unguarded
parallel environment-variable mutations.

## Stage 4 — CLI/TUI integration and documentation

- CLI login/refresh: extension owner first, built-in Codex second, existing
  API-key fallback otherwise. Codex logout removes local credentials without
  requiring an extension or attempting an unsupported revoke call.
- Add built-in Codex to existing TUI login/provider discovery and background
  action routing. Reuse interaction, cancellation, persistence and reload
  behavior; ensure account metadata survives. Do not create a plugin just
  to access the existing UI, or read stdin behind the TUI.
- `auth check` remains local-only and keeps existing JSON readiness fields
  and exit semantics. Add useful OAuth expiry information without performing
  a hidden refresh or claiming expired credentials are ready.
- Add a subscription guide, update `ICE_PORTING.md` §9.1/§9.3 and the relevant
  gap record. Mark only implemented/verified SSE scope complete; do not mark
  WebSocket/device-code behavior ported indirectly through `AI-027`.
- Regenerate embedded documentation with `scripts/sync-embedded-docs.sh`.

Proof: built-in and extension precedence, CLI login/refresh/logout/check,
TUI discovery/login/cancel/reload, account ID preservation, and regression
coverage for extension OAuth and ordinary API-key login.

## Verification and completion

Run targeted tests after each stage, then all repository-required gates:

```bash
cargo fmt --all -- --check
RUSTFLAGS="-D warnings" cargo check --workspace --all-targets --locked
RUSTFLAGS="-D warnings" RUSTDOCFLAGS="-D warnings" cargo test --workspace --locked
sh scripts/sync-embedded-docs.sh && git diff --exit-code -- crates/ice-cli/embedded-docs
python3 scripts/test_prebuilt_release.py
```

Run `cargo +1.78 check` for MSRV-sensitive dependency/language changes.
Preserve complete gate output, exit statuses, warnings and errors in local
logs; report failures and unavailable tooling honestly. Do not claim Windows
verification based only on Linux execution.

With user approval for live authentication, use an isolated config directory
and the release binary to verify browser login, catalog discovery, a short
response, TUI login/cancellation, logout, and request-time refresh. Never
hand-edit the real auth store to force expiry. Automated tests must not use
real credentials or the live OpenAI service.

Done means the four stages pass their tests, existing protocols and extension
OAuth remain compatible, secrets remain private, and manual acceptance is
reported as passed or explicitly not run. Finish the plan review before
starting Stage 1; implement in dependency order and keep each stage buildable.

## Implementation evidence (2026-10-22)

- Shared typed Responses request construction retains the existing transcript
  and SSE processor; focused ordinary Responses and Codex transport suites pass.
- Built-in PKCE login, credential import, coordinated refresh, logout races,
  callback/manual cancellation and binding isolation have automated coverage.
- Review fixes make expired-but-refreshable credentials eligible for duplicate
  model selection and cancel login saves waiting on the auth-store lock.
- Workspace format/check/test gates pass with warnings denied; embedded-doc
  synchronization and all four release-packaging tests pass. Full gate logs
  are retained locally under `/tmp/ice-codex-*`.
- Live OpenAI login and Windows behavior remain unverified. The minimum-Rust
  check fails before compilation on the pre-existing `aligned` manifest;
  this implementation does not change that locked dependency.
- Rust keeps its existing epoch-second auth-store output while accepting
  imported epoch-millisecond expiries. The separate Rust store is not shared
  with the TypeScript installation.

## Explicit non-goals

WebSockets/pooling/continuation deltas, device-code login, zstd compression,
service-tier pricing/fast mode, Codex grammar/tool-search support, new retry
policy, other built-in OAuth providers, a generic OAuth framework, a wholesale
typed SSE rewrite, and generic `Model<TApi>` conversion across the repository.
