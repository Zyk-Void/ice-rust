# ice → ice-rust gap report

What the TypeScript ice harness (`~/Zks/ice`, repo `Zyk-Void/ice`) has that this
Rust port does not, as of 2026-10-19. Each entry says where the feature lives in
ice, how it works there, and what ice-rust has today.

Path conventions: `ice:` paths are relative to `~/Zks/ice/packages/`; bare
`crates/...` paths are in this repo. Abbreviation: `CA` = `ice:coding-agent/src`.

How this was produced: mechanical diffs of enumerable surfaces (CLI flags,
settings keys, `ICE_*` env vars, RPC command types, extension API, tool names,
providers, TUI components), then reading each ice-only subsystem and probing the
Rust crates for an equivalent. "Missing" means no equivalent was found by name
or behavior. "Partial" means related code exists but the behavior differs. Not
verified by running both binaries side by side.

Size reference: ice is ~185k lines of TS (coding-agent 129k, ai 26k, tui 15k,
agent 9k, server/protocol/client/storage ~6.5k).

---

## 0. The architectural gap that shapes everything else

**ice-rust cannot load TypeScript extensions.** ice ships most of its own
features as in-process TS extensions registered by the launcher
(`CA/ice.ts:main(... extensionFactories: [...])`):

| Factory | Source | Feature |
|---|---|---|
| `ice-safe-verify` | `CA/ice-safe-verify.ts` | plan/build modes, verifier, `ask`/plan tools (§2) |
| `ice-provider-settings` | `CA/ice-provider-settings.ts` | provider submenu in `/settings` (§6.3) |
| `ice-cognee` | `CA/ice-cognee.ts` | Cognee memory (§5) |
| `ice-subagents` | `CA/subagents/v2/extension.ts` + `CA/ice-mcp-bridge.ts` | Subagents V2 (§1) |

All four run with `priority: "before-user"` and `hidden: true`. ice-rust's
extension model is native C-ABI plugins (`crates/ice-plugin-sdk`, entry symbol
`ice_plugin_register`, loaded as `.so/.dll/.dylib`). The README states it "does
not load the TypeScript" extensions. So every item in §1–§5 has to be ported
either into the Rust core or as a native plugin.

Plugin event coverage is already at parity. Every ice extension event exists in
`EventTag` (`crates/ice-plugin-sdk/src/events.rs`): `session_before_compact`,
`resources_discover`, `before_agent_start`, `agent_settled`, `tool_call`
(blocking), `tool_result`, `context`, `user_bash`, `input`, etc. The gaps are the
host API methods in §7.2 and the subsystems themselves.

---

## 1. Subagents V2 (largest gap: ~17k lines in ice)

**ice:** `CA/subagents/**`. User doc: `ice:coding-agent/docs/subagents-v2.md`.
Architecture: `docs/subagents-v2-architecture.md`. Reference: `docs/subagents-v2-reference.md`.

**ice-rust today:** `crates/ice-cli/src/child_agents.rs` (417 lines) gives
plugins bounded "child runs" (start/poll/cancel/dispose) with a concurrency permit
and timeout. The companion `ice-subagents` plugin exposes one `delegate_task`
tool (README "Subagents with inherited configuration"). It has no model-facing
agent system, no definitions, no permissions, no worktrees, no persistence, and
no hub.

### 1.1 Agent definitions
- **Files:** `CA/subagents/definitions/{schema,parse,load,discover,precedence,listing,builtin,paths}.ts`.
- **How it works:** an agent is a Markdown file with strict YAML frontmatter.
  Unknown keys make the definition unavailable. The body becomes the child's
  system-prompt contribution. Discovery covers `~/.ice/agents/` (user) and
  `.ice/agents/` (project, needs project trust). Precedence (`precedence.ts`):
  managed definitions are locked, and otherwise a higher-precedence source
  shadows a lower one with the same `name`. Invalid, untrusted, disabled,
  shadowed and managed definitions are reported separately.
  `listing.ts` builds the model-visible agent list that goes into the system
  prompt, so the model needs no listing tool.
- **Built-ins** (`builtin.ts`): `general` (default); `explore` (tools
  read/grep/find/ls); `plan` (read-only, `permissionMode: plan`, no context
  files, cannot spawn); `review` (read-only, `permissionMode: review`); `worker`
  (requests worktree isolation).
- **Fields:** model and thinking (an omitted model inherits the parent's exact
  model), tools (can only narrow the parent's set), `permissionMode`,
  `isolation`, `mcp: ["server/tool"]`, `spawns`, `contextFiles`.

### 1.2 The three model-facing tools
- **`agent`** (`CA/subagents/api/agent.ts`): `{ tasks: AgentTask[1..8], background? }`.
  Each task has `agent?`, `name?` (child name), `resume?` (continue a child),
  `description` (≤120 chars), and `prompt` (≤64 KiB, self-contained: children
  never see the parent transcript). Optional: `model`, `thinking`, `isolation`,
  `scope {roots[≤32], targets?}`, `outputSchema`, `acceptance[≤16]`, and
  `context[≤16]` items of kind `parent_note|verified_fact|evidence_ref|artifact_ref`
  (≤32 KiB each).
- **`agent_control`** (`api/agent-control.ts`): actions `status, wait, send,
  resume, stop, delete, output, list, agents`. `send` delivery is
  `message|followup|interrupt`. Limits: ≤8 ids, message ≤64 KiB,
  `wait` ≤30 min, `output` read ≤48 KiB.
- **`agent_patch`** (`api/agent-patch.ts`): `{ id, action: inspect|apply|reject }`
  on a worktree child's patch.

### 1.3 Launch path and frozen contract
- **Files:** `v2/runtime.ts` (2407 lines), `v2/session-factory.ts`,
  `contract/{child-runtime-contract,serialize,validate-resume}.ts`,
  `runtime/{preflight,context-bootstrap,prompt,resources,routing,live-session}.ts`.
- **How it works:**
  1. Resolve the parent session and trust.
  2. Load definitions.
  3. Preflight: resolve the definition, model, tools, permissions, scope and
     environment (`runtime/preflight.ts`).
  4. Build a `ChildRuntimeContract`: owner/root lineage, agent/source hashes,
     resolved prompt and resources, exact model/thinking, child tools and adapter
     fingerprints, and rule origins. Hash it canonically (`contract/serialize.ts`).
  5. Create the child as an ordinary `AgentSession` with its own JSONL
     transcript under `<session-dir>/subagents/<parent-session-id>/`
     (`registry/child-sessions.ts`).

  A later change to definitions or settings never rewrites an existing child's
  contract. `validate-resume.ts` refuses a resume whose contract no longer
  validates. `runtime/context-bootstrap.ts` gives the child a deterministic,
  hash-verified, read-only context bootstrap. Subagents require a saved parent
  session (`--no-session` is unsupported).

### 1.4 Registry, lifecycle, admission
- **Files:** `registry/{registry,state-machine,admission,child-sessions}.ts`, `v2/child-names.ts`, `v2/prune.ts`.
- **How it works:**
  - Instance states: `starting, running, idle, parked, stopping, blocked,
    aborted, deleted`. These are separate from the last turn's outcome: a
    successful turn leaves the child `idle`, not terminal.
  - Admission is a hard concurrency ceiling plus a FIFO permit queue.
  - On process restart, an in-flight turn is marked `interrupted` and the child
    is `parked`. Session switch, `/new`, fork and quit stop children the same way.
  - A tool call whose result was not recorded is never replayed.
  - Child names come from a random pool (`child-names.ts`).
  - `prune.ts` cleans transcripts and worktrees.

### 1.5 Permissions
- **Files:** `permissions/{rules,subjects,evaluate,derive-child,approval}.ts`.
- **How it works:**
  - One deny-first evaluator serves both parent and child subjects (tool, path,
    command). Rules are `allow|ask|deny`, and the most restrictive match wins.
  - A child is never more permissive than its parent. Its authority is the
    intersection of the definition, the invocation, trust/mode policy, scope and
    adapter policy.
  - Defaults: reads follow the parent. Edit, write, Bash and mutation-capable
    adapters `ask`.
  - `plan`/`review` modes hard-deny Bash, edit/write and mutating adapters.
  - External directories are at most `ask`. `--sub-yolo` cannot auto-allow them.
  - `approval.ts` brokers `ask` decisions to the parent UI (RPC:
    `respond_subagent_approval`).

### 1.6 Worktree isolation and patch integration
- **Files:** `isolation/{worktree,apply,patch-state}.ts`.
- **How it works:**
  - The parent's staged, unstaged and supported untracked content is
    snapshotted into an isolated baseline using raw blobs plus a temporary
    index. The child works in a git worktree on that baseline.
  - The child's output is a delta against the baseline, so pre-existing parent
    edits are never attributed to the child.
  - `apply.ts` merges only the child delta. It preserves the parent index byte
    for byte, checks baseline and pre/postimage before writing, and rolls back
    compare-before-restore.
  - It refuses nested repos, submodules and repos without a commit.

### 1.7 Results, assurance, output artifacts
- **Files:** `results/{report,task-contract,assurance,batch-synthesis,output-artifacts,v2-result}.ts`.
- **How it works:**
  - Model-authored reports are treated as untrusted. `report.ts` validates a
    small frozen JSON-Schema subset (`$ref` etc. rejected) for `outputSchema`.
  - `task-contract.ts` is the host-owned delegation contract.
  - `assurance.ts` computes an assurance level from evidence the host actually
    observed, never from fluent text.
  - `batch-synthesis.ts` de-duplicates exact sibling findings across a batch.
  - `output-artifacts.ts` bounds and stores large child output for `agent_control output`.

### 1.8 Tools, MCP pass-through, budgets
- **Files:** `tools/{resolve,scoped-tools,adapters,paths}.ts`, `v2/runtime-support.ts`,
  `CA/ice-mcp-bridge.ts`, `v2/turn-budget.ts`, `runtime/budgets.ts`.
- **How it works:**
  - The child tool allowlist is fixed at launch. `scoped-tools.ts` re-checks
    eligibility at the dispatch boundary.
  - MCP: a definition's `mcp: ["server/tool"]` selectors pass through the
    parent's already-authorized `pi-mcp-adapter` (`ice-mcp-bridge.ts`:
    `listPiMcpAuthorizedTools`, `createPiMcpSubagentAdapter`). No new MCP server
    is started per child.
  - `turn-budget.ts` counts uncached input, cache writes and output against a
    `maxTokens` budget.

### 1.9 Hooks, events, settings, controller, CLI, UI
- **Ported — session-local stdio MCP client** (PR: TBD — PR not yet opened):
  global and trusted-project `mcp.json`, background discovery, namespaced tools,
  per-tool approval, bounded calls/cancellation, and `--list-mcp`. `/mcp` now
  provides a status/tools panel, live reconnect, enable/disable, global add/remove,
  external configuration editing and argument completion. See
  `docs/extensions/mcp.md` and the `docs/extensions/context-mode.md` recipe.
  Resource discovery/read tools (default-on `exposeResources`), targeted tool
  and resource catalog refresh, asynchronous server-request dispatch, and bounded
  output/blob materialization are implemented in the Tier 4 stack (PRs: TBD).
  HTTP/SSE transports, OAuth, prompt discovery, resource subscriptions, sampling,
  and elicitation remain unported. Subagent MCP pass-through
  in §1.8 is separate and remains unported.
- **Ported — command hooks** (PR: TBD — PR not yet opened): settings-driven `PreToolUse`, `PostToolUse`,
  `UserPromptSubmit`, `SessionStart`, `PreCompact`, and `Stop` shell hooks are
  implemented with trusted-project gating and bounded process execution. See
  `docs/extensions/hooks.md`. Subagent launch/resume decisions, hook journals,
  and the broader `ice.hooks` namespace remain unported.
- **Hooks/events:** `hooks/command-hooks.ts`, `v2/events.ts`, `extension-support.ts`.
  Lifecycle and decision hooks run through `ice.hooks`. Decision hooks can deny
  a launch, a tool call or a resume. Redacted hook records are persisted.
  Env knobs: `ICE_SUBAGENT_DECISION_EVENTS`, `ICE_SUBAGENT_HOOK_EVENTS`,
  `ICE_SUBAGENT_OBSERVATION_EVENTS`, `ICE_SUBAGENT_SETTINGS_LIMITS`,
  `ICE_BUILTIN_CHILD_TOOLS`, `ICE_CAPABILITY_LIMITS`, `ICE_HOOK_CONTEXT_ADDITION_LIMITS`,
  `ICE_HOOK_JOURNAL_ENTRY_TYPE`.
- **Settings:** `CA/subagents/settings.ts` (916 lines) and `v2/settings.ts`
  define the `ice.subagents.*` and `ice.hooks` namespaces. Top-level settings
  include `defaultAgent`, `disabledAgents` and `askTimeoutMs`
  (`CA/core/settings-manager.ts:104-110`).
- **Controller for trusted extensions:** `v2/controller.ts`, registered under
  `ICE_SUBAGENT_CONTROLLER_KEY`. It gives versioned programmatic access, used by
  the Goals host (§2.3).
- **CLI:** `v2/cli.ts`: `ice agents list [--json]`, `ice agents validate <file>`.
  `CA/ice.ts` also handles `--import-agent-pack <ruflo-repo>`
  (`CA/ice-agent-packs.ts`: maps Ruflo agent packs to V2 definitions and reports
  unsupported fields).
- **Flags:** `--agents`, `--sub-yolo` (registered in `v2/extension.ts`).
- **UI:** `ui/hub.ts` is the Agent Hub (`/agents` or `←` on an empty editor):
  roster, pending approvals, state, usage, and a live child view (Esc returns).
  `/agents new <name> [--project]` creates a definition.
  `CA/ice-agent-view-bridge.ts` (903 lines) bridges child sessions into
  interactive, print, JSON and RPC views (descriptors, colors via
  `ICE_AGENT_VIEW_COLORS`, live controls). `modes/interactive/components/subagent-view-switcher.ts`
  switches views.
- **SDK:** `v2/sdk.ts` (`createIceSubagents()`).

---

## 2. Guarded plan/build modes (`ice-safe-verify`)

**ice:** `CA/ice-safe-verify.ts` (1895 lines). **ice-rust:** missing. The Rust TUI
only renders `plan_mode_*` tool names that come from plugins
(`crates/ice-cli/src/transcript_view.rs:348`).

### 2.1 State machine
- `IceStateV3` (`ice-safe-verify.ts:~200`) holds:
  - `mode: plan|build`
  - `guardRoot` (git root, or cwd)
  - a git `baseline` (head, branch, porcelain status)
  - `mutationGeneration` / `checkedGeneration`
  - `verifier` state
  - `plan` (`status: none|draft|pending|approved`, markdown, content hash,
    `readInBuild`, approval context policy `fresh|compact|keep`, pre-plan model
    and thinking, execution model)
- The state is persisted as a custom session entry (`appendEntry(STATE_TYPE)`).
  It is restored on `session_start`/`session_tree`, so it survives resume and
  branch navigation.

### 2.2 Enforcement
- **`tool_call` hook (`:1626`):**
  - Blocks any tool not in the mode's effective set.
  - Plan mode allows read-only tools plus `ask`, `draft_plan`, `propose_plan`
    and `read_plan`.
  - Build mode adds `write`/`edit` only when the project is trusted, and Bash
    only with `--ice-allow-bash`.
  - In build mode, mutation is blocked until `read_plan` has been called after
    approval.
  - `validateMutationPath()` rejects paths outside `guardRoot` unless
    `--allow-external` is set.
- **`tool_result` (`:1659`):** bumps `mutationGeneration` after each successful
  mutation, so the verifier knows its last pass is stale. It also integrates
  subagent `agent_patch apply` results (§1.6), including their verification status.
- **Turn context:** `before_agent_start` injects hidden context.
  `ice-plan-mode-context` goes in while planning and
  `ice-approved-plan-context` after approval. `agent_settled` injects
  `PLAN_DECISION_REMINDER` when a plan turn ended without `ask`/`propose_plan`.
  `context` trims history to the approval cutoff under the `fresh` policy.
- **Model switching:** `--ice-plan-model` / `--ice-build-model` swap model and
  thinking on a mode change. The pre-plan thinking level is restored in build.

### 2.3 Tools, commands, flags, verifier, goals host
- **Tools:**
  - `ask`: a structured question to the user.
  - `draft_plan`: creates or replaces the bounded Markdown draft.
  - `propose_plan`: submits the plan for explicit approval.
  - `read_plan`: reads the saved plan; required before build mutations.
- **Commands:** `/plan`, `/build`, `/ice-plan-review`, `/ice-verify`, `/ice-status`.
- **Flags** (validated before startup by `validateIceStartupArgs`):
  `--ice-mode plan|build`, `--ice-allow-bash`, `--allow-external`,
  `--ice-verify '<json argv>'`, `--ice-workgraph-dispatch`,
  `--ice-plan-model`, `--ice-build-model`.
- **Verifier:** `runVerifier()` spawns the trusted argv with no shell.
  Timeout `VERIFIER_TIMEOUT_MS = 120_000`, output cap 64 KiB, and a
  termination grace period. It records exit code, signal, duration and
  truncated stdout/stderr. Status values: `not-configured, pending, running,
  passed, failed, timed-out, spawn-error, cancelled, blocked-untrusted`.
- **Goals host:** `CA/ice-goals-host.ts` and `docs/ice-goals-host.md`. It is a
  versioned controller on the extension event bus at
  `Symbol.for("ice.goals.host.v1")` / `ICE_GOALS_HOST_KEY`. `snapshot(ctx)`
  returns session, mode, trust, guard root, mutation generation, the verifier
  hash and the effective tools. Verification requests require a saved session,
  trusted build mode, a configured `--ice-verify`, a re-read approved plan and
  no overlapping run. It is consumed by the external goals/Workgraph extension.

---

## 3. Background shells (`bash` run_in_background, `bash_output/kill/input/watch`)

**ice:** `CA/core/tools/{bash.ts,bash-background.ts,bash-shells.ts,bash-watch.ts}` (~86 KB).
**ice-rust:** missing. `crates/ice-tools/src/tools/bash.rs` is foreground-only:
it has no `run_in_background` or `stdin` parameters.

- **Starting a shell:** `bash` gains `run_in_background` and `stdin` parameters.
  A foreground call is also moved to the background automatically after
  `bashAutoBackgroundMs` (default 60 000), or by the user with **Ctrl+B**.
- **Registry** (`bash-background.ts`): ≤16 running shells (`MAX_RUNNING_SHELLS`).
  Each shell keeps:
  - a capped output tail
  - status and timing
  - a per-shell read cursor, so `bash_output` returns only new output
  - a full log file the model can tail or grep

  When a shell exits by itself, a `bash_background_completed` custom message is
  delivered once. Finished shells stay listed for a while. All processes are
  killed on dispose, and logs are deleted when a shell drops from the list.
- **Tools** (`bash-shells.ts`, active exactly when `bash` is active):
  - `bash_output`: status plus new output. It can wait up to 300 s, or tail/grep
    the full log. With no id it lists all shells.
  - `bash_kill`: stops a shell and its process tree.
  - `bash_input`: writes to a shell started with `stdin: true`. Limits: 8 KiB
    per call, 64 KiB per shell. Approval follows `bashInputApproval: ask|allow`,
    and it is denied when there is no UI.
- **Watches** (`bash-watch.ts`): `bash_watch` with `create|list|cancel|rearm`.
  Triggers: a literal `pattern` match, silence (≥5 s), or a fixed interval (≥30 s).
  Limits: ≤16 watches, ≤10 fires each. A fire is delivered as a
  `bash_watch_fired` message, as a steer while the agent is busy, so blocking
  waits end. Alerts never run tools or call the model themselves. Default
  watches on every shell: a silence alert (`bashSilenceAlertMs`, 120 000) and a
  "looks like a prompt" alert (`bashPromptAlertMs`, 45 000).
- **UI/RPC:** `modes/interactive/components/shells-panel.ts` (live shells view).
  RPC `background_bash`, `get_shells`, `get_shell`, `kill_shell`, `abort_bash`,
  plus `shells_update` events.

---

## 4. Observability: run history + live dashboard

**ice:** `CA/observe/{observe-recorder,observe-store,observe-server,observe-cognee}.ts`,
`observe-dashboard.html`, `docs/observability.md`. **ice-rust:** ported except
the Cognee knowledge views. `crates/ice-cli/src/observe/` holds the recorder,
the SQLite store (`~/.icers/agent/observability.sqlite`), the loopback HTTP/SSE
server (`icers --observe[=port]`, default 4659) and the dashboard; see
`docs/guides/observability.md` for the differences from TS. Cognee memory
operations appear as correlated `memory` rows (OBS-008). Remaining: the
Knowledge tab and its `/cognee/*` routes (OBS-017..019, OBS-024..029), queue-update rows (Rust emits no queue event), compaction summaries (the
harness reports only start/finish) and the install-telemetry gate and startup
timing items (OBS-001, OBS-002).

- **Recorder** (`observe-recorder.ts`, always on): subscribes to the session
  event stream. It projects tool calls (args, result preview, duration, error),
  messages, compaction, retries, skills, extension messages, goal/verifier
  transitions and Subagents V2 child lifecycle into a bounded, redacted snapshot
  plus periodic state (model, context usage, token totals). Activity updates are
  recorded so a replay shows how a tool call evolved. It is read-only.
- **Store** (`observe-store.ts`): appends events to
  `~/.ice/agent/observability.sqlite`, through the `@zykairotis/ice-storage-sqlite-node`
  package (`ice:storage/sqlite-node`). It is separate from session JSONL and
  never mutates it. Retention is set by `OBSERVE_RETENTION_MS`, replay is capped
  at 10 000 events, and an open failure only warns.
- **Server** (`observe-server.ts`, `ice --observe[=port]`, default 4649): plain
  `node:http` plus SSE, loopback only. Every route requires a per-user token,
  taken from `?token=` (which sets an HttpOnly cookie), the cookie, or a Bearer
  header. Endpoints: dashboard HTML, JSON snapshot, live SSE, run replay.
  It accepts no commands. An already-serving port is shared.
- **Knowledge tab** (`observe-cognee.ts`): read-only Cognee views: datasets,
  documents, session cache, graph, and context-only search.
- **Dashboard:** event "tape" with keyboard filters `0–9`; pulse bars use
  epoch-anchored bins.

---

## 5. Cognee memory (`ice-cognee`)

**ice:** `CA/ice-cognee.ts` (2529 lines), `ice-cognee-client.ts` (1013),
`ice-cognee-env.ts`, `ice-cognee-observer.ts`, `CA/ice-cognee-skills/`
(cognee-search/remember/sync), `docs/ice-cognee.md`. **ice-rust:** ported as a
built-in in `crates/ice-cli/src/cognee/` (COG-001..068, [PR #66](https://github.com/Zyk-Void/ice-rust/pull/66)). It runs through the in-process
harness extension seam (`crates/ice-harness/src/extension.rs`) and an agent-event
observer, not the C plugin ABI. Tests use an injected transport (no network).

Deliberate differences from ice:
- **Opt-in.** Cognee is installed only when `<agentDir>/ice-cognee/config.json`
  exists or an `ICERS_COGNEE_*` variable is set (`/cognee on` writes the file).
  ice enables it by default. This keeps a fresh install, and the test suite, from
  sending prompts to whatever listens on `127.0.0.1:8211`.
- **Offline.** `--offline` / `ICERS_OFFLINE` disables every Cognee request.
- **Names.** `ICERS_COGNEE_*` instead of `ICE_COGNEE_*`; storage is
  `~/.icers/agent/ice-cognee/` (`config.json`, `api_key.json`, `pending/`,
  `warmup/`, `dream/`, `observations.jsonl`, `compaction-recall.json`). Cognee's
  own `COGNEE_*`, `~/.cognee/.env` and `~/.cognee-plugin/api_key.json` are read
  as in ice.
- **Settings** rows show the current config each time `/settings` opens (ice
  captures them at registration).
- **`/cognee watch`** points at `observations.jsonl`; icers has no observe
  dashboard. The browse client (COG-021..030) is ported, but its dashboard
  consumers (OBS-017..019) are not.
- **Health probe** reads at most 4 KiB of the `/health` body (ice reads it
  unbounded).
- **Compaction capture** sends the checkpoint's session-cache entry in the
  background instead of awaiting it inside `session_compact`.
- **Skills** are written to `<agentDir>/ice-cognee/skills/` and loaded like other
  skill files, only while Cognee is installed.

- **Hooks:**
  - `before_agent_start`: turn-scoped recall appended to the system prompt.
    Prompts under 4 words (`RECALL_MIN_WORDS`) skip recall, and dedup TTL
    applies.
  - `tool_result`: captures allowlisted tools (`CAPTURE_TOOL_ALLOWLIST`, ≤
    `MAX_TRACES_PER_TURN`).
  - `agent_end` / `agent_settled`: remember Q&A and run an idle "improve"
    (dream pass) with a cooldown.
  - `session_before_compact` / `session_compact`: own or contribute the
    compaction summary (`compactionSummaryMode`), and optionally run checkpoint
    recall that writes `<agentDir>/ice-cognee/compaction-recall.json` for
    Blackhole (§6.1).
  - `session_shutdown`: final improve.
  - `resources_discover`: contributes the Cognee skills.
- **Datasets:** `"dataset": "$project"` resolves to
  `ice-<root-name>-<8-hex-sha256(root)>`, giving per-repo memory.
- **Client:** HTTP to a local Cognee API (default `http://127.0.0.1:8211`) with
  a circuit breaker. The key comes from `~/.ice/agent/ice-cognee/api_key.json`
  or `COGNEE_API_KEY`. `~/.cognee/.env` is read for non-secret routing keys only.
- **Surfaces:** `cognee_search` tool; `/cognee status|watch|doctor|on|off|recall|capture|tools|improve|remember|search…`;
  a Cognee section in `/settings` (`registerSettings`).
- **Observer:** every request is appended, redacted, to
  `~/.ice/agent/ice-cognee/observations.jsonl` (size-capped).
- **Env:** `ICE_COGNEE_{ENABLED,BASE_URL,DATASET,RECALL,REMEMBER,CAPTURE,CAPTURE_TOOLS,IMPROVE,COMPACTION_SUMMARY,CHECKPOINT_RECALL}`.

---

## 6. Session, compaction, resources

Native session-management work is tracked in
[`docs/session-parity-progress.md`](docs/session-parity-progress.md), against the
source baseline in [`docs/ice-gap/session.md`](docs/ice-gap/session.md). It covers
branch/query correctness, safe search/import, SQLite ownership/indexing, usage
and context projections, validated summaries, and active-branch exports. The
checklist distinguishes tested implementations from helpers and pending runtime
integration; it is not a claim of complete coding-session parity. Native v4 JSONL
and canonical Rust SQLite snapshots remain deliberate format boundaries. Native
JSONL now has storage-lifetime advisory ownership and conservative conflict
rejection; coding overflow has one bounded recovery attempt; required final-save
errors propagate; model/thinking UI changes follow successful runtime application;
sharing follows the active branch. `goal.md` records acceptance evidence and the
remaining budgeting, lifecycle, transport and review work. Known coding-request
prompt/tool overhead and the shared provider output allowance are now included
without double-counting measured usage; hook/custom-converter payloads and full
tool-pair/reopen coverage remain open. Follow-up formatting, warning-as-error
workspace checks/tests and documentation/tooling gates passed (2,125 tests
passed, 5 ignored). MSRV and Windows runtime remain unverified. The reliable-session
goal is not yet complete. Additional native resume-precedence, custom-root/cwd
and disappearing-file regressions pass; private gist/clipboard stubs verify
active-branch payloads without real transport. Full gates passed again after
these additions (2,130 passed, 0 failed, 5 ignored); MSRV and Windows runtime
remain unverified. No PR has been opened for this worktree yet.

### 6.1 Blackhole compaction extension
- **ice:** `ice:coding-agent/examples/extensions/ice-blackhole/` (index + `src/{core,extract,hooks,om}`).
  It derives from `sting8k/ice-vcc`. **ice-rust:** ported as a built-in in
  `crates/ice-cli/src/blackhole/` (BLACK-001..041, [PR #66](https://github.com/Zyk-Void/ice-rust/pull/66)). Like the optional ice package it
  is off until `<agentDir>/ice-blackhole/ice-blackhole-config.json` exists or an
  `ICERS_BLACKHOLE_*` variable is set. It hooks compaction through the harness
  extension seam (`before_compact`, `after_compact`, `boundary`,
  `transform_context`). Overflow recovery always keeps the host tail. The
  `memory` setting is persisted only; ice has no memory workers either.
- **How it works:**
  - `session_before_compact` (`hooks/before-compact.ts`) replaces the LLM
    summary with a deterministic compiler: normalize, filter noise, build
    sections (`core/summarize.ts`, `build-sections.ts`). Sections cover user
    requests, findings, files touched (`extract/files.ts`), commits
    (`extract/commits.ts`), goals (`extract/goals.ts`) and preferences.
  - Volatile "live state" (`core/live-state.ts`) is rebuilt from durable entries
    at every compaction and never merged from the previous summary. It covers
    the active goal, mode/plan, subagents, shells, and MCP/external tool use.
  - `skill-collapse.ts` collapses skill bodies.
  - `trim-kept-tail.ts` trims the kept tail.
  - It consumes Cognee's one-shot checkpoint recall (`core/checkpoint-recall.ts`).
  - `om/compaction-trigger.ts` requests boundary compactions and stops
    re-requesting after a miss.

### 6.2 Compaction behavior in core
- **Single in-flight lock** (`CA/core/agent-session.ts`, per ICE_CHANGELOG).
  `isCompacting` plus the prompt-submit guard cover in-flight compaction.
  Manual and auto compaction can never both append entries, and settings are
  read after the lock is taken. Rust: verify (no equivalent found by name).
- **Mid-run compaction:** setting `compaction.midRunCompaction` (default off)
  and `thresholdPercent` (default 85) (`settings-manager.ts:27-30`), plus RPC
  `abort_compaction`. Rust: the settings and the tool-turn boundary compaction
  exist (`crates/ice-cli/src/settings.rs`, `context_boundary_hook` in
  `crates/ice-harness/src/agent_harness.rs`); extensions can also request a
  boundary compaction (Blackhole does). RPC `abort_compaction`: missing.
- **Abort reclassification:** an `error` stop reason seen after the run signal
  aborted is reclassified as `aborted` (`ice:agent/src/agent-loop.ts`). Verify
  in `crates/ice-agent/src/abort.rs`.

### 6.3 Skills and resources
- **Skill hot reload** (`CA/core/resource-loader.ts` `refreshSkillsIfChanged`,
  `computeSkillsSignature`). Skill paths get an mtime+size signature (recursive,
  skipping dot entries and `node_modules`, contents never read). The signature
  is re-checked on every prompt submit, polled every 5 s while idle, and checked
  by RPC `get_commands`. Rust: `crates/ice-cli/src/fs_watch.rs` polls only the
  theme file and git state. Skills reload: missing.
- **`enableSkillCommands`** (registers `/skill:name`) and **`customDirectories`**:
  not found in Rust.
- **Package resource selector** (`components/config-selector.ts`, 951 lines):
  TUI to enable or disable resources per package. Missing.

---

## 7. Coding-agent surface gaps

### 7.1 CLI flags
Missing from `crates/ice-cli/src/args.rs`:

| Flag | ice | Behavior |
|---|---|---|
| `--fast` / `--no-fast` | `CA/cli/args.ts:159` | Requests the priority service tier on capable Responses models (`fastMode`). |
| `--observe[=port]` | `:91` | §4 dashboard. |
| `--print-system-template` | `:84` | Prints the system prompt template and exits. |
| `--ui-mode regular\|fullscreen` | `:201` | Fullscreen renderer that owns the mouse (`fullscreenScrollbar`). |
| `--ice-mode`, `--ice-verify`, `--ice-allow-bash`, `--allow-external`, `--ice-plan-model`, `--ice-build-model`, `--ice-workgraph-dispatch` | §2 | |
| `--agents`, `--sub-yolo`, `--import-agent-pack`, `ice agents …` | §1.9 | |
| `ice client/server/auth …` (experimental) | `CA/cli/experimental/commands/*` | Experimental remote session CLI over the §8 protocol. |

### 7.2 Extension host API methods with no Rust equivalent
From `CA/core/extensions/types.ts` (1810 lines):
- **UI:** `setWidget`, `setHeader`, `setFooter`, `setEditorComponent`,
  `getEditorComponent`, `setWorkingIndicator`, `setWorkingMessage`,
  `setWorkingVisible`, `pasteToEditor`, `getEditorText`,
  `addAutocompleteProvider`, `onTerminalInput`, `getToolsExpanded`,
  `setToolsExpanded`, `getTheme`, `getAllThemes`.
- **Settings:** `registerSettings` (adds a section to `/settings`; used by
  Cognee and provider settings).
- **Session/agent introspection:** `getContextUsage`, `isCompacting`,
  `compactAfterTurn`, `hasPendingMessages`, `hasPendingSteering`,
  `getSystemPromptOptions`, `getSessionName`, `getAllTools`, `getCommands`,
  `getFlag`, `refreshToken`.

Rust plugins register tools, commands, flags, shortcuts and providers, open
dialogs, side questions and `ask_user`, own stdio, and run child agents
(`crates/ice-extensions/src/actions.rs`). The list above must be added to
`PluginApi`/actions before §1–§5 can be built as plugins.

### 7.3 RPC protocol (`--mode rpc`)
- **ice:** `CA/modes/rpc/*` (~7.8k lines). JSONL commands, with a schema
  (`rpc-command-schema.ts`), response validation, command lanes and
  coordinator, request tracker, input admission, settings projection
  (`rpc-settings.ts`, 1120 lines) and views/pages. There is also a typed
  client, `rpc-client.ts`.
- **ice-rust:** `RunMode::Rpc` (`crates/ice-cli/src/app.rs:434`) is a headless
  lifecycle. It fires `SessionStart` so a plugin can own stdio, but implements
  no JSONL command set. `crates/ice-cli/src/remote/protocol.rs` has a separate
  9-command TCP protocol (`Prompt, Abort, GetState, SetModel, SetThinkingLevel,
  SetActiveTools, Ping, Stop, UiDialogResponse`).
- **Missing ice RPC commands (79):** `abort, abort_bash, abort_branch_summary,
  abort_compaction, abort_models_refresh, abort_retry, background_bash, bash,
  cancel_request, clear_queue, clone, compact, cycle_model,
  cycle_thinking_level, delete_session, export_html, export_jsonl, follow_up,
  fork, get_available_models, get_available_thinking_levels, get_capabilities,
  get_changelog, get_command_schema, get_commands, get_completions,
  get_diagnostics, get_entries, get_flags, get_fork_messages,
  get_last_assistant_text, get_messages, get_models, get_providers, get_queue,
  get_resources, get_scoped_models, get_session_stats, get_settings, get_shell,
  get_shells, get_shortcuts, get_state, get_subagent_messages, get_subagents,
  get_system_prompt, get_tools, get_tree, get_ui_state, import_session,
  invoke_command, invoke_shortcut, kill_shell, list_sessions, navigate_tree,
  new_session, prompt, refresh_models, reload, remove_queued_message,
  rename_session, respond_subagent_approval, set_active_tools,
  set_auto_compaction, set_auto_retry, set_entry_label, set_fast_mode,
  set_follow_up_mode, set_model, set_scoped_models, set_session_name,
  set_setting, set_steering_mode, set_thinking_level, steer, stop_after_turn,
  subagent_control, subagent_patch, switch_session`.
- **Missing events:** `catalog_changed`, `settings_changed`, `models_changed`,
  `session_changed`, `shells_update`, `subagents_update`, `extension_ui_request`.

### 7.4 Settings keys not in Rust (`CA/core/settings-manager.ts`)
`bashAutoBackgroundMs`, `bashSilenceAlertMs`, `bashPromptAlertMs`,
`bashInputApproval` (§3); `compaction.midRunCompaction`, `thresholdPercent`
(§6.2); `defaultAgent`, `disabledAgents`, `askTimeoutMs` (§1); `fastMode`;
`blockImages` (never send images to providers); `imageWidthCells` (default 60);
`anthropicExtraUsage` (default true); `footerStyle: compact|legacy`;
`toolStyle: compact|boxed`; `defaultProjectTrust` (global only, default
`ask`); `npmCommand` (argv for npm lookups); `enableInstallTelemetry`,
`enableAnalytics`, `trackingId`; `enableSkillCommands`; `favoriteModels`
(pinned in the model selector); `treeFilterMode` (`/tree` default filter);
`appearance` (v1 appearance object, §7.5); `websocketConnectTimeoutMs`;
`fullscreenScrollbar`. Also `globalFirstSettings: true`, which the ice launcher
passes to `main()` so global settings win over project ones.

### 7.5 Interactive TUI components missing

**Phase 0 TUI parity landed 2026-10-21** (PRs [#1](https://github.com/Zyk-Void/ice-rust/pull/1), [#2](https://github.com/Zyk-Void/ice-rust/pull/2), [#27](https://github.com/Zyk-Void/ice-rust/pull/27), [#28](https://github.com/Zyk-Void/ice-rust/pull/28), [#29](https://github.com/Zyk-Void/ice-rust/pull/29)). This covers the reachable defects from TUI-226/244/248/254, grapheme width and wrapping (TUI-201/202), grapheme-cell slicing and OSC 8 handling (TUI-203/204), and Text/TruncatedText/Box/Spacer contracts (TUI-205). PR #29 changes the public `ice-tui` component API and rendering behavior. The detailed `docs/ice-gap/tui.md` audit is not present on this branch; this note records only the landed Phase 0 scope, not completion of the broader TUI parity audit.

**Selected `goal.md` items landed 2026-10-21** (PR [#44](https://github.com/Zyk-Void/ice-rust/pull/44),
squash-merged as `44072f0`, replacing the older stacked PRs #6-#24 and #33-#43).
It adds cancellable, durable tree navigation summaries with a three-way/custom
prompt chooser (TUI-067), a category grid and largest-entry context projection
(TUI-058), an acknowledged old-event drain before shared session replacement,
and run summaries driven by authoritative completed outcomes (TUI-043). Tree
replay follows the active branch. `branchSummary.reserveTokens` is wired through
CLI startup; the harness exposes additive reserve getter/setter methods without
changing required options, plugin ABI, or persistence schemas. The public
agent-event enum adds an ordered `RunSettled { run_id }` marker (not persisted;
filtered from JSON/RPC output); downstream exhaustive matches must handle it.
See `docs/guides/branch-summaries.md` and `docs/ice-gap/tui-goal-followup.md`.

Known gaps after the merge: resuming a session created in another directory
keeps tools in the current directory (a note says so); Ctrl+C, Ctrl+D, Esc, Tab
and Up/Down are fixed keys, not rebindable; real-terminal checks (mouse copy,
vim and `code --wait`, OSC 52 over SSH, light/dark detection) and Rust 1.78 are
not run. This is not a claim that every parity-audit entry is complete.

From `CA/modes/interactive/components/`:
- **Appearance:** `appearance-customizer.ts` (3110 lines) edits semantic theme-token
  groups, one group per editor screen. `appearance-preview.ts` adds live preview
  scene tabs. Rust has static themes (`crates/ice-tui/src/theme.rs`) only.
- **Compact tool rendering:** `tool-group.ts` and `tool-summary.ts` group
  read/search/list calls into exploration summary rows (`toolStyle: compact`).
  `tool-toggle-link.ts` adds click-to-expand.
- **Navigation:** `sticky-prompt.ts` pins the current prompt and clicks back to
  it. `new-messages-pill.ts` scrolls to the end and counts newly visible
  assistant turns (ignoring blank/tool-only entries); Rust's `jump_to_end.rs`
  displays that count with the configured bottom-navigation key.
- **Panels:** `notices-view.ts` (NoticeCenter panel with a filter bar),
  `shells-panel.ts` (§3), `subagent-view-switcher.ts` and the Agent Hub (§1.9).
- **Status:** `run-summary.ts` (dim "worked for …" line after a run),
  `session-header.ts`, `skill-invocation-message.ts` (collapsible).
- **Selectors:** `config-selector.ts` (§6.3), `user-message-selector.ts`
  (choose a message to fork from; verify against Rust fork UI),
  `trust-selector.ts`, `oauth-selector.ts`, `login-dialog.ts`. Rust has login in
  `crates/ice-cli/src/auth.rs`; verify UI parity.
- **Easter eggs:** `armin.ts`, `daxnuts.ts`. Skip.

### 7.6 `ICE_*` environment variables not read by Rust
Feature-bound (see the sections above): Cognee ×10, subagents ×8, `ICE_GOALS_HOST_KEY`,
`ICE_AGENT_VIEW_COLORS`, `ICE_LOCAL_BASE_URL`, `ICE_LOCAL_API_KEY`, `ICE_OMNI_BASE_URL`.
General:
- `ICE_MODEL`, `ICE_REASONING_LEVEL`: default model and thinking.
- `ICE_SESSION_FILE`, `ICE_LABEL`.
- `ICE_SETTINGS`, `ICE_SETTINGS_COMMAND_NAME`, `ICE_SETTINGS_SCHEMA_ID`.
- `ICE_PACKAGE_DIR`.
- `ICE_SKIP_VERSION_CHECK`, `ICE_UPDATE_URL`, `ICE_TELEMETRY`.
- `ICE_SHARE_VIEWER_URL`: `/share` viewer.
- `ICE_ACTION_URL_PREFIX`.
- `ICE_HARDWARE_CURSOR`, `ICE_CLEAR_ON_SHRINK`: TUI rendering.
- `ICE_STARTUP_BENCHMARK`.

---

## 8. Remote sessions: protocol / client / server / storage packages

- **ice:**
  - `ice:protocol/src/{schemas,codec,framing,cbor}.ts`: runtime-neutral,
    length-prefixed CBOR framing. Messages include `hello`/`hello_error`,
    `request`/`response`/`event`, `create`, `attach`, `detach`, `list`,
    `prompt`, `abort`, `server_snapshot`, `session_snapshot`,
    `session_progress`, `session_removed`, `session_locked`, item
    `started/updated/finished`, and `assistant_delta`.
  - `ice:client/src/*`: `IceClient` over a `ByteTransport`, session handles,
    state, unix transport.
  - `ice:server/src/*`: multi-session server with listener, snapshots and a
    unix transport.
  - `ice:storage/sqlite-node`: SQLite binding used by §4.
- **ice-rust:** `crates/ice-cli/src/remote/*` is its own TCP protocol with a
  token (`--server` / `--connect`). It is wire-incompatible with ice's CBOR
  protocol and single-session. Decide whether ice-rust should speak ice's protocol.

---

## 9. AI layer (`ice:ai`)

### 9.1 Wire APIs
Rust implements 4: `anthropic-messages`, `openai-completions`, `openai-responses`,
and `openai-codex-responses` (SSE)
(`crates/ice-ai/src/providers/`). The closed `ice_ai::WireProtocol` enum
(`crates/ice-ai/src/model.rs`) lists exactly these. The provider factory, the
default endpoints and `models.json` `compat` parsing all match on it
exhaustively, so a new protocol is a compile-checked addition. Other `api`
strings (provider packages, older sessions) stay `Api::Other` and are not
served by a built-in protocol.

**Scope decision (2026-10-22):** icers speaks the OpenAI-compatible and
Anthropic-compatible wire protocols, plus the ChatGPT subscription (Codex)
endpoint. It does not port the other wire APIs below or the per-provider
catalog in §9.2. Route other providers through an OpenAI- or
Anthropic-compatible gateway (Omni router) instead.

Not planned (`ice:ai/src/api/*`), except `openai-codex-responses`:

| API | File | Notes |
|---|---|---|
| `google-generative-ai` | `google-generative-ai.ts` + `google-shared.ts` | Gemini REST + SSE |
| `google-vertex` | `google-vertex.ts` | Vertex auth/endpoint over the shared Google code |
| `bedrock-converse-stream` | `bedrock-converse-stream.ts` (42 KB) | AWS SigV4, event-stream decoding |
| `mistral-conversations` | `mistral-conversations.ts` | |
| `azure-openai-responses` | `azure-openai-responses.ts` | Azure deployment URLs on top of Responses |
| `openai-codex-responses` | `openai-codex-responses.ts` (54 KB) | Built-in ChatGPT login and SSE Responses implemented; WebSockets, compression and device-code login remain out of scope. See `docs/guides/chatgpt-subscription.md` |
| `ice-messages` | `ice-messages.ts` | ice's own protocol: one POST `{model,context,options}` to `<baseUrl>/messages`, SSE of serialized assistant events plus `done`/`error`; used by the Radius gateway or any `"api": "ice-messages"` custom provider |
| OpenRouter images | `openrouter-images.ts` | Rust has `crates/ice-ai/src/images.rs`; verify parity |

### 9.2 Built-in provider catalog
- **ice:** `ice:ai/src/providers/all.ts` registers 41 providers with generated
  model data (`models.generated.ts`, `providers/data/`): amazon-bedrock,
  ant-ling, anthropic, azure-openai-responses, baseten, cerebras,
  cloudflare-ai-gateway, cloudflare-workers-ai, deepseek, fireworks,
  github-copilot, google, google-vertex, groq, huggingface, kimi-coding,
  minimax(+cn), mistral, moonshotai(+cn), nvidia, openai, openai-codex,
  opencode(+go), openrouter(+images), qwen-token-plan(+cn), radius, together,
  vercel-ai-gateway, xai, xiaomi(+3 token-plan regions), zai(+coding-cn).
- **ice-rust:** built-in native providers are `anthropic`, `openai`, and
  `openai-codex`. Everything else must be declared in `models.json` or a provider package
  (`crates/ice-cli/src/config.rs:1518` test
  `native_provider_api_key_env_mapping_uses_exact_provider_ids`).

### 9.3 Auth / OAuth
- **ice:** `ice:ai/src/auth/{credential-store,resolve,context,helpers}.ts`,
  `auth/oauth/{anthropic,openai-codex,github-copilot,kimi-coding,openrouter,radius,xai,device-code,pkce,oauth-page}.ts`,
  `env-api-keys.ts`. Built-in PKCE and device-code flows per provider, plus a
  local callback page.
- **ice-rust:** `openai-codex` has built-in PKCE browser/manual login,
  account-bound credential storage and request-time refresh
  (`crates/ice-cli/src/oauth_codex.rs`, `codex_auth.rs`). CLI and TUI reuse the
  same protocol implementation. Extension-owned OAuth retains precedence;
  other OAuth providers still require provider extensions. Device-code login
  and Copilot authentication remain unimplemented.

### 9.4 Other ai utilities to check
`ice:ai/src/legacy-api-aliases.ts` (old API names), `compat.ts`,
`utils/provider-retry.ts`, `utils/overflow.ts` (context-overflow detection),
`utils/deferred-tools.ts`, `utils/node-http-proxy.ts` (HTTP proxy support),
`radius-config.ts` (gateway catalog refresh).

### 9.5 ice local provider + fast mode
- `CA/ice-provider.ts`: `refreshLocalModelsForStartup()` runs before every
  launch unless `--offline`. It fetches the local endpoint catalog
  (`ICE_LOCAL_BASE_URL`, `ICE_LOCAL_API_KEY`, `ICE_OMNI_BASE_URL`) and maps
  `cx/*` models: `CX_MAX_CONTEXT_TOKENS`, `serviceTiers`, openai thinking
  format. On failure it keeps the last valid catalog.
- `CA/ice-provider-settings.ts`: provider submenu in `/settings`.
- **Fast mode:** `fastMode` / `--fast` / RPC `set_fast_mode` request the
  priority tier on verified `cx/gpt-5.6*`, `cx/gpt-6-luna`, `cx/gpt-6-sol` routes.
- Rust: missing.

---

## 10. Smaller ice-only pieces

| Item | ice source | What it does | Rust |
|---|---|---|---|
| llama.cpp router | `CA/extensions/llama/{client,provider,huggingface,ui,index}.ts`, `docs/llama-cpp.md` | `/login llama.cpp`, `/llama` to load/unload router models and download GGUF from Hugging Face (`LLAMA_BASE_URL`) | missing |
| Claude SDK bridge | `CA/claude-sdk/{adapter,extension}.ts` | Bridge client to a Claude SDK process; private config, record reader | missing |
| Legacy compat | `CA/core/legacy-compat/{env,paths,identity,extension-aliases,cognee}.ts`, `CA/migrations.ts` | Maps old pi/ice env vars, paths and extension names; one-time startup migrations | n/a until users migrate |
| pi package interop | `CA/ice.ts` sets `PI_CODING_AGENT_DIR ??= agentDir` | lets pi packages (e.g. `pi-mcp-adapter`) read ICE's `mcp.json` | Rust cannot load pi TS packages (§0) |
| HTTP dispatcher | `CA/core/http-dispatcher.ts` | global proxy/dispatcher config | verify |
| Output guard, context breakdown, usage totals | `CA/core/{output-guard,context-breakdown,usage-totals}.ts` | output size guard; `/context`-style breakdown; usage aggregation | verify |
| Evals | `ice:evals/src/*` | vitest-evals harness running real `AgentSession`s in temp dirs | missing |
| Containerization docs | `docs/containerization.md` | Gondolin micro-VM extension, Docker, OpenShell patterns | docs only |

---

## 11. Rebrand-specific risks found while auditing

1. **Supply-chain risk in `ice install`.** Companion plugins resolve by name on
   crates.io (`crates/ice-cli/src/prebuilt_extensions.rs:12`, `CRATES_API`). The
   rename turned `rpi-todo`, `rpi-subagents`, etc. into `ice-*` names that
   upstream never published. Anyone who registers `ice-todo` on crates.io with
   a matching release manifest could get a native library loaded by
   `ice install ice-todo`. Fix before anyone runs `ice install`: either keep
   upstream package names in the rebrand, or pin a Zyk-Void-owned repository.
2. **Plugin ABI rename.** The entry symbol is now `ice_plugin_register`
   (was `rpi_plugin_register`), so upstream prebuilt plugins will not load.
3. **Self-update** now points at `https://github.com/zyk-void/ice-rust` releases
   (`crates/ice-cli/src/prebuilt_release.rs:11`). None exist yet, so updates
   fail closed.
4. **Runtime identity.** To keep it from touching TS ice's state, the Rust
   build runs as `icers` with `~/.icers/agent`, project `.icers/`, and `ICERS_*`
   env vars (see `FORK.md`). This report still writes "ice-rust" for the
   codebase, and `ICE_*` when it names TS ice variables. Their Rust
   equivalents are `ICERS_*`.

---

## 11a. Found while importing `~/.ice/agent` into `~/.icers/agent` (2026-10-19)

1. **`ultra` thinking level.** ice has 7 levels: `minimal|low|medium|high|xhigh|max|ultra`
   (`ice:ai/src/types.ts:80`). Rust's enum stops at `max`. Rust's `models.json`
   parser rejects the whole file when a `thinkingLevelMap` has an `ultra` key
   (`unknown variant 'ultra'`). The import dropped 3 such keys (all
   `ultra → xhigh`). Port: add `Ultra` to the Rust thinking level, or at least
   ignore unknown map keys instead of failing the entire catalog.
2. **Built-in provider auth with no catalog.** `auth.json` holds keys for `zai`,
   `opencode`, `opencode-go` and `modal`. TS ice serves the `zai` and
   `opencode*` models from its built-in catalog (§9.2). Rust has no built-in
   entries for them, so those keys are unused until the providers are
   declared in `models.json` or §9.2 is ported.
3. **Themes.** The custom theme `lavender-ice` (`~/.ice/agent/themes/*.json`)
   was not imported. The Rust theme format and loader parity with the TS JSON
   themes are unverified (§7.5).
4. **Settings.** Only `defaultProvider`, `defaultModel`, `defaultThinkingLevel`,
   `steeringMode` and `followUpMode` were imported. The rest are TS-only
   (`packages`, `extensions`, `appearance`, the `ice.*` namespace, `fastMode`,
   `favoriteModels`, `uiMode`, `statusLine`, `transport`, `async`; see §7.4).

---

## 12. Suggested porting order

1. Resolve §11 (package names and ABI symbol) before any release.
2. Extend the plugin host API (§7.2): `registerSettings`, widgets/working
   indicator, custom entries, context usage, `compactAfterTurn`. Every later
   port needs these.
3. Background shells (§3): self-contained, high daily value.
4. Plan/build guarded modes (§2): one extension's worth of logic, clear spec.
5. Skill hot reload, settings keys, CLI flags (§6.3, §7.1, §7.4).
6. ~~Observability store + dashboard (§4).~~ Ported; the Cognee views wait for §5.
7. Subagents V2 (§1) in slices: definitions, then `agent` with frozen
   contracts, then registry/persistence, permissions, worktree patches, Hub.
8. AI wire APIs + provider catalog (§9).
9. JSONL RPC parity (§7.3), then decide on the CBOR remote protocol (§8).
10. llama.cpp, Claude SDK (§10) as plugins. Cognee and Blackhole (§5, §6.1) are
    ported as built-ins.
