# Observability: installation telemetry, startup timings, durable run history, live dashboard and Cognee knowledge

This area specifies the eight observability ledger files and their runtime wiring. Findings distinguish the live in-memory projection, the redacted durable projection, replay reconstruction, HTTP/SSE transport and browser rendering. TypeScript behavior is based on the frozen dirty-worktree baseline. Rust comparisons retain their original ff9270f evidence and are reconciled to active checkout 1891337 through the coordinator's reviewed source mapping. Final acceptance is recorded in the coordinator's validation artifacts.

## OBS-001 - Install telemetry gate and best-effort version ping

### 1. Title and status

Install telemetry gate and best-effort version ping. **Status: missing.**

### 2. ice source

`ice:packages/coding-agent/src/core/telemetry.ts:1-14` exports `isInstallTelemetryEnabled`; `ice:packages/coding-agent/src/core/legacy-compat/env.ts:13-15` defines its environment lookup. The setting declaration/accessors are `ice:packages/coding-agent/src/core/settings-manager.ts:152-154` and `ice:packages/coding-agent/src/core/settings-manager.ts:1272-1280`. Startup detection and the request are `ice:packages/coding-agent/src/modes/interactive/interactive-mode.ts:1382-1429`. Attribution consumers and override order: `ice:packages/coding-agent/src/core/provider-attribution.ts:36-103`. Behavioral tests: `ice:packages/coding-agent/test/sdk-openrouter-attribution.test.ts:145-275`.

### 3. What it does

Controls the optional install/update ping and default provider attribution headers. It does not control the always-on local observability database, and disabling it is not an update-check switch.

### 4. How it works

An explicitly defined `ICE_TELEMETRY` value overrides the saved setting; otherwise install telemetry defaults to enabled. The interactive startup path sends a version ping only after detecting a first install or new changelog entries, and treats every request outcome as nonfatal.

`isTruthyEnvFlag` accepts exactly `1`, or `true`/`yes` after lowercasing; it does not trim. An empty string, `0`, `false`, or any other defined value disables this gate. `getIceEnv` reads only an own property of the supplied environment: despite the compatibility module's name, this baseline performs no legacy-prefix lookup. `getEnableInstallTelemetry` returns the effective value or `true`; its setter updates global settings, marks that key modified, and saves.

`getChangelogForDisplay` first skips sessions already containing messages. With no prior changelog version it records `VERSION`, invokes telemetry, and returns no changelog. When new entries exist it records `VERSION`, invokes telemetry, and returns normalized new-entry content. `reportInstallTelemetry` returns immediately for any truthy `process.env.ICE_OFFLINE`, then applies the telemetry gate. It starts a fetch to `https://ice.dev/api/report-install?version=` plus `encodeURIComponent(version)`, with the version-derived Ice User-Agent and a 5,000 ms AbortSignal deadline. Both resolution and rejection become `undefined`; there is no retry, response-body processing, or status-code-dependent action (`ice:packages/coding-agent/src/modes/interactive/interactive-mode.ts:1382-1429`).

The same gate suppresses only default attribution headers in `getDefaultAttributionHeaders`. Session routing headers are assembled before it and remain separate; later provider/request header sources can override defaults (`ice:packages/coding-agent/src/core/provider-attribution.ts:36-103`). The provider integration area owns the complete provider-to-header mapping.

### 5. Interfaces

`isInstallTelemetryEnabled(settingsManager: SettingsManager, telemetryEnv: string | undefined = getIceEnv("ICE_TELEMETRY")): boolean`. Setting: `enableInstallTelemetry?: boolean`, effective default `true`. Environment: `ICE_TELEMETRY` with the exact truth table above; the ping also checks `ICE_OFFLINE` by JavaScript truthiness. Request: GET `https://ice.dev/api/report-install?version=<URL-encoded version>`, `User-Agent: getIceUserAgent(version)`, timeout 5,000 ms. No model tool, slash command, local database schema, or RPC event originates in this helper.

### 6. Dependencies

`MISC-013`, `SET-001`, `SET-018`

Settings persistence and changelog/version identity are cross-area dependencies to resolve with settings and CLI packaging. Optional external endpoint: ice.dev. Provider attribution uses the same gate, but session transport headers have separate semantics.

Final dependency classification. Shared contract references: `CLI-018`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

Rust's `Settings` model has no `enableInstallTelemetry` field (`rust:crates/ice-cli/src/settings.rs:82-238`). Its known settings model therefore does not retain that boolean as an effective field. The startup path builds the harness and dispatches modes without the TypeScript report-install step (`rust:crates/ice-cli/src/app.rs:174-521`). `ProductionTelemetryContext` is a local span writer, with explicit stdout/file/disabled constructors, not this anonymous install/version protocol (`rust:crates/ice-telemetry/src/production.rs:19-115`). Repository searches for the endpoint, gate and attribution identifiers found no implementation; that search is supplemental to the inspected settings and startup control flow.

### 8. Port notes

If this behavior is desired, add an explicit privacy setting and a small CLI-owned best-effort request using the existing reqwest dependency. Preserve the separate responsibilities of update checking, attribution, local run recording and install reporting. Mirror the exact override/default/offline decisions and do not make network success a startup prerequisite. Resolve the service/branding choice before shipping a Rust report-install endpoint; no endpoint was contacted during this audit. AUTH-023 owns attribution-header consumers of this shared gate.

### 9. Size and priority

Estimated Rust implementation: **80-180 lines** (estimate, not measured source). **P3** - Compatibility and privacy-setting fidelity; not required for agent execution.

## OBS-002 - Startup timing namespaces and emission timing

### 1. Title and status

Startup timing namespaces and emission timing. **Status: partial.**

### 2. ice source

`ice:packages/coding-agent/src/core/timings.ts:1-52` exports `resetTimings`, `time` and `printTimings`. Normal CLI emission sites are `ice:packages/coding-agent/src/main.ts:948-999`. Environment lookup: `ice:packages/coding-agent/src/core/legacy-compat/env.ts:13-15`. Rust's behavior tests are inline at `rust:crates/ice-cli/src/timings.rs:111-143`; they were read, not run.

### 3. What it does

Opt-in startup diagnostics print labelled elapsed intervals grouped into main and extension namespaces. Rust already has this capability, but its clock, namespace ordering and CLI flush point differ.

### 4. How it works

TypeScript records elapsed wall-clock milliseconds only when `ICE_TIMING` equals `1` at module evaluation. It prints populated groups to stderr when the caller requests a startup report, without clearing the recorded intervals.

The module holds a Map from `main` or `extensions` to `{ timings: Array<{ label, ms }>, lastTime }`. Reset replaces one group with an empty array and `Date.now()`. `time` samples `Date.now()` first, lazily resets an absent group, subtracts its lastTime, appends the interval, and replaces lastTime. Because sampling precedes a lazy reset, the first interval can be negative if the millisecond changes; wall-clock rollback can also produce negative intervals. Printing suppresses negative intervals while accumulating the displayed total, visits Map insertion order, writes the namespace title, each `label: Nms`, the total and a divider of title length plus eight (`ice:packages/coding-agent/src/core/timings.ts:8-52`).

The main CLI calls `printTimings()` before entering RPC, interactive or print execution, with a separate startup-benchmark path after TUI initialization (`ice:packages/coding-agent/src/main.ts:948-999`). This placement is part of observability behavior: users see a startup report while a long session is still running.

### 5. Interfaces

Environment `ICE_TIMING=1`; all other values disable collection. TypeScript namespace union is `"main" | "extensions"`, with default `main`. Public functions are synchronous and return void; labels are unconstrained strings. Output goes to stderr and uses millisecond units. No persistent timing file, RPC payload, tool schema or independent telemetry service is involved.

### 6. Dependencies

CLI startup and extension loader instrumentation provide the labels. Standard wall clock and stderr are the only external facilities.

### 7. ice-rust today

`rust:crates/ice-cli/src/timings.rs:1-109` implements namespace groups behind a Mutex, caches an exact `ICERS_TIMING=1` decision in OnceLock on first use, and uses monotonic `Instant`. Its first new-group interval is 0 ms, and print order is always main then extensions. These are concrete differences from TypeScript's import-time gate, Date.now intervals and insertion ordering. The Rust CLI wraps the entire `run_inner().await` and prints after it returns, rather than before entering a long-lived run (`rust:crates/ice-cli/src/app.rs:68-79`). Both implementations preserve the general labelled stderr report. Neither baseline environment helper implements a legacy prefix alias.

### 8. Port notes

Retain Rust's monotonic clock and synchronized storage unless strict wall-clock quirks are required. Move or add a startup flush at the equivalent completed-startup boundary in ice-cli if users need the report before session shutdown. Decide whether first-use gate evaluation and fixed group ordering are intentional; document them if retained. No new crate is needed.

### 9. Size and priority

Estimated Rust implementation: **20-70 lines** (estimate, not measured source). **P2** - The current flush point can hide startup diagnostics until an interactive session exits.

## OBS-003 - Always-on durable recording and opt-in dashboard startup

### 1. Title and status

Always-on durable recording and opt-in dashboard startup. **Status: missing.**

### 2. ice source

`ice:packages/coding-agent/src/main.ts:906-946` constructs the recorder/store and optional server; `ice:packages/coding-agent/src/main.ts:1000-1007` closes them. `ice:packages/coding-agent/src/cli/args.ts:27-28` and `ice:packages/coding-agent/src/cli/args.ts:91-101` define the flag. Asset resolution: `ice:packages/coding-agent/src/config.ts:478-491`. Operational contract: `ice:packages/coding-agent/docs/observability.md:1-64`. Startup-independent recorder exit behavior is tested at `ice:packages/coding-agent/test/observe-recorder.test.ts:90-109`.

### 3. What it does

Every normal agent run records a local forensic projection even when no browser server is requested. `--observe` exposes that same recorder and history through a loopback dashboard, with the URL shown through the active output mode.

### 4. How it works

The CLI first opens `observability.sqlite` under its resolved agent directory and attaches one recorder to the runtime session. A dashboard requested with `--observe` reuses those objects, so enabling the browser does not create a second stream of projected events.

The recorder also subscribes to the in-process Cognee observation stream, wrapping each observation as `{ type: "cognee_observation", observation }`. Store/recorder initialization is in a try/catch: failure prints `Warning: observability history unavailable: <message>` and allows the run to continue. A requested dashboard receives the explicit port or default, existing recorder/store, the persisted `observe.token`, and a lazy Cognee source bound to the session cwd. Its notice includes `(shared existing port)` when appropriate. Interactive mode receives the notice as a startup notice because it clears the terminal; other modes write it to stderr. Dashboard startup failure stops the recorder and propagates the error. The outer finally closes the server if present, otherwise stops the recorder (`ice:packages/coding-agent/src/main.ts:906-946`, `ice:packages/coding-agent/src/main.ts:1000-1007`).

`--observe` alone becomes boolean true; `--observe=<text>` converts with Number and accepts only an integer greater than 0 and at most 65,535. The space-separated form does not consume a following port in this parser. Invalid equals-form values produce the exact diagnostic `--observe requires a valid port number` (`ice:packages/coding-agent/src/cli/args.ts:91-101`).

### 5. Interfaces

CLI field `observe?: number | true`; flag `--observe[=port]`, default TCP port 4649 supplied by the server. Paths under the resolved TypeScript agent directory: `observability.sqlite` and `observe.token`. Cognee source constructor: `createObserveCogneeSource({ cwd: sessionCwd })`. Bundle asset: Bun uses `<packageDir>/observe/observe-dashboard.html`; source/dist mode chooses `src` if that directory exists, otherwise `dist`, followed by `/observe/observe-dashboard.html` (`ice:packages/coding-agent/src/config.ts:478-491`). No telemetry-opt-out gate surrounds local recording.

### 6. Dependencies

`COG-060`, `OBS-004`, `OBS-011`, `OBS-014`, `OBS-017`, `SET-001`

Requires normal session event subscription and the Cognee observation publisher. SQLite and HTTP transport details are specified in the referenced entries; agent-directory resolution belongs to settings/CLI packaging.

### 7. ice-rust today

The inspected Rust startup constructs the harness and passes event_rx to mode handlers, but has no parallel forensic recorder, observability SQLite opening, or dashboard startup (`rust:crates/ice-cli/src/app.rs:304-521`). Its RPC branch runs JSONL stdio or extension-owned server lifecycle, which is a different interface (`rust:crates/ice-cli/src/app.rs:434-503`). The existing canonical SQLite repository uses one serialized session state per row, not observe runs/events (`rust:crates/ice-harness/src/session/sqlite.rs:175-218`).

### 8. Port notes

Add CLI-owned observation wiring around the authoritative Rust event stream; do not create another agent loop. Keep a separate projection store and make store failure nonfatal. Reuse one recorder across optional HTTP/SSE service and shutdown paths. Prefer an embedded immutable dashboard asset or an explicitly packaged asset with equivalent resolution; keep Rust-side config/runtime identity distinct from TypeScript. The later relocation records active icers paths.

### 9. Size and priority

Estimated Rust implementation: **140-280 lines** (estimate, not measured source). **P1** - Provides the lifecycle integration required for durable debugging and every dashboard capability.

## OBS-004 - Recorder identity, event contracts and bounded live state

### 1. Title and status

Recorder identity, event contracts and bounded live state. **Status: missing.**

### 2. ice source

DTOs and public interfaces: `ice:packages/coding-agent/src/observe/observe-recorder.ts:18-188`. Recorder creation/emission/thread setup: `ice:packages/coding-agent/src/observe/observe-recorder.ts:324-394`. Item insertion and eviction: `ice:packages/coding-agent/src/observe/observe-recorder.ts:534-562`. Header polling, subscriptions and shutdown: `ice:packages/coding-agent/src/observe/observe-recorder.ts:779-841`. Buffer/stop tests: `ice:packages/coding-agent/test/observe-recorder.test.ts:334-357`.

Named export: `startObserveRecorder` (`ice:packages/coding-agent/src/observe/observe-recorder.ts:324`).

### 3. What it does

Maintains one bounded timeline for a running session with stable run/item/thread identities. Subscribers receive new activities, patches, thread/header updates and compatibility lane events without altering canonical session history.

### 4. How it works

Starting a recorder creates a new run ID from the session ID, current epoch milliseconds and a random UUID, then builds an empty live ring and a main-thread summary. All event consumers share this projection; persistence is optional and is isolated from live delivery failures.

The ring retains at most 500 activity items, with monotonically increasing item IDs starting at 1. Each push attaches the selected thread identity, applies the supplied activity fields, then assigns its final incremented ID and current timestamp, appends the object, drops excess oldest objects, and emits an activity. The payload helper limits each retained string to 1,048,576 JavaScript UTF-16 code units and adds an omitted-count suffix; this is not a total event-byte budget. Main-thread name is trimmed sessionName or `main · ` plus the first ten session-ID code units, with blue color. Child threads are keyed by `<mainThreadId>::<childId>` and cycle violet/cyan/green/amber/rose/orange/indigo; an existing key returns its earlier thread summary. The compatibility lanes Map is included in snapshots but this implementation does not create lane records from normal session events (`ice:packages/coding-agent/src/observe/observe-recorder.ts:225-245`, `ice:packages/coding-agent/src/observe/observe-recorder.ts:324-394`, `ice:packages/coding-agent/src/observe/observe-recorder.ts:534-554`).

On emit, requested persistence is tried before live listeners; a persistence exception is swallowed. Header persistence uses serialized-header equality to suppress unchanged header writes. Listener callbacks themselves are called without individual error isolation. Initial persistence records the run and main-thread event. Session, optional session-manager, and extra-source subscriptions are then installed. A 1,000 ms default poll emits headers; `pollMs: 0` disables it. A process exit handler finishes the run if persistence exists. Stop removes that handler, unsubscribes sources, clears the poll and pending memory requests, attempts finishRun plus close, and clears listeners. Snapshot arrays are copied but their item objects are shared references; tests observe a start item's mutation through terminal updates (`ice:packages/coding-agent/src/observe/observe-recorder.ts:338-354`, `ice:packages/coding-agent/src/observe/observe-recorder.ts:779-841`, `ice:packages/coding-agent/test/observe-recorder.test.ts:46-87`).

### 5. Interfaces

Exact source DTOs and host/sink contracts (`ice:packages/coding-agent/src/observe/observe-recorder.ts:29-188`):
```ts
export type ObserveItemKind =
	| "message"
	| "tool"
	| "subagent"
	| "compaction"
	| "retry"
	| "skill"
	| "queue"
	| "memory"
	| "goal";
export type ObserveThreadKind = "main" | "subagent";
export type ObserveThreadColor =
	| "blue"
	| "violet"
	| "cyan"
	| "green"
	| "amber"
	| "rose"
	| "orange"
	| "indigo"
	| "slate";

export interface ObserveThreadIdentity {
	/** Stable path: mainThreadId for the root, parentThreadKey::childThreadId below it. */
	threadKey: string;
	/** Raw session/thread identifier for this node. */
	threadId: string;
	/** Immediate parent raw thread identifier, absent only for a root main agent. */
	parentThreadId?: string;
	/** Root main-agent raw thread identifier. */
	mainThreadId: string;
	threadKind: ObserveThreadKind;
	threadName: string;
	threadColor: ObserveThreadColor;
}

export interface ObserveThread extends ObserveThreadIdentity {
	runId?: string;
	role?: string;
	profileColor?: string;
}

export interface ObserveItem extends ObserveThreadIdentity {
	runId: string;
	id: number;
	ts: number;
	kind: ObserveItemKind;
	/** Original session event or projected subagent lifecycle event. */
	eventType?: string;
	role?: "user" | "assistant" | "custom";
	/** Complete message text, skill invocation detail, or event detail. */
	text?: string;
	/** Complete serialized source payload for forensic inspection. */
	raw?: string;
	toolCallId?: string;
	toolName?: string;
	state?: "running" | "done" | "error";
	/** Complete JSON.stringify of tool call arguments. */
	args?: string;
	/** Complete text result or compaction summary. */
	result?: string;
	durationMs?: number;
	/** Number of partial/streaming updates seen for a tool call. */
	updates?: number;
	/** Parent observatory tool when this is a child activity. */
	delegationToolName?: string;
}

export interface ObserveLane extends ObserveThreadIdentity {
	streamKey: string;
	runId?: string;
	role?: string;
	toolName: string;
	status: string;
	phase: string;
	terminal: boolean;
	startedAtMs: number;
	elapsedMs: number;
	currentTool?: string;
	currentPath?: string;
	model?: string;
	attempt?: number;
	eventType?: string;
	summary?: string;
}

export interface ObserveHeader {
	runId: string;
	sessionId: string;
	mainThreadId: string;
	mainThreadName: string;
	sessionName?: string;
	model: string | null;
	thinkingLevel: string | null;
	context: ContextUsage | undefined;
	usage: { input: number; output: number; cacheRead: number; cacheWrite: number; cost: number };
	startedAt: number;
}

export interface ObserveState extends ObserveHeader {
	items: ObserveItem[];
	lanes: ObserveLane[];
	threads: ObserveThread[];
}

export type ObserveRecorderEvent =
	| { type: "activity"; item: ObserveItem }
	| { type: "activity_update"; item: ObserveItem }
	| { type: "lane"; lane: ObserveLane }
	| { type: "thread"; thread: ObserveThread }
	| { type: "header" };

export interface ObserveRecorder {
	getState(): ObserveState;
	getHeader(): ObserveHeader;
	subscribe(listener: (event: ObserveRecorderEvent) => void): () => void;
	stop(): void;
}

/** Durable sink for the recorder's projected run history. Implementations must be synchronous. */
export interface ObservePersistence {
	recordRun(header: ObserveHeader): void;
	recordEvent(event: ObserveRecorderEvent, header: ObserveHeader): void;
	finishRun(runId: string, endedAt: number): void;
	close(): void;
}

export interface ObserveRecorderOptions {
	/** Poll interval for header state in ms (default 1000; 0 disables polling). */
	pollMs?: number;
	/** Optional durable sink. Persistence failures are isolated from the agent loop. */
	persistence?: ObservePersistence;
	/** Extra in-process event streams projected alongside the session, such as Cognee memory activity. */
	sources?: ObserveEventSource[];
}

export interface ObserveEventSource {
	subscribe(listener: (event: unknown) => void): () => void;
}

/** Session-event shape for one Cognee observation fed through an {@link ObserveEventSource}. */
export interface CogneeObserveEvent {
	type: "cognee_observation";
	observation: CogneeObservation;
}

/**
 * The session surface the recorder depends on. AgentSession satisfies this structurally,
 * which keeps the recorder unit-testable with a fake event emitter.
 */
export interface ObserveSessionLike {
	subscribe(listener: (event: unknown) => void): () => void;
	sessionId: string;
	sessionName?: string;
	readonly model: { provider: string; id: string } | null | undefined;
	readonly thinkingLevel: string;
	getContextUsage(): ContextUsage | undefined;
	/** Session tree notifications; custom entries carry extension, goal, verifier, and subagent state. */
	readonly sessionManager?: { subscribe(listener: (event: unknown) => void): () => void };
}
```
`ContextUsage` is `{ tokens: number | null; contextWindow: number; percent: number | null }` (`ice:packages/coding-agent/src/core/extensions/types.ts:315-321`). Options default pollMs to 1000 ms; 0 disables polling. Each source subscription returns an unsubscribe callback. Lane records remain part of the compatibility shape even though normal recorder events do not populate them.

### 6. Dependencies

`SESSION-022`, `SESSION-027`, `SESSION-034`

Authoritative session and custom-entry streams are supplied by the session/harness area. Random UUID and wall-clock milliseconds are platform facilities; no network service is required.

### 7. ice-rust today

Rust mode handlers already receive the harness event receiver (`rust:crates/ice-cli/src/app.rs:304-312`, `rust:crates/ice-cli/src/app.rs:393-503`), but the separate bounded observation DTOs, run identity and recorder fan-out are absent from this flow. Rust's span trait offers start_span and an RAII terminal callback, not activity patching, per-run snapshots or thread projection (`rust:crates/ice-telemetry/src/context.rs:16-35`). Existing session state and remote transcript types must be adapted as inputs, not substituted for this contract.

### 8. Port notes

Implement a projection component in ice-harness or a small observation module owned by ice-cli. Use typed enum payloads and a VecDeque/ordered map to bound retained activity; maintain stable item identity for updates and a separate persistence sink. Convert Rust event delivery into the documented semantic events. Decide and document whether to preserve shared-object and listener-exception quirks; Rust should keep projection failures from disrupting the agent. No external crate beyond existing serde/uuid and synchronization facilities is essential.

### 9. Size and priority

Estimated Rust implementation: **280-480 lines** (estimate, not measured source). **P1** - Defines the stable data contract on which replay, diagnostics and the dashboard depend.

## OBS-005 - User, skill and assistant-message projection with usage accumulation

### 1. Title and status

User, skill and assistant-message projection with usage accumulation. **Status: missing.**

### 2. ice source

`ice:packages/coding-agent/src/observe/observe-recorder.ts:194-207`, `ice:packages/coding-agent/src/observe/observe-recorder.ts:249-264`, `ice:packages/coding-agent/src/observe/observe-recorder.ts:598-653`, `ice:packages/coding-agent/src/observe/observe-recorder.ts:779-793`; tests at `ice:packages/coding-agent/test/observe-recorder.test.ts:207-244` and `ice:packages/coding-agent/test/observe-recorder.test.ts:497-527`.

### 3. What it does

Shows submitted user text, skill invocations, completed assistant text, and empty aborted/error turns as readable activities. Separately totals provider-reported input/output/cache usage and cost for the run header.

### 4. How it works

User message starts become either a normal message row or a skill row when the skill-envelope parser recognizes an invocation. Assistant message ends update usage even when no visible text is retained, and failures receive an explicit activity instead of disappearing.

The user branch accepts role `user`, extracts content when it is a string or joins text blocks with newlines, then runs `parseSkillBlock`. A recognized block becomes kind `skill` with text `/<skillName>` followed by a space and userMessage only when userMessage is nonempty; otherwise kind `message`, role `user`, with the extracted text. The raw source message is safely serialized and bounded. Activity timestamps come from pushItem's current wall clock, not from the original message timestamp (`ice:packages/coding-agent/src/observe/observe-recorder.ts:249-264`, `ice:packages/coding-agent/src/observe/observe-recorder.ts:598-622`).

On assistant message_end the recorder increments input, output, cacheRead, cacheWrite and total cost from the reported usage. It extracts only text blocks for visible response text. Nonempty trimmed text becomes a message activity; an `aborted` or `error` stop reason also creates a row when text is empty, prefixing the visible text with the bracketed stop reason and optional errorMessage, joined with a newline when text also exists. Error/aborted rows have state `error`; successful ordinary messages do not need a state chip. The raw message remains available and the header is emitted after accumulation (`ice:packages/coding-agent/src/observe/observe-recorder.ts:623-653`). Header model is provider/id when a model exists, thinking level is the session's value or null, and context is obtained fresh from getContextUsage (`ice:packages/coding-agent/src/observe/observe-recorder.ts:779-793`).

### 5. Interfaces

Inputs: message_start containing a user message; message_end containing an assistant message with content, stopReason, optional errorMessage and usage. Outputs: ObserveItem kinds `message` or `skill`; roles `user` or `assistant`; eventType preserves the source event. Usage shape is `{ input, output, cacheRead, cacheWrite, cost }`, where cost accumulates `message.usage.cost.total`. No additional server request or canonical session entry is created.

### 6. Dependencies

`AGENT-001`, `OBS-004`

Consumes the normal session event stream through the recorder. Referenced entries define shared DTOs and any external service boundary.

### 7. ice-rust today

The Rust CLI passes harness events directly into print/JSON/interactive/RPC modes (`rust:crates/ice-cli/src/app.rs:393-503`), and its telemetry contract records named span lifetimes (`rust:crates/ice-telemetry/src/context.rs:16-35`). The separate ObserveItem projection, retained run timeline and activity/activity_update contract described here are not implemented by either inspected component. Canonical session persistence is a different layer (`rust:crates/ice-harness/src/session/sqlite.rs:44-81`). This is a missing forensic projection, not a claim that Rust cannot execute messages or tools.

### 8. Port notes

Add a Rust event-to-observation adapter beside the harness stream. Reuse the existing skill-envelope parser contract from the resource/CLI area and the canonical provider usage structures, while preserving this projection's timing, filtering and empty-failure behavior. Keep rendered text separate from bounded raw payloads.

### 9. Size and priority

Estimated Rust implementation: **130-240 lines** (estimate, not measured source). **P1** - Ensures failures, skill requests and usage remain visible in run diagnostics.

## OBS-006 - One-row tool lifecycle, transient progress and forensic result storage

### 1. Title and status

One-row tool lifecycle, transient progress and forensic result storage. **Status: missing.**

### 2. ice source

`ice:packages/coding-agent/src/observe/observe-recorder.ts:232-300`, `ice:packages/coding-agent/src/observe/observe-recorder.ts:534-562`, `ice:packages/coding-agent/src/observe/observe-recorder.ts:654-712`; tests `ice:packages/coding-agent/test/observe-recorder.test.ts:46-87`, `ice:packages/coding-agent/test/observe-recorder.test.ts:111-205`.

### 3. What it does

Tracks each main-thread tool call as one activity that transitions from running to done/error. Retains bounded complete arguments and final text/details for the inspector while counting streaming updates without filling durable history with every tick.

### 4. How it works

A tool start allocates a running item; progress and completion search backward for the newest retained item whose threadKind is main with the same toolCallId and mutate it in place. Streaming progress is broadcast but deliberately not persisted, whereas the final update is persisted.

Start stores tool name, ID, JSON-serialized arguments (null when absent), state running and updates 0. A progress event increments updates and emits activity_update with persistence disabled; partialResult is not stored. Completion computes text from text content blocks joined with newlines, or a JSON fallback when the result has no supported text-content shape. It places non-text content blocks and other result metadata in raw while excluding duplicate text blocks; a result object containing only text content blocks therefore has no redundant raw copy. If a retained matching item exists, it receives terminal state, final event type, result, optional raw and elapsed duration clamped at zero, followed by activity_update. If no start is found, including ring eviction, it creates a terminal activity with duration 0. A header follows completion (`ice:packages/coding-agent/src/observe/observe-recorder.ts:266-300`, `ice:packages/coding-agent/src/observe/observe-recorder.ts:556-562`, `ice:packages/coding-agent/src/observe/observe-recorder.ts:654-712`).

The 1,048,576 UTF-16-code-unit string ceiling applies independently to serialized args/result/raw. Safe JSON formatting uses two-space indentation and catches serialization errors by falling back to String(value); truncation can make a retained serialized string no longer parseable as JSON. The dashboard handles parse failure rather than assuming every retained raw/args value parses (`ice:packages/coding-agent/src/observe/observe-recorder.ts:232-245`).

The fallback in resultText uses ordinary JSON.stringify outside the safe serializer's catch. A cyclic nonstandard result can therefore throw before emission; normal typed text-content results avoid that path. Progress with no matching retained start is ignored, whereas an unmatched completion is retained.

### 5. Interfaces

Input names: `tool_execution_start`, `tool_execution_update`, `tool_execution_end`. Correlation key: `toolCallId` constrained to a retained item whose `threadKind` is `"main"`. Tool fields: `toolName`, `args`, `partialResult`, `result`, `isError`. Projected fields: kind `tool`, state running/done/error, args/result/raw strings, updates number, durationMs number. Output events: activity for start/fallback terminal; activity_update for retained progress/completion; header after end.

### 6. Dependencies

`AGENT-001`, `OBS-004`

Consumes the normal session event stream through the recorder. Referenced entries define shared DTOs and any external service boundary.

### 7. ice-rust today

The Rust CLI passes harness events directly into print/JSON/interactive/RPC modes (`rust:crates/ice-cli/src/app.rs:393-503`), and its telemetry contract records named span lifetimes (`rust:crates/ice-telemetry/src/context.rs:16-35`). The separate ObserveItem projection, retained run timeline and activity/activity_update contract described here are not implemented by either inspected component. Canonical session persistence is a different layer (`rust:crates/ice-harness/src/session/sqlite.rs:44-81`). This is a missing forensic projection, not a claim that Rust cannot execute messages or tools.

### 8. Port notes

Implement stable retained item IDs with mutation/patch messages, and keep live delivery independent from durable persistence policy. Reuse existing Rust tool result types but explicitly split text from image/details metadata. Preserve partial-update counting, reverse correlation and eviction fallback; do not describe progress replay as exhaustive.

### 9. Size and priority

Estimated Rust implementation: **170-300 lines** (estimate, not measured source). **P1** - Tool timing, arguments, errors and final outputs are central to diagnosing execution reliability.

## OBS-007 - Compaction, retry and queue activity projection

### 1. Title and status

Compaction, retry and queue activity projection. **Status: missing.**

### 2. ice source

`ice:packages/coding-agent/src/observe/observe-recorder.ts:702-773`; behavior tests `ice:packages/coding-agent/test/observe-recorder.test.ts:246-273`. Common payload bounding: `ice:packages/coding-agent/src/observe/observe-recorder.ts:232-245`.

### 3. What it does

Shows compaction start/end, automatic retry attempts and message-queue changes on the same activity tape. Keeps full bounded compaction summaries and event metadata for later forensic inspection.

### 4. How it works

Lifecycle events become separate timeline items rather than one mutable lifecycle row. Compaction completion keeps the summary as result text and serializes the selected lifecycle fields, so the reader can inspect retained-entry and token metadata.

Compaction start records its reason. Compaction end distinguishes aborted, retrying, error, and successful outcomes in its description and retains result.summary when present; raw includes reason, result, aborted, willRetry and errorMessage. Retry start text records attempt/maxAttempts and wait delay, while raw retains the error message and other event fields; retry end records success or final error and attempt. Queue updates summarize steering and follow-up counts and serialize the event. Every retained string is subject to the common code-unit cap. These are projections of existing events, not new compaction/retry/queue control operations (`ice:packages/coding-agent/src/observe/observe-recorder.ts:702-773`).

The compaction test verifies a summary exceeding 5,000 characters survives and that raw includes firstKeptEntryId; it also verifies distinct start/end activities for compaction and retry (`ice:packages/coding-agent/test/observe-recorder.test.ts:246-273`). No test execution is claimed.

### 5. Interfaces

Inputs: `compaction_start` (reason); `compaction_end` (reason, result, aborted, willRetry, errorMessage); `auto_retry_start` (attempt, maxAttempts, delayMs, errorMessage); `auto_retry_end` (success, attempt, finalError); `queue_update` (steering and followUp arrays). Outputs: ObserveItem kind compaction/retry/queue, source eventType, bounded text/raw and optional compaction result. Exact texts: compaction start `started (<reason>)`; end `finished (<reason>)` followed by comma-separated `aborted`, `will retry`, and `error: <message>` when applicable; retry start `attempt <attempt>/<maxAttempts> in <delayMs>ms`; retry end `attempt <attempt> succeeded` or `attempt <attempt> failed: <finalError>`; queue `<steering length> steering, <followUp length> follow-up`. Source formatting intentionally has no space before `ms`; audit prose uses explicit spaced units.

### 6. Dependencies

`OBS-004`, `SESSION-013`, `SESSION-015`, `SESSION-018`, `SESSION-020`, `SESSION-022`

Consumes the normal session event stream through the recorder. Referenced entries define shared DTOs and any external service boundary.

### 7. ice-rust today

The Rust CLI passes harness events directly into print/JSON/interactive/RPC modes (`rust:crates/ice-cli/src/app.rs:393-503`), and its telemetry contract records named span lifetimes (`rust:crates/ice-telemetry/src/context.rs:16-35`). The separate ObserveItem projection, retained run timeline and activity/activity_update contract described here are not implemented by either inspected component. Canonical session persistence is a different layer (`rust:crates/ice-harness/src/session/sqlite.rs:44-81`). This is a missing forensic projection, not a claim that Rust cannot execute messages or tools.

### 8. Port notes

Map existing Rust harness lifecycle events into these observation variants after the session area settles its event names. Preserve start/end ordering and the full bounded summary/details. A queue adapter may need to project queue snapshots where Rust currently exposes only counts; do not synthesize successful lifecycle outcomes.

### 9. Size and priority

Estimated Rust implementation: **110-200 lines** (estimate, not measured source). **P2** - Makes retry and compaction behavior diagnosable without changing their execution policies.

## OBS-008 - Cognee observation correlation and recall evidence

### 1. Title and status

Cognee observation correlation and recall evidence. **Status: missing.**

### 2. ice source

`ice:packages/coding-agent/src/observe/observe-recorder.ts:169-173`, `ice:packages/coding-agent/src/observe/observe-recorder.ts:302-316`, `ice:packages/coding-agent/src/observe/observe-recorder.ts:564-596`, `ice:packages/coding-agent/src/observe/observe-recorder.ts:773-777`; startup source bridge `ice:packages/coding-agent/src/main.ts:912-919`. Tests: `ice:packages/coding-agent/test/observe-recorder.test.ts:275-332` and `ice:packages/coding-agent/test/ice-cognee-observer.test.ts:43-64`.

### 3. What it does

Combines a Cognee request's start and terminal observation into one memory row with latency, dataset/model identity and result evidence. The original request preview survives completion, enabling recall replay from the dashboard.

### 4. How it works

A nonterminal memory observation is kept in a pending map by requestId; a succeeded/failed observation patches that item when the matching request is still pending. Uncorrelated observations remain separate rows, so older producers without request IDs still display activity.

The title starts with `Cognee <operation>`, adds `(compaction)` only when observation.meta.checkpoint === true, then appends dataset and model when present. State maps failed to error, succeeded to done, and every nonterminal phase to running. The request preview becomes args. Result text combines an error, serialized metadata key/value summaries and the latest preview as available; raw serializes the observation, and a new item's duration is present only when latencyMs is supplied. For a correlated terminal update duration uses latencyMs when non-nullish, otherwise elapsed wall-clock time clamped to zero. A terminal matching request updates title/state/result/raw/duration but deliberately preserves the earlier args preview; it emits activity_update and deletes the pending key. Otherwise a new memory activity is appended, and a nonterminal request with ID is added to the pending map. That map is capped at 500 by deleting the oldest insertion-order key. A terminal observation after eviction therefore creates a new row. Stop clears pending correlation (`ice:packages/coding-agent/src/observe/observe-recorder.ts:302-316`, `ice:packages/coding-agent/src/observe/observe-recorder.ts:564-596`).

The startup bridge subscribes to already-published Cognee observations. Their producer's redaction is specified by COG-059/060; it must not be confused with general live message/tool redaction. The dedicated test proves a redacted Cognee preview reaches in-process subscribers, while normal recorder live content remains intact (`ice:packages/coding-agent/test/ice-cognee-observer.test.ts:43-64`, `ice:packages/coding-agent/test/observe-store.test.ts:64-83`).

Pending memory references are independent of the live ring: an intervening burst can evict an item from the ring while its request remains pending. A later terminal event patches and emits that object without reinserting it into the recorder ring; the browser's unknown-update handling can still show it. Nonterminal rows set args only when a preview is truthy. Metadata result text preserves Object.entries order as `key=String(value)` entries joined by comma-space, with error and preview on separate lines.

### 5. Interfaces

Extra-source envelope: `{ type: "cognee_observation"; observation: CogneeObservation }`. Important observation fields: id, requestId?, at, dataset?, operation, phase, model?, preview?, error?, latencyMs?, meta?. Output kind is `memory`, toolName is the operation, args retains the initial preview, and raw carries observation metadata used by Knowledge replay. Producer schema and its redaction/rotation rules are COG-059 and COG-060.

### 6. Dependencies

`COG-059`, `COG-060`, `OBS-004`

Consumes the normal session event stream through the recorder. Referenced entries define shared DTOs and any external service boundary.

### 7. ice-rust today

The Rust CLI passes harness events directly into print/JSON/interactive/RPC modes (`rust:crates/ice-cli/src/app.rs:393-503`), and its telemetry contract records named span lifetimes (`rust:crates/ice-telemetry/src/context.rs:16-35`). The separate ObserveItem projection, retained run timeline and activity/activity_update contract described here are not implemented by either inspected component. Canonical session persistence is a different layer (`rust:crates/ice-harness/src/session/sqlite.rs:44-81`). This is a missing forensic projection, not a claim that Rust cannot execute messages or tools.

### 8. Port notes

Subscribe to the Rust Cognee observation producer once that port exists, and maintain a bounded request correlation map separate from the activity ring. Preserve the initial recall query across terminal updates and keep producer redaction distinct from persistence redaction. No extra Cognee HTTP operation is needed for this projection.

### 9. Size and priority

Estimated Rust implementation: **110-190 lines** (estimate, not measured source). **P2** - Explains memory latency/failures and supplies the query evidence needed for recall replay.

## OBS-009 - Goal and verifier transition projection with exact deduplication

### 1. Title and status

Goal and verifier transition projection with exact deduplication. **Status: missing.**

### 2. ice source

`ice:packages/coding-agent/src/observe/observe-recorder.ts:194-230`, `ice:packages/coding-agent/src/observe/observe-recorder.ts:369-373`, `ice:packages/coding-agent/src/observe/observe-recorder.ts:396-468`, `ice:packages/coding-agent/src/observe/observe-recorder.ts:496-515`. Tests: `ice:packages/coding-agent/test/observe-recorder.test.ts:378-438` and `ice:packages/coding-agent/test/observe-recorder.test.ts:440-480`.

### 3. What it does

Displays meaningful goal and Guarded Build verifier state changes while suppressing repeated state rewrites that do not change the displayed transition summary.

### 4. How it works

Appended custom session entries carrying goal or verifier state are projected into goal activities. Deduplication compares exact summary strings, so repeated usage-only goal writes and output-only verifier rewrites do not create another row.

The dispatcher requires an object event with type entry_appended, object entry, string customType, type custom and object data. For ice-goals-state it filters data.goals to object records, stringifies each ID, and constructs a summary from status (default unknown), strict approved===true, recursively counted complete/total tasks, runsUsed/maxRuns (0/? defaults), reviewAttempts/3 (0 default), the first 12 code units of criteriaHash, and pauseReason. It stores this string under `goal:<id>`. Only a changed string emits a row; the objective becomes result, and the goal object becomes raw. State is done for complete, running for active, error for blocked/budget_limited, otherwise absent. IDs no longer present remove their transition key and emit `goal cleared · <id>`. An absent/non-array goals field is an empty set and clears prior goals (`ice:packages/coding-agent/src/observe/observe-recorder.ts:396-439`).

For ice-safe-verify-state, data.verifier must be an object. Summary is `verifier <status>` plus `exit <finite exitCode>` if available and `mode <String(data.mode)>`, separated by middle dots. The singleton key `verifier` deduplicates this summary. Output joins nonblank string failureMessage, stdout and stderr with newlines. Raw is the verifier object; finite durationMs is retained. Status passed/failed/running maps to done/error/running, with other states unset (`ice:packages/coding-agent/src/observe/observe-recorder.ts:441-468`). The source has no read of existing session state at recorder creation: these transitions arise from subsequent appended entries.

### 5. Interfaces

Consumed custom types: `ice-goals-state` and `ice-safe-verify-state`. Produced kind `goal`, toolName `goal` or `verify`, eventType equal to the custom type, text/result/raw/duration/state as above. Numeric helpers accept finite numbers only; string helpers accept strings only. Goal summary rendering uses a fixed review denominator 3 and criteria-hash prefix length 12. `countTasks` traverses `data.goals[].tasks` and each record's `subtasks` recursively. Only arrays are traversed and only non-null, non-array objects count; each counted node increments total, and only `status === "complete"` increments complete (`ice:packages/coding-agent/src/observe/observe-recorder.ts:209-222`). No command is issued to approve goals or launch verification.

### 6. Dependencies

`OBS-004`, `SAFE-003`, `SAFE-021`

Consumes recorder/session integration; the upstream state producers remain separately specified.

Final dependency classification. Shared contract references: `SAFE-022`, `SAFE-023`, `SAFE-024`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

The Rust CLI passes harness events directly into print/JSON/interactive/RPC modes (`rust:crates/ice-cli/src/app.rs:393-503`), and its telemetry contract records named span lifetimes (`rust:crates/ice-telemetry/src/context.rs:16-35`). The separate ObserveItem projection, retained run timeline and activity/activity_update contract described here are not implemented by either inspected component. Canonical session persistence is a different layer (`rust:crates/ice-harness/src/session/sqlite.rs:44-81`). This is a missing forensic projection, not a claim that Rust cannot execute messages or tools.

### 8. Port notes

Add observers for the corresponding Rust custom-entry/state signals after the safe-verify/goals port is available. Preserve exact deduplication keys and which fields participate; do not emit on every durable rewrite. Keep the observer unable to mutate approval, task or verifier state. Add producer dependency IDs during cross-area reconciliation.

### 9. Size and priority

Estimated Rust implementation: **160-280 lines** (estimate, not measured source). **P1** - Makes approval, blocked-goal and failed-verification outcomes visible without duplicate activity noise.

## OBS-010 - Subagent threads, completion rows and extension custom messages

### 1. Title and status

Subagent threads, completion rows and extension custom messages. **Status: missing.**

### 2. ice source

`ice:packages/coding-agent/src/observe/observe-recorder.ts:194-207`, `ice:packages/coding-agent/src/observe/observe-recorder.ts:318-320`, `ice:packages/coding-agent/src/observe/observe-recorder.ts:375-394`, `ice:packages/coding-agent/src/observe/observe-recorder.ts:470-531`. Tests: `ice:packages/coding-agent/test/observe-recorder.test.ts:440-529`.

### 3. What it does

Places Subagents V2 lifecycle and completion activity on child threads linked to the main session. Also makes extension custom messages visible, including context-only messages whose display flag is false.

### 4. How it works

A child lifecycle record emits activity only when state, last-turn outcome or patch state changes its deduplication summary. Completion records are separate activities and are not deduplicated; extension custom messages are projected independently of their display flag.

For ice-subagent-v2-child, the summary joins string state (default unknown), optional lastTurnOutcome and `patch <patchState>`. It is compared under `child:<String(record.id)>`. Any defined outcome other than completed marks the item error even when the current state is running; blocked also marks error. Otherwise running/starting maps to running and everything else to done. Text appends description after an em dash when present; raw stores the record and toolName is agent or `agent`. childThread resolves id or childId, uses `<mainThreadKey>::<childId>`, records parent/root identities, uses name or a ten-code-unit ID prefix, allocates the next child color, and optionally stores the agent as role. Existing thread keys retain the earlier name/role/color (`ice:packages/coding-agent/src/observe/observe-recorder.ts:375-394`, `ice:packages/coding-agent/src/observe/observe-recorder.ts:470-494`).

An ice-subagent-v2-completion custom entry maps exact outcome completed to done and every other outcome to error. It uses childId and agent to resolve the thread, text `completion <outcome or unknown>`, bounded string summary as result, and full serialized data as raw. A completion arriving before a child record can therefore establish a thread with the fallback name. Custom_message entries require a string customType; a customType beginning `ice-goals` is kind goal, all others kind message. They have role custom, toolName customType, extracted content text and raw containing customType/display/details. The display flag is observed, not enforced as a visibility gate (`ice:packages/coding-agent/src/observe/observe-recorder.ts:496-531`). Other custom types are ignored.

### 5. Interfaces

Inputs: session-manager `entry_appended`; `entry.type` custom or custom_message; customType values `ice-subagent-v2-child`, `ice-subagent-v2-completion`, and any custom_message customType. Child correlation key `child:<id>`; thread key `<mainThreadKey>::<childId>`. Output: thread events for first child discovery; activity rows with threadKind subagent and parent/root identity; custom messages have role custom. Unrecognized custom data produces no observation.

### 6. Dependencies

`OBS-004`, `SUB-016`, `SUB-017`, `SUB-036`

Consumes recorder/session integration; the upstream state producers remain separately specified.

### 7. ice-rust today

The Rust CLI passes harness events directly into print/JSON/interactive/RPC modes (`rust:crates/ice-cli/src/app.rs:393-503`), and its telemetry contract records named span lifetimes (`rust:crates/ice-telemetry/src/context.rs:16-35`). The separate ObserveItem projection, retained run timeline and activity/activity_update contract described here are not implemented by either inspected component. Canonical session persistence is a different layer (`rust:crates/ice-harness/src/session/sqlite.rs:44-81`). This is a missing forensic projection, not a claim that Rust cannot execute messages or tools.

### 8. Port notes

Bridge the authoritative subagent lifecycle and extension custom-entry APIs into the recorder. Do not infer child identity from tool names or flatten all children into the main thread. Preserve first-discovery thread identity and the distinction between deduplicated lifecycle records and every completion record. Resolve Subagents V2 producer entry IDs before publication.

### 9. Size and priority

Estimated Rust implementation: **140-250 lines** (estimate, not measured source). **P1** - Required to attribute delegated work and terminal outcomes to the correct child.

## OBS-011 - SQLite run/event schema and synchronous append semantics

### 1. Title and status

SQLite run/event schema and synchronous append semantics. **Status: missing.**

### 2. ice source

`ice:packages/coding-agent/src/observe/observe-store.ts:1-188`, `ice:packages/coding-agent/src/observe/observe-store.ts:300-330`; recorder sink interface `ice:packages/coding-agent/src/observe/observe-recorder.ts:148-163`. Persistence/reopen test: `ice:packages/coding-agent/test/observe-store.test.ts:36-62`.

### 3. What it does

Persists a forensic run index and ordered projected event stream in a dedicated SQLite database. Recording is separate from canonical session storage and supports replay after the agent has stopped.

### 4. How it works

Opening a store creates a dedicated schema and returns a synchronous persistence sink. Every recorded event upserts run metadata, inserts one JSON payload under a per-run sequence, then advances the in-memory next-sequence counter only after a successful insert.

The schema has observe_runs keyed by run_id, with session_id, main_thread_id, optional session_name/model/thinking_level, start/end epoch milliseconds and running/finished status. observe_events references run_id, stores sequence, event timestamp, event_type and payload_json, and has primary key (run_id, sequence). Indexes support session runs ordered by start and per-run timestamps/sequences (`ice:packages/coding-agent/src/observe/observe-store.ts:106-132`).

recordRun inserts running status on first creation; on conflict it updates only session_name, model and thinking_level. It does not reset original start/session/root IDs, end time or finished status. It initializes this store instance's next sequence to 1 when absent, without consulting persisted MAX(sequence). Normal recorder UUID run IDs avoid reuse, but reopening an existing run for new writes can collide with existing sequence keys; independent writers are not given an allocation transaction. recordEvent replaces the bare header signal with `{type:"header",header}`; activity and activity_update rows use the item's original ts, while other event types use current Date.now. JSON serialization applies the redaction replacer described in OBS-012. finishRun is idempotent after close and otherwise sets ended_at_ms and status finished. Other methods reject use after close with `Observe run store is closed`; close sets its closed flag and closes the database once (`ice:packages/coding-agent/src/observe/observe-store.ts:144-188`, `ice:packages/coding-agent/src/observe/observe-store.ts:300-309`).

### 5. Interfaces

Exact DDL (`ice:packages/coding-agent/src/observe/observe-store.ts:108-130`):
```sql
CREATE TABLE IF NOT EXISTS observe_runs (
	run_id TEXT PRIMARY KEY,
	session_id TEXT NOT NULL,
	main_thread_id TEXT NOT NULL,
	session_name TEXT,
	model TEXT,
	thinking_level TEXT,
	started_at_ms INTEGER NOT NULL,
	ended_at_ms INTEGER,
	status TEXT NOT NULL CHECK (status IN ('running', 'finished'))
);
CREATE INDEX IF NOT EXISTS observe_runs_session_idx
	ON observe_runs (session_id, started_at_ms DESC);
CREATE TABLE IF NOT EXISTS observe_events (
	run_id TEXT NOT NULL REFERENCES observe_runs(run_id) ON DELETE CASCADE,
	sequence INTEGER NOT NULL,
	ts_ms INTEGER NOT NULL,
	event_type TEXT NOT NULL,
	payload_json TEXT NOT NULL,
	PRIMARY KEY (run_id, sequence)
);
CREATE INDEX IF NOT EXISTS observe_events_run_ts_idx
	ON observe_events (run_id, ts_ms, sequence);
```
Sink methods: `recordRun(header)`, `recordEvent(event, header)`, `finishRun(runId, endedAt)`, `close()`, all synchronous. Stored payload is an ObserveRecorderEvent except that a header event embeds the full ObserveHeader. Sequence is an integer starting at 1 for a new recorder/store mapping. The event_type field has no enum CHECK; run status has the running/finished CHECK.

### 6. Dependencies

`OBS-004`

Node implementation uses @zykairotis/ice-storage-sqlite-node and the SQLite factory API. Rust already has rusqlite; the canonical session repository must remain separate.

### 7. ice-rust today

Rust `SqliteSessionRepo` creates a sessions table with id, created_at, parent_session_id and serialized state_json (`rust:crates/ice-harness/src/session/sqlite.rs:175-218`). Each canonical mutation refreshes/reduces/persists that snapshot (`rust:crates/ice-harness/src/session/sqlite.rs:44-173`). It does not create observe_runs/observe_events, store recorder payloads or maintain replay sequences. SQLite technology is present; this independent schema and writer behavior are missing.

### 8. Port notes

Implement a dedicated rusqlite observation store, reusing the dependency already declared at `rust:Cargo.toml:62-63`. Match schema and ordering for compatibility, or specify a migration/version plan explicitly. Keep one run writer and do not mistake the TS in-memory sequence map for a safe multiprocess allocator. If resuming writes to existing run IDs is supported, a stronger sequence allocation policy is required and should be documented as a deliberate improvement.

### 9. Size and priority

Estimated Rust implementation: **220-360 lines** (estimate, not measured source). **P1** - Provides durable evidence and stable ordering after process shutdown.

## OBS-012 - Durable redaction, retention pruning and SQLite open policy

### 1. Title and status

Durable redaction, retention pruning and SQLite open policy. **Status: missing.**

### 2. ice source

`ice:packages/coding-agent/src/observe/observe-store.ts:19-32`, `ice:packages/coding-agent/src/observe/observe-store.ts:168-180`, `ice:packages/coding-agent/src/observe/observe-store.ts:311-330`; text redactor `ice:packages/coding-agent/src/utils/redact.ts:1-8`; nonfatal startup `ice:packages/coding-agent/src/main.ts:906-924`. Tests: `ice:packages/coding-agent/test/observe-store.test.ts:64-84`, `ice:packages/coding-agent/test/observe-store.test.ts:104-130`, `ice:packages/coding-agent/test/observe-store.test.ts:132-149`.

### 3. What it does

Redacts credential-shaped strings before durable event JSON reaches disk and prunes runs older than 30 days when opening the store. Keeps normal live content unchanged, while making persistence failures nonfatal to the agent.

### 4. How it works

The store redacts individual string values during JSON serialization so it does not corrupt the outer JSON structure. It applies age retention during opening and configures SQLite for durable writes and bounded busy waits.

The sensitive-key expression is `/^(?:api[_-]?key|(?:access[_-]?|refresh[_-]?)?token|secret|password|authorization)$/i`. For any string value whose property key matches, the replacer returns `[REDACTED]`; non-string values pass through unchanged. Other strings are processed by redactCredentialText, which replaces Bearer credentials and api_key/token/secret/password/authorization assignment-shaped text. Because tool args/raw are already strings, their interiors are treated as text, not recursively parsed JSON; the exact regex behavior is narrower than a guarantee that arbitrary secrets are removed. Redaction applies to payload_json writes, not the live in-memory item, SSE data, or every run-index metadata column (`ice:packages/coding-agent/src/observe/observe-store.ts:20-29`, `ice:packages/coding-agent/src/observe/observe-store.ts:144-180`, `ice:packages/coding-agent/src/utils/redact.ts:1-8`).

openObserveRunStore creates the parent directory, opens the supplied database through an injected factory or createNodeSqliteFactory, then sets journal_mode=WAL, journal_size_limit=67,108,864, synchronous=FULL and busy_timeout=5,000. Its 30-day constant is 30*24*60*60*1,000 ms. After schema creation it deletes events for runs whose started_at_ms is older than Date.now minus retention, then deletes those runs; the two statements are not wrapped in one explicit transaction. The code does not enable foreign_keys, so the explicit child-event delete is material. Long-lived runs are aged by start time, not last event or end time. Pruning occurs only on opening; no periodic sweeper is installed. Startup failure emits a warning and leaves the agent running (`ice:packages/coding-agent/src/observe/observe-store.ts:311-330`, `ice:packages/coding-agent/src/main.ts:906-924`).

### 5. Interfaces

Retention constant: `OBSERVE_RETENTION_MS = 30 * 24 * 60 * 60 * 1000`. Open function accepts databasePath and optional SQLite factory. PRAGMAs: WAL, journal_size_limit 67108864 bytes, synchronous FULL, busy_timeout 5000 ms. JSON replacement string is `[REDACTED]`. The live-versus-durable boundary is explicitly asserted by `ice:packages/coding-agent/test/observe-store.test.ts:64-83`; no new privacy guarantee is inferred from that test.

### 6. Dependencies

`OBS-011`

Uses the credential-text helper and the SQLite storage factory; no external service is involved. Local path selection is OBS-003.

### 7. ice-rust today

The canonical Rust SQLite opener sets WAL and creates its session schema (`rust:crates/ice-harness/src/session/sqlite.rs:196-212`), but it does not implement this forensic payload redaction, 30-day run/event pruning, observation-specific busy timeout or journal limit. Rust's local telemetry writer serializes provided attributes into span JSON (`rust:crates/ice-telemetry/src/production.rs:69-115`); that is not equivalent to the specified redaction boundary.

### 8. Port notes

Keep redaction at a structured serialization boundary and write a clear data-retention contract. Existing rusqlite can apply the documented PRAGMAs. Decide whether to mirror the TS start-time pruning and nontransactional deletion or improve them with a documented migration; do not overwrite canonical session history. Preserve the live/durable distinction in UI and APIs so callers understand which content can persist.

### 9. Size and priority

Estimated Rust implementation: **130-240 lines** (estimate, not measured source). **P1** - Defines the privacy and retention requirements for any durable observation port.

## OBS-013 - Historical run queries and ordered replay reconstruction

### 1. Title and status

Historical run queries and ordered replay reconstruction. **Status: missing.**

### 2. ice source

Store API and query implementation: `ice:packages/coding-agent/src/observe/observe-store.ts:34-103`, `ice:packages/coding-agent/src/observe/observe-store.ts:190-298`. Tests: `ice:packages/coding-agent/test/observe-store.test.ts:36-62`, `ice:packages/coding-agent/test/observe-store.test.ts:86-130`.

Named export: `ObserveRunStore` (`ice:packages/coding-agent/src/observe/observe-store.ts:53`).

### 3. What it does

Lists recorded runs, retrieves ordered event pages, and reconstructs a past run's state after its process has stopped.

### 4. How it works

Replay is an ordered fold of persisted events, with separate APIs for listing runs and paging events. A full state reconstruction pages through the entire run, while an explicitly limited reconstruction intentionally folds only its first page.

listRuns orders by started_at_ms descending. listLiveRuns requires status running and at least one stored event with ts_ms >= activeSince, also ordered by start; this is recent-event evidence, not a process-liveness check. getRun selects a run ID; getLatestRunForSession chooses the greatest start time for that session. Optional limits default to 100 when undefined/nonfinite, otherwise floor and clamp into 1..10000. getEvents accepts an after cursor only if it is a nonnegative safe integer; all other values become 0. It queries sequence > after in ascending sequence order, JSON-parses each payload without a per-row recovery catch, and returns sequence, ts, type and payload. getLatestSequence returns MAX or 0 (`ice:packages/coding-agent/src/observe/observe-store.ts:83-103`, `ice:packages/coding-agent/src/observe/observe-store.ts:190-247`).

getState returns undefined for a missing run. Initial header uses run metadata, sessionName || `main · ` + sessionId.slice(0,10), undefined context and all-zero usage. Header events replace the header. Activity and activity_update both set a Map entry keyed by numeric item.id, supplying the requested runId when the payload runId is falsy. Lane keys are streamKey and thread keys threadKey. Map replacement preserves first insertion order; returned items are not timestamp-sorted here. Without options.limit it repeatedly requests 10000 events after the last sequence until an empty page; with any explicit limit it calls getEvents once and stops. It neither caps the final item Map to 500 nor interprets the limit as a latest-item window (`ice:packages/coding-agent/src/observe/observe-store.ts:249-298`).

### 5. Interfaces

`ObserveRunSummary = {runId:string; sessionId:string; mainThreadId:string; sessionName?:string; model:string|null; thinkingLevel:string|null; startedAt:number; endedAt?:number; status:"running"|"finished"}`. `ObserveReplayEvent = {sequence:number; ts:number; type:ObserveRecorderEvent["type"]; payload:ObserveRecorderEvent|{type:"header";header:ObserveHeader}}`. Read methods are listRuns(limit?), listLiveRuns(activeSince, limit?), getRun(runId), getLatestRunForSession(sessionId), getEvents(runId,{after?, limit?}?), getLatestSequence(runId), getState(runId,{limit?}?). Numbers use JS number semantics; timestamps are epoch milliseconds. The distinction between persisted original activity timestamps and sequence order is intentional.

### 6. Dependencies

`OBS-004`, `OBS-011`, `OBS-012`

Uses the dedicated observation SQLite store, not canonical session replay. No network service is required.

### 7. ice-rust today

Rust `SqliteSessionRepo` stores canonical session snapshots in `sessions.state_json` and delegates mutations through refresh/reduce/persist (`rust:crates/ice-harness/src/session/sqlite.rs:44-218`). It has no observation run/event schema or equivalent read projection. Existing SQL storage must be retained; this additional forensic API is missing.

### 8. Port notes

Implement typed run/replay responses and sequence-ordered folding on the observation store. Preserve the distinction between event pagination, full reconstruction and any newer tail-window API. Avoid carrying the TS earliest-page stale-state behavior into a method named latest state without documenting a deliberate contract change. Reject corrupt stored payloads with a clear error rather than silently dropping evidence; exact error-policy parity remains a product decision.

### 9. Size and priority

Estimated Rust implementation: **180-320 lines** (estimate, not measured source). **P1** - Allows reliable postmortem inspection of completed or crashed runs.

## OBS-014 - Loopback dashboard authentication, token file and HTML asset loading

### 1. Title and status

Loopback dashboard authentication, token file and HTML asset loading. **Status: missing.**

### 2. ice source

`ice:packages/coding-agent/src/observe/observe-server.ts:32-130`, `ice:packages/coding-agent/src/observe/observe-server.ts:422-447`, `ice:packages/coding-agent/src/observe/observe-server.ts:550-597`; asset path `ice:packages/coding-agent/src/config.ts:484-491`. Authentication tests: `ice:packages/coding-agent/test/observe-server.test.ts:87-181`.

Named export: `OBSERVE_DEFAULT_PORT` (`ice:packages/coding-agent/src/observe/observe-server.ts:33`).

Named export: `ObserveServer` (`ice:packages/coding-agent/src/observe/observe-server.ts:62`).

Named export: `startObserveServer` (`ice:packages/coding-agent/src/observe/observe-server.ts:264`).

### 3. What it does

Protects local observation data with a per-user access token and serves the bundled dashboard over loopback.

### 4. How it works

Every request first passes a loopback Host check and then presents a matching token through the printed URL, Bearer header or cookie. The HTML route exchanges a valid query token for an HttpOnly cookie so the dashboard's own requests can authenticate without adding credentials to each URL.

loadObserveToken reads and trims an existing token, accepts any nonempty value, and suppresses only ENOENT. Otherwise it creates 32 random bytes encoded as hex, creates parent directories recursively with requested mode 0700, and writes token plus newline using exclusive wx and mode 0600. If another writer won with EEXIST, it reads and trims that file. Existing permissions/format are not revalidated or repaired; an already existing empty file can reach the EEXIST branch and yield empty content. The server itself accepts an injected token or creates a per-process random token. Equality rejects absent tokens, converts both to byte buffers, requires equal byte length and calls timingSafeEqual (`ice:packages/coding-agent/src/observe/observe-server.ts:89-124`, `ice:packages/coding-agent/src/observe/observe-server.ts:264-270`).

Allowed Host values match /^(localhost|127\.0\.0\.1|\[::1\])(?::\d+)?$/i. Missing/disallowed Host returns 403 plain text forbidden before authentication. Bearer parsing is /^Bearer\s+(.+)$/i; cookies split semicolon components and rejoin value pieces at '=' without URL decoding. Any matching channel authenticates. Failed authentication returns 401 with the printed-URL instruction. '/' and every pathname beginning '/index' serve HTML, regardless of HTTP method, with content-type text/html;charset=utf-8 and Referrer-Policy:no-referrer. Successful query auth adds `ice_observe_token=<token>; HttpOnly; SameSite=Strict; Path=/`. The cookie name and query URL contract are verified by tests; source credential-like assignments were redacted by the connector and were not recovered. No Secure cookie flag or token expiry is specified (`ice:packages/coding-agent/src/observe/observe-server.ts:422-447`, `ice:packages/coding-agent/test/observe-server.test.ts:125-159`).

The HTML string is lazily read once and cached for the process. Bun resolves getPackageDir()/observe/observe-dashboard.html. Other builds select src when the packageDir/src directory exists, otherwise dist, then resolve observe/observe-dashboard.html under that directory; this choice does not test whether the individual asset exists. Listening is specifically 127.0.0.1, default port 4649; the API can request port 0 and returns the actual bound port (`ice:packages/coding-agent/src/config.ts:484-491`, `ice:packages/coding-agent/src/observe/observe-server.ts:72-77`, `ice:packages/coding-agent/src/observe/observe-server.ts:550-597`).

### 5. Interfaces

Public `loadObserveToken(path): string`. `ObserveServerOptions` has optional port, recorder, replayStore, token and cognee. Result has url, dashboardUrl, port, optional shared and async close(). Token transport: query `token`, Authorization Bearer, cookie `ice_observe_token`. Normal url is http://127.0.0.1:<actual port>; dashboardUrl carries the token query. The on-disk token path is selected at OBS-003.

### 6. Dependencies

`OBS-003`

Requires OS randomness, private filesystem creation, constant-time comparison and an HTTP listener. No external service is needed.

### 7. ice-rust today

Rust mode routing reaches print, JSON, interactive and RPC (`rust:crates/ice-cli/src/app.rs:393-503`). Its remote module defines native protocol types, a TCP JSONL client and optional four-byte length-prefixed CBOR transport (`rust:crates/ice-cli/src/remote/mod.rs:1-34`). Those facilities do not implement the read-only observation HTTP/SSE routes or their Host/token contract. The existing span API (`rust:crates/ice-telemetry/src/context.rs:16-35`) also lacks them. Relevant source-tree inventory and behavior searches are recorded in observability-read-progress.json and the independent QA evidence.

### 8. Port notes

Use a loopback-only HTTP server in ice-cli with explicit Host validation and a separately persisted access token. Reuse available rand/hex or equivalent secure primitives and a constant-time equality crate if needed; bundle the HTML as an asset. Preserve authentication on every route, including streaming and knowledge endpoints. Decide whether to tighten the broad /index prefix, existing-token validation and file-permission recovery as documented security improvements.

### 9. Size and priority

Estimated Rust implementation: **180-320 lines** (estimate, not measured source). **P1** - Authentication is a prerequisite for exposing full live prompts and tool output locally.

## OBS-015 - Read-only state, run replay and Server-Sent Event routes

### 1. Title and status

Read-only state, run replay and Server-Sent Event routes. **Status: missing.**

### 2. ice source

`ice:packages/coding-agent/src/observe/observe-server.ts:79-81`, `ice:packages/coding-agent/src/observe/observe-server.ts:258-270`, `ice:packages/coding-agent/src/observe/observe-server.ts:336-367`, `ice:packages/coding-agent/src/observe/observe-server.ts:414-548`. State/replay/SSE tests: `ice:packages/coding-agent/test/observe-server.test.ts:165-189`, `ice:packages/coding-agent/test/observe-server.test.ts:329-368`.

### 3. What it does

Lets an authenticated browser fetch current or historical observation state and subscribe to live projection changes without sending agent commands.

### 4. How it works

The server routes authenticated read requests to snapshots or stored replay and opens a live SSE stream with an initial snapshot. Ordinary routes do not expose agent mutation controls; POST is used only by the separately specified context-query API.

/state?view=all has first precedence and returns the combined snapshot. Otherwise a truthy runId with a replay store reconstructs that run, returning JSON {error:"run not found"} with 404 when absent; without both it returns the current recorder state. /runs exists only with a replay store and returns {runs:listRuns(limit)}. Replay accepts /replay, /replay/events and any /replay/<suffix>. A non-events suffix is decodeURIComponent-decoded; malformed encoding returns 400 {error:"malformed run id"}. A nonempty path ID wins over query runId, which wins over sessionId lookup of the latest run; no selection/missing run returns 404. The response is {run, events}. queryNumber ignores absent/blank/nonfinite strings and otherwise uses Number, with final limit/cursor validation delegated to OBS-013. These state/run/replay branches do not check request.method (`ice:packages/coding-agent/src/observe/observe-server.ts:258-262`, `ice:packages/coding-agent/src/observe/observe-server.ts:448-497`).

/events responds text/event-stream, cache-control:no-cache, connection:keep-alive. It obtains combined runs, initializes foreign-run cursors, writes a snapshot, then adds the response to the client set and starts shared polling. Live event payloads are flattened to item/lane/thread values; header signals are materialized from recorder.getHeader. Each frame is `event: <name>\ndata: <JSON>\n\n`. Error/close handlers remove the client; removing the last stops the shared poller. A 15000 ms timer writes comment heartbeat `: ping\n\n`, discarding nonwritable/destroyed clients and catching writes. writeSse and heartbeat do not inspect response.write's backpressure return value. There are no SSE id or retry fields, no Last-Event-ID replay protocol, and no per-client bounded queue in this implementation (`ice:packages/coding-agent/src/observe/observe-server.ts:79-81`, `ice:packages/coding-agent/src/observe/observe-server.ts:336-367`, `ice:packages/coding-agent/src/observe/observe-server.ts:414-420`, `ice:packages/coding-agent/src/observe/observe-server.ts:516-548`).

### 5. Interfaces

JSON routes: `/state[?view=all|runId=...]`, `/runs?limit=...`, `/replay?runId=...&after=...&limit=...`, `/replay?sessionId=...`, `/replay/events`, `/replay/<encoded run ID>`. SSE `/events` names snapshot, runs, activity, activity_update, lane, thread, header. Initial snapshot is CombinedObserveState; activity/update data is ObserveItem, lane ObserveLane, thread ObserveThread, header ObserveHeader, runs payload is ObserveRunSummary[]. Unknown routes return 404 plain text not found. JSON content type is application/json;charset=utf-8.

### 6. Dependencies

`OBS-004`, `OBS-013`, `OBS-014`, `OBS-016`

Consumes observation recorder/store; SSE is a standard browser transport and is separate from model RPC.

### 7. ice-rust today

Rust CLI event routing reaches print, JSON, interactive and RPC modes (`rust:crates/ice-cli/src/app.rs:393-503`); its remote module declares SDK/RPC/WebSocket facilities (`rust:crates/ice-cli/src/remote/mod.rs:1-34`). Those transports are not a loopback observation HTTP server with this read-only route/auth/SSE contract. The existing span API (`rust:crates/ice-telemetry/src/context.rs:16-35`) also lacks it. Relevant source-tree inventory and behavior searches are recorded in observability-read-progress.json.

### 8. Port notes

Add route handlers and typed SSE serialization around the observation projection/store. Existing asynchronous Rust networking can host this service, but remote control RPC must not be reused as an authorization bypass. Specify backpressure/slow-client policy explicitly; preserving unbounded Node socket buffering is not required for useful parity, but any change to reconnect/replay semantics must be documented.

### 9. Size and priority

Estimated Rust implementation: **230-380 lines** (estimate, not measured source). **P1** - Connects the durable observation model to browser and diagnostic consumers.

## OBS-016 - Combined live runs, cross-process polling and shared-port lifecycle

### 1. Title and status

Combined live runs, cross-process polling and shared-port lifecycle. **Status: missing.**

### 2. ice source

`ice:packages/coding-agent/src/observe/observe-server.ts:32-47`, `ice:packages/coding-agent/src/observe/observe-server.ts:264-420`, `ice:packages/coding-agent/src/observe/observe-server.ts:550-597`. Tests: `ice:packages/coding-agent/test/observe-server.test.ts:191-327`.

### 3. What it does

Shows multiple recent local agent runs in one dashboard and allows later agents to record into a shared store when another process owns the dashboard port.

### 4. How it works

One server merges its in-memory recorder with recently active persisted runs and polls foreign run events while clients are connected. A bind collision is treated as shared-server reuse without performing an identity or health check on the existing listener.

The run list begins with replayStore.listLiveRuns(Date.now()-600000,50), adds or replaces the current recorder summary, sorts by startedAt descending and slices to 50. Thus current inclusion is attempted before the final cap rather than guaranteed if 50 newer runs exist. For each retained run, current state comes from recorder.getState and foreign state from store.getState(runId,{limit:250}); that limit is the first 250 events, as specified in OBS-013. Items receive missing runId, sort by ts then runId then item.id, and retain the last 1000. Thread identity is qualified by runId:threadKey; lanes also receive missing runId. The top-level header remains the current recorder's header, with view:"all", liveRunId and runs; usage is not an aggregate across runs (`ice:packages/coding-agent/src/observe/observe-server.ts:276-334`).

When a subscriber arrives, known run IDs are retained and only previously absent foreign cursors initialize to each run's current maximum sequence, avoiding historical backfill on the stream. Existing cursor entries are retained when subscribers leave and return. Every 500 ms, and only while at least one client and a replay store exist, the server refreshes combined runs, emits a new combined snapshot for newly discovered runs, emits a runs event when the runId/status/endedAt signature changes, and reads up to 500 events after each foreign cursor. Cursor advances through replay events; activity/update/thread payloads are broadcast with missing run IDs added; lane payloads are forwarded unchanged. A current run uses its live recorder callback, not the polling path. New-run snapshot construction shares the first-250-event limitation; later stream events can update those rows but do not synthesize skipped historical items (`ice:packages/coding-agent/src/observe/observe-server.ts:358-420`).

On EADDRINUSE, startup clears timers and unsubscribes its broadcast listener, then resolves shared:true with the requested port and token URL. It does not verify that the listener is an ICE dashboard; the local recorder continues, and returned close only stops that recorder. Other bind errors reject; the rejection path does not perform the same explicit timer/recorder cleanup. A successful listener returns the actual port. Normal close clears poll and heartbeat, unsubscribes, stops recorder/persistence, ends every client, and awaits server.close (`ice:packages/coding-agent/src/observe/observe-server.ts:550-597`).

### 5. Interfaces

Limits: COMBINED_RUN_LIMIT=50; LIVE_RUN_ACTIVITY_MS=600000; COMBINED_ITEM_LIMIT=1000; COMBINED_STATE_ITEM_LIMIT=250 events; SHARED_POLL_MS=500; foreign replay page limit=500. `CombinedObserveState = ObserveState & {view:"all";liveRunId:string;runs:ObserveRunSummary[]}`. Runs signatures track runId/status/endedAt, not every metadata field. The `shared?:boolean` server return field distinguishes ownership.

### 6. Dependencies

`OBS-003`, `OBS-004`, `OBS-013`, `OBS-014`

Multiple processes share the configured SQLite file and per-user token; no external coordination service.

### 7. ice-rust today

Rust mode routing reaches print, JSON, interactive and RPC (`rust:crates/ice-cli/src/app.rs:393-503`). Its remote module defines native protocol types, a TCP JSONL client and optional four-byte length-prefixed CBOR transport (`rust:crates/ice-cli/src/remote/mod.rs:1-34`). Those facilities do not implement the read-only observation HTTP/SSE routes or their Host/token contract. The existing span API (`rust:crates/ice-telemetry/src/context.rs:16-35`) also lacks them. Relevant source-tree inventory and behavior searches are recorded in observability-read-progress.json and the independent QA evidence.

### 8. Port notes

Use one local HTTP owner and multiple append-only run writers, with a tail cursor for each foreign run. Avoid treating an arbitrary port collision as trusted reuse without a deliberate compatibility decision. For accurate late subscribers, implement a tail-correct reconstruction or cache rather than copying the stale first-250-event shortcut; record that improvement in interface/version notes. Ensure failed binds release timers/subscriptions deterministically.

### 9. Size and priority

Estimated Rust implementation: **200-340 lines** (estimate, not measured source). **P1** - Makes simultaneous agents inspectable and defines process-lifecycle behavior.

## OBS-017 - Lazy shared Cognee connection and dashboard-source recovery

### 1. Title and status

Lazy shared Cognee connection and dashboard-source recovery. **Status: missing.**

### 2. ice source

`ice:packages/coding-agent/src/observe/observe-cognee.ts:30-148`, `ice:packages/coding-agent/src/observe/observe-cognee.ts:183-191`, `ice:packages/coding-agent/src/observe/observe-cognee.ts:240-248`. Key-rotation recovery test: `ice:packages/coding-agent/test/observe-cognee.test.ts:12-55`.

`ice:packages/coding-agent/src/observe/observe-cognee.ts:98-148` exports `createObserveCogneeSource`; `ice:packages/coding-agent/src/main.ts:929-937` constructs it with the session cwd for the dashboard.

### 3. What it does

Connects the Knowledge dashboard to the same Cognee configuration and project dataset as the agent, loading credentials only on first use and recovering on a later request after authentication or reachability failure.

### 4. How it works

createObserveCogneeSource creates a lazy source object whose concurrent first calls share one connection promise. A failed connection or a qualifying Cognee call clears that promise so a later request can pick up edited configuration or rotated credentials.

The factory accepts cwd, optional configPath and injectable fetch. First connect loads the specified config or compatibleCogneeStorageDir(getAgentDir())/config.json through loadCogneeConnection. It derives currentDataset with resolveRuntimeCogneeDataset(config.dataset, cwd), creates a browse client from baseUrl/apiKey, and creates a memory client with maxResponseChars=max(recallMaxChars, maxResponseChars). The connection's enabled flag is exposed as metadata; this wrapper does not gate every method on it. A rejected construction resets connecting and rethrows. withConnection captures the exact promise; only a CogneeError kind auth_failed or unreachable whose promise is still current clears connecting and summaryCache. It rethrows rather than retrying the failed operation; unrelated error kinds leave the connection cached (`ice:packages/coding-agent/src/observe/observe-cognee.ts:98-148`).

Overview returns enabled, baseUrl, currentDataset and the browse listDatasets result, forwarding AbortSignal. The source does not expose API keys in its return DTO. recall forwards query/dataset/topK with timeoutMs=30000 and the caller signal but no sessionId, so it performs dataset context recall rather than session-specific recall (`ice:packages/coding-agent/src/observe/observe-cognee.ts:183-191`, `ice:packages/coding-agent/src/observe/observe-cognee.ts:240-248`).

### 5. Interfaces

Factory `createObserveCogneeSource({cwd:string,configPath?:string,fetch?:typeof fetch}): ObserveCogneeSource`. `ObserveCogneeOverview={enabled:boolean;baseUrl:string;currentDataset:string;datasets:CogneeDataset[]}`. Recall input `{query:string;dataset:string;topK?:number}`, output RecallResult[], optional AbortSignal, fixed timeout 30000 ms. COG-003 specifies connection loading; COG-004 dataset identity; COG-014 recall transport. No observation route mutates configuration.

The factory is `createObserveCogneeSource(options: { cwd: string; configPath?: string; fetch?: typeof fetch }): ObserveCogneeSource`. Its connection is created only on first use, shared through one promise, and reset after a failed initial connection. Auth/unreachable request failures clear the same still-current connection and summary cache.

### 6. Dependencies

`COG-003`, `COG-004`, `COG-014`, `COG-021`

Requires the configured Cognee HTTP service and the shared browse/memory clients. Reads only configuration on demand; no external service was contacted during this audit.

### 7. ice-rust today

No Cognee browse/recall source or observation HTTP adapter is wired into the Rust application mode flow (`rust:crates/ice-cli/src/app.rs:393-503`, `rust:crates/ice-cli/src/remote/mod.rs:1-34`). Its span telemetry (`rust:crates/ice-telemetry/src/production.rs:69-115`) neither connects to Cognee nor exposes datasets. The missing Cognee transport primitives are separately specified in COG-003 and COG-021–030; this entry covers their dashboard integration.

### 8. Port notes

Add an observation-specific adapter over the future Cognee Rust client, with an async shared connection initialization cell that can be invalidated by generation. Preserve the compare-current-promise guard to prevent a stale failure clearing a newer connection. Return configuration metadata without secrets and document the enabled flag's informational semantics. Use cancellation and a 30-second recall deadline.

### 9. Size and priority

Estimated Rust implementation: **150-250 lines** (estimate, not measured source). **P1** - Required integration seam for browsing and reusing the agent's stored knowledge.

## OBS-018 - Cognee dataset summary cache, bounded previews and graph coalescing

### 1. Title and status

Cognee dataset summary cache, bounded previews and graph coalescing. **Status: missing.**

### 2. ice source

`ice:packages/coding-agent/src/observe/observe-cognee.ts:38-96`, `ice:packages/coding-agent/src/observe/observe-cognee.ts:150-248`. Tests: `ice:packages/coding-agent/test/observe-cognee.test.ts:57-168`.

### 3. What it does

Supplies dataset status/counts, documents, graphs and session-cache views while avoiding duplicate expensive requests and limiting preview work.

### 4. How it works

The source caches the summary promise for 30 seconds from creation, including both pending and fulfilled values and coalesces graph requests only while a request is pending. It forwards ordinary browse calls and applies a few dashboard-specific limits and ordering rules.

buildSummary lists datasets, fetches statuses in sequential batches of 100 IDs, and counts each dataset's documents with at most four concurrent tasks. Results are assigned by input index, preserving dataset order. Each count task returns items.length and the lexically greatest truthy createdAt string; a dataset-data error is swallowed for that dataset, leaving documents/latestDocumentAt absent. An auth/unreachable error swallowed here does not reach the connection-reset catch. Dataset-list/status errors reject the overall summary. A status field is included only when truthy. summary creates a promise and records Date.now immediately after creation; it reuses it until Date.now()-at > 30000 (strictly greater), including pending and fulfilled promises. After expiry a new build can overlap an older pending one, so the four-request limit applies per build rather than globally. A matching rejected promise clears the cache. Although the interface accepts a signal, this implementation ignores it and buildSummary forwards none (`ice:packages/coding-agent/src/observe/observe-cognee.ts:85-96`, `ice:packages/coding-agent/src/observe/observe-cognee.ts:150-202`).

dataItem forwards signal and sets maxBytes=480 only when preview is truthy; otherwise the browse client's normal bounded document read applies (COG-025). datasetGraph keys a Map by datasetId, returns the existing pending promise, and deletes the key in finally after success or failure; settled results are not cached here. It has no AbortSignal parameter. datasetSessions fetches the complete session list, filters exact datasetId equality, then sorts descending lexically by lastActivityAt with empty-string fallback. datasetData, datasetStatus, search and session detail forward inputs/signals directly (`ice:packages/coding-agent/src/observe/observe-cognee.ts:203-248`).

### 5. Interfaces

`ObserveCogneeDatasetSummary extends CogneeDataset {status?:string;documents?:number;latestDocumentAt?:string}`. Methods: summary(signal?), datasetData(datasetId, signal?), datasetStatus(datasetIds, signal?), dataItem(datasetId, dataId,{preview?, signal?}?), search({query, searchType, dataset, topK?}, signal?), datasetGraph(datasetId), datasetSessions(datasetId, signal?), session(sessionId, signal?). DataItem returns `{text:string;truncated:boolean}`. Search returns CogneeSearchResult[]; graph CogneeGraph; sessions CogneeSessionSummary[] and CogneeSessionDetail. DTO definitions and underlying transport limits are in COG-021–030.

### 6. Dependencies

`COG-022`, `COG-023`, `COG-024`, `COG-025`, `COG-026`, `COG-028`, `COG-029`, `COG-030`, `OBS-017`

Uses the configured Cognee service and source-owned in-memory caches, not a persistent cache.

### 7. ice-rust today

No Cognee browse/recall source or observation HTTP adapter is wired into the Rust application mode flow (`rust:crates/ice-cli/src/app.rs:393-503`, `rust:crates/ice-cli/src/remote/mod.rs:1-34`). Its span telemetry (`rust:crates/ice-telemetry/src/production.rs:69-115`) neither connects to Cognee nor exposes datasets. The missing Cognee transport primitives are separately specified in COG-003 and COG-021–030; this entry covers their dashboard integration.

### 8. Port notes

Implement an adapter with ordered bounded concurrency, explicit TTL origin and per-key in-flight graph futures. Keep 'unknown document count' distinct from zero on partial failure. If cancellation for summary/graph is improved, avoid one caller canceling shared work still needed by another and document the difference. Do not indefinitely cache graph results at this layer.

### 9. Size and priority

Estimated Rust implementation: **160-280 lines** (estimate, not measured source). **P2** - Makes large knowledge stores usable without duplicate graph downloads or unbounded counting.

## OBS-019 - Cognee HTTP routes, exact validation and cancellation/error boundary

### 1. Title and status

Cognee HTTP routes, exact validation and cancellation/error boundary. **Status: missing.**

### 2. ice source

`ice:packages/coding-agent/src/observe/observe-server.ts:132-256`, `ice:packages/coding-agent/src/observe/observe-server.ts:498-514`. Route/validation/error tests: `ice:packages/coding-agent/test/observe-server.test.ts:440-583`.

### 3. What it does

Exposes authenticated, bounded read/query endpoints for knowledge browsing and context search, with sanitized failures and cancellation when a browser abandons a request.

### 4. How it works

Knowledge routes validate method, decoded path structure and bounded JSON inputs before forwarding to the source. They perform read-only browse, search or recall and map errors into small client-facing responses.

Path segments are split after /cognee and decoded individually; invalid percent encoding produces 400 malformed path. Exact segment counts select routes. GET overview returns the overview; summary wraps datasets; status splits ids on commas and removes empty values, rejecting more than 100 IDs; data list wraps items; item preview is true only for preview=1; graph calls the no-signal source method; dataset sessions wraps sessions; session returns detail. POST search additionally requires searchType in CHUNKS, CHUNKS_LEXICAL, SUMMARIES, GRAPH_COMPLETION; POST recall uses the same query validator without a searchType. Other method/path combinations produce 404 (`ice:packages/coding-agent/src/observe/observe-server.ts:196-245`; allowed context search modes are specified in COG-026).

Body Content-Type must lower-case-start with application/json; otherwise 415 expected application/json. Buffers accumulate actual byteLength up to 16384 bytes. On overflow the data listener is removed, request.resume drains remaining input, and 413 request body too large is returned rather than resetting the connection. JSON parse must yield a non-null object that is not an array; invalid input returns 400 malformed JSON body. query and dataset must be strings, trimmed, nonempty, with query.length <=2000 and dataset.length <=200 (UTF-16 code units); optional topK must be an integer 1..20. Unknown object keys are ignored when constructing forwarded input (`ice:packages/coding-agent/src/observe/observe-server.ts:145-194`).

The server's Host/token checks apply first. If no source is configured, every /cognee route yields 404 {error:"cognee unavailable"}. A response-close event aborts its controller only when the response has not ended. The signal is forwarded to source methods which support it; summary ignores it internally and graph receives none. CogneeRequestError returns its status and {error:message}; CogneeError invalid_request maps to 400, other kinds to 502 with {error:kind, message}; arbitrary failures return 502 {error:"unavailable", message:"Cognee is not configured or unreachable"}. The error response is written only if headers are not already sent. The server grants no CORS preflight route (`ice:packages/coding-agent/src/observe/observe-server.ts:247-255`, `ice:packages/coding-agent/src/observe/observe-server.ts:498-514`).

### 5. Interfaces

| Method/path | Response |
|---|---|
| GET /cognee/overview | ObserveCogneeOverview |
| GET /cognee/summary | {datasets:ObserveCogneeDatasetSummary[]} |
| GET /cognee/status?ids=id1, id2 | {status:Record<string, string>} |
| GET /cognee/datasets/:id/data | {items:CogneeDataItem[]} |
| GET /cognee/datasets/:id/data/:dataId?preview=1 | {text:string, truncated:boolean} |
| GET /cognee/datasets/:id/graph | CogneeGraph |
| GET /cognee/datasets/:id/sessions | {sessions:CogneeSessionSummary[]} |
| GET /cognee/sessions/:id | CogneeSessionDetail |
| POST /cognee/search | {results:CogneeSearchResult[]} |
| POST /cognee/recall | {results:RecallResult[]} |

Query body `{query:string,dataset:string,topK?:number}`; search adds required `searchType:"CHUNKS"|"CHUNKS_LEXICAL"|"SUMMARIES"|"GRAPH_COMPLETION"`. Limits/error texts/units are as above. No routes accept memory writes, agent execution or model completion.

### 6. Dependencies

`COG-026`, `OBS-014`, `OBS-017`, `OBS-018`

Requires configured source/Cognee HTTP service. External model completion is not part of these endpoints.

### 7. ice-rust today

Rust CLI event routing reaches print, JSON, interactive and RPC modes (`rust:crates/ice-cli/src/app.rs:393-503`); its remote module declares SDK/RPC/WebSocket facilities (`rust:crates/ice-cli/src/remote/mod.rs:1-34`). Those transports are not a loopback observation HTTP server with this read-only route/auth/SSE contract. The existing span API (`rust:crates/ice-telemetry/src/context.rs:16-35`) also lacks it. Relevant source-tree inventory and behavior searches are recorded in observability-read-progress.json. No Cognee browse/recall source or observation HTTP adapter is wired into the Rust application mode flow (`rust:crates/ice-cli/src/app.rs:393-503`, `rust:crates/ice-cli/src/remote/mod.rs:1-34`). Its span telemetry (`rust:crates/ice-telemetry/src/production.rs:69-115`) neither connects to Cognee nor exposes datasets. The missing Cognee transport primitives are separately specified in COG-003 and COG-021–030; this entry covers their dashboard integration.

### 8. Port notes

Define typed request/response DTOs and enforce byte caps while streaming the body, before allocating unbounded data. Preserve trim/length/range validation and sanitized error taxonomy. Use cancellation on disconnect while respecting shared-cache ownership; keep read-only authorization separate from any existing RPC command routes. Maintain an explicit search-mode allowlist rather than forwarding arbitrary server operation types.

### 9. Size and priority

Estimated Rust implementation: **180-320 lines** (estimate, not measured source). **P1** - Bounds and validates the dashboard-to-knowledge trust boundary.

## OBS-020 - Bounded activity tape, exact search and stable incremental rendering

### 1. Title and status

Bounded activity tape, exact search and stable incremental rendering. **Status: missing.**

### 2. ice source

`ice:packages/coding-agent/src/observe/observe-dashboard.html:678-760`, `ice:packages/coding-agent/src/observe/observe-dashboard.html:826-940`, `ice:packages/coding-agent/src/observe/observe-dashboard.html:1041-1369`, `ice:packages/coding-agent/src/observe/observe-dashboard.html:1856-1896`. Source has no dedicated dashboard DOM test in the inspected test inventory; server asset-serving tests cover delivery, not these interactions.

### 3. What it does

Renders a searchable, filterable live activity list while retaining stable row identity and limiting the browser's memory/display work.

### 4. How it works

The browser stores at most 500 activity rows keyed by run and item identity, updates existing rows in place, and coalesces paint work into animation frames. Filters and substring search operate on retained row data rather than querying the server.

MAX_ROWS=500. eventKey is `${item.runId || item.mainThreadId}:${item.id}`; thread identity uses the analogous run-qualified key. Appending an existing key patches it. A new item appends to order and evicts the oldest keys beyond 500. A patch replaces the item, invalidates its WeakMap search cache and re-appends if no prior key exists. Snapshot load independently slices its items to the last 500 even though the server's combined limit is 1000. The visible order retains append order after filtering; it is not re-sorted by client timestamps (`ice:packages/coding-agent/src/observe/observe-dashboard.html:1090-1149`, `ice:packages/coding-agent/src/observe/observe-dashboard.html:1319-1347`, `ice:packages/coding-agent/src/observe/observe-dashboard.html:1856-1896`).

Search trims and lowercases the query. The cached lowercase haystack includes kind, eventType, role, toolName, delegationToolName, threadKey, threadName, state, text, args, result and raw, taking at most 20000 UTF-16 code units from each field. Matching is substring inclusion, not regex or tokenized full-text search. Ten filter tabs map digits 0 all, 1 tool, 2 message, 3 skill, 4 compaction, 5 retry, 6 queue, 7 subagent, 8 memory, 9 goal. Filter counts reflect retained items, not the search-filtered subset. First-line previews trim and cap at 150 code units. DOM helpers assign untrusted strings with textContent; innerHTML is used for static SVG glyphs (`ice:packages/coding-agent/src/observe/observe-dashboard.html:826-940`, `ice:packages/coding-agent/src/observe/observe-dashboard.html:1041-1159`).

A pending requestAnimationFrame merges changed keys and stream/pulse invalidation. Changed rows are replaced; unchanged row nodes remain and a cursor-based reconciliation orders/inserts them without relying on detached nextSibling references. Selection is a stable key, with CSS.escape for DOM lookup. Selecting a row focuses/scrolls it and normally clears pulse-bin selection; selecting an empty visible pulse bin resets filter/query so the bin's last item can be selected. The renderer keeps event kind, role/tool, state/error, thread color and duration visible instead of flattening everything to message text (`ice:packages/coding-agent/src/observe/observe-dashboard.html:1160-1369`).

### 5. Interfaces

HTML activity list uses listbox/option semantics, filter buttons, search/clear controls and an inspector. Client state: items Map, order array, selectedId, selectedBinKeys, pulseSelectionActive, filter, query, paused, timeRange, header, lanes, threads, runs, scope, selectedRunId and liveRunId. Exact event DTOs are OBS-004; event transport OBS-015. Search budget 20000 UTF-16 code units per included field; retained row cap 500.

### 6. Dependencies

`OBS-004`, `OBS-015`

Browser DOM, requestAnimationFrame, WeakMap and CSS.escape. No external JavaScript framework or service is required.

### 7. ice-rust today

Rust routes interactive output through its terminal UI and other mode handlers (`rust:crates/ice-cli/src/app.rs:393-503`), not a bundled observation browser UI. Its remote module declares SDK/RPC/WebSocket interfaces (`rust:crates/ice-cli/src/remote/mod.rs:1-34`), and its telemetry writer produces span records (`rust:crates/ice-telemetry/src/production.rs:69-115`). None supplies this HTML/DOM/canvas dashboard behavior. Existing Rust terminal components remain useful and are assessed separately in the TUI area; this gap does not imply a missing terminal interface.

### 8. Port notes

Bundle the existing dependency-free UI asset or implement equivalent DOM behavior against the Rust observation routes. Preserve stable run-qualified keys, bounded rows and incremental updates to avoid selection loss during streaming. If search is moved server-side or made Unicode-code-point-aware, document changed matching/budget semantics. Browser behavior needs integration verification during implementation; this audit did not execute it.

### 9. Size and priority

Estimated Rust implementation: **450-750 lines** (estimate, not measured source). **P2** - Makes raw observation events usable during high-volume streaming.

## OBS-021 - Forensic inspector, tool diff rendering and responsive pane controls

### 1. Title and status

Forensic inspector, tool diff rendering and responsive pane controls. **Status: missing.**

### 2. ice source

`ice:packages/coding-agent/src/observe/observe-dashboard.html:1-676`, `ice:packages/coding-agent/src/observe/observe-dashboard.html:942-1039`, `ice:packages/coding-agent/src/observe/observe-dashboard.html:1371-1620`, `ice:packages/coding-agent/src/observe/observe-dashboard.html:1941-1975`.

### 3. What it does

Lets a user inspect full event arguments, results, raw payloads and edit/write diffs, with a resizable desktop pane and a drawer on narrower screens.

### 4. How it works

Selecting an activity opens a structured inspector while preserving its scroll/disclosure state during repeated updates. Large text is displayed with line numbers and a bounded initial render, and file-tool payloads receive specialized diff presentation.

Code blocks initially render the first 1500 lines and offer a button for the remaining lines; this is a display limit, not deletion of the underlying payload. JSON arguments are pretty-printed when parsing succeeds, otherwise left as text. write tools render content as added lines. edit tools prefer raw.details.diff and otherwise synthesize a replacement diff from path/file_path and either edits (array or parsed JSON string) or oldText/newText. Only entries with string old/new values qualify. The synthesized diff has file and replacement headers, minus old lines and plus new lines. Diff detection recognizes numbered +/-/space lines, ellipsis and @@ headers; tool names are compared case-insensitively (`ice:packages/coding-agent/src/observe/observe-dashboard.html:942-1039`).

The inspector shows event metadata, thread identity, optional arguments/text/result/raw, relevant source data and Copy event JSON. Sections deduplicate payloads where they would repeat, and raw data is an expandable disclosure. Clipboard success displays Copied then resets after 1600 ms; unavailable clipboard/error produces an unavailable state. Live selected-row updates are throttled to 500 ms and preserve inspector scrolling/details when the identity is unchanged. Memory recall arguments may supply an Open/recall-in-Knowledge action through OBS-025 (`ice:packages/coding-agent/src/observe/observe-dashboard.html:1371-1397`, `ice:packages/coding-agent/src/observe/observe-dashboard.html:1456-1619`).

Desktop inspector resizing applies only above 1060 CSS px. Bounds are minimum 500 and maximum min(1200, max(500, workspaceWidth-320)); pointer capture handles primary-button drag. ArrowLeft expands by 24 px, ArrowRight shrinks by 24, Home selects minimum and End maximum. Width persists under localStorage key ice-observe-inspector-width; storage failures are swallowed. The restore conversion uses Number(stored), so a missing value converts to 0 and is clamped to minimum 500. Resizing the window re-clamps a wide-layout pane. CSS switches to drawer at 1060, mobile structure at 780, and more compact rows at 520; reduced-motion preference reduces animation/transition durations to 0.01 ms. Close/backdrop/Escape dismiss the inspector (`ice:packages/coding-agent/src/observe/observe-dashboard.html:1-676`, `ice:packages/coding-agent/src/observe/observe-dashboard.html:1398-1455`, `ice:packages/coding-agent/src/observe/observe-dashboard.html:1941-1975`).

### 5. Interfaces

Persistent key `ice-observe-inspector-width` stores a numeric CSS-pixel width. Inspector contains selectable metadata, text/code/diff blocks, raw disclosure and clipboard actions. Exact resize bounds and keyboard deltas are above. No browser action edits files or re-executes tools; it only renders stored event data.

### 6. Dependencies

`OBS-020`, `OBS-025`

Browser clipboard, localStorage and pointer/keyboard APIs; no third-party renderer required.

### 7. ice-rust today

Rust routes interactive output through its terminal UI and other mode handlers (`rust:crates/ice-cli/src/app.rs:393-503`), not a bundled observation browser UI. Its remote module declares SDK/RPC/WebSocket interfaces (`rust:crates/ice-cli/src/remote/mod.rs:1-34`), and its telemetry writer produces span records (`rust:crates/ice-telemetry/src/production.rs:69-115`). None supplies this HTML/DOM/canvas dashboard behavior. Existing Rust terminal components remain useful and are assessed separately in the TUI area; this gap does not imply a missing terminal interface.

### 8. Port notes

Ship a browser inspector alongside the Rust server, preserving full forensic payload availability and avoiding unsafe HTML interpolation. Reuse the current asset when practical; otherwise implement the exact fallback edit parsing and diff display separately from actual patch execution. Keep mobile drawer and desktop resizing accessible. Rust LOC estimates include serving/wiring plus an equivalent asset maintained with the project, not a claim that DOM must be rewritten in Rust.

### 9. Size and priority

Estimated Rust implementation: **400-700 lines** (estimate, not measured source). **P2** - Preserves the detailed evidence needed to diagnose tool and agent failures.

## OBS-022 - Epoch-aligned activity pulse, per-agent traces and header metrics

### 1. Title and status

Epoch-aligned activity pulse, per-agent traces and header metrics. **Status: missing.**

### 2. ice source

`ice:packages/coding-agent/src/observe/observe-dashboard.html:678-760`, `ice:packages/coding-agent/src/observe/observe-dashboard.html:826-892`, `ice:packages/coding-agent/src/observe/observe-dashboard.html:1620-1768`, `ice:packages/coding-agent/src/observe/observe-dashboard.html:3341-3361`.

### 3. What it does

Shows recent activity density by agent, selectable time bins, active lanes and current-run token/context/cost metrics.

### 4. How it works

The pulse groups retained activities into 42 epoch-aligned time bins and weights different event kinds to show useful activity density. Metrics describe the selected run, or the server's current live run in a combined view, rather than summing every agent.

Selectable windows are 60000, 180000, 300000 (default) and 600000 ms. For historical selection of a run other than liveRunId, the anchor is its last item's timestamp; otherwise it is max(Date.now, last item timestamp). Bin width is window/42, last-bin index is floor(anchor/binWidth), and earliest is (lastIndex-41)*binWidth. Items are assigned by their timestamps within these fixed bins. Weights are error=2, running tool=1.25, other tool=1, subagent=0.75, message=0.25, all other kinds=0.35. Per-main-thread traces use the same bins, and the multi-agent rail is hidden when fewer than two traces exist. Nonempty bar height is 12+round(sqrt(volume/maxVolume)*76) percent, empty bars 3 percent; error and selection states are distinct. Positive inter-event gaps are averaged for the overview. Bin clicks select its item set and last activity while retaining the bin highlight (`ice:packages/coding-agent/src/observe/observe-dashboard.html:1647-1752`).

Header context warning is 70 <= percent < 90 and danger >=90; displayed meter width is clamped 0..100. Token total sums input/output/cacheRead/cacheWrite, cost prints to three decimals, percentages to one decimal, compact counts use k/m notation. Duration shows rounded milliseconds below 1000, two-decimal seconds below 60000, and minutes plus rounded remainder seconds thereafter. Active lanes exclude terminal rows; phase/status, fault indication, current tool/path and summary appear with role/thread/run identity fallbacks. failed/timed_out phases or status error are faulted (`ice:packages/coding-agent/src/observe/observe-dashboard.html:897-932`, `ice:packages/coding-agent/src/observe/observe-dashboard.html:1620-1646`, `ice:packages/coding-agent/src/observe/observe-dashboard.html:1753-1768`).

A one-second pulse refresh runs only while the document is visible, there are items, the view is not a historical run, and keyboard focus is not inside the pulse element. Streaming changes also invalidate pulse rendering. Changing the time range recalculates the same retained data rather than fetching older events (`ice:packages/coding-agent/src/observe/observe-dashboard.html:1330-1369`, `ice:packages/coding-agent/src/observe/observe-dashboard.html:3341-3361`).

### 5. Interfaces

Constants PULSE_BINS=42; ranges 1m/3m/5m/10m; client default 5m. Header fields are OBS-004. Lane fields are ObserveLane, including streamKey/status/phase/terminal/startedAtMs/elapsedMs and optional currentTool/currentPath/model/attempt/summary. Activity weights are visualization semantics, not execution progress percentages.

### 6. Dependencies

`OBS-004`, `OBS-020`, `OBS-023`

Browser clocks/rendering and existing recorder data; no metrics backend.

### 7. ice-rust today

Rust routes interactive output through its terminal UI and other mode handlers (`rust:crates/ice-cli/src/app.rs:393-503`), not a bundled observation browser UI. Its remote module declares SDK/RPC/WebSocket interfaces (`rust:crates/ice-cli/src/remote/mod.rs:1-34`), and its telemetry writer produces span records (`rust:crates/ice-telemetry/src/production.rs:69-115`). None supplies this HTML/DOM/canvas dashboard behavior. Existing Rust terminal components remain useful and are assessed separately in the TUI area; this gap does not imply a missing terminal interface.

### 8. Port notes

Preserve bin alignment, historical anchoring and one-run metric semantics so activity can be compared across clients. Keep pulse-selection focus stable during streaming and provide non-color labels for error/agent identity. A static or averaged chart would not provide the same click-to-event navigation.

### 9. Size and priority

Estimated Rust implementation: **220-380 lines** (estimate, not measured source). **P2** - Helps users distinguish active, stalled and failed work across concurrent agents.

## OBS-023 - Dashboard run switching, live-event filtering and follow controls

### 1. Title and status

Dashboard run switching, live-event filtering and follow controls. **Status: missing.**

### 2. ice source

`ice:packages/coding-agent/src/observe/observe-dashboard.html:1769-1975`, `ice:packages/coding-agent/src/observe/observe-dashboard.html:3341-3361`.

### 3. What it does

Switches between all live agents and an individual recorded run while retaining live updates for the selected run and letting the user pause automatic scrolling.

### 4. How it works

The client keeps one EventSource connected while changing the visible run through snapshot requests. Pause stops automatic scrolling only; incoming events continue to be stored and rendered.

Run choices are initially fetched once from /runs?limit=50 with cache:no-store, then updated by SSE. Dropdown options show sessionName or the first 12 session-ID code units, an eight-unit session prefix and live/finished status. A signature of selected value plus runId/status/sessionName suppresses identical option rebuilds; while the select has focus, a changed rebuild is deferred to a once-only blur listener so the open menu stays usable. Selection sets scope/selectedRunId before fetching /state?view=all or /state?runId=encoded; non-OK/network errors are silently ignored. There is no request-generation guard in loadSelectedRun, so rapid switches can race; that is a source limitation, not an asserted runtime reproduction (`ice:packages/coding-agent/src/observe/observe-dashboard.html:1785-1839`).

Live activity acceptance is all scope, missing runId, or exact selected runId/liveRunId. Combined header acceptance targets liveRunId; individual mode targets selectedRunId, with missing-target/header-ID compatibility. A combined snapshot arriving while inspecting one run updates only the run list/liveRunId; it does not replace the selected run view. Snapshot application resets items and threads, limits rows to 500, qualifies thread keys by run and also records a first unqualified fallback. If selection disappeared it selects the final retained row and clears pulse-selection activity. SSE names are snapshot/runs/activity/activity_update/lane/thread/header. Runs data is an array. Lane keys are `${runId||"current"}:${streamKey}`. Every memory activity/update is first offered to knowledge-refresh logic even when its run is not displayed. Native EventSource handles reconnect; there is no custom reconnect/backoff/error UI or Last-Event-ID protocol (`ice:packages/coding-agent/src/observe/observe-dashboard.html:1841-1939`).

Scrolling more than the 36 px bottom threshold pauses follow. Resume scrolls to the bottom. Activity keyboard shortcuts are disabled in Knowledge view; digits 0–9 filter unless typing in INPUT/TEXTAREA/SELECT, Space toggles follow except in those inputs or a focused BUTTON/A, Escape closes inspector, and Up/Down choose adjacent visible rows with endpoint clamping. The code ignores typing for Escape. Buttons provide equivalent mobile follow/close behavior (`ice:packages/coding-agent/src/observe/observe-dashboard.html:1769-1783`, `ice:packages/coding-agent/src/observe/observe-dashboard.html:1941-1975`).

### 5. Interfaces

Requests `/runs?limit=50`, `/state?view=all`, `/state?runId=<encoded>`, EventSource `/events`. scope is all/run; selectedRunId is all or a concrete run ID. Follow is a client-only paused boolean. Keys: 0–9 filters, Space follow, Escape close, ArrowUp/ArrowDown row selection, subject to focus guards above.

### 6. Dependencies

`OBS-015`, `OBS-016`, `OBS-020`

Browser EventSource/fetch and DOM focus state. Does not pause execution or the server stream.

### 7. ice-rust today

Rust routes interactive output through its terminal UI and other mode handlers (`rust:crates/ice-cli/src/app.rs:393-503`), not a bundled observation browser UI. Its remote module declares SDK/RPC/WebSocket interfaces (`rust:crates/ice-cli/src/remote/mod.rs:1-34`), and its telemetry writer produces span records (`rust:crates/ice-telemetry/src/production.rs:69-115`). None supplies this HTML/DOM/canvas dashboard behavior. Existing Rust terminal components remain useful and are assessed separately in the TUI area; this gap does not imply a missing terminal interface.

### 8. Port notes

Keep snapshot state and incoming run-qualified events consistent. Add a generation guard for asynchronous selection if intentionally improving the observed TS race, and document that choice. Preserve follow as scrolling control so users can inspect old rows without losing new evidence. Do not conflate it with stopping an agent.

### 9. Size and priority

Estimated Rust implementation: **180-320 lines** (estimate, not measured source). **P2** - Supports stable inspection of concurrent and historical runs.

## OBS-024 - Knowledge view lifecycle, coalesced polling and memory-triggered refresh

### 1. Title and status

Knowledge view lifecycle, coalesced polling and memory-triggered refresh. **Status: missing.**

### 2. ice source

`ice:packages/coding-agent/src/observe/observe-dashboard.html:760-821`, `ice:packages/coding-agent/src/observe/observe-dashboard.html:1977-2250`, `ice:packages/coding-agent/src/observe/observe-dashboard.html:3314-3361`.

### 3. What it does

Loads knowledge only when requested, keeps the selected dataset current while visible, and refreshes after relevant memory writes without flooding the server.

### 4. How it works

The browser lazily initializes the Knowledge view and coalesces overlapping dataset refreshes. Periodic and event-driven refreshes run only when that view is visible, with stale-response guards for dataset selection.

The localStorage key ice-observe-view stores activity or knowledge; invalid/missing values default to activity and storage errors are ignored. First entry coalesces /cognee/overview, sorts datasets by name.localeCompare, selects the currentDataset by name or the first dataset, and loads it. Subsequent visits refresh existing selection. cogneeFetch uses GET unless a body is supplied (then POST/application-json), attempts JSON parsing, and on non-OK throws payload.message || payload.error || HTTP-status text. Overview failures are displayed and do not crash Activity (`ice:packages/coding-agent/src/observe/observe-dashboard.html:1978-2113`).

Selecting a dataset clears prior items, sessions, known/fresh IDs, selected record, status/check/error state; increments the content request counter; resets graph state and refreshAgain; renders and starts refresh. A refresh promise is associated with its dataset ID. Another call for that same in-flight dataset sets refreshAgain=true and awaits it; after completion one follow-up refresh runs. Data, status and sessions requests run concurrently; sessions failure falls back to current cached sessions while data/status failure produces a visible error. Results apply only if the selected dataset ID still matches. Documents sort newest first by parsed createdAt. The first successful load establishes known IDs without fresh badges; later new IDs become fresh. Checked time updates, graph stale detection receives item-count/status change, and matching promise identity controls cleanup (`ice:packages/coding-agent/src/observe/observe-dashboard.html:2114-2188`).

Every 15000 ms, visible Knowledge with a selected dataset refreshes that dataset; visibility restoration triggers dataset catch-up and reloads a stale graph if its pane is visible. The explicit Refresh action separately reloads overview and selected dataset, and forces the summary only when its pane is shown. Successful memory activities are relevant only when kind=memory, state=done and toolName is remember, remember_entry or improve. If visible with an overview, they schedule a 1500 ms debounce. Dataset identity is parsed from raw evidence: current/no dataset triggers selected refresh, and an unknown named dataset triggers overview reload. Event handling runs even for activity rows outside the selected run. These are read refreshes; they never initiate a Cognee write (`ice:packages/coding-agent/src/observe/observe-dashboard.html:2189-2250`, `ice:packages/coding-agent/src/observe/observe-dashboard.html:3314-3361`).

Server date strings without a trailing Z or numeric offset are treated as UTC by appending Z before Date.parse. Status mappings use exact keys DATASET_PROCESSING_COMPLETED → cognified/ok, DATASET_PROCESSING_STARTED → processing/busy, DATASET_PROCESSING_INITIATED → queued/busy, DATASET_PROCESSING_ERRORED → errored/bad. Session-cache state maps completed/running/failed/abandoned into its separate presentation, not the dataset processing state (`ice:packages/coding-agent/src/observe/observe-dashboard.html:1984-2056`).

### 5. Interfaces

Persistent key ice-observe-view; knowledge polling 15000 ms; event debounce 1500 ms. State includes overview/loading/error, datasetId/items/sessions, knownIds/freshIds, refreshing/refreshingFor/refreshAgain, request counter, previews and graph state. Data endpoints are OBS-019. Visible means the knowledge element is not hidden and document.hidden is false.

### 6. Dependencies

`OBS-008`, `OBS-018`, `OBS-019`, `OBS-023`

Browser visibility/localStorage/fetch and configured Cognee source. No background refresh is required when the view is hidden.

### 7. ice-rust today

Rust routes interactive output through its terminal UI and other mode handlers (`rust:crates/ice-cli/src/app.rs:393-503`), not a bundled observation browser UI. Its remote module declares SDK/RPC/WebSocket interfaces (`rust:crates/ice-cli/src/remote/mod.rs:1-34`), and its telemetry writer produces span records (`rust:crates/ice-telemetry/src/production.rs:69-115`). None supplies this HTML/DOM/canvas dashboard behavior. Existing Rust terminal components remain useful and are assessed separately in the TUI area; this gap does not imply a missing terminal interface.

### 8. Port notes

Maintain a dataset generation and in-flight identity for safe asynchronous refresh. Separate unknown status, empty content and failed content, and preserve coalescing so timer/events do not create concurrent refresh storms. The current fallback to cached sessions can retain prior data during failure; decide whether a stricter per-dataset cache is warranted and document it.

### 9. Size and priority

Estimated Rust implementation: **230-400 lines** (estimate, not measured source). **P2** - Keeps knowledge browsing responsive and current during live agent work.

## OBS-025 - Knowledge document previews, session detail and context-query inspector

### 1. Title and status

Knowledge document previews, session detail and context-query inspector. **Status: missing.**

### 2. ice source

`ice:packages/coding-agent/src/observe/observe-dashboard.html:2251-2494`, `ice:packages/coding-agent/src/observe/observe-dashboard.html:1978-2031`.

### 3. What it does

Browses stored documents and session-cache traces, opens full bounded content, and runs context-only recall/search with links from observed memory activities.

### 4. How it works

The Knowledge view progressively previews visible documents and guards full-content/query rendering with a request counter. It exposes stored evidence and context results without editing memory or running model completions.

Document/session sidebar rows preserve focused selection across rerenders with escaped DOM selectors. An IntersectionObserver rooted at the sidebar and rootMargin 120 px queues near-visible document previews. At most four preview requests run concurrently. Cache/queue keys are dataId only, not dataset-qualified; null marks an in-flight/cached placeholder. Queued work whose dataset is no longer selected is removed, but an already-running preview response has no dataset-generation guard. Successful previews choose the first useful trimmed nonblank line, skipping Session ID/Session/Reason/Engine prefixes and stripping Question/Summary prefixes, then cap title to 140 code units. Failures delete the cache entry so later rendering may retry. Preview cache has no explicit count eviction in this browser code (`ice:packages/coding-agent/src/observe/observe-dashboard.html:2251-2390`).

Opening a document increments the shared request counter, fetches the full data item, and applies it only when the counter still matches. It renders text and a truncation notice (the normal browse limit is 256 KiB from COG-025), with a Show graph action. Opening a session uses the same guard, prefers effective status from the list, renders session identity/status/counts, reverses returned entries and traces so newest appears first, and shows the limited/latest-entry notice. It displays the already projected Cognee session DTO, not arbitrary server filesystem paths (`ice:packages/coding-agent/src/observe/observe-dashboard.html:2391-2443`; full DTO/transport behavior in COG-030).

Query submission trims query, requires a selected dataset, chooses recall or an allowlisted search mode, increments the request counter and measures elapsed performance.now. It POSTs query/dataset/topK:10, adding searchType for search, then renders results only if still current. Results show rank, dataset, optional score to three decimals and context text, with explicit empty/error states. Modes are recall, CHUNKS, CHUNKS_LEXICAL, SUMMARIES and GRAPH_COMPLETION. A memory activity's recall link is available only for kind memory, toolName recall and parseable args; raw may identify dataset. Opening it switches to Knowledge, loads overview, selects a matching dataset name when found, sets query/mode and runs recall. If no match is found it keeps the current dataset, so exact historical dataset replay is not guaranteed (`ice:packages/coding-agent/src/observe/observe-dashboard.html:2444-2494`).

### 5. Interfaces

Preview concurrency=4; rootMargin=120 px; preview title cap 140 UTF-16 code units; query topK=10. Requests are GET data item with preview=1, GET full data item, GET session detail, POST search/recall. Content-selection request counter rejects stale full-document/session/query responses; preview requests have the different behavior specified above. UI modes and result schemas are OBS-019/COG-021–030.

### 6. Dependencies

`COG-025`, `COG-030`, `OBS-018`, `OBS-019`, `OBS-024`

Browser IntersectionObserver, fetch, performance timing and Cognee context endpoints.

### 7. ice-rust today

Rust routes interactive output through its terminal UI and other mode handlers (`rust:crates/ice-cli/src/app.rs:393-503`), not a bundled observation browser UI. Its remote module declares SDK/RPC/WebSocket interfaces (`rust:crates/ice-cli/src/remote/mod.rs:1-34`), and its telemetry writer produces span records (`rust:crates/ice-telemetry/src/production.rs:69-115`). None supplies this HTML/DOM/canvas dashboard behavior. Existing Rust terminal components remain useful and are assessed separately in the TUI area; this gap does not imply a missing terminal interface.

### 8. Port notes

Implement progressive previews with a bounded worker queue and explicit cache identity. A safer dataset-qualified preview key/generation is a deliberate improvement over TS and should be recorded. Keep full-content request ordering and clear truncation indications; do not substitute completion-producing search modes for context queries.

### 9. Size and priority

Estimated Rust implementation: **300-500 lines** (estimate, not measured source). **P2** - Provides actionable inspection of the knowledge behind memory observations.

## OBS-026 - Dataset summary table and explicit cache-refresh semantics

### 1. Title and status

Dataset summary table and explicit cache-refresh semantics. **Status: missing.**

### 2. ice source

`ice:packages/coding-agent/src/observe/observe-dashboard.html:2495-2536`, `ice:packages/coding-agent/src/observe/observe-dashboard.html:2189-2198`, `ice:packages/coding-agent/src/observe/observe-dashboard.html:2598-2603`.

### 3. What it does

Lists every known dataset with processing status, document count and latest activity, and opens a dataset directly from the table.

### 4. How it works

The dataset table fetches the summary endpoint on first use and reuses its client-side summary until explicitly forced. Its displayed 30-second refresh wording reflects the server cache TTL, not an independently scheduled table refresh.

loadDatasetsSummary(false) renders a cached knowledge.summary immediately. Otherwise it shows a counting message, fetches /cognee/summary, assigns its datasets array and sorts descending by serverTime(latestDocumentAt || createdAt). Failure shows a summary-specific error and returns. The table uses dataset.name, marks the overview.currentDataset name as this project, marks the selected dataset row, maps exact processing statuses through DATASET_STATUS and shows an em dash for an undefined count. Aggregate documents sums known counts with unknown as zero. Clicking a dataset button calls selectDataset and switches to the Explore pane (`ice:packages/coding-agent/src/observe/observe-dashboard.html:2495-2536`).

Opening the Datasets pane invokes the non-force path. The general Refresh action first reloads overview and selected data, and only when the table is visible calls loadDatasetsSummary(true). The 15000 ms dataset poll does not force the summary. Thus the table can remain cached for longer than 30000 ms despite its literal status text 'counts refresh every 30s'; the source's real server-side promise TTL is separately specified in OBS-018 (`ice:packages/coding-agent/src/observe/observe-dashboard.html:2189-2198`, `ice:packages/coding-agent/src/observe/observe-dashboard.html:2598-2603`, `ice:packages/coding-agent/src/observe/observe-dashboard.html:3337-3342`).

### 5. Interfaces

GET `/cognee/summary` → `{datasets:ObserveCogneeDatasetSummary[]}`. Columns: dataset name, processing status, document count, latestDocumentAt-or-createdAt. No separate persistent cache key or table refresh timer. Missing count remains distinct from zero in each row, while aggregate uses zero for unknown.

### 6. Dependencies

`OBS-018`, `OBS-019`, `OBS-024`

Cognee summary service adapter and browser in-memory state.

### 7. ice-rust today

Rust routes interactive output through its terminal UI and other mode handlers (`rust:crates/ice-cli/src/app.rs:393-503`), not a bundled observation browser UI. Its remote module declares SDK/RPC/WebSocket interfaces (`rust:crates/ice-cli/src/remote/mod.rs:1-34`), and its telemetry writer produces span records (`rust:crates/ice-telemetry/src/production.rs:69-115`). None supplies this HTML/DOM/canvas dashboard behavior. Existing Rust terminal components remain useful and are assessed separately in the TUI area; this gap does not imply a missing terminal interface.

### 8. Port notes

Preserve explicit unknown counts and dataset selection. Correct or implement the advertised refresh cadence deliberately; do not infer periodic client refresh merely from the label. Reuse shared source TTL and avoid starting one count request per row from the browser.

### 9. Size and priority

Estimated Rust implementation: **70-130 lines** (estimate, not measured source). **P3** - Useful cross-dataset inventory, with a documented stale-cache limitation.

## OBS-027 - Knowledge graph loading, type filtering and incremental dataset updates

### 1. Title and status

Knowledge graph loading, type filtering and incremental dataset updates. **Status: missing.**

### 2. ice source

`ice:packages/coding-agent/src/observe/observe-dashboard.html:2538-2794`, `ice:packages/coding-agent/src/observe/observe-dashboard.html:3223-3231`.

### 3. What it does

Loads a dataset's knowledge graph on demand, filters node types, preserves layout across reloads and marks newly discovered nodes.

### 4. How it works

The graph client caches one settled graph for its selected dataset and coalesces overlapping loads for that dataset. It resets stale interaction state immediately on a dataset switch and rebuilds graph relationships while preserving same-dataset positions and type selections.

Entering Graph calls loadGraph. It returns the cached data only when datasetId matches and graph.stale is false, unless forced; even a force call joins an existing matching in-flight promise. Response and error application both compare the current knowledge.datasetId, preventing an old dataset response from replacing the new graph. Matching promise identity controls loading cleanup. resetGraphForDataset clears drawing/adjacency/selection/hover/drag/matches/fresh IDs/pending document/fit target/alpha immediately so old nodes cannot answer clicks for a new dataset (`ice:packages/coding-agent/src/observe/observe-dashboard.html:2594-2680`).

applyGraph retains finite positions from same-dataset nodes, computes freshIds as new source IDs relative to the prior graph, and otherwise clears positions/types/selection/view/find state and enables auto-fit. Nodes gain x/y/vx/vy/degree/fixed fields. Edges with a missing endpoint or a self-loop are skipped; valid edges increment both endpoint degrees and create adjacency records with outgoing direction. Degrees include all valid graph edges, not only visible ones. Types sort by count descending and preserve prior on/off choice; default visible types are Entity and EntityType, but if none of the available types is on, all types are enabled. Colors are Entity cyan, EntityType violet, TextSummary amber, DocumentChunk slate, TextDocument mint, NodeSet rose, unknown orange. Selection/hover references are rebound by ID and dragging is cleared (`ice:packages/coding-agent/src/observe/observe-dashboard.html:2682-2747`).

Visibility selects enabled-type nodes and edges whose two endpoints are visible, with a degree-descending node list. New unplaced nodes start near a placed visible neighbor at random radius 12..28 or in a disk with radius sqrt(random)*(sqrt(visibleCount)*14+40). Hidden selections clear. Initial graph heat is 1, same-dataset reload 0.4, type toggle 0.6; alpha takes max(current, heat). A changed document count or transition from a defined noncompleted status to DATASET_PROCESSING_COMPLETED marks the graph stale; it auto-reloads only while the graph pane is visible. Same-count content changes without that transition do not invalidate it (`ice:packages/coding-agent/src/observe/observe-dashboard.html:2749-2794`, `ice:packages/coding-agent/src/observe/observe-dashboard.html:3223-3231`).

### 5. Interfaces

Input CogneeGraph supplies nodes, edges and truncation metadata from COG-027/028. Graph status shows visible/total nodes, visible edges, largest-N-of-total truncation, matches, fresh-node count and stale/reload state. Client state includes datasetId/data/loading/loadingFor/stale/error, node/edge maps, adjacency, type flags, saved positions, matches and selection. Source coalescing in OBS-018 is a separate cache layer.

### 6. Dependencies

`COG-027`, `COG-028`, `OBS-018`, `OBS-019`, `OBS-024`

Browser canvas/DOM and read-only Cognee graph; no graph database or layout package is used in the browser.

### 7. ice-rust today

Rust routes interactive output through its terminal UI and other mode handlers (`rust:crates/ice-cli/src/app.rs:393-503`), not a bundled observation browser UI. Its remote module declares SDK/RPC/WebSocket interfaces (`rust:crates/ice-cli/src/remote/mod.rs:1-34`), and its telemetry writer produces span records (`rust:crates/ice-telemetry/src/production.rs:69-115`). None supplies this HTML/DOM/canvas dashboard behavior. Existing Rust terminal components remain useful and are assessed separately in the TUI area; this gap does not imply a missing terminal interface.

### 8. Port notes

Bundle the graph client or implement equivalent state and generation guards. Keep graph DTO normalization at the transport layer and UI visibility/layout state at the browser layer. Preserve same-dataset layout and new-node identity so live reloads do not scatter the graph. Document any stronger invalidation/versioning policy.

### 9. Size and priority

Estimated Rust implementation: **240-420 lines** (estimate, not measured source). **P2** - Makes graph updates and large typed knowledge datasets inspectable.

## OBS-028 - Bounded-frame Barnes–Hut graph layout and level-of-detail drawing

### 1. Title and status

Bounded-frame Barnes–Hut graph layout and level-of-detail drawing. **Status: missing.**

### 2. ice source

`ice:packages/coding-agent/src/observe/observe-dashboard.html:2796-3065`, `ice:packages/coding-agent/src/observe/observe-dashboard.html:2542-2546`.

### 3. What it does

Lays out knowledge nodes interactively while bounding work per animation frame and drawing only a limited set of labels.

### 4. How it works

The browser runs a cooling force simulation using Barnes–Hut repulsion, degree-weighted springs and a weak centering force. It stops ticking when settled, preserves positions for reloads and uses bounded label selection so dense graphs remain navigable.

buildQuadtree takes the visible-node bounding box and a square size max(width, height)+1. Each insertion updates mass and centroid. Cells split into four; after depth >24, coincident points collect in an extra list. Pair repulsion skips self, applies random jitter in [-0.0005,0.0005) per axis when squared distance <1e-6, and replaces squared distance <1 with its square root. A nonleaf cell is opened when centroid distance is zero or size²/distance² >=0.81; otherwise its total mass acts at the centroid. Per-node strength is -45*alpha (`ice:packages/coding-agent/src/observe/observe-dashboard.html:2796-2885`).

For each visible edge, displacement uses predicted positions x+vx/y+vy. distance is hypot or 1; pull=((distance-34)/distance)*alpha/min(source.degree, target.degree), and bias=source.degree/(source.degree+target.degree). Target velocity subtracts displacement*pull*bias, source adds displacement*pull*(1-bias). Every node gets centering acceleration -position*0.03*alpha. Fixed nodes zero velocity and skip movement; others multiply velocity by 0.6 then add it to position. Nonfinite positions reset randomly into a 40-unit square with zero velocity. alpha decays by alpha += (0-alpha)*0.0228, stopping at <=0.02. Empty visible nodes immediately set alpha 0 (`ice:packages/coding-agent/src/observe/observe-dashboard.html:2877-2920`).

A requested animation frame is unique until it runs. Hidden graph panes or zero width stop rendering. While hot, ticks run in a do/while for at most an approximate 12 ms performance.now budget, then draw. On settlement positions are saved and a pending fit-to selection or auto-fit is applied unless dragging. Initial fit waits until alpha<0.25. Frames continue only while hot or dragging. Resize uses stage CSS size and devicePixelRatio for the canvas backing dimensions. Fit bounds add 60 units and cap scale at 2.5. Node radius is min(14,2.5+sqrt(degree)*0.8) (`ice:packages/coding-agent/src/observe/observe-dashboard.html:2922-2989`).

Drawing dims unrelated/unmatched nodes to alpha 0.22, highlights focus edges, outlines fresh nodes mint and selected nodes white. At most 160 labels are chosen in priority order: focus, neighbors, search matches, fresh nodes; with no focus/search, the top 14 hubs and nodes whose radius*scale>=5 follow. Offscreen label margins are 40 px horizontal and 20 vertical. Labels longer than 40 code units become the first 39 plus ellipsis. Labels render in screen coordinates at 11 px with a dark stroke for contrast (`ice:packages/coding-agent/src/observe/observe-dashboard.html:2991-3065`).

### 5. Interfaces

Simulation/drawing constants: Barnes–Hut threshold squared 0.81; max split depth 24; repulsion 45; spring rest length 34; center 0.03; velocity retention 0.6; alpha decay 0.0228; settled 0.02; frame tick budget 12 ms; initial-fit threshold 0.25; fit padding 60 and max scale 2.5; labels 160; hub labels 14; node radius cap 14. These are browser layout semantics, separate from Cognee server graph-node cap.

### 6. Dependencies

`OBS-027`

Canvas 2D, requestAnimationFrame, performance.now and Math.random. Layout is nondeterministic and no numerical equivalence test was executed during the audit.

### 7. ice-rust today

Rust routes interactive output through its terminal UI and other mode handlers (`rust:crates/ice-cli/src/app.rs:393-503`), not a bundled observation browser UI. Its remote module declares SDK/RPC/WebSocket interfaces (`rust:crates/ice-cli/src/remote/mod.rs:1-34`), and its telemetry writer produces span records (`rust:crates/ice-telemetry/src/production.rs:69-115`). None supplies this HTML/DOM/canvas dashboard behavior. Existing Rust terminal components remain useful and are assessed separately in the TUI area; this gap does not imply a missing terminal interface.

### 8. Port notes

Preserve the browser asset where feasible; a Rust server need not reimplement force simulation. If moving layout to Rust/WASM or a worker, keep source and visible graph degrees, cooling, frame budget and interaction pinning explicit. Guard nonfinite positions and stop hidden-view work. Any crate/package replacement must match meaningful behavior rather than just the Barnes–Hut label.

### 9. Size and priority

Estimated Rust implementation: **250-450 lines** (estimate, not measured source). **P3** - Supplies the performance and readability behavior of large graph views.

## OBS-029 - Graph navigation, directional relationships and document-to-entity tracing

### 1. Title and status

Graph navigation, directional relationships and document-to-entity tracing. **Status: missing.**

### 2. ice source

`ice:packages/coding-agent/src/observe/observe-dashboard.html:3067-3323`.

### 3. What it does

Lets users drag, pan, zoom, search and follow graph relationships, then trace a stored document to the entities extracted from it.

### 4. How it works

Graph interaction uses run-local browser state and read-only node selection. Document highlighting follows explicit graph relationships rather than asking Cognee to generate an explanation.

Hit testing walks visible nodes in reverse drawing order using world coordinates and reach nodeRadius+3/scale. Selection rejects nodes no longer in byId, removes their fresh badge and updates the details card. Centering disables auto-fit and, when canvas width>700 and a card width exists, shifts left by half(card.width+12) to use uncovered space. The card displays type, label, connection count, sets and optional text. Adjacency groups retain incoming/outgoing direction in relation headings, sort groups by number of visible targets then total count, and render at most 80 link buttons across groups; the remainder count is explicit. Following a hidden target enables its type with heat 0.3, centers it and returns keyboard focus to the rebuilt card (`ice:packages/coding-agent/src/observe/observe-dashboard.html:3067-3153`).

Find trims/lowercases and matches visible node labels by substring only when query length>=2 code units. Input resets matchCursor=-1. Enter cycles degree-descending matches; Shift+Enter decrements modulo count. The formula `(cursor+step+count)%count` means the first backward step from -1 starts at count-2 for count>1. Document tracing begins with the node whose ID equals the data/document ID, collects adjacent DocumentChunk nodes and their neighbors except NodeSet/TextDocument, and for any TextSummary also follows its adjacent Entity nodes. Highlighting prefers Entity/EntityType results if present, otherwise the whole traced chain; it enables needed types, clears find/selection, sets matches and disables auto-fit. If simulation is hot it defers framing until settlement; otherwise it fits immediately. A missing document reports that it may still be processing. Requests before graph loading retain pendingDocument (`ice:packages/coding-agent/src/observe/observe-dashboard.html:3155-3221`, `ice:packages/coding-agent/src/observe/observe-dashboard.html:3314-3323`).

Only primary-button pointerdown begins graph drag and captures the pointer. A node is fixed while dragged; empty space pans. Movement must exceed 3 CSS px before it becomes a drag. Panning disables auto-fit; node movement converts screen to world coordinates and reheats to at least 0.2. Pointerup/cancel/lost-capture clears drag and unpins; an unmoved node click selects it and an unmoved background click clears selection. Hover updates tooltip and focus drawing. Wheel prevents default, converts line delta to 16 px and page delta to graph.height, scales by exp(-pixels*0.0015), clamps 0.03..8 and preserves the cursor's world position. ResizeObserver updates the canvas; Fit and Reload buttons call fitGraph/loadGraph(true) (`ice:packages/coding-agent/src/observe/observe-dashboard.html:3233-3313`).

### 5. Interfaces

Graph controls: node type toggles; find text (minimum 2 UTF-16 code units); Enter/Shift+Enter match navigation; node/background click; primary-pointer drag/pan; wheel zoom; Fit; Reload; node relation buttons; document Show graph. Limits: 80 relation buttons; hit margin 3/scale; drag threshold>3 px; zoom 0.03..8; line wheel 16 px; zoom exponent factor 0.0015. No operation mutates Cognee graph or agent execution.

### 6. Dependencies

`OBS-025`, `OBS-027`, `OBS-028`

Browser Pointer Events, wheel normalization, ResizeObserver and existing graph DTO.

### 7. ice-rust today

Rust routes interactive output through its terminal UI and other mode handlers (`rust:crates/ice-cli/src/app.rs:393-503`), not a bundled observation browser UI. Its remote module declares SDK/RPC/WebSocket interfaces (`rust:crates/ice-cli/src/remote/mod.rs:1-34`), and its telemetry writer produces span records (`rust:crates/ice-telemetry/src/production.rs:69-115`). None supplies this HTML/DOM/canvas dashboard behavior. Existing Rust terminal components remain useful and are assessed separately in the TUI area; this gap does not imply a missing terminal interface.

### 8. Port notes

Implement pointer capture cleanup for all termination events so nodes do not remain pinned. Keep directional relationship grouping and document extraction traversal exact. Preserve readable navigation/focus after DOM replacement. Decide whether to correct the first Shift+Enter wrap quirk; it is source-read and has not been reproduced in a browser here.

### 9. Size and priority

Estimated Rust implementation: **260-440 lines** (estimate, not measured source). **P2** - Turns stored graphs into navigable evidence tied to concrete documents.

