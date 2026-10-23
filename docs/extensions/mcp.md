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
`/mcp` panel.

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

Text and embedded text resources become text content. Images retain their
base64 data and MIME type. Other content is returned as JSON text. MCP
`isError: true` responses are tool errors.

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
- `/mcp tools`: list tools grouped by server, marking approval-required tools.
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

No HTTP, SSE, OAuth, resource discovery, prompt discovery, sampling, or
server-initiated client capabilities are implemented. Embedded resources in
tool results are supported only as content, not as a resource API. Individual protocol frames
are limited to 16 MiB. Stdout must contain only newline-delimited JSON-RPC 2.0;
servers must send diagnostics to stderr.
