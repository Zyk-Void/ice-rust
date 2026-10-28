## Superseded component-stage evidence

This file records an earlier component-stage run. The final integrated source, verification and acceptance status are in [offline-closeout-report.md](offline-closeout-report.md). Do not use earlier counts or mock-JEV metrics as current results.

# M7 offline chronological evaluation

**Status:** executed synthetic regression evaluation, not a user-quality benchmark.

This is an authored synthetic corpus and production-runtime integration check, not a blind real-user evaluation, population-quality estimate, or held-out study. It contains no private historical chat/audit data. All five arms use the same immutable 56-case corpus (`m7-synthetic-1.0.0`) with frozen `expected_required`/`expected_forbidden` identifiers and per-fact `available_at` timestamps; the runner only puts a fact in fake search results when `available_at < request_ms`. IDs and fixture SHA-256 are recorded in [`evidence/offline-replay.json`](evidence/offline-replay.json). The source fixture is [`evidence/offline-replay-cases.json`](evidence/offline-replay-cases.json).

## Reproduce

From repository root (the report argument is absolute because Cargo tests run with the crate directory as cwd):

```sh
ICERS_MEMORY_REPLAY_REPORT="$PWD/docs/cognee-jev-memory/evidence/offline-replay.json" \
  cargo test -p ice-cli chronological_offline_replay_report -- \
  --ignored --exact cognee::runtime::tests::chronological_offline_replay_report --nocapture
cargo test -p ice-cli chronological_replay_regressions_temporal_boundary_zero_calls_and_denominators
```

No model calls, nested agents, real agent config, real home state, network, or live Cognee/JEV service are used. Each case gets an isolated temporary agent directory, explicit synthetic config, fixed clock, and the actual in-module fake Cognee transport. The driver invokes `CogneeRuntime::request_context`, `search_for_tool` (the explicit lookup host path), and the persisted-context callback. The production policy and selection code are unchanged. Source interface is the `RuntimeOptions`/transport and `HarnessExtension::request_context` interface at the current branch snapshot; future API changes may require updating this test-only adapter.

## Corpus and arms

| Cohort | Count | Coverage |
|---|---:|---|
| Self-contained | 20 | ordinary requests with forbidden unrelated facts |
| Explicit historical decisions | 12 | frozen required decision IDs |
| Repeated follow-ups | 8 | repeated task/session cluster prompts |
| Boundary metadata | 8 | wrong repo/branch/revision, expired, superseded, unknown/malformed metadata, active-context duplicate labels |
| Service and edge | 8 | skill/raw-origin labels, unavailable/timeout/cancel/late-prefetch labels, unanswerable and not-yet-available case |

Arms: **A** memory off; **B** production legacy policy; **C** production event/local-selective with manual writes and helper off; **D** same runtime with mock-JEV scope explicitly reported as not eligible/unverified by this driver; **E** on-demand. On-demand explicit user history needs are simulated with deterministic calls to the actual explicit search API; this is not a claim that ordinary prompts request memory. The report includes request counts, denominators, retained/injected/persisted observations, required misses, forbidden evidence, unknown/failure counts, per-arm harm slots, and local wall latency with nearest-rank quantiles including failures.

Legacy behavior uses the actual legacy runtime policy, whose current recall gate is frozen at this code revision (`is_low_signal_recall_prompt`, four-word admission). The fake transport returns only synthetic records; no result count is described as a network hit or relevant fact. Service delay/error is simulated locally and never represented as remote latency. `faux_used` is zero. D is not evidence that mock JEV improves quality: candidate-ID mock judgment is not implemented in this runner, is marked not eligible/unverified, and no JEV quality conclusion follows.

## Measured synthetic results

| Arm (56 cases each) | Fake requests | Evidence retained | Required misses | Forbidden facts delivered | Estimated injected tokens | Local p50 / p95 (µs) |
|---|---:|---:|---:|---:|---:|---:|
| A: off | 0 | 0 | 33 | 0 | 0 | 0 / 0 |
| B: legacy | 56 | 51 | 5 | 20 | 3,309 | 1,720 / 2,802 |
| C: selective, JEV off | 33 | 28 | 5 | 0 | 1,824 | 1,257 / 2,905 |
| D: selective, JEV not exercised | 33 | 28 | 5 | 0 | 1,824 | 1,333 / 1,871 |
| E: on-demand | 12 | 12 | 21 | 0 | 0 | 366 / 1,168 |

For E, explicit tool use is simulated only in the 12 historical-decision cases; the remaining misses do not predict how a real agent would use its tool. All 20 self-contained C cases made zero fake requests, compared with 20 for B. The warmed local admission sample (`n=1,000`) measured p50 30 µs, p95 38 µs, max 106 µs and zero fake requests. Timings include the runner's local work and are not model TTFT, real backend latency, background processing latency, or task completion time. A pre-fix run incorrectly triggered recall on the negated phrase “no earlier decisions”; `b3c71e8` fixes that measured case. The D arm performs no JEV calls and is not an evaluation of a JEV judgment.

## Interpreting results

The JSON is the measured artifact; it records exact code revision, dirty state, fixture hash, generation date, case counts/cohorts, latency scope, and supplied hardware metadata status. Its tested implementation is `b3c71e883b780a15bdc57b512d70be62657ba67f`; `worktree_dirty` is true because the isolated `.target-completion/` build directory is untracked and the generated report replaces the prior tracked report. No uncommitted source changes were present when the replay started. The runner stores timings from actual runtime calls and reports a separate warmup-excluded local fast-path benchmark (1,010 admissions; first 10 excluded) with cache/window/input details and asserts zero fake-transport calls. These values are machine-specific observations, not a hard CI performance promise.

Positive traces are synthetic end-to-end runtime traces: prior fact timestamp precedes the request, active messages are empty, runtime delivers it before the decision boundary, and the frozen case expects reuse. This is synthetic asserted constraint reuse, not an observed user outcome or saved-time claim. Failure and zero/empty outcomes remain in counts. Boundary labels that are not represented by metadata fields in the fake service (e.g. malformed metadata, late task finish) are only labeled corpus cases, not proven runtime rejection semantics; consult focused production tests for those semantics.

On-demand tool evidence is counted as retained/delivered evidence, separately from automatic injection and persistence: this runner does not persist tool results through a harness. Its historical-decision cohort must have zero required misses; other cohorts do not simulate explicit tool calls. Unmeasured wrong-scope, stale, contradiction, novelty and false-positive admission metrics are JSON `null`, not asserted zero. Writes are manual in every enabled arm, so this corpus cannot measure automatic admission. The future-evidence counter measures actual delivery, not a request made when no fact was available. Context overhead uses the production estimated-token function and is not an actual model tokenizer measurement.

The local zero-request regression runs 1,000 ordinary event-policy request boundaries against the real runtime and asserts that the fake transport receives no calls. The temporal regression includes a future fact and the runner excludes it before constructing fake results. Nearest-rank quantile is tested. The result does not establish main-model task correctness, TTFT, end-to-end time benefit, JEV quality, JEV backend accepted-to-searchable lag, or rollout safety; those are explicitly unverified/unknown/off. No dollar/time-saved estimate is made.

For existing production tests covering task ownership and late delivery, request-boundary prefetch/acknowledgment, explicit lookup scope/cancellation, and duplicate/novelty behavior, see `request_tests.rs`, `lookup_tests.rs`, and the included tests in `runtime.rs`; the replay does not replace those focused assertions.
