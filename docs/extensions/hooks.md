# Command hooks

Command hooks run trusted local shell commands at coding-agent boundaries. Put
`hooks` in `~/.icers/agent/settings.json`, or in `.icers/settings.json` for a
**trusted project only**. Global groups run first, followed by project groups.
Project trust is the same explicit trust decision used for project extensions.
An untrusted project's hooks are not loaded. Settings are snapshotted when the
session is built; restart the session after changing hook configuration.

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "^(bash|read)$",
        "hooks": [
          { "type": "command", "command": "sh ./check-tool.sh", "timeout": 30 }
        ]
      }
    ]
  }
}
```

`matcher` is a regular expression over the tool name. Missing, empty, or `"*"`
matchers match everything. Invalid regexes are logged and skipped. Non-tool
events have an empty tool name; omit their matcher. Only `type: "command"` is
supported. Hooks run sequentially within an event. Parallel tool calls can run
hooks concurrently; scripts must synchronize their own shared files.

## Events and stdin

Each invocation receives one JSON object on stdin and EOF. All events include
`hook_event_name`, `session_id`, and `cwd`.

| Event | When | Extra fields |
| --- | --- | --- |
| `SessionStart` | Once while building the session, before its first request | `source: "startup"` |
| `UserPromptSubmit` | Before accepting a user prompt, steer, or follow-up | `prompt` (text; images omitted) |
| `PreToolUse` | After validation and plugin approval, before execution | `tool_name`, `tool_input` |
| `PostToolUse` | After an executed tool, before publishing its result | `tool_name`, `tool_input`, `tool_response` |
| `PreCompact` | Before explicit compaction or an automatic summarizer | `trigger: "manual"` or `"auto"` |
| `Stop` | At terminal run completion, including failure or abort | No additional fields |

`tool_response` contains `content`, `details`, and `is_error`. Post-tool input
contains the actual rewritten arguments. Blocked tools do not execute and do
not generate `PostToolUse`. Resuming a session still uses `source: "startup"`.
Stop is advisory and does not automatically restart the agent.

## Exit codes and decisions

- **0:** Parse stdout as optional JSON. Empty or non-JSON stdout has no effect.
- **2:** Block a tool or reject a submitted prompt. Stderr is the reason.
  Blocks from other events are logged only.
- **Other exits, spawn failures, cancellation, timeouts:** Log and continue.
  A missing or crashing hook does not break the session.

Example denial (a top-level `{"decision":"block","reason":"..."}` also works):

```json
{
  "hookSpecificOutput": {
    "permissionDecision": "deny",
    "permissionDecisionReason": "This command needs review"
  }
}
```

To replace tool arguments and supply model context:

```json
{
  "hookSpecificOutput": {
    "updatedInput": { "command": "git status --short" },
    "additionalContext": "Use the short status output."
  }
}
```

`updatedInput` replaces the whole argument object for `PreToolUse`; it is not a
patch and is not revalidated, matching the agent's existing trusted-plugin
rewrite contract. Later hooks receive the updated input. An earlier plugin
veto cannot be undone by a command hook. The post-tool adapter supports
composition with an earlier result adapter and preserves its overrides.

`additionalContext` is also accepted at the top level. Pre- and post-tool
context is appended as text to the tool result sent to the model; for a denied
tool it is included in the blocking result. Prompt context is appended to the
accepted user message (and persisted with it). Session-start context is added
to the session system prompt. Pre-compaction and Stop context is queued as a
user-context message at the next model-request boundary; Stop does not trigger
that request itself. Advisory queued context is in memory only.

## Integration recipes

The [context-mode recipe](context-mode.md) combines MCP tools with a compatible
PreToolUse wrapper without changing built-in tool names.

## Execution, environment, and limits

Commands execute in `cwd` via `sh -c` on Unix or `cmd /C` on Windows. Execution
runs off the async/UI thread. The default timeout is **60 seconds** per command;
`timeout` is an integer number of seconds (zero expires immediately). On expiry
or completion the owned process tree is killed, including background children.
Only the first 1 MiB of each output stream is retained; excess output is drained
and discarded. Hook output is not streamed into the TUI.

The runner clears the inherited environment and restores only `PATH`, `HOME`,
`USERPROFILE`, `SYSTEMROOT`, `WINDIR`, `COMSPEC`, `PATHEXT`, `TEMP`, `TMP`,
`TMPDIR`, `LANG`, `LC_ALL`, and `TERM`, when present. It additionally sets
`ICERS_PROJECT_DIR` and `ICERS_SESSION_ID`. It does not inject provider keys,
auth tokens, proxy settings, or other `ICERS_*` variables. Scripts that need
credentials must obtain them themselves; never put secrets in hook commands.

**Hooks are not sandboxed.** Trusted commands can read files and credentials
available to the operating-system user, invoke other programs, and access the
network. Review project hooks before granting trust. Keep shell startup files
and executables on PATH trustworthy. Environment filtering prevents accidental
credential inheritance, not intentional credential access by trusted code.
