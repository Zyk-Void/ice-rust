# MCP stdio tools

icers includes a session-local Model Context Protocol client. No plugin or
Node adapter is required. Only stdio servers are supported.

## Configuration and trust

Global configuration is `~/.icers/agent/mcp.json`. `ICERS_CODING_AGENT_DIR`
overrides the agent directory, including this file. Project configuration is
`.icers/mcp.json` in the session working directory. Project configuration is
read **only for a trusted project**, using the same trust decision as project
extensions. `--approve` explicitly trusts a project; `--no-approve` does not.
A project entry replaces the complete global entry with the same server name.

```json
{
  "mcpServers": {
    "example": {
      "command": "my-mcp-server",
      "args": ["--stdio"],
      "env": {"EXAMPLE_SETTING": "value"},
      "cwd": ".",
      "disabled": false,
      "exposeResources": true,
      "approveTools": ["write_record"],
      "directTools": ["lookup", "write_record"]
    }
  }
}
```

`command` is an executable, not a shell expression. Arguments are passed
literally. `cwd` defaults to the session working directory; relative paths
resolve against that directory. The child inherits the process environment,
with configured `env` entries overlaid. No credentials or session context are
injected by the MCP client. Servers execute with your operating-system
permissions; install only servers you trust. Avoid putting secrets in project
configuration. A disabled entry need not specify a command and is never spawned.

`directTools` selects which discovered tools are registered, using
pi-mcp-adapter's key and shape: `true` registers every tool, `false` registers
none, and a list registers those **remote** tool names. Omission registers
every tool. icers has no `mcp` proxy tool yet, so a tool that is not direct is
not callable at all; the server still runs and its tools stay listed in the
`/mcp` panel. Resource tools are controlled by `exposeResources` instead and
are not listed in the panel.

## Naming and execution

A server named `example` exposing `lookup` registers `example_lookup` in the
normal session tool registry. Names use ASCII letters, digits, underscores and
hyphens. Collisions with existing tools are skipped rather than shadowed.
The input JSON Schema is passed to the existing agent validation layer.
`--tools`, `--exclude-tools`, and `--no-tools` apply to MCP tools too.

Servers initialize concurrently in the background, with a ten-second total
startup budget per server. They do not delay the TUI launch. Prompt admission
waits for discovery and tool publication, bounded by that budget plus a
one-second scheduling margin, so the first prompt sees discovered tools.
Failed servers do not prevent the session from running. Use `/mcp reconnect`
to retry failed servers or reread MCP configuration. Extension `/reload` retains
registered MCP tools; it does not restart MCP servers.

Tool calls time out after sixty seconds. Aborting a call sends
`notifications/cancelled`; timeout and dropped requests do so as well.
Cancellation is cooperative: a server may ignore it, and remote side effects
cannot be rolled back. A disconnected server produces a tool error instead of
ending the coding session. Server stderr goes to debug tracing, not the TUI.
Session teardown kills the server process group (Unix) or job (Windows),
including descendant processes.

A server that declares the `tools` capability can send
`notifications/tools/list_changed`; `icers` then re-lists that server's tools
and replaces them in the session without restarting the process. Server
requests other than `ping` and interactive form elicitation are answered with
`Method not found`.

Text and embedded text resources become text content. Images retain their
base64 data and MIME type. Other content is returned as JSON text. MCP
`isError: true` responses are tool errors.

## Resources as tools

Servers advertising `capabilities.resources` expose one no-argument tool per
listed resource by default. Set `"exposeResources": false` on a server to
skip resource discovery and registration. Discovery follows pagination, rejects
repeated cursors, and stops with an error if more than 100 pages are needed.

Names follow `{server}_read_{sanitized name}`: non-ASCII-alphanumeric runs
become underscores, leading/trailing underscores are removed, and letters
become lowercase. Empty names become `resource`; names starting with a digit
get a `resource_` prefix. For example, resource `Project Notes` on server
`example` becomes `example_read_project_notes`.

Calling the tool sends `resources/read` with the resource's fixed URI. Text
uses the output guard below, binary blobs are materialized with its limits,
and image blobs become image blocks. Resource tools use the same `--tools`
filter and approval path as normal tools. For an approval list, use the
unprefixed generated name, e.g. `"approveTools": ["read_project_notes"]`.
Real tools take priority over resource name collisions; skipped collisions
are logged. Resources with the same sanitized name are first-wins.

`notifications/resources/list_changed` refreshes only the resource catalog on
the existing server process, adding and removing tools and approval entries.
Tool catalog refreshes preserve resources and real-tool priority. Resource
subscriptions, update notifications, and resource templates are not supported.

## Prompts (interactive TUI only)

Servers declaring the `prompts` capability expose commands named
`/mcp__<server>__<prompt>`. `/mcp prompts [server]` lists them, and slash
completion shows their descriptions. Prompt names preserve ASCII letters,
case, digits, `_` and `-`; other runs become `_`, leading/trailing `_` and
`-` are removed, empty names become `prompt`, and leading digits get `_`.
Colliding names are first-wins, without replacing built-in or extension commands.

Arguments may be positional, named (`key=value`), or mixed. Shell-style quotes
and escapes preserve spaces; a quoted `=` remains positional. Named values take
precedence over positional values for the same declared argument. For example:

```text
/mcp__notes__daily_brief today topic="important tasks"
```

Missing required arguments show usage without calling the server. The returned
messages become one user prompt, preserving `[user]`/`[assistant]` role markers
for multi-message responses. Embedded resource text is included; binary images,
audio, and resources become descriptive placeholders rather than base64 text.
The expansion uses normal history, rendering, prompt hooks, and steering when
a run is active. Server calls run asynchronously; failures are shown locally.

Reconnects and `notifications/prompts/list_changed` refresh dispatch and
completion, removing stale commands. Disable/remove removes the commands too.
Prompt commands are not available in print (`-p`) or RPC mode.

## Output guard

Large tool results are bounded before they reach the model context and the
session file:

- Text output is capped at 50 KiB or 2,000 lines, the same limits as the
  built-in `bash` tool. Larger output becomes a head preview plus a notice
  naming a temp file that holds the full text, so the agent can inspect it
  with `read` or `grep`.
- Image blocks pass through unchanged.
- Embedded binary resources (`blob`) up to 10 MiB are decoded to temp files and
  replaced with a file reference. A session may materialize at most 100 MiB and
  10,000 files; beyond that, the reference says the content was omitted.
- The raw result kept as tool-result details is capped at 16 KiB; a larger
  result is replaced by a summary (block counts, sizes, top-level keys) and
  the raw JSON is saved to a temp file.

Temp files are created with mode `600` in a private (`700`) directory under
the system temp directory, and that directory is removed when the session
ends. A forced quit (second Ctrl+C during a run) can leave it behind. Spilled
output may contain sensitive data.

Tune or disable the guard with a top-level `settings` object in `mcp.json`.
A trusted project file overrides the global value:

```json
{
  "settings": {
    "outputGuard": { "maxBytes": 51200, "maxLines": 2000, "detailsMaxBytes": 16384 }
  }
}
```

`"outputGuard": false`, or the environment variable
`ICERS_MCP_OUTPUT_GUARD=0`, disables text and details guarding
(`ICERS_MCP_OUTPUT_GUARD=1` forces it on). Binary resource materialization and
its limits stay active either way. Settings changes apply on `/mcp reconnect`.

## Server instructions

Instructions returned by `initialize` are appended to the system prompt for
connected servers. Each section is capped at 2 KiB, including its header and
truncation notice, with an 8 KiB total cap in server-name order. UTF-8 boundaries
are preserved and terminal control characters are removed. Instructions are
server-provided context, not a replacement for project trust or tool approval.

`/mcp tools <server>` (or Enter in the server panel) shows the full instructions
with terminal controls removed. Reconnect replaces instructions without duplicate
sections; disable, removal, or a detected closed connection removes them from
future prompt snapshots. An already-running request keeps its original snapshot.
Reloading the base system prompt preserves live server sections. This applies in
TUI, print, and RPC sessions; inspection via `--list-mcp` still only lists tools.

## Form elicitation (interactive TUI only)

The TUI advertises `elicitation.form` for server-initiated `elicitation/create`
requests. Print, JSON, RPC, and `--list-mcp` do not advertise elicitation.
Set `"settings": {"elicitation": false}` in `mcp.json` to disable it; reconnect
applies the setting. A trusted project setting overrides the global value.

Each request first offers Continue / Decline / Cancel. Fields use input,
boolean/enum selection, or enum-array multi-selection. Invalid values are
re-asked, and a final review offers Submit / Edit / Decline / Cancel. Optional
fields can be omitted; explicit defaults can be selected. Display labels are
control-stripped and mapped back to the original enum values. Nothing is sent
as accepted content until Submit. Form values are sent to the requesting MCP
server; do not enter credentials or other sensitive information.

Supported schemas are flat objects with up to 64 fields: strings (including
`enum`/`enumNames` or titled `oneOf` choices), finite numbers, integers, booleans,
and string enum arrays. String length/pattern and email/URI/date/date-time
formats, numeric minimum/maximum, array item counts, and required fields are
validated locally. Each enum allows at most 256 distinct values. Nested objects,
references, unknown keywords, and unsupported formats are rejected without
opening a dialog; schema validation never fetches URLs or files. Aborting or
timing out a client request cancels outstanding server dialogs on that connection;
server cancellation, disconnect, and TUI teardown reclaim dialogs too.

URL elicitation and browser-completion notifications are not implemented yet.

## Approval

`approveTools: true` requires approval for every tool on that server;
`approveTools: ["write_record"]` requires approval for those **remote** tool
names (before the server prefix is added). `false` or omission requires no
additional MCP approval. Project trust does not replace per-call approval.

Approval uses the existing before-tool-call gate and TUI confirmation dialog.
Every matching invocation requires a fresh Allow answer. Deny, cancellation,
or a missing interactive UI blocks execution. Print and RPC modes therefore
cannot run approval-required tools. Existing plugin vetoes still apply.

## Inspection

Run `icers --list-mcp` to initialize configured servers and print their ready,
failed, or disabled status and namespaced tools. It exits non-zero if any server
fails discovery; disabled servers are not failures. It needs no model credentials.
Inspection obeys project trust and does not execute any tools. It starts real
configured server executables, so inspect configuration before trusting it.

## Interactive management

- `/mcp` or `/mcp status`: open the MCP Servers panel, a centered overlay over
  the chat. Each server row shows `●` (all tools direct), `◐` (some) or `○`
  (none), the `direct/total` tool count, an estimated token cost of the direct
  tool schemas, and its status (`disabled`, `connecting`, `failed`, or
  `(not cached)` before the first successful connection). The selected failed
  server shows its error underneath.
  - Typing filters servers and tools by name; `?` searches tool descriptions
    instead. Esc clears the search, then closes.
  - Enter expands or collapses a server (or toggles the selected tool). Space
    toggles a tool, or every tool of the selected server. Ctrl+D toggles the
    server's disabled flag.
  - Toggles are staged until Ctrl+S, which writes `directTools` and `disabled`
    to the file defining each changed server and restarts those servers.
    Esc with unsaved changes asks to Discard or Keep & Close (`y`/`n`,
    Left/Right/Tab, Enter).
  - Ctrl+R reconnects the selected server and keeps the panel open. Ctrl+Y
    copies a failed server's error.
- `/mcp tools [server]`: list tools grouped by server, marking approval-required
  tools. Selecting a server also shows its full initialization instructions.
- `/mcp prompts [server]`: list available prompt slash commands.
- `/mcp reconnect [server]`: reconnect one server or reload all configuration.
  Removed servers lose their tools; newly configured servers are started.
- `/mcp enable <server>` and `/mcp disable <server>`: persist the flag in the
  file defining the effective entry and apply it immediately.
- `/mcp add <name> <command> [args...]` and `/mcp remove <name>`: edit global
  configuration and apply live. Arguments are whitespace-separated, not shell
  expressions. A trusted project entry still overrides a global entry.
- `/mcp edit [project]`: open global or trusted-project configuration with
  `ICERS_EXTERNAL_EDITOR`, `VISUAL`, or `EDITOR` (falling back to nano/notepad).
  Missing files get an empty template. Invalid JSON is reported and not applied.

Edits preserve unknown JSON keys. Newly created configuration files use mode
0600 on Unix. No `/reload` or session restart is needed. While discovery is in
progress, servers show as connecting and tools are not yet registered.

## Integration recipes

See the [context-mode recipe](context-mode.md) for a stdio server, compatible
routing hook, system instructions, and skills.

## Limits

No HTTP, SSE, OAuth, sampling, URL elicitation, or resource subscriptions are
implemented. The asynchronous dispatcher answers server
requests without blocking tool replies; unsupported methods return
`-32601 Method not found`. Tool, resource, and prompt list-change notifications refresh
the targeted catalog without restarting the server. Individual protocol frames
are limited to 16 MiB. Stdout must contain only newline-delimited JSON-RPC 2.0;
servers must send diagnostics to stderr.
