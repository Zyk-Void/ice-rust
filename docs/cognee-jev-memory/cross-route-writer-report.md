# Cross-route writer safety: native admission versus generic MCP

- Date: 2026-10-28
- Worktree: `/home/mewtwo/Zykairotis/ice-rust/.worktrees/cognee-remaining-acceptance`, base revision `87ea8968bda22fd3977b827b6109ba10b0153526`
- Scope: can content that native Cognee candidate admission would reject reach a permanent store through a generic MCP route? Offline only: fake MCP server, temp-dir config, faux provider. No live service, launcher, `~/.icers` state, credential or dataset was used. The active `~/.icers/agent/mcp.json` was read for its launch shape only (no `env` or header values) and was not changed.
- Result: **yes, by default.** Every host MCP route delivers model-authored arguments to the server unchecked unless the server's `approveTools` requires approval. A named list does not cover forwarding tools. `approveTools: true` closes every route and fails closed without an approval UI. No host source change was made; M4.2 stays **PARTIAL**.

## Why the boundaries are separate

Native admission (`cognee_propose_memory` → `validation.rs::accept_validated_candidate` → `source_validation.rs::validate_source`) checks a host-built candidate: source span, role, sensitivity, session, branch, revision and worktree scope, and expiry. A generic MCP call has none of that. Its arguments are whatever the model wrote, and `ToolCallInfo` (model, thinking level, session ID) is not provenance. The host also has no trustworthy way to know which remote tool writes memory. Tool names, server names, descriptions and MCP annotations are all claims made by the server. So native admission cannot be extended to MCP calls. The only host-owned control is per-call user approval, which has the same trust level as an explicit `/cognee remember`, a write that also skips candidate admission by design.

## Routes traced

| Route | Path | Gate before the server sees the call |
|---|---|---|
| Direct tool | agent loop `execute_prepared_tool_call` → `before_tool_call` = `mcp::approval_hook` (wired in `session.rs`) → `McpTool::execute` → `tools/call` | `approvals` set, filled from `approveTools` per registered tool |
| `mcp` gateway | `proxy::call` → `manager.live_tool` approval flag → `approve` → `McpTool::execute` | same per-tool flag |
| `mcpScript` | `script::dispatch` `"call"` → `proxy::call` | same as gateway |
| Remote forwarder (`call_tool` in the installed `--tool-mode minimal` surface) | any of the above, with the real target in the arguments | gated only under its own name (`call_tool`), never under the target's name |

`McpTool::execute` itself has no policy check. Approval is the only gate, and `Approval::None` is the default (`mcp.rs` `ServerConfig.approve_tools`).

## Regression

`crates/ice-cli/src/mcp/proxy_tests.rs::generic_mcp_writes_reach_the_server_on_every_route_unless_all_need_approval`. A faux provider drives a real `AgentHarness` with `approval_hook` and an unattached `UiDialogMailbox`, so the run is headless. The fake server (`tests/mcp_test_server.rs`, new `ICERS_MCP_TEST_WRITES` mode) records every `tools/call` it receives into a temp file that stands in for the remote store. Each route calls `echo` (the writer) and `other` (a stand-in for a forwarder such as `call_tool`).

| `approveTools` | Calls that reached the fake store |
|---|---|
| absent / `false` (default; the active `cognee` entry has no `approveTools`) | all six: direct, gateway and script × writer and forwarder |
| `["echo"]` (named writer only) | the three forwarder calls |
| `true` | none |

Focused run: `cargo test -p ice-cli --locked --lib -- mcp::proxy::tests` passed, 8/8, after `cargo test -p ice-cli --test mcp_test_server --no-run` rebuilt the fixture binary. Full gate results for this source are in [Verification](#verification).

## Decision: configuration, not a new host guard

A host guard would need to know which server is the memory backend. Today the only safe way to know that is a user-written binding, and `approveTools: true` already is one. It applies to all three routes, survives forwarding, and fails closed when headless. Adding a Cognee-specific key would duplicate it. Guessing from the server name `cognee` or the tool name `remember` is the unsound shortcut this task rules out. Explicit native remember and improve are unchanged.

**Required owner action** (not done here, because it changes a production setting): add `"approveTools": true` to the `cognee` entry in `~/.icers/agent/mcp.json`. Each `remember`, `forget`, `recall` and `call_tool` call then needs an interactive Allow. If prompting on reads is too costly, use `["remember","forget","call_tool"]`. That gates the forwarder as a whole, so `call_tool` → `recall` also prompts. Never list writer names without the forwarder.

## Limits that remain

- The approval prompt shows only `Allow MCP tool <name>?`. The user judges the arguments from the transcript's tool-call rendering (the gateway's `args`, or the script code), not from the dialog.
- An approved `remember` still runs the server's `self_improvement=True` default. That improvement happens inside the server and is outside host control.
- Approval does not prove what a third-party server writes. Servers other than `cognee` may have their own writers, and the deployed `cognee-mcp 0.5.7` / bundled `cognee 1.6.4` against API `1.4.0` remains unverified (M0.2).
- The live configuration still has the default `None` until the owner applies the change above.

## Verification

The full gates ran on this exact source (base `87ea896` plus the test and fixture change). Logs: `target/cognee-remaining-validation/cross-route-*.log`, summary in `cross-route-meta.log`.

| Gate | Exit |
|---|---|
| `cargo fmt --all -- --check` | 0 |
| `RUSTFLAGS=-D warnings cargo check --workspace --all-targets --locked` | 0 |
| `cargo test --workspace --locked --no-fail-fast` (default parallel, `-D warnings`) | 0: 93 summaries, 3,445 passed, 0 failed, 9 ignored |
| `sh scripts/sync-embedded-docs.sh && git diff --exit-code -- crates/ice-cli/embedded-docs` | 0 |
| `python3 scripts/test_prebuilt_release.py` | 0 |
| `git diff --check` | 0 |

The extra pass over the earlier 3,444 is the new regression. The run used `CARGO_TARGET_DIR=target/cognee-remaining-validation`, `CARGO_BUILD_JOBS=2` and a fresh temporary `ICERS_CODING_AGENT_DIR`.
