# icers User Guide

This is a guide to icers for users and extension authors. icers began as a Rust reimplementation of Mario Zechner’s [Pi TypeScript SDK](https://github.com/earendil-works/pi), with core capabilities provided as crates and `icers` as a ready-to-use CLI. See the NOTICE in the repository root for source and license information. This document describes the commands and behavior implemented in the current version and ships with `ice-cli`.

## 1. Installation

### Install the CLI directly

Rust/Cargo is required:

```bash
cargo install ice-cli
icers --version
icers --help
```

Install a development version from this repository:

```bash
git clone https://github.com/Zyk-Void/ice-rust.git
cd ice-rust
cargo install --path crates/ice-cli --force
```

`cargo install` installs the executable; it is not the same as `icers install`, which installs Rust dynamic-library extensions built with `ice-plugin-sdk`.

### Run and build from source

This project is a Rust Cargo workspace, not a Node/npm project; there is no `package.json` in the root directory. Run these commands from the repository root:

```bash
# Run the CLI directly
cargo run -p ice-cli

# Pass arguments to the CLI (arguments after `--` belong to icers)
cargo run -p ice-cli -- -p "Summarize README.md"
cargo run -p ice-cli -- --mode json -p "List the crates in the project"

# Build a release version
cargo build -p ice-cli --release
```

After the build, the Windows executable is at `target/release/icers.exe`; on Linux/macOS it is usually at `target/release/icers`. You can also install the local CLI to Cargo’s bin directory and run it directly:

```bash
cargo install --path crates/ice-cli --force
icers
```

For an offline example (no API key or network required), run:

```bash
cargo run -p minimal
```

### First run

Interactive mode:

```bash
icers
icers "Explain the directory structure of the current project"
```

Single-response mode:

```bash
icers -p "Summarize README.md"
icers --mode json -p "List the project risks"
```

Include files in the first message with `@`:

```bash
icers @README.md "Suggest improvements for this project"
```

## 2. Models and credentials

### Credential precedence

When multiple sources are present, they are used in this order:

1. `--api-key`
2. `~/.icers/auth.json` (written by `icers auth login`)
3. The corresponding provider’s `apiKey` in `~/.icers/agent/models.json`
4. Environment variables

Anthropic uses `ANTHROPIC_API_KEY` or `ANTHROPIC_AUTH_TOKEN`; OpenAI-compatible providers use `OPENAI_API_KEY`. Checking credentials does not make a network request:

```bash
icers auth check
icers auth check --provider gateway --json
icers auth login
icers auth logout
```

Do not put secrets in Git repositories, project documentation, or package manifests.

### Custom providers

The default configuration file is `~/.icers/agent/models.json`. You can also set the agent configuration directory with `ICERS_CODING_AGENT_DIR`:

```jsonc
{
  "providers": {
    "gateway": {
      "api": "anthropic-messages",
      "baseUrl": "https://gateway.example.com",
      "apiKey": "sk-gateway-secret",
      "models": [
        { "id": "custom-claude", "name": "Custom Claude" }
      ]
    },
    "openai-gateway": {
      "api": "openai-completions",
      "baseUrl": "https://api.example.com/v1",
      "apiKey": "sk-openai-secret",
      "models": [
        { "id": "custom-model", "name": "Custom Model" }
      ]
    }
  }
}
```

Select a model:

```bash
icers --model gateway/custom-claude -p "hello"
icers --model openai-gateway/custom-model -p "hello"
icers --provider gateway --model custom-claude -p "hello"
```

If `--model` is omitted, icers prefers an authenticated model. When standard Anthropic credentials are present, it uses the built-in default model; if only a custom gateway is authenticated, it selects the first available model from that gateway.

An Anthropic-compatible gateway can also be configured with:

```text
ANTHROPIC_BASE_URL=https://gateway.example.com
ANTHROPIC_AUTH_TOKEN=token
```

### Decision endpoints

Typed decision APIs (TypeSafe System One, OpenAI Decisions, and Inception Decisions) answer explicit Noul/predicate, Choice, and Score questions about one input. They are configured separately from chat providers under `decisionProviders` in `models.json`, so a decision model can never be selected as a chat model:

```jsonc
{
  "decisionProviders": {
    "typesafe": {
      "protocol": "typesafe-systemone",   // or "openai-decisions", "inception-decisions"
      "baseUrl": "https://api.typesafe.ai/v1",
      "apiKey": "$TYPESAFE_API_KEY"
    }
  }
}
```

`baseUrl` includes the version segment; icers appends `/systemone` or `/decisions`. It must use `https`, except for a loopback host (`localhost`, `127.0.0.1`, `::1`) such as a local proxy, because the key is sent as a bearer header. The key is an API key stored in `auth.json` under the same ID, otherwise `apiKey` (a literal, `$ENV`, or `!command`). There is no environment fallback and no OAuth for decision endpoints. Requests are never retried automatically and redirects are not followed. No built-in command sends decision requests yet; the configuration is used by features that call `ice_cli::decisions::decision_client`.

## 3. CLI commands

Common global options:

```text
--provider <name>       Provider name
--model <pattern>       Model ID; supports provider/model and thinking suffixes
--api-key <key>         Override credentials for this run
--base-url <url>        Override the model endpoint
--timeout <seconds>     LLM API request timeout (default: 600)
--thinking <level>      off/minimal/low/medium/high/xhigh/max
--print, -p             Run once and exit
--mode text|json|rpc    Output mode
--continue, -c          Continue the most recent session
--resume, -r            Select a previous session
--session <id|path>     Specify a session
--session-dir <dir>     Specify the session directory
--name, -n <name>       Set the session display name
--tools <list>          Allow only the specified tools
--exclude-tools <list>  Disable the specified tools
--no-tools              Disable all tools
--no-builtin-tools      Disable built-in tools (read/bash/edit/write/docs)
--no-skills             Skip skill discovery
--no-prompt-templates   Skip prompt-template discovery
--no-context-files      Skip AGENTS.md/CLAUDE.md discovery
--no-extensions         Disable extension loading
--extensions-dir <dir>  Also scan this Rust extension directory
--package-only         Load only the current project’s packages/resources; exclude global resources
--project-only         Alias for `--package-only`
--list-models [search]  List available models (optionally filter with fuzzy search)
--offline               Disable network checks at startup
--export <file>         Export a JSONL session as HTML
--tui-mode <mode>       TUI buffer: regular or fullscreen
--system-prompt <text>  Replace the default system prompt
--append-system-prompt <text>  Append to the system prompt (repeatable)
--debug-system-prompt   Print each parsed system-prompt section to stderr
--verbose               Show startup warnings (such as ignored flags)
--server [--port <n>] [--bind <ip>]
                        Headless server (prints a token at startup)
--connect <host:port>   Remote TUI client (no local resources)
--token <token>         Remote authentication token (also accepts ICERS_SERVER_TOKEN)
```

Subcommands:

```text
icers auth login|check|logout
icers events path|tail    Inspect the extension event log (requires ICERS_EVENT_LOG=1)
icers install <crate>
icers uninstall <crate>
icers update                 # Update the icers CLI itself
icers dev [options]          # Develop Rust extensions: compile, watch, and hot-reload
icers dev-local [options]    # Debug only the current Rust extension (isolated mode)
```

### CLI self-update

`icers update` downloads a prebuilt CLI from GitHub Releases by default; local Rust/Cargo is not required.
The version is resolved from crates.io release records; `ice-release.json` provides download URLs and sizes for each platform, plus the SHA-256 hashes of archives/executables.
After validating the target platform, integrity, and actual version, it atomically replaces the current CLI.
On Windows, the update is staged and completed after the current process exits.

`icers update --force` reinstalls the resolved version; `icers update --build` explicitly builds from source with Cargo.
A missing manifest, download failure, or failed verification results in an error and preserves the existing version; it does not fall back to a build.
`--offline` reports cached version information only; it does not download or build. If an older Release has no manifest,
wait for the protocol assets to be added, or explicitly run `icers update --build`.

### Scrolling through TUI history

The interactive TUI uses the alternate screen by default and manages transcript scrolling itself. Browse messages with `PageUp` / `PageDown`, `Home` / `End`, or the mouse and trackpad scroll wheel.

On macOS, this differs from native terminal scrollback on the main screen: an alternate-screen TUI must receive mouse-tracking events for the scroll wheel and trackpad to scroll through history. After entering raw mode, the TUI explicitly re-enables mouse tracking in alternate-screen mode. In main-screen mode it keeps mouse tracking disabled so native terminal scrolling and text selection remain available.

If the scroll wheel cannot scroll through history on macOS, check that `TuiAltScreen::start_readerless` initializes mouse tracking for the alternate screen; do not change the global macOS default directly.

### Local sessions and context

`/new` (alias `/clear`) creates a new persistent session instead of only clearing the screen. Before switching it waits for the old run and UI events to finish; it refuses to switch while a user shell command is still running, so the result is not written to another session.

`/tree` offers three choices: no summary, a branch summary, or a branch summary with custom instructions. Escape in the custom editor returns to the options, and another Escape returns to the tree view. Cancelling keeps the draft and queued prompts. `branchSummary.skipPrompt: true` skips the prompt and produces no summary; `branchSummary.reserveTokens` controls the summary reserve budget (default 16384). If a summary was persisted but a later record write fails, the error says navigation was committed; retrying or reopening the session recovers the records without asking the provider to generate the same summary again.

`/context` shows the current model capacity, a 20×10 category grid, and the five largest estimated items. It reads only the current branch, uses the standard compaction projection, and counts project instructions, the skills listing, and enabled tool schemas from the loaded prompts. The total uses the latest valid provider input and cache usage plus estimated output and following messages. Categories are estimated independently, and private per-request transformations are outside the public API projection, so this is not an exact request token count. `/usage` still shows cumulative billed usage.

### Blackhole compaction

Blackhole replaces the model-written compaction summary with a deterministic one. It compiles the summarized history into sections: session goal, user requests, findings, files read and changed, commits, preferences, recent actions and outstanding context. It also rebuilds live state (active goal, mode and plan, subagents, background shells, external tools) from the session entries at every compaction. No model call is needed, unless you turn on `refine`, which adds one short model-written "state of work" section.

Blackhole is off until you configure it. Run `/blackhole percent 85` (or any other `/blackhole` subcommand that saves a setting) to create `<agent dir>/ice-blackhole/ice-blackhole-config.json`, or set one of the environment variables below.

- `/blackhole` or `/blackhole status` shows the mid-run mode, threshold, tail, trimming, refine state and config path.
- `/blackhole percent <1-99>` compacts once input reaches that share of the active model's context window. The share is resolved again when you switch models.
- `/blackhole resume`, `/blackhole pause` and `/blackhole off` set mid-run compaction. `resume` compacts after a tool turn and continues, `pause` compacts and stops the run, and `off` turns Blackhole compaction off.
- `/settings` has a Blackhole section: compaction (`auto`, `manual`, `off`), engine (`blackhole` or `ice-default`), mid-run mode, threshold, tail (`minimal` keeps no tail; `ice-default` keeps the normal one), trimming of large kept tool output (model context only; the session keeps full output), refine, and memory.
- In `manual` mode, a bare `/compact` uses Blackhole. Overflow recovery always keeps the normal tail.
- Environment overrides: `ICERS_BLACKHOLE_COMPACTION`, `ICERS_BLACKHOLE_COMPACTION_ENGINE`, `ICERS_BLACKHOLE_MID_RUN_COMPACTION`, `ICERS_BLACKHOLE_COMPACT_AFTER_PERCENT`.

### Cognee memory

[Cognee](https://github.com/topoteretes/cognee) is an external memory service. When it is configured, icers:

- recalls memory for each prompt of four or more words and stores it, marked as untrusted reference data, as a hidden message right after that prompt, so the system prompt never changes and the provider can keep caching the conversation;
- stores each prompt with its final answer, and traces of the built-in coding tools (at most 24 per prompt), in the Cognee session cache. A trace records the action: tool, arguments, status, errors and at most 600 characters of a command's output, never the contents of files that were read or searched;
- asks Cognee to improve (consolidate) the session cache once the agent settles (at most every 5 minutes) and at shutdown, and improves up to two older sessions per idle pass;
- queues each compaction checkpoint as permanent memory (`agent_actions`) and shows it once to the next prompt.

The session only gains small `ice-cognee` operation records (IDs and hashes, no memory text). Credential-like text (`Bearer …`, `api_key=…`, `token: …`) is redacted before it leaves the machine. This is a heuristic, not data-loss prevention: other secrets in prompts or tool output can still be sent.

Cognee is off until you configure it: run `/cognee on`, which writes `<agent dir>/ice-cognee/config.json`, or set any `ICERS_COGNEE_*` variable. icers never contacts Cognee while running offline (`--offline` or `ICERS_OFFLINE=1`). The default server is `http://127.0.0.1:8211` and the default dataset is `$project`, one dataset per repository (`ice-<repo>-<hash>`); set the dataset to a fixed name such as `ice` to share memory across projects. The API key comes from `COGNEE_API_KEY`, then `<agent dir>/ice-cognee/api_key.json` or `~/.cognee-plugin/api_key.json` (used only when a stored `base_url` has the same origin as the server), then `~/.cognee/.env`.

- `/cognee` or `/cognee status` shows health, policy, counters, queue and circuit-breaker state. `/cognee doctor` shows the same as one diagnostic line.
- `/cognee on|off`, `recall on|off`, `capture on|off`, `tools on|off`, `improve on|off` and `remember on|off` change policy. `/cognee compact auto|defer|own` chooses who writes compaction summaries: `auto` and `defer` keep the icers or Blackhole checkpoint; `own` writes a local anchor without a model call.
- `/cognee search <query>` searches memory. `/cognee remember [user_context|project_docs|agent_actions] <text>` stores a fact; when Cognee is unreachable it is queued.
- `/cognee improve [now]` requests consolidation immediately. `/cognee flush [pending|uncertain]` replays up to four queued writes. A write that timed out may already have landed, so it is marked uncertain and replayed only by `/cognee flush uncertain`.
- The model gets a `cognee_search` tool while Cognee is enabled, and three skills (`cognee-search`, `cognee-remember`, `cognee-sync`).
- `/settings` has a Cognee section with the same switches, plus `checkpointRecall`: when Blackhole is active, a bounded recall for the summarized request is written to `<agent dir>/ice-cognee/compaction-recall.json` and added once to the next Blackhole checkpoint.
- The footer shows `cognee` when enabled and `disabled` otherwise. When Cognee recalls, saves or improves, a small memory logo `◉` appears in the bottom-right corner of that prompt in the transcript. Its colour shows how it went: accent while Cognee is still working, green when done, red when a step failed, grey when everything was skipped; it highlights on hover. Click it to open a panel on the right side of the message, shown in full and wrapped: every recalled memory, the saved prompt and answer, every captured tool trace and each improve pass. Click again to close them. Every prompt keeps its own badge. Activity is also logged to `<agent dir>/ice-cognee/observations.jsonl` (rotated at 2 MiB).
- Environment overrides: `ICERS_COGNEE_ENABLED`, `ICERS_COGNEE_RECALL`, `ICERS_COGNEE_REMEMBER`, `ICERS_COGNEE_CAPTURE`, `ICERS_COGNEE_CAPTURE_TOOLS`, `ICERS_COGNEE_IMPROVE`, `ICERS_COGNEE_BASE_URL`, `ICERS_COGNEE_DATASET`, `ICERS_COGNEE_COMPACTION_SUMMARY`, `ICERS_COGNEE_CHECKPOINT_RECALL`, plus Cognee's own `COGNEE_BASE_URL` and `COGNEE_LOCAL_API_URL`.

### macOS keyboard and plugin panels

Terminals that support the Kitty keyboard protocol use the same flags 7 as Pi, enabling modifier-key recognition and key-repeat and key-release events for function keys. Regular text retains UTF-8 input so input methods in terminals such as iTerm2 can submit text correctly; flag 8, which converts all text into key events, is not enabled. Key-release events for ordinary character keys (such as Space) are not guaranteed, so plugins that depend on key holds should use function keys for which the terminal supports release events. The protocol is enabled after switching to the active screen and restored before exit or suspension; full-screen mouse scrolling is also re-enabled on resume.

The protocol is also enabled by default on macOS because `Shift+Enter` relies on it to distinguish that key from `Enter`; terminals whose input method cannot enter text with the protocol enabled (such as Otty) fall back to traditional input. Set `ICERS_KITTY_KEYBOARD=1` to force it on, or `ICERS_KITTY_KEYBOARD=0` to force traditional input.

In the local Terminal.app, `Shift+Enter` uses the native macOS Shift state to recognize a newline, while regular `Enter` still submits. SSH sessions do not read the local machine’s Shift state. Terminals without enhanced keyboard protocol support continue to use traditional input; features such as key-hold shortcuts require terminal support for key-release events.

When a plugin input field, editor, selector, or history search is open, it receives input first; plugin-wide shortcuts do not steal keystrokes from these windows. Keys already handled by a plugin still receive their corresponding release events, preventing a held-key state from sticking. Overlays are laid out to fit their actual width and avoid the input field and footer. Multiple passive panels automatically avoid overlapping vertically when space permits; panels that do not fit are temporarily hidden when the window is too small. To prevent overlays from covering chat text, close the panels or use the plugin’s sidebar layout.

### Remote mode (`--server` / `--connect`)

Run the agent on a headless server and control it remotely from another process using the TUI. The client has **no local agent resources**.

```bash
icers --server --port 9899          # Server: prints a token at startup
icers --connect 127.0.0.1:9899 --token <token>   # Client

# Or pass the token through an environment variable
export ICERS_SERVER_TOKEN=<token>
icers --connect 127.0.0.1:9899
```

Token authentication is enabled on the server by default (`--no-token` disables it). The client supports commands such as `/state`, `/model`,
`/thinking`, `/tools`, and `/abort`; `/tree`, `/fork`, `/switch`, and `/export`,
`/name`, and `/reload` require the local harness and are unavailable in remote mode. For the full protocol, available commands, and
limitations, see [`remote-mode.md`](remote-mode.md).

### Run history and the observe dashboard (`--observe`)

Every run records a read-only history of its activity (prompts, assistant
replies, tool calls with arguments, results and timing, retries, compaction,
and token and cost totals) to `~/.icers/agent/observability.sqlite`. The file
is private (mode `0600`), lives apart from session storage, and keeps runs for
30 days. Credential-shaped values (Bearer tokens, `api_key=…`, `"password": …`)
are redacted before they are written. If the file cannot be opened, icers prints
a warning and keeps running.

```bash
icers --observe          # dashboard on http://127.0.0.1:4659
icers --observe=5000     # pick a port (the `=` form is required)
```

`--observe` prints a dashboard URL that carries a per-user token, stored in
`~/.icers/agent/observe.token`. The server listens only on `127.0.0.1` and accepts
no commands. If another icers already serves the port, the URL points at that
dashboard, which shows every recently active run. In the dashboard, press
`0`–`9` to filter by kind, `Space` to pause or resume auto-scroll, arrow keys
to move between rows, and `Esc` to close the inspector. See
[`docs/guides/observability.md`](../guides/observability.md) for the HTTP routes.

## 4. Built-in tools

The default tools are Pi’s `read`, `bash`, `edit`, and `write`, plus icers’ built-in read-only `docs` documentation lookup tool. `powershell` is also registered by default on Windows. `docs` can search the user guide, extension development material, Agent project structure, Rust debugging, and architecture notes. `grep`, `find`, and `ls` remain available as library implementations but are not registered by the CLI by default.

You can use the `defaultTools` field (or its shorthand `default_tools`) in `.icers/settings.json` to limit the initial tool set when `--tools` is not specified:


```json
{
  "defaultTools": ["read", "bash", "edit", "write", "docs"]
}
```

To restrict the available tools, explicitly list Pi’s four tools:

```bash
icers --tools read,bash,edit,write,docs -p "Inspect and modify project files"
```

### Background shells

When `bash` is active, the session also registers the following tools. `--exclude-tools` can still remove them.

- `bash_output` lists shells or reads a shell's new output.
- `bash_kill` stops a shell.
- `bash_input` writes to a shell's stdin.
- `bash_watch` sets an alert for a pattern, for silence, for a prompt, or on an interval.

The model starts a background shell with `run_in_background: true`. Press `Ctrl+B` to move a running foreground command to the background. The agent is told when a shell finishes or a watch fires.

Model bash calls receive the session's identity in `ICERS_SESSION_ID`, `ICERS_PROVIDER`, `ICERS_MODEL` and `ICERS_REASONING_LEVEL`. Commands you type as `!cmd` run with the same `shellCommandPrefix` as the model's bash tool.

Settings (in `settings.json`):

| Key | Default | Meaning |
|---|---|---|
| `bashInputApproval` | `"ask"` | `"allow"` lets `bash_input` write without asking |
| `bashAutoBackgroundSeconds` | unset | Move a foreground command to the background after this many seconds |
| `bashPromptAlertSeconds` | `30` | Alert when a quiet shell's last line looks like a prompt; `0` disables |
| `bashSilenceAlertSeconds` | `0` | Alert when a shell produces no output for this long; `0` disables |

## 5. Project directories and resource precedence

icers prioritizes its own directories while also supporting Pi’s original `.pi` layout:

```text
Project/
├── .icers/
│   ├── settings.json
│   ├── SYSTEM.md
│   ├── APPEND_SYSTEM.md
│   ├── skills/
│   ├── prompts/
│   ├── themes/
│   ├── extensions/
│   └── packages/
└── themes/
```

Project `.icers` takes precedence over project `.pi`. Global resources are located in `~/.icers/agent/` by default and include `settings.json`, `models.json`, `skills/`, `prompts/`, `themes/`, `extensions/`, and `packages/`. When resources have the same name, project resources take precedence over global resources, and `.icers` takes precedence over `.pi`.

Project-level `.icers/settings.json` can add resource directories:

```json
{
  "skillDirs": ["./team-skills"],
  "promptDirs": ["./prompts/shared"],
  "extensionDirs": ["./target/debug"],
  "packages": ["./packages/review-tools"]
}
```

Paths are relative to the project root; `skills`, `prompts`, and `extensions` are shorthand for the corresponding `*Dirs` fields. Custom directories are loaded along with `.icers`, `.pi`, and the conventional global directories; `.icers` takes precedence. The global `~/.icers/agent/settings.json` supports these fields too, with relative paths resolved against the agent directory.

A leading `~` or `~/` expands to your home directory. `.agents/skills` directories from other tools are not discovered automatically; to use them, list them explicitly, for example `"skillDirs": ["~/.agents/skills"]`.

To load only the current project’s packages/resources and exclude global resources, use either of these equivalent options:

```bash
icers --package-only
icers --project-only
```

Both options load the current project’s skills, prompts, themes, extensions, and packages, and read the project’s `.icers/settings.json`; global resources are not loaded automatically. `--local-only` has different semantics: it loads only extensions explicitly specified with `--extensions-dir` or `--extension`.

The configuration directory can be relocated:

```bash
ICERS_CODING_AGENT_DIR=/work/ice-agent icers
```

Sessions are saved to `.icers/sessions/` in the current working directory by default. Use `--session-dir` (or the `sessionDir` setting) to specify another directory, or `--no-session` to use a temporary session. The chosen directory is used consistently for `--resume`, `/session`, and session switching.

Interactive session operations:

- `/new` (alias `/clear`): Wait for the previous operation to finish, then create and activate a new persistent session; the previous session file and history are preserved.
- `/import <path.jsonl>`: Validate the native v4 session header and entire log, then install and switch to the session ID in the header. Existing files are never overwritten and the source file is not modified. Duplicate IDs, corrupt logs, and v3 files produce explicit errors. The target filesystem must support hard links so an incomplete copy cannot appear as an available session.
- Markdown/HTML exports and `/share` show only the current `main` branch (including compaction summaries). The raw JSONL entry archive still includes all branches, but omits the session header and recovery records, so it cannot be imported as a restorable session file. The SDK’s `ice_cli::export::export_native_branch_jsonl` can export the current branch as a reopenable native v4 session: it uses a new session ID, preserves compacted context, the name, and branch-local labels, and refuses to overwrite any existing target. It does not convert coding v3 format; exports and shares also do not automatically remove secrets or other sensitive text. `/export native` writes this reopenable branch export to `<name>.native.jsonl` in the working directory. `/export md|html|jsonl|native` never overwrites an existing file; choose another name or remove the old file first. `/share` does not send anything until you run `/share confirm`: it first states that the active `main` branch transcript will go to an external service and asks you to review it for secrets.
- `-r` and `/session` list healthy sessions and also show how many files were skipped for an unreadable or malformed header, with their paths.
- The footer context readout follows the active branch after fork, switch, tree navigation and compaction; `~` marks an estimate and `?` means unknown until the model replies after compaction.
- Native JSONL sessions use exclusive OS locks when created, opened, repaired, written, and deleted; a second independent write handle returns an explicit error. The lock is released when the last shared storage handle is dropped, or by the OS if the process crashes. Keeping the `.lock` file is normal; do not delete it to forcibly take over. This guarantee depends on lock support from the local filesystem and on writers respecting the lock; hard-link aliases and distributed filesystems are not covered.
- Duplicate sequence numbers are readable only when their decoded contents are identical; conflicting contents, identities, branches, or parent nodes cause the session to be rejected while preserving the original file. Conflicting entries are no longer silently discarded and parent links are not reset; a trailing partial line after a valid prefix can still be repaired. Recovery after process termination has been tested, but power-loss durability and `fsync` guarantees are not promised. Failure to perform a required final save returns an error instead of reporting success.
- When a coding session receives an explicit context-overflow error, it attempts at most one compaction of older history and one continuation request; the latest user turn is preserved in full and is never replaced by a summary. If summarization fails, is cancelled, or the context still overflows, the run stops and suggests shortening the input, switching to a model with a larger context, or starting a new session. Failure or cancellation during pre-prompt compaction also preserves newly accepted input and does not make an extra model request to continue. By default, the safety net compacts and continues; explicit `pause` mode displays `compaction_paused` and does not mark the task as completed normally.
- Automatic compaction estimates for coding sessions include the composed system prompt and active tool schemas. The larger of the new estimate and historical provider usage is used, avoiding double-counting prompt overhead that was already included. The input threshold also reserves space for the current model’s shared provider output limit and is recalculated after a model switch. This is a UTF-16-length heuristic, not an exact tokenizer; hooks and custom message conversions are not run in advance; when any are configured, the compaction threshold holds a bounded reserve (10% of the model window, at most 16,384 tokens) as an estimate, so content they add beyond that is not guaranteed to be included, and billing usage is unchanged. If the latest turn plus known fixed overhead already cannot fit, no futile overflow-recovery summary request is made.
- `/model`, `/thinking`, and keyboard shortcuts change only the current session; only the default model/thinking-level selection in `/settings` is saved as the default for the next launch. The display updates only after the runtime accepts a change; an active operation, unregistered provider, or unsupported thinking level returns an explicit error. Failure to save a default does not undo a session choice that was already applied, but a save error is shown. Resuming history does not force a switch to the historical model; the model selected at launch remains in use.
- `/unqueue [n]` moves one queued steering/follow-up message (1-based, in dock order; the newest when omitted) back into the editor and leaves the others queued; Alt+Q restores all of them. A message that was already sent to the model is reported as such and is not duplicated. Images attached to that message return to the editor after the queue entry is durably cancelled, alongside any images already in the draft; if the saved message cannot be read, it stays queued and an error is shown.
- When a transient provider failure is not retried automatically because the failed response contained a tool call or tools had already run, the transcript says so; use the manual retry only if you are sure replaying is safe. `/usage` also shows all-history token totals and provider-reported cost, and labels active-context occupancy as an estimate (unknown right after compaction until the model replies).
- A cancellation request and operation completion are distinct states: the underlying `abort` requests cancellation. If a compaction checkpoint has not yet been written, cancellation prevents it from being published. If storage has acknowledged the checkpoint write, the checkpoint is retained and is not rolled back, but the current run/manual compaction settles as aborted and makes no subsequent model request. Session switching waits for the old operation to finish and preserves the old session if that fails. SDK `AgentSession::retry()` explicitly resubmits the last user message and may execute tools or external operations again; it is not safe or exactly-once automatic replay.

## 6. Rust plugins

Native Rust plugins are `cdylib` libraries that depend only on `ice-plugin-sdk` and are dynamically loaded by `ice-extensions`:

```bash
cargo build -p plugin-stub
icers --extensions-dir target/debug -p 'echo "hi"'
```

Install a prebuilt extension (the default; Rust/Cargo is not required):

```bash
icers install ice-todo
icers install ice-todo --version 0.2.0
icers install ice-todo --force # Reinstall the same version
```

`icers install <package-name>` installs or upgrades directly; if the target version is already installed, it prints a message and skips the operation. Reinstalling the same version or downgrading requires `--force`. Source builds follow the same rule; during local development, use `--force` to reinstall after changing code without changing the version.

The installer selects the latest non-yanked stable version from crates.io, uses the package’s `repository` metadata to find its GitHub repository, then reads `ice-extension.json` from the `<package-name>-v<version>` Release. It matches prebuilt artifacts to the exact target triple and verifies archive size and SHA-256, the extracted library’s SHA-256, platform architecture, and declared SDK ABI. The library filename is taken from `library.path` in the manifest. Extensions in the same repository can be released independently.

Specify a custom GitHub repository (a complete HTTPS repository URL is also accepted):

```bash
icers install my-extension --github owner/repo --version 1.2.3
icers install my-extension --github https://github.com/owner/repo
```

Custom repositories must publish the same distribution protocol. An explicitly specified repository does not depend on crates.io; if no version is provided, the latest stable version is selected only from that repository’s `<package-name>-v<version>` tags. The repository’s global latest Release is not used. Subsequent `--force` reinstalls and startup update checks retain the source and verify that the new version’s manifest supports the current platform.

Publishers must upload archives for all declared platforms first and the manifest last. Installation fails if the manifest has not been published, the target platform is missing, or verification fails; it does not automatically build from source or select another version. To build from source, explicitly use:

```bash
icers install ice-todo --build
icers install ice-todo --build --version 0.2.0 --force
```

Prebuilt installation records include the version, repository, target platform, source commit, SDK ABI, and checksums; installation and uninstallation use the same transaction mechanism. The actual plugin entry point and API structure size are still validated when loading. Native system-library dependencies are provided by the operating system.

Local development:

```bash
icers install my-extension --path ../my-ice-extension --force
```

Use watch mode when developing an extension project:

```bash
cd ../my-ice-extension
icers dev
# Multi-extension workspace: icers dev --package my-extension
# Debug only the current extension: icers dev-local (isolated mode; does not load global extensions)
```

`icers dev` automatically detects Cargo `cdylib` targets, builds them initially, and loads versioned artifacts from `.icers/extensions/.dev`. Source changes trigger recompilation and hot reload; manually running `/reload` also recompiles first. If compilation fails, the currently loaded version remains active. For the full template and boundary rules, see the online extension authoring guide: <https://rpi.laofu.online/extension-authoring.md>. For debugging tips, see the `debugging` topic in `docs`.

Installed dynamic libraries are located in `~/.icers/agent/extensions` (or the directory specified by `ICERS_CODING_AGENT_DIR`) and load the next time icers starts. Plugins register tools, providers, event handlers, and resource handlers through the stable ABI; do not depend directly on private `ice-cli` modules.

### Web search and fetching

Network tools are provided by the separate [ice-package extension](https://github.com/ice-rust/ice-package),
not by the CLI’s built-in tools. Install them as needed, then restart icers:

```bash
icers install ice-websearch
icers install ice-webfetch
```

- `websearch` returns search result titles, links, and snippets.
- `webfetch` extracts readable text from public HTTP(S) pages; it rejects private addresses and oversized responses.
- These extensions are not interactive browsers: they do not support clicking, login flows, or JavaScript execution on web pages.

### Subagents that inherit the main session configuration

After installing a compatible version of `ice-subagents` and restarting, delegate tasks using `delegate_task`:

```bash
icers install ice-subagents
```

A child agent uses a snapshot of the main session’s current effective configuration: model/provider and its private credential binding, thinking level,
system prompt, resources, request options, enabled tools, project cwd, and approval hooks. It is not an `icers` subprocess started by rereading the defaults;
credentials are not passed through plugin JSON.

The child task uses an independent in-memory Session; **it does not copy or modify the parent chat history**. The host API may further restrict
the tool set but cannot expand it. `delegate_task` is removed from the child tool set, and recursive delegation through aliases is blocked.
Approvals and `ask_user` use the parent UI; without an interactive consumer, explicit rejection/errors are preserved rather than defaulting to approval.
Child tasks are cancelled when the parent run ends or is cancelled, the session switches, or the owning plugin is uninstalled/reloaded.
This is not an OS security sandbox; allowed tools such as shell retain their normal capabilities.

The host and extension must both support the corresponding protocol; an older host explicitly requests an upgrade and does not fall back to a subprocess using default configuration.
The SDK provides `AgentHarness::create_child`; extensions integrate through `GetExecutionContext` and `ChildAgent`
runtime actions. See the repository’s `docs/extensions/subagents.md` for the full protocol:
<https://github.com/Zyk-Void/ice-rust/blob/main/docs/extensions/subagents.md>。

## 7. SDK integration

If you only need to embed an Agent, depend on `ice-agent` and `ice-ai`; add `ice-tools` for built-in tools, and add `ice-harness` for sessions, compaction, skills, and context files.

```rust
use ice_agent::{Agent, AgentEvent};
use ice_ai::providers::faux::faux_provider;

let model = faux_provider().model("echo");
let mut agent = Agent::builder(model)
    .system_prompt("You are a helpful assistant.")
    .build();

let mut events = agent.subscribe();
agent.prompt("Hello from icers").await?;
while let Some(event) = events.recv().await {
    if matches!(event, AgentEvent::AgentEnd { .. }) {
        break;
    }
}
```

Implement custom tools with `ice-agent::AgentTool` and providers with `ice-ai::Provider`. Use the faux provider and `InMemoryExecutionEnv` to test the Agent loop without a network, API key, or real filesystem.

Session read models distinguish billing from current context: `AgentSession::total_usage` aggregates embedded billing data across all branches (including tool, summary, and valid child-agent usage) and no longer adds native usage recovery records. `current_context_usage`, `context_breakdown`, and `last_assistant_text` read only the active branch, applying compaction and configured entry projections/transforms; after compaction, context usage remains unknown rather than zero until valid new usage is available. Category composition uses UTF-16-length estimates, not tokenizer or provider measurements; final rewrites by asynchronous request hooks are not included in this read snapshot. By default, the composed prompt is counted entirely as the system prompt. To break down context files, skills, and tool sources, call the pure function `compute_context_breakdown` with the actual resources and source snapshots; it does not reread files or infer sources automatically.

The SQLite SDK continues to treat Rust `state_json` as the authoritative state. It adds cwd/application metadata queries, read-only discovery that does not create files, and lazily initialized FTS5 search. Writes, lease renewal, and index updates commit in the same transaction; only one writer with a fencing token is allowed per session. Writable storage requires a continuously running Tokio runtime with time enabled to send heartbeats. Explicit shutdown or dropping the last handle releases the lease; expired or taken-over handles can no longer write. Older Rust schemas can be migrated; unsupported schemas are explicitly rejected. This is not a TypeScript SQLite format converter.

## 8. Troubleshooting

### Model not found or API errors

Run `icers auth check` and verify the provider, `models.json`, environment variables, and `baseUrl`. Use `--provider` and the full `provider/model` form to avoid ambiguity between models with the same name. API errors preserve diagnostic text returned by the provider; first check the endpoint, authentication headers, and model ID.

### Extension loading failures

Confirm that the Rust plugin is a `cdylib` built for the current platform; temporarily point `--extensions-dir` to the output directory. Preserve the full error from the host when loading fails, and check whether the extension declares capabilities the host does not provide.

### Extension event handlers do not fire or startup is aborted

```bash
export ICERS_EVENT_LOG=1
icers            # Log every extension event-handler invocation
icers events tail   # Follow the log; inspect whether result is Continue or Error/Abort/Panic/Timeout
```

The log is at `~/.icers/logs/events.jsonl` by default (override with `ICERS_EVENT_LOG_PATH`). If an extension returns a veto (`EVENT_HANDLER_ABORT`) during
the `BeforeTuiStart` phase, the CLI aborts startup with exit code `3`
and prints the reason.

### Resources do not appear on the welcome screen

Confirm the files are in `.icers` (or the compatible `.pi`) directory or the global `~/.icers/agent`. Use `icers --debug-system-prompt` to inspect the final system prompt, skills, and resource counts; for duplicate resource names, check whether `.icers` is overriding `.pi`.

### Cannot connect in remote mode

Confirm that the server was started with `icers --server --port <port>` and that the token is correct (or supplied via
`ICERS_SERVER_TOKEN`). The client is a display-only interface and does not load local providers/tools/extensions;
session files reside on the server.

## 9. Development, testing, and releases

Run formatting, checks, and tests:

```bash
cargo fmt --all
cargo check --workspace
cargo test --workspace
```

Publish crates in dependency order:

```text
ice-telemetry → ice-ai → ice-agent → ice-tools → ice-harness →
ice-plugin-sdk → ice-extensions → ice-tui → ice-cli
```

Routine contributions do not require manually changing version numbers. After a regular change is merged into `main` and passes CI, automation prepares a
Release PR; a release is published only when an eligible Release PR is merged. Release notes are maintained in the root
`CHANGELOG.md`; do not create new `docs/release-vX.Y.Z.md` files. For the full process and how documentation-only changes are handled,
see the repository’s `docs/maintaining/releasing.md`:
<https://github.com/Zyk-Void/ice-rust/blob/main/docs/maintaining/releasing.md>。

After editing the model-readable documentation source in `docs/ice-tool/`, run `sh scripts/sync-embedded-docs.sh` and commit the
generated documentation snapshot. Documentation-only updates do not require a version bump or tag. The website must still be deployed separately; committing or merging
documentation does not mean the live website has been updated.

## 10. Documentation resources

- Online documentation: <https://rpi.laofu.online/docs.html>
- Source repository: <https://github.com/Zyk-Void/ice-rust>
- Rust API：<https://docs.rs/ice-agent>、<https://docs.rs/ice-plugin-sdk>

- Package and extension authoring guide: <https://rpi.laofu.online/extension-authoring.md>
- Remote mode: <docs/remote/user-guide.md>
- Rust extension and agent debugging: the `debugging` topic in the built-in `docs` tool

When online documentation and the installed version differ, refer to the corresponding Git tag and the documentation in the repository.
