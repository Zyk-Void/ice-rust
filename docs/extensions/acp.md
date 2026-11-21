# ACP: using icers from T3 Code and Zed

`icers --acp` serves the [Agent Client Protocol](https://agentclientprotocol.com)
(ACP v1) on stdin/stdout, so ACP clients such as T3 Code and Zed can drive
icers as their agent.

## How it works

```text
T3 Code / Zed ──ACP JSON-RPC over stdio──▶ icers --acp
                                             │ one child per ACP session,
                                             │ started in the session's cwd
                                             ▼
                                   icers --mode rpc --session-id <id>
```

The `--acp` process loads no model. Each ACP session is served by an ordinary
`icers --mode rpc` child started in the session's working directory. The child
loads exactly what `icers` loads in that directory: `~/.icers/agent` settings,
credentials, models, built-in and extension tools, hooks, skills, prompt
templates, configured MCP servers, Cognee memory, Blackhole compaction,
background shells and subagents. Sessions are stored in the project's normal
`.icers/sessions/` directory, so `icers --resume` in that project also lists
sessions started from an ACP client.

## Setup

Build the binary:

```bash
cargo build --release -p ice-cli --bin icers
```

### T3 Code

In Settings → Providers, add an ACP provider instance and set:

| Field | Value |
| --- | --- |
| ACP source | Local command |
| Executable override | `/home/<you>/Zykairotis/ice-rust/target/release/icers` |
| Arguments | `--acp` |

### Zed

```jsonc
{
  "agent_servers": {
    "icers": {
      "type": "custom",
      "command": "/home/<you>/Zykairotis/ice-rust/target/release/icers",
      "args": ["--acp"]
    }
  }
}
```

Any other argument is passed to every session's child, so you can add flags
such as `--observe` (each session records to the shared dashboard),
`--model <id>`, `--thinking <level>` or `--offline` as extra arguments. Flags
the bridge controls itself (`--mode`, `--print`, `--session*`, `--continue`,
`--resume`, `--fork`, `--no-session`, `--export`, `--connect`, `--server`,
`--list-*`, `--help`, `--version`) are rejected.

## What is supported

| ACP feature | Status |
| --- | --- |
| `initialize` (v1) | Supported |
| `session/new` | Supported. `cwd` must be an existing absolute directory |
| `session/prompt` | Supported. Text, inline base64 images, embedded resources (inlined as context) and resource links. Mixed and image-only prompts are forwarded to the native agent. Resolves only when the native run settles: `end_turn`, `cancelled` or `max_tokens`; failures return an error |
| `session/cancel` | Stops client or wake turns and owned background shells, including after the foreground prompt returns |
| Streaming | Assistant text and thinking chunks; final text is reconciled so no text is lost or duplicated |
| Tool calls | `tool_call` / `tool_call_update` with ACP `kind`, absolute file `locations`, diffs for `edit`/`write`, and the tool's text output |
| `session/request_permission` | Permission dialogs raised in icers (for example MCP tool approvals) are forwarded. Only an explicit selection allows the call; cancel, error or disconnect deny it |
| Model and thinking level | `configOptions` (`category: "model"` and `"thought_level"`) and `session/set_config_option`, applied to that session only. `session/set_model` is also supported. The current model is listed first |
| Slash commands | Advertised with `available_commands_update` and run like the TUI: `/skill:<name>` and `$skill` mentions, prompt templates, extension commands, and the headless built-ins `/compact`, `/model`, `/thinking`, `/tools`, `/context`, `/usage`, `/session`, `/name`, `/cognee`, `/blackhole`, `/version` and `/help`. TUI-only commands (`/settings`, `/tree`, `/export`, `/reload`, …) are not offered; extension commands that open an interactive dialog report that they need the TUI |
| Client `mcpServers` | Accepted but **not started yet** (a warning is printed to stderr) |
| `session/load` | Supported across process restarts. Opens the exact native saved session in its original `cwd`, replays the active branch (including paginated history and tool results) before responding, and restores the saved model and thinking level before returning configuration. Older sessions without selection records use their latest assistant's model. Missing, unreadable, already-open or unrestorable sessions return an error; loading never creates an empty replacement |
| `session/resume` | Not implemented; clients should use the advertised `session/load` capability |
| Images | Supported (`promptCapabilities.image: true`). Inline base64 PNG/JPEG/GIF/WebP supported by the native image detector; malformed data or mismatched MIME types are rejected. The selected provider/model must support vision |
| Audio | Not supported |
| Client fs/terminal methods | Not used. icers runs its own tools |
| Free-form questions from extensions | Cancelled (no ACP mapping yet) |

## Background shells and wake turns

A bash command started with `run_in_background`, or moved to the background by
the runtime, can outlive its foreground prompt. The prompt returns when that
foreground run settles; another prompt can run while the shell continues.

The original `toolCallId` stays `in_progress` when the native bash tool returns
its background-task ID. Its namespaced raw output is:

```json
{"type":"BackgroundTaskStarted","task_id":"shell-id","task_type":"bash","icers":{"backgroundTask":true}}
```

When the shell finishes, a `tool_call_update` for that same `toolCallId` carries
status `completed` or `failed`, the output tail as ACP text content, and:

```json
{"type":"TaskOutput","Result":{"task_id":"shell-id","status":"completed","exit_code":0,"output":"output tail"},"icers":{"backgroundTask":true,"stoppedByUser":false}}
```

This passive terminal update reports the shell result; it does not mean a later
model wake has consumed it. Early completions are retained until the native tool
result supplies the shell-to-tool mapping. Historical replay does not recreate
live background jobs.

Completion notices and watch alerts can start an unsolicited wake turn in the
dedicated ACP child. Text streams normally, followed by a best-effort
`usage_update` when the native context size is known. A wake settlement never
resolves a waiting client prompt. Prompts, compaction and resume queue behind an
active wake before normalization or input hooks run; cancellation prevents a
queued input from reaching the provider.

`session/cancel` also works after the foreground prompt has returned. It cancels
queued client work and the active wake and stops the session's owned shells.
User-stopped results and notices queued before or during Stop are recorded for
the next explicit turn instead of starting another wake. User-stopped passive
terminal markers include `stoppedByUser`. If Stop wins before native admission,
the original notice is retained without invoking its input hooks a second time.

The local/remote TUI retains its existing wake behavior. Ordinary `--mode rpc`,
`--mode json` and print runs keep idle results for the next explicit prompt.
The bash start message, automatic-background timeout description and
`bash_output` interruption guidance describe the actual session behavior:
automatic notification when wakes are enabled, or results on the next user
message plus explicit `bash_output` inspection otherwise.

## Private ACP child coordination

The bridge sets `ICERS_ACP_RPC=1` only on its dedicated `--mode rpc` child. This
is an internal transport opt-in, not a setting general RPC clients need to
enable. It is removed from the environment inherited by shell commands, so a
nested ordinary icers process does not accidentally enter this protocol.
Only the opted-in child's `ready` frame advertises these commands:

| Native command | Purpose |
| --- | --- |
| `kill_background_shells` | Cancel queued client work and the active wake, stop owned shells, and record stopped notices for the next explicit turn |
| `wake_settled_ack {wakeId}` | Release wake admission after the bridge publishes available usage or skips an unavailable snapshot |
| `client_settled_ack {settlementId}` | Release client admission after the ACP response or error has been written |

The normal native prompt `accepted` response still means acceptance, not
completion. A client-origin `agent_settled` has `origin: "client"` and a
`settlementId`; a wake has `origin: "wake"`, `wakeId`, its admitted `runId` and
its actual outcome. Wake metadata is correlated by run ID and emitted at the
harness's ordered `RunSettled` barrier after durable finalization. Suspended
or rejected wake attempts use explicit fallback settlements.

The child holds a shared operation guard through each acknowledgement. The
bridge queries wake usage outside its input pump, which must remain free to
receive the `get_state` response, then acknowledges the wake. For a client turn
it acknowledges after writing the ACP response. A prompt arriving during that
response-only tail waits for the acknowledgement; a truly active concurrent
client prompt is rejected. This preserves content, usage and response order in
both directions between client and wake turns.

Synchronous mutations such as model changes and session forking use an
immediate operation reservation and return a busy error during a run. The
native input reader never waits on an occupied operation guard, so cancellation
and acknowledgement commands remain readable. Closing releases queued
admissions and acknowledgement waiters and begins owned-shell cleanup before
waiting for the run and extension cleanup.

## Large model catalogs and attachments

The catalog comes from the normal icers runtime, including
`~/.icers/agent/models.json`; ACP does not use a separate hard-coded model list.
ACP advertises every configured model in one **Model** selector, with the active
model first. There is no separate catalog/page selector. Clients must accept the
full advertised list; older T3 Code builds cap ACP discovery at 256 models and
need the corresponding client update for larger catalogs. `/model
provider/model-id` also selects directly, including model IDs that contain slashes.

ACP and native RPC frames are limited to 16 MiB, including base64 image data
and text. Send inline image bytes, not image URLs; URL-only image blocks are
rejected without fetching. Images work with ordinary prompts and skill/template
expansion. Built-in and extension slash commands reject attachments instead
of silently discarding them. Image support in ACP does not add vision to a
text-only model.

## Safety

icers has no general tool permission gate yet, in ACP or in the TUI. Built-in
`bash`, `write` and `edit` run without asking, exactly as they do in an
interactive `icers` session. Only dialogs that icers itself raises reach the
client as permission requests. Use ACP only in workspaces where you would run
`icers` directly.

Stdout carries only JSON-RPC frames. Diagnostics, including child process
errors such as missing credentials, go to stderr.
