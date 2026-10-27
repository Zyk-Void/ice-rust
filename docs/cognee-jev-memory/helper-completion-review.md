# Helper completion review

## Implemented in this worktree

- Helper scheduling exits before candidate-directory access when Cognee is offline/disabled, writes are not selective, shutdown has started, or no helper is bound. Candidate promotion requires both `durable == Yes` and `applicable == Yes`; uncertain verdicts do not write.
- Helper activation is explicit and defaults off. Persisted `helperEnabled`, `helperProvider`, and `helperModel` fields are supported, along with `ICERS_COGNEE_HELPER_ENABLED`, `ICERS_COGNEE_HELPER_PROVIDER`, and `ICERS_COGNEE_HELPER_MODEL`. Existing configs without the fields remain helper-off and unconfigured; provider alone never activates or chooses a model.
- `/cognee helper provider <id>|model <id>|on|off|status` controls and reports effective noncredential values. `/settings` exposes helper enable/provider/model rows; provider/model entry is available through the command. No credential value is displayed or persisted in these rows.
- Model IDs are validated as explicit nonempty identifiers (slashes supported) and used unchanged in the TypeSafe request. There is no production JEV/chat fallback. Unsupported decision protocol and provider binding errors remain nonfatal, safely classified helper status errors.
- Binding is keyed to a helper-specific signature: Cognee enabled, write policy, helper opt-in, provider, and model. Unrelated settings do not reset the helper admission gate or decision client. Helper revisions cancel old tasks; helper-off/manual/config transitions prevent canceled verdicts from writing candidate state.
- Admission history is retained across helper/config and dataset changes. Flight IDs prevent an old guard from clearing a newer in-flight admission. Canceled flights are checked immediately after await, before any candidate mutation. Accepted manual outbox behavior is unchanged.
- Effective helper status applies environment overrides and names overridden fields without exposing secrets. No per-prompt credential filesystem reads were added; existing explicit reload behavior remains the boundary.

## Verification

- `CARGO_BUILD_JOBS=4 cargo test -p ice-cli --lib --locked cognee:: --quiet`: **174 passed, 0 failed, 0 ignored** (1,253 filtered out). Tests use temporary directories/faux transports/fake decision responses; no network/model/Cognee request or real user config access occurred.
- Added/extended coverage includes persisted/env roundtrip, legacy-safe defaults, invalid model identifiers, exact request model, disabled/no-provider no-client construction, unsupported protocols, gate stale-flight ownership, and canceled old verdicts leaving candidate state pending.
- Formatting was applied only to owned `config.rs`, `helper.rs`, `helper_gate.rs`, `command.rs`, and `runtime.rs` using `rustfmt --config skip_children=true`.

## Integration needs / remaining gates

- Robust process/provider-shared rolling budgets, bounded/coalesced event work, and deterministic isolated shared-gate injection remain next phase.
- Comprehensive source/revision/scope/expiry revalidation, source excerpt tightening, and stable observation request ownership remain next phase. Existing per-session candidate and generation checks remain in place.
- JEV quality and latency remain **UNVERIFIED**; no effectiveness claim is made and no default activation is enabled.

## Worktree note

`crates/ice-cli/src/cognee/coordinator.rs` had a pre-existing rustfmt-only working-tree diff when this iteration began. It was preserved and is intentionally excluded from this implementation commit.
