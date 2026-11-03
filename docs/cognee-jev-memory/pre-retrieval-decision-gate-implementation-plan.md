# Pre-retrieval Cognee decision gate — implementation plan

Date: 2026-10-30
Status: PROPOSED — planning artifact only; no implementation or live API testing performed
Target: `/home/mewtwo/Zykairotis/ice-rust`
Owner objective: prevent unnecessary Cognee *requests*, not merely suppress insertion of retrieved memory

## Objective and success state

Introduce a distinct opt-in read policy (`gated`) that runs cheap local eligibility checks and, only when useful, a bounded TypeSafe/JEV decision **before** initiating Cognee I/O. Explicit requests for older decisions always remain retrievable; the main agent retains `cognee_search` as an on-demand escape hatch.

Success criteria:
- A self-contained turn selected as `skip` causes **zero Cognee HTTP requests** and normally zero JEV requests.
- A JEV `none` or `session` result causes **zero Cognee HTTP requests**, rather than cancelling an already-started request.
- An unambiguous explicit history request retrieves scoped Cognee evidence without a JEV veto.
- A relevant admitted memory is bounded, provenance-filtered and injected only once when useful.
- No change to write admission, permanent-memory queues, TypeScript ICE identity, provider interfaces, existing `legacy`, `event`, `on_demand`, or `judged` semantics.
- Actual retrieval suppression, usefulness, false-negative rate and end-to-end latency are independently measured before changing defaults.

## Current-state evidence (2026-10-30 workspace inspection)

- `crates/ice-cli/src/cognee/config.rs:82-113`: `ReadPolicy::{Legacy, OnDemand, Event, Judged}`; default is `Legacy` at lines 219-240; `MemoryMode` presets configure read and write policies independently (lines 146-185 and 248-263).
- `crates/ice-cli/src/cognee/policy.rs:96-134`: the `legacy` admission scans for four substantive words; `event` uses history phrases; `judged` always admits the prompt for speculative recall.
- `crates/ice-cli/src/cognee/request.rs:212-279`: `context_for_request` dispatches `judged` and `legacy` separately before the event-driven coordinator. For `event`, explicit historical requests may retrieve foreground evidence and task changes/resumes can prefetch; see lines 360-472.
- `crates/ice-cli/src/cognee/runtime.rs:3990-4307`: `recall_for_prompt` builds Cognee retrieval at 4094-4113 **before** running the JEV judge concurrently at 4114-4223. A `none` verdict can suppress injection but cannot guarantee no Cognee dispatch.
- `crates/ice-cli/src/cognee/helper.rs:270-330,464-494`: `Helper::bind_read_gate`, `judge_read_gate`, and the typed `needs: none|session|memory` + `incomplete` request already exist. The current 0.75 choice-confidence threshold is not established as optimal for recall. Keep the existing TypeSafe System One client and explicit provider/model binding.
- `crates/ice-cli/src/cognee/command.rs:141-218,255-276`: /cognee mode, read, helper, recall, doctor, and search commands already exist.
- `crates/ice-cli/src/cognee/judged_tests.rs:61-101`: the test **expects one Cognee transport request even when JEV chooses `none`**. New tests must assert the opposite for `gated`.
- `docs/cognee-jev-memory/judged-recall.md:1-60`: accurately states speculative concurrency and the limitation; do not silently redefine `judged`.
- `docs/cognee-jev-memory/completion-evidence-matrix.md:1-5,26`: previous selective-memory work was scored 96.0% on its own frozen 25-unit denominator; independent held-out JEV effectiveness was still blocked. That score does not count this new `gated` feature as complete.

## Scope, non-goals, constraints

In scope: a new opt-in policy; read-only eligibility/decision routing; Cognee-dispatch suppression; bounded fallback/caching/telemetry; offline tests, docs and evaluation protocol.

Out of scope: rewriting Cognee storage, live migrations, changing memory writes, introducing a new decision provider or generative model, running real service calls, making skill discovery a prerequisite for recall, or switching the default read policy before evaluation.

Keep these invariants:
1. Honor `AGENTS.md`: Rust workspace only; never touch TS ICE config/binaries; feature work should be branched from main, with no push/commit/release without instruction.
2. Preserve session/dataset/worktree identity, generation cancellation, previously acknowledged memory packets, and read-only tool compatibility.
3. `AUTO_RECALL_SCOPE`, provenance, revision, branch and expiry filters remain in force after admission.
4. Route no secrets or full transcripts to TypeSafe. Existing heuristic redaction is useful but not guaranteed; treat remote decision use as opt-in disclosure.
5. No production state or live service should be used for tests. Use existing faux decision transports and temporary Cognee data.
6. Do not confuse a network timeout, an absent result, a negative JEV answer, and a confidently irrelevant memory. Maintain separate outcomes.

## Target design

```text
Request boundary (first provider request for a new user turn)
   |
   +-- explicit history intent? ------------ YES --> Cognee scoped search
   |
   +-- no memory exists / fully local? ----- YES --> skip (zero network)
   |
   +-- task event / possible history gap? -- NO --> skip; model retains cognee_search
   |
   +-- bounded JEV decision (optional, before Cognee)
              |
              +-- memory supported --------- YES --> Cognee scoped search
              +-- none/session ------------ NO  --> skip (zero Cognee calls)
              +-- uncertain/unavailable ---------> event fallback or on-demand
```

For correctness, explicit user history needs and explicit `cognee_search` calls bypass JEV. Never allow `none` from a fallible classifier to override an explicit request. Reuse resident or already-cached evidence when relevant, before scheduling new Cognee I/O. Local short-circuits must be conservative: a prompt length or a single history keyword is not proof that no prior constraint is relevant. Task boundary/compaction detection should be separated from mere turn count.

Keep two read modes rather than quietly changing the existing one:
- `judged`: existing speculative concurrent recall + JEV, optimized for latency when retrieval is likely.
- `gated`: local admission, JEV if ambiguous, Cognee after positive admission; optimized for avoiding unnecessary backend requests.

A small resident catalog of durable project facts can improve local admission and routing without searching the full graph. Reuse existing resident memory/coordinator and invalidate only on actual project identity, revision or fact-policy changes. Do not block every turn waiting for a remote classifier.

Decision schema: retain `needs: none|session|memory`. Use the `incomplete` score for observability only unless evaluated. Inspect the per-option probabilities as well as choice confidence; a 0.75 concentration threshold is not a measured false-negative constraint. Evaluate candidate thresholds with independently labeled historical-memory needs, counting abstentions and errors. Never claim that JEV knows what is stored in Cognee without a bounded local candidate index or retrieved shortlist.

Provider/model binding must remain explicit and valid for TypeSafe System One. Pin the evaluated model version for thresholds. Use shared helper quotas, singleflight, cancellation and cache; no chat-model fallback. A new policy should offer an explicit bounded deadline. JEV-on-every-turn can itself be a significant latency regression; call it only for unresolved eligible cases.

## Dependency order and atomic tasks

### Stage 1 — explicit configuration surface

- [ ] **G1.1 Add `ReadPolicy::Gated`** in `crates/ice-cli/src/cognee/config.rs` with `parse/as_str`, serde roundtrip and environment overrides; keep `Legacy` as default and existing named `MemoryMode` presets unchanged, yielding `custom` when `gated` combines with existing writes. **Acceptance:** `/cognee read gated` persists and reports the effective policy; all old config documents parse unchanged. **Verify:** focused config and command tests (`cargo test -p ice-cli memory_modes`, new gated config test).
- [ ] **G1.2 Extend helper binding** in `helper.rs:bind_read_gate` to accept the new `Gated` mode while preserving explicit TypeSafe protocol enforcement, model identity and helper-disabled behavior. **Acceptance:** unconfigured helper abstains; no guessed provider or chat fallback; `judged` binding is unchanged. **Verify:** new helper-bound/unbound tests plus existing `judged_config_roundtrip_and_bounded_typed_request`.

### Stage 2 — ensure admission precedes all Cognee I/O

- [ ] **G2.1 Factor `recall_for_prompt`** in `runtime.rs` into an admission-free, scoped retrieval function and the existing `judged` concurrent orchestration. Avoid calling the whole `judged` path from `gated`, which would re-run the decision and start I/O prematurely. **Acceptance:** each path dispatches no more than one retrieval per admitted request, preserves dedupe, fallback dataset, cancellation, cache and observability contracts. **Verify:** existing `judged_read_gate_choices_and_event_fallback` and focused legacy recall tests.
- [ ] **G2.2 Add `gated` branch to `request.rs:context_for_request`**. Order local explicit-history checks, active-session sufficiency and bounded ambiguous-eligibility checks before any helper or `recall_with_fallback` future is constructed/polled. If JEV returns `memory`, call the admission-free retrieval function; if `none/session`, return None. **Acceptance:** negative case asserts fake Cognee transport `sent().len() == 0`; positive case asserts exactly one request and bounded injected context. **Verify:** new `gated_skips_network_on_no_memory` and `gated_retrieves_after_positive_decision` tests.
- [ ] **G2.3 Preserve direct retrieval and task-scoped context**. Keep `/cognee search` and `cognee_search` functional; use scoped project/task/query anchors rather than whole-prompt full-graph searches. Guard against stale task branches and worktree moves. **Acceptance:** explicit previous-decision requests cannot be vetoed; disabled or changed identity yields no cross-project evidence; no additional automatic capture or improve is triggered by this change. **Verify:** request/lookup/task/provenance tests with fake transports and temporary directories.

### Stage 3 — uncertainty, budgets and observability

- [ ] **G3.1 Define and implement fallback policy** for timeout, malformed answer, provider disabled, low choice confidence, input too large, and rate limit. Explicit history must still retrieve; ordinary non-historical turns use the existing event rule or on-demand escape hatch. **Acceptance:** failure never becomes a silently confident `none`; no repeated/retried foreground JEV calls; no later Cognee send after skip. **Verify:** table-driven fake decision/error tests.
- [ ] **G3.2 Add bounded gate cache and cancellation** keyed by relevant session/project/task/branch, original prompt/available preceding-assistant excerpt, model version and policy revision. Invalidate on identity change, compaction/history mutation and cancellation. **Acceptance:** repeat turns do not create redundant remote calls and a stale positive verdict cannot trigger retrieval in a new task. **Verify:** fake clock/singleflight/cancellation tests.
- [ ] **G3.3 Expose independent counters** for local skip, JEV start/result/abstain, actual Cognee HTTP send, empty/mismatched evidence, retained/injected evidence, false-negative escape-hatch use and total foreground wait. **Acceptance:** no inference of request savings from `recallCancelled` or `recallUsed` alone; doctor and observability distinguish no-send from canceled-in-flight. **Verify:** observation assertions with three separate test outcomes: no-send, sent-and-cancelled, sent-and-used.

### Stage 4 — evaluation and rollback

- [ ] **G4.1 Add chronological, scoped offline fixtures** using past-versus-present availability timestamps, self-contained tasks, short ambiguous continuations, explicit history requests, important project constraints, task resumes, corrections, conflicting/stale facts, and service failures. Separate labeling by owner from model output. **Acceptance:** every arm includes misses, false positives, abstentions, and latency including failures; future memories cannot leak into earlier queries. **Verify:** targeted `cargo test -p ice-cli gated_` and offline replay runner on temporary state.
- [ ] **G4.2 Compare four baselines**: `legacy`, `event`, existing `judged`, and new `gated`; include memory-off/on-demand as diagnostic controls. Measure (i) Cognee request count, (ii) missed required historical facts, (iii) irrelevant injection, (iv) total TTFT and p50/p95 latency, (v) JEV calls and cost, (vi) correction/repeated investigation. **Acceptance:** any proposed production default change requires held-out evidence and documented false-negative trade-off. **Verify:** reviewed aggregate report only; do not call synthetic timings live performance.
- [ ] **G4.3 Update** `docs/cognee-jev-memory/judged-recall.md`, source user guide, and any generated embedded copy via `sh scripts/sync-embedded-docs.sh`. Document why `gated` is strictly sequential, unlike `judged`, and how to roll back with `/cognee read event` or `/cognee read on_demand`. **Acceptance:** commands and log output match runtime; no source/public API break. **Verify:** docs sync diff check, `cargo fmt --all -- --check`, focused `cargo check -p ice-cli --locked`.

## Verification matrix

| Scenario | JEV calls | Cognee calls | Required behavior |
| --- | ---: | ---: | --- |
| Self-contained task with reliable local skip | 0 | 0 | No injected memory |
| Ambiguous task, JEV says none/session | 1 | 0 | No Cognee send, no injection |
| Ambiguous task, JEV says memory | 1 | 1 | Scoped retrieval; filter stale/wrong-project facts |
| Explicit request for older session decisions | 0 | 1 | Direct scoped retrieval, no JEV veto |
| Current-session continuation already answerable | 0 | 0 | Use live context |
| JEV timeout on ordinary ambiguous task | 1 | 0 unless fallback event rule triggers | Record abstain; on-demand tool remains |
| User-initiated cognee_search | 0 | 1 | Manual tool works independently |
| Cancellation, task/branch changes | <=1 | 0 if not yet admitted | No late send or stale injection |

Counts above are *target invariants* for test fixtures, not measurements of the deployed service. Avoid assuming automatic local "sufficiency" can be decided perfectly: test false-negative prompts where a prior user preference or unresolved decision matters without explicit historical language.

## External reference decisions

- Adopt official TypeSafe System One typed decisions (`state`, `questions`, Choice/Noul, model output and explicit key), not unrelated independently branded gateways. Source: https://docs.typesafe.ai/api/system-one (accessed 2026-10-30).
- Adopt Cognee evidence-only retrieval and scoped queries; `only_context` avoids an additional answer-generation step but is not zero inference or universal raw-fact representation. Source: https://docs.cognee.ai/guides/search-basics (accessed 2026-10-30).
- Reuse existing TypeSafe decision client, runtime helper and fake transports; do not duplicate a new SDK.
- Reject mandatory JEV calls on every turn, broad unscoped search by default, use of Choice confidence as empirical correctness, and silently permitting a gate to veto explicit retrieval.

## Risks and rollback

- **False negatives:** missed historical facts can be more damaging than unneeded search. Monitor owner-labeled recall misses and allow direct `cognee_search` escape hatch. Keep old policies intact until measured.
- **Latency:** strict decision-first mode adds JEV latency on ambiguous requests that genuinely need memory; the old parallel `judged` mode may be faster for positive cases. Compare end-to-end percentiles.
- **Privacy:** remote TypeSafe sees bounded user prompt excerpts. Require explicit provider configuration and display that disclosure behavior.
- **Operational:** never change the current write policy or delete Cognee queues/datasets when changing read mode. Rollback is `/cognee read event` (selective automatic) or `/cognee read on_demand` (no automatic reads) on the same compatible binary.

## Definition of done

All G1–G4 leaf tests pass; no unrequested code or storage mutations; selected no-memory cases prove **zero actual Cognee sends**; explicit history and manual tools still retrieve; no scope/branch/privacy regressions; docs and CLI agree; at least one held-out owner-labeled evaluation explicitly quantifies false negatives and end-to-end latency before default or deployment claims. Until then mark the policy opt-in and effectiveness unproven.
