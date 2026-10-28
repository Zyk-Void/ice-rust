# Evaluation protocol: does selective memory actually help?

Prepared 2026-10-27 for the [implementation plan](../cognee-jev-memory-plan.md). Status: **protocol and acceptance targets, not a completed quality benchmark**.

The historical audit is completed. No live model or Cognee service was invoked. Current authorization permits live model testing only with `cx/gpt-6-luna`, including inference indirectly triggered by Cognee operations; JEV-specific quality/latency tests require an expanded authorization. Fake decision responses and recorded backend fixtures can validate integration behavior without making a model request. A real recall/remember/improve probe is not exempt merely because it uses HTTP or runs in the background.

## 1. Questions the evaluation must answer

1. Does the policy retrieve a needed historical fact when current context cannot answer the task?
2. Does it avoid retrieving information already available or irrelevant to the task?
3. Does remembered information change the next action or improve the answer correctly?
4. Does it reduce total task time, repeated tool work, or context consumption after including memory/helper overhead?
5. Does selective remembering preserve useful decisions and corrections while excluding routine activity?
6. Does JEV add measurable benefit beyond deterministic local selection and the main agent's existing memory tool?
7. Do tool/skill suggestions identify a currently available, applicable capability whose full instructions/schema are not already in context?

Request success, nonempty text, and recall badges are operational evidence. They do not answer these quality questions.

## 2. Baselines and ablations

Use the same repository snapshot, initial session context, generation model, generation settings, and task fixtures in each paired comparison. Do not allow one arm to read memories created by another arm.

| Arm | Behavior | Purpose |
|---|---|---|
| A — Memory off | Current session/repository/tools only; no Cognee recall/injection | Establish task baseline |
| B — Current policy | Current four-word/duplicate gate and capture/recall behavior | Reproduce the incumbent cost and quality |
| C — Local selective | Task/event gate, scoped resident facts, provenance/novelty filters, direct search escape hatch; JEV off | Measure benefit of fixing integration without another model |
| D — Local selective + JEV | Same C behavior plus bounded JEV decisions on ambiguous candidates | Isolate JEV's incremental contribution |

Include an **on-demand-only ablation**: automatic recall off, existing `cognee_search` available to the main agent. It may be sufficient for many tasks. Compare it with C before committing to complex automatic prefetch.

Additional small ablations can isolate scope sharing, source-quality filtering, and loaded-skill tracking. Do not run all combinations unless a concrete unresolved question requires them.

## 3. Evaluation layers

### Layer 1: deterministic control flow

Use fake clocks and transports to prove admission, budgets, original operation ownership, cancellation, no stale delivery, caching, status semantics, write uncertainty, and compatibility. These tests do not estimate language-model judgment quality.

Useful properties include:

- Ordinary-turn service call count is exactly zero.
- Explicit history does not require a classifier decision.
- Same event/revision produces at most one scheduled helper request.
- Ten differently worded follow-ups in the same task do not create ten prefetch events; cache expiry alone does not authorize a request.
- Different repository or branch cannot reuse a mismatched cached decision.
- Git ref/revision and conversation branch/leaf are different applicability/ownership fields.
- JEV timeout is unavailable/unknown, not a “no useful memory exists” judgment.
- A rejected durable candidate cannot be promoted through another enabled path.
- A delayed write belongs to its original operation context even after session switch.
- A timed-out accepted remote write is not automatically retried as if unsent.
- A skill is not marked fully loaded by metadata listing or a partial file read.
- A helper result arriving after cancellation cannot change an in-flight model request.
- Ready evidence can reach a later same-task request after a tool result; a late result cannot start an unsolicited turn after task completion.
- JEV-off explicit remembering and unambiguous correction work with host-resolved source spans; fabricated or unrelated provenance is rejected.
- Two sessions updating the shared resident store do not lose either update; shared provider quotas bound multi-session load.
- Retrieval results expire when remote revision is unknown, while generation changes only fence delivery and do not erase safe reuse.

### Layer 2: retrieval and selection replay

Freeze historical candidate sets and evaluate query/admission/selection without generating final answers. The retained log includes previews, not a complete temporal memory snapshot, so it supports diagnostic cases but cannot reconstruct the full historical database perfectly.

Build known-corpus fixtures with creation/availability timestamps for strong retrieval claims. Keep historical replay and synthetic contract fixtures labeled separately. Replay must respect both when a fact was stated and when it became searchable; a fact ingested later cannot be credited to an earlier query.

### Layer 3: paired task outcomes

After implementation, use the allowed generation model in isolated fixtures to compare answer correctness and actual actions. Keep output length/tool permissions consistent, include timeouts in denominators, and record warmed/cold cache state. For edit/build tasks, use disposable repository copies so one arm's changes cannot contaminate another.

A Luna-only run can compare A/B/C and tool-only behavior against recorded or inference-disabled memory fixtures. Live Cognee retrieval or promotion requires checking every downstream model against the same restriction. Such fixture comparisons establish policy/task effects, not live backend latency. A Luna-only run cannot validate D's real TypeSafe latency or accuracy. Do not silently substitute Luna for JEV and label that result a JEV benchmark. Mocked D is integration testing only.

## 4. Case construction and temporal hygiene

Start with approximately 60 explicitly labeled cases across task/session clusters. This is an engineering screening set, not enough to claim a universal accuracy rate. Expand held-out coverage when confidence intervals or rare failures remain too wide.

Suggested mix:

| Category | Starter cases | Labeling focus |
|---|---:|---|
| Required earlier decision/constraint | 12 | Fact absent from current context, available before query, relevant to action |
| Self-contained current task | 10 | No history needed; local action authoritative |
| Short continuity/ambiguous reference | 8 | Distinguish sufficient context, resolvable task reference, and genuinely unclear referent |
| Correction and contradiction | 6 | Correct role/scope/revision; old fact must not prevail |
| Cross-worktree and project boundaries | 6 | Shared repository fact versus unmerged branch state/unrelated repo |
| Tool or unloaded skill opportunity | 6 | Current catalog availability and true full-content coverage |
| Service/cancellation/write uncertainty | 6 | Correct failure semantics and no accidental replay |
| Expanded resources/compaction/novelty | 6 | Preserve original intent, budget, and reinjection only when needed |

Use whole task/session clusters for train/tuning versus held-out splitting. Adjacent turns from one task must not appear on both sides. Do not tune thresholds on the seven selected diagnostic examples and then report their success as generalization. Exclude future answers, later corrections and later memory writes from each historical query's state.

Every case record needs:

```json
{
  "case_id": "stable identifier",
  "origin": "historical or synthetic",
  "task_cluster": "cluster identifier",
  "as_of_time": "timestamp",
  "repository_id": "fixture repository",
  "worktree_id": "fixture worktree",
  "original_user_intent": "bounded request",
  "active_context_evidence_ids": [],
  "skill_descriptors_available": [],
  "skill_bodies_present": [],
  "candidate_ids_available_as_of_query": [],
  "required_fact_ids": [],
  "forbidden_or_superseded_ids": [],
  "expected_first_source": "current context / repository / capability catalog / memory",
  "annotation": "why this case requires or does not require memory"
}
```

Do not fabricate historical state merely to fill this schema. Unknown fields remain unknown; use a controlled fixture for requirements the historical evidence cannot establish.

## 5. Seed diagnostic cases from the actual audit

The full run/request IDs and sequence references are in the observability report, which is withheld from this repository because it quotes private local session activity.

| Case | Existing evidence | Proposed expected behavior |
|---|---|---|
| Build current branch | 4,423 ms recall returning routine activity; incremental build later reports 0.33 s | Skip automatic recall; inspect current instructions and build |
| “Which one is main” in Skyrim task | 3,993 ms; unrelated topics returned | Resolve subject from active task or clarify; no raw global-ish query |
| Commit count | Tool-operation count returned; answer already known from current session | Use current context/Git; avoid history search |
| Planning skill inventory | 4,375 ms; old autocomplete discussion instead of current resource inventory | Search catalog/configured skill roots locally |
| Expanded context-mode invocation | 6,269 ms querying full resource instructions | Query original task intent only if memory is actually needed |
| Prior Skyrim migration | Relevant history retrieved; live launcher state still checked | Preserve relevant scoped pointer, verify volatile state, count savings only if shown |
| Manual memory retrieval | One 404 and one timeout; tool callback marked done | Clear semantic error; do not treat completed callback as successful evidence |

These are selected failure/success-looking episodes, not a random sample. No precision percentage should be derived from them.

## 6. Labels and metrics

### Admission and retrieval

Define a historical-need label before looking at the candidate policy's answer. A case needs memory if a required fact is absent from active context/current authoritative sources and was available in the allowed memory scope at query time.

- **Needed-fact recall:** required fact IDs retrieved and retained divided by required fact IDs available. Report a case-level version as well.
- **Evidence precision:** selected useful, applicable snippets divided by all injected snippets. Missing provenance/unknown relevance is reported separately, not silently counted correct.
- **Unnecessary retrieval rate:** automatic service requests on no-history-needed cases divided by those cases.
- **Missed-need rate:** history-needed cases with no required evidence delivered before the consequential action divided by history-needed cases.
- **Novelty waste:** injected tokens whose facts already existed in active context divided by total injected tokens.
- **Stale/contradictory injection rate:** cases with forbidden, expired, superseded or wrong-scope material delivered divided by eligible cases.

If candidate IDs are absent in legacy graph text, manually label a bounded passage for the evaluation fixture; do not count normalized result objects as atomic facts. Backend `hits` and score are not these metrics.

### Use and outcomes

Track separately: proposed, retrieved, retained, injected, referenced by answer/action, and beneficial. The model saying “I used memory” is not sufficient evidence. Verify the action or answer against case labels. A fact can be mentioned yet have no effect.

For background work, report ready-before-task-completion, delivered-before-consequential-action, `ready_not_consumed`, and `late_after_task_complete` separately. A result that became ready after the answer is not a successful current-task recall. Record queue wait and shared provider admission as well as network time.

For each paired task, record correctness and constraint adherence, relevant tool calls, repeated investigation, total wall time, time to first useful response, memory/JEV service wait, background work, main-model tokens and injected tokens. Record actual tool errors, not only callback state.

Do not subtract the sum of old recall latencies and call it saved user time. Instead compare the observed paired outcome, including required verification that memory should not eliminate. Faster but wrong is a failed result.

### Remembering and promotion

- Candidate precision by category and source role.
- Useful later retrievals from selected facts.
- Duplicate durable writes per semantic key/scope.
- Correction/supersession accuracy.
- Rejected candidates accidentally promoted by another path.
- Accepted-to-searchable lag for supported backend status contracts.
- Accepted/unknown/timed-out job counts with right-censored outcomes kept visible.

A successful HTTP response does not establish searchability. Local curated availability and remote graph availability should be reported separately.

### Capability suggestions

- Applicable and available suggestion rate.
- Unavailable/disabled/forbidden suggestion count.
- Suggestions for already-loaded skill bodies.
- Correct full-body load versus partial read.
- Task improvement from the suggestion versus catalog-only/on-demand behavior.
- Discovery/JEV latency and extra context introduced.

## 7. JEV calibration and cost control

Pin provider protocol, requested model, returned model version, request template, policy revision and candidate corpus revision. Keep thresholds unchanged during the held-out pass. If aliases resolve differently, separate the cohorts.

Noul is a probability-like output, while Choice/Score confidence reflects distribution concentration. None is a universal action-correctness guarantee. Include an applicability judgment or explicit none case for finite selection; enforce availability and policy in code.

For each candidate batch, record call reason, number of candidates/questions, redacted input size, actual latency, timeout/error, selected IDs, and local decision. Batch independent questions sharing state; do not construct a question that depends on another answer in the same request.

Threshold tuning must consider false-negative cost and false-positive pollution separately. A correction/constraint may need a different gate from a suggested optimization. Do not force every borderline candidate into save or discard; bounded pending/abstain is an acceptable result.

Compare D directly with C. If JEV does not improve the held-out quality/effort trade-off, leave it off. A cheaper model bill alone is not a useful metric if the extra dependency slows tasks or loses relevant facts.

## 8. Proposed engineering release gates

These are targets, not measured results:

| Gate | Initial criterion |
|---|---|
| Ordinary fast path | Exactly zero memory/helper service calls; initialized admission p95 ≤10 ms in representative local tests |
| Explicit behavior | All deterministic fixtures for explicit requests, offline, cancel and policy precedence pass |
| Scope and trust | Zero wrong-project/forbidden-skill/permission-bypass cases in the release corpus |
| Durability | Zero automatically replayed ambiguous writes and zero selective-policy bypasses in controlled failure fixtures |
| Needed history | No loss of required facts in designated critical constraint/correction cases; report all misses elsewhere |
| Efficiency | At least 80% reduction in unnecessary automatic requests on the labeled no-history-needed set versus B, with matched task quality |
| Context growth | No duplicate append for unchanged evidence still in active context; combined memory envelope stays within budget |
| JEV enablement | D improves on C for the chosen task utility metric without violating ordinary-turn, scope, latency and cost gates |
| Capability routing | No suggestion of already-loaded full content or unavailable tools in designated exact-match fixtures |
| Observability | Skips/requests/accepted/searchable/used outcomes have correct denominators and no fabricated unknown completions |

The 80% figure is a proposed target for unnecessary calls, not a forecast from the audit. A tiny test set passing every case does not demonstrate population-level safety or a zero error rate. Report counts and uncertainty intervals, cluster-aware where feasible. Broad confidence claims require more held-out tasks.

Treat latency percentiles with care for small samples. Publish n, quantile definition, cache state, endpoint, payload size and failures. The historical audit uses nearest-rank percentiles; future reports should either preserve that convention or disclose the change.

## 9. Staged rollout and stop conditions

1. Run **local shadow admission**: compute local decisions without changing recall behavior and without JEV calls. This establishes reasons and candidate coverage.
2. Enable `readPolicy=event` with `writePolicy=manual` and JEV off for the target repository; retain direct search, explicit writes and local continuity anchors. Keep unchecked automatic promotion disabled. Compare the read canary with the baseline and inspect misses.
3. Enable curated local memory and the corrected durable outbox after reliability tests. Verify promotion paths cannot bypass policy.
4. Evaluate JEV in a separately authorized candidate-only shadow cohort. It must not change user answers or silently store facts while being calibrated.
5. Enable JEV for the specific decision types that show incremental benefit; keep the remaining decisions local/on-demand.

Stop/rollback a candidate rollout on wrong-project evidence, a superseded critical constraint being applied, unexpected foreground helper waits, uncontrolled job growth, duplicate uncertain writes, repeated unavailable-tool suggestions, or an increase in total task time without compensating quality benefit. Disable the helper independently of useful local selective memory. Preserve old data/configuration for rollback.

Do not leave the current broadly automatic mode running indefinitely just to collect perfect statistics. A short reversible local selective canary with measured misses is more useful than adding months of low-value history.

## 10. Reporting format and evidence bundle

Each evaluation report should state code revision, fixture revision, date/window, policy/model versions, case counts by category, actual requests by purpose, query/result size, latency and failure denominators, quality and harm outcomes, paired task results, and excluded/unknown cases with reasons.

Include representative failures and at least one verified positive example. A useful positive example should show the earlier source fact, why it was absent from current context, when it was searchable, the later action that depended on it, and what redundant work or error was avoided. This is the missing evidence behind the user's current complaint.

The retrospective evidence bundle includes a read-only reproduction script, sanitized aggregate JSON, and bounded named case excerpts. Keep raw conversations out of a committed report by default. The reports' episode IDs let an authorized later audit verify the cases without copying the entire database.
