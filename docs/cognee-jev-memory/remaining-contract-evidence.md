# M0.2 native and MCP contract evidence

- Date: 2026-10-28
- Scope: offline native-client contract characterization in this worktree; no deployed service or live MCP server was probed.
- Result: local implementation contracts are captured by synthetic transport regressions. Deployed native wrapper compatibility, live advertised MCP writer methods/policies, inference-free backend metadata, and a job-status contract remain unknown. M0.2 is therefore still **PARTIAL**, not complete.

## Changed files

- `crates/ice-cli/src/cognee/client.rs` — includes the isolated native contract test module in the existing client test harness; no production request behavior changed.
- `crates/ice-cli/src/cognee/contract_tests.rs` — adds fake-transport regression cases for request shape, provenance, terminal HTTP errors, and accepted-but-unverified writes.
- `docs/cognee-jev-memory/remaining-contract-evidence.md` — this evidence record.

No runtime/doctor wiring or aggregate closeout document was changed. No deployed contract was available to justify changing production behavior.

## Locally established native request behavior

These shapes are asserted against the Rust client implementation and are not claims about a deployed wrapper version:

| Operation | Native client request | Evidence |
|---|---|---|
| Recall | `POST /api/v1/recall`; JSON `query`, `topK`, `onlyContext: true`, `scope`, optional `sessionId`, optional `datasets: [dataset]` | `native_contract_inventory_matches_exact_recall_and_write_shapes` |
| Remember | `POST /api/v1/remember`; multipart `datasetName`, `node_set`, `run_in_background: true`, optional `session_id`, and `data` as `ice-cognee.txt` | Same test |
| Improve | `POST /api/v1/improve`; JSON `datasetName`, optional `sessionIds`, and `runInBackground: true` | Same test |

An HTTP-success response may contain a bounded opaque identifier under `job_id`, `jobId`, `task_id`, or `taskId`. No status polling endpoint is established by local evidence, so a handle is not completion/searchability evidence. Unrecognized status or ID shapes yield no handle and do not create another request.

Recall normalization keeps provenance absent when it is not supplied. The test confirms that missing `source_id`, dataset, branch, revision, and expiry do not get manufactured, and malformed `results` envelopes fail as malformed input.

## Failure-path regression evidence

The newly added synthetic tests assert:

- Recall `404` and `422` preserve status classification and are terminal: exactly one request is sent to the known native recall path; no retry or alternative endpoint is guessed.
- An accepted write response with only a status string or an unknown `id` field remains accepted-but-unverified without a known job handle.
- Missing provenance remains missing; a malformed recall envelope returns `CogneeErrorKind::Malformed`.

Existing fake-transport client tests, run with the focused client filter, also cover response limits, malformed responses, deadline/timeout, cancellation, and delayed/malformed accepted-write acknowledgments. These are local control-flow tests only.

## MCP and deployed contract boundary

The inspected local MCP implementation is a generic proxy/catalog path. It can list tool schemas when an MCP server is connected, but the inspected safe local source/manifests do not provide an actual connected Cognee MCP server's advertised method inventory or writer schemas. Local native `cognee_search` and `cognee_propose_memory` are not evidence of external Cognee MCP tools; proposal is a local candidate/admission path. The capability doctor reports bounded last-observed inventory and `probed: false` rather than initiating a service call.

Still unknown and not inferred here:

- deployed native recall/remember/improve schema/version compatibility;
- live MCP tool names, argument schemas, writer methods, and explicit-versus-automatic policy;
- whether a service metadata operation is conclusively inference-free and has no data effects;
- whether a backend job-status endpoint exists and its semantics;
- remote completion, indexing, and searchability timing.

No endpoint or MCP method was guessed. No live request, inference, or service was started. `~/.ice`, active `~/.icers`, and TypeScript ICE sources were not used.

## Verification

All commands used a private Cargo target under ignored `target/cognee-remaining-validation/`, a fresh temporary `ICERS_CODING_AGENT_DIR`, `CARGO_BUILD_JOBS=2`, `RUSTFLAGS=-D warnings`, and `RUSTDOCFLAGS=-D warnings`. Full command output is retained in that directory:

| Gate | Result | Log |
|---|---|---|
| `cargo fmt --all -- --check` | PASS (exit 0) | `fmt.log` |
| `cargo test -p ice-cli --locked cognee::client::tests -- --nocapture` | PASS: 25 passed, 0 failed | `focused-cognee-client.log` |
| `cargo check -p ice-cli --all-targets --locked` | PASS (exit 0; no warnings/errors) | `check-ice-cli.log` |
| `cargo test -p ice-cli --locked` | PASS: 1,550 passed across recorded test targets, 6 ignored, 0 failed | `test-ice-cli.log` |
| Exit status record | format/check/test: `0 0 0` | `exit-statuses.txt` |

The crate test run's primary unit-test binary reported 1,528 passed and 6 ignored; the remaining passing test binaries reported 22 additional test executions (some run in multiple harnesses), for 1,550 passed test executions total. No full-workspace gate was run; crate-level verification is the scoped proof for this client/test change.

## Remaining acceptance gate

M0.2 remains partial until an explicitly authorized isolated service or inference-free metadata source establishes the deployed native and actual connected MCP contracts, including any job-status semantics. That evidence must be gathered without touching user state or invoking downstream inference. Until then, retain `probed: false`, do not claim deployed compatibility/searchability, and do not invent fallback endpoints. M4.2's external MCP writer boundary likewise remains unresolved pending a real method inventory.
