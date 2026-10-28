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

## Current acceptance evidence (2026-10-28)

The baseline table above is historical, not current status. The source snapshot tested is `b3c71e883b780a15bdc57b512d70be62657ba67f`; source and test paths below are relative to `crates/ice-cli/src/` unless otherwise stated. The whole task remains **partial** whenever a listed acceptance gap is not established. Relevant changes are included in the tested snapshot; short SHAs identify integration commits, not necessarily the first introduction of every feature.

| Task | Integrated behavior and focused regression evidence | Status / remaining proof |
|---|---|---|
| M0.1 | `cognee/runtime.rs`, `observe/recorder_tests.rs`; persistence-failure and late-recall tests; request-boundary integration `7a789f4c` | Partial: complete operation-level attribution not independently demonstrated. |
| M0.2 | `cognee/client.rs`, `runtime.rs` doctor; 404/422 and unsupported-status fixtures | Partial: connected MCP Cognee capability inventory and live contract unverified. |
| M1.1 | `cognee/identity.rs`, `config.rs`; linked-worktree identity fixtures | Partial: complete cross-branch/revision and backend metadata proof remains. |
| M1.2 | `cognee/lookup.rs`, `runtime.rs`; fallback and bypass fixtures | Partial: alias cancellation/deadline and provenance coverage not complete. |
| M2.1 | `cognee/policy.rs`, `request.rs`; `negated_history_phrases_do_not_trigger_automatic_recall` (`b3c71e8`), raw-intent and fast-path tests | Partial: full task-event and sufficient-context acceptance coverage not established. |
| M2.2 | `cognee/coordinator.rs`, `request.rs`; prepared packet, persistence ack, expiry/cancellation tests | Partial: independent full lifecycle and live service proof outstanding. |
| M2.3 | `cognee/validation.rs`, `request_tests.rs`; stale task and branch/correlation tests (`6297a800`) | Partial: every generation/config transition not covered. |
| M3.1 | `cognee/client.rs`, `evidence.rs`, `lookup.rs`; fallback provenance tests | Partial: real graph normalization/source semantics unverified. |
| M3.2 | `cognee/evidence.rs`, `delivery.rs`, `suppress.rs`; branch/expiry/duplicate/active-context and suppression tests | Partial: live metadata and graph paraphrases unverified. |
| M3.3 | `cognee/delivery.rs`, `request_tests.rs`; tiny-window, shared-budget, complete-envelope and append-failure tests | Partial: full host transition coverage remains. |
| M4.1 | `cognee/admission.rs`, `queue.rs`; receipt, queue reservation, recovery, duplicate, lock and uncertain-write tests (`5e289a87`, `8d479121`) | Partial: production filesystem and remote completion semantics unverified. |
| M4.2 | `cognee/source_validation.rs`, `validation.rs`, `runtime.rs`; invalid/negated/stale-source and valid helper admission tests (`440edddc`, `6297a800`) | Partial: all historical promotion paths and live integration require review. |
| M4.3 | `cognee/suppress.rs`, `admission.rs`; accepted correction and expiry fixtures | Partial: volatile re-verification and cross-branch production coverage remain. |
| M4.4 | `cognee/runtime.rs`, `admission.rs`; acceptance/backoff/coalescing and local curated receipt tests | Partial: remote job completion/searchability endpoint unsupported or unverified. |
| M5.1 | `cognee/config.rs`, `helper.rs`, `runtime.rs`; explicit binding, off and nonfatal invalid provider tests | Partial: live provider endpoint behavior not tested; helper remains off by default. |
| M5.2 | `cognee/helper.rs`; bounded typed IDs, redaction, invalid input and abstention fixtures | Partial: complete combined client/service behavior not verified live. |
| M5.3 | `cognee/helper_gate.rs`, `runtime.rs`; shared quota, cache, stale flight and shutdown tests | Partial: settings/revision races and live provider limits remain unverified. |
| M5.4 | `offline_replay_tests.rs`, `offline-evaluation.md`; fake transport, 56 authored cases | Blocked: no JEV judgment executed; no held-out usefulness or latency evidence. JEV stays off. |
| M6.1 | `capability_catalog.rs`, `capability_service.rs`; live registry snapshots and capability tests (`17db78cc`) | Partial: all host refresh and real MCP lifecycle transitions not proven. |
| M6.2 | `skill_coverage.rs`, `capability_service.rs`; full/partial and compaction-aware tests | Partial: all restored-session and live lifecycle transitions not proven. |
| M6.3 | `capability_suggest.rs`, `capability_service.rs`; bounded advisory/miss tests, no execution | Partial: task-transition and UX coverage incomplete; no JEV ranking evaluation. |
| M6.4 | `cognee/lookup.rs`, `client.rs`, `mcp/proxy_tests.rs`; empty/error/timeout/cancellation fixtures | Locally verified; remote contracts remain deployment-dependent. |
| M7.1 | `cognee/config.rs`, `command.rs`; mode presets, helper settings and override tests | Partial: interactive TUI and real settings review outstanding. |
| M7.2 | `offline_replay_tests.rs`; 56 cases, five labelled arms, explicit ablation, failure-inclusive local timings (`3173b8b4`, `82dc9ed`, `b3c71e8`) | Partial: D does not invoke JEV; labels do not prove all harm metadata; TTFT, utility and background latency unknown. |
| M7.3 | `cognee/config.rs`, `command.rs`; manual/canary/selective/legacy modes, cancellation fixtures | Blocked: no real shadow/canary or rollback drill and no production rollout proof. |

Counts, commands, synthetic limitations and rollback guidance are in `merge-readiness-report.md` and `offline-evaluation.md`. Do not interpret synthetic false-positive counts as real-world admission quality.
