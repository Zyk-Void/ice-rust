# Built-in tools, file mutation, command execution and shell monitoring

Source-only comparison against the frozen TypeScript void worktree and reviewed active Rust checkout, preserving the original-source provenance. This area covers both coding-agent tools and the separate runtime-neutral agent-harness tool implementations. Similar tool names do not establish parity. Existing tests are read as behavioral evidence, never executed. All paths, process operations and input effects below describe the audited software, not actions performed during the audit.

## TOOLS-001 - Bash invocation schema, zero-timeout normalization and hard ceiling

### 1. Title and status

Bash invocation schema, zero-timeout normalization and hard ceiling. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/tools/bash.ts:33-87; ice:packages/coding-agent/src/core/tools/bash.ts:233-256; ice:packages/coding-agent/src/core/tools/bash.ts:375-419; ice:packages/coding-agent/src/core/tools/bash.ts:532-598. Symbols resolveTimeoutMs, BashToolInput, BashToolOptions, createBashToolDefinition. Tests: ice:packages/coding-agent/test/bash-background.test.ts:373-391 and ice:packages/coding-agent/test/bash-background.test.ts:533-552.

Named export: `BashToolDetails` (`ice:packages/coding-agent/src/core/tools/bash.ts:80`).

### 3. What it does

Accept the model's command and optional description, timeout and background/input requests while preventing a child agent from defeating an execution deadline.

### 4. How it works

Validate the requested timeout before creating a process or adopting a background shell. A host-supplied maximum is a ceiling, whereas omission or zero in the model request means no requested timeout. These rules apply to both foreground and background execution.

resolveTimeoutMs(undefined) and resolveTimeoutMs(0) return undefined. Otherwise reject non-finite or non-positive numbers with `Invalid timeout: must be a finite number of seconds`; multiply seconds by 1000 and reject results above 2,147,483,647 ms with `Invalid timeout: maximum is 2147483.647 seconds`. `maxTimeoutMs` is taken as `options?.maxTimeoutMs || undefined`; when set, it replaces an omitted or longer requested timeout, and leaves a shorter timeout unchanged. The execution backend receives the resolved timeout divided by 1000, or undefined, so custom backends see the same normalization. The command prefix, when truthy, is prepended with exactly one newline. An undefined/null process exit code is not treated as nonzero. A defined nonzero exit throws output plus a blank line and `Command exited with code N`; abort becomes output plus `Command aborted`; a backend error `timeout:<seconds>` becomes output plus `Command timed out after <seconds> seconds`. Other backend errors propagate. No automatic retry exists in this tool. The caller's schema validator owns rejecting incorrectly typed fields; `description` length is descriptive guidance, not enforced.

### 5. Interfaces

```ts
{ command: string; timeout?: number; description?: string; run_in_background?: boolean; stdin?: boolean }
```
Tool name/label: `bash`; required field: command. Host options include `commandPrefix?: string`, `maxTimeoutMs?: number`, `autoBackgroundMs?: number`, `background?: BashBackgroundRegistry`, `exposeSessionEnvironment?: boolean`, `spawnHook?: BashSpawnHook`, `operations?: BashOperations`, `shellPath?: string`. Details: `{ truncation?: TruncationResult; fullOutputPath?: string; backgroundTaskId?: string; backgroundOrigin?: "model" | "user" }`. The complete TypeBox schema is ice:packages/coding-agent/src/core/tools/bash.ts:50-76. No additional RPC shapes, persistent settings keys, slash commands, keybindings or files are declared here; session configuration and control-tool integration are separate dependencies.

### 6. Dependencies

`AGENT-048`, `AGENT-050`, `AGENT-052`, `SET-014`

External services: none intrinsically. Requires a selected local shell executable or supplied BashOperations implementation. Session/subagent callers choose host ceilings; those integration entries must cross-reference this contract.

### 7. ice-rust today

rust:crates/ice-tools/src/tools/bash.rs:38-83; rust:crates/ice-tools/src/tools/bash.rs:136-162; rust:crates/ice-tools/src/tools/bash.rs:256-302; rust:crates/ice-tools/src/tools/bash.rs:405-421. Rust BashInput contains only command and timeout; description/run_in_background/stdin have no schema or execution meaning. Rust rejects timeout 0. It uses `input.timeout.or(default_timeout)` instead of a maximum, so an explicit larger timeout overrides the host fallback. The same upper bound is present. Rust timeout error text is `Command timed out after <seconds>s`, versus TypeScript's `... after <seconds> seconds`. Rust serializes details as snake_case full_output_path rather than fullOutputPath. Existing nonzero/abort/small-output tests confirm those limited contracts: rust:crates/ice-tools/tests/bash.rs:63-155; they were read, not run.

### 8. Port notes

Extend ice-tools BashInput and options with separate default and hard-maximum semantics; normalize zero before applying the host ceiling. Preserve Rust's async prepare hook as a distinct capability and add the TypeScript-compatible result adapter. Keep all terminal errors explicit so the single agent loop records the proper tool error. Use Duration conversion only after finite/range validation; do not copy a model-supplied timeout directly into an unchecked timer.

### 9. Size and priority

Estimated Rust implementation: **100-180 lines** (estimate, not measured source). **P0** - A model can currently defeat Rust's fallback timeout, and core TypeScript Bash controls have no usable Rust contract.

## TOOLS-002 - Bash spawn rewriting and explicit session environment exposure

### 1. Title and status

Bash spawn rewriting and explicit session environment exposure. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/tools/bash.ts:93-199; ice:packages/coding-agent/src/core/tools/bash.ts:201-255; ice:packages/coding-agent/src/core/tools/bash.ts:375-419; ice:packages/coding-agent/src/core/tools/bash.ts:532-552. Symbols BashOperations, createLocalBashOperations, BashSpawnContext, BashSpawnHook, resolveSpawnContext.

### 3. What it does

Let hosts replace command execution or rewrite the command, working directory and environment while exposing current session/model metadata only through the intended spawn context.

### 4. How it works

The tool builds a fresh spawn context per invocation and passes the rewritten context to its backend. Its local backend combines stdout and stderr callbacks, handles process-tree cancellation and refuses incompatible stdin transport before spawning. Backend hooks are executable host code and must not be inferred from repository-controlled tool output.

resolveSpawnContext starts with a copy of getShellEnv(), calls clearSessionEnvironment to remove inherited session metadata, and, when exposeSessionEnvironment defaults true and an ExtensionContext exists, sets ICE_SESSION_ID, optional ICE_SESSION_FILE, model ICE_PROVIDER/ICE_MODEL and optional ICE_REASONING_LEVEL. The synchronous spawnHook receives `{command,cwd,env}` and must return the entire replacement object. There is no merge after the hook. Local operations resolve the configured shell, check cwd with F_OK, and spawn detached on non-Windows with windowsHide true; stdio is input pipe only when needed for command transport or onStdin, otherwise ignored, and output/error pipes always. Env is the supplied context env or getShellEnv(). Stdin-transport shells get the command via stdin and cannot also provide an interactive stdin pipe. Child PID is registered with detached-child tracking and delivered to onSpawn, then untracked in finally. Timeout and abort invoke killProcessTree; the timeout flag is interpreted after waitForChildProcess. Cleanup removes abort listener and timeout. A pre-aborted signal fails before spawn. No retry or persistence is defined here.

### 5. Interfaces

```ts
interface BashSpawnContext { command: string; cwd: string; env: NodeJS.ProcessEnv }
type BashSpawnHook = (context: BashSpawnContext) => BashSpawnContext;
```
BashOperations.exec(command, cwd,{onData, signal?, timeout?, env?, onStdin?, onSpawn?}) returns Promise<{exitCode:number|null}>; timeout is seconds. Local stdin+command-stdin conflict throws `This shell reads its command from stdin, so the command's stdin cannot be kept open.` A missing cwd throws `Working directory does not exist: <cwd>\nCannot execute bash commands.` ICE_SESSION_ID/ICE_SESSION_FILE/ICE_PROVIDER/ICE_MODEL/ICE_REASONING_LEVEL are runtime-injected environment variables, not new persisted files. Other CLI, RPC, slash-command and keybinding interfaces: none here.

### 6. Dependencies

`AGENT-049`, `TOOLS-001`

Depends on shell selection/environment/process-tree helpers already covered by the agent/runtime audit; coordinator must resolve exact cross-area references. An installed shell or injected remote execution backend is required.

### 7. ice-rust today

rust:crates/ice-tools/src/tools/bash.rs:57-83 and rust:crates/ice-tools/src/tools/bash.rs:146-159 provide an asynchronous mutable BashPrepare hook with command/cwd/env/inherit_env, which is a deliberate host API difference and useful Rust-only capability to retain. It starts per-call env empty and inherit_env true rather than constructing the TypeScript session context. rust:crates/ice-tools/src/os_env.rs:840-867 removes host-private inherited variables then applies configured shell_env and per-call env. rust:crates/ice-tools/src/env.rs:96-111 has no onStdin/onSpawn or command-handle return. Rust has local shell override at environment level, but no per-tool exposeSessionEnvironment switch or the TypeScript five-field injection at this tool entrypoint. Session-side injection may be performed by another host adapter and must not be presumed absent merely from this file.

### 8. Port notes

Keep BashPrepare's async design but document exact ordering and adapt the TypeScript sync hook contract where compatibility is needed. Add explicit session metadata injection at the correct host boundary, never inherit stale parent session identities. Extend the execution interface with controlled process-handle/stdin access for later entries, and preserve remote backend substitution without pretending that all backends support interactive input.

### 9. Size and priority

Estimated Rust implementation: **100-180 lines** (estimate, not measured source). **P1** - Extensions and nested sessions require reproducible spawn context and explicit environment ownership.

## TOOLS-003 - Immediate background launch with independent process cancellation ownership

### 1. Title and status

Immediate background launch with independent process cancellation ownership. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/core/tools/bash.ts:409-441; ice:packages/coding-agent/src/core/tools/bash.ts:483-499; ice:packages/coding-agent/src/core/tools/bash.ts:525-624; ice:packages/coding-agent/src/core/tools/bash-background.ts:177-252. Tests: ice:packages/coding-agent/test/bash-background.test.ts:130-155; ice:packages/coding-agent/test/bash-background.test.ts:408-421; ice:packages/coding-agent/test/bash-input.test.ts:32-75.

### 3. What it does

Start a server, watcher or long job as a background shell, immediately return a stable task ID and keep it alive across tool-call and model-turn completion.

### 4. How it works

Explicit background mode transfers process cancellation ownership from the tool invocation to a session registry. Output continues into a full-output accumulator and per-shell tail after the tool result has returned. The registry converts the eventual success or error into one completion payload rather than rejecting a detached promise.

Reject run_in_background when no background registry exists. Reject stdin:true unless run_in_background:true; reject interactive stdin with any custom operations backend. Before explicit background spawn, compare runningCount() with MAX_RUNNING_SHELLS=16 and reject a seventeenth. The cap applies at this explicit-launch entrypoint; adopt itself and user/automatic foreground releases do not enforce it. A separate execAbort controller initially follows the tool signal; explicit background mode sets detached=true and removes the forwarding listener before starting work. A signal already aborted remains aborted in execAbort. Persist the output log before runToCompletion. Adoption uses original command (not rewritten prefix), start time, an abort closure, settledOutcome (both fulfillment and rejection normalized to text/isError/optional exitCode), origin model, optional description and logIssue callback. If spawn/stdin callbacks happen before adoption, keep their values and attach them after the ID exists. Subsequent raw output is decoded with a streaming TextDecoder and appended to the registered shell. `onUpdate` stops emitting once detached. The launch result does not establish eventual success; a synchronous backend failure becomes the later failed completion. No process restart or automatic retry occurs.

### 5. Interfaces

Input additions: `run_in_background?: boolean`, `stdin?: boolean`, `description?: string`. Immediate details: `{backgroundTaskId:"bash-N",backgroundOrigin:"model",fullOutputPath:<log>}`. Result text tells the model to continue or end its turn, use bash_output, bash_input when requested, and bash_kill. Errors include `Background shells are not available here; run the command without run_in_background.`, `stdin: true requires run_in_background: true.`, `stdin: true is only supported by the local shell backend.`, and `16 background shells are already running. Stop ones you no longer need with bash_kill (bash_output lists them) before starting another.` No separate RPC command or CLI flag is declared. Process state is in-memory; log format is covered by TOOLS-010.

### 6. Dependencies

`TOOLS-001`, `TOOLS-002`

Requires the session-owned registry lifetime and later completion-delivery integration. External dependency is the shell/backend that owns the process; no external service is intrinsically required. Session integration is reviewed in SESSION-023 (tool registry/context), SESSION-024 (user Bash), and SESSION-025 (background delivery, wake and disposal); these are consumer links, not prerequisites for implementing the tool registry.

Final dependency classification. Consuming integrations: `SESSION-023`, `SESSION-025`, `RPC-026`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

rust:crates/ice-tools/src/tools/bash.rs:38-46; rust:crates/ice-tools/src/tools/bash.rs:136-302 is a single awaited foreground execution. rust:crates/ice-tools/src/env.rs:96-111 returns only a final ShellOutput and exposes no transferable process handle or stdin writer. rust:crates/ice-tools/src/os_env.rs:903-917 deliberately closes stdin after writing a command if needed. Behavioral search across crates found user !command handling in ice-cli but no model-call registry, run_in_background lifecycle, background IDs or detached model-tool control surface. The user TUI command path must not be classified as equivalent to this model-facing operation.

### 8. Port notes

Add a session-owned background task manager, not a new agent controller. Give each process its own cancellation token, transfer ownership explicitly before the tool returns, and normalize detached outcomes into the registry. Use bounded output channels and retain process-group/stdin handles behind per-task state. Enforce the documented explicit-launch cap while making any stronger cap across automatic releases an intentional design decision.

### 9. Size and priority

Estimated Rust implementation: **220-360 lines** (estimate, not measured source). **P0** - Servers and long commands otherwise occupy a model tool call until exit or timeout.

## TOOLS-004 - Foreground release by user, queued steering or elapsed-time threshold

### 1. Title and status

Foreground release by user, queued steering or elapsed-time threshold. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/core/tools/bash.ts:626-701; ice:packages/coding-agent/src/core/tools/bash-background.ts:21-24; ice:packages/coding-agent/src/core/tools/bash-background.ts:86-96; ice:packages/coding-agent/src/core/tools/bash-background.ts:152-170. Tests: ice:packages/coding-agent/test/bash-background.test.ts:38-105; ice:packages/coding-agent/test/bash-background.test.ts:236-253; ice:packages/coding-agent/test/bash-background.test.ts:584-615.

### 3. What it does

Release an already running model Bash call into background state when the user asks, a steering message needs attention or a configured duration has elapsed.

### 4. How it works

A foreground invocation registers a release closure and races its execution against that release promise. The same process survives release, and output already returned to the model is excluded from the next incremental read. Completion, abort and release are resolved without intentionally restarting the command.

registerForeground keys by toolCallId; unregister only removes the matching release closure, protecting a replacement registration. releaseAll snapshots and clears the foreground map before calling every release, and returns the number of registrations released. A 250 ms interval checks hasQueuedSteer only after 5000 ms elapsed; the helper catches a stale context exception and treats it as no steer. Follow-up-only queued messages do not satisfy hasPendingSteering. An auto-background timer is installed only when autoBackgroundMs is truthy and the resolved timeout is absent or strictly longer than that threshold. Its callback does nothing after completionSettled or caller abort. Race success returns the completed result when execution wins; execution rejection re-awaits and propagates its error. Finally always clears polling/timer and unregisters. On release, set detached, stop forwarding caller cancellation, take the current output snapshot, persist the original full log, and adopt the same process. Initial registry output is the current truncated snapshot; adoption initializes its readCursor to the initial total, so already-returned output is not repeated. Origin is model only for timer release without a message; user for ctrl+b/message release. Output-prefix text distinguishes those three reasons. No running-shell cap is checked on this release path.

### 5. Interfaces

Internal methods: registerForeground(toolCallId, command, release):()=>void; hasReleasable():boolean; releaseAll():number. Constants: USER_MESSAGE_POLL_MS=250, FOREGROUND_MESSAGE_GRACE_MS=5000. Host option autoBackgroundMs defaults unset/off at tool level; session may supply another default. Details after release contain backgroundTaskId, backgroundOrigin, fullOutputPath and optional truncation. User-facing key is described in source as ctrl+b; actual keybinding registration belongs to TUI. No persisted control state or new RPC schema in this helper.

### 6. Dependencies

`SET-014`, `TOOLS-003`, `TOOLS-005`

Session hasPendingSteering determines wakeup, and TUI invokes releaseAll. Those integrations require cross-area references from the coordinator. No additional external binary beyond the running shell. Session integration is reviewed in SESSION-023 (tool registry/context), SESSION-024 (user Bash), and SESSION-025 (background delivery, wake and disposal); these are consumer links, not prerequisites for implementing the tool registry.

Final dependency classification. Consuming integrations: `SESSION-025`, `TUI-074`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

rust:crates/ice-tools/src/tools/bash.rs:136-302 awaits capture and has no foreground registration/release race, queued-message poll, process-handle transfer or auto-background threshold. Its default_timeout fallback terminates a command and cannot preserve it as a background task. rust:crates/ice-tools/src/env.rs:96-111 exposes final execution rather than a handle. User-origin !command routing does not detach a model-issued command after it has started.

### 8. Port notes

Add a release signal to the existing Bash task ownership model and a session-level registry reference. Carefully preserve race ordering: a finished/failed command must return its genuine outcome, not a fabricated background success. Use cancellation-safe timers and avoid counting follow-ups as steering. Keep the release mechanism independent from runtime UI rendering so RPC and SDK hosts can use it.

### 9. Size and priority

Estimated Rust implementation: **140-230 lines** (estimate, not measured source). **P0** - The user must be able to interrupt waiting without destroying useful long-running work.

## TOOLS-005 - Background shell snapshots, bounded retention and completion state machine

### 1. Title and status

Background shell snapshots, bounded retention and completion state machine. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/core/tools/bash-background.ts:35-84; ice:packages/coding-agent/src/core/tools/bash-background.ts:98-150; ice:packages/coding-agent/src/core/tools/bash-background.ts:177-267; ice:packages/coding-agent/src/core/tools/bash-background.ts:354-436; ice:packages/coding-agent/src/core/tools/bash-background.ts:559-669. Tests: ice:packages/coding-agent/test/bash-background.test.ts:39-63; ice:packages/coding-agent/test/bash-background.test.ts:255-271; ice:packages/coding-agent/test/bash-background.test.ts:349-369; ice:packages/coding-agent/test/bash-background.test.ts:556-580.

Named export: `BASH_BACKGROUND_COMPLETED_CUSTOM_TYPE` (`ice:packages/coding-agent/src/core/tools/bash-background.ts:27`).

Named export: `summarizeShellCommand` (`ice:packages/coding-agent/src/core/tools/bash-background.ts:30`).

Named export: `BashBackgroundCompletion` (`ice:packages/coding-agent/src/core/tools/bash-background.ts:35`).

Named export: `BackgroundShellRead` (`ice:packages/coding-agent/src/core/tools/bash-background.ts:133`).

`ice:packages/coding-agent/src/core/tools/bash-background.ts:27-33` (`BASH_BACKGROUND_COMPLETED_CUSTOM_TYPE`, `summarizeShellCommand`).

### 3. What it does

Keep running and recently completed shells inspectable with their original command, status, timing, output and log condition.

### 4. How it works

The registry owns in-memory shell records and monotonic IDs for one session. It retains a bounded suffix of output while tracking lifetime character counts, and emits a completion when an adopted execution settles naturally. Log files are the only persistent payload owned here; the registry itself is not serialized or restored after restart.

IDs are bash-1, bash-2 and so on. Adoption creates running status, startedAt and lastOutputAt from supplied start time, inputBytes=0, empty chunk list and read cursor; optional description is stored only when truthy. Initial output uses the same append method, then cursor is moved to totalChars. appendOutput ignores unknown/non-running/empty input; otherwise appends text, increments totalChars and retainedChars by JavaScript UTF-16 code units, drops exactly the excess over 262144 code units from oldest chunks, updates lastOutputAt=Date.now(), notifies change and invokes output listeners individually with thrown listener errors ignored. It may cut a surrogate pair because the cap is code units, not bytes or code points.

Natural settlement, unless disposed or no longer running, changes status to failed or completed from isError, sets endedAt and finalText, resolves exited, prunes, notifies change, then emits completion to each completion listener under independent try/catch. Completion includes taskId, command, text, isError, durationMs, optional actual exitCode, description, logPath, live logIssue text and lingering:true. Snapshots expose output joined from chunks and optional stdinOpen/lingering only when true. list returns running shells in insertion order; snapshots returns all in insertion order. get returns a fresh snapshot. Keep the newest 20 finished non-lingering records in insertion order, not by completion timestamp; running and lingering records are exempt. Dropping a record best-effort removes its log. trackLog retains foreground logs until dispose, or removes immediately if already disposed. Change listeners in this class are not caught; unlike start/output/completion listeners, a throwing change listener can interrupt the caller. The adopted settled promise is expected to resolve because the Bash tool normalizes errors first.

### 5. Interfaces

BackgroundShellStatus=`running|completed|failed|killed`. Snapshot fields: taskId, command, origin(model|user), description?, status, startedAt, endedAt?, lastOutputAt, output, totalChars, finalText?, logPath?, logIssue?, stdinOpen?, lingering?. Completion fields are specified above; custom type constant is `bash_background_completed`, but actual session message delivery is separate. Subscription APIs onStart(taskId), onOutput(taskId, text), onChange(), onComplete(completion) each return an unsubscribe closure. Limits: MAX_OUTPUT_CHARS=256*1024, MAX_FINISHED_SHELLS=20, MAX_RUNNING_SHELLS=16 (explicit launch enforcement only). No database format, recovery file or external network service.

The completion custom-type constant is `BASH_BACKGROUND_COMPLETED_CUSTOM_TYPE = "bash_background_completed"`. `summarizeShellCommand(command: string): string` applies `trimEnd()`, splits on LF, and returns the first line unchanged for one line or first line plus ` (+N more lines)` for multiple remaining lines. Trailing whitespace and empty trailing lines disappear before counting; leading whitespace is retained, and an all-whitespace input returns the empty first string.

### 6. Dependencies

`TOOLS-003`, `TOOLS-010`

Requires the process/task manager and session completion delivery, plus process-group liveness checks for retention. Rejected settlements should be normalized by the caller, matching settledOutcome. Session integration is reviewed in SESSION-023 (tool registry/context), SESSION-024 (user Bash), and SESSION-025 (background delivery, wake and disposal); these are consumer links, not prerequisites for implementing the tool registry.

Final dependency classification. Consuming integrations: `SESSION-025`, `RPC-026`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

rust:crates/ice-tools/src/tools/mod.rs:1-27 exports Bash/read/write/edit/find/grep/ls/image/PowerShell but no background state type. rust:crates/ice-tools/src/tools/bash.rs:87-124 holds only schema/environment/options and loses task-local state after execute. rust:crates/ice-tools/src/os_env.rs:898-901 and 1067-1070 tracks active PIDs for environment cleanup but does not retain per-task outcomes, tails, read cursors or inspectable completion records. The generic PID set is not this registry.

### 8. Port notes

Implement a session-scoped ordered registry with unique IDs, explicit running/terminal states and a bounded tail. Use Rust-owned snapshots so consumers cannot mutate records. Preserve whether limits count bytes, scalars or UTF-16 units in any compatibility layer; document a deliberate safer Unicode cap if chosen. Catch consumer callback failures consistently without letting one event sink prevent lifecycle completion; if changing TypeScript's uncaught change-listener behavior, record it as a design improvement.

### 9. Size and priority

Estimated Rust implementation: **260-420 lines** (estimate, not measured source). **P0** - All later shell controls require authoritative lifetime state and honest completion reporting.

## TOOLS-006 - Incremental bash_output reads and steering-interruptible waits

### 1. Title and status

Incremental bash_output reads and steering-interruptible waits. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/core/tools/bash-shells.ts:19-56; ice:packages/coding-agent/src/core/tools/bash-shells.ts:65-211; ice:packages/coding-agent/src/core/tools/bash-background.ts:371-387; ice:packages/coding-agent/src/core/tools/bash-background.ts:412-436; ice:packages/coding-agent/src/core/tools/bash-background.ts:672-687. Tests: ice:packages/coding-agent/test/bash-background.test.ts:131-201; ice:packages/coding-agent/test/bash-background.test.ts:203-221; ice:packages/coding-agent/test/bash-background.test.ts:255-264.

Named export: `BASH_SHELL_TOOL_NAMES` (`ice:packages/coding-agent/src/core/tools/bash-shells.ts:19`).

Named export: `MAX_WAIT_SECONDS` (`ice:packages/coding-agent/src/core/tools/bash-shells.ts:22`).

Named export: `createBashOutputToolDefinition` (`ice:packages/coding-agent/src/core/tools/bash-shells.ts:87`).

### 3. What it does

Read only output not previously returned, list all tracked shells, or wait briefly for a shell to exit or produce data while still accepting steering messages.

### 4. How it works

bash_output distinguishes listing, incremental output and full-log query modes. Its waits end on the requested event, duration, caller abort or a queued steer without killing the shell. Reading incremental output advances one shared per-shell model cursor; log queries leave that cursor alone.

When id is absent or empty, list snapshots immediately and ignore validation of the other fields, returning `No background shells.` or one status line per shell. With an id, wait_seconds must be finite and >=0; clamp to 300 s. tail_lines, if supplied, must be an integer >=1; regex validation is specified in TOOLS-007. Until defaults to exit. For until=output, waitForOutput returns immediately if shell is not running, duration is zero/nonpositive, or totalChars>readCursor; otherwise race a matching output subscription with shell.exited and the wait timer/abort. Then skip any further exit wait. Exit mode waits on the shell's exited promise. The wait helper resolves on abort, never throws. A separate AbortController follows the caller signal and a 250 ms queued-steer poll; call the steer check immediately for positive waits, catch stale contexts as no-steer, and ignore follow-up-only queues. Clean up listeners and polling in finally.

Registry.read computes unread=totalChars-readCursor, returns suffix of retained chunks of that length, reports omittedChars=max(0, unread-newOutput.length), and sets cursor=totalChars after waiting. Unknown id throws with a comma-separated list of known IDs when any exist. Output begins with `<id>: <status> (<runtime>)` plus a lingering warning and first command line. Runtime floors seconds, displays Ns below a minute, Nm Ns below an hour, Nh Nm thereafter. Include steering-interruption text only if still running, then optional full-log line and logIssue. Incremental output is tail-truncated to 2000 lines/51200 bytes, with omitted-character warning before it and `(no new output)` if empty. A terminal record with finalText appends a separately capped Final result block. There is no repeated-wait guard in this tool itself.

### 5. Interfaces

```ts
{ id?: string; wait_seconds?: number; until?: "exit"|"output"; tail_lines?: number; grep?: string }
```
Tool name/label bash_output. Details always `{shells: BackgroundShellStatus[], shellIds:string[]}`; one element per returned shell in listing order. Errors: `wait_seconds must be a non-negative number`, `tail_lines must be a positive integer`, `Unknown background shell "<id>".` with optional ` Known: <ids>.`. RenderCall reuses a Text component and displays ID or (all) with wait/tail/grep annotations. No persistent data format beyond the registry/log entries and no independent RPC or CLI interface.

### 6. Dependencies

`TOOLS-005`

Requires session's hasPendingSteering behavior for message wakeup and the shared truncation contract. Waiting never executes another command or invokes the model. Session integration is reviewed in SESSION-023 (tool registry/context), SESSION-024 (user Bash), and SESSION-025 (background delivery, wake and disposal); these are consumer links, not prerequisites for implementing the tool registry.

Final dependency classification. Consuming integrations: `SESSION-023`, `RPC-026`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

rust:crates/ice-tools/src/tools/mod.rs:1-27 has no bash_output factory and rust:crates/ice-tools/src/tools/bash.rs:136-302 exposes only the final result/stream callback of the active tool. rust:crates/ice-tools/src/shell_output.rs:42-62 has progress snapshots but no task ID, read cursor, output/exit wait subscriptions or retained terminal-shell listing. General cancellation tokens are present and can support the implementation but are not this operation.

### 8. Port notes

Add an async registry query tool using event subscriptions/Notify or watch channels, with cancellation that stops only the wait. Avoid losing output between the unread check and subscription registration; preserve cursor advancement and list-before-validation semantics for compatibility. Return the exact result metadata independently of the text renderer. Do not implement polling by repeatedly spawning shell commands.

### 9. Size and priority

Estimated Rust implementation: **180-280 lines** (estimate, not measured source). **P0** - Agents need bounded, interruptible access to a job's real progress and final outcome.

## TOOLS-007 - Bounded background-log tail and grep without consuming incremental output

### 1. Title and status

Bounded background-log tail and grep without consuming incremental output. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/core/tools/bash-shells.ts:21-28; ice:packages/coding-agent/src/core/tools/bash-shells.ts:119-137; ice:packages/coding-agent/src/core/tools/bash-shells.ts:154-192; ice:packages/coding-agent/src/core/tools/bash-background.ts:438-517; ice:packages/coding-agent/src/core/tools/bash-background.ts:644-665. Tests: ice:packages/coding-agent/test/bash-background.test.ts:275-331; ice:packages/coding-agent/test/bash-background.test.ts:437-467.

### 3. What it does

Inspect a noisy background command's complete stored log using a bounded tail or regular-expression search without moving its ordinary new-output cursor.

### 4. How it works

The query path streams a bounded suffix of a stored log for grep and reads a bounded byte tail for plain tail requests. It retains only the requested newest matching lines and falls back to the in-memory tail when a log cannot be accessed. A failed or capped log remains visibly incomplete through the separate logIssue metadata.

Tool grep strings are at most 500 UTF-16 code units. Reject a quantified parenthesized group followed by another quantifier using the exact NESTED_QUANTIFIER regex below; then construct JavaScript RegExp without flags and report its compilation error when invalid. This is a heuristic, not a proof that every regular expression has bounded complexity. grep alone defaults to 100 returned lines. `readLog` floors tailLines and uses max(1, floor(...)); the public tool has already enforced a positive integer. For each line, test only its first 8192 UTF-16 code units, increment matched if it qualifies, store at most that prefix with ` [line cut]` suffix if longer, and drop oldest results above keep.

If logPath exists, grep stats its current size. Scan only its newest 32 MiB; start one byte before the cut and discard the first partial line when skippedBytes>0. Pin end to size-1 when size>0 so subsequent file growth normally does not extend the scan; zero-size files omit end. Use UTF-8 read stream and readline with crlfDelay Infinity. Midscan read errors retain matches already found and destroy the stream. If initial stat fails, use retained chunks. A plain tail opens the log and reads at most its final 1 MiB; on failure use retained output. If a byte cut occurred, discard through the first LF, including any partial UTF-8 prefix; if no LF exists, indexOf returns -1 and the expression keeps the buffer text. Remove one trailing empty split element. tail-only considers newest keep lines; grep considers all lines in bounded source. Neither path changes readCursor.

Return `{shell,lines,matched? (grep only),skippedBytes? (>0 only)}`. The tool displays how many of the matches were returned, and when bytes were skipped says that rounded MiB of older output was not searched; truncate displayed lines again to the global output cap. If logPath exists but fallback was used, source label still says log because it is based on metadata, not a separate source indicator.

### 5. Interfaces

```ts
const NESTED_QUANTIFIER = /\((?:[^()\\]|\\.)*[+*}](?:[^()\\]|\\.)*\)\s*(?:[+*]|\{\d*,?\d*\})/;
```
BashOutputInput uses `tail_lines?: number` and `grep?: string`. Errors include `grep is <N> characters; at most 500.`, the nested-quantifier simplification error, and `grep is not a valid regular expression: <message>`. Internal readLog(taskId,{tailLines:number, grep?:RegExp}) does not accept cancellation or a separate time budget. Limits: 32*1024*1024 scan bytes, 8*1024 line code units, 1024*1024 tail bytes. No new setting, CLI flag, slash command or keybinding.

### 6. Dependencies

`TOOLS-005`, `TOOLS-006`, `TOOLS-010`

Local filesystem/readline or an equivalent bounded asynchronous log reader. Rust regex semantics differ from JavaScript and require an explicit compatibility decision, not silent substitution.

### 7. ice-rust today

rust:crates/ice-tools/src/shell_output.rs:319-343 and 404-420 only create a post-completion temp file; no method tails or greps an existing task log. Rust has the ordinary filesystem grep tool, but that searches paths as a separate tool and lacks task resolution, preserved read cursor, current logIssue and the limits/output contract here. No shell-log registry/control factory exists at rust:crates/ice-tools/src/tools/mod.rs:1-27.

### 8. Port notes

Implement task-log queries inside ice-tools with bounded buffered reads, fixed scan endpoints and result ring buffers. Rust's regex crate gives safer complexity but does not support all JavaScript expressions; explicitly document/translate the supported schema. Preserve skip counts and fallback behavior. Consider adding true cancellation to the reader as a documented improvement, while ensuring caller abort cannot terminate the underlying shell.

### 9. Size and priority

Estimated Rust implementation: **170-280 lines** (estimate, not measured source). **P1** - Long-running noisy jobs need targeted evidence without flooding the model or rereading all output.

## TOOLS-008 - Stopping running shells, lingering process groups and session disposal

### 1. Title and status

Stopping running shells, lingering process groups and session disposal. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/core/tools/bash-shells.ts:214-237 createBashKillTool; ice:packages/coding-agent/src/core/tools/bash-background.ts:275-298; ice:packages/coding-agent/src/core/tools/bash-background.ts:523-588; ice:packages/coding-agent/src/core/tools/bash-background.ts:608-641 BashBackgroundRegistry.kill/dispose/pruneFinished; ice:packages/coding-agent/src/utils/shell.ts:201-268 isProcessGroupAlive/terminateProcessGroup/killProcessTree; ice:packages/coding-agent/test/bash-background.test.ts:1-617

Named export: `BashKillInput` (`ice:packages/coding-agent/src/core/tools/bash-shells.ts:63`).

Named export: `createBashKillToolDefinition` (`ice:packages/coding-agent/src/core/tools/bash-shells.ts:214`).

`ice:packages/coding-agent/src/core/tools/bash-shells.ts:58-63` (`bashKillSchema`, `BashKillInput`).

### 3. What it does

The model can stop one known background shell without cancelling unrelated tools, including descendant processes left behind after its leader exited. Session disposal stops owned processes, resolves waiters and removes owned log files.

### 4. How it works

The registry distinguishes a running shell, an ordinary finished shell and a finished shell with a still-live process group. Kill changes state immediately so polling, watches and completion delivery observe a single terminal transition even if the underlying execution promise settles later.

createBashKillTool first resolves the string ID or throws an unknown-shell error. For a running shell, kill sets status=killed, endedAt=Date.now(), finalText to the user-stop or model-stop message, invokes its retained abort callback, resolves exited waiters, prunes, then calls change listeners. The execution-settlement handler ignores shells no longer running; therefore it does not overwrite killed state or emit a second completion. Tool-originated kill uses notify=false and returns 'Stopped <id>.'. User-stop paths can request notify=true: the completion has stoppedByUser:true and isError:true, but this synthetic event does not include the normal logPath/logIssue/exitCode/lingering fields. Completion listeners are individually caught; registry change listeners are not.

A finished shell is killable only if its retained pgid exists and isProcessGroupAlive succeeds. Unix kill(-pgid,0) treats EPERM as alive; Windows reports false. This branch invokes terminateProcessGroup without rewriting the shell's terminal status or notifying change listeners, and the tool returns 'Stopped the processes <id> left running.'. Other finished shells return '<id> already <status>.' without error. Unix termination sends SIGTERM to the group, polls every 100 ms and sends SIGKILL after a 3000 ms grace only if still alive; stopping the poll immediately when the group disappears reduces PID/group-reuse risk. Normal abort uses killProcessTree: Unix SIGKILL group with single-PID fallback; Windows detached hidden taskkill /F /T /PID. Killing is best effort; this is not a retry or guaranteed acknowledgment protocol.

dispose marks the registry disposed before aborting running jobs and terminating lingering finished groups. It resolves all exited waiters, removes every per-shell log plus tracked foreground logs, clears shell/foreground maps and listeners. Log removal failures are ignored. pruneFinished excludes running and lingering jobs, keeps at most 20 ordinary finished shells in adoption order and removes evicted logs. A killed shell may still have a live pgid; linger detection remains dynamic.

### 5. Interfaces

Model tool name 'bash_kill'; parameters {id:string}; returned text as above, details:undefined. Registry kill(id,{notify?:boolean}), dispose(), setProcessGroup(id, pgid); dynamic lingering status is exposed through get(id)/snapshots(), using private isLingering(shell). Completion custom type 'bash_background_completed' is consumed by session integration; event differences for user stops are part of the contract. System dependencies: Unix process groups or Windows taskkill.

`BashKillInput = Static<typeof bashKillSchema>` projects `Type.Object({ id: Type.String({ description: "Background shell id to stop (for example bash-1)" }) })`; there is no additional runtime algorithm in the alias.

### 6. Dependencies

`AGENT-052`, `AGENT-054`, `TOOLS-003`, `TOOLS-005`

Session-owned stop notifications and disposal wiring require a genuine session entry to be linked by the coordinator. AGENT-052/053/054 cover low-level execution cancellation and cleanup, not the model-visible registry. Session integration is reviewed in SESSION-023 (tool registry/context), SESSION-024 (user Bash), and SESSION-025 (background delivery, wake and disposal); these are consumer links, not prerequisites for implementing the tool registry.

Final dependency classification. Consuming integrations: `SESSION-023`, `SESSION-025`, `RPC-026`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

rust:crates/ice-tools/src/tools/mod.rs:1-27 exports no bash_kill tool; rust:crates/ice-tools/src/env.rs:96-111 exposes cancellation on a single foreground ShellExecOptions but no retained job handle or group API. rust:crates/ice-tools/src/os_env.rs:903-1075 tracks an executing PID and removes it when execution completes; the inspected foreground cancellation path does not retain user-addressable terminal shells or allow a later kill of leftover children. Behavior-based searches across crates found no registry-equivalent stop/list/linger path. rust:crates/ice-tools/src/os_env.rs:702-750 creates detached temporary directories/files but cleanup only kills and clears active PIDs; it does not retain or remove those temporary paths. Foreground capture therefore does not supply the registry-owned log removal lifecycle either.

### 8. Port notes

Implement bash_kill in ice-tools against the same session-owned background registry as TOOLS-005. Add platform-neutral process-group ownership/probe/termination APIs to the execution environment and preserve best-effort Unix/Windows differences. Keep waiter resolution, state transition and event ordering explicit; do not hold registry locks while invoking arbitrary listeners or awaiting process termination. Tests should cover repeated kill, killed-then-late-settlement, finished-with-descendants, normal finished no-op, disposal log cleanup and notification payload omissions.

### 9. Size and priority

Estimated Rust implementation: **140-240 lines** (estimate, not measured source). **P0** - Background execution needs reliable user/model stop and disposal behavior before it can be enabled.

## TOOLS-009 - Approved, byte-bounded stdin for a running background shell

### 1. Title and status

Approved, byte-bounded stdin for a running background shell. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/core/tools/bash-shells.ts:239-300 createBashInputTool; ice:packages/coding-agent/src/core/tools/bash-background.ts:15-18; ice:packages/coding-agent/src/core/tools/bash-background.ts:285-351 attachStdin/writeInput; ice:packages/coding-agent/src/core/tools/bash.ts:123-199; ice:packages/coding-agent/src/core/tools/bash.ts:375-701 stdin launch wiring; ice:packages/coding-agent/test/bash-input.test.ts:1-108

Named export: `BashInputInput` (`ice:packages/coding-agent/src/core/tools/bash-shells.ts:249`).

Named export: `BashInputApproval` (`ice:packages/coding-agent/src/core/tools/bash-shells.ts:252`).

Named export: `createBashInputToolDefinition` (`ice:packages/coding-agent/src/core/tools/bash-shells.ts:254`).

### 3. What it does

The model can send exact text and optionally EOF to a background command intentionally started with stdin support, after the configured approval check. Input limits and an explicit backpressure message prevent accidental duplicate submissions.

### 4. How it works

bash_input requires a known running shell with an open retained writable stream; it cannot attach stdin to an arbitrary process after launch. The tool obtains any required interactive approval before the registry writes bytes, and reports whether the pipe drained within five seconds.

Parameters must contain truthy text or eof=true; empty text alone is invalid. The tool resolves the shell before asking for approval. Unless approval() returns 'allow', an interactive UI is required; otherwise it throws "bash_input needs the user's approval and no interactive UI is available. Set bashInputApproval to allow, or re-run the command non-interactively." The dialog title is 'Send input to <id>?', includes the command's first line, JSON-quoted text clipped at 400 UTF-16 code units with an ellipsis, and EOF status. A refusal throws 'The user declined to send this input.'. The execution AbortSignal is not used by this tool, so approval/writing is not cancelled by that argument.

attachStdin closes a supplied stream immediately for unknown or nonrunning shells. It replaces the retained stream and close/error callbacks detach only the matching stream reference. writeInput rejects unknown/nonrunning shells or a stream that is absent, destroyed or writableEnded, with guidance to start bash using run_in_background:true and stdin:true. UTF-8 Buffer.byteLength(text) must be <=8192 per call and cumulative inputBytes+bytes <=65536 per shell. The counter advances before attempting the write. Text is sent exactly as supplied; no newline is added. If stream.write returns false, wait for drain, close or error with a 5000 ms bound; clean up all three listeners afterward. No drain yields pending:true rather than an exception, even if close/error ended the wait. eof=true then ends the stream. There is no automatic retry or input queue, and callers must not resend pending bytes.

Success returns details:{bytes}. Normal text says how many bytes were sent and whether stdin was closed; the pending variant says the pipe remained full for five seconds, warns not to resend and directs the model to inspect output. The global/project approval precedence is exercised by bash-input.test.ts, but the setting parser and session callback belong to the settings/session audit.

### 5. Interfaces

Tool 'bash_input'; schema {id:string, text?:string, eof?:boolean}. Creation takes approval(): 'ask'|'allow' and the tool context supplies hasUI/ui.confirm. Registry writeInput(id,{text?, eof?})->Promise<{bytes:number, pending?:boolean}>. bash launch requires run_in_background:true, stdin:true and local operations; custom BashOperations with stdin are rejected. MAX_INPUT_BYTES_PER_CALL=8192, MAX_INPUT_BYTES_PER_SHELL=65536. Setting key bashInputApproval is externally supplied to the tool.

### 6. Dependencies

`SET-003`, `SET-007`, `SET-008`, `SET-009`, `SET-014`, `TOOLS-003`, `TOOLS-005`

Coordinator must link settings approval precedence and session UI confirmation/context entries. Platform shell transport must expose a writable stream without confusing command-via-stdin transport. Session integration is reviewed in SESSION-023 (tool registry/context), SESSION-024 (user Bash), and SESSION-025 (background delivery, wake and disposal); these are consumer links, not prerequisites for implementing the tool registry.

Final dependency classification. Consuming integrations: `SESSION-023`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

rust:crates/ice-tools/src/tools/bash.rs:16-32; rust:crates/ice-tools/src/tools/bash.rs:81-112 accepts only command/timeout. rust:crates/ice-tools/src/env.rs:96-111 offers no stdin writer; rust:crates/ice-tools/src/os_env.rs:886-917 pipes stdin only to deliver shell transport commands and drops it before execution waiting. There is no bash_input factory in rust:crates/ice-tools/src/tools/mod.rs:1-27 and no per-shell input budget/approval/backpressure result in the searched Rust implementation.

### 8. Port notes

Add an explicitly requested stdin capability to the background process handle, then implement bash_input in ice-tools. Serialize or otherwise define concurrent writes so EOF and byte accounting remain deterministic; preserve current TS lifetime charging and pending semantics rather than silently retrying. Route approval through a host UI capability and settings callback instead of reading real user configuration within the tool. Rust async-write APIs should distinguish accepted bytes from drained completion and return the same no-resend instruction.

### 9. Size and priority

Estimated Rust implementation: **180-290 lines** (estimate, not measured source). **P0** - Commands awaiting input otherwise hang or require unsafe unapproved interaction.

## TOOLS-010 - Raw output accumulation, bounded live log spill and trustworthy full-output paths

### 1. Title and status

Raw output accumulation, bounded live log spill and trustworthy full-output paths. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/tools/output-accumulator.ts:1-291 OutputAccumulator,describeLogFileIssue; ice:packages/coding-agent/src/core/tools/bash.ts:410-502; ice:packages/coding-agent/src/core/tools/bash.ts:526-701; ice:packages/coding-agent/src/core/tools/truncate.ts:1-276; ice:packages/coding-agent/test/output-accumulator-log-safety.test.ts:1-70; ice:packages/coding-agent/test/tools.test.ts:474-771

Named export: `OutputAccumulatorOptions` (`ice:packages/coding-agent/src/core/tools/output-accumulator.ts:7`).

Named export: `MAX_LOG_FILE_BYTES` (`ice:packages/coding-agent/src/core/tools/output-accumulator.ts:16`).

Named export: `OutputSnapshot` (`ice:packages/coding-agent/src/core/tools/output-accumulator.ts:29`).

### 3. What it does

Model Bash retains a bounded recent output view while preserving earlier raw bytes in a live temporary log once truncation or backgrounding requires it. A capped or failed log is explicitly labelled so the model does not mistake its path for a complete record.

### 4. How it works

OutputAccumulator consumes Buffer chunks and decodes display text with one streaming TextDecoder, preserving split UTF-8 code points. It starts a log before old bytes are discarded and keeps the output-tail and log failure state separate, so output can continue after file logging is disabled.

Defaults: maxLines=2000, maxBytes=51200, maxRollingBytes=max(maxBytes*2,1), maxFileBytes=64*1024*1024. Track total raw bytes, total decoded UTF-8 bytes, newline/open-line counts, current-line bytes, retained text and raw pre-spill buffers. A final LF does not create an extra logical line. Spill when rawBytes>maxBytes OR decodedBytes>maxBytes OR totalLines>maxLines; persist() forces spill immediately for background adoption. The name is <tempFilePrefix>-<randomBytes(8).hex>.log in tmpdir, default prefix ice-output and Bash prefix ice-bash. ensureTempFile first writes all buffered raw chunks, then later chunks are written directly. Therefore file bytes retain ANSI, CR and binary bytes; the model tool does not apply the separate user-Bash sanitization path.

The rolling decoded string is trimmed only after it exceeds 2*maxRollingBytes, to the latest maxRollingBytes at a UTF-8 boundary. Remember whether the retained prefix begins at a line boundary; snapshots discard a cut first line when another LF exists. truncateTail enforces final display limits, reports global total lines/bytes and differentiates a partial last line. finish is idempotent, stops future append, flushes the decoder and starts a file if that flush crosses a limit. snapshot({persistIfTruncated:true}) can create a log; a forcibly persisted path can be present even when the visible snapshot is not truncated.

File payload is capped with a 256-byte marker reserve: write at most max(0, maxFileBytes-256) raw data bytes, append a newline marker explaining the limit, record the first LogFileIssue and end logging. The boundary can split raw UTF-8, by design of the raw-byte cap. Later output remains only in the retained tail. Stream writes do not await drain; total queued file data is bounded by the log cap. An asynchronous write failure records {kind:'write_failed', error}, disables/destroys the stream and preserves the first issue. Descriptions distinguish 'log stopped at <size>; later output is only in the retained tail' from a write failure. closeTempFile waits for the stream to finish; an error arriving during that close promise can still reject, so this is not a blanket guarantee that every late file error becomes a successful tool result.

Bash prevents late onData calls from entering the accumulator after settlement, finishes it, flushes the final model update, snapshots, then awaits file close. Background shells retain the live log path and issue; foreground logs are registered for session cleanup. Tests read (not executed) specify cap markers, preserved first log bytes versus latest tail, byte-identical small logs, and continued snapshots after an already-observed write failure.

### 5. Interfaces

OutputAccumulator({maxLines?, maxBytes?, tempFilePrefix?, maxFileBytes?}); append(Buffer), finish(), persist(), snapshot({persistIfTruncated?}), closeTempFile(); snapshot has output, truncation, fullOutputPath. LogFileIssue = {kind:'truncated', maxBytes:number}|{kind:'write_failed', error:string}; log issue descriptions are surfaced in Bash text and background snapshots/completions. fullOutputPath is a temporary path, not a promise of unlimited history after a reported cap/failure.

### 6. Dependencies

`AGENT-050`, `AGENT-051`, `AGENT-053`, `TOOLS-019`

Shared truncation and execution-environment contracts apply. Background ownership/cleanup is TOOLS-005/008. Existing AGENT-050/051/053 own live pipe draining, incremental transport decoding and post-exit idle grace; these are prerequisites for end-to-end correctness, separate from accumulator parity.

### 7. ice-rust today

rust:crates/ice-tools/src/shell_output.rs:69-171 keeps at most MAX_OUTPUT_BYTES=102400 bytes and tracks global totals, but rust:crates/ice-tools/src/shell_output.rs:319-343; rust:crates/ice-tools/src/shell_output.rs:404-420 waits until env.exec returns and creates the advertised full-output file from state.tail_output. Exact reachable loss condition: callbacks deliver more than 102400 retained UTF-8 bytes, truncation is true, no earlier full_output_path exists and temp creation/append succeeds; the file then contains only the bounded suffix and omits all preceding output. This is established by source control/data flow, not a runtime reproduction. InMemory/custom execution environments can deliver such output independently of the OS pipe-draining issue. The Rust Bash spill test uses 2001 short lines below 100 KiB; its prefix/suffix checks do not cover this condition (rust:crates/ice-tools/tests/bash.rs:99-154). Rust sanitizes each String callback before accumulation (rust:crates/ice-tools/src/shell_output.rs:175-236; rust:crates/ice-tools/src/shell_output.rs:257-304), unlike TS raw model output. A failed spill logs a warning and leaves no path; there is no structured capped/failed live-log state. try_lock failure in capture callbacks returns before ingest, so concurrent custom-environment callback/reader contention can discard a chunk; this is a source-only risk, not observed reproduction. Underlying OsExecutionEnv also collects unbounded stdout/stderr strings (rust:crates/ice-tools/src/os_env.rs:929-982; rust:crates/ice-tools/src/os_env.rs:1069-1075). Local Git blame for lines 319–343 attributes this tail-only initialization to commit 909e85120146cfcf46b7835d69a8805bce3edcff, original crates/pi-tools/src/shell_output.rs lines 262–286 (local history only, no remote verification). The full source hash and reachable-condition evidence are persisted in evidence/tools-defect-evidence.json. rust:crates/ice-tools/src/os_env.rs:702-750 creates an isolated temporary directory for each temporary file and does not remove it from cleanup; session-owned log retention/removal must be added explicitly rather than inferred from temporary-file naming.

### 8. Port notes

Replace post-exit tail-only spill with an accumulator owned for the whole execution in ice-tools, writing raw bytes before pruning. The host execution API needs byte-safe streams or incremental decoding before String conversion; use tokio file I/O and bounded accounting, retain temp handles and structured LogFileIssue. Avoid lossy try_lock on authoritative ingestion; keep snapshots cheap while ingest remains reliable. Add meaningful tests with output substantially above 100 KiB that verify the first, middle and last log bytes, split UTF-8 and ANSI, forced early persistence, line-only overflow, capped log plus continuing tail, and file failure during close. Preserve Rust's platform-independent filesystem abstraction while making file creation visible during execution.

### 9. Size and priority

Estimated Rust implementation: **260-420 lines** (estimate, not measured source). **P0** - The current Rust full-output path can silently omit the output the model needs to diagnose a long command.

## TOOLS-011 - Session-local bash_watch management and exact watch schemas

### 1. Title and status

Session-local bash_watch management and exact watch schemas. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/core/tools/bash-watch.ts:1-364; ice:packages/coding-agent/src/core/tools/bash-watch.ts:428-492 BashWatchRegistry/createBashWatchTool; ice:packages/coding-agent/test/bash-watch.test.ts:1-196

Named export: `BashWatchKind` (`ice:packages/coding-agent/src/core/tools/bash-watch.ts:13`).

Named export: `BASH_WATCH_FIRED_CUSTOM_TYPE` (`ice:packages/coding-agent/src/core/tools/bash-watch.ts:16`).

Named export: `BashWatchSpec` (`ice:packages/coding-agent/src/core/tools/bash-watch.ts:18`).

Named export: `BashWatchSnapshot` (`ice:packages/coding-agent/src/core/tools/bash-watch.ts:34`).

Named export: `BashWatchFire` (`ice:packages/coding-agent/src/core/tools/bash-watch.ts:49`).

Named export: `MAX_WATCH_FIRES` (`ice:packages/coding-agent/src/core/tools/bash-watch.ts:68`).

Named export: `BashWatchInput` (`ice:packages/coding-agent/src/core/tools/bash-watch.ts:128`).

Named export: `createBashWatchToolDefinition` (`ice:packages/coding-agent/src/core/tools/bash-watch.ts:142`).

### 3. What it does

The model can create, inspect, cancel and rearm bounded watches on running background shells. Watches are removed when their shell finishes or their lifetime fire budget is consumed.

### 4. How it works

One BashWatchRegistry subscribes to shell start, output and state changes, and owns watch IDs, timers and fire listeners for the session. Explicit watches and automatic defaults share lifetime semantics, while only explicit watches count toward the creation limit.

The tool action is required: create|list|cancel|rearm. list optionally filters by shell and returns descriptions or 'No watches.'. cancel/rearm require watch_id; unknown IDs throw, and rearm explains that an exhausted watch may already have been removed. create requires shell and kind, then checks the target exists and is running before enforcing MAX_BASH_WATCHES=16 explicit watches. max_fires defaults to 1 for pattern and 3 for other explicit kinds; it must be an integer 1..10. Pattern requires a nonempty literal <=200 UTF-16 code units. silence_seconds*1000 must be finite and >=5000; interval_seconds*1000 finite and >=30000. Only fields relevant to the chosen kind are carried into the internal spec. Default rearm is on_activity for silence, manual otherwise.

IDs increase monotonically as watch-N. On insertion: fires=0, sequence=0, armed=true, carry='', then store, schedule and notify. list preserves insertion order. cancel clears timers and removes the watch. rearm sets armed=true and clears carry, then schedules; it does not reset lifetime fires or fire sequence. Rearming a silence watch uses the shell's existing lastOutputAt, so it can fire immediately if already overdue. Each fire increments fires and emits fireId='<watchId>#<sequence>' with a strictly increasing per-watch sequence. At maxFires, removal happens before listener delivery. Fire and watch-change listener errors are caught individually. Shell change removes watches whose target is no longer running. dispose marks disposed, clears all timers/watches/listeners and unsubscribes from shell events.

Snapshot fields are id, target, kind, pattern?, silenceMs?, intervalMs?, maxFires, fires, armed, rearm, auto. User-facing descriptions include kind, fires/max, armed/waiting-to-rearm, rearm mode and whether default. Details carry {watches:[...]}; cancel returns an empty watch list, rearm the one snapshot. The dirty bash-watch.ts is the historical accepted source snapshot, not reconstructed from clean Git.

### 5. Interfaces

Tool 'bash_watch'; {action:'create'|'list'|'cancel'|'rearm', shell?:string, watch_id?:string, kind?:'pattern'|'silence'|'interval', pattern?:string, silence_seconds?:number, interval_seconds?:number, max_fires?:number, rearm?:'manual'|'on_activity'}. Internal kind also includes 'prompt' for automatic defaults, but users cannot create prompt explicitly. Event custom type 'bash_watch_fired'; fire {watchId, target, kind, reason, tail, logPath?, auto, fireId}. Registry onFire/onChange return unsubscribe functions.

### 6. Dependencies

`TOOLS-005`

Session event persistence, delivery/deduplication and model wake policy must link the session-owned entries. Pattern behavior TOOLS-012 and timed/default behavior TOOLS-013 complete this registry. Session integration is reviewed in SESSION-023 (tool registry/context), SESSION-024 (user Bash), and SESSION-025 (background delivery, wake and disposal); these are consumer links, not prerequisites for implementing the tool registry.

Final dependency classification. Consuming integrations: `SESSION-023`, `SESSION-025`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

rust:crates/ice-tools/src/tools/mod.rs:1-27 has no bash_watch export; rust:crates/ice-tools/src/env.rs:96-111 and rust:crates/ice-tools/src/tools/bash.rs:136-302 expose foreground execution rather than a live target registry. Behavior searches found no equivalent watch identity, fire budget, rearm state or bash_watch_fired event in Rust. No claim of parity is inferred from generic timers or existing user Bash execution.

### 8. Port notes

Implement a session-scoped registry in ice-tools or a coding-session tool service using cancellable Tokio timer handles, stable IDs and explicit listener/event delivery. Avoid holding a registry mutex while invoking callbacks. Model the removal-before-fire ordering and lifetime budgets; preserve automatic prompt as an internal kind. Share the same registry instance among bash, bash_output, bash_kill, bash_input and bash_watch factories.

### 9. Size and priority

Estimated Rust implementation: **180-290 lines** (estimate, not measured source). **P0** - Watch creation and lifetime ownership are required for background completion monitoring to be reliable.

## TOOLS-012 - Literal pattern watches with cross-chunk matching and rearm semantics

### 1. Title and status

Literal pattern watches with cross-chunk matching and rearm semantics. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/core/tools/bash-watch.ts:238-274; ice:packages/coding-agent/src/core/tools/bash-watch.ts:389-448; ice:packages/coding-agent/test/bash-watch.test.ts:1-196

### 3. What it does

A model can watch for a literal readiness/error phrase even when its bytes arrive across separate output callbacks. One matching chunk produces at most one alert and rearming preserves the lifetime fire limit.

### 4. How it works

Pattern watches perform a case-sensitive literal substring search over a short carry plus each new decoded output chunk. They do not replay old shell history or use regular expressions, and a disarmed on_activity watch deliberately consumes the next chunk only to rearm.

A pattern must be nonempty and <=200 UTF-16 code units. While armed, search haystack=carry+text using indexOf(pattern). If no match, retain only the last pattern.length-1 UTF-16 code units (empty for length one); this is sufficient for the next chunk to complete a boundary match. If a match occurs, clear carry, form the reason from the matching line found by lastIndexOf(pattern) in the shell's retained output, trim and clip the line to 300 code units, fire once and set armed=false. Several occurrences in one chunk still cause one alert. A pattern truncated out of the retained shell tail may therefore have a reason excerpt different from the exact first matching occurrence; detection itself is on carry+chunk.

While disarmed, rearm='manual' ignores output. With on_activity, the first later chunk sets armed=true and clears carry, then skips matching that chunk. A following chunk may fire again. Explicit rearm also clears carry and does not reset fires; reaching maxFires removes the watch. The event carries the current shell tail: trimEnd(), last 20 lines, then last 2000 UTF-16 code units, plus logPath when known. This tail limit is independent of the 300-character reason excerpt and the registry's larger output retention. Test bodies cover split-chunk matches, the skipped rearming chunk and lifetime counts; none were executed in this audit.

### 5. Interfaces

bash_watch create with kind:'pattern', pattern:string, max_fires?:1..10, rearm?:'manual'|'on_activity'; default max_fires=1, rearm='manual'. Matching is literal/case-sensitive. Fire kind='pattern', unique fireId and bounded tail; no regex flags, arbitrary code execution or rescanning old output.

### 6. Dependencies

`TOOLS-005`, `TOOLS-011`

SESSION-025 propagates fireId and chooses alert delivery timing; matching generates distinct identities but no external-delivery deduplication is established and chooses alert delivery timing; those policies are outside tool matching. Session integration is reviewed in SESSION-023 (tool registry/context), SESSION-024 (user Bash), and SESSION-025 (background delivery, wake and disposal); these are consumer links, not prerequisites for implementing the tool registry.

Final dependency classification. Consuming integrations: `SESSION-025`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

No corresponding literal background-output matcher or rearming watch is exported in rust:crates/ice-tools/src/tools/mod.rs:1-27 or connected to rust:crates/ice-tools/src/shell_output.rs:257-304. Rust grep is a file search tool, not a subscription over future shell chunks. Generic output callbacks alone do not provide the state, fire budget or event contract.

### 8. Port notes

Use a small retained suffix and case-sensitive substring matching in the shared watch registry. JS indexes by UTF-16 code units, whereas Rust String uses UTF-8; define and test the exact compatibility boundary for length limits and clipping, without slicing invalid UTF-8. Preserve the unusual activity-rearm skip instead of matching immediately. Test multi-byte split transport separately from decoded-chunk pattern splitting.

### 9. Size and priority

Estimated Rust implementation: **70-120 lines** (estimate, not measured source). **P1** - Readiness and error alerts must not miss phrases split across output callbacks or fire repeatedly without rearming.

## TOOLS-013 - Silence, prompt and interval watches with automatic defaults

### 1. Title and status

Silence, prompt and interval watches with automatic defaults. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/core/tools/bash-watch.ts:17-92; ice:packages/coding-agent/src/core/tools/bash-watch.ts:238-274; ice:packages/coding-agent/src/core/tools/bash-watch.ts:323-387; ice:packages/coding-agent/src/core/tools/bash-watch.ts:405-492; ice:packages/coding-agent/test/bash-watch.test.ts:1-196; ice:packages/coding-agent/test/suite/regressions/execution-reliability-1-watch-alerts.test.ts:1-70

Named export: `MIN_SILENCE_MS` (`ice:packages/coding-agent/src/core/tools/bash-watch.ts:66`).

Named export: `MIN_INTERVAL_MS` (`ice:packages/coding-agent/src/core/tools/bash-watch.ts:67`).

Named export: `BashWatchDefaults` (`ice:packages/coding-agent/src/core/tools/bash-watch.ts:77`).

### 3. What it does

Background jobs can alert periodically, when output stops, or when their last output looks like an interactive prompt. Automatic watches are configured when each shell starts and carry an auto flag so session policy can handle idle notifications differently.

### 4. How it works

Time-based watches schedule against each shell's lastOutputAt rather than simply its adoption time. Automatic prompt and silence watches are installed on shell start using a defaults callback; they remain distinct events even when both could fire.

For explicit silence the minimum is 5000 ms; interval minimum 30000 ms. Default callback values silenceAlertMs and promptAlertMs are accepted when >0 without the explicit-create minimum validation; zero disables that default. addDefaults installs prompt first, then silence. Both defaults use maxFires=3, rearm='on_activity', auto=true; prompt also requirePrompt=true. Automatic defaults do not consume the 16 explicit-watch capacity. This tool module does not define the user's persisted setting defaults; those must be cited from settings/session.

schedule clears old timer handles first and does nothing while disarmed. Interval uses setInterval(intervalMs), unref(), and fires every tick until cancelled/finished/exhausted; it is not disarmed by each fire and output does not reset its cadence. Silence/prompt uses a one-shot timeout max(0, silenceMs-(Date.now()-lastOutputAt)), unref(). On timeout require the shell still exists/runs and the watch is armed. Report quiet seconds with Math.round. A silence fire disarms the watch after delivery. New output rearms only when rearm='on_activity', then reschedules from the updated shell timestamp; a manual disarmed watch stays quiet.

Prompt mode examines the last nonempty retained output line. Its case-insensitive heuristic recognizes y/n or yes/no, [y/n] variants, 'press enter/return/any key', 'continue?'/'proceed?'/'overwrite?', password or passphrase prompts ending in colon, or any trailing question mark. Exact regex is in ice:packages/coding-agent/src/core/tools/bash-watch.ts:17-25. If no heuristic match, schedule another full prompt period while staying armed; do not fire or consume budget. A match emits a reason containing the trimmed last line clipped to 200 UTF-16 units and advice to kill/re-run non-interactively, then disarms. That advice is current source behavior even though bash_input exists. All event tails use 20 lines then 2000 code units; at lifetime maximum the watch is removed before listeners run. A session's busy/idle wake or deferred delivery decision is not made in this timer implementation.

### 5. Interfaces

Explicit bash_watch kinds:'silence'|'interval', seconds fields converted to milliseconds; max_fires defaults 3 (1..10); silence default rearm:on_activity, interval default manual. Internal automatic kind:'prompt'. Defaults callback returns {silenceAlertMs:number, promptAlertMs:number}. Fire event includes auto:boolean, fireId, kind, reason, tail, optional logPath. System Node timers are unref'ed; Rust equivalent must not keep a disposed session alive.

### 6. Dependencies

`SET-014`, `TOOLS-005`, `TOOLS-011`

Coordinator must link the settings keys/defaults/validation and session notification/wake/idle entries. Regression execution-reliability-1-watch-alerts.test.ts is a current dirty-test snapshot only; its historical content hash was unavailable. Session integration is reviewed in SESSION-023 (tool registry/context), SESSION-024 (user Bash), and SESSION-025 (background delivery, wake and disposal); these are consumer links, not prerequisites for implementing the tool registry.

Final dependency classification. Consuming integrations: `SESSION-025`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

The inspected Rust tool factory list (rust:crates/ice-tools/src/tools/mod.rs:1-27), foreground Bash schema (rust:crates/ice-tools/src/tools/bash.rs:81-112) and ShellExecOptions (rust:crates/ice-tools/src/env.rs:96-111) provide none of these watch kinds, automatic defaults or event payloads. Behavior-based searches across crates found no equivalent prompt/silence/interval background shell service.

### 8. Port notes

Implement cancellable timer tasks against a monotonic time base while reproducing the observable elapsed/scheduling semantics. Store handles per watch, cancel on every terminal target transition and registry disposal, and avoid callbacks after removal. Preserve prompt polling without consuming its budget and separate automatic prompt from automatic silence. Tests should advance a controllable clock through activity, immediate overdue rearm, no-prompt polling, completion cancellation and max-fire removal.

### 9. Size and priority

Estimated Rust implementation: **180-290 lines** (estimate, not measured source). **P0** - Long-running or blocked commands need bounded monitoring and correct session wake behavior.

## TOOLS-014 - Bash streaming updates, elapsed-time rendering and final-result formatting

### 1. Title and status

Bash streaming updates, elapsed-time rendering and final-result formatting. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/tools/bash.ts:258-373; ice:packages/coding-agent/src/core/tools/bash.ts:410-551; ice:packages/coding-agent/src/core/tools/bash.ts:675-749; ice:packages/coding-agent/test/tools.test.ts:474-771; rust:crates/ice-tools/tests/bash_throttle.rs:1-241

### 3. What it does

Bash shows prompt initial feedback, throttled current output and an accurate final result, while the interactive view offers a compact five-line tail with expansion and elapsed time. Error and truncation messages preserve command output where the source specifies it.

### 4. How it works

The model-update path and TUI redraw path use separate throttles. Model updates snapshot the accumulator at most once per 100 ms with one pending timer, while result rendering invalidates once per second only while execution is partial.

Execution emits an initial update with content:[] and details:undefined before spawning. lastUpdateAt=0 allows the first data callback to emit immediately. Later chunks share one scheduled timeout; no additional timer is created until it fires. A snapshot can force log persistence when truncated. On settlement disable acceptingOutput, finish decoder/accounting, clear the pending timer, emit a final update, capture the snapshot and close the file. A late data callback after completion is ignored. Cancellation/timeout/nonzero errors include the retained output and the corresponding termination message; an arbitrary other backend error is propagated without being transformed into a captured-output error. Empty success uses '(no output)'. No automatic retries occur here.

The result formatter attaches fullOutputPath/truncation only as needed and distinguishes line truncation, byte truncation and an oversized last line. A log issue adds an explicit warning. The result renderer joins text content, trims it, and strips a trailing truncation footer that duplicates the full-path warning before rendering. Collapsed view shows the latest five visual lines, cached by terminal width, with the number of earlier lines and app.tools.expand key hint. Expanded view shows all retained result text. Call title is '$ <command>' with timeout text when supplied; malformed arguments are shown with invalidArgText. Timing starts once executionStarted is observed; partial rendering installs a 1000 ms invalidate interval, and terminal/error state records endedAt once and clears it. These are presentation behaviors, separate from the model tool's 2000-line/50-KiB output cap.

### 5. Interfaces

BashDetails camelCase {truncation?, fullOutputPath?, backgroundTaskId?, backgroundOrigin?:'model'|'user'}. Update callback receives AgentToolResult; initial content is an empty array. Renderer context includes expanded, isPartial, executionStarted, invalidate and app.tools.expand keybinding. BASH_UPDATE_THROTTLE_MS=100; BASH_PREVIEW_LINES=5.

### 6. Dependencies

`TOOLS-001`, `TOOLS-010`

Coordinator should link the shared TUI render-component and tool execution event contracts; tool-specific formatting remains in this entry.

Final dependency classification. Consuming integrations: `TUI-031`, `TUI-032`, `TUI-035`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

rust:crates/ice-tools/src/tools/bash.rs:35-36; rust:crates/ice-tools/src/tools/bash.rs:136-258 uses a 2000 ms model throttle and initializes last_flush=Instant::now(), so first output is delayed instead of immediate. Every early chunk spawns another sleeping Tokio task, unlike the single coalesced TS timer; pending tasks have no cancellation handle, though final flush clears dirty state. Initial update is a text block containing an empty string, not content:[]. Rust result details serialize snake_case full_output_path and its own truncation shape (rust:crates/ice-tools/src/tools/bash.rs:22-32; rust:crates/ice-tools/src/tools/bash.rs:307-402). The throttle test emits 500 synchronous chunks and only checks updates<25 and >=1; it neither establishes 100 ms latency nor a single pending task. Generic Rust UI rendering may provide some display features; exact Bash five-visual-line footer/timing/key-hint parity needs the TUI-owned cross-reference rather than being assumed absent.

### 8. Port notes

Align tool output update behavior in ice-tools using one cancellable coalescing task or channel and an immediate first emission. Ensure final delivery is awaited and no stale partial update follows it. Coordinate serializable details casing and tool-specific render metadata with the TUI adapter; preserve any intentional Rust schema change only with an explicit compatibility decision. A meaningful timing test should observe first-output latency, bounded timer/task ownership and one final snapshot, not just a broad update count.

### 9. Size and priority

Estimated Rust implementation: **140-230 lines** (estimate, not measured source). **P1** - Two-second initial output delay and divergent details degrade responsiveness and integration even when command execution succeeds.

## TOOLS-015 - Harness shell capture sanitation, early spill and failure ordering

### 1. Title and status

Harness shell capture sanitation, early spill and failure ordering. **Status: partial.**

### 2. ice source

ice:packages/agent/src/harness/utils/shell-output.ts:1-195 executeShellWithCapture; ice:packages/agent/src/harness/tools/bash.ts:1-161 createBashTool; ice:packages/agent/test/harness/tools.test.ts:1-623

Named export: `BashToolDetails` (`ice:packages/agent/src/harness/tools/bash.ts:18`).

Named export: `ShellCaptureProgress` (`ice:packages/agent/src/harness/utils/shell-output.ts:4`).

Named export: `ShellCaptureOptions` (`ice:packages/agent/src/harness/utils/shell-output.ts:11`).

Named export: `ShellCaptureResult` (`ice:packages/agent/src/harness/utils/shell-output.ts:17`).

### 3. What it does

The environment-based harness Bash tool streams sanitized output, preserves a full sanitized log before pruning, and returns explicit capture or execution failures. Its capture contract differs from the coding-agent raw-byte accumulator.

### 4. How it works

The harness captures a sanitized text stream through ExecutionEnv while maintaining a 100 KiB recent tail and global output statistics. File writes are serialized in a promise chain, and all accepted output is settled before the result is returned.

On each accepted chunk remove C0 controls except tab/LF/CR and U+FFF9–U+FFFB, then remove CR. This removes ESC but does not parse the remaining ANSI sequence. Count sanitized UTF-8 bytes, LF/open-line counts and last-line bytes. On crossing 51200 bytes or 2000 logical lines, call ensureFullOutputFile before trimming to the most recent 102400 UTF-8 bytes, so its initial append includes the prefix about to be discarded. env.createTempFile({prefix:'bash-', suffix:'.log'}) and subsequent env.appendFile calls share writeChain; each new chunk appends after the initial write. There is no 64 MiB file cap in this harness helper. Existing line/byte metadata feeds truncateTail and the fullOutputPath.

onChunk receives sanitized text plus a progress getter. A callback exception records captureError; file errors record writeError. acceptingOutput is turned off after execution settles so late callbacks are ignored; then await writeChain. Terminal precedence is writeError, then captureError, then executionError. Abort is represented as cancelled with no exit code. returnExecutionErrors permits an executionError field in an otherwise returned capture; without that option a non-abort execution failure returns an error result. No retry is present. The Bash wrapper prepends commandPrefix, awaits prepare(execution, context, signal), uses a 100 ms coalesced update timer and always clears it; it formats nonzero/timeout/abort failures from the captured output.

### 5. Interfaces

ExecutionToolContext {env:ExecutionEnv}; BashExecution {command, cwd, env, inheritEnv}; BashPrepare receives execution, context and optional AbortSignal. executeShellWithCapture(env, command,{cwd?, env?, inheritEnv?, timeout?, signal?, onChunk?, returnExecutionErrors?}) returns a Result containing output, exitCode, cancelled, truncated, truncation, fullOutputPath and optional executionError. Harness model schema remains {command:string, timeout?:number}; timeout zero is invalid here (coding-agent TOOLS-001 deliberately differs). No settings or RPC methods are declared by this helper.

### 6. Dependencies

`AGENT-047`, `AGENT-050`, `AGENT-051`, `AGENT-053`, `TOOLS-010`, `TOOLS-019`

Uses the environment filesystem/shell interfaces and the shared Bash low-level execution entries. TOOLS-010 owns raw model capture and the shared Rust prefix-loss defect; keep the two TS sanitation policies explicit.

### 7. ice-rust today

rust:crates/ice-tools/src/shell_output.rs:175-236 strips full ANSI CSI/OSC sequences as well as controls, while TS harness only deletes ESC/control characters. The Rust callback receives the original chunk even though capture ingests its sanitized copy (rust:crates/ice-tools/src/shell_output.rs:257-304), so callback and result differ. Rust creates spill only after exit from the pruned tail, and spill failure is merely a tracing warning (rust:crates/ice-tools/src/shell_output.rs:319-343), not TS's higher-priority writeError. rust:crates/ice-tools/src/tools/bash.rs:53-83 exposes an async mutable execution prepare hook but not the TS context/signal arguments. Existing Abort/timeout/nonzero/capture result support is present; it is not an entirely missing harness.

### 8. Port notes

Add an explicit capture policy to ice-tools so harness sanitation and coding-agent raw capture are not accidentally conflated. Serialize ordered spill writes before pruning and propagate the documented error precedence. Expand prepare-hook context with cancellation without breaking existing native hooks. Keep a compatibility adapter for wire field naming, and cover sanitization residues, callback failures, simultaneous execution/file failure and late output in future source-change tests.

### 9. Size and priority

Estimated Rust implementation: **160-260 lines** (estimate, not measured source). **P0** - Harness clients currently receive different output and weaker persistence-failure signals, including the same silent prefix loss.

## TOOLS-016 - User Bash execution helper and its distinct output contract

### 1. Title and status

User Bash execution helper and its distinct output contract. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/bash-executor.ts:1-156 executeBashWithOperations,executeBash; ice:packages/coding-agent/src/utils/child-process.ts:1-138; ice:packages/coding-agent/src/core/tools/bash.ts:233-374

Named export: `BashExecutorOptions` (`ice:packages/coding-agent/src/core/bash-executor.ts:22`).

### 3. What it does

User-entered shell commands produce live readable text, cancellation/exit metadata and a temporary full-output path without a model tool call. This helper has a different sanitation and buffering policy from model Bash.

### 4. How it works

executeBashWithOperations feeds a streaming TextDecoder into stripAnsi, binary-control sanitation and CR removal before invoking onChunk. It tracks raw input bytes for the spill threshold and sanitized text for the returned tail.

Once raw bytes exceed 51200, create a random ice-bash log in the system temporary directory, replay all earlier sanitized chunks, then write subsequent sanitized chunks. The tail's counter uses JavaScript string length despite a byte-like variable name: while over 102400 UTF-16 units and there is more than one chunk, drop whole old chunks. One oversized chunk can therefore exceed that retention target. After settlement, truncateTail applies the 2000-line/51200-byte display bound; if only a line overflow caused truncation, create the log then from the retained chunks. The helper does not flush the decoder at EOF. It ends the stream without awaiting finish and installs no structured logging-error policy or file cap.

Default execution delegates to local Bash operations. onData forwards sanitized deltas; onChunk is optional. Abort becomes cancelled:true, exitCode undefined; a null exit code becomes undefined. Other execution errors are thrown. The returned object includes output, exitCode, cancelled, truncated, fullOutputPath. This utility does not create a background registry, persist a session message, or decide whether a user command is excluded from model context; those are caller responsibilities.

### 5. Interfaces

executeBash(command:string, cwd:string,{onChunk?, signal?}) and executeBashWithOperations(command, cwd, operations, options). BashOperations is the same injectable execution interface used by TOOLS-001/002. Output has {output:string, exitCode?:number, cancelled:boolean, truncated:boolean, fullOutputPath?:string}. No timeout option is accepted by this helper. The user-facing !/!! dispatch and session persistence belong to session integration.

### 6. Dependencies

`TOOLS-002`, `TOOLS-015`, `TOOLS-019`

Session integration must select this user-command policy and own cancellation slots/messages. External dependency is the selected Bash executable or injected operations. Session integration is reviewed in SESSION-023 (tool registry/context), SESSION-024 (user Bash), and SESSION-025 (background delivery, wake and disposal); these are consumer links, not prerequisites for implementing the tool registry.

Final dependency classification. Consuming integrations: `SESSION-024`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

rust:crates/ice-cli/src/interactive_tui.rs:563-647 already runs ! commands outside the model, registers a cancellation slot before spawn, updates BashExecutionComponent and reports UserBashFinished with the exclude_from_context bit. It directly constructs OsExecutionEnv using cwd and execute_shell_with_capture, so the helper policy is the Rust harness capture (TOOLS-015), not the TS user helper. It does not pass shell-path/spawn-hook/session-environment options from a shared executor. Its on_chunk snapshots the complete tail, while TS helper emits deltas. Rust avoids the TS whole-chunk retention overshoot but inherits post-exit incomplete logs. Transport errors become visible 'Bash command failed: ...'; the source read shows the capture execution_error field is not separately surfaced by this tuple mapping when return_execution_errors:true.

### 8. Port notes

Keep Rust's early cancellation-slot registration and existing UserBashFinished message flow. Introduce a shared user Bash service receiving the same configured execution backend as the model tool while retaining its own sanitation policy. Make ownership of delta versus snapshot callbacks explicit. Preserve truthful full-output/cancellation metadata and improve TS's unflushed decoder and unobserved log-close weaknesses as documented Rust safety decisions rather than copying them blindly.

### 9. Size and priority

Estimated Rust implementation: **140-240 lines** (estimate, not measured source). **P1** - User shell commands work today, but configuration and output semantics diverge from the shared TypeScript user executor.

## TOOLS-017 - Direct argv execution with capture, cancellation and timeout result

### 1. Title and status

Direct argv execution with capture, cancellation and timeout result. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/exec.ts:1-107 execCommand,ExecOptions,ExecResult; ice:packages/coding-agent/src/utils/child-process.ts:1-138 waitForChildProcess; ice:packages/coding-agent/test/suite/regressions/5303-bash-output-truncation.test.ts:1-83

### 3. What it does

Extension callers can execute an exact executable-plus-arguments array, capture stdout/stderr separately, and receive a result even on spawn failure or timeout.

### 4. How it works

execCommand spawns command with args, shell:false, the explicit cwd argument, ignored stdin and piped stdout/stderr. It appends each Buffer.toString() chunk to separate unbounded strings and resolves one result after waitForChildProcess settles.

Abort handling is installed after spawn; a pre-aborted signal still permits spawning before the abort check. abort and a positive/truthy timeout both set killed=true and send SIGTERM to the direct process. A five-second timer then sends SIGKILL only if !proc.killed; Node's killed flag indicates a signal was sent, so that check does not establish that the process exited. The escalation timer is not cleared. waitForChildProcess releases an exited child after 100 ms of pipe inactivity, rearming for each later stdout/stderr chunk, or settles on close/error. It does not retry execution.

On normal exit, clear timeout and remove the abort listener; code is exitCode ?? 0. Any execution rejection resolves {stdout, stderr, code:1, killed}, without an error string. stdout and stderr decode independently per chunk, so split UTF-8 can be replaced. options.cwd is declared but this implementation uses the separate cwd argument; its caller chooses the effective cwd before invoking it.

### 5. Interfaces

```ts
interface ExecOptions { signal?: AbortSignal; timeout?: number; cwd?: string }
interface ExecResult { stdout: string; stderr: string; code: number; killed: boolean }
execCommand(command: string, args: string[], cwd: string, options?: ExecOptions): Promise<ExecResult>
```
timeout is milliseconds, unlike Bash tool seconds. No shell metacharacter interpretation, stdin API, output limit, session path or RPC protocol is defined here.

### 6. Dependencies

`AGENT-050`, `AGENT-051`, `AGENT-052`, `AGENT-053`

Extension host wiring/captured cwd is owned by the extensions area (dependency request retained until that entry is published). Low-level process draining and decoding overlap existing AGENT-050/051/053; this entry owns the distinct direct-argv API contract.

Final dependency classification. Shared contract references: `EXT-056`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

rust:crates/ice-tools/src/env.rs:273-287 exposes Shell::exec(command:&str, ShellExecOptions), and rust:crates/ice-tools/src/os_env.rs:810-1110 implements shell-string execution, typed execution errors and process-tree cancellation. Those capabilities do not implement the exact argv helper or its always-resolved ExecResult shape. No exec_command/direct argv helper is exported by rust:crates/ice-tools/src/lib.rs:14-54. The extensions worker independently confirmed the native runtime action enum/dispatcher has no ExtensionAPI.exec equivalent; it owns that boundary evidence. Existing argv subprocesses for installation/build helpers are purpose-specific, not this reusable API.

### 8. Port notes

Add a direct argv operation in ice-tools with explicit cwd, millisecond timeout and cancellation, then expose it through the native extension boundary. Use tokio::process::Command with no shell and separate streaming decoders; return transport failures distinctly or provide the exact compatibility result adapter. Preserve Rust process-tree ownership rather than reproducing the proc.killed escalation bug. Bound capture only with an explicit documented API policy and truncation metadata, since TS has no cap. Future tests should cover pre-abort, spawn failure, nonzero/null/signal exit, split UTF-8 and continuously writing post-exit descendants.

### 9. Size and priority

Estimated Rust implementation: **130-220 lines** (estimate, not measured source). **P1** - Extensions cannot reuse the TypeScript direct-argv contract even though Rust has lower-level shell execution.

## TOOLS-018 - Tool factories, definition adapters and execution context

### 1. Title and status

Tool factories, definition adapters and execution context. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/tools/index.ts:1-196; ice:packages/coding-agent/src/core/tools/tool-definition-wrapper.ts:1-47; ice:packages/agent/src/harness/tools/index.ts:1-23; ice:packages/agent/src/harness/tools/tool-context.ts:1-6

Named export: `ToolDef` (`ice:packages/coding-agent/src/core/tools/index.ts:82`).

### 3. What it does

SDK users can construct consistent coding, read-only or complete built-in tool sets and move between rich UI definitions and runtime tools without losing execution metadata.

### 4. How it works

The coding-agent factory switches over seven fixed built-ins and passes only that tool's options. Definition factories retain render/prompt metadata, while runtime factories wrap the executable subset for the agent loop.

ToolName and allToolNames contain read, bash, edit, write, grep, find, ls. Coding sets are ordered [read, bash, edit, write]; read-only sets are [read, grep, find, ls]; all sets are records in read, bash, edit, write, grep, find, ls order. Both individual factory switches throw 'Unknown tool name: <name>' on an unsupported runtime value. Options are independently scoped read?, bash?, write?, edit?, grep?, find?, ls?. Control tools are intentionally not in these seven-name factories; session activation creates them when Bash is active.

wrapToolDefinition copies name, label, description, parameters, constrainedSampling, prepareArguments, executionMode and forwards execute. An explicit call context wins over ctxFactory; the factory is evaluated per call only when context is nullish. Missing both produces undefined context at runtime despite the cast. Reverse createToolDefinitionFromAgentTool preserves the same metadata but invokes the original execute without forwarding ExtensionContext, so it cannot reconstruct rich renderers/prompt additions. The harness index separately exports its four environment tools, and ExecutionToolContext is {env:ExecutionEnv}. No persisted state or retries exist in these adapters.

### 5. Interfaces

createToolDefinition/createTool(name, cwd, ToolsOptions?), createCodingToolDefinitions/createCodingTools, createReadOnlyToolDefinitions/createReadOnlyTools, createAllToolDefinitions/createAllTools; wrapToolDefinition(definition, ctxFactory?), wrapToolDefinitions, createToolDefinitionFromAgentTool. The seven-name union and ordered subsets above are public contracts. The harness tool context holds an ExecutionEnv; coding-agent definitions additionally receive ExtensionContext on execute.

### 6. Dependencies

`AGENT-012`, `AGENT-014`, `AGENT-016`

Individual behavior is specified in TOOLS-001–030. Session active-tool registry and extension renderer/prompt metadata require cross-area integration; do not silently include background-control tools in the seven-name union. Session integration is reviewed in SESSION-023 (tool registry/context), SESSION-024 (user Bash), and SESSION-025 (background delivery, wake and disposal); these are consumer links, not prerequisites for implementing the tool registry.

Final dependency classification. Consuming integrations: `SESSION-023`. Shared contract references: `EXT-035`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

rust:crates/ice-tools/src/lib.rs:30-54 and rust:crates/ice-tools/src/tools/mod.rs:1-27 export individual factories and also the Rust-native PowerShell tool. No equivalent coding/read-only/all set factory or generic name switch is exported. rust:crates/ice-agent/src/agent_tool.rs:29-65 preserves schema, label, execution_mode, prepare_arguments and execute, but execution context is captured by implementations rather than passed per call, and there is no TS definition-to-runtime metadata adapter at this layer. rust:crates/ice-tools/src/tools/tool_context.rs:22-42 splits env and optional mutating_env; write/edit constructors expect the latter. That explicit ownership split is a deliberate Rust design worth retaining, not a missing filesystem abstraction.

### 8. Port notes

Add idiomatic typed factory-set helpers and an adapter boundary in ice-tools/ice-agent only where needed by the native SDK. Preserve the read-only versus mutating environment split and validate unavailable mutation capability with a construction error rather than an unexpected panic. Keep UI metadata in the TUI/extension layer and define context precedence explicitly. Preserve Rust-only PowerShell in its own selectable set without changing the seven TypeScript built-in names.

### 9. Size and priority

Estimated Rust implementation: **100-170 lines** (estimate, not measured source). **P2** - Factory and adapter mismatches make SDK consumers reconstruct tool sets and can drop execution or display metadata.

## TOOLS-019 - Exact truncation metadata, UTF-8 boundaries and line numbering

### 1. Title and status

Exact truncation metadata, UTF-8 boundaries and line numbering. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/tools/truncate.ts:1-276; ice:packages/agent/src/harness/utils/truncate.ts:1-350; ice:packages/agent/test/harness/truncate.test.ts:1-179; ice:packages/coding-agent/src/core/tools/output-accumulator.ts:1-291

Named export: `GREP_MAX_LINE_LENGTH` (`ice:packages/agent/src/harness/utils/truncate.ts:13`).

Named export: `GREP_MAX_LINE_LENGTH` (`ice:packages/coding-agent/src/core/tools/truncate.ts:13`).

### 3. What it does

Read, search and Bash output share line/byte limits and metadata that tell the model exactly which part of a file or command it sees.

### 4. How it works

Head truncation preserves complete initial lines; tail truncation preserves complete final lines unless the final line alone exceeds the byte limit. Both track UTF-8 bytes and logical LF-delimited lines, excluding the empty split element after a final LF.

Defaults are 2000 lines and 50*1024 bytes; grep line clipping is 500 UTF-16 code units. Untruncated output preserves input exactly, including its final LF. On truncation, joined complete lines omit that final LF; each intervening newline costs one byte. Head returns empty content with firstLineExceedsLimit=true if the first line alone is too large. Tail walks backward; if no full line fits, keep the last maxBytes snapped forward past UTF-8 continuation bytes and set lastLinePartial=true. Otherwise stop before the first line that would exceed the budget. Record the actual collected array length as outputLines. After collecting maxLines, the reported binding limit is 'lines', even if a partial final line was used with a one-line cap. No truncation sets truncatedBy:null.

There is a narrow TS edge: when only a trailing LF pushes bytes over the budget but all logical line bodies fit and the line cap is not reached, it still reports truncatedBy:'lines' because that is the initialized default. The harness implementation separately computes UTF-8 byte lengths without Buffer, including surrogate pairs/replacement handling, while coding-agent uses Buffer.byteLength and Buffer slicing. formatSize is NB below 1024, N.NKB below 1 MiB, otherwise N.NMB, without parentheses. truncateLine returns the first maxChars JS code units plus '... [truncated]' and can bisect a surrogate pair. These helpers do not validate negative/fractional options; normal tool callers use bounded defaults.

### 5. Interfaces

TruncationOptions {maxLines?:number, maxBytes?:number}; TruncationResult {content, truncated, truncatedBy:'lines'|'bytes'|null, totalLines, totalBytes, outputLines, outputBytes, lastLinePartial, firstLineExceedsLimit, maxLines, maxBytes}. truncateHead, truncateTail, truncateLine, formatSize. Detail field names are camelCase on the TS wire. There are no settings, filesystem writes, retries or cancellation in pure truncation.

### 6. Dependencies

TOOLS-010/015 use these counts for Bash range notices; TOOLS-022/028/029/030 use head/line limits. Rust implementation can retain safe scalar boundaries while explicitly documenting any Unicode display deviation.

### 7. ice-rust today

rust:crates/ice-tools/src/truncate.rs:121-188 largely preserves head behavior but initializes truncated_by=None, leaving it null for the trailing-LF-only byte-overflow edge. rust:crates/ice-tools/src/truncate.rs:193-285 computes the correct tail text but recomputes output_lines as min(total_lines, max_lines), ignoring max_bytes unless the single-partial-line branch applies. Thus truncate_tail('aaaa\nbbbb\ncccc',{max_lines:2000, max_bytes:9}) emits 'bbbb\ncccc' but reports three output lines; this is direct source reasoning, not an executed example. Range notices using that count can identify the wrong first line. Rust serializes snake_case field names (lines 48–69), formats sizes with parentheses '(50.0)KB' (107–115), and truncate_line counts Unicode scalars rather than JS UTF-16 units (rust:crates/ice-tools/src/truncate.rs:308-335). Existing tests exercise common output and partial final lines, not the byte-bound multi-line recount.

### 8. Port notes

Return the actual collected line count from truncate_tail and remove the approximate recount. Add a TS wire adapter or serde camelCase aliases rather than duplicating logic in each tool. Define whether to preserve the trailing-LF edge's label or intentionally correct it consistently across clients. Keep Rust's valid UTF-8 scalar slicing as a documented safety deviation where JS could emit a lone surrogate. Future tests must verify both text and metadata at exact byte/line boundaries, including multibyte final lines, empty input and final LF.

### 9. Size and priority

Estimated Rust implementation: **80-140 lines** (estimate, not measured source). **P0** - Incorrect outputLines makes Bash range notices misleading even when the visible tail text is correct.

## TOOLS-020 - Tool path normalization and ordered read-path fallback

### 1. Title and status

Tool path normalization and ordered read-path fallback. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/tools/path-utils.ts:1-118; ice:packages/coding-agent/src/utils/paths.ts:7-100; ice:packages/agent/src/harness/tools/path-utils.ts:1-30; ice:packages/coding-agent/test/path-utils.test.ts:1-175

`PathInputOptions` — `ice:packages/coding-agent/src/utils/paths.ts:9`.

### 3. What it does

Tool calls accept user-style paths and locate common macOS screenshot spellings while keeping write/edit destinations literal after normalization.

### 4. How it works

All coding-agent tool paths normalize a defined Unicode-space set, remove one leading @, expand only literal ~ or ~/ (also ~\ on Windows), decode file:// URLs, and resolve dot segments against normalized cwd. They do not trim ordinary leading/trailing whitespace or canonicalize symlinks at this stage.

The normalized space set is U+00A0, U+2000–U+200A, U+202F, U+205F, U+3000. fileURLToPath errors propagate. Read fallback tests candidates in order: original resolved path; replace space before case-insensitive AM/PM followed by '.' with U+202F; NFD decomposition; ASCII apostrophe→U+2019; NFD plus curly apostrophe. There is no AM/PM+NFD cross-product. First existing candidate wins; if none exists, return the original resolved path so the tool reports its normal error. Sync/async coding helpers use access(F_OK) and treat every access failure as false. They use the local filesystem even when a read tool has custom operations.

The harness delegates absolute-path/exists probes to ExecutionEnv with the supplied signal. Its env.exists Result errors propagate through getOrThrow instead of being treated as false. Only read uses the spelling variants in TypeScript; find/grep/ls resolve the supplied path normally. Mutation queue canonicalization is a separate later step. This helper performs no authorization check; trust and sandbox policy belong to the host.

### 5. Interfaces

expandPath, resolveToCwd, resolveReadPath, resolveReadPathAsync, pathExists; harness normalizeToolPath, resolveToolPath, resolveReadToolPath. Shared normalizePath options {trim?, expandTilde?, homeDir?, stripAtPrefix?, normalizeUnicodeSpaces?}; tool callers enable stripAtPrefix and normalizeUnicodeSpaces. Inputs remain strings, and errors from file URLs or harness filesystem operations can reject the tool.

Exported supporting type (verbatim):
```ts
export interface PathInputOptions {
	/** Trim leading/trailing whitespace before normalization. */
	trim?: boolean;
	/** Expand leading `~` to a home directory. Defaults to true. */
	expandTilde?: boolean;
	/** Home directory used for `~` expansion. Defaults to `os.homedir()`. */
	homeDir?: string;
	/** Strip a leading `@`, used for CLI @file paths. */
	stripAtPrefix?: boolean;
	/** Normalize unicode space variants to regular spaces. */
	normalizeUnicodeSpaces?: boolean;
}
```
Source: `ice:packages/coding-agent/src/utils/paths.ts:9-20`.

### 6. Dependencies

`AGENT-038`, `AGENT-043`, `AGENT-045`

Host filesystem abstraction and path trust checks are cross-area responsibilities. TOOLS-021 canonicalizes mutation aliases after this normalization; TOOLS-022 owns read fallback use.

### 7. ice-rust today

rust:crates/ice-tools/src/path_utils.rs:1-193 implements the same ordered read variants and delegates errors to ExecutionEnv, but the normalized-space table omits U+2000/U+2001. rust:crates/ice-tools/src/os_env.rs:147-170 expands tilde and resolves paths but keeps a literal file:// string if parsing fails, unlike TS fileURLToPath throwing. OsExecutionEnv.exists uses symlink_metadata, so a dangling symlink counts as existing, while TS coding access(F_OK) follows the target (rust:crates/ice-tools/src/os_env.rs:647-658). Rust find/grep/ls call resolve_read_tool_path, broadening fallback beyond TypeScript read. Existing Rust behavior correctly preserves literal names beginning '~' other than the explicit expansion forms; do not label all tilde handling missing.

### 8. Port notes

Align the Unicode set and define file URL failure behavior in ice-tools. Keep generic path resolution separate from read spelling heuristics, and apply the latter only to read for TS parity. Expose exists/access semantics distinctly if custom environments need dangling-symlink behavior. Port the existing TS filename test matrix as future tests using temporary fixtures during implementation, with no actual user-home access.

### 9. Size and priority

Estimated Rust implementation: **70-130 lines** (estimate, not measured source). **P1** - Small normalization and fallback differences can read an unintended variant or reject a path accepted by the TypeScript tools.

## TOOLS-021 - Canonical-path mutation queue, admission order and alias ownership

### 1. Title and status

Canonical-path mutation queue, admission order and alias ownership. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/tools/file-mutation-queue.ts:1-78 withFileMutationQueue; ice:packages/agent/src/harness/tools/file-mutation-queue.ts:1-56; ice:packages/coding-agent/test/file-mutation-queue.test.ts:1-301; ice:packages/agent/test/harness/tools.test.ts:1-623

### 3. What it does

Concurrent edits and writes to the same destination serialize in invocation order, including symlink spellings and not-yet-created targets, while different files can proceed concurrently.

### 4. How it works

The coding-agent queue separates admission from execution: a global registration promise serializes asynchronous canonical-key resolution, then each canonical path has its own promise gate. This preserves call order even when the first path lookup is slower, without forcing different files' mutations to run serially.

Canonical resolution first tries realpath. On ENOENT/ENOTDIR, lstat detects a dangling symlink; readlink resolves its target and recursively computes the key. Otherwise walk up missing parent components, canonicalize the existing ancestor, then append the missing suffix. Other errors propagate. During registration, capture the prior path promise, install a chained gate and release the global registration slot; await the prior path gate, execute the mutation, and finally resolve this gate. Delete the Map entry only if it is still the latest gate for that path, so an earlier completion cannot evict a queued successor.

The mutation closure includes all awaited access/read/mkdir/write operations and post-I/O abort checks. Abort does not release the path lock while an already-started operation remains pending. There is no retry or durable queue; registry state is process-local. The harness version uses a WeakMap keyed by ExecutionEnv, with a registration promise and path map per environment. It calls absolutePath/canonicalPath without an abort signal during admission; NotFound/NotSupported canonicalization falls back to absolute path rather than implementing the coding-agent's missing-parent/symlink recursion. The tests distinguish sequential same-path ordering, parallel independent files, canonical/symlink aliasing, missing parents and in-flight cancellation.

### 5. Interfaces

withFileMutationQueue<T>(absolutePath:string, fn:()=>Promise<T>):Promise<T> for coding-agent; harness withFileMutationQueue<T>(env, path, fn) with a per-env registry. Only internal canonical keys/gates are stored; there is no on-disk lock file, user setting, RPC shape or tool parameter.

### 6. Dependencies

`AGENT-045`, `TOOLS-020`

TOOLS-024/025 must hold this queue through all mutation I/O. ExecutionEnv canonical-path and abort semantics are host dependencies. The coding-agent and harness alias policies are distinct source contracts.

### 7. ice-rust today

rust:crates/ice-tools/src/file_mutation_queue.rs:1-137 already uses a per-MutatingEnv registry and per-key Tokio mutex held across the full mutation closure. rust:crates/ice-tools/tests/abort_bracketing.rs:232-308 establishes the intended in-flight cancellation lock invariant; tests were read, not run. However key resolution occurs before registry locking and no global registration queue preserves invocation order, so slower canonicalization can be overtaken. Keys use caller cancellation during resolution, unlike TS harness admission. Completed keys are never removed from the HashMap. OsExecutionEnv.canonical_path is simply tokio::fs::canonicalize (rust:crates/ice-tools/src/os_env.rs:635-645); on a missing target the registry falls back to lexical absolute path, so it misses coding-agent canonicalization through an existing symlinked ancestor or dangling link. The Rust sharing test explicitly uses identical absolute spellings instead of real symlinks and allows either concurrent write to win (rust:crates/ice-tools/tests/edit_mutation_queue_sharing.rs:1-13; rust:crates/ice-tools/tests/edit_mutation_queue_sharing.rs:53-110); it does not prove TS invocation order or missing-target alias behavior.

### 8. Port notes

Add an admission-order primitive to MutationQueueRegistry and a canonical-existing-ancestor resolver for coding-agent compatibility. Keep lock ownership through all in-flight I/O after cancellation. Remove idle entries only when no queued owner can reference a different replacement lock; use Arc identity plus guarded registration. Preserve the explicit MutatingEnv registry and do not use a global filesystem lock. Future tests need deliberately inverted canonicalization delays, symlinked missing targets, dangling aliases, parallel different files and cancellation while write remains blocked.

### 9. Size and priority

Estimated Rust implementation: **140-240 lines** (estimate, not measured source). **P0** - Concurrent edits can be reordered or escape alias serialization, causing lost updates to the same real file.

## TOOLS-022 - Read-tool pagination, cancellation and model-aware image handoff

### 1. Title and status

Read-tool pagination, cancellation and model-aware image handoff. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/tools/read.ts:1-351 createReadToolDefinition; ice:packages/agent/src/harness/tools/read.ts:1-144 createReadTool; ice:packages/coding-agent/test/tools.test.ts:1-473; ice:packages/agent/test/harness/tools.test.ts:1-623; ice:packages/coding-agent/test/image-resize-callers.test.ts:1-54

Named export: `ReadToolDetails` (`ice:packages/agent/src/harness/tools/read.ts:24`).

Named export: `ReadImageProcessorResult` (`ice:packages/agent/src/harness/tools/read.ts:28`).

Named export: `ReadToolDetails` (`ice:packages/coding-agent/src/core/tools/read.ts:28`).

### 3. What it does

The read tool returns a bounded text page or a processed image, explains truncation and provides the next offset. It reports image limitations without silently pretending binary bytes are ordinary text.

### 4. How it works

Read first resolves the ordered path variants, checks readability, then branches on detected image MIME. Text is read as a whole file, decoded, sliced by line offset/limit and passed through head truncation; this is bounded output, not streaming bounded-memory file reading.

The schema is {path:string, offset?:number, limit?:number}. offset is one-based; start is offset ? max(0, offset-1) : 0. Split on LF without dropping the final empty element for pagination totals; an offset beyond that array throws. A defined limit chooses min(start+limit, totalLines), then JS slice coercion applies (the TypeBox Number fields themselves do not enforce integers or nonnegative values). Each selected page is joined with LF and truncated to 2000 lines/51200 UTF-8 bytes. If its first line exceeds the byte cap, return the oversized-line explanation and a suggested sed -n command. That suggestion embeds the original path string; the tool does not execute it. Otherwise append shown-range/total/next-offset guidance for truncation, or a next-offset notice when only the caller limit ended the page. Truncation details appear only when needed.

The coding implementation reads Buffer.toString('utf-8'); the harness uses TextDecoder, whose default consumes an initial UTF-8 BOM. Coding's default detectImageMimeType opens and probes only the first 4100 bytes; custom read operations may omit detection and then all data is treated as text. On an image, read the full bytes and call processImage(autoResizeImages default true); errors/unsupported images become explicit text-only omissions. When ctx.model lacks image input, prepend '[Current model does not support images. The image will be omitted from this request.]' while still returning the image block; later message conversion owns omission from the actual request.

Coding read checks abort before starting, installs an abort listener that rejects immediately, checks after path/readability work and again after the image/text work. Local filesystem promises themselves are not canceled, so they may finish after rejection. Listener cleanup happens on settlement. Harness delegates cancellation through env calls; its image callback need not be cancellation-aware. No retry, file mutation, per-file read lock or snapshot consistency guarantee exists.

### 5. Interfaces

Tool name/label read; parameters {path:string, offset?:number, limit?:number}; ReadOperations {readFile(path):Promise<Buffer>, access(path):Promise<void>, detectImageMimeType?(path):Promise<string|null|undefined>}; ReadToolOptions {autoResizeImages?:boolean, operations?:ReadOperations}; harness additionally accepts imageProcessor. Result content contains text and optionally {type:'image', data:<base64>, mimeType}; details {truncation?:TruncationResult}. ctx.model.input controls the visible nonvision warning; autoResizeImages is injected from settings by the session.

### 6. Dependencies

`AGENT-040`, `SET-016`, `TOOLS-019`, `TOOLS-020`, `TOOLS-023`

Session model context and image filtering/settings are cross-area dependencies. The read renderer is TOOLS-027. There is no intrinsic file permission approval policy in this tool.

### 7. ice-rust today

rust:crates/ice-tools/src/tools/read.rs:1-350 already implements text pages, oversized-line guidance, image detection and optional ReadImageProcessor. The CLI supplies InlineImageProcessor (rust:crates/ice-cli/src/session.rs:1972-1981), so image processing is present. Rust deserializes offset/limit as unsigned integers, rejecting TS-permitted negative/fractional numbers, and uses saturating one-based offset conversion. Its bytes use String::from_utf8_lossy, retaining a BOM unlike the harness TextDecoder. Its context has no per-call selected-model capability so it does not emit the coding-agent nonvision warning. Full-file image detection differs from the 4100-byte probe (TOOLS-023). Cancellation is env-token based; there is no coding-agent-style immediate-rejection listener around an uncancelable image processor. Detail/truncation keys use snake_case. rust:crates/ice-tools/tests/read_truncation.rs:1-243 covers pagination/truncation, but not these model, coercion or cancellation differences.

### 8. Port notes

Retain Rust's whole-file environment abstraction and implement a deliberate compatible wire schema, including a stated choice to keep unsigned integer pagination validation. Pass model image capability and auto-resize policy through a tool invocation context or adapter. Use an async image operation with cancellation disposition; ensure late results cannot mutate settled output. Keep the model message-filtering responsibility outside the read tool. Future tests should verify exact next offsets, trailing LF, BOM, byte-first-line overflow, nonvision warnings and abort during image work.

### 9. Size and priority

Estimated Rust implementation: **110-190 lines** (estimate, not measured source). **P1** - The basic tool works, but pagination coercion, image warnings and cancellation contracts differ in model-visible ways.

## TOOLS-023 - Image signatures, conversion, resizing and coordinate hints

### 1. Title and status

Image signatures, conversion, resizing and coordinate hints. **Status: partial.**

### 2. ice source

ice:packages/agent/src/harness/tools/image.ts:1-104; ice:packages/coding-agent/src/utils/mime.ts:1-117; ice:packages/coding-agent/src/utils/image-process.ts:1-120; ice:packages/coding-agent/src/utils/image-convert.ts:1-50; ice:packages/coding-agent/src/utils/image-resize.ts:1-124; ice:packages/coding-agent/src/utils/image-resize-core.ts:1-165; ice:packages/coding-agent/src/utils/image-resize-worker.ts:1-43; ice:packages/coding-agent/src/utils/exif-orientation.ts:1-184; ice:packages/coding-agent/src/utils/photon.ts:1-140; ice:packages/coding-agent/test/image-process.test.ts:1-54; ice:packages/coding-agent/test/image-processing.test.ts:1-171; ice:packages/coding-agent/test/image-resize-callers.test.ts:1-54

Named export: `encodeBase64` (`ice:packages/agent/src/harness/tools/image.ts:12`).

`applyExifOrientation` — `ice:packages/coding-agent/src/utils/exif-orientation.ts:147`. `convertImageBytesToPng` — `ice:packages/coding-agent/src/utils/image-convert.ts:4`. `ProcessImageOptions` — `ice:packages/coding-agent/src/utils/image-process.ts:4`. `ProcessImageResult` — `ice:packages/coding-agent/src/utils/image-process.ts:11`. `ImageResizeOptions` — `ice:packages/coding-agent/src/utils/image-resize-core.ts:4`. `ResizedImage` — `ice:packages/coding-agent/src/utils/image-resize-core.ts:11`. `resizeImageInProcess` — `ice:packages/coding-agent/src/utils/image-resize-core.ts:59`. `detectSupportedImageMimeTypeFromFile` — `ice:packages/coding-agent/src/utils/mime.ts:25`. `loadPhoton` — `ice:packages/coding-agent/src/utils/photon.ts:116`.

### 3. What it does

Image reads attach supported content within model size limits, convert BMP when necessary, and tell the model how displayed coordinates map back to the original image.

### 4. How it works

Image processing first validates magic bytes, then normalizes/converts the MIME and optionally resizes the decoded image. Resizing uses a worker when possible and returns text-only omission if safe output cannot be produced.

Detection: JPEG begins FF D8 FF but rejects a fourth F7 marker; PNG requires at least 16 bytes and IHDR length 13, then rejects acTL before IDAT; GIF accepts the first three bytes 'GIF'; WebP requires RIFF and WEBP at offset 8. BMP requires at least 26 bytes, consistent declared sizes, a DIB header 12 or 40–124, planes 1 and bit depth 1/4/8/16/24/32. Bounds-safe header readers return 0. The coding read's 4100-byte probe may not see a later acTL chunk; the harness tests the complete bytes. Base64 uses the standard alphabet with padding and no line breaks.

processImage strips MIME parameters, trims/lowercases and maps image/jpg to image/jpeg. PNG/JPEG/GIF/WebP are model-supported; other detected image types are converted to PNG via Photon. Conversion failure produces an omission message. autoResizeImages defaults true; false returns normalized supported bytes (or converted PNG) plus '[Image converted from <old> to <new>.]' when conversion happened. Resize failure/null also produces text-only omission, never an unsafe original fallback.

Resize defaults: maxWidth=maxHeight=2000, maxBytes=4.5*1024*1024 measured on base64 string length, jpegQuality80. Decode via Photon, apply EXIF orientation before measuring dimensions, and preserve original bytes if oriented dimensions and original base64 length are already within the limits. That early-return path keeps original encoded orientation even when a decoded orientation transform was applied. Otherwise constrain width then height with Math.round and Lanczos3. At each candidate dimension, resize from the original decoded image, try PNG first, then unique JPEG qualities [requestedQuality,85,70,55,40], and return the first candidate strictly smaller than maxBytes. It does not search for the smallest successful encoding. If none fits, floor both dimensions times 0.75 (minimum 1) and repeat down to 1x1; failure returns null. WASM images are freed in finally.

The dimension hint is '[Image: original WxH, displayed at wxh. Multiply coordinates by SCALE to map to original image.]', SCALE=(originalWidth/width).toFixed(2); it is emitted whenever wasResized=true, including re-encoding without dimension change. EXIF support probes JPEG APP1 and WebP EXIF, TIFF orientation 1–8; flips mutate in place, rotations allocate a new image. It does not scan PNG EXIF. The first JPEG APP1 without an Exif header ends the lookup; malformed/absent orientation defaults 1.

Worker transport copies caller bytes before transfer, validates result/error messages, rejects worker errors or exit before a result, and terminates the worker in finally. Bun compiled builds try './src/utils/image-resize-worker.ts', then TS/JS URL candidates, then in-process fallback. There is no explicit AbortSignal or timer. Photon module loading caches the promise (including failure), temporarily patches readFileSync to locate photon_rs_bg.wasm near the executable or cwd, and restores it after load. These packaging mechanics need not be reproduced by native Rust.

### 5. Interfaces

detectSupportedImageMimeType(bytes) -> image/jpeg|image/png|image/gif|image/webp|image/bmp|null; processImage(bytes, mime,{autoResizeImages?, resizeOptions?:ImageResizeOptions}) -> {ok:true, data, mimeType, hints}|{ok:false, message}; resizeImage(bytes, mime,{maxWidth?, maxHeight?, maxBytes?, jpegQuality?}) -> {data, mimeType, width, height, originalWidth, originalHeight, wasResized}|null; formatDimensionNote(result). ReadImageProcessor returns base64 image content plus optional explanatory text. External dependency: @silvia-odwyer/photon-node WASM in TS; no network service is contacted by the image pipeline.

Exported supporting type (verbatim):
```ts
export interface ProcessImageOptions {
	/** Whether to resize images to inline provider limits. Default: true */
	autoResizeImages?: boolean;
	/** Optional resize overrides. Uses resizeImage defaults when omitted. */
	resizeOptions?: ImageResizeOptions;
}
```
Source: `ice:packages/coding-agent/src/utils/image-process.ts:4-9`.

Exported supporting type (verbatim):
```ts
export type ProcessImageResult =
	| {
			ok: true;
			data: string;
			mimeType: string;
			hints: string[];
	  }
	| {
			ok: false;
			message: string;
	  };
```
Source: `ice:packages/coding-agent/src/utils/image-process.ts:11-21`.

Exported supporting type (verbatim):
```ts
export interface ImageResizeOptions {
	maxWidth?: number; // Default: 2000
	maxHeight?: number; // Default: 2000
	maxBytes?: number; // Default: 4.5MB of base64 payload (below Anthropic's 5MB limit)
	jpegQuality?: number; // Default: 80
}
```
Source: `ice:packages/coding-agent/src/utils/image-resize-core.ts:4-9`.

Exported supporting type (verbatim):
```ts
export interface ResizedImage {
	data: string; // base64
	mimeType: string;
	originalWidth: number;
	originalHeight: number;
	width: number;
	height: number;
	wasResized: boolean;
}
```
Source: `ice:packages/coding-agent/src/utils/image-resize-core.ts:11-19`.

### 6. Dependencies

`SET-016`

This entry and TOOLS-022 are a behavior interface pair; avoid a build dependency cycle by placing pure image interfaces below read. Session owns autoResizeImages settings and model image transport. CLI file attachments reuse this pipeline but their argument processing belongs to CLI.

Final dependency classification. Shared contract references: `CLI-009`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

rust:crates/ice-tools/src/image.rs:1-236 has native signature detection/base64. It requires GIF length>=6 instead of TS's 3 and accepts BMP DIB lengths 13–39 in its non 12 branch, which TS rejects. rust:crates/ice-tools/src/image_processing.rs:1-485 and rust:crates/ice-cli/src/session.rs:1972-1981 provide actual native read-image processing. Rust applies decoded orientation and preserves original bytes only with NoTransforms; it tries PNG then JPEG85/70/50 with white alpha compositing and at most 8 shrink iterations, resizing the current image rather than the original each time. TS quality 80/40 candidates, unbounded-until 1x1 search and exact encoding selection are absent. Rust's dimension hint is '[Image resized from ...]' and lacks the original-coordinate multiplier. With auto-resize disabled, BMP conversion does not include the TS conversion hint. The ReadImageProcessor trait is synchronous and runs CPU work on the calling path rather than TS worker isolation. Native code avoids Photon/WASM bootstrap requirements, which is a useful design difference, not a gap. Tests read cover BMP conversion, valid images, size bounds and malformed input, but no pixel-equivalence or orientation-runtime result is claimed.

### 8. Port notes

Keep native image/base64 crates. Align signature edge cases, MIME normalization, omission messages and coordinate/conversion hints, then decide which encoder-quality/search differences are accepted native behavior. Use spawn_blocking or a dedicated bounded image worker with cancellation disposition to avoid blocking async execution. Preserve Rust's corrected orientation handling and explicit decode errors rather than reproducing TS's original-byte early-return quirk. Future tests should cover BMP malformed DIBs, APNG placement, EXIF rotation, transparent-to-JPEG behavior, original-coordinate hints and outputs near the strict base64 budget.

### 9. Size and priority

Estimated Rust implementation: **180-310 lines** (estimate, not measured source). **P1** - Image attachment support exists, but format edges, size search and missing coordinate hints can change what the model sees or where it acts.

## TOOLS-024 - Write-tool I/O ordering, abort bracketing and result text

### 1. Title and status

Write-tool I/O ordering, abort bracketing and result text. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/tools/write.ts:1-267; ice:packages/agent/src/harness/tools/write.ts:1-39; ice:packages/coding-agent/test/file-mutation-queue.test.ts:1-301; ice:packages/agent/test/harness/tools.test.ts:1-623

Named export: `createWriteTool` (`ice:packages/agent/src/harness/tools/write.ts:15`).

Named export: `createWriteToolDefinition` (`ice:packages/coding-agent/src/core/tools/write.ts:181`).

Named export: `createWriteTool` (`ice:packages/coding-agent/src/core/tools/write.ts:265`).

### 3. What it does

Write creates missing parent directories and replaces a UTF-8 file while serializing against edits and preserving the in-flight mutation lock after cancellation.

### 4. How it works

The write tool resolves the destination and enters the canonical-path mutation queue before creating parents or writing. The full asynchronous mutation stays inside that queue, so abort cannot let a following edit race unfinished I/O.

Coding write checks abort before queue work, inside the queue, after recursive mkdir and after writeFile. The default operations call mkdir(dirname(path),{recursive:true}) and writeFile(path, content,'utf8'); injected operations may substitute both methods. A failure propagates and releases the queue only after its promise settles. The operation is not specified as atomic temp-file rename and performs no backup or retry. The success string is 'Successfully wrote <content.length> bytes to <path>'; content.length is UTF-16 code units, so the label 'bytes' is inaccurate for non-ASCII text. It preserves the caller's original path in that message.

The harness delegates to env.writeFile with FileContent text, inside the same per-environment queue and with abort checks before/after. Parent-directory creation is delegated to that environment. Neither tool authorizes external paths itself; the host policy must do so. Rendering of streaming input content is a separate definition concern in TOOLS-027.

### 5. Interfaces

Tool write; parameters {path:string, content:string}; WriteOperations {writeFile(path, content):Promise<void>, mkdir(dir):Promise<void>}; WriteToolOptions {operations?:WriteOperations}. The result is one text content block; no file-size or content-hash field is returned. No user settings, additional RPC command or persisted journal is declared.

### 6. Dependencies

`AGENT-041`, `AGENT-046`, `TOOLS-020`, `TOOLS-021`

Host filesystem permissions and caller trust gates are cross-area dependencies. TOOLS-027 owns content preview; queue behavior must be shared with TOOLS-025 edit.

### 7. ice-rust today

rust:crates/ice-tools/src/tools/write.rs:1-154 already resolves paths, requires a MutatingEnv, queues writes and brackets the awaited write with cancellation checks. rust:crates/ice-tools/tests/abort_bracketing.rs:232-308 covers keeping the queue locked until a canceled write settles. Rust success text is 'Successfully wrote to <path>' and omits TS's count, and custom I/O is supplied through ExecutionEnv rather than per-tool operations. The existing queue's canonicalization/admission differences remain TOOLS-021. These are partial interface differences; the write operation itself is present.

### 8. Port notes

Keep the ExecutionEnv-based write and shared registry. For a TS-compatible text adapter either reproduce the UTF-16 count label exactly or deliberately report a real UTF-8 byte count with a documented message change; do not assume String::len matches TS content.length. Expose construction failure when no mutating environment exists, and preserve the lock through post-I/O abort checks. No new atomic-write promise should be implied without implementation.

### 9. Size and priority

Estimated Rust implementation: **40-90 lines** (estimate, not measured source). **P2** - The operation is implemented; remaining result and injection differences matter mainly to SDK compatibility and shared queue correctness.

## TOOLS-025 - Atomic edit batches with exact-match precedence and localized fuzzy normalization

### 1. Title and status

Atomic edit batches with exact-match precedence and localized fuzzy normalization. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/tools/edit.ts:1-437; ice:packages/coding-agent/src/core/tools/edit-diff.ts:1-594; ice:packages/agent/src/harness/tools/edit.ts:1-127; ice:packages/agent/src/harness/tools/edit-diff.ts:1-500; ice:packages/coding-agent/test/edit-diff-exact-precedence.test.ts:1-52; ice:packages/coding-agent/test/edit-tool-legacy-input.test.ts:1-117; ice:packages/coding-agent/test/tools.test.ts:1-1214

Named export: `detectLineEnding` (`ice:packages/agent/src/harness/tools/edit-diff.ts:7`).

Named export: `normalizeToLF` (`ice:packages/agent/src/harness/tools/edit-diff.ts:15`).

Named export: `restoreLineEndings` (`ice:packages/agent/src/harness/tools/edit-diff.ts:19`).

Named export: `normalizeForFuzzyMatch` (`ice:packages/agent/src/harness/tools/edit-diff.ts:30`).

Named export: `applyReplacementsPreservingUnchangedLines` (`ice:packages/agent/src/harness/tools/edit-diff.ts:128`).

Named export: `FuzzyMatchResult` (`ice:packages/agent/src/harness/tools/edit-diff.ts:171`).

Named export: `AppliedEditsResult` (`ice:packages/agent/src/harness/tools/edit-diff.ts:192`).

Named export: `fuzzyFindText` (`ice:packages/agent/src/harness/tools/edit-diff.ts:203`).

Named export: `stripBom` (`ice:packages/agent/src/harness/tools/edit-diff.ts:244`).

Named export: `applyEditsToNormalizedContent` (`ice:packages/agent/src/harness/tools/edit-diff.ts:301`).

Named export: `EditToolDetails` (`ice:packages/agent/src/harness/tools/edit.ts:42`).

Named export: `createEditTool` (`ice:packages/agent/src/harness/tools/edit.ts:77`).

Named export: `detectLineEnding` (`ice:packages/coding-agent/src/core/tools/edit-diff.ts:10`).

Named export: `normalizeToLF` (`ice:packages/coding-agent/src/core/tools/edit-diff.ts:18`).

Named export: `restoreLineEndings` (`ice:packages/coding-agent/src/core/tools/edit-diff.ts:22`).

Named export: `normalizeForFuzzyMatch` (`ice:packages/coding-agent/src/core/tools/edit-diff.ts:33`).

Named export: `applyReplacementsPreservingUnchangedLines` (`ice:packages/coding-agent/src/core/tools/edit-diff.ts:138`).

Named export: `FuzzyMatchResult` (`ice:packages/coding-agent/src/core/tools/edit-diff.ts:181`).

Named export: `AppliedEditsResult` (`ice:packages/coding-agent/src/core/tools/edit-diff.ts:202`).

Named export: `fuzzyFindText` (`ice:packages/coding-agent/src/core/tools/edit-diff.ts:213`).

Named export: `stripBom` (`ice:packages/coding-agent/src/core/tools/edit-diff.ts:254`).

Named export: `applyEditsToNormalizedContent` (`ice:packages/coding-agent/src/core/tools/edit-diff.ts:324`).

Named export: `EditToolDetails` (`ice:packages/coding-agent/src/core/tools/edit.ts:61`).

Named export: `EditToolOptions` (`ice:packages/coding-agent/src/core/tools/edit.ts:89`).

Named export: `createEditToolDefinition` (`ice:packages/coding-agent/src/core/tools/edit.ts:287`).

Named export: `createEditTool` (`ice:packages/coding-agent/src/core/tools/edit.ts:435`).

### 3. What it does

Edit applies a nonempty batch against one original file snapshot, rejects ambiguous or overlapping targets, and preserves BOM/line endings and untouched text. Exact occurrences take precedence over fuzzy lookalikes in the current coding-agent.

### 4. How it works

prepareArguments normalizes legacy input before schema validation. Execution resolves a path, enters the shared mutation queue, reads one original file and validates the entire batch before writing once; a missing/duplicate/overlapping/no-op edit leaves the file untouched.

The public schema is {path:string, edits:[{oldText:string, newText:string}]}. A JSON-string edits value is parsed only if it becomes an array. If both top-level oldText/newText are strings, append that legacy edit after a copied existing array and delete the legacy fields; invalid JSON/non-array values remain for validation. Runtime rejects an empty array or an empty oldText. Coding access requires read+write, then reads UTF-8 bytes; strip one leading BOM, detect the first newline style (CRLF iff that first LF is preceded by CR), normalize CRLF and lone CR to LF, and restore original BOM plus the detected style on output.

For each edit, count exact nonoverlapping occurrences first. More than one exact match is ambiguous; exactly one wins even if other fuzzy-normalized lookalikes exist. Only with zero exact matches normalize using NFKC, per-line trimEnd, smart single/double quotes, dash variants U+2010–2015/U+2212 to '-', and the defined space variants to ASCII. Zero fuzzy occurrences is not-found; more than one is ambiguous. If any edit is fuzzy, exact matches retain their original positions mapped into the normalized text (normalizing the prefix without trimEnd) rather than being searched again and accidentally moved. Verify the mapped needle still matches.

Sort all matches by position; reject overlapping ranges with original edit indices, then replace from the end. Fuzzy use widens changed areas to touched whole-line groups, merges overlapping groups, and copies untouched original lines verbatim; it checks line-count correspondence. This avoids normalizing unrelated smart quotes/trailing whitespace elsewhere in the file. Reject a final no-op result. Write only after every edit passes, check abort again after I/O, and return success count plus display diff/unified patch/firstChangedLine.

The older environment harness follows a different algorithm: count fuzzy occurrences even when an exact match exists and re-search against the chosen normalized base. Its read/file-info errors are wrapped with a tool-specific error code message. Coding errors use the access check and detailed per-edit not-found/duplicate/overlap messages. No retry, automatic alternate edit or partial batch commit occurs.

### 5. Interfaces

edit {path:string, edits:Array<{oldText:string, newText:string}>}; legacy top-level oldText/newText and JSON-array-string edits are accepted by prepareArguments. EditOperations {readFile, writeFile, access}; result text 'Successfully replaced N block(s) in <path>.' with details {diff:string, patch:string, firstChangedLine?:number}. The original file path is used in patch headers. Neither schema nor execution narrows path authority by itself.

### 6. Dependencies

`AGENT-039`, `AGENT-041`, `TOOLS-020`, `TOOLS-021`

TOOLS-026 specifies both diff formats and asynchronous preflight preview. Host authorization must run before mutations; session/extension argument preparation order is a cross-area dependency.

Final dependency classification. Shared contract references: `EXT-035`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

rust:crates/ice-tools/src/tools/edit.rs:1-294 already has queued batch edits, BOM/newline preservation, no-op/overlap validation and post-write cancellation. rust:crates/ice-tools/src/tools/edit_diff.rs:1-425 follows the older harness fuzzy occurrence/rescan algorithm, so a unique exact target can be rejected due to a distinct fuzzy lookalike or moved after normalization. Current coding-agent regression tests explicitly require exact precedence and preserving the original exact location. Rust adds accepted single edit objects and JSON-string single objects (rust:crates/ice-tools/tests/edit_fuzzy.rs:66-93), beyond TS's array-string normalization, and adds helpful duplicate-match context (up to 5 matches with surrounding lines and bounded snippets); preserve those native improvements if intentional. Its result detail key is first_changed_line, not firstChangedLine. Existing Rust tests verify common exact/fuzzy edits and untouched-on-error behavior (rust:crates/ice-tools/tests/edit_fuzzy.rs:95-254), but do not establish current coding-agent exact precedence.

### 8. Port notes

Port the current coding-agent matching pipeline into ice-tools while retaining Rust's improved ambiguity diagnostics and explicit single-object compatibility if desired. Preserve exact offsets across fuzzy normalization using a correct byte-index mapping, since TS indices are UTF-16 and Rust string indices are UTF-8. Keep the whole read/validate/write inside TOOLS-021. Future tests should use an exact occurrence plus smart-quote lookalike, exact and fuzzy edits in one batch, untouched Unicode lines, BOM/CRLF and failing final edit in a batch.

### 9. Size and priority

Estimated Rust implementation: **180-300 lines** (estimate, not measured source). **P0** - The current Rust matcher can reject or target a different occurrence than the audited coding-agent edit contract.

## TOOLS-026 - Trustworthy edit diffs and asynchronous preflight preview

### 1. Title and status

Trustworthy edit diffs and asynchronous preflight preview. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/tools/edit-diff.ts:1-594 generateDiffString,generateUnifiedPatch,computeEditsDiff; ice:packages/coding-agent/src/core/tools/edit.ts:79-292; ice:packages/agent/src/harness/tools/edit-diff.ts:1-500; ice:packages/coding-agent/test/edit-tool-no-full-redraw.test.ts:1-237

Named export: `EditDiffResult` (`ice:packages/coding-agent/src/core/tools/edit-diff.ts:539`).

Named export: `EditDiffError` (`ice:packages/coding-agent/src/core/tools/edit-diff.ts:544`).

Named export: `computeEditDiff` (`ice:packages/coding-agent/src/core/tools/edit-diff.ts:587`).

### 3. What it does

Before an edit executes, the terminal can show the predicted change or an applicability error. After execution it displays the actual diff without a duplicate success message or unnecessary full redraw.

### 4. How it works

The edit definition computes a read-only preview only after arguments are complete and replaces it with the result's actual diff on settlement. A key based on path+edits prevents a stale asynchronous preview from overwriting newer arguments.

computeEditsDiff checks read access, reads the current local file, normalizes BOM/line endings and calls the same edit algorithm without acquiring the mutation queue or writing. It catches errors into {error:string}; the file can change after preview, so execution remains authoritative. The renderer stores the JSON input key, pending state, preview result and component instance. Changed arguments clear the prior preview; identical pending keys do not launch duplicate work; completion is ignored if its key is obsolete. Compare diff and firstChangedLine before invalidating the UI. A settled result reconstructs the boxed diff even when argsComplete was never observed (session replay) and no file can still be read. Errors get their own preflight panel; success result text and duplicate error content are suppressed when the call panel already shows the payload.

The unified patch uses createTwoFilesPatch with identical file names, no timestamps, FILE_HEADERS_ONLY and context 4. The display diff uses diffLines with old/new line counters, '+' plus new number for additions, '-' plus old number for removals, four unchanged context lines near changes and an ellipsis for omitted gaps; number padding comes from the larger raw LF-split line count. firstChangedLine records the new-side position at the first change, including a deletion. The returned display diff is a presentation format, not an applyable patch. Tests assert a large multi-hunk preview remains visible after settlement without a new full redraw, replay uses result details after the file was removed, and inapplicable edits show an error without plus/minus lines.

### 5. Interfaces

computeEditsDiff(path, edits, cwd):Promise<{diff, patch, firstChangedLine?}|{error}>; generateDiffString(oldText, newText, contextLines=4) and generateUnifiedPatch(path, oldText, newText, contextLines=4). ToolDefinition renderCall/renderResult receive argsComplete, result, state and invalidate hooks. Result detail keys are diff, patch, firstChangedLine. No disk preview artifact or durable cache is written.

### 6. Dependencies

`TOOLS-025`

TUI component lifecycle, requestRender/invalidate and extension renderer hooks require cross-area wiring. Matching and mutation are owned by TOOLS-025/021; the preview must remain advisory.

Final dependency classification. Consuming integrations: `TUI-031`, `TUI-036`. Shared contract references: `EXT-036`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

rust:crates/ice-tools/src/tools/edit_diff.rs:426-600 has display diff and unified patch generation. In the display formatter a Replace operation chooses new_lines as raw, then marks both is_added and is_removed and emits those same new lines for '+' and '-' while advancing both counters by the new length. If the diff engine yields Replace, the old block and later line numbering are wrong; this is a source-only reachable-branch finding, not a runtime reproduction. Its unified patch uses the diff library's proper patch builder, so the two formats can disagree. rust:crates/ice-tools/tests/edit_fuzzy.rs:118-122 only checks new text appears, not old removal text. Rust ToolExecutionComponent stores optional pre-rendered diff, shows 6 lines collapsed/40 expanded, and has a completed-panel revision/theme cache (rust:crates/ice-tui/src/tool_execution.rs:18-26; rust:crates/ice-tui/src/tool_execution.rs:218-225; rust:crates/ice-tui/src/tool_execution.rs:548-616). It has no tool-definition preflight read/key/callback lifecycle in that component. Existing result diffs/rendering must be preserved, not described as wholly missing.

### 8. Port notes

Fix Replace formatting in ice-tools by independently iterating old and new ranges; derive counters from each side. Add preview calculation behind the native tool-definition renderer or TUI host, with stale-key suppression and replay-from-result support. Keep preview outside the mutation lock and clearly let execution revalidate. Preserve Rust's bounded generic panel as a design choice if desired, but provide an accessible complete actual diff. Future tests must assert exact removed text and line numbers for unequal replacement lengths, multi-hunk context and no additional full redraw when identical result details arrive.

### 9. Size and priority

Estimated Rust implementation: **180-310 lines** (estimate, not measured source). **P0** - A displayed edit can misrepresent removed text, and preflight/replay behavior is materially behind the source tool.

## TOOLS-027 - Per-tool call and result renderers with streaming content previews

### 1. Title and status

Per-tool call and result renderers with streaming content previews. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/tools/read.ts:33-209; ice:packages/coding-agent/src/core/tools/write.ts:30-186; ice:packages/coding-agent/src/core/tools/grep.ts:1-128; ice:packages/coding-agent/src/core/tools/find.ts:1-123; ice:packages/coding-agent/src/core/tools/ls.ts:1-100; ice:packages/coding-agent/src/core/tools/render-utils.ts:1-85; ice:packages/coding-agent/src/core/tools/edit.ts:79-292

Named export: `ToolRenderResultLike` (`ice:packages/coding-agent/src/core/tools/render-utils.ts:66`).

### 3. What it does

The terminal gives each file tool a useful compact presentation: read titles explain the resource, writes preview arriving content, and search tools show bounded results that expand on demand.

### 4. How it works

Each rich ToolDefinition supplies renderCall/renderResult functions and reusable state rather than relying only on a generic JSON viewer. These functions format already-authorized tool data; they do not make filesystem mutation decisions.

Read recognizes SKILL.md as '[skill] <parent-directory>', package README/docs/examples as documentation, and AGENTS.md/AGENTS.MD or CLAUDE.md/CLAUDE.MD as resource reads. Ordinary titles show path and requested line range. Relative display paths are used only inside cwd; outside paths remain absolute. Collapsed successful read results hide ordinary text, while errors show up to 10 lines; expansion shows all. Syntax highlighting follows extension, tabs become three spaces and trailing empty lines are trimmed. Nonvision image warning text remains visible. Images receive a text fallback when the terminal lacks image support or showImages is false, including known dimensions.

Write previews argument content as it streams, first 10 logical lines collapsed and all when expanded. State caches path/language/content: append-only updates highlight new lines incrementally, changes to an earlier prefix/path/language rebuild, and the first 50 lines are periodically fully highlighted to stabilize syntax. argsComplete triggers full highlighting. Successful result text is hidden because the call preview already shows the write; errors remain visible. Edit's box/preview is TOOLS-026. Grep previews 15 lines; find and ls preview 20. Expansion shows all and truncation notices stay visible through the tool's result details.

render-utils converts undefined/null strings to empty, rejects non-string path values for meaningful titles, shortens a home prefix for display, resolves relative file links to file URLs when hyperlink capability exists, and sanitizes text by stripping ANSI/control bytes/CR. Home shortening uses a plain prefix check (no path-separator boundary). It does not dereference a hyperlink target. Renderer state lives in the component instance, and no output file/cache is persisted.

### 5. Interfaces

ToolDefinition renderCall(args, theme, context) and renderResult(result,{expanded, isPartial}, theme, context) plus argsComplete/result/state/invalidate context hooks; showImages and terminal capability flags select fallbacks. Helpers are shortenPath, linkPath, str, replaceTabs, normalizeDisplayText, getTextOutput, invalidArgText, renderToolPath. Exact tool schemas remain in their execution entries. No additional CLI/RPC command is declared here.

### 6. Dependencies

`TOOLS-022`, `TOOLS-024`, `TOOLS-026`, `TOOLS-028`, `TOOLS-029`, `TOOLS-030`

TUI toolkit/theme/syntax-highlighter/image capability and extension definition-renderer interfaces require cross-area integration. TOOLS-014 owns Bash rendering. Existing Rust-native panel styling and accessibility should be retained.

Final dependency classification. Consuming integrations: `TUI-031`, `TUI-032`. Shared contract references: `EXT-036`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

rust:crates/ice-tui/src/tool_execution.rs:264-626 already renders status, elapsed time, wrapped known-tool argument summaries, errors,12 visual result rows collapsed, all result rows expanded and 6/40 diff lines. It caches completed output by width/revision/theme, has an explicit skill box and can render markdown (rust:crates/ice-tui/src/tool_execution.rs:197-239; rust:crates/ice-tui/src/tool_execution.rs:628-713). It does not implement these tool-specific read-resource labels/ranges, hidden successful read/write bodies, write argument-content preview/highlighting or per-search preview lengths. Its normalization converts CR to LF and tabs to four spaces (731–742), differing from TS sanitation. Known read/write summaries show only path and edit count (792–846). Existing Rust tests confirm 12-row collapse/expansion and skill rendering (1524–1595); they do not claim rich definition renderer parity.

### 8. Port notes

Add renderer adapters in ice-tui and the CLI definition registry, keeping generic fallback and Rust's theme-aware cache. Pass typed partial arguments/results and a stable per-call renderer state; avoid rereading files just to render output. Preserve capability-aware image fallbacks and safe hyperlinks. Implement append-only write preview updates and explicit invalidation only when content changes; verify wide/narrow rendering and replay with source fixtures during implementation.

### 9. Size and priority

Estimated Rust implementation: **260-430 lines** (estimate, not measured source). **P2** - Execution is usable, but missing per-tool rendering hides useful context and makes long file/search calls harder to inspect.

## TOOLS-028 - Find glob semantics, hierarchical ignore rules and result boundaries

### 1. Title and status

Find glob semantics, hierarchical ignore rules and result boundaries. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/tools/find.ts:1-377 createFindToolDefinition,relativizeFindResultPath; ice:packages/coding-agent/test/tools.test.ts:1-1214; ice:packages/coding-agent/test/suite/regressions/3302-find-path-glob.test.ts:1-81; ice:packages/coding-agent/test/suite/regressions/3303-find-nested-gitignore.test.ts:1-84; ice:packages/coding-agent/test/suite/regressions/6104-find-root-relativization.test.ts:1-92

Named export: `FindToolDetails` (`ice:packages/coding-agent/src/core/tools/find.ts:41`).

Named export: `FindToolOptions` (`ice:packages/coding-agent/src/core/tools/find.ts:63`).

Named export: `createFindTool` (`ice:packages/coding-agent/src/core/tools/find.ts:375`).

### 3. What it does

Find searches file names or directory-containing globs, respects hierarchical ignore rules, and returns paths relative to the requested root with precise result-limit notices.

### 4. How it works

The default tool runs fd directly with a protected argv array, while custom FindOperations can supply the entire glob search. Results are relativized and capped for displayed bytes, with cancellation and one-shot settlement around asynchronous work.

Resolve searchDir||'.' against cwd, use limit??1000 without integer/minimum normalization. With custom glob, require exists(searchPath), check abort, call glob(pattern, searchPath,{ignore:['**/node_modules/**','**/.git/**'], limit}), check abort again, and use the returned array without an additional result-count slice. Empty results return 'No files found matching pattern'. Otherwise relative/absolute paths are normalized with path.relative only when absolute, separators become '/', and one trailing directory separator is retained (including drive roots). Flag resultLimitReached when returned count>=effectiveLimit; truncateHead has maxLines=Number.MAX_SAFE_INTEGER and maxBytes51200.

Default execution awaits ensureTool('fd', true), then checks abort and fails explicitly if fd is unavailable. Args start ['--glob','--color=never','--hidden']. Probe searchPath and all parents for a .git entry; add --no-require-git only outside a repository, preserving fd's nested repository ignore boundaries inside one. Add --max-results with the supplied limit. Strip repeated leading './' from the glob (fall back to original if nothing remains). If it contains '/', add --full-path and prefix '**/' unless already absolute or '**/'-prefixed. Add '--', effectivePattern, searchPath so a pattern starting '-' is not an option.

Read stdout as lines and collect stderr. On abort, kill the child if registered and reject immediately; normal close rejects nonzero status only when there is no output, so useful partial fd output can survive an error. Strip a trailing CR, trim each path line, drop empty lines and relativize. Default limit notices suggest doubling the limit; custom-glob notices omit that suggestion. No automatic execution retry. A source edge remains in TS: abort during asynchronous .git ancestor probes rejects the call before stopChild exists, but there is no final abort check before spawn, so a child can still start after rejection. A port should preserve cancellation intent, not this late-spawn bug.

### 5. Interfaces

Tool find; {pattern:string, path?:string, limit?:number}. FindOperations {exists(path):boolean|Promise<boolean>, glob(pattern, cwd,{ignore:string[], limit:number}):string[]|Promise<string[]>}; options {operations?}. Result details {truncation?:TruncationResult, resultLimitReached?:number}. Default external binary fd may be provisioned by ensureTool; this audit only read that call and did not download or execute it. No settings/RPC methods/files are declared by find.

### 6. Dependencies

`MISC-010`, `TOOLS-019`, `TOOLS-020`

External tool provisioning and distribution policy belong to utilities/CLI. TOOLS-027 owns find rendering. The Rust in-process search design can be retained if its observable matching/ignore semantics are implemented.

### 7. ice-rust today

rust:crates/ice-tools/src/tools/find.rs:1-300 provides an in-process globset matcher and sorted breadth-first FileSystem traversal, with defaults and byte-limited output. It does not read hierarchical .gitignore rules; it only excludes .git directories and skips symlinks. It applies a path glob to relative paths without stripping leading './', and an absolute pattern cannot match those relative candidates. Basename patterns are not explicitly matched against each basename, so a literal basename such as 'target.txt' differs for nested paths. Root resolution uses read spelling fallback instead of TS normal resolution. Rust unsigned limit is forced to at least 1; count-limit notice appears only when another traversal iteration observes the cap, unlike TS's >= count after fd. list_dir errors, including cancellation, are ignored inside traversal, and there is no separate cancellation check for each pure traversal step, permitting false-empty/partial success after cancellation. rust:crates/ice-tools/tests/find.rs:1-183 verifies ordinary globs/limits but not hierarchical ignores, literal nested basenames, './' or the upstream root/path regressions.

### 8. Port notes

Keep FileSystem-based in-process search, but add an ignore-rule stack respecting repository boundaries (e.g. the ignore crate behind an environment-aware adapter), explicit basename versus full-path glob mode and the tested root relativization behavior. Check cancellation at traversal boundaries and distinguish expected inaccessible-entry skips from cancellation. Normalize or deliberately reject unsupported numeric limits at the wire boundary. Future tests should cover sibling/deep ignores, nested repositories, literal basenames, './', absolute patterns, Windows/POSIX roots and abort during setup.

### 9. Size and priority

Estimated Rust implementation: **220-380 lines** (estimate, not measured source). **P1** - Search can return materially different files because ignore and path-glob semantics are missing from the native traversal.

## TOOLS-029 - Grep match stream, context formatting and error/cancellation semantics

### 1. Title and status

Grep match stream, context formatting and error/cancellation semantics. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/tools/grep.ts:1-385 createGrepToolDefinition; ice:packages/coding-agent/test/tools.test.ts:1-1214; ice:packages/coding-agent/src/core/tools/truncate.ts:264-276

Named export: `GrepToolDetails` (`ice:packages/coding-agent/src/core/tools/grep.ts:41`).

Named export: `GrepToolOptions` (`ice:packages/coding-agent/src/core/tools/grep.ts:63`).

Named export: `createGrepTool` (`ice:packages/coding-agent/src/core/tools/grep.ts:383`).

### 3. What it does

Grep returns bounded matches with exact file/line labels and optional surrounding context while respecting ripgrep's ignore and glob behavior.

### 4. How it works

The tool runs rg in JSON mode, collects match events up to a global limit, then formats output after the child exits so context reads may be asynchronous. A custom operations object replaces stat/context reads only; it does not replace local rg search.

Schema fields are pattern, path?, glob?, ignoreCase?, literal?, context?, limit?. context becomes its supplied positive value or 0; limit is Math.max(1, limit??100). Await ensureTool('rg', true), resolve searchDir||'.', and call isDirectory; any stat error becomes 'Path not found: <path>'. Spawn argv ['--json','--line-number','--color=never','--hidden'], optional --ignore-case/--fixed-strings/--glob value, then '--', pattern, searchPath. No shell is involved. Invalid JSON lines are ignored. Every event.type==='match' increments matchCount even if its path/line fields are unusable; store entries only with a truthy path and numeric line. As soon as count>=limit, mark matchLimitReached and kill rg. Result order is rg stream order, not sorted by this tool.

Abort listener is installed only after binary/path setup and spawn. It sets aborted and kills the child, then close rejects; a signal that fires during setup can be missed because there is no intervening recheck. error/close cleanup removes the listener. Exit 0 and 1 are accepted; other codes reject with stderr unless this tool killed due to the result limit. Zero matchCount returns 'No matches found'. Context formatting happens after cleanup, so a later abort during context reads is not observed by this implementation.

With zero context and event text available, normalize CRLF to LF, remove remaining CR and one terminal LF, then clip to 500 UTF-16 units. Otherwise cache full files per path, normalize CRLF and lone CR to LF and split; read failure becomes an empty array and emits '<path>:<line>: (unable to read file)'. Context before/after uses '<path>-N- ', the match uses '<path>:N: '; overlapping windows repeat lines rather than merging. Positive fractional context is not rounded by this code. Directory-relative paths are used only when nonempty and not starting '..'; otherwise basename; backslashes are replaced on the relative display path.

Aggregate displayed output is truncateHead with no practical line cap and 51200-byte cap. Append separate actionable notices for match cap (suggest limit*2), byte cap and 500-character line clipping; details appear only for those conditions. There is no execution retry or persistent search index.

### 5. Interfaces

grep {pattern:string, path?:string, glob?:string, ignoreCase?:boolean, literal?:boolean, context?:number, limit?:number}; GrepOperations {isDirectory(path):boolean|Promise<boolean>, readFile(path):string|Promise<string>}; options {operations?}. Details {truncation?:TruncationResult, matchLimitReached?:number, linesTruncated?:boolean}. External binary rg with --json; its event path.text/lines.text/line_number are consumed. No RPC or persistent setting is introduced here.

### 6. Dependencies

`MISC-010`, `TOOLS-019`, `TOOLS-020`

CLI/utilities own ensureTool provisioning. TOOLS-027 owns grep rendering. Rust's native regex traversal is a useful design to preserve, but it still needs the user-visible ignore/match/error contract.

### 7. ice-rust today

rust:crates/ice-tools/src/tools/grep.rs:1-436 uses regex/globset and FileSystem traversal rather than rg. It has regex/literal/case/context/limit support and bounded formatted output, but no hierarchical ignore processing, rg binary detection or negated-glob semantics. It reads whole files as strict UTF-8 and silently skips read/list failures (including cancellation); TS rg can emit usable events without those whole-file reads. Rust normalizes lone CR into a newline before regex matching and splits a trailing empty line, which can create an extra '^$' match absent from rg's ordinary newline semantics. Path search uses read fallback. Context/limit are unsigned integers; clipping is 500 Unicode scalars instead of JS code units; result detail keys are snake_case. Limit detection occurs on the next iteration rather than immediately when a match reaches the cap, so exact-terminal-match notices differ. Existing tests validate common matching, context separators, clips and limits (rust:crates/ice-tools/tests/grep.rs:1-226) but not ignored files, encoding, terminal empty lines or cancellation swallowing.

### 8. Port notes

Use an environment-aware ignore/regex search engine and preserve direct argv safety if rg remains an optional backend. Separate binary/encoding policy, match line semantics and context hydration so one failed context read cannot silently rewrite match meaning. Add cancellation checks before/after setup, between reads and throughout formatting; treat Aborted distinctly from inaccessible files. Match TS result casing and exact notices where compatibility is desired, while retaining safe scalar clipping as an explicit deviation. Future tests should cover ignored files, glob negation, CR/trailing-LF anchors, invalid UTF-8, exact cap, context overlap/read failure and setup/post-exit cancellation.

### 9. Size and priority

Estimated Rust implementation: **230-400 lines** (estimate, not measured source). **P1** - Search results and cancellation can silently diverge even though basic regex and context functionality exists.

## TOOLS-030 - Directory listing symlinks, collation and per-entry failure handling

### 1. Title and status

Directory listing symlinks, collation and per-entry failure handling. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/tools/ls.ts:1-225 createLsToolDefinition; ice:packages/coding-agent/test/tools.test.ts:1-1214; ice:packages/coding-agent/test/readdir-order.test.ts:1-33

Named export: `LsToolDetails` (`ice:packages/coding-agent/src/core/tools/ls.ts:23`).

Named export: `LsToolOptions` (`ice:packages/coding-agent/src/core/tools/ls.ts:47`).

Named export: `createLsTool` (`ice:packages/coding-agent/src/core/tools/ls.ts:223`).

### 3. What it does

Ls lists dotfiles and directory suffixes in a stable case-insensitive order, tolerates individual unstatable entries and tells the model when entries or bytes were omitted.

### 4. How it works

Resolve path||'.', check existence and directory status, fetch names, sort them, then stat each entry to choose its suffix. Only successfully statted entries count toward the limit.

Default limit is 500 with no minimum/integer coercion. exists false produces 'Path not found: ...'; a non-directory root produces 'Not a directory: ...'; readdir failures become 'Cannot read directory: <message>'. Default stat follows symlinks both for the root and entries. Sort with a.toLowerCase().localeCompare(b.toLowerCase()); skip any per-entry stat exception. A directory gets '/', other supported stat results keep the name. At each next entry, if results.length>=limit, set entryLimitReached and break. If there are exactly limit entries and no next entry, there is no limit notice. limit 0 returns '(empty directory)' with no details, even when names existed.

After removing the abort listener, empty results return '(empty directory)'; otherwise join with LF, truncateHead at 51200 bytes with maxLines=Number.MAX_SAFE_INTEGER, and append entry/byte-cap notices. Entry notices suggest doubling the limit. The abort listener rejects immediately but does not stop already-running filesystem operations; early path/not-directory/readdir error returns do not remove that listener, while normal completion and the outer catch do. No retry, persistent directory cache or per-entry authorization is implemented.

### 5. Interfaces

ls {path?:string, limit?:number}; LsOperations {exists(path):boolean|Promise<boolean>, stat(path):{isDirectory():boolean}|Promise<...>, readdir(path):string[]|Promise<string[]>}; options {operations?}; details {truncation?:TruncationResult, entryLimitReached?:number}. Dotfiles are included; suffix '/' is display data, not a returned structured type. No external binary, setting or RPC extension is required.

### 6. Dependencies

`AGENT-043`, `AGENT-044`, `TOOLS-019`, `TOOLS-020`

Directory enumeration/FileInfo semantics overlap host filesystem entries, including existing AGENT-044; coordinator should link the exact accepted ID. TOOLS-027 owns terminal listing preview.

### 7. ice-rust today

rust:crates/ice-tools/src/tools/ls.rs:1-226 already lists and sorts entries, marks directories and truncates output. Rust uses read-path fallback, unsigned limit forced to at least 1, and lowercase Unicode codepoint ordering rather than localeCompare. OsExecutionEnv.file_info/list_dir use symlink_metadata, so a symlink root is not recognized as a directory and symlink-to-directory entries lack '/'; TS stat follows them. The Rust list_dir resolves all metadata and propagates ordinary per-entry stat failures instead of skipping them (rust:crates/ice-tools/src/os_env.rs:534-632), which can fail the entire listing. Rust detail fields are snake_case. Tests read verify ordinary ASCII sort, suffixes, empty directory, limits and root errors (rust:crates/ice-tools/tests/ls.rs:1-152); they do not cover symlink targets or individual stat failures.

### 8. Port notes

Keep the in-process FileSystem listing, but offer follow-target metadata or a tool-level policy for root/entry symlinks and per-entry failure skips. Choose and document locale-independent ordering if exact localeCompare parity is not portable; do not claim equivalence outside the tested ASCII subset. Preserve typed cancellation errors and clean up listeners/tasks on every exit. Add future fixtures for dangling/symlink directories, a disappearing entry, zero/fractional limit validation and non-ASCII ordering.

### 9. Size and priority

Estimated Rust implementation: **90-170 lines** (estimate, not measured source). **P1** - A normal symlinked directory or one inaccessible entry can produce a different listing or a whole-tool failure.

## TOOLS-031 - Stdout isolation, queued UTF-8 byte limits and transient sink retries

### 1. Title and status

Stdout isolation, queued UTF-8 byte limits and transient sink retries. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/output-guard.ts:1-147 takeOverStdout,restoreStdout,writeRawStdout,configureRawStdoutLimit,waitForRawStdoutBackpressure,flushRawStdout; ice:packages/coding-agent/test/output-guard.test.ts:1-174

Named export: `isStdoutTakenOver` (`ice:packages/coding-agent/src/core/output-guard.ts:110`).

### 3. What it does

Machine-readable output stays on its original stdout stream while incidental stdout writes are redirected to stderr. Explicit raw output is ordered, byte-budgeted and retried for transient sink pressure.

### 4. How it works

takeOverStdout is idempotent and records the original stdout/stderr write methods before redirecting ordinary process.stdout.write to stderr. Raw output bypasses that redirection through a single serialized promise tail, with byte reservations made synchronously before queuing.

configureRawStdoutLimit(undefined) disables the limit; otherwise require a nonnegative safe integer or throw RangeError without changing the prior value. Empty raw writes are ignored. Compute UTF-8 bytes for each text string; if that one write exceeds the configured limit, synchronously throw RawStdoutLimitError with code output_too_large. If it exceeds remaining capacity after queued+in-flight reservations, throw output_overflow. Rejecting a write reserves no bytes. Reserve before scheduling; release only in finally after the sink callback succeeds/fails, including queued operations skipped when an earlier tail promise rejected.

writeRawStdoutChunk wraps the chosen write callback. On ENOBUFS, EAGAIN, EWOULDBLOCK, retry the entire text after 10 ms, keeping its reservation, indefinitely; any other callback/throw error rejects. There is no retry count, cancellation or timeout. A permanent failure poisons the tail, later queued writes are skipped, and each attached failure handler calls process.exit(1). writeRawStdout itself returns void. waitForRawStdoutBackpressure repeatedly awaits the current tail until no newer write was appended; flushRawStdout drains then performs an empty sink write callback as a flush barrier.

restoreStdout disables the limit and restores the original ordinary writer even without a prior takeover. It deliberately retains queued byte reservations and the promise tail across restore/new takeover, because in-flight operations still own their reservations. The sink writer is looked up when a queued write executes. Redirection honors callback position but converts a non-string chunk with toString and does not preserve the supplied encoding semantics. This is process-local state, not a persistent log or RPC protocol.

### 5. Interfaces

takeOverStdout(), restoreStdout(), writeRawStdout(text:string):void, configureRawStdoutLimit(maxBytes?:number), getRawStdoutQueuedBytes():number, waitForRawStdoutBackpressure():Promise<void>, flushRawStdout():Promise<void>; RawStdoutLimitError.name='RawStdoutLimitError', code='output_too_large'|'output_overflow'. Default byte budget is unlimited; callers select the cap. No setting/env/CLI flag is declared by this module. RPC/print consumers choose takeover and output policies.

### 6. Dependencies

RPC and print-mode lifecycle must call the guard and translate/handle limit failures; coordinator must resolve the RPC entry ID. This entry owns stream/byte accounting and retries only, not command schemas or response/event ordering.

Final dependency classification. Consuming integrations: `RPC-017`, `MISC-015`, `MISC-016`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

rust:crates/ice-cli/src/modes.rs:602-635 has an actual bounded RpcWriter: channel capacity 64 Vec<u8> messages, one spawn_blocking writer, write_all+flush per line and a failure cancellation token. Shutdown joins producers and the writer and returns exit 1 on failure (570–599). This is useful ordering/backpressure, but it does not cap individual or aggregate UTF-8 bytes, expose output_too_large/output_overflow, retry WouldBlock-like errors after 10 ms, or redirect incidental process stdout to stderr. Each line is materialized before awaiting queue capacity. Print mode uses direct stdout.write_all/flush and discards I/O errors (117–142). Tests read establish message-count backpressure, EOF drain, completion-after-final-event and broken-sink shutdown (rust:crates/ice-cli/src/modes.rs:1121-1300), not the TS byte reservation contract. No application output was executed during the audit.

### 8. Port notes

Extend a reusable Rust output writer with weighted byte reservations and an explicit per-write cap while retaining the dedicated blocking sink and ordered queue. Provide controlled raw-output handles rather than globally monkey-patching Rust stdio; route host incidental logs to stderr and document limitations for native plugin writes. Translate transient error kinds carefully and retry only under a stated delivery guarantee to avoid duplicating partially written data. Preserve failure cancellation and producer joins. Make print-mode sink failures visible. Future tests should exercise multibyte exact budgets, pending reservations across reconfiguration, retries, poisoned queues and flush barriers using an injected writer.

### 9. Size and priority

Estimated Rust implementation: **160-280 lines** (estimate, not measured source). **P1** - RPC is message-count bounded, but a burst or single large write has no TypeScript-equivalent byte budget and print failures can be hidden.

