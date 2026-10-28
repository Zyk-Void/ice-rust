# M5.4 remaining JEV evaluation evidence and authorization request

- Date: 2026-10-28
- Status: **BLOCKED / not run**. This document prepares a reproducible protocol; it is not a benchmark result.
- No real JEV request, endpoint contact, model call, independent label, or new A/B/C/D/E result was produced. The helper setting is enabled by default in repository configuration, but no endpoint/model is inferred or auto-configured; missing binding safely prevents dispatch. Existing synthetic offline results remain synthetic.

## What is already locally established

The helper constructs a bounded typed `typesafe::Request`, wraps a bound `DecisionClient` in `ClientDecider`, enforces one deadline/cancellation path, and validates exact response question IDs/types before returning judgments. `DecisionClient` fixes the TypeSafe System One endpoint path, disables redirects, avoids retries, bounds response data and checks cancellation/deadline. Provider configuration comes only from `models.json` `decisionProviders`; it does not fall back to a chat provider. The local tests exercise helper behavior through `FakeDecider` and also join the layers with the real typed client against a synthetic loopback fixture.

This change adds `helper_judges_through_the_real_typesafe_http_client`: `Helper::judge` uses the actual `ClientDecider(DecisionClient)` path against a one-shot loopback TCP fixture, returning synthetic typed answers. It verifies TypeSafe path, bearer request routing and decoded judgments. It has no external network, provider credential, JEV call or effectiveness score. It is integration coverage only; D remains **not executed**.

The current offline runner uses frozen authored `m7-synthetic-1.0.0` (56 synthetic cases with availability timestamps and expected required/forbidden evidence IDs). It is a chronological policy/runtime regression fixture, not independent human labeling, a held-out user study or JEV data. Its report explicitly states D is not executed and leaves model correctness, TTFT, benefit and backend searchability unknown. Do not relabel that corpus or report as held-out effectiveness evidence.

## Frozen paired evaluation protocol (future; requires authorization)

### Units, split, and labels

1. Before looking at treatment results, freeze an immutable corpus snapshot and SHA-256 manifest. Store only sanitized task/session fixtures and evidence identifiers; no live user transcript is included by default.
2. Split by whole project/task/session clusters, never by adjacent turns: a development partition for prompt/protocol/debugging and a separate held-out partition whose labels and expected outcomes remain unavailable to implementers until the run is locked. Record split IDs, timestamps, repo revision/worktree, corpus hash, task order, eligibility/exclusion reasons and label provenance. The same fact must not cross partitions through duplicated paraphrases or sessions.
3. Labels must be independently produced by authorized human reviewers from source evidence, then adjudicated without access to arm outputs. Record disagreement and unresolved/unknown labels. No labels are fabricated from model responses, synthetic fixture expectations, or the main model's answer. If an independent labeler is unavailable, D is not scored and the effectiveness gate stays blocked.
4. Keep development data separate from held-out data and never retune prompts, thresholds, candidate filters or metrics against held-out results. Any post-unblinding change creates a new protocol/corpus version and needs a fresh held-out set.

### Arms and controls

Pair the same frozen task, starting repository snapshot, initial context, primary generation model/settings, permission set and task order across arms, with isolated stores and no cross-arm writes:

- **A — off:** Cognee disabled; current session/repository/tools only.
- **B — legacy:** current production legacy recall/capture policy, with all inferred writes confined to the isolated authorized dataset.
- **C — selective, helper off:** event/local selective behavior, helper/JEV explicitly off (including explicit `helperEnabled: false`), manual user writes only.
- **D — selective + JEV:** identical to C except the configured TypeSafe JEV judges eligible candidates. This arm is forbidden until explicitly authorized and the isolated endpoint/model are confirmed.
- **E — on demand:** automatic retrieval off; the same explicit search tool is available to the primary model under the same permission policy.

Randomize or counterbalance paired arm order within task/session cluster. Use a fresh isolated profile, project copy and dataset for each arm; snapshot each initial store. Hold service version, endpoint, provider/model ID, prompt, timeout, cache state and task budget fixed within D. Report warm and cold cache separately if both are run.

### Failure-inclusive outcomes

For every eligible paired task, retain a row for every arm, including timeout, cancellation, unavailable endpoint, empty result, unconsumed evidence, task abort and unknown label. Never discard failures from denominators. Record, separately:

- task correctness and constraint adherence, adjudicated from independent source-grounded labels;
- required evidence missed, forbidden/wrong-scope/stale evidence delivered, contradiction/supersession harms;
- evidence requested, retrieved, locally prepared, delivered to the primary model, persisted/accepted, searchable, and independently observed as used (each is a different event; unknown remains unknown);
- primary-model tool actions, repeated investigation, edits/build results, task completion/abort;
- wall time to first useful response and total task time including background work, failures, and memory/helper wait; decision endpoint latency and tokens; query/result/prompt bytes and primary-model token overhead;
- number of eligible paired tasks, missing labels, exclusions with reasons, per-arm failures, and cluster-aware uncertainty intervals.

For categorical quality, publish paired task outcomes and a two-sided 95% cluster-bootstrap interval over whole task/session clusters; no claim if the held-out sample cannot support a stable interval. For latency, publish sample count and nearest-rank p50/p95 including failures as censored/failed outcomes rather than dropping them; publish raw failure count and do not impute a successful latency. Report service accepted != indexed/searchable != delivered != used. No single average or model-reported confidence replaces adjudication.

### Predeclared screening gates and stopping rules

These are prospective gates, not results or guarantees:

- Required pre-run control gates: 100% of critical source-grounded constraints retained in A–E where the arm is expected to support them; zero known cross-project, forbidden-skill, stale-revision or superseded-critical evidence delivered; zero selective-policy bypass; zero ambiguous write automatically replayed; zero ordinary self-contained Cognee/JEV calls in C; D calls only for eligible candidate batches.
- D can advance only if the held-out paired D-minus-C task-correctness difference has a 95% cluster-bootstrap lower bound above zero **and** the predeclared quality/effort composite (correctness primary; total task time and repeated investigation secondary) improves without violating any control gate. This is a conservative engineering screen, not a population guarantee. Report D vs C directly; D vs B is not evidence of incremental JEV benefit.
- Do not dispatch D-arm calls unless an explicit provider/model binding and the separate endpoint, data, credential, request, and billing authorizations below are present. The repository default-on setting is not authorization to contact a service. Keep D off if the quality interval includes no benefit, critical harms occur, the sample is underpowered/unstable, labels are incomplete, costs/latency exceed the authorized budget, or endpoint/version changes. Do not continue until a favorable threshold is reached; freeze the planned sample size and maximum duration before unblinding. A new look or interim analysis requires a predeclared sequential plan.
- Stop immediately on unexpected data destination, unauthorized inference, wrong-project/forbidden evidence, credential leakage, service mutation outside isolated data, excess budget, unexpected foreground blocking, uncontrolled job growth, repeated timeouts, ambiguous duplicate writes or any rollback failure. Preserve raw logs securely, stop new requests, disable helper and retain uncertainty.

The thresholds above are initial safety/engineering acceptance targets and must be approved by the owner and independent evaluator before touching held-out data. They do not imply statistical power or a universal quality claim.

## Exact owner authorization required before any D run

The owner must provide and approve all of the following in writing; a missing/unknown endpoint remains a blocker:

1. Exact decision model/provider identifier and protocol (TypeSafe System One), model/version, provider identity and any requested version pin. Do not infer JEV endpoint from provider names.
2. Exact HTTPS endpoint/base URL and deployment/region, certificate/origin expectations, whether it is an isolated test tenant, who controls it, and confirmation that it invokes only the approved model (including all downstream processing; no fallback/routing to other models).
3. Exact data allowed to leave the host: sanitized candidate text/context fields, maximum payload, retention/training/logging policy, residency, privacy classification, secret/PII screening, deletion and incident contact. Approve the frozen fixture and a data-flow review first.
4. Credential provisioning method (never paste credentials into logs/chat), authorized credential owner, rotation/revocation plan, and confirmation no credential is present in a committed fixture.
5. Hard request, token and monetary budget; max concurrency; per-call deadline; total run window; rate limits; cancellation/stop owner; allowed retries (default none) and billing project.
6. Explicit permission to issue D-arm inference on the named held-out data and whether a separately isolated Cognee service is permitted. JEV/helper and every incidental model must be listed. Current authorization to use `cx/gpt-6-luna` does **not** authorize live JEV.
7. Named independent labelers/adjudicator, conflict handling, access controls, retention/deletion for raw traces, and who may see held-out labels.

Until these are supplied and reviewed, use only offline fixtures and local fake HTTP integration tests. Do not contact a guessed or real endpoint or execute D. The helper setting may remain enabled, but without its explicit binding and separately authorized eligible work it must produce no remote request.