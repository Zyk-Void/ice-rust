# ACP integration audit: icers as an ACP agent for T3 Code

**Audit date:** 2026-10-30
**ice-rust checkout:** `main` at `314b7c22` (`feat(cognee): add the Cognee memory integration (#86)`); `base` at `ef71fbf6`
**Target client:** T3 Code, `github.com/pingdotgg/t3code` at `32ecd717` (2026-10-30)
**Reference implementation:** upstream `rpi-acp`, `github.com/pi-rust/rpi-package` at `9906990d` (2026-10-30, MIT)
**Supersedes:** the earlier draft of this file. Section 7 lists what that draft got wrong.

This is a source audit. Nothing was built, installed or run against T3 Code. Every claim is marked:

- **Verified (source):** read in the cited file at the commit above.
- **Verified (network):** checked against a public endpoint on the audit date.
- **Not verified:** inferred or untested. Treat as a hypothesis.

## 1. Summary

1. **icers has no ACP agent today.** There is no `--acp` flag, no ACP dependency and no `acp` module (verified, source). `--mode rpc` is the native JSONL protocol (`crates/ice-cli/src/rpc/protocol.rs:20-97`), not ACP JSON-RPC 2.0.
2. **T3 Code can launch any local ACP agent.** Its `acpRegistry` provider has a "Local command" source with an executable path and a list of literal arguments. It spawns that command over stdio with no shell (verified, source: `packages/provider-acp-registry/src/settings.ts`, `AcpRegistrySupport.ts:1812-1831`, `apps/web/src/components/settings/ProviderSettingsForm.tsx:187-225`). Once `icers --acp` exists, setting it up in T3 needs no changes to T3.
3. **A working Rust implementation already exists upstream.** `rpi-acp` (about 6,200 lines) is an ACP v1 server for the same native RPC contract that icers inherited. It ships as a plugin (`rpi --acp`) and as a standalone `rpi-acp-bridge` binary, with one `rpi --mode rpc` child per session (verified, source: `packages/rpi-acp/README.md`, `Cargo.toml`). The website's `ice-acp` entry is this package with the rebrand applied (section 7).
4. **Two things T3 needs that upstream `rpi-acp` does not do:**
   - **Per-session stdio MCP servers.** When T3 runs a session with its MCP server enabled, it passes its own `t3-code` stdio MCP server in `session/new.mcpServers`. Upstream only checks that the field is an array and then drops it.
   - **Tool-call `locations`.** T3's auto-approval policy denies edits that have no `locations`.

   Both are covered in section 3.
5. **icers has no general tool permission gate.** Upstream relies on a separate `rpi-permission-gate` plugin. In icers, `--approve` means *project trust*, not tool approval (`args.rs:513-514`). The only built-in approval hook covers MCP tools that are explicitly configured to need approval (`mcp.rs:1480-1505`). An ACP release that can write or run commands needs a gate first (section 4).
6. **T3's Pi provider is not a shortcut.** It drives `pi --mode rpc` directly. But it injects a TypeScript extension that imports `@earendil-works/pi-coding-agent`, requires Pi ≥ 0.80.5, and uses RPC commands icers does not implement (section 5). ACP is the right route.

**Recommendation:** add an in-tree `icers --acp` that ports `rpi-acp`'s session and translation logic to the icers host, built in this order: the T3 requirements in section 3, a built-in permission gate (section 4), then per-session MCP. The decision is spelled out in section 6.

## 2. What T3 Code expects from an ACP agent

All of the following is verified from source at `32ecd717`, unless noted.

### 2.1 Launch and handshake

| Behaviour | Evidence | What `icers --acp` must do |
| --- | --- | --- |
| Local agents run as `commandPath` + `commandArgs` with `shell: false`, in the thread's cwd, with the instance's environment merged in | `AcpRegistrySupport.ts:1812-1831` | Accept `--acp` as the whole argument list. Keep stdout strictly JSON-RPC: no banners, no `ready` marker, no logs. |
| `initialize` sends a merged v1/v2 request. A response containing `info` is treated as v2; anything else as v1 | `effect-acp/src/client.ts:684-700, 1081-1090` | Reply with a v1 `InitializeResponse` and `protocolVersion: 1`. Do not echo `2`. |
| Client capabilities: `fs.readTextFile`/`writeTextFile` and `terminal` only when the flavor provides them, plus elicitation forms and `session.notices` | `provider-acp/src/server/adapter.ts:1996-2003`, `AcpSessionRuntime.ts:1770-1777` | Do not depend on client fs/terminal. icers' own tools stay authoritative. |
| A probe initializes and lists models, commands and config within 60 s | `provider-acp-registry/src/server/probe.ts` (`PROBE_TIMEOUT_SECONDS = 60`) | Keep `initialize` and `session/new` fast. Do not load providers in the parent process. |
| Auth: uses `authMethods`/`authenticate` when advertised | `settings.ts` `authMethodId`; `auth.ts` | Advertise `authMethods: []`. icers already has `~/.icers/agent/auth.json`. Not verified: T3's behaviour with an empty list. Confirm in the smoke test. |

### 2.2 Methods T3 calls

| Method | How T3 uses it | Required for T3? |
| --- | --- | --- |
| `session/new` | Always. Passes `cwd` and `mcpServers` | **Yes** |
| `session/prompt` | Text, plus image blocks only if `promptCapabilities.image` | **Yes** |
| `session/cancel` | Interrupting a turn | **Yes** |
| `session/update` (agent→client) | Handles `agent_message_chunk`, `agent_thought_chunk`, `tool_call`, `tool_call_update`, `plan`, `available_commands_update`, `config_option_update`, `current_mode_update`, `usage_update`, `session_info_update` and others | **Yes** (message, thought and tool kinds) |
| `session/request_permission` (agent→client) | Answered from T3's runtime policy (section 2.3) | **Yes**, once a gate exists |
| `session/load` / `session/resume` | Used when reopening a thread. Fails with "ACP agent does not advertise session/load or session/resume support" if neither is advertised (`AcpSessionRuntime.ts:2162-2196`) | **Yes** for reopening threads after a restart |
| `session/set_config_option` | Model picker: the first option with `category: "model"` and `type: "select"` (`runtimeModel.ts:414-421`). Mode: `category: "mode"` (`:456`) | Recommended. Upstream already emits `model` and `thought_level` options |
| `session/set_model`, `session/set_mode` | Fallbacks for agents without config options | No |
| `session/list`, `session/fork`, `session/delete`, `session/close` | Only when advertised in `sessionCapabilities` | No |
| `session/elicitation` | Forms (questions) | No. Upstream maps `ask_user` to it |
| `fs/*`, `terminal/*` (agent→client) | Only if the agent calls them | No |

### 2.3 How T3 answers permission requests (this shapes the design)

T3 never shows a raw permission prompt in its auto modes. It decides from the request's `toolCall.kind` and `toolCall.locations` (`provider-acp/src/server/clientPolicy.ts:100-191`):

- `read`-like kinds: allowed under the read policy.
- Mode `approval-required`: **ask the user**.
- Sandbox `workspaceWrite`: allow `edit`/`delete`/`move` **only when every `location` resolves inside the cwd or a writable root**. A missing or empty `locations` list means **deny**. Every other kind is denied.
- `dangerFullAccess`: allow.
- The response is the option whose `kind` is `allow_once` (preferred), `allow_always` or `reject_once`. If no option matches, T3 answers `cancelled` (`adapter.ts:980-1003, 6013-6040`).

What follows for icers:

1. The agent should **always ask** before mutating or executing, and let T3's policy answer. T3 runs the policy; icers enforces it.
2. Every permission request must carry an accurate `kind` (`edit`, `execute`, `read`, `delete`, `move`, ...) and absolute `locations` for file tools. Otherwise T3's auto-accept-edits mode turns into deny-all for edits.
3. Every request must offer options with the standard kinds `allow_once`, `allow_always` and `reject_once`.

### 2.4 MCP servers from T3

When the thread has an MCP session, T3 adds a stdio server named `t3-code` to `session/new.mcpServers`. It launches T3's own executable as `acp-mcp-bridge`, with `T3_ACP_MCP_ENDPOINT` and `T3_ACP_MCP_AUTHORIZATION` in its environment (`provider-acp/src/server/adapter.ts:598-634`). A source comment notes that codex-acp and pi-acp drop injected HTTP servers, which is why T3 bridges through stdio. Separately, an `acp`-type MCP server is offered only if the agent advertises `mcpCapabilities.acp`.

What follows for icers:

- **Rejecting a non-empty `mcpServers` breaks T3 session creation.** The earlier plan proposed exactly that, and it must change.
- **Silently ignoring it (upstream's behaviour) works, but T3's own tools are then missing.** Not verified: which features depend on those tools. The orchestration instructions suggest thread and task coordination.
- **The target is to start each supplied stdio server in that session's child only.** Nothing is written to global or project settings. The secrets stay in that child's process environment and are never logged.

## 3. Gap list: what T3 needs and what icers has

| # | Requirement | icers today (verified, source) | Upstream `rpi-acp` (verified, source) | Work in icers |
| --- | --- | --- | --- | --- |
| G1 | `--acp` stdio entry, clean stdout | None. Generic `ownsStdio` plugin claim exists (`app.rs:241, 329-336`; `ice-extensions/src/registry.rs:371-377, 576-579`) | Plugin claims stdio, or standalone bridge | Early dispatch in `app.rs` before model resolution and TUI. No plugin, because `icers install` is blocked (AGENTS.md §2.6) |
| G2 | v1 `initialize` with honest capabilities | — | `protocol.rs:343-371`: `loadSession`, `resume`/`delete`/`close`, `embeddedContext`, `image: false` | Port it; rename `agentInfo` to `icers`; drop `rpi/durable` unless it is ported |
| G3 | One native child per session, cwd = `session/new.cwd` | `--mode rpc` + `--session-id` + `--session-dir` exist (`args.rs`) | `session.rs`, `rpi-extcore` process helpers | Port. Resolve the child binary from `current_exe()`, never PATH. Never pass `--acp` to children |
| G4 | Prompt turn ends only on settlement | Native `prompt` is acknowledged first; `agent_settled` comes later (`modes.rs:545-690`) | Waits for `agent_settled` and reconciles text against `message_end` snapshots | Port it with its tests |
| G5 | Streaming text, thinking and tools | Event projection in `remote/protocol.rs:158-263` | Implemented | Port; add `locations` (G7) |
| G6 | Cancel | Native `abort` (`modes.rs:697-708`) | Implemented | Port |
| G7 | Tool-call `kind` + absolute `locations` | Tool names and args are in events | Has `kind` (`session.rs:821-830`, where `bash` maps to `other`); **no `locations`** | **New.** Map `read`/`write`/`edit`/`ls`/`grep`/`find` path arguments to absolute paths under the cwd. Map `bash` to `execute` |
| G8 | Permission requests through `session/request_permission` | Headless dialog bridge exists: `ui_dialog_request`/`ui_dialog_response`/`ui_dialog_closed` (`modes.rs:385-390`). **No general gate** | Uses the `rpi-permission-gate` plugin; children get `--permission-gate`; `--bypass` skips it | **New built-in gate** (section 4) |
| G9 | `session/load` + `session/resume` | `--session-id` **creates a missing session** (`session.rs:160`) | Checks the session exists before spawning; unknown IDs are errors; replays history on load | Port; keep the existence check; store under `~/.icers/agent/acp/sessions` |
| G10 | Model and thinking config options | `get_available_models`, `set_model`, thinking-level controls (`rpc/protocol.rs:64-84`) | `category: "model"` and `"thought_level"` (`protocol.rs:1457-1467`) | Port. The change must apply to that session only, never to global settings |
| G11 | Per-session stdio `mcpServers` | `McpManager` starts headless from settings (`session.rs` ~968-998). **No per-process injection flag** | Validates the array, then ignores it | **New.** A child-only MCP config input (for example a temp file with mode 600 passed by a new flag). Must not touch `settings.json` |
| G12 | Images | Native RPC rejects `images` (`modes.rs:546-552`) | `image: false` | Defer; advertise `image: false` |
| G13 | Event-loss handling | Native RPC drops lagged broadcast events with `continue` (`modes.rs:195, 376, 775`) | Reconciles final text from `message_end` | Keep the reconciliation. Not verified: whether tool events can be lost under load. Add a stress test |

Not required for T3: client fs/terminal, `session/set_mode`, list/fork/delete, ACP v2, auth flows, the `rpi/durable` extension.

## 4. Security findings

1. **No tool-approval boundary exists for ACP (blocker for writes and commands).** `before_tool_call` is the right interception point (`ice-agent/src/agent_loop.rs:1039-1046`). `session.rs:784-817` chains it through extension hooks, command hooks and `mcp::approval_hook`, but nothing gates `bash`/`write`/`edit`. Proposal: port `rpi-permission-gate`'s policy as a built-in hook that ACP children enable through a flag or env (`ICERS_PERMISSION_GATE=1`). It asks through the existing `UiDialog` bridge, which the parent translates to `session/request_permission`.
   - The gate must fail closed: disconnect, timeout, malformed reply, `cancelled` and `reject_*` all block the call.
   - It must cover extension tools and MCP tools by default (ask whenever unsure), not a fixed list of tool names.
   - A `--bypass` equivalent, if added, must apply to that process only and never be persisted.
2. **`allow_always` must not persist outside the session.** T3 prefers `allow_once`, but a client may pick `allow_always`. Scope it to that ACP session in memory.
3. **Native acceptance is not completion.** Replying to `session/prompt` on the native `accepted` ack would report success while the run is still going, or after it has failed.
4. **Restoring a session can silently create a new one** (`session.rs:160`). Check that the session file exists and its cwd matches before spawning the child.
5. **Client inputs widen the trust boundary.**
   - `cwd` must be absolute, exist and be a directory. It is not a sandbox.
   - `mcpServers` launch arbitrary commands chosen by the client. That is acceptable because the client already controls the agent, but never persist them, and never log their environment (T3 puts a bearer token there).
   - Project trust: ACP children must not pass `--approve` implicitly. Decide whether to follow the user's stored trust, and document the choice.
6. **Stdout purity and process hygiene.** Any stray stdout write corrupts the ACP stream. Kill and reap children when the client disconnects; child stderr goes to the parent's stderr with secrets redacted.
7. **Do not install the published `ice-acp`/`rpi-acp` crate.** The crates.io name `ice-acp` is not ours (AGENTS.md §2.6). Upstream `rpi-acp` targets the `rpi` ABI, `~/.rpi` and `RPI_*`. Port the source instead.

## 5. Alternatives considered

| Option | Verdict | Why |
| --- | --- | --- |
| **A. In-tree `icers --acp`, porting `rpi-acp` logic** | **Recommended** | Same native RPC lineage, so most translation logic carries over unchanged, along with its tests. No plugin install, no ABI or name problems. One binary for T3 to point at |
| B. In-tree, written fresh on the official SDK (`agent-client-protocol` 3.6.0, MSRV 1.88, verified network) | Viable | Typed schema, less framing code. But the session and translation logic still has to be written. A reasonable later refactor once option A works; could use only `agent-client-protocol-schema` (1.14.0) for types |
| C. Port `rpi-acp` as a cdylib plugin | Rejected | Needs `icers install` or manual plugin placement, ABI matching and `rpi-extcore`. The plugin package-name question is still open |
| D. Upstream `pi-acp` (npm 0.0.34) with `PI_ACP_PI_COMMAND=icers` | Rejected, except as a throwaway experiment | Reads `~/.pi/agent` for sessions, settings and skills; TypeScript; untested against icers' RPC |
| E. T3's Pi provider with `binaryPath=icers` | Rejected | Needs `--version` ≥ 0.80.5 (icers is 0.6.1). Injects a TS extension importing `@earendil-works/pi-coding-agent` (`provider-pi/src/server/mcpExtensionSource.ts`). Calls `get_commands`, `get_entries`, `new_session`, `switch_session`, `fork`, `set_session_name`, `get_session_stats` and `extension_ui_response`, none of which icers implements (`rpc/protocol.rs:20-97`) |
| F. Direct in-process ACP on `AgentHarness` | Deferred | Needs a multi-session registry and hook routing. Revisit only if child startup latency matters |

## 6. Implementation order (stacked PRs, about 600 lines each excluding tests)

| PR | Deliverable | Done when |
| --- | --- | --- |
| 1 | `acp/` skeleton: `--acp` early dispatch, JSON-RPC framing, `initialize`, errors for unknown methods, stdout purity | Offline frame tests pass; TUI, `-p` and `--mode rpc` unchanged |
| 2 | Session bridge: `session/new` (accepts and **ignores** `mcpServers` with a stderr warning), child spawn, `session/prompt` up to settlement, streaming text and thinking, `session/cancel` | Faux-provider test: two sessions in two temp cwds, prompt, cancel, no cross-talk. **First T3 smoke test (read-only model use)** |
| 3 | Tool calls with `kind` + `locations`; built-in permission gate wired to `session/request_permission` | Denial, timeout and disconnect tests; T3 approval-required, auto-accept-edits and full-access modes behave as in section 2.3 |
| 4 | `session/load` / `resume` + model and thinking config options | Restart the ACP process and reopen a thread in T3; unknown ID and wrong cwd return errors |
| 5 | Per-session stdio `mcpServers` | Fixture MCP server reachable from the child; nothing written to settings; T3's `t3-code` tools appear |
| 6 | Docs (`docs/extensions/acp.md`, T3 and Zed setup) and CI fixtures for Linux and Windows | All §5 gates in AGENTS.md pass |

Every PR follows AGENTS.md: faux provider only, a temp `ICERS_CODING_AGENT_DIR`, no network in tests, and an MIT attribution header on files ported from `rpi-acp`.

### T3 setup once PR 2 lands (from T3 source, not yet tried)

Settings → Providers → add an ACP provider instance:

- **ACP source:** Local command
- **Executable override:** `/home/mewtwo/Zykairotis/ice-rust/target/release/icers`
- **Arguments:** `--acp`

T3 starts it in the thread's worktree cwd. icers reads `~/.icers/agent` as usual.

## 7. Corrections to the earlier draft

| Earlier claim | Correction |
| --- | --- |
| The advertised `ice-acp` package is "Unknown / distribution risk"; its repository returned 404 | `website/data/packages.json:128-141` is upstream's `rpi-acp` listing after the rebrand mapping was applied (`pi-rust/rpi-package` → `ice-rust/ice-package`). The real source exists at `github.com/pi-rust/rpi-package` (HEAD `9906990d`, verified network). The `ice-acp` crates.io name is still not ours. The entry's description is in Chinese; it should be translated (AGENTS.md "Language") in a separate change |
| No reference implementation was considered | `rpi-acp`, `rpi-extcore` and `rpi-permission-gate` cover most of the design (section 3) |
| Reject non-empty client `mcpServers` | That breaks T3, which always sends one when MCP is enabled (section 2.4) |
| SDK pinned to `=3.5.0` (in the plan) | Latest is 3.6.0 (2026-10-28), MSRV 1.88 (verified network). Optional under option A |
| `--approve`-style approval and existing dialogs as a possible gate | `--approve` is project trust (`args.rs:513-514`). No general tool gate exists (section 4) |
| "RPC drops events near `modes.rs:390`" | The lagged `continue` is at `modes.rs:195, 376, 775` |
| No target client | T3 Code's requirements are now the acceptance criteria (section 2) |

Still accurate from the earlier draft: no in-tree ACP; native RPC is not ACP; acceptance ≠ completion; `--session-id` creates missing sessions; images are rejected; the RPC test counts it reported (11 + 17 native RPC tests; this audit did not re-run them).

## 8. Verification performed and limits

- **Source (ice-rust `314b7c22`):** `git grep` for `acp`, `--acp` and `agent-client-protocol` in `crates/**` (only comments in `app.rs` and `ice-extensions`); read `args.rs` flags, `rpc/protocol.rs` commands, `modes.rs` dialog bridge, lag handling and image rejection, `session.rs` hook chain and session-id semantics, `mcp.rs` approval hook, and `website/data/packages.json`.
- **Source (T3 `32ecd717`, shallow clone, read only, nothing executed):** `packages/effect-acp`, `provider-acp`, `provider-acp-registry`, `provider-pi`, `apps/web` settings form.
- **Source (`rpi-package` `9906990d` and `pi-acp` `04d0a15`, shallow clones, read only):** `rpi-acp`, `rpi-extcore`, `rpi-permission-gate` READMEs and sources.
- **Network:** ACP registry `cdn.agentclientprotocol.com/registry/v1/latest/registry.json` (43 agents, `pi-acp` 0.0.34, no icers entry); crates.io `agent-client-protocol` and `agent-client-protocol-schema`; npm `pi-acp`.
- **Not performed:** building anything, running T3 Code, an ACP conformance run, a Zed test, tool-by-tool security review, or Windows behaviour. The T3 UI labels in section 6 come from source and have not been seen in the running app.
- **Open questions for the smoke test:**
  - How T3 handles an empty `authMethods`.
  - Whether T3 needs `session_info_update` or `usage_update` for its thread UI.
  - Which T3 features depend on the `t3-code` MCP tools.
