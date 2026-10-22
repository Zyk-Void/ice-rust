# RPC, remote clients and framed protocol parity

Source-only audit of the frozen TypeScript and Rust checkouts. Coding-agent JSONL RPC, the newer framed-CBOR protocol, and the native TCP JSON-RPC proxy are distinct contracts. All 63 assigned TS source/documentation files and 51 related TS test/helper files have source-body review; 50 test/helper files are fully visible and one excludes a connector-redacted line. Native counterparts, actual call paths, inline tests and the remote integration-test source were inspected without execution. The coordinator reviewed all 36 entries and independently checked selected material boundaries. Exact read ranges, identities and limitations are in rpc-source-review.json and qa-rpc-coordinator-review.json.

## RPC-001 - Bounded binary framing and terminal decoder state

### 1. Title and status

Bounded binary framing and terminal decoder state. **Status: partial.**

### 2. ice source

ice:packages/protocol/src/framing.ts:1-166; ice:packages/protocol/test/framing.test.ts:14-117; rust:crates/ice-cli/src/remote/framing.rs:1-214.

Exported symbols: `DEFAULT_MAX_FRAME_LENGTH`, `FrameDecoderOptions`, `FrameError` (ice:packages/protocol/src/framing.ts:6; ice:packages/protocol/src/framing.ts:8; ice:packages/protocol/src/framing.ts:12).

### 3. What it does

Decode split or coalesced binary messages without trusting announced allocation sizes, and make malformed or ended streams permanently unusable.

### 4. How it works

Binary messages are assembled from a length header and the bytes that follow it. The decoder bounds retained data and keeps a failed or ended stream from being used again.

The wire prefix is an unsigned 32-bit big-endian payload length occupying four bytes. Low-level FrameDecoder defaults to 16 MiB, accepts configured integer lengths from zero through the unsigned 32-bit maximum, and rejects invalid numbers before input. encodeFrame checks only the unsigned 32-bit representable payload length; assertCompleteFrame separately enforces the caller's maximum and exactly one complete frame with no suffix. Empty payloads are valid at this layer, although empty CBOR is invalid at the next layer (ice:packages/protocol/src/framing.ts:1-56).

FrameDecoder starts open, retains up to four header bytes, rejects an oversized length as soon as the header completes, and copies incoming payload bytes into incrementally allocated 64 KiB blocks. One complete block can be returned directly; multiple blocks are combined only when the frame completes. Input chunk mutation cannot alter returned frames. Each push may emit multiple frames in order, including empty frames. end succeeds exactly once on a clean boundary; a partial header or payload fails. Every failure clears retained header/payload state and permanently marks failed. Calls after ended or failed throw instead of resuming (ice:packages/protocol/src/framing.ts:58-166; ice:packages/protocol/test/framing.test.ts:29-108).

### 5. Interfaces

encodeFrame(Uint8Array)→Uint8Array; assertCompleteFrame(frame,{maxFrameLength?})→void; FrameDecoder.push(chunk)→Uint8Array[] and end()→void. Default frame limit is 16 MiB; the protocol/server/client wrappers tighten configured zero to invalid where documented in their own entries. Errors distinguish oversized, incomplete, trailing, ended and failed streams.

`DEFAULT_MAX_FRAME_LENGTH` is 16*1024*1024, `FrameDecoderOptions` exposes only optional maxFrameLength, and `FrameError` extends Error with name=`FrameError` and the supplied message (ice:packages/protocol/src/framing.ts:5-17).

### 6. Dependencies

Foundation for CBOR/schema validation and both binary transport endpoints; independent of the older JSONL protocol.

### 7. ice-rust today

Rust has genuine framing code with the same four-byte prefix, default limit, fragmentation/coalescing and initial capacity bounded by 64 KiB. It uses one growing Vec rather than the TS block list, which is an acceptable allocation implementation if the limit remains enforced. Its constructor takes arbitrary usize without the TS unsigned-32-bit option validation. More significantly, end never checks the prior state: after an oversized header marks Failed and resets header_len, end can succeed and replace Failed with Ended; repeated clean end also succeeds. push does check both states. These are concrete terminal-state differences, not absence of binary framing (rust:crates/ice-cli/src/remote/framing.rs:81-156). Existing in-file tests cover round-trip, fragmentation, empty frames, oversize and trailing assertions, but not all terminal-state cases; source inspection only (rust:crates/ice-cli/src/remote/framing.rs:160-214).

### 8. Port notes

Retain the native frame implementation and enforce the same constructor domain and terminal transition rules. Check state first in end; on failure clear buffered frame state and retain Failed. Preserve the exact-at-limit boundary, empty frames and copied-input ownership. Add meaningful implementation checks for end after oversize, repeated end, every split point and no input-buffer aliasing when implementation work is authorized; no tests were run in this audit.

### 9. Size and priority

Estimated Rust implementation: **45-110 lines** (estimate, not measured source). **P1** - The byte envelope is already implemented, but terminal-state drift lets callers treat a rejected stream as a successful clean EOF.

## RPC-002 - Strict CBOR value domain, allocation limits and tag-depth defect

### 1. Title and status

Strict CBOR value domain, allocation limits and tag-depth defect. **Status: partial.**

### 2. ice source

ice:packages/protocol/src/cbor/options.ts:1-53; ice:packages/protocol/src/cbor/encoder.ts:1-217; ice:packages/protocol/src/cbor/decoder.ts:1-169; ice:packages/protocol/test/cbor/cbor.test.ts:23-174; rust:crates/ice-cli/src/remote/cbor.rs:1-378.

Exported symbols: `UINT32_BASE`, `MAX_UINT32`, `DEFAULT_MAX_CBOR_BYTE_LENGTH`, `DEFAULT_MAX_CBOR_CONTAINER_LENGTH`, `DEFAULT_MAX_CBOR_DEPTH`, `CborOptions`, `ResolvedCborOptions`, `textEncoder`, `textDecoder`, `resolveOptions` (ice:packages/protocol/src/cbor/options.ts:1; ice:packages/protocol/src/cbor/options.ts:2; ice:packages/protocol/src/cbor/options.ts:6; ice:packages/protocol/src/cbor/options.ts:7; ice:packages/protocol/src/cbor/options.ts:8; ice:packages/protocol/src/cbor/options.ts:10; ice:packages/protocol/src/cbor/options.ts:19; ice:packages/protocol/src/cbor/options.ts:32; ice:packages/protocol/src/cbor/options.ts:33; ice:packages/protocol/src/cbor/options.ts:42).

### 3. What it does

Exchange one unambiguous bounded CBOR value without accepting lossy numbers, duplicate keys or recursively unbounded tags.

### 4. How it works

The binary codec accepts a deliberately limited set of values and rejects ambiguous or excessive input. Its size and nesting limits must apply before expensive allocation or recursion.

Defaults are 16 MiB encoded bytes, one million array/map members and depth 64. Byte/container limits accept safe integers between zero and the unsigned 32-bit maximum; depth accepts integers zero through 512. Root depth is zero, each nested container member advances depth. The encoder's bounded writer rejects aggregate growth before allocating past the byte limit; declared byte/container lengths and depth are checked before traversing decoder values (ice:packages/protocol/src/cbor/options.ts:1-53; ice:packages/protocol/src/cbor/encoder.ts:1-86; ice:packages/protocol/src/cbor/decoder.ts:26-86).

Supported values are null, booleans, finite numbers, Unicode strings, Uint8Array, arrays and plain or null-prototype objects. Integers must fit JavaScript's safe range; negative zero is preserved with float64. Noninteger numbers use float64. Strings must round-trip through fatal UTF-8 encoding/decoding; unpaired surrogates fail. Object properties with undefined values are omitted, but array holes/undefined fail. Enumerable symbol keys, unsupported object prototypes and cycles fail; repeated noncyclic references are allowed. Object property iteration order is retained (ice:packages/protocol/src/cbor/encoder.ts:88-217; ice:packages/protocol/test/cbor/cbor.test.ts:69-117).

Decoder accepts exactly one definite-length item, rejects trailing bytes, non-string or duplicate map keys, tags, indefinite forms, reserved simple values, undefined, break, float16/float32, nonfinite float64 and unsafe integer values even if encoded as float64. Byte strings return a fresh Uint8Array. Map properties are defined as own data properties, so __proto__ remains data rather than changing the object's prototype; a leading BOM is preserved (ice:packages/protocol/src/cbor/decoder.ts:1-169; ice:packages/protocol/test/cbor/cbor.test.ts:74-82; ice:packages/protocol/test/cbor/cbor.test.ts:120-174).

### 5. Interfaces

encodeCbor(unknown,{maxByteLength?, maxContainerLength?, maxDepth?})→Uint8Array; decodeCbor(bytes, options?)→unknown; failures are CborError. Raw bytes are supported by this low-level codec but deliberately rejected in JSON-valued protocol fields by RPC-003.

`CborOptions` makes maxByteLength/maxContainerLength/maxDepth optional; `ResolvedCborOptions` requires all three. `UINT32_BASE` is 0x1_0000_0000 and `MAX_UINT32` is 0xffff_ffff. The exported TextEncoder and fatal UTF-8 TextDecoder with ignoreBOM:true are the encoder/decoder instances used by this value-domain boundary. `resolveOptions` applies nullish defaults then safe-integer inclusive limits of MAX_UINT32 for byte/container length and 512 for depth (ice:packages/protocol/src/cbor/options.ts:1-53).

### 6. Dependencies

`RPC-001`

The codec and frame limits must agree at protocol entrypoints. No model/provider or authenticated service is required for these pure transformations.

### 7. ice-rust today

Rust's public CBOR module has the same three defaults and real encode/decode behavior, but serde_json::Value narrows and changes its value domain. Full u64/i64 values are accepted rather than safe JavaScript integers; byte strings become arrays of JSON numbers, duplicate map keys overwrite earlier values, tags are ignored, float32 is accepted, and NaN/infinity float32/64 decode to null. Encoder aggregate size is checked only after the complete Vec has been built. Options are arbitrary usize with no TS range/depth validation (rust:crates/ice-cli/src/remote/cbor.rs:35-140; rust:crates/ice-cli/src/remote/cbor.rs:185-315).

Source-only native defect: the major-type-six tag branch calls decode_value with unchanged depth, after the ordinary depth guard. A long chain of small tags can therefore recurse far beyond max_depth and exhaust stack before consuming the allowed byte budget. No reproduction was executed. This applies to direct users of this public helper; audited searches found binary codec references in its module/tests, while the inspected RemoteDriver uses the separate TCP JSON-RPC path. It is not evidence that the currently inspected native --connect endpoint accepts CBOR (rust:crates/ice-cli/src/remote/cbor.rs:185-195; rust:crates/ice-cli/src/remote/cbor.rs:276-279; rust:crates/ice-cli/src/remote/codec.rs:83-90; rust:crates/ice-cli/src/session_driver.rs:215-264).

### 8. Port notes

Use a codec value representation that retains byte strings until schema validation, or reject byte strings explicitly in the protocol decoder rather than silently converting them to valid JSON arrays. Enforce safe-number, duplicate-key, tag/simple-value and finite-float rules; reject tags immediately to close the depth bypass. Bound writer growth before each append, validate options and retain detailed decode errors. Native JSON numbers must not silently turn nonfinite data into null. Preserve the existing public helper only behind a clearly named permissive mode if compatibility consumers require it; the TS-compatible path must remain strict.

### 9. Size and priority

Estimated Rust implementation: **150-330 lines** (estimate, not measured source). **P1** - Lossy decoding changes tool/event data, and tag recursion bypasses the advertised resource bound before any future binary transport exposes this helper.

## RPC-003 - Closed version-one binary schemas and validated codec boundaries

### 1. Title and status

Closed version-one binary schemas and validated codec boundaries. **Status: missing.**

### 2. ice source

ice:packages/protocol/src/schemas.ts:1-447; ice:packages/protocol/src/codec.ts:1-173; ice:packages/protocol/test/protocol.test.ts:54-381; rust:crates/ice-cli/src/remote/codec.rs:44-96; rust:crates/ice-cli/src/remote/protocol.rs:24-119.

Exported symbols: `createClientMessageDecoder`, `createServerMessageDecoder`, `isSupportedProtocolVersion` (ice:packages/protocol/src/codec.ts:162; ice:packages/protocol/src/codec.ts:166; ice:packages/protocol/src/codec.ts:170).
Exported symbols: `PROTOCOL_VERSION`, `JsonValueSchema`, `ThinkingLevelSchema`, `SessionPhaseSchema`, `SessionPhase`, `ModelRefSchema`, `ModelCostSchema`, `ModelMetadataSchema`, `ModelMetadata`, `TextContentSchema`, `ThinkingContentSchema`, `ImageContentSchema`, `ToolCallContentSchema`, `UserContentSchema`, `AssistantContentSchema`, `ToolContentSchema`, `ToolCallContent`, `UsageSchema`, `UserTranscriptItemSchema`, `AssistantTranscriptItemSchema`, `ToolTranscriptItemSchema`, `TranscriptItemSchema`, `UserTranscriptItem`, `AssistantTranscriptItem`, `ToolTranscriptItem`, `TranscriptItem`, `TranscriptProgressSchema`, `SessionSummarySchema`, `SessionSnapshotSchema`, `ServerSnapshotSchema`, `ProtocolErrorCodeSchema`, `ProtocolErrorSchema`, `ProtocolErrorCode`, `ProtocolError`, `ListCommandSchema`, `CreateCommandSchema`, `AttachCommandSchema`, `DetachCommandSchema`, `PromptCommandSchema`, `SteerCommandSchema`, `AbortCommandSchema`, `SetModelCommandSchema`, `SetThinkingCommandSchema`, `CommandSchema`, `CommandName`, `CreateResultSchema`, `AttachResultSchema`, `PromptResultSchema`, `SteerResultSchema`, `AbortResultSchema`, `SetModelResultSchema`, `SetThinkingResultSchema`, `ListResultSchema`, `DetachResultSchema`, `CommandResultSchema`, `CommandResult`, `ResultForCommand`, `ClientHelloSchema`, `ClientHello`, `RequestEnvelopeSchema`, `RequestEnvelope`, `ClientMessageSchema`, `ServerEventSchema`, `ServerHelloSchema`, `ServerHelloErrorSchema`, `ResponseEnvelopeSchema`, `EventEnvelopeSchema`, `ServerMessageSchema`, `ServerHello`, `ServerHelloError`, `ResponseEnvelope`, `EventEnvelope` (ice:packages/protocol/src/schemas.ts:3; ice:packages/protocol/src/schemas.ts:24; ice:packages/protocol/src/schemas.ts:26; ice:packages/protocol/src/schemas.ts:39; ice:packages/protocol/src/schemas.ts:46; ice:packages/protocol/src/schemas.ts:48; ice:packages/protocol/src/schemas.ts:54; ice:packages/protocol/src/schemas.ts:61; ice:packages/protocol/src/schemas.ts:74; ice:packages/protocol/src/schemas.ts:76; ice:packages/protocol/src/schemas.ts:80; ice:packages/protocol/src/schemas.ts:85; ice:packages/protocol/src/schemas.ts:90; ice:packages/protocol/src/schemas.ts:96; ice:packages/protocol/src/schemas.ts:97; ice:packages/protocol/src/schemas.ts:98; ice:packages/protocol/src/schemas.ts:102; ice:packages/protocol/src/schemas.ts:104; ice:packages/protocol/src/schemas.ts:121; ice:packages/protocol/src/schemas.ts:157; ice:packages/protocol/src/schemas.ts:189; ice:packages/protocol/src/schemas.ts:194; ice:packages/protocol/src/schemas.ts:199; ice:packages/protocol/src/schemas.ts:200; ice:packages/protocol/src/schemas.ts:201; ice:packages/protocol/src/schemas.ts:202; ice:packages/protocol/src/schemas.ts:205; ice:packages/protocol/src/schemas.ts:247; ice:packages/protocol/src/schemas.ts:248; ice:packages/protocol/src/schemas.ts:258; ice:packages/protocol/src/schemas.ts:267; ice:packages/protocol/src/schemas.ts:274; ice:packages/protocol/src/schemas.ts:279; ice:packages/protocol/src/schemas.ts:280; ice:packages/protocol/src/schemas.ts:287; ice:packages/protocol/src/schemas.ts:288; ice:packages/protocol/src/schemas.ts:295; ice:packages/protocol/src/schemas.ts:296; ice:packages/protocol/src/schemas.ts:297; ice:packages/protocol/src/schemas.ts:298; ice:packages/protocol/src/schemas.ts:299; ice:packages/protocol/src/schemas.ts:300; ice:packages/protocol/src/schemas.ts:305; ice:packages/protocol/src/schemas.ts:310; ice:packages/protocol/src/schemas.ts:322; ice:packages/protocol/src/schemas.ts:324; ice:packages/protocol/src/schemas.ts:328; ice:packages/protocol/src/schemas.ts:332; ice:packages/protocol/src/schemas.ts:336; ice:packages/protocol/src/schemas.ts:340; ice:packages/protocol/src/schemas.ts:344; ice:packages/protocol/src/schemas.ts:348; ice:packages/protocol/src/schemas.ts:353; ice:packages/protocol/src/schemas.ts:357; ice:packages/protocol/src/schemas.ts:361; ice:packages/protocol/src/schemas.ts:372; ice:packages/protocol/src/schemas.ts:374; ice:packages/protocol/src/schemas.ts:381; ice:packages/protocol/src/schemas.ts:385; ice:packages/protocol/src/schemas.ts:387; ice:packages/protocol/src/schemas.ts:392; ice:packages/protocol/src/schemas.ts:393; ice:packages/protocol/src/schemas.ts:396; ice:packages/protocol/src/schemas.ts:408; ice:packages/protocol/src/schemas.ts:414; ice:packages/protocol/src/schemas.ts:418; ice:packages/protocol/src/schemas.ts:432; ice:packages/protocol/src/schemas.ts:436; ice:packages/protocol/src/schemas.ts:442; ice:packages/protocol/src/schemas.ts:443; ice:packages/protocol/src/schemas.ts:444; ice:packages/protocol/src/schemas.ts:445).
Exported symbols: `WireChannel`, `ProtocolTestClient` (ice:packages/server/src/testing/client.ts:20; ice:packages/server/src/testing/client.ts:26).

### 3. What it does

Make binary clients and servers agree on an exact versioned message vocabulary and reject unsupported shapes before dispatch or transmission.

### 4. How it works

Version one defines the exact messages a binary client and server may exchange. Both sides validate complete shapes before sending or acting on a message.

Version is 1. All public object schemas reject additional properties. Identifiers are nonempty strings and timestamps/revisions/counts nonnegative integers. Client hello permits any nonnegative integer version for negotiation, while server hello and ServerSnapshot require version 1. Envelopes are client {type:'hello', version} or {type:'request', id, request}; server hello adds connectionId and snapshot, hello_error carries a typed error, response has id plus ok:true/result or ok:false/error, and event wraps a ServerEvent. Error codes are exactly version, busy, session_locked, not_found and invalid_request; error details are optional JSON values (ice:packages/protocol/src/schemas.ts:3-24; ice:packages/protocol/src/schemas.ts:258-280; ice:packages/protocol/src/schemas.ts:381-447).

The nine binary commands are list, create, attach, detach, prompt, steer, abort, set_model and set_thinking. create may carry nonempty cwd, any string name, ModelRef and ThinkingLevel; other session commands require sessionId. prompt/steer accept text only, including empty text, with no image input field. set_model uses {provider, id}; set_thinking uses thinkingLevel. Results retain the command discriminator; list returns summaries, detach returns sessionId, and the other commands return a SessionSnapshot. Session phases are idle, turn, compaction, branch_summary and retry; thinking levels run off/minimal/low/medium/high/xhigh/max/ultra (ice:packages/protocol/src/schemas.ts:26-74; ice:packages/protocol/src/schemas.ts:282-379; ice:packages/protocol/test/protocol.test.ts:62-97).

ServerSnapshot includes serverId, protocolVersion, revision, sessions and models. SessionSummary includes id, optional name, cwd, createdAt, updatedAt, phase, model, thinkingLevel, attached and locked. SessionSnapshot adds revision, transcript, queuedSteer and queuedSteerCount; the schema does not assert count equals array length. Model metadata includes provider/id/name/api, reasoning, text/image input capabilities, positive integer contextWindow/maxTokens, four nonnegative cost fields, at least one supported thinking level and authenticated (ice:packages/protocol/src/schemas.ts:48-74; ice:packages/protocol/src/schemas.ts:234-265).

Transcript roles are user, assistant and tool. User has text/image content. Assistant has text/thinking/toolCall content, model, optional responseModel/usage and timestamp; streaming forbids a stopReason, complete requires stop/length/toolUse, error requires error and permits only a nonempty optional errorMessage, aborted requires aborted and permits an empty optional errorMessage. Tool items carry toolCallId/toolName/input, text/image content, optional details/usage and timestamp; running/complete require isError:false, error requires true. Tool-call input is JSON and uses the field name input. Usage preserves input/output/cacheRead/cacheWrite/totalTokens, optional reasoning and five nonnegative cost fields. Progress types are item_started, assistant_delta, item_updated and item_finished; only terminal assistant/tool items are allowed in item_finished. Server events are server_snapshot, session_snapshot, session_progress and session_removed (ice:packages/protocol/src/schemas.ts:76-232; ice:packages/protocol/src/schemas.ts:396-406).

Before schema checking, protocol values must be finite JSON-compatible trees with ordinary Object.prototype objects; null-prototype objects, byte strings, cycles and array undefined values fail. Explicit undefined object properties are allowed to be omitted by encoding. parse APIs take decoded values, not JSON strings. Encoding validates before CBOR/framing and enforces the output frame limit. Any framing, CBOR or schema failure permanently fails the validated decoder; errors are bounded to 500 characters and do not retain the rejected payload (ice:packages/protocol/src/codec.ts:18-127; ice:packages/protocol/test/protocol.test.ts:252-381).

Source-read conformance support: `WireChannel` supplies send, sendFragmented and close promises. `ProtocolTestClient` encodes client messages with the same codec and accumulates messages returned by ServerMessageDecoder. hello waits for hello/hello_error; request allocates request-N unless an id is supplied, registers a matching response waiter before sending, then returns that response. next/nextFrom may reuse an already recorded matching message; otherwise a closed client rejects and an open client registers a waiter. receive resolves all matching waiters for each decoded message. fail rejects/clears current waiters without setting closed; markClosed is idempotent, sets closed, resolves waitForClose and rejects remaining waiters. It does not call decoder.end on close. These exported fixtures are test support for this protocol contract, not an additional production client implementation (ice:packages/server/src/testing/client.ts:20-114).

### 5. Interfaces

ClientMessage/ServerMessage and all schemas/types are package exports. parseClientMessage/parseServerMessage return the original checked value; encodeClientMessage/encodeServerMessage return one complete frame. Separate ClientMessageDecoder/ServerMessageDecoder expose push/end and latch failure. The nine-command binary schema is distinct from the coding-agent JSONL command union and the native TCP proxy's method envelope.

The explicitly named schema/type exports in field 2 are the existing version-one vocabulary. Text is `{type:'text',text:string}`; thinking adds optional redacted:boolean; image requires string data and nonempty mimeType; toolCall uses nonempty toolCallId/toolName plus JSON input. Model cost requires four nonnegative numeric rates, and usage requires five nonnegative integer token totals plus five nonnegative numeric costs, with optional nonnegative reasoning count. `ResultForCommand` selects the list result, detach result or the matching discriminated command result; it does not introduce a different wire response. Decoder factories construct the existing ClientMessageDecoder/ServerMessageDecoder with optional frame options, and isSupportedProtocolVersion requires Number.isInteger plus equality to 1 (ice:packages/protocol/src/schemas.ts:54-118; ice:packages/protocol/src/schemas.ts:374-378; ice:packages/protocol/src/codec.ts:162-172).

### 6. Dependencies

`AI-005`, `RPC-001`, `RPC-002`

RPC owns transport DTOs, field consistency and schema rejection. AI-005 owns shared thinking-level behavior. Adapters must intentionally project the cited source data rather than serialize every provider-specific field; general DTO shape is not an AI-006 hard-output-authority dependency.

Final dependency classification. Shared contract references: `AI-019`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

Rust's MessageDecoder returns arbitrary serde_json::Value and encodes any JSON value; it does not validate either direction against this closed schema and has no latch for a CBOR-level failure. Its existing RemoteEvent/RemoteCommand/RemoteResponse types are genuine but describe the older ready/response/status JSONL protocol, with agent_start/message_update/tool_execution events rather than authoritative binary snapshots. Reusing them would change the new protocol's envelope and data semantics. A real framing/CBOR helper therefore does not establish binary protocol schema parity (rust:crates/ice-cli/src/remote/codec.rs:44-96; rust:crates/ice-cli/src/remote/protocol.rs:24-119; rust:crates/ice-cli/src/remote/protocol.rs:254-395).

### 8. Port notes

Define the version-one DTOs with closed serde schemas, explicit enum states and validation for numeric/identifier constraints. Validate all inbound/outbound messages and match response command types to requests. Add JSON-domain rejection before lossy CBOR conversion, bounded errors and a failure latch encompassing frame, CBOR and schema stages. Keep the native JSONL DTOs for their existing transport; select protocols explicitly rather than infer from overlapping command names. Preserve every listed field/omission, including ultra, pending-without-stopReason and text-only command input.

### 9. Size and priority

Estimated Rust implementation: **450-850 lines** (estimate, not measured source). **P1** - Without the exact envelope and schemas, the existing binary bytes cannot interoperate safely with the TS client/server contract.

## RPC-004 - Explicit AI-to-protocol transcript and diagnostic projection

### 1. Title and status

Explicit AI-to-protocol transcript and diagnostic projection. **Status: missing.**

### 2. ice source

ice:packages/server/src/protocol.ts:1-383; ice:packages/server/test/protocol.test.ts:84-304; rust:crates/ice-cli/src/remote/protocol.rs:146-252; rust:crates/ice-cli/src/session_driver.rs:101-201.

Exported symbols: `AssistantTranscriptOptions`, `UserTranscriptOptions`, `ToolTranscriptOptions` (ice:packages/server/src/protocol.ts:108; ice:packages/server/src/protocol.ts:112; ice:packages/server/src/protocol.ts:116).

### 3. What it does

Expose stable transcript data while preserving tool-call identity and preventing provider replay fields or arbitrary diagnostic objects from leaking into the binary contract.

### 4. How it works

The server converts internal model and transcript data into stable protocol records. Execution inputs remain strict, while diagnostic details follow a separate, explicitly lossy conversion.

Compile-time ExactKeys assertions enumerate both retained and deliberately omitted AI fields. Projection excludes provider replay/signature data, diagnostic transport details, cacheWrite1h, sampling/configuration/pricing-tier state and deferred-tool availability; this is an intentional wire boundary, not a request to add every omitted field (ice:packages/server/src/protocol.ts:1-106).

Execution data uses toProtocolJsonValue: null/string/boolean and finite numbers pass, arrays and ordinary/null-prototype objects recurse, and cycles, undefined, functions, symbols, bigint, nonfinite numbers and unsupported object prototypes throw. Sparse arrays are traversed with Array.from, so holes throw as undefined. Object entries are assigned into a fresh ordinary object; the implementation does not use the low-level CBOR decoder's defineProperty construction. Diagnostic details use a different lossy sanitizer: nonfinite numbers and bigint become strings, Date becomes ISO text, unsupported scalar values are omitted, array omissions become null and cycles become '[Circular]'. Objects are reduced through enumerable entries even for class instances (ice:packages/server/src/protocol.ts:142-186; ice:packages/server/test/protocol.test.ts:137-180; ice:packages/server/test/protocol.test.ts:288-304).

Usage floors finite token counts and clamps them at zero, substitutes zero for nonfinite mandatory counts/costs, omits nonfinite optional reasoning, and does not recompute totalTokens or total cost. Model metadata validates nonempty identifiers, copies input, obtains supported levels from the shared helper and accepts an explicit authenticated flag. contextWindow/maxTokens are floored and clamped to at least one; nonfinite values are not normalized there and will fail downstream encoding (ice:packages/server/src/protocol.ts:188-245).

User text strings become one text block; arrays preserve text/images. Identifier and timestamp checks precede transcript output. Assistant tool calls rename id/name/arguments to toolCallId/toolName/input and validate arguments without lossy conversion. pending becomes streaming with no stopReason; stop/length/toolUse become complete; deferred throws because version one has no deferred state; error allows missing but rejects empty errorMessage; aborted allows empty. Tool-result conversion requires the supplied call's id and name to match the result, carries strict input plus sanitized details, and maps isError to error/complete consistently (ice:packages/server/src/protocol.ts:247-381; ice:packages/server/test/protocol.test.ts:182-286).

### 5. Interfaces

toProtocolUserMessage(message,{id}), toProtocolAssistantMessage(message,{id}), toProtocolToolResultMessage(message,{id, call}), toProtocolModelMetadata(model, authenticated), toProtocolUsage, toProtocolJsonValue and sanitizeProtocolDetails. These helpers return normalized DTOs; full closed-schema/frame validation still occurs at encoding.

`AssistantTranscriptOptions` and `UserTranscriptOptions` each require id:string; `ToolTranscriptOptions` additionally requires call:ToolCall. These are projection inputs, not optional transcript wire fields (ice:packages/server/src/protocol.ts:108-119).

### 6. Dependencies

`AI-005`, `AI-019`, `RPC-003`

The projection owns transport-specific omission and normalization. Shared AI wire/state semantics remain AI-owned, and the binary schema intentionally rejects deferred assistant messages.

### 7. ice-rust today

Rust already projects process-local AgentEvent values into a stable native JSONL RemoteEvent, and its shared TUI driver translates assistant/tool content into presentation events. Those are useful native capabilities to preserve, but neither produces the new role/status/id/snapshot DTOs or this split between strict execution JSON and sanitized diagnostic JSON. Serializing current native agent messages directly would retain a different shape and cannot satisfy this bridge (rust:crates/ice-cli/src/remote/protocol.rs:146-252; rust:crates/ice-cli/src/session_driver.rs:101-201; rust:crates/ice-cli/src/session_driver.rs:333-431).

### 8. Port notes

Add a dedicated binary DTO adapter at the backend boundary, keeping the current native JSONL projection. Require stable transcript IDs and matching originating tool calls, and explicitly map each assistant terminal state. Implement separate strict input conversion and lossy diagnostic normalization, with the listed numeric and omission rules. Do not turn the intentional deferred/replay omissions into invented missing features; document those protocol-v1 limits. Preserve any shared AI corrections through the dependencies rather than duplicating provider transformations.

### 9. Size and priority

Estimated Rust implementation: **180-360 lines** (estimate, not measured source). **P1** - A schema-compatible transport still needs correct transcript identity and data conversion; direct serialization would corrupt or reject common message/tool states.

## RPC-005 - Binary server handshake stages and failure containment

### 1. Title and status

Binary server handshake stages and failure containment. **Status: missing.**

### 2. ice source

ice:packages/server/src/server.ts:34-337; ice:packages/server/src/connection.ts:1-37; ice:packages/server/test/conformance.test.ts:44-163; ice:packages/server/test/server.test.ts:56-99.

Exported symbols: `ByteConnectionHandler`, `ByteConnectionAcceptor`, `ConnectionStage`, `ConnectionState`, `isTerminalConnection` (ice:packages/server/src/connection.ts:12; ice:packages/server/src/connection.ts:18; ice:packages/server/src/connection.ts:20; ice:packages/server/src/connection.ts:22; ice:packages/server/src/connection.ts:34).
Exported symbols: `IceServerOperationErrorCode`, `SessionBusyError`, `SessionLockedError`, `SessionNotFoundError` (ice:packages/server/src/errors.ts:3; ice:packages/server/src/errors.ts:21; ice:packages/server/src/errors.ts:28; ice:packages/server/src/errors.ts:35).
Exported symbols: `TestServerOptions`, `TestServer`, `createTestServer` (ice:packages/server/src/testing/server.ts:5; ice:packages/server/src/testing/server.ts:9; ice:packages/server/src/testing/server.ts:15).
Exported symbols: `IceServerOptions` (ice:packages/server/src/types.ts:14).

### 3. What it does

Negotiate exactly one binary hello before dispatching concurrent requests, with a deadline that covers snapshot construction and sending the server hello.

### 4. How it works

A binary connection must complete its version handshake before requests may run. The deadline includes snapshot preparation and sending the server hello, so slow backend work cannot leave negotiation open indefinitely.

A connection receives a UUID connectionId, its own ClientMessageDecoder and attached-session set, and starts awaitingHello. Defaults are 16 MiB maximum frame length and a 5,000 ms handshake deadline. Configured frame length must be an integer from one through the unsigned 32-bit maximum; timeout must be a positive safe integer no greater than Node's maximum timer delay. The unref timer begins at accept, not at receipt of client hello. First message must be hello; the server moves to handshaking, checks version 1, obtains the server snapshot and sends hello. It becomes ready only after the send settles, clears the timer then, and publishes a catch-up snapshot if revision changed during negotiation (ice:packages/server/src/server.ts:34-77; ice:packages/server/src/server.ts:113-145; ice:packages/server/src/server.ts:180-245).

Requests received during handshaking wait on that handshake promise and dispatch only if the connection is still ready. Duplicate hello fails in every later stage. Ready requests execute concurrently and may respond out of order; this core has no duplicate-request-id tracker or request admission queue. Those mechanisms in coding-agent JSONL RPC must not be attributed to the binary server. Each success retains the request id and result command; failures produce a typed error response (ice:packages/server/src/server.ts:180-264; ice:packages/server/test/conformance.test.ts:275-295).

Malformed framing/CBOR/schema input, timeout and negotiation violations move the connection to closing, clear its timer, and attempt a final hello_error frame even if the connection previously completed a handshake. Encoding failure may prevent that last frame. close receives the optional final bytes and disconnect cleanup runs without waiting on a blocked ordinary send queue. Transport EOF asks the decoder to finish and reports partial-frame errors. All observer errors are isolated. Only known IceServerError and ProtocolValidationError details cross the wire; unexpected backend failures are reported locally and return invalid_request with 'Internal server error' (ice:packages/server/src/server.ts:266-365; ice:packages/server/test/server.test.ts:56-82; ice:packages/server/test/conformance.test.ts:248-273).

`isTerminalConnection` returns true when disconnected or stage is closing or closed. `IceServerOperationErrorCode` selects busy|session_locked|not_found|invalid_request from the protocol error union. SessionBusyError, SessionLockedError and SessionNotFoundError extend IceServerError with the corresponding code and default messages `Session is busy`, `Session is locked` and `Session was not found`; they preserve supplied message/optional JSON details and set their own class names. They are the recognized safe failure family already described above (ice:packages/server/src/connection.ts:34-36; ice:packages/server/src/errors.ts:3-40).

Source-read conformance support: TestServerOptions extends IceServerOptions with optional backend; TestServer contains server and backend. createTestServer chooses options.backend or a new TestSessionBackend, constructs an unstarted IceServer and forwards listeners/maxFrameLength/handshakeTimeoutMs/serverId/onError. It neither starts listeners nor authenticates a peer; these helpers support the existing handshake/server conformance contract (ice:packages/server/src/testing/server.ts:5-27).

### 5. Interfaces

IceServer.accept(ByteConnection)→{onData, onError, onClose}; ByteConnection.send returns completion and close optionally accepts a final chunk. Connection stages awaitingHello/handshaking/ready/closing plus disconnected prevent late handshake resurrection. Options include listeners, serverId, maxFrameLength, handshakeTimeoutMs and onError.

Exact internal connection and server construction DTOs:

```ts
export interface ByteConnectionHandler {
	onData(chunk: Uint8Array): void;
	onClose(): void;
	onError(error: Error): void;
}

export type ByteConnectionAcceptor = (connection: ByteConnection) => ByteConnectionHandler;

export type ConnectionStage = "awaitingHello" | "handshaking" | "ready" | "closing" | "closed";

export interface ConnectionState {
	id: string;
	connection: ByteConnection;
	decoder: ClientMessageDecoder;
	sessionIds: Set<string>;
	stage: ConnectionStage;
	disconnected: boolean;
	handshakeComplete: boolean;
	handshake?: Promise<void>;
	handshakeTimeout: NodeJS.Timeout;
}
```
(ice:packages/server/src/connection.ts:12-32).

```ts
export interface IceServerOptions {
	listeners: readonly IceServerListener[];
	maxFrameLength?: number;
	handshakeTimeoutMs?: number;
	serverId?: string;
	onError?: (error: Error) => void;
}
```
(ice:packages/server/src/types.ts:14-20).

### 6. Dependencies

`RPC-001`, `RPC-002`, `RPC-003`

Requires a validated binary decoder and transport byte ordering; session execution/ownership and snapshot publication are specified separately.

### 7. ice-rust today

The inspected native CLI has a real JSONL readiness marker and TCP JSON-RPC proxy authentication, but neither performs this framed version-one hello/snapshot exchange. Native RemoteClient::call waits for method responses, and the headless loop emits ready immediately before reading JSON lines. Its genuine binary codec has no server handshake state machine. Audited crates searches found no IceServer/SessionSnapshot/UnixListener implementation or production binary MessageDecoder caller (rust:crates/ice-cli/src/remote/client.rs:27-106; rust:crates/ice-cli/src/modes.rs:395-450; rust:crates/ice-cli/src/remote/codec.rs:65-96). External ice-server package implementations are outside the permitted repository review and remain unverified.

### 8. Port notes

Add an explicit binary endpoint using the staged handshake and deadline, with separate validated input and queued byte output. Preserve ready requests' concurrent response correlation and do not reuse the native JSONL ready/status shape. Couple every asynchronous handshake continuation to current connection state, and send catch-up snapshots after a raced handshake. Keep unexpected backend errors local while returning only the public error schema.

### 9. Size and priority

Estimated Rust implementation: **230-450 lines** (estimate, not measured source). **P1** - A working transport needs a deterministic handshake, bounded startup and safe failure behavior before clients can attach to sessions.

## RPC-006 - Binary server listener lifecycle and serialized snapshot revisions

### 1. Title and status

Binary server listener lifecycle and serialized snapshot revisions. **Status: missing.**

### 2. ice source

ice:packages/server/src/server.ts:34-165; ice:packages/server/src/server.ts:325-337; ice:packages/server/src/snapshots.ts:21-64; ice:packages/server/test/listener.test.ts:30-56; ice:packages/server/test/sessions.test.ts:79-145.

### 3. What it does

Compose explicit listeners, publish monotonic server snapshots in order and release every owned connection/runtime when the binary server closes.

### 4. How it works

The server owns an explicit set of listeners and closes the resources it acquired. It serializes snapshot publication so asynchronous lookups cannot reverse the order of revisions.

Server construction requires a listeners array, including an explicitly empty array when desired; serverId defaults to a UUID and may not be empty. start rejects if already started, starting, closing or closed. Listeners start sequentially; a startup failure marks closing, closes already-started listeners with allSettled, closes server state and rethrows. The failing listener must clean its own partial resources. close returns the same promise, marks closing before awaits, waits a pending start, closes listeners and always closes server state. Server-state teardown first marks all connections closing and cancels handshake timers, closes all byte connections, disconnects their session attachments, closes session manager and clears connection tracking (ice:packages/server/src/server.ts:34-165; ice:packages/server/src/server.ts:325-337; ice:packages/server/test/listener.test.ts:47-56).

ServerSnapshotPublisher starts revision at zero. get captures serverId/version/revision and asynchronously obtains per-connection session summaries plus either supplied models or backend.listModels. broadcast appends work to a promise tail; failures are reported on the tail so later broadcasts continue, while the returned promise still rejects. A broadcast takes the currently ready, connected recipients, does nothing and does not advance revision when none exist or the server is closing, otherwise increments revision before reading models. It fetches models once, builds each recipient's attachment-relative session list, pins the one allocated revision on every emitted snapshot and awaits each send in recipient order. This serialization prevents a slow earlier model lookup from publishing a newer revision after a later one (ice:packages/server/src/snapshots.ts:21-64; ice:packages/server/test/sessions.test.ts:20-39; ice:packages/server/test/sessions.test.ts:79-114).

### 5. Interfaces

IceServer.start→Promise<this>, close→Promise<void>, addresses reflects defined listener addresses, accept attaches externally supplied byte transports. IceServerListener has optional address and asynchronous start/close. ServerSnapshotPublisher.get(models?, connection?), broadcast() and currentRevision are internal coordination contracts.

### 6. Dependencies

`RPC-005`, `RPC-007`

The live-session manager supplies attachment-relative summaries; protocol revision is a server publication counter, separate from per-runtime session revisions.

### 7. ice-rust today

Native modes::rpc has its own bounded stdout writer and joins active work on EOF/stop, which must be preserved. The --connect driver starts and stops a remote session through an external TCP server. Neither inspected path maintains this binary server's explicit listener set, per-recipient snapshot composition or serialized global revision counter. There is no genuine binary server manager to relabel as equivalent (rust:crates/ice-cli/src/modes.rs:571-648; rust:crates/ice-cli/src/session_driver.rs:215-281).

### 8. Port notes

Build listener composition and idempotent server teardown around the binary endpoint. Introduce one serialized global publication queue with recipient-relative attached flags, pin its revision across asynchronous model/session fetches, and preserve error recovery so one failed snapshot does not poison all future notifications. Keep per-session revision ownership in each runtime.

### 9. Size and priority

Estimated Rust implementation: **130-280 lines** (estimate, not measured source). **P1** - Out-of-order snapshots or leaked listeners/runtime handles make reconnect and authoritative client state unreliable.

## RPC-007 - Shared live-runtime ownership and work survival after detach

### 1. Title and status

Shared live-runtime ownership and work survival after detach. **Status: missing.**

### 2. ice source

ice:packages/server/src/sessions.ts:43-354; ice:packages/server/src/types.ts:1-65; ice:packages/server/test/sessions.test.ts:116-385; ice:packages/server/test/conformance.test.ts:165-245.

Exported symbols: `LiveSessionManager` (ice:packages/server/src/sessions.ts:43).
Exported symbols: `TEST_MODEL`, `TestSessionRuntime`, `TestSessionBackend` (ice:packages/server/src/testing/backend.ts:19; ice:packages/server/src/testing/backend.ts:52; ice:packages/server/src/testing/backend.ts:198).
Exported symbols: `MaybePromise`, `PromptInput`, `SteerInput`, `CreateSessionOptions`, `IceSessionRuntimeEvent`, `SessionRuntime`, `SessionBackend`, `SessionRuntimeEvent` (ice:packages/server/src/types.ts:22; ice:packages/server/src/types.ts:24; ice:packages/server/src/types.ts:25; ice:packages/server/src/types.ts:27; ice:packages/server/src/types.ts:36; ice:packages/server/src/types.ts:62; ice:packages/server/src/types.ts:63; ice:packages/server/src/types.ts:64).

### 3. What it does

Let several clients control one live session while holding one backend acquisition, and keep active work alive until it can be safely disposed.

### 4. How it works

Several client connections can share one acquired session runtime. Detaching the last client does not dispose work that is still running.

create assigns a UUID in the server and passes it to backend.createSession; attach obtains backend.openSession. Concurrent opens of the same id share one opening promise. Existing live runtimes are reused, terminal runtimes reject session_locked, and reacquisition waits for an already-disposing runtime before retrying. A newly acquired runtime must snapshot the assigned id. Wrong-id, subscription failure or server-closing acquisition is disposed; cleanup errors are reported while the original failure propagates. Subscription is established before the runtime is marked ready (ice:packages/server/src/sessions.ts:53-117; ice:packages/server/src/sessions.ts:193-253).

Each connection may attach multiple sessions, and multiple connections may attach the same runtime. Every attached client can prompt/steer/abort/change model/thinking. Operations increment operationCount before awaiting the backend, then broadcast the resulting snapshot and return attachment state relative to the requesting connection. Backend operations, not this manager, reject conflicting work; there is no prompt queue here. A prompt response waits for backend.prompt to settle, unlike the legacy JSONL admission acknowledgement. Controls may run while that prompt promise is pending (ice:packages/server/src/sessions.ts:53-117; ice:packages/server/src/sessions.ts:178-191; ice:packages/server/test/sessions.test.ts:220-307).

Full session snapshots and progress go only to attached connections. Normalized snapshots use runtime.getPhase, mark locked true while the live acquisition exists and calculate attached per recipient. listSessions merges stored summaries with live normalized summaries by id, preserving stored order and appending live-only ids. Stored nonlive entries become attached false but keep backend lock state. Per-session revision is supplied by the backend; this manager does not allocate it. Detach is idempotent even for unknown sessions. Disconnect removes all of that connection's attachments before checking disposal (ice:packages/server/src/sessions.ts:119-176; ice:packages/server/src/sessions.ts:255-325).

Unattached runtimes are disposed only when ready, not already disposing, operationCount is zero and either terminal or phase idle. Unattached busy work therefore survives; an idle event or the operation's finally block schedules disposal later. The subscription is removed before dispose, the disposal promise is stored immediately, and the live-map entry is removed in finally only if identity still matches. A terminal runtime error reports once, unsubscribes, closes every attached byte connection, disconnects them and disposes once eligible. Those connections may also own other sessions, so their whole attachment set is released. Closing the server waits all opens, clears live tracking and disposes each runtime (ice:packages/server/src/sessions.ts:142-176; ice:packages/server/src/sessions.ts:255-281; ice:packages/server/src/sessions.ts:327-354; ice:packages/server/test/sessions.test.ts:310-357; ice:packages/server/test/conformance.test.ts:221-245).

Source-read conformance support: TEST_MODEL is test/small, display Test Small, API test-api, reasoning enabled, text+image input, contextWindow16000/maxTokens2000, four zero costs, supported levels off/medium/high and authenticated:true. This is a synthetic test catalog row. TestSessionRuntime clones snapshots, keeps a pending prompt latch, appends a user item and enters turn, then appends `reply:<text>`/complete or an empty aborted assistant when the latch resolves. Steer requires non-idle, records the input and queued user item/count; abort resolves a pending latch or throws busy. Model/thinking setters require idle. update increments revision/updatedAt and emits snapshot; helper emit methods notify live listener sets directly. dispose increments disposeCount on each call, invokes its unlock callback and resolves a disposal latch, without implementing its own once-only guard (ice:packages/server/src/testing/backend.ts:19-31; ice:packages/server/src/testing/backend.ts:52-191).

TestSessionBackend keeps in-memory stored snapshots/runtime history/lock IDs, can pause exactly the next list call with entered/release latches, returns the synthetic model, and seeds deterministic idle revision-zero snapshots. Create rejects an existing id, open rejects missing/locked ids, and acquire records a new runtime whose disposal removes the lock. The fixture default cwd is `/tmp/ice-server-conformance`; no filesystem read/write is performed by this in-memory helper. These exports are conformance scaffolding for the backend/runtime boundary, without adding a production storage or model capability (ice:packages/server/src/testing/backend.ts:198-292).

### 5. Interfaces

IceSessionBackend.listSessions/listModels/createSession/openSession; acquired IceSessionRuntime.snapshot/getPhase/prompt/steer/abort/setModel/setThinking/subscribe/dispose. Backend acquisition is exclusive relative to other owners, while connections within this server share that one runtime. Runtime events are snapshot, progress or terminal error. Protocol command results are listed in RPC-003.

Exact backend input/event types and compatibility aliases:

```ts
export type MaybePromise<T> = T | Promise<T>;

export type PromptInput = Omit<Extract<Command, { command: "prompt" }>, "command" | "sessionId">;
export type SteerInput = Omit<Extract<Command, { command: "steer" }>, "command" | "sessionId">;

export interface CreateSessionOptions {
	/** A collision-resistant ID assigned by IceServer. The backend must persist this exact ID. */
	id: string;
	cwd?: string;
	name?: string;
	model?: ModelRef;
	thinkingLevel?: ThinkingLevel;
}

export type IceSessionRuntimeEvent =
	| { type: "snapshot" }
	| { type: "progress"; progress: TranscriptProgress }
	| { type: "error"; error: IceServerError };
```
(ice:packages/server/src/types.ts:22-39).

```ts
export type SessionRuntime = IceSessionRuntime;
export type SessionBackend = IceSessionBackend;
export type SessionRuntimeEvent = IceSessionRuntimeEvent;
```
(ice:packages/server/src/types.ts:62-64).

### 6. Dependencies

`RPC-003`, `RPC-004`, `RPC-005`, `SESSION-006`, `SESSION-028`

SESSION-006 covers local storage lease/ownership behavior and SESSION-028 SDK construction. RPC adds connection sharing, acquisition coalescing and backend disposal timing; it does not replace storage ownership.

### 7. ice-rust today

Native RemoteDriver owns one remote session id per driver and sends start_session/subscribe/stop_session through a separate external server. The inspected local JSONL loop drives one main lane and joins its active prompt on input closure. Real local session leases exist in SESSION-006, but the audited native remote modules do not implement this binary server's shared attachment map, per-runtime operation counts or lazy reopen manager. External package server ownership cannot be verified from this repository (rust:crates/ice-cli/src/session_driver.rs:207-311; rust:crates/ice-cli/src/modes.rs:316-319; rust:crates/ice-cli/src/modes.rs:571-599).

### 8. Port notes

Implement an adapter from the native session SDK to the backend/runtime interfaces, retaining one acquisition per id and coalescing opens. Track attachments separately from operation lifetime; dispose after detach only when no work or busy phase remains. Validate backend id before publishing, preserve connection-relative attached flags, and return terminal-operation snapshots after actual settlement. Port disposal/reacquisition races and multiple-controller scenarios from the source assertions when implementation is authorized.

### 9. Size and priority

Estimated Rust implementation: **280-550 lines** (estimate, not measured source). **P1** - This lifecycle is central to durable remote work, reconnect and safe concurrent control; simple session spawning does not supply it.

## RPC-008 - Unix listener publication, permissions and inode-safe cleanup

### 1. Title and status

Unix listener publication, permissions and inode-safe cleanup. **Status: missing.**

### 2. ice source

ice:packages/server/src/transports/unix/listener.ts:11-201; ice:packages/server/src/transports/unix/listener.ts:307-434; ice:packages/server/src/transports/unix/preset.ts:1-24; ice:packages/server/test/unix.test.ts:42-117; ice:packages/server/test/server.test.ts:32-53.

Exported symbols: `validateUnixSocketPath` (ice:packages/server/src/transports/unix/listener.ts:18).
Exported symbols: `UnixListenerOptions`, `UnixServerOptions` (ice:packages/server/src/transports/unix/types.ts:3; ice:packages/server/src/transports/unix/types.ts:15).

### 3. What it does

Expose the binary server on a local Unix socket while refusing live or non-socket collisions and protecting replacement files during cleanup.

### 4. How it works

The Unix listener publishes a socket only after checking ownership and permissions. Cleanup uses filesystem identity checks to preserve a replacement file or another process's socket.

Socket paths must be nonempty and fit 107 UTF-8 bytes on Linux or 103 elsewhere. The listener derives a private bind path in the same parent, named .p- plus the first eight SHA-256 hex characters of the configured path, validates its length too, creates parents recursively with requested mode 0700 and checks both public/private paths for staleness. It binds the private path, records its socket device/inode, hard-links that inode to the public path, applies configured mode (default 0600) and only then exposes address. Mode accepts integers zero through octal 0777. chmod ENOSYS/ENOTSUP is tolerated, while other failures trigger startup cleanup (ice:packages/server/src/transports/unix/listener.ts:11-99; ice:packages/server/src/transports/unix/listener.ts:307-309; ice:packages/server/src/transports/unix/listener.ts:378-434).

Stale cleanup refuses any non-socket. It probes a socket with a one-second unref deadline; connect or timeout counts as live, ECONNREFUSED/ENOENT/EPIPE/ECONNRESET counts stale, and other errors propagate. A suspected stale path is renamed to a randomized .s- preservation name and re-stat'd. Only matching socket device/inode is removed. If identity changed, the replacement is restored when the public path is absent, and cleanup throws rather than silently deleting it (ice:packages/server/src/transports/unix/listener.ts:312-375).

Shutdown clears address, closes listener/connections and removes private binding resources. Public-path removal first confirms stored socket identity, renames it to a .c- preservation name, checks identity again and unlinks only the owned socket. A raced replacement is restored if possible and otherwise kept at the preservation path with an error describing it. A replacement regular file or inode seen before rename is left untouched. start rejects repeat/closing state and accepted connections are destroyed while closing. Observer errors cannot change listener state (ice:packages/server/src/transports/unix/listener.ts:101-201; ice:packages/server/test/unix.test.ts:43-117).

### 5. Interfaces

createUnixListener({path, mode?, maxFrameLength?, maxPendingBytes?, gracefulCloseTimeoutMs?, onError?}) implements IceServerListener. createUnixServer combines this listener with the binary server options. The local filesystem permission boundary is the transport authorization here; protocol hello has no token field.

Exact Unix construction options; UnixServerOptions composes ordinary server options except listeners with the listener options:

```ts
export interface UnixListenerOptions {
	path: string;
	/** Socket filesystem permissions. Defaults to owner read/write only (0o600). */
	mode?: number;
	/** Maximum framed bytes queued per connection before a slow peer is disconnected. */
	maxPendingBytes?: number;
	gracefulCloseTimeoutMs?: number;
	/** Used to derive and validate maxPendingBytes. Must match the server when customized. */
	maxFrameLength?: number;
	onError?: (error: Error) => void;
}

export interface UnixServerOptions extends Omit<IceServerOptions, "listeners">, UnixListenerOptions {}
```
(ice:packages/server/src/transports/unix/types.ts:3-15).

### 6. Dependencies

`RPC-005`, `RPC-006`, `RPC-009`

The listener owns socket filesystem publication and cleanup; RPC-009 owns accepted byte-connection queues and final close. Experimental --auth-token parsing is separate and does not prove an auth layer in this socket transport.

### 7. ice-rust today

The inspected native --connect implementation opens tokio TCP streams and invokes the external server's authenticate method; native framed-CBOR modules contain no socket listener. Audited crates-wide symbol searches did not find a UnixListener or an IceServer binary server implementation. This is a missing transport surface, not evidence that native TCP authentication is absent (rust:crates/ice-cli/src/remote/client.rs:18-70; rust:crates/ice-cli/src/remote/mod.rs:1-34).

### 8. Port notes

Add a Unix listener for the new binary endpoint with explicit path-length/platform behavior, private binding publication and identity-based stale/owned cleanup. Preserve refusal of regular files and live sockets, probe timeout semantics and replacement preservation. Keep socket permissions configurable and surface unsupported-platform behavior clearly; do not silently reinterpret unix addresses as TCP endpoints.

### 9. Size and priority

Estimated Rust implementation: **180-360 lines** (estimate, not measured source). **P1** - A local transport needs reliable ownership and cleanup; accidental deletion or a stale/live socket collision can break unrelated processes.

## RPC-009 - Bounded ordered Unix byte transport and final close semantics

### 1. Title and status

Bounded ordered Unix byte transport and final close semantics. **Status: missing.**

### 2. ice source

ice:packages/server/src/transports/unix/listener.ts:203-305; ice:packages/server/src/transports/unix/listener.ts:405-434; ice:packages/client/src/unix.ts:1-157; ice:packages/server/test/unix-connection.test.ts:7-65; ice:packages/client/test/unix.test.ts:40-222.

Exported symbols: `ByteTransportHandlers` (ice:packages/client/src/transport.ts:8).
Exported symbols: `UnixTransportOptions` (ice:packages/client/src/unix.ts:7).
Exported symbols: `connectUnixTestClient` (ice:packages/server/src/testing/client.ts:116).

### 3. What it does

Preserve frame byte ordering under backpressure, cap retained writes and deliver one terminal transport notification.

### 4. How it works

The Unix transports copy and serialize outgoing bytes within a retained-byte budget. Closing settles pending work according to the client or server contract and delivers a single terminal notification.

Server UnixByteConnection validates Uint8Array, rejects sends after closing/closed and rejects when retained bytes plus the new chunk exceed maxPendingBytes. It copies accepted bytes, increments retained count before queuing on a promise tail, writes one chunk at a time and decrements in finally. The recovered tail prevents one rejected write from poisoning future queue bookkeeping. Defaults derive pending bytes as four times maxFrameLength; configuration must be a safe integer at least maxFrameLength plus four bytes. Writes settle on socket callback or close; this server wrapper does not explicitly wait for drain (ice:packages/server/src/transports/unix/listener.ts:203-241; ice:packages/server/src/transports/unix/listener.ts:283-305; ice:packages/server/src/transports/unix/listener.ts:411-418).

Server close is idempotent, copies an optional final frame, marks closing immediately and starts a default five-second unref forced-destroy timer. After its write tail settles it socket.end's the final bytes, then resolves on markClosed; the final chunk bypasses ordinary pending-byte admission. Exact source boundary: a write already entered before close is awaited, but accepted queued writes that have not entered private write will see closing and reject. The existing final-error test starts the first write before close and verifies that in-flight case only. Do not turn that test into a claim that all previously queued chunks are delivered (ice:packages/server/src/transports/unix/listener.ts:243-304; ice:packages/server/test/unix-connection.test.ts:44-65).

The client factory validates nonempty/path byte limits, rejects Windows, and defaults maxPendingBytes to four times the protocol's 16 MiB frame default; its pending limit only needs to be a positive safe integer. Each factory call makes a fresh socket. Before connection completion, errors/close reject acquisition; afterward exactly one onError or onClose is delivered. Local close is idempotent and suppresses a later terminal callback. Client writes also copy, count and serialize bytes, but explicitly wait for both the write callback and drain if socket.write returns false, including synchronous-callback ordering. Remote end closes the transport and pending writes fail (ice:packages/client/src/unix.ts:1-157; ice:packages/client/test/unix.test.ts:94-165).

`connectUnixTestClient(path)` is conformance support: it awaits a socket connection, wraps full-byte sends or two awaited subarray sends around the test channel, and forwards data/error/close to ProtocolTestClient. Its close destroys the socket and awaits close unless already destroyed. This is a source-read helper; no socket was opened during this audit (ice:packages/server/src/testing/client.ts:116-147).

### 5. Interfaces

createUnixTransportFactory({path, maxPendingBytes?})→ByteTransportFactory; ByteTransport.send(Uint8Array)→Promise<void>, close()→void. Server ByteConnection.close(finalChunk?)→Promise<void> and closed getter. Graceful timeout maximum is Node's maximum timer delay; no explicit per-write timeout is imposed on ordinary sends.

Exact transport callback and client Unix options DTOs:

```ts
export interface ByteTransportHandlers {
	/** Delivers an arbitrary inbound byte chunk. */
	onData(chunk: Uint8Array): void;
	/** Reports an orderly terminal close. */
	onClose(): void;
	/** Reports a terminal transport failure. */
	onError(error: Error): void;
}
```
(ice:packages/client/src/transport.ts:8-15).

```ts
export interface UnixTransportOptions {
	path: string;
	maxPendingBytes?: number;
}
```
(ice:packages/client/src/unix.ts:7-10).

### 6. Dependencies

`RPC-001`, `RPC-008`

This entry covers byte queue ownership and backpressure, not request admission, model turn completion or transport protocol parsing.

### 7. ice-rust today

Native RemoteClient serializes JSON-RPC into a String and writes/flushed it to a TCP OwnedWriteHalf, while the headless JSONL server has a real 64-message bounded writer on a blocking task. Those mechanisms protect different paths: neither is a copied-byte Unix transport with this pending-byte budget, terminal callback contract or final-frame close API. Preserve native bounded JSONL output when adding the binary transport (rust:crates/ice-cli/src/remote/client.rs:161-191; rust:crates/ice-cli/src/modes.rs:602-635).

### 8. Port notes

Implement ordered copied-byte queues with explicit byte accounting for the Unix client and server. Separate connection acquisition errors from terminal callbacks, keep close idempotent and ensure pending promises settle on socket closure. Make the server's handling of pre-close queued bytes an explicit design decision; if improving it to drain every admitted chunk, document that correction rather than falsely claiming it is already guaranteed by TS. Keep the final error frame bounded by the protocol encoder and forced-close deadline.

### 9. Size and priority

Estimated Rust implementation: **150-300 lines** (estimate, not measured source). **P1** - Unbounded retained writes or ambiguous close behavior can stall or lose a remote client's final protocol records.

## RPC-010 - Binary client connection generations and deterministic disposal

### 1. Title and status

Binary client connection generations and deterministic disposal. **Status: missing.**

### 2. ice source

ice:packages/client/src/connection.ts:1-237; ice:packages/client/src/client.ts:68-135; ice:packages/client/src/client.ts:321-357; ice:packages/client/test/connection.test.ts:21-325; ice:packages/client/test/disposal.test.ts:6-54.

Exported symbols: `IceDisconnectedError`, `toDisconnectedError` (ice:packages/client/src/errors.ts:15; ice:packages/client/src/errors.ts:53).
Exported symbols: `PromiseResolvers`, `createPromiseResolvers` (ice:packages/client/src/promise.ts:1; ice:packages/client/src/promise.ts:8).
Exported symbols: `ConnectionState`, `ConnectionStateChange`, `ListenerErrorHandler`, `IceClientOptions` (ice:packages/client/src/types.ts:4; ice:packages/client/src/types.ts:6; ice:packages/client/src/types.ts:12; ice:packages/client/src/types.ts:14).

### 3. What it does

Connect through a fresh byte transport, reject stale or invalid handshakes, and ensure disconnect/disposal cannot be undone by late callbacks.

### 4. How it works

Each binary connection attempt has its own identity and decoder. Late transport results or callbacks cannot restore a connection that was disconnected or disposed.

The connection state is disconnected, connecting or connected. maxFrameLength defaults to 16 MiB and must be a positive safe integer within the unsigned 32-bit range. connect rejects concurrent or already-connected attempts rather than coalescing them. Each attempt gets a sequence identity and a new decoder. The factory must supply a fresh connected transport; data delivered before the returned transport is installed and client hello is sent is a protocol error. Late factory results are closed if the attempt was superseded. Factory/send/transport exceptions disconnect only the still-current attempt (ice:packages/client/src/connection.ts:1-151; ice:packages/client/test/connection.test.ts:22-77).

First server record must be hello or hello_error. A typed hello_error rejects connection with IceServerError; a duplicate hello after connection or any other first record fails protocol. On accepted hello, connected lifecycle state is installed before the snapshot callback, then state-change notification occurs, then the connection promise resolves, with identity/state checks between callbacks. A subscriber may disconnect or immediately reconnect without the old continuation restoring its connection. Decoder error and truncated EOF fail the connection; processing of a coalesced batch stops after disconnect (ice:packages/client/src/connection.ts:153-237; ice:packages/client/test/connection.test.ts:126-177; ice:packages/client/test/connection.test.ts:230-258; ice:packages/client/test/connection.test.ts:296-315).

IceClient.connect resets cached state only when starting from disconnected; reconnect is an alias, with no automatic retry/backoff or client-side handshake deadline. Disconnect clears attachments, invalidates lease generations and rejects all pending requests, but leaves cached snapshots until a fresh connect resets them. dispose marks disposed, returns one stable promise, rejects pending work with IceClientDisposedError, disconnects, clears state/listeners and invalidates children. Static connect disposes its newly owned client on initialization failure. Listener and error-observer exceptions are isolated (ice:packages/client/src/client.ts:96-135; ice:packages/client/src/client.ts:321-357; ice:packages/client/src/client.ts:402-431; ice:packages/client/test/disposal.test.ts:27-53).

`IceDisconnectedError` defaults to `Ice client is disconnected` and sets its class name. `toDisconnectedError` first preserves an Error or creates Error(String(value)); an existing IceDisconnectedError is returned unchanged, while another error becomes a new IceDisconnectedError carrying only its message, without attaching the original as cause. `createPromiseResolvers` captures the native Promise executor's resolve/reject functions and returns them with that one promise; it adds no timeout, queue or cancellation. Its handshake consumer is ice:packages/client/src/connection.ts:71 and its request-correlation consumer is ice:packages/client/src/client.ts:193 (ice:packages/client/src/errors.ts:15-20; ice:packages/client/src/errors.ts:49-56; ice:packages/client/src/promise.ts:1-16; ice:packages/client/src/connection.ts:69-73; ice:packages/client/src/client.ts:191-195).

### 5. Interfaces

IceClient.connect(options) factory; instance connect/reconnect→ServerSnapshot, disconnect(reason?), dispose and Symbol.asyncDispose. Public connected/connectionState/snapshot/disposed getters and snapshot/event/connection-state subscriptions. ByteTransportFactory receives onData/onError/onClose and returns a new ByteTransport; the client does not add token authentication to that factory.

Exact connection and resolver-triple DTOs:

```ts
export type ConnectionState = "disconnected" | "connecting" | "connected";

export interface ConnectionStateChange {
	state: ConnectionState;
	error?: Error;
}

export type Unsubscribe = () => void;
export type ListenerErrorHandler = (error: Error) => void;

export interface IceClientOptions {
	transportFactory: ByteTransportFactory;
	maxFrameLength?: number;
	/** Reports subscriber failures without allowing them to corrupt client state. */
	onListenerError?: ListenerErrorHandler;
}
```
(ice:packages/client/src/types.ts:4-19).

```ts
export interface PromiseResolvers<T> {
	promise: Promise<T>;
	resolve(value: T | PromiseLike<T>): void;
	reject(reason?: unknown): void;
}
```
(ice:packages/client/src/promise.ts:1-5).

### 6. Dependencies

`RPC-003`, `RPC-009`, `RPC-011`, `RPC-012`

Validated binary schemas, byte transport and owned child leases must agree on connection lifetime. This connection contract is separate from the legacy child-process RpcClient.

### 7. ice-rust today

Native RemoteClient has a real TCP connector and authenticated method calls, then permanently transfers its single reader into an event pump. RemoteDriver::connect creates one new session and subscription and shutdown sends best-effort stop_session. It has no binary hello/snapshot connection generations, reconnect API, cached attachment invalidation or idempotent ownership factory matching this contract (rust:crates/ice-cli/src/remote/client.rs:18-160; rust:crates/ice-cli/src/session_driver.rs:207-281).

### 8. Port notes

Add a binary client around the native strict codec with an explicit attempt generation and reentrancy-safe state notifications. Close stale factory results, reject pending work on terminal transport state and invalidate all child lease generations. Preserve explicit reconnect semantics and the absence of an implicit request replay. Keep native TCP proxy support as a distinct adapter instead of overwriting its authentication/method protocol.

### 9. Size and priority

Estimated Rust implementation: **220-440 lines** (estimate, not measured source). **P1** - Late handshake and reconnect races can revive stale transports or leave request/session ownership inconsistent.

## RPC-011 - Binary request correlation and authoritative client snapshots

### 1. Title and status

Binary request correlation and authoritative client snapshots. **Status: missing.**

### 2. ice source

ice:packages/client/src/client.ts:137-207; ice:packages/client/src/client.ts:294-340; ice:packages/client/src/state.ts:5-158; ice:packages/client/test/requests.test.ts:4-68; ice:packages/client/test/state.test.ts:4-118.

### 3. What it does

Correlate out-of-order command results and apply only authoritative server/session snapshots without turning progress deltas into durable state.

### 4. How it works

The binary client matches each response to its request and applies authoritative snapshots before reporting success. Progress notifications remain separate from the cached snapshot.

Each request uses a monotonically increasing request-N id, registers pending state before encoding, validates/encodes with the connection frame limit, and then sends. Invalid outgoing data rejects only that request without sending a frame. Transport failure disconnects and rejects all pending requests. There is no request timeout or pending-request-count cap in this binary client. Unknown response id is a fatal protocol error; a mismatched result.command rejects the matched request and fails the connection. Typed server errors reject only the matched request. Successful results are applied to cache before resolving the request promise (ice:packages/client/src/client.ts:189-207; ice:packages/client/src/client.ts:294-340; ice:packages/client/test/requests.test.ts:5-66).

ClientState ignores server or per-session snapshots only when their revision is lower; equal revisions are accepted and notified. Server snapshots replace the global snapshot and rebuild attached-session ids. Session snapshots replace only that id's cache and attached flag. A list result is returned but does not update cache. A detach response forcibly marks any cached session detached without changing its revision. session_removed deletes cache/attachment and invalidates its leases before event delivery. Progress is emitted as an event and never edits cached snapshots (ice:packages/client/src/state.ts:77-115; ice:packages/client/src/client.ts:294-298; ice:packages/client/test/state.test.ts:5-54).

Event reduction occurs before global event listeners and then session-specific event listeners; snapshot listeners run when the snapshot is applied. Subscriptions do not emit an initial snapshot. Listeners are live Sets, exceptions are isolated and reporting exceptions are swallowed. Values are stored and returned by reference, not deep-cloned or frozen at this layer. Reacquisition forgets an old runtime snapshot before attach, allowing revision reset; if attach fails the old snapshot is restored only when a new one has not arrived (ice:packages/client/src/state.ts:49-75; ice:packages/client/src/state.ts:88-158; ice:packages/client/src/client.ts:179-187; ice:packages/client/test/state.test.ts:56-117).

### 5. Interfaces

listSessions returns summaries; create/attach/session command results carry snapshots. subscribe/onEvent and per-session subscriptions return unsubscribe functions. State reducers consume ServerSnapshot, SessionSnapshot and ServerEvent from RPC-003, with no optimistic transcript mutation.

### 6. Dependencies

`RPC-003`, `RPC-010`

RPC-013 owns the separate transient transcript view; that view must not mutate this authoritative cache.

### 7. ice-rust today

Native RemoteClient::call serially ignores unmatched response ids and method notifications while waiting; after starting the event pump, command acknowledgements are not correlated into pending promises. RemoteSession folds native ready/status and agent events into its own mutable transcript/status, with no versioned ServerSnapshot/SessionSnapshot caches. These are genuine native behaviors but do not supply this request map or authoritative revision reducer (rust:crates/ice-cli/src/remote/client.rs:72-159; rust:crates/ice-cli/src/remote/session.rs:88-255).

### 8. Port notes

Implement typed pending requests keyed by id and fail the connection on unmatched or command-mismatched binary responses. Apply result snapshots before resolving, enforce lower-revision rejection and handle reacquired-runtime revision resets. Keep progress in a separate projection and preserve subscription ordering/error isolation. Choose any stronger immutability policy explicitly rather than claiming TS currently deep-clones this cache.

### 9. Size and priority

Estimated Rust implementation: **160-330 lines** (estimate, not measured source). **P1** - Incorrect correlation or stale snapshot application can report success for the wrong command or roll client state backward.

## RPC-012 - Client shared/exclusive session leases and failed-cleanup reconciliation

### 1. Title and status

Client shared/exclusive session leases and failed-cleanup reconciliation. **Status: missing.**

### 2. ice source

ice:packages/client/src/client.ts:141-187; ice:packages/client/src/client.ts:209-292; ice:packages/client/src/client.ts:363-412; ice:packages/client/src/session-handle.ts:11-111; ice:packages/client/test/sessions.test.ts:5-261; ice:packages/coding-agent/test/client/remote-session-ownership.test.ts:18-173.

Exported symbols: `SessionLeaseMode`, `AcquireSessionOptions`, `SessionHandleCallbacks`, `SessionHandle` (ice:packages/client/src/session-handle.ts:13; ice:packages/client/src/session-handle.ts:15; ice:packages/client/src/session-handle.ts:37; ice:packages/client/src/session-handle.ts:47).
Exported symbols: `CreateSessionOptions` (ice:packages/client/src/types.ts:21).

### 3. What it does

Give borrowed consumers independent session handles without detaching a shared server session too early or retaining exclusive ownership after failed disposal.

### 4. How it works

Consumers borrow independent session handles while the client tracks shared or exclusive ownership. Releasing one handle cannot detach other owners, and failed cleanup must be reconciled before reacquisition.

attachSession defaults to shared; acquireSession requires an explicit shared/exclusive mode, and createSession reserves exclusive ownership after successful server creation. Exclusive acquisition fails if any local lease exists; shared fails while an exclusive token exists. Counts/tokens are reserved before asynchronous attachment. Concurrent acquisitions share one attach promise and wait any final detachment; attach failure releases the reservation. A previously failed disposal first triggers one coalesced detach reconciliation, clears cleanup-required only after success, then attaches again (ice:packages/client/src/client.ts:141-187; ice:packages/client/src/client.ts:363-400).

Each handle captures its session generation and moves among active, releasing, released and invalidated. Active also requires the authoritative attached id. Every request checks client disposal, connection and active ownership. Releasing a nonfinal shared handle decrements locally without a protocol request; final release sends detach and holds a detachment promise so reacquisition cannot overtake it. Concurrent releases coalesce internally. Explicit detach failure restores active and permits retry. dispose failure instead relinquishes the local count/exclusive token, marks released and cleanup-required, and still rejects; a subsequent acquirer must reconcile server detachment. If the generation was invalidated while release failed, that failure is suppressed because ownership is already gone (ice:packages/client/src/client.ts:209-292; ice:packages/client/test/sessions.test.ts:40-225).

Disconnect and session_removed advance generation and clear counts/exclusive/cleanup state. Invalidated or released handles dispose successfully without another wire detach. Handle snapshot is available only while active; subscriptions require active ownership and forward only while active, except session_removed is deliberately delivered. Different session ids remain independent. A reacquired runtime may restart revision at zero, so old snapshot history is removed before the new attach (ice:packages/client/src/client.ts:269-289; ice:packages/client/src/client.ts:402-412; ice:packages/client/test/sessions.test.ts:116-134; ice:packages/client/test/sessions.test.ts:227-259).

### 5. Interfaces

SessionLease exposes id, active/attached, snapshot, subscribe/onEvent, detach/dispose/asyncDispose, prompt/steer/abort/setModel/setThinking. IceSessionHandle is an alias. Commands return the server's SessionSnapshot. IceSessionOwnershipError and IceSessionDetachedError identify local ownership violations.

Client `CreateSessionOptions` has no id; the server assigns it. This same-named type is distinct from the backend's required-id input in RPC-007. Named lease mode/options and callback interface retain the ownership behavior above:

```ts
export interface CreateSessionOptions {
	cwd?: string;
	name?: string;
	model?: ModelRef;
	thinkingLevel?: ThinkingLevel;
}
```
(ice:packages/client/src/types.ts:21-26).

```ts
export type SessionLeaseMode = "shared" | "exclusive";

export interface AcquireSessionOptions {
	mode: SessionLeaseMode;
}
```
(ice:packages/client/src/session-handle.ts:13-17).

```ts
export interface SessionHandleCallbacks {
	isAttached(): boolean;
	getSnapshot(): SessionSnapshot | undefined;
	subscribe(listener: (snapshot: SessionSnapshot) => void): Unsubscribe;
	onEvent(listener: (event: ServerEvent) => void): Unsubscribe;
	detach(): Promise<void>;
	dispose(): Promise<void>;
	request<const TCommand extends SessionCommand>(command: TCommand): Promise<ResultForCommand<TCommand>>;
}
```
(ice:packages/client/src/session-handle.ts:37-45).

### 6. Dependencies

`RPC-010`, `RPC-011`

These are local client ownership leases, separate from storage locks and the server's shared runtime acquisition. The RemoteSession coordinator uses an exclusive client lease in RPC-014.

### 7. ice-rust today

Native RemoteDriver owns one remote session and RemoteClient has no per-session local reference count, shared/exclusive lease API, generation invalidation or cleanup reconciliation. Native storage locks are not this borrowed-client ownership layer. stop_session best-effort teardown cannot replace a final shared detach or retryable failed detach (rust:crates/ice-cli/src/session_driver.rs:207-311; rust:crates/ice-cli/src/remote/client.rs:135-174).

### 8. Port notes

Add owned session-handle tokens and per-id counts/generations around the binary client. Make detach retryable while disposal relinquishes ownership even on error and records required reconciliation. Serialize reacquisition behind final release and coalesce cleanup/attach attempts. Preserve borrowed-client independence so disposing one RemoteSession never disconnects unrelated handles.

### 9. Size and priority

Estimated Rust implementation: **180-360 lines** (estimate, not measured source). **P1** - This prevents premature detachment, leaked exclusive reservations and stale handles acting after reconnect.

## RPC-013 - Transient transcript projection with authoritative snapshot reset

### 1. Title and status

Transient transcript projection with authoritative snapshot reset. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/client/transcript.ts:1-102; ice:packages/coding-agent/test/client/transcript.test.ts:37-220; rust:crates/ice-cli/src/remote/session.rs:1-493.

### 3. What it does

Render streaming progress over a stable snapshot while replacing transient items by id and preserving incomplete tool argument fragments.

### 4. How it works

The remote transcript view combines an authoritative snapshot with transient streaming changes. A new accepted snapshot resets those changes, while incomplete tool arguments remain visible until valid JSON arrives.

createTranscriptState structured-clones the snapshot and starts empty progress order/map and tool-argument buffers. applyTranscriptSnapshot ignores only a lower revision for the same session id; any accepted snapshot, including equal revision or a different session with lower revision, resets transient progress completely. Reacquisition starts a new state to permit same-id runtime revision reset (ice:packages/coding-agent/src/client/transcript.ts:1-31; ice:packages/coding-agent/test/client/transcript.test.ts:167-219).

item_started, item_updated and item_finished clone the item into an id-keyed progress map, appending that id to display order only the first time. Finished items clear argument buffers for that item's id prefix. assistant_delta finds the current progress item or snapshot item by messageId and ignores unknown/non-assistant messages. It clones content and appends text/thinking only to a matching block type. ToolCall deltas append to a buffer keyed by messageId/contentIndex; initial partial string input can seed the buffer. A complete parse becomes input only if recursively finite JSON with ordinary objects; incomplete/invalid JSON remains the raw accumulated string. item_updated does not discard the separate partial buffer (ice:packages/coding-agent/src/client/transcript.ts:33-86; ice:packages/coding-agent/test/client/transcript.test.ts:52-120).

selectTranscript walks authoritative snapshot order, replacing matching ids with transient items; appends unseen transient ids in first-seen order; then appends queued steering user items not already displayed. It returns these selected references without a final clone. Progress never mutates the source snapshot. Content-index or block-kind mismatches do not invent missing blocks (ice:packages/coding-agent/src/client/transcript.ts:88-102; ice:packages/coding-agent/test/client/transcript.test.ts:38-50; ice:packages/coding-agent/test/client/transcript.test.ts:122-201).

### 5. Interfaces

TranscriptState holds snapshot plus transient item and tool-input buffers. createTranscriptState/applyTranscriptSnapshot/applyTranscriptProgress/selectTranscript consume binary SessionSnapshot and TranscriptProgress. This is display projection, not durable session persistence.

### 6. Dependencies

`RPC-003`, `RPC-011`

The binary schema provides stable item ids/status/content. SESSION persistence remains the authority behind snapshots; this reducer does not save events.

### 7. ice-rust today

Rust RemoteSession is a genuine native JSONL fold: it appends text/thinking/tool transcript variants, updates open blocks from deltas/final messages and keeps status/retry state. It does not retain a binary authoritative snapshot plus transient id replacement map, queuedSteer projection or incrementally parsed tool-call input. The shared TUI driver also projects full native assistant/tool events, a separate presentation path (rust:crates/ice-cli/src/remote/session.rs:1-255; rust:crates/ice-cli/src/session_driver.rs:354-431).

### 8. Port notes

Implement a binary transcript reducer using cloned snapshots, stable ids and transient overlays. Preserve incomplete tool input as text until valid JSON, maintain buffers across item updates and reset them on finished/snapshot events. Keep snapshot revision and runtime identity semantics separate so reconnect/reacquisition does not suppress fresh low-revision state.

### 9. Size and priority

Estimated Rust implementation: **110-230 lines** (estimate, not measured source). **P1** - A remote UI otherwise duplicates messages, loses streamed tool arguments or confuses transient deltas with authoritative history.

## RPC-014 - Borrowed-client RemoteSession operations, replacement rollback and disposal

### 1. Title and status

Borrowed-client RemoteSession operations, replacement rollback and disposal. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/client/remote-session.ts:26-420; ice:packages/coding-agent/test/client/remote-session-lifecycle.test.ts:15-190; ice:packages/coding-agent/test/client/remote-session-ownership.test.ts:18-173; ice:packages/coding-agent/test/client/remote-session.test.ts:4-196.

Exported symbols: `RemoteSessionOperation`, `RemoteSessionLifecycle`, `RemoteSessionState`, `CreateRemoteSessionOptions`, `RemoteSessionOptions` (ice:packages/coding-agent/src/client/remote-session.ts:26; ice:packages/coding-agent/src/client/remote-session.ts:28; ice:packages/coding-agent/src/client/remote-session.ts:34; ice:packages/coding-agent/src/client/remote-session.ts:40; ice:packages/coding-agent/src/client/remote-session.ts:46).

### 3. What it does

Offer a single-session coordinator over a borrowed binary client, with explicit operation state, safe session replacement and cleanup that survives disposal races.

### 4. How it works

RemoteSession coordinates one selected session while borrowing a client that may serve other consumers. Replacement and disposal preserve ownership across asynchronous work and cleanup failures.

Lifecycle is unbound, ready, busy with operation open/create/submit/abort/setModel/setThinking/reconnect, or disposed. The constructor is private; static open/create allocate a coordinator and dispose it on initialization failure. open uses an exclusive lease and returns early for the same id already ready. create requests a new exclusive session. The coordinator borrows its IceClient; models, sessions and connection state come from that client, and coordinator disposal must not disconnect it. subscribe emits the current coordinator state immediately and then live updates, isolating listener/diagnostic failures (ice:packages/coding-agent/src/client/remote-session.ts:26-173; ice:packages/coding-agent/src/client/remote-session.ts:366-386).

submit trims text and returns immediately for empty input before availability checks. Otherwise it rejects local busy state, requires a handle, accepts only server phase idle/turn, and sends prompt when idle or steer during turn. setModel/setThinking require idle. abort may preempt a locally pending submit; a normal idle abort is a no-op, while preemption still sends abort even if the last snapshot is idle. Each operation publishes a unique busy state. A preempting abort restores the previous submit state only if that operation is still active; otherwise it becomes ready/unbound. Promise completion never resurrects disposed lifecycle (ice:packages/coding-agent/src/client/remote-session.ts:175-224; ice:packages/coding-agent/src/client/remote-session.ts:295-336; ice:packages/coding-agent/test/client/remote-session.test.ts:76-177).

Replacement requires current idle phase before starting, prepares the new lease before detaching the old one, requires a snapshot, and rechecks the old session's phase after the asynchronous acquire. If the old session became busy, the new lease is disposed and the old binding remains. Old-detach failure also disposes the new lease, combining both errors in AggregateError if cleanup fails. Only after old detach and a final disposal check does it bind the replacement. reconnect waits client.reconnect, reacquires the same id exclusively and binds a fresh transcript state (ice:packages/coding-agent/src/client/remote-session.ts:213-293; ice:packages/coding-agent/test/client/remote-session-lifecycle.test.ts:16-107).

Disposal marks disposed and resolves an internal signal immediately, clears subscriptions/handle/transcript, then waits all tracked attachment preparations plus disposal of the current lease. In-flight operations race the disposal signal, so callers reject promptly while late attachments are still awaited and disposed. Multiple cleanup failures aggregate; the special internal after-await disposed marker is ignored after its own lease cleanup. One stable dispose promise is returned, disposed state is notified, and listeners are cleared. session_removed unbinds and clears transcript, retaining busy lifecycle until the current operation finally unwinds. Ordinary disconnect leaves the coordinator's last transcript available; explicit reconnect replaces it (ice:packages/coding-agent/src/client/remote-session.ts:226-264; ice:packages/coding-agent/src/client/remote-session.ts:338-364; ice:packages/coding-agent/src/client/remote-session.ts:407-419; ice:packages/coding-agent/test/client/remote-session-lifecycle.test.ts:109-188).

### 5. Interfaces

RemoteSession.open(client, id, options?), create(client,{cwd, model?, thinkingLevel?}, options?), instance open/create/submit/abort/setModel/setThinking/reconnect/dispose/asyncDispose. State contains lifecycle, optional snapshot and projected transcript. Public getters id/phase/operation/models/sessions/connectionState/disposed and onConnectionStateChange do not confer ownership of the borrowed client.

### 6. Dependencies

`RPC-010`, `RPC-011`, `RPC-012`, `RPC-013`

The coordinator composes exclusive local leases and transient transcript projection; it is not the older child-process RpcClient or Rust's same-named event fold.

### 7. ice-rust today

Rust's RemoteSession is a data fold rather than this operation-owning SDK. RemoteDriver connects to the TCP proxy, starts a session, streams normalized TUI events and sends commands, but has no open/create replacement rollback, borrowed client lease, preempting operation-state reducer or asynchronous disposal reconciliation. Similar names therefore do not establish counterpart equivalence (rust:crates/ice-cli/src/remote/session.rs:88-255; rust:crates/ice-cli/src/session_driver.rs:207-311).

### 8. Port notes

Build a distinct binary RemoteSession coordinator using the lease and transcript layers. Preserve phase checks both before and after asynchronous replacement, acquire-before-release rollback and aggregate cleanup failures. Race visible operation settlement against disposal while still waiting for late attachment cleanup; leave the shared client connected. Keep native event folding and the new coordinator conceptually separate in module/API naming.

### 9. Size and priority

Estimated Rust implementation: **240-480 lines** (estimate, not measured source). **P1** - This SDK contract prevents replacing an active session, leaking a late attachment or disconnecting other users of a shared client.

## RPC-015 - Experimental remote command parsing and native connect-mode boundary

### 1. Title and status

Experimental remote command parsing and native connect-mode boundary. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/cli/experimental/command.ts:1-205; ice:packages/coding-agent/src/cli/experimental/command-options.ts:6-37; ice:packages/coding-agent/src/cli/experimental/auth.ts:1-20; ice:packages/coding-agent/src/cli/experimental/transport-address.ts:10-47; ice:packages/coding-agent/src/cli/experimental/commands/ice.ts:13-47; ice:packages/coding-agent/src/cli/experimental/commands/server.ts:13-44; ice:packages/coding-agent/src/cli/experimental/commands/client.ts:13-44; ice:packages/coding-agent/test/experimental-cli-command.test.ts:1-164; ice:packages/coding-agent/test/experimental-cli-resolution.test.ts:1-97.

Exported symbols: `RawAuthOptions`, `parseAuthInput` (ice:packages/coding-agent/src/cli/experimental/auth.ts:5; ice:packages/coding-agent/src/cli/experimental/auth.ts:10).
Exported symbols: `authTokenOption`, `authTokenFileOption`, `transportOption`, `parseAuth`, `parseLegacyOptions`, `unsupportedLegacyOptions` (ice:packages/coding-agent/src/cli/experimental/command-options.ts:6; ice:packages/coding-agent/src/cli/experimental/command-options.ts:7; ice:packages/coding-agent/src/cli/experimental/command-options.ts:9; ice:packages/coding-agent/src/cli/experimental/command-options.ts:18; ice:packages/coding-agent/src/cli/experimental/command-options.ts:25; ice:packages/coding-agent/src/cli/experimental/command-options.ts:35).
Exported symbols: `CommandParseResult`, `CommandExecutionResult`, `CommandOptionParseResult`, `CommandOption`, `valueOption`, `stringOption`, `ParsedCommandInput`, `CommandBuildResult` (ice:packages/coding-agent/src/cli/experimental/command.ts:5; ice:packages/coding-agent/src/cli/experimental/command.ts:9; ice:packages/coding-agent/src/cli/experimental/command.ts:13; ice:packages/coding-agent/src/cli/experimental/command.ts:17; ice:packages/coding-agent/src/cli/experimental/command.ts:22; ice:packages/coding-agent/src/cli/experimental/command.ts:29; ice:packages/coding-agent/src/cli/experimental/command.ts:33; ice:packages/coding-agent/src/cli/experimental/command.ts:39).
Exported symbols: `ClientCommand`, `ClientCommandContext`, `clientCommand` (ice:packages/coding-agent/src/cli/experimental/commands/client.ts:13; ice:packages/coding-agent/src/cli/experimental/commands/client.ts:19; ice:packages/coding-agent/src/cli/experimental/commands/client.ts:25).
Exported symbols: `IceCommand`, `IceCommandContext`, `iceCommand` (ice:packages/coding-agent/src/cli/experimental/commands/ice.ts:13; ice:packages/coding-agent/src/cli/experimental/commands/ice.ts:20; ice:packages/coding-agent/src/cli/experimental/commands/ice.ts:26).
Exported symbols: `ServerCommand`, `ServerCommandContext`, `serverCommand` (ice:packages/coding-agent/src/cli/experimental/commands/server.ts:13; ice:packages/coding-agent/src/cli/experimental/commands/server.ts:19; ice:packages/coding-agent/src/cli/experimental/commands/server.ts:25).
Exported symbols: `UnixTransportAddress`, `parseTransportAddress` (ice:packages/coding-agent/src/cli/experimental/transport-address.ts:3; ice:packages/coding-agent/src/cli/experimental/transport-address.ts:10).

### 3. What it does

Preserve the new, explicitly experimental Unix-address command parser without confusing its injected actions with an already wired production server, or with Rust's TCP --connect interception.

### 4. How it works

The experimental parser separates its Unix transport options from legacy command arguments. Its injected actions are a parsing and composition API whose production launch wiring must be established separately.

Command dispatch selects a registered subcommand only from argv[0]. Its own parser consumes registered value options until the first unknown token or literal --, preserving that token and every later argument for legacy parsing. Both --name=value and a separate next value are supported; a separate value beginning with '-' is not consumed, and an absent/empty value is an error. A second successfully parsed occurrence of any option is rejected, including --listen even though its invocation type is an array. Builders still run after option errors, and parse errors precede builder errors in the returned list. Duplicate option or subcommand registration throws; missing builder/action throws; action exceptions propagate rather than becoming parse errors (ice:packages/coding-agent/src/cli/experimental/command.ts:78-203).

The root ice command accepts --listen, --auth-token and --auth-token-file before its legacy tail, preserves parsed legacy options and explicitly rejects a legacy unknown flag named connect. server/client accept the same auth alternatives and --listen/--connect respectively, but reject every nonempty legacy tail after collecting its parser errors. Omitted addresses/auth remain absent; no default socket is selected here. Token and token-file inputs are mutually exclusive, are carried verbatim as tagged input values, and the parser neither reads an auth file nor authenticates a peer. Execute calls only the context's runIce/runServer/runClient action (ice:packages/coding-agent/src/cli/experimental/auth.ts:10-20; ice:packages/coding-agent/src/cli/experimental/command-options.ts:25-37; ice:packages/coding-agent/src/cli/experimental/commands/ice.ts:26-47; ice:packages/coding-agent/src/cli/experimental/commands/server.ts:25-44; ice:packages/coding-agent/src/cli/experimental/commands/client.ts:25-44).

Address parsing accepts only a canonical URL beginning exactly unix:/// and not unix:////, without authority, credentials, port, query or fragment. It requires url.href to equal the input, percent-decodes pathname, rejects decode errors and NUL, and requires a POSIX absolute result. This is a parser contract, not a statement that every accepted path is a bindable socket. The experimental CLI composes three commands, while the core experimental module exposes opt-in experimental APIs. Source searches of the frozen coding-agent production tree found definitions/actions and tests but no main-path invocation of experimentalCli/runServer/runClient; no completed TS standalone server launch is inferred from these modules (ice:packages/coding-agent/src/cli/experimental/transport-address.ts:10-47; ice:packages/coding-agent/src/cli/experimental/cli.ts:1-7; ice:packages/coding-agent/src/core/experimental.ts:1-6).

`valueOption(name,parse)` returns that name/parser pair; `stringOption` supplies an always-successful parser preserving the input string. The auth option constants use `--auth-token` and `--auth-token-file`. `UnixTransportAddress` is `{readonly transport:'unix'; readonly path:string}` and TransportAddress aliases it; no additional transport variant is hidden by the union alias (ice:packages/coding-agent/src/cli/experimental/command.ts:22-31; ice:packages/coding-agent/src/cli/experimental/command-options.ts:6-7; ice:packages/coding-agent/src/cli/experimental/transport-address.ts:3-8).

### 5. Interfaces

NamedCommandInvocation; Command.parse/execute; {ok:true, command}|{ok:false, errors}; AuthInput token/file; TransportAddress {transport:'unix', path}; injected ExperimentalCliContext. CLI option values and parser diagnostic order are observable.

### 6. Dependencies

`RPC-005`, `RPC-008`, `RPC-010`

Binary server/client and Unix transport implementations are separate from this parser. The production CLI owner must decide and document launch wiring; this entry does not invent an existing TS caller.

### 7. ice-rust today

Native app scans all argv for the first --connect value or --connect=value before local startup. Its helper does not stop at -- or reject a next value beginning '-'. Explicit --token wins verbatim; otherwise ICERS_SERVER_TOKEN is trimmed and empty input ignored. It starts a TCP JSON-RPC proxy RemoteDriver, not a Unix binary client. One rust:crates/ice-cli/src/app.rs:96 expression was connector-redacted and remains unverified. Native RunMode::Rpc uses the built-in JSONL loop unless unknown_flags contains server or stdio was claimed, in which case extension I/O owns the run and the process waits for its lifecycle/interrupt. That conditional path cannot justify saying all native RPC is unimplemented (rust:crates/ice-cli/src/app.rs:43-65; rust:crates/ice-cli/src/app.rs:95; rust:crates/ice-cli/src/app.rs:97-98; rust:crates/ice-cli/src/app.rs:434-505; rust:crates/ice-cli/src/session_driver.rs:207-264).

### 8. Port notes

Add explicit, separately named experimental parser/entry integration only where intended. Reproduce stop-at-first-unknown parsing, diagnostics, mutual exclusion and canonical Unix URL checks; do not treat an auth-file descriptor as a loaded credential. Keep existing TCP --connect behavior identifiable and preserve extension-owned stdio dispatch. Supply an actual production caller before advertising a runnable binary server/client command.

### 9. Size and priority

Estimated Rust implementation: **150-300 lines** (estimate, not measured source). **P1** - Transport/address and entry-point ambiguity can direct clients to incompatible protocols or overstate readiness.

## RPC-016 - JSONL framing, command inventory and pre-dispatch validation

### 1. Title and status

JSONL framing, command inventory and pre-dispatch validation. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/modes/rpc/jsonl.ts:1-93; ice:packages/coding-agent/src/modes/rpc/rpc-validation.ts:1-244; ice:packages/coding-agent/src/modes/rpc/rpc-types.ts:39-185; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:2110-2134; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:2211-2235; ice:packages/coding-agent/test/rpc-jsonl.test.ts:6-88; ice:packages/coding-agent/test/rpc-validation.test.ts:4-62.

Exported symbols: `JsonlReaderOptions`, `attachJsonlLineReader` (ice:packages/coding-agent/src/modes/rpc/jsonl.ts:21; ice:packages/coding-agent/src/modes/rpc/jsonl.ts:28).
Exported symbols: `RpcCommand` (ice:packages/coding-agent/src/modes/rpc/rpc-types.ts:39).
Exported symbols: `RPC_MAX_INPUT_LINE_BYTES`, `RPC_COMMAND_TYPES`, `RpcInputValidation`, `validateRpcInput` (ice:packages/coding-agent/src/modes/rpc/rpc-validation.ts:4; ice:packages/coding-agent/src/modes/rpc/rpc-validation.ts:198; ice:packages/coding-agent/src/modes/rpc/rpc-validation.ts:200; ice:packages/coding-agent/src/modes/rpc/rpc-validation.ts:210).

### 3. What it does

Offer the frozen 79-command JSONL wire contract with bounded LF framing and command-specific validation before side effects; preserve native JSONL as a real but incompatible smaller implementation.

### 4. How it works

The coding-agent transport reads bounded JSON records and validates each command before dispatch. Its 79-command contract differs from the native loop's smaller command set and response envelope.

serializeJsonLine is JSON.stringify plus LF. The reader uses a UTF-8 StringDecoder, splits only on LF, removes one trailing CR from a delivered record, preserves U+2028/U+2029, emits a final unterminated record on EOF, and detaches both data/end listeners on cleanup. Its generic default limit is Infinity; finite limits must be positive safe integers. It counts UTF-8 bytes excluding LF, including a CR, across chunks; overflow notifies once and discards through the next LF before accepting more records. Small fragments are coalesced after 8192 UTF-16 code units. RPC configures 16 MiB and installs the reader's EOF handler before shutdown's EOF handler so a final record is admitted first (ice:packages/coding-agent/src/modes/rpc/jsonl.ts:1-93; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:2211-2235).

The input validator requires a non-null, non-array object, a string type and optional string id; the empty id is valid. Unknown extra properties are generally accepted, not stripped, and object checks do not require a plain prototype. Command lookup uses an own-property check. Invalid envelope/id/type uses invalid_request, an unknown string command uses unknown_command while preserving a valid id and command, and a malformed known command uses invalid_argument. JSON parse failure and line overflow produce command=parse with no id and generic text; raw malformed input is not echoed. extension_ui_response is validated separately and bypasses ordinary command dispatch without a response acknowledgment (ice:packages/coding-agent/src/modes/rpc/rpc-validation.ts:1-244; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:2110-2134).

The command union has 79 commands. Common field strings are type-checked but may be empty unless bounded explicitly. Images require type=image plus string data and mimeType, without base64/MIME semantic validation. Page limits are safe integers 1–1000; shell sinceChars is nonnegative safe integer. Thinking includes off/minimal/low/medium/high/xhigh/max/ultra; queue mode is all/one-at-a-time; direction forward/backward; scopes and action enums are exact. Prompt attachments allow at most 16 items: nonempty file path of at most 4096 UTF-16 units, or inline text/image with name at most 256 units; inline text is limited to 1 MiB UTF-8. Request deadlines 1–300000 apply only to compact/navigate_tree/bash; other commands reject timeoutMs except refresh_models, whose separate range is 1–30000. get_completions prefix and session query cap at 256 units, shortcut is nonempty and at most 64, get_messages cursor at most 1024. Other cursor strings have no special length cap beyond the whole record. Structured arguments/options must be objects and optional scope global/project; unknown option members are accepted. set_setting delegates its field validation to the settings bridge. Dialog replies need string id plus at least one valid value:string, confirmed:boolean or cancelled:true; multiple valid forms can coexist, so consuming method precedence matters (ice:packages/coding-agent/src/modes/rpc/rpc-validation.ts:28-244).

### 5. Interfaces

JSONL command {type, id?,...}; response {type:'response', command, id?, success, data?}|{..., success:false, error, errorCode?, errorDetails?}. JSONL protocolVersion 1 is discovered via get_capabilities and is distinct from binary protocol version 1. The union and validator map are compile-time exhaustive.

`JsonlReaderOptions` contains optional maxLineBytes and onOverflow. `RpcInputValidation` is `{valid:true,command:RpcCommand|RpcExtensionUIResponse}` or `{valid:false,id?:string,command:string,error:string,errorCode:'invalid_request'|'invalid_argument'|'unknown_command'}`. `RPC_COMMAND_TYPES` is Object.keys(commandFields), not a separately maintained command inventory (ice:packages/coding-agent/src/modes/rpc/jsonl.ts:21-26; ice:packages/coding-agent/src/modes/rpc/rpc-validation.ts:198-208).

### 6. Dependencies

`RPC-017`, `RPC-018`, `RPC-019`

Transport admission, output limits and request lifetimes are independent bounds. Per-command handlers below supply semantic/resource checks after this structural validation.

### 7. ice-rust today

Native rpc_with_io reads stdin with lines(), skips trimmed blank lines, accepts arbitrary JSON id/default null, and dispatches ui_dialog_response, stop, prompt, abort, get_state, set_model, set_thinking_level, set_active_tools and ping. It has no equivalent input byte ceiling, command schema map, 79-command inventory or generic typed errorCode envelope. Native RemoteResponse uses status/result/error instead of success/data, and invalid JSON is reported with serde's error text (rust:crates/ice-cli/src/modes.rs:403-571; rust:crates/ice-cli/src/remote/protocol.rs:318-362).

### 8. Port notes

Place a compatibility JSONL boundary around native session operations rather than substituting the binary codec or TCP proxy. Implement LF/UTF-8 limits and resynchronization first, then an exhaustive command validator preserving the exact optional/empty/enum rules and error identities. Keep protocol families independently versioned and document native-only stop/ping/ui_dialog_response behavior.

### 9. Size and priority

Estimated Rust implementation: **350-650 lines** (estimate, not measured source). **P1** - Clients require compatible envelopes and deterministic rejection before mutation; unbounded or ambiguous input cannot provide that contract.

## RPC-017 - Bounded JSONL output, stream projection and sink-failure boundary

### 1. Title and status

Bounded JSONL output, stream projection and sink-failure boundary. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/output-guard.ts:1-148; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:215-270; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:659-669; ice:packages/coding-agent/src/modes/json-event.ts:1-40; ice:packages/coding-agent/src/modes/rpc/rpc-command-lanes.ts:17-21. rust:crates/ice-cli/src/remote/protocol.rs:147-212; rust:crates/ice-agent/src/message.rs:25-31; rust:crates/ice-agent/src/message.rs:82-90; rust:crates/ice-ai/src/types.rs:390-414; rust:crates/ice-ai/src/types.rs:654-679.

Exported symbols: `RPC_MAX_OUTPUT_LINE_BYTES` (ice:packages/coding-agent/src/modes/rpc/rpc-command-lanes.ts:19).

### 3. What it does

Serialize protocol output in order, reserve queued/in-flight bytes, forward session backpressure, and preserve the difference between recoverable oversized responses and fatal transport failures.

### 4. How it works

Protocol output reserves bytes before joining an ordered write queue. Oversized individual responses, exhausted queue capacity and failed output sinks have different recovery and shutdown behavior.

takeOverStdout is idempotent: it captures original stdout/stderr writers and redirects ordinary process.stdout.write to stderr while protocol writes use the captured stdout. The redirection converts chunks through String and passes callback but does not preserve a requested encoding. A global promise tail serializes raw writes. Empty writeRawStdout input is a no-op. With the RPC ceiling of 32 MiB, UTF-8 bytes are reserved synchronously before scheduling, count queued and in-flight data, and are released in finally. A single record exceeding the limit throws output_too_large; aggregate exhaustion throws output_overflow. Outside RPC the limit is opt-in/unlimited; restore disables the limit without resetting reservations still owned by pending tails (ice:packages/coding-agent/src/core/output-guard.ts:29-37; ice:packages/coding-agent/src/core/output-guard.ts:72-132; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:215-218).

Each raw write waits its callback and retries the whole text indefinitely after 10 ms only for ENOBUFS/EAGAIN/EWOULDBLOCK. Other errors reject the tail. waitForRawStdoutBackpressure loops until the tail it awaited is still current; flush also performs a final empty underlying write. RPC subscribes an async agent listener that awaits this backpressure, and command handling waits after an emitted response before releasing admitted input (ice:packages/coding-agent/src/core/output-guard.ts:47-69; ice:packages/coding-agent/src/core/output-guard.ts:134-147; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:659-669; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:2163-2171).

RPC observes extension UI state before serialization. If a response alone is oversized, it attempts a small output_too_large failure retaining string id/command and explicitly saying prior effects are not rolled back. An oversized event, aggregate overflow or failure admitting that replacement marks transportFailed, writes a generic stderr diagnostic and begins cooperative shutdown; later output is suppressed. This wrapper catches synchronous serialization/admission failures. An asynchronous non-retryable raw sink failure instead hits output-guard's own process.exit(1), so source does not guarantee admitted-handler settlement for every sink failure. Preserve that material boundary rather than claiming all fatal output errors go through the host shutdown path (ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:239-270; ice:packages/coding-agent/src/core/output-guard.ts:124-131).

For message_update, toJsonEvent emits only type and assistantMessageEvent, removing its cumulative partial field when present; all other event objects pass through unchanged. Clients reconstruct with message_start and deltas and replace with authoritative message_end. This removes cumulative snapshots, not a general redaction or exhaustive event sanitizer (ice:packages/coding-agent/src/modes/json-event.ts:21-40).

### 5. Interfaces

configureRawStdoutLimit(maxBytes?: nonnegative safe integer); writeRawStdout(text); waitForRawStdoutBackpressure; flushRawStdout; RawStdoutLimitError codes output_too_large/output_overflow. JSONL output ceiling 33554432 bytes includes terminating LF.

### 6. Dependencies

`RPC-016`, `RPC-020`, `SESSION-022`, `TOOLS-031`

Session event semantics remain SESSION-022. This entry owns their JSONL projection and the adapter's byte/ordering contract, not binary transcript encoding.

### 7. ice-rust today

Native start_rpc_writer uses a bounded 64-message channel and a blocking writer worker, not a byte budget. It performs write_all then flush per line, cancels the writer-failure token on error, and rpc_with_io selects that token while reading commands. The agent pump also honors a flush-request channel so final agent events drain before a prompt completion response. These are real native ordering/responsiveness capabilities to retain, but a message count cannot bound one huge JSON value, and broadcast Lagged events are skipped (rust:crates/ice-cli/src/modes.rs:319-353; rust:crates/ice-cli/src/modes.rs:527-538; rust:crates/ice-cli/src/modes.rs:596-649; rust:crates/ice-cli/src/modes.rs:1188-1268).

A live wire mismatch is confirmed from the serialization chain, not only enum inspection: rust:crates/ice-cli/src/modes.rs:345 passes agent_event_json to the writer; :775-779 serializes RemoteEvent::from_agent_event. That projection directly serializes AgentMessage for agent_end, turn_end, message_start and message_end, while message_update uses assistant_json (rust:crates/ice-cli/src/remote/protocol.rs:151-179; rust:crates/ice-cli/src/remote/protocol.rs:203-208). AgentMessage derives serde and assistant_json serializes a borrowed AssistantMessage, without stop-reason conversion (rust:crates/ice-agent/src/message.rs:25-31; rust:crates/ice-agent/src/message.rs:82-90). AssistantMessage.stop_reason is an ordinary StopReason field under camelCase; StopReason's lowercase serde maps ToolUse to tooluse although as_str returns toolUse. Consequently actual native JSONL message payloads carry stopReason=tooluse, differing from TS toolUse; shared Rust producer/consumer serde can still agree with each other. The helper test only compares the two native encodings and does not assert this external token (rust:crates/ice-ai/src/types.rs:390-414; rust:crates/ice-ai/src/types.rs:654-679; rust:crates/ice-agent/src/message.rs:178-190; ice:packages/coding-agent/src/modes/rpc/rpc-response-validation.ts:92).

### 8. Port notes

Use native ordered writer infrastructure with explicit UTF-8 reservations for queued and in-flight records. Preserve native final-event-before-completion guarantees and add the compatibility event projection. Distinguish per-response size recovery from aggregate/fatal transport shutdown; settle work where feasible and document any immediate sink-failure exit rather than promising rollback or replay. Normalize the external ToolUse spelling explicitly in the compatibility boundary. If changing shared enum serde instead, preserve readability of existing native serialized records through aliases/migration; do not assume changing as_str alone changes serde.

### 9. Size and priority

Estimated Rust implementation: **180-340 lines** (estimate, not measured source). **P1** - Output pressure can otherwise exhaust memory, corrupt client state or cause misleading completion acknowledgments.

## RPC-018 - JSONL command admission, mutation ordering and transition deadlock guards

### 1. Title and status

JSONL command admission, mutation ordering and transition deadlock guards. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/modes/rpc/rpc-command-coordinator.ts:1-114; ice:packages/coding-agent/src/modes/rpc/rpc-input-admission.ts:1-65; ice:packages/coding-agent/src/modes/rpc/rpc-command-lanes.ts:3-17; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:151-189; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:2136-2208; ice:packages/coding-agent/test/rpc-command-coordinator.test.ts:12-189; ice:packages/coding-agent/test/rpc-input-admission.test.ts:9-48.

Exported symbols: `RPC_MAX_PENDING_COMMANDS` (ice:packages/coding-agent/src/modes/rpc/rpc-command-coordinator.ts:4).
Exported symbols: `RPC_LIVE_CONTROLS` (ice:packages/coding-agent/src/modes/rpc/rpc-command-lanes.ts:4).
Exported symbols: `RPC_MAX_RETAINED_INPUT_BYTES`, `RPC_MAX_CONTROL_INPUT_BYTES`, `RPC_MAX_LIVE_CONTROLS` (ice:packages/coding-agent/src/modes/rpc/rpc-input-admission.ts:5; ice:packages/coding-agent/src/modes/rpc/rpc-input-admission.ts:6; ice:packages/coding-agent/src/modes/rpc/rpc-input-admission.ts:7).

### 3. What it does

Keep reads responsive while preserving wire order for short mutations and session transitions, with reserved control capacity and explicit rejection of self-dependent extension transitions.

### 4. How it works

Admission keeps reads responsive while preserving the order of mutations and session transitions. Separate control capacity allows cancellation, and dependency checks prevent an extension transition from waiting on itself.

Input admission reserves a count and UTF-8 byte size for the validated line before creating request ownership. Normal requests allow at most 256 and 32 MiB; live controls have separate 64-request/1-MiB limits. Exact byte ceilings are accepted; an already-full count or a sum exceeding the ceiling rejects busy before dispatch. close permanently rejects new admissions as cancelled. Idempotent release returns both bytes/count and resolves idle waiters only when both lanes have no work. Server counting excludes LF because it receives a decoded line; the child-process client reserves its serialized line including LF (ice:packages/coding-agent/src/modes/rpc/rpc-input-admission.ts:5-64; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:2136-2142; ice:packages/coding-agent/src/modes/rpc/rpc-client.ts:1203-1213).

The coordinator separately defaults to 256 active admissions and validates a positive safe configured count. It installs an admission/latch synchronously before running code or yielding. A transition depends on every active admission except an allowed extension caller. Other operations depend on the current transition barrier, plus the previous mutation tail only when classified as a mutation; duplicate dependencies collapse. Pure reads otherwise run concurrently. Latches resolve in finally even when operations reject, releasing dependent requests without asserting rollback. A completed older barrier/tail clears the pointer only if it still owns it (ice:packages/coding-agent/src/modes/rpc/rpc-command-coordinator.ts:21-29; ice:packages/coding-agent/src/modes/rpc/rpc-command-coordinator.ts:72-112).

The seven transitions are new_session, switch_session, fork, clone, import_session, reload and navigate_tree. Twenty-seven mutations cover prompt/steer/follow_up; queue writes; tools/scopes/settings/model/thinking/fast/queue/auto-compaction/auto-retry setters and cycles; labels/name; get_commands and get_completions because they refresh skills; invoke_command/invoke_shortcut; subagent_patch; refresh_models; delete_session/rename_session. Long compact and bash retain full admission latches for transitions but do not occupy the mutation lane; prompt admission ends at preflight settlement while streaming can continue. Twelve live controls bypass the coordinator: abort, abort_bash, abort_retry, abort_compaction, abort_branch_summary, respond_subagent_approval, stop_after_turn, kill_shell, background_bash, get_capabilities, abort_models_refresh and cancel_request. Dialog replies bypass both command admission and coordinator and are bounded separately by pending dialogs (ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:151-189; ice:packages/coding-agent/src/modes/rpc/rpc-command-lanes.ts:3-17; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:2125-2134; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:2163-2165).

AsyncLocalStorage remembers each admission. Mutation admissions initially permit extension transitions; prompt preflight acceptance retires that permission synchronously. runTransition first rejects busy if a remembered owner disallows transition and streaming/compaction/user-Bash is active, including a retired prompt context no longer in the active set. Then an active owner rejects if any barrier exists or any active admission depends on it transitively; otherwise a nested transition may exclude only that active caller. A completed ordinary context is not a permanent exemption. This prevents a hook awaiting replacement while replacement waits for that same operation or a dependent queued command (ice:packages/coding-agent/src/modes/rpc/rpc-command-coordinator.ts:32-70; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:220-225).

### 5. Interfaces

RpcInputAdmission.begin(bytes, control) returns release; close/isClosing/waitForIdle. RpcCommandCoordinator.run(operation, transition=false, mutation=false), runTransition, retirePromptPreflight. busy errors do not cancel a previously admitted command.

### 6. Dependencies

`RPC-016`, `RPC-019`, `RPC-020`, `SESSION-021`

Replacement/disposal belongs to the session runtime, but wire admission and reentrant host-action ordering must surround it. Controls address the currently bound session, not a future replacement.

### 7. ice-rust today

Native rpc_with_io has one background prompt task and continues reading simple controls, so prompt responsiveness exists. It rejects a second prompt with 'a run is already active'; there is no coordinator for the missing mutation/transition command family, reserved input-byte lanes, queued-ID cancellation or extension-transition dependency graph. Native UI mailbox polling is a separate mechanism and must not be counted as this admission system (rust:crates/ice-cli/src/modes.rs:355-571).

### 8. Port notes

Introduce an adapter-level admission graph and two bounded input lanes around native session operations. Reserve transitions synchronously, distinguish preflight from streaming lifetime, keep pure snapshots concurrent and long work out of the short mutation tail. Track extension caller ownership and transitively dependent admissions so nested replacement fails busy instead of deadlocking. Release latches only when predecessors and operations actually settle.

### 9. Size and priority

Estimated Rust implementation: **240-430 lines** (estimate, not measured source). **P1** - Incorrect admission order can apply mutations to the wrong session or deadlock an extension hook during replacement.

## RPC-019 - Request-owned cancellation, queue-time deadlines and terminal response semantics

### 1. Title and status

Request-owned cancellation, queue-time deadlines and terminal response semantics. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/modes/rpc/rpc-request-tracker.ts:1-94; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:861-866; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:1432-1493; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:1640-1671; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:2136-2208; ice:packages/coding-agent/test/rpc-request-tracker.test.ts:6-85.

Exported symbols: `RpcRequestExecutionError`, `RPC_MAX_REQUEST_TIMEOUT_MS`, `RPC_CANCELLABLE_COMMANDS`, `RpcRequestScope` (ice:packages/coding-agent/src/modes/rpc/rpc-request-tracker.ts:3; ice:packages/coding-agent/src/modes/rpc/rpc-request-tracker.ts:14; ice:packages/coding-agent/src/modes/rpc/rpc-request-tracker.ts:15; ice:packages/coding-agent/src/modes/rpc/rpc-request-tracker.ts:18).

### 3. What it does

Cancel one queued or supported running request without releasing its ordering barrier early, hijacking another operation's controller, or implying that committed effects were rolled back.

### 4. How it works

Every admitted request owns its cancellation signal and any deadline. Cancellation changes the terminal response after work settles; it does not establish that earlier effects were undone.

Each admitted command receives an AbortController, started=false and running-cancellable flag. compact, navigate_tree and bash alone set that flag. A defined id, including empty string, must be unique among active records; duplicate invalid_request leaves original ownership intact. Anonymous records remain in the active set for deadline/shutdown ownership. Only those three commands may install an optional 1–300000-ms timer; it starts in begin before coordinator wait, is unref'ed, and aborts with a typed timeout reason. Omission installs no server deadline. finish clears the timer and active record, removing by-id ownership only if it still refers to that record, so repeated stale cleanup cannot erase a later request reusing the id (ice:packages/coding-agent/src/modes/rpc/rpc-request-tracker.ts:14-70; ice:packages/coding-agent/test/rpc-request-tracker.test.ts:36-84).

cancel_request returns accepted=false for unknown/completed/already-aborted targets or a dispatched unsupported command; every not-yet-started command can be signalled. A successful cancellation aborts with cancelled and returns true without awaiting target settlement. The first abort reason wins. abortAll signals queued records and the three supported running kinds, leaves unsupported running effects alone, and clears all timers (ice:packages/coding-agent/src/modes/rpc/rpc-request-tracker.ts:73-92; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:861-866).

Execution checks shutdown/runtime availability, then throws an already-aborted request signal before mark-start and dispatch. It awaits the coordinator/handler and checks the signal again before publishing success. A signalled request's reason overrides a concurrent caught error for the terminal failure, preserving cancelled versus timeout. Request finish occurs before response publication and again defensively in finally; admitted input capacity is released after response/backpressure handling. Aborting a queued command does not remove its coordinator dependency latch: it still waits predecessors before reaching the dispatch check, so later reads cannot overtake an earlier transition. Running cooperative work must settle before its terminal response; there is no hard kill or rollback guarantee (ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:2144-2208).

compact and navigate_tree receive the request signal in their native session options. Direct Bash first awaits user_bash hooks; a returned hook result is recorded immediately, whereas host fallback checks the signal before spawning and passes it to executeBash. The outer post-handler check can still report cancellation after an extension result or another effect committed. Operation-wide abort_compaction/abort_branch_summary/abort_bash/abort_retry remain separate live controls. The manual compaction pre-save versus post-save checkpoint distinction is SESSION-016; no request tracker makes a non-cooperative hook interruptible (ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:1432-1493; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:1640-1671).

### 5. Interfaces

RpcRequestTracker.begin/start/finish/cancel/abortAll; cancel_request {requestId} -> {accepted:boolean}; timeoutMs only compact/navigate_tree/bash (refresh_models separate). Failure codes invalid_request/timeout/cancelled with the original command/id; errorDetails undefined for request ownership errors.

`RpcRequestScope` exposes readonly signal plus start()/finish(). `RpcRequestExecutionError` carries errorCode invalid_request|timeout|cancelled, errorDetails=undefined, the supplied message and class name; `RPC_MAX_REQUEST_TIMEOUT_MS` is 300000 and `RPC_CANCELLABLE_COMMANDS` is the readonly tuple compact/navigate_tree/bash (ice:packages/coding-agent/src/modes/rpc/rpc-request-tracker.ts:3-22).

### 6. Dependencies

`RPC-018`, `RPC-020`, `SESSION-016`, `SESSION-024`, `SESSION-026`

Session-owned operation controllers and pre-commit checks remain authoritative. Client-side immediate cancellation and best-effort wire cancellation are specified separately in RPC-033.

### 7. ice-rust today

Native prompt cancellation uses a run-wide CancellationToken and lane.abort; abort returns an acknowledgment before the active prompt task necessarily settles. Native stop cancels/joins during shutdown. These are useful operation controls but there is no request-ID registry, cancel_request, queued deadline or per-request compaction/navigation/Bash ownership in the native JSONL dispatcher (rust:crates/ice-cli/src/modes.rs:395-398; rust:crates/ice-cli/src/modes.rs:490-588; rust:crates/ice-cli/src/modes.rs:652-690).

### 8. Port notes

Add request-owned state independently of the agent run token, start deadlines at admission, and preserve first cancellation cause and id ownership. Keep the coordinator latch until predecessors/native work settle, forward signals only to the target operation and reject late success when signalled. Expose cancellation acceptance separately from terminal settlement, retaining native run-wide abort as its own command.

### 9. Size and priority

Estimated Rust implementation: **170-310 lines** (estimate, not measured source). **P1** - Otherwise clients can believe a mutation stopped, release ordering too soon or cancel an unrelated active operation.

## RPC-020 - JSONL bootstrap, runtime rebinding, shutdown and explicit recovery

### 1. Title and status

JSONL bootstrap, runtime rebinding, shutdown and explicit recovery. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:215-238; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:577-701; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:2062-2108; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:2136-2251; ice:packages/coding-agent/docs/rpc.md:104-110.

Exported symbols: `RpcModeOptions` (ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:143).
Exported symbols: `RpcSessionChangedEvent` (ice:packages/coding-agent/src/modes/rpc/rpc-types.ts:708).

### 3. What it does

Bind extension UI and session subscriptions before declaring readiness, fence session replacement, and settle admitted work before normal runtime disposal while allowing deliberate recovery from an unavailable runtime.

### 4. How it works

RPC startup binds the runtime and extension interfaces before declaring readiness. Rebinding and shutdown coordinate admitted work, subscriptions and disposal so stale session state does not remain active.

Input listeners are installed before the initial extension bind. Bootstrap reserves a zero-byte normal admission and a transition, executes rebindSession, then clears bootstrapping, resolves the bootstrap-ready promise and releases admission in finally. A validated get_capabilities may use the control lane but awaits bootstrapReady before execution; dialog replies remain live during startup hooks. Bootstrap failure starts failure shutdown and rethrows. The normal runRpcMode return is a never-settling promise; process shutdown is explicit (ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:2136-2165; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:2211-2251).

A runtime-host rebind callback remembers the previous bound id, awaits full rebind, then changes bound id, resets remembered catalog refresh failure and emits session_changed with previous/current id, optional sessionFile, cwd and leafId before the transition's response. Initial binding, same-session navigation and reload do not use that replacement notification path. rebindSession updates the local session, resets UI state and awaits bindExtensions(mode='rpc') with waitForIdle and coordinated new/fork/navigate/switch/reload host actions; reload clears UI immediately before reloading. It then replaces catalog/settings/shell/session/backpressure subscriptions, cancelling a pending old shell coalescing timer. Extension errors become extension_error output. The subagent bridge subscription is installed once against the host bridge, separately from per-session rebind (ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:577-701).

startShutdown is guarded by shuttingDown and closes input admission synchronously before awaiting anything. It detaches/pause stdin, aborts tracked queued/supported requests and model-refresh controllers, resolves pending extension dialogs as cancelled, clears conversational queues, aborts user Bash and branch summary, then starts session.abort. Its asynchronous path waits admitted handlers and that abort before runtimeHost.dispose. Failure changes exit status to 1 and emits a generic recovery diagnostic. Finally it removes signal and all session/bridge subscriptions and pending shell timer. It flushes raw stdout unless the signal is SIGTERM, then exits with the chosen code. A repeated request/signal shares this path; there is no hard settlement timeout. SIGTERM and, off Windows, SIGHUP handlers first kill tracked detached children and use exit codes 143/129. EOF uses the same shutdown after the final unterminated record has been consumed (ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:672-686; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:2062-2108; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:2211-2235).

Before dispatch, an unavailable AgentSessionRuntime allows only get_capabilities, cancel_request, abort_models_refresh, new_session, switch_session and import_session; other commands fail runtime_unavailable. Recovery uses those explicit transitions rather than replaying effects. Error classification prioritizes typed settings and execution errors, then unavailable runtime, import file not found, ModelsError and AbortError; remaining failures default to failed. Session runtime construction, stale-context invalidation and failed replacement behavior are owned by SESSION-021. Immediate asynchronous output-guard sink exit remains the separate RPC-017 limitation (ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:2145-2161; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:2173-2208).

### 5. Interfaces

runRpcMode(runtimeHost,{agentViewBridge?}); get_capabilities readiness; session_changed {previousSessionId, sessionId, sessionFile?, cwd, leafId}; runtime_unavailable failure; host-internal rebind and shutdown hooks. No automatic recovery/reconnect/retry.

Exact optional host bridge and replacement notification DTOs:

```ts
export interface RpcModeOptions {
	/** ICE-only bridge exposing the subagent overview and approval answers. */
	agentViewBridge?: IceAgentViewBridge;
}
```
(ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:143-146).

```ts
export interface RpcSessionChangedEvent {
	type: "session_changed";
	previousSessionId: string;
	sessionId: string;
	sessionFile?: string;
	cwd: string;
	leafId: string | null;
}
```
(ice:packages/coding-agent/src/modes/rpc/rpc-types.ts:708-715).

### 6. Dependencies

`EXT-019`, `RPC-017`, `RPC-018`, `RPC-019`, `SESSION-021`

This is adapter lifecycle around the existing replaceable runtime, not a second session factory. Extension live contexts and their invalidation remain owned by EXT-019 and SESSION-021.

### 7. ice-rust today

Native RPC emits ready before reading commands, attaches a UI dialog mailbox/polling pump, and keeps one prompt task. On ordinary EOF it does not cancel that accepted prompt: it detaches UI input, awaits prompt completion, drains event pumps and joins the writer; stop, stdin error or writer failure set cancel_on_exit and cancel before joining. A prompt flush request ensures all final agent events precede its completion response. Native app also has BeforeTuiStart/SessionStart/SessionShutdown hooks and the conditional extension-owned stdio branch. These real finite-input and writer-order guarantees differ from TS EOF abort/clear behavior and must be preserved or deliberately translated; native has no missing-command runtime replacement/recovery gate (rust:crates/ice-cli/src/modes.rs:355-403; rust:crates/ice-cli/src/modes.rs:406-421; rust:crates/ice-cli/src/modes.rs:527-588; rust:crates/ice-cli/src/app.rs:434-518).

### 8. Port notes

Implement the JSONL compatibility host lifecycle around native session ownership with input-before-bind readiness and coordinated rebind callbacks. Drain admitted work before normal disposal, cancel dialogs/queues/controllers consistently and make unavailable runtime recovery explicit. Keep native EOF-completes-accepted-work behavior documented; select a compatibility policy deliberately rather than erasing that capability. Do not promise clean cooperative settlement for fatal sink errors that bypass this path.

### 9. Size and priority

Estimated Rust implementation: **230-440 lines** (estimate, not measured source). **P1** - Readiness and disposal races otherwise expose half-bound sessions, stale extension state or incomplete responses, while blind recovery can replay mutations.

## RPC-021 - JSONL prompt admission, attachments and conversational queue controls

### 1. Title and status

JSONL prompt admission, attachments and conversational queue controls. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/modes/rpc/rpc-validation.ts:24-74; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:792-878; ice:packages/coding-agent/src/cli/file-processor.ts:1-101; ice:packages/coding-agent/test/rpc-prompt-response-semantics.test.ts:1-457; ice:packages/coding-agent/test/suite/rpc-introspection.test.ts:453-479.

Exported symbols: `RpcQueueSnapshot`, `RpcAttachment` (ice:packages/coding-agent/src/modes/rpc/rpc-types.ts:300; ice:packages/coding-agent/src/modes/rpc/rpc-types.ts:574).

### 3. What it does

Expose prompt, steer, follow_up, get_queue, clear_queue, remove_queued_message and abort with the TypeScript preflight acknowledgment boundary and host-side attachment processing.

### 4. How it works

Prompt responses acknowledge successful preflight separately from the model run. Attachments and conversational queue commands preserve the session's own ordering, expansion and cancellation rules.

prompt requires a string message, optional image array, streamingBehavior steer|followUp, boolean expandPromptTemplates, and at most 16 attachments. Inline text/image names may be empty but are at most 256 UTF-16 code units; each text payload is at most 1 MiB UTF-8. File paths must be nonempty and at most 4096 code units. Image data and MIME type are strings; this validator does not verify base64 or restrict MIME types. The overall 16 MiB input-line limit remains authoritative. File attachments resolve against the bound session cwd and use image auto-resize settings and maxTextBytes=1048576. A FileArgumentError too_large maps to invalid_argument; not_found/unreadable retain their codes (ice:packages/coding-agent/src/modes/rpc/rpc-validation.ts:27-66; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:792-808; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:212-213).

Inline text attachments are wrapped with a JSON-quoted name in an attachment element, kept in their relative order, then followed by the aggregate host-file text with its trailing whitespace trimmed. The combined text is prefixed to the original message with two newlines. Images are ordered command.images, inline image attachments, then file images. Any nonempty attached text forces expandPromptTemplates=false; image-only inline attachments do not. The file helper skips empty files, rejects nonregular files, checks text size from stat before reading, and emits absolute-path file wrappers without escaping the path. Successfully processed image files also emit textual file annotations, and image-processing errors are represented in that text rather than failing the entire prompt. The pre-read size check is not a post-read race-proof bound (ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:809-832; ice:packages/coding-agent/src/cli/file-processor.ts:35-97).

The host calls session.prompt with source=rpc and a preflightResult callback. Only successful preflight retires the coordinator's prompt-preflight barrier and resolves the one success response; if shutdown began, that callback throws cancelled instead. A preflight rejection produces a failure response; errors after successful preflight are session events and cannot produce a second response. Admission is held through asynchronous input/extension preflight but released while model streaming continues. The session owns expansion, input hooks, busy/queue decisions, steering wake and final settlement (SESSION-019/020); the RPC wrapper must not wait for model completion before acknowledging an accepted prompt (ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:824-843; ice:packages/coding-agent/test/rpc-prompt-response-semantics.test.ts:1-457).

steer/follow_up await the corresponding session operation before a data-less success. get_queue returns the session's steering and followUp snapshots; clear_queue returns the removed queues; remove_queued_message returns {removed:boolean}. abort awaits session.abort and is a live control, distinct from cancel_request's targeted admission/deadline semantics. Queue and prompt effects remain owned by the session; string message validation does not trim or reject empty text (ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:846-878; ice:packages/coding-agent/src/modes/rpc/rpc-validation.ts:34-74).

### 5. Interfaces

Commands prompt {message, images?, streamingBehavior?, expandPromptTemplates?, attachments?}, steer/follow_up {message, images?}, get_queue, clear_queue, remove_queued_message {messageId}, abort. Attachment union: {type:'text', name, text}, {type:'image', name, data, mimeType}, {type:'file', path}. Accepted prompt acknowledgment is separate from agent_settled.

Exact public queue and attachment declarations:

```ts
export interface RpcQueueSnapshot {
	steering: readonly string[];
	followUp: readonly string[];
}
```
(ice:packages/coding-agent/src/modes/rpc/rpc-types.ts:300-303).

```ts
export type RpcAttachment =
	| { type: "text"; name: string; text: string }
	| { type: "image"; name: string; data: string; mimeType: string }
	/** A host file read like a CLI `@file` argument; relative paths resolve against the session cwd. */
	| { type: "file"; path: string };
```
(ice:packages/coding-agent/src/modes/rpc/rpc-types.ts:574-578).

### 6. Dependencies

`RPC-016`, `RPC-018`, `RPC-019`, `RPC-020`, `SESSION-019`, `SESSION-020`

SESSION owns conversational semantics; this entry owns the JSONL adapter, attachment assembly and precise acknowledgment point. CLI file-helper reuse is credited as source, not a new RPC file-access sandbox.

### 7. ice-rust today

The native JSONL loop has real prompt/abort behavior: a single active prompt task is rejected if already running, prompt text is taken from content or prompt, and completion responds only after the run and final event drain. Its content.or_else(prompt).and_then(as_str) means a present nonstring content prevents fallback to a valid prompt string. It does not parse TS message/images/attachments/streamingBehavior options or expose steering/follow-up queue commands in the inspected exhaustive dispatch. Native prompt cancellation stays responsive because the reader loop continues while the prompt task runs (rust:crates/ice-cli/src/modes.rs:450-566; rust:crates/ice-cli/src/modes.rs:652-696).

### 8. Port notes

Add a compatibility prompt/queue adapter over the native session rather than replacing its task/abort infrastructure. Preserve preflight acknowledgment separately from run completion, host-file resolution order, attachment limits, literal text behavior and one-response semantics. Treat the file-size race and unescaped path wrapper as current TS limitations; a stricter implementation should be an explicit reviewed difference, not an accidental change.

### 9. Size and priority

Estimated Rust implementation: **220-420 lines** (estimate, not measured source). **P1** - Clients otherwise wait for the wrong completion point, lose queue controls or submit differently expanded text and attachments.

## RPC-022 - JSONL capabilities, state, resource introspection and invalidation events

### 1. Title and status

JSONL capabilities, state, resource introspection and invalidation events. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:890-1052; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:1084-1123; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:632-669; ice:packages/coding-agent/src/modes/rpc/rpc-views.ts:26-29; ice:packages/coding-agent/src/modes/rpc/rpc-views.ts:63-137; ice:packages/coding-agent/src/modes/rpc/rpc-views.ts:172-191; ice:packages/coding-agent/test/suite/rpc-introspection.test.ts:139-197; ice:packages/coding-agent/test/suite/rpc-introspection.test.ts:365-450.

Exported symbols: `RpcCapabilities`, `RpcSessionState`, `RpcExtensionFlag`, `RpcExtensionShortcut`, `RpcDiagnostics`, `RpcCompletions`, `RpcToolInfo`, `RpcChangelogEntry`, `RpcResources` (ice:packages/coding-agent/src/modes/rpc/rpc-types.ts:213; ice:packages/coding-agent/src/modes/rpc/rpc-types.ts:305; ice:packages/coding-agent/src/modes/rpc/rpc-types.ts:587; ice:packages/coding-agent/src/modes/rpc/rpc-types.ts:597; ice:packages/coding-agent/src/modes/rpc/rpc-types.ts:603; ice:packages/coding-agent/src/modes/rpc/rpc-types.ts:621; ice:packages/coding-agent/src/modes/rpc/rpc-types.ts:638; ice:packages/coding-agent/src/modes/rpc/rpc-types.ts:664; ice:packages/coding-agent/src/modes/rpc/rpc-types.ts:670).
Exported symbols: `toPublicModel`, `listTools`, `getDiagnostics`, `readChangelog` (ice:packages/coding-agent/src/modes/rpc/rpc-views.ts:26; ice:packages/coding-agent/src/modes/rpc/rpc-views.ts:63; ice:packages/coding-agent/src/modes/rpc/rpc-views.ts:68; ice:packages/coding-agent/src/modes/rpc/rpc-views.ts:187).

### 3. What it does

Give clients an authoritative command/capability handshake, current session state, safe diagnostic summaries, tool/resource metadata and extension flag/shortcut discovery.

### 4. How it works

Clients can discover supported commands, inspect current state and request bounded diagnostic views. Invalidation events tell them when to fetch fresh data after runtime changes.

get_capabilities returns protocolVersion 1, all 79 validated command names, limits and cancellation metadata after bootstrap readiness. Advertised features are validated_input, queue_snapshot, prompt_options, working_status, transition_barrier, session_changed, session_configuration_events, provider_snapshot, catalog_refresh, entry_session_pagination, bounded_admission, request_cancellation, shutdown_settlement, bounded_transport_admission, view_pagination, runtime_invalidations, safe_diagnostics, command_completions, ui_text_snapshot, inline_attachments, file_attachments, argument_completions, extension_flags, extension_shortcuts, session_file_management, inline_export, ui_chrome_text, cache_stats and explicit_runtime_recovery. Limits are 16 MiB input lines,256 pending normal commands,300000 ms request deadline,32 MiB retained normal input,1 MiB control input,64 live controls and 32 MiB output. Cancellable-running command names come from the three-command tracker; queuedCommands excludes live controls, and cooperative/includesQueueTime are true. host.subagents means a bridge was supplied, not that its runtime is enabled (ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:890-941; RPC-018/019).

get_state projects the selected public model (only its headers field is removed), thinking and fast-mode values/availability, streaming/compacting state, queue modes, session id/file/name/leaf/directory/persistence/parent, auto-compaction, message and pending counts, cwd, context usage, retry flags/attempt, foreground Bash state and running background-shell count. Optional undefined fields are omitted by JSON serialization; notably absent model is omitted, not normalized to the null shown in some documentation. get_session_stats additionally includes computed cacheWaste (RPC-024). get_tools returns all tool metadata/schema with active membership; set_active_tools rejects the entire request if any name is unknown and otherwise returns actual active names. get_system_prompt returns the complete effective text (ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:944-993; ice:packages/coding-agent/src/modes/rpc/rpc-views.ts:26-29; ice:packages/coding-agent/src/modes/rpc/rpc-views.ts:63-66).

get_resources returns context-file paths, system/base/append prompt paths, skill/prompt/theme/extension metadata and provenance, extensionErrors and aggregated resource diagnostics, without loading body content into this response. It passes extensionErrors and resource diagnostic objects through; it is not the sanitized get_diagnostics view. get_diagnostics instead synthesizes generic catalog/settings/extension/resource messages, keeps at most 100 in that order, truncates optional paths to 1024 code units, reports truncated, and does not drain settings write errors. get_changelog parses and reverses entries into newest-first version/content pairs (ice:packages/coding-agent/src/modes/rpc/rpc-views.ts:68-137; ice:packages/coding-agent/src/modes/rpc/rpc-views.ts:186-191).

get_flags returns registered name/description/type/default/current value/extensionPath; it does not mutate startup flags. get_shortcuts applies current effective keybindings and returns resolved shortcut metadata. invoke_shortcut matches the resolved shortcut case-insensitively, fails not_found when absent, then awaits the handler with a fresh extension context. get_completions accepts a prefix up to 256 code units and limit 1–1000 (default 100), refreshes the command inventory first, and for a slash prefix containing no literal space performs case-sensitive command-name prefix matching with source/description. Otherwise it reuses CombinedAutocompleteProvider with cwd, fd and registered argument completers plus the request signal; it returns kind argument for slash prefixes or path otherwise, replacement prefix, bounded items and hasMore. Its inventory/refresh mutation is coordinated; the autocomplete engine itself remains TUI-owned (ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:999-1044; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:1084-1123; ice:packages/coding-agent/src/modes/rpc/rpc-validation.ts:83-98).

Catalog and settings subscriptions emit payload-free catalog_changed/settings_changed invalidations while not shutting down. Session configuration events pass through the main SESSION-022 event projection; consumers refetch authoritative views. Per-session subscriptions are replaced on rebind, preventing an old bound session from continuing to publish these notifications (ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:632-669; RPC-020).

### 5. Interfaces

get_capabilities, get_state, get_tools, set_active_tools {toolNames:string[]}, get_system_prompt, get_resources, get_diagnostics, get_changelog, get_flags, get_shortcuts, invoke_shortcut {shortcut}, get_completions {prefix, limit?}; events catalog_changed, settings_changed and existing session configuration events. get_ui_state belongs RPC-028.

Exact public capability and read-side view DTOs. These type declarations do not strengthen the runtime validators beyond the checks described in the mechanism:

```ts
export interface RpcCapabilities {
	protocolVersion: 1;
	commands: RpcCommandType[];
	features: string[];
	limits: {
		maxInputLineBytes: number;
		maxPendingCommands?: number;
		maxRequestTimeoutMs?: number;
		maxRetainedInputBytes?: number;
		maxControlInputBytes?: number;
		maxLiveControls?: number;
		maxQueuedOutputBytes?: number;
	};
	cancellation?: {
		commands: RpcCommandType[];
		queuedCommands?: RpcCommandType[];
		cooperative: true;
		includesQueueTime: true;
	};
	host: { subagents: boolean };
}
```
(ice:packages/coding-agent/src/modes/rpc/rpc-types.ts:213-233).

```ts
export interface RpcSessionState {
	model?: Model<any>;
	thinkingLevel: ThinkingLevel;
	fastMode: boolean;
	fastModeAvailable: boolean;
	isStreaming: boolean;
	isCompacting: boolean;
	steeringMode: "all" | "one-at-a-time";
	followUpMode: "all" | "one-at-a-time";
	sessionFile?: string;
	sessionId: string;
	sessionName?: string;
	leafId: string | null;
	sessionDir: string;
	persisted: boolean;
	parentSessionPath?: string;
	autoCompactionEnabled: boolean;
	messageCount: number;
	pendingMessageCount: number;
	cwd: string;
	/** Estimated context usage; undefined when no model is selected. */
	contextUsage?: ContextUsage;
	autoRetryEnabled: boolean;
	isRetrying: boolean;
	retryAttempt: number;
	/** True while a foreground `bash` command (user or RPC) is running. */
	isBashRunning: boolean;
	/** Background shells still running. */
	backgroundShellCount: number;
}
```
(ice:packages/coding-agent/src/modes/rpc/rpc-types.ts:305-334).

```ts
export interface RpcExtensionFlag {
	name: string;
	description?: string;
	type: "boolean" | "string";
	default?: boolean | string;
	value?: boolean | string;
	extensionPath: string;
}

/** A keyboard shortcut registered by an extension, after built-in conflict resolution. */
export interface RpcExtensionShortcut {
	shortcut: string;
	description?: string;
	extensionPath: string;
}

export interface RpcDiagnostics {
	diagnostics: Array<{ source: string; severity: "warning" | "error"; code: string; path?: string; message: string }>;
	truncated: boolean;
}
```
(ice:packages/coding-agent/src/modes/rpc/rpc-types.ts:587-606).

```ts
export interface RpcCompletions {
	/** `command`: slash names; `argument`: a command's argument completions; `path`: `@file` or path tokens. */
	kind: "command" | "argument" | "path";
	/** Trailing text of `prefix` that each item's `value` replaces. */
	replace: string;
	items: Array<{ value: string; label?: string; description?: string; source?: RpcCommandSource }>;
	hasMore: boolean;
}
```
(ice:packages/coding-agent/src/modes/rpc/rpc-types.ts:621-628).

```ts
export type RpcToolInfo = ToolInfo & { active: boolean };
```
(ice:packages/coding-agent/src/modes/rpc/rpc-types.ts:638-638).

```ts
export interface RpcChangelogEntry {
	version: string;
	content: string;
}
```
(ice:packages/coding-agent/src/modes/rpc/rpc-types.ts:664-667).

```ts
export interface RpcResources {
	contextFiles: string[];
	systemPromptPath?: string;
	baseSystemPromptPath?: string;
	appendSystemPromptPaths: string[];
	skills: Array<{ name: string; description: string; filePath: string; sourceInfo: SourceInfo }>;
	prompts: Array<{ name: string; description?: string; sourceInfo: SourceInfo }>;
	themes: Array<{ name?: string; sourcePath?: string }>;
	extensions: Array<{ path: string; sourceInfo: SourceInfo }>;
	extensionErrors: Array<{ path: string; error: string }>;
	diagnostics: ResourceDiagnostic[];
}
```
(ice:packages/coding-agent/src/modes/rpc/rpc-types.ts:670-681).

### 6. Dependencies

`EXT-019`, `RPC-016`, `RPC-018`, `RPC-020`, `RPC-024`, `RPC-028`, `RPC-031`, `SESSION-022`, `SESSION-023`

Underlying extension registration, settings/catalog state and autocomplete algorithms remain in their assigned areas. This adapter owns discovery shape, bounds, refresh triggers and event delivery.

### 7. ice-rust today

Native get_state exists and returns model id, thinkingLevel, activeTools, leafId and sessionId. Getter failures are converted to None/default arrays rather than typed failure responses. Native set_active_tools reads a tools array and silently filters nonstrings before delegating, which differs from strict toolNames validation and the whole-update unknown-name check. There is no get_capabilities or the remaining metadata/flag/shortcut/completion/diagnostic command family in the complete native dispatch. Native extension and TUI introspection capabilities are not erased by that missing wire exposure (rust:crates/ice-cli/src/modes.rs:450-566; rust:crates/ice-cli/src/modes.rs:698-769).

### 8. Port notes

Publish the full compatibility handshake and exact command inventory, project native state into TS fields, validate tool selections before changing them, and bind refetch invalidations to session replacement. Keep sanitized diagnostics separate from raw resource diagnostic metadata and remove only the model fields the contract specifies. Reuse native extension/TUI registries behind the adapter instead of inventing parallel registration stores.

### 9. Size and priority

Estimated Rust implementation: **280-520 lines** (estimate, not measured source). **P1** - Remote clients need discovery and coherent current-state views to select commands safely and refresh after live changes.

## RPC-023 - JSONL model catalogs, explicit refresh and live session controls

### 1. Title and status

JSONL model catalogs, explicit refresh and live session controls. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:296-301; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:1054-1082; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:1234-1458; ice:packages/coding-agent/src/modes/rpc/rpc-views.ts:139-170; ice:packages/coding-agent/src/modes/rpc/rpc-views.ts:193-215; ice:packages/coding-agent/src/modes/rpc/rpc-pages.ts:11-27; ice:packages/coding-agent/src/modes/rpc/rpc-validation.ts:118-147.

Exported symbols: `RpcProviderInfo`, `RpcProviderSnapshot`, `RpcModelSnapshot`, `RpcModelsRefreshResult`, `RpcModelsChangedEvent`, `RpcScopedModelRef` (ice:packages/coding-agent/src/modes/rpc/rpc-types.ts:236; ice:packages/coding-agent/src/modes/rpc/rpc-types.ts:246; ice:packages/coding-agent/src/modes/rpc/rpc-types.ts:252; ice:packages/coding-agent/src/modes/rpc/rpc-types.ts:274; ice:packages/coding-agent/src/modes/rpc/rpc-types.ts:281; ice:packages/coding-agent/src/modes/rpc/rpc-types.ts:631).
Exported symbols: `getProviderSnapshot`, `getScopedModelRefs`, `resolveScopedModels` (ice:packages/coding-agent/src/modes/rpc/rpc-views.ts:140; ice:packages/coding-agent/src/modes/rpc/rpc-views.ts:193; ice:packages/coding-agent/src/modes/rpc/rpc-views.ts:202).

### 3. What it does

Expose cached provider/model discovery, explicit bounded catalog refresh, model/thinking selection, compaction/retry settings and immediate cancellation controls through JSONL.

### 4. How it works

Model discovery uses cached metadata until an explicit refresh is requested. Selection, refresh cancellation and session controls expose their actual result and persistence boundaries through the wire API.

get_providers inspects cached provider/auth status and all/available model snapshots only: it returns id/name, api_key/oauth capability tags, configured/source and conditional credential type, counts, dynamicCatalog and hasCatalogError. It neither resolves credentials nor runs auth commands or refreshes OAuth. get_models optionally filters provider (unknown provider is not_found) and availability, while get_available_models always uses the cached available snapshot. Both sort provider then id with English localeCompare, remove only model.headers, and use JSON.stringify([provider, id]) as the live key cursor. The cursor must identify an item in the current filtered list; unknown keys are not_found. Limit is 1–1000 if given, otherwise all remaining items; nextCursor is emitted only with hasMore. get_models/get_providers OR the current runtime error with the adapter's remembered refresh failure; get_available_models has no error flag (ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:1330-1379; ice:packages/coding-agent/src/modes/rpc/rpc-views.ts:139-170; ice:packages/coding-agent/src/modes/rpc/rpc-pages.ts:11-27).

refresh_models first rejects busy when streaming/compacting, then denies allowNetwork=true whenever ICE_OFFLINE is defined, including an empty string, then rejects unknown provider ids before any refresh. It captures the current session, tracks its own AbortController and starts a default 10000 ms timer (input permits 1–30000), then calls modelRuntime.refresh with allowNetwork default false, provider filter, force and signal. It refreshes selected-model metadata after the awaited result. The response is success=true even when refresh reports errors, aborts, times out or throws: data contains aborted, timedOut, generic provider/runtime error records with ModelsError.code or failed, and hasCatalogError. A thrown abort yields no errors; a nonaborted throw yields a generic runtime error. It emits models_changed with that same result before the response. Nonaborted results replace lastCatalogRefreshFailed with errors.length>0; aborted results only set it true when errors exist, never clear a prior failure. The result's error flag is ORed with that memory, reset on session replacement. Finally removes the controller and timer. abort_models_refresh aborts every tracked refresh controller and acknowledges without waiting for completion (ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:296-301; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:1254-1328; RPC-020).

set_model finds an exact provider/id pair among available models, returns not_found if absent, awaits session.setModel and returns that public model. cycle_model delegates direction and returns null when no result or the cycle result with public model. get/set_scoped_models use exact available provider/modelId references; an unknown reference throws before the session list changes. Optional thinkingLevel is preserved when present, including off; the session owns scope persistence and cycling. set_thinking_level returns the actual session level after clamping; cycle_thinking_level returns null or {level}; get_available_thinking_levels returns the actual list. All eight TS wire levels through ultra are validated; underlying thinking policy is AI-005/SESSION-036 (ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:1054-1067; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:1234-1252; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:1385-1406; ice:packages/coding-agent/src/modes/rpc/rpc-views.ts:193-215).

set_fast_mode, set_steering_mode, set_follow_up_mode, set_auto_compaction and set_auto_retry delegate validated boolean or all|one-at-a-time values and return data-less success. compact awaits session.compact(customInstructions,{signal}) and returns its result; targeted request cancellation can still replace a late post-save result with cancelled while preserving committed session effects. abort_retry/abort_compaction/abort_branch_summary call the corresponding synchronous abort methods and immediately acknowledge. stop_after_turn requests a turn-boundary stop. reload rejects streaming/compacting, awaits reload and resets UI immediately before session_start. These commands must preserve session method preconditions, event ordering and persistence rather than inventing RPC-local copies of the same state (ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:1069-1082; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:1390-1458; RPC-019).

### 5. Interfaces

Model commands get_providers/get_models/get_available_models/refresh_models/abort_models_refresh/set_model/cycle_model/get_scoped_models/set_scoped_models; thinking set/cycle/get_available_thinking_levels; mode/settings controls listed above; compact {customInstructions?, timeoutMs?}; models_changed {providers?, result}. Refresh deadlines are a separate controller family from request-targeted operation cancellation.

Exact cached provider/model, refresh event and scoped-model DTOs; auth status remains a non-secret cached view:

```ts
export interface RpcProviderInfo {
	id: string;
	name: string;
	authTypes: Array<"api_key" | "oauth">;
	auth: Pick<AuthStatus, "configured" | "source"> & { type?: "api_key" | "oauth" };
	modelCount: number;
	availableModelCount: number;
	dynamicCatalog: boolean;
}

export interface RpcProviderSnapshot {
	providers: RpcProviderInfo[];
	/** A local configuration/composition/availability error exists; raw error text is omitted. */
	hasCatalogError: boolean;
}

export interface RpcModelSnapshot {
	models: readonly Model<string>[];
	hasCatalogError: boolean;
	hasMore?: boolean;
	nextCursor?: string;
}
```
(ice:packages/coding-agent/src/modes/rpc/rpc-types.ts:236-257).

```ts
export interface RpcModelsRefreshResult {
	aborted: boolean;
	timedOut: boolean;
	errors: Array<{ provider: string; errorCode: string; message: string }>;
	hasCatalogError: boolean;
}

export interface RpcModelsChangedEvent {
	type: "models_changed";
	providers?: string[];
	result: RpcModelsRefreshResult;
}
```
(ice:packages/coding-agent/src/modes/rpc/rpc-types.ts:274-285).

```ts
export interface RpcScopedModelRef {
	provider: string;
	modelId: string;
	thinkingLevel?: ThinkingLevel;
}
```
(ice:packages/coding-agent/src/modes/rpc/rpc-types.ts:631-635).

### 6. Dependencies

`AI-002`, `AI-005`, `AUTH-024`, `RPC-016`, `RPC-018`, `RPC-019`, `RPC-020`, `SESSION-016`, `SESSION-018`, `SESSION-036`, `SET-011`

AUTH owns credential/catalog composition and model matching, AI owns model collection and thinking helpers, and SESSION owns runtime application/compaction/retry. RPC exposes their contracts with cached discovery and explicit refresh ordering.

### 7. ice-rust today

The native loop implements set_model and set_thinking_level, but set_model accepts only a model string, chooses the first case-insensitive catalog id match without provider disambiguation, and returns {model:id}. Thinking accepts level or thinkingLevel through native serde and returns {thinkingLevel}, whose enum lacks ultra. Its catalog is supplied at startup; the complete native dispatch has no provider/cached-model discovery, dynamic refresh/abort, scope/cycle, compaction/retry settings or their invalidation result family. Native local model selection and compaction remain real capabilities in their owning areas (rust:crates/ice-cli/src/modes.rs:450-566; rust:crates/ice-cli/src/modes.rs:716-750; rust:crates/ice-ai/src/types.rs:489-503).

### 8. Port notes

Build the compatibility commands over the native model/session runtime, with exact provider/id resolution, cached auth-safe views and result/event ordering. Keep refresh failures in the documented success payload and retain abort/error history semantics. Add ultra through AI-005 rather than parsing it only in RPC; preserve operation signals and actual clamped values across command responses.

### 9. Size and priority

Estimated Rust implementation: **250-470 lines** (estimate, not measured source). **P1** - Remote selection and refresh otherwise use stale or ambiguous models, run unwanted credential work, or misreport partial/aborted refresh as a transport failure.

## RPC-024 - JSONL session listings, tree/message pages and text/statistic views

### 1. Title and status

JSONL session listings, tree/message pages and text/statistic views. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:1498-1502; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:1540-1582; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:1678-1736; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:1751-1767; ice:packages/coding-agent/src/modes/rpc/rpc-pages.ts:1-75; ice:packages/coding-agent/src/modes/rpc/rpc-views.ts:172-184; ice:packages/coding-agent/src/core/agent-session.ts:3980-4006; ice:packages/coding-agent/src/modes/rpc/rpc-response-validation.ts:25-30; ice:packages/coding-agent/src/modes/rpc/rpc-response-validation.ts:543.

Exported symbols: `RpcPage`, `pageAfterKey`, `pageMessages` (ice:packages/coding-agent/src/modes/rpc/rpc-pages.ts:5; ice:packages/coding-agent/src/modes/rpc/rpc-pages.ts:12; ice:packages/coding-agent/src/modes/rpc/rpc-pages.ts:30).
Exported symbols: `RpcMessagePage`, `RpcTreePage`, `RpcEntryPage`, `RpcSessionPage`, `RpcSessionInfo` (ice:packages/coding-agent/src/modes/rpc/rpc-types.ts:259; ice:packages/coding-agent/src/modes/rpc/rpc-types.ts:266; ice:packages/coding-agent/src/modes/rpc/rpc-types.ts:287; ice:packages/coding-agent/src/modes/rpc/rpc-types.ts:294; ice:packages/coding-agent/src/modes/rpc/rpc-types.ts:652).
Exported symbols: `toSessionInfo` (ice:packages/coding-agent/src/modes/rpc/rpc-views.ts:172).

### 3. What it does

Expose ordered entry/session/tree views, guarded message-offset pagination, fork candidates, final assistant text and session/cache statistics.

### 4. How it works

History commands provide ordered session, tree and message views with explicit cursor rules. The client distinguishes a missing cursor from a stale view, and statistics use the session's own accounting.

list_sessions defaults to cwd scope; all scope uses SessionManager.listAll() for the default session root or listAll(configuredSessionDir) for a custom root. Results sort modified descending, then path by ordinary string order, and an optional query (at most 256 code units) is trimmed/lowercased and matched against name/cwd/firstMessage/allMessagesText. A cursor is the exact path in that already-filtered current list; absent cursors fail not_found rather than restarting. Optional limit 1–1000 bounds the remaining page; omission returns all. Responses include path/id/cwd/name/parent path, ISO created/modified timestamps, messageCount and firstMessage, omitting the full-text index used for search. No retained historical listing snapshot is created (ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:1545-1582; ice:packages/coding-agent/src/modes/rpc/rpc-views.ts:172-184).

get_entries starts from SessionManager insertion order; since is an exact entry id whose preceding and matching items are skipped, with not_found when absent. It returns raw entries, current leafId, hasMore and conditional last-entry nextCursor; limit omission returns all. get_tree with no limit/cursor/parentId returns the complete nested tree and leafId. Any of those options switches to one-level pagination: parentId null/omitted selects roots, a nonnull id is found by an explicit stack walk or fails not_found, and the selected children use live id cursors and default limit 100. Each returned node has children=[] plus hasChildren, alongside parentId (null for roots), leafId and page fields. Thus requesting parentId:null alone changes both shape and default bound (ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:1678-1730; ice:packages/coding-agent/src/modes/rpc/rpc-pages.ts:11-27).

get_messages computes a SHA256 hex snapshot of JSON.stringify([sessionId, leafId, entryCount, messages.length]). Its cursor is base64url JSON [snapshot, nextOffset]. The decoded pair must have exactly two elements, a string snapshot and a safe integer offset in 0..messages.length; malformed input throws validation, while a well-formed differing snapshot throws stale. It slices the current messages (not a retained immutable copy), returns snapshot/hasMore and a conditional next cursor. The token detects session/branch/entry-count/length changes, not content mutations that preserve those four values; do not describe it as a content hash or complete streaming snapshot isolation. The wire cursor has a 1024-code-unit input cap and the usual 1–1000 optional limit (ice:packages/coding-agent/src/modes/rpc/rpc-pages.ts:29-74; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:1751-1767; ice:packages/coding-agent/src/modes/rpc/rpc-validation.ts:183).

get_fork_messages returns the session's user-message candidates. get_session_stats combines session statistics with computeCacheWaste over stored entries and model prices. get_last_assistant_text searches backward for the last assistant message except aborted messages with empty content, concatenates its text blocks without separators and trims them; absent/blank text yields undefined. There is a confirmed current-TS host/client defect: the host returns {text:undefined}, JSON serialization drops text, but its strict client requires an own text field containing a string or null. The resulting no-text data={} is rejected as invalid_response. The credential-gated live test expects undefined and was only read, not run; it does not resolve this mismatch. Record the disagreement instead of silently claiming null normalization already exists (ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:1498-1502; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:1540-1543; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:1733-1735; ice:packages/coding-agent/src/core/agent-session.ts:3984-4006; ice:packages/coding-agent/src/modes/rpc/rpc-response-validation.ts:543; ice:packages/coding-agent/test/rpc.test.ts:293-306).

### 5. Interfaces

list_sessions {scope?:cwd|all, query?, cursor?, limit?}; get_entries {since?, limit?}; get_tree {parentId?:string|null, cursor?, limit?}; get_messages {cursor?, limit?}; get_fork_messages; get_last_assistant_text; get_session_stats. Live-key errors use not_found; message cursor structure uses validation and changed identity uses stale.

Exact generic page, history page and session-list DTOs, retaining optionality and nullable tree identity fields:

```ts
export interface RpcPage<T> {
	items: T[];
	hasMore: boolean;
	nextCursor?: string;
}
```
(ice:packages/coding-agent/src/modes/rpc/rpc-pages.ts:5-9).

```ts
export interface RpcMessagePage {
	messages: AgentMessage[];
	snapshot: string;
	hasMore: boolean;
	nextCursor?: string;
}

export interface RpcTreePage {
	tree: Array<SessionTreeNode & { hasChildren?: boolean }>;
	leafId: string | null;
	hasMore?: boolean;
	nextCursor?: string;
	parentId?: string | null;
}
```
(ice:packages/coding-agent/src/modes/rpc/rpc-types.ts:259-272).

```ts
export interface RpcEntryPage {
	entries: SessionEntry[];
	leafId: string | null;
	hasMore: boolean;
	nextCursor?: string;
}

export interface RpcSessionPage {
	sessions: RpcSessionInfo[];
	hasMore: boolean;
	nextCursor?: string;
}
```
(ice:packages/coding-agent/src/modes/rpc/rpc-types.ts:287-298).

```ts
export interface RpcSessionInfo {
	path: string;
	id: string;
	cwd: string;
	name?: string;
	parentSessionPath?: string;
	created: string;
	modified: string;
	messageCount: number;
	firstMessage: string;
}
```
(ice:packages/coding-agent/src/modes/rpc/rpc-types.ts:652-662).

### 6. Dependencies

`RPC-016`, `RPC-017`, `RPC-034`, `SESSION-027`, `SESSION-032`, `SESSION-033`, `SESSION-034`

Native/TS session storage, indexing, branch selection and statistics are audited in SESSION; this entry owns the wire projection, ordering, cursor rules and discovered TS response inconsistency.

### 7. ice-rust today

Native JSONL exposes only the smaller get_state query and no command in this listing/tree/message/statistics family in its complete dispatch. Native sessions and branch/history storage exist and must be adapted through SESSION ownership; their existence does not supply these cursor/error/projection contracts automatically. The native TCP RemoteSession maintains a local event-fed transcript, which is a different contract from paginated durable store queries (rust:crates/ice-cli/src/modes.rs:450-566; rust:crates/ice-cli/src/modes.rs:698-714; rust:crates/ice-cli/src/remote/session.rs:1-493).

### 8. Port notes

Add compatibility projections over native session storage, preserving order, current-view cursor invalidation, optional full-list behavior and one-level tree shape. Explicitly decide the no-assistant-text repair: emitting text:null matches the current declared/validated wire type, with a corresponding TS test correction; copying omitted text would perpetuate the observed client failure. Keep current message-token limitations explicit if stronger snapshot semantics are introduced.

### 9. Size and priority

Estimated Rust implementation: **230-420 lines** (estimate, not measured source). **P1** - Remote history/navigation clients otherwise cannot enumerate sessions reliably or distinguish stale pages from valid continuation.

## RPC-025 - JSONL session replacement, branch navigation and file/export mutations

### 1. Title and status

JSONL session replacement, branch navigation and file/export mutations. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:757-763; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:880-884; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:1504-1538; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:1584-1676; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:1738-1745; ice:packages/coding-agent/test/suite/rpc-introspection.test.ts:219-255; ice:packages/coding-agent/test/suite/rpc-introspection.test.ts:481-520.

Exported symbols: `RpcExportResult` (ice:packages/coding-agent/src/modes/rpc/rpc-types.ts:580).

### 3. What it does

Provide replaceable-session commands, fork/clone/navigation, labels/names, branch exports, and bounded session-file management through the JSONL host.

### 4. How it works

Session-changing commands run through the shared transition coordinator. File mutations and exports preserve validation order and make clear when a failure can occur after a durable effect.

new_session passes parentSession only when truthy; switch_session passes an optional cwdOverride only when cwd is defined; import_session calls runtimeHost.importFromJsonl(path, cwd). fork accepts before|at when supplied and returns selectedText as text plus cancelled. clone requires a current leaf (otherwise not_found), forks that leaf at position=at and returns only cancelled. These runtime-host transitions acquire the RPC transition barrier, settle prior admitted work and rebind through RPC-020; they must not be implemented by mutating the old session reference in place (ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:880-884; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:1512-1538; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:1584-1590).

navigate_tree checks target entry existence first (not_found), then streaming/compacting (busy), calls session.navigateTree with signal/summarize/customInstructions/replaceInstructions/label, and projects cancelled plus optional aborted/editorText/summaryEntryId. set_entry_label validates types, rejects streaming/compacting, then appends a label change with label.trim() or undefined to clear; store-level target validation remains authoritative. set_session_name trims and rejects blank names with invalid_argument before calling session.setSessionName. Labels and names are mutations rather than runtime replacement notifications (ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:1637-1676; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:1738-1745).

export_html awaits session.exportToHtml; export_jsonl calls session.exportToJsonl. Both accept an optional outputPath and return the path, optionally reading the completed UTF-8 file as content. The file effect happens before inline-content reading/serialization/output admission; an output_too_large or later read failure does not undo the export. Session export content and theme behavior remain SESSION-038/TUI-004 (ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:1504-1510; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:1592-1598; RPC-017).

Session-file mutation targets resolve relative to the current session cwd. isSessionFilePath requires a .jsonl suffix and a nonempty relative path under either the global session directory or the configured directory, rejecting relative strings beginning '..' and absolute relative results. This is lexical resolve/relative containment, not realpath/symlink containment; it also rejects a within-root filename beginning '..'. delete_session first requires that containment (invalid_argument), then rejects the active session (busy), then checks existence (not_found), then calls deleteSessionFile and returns its trash|unlink method or failure. rename_session trims/rejects blank names first; an exact active session path is renamed via session.setSessionName before the containment check; otherwise an out-of-root or missing file both return not_found, and a valid file is opened and receives a SessionInfo append. Preserve this asymmetric error/order contract, including the active-path fast path (ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:757-763; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:1600-1635; ice:packages/coding-agent/test/suite/rpc-introspection.test.ts:481-520).

### 5. Interfaces

new_session {parentSession?}; switch_session {sessionPath, cwd?}; import_session {path, cwd?}; fork {entryId, position?:before|at}; clone; navigate_tree {targetId, summarize?, customInstructions?, replaceInstructions?, label?, timeoutMs?}; set_entry_label {entryId, label?}; set_session_name {name}; export_html/export_jsonl {outputPath?, includeContent?}; delete_session {sessionPath}; rename_session {sessionPath, name}.

Exact export result DTO:

```ts
export interface RpcExportResult {
	path: string;
	/** File contents, present only when `includeContent` was requested. */
	content?: string;
}
```
(ice:packages/coding-agent/src/modes/rpc/rpc-types.ts:580-584).

### 6. Dependencies

`MISC-017`, `RPC-017`, `RPC-018`, `RPC-019`, `RPC-020`, `SESSION-021`, `SESSION-026`, `SESSION-031`, `SESSION-033`, `SESSION-038`

This entry owns RPC orchestration, mutation validation and response shape; the runtime, session storage, fork/navigation and actual HTML/JSONL renderers remain session-owned.

Final dependency classification. Shared contract references: `TUI-004`, `MISC-018`, `MISC-019`, `MISC-020`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

The complete native JSONL command loop has no replacement/import/fork/clone/navigation/label/export/delete/rename commands; its single supplied harness remains bound until stop/EOF. Native local session selection, branch storage and HTML export are existing features documented in SESSION, not proof of JSONL command coverage. The TCP proxy's new-session establishment is also different from these in-process replaceable runtime transitions (rust:crates/ice-cli/src/modes.rs:302-316; rust:crates/ice-cli/src/modes.rs:450-588; rust:crates/ice-cli/src/session_driver.rs:215-264).

### 8. Port notes

Adapt the native session/runtime services through the shared transition coordinator and preserve notification-before-response ordering. Implement precise existence/busy/containment/error order and durable-before-inline-export behavior. If physical path containment or different '..' filename handling is desired, document it as an intentional correction instead of describing the current TS lexical check as a full filesystem sandbox.

### 9. Size and priority

Estimated Rust implementation: **220-430 lines** (estimate, not measured source). **P1** - Session-changing commands otherwise race active work, act on stale sessions or acknowledge file/export effects with misleading rollback assumptions.

## RPC-026 - JSONL user Bash and background-shell observation/control

### 1. Title and status

JSONL user Bash and background-shell observation/control. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:149; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:641-657; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:1129-1156; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:1464-1492; ice:packages/coding-agent/src/modes/rpc/rpc-views.ts:31-61; ice:packages/coding-agent/test/suite/rpc-introspection.test.ts:283-363.

Exported symbols: `RpcShellSummary`, `RpcShellDetail`, `RpcShellsUpdateEvent` (ice:packages/coding-agent/src/modes/rpc/rpc-types.ts:641; ice:packages/coding-agent/src/modes/rpc/rpc-types.ts:643; ice:packages/coding-agent/src/modes/rpc/rpc-types.ts:684).
Exported symbols: `toShellSummary`, `listShellSummaries`, `readShellDetail` (ice:packages/coding-agent/src/modes/rpc/rpc-views.ts:31; ice:packages/coding-agent/src/modes/rpc/rpc-views.ts:36; ice:packages/coding-agent/src/modes/rpc/rpc-views.ts:44).

### 3. What it does

Run a user-origin Bash operation with extension interception and request cancellation, and expose nonconsuming background-shell snapshots plus stop/release controls.

### 4. How it works

Remote callers can run user Bash and inspect background shell activity without consuming the model's output cursor. Cancellation, process termination and foreground release remain separate operations.

bash emits user_bash first with command, excludeFromContext default false and current cwd. If an extension supplies a result, the host records that result and returns it; this branch precedes the inner requestSignal.throwIfAborted check. Otherwise it checks cancellation, then calls session.executeBash with excludeFromContext, the RPC id, extension-supplied operations and the request signal, and returns the BashResult. The outer post-handler cancellation check can still turn a late extension-supplied committed result into a cancelled response. timeoutMs is the request's overall queue-inclusive operation deadline, not a separate shell-tool timeout. abort_bash calls session.abortBash and immediately acknowledges; it is distinct from killing a durable background task (ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:1464-1492; RPC-019; SESSION-024).

get_shells maps every registry snapshot to a summary that removes output and finalText while retaining other metadata. get_shell returns null for an unknown id; otherwise retainedFrom=totalChars-output.length, start=clamp((sinceChars??0)-retainedFrom,0, output.length), output=retainedOutput.slice(start), outputOffset=retainedFrom+start and a summary that restores finalText when defined. The outer input validator requires a nonnegative safe integer sinceChars. Offsets are JavaScript string/code-unit counts, not UTF-8 byte offsets; requesting before retained history clamps to its retained start, and requesting past the end yields an empty string at totalChars. These reads do not advance the model's bash_output cursor. kill_shell invokes registry.kill(id,{notify:true}) and returns stopped:boolean even for unknown/already-stopped ids; background_bash calls releaseAll and reports the number of foreground calls released (ice:packages/coding-agent/src/modes/rpc/rpc-views.ts:31-61; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:1129-1156; ice:packages/coding-agent/src/modes/rpc/rpc-validation.ts:99-105).

Registry changes schedule at most one timer 200 ms after the first change in a burst. Later changes while pending do not restart that timer, so the constant's debounce comment is not a trailing-edge quiet-period implementation. On fire the host reads current summaries, compares their JSON to the last emitted summaries and sends shells_update only when changed. Rebind cancels the old timer/subscription and clears remembered serialization. There is no initial snapshot push merely from subscribing; clients can fetch get_shells. SESSION-025 owns background completion/watch delivery and wake, while TOOLS owns durable task/log/output/kill semantics (ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:641-657; ice:packages/coding-agent/test/suite/rpc-introspection.test.ts:301-363).

### 5. Interfaces

bash {command, excludeFromContext?, timeoutMs?} -> BashResult; abort_bash; get_shells -> {shells}; get_shell {shellId, sinceChars?} -> detail|null; kill_shell {shellId} -> {stopped}; background_bash -> {released}; shells_update {shells}.

Exact shell summary, incremental detail and update event DTOs; outputOffset counts JavaScript string units as specified above:

```ts
export type RpcShellSummary = Omit<BackgroundShellSnapshot, "output" | "finalText">;

export interface RpcShellDetail {
	shell: RpcShellSummary & { finalText?: string };
	/** Retained output from `sinceChars` onward (all of it when omitted). */
	output: string;
	/** Character offset of `output[0]` in the shell's lifetime output; pass `outputOffset + output.length` as the next `sinceChars`. */
	outputOffset: number;
}
```
(ice:packages/coding-agent/src/modes/rpc/rpc-types.ts:641-649).

```ts
export interface RpcShellsUpdateEvent {
	type: "shells_update";
	shells: RpcShellSummary[];
}
```
(ice:packages/coding-agent/src/modes/rpc/rpc-types.ts:684-687).

### 6. Dependencies

`RPC-016`, `RPC-019`, `RPC-020`, `SESSION-024`, `SESSION-025`, `TOOLS-004`, `TOOLS-005`, `TOOLS-006`, `TOOLS-008`

This entry owns RPC interception, cursor-independent views and coalesced notifications, not registry mechanics or watch fire identity handling.

### 7. ice-rust today

The native JSONL dispatch has no user bash, background shell query, kill or foreground-release commands. Native local user-shell execution in the interactive TUI and model Bash tool execution are distinct existing paths; neither supplies this protocol/registry view automatically. The complete native loop still has prompt/abort responsiveness and can carry tool events, which must be retained (rust:crates/ice-cli/src/modes.rs:450-566; rust:crates/ice-cli/src/remote/protocol.rs:214-244; SESSION-024/025; TOOLS-004/005/008).

### 8. Port notes

Wire native session user-shell execution and the background registry to these commands after the shared tools/session dependencies exist. Preserve extension interception before spawning, request-wide cancellation, nonconsuming offsets, idempotent stopped/released results, and the first-change timer plus serialized-view deduplication. Do not reuse bash_output's consumptive cursor for client observation.

### 9. Size and priority

Estimated Rust implementation: **140-270 lines** (estimate, not measured source). **P1** - Remote shell observation must not hide output from the model or confuse foreground cancellation with durable task termination.

## RPC-027 - JSONL subagent overview, approvals, controls and authorized patch wrapper

### 1. Title and status

JSONL subagent overview, approvals, controls and authorized patch wrapper. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:688-748; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:1176-1227; ice:packages/coding-agent/src/modes/rpc/rpc-validation.ts:109-117; ice:packages/coding-agent/test/suite/rpc-subagents.test.ts:125-229; ice:packages/coding-agent/test/suite/rpc-subagents.test.ts:231-485.

Exported symbols: `RpcSubagentPatchAction`, `RpcSubagentPatchResult`, `RpcSubagentsUpdateEvent` (ice:packages/coding-agent/src/modes/rpc/rpc-types.ts:689; ice:packages/coding-agent/src/modes/rpc/rpc-types.ts:692; ice:packages/coding-agent/src/modes/rpc/rpc-types.ts:702).

### 3. What it does

Expose the parent-owned subagent bridge over RPC and run user patch actions through the parent's active registered tool and extension hooks.

### 4. How it works

RPC exposes the parent-owned subagent bridge and approval decisions. Patch actions pass through the parent's active tool and extension hooks before the underlying patch authority runs.

get_subagents returns the bridge's bounded overview or null; get_subagent_messages returns its owned/redacted live, parked or historical transcript or null. The bridge is optional, and merely supplying it is what capabilities.host.subagents reports. Its ownership, bounds, lifecycle and redaction rules remain SUB-036. respond_subagent_approval validates allow-once|allow-session|deny and returns accepted:false when no bridge or unknown/expired approval; the broker owns actual authorization. subagent_control supports stop/resume/delete/send, requires a string message for send, errors if no runtime bridge exists, awaits controlSubagent and returns its message. Adapter validation does not trim an empty send string, and an optional message is passed through even for other actions (ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:1176-1227; ice:packages/coding-agent/src/modes/rpc/rpc-validation.ts:109-117; ice:packages/coding-agent/src/modes/rpc/rpc-validation.ts:240-241).

A single bridge subscription reads the current overview on each notification; null is ignored and identical consecutive JSON serializations are skipped. Otherwise it emits subagents_update {overview}. It does not emit a synthetic null update on bridge disappearance, nor an initial event solely on subscribing. This delivery deduplication is at the overview level; it is unrelated to Bash watch fireId identity or session notification handling (ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:688-697).

subagent_patch accepts inspect|apply|reject and calls runUserPatchAction. The wrapper looks up agent_patch in the parent's registered tools and checks active membership; unavailable returns success=true with data.isError=true. It creates rpc-patch-UUID and input={action, id:subagentId}, invokes tool_call hooks if present, and on a block returns isError with the supplied/default reason without running the tool or tool_result hooks. Otherwise it calls tool.execute with the synthetic call id/input. A thrown execution becomes text plus isError=true; returned content/details are used on success. Then tool_result hooks may override content/details/isError through nullish fallback. Final text concatenates only text parts without separators and retains details. Tool-call/result hook failures outside the execute catch propagate to the ordinary RPC failure response. This is an extension-hook wrapper, not a fabricated model tool turn or direct patch application bypass (ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:703-748; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:1212-1217; ice:packages/coding-agent/test/suite/rpc-subagents.test.ts:362-459).

The existing suite exercises overview deduplication, null/unknown cases, approval answers, bridge controls and patch interception. Its synthetic credential-value line 230 was connector-redacted; only the remaining 1–229 and 231–485 ranges are claimed read. No test was executed (ice:packages/coding-agent/test/suite/rpc-subagents.test.ts:125-229; ice:packages/coding-agent/test/suite/rpc-subagents.test.ts:231-485).

### 5. Interfaces

get_subagents; get_subagent_messages {subagentId}; respond_subagent_approval {approvalId, answer}; subagent_control {subagentId, action, message?}; subagent_patch {subagentId, action}; subagents_update {overview}. Tool-level failure is data.isError inside a successful subagent_patch RPC response, unlike malformed command or thrown outer-hook failure.

Exact user patch action/result and full overview event DTOs. The underlying tool/bridge retains authority:

```ts
export type RpcSubagentPatchAction = "inspect" | "apply" | "reject";

/** Outcome of an `agent_patch` run on the user's behalf, exactly as the model would see it. */
export interface RpcSubagentPatchResult {
	/** True when an extension blocked the call or the tool failed. */
	isError: boolean;
	/** Tool text: changed paths and a bounded, redacted patch preview for inspect, or the outcome. */
	text: string;
	/** Tool details, for example `{ status, changedPaths }`. */
	details?: unknown;
}

/** Emitted whenever the subagent overview changes; carries the full bounded overview. */
export interface RpcSubagentsUpdateEvent {
	type: "subagents_update";
	overview: IceSubagentOverview;
}
```
(ice:packages/coding-agent/src/modes/rpc/rpc-types.ts:689-705).

### 6. Dependencies

`EXT-024`, `RPC-016`, `RPC-018`, `RPC-020`, `SUB-008`, `SUB-016`, `SUB-019`, `SUB-031`, `SUB-032`, `SUB-036`

SUB owns child ownership/transcript policy, approval/revalidation and patch artifacts/integration. RPC must call those existing authorities through the bridge/tool hooks; it must not reconstruct authorization from UI fields.

### 7. ice-rust today

No corresponding subagent command, bridge overview/transcript query or patch wrapper exists in the complete native JSONL dispatch. Native subagent and extension machinery are real capabilities in SUB/EXT, while the native UiDialog mailbox is a separate generic interaction bridge and does not implement these parent-owned command shapes (rust:crates/ice-cli/src/modes.rs:355-394; rust:crates/ice-cli/src/modes.rs:450-566; rust:crates/ice-cli/src/remote/protocol.rs:30-41).

### 8. Port notes

Expose the native parent-owned subagent services through equivalent typed RPC commands, preserve nullable/accepted/error distinctions, and keep overview deduplication separate from lifecycle authority. Route patch actions through registered tool availability and extension hooks before SUB-031/032 enforcement; preserve blocked-call and execution-error response semantics.

### 9. Size and priority

Estimated Rust implementation: **150-290 lines** (estimate, not measured source). **P1** - A remote control surface must preserve parent ownership, approval authority and patch gates while reporting task state consistently.

## RPC-028 - JSONL extension dialog bridge and bounded UI text snapshots

### 1. Title and status

JSONL extension dialog bridge and bounded UI text snapshots. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:303-307; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:322-575; ice:packages/coding-agent/src/modes/rpc/rpc-ui-state.ts:1-75; ice:packages/coding-agent/src/modes/rpc/rpc-validation.ts:222-231; ice:packages/coding-agent/src/modes/rpc/rpc-types.ts:733-801; ice:packages/coding-agent/test/suite/rpc-shutdown-contract.test.ts:79-101; ice:packages/coding-agent/test/suite/rpc-shutdown-contract.test.ts:164-193.

Exported symbols: `RpcExtensionUIRequest`, `RpcExtensionUIResponse` (ice:packages/coding-agent/src/modes/rpc/rpc-types.ts:733; ice:packages/coding-agent/src/modes/rpc/rpc-types.ts:792).

### 3. What it does

Translate headless extension dialogs and text/chrome operations into RPC events, correlate replies, and retain a bounded data-only UI snapshot without executing terminal component factories.

### 4. How it works

The headless extension bridge sends dialogs and text-based UI updates to the client. It correlates replies and maintains a bounded snapshot without executing terminal component factories.

select/confirm/input/editor use a pending UUID map capped at 64. Shutdown, an already-aborted signal or full capacity returns the method's cancellation default without emitting a request. After registration, a signal abort or a truthy timeout cleans the map/listener/timer and resolves that default; omitted/zero timeout installs no timer. select includes optional timeout/content; confirm/input include their title/message or placeholder; editor supplies title/prefill without signal/timeout options. Matching replies clean state before resolving. select/input choose cancelled over value; confirm chooses cancelled over confirmed; editor reads value whenever present even if cancelled=true is also present. The wire validator allows a response with several valid fields, so this precedence is observable. Unknown/late ids do not resurrect requests; replies are correlation messages rather than ordinary response-producing commands. Shutdown settles pending dialogs through defaults, including during initial session_start binding (ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:322-396; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:531-535; ice:packages/coding-agent/src/modes/rpc/rpc-validation.ts:222-231; RPC-020).

Fire-and-forget extension_ui_request methods are notify, setStatus, setWorkingMessage, setWorkingVisible, setWorkingIndicator, setWidget, setHeader, setFooter, setTitle and set_editor_text. Each receives a fresh UUID but no pending reply slot. Widget factories are never executed; a function without options.rpcText is ignored, otherwise only undefined or arrays consisting of strings are emitted. Undefined widget lines clear that key. Header/footer always emit a request; undefined factory or missing rpcText yields undefined lines and clears the projected chrome. pasteToEditor falls back to setEditorText. These outgoing event objects are not clipped by the UI store; they remain subject to the global RPC output budget (ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:398-503; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:510-523; RPC-017).

The context deliberately supplies headless defaults: onTerminalInput returns a no-op unsubscriber; setHiddenThinkingLabel, addAutocompleteProvider, setEditorComponent and setToolsExpanded are no-ops; custom resolves undefined; getEditorComponent returns undefined; getEditorText returns an empty string even when the last sent editor text exists in the snapshot; getToolsExpanded returns false. The theme getter, getAllThemes and getTheme use the actual theme facilities, whereas setTheme returns a failure saying switching is unsupported. Some rpc.md claims about empty theme lists/header-footer no-ops are stale relative to this source (ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:409-412; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:452-454; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:505-575).

RpcUIStateStore observes UI requests before output and keeps only statuses, widgets, header/footer, title, editor text, working message/visibility and truncated. Strings and keys are sliced to 1024 UTF-16 units; at most 64 statuses and 64 widgets are retained, dropping oldest after insertion. A key replacement removes the old item then appends, so it moves to newest; truncated keys can collide. Widget lines and header/footer lines are limited to 32; widget placement defaults aboveEditor. Undefined status/widget/chrome values clear their entries, and truncated remains sticky until reset. setWorkingIndicator, notify and pending dialogs are not retained. get_ui_state returns a structuredClone; reset on rebind/reload removes all projected state. The snapshot bound does not retroactively bound the original event payload (ice:packages/coding-agent/src/modes/rpc/rpc-ui-state.ts:5-74; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:239-244; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:997-998).

### 5. Interfaces

extension_ui_request {id, method,...}; extension_ui_response {id, value?:string, confirmed?:boolean, cancelled?:true}; get_ui_state -> RpcUIState. select/input/editor cancellation defaultundefined; confirm defaultfalse. Correlation, timeout and snapshot limits are independent of ordinary command admission.

### 6. Dependencies

`RPC-016`, `RPC-017`, `RPC-020`, `TUI-004`

EXT owns the host UI API and extension contexts; RPC owns this mode-specific bridge/defaults/snapshot. Theme resolution stays TUI-owned; no terminal renderer is introduced into RPC.

Final dependency classification. Shared contract references: `EXT-019`, `EXT-022`, `EXT-009`, `EXT-040`, `EXT-041`, `EXT-042`, `EXT-043`, `EXT-044`, `EXT-049`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

Native RPC already attaches a UiDialogMailbox, polls it every 30 ms, emits ui_dialog_request with requestId/toolCallId/ui and ui_dialog_closed notifications, accepts ui_dialog_response with cancelled or answer (defaultNull), and keeps reading while a prompt is active. Its acknowledgment does not itself prove a provider/extension accepted a late answer. Native prompt completion cancels outstanding dialogs. These are genuine generic dialog capabilities, including an explicit closed notification absent from this TS adapter, but they differ from the TS method-specific value/confirmed protocol and do not supply the UI text/chrome snapshot or headless method defaults (rust:crates/ice-cli/src/modes.rs:355-394; rust:crates/ice-cli/src/modes.rs:450-480; rust:crates/ice-cli/src/modes.rs:527-538; rust:crates/ice-cli/src/remote/protocol.rs:30-41).

### 8. Port notes

Adapt native mailbox interactions to the TS dialog shapes and preserve reply precedence, cancellation defaults and input-before-bootstrap readiness. Add the bounded UI text store and data-only chrome projections without evaluating component factories. Retain native closed notifications as an explicit extra if compatibility clients tolerate them, and preserve real theme lookup while keeping unsupported mode operations explicit.

### 9. Size and priority

Estimated Rust implementation: **220-420 lines** (estimate, not measured source). **P1** - Headless extensions otherwise hang on unanswered dialogs or lose UI state, while accidentally executing terminal factories would violate the mode contract.

## RPC-029 - JSONL settings catalog, effective-value projection and diagnostics

### 1. Title and status

JSONL settings catalog, effective-value projection and diagnostics. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/modes/rpc/rpc-settings.ts:14-71; ice:packages/coding-agent/src/modes/rpc/rpc-settings.ts:115-186; ice:packages/coding-agent/src/modes/rpc/rpc-settings.ts:224-838; ice:packages/coding-agent/src/modes/rpc/rpc-settings.ts:888-906; ice:packages/coding-agent/src/modes/rpc/rpc-settings.ts:963-1080; ice:packages/coding-agent/src/subagents/permissions/approval.ts:15; ice:packages/coding-agent/test/rpc-settings.test.ts:1-419.

Exported symbols: `RpcSettingsFieldKind`, `RpcSettingsFieldScope`, `RpcSettingsValue`, `RpcSettingsConstraints`, `RpcSettingsField`, `RpcSettingsDiagnostic`, `RpcSettingsContext`, `createRpcSettingsSnapshot` (ice:packages/coding-agent/src/modes/rpc/rpc-settings.ts:14; ice:packages/coding-agent/src/modes/rpc/rpc-settings.ts:15; ice:packages/coding-agent/src/modes/rpc/rpc-settings.ts:16; ice:packages/coding-agent/src/modes/rpc/rpc-settings.ts:18; ice:packages/coding-agent/src/modes/rpc/rpc-settings.ts:44; ice:packages/coding-agent/src/modes/rpc/rpc-settings.ts:46; ice:packages/coding-agent/src/modes/rpc/rpc-settings.ts:66; ice:packages/coding-agent/src/modes/rpc/rpc-settings.ts:1071).

### 3. What it does

Expose a versioned settings form with 51 built-in fields plus simple extension settings, preserving global configured values, effective values, trust/source metadata, defaults, constraints and read-side diagnostic consumption.

### 4. How it works

The settings endpoint exposes configured values alongside effective values and their scope. Its complete field catalog also carries validation metadata, trust decisions and diagnostics consumed by the read.

get_settings returns protocolVersion 1, cwd, projectTrusted, fields and diagnostics. Each field carries key/label/description/group/kind/scope/value/effectiveValue/projectOverride/effectiveSource/restartRequired, with optional hostOnly/options/constraints. value is the normalized global configured value or the definition default, not the currently selected scope or project value; effectiveValue is the definition's runtime getter normalized for display. projectOverride reports a project path when the field is not global-only, unless global-first precedence has a global value; this flag itself does not require project trust. effectiveSource normally uses the manager, with an ICE-subagent special case: a trusted project value equal to the effective value wins the source label, otherwise a present global path wins (ice:packages/coding-agent/src/modes/rpc/rpc-settings.ts:178-186; ice:packages/coding-agent/src/modes/rpc/rpc-settings.ts:1008-1037).

Snapshot normalization is weaker than write validation. Missing/wrongly typed values fall back to defaults; select values must be declared options; numeric values need only be finite, without applying integer/range constraints; text needs only string type; string lists use the common 4096-code-unit/NUL-free item predicate, without each field's maxItems/maxLength; package sources use their full structural predicate. This is a display projection, not proof that every stored value is valid for mutation (ice:packages/coding-agent/src/modes/rpc/rpc-settings.ts:840-906).

ICE subagent effective values come from the V2 resolver, including trusted-project narrowing and diagnostics; enabled is false whenever the resolver reports diagnostics. Hooks are false if the raw global setting explicitly disables them, otherwise global OR trusted-project enabled, and any parse failure produces false. The default approval timeout is 600000 ms. These security-sensitive rules are delegated to SUB-005/008 rather than ordinary last-writer preference merging (ice:packages/coding-agent/src/modes/rpc/rpc-settings.ts:132-175; ice:packages/coding-agent/src/subagents/permissions/approval.ts:15).

The complete built-in form inventory follows. Source-line numbers refer to rpc-settings.ts; both means global and project writes are supported subject to trust. restartRequired/hostOnly are metadata and do not make a field read-only. The listed values are this RPC definition's fallbacks, which can differ from other UI/runtime defaults:

| Key (source line) | Kind | Default | Scope | Constraint / options | Restart / host only |
| --- | --- | --- | --- | --- | --- |
| ice.subagents.enabled (226) | boolean | true | both | — | false / false |
| ice.subagents.defaultAgent (238) | text | "general" | both | { maxLength: 64 } | false / false |
| ice.subagents.maxConcurrent (250) | number | 4 | both | { min: 1, integer: true } | true / false |
| ice.subagents.maxDepth (262) | number | 0 | both | { min: 0, max: 4, integer: true } | false / false |
| ice.subagents.askTimeoutMs (275) | number | 600000 | global | { min: 1_000, max: 60 * 60 * 1000, integer: true } | true / false |
| ice.subagents.disabledAgents (287) | string-list | [] | both | { maxItems: 64, maxLength: 64 } | false / false |
| ice.hooks.enabled (299) | boolean | false | both | — | false / false |
| defaultProvider (315) | text | "" | both | { maxLength: 256 } | true / false |
| defaultModel (327) | text | "" | both | { maxLength: 512 } | true / false |
| defaultThinkingLevel (339) | select | "off" | both | off, minimal, low, medium, high, xhigh, max, ultra | false / false |
| fastMode (351) | boolean | false | both | — | false / false |
| transport (362) | select | "auto" | both | auto, sse, websocket | false / false |
| steeringMode (374) | select | "one-at-a-time" | both | all, one-at-a-time | false / false |
| followUpMode (386) | select | "one-at-a-time" | both | all, one-at-a-time | false / false |
| compaction.enabled (398) | boolean | true | both | — | false / false |
| compaction.thresholdPercent (409) | number | 85 | both | { min: 1, max: 99, integer: true } | false / false |
| compaction.midRunCompaction (421) | select | "off" | both | off, pause, resume | false / false |
| compaction.reserveTokens (433) | number | 16384 | both | { min: 1, integer: true } | false / false |
| compaction.keepRecentTokens (445) | number | 20000 | both | { min: 1, integer: true } | false / false |
| branchSummary.reserveTokens (457) | number | 16384 | both | { min: 1, integer: true } | false / false |
| branchSummary.skipPrompt (469) | boolean | false | both | — | false / false |
| retry.enabled (480) | boolean | true | both | — | false / false |
| quietStartup (491) | boolean | false | both | — | false / false |
| enableInstallTelemetry (502) | boolean | true | global | — | false / false |
| collapseChangelog (513) | boolean | false | both | — | false / false |
| enableSkillCommands (524) | boolean | true | both | — | true / false |
| enabledModels (535) | string-list | [] | both | { maxItems: 1000, maxLength: 512 } | true / false |
| packages (547) | package-sources | [] | both | { maxItems: 100 } | true / false |
| doubleEscapeAction (559) | select | "tree" | both | fork, tree, none | false / false |
| treeFilterMode (571) | select | "default" | both | default, no-tools, user-only, labeled-only, all | false / false |
| defaultProjectTrust (583) | select | "ask" | global | ask, always, never | false / false |
| warnings.anthropicExtraUsage (595) | boolean | true | both | — | false / false |
| terminal.showImages (606) | boolean | true | both | — | true / true |
| terminal.imageWidthCells (618) | number | 60 | both | { min: 1, max: 1000, integer: true } | true / true |
| terminal.clearOnShrink (631) | boolean | false | both | — | true / true |
| terminal.showTerminalProgress (643) | boolean | false | both | — | true / true |
| images.autoResize (655) | boolean | true | both | — | true / false |
| images.blockImages (666) | boolean | false | both | — | true / false |
| hideThinkingBlock (677) | boolean | false | both | — | true / true |
| showCacheMissNotices (689) | boolean | false | both | — | true / true |
| editorPaddingX (701) | number | 0 | both | { min: 0, max: 3, integer: true } | true / true |
| outputPad (714) | number | 1 | both | { min: 0, max: 1, integer: true } | true / true |
| autocompleteMaxVisible (727) | number | 5 | both | { min: 3, max: 20, integer: true } | true / true |
| uiMode (740) | select | "regular" | both | regular, fullscreen | true / true |
| fullscreenScrollbar (753) | select | "hidden" | both | auto, always, hidden | true / true |
| showHardwareCursor (766) | boolean | false | both | — | true / true |
| favoriteModels (778) | string-list | [] | global | { maxItems: 1000, maxLength: 512 } | false / false |
| httpIdleTimeoutMs (790) | number | 300000 | global | { min: 0, integer: true } | true / false |
| enableAnalytics (802) | boolean | false | global | — | false / false |
| footerStyle (813) | select | "compact" | global | ["compact", "legacy"] | true / true |
| toolStyle (826) | select | "compact" | global | ["compact", "boxed"] | true / true |

The option arrays are declared at 115–123; the definition list and its runtime getters occupy 224–838. This table includes every built-in key, not merely mechanically extracted interface property names (ice:packages/coding-agent/src/modes/rpc/rpc-settings.ts:115-123; ice:packages/coding-agent/src/modes/rpc/rpc-settings.ts:224-838).

Simple registered extension items are appended in registration/item order, skipping submenus and missing/empty values. The key is extension.<settings.name>.<item.id>; exactly two values containing true and false produce a boolean, otherwise a select. The scope is global, restartRequired=false, and currentValue supplies both fallback and effective value; descriptions fall back to the item's label. The underlying callback remains extension-owned (ice:packages/coding-agent/src/modes/rpc/rpc-settings.ts:963-1006).

Diagnostics are ordered settings-load parse failures, invalid global ICE policy, invalid trusted-project ICE policy, V2 deprecations, then drained settings I/O errors. Messages are generic for load/policy/I/O failures, while deprecation messages come from the resolver. Creating a settings snapshot drains write errors; get_diagnostics in RPC-022 does not. This read-side consumption also occurs when generating a structured settings schema. It must not be described as an immutable/nonconsuming settings inspection (ice:packages/coding-agent/src/modes/rpc/rpc-settings.ts:1039-1080; RPC-031).

### 5. Interfaces

get_settings -> RpcSettingsSnapshot; field kinds boolean/select/number/text/string-list/package-sources; scopes global/project/both; diagnostics settings_io_error/settings_parse_error/ice_policy_error/ice_deprecated_setting. The 51-key catalog above is exhaustive for built-ins; extension keys are dynamic.

Exact settings field/value/constraint/diagnostic/context declarations; RpcSettingsField aliases the field base without adding runtime validation:

```ts
export type RpcSettingsFieldKind = "boolean" | "select" | "number" | "text" | "string-list" | "package-sources";
export type RpcSettingsFieldScope = SettingsScope | "both";
export type RpcSettingsValue = boolean | string | number | string[] | PackageSource[];

export interface RpcSettingsConstraints {
	min?: number;
	max?: number;
	integer?: boolean;
	maxItems?: number;
	maxLength?: number;
}

interface RpcSettingsFieldBase {
	key: string;
	label: string;
	description: string;
	group: string;
	kind: RpcSettingsFieldKind;
	scope: RpcSettingsFieldScope;
	projectOverride: boolean;
	/** Preference source; ICE restrictions may further narrow effectiveValue. */
	effectiveSource?: SettingsScope;
	restartRequired: boolean;
	hostOnly?: boolean;
	value: RpcSettingsValue;
	effectiveValue: RpcSettingsValue;
	options?: string[];
	constraints?: RpcSettingsConstraints;
}

export type RpcSettingsField = RpcSettingsFieldBase;

export interface RpcSettingsDiagnostic {
	code: "settings_io_error" | "settings_parse_error" | "ice_policy_error" | "ice_deprecated_setting";
	scope: SettingsScope;
	message: string;
}
```
(ice:packages/coding-agent/src/modes/rpc/rpc-settings.ts:14-50).

```ts
export interface RpcSettingsContext {
	cwd: string;
	settingsManager: SettingsManager;
	getExtensionSettings?: () => RegisteredSettings[];
	applyLive?: () => void;
}
```
(ice:packages/coding-agent/src/modes/rpc/rpc-settings.ts:66-71).

### 6. Dependencies

`OBS-001`, `RPC-016`, `RPC-022`, `RPC-030`, `RPC-031`, `SET-003`, `SET-011`, `SUB-003`, `SUB-005`, `SUB-007`, `SUB-008`

SET owns persistence/default policy, SUB owns narrowing/delegation safety, OBS owns install telemetry, and RPC owns this catalog/projection/diagnostic consumption. Do not infer native key support from a UI label alone. SUB-003 owns trust-filtered source discovery and SUB-007 owns source-sensitive permission ceilings; this settings projection must preserve their authority decisions rather than infer permission from display metadata.

### 7. ice-rust today

The native JSONL command dispatch exposes neither get_settings nor an equivalent versioned field catalog/effective-source/diagnostic response. Native configuration storage and local TUI controls are credited in SET/TUI, but their existence does not supply the RPC global-versus-effective projection,51-field catalog or extension item adapters (rust:crates/ice-cli/src/modes.rs:450-566; rust:crates/ice-cli/src/modes.rs:698-769).

### 8. Port notes

Project native settings through the complete typed catalog and existing shared policy resolvers. Preserve configured/effective distinctions, host/restart markers, simple extension enumeration and exact constraints/options. Make diagnostic draining explicit so repeated readers and form revision generation have predictable effects; if changing that consumption, document the difference.

### 9. Size and priority

Estimated Rust implementation: **260-480 lines** (estimate, not measured source). **P1** - A remote settings form must show the actual scope and effective policy rather than silently enabling invalid or untrusted delegation.

## RPC-030 - JSONL settings mutation validation, persistence and live-apply ordering

### 1. Title and status

JSONL settings mutation validation, persistence and live-apply ordering. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:750-755; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:1162-1170; ice:packages/coding-agent/src/modes/rpc/rpc-settings.ts:73-97; ice:packages/coding-agent/src/modes/rpc/rpc-settings.ts:189-221; ice:packages/coding-agent/src/modes/rpc/rpc-settings.ts:861-882; ice:packages/coding-agent/src/modes/rpc/rpc-settings.ts:912-995; ice:packages/coding-agent/src/modes/rpc/rpc-settings.ts:1082-1120; ice:packages/coding-agent/test/rpc-settings.test.ts:1-419.

Exported symbols: `RpcSettingUpdate`, `RpcSettingsErrorCode` (ice:packages/coding-agent/src/modes/rpc/rpc-settings.ts:60; ice:packages/coding-agent/src/modes/rpc/rpc-settings.ts:73).

### 3. What it does

Apply one validated settings update in the requested scope, await persistence, inspect diagnostics, then synchronize the live session and return a fresh form snapshot.

### 4. How it works

A settings update validates scope, trust and value before changing the selected setting. Persistence and live application occur in order, so a reported error can follow an already committed effect.

set_setting selects command.data whenever that property exists, otherwise the flattened command object; the outer RPC validator intentionally declares no setting fields and delegates authoritative validation to applyRpcSetting. It does not validate the nested value as a record first, so malformed null/undefined payloads can throw a generic failure while reading update.scope. The ordered normal path validates scope global|project, finds the first definition for key, rejects project scope on global-only definitions, rejects any project write when untrusted, then validates the value. The corresponding typed codes are invalid_scope, unknown_key, scope_not_supported and project_untrusted; their order is observable (ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:1166-1170; ice:packages/coding-agent/src/modes/rpc/rpc-validation.ts:107-108; ice:packages/coding-agent/src/modes/rpc/rpc-settings.ts:1082-1099).

Boolean requires boolean (invalid_type). Select requires string then declared membership (invalid_type/invalid_option). Number requires finite number, then min/max and Number.isInteger where requested (invalid_type/out_of_range); integer is not safe-integer validation. Text requires string then forbids NUL and exceeds-maxLength (invalid_type/invalid_value). String-list first uses the common string/NUL/4096-unit predicate (invalid_type), then maxItems/field item-length checks (invalid_value), and clones the array. Package sources allow at most 100 elements; each is a NUL-free string of at most 4096 units, including empty strings, or an object with only source/autoload/extensions/skills/prompts/themes, a safe source string, optional boolean autoload and optional safe string arrays. Filter arrays have no separate element-count cap. Packages are structured-cloned. These precise distinctions are stricter than snapshot normalization (ice:packages/coding-agent/src/modes/rpc/rpc-settings.ts:861-885; ice:packages/coding-agent/src/modes/rpc/rpc-settings.ts:912-961).

Ordinary built-ins call settings.setSettingValue(scope, key, value). ICE namespace updates clone the existing scoped ice object, descend the declared key path, reject a nonobject intermediate with invalid_value, clone the leaf value, validate the entire prospective namespace through parseIceSettings and only then setIceSettingsValue. Unrelated ICE fields survive. Extension updates synchronously call settings.onChange(item.id, serializedValue) then update item.currentValue; synchronous callback failures become extension_change_failed with a generic message. The function does not await an extension callback promise (ice:packages/coding-agent/src/modes/rpc/rpc-settings.ts:125-129; ice:packages/coding-agent/src/modes/rpc/rpc-settings.ts:189-221; ice:packages/coding-agent/src/modes/rpc/rpc-settings.ts:963-994).

After definition.set, applyRpcSetting awaits manager.flush. A typed RpcSettingsError survives; other thrown write/flush errors become persistence_failed. It then gathers the same diagnostics as get_settings, draining write errors. Any diagnostic at all—including an existing parse/policy diagnostic or an ICE deprecation—causes persistence_failed using the first diagnostic's message/scope. This check occurs after the setter/flush; the source does not roll back a setting already saved or an extension callback already run. Only a clean diagnostic list proceeds to context.applyLive (session.syncSettingsFromManager), then a fresh snapshot. applyLive is outside the persistence catch. Direct set_setting has no explicit streaming/compacting busy check, unlike structured /settings invocation; both still obey RPC coordinator admission (ice:packages/coding-agent/src/modes/rpc/rpc-settings.ts:1082-1120; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:750-755; RPC-031).

### 5. Interfaces

set_setting accepts flattened {key, scope, value} or a data payload of that shape. RpcSettingsError codes: unknown_key, invalid_scope, scope_not_supported, project_untrusted, invalid_type, invalid_option, out_of_range, invalid_value, extension_change_failed, persistence_failed. The RPC failure preserves the code and key/scope details; success returns a complete snapshot.

Exact settings update and typed error-code declarations:

```ts
export interface RpcSettingUpdate {
	key: string;
	scope: SettingsScope;
	value: unknown;
}
```
(ice:packages/coding-agent/src/modes/rpc/rpc-settings.ts:60-64).

```ts
export type RpcSettingsErrorCode =
	| "unknown_key"
	| "invalid_scope"
	| "scope_not_supported"
	| "project_untrusted"
	| "invalid_type"
	| "invalid_option"
	| "out_of_range"
	| "invalid_value"
	| "extension_change_failed"
	| "persistence_failed";
```
(ice:packages/coding-agent/src/modes/rpc/rpc-settings.ts:73-83).

### 6. Dependencies

`RPC-016`, `RPC-018`, `RPC-029`, `RPC-031`, `SET-003`, `SET-007`, `SET-009`, `SUB-005`

Underlying file writes and global/project policy are SET-owned. This entry owns validation order, namespace/extension adaptation and the saved-before-diagnostics/live-apply boundary.

### 7. ice-rust today

There is no set_setting JSONL command, typed settings failure family or field-catalog update path in the complete native RPC loop. Native configuration/TUI setting writes are real existing operations, but must be wrapped deliberately; they do not automatically provide this layered validation, deferred flush or complete post-update projection (rust:crates/ice-cli/src/modes.rs:450-566).

### 8. Port notes

Implement the adapter over native settings storage with exact scope/trust/type validation and explicit live synchronization. Preserve error metadata and distinguish validation rejection from a failure reported after an effect committed. Review whether unrelated diagnostics should block live apply after a successful save; if correcting that current TS behavior, document it and avoid claiming transactional rollback.

### 9. Size and priority

Estimated Rust implementation: **190-360 lines** (estimate, not measured source). **P1** - Clients need trustworthy scope validation and must not assume an error means a setting was never persisted.

## RPC-031 - Structured slash-command inventory, schemas, revisions and invocation

### 1. Title and status

Structured slash-command inventory, schemas, revisions and invocation. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/modes/rpc/rpc-command-schema.ts:1-705; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:191-210; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:765-786; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:1773-2049; ice:packages/coding-agent/test/rpc-command-schema.test.ts:1-455; ice:packages/coding-agent/test/legacy-compat/runtime-contracts.test.ts:119-147.

Exported symbols: `RpcCommandSource`, `RpcCommandOption`, `RpcCommandFieldValueType`, `RpcCommandFieldBase`, `RpcBooleanCommandField`, `RpcIntegerCommandField`, `RpcNumberCommandField`, `RpcStringCommandField`, `RpcEnumCommandField`, `RpcStringListCommandField`, `RpcCommandField`, `RpcCommandGroup`, `RpcCommandSchema`, `RpcCommandSchemaResult`, `RpcCommandInvocationResult`, `RpcCommandErrorDetails`, `RpcCommandExecutionError`, `COMMAND_FIELD_KEY_PATTERN`, `ICE_SETTINGS_COMMAND_NAME`, `ICE_SETTINGS_SCHEMA_ID`, `buildIceSettingsCommandSchema`, `getIceSettingsCommandSchemaResult`, `invokeIceSettingsCommand`, `CustomStructuredCommand`, `mergeCustomStructuredCommandInventory`, `validateStructuredInvocation`, `registerCustomStructuredCommand`, `clearCustomStructuredCommands` (ice:packages/coding-agent/src/modes/rpc/rpc-command-schema.ts:12; ice:packages/coding-agent/src/modes/rpc/rpc-command-schema.ts:14; ice:packages/coding-agent/src/modes/rpc/rpc-command-schema.ts:20; ice:packages/coding-agent/src/modes/rpc/rpc-command-schema.ts:22; ice:packages/coding-agent/src/modes/rpc/rpc-command-schema.ts:35; ice:packages/coding-agent/src/modes/rpc/rpc-command-schema.ts:41; ice:packages/coding-agent/src/modes/rpc/rpc-command-schema.ts:49; ice:packages/coding-agent/src/modes/rpc/rpc-command-schema.ts:58; ice:packages/coding-agent/src/modes/rpc/rpc-command-schema.ts:67; ice:packages/coding-agent/src/modes/rpc/rpc-command-schema.ts:74; ice:packages/coding-agent/src/modes/rpc/rpc-command-schema.ts:82; ice:packages/coding-agent/src/modes/rpc/rpc-command-schema.ts:90; ice:packages/coding-agent/src/modes/rpc/rpc-command-schema.ts:96; ice:packages/coding-agent/src/modes/rpc/rpc-command-schema.ts:109; ice:packages/coding-agent/src/modes/rpc/rpc-command-schema.ts:115; ice:packages/coding-agent/src/modes/rpc/rpc-command-schema.ts:121; ice:packages/coding-agent/src/modes/rpc/rpc-command-schema.ts:137; ice:packages/coding-agent/src/modes/rpc/rpc-command-schema.ts:153; ice:packages/coding-agent/src/modes/rpc/rpc-command-schema.ts:154; ice:packages/coding-agent/src/modes/rpc/rpc-command-schema.ts:155; ice:packages/coding-agent/src/modes/rpc/rpc-command-schema.ts:251; ice:packages/coding-agent/src/modes/rpc/rpc-command-schema.ts:292; ice:packages/coding-agent/src/modes/rpc/rpc-command-schema.ts:310; ice:packages/coding-agent/src/modes/rpc/rpc-command-schema.ts:389; ice:packages/coding-agent/src/modes/rpc/rpc-command-schema.ts:414; ice:packages/coding-agent/src/modes/rpc/rpc-command-schema.ts:445; ice:packages/coding-agent/src/modes/rpc/rpc-command-schema.ts:673; ice:packages/coding-agent/src/modes/rpc/rpc-command-schema.ts:696).
Exported symbols: `RpcSlashCommand` (ice:packages/coding-agent/src/modes/rpc/rpc-types.ts:192).

### 3. What it does

Discover slash commands with source identity, expose a versioned single-field form schema where supported, and reject ambiguous/stale/invalid/busy structured invocations before invoking their handler.

### 4. How it works

Structured commands expose source identity, a form schema and a revision token. Invocation checks ambiguity, staleness, scope and value before calling the chosen handler.

get_commands refreshes skills first. It returns built-in settings as form schema ice.settings, then only built-ins having a direct RPC mapping, then extension invocation names, prompt templates and skill:<name> entries with sourceInfo. The direct mapping is fast→set_fast_mode, model→set_model, scoped-models→set_scoped_models, export→export_html, import→import_session, name→set_session_name, session→get_session_stats, changelog→get_changelog, shells→get_shells, resources→get_resources, fork→fork, clone→clone, tree→navigate_tree, new→new_session, compact→compact, resume→switch_session and reload→reload. A module-global custom registry decorates matching source:name entries with form metadata and appends unseen custom entries while preserving base order; discovery does not turn every direct RPC mapping into a structured schema (ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:191-210; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:1773-1829; ice:packages/coding-agent/src/modes/rpc/rpc-command-schema.ts:404-443).

Settings schema mapping permits only field keys matching ^[A-Za-z0-9._-]+$. Boolean/select/number/text/string-list map to boolean/enum/integer-or-number/string/string-list; integer display values use Math.round. both scope maps to either. Options become value/label pairs. package-sources and invalid keys are omitted with a dedicated-page diagnostic. The mapping carries label/description/group/projectOverride/restartRequired but drops hostOnly/effectiveSource and string-list item maxLength; authoritative settings mutation still validates those omitted constraints. Groups preserve first encounter order. Revision is unsigned base36 FNV-1a32 over JSON.stringify({commandName, schemaId, cwd, projectTrusted, fields}), using UTF-16 charCodeAt and Math.imul; it is a change token, not a cryptographic authenticity proof. Schema protocolVersion 1, name=settings, id=ice.settings, invocationMode=single-field, fields/groups/diagnostics are returned as available, or failed with schema:null and error diagnostics when generation throws (ice:packages/coding-agent/src/modes/rpc/rpc-command-schema.ts:153-308).

get_command_schema first rejects a name ambiguous across source categories when source is omitted. It collects builtin-settings plus matching custom structured candidates; multiple candidates remain ambiguous even with a source selector. A builtin/custom schema result passes through or is wrapped as available; a truthy supplied schemaId differing from a present schema returns stale with expected id/revision. A known raw extension/prompt/skill command without a schema returns success with capability=unsupported and schema:null; multiple raw matches are ambiguous, and an absent match is not_found. Non-settings built-ins listed only through rpcCommand do not enter that raw fallback. Custom getSchema is called synchronously; its declared shape is not independently schema-compiled here (ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:1831-1945).

Generic validation checks truthy schemaId then truthy revision for stale, requires exactly one own enumerable argument for single-field mode, finds its field (not_found), rejects readOnly, then validates scope before value. either requires explicit scope; fixed global/project rejects the opposite; unscoped rejects a scope option. It does not coerce values. Integer uses Number.isInteger and min/max, number uses finite/min/max, enum requires exact declared string, string uses UTF-16 length bounds, and string-list checks string members and count bounds. The number step and required metadata are not enforced, and list elements have no generic maxLength check. The fallback for a runtime schema with another invocationMode only requires nonempty arguments and validates its first field; the declared/response-validated public schema supports single-field only. Preserve this implementation limit rather than claiming general multi-field validation (ice:packages/coding-agent/src/modes/rpc/rpc-command-schema.ts:445-670).

invoke_command repeats source/candidate ambiguity resolution. For settings, streaming/compacting is rejected before schema generation; arguments are copied with defineProperty into own data fields, current schema is generated, stale/generic validation runs, then applyRpcSetting performs the second authoritative constraint/trust/persistence phase. project_untrusted becomes trust; every other RpcSettingsError—including persistence_failed—maps to validation with field/scope. Successful settings invocation returns status=updated, new schema and a message. For custom commands, busy is checked before getSchema, null schema is unsupported, generic validation runs, then invoke receives the original arguments/options and is awaited. Typed command errors preserve code/details; other failures become failed. With no structured candidate, invoke_command returns unsupported, even for a wholly unknown name, whereas get_command_schema returns not_found (ice:packages/coding-agent/src/modes/rpc/rpc-command-schema.ts:310-387; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:1947-2049).

Custom registration is unique per name/source pair and throws on duplicates; unregister removes by object identity so a stale cleanup cannot remove a later replacement, and clear removes all. listCustomStructuredCommands flattens to a new array; findCustomStructuredCommands without source returns the stored array itself, while a source filter allocates a new one. These are synchronous process-local registration APIs, separate from extension command registration and binary protocol methods (ice:packages/coding-agent/src/modes/rpc/rpc-command-schema.ts:389-411; ice:packages/coding-agent/src/modes/rpc/rpc-command-schema.ts:673-704).

### 5. Interfaces

get_commands; get_command_schema {name, source?, schemaId?}; invoke_command {name, source?, schemaId?, schemaRevision?, arguments, options?:{scope?}}. Sources builtin/extension/prompt/skill. Field types boolean/integer/number/string/enum/string-list. Schema capability available/unsupported/failed; invocation status updated/completed/accepted. Error codes unsupported/not_found/ambiguous/validation/stale/busy/trust/failed.

Exact discovery DTO added for the previously uncited RpcSlashCommand export:

```ts
export interface RpcSlashCommand {
	/** Command name (without leading slash) */
	name: string;
	/** Human-readable description */
	description?: string;
	/** Argument syntax hint, when provided by the source. */
	argumentHint?: string;
	/** What kind of command this is */
	source: RpcCommandSource;
	/** Source metadata for the owning resource */
	sourceInfo?: SourceInfo;
	/** Declarative structured interaction metadata */
	interaction?: { type: "form"; schemaId: string } | { type: "text" };
	/** Built-in interactive command: the RPC command that performs it. Sending it as a prompt does not run it. */
	rpcCommand?: RpcCommandType;
}
```
(ice:packages/coding-agent/src/modes/rpc/rpc-types.ts:192-207).

### 6. Dependencies

`RPC-016`, `RPC-018`, `RPC-022`, `RPC-029`, `RPC-030`

RPC owns this structured-command registry/schema layer. Extension command naming/provenance and actual settings effects remain in their owners; this validator is distinct from AI tool JSON-schema validation.

Final dependency classification. Shared contract references: `EXT-019`, `EXT-013`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

No get_commands/get_command_schema/invoke_command family or corresponding structured form registry is exposed by the native JSONL dispatch. Native extension slash commands and TUI settings are existing capabilities, but are not equivalent to these source-qualified RPC schema/revision/invocation contracts (rust:crates/ice-cli/src/modes.rs:450-566).

### 8. Port notes

Implement source-qualified discovery, the complete form schema mapping and revision token, then validate with the documented order before calling native settings or extension handlers. Preserve unknown-versus-unsupported distinctions, busy checks and persistence-error remapping. Do not promise unsupported multi-field or step validation; if hardening malformed custom schemas or returning copied registry lists, mark the change explicitly.

### 9. Size and priority

Estimated Rust implementation: **300-570 lines** (estimate, not measured source). **P1** - Structured clients need stable identity/revision checks and authoritative validation to avoid applying an update to the wrong command, source or current schema.

## RPC-032 - JSONL child-process client startup, handshake and termination

### 1. Title and status

JSONL child-process client startup, handshake and termination. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/modes/rpc/rpc-client.ts:78-155; ice:packages/coding-agent/src/modes/rpc/rpc-client.ts:160-323; ice:packages/coding-agent/test/rpc-client-contract.test.ts:258-313; ice:packages/coding-agent/test/rpc-client-contract.test.ts:376-408; ice:packages/coding-agent/test/rpc-client-process-exit.test.ts:1-45.

Exported symbols: `RpcClientOptions`, `ModelInfo` (ice:packages/coding-agent/src/modes/rpc/rpc-client.ts:78; ice:packages/coding-agent/src/modes/rpc/rpc-client.ts:103).

### 3. What it does

Expose a reusable host SDK that spawns the coding-agent JSONL process, proves its capability handshake before start resolves, retains bounded diagnostics, and rejects all pending requests and waits when the child fails or stops.

### 4. How it works

The host client starts a child process and verifies its capability response before reporting readiness. Child failure or stop rejects outstanding work and retains bounded diagnostics for the caller.

start rejects whenever a process is already attached. startupTimeoutMs defaults to 30000 and must be a positive safe integer no greater than 2147483647. It resets the exit error and stderr tail, then spawns node with the selected cliPath (default dist/ice.js), --mode rpc, truthy provider/model flags and extra arguments in that order. cwd is passed through; custom environment keys overlay process.env, and all three standard streams are piped. Request/startup counters and registered event listeners survive stop/start; this is not a fresh logical client object on each launch (ice:packages/coding-agent/src/modes/rpc/rpc-client.ts:136-189).

stderr chunks append to a tail of the latest 65536 JavaScript characters while the complete incoming chunk is forwarded to host stderr. Exit, child error and stdin error handlers check that their child still owns this.process, retain the first exit error and reject pending work/termination observers. stdout uses the LF reader with a 32-MiB record limit. A too-large stdout record records output_too_large, rejects pending work and asynchronously starts stop. This is a fatal client transport condition, unlike the server input reader's recoverable rejection of one overlarge line (ice:packages/coding-agent/src/modes/rpc/rpc-client.ts:113-114; ice:packages/coding-agent/src/modes/rpc/rpc-client.ts:193-235).

Startup sends get_capabilities with its own monotonically increasing startup_<n> ID and configured handshake deadline. Ordinary response validation runs first, then startup explicitly requires protocolVersion=1, string command and feature arrays, get_capabilities in commands, a positive safe integer maxInputLineBytes, and boolean host.subagents. A mismatch, server failure or timeout awaits stop before rejecting. Startup does not sleep a fixed initialization delay, auto-retry, fall back to an older protocol, or adopt negotiated limits into the client's hard-coded request budget (ice:packages/coding-agent/src/modes/rpc/rpc-client.ts:237-274; ice:packages/coding-agent/test/rpc-client-contract.test.ts:258-313).

stop is a no-op without an attached child. Otherwise it installs a stopped error only if no earlier error exists, rejects all requests/waiters, detaches stdout and, if the process has not exited, sends SIGTERM. It waits for exit or 1000 ms; the timer sends SIGKILL and resolves without awaiting acknowledgement of that kill. The process reference is cleared only if it still refers to the stopped child. An ordinary exit callback alone does not clear the reference, so callers stop before restarting after a crash. Bounded stderr remains available through getStderr after stop; the next start clears it. Pending timer/admission cleanup runs through request rejection, and active event collectors/idle waiters are also terminated (ice:packages/coding-agent/src/modes/rpc/rpc-client.ts:280-305; ice:packages/coding-agent/src/modes/rpc/rpc-client.ts:319-323; ice:packages/coding-agent/src/modes/rpc/rpc-client.ts:1132-1140; ice:packages/coding-agent/test/rpc-client-contract.test.ts:376-408).

### 5. Interfaces

RpcClient(options), start():Promise<void>, stop():Promise<void>, getStderr():string. Options cliPath/cwd/env/provider/model/args/startupTimeoutMs/onListenerError. start resolving proves the get_capabilities handshake, not provider authentication or completion of a prompt.

Exact child-process SDK options and compact model metadata DTO. The latter is an exported TypeScript interface, not an independent runtime payload validator:

```ts
export interface RpcClientOptions {
	/** Path to the ICE CLI entry point (default: searches for dist/ice.js) */
	cliPath?: string;
	/** Working directory for the agent */
	cwd?: string;
	/** Environment variables */
	env?: Record<string, string>;
	/** Provider to use */
	provider?: string;
	/** Model ID to use */
	model?: string;
	/** Additional CLI arguments */
	args?: string[];
	/** Deadline for the capability handshake (default: 30 seconds). */
	startupTimeoutMs?: number;
	/** Reports a subscriber failure without interrupting delivery to other subscribers. */
	onListenerError?: (error: Error, event: RpcEvent) => void;
}
```
(ice:packages/coding-agent/src/modes/rpc/rpc-client.ts:78-95).

```ts
export interface ModelInfo {
	provider: string;
	id: string;
	contextWindow: number;
	reasoning: boolean;
}
```
(ice:packages/coding-agent/src/modes/rpc/rpc-client.ts:103-108).

### 6. Dependencies

`RPC-016`, `RPC-022`, `RPC-033`, `RPC-034`, `RPC-035`

This is the older coding-agent JSONL child-process SDK. The framed-CBOR package client in RPC-010–014 and native TCP proxy in RPC-036 use different transport, lifecycle and response contracts.

### 7. ice-rust today

The inspected native RemoteClient connects a TCP address and RemoteDriver starts a remotely hosted session. Native rpc_with_io implements the child server side, but neither is a host SDK that spawns a local child, validates the TS capability response, supervises stderr, or exposes this restart/termination behavior. Native TCP authentication and session teardown remain separate existing capabilities (rust:crates/ice-cli/src/remote/client.rs:24-74; rust:crates/ice-cli/src/session_driver.rs:215-281; rust:crates/ice-cli/src/modes.rs:278-355).

### 8. Port notes

Add an equivalent host client around the native JSONL compatibility adapter, with explicit executable/argument handling, a validated startup handshake, bounded diagnostic retention, stale-child event guards and deterministic rejection of every pending operation on failure. Preserve the one-second termination escalation and restart identity rules or document an intentional platform-specific process-supervision alternative.

### 9. Size and priority

Estimated Rust implementation: **180-330 lines** (estimate, not measured source). **P1** - Hosts cannot safely submit work or recover from child failure without a verified startup boundary and a complete termination contract.

## RPC-033 - JSONL client request ownership, bounded writes and deadline cancellation

### 1. Title and status

JSONL client request ownership, bounded writes and deadline cancellation. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/modes/rpc/rpc-client.ts:122-134; ice:packages/coding-agent/src/modes/rpc/rpc-client.ts:336-959; ice:packages/coding-agent/src/modes/rpc/rpc-client.ts:1142-1301; ice:packages/coding-agent/test/rpc-client-contract.test.ts:110-257; ice:packages/coding-agent/test/rpc-client-contract.test.ts:356-375; ice:packages/coding-agent/test/rpc-client-contract.test.ts:418-542.

### 3. What it does

Correlate typed commands with one validated response, bound in-flight data while reserving live-control capacity, and distinguish local response timeout, cooperative server cancellation, and confirmed remote settlement.

### 4. How it works

The child-process client gives each command one pending response owner and bounds retained requests. Local timeout, server cancellation and confirmed settlement remain distinct outcomes.

send first rejects an already-aborted signal, verifies a writable attached child, allocates req_<n> unless a private startup ID was supplied, and serializes the command with the allocated ID taking precedence. UTF-8 bytes excluding the trailing LF must not exceed 16 MiB. It then reserves normal or live-control admission using serialized bytes including LF; admission errors become busy. This uses the same 256/32-MiB normal and 64/1-MiB control ceilings as RPC-018, but its accounting includes LF. The reservation remains held until the request resolves/rejects, not merely until stdin's write callback (ice:packages/coding-agent/src/modes/rpc/rpc-client.ts:1142-1163; ice:packages/coding-agent/src/modes/rpc/rpc-client.ts:1196-1220).

A pending entry is installed before adding the abort listener and a second aborted check closes the race. The timer, signal listener and admission release are removed on every settlement path. Local failure removes the pending entry before rejecting; when requested and a write has begun, it sends cancel_request best-effort and ignores cancellation-request failure. written becomes true immediately before stdin.write, so it means the client attempted the write, not that the server accepted or started the operation. A synchronous or callback write error fails the original request. A successful write callback does not settle it. Late/unmatched responses are discarded; duplicate responses cannot resolve an already-removed request (ice:packages/coding-agent/src/modes/rpc/rpc-client.ts:1084-1108; ice:packages/coding-agent/src/modes/rpc/rpc-client.ts:1221-1286).

The generic local response timeout defaults to 30000 ms and includes time waiting for the stdin write. It rejects with a message that remote settlement is unconfirmed. Only sendOperation for compact/navigate_tree/bash opts into cancellation on timeout. These methods accept signal plus an optional positive safe integer timeoutMs no greater than 300000. An explicit timeout is sent to the server and sets the local response deadline to timeoutMs+5000; an omitted timeout is absent on the wire and uses the client's 30000-ms wait. Server request tracking has no implicit deadline for the omitted case. Signal cancellation is not serialized. refreshModels separately defaults its server timeout to 10000 ms, accepts 1–30000, and waits an additional 5000 locally without cancelOnTimeout; abortModelsRefresh is its explicit cancellation command (ice:packages/coding-agent/src/modes/rpc/rpc-client.ts:608-619; ice:packages/coding-agent/src/modes/rpc/rpc-client.ts:1165-1193; ice:packages/coding-agent/src/modes/rpc/rpc-client.ts:1229-1258).

respondUI writes an extension_ui_response with the caller's existing dialog ID and reserves control bytes only for the duration of the write callback. It does not allocate a req ID or pending response and therefore cannot overwrite dialog identity or leak an unanswered pending request. It has no separate 16-MiB input-line check, although the one-MiB control admission ceiling still bounds it (ice:packages/coding-agent/src/modes/rpc/rpc-client.ts:363-374).

Typed wrappers preserve the command-specific options and unwrap validated data; even void wrappers reject unsuccessful server responses. RpcRequestError preserves errorCode, errorDetails and command. Method comments claiming immediate prompt return do not override the awaited server response/preflight boundary. Convenience page methods default message/tree limit to 100 before spreading caller options. importSession includes cwd only when truthy and exportJsonl includes outputPath only when truthy, so those wrappers omit an empty string even where the raw command schema accepts it. The wrapper inventory and forwarding tests cover prompt options, Bash direction/context options, catalog queries, session operations and clone; the corresponding server entries specify their effects (ice:packages/coding-agent/src/modes/rpc/rpc-client.ts:336-959; ice:packages/coding-agent/src/modes/rpc/rpc-client.ts:1288-1301; ice:packages/coding-agent/test/rpc-client-contract.test.ts:418-542; ice:packages/coding-agent/test/rpc-client-clone.test.ts:1-30).

### 5. Interfaces

Typed methods for the 79-command union; RpcOperationOptions {timeoutMs?, signal?}; cancelRequest(requestId); respondUI(existingDialogResponse). Local IDs req_<n>, startup IDs startup_<n>. RpcRequestError(error, errorCode?, errorDetails?, command?). No rollback is promised by timeout, cancellation acknowledgement or local stop.

### 6. Dependencies

`RPC-016`, `RPC-018`, `RPC-019`, `RPC-028`, `RPC-032`, `RPC-034`

The server owns admission/start/settlement and cooperative cancellation. The client separately owns write admission, timers, response correlation and best-effort cancellation; neither ownership substitutes for the other.

### 7. ice-rust today

Native RemoteClient.call allocates numeric JSON-RPC IDs and waits for a matching outer response, but has no bounded pending map, request timeout, AbortSignal equivalent, TS typed error/details or reserved control lane. After event-pump takeover, RemoteDriver sends child command objects through send_command and only awaits writing the outer send request; it does not await a per-command child response. This is useful native interactive transport, not the JSONL SDK request contract (rust:crates/ice-cli/src/remote/client.rs:46-121; rust:crates/ice-cli/src/remote/client.rs:169-190; rust:crates/ice-cli/src/session_driver.rs:290-311).

### 8. Port notes

Implement one pending owner and cleanup path per request, including failed writes and process termination. Match command as well as ID, reserve control capacity, keep dialog responses uncorrelated, and expose server deadlines separately from local waiting. Retain unconfirmed-settlement errors and never translate a local timeout into success or rollback.

### 9. Size and priority

Estimated Rust implementation: **200-360 lines** (estimate, not measured source). **P1** - Incorrect timeout or ID ownership can leak retained requests, cancel the wrong operation, or tell a host that a mutation stopped when it may still have taken effect.

## RPC-034 - Command-specific JSONL response validation and bounded opaque metadata

### 1. Title and status

Command-specific JSONL response validation and bounded opaque metadata. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/modes/rpc/rpc-response-validation.ts:1-653; ice:packages/coding-agent/src/modes/rpc/rpc-client.ts:1084-1108; ice:packages/coding-agent/test/rpc-response-validation.test.ts:1-289; ice:packages/coding-agent/test/rpc-client-contract.test.ts:314-331.

Exported symbols: `RpcResponse` (ice:packages/coding-agent/src/modes/rpc/rpc-types.ts:341).

### 3. What it does

Validate every typed response before resolving a pending request, including nested messages/models/session trees and bounded extension metadata, while rejecting mismatched command identities and malformed success/error payloads.

### 4. How it works

The client validates a response's command and nested payload before resolving its request. Opaque metadata is permitted only within explicit shape, depth and value-count limits.

validateRpcResponse first requires a known command and runs boundedData on the entire envelope. It then requires an object with own type, command and success properties; type must be response, command must equal the pending command, id is optional string and success is boolean. Failure requires a string error and optional string errorCode/typed errorDetails. Success dispatches to a validator for every RpcCommandType, enforced by the exhaustive Record declaration. All thrown validation errors return false. The client removes the matched pending entry before validation; a bad response rejects only that request with invalid_response. Responses without a known pending string ID are ignored. The validator itself does not prove that an optional response ID matches a request; the client performs correlation first (ice:packages/coding-agent/src/modes/rpc/rpc-response-validation.ts:567-652; ice:packages/coding-agent/src/modes/rpc/rpc-client.ts:1084-1108).

The iterative whole-envelope bound permits null/undefined/string/boolean/finite number and plain or null-prototype objects. It rejects other primitive types, exotic objects, ancestor cycles, accessors, symbol keys and malformed arrays. The budget is 1000000 visited values and maximum depth 128 inclusive, with exit markers not charged. A WeakSet tracks only the current ancestry, so shared acyclic metadata is allowed. Arrays must be dense with exactly length own keys after removing length, each a canonical nonnegative safe-integer index below length; object/array values are read from own data descriptors. Opaque fields and unknown extra keys are admitted only within these global limits, without claiming their semantic validity (ice:packages/coding-agent/src/modes/rpc/rpc-response-validation.ts:588-628).

Object validators require each required field to be an own property, validate recognized optional fields when non-undefined, and allow unknown extras. Numeric validation ordinarily means finite number, not nonnegative/integer unless a particular validator adds that restriction. Models require id/name/api/provider/baseUrl/reasoning/input/cost/contextWindow/maxTokens; optional headers, samplingParams, compat and thinkingLevelMap are shape-checked, but the server's model projection separately strips secret headers. Usage requires input/output/cacheRead/cacheWrite/totalTokens and cost with total. Message roles are user, assistant, toolResult, bashExecution, custom, branchSummary and compactionSummary; all require finite timestamp. Assistant content is text/thinking/toolCall, requires api/provider/model/usage and one of pending/stop/length/toolUse/error/aborted/deferred, with separately checked deferred and diagnostic metadata. Persisted entries use the closed known entry-type dispatch and tree validation has its own lower recursion guard. This is stricter than accepting any JSON object with a role or a type (ice:packages/coding-agent/src/modes/rpc/rpc-response-validation.ts:25-178).

The remaining command-specific validators cover capabilities/state, model/catalog refresh, queues/tools/flags/resources/completions, paginated session data, shell/subagent views and patches, settings/schema diagnostics, UI state and command invocation. No-data commands accept data=undefined. get_last_assistant_text requires an own text property whose value is string or null; the existing TS server can omit it when its helper returns undefined, which is the source-level incompatibility recorded in RPC-024 rather than an accepted response shape. Error details permit typed field/expectedType/range/count/allowedValues/scope/schemaId/schemaRevision keys, with bounded unknown extras. Source tests supply malformed nested payloads, shared acyclic metadata, cycles/accessors/deep trees and mismatched commands; they were inspected, not executed (ice:packages/coding-agent/src/modes/rpc/rpc-response-validation.ts:179-586; ice:packages/coding-agent/test/rpc-response-validation.test.ts:203-288; ice:packages/coding-agent/test/rpc-client-contract.test.ts:314-331).

### 5. Interfaces

validateRpcResponse(command:RpcCommandType, value:unknown):boolean. Required command equality, optional string envelope ID, exhaustive success validator and bounded failure details. Invalid responses become RpcRequestError with errorCode=invalid_response at the correlated client boundary.

`RpcResponse` has common success fields `{id?:string,type:'response',command:<literal>,success:true}` and exactly the command-specific data member below; `no data member` means the declared success variant omits it. All 79 input command names are represented once in this grouped mapping. Failure is `{id?:string,type:'response',command:string,success:false,error:string,errorCode?:string,errorDetails?:RpcCommandErrorDetails}` (ice:packages/coding-agent/src/modes/rpc/rpc-types.ts:341-572).

| Command(s) | Success data |
| --- | --- |
| `prompt`, `steer`, `follow_up`, `abort`, `reload`, `stop_after_turn`, `invoke_shortcut`, `abort_models_refresh`, `set_fast_mode`, `set_steering_mode`, `set_follow_up_mode`, `set_auto_compaction`, `set_auto_retry`, `abort_retry`, `abort_compaction`, `abort_branch_summary`, `abort_bash`, `rename_session`, `set_session_name` | `no data member` |
| `remove_queued_message` | `{removed:boolean}` |
| `get_queue`, `clear_queue` | `RpcQueueSnapshot` |
| `cancel_request`, `respond_subagent_approval` | `{accepted:boolean}` |
| `new_session`, `switch_session`, `clone`, `import_session` | `{cancelled:boolean}` |
| `get_capabilities` | `RpcCapabilities` |
| `get_state` | `RpcSessionState` |
| `get_tools` | `{tools:RpcToolInfo[]}` |
| `set_active_tools` | `{active:string[]}` |
| `get_system_prompt` | `{text:string}` |
| `get_diagnostics` | `RpcDiagnostics` |
| `get_ui_state` | `RpcUIState` |
| `get_completions` | `RpcCompletions` |
| `get_resources` | `RpcResources` |
| `get_changelog` | `{entries:RpcChangelogEntry[]}` |
| `get_scoped_models`, `set_scoped_models` | `{models:RpcScopedModelRef[]}` |
| `get_flags` | `{flags:RpcExtensionFlag[]}` |
| `get_shortcuts` | `{shortcuts:RpcExtensionShortcut[]}` |
| `get_shells` | `{shells:RpcShellSummary[]}` |
| `get_shell` | `RpcShellDetail\|null` |
| `kill_shell` | `{stopped:boolean}` |
| `background_bash` | `{released:number}` |
| `get_settings`, `set_setting` | `RpcSettingsSnapshot` |
| `get_subagents` | `IceSubagentOverview\|null` |
| `subagent_control` | `{message:string}` |
| `subagent_patch` | `RpcSubagentPatchResult` |
| `get_subagent_messages` | `IceSubagentTranscript\|null` |
| `set_model` | `Model<any>` |
| `cycle_model` | `{model:Model<any>,thinkingLevel:ThinkingLevel,isScoped:boolean}\|null` |
| `get_available_models` | `{models:Model<any>[],hasMore?:boolean,nextCursor?:string}` |
| `get_providers` | `RpcProviderSnapshot` |
| `get_models` | `RpcModelSnapshot` |
| `refresh_models` | `RpcModelsRefreshResult` |
| `set_thinking_level` | `{level:ThinkingLevel}` |
| `cycle_thinking_level` | `{level:ThinkingLevel}\|null` |
| `get_available_thinking_levels` | `{levels:ThinkingLevel[]}` |
| `compact` | `CompactionResult` |
| `bash` | `BashResult` |
| `get_session_stats` | `SessionStats & {cacheWaste:CacheWasteTotals}` |
| `export_html`, `export_jsonl` | `RpcExportResult` |
| `fork` | `{text?:string,cancelled:boolean}` |
| `list_sessions` | `RpcSessionPage` |
| `delete_session` | `{method:'trash'\|'unlink'}` |
| `navigate_tree` | `{cancelled:boolean,aborted?:boolean,editorText?:string,summaryEntryId?:string}` |
| `set_entry_label` | `{entryId:string}` |
| `get_fork_messages` | `{messages:Array<{entryId:string,text:string}>}` |
| `get_entries` | `RpcEntryPage` |
| `get_tree` | `RpcTreePage` |
| `get_last_assistant_text` | `{text:string\|null}` |
| `get_messages` | `RpcMessagePage` |
| `get_commands` | `{commands:RpcSlashCommand[]}` |
| `get_command_schema` | `RpcCommandSchemaResult` |
| `invoke_command` | `RpcCommandInvocationResult` |

The declared get_last_assistant_text payload requires text:string|null. It does not resolve the existing host omission of undefined text documented in RPC-024; that source-level mismatch remains an open port/reference issue.

### 6. Dependencies

`AI-019`, `RPC-004`, `RPC-016`, `RPC-017`, `RPC-024`, `RPC-029`, `RPC-031`, `RPC-033`

RPC owns the exact JSONL message projection and validation in this entry. The binary protocol has a different schema. This validator covers data shape and bounded admission, not full semantic validation of arbitrary extension fields or provider behavior; shared request hard-output authority is a separate AI-006 concern.

### 7. ice-rust today

Native serde RemoteResponse decodes its status/result/error envelope and native events, while RemoteClient.call reads arbitrary serde_json::Value and matches only an outer numeric JSON-RPC ID. There is no exhaustive TS-command success schema, pending-command equality check, or TS nested response compatibility validator. Rust's JSON value domain already excludes JavaScript accessors/exotic objects, so port the relevant depth/count/shape guarantees rather than mechanically recreating JavaScript-only hazards (rust:crates/ice-cli/src/remote/protocol.rs:318-373; rust:crates/ice-cli/src/remote/client.rs:78-121; rust:crates/ice-cli/src/remote/session.rs:202-238).

### 8. Port notes

Define and enforce the complete JSONL envelope and per-command payload schemas at the client boundary, retaining precise null-versus-omitted behavior and command identity. Bound parser/validation work, opaque metadata and nested trees in native terms. Resolve the recorded empty assistant-text response mismatch explicitly, and map native messages to the TS roles and stopReason spellings before validation.

### 9. Size and priority

Estimated Rust implementation: **330-560 lines** (estimate, not measured source). **P1** - Typed callers otherwise consume malformed or unrelated nested payloads as valid data, and silent wire-shape differences defeat compatibility despite successful JSON parsing.

## RPC-035 - JSONL client event delivery, idle waiters and bounded collectors

### 1. Title and status

JSONL client event delivery, idle waiters and bounded collectors. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/modes/rpc/rpc-client.ts:310-317; ice:packages/coding-agent/src/modes/rpc/rpc-client.ts:970-1140; ice:packages/coding-agent/src/modes/rpc/rpc-command-lanes.ts:19-21; ice:packages/coding-agent/test/rpc-client-contract.test.ts:125-183; ice:packages/coding-agent/test/rpc-client-contract.test.ts:332-355; ice:packages/coding-agent/test/rpc-client-contract.test.ts:398-417.

Exported symbols: `RpcEventListener` (ice:packages/coding-agent/src/modes/rpc/rpc-client.ts:111).
Exported symbols: `RPC_MAX_COLLECTED_EVENT_BYTES`, `RPC_MAX_COLLECTED_EVENTS` (ice:packages/coding-agent/src/modes/rpc/rpc-command-lanes.ts:20; ice:packages/coding-agent/src/modes/rpc/rpc-command-lanes.ts:21).

### 3. What it does

Deliver future JSONL events to independent subscribers, expose an explicit agent_settled waiting boundary, and bound event collections without leaving timers/listeners behind when a prompt, process or collector fails.

### 4. How it works

Event subscribers receive future notifications independently, while idle waiters and collectors own their cleanup. The completion boundary is agent_settled, and collection cancellation does not cancel remote work.

handleLine ignores all input once exitError is set, silently ignores invalid JSON and objects without a string type, and treats type=response only as request correlation. Any other object with a string type is cast to RpcEvent without validating the event-specific payload. Delivery iterates a snapshot of the listener array, so additions/removals during one callback affect later deliveries rather than skipping another callback in the current snapshot. Each synchronous callback exception is reported through onListenerError(error, event), or a generic stderr message if no reporter is configured; a throwing reporter is caught and reported generically. Returned promises are not awaited, so this is not asynchronous callback-rejection isolation. onEvent appends a listener and its unsubscribe removes the first matching identity (ice:packages/coding-agent/src/modes/rpc/rpc-client.ts:310-317; ice:packages/coding-agent/src/modes/rpc/rpc-client.ts:1084-1126).

waitForIdle defaults to 60000 ms and waits for a future event whose type is agent_settled. It does not query state or immediately resolve because a session was idle before subscription. Before checking exitError it rejects busy when terminationListeners already has at least 64 members; this shared set includes collectors as well as idle waiters. Success, timeout or process termination clears the timer, unsubscribes and removes the termination callback. Timeout includes the bounded stderr tail. No positive/safe timeout validation is added by this method (ice:packages/coding-agent/src/modes/rpc/rpc-client.ts:970-999).

collectEvents also defaults to 60000 ms, rejects an existing exitError first, then an already-aborted signal, then busy at four active collectors. It retains the same event object references delivered to listeners, charges each serialized JSON line including LF, and rejects output_overflow before adding an event when the existing count is at least 16384 or cumulative bytes exceed 16 MiB. Overflow clears the retained array. A future agent_settled is included in a successful returned collection. Idempotent cleanup decrements activeCollectors and removes its timeout, listener, signal handler and termination callback on every completion/error/cancel path. Collector signal cancellation stops collection only; it does not send a remote abort. Collection timeout and cancellation use ordinary Error, while capacity/overflow use RpcRequestError (ice:packages/coding-agent/src/modes/rpc/rpc-command-lanes.ts:19-21; ice:packages/coding-agent/src/modes/rpc/rpc-client.ts:1005-1061).

promptAndWait installs the collector before submitting the prompt, observes its rejection immediately to avoid an unhandled rejection while prompt preflight is pending, then awaits prompt before returning the event promise. Finally it aborts the collector. A submission failure is preserved even if the collector also times out or fails; cleanup does not imply remote rollback or identify which concurrently submitted operation caused an agent_settled event. Process termination snapshots and clears the shared termination listener set after rejecting pending requests, then notifies every registered waiter (ice:packages/coding-agent/src/modes/rpc/rpc-client.ts:1066-1078; ice:packages/coding-agent/src/modes/rpc/rpc-client.ts:1132-1140).

### 5. Interfaces

onEvent(listener):unsubscribe; waitForIdle(timeout=60000):Promise<void>; collectEvents(timeout=60000, signal?):Promise<RpcEvent[]>; promptAndWait(message, images?, timeout=60000). Settlement is agent_settled, not agent_end, raw process exit or a successful prompt preflight response.

`RpcEventListener = (event: RpcEvent) => void`; its return is not awaited by the event loop. The named collection constants are RPC_MAX_COLLECTED_EVENT_BYTES=16*1024*1024 and RPC_MAX_COLLECTED_EVENTS=16384; shared output record ceiling is 32*1024*1024 (ice:packages/coding-agent/src/modes/rpc/rpc-client.ts:111; ice:packages/coding-agent/src/modes/rpc/rpc-command-lanes.ts:19-21).

### 6. Dependencies

`RPC-017`, `RPC-020`, `RPC-021`, `RPC-032`, `RPC-033`

The server session emits the settlement event after its defined run/queue boundary. The client waits for that wire event and cannot substitute its own inferred idle state.

### 7. ice-rust today

The native TCP event pump feeds an unbounded channel; RemoteDriver folds and projects events to another unbounded channel and signals Ended when the pump closes. It has no TS subscriber SDK, bounded event collector, agent_settled waiter or promptAndWait contract. Native transcript/status folding and modal-dialog forwarding are retained as the distinct capability in RPC-036 (rust:crates/ice-cli/src/remote/client.rs:127-165; rust:crates/ice-cli/src/session_driver.rs:243-264; rust:crates/ice-cli/src/remote/session.rs:240-384).

### 8. Port notes

Add snapshot-based subscriber delivery and bounded waiter/collector ownership to the native host SDK. Preserve the explicit settlement event, listener error isolation and cleanup order. Keep collector cancellation independent from operation cancellation and document that unvalidated events and shared event-object references are current TS behavior, not stronger guarantees.

### 9. Size and priority

Estimated Rust implementation: **130-230 lines** (estimate, not measured source). **P1** - Unbounded collectors or leaked waiters retain memory and timers; confusing agent_end with settlement can return before queued work has completed.

## RPC-036 - Native TCP proxy and transcript projection preservation, with premature stream termination

### 1. Title and status

Native TCP proxy and transcript projection preservation, with premature stream termination. **Status: different-by-design.**

### 2. ice source

ice:packages/coding-agent/src/modes/rpc/rpc-client.ts:160-305; ice:packages/client/src/client.ts:1-433; ice:packages/coding-agent/src/client/remote-session.ts:1-421; rust:crates/ice-cli/src/remote/client.rs:1-192; rust:crates/ice-cli/src/remote/session.rs:1-493; rust:crates/ice-cli/src/session_driver.rs:207-444; rust:crates/ice-cli/tests/remote_client_smoke.rs:1-510.

### 3. What it does

Retain the native TCP JSON-RPC remote-session path and its shared TUI event projection as an existing native capability, while keeping it distinct from both TS clients and repairing a content-sensitive event-pump termination condition.

### 4. How it works

Rust's TCP proxy and shared TUI projection are existing capabilities to preserve. They use a different outer protocol from the two TypeScript clients, and their current terminal-event check can mistake ordinary content for stream completion.

The TS comparison contains two different host APIs: a child-process JSONL client with capability handshake and typed command responses (RPC-032–035), and a newer framed-CBOR client/RemoteSession with server-session handles, leases and snapshots (RPC-010–014). Neither is evidence that native TCP JSON-RPC is absent; neither provides the same outer authenticate/start_session/subscribe/send/stop_session protocol. External ice-server package implementation and its deployments are outside the inspected checkout and remain unverified (ice:packages/coding-agent/src/modes/rpc/rpc-client.ts:160-305; ice:packages/client/src/client.ts:1-433; ice:packages/coding-agent/src/client/remote-session.ts:1-421).

Native RemoteClient connects TCP, owns one BufReader and write half, and allocates numeric IDs from one. Each request serializes JSON, appends LF, writes all and flushes. authenticate returns success locally for an empty token; otherwise it calls authenticate with the token and ignores a successful result's shape. Empty-token local success does not prove that the server permits unauthenticated sessions. call writes the request before checking whether it still owns a reader. It skips blank/invalid JSON, any method-bearing notification and mismatched IDs, returns matching error.message or serialized error, and otherwise returns result or null. It has no transport deadline, line-byte ceiling, TLS layer or JSON-RPC version check in this implementation. Calling call after event-pump takeover can therefore write a request before returning the streaming-mode error (rust:crates/ice-cli/src/remote/client.rs:24-121).

RemoteDriver connects, authenticates, calls start_session with an empty object, requires sessionId:string, subscribes to that session, requires subscriptionId:u64, then transfers the reader to an event pump. It best-effort sends get_state for initial chrome state. The driver exposes its event receiver once and serializes writes through a mutex. Prompt/Abort/RefreshState/SetModel/SetThinkingLevel/SetActiveTools/UiDialogResponse map to the native child command union with a separate atomic child ID. send_command wraps the command in outer method=send and returns after writing; later child responses arrive through event notifications rather than being awaited by that send. shutdown writes stop_session best-effort. Authentication-like errors get a --token retrieval hint (rust:crates/ice-cli/src/session_driver.rs:215-311; rust:crates/ice-cli/src/session_driver.rs:434-444; rust:crates/ice-cli/src/remote/client.rs:169-190).

The pump ignores invalid/blank lines, non-event methods and subscriptions whose numeric ID differs; it forwards params.event or null into an unbounded channel. Reader EOF/error or a dropped receiver ends the pump. A second attempt to start a pump returns an already-closed receiver. After forwarding each accepted event, it currently searches the complete serialized outer line for the substring containing a quoted session_end token and breaks if found. For example, a valid matching event containing a text field whose complete value is session_end is forwarded and then closes the stream, even though its event type is not terminal. Driver pump closure subsequently emits DriverEvent::Ended, so this defect is reachable from ordinary remote content. This conclusion follows directly from source, not an executed reproduction. The condition should inspect the parsed event discriminator defined by the outer server protocol, preserving genuine terminal behavior (rust:crates/ice-cli/src/remote/client.rs:127-165; rust:crates/ice-cli/src/session_driver.rs:243-264).

Native RemoteSession folds ready, decoded responses and decoded RemoteEvent values; unknown/invalid payloads are ignored. Error responses set last_error and append a notice; a successful result updates SessionState only when sessionId and activeTools are present and decoding succeeds. AgentStart/AgentEnd toggle streaming and clear retry/open-block state; retries retain attempt/delay and append a notice; turn events and UI dialogs do not enter its transcript. Role reading tries kind before role, so an existing non-string kind prevents fallback to role. Assistant start creates a new block; text/thinking deltas append to lazily opened blocks, and assistant end replaces an open block only with nonempty final text or appends a nonempty final block when none is open. Tool-call deltas are ignored by this fold; tool execution start appends a tool item, update stores partial text, and end stores final text/error/done on the matching tool. Tool-result text joins supported text content. Source unit tests cover streamed text, tool errors, thinking and state/error lines (rust:crates/ice-cli/src/remote/session.rs:136-162; rust:crates/ice-cli/src/remote/session.rs:191-493).

Production RemoteDriver publishes status after every pumped line, emits an error notice only when last_error changes, and separately maps RemoteEvent to the shared UI event abstraction. This projection reads complete message snapshots for assistant start/update/end and tool calls, so the fold's ignored tool-call deltas do not imply the production TUI cannot render tool calls. UI dialog requests and closes are explicitly forwarded to the modal slot outside the transcript; turn events have no UI event. Native smoke-test source covers TCP handshake/run folding, answering/cancelling approvals, filtering request responses and token authentication. A separate optional real-server test is gated by ICERS_REMOTE_E2E_ADDR and does not establish that an external server was exercised here (rust:crates/ice-cli/src/session_driver.rs:243-264; rust:crates/ice-cli/src/session_driver.rs:314-431; rust:crates/ice-cli/tests/remote_client_smoke.rs:130-509).

### 5. Interfaces

Native RemoteClient::connect/authenticate/call/start_event_pump/send_command/stop_session; RemoteDriver implements SessionDriver. Outer TCP JSON-RPC authenticate/start_session/subscribe/send/stop_session; event notifications carry subscriptionId and a child event. Driver commands and native status/result/error response envelope remain distinct from TS success/data/error JSONL and framed CBOR.

### 6. Dependencies

`RPC-010`, `RPC-014`, `RPC-015`, `RPC-017`, `RPC-028`, `RPC-032`, `RPC-035`

Retain existing native shared-driver TUI and extension modal-dialog support. Compatibility work must select the intended outer transport rather than conflating this proxy with TS server leases, binary snapshots or the local process SDK.

### 7. ice-rust today

Present as a native-specific implementation in remote/client.rs, remote/session.rs and session_driver.rs, with source-visible tests. The raw-string terminal check at rust:crates/ice-cli/src/remote/client.rs:160 is a real source-level defect in that existing path. Unbounded lines/channels and lack of deadlines are additional explicit limits, not evidence that TCP remote sessions are wholly missing.

### 8. Port notes

Preserve the native TCP path and UI/dialog projection, fix terminal detection against the parsed event field, and decide explicit budgets/deadlines if this path is exposed to compatibility clients. Add TS child-process and binary protocols as separately identified adapters; do not replace the native driver with an empty stub or mark its external server deployment verified. A compatibility check should distinguish ordinary content containing session_end from an actual terminal event and verify that the pump remains live in the former case.

### 9. Size and priority

Estimated Rust implementation: **40-100 lines** (estimate, not measured source). **P1** - Existing remote sessions and approval dialogs must survive port work; the current content-sensitive stop can prematurely disconnect a valid remote run.

