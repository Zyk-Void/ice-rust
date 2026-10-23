# context-mode recipe

Use [context-mode](https://github.com/mksglu/context-mode) as a stdio MCP
server, with a command hook that redirects data-heavy built-in calls. This
recipe was tested with the globally installed **1.0.169** package on Linux.
Node and context-mode are external dependencies; icers itself does not require
Node. Review the package and hook before allowing them to execute local code.
Do not run `context-mode setup` against another agent's configuration.

## Compatibility decision

Use a **documented wrapper**, not a change to icers or the installed package.
The installed distribution has `build/adapters/` (not `src/adapters/`). Its
platform detector checks `CONTEXT_MODE_PLATFORM` before host environment and
client detection. The OMP adapter is MCP-only: its hook methods throw and it
uses OMP storage paths. The Pi adapter does not supply a compatible command-hook
integration either. Selecting either is not sufficient.

The Claude Code `hooks/pretooluse.mjs` explicitly routes/formats as
`claude-code`, regardless of the detector override. The shared routing core
already aliases lowercase `bash`, `read`, and `grep` to `Bash`, `Read`, and
`Grep`; it also handles `WebFetch` and agent tools on hosts providing them.
`read`'s `path` and `bash`'s `command` fields already work. No rename of icers
built-ins is needed (`find`, `ls`, `write`, and `edit` remain ordinary tools).
However, its MCP leaf parser splits `__` or `/`, not icers' single underscore,
and its guidance uses `mcp__plugin_context-mode_context-mode__ctx_*` names.

The adjacent [context-mode-hook.mjs](context-mode-hook.mjs) translates incoming
`context-mode_ctx_*` names to `mcp__context-mode__ctx_*` for routing, and outgoing
guidance back to the actual icers names. Keep the server name `context-mode`.
The Claude formatter emits `hookSpecificOutput.permissionDecision: "deny"`
with `permissionDecisionReason` for curl redirects, `updatedInput` for other
rewrites, or `additionalContext` for guidance. icers already consumes these
shapes. The wrapper preserves child exit codes and stderr. As with all command
hooks, crashes/timeouts are **fail-open**, not a security sandbox.

This recipe intentionally installs only PreToolUse routing, not transcript
capture/session-resume hooks. Claude transcript/session conventions do not
match icers; do not copy context-mode's entire `hooks.json` blindly. External
MCP nudges are also not implemented by this wrapper (other servers retain
icers names). Recheck compatibility after upgrading context-mode.

## Configuration

Find the package root with `npm root -g` and append `/context-mode`. Use
absolute paths below: replace `/ABS/PACKAGE`, `/ABS/AGENT`, and `/ABS/RECIPE`.
`/ABS/AGENT` normally means `~/.icers/agent`; for an isolated test it is a
private temporary directory. The example binary path is illustrative; find
its real path with `command -v context-mode`.

Add to `/ABS/AGENT/mcp.json` (or a trusted project's `.icers/mcp.json`):

```json
{
  "mcpServers": {
    "context-mode": {
      "command": "/home/you/.npm-global/bin/context-mode",
      "env": {
        "CONTEXT_MODE_PLATFORM": "claude-code",
        "CLAUDE_CONFIG_DIR": "/ABS/AGENT/context-mode-state"
      }
    }
  }
}
```

Using the Claude adapter here only selects context-mode storage conventions;
`CLAUDE_CONFIG_DIR` keeps those files under the icers-owned state directory,
not the real Claude configuration. The server inherits the environment;
explicit platform selection avoids accidental detection of the parent CLI.
This is storage placement, not data isolation: `ctx_stats` in 1.0.169 also
aggregates lifetime activity from other installed adapters' databases. The smoke
test returned existing cross-host lifetime totals despite the temporary config
root. Do not treat its totals as exclusive to this icers session.
Tools run with your OS permissions. Omitted `approveTools` means no extra MCP
confirmation; set it to true if desired, but approval-required tools cannot
execute in print mode. Never commit credentials in project configuration.

Copy the wrapper to a stable location you control, then merge this block into
`/ABS/AGENT/settings.json`, preserving existing settings and hooks:

```json
{
  "hooks": {
    "PreToolUse": [{
      "matcher": "^(bash|read|grep|context-mode_.*)$",
      "hooks": [{
        "type": "command",
        "command": "node '/ABS/RECIPE/context-mode-hook.mjs' '/ABS/PACKAGE' '/ABS/AGENT/context-mode-state'",
        "timeout": 30
      }]
    }]
  }
}
```

Hook environments are filtered, so set the state location in the command;
merely exporting `CLAUDE_CONFIG_DIR` in the parent does not pass it to hooks.
The wrapper sets it for its child. Use quoted absolute paths without literal
single quotes in their names. For project-local configuration, review code
before trusting the project (`--approve` grants that trust). Restart after
configuration changes. See [MCP](mcp.md) and [hooks](hooks.md) for trust,
timeouts, process cleanup, and other limits.

## Routing instructions and skills

Append the following to `/ABS/AGENT/APPEND_SYSTEM.md`, preserving existing
instructions. Alternatively use a trusted project's `.icers/APPEND_SYSTEM.md`.
The CLI's `resource_dirs.rs` and `session.rs` load the trusted project file in
preference to the global file, **not both**. `--append-system-prompt` takes
precedence over discovery. An untrusted project file is excluded.

```markdown
## context-mode

Use context-mode_ctx_execute or context-mode_ctx_batch_execute for large
command output; print only the derived answer. Use
context-mode_ctx_execute_file to analyze files without loading all bytes into
conversation, and context-mode_ctx_fetch_and_index plus context-mode_ctx_search
for web content. Use context-mode_ctx_stats for usage statistics.
Tool names in upstream skills or guidance may be bare ctx_* or
mcp__plugin_context-mode_context-mode__ctx_*: translate them to context-mode_ctx_*.
For edits, read exact current source with read before edit; do not write files
through a context-mode sandbox. Preserve mandatory verification logs, exit
statuses, errors, and warnings. Small commands can still use bash.
```

icers discovers `SKILL.md` directories under `~/.icers/agent/skills/` and
trusted `.icers/skills/` (plus configured resource directories). The global
agent root follows `ICERS_CODING_AGENT_DIR`. Copy the installed package's skill
directories, reviewing existing destinations first; do not overwrite local
customizations:

```sh
mkdir -p /ABS/AGENT/skills
# Repeat for desired skills; omit any destination that already exists.
cp -R /ABS/PACKAGE/skills/context-mode /ABS/AGENT/skills/context-mode
cp -R /ABS/PACKAGE/skills/ctx-stats /ABS/AGENT/skills/ctx-stats
cp -R /ABS/PACKAGE/skills/ctx-search /ABS/AGENT/skills/ctx-search
```

Other shipped `ctx-*` skill directories are optional. Review them before use:
upstream host-specific commands, upgrade/purge operations, and deferred-tool
instructions are not icers features. The APPEND_SYSTEM mapping above applies
to their tool references; skill copies do not install additional hooks.

## Isolated smoke test

Build with `cargo build --release -p ice-cli --bin icers`. Create a private
`mktemp -d` agent root, mode 700. Copy only `auth.json`, `models.json`, and
`settings.json` from your existing icers agent root and keep copies mode 600.
Add the MCP/hook configuration above using the temporary absolute paths, and
use `ICERS_CODING_AGENT_DIR=/ABS/TEMP/AGENT` for every invocation. Run in a
separate scratch project so real project hooks/configuration are not loaded.
Do not print credentials. Remove the temporary agent root on success **and**
failure (a shell EXIT trap is recommended).

1. Run `icers --list-mcp`: expect `context-mode` ready and `context-mode_ctx_*`.
2. Run `icers -p 'Call context-mode_ctx_stats and report its response.'`.
   MCP discovery is asynchronous and a run may retain its initial tool snapshot.
   A bash sleep inside that run is not sufficient on every path. For a repeatable
   one-shot smoke test, temporarily add a `UserPromptSubmit` command hook running
   `sleep 3` (timeout 10) to the **temporary** settings, so discovery can finish
   before the first request. Remove that test-only hook afterward. This is a
   startup workaround, not a requirement for normal interactive use.
3. Run `icers -p 'Call bash with command curl -s https://example.com exactly;
   if blocked, report the hook reason without retrying another tool.'`.
   Expect a denial containing `context-mode: curl/wget redirected` and actual
   `context-mode_ctx_execute` / `context-mode_ctx_fetch_and_index` names.
4. If auth or networking prevents a live model call, also pipe a synthetic
   payload into the same configured wrapper and record its JSON decision:

```sh
printf '%s\n' '{"hook_event_name":"PreToolUse","session_id":"smoke","cwd":"/ABS/SCRATCH","tool_name":"bash","tool_input":{"command":"curl -s https://example.com"}}' |
  node /ABS/RECIPE/context-mode-hook.mjs /ABS/PACKAGE /ABS/AGENT/context-mode-state
```

A synthetic decision proves routing/formatting, not an end-to-end model call.
A model that refuses to issue the requested bash call is not proof of a hook
invocation either. Inspect tool events when collecting evidence.
