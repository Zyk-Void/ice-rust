# Corrected plan: typed Decision APIs

Date: 2026-10-26. Baseline: `e8c4d70`. Status: D1 codecs, D2 transport (`ice_ai::decision::client`, `providers` feature) and D3 host configuration (`models.json` `decisionProviders`, `ice_cli::decisions`) implemented and verified offline against local HTTP fixtures. No live API call has been made. No built-in feature uses decisions yet; plugin ABI exposure is not implemented.

## Scope correction

The requested feature is Decision APIs: explicit questions evaluated against shared
input, with typed answers, probabilities, confidence where defined, and usage.
It is not generic JSON-schema output from a chat model. The previous
structured-output plan is superseded and is not part of this change. Its runtime
changes, including unrelated strict-tool fixes, were archived and removed before
starting this implementation.

Research:
- [TypeSafe/Jev and independent gateway](decision-api-jev-research.md)
- [OpenAI and Inception](decision-api-openai-inception-research.md)

## Verified contracts and boundaries

| Service/protocol | Operation | Shape | Credential |
| --- | --- | --- | --- |
| OpenAI Decisions | `POST /v1/decisions` | `input`, ordered typed `questions`; ordered `answers` | Bearer API key |
| TypeSafe native System One | `POST https://api.typesafe.ai/v1/systemone` | `state`, named typed question map; named answers | Opaque Bearer API key |
| Inception Decisions | `POST https://api.inceptionlabs.ai/v1/decisions` | `state`, named typed question map; named answers | Bearer API key |
| Independent JevTypeSafeAI gateway | Separate documented gateway routes | Do not assume native TypeSafe endpoint or credentials | Separate gateway key |

The independent JevTypeSafeAI site explicitly disclaims affiliation with TypeSafe
AI. Its key prefix and billing rules must not be attributed to the native API.
Official TypeSafe console sign-in provisions API keys; no public API OAuth/token
exchange contract was found. Reuse opaque bearer storage/transport, not JWT
parsing or a fabricated login flow.

## Invariants

1. Decision calls are one-shot typed operations, separate from assistant token
   streams, chat history, tools, session messages, and generic response schemas.
2. A decision model must not masquerade as a chat model or silently fall back to
   a chat completion. No inference by provider/model name.
3. Preserve protocol-specific question/answer semantics. Common concepts do not
   imply identical wire shapes, limits, refusal behavior, or score criteria.
4. Keep confidence separate from predicate probabilities and categorical
   distributions. Do not invent a confidence formula or threshold policy.
5. Validate response correlation, answer discriminants, allowed choices,
   probability ranges, finite scores, and distribution structure before returning
   typed success. Per-question refusal is not a successful classification.
6. Never coerce malformed replies, repair output, drop unanswered questions, or
   relabel a different question type as compatible. Preserve resolved model and
   usage without requiring a model alias to equal its resolved version.
7. No provider secrets in catalogs/errors/logs or credential-bearing URLs.
   Authentication remains explicit and provider-isolated.
8. Custom endpoints select an explicit decision wire protocol. Shared HTTP
   transport is not evidence that two decision protocols are interchangeable.
9. Vendor defaults belong in external provider packages. Core may own genuinely
   reusable protocol/types support, not vendor-name branches in the CLI.
10. No paid calls, real authentication, release, commit, or push without approval.

## Ordered slices

### D1 — typed contracts and offline codecs

Add an additive decision module in `ice-ai`, independent of `Provider::stream_simple`.
Use protocol-native typed request/response representations where a lossless shared
model is not established. Shared probability/usage/identity types may be factored
only when their semantics match. Start with documented predicate/noul, choice,
and score. OpenAI refusal remains explicit; TypeSafe preview box/images and
Inception extract (guide/OpenAPI mismatch) remain unsupported or separately typed
without claiming portable support. Explicitly state the initial input subset.

Before coding, inspect current architectural seams and primary SDK/OpenAPI details
for required/optional fields. Define request-aware response validation and safe
errors. Reject invalid protocol/type combinations before any transport. Add
fixtures from the documented contracts and adversarial tests. No new dependencies
unless existing dependencies cannot own the requirement.

Done when the actual Decision API request/answer shapes round-trip with types,
invalid/cross-protocol answers fail safely, existing chat APIs remain untouched,
and default/provider-feature builds pass. This slice does not claim HTTP support.

### D2 — reusable one-shot protocol transport

Implement explicit decision clients with timeout/cancellation and existing HTTP
error/retry primitives where appropriate. Preserve complete result metadata and
per-question failures. Do not pretend there is an SSE/token stream. Reuse caller
provided base URL, model ID, and private auth; no environment fallback to another
provider's key. Distinguish supported decision protocols without adding chat-only
`Api` variants merely to route decision calls. Decide transport ownership after D1
review; keep native TypeSafe and independent gateway separate.

Done when local HTTP fixtures prove exact endpoint/body/auth, error handling,
response bounds, cancellation, and zero network on invalid input. No live calls.

### D3 — custom model configuration and authentication integration

Trace the existing model registry, provider factories, external package declaration,
and private execution bindings before extending them. Add explicit decision
capability/model routing that cannot be selected accidentally as chat. Support
custom wire model IDs and endpoints, independent keys, and old config defaults.
Reuse host credential storage and API-key auth commands; document that a browser
console login is not an API OAuth integration. No named OAuth without its contract.
Do not alter the C ABI without approval; old plugins must not silently accept a
capability they cannot execute. Keep vendor catalogs/packages outside the CLI.

Done when configuration reaches private execution, endpoint/key isolation and
reload hold, and unnegotiated plugins fail explicitly. If the current chat-only
registry cannot express decision models safely, keep an explicit library-only
API until a reviewed extension is ready rather than forcing a chat abstraction.

### D4 — examples, documentation, and final proof

Provide offline runnable decision examples and exact custom endpoint/key examples
using placeholders. Clearly state supported question types/input modalities for
each protocol and missing preview features. Mark historical schema-output material
superseded. Regenerate embedded docs if their source set changes.

Run repository gates from `AGENTS.md`, plus relevant `providers` feature tests and
Rust 1.88. Preserve complete stdout/stderr/exit statuses, including initial failures.
A skipped external package test or unrun full workspace suite is not a pass.

## Verification matrix

- Exact OpenAI array and System One-style map encodings; named answer correlation.
- Predicate/noul, categorical selection and distributions, fractional ordinal
  score, confidence preserved separately; per-question refusals.
- Wrong type, unknown/duplicate/missing question names, missing/extra choices,
  invalid distributions/ranges, non-finite/overflow numbers, malformed/trailing
  JSON, oversized bodies; no raw state/answers/secret keys in default errors.
- Explicit per-protocol request limits; no automatic lossy normalization of
  instructions, typed choices, score labels, images, or provider extensions.
- Custom endpoint/model/auth binding, no chat fallback, local HTTP failures and
  cancellation; ordinary chat payloads and source APIs unchanged.

## Current evidence

- Three user-supplied Decision API pages fetched successfully.
- Official TypeSafe docs and Inception index discovered and reviewed.
- JevTypeSafeAI `llms.txt` returned HTTP 500; its API docs loaded successfully.
- Generic web search returned no results; direct primary documentation succeeded.
- Off-target runtime archive and restoration verified by the cleanup agent;
  formatting, diff check, and warning-denied offline `ice-ai` check passed.
- D1 adds protocol-specific offline request/answer codecs and request-aware validation.
  D1 alone claimed no HTTP transport, registry/auth integration, or
  live-provider proof.
- D1 implementation verification reports 19 focused decision tests, default and
  provider-feature `ice-ai` tests, locked workspace check,
  Rust 1.88 check, formatting, and embedded-doc checks passing.
- The warning-denied default-only `ice-ai` check fails on seven dead-code warnings
  in unchanged `strict_schema.rs`; the warning-denied provider-feature check
  passes. The full workspace test gate has not been established as passing.
- Local regression tests reproduced acceptance of an all-zero distribution and
  rejection of a valid nested score legend before the fixes. Now 25 focused
  Decision tests pass, along with default/provider `ice-ai` tests,
  warning-denied locked all-target workspace check, Rust 1.88 provider
  check, formatting and diff checks.
- Fixes enforce numerical consistency with explicit local tolerance, include
  criteria containers in depth accounting, allow native response-envelope
  headroom, and stop native serialization at the body limit. OpenAI choice
  argmax checking was not added without an explicit documented guarantee.
- D2 transport should reuse `ice-ai`'s HTTP builder for proxy/idle policy, retain
  explicit endpoint/key binding, disable redirects for credential isolation, and
  avoid automatic retries of billable POSTs without an established safe replay
  contract. This is a design boundary, not implemented transport support.

## D2/D3 implementation (2026-10-26)

- `ice_ai::decision::client`: `DecisionProtocol` (`openai-decisions`,
  `typesafe-systemone`, `inception-decisions`), sealed `DecisionRequest` per
  codec, and `DecisionClient::decide(&request, timeout, &cancel)`. Protocol
  mismatch and invalid requests fail before network I/O. Bearer auth only,
  `authorization` cannot be overridden by headers, redirects disabled, no
  automatic retries, whole-exchange timeout, cancellation, 32 MiB streamed
  body bound, and errors carry HTTP status only (no body, key or content).
- `models.json` `decisionProviders` (separate from chat `providers`), modeled in
  `ModelsConfig` so the interactive provider form no longer drops it on save.
  A bad `protocol` string disables only that entry.
- `ice_cli::decisions::decision_client(id)`: key precedence is a stored
  `auth.json` API key, then the entry `apiKey` (`$ENV`/`!command` allowed); no
  environment fallback, no OAuth reuse; honors `httpIdleTimeout`.
- Verification: 6 transport tests (local TCP fixtures: exact path/body/auth,
  401/500, redirect, invalid answer, oversized body, timeout, cancellation),
  4 configuration tests, warning-denied workspace check, Rust 1.88 `ice-cli`
  check, full workspace tests (2678 passed, 0 failed, 6 ignored), embedded-doc
  sync and prebuilt-release script.
- Not done: live provider proof, plugin ABI exposure, an agent tool or CLI
  command, provider error-body diagnostics.
