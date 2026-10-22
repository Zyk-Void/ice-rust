# Agent loop, SDK, execution environment and runtime-neutral helpers

The active low-level Agent/agentLoop implementation is distinct from the exported AgentHarness v2 scaffold. Entries identify exact call sites and do not infer implementation from comments or type declarations. Existing tests were read, not run. All filesystem/network/process behavior below is a specification of source, not work performed by this audit.

## AGENT-001 - Open message union and complete agent event payloads

### 1. Title and status

Open message union and complete agent event payloads. **Status: different-by-design.**

### 2. ice source

ice:packages/agent/src/types.ts:294-437; exported AgentMessage, CustomAgentMessages, AgentContext, AgentEvent, AgentTool/AgentToolResult. Tests: ice:packages/agent/test/agent-loop.test.ts:119-164 (`should emit events with AgentMessage types`); ice:packages/agent/test/agent-loop.test.ts:166-219 (`should handle custom message types via convertToLlm`).

### 3. What it does

Expose user, assistant, tool-result and application-defined messages through one typed lifecycle event stream.

### 4. How it works

AgentMessage is the provider Message union plus every value of the declaration-mergeable CustomAgentMessages interface. No central runtime registration or schema validates a new custom role. AgentContext holds a system-prompt string, message array and optional tools. AgentEvent has the ten variants quoted below. message_start/end and turn_end accept any AgentMessage; message_update also names AgentMessage at the type boundary even though the loop emits assistant updates. Tool result content is text/image only; details is an application-defined required value, usage and addedToolNames are optional, and terminate is not a message role or persistent event. A consumer must not treat arbitrary custom data or tool output as policy. There is no event retention, serial number, size cap or persistence in these types; those belong to higher layers.

### 5. Interfaces

```ts
export type ThinkingLevel = "off" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max" | "ultra";

/**
 * Extensible interface for custom app messages.
 * Apps can extend via declaration merging:
 *
 * @example
 * ```typescript
 * declare module "@zykairotis/ice-agent-core" {
 *   interface CustomAgentMessages {
 *     artifact: ArtifactMessage;
 *     notification: NotificationMessage;
 *   }
 * }
 * ```
 */
export interface CustomAgentMessages {
	// Empty by default - apps extend via declaration merging
}

/**
 * AgentMessage: Union of LLM messages + custom messages.
 * This abstraction allows apps to add custom message types while maintaining
 * type safety and compatibility with the base LLM messages.
 */
export type AgentMessage = Message | CustomAgentMessages[keyof CustomAgentMessages];
```
Source: ice:packages/agent/src/types.ts:294-319.
```ts
export interface AgentToolResult<T> {
	/** Text or image content returned to the model. */
	content: (TextContent | ImageContent)[];
	/** Arbitrary structured details for logs or UI rendering. */
	details: T;
	/** Usage from the final tool execution itself, if available. Not used for main LLM context accounting. */
	usage?: Usage;
	/** Names of tools introduced by this result and available from this transcript point onward. */
	addedToolNames?: string[];
	/**
	 * Hint that the agent should stop after the current tool batch.
	 * Early termination only happens when every finalized tool result in the batch sets this to true.
	 */
	terminate?: boolean;
}

/**
 * Callback used by tools to stream partial execution updates.
 *
 * The callback is scoped to the current `execute()` invocation. Calls made after
 * the tool promise settles are ignored.
 */
export type AgentToolUpdateCallback<T = any> = (partialResult: AgentToolResult<T>) => void;

/** Tool definition used by the agent runtime. */
export interface AgentTool<TParameters extends TSchema = TSchema, TDetails = any> extends Tool<TParameters> {
	/** Human-readable label for UI display. */
	label: string;
	/**
	 * Optional compatibility shim for raw tool-call arguments before schema validation.
	 * Must return an object that matches `TParameters`.
	 */
	prepareArguments?: (args: unknown) => Static<TParameters>;
	/** Execute the tool call. Throw on failure instead of encoding errors in `content`. */
	execute: (
		toolCallId: string,
		params: Static<TParameters>,
		signal?: AbortSignal,
		onUpdate?: AgentToolUpdateCallback<TDetails>,
	) => Promise<AgentToolResult<TDetails>>;
	/**
	 * Per-tool execution mode override.
	 * - "sequential": this tool must execute one at a time with other tool calls.
	 * - "parallel": this tool can execute concurrently with other tool calls.
	 *
	 * If omitted, the default execution mode applies.
	 */
	executionMode?: ToolExecutionMode;
}

/** Context snapshot passed into the low-level agent loop. */
export interface AgentContext {
	/** System prompt included with the request. */
	systemPrompt: string;
	/** Transcript visible to the model. */
	messages: AgentMessage[];
	/** Tools available for this run. */
	tools?: AgentTool<any>[];
}

/**
 * Events emitted by the Agent for UI updates.
 *
 * `agent_end` is the last event emitted for a run, but awaited `Agent.subscribe()`
 * listeners for that event are still part of run settlement. The agent becomes
 * idle only after those listeners finish.
 */
export type AgentEvent =
	// Agent lifecycle
	| { type: "agent_start" }
	| { type: "agent_end"; messages: AgentMessage[] }
	// Turn lifecycle - a turn is one assistant response + any tool calls/results
	| { type: "turn_start" }
	| { type: "turn_end"; message: AgentMessage; toolResults: ToolResultMessage[] }
	// Message lifecycle - emitted for user, assistant, and toolResult messages
	| { type: "message_start"; message: AgentMessage }
	// Only emitted for assistant messages during streaming
	| { type: "message_update"; message: AgentMessage; assistantMessageEvent: AssistantMessageEvent }
	| { type: "message_end"; message: AgentMessage }
	// Tool execution lifecycle
	| { type: "tool_execution_start"; toolCallId: string; toolName: string; args: any }
	| { type: "tool_execution_update"; toolCallId: string; toolName: string; args: any; partialResult: any }
	| { type: "tool_execution_end"; toolCallId: string; toolName: string; result: any; isError: boolean };
```
Source: ice:packages/agent/src/types.ts:355-437.
None: additional CLI flags, slash commands, keybindings, core settings keys, agent RPC commands, or persistent files beyond those explicitly specified. This entry concerns only the named in-process APIs/events.

### 6. Dependencies

`AI-019`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

Final dependency classification. Shared contract references: `AI-006`, `RPC-004`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

rust:crates/ice-agent/src/types.rs:37-54; rust:crates/ice-agent/src/types.rs:162-220; rust:crates/ice-agent/src/message.rs:1-160; rust:crates/ice-agent/src/events.rs:18-110. Rust models custom roles using an open JSON-backed enum instead of TypeScript declaration merging. AgentEvent is not serde-serialized, its update payload is Arc<AssistantMessage> rather than any AgentMessage, and it adds RetryScheduled. Rust details is JSON only, added names is an always-present vector and terminate defaults false. These ownership/type differences are deliberate; any JSONL adapter still needs the exact TypeScript field/tag shapes, not Rust debug output. Direct Rust serde AgentMessage encoding additionally uses a kind discriminator with snake_case variants (tool_result), and custom role fields are nested in data; TypeScript AgentMessage is flat and role-discriminated. The code derives serde directly, contrary to the file comment suggesting a custom flattening implementation. Source: rust:crates/ice-agent/src/message.rs:25-32 and :148-155. Session/RPC adapters may transform this and are audited separately.

### 8. Port notes

Target ice-agent unless explicitly identified as an ice-harness or ice-tools helper. Preserve one authoritative loop. Use owned snapshots and explicit async error/cancellation contracts; native plugin consumers require the same host-facing hook/event data, not a second controller. Existing workspace dependencies tokio, tokio-util, futures and serde_json cover the primitives.

### 9. Size and priority

Estimated Rust implementation: **130-230 lines** (estimate, not measured source). **P1** - Required for deterministic SDK, event and tool behavior during a TypeScript-to-Rust port.

## AGENT-002 - Asynchronous stream-function construction

### 1. Title and status

Asynchronous stream-function construction. **Status: partial.**

### 2. ice source

ice:packages/agent/src/types.ts:20-33; ice:packages/agent/src/agent-loop.ts:281-308; ice:packages/agent/src/agent.ts:506-547. Symbols StreamFn/streamAssistantResponse. Tests: ice:packages/agent/test/agent.test.ts:159-188 (`emits full lifecycle events for thrown run failures`).

### 3. What it does

Allow an embedding to obtain credentials or initialize a provider asynchronously before returning the assistant stream.

### 4. How it works

StreamFn may return an AssistantMessageEventStream immediately or a Promise of it. streamAssistantResponse awaits the callable before consuming events and then awaits the stream result on completion. There is no construction deadline or cancellation race outside the supplied signal. A rejected construction promise propagates from the raw run function; the Agent wrapper synthesizes a terminal error lifecycle. The public contract asks implementations to put expected request failures into stream error events, but the wrapper also handles violations. Do not replace an async factory with a detached producer silently: factory error timing and cancellation ownership differ.

### 5. Interfaces

```ts
 * this shape.
 *
 * Contract:
 * - Must not throw or return a rejected promise for request/model/runtime failures.
 * - Must return an AssistantMessageEventStream.
 * - Failures must be encoded in the returned stream via protocol events and a
 *   final AssistantMessage with stopReason "error" or "aborted" and errorMessage.
 */
export type StreamFn = (
	model: Model<Api>,
	context: Context,
	options?: SimpleStreamOptions,
) => AssistantMessageEventStream | Promise<AssistantMessageEventStream>;

```
Source: ice:packages/agent/src/types.ts:20-33.
```ts
		tools: context.tools,
	};

	// Resolve API key (important for expiring tokens)
	const resolvedApiKey =
		(config.getApiKey ? await config.getApiKey(config.model.provider) : undefined) || config.apiKey;

	const response = await streamFunction(config.model, llmContext, {
```
Source: ice:packages/agent/src/agent-loop.ts:301-308.
None: additional CLI flags, slash commands, keybindings, core settings keys, agent RPC commands, or persistent files beyond those explicitly specified. This entry concerns only the named in-process APIs/events.

### 6. Dependencies

`AGENT-001`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

### 7. ice-rust today

rust:crates/ice-agent/src/stream_fn.rs:21-41. Rust StreamFn returns a consumer synchronously. The comment at lines 4-8 incorrectly says the TypeScript loop never awaits the callable. Rust tests bridge async provider creation with tokio::spawn instead, rust:crates/ice-agent/src/agent.rs:598-617. Add an async factory contract or an explicit adapter whose error/cancellation semantics are documented.

### 8. Port notes

Target ice-agent unless explicitly identified as an ice-harness or ice-tools helper. Preserve one authoritative loop. Use owned snapshots and explicit async error/cancellation contracts; native plugin consumers require the same host-facing hook/event data, not a second controller. Existing workspace dependencies tokio, tokio-util, futures and serde_json cover the primitives.

### 9. Size and priority

Estimated Rust implementation: **45-85 lines** (estimate, not measured source). **P1** - Required for deterministic SDK, event and tool behavior during a TypeScript-to-Rust port.

## AGENT-003 - Replaceable default stream registry and package entry points

### 1. Title and status

Replaceable default stream registry and package entry points. **Status: partial.**

### 2. ice source

ice:packages/agent/src/stream-fn.ts:1-21; ice:packages/agent/src/agent.ts:225-245; ice:packages/agent/src/agent-loop.ts:95-143; ice:packages/agent/src/index.ts:1-81; ice:packages/agent/src/node.ts:1-3. Exports setDefaultStreamFn/getDefaultStreamFn and package barrels. Tests: ice:packages/agent/test/agent.test.ts:85-104 (`uses the configured default when a legacy caller omits streamFn`); ice:packages/agent/test/agent-loop.test.ts:85-115 (`uses the configured default when a legacy caller omits streamFn`).

### 3. What it does

Support legacy callers that omit an explicit stream function and allow applications/tests to replace or clear the fallback.

### 4. How it works

The module owns one mutable defaultStreamFn. setDefaultStreamFn assigns the supplied function or undefined on every call; getDefaultStreamFn throws the exact error below while empty. Agent construction resolves a missing function immediately, including untyped callers that omit options entirely. The raw loop resolves its fallback after emitting initial lifecycle/prompt events. The main index reexports the loop, Agent, proxy, types, harness/session/compaction/resource helpers; the node entrypoint additionally reexports NodeJsExecutionEnv. Importing the agent package alone does not choose a provider. This is process/module state only, with no file or environment-variable configuration.

### 5. Interfaces

```ts
import type { StreamFn } from "./types.ts";

let defaultStreamFn: StreamFn | undefined;

/**
 * Configure the fallback used by Agent and low-level loops when callers omit streamFn.
 *
 * Hosts that provide a default model runtime can install its stream function here
 * without making ice-agent-core depend on a provider catalog or compatibility layer.
 */
export function setDefaultStreamFn(streamFn: StreamFn | undefined): void {
	defaultStreamFn = streamFn;
}

export function getDefaultStreamFn(): StreamFn {
	if (!defaultStreamFn) {
		throw new Error("No default stream function configured. Pass streamFn explicitly or call setDefaultStreamFn().");
	}
	return defaultStreamFn;
}

```
Source: ice:packages/agent/src/stream-fn.ts:1-21.
None: additional CLI flags, slash commands, keybindings, core settings keys, agent RPC commands, or persistent files beyond those explicitly specified. This entry concerns only the named in-process APIs/events.

### 6. Dependencies

`AGENT-002`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

### 7. ice-rust today

rust:crates/ice-agent/src/stream_fn.rs:44-77; rust:crates/ice-agent/src/agent.rs:250-305. Rust uses OnceLock: first Some wins, later replacements and None are ignored. Missing fallback errors differ in wording and Result boundary. No Rust equivalent is needed for Node-specific import paths, but default replacement/clear semantics are a real SDK gap.

### 8. Port notes

Target ice-agent unless explicitly identified as an ice-harness or ice-tools helper. Preserve one authoritative loop. Use owned snapshots and explicit async error/cancellation contracts; native plugin consumers require the same host-facing hook/event data, not a second controller. Existing workspace dependencies tokio, tokio-util, futures and serde_json cover the primitives.

### 9. Size and priority

Estimated Rust implementation: **30-65 lines** (estimate, not measured source). **P2** - Required for deterministic SDK, event and tool behavior during a TypeScript-to-Rust port.

## AGENT-004 - Raw loop entry, continuation, sink and return boundaries

### 1. Title and status

Raw loop entry, continuation, sink and return boundaries. **Status: different-by-design.**

### 2. ice source

ice:packages/agent/src/agent-loop.ts:19-143. Exports agentLoop/agentLoopContinue/runAgentLoop/runAgentLoopContinue/AgentEventSink. Tests: ice:packages/agent/test/agent-loop.test.ts:1492-1509 (`should throw when context has no messages`); ice:packages/agent/test/agent-loop.test.ts:1511-1551 (`should continue from existing context without emitting user message events`); ice:packages/agent/test/agent-loop.test.ts:1553-1612 (`should allow custom message types as last message (caller responsibility)`).

### 3. What it does

Start or continue a run with explicit lifecycle events, preserving which messages are considered newly produced.

### 4. How it works

runAgentLoop clones the context object and transcript array, initializes newMessages with the supplied prompts, emits agent_start then turn_start, emits start/end for each prompt, and enters the loop. Prompt objects themselves are not deep-cloned. runAgentLoopContinue rejects an empty transcript or an assistant tail, permits any other custom last role, emits only agent_start/turn_start, and initializes newMessages empty. Its shallow context copy retains the original messages array, so raw continuation appends can mutate the caller array; Agent avoids that by supplying its own snapshot.

The sink may return void or a promise and every explicit await is an ordering/backpressure point. Raw run functions return newly added messages; no session file or event journal is written. agentLoop/agentLoopContinue adapt the sink to EventStream, complete its result when agent_end arrives and end it after the run promise resolves. The adapter attaches only a fulfillment handler; an exceptional raw run can reject without ending the adapter stream. This is an existing failure boundary, not proof of a reliable retry/recovery mechanism.

### 5. Interfaces

```ts
	AgentTool,
	AgentToolCall,
	AgentToolResult,
	StreamFn,
} from "./types.ts";

export type AgentEventSink = (event: AgentEvent) => Promise<void> | void;

/**
 * Start an agent loop with a new prompt message.
 * The prompt is added to the context and events are emitted for it.
 */
export function agentLoop(
	prompts: AgentMessage[],
	context: AgentContext,
	config: AgentLoopConfig,
	signal: AbortSignal | undefined,
	streamFn: StreamFn,
): EventStream<AgentEvent, AgentMessage[]> {
	const stream = createAgentStream();

	void runAgentLoop(
		prompts,
		context,
		config,
		async (event) => {
			stream.push(event);
		},
		signal,
		streamFn,
	).then((messages) => {
		stream.end(messages);
	});

	return stream;
}

/**
 * Continue an agent loop from the current context without adding a new message.
 * Used for retries - context already has user message or tool results.
 *
 * **Important:** The last message in context must convert to a `user` or `toolResult` message
 * via `convertToLlm`. If it doesn't, the LLM provider will reject the request.
 * This cannot be validated here since `convertToLlm` is only called once per turn.
 */
export function agentLoopContinue(
	context: AgentContext,
	config: AgentLoopConfig,
	signal: AbortSignal | undefined,
	streamFn: StreamFn,
): EventStream<AgentEvent, AgentMessage[]> {
	if (context.messages.length === 0) {
		throw new Error("Cannot continue: no messages in context");
	}

	if (context.messages[context.messages.length - 1].role === "assistant") {
		throw new Error("Cannot continue from message role: assistant");
	}

	const stream = createAgentStream();

	void runAgentLoopContinue(
		context,
		config,
		async (event) => {
			stream.push(event);
		},
		signal,
		streamFn,
	).then((messages) => {
		stream.end(messages);
	});

	return stream;
}
```
Source: ice:packages/agent/src/agent-loop.ts:19-93.
None: additional CLI flags, slash commands, keybindings, core settings keys, agent RPC commands, or persistent files beyond those explicitly specified. This entry concerns only the named in-process APIs/events.

### 6. Dependencies

`AGENT-001`, `AGENT-002`, `AGENT-003`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

### 7. ice-rust today

rust:crates/ice-agent/src/agent_loop.rs:1-211; rust:crates/ice-agent/src/events.rs:113-201. Rust run_agent_loop returns Result<Vec<AgentMessage>>, while run_agent_loop_continue/with_state return a LoopOutcome through an explicit emitter. There is no matching TS EventStream convenience facade. Owned Rust context avoids the raw TS continuation array alias. Rust also has run_agent_loop_from_assistant for interrupted tool-batch recovery, not the TS queued-prompt continuation contract.

### 8. Port notes

Target ice-agent unless explicitly identified as an ice-harness or ice-tools helper. Preserve one authoritative loop. Use owned snapshots and explicit async error/cancellation contracts; native plugin consumers require the same host-facing hook/event data, not a second controller. Existing workspace dependencies tokio, tokio-util, futures and serde_json cover the primitives.

### 9. Size and priority

Estimated Rust implementation: **120-220 lines** (estimate, not measured source). **P1** - Required for deterministic SDK, event and tool behavior during a TypeScript-to-Rust port.

## AGENT-005 - Turn progression and exact steering/follow-up drain order

### 1. Title and status

Turn progression and exact steering/follow-up drain order. **Status: partial.**

### 2. ice source

ice:packages/agent/src/agent-loop.ts:148-275. Internal runLoop. Tests: ice:packages/agent/test/agent-loop.test.ts:681-785 (`should inject queued messages after all tool calls complete`); ice:packages/agent/test/agent-loop.test.ts:1377-1440 (`should continue after parallel tool calls when not all tool results terminate`).

### 3. What it does

Run tools and additional model turns until completion while inserting queued user work only at defined boundaries.

### 4. How it works

After initial events, poll steering once. The first turn does not emit a second turn_start; subsequent turns do. Inject pending messages in order, emitting start/end before appending each to currentContext/newMessages. Stream one assistant, append it to newMessages, and immediately end on stopReason error/aborted without tools, prepare/stop hooks or queue drains. Otherwise execute tool blocks when present regardless of nonerror stop reason, append finalized tool results in source order, emit turn_end, apply prepareNextTurn, then test shouldStopAfterTurn.

Only then drain steering. A nonempty tool batch normally causes another turn unless every finalized result requests termination; steering still causes a turn even when the batch terminates. When the inner tool/steering loop would stop, poll follow-ups and begin another turn if any. Error/abort terminal responses leave queued messages untouched. The TypeScript loop has no extra signal-only exit between tool completion and the next stream/follow-up, no turn limit, no batch concurrency limit and no queue-size cap. Cancellation is observed by providers/tools and the dispatch checks, not by inventing a new prompt or retry.

### 5. Interfaces

```ts
		(event: AgentEvent) => (event.type === "agent_end" ? event.messages : []),
	);
}

/**
 * Main loop logic shared by agentLoop and agentLoopContinue.
 */
async function runLoop(
	initialContext: AgentContext,
	newMessages: AgentMessage[],
	initialConfig: AgentLoopConfig,
	signal: AbortSignal | undefined,
	emit: AgentEventSink,
	streamFunction: StreamFn,
): Promise<void> {
	let currentContext = initialContext;
	let config = initialConfig;
	let firstTurn = true;
	// Check for steering messages at start (user may have typed while waiting)
	let pendingMessages: AgentMessage[] = (await config.getSteeringMessages?.()) || [];

	// Outer loop: continues when queued follow-up messages arrive after agent would stop
	while (true) {
		let hasMoreToolCalls = true;

		// Inner loop: process tool calls and steering messages
		while (hasMoreToolCalls || pendingMessages.length > 0) {
			if (!firstTurn) {
				await emit({ type: "turn_start" });
			} else {
				firstTurn = false;
			}

			// Process pending messages (inject before next assistant response)
```
Source: ice:packages/agent/src/agent-loop.ts:148-181.
```ts
			await emit({ type: "turn_end", message, toolResults });

			const nextTurnContext = {
				message,
				toolResults,
				context: currentContext,
				newMessages,
			};
			const nextTurnSnapshot = await config.prepareNextTurn?.(nextTurnContext);
			if (nextTurnSnapshot) {
				currentContext = nextTurnSnapshot.context ?? currentContext;
				config = {
					...config,
					model: nextTurnSnapshot.model ?? config.model,
					reasoning:
						nextTurnSnapshot.thinkingLevel === undefined
							? config.reasoning
							: nextTurnSnapshot.thinkingLevel === "off"
								? undefined
								: nextTurnSnapshot.thinkingLevel,
				};
			}

			if (
				await config.shouldStopAfterTurn?.({
					message,
					toolResults,
					context: currentContext,
					newMessages,
				})
			) {
				await emit({ type: "agent_end", messages: newMessages });
				return;
			}

			pendingMessages = (await config.getSteeringMessages?.()) || [];
		}

		// Agent would stop here. Check for follow-up messages.
		const followUpMessages = (await config.getFollowUpMessages?.()) || [];
		if (followUpMessages.length > 0) {
			// Set as pending so inner loop processes them
			pendingMessages = followUpMessages;
			continue;
		}

		// No more messages, exit
		break;
	}

	await emit({ type: "agent_end", messages: newMessages });
}
```
Source: ice:packages/agent/src/agent-loop.ts:224-275.
None: additional CLI flags, slash commands, keybindings, core settings keys, agent RPC commands, or persistent files beyond those explicitly specified. This entry concerns only the named in-process APIs/events.

### 6. Dependencies

`AGENT-004`, `AGENT-010`, `AGENT-017`, `AGENT-018`, `AGENT-019`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

### 7. ice-rust today

rust:crates/ice-agent/src/agent_loop.rs:213-432. Rust largely follows the order, but invokes its additional after_tool_results hook before turn_end and exits Aborted immediately after steering drain when the signal is cancelled and no steering is pending (lines 334-347,402-411). That can suppress the additional aborted assistant/follow-up path that TypeScript would attempt. Its explicit LoopOutcome is also extra. Treat early cancellation as a conscious behavioral choice, not full event parity.

### 8. Port notes

Target ice-agent unless explicitly identified as an ice-harness or ice-tools helper. Preserve one authoritative loop. Use owned snapshots and explicit async error/cancellation contracts; native plugin consumers require the same host-facing hook/event data, not a second controller. Existing workspace dependencies tokio, tokio-util, futures and serde_json cover the primitives.

### 9. Size and priority

Estimated Rust implementation: **100-180 lines** (estimate, not measured source). **P1** - Required for deterministic SDK, event and tool behavior during a TypeScript-to-Rust port.

## AGENT-006 - Context transformation before provider-message conversion

### 1. Title and status

Context transformation before provider-message conversion. **Status: partial.**

### 2. ice source

ice:packages/agent/src/agent-loop.ts:281-300; ice:packages/agent/src/types.ts:144-205; ice:packages/agent/src/agent.ts:33-37. transformContext/convertToLlm. Tests: ice:packages/agent/test/agent-loop.test.ts:221-272 (`should apply transformContext before convertToLlm`); ice:packages/agent/test/agent-loop.test.ts:166-219 (`should handle custom message types via convertToLlm`).

### 3. What it does

Let trusted host hooks prune or augment application messages before converting them to provider wire messages.

### 4. How it works

For each request pass the current AgentMessage array and signal to optional transformContext, await the returned array, then await convertToLlm on that result. Conversion sees custom roles; it is required for raw loop configurations and defaults in Agent to filtering only user/assistant/toolResult. The returned transformed array affects the provider request but is not assigned back to durable/current history by this function; a hook can nevertheless mutate shared message objects/arrays in TypeScript. Build provider Context from the current systemPrompt, converted messages and tools. No automatic token pruning, custom-role rendering, sanitization or trust approval occurs here. Hook failure propagates to the enclosing run boundary.

### 5. Interfaces

```ts
async function streamAssistantResponse(
	context: AgentContext,
	config: AgentLoopConfig,
	signal: AbortSignal | undefined,
	emit: AgentEventSink,
	streamFunction: StreamFn,
): Promise<AssistantMessage> {
	// Apply context transform if configured (AgentMessage[] → AgentMessage[])
	let messages = context.messages;
	if (config.transformContext) {
		messages = await config.transformContext(messages, signal);
	}

	// Convert to LLM-compatible messages (AgentMessage[] → Message[])
	const llmMessages = await config.convertToLlm(messages);

	// Build LLM context
	const llmContext: Context = {
		systemPrompt: context.systemPrompt,
		messages: llmMessages,
```
Source: ice:packages/agent/src/agent-loop.ts:281-300.
```ts
function defaultConvertToLlm(messages: AgentMessage[]): Message[] {
	return messages.filter(
		(message) => message.role === "user" || message.role === "assistant" || message.role === "toolResult",
	);
}
```
Source: ice:packages/agent/src/agent.ts:33-37.
None: additional CLI flags, slash commands, keybindings, core settings keys, agent RPC commands, or persistent files beyond those explicitly specified. This entry concerns only the named in-process APIs/events.

### 6. Dependencies

`AGENT-001`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

### 7. ice-rust today

rust:crates/ice-agent/src/hooks.rs:26-38; rust:crates/ice-agent/src/hooks.rs:191-211; rust:crates/ice-agent/src/agent_loop.rs:437-480. Rust has both hooks in the raw loop but passes owned cloned values and infallible future outputs rather than a fallible promise. The public Agent wrapper hardcodes transform_context=None, rust:crates/ice-agent/src/agent.rs:510-541. Keep the wrapper gap separate from raw-loop support; native AgentHarness has its own transform_context path.

### 8. Port notes

Target ice-agent unless explicitly identified as an ice-harness or ice-tools helper. Preserve one authoritative loop. Use owned snapshots and explicit async error/cancellation contracts; native plugin consumers require the same host-facing hook/event data, not a second controller. Existing workspace dependencies tokio, tokio-util, futures and serde_json cover the primitives.

### 9. Size and priority

Estimated Rust implementation: **55-105 lines** (estimate, not measured source). **P1** - Required for deterministic SDK, event and tool behavior during a TypeScript-to-Rust port.

## AGENT-007 - Per-request dynamic key and full stream-option forwarding

### 1. Title and status

Per-request dynamic key and full stream-option forwarding. **Status: partial.**

### 2. ice source

ice:packages/agent/src/agent-loop.ts:298-309; ice:packages/agent/src/types.ts:144-205; ice:packages/agent/src/agent.ts:465-503. getApiKey/streamAssistantResponse. Tests: ice:packages/agent/test/agent.test.ts:803-827 (`forwards sessionId to streamFunction options`); ice:packages/agent/test/agent-loop.test.ts:1155-1226 (`should use prepareNextTurn snapshot before continuing`).

### 3. What it does

Resolve the current provider credential for every model request and preserve the caller’s provider options.

### 4. How it works

Call getApiKey with current model.provider on every request, await it, and use its value when truthy; undefined or empty string falls back to config.apiKey. Then invoke the stream callable with {...config, apiKey:resolvedApiKey, signal}. Thus every SimpleStreamOptions field supplied to the raw loop survives, including sampling, token budget, service tier, metadata, headers, transport, response/payload callbacks and retry hints; loop-only properties also remain in the JavaScript object and provider code ignores them. There is no credential caching, refresh lock, independent retry or secret logging at this layer. A changed model from prepareNextTurn changes the next credential lookup. An exception in the resolver is a run failure.

### 5. Interfaces

```ts
export interface AgentLoopConfig extends SimpleStreamOptions {
	model: Model<any>;

	/**
	 * Converts AgentMessage[] to LLM-compatible Message[] before each LLM call.
	 *
	 * Each AgentMessage must be converted to a UserMessage, AssistantMessage, or ToolResultMessage
	 * that the LLM can understand. AgentMessages that cannot be converted (e.g., UI-only notifications,
	 * status messages) should be filtered out.
	 *
	 * Contract: must not throw or reject. Return a safe fallback value instead.
	 * Throwing interrupts the low-level agent loop without producing a normal event sequence.
	 *
	 * @example
	 * ```typescript
	 * convertToLlm: (messages) => messages.flatMap(m => {
	 *   if (m.role === "custom") {
	 *     // Convert custom message to user message
	 *     return [{ role: "user", content: m.content, timestamp: m.timestamp }];
	 *   }
	 *   if (m.role === "notification") {
	 *     // Filter out UI-only messages
	 *     return [];
	 *   }
	 *   // Pass through standard LLM messages
	 *   return [m];
	 * })
	 * ```
	 */
	convertToLlm: (messages: AgentMessage[]) => Message[] | Promise<Message[]>;

	/**
	 * Optional transform applied to the context before `convertToLlm`.
	 *
	 * Use this for operations that work at the AgentMessage level:
	 * - Context window management (pruning old messages)
	 * - Injecting context from external sources
	 *
	 * Contract: must not throw or reject. Return the original messages or another
	 * safe fallback value instead.
	 *
	 * @example
	 * ```typescript
	 * transformContext: async (messages) => {
	 *   if (estimateTokens(messages) > MAX_TOKENS) {
	 *     return pruneOldMessages(messages);
	 *   }
	 *   return messages;
	 * }
	 * ```
	 */
	transformContext?: (messages: AgentMessage[], signal?: AbortSignal) => Promise<AgentMessage[]>;

	/**
	 * Resolves an API key dynamically for each LLM call.
	 *
	 * Useful for short-lived OAuth tokens (e.g., GitHub Copilot) that may expire
	 * during long-running tool execution phases.
	 *
	 * Contract: must not throw or reject. Return undefined when no key is available.
	 */
	getApiKey?: (provider: string) => Promise<string | undefined> | string | undefined;
```
Source: ice:packages/agent/src/types.ts:144-205.
Stream options spread and key fallback: ice:packages/agent/src/agent-loop.ts:298-309. None: additional CLI flags, slash commands, keybindings, core settings keys, agent RPC commands, or persistent files beyond those explicitly specified. This entry concerns only the named in-process APIs/events.

### 6. Dependencies

`AGENT-002`, `AGENT-006`, `AI-006`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

Final dependency classification. Shared contract references: `AI-003`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

rust:crates/ice-agent/src/hooks.rs:99-129; rust:crates/ice-agent/src/hooks.rs:162-188; rust:crates/ice-agent/src/agent_loop.rs:480-499. Rust raw AgentLoopConfig exposes only a curated subset and to_stream_options explicitly sets headers/metadata=None; other provider fields use defaults. An empty Some key is retained instead of falling back. Agent build_config supplies no dynamic resolver. AgentHarness can wrap provider streaming separately, so this finding is the public low-level/Agent API gap, not a blanket claim that CLI headers or OAuth are unavailable.

### 8. Port notes

Target ice-agent unless explicitly identified as an ice-harness or ice-tools helper. Preserve one authoritative loop. Use owned snapshots and explicit async error/cancellation contracts; native plugin consumers require the same host-facing hook/event data, not a second controller. Existing workspace dependencies tokio, tokio-util, futures and serde_json cover the primitives.

### 9. Size and priority

Estimated Rust implementation: **95-160 lines** (estimate, not measured source). **P1** - Required for deterministic SDK, event and tool behavior during a TypeScript-to-Rust port.

## AGENT-008 - Streaming assistant reduction and terminal result selection

### 1. Title and status

Streaming assistant reduction and terminal result selection. **Status: partial.**

### 2. ice source

ice:packages/agent/src/agent-loop.ts:281-378. streamAssistantResponse. Tests: ice:packages/agent/test/agent-loop.test.ts:119-164 (`should emit events with AgentMessage types`); ice:packages/agent/test/e2e.test.ts:242-264 (`preserves thinking content blocks`).

### 3. What it does

Project provider stream events into one assistant message without committing a separate message for each delta.

### 4. How it works

Await the stream factory. On start, take event.partial as the active message, append it to current context, and emit message_start with a shallow copy. Text/thinking/tool-call start/delta/end events are ignored until a start has been seen; afterward replace the current last message with event.partial and emit message_update, retaining the underlying provider event. On done/error, await stream.result rather than trusting only the event payload. Replace the partial message, or append and synthesize message_start if no start arrived; emit message_end and return.

If iteration ends without a terminal event, await result and perform the same append/replace/end fallback, but not the abort reclassification described separately. No stream-shape validation prevents multiple starts, missing results or malformed partial arrays; no deadline bounds a result promise. Shallow event copies can share content with later deltas. The agent wrapper reducer, not this helper, owns the public transcript. Provider result text/images/signatures/usage are retained without a second parse.

### 5. Interfaces

```ts
		apiKey: resolvedApiKey,
		signal,
	});

	let partialMessage: AssistantMessage | null = null;
	let addedPartial = false;

	for await (const event of response) {
		switch (event.type) {
			case "start":
				partialMessage = event.partial;
				context.messages.push(partialMessage);
				addedPartial = true;
				await emit({ type: "message_start", message: { ...partialMessage } });
				break;

			case "text_start":
			case "text_delta":
			case "text_end":
			case "thinking_start":
			case "thinking_delta":
			case "thinking_end":
			case "toolcall_start":
			case "toolcall_delta":
			case "toolcall_end":
				if (partialMessage) {
					partialMessage = event.partial;
					context.messages[context.messages.length - 1] = partialMessage;
					await emit({
						type: "message_update",
						assistantMessageEvent: event,
						message: { ...partialMessage },
					});
				}
				break;

			case "done":
			case "error": {
				const finalMessage = await response.result();
				// A provider can report an abort as an error when its signal checks race
				// the abort. Once the run signal is aborted, an error stop reason is an
				// abort artifact, not a provider failure - reclassify so cancellation is
				// not surfaced, retried, or compacted as an error.
				if (finalMessage.stopReason === "error" && signal?.aborted) {
					finalMessage.stopReason = "aborted";
				}
				if (addedPartial) {
					context.messages[context.messages.length - 1] = finalMessage;
				} else {
					context.messages.push(finalMessage);
				}
				if (!addedPartial) {
					await emit({ type: "message_start", message: { ...finalMessage } });
				}
				await emit({ type: "message_end", message: finalMessage });
				return finalMessage;
			}
		}
	}

	const finalMessage = await response.result();
	if (addedPartial) {
		context.messages[context.messages.length - 1] = finalMessage;
	} else {
		context.messages.push(finalMessage);
		await emit({ type: "message_start", message: { ...finalMessage } });
	}
	await emit({ type: "message_end", message: finalMessage });
	return finalMessage;
```
Source: ice:packages/agent/src/agent-loop.ts:310-378.
None: additional CLI flags, slash commands, keybindings, core settings keys, agent RPC commands, or persistent files beyond those explicitly specified. This entry concerns only the named in-process APIs/events.

### 6. Dependencies

`AGENT-001`, `AGENT-002`, `AGENT-006`, `AGENT-007`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

### 7. ice-rust today

rust:crates/ice-agent/src/agent_loop.rs:437-576; rust:crates/ice-agent/src/events.rs:49-64. Rust has corresponding event categories, terminal result selection and an EOF/result fallback, using Arc snapshots and owned context instead of shared mutable TS objects. Its consumer.result returns an AssistantMessage, not a Result. Terminal abort normalization and public stream-option differences remain separate entries; do not infer those from event-name parity.

### 8. Port notes

Target ice-agent unless explicitly identified as an ice-harness or ice-tools helper. Preserve one authoritative loop. Use owned snapshots and explicit async error/cancellation contracts; native plugin consumers require the same host-facing hook/event data, not a second controller. Existing workspace dependencies tokio, tokio-util, futures and serde_json cover the primitives.

### 9. Size and priority

Estimated Rust implementation: **90-160 lines** (estimate, not measured source). **P1** - Required for deterministic SDK, event and tool behavior during a TypeScript-to-Rust port.

## AGENT-009 - Terminal error-to-aborted reclassification

### 1. Title and status

Terminal error-to-aborted reclassification. **Status: missing.**

### 2. ice source

ice:packages/agent/src/agent-loop.ts:350-378. streamAssistantResponse. Test: ice:packages/agent/test/agent-loop.test.ts:1616-1658 (`reclassifies an error response as aborted when the run signal is already aborted`).

### 3. What it does

Do not report a provider error as an ordinary failure when the user already cancelled the run.

### 4. How it works

In the done/error event branch, await response.result; if finalMessage.stopReason is exactly error and the run AbortSignal is already aborted, mutate it to aborted before message_end and turn_end. Keep the provider errorMessage, usage and content unchanged. Do not convert stop/length/toolUse merely because cancellation happened. The EOF-with-result fallback does not apply this normalization, so the contract is narrower than “all errors after cancellation”. This changes downstream retry/notice classification and stops the loop through its normal aborted terminal path.

### 5. Interfaces

```ts
				// the abort. Once the run signal is aborted, an error stop reason is an
				// abort artifact, not a provider failure - reclassify so cancellation is
				// not surfaced, retried, or compacted as an error.
				if (finalMessage.stopReason === "error" && signal?.aborted) {
					finalMessage.stopReason = "aborted";
				}
				if (addedPartial) {
					context.messages[context.messages.length - 1] = finalMessage;
				} else {
					context.messages.push(finalMessage);
```
Source: ice:packages/agent/src/agent-loop.ts:350-359.
None: additional CLI flags, slash commands, keybindings, core settings keys, agent RPC commands, or persistent files beyond those explicitly specified. This entry concerns only the named in-process APIs/events.

### 6. Dependencies

`AGENT-008`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

### 7. ice-rust today

rust:crates/ice-agent/src/agent_loop.rs:515-576; rust:crates/ice-agent/src/abort.rs:1-94. The Rust stream reduction preserves the returned error stop reason. Abort helpers provide cancellation primitives but not this normalization. The separate post-tool LoopOutcome::Aborted exit is not equivalent to correcting the assistant message.

### 8. Port notes

Target ice-agent unless explicitly identified as an ice-harness or ice-tools helper. Preserve one authoritative loop. Use owned snapshots and explicit async error/cancellation contracts; native plugin consumers require the same host-facing hook/event data, not a second controller. Existing workspace dependencies tokio, tokio-util, futures and serde_json cover the primitives.

### 9. Size and priority

Estimated Rust implementation: **12-30 lines** (estimate, not measured source). **P1** - Cancellation must not be mistaken for a retryable provider failure or leave misleading persisted stop reasons.

## AGENT-010 - Whole-batch sequential override and truncated-call rejection

### 1. Title and status

Whole-batch sequential override and truncated-call rejection. **Status: partial.**

### 2. ice source

ice:packages/agent/src/agent-loop.ts:388-493; ice:packages/agent/src/types.ts:34-51. failToolCallsFromTruncatedMessage/executeToolCalls/executeToolCallsSequential. Tests: ice:packages/agent/test/agent-loop.test.ts:371-442 (`should not execute tool calls from a length-truncated assistant message`); ice:packages/agent/test/agent-loop.test.ts:870-955 (`should force sequential execution when one of multiple tools has executionMode=sequential`); ice:packages/agent/test/agent-loop.test.ts:957-1029 (`should allow parallel execution when all tools have executionMode=parallel`).

### 3. What it does

Protect truncated tool arguments and respect tools that require serial execution.

### 4. How it works

Collect every assistant content block whose type is toolCall in source order. If stopReason=length, never execute any of those calls, even if some arguments look complete. Emit start, an error execution-end, then tool-result message start/end for each, using the exact error below, and return terminate=false so the model can reissue. Missing/blocked tools are not consulted on this path.

For other nonterminal responses choose sequential when config.toolExecution=sequential OR any named tool in the current registry has executionMode=sequential; otherwise parallel. Unknown tool names do not force sequential. In sequential mode emit start, prepare, execute/finalize or use the immediate error, emit execution-end and message start/end before the next tool. After each finalized call, stop dispatching remaining calls if the signal is aborted; do not synthesize results for never-dispatched remainder. CurrentContext is not appended with batch results until the entire batch returns, although event consumers see sequential result messages immediately. No per-batch size or concurrency cap exists here.

### 5. Interfaces

```ts
async function failToolCallsFromTruncatedMessage(
	toolCalls: AgentToolCall[],
	emit: AgentEventSink,
): Promise<ExecutedToolCallBatch> {
	const messages: ToolResultMessage[] = [];
	for (const toolCall of toolCalls) {
		await emit({
			type: "tool_execution_start",
			toolCallId: toolCall.id,
			toolName: toolCall.name,
			args: toolCall.arguments,
		});
		const finalized: FinalizedToolCallOutcome = {
			toolCall,
			result: createErrorToolResult(
				`Tool call "${toolCall.name}" was not executed: the response hit the output token limit, so its arguments may be truncated. Re-issue the tool call with complete arguments.`,
			),
			isError: true,
		};
		await emitToolExecutionEnd(finalized, emit);
		const toolResultMessage = createToolResultMessage(finalized);
		await emitToolResultMessage(toolResultMessage, emit);
		messages.push(toolResultMessage);
	}
	return { messages, terminate: false };
}

/**
 * Execute tool calls from an assistant message.
 */
async function executeToolCalls(
	currentContext: AgentContext,
	assistantMessage: AssistantMessage,
	config: AgentLoopConfig,
	signal: AbortSignal | undefined,
	emit: AgentEventSink,
): Promise<ExecutedToolCallBatch> {
	const toolCalls = assistantMessage.content.filter((c) => c.type === "toolCall");
	const hasSequentialToolCall = toolCalls.some(
		(tc) => currentContext.tools?.find((t) => t.name === tc.name)?.executionMode === "sequential",
	);
	if (config.toolExecution === "sequential" || hasSequentialToolCall) {
		return executeToolCallsSequential(currentContext, assistantMessage, toolCalls, config, signal, emit);
	}
	return executeToolCallsParallel(currentContext, assistantMessage, toolCalls, config, signal, emit);
}
```
Source: ice:packages/agent/src/agent-loop.ts:388-433.
None: additional CLI flags, slash commands, keybindings, core settings keys, agent RPC commands, or persistent files beyond those explicitly specified. This entry concerns only the named in-process APIs/events.

### 6. Dependencies

`AGENT-012`, `AGENT-013`, `AGENT-014`, `AGENT-015`, `AGENT-016`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

### 7. ice-rust today

rust:crates/ice-agent/src/agent_loop.rs:581-688. Rust has the same batch-mode and length rejection structure; error details serialize as null instead of the TypeScript empty object, and common cancellation/update/finalization differences still apply. This entry is partial, not a missing scheduler. Sequential execution remains a whole-batch property, not a per-tool mutex.

### 8. Port notes

Target ice-agent unless explicitly identified as an ice-harness or ice-tools helper. Preserve one authoritative loop. Use owned snapshots and explicit async error/cancellation contracts; native plugin consumers require the same host-facing hook/event data, not a second controller. Existing workspace dependencies tokio, tokio-util, futures and serde_json cover the primitives.

### 9. Size and priority

Estimated Rust implementation: **45-90 lines** (estimate, not measured source). **P1** - Required for deterministic SDK, event and tool behavior during a TypeScript-to-Rust port.

## AGENT-011 - Parallel completion events during later permission preparation

### 1. Title and status

Parallel completion events during later permission preparation. **Status: partial.**

### 2. ice source

ice:packages/agent/src/agent-loop.ts:496-565. executeToolCallsParallel. Tests: ice:packages/agent/test/agent-loop.test.ts:586-679 (`should emit tool_execution_end in completion order but persist tool results in source order`); ice:packages/agent/test/agent-loop.test.ts:1031-1093 (`starts a prepared parallel tool while a later call still awaits beforeToolCall`).

### 3. What it does

Show a finished tool immediately even while a later tool is waiting for validation or user approval.

### 4. How it works

Prepare calls serially in assistant order. Emit tool_execution_start before each preparation. An immediate missing/blocked/aborted/invalid outcome emits execution-end immediately and becomes an already-resolved slot. A prepared call starts execute and finalize at once in an async closure; that closure awaits its execution-end sink before resolving. Later beforeToolCall work therefore does not delay earlier tool updates or end events. Attach a catch to running promises only to prevent temporary unhandled rejection while preparing later calls; Promise.all still observes failure.

After all dispatched calls settle, iterate Promise.all results in source order and emit tool-result message start/end. Completion events follow actual finalization/sink progress, transcript results follow source order. Cancellation stops starting further preparation after the current slot but does not abandon already-running promises; they receive the same signal and are awaited. There is no independent worker count, timeout or cross-tool isolation.

### 5. Interfaces

```ts
async function executeToolCallsParallel(
	currentContext: AgentContext,
	assistantMessage: AssistantMessage,
	toolCalls: AgentToolCall[],
	config: AgentLoopConfig,
	signal: AbortSignal | undefined,
	emit: AgentEventSink,
): Promise<ExecutedToolCallBatch> {
	const finalizedCalls: Promise<FinalizedToolCallOutcome>[] = [];

	// Preparation (validation and beforeToolCall, which may ask for approval) stays sequential and
	// in call order; each prepared call starts executing at once instead of waiting for later preparations.
	for (const toolCall of toolCalls) {
		await emit({
			type: "tool_execution_start",
			toolCallId: toolCall.id,
			toolName: toolCall.name,
			args: toolCall.arguments,
		});

		const preparation = await prepareToolCall(currentContext, assistantMessage, toolCall, config, signal);
		if (preparation.kind === "immediate") {
			const finalized = {
				toolCall,
				result: preparation.result,
				isError: preparation.isError,
			} satisfies FinalizedToolCallOutcome;
			await emitToolExecutionEnd(finalized, emit);
			finalizedCalls.push(Promise.resolve(finalized));
			if (signal?.aborted) {
				break;
			}
			continue;
		}

		const running = (async () => {
			const executed = await executePreparedToolCall(preparation, signal, emit);
			const finalized = await finalizeExecutedToolCall(
				currentContext,
				assistantMessage,
				preparation,
				executed,
				config,
				signal,
			);
			await emitToolExecutionEnd(finalized, emit);
			return finalized;
		})();
		// Promise.all below observes a failure; this only keeps it from being reported as
		// unhandled while later calls are still being prepared.
		running.catch(() => {});
		finalizedCalls.push(running);
		if (signal?.aborted) {
			break;
		}
	}

	const orderedFinalizedCalls = await Promise.all(finalizedCalls);
	const messages: ToolResultMessage[] = [];
	for (const finalized of orderedFinalizedCalls) {
		const toolResultMessage = createToolResultMessage(finalized);
		await emitToolResultMessage(toolResultMessage, emit);
		messages.push(toolResultMessage);
	}

	return {
		messages,
		terminate: shouldTerminateToolBatch(orderedFinalizedCalls),
	};
}
```
Source: ice:packages/agent/src/agent-loop.ts:496-565.
None: additional CLI flags, slash commands, keybindings, core settings keys, agent RPC commands, or persistent files beyond those explicitly specified. This entry concerns only the named in-process APIs/events.

### 6. Dependencies

`AGENT-010`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

### 7. ice-rust today

rust:crates/ice-agent/src/agent_loop.rs:700-877. Rust spawns execute/finalize while preparing later calls, but the task does not emit execution-end. End events are collected only after the preparation loop finishes (lines 745-756 and 758 onward). If several handles finished during that wait, the is_finished scan emits them in source-index order, not their original completion order. Panics also lose original call identity through a <panic> placeholder. Preserve identity and emit finalization events inside tasks or a completion channel that is drained during preparation.

### 8. Port notes

Target ice-agent unless explicitly identified as an ice-harness or ice-tools helper. Preserve one authoritative loop. Use owned snapshots and explicit async error/cancellation contracts; native plugin consumers require the same host-facing hook/event data, not a second controller. Existing workspace dependencies tokio, tokio-util, futures and serde_json cover the primitives.

### 9. Size and priority

Estimated Rust implementation: **130-220 lines** (estimate, not measured source). **P0** - Approval waits must not hide completed work or break lifecycle ordering relied on by UI, observability and persistence.

## AGENT-012 - Argument preparation, validation and trusted before-tool hook

### 1. Title and status

Argument preparation, validation and trusted before-tool hook. **Status: partial.**

### 2. ice source

ice:packages/agent/src/agent-loop.ts:595-673; ice:packages/agent/src/types.ts:93-119. prepareToolCallArguments/prepareToolCall. Tests: ice:packages/agent/test/agent-loop.test.ts:444-504 (`should execute mutated beforeToolCall args without revalidation`); ice:packages/agent/test/agent-loop.test.ts:506-584 (`should prepare tool arguments for validation`).

### 3. What it does

Validate model arguments once, then allow a trusted host hook to inspect, block or deliberately rewrite the validated value.

### 4. How it works

Find the first registry tool whose name exactly equals toolCall.name; missing returns Tool <name> not found with no hook. Run optional synchronous prepareArguments(raw arguments). If it returns the same object retain the original call; otherwise create a call copy with the prepared arguments only for validation. Pass it to validateToolArguments. Then invoke beforeToolCall with original assistantMessage, original raw toolCall, validated args and currentContext, plus signal. A hook can mutate args in place and execution receives that mutation without revalidation. The raw toolCall stays distinct unless shared references were mutated.

After awaiting the hook, cancellation wins over a block decision. A truthy block emits an error using reason when nonempty/truthy, else Tool execution was blocked. There is no permission prompt in the loop itself and a missing hook means no additional policy gate. Preparation/validation/hook exceptions become immediate error results; afterToolCall is not invoked for these immediate paths. The AI validator’s schema/coercion contract is a separate dependency, not implied by a substring match.

### 5. Interfaces

```ts
function prepareToolCallArguments(tool: AgentTool<any>, toolCall: AgentToolCall): AgentToolCall {
	if (!tool.prepareArguments) {
		return toolCall;
	}
	const preparedArguments = tool.prepareArguments(toolCall.arguments);
	if (preparedArguments === toolCall.arguments) {
		return toolCall;
	}
	return {
		...toolCall,
		arguments: preparedArguments as Record<string, any>,
	};
}

async function prepareToolCall(
	currentContext: AgentContext,
	assistantMessage: AssistantMessage,
	toolCall: AgentToolCall,
	config: AgentLoopConfig,
	signal: AbortSignal | undefined,
): Promise<PreparedToolCall | ImmediateToolCallOutcome> {
	const tool = currentContext.tools?.find((t) => t.name === toolCall.name);
	if (!tool) {
		return {
			kind: "immediate",
			result: createErrorToolResult(`Tool ${toolCall.name} not found`),
			isError: true,
		};
	}

	try {
		const preparedToolCall = prepareToolCallArguments(tool, toolCall);
		const validatedArgs = validateToolArguments(tool, preparedToolCall);
		if (config.beforeToolCall) {
			const beforeResult = await config.beforeToolCall(
				{
					assistantMessage,
					toolCall,
					args: validatedArgs,
					context: currentContext,
				},
				signal,
			);
			if (signal?.aborted) {
				return {
					kind: "immediate",
					result: createErrorToolResult("Operation aborted"),
					isError: true,
				};
			}
			if (beforeResult?.block) {
				return {
					kind: "immediate",
					result: createErrorToolResult(beforeResult.reason || "Tool execution was blocked"),
					isError: true,
				};
			}
		}
		if (signal?.aborted) {
			return {
				kind: "immediate",
				result: createErrorToolResult("Operation aborted"),
				isError: true,
			};
		}
		return {
			kind: "prepared",
			toolCall,
			tool,
			args: validatedArgs,
		};
	} catch (error) {
		return {
			kind: "immediate",
			result: createErrorToolResult(error instanceof Error ? error.message : String(error)),
			isError: true,
		};
	}
}
```
Source: ice:packages/agent/src/agent-loop.ts:595-673.
```ts
export interface BeforeToolCallResult {
	block?: boolean;
	reason?: string;
}
```
Source: ice:packages/agent/src/types.ts:61-64.
None: additional CLI flags, slash commands, keybindings, core settings keys, agent RPC commands, or persistent files beyond those explicitly specified. This entry concerns only the named in-process APIs/events.

### 6. Dependencies

`AGENT-001`, `AGENT-016`, `AI-012`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

### 7. ice-rust today

rust:crates/ice-agent/src/agent_loop.rs:922-1023; rust:crates/ice-agent/src/types.rs:105-152; rust:crates/ice-agent/src/agent_tool.rs:43-47. Rust represents mutation by returned args, which correctly bypasses revalidation, but supplies the prepared call rather than original raw call to the before hook. Some("") block reason is retained rather than using the fallback; before hooks also have a Rust-only terminate result. Hooks return infallible Option futures, so TypeScript exception-to-result behavior has no direct Result channel. Keep trusted overrides separate from untrusted model input.

### 8. Port notes

Target ice-agent unless explicitly identified as an ice-harness or ice-tools helper. Preserve one authoritative loop. Use owned snapshots and explicit async error/cancellation contracts; native plugin consumers require the same host-facing hook/event data, not a second controller. Existing workspace dependencies tokio, tokio-util, futures and serde_json cover the primitives.

### 9. Size and priority

Estimated Rust implementation: **90-160 lines** (estimate, not measured source). **P0** - Changing the raw-versus-validated boundary can invalidate permission matching and audit evidence.

## AGENT-013 - Final cancellation check immediately before tool execution

### 1. Title and status

Final cancellation check immediately before tool execution. **Status: partial.**

### 2. ice source

ice:packages/agent/src/agent-loop.ts:628-665; ice:packages/agent/src/agent-loop.ts:675-719; ice:packages/agent/src/agent-loop.ts:722-766. executePreparedToolCall. Tests: abort behavior in ice:packages/agent/test/e2e.test.ts:207-218 (`handles abort during streaming`); no dedicated test isolates the microtask gap; the guard is explicit source.

### 3. What it does

Close the asynchronous gap between successful preparation and dispatch of a side-effecting tool.

### 4. How it works

prepareToolCall checks abort after an awaited before hook and again before returning prepared. executePreparedToolCall checks the same signal once more at its own entry because awaiting preparation yields to the event loop. If cancelled, return Operation aborted/isError=true without calling tool.execute or generating tool updates. It still enters afterToolCall through the executed-outcome path; this is different from cancellation detected during preparation, whose immediate result bypasses afterToolCall. Cancellation after execute begins is cooperative: pass the signal, await the tool, and do not automatically undo or replay it.

### 5. Interfaces

```ts
async function executePreparedToolCall(
	prepared: PreparedToolCall,
	signal: AbortSignal | undefined,
	emit: AgentEventSink,
): Promise<ExecutedToolCallOutcome> {
	// Preparation yields to the event loop; cancellation may arrive after its last check.
	if (signal?.aborted) {
		return { result: createErrorToolResult("Operation aborted"), isError: true };
	}
	const updateEvents: Promise<void>[] = [];
	let acceptingUpdates = true;

```
Source: ice:packages/agent/src/agent-loop.ts:675-686.
None: additional CLI flags, slash commands, keybindings, core settings keys, agent RPC commands, or persistent files beyond those explicitly specified. This entry concerns only the named in-process APIs/events.

### 6. Dependencies

`AGENT-012`, `AGENT-016`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

### 7. ice-rust today

rust:crates/ice-agent/src/agent_loop.rs:979-1069. Rust checks cancellation in preparation but execute_prepared_tool_call begins by installing update state and calls execute without an equivalent entry recheck. A cancellation arriving between those stages may still invoke a mutating tool, even though it receives a cancelled token. Require the host-side dispatch check as well as cooperative tool cancellation.

### 8. Port notes

Target ice-agent unless explicitly identified as an ice-harness or ice-tools helper. Preserve one authoritative loop. Use owned snapshots and explicit async error/cancellation contracts; native plugin consumers require the same host-facing hook/event data, not a second controller. Existing workspace dependencies tokio, tokio-util, futures and serde_json cover the primitives.

### 9. Size and priority

Estimated Rust implementation: **15-35 lines** (estimate, not measured source). **P0** - A cancelled operation must not cross the tool side-effect boundary merely because preparation already succeeded.

## AGENT-014 - Tool update acknowledgement, late-update suppression and raw arguments

### 1. Title and status

Tool update acknowledgement, late-update suppression and raw arguments. **Status: partial.**

### 2. ice source

ice:packages/agent/src/agent-loop.ts:684-719. executePreparedToolCall update callback. Tests: ice:packages/agent/test/agent.test.ts:301-364 (`should ignore tool updates after the tool execution settles`); ice:packages/agent/test/agent.test.ts:366-440 (`should ignore a settled parallel tool update while another tool is still running`).

### 3. What it does

Keep all accepted progress updates ordered before a tool’s final event and ignore callbacks after execute settles.

### 4. How it works

Each prepared execution creates acceptingUpdates=true and an array of sink promises. The supplied callback ignores calls after the gate closes; while open it invokes the sink immediately with tool_execution_update, original call ID/name/raw arguments and partialResult, wraps its return in Promise.resolve and retains the promise. After execute resolves or throws set the gate false, await all retained update promises, then return the success/error outcome. finally also closes the gate. No rate limit, count cap or payload cap is imposed.

A sink rejection can propagate: the catch path awaits the same failed promises again. A callback that throws synchronously while invoking the sink can become a tool execution error. Concurrent tools have separate gates and can interleave sink calls; this is not a global serialized event queue. Accepting an update means its asynchronous consumers finish before execution-end, not merely enqueueing a broadcast. Raw arguments in update events intentionally differ from prepared/mutated execute params.

### 5. Interfaces

```ts
	const updateEvents: Promise<void>[] = [];
	let acceptingUpdates = true;

	try {
		const result = await prepared.tool.execute(
			prepared.toolCall.id,
			prepared.args as never,
			signal,
			(partialResult) => {
				if (!acceptingUpdates) return;
				updateEvents.push(
					Promise.resolve(
						emit({
							type: "tool_execution_update",
							toolCallId: prepared.toolCall.id,
							toolName: prepared.toolCall.name,
							args: prepared.toolCall.arguments,
							partialResult,
						}),
					),
				);
			},
		);
		acceptingUpdates = false;
		await Promise.all(updateEvents);
		return { result, isError: false };
	} catch (error) {
		acceptingUpdates = false;
		await Promise.all(updateEvents);
		return {
			result: createErrorToolResult(error instanceof Error ? error.message : String(error)),
			isError: true,
		};
	} finally {
		acceptingUpdates = false;
	}
```
Source: ice:packages/agent/src/agent-loop.ts:684-719.
None: additional CLI flags, slash commands, keybindings, core settings keys, agent RPC commands, or persistent files beyond those explicitly specified. This entry concerns only the named in-process APIs/events.

### 6. Dependencies

`AGENT-001`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

### 7. ice-rust today

rust:crates/ice-agent/src/agent_loop.rs:1025-1069; rust:crates/ice-agent/src/events.rs:113-165. Rust uses an AtomicBool gate and synchronous try_emit with prepared args. It does not await downstream asynchronous update processing; the default try_emit implementation drops updates entirely. Native task-local ToolExecutionScope tracks other host callbacks but is not an acknowledgement for this update stream. Add an owned per-execution update drain/acknowledgement mechanism without holding registry locks across callbacks.

### 8. Port notes

Target ice-agent unless explicitly identified as an ice-harness or ice-tools helper. Preserve one authoritative loop. Use owned snapshots and explicit async error/cancellation contracts; native plugin consumers require the same host-facing hook/event data, not a second controller. Existing workspace dependencies tokio, tokio-util, futures and serde_json cover the primitives.

### 9. Size and priority

Estimated Rust implementation: **110-200 lines** (estimate, not measured source). **P0** - Final events must not overtake accepted updates or expose different arguments to policy/telemetry consumers.

## AGENT-015 - After-tool override and exception conversion

### 1. Title and status

After-tool override and exception conversion. **Status: partial.**

### 2. ice source

ice:packages/agent/src/types.ts:79-119; ice:packages/agent/src/agent-loop.ts:722-766. finalizeExecutedToolCall. Tests: ice:packages/agent/test/agent-loop.test.ts:1442-1488 (`should allow afterToolCall to mark a tool batch as terminating`).

### 3. What it does

Let trusted hooks inspect executed outcomes and replace selected result fields before any final event or persisted tool message.

### 4. How it works

Only a prepared/executed path reaches afterToolCall, including execute exceptions converted to an error and cancellation caught at the execute-entry guard. Supply original assistant/raw call, final prepared args, result, isError and currentContext with signal. If the hook returns a value, spread the original result then replace content, details, usage and terminate using nullish fallback; isError also uses nullish fallback. Empty content arrays and false are meaningful overrides, null/undefined do not erase a field. Preserve addedToolNames and any untyped extra original properties. No deep merge or schema validation follows. A hook exception replaces the entire result with a text error/details={} and sets isError=true, losing prior usage/termination. There is no retry of the tool or hook.

### 5. Interfaces

```ts
export interface AfterToolCallResult {
	content?: (TextContent | ImageContent)[];
	details?: unknown;
	isError?: boolean;
	/** Usage from the final tool execution itself, if available. Not used for main LLM context accounting. */
	usage?: Usage;
	/**
	 * Hint that the agent should stop after the current tool batch.
	 * Early termination only happens when every finalized tool result in the batch sets this to true.
	 */
	terminate?: boolean;
}

/** Context passed to `beforeToolCall`. */
export interface BeforeToolCallContext {
	/** The assistant message that requested the tool call. */
	assistantMessage: AssistantMessage;
	/** The raw tool call block from `assistantMessage.content`. */
	toolCall: AgentToolCall;
	/** Validated tool arguments for the target tool schema. */
	args: unknown;
	/** Current agent context at the time the tool call is prepared. */
	context: AgentContext;
}

/** Context passed to `afterToolCall`. */
export interface AfterToolCallContext {
	/** The assistant message that requested the tool call. */
	assistantMessage: AssistantMessage;
	/** The raw tool call block from `assistantMessage.content`. */
	toolCall: AgentToolCall;
	/** Validated tool arguments for the target tool schema. */
	args: unknown;
	/** The executed tool result before any `afterToolCall` overrides are applied. */
	result: AgentToolResult<any>;
	/** Whether the executed tool result is currently treated as an error. */
	isError: boolean;
	/** Current agent context at the time the tool call is finalized. */
	context: AgentContext;
}

```
Source: ice:packages/agent/src/types.ts:79-119.
```ts
async function finalizeExecutedToolCall(
	currentContext: AgentContext,
	assistantMessage: AssistantMessage,
	prepared: PreparedToolCall,
	executed: ExecutedToolCallOutcome,
	config: AgentLoopConfig,
	signal: AbortSignal | undefined,
): Promise<FinalizedToolCallOutcome> {
	let result = executed.result;
	let isError = executed.isError;

	if (config.afterToolCall) {
		try {
			const afterResult = await config.afterToolCall(
				{
					assistantMessage,
					toolCall: prepared.toolCall,
					args: prepared.args,
					result,
					isError,
					context: currentContext,
				},
				signal,
			);
			if (afterResult) {
				result = {
					...result,
					content: afterResult.content ?? result.content,
					details: afterResult.details ?? result.details,
					usage: afterResult.usage ?? result.usage,
					terminate: afterResult.terminate ?? result.terminate,
				};
				isError = afterResult.isError ?? isError;
			}
		} catch (error) {
			result = createErrorToolResult(error instanceof Error ? error.message : String(error));
			isError = true;
		}
	}

	return {
		toolCall: prepared.toolCall,
		result,
		isError,
	};
```
Source: ice:packages/agent/src/agent-loop.ts:722-766.
None: additional CLI flags, slash commands, keybindings, core settings keys, agent RPC commands, or persistent files beyond those explicitly specified. This entry concerns only the named in-process APIs/events.

### 6. Dependencies

`AGENT-001`, `AGENT-016`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

### 7. ice-rust today

rust:crates/ice-agent/src/agent_loop.rs:1071-1119; rust:crates/ice-agent/src/types.rs:124-152; rust:crates/ice-agent/src/hooks.rs:82-91. Rust has field-by-field optional overrides but Some(JSON null) can replace details whereas TypeScript null retains the original. The hook type has no error return and a panic is not converted equivalently; parallel task panic eventually yields a placeholder call, sequential panic can unwind. Provide explicit hook errors and preserve call identity. Do not weaken denial by treating a failed hook as success.

### 8. Port notes

Target ice-agent unless explicitly identified as an ice-harness or ice-tools helper. Preserve one authoritative loop. Use owned snapshots and explicit async error/cancellation contracts; native plugin consumers require the same host-facing hook/event data, not a second controller. Existing workspace dependencies tokio, tokio-util, futures and serde_json cover the primitives.

### 9. Size and priority

Estimated Rust implementation: **75-140 lines** (estimate, not measured source). **P0** - Post-tool policy and result sanitation need deterministic failure semantics and stable call identity.

## AGENT-016 - Tool-result normalization, empty content and additional tool discovery

### 1. Title and status

Tool-result normalization, empty content and additional tool discovery. **Status: partial.**

### 2. ice source

ice:packages/agent/src/agent-loop.ts:769-805; ice:packages/agent/src/types.ts:355-378. createErrorToolResult/createToolResultMessage/emitToolResultMessage. Tests: ice:packages/agent/test/agent-loop.test.ts:274-369 (`should handle tool calls and results`); missing-content fallback is explicit source, with no dedicated assertion in the inspected agent tests.

### 3. What it does

Persist a model-visible result separately from the richer execution result without letting missing content enter history.

### 4. How it works

createErrorToolResult produces exactly one text block and details={}, with no usage or terminate property. Execution-end carries the full result plus isError. createToolResultMessage keeps call ID/name, uses result.content??[] so untyped JavaScript tools with missing/null content become an empty array, copies details/usage, includes addedToolNames only when its length is truthy, adds isError and Date.now timestamp. It does not persist terminate or arbitrary other result properties. Emit message_start then message_end for the result. No tool result schema is revalidated here; callers can return malformed nonnull content. Empty/missing content is a compatibility normalization, not a guarantee of safe text.

### 5. Interfaces

```ts
function createErrorToolResult(message: string): AgentToolResult<any> {
	return {
		content: [{ type: "text", text: message }],
		details: {},
	};
}

async function emitToolExecutionEnd(finalized: FinalizedToolCallOutcome, emit: AgentEventSink): Promise<void> {
	await emit({
		type: "tool_execution_end",
		toolCallId: finalized.toolCall.id,
		toolName: finalized.toolCall.name,
		result: finalized.result,
		isError: finalized.isError,
	});
}

function createToolResultMessage(finalized: FinalizedToolCallOutcome): ToolResultMessage {
	return {
		role: "toolResult",
		toolCallId: finalized.toolCall.id,
		toolName: finalized.toolCall.name,
		// Untyped tools (JS extensions) can return results without content; normalize
		// so the null never enters session history or provider payloads.
		content: finalized.result.content ?? [],
		details: finalized.result.details,
		usage: finalized.result.usage,
		...(finalized.result.addedToolNames?.length ? { addedToolNames: finalized.result.addedToolNames } : {}),
		isError: finalized.isError,
		timestamp: Date.now(),
	};
}

async function emitToolResultMessage(toolResultMessage: ToolResultMessage, emit: AgentEventSink): Promise<void> {
	await emit({ type: "message_start", message: toolResultMessage });
	await emit({ type: "message_end", message: toolResultMessage });
}
```
Source: ice:packages/agent/src/agent-loop.ts:769-805.
None: additional CLI flags, slash commands, keybindings, core settings keys, agent RPC commands, or persistent files beyond those explicitly specified. This entry concerns only the named in-process APIs/events.

### 6. Dependencies

`AGENT-001`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

### 7. ice-rust today

rust:crates/ice-agent/src/agent_loop.rs:1265-1294; rust:crates/ice-agent/src/types.rs:37-92. Rust native result content is always a vector and details defaults to JSON null; optional usage/additional names have corresponding fields. The native C-ABI JSON decoder must normalize omitted/null content at its boundary to achieve TypeScript behavior. Current low-level errors use null rather than {}; this is observable in JSON snapshots and hook payloads.

### 8. Port notes

Target ice-agent unless explicitly identified as an ice-harness or ice-tools helper. Preserve one authoritative loop. Use owned snapshots and explicit async error/cancellation contracts; native plugin consumers require the same host-facing hook/event data, not a second controller. Existing workspace dependencies tokio, tokio-util, futures and serde_json cover the primitives.

### 9. Size and priority

Estimated Rust implementation: **50-95 lines** (estimate, not measured source). **P1** - Required for deterministic SDK, event and tool behavior during a TypeScript-to-Rust port.

## AGENT-017 - All-results termination vote

### 1. Title and status

All-results termination vote. **Status: partial.**

### 2. ice source

ice:packages/agent/src/agent-loop.ts:388-413; ice:packages/agent/src/agent-loop.ts:490-493; ice:packages/agent/src/agent-loop.ts:561-563; ice:packages/agent/src/agent-loop.ts:591-593; ice:packages/agent/src/agent-loop.ts:206-267. shouldTerminateToolBatch. Tests: ice:packages/agent/test/agent-loop.test.ts:1325-1375 (`should stop after a tool batch when every tool result sets terminate=true`); ice:packages/agent/test/agent-loop.test.ts:1377-1440 (`should continue after parallel tool calls when not all tool results terminate`); ice:packages/agent/test/agent-loop.test.ts:1442-1488 (`should allow afterToolCall to mark a tool batch as terminating`).

### 3. What it does

Stop automatic tool continuation only when every dispatched result explicitly requests termination.

### 4. How it works

For sequential or parallel finalized calls, terminate is true exactly when the list is nonempty and every final result.terminate===true. No truthy coercion: an untyped 1 or string true does not count. Evaluate after afterToolCall overrides; an error can vote true if a hook explicitly sets it. An immediate not-found/blocked/validation/aborted result has no terminate and prevents the vote. A partial sequential batch cut short by cancellation votes over only finalized calls. The vote suppresses the tool-driven next turn but does not suppress pending steering or follow-up queues. A length-truncated batch explicitly returns false.

### 5. Interfaces

```ts
function shouldTerminateToolBatch(finalizedCalls: FinalizedToolCallOutcome[]): boolean {
	return finalizedCalls.length > 0 && finalizedCalls.every((finalized) => finalized.result.terminate === true);
}
```
Source: ice:packages/agent/src/agent-loop.ts:591-593.
None: additional CLI flags, slash commands, keybindings, core settings keys, agent RPC commands, or persistent files beyond those explicitly specified. This entry concerns only the named in-process APIs/events.

### 6. Dependencies

`AGENT-016`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

### 7. ice-rust today

rust:crates/ice-agent/src/agent_loop.rs:684-687; rust:crates/ice-agent/src/agent_loop.rs:832-835; rust:crates/ice-agent/src/types.rs:105-122. Rust has the same nonempty/all-results vote for executed results, but before_tool_call can attach terminate to a blocked immediate result, an authority not present in TypeScript. Its post-tool cancellation exit can bypass queue continuation independently of the vote. Native bool types avoid untyped truthiness differences.

### 8. Port notes

Target ice-agent unless explicitly identified as an ice-harness or ice-tools helper. Preserve one authoritative loop. Use owned snapshots and explicit async error/cancellation contracts; native plugin consumers require the same host-facing hook/event data, not a second controller. Existing workspace dependencies tokio, tokio-util, futures and serde_json cover the primitives.

### 9. Size and priority

Estimated Rust implementation: **30-60 lines** (estimate, not measured source). **P1** - Required for deterministic SDK, event and tool behavior during a TypeScript-to-Rust port.

## AGENT-018 - Atomic next-turn context, model and thinking replacement

### 1. Title and status

Atomic next-turn context, model and thinking replacement. **Status: partial.**

### 2. ice source

ice:packages/agent/src/agent-loop.ts:224-246; ice:packages/agent/src/types.ts:121-142. prepareNextTurn/AgentLoopTurnUpdate. Test: ice:packages/agent/test/agent-loop.test.ts:1155-1226 (`should use prepareNextTurn snapshot before continuing`).

### 3. What it does

Apply a host-provided runtime snapshot before the next tool-following or queued-message turn.

### 4. How it works

After turn_end call prepareNextTurn with current assistant, ordered tool results, current context and all newMessages. Await a result. Replace currentContext if result.context is supplied. If model or thinkingLevel is not undefined, copy config and replace model with result.model??old; preserve reasoning when thinkingLevel is undefined, clear reasoning when off, otherwise use the selected level. Do not change the already-selected stream function. The following shouldStopAfterTurn sees the updated context, and any next request uses the new model/provider and dynamic key lookup. No persistent model-change entry or Agent public-state mutation is automatically written by this raw hook. A returned invalid model/context is trusted host data and is not validated here. Failure propagates, with no fallback snapshot.

### 5. Interfaces

```ts
				message,
				toolResults,
				context: currentContext,
				newMessages,
			};
			const nextTurnSnapshot = await config.prepareNextTurn?.(nextTurnContext);
			if (nextTurnSnapshot) {
				currentContext = nextTurnSnapshot.context ?? currentContext;
				config = {
					...config,
					model: nextTurnSnapshot.model ?? config.model,
					reasoning:
						nextTurnSnapshot.thinkingLevel === undefined
							? config.reasoning
							: nextTurnSnapshot.thinkingLevel === "off"
								? undefined
								: nextTurnSnapshot.thinkingLevel,
				};
			}
```
Source: ice:packages/agent/src/agent-loop.ts:227-245.
```ts
export interface ShouldStopAfterTurnContext {
	/** The assistant message that completed the turn. */
	message: AssistantMessage;
	/** Tool result messages passed to the preceding `turn_end` event. */
	toolResults: ToolResultMessage[];
	/** Current agent context after the turn's assistant message and tool results have been appended. */
	context: AgentContext;
	/** Messages that this loop invocation will return if it exits at this point. Prompt runs include the initial prompt messages; continuation runs do not include pre-existing context messages. */
	newMessages: AgentMessage[];
}

/** Replacement runtime state used by the agent loop before starting another provider request. */
export interface AgentLoopTurnUpdate {
	/** Context for the next provider request. */
	context?: AgentContext;
	/** Model for the next provider request. */
	model?: Model<any>;
	/** Thinking level for the next provider request. */
	thinkingLevel?: ThinkingLevel;
}

export interface PrepareNextTurnContext extends ShouldStopAfterTurnContext {}
```
Source: ice:packages/agent/src/types.ts:121-142.
None: additional CLI flags, slash commands, keybindings, core settings keys, agent RPC commands, or persistent files beyond those explicitly specified. This entry concerns only the named in-process APIs/events.

### 6. Dependencies

`AGENT-007`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

### 7. ice-rust today

rust:crates/ice-agent/src/types.rs:197-204; rust:crates/ice-agent/src/agent_loop.rs:360-375. Rust declares all three fields but applies only context. Neither the raw loop nor Agent implements the comment’s claimed ownership of the ignored model/thinking values; Agent does not expose the hook at all. Native AgentHarness config sets prepare_next_turn=None. Implement immutable-per-turn config replacement after the hook, preserving current callback/stream handles.

### 8. Port notes

Target ice-agent unless explicitly identified as an ice-harness or ice-tools helper. Preserve one authoritative loop. Use owned snapshots and explicit async error/cancellation contracts; native plugin consumers require the same host-facing hook/event data, not a second controller. Existing workspace dependencies tokio, tokio-util, futures and serde_json cover the primitives.

### 9. Size and priority

Estimated Rust implementation: **65-125 lines** (estimate, not measured source). **P0** - A requested mode/model transition must actually affect the next provider request and its credential route.

## AGENT-019 - Stop-after-turn gate before queue consumption

### 1. Title and status

Stop-after-turn gate before queue consumption. **Status: partial.**

### 2. ice source

ice:packages/agent/src/agent-loop.ts:247-261; ice:packages/agent/src/agent.ts:465-482. shouldStopAfterTurn. Tests: ice:packages/agent/test/agent-loop.test.ts:1228-1323 (`should stop after the current turn when shouldStopAfterTurn returns true`); ice:packages/agent/test/agent.test.ts:759-801 (`forwards shouldStopAfterTurn through AgentOptions`).

### 3. What it does

Allow the host to pause after a settled turn without losing messages queued for a later run.

### 4. How it works

Invoke shouldStopAfterTurn after preparation and before steering/follow-up drains. Pass assistant, ordered tool results, the possibly replaced current context and newMessages. Await boolean; when true emit agent_end(newMessages) and return. No extra assistant text or turn_end is synthesized. Queues remain untouched, including messages queued during a long tool. Error/aborted assistant responses exit earlier and never invoke this hook. The Agent wrapper captures the shouldStopAfterTurn function when building its run config and supplies its active signal; changing that field during the run does not replace the captured function.

### 5. Interfaces

```ts
			if (
				await config.shouldStopAfterTurn?.({
					message,
					toolResults,
					context: currentContext,
					newMessages,
				})
			) {
				await emit({ type: "agent_end", messages: newMessages });
				return;
			}

			pendingMessages = (await config.getSteeringMessages?.()) || [];
		}

```
Source: ice:packages/agent/src/agent-loop.ts:247-261.
```ts
	private createLoopConfig(options: { skipInitialSteeringPoll?: boolean } = {}): AgentLoopConfig {
		let skipInitialSteeringPoll = options.skipInitialSteeringPoll === true;
		const shouldStopAfterTurn = this.shouldStopAfterTurn;
		return {
			model: this._state.model,
			reasoning: this._state.thinkingLevel === "off" ? undefined : this._state.thinkingLevel,
			sessionId: this.sessionId,
			onPayload: this.onPayload,
			onResponse: this.onResponse,
			transport: this.transport,
			thinkingBudgets: this.thinkingBudgets,
			maxRetryDelayMs: this.maxRetryDelayMs,
			toolExecution: this.toolExecution,
			beforeToolCall: this.beforeToolCall,
			afterToolCall: this.afterToolCall,
			shouldStopAfterTurn: shouldStopAfterTurn
				? async (context) => await shouldStopAfterTurn(context, this.signal)
				: undefined,
```
Source: ice:packages/agent/src/agent.ts:465-482.
None: additional CLI flags, slash commands, keybindings, core settings keys, agent RPC commands, or persistent files beyond those explicitly specified. This entry concerns only the named in-process APIs/events.

### 6. Dependencies

`AGENT-004`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

### 7. ice-rust today

rust:crates/ice-agent/src/hooks.rs:43-46; rust:crates/ice-agent/src/agent_loop.rs:377-394; rust:crates/ice-agent/src/agent.rs:510-541. The raw Rust gate and order exist, but the public Agent wrapper does not expose/forward it. Native AgentHarness has other stop controls; those do not establish this SDK callback contract. Preserve pending queues rather than draining them before returning.

### 8. Port notes

Target ice-agent unless explicitly identified as an ice-harness or ice-tools helper. Preserve one authoritative loop. Use owned snapshots and explicit async error/cancellation contracts; native plugin consumers require the same host-facing hook/event data, not a second controller. Existing workspace dependencies tokio, tokio-util, futures and serde_json cover the primitives.

### 9. Size and priority

Estimated Rust implementation: **35-75 lines** (estimate, not measured source). **P1** - Required for deterministic SDK, event and tool behavior during a TypeScript-to-Rust port.

## AGENT-020 - Mutable Agent state, snapshots and reducer visibility

### 1. Title and status

Mutable Agent state, snapshots and reducer visibility. **Status: different-by-design.**

### 2. ice source

ice:packages/agent/src/agent.ts:39-95; ice:packages/agent/src/agent.ts:249-289; ice:packages/agent/src/agent.ts:564-610. Agent.state/createMutableAgentState/processEvents. Tests: ice:packages/agent/test/agent.test.ts:106-119 (`should create an agent instance with default state`); ice:packages/agent/test/agent.test.ts:121-135 (`should create an agent instance with custom initial state`); ice:packages/agent/test/agent.test.ts:442-479 (`should update state with mutators`); ice:packages/agent/test/e2e.test.ts:192-205 (`executes tools and tracks pending tool calls`).

### 3. What it does

Expose current transcript/model/tools and runtime status, with mutations visible before event listeners run.

### 4. How it works

Initialize systemPrompt="", model=the exact unknown placeholder, thinkingLevel=off, tools/messages=[], streaming=false, no streaming/error message and an empty pending-tool Set, overridden by supplied initial configuration. Copy tools/messages on assignment and initialization but expose the internal arrays on reads; elements and models are not deep-cloned. Public state assignment therefore changes future snapshots and direct array mutation is possible.

Reducer sets streamingMessage for any message_start/update, clears and appends on message_end, replaces the pending Set with a copied add/delete set at tool start/end, and sets errorMessage only for assistant turn_end with a truthy error string. agent_end clears streamingMessage but does not mark idle. Reducer changes occur before subscribers. Run start clears prior error and sets streaming; final lifecycle cleanup clears pending state. No filesystem/session persistence is provided by Agent itself.

### 5. Interfaces

```ts
const EMPTY_USAGE = {
	input: 0,
	output: 0,
	cacheRead: 0,
	cacheWrite: 0,
	totalTokens: 0,
	cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
};

const DEFAULT_MODEL = {
	id: "unknown",
	name: "unknown",
	api: "unknown",
	provider: "unknown",
	baseUrl: "",
	reasoning: false,
	input: [],
	cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
	contextWindow: 0,
	maxTokens: 0,
} satisfies Model<any>;

type MutableAgentState = Omit<AgentState, "isStreaming" | "streamingMessage" | "pendingToolCalls" | "errorMessage"> & {
	isStreaming: boolean;
	streamingMessage?: AgentMessage;
	pendingToolCalls: Set<string>;
	errorMessage?: string;
};

function createMutableAgentState(
	initialState?: Partial<Omit<AgentState, "pendingToolCalls" | "isStreaming" | "streamingMessage" | "errorMessage">>,
): MutableAgentState {
	let tools = initialState?.tools?.slice() ?? [];
	let messages = initialState?.messages?.slice() ?? [];

	return {
		systemPrompt: initialState?.systemPrompt ?? "",
		model: initialState?.model ?? DEFAULT_MODEL,
		thinkingLevel: initialState?.thinkingLevel ?? "off",
		get tools() {
			return tools;
		},
		set tools(nextTools: AgentTool<any>[]) {
			tools = nextTools.slice();
		},
		get messages() {
			return messages;
		},
		set messages(nextMessages: AgentMessage[]) {
			messages = nextMessages.slice();
		},
		isStreaming: false,
		streamingMessage: undefined,
		pendingToolCalls: new Set<string>(),
		errorMessage: undefined,
	};
}
```
Source: ice:packages/agent/src/agent.ts:39-95.
```ts
export interface AgentState {
	/** System prompt sent with each model request. */
	systemPrompt: string;
	/** Active model used for future turns. */
	model: Model<any>;
	/** Requested reasoning level for future turns. */
	thinkingLevel: ThinkingLevel;
	/** Available tools. Assigning a new array copies the top-level array. */
	set tools(tools: AgentTool<any>[]);
	get tools(): AgentTool<any>[];
	/** Conversation transcript. Assigning a new array copies the top-level array. */
	set messages(messages: AgentMessage[]);
	get messages(): AgentMessage[];
	/**
	 * True while the agent is processing a prompt or continuation.
	 *
	 * This remains true until awaited `agent_end` listeners settle.
	 */
	readonly isStreaming: boolean;
	/** Partial assistant message for the current streamed response, if any. */
	readonly streamingMessage?: AgentMessage;
	/** Tool call ids currently executing. */
	readonly pendingToolCalls: ReadonlySet<string>;
	/** Error message from the most recent failed or aborted assistant turn, if any. */
	readonly errorMessage?: string;
}

```
Source: ice:packages/agent/src/types.ts:327-353.
None: additional CLI flags, slash commands, keybindings, core settings keys, agent RPC commands, or persistent files beyond those explicitly specified. This entry concerns only the named in-process APIs/events.

### 6. Dependencies

`AGENT-001`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

### 7. ice-rust today

rust:crates/ice-agent/src/types.rs:206-271; rust:crates/ice-agent/src/agent.rs:47-153; rust:crates/ice-agent/src/agent.rs:324-328. Rust returns an owned snapshot, supports only an assistant streaming payload and does not expose TypeScript-style state mutation/setter methods. Its placeholder uses Model::new defaults rather than explicitly zeroing every model field here. Preserve Rust ownership, but expose explicit configuration/transcript mutation APIs needed by callers; do not pretend mutating a returned snapshot changes the live agent.

### 8. Port notes

Target ice-agent unless explicitly identified as an ice-harness or ice-tools helper. Preserve one authoritative loop. Use owned snapshots and explicit async error/cancellation contracts; native plugin consumers require the same host-facing hook/event data, not a second controller. Existing workspace dependencies tokio, tokio-util, futures and serde_json cover the primitives.

### 9. Size and priority

Estimated Rust implementation: **95-175 lines** (estimate, not measured source). **P1** - Required for deterministic SDK, event and tool behavior during a TypeScript-to-Rust port.

## AGENT-021 - Awaited Agent subscribers and the true idle boundary

### 1. Title and status

Awaited Agent subscribers and the true idle boundary. **Status: partial.**

### 2. ice source

ice:packages/agent/src/agent.ts:249-262; ice:packages/agent/src/agent.ts:506-562; ice:packages/agent/src/agent.ts:604-610. subscribe/processEvents/finishRun. Tests: ice:packages/agent/test/agent.test.ts:190-226 (`should await async subscribers before prompt resolves`); ice:packages/agent/test/agent.test.ts:228-261 (`waitForIdle should wait for async subscribers`); ice:packages/agent/test/agent.test.ts:263-299 (`should pass the active abort signal to subscribers`).

### 3. What it does

Ensure a run does not become idle until all registered lifecycle handlers have completed.

### 4. How it works

subscribe stores callbacks in a Set and returns a closure that removes that exact callback; there is no replay. processEvents reduces state first, requires an active AbortController, then iterates subscribers in Set insertion order and awaits each with the same event and active signal. Handlers can enqueue work or await persistence before the producer advances. A thrown/rejected subscriber enters the run failure path; a failure while emitting that synthetic lifecycle can still reject prompt, but finally cleans runtime state.

agent_end means no further normal loop events, not idle. isStreaming remains true and waitForIdle remains pending while its subscribers run. finishRun clears state and resolves the stored promise only afterward. No listener timeout, maximum count, payload copy isolation or concurrent fan-out is added. Tool-update sink calls from parallel tools can overlap; the rule is per invocation, not a global event mutex.

### 5. Interfaces

```ts
	/**
	 * Reduce internal state for a loop event, then await listeners.
	 *
	 * `agent_end` only means no further loop events will be emitted. The run is
	 * considered idle later, after all awaited listeners for `agent_end` finish
	 * and `finishRun()` clears runtime-owned state.
	 */
	private async processEvents(event: AgentEvent): Promise<void> {
		switch (event.type) {
			case "message_start":
				this._state.streamingMessage = event.message;
				break;

			case "message_update":
				this._state.streamingMessage = event.message;
				break;

			case "message_end":
				this._state.streamingMessage = undefined;
				this._state.messages.push(event.message);
				break;

			case "tool_execution_start": {
				const pendingToolCalls = new Set(this._state.pendingToolCalls);
				pendingToolCalls.add(event.toolCallId);
				this._state.pendingToolCalls = pendingToolCalls;
				break;
			}

			case "tool_execution_end": {
				const pendingToolCalls = new Set(this._state.pendingToolCalls);
				pendingToolCalls.delete(event.toolCallId);
				this._state.pendingToolCalls = pendingToolCalls;
				break;
			}

			case "turn_end":
				if (event.message.role === "assistant" && event.message.errorMessage) {
					this._state.errorMessage = event.message.errorMessage;
				}
				break;

			case "agent_end":
				this._state.streamingMessage = undefined;
				break;
		}

		const signal = this.activeRun?.abortController.signal;
		if (!signal) {
			throw new Error("Agent listener invoked outside active run");
		}
		for (const listener of this.listeners) {
			await listener(event, signal);
		}
```
Source: ice:packages/agent/src/agent.ts:557-610.
None: additional CLI flags, slash commands, keybindings, core settings keys, agent RPC commands, or persistent files beyond those explicitly specified. This entry concerns only the named in-process APIs/events.

### 6. Dependencies

`AGENT-001`, `AGENT-020`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

### 7. ice-rust today

rust:crates/ice-agent/src/agent.rs:128-153; rust:crates/ice-agent/src/agent.rs:285-322; rust:crates/ice-agent/src/events.rs:134-165. Rust Agent subscribers receive tokio broadcast messages (capacity 256) and are not awaited; slow receivers can lag/drop, no active signal accompanies each callback and completion does not acknowledge downstream persistence. The raw emitter can await a custom sink, but the public wrapper does not expose an awaited subscription path. Add a host-critical awaited sink separate from optional lossy UI broadcast.

### 8. Port notes

Target ice-agent unless explicitly identified as an ice-harness or ice-tools helper. Preserve one authoritative loop. Use owned snapshots and explicit async error/cancellation contracts; native plugin consumers require the same host-facing hook/event data, not a second controller. Existing workspace dependencies tokio, tokio-util, futures and serde_json cover the primitives.

### 9. Size and priority

Estimated Rust implementation: **130-230 lines** (estimate, not measured source). **P0** - Persistence and shutdown must not treat queued broadcast delivery as completed event handling.

## AGENT-022 - Steering and follow-up queue CRUD and delivery modes

### 1. Title and status

Steering and follow-up queue CRUD and delivery modes. **Status: partial.**

### 2. ice source

ice:packages/agent/src/agent.ts:125-168; ice:packages/agent/src/agent.ts:291-345. PendingMessageQueue and Agent queue methods. Tests: ice:packages/agent/test/agent.test.ts:501-511 (`reports queued steering separately from follow-ups`); ice:packages/agent/test/agent.test.ts:513-551 (`removes only the selected queued steering and follow-up messages`).

### 3. What it does

Queue, inspect, remove and clear pending messages without accidentally changing delivery order.

### 4. How it works

Use two independent FIFO arrays. Default mode is one-at-a-time; all returns a shallow copy of the entire queue and clears it, while one-at-a-time removes only the first element. enqueue retains the supplied message reference. remove selects the first object-identical message using indexOf, not equal JSON; return false if absent. clear removes every queued item. Agent exposes steer, followUp, removeSteeringMessage, removeFollowUpMessage, clearSteeringQueue, clearFollowUpQueue, clearAllQueues, hasQueuedMessages (either queue), hasQueuedSteeringMessages and mutable steering/follow-up mode accessors. No IDs, persistence, caps, priority promotion or duplicate suppression exist at this layer. Queue mutation is synchronous between JavaScript awaits.

### 5. Interfaces

```ts
class PendingMessageQueue {
	private messages: AgentMessage[] = [];
	public mode: QueueMode;

	constructor(mode: QueueMode) {
		this.mode = mode;
	}

	enqueue(message: AgentMessage): void {
		this.messages.push(message);
	}

	hasItems(): boolean {
		return this.messages.length > 0;
	}

	drain(): AgentMessage[] {
		if (this.mode === "all") {
			const drained = this.messages.slice();
			this.messages = [];
			return drained;
		}

		const first = this.messages[0];
		if (!first) {
			return [];
		}
		this.messages = this.messages.slice(1);
		return [first];
	}

	remove(message: AgentMessage): boolean {
		const index = this.messages.indexOf(message);
		if (index === -1) {
			return false;
		}
		this.messages.splice(index, 1);
		return true;
	}

	clear(): void {
		this.messages = [];
	}
}
```
Source: ice:packages/agent/src/agent.ts:125-168.
```ts
	/** Queue a message to be injected after the current assistant turn finishes. */
	steer(message: AgentMessage): void {
		this.steeringQueue.enqueue(message);
	}

	/** Queue a message to run only after the agent would otherwise stop. */
	followUp(message: AgentMessage): void {
		this.followUpQueue.enqueue(message);
	}

	/** Remove one pending steering message by object identity. */
	removeSteeringMessage(message: AgentMessage): boolean {
		return this.steeringQueue.remove(message);
	}

	/** Remove one pending follow-up message by object identity. */
	removeFollowUpMessage(message: AgentMessage): boolean {
		return this.followUpQueue.remove(message);
	}

	/** Remove all queued steering messages. */
	clearSteeringQueue(): void {
		this.steeringQueue.clear();
	}

	/** Remove all queued follow-up messages. */
	clearFollowUpQueue(): void {
		this.followUpQueue.clear();
	}

	/** Remove all queued steering and follow-up messages. */
	clearAllQueues(): void {
		this.clearSteeringQueue();
		this.clearFollowUpQueue();
	}

	/** Returns true when either queue still contains pending messages. */
	hasQueuedMessages(): boolean {
		return this.steeringQueue.hasItems() || this.followUpQueue.hasItems();
	}

	/** Returns true when the steering queue contains messages for the next turn boundary. */
	hasQueuedSteering(): boolean {
		return this.steeringQueue.hasItems();
	}

	/** Active abort signal for the current run, if any. */
	get signal(): AbortSignal | undefined {
		return this.activeRun?.abortController.signal;
	}

	/** Abort the current run, if one is active. */
	abort(): void {
		this.activeRun?.abortController.abort();
	}
```
Source: ice:packages/agent/src/agent.ts:291-345.
None: additional CLI flags, slash commands, keybindings, core settings keys, agent RPC commands, or persistent files beyond those explicitly specified. This entry concerns only the named in-process APIs/events.

### 6. Dependencies

`AGENT-001`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

### 7. ice-rust today

rust:crates/ice-agent/src/queue.rs:1-116; rust:crates/ice-agent/src/agent.rs:330-355; rust:crates/ice-agent/src/agent.rs:432-443. Rust MessageQueue implements FIFO drains, but public Agent exposes only enqueue and combined has_queued_messages. Selected removal, individual clear/inspection and delivery-mode updates are absent there. reset calls try_drain once per queue, so default one-at-a-time leaves all but the oldest message queued. Native AgentHarness has durable queue APIs; their IDs/semantics differ from this wrapper and should be adapted explicitly.

### 8. Port notes

Target ice-agent unless explicitly identified as an ice-harness or ice-tools helper. Preserve one authoritative loop. Use owned snapshots and explicit async error/cancellation contracts; native plugin consumers require the same host-facing hook/event data, not a second controller. Existing workspace dependencies tokio, tokio-util, futures and serde_json cover the primitives.

### 9. Size and priority

Estimated Rust implementation: **105-185 lines** (estimate, not measured source). **P1** - Required for deterministic SDK, event and tool behavior during a TypeScript-to-Rust port.

## AGENT-023 - Continuation from an assistant tail using queued work

### 1. Title and status

Continuation from an assistant tail using queued work. **Status: partial.**

### 2. ice source

ice:packages/agent/src/agent.ts:380-408; ice:packages/agent/src/agent.ts:429-455; ice:packages/agent/src/agent.ts:495-503. Agent.continue/runPromptMessages/createLoopConfig. Tests: ice:packages/agent/test/agent.test.ts:636-672 (`continue() should process queued follow-up messages after an assistant turn`); ice:packages/agent/test/agent.test.ts:674-716 (`continue() should keep one-at-a-time steering semantics from assistant tail`).

### 3. What it does

Resume an otherwise completed agent when steering or follow-up work is waiting, without consuming two one-at-a-time messages at once.

### 4. How it works

Reject an active run first and reject an empty transcript. If the last message is assistant, drain steering according to its mode. When nonempty, start a new prompt run from those messages and skip exactly the first steering poll inside the loop; later turns poll normally. Otherwise drain follow-ups and run those as prompt messages without the skip. If both drains are empty, throw Cannot continue from message role: assistant. For a nonassistant tail use raw continuation with a context snapshot and no re-emission of its existing last message. Queue entries removed for the new run become the new prompt batch and are not restored on later provider failure.

### 5. Interfaces

```ts
	/** Continue from the current transcript. The last message must be a user or tool-result message. */
	async continue(): Promise<void> {
		if (this.activeRun) {
			throw new Error("Agent is already processing. Wait for completion before continuing.");
		}

		const lastMessage = this._state.messages[this._state.messages.length - 1];
		if (!lastMessage) {
			throw new Error("No messages to continue from");
		}

		if (lastMessage.role === "assistant") {
			const queuedSteering = this.steeringQueue.drain();
			if (queuedSteering.length > 0) {
				await this.runPromptMessages(queuedSteering, { skipInitialSteeringPoll: true });
				return;
			}

			const queuedFollowUps = this.followUpQueue.drain();
			if (queuedFollowUps.length > 0) {
				await this.runPromptMessages(queuedFollowUps);
				return;
			}

			throw new Error("Cannot continue from message role: assistant");
		}

		await this.runContinuation();
	}
```
Source: ice:packages/agent/src/agent.ts:380-408.
```ts
			getSteeringMessages: async () => {
				if (skipInitialSteeringPoll) {
					skipInitialSteeringPoll = false;
					return [];
				}
				return this.steeringQueue.drain();
			},
			getFollowUpMessages: async () => this.followUpQueue.drain(),
		};
```
Source: ice:packages/agent/src/agent.ts:495-503.
None: additional CLI flags, slash commands, keybindings, core settings keys, agent RPC commands, or persistent files beyond those explicitly specified. This entry concerns only the named in-process APIs/events.

### 6. Dependencies

`AGENT-004`, `AGENT-022`, `AGENT-026`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

### 7. ice-rust today

rust:crates/ice-agent/src/agent.rs:557-577. Rust Agent::continue_run rejects every assistant tail before consulting either queue, despite its comment promising the exception. Its additional run_agent_loop_from_assistant recovery function executes pending tools and is not a substitute for queue-driven new prompts. Implement the one-poll skip so the one-at-a-time contract is preserved.

### 8. Port notes

Target ice-agent unless explicitly identified as an ice-harness or ice-tools helper. Preserve one authoritative loop. Use owned snapshots and explicit async error/cancellation contracts; native plugin consumers require the same host-facing hook/event data, not a second controller. Existing workspace dependencies tokio, tokio-util, futures and serde_json cover the primitives.

### 9. Size and priority

Estimated Rust implementation: **65-115 lines** (estimate, not measured source). **P1** - Required for deterministic SDK, event and tool behavior during a TypeScript-to-Rust port.

## AGENT-024 - Synthetic failure lifecycle and guaranteed Agent finalization

### 1. Title and status

Synthetic failure lifecycle and guaranteed Agent finalization. **Status: partial.**

### 2. ice source

ice:packages/agent/src/agent.ts:506-555. runWithLifecycle/handleRunFailure/finishRun. Test: ice:packages/agent/test/agent.test.ts:159-188 (`emits full lifecycle events for thrown run failures`).

### 3. What it does

Turn an unexpected thrown run failure into an assistant error message and balanced terminal events.

### 4. How it works

runWithLifecycle rejects reentrancy, creates one AbortController plus a completion promise, sets activeRun and streaming state before invoking the executor. Catch any executor rejection, create an assistant with one empty text block, current public model identity, shared all-zero usage, stopReason aborted when signal is cancelled else error, error.message or String(error), and current timestamp. Emit message_start, message_end, turn_end with empty toolResults, then agent_end containing only that failure message. Do not replay a tool or re-emit earlier messages.

Finally always clear streamingMessage/isStreaming/pending tool IDs, resolve the idle promise and clear activeRun. If a subscriber rejects while handling the synthetic failure, prompt can still reject, but cleanup runs. Ordinary provider error events already produce a terminal assistant and do not enter this synthetic path. The low-level EventStream adapters lack this wrapper guarantee.

### 5. Interfaces

```ts
	private async runWithLifecycle(executor: (signal: AbortSignal) => Promise<void>): Promise<void> {
		if (this.activeRun) {
			throw new Error("Agent is already processing.");
		}

		const abortController = new AbortController();
		let resolvePromise = () => {};
		const promise = new Promise<void>((resolve) => {
			resolvePromise = resolve;
		});
		this.activeRun = { promise, resolve: resolvePromise, abortController };

		this._state.isStreaming = true;
		this._state.streamingMessage = undefined;
		this._state.errorMessage = undefined;

		try {
			await executor(abortController.signal);
		} catch (error) {
			await this.handleRunFailure(error, abortController.signal.aborted);
		} finally {
			this.finishRun();
		}
	}

	private async handleRunFailure(error: unknown, aborted: boolean): Promise<void> {
		const failureMessage = {
			role: "assistant",
			content: [{ type: "text", text: "" }],
			api: this._state.model.api,
			provider: this._state.model.provider,
			model: this._state.model.id,
			usage: EMPTY_USAGE,
			stopReason: aborted ? "aborted" : "error",
			errorMessage: error instanceof Error ? error.message : String(error),
			timestamp: Date.now(),
		} satisfies AgentMessage;
		await this.processEvents({ type: "message_start", message: failureMessage });
		await this.processEvents({ type: "message_end", message: failureMessage });
		await this.processEvents({ type: "turn_end", message: failureMessage, toolResults: [] });
		await this.processEvents({ type: "agent_end", messages: [failureMessage] });
	}

	private finishRun(): void {
		this._state.isStreaming = false;
		this._state.streamingMessage = undefined;
		this._state.pendingToolCalls = new Set<string>();
		this.activeRun?.resolve();
		this.activeRun = undefined;
	}
```
Source: ice:packages/agent/src/agent.ts:506-555.
None: additional CLI flags, slash commands, keybindings, core settings keys, agent RPC commands, or persistent files beyond those explicitly specified. This entry concerns only the named in-process APIs/events.

### 6. Dependencies

`AGENT-001`, `AGENT-020`, `AGENT-021`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

### 7. ice-rust today

rust:crates/ice-agent/src/agent.rs:391-416; rust:crates/ice-agent/src/agent.rs:449-480; rust:crates/ice-agent/src/agent.rs:544-577. Rust normal returned errors call finish_run and propagate Result, but do not synthesize the TypeScript assistant/event sequence. There is no RAII run guard in this wrapper; dropping/cancelling its future or unwinding a panic can leave active_run and streaming state set. The CLI AgentHarness has separate guards and must be evaluated independently. Add cancellation-safe finalization and an explicit compatibility error-event policy.

### 8. Port notes

Target ice-agent unless explicitly identified as an ice-harness or ice-tools helper. Preserve one authoritative loop. Use owned snapshots and explicit async error/cancellation contracts; native plugin consumers require the same host-facing hook/event data, not a second controller. Existing workspace dependencies tokio, tokio-util, futures and serde_json cover the primitives.

### 9. Size and priority

Estimated Rust implementation: **105-185 lines** (estimate, not measured source). **P0** - A run failure must not strand the agent as busy or omit the persisted terminal failure that consumers expect.

## AGENT-025 - Agent option and mutable hook forwarding

### 1. Title and status

Agent option and mutable hook forwarding. **Status: partial.**

### 2. ice source

ice:packages/agent/src/agent.ts:97-123; ice:packages/agent/src/agent.ts:188-245; ice:packages/agent/src/agent.ts:457-503. AgentOptions/createLoopConfig. Tests: ice:packages/agent/test/agent.test.ts:718-757 (`keeps legacy prepareNextTurn signal callback behavior`); ice:packages/agent/test/agent.test.ts:759-801 (`forwards shouldStopAfterTurn through AgentOptions`); ice:packages/agent/test/agent.test.ts:803-827 (`forwards sessionId to streamFunction options`).

### 3. What it does

Make the raw-loop extension points available through the stateful Agent SDK.

### 4. How it works

Construction and public fields accept convertToLlm, transformContext, streamFunction, getApiKey, onPayload/onResponse, beforeToolCall/afterToolCall, shouldStopAfterTurn, legacy prepareNextTurn(signal), context-aware prepareNextTurnWithContext(context, signal), sessionId, thinkingBudgets, transport(default auto), maxRetryDelayMs and toolExecution(default parallel). When building a run config snapshot model/thinking/system/tools/messages and most callback references. Context-aware preparation wins over legacy when both are present; its wrapper checks the current preparation fields on each invocation. shouldStopAfterTurn is captured once. The active signal is passed explicitly to legacy/context-aware preparation and stop callbacks.

Thinking off becomes undefined reasoning, not a literal provider off. The wrapper intentionally exposes only selected SimpleStreamOptions; the raw loop accepts the larger set. Mutating a provider/callback field mid-run does not uniformly replace a captured reference, so preserve the exact snapshot/dynamic distinction rather than claiming all settings are live. No plugin loading, permission policy or additional loop is introduced here.

### 5. Interfaces

```ts
/** Options for constructing an {@link Agent}. */
export interface AgentOptions {
	initialState?: Partial<Omit<AgentState, "pendingToolCalls" | "isStreaming" | "streamingMessage" | "errorMessage">>;
	convertToLlm?: (messages: AgentMessage[]) => Message[] | Promise<Message[]>;
	transformContext?: (messages: AgentMessage[], signal?: AbortSignal) => Promise<AgentMessage[]>;
	streamFn: StreamFn;
	getApiKey?: (provider: string) => Promise<string | undefined> | string | undefined;
	onPayload?: SimpleStreamOptions["onPayload"];
	onResponse?: SimpleStreamOptions["onResponse"];
	beforeToolCall?: (context: BeforeToolCallContext, signal?: AbortSignal) => Promise<BeforeToolCallResult | undefined>;
	afterToolCall?: (context: AfterToolCallContext, signal?: AbortSignal) => Promise<AfterToolCallResult | undefined>;
	shouldStopAfterTurn?: (context: ShouldStopAfterTurnContext, signal?: AbortSignal) => boolean | Promise<boolean>;
	prepareNextTurn?: (
		signal?: AbortSignal,
	) => Promise<AgentLoopTurnUpdate | undefined> | AgentLoopTurnUpdate | undefined;
	prepareNextTurnWithContext?: (
		context: PrepareNextTurnContext,
		signal?: AbortSignal,
	) => Promise<AgentLoopTurnUpdate | undefined> | AgentLoopTurnUpdate | undefined;
	steeringMode?: QueueMode;
	followUpMode?: QueueMode;
	sessionId?: string;
	thinkingBudgets?: ThinkingBudgets;
	transport?: Transport;
	maxRetryDelayMs?: number;
	toolExecution?: ToolExecutionMode;
}
```
Source: ice:packages/agent/src/agent.ts:97-123.
```ts
	private createLoopConfig(options: { skipInitialSteeringPoll?: boolean } = {}): AgentLoopConfig {
		let skipInitialSteeringPoll = options.skipInitialSteeringPoll === true;
		const shouldStopAfterTurn = this.shouldStopAfterTurn;
		return {
			model: this._state.model,
			reasoning: this._state.thinkingLevel === "off" ? undefined : this._state.thinkingLevel,
			sessionId: this.sessionId,
			onPayload: this.onPayload,
			onResponse: this.onResponse,
			transport: this.transport,
			thinkingBudgets: this.thinkingBudgets,
			maxRetryDelayMs: this.maxRetryDelayMs,
			toolExecution: this.toolExecution,
			beforeToolCall: this.beforeToolCall,
			afterToolCall: this.afterToolCall,
			shouldStopAfterTurn: shouldStopAfterTurn
				? async (context) => await shouldStopAfterTurn(context, this.signal)
				: undefined,
			prepareNextTurn:
				this.prepareNextTurnWithContext || this.prepareNextTurn
					? async (context) => {
							if (this.prepareNextTurnWithContext) {
								return await this.prepareNextTurnWithContext(context, this.signal);
							}
							return await this.prepareNextTurn?.(this.signal);
						}
					: undefined,
			convertToLlm: this.convertToLlm,
			transformContext: this.transformContext,
			getApiKey: this.getApiKey,
			getSteeringMessages: async () => {
				if (skipInitialSteeringPoll) {
					skipInitialSteeringPoll = false;
					return [];
				}
				return this.steeringQueue.drain();
			},
			getFollowUpMessages: async () => this.followUpQueue.drain(),
		};
```
Source: ice:packages/agent/src/agent.ts:465-503.
None: additional CLI flags, slash commands, keybindings, core settings keys, agent RPC commands, or persistent files beyond those explicitly specified. This entry concerns only the named in-process APIs/events.

### 6. Dependencies

`AGENT-002`, `AGENT-006`, `AGENT-007`, `AGENT-012`, `AGENT-015`, `AGENT-018`, `AGENT-019`, `AGENT-020`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

Final dependency classification. Shared contract references: `EXT-009`, `EXT-024`, `EXT-025`, `EXT-030`, `EXT-033`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

rust:crates/ice-agent/src/agent.rs:20-46; rust:crates/ice-agent/src/agent.rs:162-172; rust:crates/ice-agent/src/agent.rs:510-541. Rust Agent options/builder carry a much smaller set and hardcode transform, key resolution, before/after/prepare/stop hooks to None. Transport, thinking budgets, payload/response callbacks and retry-delay forwarding are also not exposed equivalently. Raw Rust hooks and the CLI harness can implement parts, but those do not fill the public Agent wrapper surface automatically.

### 8. Port notes

Target ice-agent unless explicitly identified as an ice-harness or ice-tools helper. Preserve one authoritative loop. Use owned snapshots and explicit async error/cancellation contracts; native plugin consumers require the same host-facing hook/event data, not a second controller. Existing workspace dependencies tokio, tokio-util, futures and serde_json cover the primitives.

### 9. Size and priority

Estimated Rust implementation: **150-260 lines** (estimate, not measured source). **P1** - Required for deterministic SDK, event and tool behavior during a TypeScript-to-Rust port.

## AGENT-026 - Prompt overloads and content-shape preservation

### 1. Title and status

Prompt overloads and content-shape preservation. **Status: partial.**

### 2. ice source

ice:packages/agent/src/agent.ts:370-377; ice:packages/agent/src/agent.ts:410-441. prompt/normalizePromptInput. Tests: ice:packages/agent/test/e2e.test.ts:186-190 (`handles a basic text prompt`); content-array/image behavior is specified directly in normalizePromptInput.

### 3. What it does

Accept text with optional images, a single typed message or an ordered message batch.

### 4. How it works

prompt checks busy status before normalization. An array is used directly, a nonstring message becomes a one-element array, and a string always becomes a user message with content=[{type:text, text:input},...images] and Date.now timestamp. Empty strings still create a text block; an empty image list adds nothing. Images supplied alongside a message/array are ignored because those overload paths return early. No input trim, image validation, size limit, path loading or policy prompt occurs in Agent. Initial prompt events and newMessages preserve this block-array shape even when the provider later supports plain strings.

### 5. Interfaces

```ts
	async prompt(input: string | AgentMessage | AgentMessage[], images?: ImageContent[]): Promise<void> {
		if (this.activeRun) {
			throw new Error(
				"Agent is already processing a prompt. Use steer() or followUp() to queue messages, or wait for completion.",
			);
		}
		const messages = this.normalizePromptInput(input, images);
		await this.runPromptMessages(messages);
```
Source: ice:packages/agent/src/agent.ts:370-377.
```ts
	private normalizePromptInput(
		input: string | AgentMessage | AgentMessage[],
		images?: ImageContent[],
	): AgentMessage[] {
		if (Array.isArray(input)) {
			return input;
		}

		if (typeof input !== "string") {
			return [input];
		}

		const content: Array<TextContent | ImageContent> = [{ type: "text", text: input }];
		if (images && images.length > 0) {
			content.push(...images);
		}
		return [{ role: "user", content, timestamp: Date.now() }];
	}
```
Source: ice:packages/agent/src/agent.ts:410-427.
None: additional CLI flags, slash commands, keybindings, core settings keys, agent RPC commands, or persistent files beyond those explicitly specified. This entry concerns only the named in-process APIs/events.

### 6. Dependencies

`AGENT-001`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

### 7. ice-rust today

rust:crates/ice-agent/src/agent.rs:376-403. Rust has text/single/batch convenience methods, but text constructs UserContent::Text and there is no text-plus-images convenience overload. A caller can construct UserMessage blocks manually, so image capability is not wholly missing. A TypeScript-compatible adapter should normalize the public event/persistence shape rather than merely relying on provider equivalence.

### 8. Port notes

Target ice-agent unless explicitly identified as an ice-harness or ice-tools helper. Preserve one authoritative loop. Use owned snapshots and explicit async error/cancellation contracts; native plugin consumers require the same host-facing hook/event data, not a second controller. Existing workspace dependencies tokio, tokio-util, futures and serde_json cover the primitives.

### 9. Size and priority

Estimated Rust implementation: **25-55 lines** (estimate, not measured source). **P2** - Required for deterministic SDK, event and tool behavior during a TypeScript-to-Rust port.

## AGENT-027 - Abort, wait-for-idle and reset lifetime rules

### 1. Title and status

Abort, wait-for-idle and reset lifetime rules. **Status: partial.**

### 2. ice source

ice:packages/agent/src/agent.ts:347-368; ice:packages/agent/src/agent.ts:506-555. abort/signal/waitForIdle/reset. Tests: ice:packages/agent/test/agent.test.ts:553-558 (`should handle abort controller`); ice:packages/agent/test/agent.test.ts:228-261 (`waitForIdle should wait for async subscribers`); ice:packages/agent/test/e2e.test.ts:207-218 (`handles abort during streaming`).

### 3. What it does

Provide cooperative cancellation and a stable completion wait while allowing explicit state reset.

### 4. How it works

Agent.abort aborts only the current controller and is a no-op while idle. signal getter is active signal or undefined. waitForIdle awaits the current run promise, or an already-resolved promise if none; it includes awaited subscribers. reset synchronously clears transcript, streaming/error/pending state and both entire queues, but keeps model/tools/system/options and does not abort or reject an active run. Calling reset during execution can therefore be followed by fresh events that repopulate state; do not describe it as a cancellation primitive. Only runWithLifecycle owns activeRun cleanup. There is no timeout on waiting and no recursive child/process cancellation in the Agent wrapper.

### 5. Interfaces

```ts
	/**
	 * Resolve when the current run and all awaited event listeners have finished.
	 *
	 * This resolves after `agent_end` listeners settle.
	 */
	waitForIdle(): Promise<void> {
		return this.activeRun?.promise ?? Promise.resolve();
	}

	/** Clear transcript state, runtime state, and queued messages. */
	reset(): void {
		this._state.messages = [];
		this._state.isStreaming = false;
		this._state.streamingMessage = undefined;
		this._state.pendingToolCalls = new Set<string>();
		this._state.errorMessage = undefined;
		this.clearFollowUpQueue();
		this.clearSteeringQueue();
	}

	/** Start a new prompt from text, a single message, or a batch of messages. */
	async prompt(message: AgentMessage | AgentMessage[]): Promise<void>;
```
Source: ice:packages/agent/src/agent.ts:347-368.
None: additional CLI flags, slash commands, keybindings, core settings keys, agent RPC commands, or persistent files beyond those explicitly specified. This entry concerns only the named in-process APIs/events.

### 6. Dependencies

`AGENT-020`, `AGENT-021`, `AGENT-022`, `AGENT-024`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

### 7. ice-rust today

rust:crates/ice-agent/src/agent.rs:358-480. Rust abort is equivalent cooperative token cancellation, but wait_for_idle clones a Notify then registers its wait after releasing the lock; notify_waiters can occur in between, producing a missed wakeup. This is a source-derived race, not runtime-tested. reset rejects active runs, leaves queued tail items via one try_drain, and takes state then active_run locks opposite to start_active_run (active_run then state), allowing a concurrent deadlock. Preserve Rust’s safer reject-active policy deliberately, fix queue clearing/lock order, and use a retained completion future/watch state.

### 8. Port notes

Target ice-agent unless explicitly identified as an ice-harness or ice-tools helper. Preserve one authoritative loop. Use owned snapshots and explicit async error/cancellation contracts; native plugin consumers require the same host-facing hook/event data, not a second controller. Existing workspace dependencies tokio, tokio-util, futures and serde_json cover the primitives.

### 9. Size and priority

Estimated Rust implementation: **95-175 lines** (estimate, not measured source). **P0** - Completion waits, reset and cancellation must not hang or retain stale queued work.

## AGENT-028 - Bearer-authenticated compact-event proxy request

### 1. Title and status

Bearer-authenticated compact-event proxy request. **Status: missing.**

### 2. ice source

ice:packages/agent/src/proxy.ts:59-82; ice:packages/agent/src/proxy.ts:103-183. Export streamProxy/ProxyStreamOptions; internal buildProxyRequestOptions/ProxyMessageEventStream. Tests: ice:packages/agent/test/proxy.test.ts:12-47 uses a stubbed proxy response; no request-body assertion in that file.

### 3. What it does

Route a model request through an application server that owns provider credentials while returning the ordinary assistant event stream.

### 4. How it works

streamProxy returns a ProxyMessageEventStream immediately and launches one asynchronous request. Initialize one assistant partial with pending stop reason, empty content, exact model/provider/API identity, zero usage/cost fields and Date.now timestamp. POST to the literal concatenation proxyUrl + /api/stream, without trimming a trailing slash. Set only Authorization: Bearer <authToken> and Content-Type: application/json as HTTP headers. Serialize full model and Context plus the twelve explicitly selected options below; JSON.stringify omits undefined values. options.headers is a field inside the JSON options object, not an override of the proxy HTTP headers. authToken/proxyUrl/signal and provider callbacks/API keys are not copied into that options object. The context/model themselves are not redacted.

Pass the local AbortSignal to fetch. There is no URL scheme/host allowlist, project-trust check, request cap, request timeout, retry loop, credential refresh or proxy server implementation in this module. An application that places secrets in headers/metadata/context sends them to the selected proxy. Error-response handling and stream parsing are separate entries.

### 5. Interfaces

```ts
type ProxySerializableStreamOptions = Pick<
	SimpleStreamOptions,
	| "temperature"
	| "samplingParams"
	| "maxTokens"
	| "serviceTier"
	| "reasoning"
	| "cacheRetention"
	| "sessionId"
	| "headers"
	| "metadata"
	| "transport"
	| "thinkingBudgets"
	| "maxRetryDelayMs"
>;

export interface ProxyStreamOptions extends ProxySerializableStreamOptions {
	/** Local abort signal for the proxy request */
	signal?: AbortSignal;
	/** Auth token for the proxy server */
	authToken: string;
	/** Proxy server URL (e.g., "https://genai.example.com") */
	proxyUrl: string;
}
```
Source: ice:packages/agent/src/proxy.ts:59-82.
```ts
function buildProxyRequestOptions(options: ProxyStreamOptions): ProxySerializableStreamOptions {
	return {
		temperature: options.temperature,
		samplingParams: options.samplingParams,
		maxTokens: options.maxTokens,
		serviceTier: options.serviceTier,
		reasoning: options.reasoning,
		cacheRetention: options.cacheRetention,
		sessionId: options.sessionId,
		headers: options.headers,
		metadata: options.metadata,
		transport: options.transport,
		thinkingBudgets: options.thinkingBudgets,
		maxRetryDelayMs: options.maxRetryDelayMs,
	};
```
Source: ice:packages/agent/src/proxy.ts:103-117.
```ts
		try {
			const response = await fetch(`${options.proxyUrl}/api/stream`, {
				method: "POST",
				headers: {
					Authorization: `Bearer ${options.authToken}`,
					"Content-Type": "application/json",
				},
				body: JSON.stringify({
					model,
					context,
					options: buildProxyRequestOptions(options),
				}),
				signal: options.signal,
			});
```
Source: ice:packages/agent/src/proxy.ts:155-168.
None: additional CLI flags, slash commands, keybindings, core settings keys, agent RPC commands, or persistent files beyond those explicitly specified. This entry concerns only the named in-process APIs/events.

### 6. Dependencies

`AGENT-001`, `AGENT-002`

External service: application-owned HTTP proxy implementing POST /api/stream. Provider authentication is a server concern. No proxy/network request was performed in this audit.

### 7. ice-rust today

rust:crates/ice-agent/src/stream_fn.rs:21-41; rust:crates/ice-agent/src/agent_loop.rs:441-477. Rust accepts a replaceable native stream function and has HTTP/provider streaming primitives, but the inspected crates contain no implementation of the /api/stream compact-event proxy client or its message reducer. Generic HTTP proxy configuration and the separate remote-session protocol are not counterparts. The provider factory or an injected StreamFn is the insertion point.

### 8. Port notes

Target ice-agent proxy module or an ice-ai transport adapter exposed through the existing StreamFn seam. reqwest, bytes, serde_json and futures are already workspace dependencies. Use an incremental UTF-8 decoder and the exact line grammar; do not assume a standard multiline SSE parser has identical behavior. Native plugins need only a provider/stream registration seam, not local child-agent execution. Add bounds/URL policy only as an explicitly documented hardening difference.

### 9. Size and priority

Estimated Rust implementation: **90-150 lines** (estimate, not measured source). **P1** - Required for deterministic SDK, event and tool behavior during a TypeScript-to-Rust port.

## AGENT-029 - Compact proxy line framing and terminal EOF handling

### 1. Title and status

Compact proxy line framing and terminal EOF handling. **Status: missing.**

### 2. ice source

ice:packages/agent/src/proxy.ts:183-218. streamProxy reader loop. Tests: ice:packages/agent/test/proxy.test.ts:12-20 parameterizes empty/LF/CRLF suffix; ice:packages/agent/test/proxy.test.ts:22-29 (`reports premature EOF instead of leaving result pending`); ice:packages/agent/test/proxy.test.ts:31-48 (`stops reading once a terminal event arrives`).

### 3. What it does

Consume the proxy’s one-JSON-object-per-data-line stream without hanging when a terminal event lacks a trailing newline.

### 4. How it works

Use response.body.getReader and a streaming TextDecoder. After each read, fail if the signal is aborted. Append decoded bytes, or on EOF flush the decoder and append an artificial newline. Split on LF and retain the final incomplete segment. Only lines beginning exactly data: followed by one space are candidates; trim the remainder, ignore it when empty, otherwise JSON.parse it and reduce it as one event. Ignore comments, event/id/retry lines, data: without a space and other prefixes. This is not standard SSE multiline-data assembly: each accepted line must be an independently valid JSON object. CRLF works because trim removes the final CR.

Push each recognized event. Stop processing the current chunk and stop reading as soon as a done/error event changes partial.stopReason from pending. Discard later lines in that chunk. EOF before such a terminal result throws Proxy stream ended before a terminal response event. Decoder/line/event buffers have no byte or index cap. Parsing/reduction exceptions are converted by the outer failure path, not skipped. A null response body throws at getReader and follows the same path.

### 5. Interfaces

```ts
			reader = response.body!.getReader();
			const decoder = new TextDecoder();
			let buffer = "";

			while (true) {
				const { done, value } = await reader.read();
				if (options.signal?.aborted) {
					throw new Error("Request aborted by user");
				}

				buffer += done ? `${decoder.decode()}\n` : decoder.decode(value, { stream: true });
				const lines = buffer.split("\n");
				buffer = lines.pop() || "";

				for (const line of lines) {
					if (line.startsWith("data: ")) {
						const data = line.slice(6).trim();
						if (data) {
							const proxyEvent = JSON.parse(data) as ProxyAssistantMessageEvent;
							const event = processProxyEvent(proxyEvent, partial);
							if (event) {
								stream.push(event);
								if (event.type === "done" || event.type === "error") break;
							}
						}
					}
				}
				if (done || partial.stopReason !== "pending") break;
			}

			if (partial.stopReason === "pending") {
				throw new Error("Proxy stream ended before a terminal response event");
			}
			if (options.signal?.aborted) {
				throw new Error("Request aborted by user");
			}
```
Source: ice:packages/agent/src/proxy.ts:183-218.
None: additional CLI flags, slash commands, keybindings, core settings keys, agent RPC commands, or persistent files beyond those explicitly specified. This entry concerns only the named in-process APIs/events.

### 6. Dependencies

`AGENT-028`, `AGENT-030`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

### 7. ice-rust today

rust:crates/ice-agent/src/stream_fn.rs:21-41; rust:crates/ice-agent/src/agent_loop.rs:441-477. Rust accepts a replaceable native stream function and has HTTP/provider streaming primitives, but the inspected crates contain no implementation of the /api/stream compact-event proxy client or its message reducer. Generic HTTP proxy configuration and the separate remote-session protocol are not counterparts. The provider factory or an injected StreamFn is the insertion point.

### 8. Port notes

Target ice-agent proxy module or an ice-ai transport adapter exposed through the existing StreamFn seam. reqwest, bytes, serde_json and futures are already workspace dependencies. Use an incremental UTF-8 decoder and the exact line grammar; do not assume a standard multiline SSE parser has identical behavior. Native plugins need only a provider/stream registration seam, not local child-agent execution. Add bounds/URL policy only as an explicitly documented hardening difference.

### 9. Size and priority

Estimated Rust implementation: **95-165 lines** (estimate, not measured source). **P1** - Required for deterministic SDK, event and tool behavior during a TypeScript-to-Rust port.

## AGENT-030 - Compact proxy content-block reconstruction and signatures

### 1. Title and status

Compact proxy content-block reconstruction and signatures. **Status: missing.**

### 2. ice source

ice:packages/agent/src/proxy.ts:36-57; ice:packages/agent/src/proxy.ts:247-376. ProxyAssistantMessageEvent/processProxyEvent. Tests: terminal result tests ice:packages/agent/test/proxy.test.ts:12-47; no dedicated malformed-index/signature/reconstruction test in that file.

### 3. What it does

Rebuild partial text, thinking and tool-call content from small events that omit full assistant snapshots.

### 4. How it works

start returns the existing partial without resetting it. text_start/thinking_start overwrite content[contentIndex] with an empty block of that type. Deltas append to that block and error if the index is absent or has the wrong type. End events assign the optional signature, including undefined, and return accumulated text/thinking. Tool-call start creates type toolCall/id/name/arguments={} plus a temporary partialJson string. Deltas append raw JSON text, assign parseStreamingJson(text)||{}, then replace the block with a shallow copy for reactivity. End deletes partialJson and emits the toolCall; a wrong-type toolcall_end silently returns undefined rather than throwing.

done replaces stopReason/usage and returns the shared partial as message; error also assigns errorMessage and returns it as error. No final argument validation, usage validation, index monotonicity, start-before-delta protocol validator, sparse-array bound, content-size limit or deep snapshot copy exists. Unknown event types console.warn and are ignored. A premature/error path can retain partialJson because only toolcall_end removes it. Every ordinary emitted event points at the mutable partial; consumers must not assume immutable historical snapshots. The JSON-repair utility is an AI-layer dependency, not an independent tool invocation.

### 5. Interfaces

```ts
export type ProxyAssistantMessageEvent =
	| { type: "start" }
	| { type: "text_start"; contentIndex: number }
	| { type: "text_delta"; contentIndex: number; delta: string }
	| { type: "text_end"; contentIndex: number; contentSignature?: string }
	| { type: "thinking_start"; contentIndex: number }
	| { type: "thinking_delta"; contentIndex: number; delta: string }
	| { type: "thinking_end"; contentIndex: number; contentSignature?: string }
	| { type: "toolcall_start"; contentIndex: number; id: string; toolName: string }
	| { type: "toolcall_delta"; contentIndex: number; delta: string }
	| { type: "toolcall_end"; contentIndex: number }
	| {
			type: "done";
			reason: Extract<StopReason, "stop" | "length" | "toolUse">;
			usage: AssistantMessage["usage"];
	  }
	| {
			type: "error";
			reason: Extract<StopReason, "aborted" | "error">;
			errorMessage?: string;
			usage: AssistantMessage["usage"];
	  };
```
Source: ice:packages/agent/src/proxy.ts:36-57.
```ts
		case "toolcall_start":
			partial.content[proxyEvent.contentIndex] = {
				type: "toolCall",
				id: proxyEvent.id,
				name: proxyEvent.toolName,
				arguments: {},
				partialJson: "",
			} satisfies ToolCall & { partialJson: string } as ToolCall;
			return { type: "toolcall_start", contentIndex: proxyEvent.contentIndex, partial };

		case "toolcall_delta": {
			const content = partial.content[proxyEvent.contentIndex];
			if (content?.type === "toolCall") {
				(content as any).partialJson += proxyEvent.delta;
				content.arguments = parseStreamingJson((content as any).partialJson) || {};
				partial.content[proxyEvent.contentIndex] = { ...content }; // Trigger reactivity
				return {
					type: "toolcall_delta",
					contentIndex: proxyEvent.contentIndex,
					delta: proxyEvent.delta,
					partial,
				};
			}
			throw new Error("Received toolcall_delta for non-toolCall content");
		}

		case "toolcall_end": {
			const content = partial.content[proxyEvent.contentIndex];
			if (content?.type === "toolCall") {
				delete (content as any).partialJson;
				return {
					type: "toolcall_end",
					contentIndex: proxyEvent.contentIndex,
					toolCall: content,
					partial,
				};
			}
			return undefined;
		}

		case "done":
			partial.stopReason = proxyEvent.reason;
			partial.usage = proxyEvent.usage;
			return { type: "done", reason: proxyEvent.reason, message: partial };

		case "error":
			partial.stopReason = proxyEvent.reason;
			partial.errorMessage = proxyEvent.errorMessage;
			partial.usage = proxyEvent.usage;
			return { type: "error", reason: proxyEvent.reason, error: partial };

		default: {
			const _exhaustiveCheck: never = proxyEvent;
			console.warn(`Unhandled proxy event type: ${(proxyEvent as any).type}`);
			return undefined;
		}
	}
```
Source: ice:packages/agent/src/proxy.ts:319-375.
Text/thinking reduction: ice:packages/agent/src/proxy.ts:247-317. None: additional CLI flags, slash commands, keybindings, core settings keys, agent RPC commands, or persistent files beyond those explicitly specified. This entry concerns only the named in-process APIs/events.

### 6. Dependencies

`AGENT-001`, `AI-011`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

### 7. ice-rust today

rust:crates/ice-agent/src/stream_fn.rs:21-41; rust:crates/ice-agent/src/agent_loop.rs:441-477. Rust accepts a replaceable native stream function and has HTTP/provider streaming primitives, but the inspected crates contain no implementation of the /api/stream compact-event proxy client or its message reducer. Generic HTTP proxy configuration and the separate remote-session protocol are not counterparts. The provider factory or an injected StreamFn is the insertion point.

### 8. Port notes

Target ice-agent proxy module or an ice-ai transport adapter exposed through the existing StreamFn seam. reqwest, bytes, serde_json and futures are already workspace dependencies. Use an incremental UTF-8 decoder and the exact line grammar; do not assume a standard multiline SSE parser has identical behavior. Native plugins need only a provider/stream registration seam, not local child-agent execution. Add bounds/URL policy only as an explicitly documented hardening difference. Prefer owned block snapshots in Rust while preserving emitted values. Rejecting malformed indices/usage is recommended but is a deliberate trust hardening, not present TypeScript behavior.

### 9. Size and priority

Estimated Rust implementation: **170-280 lines** (estimate, not measured source). **P1** - Required for deterministic SDK, event and tool behavior during a TypeScript-to-Rust port.

## AGENT-031 - Proxy failure conversion, cancellation and reader cleanup

### 1. Title and status

Proxy failure conversion, cancellation and reader cleanup. **Status: missing.**

### 2. ice source

ice:packages/agent/src/proxy.ts:143-239. streamProxy abortHandler and catch/finally. Tests: ice:packages/agent/test/proxy.test.ts:22-29 (`reports premature EOF instead of leaving result pending`); ice:packages/agent/test/proxy.test.ts:31-48 (`stops reading once a terminal event arrives`).

### 3. What it does

Always settle the result stream on a request/parse/premature-EOF failure and release the HTTP reader after completion.

### 4. How it works

Register an abort listener that calls reader.cancel("Request aborted by user") when a reader exists and swallows cancellation rejection. fetch receives the same signal; an already-aborted signal relies on fetch rather than a separate immediate callback. For non-OK responses default to Proxy error: <status> <statusText>; attempt to parse the whole JSON error body and use a truthy error field when present. No error-body cap applies. Any thrown error becomes partial.stopReason=aborted only if the local signal is currently aborted, otherwise error; set errorMessage, push an error event and end the stream. A server error event already supplies its own stop reason/usage and normally finishes without entering catch.

After a terminal frame recheck local cancellation before ending. Because the partial object is shared, a cancellation racing after a terminal push can mutate the previously returned message even if EventStream ignores a second terminal event. finally await reader.cancel, swallow its rejection, releaseLock and remove the abort listener. The result stream is ended before this cleanup: a reader.cancel promise that never settles can strand cleanup but not the already-completed result. releaseLock is not caught and can reject the detached asynchronous task. No callback clears unfinished partialJson, no request retry is attempted and no remote cancellation endpoint is called.

### 5. Interfaces

```ts
		let reader: ReadableStreamDefaultReader<Uint8Array> | undefined;

		const abortHandler = () => {
			if (reader) {
				reader.cancel("Request aborted by user").catch(() => {});
			}
		};

		if (options.signal) {
			options.signal.addEventListener("abort", abortHandler);
		}
```
Source: ice:packages/agent/src/proxy.ts:143-153.
```ts
			if (!response.ok) {
				let errorMessage = `Proxy error: ${response.status} ${response.statusText}`;
				try {
					const errorData = (await response.json()) as { error?: string };
					if (errorData.error) {
						errorMessage = `Proxy error: ${errorData.error}`;
					}
				} catch {
					// Couldn't parse error response
				}
				throw new Error(errorMessage);
			}
```
Source: ice:packages/agent/src/proxy.ts:170-181.
```ts
			if (partial.stopReason === "pending") {
				throw new Error("Proxy stream ended before a terminal response event");
			}
			if (options.signal?.aborted) {
				throw new Error("Request aborted by user");
			}

			stream.end();
		} catch (error) {
			const errorMessage = error instanceof Error ? error.message : String(error);
			const reason = options.signal?.aborted ? "aborted" : "error";
			partial.stopReason = reason;
			partial.errorMessage = errorMessage;
			stream.push({
				type: "error",
				reason,
				error: partial,
			});
			stream.end();
		} finally {
			await reader?.cancel().catch(() => {});
			reader?.releaseLock();
			if (options.signal) {
				options.signal.removeEventListener("abort", abortHandler);
			}
		}
	})();
```
Source: ice:packages/agent/src/proxy.ts:213-239.
None: additional CLI flags, slash commands, keybindings, core settings keys, agent RPC commands, or persistent files beyond those explicitly specified. This entry concerns only the named in-process APIs/events.

### 6. Dependencies

`AGENT-028`, `AGENT-029`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

### 7. ice-rust today

rust:crates/ice-agent/src/stream_fn.rs:21-41; rust:crates/ice-agent/src/agent_loop.rs:441-477. Rust accepts a replaceable native stream function and has HTTP/provider streaming primitives, but the inspected crates contain no implementation of the /api/stream compact-event proxy client or its message reducer. Generic HTTP proxy configuration and the separate remote-session protocol are not counterparts. The provider factory or an injected StreamFn is the insertion point.

### 8. Port notes

Target ice-agent proxy module or an ice-ai transport adapter exposed through the existing StreamFn seam. reqwest, bytes, serde_json and futures are already workspace dependencies. Use an incremental UTF-8 decoder and the exact line grammar; do not assume a standard multiline SSE parser has identical behavior. Native plugins need only a provider/stream registration seam, not local child-agent execution. Add bounds/URL policy only as an explicitly documented hardening difference. Use RAII/drop-safe reader cancellation without making final-result availability wait on indefinite transport cleanup.

### 9. Size and priority

Estimated Rust implementation: **80-140 lines** (estimate, not measured source). **P1** - Required for deterministic SDK, event and tool behavior during a TypeScript-to-Rust port.

## AGENT-032 - AgentHarness v2 operations are explicit scaffold rejections

### 1. Title and status

AgentHarness v2 operations are explicit scaffold rejections. **Status: different-by-design.**

### 2. ice source

ice:packages/agent/src/harness/agent-harness.ts:56-81; ice:packages/agent/src/harness/agent-harness.ts:280-312; ice:packages/agent/src/harness/agent-harness.ts:354-422; ice:packages/agent/src/harness/agent-harness.ts:463-465. AgentHarness.create/unavailable and listed methods. Tests: ice:packages/agent/test/harness/agent-harness-scaffold.test.ts:8-20 (`opens the main lane over a v4 session`); ice:packages/agent/test/harness/agent-harness-scaffold.test.ts:22-33 (`rejects unimplemented operations explicitly`).

### 3. What it does

Expose the proposed v2 harness surface without pretending its run, queue, compaction or navigation operations are implemented.

### 4. How it works

AgentHarness.create simply constructs the object and returns {harness, suspended:[]}; it does not scan records, recover operations, validate model identities or start a loop. The following methods return Promise.reject(HarnessNotImplemented(operation)): prompt, skill, promptFromTemplate, compact, navigateTree, resume, abort, steer, followUp, nextRun, cancelQueued, recordUsage and createLane. After close the same unavailable helper rejects HarnessClosed instead. Arguments are ignored before rejection; no model, tool, queue or persistence mutation occurs.

waitForIdle and runToCompletion resolve immediately; peekAction and executeAction resolve undefined; runWhenIdle immediately invokes/awaits the supplied callback, with no idle gate. These behaviors remain after close because only unavailable checks closed. This is a shipped scaffold contract, not a missing Rust agent implementation. The active TypeScript coding-agent/low-level Agent follows other paths and must not be confused with this exported class.

### 5. Interfaces

```ts
export class HarnessFault extends Error {
	readonly cause: unknown;

	constructor(message: string, cause: unknown) {
		super(message);
		this.name = "HarnessFault";
		this.cause = cause;
	}
}

export class HarnessClosed extends Error {
	constructor() {
		super("AgentHarness was closed while the operation was active");
		this.name = "HarnessClosed";
	}
}

export class HarnessNotImplemented extends Error {
	readonly operation: string;

	constructor(operation: string) {
		super(`AgentHarness.${operation} is not implemented yet`);
		this.name = "HarnessNotImplemented";
		this.operation = operation;
	}
}
```
Source: ice:packages/agent/src/harness/agent-harness.ts:56-81.
```ts
export interface AgentLane {
	readonly name: string;
	getLeafId(): Promise<string | null>;
	prompt(text: string, images?: ImageContent[]): Promise<RunResult>;
	prompt(message: AgentMessage | AgentMessage[]): Promise<RunResult>;
	skill(name: string, additionalInstructions?: string): Promise<RunResult>;
	promptFromTemplate(name: string, args?: string[]): Promise<RunResult>;
	compact(options?: { customInstructions?: string }): Promise<CompactionResult>;
	navigateTree(targetId: string | null, options?: NavigateOptions): Promise<NavigationResult>;
	resume(): Promise<ResumeResult>;
	abort(): Promise<AbortResult>;
	steer(text: string, images?: ImageContent[]): Promise<QueueResult>;
	steer(message: AgentMessage): Promise<QueueResult>;
	followUp(text: string, images?: ImageContent[]): Promise<QueueResult>;
	followUp(message: AgentMessage): Promise<QueueResult>;
	nextRun(text: string, images?: ImageContent[]): Promise<QueueResult>;
	nextRun(message: AgentMessage): Promise<QueueResult>;
	cancelQueued(entryId: string): Promise<CancelQueuedResult>;
	recordUsage(usage: Usage, options?: { entryId?: string; details?: JsonValue }): Promise<RecordUsageResult>;
	waitForIdle(): Promise<void>;
	runWhenIdle(callback: () => void | Promise<void>): Promise<void>;
	peekAction(): Promise<ActionInfo | undefined>;
	executeAction(): Promise<ActionInfo | undefined>;
	runToCompletion(): Promise<void>;
	getModel(): Promise<Model<Api>>;
	setModel(model: Model<Api>): Promise<void>;
	getThinkingLevel(): Promise<ThinkingLevel>;
	setThinkingLevel(level: ThinkingLevel): Promise<void>;
	getActiveTools(): Promise<string[]>;
	setActiveTools(names: string[]): Promise<void>;
	readonly session: SessionTree;
	watch(): Promise<WatchHandle<LaneSnapshot>>;
}
```
Source: ice:packages/agent/src/harness/agent-harness.ts:280-312.
```ts
	static async create(
		options: AgentHarnessOptions,
	): Promise<{ harness: AgentHarness; suspended: SuspendedOperation[] }> {
		return { harness: new AgentHarness(options), suspended: [] };
	}

	private unavailable<T>(operation: string): Promise<T> {
		return Promise.reject(this.closed ? new HarnessClosed() : new HarnessNotImplemented(operation));
	}

	async getLeafId(): Promise<string | null> {
		return this.durableSession.getLeafId();
	}

	async prompt(_text: string, _images?: ImageContent[]): Promise<RunResult>;
	async prompt(_message: AgentMessage | AgentMessage[]): Promise<RunResult>;
	async prompt(_input: string | AgentMessage | AgentMessage[], _images?: ImageContent[]): Promise<RunResult> {
		return this.unavailable("prompt");
	}
	async skill(_name: string, _additionalInstructions?: string): Promise<RunResult> {
		return this.unavailable("skill");
	}
	async promptFromTemplate(_name: string, _args?: string[]): Promise<RunResult> {
		return this.unavailable("promptFromTemplate");
	}
	async compact(_options?: { customInstructions?: string }): Promise<CompactionResult> {
		return this.unavailable("compact");
	}
	async navigateTree(_targetId: string | null, _options?: NavigateOptions): Promise<NavigationResult> {
		return this.unavailable("navigateTree");
	}
	async resume(): Promise<ResumeResult> {
		return this.unavailable("resume");
	}
	async abort(): Promise<AbortResult> {
		return this.unavailable("abort");
	}
	async steer(_text: string, _images?: ImageContent[]): Promise<QueueResult>;
	async steer(_message: AgentMessage): Promise<QueueResult>;
	async steer(_input: string | AgentMessage, _images?: ImageContent[]): Promise<QueueResult> {
		return this.unavailable("steer");
	}
	async followUp(_text: string, _images?: ImageContent[]): Promise<QueueResult>;
	async followUp(_message: AgentMessage): Promise<QueueResult>;
	async followUp(_input: string | AgentMessage, _images?: ImageContent[]): Promise<QueueResult> {
		return this.unavailable("followUp");
	}
	async nextRun(_text: string, _images?: ImageContent[]): Promise<QueueResult>;
	async nextRun(_message: AgentMessage): Promise<QueueResult>;
	async nextRun(_input: string | AgentMessage, _images?: ImageContent[]): Promise<QueueResult> {
		return this.unavailable("nextRun");
	}
	async cancelQueued(_entryId: string): Promise<CancelQueuedResult> {
		return this.unavailable("cancelQueued");
	}
	async recordUsage(_usage: Usage, _options?: { entryId?: string; details?: JsonValue }): Promise<RecordUsageResult> {
		return this.unavailable("recordUsage");
	}
	async waitForIdle(): Promise<void> {}
	async runWhenIdle(callback: () => void | Promise<void>): Promise<void> {
		await callback();
	}
	async peekAction(): Promise<ActionInfo | undefined> {
		return undefined;
	}
	async executeAction(): Promise<ActionInfo | undefined> {
		return undefined;
	}
	async runToCompletion(): Promise<void> {}
```
Source: ice:packages/agent/src/harness/agent-harness.ts:354-422.
None: additional CLI flags, slash commands, keybindings, core settings keys, agent RPC commands, or persistent files beyond those explicitly specified. This entry concerns only the named in-process APIs/events.

### 6. Dependencies

`AGENT-036`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

Final dependency classification. Shared contract references: `SESSION-001`, `SESSION-003`, `SESSION-030`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

rust:crates/ice-harness/src/agent_harness.rs:649-810; rust:crates/ice-harness/src/agent_harness.rs:2005-2190. Rust AgentHarness is a working session runner with persisted operation admission and recovery; create defaults to rejecting existing records unless allow_existing_session. Its comment claiming TypeScript create.restore rejects is stale: current TS create performs no such check. Do not port the scaffold’s failures over working Rust behavior. Full Rust session semantics are audited in the session area.

### 8. Port notes

Target documentation/compatibility facade only, not a new engine. Preserve the operational Rust ice-harness. The v2 TypeScript surface is not an implementation roadmap authorized by this audit.

### 9. Size and priority

Estimated Rust implementation: **0-0 lines** (estimate, not measured source). **P3** - Prevents planned or stubbed interfaces from being misreported as working TypeScript capabilities.

## AGENT-033 - Dormant v2 hook, action, result and trace type contracts

### 1. Title and status

Dormant v2 hook, action, result and trace type contracts. **Status: different-by-design.**

### 2. ice source

ice:packages/agent/src/harness/agent-harness.ts:27-54; ice:packages/agent/src/harness/agent-harness.ts:83-272; ice:packages/agent/src/harness/agent-harness.ts:317-318; ice:packages/agent/src/harness/types.ts:80-126. PassiveRegistry/HookName/ActionInfo/RunOutcome/CompactionOutcome/NavigationOutcome/SuspendedOperation/ExecutionSpan/AgentHarnessOptions. Tests: scaffold test ice:packages/agent/test/harness/agent-harness-scaffold.test.ts:7-33 establishes only construction and explicit rejections; no passive hook emission test.

### 3. What it does

Publish names and payload types for a future lane-driven harness, while current hook/event registration intentionally does nothing.

### 4. How it works

PassiveRegistry implements both hooks.on and events.on by ignoring name/handler/options and returning an empty unsubscribe closure. It stores no listeners, never calls a hook, and emits no events. Consequently the eleven HookName values below are declarations, not active lifecycle gates. ActionInfo, operation outcomes, rejection classes, suspended-operation metadata, snapshots and execution-span interfaces are likewise type contracts; this scaffold has no dispatcher or trace exporter behind them. HarnessTool.replay=never|safe is not enforced here. AgentHarnessOptions includes drive automatic/manual, model collection, system/tool-context factories, projectors and execution context, but the constructor does not consume most of them.

Preserve exact declared DTOs where a remote/SDK compatibility layer uses them, while labeling unreachable outcomes as declared-only. Do not mark Rust missing a working durable manual-drive scheduler merely because these names appear in TypeScript. Each named operation’s actual behavior remains AGENT-032. No new files, model calls, hooks or telemetry are generated by registration.

### 5. Interfaces

```ts
export class LaneBusy extends TaggedError("LaneBusy")<{
	lane: string;
	operationId: string;
	operationKind: "run" | "compaction" | "navigation";
	message: string;
}> {}
export class MissingIdentities extends TaggedError("MissingIdentities")<{
	lane: string;
	tools: string[];
	models: string[];
	message: string;
}> {}
export class NoActiveRun extends TaggedError("NoActiveRun")<{ lane: string; message: string }> {}
export class NoActiveOperation extends TaggedError("NoActiveOperation")<{ lane: string; message: string }> {}
export class NothingToResume extends TaggedError("NothingToResume")<{ lane: string; message: string }> {}
export class InvalidMessage extends TaggedError("InvalidMessage")<{ lane: string; reason: string; message: string }> {}
export class UnknownSkill extends TaggedError("UnknownSkill")<{ name: string; message: string }> {}
export class UnknownTemplate extends TaggedError("UnknownTemplate")<{ name: string; message: string }> {}
export class UnknownTarget extends TaggedError("UnknownTarget")<{ targetId: string; message: string }> {}
export class UnknownQueueItem extends TaggedError("UnknownQueueItem")<{
	lane: string;
	entryId: string;
	message: string;
}> {}
export class LaneExists extends TaggedError("LaneExists")<{ lane: string; message: string }> {}
export class InvalidLane extends TaggedError("InvalidLane")<{ lane: string; reason: string; message: string }> {}
export class NothingToCompact extends TaggedError("NothingToCompact")<{ lane: string; message: string }> {}
export class Closed extends TaggedError("Closed")<{ message: string }> {}
```
Source: ice:packages/agent/src/harness/agent-harness.ts:27-54.
```ts
export interface OperationError {
	code: string;
	message: string;
}

export type RunOutcome =
	| { kind: "completed"; leafId: string; finalEntryId: string; finalMessage: AssistantMessage }
	| { kind: "aborted"; leafId: string; finalEntryId: string; finalMessage: AssistantMessage }
	| { kind: "failed"; leafId: string; error: OperationError; finalEntryId?: string; finalMessage?: AssistantMessage }
	| { kind: "suspended"; leafId: string; finalEntryId: string; deferred: DeferredHandle };

export type CompactionOutcome =
	| { kind: "completed"; leafId: string; entry: CompactionEntry }
	| { kind: "declined" | "aborted"; leafId: string }
	| { kind: "failed"; leafId: string; error: OperationError };

export type NavigationOutcome =
	| { kind: "completed"; newLeafId: string | null; summaryEntry?: BranchSummaryEntry }
	| { kind: "declined" | "aborted"; leafId: string | null }
	| { kind: "failed"; leafId: string | null; error: OperationError };

export type RunRejected = LaneBusy | InvalidMessage | UnknownSkill | UnknownTemplate | Closed;
export type CompactionRejected = LaneBusy | NothingToCompact | Closed;
export type NavigationRejected = LaneBusy | UnknownTarget | Closed;
export type ResumeRejected = LaneBusy | NothingToResume | MissingIdentities | Closed;
export type QueueRejected = NoActiveRun | InvalidMessage | Closed;
export type CancelQueuedRejected = UnknownQueueItem | Closed;
export type AbortRejected = NoActiveOperation | Closed;

export type RunResult = ResultValue<{ runId: string } & RunOutcome, RunRejected>;
export type CompactionResult = ResultValue<{ runId: string } & CompactionOutcome, CompactionRejected>;
export type NavigationResult = ResultValue<{ runId: string } & NavigationOutcome, NavigationRejected>;
export type QueueResult = ResultValue<{ entryId: string }, QueueRejected>;
export type CancelQueuedResult = ResultValue<
	{ outcome: "cancelled" | "already_consumed" | "already_cleared" },
	CancelQueuedRejected
>;
export type RecordUsageResult = ResultValue<void, Closed>;
export type AbortResult = ResultValue<
	{ runId: string; steer: AgentMessage[]; followUp: AgentMessage[] },
	AbortRejected
>;

export type ResumeOutcome =
	| ({ operation: "run"; runId: string } & RunOutcome)
	| ({ operation: "compaction"; runId: string } & CompactionOutcome)
	| ({ operation: "navigation"; runId: string } & NavigationOutcome);
export type ResumeResult = ResultValue<ResumeOutcome, ResumeRejected>;
export type CreateLaneResult = ResultValue<AgentLane, LaneExists | InvalidLane | UnknownTarget | Closed>;

export interface NavigateOptions {
	summarize?: boolean;
	customInstructions?: string;
	label?: string;
}

export interface SuspendedOperation {
	lane: string;
	kind: "run" | "compaction" | "navigation";
	id: string;
	startedAt: number;
	reason: "crash" | "deferred";
	prompt?: AgentMessage[];
	deferred?: DeferredHandle;
	aborting?: { steer: AgentMessage[]; followUp: AgentMessage[] };
	missing: { tools: string[]; models: string[] };
}

export interface LaneInfo {
	name: string;
	leafId: string | null;
	operation: null | {
		id: string;
		kind: "run" | "compaction" | "navigation";
		status: "running" | "suspended" | "aborting";
	};
}

export interface QueuedItem {
	entryId: string;
	message: AgentMessage;
}

export interface LaneSnapshot {
	lane: string;
	transcript: Entry[];
	leafId: string | null;
	operation: LaneInfo["operation"];
	queues: { steer: QueuedItem[]; followUp: QueuedItem[]; nextRun: QueuedItem[] };
	pendingWrites: { id: string; entry: ProvisionedEntry }[];
	faulted: boolean;
}

export interface SessionSnapshot {
	lanes: (LaneInfo & { suspended?: SuspendedOperation })[];
	faulted: boolean;
}

export type ActionInfo =
	| { kind: "append_entry"; entryType: Entry["type"]; entryId: string }
	| { kind: "append_record"; recordType: string }
	| { kind: "move_lane"; to: string | null }
	| { kind: "set_fact"; fact: "name" | "label" }
	| { kind: "try_finish_run"; outcome: "completed" | "failed" }
	| { kind: "finish_operation"; outcome: "completed" | "declined" | "failed" | "aborted" }
	| { kind: "commit_follow_up" }
	| { kind: "consume_queue_item"; queue: "steer" | "followUp"; entryId: string }
	| { kind: "apply_pending_write"; entryId: string }
	| { kind: "stream_assistant"; step: "assistant" | "compaction" | "branch_summary"; attempt: number }
	| { kind: "execute_tool"; toolCallId: string; toolName: string }
	| { kind: "fetch_deferred" | "cancel_deferred"; provider: string; id: string }
	| { kind: "hook"; name: HookName }
	| { kind: "sleep"; delayMs: number };

export type HookName =
	| "before_run"
	| "before_resume"
	| "before_run_end"
	| "transform_context"
	| "before_request"
	| "before_payload"
	| "after_response"
	| "before_tool"
	| "after_tool"
	| "before_compaction"
	| "before_navigation";

export interface Hooks {
	on(name: HookName, handler: (event: unknown) => unknown | Promise<unknown>, options?: { id?: string }): () => void;
}

export interface Events {
	on(type: string, listener: (event: unknown) => void | Promise<void>): () => void;
}

class PassiveRegistry implements Hooks, Events {
	on(
		_name: HookName | string,
		_handler: (event: unknown) => unknown | Promise<unknown>,
		_options?: { id?: string },
	): () => void {
		return () => {};
	}
}

export interface ExecutionSpan extends ExecutionContext {
	addEvent(name: string, attributes?: SpanAttributes): void;
	setAttributes(attributes: SpanAttributes): void;
	end(result: SpanEnd): void;
}

export interface ExecutionContext {
	startSpan(name: string, attributes?: SpanAttributes): ExecutionSpan;
}
export interface SpanAttributes {
	[name: string]: string | number | boolean | undefined;
}
export interface SpanEnd {
	status: "ok" | "error";
	error?: { name: string; message: string };
	attributes?: SpanAttributes;
}

export type HarnessTool = AgentTool & { replay?: "never" | "safe" };
export type Resources = AgentHarnessResources<Skill, PromptTemplate>;
export type StreamOptions = SimpleStreamOptions;
export type StreamOptionsPatch = Partial<SimpleStreamOptions>;
export type EntryProjector = (entry: Entry) => AgentMessage[] | Promise<AgentMessage[]>;

export interface AgentHarnessOptions {
	session: Session;
	models: Models;
	model: Model<Api>;
	thinkingLevel?: ThinkingLevel;
	activeToolNames?: string[];
	tools?: HarnessTool[];
	toolContext?: object | (() => object | Promise<object>);
	systemPrompt?: string | (() => string | Promise<string>);
	resources?: Resources;
	streamOptions?: StreamOptions;
	retry?: RetryPolicy;
	compaction?: CompactionSettings;
	steeringMode?: QueueMode;
	followUpMode?: QueueMode;
	toolExecution?: "sequential" | "parallel";
	drive?: "automatic" | "manual";
	toProviderMessages?: (messages: AgentMessage[]) => Message[] | Promise<Message[]>;
	entryProjectors?: Record<string, EntryProjector>;
	context?: ExecutionContext;
}
```
Source: ice:packages/agent/src/harness/agent-harness.ts:83-272.
Supplemental tool-context and stream-patch declarations: ```ts
/** Tool definition executed by an {@link AgentHarness} with an application-defined context. */
export type AgentHarnessTool<
	TContext extends object | undefined,
	TParameters extends TSchema = TSchema,
	TDetails = unknown,
> = Omit<AgentTool<TParameters, TDetails>, "execute"> & {
	/** Execute the tool call with the context resolved for the current turn snapshot. */
	execute(
		toolCallId: string,
		params: Static<TParameters>,
		signal: AbortSignal | undefined,
		onUpdate: AgentToolUpdateCallback<TDetails> | undefined,
		context: TContext,
	): Promise<AgentToolResult<TDetails>>;
};

/** Static tool context or zero-argument provider resolved for each turn snapshot. */
export type AgentHarnessToolContextSource<TContext extends object | undefined> =
	| TContext
	| (() => TContext | Promise<TContext>);

/** Curated provider request options owned by the harness and snapshotted per turn. */
export interface AgentHarnessStreamOptions {
	/** Preferred transport forwarded to the stream function. */
	transport?: Transport;
	/** Provider request timeout in milliseconds. */
	timeoutMs?: number;
	/** Maximum provider retry attempts. */
	maxRetries?: number;
	/** Optional cap for provider-requested retry delays. */
	maxRetryDelayMs?: number;
	/** Additional request headers merged with auth and lifecycle headers. */
	headers?: Record<string, string>;
	/** Provider metadata forwarded with requests. */
	metadata?: SimpleStreamOptions["metadata"];
	/** Provider cache retention hint. */
	cacheRetention?: SimpleStreamOptions["cacheRetention"];
}

/** Per-request stream option patch returned by provider hooks. */
export interface AgentHarnessStreamOptionsPatch
	extends Omit<Partial<AgentHarnessStreamOptions>, "headers" | "metadata"> {
	/** Header patch. `undefined` values delete keys; explicit `headers: undefined` clears all headers. */
	headers?: Record<string, string | undefined>;
	/** Metadata patch. `undefined` values delete keys; explicit `metadata: undefined` clears all metadata. */
	metadata?: Record<string, unknown | undefined>;
}
```
Source: ice:packages/agent/src/harness/types.ts:80-126.
None: additional CLI flags, slash commands, keybindings, core settings keys, agent RPC commands, or persistent files beyond those explicitly specified. This entry concerns only the named in-process APIs/events.

### 6. Dependencies

`AGENT-036`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

### 7. ice-rust today

rust:crates/ice-harness/src/types.rs:1-526; rust:crates/ice-harness/src/agent_harness.rs:649-810. Rust has native result/error/hook/event types and operational harness features instead of the same passive registry. Names and shapes differ. This entry is declaration coverage, not a requirement to add nonfunctional hooks or discard Rust safety/recovery behavior.

### 8. Port notes

Keep Rust enums/traits and serialize explicit compatibility DTOs only where a consumer needs this exact surface. No plugin API addition is needed to imitate a no-op registry.

### 9. Size and priority

Estimated Rust implementation: **0-80 lines** (estimate, not measured source). **P3** - Distinguishes enumerated names from reachable capabilities and preserves useful Rust behavior.

## AGENT-034 - Scaffold configuration copies and unchecked setters

### 1. Title and status

Scaffold configuration copies and unchecked setters. **Status: different-by-design.**

### 2. ice source

ice:packages/agent/src/harness/agent-harness.ts:332-352; ice:packages/agent/src/harness/agent-harness.ts:423-440; ice:packages/agent/src/harness/agent-harness.ts:473-521. AgentHarness getters/setters. Tests: no dedicated setter/copy-depth test in scaffold test; explicit implementation is the authority.

### 3. What it does

Store local model, tools, resources and policy values without persisting them or applying them to an executing run.

### 4. How it works

Construction keeps the supplied session/model by reference; sets thinking off; active tools to supplied activeToolNames, else every supplied tool name, else[]; copies tool/resource arrays; shallow-copies stream options; retains supplied retry/compaction objects or uses defaults. Defaults: retry {enabled:false, maxRetries:0, baseDelayMs:1000}; compaction {enabled:true, reserveTokens:16384, keepRecentTokens:20000}; both queue modes one-at-a-time. No tool-name/model/settings validation occurs.

getModel returns the same model reference; setModel/setThinkingLevel replace local fields. getActiveTools/getTools return array copies; setActiveTools accepts any names, setTools resets active names to its explicit list or all replacement tool names. getResources/setResources copy only arrays, not item objects. Stream/retry/compaction getters/setters make one-level object copies: nested headers/metadata objects remain shared. setStreamOptions replaces the whole object, not a merge/patch and not the key-deletion semantics documented by AgentHarnessStreamOptionsPatch. Queue mode getters/setters only store values. None of these methods checks closed, emits a view/event, appends session entries or invokes a provider.

### 5. Interfaces

```ts
	private constructor(options: AgentHarnessOptions) {
		this.durableSession = options.session;
		this.session = options.session;
		this.model = options.model;
		this.thinkingLevel = options.thinkingLevel ?? "off";
		this.activeToolNames = [...(options.activeToolNames ?? options.tools?.map((tool) => tool.name) ?? [])];
		this.tools = [...(options.tools ?? [])];
		this.resources = {
			skills: options.resources?.skills ? [...options.resources.skills] : undefined,
			promptTemplates: options.resources?.promptTemplates ? [...options.resources.promptTemplates] : undefined,
		};
		this.streamOptions = { ...(options.streamOptions ?? {}) };
		this.retryPolicy = options.retry ?? { enabled: false, maxRetries: 0, baseDelayMs: 1000 };
		this.compactionSettings = options.compaction ?? {
			enabled: true,
			reserveTokens: 16384,
			keepRecentTokens: 20000,
		};
		this.steeringMode = options.steeringMode ?? "one-at-a-time";
		this.followUpMode = options.followUpMode ?? "one-at-a-time";
	}
```
Source: ice:packages/agent/src/harness/agent-harness.ts:332-352.
```ts
	async getModel(): Promise<Model<Api>> {
		return this.model;
	}
	async setModel(model: Model<Api>): Promise<void> {
		this.model = model;
	}
	async getThinkingLevel(): Promise<ThinkingLevel> {
		return this.thinkingLevel;
	}
	async setThinkingLevel(level: ThinkingLevel): Promise<void> {
		this.thinkingLevel = level;
	}
	async getActiveTools(): Promise<string[]> {
		return [...this.activeToolNames];
	}
	async setActiveTools(names: string[]): Promise<void> {
		this.activeToolNames = [...names];
	}
```
Source: ice:packages/agent/src/harness/agent-harness.ts:423-440.
```ts
	async getTools(): Promise<HarnessTool[]> {
		return [...this.tools];
	}
	async setTools(tools: HarnessTool[], activeNames?: string[]): Promise<void> {
		this.tools = [...tools];
		this.activeToolNames = [...(activeNames ?? tools.map((tool) => tool.name))];
	}
	async getResources(): Promise<Resources> {
		return {
			skills: this.resources.skills ? [...this.resources.skills] : undefined,
			promptTemplates: this.resources.promptTemplates ? [...this.resources.promptTemplates] : undefined,
		};
	}
	async setResources(resources: Resources): Promise<void> {
		this.resources = {
			skills: resources.skills ? [...resources.skills] : undefined,
			promptTemplates: resources.promptTemplates ? [...resources.promptTemplates] : undefined,
		};
	}
	async getStreamOptions(): Promise<StreamOptions> {
		return { ...this.streamOptions };
	}
	async setStreamOptions(options: StreamOptions): Promise<void> {
		this.streamOptions = { ...options };
	}
	async getRetryPolicy(): Promise<RetryPolicy> {
		return { ...this.retryPolicy };
	}
	async setRetryPolicy(policy: RetryPolicy): Promise<void> {
		this.retryPolicy = { ...policy };
	}
	async getCompactionSettings(): Promise<CompactionSettings> {
		return { ...this.compactionSettings };
	}
	async setCompactionSettings(settings: CompactionSettings): Promise<void> {
		this.compactionSettings = { ...settings };
	}
	async getSteeringMode(): Promise<QueueMode> {
		return this.steeringMode;
	}
	async setSteeringMode(mode: QueueMode): Promise<void> {
		this.steeringMode = mode;
	}
	async getFollowUpMode(): Promise<QueueMode> {
		return this.followUpMode;
	}
	async setFollowUpMode(mode: QueueMode): Promise<void> {
		this.followUpMode = mode;
	}
```
Source: ice:packages/agent/src/harness/agent-harness.ts:473-521.
None: additional CLI flags, slash commands, keybindings, core settings keys, agent RPC commands, or persistent files beyond those explicitly specified. This entry concerns only the named in-process APIs/events.

### 6. Dependencies

`AGENT-032`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

### 7. ice-rust today

rust:crates/ice-harness/src/agent_harness.rs:2005-2190; rust:crates/ice-harness/src/types.rs:1-526. Rust exposes operational setters using owned data and shared harness state, not JavaScript reference aliasing. Its configured retry/compaction policy participates in runs. Preserve native ownership and persistence semantics; a compatibility facade may normalize setter replacement rules without emulating unsafe shared-object mutation.

### 8. Port notes

Target ice-harness SDK facade. No source-model registry duplication or provider engine is required. Treat JSON/object copy depth as a boundary contract, not a reason to introduce aliased mutable Rust state.

### 9. Size and priority

Estimated Rust implementation: **30-90 lines** (estimate, not measured source). **P2** - Prevents integration callers from assuming these setters perform validation or durable state transitions.

## AGENT-035 - Scaffold lane lookup, static watches and close flag

### 1. Title and status

Scaffold lane lookup, static watches and close flag. **Status: different-by-design.**

### 2. ice source

ice:packages/agent/src/harness/agent-harness.ts:315-330; ice:packages/agent/src/harness/agent-harness.ts:364-366; ice:packages/agent/src/harness/agent-harness.ts:441-472; ice:packages/agent/src/harness/agent-harness.ts:522-531. watch/watchSession/lane/lanes/close. Tests: ice:packages/agent/test/harness/agent-harness-scaffold.test.ts:8-20 (`opens the main lane over a v4 session`); ice:packages/agent/test/harness/agent-harness-scaffold.test.ts:22-33 (`rejects unimplemented operations explicitly`).

### 3. What it does

Read a static main-lane/session snapshot but provide no live watch stream or second lane handle.

### 4. How it works

name is always main. getLeafId delegates to durableSession.getLeafId() without a lane argument or closed check. watch first reads that leaf, then fetches findEntriesOnBranch({start:leafId, order:oldestFirst}) when nonnull, and returns the snapshot below; start/unsubscribe are empty methods. There is no snapshot/watch race protection or event buffering. lane(name) returns this only for main, otherwise undefined. lanes asks the underlying session for every durable lane and maps it to name/leafId/operation:null; this can advertise lanes for which lane(name) returns undefined. watchSession snapshots that list, faultedfalse, with empty start/unsubscribe.

close sets one boolean only. It does not close the session/storage, stop processes, abort active work, unsubscribe resources or prevent getters/configuration updates. Only unavailable operations switch from HarnessNotImplemented to HarnessClosed. No transcript/operation/queue/pending-write updates are manufactured from model output.

### 5. Interfaces

```ts
	async getLeafId(): Promise<string | null> {
		return this.durableSession.getLeafId();
	}
```
Source: ice:packages/agent/src/harness/agent-harness.ts:364-366.
```ts
	async watch(): Promise<WatchHandle<LaneSnapshot>> {
		const leafId = await this.getLeafId();
		const transcript =
			leafId === null ? [] : await this.session.findEntriesOnBranch({ start: leafId, order: "oldestFirst" });
		return {
			snapshot: {
				lane: this.name,
				transcript,
				leafId,
				operation: null,
				queues: { steer: [], followUp: [], nextRun: [] },
				pendingWrites: [],
				faulted: false,
			},
			start() {},
			unsubscribe() {},
		};
	}

	async lane(name: string): Promise<AgentLane | undefined> {
		return name === "main" ? this : undefined;
	}
	async createLane(_name: string, _at: string | null): Promise<CreateLaneResult> {
		return this.unavailable("createLane");
	}
	async lanes(): Promise<LaneInfo[]> {
		return (await this.durableSession.getLanes()).map(({ lane, leafId }) => ({
			name: lane,
			leafId,
			operation: null,
		}));
	}
```
Source: ice:packages/agent/src/harness/agent-harness.ts:441-472.
```ts
	async watchSession(): Promise<WatchHandle<SessionSnapshot>> {
		return {
			snapshot: { lanes: await this.lanes(), faulted: false },
			start() {},
			unsubscribe() {},
		};
	}
	async close(): Promise<void> {
		this.closed = true;
	}
```
Source: ice:packages/agent/src/harness/agent-harness.ts:522-531.
None: additional CLI flags, slash commands, keybindings, core settings keys, agent RPC commands, or persistent files beyond those explicitly specified. This entry concerns only the named in-process APIs/events.

### 6. Dependencies

`AGENT-032`, `AGENT-034`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

### 7. ice-rust today

rust:crates/ice-harness/src/agent_harness.rs:649-668; rust:crates/ice-harness/src/agent_harness.rs:2005-2190. Rust implements lane-bound shared runners and event/state ownership instead of static no-op watches. Its full watch/close behavior belongs to the session audit. Current TypeScript does not justify replacing that implementation with the scaffold’s static view.

### 8. Port notes

No functional port is required for no-op watch behavior. Add a DTO projection only for clients that explicitly target this scaffold contract; retain native live watches and resource cleanup.

### 9. Size and priority

Estimated Rust implementation: **0-50 lines** (estimate, not measured source). **P3** - Avoids falsely classifying Rust’s working lanes and lifecycle as missing.

## AGENT-036 - Tagged errors and explicit Result adapters

### 1. Title and status

Tagged errors and explicit Result adapters. **Status: different-by-design.**

### 2. ice source

ice:packages/agent/src/harness/result.ts:1-64; ice:packages/agent/src/harness/types.ts:5-38. Result/TaggedError/matchError/ok/err/getOrThrow/getOrUndefined/toError. Tests: scaffold rejection assertions ice:packages/agent/test/harness/agent-harness-scaffold.test.ts:22-33; Node environment tests consume getOrThrow; no dedicated TaggedError serialization test in this package.

### 3. What it does

Expose predictable result discrimination and error metadata without relying only on thrown strings.

### 4. How it works

Two modules define the same structural Result union. result.ts exports Result.ok/err/isOk/isErr; harness/types.ts exports standalone ok/err, getOrThrow, object-constrained getOrUndefined and toError. toError preserves Error instances, wraps strings, else tries JSON.stringify and falls back to String only when serialization throws. getOrThrow throws exactly result.error; no wrapping. The object constraint on getOrUndefined is compile-time only.

TaggedError(tag) creates a distinct Error subclass with _tag=tag/name=tag and then Object.assign(props), so untyped properties can overwrite name/_tag or other fields. static is tests instanceof that particular generated class, not string equality. toJSON collects enumerable own fields except _tag and returns {_tag:factoryTag, message:this.message,...payload}; enumerable props can override message in the spread. Native stack/cause may be absent unless copied as enumerable properties. matchError indexes handlers by error._tag and invokes directly; a missing handler throws. No file or network serialization happens automatically.

### 5. Interfaces

```ts
export type Result<TValue, TError> = { ok: true; value: TValue } | { ok: false; error: TError };

export const Result = {
	ok<TValue>(value: TValue): Result<TValue, never> {
		return { ok: true, value };
	},
	err<TError>(error: TError): Result<never, TError> {
		return { ok: false, error };
	},
	isOk<TValue, TError>(result: Result<TValue, TError>): result is { ok: true; value: TValue } {
		return result.ok;
	},
	isErr<TValue, TError>(result: Result<TValue, TError>): result is { ok: false; error: TError } {
		return !result.ok;
	},
};

export interface TaggedErrorValue<Tag extends string> extends Error {
	readonly _tag: Tag;
	toJSON(): { _tag: Tag; message: string } & Record<string, unknown>;
}

export interface TaggedErrorFactory<Tag extends string> {
	new <Props extends { message: string }>(props: Props): TaggedErrorValue<Tag> & Readonly<Props>;
	is(value: unknown): value is TaggedErrorValue<Tag>;
}

export function TaggedError<Tag extends string>(tag: Tag): TaggedErrorFactory<Tag> {
	class TaggedErrorClass extends Error {
		readonly _tag = tag;

		constructor(props: { message: string } & Record<string, unknown>) {
			super(props.message);
			this.name = tag;
			Object.assign(this, props);
		}

		toJSON(): { _tag: Tag; message: string } & Record<string, unknown> {
			const payload: Record<string, unknown> = {};
			for (const key of Object.keys(this)) {
				if (key !== "_tag") payload[key] = (this as unknown as Record<string, unknown>)[key];
			}
			return { _tag: tag, message: this.message, ...payload };
		}

		static is(value: unknown): value is TaggedErrorValue<Tag> {
			return value instanceof TaggedErrorClass;
		}
	}
	return TaggedErrorClass as unknown as TaggedErrorFactory<Tag>;
}

export type ErrorMatchers<TError extends TaggedErrorValue<string>, TValue> = {
	[Tag in TError["_tag"]]: (error: Extract<TError, { _tag: Tag }>) => TValue;
};

export function matchError<TError extends TaggedErrorValue<string>, TValue>(
	error: TError,
	matchers: ErrorMatchers<TError, TValue>,
): TValue {
	const matcher = (matchers as unknown as Record<string, (value: TError) => TValue>)[error._tag];
	return matcher(error);
}

```
Source: ice:packages/agent/src/harness/result.ts:1-64.
```ts
/** Result of a fallible operation. Expected failures are returned as `ok: false` instead of thrown. */
export type Result<TValue, TError> = { ok: true; value: TValue } | { ok: false; error: TError };

/** Create a successful {@link Result}. */
export function ok<TValue, TError>(value: TValue): Result<TValue, TError> {
	return { ok: true, value };
}

/** Create a failed {@link Result}. */
export function err<TValue, TError>(error: TError): Result<TValue, TError> {
	return { ok: false, error };
}

/** Return the success value or throw the failure error. Intended for tests and explicit adapter boundaries. */
export function getOrThrow<TValue, TError>(result: Result<TValue, TError>): TValue {
	if (!result.ok) throw result.error;
	return result.value;
}

/** Return the success value or `undefined`. Only object values are allowed to avoid truthiness bugs with primitives. */
export function getOrUndefined<TValue extends object, TError>(result: Result<TValue, TError>): TValue | undefined {
	return result.ok ? result.value : undefined;
}

/** Normalize unknown thrown values into Error instances before using them as typed error causes. */
export function toError(error: unknown): Error {
	if (error instanceof Error) return error;
	if (typeof error === "string") return new Error(error);
	try {
		return new Error(JSON.stringify(error));
	} catch {
		return new Error(String(error));
	}
}
```
Source: ice:packages/agent/src/harness/types.ts:5-38.
None: additional CLI flags, slash commands, keybindings, core settings keys, agent RPC commands, or persistent files beyond those explicitly specified. This entry concerns only the named in-process APIs/events.

### 6. Dependencies

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

### 7. ice-rust today

rust:crates/ice-harness/src/error.rs:10-80; rust:crates/ice-tools/src/error.rs:1-234. Rust uses native Result and typed error enums/structs. It has no need for JavaScript instanceof factories; exact _tag DTO compatibility is not supplied by native Display. ExecutionError stores only source text, while FileError can retain a source. Keep native causes/typed errors and implement explicit JSON conversion only at an API boundary.

### 8. Port notes

Target shared error/serialization adapters in ice-harness and ice-tools. thiserror/serde already cover the primitives. Do not mimic arbitrary property overwrite in trusted Rust enums.

### 9. Size and priority

Estimated Rust implementation: **45-90 lines** (estimate, not measured source). **P2** - Stable error payloads matter at SDK/RPC boundaries, but language-specific mechanics need not be copied.

## AGENT-037 - Execution environment interfaces and error classification

### 1. Title and status

Execution environment interfaces and error classification. **Status: partial.**

### 2. ice source

ice:packages/agent/src/harness/types.ts:128-176; ice:packages/agent/src/harness/types.ts:208-314; ice:packages/agent/src/harness/env/nodejs.ts:66-122; ice:packages/agent/src/harness/env/nodejs.ts:344-354. ExecutionEnv/FileSystem/Shell/FileError/ExecutionError/NodeExecutionEnv/toFileError. Tests: ice:packages/agent/test/harness/nodejs-env.test.ts:160-174 (`returns FileError for missing paths and keeps exists false for missing paths`); ice:packages/agent/test/harness/nodejs-env.test.ts:176-186 (`returns FileError for listing non-directories`); ice:packages/agent/test/harness/nodejs-env.test.ts:224-243 (`returns aborted results for pre-aborted cancellable file operations`).

### 3. What it does

Provide filesystem/process capabilities with structured results rather than model-facing tools or implicit sandboxing.

### 4. How it works

ExecutionEnv is FileSystem+Shell. FileSystem operations promise explicit Result failures, and cleanup promises best-effort completion. NodeExecutionEnv requires {cwd, shellPath?, shellEnv?}; it retains cwd exactly as supplied, exposes it as mutable, and tracks active child PIDs. It is a direct host-filesystem/process adapter, not a root restriction, trust policy, lock or sandbox. The thirteen file methods and shell contract are quoted below.

FileError preserves existing FileError unchanged, else wraps an Error cause. Node errno mapping is ABORT_ERR→aborted, ENOENT→not_found, EACCES/EPERM→permission_denied, ENOTDIR→not_directory, EISDIR→is_directory, EINVAL→invalid, everything else→unknown. Although not_supported is a declared code, this adapter never maps an errno to it. Unsupported stat object kind is invalid. Several signatures in the implementation omit the optional abort parameter declared by FileSystem and therefore ignore it; their individual entries document this. Pure path resolution happens outside some try/catch blocks, so untyped invalid path values can still throw despite the interface’s never-reject promise.

### 5. Interfaces

```ts
/** Kind of filesystem object as addressed by a {@link FileSystem}. Symlinks are not followed automatically. */
export type FileKind = "file" | "directory" | "symlink";

/** Stable, backend-independent file error codes returned by {@link FileSystem} file operations. */
export type FileErrorCode =
	| "aborted"
	| "not_found"
	| "permission_denied"
	| "not_directory"
	| "is_directory"
	| "invalid"
	| "not_supported"
	| "unknown";

/** Error returned by {@link FileSystem} file operations. */
export class FileError extends Error {
	/** Backend-independent error code. */
	public code: FileErrorCode;
	/** Absolute addressed path associated with the failure, when available. */
	public path?: string;

	constructor(code: FileErrorCode, message: string, path?: string, cause?: Error) {
		super(message, cause === undefined ? undefined : { cause });
		this.name = "FileError";
		this.code = code;
		this.path = path;
	}
}

/** Stable, backend-independent execution error codes returned by {@link ExecutionEnv.exec}. */
export type ExecutionErrorCode =
	| "aborted"
	| "timeout"
	| "shell_unavailable"
	| "spawn_error"
	| "callback_error"
	| "unknown";

/** Error returned by {@link ExecutionEnv.exec}. */
export class ExecutionError extends Error {
	/** Backend-independent error code. */
	public code: ExecutionErrorCode;

	constructor(code: ExecutionErrorCode, message: string, cause?: Error) {
		super(message, cause === undefined ? undefined : { cause });
		this.name = "ExecutionError";
		this.code = code;
	}
}
```
Source: ice:packages/agent/src/harness/types.ts:128-176.
```ts
/** Metadata for one filesystem object in a {@link FileSystem}. */
export interface FileInfo {
	/** Basename of {@link path}. */
	name: string;
	/** Absolute, syntactically normalized addressed path in the execution environment. Symlinks are not followed. */
	path: string;
	/** Object kind. Symlink targets are not followed; use {@link FileSystem.canonicalPath} explicitly. */
	kind: FileKind;
	/** Size in bytes for the addressed filesystem object. */
	size: number;
	/** Modification time as milliseconds since Unix epoch. */
	mtimeMs: number;
}

/**
 * Filesystem capability used by the harness.
 *
 * Paths passed to methods may be absolute or relative to {@link cwd}. Paths returned by file operations are addressed paths
 * in the filesystem namespace, but are not canonicalized through symlinks unless returned by {@link canonicalPath}.
 *
 * Operation methods must never throw or reject. All filesystem failures, including unexpected backend failures, must be
 * encoded in the returned {@link Result}. Implementations must preserve this invariant.
 */
export interface FileSystem {
	/** Current working directory for relative paths. */
	cwd: string;

	/** Return an absolute addressed path without requiring it to exist and without resolving symlinks. */
	absolutePath(path: string, abortSignal?: AbortSignal): Promise<Result<string, FileError>>;
	/** Join path segments in the filesystem namespace without requiring the result to exist. */
	joinPath(parts: string[], abortSignal?: AbortSignal): Promise<Result<string, FileError>>;
	/** Read a UTF-8 text file. */
	readTextFile(path: string, abortSignal?: AbortSignal): Promise<Result<string, FileError>>;
	/** Read UTF-8 text lines. Implementations should stop once `maxLines` lines have been read. */
	readTextLines(
		path: string,
		options?: { maxLines?: number; abortSignal?: AbortSignal },
	): Promise<Result<string[], FileError>>;
	/** Read a binary file. */
	readBinaryFile(path: string, abortSignal?: AbortSignal): Promise<Result<Uint8Array, FileError>>;
	/** Create or overwrite a file, creating parent directories when supported. */
	writeFile(path: string, content: string | Uint8Array, abortSignal?: AbortSignal): Promise<Result<void, FileError>>;
	/** Create or append to a file, creating parent directories when supported. */
	appendFile(path: string, content: string | Uint8Array, abortSignal?: AbortSignal): Promise<Result<void, FileError>>;
	/** Return metadata for the addressed path without following symlinks. */
	fileInfo(path: string, abortSignal?: AbortSignal): Promise<Result<FileInfo, FileError>>;
	/** List direct children of a directory without following symlinks. */
	listDir(path: string, abortSignal?: AbortSignal): Promise<Result<FileInfo[], FileError>>;
	/** Return the canonical path for an existing path, resolving symlinks where supported. */
	canonicalPath(path: string, abortSignal?: AbortSignal): Promise<Result<string, FileError>>;
	/** Return false for missing paths. Other errors, such as permission failures, return a {@link FileError}. */
	exists(path: string, abortSignal?: AbortSignal): Promise<Result<boolean, FileError>>;
	/** Create a directory. Defaults: `recursive: true`, no abort signal. */
	createDir(
		path: string,
		options?: { recursive?: boolean; abortSignal?: AbortSignal },
	): Promise<Result<void, FileError>>;
	/** Remove a file or directory. Defaults: `recursive: false`, `force: false`, no abort signal. */
	remove(
		path: string,
		options?: { recursive?: boolean; force?: boolean; abortSignal?: AbortSignal },
	): Promise<Result<void, FileError>>;
	/** Create a temporary directory and return its absolute path. Defaults: `prefix: "tmp-"`, no abort signal. */
	createTempDir(prefix?: string, abortSignal?: AbortSignal): Promise<Result<string, FileError>>;
	/** Create a temporary file and return its absolute path. Defaults: `prefix: ""`, `suffix: ""`, no abort signal. */
	createTempFile(options?: {
		prefix?: string;
		suffix?: string;
		abortSignal?: AbortSignal;
	}): Promise<Result<string, FileError>>;

	/** Release filesystem resources. Must be best-effort and must not throw or reject. */
	cleanup(): Promise<void>;
}

/** Options for {@link Shell.exec}. */
export interface ShellExecOptions {
	/** Working directory for the command. Relative paths are resolved against {@link ExecutionEnv.cwd}. Defaults to {@link ExecutionEnv.cwd}. */
	cwd?: string;
	/** Environment variables for the command. Values override inherited defaults when `inheritEnv` is true. */
	env?: Record<string, string>;
	/** Whether to inherit the execution environment's default variables. Defaults to true. */
	inheritEnv?: boolean;
	/** Timeout in seconds. Implementations should return a timeout error when the command exceeds this duration. Defaults to no timeout. */
	timeout?: number;
	/** Abort signal used to terminate the command. Defaults to no abort signal. */
	abortSignal?: AbortSignal;
	/** Called with stdout chunks as they are produced. */
	onStdout?: (chunk: string) => void;
	/** Called with stderr chunks as they are produced. */
	onStderr?: (chunk: string) => void;
}

/** Shell execution capability used by the harness. */
export interface Shell {
	/** Execute a shell command in {@link FileSystem.cwd} unless `options.cwd` is provided. */
	exec(
		command: string,
		options?: ShellExecOptions,
	): Promise<Result<{ stdout: string; stderr: string; exitCode: number }, ExecutionError>>;
	/** Release shell resources. Must be best-effort and must not throw or reject. */
	cleanup(): Promise<void>;
}

/** Filesystem and process execution environment used by the harness. */
export interface ExecutionEnv extends FileSystem, Shell {}

```
Source: ice:packages/agent/src/harness/types.ts:208-314.
```ts
function toFileError(error: unknown, path?: string): FileError {
	if (error instanceof FileError) return error;
	const cause = toError(error);
	if (isNodeError(error)) {
		const message = error.message;
		switch (error.code) {
			case "ABORT_ERR":
				return new FileError("aborted", message, path, cause);
			case "ENOENT":
				return new FileError("not_found", message, path, cause);
			case "EACCES":
			case "EPERM":
				return new FileError("permission_denied", message, path, cause);
			case "ENOTDIR":
				return new FileError("not_directory", message, path, cause);
			case "EISDIR":
				return new FileError("is_directory", message, path, cause);
			case "EINVAL":
				return new FileError("invalid", message, path, cause);
		}
	}
	return new FileError("unknown", cause.message, path, cause);
}

function abortResult<TValue>(signal: AbortSignal | undefined, path?: string): Result<TValue, FileError> | undefined {
	return signal?.aborted ? err(new FileError("aborted", "aborted", path)) : undefined;
}
```
Source: ice:packages/agent/src/harness/env/nodejs.ts:96-122.
Constructor ```ts
export class NodeExecutionEnv implements ExecutionEnv {
	cwd: string;
	private shellPath?: string;
	private shellEnv?: NodeJS.ProcessEnv;
	private activeChildPids = new Set<number>();

	constructor(options: { cwd: string; shellPath?: string; shellEnv?: NodeJS.ProcessEnv }) {
		this.cwd = options.cwd;
		this.shellPath = options.shellPath;
		this.shellEnv = options.shellEnv;
	}
```
Source: ice:packages/agent/src/harness/env/nodejs.ts:344-354.
None: additional CLI flags, slash commands, keybindings, core settings keys, agent RPC commands, or persistent files beyond those explicitly specified. This entry concerns only the named in-process APIs/events.

### 6. Dependencies

`AGENT-036`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

### 7. ice-rust today

rust:crates/ice-tools/src/env.rs:1-325; rust:crates/ice-tools/src/error.rs:1-234; rust:crates/ice-tools/src/os_env.rs:106-144. Rust traits cover this capability family and add rename_file/MutatingEnv and a shell-program override. ErrorKind AlreadyExists maps to Invalid, TimedOut/Interrupted to Aborted, Unsupported to NotSupported; TS maps corresponding unhandled errno to unknown. Rust retains FileError causes but flattens ExecutionError source to text; Display prefixes code/path unlike Error.message. Every native file method takes a cancellation option, more than the implemented TS methods honor.

### 8. Port notes

Target ice-tools::env/os_env/error, retaining native Result and nonthrowing adapters. Native plugins should request these host capabilities instead of receiving arbitrary mutable filesystem state. Keep trust/path authorization above this raw adapter.

### 9. Size and priority

Estimated Rust implementation: **80-140 lines** (estimate, not measured source). **P1** - Required for deterministic SDK, event and tool behavior during a TypeScript-to-Rust port.

## AGENT-038 - Home/file-URL path normalization and join semantics

### 1. Title and status

Home/file-URL path normalization and join semantics. **Status: partial.**

### 2. ice source

ice:packages/agent/src/harness/env/nodejs.ts:50-64; ice:packages/agent/src/harness/env/nodejs.ts:350-362. resolvePath/absolutePath/joinPath. Tests: ice:packages/agent/test/harness/nodejs-env.test.ts:98-104 (`expands home-relative paths and file URLs`); ice:packages/agent/test/harness/nodejs-env.test.ts:71-96 (`reads, writes, lists, and removes files and directories`).

### 3. What it does

Resolve logical paths without requiring existence, while keeping joining distinct from absolute/canonical resolution.

### 4. How it works

NodeExecutionEnv.absolutePath calls resolvePath(cwd, input): exact ~ expands to homedir; ~/ and, only on Windows, ~\ expand via join(homedir, remainder). A file:// prefix is parsed with Node fileURLToPath, including percent decoding and platform URL rules; a malformed URL is retained as a literal path. Then absolute input is normalized with resolve(input), relative input with resolve(cwd, input). Symlinks are not resolved. cwd is retained as supplied by the constructor, not normalized there. Neither absolutePath nor joinPath accepts/observes the optional abort signal declared by the interface.

joinPath returns Node path.join(...parts), not resolve(cwd,...parts). Relative joins stay relative; an empty parts array returns dot; later absolute-looking segments are joined rather than replacing all preceding segments as PathBuf::push does. Native platform separators/dot/dot-dot normalization apply. Invalid untyped arguments can throw before a Result is produced. No root containment or credential/path redaction is performed.

### 5. Interfaces

```ts
function resolvePath(cwd: string, path: string): string {
	let normalized = path;
	if (normalized === "~") {
		normalized = homedir();
	} else if (normalized.startsWith("~/") || (process.platform === "win32" && normalized.startsWith("~\\"))) {
		normalized = join(homedir(), normalized.slice(2));
	} else if (normalized.startsWith("file://")) {
		try {
			normalized = fileURLToPath(normalized);
		} catch {
			// Keep malformed URLs as ordinary paths so filesystem methods preserve their non-throwing contract.
		}
	}
	return isAbsolute(normalized) ? resolve(normalized) : resolve(cwd, normalized);
}
```
Source: ice:packages/agent/src/harness/env/nodejs.ts:50-64.
```ts
	constructor(options: { cwd: string; shellPath?: string; shellEnv?: NodeJS.ProcessEnv }) {
		this.cwd = options.cwd;
		this.shellPath = options.shellPath;
		this.shellEnv = options.shellEnv;
	}

	async absolutePath(path: string): Promise<Result<string, FileError>> {
		return ok(resolvePath(this.cwd, path));
	}

	async joinPath(parts: string[]): Promise<Result<string, FileError>> {
		return ok(join(...parts));
	}
```
Source: ice:packages/agent/src/harness/env/nodejs.ts:350-362.
None: additional CLI flags, slash commands, keybindings, core settings keys, agent RPC commands, or persistent files beyond those explicitly specified. This entry concerns only the named in-process APIs/events.

### 6. Dependencies

`AGENT-037`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

### 7. ice-rust today

rust:crates/ice-tools/src/os_env.rs:146-171; rust:crates/ice-tools/src/os_env.rs:236-270; rust:crates/ice-tools/src/os_env.rs:397-417. Rust file_url_to_path strips prefixes manually and does not percent-decode or enforce Node host/URL rules; a file URL containing spaces encoded as %20 targets a different filename. Rust join_path repeatedly pushes PathBuf components, returns an empty path for no components and does not perform Node join normalization. It also honors pre-cancellation. Adopt explicit lexical joining/URL conversion, not filesystem canonicalize, which would require existence and follow symlinks.

### 8. Port notes

Target ice-tools::OsExecutionEnv/FileSystem. Keep paths as owned PathBuf and errors as Result; use compatibility conversion only at public boundaries. No native plugin API extension beyond the filesystem capability is required. This adapter provides no sandbox, authorization or file mutation lock; those must remain in the calling policy/tool layer.

### 9. Size and priority

Estimated Rust implementation: **90-150 lines** (estimate, not measured source). **P1** - Required for deterministic SDK, event and tool behavior during a TypeScript-to-Rust port.

## AGENT-039 - Full UTF-8 text reads and malformed-byte policy

### 1. Title and status

Full UTF-8 text reads and malformed-byte policy. **Status: partial.**

### 2. ice source

ice:packages/agent/src/harness/env/nodejs.ts:499-508. readTextFile. Tests: ice:packages/agent/test/harness/nodejs-env.test.ts:71-96 (`reads, writes, lists, and removes files and directories`); ice:packages/agent/test/harness/nodejs-env.test.ts:224-243 (`returns aborted results for pre-aborted cancellable file operations`). Malformed UTF-8 behavior follows the explicit Node utf8 decoder; no dedicated malformed-byte test in this file.

### 3. What it does

Read a complete text file with replacement decoding rather than rejecting a file containing malformed UTF-8.

### 4. How it works

Resolve the path, return an aborted FileError before I/O when signalled, then call readFile with encoding utf8 and the AbortSignal. Return its decoded string or mapped FileError with resolved path/cause. Node UTF-8 decoding replaces malformed sequences instead of rejecting the whole read. No BOM removal, newline normalization, size limit or streaming/chunk return occurs. The resolved path is computed before the try block. Cancellation can interrupt reading, but this helper does not promise that the OS read did no work. The complete string is held in memory.

### 5. Interfaces

```ts
	async readTextFile(path: string, abortSignal?: AbortSignal): Promise<Result<string, FileError>> {
		const resolved = resolvePath(this.cwd, path);
		const aborted = abortResult<string>(abortSignal, resolved);
		if (aborted) return aborted;
		try {
			return ok(await readFile(resolved, { encoding: "utf8", signal: abortSignal }));
		} catch (error) {
			return err(toFileError(error, resolved));
		}
	}
```
Source: ice:packages/agent/src/harness/env/nodejs.ts:499-508.
None: additional CLI flags, slash commands, keybindings, core settings keys, agent RPC commands, or persistent files beyond those explicitly specified. This entry concerns only the named in-process APIs/events.

### 6. Dependencies

`AGENT-037`, `AGENT-038`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

### 7. ice-rust today

rust:crates/ice-tools/src/os_env.rs:419-430; rust:crates/ice-tools/src/os_env.rs:753-767. Rust reads the full bytes then String::from_utf8 returns Invalid on malformed bytes. It retains cancellation support but does not provide the replacement-decoding behavior. String::from_utf8_lossy on the complete buffer is the relevant compatibility choice, unlike per-chunk lossy decoding used by the shell path.

### 8. Port notes

Target ice-tools::OsExecutionEnv/FileSystem. Keep paths as owned PathBuf and errors as Result; use compatibility conversion only at public boundaries. No native plugin API extension beyond the filesystem capability is required. This adapter provides no sandbox, authorization or file mutation lock; those must remain in the calling policy/tool layer.

### 9. Size and priority

Estimated Rust implementation: **15-40 lines** (estimate, not measured source). **P1** - Required for deterministic SDK, event and tool behavior during a TypeScript-to-Rust port.

## AGENT-040 - Incremental text-line reads and exact limit behavior

### 1. Title and status

Incremental text-line reads and exact limit behavior. **Status: partial.**

### 2. ice source

ice:packages/agent/src/harness/env/nodejs.ts:510-539. readTextLines. Tests: ice:packages/agent/test/harness/nodejs-env.test.ts:153-158 (`stops reading text lines at the requested limit`); ice:packages/agent/test/harness/nodejs-env.test.ts:224-243 (`returns aborted results for pre-aborted cancellable file operations`).

### 3. What it does

Stop reading a text file after enough lines instead of loading the entire file before truncation.

### 4. How it works

Resolve/precheck cancellation. If maxLines is supplied and <=0 return[] without opening the file, even when it does not exist. Otherwise create a UTF-8 read stream with the signal and a readline interface with crlfDelay:Infinity. For each yielded line check cancellation, append it, then stop when length>=maxLines. Check cancellation again after the loop. Always close the line reader and destroy the stream in finally. Empty files produce no lines; a trailing line terminator does not create an extra final empty line; CRLF is one terminator and CR separators are handled by readline rather than retained in text.

No numeric validation is applied: a positive fractional limit stops at its ceiling, NaN never satisfies the comparison and effectively reads all, Infinity reads all. There is no per-line byte cap; a single enormous line can still use substantial memory. Native decoding replaces malformed UTF-8. Return an error rather than partial accumulated lines on cancellation or stream failure.

### 5. Interfaces

```ts
	async readTextLines(
		path: string,
		options?: { maxLines?: number; abortSignal?: AbortSignal },
	): Promise<Result<string[], FileError>> {
		const resolved = resolvePath(this.cwd, path);
		const aborted = abortResult<string[]>(options?.abortSignal, resolved);
		if (aborted) return aborted;
		if (options?.maxLines !== undefined && options.maxLines <= 0) return ok([]);
		let stream: ReturnType<typeof createReadStream> | undefined;
		let lineReader: ReturnType<typeof createInterface> | undefined;
		try {
			stream = createReadStream(resolved, { encoding: "utf8", signal: options?.abortSignal });
			lineReader = createInterface({ input: stream, crlfDelay: Infinity });
			const lines: string[] = [];
			for await (const line of lineReader) {
				const loopAbort = abortResult<string[]>(options?.abortSignal, resolved);
				if (loopAbort) return loopAbort;
				lines.push(line);
				if (options?.maxLines !== undefined && lines.length >= options.maxLines) break;
			}
			const afterReadAbort = abortResult<string[]>(options?.abortSignal, resolved);
			if (afterReadAbort) return afterReadAbort;
			return ok(lines);
		} catch (error) {
			return err(toFileError(error, resolved));
		} finally {
			lineReader?.close();
			stream?.destroy();
		}
	}
```
Source: ice:packages/agent/src/harness/env/nodejs.ts:510-539.
None: additional CLI flags, slash commands, keybindings, core settings keys, agent RPC commands, or persistent files beyond those explicitly specified. This entry concerns only the named in-process APIs/events.

### 6. Dependencies

`AGENT-039`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

### 7. ice-rust today

rust:crates/ice-tools/src/os_env.rs:433-454. Rust accepts only usize limits, reads the entire file through strict read_text_file, splits only on LF, retains CR characters and returns [""] for an empty file. Its truncate does not save I/O or memory. Implement buffered line iteration with compatible CR/LF decoding; reject nonsensical JSON limits deliberately instead of casting silently.

### 8. Port notes

Target ice-tools::OsExecutionEnv/FileSystem. Keep paths as owned PathBuf and errors as Result; use compatibility conversion only at public boundaries. No native plugin API extension beyond the filesystem capability is required. This adapter provides no sandbox, authorization or file mutation lock; those must remain in the calling policy/tool layer.

### 9. Size and priority

Estimated Rust implementation: **75-135 lines** (estimate, not measured source). **P1** - Required for deterministic SDK, event and tool behavior during a TypeScript-to-Rust port.

## AGENT-041 - Binary read and overwrite cancellation boundaries

### 1. Title and status

Binary read and overwrite cancellation boundaries. **Status: partial.**

### 2. ice source

ice:packages/agent/src/harness/env/nodejs.ts:541-569; ice:packages/agent/src/harness/types.ts:246-251. readBinaryFile/writeFile. Tests: ice:packages/agent/test/harness/nodejs-env.test.ts:71-96 (`reads, writes, lists, and removes files and directories`); ice:packages/agent/test/harness/nodejs-env.test.ts:224-243 (`returns aborted results for pre-aborted cancellable file operations`).

### 3. What it does

Read bytes unchanged and create parent directories before overwriting a file.

### 4. How it works

readBinaryFile resolves/prechecks then calls native readFile with signal and returns Uint8Array bytes without text decoding. writeFile resolves/prechecks, creates the parent from resolve(target,"..") recursively, checks cancellation again, then calls native writeFile with string-or-byte content and the signal. A cancellation after mkdir can leave newly created directories; a cancelled overwrite can leave partial/truncated data. No atomic rename, fsync, metadata preservation, maximum size, collision check or mutation queue is implemented by this adapter. Concurrent writers are not serialized here. Both methods map caught filesystem errors with the resolved target path.

### 5. Interfaces

```ts
	async readBinaryFile(path: string, abortSignal?: AbortSignal): Promise<Result<Uint8Array, FileError>> {
		const resolved = resolvePath(this.cwd, path);
		const aborted = abortResult<Uint8Array>(abortSignal, resolved);
		if (aborted) return aborted;
		try {
			return ok(await readFile(resolved, { signal: abortSignal }));
		} catch (error) {
			return err(toFileError(error, resolved));
		}
	}

	async writeFile(
		path: string,
		content: string | Uint8Array,
		abortSignal?: AbortSignal,
	): Promise<Result<void, FileError>> {
		const resolved = resolvePath(this.cwd, path);
		const aborted = abortResult<void>(abortSignal, resolved);
		if (aborted) return aborted;
		try {
			await mkdir(resolve(resolved, ".."), { recursive: true });
			const afterMkdirAbort = abortResult<void>(abortSignal, resolved);
			if (afterMkdirAbort) return afterMkdirAbort;
			await writeFile(resolved, content, { signal: abortSignal });
			return ok(undefined);
		} catch (error) {
			return err(toFileError(error, resolved));
		}
	}
```
Source: ice:packages/agent/src/harness/env/nodejs.ts:541-569.
None: additional CLI flags, slash commands, keybindings, core settings keys, agent RPC commands, or persistent files beyond those explicitly specified. This entry concerns only the named in-process APIs/events.

### 6. Dependencies

`AGENT-037`, `AGENT-038`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

### 7. ice-rust today

rust:crates/ice-tools/src/os_env.rs:456-484; rust:crates/ice-tools/src/os_env.rs:753-793. Byte behavior and parent creation are substantially present. Rust takes FileContent, opens/truncates the file before entering its write-vs-cancel select, and explicitly flushes buffered Tokio writes; Node passes cancellation into writeFile itself. Path URL and error-classification differences remain AGENT-037/038. Neither version provides transactional cancellation. Do not add a separate write lock here if the tool-level queue already owns it.

### 8. Port notes

Target ice-tools::OsExecutionEnv/FileSystem. Keep paths as owned PathBuf and errors as Result; use compatibility conversion only at public boundaries. No native plugin API extension beyond the filesystem capability is required. This adapter provides no sandbox, authorization or file mutation lock; those must remain in the calling policy/tool layer.

### 9. Size and priority

Estimated Rust implementation: **20-50 lines** (estimate, not measured source). **P2** - Required for deterministic SDK, event and tool behavior during a TypeScript-to-Rust port.

## AGENT-042 - Append parent errors and ignored TypeScript cancellation

### 1. Title and status

Append parent errors and ignored TypeScript cancellation. **Status: different-by-design.**

### 2. ice source

ice:packages/agent/src/harness/env/nodejs.ts:571-580; ice:packages/agent/src/harness/types.ts:248-251. appendFile. Test: ice:packages/agent/test/harness/nodejs-env.test.ts:188-194 (`appends to new files and creates parent directories`).

### 3. What it does

Append text or bytes after creating missing parent directories, with the current TypeScript helper ignoring an optional cancellation argument.

### 4. How it works

The FileSystem interface declares appendFile(path, content, abortSignal?), but NodeExecutionEnv implements only path/content. Resolve path, await recursive parent mkdir, await appendFile and return success; any caught mkdir/append error is returned with the target path. A pre-aborted third argument is ignored. There is no post-write readback, file lock, atomic multi-call append record, fsync or output cap. Native appends may interleave across processes; caller-level ordering remains necessary. Error precedence is the first failing parent or append operation.

### 5. Interfaces

```ts
	async appendFile(path: string, content: string | Uint8Array): Promise<Result<void, FileError>> {
		const resolved = resolvePath(this.cwd, path);
		try {
			await mkdir(resolve(resolved, ".."), { recursive: true });
			await appendFile(resolved, content);
			return ok(undefined);
		} catch (error) {
			return err(toFileError(error, resolved));
		}
	}
```
Source: ice:packages/agent/src/harness/env/nodejs.ts:571-580.
Interface declaration: ice:packages/agent/src/harness/types.ts:248-251. None: additional CLI flags, slash commands, keybindings, core settings keys, agent RPC commands, or persistent files beyond those explicitly specified. This entry concerns only the named in-process APIs/events.

### 6. Dependencies

`AGENT-037`, `AGENT-038`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

### 7. ice-rust today

rust:crates/ice-tools/src/os_env.rs:486-518. Rust checks cancellation before work, but discards create_dir_all errors and only reports a later open/write error. It writes and flushes the append before returning. Retain the safer cancellation support as an intentional difference and propagate the original parent error; do not remove Rust cancellation merely to copy the TypeScript omission.

### 8. Port notes

Target ice-tools::OsExecutionEnv/FileSystem. Keep paths as owned PathBuf and errors as Result; use compatibility conversion only at public boundaries. No native plugin API extension beyond the filesystem capability is required. This adapter provides no sandbox, authorization or file mutation lock; those must remain in the calling policy/tool layer.

### 9. Size and priority

Estimated Rust implementation: **20-45 lines** (estimate, not measured source). **P2** - Append error provenance and cancellation differences must be explicit; the Rust cancellation guard should be retained.

## AGENT-043 - Non-following metadata, fractional timestamps and root basename

### 1. Title and status

Non-following metadata, fractional timestamps and root basename. **Status: partial.**

### 2. ice source

ice:packages/agent/src/harness/env/nodejs.ts:66-89; ice:packages/agent/src/harness/env/nodejs.ts:582-589. fileKindFromStats/fileInfoFromStats/fileInfo. Test: ice:packages/agent/test/harness/nodejs-env.test.ts:106-136 (`returns fileInfo for files, directories, and symlinks without following symlinks`).

### 3. What it does

Describe the addressed file, directory or symlink without silently following it.

### 4. How it works

fileInfo resolves the logical path and calls lstat. Accept only file/directory/symlink; any other kind returns FileError(invalid,"Unsupported file type", path). Return {name:basename(path), path, kind, size, mtimeMs}. mtimeMs is the native numeric millisecond value and can include a fraction or a pre-epoch negative time. A symlink reports the link’s size/time, not target metadata. Node basename at a filesystem root can be empty. The implementation ignores the interface’s optional abort argument. It does not canonicalize, sort, stat a target or suppress permission failures.

### 5. Interfaces

```ts
function fileKindFromStats(stats: {
	isFile(): boolean;
	isDirectory(): boolean;
	isSymbolicLink(): boolean;
}): FileKind | undefined {
	if (stats.isFile()) return "file";
	if (stats.isDirectory()) return "directory";
	if (stats.isSymbolicLink()) return "symlink";
	return undefined;
}

function fileInfoFromStats(
	path: string,
	stats: { isFile(): boolean; isDirectory(): boolean; isSymbolicLink(): boolean; size: number; mtimeMs: number },
): Result<FileInfo, FileError> {
	const kind = fileKindFromStats(stats);
	if (!kind) return err(new FileError("invalid", "Unsupported file type", path));
	return ok({
		name: basename(path),
		path,
		kind,
		size: stats.size,
		mtimeMs: stats.mtimeMs,
	});
```
Source: ice:packages/agent/src/harness/env/nodejs.ts:66-89.
```ts
	async fileInfo(path: string): Promise<Result<FileInfo, FileError>> {
		const resolved = resolvePath(this.cwd, path);
		try {
			return fileInfoFromStats(resolved, await lstat(resolved));
		} catch (error) {
			return err(toFileError(error, resolved));
		}
	}
```
Source: ice:packages/agent/src/harness/env/nodejs.ts:582-589.
None: additional CLI flags, slash commands, keybindings, core settings keys, agent RPC commands, or persistent files beyond those explicitly specified. This entry concerns only the named in-process APIs/events.

### 6. Dependencies

`AGENT-037`, `AGENT-038`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

### 7. ice-rust today

rust:crates/ice-tools/src/os_env.rs:534-573. Rust uses symlink_metadata and the same kind restrictions, but mtime is integer milliseconds, pre-epoch/failed modified time becomes 0, and a missing file_name falls back to the entire path. Pre-cancellation is supported. Preserve link identity; document timestamp precision/root-name projection rather than claiming exact DTO parity.

### 8. Port notes

Target ice-tools::OsExecutionEnv/FileSystem. Keep paths as owned PathBuf and errors as Result; use compatibility conversion only at public boundaries. No native plugin API extension beyond the filesystem capability is required. This adapter provides no sandbox, authorization or file mutation lock; those must remain in the calling policy/tool layer.

### 9. Size and priority

Estimated Rust implementation: **25-55 lines** (estimate, not measured source). **P2** - Required for deterministic SDK, event and tool behavior during a TypeScript-to-Rust port.

## AGENT-044 - Directory listing order, special-file skipping and stat failures

### 1. Title and status

Directory listing order, special-file skipping and stat failures. **Status: partial.**

### 2. ice source

ice:packages/agent/src/harness/env/nodejs.ts:591-613. listDir. Tests: ice:packages/agent/test/harness/nodejs-env.test.ts:138-151 (`lists symlinks as symlinks`); ice:packages/agent/test/harness/nodejs-env.test.ts:176-186 (`returns FileError for listing non-directories`); ice:packages/agent/test/harness/nodejs-env.test.ts:224-243 (`returns aborted results for pre-aborted cancellable file operations`).

### 3. What it does

List direct children with logical paths, preserving filesystem enumeration order and excluding unsupported object kinds.

### 4. How it works

Resolve/precheck cancellation, call readdir with dirents, then process entries serially in the returned order. Before each child stat, check cancellation. Resolve child path and lstat it; include a successful FileInfo, skip a returned invalid unsupported-kind result, and return immediately on a thrown stat error with that child path. A listing error uses the parent path. There is no explicit sort, recursion, hidden-file filter, symlink traversal or maximum entry count. Failure discards any accumulated entries. No final cancellation check follows the last completed stat. A disappearing child is an error, not silently skipped.

### 5. Interfaces

```ts
	async listDir(path: string, abortSignal?: AbortSignal): Promise<Result<FileInfo[], FileError>> {
		const resolved = resolvePath(this.cwd, path);
		const aborted = abortResult<FileInfo[]>(abortSignal, resolved);
		if (aborted) return aborted;
		try {
			const entries = await readdir(resolved, { withFileTypes: true });
			const infos: FileInfo[] = [];
			for (const entry of entries) {
				const loopAbort = abortResult<FileInfo[]>(abortSignal, resolved);
				if (loopAbort) return loopAbort;
				const entryPath = resolve(resolved, entry.name);
				try {
					const info = fileInfoFromStats(entryPath, await lstat(entryPath));
					if (info.ok) infos.push(info.value);
				} catch (error) {
					return err(toFileError(error, entryPath));
				}
			}
			return ok(infos);
		} catch (error) {
			return err(toFileError(error, resolved));
		}
	}
```
Source: ice:packages/agent/src/harness/env/nodejs.ts:591-613.
None: additional CLI flags, slash commands, keybindings, core settings keys, agent RPC commands, or persistent files beyond those explicitly specified. This entry concerns only the named in-process APIs/events.

### 6. Dependencies

`AGENT-043`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

### 7. ice-rust today

rust:crates/ice-tools/src/os_env.rs:575-633. Rust has serial non-following enumeration and skips sockets/FIFOs, but also skips symlink_metadata errors classified NotSupported, whereas TS maps the thrown errno to an error. Timestamp/name differences follow AGENT-043. Ordering is deliberately not guaranteed lexical by either adapter; higher-level loaders apply their own sort.

### 8. Port notes

Target ice-tools::OsExecutionEnv/FileSystem. Keep paths as owned PathBuf and errors as Result; use compatibility conversion only at public boundaries. No native plugin API extension beyond the filesystem capability is required. This adapter provides no sandbox, authorization or file mutation lock; those must remain in the calling policy/tool layer.

### 9. Size and priority

Estimated Rust implementation: **20-45 lines** (estimate, not measured source). **P2** - Required for deterministic SDK, event and tool behavior during a TypeScript-to-Rust port.

## AGENT-045 - Canonical paths and existence of unsupported objects

### 1. Title and status

Canonical paths and existence of unsupported objects. **Status: partial.**

### 2. ice source

ice:packages/agent/src/harness/env/nodejs.ts:615-629. canonicalPath/exists. Tests: ice:packages/agent/test/harness/nodejs-env.test.ts:106-136 (`returns fileInfo for files, directories, and symlinks without following symlinks`); ice:packages/agent/test/harness/nodejs-env.test.ts:160-174 (`returns FileError for missing paths and keeps exists false for missing paths`).

### 3. What it does

Resolve existing symlinks explicitly and distinguish missing paths from other filesystem failures.

### 4. How it works

canonicalPath resolves the logical input then awaits realpath, returning the canonical path or a mapped FileError. exists calls this.fileInfo: true on a supported object, false only on not_found, otherwise propagate the error. Thus a dangling symlink exists because lstat succeeds, but a socket/FIFO is invalid rather than true. Both implementations omit the optional abort argument promised by FileSystem and therefore ignore it. Neither operation is an authorization check or a stable guard against a later symlink change.

### 5. Interfaces

```ts
	async canonicalPath(path: string): Promise<Result<string, FileError>> {
		const resolved = resolvePath(this.cwd, path);
		try {
			return ok(await realpath(resolved));
		} catch (error) {
			return err(toFileError(error, resolved));
		}
	}

	async exists(path: string): Promise<Result<boolean, FileError>> {
		const result = await this.fileInfo(path);
		if (result.ok) return ok(true);
		if (result.error.code === "not_found") return ok(false);
		return err(result.error);
	}
```
Source: ice:packages/agent/src/harness/env/nodejs.ts:615-629.
None: additional CLI flags, slash commands, keybindings, core settings keys, agent RPC commands, or persistent files beyond those explicitly specified. This entry concerns only the named in-process APIs/events.

### 6. Dependencies

`AGENT-043`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

### 7. ice-rust today

rust:crates/ice-tools/src/os_env.rs:635-659. Rust canonicalize is corresponding host behavior, but exists directly accepts any successful symlink_metadata, including unsupported special files. Both Rust methods precheck cancellation. A compatibility exists wrapper can use the same supported-kind classifier; retain explicit canonical-vs-addressed paths in security-sensitive callers.

### 8. Port notes

Target ice-tools::OsExecutionEnv/FileSystem. Keep paths as owned PathBuf and errors as Result; use compatibility conversion only at public boundaries. No native plugin API extension beyond the filesystem capability is required. This adapter provides no sandbox, authorization or file mutation lock; those must remain in the calling policy/tool layer.

### 9. Size and priority

Estimated Rust implementation: **20-45 lines** (estimate, not measured source). **P2** - Required for deterministic SDK, event and tool behavior during a TypeScript-to-Rust port.

## AGENT-046 - Directory creation and file/directory removal semantics

### 1. Title and status

Directory creation and file/directory removal semantics. **Status: partial.**

### 2. ice source

ice:packages/agent/src/harness/env/nodejs.ts:631-649; ice:packages/agent/src/harness/types.ts:260-269. createDir/remove. Test: ice:packages/agent/test/harness/nodejs-env.test.ts:206-222 (`honors createDir recursive false and remove recursive/force options`).

### 3. What it does

Create directories recursively by default and remove targets with explicit recursive/force options.

### 4. How it works

createDir resolves then mkdir({recursive:options.recursive??true}); remove resolves then rm({recursive:options.recursive??false, force:options.force??false}). Both catch/marshal errors; neither implementation observes the interface’s abortSignal. With recursive=false, Node rm refuses directories, including empty directories; with recursive=true it can remove a regular file or a tree. force suppresses missing-target errors, not general permission failures. No project-root containment, confirmation, dry-run, lock, rollback or concurrent-change fencing is supplied. A partially removed tree is not restored after error.

### 5. Interfaces

```ts
	async createDir(path: string, options?: { recursive?: boolean }): Promise<Result<void, FileError>> {
		const resolved = resolvePath(this.cwd, path);
		try {
			await mkdir(resolved, { recursive: options?.recursive ?? true });
			return ok(undefined);
		} catch (error) {
			return err(toFileError(error, resolved));
		}
	}

	async remove(path: string, options?: { recursive?: boolean; force?: boolean }): Promise<Result<void, FileError>> {
		const resolved = resolvePath(this.cwd, path);
		try {
			await rm(resolved, { recursive: options?.recursive ?? false, force: options?.force ?? false });
			return ok(undefined);
		} catch (error) {
			return err(toFileError(error, resolved));
		}
	}
```
Source: ice:packages/agent/src/harness/env/nodejs.ts:631-649.
None: additional CLI flags, slash commands, keybindings, core settings keys, agent RPC commands, or persistent files beyond those explicitly specified. This entry concerns only the named in-process APIs/events.

### 6. Dependencies

`AGENT-037`, `AGENT-038`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

### 7. ice-rust today

rust:crates/ice-tools/src/os_env.rs:661-700. Rust requires explicit bools and honors pre-cancellation. Its recursive branch always uses remove_dir_all, which rejects a regular file; its nonrecursive branch tries remove_file then remove_dir, allowing an empty directory and replacing the original file error with the directory attempt’s error. Match target-kind dispatch deliberately, and preserve safer policy checks above the adapter.

### 8. Port notes

Target ice-tools::OsExecutionEnv/FileSystem. Keep paths as owned PathBuf and errors as Result; use compatibility conversion only at public boundaries. No native plugin API extension beyond the filesystem capability is required. This adapter provides no sandbox, authorization or file mutation lock; those must remain in the calling policy/tool layer.

### 9. Size and priority

Estimated Rust implementation: **45-85 lines** (estimate, not measured source). **P1** - Required for deterministic SDK, event and tool behavior during a TypeScript-to-Rust port.

## AGENT-047 - Temporary directory/file naming and lifetime

### 1. Title and status

Temporary directory/file naming and lifetime. **Status: partial.**

### 2. ice source

ice:packages/agent/src/harness/env/nodejs.ts:651-675. createTempDir/createTempFile. Test: ice:packages/agent/test/harness/nodejs-env.test.ts:196-204 (`creates temporary directories and files`).

### 3. What it does

Create caller-managed temporary paths without automatically deleting them when the environment is cleaned up.

### 4. How it works

createTempDir defaults prefix tmp-, calls mkdtemp(join(os.tmpdir(), prefix)), and returns its path or a FileError without a target path. createTempFile always creates a new tmp- directory, joins <prefix default empty><hyphenated randomUUID><suffix default empty>, then writes an empty file and returns its path. On file creation failure the newly created directory remains. These helpers ignore the interface’s abort signal, impose no prefix/suffix validation or permissions beyond native defaults, and register no cleanup ownership. A supplied separator/traversal in a prefix/suffix is not rejected. cleanup later kills processes only; callers must remove temporary paths explicitly.

### 5. Interfaces

```ts
	async createTempDir(prefix: string = "tmp-"): Promise<Result<string, FileError>> {
		try {
			return ok(await mkdtemp(join(tmpdir(), prefix)));
		} catch (error) {
			return err(toFileError(error));
		}
	}

	async createTempFile(options?: { prefix?: string; suffix?: string }): Promise<Result<string, FileError>> {
		const dir = await this.createTempDir("tmp-");
		if (!dir.ok) return dir;
		const filePath = join(dir.value, `${options?.prefix ?? ""}${randomUUID()}${options?.suffix ?? ""}`);
		try {
			await writeFile(filePath, "");
			return ok(filePath);
		} catch (error) {
			return err(toFileError(error, filePath));
		}
	}

	async cleanup(): Promise<void> {
		for (const pid of this.activeChildPids) killProcessTree(pid);
		this.activeChildPids.clear();
	}
}
```
Source: ice:packages/agent/src/harness/env/nodejs.ts:651-675.
None: additional CLI flags, slash commands, keybindings, core settings keys, agent RPC commands, or persistent files beyond those explicitly specified. This entry concerns only the named in-process APIs/events.

### 6. Dependencies

`AGENT-037`, `AGENT-038`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

### 7. ice-rust today

rust:crates/ice-tools/src/os_env.rs:702-750; rust:crates/ice-tools/src/os_env.rs:795-798. Rust also leaves temporary paths for callers, but first creates a tempfile directory, forgets its guard and renames it to prefix+unhyphenatedUUID. Rename failure returns the original random path, silently losing the requested prefix. File names use unhyphenated UUIDs and pre-cancellation is checked. Prefer tempfile::Builder with explicit persistence for exact prefixes; do not introduce automatic deletion that invalidates returned artifact paths.

### 8. Port notes

Target ice-tools::OsExecutionEnv/FileSystem. Keep paths as owned PathBuf and errors as Result; use compatibility conversion only at public boundaries. No native plugin API extension beyond the filesystem capability is required. This adapter provides no sandbox, authorization or file mutation lock; those must remain in the calling policy/tool layer.

### 9. Size and priority

Estimated Rust implementation: **40-80 lines** (estimate, not measured source). **P2** - Required for deterministic SDK, event and tool behavior during a TypeScript-to-Rust port.

## AGENT-048 - Shell discovery, POSIX fallback and legacy WSL command transport

### 1. Title and status

Shell discovery, POSIX fallback and legacy WSL command transport. **Status: partial.**

### 2. ice source

ice:packages/agent/src/harness/env/nodejs.ts:124-235. getShellConfig/findBashOnPath/getBashShellConfig/isLegacyWslBashPath/runCommand. Tests: ice:packages/agent/test/harness/nodejs-env.test.ts:285-316 (`uses stdin command transport for legacy WSL bash paths`); ice:packages/agent/test/harness/nodejs-env.test.ts:411-425 (`returns shell unavailable and spawn errors`).

### 3. What it does

Choose a usable Bash-like shell and avoid passing command text through the legacy WSL bash.exe argv boundary.

### 4. How it works

On every exec call resolve shell configuration. A truthy explicit shellPath is checked with access(F_OK); missing returns shell_unavailable with the exact path. Existing paths are accepted without executable/type validation until spawn. For Windows try ProgramFiles then ProgramFiles(x86), each appended with Git\bin\bash.exe, then the first line of successful where bash.exe output. If none, return detailed shell_unavailable options/searched paths. POSIX tries /bin/bash, then first line of which bash, then falls back to sh without checking its existence.

Probe commands use a 5,000 ms timer, collect UTF-8 stdout, ignore stderr and return empty/statusnull on spawn error. The timer kills the process tree but waits for close; it is not a guaranteed hard deadline. A case-insensitive normalized path matching drive:\Windows\(System32|Sysnative)\bash.exe uses args[-s] and commandTransport stdin; every other path uses[-c] plus command argv. No PowerShell syntax detection or shell escaping of the command is performed. Probes have no caller AbortSignal; exec checks cancellation again only after spawning/arming its listener.

### 5. Interfaces

```ts
async function findBashOnPath(): Promise<string | null> {
	const result =
		process.platform === "win32"
			? await runCommand("where", ["bash.exe"], 5000)
			: await runCommand("which", ["bash"], 5000);
	if (result.status !== 0 || !result.stdout) return null;
	const firstMatch = result.stdout.trim().split(/\r?\n/)[0];
	return firstMatch && (await pathExists(firstMatch)) ? firstMatch : null;
}

interface ShellConfig {
	shell: string;
	args: string[];
	commandTransport?: "argv" | "stdin";
}

function isLegacyWslBashPath(path: string): boolean {
	const normalized = path.replace(/\//g, "\\").toLowerCase();
	return /^[a-z]:\\windows\\(?:system32|sysnative)\\bash\.exe$/.test(normalized);
}

function getBashShellConfig(shell: string): ShellConfig {
	return isLegacyWslBashPath(shell) ? { shell, args: ["-s"], commandTransport: "stdin" } : { shell, args: ["-c"] };
}

async function getShellConfig(customShellPath?: string): Promise<Result<ShellConfig, ExecutionError>> {
	if (customShellPath) {
		if (await pathExists(customShellPath)) {
			return ok(getBashShellConfig(customShellPath));
		}
		return err(new ExecutionError("shell_unavailable", `Custom shell path not found: ${customShellPath}`));
	}
	if (process.platform === "win32") {
		const candidates: string[] = [];
		const programFiles = process.env.ProgramFiles;
		if (programFiles) candidates.push(`${programFiles}\\Git\\bin\\bash.exe`);
		const programFilesX86 = process.env["ProgramFiles(x86)"];
		if (programFilesX86) candidates.push(`${programFilesX86}\\Git\\bin\\bash.exe`);
		for (const candidate of candidates) {
			if (await pathExists(candidate)) {
				return ok(getBashShellConfig(candidate));
			}
		}
		const bashOnPath = await findBashOnPath();
		if (bashOnPath) {
			return ok(getBashShellConfig(bashOnPath));
		}
		return err(
			new ExecutionError(
				"shell_unavailable",
				`No bash shell found. Options:\n` +
					`  1. Install Git for Windows: https://git-scm.com/download/win\n` +
					`  2. Add your bash to PATH (Cygwin, MSYS2, etc.)\n` +
					"  3. Configure an explicit shellPath\n\n" +
					`Searched Git Bash in:\n${candidates.map((path) => `  ${path}`).join("\n")}`,
			),
		);
	}

	if (await pathExists("/bin/bash")) {
		return ok(getBashShellConfig("/bin/bash"));
	}
	const bashOnPath = await findBashOnPath();
	if (bashOnPath) {
		return ok(getBashShellConfig(bashOnPath));
	}
	return ok({ shell: "sh", args: ["-c"] });
}
```
Source: ice:packages/agent/src/harness/env/nodejs.ts:168-235.
```ts
async function runCommand(
	command: string,
	args: string[],
	timeoutMs: number,
): Promise<{ stdout: string; status: number | null }> {
	return await new Promise((resolve) => {
		let stdout = "";
		let child: ReturnType<typeof spawn>;
		try {
			child = spawn(command, args, {
				stdio: ["ignore", "pipe", "ignore"],
				windowsHide: true,
			});
		} catch {
			resolve({ stdout: "", status: null });
			return;
		}
		const timeout = setTimeout(() => {
			if (child.pid) killProcessTree(child.pid);
		}, timeoutMs);
		child.stdout?.setEncoding("utf8");
		child.stdout?.on("data", (chunk: string) => {
			stdout += chunk;
		});
		child.on("error", () => {
			clearTimeout(timeout);
			resolve({ stdout: "", status: null });
		});
		child.on("close", (status) => {
			clearTimeout(timeout);
			resolve({ stdout, status });
		});
	});
}
```
Source: ice:packages/agent/src/harness/env/nodejs.ts:133-166.
Environment: ProgramFiles, ProgramFiles(x86), inherited PATH used by where/which. Constructor shellPath is an embedding option, not a new CLI flag. None: additional CLI flags, slash commands, keybindings, core settings keys, agent RPC commands, or persistent files beyond those explicitly specified. This entry concerns only the named in-process APIs/events.

### 6. Dependencies

`AGENT-037`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

### 7. ice-rust today

rust:crates/ice-tools/src/os_env.rs:173-227; rust:crates/ice-tools/src/os_env.rs:272-355. Rust has the same broad shell choices and WSL transport. Its POSIX which branch checks the entire trimmed stdout as one path, unlike TS first-line selection. Windows missing-shell diagnostics are shorter; probe timeout drops a wait_with_output future without the explicit TypeScript tree kill. Native per-call program/args overrides for PowerShell are an additional Rust capability to retain.

### 8. Port notes

Target ice-tools::OsExecutionEnv shell resolver. Existing tokio::process and platform support suffice. Preserve Rust’s explicit per-call shell override; no provider/plugin agent launcher is needed.

### 9. Size and priority

Estimated Rust implementation: **55-105 lines** (estimate, not measured source). **P2** - Required for deterministic SDK, event and tool behavior during a TypeScript-to-Rust port.

## AGENT-049 - Shell environment replacement and host-private variable handling

### 1. Title and status

Shell environment replacement and host-private variable handling. **Status: partial.**

### 2. ice source

ice:packages/agent/src/harness/env/nodejs.ts:237-248; ice:packages/agent/src/harness/env/nodejs.ts:413-424. getShellEnv. Tests: ice:packages/agent/test/harness/nodejs-env.test.ts:262-283 (`can replace rather than inherit the default shell environment`); ice:packages/agent/test/harness/nodejs-env.test.ts:251-260 (`executes commands in cwd with env overrides`).

### 3. What it does

Let a caller replace the complete inherited/default environment instead of only overlaying variables.

### 4. How it works

With inheritEnv omitted/true, build a new map by spreading process.env, then constructor shellEnv, then call env; later values win. With inheritEnv=false, return only the call env object, excluding both process.env and configured shellEnv. Values are not interpolated, redacted or validated beyond their TypeScript types; a constructor ProcessEnv value can be undefined. This helper does not strip host session/trace variables, so a nested program can inherit them unless the caller replaces the environment. Policy/capability selection must happen before supplying these maps.

### 5. Interfaces

```ts
function getShellEnv(
	baseEnv?: NodeJS.ProcessEnv,
	extraEnv?: Record<string, string>,
	inheritEnv = true,
): NodeJS.ProcessEnv {
	if (!inheritEnv) return { ...extraEnv };
	return {
		...process.env,
		...baseEnv,
		...extraEnv,
	};
}
```
Source: ice:packages/agent/src/harness/env/nodejs.ts:237-248.
Applied at ice:packages/agent/src/harness/env/nodejs.ts:413-424. None: additional CLI flags, slash commands, keybindings, core settings keys, agent RPC commands, or persistent files beyond those explicitly specified. This entry concerns only the named in-process APIs/events.

### 6. Dependencies

`AGENT-037`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

### 7. ice-rust today

rust:crates/ice-tools/src/os_env.rs:67-104; rust:crates/ice-tools/src/os_env.rs:840-867. Rust clears env then optionally copies process variables while excluding HOST_PRIVATE_ENV and SDK SUBAGENT_PARENT_ENV; however it applies constructor shell_env even when inherit_env=false. That violates the TypeScript replacement test and may expose configured credentials. Retain Rust’s intentional private-variable filtering on inherited env and correct the false branch to exclude defaults. Explicit per-call/default overlays can still intentionally reintroduce private names in Rust.

### 8. Port notes

Target ice-tools environment assembly. Preserve the existing SDK-owned private-variable list rather than duplicating names. No new dependency is needed.

### 9. Size and priority

Estimated Rust implementation: **25-55 lines** (estimate, not measured source). **P0** - inheritEnv=false must not leak configured default credentials or unintended process identity.

## AGENT-050 - Shell spawn, live pipe draining and process registration

### 1. Title and status

Shell spawn, live pipe draining and process registration. **Status: partial.**

### 2. ice source

ice:packages/agent/src/harness/env/nodejs.ts:364-497. NodeExecutionEnv.exec. Tests: ice:packages/agent/test/harness/nodejs-env.test.ts:353-371 (`streams stdout and stderr chunks`); ice:packages/agent/test/harness/nodejs-env.test.ts:373-382 (`reports a missing working directory before spawning`); ice:packages/agent/test/harness/nodejs-env.test.ts:438-447 (`captures large shell output to a full output file through the execution env`).

### 3. What it does

Run a command while consuming stdout/stderr concurrently so a full pipe cannot block the child indefinitely.

### 4. How it works

Precheck abort, validate timeout, choose cwd from a truthy call cwd resolved against environment cwd or use stored cwd, resolve the shell, then access cwd(F_OK). Any access failure becomes spawn_error with Working directory does not exist text even if it was a permission failure. A file used as cwd can pass access and fail at spawn. Spawn with the chosen args, detached=true only outside Windows, windowsHide=true, stdin piped only for legacy WSL stdin transport, otherwise ignored; stdout/stderr always pipes. Add child.pid to activeChildPids when present. For stdin transport ignore stdin errors and end(command) immediately.

Arm timeout and abort handling, set both output streams to UTF-8 and register data callbacks before awaiting waitForChildProcess. Data is therefore consumed while the child runs. Output is retained without a cap in this adapter; higher-level executeShellWithCapture owns truncation/artifacts. A synchronous or child error becomes spawn_error. On settle clear timeout, detach abort listener, remove PID and resolve once. There is no concurrency limit, process isolation, output log file or automatic retry at this layer.

### 5. Interfaces

```ts
	async exec(
		command: string,
		options?: ShellExecOptions,
	): Promise<Result<{ stdout: string; stderr: string; exitCode: number }, ExecutionError>> {
		if (options?.abortSignal?.aborted) return err(new ExecutionError("aborted", "aborted"));
		const timeoutMsResult = resolveTimeoutMs(options?.timeout);
		if (!timeoutMsResult.ok) return err(timeoutMsResult.error);
		const timeoutMs = timeoutMsResult.value;

		const cwd = options?.cwd ? resolvePath(this.cwd, options.cwd) : this.cwd;
		const shellConfig = await getShellConfig(this.shellPath);
		if (!shellConfig.ok) return shellConfig;
		try {
			await access(cwd, constants.F_OK);
		} catch (error) {
			const cause = toError(error);
			return err(
				new ExecutionError(
					"spawn_error",
					`Working directory does not exist: ${cwd}\nCannot execute bash commands.`,
					cause,
				),
			);
		}

		return await new Promise((resolvePromise) => {
			let stdout = "";
			let stderr = "";
			let settled = false;
			let timedOut = false;
			let callbackError: ExecutionError | undefined;
			let child: ReturnType<typeof spawn> | undefined;
			let timeoutId: ReturnType<typeof setTimeout> | undefined;

			const onAbort = () => {
				if (child?.pid) {
					killProcessTree(child.pid);
				}
			};

			const settle = (result: Result<{ stdout: string; stderr: string; exitCode: number }, ExecutionError>) => {
				if (timeoutId) clearTimeout(timeoutId);
				if (options?.abortSignal) options.abortSignal.removeEventListener("abort", onAbort);
				if (child?.pid) this.activeChildPids.delete(child.pid);
				if (settled) return;
				settled = true;
				resolvePromise(result);
			};

			try {
				const commandFromStdin = shellConfig.value.commandTransport === "stdin";
				child = spawn(
					shellConfig.value.shell,
					commandFromStdin ? shellConfig.value.args : [...shellConfig.value.args, command],
					{
						cwd,
						detached: process.platform !== "win32",
						env: getShellEnv(this.shellEnv, options?.env, options?.inheritEnv),
						stdio: [commandFromStdin ? "pipe" : "ignore", "pipe", "pipe"],
						windowsHide: true,
					},
				);
				if (child.pid) this.activeChildPids.add(child.pid);
				if (commandFromStdin) {
					child.stdin?.on("error", () => {});
					child.stdin?.end(command);
				}
			} catch (error) {
				const cause = toError(error);
				settle(err(new ExecutionError("spawn_error", cause.message, cause)));
				return;
			}
```
Source: ice:packages/agent/src/harness/env/nodejs.ts:364-435.
```ts
			timeoutId =
				timeoutMs !== undefined
					? setTimeout(() => {
							timedOut = true;
							if (child?.pid) {
								killProcessTree(child.pid);
							}
						}, timeoutMs)
					: undefined;

			if (options?.abortSignal) {
				if (options.abortSignal.aborted) {
					onAbort();
				} else {
					options.abortSignal.addEventListener("abort", onAbort, { once: true });
				}
			}

			child.stdout?.setEncoding("utf8");
			child.stderr?.setEncoding("utf8");
```
Source: ice:packages/agent/src/harness/env/nodejs.ts:437-456.
None: additional CLI flags, slash commands, keybindings, core settings keys, agent RPC commands, or persistent files beyond those explicitly specified. This entry concerns only the named in-process APIs/events.

### 6. Dependencies

`AGENT-037`, `AGENT-048`, `AGENT-049`, `AGENT-051`, `AGENT-052`, `AGENT-053`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

Final dependency classification. Shared contract references: `TOOLS-010`, `TOOLS-015`, `TOOLS-005`, `TOOLS-008`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

rust:crates/ice-tools/src/os_env.rs:800-1058. Rust constructs stdout/stderr futures but at 1044-1045 only awaits status_fut; neither reader is polled until the process exits. A child producing more than pipe capacity can stall, and a command waiting for a streamed callback receives none until exit. This is a source-proven scheduling defect, not a benchmark or executed reproducer. Rust also awaits WSL stdin write/flush before timeout/cancellation/output readers, adding another possible blocking boundary. Poll child wait and both readers together, with bounded post-exit cleanup.

### 8. Port notes

Target ice-tools::OsExecutionEnv::exec. Use tokio::select!/join or spawned reader tasks that begin immediately, and a single owned process lifecycle guard. No extra agent/controller or shell service is required.

### 9. Size and priority

Estimated Rust implementation: **150-260 lines** (estimate, not measured source). **P0** - Foreground shell execution can deadlock on ordinary large output because Rust does not poll its pipe readers while waiting.

## AGENT-051 - Incremental UTF-8 output and callback failure precedence

### 1. Title and status

Incremental UTF-8 output and callback failure precedence. **Status: partial.**

### 2. ice source

ice:packages/agent/src/harness/env/nodejs.ts:455-495. exec data callbacks and settlement. Tests: ice:packages/agent/test/harness/nodejs-env.test.ts:353-371 (`streams stdout and stderr chunks`); ice:packages/agent/test/harness/nodejs-env.test.ts:399-409 (`returns callback errors from exec stream handlers`).

### 3. What it does

Decode split UTF-8 sequences correctly and turn a throwing output callback into a structured execution failure.

### 4. How it works

setEncoding(utf8) on each child pipe maintains decoder state across chunks. For each data event append the decoded chunk to stdout/stderr before invoking the corresponding callback. Catch callback exceptions, normalize the cause, assign callbackError and kill the child tree. A later callback exception overwrites the stored one; callbacks are not disabled immediately, and output already received remains in local buffers. After child settlement callbackError takes precedence over timeout, cancellation and ordinary exit status. The failure Result contains the ExecutionError, not partial output. No stdout/stderr global ordering, per-chunk byte limit, callback deadline or output cap is promised. Stream errors outside the child-error/callback paths are not independently wrapped by this helper.

### 5. Interfaces

```ts
			child.stdout?.setEncoding("utf8");
			child.stderr?.setEncoding("utf8");
			child.stdout?.on("data", (chunk: string) => {
				stdout += chunk;
				try {
					options?.onStdout?.(chunk);
				} catch (error) {
					const cause = toError(error);
					callbackError = new ExecutionError("callback_error", cause.message, cause);
					onAbort();
				}
			});
			child.stderr?.on("data", (chunk: string) => {
				stderr += chunk;
				try {
					options?.onStderr?.(chunk);
				} catch (error) {
					const cause = toError(error);
					callbackError = new ExecutionError("callback_error", cause.message, cause);
					onAbort();
				}
			});

			void waitForChildProcess(child).then(
				(code) => {
					if (callbackError) {
						settle(err(callbackError));
						return;
					}
					if (timedOut) {
						settle(err(new ExecutionError("timeout", `timeout:${options?.timeout}`)));
						return;
					}
					if (options?.abortSignal?.aborted) {
						settle(err(new ExecutionError("aborted", "aborted")));
						return;
					}
					settle(ok({ stdout, stderr, exitCode: code ?? 0 }));
				},
				(error: Error) => settle(err(new ExecutionError("spawn_error", error.message, error))),
			);
```
Source: ice:packages/agent/src/harness/env/nodejs.ts:455-495.
None: additional CLI flags, slash commands, keybindings, core settings keys, agent RPC commands, or persistent files beyond those explicitly specified. This entry concerns only the named in-process APIs/events.

### 6. Dependencies

`AGENT-037`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

### 7. ice-rust today

rust:crates/ice-tools/src/os_env.rs:929-982; rust:crates/ice-tools/src/os_env.rs:1072-1095. Rust decodes each 8192-byte read independently with from_utf8_lossy, corrupting a multibyte codepoint split across reads. It invokes the callback before appending, ignores read errors, and does not catch callback panic despite a comment claiming callback errors are swallowed. There is no callback_error resolution path. Use a stateful UTF-8 decoder and explicit fallible callback adapter; a panic must not strand the child or PID tracking.

### 8. Port notes

Target ice-tools shell output decoder/callback boundary. Existing std/tokio primitives suffice; preserve incomplete UTF-8 tails between reads. Native plugins should return explicit callback errors where possible instead of unwinding across C ABI.

### 9. Size and priority

Estimated Rust implementation: **100-180 lines** (estimate, not measured source). **P0** - Output corruption and unhandled callback failure break tool evidence and can leave processes running.

## AGENT-052 - Timeout/abort validation, termination and final error ordering

### 1. Title and status

Timeout/abort validation, termination and final error ordering. **Status: partial.**

### 2. ice source

ice:packages/agent/src/harness/env/nodejs.ts:33-48; ice:packages/agent/src/harness/env/nodejs.ts:250-273; ice:packages/agent/src/harness/env/nodejs.ts:368-371; ice:packages/agent/src/harness/env/nodejs.ts:398-495. resolveTimeoutMs/onAbort/settle/killProcessTree. Tests: ice:packages/agent/test/harness/nodejs-env.test.ts:384-389 (`returns non-zero command exit codes as successful execution results`); ice:packages/agent/test/harness/nodejs-env.test.ts:391-397 (`returns timeout errors for commands exceeding the timeout`); ice:packages/agent/test/harness/nodejs-env.test.ts:427-436 (`returns an aborted result for aborted commands`); ice:packages/agent/test/harness/nodejs-env.test.ts:341-351 (`cleanup terminates active shell processes`).

### 3. What it does

Reject invalid timeouts before spawning, terminate a running process on cancellation and distinguish timeout from ordinary nonzero exit.

### 4. How it works

Undefined timeout means none. Reject nonfinite or <=0 seconds with timeout error Invalid timeout: must be a finite number of seconds. Multiply by 1000 and reject values above 2147483647 ms (2147483.647 seconds). Positive fractional values are accepted and passed to the JavaScript timer. After spawn, timer sets timedOut=true then kills the child tree; abort listener kills without setting timedOut. A signal already aborted after shell/cwd awaits is detected after spawn and immediately killed, not prevented from spawning. Both paths still await child settlement.

Successful child settlement resolves callback_error first, then timeout with message timeout:<originalseconds>, then aborted with message aborted, else success including nonzero exitCode. A signal-only exit with null code becomes 0. A child wait error returns spawn_error through the rejection branch rather than applying those success-branch priorities. No kill grace/escalation is used: POSIX sends SIGKILL to group, falling back to PID; Windows starts taskkill /F /T /PID without awaiting it. External cleanup termination alone can yield a successful execution result because it sets neither timedOut nor AbortSignal.

### 5. Interfaces

```ts
const MAX_TIMEOUT_MS = 2_147_483_647;
const MAX_TIMEOUT_SECONDS = MAX_TIMEOUT_MS / 1000;
const EXIT_STDIO_GRACE_MS = 100;

function resolveTimeoutMs(timeout: number | undefined): Result<number | undefined, ExecutionError> {
	if (timeout === undefined) return ok(undefined);
	if (!Number.isFinite(timeout) || timeout <= 0) {
		return err(new ExecutionError("timeout", "Invalid timeout: must be a finite number of seconds"));
	}

	const timeoutMs = timeout * 1000;
	if (timeoutMs > MAX_TIMEOUT_MS) {
		return err(new ExecutionError("timeout", `Invalid timeout: maximum is ${MAX_TIMEOUT_SECONDS} seconds`));
	}
	return ok(timeoutMs);
}
```
Source: ice:packages/agent/src/harness/env/nodejs.ts:33-48.
```ts
			const onAbort = () => {
				if (child?.pid) {
					killProcessTree(child.pid);
				}
			};

			const settle = (result: Result<{ stdout: string; stderr: string; exitCode: number }, ExecutionError>) => {
				if (timeoutId) clearTimeout(timeoutId);
				if (options?.abortSignal) options.abortSignal.removeEventListener("abort", onAbort);
				if (child?.pid) this.activeChildPids.delete(child.pid);
				if (settled) return;
				settled = true;
				resolvePromise(result);
```
Source: ice:packages/agent/src/harness/env/nodejs.ts:398-410.
```ts
			timeoutId =
				timeoutMs !== undefined
					? setTimeout(() => {
							timedOut = true;
							if (child?.pid) {
								killProcessTree(child.pid);
							}
						}, timeoutMs)
					: undefined;

			if (options?.abortSignal) {
				if (options.abortSignal.aborted) {
					onAbort();
				} else {
					options.abortSignal.addEventListener("abort", onAbort, { once: true });
				}
			}
```
Source: ice:packages/agent/src/harness/env/nodejs.ts:437-453.
```ts
			void waitForChildProcess(child).then(
				(code) => {
					if (callbackError) {
						settle(err(callbackError));
						return;
					}
					if (timedOut) {
						settle(err(new ExecutionError("timeout", `timeout:${options?.timeout}`)));
						return;
					}
					if (options?.abortSignal?.aborted) {
						settle(err(new ExecutionError("aborted", "aborted")));
						return;
					}
					settle(ok({ stdout, stderr, exitCode: code ?? 0 }));
				},
				(error: Error) => settle(err(new ExecutionError("spawn_error", error.message, error))),
			);
```
Source: ice:packages/agent/src/harness/env/nodejs.ts:478-495.
None: additional CLI flags, slash commands, keybindings, core settings keys, agent RPC commands, or persistent files beyond those explicitly specified. This entry concerns only the named in-process APIs/events.

### 6. Dependencies

`AGENT-037`, `AGENT-048`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

### 7. ice-rust today

rust:crates/ice-tools/src/os_env.rs:357-389; rust:crates/ice-tools/src/os_env.rs:984-1095; rust:crates/ice-tools/src/os_env.rs:1116-1140; rust:crates/ice-tools/src/error.rs:171-194. Rust validates the same finite range using Duration::from_secs_f64, but timeout error text casts seconds to u64 (fractional input such as 0.01 becomes 0 s). Rust awaits taskkill without a timeout, which can itself stall cancellation. It has no callback-error precedence and waits for status before reading output. Retain explicit cancellation state and fix hard-deadline ownership rather than treating timeout as proof the entire process tree is gone.

### 8. Port notes

Target ice-tools shell lifecycle. Use one deadline/cancellation state and a bounded kill/reap policy. Do not blindly rerun a timed-out command; it may already have performed side effects.

### 9. Size and priority

Estimated Rust implementation: **85-155 lines** (estimate, not measured source). **P0** - Timeout/cancellation must not hang during termination or misclassify an ambiguous side-effecting command.

## AGENT-053 - Post-exit pipe idle grace rather than a fixed drain deadline

### 1. Title and status

Post-exit pipe idle grace rather than a fixed drain deadline. **Status: partial.**

### 2. ice source

ice:packages/agent/src/harness/env/nodejs.ts:35-35; ice:packages/agent/src/harness/env/nodejs.ts:275-342. waitForChildProcess. Test: ice:packages/agent/test/harness/nodejs-env.test.ts:318-339 (`settles after the shell exits when a detached descendant retains inherited stdio`).

### 3. What it does

Finish promptly when descendants retain inherited pipes while still accepting trailing output that continues to arrive.

### 4. How it works

waitForChildProcess listens for child error/exit/close and each pipe end/data event. Initially an absent pipe counts ended. On exit record code, finish immediately if both streams ended, otherwise arm a 100 ms idle timer. Every data event after exit resets that timer. Each stream end updates its flag and finishes when both ended. close finishes immediately. Finalize is once-only: clear timer/remove listeners, destroy both streams and resolve the code. A child error clears listeners and rejects. The same implementation applies on all platforms, although the inherited-stdio regression test is Windows-gated.

Continuous descendant output can indefinitely reset the post-exit idle timer; it is not a total 100 ms deadline. Once idle elapses, later descendant output is intentionally discarded by destroying pipes. There is no requirement to wait for every descendant EOF, and no model-visible background shell is registered by this helper.

### 5. Interfaces

```ts
function waitForChildProcess(child: ChildProcess): Promise<number | null> {
	return new Promise((resolvePromise, reject) => {
		let settled = false;
		let exited = false;
		let exitCode: number | null = null;
		let postExitTimer: ReturnType<typeof setTimeout> | undefined;
		let stdoutEnded = child.stdout === null;
		let stderrEnded = child.stderr === null;

		const cleanup = (): void => {
			if (postExitTimer) clearTimeout(postExitTimer);
			child.removeListener("error", onError);
			child.removeListener("exit", onExit);
			child.removeListener("close", onClose);
			child.stdout?.removeListener("end", onStdoutEnd);
			child.stderr?.removeListener("end", onStderrEnd);
			child.stdout?.removeListener("data", onData);
			child.stderr?.removeListener("data", onData);
		};
		const finalize = (code: number | null): void => {
			if (settled) return;
			settled = true;
			cleanup();
			child.stdout?.destroy();
			child.stderr?.destroy();
			resolvePromise(code);
		};
		const maybeFinalizeAfterExit = (): void => {
			if (exited && stdoutEnded && stderrEnded) finalize(exitCode);
		};
		const armIdleTimer = (): void => {
			if (postExitTimer) clearTimeout(postExitTimer);
			postExitTimer = setTimeout(() => finalize(exitCode), EXIT_STDIO_GRACE_MS);
		};
		const onData = (): void => {
			if (exited && !settled) armIdleTimer();
		};
		const onStdoutEnd = (): void => {
			stdoutEnded = true;
			maybeFinalizeAfterExit();
		};
		const onStderrEnd = (): void => {
			stderrEnded = true;
			maybeFinalizeAfterExit();
		};
		const onError = (error: Error): void => {
			if (settled) return;
			settled = true;
			cleanup();
			reject(error);
		};
		const onExit = (code: number | null): void => {
			exited = true;
			exitCode = code;
			maybeFinalizeAfterExit();
			if (!settled) armIdleTimer();
		};
		const onClose = (code: number | null): void => finalize(code);

		child.stdout?.once("end", onStdoutEnd);
		child.stderr?.once("end", onStderrEnd);
		child.stdout?.on("data", onData);
		child.stderr?.on("data", onData);
		child.once("error", onError);
		child.once("exit", onExit);
		child.once("close", onClose);
	});
}
```
Source: ice:packages/agent/src/harness/env/nodejs.ts:275-342.
EXIT_STDIO_GRACE_MS=100. None: additional CLI flags, slash commands, keybindings, core settings keys, agent RPC commands, or persistent files beyond those explicitly specified. This entry concerns only the named in-process APIs/events.

### 6. Dependencies

`AGENT-037`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

### 7. ice-rust today

rust:crates/ice-tools/src/os_env.rs:1035-1065. Rust drains both readers only after child wait and bounds the entire join with a fixed 100 ms timeout; it does not reset on incoming data. That can truncate trailing output which TypeScript would continue accepting. The fixed bound is arguably safer against noisy descendants, but it is different-by-policy and does not fix the earlier pipe-draining deadlock. Decide idle-vs-total grace explicitly.

### 8. Port notes

Target ice-tools process/output state machine. Keep reader tasks running before exit, then implement an explicit idle timer or document a bounded-total policy. No new host/plugin surface is required.

### 9. Size and priority

Estimated Rust implementation: **75-130 lines** (estimate, not measured source). **P1** - Required for deterministic SDK, event and tool behavior during a TypeScript-to-Rust port.

## AGENT-054 - Best-effort process cleanup without temporary-file ownership

### 1. Title and status

Best-effort process cleanup without temporary-file ownership. **Status: different-by-design.**

### 2. ice source

ice:packages/agent/src/harness/env/nodejs.ts:250-273; ice:packages/agent/src/harness/env/nodejs.ts:671-675. cleanup/killProcessTree. Tests: ice:packages/agent/test/harness/nodejs-env.test.ts:245-249 (`cleanup is best-effort`); ice:packages/agent/test/harness/nodejs-env.test.ts:341-351 (`cleanup terminates active shell processes`).

### 3. What it does

Terminate currently tracked shell trees without deleting artifacts or claiming all descendants have exited.

### 4. How it works

cleanup iterates the current activeChildPids Set, calls killProcessTree for each, then clears the Set. POSIX group/PID kills are synchronous best-effort; Windows starts detached taskkill and returns without waiting. It does not abort per-exec signals, wait for the exec promises, retain failed PIDs, mark the environment closed, block a new exec, remove temporary files or close caller-owned sessions. A command killed solely by cleanup may return ok with exitCode0 when its signal exit code is null. Newly starting executions can race around awaits in shell discovery/cwd checks and register after cleanup; this is not a shutdown barrier.

### 5. Interfaces

```ts
function killProcessTree(pid: number): void {
	if (process.platform === "win32") {
		try {
			spawn("taskkill", ["/F", "/T", "/PID", String(pid)], {
				stdio: "ignore",
				detached: true,
				windowsHide: true,
			});
		} catch {
			// Ignore errors.
		}
		return;
	}

	try {
		process.kill(-pid, "SIGKILL");
	} catch {
		try {
			process.kill(pid, "SIGKILL");
		} catch {
			// Process already dead.
		}
	}
}
```
Source: ice:packages/agent/src/harness/env/nodejs.ts:250-273.
```ts
	async cleanup(): Promise<void> {
		for (const pid of this.activeChildPids) killProcessTree(pid);
		this.activeChildPids.clear();
	}
}
```
Source: ice:packages/agent/src/harness/env/nodejs.ts:671-675.
None: additional CLI flags, slash commands, keybindings, core settings keys, agent RPC commands, or persistent files beyond those explicitly specified. This entry concerns only the named in-process APIs/events.

### 6. Dependencies

`AGENT-037`, `AGENT-052`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

### 7. ice-rust today

rust:crates/ice-tools/src/os_env.rs:357-389; rust:crates/ice-tools/src/os_env.rs:744-750; rust:crates/ice-tools/src/os_env.rs:1098-1100. Rust snapshots PIDs under an async mutex, awaits each kill and finally clears the shared set. A concurrent newly registered PID can therefore be cleared without being in the snapshot. Windows taskkill is awaited indefinitely rather than fire-and-forget. Both leave temporary paths to callers. Preserve best-effort cleanup but add lifecycle fencing/RAII if stronger shutdown guarantees are required; do not claim current code provides them.

### 8. Port notes

Target ice-tools lifecycle ownership. A closed/generation flag and drainable execution handles can provide stronger shutdown guarantees; this is an intentional improvement over both current implementations.

### 9. Size and priority

Estimated Rust implementation: **50-100 lines** (estimate, not measured source). **P1** - Cleanup must not be mistaken for a complete, race-free shutdown or artifact deletion mechanism.

## AGENT-055 - Prompt-template path discovery, symlinks and ordering

### 1. Title and status

Prompt-template path discovery, symlinks and ordering. **Status: partial.**

### 2. ice source

ice:packages/agent/src/harness/prompt-templates.ts:4-16; ice:packages/agent/src/harness/prompt-templates.ts:30-62; ice:packages/agent/src/harness/prompt-templates.ts:95-149; ice:packages/agent/src/harness/prompt-templates.ts:168-196. loadPromptTemplates/loadTemplatesFromDir/resolveKind. Tests: ice:packages/agent/test/harness/prompt-templates.test.ts:13-29 (`loads markdown templates non-recursively from one or more dirs`); ice:packages/agent/test/harness/prompt-templates.test.ts:68-80 (`loads explicit markdown files and symlinked files`).

### 3. What it does

Load direct Markdown prompt templates from explicit files or directories while reporting recoverable read/discovery failures.

### 4. How it works

Accept one path or an ordered path array. Process each sequentially. fileInfo not_found silently skips; other failures append a warning with code file_info_failed and the original input path. Resolve a symlink only to determine target kind using canonicalPath then fileInfo; preserve the original logical path and filename for reading/naming. Missing link targets are silent; other canonical/stat failures report the link path. Directories load only direct file children whose names end with lowercase .md, sorted with JavaScript localeCompare. Explicit files use the same case-sensitive .md gate. Nested directories, non-Markdown files and unsupported kinds are skipped. Hidden .md children are not filtered.

Load every matching file independently and concatenate results/diagnostics in discovery order. No deduplication, first-wins collision policy, recursion, input count/file size limit, caller cancellation or project-trust decision occurs here. Directory list failure returns a list_failed warning; a read/parse failure skips that template and retains its diagnostic. A symlinked directory is listed through its logical path; target identity is not used to deduplicate. File contents are data until a caller explicitly expands a template.

### 5. Interfaces

```ts
export type PromptTemplateDiagnosticCode = "file_info_failed" | "list_failed" | "read_failed" | "parse_failed";

/** Warning produced while loading prompt templates. */
export interface PromptTemplateDiagnostic {
	/** Diagnostic severity. Currently only warnings are emitted. */
	type: "warning";
	/** Stable diagnostic code. */
	code: PromptTemplateDiagnosticCode;
	/** Human-readable diagnostic message. */
	message: string;
	/** Path associated with the diagnostic. */
	path: string;
}
```
Source: ice:packages/agent/src/harness/prompt-templates.ts:4-16.
```ts
export async function loadPromptTemplates(
	env: ExecutionEnv,
	paths: string | string[],
): Promise<{ promptTemplates: PromptTemplate[]; diagnostics: PromptTemplateDiagnostic[] }> {
	const promptTemplates: PromptTemplate[] = [];
	const diagnostics: PromptTemplateDiagnostic[] = [];
	for (const path of Array.isArray(paths) ? paths : [paths]) {
		const infoResult = await env.fileInfo(path);
		if (!infoResult.ok) {
			if (infoResult.error.code !== "not_found") {
				diagnostics.push({
					type: "warning",
					code: "file_info_failed",
					message: infoResult.error.message,
					path,
				});
			}
			continue;
		}
		const info = infoResult.value;
		const kind = await resolveKind(env, info, diagnostics);
		if (kind === "directory") {
			const result = await loadTemplatesFromDir(env, info.path);
			promptTemplates.push(...result.promptTemplates);
			diagnostics.push(...result.diagnostics);
		} else if (kind === "file" && info.name.endsWith(".md")) {
			const result = await loadTemplateFromFile(env, info.path, info.name);
			if (result.promptTemplate) promptTemplates.push(result.promptTemplate);
			diagnostics.push(...result.diagnostics);
		}
	}
	return { promptTemplates, diagnostics };
}
```
Source: ice:packages/agent/src/harness/prompt-templates.ts:30-62.
```ts
async function loadTemplatesFromDir(
	env: ExecutionEnv,
	dir: string,
): Promise<{ promptTemplates: PromptTemplate[]; diagnostics: PromptTemplateDiagnostic[] }> {
	const promptTemplates: PromptTemplate[] = [];
	const diagnostics: PromptTemplateDiagnostic[] = [];
	const entriesResult = await env.listDir(dir);
	if (!entriesResult.ok) {
		diagnostics.push({
			type: "warning",
			code: "list_failed",
			message: entriesResult.error.message,
			path: dir,
		});
		return { promptTemplates, diagnostics };
	}
	const entries = entriesResult.value;

	for (const entry of entries.sort((a, b) => a.name.localeCompare(b.name))) {
		const kind = await resolveKind(env, entry, diagnostics);
		if (kind !== "file" || !entry.name.endsWith(".md")) continue;
		const result = await loadTemplateFromFile(env, entry.path, entry.name);
		if (result.promptTemplate) promptTemplates.push(result.promptTemplate);
		diagnostics.push(...result.diagnostics);
	}
	return { promptTemplates, diagnostics };
}
```
Source: ice:packages/agent/src/harness/prompt-templates.ts:95-121.
```ts
async function resolveKind(
	env: ExecutionEnv,
	info: FileInfo,
	diagnostics: PromptTemplateDiagnostic[],
): Promise<"file" | "directory" | undefined> {
	if (info.kind === "file" || info.kind === "directory") return info.kind;
	const canonicalPath = await env.canonicalPath(info.path);
	if (!canonicalPath.ok) {
		if (canonicalPath.error.code !== "not_found") {
			diagnostics.push({
				type: "warning",
				code: "file_info_failed",
				message: canonicalPath.error.message,
				path: info.path,
			});
		}
		return undefined;
	}
	const target = await env.fileInfo(canonicalPath.value);
	if (!target.ok) {
		if (target.error.code !== "not_found") {
			diagnostics.push({
				type: "warning",
				code: "file_info_failed",
				message: target.error.message,
				path: info.path,
			});
		}
		return undefined;
```
Source: ice:packages/agent/src/harness/prompt-templates.ts:168-196.
None: new CLI flags, keybindings, RPC commands or persistent writes. Read paths are the caller-supplied inputs.

### 6. Dependencies

`AGENT-038`, `AGENT-039`, `AGENT-043`, `AGENT-044`, `AGENT-045`, `AGENT-056`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

Final dependency classification. Shared contract references: `SET-028`, `SET-003`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

rust:crates/ice-harness/src/prompt_templates.rs:21-106; rust:crates/ice-harness/src/prompt_templates.rs:165-207; rust:crates/ice-harness/src/prompt_templates.rs:291-335. Rust implements the same nonrecursive/path-kind workflow, but orders names with Unicode ordinal String::cmp rather than localeCompare and omits diagnostic type:"warning" from its native struct. It adds a resource_diagnostics vector for higher-level collision handling. Preserve those extra diagnostics while projecting the TypeScript warning DTO; locale ordering can affect same-name precedence in downstream consumers.

### 8. Port notes

Target ice-harness resource helpers using the existing ice-tools ExecutionEnv abstraction. Keep discovery, provenance and formatting separate from project trust and extension execution. No new model loop or provider registry is needed. The plugin host resource-discovery API must preserve logical paths and caller-owned provenance; native owned strings/arrays are appropriate.

### 9. Size and priority

Estimated Rust implementation: **60-110 lines** (estimate, not measured source). **P1** - Required for deterministic SDK, event and tool behavior during a TypeScript-to-Rust port.

## AGENT-056 - Frontmatter fences, strict YAML and template description fallback

### 1. Title and status

Frontmatter fences, strict YAML and template description fallback. **Status: partial.**

### 2. ice source

ice:packages/agent/src/harness/prompt-templates.ts:18-22; ice:packages/agent/src/harness/prompt-templates.ts:140-166; ice:packages/agent/src/harness/prompt-templates.ts:198-212; ice:packages/agent/src/harness/skills.ts:312-326. parseFrontmatter/loadTemplateFromFile. Tests: ice:packages/agent/test/harness/prompt-templates.test.ts:50-66 (`attaches source info to diagnostics`); ice:packages/agent/test/harness/prompt-templates.test.ts:13-29 (`loads markdown templates non-recursively from one or more dirs`).

### 3. What it does

Extract template or skill metadata while retaining body text and exact fallback rules.

### 4. How it works

Both TS resource modules duplicate the same frontmatter parser. Normalize CRLF and lone CR to LF. Only startsWith("---") is tested, not a complete opening-fence line. Find the first substring newline+--- starting at offset 3. If absent, return empty metadata and the entire normalized content untrimmed. Otherwise parse the slice from offset 4 to the closing-fence start using yaml.parse; null/undefined becomes{}. Trim the body after the four-character closing marker. A closing marker with trailing text is accepted and that text enters the body. There is no explicit mapping-only check or local metadata/body size limit. A YAML parse exception is converted to parse_failed and the file is skipped. The installed YAML dependency owns its remaining syntax/alias behavior; no parser code was executed here.

For templates, use a string description or empty string. A falsy empty description falls back to the first body line with any non-whitespace content, retaining that line’s leading/trailing whitespace. Take its first 60 UTF-16 code units and append three dots when its original length exceeds 60; this can split a surrogate pair. If there is no such line, return description="". Remove one trailing .md case-insensitively from the filename (the earlier gate already requires lowercase). argument-hint and every unknown metadata key are ignored. No schema validation or template-name restriction is applied.

### 5. Interfaces

```ts
	const { frontmatter, body } = parsed.value;
	const firstLine = body.split("\n").find((line) => line.trim());
	let description = typeof frontmatter.description === "string" ? frontmatter.description : "";
	if (!description && firstLine) {
		description = firstLine.slice(0, 60);
		if (firstLine.length > 60) description += "...";
	}
	return {
		promptTemplate: {
			name: fileName.replace(/\.md$/i, ""),
			description,
			content: body,
		},
		diagnostics,
	};
}
```
Source: ice:packages/agent/src/harness/prompt-templates.ts:151-166.
```ts
	return target.value.kind === "file" || target.value.kind === "directory" ? target.value.kind : undefined;
}

function parseFrontmatter<T extends Record<string, unknown>>(
	content: string,
): Result<{ frontmatter: T; body: string }, Error> {
	try {
		const normalized = content.replace(/\r\n/g, "\n").replace(/\r/g, "\n");
		if (!normalized.startsWith("---")) return { ok: true, value: { frontmatter: {} as T, body: normalized } };
		const endIndex = normalized.indexOf("\n---", 3);
		if (endIndex === -1) return { ok: true, value: { frontmatter: {} as T, body: normalized } };
		const yamlString = normalized.slice(4, endIndex);
		const body = normalized.slice(endIndex + 4).trim();
		return { ok: true, value: { frontmatter: (parse(yamlString) ?? {}) as T, body } };
	} catch (error) {
```
Source: ice:packages/agent/src/harness/prompt-templates.ts:198-212.
Shared skill parser: ```ts
function parseFrontmatter<T extends Record<string, unknown>>(
	content: string,
): Result<{ frontmatter: T; body: string }, Error> {
	try {
		const normalized = content.replace(/\r\n/g, "\n").replace(/\r/g, "\n");
		if (!normalized.startsWith("---")) return { ok: true, value: { frontmatter: {} as T, body: normalized } };
		const endIndex = normalized.indexOf("\n---", 3);
		if (endIndex === -1) return { ok: true, value: { frontmatter: {} as T, body: normalized } };
		const yamlString = normalized.slice(4, endIndex);
		const body = normalized.slice(endIndex + 4).trim();
		return { ok: true, value: { frontmatter: (parse(yamlString) ?? {}) as T, body } };
	} catch (error) {
		return { ok: false, error: toError(error) };
	}
}
```
Source: ice:packages/agent/src/harness/skills.ts:312-326.
None: additional settings, commands or files; metadata is read from each input Markdown document.

### 6. Dependencies

`AGENT-036`, `AGENT-039`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

### 7. ice-rust today

rust:crates/ice-harness/src/frontmatter.rs:22-89; rust:crates/ice-harness/src/prompt_templates.rs:251-286. Rust has the same basic fence extraction but its YAML helper retries malformed top-level unquoted scalar values containing colon-space after quoting them, a legacy acceptance not present in TS. Template description is Option<String>; Some("") prevents fallback, no body yields None, and fallback truncates 60 Unicode scalar values rather than UTF-16 units. Do not label the permissive YAML retry as strict parity. Exact dependency-level YAML edge equivalence remains unverified; parser packages differ and no cross-runtime fixtures were run.

### 8. Port notes

Target ice-harness resource helpers using the existing ice-tools ExecutionEnv abstraction. Keep discovery, provenance and formatting separate from project trust and extension execution. No new model loop or provider registry is needed. The plugin host resource-discovery API must preserve logical paths and caller-owned provenance; native owned strings/arrays are appropriate. yaml_serde already exists in the workspace. A UTF-16-compatible truncation helper is needed only when exact legacy text equality is required. Retain or remove Rust’s legacy YAML salvage through an explicit migration policy, not silently.

### 9. Size and priority

Estimated Rust implementation: **70-130 lines** (estimate, not measured source). **P1** - Required for deterministic SDK, event and tool behavior during a TypeScript-to-Rust port.

## AGENT-057 - Prompt argument lexer and ordered JavaScript substitution

### 1. Title and status

Prompt argument lexer and ordered JavaScript substitution. **Status: partial.**

### 2. ice source

ice:packages/agent/src/harness/prompt-templates.ts:214-263. formatPromptTemplateInvocation/substituteArgs/parseCommandArgs. Tests: ice:packages/agent/test/harness/prompt-templates.test.ts:84-89 (`substitutes command arguments`); ice:packages/agent/test/harness/resource-formatting.test.ts:19-23 (`formats prompt template invocations with positional arguments`). No existing test in these files covers Unicode or JavaScript replacement metasequences; those differences are derived directly from the implementations.

### 3. What it does

Expand positional and aggregate placeholders while preserving quoted arguments and literal non-ASCII template text.

### 4. How it works

Prompt invocation substitutes an already supplied argument array into template text, while the separate argument lexer splits a raw command string. Quoting affects token boundaries, and the four ordered replacement passes can interpret JavaScript replacement metasequences.

formatPromptTemplateInvocation accepts args: string[] (default []) and calls substituteArgs directly; parseCommandArgs is a separate exported helper. The lexer iterates UTF-16 code units through indexed string access, recognizes single/double quote delimiters, closes only the same quote, retains the other quote literally inside it, and separates only on space or tab outside quotes. It does not process backslash escapes; newlines are ordinary characters. Unclosed quotes are accepted. Empty quoted arguments disappear because only nonempty current tokens are pushed; adjacent quoted/unquoted segments concatenate.

Substitution is four full passes in this order: $<digits>, ${@:start[:length]}, $ARGUMENTS, $@. Positional indices are one-based; $0/missing/empty values become empty text. Slice starts are max(0, parseInt(start)-1), optional length is a count (including 0), and JavaScript Array.slice clamps out-of-range values; no negative indices are accepted by the regex. The first two passes use replacement callbacks and therefore insert argument text literally. The final two pass args.join(" ") as a replacement string, so JavaScript replacement metasequences in arguments are interpreted: $$→literal dollar, $&→the matched placeholder, dollar-backtick→prefix, dollar-apostrophe→suffix. Newly inserted placeholders can be consumed by later passes but not recursively by the same pass. Unknown forms remain literal. No output cap or escaping for shells/XML exists; this is prompt text, never command execution.

### 5. Interfaces

```ts
	}
}

/** Parse an argument string using simple shell-style single and double quotes. */
export function parseCommandArgs(argsString: string): string[] {
	const args: string[] = [];
	let current = "";
	let inQuote: string | null = null;

	for (let i = 0; i < argsString.length; i++) {
		const char = argsString[i]!;
		if (inQuote) {
			if (char === inQuote) inQuote = null;
			else current += char;
		} else if (char === '"' || char === "'") {
			inQuote = char;
		} else if (char === " " || char === "\t") {
			if (current) {
				args.push(current);
				current = "";
			}
		} else {
			current += char;
		}
	}
	if (current) args.push(current);
	return args;
}

/** Substitute prompt template placeholders (`$1`, `$@`, `$ARGUMENTS`, `${@:N}`, `${@:N:L}`) with command arguments. */
export function substituteArgs(content: string, args: string[]): string {
	let result = content;
	result = result.replace(/\$(\d+)/g, (_, num: string) => args[parseInt(num, 10) - 1] ?? "");
	result = result.replace(/\$\{@:(\d+)(?::(\d+))?\}/g, (_, startStr: string, lengthStr?: string) => {
		let start = parseInt(startStr, 10) - 1;
		if (start < 0) start = 0;
		if (lengthStr) return args.slice(start, start + parseInt(lengthStr, 10)).join(" ");
		return args.slice(start).join(" ");
	});
	const allArgs = args.join(" ");
	result = result.replace(/\$ARGUMENTS/g, allArgs);
	result = result.replace(/\$@/g, allArgs);
	return result;
}

/** Format a prompt template invocation with positional arguments. */
export function formatPromptTemplateInvocation(template: PromptTemplate, args: string[] = []): string {
	return substituteArgs(template.content, args);
}

```
Source: ice:packages/agent/src/harness/prompt-templates.ts:214-263.
None: new tool/CLI/RPC surface. Public helpers formatPromptTemplateInvocation(template, args:string[] = []), substituteArgs(content, args:string[]) and parseCommandArgs(argsString:string); invocation does not itself parse a raw string.

### 6. Dependencies

`AGENT-056`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

### 7. ice-rust today

rust:crates/ice-harness/src/prompt_templates.rs:338-479. Rust tokenization is largely equivalent, but both replacement scanners walk UTF-8 bytes and cast each ordinary byte to char, corrupting non-ASCII text even without a placeholder. Large numeric parsing falls back to 0 on usize overflow rather than JavaScript’s large-number/out-of-range behavior; start+length can overflow. Its str::replace treats aggregate replacement text literally, unlike JS replacement strings. Preserve literal spans by UTF-8 boundaries or use regex callbacks, then implement the explicitly selected replacement-string compatibility semantics.

### 8. Port notes

Target ice-harness resource helpers using the existing ice-tools ExecutionEnv abstraction. Keep discovery, provenance and formatting separate from project trust and extension execution. No new model loop or provider registry is needed. The plugin host resource-discovery API must preserve logical paths and caller-owned provenance; native owned strings/arrays are appropriate. Use byte offsets only to slice valid UTF-8 spans, never cast arbitrary bytes to char. Existing regex can match ASCII placeholders while preserving Unicode. No provider or external binary is involved.

### 9. Size and priority

Estimated Rust implementation: **100-180 lines** (estimate, not measured source). **P0** - Current Rust expansion corrupts multilingual prompt text and can produce different user instructions.

## AGENT-058 - Source-tagged template mapper and flattened diagnostics

### 1. Title and status

Source-tagged template mapper and flattened diagnostics. **Status: partial.**

### 2. ice source

ice:packages/agent/src/harness/prompt-templates.ts:70-93. loadSourcedPromptTemplates. Tests: ice:packages/agent/test/harness/prompt-templates.test.ts:31-48 (`preserves source info for sourced prompt templates`); ice:packages/agent/test/harness/prompt-templates.test.ts:50-66 (`attaches source info to diagnostics`).

### 3. What it does

Attach caller-defined provenance to loaded templates and allow an application to enrich each template before returning it.

### 4. How it works

loadSourcedPromptTemplates processes inputs {path, source} sequentially through loadPromptTemplates. For each loaded template invoke optional synchronous mapPromptTemplate(template, source), or preserve the base object. Return {promptTemplate:mapped, source} for every result. Diagnostics become {...diagnostic, source}; the source value is preserved exactly, including shared object identity, and overrides any untyped preexisting source property. A mapper exception rejects the operation; no diagnostic conversion or fallback to the unmapped template occurs. There is no deduplication, merging or interpretation of provenance, and mapper output is not validated. Source ownership and higher-level collision policy are caller concerns.

### 5. Interfaces

```ts
export async function loadSourcedPromptTemplates<TSource, TPromptTemplate extends PromptTemplate = PromptTemplate>(
	env: ExecutionEnv,
	inputs: Array<{ path: string; source: TSource }>,
	mapPromptTemplate?: (promptTemplate: PromptTemplate, source: TSource) => TPromptTemplate,
): Promise<{
	promptTemplates: Array<{ promptTemplate: TPromptTemplate; source: TSource }>;
	diagnostics: Array<PromptTemplateDiagnostic & { source: TSource }>;
}> {
	const promptTemplates: Array<{ promptTemplate: TPromptTemplate; source: TSource }> = [];
	const diagnostics: Array<PromptTemplateDiagnostic & { source: TSource }> = [];
	for (const input of inputs) {
		const result = await loadPromptTemplates(env, input.path);
		for (const promptTemplate of result.promptTemplates) {
			promptTemplates.push({
				promptTemplate: mapPromptTemplate
					? mapPromptTemplate(promptTemplate, input.source)
					: (promptTemplate as TPromptTemplate),
				source: input.source,
			});
		}
		for (const diagnostic of result.diagnostics) diagnostics.push({ ...diagnostic, source: input.source });
	}
	return { promptTemplates, diagnostics };
}
```
Source: ice:packages/agent/src/harness/prompt-templates.ts:70-93.
None: persistence or external services. Generic return shape is part of the in-process SDK.

### 6. Dependencies

`AGENT-055`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

### 7. ice-rust today

rust:crates/ice-harness/src/prompt_templates.rs:108-163. Rust preserves cloned generic source values but has no mapper parameter/custom template output type. Its native return names templates/template differ from promptTemplates/promptTemplate and sourced diagnostics nest diagnostic instead of flattening fields. A higher-level consumer can map afterward, but the public function contract is not identical. Do not require JavaScript object identity in Rust; use explicit shared ownership where a caller needs it.

### 8. Port notes

Target ice-harness resource helpers using the existing ice-tools ExecutionEnv abstraction. Keep discovery, provenance and formatting separate from project trust and extension execution. No new model loop or provider registry is needed. The plugin host resource-discovery API must preserve logical paths and caller-owned provenance; native owned strings/arrays are appropriate.

### 9. Size and priority

Estimated Rust implementation: **45-85 lines** (estimate, not measured source). **P2** - Required for deterministic SDK, event and tool behavior during a TypeScript-to-Rust port.

## AGENT-059 - Skill discovery precedence, logical symlinks and recursive scope

### 1. Title and status

Skill discovery precedence, logical symlinks and recursive scope. **Status: partial.**

### 2. ice source

ice:packages/agent/src/harness/skills.ts:49-75; ice:packages/agent/src/harness/skills.ts:103-175; ice:packages/agent/src/harness/skills.ts:328-359. loadSkills/loadSkillsFromDirInternal/resolveKind. Tests: ice:packages/agent/test/harness/skills.test.ts:9-36 (`loads SKILL.md files through the execution environment`); ice:packages/agent/test/harness/skills.test.ts:38-52 (`loads skills through symlinked directories`); ice:packages/agent/test/harness/skills.test.ts:104-115 (`loads direct markdown children only from the root directory`).

### 3. What it does

Discover whole skill directories and direct root Markdown skills without treating nested reference documents as independent skills.

### 4. How it works

Accept a directory string or ordered array; explicit file inputs are ignored by this agent-package helper. Missing roots are silent, other fileInfo failures produce warnings. Resolve symlink kind with canonicalPath then fileInfo while retaining the original logical path/name. For each root create a fresh ignore matcher, then recursively restat each directory, load its ignore files, and list entries. Before sorting, search for exact SKILL.md that resolves to a nonignored file. Load it and return immediately for that directory even when metadata/read/parse failure makes it unavailable; sibling Markdown and child directories are suppressed. An ignored or nonfile SKILL.md does not trigger that return.

Otherwise localeCompare-sort entries, skip names beginning dot and exact node_modules, resolve kinds, check ignore rules (directory paths include a trailing slash), recurse into directories and load lowercase .md files only when includeRootFiles is true. A root loose file’s fallback skill name is the containing directory’s name, not the file stem, so multiple root files can share a name. There is no visited-realpath set, depth bound, symlink-cycle protection, duplicate suppression, file-size limit or trust check. Each recursive branch receives the same mutable matcher object in TS; separate input roots do not share it. Logical symlink paths are returned to the model.

### 5. Interfaces

```ts
export async function loadSkills(
	env: ExecutionEnv,
	dirs: string | string[],
): Promise<{ skills: Skill[]; diagnostics: SkillDiagnostic[] }> {
	const skills: Skill[] = [];
	const diagnostics: SkillDiagnostic[] = [];
	for (const dir of Array.isArray(dirs) ? dirs : [dirs]) {
		const rootInfoResult = await env.fileInfo(dir);
		if (!rootInfoResult.ok) {
			if (rootInfoResult.error.code !== "not_found") {
				diagnostics.push({
					type: "warning",
					code: "file_info_failed",
					message: rootInfoResult.error.message,
					path: dir,
				});
			}
			continue;
		}
		const rootInfo = rootInfoResult.value;
		if ((await resolveKind(env, rootInfo, diagnostics)) !== "directory") continue;
		const result = await loadSkillsFromDirInternal(env, rootInfo.path, true, ignore(), rootInfo.path);
		skills.push(...result.skills);
		diagnostics.push(...result.diagnostics);
	}
	return { skills, diagnostics };
}
```
Source: ice:packages/agent/src/harness/skills.ts:49-75.
```ts
async function loadSkillsFromDirInternal(
	env: ExecutionEnv,
	dir: string,
	includeRootFiles: boolean,
	ignoreMatcher: IgnoreMatcher,
	rootDir: string,
): Promise<{ skills: Skill[]; diagnostics: SkillDiagnostic[] }> {
	const skills: Skill[] = [];
	const diagnostics: SkillDiagnostic[] = [];

	const dirInfoResult = await env.fileInfo(dir);
	if (!dirInfoResult.ok) {
		if (dirInfoResult.error.code !== "not_found") {
			diagnostics.push({
				type: "warning",
				code: "file_info_failed",
				message: dirInfoResult.error.message,
				path: dir,
			});
		}
		return { skills, diagnostics };
	}
	const dirInfo = dirInfoResult.value;
	if ((await resolveKind(env, dirInfo, diagnostics)) !== "directory") return { skills, diagnostics };

	await addIgnoreRules(env, ignoreMatcher, dir, rootDir, diagnostics);

	const entriesResult = await env.listDir(dir);
	if (!entriesResult.ok) {
		diagnostics.push({ type: "warning", code: "list_failed", message: entriesResult.error.message, path: dir });
		return { skills, diagnostics };
	}
	const entries = entriesResult.value;

	for (const entry of entries) {
		if (entry.name !== "SKILL.md") continue;
		const fullPath = entry.path;
		const kind = await resolveKind(env, entry, diagnostics);
		if (kind !== "file") continue;
		const relPath = relativeEnvPath(rootDir, fullPath);
		if (ignoreMatcher.ignores(relPath)) continue;

		const result = await loadSkillFromFile(env, fullPath, dirInfo.name);
		if (result.skill) skills.push(result.skill);
		diagnostics.push(...result.diagnostics);
		return { skills, diagnostics };
	}

	for (const entry of entries.sort((a, b) => a.name.localeCompare(b.name))) {
		if (entry.name.startsWith(".") || entry.name === "node_modules") continue;
		const fullPath = entry.path;
		const kind = await resolveKind(env, entry, diagnostics);
		if (!kind) continue;

		const relPath = relativeEnvPath(rootDir, fullPath);
		const ignorePath = kind === "directory" ? `${relPath}/` : relPath;
		if (ignoreMatcher.ignores(ignorePath)) continue;

		if (kind === "directory") {
			const result = await loadSkillsFromDirInternal(env, fullPath, false, ignoreMatcher, rootDir);
			skills.push(...result.skills);
			diagnostics.push(...result.diagnostics);
			continue;
		}

		if (kind !== "file" || !includeRootFiles || !entry.name.endsWith(".md")) continue;
		const result = await loadSkillFromFile(env, fullPath, dirInfo.name);
		if (result.skill) skills.push(result.skill);
		diagnostics.push(...result.diagnostics);
	}

	return { skills, diagnostics };
}
```
Source: ice:packages/agent/src/harness/skills.ts:103-175.
```ts
async function resolveKind(
	env: ExecutionEnv,
	info: FileInfo,
	diagnostics: SkillDiagnostic[],
): Promise<"file" | "directory" | undefined> {
	if (info.kind === "file" || info.kind === "directory") return info.kind;
	const canonicalPath = await env.canonicalPath(info.path);
	if (!canonicalPath.ok) {
		if (canonicalPath.error.code !== "not_found") {
			diagnostics.push({
				type: "warning",
				code: "file_info_failed",
				message: canonicalPath.error.message,
				path: info.path,
			});
		}
		return undefined;
	}
	const target = await env.fileInfo(canonicalPath.value);
	if (!target.ok) {
		if (target.error.code !== "not_found") {
			diagnostics.push({
				type: "warning",
				code: "file_info_failed",
				message: target.error.message,
				path: info.path,
			});
		}
		return undefined;
	}
	return target.value.kind === "file" || target.value.kind === "directory" ? target.value.kind : undefined;
}
```
Source: ice:packages/agent/src/harness/skills.ts:328-359.
None: new model tool or command; caller-supplied skill roots are the only inputs here.

### 6. Dependencies

`AGENT-038`, `AGENT-039`, `AGENT-043`, `AGENT-044`, `AGENT-045`, `AGENT-060`, `AGENT-061`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

Final dependency classification. Shared contract references: `SET-030`, `SET-032`, `SET-003`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

rust:crates/ice-harness/src/skills.rs:164-236; rust:crates/ice-harness/src/skills.rs:296-423; rust:crates/ice-harness/src/skills.rs:568-612. Rust additionally accepts explicit .md file inputs, useful for plugin resource discovery, and clones its matcher per directory rather than sharing mutations across already-visited sibling branches. It sorts ordinally, not localeCompare. Do not remove explicit-file support merely to reproduce this lower-level TS helper; the coding-agent wrapper has a broader separate contract. Both need a deliberate cycle/limit policy if exposed to untrusted resource trees.

### 8. Port notes

Target ice-harness resource helpers using the existing ice-tools ExecutionEnv abstraction. Keep discovery, provenance and formatting separate from project trust and extension execution. No new model loop or provider registry is needed. The plugin host resource-discovery API must preserve logical paths and caller-owned provenance; native owned strings/arrays are appropriate.

### 9. Size and priority

Estimated Rust implementation: **100-175 lines** (estimate, not measured source). **P1** - Required for deterministic SDK, event and tool behavior during a TypeScript-to-Rust port.

## AGENT-060 - Skill ignore-file prefixing and full matcher dependency

### 1. Title and status

Skill ignore-file prefixing and full matcher dependency. **Status: partial.**

### 2. ice source

ice:packages/agent/src/harness/skills.ts:1-9; ice:packages/agent/src/harness/skills.ts:128-169; ice:packages/agent/src/harness/skills.ts:177-241; ice:packages/agent/src/harness/skills.ts:368-375. addIgnoreRules/prefixIgnorePattern/relativeEnvPath. Tests: agent-package skills.test.ts has no ignore-pattern assertions; reading the dependency was blocked by connector safety rules, so advanced pattern semantics remain unverified rather than inferred.

### 3. What it does

Apply .gitignore, .ignore and .fdignore patterns before loading skill directories and documents.

### 4. How it works

For each visited directory read the three names in that order using env.joinPath and fileInfo. Only a direct file kind is accepted; an ignore-file symlink is skipped rather than resolved. Missing files are silent, join/stat/read failures become warning diagnostics with their specific path/code. Read each full file, split on CRLF-or-LF, prefix nonempty rules with the logical directory’s root-relative prefix and add them to the same ignore-package matcher.

Prefixing uses trim only to detect empty/comment lines, retaining the original line’s other whitespace. A line whose trimmed form begins # is dropped. Leading ! marks negation and is stripped/re-added after prefixing; leading backslash-! loses its backslash; one leading slash is stripped before prefixing. The latter can remove root anchoring from a basename pattern, and a root escaped exclamation becomes a leading ! again when passed to the matcher. Relative paths normalize backslashes to slashes and strip trailing slashes; outside-root paths fall back to their leading-slash-stripped form rather than being rejected. Directory ignore checks use a trailing slash.

The actual pattern semantics are delegated to the installed JavaScript ignore package, not the handwritten subset in Rust. The audit verified the call sites and exact prefix transform but the connector blocked reading node_modules/ignore; dependency-level matching edge cases are explicitly unverified here. No ignore package was executed or installed. A Rust port should use a complete gitignore engine, then retain this prefix/ordering layer and document any intentional corrections to its anchoring/escape quirks. This limitation must not be presented as fully verified pattern parity.

### 5. Interfaces

```ts
async function addIgnoreRules(
	env: ExecutionEnv,
	ig: IgnoreMatcher,
	dir: string,
	rootDir: string,
	diagnostics: SkillDiagnostic[],
): Promise<void> {
	const relativeDir = relativeEnvPath(rootDir, dir);
	const prefix = relativeDir ? `${relativeDir}/` : "";

	for (const filename of IGNORE_FILE_NAMES) {
		const ignorePathResult = await env.joinPath([dir, filename]);
		if (!ignorePathResult.ok) {
			diagnostics.push({
				type: "warning",
				code: "file_info_failed",
				message: ignorePathResult.error.message,
				path: dir,
			});
			continue;
		}
		const ignorePath = ignorePathResult.value;
		const info = await env.fileInfo(ignorePath);
		if (!info.ok) {
			if (info.error.code !== "not_found") {
				diagnostics.push({
					type: "warning",
					code: "file_info_failed",
					message: info.error.message,
					path: ignorePath,
				});
			}
			continue;
		}
		if (info.value.kind !== "file") continue;
		const content = await env.readTextFile(ignorePath);
		if (!content.ok) {
			diagnostics.push({ type: "warning", code: "read_failed", message: content.error.message, path: ignorePath });
			continue;
		}
		const patterns = content.value
			.split(/\r?\n/)
			.map((line) => prefixIgnorePattern(line, prefix))
			.filter((line): line is string => Boolean(line));
		if (patterns.length > 0) ig.add(patterns);
	}
}

function prefixIgnorePattern(line: string, prefix: string): string | null {
	const trimmed = line.trim();
	if (!trimmed) return null;
	if (trimmed.startsWith("#") && !trimmed.startsWith("\\#")) return null;

	let pattern = line;
	let negated = false;
	if (pattern.startsWith("!")) {
		negated = true;
		pattern = pattern.slice(1);
	} else if (pattern.startsWith("\\!")) {
		pattern = pattern.slice(1);
	}
	if (pattern.startsWith("/")) pattern = pattern.slice(1);
	const prefixed = prefix ? `${prefix}${pattern}` : pattern;
	return negated ? `!${prefixed}` : prefixed;
}
```
Source: ice:packages/agent/src/harness/skills.ts:177-241.
```ts
function relativeEnvPath(root: string, path: string): string {
	const normalizedRoot = root.replace(/\\/g, "/").replace(/\/+$/, "");
	const normalizedPath = path.replace(/\\/g, "/").replace(/\/+$/, "");
	if (normalizedPath === normalizedRoot) return "";
	return normalizedPath.startsWith(`${normalizedRoot}/`)
		? normalizedPath.slice(normalizedRoot.length + 1)
		: normalizedPath.replace(/^\/+/, "");
}
```
Source: ice:packages/agent/src/harness/skills.ts:368-375.
Ignore file names: .gitignore, .ignore, .fdignore. No writes, CLI flags or settings are introduced.

### 6. Dependencies

`AGENT-038`, `AGENT-039`, `AGENT-043`, `AGENT-044`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

### 7. ice-rust today

rust:crates/ice-harness/src/skills.rs:332-333; rust:crates/ice-harness/src/skills.rs:614-681; rust:crates/ice-harness/src/skills.rs:775-905. Rust explicitly implements only literal text and star splitting, not a full gitignore engine: question marks/classes/escaping are not parsed, consecutive stars have no directory semantics, a trailing star accepts arbitrary remaining slash segments, dir_only does not require an actual directory marker, and parent exclusion/negation rules are not enforced as a complete engine. Its line split also discards text after a lone CR. These are concrete source differences; exact results of every JS pattern cannot be certified without the blocked dependency/fixtures.

### 8. Port notes

Target ice-harness resource helpers using the existing ice-tools ExecutionEnv abstraction. Keep discovery, provenance and formatting separate from project trust and extension execution. No new model loop or provider registry is needed. The plugin host resource-discovery API must preserve logical paths and caller-owned provenance; native owned strings/arrays are appropriate. Candidate dependency: the Rust ignore crate/gitignore matcher; its version and exact integration have not been verified locally. Do not add a partial custom glob parser or execute resource code to interpret ignores.

### 9. Size and priority

Estimated Rust implementation: **130-230 lines** (estimate, not measured source). **P1** - Discovery must not silently expose or omit skill files because the two pattern engines differ.

## AGENT-061 - Skill metadata limits and preserved invocation/list formatting

### 1. Title and status

Skill metadata limits and preserved invocation/list formatting. **Status: partial.**

### 2. ice source

ice:packages/agent/src/harness/skills.ts:243-310; ice:packages/agent/src/harness/skills.ts:37-41; ice:packages/agent/src/harness/skills.ts:361-366; ice:packages/agent/src/harness/system-prompt.ts:1-35. loadSkillFromFile/validateName/validateDescription/formatSkillInvocation/formatSkillsForSystemPrompt. Tests: ice:packages/agent/test/harness/skills.test.ts:9-36 (`loads SKILL.md files through the execution environment`); ice:packages/agent/test/harness/skills.test.ts:82-102 (`attaches source info to diagnostics`); ice:packages/agent/test/harness/resource-formatting.test.ts:6-17 (`formats skill invocations with additional instructions`); ice:packages/agent/test/harness/system-prompt.test.ts:27-46 (`formats visible skills in order and skips model-disabled skills`); ice:packages/agent/test/harness/system-prompt.test.ts:52-65 (`escapes XML in all model-visible skill fields`).

### 3. What it does

Validate skill metadata with warnings, omit skills lacking descriptions, and retain the already-ported model-visible formatting contracts.

### 4. How it works

Read/parse the skill file. Description must be a string and nonblank after trim; otherwise append invalid_metadata "description is required" and omit the skill. A description longer than 1024 UTF-16 code units gets a warning but still loads. Name is a string frontmatter name when truthy, else the containing directory name. Warn on name/parent mismatch, length>64 UTF-16 units, characters outside lowercase ASCII letters/digits/hyphens, leading/trailing hyphen and consecutive hyphens. Invalid names still load when description is valid; unknown metadata is ignored. Return body, logical filePath and disableModelInvocation equal to the exact boolean true comparison, so omitted/false/nonboolean values become false. Warnings remain ordered: description first, then each name rule.

Preserved baseline: formatSkillInvocation emits the exact raw <skill name/location> block below, with body and attributes unescaped; additional instructions append only when truthy/nonempty. Its dirname helper supports either slash style and preserves a Windows drive root. This is untrusted prompt data, not authority. formatSkillsForSystemPrompt filters disabled skills, preserves order, returns empty when none, and emits only name/description/location, escaping &,<,>, double/single quotes. It does not insert skill bodies. Rust already matches these formatting values for valid inputs; they are documented here for completeness, not claimed missing.

### 5. Interfaces

```ts
async function loadSkillFromFile(
	env: ExecutionEnv,
	filePath: string,
	parentDirName: string,
): Promise<{ skill: Skill | null; diagnostics: SkillDiagnostic[] }> {
	const diagnostics: SkillDiagnostic[] = [];
	const rawContent = await env.readTextFile(filePath);
	if (!rawContent.ok) {
		diagnostics.push({ type: "warning", code: "read_failed", message: rawContent.error.message, path: filePath });
		return { skill: null, diagnostics };
	}

	const parsed = parseFrontmatter<SkillFrontmatter>(rawContent.value);
	if (!parsed.ok) {
		diagnostics.push({ type: "warning", code: "parse_failed", message: parsed.error.message, path: filePath });
		return { skill: null, diagnostics };
	}

	const { frontmatter, body } = parsed.value;
	const description = typeof frontmatter.description === "string" ? frontmatter.description : undefined;

	for (const error of validateDescription(description)) {
		diagnostics.push({ type: "warning", code: "invalid_metadata", message: error, path: filePath });
	}

	const frontmatterName = typeof frontmatter.name === "string" ? frontmatter.name : undefined;
	const name = frontmatterName || parentDirName;
	for (const error of validateName(name, parentDirName)) {
		diagnostics.push({ type: "warning", code: "invalid_metadata", message: error, path: filePath });
	}

	if (!description || description.trim() === "") {
		return { skill: null, diagnostics };
	}

	return {
		skill: {
			name,
			description,
			content: body,
			filePath,
			disableModelInvocation: frontmatter["disable-model-invocation"] === true,
		},
		diagnostics,
	};
}

function validateName(name: string, parentDirName: string): string[] {
	const errors: string[] = [];
	if (name !== parentDirName) errors.push(`name "${name}" does not match parent directory "${parentDirName}"`);
	if (name.length > MAX_NAME_LENGTH) errors.push(`name exceeds ${MAX_NAME_LENGTH} characters (${name.length})`);
	if (!/^[a-z0-9-]+$/.test(name)) {
		errors.push("name contains invalid characters (must be lowercase a-z, 0-9, hyphens only)");
	}
	if (name.startsWith("-") || name.endsWith("-")) errors.push("name must not start or end with a hyphen");
	if (name.includes("--")) errors.push("name must not contain consecutive hyphens");
	return errors;
}

function validateDescription(description: string | undefined): string[] {
	const errors: string[] = [];
	if (!description || description.trim() === "") {
		errors.push("description is required");
	} else if (description.length > MAX_DESCRIPTION_LENGTH) {
		errors.push(`description exceeds ${MAX_DESCRIPTION_LENGTH} characters (${description.length})`);
	}
	return errors;
}
```
Source: ice:packages/agent/src/harness/skills.ts:243-310.
```ts
/** Format a skill invocation prompt, optionally appending additional user instructions. */
export function formatSkillInvocation(skill: Skill, additionalInstructions?: string): string {
	const skillBlock = `<skill name="${skill.name}" location="${skill.filePath}">\nReferences are relative to ${dirnameEnvPath(skill.filePath)}.\n\n${skill.content}\n</skill>`;
	return additionalInstructions ? `${skillBlock}\n\n${additionalInstructions}` : skillBlock;
}
```
Source: ice:packages/agent/src/harness/skills.ts:37-41.
```ts
function dirnameEnvPath(path: string): string {
	const normalized = path.replace(/[\\/]+$/, "");
	const separatorIndex = Math.max(normalized.lastIndexOf("/"), normalized.lastIndexOf("\\"));
	if (separatorIndex === 2 && normalized[1] === ":") return normalized.slice(0, 3);
	return separatorIndex <= 0 ? "/" : normalized.slice(0, separatorIndex);
}
```
Source: ice:packages/agent/src/harness/skills.ts:361-366.
```ts
import type { Skill } from "./types.ts";

export function formatSkillsForSystemPrompt(skills: Skill[]): string {
	const visibleSkills = skills.filter((skill) => !skill.disableModelInvocation);
	if (visibleSkills.length === 0) return "";

	const lines = [
		"The following skills provide specialized instructions for specific tasks.",
		"Read the full skill file when the task matches its description.",
		"When a skill file references a relative path, resolve it against the skill directory (parent of SKILL.md / dirname of the path) and use that absolute path in tool commands.",
		"",
		"<available_skills>",
	];

	for (const skill of visibleSkills) {
		lines.push("  <skill>");
		lines.push(`    <name>${escapeXml(skill.name)}</name>`);
		lines.push(`    <description>${escapeXml(skill.description)}</description>`);
		lines.push(`    <location>${escapeXml(skill.filePath)}</location>`);
		lines.push("  </skill>");
	}

	lines.push("</available_skills>");
	return lines.join("\n");
}

function escapeXml(value: string): string {
	return value
		.replace(/&/g, "&amp;")
		.replace(/</g, "&lt;")
		.replace(/>/g, "&gt;")
		.replace(/"/g, "&quot;")
		.replace(/'/g, "&apos;");
}

```
Source: ice:packages/agent/src/harness/system-prompt.ts:1-35.
None: new CLI/RPC/tool names. Metadata keys are name, description and disable-model-invocation.

### 6. Dependencies

`AGENT-039`, `AGENT-056`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

Final dependency classification. Shared contract references: `SET-031`, `TUI-030`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

rust:crates/ice-harness/src/skills.rs:435-566; rust:crates/ice-harness/src/skills.rs:85-158; rust:crates/ice-harness/src/skills.rs:733-755. Rust uses Unicode scalar counts rather than UTF-16 lengths, retains an explicit empty name instead of TS’s truthy fallback, and returns Option<bool> for disable_model_invocation rather than a concrete false. It has equivalent raw invocation and escaped listing formatters; those should be retained unchanged for valid inputs. Metadata normalization and warning DTO differences remain the gap.

### 8. Port notes

Target ice-harness resource helpers using the existing ice-tools ExecutionEnv abstraction. Keep discovery, provenance and formatting separate from project trust and extension execution. No new model loop or provider registry is needed. The plugin host resource-discovery API must preserve logical paths and caller-owned provenance; native owned strings/arrays are appropriate.

### 9. Size and priority

Estimated Rust implementation: **60-115 lines** (estimate, not measured source). **P1** - Required for deterministic SDK, event and tool behavior during a TypeScript-to-Rust port.

## AGENT-062 - Source-tagged skill enrichment and diagnostics

### 1. Title and status

Source-tagged skill enrichment and diagnostics. **Status: partial.**

### 2. ice source

ice:packages/agent/src/harness/skills.ts:83-101. loadSourcedSkills. Tests: ice:packages/agent/test/harness/skills.test.ts:54-80 (`preserves source info for sourced skills`); ice:packages/agent/test/harness/skills.test.ts:82-102 (`attaches source info to diagnostics`).

Named export: `SkillDiagnosticCode` (`ice:packages/agent/src/harness/skills.ts:11`).

`ice:packages/agent/src/harness/skills.ts:11-28` defines the named diagnostic discriminant and fields.

### 3. What it does

Carry arbitrary provenance through skill loading and allow application-specific skill fields to be added synchronously.

### 4. How it works

loadSourcedSkills processes each {path, source} input in order through loadSkills. Optionally call mapSkill(skill, source), otherwise preserve the base skill; pair each result with the same source value. Flatten diagnostics as {...diagnostic, source}. No provenance normalization, deduplication, collision resolution or trust decision occurs. Mapper exceptions propagate and stop the loader; output is not schema-checked. The generic mapped type extends Skill, allowing the coding-agent to attach application metadata without changing the low-level loader. JavaScript object identity is retained; the Rust equivalent may use explicit Arc ownership rather than aliasing mutable values accidentally.

### 5. Interfaces

```ts
export async function loadSourcedSkills<TSource, TSkill extends Skill = Skill>(
	env: ExecutionEnv,
	inputs: Array<{ path: string; source: TSource }>,
	mapSkill?: (skill: Skill, source: TSource) => TSkill,
): Promise<{
	skills: Array<{ skill: TSkill; source: TSource }>;
	diagnostics: Array<SkillDiagnostic & { source: TSource }>;
}> {
	const skills: Array<{ skill: TSkill; source: TSource }> = [];
	const diagnostics: Array<SkillDiagnostic & { source: TSource }> = [];
	for (const input of inputs) {
		const result = await loadSkills(env, input.path);
		for (const skill of result.skills) {
			skills.push({ skill: mapSkill ? mapSkill(skill, input.source) : (skill as TSkill), source: input.source });
		}
		for (const diagnostic of result.diagnostics) diagnostics.push({ ...diagnostic, source: input.source });
	}
	return { skills, diagnostics };
}
```
Source: ice:packages/agent/src/harness/skills.ts:83-101.
None: new files or runtime side effects beyond the underlying reads.

The diagnostic code is `SkillDiagnosticCode = "file_info_failed" | "list_failed" | "read_failed" | "parse_failed" | "invalid_metadata"`. `SkillDiagnostic` has `type: "warning"`, `code: SkillDiagnosticCode`, `message: string`, and `path: string`; sourced enrichment adds the caller's `source` without changing these fields.

### 6. Dependencies

`AGENT-059`, `AGENT-061`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

### 7. ice-rust today

rust:crates/ice-harness/src/skills.rs:238-294. Rust clones the generic source and provides no mapper/custom output type. Diagnostics are nested under diagnostic instead of the flat TS shape. Add a map-capable variant or a caller adapter, and preserve native resource_diagnostics separately rather than discarding them.

### 8. Port notes

Target ice-harness resource helpers using the existing ice-tools ExecutionEnv abstraction. Keep discovery, provenance and formatting separate from project trust and extension execution. No new model loop or provider registry is needed. The plugin host resource-discovery API must preserve logical paths and caller-owned provenance; native owned strings/arrays are appropriate.

### 9. Size and priority

Estimated Rust implementation: **40-80 lines** (estimate, not measured source). **P2** - Required for deterministic SDK, event and tool behavior during a TypeScript-to-Rust port.

## AGENT-063 - Bash-execution custom message shape and text projection

### 1. Title and status

Bash-execution custom message shape and text projection. **Status: partial.**

### 2. ice source

ice:packages/agent/src/harness/messages.ts:19-29; ice:packages/agent/src/harness/messages.ts:54-79; ice:packages/agent/src/harness/messages.ts:128-136. BashExecutionMessage/bashExecutionToText. Existing agent-package tests do not directly assert this renderer; its exact implementation and Rust unit coverage were read. Related shell-output/session tests belong to their respective areas.

### 3. What it does

Represent a user-run shell command in history and render it as a bounded-by-caller text message for provider context.

### 4. How it works

BashExecutionMessage is a flat custom role with command/output/exitCode/cancelled/truncated/timestamp and optional fullOutputPath/excludeFromContext. This module exports no bash-message constructor; callers build the object. bashExecutionToText writes Ran `command` plus newline, then fenced output when truthy or (no output). Cancellation notice wins over a nonzero exit-code notice. Add the truncation suffix only when truncated and fullOutputPath are both truthy. Command/output/path text is interpolated without escaping backticks, fences, control characters or secrets, and no size cap applies here. The converter separately drops a truthy excludeFromContext message; rendering alone does not. There is no process execution or log creation in this helper.

### 5. Interfaces

```ts
export interface BashExecutionMessage {
	role: "bashExecution";
	command: string;
	output: string;
	exitCode: number | undefined;
	cancelled: boolean;
	truncated: boolean;
	fullOutputPath?: string;
	timestamp: number;
	excludeFromContext?: boolean;
}
```
Source: ice:packages/agent/src/harness/messages.ts:19-29.
```ts
export function bashExecutionToText(msg: BashExecutionMessage): string {
	let text = `Ran \`${msg.command}\`\n`;
	if (msg.output) {
		text += `\`\`\`\n${msg.output}\n\`\`\``;
	} else {
		text += "(no output)";
	}
	if (msg.cancelled) {
		text += "\n\n(command cancelled)";
	} else if (msg.exitCode !== null && msg.exitCode !== undefined && msg.exitCode !== 0) {
		text += `\n\nCommand exited with code ${msg.exitCode}`;
	}
	if (msg.truncated && msg.fullOutputPath) {
		text += `\n\n[Output truncated. Full output: ${msg.fullOutputPath}]`;
	}
	return text;
}
```
Source: ice:packages/agent/src/harness/messages.ts:63-79.
None: new model tool, CLI flag, settings key or file; the message is supplied to the session/provider layer.

### 6. Dependencies

`AGENT-001`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

Final dependency classification. Shared contract references: `SESSION-029`, `SESSION-038`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

rust:crates/ice-harness/src/messages.rs:53-70; rust:crates/ice-harness/src/messages.rs:113-142; rust:crates/ice-harness/src/messages.rs:264-284; rust:crates/ice-agent/src/message.rs:25-32; rust:crates/ice-agent/src/message.rs:148-155. Rust adds its own constructor and stores role-specific fields inside data. Direct serde serialization adds kind:"custom" and nested data instead of the TS flat object; this is an SDK serialization difference even if another transport later adapts it. Rust includes the truncation suffix for Some("") whereas TS suppresses it, and exit codes are i32. Keep the same human-readable rendering while adapting the exact flat message shape at compatibility boundaries.

### 8. Port notes

Target ice-harness messages plus explicit session/RPC codec adapters. Preserve internal typed data ownership; do not infer direct wire compatibility from the misleading “mirrors TS” comment. No shell implementation is required by this entry.

### 9. Size and priority

Estimated Rust implementation: **45-90 lines** (estimate, not measured source). **P1** - Required for deterministic SDK, event and tool behavior during a TypeScript-to-Rust port.

## AGENT-064 - Custom and summary constructors, timestamps and envelopes

### 1. Title and status

Custom and summary constructors, timestamps and envelopes. **Status: partial.**

### 2. ice source

ice:packages/agent/src/harness/messages.ts:4-17; ice:packages/agent/src/harness/messages.ts:31-61; ice:packages/agent/src/harness/messages.ts:81-122. createBranchSummaryMessage/createCompactionSummaryMessage/createCustomMessage and role interfaces. Existing harness session/compaction code consumes these; no dedicated constructor edge-case test exists in the inspected agent tests.

### 3. What it does

Construct application/branch/compaction messages with stable metadata and exact provider summary envelopes.

### 4. How it works

createBranchSummaryMessage stores summary/fromId; createCompactionSummaryMessage stores summary/tokensBefore; createCustomMessage stores customType/content/display/details. All are flat role-tagged objects. Numeric timestamps pass unchanged, including untyped nonfinite values; string timestamps use new Date(value).getTime(), yielding NaN when invalid. No timestamp validation, content copy, hash, redaction, message-length cap or persistence happens here. Custom content remains a string or text/image array until conversion, and display controls UI intent only, not whether provider context includes it. Undefined details is still an object property in memory but disappears under ordinary JSON serialization.

Compaction and branch prefixes are different exact strings; compaction suffix includes a leading newline while branch suffix does not. Summary text is interpolated verbatim between them, without escaping a nested closing summary tag. fromId/tokensBefore are metadata and do not enter the rendered provider text. The complete constructors and envelopes below are the contract; the session writer owns any durable record format.

### 5. Interfaces

```ts
export const COMPACTION_SUMMARY_PREFIX = `The conversation history before this point was compacted into the following summary:

<summary>
`;

export const COMPACTION_SUMMARY_SUFFIX = `
</summary>`;

export const BRANCH_SUMMARY_PREFIX = `The following is a summary of a branch that this conversation came back from:

<summary>
`;

export const BRANCH_SUMMARY_SUFFIX = `</summary>`;
```
Source: ice:packages/agent/src/harness/messages.ts:4-17.
```ts
export interface CustomMessage<T = unknown> {
	role: "custom";
	customType: string;
	content: string | (TextContent | ImageContent)[];
	display: boolean;
	details?: T;
	timestamp: number;
}

export interface BranchSummaryMessage {
	role: "branchSummary";
	summary: string;
	fromId: string;
	timestamp: number;
}

export interface CompactionSummaryMessage {
	role: "compactionSummary";
	summary: string;
	tokensBefore: number;
	timestamp: number;
}
```
Source: ice:packages/agent/src/harness/messages.ts:31-52.
```ts
export function createBranchSummaryMessage(
	summary: string,
	fromId: string,
	timestamp: string | number,
): BranchSummaryMessage {
	return {
		role: "branchSummary",
		summary,
		fromId,
		timestamp: typeof timestamp === "number" ? timestamp : new Date(timestamp).getTime(),
	};
}

export function createCompactionSummaryMessage(
	summary: string,
	tokensBefore: number,
	timestamp: string | number,
): CompactionSummaryMessage {
	return {
		role: "compactionSummary",
		summary,
		tokensBefore,
		timestamp: typeof timestamp === "number" ? timestamp : new Date(timestamp).getTime(),
	};
}

export function createCustomMessage(
	customType: string,
	content: string | (TextContent | ImageContent)[],
	display: boolean,
	details: unknown | undefined,
	timestamp: string | number,
): CustomMessage {
	return {
		role: "custom",
		customType,
		content,
		display,
		details,
		timestamp: typeof timestamp === "number" ? timestamp : new Date(timestamp).getTime(),
	};
}
```
Source: ice:packages/agent/src/harness/messages.ts:81-122.
None: additional CLI/RPC/tool surfaces or persistent files.

### 6. Dependencies

`AGENT-001`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

### 7. ice-rust today

rust:crates/ice-harness/src/messages.rs:28-106; rust:crates/ice-harness/src/messages.rs:144-212; rust:crates/ice-agent/src/message.rs:25-32; rust:crates/ice-agent/src/message.rs:148-155. Rust preserves exact envelope strings, but constructors accept i64 timestamps/tokens, normalize custom string content to blocks immediately and put metadata in a nested data object under kind:"custom". It cannot represent NaN timestamps directly. A flat TS JSON decoder/encoder and explicit invalid-date policy are required for migration; do not copy JavaScript NaN into otherwise valid Rust session data silently.

### 8. Port notes

Target ice-harness message constructors and session import/export adapters. serde_json cannot serialize nonfinite JSON numbers; decide rejection or explicit migration diagnostics. No additional plugin execution API is needed.

### 9. Size and priority

Estimated Rust implementation: **75-135 lines** (estimate, not measured source). **P1** - Required for deterministic SDK, event and tool behavior during a TypeScript-to-Rust port.

## AGENT-065 - Custom-role provider projection and Rust-only orphan repair

### 1. Title and status

Custom-role provider projection and Rust-only orphan repair. **Status: different-by-design.**

### 2. ice source

ice:packages/agent/src/harness/messages.ts:124-168; ice:packages/agent/src/agent.ts:33-37. convertToLlm/defaultConvertToLlm. Tests: ice:packages/agent/test/agent-loop.test.ts:166-219 (`should handle custom message types via convertToLlm`) covers the pluggable converter seam; the specific custom-role mapping is explicit source. Rust recovery tests in crates/ice-harness/src/messages.rs were read, not executed.

### 3. What it does

Turn supported custom messages into ordinary user context while preserving a clear boundary between formatting and recovery.

### 4. How it works

TypeScript convertToLlm maps in original order: drop bashExecution when excludeFromContext is truthy, otherwise render it into one text block; convert custom string to one text block or reuse its content array, regardless of display; wrap branch/compaction summaries using the exact envelopes; pass user/assistant/toolResult objects through unchanged; drop every other role. Timestamps are retained. It does not reorder messages, repair missing tool results, validate required custom fields, clone base messages or invoke tools. Malformed untyped content can throw/interpolate undefined; this is not a sanitizing parser. The low-level Agent default converter is different: it drops all custom roles.

The Rust helper adds a distinct recovery stage before projection: collect tool-call IDs per assistant, consume subsequent messages until the next assistant, emit matching tool results in their encountered order, omit unmatched/duplicate/orphan results, buffer non-tool interleaved messages, synthesize missing results with the exact recovery text and assistant timestamp, then append buffered messages. It changes provider context only, never durable history or tool execution. That recovery is not implemented by this TS helper; other TS provider/session layers may perform related repair and must be assessed separately. Preserve the useful Rust safety behavior unless an exact-context compatibility mode is explicitly required.

### 5. Interfaces

```ts
export function convertToLlm(messages: AgentMessage[]): Message[] {
	return messages
		.map((m): Message | undefined => {
			switch (m.role) {
				case "bashExecution":
					if (m.excludeFromContext) {
						return undefined;
					}
					return {
						role: "user",
						content: [{ type: "text", text: bashExecutionToText(m) }],
						timestamp: m.timestamp,
					};
				case "custom": {
					const content = typeof m.content === "string" ? [{ type: "text" as const, text: m.content }] : m.content;
					return {
						role: "user",
						content,
						timestamp: m.timestamp,
					};
				}
				case "branchSummary":
					return {
						role: "user",
						content: [{ type: "text" as const, text: BRANCH_SUMMARY_PREFIX + m.summary + BRANCH_SUMMARY_SUFFIX }],
						timestamp: m.timestamp,
					};
				case "compactionSummary":
					return {
						role: "user",
						content: [
							{ type: "text" as const, text: COMPACTION_SUMMARY_PREFIX + m.summary + COMPACTION_SUMMARY_SUFFIX },
						],
						timestamp: m.timestamp,
					};
				case "user":
				case "assistant":
				case "toolResult":
					return m;
				default:
					return undefined;
			}
		})
		.filter((m): m is Message => m !== undefined);
}
```
Source: ice:packages/agent/src/harness/messages.ts:124-168.
Rust-only recovery result text: "Tool call was interrupted before its result was persisted. The tool was not re-executed during session recovery." No new persisted message is appended by the converter itself.

### 6. Dependencies

`AGENT-001`, `AGENT-063`, `AGENT-064`

External services/binaries: none for this layer. The supplied provider stream or execution environment owns external I/O. No source module, test, network request or agent was executed during verification.

Final dependency classification. Shared contract references: `AI-014`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

rust:crates/ice-harness/src/messages.rs:302-438. Rust projects supported roles but first repairs/orients tool results as described. Bash/summary projection uses UserContent::Text while TS emits an explicit one-text-block array; generic custom content remains blocks. Invalid structured data for bash/summary is silently skipped after serde parse failure, unlike TS’s unchecked property access. Unknown roles are dropped in both. Native recovery emits tracing warnings/report IDs; TS helper emits none.

### 8. Port notes

Target ice-harness conversion and session import compatibility. Retain orphan repair and its no-reexecution guarantee. Provide explicit formatting/repair stages so callers can compare provider contexts without silently changing durable messages.

### 9. Size and priority

Estimated Rust implementation: **70-135 lines** (estimate, not measured source). **P1** - Recovery and formatting must remain separate so the port does not remove native safety behavior or silently change transcript order.

