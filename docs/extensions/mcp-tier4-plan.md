# MCP Tier 4 port plan

Status: Phases 0 to 6 are implemented on `feat/cli-mcp-elicitation-url`,
rebased onto `main@743f750` after the Tier 1 `/mcp` panel (#62) merged.
Deferred inside those phases: resource subscriptions, the `-32042` URL
fallback, sampling usage accounting and the Codex `config.toml` importer.
Phase 7 and its Tier 2/3 prerequisites are planned as steps 7.0–7.8. Phase 1 was cherry-picked from `ae868f2`; the
original output-guard branch is unchanged. The plan was first written against
`main@ae1e46b`.

Tier 4 is the "wider MCP protocol" set from the `/mcp` gap review: resources,
prompts, server-initiated requests (elicitation, sampling, instructions),
setup/import, output guarding and result rendering, and the large optional
surfaces (scripting, MCP UI, shared processes). Tiers 1 to 3 (the `/mcp`
overlay panel, the `mcp` proxy tool, metadata cache, lazy lifecycle, HTTP and
OAuth) are separate work. Where a Tier 4 item depends on them, it says so.

## References

Abbreviations used below:

| Tag | What | Where |
|---|---|---|
| **PMA** | `pi-mcp-adapter` 2.32.1, the TS adapter ice uses (MIT, Nico Bailon) | `~/.pi/agent/npm/node_modules/pi-mcp-adapter/` (read-only reference; it is not part of `~/.ice`) |
| **PMA-README** | Its README, the behavioral spec (the package ships no tests) | `PMA/README.md` |
| **RMA** | `rpi-mcp-adapter` 0.2.4, the upstream Rust plugin (MIT) | <https://docs.rs/crate/rpi-mcp-adapter/latest>, source: `https://crates.io/api/v1/crates/rpi-mcp-adapter/0.2.4/download` |
| **MCP spec** | Model Context Protocol, revision 2025-06-18 (the version `icers` sends in `initialize`) | <https://modelcontextprotocol.io/specification/2025-06-18> |
| **icers MCP** | Current Rust client | `crates/ice-cli/src/mcp.rs`, `crates/ice-cli/src/mcp/manager.rs`, `crates/ice-cli/src/interactive_tui/commands/mcp.rs`, docs `docs/extensions/mcp.md` |

AGENTS.md §2.6 still applies: RMA is a reference for reading only. Do not
`icers install` it or depend on it as a crate (it targets `rpi-plugin-sdk`,
not our in-process client).

Line numbers are from the versions above and `main@ae1e46b`; re-check them
before editing.

## Baseline before Phase 0 (`main@ae1e46b`)

What existed in `crates/ice-cli/src/mcp.rs` before this stack:

- `Client::spawn` (`mcp.rs:125`) runs a std-thread reader. Server requests are
  answered inline: `ping` gets `{}`, everything else gets `-32601 Method not
  found` (`mcp.rs:196-205`). Server notifications are dropped (the `continue`
  after the request branch).
- `initialize` (`mcp.rs:303`) sends `"capabilities":{}` and ignores
  `result.capabilities` and `result.instructions`.
- Discovery calls only `tools/list` (`mcp.rs:330`). No `resources/*`, no
  `prompts/*`.
- `McpTool::execute` (`mcp.rs:414`) passes results through `map_result`
  (`mcp.rs:441`) with no size limit. Resource-typed blocks are flattened to
  text; `blob` resources become raw JSON text.
- Approval dialogs go through `UiDialogMailbox`
  (`crates/ice-extensions/src/actions.rs:89`) from `approval_hook`
  (`mcp.rs:578`). The TUI renders `confirm`, `input` and option-select
  dialogs through `AskUserPrompt`
  (`crates/ice-cli/src/interactive_tui/extensions.rs:603`).

Everything in Phase 0 below exists to lift those limits once, so the feature
phases stay small.

## Phase overview

| Phase | Gap # | Item | Depends on | Est. size (excl. tests) | Recommendation |
|---|---|---|---|---|---|
| 0 | — | Bidirectional client foundation | — | ~200 | Do first |
| 1 | 22 | Output guard (+ rendering check) | — | ~250 | Do |
| 2 | 18 | Resources as tools | 0, 1 | ~300 | Implemented |
| 3 | 19 | Prompts as slash commands | 0 | ~300 | Implemented |
| 4 | 20a, 20c | Server instructions + elicitation | 0 | ~400 | Implemented |
| 5 | 20b | Sampling | 0, 4 (dialogs) | ~350 | Implemented (usage label deferred) |
| 6 | 21 | Config import + `/mcp setup` | Tier 1 panel for the setup UI | ~450 | Implemented (Codex importer deferred) |
| 7 | 23 | Scripting, MCP UI, rmcp-mux (+ Tier 2/3 prerequisites) | 7.0–7.5 | ~3,500 | Planned (steps 7.0–7.8) |

Each phase is one PR off `main` (stacked where it depends on Phase 0), in line
with the ~600-line guideline in AGENTS.md §7.3.

---

## Phase 0: bidirectional client foundation

**Why:** Sampling, elicitation, list-changed refresh and resource updates all
need the client to (a) answer server requests asynchronously, possibly after
a user dialog, and (b) act on notifications. The current reader cannot do
either.

**Spec:** MCP spec "Lifecycle" (capability negotiation) and "Base protocol";
PMA `server-manager.ts:1028-1036` (advertised client capabilities),
`server-manager.ts:1069-1086` (handler registration),
`server-manager.ts:878-889` (`getInstructions`), `server-manager.ts:1119`,
`:1138` (list-changed refresh).

**Change (all in `crates/ice-cli/src/mcp.rs`):**

1. Add a per-client `tokio::sync::mpsc::UnboundedSender<Inbound>` where
   `Inbound = Request { id, method, params } | Notification { method, params }`.
   The reader thread forwards instead of answering. `ping` stays inline
   (it must never block). Unknown methods still get `-32601`, now from the
   dispatcher.
2. Dispatcher task per client: owns the handler table (sampling, elicitation)
   and replies through the existing `outgoing` sender. Each request runs in
   its own task so a slow dialog cannot block `ping` or tool results.
   Cancellation: honour `notifications/cancelled` from the server by
   cancelling that request's token.
3. `initialize`: send capabilities only for handlers that are enabled
   (`sampling: {}`, `elicitation: {}` or `{form:{}, url:{}}`), so servers
   never see a capability we cannot serve. Store
   `result.capabilities` (`resources`, `prompts`, `listChanged`,
   `subscribe`) and `result.instructions` on the `Client`.
4. Notifications: route `notifications/tools/list_changed`,
   `.../prompts/list_changed`, `.../resources/list_changed` to a manager
   callback that re-lists only that catalog on the live client (implemented
   as `McpManager::refresh_catalog`, which reuses the `register` / `unregister`
   helpers split out of `reconnect_locked`; no process restart). Prompts and
   resources notifications are ignored until Phases 2-3 add those catalogs.
   `notifications/resources/updated` remains ignored; subscriptions are deferred
   until Tier 2, not part of Phase 2.
5. Bounds stay as they are: `MAX_LINE` framing, `START_TIMEOUT`,
   `CALL_TIMEOUT`. Add a cap on concurrent inbound server requests per
   client (e.g. 8; reply `-32603` when exceeded) so a hostile server cannot
   pile up dialogs.

**Tests (`crates/ice-cli/src/mcp/tests.rs` + fixture server
`crates/ice-cli/tests/mcp_test_server.rs`; `tests/support/mcp_fixture.rs`
only locates its binary):**
- Existing `call_answers_server_ping_and_unknown_requests` must keep passing.
- New: a server request answered after a delayed handler does not block a
  concurrent `tools/call` or `ping`.
- New: a capability is advertised only when its handler is enabled.
- New: a `list_changed` notification triggers a re-list without restarting
  the process.

**Acceptance:** no behavior change for existing users; tests above pass.

---

## Phase 1: output guard (#22)

**Why:** A single large MCP result currently goes straight into the model
context and the session JSONL. This is the highest-impact safety gap in
Tier 4 and needs no protocol work.

**Spec:** PMA-README "Output Guard" (`README.md:462-482`);
`PMA/mcp-output-guard.ts`: limits at lines 7-15, `guardMcpOutput` at 92,
`truncateHead` at 220, `formatTruncationNotice` at 253, `boundMcpResult` at 270,
`summarizeMcpResult` at 303.

**Reuse, don't re-implement:**
- `ice_tools::truncate_head` (`crates/ice-tools/src/truncate.rs:121`) and
  `DEFAULT_MAX_BYTES` / `DEFAULT_MAX_LINES` (50 KiB / 2,000 lines, same as
  PMA's defaults).
- The bash spill pattern in `crates/ice-tools/src/shell_output.rs:402-410`
  (`create_temp_file("bash-", ".log")`) for the full-output file.

**Change:**
- New `crates/ice-cli/src/mcp/output_guard.rs`, applied inside `map_result`
  (`mcp.rs:441`):
  - Text blocks: join, `truncate_head`, write the full text to a private
    temp file (mode `600`), append the notice with the path.
  - Image blocks: pass through unchanged (PMA behavior).
  - `resource` blocks with `blob`: decode up to 10 MiB to a temp file and
    replace with a file reference; per-session caps of 100 MiB / 10,000 files;
    delete on session teardown (PMA-README line 466-467).
  - `details`: keep raw JSON up to 16 KiB, otherwise a compact summary
    (block counts, sizes, key preview) plus a spill file (PMA
    `summarizeMcpResult`).
- Settings in `mcp.json`, top-level `settings.outputGuard`:
  `true` (default) | `false` | `{ maxBytes, maxLines, detailsMaxBytes }`.
  Kill switch env `ICERS_MCP_OUTPUT_GUARD=0` (the `ICERS_` prefix per
  AGENTS.md §2.1, not PMA's `MCP_OUTPUT_GUARD`). Blob materialization limits
  stay on even when the guard is off (PMA behavior).
- `Config` (`mcp.rs:66`) gains an optional `settings` object; unknown keys are
  already ignored, so older files stay valid.

**Rendering (second half of #22):** PMA's compact rows
(`PMA/tool-result-renderer.ts`, `README.md:734`) collapse a result to
one line with a Ctrl+O hint. `icers` already collapses tool output with
Ctrl+O through `ToolExecutionComponent`
(`crates/ice-tui/src/tool_execution.rs:95`). Action: verify an MCP call shows
a bounded input preview and one result line when collapsed. Only port
`formatCompactInputPreview` (`tool-result-renderer.ts:268`) if it does not.
Do not add `toolResultRendering: "boxed"`; there is no legacy row to restore.

**Tests:** oversize text truncated with notice and spill file; image
untouched; blob materialized and capped; guard disabled via settings and via
env; spill files removed on drop; spill file mode `600` (unix).

---

## Phase 2: resources as tools (#18) — done

Implemented on `feat/cli-mcp-resources`, stacked on Phases 0 and 1. Resource
and real-tool definitions are cached separately by kind so notifications only
re-list their target catalog; registration always orders real tools first.
This also handles real tools added after an initially colliding resource.
`directTools` does not exist on this branch and is not introduced here.
Discovery also skips `tools/list` when `capabilities.tools` is absent, so a
resource-only server can connect without implementing the tools API.

**Spec:** MCP spec "Server features / Resources"; PMA
`resource-tools.ts:3` (`resourceNameToToolName`),
`direct-tools.ts:221-260` (registering `read_<resource>` tools),
`direct-tools.ts:492` (`readResource`), `server-manager.ts:1403`
(`listResources` pagination), `server-manager.ts:724` (updated listener);
PMA-README `exposeResources` (`README.md:287`).
RMA `src/runtime.rs:529-575` (`resources/read` and flattening `contents`),
`:577-627` (paginated `resources/list` with repeated-cursor and 100-page
guards), `:433`, `:649`, `:695` (subscribe/unsubscribe),
`:471` (`notifications/resources/updated`).

**Change:**
- Discovery: when the server advertises `capabilities.resources`, page
  `resources/list` with the same repeated-cursor guard used for tools
  (`mcp.rs:330-380`) plus RMA's 100-page cap.
- Register one tool per resource, named `{server}_read_{sanitized name}`
  (PMA naming with our existing `{server}_` prefix). No input parameters.
  Execution calls `resources/read {uri}` and returns through the Phase 1 guard.
  Text contents become text blocks, `blob` contents go through blob
  materialization, image MIME types become image blocks.
- Per-server `exposeResources: bool` (default `true`, as in PMA). Resource
  tools follow the same approval and `--tools` filtering as normal tools
  (`session::tool_name_allowed`, `crates/ice-cli/src/session.rs:2205`).
- Name collisions with a real tool: the real tool wins, the resource tool is
  skipped and logged. This matches the existing duplicate check in
  `McpManager::register` (`crates/ice-cli/src/mcp/manager.rs`).
- `resources/list_changed` (Phase 0) re-lists and re-registers.
- Subscriptions (`resources/subscribe`) are **out of scope** here: without a
  proxy tool there is no model-facing way to subscribe. Revisit with Tier 2.

**Tests:** fixture server gains `resources/list` (two pages) and
`resources/read` (text + blob); assert tool names, guard applied, server
without the capability is not probed, `exposeResources:false` registers
nothing. Additional coverage pins real-tool collision priority, resource additions
and removals, approval cleanup, targeted refresh without process restart,
`--tools` filtering, repeated cursors, the 100-page cap, and PMA sanitization.

---

## Phase 3: prompts as slash commands (#19) — done

Implemented on `feat/cli-mcp-prompts`: paginated capability-gated metadata,
PMA-style prompt names, quoted positional/named arguments, required-argument
validation before server calls, multi-message flattening, and asynchronous
normal TUI submission. A manager watch channel refreshes both slash dispatch
and autocomplete after discovery, reconnect, disable/remove, and targeted
prompt catalog changes. Offline fixtures cover these lifecycle paths.

**Spec:** PMA-README "MCP Prompts" (`README.md:521-532`); `PMA/prompts.ts`:
`resolveCachedPrompts` 19, `parsePromptArgs` 44 (quotes, `key=value`),
`resolvePromptArgs` 140 (required-argument check), `buildUsageMessage` 170,
`formatPromptResult` 185 (flatten with `[user]`/`[assistant]` markers),
`createPromptCommand` 229; `server-manager.ts:1380` (`listPrompts`),
`:1454` (`getPrompt`); `commands.ts:116` (`showPrompts`).

**Change:**
- Discovery: when `capabilities.prompts` is present, page `prompts/list` and
  store `{name, description, arguments[]}` per server.
- TUI: register `/mcp__<server>__<prompt>` commands. Reuse
  `register_extension_commands`
  (`crates/ice-cli/src/interactive_tui/commands.rs:911`) as the model for
  dynamic registration, and re-run it after reconnect and `list_changed`.
- Argument parsing: use `shlex` (already used in `commands/mcp.rs`) for
  positional arguments plus `key=value`; missing required arguments print the
  usage message and do not call the server.
- Execution: `prompts/get {name, arguments}`, flatten messages to one user
  message, submit it as the next prompt (same path as a typed message).
- `/mcp prompts` subcommand lists them; add it to the `/mcp` autocomplete.
- Non-interactive modes (`-p`, RPC): prompts are TUI-only, like extension
  commands.

**Tests:** argument parser cases from PMA (quoted, named, mixed, missing
required); flattening of multi-message results; command set refreshes after
`list_changed`; server without the capability not probed.

---

## Phase 4: server instructions + elicitation (#20a, #20c)

Phase 4 is split into independently verifiable branches to keep the
instructions change separate from elicitation's schema validation, dialogs,
URL consent, and cancellation surface.

### 4a. Server instructions — done

Implemented on `feat/cli-mcp-instructions`. The harness stores server sections
separately from the base prompt and includes them in run, context-inspection,
and token-budget snapshots. Each server section has a 2 KiB UTF-8 byte cap;
the total is capped at 8 KiB, including headers and truncation notices.
Reconnect replaces sections, reload preserves them, and disable/remove or a
detected connection close removes them. `/mcp tools <server>` shows full
instructions with terminal controls removed. Tests cover caps, multibyte text,
reload isolation, and fixture reconnect/disable/remove/disconnect lifecycle.
The public harness API adds `set_mcp_instructions`; no persistence format or
plugin ABI changes.

**Spec:** MCP spec `InitializeResult.instructions`; PMA
`server-manager.ts:878-889`, `proxy-modes.ts:751-757` (preview plus "use
`mcp({ instructions })` for the full text").

**Change:** With no proxy tool yet, append each connected server's
instructions to the system prompt as a bounded section ("MCP server `<name>`
instructions", truncated at e.g. 2 KiB per server with a total cap). Put it
next to the other sections in `crates/ice-harness/src/system_prompt.rs`. Show the full text
in `/mcp tools <server>`. When Tier 2 lands, switch to PMA's preview-plus-tool
model.

**Tests:** instructions captured from the fixture's `initialize`; truncation
cap; disappears on disconnect.

### 4b. Elicitation

Split into independently reviewable changes after the combined implementation
exceeded the ~600 non-test-line target:

- **4b1 — form elicitation: done** on `feat/cli-mcp-elicitation-forms`. Reuses
  `UiDialogMailbox` and the workspace `jsonschema` validator. Flat schemas are
  validated locally with no reference resolution; input, enum/boolean selection,
  multi-selection, omission/defaults, retry, review/edit, decline, and cancel are
  covered. Only TUI sessions advertise `elicitation.form`; settings can disable
  it. Aborted/timed-out client requests cancel all outstanding server handlers
  on that connection, since the protocol does not reliably link them to tools.
  Adds the existing workspace `jsonschema` dependency to the CLI, with no new
  package versions, plugin ABI, persistence format, or public crate API changes.
- **4b2 — URL elicitation: implemented** on `feat/cli-mcp-elicitation-url`, stacked
  above forms. TUI-only `elicitation.url` validates HTTP(S) URLs without embedded
  credentials, asks explicit browser consent, and tracks accepted IDs per
  connection for one-shot completion notices. Tracking is bounded and pending
  reservations are reclaimed on decline/cancel/drop. Offline tests inject the
  browser launcher and cover transport notifications. Reuses the installed URL
  parser and browser launcher; no dependency, ABI, persistence, or public crate
  API changes. The optional `-32042` URL-required tool-error fallback is deferred.

**Spec:** MCP spec "Client features / Elicitation"; PMA-README
`README.md:533-538`; `PMA/elicitation-handler.ts`:
`handleElicitationRequest` 35, `handleFormElicitation` 44,
`collectField` 114, `coerceAndValidateFormValues` 192 (type coercion,
enum, min/max, required), `formatReview` 295 (review/edit step),
`handleUrlElicitation` 305 (consent, host display);
`server-manager.ts:1079-1086` (`notifications/elicitation/complete`),
`:1153-1172` (`-32042` URL-required errors).

**Change:**
- New `crates/ice-cli/src/mcp/elicitation.rs`, registered as a Phase 0
  handler for `elicitation/create`.
- Form mode: walk `requestedSchema.properties` (string, number, integer,
  boolean, enum, enum-array). For each field, raise a dialog through
  `UiDialogMailbox`: `kind: "input"` for free text and numbers, option select
  for enum and boolean, multi-select for enum arrays (`allow_multiple`). These
  map onto the existing `AskUserPrompt` (`interactive_tui/extensions.rs:603`).
  Validate and re-ask on bad input, then show a review step with
  Submit / Edit / Decline / Cancel. Reply `{action: accept|decline|cancel,
  content}`.
- URL mode: advertise only in the TUI. Show the server name, target host and
  full URL, and always ask for consent before opening the browser (reuse
  `open_browser` in `crates/ice-tools/src/utils/open_browser.rs:76`). Track accepted ids for
  `notifications/elicitation/complete`.
- No UI attached (`-p`, RPC without a dialog host): don't advertise the
  capability (Phase 0 rule), so servers fall back cleanly.
- Settings: `settings.elicitation` (default `true` when interactive).
- Security: field text and URLs are untrusted. Strip control characters
  before display, and never auto-open a URL.

**Tests:** schema coercion table (from `coerceAndValidateFormValues`);
decline and cancel paths; capability absent in non-interactive mode; URL
consent required; dialog cancellation when the tool call is aborted.

---

## Phase 5: sampling (#20b) — implemented

**Status:** implemented in `crates/ice-cli/src/mcp/sampling.rs`, stacked above
elicitation. Differences from the change list below:

- The call uses `Provider::stream_simple` on the live provider list shared with
  `/reload` (`McpManager::with_sampling`), with the session's current model read
  from the main lane per request. It does not use a child harness, so it never
  touches the transcript.
- Interactive TUI only, like elicitation: no UI means the capability is not
  advertised, so the "no UI and no auto-approve means denied" case cannot occur.
- The permission dialog is the existing MCP approval dialog, generalized as
  `mcp::confirm`. Settings: `sampling` (default `true`), `samplingAutoApprove`
  (default `false`; an unreadable `mcp.json` leaves both off).
- **Deferred:** counting sampling tokens in `/usage` under a separate label. The
  reply's `usage` is available, but `total_usage` is projected from session
  entries (assistant messages and validated `subagent_usage` custom entries), so
  this needs a similar custom entry and a persistence-format decision (AGENTS.md
  §7.3), which is its own change.

**Spec:** MCP spec "Client features / Sampling"; PMA-README settings
`sampling`, `samplingAutoApprove` (`README.md:419-420`) and limitations
(`README.md:810`: text-only; context inclusion, tools, stop sequences, audio
and images are rejected); `PMA/sampling-handler.ts`:
`handleSamplingRequest` 35, `formatRequestApproval` 95,
`resolveSamplingModel` 127 (`modelPreferences.hints`, then current model,
then default), `confirmSampling` 185, `convertSamplingMessage` 196,
`convertAssistantResult` 232, `mapStopReason` 262.

**Change:**
- New `crates/ice-cli/src/mcp/sampling.rs`, a Phase 0 handler for
  `sampling/createMessage`.
- Model resolution: match each hint name as a substring of catalog model ids,
  then fall back to the session's current model. Call the provider directly
  through `stream_simple` (`crates/ice-ai/src/provider.rs:181`) with no tools
  and `max_tokens` from the request. This is separate from the session
  transcript; `crates/ice-cli/src/side_questions.rs` is the precedent for an
  out-of-band model call.
- Confirmation: before the call, show the server name, model, and the
  request text (truncated); after the call, show the response before
  returning it (PMA's two-step approval). `samplingAutoApprove: true` skips
  both. Without a UI, deny unless auto-approve is set.
- Reject unsupported inputs with explicit errors, as PMA does:
  `includeContext` other than `none`, tools, stop sequences, image and audio.
- Usage and cost: count sampling tokens in session usage under a separate
  label so `/usage` shows them.

**Billing note:** sampling spends the user's model quota on a server's
behalf. Keep the confirmation on by default (decision D3 below).

**Tests (faux provider):** hint resolution and fallback; approval denied
returns an error to the server; unsupported fields rejected; stop-reason
mapping; no UI and no auto-approve means denied.

---

## Phase 6: config import + `/mcp setup` (#21)

**Status:** 6a is implemented for the JSON-based kinds in
`crates/ice-cli/src/mcp/import.rs`; 6b is implemented in
`crates/ice-cli/src/mcp/setup.rs` and
`crates/ice-cli/src/interactive_tui/mcp_setup_panel.rs`.

- **Done:** `imports` in `mcp.json`; cursor, claude-code (including this project's
  `~/.claude.json` section, trusted only), claude-desktop, windsurf, vscode and
  opencode readers; lowest-layer precedence; project-scoped sources gated on
  trust; `url`/remote entries, bad names and unreadable files reported as notes
  in `--list-mcp`; copy-on-edit in `set_disabled`. No new dependency.
- **Deferred:** the Codex importer (`config.toml` needs a `toml` dependency, which
  is not in `Cargo.lock`; it reports `unsupported` today), and showing notes in
  the `/mcp` panel. `source` shows the originating file path rather than the
  `import:<kind>` label, because `ServerView.source` is already a path.
- **Differences from the list below:** Claude Desktop is also read from
  `%APPDATA%` on Windows; the Linux path `~/.config/Claude/...` and the Codeium Windsurf path are
  added; files must be strict JSON, so JSONC comments in `opencode.json` or
  `.vscode/mcp.json` are reported as a parse note.

**Spec:** PMA-README "Import Existing Configs" (`README.md:698-712`);
`PMA/config.ts`: `KNOWN_SERVER_PRESETS` 32, `IMPORT_PATHS` 75-93,
`findAvailableImportConfigs` 208, `getMcpDiscoverySummary` 246,
`loadDiscoveredHostConfigs` 361, `getConfigConflicts` 373,
`mergeConfigs` 510; `PMA/commands.ts:493` (`openMcpSetup`);
`PMA/mcp-setup-panel.ts:697` (`createMcpSetupPanel`, modes `empty`/`setup`).

**6a. Imports (no UI dependency):**
- `mcp.json` gains `"imports": ["cursor", "claude-code", "claude-desktop",
  "codex", "opencode", "windsurf", "vscode"]`.
- Readers per format, from `IMPORT_PATHS`: JSON `mcpServers` for most,
  `~/.claude.json` per-project sections, Codex `config.toml`
  `[mcp_servers.*]` (needs a new `toml` dependency, which is not in `Cargo.lock` today; check MSRV 1.88 with `cargo +1.88 check`),
  OpenCode V1 `mcp` entries.
- Precedence: imported servers are the lowest layer. Our global, then trusted
  project config, override by name. Imported entries show `source: import:<kind>`
  in `/mcp` and `--list-mcp`.
- Trust: project-relative import paths (`.vscode/mcp.json`,
  `./opencode.json`) are read only when the project is trusted, like
  `.icers/mcp.json` today (`load_config`, `mcp.rs:72`).
- **Hard rule:** never read `~/.ice/` or `.ice/` (AGENTS.md §2.1). `ice` is not
  an import kind.
- Imports are read-only. Editing an imported server (enable/disable, Tier 1
  toggles) copies it into `~/.icers/agent/mcp.json` first, which is PMA's
  "will copy to user config on save" behavior (`mcp-panel.ts:467`).
- Only stdio entries are imported until Tier 3 HTTP exists. `url` entries are
  listed as "unsupported (http)" rather than dropped silently.

**6b. `/mcp setup` panel — done:** Centered overlay (sharing the Tier 1
panel's frame) listing detected import sources with server counts, conflicts
in precedence order, skipped-import notes, and the `KNOWN_SERVER_PRESETS`
quick-adds. Enter toggles a kind in the global `imports` list or writes a
preset through the atomic `edit_config`; both reload servers live. `/mcp` opens
it when nothing is configured, like PMA's `empty` mode. Only the stdio preset
(Chrome DevTools) can be added; the five HTTP presets are listed as needing
HTTP. PMA's shared `.mcp.json` scaffold, example config and precedence screens
are not ported: icers has no shared-config layer. Saving `/mcp` panel edits to
an imported server copies it into the global `mcp.json`, like `set_disabled`.

**Tests:** each importer against fixture files in a temp `HOME`
(`ICERS_CODING_AGENT_DIR` set, as existing tests do); precedence; untrusted
project imports ignored; `url` entries reported; copy-on-edit.

---

## Phase 7 and its prerequisites (#23): plan

Phase 7 was deferred because each item sits on work icers does not have yet:
MCP UI needs the Tier 2 `mcp` proxy tool (its `ui-messages` action and turn
trigger), rmcp-mux needs the Tier 3 transport seam, and scripting needs an
embedded JavaScript engine. This plan closes all of it as one stack on
`feat/cli-mcp-tier2-3`, built in this worktree, one reviewable commit per
step. Each step lists its PMA source, what ships, and how it is proven.

| Step | What | PMA source | Est. size (excl. tests) |
|---|---|---|---|
| 7.0 | Transport seam | `server-manager.ts` (transport creation) | ~150 |
| 7.1 | Metadata cache | `metadata-cache.ts` | ~250 |
| 7.2 | `mcp` proxy tool | `proxy-modes.ts`, `search-ranking.ts`, `namespace-tools.ts` | ~500 |
| 7.3 | Lifecycle modes | `lifecycle.ts`, README "Lifecycle Modes" | ~300 |
| 7.4 | Streamable HTTP + SSE | `server-manager.ts`, README "Server Options" | ~500 |
| 7.5 | OAuth and bearer auth | `mcp-auth*.ts`, `mcp-oauth-provider.ts`, `mcp-callback-server.ts` | ~650 |
| 7.6 | rmcp-mux sockets | `unix-socket-transport.ts`, README `:317` | ~120 |
| 7.7 | MCP UI | `ui-session.ts`, `ui-server.ts`, `ui-resource-handler.ts`, `host-html-template.ts` | ~650 |
| 7.8 | `mcpScript` | `mcp-code.ts`, `mcp-script-worker.mjs`, README `:483` | ~400 |

### 7.0 Transport seam
`Client` today owns a child process and its stdio framing. Split the framing
(`send(Value)`, inbound frames into the existing reader/dispatcher) from the
connection so stdio, HTTP and Unix sockets share the dispatcher, pending map,
cancellation and handlers. No behavior change; the existing MCP tests are the
proof.

### 7.1 Metadata cache
Persist each server's last catalog (tools with schemas, resources, prompts,
instructions) to `~/.icers/agent/mcp-cache.json` (mode 600), keyed by server
name plus a fingerprint of its launch config, so a changed command never
serves stale tools. Written after each successful discovery or refresh; read
at startup. The `/mcp` panel shows cached catalogs before a server connects.
Proof: write/read round trip, fingerprint invalidation, corrupt-file fallback.

### 7.2 `mcp` proxy tool (Tier 2)
One `mcp` tool, registered when any server is configured, with PMA's modes:
status (no args), `server` (list), `search` (ranked over name, server,
description and per-server `searchKeywords`, paged by `limit`/`offset`),
`describe`, `instructions`, `tool` + `args` (call), `connect`, and
`action: "ui-messages"` (7.7) / `"auth-start"` / `"auth-complete"` (7.5).
Names resolve fuzzily (`-` vs `_`, missing prefix) with suggestions on a miss.
This makes `directTools` meaningful: a tool that is not direct is now
reachable through the proxy instead of not at all. Proxy calls reuse the
output guard and the per-tool approval list (the approval prompt names the
real target tool). Instructions move from the system prompt to the proxy for
servers with no direct tools, as PMA does. Proof: faux-provider tests for
each mode, ranking order, approval of a proxied call.

### 7.3 Lifecycle modes
Per-server `lifecycle` (`eager`, `lazy`, `keep-alive`, `lazy-keep-alive`),
`idleTimeout` (minutes, plus `settings.idleTimeout`) and `requestTimeoutMs`.
Lazy servers connect on the first proxy or direct call (direct tools come from
the cache) and disconnect after the idle timeout; keep-alive servers reconnect
with bounded backoff (30 s up to 5 min). A lazy server with no cache entry is
connected once at startup to fill it. Default: see D6. Proof: idle disconnect
and reconnect on call with a paused clock; backoff bounds.

### 7.4 Streamable HTTP + SSE (Tier 3)
`url` servers speak Streamable HTTP (POST JSON-RPC, JSON or SSE responses, a
GET SSE stream for server-to-client messages, `Mcp-Session-Id`, DELETE on
close), falling back to legacy SSE only when the endpoint rejects Streamable
HTTP (404/405/406/415). `headers` and `url` support `${VAR}` interpolation;
a missing variable fails before any request. `bearerToken`/`bearerTokenEnv`
with `auth: "bearer"`. Imports stop skipping `url` entries, and the five HTTP
presets in `/mcp setup` become addable. Uses the existing `reqwest`
(`stream` feature). Proof: an in-process HTTP fixture server covering JSON
and SSE responses, the session header, server requests over the GET stream,
SSE fallback, and 401 surfacing.

### 7.5 OAuth and bearer auth
`auth: "oauth"`: protected-resource discovery from `WWW-Authenticate`
(RFC 9728), authorization-server metadata with the issuer check (RFC 8414;
`oauth.authServerMetadataUrl` and `skipIssuerMetadataValidation` as in PMA),
dynamic client registration fallback (RFC 7591) when `oauth.clientId` is
absent, PKCE S256 authorization code with a loopback callback (reusing the
listener in `oauth_codex.rs`) or manual paste of the callback URL for remote
sessions, refresh tokens, and `grantType: "client_credentials"`. Tokens are
stored per D7. Commands: `/mcp-auth [server]`, `/mcp logout <server>`; proxy
actions `auth-start` / `auth-complete`; `settings.autoAuth`. Proof: fixture
authorization server for discovery, issuer mismatch rejection, DCR, code
exchange, refresh, logout.

### 7.6 rmcp-mux sockets
`socket` (mutually exclusive with `command`/`url`, with `~` and `${VAR}`
expansion) connects to an rmcp-mux Unix socket through the 7.0 seam. icers
never starts or stops the mux. Unix only; on Windows the server reports
"unsupported". Proof: a test listener speaking the fixture protocol.

### 7.7 MCP UI
When a called tool declares `_meta.ui.resourceUri`, read that `ui://`
resource and serve it from a loopback HTTP server (token-guarded, reusing the
observe server's request parsing) with the CSP from `_meta.ui.csp`. The host
page is written in this repo (D8) and speaks the MCP Apps postMessage
protocol to the app iframe: initialize, tool calls back to the server (behind
a per-server consent prompt: never / once / always), `ui/message` prompts
and intents (queued for `mcp({action: "ui-messages"})` and waking the agent
like a follow-up), and pushing later results to the open session.
`_meta.ui.visibility` app-only tools stay out of the model's tool list.
`ICERS_MCP_UI_VIEWER=browser|none`. Glimpse native windows are not ported.
Proof: host-page protocol tests against the served page, consent, message
queue and turn trigger, CSP header, token rejection.

### 7.8 `mcpScript`
A default-on `mcpScript` tool (`settings.scriptMode: false` hides it) runs
model-written JavaScript with `tools.search`, `tools.describe`, `tools.call`,
`emit` and a captured `console`, in an embedded engine (D9) with a 30 s
deadline enforced by the engine's interrupt hook and a memory limit. Calls
return `{ok, data}` / `{ok: false, error}` and go through the same lazy
connect, approval and output guard as proxy calls; details carry a `calls`
trace. Like PMA, this is a convenience layer, not an isolation boundary.
Proof: chained calls, infinite loop killed at the deadline, memory cap,
approval inside a script.

### Not ported (non-goals)
Glimpse windows, the npx binary resolver, `requestHeadersCommand`, `!command`
secret values, `bearerTokenStore` (OS keychain), protocol tracing, MCP
2026-07-28 negotiation (`protocolVersion`), agent-plugin and package
manifests, `toolPrefix` variants, and Pi-tool entries in `mcp` search.

### Decisions (D6–D9)
- **D6. Default lifecycle.** PMA defaults to `lazy`. Recommendation: keep
  `eager` as the icers default so existing setups behave the same; `lazy` is
  opt-in per server.
- **D7. Token storage.** PMA uses the OS keychain and fails closed without
  one. Recommendation: `~/.icers/agent/mcp-auth.json` at mode 600, the same
  convention as `auth.json`, so headless Linux works and no new dependency
  is needed.
- **D8. MCP UI host page.** PMA vendors a 288 KB minified AppBridge bundle
  (MCP SDK + Zod). Recommendation: a small hand-written host page for the
  host side of the postMessage protocol; nothing minified is vendored.
- **D9. Script engine.** Recommendation: `rquickjs` (QuickJS: small,
  interrupt hook for deadlines, memory limits; builds C via `cc`, which CI
  already supports) over `boa_engine` (pure Rust but larger and slower) and
  `deno_core` (V8, far too large). Check `cargo +1.88 check` before adding.

---

## Cross-cutting rules for every phase

- **Tests offline only.** Extend the existing fixture server
  (`crates/ice-cli/tests/mcp_test_server.rs`) with resources, prompts,
  and server-initiated requests. Use the faux provider for sampling. Set
  `ICERS_CODING_AGENT_DIR` to a temp dir and never touch `~/.icers`
  (AGENTS.md §5).
- **Untrusted input.** Server names, tool and prompt descriptions, resource
  names, elicitation text and URLs are server-controlled. Strip control
  characters before rendering, as the panel work already does.
- **Config compatibility.** Use PMA's key names (`exposeResources`,
  `settings.outputGuard`, `settings.sampling`, `settings.samplingAutoApprove`,
  `settings.elicitation`, `imports`) so a PMA `mcp.json` reads the same.
  Use `ICERS_*` for env vars.
- **Docs.** Update `docs/extensions/mcp.md` in each PR, and `ICE_PORTING.md`
  (the MCP entry near line 183). Run `sh scripts/sync-embedded-docs.sh` if
  `docs/ice-tool/` changes.
- **Gates.** Each PR runs the AGENTS.md §5 gates. OCR reviews use the
  code-only allowlist (AGENTS.md §2.8-2.9); this plan file is Markdown and is
  excluded.

## Decisions needed from you

- **D1. Order.** Done: 0 → 1 → 2 → 3 → 4 → 5 → 6a → 6b.
  Phase 1 does not depend on Phase 0 and could go first if you want the
  safety fix soonest.
- **D2. Decided:** one tool per resource, PMA style, named
  `{server}_read_{sanitized name}`, gated by per-server `exposeResources`
  (default `true`). A future Tier 1 panel may expose this setting.
- **D3. Sampling defaults.** PMA enables sampling by default with a
  confirmation step. Recommendation: same, confirmation always on unless
  `samplingAutoApprove` is set.
- **D4. Instructions placement.** System prompt (until Tier 2) or only
  visible in `/mcp`? Recommendation: system prompt, capped.
- **D5. Revised (2026-10-26):** Phase 7 is planned above together with the
  Tier 2 proxy and Tier 3 transports it depends on (steps 7.0–7.8).
- **D6–D9:** see "Phase 7 and its prerequisites".
