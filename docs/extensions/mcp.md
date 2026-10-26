# MCP tools

icers includes a session-local Model Context Protocol client. No plugin or
Node adapter is required. Servers run as local processes over stdio, or are
reached over Streamable HTTP (with the legacy HTTP+SSE fallback).

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
every tool. A tool that is not direct is not in the model's tool list but
stays callable through the `mcp` proxy tool below, and stays listed in the
`/mcp` panel. Resource tools are controlled by `exposeResources` instead and
are not listed in the panel.

### Importing other tools' servers

List tools in `"imports"` to reuse the servers you already configured in
them:

```json
{"imports": ["cursor", "claude-code", "claude-desktop", "windsurf", "vscode", "opencode"]}
```

Imported servers are the lowest layer: an entry in your global `mcp.json`, then a
trusted project entry, replaces an imported one with the same name. If two
imported tools define the same name, the one listed first in `imports` wins.
Within one tool, a later file in the table wins; if its entry cannot be
imported, the earlier entry of that name is not used either.
`/mcp` and `--list-mcp` show the file an imported server came from.

| Kind | Files read | Table |
|---|---|---|
| `cursor` | `~/.cursor/mcp.json` | `mcpServers` |
| `claude-code` | `~/.claude/claude_desktop_config.json`, `~/.claude.json`, `~/.claude/mcp.json` (later wins) | `mcpServers`, plus this project's section in `~/.claude.json` (trusted projects only) |
| `claude-desktop` | `~/Library/Application Support/Claude/claude_desktop_config.json`, `~/.config/Claude/claude_desktop_config.json`, `%APPDATA%\Claude\claude_desktop_config.json` (Windows) | `mcpServers` |
| `windsurf` | `~/.codeium/windsurf/mcp_config.json`, `~/.windsurf/mcp.json` | `mcpServers` |
| `vscode` | `.vscode/mcp.json` (trusted projects only) | `servers` or `mcpServers` |
| `opencode` | `~/.config/opencode/opencode.json`, then `./opencode.json` (trusted projects only) | `mcp` (`type: "local"`; `enabled: false` imports as disabled) |

Imports are read-only: the other tool's file is never written. Disabling or
enabling an imported server, or saving `/mcp` panel changes to it, copies its
entry into your global `mcp.json` first, and that copy then takes over. Values are used as written, so `${VAR}` placeholders
are not expanded, and files must be strict JSON (comments are not accepted).

Remote entries (`url`, Windsurf's `serverUrl`, OpenCode's `type: "remote"`)
import with their `headers`. Anything that cannot be imported is reported
instead of dropped silently: `--list-mcp` prints a `skipped import:<kind>: ...`
line for entries with no command or URL, invalid names or fields, unreadable
files, and unknown kinds. `codex` is accepted but reports `unsupported`, because its `config.toml`
needs a TOML parser the project does not depend on yet. `ice` is not an import
kind.

## Remote (HTTP) servers

Set `url` instead of `command` to reach a server over HTTP:

```json
{
  "mcpServers": {
    "docs": {
      "url": "https://mcp.example.com/mcp",
      "headers": {"X-Team": "${TEAM_ID}"},
      "auth": "bearer",
      "bearerTokenEnv": "EXAMPLE_MCP_TOKEN"
    }
  }
}
```

icers speaks Streamable HTTP (MCP 2025-06-18): each message is a POST, replies
come back as JSON or an event stream, the `Mcp-Session-Id` from `initialize`
is sent on later requests, and a GET stream (when the server offers one)
carries server-initiated requests. If the endpoint rejects Streamable HTTP
(HTTP 404, 405, 406 or 415 on `initialize`), icers falls back to the legacy
HTTP+SSE transport and posts to the `endpoint` the stream names; an endpoint
on another origin is refused. The session is closed with a DELETE.

`url` and `headers` values expand `${VAR}` and `$env:VAR`; a variable that is
not set fails the connection before anything is sent. `auth: "bearer"` sends
`Authorization: Bearer` with `bearerToken` (which also expands variables) or
the value of the variable named by `bearerTokenEnv`. A 401 or 403 is reported
as needing authentication. Setting both `command` and `url` is an error.

### OAuth

`auth: "oauth"` signs in with OAuth 2.1:

```json
{"mcpServers": {"notion": {"url": "https://mcp.notion.com/mcp", "auth": "oauth"}}}
```

Run `/mcp-auth notion` (or `/mcp auth notion`). icers finds the authorization
server from the server's protected-resource metadata (RFC 9728, including the
`resource_metadata` hint in a 401), reads its metadata (RFC 8414) and checks
that its `issuer` matches, registers a client (RFC 7591) unless
`oauth.clientId` is set, and opens the browser for a PKCE (S256) sign-in that
returns to a loopback listener on `127.0.0.1`. Over SSH, copy the address the
browser ends on and run `/mcp-auth notion <that URL>`. The model can do the
same with `mcp({ action: "auth-start", server })` and
`mcp({ action: "auth-complete", server, args: { redirectUrl } })`.

Tokens are stored in `~/.icers/agent/mcp-auth.json` (mode 600), bound to the
server URL they were issued for, and refreshed before they expire. A token
that expires mid-session makes calls fail with an authentication error until
`/mcp reconnect <server>` refreshes it. `/mcp logout <server>` deletes the
stored tokens. Until a server is signed in it shows `failed: needs sign-in`.

| `oauth` key | Meaning |
|---|---|
| `grantType` | `authorization_code` (default) or `client_credentials` (no browser; needs `clientId` and usually `clientSecret`, which expands `${VAR}`) |
| `clientId`, `clientSecret` | A pre-registered client instead of dynamic registration |
| `scope` | Requested scopes |
| `redirectUri` | Exact registered redirect: a loopback `http://127.0.0.1:<port>/path` gets a listener on that port; anything else is completed by pasting |
| `clientName` | Name sent during dynamic registration (default `icers`) |
| `authServerMetadataUrl` | Use this metadata document instead of discovery; its `issuer` must be on the same origin |
| `skipIssuerMetadataValidation` | Disable the issuer check for one misconfigured internal server (weakens mix-up protection) |

Metadata, registration and token endpoints must use HTTPS (plain HTTP is
allowed only for loopback addresses), and redirects are not followed.

## Shared servers through rmcp-mux

To share one server process between sessions, run it under
[rmcp-mux](https://github.com/VetCoders/rmcp-mux) and point each session at
its socket (Unix only):

```json
{"mcpServers": {"memory": {"socket": "~/.rmcp-servers/rmcp-mux/sockets/memory.sock"}}}
```

`socket` expands `~` and `${VAR}`, and excludes `command` and `url`. icers
owns only its connection: closing a session, `/mcp reconnect`, or idling out a
`lazy` server closes the socket and never starts or stops the mux or its
server. Point a socket only at services whose tools, state and credentials you
mean to share.

## The `mcp` proxy tool

While any server is configured, the model also gets one `mcp` tool (subject to
`--tools` filtering), which reaches every server's tools without registering
each one:

| Call | Does |
|---|---|
| `mcp({})` | Server status and tool counts |
| `mcp({ server: "name" })` | List a server's tools |
| `mcp({ search: "words", limit: 12, offset: 0 })` | Ranked search over tool name, server, description and `searchKeywords` |
| `mcp({ describe: "tool" })` | One tool's description and JSON parameters |
| `mcp({ instructions: "name" })` | A server's full instructions |
| `mcp({ connect: "name" })` | Reconnect a server and refresh its catalog |
| `mcp({ tool: "name", args: {...} })` | Call one tool; `args` may also be a JSON string, and `server` narrows the lookup |

Tool names match exactly, then with `-`/`_` and case ignored, then by the
server's own (unprefixed) name; an ambiguous or unknown name returns the
candidates or the closest matches. Proxied calls use the same approval list
and output guard as direct tools; without an approval dialog (print or RPC
mode) a tool that requires approval is refused. Servers that are disabled or
not connected list their cached tools but cannot be called. Per-server
`searchKeywords` adds search vocabulary that the model never sees:

```json
{"mcpServers": {"github": {"command": "github-mcp", "searchKeywords": {"search_code": ["grep"], "*": ["gh"]}}}}
```

## Lifecycle

Each server may set `lifecycle`:

| Value | Behavior |
|---|---|
| `eager` (default) | Starts with the session. |
| `lazy` | Starts on its first call and stops after `idleTimeout` minutes without a request (default 10, or `settings.idleTimeout`). |
| `keep-alive` | Starts with the session; if it exits or fails, it is restarted after 30 s, doubling to at most 5 min between attempts. |
| `lazy-keep-alive` | Starts on its first call, then never idles out and restarts like `keep-alive`. |

A lazy server that is not running is shown as `idle`. Its tools, resources and
prompts come from the metadata cache, so they stay in the model's tool list
and in `/mcp`; calling one (directly, through `mcp`, or as a prompt command)
starts the server first. A lazy server with no cache entry yet starts with the
session once to fill it. `/mcp reconnect <server>` and `mcp({ connect })`
start a lazy server immediately. `requestTimeoutMs` sets a per-server call
timeout. pi-mcp-adapter defaults to `lazy`; icers keeps `eager` so existing
setups behave the same.

## MCP apps (interactive TUI only)

A tool whose metadata names an app (`_meta.ui.resourceUri`, a `ui://`
resource) opens that app in your browser after a successful call, whether the
model called it directly or through `mcp`. icers reads the resource and serves
it from a loopback server on `127.0.0.1` behind a random token, inside a host
page that runs the app in a sandboxed iframe. The app's own CSP comes from
`_meta.ui.csp` (`connectDomains`, `resourceDomains`); everything else is
blocked. Calling the same tool again updates the open window.

The app can call its own server's tools and read its resources. The first
tool call asks once per server and session whether the app may do that, and
tools on `approveTools` still ask every time. Tools marked
`_meta.ui.visibility: ["app"]` are callable only by the app, never by the
model. When the app sends a message (`ui/message`), it is submitted like
typed input (starting a turn, or steering one in progress) and also queued;
`mcp({ action: "ui-messages" })` returns and clears the queue.
`ICERS_MCP_UI_VIEWER=none` prints the window's URL instead of opening a
browser. Print and RPC modes do not open apps. Native (Glimpse) windows are not
supported.

## Metadata cache

After each successful discovery or catalog refresh, a server's tools,
resources, prompts and instructions are saved to
`~/.icers/agent/mcp-cache.json` (mode 600). The `/mcp` panel uses the cache to
list a server's tools before it connects, while it is disabled, or after it
fails. An entry is used only while the server's `command`, `args`, `env` and
`cwd` are unchanged, and entries for removed servers are dropped on a full
reload. Deleting the file is safe; it is rebuilt on the next connection.

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
requests other than `ping`, interactive form/URL elicitation, and sampling are answered with
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
are logged. Resources whose names sanitize alike (`Project Notes`,
`project-notes`) get numeric suffixes in listing order: `read_project_notes`,
then `read_project_notes_2`.

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
Colliding names are first-wins, without replacing built-in or extension
commands; a skipped prompt is logged. A server whose `prompts/list` fails keeps
its tools and has no prompt commands until the next successful listing.

Arguments may be positional, named (`key=value`), or mixed. Shell-style quotes
and escapes preserve spaces; a quoted `=` remains positional. Named values take
precedence over positional values for the same declared argument. For example:

```text
/mcp__notes__daily_brief today topic="important tasks"
```

Missing required arguments show usage without calling the server. The returned
messages become one user prompt, preserving `[user]`/`[assistant]` role markers
for multi-message responses. The expansion is bounded by the output guard
below, like a tool result. Embedded resource text is included; binary images,
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
- Image blocks keep their position. Each image must be valid base64 of at
  most 10 MiB decoded, or it is replaced by an omission notice; this check
  stays on when the guard is off.
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
(`ICERS_MCP_OUTPUT_GUARD=1` forces it on). Values below 1,024 bytes
(`maxBytes`, `detailsMaxBytes`) or 8 lines (`maxLines`) are raised to that
floor, which still fits the truncation notice. Binary resource materialization
and its limits stay active either way. Error results are formatted from their
mapped blocks, so blobs and images never appear in them as base64. Settings changes apply on `/mcp reconnect`.

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

### URL elicitation

The same TUI-only setting enables `elicitation.url`. A URL request displays the
server, message, target host, and full normalized URL, then offers Open / Decline /
Cancel. Only Open launches the system browser; nothing opens automatically.
Only HTTP(S) URLs without embedded credentials or control characters are accepted.
The browser launch is best-effort: a successful launcher spawn returns `accept`,
not proof that authentication or the browser interaction succeeded. A launcher
spawn failure returns `cancel` and shows a notice without exposing the URL.

Accepted `elicitationId` values are tracked only on their current connection.
A matching `notifications/elicitation/complete` displays a notice that the tool
can be retried; it never retries the tool or opens another URL automatically.
Unknown, premature, and duplicate completions are ignored. IDs must contain
1–256 bytes without control characters; at most 128 pending or accepted browser
requests are tracked per connection. Decline, cancellation, and dropped dialogs
release pending IDs; completion releases accepted IDs. Reconnecting resets this
tracking. Tool errors using the optional `-32042` URL-required fallback are not
automatically converted into browser requests.

## Sampling (interactive TUI only)

The TUI advertises `sampling` so a server can ask your model for one completion
with `sampling/createMessage`. Print, JSON, RPC, and `--list-mcp` do not advertise
it. Set `"settings": {"sampling": false}` in `mcp.json` to disable it; reconnect
servers for the change to apply.

A sampling request spends your model quota on the server's behalf, so you confirm
twice by default: first the server, model, the request's system prompt (if
any), and request text (Allow / Deny), then
the model's reply before it is returned. Either Deny returns an error to the
server. `"samplingAutoApprove": true` skips both prompts; it defaults to `false`
and an unreadable `mcp.json` leaves it off.

The model is the first `modelPreferences.hints` name that is a case-insensitive
substring of an available model id, otherwise the session's current model. The
call is separate from the conversation: it sees only the request's messages and
`systemPrompt`, has no tools, and is not added to the transcript. `maxTokens` and
`temperature` are passed through. The reply's stop reason maps to `endTurn` or
`maxTokens`.

Text only. Requests with `includeContext` other than `none`, tools or
`toolChoice`, non-empty `stopSequences`, or image/audio content are rejected with
an explicit error. Sampling tokens are not yet shown separately in `/usage`.

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
    to the file defining each changed server and restarts those servers. An
    imported server is copied into your global `mcp.json` first.
    Esc with unsaved changes asks to Discard or Keep & Close (`y`/`n`,
    Left/Right/Tab, Enter).
  - Ctrl+R reconnects the selected server and keeps the panel open. Ctrl+Y
    copies a failed server's error.
- `/mcp setup`: open the setup panel, which `/mcp` also opens when no server is
  configured. It lists other tools' MCP configs found on this machine with
  their server counts; Enter adds or removes the tool in the global `imports`
  list and reloads servers. Quick add writes a preset to the global `mcp.json`
  (DeepWiki, Context7, Parallel Search, Notion, GitHub and Chrome DevTools;
  Notion and GitHub are marked `sign-in` because they use OAuth). The panel also
  shows server names defined in more than one place, in precedence order, and
  the imports that were skipped. Codex is detected but not importable yet. An
  import listed in the trusted project file has to be removed there.
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

Resource subscriptions are not implemented. The asynchronous dispatcher answers server
requests without blocking tool replies; unsupported methods return
`-32601 Method not found`. At most 64 server messages wait for the dispatcher;
beyond that, reading pauses. When a call is cancelled or times out, open server
requests (such as a dialog) are cancelled only if no other call is still
waiting on that server, because the client cannot tie a request to its call. Tool, resource, and prompt list-change notifications refresh
the targeted catalog without restarting the server. Individual protocol frames
are limited to 16 MiB. Stdout must contain only newline-delimited JSON-RPC 2.0;
servers must send diagnostics to stderr.
