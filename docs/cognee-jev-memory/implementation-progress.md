# Cognee + JEV implementation progress

Baseline: `09a728cdc5b98e270b8f01e38d2575ca047d45ff` on `feat/cognee-helper`.

## Baseline verification

- `cargo fmt --all -- --check`: passed.
- `cargo test -p ice-cli --locked cognee --lib`: passed, 76 tests; 0 failed; 1.25 s test execution (cold build 1m 17s). These are not end-to-end latency measurements.
- Historical measured recall and evaluation limitations are recorded in the existing observability audit and evaluation protocol. No live model or remote probe was run.
- The historical evidence JSON and some reports contain private session excerpts or descriptions. They remain untracked locally and must not be committed or published without a privacy-safe version. Original files have not been altered.

## Task ledger

| Tasks | Status | Evidence / blockers |
|---|---|---|
| M0.1, M0.2 | Not started | Truthful telemetry and adapter contracts; both required for Stage 0. |
| M1.1, M1.2 | Not started | Depends on M0.1; legacy read-through also depends on M3.1. |
| M2.1, M2.2, M2.3 | Not started | Selective recall; preserve legacy defaults. |
| M3.1, M3.2, M3.3 | Not started | Evidence scope and context coverage. |
| M4.1, M4.2, M4.3, M4.4 | Not started | Selective persistence and durability. |
| M5.1, M5.2, M5.3, M5.4 | Not started | Optional JEV; no live JEV evaluation under the Luna-only restriction. |
| M6.1, M6.2, M6.3, M6.4 | Not started | Capability discovery and tool error semantics. |
| M7.1, M7.2, M7.3 | Not started | Evaluation, rollout and release gates. |

No milestone is complete. No performance improvement or release gate is claimed.
