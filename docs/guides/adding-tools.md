# Adding and changing tools in icers

A field guide for anyone who wants to give the `icers` agent more tools, or to
fix the ones it has. It covers four things:

1. How a tool actually reaches the model (a traced path through the code).
2. What tools icers ships today, and which of them are really available.
3. Which tools the TypeScript harness (**ice**) has that icers lacks, and which
   existing icers tools need alteration.
4. Every supported way to add a tool, with a checklist for each.

> **Provenance.** Everything below was checked against the source of this
> repository at commit `dd0e240` (`main`, 2026-10-27) and, for the TypeScript
> side, against the local ice checkout (`~/Zks/ice/packages/*/src`). File and
> line references are accurate for that commit; line numbers drift, so search
> for the symbol name if a line is off. Section 14 lists exactly what was *not*
> verified. Where a statement is an inference, it says so.

## Contents

1. [Quick decision table](#1-quick-decision-table)
2. [Vocabulary](#2-vocabulary)
3. [How a tool reaches the model](#3-how-a-tool-reaches-the-model)
4. [The tools icers has today](#4-the-tools-icers-has-today)
5. [Gap map: ice versus icers](#5-gap-map-ice-versus-icers)
6. [Existing tools that need alteration](#6-existing-tools-that-need-alteration)
7. [The `AgentTool` contract in depth](#7-the-agenttool-contract-in-depth)
8. [Route A: a built-in tool in `ice-tools`](#8-route-a-a-built-in-tool-in-ice-tools)
9. [Route B: a host tool in `ice-cli`](#9-route-b-a-host-tool-in-ice-cli)
10. [Route C: a native plugin (cdylib)](#10-route-c-a-native-plugin-cdylib)
11. [Route D: an MCP server (no Rust)](#11-route-d-an-mcp-server-no-rust)
12. [Cross-cutting checklist](#12-cross-cutting-checklist)
13. [Worked examples](#13-worked-examples)
14. [What was not verified](#14-what-was-not-verified)
15. [Inconsistencies found while writing this](#15-inconsistencies-found-while-writing-this)
16. [Suggested order of work](#16-suggested-order-of-work)
17. [Appendix: file map and commands](#17-appendix-file-map-and-commands)

---

## 1. Quick decision table

Pick the route by what the tool needs, not by what is convenient to write.

| The tool needs… | Use | Section |
|---|---|---|
| Nothing from icers: it wraps an existing service, CLI or API | **MCP server** (Route D). Zero Rust, no rebuild. | 11 |
| Files, shell, or the working directory, and should ship in every icers | **Built-in in `ice-tools`** (Route A) | 8 |
| Host state that `ice-tools` cannot see: session, settings, TUI, extension session, harness | **Host tool in `ice-cli`** (Route B) | 9 |
| To be optional, versioned and installed separately; or written by someone else | **Native plugin** (Route C) | 10 |
| To call other tools in a loop or chain (many MCP calls, one result) | The existing `mcpScript` tool, no new tool | 11.5 |
| To run a bounded child agent | A plugin using the host `child_agent` action | 10.9 |

Rules of thumb:

- **If it already exists in `ice-tools` but is not wired, wiring it is a few
  lines.** See section 5.1. That is the cheapest win in this guide.
- **Prefer fewer, broader tools.** Every registered tool costs prompt tokens on
  every request. Section 12.6 explains where that cost is paid.
- **Do not add a built-in when an MCP server would do.** Built-ins are compiled
  into every icers binary and must pass the full CI gate and MSRV 1.88.

---

## 2. Vocabulary

| Term | Meaning in this repo |
|---|---|
| **Tool** | A function the model can call by name, with a JSON-Schema parameter object. In Rust it is an `Arc<dyn AgentTool>`. |
| **`AgentTool`** | The trait every tool implements. `crates/ice-agent/src/agent_tool.rs`. |
| **`Tool`** (struct) | The *provider-facing* definition: `name`, `description`, `parameters`, `constrained_sampling`. Returned by `AgentTool::schema()`. `ice_ai::types::Tool`. |
| **`HarnessTool`** | An `AgentTool` plus a replay policy. `crates/ice-harness/src/types.rs`. This is what the session actually stores. |
| **Built-in tool** | Created in-process by icers itself (`read`, `bash`, `edit`, `write`, `docs`, …). |
| **Active tool** | A registered tool the model is currently offered. A tool can be registered but inactive. |
| **Registered tool** | Present in the harness registry (`harness.set_tools`). |
| **Extension tool** | Registered by a loaded plugin (`PluginToolAdapter`). |
| **MCP tool** | Discovered from a Model Context Protocol server. |
| **Lane** | A harness conversation lane; the main one is `harness.lane("main")`. It owns the active-tool list. |
| **Replay policy** | Whether a tool may be re-run after a crash left its outcome unknown. `Never` (default) or `Safe`. |
| **ice / TS ice** | The TypeScript harness this fork ports. Never touch its state (`AGENTS.md` §2.1). |

---

## 3. How a tool reaches the model

This is the traced path. Every later section refers back to it. Read it once
before choosing a route.

```
                        ┌──────────────────────────────────────────────┐
  built-ins             │ ice-cli/src/session.rs  build_tools()        │
  read bash edit write  │   builds Vec<(name, HarnessTool)>            │
  docs [powershell]  ──►│   applies settings.defaultTools, --tools,    │
                        │   --no-builtin-tools, adds bash_* shells,    │
                        │   then --exclude-tools                       │
                        └──────────────────────┬───────────────────────┘
                                               │
  plugins (cdylib)                             ▼
  PluginToolAdapter ──►  merge_extension_tools()   same-name plugin tool REPLACES a built-in
                                               │
  MCP tools ────────────► tools.extend(mcp_tools.registered_tools())   only names not already taken
                                               │
                                               ▼
                        active_tool_names() → filter_active_tool_names()   (tool_name_allowed)
                                               │
                                               ▼
                        harness.set_tools(tools, Some(active))
                                               │
  cognee_search ───────►  builtins.register_tools(...)   re-registered after every rebuild
                                               │
                                               ▼
              agent loop → provider request carries each active tool's schema()
                                               │
                                               ▼
              model emits tool call → validate args → BeforeToolCall hooks
                → AgentTool::execute() → AfterToolCall hooks → ToolResultMessage
                → session JSONL → TUI render
```

### 3.1 The five places a tool is touched

1. **Construction.** `build_tools` (`session.rs`, search for `fn build_tools`)
   for built-ins; `merge_extension_tools` for plugins; `McpManager` for MCP;
   `cognee/runtime.rs` for the Cognee tool.
2. **Policy.** `tool_name_allowed` (`session.rs`) applies `--no-tools`,
   `--tools` (allowlist) and `--exclude-tools` (denylist) to *every* source,
   built-in or not. `settings.defaultTools` applies **only to built-ins**, and
   only when `--tools` was not given.
3. **Activation.** `active_tool_names` + `filter_active_tool_names`. With no
   `--tools`, *every registered tool is active*. There is no "registered but
   inactive by default" mechanism (TS has one; see 6.5).
4. **Prompt.** The system prompt has a **hard-coded** "Available tools:" list in
   `session.rs` (search for `Available tools:`). Registering a tool does *not*
   add it there. The tool's own `description` still reaches the model through
   the provider tool schema. See 6.1.
5. **Presentation.** TUI grouping and previews are keyed on tool *names*
   (`ice-tui/src/tool_group.rs`, `tool_render.rs`). Unknown names fall back to
   a generic rendering. See 12.7.

### 3.2 Reload

`/reload` rebuilds the tool set from scratch: `build_tools`, then
`merge_extension_tools`, then MCP tools, then `set_tools`, then it registers the
built-in extension tools again (`cognee_search`). A tool added in any of those
places must survive that rebuild. If you add a *new category* of tool, add it
to the rebuild too, or it will vanish on the first reload. This is the single
most likely regression when adding a host tool.

### 3.3 Name collisions

- A plugin tool with the same name as a built-in **replaces** it
  (`merge_extension_tools`). Between two plugins, the first registered wins
  (`register_tool` keeps the prior).
- An MCP tool whose name is already taken is **skipped** silently
  (`tools.extend(...filter(...))`).
- MCP directly-registered tools are named `{server}_{tool}`. The gateway tools
  are literally `mcp` and `mcpScript`.

---

## 4. The tools icers has today

Derived from `build_tools`, `background_shells.rs`, `mcp/*`, and
`cognee/runtime.rs`. "Active by default" assumes no `--tools`,
`--exclude-tools`, or `defaultTools`.

### 4.1 Core built-ins

| Name | Source | Active by default | Replay | Notes |
|---|---|---|---|---|
| `read` | `ice-tools/src/tools/read.rs` | yes | `Safe` | Text and images. Inline image processor is attached in `build_tools`. Truncates by lines and bytes. |
| `bash` | `ice-tools/src/tools/bash.rs` | yes | `Never` | A foreground call without `timeout` moves to the background after 120 s (`ICERS_BASH_TIMEOUT` overrides), or stops where there are no background shells; background shells have no default timeout. Auto-backgrounds after `bashAutoBackgroundSeconds`. `shellCommandPrefix` from settings. |
| `edit` | `ice-tools/src/tools/edit.rs` | yes | `Never` | Needs a mutation-capable context. Fuzzy matching in `edit_diff.rs`. |
| `write` | `ice-tools/src/tools/write.rs` | yes | `Never` | Needs a mutation-capable context. Goes through the file-mutation queue. |
| `docs` | `ice-cli/src/docs_tool.rs` | yes | `Safe` | Read-only lookup over the **embedded** docs (see 12.9). |
| `powershell` | `ice-tools/src/tools/powershell.rs` | **Windows only** | `Never` | Registered under `#[cfg(target_os = "windows")]`. On Linux it does not exist. |

### 4.2 Background-shell tools

Added by `build_tools` **only when `bash` survived filtering and a
`SessionShells` handle exists** (`session.rs`, the `shells.tools()` call).
Constructed in `ice-cli/src/background_shells.rs`, implemented in
`ice-tools/src/tools/bash_shells.rs` and `bash_watch.rs`.

| Name | Purpose |
|---|---|
| `bash_output` | Status plus new output of a background shell; can wait, tail, or grep the log. |
| `bash_kill` | Stop a shell and its process tree. |
| `bash_input` | Write to a shell started with stdin. Approval governed by `bashInputApproval` (`ask` default, or `allow`). |
| `bash_watch` | Create/list/cancel/rearm watches that wake the agent on a pattern, silence, or interval. |

> These exist and are wired. `ICE_PORTING.md` §3 still says they are
> "missing"; that section is stale. See section 15.

### 4.3 Host-registered tools

| Name | Source | When present |
|---|---|---|
| `cognee_search` | `ice-cli/src/cognee/command.rs`, registered in `cognee/runtime.rs` (`SEARCH_TOOL_NAME`) | When Cognee memory is enabled and the name passes `tool_name_allowed`. Re-registered after every rebuild. |
| `mcp` | `ice-cli/src/mcp/proxy.rs` | When MCP is active. Gateway: status, list, search, describe, call, connect, auth. |
| `mcpScript` | `ice-cli/src/mcp/script.rs` | When MCP is active. Async JavaScript that chains MCP calls; 30 s limit. |
| UI tool | `ice-cli/src/mcp/ui.rs` (`UiTool`) | MCP apps that open UI windows. |
| `{server}_{tool}` | `ice-cli/src/mcp.rs` (`McpTool`) | Every discovered tool by default (`directTools` defaults to `true`); narrow with `directTools: false` or a list. Anything not registered directly is still reachable through `mcp`. |

### 4.4 Implemented but **not** registered anywhere

These compile, have tests, and are exported from `ice-tools`, but nothing in
`ice-cli` calls them. Verified by grepping `ice-cli`, `ice-harness`,
`ice-extensions` and `examples` for `create_grep_tool`, `create_find_tool`,
`create_ls_tool`, `create_image_tool`, `create_all_tools`,
`create_coding_tools`, `create_read_only_tools`: zero hits.

| Name | File | Why it matters |
|---|---|---|
| `grep` | `ice-tools/src/tools/grep.rs` | In-process regex search honoring `.gitignore`. No `rg` needed. |
| `find` | `ice-tools/src/tools/find.rs` | In-process glob search. No `fd` needed. |
| `ls` | `ice-tools/src/tools/ls.rs` | Directory listing. |
| `image` | `ice-tools/src/tools/image.rs` | resize/convert/info. Takes **base64 in the parameters**; see 5.1. |

A consequence worth knowing: `icers --tools read,grep` does not work today,
because `--tools` is an allowlist over what `build_tools` constructed, and
`grep` was never constructed.

### 4.5 Name of the other "image tool"

`ice-harness/src/image_tool.rs` is **not** a tool. Its function `wrap()` is a
decorator applied to *every* tool in `agent_harness.rs` that normalizes images
in tool results before they reach the model or transcript. Do not confuse it
with the `image` tool in 4.4.

---

## 5. Gap map: ice versus icers

ice's tool inventory was read from source, not from memory:

- Core: `packages/coding-agent/src/core/tools/` →
  `read bash edit write grep find ls` (the `ToolName` set,
  `allToolNames`), plus `bash_output bash_kill bash_input bash_watch`.
- Subagents V2: `packages/coding-agent/src/subagents/v2/extension.ts` →
  `agent`, `agent_control`, `agent_patch`.
- Plan/build safe-verify: `packages/coding-agent/src/ice-safe-verify.ts` →
  `ask`, `draft_plan`, `propose_plan`, `read_plan`.
- Cognee: `packages/coding-agent/src/ice-cognee.ts` → `cognee_search`.
- Workgraph: `imported_extensions/pi-workgraph/src/tools.ts` →
  `workgraph_claim` / `ice_workgraph_claim` (only these two names matched my
  search; there may be more registered through a different call shape).

ice's **default active** tools are `["read","bash","edit","write"]`
(`core/agent-session.ts`, `core/sdk.ts`). `grep`, `find`, `ls` are registered
but inactive until asked for.

### 5.1 Implemented in Rust, not wired (cheap)

| Tool | ice behavior | icers status | Work |
|---|---|---|---|
| `grep` | registered, inactive by default | built, tested, unwired | Wire in `build_tools`; decide default-active (see 6.5); add to the system prompt list; mark `ToolReplay::Safe`. |
| `find` | same | same | same |
| `ls` | same | same | same |
| `image` | **no such tool in ice's tool list** | Rust-only, unwired | **Recommend not wiring as-is.** It takes `image_data` as base64 *in the model's parameters*, so the model would have to emit the whole image as tokens. A path-based variant would be useful; the current one is not. |

`ice-tools/src/tools/sets.rs` already provides `create_read_only_tools`
(`read, grep, find, ls`), `create_coding_tools` (`read, bash, edit, write`) and
`create_all_tools`. The CLI builds its own list instead of using them, which is
why the three search tools were silently left out. A worked example that wires
them is in 13.1.

### 5.2 Missing entirely

| Tool | ice source | What it does | Rust route | Size |
|---|---|---|---|---|
| `agent`, `agent_control`, `agent_patch` | `subagents/v2/extension.ts` (~17k lines across `subagents/`) | Subagents V2: definitions, launch, registry, permissions, worktree isolation, patch integration. | Plugin using the host `child_agent` action, or a host tool. `ICE_PORTING.md` §1. **Largest gap.** | Very large |
| `ask`, `draft_plan`, `propose_plan`, `read_plan` | `ice-safe-verify.ts` | Plan/build guarded mode: the model drafts a plan, asks questions, proposes it for approval. | Host tools plus a state machine; needs the `access` concept (see 6.4). `ICE_PORTING.md` §2. | Large |
| `workgraph_claim` | `imported_extensions/pi-workgraph` | Claim a unit of work in a workgraph. | Plugin. Depends on the workgraph host; check whether you want it at all. | Unknown |
| `todo` | not in the ice core tools; ships as the companion plugin `ice-todo` | Persistent todo list. The icers **system prompt tells the model to use it** (6.1). | Companion plugin. **Do not `icers install` it** until the package-name decision is made (`AGENTS.md` §2.6). | Small if built in-tree |
| `question`, `questionnaire`, `structured-output`, `ssh`, `sandbox`, … | `examples/extensions/*` shipped inside the ice package | Example extensions, not core. | Only if you actually want them. | Per tool |

`ICE_PORTING.md` §7.2 lists the host API methods a Rust plugin lacks that these
ports would need (`registerSettings`, `getAllTools`, widget/header/footer
setters, and others). A plugin can already register tools, commands, flags,
shortcuts and providers, open dialogs and side questions, call `ask_user`, own
stdio, and run child agents (`ice-extensions/src/actions.rs`).

### 5.3 Per-field comparison of the tool definition

The ice `ToolDefinition` (`core/extensions/types.ts`) carries more than the
Rust trait does.

| ice field | Rust equivalent | Gap |
|---|---|---|
| `name`, `description`, `parameters` | `Tool { name, description, parameters }` | none |
| `label` | `AgentTool::label()` | none |
| `constrainedSampling` | `Tool.constrained_sampling` | none (field exists; the built-ins pass `None`) |
| `prepareArguments` | `AgentTool::prepare_arguments` | none |
| `executionMode` (`sequential`/`parallel`) | `AgentTool::execution_mode` | none |
| `execute(id, params, signal, onUpdate, ctx)` | `execute(id, params, signal, on_update)` | **no `ctx` argument.** Built-ins capture an `ExecutionToolContext` at construction; plugins get `cwd`/`sessionId` injected as `__ice` (10.6). |
| `promptSnippet` | none found | The "Available tools" prompt section is hard-coded. |
| `promptGuidelines` | none found | Same. |
| `access: "read" \| "write"` | none | Needed by plan/build modes (§2 of `ICE_PORTING.md`). |
| `renderShell`, `renderCall`, `renderResult` | none | Rust tools cannot supply their own TUI rendering. Rendering is keyed on tool name in `ice-tui`. |
| `AgentToolResult.addedToolNames` | `AgentToolResult.added_tool_names` | exists (deferred-tool loading) |
| `AgentToolResult.terminate` | `AgentToolResult.terminate` | exists |

### 5.4 Where the exhaustive spec lives

This guide is a map, not the specification. The detailed, source-referenced
audit is `docs/gaps.md` (the prioritized ledger) with one file per area under
`docs/ice-gap/` (`tools.md`, `subagents.md`, `safe-verify.md`, `extensions.md`,
`cognee.md`, …). Entries carry stable IDs. The ones that matter for this guide:

| Topic here | Audit entry | Status recorded there |
|---|---|---|
| Tool factories, sets, definition adapters (5.1, 5.3) | `TOOLS-018` | partial |
| Per-tool call/result renderers (5.3, 12.7) | `TOOLS-027` | partial |
| `find`, `grep`, `ls` semantics | `TOOLS-028`, `TOOLS-029`, `TOOLS-030` | partial |
| Image handling | `TOOLS-023` | partial |
| Background shells and watches (4.2) | `TOOLS-003` … `TOOLS-013` | see `docs/gaps.md` M6 and the "Tools area" note |
| Subagents V2 (5.2) | `SUB-001` … in `subagents.md` | missing/partial |
| Plan/build tools (5.2) | `safe-verify.md` | missing/partial |

Two cautions when reading it:

- **Statuses are frozen at the audit baseline** (the Rust side was `main` at
  `1891337`), not at today's `main`. Example: `TOOLS-018` says no
  coding/read-only/all set factory exists, but `ice-tools/src/tools/sets.rs`
  now provides them. Check the code before trusting a "missing".
- `docs/gaps.md` states that every Tools entry except `TOOLS-019` is ported on
  the branch `claude/awesome-einstein-pcdwne`. That branch's tool work is in
  `main` (background shells are present), but the per-entry statuses in
  `docs/ice-gap/tools.md` were not all updated. Treat the ledger checkboxes as
  the more current signal and the per-entry "Status" lines as the baseline.

The repository's stated policy (and the maintainer's) is **impact-first**: port
the gaps users actually hit, not exhaustive parity. Section 16's order follows
that.

---

## 6. Existing tools that need alteration

Ordered roughly by how likely they are to bite a user.

### 6.1 The system prompt's tool list is hard-coded and out of date

Location: `ice-cli/src/session.rs`, the `format!` that begins
`"You are an expert coding assistant operating inside icers…"`.

It lists exactly five tools: `read`, `bash`, `edit`, `write`, `docs`. It does
not mention `bash_output`, `bash_kill`, `bash_input`, `bash_watch`,
`cognee_search`, `mcp`, `mcpScript`, or `powershell` (although a later
guideline mentions "if a powershell tool is available").

It *does* tell the model: "Track multi-step work with the todo tool instead of
re-listing the plan in prose". There is no built-in `todo` tool. If the
companion plugin is not installed, the model is instructed to use a tool that
does not exist, and may attempt the call.

Alterations to consider:

1. Make the list **derive from the active tool set** rather than a string
   literal, so a new tool cannot be forgotten. This is the structural fix and
   it removes a whole class of bug.
2. Make the `todo` guideline **conditional on the tool being registered.**
3. Until that is done, every built-in you add must be added to this literal by
   hand. That is easy to forget; the compiler will not tell you.

The tool's own `description` still reaches the model via the provider tool
schema regardless, so a missing prompt line degrades guidance, it does not hide
the tool.

### 6.2 `grep` / `find` / `ls` wiring

See 5.1 and the worked example 13.1. This is the highest value/lowest cost
alteration.

### 6.3 `docs` tool reads a frozen snapshot

`docs` serves `crates/ice-cli/embedded-docs/*`, copied byte-for-byte from
`docs/ice-tool/*` by `scripts/sync-embedded-docs.sh`. **This file you are
reading is under `docs/guides/`, which is not embedded**, so the model cannot
find it with `docs`. If you want the model to know how to add tools, either
move a condensed version into `docs/ice-tool/extension-authoring.md` and run
the sync script, or leave it as human documentation. See 12.9.

### 6.4 No `access` classification

ice marks extension tools `access: "read"` or `"write"` so plan/build modes can
expose only the safe ones. icers has no such field on `AgentTool`. Until plan
mode (§2) is ported nothing consumes it, so do **not** add it speculatively.
Add it in the same change that adds the consumer.

### 6.5 No "registered but inactive by default"

ice registers all seven core tools and activates four. icers' activation rule is
"everything registered is active unless filtered". Consequences:

- Wiring `grep`, `find`, `ls` makes them **active by default**, unlike ice. That
  costs three more tool schemas in every request.
- `settings.defaultTools` can restrict the built-ins, but it filters the
  *constructed* list, so a tool you want "available on request but off by
  default" cannot be expressed.

This is a design decision, not a bug. Decide before wiring (13.1 discusses the
trade-off). If you want ice's behavior, the change belongs in
`active_tool_names` / `filter_active_tool_names` and must survive `/reload`.

### 6.6 `--no-builtin-tools` comment is stale

In `build_tools`: "v1 has none, so it's equivalent to `--no-tools` here". That
was true before plugin and MCP tools existed. Today `--no-builtin-tools` clears
only the built-ins; plugin, MCP and Cognee tools are still merged afterwards
(filtered by `tool_name_allowed`). The behavior is arguably right; the comment
is wrong. Fix the comment when you touch the function.

### 6.7 `powershell` is Windows-only by `cfg`

`#[cfg(target_os = "windows")]` in `build_tools`. The tool source compiles on
Linux (it is in `ice-tools`), it is simply never registered. If you want it
testable on Linux CI, the cfg would have to move to runtime detection; weigh
that against exposing a tool the model cannot use.

### 6.8 Policy asymmetry between sources

- `settings.defaultTools`: built-ins only.
- `--tools` / `--exclude-tools`: all sources.
- MCP tools are appended **without** passing through `tool_name_allowed` at
  registration; they are filtered at *activation* through
  `filter_active_tool_names`. The net effect is that a denied MCP tool is
  registered but inactive. That is fine for the model; it matters if you ever
  enumerate "registered" tools for a UI (the RPC `get_available_tools` returns
  both lists).

### 6.9 Replay policy defaults

`HarnessTool::new` defaults to `ToolReplay::Never`. Only `read` and `docs` opt
into `Safe`. Plugin, MCP and Cognee tools are `Never`. That is the safe
default; do not flip it for a tool unless re-running it is truly harmless.
`grep`, `find`, `ls` are read-only and should be `Safe` when wired.

---

## 7. The `AgentTool` contract in depth

Source: `crates/ice-agent/src/agent_tool.rs`.

```rust
#[async_trait]
pub trait AgentTool: Send + Sync {
    fn schema(&self) -> &Tool;                       // name, description, parameters
    fn label(&self) -> &str;                         // UI label
    fn execution_mode(&self) -> ToolExecutionMode { ToolExecutionMode::Parallel }
    fn prepare_arguments(&self, args: serde_json::Value)
        -> Result<serde_json::Value, AgentError> { Ok(args) }
    async fn execute(
        &self,
        tool_call_id: &str,
        params: serde_json::Value,
        signal: CancellationToken,
        on_update: Arc<dyn Fn(ToolResultPartial) + Send + Sync>,
    ) -> Result<AgentToolResult, AgentError>;
}
```

### 7.1 `schema()`

Returns `&ice_ai::types::Tool`:

```rust
Tool {
    name: "docs".to_string(),
    description: "…".to_string(),
    parameters: ice_ai::types::Schema::new(serde_json::to_value(schemars::schema_for!(Input))?),
    constrained_sampling: None,
}
```

- **Name.** Lowercase, `snake_case` for built-ins (`bash_output`). The gateway
  `mcpScript` is camelCase, so the code does not forbid it, but providers differ
  in what they accept; stay with `[a-z0-9_]`. Provider limits on tool-name
  length and characters are provider-specific and were not audited here.
- **Parameters.** Every built-in derives the schema from a `schemars::JsonSchema`
  struct (`ImageToolInput`, `DocsInput`, …). Hand-written JSON via
  `serde_json::json!` is used by the MCP gateway tools. Both work. Prefer the
  derive: schema and deserialization cannot drift.
- **`additionalProperties`.** The MCP tools set `"additionalProperties": false`.
  Plugin tools must *tolerate* unknown top-level keys (10.6).
- **Description.** This is the model's only guidance for a tool that is not in
  the hard-coded prompt list (6.1). Write it like documentation: what it does,
  when to use it instead of `bash`, limits, and the output shape.

### 7.2 `execute`

- `params` is the model's JSON after `prepare_arguments`. Deserialize it
  yourself; return `AgentError::Validation` for a bad shape (7.4).
- `signal` is a `tokio_util::sync::CancellationToken`. **Honor it.** Select on
  `signal.cancelled()` around any await that can block, and kill any child
  process you started. The test suite has abort-bracketing tests
  (`ice-tools/tests/abort_bracketing.rs`) that show the expected behavior.
- `on_update` streams partial results (progress, tail output). The agent loop
  gates it with an `accepting_updates` flag, so a call made after `execute`
  returns is silently dropped. You do not need to track that yourself, and you
  may clone the `Arc` into a background task. The bash tool does exactly that.

### 7.3 Results

```rust
pub struct AgentToolResult {
    pub content: Vec<TextContentOrImage>,   // what the MODEL sees
    pub details: serde_json::Value,         // what the UI and session record see
    pub usage: Option<Usage>,
    pub added_tool_names: Vec<String>,      // activate more tools from a result
    pub terminate: bool,                    // hint: stop the batch
}
```

- `content` is the only thing the model reads. `TextContentOrImage::text(s)` for
  text.
- `details` is structured data for the UI. Put machine-readable status there,
  not in prose.
- **Bound your output.** Use the truncation helpers in
  `ice-tools/src/truncate.rs` (`truncate_head`, `truncate_tail`,
  `DEFAULT_MAX_LINES`, `DEFAULT_MAX_BYTES`, `GREP_MAX_LINE_LENGTH`). An unbounded
  result is a context-window incident. `grep` and `read` show the convention:
  limit, truncate, and say in the text that truncation happened and how to get
  the rest.
- `terminate: true` is a hint to end the batch; use it rarely.

### 7.4 Errors

`ice_agent::error::AgentError`:

| Variant | Use for |
|---|---|
| `Tool(String)` | The tool ran and failed (file not found, command failed). Constructor: `AgentError::tool(msg)`. |
| `Validation(String)` | The model's arguments were wrong. |
| `Abort` | The run was cancelled. |
| `Provider`, `Queue`, `State` | Loop/harness internals; not for tools. |

The loop turns an `Err` into an error `ToolResultMessage` the model sees. Write
error messages for the model: say what was wrong *and what to do next*.

### 7.5 Execution mode

`Parallel` (default) lets the loop run several calls from one assistant turn
concurrently. If **any** tool in the batch reports `Sequential`, the whole batch
runs one at a time. Declare `Sequential` for anything that mutates shared state
the loop cannot see. File writes do not need it: they go through the
file-mutation queue (7.6).

### 7.6 The execution environment

Built-in tools never touch `std::fs` or `std::process` directly. They go through
`ice_tools::ExecutionToolContext` (`tools/tool_context.rs`), which wraps:

- `ExecutionEnv` (read-only view: filesystem + shell), with `OsExecutionEnv` for
  real use and `InMemoryExecutionEnv` for tests.
- `MutatingEnv` (adds the **file-mutation queue**, a per-canonical-path lock so
  concurrent `edit`/`write` calls on one file serialize).

This is what makes tools testable without a disk. **A new file-touching built-in
should follow it.** Constructing a mutating tool without a mutation-capable
context is a construction error (`create_tool` returns
`"The <name> tool needs a mutation-capable execution context."`), while
`create_write_tool` / `create_edit_tool` `expect()` it. Keep read-only tools on
the read-only view so they work in read-only contexts.

Path handling: `path_utils.rs` (`normalize_tool_path`, `resolve_tool_path`,
`resolve_read_tool_path`). Use them. Do not join user paths yourself.

---

## 8. Route A: a built-in tool in `ice-tools`

Use when the tool is about files, the shell or the working directory and should
exist in every build.

### 8.1 Steps

1. **Create** `crates/ice-tools/src/tools/<name>.rs`. Copy the closest sibling:
   `ls.rs` (small, read-only) or `write.rs` (mutating).
2. **Define the input** as a `#[derive(Deserialize, JsonSchema)]` struct; build
   the `Tool` in a `schema()` constructor.
3. **Implement `AgentTool`** (section 7).
4. **Export** from `tools/mod.rs` (`pub mod <name>;` and a re-export of
   `create_<name>_tool`), then from `lib.rs` if SDK users should reach it.
5. **Decide whether it joins the public set.** `sets.rs` `ToolName` is "ice's
   public built-in set" (seven names). It deliberately excludes the background
   shell tools and `powershell`. Add to `ToolName` only if ice has it too;
   otherwise create it directly, as `bash_shells.rs` does.
6. **Register** in `ice-cli` `build_tools` (section 3). Choose the replay policy.
7. **Add to the system prompt list** (6.1).
8. **Add TUI handling** if it deserves grouping or a preview (12.7).
9. **Tests** (12.8). A new behavior needs a test; a bug fix needs a regression
   test that fails before the fix (`AGENTS.md` §5).
10. **Docs**: update the user guide and re-sync embedded docs (12.9).
11. **Update `ICE_PORTING.md`** if it ports an ice tool (`AGENTS.md` §8.5).

### 8.2 Skeleton (read-only tool)

Modeled on `docs_tool.rs` and `ls.rs`. Compile it before trusting it; the
bracketed parts are yours.

```rust
//! `<name>` — <one line>. Mirrors `packages/coding-agent/src/core/tools/<name>.ts`.

use std::sync::Arc;

use async_trait::async_trait;
use ice_agent::agent_tool::AgentTool;
use ice_agent::error::AgentError;
use ice_agent::types::{AgentToolResult, TextContentOrImage, ToolResultPartial};
use ice_ai::types::{Schema, Tool};
use schemars::JsonSchema;
use serde::Deserialize;
use tokio_util::sync::CancellationToken;

use crate::tools::tool_context::ExecutionToolContext;

#[derive(Debug, Deserialize, JsonSchema)]
struct Input {
    /// What the model should put here, in one sentence.
    path: String,
}

pub struct ExampleTool {
    schema: Tool,
    // hold what you need from the context at construction time
}

impl ExampleTool {
    fn new(_context: &ExecutionToolContext) -> Self {
        let params = schemars::schema_for!(Input);
        Self {
            schema: Tool {
                name: "example".to_string(),
                description: "Explain what it does, when to prefer it over bash, \
                              its limits, and what the output looks like."
                    .to_string(),
                parameters: Schema::new(serde_json::to_value(params).unwrap_or_default()),
                constrained_sampling: None,
            },
        }
    }
}

pub fn create_example_tool(context: &ExecutionToolContext) -> Arc<dyn AgentTool> {
    Arc::new(ExampleTool::new(context))
}

#[async_trait]
impl AgentTool for ExampleTool {
    fn schema(&self) -> &Tool { &self.schema }
    fn label(&self) -> &str { "example" }

    async fn execute(
        &self,
        _tool_call_id: &str,
        params: serde_json::Value,
        signal: CancellationToken,
        _on_update: Arc<dyn Fn(ToolResultPartial) + Send + Sync>,
    ) -> Result<AgentToolResult, AgentError> {
        let input: Input = serde_json::from_value(params)
            .map_err(|e| AgentError::Validation(format!("invalid arguments: {e}")))?;
        if signal.is_cancelled() {
            return Err(AgentError::Abort);
        }
        // … do the work through the ExecutionEnv, bounded and cancellable …
        let text = format!("result for {}", input.path);
        Ok(AgentToolResult {
            content: vec![TextContentOrImage::text(text)],
            details: serde_json::json!({ "path": input.path }),
            ..Default::default()
        })
    }
}
```

### 8.3 What can go wrong

- **Forgetting `.with_replay(ToolReplay::Safe)`** on a read-only tool is not an
  error, it just means recovery reports "unknown outcome" instead of re-running.
- **Forgetting the system-prompt line** (6.1) leaves the tool discoverable only
  through its schema description.
- **Forgetting reload.** Built-ins go through `build_tools`, which `/reload`
  already re-runs, so a built-in is safe. A tool registered *somewhere else*
  must be re-registered (3.2).
- **Blocking the runtime.** Do not call blocking `std::fs` from `execute`; use
  the `ExecutionEnv` or `spawn_blocking`.

---

## 9. Route B: a host tool in `ice-cli`

Use when the tool needs state `ice-tools` cannot see: the extension session,
settings, the harness, the TUI, the MCP manager, a Cognee client. Examples in
tree: `docs_tool.rs` (embedded docs), `cognee/command.rs` (`cognee_search`),
`mcp/*` (gateway tools), `background_shells.rs` (shell controls).

### 9.1 Two registration shapes

**Shape 1: constructed in `build_tools`.** Right when the tool needs nothing
beyond `ExecutionToolContext`, args, and the optional `SessionShells` handle.
Add it to the `all` vector with a name. It then participates in `--tools`,
`--exclude-tools`, `defaultTools`, and reload for free. `docs` is the model.

**Shape 2: registered by a runtime owner after the harness exists.** Right when
the tool needs a live object that is created later (Cognee runtime, MCP
manager). Then you own three obligations:

1. Register it after `set_tools`.
2. Gate it with `tool_name_allowed(NAME, &args)` so `--tools` and
   `--exclude-tools` still work (Cognee does exactly this).
3. **Re-register it on reload** (see the end of the reload block in
   `session.rs`, which calls `ctx.builtins.register_tools(...)`).

Skipping obligation 3 is the classic bug: the tool works at launch and vanishes
after `/reload`.

### 9.2 Holding state safely

Tools are `Send + Sync` and held in `Arc`. The MCP tools hold a **weak**
reference to the manager (`self.manager.upgrade()`), and fail with
`"MCP session ended"` if it is gone. Copy that pattern for anything that can
outlive the session; a strong reference from a tool to its owner creates a
cycle.

### 9.3 Output hygiene for host tools

MCP output passes through `mcp/output_guard.rs`. If your tool returns
third-party text, run it through a guard like that one rather than trusting it:
bound the size, and treat it as data, never as instructions.

---

## 10. Route C: a native plugin (cdylib)

Use when the tool should be optional, separately versioned, or written by
someone outside the core. This is also the route for the unported ice
extensions (`todo`, a `delegate_task` subagent tool, workgraph).

Authoritative references, in order of trust:

1. `examples/plugin-stub/src/lib.rs` (851 lines, a runnable ABI template that
   registers an `echo` tool). **Copy this, do not redesign the ownership
   protocol.**
2. `crates/ice-plugin-sdk/src/lib.rs` (`PluginApi`, `StableToolSchema`,
   function-pointer types).
3. `crates/ice-extensions/src/tool.rs` (`PluginToolHandle`,
   `PluginToolAdapter`, the host side).
4. `docs/extensions/authoring.md`. It is the most complete prose, **but it is
   written in Chinese** and contains one statement that conflicts with the
   code (15.2). `AGENTS.md` asks that non-English prose be translated when
   touched; that has not been done for this file.

### 10.1 Crate shape

```toml
[package]
name = "my-tools"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
ice-plugin-sdk = { path = "…/crates/ice-plugin-sdk" }   # workspace dep in-tree
serde_json = "1"
```

The entry symbol is **`ice_plugin_register`**
(`REGISTER_SYMBOL_UNIFIED = b"ice_plugin_register\0"`) and the ABI version is
`ICERS_PLUGIN_ABI_VERSION_UNIFIED = 4`. The signature is
`extern "C" fn(api: *const PluginApi) -> i32`; return `0` on success.

> The symbol was renamed from `rpi_plugin_register` in the rebrand, so prebuilt
> upstream plugins will not load (`ICE_PORTING.md` §11.2). The plugin ABI symbol
> and package names are still an **open decision** (`AGENTS.md` §10). Do not
> publish plugins against names that may change.

### 10.2 The registration call

`PluginApi.register_tool` is an `Option<extern "C" fn(...)>`:

```rust
register_tool(
    schema: *const StableToolSchema,   // name + description + parameters JSON
    execute_fn: ToolExecuteFn,
    poll_fn: ToolPollFn,
    cancel_fn: ToolCancelFn,
    destroy_fn: ToolDestroyFn,
    plugin_free_string: FreeStringFn,
) -> i32
```

Check `api.register_tool` for `None` before calling: an older host leaves the
slot empty, and you should refuse to load with a nonzero return rather than
crash. Fields are **append-only** across versions so old plugins keep their
offsets; check `api.struct_size` before reading any slot you added later.

### 10.3 The four-function lifecycle

A tool is driven by four plugin functions, per call:

| Function | Contract |
|---|---|
| `execute(tool_call_id, params, …) -> StepHandle` | Parse the JSON (an owned `StbString`, freed with the host's `free_string`). Create a **per-call** handle (`Box::into_raw`). Do not block. |
| `poll(handle, …) -> Step` | **Must be non-blocking.** Return `Pending` (optionally calling the progress callback synchronously), `Done`, or `Err`. |
| `cancel(handle)` | Set a thread-safe cancel flag. **Idempotent. Must not free the handle.** |
| `destroy(handle)` | Called by the host **exactly once** at the end; frees the handle. A null pointer is a no-op. |

The host side (`PluginToolAdapter`) runs a *blocking* driver on
`spawn_blocking` that loops `poll`, forwards `Pending` partials, and delivers
the final result through a oneshot. `cancel` is not `destroy`.

### 10.4 Ownership of strings

- A `StbString` the **host** produces (your `params`, event payloads) is freed
  by **you**, through the host's `free_string`.
- A `StbString` **you** produce (the `Done` result, partial progress) is freed
  by the **host**, through the `plugin_free_string` you passed at registration.
- **No panic may cross an `extern "C"` boundary.** Wrap bodies in
  `ice_plugin_sdk::guard` / `guard_or` and convert a panic into an error result.
  `REGISTER_PANIC_STATUS` (70) is returned if registration itself panics.

Use `ice_plugin_sdk::handles::ToolHandles` for the per-call state map instead
of raw `static mut`; the stub does (`ECHO_DRIVES`).

### 10.5 Result shape

A plugin returns its result as a JSON string. The host parses it leniently
(`json_to_result` in `ice-extensions/src/tool.rs`): a missing field takes its
default, and unparseable input yields an empty result rather than an error.

```json
{
  "content": [
    { "type": "text",  "text": "what the model should read" },
    { "type": "image", "data": "<base64>", "mimeType": "image/png" }
  ],
  "details": { "anything": "structured, for the UI and the session record" },
  "terminate": false,
  "addedToolNames": ["another_tool"],
  "usage": { "…": "optional, see below" }
}
```

| Field | Notes |
|---|---|
| `content[].type` | `"image"` is recognized; **any other value is treated as text**. A text block with no `text` becomes the empty string. |
| `content[].mimeType` | Also accepted as `mime_type`. Defaults to `image/png` if absent. |
| `details` | Any JSON. Defaults to null. |
| `terminate` | Boolean hint to end the batch. |
| `addedToolNames` | Also accepted as `added_tool_names`. Activates more tools from a result. |
| `usage` | Parsed into `Usage` when it deserializes; ignored otherwise. The source comment says usage does **not** cross to the plugin on the way *out* (v1 limit); on the way *in* it is read. |

Partial progress (`Pending`) uses the same shape, delivered through the
progress callback; the host forwards it as a tool update.

**Failure.** Return the error step with a message. The host turns it into
`AgentError::Tool(message)` and the model sees it as a tool error. A plugin that
panics, a poll that never makes progress, or a handle failure is also reported
as a tool or state error, not a host crash. Cancellation reports
`"plugin tool cancelled"`.

**Execution mode.** Plugin tools are always `Parallel` in v1 (the adapter's
comment: a plugin "could declare Sequential via a future schema field"). A
plugin tool that must not run beside other calls cannot ask for that today.

**Replay.** Plugin tools get the default `ToolReplay::Never`; a plugin cannot
opt into `Safe`.

### 10.6 The injected `__ice` context

Before the parameters reach your `execute`, the host adds a reserved key:

```json
{ "your": "args", "__ice": { "cwd": "/path/to/project", "sessionId": "01adb…" } }
```

The key is `ice_plugin_sdk::HOST_CONTEXT_KEY`.

- The **model cannot see or write it.** Injection happens after argument parsing
  and before `execute`; it is not in tool events or the session log, and the
  host's value overrides a same-named model key.
- **Either field may be missing**, and so may the whole key (no cwd available;
  an ephemeral session has no `sessionId`; an older host injects nothing).
- **Therefore your parameter type must tolerate unknown top-level fields.**
  Take `serde_json::Value`, or a struct *without*
  `#[serde(deny_unknown_fields)]`. A strict struct fails on every call. This
  directly contradicts item 3 of the "best practices" list in
  `authoring.md` (15.2).
- For child-agent invocations the host also adds `runtimeHandle` and
  `runtimeKey` inside `__ice` (see `PluginToolAdapter::execute`); they identify
  the host runtime for action callbacks and are not for general use.
- For per-session state, key on `sessionId`. When it is absent, fall back to
  project-level state **and say so in the reply**; do not invent a shared fake
  session.

### 10.7 Safety expectations

- Bound paths, URLs, commands, counts and output sizes.
- A single tool failure returns a structured error; it never exits the host.
- No network or filesystem access outside what the tool's description declares.
  A plugin is native code running in the icers process with the user's
  privileges. There is no sandbox in this ABI (inference from the design: the
  loader uses `libloading`; I found no isolation layer).
- README your schema, permission boundary, persistence files, network use,
  supported platforms and uninstall path (`authoring.md` §6.8).

### 10.8 Developing and loading

- `icers dev` (`ice-cli/src/dev_extension.rs`) builds and hot-loads an
  extension from a source directory. `authoring.md` §7 documents it.
- Project-local packages: an `.icers` project package list can point at an
  extension source directory (`authoring.md` §8).
- **Do not run `icers install <plugin>`** for the companion names
  (`ice-todo`, `ice-subagents`, …). They resolve by name on crates.io and are
  not owned by this project or by upstream (`AGENTS.md` §2.6). Build from
  source and load by path instead.
- Project-level extensions are only discovered when the project is trusted;
  an untrusted project prints a warning and skips them (`session.rs`, the
  "discovered extensions are explicitly disabled" message; `--approve` or
  `/trust yes` re-enables).

### 10.9 Child agents from a plugin

A plugin can run a bounded child agent through the host `child_agent` action
(`ice-extensions/src/actions.rs`, implemented in
`ice-cli/src/extensions_actions.rs` and `child_agents.rs`). Constraints
visible in the code: at most 16 jobs (`MAX_JOBS`), 4 MiB of output
(`MAX_OUTPUT`), 8192 events (`MAX_EVENTS`); the child runs in an in-memory
session inheriting the effective provider binding, project tools and the
parent's approval UI; tool scope can only narrow; budgets and cancellation are
host-managed. This is the building block for a Rust `delegate_task`/`agent`
tool. The full ice Subagents V2 (definitions, registry, worktree isolation,
patch integration) is **not** provided by it.

### 10.10 Reload and tool lifetime

A reloaded plugin may add or remove tools; `/reload` re-runs
`merge_extension_tools` against the new snapshot. Plugin tools are registered at
**load time only**. There is no runtime `registerTool` after load (ice's
`dynamic-tools` example has no Rust equivalent).

---

## 11. Route D: an MCP server (no Rust)

Use when the capability is already (or easily becomes) a process that speaks the
Model Context Protocol. You get a tool with no icers rebuild, no ABI, and no
CI gate.

### 11.1 Configuration

icers reads `mcp.json` from two places (`ice-cli/src/mcp.rs`, `read_configs`):

1. Global: `<agent dir>/mcp.json` (`~/.icers/agent/mcp.json`).
2. Project: `<project>/.icers/mcp.json`, **only when the project is trusted**.

A malformed file is an error naming the path (it is not skipped). Servers merge
**by name, replacing the whole entry, never field by field**. Precedence, lowest
to highest: imported configs, then global, then trusted project. Server names
must match `[A-Za-z0-9_-]+`.

Top-level keys:

| Key | Type | Meaning |
|---|---|---|
| `mcpServers` | object | Server name → entry (table below). |
| `imports` | string[] | Pull in other tools' stdio servers as the lowest layer. Kinds: `cursor`, `claude-code`, `claude-desktop`, `codex`, `opencode`, `windsurf`, `vscode`. |
| `settings` | object | Global MCP tuning, read leniently (a bad value falls back to its default). Keys below. |

A server entry (`ServerConfig`, camelCase keys). **Set exactly one of `command`,
`url`, `socket`**; setting more than one is an error (`"MCP server must set only
one of command, url and socket"`).

| Field | Type | Default | Meaning |
|---|---|---|---|
| `command` | string | — | Executable for a stdio server. |
| `args` | string[] | `[]` | Arguments. |
| `env` | object | `{}` | Extra environment for the child process. |
| `cwd` | path | project cwd | Working directory of the child process. |
| `url` | string | — | Streamable HTTP (or legacy SSE) endpoint. |
| `socket` | string | — | An rmcp-mux Unix socket (Unix only). Expands `~` and `${VAR}`. icers owns only the connection, never the server process. |
| `headers` | object | `{}` | HTTP headers. Values support `${VAR}` and `$env:VAR`. |
| `auth` | `"bearer"` \| `"oauth"` | none | Authentication style for HTTP servers. |
| `bearerToken` | string | — | Literal token for `auth: "bearer"`. Prefer the env form. |
| `bearerTokenEnv` | string | — | Name of an environment variable holding the bearer token. |
| `oauth` | object | `{}` | For `auth: "oauth"`: `grantType` (`authorization_code` default, or `client_credentials`), `clientId`, `clientSecret`, `scope`, `redirectUri`, `clientName`, `authServerMetadataUrl`, `skipIssuerMetadataValidation`. |
| `disabled` | bool | `false` | Keep configured but do not start. |
| `lifecycle` | `"eager"` \| `"lazy"` \| `"keep-alive"` \| `"lazy-keep-alive"` | `"eager"` | `eager`: start with the session. `lazy`: start on first use and stop after the idle timeout. `keep-alive`: start with the session and restart with backoff. `lazy-keep-alive`: start on first use, then never idle out and restart with backoff. |
| `idleTimeout` | number (minutes) | `settings.idleTimeout`, else 10 | Idle time before a `lazy` server is stopped. |
| `requestTimeoutMs` | integer | 60 000 | Per-call timeout for this server's tools. Capped at one day. |
| `directTools` | bool \| string[] | **`true`** | Which discovered tools register as their own tool. See 11.3. |
| `exposeResources` | bool | `true` | Expose the server's resources as tools. Resource tools are gated by this, not by `directTools`. |
| `approveTools` | bool \| string[] | none | Require approval: `true` for every tool, or a list of remote tool names. Applies to calls through the `mcp` gateway as well. |
| `searchKeywords` | object | `{}` | Extra `mcp` search vocabulary: `{ "tool-or-glob": ["keyword", …] }`. |

`settings` keys found in the source:

| Key | Meaning |
|---|---|
| `idleTimeout` | Default idle minutes for `lazy` servers. |
| `scriptMode` | Set to `false` to remove the `mcpScript` tool. Any other value keeps it. |
| `outputGuard` | `false` disables result guarding. An object tunes it with `maxBytes`, `maxLines`, `detailsMaxBytes` (non-positive values fall back to defaults and small values are raised to a floor). The environment variable `ICERS_MCP_OUTPUT_GUARD` (`0/false/no/off`, `1/true/yes/on`) overrides the setting. |

Timeouts worth knowing: starting a server has a 10 s limit (`START_TIMEOUT`),
a call defaults to 60 s, a single stdio line is capped at 16 MiB.

> This table was read from `ServerConfig`, `Config`, `OAuthConfig`,
> `output_guard.rs` and `manager.rs`. Behavior of individual fields (for
> example exactly how `${VAR}` expansion treats a missing variable) was not
> exercised. The safest way to write a valid entry is still `/mcp` (11.2),
> which writes through the same types.

### 11.2 Managing from the TUI

`/mcp` has subcommands including `tools [server]`, `prompts`, `reconnect`,
`enable`, `disable`, `remove`, `auth`, `logout` (autocomplete in
`interactive_tui/autocomplete.rs`). `icers --list-mcp` lists configured servers
and tools from the shell.

### 11.3 Two ways a tool becomes visible to the model

- **Through the gateway.** While any server is configured, the `mcp` tool is
  registered (and `mcpScript`, unless `settings.scriptMode` is `false`). Every
  MCP tool is reachable through `mcp` (`search`, `describe`, then `tool` +
  `args`). One schema stands in for hundreds of tools.
- **Directly.** A tool that `directTools` allows is also registered under the
  name `{server}_{tool}` and carries its own schema in every request.

**The default is direct.** `directTools` defaults to `true`, so *every*
discovered tool of a server is registered directly unless you narrow it. A
server with 80 tools therefore adds 80 schemas to every request. To keep the
tool list short:

```json
{ "mcpServers": { "tickets": { "command": "tickets-mcp", "directTools": false } } }
```

or keep only the hot ones: `"directTools": ["create-ticket", "search-tickets"]`
(these are the remote tool names). The gateway still reaches the rest. The
proxy tests in the tree use `"directTools": false`, which is the configuration
the gateway design assumes. The doc comment on `DirectTools` in `mcp.rs` ("absent
means every tool, since there is no proxy tool") predates the gateway and is
stale.

Invalid or duplicate tool names from a server are rejected
(`"invalid or duplicate MCP tool name"`), and a direct tool whose final name is
already taken is skipped.

### 11.4 Security notes

- Project `mcp.json` is ignored for untrusted projects.
- Responses pass through `mcp/output_guard.rs`.
- Servers run with the user's privileges; vet them like any dependency.
- Call deadlines are enforced (commit `baa11de`).

### 11.5 Chaining: `mcpScript`

For "call tool A for each result of tool B and summarize", do not add a tool.
`mcpScript` runs async JavaScript with `tools.search`, `tools.describe`,
`tools.call(path, args)` (returns `{ ok: true, data }` or `{ ok: false, error }`),
`emit(value)` and `console.log`. Calls run one at a time with the usual
approvals; the script stops after 30 seconds. For a single call, use `mcp`.

### 11.6 When MCP is the wrong answer

- The tool must touch the **session** (todo that persists per session, plan
  state): MCP has no access to it.
- The tool must **render richly** in the TUI: MCP tools get generic rendering.
- Latency or token cost of a gateway round trip matters.

---

## 12. Cross-cutting checklist

Run through this for any new or changed tool, regardless of route.

### 12.1 Naming

- `[a-z0-9_]`, descriptive, stable. A name is part of the persisted transcript
  and of user config (`--tools`, `defaultTools`). Renaming breaks both.
- Do not shadow a built-in unless you intend to replace it (3.3).
- Do not reuse an ice name for different behavior. Users porting from ice
  expect the same contract.

### 12.2 Schema and description

- Derive from a `JsonSchema` struct where you can.
- Describe every field. These strings are model-facing documentation.
- Say what the tool is *not* for, and what to use instead.

### 12.3 Output bounds

- Cap lines, bytes, and per-line length. Say when you truncated and how to get
  the rest.
- Never return secrets. Redact environment and header values.

### 12.4 Cancellation and timeouts

- Honor the `CancellationToken`.
- Give any external call a deadline. The bash default of 120 s exists because a
  forgotten timeout "hung the run forever" (comment in `bash_options`). It
  guards only the foreground, and moves the call to the background rather
  than losing the command: a background shell does not block the run, so it
  keeps just its own timeout.

### 12.5 Replay and idempotence

- Read-only and idempotent: `.with_replay(ToolReplay::Safe)`.
- Everything else: leave the default `Never`.

### 12.6 Token cost

Each *active* tool adds its full schema to every request. Prefer:

- One tool with an `action` enum over five near-identical tools (as `mcp`,
  `bash_watch` and `image` do).
- The `mcp` gateway over direct MCP tools unless the tool is hot. Remember that
  MCP servers register **all** their tools directly by default (11.3), so set
  `directTools` explicitly.
- A short description that is still complete. Cut examples that do not change
  behavior.

Measure with `crate::context_breakdown` (the `/context` view) rather than
guessing. It reports per-category tokens.

### 12.7 TUI presentation

- `ice-tui/src/tool_group.rs` merges consecutive "exploration" calls
  (`read`, `grep`, `find`, `ls`, and read-only shell commands) into a compact
  group via `describe_exploration`. This already handles `grep`/`find`/`ls`
  **by name**, so wiring them (13.1) needs no TUI change.
- `ice-tui/src/tool_render.rs` has per-tool preview lengths
  (`result_preview_lines`; `grep` gets 15) and helpers for `read` and `write`
  titles.
- An unknown tool name gets generic rendering. That is acceptable for most
  tools. A Rust tool cannot supply its own renderer (5.3); to give it one you
  must edit `ice-tui`.

### 12.8 Tests

Rules from `AGENTS.md` §5: no network, no live provider, never touch the real
`~/.icers`, set `ICERS_CODING_AGENT_DIR` to a temp dir, use the faux provider
(`crates/ice-ai/src/providers/faux.rs`).

| Layer | Pattern | Where to look |
|---|---|---|
| Tool logic | `InMemoryExecutionEnv`, call `execute` directly | `ice-tools/tests/ls.rs`, `grep.rs`, `find.rs` |
| Cancellation | abort mid-call | `ice-tools/tests/abort_bracketing.rs` |
| Truncation | oversized input | `ice-tools/tests/read_truncation.rs` |
| Loop integration | a stub `AgentTool` driven by the real loop | `ice-agent/tests/event_sequence.rs`, `steering.rs` |
| Harness e2e | faux provider calling your tool | `ice-harness/tests/harness_run_e2e.rs` |
| Registration | assert the name set and filters | `ice-tools/src/tools/sets.rs` `tests` (pattern to copy). `build_tools` itself has no test yet. |
| Plugin ABI | load the stub cdylib | `ice-extensions/tests/plugin_stub_smoke.rs` |

A tool that is registered in `ice-cli` needs a test that proves it is *in the
built list* and that `--tools` / `--exclude-tools` act on it. That test is what
would have caught `grep`/`find`/`ls` being left out.

### 12.9 Documentation

- User docs live under `docs/`. The model-facing `docs` tool serves only the
  **embedded** copies, generated from `docs/ice-tool/*` by
  `sh scripts/sync-embedded-docs.sh`. CI fails if they drift.
- `docs/guides/*` (including this file) is **not** embedded.
- `docs/ice-tool/extension-authoring.md` is the embedded authoring doc; if a
  tool route changes, change it there and re-sync.

### 12.10 Security review prompts

- Does it read or write outside the project? Through which helper?
- Does it execute a command? Is the string built from model input? Quote it.
- Does it fetch a URL? Allowlist the scheme, refuse credentials in the URL, cap
  the size, set a timeout, do not follow redirects blindly. (The model refresh
  code in `provider_model_refresh.rs` is a reasonable local example of all of
  these.)
- Does it surface third-party text to the model? Treat it as untrusted data.
- Does it need an approval? PreToolUse hooks (`ice-cli/src/hooks.rs`) can veto
  any tool by name, and bash-style tools have settings such as
  `bashInputApproval`. There is no generic per-tool approval field on
  `AgentTool`.

### 12.11 Compatibility

- **MSRV 1.88.** Test dependency or language-feature changes with
  `cargo +1.88 check`.
- **Plugin ABI.** Changing `ice-plugin-sdk` affects every built plugin. Append
  new `PluginApi` slots at the end; never reorder. Call it out explicitly in the
  PR (`AGENTS.md` §7.3).
- **On-disk formats.** A tool that persists state under `~/.icers` or `.icers/`
  needs a documented format and a migration story.
- **TS formats.** Where ice defines a wire or disk format (sessions,
  `models.json`, settings), keep it readable unless the difference is
  deliberate and documented (`AGENTS.md` §8.3).

### 12.12 CI gates

```bash
cargo fmt --all -- --check
RUSTFLAGS="-D warnings" cargo check --workspace --all-targets --locked
RUSTFLAGS="-D warnings" RUSTDOCFLAGS="-D warnings" cargo test --workspace --locked
sh scripts/sync-embedded-docs.sh && git diff --exit-code -- crates/ice-cli/embedded-docs
python3 scripts/test_prebuilt_release.py
```

For a fast loop: `cargo check -p ice-cli` and `cargo test -p ice-tools`.

### 12.13 Repository workflow

Per `AGENTS.md`: branch off an up-to-date `main` (`feat/tools-…`), one logical
change per PR, Conventional Commits (`feat(tools): …`, scope without the `ice-`
prefix), stage explicit paths, read `git diff --cached`, `Co-Authored-By`
trailer for AI-authored commits, squash-merge. Do not publish, tag, or bump
versions.

---

## 13. Worked examples

### 13.1 Wire `grep`, `find` and `ls` (Route A, shape 1)

**Goal:** `grep`, `find`, `ls` become available to the model and to `--tools`.

**Decide first.** icers activates everything it registers (6.5). Wiring three
more tools adds three schemas to every request. Options:

| Option | Behavior | Cost |
|---|---|---|
| **A. Register and activate** | Simple; they appear by default. | +3 schemas per request; the model may use `grep` where it previously used `bash rg`, which is usually an improvement (bounded output, ignore rules). |
| **B. Register, default-inactive** | Matches ice (`read bash edit write` active). | Needs a default-inactive mechanism in `active_tool_names` that survives reload. More code. |
| **C. Do nothing** | Status quo. | The models keeps shelling out for search; `--tools grep` stays unusable. |

Recommendation: **A**, because it is the smallest correct change, and revisit B
only if token cost becomes a measured problem. (This is a recommendation, not a
verified measurement. Run `/context` before and after.)

**Change 1: `ice-cli/src/session.rs`, `build_tools`.** Import the three
constructors next to the existing `ice_tools` imports and add them to the `all`
vector. The three constructors take `(&ExecutionToolContext, Option<…Options>)`:

```rust
use ice_tools::{create_find_tool, create_grep_tool, create_ls_tool};
// …
("grep", HarnessTool::new(create_grep_tool(ctx, None)).with_replay(ToolReplay::Safe)),
("find", HarnessTool::new(create_find_tool(ctx, None)).with_replay(ToolReplay::Safe)),
("ls",   HarnessTool::new(create_ls_tool(ctx, None)).with_replay(ToolReplay::Safe)),
```

`grep`, `find`, `ls` are exported from `ice_tools` at the crate root
(`lib.rs` `pub use tools::{create_find_tool, create_grep_tool, create_ls_tool, …}`).
The options argument is a forward-compat seam with no fields in v1 (the
constructors do `let _ = options;`), so `None` is correct.

**Change 2: system prompt** (`session.rs`), add to "Available tools:":

```
- grep  — Search file contents for a pattern (respects .gitignore)
- find  — Find files by glob pattern
- ls    — List directory contents
```

and, ideally, replace the literal with a list built from the active tools (6.1).

**Change 3: tests.** `session.rs` has no test of `build_tools` today (I found
none by name), which is how the three tools went unnoticed. Add one in
`session.rs`'s `tests` module that builds the list with an in-memory
`ExecutionToolContext` and asserts: the names include `grep`, `find`, `ls`;
`--tools read,grep` yields exactly `read` and `grep`; `--exclude-tools grep`
removes it. `tool_name_allowed` and `filter_active_tool_names` are plain
functions and can be unit-tested directly.

**Change 4: docs.** Update the tool list in `docs/guides/user-guide.en.md` and
the matching `docs/ice-tool/user-guide.md`, then
`sh scripts/sync-embedded-docs.sh`.

**Change 5: housekeeping.** `args.rs` help text says the built-ins are
"read, bash, edit, write, docs" (`--no-builtin-tools` line and the header
string). Update both. Update `ICE_PORTING.md` if it tracks these.

**Proof:**

```bash
cargo test -p ice-cli --lib --locked session::tests
cargo test -p ice-tools --locked
cargo build --release -p ice-cli --bin icers
./target/release/icers --list-mcp >/dev/null   # smoke only
```

then start `icers`, ask it to "find all Rust files that mention ToolName", and
confirm the transcript shows a `find`/`grep` call rather than `bash`.

### 13.2 A new read-only built-in from scratch (Route A)

Follow 8.1 with the skeleton in 8.2. The decisions that matter, in order:

1. Does `ice` have it? If yes, read `~/Zks/ice/packages/coding-agent/src/core/tools/<name>.ts`
   **and its tests**. The tests are the spec (`AGENTS.md` §8.1). Mirror the name,
   parameters and output shape.
2. Which environment view does it need? Read-only → `ExecutionEnv`. Mutating →
   `MutatingEnv` and the queue.
3. Output bound and truncation message.
4. Replay: `Safe` if read-only.
5. Register + prompt line + tests + docs.

### 13.3 A tool as an MCP server (Route D)

To expose, say, an internal ticket system:

1. Run an MCP server for it (stdio command or HTTP).
2. `/mcp` to add it, or write `~/.icers/agent/mcp.json`:
   ```json
   { "mcpServers": { "tickets": { "command": "tickets-mcp", "args": ["--stdio"], "directTools": false } } }
   ```
   (Field reference: 11.1.)
3. Verify: `icers --list-mcp`, then `/mcp tools tickets`.
4. Set `"directTools": false` so the model reaches it through `mcp` (search,
   describe, call), then list only the one or two tools it uses constantly in
   `directTools`. Without this every tool of the server is registered
   directly (11.3).
5. For multi-step workflows, let it use `mcpScript`.

No Rust change, no CI, no prompt edit.

### 13.4 A plugin tool (Route C)

1. `cp -r examples/plugin-stub examples/my-tools` and rename the crate.
2. Replace the `echo` schema (`StableToolSchema`) with yours.
3. Replace `echo_execute` / poll / cancel / destroy bodies; keep the ownership
   protocol and `ToolHandles` as is.
4. Parse params as `serde_json::Value` or a struct without
   `deny_unknown_fields` (10.6).
5. Build: `cargo build -p my-tools` (a `cdylib`).
6. Load it with `icers dev` against the source directory, or via a project
   package path; **not** `icers install` (10.8).
7. Test: copy `ice-extensions/tests/plugin_stub_smoke.rs`.
8. Confirm it registered: RPC `get_available_tools` lists registered and active
   tool names (`/mcp tools` only covers MCP servers, not plugins).

---

## 14. What was not verified

Being explicit about the limits of this guide:

- **No live TUI run.** The "active by default" lists in section 4 are derived
  from reading `build_tools` and the reload path, not from querying a running
  session. A runtime check is `get_available_tools` over RPC
  (`modes.rs`, `rpc_get_available_tools`) or the TUI.
- **Nothing in 13.1 was compiled.** The wiring code is written from the
  verified function signatures, but it has not been built or tested. Treat it as
  a precise plan, not a tested patch.
- **Provider tool-name rules.** I did not audit per-provider limits on tool-name
  characters or length.
- **MCP field behavior.** The field table in 11.1 was read from the config
  types, not exercised. Details such as `${VAR}` expansion of a missing
  variable were not tested.
- **The plugin tool schema struct.** 10.5 documents the *result* JSON read from
  `json_to_result`. The `StableToolSchema` fields were not re-read in full;
  copy them from `examples/plugin-stub`.
- **ice's workgraph tools.** Only two names matched; the real set may be larger.
- **Token cost figures.** None were measured. Use `/context`.
- **Windows behavior** of `powershell` and of path helpers was not exercised on
  this Linux host.
- **`ICE_PORTING.md` beyond the sections cited.** I did not audit it for other
  stale entries; section 15.1 is one I happened to find.

---

## 15. Inconsistencies found while writing this

These are small, real, and cheap to fix. Listed so they are not rediscovered.

### 15.1 `ICE_PORTING.md` §3 says background shells are missing

It says `crates/ice-tools/src/tools/bash.rs` "is foreground-only: it has no
`run_in_background` or `stdin` parameters" and that the registry and tools are
"missing". The code has `bash_background.rs`, `bash_shells.rs`, `bash_watch.rs`,
`ice-cli/src/background_shells.rs`, and `bash_options` wires the registry and
auto-background timer. The port commit `c6c796f` ("mark the tools gaps ported
and document background shells") is in `main`, but it updated `docs/gaps.md`
and the user guide, **not** `ICE_PORTING.md`, so §3 of that file was never
corrected. Mark it ported, and check its other sections against `docs/gaps.md`
before trusting them.

### 15.2 `authoring.md` contradicts itself on unknown fields

§6 item 3 says schema and implementation should "reject unknown fields".
§4 (injected host context) says tool parameter types **must tolerate** unknown
top-level fields because the host injects `__ice`. The second is correct for any
tool that reads host context: a `deny_unknown_fields` struct fails every call.
Reconcile the best-practices list.

### 15.3 `authoring.md` is not in English

`docs/extensions/authoring.md` (569 lines) is Chinese, and so is its copy under
`docs/ice-tool/extension-authoring.md` that is embedded for the `docs` tool.
`AGENTS.md` asks for non-English prose to be translated into English with the
embedded copy regenerated by `sh scripts/sync-embedded-docs.sh`. The model-facing
`docs` tool therefore returns Chinese for extension authoring. This is the
document a tool author is most likely to be sent to.

### 15.4 System prompt names a tool that is not built in

6.1: the `todo` guideline.

### 15.5 `--no-builtin-tools` comment

6.6.

### 15.6 Help text lists the built-ins

`args.rs`: the header says "read, bash, edit, write, docs tools" and
`--no-builtin-tools` repeats the five names. It omits the background-shell
tools that are registered whenever `bash` is. Keep it in step with
`build_tools` when the set changes.

### 15.7 Two meanings of "image tool"

4.5.

---

## 16. Suggested order of work

Per the project's own rule (port gaps users actually hit; no exhaustive parity
runs), and ordered by value over cost:

1. **Wire `grep`, `find`, `ls`** (13.1). Small, already tested, immediately
   useful, makes `--tools read,grep` real.
2. **Make the system prompt tool list derive from the registered tools** and
   make the `todo` guideline conditional (6.1). Prevents a recurring class of
   bug, and fixes a prompt that currently instructs the model to use a missing
   tool.
3. **Fix the doc drift**: `ICE_PORTING.md` §3 (15.1), the `authoring.md`
   contradiction (15.2), the `--no-builtin-tools` comment and help text
   (15.5, 15.6).
4. **Translate `authoring.md`** and re-sync embedded docs (15.3), so `docs` can
   actually teach extension authoring.
5. **A `todo` tool** in-tree or as a built-in-extension, since the prompt
   already depends on it. Smallest of the missing tools. Decide first whether to
   avoid the package-name problem by building it in rather than as a companion
   plugin.
6. **A `delegate_task`-style tool** on top of the existing host `child_agent`
   action (10.9) as a stepping stone toward Subagents V2, if delegation is the
   behavior users actually miss.
7. **Plan/build tools and `access`** (5.2, 6.4) only once the mode state machine
   is being ported, in the same change.
8. **Subagents V2** last; it is the largest single gap and needs its own plan
   (`ICE_PORTING.md` §1).
9. Revisit **default-inactive tools** (6.5) only if measured token cost shows
   the registered-and-active approach hurts.

Open decisions that block parts of this and are not mine to make: plugin package
names and the ABI symbol (`AGENTS.md` §10), and whether `icers` stays the binary
name.

---

## 17. Appendix: file map and commands

### 17.1 File map

| Concern | Path |
|---|---|
| Tool trait | `crates/ice-agent/src/agent_tool.rs` |
| Result and error types | `crates/ice-agent/src/types.rs`, `error.rs` |
| Hooks (`BeforeToolCall`, `AfterToolCall`) | `crates/ice-agent/src/hooks.rs`; command hooks in `crates/ice-cli/src/hooks.rs` |
| `Tool` / `Schema` structs | `crates/ice-ai/src/types.rs` |
| `HarnessTool`, `ToolReplay` | `crates/ice-harness/src/types.rs` |
| Image-normalizing wrapper | `crates/ice-harness/src/image_tool.rs` (applied in `agent_harness.rs`) |
| Built-in tools | `crates/ice-tools/src/tools/*.rs` |
| Tool sets and `ToolName` | `crates/ice-tools/src/tools/sets.rs` |
| Execution env, mutation queue | `crates/ice-tools/src/{env,os_env,in_memory,file_mutation_queue}.rs` |
| Truncation, path helpers | `crates/ice-tools/src/{truncate,path_utils}.rs` |
| Tool assembly, policy, system prompt, reload | `crates/ice-cli/src/session.rs` |
| Flags | `crates/ice-cli/src/args.rs` (`--tools`, `-t`, `--exclude-tools`, `-xt`, `--no-tools`, `-nt`, `--no-builtin-tools`, `-nbt`) |
| Settings keys | `crates/ice-cli/src/settings.rs` (`defaultTools`, `bashInputApproval`, `bashAutoBackgroundSeconds`, `shellCommandPrefix`) |
| Docs tool | `crates/ice-cli/src/docs_tool.rs`, `crates/ice-cli/embedded-docs/` |
| Background shells | `crates/ice-cli/src/background_shells.rs`, `ice-tools/src/{bash_background,bash_watch}.rs`, `tools/bash_shells.rs` |
| Cognee tool | `crates/ice-cli/src/cognee/{command,runtime}.rs` |
| MCP | `crates/ice-cli/src/mcp.rs`, `mcp/*` |
| Plugin ABI | `crates/ice-plugin-sdk/src/lib.rs`, `handles.rs`, `callbacks.rs` |
| Plugin host | `crates/ice-extensions/src/{loader,registry,tool,actions}.rs` |
| Plugin example | `examples/plugin-stub/` |
| Child agents | `crates/ice-cli/src/{child_agents,extensions_actions}.rs` |
| TUI tool grouping/preview | `crates/ice-tui/src/{tool_group,tool_render}.rs` |
| RPC tool listing | `crates/ice-cli/src/modes.rs` (`get_available_tools`, `set_active_tools`) |
| TS reference | `~/Zks/ice/packages/coding-agent/src/core/tools/`, `subagents/v2/extension.ts`, `ice-safe-verify.ts`, `ice-cognee.ts` |
| Gap map | `ICE_PORTING.md`, `ASTRA_GAP_AUDIT_PROMPT.md` |

### 17.2 Useful commands

```bash
# Where is a tool name registered?
grep -rn '"grep"' crates/ice-cli/src crates/ice-tools/src

# Which constructors are unused by the CLI?
grep -rn 'create_grep_tool\|create_find_tool\|create_ls_tool' crates/ice-cli/src

# Every AgentTool implementation
grep -rn 'AgentTool for' crates examples --include='*.rs'

# What tools does a running session expose? (RPC mode)
#   {"type":"get_available_tools"}  →  {"tools":[…],"activeTools":[…]}

# Fast loops
cargo check -p ice-cli
cargo test -p ice-tools --locked
cargo test -p ice-cli --lib --locked session::tests

# Release binary for manual testing (do not cargo-install it as `ice`)
cargo build --release -p ice-cli --bin icers
```

### 17.3 How the facts in this guide were gathered

- `crates/ice-cli/src/session.rs`: `build_tools`, `merge_extension_tools`,
  `tool_name_allowed`, `active_tool_names`, the reload block, the system prompt.
- `crates/ice-tools/src/tools/{mod,sets,tool_context,grep,find,ls,image,docs}.rs`
  headers and constructor signatures.
- Repository-wide greps for `AgentTool for`, `create_*_tool`, `register_tool`.
- `crates/ice-cli/src/{background_shells,docs_tool,mcp*,cognee/*}.rs` for the
  host-registered tools.
- `crates/ice-plugin-sdk/src/lib.rs` (`PluginApi`, constants) and
  `crates/ice-extensions/src/tool.rs` for the plugin ABI.
- `docs/extensions/authoring.md` for the lifecycle and `__ice` rules.
- `~/Zks/ice/packages/coding-agent/src` for the TS inventory (`core/tools`,
  `subagents/v2/extension.ts`, `ice-safe-verify.ts`, `ice-cognee.ts`,
  `core/extensions/types.ts` `ToolDefinition`, `core/agent-session.ts` default
  active tools).
- `ICE_PORTING.md` for the gap sections cited, cross-checked against the code.
