# Harness reliability and quality improvement plan

Date: 2026-11-03
Status: implementation in progress; acceptance tracked by milestone below
Evidence: [Harness audit](harness-audit-2026-11-03.md), especially §11
Audited revision: `2fcc1618c0e81aec3e465c9fb24669a0d5655d84`

## Objective and operating rules

Make the harness safer, more predictable, and measurably more useful without a
large rewrite. Address every audit workstream, but do not treat historical
symptoms or plausible mechanisms as reproduced current defects.

This plan authorizes no release, publication, production-data migration, or
changes to live settings. Work item IDs below are local planning IDs, not GitHub
issues or promises that PRs have been created. Ownership identifies code areas,
not assigned people. Reconfirm source behavior when implementing against a newer
revision.

Rules for every work item:

- Add a failing regression case before fixing a reproduced defect. For a
  hypothesis, reproduce or disprove it first; a passing invariant test and a
  documented explanation can close an investigation without a production change.
- Reuse existing budgeting, recovery, memory, and permission mechanisms. Do not
  introduce a new crate, framework, or plugin ABI change without demonstrated need.
- Use faux providers, fake/local transports, synthetic secrets, isolated temporary
  agent directories, and sanitized fixtures. Never commit private histories or
  depend on the audit's temporary snapshot remaining available.
- Preserve permission boundaries. Never solve a denial by widening roots or
  silently switching to a more privileged tool.
- Preserve unsafe-tool replay restrictions and honest unknown outcomes. Exactly
  once execution cannot be promised for arbitrary external side effects.
- Keep user cancellation and interactive progress independent of optional
  telemetry and background memory work.
- Separate small fixes from format changes and architecture changes. Each logical
  patch should build and pass tests independently. Keep review-sized changes;
  split large work items into tests, implementation, and migration when necessary.
- Keep security findings private until the owner approves disclosure. No upstream
  issue, PR, push, commit, or release is implied by this plan.

## Execution contract (revised after implementation probes)

The H01–H18 scope below remains authoritative. Track implementation separately
from acceptance in the execution log; an unchecked program item does not mean
no code exists, and passing a focused test does not close a program item.

### Status and evidence rules

Each item records: owner, investigation status, patch status, focused verification,
integration verification, residual acceptance, and external dependency. Use
`not started`, `investigating`, `implemented / focused verified`, `integration
verified`, `accepted`, or `external dependency`; never derive a completion
percentage from checkbox count. Only acceptance closes the H-item.

Every verification record must name the source revision plus dirty-tree state,
command, selected test names/count, exit status, ignored tests, warnings/errors,
and artifact path. Zero selected tests are not verification. Retain failing-before
and passing-after logs. A compilation error or broken fixture is an agent/test
error, not evidence that the intended production regression reproduced.

### Reviewable milestones

| Milestone | Scope | Exit evidence |
| --- | --- | --- |
| M0: trustworthy green gate | H01, baseline fixture failures, current patch verification | Diagnose all seven known baseline failures; preserve behavior assertions; full warning-denying gates and final diff review. Record newly introduced failures separately. |
| M1: demonstrated local defects | H02–H06, local portions of H10/H13–H15 | Regression matrix across provider, telemetry, acknowledgment, queue, and artifact boundaries; final serialized/wire behavior tested; compatibility notes for every public API/format change. |
| M2: recovery and lifecycle contract | H07–H09; H11/H15 dependent cleanup | Written ownership/deadline contract before patching; disk faults, repeated crash, concurrent independent owners, forced termination, and unknown-outcome scenarios. |
| M3: bounded optional services and analysis | H10–H16 remaining local work | Causal memory outcomes, bounded work/retention, permission-owner matrix, private read-consistent export, and pressure measurements. External dependencies remain explicit. |
| M4: semantic and task evaluation | H17–H18 | Authorized fixture manifest, human-reviewed labels, matched workload comparisons, sample sizes/variability, and no critical safety or continuity regressions. |

Finish and review the current patch before broadening production changes. Keep
provider, privacy, recovery, typed acknowledgments, queue/artifact bounds, and
baseline-fixture corrections as separate logical review slices. Do not rewrite
published history or commit/push merely to organize these slices.

### Contract gate for M2

Before implementing heartbeat/schema or shutdown changes, specify:

1. Which storage owner excludes independent facades/processes, and what happens
   when ownership cannot be acquired. A cloned-handle mutex is not enough.
2. Durable operation status versus observed process liveness; absence of evidence
   means unknown, never successful completion. Define process-instance identity,
   clock/suspend behavior, and legacy presentation without historical mutation.
3. Caller-visible cancellation acknowledgment versus completed durable shutdown.
   Allocate one deadline among essential writes and optional drains. Choose and
   justify numeric budgets from supported-mode requirements and measurements.
4. A bounded diagnostic route for failed terminal writes that does not depend on
   the failed store. No abandoned task may later mutate an unowned session.
5. Fault matrix: normal exit, EOF/disconnect, cancellation before/after append,
   disk-full/permission/torn-tail/corruption, blocked optional service, full queue,
   crash during repair, and reopen twice. Assert state and side effects, not only
   event counts. Run process/path cases on Linux and Windows before acceptance.

### Measurable acceptance additions

- **Privacy:** marker-free live snapshot, HTTP, SSE, durable reopen, legacy read,
  mirror, and export fixtures; unchanged authoritative input; idempotence and
  ordinary diagnostics preserved. Report untested routes explicitly.
- **Recovery:** each injected before/after write boundary has a named fixture;
  paired tool results, preserved assistant text, no duplicate unsafe execution,
  and surfaced unresolved writes after repeated reopen.
- **Pressure/output:** report admitted/lost/coalesced events and retained bytes;
  under-capacity traffic has zero loss. Test concurrent producers, slow consumers,
  saturation, drain/recovery, and closure. Current 512-event/8-MiB queue and
  100-MiB/10,000-file spill caps are implementation limits, not measured heap or
  per-turn token guarantees. Measure overhead before accepting performance claims.
- **Memory:** distinguish rejection, append, provider delivery, remote acceptance,
  and completion; unknown fields stay unknown. Test timeout-after-accept and
  retry/restart suppression without assuming backend idempotency.
- **Evaluation:** create synthetic fixtures for each named failure class; record
  retained requirements, false completion, task success, retries and foreground
  latency separately. Declare matched workloads and sample sizes in advance.
  Establish timing thresholds from repeatable baseline runs before optimization;
  do not retrofit thresholds to favorable results or call faux replay a live
  task-quality benchmark.

### Dependency and blocker register

| ID | Owner / limitation | Unblocking evidence | Safe work available now |
| --- | --- | --- | --- |
| B1 | Repository tests: seven baseline assertions | Root cause and focused/full green reruns; no weakened assertions or disabled safety policy | Correct stale fixtures only where demonstrated, otherwise repair owning code |
| B2 | Session storage owner: process/lease and facade recovery | Shared-facade repair, JSONL process exclusion/kill after progress and at ten recovery append boundaries, and SQLite expired-lease recovery fault/reopen matrix verified locally; still require mid-write termination, physical I/O-failure, power-loss, SQLite process-crash, and Windows evidence | Preserve backend locks; extend crash and physical I/O fault matrix |
| B3 | Cognee backend: timeout-after-accept/idempotency | Documented stable job/idempotency contract plus fake acceptance/timeout/restart tests | Surface unknown mutation outcomes; bound attempts and avoid blind replay claims |
| B4 | Outer host/MCP: permission and output transformation | Identify actual authority and supported approval/retrieval API; reproduce using authorized synthetic files | Improve Rust-owned diagnostics/capability contracts; never bypass a denial |
| B5 | Historical audit sample unavailable through authorized access | Owner-authorized accessible sample, or explicit acceptance of fresh-only coverage | Mark historical semantic coverage unavailable (not 48/48); use fresh synthetic ledgers |
| B6 | Human semantic evaluation and supported platforms | Human-reviewed labels and Windows test evidence | Prepare deterministic rubric/fixtures and local Linux evidence; do not invent sign-off |

B2 is partially verified local engineering work, not an external excuse. JSONL
already owns a stable OS-locked sidecar and SQLite has a writer lease; do not
replace them with a facade mutex. Shared-storage facades now coordinate recovery
by object identity. SQLite takeover recovery is exercised at five write boundaries,
before/after each append, with stale-owner rejection and two disk reopens. JSONL
recovery also survives killing its owning child after progress is acknowledged
and before/after each of five recovery appends; this does not interrupt an OS
write and is not a power-loss test. This is not a transaction against arbitrary concurrent writes
or wrappers, and storage must not be rebound mid-repair. B3–B6 limit specific
acceptance claims, not all repository work. No live credentials, network service,
private-history access, or platform result may be assumed to close them. A local
milestone may be verified while program acceptance remains pending; only the
owner may accept a residual external dependency.

## Delivery order and checkpoints

| Phase | Work items | Checkpoint |
| --- | --- | --- |
| 0. Establish a trustworthy baseline | H01 | Reproducible fixtures and selected-test accounting exist |
| 1. Fix demonstrated defects | H02–H06 | Four reproduced gaps closed; historical provider symptoms classified |
| 2. Make interruption and shutdown safe | H07–H09 | Repeated crashes preserve recovery; liveness and shutdown are truthful |
| 3. Make memory bounded and explainable | H10–H11 | Failures have meaningful outcomes; optional work respects budgets |
| 4. Make tool and telemetry contracts consistent | H12–H16 | Permissions, capabilities, payloads, and queues have explicit contracts |
| 5. Validate task continuity and productivity | H17–H18 | Semantic and task-level evaluation supports the changes |

These are checkpoints, not a requirement to finish one entire phase before
starting another. H02, H03, H04, H05, and H07 can proceed independently after their
local fixtures exist. Start H12/H13 ownership investigation and H17 evaluation
fixtures early. Do not delay a small security or correctness fix for a complete
observability redesign. Do not parallelize patches to the same core file without
agreeing the integration order.

### First implementation batch

1. **H02:** test and align redaction at actual live/durable telemetry boundaries.
2. **H03:** fix completed-memory projection and pin phase semantics.
3. **H04:** stop permanent billing failures from entering retry loops.
4. **H05:** make provider headroom account for tool arguments and schemas.
5. **H07 investigation:** inject crashes/write failures during recovery before
   choosing a persistence change.

H01 is a lightweight baseline task carried alongside these, not a new project
that blocks them. The smallest first patch is H03; the highest-priority security
workstream is H02. These should be separate changes.

## Phase 0 — evidence and verification

### H01. Preserve reproducible cases and a valid baseline

**Owner:** existing crate test suites; audit/evaluation documentation.
**Evidence:** audit §11.1 and §11.9. **Dependencies:** none.

- Turn the four synthetic public-API probes into normal crate regression tests,
  colocated with each fix; retain sanitized error shapes, not captured messages.
- Record selected test names/counts, features, build revision, command, exit code,
  warnings, ignored tests, and isolated environment. Flag zero selected tests as
  an invalid verification result, not a pass.
- Establish a small offline replay fixture manifest with expected invariants and
  evidence provenance. Keep logical events distinct from updates/retries.
- Use the audit's 159 passing tests and one ignored test only as a historical
  baseline; rerun relevant suites against each actual implementation revision.

**Done when:** each confirmed defect has a failing-before/passing-after test;
fixture runs are reproducible without private data or a live service; benchmark
and test reports cannot silently count an empty selection as verification.

## Phase 1 — demonstrated defects and provider contracts

### H02. Apply equivalent redaction to every telemetry surface

**Owner:** `ice-cli` observability recorder, store, server, mirrors, and exports.
**Evidence:** §11.6; synthetic live/store discrepancy. **Dependencies:** none.

- Trace real event construction through live snapshots, SSE, persistence,
  mirroring, and exports. Do not stop at testing the generic SSE serializer.
- Introduce the smallest common sanitization boundary before telemetry fan-out;
  retain defense-in-depth at persistence/export boundaries. Sanitization must
  not mutate provider inputs, tool execution arguments, or authoritative session
  history: telemetry copies are the scope of this change.
- Cover structured and stringified keys, nested values, prefixed environment
  assignments, authorization/cookie headers, URL credentials, and mixed prose.
- Verify that redaction is idempotent, bounded in cost, and preserves useful
  nonsecret diagnostics. Keep loopback/token and restrictive-file controls.
- Define how old telemetry is handled: sanitize on reading/exporting where
  feasible; inventory stored exposure privately. Do not automatically rewrite
  or delete historical databases. Any cleanup needs a backup and owner approval.

**Done when:** no synthetic marker escapes any tested telemetry route, before or
following restart; ordinary diagnostics survive; legacy reads do not bypass the
policy. No real credentials are needed to demonstrate success.

### H03. Correct memory lifecycle projection

**Owner:** `ice-cli` Cognee observation producers and observe recorder.
**Evidence:** §11.2; `completed` maps to `Running`. **Dependencies:** none.

- Inventory emitted phases and define one explicit mapping for started,
  succeeded/completed, failed, cancelled, and unrecognized observations.
- Fix `warmup_replay` immediately. Share a parser/mapping within the owning code
  rather than redesigning the entire event system for one bug.
- Preserve original phase values for diagnostics. Unknown phases must not
  silently assert active execution; settle their UI representation explicitly.
- Test update ordering, repeated terminal events, missing starts, and live/history
  parity. Avoid reopening an already-terminal activity on an older update.
- Decide how old misprojected rows are shown from available raw phase evidence;
  otherwise disclose the limitation. Do not bulk-rewrite raw audit history.

**Done when:** completed replay is terminal in live and persisted views, unknown
phases are distinguishable, and terminal events remain stable under replay.

### H04. Classify billing errors before transient retry rules

**Owner:** `ice-ai/src/retry.rs` and provider error adapters where necessary.
**Evidence:** §11.5; HTTP 429/code 1113 wrongly retries. **Dependencies:** none.

- Prefer structured, provider-scoped codes/types over generic status matching;
  retain conservative text fallbacks when structured data is absent.
- Add the exact sanitized insufficient-balance fixture and wording variants.
  Keep genuine rate limiting, retry-after, transient failures, cancellation, and
  exhausted retry budgets covered.
- Return an actionable billing error without automatic repeated attempts. Do
  not interpret a coincidental numeric code from another provider as billing.

**Done when:** permanent billing fixtures trigger zero automatic retries;
genuine transient fixtures retain bounded retry behavior; existing unsafe-tool
retry restrictions remain intact.

### H05. Make provider budgeting account for the complete request

**Owner:** `ice-ai` simple options/provider serializers and `ice-harness` token
estimation. **Evidence:** §11.5; a 200,000-character tool argument estimates zero.
**Dependencies:** none; coordinate with H06 and later H14.

- First cover tool-call arguments in the provider estimator. Then include tool
  schemas, system text, image allowances, and provider framing in headroom
  decisions at the point the request shape is known.
- Compare the existing provider and harness estimators. Share a small pure helper
  or an explicit estimate contract if needed; do not introduce a dependency cycle
  or replace the stronger harness estimator with the weaker one.
- Preserve hard output limits and the existing safety margin. Reject or compact
  already-overfull input explicitly; do not pretend one output token makes an
  impossible request fit.
- Add fixtures for large arguments, Unicode, schemas, images, thinking content,
  very small context windows, and model overrides. Test final serialized requests,
  not just an intermediate options object.

**Done when:** tool arguments cannot contribute zero merely because they are tool
calls; adding input cannot increase output allowance for a fixed model/options;
final request fixtures respect both output and context budgets. Document that
estimates are conservative approximations, not exact provider tokenization.

### H06. Separate historical provider symptoms from remaining defects

**Owner:** `ice-ai` provider adapters; CLI model metadata/configuration boundary.
**Evidence:** §4 and §11.5. **Dependencies:** H04/H05 fixtures where relevant.

- Build sanitized wire fixtures for content type/headers, image/content-block
  compatibility, thinking-disabled models, output limits, and context overflow.
- Attribute old incidents to adapter/build versions only where evidence permits;
  otherwise label them historical/unattributed. Existing Codex JSON-header
  protection must not be reimplemented as if absent.
- Test normalization/validation after model metadata and overrides are applied.
  Keep transport MIME errors distinct from rejected JSON content blocks.
- Patch only reproduced current gaps; close already-covered symptoms with a
  regression test and explanation rather than an unrelated adapter rewrite.

**Done when:** each historical error class has a current fixture or an explicit
reproduction blocker, and invalid current requests produce useful local errors
where the provider contract makes validation possible.

## Phase 2 — recovery, liveness, and shutdown

### H07. Fault-inject recovery, then repair the proven failure window

**Owner:** `ice-harness` recovery/session persistence and integration tests.
**Evidence:** §11.3; terminal record precedes salvage. **Dependencies:** none.

- Inject failure before/after the terminal append, during frame/result salvage,
  and after each salvaged message append. Reopen twice and test concurrent openers
  under the existing session ownership/locking rules.
- Assert tool-call/result pairing, recoverable assistant text, deterministic
  repeated recovery, and no automatic replay of unsafe external side effects.
- If the suspected omission reproduces, implement resumable/idempotent recovery
  or atomic publication using existing storage primitives. Merely moving one
  append is not sufficient if partial salvage can duplicate on retry.
- If existing safeguards prevent the scenario, document why and retain the test;
  do not change session format solely to satisfy a hypothesis.
- Treat disk-full, permission failure, torn tail, and corrupted records separately.
  Recovery-write errors must not silently advertise successful repair.

**Done when:** every injected boundary reaches a consistent recoverable outcome,
never duplicates unsafe actions, and exposes unresolved outcomes honestly. Any
format change includes old-format fixtures, migration/crash behavior, and an
explicit downgrade policy reviewed before implementation.

### H08. Model liveness independently of historical run status

**Owner:** `ice-cli` observe runtime/store and run/process startup.
**Evidence:** §11.2; 227 recorded running runs are not 227 live processes.
**Dependencies:** H03 for phase consistency, not for a new schema dependency.

- Add minimal build/schema provenance and process-instance identity to new runs.
  Avoid full command lines, environment dumps, or unnecessary machine identity.
- Define heartbeat/lease ownership, start/closing/finished/interrupted/unknown
  semantics, and session-to-run relationships. PID alone is insufficient because
  of reuse; wall-clock movement and suspend/resume also need tests.
- Reconcile only within a defensible ownership boundary. Missing heartbeat or
  another process's inaccessible state must not become a false successful finish.
- Preserve old rows; use explicit legacy/unknown presentation where liveness is
  unknowable. Capture terminal-write failures via a bounded independent diagnostic
  path, without recursively depending on the failed telemetry store.

**Done when:** normal exit, crash, restart, clock changes, and concurrent processes
produce truthful states; old data remains readable; terminal persistence failure
is visible. No blanket conversion of historical `running` rows to `finished`.

### H09. Bound shutdown without sacrificing durable recovery

**Owner:** CLI ACP/RPC modes, observe runtime, Cognee runtime; harness cancellation.
**Evidence:** §11.2; three-second ACP grace versus potentially longer cleanup.
**Dependencies:** H07 invariants and H08 lifecycle contract.

- Define one shutdown deadline contract and allocate it across cancellation,
  essential durable bookkeeping, observer drain, and optional memory cleanup.
- Separate caller-visible stop acknowledgment from fully completed shutdown.
  Do not mark work finished while durable operation state remains unresolved.
- Bound optional work and make cancellation cooperative; fallback termination
  must leave restart recovery possible. Increasing ACP's grace alone is not a fix.
- Exercise normal stop, EOF, disconnect, cancellation, slow/hung memory, slow disk,
  failed terminal writes, and forced kill independently on supported platforms.
- Preserve useful background work only through an explicit resumable mechanism;
  do not silently detach tasks that can later mutate an abandoned session.

**Done when:** cooperative shutdown respects its declared deadline, optional
cleanup cannot block it indefinitely, and forced termination yields a truthful
interrupted/unknown state with safe recovery. Historical attribution remains
separate from the synthetic scenario's success.

## Phase 3 — memory usefulness and bounded cost

### H10. Give memory outcomes and latency meaningful semantics

**Owner:** Cognee observer/request/runtime and harness extension acknowledgments.
**Evidence:** §11.4; ambiguous version-1 acknowledgments and sparse wait spans.
**Dependencies:** H02 sanitization; H08's minimal provenance where available.

- Distinguish logical job, transport attempt, server acceptance, local persistence,
  model delivery, cancellation, and final useful completion. Correlate with agent
  run/operation and request without logging memory content by default.
- Replace overloaded `append_failed` reporting with explicit budget/provenance
  rejection, stale operation, cancellation, append I/O failure, persisted, and
  delivered outcomes. Preserve version-1 decoding; do not relabel old evidence as
  if it had version-2 detail.
- Measure foreground wait separately from remote duration and background work.
  Join spans by identity; never sum overlapping durations into wall-clock delay.
- Keep request index, selected token/byte budget, circuit state, and suppression
  reason in bounded sanitized metadata.

**Done when:** fake scenarios each yield the correct outcome and causal span;
persistence is never mislabeled delivery; old logs still parse; unavailable
measurements are explicit rather than inferred as zero.

### H11. Bound memory work and avoid duplicate remote improvement

**Owner:** Cognee request/runtime/client and offline memory tests.
**Evidence:** §11.4; 148 deadline-shaped failures, existing suppression controls.
**Dependencies:** H10 to measure behavior; H09 for shutdown integration.

- Test a fake service that accepts work, exceeds the local timeout, then succeeds;
  also cover rejection, unreachable service, and cancellation before/after accept.
- Reuse cooldown, revision, in-flight, cache, and circuit protections. Verify their
  scope across attempts/restarts before changing thresholds.
- If the backend supports stable job IDs/idempotency keys, use them. Otherwise
  treat timed-out mutation outcomes as unknown and avoid claiming safe blind
  replay. Backend cooperation is an explicit external dependency, not an assumed
  capability to implement only on the client.
- Preserve network-free ordinary request paths and budget explicit-history recall.
  Test that stale/prefetched evidence is consumed only by the correct task after
  acknowledgment, and that disabled/degraded memory leaves the core agent usable.
- Measure task benefit alongside latency before raising budgets or disabling
  memory. Keep automatic recall, explicit recall, and improvement populations
  separate.

**Done when:** timeout-after-accept cannot silently cause unbounded duplicate work;
foreground and shutdown budgets hold under degradation; existing cancellation,
provenance, budget-rejection, and circuit tests still pass. Residual unknown
backend outcomes are visible and recoverable within documented constraints.

## Phase 4 — tool and telemetry contracts

### H12. Make permission failures understandable and recoverable

**Owner:** actual permission authority in tool execution, MCP/host adapter, and
`ice-tools` where applicable. **Evidence:** §3 and §11.8. **Dependencies:** none.

- Reproduce representative file-root denials with synthetic files. Identify which
  component enforces each boundary before attributing a fix to Rust core.
- Expose structured failure category, applicable scope, and the allowed recovery
  route. Preflight must use the same policy as execution, including path
  normalization, symlinks, platform path rules, and changes between check/use.
- Retain tool-specific grants when intentional; consistency means predictable
  policy and messages, not identical or broader privileges for every tool.
- Support narrowly scoped approval only where the authority permits it. Do not
  probe denied content or use another tool to evade a decision.

**Done when:** denial/retry/approval journeys are deterministic and explainable;
authorization is enforced again at execution; an agent can recover through an
explicitly authorized action without repeated doomed calls or wider access.
External-host work is tracked separately if this repository cannot implement it.

### H13. Generate instructions from actual capabilities

**Owner:** harness prompt/skill assembly and CLI/MCP/outer-host capability adapters.
**Evidence:** §8 and §11.8; absent todo, truncated skills, script echo.
**Dependencies:** none; coordinate errors with H12 and size contracts with H14.

- Make mandatory tool instructions conditional on advertised, enabled capability.
  Distinguish configured, available, authenticated, and permission-restricted.
- Provide an explicit fallback such as a durable checklist when todo is absent;
  do not install unowned companion plugins to satisfy an instruction.
- Ensure skill truncation includes an offset/continuation or recoverable artifact.
  Bound capability/skill injection and support on-demand discovery without hiding
  required safety instructions.
- Attribute script echo and response-transformer truncation to their actual
  producer; patch there if available, otherwise provide an integration contract
  and a fixture rather than claiming a Rust-core fix.

**Done when:** tests with missing/disabled/unavailable tools do not produce
impossible mandatory actions; incomplete reads are detectable and continuable;
critical instructions are not silently lost to truncation.

### H14. Enforce recoverable output budgets across a turn

**Owner:** tool-result ingestion in the harness plus CLI/MCP/native-tool adapters.
**Evidence:** §5 and §11.7–§11.8. **Dependencies:** H02; H05 contract coordination.

- Budget individual results and aggregate per-turn/model-request text, including
  parallel tool completions, nested content, metadata, and wrapper/script echo.
- Separate UTF-8/storage bytes, estimated text tokens, and image/media payloads.
  Never classify image-bearing reads as million-character text leaks.
- Preserve exit status, warnings/errors, provenance, truncation reason, original
  size, and retrieval instructions. Store full output in access-controlled
  artifacts only when necessary, with a defined retention policy and bounded disk
  use. Never return an artifact that the intended caller cannot access.
- Test adversarially large/Unicode outputs and multiple individually-small results
  that exceed the aggregate budget. Retrieval itself must respect budgets.

**Done when:** configured budgets hold under concurrent results; required status
and diagnostic summaries survive; full evidence is recoverable through authorized
paging/search rather than hidden or repeatedly dumped into model context.

### H15. Bound observer queues and retained bytes

**Owner:** `ice-cli` observe worker, recorder, store, and SSE consumers.
**Evidence:** §11.7; unbounded command queue, count/per-string caps only.
**Dependencies:** H02; H08/H09 terminal-event requirements.

- Measure queue count/bytes, serialization copies, store latency, listener lag,
  and retained payload bytes. Choose limits from a synthetic stress baseline.
- Bound the worker queue and retained bytes; coalesce replaceable progress updates.
  Prioritize terminal/control events without blocking the agent indefinitely.
- Define overload behavior explicitly. A finite queue cannot promise lossless
  delivery during unlimited overload: use bounded durable handling where viable
  and an explicit gap/unknown signal when critical telemetry cannot be stored.
- Stress slow disks/listeners, many sessions, large payloads, and shutdown with a
  full queue. Count drops/coalescing, and test recovery after pressure subsides.

**Done when:** memory/queue growth stays within declared limits plus measured
bounded overhead; model/tool progress is not held hostage by optional telemetry;
lost terminal evidence never appears as a fabricated successful outcome.

### H16. Provide a stable, private analysis export

**Owner:** observe store/query/export contract; Cognee/session adapters as needed.
**Evidence:** §6 and §11.7. **Dependencies:** H02, H03, H08, H10.

- Offer a versioned normalized event view with IDs, timestamp units, update/attempt
  semantics, lifecycle phase, provenance, and explicit unknown/missing fields.
- Decode nested/stringified JSON safely; keep raw source representation available
  only through appropriate access controls. Export sanitized fields by default.
- Deduplicate updates by documented identity. Keep run, operation, memory job,
  transport attempt, tool call, and session populations distinct.
- Support read-consistent snapshots and record capture boundaries; joining
  independently captured files must not imply a cross-store atomic snapshot.

**Done when:** offline queries reproduce fixture counts and timelines without UI
string heuristics; malformed/legacy entries do not silently disappear; exports
pass the same privacy tests as live telemetry.

## Phase 5 — continuity and task outcomes

### H17. Evaluate compaction semantics before changing the policy

**Owner:** `ice-harness` compaction plus private evaluation workflow.
**Evidence:** §11.8; pairing verified, semantic fidelity not comprehensively scored.
**Dependencies:** can start early; use H14 output contract for size-related cases.

- Privately score all accessible 48 audited compactions if snapshots remain
  available. Otherwise record unavailable cases and build a fresh, authorized
  sample; do not present partial coverage as 48/48.
- For each, establish a pre-compaction ledger of explicit requirements, safety
  constraints, decisions, paths, test results, and unfinished work. Compare the
  summary plus retained tail and subsequent actions, not the summary alone.
- Separate factual omissions, false completion, stale state, and legitimate
  rereads. Exact repeated-call counts are not a semantic metric.
- Turn failure classes into synthetic deterministic fixtures. Change summary
  structure, tail selection, or durable task state only for observed failure
  classes; retain existing tool-pairing and split-turn guarantees.
- Use human review for semantic labels; an LLM judge alone is not ground truth.

**Done when:** coverage and rubric are explicit, every found critical omission
has a fixture, and proposed policy changes preserve all critical requirements in
that fixture suite without breaking pairing or context budgets. This is not a
claim of perfect summaries on arbitrary future conversations.

### H18. Measure useful progress, then remove demonstrated waste

**Owner:** offline evaluation runner and the specific agent/tool/host component
identified by each experiment. **Evidence:** §7, §10, §11.8. **Dependencies:** H01;
use H10/H16 for causality and H17 for continuity scoring.

- Define representative task families: edit/test/debug, long multi-step work,
  recovery/resume, denied-tool recovery, large outputs, and memory-assisted tasks.
- Use deterministic faux replays for correctness and identical controlled
  workloads for performance. Any live-provider evaluation requires separate
  approval and must not become a CI dependency.
- Report task success, user corrections, retained requirements, foreground memory
  wait, provider/tool time, time to first useful progress, retries, empty polls,
  input/output volume, and total wall-clock latency. Deduplicate copied histories
  and account for concurrent waits instead of summing them as wasted time.
- Compare memory on/off or policies only on the same authorized workload; report
  sample size and variability. Avoid improving a latency metric by lowering task
  quality or hiding failures.
- Where repeated empty polling is demonstrated, prefer existing completion
  notifications/watch mechanisms or bounded waits. Preserve cancellation and
  interactive feedback; do not remove all polling indiscriminately.
- Establish baseline-derived performance thresholds before accepting optimization
  patches. Commit stable correctness checks to CI; keep noisy timing diagnostics
  in a separate benchmark job until repeatability is demonstrated.

**Done when:** accepted changes have task-level evidence, no critical safety or
continuity regressions, and documented trade-offs. Remaining failures are linked
to an owner and a reproduction rather than explained by aggregate latency alone.

## Dependencies and integration boundaries

```text
H01 fixtures ─────────────── accompanies every change
H02 redaction ───────────── H10 / H14 / H15 / H16
H03 phase semantics ─────── H08 / H16
H04 retry + H05 budgets ─── H06 provider contracts
H07 recovery invariants ─── H09 shutdown
H08 liveness ────────────── H09 / H10 provenance / H15 / H16
H10 memory outcomes ─────── H11 memory behavior / H16 / H18
H12 permission policy ───── coordinate H13 capability guidance / H14 retrieval
H14 output budgets ─────── H17 size-sensitive fixtures
H17 continuity + H16 data ─ H18 integrated evaluation
```

Dependencies are contracts, not instructions to combine work into one large PR.
H11 may require backend changes; H12/H13 may require host/MCP/transformer changes.
Record those external blockers explicitly. Repository changes cannot guarantee
behavior in a component they do not control.

## Validation, rollout, and rollback

### Per-change gate

1. Reproduce the case on the current baseline and record actual selected tests.
2. Make the smallest owning-layer change and run focused offline tests in a
   temporary `ICERS_CODING_AGENT_DIR`; preserve full logs, exit codes, and warnings.
3. Review compatibility: public API, plugin ABI, session JSONL, observability
   SQLite, settings, and event schemas. Call out all affected formats explicitly.
4. Run repository CI gates before calling an implementation PR ready:

   ```sh
   cargo fmt --all -- --check
   RUSTFLAGS="-D warnings" cargo check --workspace --all-targets --locked
   RUSTFLAGS="-D warnings" RUSTDOCFLAGS="-D warnings" cargo test --workspace --locked
   sh scripts/sync-embedded-docs.sh && git diff --exit-code -- crates/ice-cli/embedded-docs
   python3 scripts/test_prebuilt_release.py
   ```

   Tests must not require a live provider; use cached dependencies/offline mode
   during isolated loops. Test MSRV-sensitive changes with Rust 1.88, and verify
   path/process behavior on Linux and Windows. Fix feature/test selection before
   trusting any filtered suite.
5. Update source docs and generated copies only where applicable. This planning
   document itself is not an embedded-doc source. Update the porting map if a
   work item closes or changes a recorded gap.

### Staged rollout

- **Offline:** deterministic fixtures, fault injection, stress tests, and
  backward-compatibility tests first.
- **Isolated local run:** temporary profiles and synthetic sessions; no live
  credential or history migration.
- **Owner-approved pilot:** bounded real usage with sanitized metrics and known
  build IDs. Define the workload, observation window, and acceptable latency/
  quality thresholds before measuring; do not choose thresholds after results.
- **Broader use:** only after safety invariants and agreed pilot thresholds pass.
  Releasing or publishing still requires separate explicit approval.

Simple correctness/privacy fixes should not create permanent configuration
switches. New optional policies may need a narrowly scoped fallback. Never roll
back by disabling redaction, bypassing authorization, or replaying unsafe tools.
If a security fix causes trouble, fail closed or disable the optional telemetry
surface rather than restoring a known unsafe path.

For on-disk changes, prefer additive/backward-readable formats when supported.
Test old/new readers rather than assuming additive fields are always safe.
Back up before an approved migration; prevent unsupported downgrade, provide a
conversion path, or restore a compatible backup. A code revert alone is not a
rollback plan for a changed data format.

## Completion scoreboard

All implementation items are initially unchecked. A work item is complete only
when its acceptance criteria, relevant tests, and documentation are recorded;
writing this plan or running existing tests does not complete implementation.

- [ ] H01 — Reproducible baseline and valid test accounting
- [ ] H02 — Common telemetry redaction policy across all surfaces
- [ ] H03 — Correct memory phase projection
- [ ] H04 — Billing-aware bounded retry behavior
- [ ] H05 — Full-request provider headroom estimation
- [ ] H06 — Provider wire-contract fixtures and incident classification
- [ ] H07 — Crash-during-recovery invariants and any required repair
- [ ] H08 — Honest run liveness and build/process provenance
- [ ] H09 — Bounded, recoverable shutdown
- [ ] H10 — Typed/versioned memory outcomes and causal latency
- [ ] H11 — Bounded memory work and explicit unknown remote outcomes
- [ ] H12 — Permission preflight, scoped recovery, and enforcement consistency
- [ ] H13 — Capability-aware instructions and detectable skill truncation
- [ ] H14 — Aggregate text/media output budgets with recoverable artifacts
- [ ] H15 — Bounded observer queues/bytes and explicit overload behavior
- [ ] H16 — Versioned sanitized analysis export
- [ ] H17 — Semantic compaction evaluation and targeted improvements
- [ ] H18 — Task-quality/performance evaluation and proven polling improvements

### Audit coverage map

| Audit finding/workstream | Plan coverage |
| --- | --- |
| Completed memory shown running | H03 |
| Stale runs, process ambiguity, ACP cleanup mismatch | H08, H09 |
| Missing tool results and second-crash recovery risk | H07 |
| Improve timeouts, ambiguous injection outcomes, critical-path delay | H10, H11 |
| Billing retry, token-estimator gap, historical wire failures | H04–H06 |
| Live/store redaction and synthetic key/cookie coverage | H02 |
| Per-result/aggregate output, image accounting, truncated evidence | H14 |
| Unbounded observer queue, retention bytes, slow consumers | H15 |
| UI-oriented schema, updates, missing build attribution | H08, H16 |
| Permission denials and tool-switching journeys | H12 |
| Missing todo, capability instructions, skill truncation, script echo | H13, H14 |
| Compaction pairing versus semantic continuity | H17 |
| Raw latency versus useful progress, repeated polling, task outcomes | H18 |
| Snapshot limits, copied histories, test-selection mistakes | H01, H16, H18 |

## Definition of program completion

The program is complete when reproduced defects have regression-tested fixes,
credible safety risks have been reproduced and repaired or ruled out with tests,
legacy data remains readable under documented compatibility rules, external
ownership gaps are resolved or explicitly accepted by the owner, and controlled
task evaluation shows no critical safety/continuity regression.

“Every old running row is finished,” “all memory calls succeed,” and “all tests
were green once” are not success criteria. The goal is honest state, bounded
failure, recoverable work, and demonstrably better task outcomes.
