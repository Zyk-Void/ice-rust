# ICE-Rust ACP (Agent Client Protocol) integration plan

Status: planning only; no ACP implementation has been merged or tested here.
Audit baseline: 2026-10-28, `main` at `dd0e2405`. Owner: `crates/ice-cli`.
Objective: make `icers --acp` usable as a standard ACP **agent** launched over stdio by Zed and other ACP clients, while keeping existing TUI, native JSONL RPC, tools, memory and session formats intact.

## Decision and scope

**Recommended implementation:** build a small in-tree ACP v1 server in `crates/ice-cli/src/acp/`, entered early by `icers --acp`. For each ACP session, launch exactly one child `icers --mode rpc --session-id <UUID> --session-dir <ACP_DIR>` with its process working directory set to the absolute `session/new.cwd`. Translate ACP JSON-RPC messages to/from the existing native JSONL RPC contract. Use the official typed Rust SDK (`agent-client-protocol = { version = "=3.5.0", features = ["stdio"] }`), subject to a compile/lockfile verification before committing. Its upstream Rust workspace sets MSRV 1.88, matching ours. Do not change `ice-agent` or provider APIs merely to add transport.

Why: existing headless RPC already exposes prompt acceptance/settlement, cancellation, history, model selection, and asynchronous UI dialogs. A bridge avoids a second agent loop and a big unsafe reimplementation. Keep ACP framing separate from `crates/ice-cli/src/rpc/` because **native RPC is not ACP JSON-RPC 2.0**.

**Explicit alternatives:**
- Standalone `icers-acp-bridge` binary: fastest isolated initial spike, same design; less ergonomic client configuration. A standalone adapter and `icers --acp` should share one implementation, not drift.
- Rebrand upstream `rpi-acp` dynamic plugin: reusable implementation but requires porting both `rpi-acp` and parts of `rpi-extcore`, matching the ICE plugin ABI, replacing every RPI binary/path/environment assumption, and verifying permission gate integration. This is **not** a drop-in. Do not install the upstream crate/plugin into icers.
- Direct in-process ACP -> `AgentHarness`: potentially fewer processes/less startup latency later, but requires isolated per-cwd sessions, multi-session registry, hook/dialog routing, and a new lifecycle surface. Defer until the child-process adapter is proven.

Initial release: ACP **stable v1** stdio; support initialize, session/new, session/prompt, session/cancel, streaming assistant/thinking/tools, safe permission policy, and successful cleanup. Then durable load/resume, model config and client-supplied MCP servers. Do not advertise unsupported capabilities. Defer the draft ACP v2 until a separate compatibility milestone.

## Current codebase evidence (verified)

- `crates/ice-cli/src/app.rs:91-98` has a precedent for early pure-client dispatch (`--connect`). `app.rs:239-247,327-375` deliberately protects stdin/stdout from competing consumers when a plugin claims stdio. `app.rs:395-411,477-557` shows headless lifecycle and `StdioLifecycle`.
- `crates/ice-cli/src/args.rs:92-99,123-146,347-378,600-622` models text/json/rpc only; no native ACP mode today. `args.rs:433-459` supports `--session-id`/`--session-dir`.
- `crates/ice-cli/src/rpc/protocol.rs:1-162`: native JSONL frames are `{"type":"prompt","message":"..."}` and responses `{"type":"response","success":true,"data":...}`; command set includes get_state, history, model/tool controls, abort. `rpc/framing.rs:1-45` has a 1-MiB frame limit.
- `crates/ice-cli/src/modes.rs:298-434`: native RPC is async, serializes its output and bridges extension dialogs. `modes.rs:545-690`: `message` prompt acknowledges acceptance, completion comes later as `agent_settled`. `modes.rs:697-730` exposes abort, history and model controls. `modes.rs:545-555` rejects image attachments.
- `crates/ice-cli/src/remote/protocol.rs:158-263` provides the stable event projection for assistant deltas, thinking, tools; `rpc/events.rs:1-12` reuses it.
- `crates/ice-cli/src/session.rs:147-228` implements native JSONL sessions, exact `--session-id` and default per-project `.icers/sessions`; `session.rs:2364-2376` **creates a new session if a specified exact ID is absent**. ACP load must independently verify identity and on-disk existence first, to avoid fake restoration.
- `crates/ice-plugin-sdk/src/lib.rs:1065,1358,1553-1576` defines the ICE-specific `StdioLifecycle` action and `ice_plugin_register` symbol. `crates/ice-extensions/src/registry.rs:370-374` supports a plugin stdio claim, but we do not require a plugin for the recommended route.
- `crates/ice-cli/src/session_driver.rs:1-95` is a transport abstraction for *TUI* and does not by itself implement an ACP agent endpoint.
- `AGENTS.md:32-59,79-89,128-175,195-246`: ICE-only paths/env, no upstream edits/publishing/install, feature-branch/PR policy, test requirements and MSRV.

**Audit result:** no in-tree ACP module or ACP server found on `main`. ACP comments and an inherited upstream changelog mention do not constitute a working ACP server.

## Feature-preservation addendum: the whole ICE runtime, not a stripped ACP agent

This plan must be evaluated against **two code states**: §main@dd0e2405§ and the active feature worktree §.worktrees/extra-fx-bug-fix§ on §feat/cognee-helper@d912a00e§. The latter is **not merged into main** as of 2026-10-28. Do not build ACP from an obsolete branch snapshot, silently drop the pending work, or describe a pure helper API as a live feature. At integration time re-audit the actual merge target and run its focused fixtures.

The key architecture contract: the ACP layer must invoke the **same current §session::build_prepared§** path (via one ordinary §icers --mode rpc§ child per session), with the same agent directory, project cwd, trusted resources, settings, credentials, plugins, provider model catalog and active-tool policy. Do **not** construct a second restricted §AgentHarness§ for ACP. An ACP client needs not see every internal subsystem as a standalone ACP method for the **agent** to use that subsystem; only interactive management/UI and client-supplied capabilities require additional transport work.

| Feature | What code actually does | ACP impact / acceptance test |
| --- | --- | --- |
| Core tools, extensions, hooks, providers, settings | §crates/ice-cli/src/session.rs:416-444, 775-886, 2190-2268§ builds the same tools/hooks/provider bindings for native RPC as for TUI. | Reuse the entire pipeline. Compare ACP-child §get_available_tools§ vs a comparable native RPC child under identical temp config. Prove plugin tool invocation and error propagation. Security policy may deny dangerous calls; **never delete tools silently**. |
| Background shells/watches | §session.rs:431-434, 2258-2261§ adds shell tool controls when bash is enabled; §background_shells.rs:71-160§ constructs and connects registries. | Agent-initiated commands can work in ACP; terminal §Ctrl+B§ is not a protocol feature. Verify child shutdown, cancellation and job cleanup without accidentally killing other sessions. |
| MCP, resources and scripts | §session.rs:968-998§ starts §McpManager§ even headlessly. §mcp/manager.rs:887-933§ adds the §mcp§ proxy and conditional §mcpScript§ tool when a server is configured. §session.rs:980-992§ restricts elicitation/sampling setup to interactive TUI at present. | Configured MCP **tools** can be usable by ACP agents. ACP-client-provided §mcpServers§, MCP sampling and elicitation are **not** automatically enabled: add safe per-session provisioning, request routing and honest capability negotiation. Test server failure, late registration and permission prompt. |
| Skills and prompt templates | §session.rs:517-584§ discovers skills/templates for every run; §interactive_tui.rs:1484-1504§ registers template/extension/MCP slash commands only in TUI. | Preserve skill listing/context in the agent. Do not claim that sending literal §/skill§, §/mcp§ or §/template§ to ACP automatically executes the TUI command. Add explicit compatible command listing/invocation or a documented non-interactive expansion path; test against the same resolved resources. |
| Cognee memory | §builtin_extensions.rs:99-207§ binds Cognee to harness events, search tool, lifecycle and §HarnessExtensions§ for any mode. Notices are TUI-only (§builtin_extensions.rs:1-6, 50-64§). | Auto-recall/capture, memory tool and compaction integration can run headlessly **if configured**. Preserve lifecycle and failure semantics. Provide optional ACP-safe status/diagnostic retrieval; do not promise a TUI footer or notice stream. Verify memory context reaches faux-model input and failures degrade gracefully. |
| Blackhole compaction | §builtin_extensions.rs:196-207§ registers Blackhole with the harness; §blackhole/engine.rs:88-115§ makes it opt-in. §modes.rs:545-689§ already supports native §compact§. §interactive_tui/commands.rs:777-827§ owns §/blackhole§ and §/cognee§ UI commands. | Automatic compaction remains internal. Expose a deliberate capability-gated manual compact operation if desired; preserve Blackhole's manual marker. Do not treat the TUI command itself as a model prompt. |
| Subagents/child action bridge | §extensions_actions.rs:82-84, 407-420§ provides the native plugin §ChildAgent§ action; §child_agents.rs:131-205§ manages start/poll/cancel. Availability of a **model-invocable agent tool** depends on installed and loaded extensions. | Do not assert built-in ACP subagent support simply because the host action exists. Verify installed plugin tool discovery, actual child invocation, streamed parent tool status, cancellation and worker ownership; add richer ACP child metadata only as an optional extension. |
| Side questions and TUI-specific controls | §side_questions.rs:1-15, 26-98§ owns in-memory conversations, exposed through runtime actions in §extensions_actions.rs:352-379§; UI rendering belongs to TUI. | Preserve the runtime action, but ACP has no terminal side-question pane. Build an ACP UI mapping where supported or advertise that interactive side-question UI is unavailable. Never swallow pending approval or ask dialogs. |
| Observability | §app.rs:423-434§ starts the existing observer around all run modes; §observe/*§ persists/serves activity. | Child processes must retain observability, correlate native run ID and ACP session ID, and avoid duplicate/misleading activity on restore. Check concurrent SQLite writer safety and secret redaction. |
| Provider/model/thinking/session controls | §rpc/protocol.rs:20-126§ already offers §get_state§, §set_model§, §set_thinking_level§, history, queue, compact, navigate and tools. | Bridge relevant controls to standard ACP config/session methods where available; retain the rest as private namespaced methods or deliberately unsupported. Never mutate global model/settings merely because one ACP client changed its session. |
| Decision/JEV endpoints | §decisions.rs:1-80§ provides typed §decision_client§ bindings. **This is not itself an agent tool**. | Do not advertise a JEV model/tool merely because typed clients exist. If Cognee's optional decision helper is enabled, verify its actual runtime integration separately. No ACP-specific JEV transport should be invented. |
| Unmerged Cognee/JEV helper and memory modes | §feat/cognee-helper@d912a00e§ has §cognee/helper.rs§, §cognee/propose.rs§ and §cognee/runtime.rs:1706-1749§ for optional helper, selective proposal tool and memory modes. §docs/cognee-jev-memory/implementation-progress.md:27-38§ explicitly says helper-to-coordinator wiring and live JEV validation are incomplete. | AFTER those commits merge, verify ACP child inherits new §cognee_propose_memory§ and all configured memory behavior. Do not imply unfinished JEV ranking or selective-memory quality has been validated. |
| Unmerged capability catalog/suggestions/skill coverage | Same worktree: §capability_catalog.rs:68-204§; §session.rs:1225-1237§ supplies a snapshot API; §capability_suggest.rs:93§ and §skill_coverage.rs:55§ are pure helpers. Current usage search finds only tests for suggestion/coverage, not runtime consumers. | Preserve these APIs on merge. To offer proactive ACP capability suggestions, **first** wire the feature into the agent/session dispatch independent of TUI and prove suppression/skill state; then optionally send structured ACP notifications. Do not claim suggestions run simply because their modules compile. |
| TUI-only UX, extension commands and provider refresh | §interactive_tui/commands.rs:751-827§ has UI settings/Cognee/Blackhole; §interactive_tui.rs:1484-1504§ registers slash commands; unmerged §provider_model_refresh.rs§ is called from the TUI. | Standard ACP agents get tools, not the TUI. Supply an optional typed management surface (or explain omissions); never forward slash commands as normal model text and claim they executed. |

**Definition of parity:** runtime behavior is retained when §icers --acp§ launches a normal configured native child; separate ACP UX parity is **not** implied. Explicitly publish a per-feature matrix with **runtime reused**, **ACP-native interface**, **extension-only**, **TUI-only**, **not yet integrated**, or **disabled by policy**. For each feature, record evidence from the final merge target and a focused offline test. Avoid a global "all features supported" claim until that matrix passes.

## ACP feature-parity verification track (cross-cutting, required)

These tests extend Phases 0–6; they are distinct integration checks and must not be treated as passed by protocol handshake tests alone.

- [ ] **Runtime parity:** use matching temporary §ICERS_CODING_AGENT_DIR§, cwd, files, model catalog and extensions for native RPC and ACP-backed child; compare §get_available_tools§, provider, active tools, system-prompt resources and session identity. Any differences must be intentional, documented and testable (permission filters are separate from tool discovery).
  - Target: §acp/session.rs§, §session.rs§, native §modes.rs§ test fixtures.
  - Verify: offline faux-provider dual-launch test; both return equivalent tool catalogs and initialization errors.
- [ ] **MCP + mcpScript:** configure an isolated fixture MCP server and ensure its proxy/script tools register on the ACP child. Verify direct agent tool use, delayed startup, error mapping, denied approval, and that client-supplied ACP §mcpServers§ either provision safely or are explicitly rejected.
  - Target: §acp/process.rs§, §acp/session.rs§ and §mcp/manager.rs§ tests.
  - Verify: existing MCP fixture server with deterministic RPC interaction.
- [ ] **Memory/compaction:** verify Cognee automatic recall and capture and Blackhole behavior with a fake memory transport and configured temp agent dir; ensure non-TUI notices cannot be lost silently if they represent errors; test correct flush/drain on child shutdown.
  - Target: §acp/session.rs§, §builtin_extensions.rs§, §cognee/runtime.rs§, §blackhole/engine.rs§.
  - Verify: faux-model input/context inspection, no live memory server or user directory access.
- [ ] **Extensions + child agents:** configure a local synthetic plugin tool that invokes the native child-agent action, then verify ACP streams its parent-level lifecycle, handles cancellation/ownership, and does not leak child session details between ACP sessions.
  - Target: §acp/translate.rs§, §extensions_actions.rs§, §child_agents.rs§, plugin test fixture.
  - Verify: isolated plugin and faux-provider tests; no user-installed plugins required.
- [ ] **UI command boundary:** list TUI-only features (§/cognee§, §/blackhole§, §/mcp§, §/settings§, templates, extension commands, side-question panes, model refresh, Ctrl+B§). Implement only deliberate safe ACP equivalents or label unsupported. Verify a literal command does not secretly invoke privileged actions.
  - Target: §acp/protocol.rs§, §acp/commands.rs§ if needed, §interactive_tui/commands.rs§ read-only reference.
  - Verify: supported method and unsupported-command fixtures.
- [ ] **Pending branch features:** at ACP implementation/merge time inspect §feat/cognee-helper§ and any subsequent merged work; test optional §cognee_propose_memory§, memory policies, capability catalog and JEV helper only where they are genuinely wired. Do not silently rebase away unfinished worktrees or promise incomplete helper capabilities.
  - Target: §acp/tests.rs§ and corresponding Cognee/capability modules on merge target.
  - Verify: focused native/ACP parity fixtures; distinguish existence from live activation.
- [ ] **Session + observability:** check two ACP sessions using independent cwd, config and outputs; stable native IDs across reload; deterministic session histories, memory attribution and observation linkages, no duplicate tools or Cognee captures.
  - Target: §acp/registry.rs§, §acp/session.rs§ and §observe/*§ test fixtures.
  - Verify: fake sessions + concurrent local SQLite fixture, clean process shutdown.
- [ ] **Upgrade safety:** run the same parity fixture against the final merged branch, not just the pre-JEV main; code-review differences to initialized tools, hooks and permissions. Keep original TUI/native RPC behavior intact.
  - Target: integration test targets and documented compatibility matrix.
  - Verify: rustfmt, focused unit/integration tests, locked workspace checks, CLI/TUI/RPC smoke tests.

## External reference material (read-only)

1. ACP official v1 protocol: https://github.com/agentclientprotocol/agent-client-protocol/blob/main/docs/protocol/v1/overview.mdx
2. ACP v1 session semantics and cwd security: https://github.com/agentclientprotocol/agent-client-protocol/blob/main/docs/protocol/v1/session-setup.mdx
3. Official Rust SDK v3.5.0, `stdio` feature and MSRV 1.88: https://github.com/agentclientprotocol/rust-sdk/tree/v3.5.0/src/agent-client-protocol ; https://github.com/agentclientprotocol/rust-sdk/blob/v3.5.0/src/agent-client-protocol/examples/simple_agent.rs
4. Working adapter implementation as behavior reference: https://github.com/pi-rust/rpi-package/tree/e28cbcd0344215ca951bcbd3f86b2b107e7fc22d/packages/rpi-acp ; especially `README.md`, `src/protocol.rs`, `src/session.rs`, `src/questions.rs`, `src/zed.rs`.
5. Upstream bridge host/process assumptions: https://github.com/pi-rust/rpi-package/tree/e28cbcd0344215ca951bcbd3f86b2b107e7fc22d/packages/rpi-extcore

Upstream ACP plugin is `rpi-acp` 0.1.19, built for `rpi-plugin-sdk` with `rpi_plugin_register`, `RPI_BIN` and `~/.rpi` settings; its shared `rpi-extcore` process helper also appends `--permission-gate` by default. None of that may be carried into ICE-Rust unexamined. Licensing: official SDK Apache-2.0 and upstream adapter MIT; preserve required attribution if copying code rather than merely translating behavior.

## Protocol boundary and example message flow

```text
Zed / ACP client
  |  ACP v1 JSON-RPC 2.0 over stdin/stdout (icers --acp)
  v
crates/ice-cli/src/acp (typed protocol, session map, outbound writer, requests)
  |  one process per ACP session, cwd = session/new.cwd
  +--> child icers --mode rpc --session-id <id> --session-dir <dir>
          |  native JSONL over child stdio; stderr is diagnostics
          +--> AgentHarness + existing models/tools/hooks/Cognee/session JSONL

initialize (ACP)                     -> advertise accurate v1 capabilities
session/new {cwd,mcpServers}           -> validate cwd, spawn child, await ready + get_state
session/prompt {sessionId,prompt}      -> child {"type":"prompt","message":"..."}
child {"data":{"accepted":true}}      -> bridge keeps ACP request pending
child message_update/text_delta       -> ACP session/update agent_message_chunk
child message_update/thinking_delta   -> ACP session/update agent_thought_chunk
child tool_execution_*                -> ACP session/update tool_call(_update)
child agent_settled completed          -> ACP prompt response {"stopReason":"end_turn"}
child agent_settled aborted            -> ACP prompt response {"stopReason":"cancelled"}
child agent_settled failed             -> ACP request error (no false successful turn)
session/cancel                         -> child {"type":"abort"}; wait for settlement
```

ACP v1 `session/prompt` response is **terminal**, unlike native RPC's initial acceptance response. Do not conflate ACP v2's draft acceptance semantics with v1. Never emit a native `ready` marker or non-JSON diagnostic on the ACP parent stdout.

## Phase 0 — Contract tests and dependency selection (PR 1)

- [ ] Pin current main and upstream ACP reference commits in issue/PR; record expected capabilities and the stable v1 schema.
- [ ] Verify `agent-client-protocol` v3.5.0 `stdio` and its transitive dependencies build with Rust 1.88 and the current lockfile, on Linux + Windows; retain lockfile reproducibility.
- [ ] Define explicit ACP ⇔ native-RPC mapping table for initialize, new, prompt, cancel, load/resume, permission requests and config options. Document unsupported image/audio, MCP, multi-root features.
- [ ] Add offline unit fixtures with JSON-RPC frames, strict request ID correlation, unknown method/error codes, out-of-order events, duplicate/missing deltas, EOF and partial frames.
- [ ] Decide minimum read-only baseline: no writable/shell tools until a genuine approval gate exists. Inspect every route that registers extension tools so no permissions bypass is possible.
- [ ] Acceptance: a deterministic, offline adapter contract test suite exists; no user session, provider, network or existing TUI behavior is touched.

## Phase 1 — Native ACP stdio entry point (PR 2)

Target files: `crates/ice-cli/src/app.rs`, `args.rs`, `lib.rs`, new `crates/ice-cli/src/acp/mod.rs` and `acp/protocol.rs`, `crates/ice-cli/Cargo.toml` / `Cargo.lock`.

- [ ] Add the explicit `icers --acp` startup surface, intercepting before provider/model resolution, harness construction, TUI startup, and any attempt to read piped stdin. Reject conflicting `--mode rpc`/`--connect`/`--print` invocation instead of ambiguous dispatch.
- [ ] Use official SDK Agent handler + `Stdio` and v1 typed requests. Parent process must not load a model or open a TUI; ACP session children own model resolution.
- [ ] Advertise agent identity `icers`, stable v1 version and **only** implemented capabilities; avoid promising load, resume, auth, terminal, fs or MCP until implemented.
- [ ] Implement initialize, session/new validation errors and unknown-method errors. No fake prompt success; stub operations must return clear JSON-RPC unsupported errors.
- [ ] Ensure stdout strictly ACP frames; stderr only diagnostic logs with credential redaction. Bound frame sizes, active requests and outbound queues; return errors rather than panic.
- [ ] Acceptance: an ACP client can initialize and create a session over piped stdio; default TUI and `--mode rpc` are unchanged.

## Phase 2 — Session-to-native-RPC bridge (PR 3)

Target files: `acp/session.rs`, `acp/process.rs`, `acp/registry.rs`, `acp/tests.rs`.

- [ ] Resolve a trustworthy absolute `icers` executable path (prefer `current_exe`, optional explicit `ICERS_ACP_BIN` for tests), never `rpi` or another PATH fallback.
- [ ] One child `icers --mode rpc --session-id <uuid> --session-dir <absolute ACP session dir>` per ACP session with `current_dir(cwd)`, restricted stdio pipes, process exit monitoring, and kill/reap on disconnect. Do not pass `--acp` to children.
- [ ] Validate `cwd` is absolute, exists and is a directory; ensure process/tool relative paths use it. Don't claim OS sandbox isolation: cwd is not a sandbox.
- [ ] Await native `ready`, correlate child RPC responses by request ID, inspect `get_state` sessionId, preserve native child stderr separately, and bound initialization timeout.
- [ ] Provide an ACP session registry mapping opaque ACP ID -> cwd, native session ID, disk root and active child; never mix event streams across sessions.
- [ ] Ensure two ACP sessions can run independently, while each session serializes foreground prompts. Handle child death, cancellation, client EOF and process-tree cleanup.
- [ ] Acceptance: parallel sessions rooted in distinct temp workspaces function without crossing state or touching `~/.ice`.

## Phase 3 — Live output, termination and cancellation (PR 4)

Target files: `acp/translate.rs`, `acp/session.rs`, `acp/tests.rs`.

- [ ] Convert child `message_update` `text_delta`/`thinking_delta` into v1 ACP `agent_message_chunk`/`agent_thought_chunk`, preserving order, UTF-8, and opaque IDs.
- [ ] Convert `tool_execution_start/update/end` into ACP `tool_call` / `tool_call_update`, including accurate status, names, rawInput/rawOutput and useful edit/write diff content **only** when grounded in actual tool events.
- [ ] Reconcile partial/cumulative assistant output at `message_end` so no dropped final text or duplicated chunks; bound history/large tool outputs and never leak auth headers.
- [ ] Send native prompt with `message` key; treat accepted response as acknowledgment only and wait for `agent_settled` before sending ACP v1 prompt response.
- [ ] Map completed→`end_turn`, aborted→`cancelled`, token exhaustion→`max_tokens` only when actually detected, failed→error. Propagate retries and never swallow failed tool/run outcomes.
- [ ] Route concurrent `session/cancel` into native `abort` while a prompt is pending; enforce exactly-once settlement and no output after session close.
- [ ] Acceptance: an offline faux-provider stream shows text/thinking/tool updates in Zed-compatible format, with mid-run cancel and no duplicate responses.

## Phase 4 — Security and permission enforcement (PR 5; required before write-capable general release)

- [ ] Audit the available per-tool execution intercept/hook path. Existing RPC emits `ui_dialog_request` but that alone **does not prove** an approval gate protects built-in `bash`/`write`/`edit`. Test every write/command path, including extension/MCP-created tools.
- [ ] Implement a fail-closed tool policy at the actual execution boundary: allow read-only operations initially; potentially dangerous shell, edit/write, external network or user-specified paths require `session/request_permission` approval if capabilities permit.
- [ ] Translate native `ui_dialog_request` -> ACP `session/request_permission`, track pending outbound request IDs, send `ui_dialog_response` only upon an explicit allow decision; deny on disconnect, missing approval capability, client timeout, or malformed response.
- [ ] Limit approval to the exact tool call and action details; no global `--bypass` or silent auto-approval. Ensure requests remain serviceable while the foreground prompt is running; use ~600s human approval budget where appropriate.
- [ ] Verify platform-independent cancellation, process reaping, safe path encoding and that credentials/recovered transcript are never logged on ACP stderr.
- [ ] If full enforcement is not achievable in this phase, continue **read-only ACP** and advertise the limitation. Do not ship a write-enabled editor integration that merely displays cosmetic permission dialogs.
- [ ] Acceptance: malicious or absent ACP client approval cannot run unauthorized shell/edit/writes. Add regression tests for permission denial, cancellation and extension/MCP-tool bypasses.

## Phase 5 — Durable sessions, config, MCP integration (PR 6+; capability-gated)

- [ ] Define `~/.icers/agent/acp/` registry (ACP ID, exact native ID, absolute cwd and disk session root). Secure perms; no second transcript copy. Reuse existing native JSONL storage and crash recovery.
- [ ] Implement `session/load` with verified pre-existing native JSONL file + cwd binding, accurate ordered transcript replay via `get_messages`/`get_history` with pagination, including tool calls/results; fail on unknown session. Native `--session-id` **will create absent IDs**, so the bridge must verify before spawn.
- [ ] Implement `session/resume` with same verified restore but no replay. Gate capability metadata only once the exact semantics are tested; recover interrupted tool calls without blindly replaying side effects.
- [ ] Map supported models/thinking to ACP `session/set_config_option` using native get/set controls; publish accurate config option changes, persist selection across reconnect where intended.
- [ ] Support ACP client-supplied `mcpServers` only through safe **per-session** MCP config forwarding, not by writing or overwriting global/project `settings.json`. If non-empty MCP servers are not yet supported, return an explicit session/new error rather than silently ignoring them.
- [ ] Image/audio attachments, client fs/terminal methods, optional auth, advanced session modes, ACP v2 draft, subagent-specific metadata and durable custom extensions are separately scoped follow-ups.
- [ ] Acceptance: restart ACP process and successfully reload/resume the same session in its proper workspace; reject invalid IDs and wrong cwd; no old prompts/tools rerun as side effects.

## Phase 6 — External client and CI acceptance (PR 7)

- [ ] Add `docs/extensions/acp.md` setup/compatibility guide; expose `icers --acp` in help; provide a Zed settings snippet using absolute or resolved `icers` command. Do not modify the user's editor config automatically.
- [ ] Test protocol frames with an actual ACP client (Zed at minimum): initialize, open project, submit prompt, stream tool calls, cancel, approve/deny, restart/load. Run protocol conformance where available.
- [ ] Add Linux/Windows fixture CI cases and deterministic stdout-purity tests; no live API keys, no private `~/.icers` reads or writes; use temporary `ICERS_CODING_AGENT_DIR`.
- [ ] Verify `cargo fmt --all -- --check`; `RUSTFLAGS="-D warnings" cargo check --workspace --all-targets --locked`; focused `cargo test -p ice-cli --locked`; full workspace test gate and embedded-docs sync only if a source document is changed.
- [ ] Document unsupported features and stability; verify normal `icers` TUI, `icers -p`, `icers --mode rpc`, `icers --server` still behave exactly as before.
- [ ] Keep commits per feature, PRs into fork's `main`, no upstream PRs, no release/publish, and no rebrand/base branch edits (`AGENTS.md`).

## Initial Zed configuration after implementation

```jsonc
{
  "agent_servers": {
    "icers": {
      "type": "custom",
      "command": "/home/mewtwo/Zykairotis/ice-rust/target/release/icers",
      "args": ["--acp"]
    }
  }
}
```

This will **not work before the integration exists**. The settings structure is for Zed's custom ACP agent configuration and should be verified against Zed's current release during the final smoke test.

## Principal risks / non-goals

1. **Permissions** are the critical blocker for a *write-capable* release. RPC dialog plumbing exists, but a universal tool-execution approval gate has not been demonstrated by this audit.
2. **Session restoration** can silently invent a new session if the bridge simply supplies an unrecognized `--session-id`. Pre-check persisted mapping and files.
3. **Prompt completion** is easy to get wrong. Native `accepted` is **not** an ACP v1 turn-ending response; wait for `agent_settled`.
4. **Upstream identity and ABI:** `rpi-acp` and `rpi-extcore` target the upstream host; never use its binaries, `RPI_*` defaults, `~/.rpi` storage or `rpi_plugin_register` in our release.
5. **Per-session MCP** is distinct from already having an MCP client configured in icers; do not claim support for editor-provided MCP servers until passed to individual children.
6. **Cross-platform stdout and process safety:** no incidental stdout logs; no orphan child processes, no unbounded pipes, safe concurrent streams.
7. **ACP v2:** its lifecycle differs (prompt acknowledgment + later state update). Do not start with it while stable v1 clients remain dominant.

## Definition of done

- [ ] `icers --acp` is a genuine spec-compliant ACP v1 stdio agent with no extra stdout noise.
- [ ] Two independently rooted sessions survive prompt/cancel/tool traffic without state contamination.
- [ ] ACP response resolves only on native terminal settlement, with correct streaming and error propagation.
- [ ] Write/execute actions require an enforceable approval policy or are disabled by default.
- [ ] Supported load/resume and config are advertised honestly and tested against durable native JSONL state.
- [ ] Zed integration and offline CI fixtures pass; legacy TUI/RPC behavior is unchanged.
- [ ] Code reviewed in focused feature PRs, no accidental changes to the protected TS ICE paths or user config.
