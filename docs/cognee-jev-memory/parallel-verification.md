# Parallel test reliability verification

- Date: 2026-10-28
- Worktree: `/home/mewtwo/Zykairotis/ice-rust/.worktrees/cognee-remaining-acceptance`
- Revision tested: `87ea8968bda22fd3977b827b6109ba10b0153526`
- Scope: reproduce the earlier Codex token refresh report, verify the auth serialization regression in isolation and in the full workspace, and identify known test-process environment interference. No production source or test code was changed.
- Result: the historical unexpected refresh request remains unexplained and was not reproduced by isolated or current workspace runs. The full parallel workspace run passed on this revision. A distinct set of prior parallel and serial logs demonstrates process-global environment mutation and cascading poisoned test mutexes in other runs, but is not enough to prove that this was the cause of the original second refresh report.

## Test behavior and synchronization

The auth test harness changes `ICERS_CODING_AGENT_DIR` while holding `config::test_support::env_lock()` for the full test fixture lifetime (`crates/ice-cli/src/codex_auth/tests.rs:16-43`). The cross-holder refresh test starts two independent `CodexToken` holders with the same expiring credential, holds the fake token-server response until the second holder starts, asserts both calls return the rotated credential, checks the persisted rotation, then waits 150 ms for an unexpected second request (`codex_auth/tests.rs:102-116,246-308`).

Production refresh obtains the cross-process auth-store transaction before rereading credentials, retains that transaction through refresh and persistence, and returns a newly rotated unexpired credential to a waiting holder instead of refreshing it again (`crates/ice-cli/src/codex_auth.rs:80-99,193-245`). This ownership and lock path is present in the tested revision; no production change was justified by the evidence.

## Verification results

Commands ran from this worktree with `CARGO_TARGET_DIR=target/cognee-remaining-validation`, `CARGO_BUILD_JOBS=2`, `RUSTFLAGS=-D warnings`, `RUSTDOCFLAGS=-D warnings`, and a new temporary `ICERS_CODING_AGENT_DIR` per full run or stress iteration.

| Gate | Result | Evidence |
|---|---|---|
| Earlier focused refresh stress | 100/100 repetitions passed | `target/cognee-remaining-validation/auth-refresh-stress.log` |
| Additional isolated auth stress | 75/75 passed: cross-holder refresh 25/25; unexpired credential 25/25; cancellation while waiting for store lock 25/25 | `target/cognee-remaining-validation/auth-parallel-recheck.log` |
| `cargo test --workspace --locked --no-fail-fast` (default parallel test execution) | Exit 0; 92 result summaries, 3,444 passed, 0 failed, 9 ignored; one nested fixture intentionally panics and is caught by its test | `target/cognee-remaining-validation/current-parallel-workspace.log` |
| `ice-cli --lib` with 32 test threads | Exit 0; 1,534 passed, 0 failed, 6 ignored | `target/cognee-remaining-validation/parallel-ice-cli-lib.log` |
| Earlier current workspace parallel attempt | Exit 101; 1,343 passed, 191 failed, 6 ignored in the `ice-cli` library binary. First recorded auth panic was `cancellation_while_waiting_for_the_store_lock_is_prompt` at `tests.rs:110`, followed by `TempConfig` env-mutex poison panics. | `target/cognee-remaining-validation/current-workspace-concurrent.log` |
| Earlier full workspace serial attempt | Exit 101; library test failures elsewhere (interactive TUI/session and update tests) and subsequent poison cascades; not evidence of a serially passing suite | `target/cognee-remaining-validation/current-workspace-tests-serial.log` |

The successful current default-parallel command was:

```sh
cfg_dir=$(mktemp -d)
CARGO_TARGET_DIR=target/cognee-remaining-validation \
CARGO_BUILD_JOBS=2 RUSTFLAGS='-D warnings' RUSTDOCFLAGS='-D warnings' \
ICERS_CODING_AGENT_DIR="$cfg_dir" \
cargo test --workspace --locked --no-fail-fast
```

The temporary directory was removed after the command. The saved log contains full output and exited 0. The 92 summaries include nested test processes; counts are executions, not unique test definitions.

## Failure analysis and limits

The earlier failed auth traces are retained in `current-workspace-concurrent.log`, `current-workspace-tests-serial.log`, and `jev-default-workspace-tests.log`. The observed request failure occurred in the cancellation-while-waiting test in one parallel run and the unexpired-token test in one serial run; the independent-holder rotation test failed at the same 150 ms no-request assertion in a different historical run. The parallel auth cascade then includes `PoisonError`s because the failed test panicked while owning `TempConfig`'s global environment guard. That makes the mutex poison secondary but does not identify who triggered the unexpected server request or establish a single cause shared by these runs. Current isolated stress and full-workspace success demonstrate non-reproduction, not a root-cause fix.

Separately, workspace test modules contain numerous process-global environment mutations; they do not all use the config test mutex, and some independently lock environment state. Examples include the `ICERS_OFFLINE` and config-directory mutations in `crates/ice-cli/src/updates.rs` and environment changes in config/provider/auth tests. The earlier parallel and serial failure logs contain failures outside Codex auth and poison cascades, while the latest full parallel run passed. This is a credible general test-isolation hazard, not proof it caused the original refresh request. No global test serialization or weakened no-request assertion was introduced.

## Conclusion and remaining risk

- The shared auth-store transaction and rotation reuse invariant remain intact.
- The focused refresh and cancellation assertions passed repeated isolated runs; the full default-parallel workspace suite also passed once on this exact revision.
- The historical second refresh is **not root-caused**. Continue to treat it as an intermittent, unreproduced report; do not claim it is fixed merely because these runs passed.
- A repeatable parallel stress run across the entire workspace and a captured original failure with sufficient request timeline remain useful follow-up if the issue recurs.
