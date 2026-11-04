# Harness audit: session history, observability, and agent experience

Date: 2026-11-03

## Executive summary

Prioritize reliability, lifecycle correctness, and consistent tool contracts
before expanding the feature surface. The harness already has context budgeting,
memory timeouts, circuit breakers, background tools, and substantial telemetry.
The opportunity is to make those protections consistent and measurable.

The initial audit found a memory-status mapping defect, substantial stale run
state, recurring memory failures, permission-boundary failures, and historical
provider request-validation failures. The deeper pass reproduced the lifecycle
mapping bug, an insufficient-balance retry-classification gap, a zero-token
estimate for large tool arguments in the provider-budget estimator, and a
redaction difference between live SSE serialization and persistence. It also
identified a recovery write-ordering risk that still needs fault injection.

**Read §11 for the deeper investigation, current-code evidence, verification,
revised priorities, and unresolved questions.** The earlier sections preserve the
initial measurements; their populations and timestamps are not interchangeable
with the later frozen copies.

No production configuration, credentials, or session data was modified. The
initial pass was observational; the deeper pass added offline tests and synthetic
probes. Not every historical symptom was reproduced. Recommendations are
proposals, not implemented fixes, and no findings have been published.

Follow-up: [Harness reliability and quality improvement plan](harness-improvement-plan-2026-11-03.md)
turns these findings into phased work items, dependencies, acceptance tests, and
rollout safeguards. The plan is proposed; implementation has not started.

## Scope and method

Sources examined:

- `~/.icers/agent/observability.sqlite`, opened read-only.
- Session JSONL under `~/.icers/sessions/` and available directories referenced by
  `~/.icers/agent/session-directories/`.
- Relevant current Rust source, especially observability recording, provider
  token budgeting, and Cognee runtime protections.
- Direct experience using the available tools during the audit.

Coverage at the respective snapshots:

| Source | Coverage |
| --- | --- |
| Available session roots | 19 |
| Unavailable registered directories | 3 |
| Session files | 347 |
| Session file size | 111,014,933 bytes, approximately 111 MB |
| Files containing messages | 163 |
| User messages | 632 |
| Assistant messages | 3,458 |
| Tool results | 4,574 |
| Observability runs, initial snapshot | 240 |
| Observability events, initial snapshot | 12,358 |
| Invalid JSONL lines encountered | 0 |
| Session creation-time range | 2026-10-19 through 2026-11-03 UTC |

The first inventory covered only 200 default-directory files. Discovery of the
session-directory registry expanded the analysis to the totals above. Conclusions
in this report use the expanded session inventory unless explicitly identified as
observability-only.

Method:

1. Inventory storage without opening credentials or configuration secrets.
2. Aggregate every available session file structurally.
3. Inspect database schema and event shapes.
4. Reduce observability activity updates to their latest state using
   `(run_id, activity_id)` rather than counting every update as a separate action.
5. Classify recurrent errors and inspect selected implementation locations.
6. Distinguish existing protections from missing or inconsistent behavior.

Limitations:

- The initial database and logs were live, so successive queries have slightly
  different counts. The deeper pass (§11) used a SQLite backup and frozen file
  copies, but capture was not atomic across the database and all logs.
- The observability database covers a different population from the session logs.
  Their totals must not be combined into a single failure rate.
- Session files were deduplicated by file path, not by logical message identity
  across copied histories. Message and token totals are recorded-history totals,
  not guaranteed unique production usage or billing totals.
- Logs were processed structurally; every conversation was not manually read.
- Three registered directories were unavailable. Unregistered histories elsewhere
  on disk were not exhaustively searched.
- Historical records were not attributed to exact binary/build versions.
- The audit did not inspect the TypeScript installation or credential files.
- Raw private conversations are intentionally not reproduced here.

## 1. Lifecycle telemetry is not trustworthy enough

### 1.1 Confirmed defect: completed memory replay appears running

**Priority: P0. Confidence: source-confirmed.**

The reduced observability data contained 123 `warmup_replay` activities with
`phase = completed` but display state `running`.

At audit time, `crates/ice-cli/src/observe/recorder.rs:978` recognized only
`succeeded` and `failed` as terminal phases:

```rust
let terminal = matches!(observation.phase.as_str(), "succeeded" | "failed");
let state = match observation.phase.as_str() {
    "failed" => ObserveItemState::Error,
    "succeeded" => ObserveItemState::Done,
    _ => ObserveItemState::Running,
};
```

Consequently, `completed` becomes `Running`. This is a reporting defect, not proof
that those 123 operations remained active.

Recommended changes:

- Normalize lifecycle phases at the producer/consumer boundary.
- Prefer a typed shared contract over independent string interpretation.
- Treat completed warmup replay as terminal success.
- Define how cancelled, skipped, timed-out, and unknown phases are represented.
- Add a regression test for the observed completed-phase payload.

Acceptance criteria: a completed warmup replay has a terminal state, and updates
to the same request cannot leave a phantom running activity.

### 1.2 Persisted run status does not establish liveness

**Priority: P0. Confidence: strong symptom; root cause not isolated.**

At the initial database snapshot, 227 of 240 runs were marked `running`. Of these,
179 started more than 24 hours before the newest run's start time.

This does not establish that 227 processes were stuck. Long-lived sessions,
unclean exits, missing lifecycle calls, and persistence failures need to be
distinguished.

`crates/ice-cli/src/observe/recorder.rs:1052` marks a run finished in `stop()`.
Failures from `finish_run` are logged at debug level, and the result of closing
persistence is discarded at that location.

Recommended changes:

- Record process-instance identity and heartbeat timestamps.
- Separate execution completion from process/session lifetime.
- Introduce explicit interrupted/unknown status rather than assuming stale means
  finished or failed.
- Reconcile orphaned runs without modifying another active process's runs.
- Surface recorder persistence failures as actionable diagnostics.
- Verify shutdown wiring for ACP disconnect, EOF, cancellation, normal exit,
  abrupt termination, and restart.

Acceptance criteria: crash/restart tests produce honest terminal or interrupted
state, and a concurrently live process is never declared dead merely because
another process restarted.

## 2. Memory failures need isolation and usefulness measurements

**Priority: P1. Confidence: recurring failures confirmed; critical-path impact
not established for all operations.**

Latest-state memory activity counts were:

- 1,460 done.
- 243 error.
- 161 running, including the 123 completed-phase mapping errors above.

Selected operation counts:

| Operation | Successful | Failed | Other observed state |
| --- | ---: | ---: | --- |
| improve | 34 | 118 | 36 started/running |
| injection | 8 | 100 | None in the selected grouping |
| recall | 264 | 16 | 2 started/running |
| trace | 765 | 9 | None in the selected grouping |
| remember_entry | 267 | 0 | None in the selected grouping |
| session | 110 | 0 | None in the selected grouping |
| warmup_replay | 123 completed but displayed running | 0 | Mapping defect |

Error-field aggregation found 141 timeout diagnostics. These are diagnostic
occurrences, not an independently validated count of unique timeout requests.
Some metadata also contained `dream`; that is not sufficient to classify a root
cause.

Two explicit `cognee_recall` tool calls took approximately 40 and 89 seconds.
This sample is too small to infer a general latency distribution.

Current implementation already includes timeouts and recall/remember circuit
breakers, including state in `crates/ice-cli/src/cognee/runtime.rs`. The solution
should extend or validate existing protections, not create duplicate mechanisms.

Recommended changes:

- Trace foreground blocking separately from background enrichment.
- Give foreground recall a defined latency budget and cancellation behavior.
- Verify whether improve and injection receive appropriate failure isolation.
- Let optional enrichment fail open, while exposing degraded status.
- Distinguish service failure, timeout, policy abstention, empty retrieval, and
  successful-but-unused retrieval.
- Measure memory usefulness: injected facts used, repeated corrections avoided,
  and task outcomes improved.

Acceptance criteria: an unavailable memory service cannot indefinitely block a
normal task; cancellation propagates; timeout is distinguishable from an empty
successful recall.

## 3. Tool permissions and capability discovery are inconsistent

**Priority: P1. Confidence: observed.**

`context-mode_ctx_execute_file` produced 13 errors out of 19 recorded calls.
All 13 errors were permission-boundary rejections.

This is not evidence that sandbox restrictions should be removed. It indicates
that the agent repeatedly chose a tool before discovering the relevant boundary.

Recommended changes:

- Expose effective project root and allowed read roots consistently.
- Preflight permissions before running an analysis job.
- Return structured error categories such as `permission_denied`,
  `outside_project_root`, and `approval_required`.
- Provide an explicit, scoped approval path without silently widening access.
- Explain intentional differences between native read tools and MCP tools.

Direct audit experience also exposed a capability mismatch: instructions required
a todo tool, but the capability search returned no matching capability. That
proves a discovery/instruction mismatch in this session, not universal absence of
any todo integration.

Recommended change: generate capability-dependent instructions from the active
session catalog. Do not mandate unavailable tools. Where a capability is lazy
loaded, provide a dependable discovery route.

## 4. Provider validation and retry classification need stronger contracts

**Priority: P1. Confidence: historical symptoms confirmed; current reproduction
and version attribution remain necessary.**

Selected historical errors:

| Error class | Recorded occurrences |
| --- | ---: |
| Unsupported content type | 5 |
| Output-token-limit validation | 3 |
| Prompt plus requested output exceeded context window | 1 |
| Model rejected disabled thinking | 1 |
| Insufficient balance reported as HTTP 429 | 5 |
| Invalid API key | At least 1 |
| Local endpoint transport failures | Multiple |
| Missing browser executable in an upstream service | 1 |

These have different owners and recovery paths. A billing-related HTTP 429 must
not be treated as an ordinary temporary rate limit. An upstream missing-browser
failure is not proof of a defect in the Rust harness.

Current `crates/ice-ai/src/simple_options.rs:95` already clamps output allowance
against estimated context size, with a 4,096-token margin. `request_max_tokens`
also applies hard output limits. Historical failures do not prove these protections
are absent or still broken in the same way.

Recommended changes:

- Define provider/model contracts for accepted content, thinking modes, and
  context/output limits.
- Validate final wire payloads after adapter transformations.
- Include tool schemas and other actual request overhead in budget validation.
- Classify billing, authentication, invalid request, overload, and transport
  errors separately.
- Retry only failures that are plausibly transient; make any repair attempt
  bounded and visible.
- Record build and adapter identity to separate fixed historical errors from
  current regressions.
- Convert observed error shapes into offline adapter tests.

## 5. Output budgeting should not depend entirely on agent behavior

**Priority: P1/P2. Confidence: large text results observed; waste not established
for every result.**

There were 48 tool results with more than 50,000 characters of text. The largest
was approximately 85,000 characters from `context-mode_ctx_execute`.

An earlier serialized-size calculation found results above 100,000 characters,
including one around 1 MB. Inspection showed those eight oversized serialized
results were image-bearing `read` outputs. They are not megabyte-scale text
context leaks. Text and image budgets must be evaluated separately.

Recommended changes:

- Apply an aggregate per-turn result budget, not only a per-tool size cap.
- Preserve full output as an artifact with a stable retrieval handle.
- Return relevant excerpts and structured status rather than silent truncation.
- Preserve exit codes, errors, warnings, and mandatory verification results.
- Avoid repeatedly echoing long input scripts with their outputs when unnecessary.
- Treat image payload size separately from estimated model input cost.

Context-mode is helpful, but prompting the agent to always select the right tool
is weaker than a harness-level budget and recoverable artifact protocol.

## 6. Observability needs an analysis contract, not only UI events

**Priority: P2. Confidence: directly observed schema friction.**

The database stores `thread`, `header`, `activity`, and `activity_update` events.
Many useful facts are nested in JSON payloads; some `raw` fields are JSON encoded
as strings. Counting updates as independent actions inflates totals.

During the audit, an analysis script failed because it assumed a `raw` field was
an object when it was a string. That script mistake belongs to the auditor, but
a stable typed query surface would avoid the need for repeated schema discovery.

Recommended changes:

- Publish a versioned event schema and documented lifecycle semantics.
- Offer normalized read-only views for latest activities, tool outcomes, and run
  liveness.
- Keep stable request/activity identifiers for deduplication.
- Record foreground/background status and critical-path spans.
- Include build, configuration-schema, and provider-adapter versions.
- Make privacy-safe aggregate export possible without exporting conversations.

## 7. Raw tool latency is not equivalent to wasted time

The observability sample showed 144 `bash_output` calls with approximately 5,120
seconds of summed duration; p95 was approximately 169 seconds. These waits may
be deliberate waits on useful work, not inefficient polling. Summed tool time
also is not user wall-clock time when operations overlap.

Selected observed latency samples:

| Tool | Sample count | Median duration | p95 duration |
| --- | ---: | ---: | ---: |
| bash_output | 144 | 994 ms | 168,810 ms |
| bash | 723 | 150 ms | 5,093 ms |
| context-mode_ctx_execute | 517 | 178 ms | 13,238 ms |
| read | 692 | 154 ms | 350 ms |
| context-mode_ctx_search | 51 | 91 ms | 222 ms |

These are descriptive samples from deduplicated observability activities, not a
cross-tool benchmark. Workloads differ.

Deeper work should identify repeated empty polls, avoidable serial waits,
background notification opportunities, and time spent waiting after work was
already complete before recommending orchestration changes.

## 8. Agent-experience and instruction quality

Directly observed during this audit:

- A required todo workflow had no discoverable matching tool.
- Tool-selection and output-management guidance was overlapping and sometimes
  cumbersome to reconcile.
- A skill read was visibly transformed/truncated, despite instructions to read
  the complete skill.
- Context tools sometimes echoed substantial input code alongside results.
- Indexed output retrieval introduced extra round trips for some simple answers.
- A chart-rendering attempt did not yield a confirmed rendered chart. Tool
  discovery and shell interception did not provide a clear completion path.

These observations are specific to this composed session environment. Attribution
among the Rust harness, MCP server, output transformer, and outer host requires
tracing; they should not all be filed as Rust-core defects.

Recommended changes:

- Generate a compact, capability-aware instruction set.
- Establish clear precedence for overlapping workflow rules.
- Distinguish exact file reads from summarized/transformed reads.
- Provide an explicit complete-read path for mandatory instructions.
- Return consistent tool availability and completion errors.
- Measure instruction overhead before attempting prompt-size reductions.

Some tool failures during the audit were agent-authored script errors. Better
recovery and diagnostics are appropriate; blaming all such errors on the harness
is not.

## 9. Additional session aggregates

Recorded assistant stop reasons:

| Stop reason | Count |
| --- | ---: |
| tooluse | 2,863 |
| stop | 523 |
| aborted | 49 |
| error | 22 |
| length | 1 |

Aborts are not necessarily defects: user cancellation and steering can be valid.
Likewise, a failed shell command can be an expected test result or exploratory
probe rather than a harness failure.

Selected tool result counts:

| Tool | Results | Marked error |
| --- | ---: | ---: |
| bash | 1,999 | 148 |
| read | 1,233 | 11 |
| context-mode_ctx_execute | 675 | 28 |
| edit | 209 | 8 |
| bash_output | 134 | 0 |
| context-mode_ctx_search | 79 | 0 |
| context-mode_ctx_execute_file | 19 | 13 |

Context execution failures included agent syntax/type mistakes, a missing module,
and an MCP timeout. These should not be treated as a single infrastructure failure
class.

Other recorded lifecycle facts:

- 625 operation-started entries and 603 operation-finished entries.
- 4,598 tool-started entries and 4,574 tool results.
- 48 compaction entries and 14 retry-pending entries.

Differences between starts and finishes are investigation leads, not proven lost
operations: active sessions, historical format differences, and replay semantics
must be reconciled by identity.

Recorded usage totals included 151,898,292 total tokens, 138,984,776 cache-read
tokens, 11,654,346 input tokens, and 1,259,170 output tokens. These are cumulative
message usage fields, not unique task content, a billing audit, or a measure of
avoidable waste. Cache semantics and copied histories need verification before
cost conclusions.

## 10. Build a replay-based evaluation suite

Tool success is not task success. The current evidence is useful for reliability
triage but insufficient to establish task completion quality.

Create sanitized fixtures covering:

1. Completed memory replay and unknown lifecycle phases.
2. Abrupt exit, restart, ACP disconnect, and live concurrent sessions.
3. Memory timeout while the main task continues.
4. Permission denial with a scoped approval/recovery path.
5. Provider validation, billing-related 429, and authentication failures.
6. Large text output with recoverable full artifacts and preserved diagnostics.
7. Cancellation during a side-effecting tool and restart without duplicate effects.
8. Compaction preserving constraints, decisions, and unfinished work.

Measure task completion, user correction, time to first useful action, foreground
waiting by subsystem, repeated attempts, context overhead, and interruption
recovery correctness. Use fixed faux-provider tests for deterministic guarantees
and a separate controlled model evaluation for agent behavior.

## Recommended implementation sequence

The initial sequence below is superseded by the deeper-pass priorities in
§11.10, which elevate telemetry privacy and recovery fault injection.

1. Fix completed-memory lifecycle mapping and add a regression test.
2. Establish truthful run liveness and interruption state.
3. Trace and bound foreground memory work using existing protections.
4. Align permission preflight and capability-dependent instructions.
5. Harden final provider payload validation and retry classification.
6. Add aggregate output budgets and normalized observability queries.
7. Grow a replay evaluation suite alongside each fix.

Keep these as focused changes. Avoid a large rewrite that mixes observability,
memory, provider, and prompt changes without independent acceptance tests.

## Deeper investigation: feasible and recommended

The following plan was recorded after the initial pass. Section 11 documents the
subsequent deeper investigation, including which parts of this plan were completed
and which still need controlled experiments or semantic evaluation.

### A. Reconstruct incomplete execution lifecycles

Join run, operation, step, assistant entry, and tool-call identities. Explain each
unmatched start and stale run. Trace shutdown ownership across ACP and interactive
paths. Deliver a classified list of live, interrupted, replayed, and genuinely
unreconciled operations, plus minimal reproductions.

### B. Explain every recurrent memory failure

Join memory request IDs to phase transitions and main-agent spans. Separate
upstream timeout from local deadline, cancellation, and classification bugs.
Measure critical-path delay, verify circuit-breaker coverage, and test degraded
service behavior. No live service changes are required for offline reproductions.

### C. Verify provider fixes against actual versions

Attribute failures to build/provider versions where recoverable. Trace model
metadata through final wire serialization. Reproduce error shapes with fixtures;
verify image/content compatibility, thinking constraints, tool-schema overhead,
and output budgets. Report which symptoms are already fixed versus current.

### D. Audit compaction and task continuity

Inspect all 48 compaction records and surrounding retained history. Evaluate
whether user requirements, file paths, tests, decisions, and unresolved work
survive. Look for repeated reads and user corrections after compaction. This
requires semantic review, not just counts.

### E. Analyze permission and retry journeys

Follow each denied tool invocation through the agent's next actions. Identify
repeated doomed calls, unclear approval paths, or accidental tool substitution
across permission boundaries. Preserve security restrictions while improving
recoverability.

### F. Measure real productivity and context overhead

Select representative completed and abandoned tasks for manual review. Attribute
latency to provider generation, tools, memory, waiting, and orchestration. Estimate
prompt/tool-output overhead using the actual request shape. Do not infer wasted
time from summed tool durations alone.

### G. Evaluate persistence and privacy

Inspect crash recovery, write ordering, replay idempotency, and cancellation
semantics for side effects. Separately review telemetry redaction, artifact
retention, access controls, and whether query exports can avoid private content.
No secret values need to be printed or copied into reports.

### Deliverables for the deeper stage

- Version-attributed findings with confidence and ownership.
- Sanitized, reproducible fixtures for confirmed defects.
- A causal timeline for representative failures.
- A prioritized issue/PR plan with acceptance tests, without publishing anything
  unless requested.
- Explicit unresolved questions and coverage gaps.

The highest-value immediate deeper investigation is lifecycle reconstruction,
followed by memory critical-path tracing. They address the strongest evidence and
improve the trustworthiness of all subsequent analysis.

## 11. Deep investigation: frozen evidence, source tracing, and probes

This follow-up was performed at repository revision
`2fcc1618c0e81aec3e465c9fb24669a0d5655d84`, on
`docs/harness-audit-findings`. Current-source conclusions below refer to that
revision, not necessarily the binaries that generated historical records.

Evidence labels:

- **Reproduced:** a synthetic offline probe exercised the current implementation.
- **Observed:** present in captured records, without proving a historical cause.
- **Source-confirmed:** implementation behavior established by source inspection.
- **Risk/hypothesis:** plausible failure mechanism, not yet reproduced end to end.

### 11.1 Capture, provenance, and privacy boundaries

The deeper pass used a private workspace, `/tmp/icers-deep-audit-MmUPQi`:

- SQLite backup of the live observability database, queried read-only afterward.
- Copies of 347 session JSONL files from the available registered roots,
  totaling 111,539,304 bytes, and a source manifest.
- The current and rotated Cognee observation logs: 4,292 events in total.
- Analysis helpers, synthetic probes, an isolated test-agent directory, and
  selected verification logs.

The workspace directory is mode `700`; the captured database and manifest are
mode `600`. The source agent directory is mode `700`, and the inspected source
SQLite and Cognee log files are mode `600`. Credentials were not inspected or
included in the report. The copies contain private conversations and must not be
committed, uploaded, or treated as public fixtures. They remain temporary local
artifacts, not a durable evidence archive; the findings and synthetic examples
in this document are the durable deliverable.

The database backup contains **240 runs and 12,487 events**. It is internally
consistent, but the database and each copied log were not captured in one atomic
cross-file operation. A file copied while its writer is active can end before a
later result. Three registered roots were unavailable. File-path deduplication
does not eliminate copied logical conversations.

The initial totals above and these later totals describe different captures.
Neither population is a denominator for the other's error rate. No exact
historical build attribution was recovered, and no live provider or memory
service was invoked for validation.

### 11.2 Lifecycle: reproduced mapping bug, unresolved shutdown attribution

**Reproduced — completed memory replay is still marked running.** Constructing an
`Observation` with `operation = "warmup_replay"` and `phase = "completed"`, then
passing it to `ObserveRecorder::handle_memory`, produces
`Some(ObserveItemState::Running)`. This confirms the current-source defect behind
the 123 misleading replay rows identified in §1.1.

`crates/ice-cli/src/observe/recorder.rs:978` recognizes only `"succeeded"` and
`"failed"` as terminal memory phases. The fix should share lifecycle semantics
between producers, persistence, and UI projection rather than add unrelated
string checks in each consumer. Unknown phases should remain observable rather
than silently claiming that an operation is actively running.

**Observed — run status substantially outlives plausible process liveness.** The
backup still has 227 `running` and 13 `finished` runs, across 228 session IDs; six
session IDs have multiple runs. Thirty-six runs have no activity events. A local
process inspection saw four `icers` executable processes, but this is not a
process-to-run join and cannot classify each database row. Stale running rows
occur on multiple dates beginning October 26, not only in the latest ACP usage.

**Source-confirmed — ACP attempts graceful shutdown.**
`crates/ice-cli/src/acp/session.rs:499–505` sends RPC `stop`, waits three seconds,
and kills only if the child does not exit. The RPC handler in
`crates/ice-cli/src/modes.rs:570–581` acknowledges stop, sets cancellation on exit,
and breaks. It is incorrect to describe ACP as unconditionally killing every
session without a stop request.

**Risk — the shutdown budgets and ownership may not agree.** Cognee's
`wait_for_background` drains join handles without an outer deadline
(`cognee/runtime.rs:1602–1614`). `session_shutdown` cancels helper work, can await
shutdown improve work, drains background work, and unregisters the agent
(`runtime.rs:3495–3536`). Improve calls have a 30-second deadline. Cleanup paths
that await this work before closing observability can exceed ACP's three-second
grace period. This is a credible mechanism, not proof that it caused the 227
historical rows.

`ObserveRuntime::stop` already drains forwarding, joins its worker, stops the
recorder, and finishes mirroring (`observe/mod.rs:364–383`). The recorder calls
`finish_run`, but persistence failures do not become a durable closing failure.
`update_session_info` does not replace the recorder's run ID; this inspection did
not establish session-info updates as a stale-run generator.

**Acceptance:** test normal RPC stop, stdin EOF, ACP disconnect, cancellation,
slow memory cleanup, process kill, and restart separately. Persist build ID,
process-instance identity, heartbeat/lease expiry, and closing reason. Show
`interrupted` or `unknown` when appropriate; do not rewrite every old `running`
row as successfully finished. A terminal-write failure must be diagnosable.

### 11.3 Execution recovery: good safeguards and a second-crash window

**Observed — identity reconciliation narrows the missing-result problem.** There
are 22 unmatched operation starts and 24 tool calls without results in the
captured histories. All 24 missing results belong to unfinished operations.
This is stronger than comparing unrelated start/end totals, but it does not
establish a committed successful operation that lost its results. Capture
boundaries, crashes, and cancellation remain possible explanations.

**Existing safeguards:** recovery distinguishes tools safe to replay from tools
that must never be replayed automatically, and supplies unknown-outcome results
for unresolved side effects. Assistant-frame salvage, duplicate-recovery
protection, torn-tail handling, atomic publication, and tool-pairing tests exist.
The selected integration suites passed (§11.9). Do not replace these protections
with unconditional tool re-execution.

**Source-level risk — recovery marks completion before salvage is durable.** In
`crates/ice-harness/src/agent_harness.rs:3073–3156`:

1. Find an open operation; return immediately if none exists.
2. Append `OperationFinished` with outcome `Aborted` and code `interrupted`.
3. Resolve interrupted assistant frames and tool results.
4. Append salvaged messages individually.
5. Warn on salvage or message-append failure, then return success.

A second crash or append failure between steps 2 and 4 can leave the operation
closed before salvage is complete. On the next open, the early return can skip
that operation. The existing test
`recovering_the_same_crashed_session_twice_does_not_duplicate` explicitly relies
on the no-open-operation behavior (`tests/harness_run_e2e.rs:1504–1508`), but does
not by itself establish safety during a failed first recovery.

This is **not a reproduced historical data-loss finding**. Original frame and
tool records may still exist and be recoverable by other means. The demonstrated
concern is completion ordering and recovery eligibility, not physical deletion.

**Acceptance:** inject failure after the terminal record, during salvage, and
after each message append; restart twice. Require deterministic, idempotent
recovery with no duplicated side effects, no orphan tool results, and no silently
omitted salvage. Use a resumable recovery marker or atomic/idempotent publication
strategy. Simply moving the terminal append is insufficient unless partial
salvage is deduplicated and concurrent openers are handled.

### 11.4 Memory: local deadline signature, legacy acknowledgments, limited spans

Standalone Cognee logs provide a different population from the UI database:

| Recorded class | Count | Median latency | p95 latency | Maximum |
| --- | ---: | ---: | ---: | ---: |
| Successful recall observations | 334 | 3,813 ms | 5,457 ms | 8,656 ms |
| Successful recall, excluding `short_prompt` | 261 | 3,982 ms | 5,799 ms | 8,656 ms |
| Failed recall observations | 16 | 10,001 ms | 10,005 ms | 10,007 ms |
| Successful improve observations | 58 | 3,032 ms | 25,109 ms | 27,715 ms |
| Failed improve observations | 148 | 30,001 ms | 30,002 ms | 30,008 ms |

Quantiles use sorted observations, median index `n // 2`, and p95 index
`floor((n - 1) * 0.95)`. The 334 successful recall observations include 73
`short_prompt` outcomes; they are not 334 demonstrated remote backend searches.

**Observed plus source-confirmed — improve failures match a local deadline.** All
148 failed improves are timeout outcomes between 30,000 and 30,008 ms. The current
runtime defines `DREAM_IMPROVE_TIMEOUT_MS = 30_000` and passes it to improve calls
(`cognee/runtime.rs:97,2391,2612`). This local deadline signature does not tell us
whether the remote service continued working, accepted a request before the
client timed out, or committed useful memory afterward.

There are 250 improve starts and 206 terminal observations, leaving 44 unmatched
start IDs within these two logs; there are no duplicate nonempty start IDs and
no unmatched terminal records for this operation. Rotation boundaries, shutdown,
active calls, or interrupted logging can all contribute. Do not count the 44 as
44 proven lost jobs or assume the 148 failures are retries of the same content.

**Current protections are more substantial than a timeout alone.** The runtime
has in-flight suppression, accepted-revision checks, a five-minute idle cooldown,
and failure delays of one minute then five minutes in its improve suppression
logic (`runtime.rs:2181–2397`). Recall/remember circuit breakers also already
exist. Historical timeouts do not prove those current protections were absent,
ineffective, or configured identically in the generating binaries.

**Observed — injection failures use a legacy outcome schema.** All 100 failed
injection observations have `meta.outcome = "append_failed"`,
`meta.outcomeVersion = 1`, `persistedBytes = 0`, and
`estimatedPersistedTokens = 0`. Their top-level `error` and `reason` fields are
absent. Eight successful injection acknowledgments also use version 1. Failed
acknowledgments span October 30 through November 3, 07:01 UTC.

The boolean persistence callback conflates rejected packets with failed durable
appends. A false acknowledgment can follow rejection before a filesystem append,
not just I/O failure. See `agent_harness.rs:4865–4946`,
`cognee/runtime.rs:4432–4470`, and `cognee/request.rs:560–615`.
The current request-based path emits outcome version 2, so the captured version-1
failures cannot simply be assigned to it. Also, the extension contract explicitly
says persisted does **not** guarantee delivery to the model
(`crates/ice-harness/src/extension.rs:185–188`).

**Foreground impact is only partially measured.** Three successful recall
observations explicitly record `foregroundWaitMs`: 4,338, 4,399, and 4,506 ms.
Remote-wait fields appear on 110 successes and one failure, but remote elapsed
time is not necessarily main-agent blocking time. The much longer explicit
`cognee_recall` tool durations in the initial audit are another population.

The current request path can await explicit-history recall under a configured
budget and cancellation signal (`cognee/request.rs:395–421`). Ordinary request
handling also has cache/prefetch paths; it is inaccurate to claim every turn
blocks on a remote recall. Existing request-boundary tests check plain-prompt
network avoidance, cancellation, provenance, budget rejection, and acknowledgments.

**Acceptance:** correlate logical memory job ID, transport attempt, server
acceptance, revision, agent operation, foreground span, and final publication.
Separate `rejected_budget`, `stale_operation`, `cancelled`, `append_io_failed`,
`persisted`, and `delivered` outcomes. Test a server that accepts work then
outlives the client deadline, and bound shutdown independently from optional
background improvement. Measure usefulness and task quality before increasing
latency budgets or disabling memory globally.

### 11.5 Provider behavior: a current retry bug and inconsistent estimators

**Reproduced — billing failure is classified as retryable.** Feeding an assistant
error with HTTP 429 and this sanitized error shape into
`ice_ai::is_retryable_assistant_error` returns `true`:

```json
{"error":{"code":"1113","message":"Insufficient balance or no resource package. Please recharge."}}
```

The classifier in `crates/ice-ai/src/retry.rs:195–230` rejects several quota and
billing phrases but not this one, so the general 429 rule wins. Historical
sessions with this billing error actually scheduled retries. The current five
retry unit tests pass, demonstrating a missing fixture rather than disproving
the gap. This does not mean every 429 is a billing failure.

**Acceptance:** classify provider error codes and structured billing/quota fields
before generic transient-status rules. Add this exact sanitized fixture, related
wording variants, genuine rate-limit cases, and retry-after cases. Billing errors
should produce a direct actionable message without exponential retry loops;
ordinary recoverable rate limiting should remain retryable.

**Reproduced — a 200,000-character tool argument contributes zero estimated tokens
in the provider estimator.** An assistant message containing only a `write` tool
call with `arguments.content = "x".repeat(200000)`, with no system prompt, yields
`0` from `ice_ai::simple_options::estimate_context_tokens`.

That estimator counts text, thinking, and image allowances, but ignores tool
calls. Its signature also has no tools/schema argument. It feeds the context
headroom clamp, which already reserves 4,096 tokens and applies output limits
(`crates/ice-ai/src/simple_options.rs:67–114`). The blind spot can leave too much
output headroom even though output-token limits themselves are enforced.

**Important correction:** the harness has a separate, stronger estimator.
`crates/ice-harness/src/compaction/tokens.rs:178–179,277–280` counts tool arguments,
and `estimate_request_tokens` at lines 377–394 includes schemas and system text.
This is estimator inconsistency, not proof that compaction ignores every tool
argument. Nor does the synthetic zero prove the historical context-overflow
incident had this cause.

**Acceptance:** evaluate the complete final provider request, including tool
schemas, tool-call arguments, system text, images, and protocol overhead. Add
cross-estimator fixtures or one shared contract, preserving provider-specific
allowances. Verify that increased serialized input never increases permitted
output and that already-overfull input is handled explicitly.

**Historical content-type failures may already have a safeguard.** The current
Codex adapter sets `content-type: application/json`
(`providers/openai_codex_responses.rs:420`) and has a local server fixture that
rejects the wrong request content type (lines 1300–1335). This does not establish
which adapter/version produced every historical “Unsupported content type”
message. Test headers, response MIME types, and JSON content-block validation
separately rather than assuming that phrase always identifies the same defect.

### 11.6 Privacy: live and durable redaction differ

**Reproduced — live SSE serialization does not apply the store redactor.** For a
synthetic payload containing `api_key=DUMMY_AUDIT_NOT_A_REAL_SECRET`, calling the
public SSE frame serializer preserves the dummy value; passing the same value
through `observe::store::redact_value` removes it.

Source tracing establishes the relevant path: recorder events are emitted from
in-memory values (`observe/recorder.rs:510–516`), while database writes use the
store redactor. Live SSE frames serialize those events directly
(`observe/server.rs:837–858`). A generic serializer alone need not redact, but
these call paths require a common sanitization boundary if the live view is
expected to offer the same protection as persisted telemetry.

**Additional synthetic redactor checks:** structured `api_key` and a
JSON-stringified `api_key` are redacted; a prefixed environment assignment such as
`EXAMPLE_API_KEY=DUMMY…` and a `Cookie` header containing a dummy session value are
not. These are coverage gaps in `observe/store.rs:62–125`, not a demonstration
that real credentials appeared in telemetry.

**Scope and existing controls:** the observer server is loopback-bound and uses
a per-user access token; source database/log permissions are restrictive. No
unauthenticated remote exposure, token bypass, or actual secret disclosure was
demonstrated. Local authorization reduces exposure but does not make inconsistent
redaction harmless to authorized viewers, exports, screenshots, or later sharing.

**Acceptance:** sanitize once before any telemetry fan-out and apply equivalent
policy to live snapshots, SSE, durable history, mirrors, and exports. Test nested
and stringified JSON, authorization and cookie headers, environment assignments,
URLs, and mixed prose using synthetic values only. Avoid blanket removal that
makes debugging impossible; prefer explicit allowlists for sensitive structured
fields and conservative text handling. Review this finding privately with the
owner before any publication.

### 11.7 Observability memory pressure and analysis contracts

The snapshot's 12,487 event payloads occupy **48,997,140 UTF-8 bytes** when summed
from `payload_json`, excluding database/index overhead. Median event size is
1,441 bytes, p95 8,258 bytes, and maximum 1,051,947 bytes.

Reducing activity/update records to the latest `(run_id, item.id)` yields 6,345
items. Compact JSON reserialization of those items totals **41,327,872 bytes**;
median 2,110 bytes, p95 13,493 bytes, maximum 1,051,920 bytes. Different
serialization and populations mean these totals are not a precise compression
ratio, and neither is a measurement of process heap usage.

**Source-confirmed capacity gap:** recorder retention caps items at 500 and
individual payload strings at 1,048,576 characters
(`observe/recorder.rs:31–34`). Its broadcast channel is count-bounded, but the
worker command channel uses `std::sync::mpsc::channel`, which is unbounded
(`observe/mod.rs:227`). Per-string and per-item caps do not impose an aggregate
byte limit across items, queue entries, listeners, serialization buffers, or
copies. No overload/OOM was reproduced in this audit.

**Acceptance:** instrument queue depth and bytes, slow-consumer lag, persistence
latency, dropped/coalesced updates, and artifact sizes. Give high-frequency
updates bounded/coalescing behavior while preserving terminal events; test slow
disk and slow listeners. Separate image bytes from model-facing text budgets.
Return recoverable artifact references for large payloads, with access controls
and retention policy, rather than silently dropping evidence.

A versioned analysis export should normalize nested/stringified JSON, phase
semantics, timestamp units, build provenance, and identities. Include whether a
record is an update, whether a terminal outcome is known, and why an event was
dropped. Analysts should not need to reconstruct lifecycle rules from UI strings.

### 11.8 Compaction, permission journeys, and agent behavior

**Observed — tool pairing survives the captured compaction tails.** All 48
compaction records were checked structurally; the retained tails had zero orphan
tool results and zero unresolved tool calls. Summaries commonly carry goals,
constraints, progress, decisions, next steps, and critical context. Representative
summaries were inspected, but no complete requirement-by-requirement semantic
scoring of all 48 summaries was performed.

A repeat heuristic examined the next ten tool calls after each compaction, or
fewer if the history ended: 401 windowed calls in total. Only two of the 48
windows included exact `(tool name, canonical JSON arguments)` repeats of earlier
calls in the same captured file: two `read` calls and one `mcp` call. Windows can
overlap, repeated reads can be legitimate, and paraphrased or reformulated work
is not detected. This is not a measured productivity-loss rate or proof of
semantic fidelity.

**Acceptance:** establish a human-reviewed requirement/decision ledger for each
compaction, compare it with the summary plus retained tail, and inspect subsequent
behavior. Score missing constraints, incorrect completion claims, paths/tests
lost, and user corrections separately from justified rechecks. Structural pairing
tests should remain a distinct gate.

**Permissions:** the original 13 failures in 19 `context-mode_ctx_execute_file`
calls were boundary rejections. Selected histories show tool switching after
errors/denials. This justifies investigating common effective roots and approval
semantics, but does not prove unauthorized access or a security bypass. Different
tools may intentionally have different grants; agents should not substitute a
tool merely to evade a denial. A useful regression journey records the requested
path, effective allowed roots, rejection category, approval scope, and subsequent
authorized action without logging private file contents.

**Agent and host friction reproduced during this investigation:** broad output
was visibly truncated by the response transformer, skill reads needed smaller
ranges, context-execution output echoed scripts, and no todo capability was
discoverable despite a mandatory todo instruction. Responsibility may lie in the
outer host, MCP server, transformer, or Rust harness; the observed symptom alone
does not assign ownership. Capability-aware instructions and truncation metadata
would reduce repeated discovery and false confidence in partial reads.

The investigation itself also selected zero tests twice with incorrect
filters/features. Those commands were detected and corrected rather than counted
as passes. Test tooling should surface “zero selected” as a verification warning.

### 11.9 Reproduction and verification ledger

All executed tests used offline dependency resolution. Test execution set
`ICERS_CODING_AGENT_DIR=/tmp/icers-deep-audit-MmUPQi/test-agent` and
`ICERS_OFFLINE=1`; tests used synthetic/local fixtures rather than live providers.
These were focused checks, not the full workspace CI gate.

| Command / selection | Result |
| --- | --- |
| `cargo test -p ice-ai --lib --locked --offline retry::` | Exit 0; 5 passed |
| `cargo test -p ice-cli --lib --locked --offline observe::` | Exit 0; 59 passed, 1 ignored |
| `cargo test -p ice-harness --lib --locked --offline interrupted` | Exit 0; 1 passed |
| Selected harness integration suites listed below | Exit 0; 65 passed |
| `cargo test -p ice-cli --lib --locked --offline request_boundary_` | Exit 0; 9 passed |
| `cargo test -p ice-ai --lib --locked --offline --features providers openai_codex_responses::` | Exit 0; 20 passed |
| Temporary public-API Cargo probe | Exit 0; all four reported gaps reproduced |

Total across the selected test runs: **159 passed, 0 failed, 1 ignored**. The
corrected memory and Codex runs emitted no warnings. Full logs for those runs are
in `memory-request-tests-corrected.log` and `codex-tests-corrected.log` inside the
private workspace. These passing suites do not include newly committed regression
tests for the four probe findings; no production/test source was changed.

Harness integration command:

```sh
cargo test -p ice-harness --locked --offline \
  --test frame_progress_recovery \
  --test m5b_session_state \
  --test m5c_jsonl_torn_tail \
  --test m5c_jsonl_atomic_publish \
  --test coding_retry_safety \
  --test compaction_cut_point \
  --test compaction_split_turn
```

The suites passed 6, 24, 8, 12, 4, 8, and 3 tests respectively. Passing these tests
demonstrates useful existing safeguards, not coverage of the new failure cases.

The temporary Cargo project used path dependencies on the current `ice-cli` and
`ice-ai` crates, cached dependencies, and the repository target directory. It
lived outside the repository and did not modify the workspace manifest or lock.
Its synthetic inputs and observed output were:

| Input and public API | Observed output |
| --- | --- |
| Completed `warmup_replay` → `ObserveRecorder::handle_memory` | `completed_memory_state=Some(Running)` |
| Dummy `api_key` text → `observe::server::sse_frame` | `live_sse_dummy_survives=true` |
| Same payload → `observe::store::redact_value` | `persisted_redaction_dummy_survives=false` |
| HTTP 429 / code 1113 billing error → `is_retryable_assistant_error` | `billing_retryable=true` |
| Assistant tool call with 200,000-character argument → provider estimator | `tool_argument_chars=200000 estimated_tokens=0` |

The exact constructors are `ObserveSessionInfo::default()`, no persistence sink,
`AssistantMessage::empty(Api::AnthropicMessages, "faux", "faux-1", 0)`, and
`Message::Assistant(Box::new(message))`. The synthetic error sets
`StopReason::Error`; the estimator uses no system prompt. No real credential is
needed for any reproduction.

A separate extracted-function Rust probe checked structured/stringified keys,
prefixed environment assignments, and cookies. It compiled with a
private-interface warning caused by its local stub. An initial direct-link probe
failed with `E0463` (dependency artifact mismatch); the Cargo version initially
failed with `E0308` (missing `Box` in the probe), then passed after correction.
These are audit-probe setup errors, not repository build failures.

The first `cognee::request_tests::` and featureless `openai_codex_responses::`
filters both exited 0 with **zero selected tests**. They are excluded from the
passing-test counts. The corrected selections appear in the table above.

Document checks: `git diff --check` exited 0, but the report is untracked, so it
was also checked directly. `git diff --no-index --check -- /dev/null
<report-path>` emitted no whitespace diagnostics and exited 1 because the file
differs from `/dev/null`; that exit is not a test failure. Direct checks found no
trailing whitespace and balanced fenced-code delimiters. Working-tree inspection
confirmed that only this report is untracked; no tracked source/configuration
changes, commit, push, or PR resulted from the investigation.

### 11.10 Revised priorities and remaining depth

| Priority | Focused change / experiment | Evidence and exit criterion |
| --- | --- | --- |
| P1 | Common telemetry redaction boundary | Synthetic live/store discrepancy; no dummy secret survives any telemetry surface |
| P1 | Completed-memory lifecycle mapping | Reproduced current defect; shared terminal-state fixtures pass |
| P1 | Recovery interruption fault injection | Source-level ordering risk; recover correctly after every write boundary without replaying unsafe tools |
| P1 | Truthful liveness and bounded shutdown | Stale records plus deadline mismatch; distinguish live, interrupted, unknown, and finished under process/transport failure |
| P2 | Billing-aware retry classification | Reproduced 1113/429 case; stop permanent billing retries while retaining transient retry behavior |
| P2 | Consistent full-request token budgeting | Reproduced provider estimate of zero; arguments, schemas, and protocol overhead included |
| P2 | Memory outcome/critical-path tracing | Deadline signature and ambiguous legacy acknowledgments; distinguish acceptance, persistence, delivery, and useful completion |
| P2 | Permission preflight and capability-aware instructions | Repeated denials and missing todo; recover via authorized actions without blind retries |
| P3 | Byte-bounded observability and output artifacts | Source-level unbounded queue; demonstrate bounded memory under slow consumers |
| P3 | Semantic compaction and task-outcome evaluation | Pairing is sound in the sample; requirement preservation and actual productivity still need scoring |

No production fixes were implemented, no commits were created, and nothing was
pushed or published. The report is the only repository change. This debugging
report is not part of the `docs/ice-tool/` embedded-document source set; no
generated embedded copy is needed.

The next meaningful depth comes from **controlled experiments**, not more raw
aggregate counting:

1. Use a faux provider and injected append failures to establish whether the
   second-crash recovery window actually omits salvage after restart.
2. Use an isolated ACP child with deliberately slow optional cleanup; verify
   terminal writes, cancellation propagation, and reconciliation after forced
   exit. Repeat EOF, abrupt disconnect, and concurrent-process cases separately.
3. Add a fake memory server that acknowledges before completion, exceeds the
   local deadline, and later succeeds; measure idempotency and foreground delay.
4. Attribute historical incidents to executable/build identity where recoverable.
   Current checkout tests cannot prove an old installed binary had the same code.
5. Semantically score all 48 compactions and representative task outcomes against
   a requirement ledger. No comprehensive task-quality or user-correction rate
   has yet been established.
6. Turn synthetic cases into small repository regression fixtures alongside each
   approved fix. Keep private histories out of the fixtures and do not publish
   security findings or open issues without owner approval.

These limits are deliberate: this pass goes beyond symptoms with reproducible
current-code evidence, but does not claim a complete causal explanation for every
historical timeout, stale run, interrupted operation, or abandoned task.
