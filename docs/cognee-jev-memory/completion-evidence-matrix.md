# Frozen completion evidence matrix

Denominator: exactly 25 task IDs from the original plan (SHA-256 in `completion-audit.md`). Complete = 1, partial = 0.5, missing/blocked = 0. Baseline: 10.5 / 25 = 42%. Do not remove or weaken requirements to improve the score. Status below is the pre-change baseline; completion evidence will be added separately.

| ID | Frozen acceptance requirement | Baseline | Source evidence / remaining verification |
|---|---|---|---|
| M0.1 | Distinct skip/request/retrieval/injection/accepted/searchable outcomes; original operation/context IDs; actual wait and injection estimates; no transcript dumps | Partial | runtime observations and harness persistence callback exist; same-task late delivery attribution missing |
| M0.2 | Explicit native/MCP contracts and health; unsupported operations distinct; no guessed fallback | Partial | client contract fixtures pass; doctor reports unprobed MCP rather than connected catalog inventory |
| M1.1 | Cached repo/worktree identity, shared repo scope and restricted branch facts, unrelated repos separate, no per-prompt subprocess | Partial | common-dir dataset resolution exists; branch/revision applicability missing |
| M1.2 | Non-destructive known legacy aliases, one relevant fallback, bounded shared deadline, no cross-project leakage | Partial | empty-success read-through exists; tool fallback/deadline and provenance tests incomplete |
| M2.1 | Pure admission with explicit history bypass, sufficient-context zero-call fast path, stable task-event producers and original intent | Partial | phrase policy exists; expanded skill text, task events and context coverage missing |
| M2.2 | Bounded resident caches, coalesced event work, ordinary prompts read ready state only, separate retrieval/judgment freshness | Partial | lifecycle cache exists; delivery transaction and contextual revalidation missing |
| M2.3 | Immutable session/conversation-branch/Git/task/generation ownership; same-run next-boundary delivery; no stale injection or transform persistence | Partial | identity generation exists; host request boundary and task ownership missing |
| M3.1 | Candidate-level source/scope/time retained, object and snippet counts separate, context-only search | Partial | optional metadata tags exist; graph normalization and cache lose provenance |
| M3.2 | Drop routine/stale/superseded/wrong-scope/exact duplicates; useful short history; active context wins | Partial | basic selector exists; active context, branch identity and post-persistence novelty missing |
| M3.3 | One model-relative budget for checkpoint/recall/headers, complete envelopes, append-only safe boundary, no repeated growth | Partial | fixed 800-token estimate exists; model window and same-run append absent |
| M4.1 | Durable intent before dispatch, ambiguous acceptance never auto-replayed, scoped dedupe, concurrent/restart-safe recovery | Partial | intent/inflight/uncertain implemented; cross-process claim and recovery ownership missing |
| M4.2 | Source/span-validated explicit constraints/outcomes with JEV off; one automatic promotion policy; bounded optional candidates separate from durable intent | Partial | proposal tool exists; unrelated verifier, capacity failure and durable attribution ordering need repair |
| M4.3 | Newer same-scope correction supersedes; unrelated branches isolated; volatile facts reverified | Missing | unintegrated suppression commit available; branch/revision and correction tests required |
| M4.4 | Acceptance distinct from searchability; supported job handles retained; unsupported status explicit; revision coalescing/backoff; local curated fallback | Partial | accepted revision/backoff exists; response receipts and resident curated facts missing |
| M5.1 | Helper off constructs no client; selected explicit provider/model bound once/revision; invalid binding visible/nonfatal; no chat fallback | Partial | helper library/provider field exists; runtime model/config binding incomplete |
| M5.2 | Bounded redacted typed candidate-ID judgments; independent questions; uncertain/none; no invented memory text | Partial | helper unit fixtures pass; composition with attribution and caller not complete |
| M5.3 | 1.5 s bound, no per-prompt/tool loop/retry, shared provider quotas, stale queue expiry, cancellation on identity/settings/offline | Missing | unintegrated gate available; review reset and process-wide rate/concurrency behavior |
| M5.4 | Separate integration correctness from JEV usefulness; no quality claim/default enable without authorized held-out improvement | Missing | evaluation specification only; no execution artifact; live JEV prohibited by model restriction |
| M6.1 | Current bounded registry catalog with stable namespaced IDs/hash/availability; hidden/disabled excluded; no broad scans/reconnects | Partial | catalog/session snapshot exists; shared live consumer refresh and native tool representation need wiring |
| M6.2 | Advertised versus full/partial/changed/compacted skill coverage; correct branch/revision invalidation and explicit rendering | Partial | pure coverage implementation/tests exist but unused |
| M6.3 | Exact inventory local; obvious choices no JEV; ambiguous once/task; bounded advisory hints only | Partial | pure suggestion implementation/tests exist but unused |
| M6.4 | Distinct empty/timeout/cancel/unsupported/error/evidence results; callback completion not inflated; nonfatal tool errors | Complete | fresh Cognee fixtures plus existing MCP proxy error fixtures; rerun at handoff |
| M7.1 | Coherent atomic off/manual/legacy/selective presets; explicit read/write/helper provider/model/budgets/override warnings | Partial | preset/config command tests exist; helper controls and consumer behavior incomplete |
| M7.2 | Focused failure tests and chronological evaluation with denominators/harm/unknown/latency; isolated state; final CI checks | Missing | fresh baseline only; candidate evaluation and final gates not run |
| M7.3 | Local-only shadow/canary rollout; separate write/JEV gates; nondestructive data; instant rollback/cancel; docs | Missing | mode primitives exist; release/rollback proof and runbook missing |

## Integration update (2026-10-28)

The integrated implementation adds request-boundary persistence acknowledgment, durable admission receipts and recovery, fresh source/task/session checks before helper IO and admission, capability miss/compaction coverage, and a synthetic chronological replay. Selected regression evidence: 12 helper tests, 46 capability tests, 1 chronological replay regression, and the ignored synthetic replay report generator passed. The report covers 56 authored cases and five arms; it is not held-out user-quality evidence, does not evaluate a real JEV model, and does not establish production latency or usefulness. Embedded docs sync/diff and four prebuilt release tests passed.

Final integrated gates passed: format check; `-D warnings` workspace all-target check; `-D warnings` workspace tests (3,395 passed, 7 ignored); final embedded-doc sync/no-diff; prebuilt tests; and `git diff --check`. An earlier full workspace run exposed a stale expected Codex tool list which was corrected before the final pass. No live Cognee/JEV/provider requests or user data were used. The 25-task denominator remains frozen; this update does not mark all remaining partial/missing tasks complete.

## Final evidence

Record the final commit, exact commands and selected test counts, failed and skipped checks, fixture provenance, latency measurement scope, and remaining deployment-only gates before claiming readiness.
