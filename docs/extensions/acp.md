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

The bridge sets `ICERS_ACP_RPC=1` only for its dedicated native child. This
opt-in adds `compaction_status` RPC events with a per-attempt `compactionId`
and `in_progress`, `completed`, `failed` or `cancelled` status. Completion
comes from the persisted-checkpoint hook, not the legacy JSON-mode activity
flag. Ordinary RPC, JSON mode and the TUI keep their existing event shapes.

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
| `session/list` | Supported for an explicit existing absolute `cwd`. Lists only that project, newest activity first, in pages of 50 with an opaque `nextCursor`. Includes saved names and UTC update times. Reads live sessions without taking ownership or modifying their files; cursors cannot be reused for another project |
| `session/prompt` | Supported. Text, inline base64 images, embedded resources (inlined as context) and resource links. Mixed and image-only prompts are forwarded to the native agent. Resolves only when the native run settles: `end_turn`, `cancelled` or `max_tokens`; failures return an error |
| `session/cancel` | During a turn, aborts that turn only; background shells keep running. With no turn running, stops owned background shells and any wake reply |
| `session/close` | Supported. Removes the live session, releases pending requests, stops its child and owned shells, and releases the saved-session writer lock so load or resume can reopen it |
| Streaming | Assistant text and thinking chunks; final text is reconciled so no text is lost or duplicated |
| Context and cost | `usage_update` follows settled turns and loaded history, reporting current context use and the model's context window, plus cumulative USD cost when positive. Unknown context windows or unavailable state omit the update without failing the turn |
| Tool calls | `tool_call` / `tool_call_update` with ACP `kind`, absolute file `locations`, diffs for `edit`/`write`, and the tool's text output. Live and replayed calls carry `_meta.icers.toolName`; partial and final `rawOutput` preserve the native result and structured details for plan cards and parent subagent rows |
| Todo plans | Successful `todo` results replace the ACP plan while keeping the tool card. Empty lists clear the plan; failed or malformed results leave the current plan unchanged. Saved todo results restore the plan on session load |
| Automatic compaction | Negotiated with `clientCapabilities.session.compaction: {}`. Progress completes only after the native checkpoint and its step record are saved; provider/storage failures, cancellation, extension vetoes and process exit close the same progress item. Manual compaction remains a separate prompt operation |
| `session/request_permission` | Permission dialogs raised in icers (for example MCP tool approvals) are forwarded. Only an explicit selection allows the call; cancel, error or disconnect deny it |
| Model and thinking level | `configOptions` (`category: "model"` and `"thought_level"`) and `session/set_config_option`, applied to that session only, including while a turn runs (the change applies from the next model request). `session/set_model` is also supported. The current model is listed first |
| Manual compaction | `/compact` emits `compaction_update` progress and its final summary when the client advertises `session.compaction`. The same compaction ID closes on success, cancellation or failure. Other clients retain the text summary. Automatic compaction is reported separately (see above) |
| Session title | `session_info_update` publishes the actual saved name after a successful `/name <display name>` and after history replay on `session/load`. Unnamed sessions have no generated title. Native RPC `get_state` includes an additive nullable `sessionName` field; a missing name or unavailable state does not fail the ACP operation |
| Slash commands | Advertised with `available_commands_update` and run like the TUI: `/skill:<name>` and `$skill` mentions, prompt templates, extension commands, and the headless built-ins `/compact`, `/model`, `/thinking`, `/tools`, `/context`, `/usage`, `/session`, `/name`, `/cognee`, `/blackhole`, `/version` and `/help`. TUI-only commands (`/settings`, `/tree`, `/export`, `/reload`, …) are not offered; extension commands that open an interactive dialog report that they need the TUI |
| Client `mcpServers` | Accepted but **not started yet** (a warning is printed to stderr) |
| `session/load` | Supported across process restarts. Opens the exact native saved session in its original `cwd`, replays the active branch (including paginated history and tool results) before responding, and restores the saved model and thinking level before returning configuration. Older sessions without selection records use their latest assistant's model. Missing, unreadable, already-open or unrestorable sessions return an error; loading never creates an empty replacement |
| `session/resume` | Supported with the same saved-session and cwd validation as load. Restores model and thinking configuration without replaying history or rewriting the saved conversation; returns `configOptions` and advertises commands after the response |
| `session/fork` | Supported for an open, idle session in the same `cwd`. Captures the live source's JSONL branch, facts and configuration coherently, creates a new native session ID with the source as parent, then replays the fork before returning. The original child and session file remain unchanged and usable |
| Images | Supported (`promptCapabilities.image: true`). Inline base64 PNG/JPEG/GIF/WebP supported by the native image detector; malformed data or mismatched MIME types are rejected. The selected provider/model must support vision |
| Audio | Not supported |
| Client fs/terminal methods | Not used. icers runs its own tools |
| Questions from extensions (`ask_user`) | Forwarded as `session/elicitation` forms when the client advertises form support. Single and multiple selections, text input and Yes/No confirmations use the same answer format as the TUI. Decline, cancel, invalid answers and client errors cancel the native question; clients without form support also cancel it |

When a question offers choices and allows another answer, the form includes an
optional text field. Entered text takes precedence over selected choices. T3
combines that field with single-choice questions. For multiple-choice questions,
T3 currently shows a separate text field and requires at least one selection
before submitting, even when another answer is entered.

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
resolves a waiting client prompt. Prompts, compaction, resume and inline slash
commands such as `/name` queue behind an active wake before normalization or
input hooks run; cancellation prevents a
queued input from reaching the provider.

`session/cancel` during a client turn aborts only that turn, so stopping an
unrelated reply never kills a server the agent started earlier. With no client
turn running (for example Stop on a waiting background row), it cancels the
active wake and stops the session's owned shells.
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
