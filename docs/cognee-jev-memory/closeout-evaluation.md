## Superseded component-stage evidence

This file records an earlier component-stage run. The final integrated source, verification and acceptance status are in [offline-closeout-report.md](offline-closeout-report.md). Do not use earlier counts or mock-JEV metrics as current results.

# Cognee/JEV offline closeout evaluation

Date: 2026-10-28. Scope: isolated synthetic fixtures and the production local evidence selector. No Cognee, JEV, provider, or model service was contacted. No production settings or user directories were changed. The only transport used was the existing fake test transport; test files and runtime state used temporary directories. Live model authorization remains `cx/gpt-6-luna` only, and no model was needed.

## What was executed

- The chronological synthetic corpus (`m7-synthetic-1.0.0`, 56 labeled cases) ran through A memory-off, B legacy, C event/local selective, and E on-demand. Its evidence is [closeout-chronological-report.json](evidence/closeout-chronological-report.json), also generated at `target/cognee-closeout-chronological-report.json`.
- D was **not executed or eligible**: this driver does not wire a typed fake helper judgment. It is not a JEV control-flow test, and no C measurements are relabeled as D. JEV quality, accuracy, latency, and incremental value are unmeasured.
- The report separately represents packet-prepared context, harness persistence callback, Cognee persistence acknowledgment, tool-delivered text, delivered-to-model, downstream-observed use, and benefit. A prepared packet/callback is not evidence it entered a model request or was persisted remotely. Model delivery, downstream use, and benefit remain null. No `constraint_reused` claim is made.
- Failure-inclusive replay denominator: 56 cases per arm. C had 33 requests, 28 prepared/delivered results, 5 required-fact misses; B had 56 requests and 5 misses; A had 33 misses; E had 12 on-demand requests and 21 misses. These synthetic fixture counts do not estimate real-user utility. Failures/timeouts remain in the case/latency denominators. No future evidence was delivered (0 per arm).
- The bounded frozen selector fixture was labeled before execution and exercised the real production selector. The test measured retention for the positive/valid-at-boundary cases and rejection for active-context duplicate, wrong dataset scope, stale revision, superseded, and expired-at-boundary records. The self-contained-negative is explicitly not exercised by the selector because admission precedes selection. Fixture: [closeout-selector-cases.json](evidence/closeout-selector-cases.json). It is synthetic, not historical, held-out, or a user-effectiveness set.
- An isolated configuration dry-run used public runtime persistence methods to configure event/manual canary, switch to manual, disable Cognee/helper work, and roll back to legacy. The fake transport remained at zero requests. A pre-existing uncertain durable record survived disable and rollback and was not drained/replayed. This is not a real rollout, shadow deployment, or production rollback completion.

## Chronological replay observations (not effectiveness claims)

| Arm | Requests / 56 | Context packets prepared (callbacks) | Required misses / 56 | Future evidence delivered | Local measured p50 / p95 / max (µs) |
|---|---:|---:|---:|---:|---:|
| A — memory off | 0 | 0 | 33 | 0 | 0 / 0 / 3 |
| B — legacy | 56 | 51 | 5 | 0 | 1,856 / 2,083 / 1,002,105 |
| C — local selective | 33 | 28 | 5 | 0 | 1,365 / 2,366 / 1,002,446 |
| E — on-demand | 12 | 12 tool results | 21 | 0 | 384 / 1,476 / 1,520 |

Times above are local test-driver elapsed times, including fake-runtime overhead and failure delays. They are not remote service latency, end-to-end task time, or performance promises. The ~1s maxima include scripted timeout cases. There is no paired answer/action observation, no verified repeated-work reduction, and no beneficial-use denominator. `unknown` or unsupported quality/harm fields are not recast as zero harms.

## Verification

With inherited `CARGO_TARGET_DIR`, `CARGO_BUILD_JOBS=2`, and `RUSTFLAGS=-Dwarnings`:

- `cargo test -p ice-cli --lib --locked chronological_replay_regressions_temporal_boundary_zero_calls_and_denominators -- --nocapture` — passed (1 test).
- `cargo test -p ice-cli --lib --locked frozen_selector_fixture_exercises_production_provenance_filters -- --nocapture` — passed (1 test).
- `cargo test -p ice-cli --lib --locked isolated_mode_disable_and_rollback_preserve_uncertain_records -- --nocapture` — passed (1 test).
- `ICERS_MEMORY_REPLAY_REPORT=$PWD/target/cognee-closeout-chronological-report.json cargo test -p ice-cli --lib --locked chronological_offline_replay_report -- --ignored --nocapture` — passed (1 ignored opt-in test executed).

Evidence SHA-256:

- `closeout-chronological-report.json`: `309b137732d187b3a26f8ca303a2e1e4eb1e0304c3a207bc386e791de3267ca8`
- `closeout-selector-cases.json`: `5ddbc92473be29cb9175de4cb8485aa14f703b426847080ded4e968c218260c9`

## Remaining authorization and rollout gates

- **M5.4 remains incomplete.** Mocks/control-flow and deterministic filtering cannot establish JEV effectiveness. A future evaluation needs separate authorization for JEV/provider access, typed fake integration tests for control-flow correctness, and appropriately frozen/held-out cases to establish incremental value beyond C. No fake judge may be described as evidence of judge quality.
- **M7.3 remains incomplete.** No real target-repository local shadow/canary or operational rollback was started. Enabling event read, automatic writes, or JEV requires owner authorization and the plan’s operational/effectiveness gates. This test establishes only an isolated dry-run using temporary state and fake transport. No production settings, datasets, auth, or user memory were touched.
- The original chronological corpus does not provide selector metadata for wrong-scope, expiry, active-context novelty, or supersession. Those harms are not measured by that replay. The separate selector fixture exercises explicitly supplied metadata fields through the actual selector but does not turn those synthetic outcomes into historical prevalence or population safety estimates.
- The report does not establish task correctness, latency benefit, searchable Cognee persistence, historical memory coverage, real user benefit, or rollout readiness.
