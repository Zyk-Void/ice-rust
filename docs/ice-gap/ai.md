# AI protocol, model and provider runtime gaps

Source-derived comparison of TypeScript ice-ai with the actual Rust provider and harness implementations. This area contains 50 reviewed gap entries and reconciles all 67 assigned source rows, 14 mechanical candidates and the manually discovered public surfaces. Native providers, typed events, schema support, durable partial-progress frames and compaction are credited. No application, generator, build, test or provider call was executed. The coordinator's final acceptance artifacts record cross-area and active-checkout verification.

## AI-001 - Shared assistant retry classification, bounded delay and lifecycle callbacks

### 1. Title and status

Shared assistant retry classification, bounded delay and lifecycle callbacks. **Status: partial.**

### 2. ice source

ice:packages/ai/src/utils/retry.ts:1-237 (getRetryDelayMs, retryAssistantCall, isRetryableAssistantError); ice:packages/ai/test/retry.test.ts:21-229 (classification, delay, lifecycle and abort assertions).

### 3. What it does

SDK and summarization callers reuse one assistant-message retry policy with visible, awaited lifecycle callbacks. It classifies transient failures separately from account/quota exhaustion and keeps exponential delays finite even for extreme attempt counts.

### 4. How it works

The reusable retry loop always makes the initial assistant-producing call, then decides from its returned message whether another call is allowed. It never retries an aborted response and invokes retry callbacks only after an actual retry has been scheduled.

Policy contains enabled, maxRetries, baseDelayMs and optional maxDelayMs. An absent/disabled policy permits zero retries; the initial call is not a retry. A non-error/non-aborted stop returns immediately. For an error, the loop first checks the exhausted budget and then the classifier. Non-retryable patterns win over retryable patterns, so '429 quota exceeded' is terminal. The excluded account/billing vocabulary includes GoUsageLimitError, FreeUsageLimitError, Monthly usage limit reached, available balance, insufficient_quota, out of budget, quota exceeded and billing. Retryable expressions cover load/status failures, flexible separators in network/server phrases, DNS errors, socket/WebSocket failures, 'terminated', premature protocol EOF, explicit retry guidance, retry-delay-cap errors and ResourceExhausted (ice:packages/ai/src/utils/retry.ts:7-89; ice:packages/ai/src/utils/retry.ts:232-237).

getRetryDelayMs validates maxDelayMs as finite/nonnegative, caps it at 2147483647, otherwise uses 60000; a nonfinite/nonpositive base gives zero. It clamps the exponent to 0..1023, then takes the cap, so attempt 2048 cannot overflow a JavaScript timer. This assistant-level delay has no jitter. Before each wait, onRetryScheduled receives one-based attempt, maximum attempts, delay and error text with 'Unknown error' fallback. After the abortable wait, onRetryAttemptStart is awaited immediately before the next call. onRetryFinished is awaited once on a normal terminal exit after scheduling: success for a non-error result, false for abort, or false plus final error for exhaustion/nonretryable failure. Abort during the wait retains the last assistant content/usage, changes stopReason to aborted and clears errorMessage; it reports the last error through the finish callback (ice:packages/ai/src/utils/retry.ts:108-220).

The function does not catch a rejected produce() or callback promise into an assistant message and has no encompassing finally callback; those exceptions propagate. Cancellation of the initial request is the producer's responsibility. No filesystem state, credential lookup or network transport is owned by this helper; per-session side-effect gates and compaction continuation belong to SESSION-018/015.

### 5. Interfaces

RetryPolicy {enabled:boolean, maxRetries:number, baseDelayMs:number, maxDelayMs?:number}; RetryCallbacks.onRetryScheduled(attempt, maxAttempts, delayMs, errorMessage), onRetryAttemptStart(), onRetryFinished(success, attempt, finalError?). retryAssistantCall(produce, policy, signal, callbacks):Promise<AssistantMessage>; getRetryDelayMs(base, attempt, max?):number; isRetryableAssistantError(message):boolean. All callbacks may return promises and are awaited.

### 6. Dependencies

None for the pure classifier and retry loop. SESSION-011/012/018 consume this capability; transport retries remain a separate layer in AI-004.

### 7. ice-rust today

Rust already implements a real assistant retry loop in rust:crates/ice-harness/src/compaction/compaction.rs:394-494 and a classifier at rust:crates/ice-harness/src/compaction/compaction.rs:500-564. It preserves abort-terminal, budget, success-fast and cancellation-during-sleep behavior, but the loop is private to compaction and has no callback sink. Its substring table omits the literal 'terminated' and cannot match regex separator variants such as 'rate_limit' that the TS table accepts. It adds 'exceeded request buffer limit while retrying upstream', so the tables are not identical in either direction. Delay uses saturating_mul(1u64 << (attempt-1)); saturation happens after the unchecked shift. A configured retry count reaching attempt 65 can trigger a shift-overflow panic when overflow checks are enabled, so this is not equivalent to the timer-safe TS clamp. Default Rust RetryPolicy is enabled=true, max_retries=10, base_delay_ms=2000, max_agent_delay_ms=60000 at rust:crates/ice-harness/src/types.rs:278-301. This defect analysis is source-only.

### 8. Port notes

Move or expose a reusable assistant-level retry primitive under the appropriate shared Rust layer and keep existing harness callers on it. Preserve quota-before-transient precedence, port the full classifier with bounded regex or explicit normalized matching, and use checked/capped exponentiation before constructing the duration. Add an awaited callback interface and retain the exact abort/result normalization and exception policy. Keep provider HTTP retry budgets independent. Future implementation checks should cover 'terminated', separator variants, mixed quota/429 text, attempt 2048, callback ordering, callback failure and abort during backoff; this audit did not create or execute tests.

### 9. Size and priority

Estimated Rust implementation: **160-300 lines** (estimate, not measured source). **P1** - Transient stream failures currently differ across callers and extreme configured retry counts can violate bounded backoff.

## AI-002 - Provider collection with cache-first refresh and generation-checked publication

### 1. Title and status

Provider collection with cache-first refresh and generation-checked publication. **Status: partial.**

### 2. ice source

ice:packages/ai/src/models.ts:39-236 (Provider/Models contracts), ice:packages/ai/src/models.ts:254-541 (collection, refresh and availability), ice:packages/ai/src/models.ts:735-862 (createProvider); source-only full test review by the authentication worker recorded at evidence/providers-auth-source-review.json for ice:packages/ai/test/models-runtime.test.ts:1-1160.

Exported symbols: `ModelsRefreshOptions`, `ModelsRefreshResult`, `ModelsRequestTransforms`, `ModelsApiStreamOptions`, `ModelsSimpleStreamOptions`, `ModelsDeferredFetchOptions`, `ModelsDeferredCancelOptions`, `CreateModelsOptions` (ice:packages/ai/src/models.ts:64; ice:packages/ai/src/models.ts:73; ice:packages/ai/src/models.ts:78; ice:packages/ai/src/models.ts:83; ice:packages/ai/src/models.ts:84; ice:packages/ai/src/models.ts:85; ice:packages/ai/src/models.ts:86; ice:packages/ai/src/models.ts:232).
Exported symbols: `CombinedAbortSignal`, `combineAbortSignals` (ice:packages/ai/src/utils/abort-signals.ts:1; ice:packages/ai/src/utils/abort-signals.ts:6).
Exported symbols: `raceWithAbortSignal` (ice:packages/ai/src/utils/abort.ts:17).

### 3. What it does

The reusable Models collection owns provider identity, known catalogs, availability and concurrent dynamic refresh. A replacement or newer refresh prevents stale asynchronous work from publishing over the current provider generation.

### 4. How it works

Models separates synchronous last-known catalog reads from asynchronous credential-aware availability and refresh. Each provider owns its catalog and streaming behavior; the collection supplies shared credential/catalog stores and controls publication ordering.

createModels defaults to in-memory credential and model stores plus the default auth context. setProvider upserts by provider.id while invalidating and aborting its prior refresh; deleteProvider and clearProviders do the same before removing providers. Map insertion order determines getProviders/all-model order. getModels catches each provider's thrown getter and treats it as no models; getModel selects the first matching id. getAvailable concurrently checks provider auth, then calls each getter and optional filterModels with the stored credential; unlike getModels, a throwing getter/filter here rejects the request (ice:packages/ai/src/models.ts:254-335; ice:packages/ai/src/models.ts:485-541).

refresh defaults allowNetwork=true and selects all providers with refreshModels, or the requested subset; unknown/static providers are ignored. A pre-aborted caller returns {aborted:true, errors:empty}. Each selected provider gets a new generation and controller combined with the caller signal. It first reads the credential, remembers read failure, and runs an offline restore phase with a cloned provider-scoped stored catalog before rethrowing the credential error. With network disabled it stops there. Otherwise it resolves/refreshes configured credentials, skips an unconfigured provider, and runs the network phase with force forwarded only there. Providers run concurrently; non-abort failures are retained per provider, non-Error failures become ModelsError('model_source'), and the returned errors map is copied. A superseded provider's signal abort does not set the caller-level aborted result (ice:packages/ai/src/models.ts:367-475).

publish serializes operations per provider behind the previous publication promise even when it failed. It checks generation/signal before persistence and again before synchronous update. persist omitted leaves storage unchanged; null deletes; a value writes a structured clone. Only after that mutation succeeds and remains current does update() change provider-private memory. The public waiter can abort while the underlying promise remains observed; generation checks suppress later memory publication. This is a guarded sequence, not a general database transaction or rollback promise (ice:packages/ai/src/models.ts:338-365).

createProvider retains a static baseline and overlays dynamic models by id in place, appending new ids. Cached models are filtered to this provider before restoration; successful fetch results persist with checkedAt=Date.now() before replacing dynamic memory. Failure retains previous state. OAuth refresh errors preserve stored credentials for retry; automatic deletion/re-login is not implemented by this collection. Exact redacted expressions at models.ts469/493 are not reconstructed; visible method flow and independently read tests support the stated boundary.

Cancellation helpers make the collection's generation/caller rule precise. `combineAbortSignals` drops undefined inputs, returns no signal for zero inputs, reuses the single input unchanged, and creates a controller for multiple inputs. The first observed abort preserves its reason; an already aborted input stops listener registration at that input, and the explicit cleanup closure removes every listener installed earlier. It does not cancel the input controllers. `raceWithAbortSignal` rejects an already aborted waiter while attaching a catch to the operation, otherwise settles once on operation completion/failure or abort and removes its listener. It checks again after listener installation to close the registration race. The operation remains observed after waiter cancellation; this helper does not stop its side effects. An undefined abort reason becomes Error('The operation was aborted') with name AbortError (ice:packages/ai/src/utils/abort-signals.ts:1-41; ice:packages/ai/src/utils/abort.ts:1-50).

### 5. Interfaces

MutableModels.setProvider/deleteProvider/clearProviders; getProviders/getProvider/getModels/getModel; refresh({allowNetwork?, providers?, force?, signal?}) → {aborted, errors:ReadonlyMap<string, Error>}; getAvailable(providerId?,{signal?}). RefreshModelsContext {credential?, stored?, publish, allowNetwork, force?, signal}; ModelsPublication {persist?:ModelsStoreEntry|null, update?:()=>void}. Provider.refreshModels and filterModels are optional; signal is concrete in refresh phases.

`CombinedAbortSignal { signal?: AbortSignal; cleanup: () => void }`; `combineAbortSignals(signals: readonly (AbortSignal | undefined)[]): CombinedAbortSignal`; `raceWithAbortSignal<T>(operation: Promise<T>, signal: AbortSignal): Promise<T>`. `ModelsRequestTransforms.transformHeaders?: (headers: ProviderHeaders) => ProviderHeaders | Promise<ProviderHeaders>` is intersected into raw/simple/deferred collection request option types (ice:packages/ai/src/models.ts:78-86).

### 6. Dependencies

`AUTH-001`, `AUTH-010`

AUTH-001 supplies credential storage/auth primitives; AUTH-010 owns persistent model catalog storage and provider freshness policy. The collection itself is reusable without the coding-agent UI/runtime wrapper. Provider-specific catalog declarations and auth flows remain AUTH-owned.

### 7. ice-rust today

A genuine Rust ModelRegistry exists at rust:crates/ice-cli/src/model_registry.rs:61-342. It stores metadata and provider instances under separate RwLocks, can register/unregister providers, update auth status and broadcast events through a capacity-100 channel. refresh_catalog loads local models.json, replaces metadata and updates last_refresh; it has no provider-owned dynamic refresh hook, offline-then-auth/network phases, per-provider generation/controller, persistence publication chain or returned provider-error map. The core rust:crates/ice-ai/src/provider.rs:149-175 trait exposes id/models/stream_simple and an optional deferred sidecar, without this collection/auth/refresh contract. Existing registry functionality should be reused where appropriate; its existence alone does not certify the Models lifecycle.

### 8. Port notes

Add a reusable provider collection around the native Provider trait, with explicit mutable provider identity and separate last-known versus available-model queries. Introduce provider refresh context, catalog-store operations and per-provider cancellation/generation guards; serialize persistence before memory publication and document the late-cancellation boundary. Preserve previous catalogs on fetch failures and expose per-provider errors without discarding successful peers. Integrate the coding ModelRuntime through AUTH's wrapper rather than creating a second independent registry. Future checks should cover cache restore before failed auth, replacing a provider during refresh, overlapping persistence, offline force suppression and aborting an ignored provider signal.

### 9. Size and priority

Estimated Rust implementation: **500-950 lines** (estimate, not measured source). **P1** - Reliable dynamic model discovery and extension provider replacement require one coherent catalog lifecycle.

## AI-003 - API factory dispatch, lazy setup errors and legacy registry compatibility

### 1. Title and status

API factory dispatch, lazy setup errors and legacy registry compatibility. **Status: partial.**

### 2. ice source

ice:packages/ai/src/models.ts:628-862 (request dispatch and createProvider), ice:packages/ai/src/api/lazy.ts:1-98, ice:packages/ai/src/compat.ts:83-298, ice:packages/ai/src/legacy-api-aliases.ts:1-108; ice:packages/ai/test/lazy-module-load.test.ts:65-119 and ice:packages/ai/test/compat-env.test.ts:41-74.

Exported symbols: `anthropicMessagesApi` (ice:packages/ai/src/api/anthropic-messages.lazy.ts:4).
Exported symbols: `azureOpenAIResponsesApi` (ice:packages/ai/src/api/azure-openai-responses.lazy.ts:4).
Exported symbols: `googleGenerativeAIApi` (ice:packages/ai/src/api/google-generative-ai.lazy.ts:4).
Exported symbols: `googleVertexApi` (ice:packages/ai/src/api/google-vertex.lazy.ts:4).
Exported symbols: `iceMessagesApi` (ice:packages/ai/src/api/ice-messages.lazy.ts:4).
Exported symbols: `LazyApiCapabilities` (ice:packages/ai/src/api/lazy.ts:68).
Exported symbols: `mistralConversationsApi` (ice:packages/ai/src/api/mistral-conversations.lazy.ts:4).
Exported symbols: `openAICodexResponsesApi` (ice:packages/ai/src/api/openai-codex-responses.lazy.ts:4).
Exported symbols: `openAICompletionsApi` (ice:packages/ai/src/api/openai-completions.lazy.ts:4).
Exported symbols: `openAIResponsesApi` (ice:packages/ai/src/api/openai-responses.lazy.ts:4).
Exported symbols: `ApiStreamFunction`, `ApiStreamSimpleFunction`, `ApiProvider`, `getApiProviders`, `registerFauxProvider` (ice:packages/ai/src/compat.ts:71; ice:packages/ai/src/compat.ts:77; ice:packages/ai/src/compat.ts:83; ice:packages/ai/src/compat.ts:144; ice:packages/ai/src/compat.ts:160).
Exported symbols: `streamAnthropic`, `streamSimpleAnthropic`, `streamAzureOpenAIResponses`, `streamSimpleAzureOpenAIResponses`, `streamGoogle`, `streamSimpleGoogle`, `streamGoogleVertex`, `streamSimpleGoogleVertex`, `streamMistral`, `streamSimpleMistral`, `streamOpenAICodexResponses`, `streamSimpleOpenAICodexResponses`, `streamOpenAICompletions`, `streamSimpleOpenAICompletions`, `streamOpenAIResponses`, `streamSimpleOpenAIResponses` (ice:packages/ai/src/legacy-api-aliases.ts:29; ice:packages/ai/src/legacy-api-aliases.ts:34; ice:packages/ai/src/legacy-api-aliases.ts:40; ice:packages/ai/src/legacy-api-aliases.ts:45; ice:packages/ai/src/legacy-api-aliases.ts:51; ice:packages/ai/src/legacy-api-aliases.ts:53; ice:packages/ai/src/legacy-api-aliases.ts:59; ice:packages/ai/src/legacy-api-aliases.ts:61; ice:packages/ai/src/legacy-api-aliases.ts:67; ice:packages/ai/src/legacy-api-aliases.ts:72; ice:packages/ai/src/legacy-api-aliases.ts:78; ice:packages/ai/src/legacy-api-aliases.ts:83; ice:packages/ai/src/legacy-api-aliases.ts:89; ice:packages/ai/src/legacy-api-aliases.ts:94; ice:packages/ai/src/legacy-api-aliases.ts:100; ice:packages/ai/src/legacy-api-aliases.ts:105).
Exported symbols: `CreateProviderOptions`, `hasApi` (ice:packages/ai/src/models.ts:739; ice:packages/ai/src/models.ts:874).
Exported symbols: `ApiOptionsMap`, `ApiStreamOptions` (ice:packages/ai/src/types.ts:251; ice:packages/ai/src/types.ts:268).
Exported symbols: `createAssistantMessageEventStream` (ice:packages/ai/src/utils/event-stream.ts:86).

### 3. What it does

Providers may use one API implementation or dispatch mixed models by model.api, while legacy extensions register global API adapters with source ownership. Async auth/module setup still returns an event stream immediately and reports setup failure in the stream.

### 4. How it works

createProvider accepts either a single ProviderStreams object or a map of API names to implementations. The returned provider delegates stream and streamSimple to the selected implementation, and exposes deferred methods only when at least one implementation advertises that capability.

Missing API implementation yields a lazy stream error naming provider and API. Deferred fetch/cancel resolves the implementation for the requested model and errors if that specific API lacks the operation, even if another API made the provider-level method available. Models dispatch resolves the provider before auth, applies auth/request transformations, then calls the provider selected for the original request. complete/completeSimple/fetchDeferred await the stream result; cancelDeferred rejects directly (ice:packages/ai/src/models.ts:628-862).

lazyStream creates an outer AssistantMessageEventStream synchronously, awaits setup, forwards every event and then the inner result if supplied. Setup/forwarding failure creates an assistant error with original API/provider/model, empty content, zero usage/cost, current timestamp and the thrown error text. No synthetic start is guaranteed on that failure path. lazyApi dynamically imports on first stream/deferred call; the host module cache deduplicates imports. Its advertised deferred flags determine whether methods exist before loading. Source tests assert root/compat/catalog imports do not load the five provider SDKs and an Anthropic stream loads only that SDK; tests were read, not run (ice:packages/ai/src/api/lazy.ts:4-98; ice:packages/ai/test/lazy-module-load.test.ts:65-119).

The compatibility registry upserts by API string, stores optional sourceId, guards a wrapper against a mismatched model.api and removes only registrations currently owned by a matching sourceId. Built-in registration fills missing slots without overwriting existing overrides; reset clears and restores built-ins. Dispatch uses the original built-in provider only while the registry instance identity still matches the captured built-in instance and that provider advertises the requested API. Cloudflare without already resolved auth goes through Models; other paths inject a nonblank environment key unless it is the ambient-auth marker. Unknown legacy API lookup and mismatch guards can throw synchronously, unlike the new lazy setup contract (ice:packages/ai/src/compat.ts:100-298).

The TS EventStream queues events without a bound and hands each event to one waiter or the queue. A done/error event settles result() and makes later pushes no-ops; iteration drains queued events before stopping. end() without an explicit result closes iterators but leaves result() unresolved, and producer abandonment has no automatic settlement. The existing native split producer/consumer channel and producer-drop error are preferable termination guarantees; they should be retained when adding lazy/setup and deferred dispatch rather than reproducing the unresolved-promise case (ice:packages/ai/src/utils/event-stream.ts:4-88; rust:crates/ice-ai/tests/event_stream.rs:76-107).

The exported lazy factory names listed in field 2 return new adapter objects around the shared loader; JavaScript module caching supplies implementation-load reuse. `hasApi(model, api)` is only the equality/type guard `model.api === api`, with no auth, loading or I/O (ice:packages/ai/src/models.ts:874-876).

### 5. Interfaces

ProviderStreams {stream, streamSimple, fetchDeferred?, cancelDeferred?}; createProvider({id, name?, baseUrl?, headers?, auth, models, fetchModels?, filterModels?, api}). lazyApi(load,{fetchDeferred?, cancelDeferred?}); lazyStream(model, setup). Legacy registerApiProvider(provider, sourceId?), unregisterApiProviders(sourceId), getApiProvider(s), registerBuiltInApiProviders(), resetApiProviders(), stream/complete/streamSimple/completeSimple. Deprecated per-API stream aliases remain exported.

`ApiStreamFunction` and `ApiStreamSimpleFunction` take `(model: Model<Api>, context: Context, options?: StreamOptions | SimpleStreamOptions)` respectively and return `AssistantMessageEventStream`. `ApiOptionsMap` maps the ten built-in API literals to their concrete provider options; `ApiStreamOptions<TApi> = TApi extends keyof ApiOptionsMap ? ApiOptionsMap[TApi] : StreamOptions & Record<string, unknown>`. These imports are type-only and add no eager provider SDK loading (ice:packages/ai/src/compat.ts:71-87; ice:packages/ai/src/types.ts:246-270).

### 6. Dependencies

`AI-002`, `AUTH-009`

AI-002 supplies the provider collection; AUTH-009 owns credential/header assembly. Extension registration bridges consume this dispatch seam. Native Rust compilation does not require JavaScript module loading, but setup-failure and capability contracts still need an explicit equivalent.

### 7. ice-rust today

Rust has a real Provider trait, event stream and optional DeferredProvider sidecar at rust:crates/ice-ai/src/provider.rs:149-204 and rust:crates/ice-ai/src/event_stream.rs:18-172. The trait currently exposes only stream_simple and fixed SimpleStreamOptions, so the API-specific typed/raw stream surface and mixed-API adapter map are not represented there. Concrete modules are compiled behind the providers feature at rust:crates/ice-ai/src/providers/mod.rs:8-17; lazy SDK imports are a language/deployment difference, not evidence of a missing native feature. The native CLI ModelRegistry registers provider instances by provider id, not the TS source-owned global API registry (rust:crates/ice-cli/src/model_registry.rs:113-170). Preserve producer-drop RecvError reporting, which is stronger than an unresolved bare TS result promise.

### 8. Port notes

Define the supported native raw/simple request boundary and mixed-API dispatch explicitly, sharing one provider collection. Add capability-aware deferred dispatch and convert async setup failures into terminal stream messages. Implement only the legacy registry compatibility needed by the extension bridge, preserving source ownership and override identity; document intentional synchronous-error differences. Keep native feature-based compilation while ensuring unused provider initialization has no unwanted startup work. Reuse the existing event channel rather than duplicating it.

### 9. Size and priority

Estimated Rust implementation: **230-480 lines** (estimate, not measured source). **P1** - Custom and extension providers cannot safely depend on a uniform dispatch contract while raw options, capability selection and setup errors vary.

## AI-004 - Provider request retry headers, delay validation and typed error preservation

### 1. Title and status

Provider request retry headers, delay validation and typed error preservation. **Status: partial.**

### 2. ice source

ice:packages/ai/src/utils/provider-retry.ts:1-129; ice:packages/ai/test/provider-retry.test.ts:11-102.

### 3. What it does

Provider HTTP requests retry with SDK-compatible status/header decisions and interruptible backoff. Server-specified delays are validated and capped before waiting, and explicitly nonretryable responses stay terminal.

### 4. How it works

The request retry helper wraps an SDK call configured with maxRetries=0 so every retry is an independently cancellable request. It operates on thrown provider errors before streaming begins, separately from AI-001's assistant-message retry loop.

A provider error must be an Error containing status and headers, with status undefined or numeric and headers undefined or an actual Headers object. x-should-retry='true' overrides the status to retry; 'false' prohibits it. Otherwise undefined status, 408, 409, 429 and status>=500 are retryable. maxRetries defaults to zero. The request is attempted before the catch's cancellation check; after a failed attempt, an aborted signal becomes AbortError('Request aborted'), otherwise budget/type/classification decide whether to retry (ice:packages/ai/src/utils/provider-retry.ts:14-35; ice:packages/ai/src/utils/provider-retry.ts:109-129).

retry-after-ms is parsed first with parseFloat and accepted only if finite and nonnegative. retry-after next accepts numeric seconds or an HTTP date relative to Date.now(), again finite/nonnegative. A valid server delay above maxRetryDelayMs (default 60000) fails immediately with an error showing ceil(seconds) and the original provider message; cap zero disables that check. Invalid headers fall through to exponential min(0.5*2^retryIndex,8)*1000 multiplied by 1-random*0.25. The cap applies to server-requested delays, not the fallback exponential. Sleep cleans up its timer/listener and rejects on abort. Tests explicitly cover negative/infinite headers, x-should-retry=false, oversized delays, disabled cap and timer cancellation (ice:packages/ai/src/utils/provider-retry.ts:37-98; ice:packages/ai/test/provider-retry.test.ts:17-100).

No payload replay safety is inferred for a stream already producing content; individual adapters choose which opening request to wrap. Callback errors and malformed non-provider errors propagate without this retry policy. No persistent state is used.

### 5. Interfaces

retryProviderRequest<T>(request, {maxRetries?, maxRetryDelayMs?, signal?}):Promise<T>. Error transport fields: status:number|undefined, headers:Headers|undefined. Headers read: x-should-retry, retry-after-ms, retry-after. Error on excessive server delay: 'Server requested <ceil seconds>s retry delay (max: <ceil seconds>s). <provider message>'.

### 6. Dependencies

None for the helper. Concrete HTTP adapters must carry status and retry headers into this policy; AI-001 and SESSION-018 own outer retry decisions.

### 7. ice-rust today

rust:crates/ice-ai/src/providers/anthropic/retry.rs:21-49 has a real status/header decision type, and rust:crates/ice-ai/src/providers/anthropic/retry.rs:75-197 implements delay/cancellation/retry. However retry_provider_request accepts AiError; From<&AiError> at lines 51-71 erases all retry headers, maps every non-HTTP error to status=None and therefore makes it retryable unless the cancellation token is set. AiError itself has no retry-header fields (rust:crates/ice-ai/src/error.rs:13-31). The real Anthropic request path consumes the response body and creates only AiError::Http{status, message} at rust:crates/ice-ai/src/providers/anthropic/mod.rs:261-315, so helper unit tests that construct ProviderRequestError headers do not prove production propagation. retry_delay ignores HTTP dates, removes jitter, and casts parsed negative/nonfinite f64 values into u64 durations rather than enforcing TS finite/nonnegative checks. Existing status retries and abortable waits should be retained. These are source findings, not network reproductions.

### 8. Port notes

Carry a typed request error with response headers from every adapter into the retry loop, and keep non-HTTP auth/schema/provider errors terminal unless explicitly classified. Port header precedence, finite/nonnegative parsing, HTTP-date handling, cap-zero semantics and jitter. Retain cancellation both while opening a request and during sleep. Correct comments/tests that currently imply header propagation through AiError. Future checks should exercise the actual adapter-to-loop error path, not only a manually constructed helper struct.

### 9. Size and priority

Estimated Rust implementation: **150-320 lines** (estimate, not measured source). **P1** - Dropped retry directives can cause unwanted retries, ignore server wait limits and obscure deterministic failures.

## AI-005 - Thinking-level availability, ultra and upward-first clamping

### 1. Title and status

Thinking-level availability, ultra and upward-first clamping. **Status: partial.**

### 2. ice source

ice:packages/ai/src/types.ts:80-99; ice:packages/ai/src/models.ts:900-953; ice:packages/ai/src/api/simple-options.ts:90-131; ice:packages/ai/test/max-thinking.test.ts:15-164 and ice:packages/ai/test/supports-xhigh.test.ts:4-157.

### 3. What it does

Model metadata controls which reasoning levels are valid, including the opt-in ultra level. Unsupported requests clamp to the next supported higher level before falling back lower, so holes in a provider's level map preserve the intended effort.

### 4. How it works

The ordered display/runtime levels are off, minimal, low, medium, high, xhigh, max and ultra. A nonreasoning model always reports only off; a reasoning model filters this ordered list using its thinkingLevelMap.

A map value null removes a level. xhigh/max/ultra are included only when their map value is defined and non-null; ordinary levels remain available when omitted. clampThinkingLevel returns an already supported request, then searches upward from its index, then downward; an unknown value or no remaining level falls back to the first available or off. The source test with xhigh:null, max:'max' requires xhigh to clamp to max, not high. A mapped ultra value survives the helper and is forwarded by compatible OpenAI requests (ice:packages/ai/src/models.ts:900-941; ice:packages/ai/test/max-thinking.test.ts:35-85; ice:packages/ai/test/max-thinking.test.ts:105-121).

Budget-based adapters use clampReasoning, which maps xhigh/max/ultra to high. Default budgets are minimal 1024, low 2048, medium 8192, high 16384; caller budgets override per level. With no explicit caller cap the model ceiling contains thinking; with a caller cap, thinking is added but capped by model/hard authority. If the ceiling does not exceed the thinking budget, thinking shrinks to leave 1024 ordinary output tokens. AI-006 owns hard-authority validation. modelsAreEqual compares only provider and id, returning false if either model is absent; metadata updates do not create a new identity.

### 5. Interfaces

ModelThinkingLevel='off'|ThinkingLevel; ThinkingLevel includes minimal/low/medium/high/xhigh/max/ultra. ThinkingLevelMap=Partial<Record<ModelThinkingLevel, string|null>>. getSupportedThinkingLevels(model), clampThinkingLevel(model, level), modelsAreEqual(a, b); clampReasoning(effort) for budget providers. Provider-specific mapped strings remain provider metadata.

### 6. Dependencies

None for level availability/clamping. AUTH-007 owns models.json parsing and AUTH provider entries own catalog declarations; SESSION-036 owns user preference persistence and model-selection events.

### 7. ice-rust today

Rust exposes Off..Max without Ultra at rust:crates/ice-ai/src/types.rs:489-507. Model::supported_thinking_levels at rust:crates/ice-ai/src/model.rs:235-258 does not check model.reasoning, so a default nonreasoning model can advertise Off/Minimal/Low/Medium/High. clamp_thinking_level searches only downward at rust:crates/ice-ai/src/model.rs:260-282, conflicting with the concrete TS hole fixture. Budget arithmetic exists at rust:crates/ice-ai/src/providers/anthropic/build_params.rs:1066-1107, but lacks the hard-authority parameter. The Anthropic mapped-effort helper at rust:crates/ice-ai/src/providers/anthropic/build_params.rs:1109-1133 only accepts a fixed list of recognized strings and falls back to high for other mappings. Do not infer ultra support merely from retaining custom JSON text elsewhere.

### 8. Port notes

Add Ultra consistently to native parsing, serialization, metadata and request adapters. Make nonreasoning models return only Off; preserve null versus missing map entries and implement upward-then-downward clamping. Keep the budget-level reduction separate from provider effort-string mapping. Route the same helper through session settings and model selectors, coordinating AUTH-007 and SESSION-036, and preserve provider/id identity semantics during metadata refresh.

### 9. Size and priority

Estimated Rust implementation: **90-210 lines** (estimate, not measured source). **P1** - Incorrect level selection can reject restored configurations or silently change requested reasoning effort.

## AI-006 - Shared request options and non-widening hard output authority

### 1. Title and status

Shared request options and non-widening hard output authority. **Status: partial.**

### 2. ice source

ice:packages/ai/src/types.ts:104-244 and ice:packages/ai/src/types.ts:315-338; ice:packages/ai/src/api/simple-options.ts:12-131; ice:packages/ai/test/simple-options.test.ts:27-62; ice:packages/ai/test/hard-max-output-tokens.test.ts:38-143.

Exported symbols: `clampMaxTokensToContext`, `adjustMaxTokensForThinking` (ice:packages/ai/src/api/simple-options.ts:15; ice:packages/ai/src/api/simple-options.ts:96).
Exported symbols: `ServiceTier`, `FetchFunction`, `SessionAffinityFormat`, `ProviderResponse`, `ProviderRequestOptions`, `ProviderStreamOptions`, `DeferredFetchOptions`, `DeferredCancelOptions`, `StreamFunction` (ice:packages/ai/src/types.ts:106; ice:packages/ai/src/types.ts:111; ice:packages/ai/src/types.ts:112; ice:packages/ai/src/types.ts:114; ice:packages/ai/src/types.ts:120; ice:packages/ai/src/types.ts:233; ice:packages/ai/src/types.ts:235; ice:packages/ai/src/types.ts:244; ice:packages/ai/src/types.ts:334).
Exported symbols: `headersToRecord` (ice:packages/ai/src/utils/headers.ts:3).

### 3. What it does

Every adapter receives a common request envelope and must respect a runtime hard output allowance after reasoning and payload customization. This authority is distinct from a user preference or a model's nominal maximum.

### 4. How it works

buildBaseOptions combines model and request settings into the shared adapter envelope, with request sampling keys overriding model defaults. It first clamps requested output to estimated context headroom, then applies the runtime hard limit, and forwards the remaining request callbacks and transport controls.

Context clamping reserves 4096 tokens and keeps a minimum of one: with no positive model window it returns max(1, requested); otherwise min(requested, max(1, window-estimate-4096)). Hard authority absent leaves the requested value, including null-as-unset, unchanged. If supplied it must be positive and finite, and any explicit requested value must also be positive/finite; an absent request becomes the hard allowance and an explicit one becomes min(requested, hard). enforceHardMaxTokensInRecord rejects a nonnumeric non-null payload field and reapplies this clamp after onPayload, preventing a callback from widening or removing authority (ice:packages/ai/src/api/simple-options.ts:15-49).

The envelope preserves serviceTier, temperature, samplingParams, maxTokens, hardMaxOutputTokens, toolChoice, signal, apiKey, fetch, transport, cacheRetention, sessionId, promptCacheKey, headers, onPayload, onResponse, timeoutMs, websocketConnectTimeoutMs, maxRetries, maxRetryDelayMs, metadata and env. An explicitly supplied helper apiKey uses truthy precedence over options.apiKey. Header null means suppress the same default name. onPayload can inspect or replace assembled wire data; onResponse is awaited after HTTP headers arrive and before consuming a response body. These are transport lifecycle hooks, not the harness terminal-assistant callback (ice:packages/ai/src/types.ts:119-231; ice:packages/ai/src/api/simple-options.ts:51-87).

Thinking adjustment caps the combined allowance to model/hard ceiling and rejects hard authority below the 1024 ordinary-output floor for budget-based thinking. The ten-adapter source test inspects each actual output field before a deliberate callback throw, including nested Google/Bedrock/ICE fields; it also requires Azure to reject authority below its provider floor before the payload callback. These assertions were read as source, not executed (ice:packages/ai/test/hard-max-output-tokens.test.ts:38-143). Concrete adapter entries retain their per-provider floor and callback details.

`headersToRecord` copies each `[key,value]` from the platform Headers iterator into an ordinary object, retaining the iterator's resulting names and values. It is distinct from `providerHeadersToRecord`, which removes null deletion markers and returns undefined when nothing remains; the response-header snapshot helper has no credential resolution or additional normalization (ice:packages/ai/src/utils/headers.ts:3-17).

### 5. Interfaces

StreamOptions and SimpleStreamOptions fields listed above; transport='sse'|'websocket'|'websocket-cached'|'auto'; cacheRetention='none'|'short'|'long'; toolChoice simple subset 'auto'|'none'; ProviderHeaders maps names to string|null. clampMaxTokensToHardLimit(max, hard), enforceHardMaxTokensInRecord(payload, field, hard), buildBaseOptions(model, context, options, apiKey?). Errors explicitly name invalid positive finite limits or provider output floors.

Additional exact aliases: `ServiceTier = 'auto' | 'default' | 'flex' | 'scale' | 'priority' | null`; `FetchFunction = typeof globalThis.fetch`; `SessionAffinityFormat = 'openai' | 'openai-nosession' | 'openrouter'`; `ProviderResponse { status: number; headers: Record<string,string> }`. `DeferredFetchOptions.wait?: number` has documented default 0 for one status check; `DeferredCancelOptions` is the shared provider-request option type, not an independent scheduler (ice:packages/ai/src/types.ts:106-117; ice:packages/ai/src/types.ts:235-244).

### 6. Dependencies

The pure authority clamp has no prerequisite. Provider adapters consume it; SESSION-028 supplies settings and extension hooks, while bounded agent/subagent finalization supplies the runtime allowance. Context-estimation differences are a separate AI entry and SESSION-010 supplies the shared Unicode compatibility primitive.

### 7. ice-rust today

rust:crates/ice-ai/src/provider.rs:31-88 includes timeout, retry, headers, string metadata, cache retention, session id, cancellation, reasoning, max_tokens, temperature and thinking budgets, but has no hard output authority, toolChoice, per-request sampling map, service tier, prompt-cache key, transport/connect timeout, scoped env/custom fetch or wire payload/HTTP-response callbacks. ProviderHooks at rust:crates/ice-ai/src/provider.rs:229-283 runs at the harness option/context boundary and observes the terminal assistant, so it is not the HTTP lifecycle contract. Anthropic already clamps context and fits thinking at rust:crates/ice-ai/src/providers/anthropic/build_params.rs:1066-1107 and rust:crates/ice-ai/src/providers/anthropic/build_params.rs:1186-1205, but no hard-authority argument survives thinking. Some call-site expressions at 1199/1294/1295/1331 are connector-redacted and are not reconstructed. Absence of hard authority is established by the actual public structs and helper signatures, not those redacted expressions.

### 8. Port notes

Extend the native request envelope with the required common fields and distinguish trusted runtime hard authority from mutable provider/user request options. Validate it once, carry it through every adapter, and reapply it after payload transforms and reasoning adjustments; reject impossible provider floors. Add explicit wire payload and HTTP response hooks while preserving existing harness hooks as separate semantics. Use an injectable native transport abstraction for testability instead of emulating JavaScript fetch literally. Coordinate changes across all adapters and extension serialization so no path drops the authority.

### 9. Size and priority

Estimated Rust implementation: **350-700 lines** (estimate, not measured source). **P0** - A missing immutable output bound undermines bounded finalization and request resource authority across providers.

## AI-007 - Provider overflow and prematurely truncated length classification

### 1. Title and status

Provider overflow and prematurely truncated length classification. **Status: missing.**

### 2. ice source

ice:packages/ai/src/utils/overflow.ts:37-179 (isContextOverflow, isRecoverableLength, getOverflowPatterns); ice:packages/ai/test/overflow.test.ts:1-178.

### 3. What it does

The shared AI layer distinguishes context exhaustion from transient overload and identifies length stops eligible for one bounded compact-and-retry attempt. Callers can recognize providers that report successful but oversized input or consume their whole window before producing output.

### 4. How it works

isContextOverflow classifies the completed assistant message using both the normalized stop reason and provider evidence. The classifier itself is pure and does not compact, retry or mutate a conversation.

For stopReason=error with nonempty errorMessage, exclusion patterns run first: prefixed Bedrock 'Throttling error'/'Service unavailable', rate limit and too many requests suppress all overflow matches. The positive table includes prompt-is-too-long, request_too_large, Bedrock input-too-long, OpenAI context-window/maximum-length variants, Gemini input count, xAI maximum prompt, Groq reduce-messages, OpenRouter/Poolside input limits, Together token counts, Copilot limit, llama.cpp available context, LM Studio initial prompt, MiniMax window, Kimi model limit, Mistral maximum context, DS4 configured size, z.ai finish-reason text, Ollama, Qwen input range, generic context_length_exceeded/too-many-tokens/token-limit and exact bodyless 400/413 prefixes (ice:packages/ai/src/utils/overflow.ts:37-78).

With a truthy supplied contextWindow and stopReason=stop, input+cacheRead must be strictly greater than the window. For stopReason=length, output must be exactly zero and input+cacheRead at least 99% of the window. cacheWrite and totalTokens do not enter these two predicates. Missing window disables silent/length overflow classification; silent provider truncation without those signals remains undetectable. isRecoverableLength independently requires length, desiredMaxOutput>0 and output<desiredMaxOutput, where the caller must supply the original allowance before context clamping. getOverflowPatterns returns a new array containing the regex objects. Invalid numeric inputs receive no extra finite/range validation here. SESSION-015 owns admission, one-recovery guard and persistence.

### 5. Interfaces

isContextOverflow(AssistantMessage, contextWindow?:number):boolean; isRecoverableLength(AssistantMessage, desiredMaxOutput:number):boolean; getOverflowPatterns():RegExp[]. Inputs use stopReason, errorMessage and usage.input/cacheRead/output.

### 6. Dependencies

No prerequisite for the pure predicates. SESSION-015 consumes classification and owns compact-and-retry control; AI-008 supplies separate request-context estimates.

### 7. ice-rust today

The native provider layer has request-side context clamping (rust:crates/ice-ai/src/providers/anthropic/build_params.rs:1139-1205), and the harness has actual pre-run/mid-loop compaction and retry. Those do not implement this completed-message classification. The fully reviewed harness run branches lack the coding-session one-overflow recovery branch (rust:crates/ice-harness/src/agent_harness.rs:3547-3785; rust:crates/ice-harness/src/agent_harness.rs:4012-4279; SESSION-015). A bounded crates-wide search for context overflow/length-exceeded predicates found only clamp/threshold functions; absence is scoped to this public classifier and reviewed control path, not to every native compaction capability.

### 8. Port notes

Add public, table-driven completed-message predicates with exclusion-before-positive ordering and the exact silent/zero-output thresholds. Preserve the original requested output allowance separately from clamped wire values, then connect the predicates to SESSION-015's bounded recovery state machine. Carry provider error bodies through AI-010 so useful overflow text is retained. Treat provider-side silent truncation without evidence as an explicit limit of detection.

### 9. Size and priority

Estimated Rust implementation: **130-250 lines** (estimate, not measured source). **P1** - Provider context errors otherwise terminate recoverable runs or can be confused with rate limiting.

## AI-008 - Request-context usage checkpoints, prefix invalidation and deferred tool estimates

### 1. Title and status

Request-context usage checkpoints, prefix invalidation and deferred tool estimates. **Status: partial.**

### 2. ice source

ice:packages/ai/src/utils/estimate.ts:1-143 (estimateContextTokens and helpers); ice:packages/ai/test/context-estimate.test.ts:1-82; ice:packages/ai/test/deferred-tools.test.ts:460-483.

### 3. What it does

Request construction estimates current context using applicable reported usage, later messages, system text and tool schemas. It invalidates stale usage when newer prefix material precedes the response and adds definitions introduced after the usage checkpoint.

### 4. How it works

estimateContextTokens accepts either messages alone or a full Context and returns both the total estimate and its checkpoint/trailing components. It estimates unreported content using JavaScript UTF-16 string length divided by four, rounded up at the helper/message boundaries.

Text and thinking contribute their lengths; tool calls contribute name plus safe JSON argument serialization. Each user/tool-result image contributes 4800 characters, or 1200 estimated tokens. JSON serialization failure becomes '[unserializable]' and undefined becomes 'undefined'. calculateContextTokens uses truthy totalTokens, otherwise input+output+cacheRead+cacheWrite. The scan tracks the largest timestamp in the preceding prefix; an assistant usage block qualifies only when its timestamp is at least that prefix maximum, its stop reason is neither error nor aborted, and its calculated usage is positive. The last qualifying block wins. A later invalid assistant does not explicitly clear an earlier qualifying checkpoint (ice:packages/ai/src/utils/estimate.ts:14-86).

After a checkpoint, only subsequent messages are estimated. The full-Context form also gathers addedToolNames from later tool results and charges the currently active tool definitions with those names; it does not resurrect definitions absent from Context.tools. Without a checkpoint, it adds the system prompt and serialized entire tools list. The array form cannot account for those prefix fields. The exact expression at line 92 was connector-redacted and was not recovered; the surrounding contract and independent tests show usage-plus-trailing output, but that hidden expression is not asserted as directly read. The estimate is a heuristic rather than an exact tokenizer or a safety authority.

### 5. Interfaces

ContextUsageEstimate {tokens, usageTokens, trailingTokens, lastUsageIndex:number|null}; calculateContextTokens(Usage), estimateTextTokens, estimateTextAndImageContentTokens, estimateMessageTokens, estimateContextTokens(Context|readonly Message[]). Full Context adds systemPrompt and tools.

### 6. Dependencies

`SESSION-010`

SESSION-010 provides the shared string-length compatibility decision; this entry owns request-time checkpoint and schema accounting, not compaction serialization. AI-006 consumes its estimate for request headroom.

### 7. ice-rust today

Rust's real Anthropic request estimator counts Unicode scalar values across all messages and system text, then rounds once (rust:crates/ice-ai/src/providers/anthropic/build_params.rs:1139-1184). Its match arms ignore assistant tool-call name/arguments, its signature has no tools input, and it does not select reported-usage checkpoints or deferred-tool schema additions. Image=4800 and the 4096-token clamp reserve exist. Scalar counting also differs from TS UTF-16 for astral characters. The separate native compaction estimator has usage/trailing behavior but UTF-8 byte counting (SESSION-010); it is not the request helper.

### 8. Port notes

Implement one reusable request-estimation function with explicit Context and structured estimate output. Preserve checkpoint timestamp eligibility, per-message rounding, current-tool membership and post-checkpoint addedToolNames accounting; use a documented UTF-16 compatibility helper if exact TS parity is required. Route every adapter's context clamp through it, while keeping hard output authority independent. Resolve the redacted line through an authorized source review before certifying literal implementation equivalence.

### 9. Size and priority

Estimated Rust implementation: **180-320 lines** (estimate, not measured source). **P1** - Missing tool/schema and usage accounting can send requests that exceed model context and produce inconsistent output allowances.

## AI-009 - Structured assistant diagnostics that preserve nonfatal provider failures

### 1. Title and status

Structured assistant diagnostics that preserve nonfatal provider failures. **Status: missing.**

### 2. ice source

ice:packages/ai/src/utils/diagnostics.ts:1-45; ice:packages/ai/src/types.ts:427-441 (AssistantMessage.diagnostics).

### 3. What it does

Provider adapters can append timestamped diagnostic records to an assistant message without replacing its user-visible content or terminal error. Thrown values are normalized while retaining useful native Error metadata.

### 4. How it works

createAssistantMessageDiagnostic builds one diagnostic from an arbitrary thrown value and optional details. appendAssistantMessageDiagnostic replaces the diagnostics array with its old elements plus the new record, preserving insertion order.

An Error contributes name when truthy, message with name fallback, stack, and code only when code is a string or number. A non-Error becomes name='ThrownValue' and a message from Error/string/String(value) normalization; objects therefore normally become their JavaScript string form, not JSON. Timestamps use Date.now at creation. type is an unrestricted string and details is an optional unknown-valued record; neither is validated or redacted. These helpers own no filesystem, telemetry transport, retry, bounded queue or credential policy. Adapters must choose when to emit and callers must decide how to display or persist sensitive diagnostic data.

### 5. Interfaces

DiagnosticErrorInfo {name?, message, stack?, code?:string|number}; AssistantMessageDiagnostic {type, timestamp, error?, details?}; formatThrownValue, extractDiagnosticError, createAssistantMessageDiagnostic, appendAssistantMessageDiagnostic. AssistantMessage.diagnostics is optional.

### 6. Dependencies

No pure-helper prerequisite. Provider adapters supply diagnostics and SESSION-022/029 own event/persistence carriers if the native product elects to expose them.

### 7. ice-rust today

The actual native AssistantMessage struct carries content, provider/model identifiers, response model/id, usage, stop reason, error text and timestamp, but no diagnostics array (rust:crates/ice-ai/src/types.rs:654-701). AiError has typed variants and Display text (rust:crates/ice-ai/src/error.rs:13-56), so errors are not absent; it does not provide this ordered nonfatal diagnostic carrier. Native observability spans and harness events are distinct transports.

### 8. Port notes

Define a serializable diagnostic record and optional assistant field, plus a helper that preserves native error-chain metadata with a documented equivalent to TS thrown values. Carry records through provider results, frame/session serialization and client rendering. Specify size/redaction policy at trust boundaries rather than implying the TS helper already sanitizes arbitrary details.

### 9. Size and priority

Estimated Rust implementation: **90-180 lines** (estimate, not measured source). **P2** - Nonfatal provider recovery evidence is lost across message/session boundaries, complicating diagnosis.

## AI-010 - Bounded provider error normalization and response-body preservation

### 1. Title and status

Bounded provider error normalization and response-body preservation. **Status: partial.**

### 2. ice source

ice:packages/ai/src/utils/error-body.ts:1-149; ice:packages/ai/test/error-body.test.ts:1-227; ice:packages/ai/test/provider-error-body-regression.test.ts:1-214; ice:packages/ai/test/provider-error-body-passthrough.test.ts:1-79.

### 3. What it does

Adapters retain provider status and response-body details when an SDK Error.message is generic. The shared formatter selects the useful body without consuming a response stream and preserves messages that already contain it.

### 4. How it works

normalizeProviderError extracts SDK-shaped status/body data and marks whether the original Error.message already carries the selected body. formatProviderError then chooses the original message or the status/body form according to that normalized record.

Only actual Error instances undergo SDK field probing; a non-Error becomes safeJsonStringify(value), with messageCarriesBody=false and no status/body. The first numeric status wins in statusCode, status, $metadata.httpStatusCode, $response.statusCode order. Body selection tries string body, nonempty plain-object error, then string or nonempty plain-object $response.body. Stream-like and class instances are rejected; plain objects with null prototypes are accepted. Selection precedes trimming: an empty body string blocks later field fallback, then becomes no extracted body. The selected trimmed body is capped at 4000 UTF-16 code units, with an appended omitted-character count marker (ice:packages/ai/src/utils/error-body.ts:16-117).

messageCarriesBody is true when no body was extracted or Error.message includes the already truncated body string. If it is true, or status/body is missing, the formatter returns the message, optionally prefixed as '<prefix> (<status>): <message>' when both prefix and status exist. Otherwise it returns '<status>: <body>' or '<prefix> (<status>): <body>', replacing the generic message. It does not independently detect duplicate prefix/status text. safeJsonStringify uses JSON.stringify with String(value) fallback, so a non-Error thrown string becomes quoted JSON. Empty Error.message receives no name fallback. Formatting applies no global bound to the original message and is not a secret scrubber (ice:packages/ai/src/utils/error-body.ts:119-149).

The regression suites mock SDK failures and assert body propagation, including preserving real exception text rather than serialized SDK wrapper internals. These are source assertions, not executed results.

### 5. Interfaces

MAX_PROVIDER_ERROR_BODY_CHARS=4000; NormalizedProviderError {status?:number, body?:string, message:string, messageCarriesBody:boolean}; normalizeProviderError(error), formatProviderError(normalized, prefix?), truncateErrorText(text, maxChars), safeJsonStringify(value).

### 6. Dependencies

Pure formatting has no prerequisite. AI-001/004/007 consume resulting text/status for retries and overflow classification; provider-specific transport entries own extraction from actual HTTP responses.

### 7. ice-rust today

AiError retains HTTP status/message and structured Provider code/message, with deterministic display prefixes (rust:crates/ice-ai/src/error.rs:13-51). It has no separate response-body/headers carrier or shared bounded extraction contract, and Display itself applies no 4000-unit bound or duplicate suppression. Actual adapters already retain some response text; therefore this is partial consistency and information-preservation parity, not a claim that Rust discards every error body. The typed error carrier also needs to survive production retry call sites (AI-004).

### 8. Port notes

Introduce a structured transport failure with status, selected headers and bounded body, then use one formatter across adapters. Extract the body once at the transport boundary, preserve typed classification separately from display text, and avoid attempting to emulate arbitrary JavaScript SDK objects. Match message/body selection, optional prefix and truncation semantics where clients depend on error strings; preserve body evidence for overflow/retry classification.

### 9. Size and priority

Estimated Rust implementation: **130-260 lines** (estimate, not measured source). **P1** - Generic or inconsistent error messages can hide recoverable provider failures and misclassify retry/context exhaustion.

## AI-011 - Tolerant streaming tool-argument JSON parsing and malformed escape policy

### 1. Title and status

Tolerant streaming tool-argument JSON parsing and malformed escape policy. **Status: partial.**

### 2. ice source

ice:packages/ai/src/utils/json-parse.ts:1-124 (repairJson, parseJsonWithRepair, parseStreamingJson).

### 3. What it does

Streaming tool deltas can expose usable partial argument values before a complete JSON object arrives. The parser first accepts valid JSON, then attempts targeted string repair and partial parsing without throwing into the stream.

### 4. How it works

parseStreamingJson tries complete JSON parsing before applying any repair. It returns an empty object only after the complete and partial parsing paths have all failed.

The repair pass tracks whether it is inside a JSON string, escapes literal control characters there, preserves recognized escapes, and doubles backslashes before unrecognized escape letters. A syntactically valid four-hex Unicode escape is consumed as a unit. A malformed Unicode escape still falls through the recognized 'u' escape branch and is not fully repaired, which is a TS edge case. Parsing order is JSON.parse(raw), JSON.parse(repaired) when changed, partial-json(raw), then partial-json(repaired). Empty/undefined input gives {}. Complete successful parses are returned via a generic type assertion, including primitive/array/null roots; partial parser nullish results become {}. parseJsonWithRepair, unlike the streaming wrapper, throws when both complete parses fail and preserves the original parse error when no repair changed the input. The helper has no schema validation, size bound, execution authority or persistence role. AI-012 remains the execution-time validator.

### 5. Interfaces

repairJson(json:string):string; parseJsonWithRepair<T>(json:string):T; parseStreamingJson<T=Record<string, unknown>>(partialJson:string|undefined):T. The generic return type does not enforce the runtime root type.

### 6. Dependencies

No execution dependency for parsing. AI-012 must validate arguments before tool execution; provider stream mappers consume the partial values.

### 7. ice-rust today

Rust has a real shared partial parser with consumers in providers and event frames (rust:crates/ice-ai/src/json_parse.rs:1-305). It repairs malformed Unicode escapes by escaping the backslash, unlike the TS branch, and closes unfinished strings/containers and trailing commas with a local scanner instead of the partial-json package. Prefixes ending in an incomplete property/colon can still fail and return an empty object rather than retain prior valid members; dangling escape cases differ. Its comment about ignoring trailing garbage is not implemented by a dedicated trailing-input removal step. The historical Anthropic json_parse module re-exports the shared implementation, rather than maintaining a separate parser (rust:crates/ice-ai/src/providers/anthropic/json_parse.rs:1-9).

### 8. Port notes

Define a compatibility matrix for valid roots, unfinished keys/values/containers, bad escapes, surrogate pairs and trailing garbage. Reuse one native streaming parser across provider/frame consumers and deliberately document any repair policy that differs from TS; do not mistake tolerant parsing for schema validation. Preserve cumulative raw buffers so a later complete argument object can replace partial output deterministically.

### 9. Size and priority

Estimated Rust implementation: **150-300 lines** (estimate, not measured source). **P1** - Different partial parsing changes visible tool argument updates and can lose usable prefixes during streamed calls.

## AI-012 - Fail-closed tool schema validation, coercion and accurate argument errors

### 1. Title and status

Fail-closed tool schema validation, coercion and accurate argument errors. **Status: partial.**

### 2. ice source

ice:packages/ai/src/utils/validation.ts:1-317 (validateToolArguments, JSON Schema coercion); ice:packages/ai/test/validation.test.ts:1-166.

Exported symbols: `StringEnum` (ice:packages/ai/src/utils/typebox-helpers.ts:14).
Exported symbols: `validateToolCall` (ice:packages/ai/src/utils/validation.ts:270).

### 3. What it does

Tool execution checks cloned model-produced arguments against the declared schema after compatibility coercion. A schema-compilation failure must remain a failure rather than silently granting execution with arbitrary arguments.

### 4. How it works

The TS validator caches a compiled TypeBox validator by schema object identity in a WeakMap. It clones the incoming arguments, applies TypeBox conversion, and for plain JSON schemas also applies the custom recursive coercion rules before checking validity.

Object properties, array items and additionalProperties are visited; allOf runs sequentially. For anyOf/oneOf, an already matching original value wins, otherwise coerced clones are tried in order and the first valid candidate is selected. Type arrays preserve an already matching original type. Numeric conversion uses finite JavaScript Number semantics for numeric strings, booleans and null; integer requires Number.isInteger. Boolean coercion recognizes true/false,1/0 and null; string coercion maps null to an empty string and numbers/booleans to text; null schemas recognize empty string,0 and false. Blank numeric strings are excluded. Optional nulls are not first deleted by the current TS implementation (ice:packages/ai/src/utils/validation.ts:1-284).

Compile and clone failures propagate. Normal invalid object arguments throw a formatted tool-specific error with property paths, missing required-property names and the original arguments JSON. There is an exceptional root-primitive branch: if coercion changed the root and the result validates it returns the converted root, otherwise it returns the original arguments without throwing. That source quirk must be resolved explicitly, not described as universally fail-closed TS behavior (ice:packages/ai/src/utils/validation.ts:285-317). The runtime validator is a separate trust gate from provider-side strict/grammar generation.

`validateToolCall` chooses the first tool whose name exactly equals `toolCall.name`, throws `Tool "<name>" not found` when absent, and then calls `validateToolArguments`. `StringEnum(values, options?)` constructs a Type.Unsafe primitive schema `{ type: 'string', enum: values }`; it preserves the supplied value array and includes description/default only when each is truthy, so an empty-string default is omitted. It performs no deduplication or separate value validation. This is schema construction, not a bypass of the subsequent execution validator (ice:packages/ai/src/utils/validation.ts:270-275; ice:packages/ai/src/utils/typebox-helpers.ts:14-23).

### 5. Interfaces

validateToolArguments(tool:Tool, toolCall:ToolCall):Record<string, unknown> (runtime primitives can escape the asserted type); TypeBox Compile/Value.Convert/Value.Check plus plain-schema coercion. Error text includes tool name, property path(s), validator messages and received arguments.

### 6. Dependencies

This is the execution-boundary schema gate consumed by the agent loop and extension tool wrappers. AI-013's provider sampling constraints do not replace it. External TS dependencies are TypeBox compilation/value helpers; native uses jsonschema.

### 7. ice-rust today

Rust implements recursive coercion, formatted errors and compiled-schema caching (rust:crates/ice-ai/src/schema.rs:1-623; rust:crates/ice-ai/tests/coerce_table.rs:1-202). However validate_tool_arguments treats get_sub_schema_validator returning None as permission to compile boolean true, then returns Ok for any value (rust:crates/ice-ai/src/schema.rs:392-407). The agent loop calls this validator before the tool hook/execution path (rust:crates/ice-agent/src/agent_loop.rs:917-990), establishing a reachable fail-open source defect. Rust also deletes optional nulls first (rust:crates/ice-ai/src/schema.rs:299-365), unlike current TS; parses integers through f64 then casts to i64, allowing saturation for large integral values; and caches by serialized schema content in an unbounded process-global map rather than weak identity. Its tests deliberately cover some older null/coercion semantics, not parity with the current TS source.

### 8. Port notes

Return an explicit Schema error when compilation fails and never substitute an always-valid schema. Preserve enough compile diagnostics to identify the tool/schema. Reconcile optional-null and numeric coercion rules, reject lossy or out-of-range integer conversion, and bound/cache validators according to native lifetime requirements. Decide and document a correction for TS's primitive-root bypass rather than reproducing it accidentally. Future implementation verification should exercise malformed schemas through the real agent execution gate as well as coercion tables; this audit made no code or test changes.

### 9. Size and priority

Estimated Rust implementation: **220-420 lines** (estimate, not measured source). **P0** - Invalid tool schemas can currently bypass validation and admit arbitrary model-produced arguments to execution.

## AI-013 - Provider strict-schema and grammar constraints with explicit require/prefer semantics

### 1. Title and status

Provider strict-schema and grammar constraints with explicit require/prefer semantics. **Status: partial.**

### 2. ice source

ice:packages/ai/src/api/constrained-sampling.ts:1-148; ice:packages/ai/test/constrained-sampling.test.ts:1-230; ice:packages/ai/src/types.ts:487-514.

Exported symbols: `GrammarConstrainedSampling` (ice:packages/ai/src/api/constrained-sampling.ts:9).

### 3. What it does

Tools can request provider-enforced JSON schema or a grammar over one string argument. Required constraints fail when unsupported, while preferred constraints can fall back deliberately.

### 4. How it works

resolveJsonSchemaStrictSampling returns true for a supported strict request and undefined when strict is absent or unsupported in prefer mode. An unsupported require request throws, making capability negotiation observable before provider execution.

Grammar use is gated by provider support; an unsupported provider falls back without validating the grammar definition. When supported, a nonblank Lark grammar takes precedence over regex; empty or missing variants fail. Grammar tools must have an object schema with exactly one required string property, and that property becomes the custom tool's input carrier. The buffer helper projects a growing raw grammar string into JSON for that property by escaping text as it appends. Updates must extend the prior raw string; non-prefix rewrites throw. Closing yields the JSON terminator and repeated identical close/update operations avoid duplicate output (ice:packages/ai/src/api/constrained-sampling.ts:1-148).

These are sampling constraints, not local authorization or schema validation. A provider can still fail or return malformed data, so execution must pass AI-012. The helper owns no filesystem or credential state.

### 5. Interfaces

Tool.constrainedSampling: JSON-schema strict=require|prefer or grammar variants.openai_lark/openai_regex; resolveJsonSchemaStrictSampling; resolveGrammarConstrainedSampling→{format, definition, inputProperty}|undefined; GrammarToolInputJsonBuffer {input, started, closed}; appendGrammarToolInputJsonDelta(buffer, property, nextInput, close); getGrammarToolInput; createGrammarToolInputProperties→ReadonlyMap<string, string>.

### 6. Dependencies

Pure negotiation helpers have no prerequisites. AI-012 remains independently required at execution; each provider wire entry supplies its advertised support and payload/event implementation.

### 7. ice-rust today

The native model already carries ConstrainedSamplingConfig/StrictMode, and strict_schema.rs transforms eligible schemas by requiring properties and adding additionalProperties=false (rust:crates/ice-ai/src/strict_schema.rs:1-197). Responses has real custom grammar-tool construction and ID handling (rust:crates/ice-ai/src/providers/openai_responses.rs:972-1085); therefore grammar support is not missing wholesale. However malformed supported grammar schemas can fall back through Option rather than the TS error, and strict_tool_parameters discards the require/prefer distinction. Anthropic resolve_strict_mode also converts an unsupported require request to None despite its comment promising an error (rust:crates/ice-ai/src/providers/anthropic/build_params.rs:562-615). The separate public constrained.rs abstractions and basic grammar validator are not evidence that every adapter honors these modes (rust:crates/ice-ai/src/constrained.rs:1-324).

### 8. Port notes

Make capability resolution return a Result that preserves require failures and prefer fallback. Use one validated grammar input-property/buffer implementation across supporting providers, keep fallback for unsupported grammar exactly documented, and connect advertised compatibility flags to actual request/event paths. Retain strict schema transformation but expose unsupported constructs as explicit errors when required.

### 9. Size and priority

Estimated Rust implementation: **200-400 lines** (estimate, not measured source). **P1** - Silently dropping required generation constraints breaks tool contracts and obscures compatibility failures.

## AI-014 - Cross-model transcript normalization and synthetic tool results

### 1. Title and status

Cross-model transcript normalization and synthetic tool results. **Status: partial.**

### 2. ice source

ice:packages/ai/src/api/transform-messages.ts:1-223 (transformMessages); ice:packages/ai/src/api/anthropic-messages.ts:1093-1096 (Anthropic ID normalization); ice:packages/ai/test/deferred-tools.test.ts:223-253.

Exported symbols: `sanitizeSurrogates` (ice:packages/ai/src/utils/sanitize-unicode.ts:21).

### 3. What it does

A provider request projects stored messages into a history suitable for the target model, removing model-bound signatures and repairing missing tool results. This normalization is independent of the agent/session history projection and accepts an adapter-specific tool-call ID normalizer.

### 4. How it works

transformMessages first normalizes nullish message content to an empty array and downgrades unsupported user/tool images to distinct omission placeholders. It then adjusts assistant blocks for target provider/API/model identity before a second pass repairs missing results and drops error/aborted assistants.

Consecutive images collapse to one placeholder; an existing text block exactly equal to the placeholder suppresses following images, but text blocks themselves are still appended. Same-model redacted thinking is retained and foreign redacted thinking is removed. A same-model truthy thinkingSignature preserves even empty thinking; other empty/whitespace thinking is removed. Nonempty foreign thinking becomes text, foreign text loses textSignature, and foreign tool calls lose thoughtSignature. The optional normalizer receives id, target model and source assistant only for foreign calls; changed IDs are remembered for subsequent results in the same forward pass. Results appearing before their call cannot use a mapping not yet created (ice:packages/ai/src/api/transform-messages.ts:12-157).

At each next assistant or user boundary, and at the end, unresolved pending calls get an isError tool result with text 'No result provided' and current Date.now timestamp. Existing results are retained, including orphan results. Previous pending calls flush before an error/aborted assistant is skipped, and calls inside that skipped assistant do not become pending. This is request projection, not a durable repair of the original session and not execution of any tool. Anthropic's callback replaces each disallowed UTF-16 code unit with '_' and slices to 64 units.

Before wire serialization, `sanitizeSurrogates(text)` removes unpaired UTF-16 high surrogates and unpaired low surrogates using a non-Unicode global regular expression; it preserves valid surrogate pairs and all other text. This helper is a request-text projection and does not rewrite persisted messages (ice:packages/ai/src/utils/sanitize-unicode.ts:21-24).

### 5. Interfaces

transformMessages<TApi>(messages, model, normalizeToolCallId?:(id, model, sourceAssistant)=>string):Message[]; target identity provider/api/id; input-modality downgrade; ToolCall.id and ToolResultMessage.toolCallId relationship.

### 6. Dependencies

No source-runtime prerequisite for pure projection. Provider-specific request builders consume it; SESSION-029/034 own durable history and AGENT-065 owns a different agent-to-provider projection layer.

### 7. ice-rust today

Rust implements nearly the whole transform in the Anthropic request module, including signature removal, omission placeholders, pending-call repair and skipped failed assistants (rust:crates/ice-ai/src/providers/anthropic/build_params.rs:223-478). Its helper hardcodes Anthropic ID normalization rather than accepting a provider callback; synthetic result timestamps are 0 rather than current time. It trims the signature when deciding presence, unlike TS truthiness, and can turn a foreign empty thinking block with a nonblank signature into empty text instead of dropping it; later Anthropic serialization normally filters that text. Its scalar-value ID replacement emits one underscore for an astral character where the TS non-Unicode regex emits two. Typed deserialization rejects null content instead of admitting it to this helper; that boundary difference is deliberate and should be documented. Other native adapters have separate projection logic and require their own review.

Native `String` already excludes lone surrogate code points. Anthropic therefore implements its named sanitizer as a pass-through, while the image adapter's scalar filter cannot encounter a surrogate in a valid `&str`; these are representation differences, not a missing native text-cleaning feature (rust:crates/ice-ai/src/providers/anthropic/build_params.rs:704-709; rust:crates/ice-ai/src/images.rs:426-433).

### 8. Port notes

Expose a shared native projection with an adapter-specific ID policy and an injected timestamp source if observable parity is required. Preserve ordering and retained orphan-result behavior, then reconcile each adapter's existing implementation against it. Decide explicitly whether malformed imported null content should be normalized at deserialization; avoid weakening native typed interfaces merely to imitate untyped JavaScript callers.

### 9. Size and priority

Estimated Rust implementation: **140-280 lines** (estimate, not measured source). **P2** - Model switching and imported histories can produce different replay payloads and tool-result identities.

## AI-015 - Image-provider collections, catalog refresh and resolved-auth generation

### 1. Title and status

Image-provider collections, catalog refresh and resolved-auth generation. **Status: partial.**

### 2. ice source

ice:packages/ai/src/images-models.ts:8-95 (interfaces), ice:packages/ai/src/images-models.ts:97-276 (collection and factory), ice:packages/ai/src/image-models.ts:1-43 (legacy generated lookup), ice:packages/ai/src/images-api-registry.ts:1-54; ice:packages/ai/test/images-models.test.ts:73-208; ice:packages/ai/scripts/generate-image-models.ts:36-163.

Exported symbols: `ImagesApiFunction`, `ImagesApiProvider` (ice:packages/ai/src/images-api-registry.ts:3; ice:packages/ai/src/images-api-registry.ts:9).
Exported symbols: `CreateImagesProviderOptions` (ice:packages/ai/src/images-models.ts:231).
Exported symbols: `KnownImagesApi`, `ImagesApi`, `KnownImagesProvider`, `ImagesProviderId`, `ProviderImages`, `ProviderImagesOptions`, `ImagesFunction`, `ImagesInputContent`, `ImagesOutputContent`, `ImagesStopReason` (ice:packages/ai/src/types.ts:30; ice:packages/ai/src/types.ts:32; ice:packages/ai/src/types.ts:76; ice:packages/ai/src/types.ts:78; ice:packages/ai/src/types.ts:297; ice:packages/ai/src/types.ts:313; ice:packages/ai/src/types.ts:340; ice:packages/ai/src/types.ts:464; ice:packages/ai/src/types.ts:465; ice:packages/ai/src/types.ts:471).

### 3. What it does

Manage image-generation providers and models as runtime objects with auth resolution and refresh, alongside the existing legacy API-ID dispatcher.

### 4. How it works

TS has both a legacy image API registry and a provider-owned ImagesModels collection. The latter combines model listing, dynamic refresh and credential resolution before dispatching to the owning image provider.

The collection defaults to an in-memory credential store and default auth context. Providers are keyed by id in insertion order; set replaces, delete/clear remove. getModels is synchronous and best-effort, swallowing a provider's listing exception; getModel scans the last-known list. refresh(id) is a no-op for unknown/static providers, preserves an existing ModelsError or wraps other failures as model_source. refresh() runs all hooks concurrently through allSettled. createImagesProvider preserves old models on asynchronous refresh failure and shares an in-flight refresh; a synchronous throw before its first await can leave the rejected promise installed after the inner finally, so the documented retry policy is not unconditional for ill-behaved hooks.

getAuth accepts provider ID or model and delegates to shared auth resolution. Generation resolves by model.provider; explicit request key wins via nullish precedence, headers/env merge by key with explicit options last, and an auth base URL replaces the request model's endpoint. Unknown provider and caught failures produce an empty image error result. When no auth result exists, generation still dispatches to the provider. That branch returns provider.generateImages without await inside the try, so an asynchronous rejection can escape despite the interface's 'never rejects' documentation. The resolved-auth branch awaits and catches. Neither collection error conversion specially preserves an aborted reason.

Legacy image models come from generated IMAGE_MODELS, indexed by provider/model; an unknown runtime key can return undefined despite the nonoptional typed lookup. The legacy API registry replaces by API ID and wraps calls with an exact model.api check. It records sourceId but has no unregister or source-cleanup API in this file. Top-level legacy generateImages throws/rejects for an unknown API. The image generator selects output-image models from OpenRouter, filters input/output modalities to text/image, defaults empty input to text, multiplies catalog per-token prices by one million and sorts IDs before writing. Strict mode rejects missing/empty/no-usable catalogs; nonstrict fetch failure can generate an empty catalog. Generator and consumer were read, while repetitive generated image values are explicitly grouped rather than claimed individually read.

### 5. Interfaces

ImagesProvider, ImagesModels, MutableImagesModels, createImagesModels, createImagesProvider; getProviders/getProvider/getModels/getModel/refresh/getAuth/generateImages/setProvider/deleteProvider/clearProviders; legacy registerImagesApiProvider(provider, sourceId?), getImagesApiProvider, getImageModel/getImageProviders/getImageModels.

Exact image aliases and records: `KnownImagesApi = 'openrouter-images'`; `ImagesApi = KnownImagesApi | (string & {})`; `KnownImagesProvider = 'openrouter'`; `ImagesProviderId = KnownImagesProvider | string`; `ImagesInputContent = TextContent | ImageContent`; `ImagesOutputContent = TextContent | ImageContent`; `ImagesStopReason = 'stop' | 'error' | 'aborted'`. `ProviderImages.generateImages(model, context, options?)` and `ImagesFunction` return `Promise<AssistantImages>`; `ProviderImagesOptions = ImagesOptions & Record<string, unknown>`. `ImagesContext { input: ImagesInputContent[] }`; `AssistantImages` requires api/provider/model/output/stopReason/timestamp, with optional responseId/usage/errorMessage. The metadata option extends the shared request envelope (ice:packages/ai/src/types.ts:30-32; ice:packages/ai/src/types.ts:76-78; ice:packages/ai/src/types.ts:297-313; ice:packages/ai/src/types.ts:340-344; ice:packages/ai/src/types.ts:464-483).

### 6. Dependencies

`AI-002`, `AI-003`, `AUTH-001`, `AUTH-008`

Reuse provider/auth abstractions while retaining image-specific result/model types. AI-057 owns the concrete OpenRouter image HTTP behavior; that adapter and this collection can be developed independently around the shared interfaces.

### 7. ice-rust today

A real global native image API registry and OpenRouter adapter already exist at rust:crates/ice-ai/src/images.rs:149-211 and rust:crates/ice-ai/src/images.rs:217-335. The RwLock registry supports register/lookup, and unknown API returns an AssistantImages error, a deliberate difference from the legacy TS rejection. Native ImagesModel at rust:crates/ice-ai/src/images.rs:58-84 lacks the TS display name/input-modality metadata and generated provider catalog. The full 511-line module has no provider-ID collection, dynamic model-list refresh, auth resolution, credential-store injection or explicit per-provider listing contract. Its registered callback can also panic; the comment claiming it never panics is not a catch boundary. Existing image dispatch/output types should be retained.

### 8. Port notes

Add the provider-owned image collection and generated/imported catalog above the existing native API dispatcher. Reuse shared auth resolution and explicit option precedence. Make generation failures consistently return structured results, including the unauthenticated branch, and make refresh cleanup robust to synchronous failures. Preserve native unknown-API error results as an explicit design choice and document source ownership rather than inventing an unregister contract.

### 9. Size and priority

Estimated Rust implementation: **240-410 lines** (estimate, not measured source). **P2** - Image generation exists, but callers cannot obtain the same provider catalog, refresh lifecycle or auth-prepared generation behavior.

## AI-016 - Request-scoped provider environment and proxy precedence

### 1. Title and status

Request-scoped provider environment and proxy precedence. **Status: partial.**

### 2. ice source

ice:packages/ai/src/utils/provider-env.ts:1-51; ice:packages/ai/src/utils/node-http-proxy.ts:1-112; ice:packages/ai/test/provider-env-legacy-compat.test.ts:1-23; ice:packages/ai/test/node-http-proxy.test.ts:1-77.

### 3. What it does

A request can supply environment values without mutating the process, and transports resolve proxy routing consistently from scoped and ambient variables. Historical PI-prefixed variables are not treated as aliases for current ICE provider settings.

### 4. How it works

getProviderEnvValue checks an own property in the supplied environment first and falls back only when its value is undefined; an explicit empty string therefore remains meaningful. Ambient lookup uses process.env or a once-cached Bun/process fallback, including a cached empty result on read failure.

Proxy resolution separately uses truthy precedence: scoped lowercase, scoped uppercase, ambient lowercase, ambient uppercase. An empty scoped value does not suppress ambient proxy selection. HTTP_PROXY/HTTPS_PROXY and ALL_PROXY provide protocol-specific/fallback routing, and NO_PROXY accepts comma/whitespace-separated host patterns, optional ports, exact hosts, suffix/star forms and '*' for global bypass. An invalid target URL returns undefined; a selected invalid proxy throws. Missing schemes are prefixed appropriately and only HTTP/HTTPS proxies are accepted. The explicit resolver owns no authentication persistence and does not mutate environment variables. The difference between general environment lookup and proxy truthy selection is intentional source behavior, not interchangeable helper semantics.

### 5. Interfaces

getProviderEnvValue(name:string, env?:ProviderEnv):string|undefined; resolveHttpProxyUrlForTarget(targetUrl:string|URL, env?:ProviderEnv):URL|undefined; UNSUPPORTED_PROXY_PROTOCOL_MESSAGE; uppercase/lowercase HTTP_PROXY, HTTPS_PROXY, ALL_PROXY, NO_PROXY.

### 6. Dependencies

No prerequisite for pure resolution. AI-006 must carry per-request env into native transport construction; SET-005 owns coding-agent environment/header configuration.

### 7. ice-rust today

Native http.rs has a real proxy matcher, target URL parsing, supported-proxy validation and scoped environment interface (rust:crates/ice-ai/src/http.rs:1-286). Its environment helper selects the first defined value including empty strings, unlike TS proxy truthy precedence. Invalid targets return an error rather than undefined, and an out-of-range parsed NO_PROXY port falls back to zero, which behaves as no port restriction. The client builder applies an explicit proxy when resolved; its no-explicit-proxy branch does not itself disable reqwest ambient proxy discovery, so exact effective bypass semantics require dependency-level verification rather than a source-only claim that routing is identical. The public StreamOptions still lacks request env (AI-006).

### 8. Port notes

Carry scoped environment through request options and make the distinction between own-property environment lookup and proxy truthy precedence explicit. Parse NO_PROXY ports without treating malformed/overflow values as wildcard ports. Configure the native client so the resolver's bypass/selection result is authoritative, and document platform/environment differences without restoring obsolete PI aliases.

### 9. Size and priority

Estimated Rust implementation: **130-260 lines** (estimate, not measured source). **P1** - Proxy and credential-related environment precedence can route requests differently from the caller's scoped configuration.

## AI-017 - Standalone AI-package OAuth CLI and current-directory credential file

### 1. Title and status

Standalone AI-package OAuth CLI and current-directory credential file. **Status: different-by-design.**

### 2. ice source

ice:packages/ai/src/cli.ts:8-43 (provider discovery, file and prompts), ice:packages/ai/src/cli.ts:45-77 (login), ice:packages/ai/src/cli.ts:79-120 (commands and exit).

### 3. What it does

Provide or explicitly retire the separate AI-package login/list command whose credential file is relative to the invocation directory.

### 4. How it works

The standalone TS AI-package CLI filters built-in providers to those declaring OAuth and exposes help, list and login. It is separate from the coding-agent's auth command and credential-storage layout.

Missing command/help/--help/-h prints usage and the OAuth provider list. list prints IDs and names. login accepts an exact provider ID or prompts for a one-based choice; selection parsing uses parseInt and rejects an absent selected entry. Select auth prompts print numbered labels and return option IDs; text prompts include a placeholder when present and use readline without secret masking. Notifications print auth URLs, device codes, instructions, progress and information. The login receives a fresh AbortController signal that this CLI never explicitly aborts, and readline is closed in finally.

AUTH_FILE is the literal relative path auth.json. loadAuth returns an empty object for a missing file or any read/JSON error; it performs no schema validation. Login obtains the credential before re-reading the file, assigns by provider ID and writes formatted JSON synchronously. There is no lock, atomic replacement, permission-mode request or preservation of corrupt contents in this module. A top-level error prints Error: plus its message and exits one; successful help/list/login returns normally. This audit examined the code only and did not open any real auth.json file.

### 5. Interfaces

npx @zykairotis/ice-ai [help|list|login [provider]]; AUTH_FILE='auth.json' relative to process cwd; AuthPrompt select/text and auth_url/device_code/info/progress notifications; OAuthCredential map keyed by provider.

### 6. Dependencies

`AUTH-015`

OAuth provider flows and the coding-agent/native auth UI are owned by AUTH. This entry is only the standalone packaging, invocation and credential-path contract.

### 7. ice-rust today

Native auth is integrated into the application command at rust:crates/ice-cli/src/auth.rs:35-68, with extension-aware dispatch and the credential helpers imported at rust:crates/ice-cli/src/auth.rs:25-27. AUTH's full source review documents persistent host credential management and provider-owned extension OAuth. That is a meaningful native replacement, but does not reproduce the TS AI-package executable or its cwd-relative auth.json convention. Rust library distribution also differs from an npx package; absence of a second executable is not by itself a defect.

### 8. Port notes

Decide whether standalone AI-library consumers need a compatibility command. If yes, add a small facade over native auth/provider interfaces with an explicit credential path and safe write/secret-prompt behavior; avoid copying the source's silent-corruption overwrite and unmasked secret prompt. If integrated native auth is the intended product surface, document the migration and keep this difference intentional.

### 9. Size and priority

Estimated Rust implementation: **80-150 lines** (estimate, not measured source). **P3** - This affects standalone library onboarding rather than the primary native agent runtime; the integrated native auth path already exists.

## AI-018 - Provider continuation diagnostics, request sequences and bounded identifier hashes

### 1. Title and status

Provider continuation diagnostics, request sequences and bounded identifier hashes. **Status: missing.**

### 2. ice source

ice:packages/ai/src/utils/provider-continuation-diag.ts:1-245; ice:packages/ai/test/provider-continuation-diag.test.ts:1-68.

Exported symbols: `ContinuationRequestContextMode`, `ContinuationRequestDiag`, `ContinuationResponseDiag`, `ContinuationErrorDiag`, `isContinuationDiagEnabled`, `hashOpaqueId`, `nextContinuationRequestSequence`, `rememberSuccessfulResponseId`, `forgetSuccessfulResponseId`, `isStalePreviousResponseError`, `lastSuccessfulResponseIdHash`, `classifyRequestContextMode`, `logContinuationResponse`, `logContinuationError`, `extractProviderErrorFields`, `inputItemCount`, `resetContinuationDiagForTests` (ice:packages/ai/src/utils/provider-continuation-diag.ts:5; ice:packages/ai/src/utils/provider-continuation-diag.ts:7; ice:packages/ai/src/utils/provider-continuation-diag.ts:31; ice:packages/ai/src/utils/provider-continuation-diag.ts:45; ice:packages/ai/src/utils/provider-continuation-diag.ts:96; ice:packages/ai/src/utils/provider-continuation-diag.ts:104; ice:packages/ai/src/utils/provider-continuation-diag.ts:113; ice:packages/ai/src/utils/provider-continuation-diag.ts:118; ice:packages/ai/src/utils/provider-continuation-diag.ts:123; ice:packages/ai/src/utils/provider-continuation-diag.ts:128; ice:packages/ai/src/utils/provider-continuation-diag.ts:142; ice:packages/ai/src/utils/provider-continuation-diag.ts:147; ice:packages/ai/src/utils/provider-continuation-diag.ts:192; ice:packages/ai/src/utils/provider-continuation-diag.ts:196; ice:packages/ai/src/utils/provider-continuation-diag.ts:200; ice:packages/ai/src/utils/provider-continuation-diag.ts:236; ice:packages/ai/src/utils/provider-continuation-diag.ts:242).

### 3. What it does

Continuation-capable providers can record how each request reuses a previous response, which input projection was sent, and why a stale continuation was reset. The diagnostic path uses identifier hashes for correlation while retaining provider/request metadata.

### 4. How it works

The enable helper recognizes ICE_PROVIDER_CONTINUATION_DIAG values 0/false/off and 1/true/on; otherwise Vitest disables it and the exact provider ID codexlb is enabled by default. Logging and enablement are separate functions, so calling the writer directly is not automatically gated.

A process-local global counter assigns request sequences, while a separate map remembers the last successful response ID per supplied session key. The map has explicit remember/forget operations but no automatic eviction, and neither state is durable or coordinated across processes. Records include model/provider/base URL, message/item/tool counts, selected continuation fields and response metadata. Request context mode is a diagnostic heuristic: no previous ID means full; an unknown input item count means continuation-delta; otherwise input item count at least the message count means full-with-id and a smaller count means continuation-delta. It is not proof of how the remote service interpreted the request.

Identifier hashing returns null for empty input, SHA-256 with a sha256: prefix and first 16 hex characters when Node builtin crypto is available, otherwise h: plus shortHash. Stale continuation is recognized by exact previous_response_not_found; invalid_request_error requires a previous_response_id parameter/message mention, and the remaining fallback requires that identifier mention together with invalid or not found. Error extraction reads numeric status/statusCode and string code/message/param from the SDK-shaped error object, without searching arbitrary nested data.

The default source-defined file is ~/.ice/agent/logs/provider-continuation-diag.jsonl, overrideable through trimmed nonempty ICE_PROVIDER_CONTINUATION_DIAG_FILE. The writer serializes a record once, synchronously creates the directory and appends JSONL when builtin fs/path support exists; filesystem errors are swallowed. ICE_PROVIDER_CONTINUATION_DIAG_STDERR=1 also mirrors the line to stderr with its own caught write errors. JSON.stringify occurs before either catch, so arbitrary unserializable records are not guaranteed harmless. logContinuationRequest/Response/Error prepend phase before spreading the supplied record; an untyped caller could override phase. No real runtime directory was inspected.

Hashing is selective: provider, base URL and some account/upstream metadata remain plain text, and arbitrary records are not a general redaction boundary. There is no rotation or retention policy in this module. Diagnostics must not be mistaken for safe persistence of continuation state.

### 5. Interfaces

Continuation diagnostic enable/reset/record helpers; ICE_PROVIDER_CONTINUATION_DIAG, ICE_PROVIDER_CONTINUATION_DIAG_FILE, ICE_PROVIDER_CONTINUATION_DIAG_STDERR; JSONL records with type/time/provider/model/session hashes, sequence, input mode/counts and previous-response information.

### 6. Dependencies

`AI-009`

AI-009 provides the shared assistant diagnostic vocabulary; this entry owns the separate source-defined JSONL continuation trace. Concrete continuation request behavior belongs to the Responses/Codex wire entries.

### 7. ice-rust today

The native common request options and AssistantMessage expose session/response identifiers, and Responses has its own request state and wire path; these are real continuation building blocks (rust:crates/ice-ai/src/provider.rs:31-88; rust:crates/ice-ai/src/types.rs:654-701). No counterpart to this enable policy, per-session diagnostic sequence and JSONL writer exists in the fully inventoried ice-ai modules. Native generic observability does not reproduce this particular diagnostic artifact or metadata contract.

### 8. Port notes

Add an opt-in native continuation diagnostic sink and source-compatible metadata schema, using active-product ICERS_* and .icers identity where appropriate at final integration. Keep enablement centralized so direct writer calls cannot accidentally bypass it, bound in-memory session state and define file retention/redaction. Preserve external provider protocol names rather than mechanically renaming them.

### 9. Size and priority

Estimated Rust implementation: **170-330 lines** (estimate, not measured source). **P2** - Continuation resets and cache-affinity failures are difficult to diagnose without request-sequence evidence.

## AI-019 - Complete shared message, usage, content and model data contracts

### 1. Title and status

Complete shared message, usage, content and model data contracts. **Status: partial.**

### 2. ice source

ice:packages/ai/src/types.ts:16-99; ice:packages/ai/src/types.ts:346-546; ice:packages/ai/src/types.ts:791-847. Focused tests read: ice:packages/ai/test/lax-message-content.test.ts:1-68; ice:packages/ai/test/openai-completions-response-model.test.ts:1-141. Native comparison: rust:crates/ice-ai/src/types.rs:1-1157; rust:crates/ice-ai/src/model.rs:1-284; rust:crates/ice-ai/src/lib.rs:1-47; rust:crates/ice-ai/tests/content_json.rs:1-134. Bounded active event serializer caller: rust:crates/ice-cli/src/remote/protocol.rs:181-214.

Exported symbols: `calculateCost` (ice:packages/ai/src/models.ts:878).
Exported symbols: `KnownApi`, `KnownProvider`, `GrammarFormat`, `GrammarVariants`, `ModelCostRates`, `ModelCostTier`, `ModelCost` (ice:packages/ai/src/types.ts:16; ice:packages/ai/src/types.ts:34; ice:packages/ai/src/types.ts:488; ice:packages/ai/src/types.ts:490; ice:packages/ai/src/types.ts:791; ice:packages/ai/src/types.ts:798; ice:packages/ai/src/types.ts:803).

### 3. What it does

Keep the exact nested provider message and model vocabulary available to agent, session, extension and RPC consumers. This entry fills the explicit contract gap between AGENT-001's imported Message types and AI-006's request options while preserving the substantial native data model already present.

### 4. How it works

Every provider-facing conversation consists of user, assistant and tool-result records with exact role tags and role-specific content. Usage, response provenance and deferred-response metadata travel with those records; streamed updates carry the same assistant shape. The declarations specify the data contract but do not themselves validate, normalize, persist, retry or cancel anything.

Content and identity: UserMessage.timestamp, AssistantMessage.timestamp and ToolResultMessage.timestamp are Unix timestamps in milliseconds. textSignature is a string, including a legacy provider ID or JSON-encoded TextSignatureV1; it is not an inline signature object. Thinking signatures and ToolCall.thoughtSignature are opaque continuity data. redacted=true means the thinking payload was filtered and the opaque encrypted continuation is in thinkingSignature; no requirement here says to display it. Image data is base64 text with a MIME string, without decoding/validation in the type. A user can use one text string or a text/image array; an assistant array permits text/thinking/toolCall but not images, and tool-result arrays permit text/images. Tool calls use id/name/arguments while tool results use toolCallId/toolName; transport renaming belongs to the consuming adapter. These types do not prove call-result identity or schema correctness (ice:packages/ai/src/types.ts:346-380; ice:packages/ai/src/types.ts:421-462).

Usage and provenance: all five token counters and five cost fields are required except cacheWrite1h and reasoning, which are optional breakdowns. cacheWrite1h is a subset of cacheWrite, and reasoning is a subset of output; adding either a second time double-counts usage. Reported zero reasoning remains distinguishable from no breakdown. The type neither recomputes totals nor enforces finite, integer, positive, or internally consistent numbers. Tool-result usage belongs to the tool execution and is not a new main-model context checkpoint. Assistant.model identifies the requested model; responseModel can identify a different actual routed model and responseId is the upstream response identity. The focused response-model suite asserts different/identical/empty/missing chunk-model cases without changing the requested model, with adapter implementation retained under AI-052 (ice:packages/ai/src/types.ts:382-403; ice:packages/ai/src/types.ts:427-459; ice:packages/ai/test/openai-completions-response-model.test.ts:56-139).

Terminal and deferred state: StopReason is exactly pending/stop/length/toolUse/error/aborted/deferred. DeferredHandle requires provider, modelId, api and id, with optional expiresAt, pollAfterMs and recursive JSON data; explicit data:null is in its declared domain. The interface does not make deferred mandatory when stopReason is deferred, restrict timestamp values, or require errorMessage for every error object. The stream documentation expects start, partial updates, then done or error; terminal reason types narrow the outer reason but do not make the nested assistant's stopReason statically equal to it. Consumers must preserve this distinction between documented lifecycle expectations, the structural union and actual provider/event-stream enforcement. There is no new timeout, polling scheduler or cancellation authority in these DTOs (ice:packages/ai/src/types.ts:405-442; ice:packages/ai/src/types.ts:522-546).

Context and optional values: Context requires messages, with optional systemPrompt and tools. ToolResultMessage.details is generic/any at the in-process boundary and is not inherently JSON, whereas DeferredHandle.data is JsonValue. addedToolNames identifies which Context.tools became available after that result; the type owns neither tool registration nor autoload policy. Required content remains required in the declared schema: the separate AI-014 transform intentionally admits untyped null/missing content and normalizes it to [], as the focused lax-message-content suite documents. Do not infer a globally optional content field from that defensive path (ice:packages/ai/src/types.ts:444-520; ice:packages/ai/test/lax-message-content.test.ts:1-68).

Model metadata: Model holds the required endpoint/identity/input/cost/window/max-output data and optional thinking map, advertised service tiers, sampling defaults, headers and an API-conditioned compatibility object. Cost fields are dollars per million tokens. Pricing tiers attach inputTokensAbove (total input usage must exceed the threshold) and select the highest matching threshold request-wide, as documented by the type; the calculation itself belongs to existing model/provider cost logic. No type-level enforcement makes a window positive or a price nonnegative. Keep this rich internal shape separate from RPC-003/004's deliberately reduced, normalized metadata. Model and usage contracts contain no filesystem paths, credentials lookup or network operation by themselves (ice:packages/ai/src/types.ts:791-840).

The exported `calculateCost(model, usage)` selects rates using total billable input `usage.input + usage.cacheRead + usage.cacheWrite`. It chooses the largest tier threshold strictly exceeded, starting matchedThreshold at -1. It mutates and returns the existing `usage.cost`: input/output/cacheRead are their counts times the respective per-million rates; cacheWrite is `(rates.cacheWrite * (usage.cacheWrite - (usage.cacheWrite1h ?? 0)) + rates.input * 2 * (usage.cacheWrite1h ?? 0)) / 1_000_000`; total sums the four resulting components. There is no validation or clamping of malformed numeric counters in this helper (ice:packages/ai/src/models.ts:878-897).

### 5. Interfaces

Type-only contract from ice:packages/ai/src/types.ts:346-546 and ice:packages/ai/src/types.ts:791-840. A question mark means optional at the TS API boundary; these declarations add no runtime validator or serializer.

```ts
interface TextSignatureV1 { v: 1; id: string; phase?: "commentary" | "final_answer" }
interface TextContent { type: "text"; text: string; textSignature?: string }
interface ThinkingContent { type: "thinking"; thinking: string; thinkingSignature?: string; redacted?: boolean }
interface ImageContent { type: "image"; data: string; mimeType: string }
interface ToolCall { type: "toolCall"; id: string; name: string; arguments: Record<string, any>; thoughtSignature?: string }
interface Usage {
  input: number; output: number; cacheRead: number; cacheWrite: number;
  cacheWrite1h?: number; reasoning?: number; totalTokens: number;
  cost: { input: number; output: number; cacheRead: number; cacheWrite: number; total: number };
}
type StopReason = "pending" | "stop" | "length" | "toolUse" | "error" | "aborted" | "deferred";
type JsonValue = string | number | boolean | null | JsonValue[] | { [key: string]: JsonValue };
interface DeferredHandle { provider: string; modelId: string; api: string; id: string; expiresAt?: number; pollAfterMs?: number; data?: JsonValue }
interface UserMessage { role: "user"; content: string | (TextContent | ImageContent)[]; timestamp: number }
interface AssistantMessage {
  role: "assistant"; content: (TextContent | ThinkingContent | ToolCall)[];
  api: Api; provider: ProviderId; model: string; responseModel?: string; responseId?: string;
  diagnostics?: AssistantMessageDiagnostic[]; usage: Usage; stopReason: StopReason;
  deferred?: DeferredHandle; errorMessage?: string; rawStopReason?: string; timestamp: number;
}
interface ToolResultMessage<TDetails = any> {
  role: "toolResult"; toolCallId: string; toolName: string; content: (TextContent | ImageContent)[];
  details?: TDetails; usage?: Usage; addedToolNames?: string[]; isError: boolean; timestamp: number;
}
type Message = UserMessage | AssistantMessage | ToolResultMessage;
type GrammarFormat = "openai_lark" | "openai_regex";
type GrammarVariants = Partial<Record<GrammarFormat, string>>;
type ConstrainedSamplingConfig =
  | { type: "json_schema"; strict: "prefer" | "require" }
  | { type: "grammar"; variants: GrammarVariants };
interface Tool<TParameters extends TSchema = TSchema> { name: string; description: string; parameters: TParameters; constrainedSampling?: false | ConstrainedSamplingConfig }
interface Context { systemPrompt?: string; messages: Message[]; tools?: Tool[] }
type AssistantMessageEvent =
  | { type: "start"; partial: AssistantMessage }
  | { type: "text_start"; contentIndex: number; partial: AssistantMessage }
  | { type: "text_delta"; contentIndex: number; delta: string; partial: AssistantMessage }
  | { type: "text_end"; contentIndex: number; content: string; partial: AssistantMessage }
  | { type: "thinking_start"; contentIndex: number; partial: AssistantMessage }
  | { type: "thinking_delta"; contentIndex: number; delta: string; partial: AssistantMessage }
  | { type: "thinking_end"; contentIndex: number; content: string; partial: AssistantMessage }
  | { type: "toolcall_start"; contentIndex: number; partial: AssistantMessage }
  | { type: "toolcall_delta"; contentIndex: number; delta: string; partial: AssistantMessage }
  | { type: "toolcall_end"; contentIndex: number; toolCall: ToolCall; partial: AssistantMessage }
  | { type: "done"; reason: "stop" | "length" | "toolUse" | "deferred"; message: AssistantMessage }
  | { type: "error"; reason: "aborted" | "error"; error: AssistantMessage };
interface ModelCostRates { input: number; output: number; cacheRead: number; cacheWrite: number }
interface ModelCostTier extends ModelCostRates { inputTokensAbove: number }
interface ModelCost extends ModelCostRates { tiers?: ModelCostTier[] }
interface Model<TApi extends Api> {
  id: string; name: string; api: TApi; provider: ProviderId; baseUrl: string; reasoning: boolean;
  thinkingLevelMap?: ThinkingLevelMap; input: ("text" | "image")[]; cost: ModelCost;
  contextWindow: number; maxTokens: number; serviceTiers?: string[];
  samplingParams?: Record<string, unknown>; headers?: Record<string, string>;
  compat?: TApi extends "openai-completions" ? OpenAICompletionsCompat
    : TApi extends "openai-responses" | "azure-openai-responses" | "openai-codex-responses" ? OpenAIResponsesCompat
    : TApi extends "anthropic-messages" ? AnthropicMessagesCompat
    : TApi extends "bedrock-converse-stream" ? BedrockCompat : never;
}
```

Api is the known API literal union plus an open string; ProviderId is likewise open. KnownApi includes openai-completions, mistral-conversations, openai-responses, azure-openai-responses, openai-codex-responses, anthropic-messages, bedrock-converse-stream, google-generative-ai, google-vertex and ice-messages (ice:packages/ai/src/types.ts:16-28). Model.compat selects OpenAICompletionsCompat for openai-completions, OpenAIResponsesCompat for openai/azure/codex Responses, AnthropicMessagesCompat for anthropic-messages, BedrockCompat for bedrock-converse-stream, and never for other APIs at the TS generic type boundary. Runtime custom JSON admission is separately owned.
ThinkingLevelMap is a partial mapping from ModelThinkingLevel (off/minimal/low/medium/high/xhigh/max/ultra) to string|null; missing uses provider default and null means unsupported. ThinkingLevel alone excludes off. AI-005 owns availability/clamping. AssistantMessageDiagnostic's exact record/helper is AI-009. Image-generation DTOs ImagesContext/AssistantImages/ImagesModel remain AI-015/057; image content blocks and their Usage dependency use the base definitions above.

### 6. Dependencies

`AI-009`

AI-009 supplies the referenced AssistantMessageDiagnostic record and optional diagnostic carrier; its implementation/LOC stays there. AI-005 (thinking behavior), AI-006 (request options), AI-012/013 (tool validation/sampling), AI-014 (history normalization), AI-015/057 (image generation), AUTH-033 (compatibility fields), and AI-052/054 (wire provenance/signatures) remain related owners, not prerequisites for a second provider implementation. AGENT-001 and RPC-003/004/034 consume this schema and should link to AI-019. Existing native stop-reason defect RUST-035 is cross-linked, not counted or recreated. No consumer is made a build prerequisite of the shared types.

### 7. ice-rust today

Most of the contract already exists. types.rs defines concrete text/thinking/image/tool-call blocks with marker serializers, Usage/UsageCost with optional cache_write_1h/reasoning, all seven stop-reason variants, open provider IDs, DeferredHandle, concrete messages, Context and all twelve assistant event variants. Content manually delegates Serialize to the inner struct so the type tag is emitted once; concrete marker deserializers reject an incorrect supplied literal, while the default marker permits an omitted type only when deserializing a standalone concrete block; tagged Content still requires its type, and the raw JSON regression suite verifies content discriminator uniqueness plus concrete user/assistant/tool-result round trips (rust:crates/ice-ai/src/types.rs:53-223; rust:crates/ice-ai/src/types.rs:277-383; rust:crates/ice-ai/src/types.rs:529-793; rust:crates/ice-ai/src/types.rs:857-1157; rust:crates/ice-ai/tests/content_json.rs:39-133). The tests were read, not run.

The native representation deliberately has a larger content domain: UserContent::Blocks, AssistantMessage.content and ToolResultMessage.content all use the common Content enum, so assistant images and user/tool-result thinking or tool calls are representable. ToolCall.arguments is any serde_json::Value rather than the TS Record shape, consistent with keeping partial parse results until execution validation. Native ToolCall.namespace and AssistantMessage.end_turn are additional retained fields; they must not be removed merely to emulate TS. The base message enum manually deserializes through Value after reading role, while agent custom-message encoding is a separate AGENT-001 concern (rust:crates/ice-ai/src/types.rs:172-223; rust:crates/ice-ai/src/types.rs:549-679; rust:crates/ice-ai/src/types.rs:755-793).

Exact remaining differences: (1) StopReason derives serde rename_all=lowercase, giving ToolUse the derived spelling tooluse, while StopReason::as_str and event DoneReason::as_str emit toolUse. Existing RUST-035 owns this source-proven mismatch. AssistantMessageEvent's manual serializer writes the outer reason via as_str and the nested assistant through its derived serializer; native remote MessageUpdate projects that event with serde_json::to_value, so the spelling difference reaches the current JSONL event payload (rust:crates/ice-ai/src/types.rs:390-414; rust:crates/ice-ai/src/types.rs:943-977; rust:crates/ice-ai/src/types.rs:1140-1152; rust:crates/ice-cli/src/remote/protocol.rs:181-214). The content_json assistant fixture uses ToolUse but only checks same-type round-trip, not TS spelling.

(2) The outer Message enum still derives an internally tagged Serialize with role even though each concrete message emits role. This is a source-derived duplicate-role risk on raw Message/Context JSON serialization, analogous to the already fixed Content pattern; the inspected regression suite does not instantiate Message or count raw role tags. Message's custom Value-based deserializer can normalize duplicates before constructing a concrete record. No raw output or production failure was reproduced in this audit, and no claim is made that the current RPC Value projection preserves duplicate keys. Coordinator defect candidate evidence records this exact boundary separately (rust:crates/ice-ai/src/types.rs:212-223; rust:crates/ice-ai/src/types.rs:549-614; rust:crates/ice-ai/src/types.rs:654-698; rust:crates/ice-ai/src/types.rs:755-793; rust:crates/ice-ai/tests/content_json.rs:39-118).

(3) Optional redacted is represented as default false with false omitted; added_tool_names, cost.tiers and Context.tools are default vectors omitted when empty. Explicit null for Option<Value> details/deferred data deserializes as None and is then omitted. These normalizations erase distinctions that the TS in-process shapes can retain. Native counters/timestamps/tier thresholds use i64, windows/maxTokens use u64, and event indices use usize; TS number types alone do not enforce those ranges. This is a representation boundary to specify, not evidence that ordinary nonnegative integral provider usage is absent (rust:crates/ice-ai/src/types.rs:81-114; rust:crates/ice-ai/src/types.rs:277-338; rust:crates/ice-ai/src/types.rs:529-678; rust:crates/ice-ai/src/types.rs:811-939; rust:crates/ice-ai/src/model.rs:178-199).

(4) Assistant diagnostics is absent (AI-009); native Model has id/name/api/provider/base_url/reasoning/thinking map/input/cost/context_window/max_tokens/sampling_params/headers/compat and useful cost_configured, but no service_tiers. Native Tool.constrained_sampling is Option<ConstrainedSamplingConfig> and cannot directly represent the TS explicit false value. Native model compatibility uses the separate tagged StreamingProtocolCompat representation, with field/adapter details owned by AUTH-033. The native API enum retains PiMessages and Faux, and can still round-trip ice-messages through Other(String); this is not evidence that the ICE gateway executes (AI-029 owns that). Missing ultra stays AI-005. None of those separate behavior estimates is duplicated (rust:crates/ice-ai/src/types.rs:416-523; rust:crates/ice-ai/src/types.rs:654-679; rust:crates/ice-ai/src/types.rs:811-855; rust:crates/ice-ai/src/model.rs:145-199).

Native events use Arc<AssistantMessage> for partial snapshots and owned final assistants; their manual serializer already emits the exact start/text_*/thinking_*/toolcall_*/done/error keys, contentIndex, delta/content/toolCall, partial and final fields. types.rs supplies Serialize but no AssistantMessageEvent Deserialize implementation. Type declarations and enum construction alone neither enforce stream order nor check index-to-content matching; event-stream/provider implementations remain their existing owners (rust:crates/ice-ai/src/types.rs:878-1157).

The native `calculate_cost` preserves the same strict highest-threshold rule and ordinary cost arithmetic, returning a new UsageCost instead of mutating the input. It clamps long cache-write count to at least zero and short cache writes to at least zero, unlike TS for inconsistent/negative counters. Retain this explicit native defensive behavior unless a malformed-input compatibility requirement is chosen; the ordinary valid-input calculation is already implemented (rust:crates/ice-ai/src/providers/anthropic/cost.rs:17-48).

### 8. Port notes

Keep the existing native structs, explicit content markers, event serializer, usage helpers and extra native namespace/end_turn/cost_configured capabilities. Add a documented TS-compatible boundary representation or explicit projections rather than blindly serializing every native field. Match the three role/content domains, exact literal spellings and field names quoted here; preserve requested versus response model identity, signatures, optional reported zero, deferred JSON null and explicitly present optional arrays/booleans where the chosen interoperable representation distinguishes them. Define admission/conversion for native integer limits and in-process non-JSON details at the appropriate transport boundary; do not claim the TS interfaces themselves validate those values.

Correct the existing RUST-035 spelling through its owning fix, inspect and fix raw Message serialization to emit one role if the source-derived candidate is confirmed, add advertised model serviceTiers, and admit explicit constrainedSampling:false at the compatibility boundary without changing AI-013's negotiation semantics. Integrate AI-009's diagnostic field without duplicating its helper. Keep binary RPC's explicit lossy projection and closed validation, JSONL's command validators and agent custom-role encoding separately owned. Future targeted verification should assert raw and Value JSON field/tag shapes independently, TS fixtures decoding into native boundary DTOs, allowed content per role, optional null/empty/false/zero behavior, requested/response IDs, and terminal outer-versus-inner spelling. The current source-only audit did not write or execute those tests.

### 9. Size and priority

Estimated Rust implementation: **140-280 lines** (estimate, not measured source). **P1** - Agent and RPC consumers require an implementable shared data contract; direct native serialization can change tags or omit permitted metadata even though the core message types already exist.

## AI-021 - Anthropic request options, thinking modes and compatibility-controlled payloads

### 1. Title and status

Anthropic request options, thinking modes and compatibility-controlled payloads. **Status: partial.**

### 2. ice source

ice:packages/ai/src/api/anthropic-messages.ts:172-268; ice:packages/ai/src/api/anthropic-messages.ts:493-581; ice:packages/ai/src/api/anthropic-messages.ts:789-858; ice:packages/ai/src/api/anthropic-messages.ts:956-1090; ice:packages/ai/test/anthropic-force-adaptive-thinking.test.ts:70-123; ice:packages/ai/test/anthropic-thinking-disable.test.ts:112-171; ice:packages/ai/test/anthropic-temperature-compat.test.ts:67-103.

Exported symbols: `AnthropicEffort`, `AnthropicThinkingDisplay` (ice:packages/ai/src/api/anthropic-messages.ts:172; ice:packages/ai/src/api/anthropic-messages.ts:174).

### 3. What it does

The Anthropic adapter exposes both direct protocol options and a simple reasoning-level API, with explicit capability overrides for compatible endpoints. It preserves request customization, hard output authority, response hooks, thinking display and tool choice.

### 4. How it works

The direct stream accepts AnthropicOptions, while streamSimple authenticates synchronously and builds the shared base envelope before selecting adaptive or budget-based thinking. Request assembly precedes onPayload, the hard max is enforced again afterward, SDK retries are disabled, and the shared HTTP retry wrapper obtains the raw response before awaiting onResponse and publishing start.

Direct options add thinkingEnabled, thinkingBudgetTokens, effort including ultra, thinkingDisplay=summarized|omitted, interleavedThinking, toolChoice=auto|any|none|{type:tool, name}, and a prebuilt client. Injecting a client skips client construction/auth checks and fixes the OAuth transform flag to false. Default direct thinking is omitted; streamSimple without reasoning explicitly sets thinkingEnabled=false. forceAdaptiveThinking=true selects adaptive mode, with the model's thinkingLevelMap string winning over low/medium/high fallback. Otherwise it fits the configured/default thinking budget inside model/context/hard limits while reserving ordinary output. The expression at line 847 was connector-redacted and is not reconstructed. Direct budget zero is falsy and becomes 1024 in buildParams, a source edge case that must not be described as preserving an explicit zero budget (ice:packages/ai/src/api/anthropic-messages.ts:814-858; ice:packages/ai/src/api/anthropic-messages.ts:1044-1073).

Thinking payloads are emitted only for reasoning models. Display defaults to summarized; disabling is omitted when thinkingLevelMap.off is null. Temperature is sent only when provided, thinking is not enabled and supportsTemperature allows it. Only string metadata.user_id is copied. Tool choice is passed as its protocol object. Default capability values include eager input, long cache, tool cache and temperature=true; affinity, empty signatures and strict tools=false; tool references use model identity/version inference. The enabled beta headers, auth identity, history and cache-marker rules are detailed in AI-022/024. Tests were read without execution; the live Opus smoke assertion expects {type:adaptive} alone at test line 48, which conflicts with the current summarized display field and is not accepted as evidence of a passing suite.

### 5. Interfaces

AnthropicOptions extends StreamOptions; stream and streamSimple for api='anthropic-messages'; thinkingEnabled/budget/effort/display/interleavedThinking/client/toolChoice; compat flags listed above; metadata.user_id; POST Messages payload and onPayload/onResponse hooks.

### 6. Dependencies

`AI-003`, `AI-004`, `AI-005`, `AI-006`, `AI-008`, `AI-013`, `AI-014`

AI-003 dispatch; AI-004 transport retry; AI-005 reasoning support; AI-006 envelope/hard authority; AI-008 context estimates; AI-013 sampling constraints; AI-014 history projection. External TS dependency is @anthropic-ai/sdk; native uses reqwest and its SSE mapper.

### 7. ice-rust today

The native adapter has real request building, adaptive/budget thinking, temperature gating, user_id metadata, deferred tools, cache control and retry (rust:crates/ice-ai/src/providers/anthropic/build_params.rs:1256-1455; rust:crates/ice-ai/src/providers/anthropic/mod.rs:189-331). It folds the simple API into one options struct, so direct thinkingEnabled/budget/display, interleaved override, client injection and tool choice are unavailable. Display is fixed summarized; only explicit Some(Off) emits disabled, so absent reasoning differs from TS streamSimple. Effort mapping recognizes fixed strings and maps unknown strings, including ultra, to high (rust:crates/ice-ai/src/providers/anthropic/build_params.rs:1110-1133). Interleaved beta is gated by thinking_enabled, while TS defaults its option to true even with no reasoning on non-adaptive models. Header/body hooks and immutable hard output authority are absent in the public envelope. Native local model defaults are a curated subset, not the generated catalog (rust:crates/ice-ai/src/providers/anthropic/models.rs:1-239).

### 8. Port notes

Add a protocol-specific options carrier alongside the simple envelope, preserving the distinction between absent direct thinking and explicit simple off. Support requested display, tool choice and interleaved behavior, and carry hard authority through all transforms. Keep native transport/client injection idiomatic while exposing equivalent payload/response observation. Reconcile the source/test display mismatch and the zero-budget/floor edge case before declaring provider parity.

### 9. Size and priority

Estimated Rust implementation: **260-520 lines** (estimate, not measured source). **P1** - Common Anthropic and compatible-provider requests cannot express several supported controls and can produce different thinking payloads.

## AI-022 - Anthropic OAuth and Copilot wire identity with reversible tool naming

### 1. Title and status

Anthropic OAuth and Copilot wire identity with reversible tool naming. **Status: partial.**

### 2. ice source

ice:packages/ai/src/api/anthropic-messages.ts:81-118; ice:packages/ai/src/api/anthropic-messages.ts:519-554; ice:packages/ai/src/api/anthropic-messages.ts:860-953; ice:packages/ai/src/api/anthropic-messages.ts:992-1017; ice:packages/ai/test/anthropic-tool-name-normalization.test.ts:1-205; ice:packages/ai/src/api/github-copilot-headers.ts:1-37.

Exported symbols: `inferCopilotInitiator`, `hasCopilotVisionInput`, `buildCopilotDynamicHeaders` (ice:packages/ai/src/api/github-copilot-headers.ts:5; ice:packages/ai/src/api/github-copilot-headers.ts:11; ice:packages/ai/src/api/github-copilot-headers.ts:23).

### 3. What it does

Authenticated Anthropic requests use different wire identities for API keys, Claude OAuth and GitHub Copilot. OAuth tool names are normalized reversibly against active tools so returned calls still target the user's actual tool registry.

### 4. How it works

The adapter checks a provided key or nonempty authorization/x-api-key/cf-aig-authorization request header before constructing a client. Copilot takes precedence over token-pattern detection; its client uses Bearer auth, selective beta features and dynamic initiator/vision headers.

A key containing the source-defined OAuth marker selects the OAuth arm, with authToken, Claude CLI user-agent version 2.1.75, x-app=cli, Claude Code/OAuth beta headers and the caller's model/request header overlays. It prepends the Claude Code identity system block, followed by the caller system prompt when present. This fixed external identity is a protocol compatibility string, not the Rust product name. Outbound recognized names are matched case-insensitively against the explicit Claude tool-name table; unrelated names such as find remain unchanged. Inbound names are mapped to the first case-insensitive match in Context.tools, otherwise preserved. Deferred tools use canonical names before deduplication/prior-use checks. Provider client injection deliberately disables these OAuth transforms (ice:packages/ai/src/api/anthropic-messages.ts:81-118; ice:packages/ai/src/api/anthropic-messages.ts:523-554; ice:packages/ai/src/api/anthropic-messages.ts:907-929).

For normal key or header-owned auth, affinity is sent only when sessionId is present, caching is not none and compat.sendSessionAffinityHeaders is true. Headers combine defaults, affinity, model and request sources in that order. Copilot combines defaults, model, dynamic and request headers. Null header values can suppress defaults through the SDK carrier. This entry owns request identity after credential resolution; OAuth login, refresh, storage and account selection belong to AUTH. The credential-dependent naming tests are conditional source evidence, not proof of live service acceptance.

The named Copilot helpers classify an empty history or a last user message as user-initiated, and any other last role as agent-initiated. Vision detection examines only user/tool-result array content for image blocks. Dynamic headers always include `X-Initiator` and `Openai-Intent: conversation-edits`; `Copilot-Vision-Request: true` appears only when the supplied hasImages flag is true (ice:packages/ai/src/api/github-copilot-headers.ts:5-36).

### 5. Interfaces

createClient auth arms; isOAuthToken marker check; toClaudeCodeName/fromClaudeCodeName; Claude canonical names; beta/user-agent/x-app/x-session-affinity and Copilot dynamic headers; Context.tools for inbound name resolution.

### 6. Dependencies

`AI-006`, `AI-014`, `AUTH-009`, `AUTH-016`, `AUTH-018`

AUTH-009 supplies resolved request auth/headers, AUTH-016 Anthropic OAuth and AUTH-018 Copilot OAuth; AI-006 preserves header suppression/scoped controls and AI-014 history normalization. Login/storage is not duplicated here.

### 7. ice-rust today

Native Anthropic supports key and header-owned auth, model/request header merging and opt-in affinity, including the custom-provider environment isolation path (rust:crates/ice-ai/src/providers/anthropic/mod.rs:205-255; rust:crates/ice-ai/src/providers/anthropic/mod.rs:495-623). Production always calls build_params with is_oauth=false and sends any resolved key as x-api-key; merely having OAuth metadata elsewhere does not activate Bearer/tool-name identity here. The dormant true branch builds the Claude system prefix but has no production caller in this module (rust:crates/ice-ai/src/providers/anthropic/build_params.rs:1349-1374). Header auth accepts a matching key name even for an empty value, unlike TS's nonblank check; native header maps cannot carry null suppression. The default native user-agent is curl/8.0. OAuth/Copilot wire transforms and inbound canonical-name mapping are absent from this producer path.

### 8. Port notes

Connect credential mode to actual provider transport selection, build the OAuth/Copilot header and body identity explicitly, and normalize names reversibly without conflating different tools. Keep custom-provider credentials isolated and do not fall back to the official provider key for foreign endpoints. Preserve exact external protocol identity strings while using native product identity for user-facing paths/configuration.

### 9. Size and priority

Estimated Rust implementation: **230-450 lines** (estimate, not measured source). **P1** - Successful credential acquisition does not produce usable OAuth/Copilot requests without the matching wire and tool-name contract.

## AI-023 - Anthropic SSE terminal integrity, live usage and lossless delta delivery

### 1. Title and status

Anthropic SSE terminal integrity, live usage and lossless delta delivery. **Status: partial.**

### 2. ice source

ice:packages/ai/src/api/anthropic-messages.ts:301-491; ice:packages/ai/src/api/anthropic-messages.ts:581-787; ice:packages/ai/src/api/anthropic-messages.ts:1342-1367; ice:packages/ai/test/anthropic-sse-parsing.test.ts:1-425; ice:packages/ai/test/anthropic-cache-write-1h-cost.test.ts:1-87.

### 3. What it does

The adapter turns raw Anthropic SSE into ordered text, thinking and tool-call events with usage and a trustworthy terminal result. A stream that started a message but never received its completion marker must not be reported as complete merely because an earlier delta supplied a stop reason.

### 4. How it works

The TS decoder buffers UTF-8 text, accepts CR/LF line boundaries, joins multiple data fields with newlines and flushes a trailing frame at EOF. It ignores unrecognized SSE event names, rejects error events and parses recognized payloads with JSON repair before dispatching on payload.type.

After message_start it requires message_stop by EOF; the outer stream also rejects a still-pending reason and checks cancellation after iteration. It continues reading after message_stop, so later recognized error/malformed frames can still turn the result into an error. content_block_start preserves initial content; text/thinking deltas append and publish immediately, signature deltas append without a separate event, and tool JSON is reparsed cumulatively and finalized on block_stop. Scratch indices/partialJson are stripped before terminal persistence on success or failure. Initial usage and every usage delta trigger cost calculation; omitted/null usage fields preserve prior values. cacheWrite1h and reasoning counts remain subsets of their corresponding totals (ice:packages/ai/src/api/anthropic-messages.ts:452-491; ice:packages/ai/src/api/anthropic-messages.ts:586-781).

Raw stop reasons are retained. end_turn/pause_turn/stop_sequence map to stop, max_tokens to length, tool_use to toolUse, refusal to error with explanation/default, sensitive to a descriptive error and unknown values throw. Errors retain partial content and are normalized to aborted when the request signal is aborted. The decoder releases its reader lock but does not itself race reader.read against an abort promise; injected transports must honor the signal for prompt cancellation. Comment-only raw lines are not cleared by an empty no-event flush in TS, and split CRLF chunk handling is weaker than the native decoder; these are source limitations rather than native gaps.

### 5. Interfaces

Raw SSE event/data frames; AssistantMessageEvent start/text_*/thinking_*/toolcall_*/done/error; responseId, rawStopReason, usage/cost/reasoning/cacheWrite1h; pending/stop/length/toolUse/error/aborted reasons; iterateSseMessages/iterateAnthropicEvents.

### 6. Dependencies

`AI-004`, `AI-011`, `AI-021`

AI-004 owns pre-stream HTTP retries; AI-011 JSON repair; AI-021 assembles the request and lifecycle hooks. The stream must not repeat already emitted content via an HTTP retry.

### 7. ice-rust today

Rust has a real pull decoder, incremental UTF-8 repair, cancellation raced against body reads, SSE diagnostics and a comprehensive mapper (rust:crates/ice-ai/src/providers/anthropic/sse.rs:1-430; rust:crates/ice-ai/src/providers/anthropic/mapper.rs:170-861). The production run_mapper calls next_event/parse/apply directly and finalizes at EOF. Its start-without-stop validation exists only in the collecting iterate_anthropic_events helper, which production does not use. If message_delta has already set a nonpending reason, EOF reaches successful finalize without requiring message_stop (rust:crates/ice-ai/src/providers/anthropic/mapper.rs:718-827): a source-confirmed terminal-integrity defect. Native deliberately finalizes immediately at message_stop and allows that marker without a stop reason, unlike TS. It batches deltas using byte/elapsed checks on later arrivals; no timer wakes an otherwise stalled stream. Successful finalize does not flush pending batches, so a proxy terminal without content_block_stop can omit the tail from delta events while retaining it in the final message. Cost is calculated only on successful finalization; partial/error results can carry zero cost despite recorded usage. Native tests exercise mapper fixtures and cancellation but do not establish these missing production invariants.

### 8. Port notes

Move completion-marker validation into the production state machine and distinguish deliberate marker-based compatibility from truncated EOF. Flush pending deltas before every terminal result, add a real time-based flush if promised by the batching interval, and calculate usage cost for partial/error outcomes. Keep native UTF-8/CRLF/comment-state fixes and stalled-read cancellation. Define whether post-marker events are ignored as an intentional divergence, with source-driven future regression cases through the actual producer path.

### 9. Size and priority

Estimated Rust implementation: **220-440 lines** (estimate, not measured source). **P1** - Truncated responses can be accepted as complete, and delta consumers or cost accounting can miss the final partial state.

## AI-024 - Anthropic history encoding, deferred tool placement and cache marker fidelity

### 1. Title and status

Anthropic history encoding, deferred tool placement and cache marker fidelity. **Status: partial.**

### 2. ice source

ice:packages/ai/src/utils/deferred-tools.ts:1-39; ice:packages/ai/src/api/anthropic-messages.ts:55-78; ice:packages/ai/src/api/anthropic-messages.ts:179-206; ice:packages/ai/src/api/anthropic-messages.ts:956-1042; ice:packages/ai/src/api/anthropic-messages.ts:1098-1339; ice:packages/ai/test/deferred-tools.test.ts:179-335; ice:packages/ai/test/anthropic-eager-tool-input-compat.test.ts:120-160; ice:packages/ai/test/anthropic-empty-thinking-signature-compat.test.ts:81-108.

### 3. What it does

Anthropic request history preserves text/image ordering and batches tool results correctly while loading later tool definitions at their historical marker. Caching depends on stable tool order and correctly placed markers across system, tool and final-user content.

### 4. How it works

The adapter projects messages, deduplicates active tools by normalized name using Map insertion order with last value winning, and separates immediate tools from tools first marked available before use. Unsupported tool-reference models use the normal active list; if all active tools would be deferred, all are promoted to immediate for that request.

Default tool-reference support requires first-party Anthropic, excludes Haiku, and recognizes Opus/Sonnet/Fable versions newer than 4 or at least 4.5. Explicit compatibility overrides win. Only active definitions can be deferred; a name used before its marker stays immediate. Deferred definitions carry defer_loading=true and no cache marker. A result marker emits each reference once and moves ordinary result text/images into sibling user content after every consecutive tool_result block. Text-only results join with newlines; image-only results receive '(see attached image)' before images. Empty user/assistant text is filtered. Same-model signed thinking is replayed, missing signatures become text unless allowEmptySignature allows an empty signature, and redacted payloads remain opaque (ice:packages/ai/src/api/anthropic-messages.ts:1098-1297).

Cache retention defaults short, with ICE_CACHE_RETENTION=long fallback when no explicit option; none omits markers and long adds ttl=1h only when supported. Markers attach to system blocks, the last immediate tool when supportsCacheControlOnTools, and the final block of the last message only if that message is user. Eligible final blocks are text, image or tool_result. Eager input is per-tool by default; disabling it with any tools enables the fine-grained beta. Non-strict tool input schema is rebuilt as type/object plus properties/required only; strict requests preserve the remaining schema fields and set strict=true. This can omit referenced definitions in the TS non-strict path, a source limitation that native has deliberately addressed.

### 5. Interfaces

splitDeferredTools(Context, enabled, normalizeName?)→immediate/deferred; ToolResultMessage.addedToolNames; defer_loading/tool_reference/eager_input_streaming/strict/input_schema; cache_control {type:ephemeral, ttl?:1h}; compat.supportsToolReferences/CacheControlOnTools/EagerToolInputStreaming/LongCacheRetention/allowEmptySignature.

### 6. Dependencies

`AI-013`, `AI-014`, `AI-021`, `AI-022`

AI-013 strict constraint negotiation; AI-014 target-model projection; AI-021 request configuration; AI-022 normalized OAuth names. Definitions must be the current tool registry, not arbitrary names from untrusted result text.

### 7. ice-rust today

Native supports deferred classification, promotion, reference displacement, mixed images, empty-signature compatibility and cache control (rust:crates/ice-ai/src/providers/anthropic/build_params.rs:480-1039; rust:crates/ice-ai/src/providers/anthropic/build_params.rs:1262-1290). Its BTreeMap sorts deduplicated tools instead of retaining TS insertion order, changing which tool receives the last-tool cache marker. tool_references defaults true without inspecting target identity/version (rust:crates/ice-ai/src/model.rs:59-64). ToolResult wire variants have no cache_control field, and set_cache_control_on_block handles text/image/tool-use only, so the final tool-result cache marker is lost (rust:crates/ice-ai/src/providers/anthropic/build_params.rs:1008-1039). The native image-only result deliberately omits the TS placeholder and has explicit tests for that behavior (rust:crates/ice-ai/tests/anthropic_tool_images.rs:185-214). It retains definitions/$defs in non-strict schemas to fix unresolved references, and strict conversion uses its own transformation/fallback policy (rust:crates/ice-ai/src/providers/anthropic/build_params.rs:609-647). Cache TTL support exists, but the helper does not read TS's environment fallback (rust:crates/ice-ai/src/providers/anthropic/cache_control.rs:90-140).

### 8. Port notes

Preserve stable insertion order for active/deferred tools, compute default support from the model identity or fully materialize equivalent metadata, and serialize cache_control on tool_result blocks. Keep native image-only payload and reference-definition fixes as explicit design decisions instead of regressing them for byte-for-byte similarity. Align strict failure policy with AI-013 and route environment-derived retention through request configuration. Future checks should compare request payloads, not just helper return shapes.

### 9. Size and priority

Estimated Rust implementation: **240-460 lines** (estimate, not measured source). **P1** - Tool availability and cache-boundary differences affect provider acceptance, deferred loading and repeat-request behavior.

## AI-025 - Chat Completions request compatibility, thinking formats and tool controls

### 1. Title and status

Chat Completions request compatibility, thinking formats and tool controls. **Status: partial.**

### 2. ice source

ice:packages/ai/src/api/openai-completions.ts:141-266 (OpenAICompletionsOptions and stream request lifecycle), ice:packages/ai/src/api/openai-completions.ts:629-934 (streamSimple, createClient, buildParams and template variables), ice:packages/ai/src/api/openai-completions.ts:1343-1378 (convertTools), ice:packages/ai/src/api/openai-completions.ts:1449-1580 (detectCompat/getCompat); ice:packages/ai/test/openai-completions-tool-choice.test.ts:106-507 and ice:packages/ai/test/openai-completions-tool-choice.test.ts:1603-1796 (request assertions).

Exported symbols: `ConvertCompletionsMessagesOptions` (ice:packages/ai/src/api/openai-completions.ts:146).

### 3. What it does

One Chat Completions adapter supports multiple provider dialects through detected defaults plus explicit per-model overrides. The missing Rust behavior is the actual request projection, beyond merely accepting a compatible base URL or model record.

### 4. How it works

The adapter resolves a concrete compatibility record before converting history, tools and request options. Explicit non-null model compatibility values override provider/URL detection, and the final request passes through awaited hooks and the shared HTTP retry boundary.

Detection classifies Z.AI, Together, Moonshot, OpenRouter, Cloudflare Workers/Gateway, NVIDIA and Ant Ling plus nonstandard endpoint patterns. It independently selects store=false support, developer versus system roles, reasoning-effort support, max_tokens versus max_completion_tokens, streaming usage, finish-reason policy, strict/grammar tool support and cache/affinity behavior. For example, OpenRouter Anthropic/OpenAI reasoning models retain developer instructions; other OpenRouter models default to system. DeepSeek defaults require reasoning_content and its own thinking object. This is capability-by-capability detection, not one generic 'OpenAI compatible' switch (ice:packages/ai/src/api/openai-completions.ts:1449-1580).

buildParams sends streaming model/messages, optional include_usage and store=false; max-output handling uses AI-006, temperature is included when defined, and tools are omitted when there are neither active tools nor historical tool calls/results. Historical tool activity instead forces tools:[] for proxy compatibility. Z.AI tool_stream is emitted only with active tools and enabled compatibility. toolChoice is forwarded directly and, unusually among these adapters, explicitly carried from the simple options cast. Grammar tools become custom grammar definitions; function tools send the original schema and strict=false by default when supported, with constrained strict resolution through AI-013. The empty-tools and token-budget cases are asserted in ice:packages/ai/test/openai-completions-empty-tools.test.ts:56-303.

Thinking request formats include OpenAI reasoning_effort; Z.AI thinking enabled/disabled plus clear_thinking=false; Qwen enable_thinking; Qwen chat_template_kwargs with preserve_thinking; configurable chat-template kwargs; Baseten chat_template_args; DeepSeek thinking objects respecting thinkingCanDisable and off:null; OpenRouter nested reasoning.effort; Ant Ling mapped nested effort only; Together reasoning.enabled; and string-valued thinking. Mapped effort strings and off:null control omission. Template values may be static primitives/null or variables thinking.enabled/thinking.effort, with omitWhenOff; unsupported mapped value types are omitted. OpenRouter provider routing and Vercel providerOptions.gateway only/order are added independently. samplingParams is assigned last over named fields, followed by the hard limit enforcement; onPayload may then replace the object and is followed by another hard limit check (ice:packages/ai/src/api/openai-completions.ts:704-934).

An explicit key is accepted, otherwise a nonblank authorization or cf-aig-authorization header supplies the SDK's placeholder credential. Copilot dynamic headers precede caller overrides. The SDK receives model.baseUrl and custom fetch; request timeout/signal are explicit, SDK maxRetries=0, retryProviderRequest owns retries, onResponse is awaited before start. Request conversion/hook/auth failures become terminal assistant errors; streamSimple's preliminary key check can throw synchronously. No credential file is read by this wire adapter.

### 5. Interfaces

OpenAICompletionsOptions extends StreamOptions with toolChoice and reasoningEffort minimal|low|medium|high|xhigh|max|ultra. stream(model, context, options), streamSimple(model, context, options), convertMessages(model, context, resolvedCompat,{grammarToolInputProperties?}). Compatibility includes supportsStore, supportsDeveloperRole, supportsReasoningEffort, supportsUsageInStreaming, supportsFinishReason, maxTokensField, requiresToolResultName, requiresAssistantAfterToolResult, requiresThinkingAsText, requiresReasoningContentOnAssistantMessages, thinkingFormat, chatTemplateKwargs/chatTemplateArgs, openRouterRouting/vercelGatewayRouting, zaiToolStream, supportsStrictMode and supportsOpenAIGrammarTools; cache-related fields are AI-050.

### 6. Dependencies

`AI-002`, `AI-003`, `AI-004`, `AI-005`, `AI-006`, `AI-010`, `AI-013`

AI-002 supplies resolved model metadata; AI-003 dispatches; AI-004 owns HTTP retries; AI-005/006 supply thinking/output limits; AI-010 normalizes errors; AI-013 governs constrained tools. AUTH-033 owns configuration ingestion of compatibility fields. External boundary: OpenAI SDK in TS and reqwest/serde_json in Rust; do not copy SDK implementation.

### 7. ice-rust today

OpenAiCompletionsProvider is live, not missing: rust:crates/ice-ai/src/providers/openai_completions.rs:31-259 constructs providers, sends authenticated requests with timeout/cancellation/retries and streams SSE. build_request at rust:crates/ice-ai/src/providers/openai_completions.rs:261-453 already handles basic instructions, tool images/placeholders, optional usage, max-token fields, temperature, ordinary function tools, store and model sampling_params. Its helpers at rust:crates/ice-ai/src/providers/openai_completions.rs:528-605 default developer/store support false and max-token field from model.reasoning; there is no TS provider/URL detection table. Request building has only generic reasoning_effort, no above dialect projection, tool_choice, Kimi deferred placement, grammar/custom tool definitions or strict resolution in this adapter. Empty tools are always omitted, including historical-tool proxy cases. Request-level sampling/hooks/fetch have no equivalent in this builder. The native endpoint helper appends /v1 for host roots and recognizes version segments or a complete /chat/completions URL (rust:crates/ice-ai/src/providers/openai_completions.rs:498-526); TS passes baseURL to its SDK, with a host-root /chat/completions assertion in ice:packages/ai/test/openai-completions-thinking-as-text.test.ts:146-221, so native URL convenience is a different contract. Native explicit anonymous/custom providers and exact-openai-only environment inheritance are useful isolation choices. Native apply_api_key overwrites any existing case-insensitive Authorization when a key is supplied, and auth-header recognition checks key presence (authorization/x-api-key), unlike TS nonblank authorization/cf-aig checks (rust:crates/ice-ai/src/providers/openai_completions.rs:607-642).

### 8. Port notes

Introduce a typed resolved compatibility layer shared by model ingestion and this request builder. Port each thinking format, dynamic template variable and routing field with explicit omission semantics; carry request-level tool choice, constrained tools and sampling overrides, preserving the hard ceiling after all mutations. Keep the native anonymous-provider and credential-isolation constructors while specifying header precedence and gateway-auth support. Keep endpoint normalization separately configurable so existing native custom URLs continue to work. Future verification should translate the existing empty-tool/history, reasoning dialect, strict/custom tool, header and routing fixtures into native request-shape checks; no tests were created or run for this audit.

### 9. Size and priority

Estimated Rust implementation: **650-1100 lines** (estimate, not measured source). **P1** - Many declared provider/model combinations can reach an endpoint but send incompatible reasoning, tool or instruction fields.

## AI-026 - Responses request controls, cache affinity, service-tier cost and stale-continuation recovery

### 1. Title and status

Responses request controls, cache affinity, service-tier cost and stale-continuation recovery. **Status: partial.**

### 2. ice source

ice:packages/ai/src/api/openai-responses.ts:45-111 (defaults/options), ice:packages/ai/src/api/openai-responses.ts:143-330 (request lifecycle and stale retry), ice:packages/ai/src/api/openai-responses.ts:337-519 (simple wrapper, client, request and pricing); ice:packages/ai/test/openai-responses-compat.test.ts:69-654.

### 3. What it does

The Responses adapter combines stateless transcript replay with explicit cache, reasoning, service-tier, tool and extension hooks. Native Responses streaming exists, but its request controls and continuation failure handling cover a smaller contract.

### 4. How it works

The default request includes the complete locally converted input, stream=true and store=false. A previous_response_id is not generated by core request building; it can be introduced by sampling parameters or the payload hook and is treated as optional external continuation state.

Compatibility defaults developer-role and long-cache support true, strict/grammar/tool-search/explicit-cache-mode support false, with affinity format detected as OpenRouter or OpenAI. Retention is explicit option, otherwise ICE_CACHE_RETENTION=long, otherwise short. With non-none retention, promptCacheKey or sessionId is clipped to 64 Unicode code points for prompt_cache_key; long supported retention adds '24h'. With retention none and supportsExplicitPromptCacheMode, prompt_cache_options={mode:'explicit'} disables implicit caching. Generated affinity uses x-session-id for OpenRouter; session_id plus x-client-request-id for OpenAI; only x-client-request-id for openai-nosession. Caller headers override generated values, and none suppresses generated identities, not explicit caller headers (ice:packages/ai/src/api/openai-responses.ts:64-99 and ice:packages/ai/src/api/openai-responses.ts:354-430).

The request supports temperature, serviceTier, toolChoice, immediate versus deferred tools, strict/custom grammar definitions and reasoning summary. For reasoning models, requested effort/summary sends mapped effort (medium if only a summary is requested), summary default auto and include=['reasoning.encrypted_content']. With neither, non-Copilot models use mapped off or 'none' unless off:null; xAI always requests encrypted content. The simple wrapper clamps reasoning but does not separately forward the direct-only serviceTier/reasoningSummary/toolChoice options. Output limits use the shared hard ceiling plus a 16-token Responses floor; an incompatible hard ceiling errors rather than silently exceeding it. samplingParams overrides named fields before the hard-limit recheck; onPayload may replace the request and is followed by another floor/ceiling check (ice:packages/ai/src/api/openai-responses.ts:398-489).

SDK retries are disabled; the shared request retry helper owns HTTP attempts. onResponse is awaited before start. One additional stale-continuation recovery is allowed only before start, before cancellation and before a prior stale recovery: recognizable stale previous-response errors cause the stored diagnostic response ID to be forgotten and previous_response_id to be deleted from the final payload, then the same local input is sent again. This path does not reconstruct input or rerun onPayload. If an extension had replaced full input with a delta, deleting the ID alone cannot recreate omitted history; that limitation should remain explicit. After a successful terminal reason, response-ID diagnostics may be remembered under AI-018.

Service-tier pricing uses the terminal response tier when present, otherwise the requested tier: flex multiplies all cost components by 0.5; priority uses 2.5 for exact model id gpt-5.5 and 2 for others; other tiers use 1. These are frozen source policies, not a claim about current public prices. Tests asserting request shape sometimes use a bare [DONE] fixture and intentionally accept either terminal event; they are not evidence that bare [DONE] is valid Responses completion.

### 5. Interfaces

OpenAIResponsesOptions extends StreamOptions: reasoningEffort minimal|low|medium|high|xhigh|max|ultra, reasoningSummary auto|detailed|concise|null, serviceTier, toolChoice. stream/streamSimple; compatibility supportsDeveloperRole, sessionAffinityFormat, supportsLongCacheRetention, supportsStrictMode, supportsOpenAIGrammarTools, supportsToolSearch, supportsExplicitPromptCacheMode. Wire cache fields, reasoning/include, service_tier, tool_choice, previous_response_id; awaited onPayload/onResponse.

### 6. Dependencies

`AI-002`, `AI-003`, `AI-004`, `AI-005`, `AI-006`, `AI-013`, `AI-016`, `AI-018`

Model/dispatch/retry/thinking/limit/constrained/env/diagnostic contracts are supplied by the listed AI entries. AUTH-033 owns config ingestion; AI-054 owns history/tool-search projection. External boundary: OpenAI SDK or equivalent HTTP client.

### 7. ice-rust today

rust:crates/ice-ai/src/providers/openai_responses.rs:1152-1205 is a working authenticated HTTP/SSE path with cancellation, timeout and shared retries. Native build_request at rust:crates/ice-ai/src/providers/openai_responses.rs:1545-1798 already uses full local history, store=false, a 16-token minimum, mapped reasoning effort, strict-schema transformation and grammar tools. It always emits context.system_prompt as instructions, without consulting supports_developer_role. It does not project opts.temperature, cache_retention/session_id, prompt-cache identity/retention, service tier, reasoning summary/encrypted include, tool choice or deferred-tool search. model.sampling_params may inject static fields but lacks the per-call policy/hook contract. There is no one-time stale-continuation recovery; injected previous_response_id therefore receives ordinary HTTP failure handling. Cost uses calculate_cost without a service-tier multiplier (rust:crates/ice-ai/src/providers/openai_responses.rs:2303-2351). Native endpoint assembly only recognizes a /v1 suffix and otherwise appends /v1/responses (rust:crates/ice-ai/src/providers/openai_responses.rs:2197-2204); Azure/custom full endpoint support is not supplied by this helper. Exact-openai-only credential inheritance and custom-provider isolation are already tested and should be retained.

### 8. Port notes

Extend the existing provider request structure rather than create another generic Responses implementation. Add typed per-call cache/affinity, reasoning-summary, service-tier and tool-choice controls, then integrate the shared hook/hard-limit pipeline. Add explicit stale-continuation recovery that preserves full input; reject or deliberately reconstruct hook-supplied delta-only input before retrying without its ID. Keep source-frozen pricing policy configurable and auditable. Verify request ordering, all cache formats, off:null, explicit-cache-mode, hard limit versus floor and pre-start-only stale retry using future native fixtures.

### 9. Size and priority

Estimated Rust implementation: **380-620 lines** (estimate, not measured source). **P1** - The existing transport can send requests, but missing controls change reasoning, cache behavior, billing estimates and recovery from stale continuation state.

## AI-027 - Codex account-bound Responses request and cache-affinity dialect

### 1. Title and status

Codex account-bound Responses request and cache-affinity dialect. **Status: missing.**

### 2. ice source

ice:packages/ai/src/api/openai-codex-responses.ts:71-115 (options), ice:packages/ai/src/api/openai-codex-responses.ts:249-323 (request setup), ice:packages/ai/src/api/openai-codex-responses.ts:546-670 (payload, tier and URL), ice:packages/ai/src/api/openai-codex-responses.ts:1601-1677 (account and headers); ice:packages/ai/test/openai-codex-stream.test.ts:492-745, ice:packages/ai/test/openai-codex-stream.test.ts:808-1130.; ice:packages/ai/src/api/openai-prompt-cache.ts:1-9; ice:packages/ai/src/utils/uuid.ts:1-49

Exported symbols: `OPENAI_PROMPT_CACHE_KEY_MAX_LENGTH` (ice:packages/ai/src/api/openai-prompt-cache.ts:1).

### 3. What it does

Expose the Codex backend's account-bound Responses API as its own provider, preserving its request shape, headers and separate session/cache identities.

### 4. How it works

TS extracts chatgpt_account_id from the token's https://api.openai.com/auth payload claim and sends an account-bound Codex request. This extraction supplies wire metadata; it is not local JWT signature verification or an authorization decision.

The base URL defaults to https://chatgpt.com/backend-api and is normalized to /codex/responses, respecting an already complete /codex or /codex/responses suffix. The payload always carries instructions (system prompt or a default helpful-assistant sentence), store:false, stream:true, parallel_tool_calls:true, text verbosity low by default and encrypted reasoning in include. User/system history is converted with includeSystemPrompt:false because instructions is separate. tool_choice defaults auto; temperature and service_tier are optional. Function tools use strict:null when unconstrained and resolved strict:true when requested; model compatibility can enable grammar tools, but deferred tool-search support is disabled in this builder. Reasoning is added only when explicitly requested, with summary auto by default. Mapping uses nullish fallback, so an off:null map does not suppress the field here: 'none' remains 'none'. The body does not merge model.samplingParams. onPayload may replace it, after which immutable hard output authority is reapplied without the ordinary Responses 16-token floor.

Explicit cacheRetention:none removes generated session and prompt-cache identity; otherwise the wire session is sessionId clipped to 64 Unicode code points (Array.from in clampOpenAIPromptCacheKey) and prompt_cache_key independently uses promptCacheKey ?? sessionId clipped by that same code-point helper. The socket pool uses the unclipped session ID, never the prompt cache key (AI-055). There is no legacy cache-retention environment lookup in this adapter. Model and caller headers are merged first, including caller null deletion; Authorization, chatgpt-account-id, originator and user-agent are then set by the adapter. SSE uses OpenAI-Beta responses=experimental, accept:text/event-stream and session-id/x-client-request-id when a session exists. WebSocket headers use a request ID even without a cached session. These hyphenated names differ from the generic Responses affinity dialect.

Cost accounting uses the source's flex 0.5 and priority 2 multipliers, with priority 2.5 for the gpt-5.5 prefix. If the backend echoes service_tier:'default' after a client requested flex/priority, the requested tier still determines the source's cost calculation. These are frozen source rules, not independently verified current provider prices.

The fallback request ID comes from the shared uuidv7 helper. It embeds a nondecreasing millisecond timestamp and a sequence incremented when the clock does not advance, with crypto.getRandomValues when available and a Math.random fallback otherwise. That ID is correlation metadata, not an authentication secret; a native port can keep its existing stronger random source while making time-ordering and wire-ID expectations explicit.

### 5. Interfaces

OpenAICodexResponsesOptions reasoningEffort/reasoningSummary/serviceTier/textVerbosity/toolChoice plus common options; API 'openai-codex-responses'; account claim https://api.openai.com/auth.chatgpt_account_id; session-id, x-client-request-id, chatgpt-account-id; prompt_cache_key; /codex/responses.

### 6. Dependencies

`AI-005`, `AI-006`, `AI-013`, `AI-054`, `AUTH-017`

AUTH-017 owns OAuth credentials; this entry owns the backend payload and account header. AI-055 and AI-056 are optional transport implementations built on this request dialect, not prerequisites in the opposite direction.

### 7. ice-rust today

Native has a substantial generic Responses request encoder at rust:crates/ice-ai/src/providers/openai_responses.rs:1484-1798 and replay/grammar/schema helpers; these should be reused where compatible. Its endpoint/header construction and common options do not supply this account claim, Codex URL, verbosity, tier, header precedence or Codex-specific strict defaults. The fully read production registry at rust:crates/ice-cli/src/provider/factories.rs:16-145 registers Anthropic, OpenAI Completions and OpenAI Responses protocols and can consult a matching external provider extension; it has no built-in Codex protocol factory. Native module exports at rust:crates/ice-ai/src/providers/mod.rs:1-47 confirm the same boundary. An OAuth entry or user-configured base URL is not this complete wire adapter; a matching installed external package was not assumed.

### 8. Port notes

Add a dedicated Codex request/configuration layer around the native Responses history and event machinery. Keep session identity, prompt affinity and account identity distinct. Add explicit direct options and immutable output authority; preserve provider protocol header names and make native product attribution a conscious compatibility choice. Review off:null and absent samplingParams as source behavior rather than silently claiming uniform provider semantics.

### 9. Size and priority

Estimated Rust implementation: **350-580 lines** (estimate, not measured source). **P1** - A Codex credential and model listing cannot provide working Codex requests without the required backend payload and account metadata.

## AI-028 - Azure Responses endpoint, deployment and API-version resolution

### 1. Title and status

Azure Responses endpoint, deployment and API-version resolution. **Status: missing.**

### 2. ice source

ice:packages/ai/src/api/azure-openai-responses.ts:26-102 (options/deployment), ice:packages/ai/src/api/azure-openai-responses.ts:104-215 (stream), ice:packages/ai/src/api/azure-openai-responses.ts:217-359 (client and payload); ice:packages/ai/test/azure-openai-base-url.test.ts:87-205; ice:packages/ai/test/azure-openai-responses-reasoning-replay.test.ts:1-137; ice:packages/ai/test/azure-utils.ts:1-29.

### 3. What it does

Resolve Azure service endpoints, deployment names and API versions into a dedicated Responses client while retaining neutral model identity and reasoning replay.

### 4. How it works

TS resolves the deployment separately from the logical model and uses an AzureOpenAI SDK client. The outgoing model field is the deployment name, while the returned assistant message retains the original model ID and provider identity.

Deployment precedence is explicit truthy azureDeploymentName, then the AZURE_OPENAI_DEPLOYMENT_NAME_MAP entry for model.id, then model.id. The comma-separated model=deployment map trims pieces, skips missing sides and lets later duplicate keys replace earlier ones. API version is explicit option, environment or v1. Endpoint precedence is trimmed explicit azureBaseUrl, trimmed AZURE_OPENAI_BASE_URL, resource name from option/environment, then model.baseUrl; missing/invalid URL becomes a stream error. For hosts ending .openai.azure.com, .cognitiveservices.azure.com or .ai.azure.com, root, /openai and /openai/v1/responses normalize to /openai/v1 and discard query parameters. Other paths and proxy hosts retain their path/query. The SDK receives baseURL and apiVersion and owns the final request URL details; no guessed SDK behavior is treated as audited local source.

An explicit API key is required. Model headers are merged with caller headers, and an injected fetch is supported. The full-history payload uses Responses conversion including the system prompt, store:false and prompt_cache_key from explicit key/session unless retention is none, clipped to 64 Unicode code points (Array.from in clampOpenAIPromptCacheKey). Unlike generic OpenAI, no long-cache environment fallback, 24h retention or session-affinity header policy is added. Direct temperature/toolChoice/reasoning summary are supported; strict tool mode defaults true where supported and grammar support defaults false. All context tools are passed to conversion without a deferred-tool split. Sampling parameters merge after initial fields; hard output authority is then enforced, including a 16-token minimum feasibility check and rechecking after onPayload replacement.

The SDK's retries are disabled and shared retryProviderRequest controls the HTTP setup; onResponse runs before start. Shared Responses parsing preserves or backfills encrypted reasoning, requires terminal completion and checks cancellation afterward. The replay fixture distinguishes an existing done-item signature from a missing one, preserving the former and filling only the latter.

### 5. Interfaces

AzureOpenAIResponsesOptions azureApiVersion, azureResourceName, azureBaseUrl, azureDeploymentName, toolChoice, reasoningEffort, reasoningSummary; AZURE_OPENAI_API_VERSION/BASE_URL/RESOURCE_NAME/DEPLOYMENT_NAME_MAP; API 'azure-openai-responses'; AzureOpenAI({apiKey, baseURL, apiVersion, defaultHeaders, fetch}).

### 6. Dependencies

`AI-004`, `AI-006`, `AI-053`, `AI-054`, `AUTH-011`

AUTH-011 owns registered provider/catalog visibility; this entry owns Azure-specific request resolution. Shared Responses history and output-limit policy should be reused.

### 7. ice-rust today

Native provides configurable generic Responses HTTP requests and history at rust:crates/ice-ai/src/providers/openai_responses.rs:1153-1219 and rust:crates/ice-ai/src/providers/openai_responses.rs:1484-1798. Those are useful for a manually configured compatible endpoint, but do not implement Azure API-version/deployment-map/resource normalization or a built-in azure-openai-responses factory. The fully read protocol registry at rust:crates/ice-cli/src/provider/factories.rs:16-145 has three built-in HTTP protocols plus the existing matching extension seam. Do not discard native replay repair or ordered tool-result images when adding the Azure dialect.

### 8. Port notes

Add a typed Azure configuration and endpoint/deployment resolver before invoking shared native Responses encoding. Preserve logical model identity, make request API version explicit and test URL composition against source fixtures without assuming every Azure hostname/path is equivalent. Implement the stricter output authority and reuse shared retry/header propagation; keep native replay improvements.

### 9. Size and priority

Estimated Rust implementation: **220-380 lines** (estimate, not measured source). **P1** - Azure users require deployment and API-version routing that a generic provider name or model catalog cannot supply.

## AI-029 - ICE messages gateway protocol, rewrite diagnostics and neutral event reconstruction

### 1. Title and status

ICE messages gateway protocol, rewrite diagnostics and neutral event reconstruction. **Status: missing.**

### 2. ice source

ice:packages/ai/src/api/ice-messages.ts:1-175 (wire/error/rewrite types), ice:packages/ai/src/api/ice-messages.ts:177-344 (conversion/framing/errors), ice:packages/ai/src/api/ice-messages.ts:346-451 (request and simple wrapper); ice:packages/ai/test/ice-messages.test.ts:96-274.

Exported symbols: `IceMessagesEvent`, `IceMessagesResponseError` (ice:packages/ai/src/api/ice-messages.ts:53; ice:packages/ai/src/api/ice-messages.ts:95).

### 3. What it does

Call the ICE messages/Radius gateway with the neutral conversation format and reconstruct its serialized assistant events, usage and rewrite metadata.

### 4. How it works

TS posts {model, context, options} to the model base URL plus /messages and optionally adds debug=1. The gateway receives neutral history and tool definitions directly, rather than an Anthropic or OpenAI payload.

The request requires an explicit API key and uses Bearer authorization, SSE accept and JSON content type. Caller headers are spread last and can override those defaults; model.headers are not separately merged here. Options include temperature, clamped maxTokens, hardMaxOutputTokens, reasoning, cacheRetention, sessionId and toolChoice. Retention is explicit when truthy, otherwise only ICE_CACHE_RETENTION='long' is forwarded; unset leaves backend defaults in control. onPayload can replace the body. Under hard authority the replacement and nested options must remain non-array objects, and maxTokens is re-enforced; the transport redacts the adjacent assignment expression at line 396, so that expression is not claimed as inspected. The complete hard-limit fixture independently asserts the forwarded authority. Injected/global fetch receives caller signal, and onResponse runs before HTTP status handling. No adapter-local retry, timeout or compression layer appears.

SSE parsing streams UTF-8, normalizes CRLF and flushes a trailing unterminated frame. It accepts the first data: line in each event, ignores [DONE] and directly JSON-parses/casts the neutral event union; it does not implement a multiline SSE-data join or runtime schema validation. Text/thinking/tool blocks are addressed by contentIndex, text/signatures are finalized by end events, and tool deltas use a per-index incremental JSON buffer. The converter expects well-ordered start/delta/end events; malformed indexes can throw. Done/error terminal events overwrite usage and response ID from the server and append ice_messages_rewrite diagnostics carrying policy ID/version and count-change flags. Server usage and cost are trusted instead of recomputed from the model catalog.

The first terminal event ends consumption; EOF without one is an error. A transport/protocol exception creates a fresh empty assistant error, losing accumulated content/usage from the converter, while a server-sent error retains that partial state. HTTP object-shaped error bodies contribute code/message and structured diagnostics; non-object bodies are truncated to 8192 characters for diagnostics. Finally releases the reader lock but does not explicitly cancel the underlying body, including early terminal return. These are source limitations to address during a port.

### 5. Interfaces

API 'ice-messages'; IceMessagesOptions reasoning/toolChoice/debug; neutral SSE start/text_*/thinking_*/toolcall_*/done/error; IceMessagesRewriteImpact {policyId, policyVersion, changed, tokenCountChange, messageCountChange, systemPromptChanged}; ice_messages_rewrite and ice_messages_response_failure diagnostics.

### 6. Dependencies

`AI-003`, `AI-006`, `AI-009`, `AI-011`, `AUTH-022`

AUTH-022 owns Radius credentials/provider construction; the neutral gateway wire adapter is independent of that auth flow. The literal ice-messages API name and gateway diagnostic identifiers are protocol strings, not automatic rename targets.

### 7. ice-rust today

Native neutral AssistantMessage/Event types and streaming channels exist at rust:crates/ice-ai/src/types.rs:654-701 and rust:crates/ice-ai/src/event_stream.rs:18-172. No built-in neutral /messages gateway encoder/decoder exists in the fully read rust:crates/ice-ai/src/providers/mod.rs:1-47 or rust:crates/ice-cli/src/provider/factories.rs:16-145 registry; the similarly named Anthropic /messages endpoint uses a different body and event vocabulary. A matching external provider extension remains possible through the existing native seam, but was not inferred from its existence.

### 8. Port notes

Add the neutral gateway adapter with validated event ordering/indexes, bounded framing and explicit reader cancellation. Preserve server-provided usage and rewrite metadata, distinguish server errors from transport errors, and retain partial output on failures as a deliberate robustness improvement. Reapply hard authority after hooks and keep gateway protocol names stable across native product renaming.

### 9. Size and priority

Estimated Rust implementation: **280-470 lines** (estimate, not measured source). **P1** - Radius/ICE gateway models cannot run through any of the existing three native HTTP dialects; incomplete event reconstruction would also lose usage and diagnostic evidence.

## AI-030 - Bedrock Converse transport, endpoint precedence and signed request headers

### 1. Title and status

Bedrock Converse transport, endpoint precedence and signed request headers. **Status: missing.**

### 2. ice source

ice:packages/ai/src/api/bedrock-converse-stream.ts:71-274; ice:packages/ai/src/api/bedrock-converse-stream.ts:438-472; ice:packages/ai/src/api/bedrock-converse-stream.ts:1054-1102; ice:packages/ai/src/api/bedrock-converse-stream.lazy.ts:1-31; ice:packages/ai/test/bedrock-custom-headers.test.ts:1-203; ice:packages/ai/test/bedrock-endpoint-resolution.test.ts:1-209

Exported symbols: `bedrockConverseStreamApi` (ice:packages/ai/src/api/bedrock-converse-stream.lazy.ts:26).

Exported symbols: `bedrockProviderModule` (ice:packages/ai/src/bedrock-provider.ts:3).

### 3. What it does

Send Bedrock ConverseStream requests through AWS authentication and endpoint rules while preserving custom endpoints, scoped credentials, proxy selection and signed extra headers.

### 4. How it works

Bedrock requests use a fresh AWS SDK client with explicit endpoint and credential precedence. Request headers, cancellation and response metadata pass through that SDK boundary, while several common timeout and retry options are not wired into it.

Direct stream builds a fresh BedrockRuntimeClient per request. Explicit profile is options.profile || options.env.AWS_PROFILE; client profile otherwise comes from scoped/ambient provider environment. Configured region is option, AWS_REGION, AWS_DEFAULT_REGION. Standard endpoint host matches bedrock-runtime[-fips].REGION.amazonaws.com[.cn]; a standard URL is pinned only if no configured region and no ambient AWS_PROFILE. Custom URL is always pinned. Node/Bun region precedence is model ARN region > configured region > pinned standard-endpoint region > us-east-1 only without ambient profile, otherwise SDK chain. Explicit/scoped profile alone still allows endpoint-derived region; the ambient-profile test uses process environment independently of scoped profile. Browser path skips ARN extraction and uses configured/pinned/default region.

Bearer precedence is bearerToken, apiKey, scoped/ambient AWS_BEARER_TOKEN_BEDROCK, unless AWS_BEDROCK_SKIP_AUTH='1'. Bearer config uses token and httpBearerAuth scheme. Skip-auth sets dummy credentials but does not install a no-op signer; describe the actual config rather than promising unsigned requests. Complete scoped/ambient key pair plus optional session token is supplied explicitly only when no explicit/scoped profile and not skip-auth. Proxy selection examines model.baseUrl and scoped environment; a proxy forces Smithy NodeHttpHandler with HTTP/HTTPS agents, otherwise AWS_BEDROCK_FORCE_HTTP1='1' forces HTTP1. Custom headers come from options.headers, are injected in a low-priority build middleware before signing, skip authorization/host/x-amz-* case-insensitively, and use original key spelling for other assignments. Model headers are not directly merged here. Response hook sees SDK status and request ID header when status exists. Signal is passed to client.send.

Common timeout, maxRetries and fetch options are not wired into this client config; AWS SDK defaults still apply. No explicit destroy occurs. Configuration/proxy construction precedes try/catch, a source failure-boundary limitation. Lazy loading uses a variable module specifier rewritten .ts→.js for compiled output; setBedrockProviderModule supports a statically supplied Bun module, while public bedrock-provider exports stream/streamSimple.

`bedrockProviderModule` is the entry-module object `{stream,streamSimple}` imported directly from api/bedrock-converse-stream.ts. It groups the two existing transport functions and adds no separate request execution or auth behavior (ice:packages/ai/src/bedrock-provider.ts:1-6).

### 5. Interfaces

BedrockOptions region, profile, bearerToken, headers, env, signal, onPayload, onResponse, requestMetadata; AWS_PROFILE/AWS_REGION/AWS_DEFAULT_REGION/AWS_ACCESS_KEY_ID/AWS_SECRET_ACCESS_KEY/AWS_SESSION_TOKEN/AWS_BEARER_TOKEN_BEDROCK/AWS_BEDROCK_SKIP_AUTH/AWS_BEDROCK_FORCE_HTTP1; ConverseStreamCommand; setBedrockProviderModule.

### 6. Dependencies

`AI-003`, `AI-006`, `AI-016`, `AUTH-009`, `AUTH-012`

AUTH supplies credential choices and request preparation; AI-016 owns scoped environment/proxy parsing. Native transport implementation must distinguish source SDK behavior from intended common retry/timeout improvements.

### 7. ice-rust today

Independent full reads rust:crates/ice-ai/src/providers/mod.rs:1-47, rust:crates/ice-ai/src/provider.rs:1-392 and rust:crates/ice-ai/src/lib.rs:1-47 plus full 26-file ice-ai/src inventory show no Bedrock Converse implementation. Native Provider/SimpleStreamOptions and common event types exist, with Anthropic/OpenAI/Faux concrete producers only. AUTH-011/012 own catalog/credential selection; those do not implement the missing AWS wire protocol. Independently read rust:crates/ice-cli/src/provider/factories.rs:16-145 confirms production default factories only register the three native HTTP protocols; configure first tries a matching provider-specific extension factory, then protocol factory, otherwise returns None. A native external provider seam exists and its private execution binding and API-matching checks should be retained; no matching installed package implementation is inferred.

### 8. Port notes

Implement an AWS Converse provider behind native Provider with explicit regional/endpoint resolution, credential provider chain or bearer path, proxy support and middleware-equivalent header policy. Wrap all setup failures into the returned assistant stream and make native timeout/retry configuration explicit. Keep auth storage/catalog outside this adapter. Expose direct Bedrock options without forcing AWS-only fields into every provider.

### 9. Size and priority

Estimated Rust implementation: **330-560 lines** (estimate, not measured source). **P1** - Bedrock catalog/auth availability cannot produce requests without its distinct transport and signature contract.

## AI-031 - Bedrock Claude reasoning payloads and bounded output allowance

### 1. Title and status

Bedrock Claude reasoning payloads and bounded output allowance. **Status: missing.**

### 2. ice source

ice:packages/ai/src/api/bedrock-converse-stream.ts:235-265; ice:packages/ai/src/api/bedrock-converse-stream.ts:474-524; ice:packages/ai/src/api/bedrock-converse-stream.ts:650-709; ice:packages/ai/src/api/bedrock-converse-stream.ts:1104-1166; ice:packages/ai/test/bedrock-thinking-payload.test.ts:1-267

Exported symbols: `BedrockThinkingDisplay` (ice:packages/ai/src/api/bedrock-converse-stream.ts:69).

### 3. What it does

Map direct/simple reasoning options into Bedrock Claude adaptive or fixed-budget fields and preserve the hard output ceiling through payload hooks.

### 4. How it works

The Bedrock adapter builds shared inference fields and Claude-specific thinking controls before sending a command. It checks the hard output limit again after the payload hook, while model family and region determine the reasoning fields it emits.

Direct inference maxTokens is caller maxTokens clamped to hard limit, else Claude model.maxTokens clamped to hard limit, else only hard limit if present; non-Claude requests without a cap omit maxTokens. Temperature is included whenever defined, without a Bedrock-specific reasoning incompatibility gate. onPayload can replace the complete command input; afterward hardMaxOutputTokens is re-enforced inside inferenceConfig. requestMetadata is passed unchanged, with documented tag constraints not locally validated.

Claude recognition checks id for anthropic.claude or anthropic/claude and name additionally for claude. Version matching lowercases id and name and also normalizes spaces/underscore/dot/colon to hyphen. Adaptive list is opus4.6/4.7/4.8/5, sonnet4.6/5 and fable5. With reasoning requested and model.reasoning true, adaptive fields are thinking{type:adaptive, display} and output_config.effort. xhigh is forced native for opus4.7/4.8/5, sonnet5, fable5; otherwise string thinkingLevelMap entry wins, then minimal/low→low, medium→medium, high/default→high. Extended levels therefore need explicit map outside native-xhigh support.

Fixed-budget direct reasoning defaults minimal 1024, low 2048, medium 8192, high 16384; xhigh/max/ultra use high override/default. It emits enabled/budget_tokens and defaults interleavedThinking true with the exact interleaved-thinking beta. thinkingDisplay defaults summarized, but is entirely omitted for configured us-gov region or model IDs starting us-gov. or arn:aws-us-gov:. Non-Claude reasoning adds no additional fields. streamSimple uses shared base options; absent reasoning removes it, adaptive forwards levels/budgets, fixed-budget adjusts output then context/hard clamps and limits chosen budget to max(0, maxTokens-1024). The transport redacted the assignment name at 503, so only surrounding visible policy is asserted. Credential-gated live test exists at 165–196; it was read but not run.

### 5. Interfaces

BedrockOptions.reasoning, thinkingBudgets, interleavedThinking, thinkingDisplay, requestMetadata; inferenceConfig.maxTokens; additionalModelRequestFields.thinking/output_config/anthropic_beta; model.name fallback for application inference profiles.

### 6. Dependencies

`AI-005`, `AI-006`, `AI-008`, `AI-030`

### 7. ice-rust today

Independent full reads rust:crates/ice-ai/src/providers/mod.rs:1-47, rust:crates/ice-ai/src/provider.rs:1-392 and rust:crates/ice-ai/src/lib.rs:1-47 plus full 26-file ice-ai/src inventory show no Bedrock Converse implementation. Native Provider/SimpleStreamOptions and common event types exist, with Anthropic/OpenAI/Faux concrete producers only. AUTH-011/012 own catalog/credential selection; those do not implement the missing AWS wire protocol. Native shared options have reasoning/budgets/max_tokens but no immutable hardMaxOutputTokens or direct Bedrock display/interleaved/requestMetadata carrier; those prerequisites are scoped to AI-006. Independently read rust:crates/ice-cli/src/provider/factories.rs:16-145 confirms production default factories only register the three native HTTP protocols; configure first tries a matching provider-specific extension factory, then protocol factory, otherwise returns None. A native external provider seam exists and its private execution binding and API-matching checks should be retained; no matching installed package implementation is inferred.

### 8. Port notes

Add Bedrock-specific reasoning builders and exact source model/name matching. Reuse shared allowance calculation without allowing payload hooks or adaptive/default expansion to exceed hard authority. Keep source limitations (direct fixed budget can be incompatible with a small cap; unconditional temperature) explicit design decisions, and preserve GovCloud omission.

### 9. Size and priority

Estimated Rust implementation: **190-320 lines** (estimate, not measured source). **P1** - Thinking and output limits differ by Claude generation and endpoint; wrong payloads fail or consume unintended output allowance.

## AI-032 - Bedrock history projection, multimodal tool results and cache points

### 1. Title and status

Bedrock history projection, multimodal tool results and cache points. **Status: missing.**

### 2. ice source

ice:packages/ai/src/api/bedrock-converse-stream.ts:711-805; ice:packages/ai/src/api/bedrock-converse-stream.ts:807-1035; ice:packages/ai/src/api/bedrock-converse-stream.ts:1168-1196; ice:packages/ai/test/bedrock-convert-messages.test.ts:1-275

### 3. What it does

Encode normalized history in Converse roles and content unions, combine parallel tool results, preserve signed thinking and place supported cache markers.

### 4. How it works

Bedrock history conversion translates neutral messages into Converse content blocks and groups consecutive tool results. It normalizes IDs, validates image formats and chooses cache points according to the model and retention policy.

transformMessages first normalizes tool IDs by replacing nonalphanumeric/underscore/hyphen with underscore and truncating 64. User strings require a nonblank sanitized text block or '<empty>'; arrays drop blank text and unknown block types but preserve supported images, adding placeholder only when all content disappears. Assistant empty arrays/all-filtered content are skipped. Text/thinking is surrogate-sanitized; blank thinking is omitted even if a signature exists. Claude thinking with missing/blank signature falls back to text, otherwise reasoningText carries text+signature; non-Claude reasoningText omits signature. Consecutive toolResult messages are combined into one user message, each toolUseId/status and ordered text/image content; empty result becomes '<empty>'. Images decode base64 into bytes and accept jpeg/jpg/png/gif/webp only; unknown MIME throws.

Retention explicit value wins, else scoped ICE_CACHE_RETENTION='long', else short. Cache support examines normalized id/name: a Claude reference plus fable/opus/sonnet5, any '-4-', claude3.7sonnet or 3.5haiku is allowed. AWS_BEDROCK_FORCE_CACHE='1' applies only when no Claude reference was found, not an override for an unsupported known Claude. Cache points follow a nonempty system text and the final message only if that final message is user; long adds ONE_HOUR TTL, none omits. Tools preserve full JSON schema under inputSchema.json, with native strict true only when constraint resolution and compat.supportsStrictMode allow. Empty tools or toolChoice='none' omit entire toolConfig. auto/any/named choices serialize distinct Converse toolChoice variants; unspecified choice omits only that field. No tool cache point is added.

### 5. Interfaces

Converse Message role/content, reasoningContent.reasoningText, toolUse/toolResult/status, binary image source; ToolConfiguration.toolSpec.inputSchema.json/strict/toolChoice; CachePointType.DEFAULT and CacheTTL.ONE_HOUR; AWS_BEDROCK_FORCE_CACHE, ICE_CACHE_RETENTION.

### 6. Dependencies

`AI-011`, `AI-013`, `AI-014`, `AI-030`

### 7. ice-rust today

Independent full reads rust:crates/ice-ai/src/providers/mod.rs:1-47, rust:crates/ice-ai/src/provider.rs:1-392 and rust:crates/ice-ai/src/lib.rs:1-47 plus full 26-file ice-ai/src inventory show no Bedrock Converse implementation. Native Provider/SimpleStreamOptions and common event types exist, with Anthropic/OpenAI/Faux concrete producers only. AUTH-011/012 own catalog/credential selection; those do not implement the missing AWS wire protocol. Native Anthropic message conversion is a different wire union and is not a Converse substitute. Reuse shared projection/schema infrastructure only after preserving these Bedrock-specific blank, image, signature and cache rules. Independently read rust:crates/ice-cli/src/provider/factories.rs:16-145 confirms production default factories only register the three native HTTP protocols; configure first tries a matching provider-specific extension factory, then protocol factory, otherwise returns None. A native external provider seam exists and its private execution binding and API-matching checks should be retained; no matching installed package implementation is inferred.

### 8. Port notes

Create a typed Converse history/tool encoder with source-compatible ID normalization, consecutive-result grouping and cache eligibility. Reuse common sanitation and constrained-schema validation; keep MIME decoding failures inside stream setup. Preserve full tool schemas and avoid injecting Anthropic JSON message blocks directly into Bedrock.

### 9. Size and priority

Estimated Rust implementation: **280-460 lines** (estimate, not measured source). **P1** - Role grouping, empty blocks, signatures and cache markers determine whether Bedrock accepts multi-turn tool conversations.

## AI-033 - Bedrock event-stream deltas, completion integrity and failure metadata

### 1. Title and status

Bedrock event-stream deltas, completion integrity and failure metadata. **Status: missing.**

### 2. ice source

ice:packages/ai/src/api/bedrock-converse-stream.ts:109-135; ice:packages/ai/src/api/bedrock-converse-stream.ts:267-436; ice:packages/ai/src/api/bedrock-converse-stream.ts:526-648; ice:packages/ai/src/api/bedrock-converse-stream.ts:1037-1052; ice:packages/ai/test/bedrock-error-metadata.test.ts:1-213; ice:packages/ai/test/bedrock-raw-stop-reason.test.ts:1-83

### 3. What it does

Turn AWS Converse streaming union events into ordered assistant deltas, retain raw stop reasons and diagnostic correlation, and reject an incomplete response.

### 4. How it works

The Bedrock stream mapper reconstructs indexed text, thinking and tool blocks from AWS events. It preserves raw stop reasons and rejects unfinished streams while carrying available usage and diagnostic metadata into the terminal result.

messageStart must be assistant and emits start. Tool start allocates an indexed tool block with empty arguments/scratch JSON; text/thinking blocks are lazily created on their first delta. Text deltas append and emit immediately, tool input reparses cumulative JSON, thinking text appends and emits while signature chunks append silently. contentBlockStop removes SDK index, emits the corresponding end event, and finalizes/removes tool scratch. Metadata overwrites input/output/cacheRead/cacheWrite with value-or-zero and recalculates cost; total assignment at 617 is transport-redacted, so the unknown operand is not reconstructed, with visible fallback input+output. messageStop retains raw reason; end_turn/stop_sequence→stop, max_tokens/model_context_window_exceeded→length, tool_use→toolUse, other→error plus 'Provider stopped with: VALUE'. EOF requires a nonpending reason; abort is checked again after iteration. Error/aborted mapped stop triggers error terminal. Event union exceptions for internal/model-stream/validation/throttling/unavailable are thrown; processing continues after messageStop until iterator ends.

Catch preserves partial content, removes index and partialJson from every block, maps signal-aborted to aborted, formats normalized raw body/status when message omitted body and adds a data-retention documentation hint on matching text. SDK ServiceException names receive stable human prefixes used by downstream retry classification. A separate bedrock_response_failure diagnostic is emitted only for nonabort errors with known details: numeric status, Error.name ending Exception, request ID from error metadata or captured response. Header-derived strings are trim/nonempty/max 200, dropped if oversized; bare object stream exceptions may supply only fallback request ID. Error message is not modified by diagnostics. Success cleanup relies on block-stop events and does not globally strip unfinished-block scratch at done, a source limitation to address deliberately.

### 5. Interfaces

AWS response.stream union; AssistantMessageEvent start/text_*/thinking_*/toolcall_*/done/error; rawStopReason; bedrock_response_failure details.status/errorCode/requestId; options.signal and onResponse.

### 6. Dependencies

`AI-004`, `AI-009`, `AI-010`, `AI-011`, `AI-030`

AI-004 supplies common retry contracts; source Bedrock delegates request retry to SDK defaults and has no explicit replay-after-delta loop. AI-009/010 own generic diagnostics/error normalization, not the AWS metadata mapping.

### 7. ice-rust today

Independent full reads rust:crates/ice-ai/src/providers/mod.rs:1-47, rust:crates/ice-ai/src/provider.rs:1-392 and rust:crates/ice-ai/src/lib.rs:1-47 plus full 26-file ice-ai/src inventory show no Bedrock Converse implementation. Native Provider/SimpleStreamOptions and common event types exist, with Anthropic/OpenAI/Faux concrete producers only. AUTH-011/012 own catalog/credential selection; those do not implement the missing AWS wire protocol. Existing native SSE decoders do not parse AWS event-stream unions. Common assistant stream/result types can be reused, retaining cancellation and partial-message delivery rather than duplicating them. Independently read rust:crates/ice-cli/src/provider/factories.rs:16-145 confirms production default factories only register the three native HTTP protocols; configure first tries a matching provider-specific extension factory, then protocol factory, otherwise returns None. A native external provider seam exists and its private execution binding and API-matching checks should be retained; no matching installed package implementation is inferred.

### 8. Port notes

Implement a Converse stream mapper using AWS SDK events or a native event-stream decoder, with correct indexed block lifecycle, usage, raw reasons and bounded diagnostic metadata. Require terminal integrity, clean scratch at all terminal paths and preserve cancellation during stalled reads. Keep retries before any emitted content unless an explicit resumable protocol is introduced.

### 9. Size and priority

Estimated Rust implementation: **260-440 lines** (estimate, not measured source). **P1** - Reliable completion/error distinction and recoverable partial output are necessary for tool loops and retry classification.

## AI-034 - Google Generative AI request transport, hooks and retry envelope

### 1. Title and status

Google Generative AI request transport, hooks and retry envelope. **Status: missing.**

### 2. ice source

ice:packages/ai/src/api/google-generative-ai.ts:40-101; ice:packages/ai/src/api/google-generative-ai.ts:303-422; ice:packages/ai/src/api/google-shared.ts:384-415; ice:packages/ai/src/api/google-generative-ai.lazy.ts:1-5; ice:packages/ai/test/google-shared-retry.test.ts:1-41

### 3. What it does

Create Gemini API requests with explicit credentials, configured endpoint/headers, payload observation, bounded output and pre-stream retry.

### 4. How it works

The Google adapter constructs a GoogleGenAI request and converts its stream into the common assistant events. Direct and simple calls have different setup-error timing, and retries apply to stream acquisition rather than iteration.

Direct stream returns an assistant event stream and performs setup inside its async catch boundary. It rejects a custom fetch only when supplied and not the current globalThis.fetch, requires truthy options.apiKey with no local environment fallback, constructs GoogleGenAI and builds model/contents/config. streamSimple instead checks missing API key synchronously before returning any stream, then uses common base options and explicit thinking enable/disable mapping. Model baseUrl, when truthy, is passed verbatim with httpOptions.apiVersion='' regardless of whether it already contains a version. Model headers and request headers are spread in that order then converted to SDK headers. Temperature is emitted whenever defined; systemInstruction only for truthy system prompt after surrogate sanitation; tools only when nonempty. No timeout or onResponse integration is present in this source adapter.

onPayload can replace the entire params object; hardMaxOutputTokens is enforced again in params.config.maxOutputTokens afterward. The initial maxOutputTokens assignment at 377 was transport-redacted and is not reconstructed. Abort is rejected before request and its signal passed in config. retryGoogleRequest wraps only generateContentStream acquisition, forwarding maxRetries/maxRetryDelayMs/signal to common provider retry; an Error with status but no headers is given an own headers:undefined field to satisfy common status retry classification. It does not retry errors from later async iteration or arbitrary plain objects. Lazy factory imports the implementation through lazyApi. Mock retry tests pin unset maxRetries as no retries; no live request was executed.

### 5. Interfaces

GoogleOptions extends StreamOptions, toolChoice auto/none/any, thinking{enabled, budgetTokens?, level?}; GoogleGenAI apiKey/httpOptions; GenerateContentParameters model, contents, config; onPayload; custom-fetch rejection; retryGoogleRequest.

### 6. Dependencies

`AI-003`, `AI-004`, `AI-006`, `AUTH-009`

Credential lookup precedes the direct adapter; AUTH-009 request preparation supplies apiKey and headers. Shared history, strict mode and result mapping are separate entries AI-037/038/039, avoiding duplicate effort estimates.

### 7. ice-rust today

Independent full reads rust:crates/ice-ai/src/providers/mod.rs:1-47, rust:crates/ice-ai/src/provider.rs:1-392, rust:crates/ice-ai/src/lib.rs:1-47 and the full 26-file ice-ai/src inventory establish that no Google Generative AI or Vertex concrete producer is present. Native common provider/event/schema infrastructure is usable but Anthropic/OpenAI wire adapters do not implement Google Content/Part and generateContentStream. AUTH-011/012 cover catalog/auth metadata only; external package factories are an extension seam, not evidence of an installed Google wire implementation. Native SimpleStreamOptions has no payload/response hooks, custom fetch carrier, strict tool choice or provider-specific thinking struct; AI-006 owns shared envelope additions. Independently read rust:crates/ice-cli/src/provider/factories.rs:16-145 confirms production default factories only register the three native HTTP protocols; configure first tries a matching provider-specific extension factory, then protocol factory, otherwise returns None. A native external provider seam exists and its private execution binding and API-matching checks should be retained; no matching installed package implementation is inferred.

### 8. Port notes

Implement a Gemini producer with an idiomatic native HTTP or SDK client, explicit auth/endpoint/header rules and a catch-all setup-to-stream boundary. Reuse common bounded retry only before streamed output. Carry hard authority after payload transforms. Expose deliberate native timeout/transport injection improvements rather than asserting the source already supports them.

### 9. Size and priority

Estimated Rust implementation: **190-320 lines** (estimate, not measured source). **P1** - Google catalog entries need their actual request transport before any shared protocol parity is reachable.

## AI-035 - Vertex API-key versus ADC client selection and custom endpoint scope

### 1. Title and status

Vertex API-key versus ADC client selection and custom endpoint scope. **Status: missing.**

### 2. ice source

ice:packages/ai/src/api/google-vertex.ts:45-119; ice:packages/ai/src/api/google-vertex.ts:320-520; ice:packages/ai/test/google-vertex-api-key-resolution.test.ts:1-224; ice:packages/ai/src/api/google-vertex.lazy.ts:1-5

### 3. What it does

Select the correct Vertex authentication/client form and endpoint resource scope without sending credential markers as API keys.

### 4. How it works

Vertex selects API-key authentication or application-default credentials before constructing its GoogleGenAI client. Explicit options, scoped environment values and custom endpoint shape determine project, location and resource addressing.

resolveApiKey trims only options.apiKey and treats empty, exact gcp-vertex-credentials, or /^<[^>]+>$/ placeholder as absent. It does not itself consult GOOGLE_CLOUD_API_KEY; that is upstream credential preparation, despite a test title referring to environment. A real key creates GoogleGenAI{vertexai:true, apiKey, apiVersion:'v1'} without project/location. Otherwise ADC requires project from option, scoped/ambient GOOGLE_CLOUD_PROJECT then GCLOUD_PROJECT and location from option then GOOGLE_CLOUD_LOCATION, throwing descriptive errors if missing. GOOGLE_APPLICATION_CREDENTIALS via scoped/ambient provider environment becomes googleAuthOptions.keyFilename; ambient SDK credential resolution is left to GoogleGenAI.

Custom baseUrl is trimmed; empty or containing literal {location} is omitted so generated templates are not sent. A retained URL sets ResourceScope.COLLECTION; if any URL path segment matches vDIGITS optionally betaDIGITS (beta digits optional), httpOptions.apiVersion='' avoids double version. Invalid URL strings use equivalent slash regex fallback. Both auth clients merge model/request headers and use this HTTP option builder. Request config/hook/hard limit/abort and pre-stream retry follow the same ordering as Gemini; custom non-global fetch is rejected, and timeout/onResponse are not wired. streamSimple builds shared base options without a local key gate and then delegates to the direct API. Exact credential marker tests and constructor option tests were read; test environment assignment values redacted by transport were not recovered.

### 5. Interfaces

GoogleVertexOptions.project/location plus common Google thinking/tool options; GOOGLE_CLOUD_PROJECT, GCLOUD_PROJECT, GOOGLE_CLOUD_LOCATION, GOOGLE_APPLICATION_CREDENTIALS; gcp-vertex-credentials and angle-bracket placeholder markers; ResourceScope.COLLECTION; GoogleGenAI vertexai/apiVersion.

### 6. Dependencies

`AI-003`, `AI-004`, `AI-006`, `AI-016`, `AUTH-009`, `AUTH-012`

### 7. ice-rust today

Independent full reads rust:crates/ice-ai/src/providers/mod.rs:1-47, rust:crates/ice-ai/src/provider.rs:1-392, rust:crates/ice-ai/src/lib.rs:1-47 and the full 26-file ice-ai/src inventory establish that no Google Generative AI or Vertex concrete producer is present. Native common provider/event/schema infrastructure is usable but Anthropic/OpenAI wire adapters do not implement Google Content/Part and generateContentStream. AUTH-011/012 cover catalog/auth metadata only; external package factories are an extension seam, not evidence of an installed Google wire implementation. A generic bearer key header is insufficient to replace ADC token acquisition/project/location routing; native source inventory has no Vertex client implementation. Independently read rust:crates/ice-cli/src/provider/factories.rs:16-145 confirms production default factories only register the three native HTTP protocols; configure first tries a matching provider-specific extension factory, then protocol factory, otherwise returns None. A native external provider seam exists and its private execution binding and API-matching checks should be retained; no matching installed package implementation is inferred.

### 8. Port notes

Implement Vertex as a distinct client/auth configuration on the shared Google wire mapper, accepting an explicit resolved credential mode rather than inferring all auth from arbitrary strings. Preserve source marker compatibility at the public boundary, regional collection URL behavior and scoped key-file selection. Reuse AUTH storage/selection; add no duplicate credential store.

### 9. Size and priority

Estimated Rust implementation: **200-340 lines** (estimate, not measured source). **P1** - Vertex API keys and ADC need different request construction; placeholder credentials and versioned proxies otherwise fail before generation.

## AI-036 - Gemini and Vertex thinking levels, disable policy and token budgets

### 1. Title and status

Gemini and Vertex thinking levels, disable policy and token budgets. **Status: missing.**

### 2. ice source

ice:packages/ai/src/api/google-generative-ai.ts:303-342; ice:packages/ai/src/api/google-generative-ai.ts:392-405; ice:packages/ai/src/api/google-generative-ai.ts:424-535; ice:packages/ai/src/api/google-vertex.ts:320-357; ice:packages/ai/src/api/google-vertex.ts:491-503; ice:packages/ai/src/api/google-vertex.ts:522-610; ice:packages/ai/test/google-thinking-disable.test.ts:1-161

### 3. What it does

Translate common reasoning choices into Google level or budget payloads, including model-specific lowest-thinking behavior when the caller requests no visible reasoning.

### 4. How it works

Google thinking configuration depends on the model family and whether the caller uses direct or simple options. Explicit enable, disable, effort and token-budget values are translated separately, with the hard output limit bounding nonnegative thinking budgets.

Direct API applies thinking only for model.reasoning. Enabled emits includeThoughts=true and prefers explicit level over budget; nonnegative budget is min(budget, hardMaxOutputTokens) when hard cap exists, while negative dynamic budget is retained. An absent thinking option emits no thinkingConfig; explicit enabled=false emits a model-specific disable form. Simple absent reasoning always passes enabled=false. Simple requested reasoning is clampThinkingLevel(model, requested), coerces returned off to high, then casts to minimal/low/medium/high without a further runtime extended-level guard.

Gemini3 Pro matches /gemini-3(?:\.\d+)?-pro/ and maps minimal/low→LOW, medium/high→HIGH; disable uses LOW without includeThoughts. Gemini3 Flash matches equivalent flash regex or exact gemini-flash-latest/gemini-flash-lite-latest, maps minimal/low/medium/high→MINIMAL/LOW/MEDIUM/HIGH and disables with MINIMAL. Generative AI also recognizes /gemma-?4/: minimal/low→MINIMAL, medium/high→HIGH and disable MINIMAL; Vertex has no Gemma4 special case. Other models disable with thinkingBudget0.

Budget override by selected level wins. Case-sensitive id substring 2.5-pro defaults 128/2048/8192/32768. Generative AI2.5-flash-lite defaults 512/2048/8192/24576 before its 2.5-flash 128/2048/8192/24576 branch; Vertex has only the latter flash branch, so flash-lite minimal differs 128. Unknown budget family returns-1. Level helpers do not handle extended values at runtime if a custom model advertises them; the cast alone cannot clamp them, a source limitation. Output cap is separately enforced after payload hook, but thinking budget itself is not re-clamped after arbitrary hook replacement. The thinking-disable test spans other providers and uses credential-gated E2E assertions; it establishes intended cases only, not executed acceptance.

### 5. Interfaces

thinkingConfig includeThoughts, thinkingLevel, thinkingBudget; GoogleThinkingLevel enum-string mapping; direct thinking.enabled vs absent; common reasoning/thinkingBudgets/hardMaxOutputTokens.

### 6. Dependencies

`AI-005`, `AI-006`, `AI-034`, `AI-035`

### 7. ice-rust today

Independent full reads rust:crates/ice-ai/src/providers/mod.rs:1-47, rust:crates/ice-ai/src/provider.rs:1-392, rust:crates/ice-ai/src/lib.rs:1-47 and the full 26-file ice-ai/src inventory establish that no Google Generative AI or Vertex concrete producer is present. Native common provider/event/schema infrastructure is usable but Anthropic/OpenAI wire adapters do not implement Google Content/Part and generateContentStream. AUTH-011/012 cover catalog/auth metadata only; external package factories are an extension seam, not evidence of an installed Google wire implementation. Native shared reasoning enum/budgets are not converted into Google generationConfig. Preserve native common clamp infrastructure while adding exact Google model branches. Independently read rust:crates/ice-cli/src/provider/factories.rs:16-145 confirms production default factories only register the three native HTTP protocols; configure first tries a matching provider-specific extension factory, then protocol factory, otherwise returns None. A native external provider seam exists and its private execution binding and API-matching checks should be retained; no matching installed package implementation is inferred.

### 8. Port notes

Implement shared Google reasoning conversion with explicit per-API differences for Gemma4 and flash-lite. Validate extended-level handling instead of relying on a type cast, and define whether hooks may change thinking budgets under hard authority. Preserve hidden minimum reasoning semantics rather than promising the provider performs zero internal reasoning.

### 9. Size and priority

Estimated Rust implementation: **170-270 lines** (estimate, not measured source). **P1** - Reasoning-off and budget behavior materially affect latency, cost and provider request validity.

## AI-037 - Google history signatures, function IDs and multimodal response grouping

### 1. Title and status

Google history signatures, function IDs and multimodal response grouping. **Status: missing.**

### 2. ice source

ice:packages/ai/src/api/google-shared.ts:18-248; ice:packages/ai/test/google-shared-gemini3-unsigned-tool-call.test.ts:1-168; ice:packages/ai/test/google-shared-signed-empty-blocks.test.ts:1-118; ice:packages/ai/test/google-shared-image-tool-result-routing.test.ts:1-103; ice:packages/ai/test/google-thinking-signature.test.ts:1-39

### 3. What it does

Preserve replay-valid Google signatures and function identity while converting history and tool images into the correct Content/Part arrangement.

### 4. How it works

Google history conversion preserves ordered multimodal content and attaches replay signatures only when identity and syntax checks permit them. Function-call IDs and tool-result image placement vary by model family.

transformMessages precedes conversion. IDs are required for case-sensitive claude-/gpt-oss- prefixes and Gemini major>=3 parsed case-insensitively from ^gemini(?:-live)?-(digits); only required IDs are sanitized to alphanumeric/underscore/hyphen and 64 chars. User strings become sanitized text even if empty; array text/images preserve order, and only empty arrays are skipped. Assistant signature reuse additionally requires same provider/model plus base64-looking nonempty signature, length divisible 4, alphabet A-Z/a-z/0-9/+// and up to 2 trailing=. It is a syntax check, not cryptographic validation. Text/thinking blocks with empty visible content survive only if a usable signature exists. Same-model thinking carries thought:true; foreign thinking becomes plain text without tags and signature. Tool calls carry name, args(default{}), required ID and usable thoughtSignature. Unsigned Gemini3 calls stay unsigned: no skip-validator sentinel and no synthetic historical text is injected.

Tool text joins with newline. Images are retained only if model.input includes image. response is {output:text} or {error:text}; no text with images uses '(see attached image)', no text/images uses ''. For Gemini major>=3 and every non-Gemini model, images nest in functionResponse.parts; Gemini<3 gets a separate user turn with 'Tool result image:' and image parts. The actual helper returns true for non-Gemini, despite a nearby contrary comment. Function responses merge only into the immediately prior user content if it already contains a functionResponse. Thus a separate image turn for Gemini2 can split a consecutive result group; tests explicitly pin this. IDs are added to functionResponse only for required-ID models. Unknown user array types fall through to the image-shaped branch rather than generic unknown-block filtering.

### 5. Interfaces

Google Content role user/model; Part text/thought/thoughtSignature/functionCall/functionResponse/inlineData; TextContent.textSignature, ThinkingContent.thinkingSignature, ToolCall.thoughtSignature; requiresToolCallId; convertMessages.

### 6. Dependencies

`AI-014`, `AI-034`, `AI-035`

### 7. ice-rust today

Independent full reads rust:crates/ice-ai/src/providers/mod.rs:1-47, rust:crates/ice-ai/src/provider.rs:1-392, rust:crates/ice-ai/src/lib.rs:1-47 and the full 26-file ice-ai/src inventory establish that no Google Generative AI or Vertex concrete producer is present. Native common provider/event/schema infrastructure is usable but Anthropic/OpenAI wire adapters do not implement Google Content/Part and generateContentStream. AUTH-011/012 cover catalog/auth metadata only; external package factories are an extension seam, not evidence of an installed Google wire implementation. Native common Content fields and Anthropic/OpenAI history handling cannot substitute for these Google signature rules or functionResponse image layouts. Independently read rust:crates/ice-cli/src/provider/factories.rs:16-145 confirms production default factories only register the three native HTTP protocols; configure first tries a matching provider-specific extension factory, then protocol factory, otherwise returns None. A native external provider seam exists and its private execution binding and API-matching checks should be retained; no matching installed package implementation is inferred.

### 8. Port notes

Add a shared Google history encoder with separate signature validity/provenance checks, explicit model-family ID rules and source-compatible result grouping. Keep signed empty parts and caller tool IDs through persistence. Resolve the non-Gemini multimodal comment discrepancy from actual desired protocol behavior before changing the source-derived policy.

### 9. Size and priority

Estimated Rust implementation: **250-410 lines** (estimate, not measured source). **P1** - Dropping or moving signed parts and mismatching function IDs breaks multi-turn reasoning/tool continuity.

## AI-038 - Google function schemas, validated mode and legacy OpenAPI conversion

### 1. Title and status

Google function schemas, validated mode and legacy OpenAPI conversion. **Status: missing.**

### 2. ice source

ice:packages/ai/src/api/google-shared.ts:250-337; ice:packages/ai/src/api/google-generative-ai.ts:380-390; ice:packages/ai/src/api/google-vertex.ts:479-489; ice:packages/ai/test/google-shared-convert-tools.test.ts:1-204

Exported symbols: `supportsGoogleStrictToolSampling`, `mapToolChoice` (ice:packages/ai/src/api/google-shared.ts:304; ice:packages/ai/src/api/google-shared.ts:310).

### 3. What it does

Publish tool declarations using the correct schema field and choose validated function-calling mode when strict sampling is requested and supported.

### 4. How it works

Google tool conversion uses full JSON Schema by default and supports a separate legacy parameter conversion. Strict sampling and tool choice determine the function-calling mode, with explicit errors when a required capability is unsupported.

convertTools returns undefined for empty input, otherwise one wrapper containing functionDeclarations in input order with name, description and full parametersJsonSchema by default. Optional useParameters=true uses legacy parameters and recursively clones ordinary objects while dropping $schema,$id,$anchor,$dynamicAnchor,$vocabulary,$comment,$defs, definitions. Arrays are returned untouched, so this is not recursive through anyOf/items arrays; $ref is retained even if definitions are removed. The sanitizer also has no schema-position distinction when dropping matching keys. These are source limitations, not proof of a valid fully transformed OpenAPI schema. Both direct Gemini/Vertex builders use the default JSON-schema branch.

Strict support is inferred from Gemini parsed major>=3, not a general compat override. resolveGoogleFunctionCallingMode evaluates tools.some(resolveJsonSchemaStrictSampling===true) before handling toolChoice; a required unsupported tool encountered before short-circuit throws even if choice='none'. Conversely, once an earlier tool returns true, later tool constraints are not checked by this some call. none→NONE and any→ANY take precedence over strict mode after this evaluation; otherwise any strict result→VALIDATED, explicit auto/unknown→AUTO, absent choice→undefined. Tool declarations remain present when mode NONE, unlike Bedrock's omission policy.

### 5. Interfaces

functionDeclarations.parametersJsonSchema vs parameters; Google FunctionCallingConfigMode AUTO/NONE/ANY/VALIDATED; Tool.constrainedSampling; convertTools(tools, useParameters=false).

### 6. Dependencies

`AI-013`, `AI-034`, `AI-035`

### 7. ice-rust today

Independent full reads rust:crates/ice-ai/src/providers/mod.rs:1-47, rust:crates/ice-ai/src/provider.rs:1-392, rust:crates/ice-ai/src/lib.rs:1-47 and the full 26-file ice-ai/src inventory establish that no Google Generative AI or Vertex concrete producer is present. Native common provider/event/schema infrastructure is usable but Anthropic/OpenAI wire adapters do not implement Google Content/Part and generateContentStream. AUTH-011/012 cover catalog/auth metadata only; external package factories are an extension seam, not evidence of an installed Google wire implementation. Native strict-schema and constrained-sampling infrastructure exists, but no Google declaration builder or VALIDATED mode serializer is present. Independently read rust:crates/ice-cli/src/provider/factories.rs:16-145 confirms production default factories only register the three native HTTP protocols; configure first tries a matching provider-specific extension factory, then protocol factory, otherwise returns None. A native external provider seam exists and its private execution binding and API-matching checks should be retained; no matching installed package implementation is inferred.

### 8. Port notes

Implement schema-field selection and validated-mode request integration using shared constraint policy. Preserve complete JSON Schema in default mode. Treat legacy schema transformation as a deliberate compatibility path, address dangling refs/array traversal if strengthening it, and validate all required constraints rather than preserving accidental short-circuit omissions.

### 9. Size and priority

Estimated Rust implementation: **100-180 lines** (estimate, not measured source). **P1** - Strict tool requirements and schema compatibility must remain enforceable at the provider boundary.

## AI-039 - Google and Vertex stream block assembly, usage and finish-reason precedence

### 1. Title and status

Google and Vertex stream block assembly, usage and finish-reason precedence. **Status: missing.**

### 2. ice source

ice:packages/ai/src/api/google-generative-ai.ts:99-298; ice:packages/ai/src/api/google-vertex.ts:117-315; ice:packages/ai/src/api/google-shared.ts:35-51; ice:packages/ai/src/api/google-shared.ts:338-415; ice:packages/ai/test/google-raw-stop-reason.test.ts:1-107

Exported symbols: `isThinkingPart`, `retainThoughtSignature`, `mapStopReason`, `mapStopReasonString` (ice:packages/ai/src/api/google-shared.ts:35; ice:packages/ai/src/api/google-shared.ts:48; ice:packages/ai/src/api/google-shared.ts:341; ice:packages/ai/src/api/google-shared.ts:373).

### 3. What it does

Convert Gemini/Vertex streamed parts into assistant events while retaining response/signature identity, usage and the exact completion/error state.

### 4. How it works

Google and Vertex share a mapper that turns the first response candidate into consecutive assistant blocks. It merges compatible chunks, closes blocks at transitions and tracks available usage, signatures and stop reasons.

After SDK stream acquisition emit start; inspect only candidates[0] per chunk and keep the first truthy responseId. Every part with text!==undefined is classified thinking only when thought===true, not by signature alone. Contiguous parts/chunks of the same kind merge into one current block; changing kind or seeing functionCall closes that block. Nonempty incoming thoughtSignature replaces the current retained signature, while omitted/empty leaves it unchanged; text stores textSignature and thinking stores thinkingSignature. The implementation has no distinct part index boundary, so adjacent same-kind signature-bearing parts merge despite a helper comment cautioning against moving signatures across parts. Signature-only parts with neither text nor functionCall are ignored.

Each functionCall is treated as a complete call, not incremental JSON: keep provided unique nonempty ID, else generate name_Date.now_counter with module-local counter; args defaults{}, attach truthy thoughtSignature, append then emit start/delta(JSON.stringify args)/end. A part with text and functionCall may perform both branches. finishReason is retained raw and mapped STOP→stop, MAX_TOKENS→length; listed safety/recitation/malformed/unexpected/unspecified/other/language/image reasons→error, unknown enum values throw. After a recognized mapping, presence of any toolCall overrides it to toolUse, including a mapped error or length. That error precedence is a source limitation to address explicitly; raw-stop tests cover error-only responses without tool calls.

Every usageMetadata replaces usage: input=prompt-cached (not clamped), output=candidates+thoughts, cacheRead=cached, cacheWrite0, reasoning=thoughts, total=reported-or0, then cost recalculates. Normal iterator end closes a current block, checks signal, rejects pending or mapped error, else done. Catch retains partial blocks and usage, removes incidental index fields, sets aborted if signal else error, and normalizes provider error. No retry occurs after iteration begins; continue reading after finishReason so later errors still fail the turn. If iteration throws, the open block is not first ended. Both APIs duplicate this mechanism with their own counter/api identity.

### 5. Interfaces

AssistantMessageEvent stream; responseId/rawStopReason; textSignature/thinkingSignature/thoughtSignature; FinishReason enum mapping; usage.reasoning subset of output; retryGoogleRequest acquisition boundary.

### 6. Dependencies

`AI-004`, `AI-009`, `AI-010`, `AI-034`, `AI-035`, `AI-037`

### 7. ice-rust today

Independent full reads rust:crates/ice-ai/src/providers/mod.rs:1-47, rust:crates/ice-ai/src/provider.rs:1-392, rust:crates/ice-ai/src/lib.rs:1-47 and the full 26-file ice-ai/src inventory establish that no Google Generative AI or Vertex concrete producer is present. Native common provider/event/schema infrastructure is usable but Anthropic/OpenAI wire adapters do not implement Google Content/Part and generateContentStream. AUTH-011/012 cover catalog/auth metadata only; external package factories are an extension seam, not evidence of an installed Google wire implementation. Native SSE streaming/cancellation and event types can be reused, but neither Google candidate/Part mapper nor Google usage/finish semantics is implemented. Independently read rust:crates/ice-cli/src/provider/factories.rs:16-145 confirms production default factories only register the three native HTTP protocols; configure first tries a matching provider-specific extension factory, then protocol factory, otherwise returns None. A native external provider seam exists and its private execution binding and API-matching checks should be retained; no matching installed package implementation is inferred.

### 8. Port notes

Build one shared native Google mapper parameterized by API identity, with explicit block/signature boundaries, complete function-call emission and provider usage accounting. Retain raw reasons and make error-vs-tool precedence an intentional reviewed policy. Reject truncated EOF and clean partial state on every error, preserving native prompt cancellation while not replaying emitted content.

### 9. Size and priority

Estimated Rust implementation: **270-450 lines** (estimate, not measured source). **P1** - Tool loops require trustworthy completion reasons and lossless replay signatures across both Google endpoints.

## AI-040 - Mistral chat-stream transport, reasoning controls and cache affinity

### 1. Title and status

Mistral chat-stream transport, reasoning controls and cache affinity. **Status: missing.**

### 2. ice source

ice:packages/ai/src/api/mistral-conversations.ts:32-143; ice:packages/ai/src/api/mistral-conversations.ts:225-289; ice:packages/ai/src/api/mistral-conversations.ts:646-672; ice:packages/ai/src/api/mistral-conversations.lazy.ts:1-5; ice:packages/ai/test/mistral-reasoning-mode.test.ts:1-99

### 3. What it does

Implement the API named mistral-conversations using Mistral chat.stream, including per-request clients, explicit reasoning mode, prompt-cache affinity and direct options.

### 4. How it works

The Mistral adapter uses the SDK chat-stream endpoint despite its historical API name. Each request gets an isolated client, transformed history and an explicit SDK retry policy, with hard output authority reapplied after payload mutation.

The source API name is mistral-conversations but implementation calls Mistral.chat.stream, not a stateful Conversations endpoint. Direct stream requires truthy options.apiKey inside async error boundary; streamSimple throws synchronously if absent. Client is new per request with apiKey, serverURL=model.baseUrl and optional HTTPClient(fetcher) for custom fetch, isolating mutable SDK state. Shared history projection runs with a fresh request-local Mistral ID normalizer. Payload has model, stream:true, messages; tools when nonempty, temperature/maxTokens when defined, truthy toolChoice/promptMode/reasoningEffort, and truthy system prompt prepended after sanitation. onPayload can replace payload, then hardMaxOutputTokens is enforced on maxTokens. Initial assignment expression at 269 was transport-redacted. Request options always set retries{strategy:'none'}, pass signal and merged model/request headers directly. maxRetries/maxRetryDelayMs/timeout/onResponse are not otherwise applied.

Prompt cache key is undefined when cacheRetention='none', else promptCacheKey ?? sessionId; an explicit empty key suppresses session fallback. Truthy key is sent in payload and auto-added as lowercase x-affinity only if that exact existing header is falsy. Uppercase affinity is not checked, null/undefined header values are not normalized through providerHeadersToRecord, and none does not remove explicit caller affinity headers. This is prefix caching, with no conversation continuation object.

Simple reasoning is clamped; off becomes absent, and model.reasoning must be true. Exact IDs mistral-small-2603, mistral-small-latest, mistral-medium-3.5 use reasoningEffort=model.thinkingLevelMap[level]??'high' cast to none/high without runtime validation. Other reasoning models use promptMode='reasoning'. When simple reasoning is absent, both controls are omitted rather than explicitly set none. Direct options can supply either/both independently. Lazy wrapper delegates to lazyApi. Payload tests point at local port 9 and were inspected without making those requests.

### 5. Interfaces

MistralOptions.toolChoice auto/none/any/required/named-function, promptMode, reasoningEffort; HTTPClient fetcher; payload.promptCacheKey/maxTokens; x-affinity; request retries{strategy:none}; stream/streamSimple API mistral-conversations.

### 6. Dependencies

`AI-003`, `AI-005`, `AI-006`, `AUTH-009`

### 7. ice-rust today

Independent full native reads rust:crates/ice-ai/src/providers/mod.rs:1-47, rust:crates/ice-ai/src/provider.rs:1-392, rust:crates/ice-ai/src/lib.rs:1-47 and rust:crates/ice-cli/src/provider/factories.rs:1-623 show no Mistral-conversations/chat.stream producer. RuntimeFactories registers only AnthropicMessages/OpenaiCompletions/OpenaiResponses; an extension factory may supply a matching provider/API, but no concrete Mistral external package implementation was established. A configured OpenAI-compatible Mistral endpoint can use a different existing wire protocol; that is not implementation of this source API. Common native options already carry session/cache hints, headers and reasoning but lack explicit promptCacheKey and Mistral promptMode/effort/named tool choice, owned envelope additions under AI-006.

### 8. Port notes

Add a Mistral producer and typed direct option carrier around chat-completion streaming. Reuse credential/header binding and hard allowance infrastructure. Preserve explicit source no-retry policy or document an intentional common retry upgrade; do not infer continuation state from affinity. Normalize header semantics deliberately and keep per-request client isolation.

### 9. Size and priority

Estimated Rust implementation: **210-350 lines** (estimate, not measured source). **P1** - Mistral requests require distinct SDK fields and reasoning/cache controls beyond a generic OpenAI-style endpoint.

## AI-041 - Mistral nine-character tool identity, history and symbol-free schemas

### 1. Title and status

Mistral nine-character tool identity, history and symbol-free schemas. **Status: missing.**

### 2. ice source

ice:packages/ai/src/api/mistral-conversations.ts:165-195; ice:packages/ai/src/api/mistral-conversations.ts:507-644; ice:packages/ai/src/utils/hash.ts:1-14; ice:packages/ai/test/mistral-tool-schema.test.ts:1-64

### 3. What it does

Normalize tool identities consistently within each request and serialize Mistral history, tool images/errors and schemas accepted by the SDK.

### 4. How it works

Mistral history conversion assigns request-local tool IDs and reuses each mapping for calls and results. It serializes neutral content into Mistral messages and clones tool schemas into ordinary JSON objects.

A request-local forward/reverse map guarantees repeated original IDs reuse the chosen ID and collisions with distinct originals retry. Derivation strips nonalphanumeric characters; attempt 0 keeps that sanitized value only if length 9, otherwise hashes sanitized-or-original seed, or seed:attempt on collisions. shortHash is the exact two 32-bit UTF16-code-unit algorithm in utils/hash.ts using wrapping Math.imul, final mixing and concatenated unsigned base36 halves; the first 9 alphanumerics are taken. No padding is added, so the code truncates to at most 9 rather than proving fixed length for every possible hash. Preserve deterministic same-request mapping through transformMessages for calls and results.

User text is sanitized; array images become data-URL image_url chunks only when model supports images. Image-only unsupported content gets an explicit omission placeholder; an empty array without images is skipped. Assistant nonblank text and thinking become their respective content chunk kinds, tool calls are a separate toolCalls array with JSON arguments, and empty assistant messages are omitted. Thinking chunk output has no signature carrier. Tool results are individual role=tool messages with toolCallId/name and a text chunk first, then supported images. Text trims surrounding whitespace; errors prefix '[tool error] '. Unsupported accompanying images add an omission suffix; no text yields '(see attached image)', '(image omitted: model does not support images)' or '(no tool output)' as appropriate, with the same error prefix variants.

Each tool emits type:function, name, description, parameters and strict. Strict resolution assumes provider supports strict unconditionally; resulting true/false defaults false. parameters recursively clone arrays and Object.entries of objects, removing symbol keys at every visited level while retaining enumerable string-key schema structure. This avoids TypeBox metadata tripping SDK validation without JSON roundtrip semantics. The test verifies symbol removal and expects a request error at a local unreachable endpoint, not successful service execution.

### 5. Interfaces

createMistralToolCallIdNormalizer/deriveMistralToolCallId; chat message content/toolCalls/toolCallId; ContentChunk text/thinking/image_url; FunctionTool.function.parameters/strict; exact shortHash protocol helper.

### 6. Dependencies

`AI-011`, `AI-013`, `AI-014`, `AI-040`

### 7. ice-rust today

Independent full native reads rust:crates/ice-ai/src/providers/mod.rs:1-47, rust:crates/ice-ai/src/provider.rs:1-392, rust:crates/ice-ai/src/lib.rs:1-47 and rust:crates/ice-cli/src/provider/factories.rs:1-623 show no Mistral-conversations/chat.stream producer. RuntimeFactories registers only AnthropicMessages/OpenaiCompletions/OpenaiResponses; an extension factory may supply a matching provider/API, but no concrete Mistral external package implementation was established. A configured OpenAI-compatible Mistral endpoint can use a different existing wire protocol; that is not implementation of this source API. Existing Rust serde JSON values are naturally symbol-free, so do not add a needless TypeBox compatibility layer; the exact schema, ID, image and error-message contracts still need native serialization.

### 8. Port notes

Implement a collision-aware request-local tool-ID normalizer with compatible UTF16/wrapping hash semantics where identity parity matters, and a Mistral history encoder preserving role/result ordering. Use native JSON directly while honoring full schema and strict flag. Keep omission/error placeholders as intentional wire text and avoid forwarding unsupported images.

### 9. Size and priority

Estimated Rust implementation: **210-350 lines** (estimate, not measured source). **P1** - Tool results must address the same normalized IDs and preserve enough image/error context for reliable follow-up turns.

## AI-042 - Mistral streamed tool reconstruction, cache usage and error completion

### 1. Title and status

Mistral streamed tool reconstruction, cache usage and error completion. **Status: missing.**

### 2. ice source

ice:packages/ai/src/api/mistral-conversations.ts:87-113; ice:packages/ai/src/api/mistral-conversations.ts:197-223; ice:packages/ai/src/api/mistral-conversations.ts:291-505; ice:packages/ai/src/api/mistral-conversations.ts:674-689; ice:packages/ai/test/mistral-raw-stop-reason.test.ts:1-78

### 3. What it does

Map Mistral SDK completion events into assistant deltas with stable response identity, cumulative tool arguments, cache-aware usage and bounded error text.

### 4. How it works

The Mistral mapper assembles text, thinking and tool calls from the first streamed choice. It reads several cached-token aliases, preserves partial content on failure and finalizes the common assistant result.

Emit start after chat.stream acquisition. Each event.data keeps the first truthy chunk.id as responseId and updates usage before selecting choices[0]. Cached prompt tokens probe, in nullish order: promptTokensDetails.cachedTokens, prompt_tokens_details.cached_tokens, promptTokenDetails.cachedTokens, prompt_token_details.cached_tokens, numCachedTokens, num_cached_tokens; accept only finite numeric value and clamp to 0..promptTokens. Input is max(0, prompt-cached), output=completionTokens||0, cacheRead=cached, cacheWrite0, total=reported truthy total else sum of these; cost recalculates each usage event. Two local token assignment expressions at 351–352 were redacted by transport; their names/operands were not recovered beyond visible helper/caller behavior.

Only first choice is used. Truthy finishReason is recorded before delta processing: stop→stop, length/model_length→length, tool_calls→toolUse, error/unknown→error with raw explanation. The helper maps null→stop but caller's truthy guard makes that branch unreachable here. String content and text chunks emit sanitized text, including empty sanitized deltas; thinking chunks concatenate nested text parts, sanitize and skip empty. Same-kind contiguous content merges; changes and tool deltas close current text/thinking.

Tool map key is callId:index-or0. Nonempty ID except literal 'null' is used; otherwise derive from 'toolcall:index-or0'. New block captures initial name and scratch partialArgs. Each arguments string is appended; nonstring args JSON.stringify(value||{}), then tolerant JSON parsing updates arguments and emits delta. Later missing/different ID for the same index selects a different key and can split a logical call; names are not concatenated/updated. These are source limitations, not an established correct provider invariant. At normal EOF finish text/thinking then finalize every mapped tool in insertion order, strip partialArgs and emit toolcall_end. Outer completion then checks abort/pending/error and emits done or error. Catch strips scratch on all tools but does not emit missing end events; partial content remains.

Error formatting uses Error.statusCode if numeric and a trimmed string body truncated at 4000 UTF16 characters with omitted-count suffix. Status without body uses message; Error without status uses message. Non-Error values use safe JSON.stringify then String fallback. No generic structured provider diagnostic or retry wrapper is applied. Unknown stop tests confirm raw preservation and error mapping only, without live requests.

### 5. Interfaces

CompletionEvent.data id/usage/choices[0].delta; toolCalls id/index/function.arguments; AssistantMessageEvent text/thinking/toolcall/done/error; rawStopReason, responseId; Mistral SDK statusCode/body error.

### 6. Dependencies

`AI-010`, `AI-011`, `AI-040`, `AI-041`

### 7. ice-rust today

Independent full native reads rust:crates/ice-ai/src/providers/mod.rs:1-47, rust:crates/ice-ai/src/provider.rs:1-392, rust:crates/ice-ai/src/lib.rs:1-47 and rust:crates/ice-cli/src/provider/factories.rs:1-623 show no Mistral-conversations/chat.stream producer. RuntimeFactories registers only AnthropicMessages/OpenaiCompletions/OpenaiResponses; an extension factory may supply a matching provider/API, but no concrete Mistral external package implementation was established. A configured OpenAI-compatible Mistral endpoint can use a different existing wire protocol; that is not implementation of this source API. Existing OpenAI completion mapper is a potential shared base but its delta keying, content chunk types, Mistral usage aliases and error envelope differ; reuse needs explicit protocol configuration rather than labeling it implemented.

### 8. Port notes

Implement a Mistral completion mapper with explicit tool correlation, streamed content variants and alias-aware cache accounting. Preserve raw reasons and reject missing finish at EOF. Correctly handle IDs arriving once or changing after index allocation, define name delta behavior and clean partial buffers on every terminal path. Keep cancellation responsive and avoid replaying emitted deltas under request retry.

### 9. Size and priority

Estimated Rust implementation: **260-430 lines** (estimate, not measured source). **P1** - Tool correlation, cached usage and terminal integrity determine whether the native harness can safely consume Mistral streams.

## AI-050 - Chat Completions prompt cache, session affinity and Anthropic-style cache markers

### 1. Title and status

Chat Completions prompt cache, session affinity and Anthropic-style cache markers. **Status: missing.**

### 2. ice source

ice:packages/ai/src/api/openai-completions.ts:187-195 (retention fallback), ice:packages/ai/src/api/openai-completions.ts:223-242 (cache identity selection), ice:packages/ai/src/api/openai-completions.ts:648-691 (affinity headers), ice:packages/ai/src/api/openai-completions.ts:704-755 and ice:packages/ai/src/api/openai-completions.ts:936-1050 (payload and marker placement); ice:packages/ai/test/openai-completions-prompt-cache.test.ts:66-280 and ice:packages/ai/test/openai-completions-cache-control-format.test.ts:125-231.

### 3. What it does

Cache identity and transport affinity are separate per-request inputs, and compatible proxies can receive explicit Anthropic cache markers. Rust's Chat Completions request path does not currently project these controls.

### 4. How it works

Retention resolves from an explicit truthy option, otherwise ICE_CACHE_RETENTION=long, otherwise short. Retention none disables generated prompt-cache keys and generated session-affinity headers before the client and payload are built.

For direct OpenAI base URLs containing api.openai.com, enabled caching sends a key derived from promptCacheKey or sessionId and truncated to 64 Unicode code points. Other base URLs receive it only for long retention with supportsLongCacheRetention. Long supported retention sends prompt_cache_retention='24h'. The code uses substring detection, not a parsed-host equality check; do not silently strengthen that historical behavior without a decision. promptCacheKey can share a fork prefix while sessionId remains unique to the child transport. An undefined identity omits the key (ice:packages/ai/src/api/openai-completions.ts:704-718; ice:packages/ai/test/openai-completions-prompt-cache.test.ts:116-176).

Affinity is opt-in through sendSessionAffinityHeaders. openrouter uses x-session-id only; openai sets session_id plus x-client-request-id and x-session-affinity; openai-nosession omits session_id but retains the other two. Provider/URL detection chooses the format, not whether to send. Explicit caller headers merge last and may override these values. Retention none prevents generated values but does not strip explicit caller headers (ice:packages/ai/src/api/openai-completions.ts:648-691; ice:packages/ai/test/openai-completions-prompt-cache.test.ts:178-279).

cacheControlFormat='anthropic' with non-none retention creates {type:'ephemeral'}, adding ttl:'1h' only for long supported retention. It marks the first system/developer instruction, the last tool, and the last user/assistant/tool message with markable text, scanning backward if the final message is unmarkable. Nonempty string content becomes a text-part array; array content marks its last text part. Images alone do not receive this marker. This mutation happens after messages/tools conversion and before arbitrary sampling/onPayload overrides. The OpenRouter Anthropic default and final tool-result marker are explicit assertions. This path owns neither persistent local cache storage nor a guarantee that the remote provider will honor markers.

### 5. Interfaces

StreamOptions.cacheRetention none|short|long, sessionId, promptCacheKey and env; OpenAICompletionsCompat.cacheControlFormat='anthropic', sendSessionAffinityHeaders, sessionAffinityFormat openai|openai-nosession|openrouter, supportsLongCacheRetention. Wire keys prompt_cache_key, prompt_cache_retention, cache_control; headers session_id, x-client-request-id, x-session-affinity, x-session-id.

### 6. Dependencies

`AI-016`, `AI-025`

AI-025 supplies the compatibility/request assembly; AI-016 supplies scoped environment lookup. AUTH-033 carries the corresponding model configuration. External providers decide cache eligibility and billing; a marker is not proof of a cache hit.

### 7. ice-rust today

The complete native Chat Completions request builder and client path (rust:crates/ice-ai/src/providers/openai_completions.rs:134-259 and rust:crates/ice-ai/src/providers/openai_completions.rs:261-453) never read cache_retention or session_id and do not generate cache/affinity fields. Arbitrary static model sampling_params can inject payload fields, and headers can be supplied, but that is not per-request identity, retention disabling, scoped env fallback or marker placement. Native usage parsing already separates cache_read/cache_write at rust:crates/ice-ai/src/providers/openai_completions.rs:1023-1058; accounting support does not supply request cache policy.

### 8. Port notes

Add cache policy resolution after compatibility selection, keeping shared prompt identity distinct from per-session affinity. Implement the three header formats and explicit override ordering, then marker placement on the already converted message/tool arrays. Reapply no-cache policy only if that is the chosen public contract; the TS path intentionally permits later explicit payload/header overrides. Future checks should cover all retention/identity combinations, 64-character clipping, proxy long-retention support, final tool output and unmarkable image-only content.

### 9. Size and priority

Estimated Rust implementation: **180-310 lines** (estimate, not measured source). **P1** - Missing request cache and affinity controls change latency, billed usage and fork/session locality on supported gateways.

## AI-051 - Chat Completions history repair, reasoning replay and Kimi deferred tool placement

### 1. Title and status

Chat Completions history repair, reasoning replay and Kimi deferred tool placement. **Status: partial.**

### 2. ice source

ice:packages/ai/src/api/openai-completions.ts:77-109 (history and deferred names), ice:packages/ai/src/api/openai-completions.ts:1052-1341 (convertMessages); ice:packages/ai/test/openai-completions-reasoning-details.test.ts:92-117, ice:packages/ai/test/openai-completions-thinking-as-text.test.ts:107-221, ice:packages/ai/test/openai-completions-tool-result-images.test.ts:75-156.

### 3. What it does

A transcript is normalized for the target provider before Chat Completions serialization, including stable tool IDs, reasoning signatures and deferred tool definitions. Native serialization covers basic messages and tool images but does not reproduce the full replay contract.

### 4. How it works

convertMessages first calls the shared cross-provider transform with a Chat Completions-specific tool-ID normalizer. It then serializes the transformed conversation while preserving consecutive tool-result grouping and provider-specific assistant requirements.

A pipe-separated Responses ID combines sanitized call and item parts to retain item uniqueness. If the combined ID exceeds 40 characters, a bounded call prefix plus eight characters of shortHash(original) is used; ordinary OpenAI IDs are clipped at 40 while other providers' non-pipe IDs are retained. The shared transform repairs missing results and drops unusable aborted/error turns as described in AI-014. System instructions use developer only for reasoning models with support. Empty user block arrays are skipped. Assistant text drops whitespace-only blocks and concatenates remaining text without a separator. Empty assistant records lacking tool calls are dropped (ice:packages/ai/src/api/openai-completions.ts:1052-1248).

For requiresThinkingAsText, nonempty thinking blocks are joined with two newlines into one text part placed before assistant text parts, without tags. Otherwise a first thinking signature names the replay field; OpenCode Go normalizes reasoning to reasoning_content. Text stays a string, and signed reasoning joins with one newline. Encrypted tool thought signatures are JSON-parsed into reasoning_details; malformed signatures are omitted. requiresReasoningContentOnAssistantMessages fills an absent reasoning_content with an empty string only for reasoning models. This field name is part of the TS compatibility contract, not the shorter native config spelling.

Tool calls serialize as function or grammar-custom calls, preserving the normalized ID and JSON/raw input. Consecutive results produce tool messages in order; text joins with newlines, empty text becomes '(see attached image)' if images exist or '(no tool output)' otherwise. Optional tool result name is included. Vision-capable models receive the collected images in one following user message headed 'Attached image(s) from tool result:'. Providers requiring an assistant bridge receive it before that image message or before a following ordinary user message. lastRole becomes user after the image batch, preventing an additional bridge for the next user (ice:packages/ai/src/api/openai-completions.ts:1249-1338).

In deferredToolsMode='kimi', every historical addedToolNames value removes that name from top-level active tools. After each consecutive result group, matching definitions are emitted in a content-less system message with a tools array; the group uses first-seen name ordering and last definition by name. Missing names are ignored. Definitions are not globally deduplicated across separate result groups. This preserves the TS discovery protocol without claiming provider-native tool search.

### 5. Interfaces

convertMessages(model, context, resolvedCompat,{grammarToolInputProperties?}); ToolResultMessage.addedToolNames, toolCallId/toolName/content; ThinkingContent.thinkingSignature; ToolCall.thoughtSignature. Wire assistant reasoning fields, reasoning_details, tool_calls/custom input, tool/name/tool_call_id and Kimi system.tools.

### 6. Dependencies

`AI-011`, `AI-013`, `AI-014`, `AI-025`

AI-014 owns shared transcript repair; AI-025 supplies resolved compatibility; AI-013 defines custom tool shapes; AI-011 supplies streamed argument repair. Tool discovery producers are covered by the tools/extension areas; this entry only projects their already supplied addedToolNames.

### 7. ice-rust today

rust:crates/ice-ai/src/providers/openai_completions.rs:261-403 iterates ctx.messages directly and does not call the shared Anthropic transform. It retains raw call IDs and error/aborted assistant records when they contain content, has no missing-result synthesis or Kimi system.tools placement, and never serializes ToolCall.thought_signature. It supports requires_thinking_as_text but preserves block order and checks is_empty rather than trim; ordinary text joins with newlines, unlike TS concatenation. It can explicitly replay all thinking into reasoning_content, including an empty value, but does not preserve the original signature-selected field. Basic result placeholders, names, image batching and assistant bridges are already present at rust:crates/ice-ai/src/providers/openai_completions.rs:364-495 with inline coverage at rust:crates/ice-ai/src/providers/openai_completions.rs:1454-1506. After flushing an image batch before a user message, last_was_tool_result remains true, so a required assistant bridge is inserted again after the image user message; TS explicitly changes lastRole to user. That difference follows directly from the builder's state transitions and was not executed.

### 8. Port notes

Route native Chat Completions through a target-specific transcript preparation stage that preserves existing image support. Add stable pipe-ID normalization, signature-aware reasoning/details replay, missing-result repair and Kimi deferred placement, and carry the last emitted role across image flushes. Specify whether exact TS text joining and whitespace filtering are required; do not label native ordered image/text behavior as absent. Future verification should cover mixed same/foreign model history, multiple calls sharing a call_id, malformed signatures, missing results, deferred names and an image-result batch followed by a user message.

### 9. Size and priority

Estimated Rust implementation: **330-550 lines** (estimate, not measured source). **P1** - Replayed tool and reasoning histories can become invalid or lose provider-required state on subsequent turns.

## AI-052 - Chat Completions independent stream blocks, stable tool identities and diagnostic metadata

### 1. Title and status

Chat Completions independent stream blocks, stable tool identities and diagnostic metadata. **Status: partial.**

### 2. ice source

ice:packages/ai/src/api/openai-completions.ts:270-623 (stream block state, usage, terminal cleanup); ice:packages/ai/test/openai-completions-tool-choice.test.ts:509-1035 (EOF, mixed channels and changing IDs), ice:packages/ai/test/openai-completions-response-model.test.ts:56-140, ice:packages/ai/test/openai-completions-reasoning-details.test.ts:92-117.

### 3. What it does

The stream parser maintains independent text, reasoning and tool-call state while exposing stable event indices and replayable metadata. Rust's mapper has the core event lifecycle but differs on indexless calls, repeated IDs, reasoning field fallback and metadata timing.

### 4. How it works

The TS adapter emits start only after the HTTP request and response hook succeed. It keeps one text block, one thinking block and tool blocks indexed both by provider index and by ID, then finalizes every block in content order.

Each valid object chunk may supply responseId (first truthy value), a differing nonempty responseModel (first such value), usage and the first choice. Chunk usage wins; choice.usage is the fallback. Finish reasons retain rawStopReason; stop/end, length and function_call/tool_calls map to neutral reasons, while content_filter/network_error/unknown strings become explicit errors. Missing finish_reason is an error by default; supportsFinishReason=false permits inference from whether tools appeared. Signal abort is checked after stream consumption, before success (ice:packages/ai/src/api/openai-completions.ts:453-601).

Reasoning picks the first nonempty string among reasoning_content, reasoning and reasoning_text, recording its field as thinkingSignature, with the OpenCode Go rename. Tool identity lookup prefers numeric index, then ID; the first nonempty block ID/name remains stable even when a provider changes IDs at that index. Indexless calls with distinct IDs remain distinct. Function argument fragments are repaired incrementally; grammar custom input is wrapped through AI-013. Encrypted reasoning_details may precede their call and are queued by ID, then attached as thoughtSignature. Invalid/non-encrypted details are ignored.

At EOF, text/thinking end events and final parsed tool calls are emitted; partialArgs/customInput/streamIndex scratch state is deleted in both success and error paths. Normalization preserves partial content, sets error versus aborted from the signal and appends OpenRouter raw metadata only when absent from the formatted error. Usage separates read/write cache from uncached input, counts reasoning inside completion output and calculates cost when usage arrives. Specific redacted assignment lines were excluded from coverage; the visible arithmetic and full usage assertions at ice:packages/ai/test/openai-completions-tool-choice.test.ts:1481-1600 establish the counted fields.

### 5. Interfaces

AssistantMessageEvent start, text_start/delta/end, thinking_start/delta/end, toolcall_start/delta/end, done/error; stable contentIndex. AssistantMessage.responseId, responseModel, rawStopReason, usage.reasoning and cache fields. Streaming scratch fields are internal and must not persist.

### 6. Dependencies

`AI-010`, `AI-011`, `AI-013`, `AI-025`

AI-025 owns request/auth/transport setup; AI-011/013 handle argument fragments; AI-010 normalizes failures. Native SSE parsing is shared with AI-023. No new persistence subsystem is required.

### 7. ice-rust today

The native StreamState at rust:crates/ice-ai/src/providers/openai_completions.rs:644-1021 already preserves text/thinking/tool events, raw finish errors, missing-finish compatibility, usage fallback and terminal batch flushing. However, tools are keyed only by index with missing index forced to zero (rust:crates/ice-ai/src/providers/openai_completions.rs:857-923), so multiple indexless calls merge; ID and name fragments are appended, so a repeated or changed complete ID/name corrupts them relative to the TS first-value contract. Native reasoning uses Option.or_else before checking string/nonempty (rust:crates/ice-ai/src/providers/openai_completions.rs:774-782): an empty/null first field suppresses a later valid reasoning field. thinking_signature is always None and reasoning_details/custom-tool deltas are absent. response ID/model are overwritten on every string, including an empty or echoed model, unlike TS first differing metadata. Native cost is computed only at successful finish, so an error with accumulated usage retains default cost. It accepts either [DONE] or EOF after a finish reason, which is compatible with the normal Chat Completions terminal contract. Native batching is useful, but push_text does not flush pending thinking while push_thinking flushes pending text; a thinking-to-text transition can deliver the later text event before the earlier pending thinking (rust:crates/ice-ai/src/providers/openai_completions.rs:695-732 and rust:crates/ice-ai/src/providers/openai_completions.rs:801-855). The existing channel-switch test covers only text-to-thinking.

### 8. Port notes

Keep batching but flush the previous channel before every channel transition and on terminal/error boundaries. Add ID fallback indexing, stable first ID/name semantics, nonempty reasoning field selection and signature/details storage; support custom input using the common constrained helper. Calculate partial usage cost when metadata is accepted or before either terminal outcome. Future checks should replay the mixed four-tool fixture, mutable IDs, empty-primary reasoning field, same-model response metadata and both channel-switch directions; these source-derived defects were not reproduced by execution.

### 9. Size and priority

Estimated Rust implementation: **250-440 lines** (estimate, not measured source). **P1** - Malformed tool identities or merged parallel calls can prevent correct execution; dropped reasoning and reordered deltas also affect replay and UI consumers.

## AI-053 - Responses stream finalization, incomplete reasons and per-output delta fidelity

### 1. Title and status

Responses stream finalization, incomplete reasons and per-output delta fidelity. **Status: partial.**

### 2. ice source

ice:packages/ai/src/api/openai-responses-shared.ts:409-740 (indexed stream processing), ice:packages/ai/src/api/openai-responses-shared.ts:742-772 (stop mapping); ice:packages/ai/test/openai-responses-terminal-event.test.ts:206-355 and ice:packages/ai/test/openai-responses-partial-json-cleanup.test.ts:67-106.

Exported symbols: `OpenAIResponsesStreamOptions` (ice:packages/ai/src/api/openai-responses-shared.ts:105).

### 3. What it does

Responses events must end at a real terminal response and retain the provider's specific incomplete reason. Native handling already reconstructs many gateway variants, but terminal semantics and batched delta delivery differ materially from the current TS implementation.

### 4. How it works

The shared TS processor maps output_index to independent thinking, text or tool slots. It updates the neutral assistant message and emits matching indexed events, while the outer adapter owns start, final done/error and post-stream cancellation checks.

output_item.added creates slots; text/refusal and reasoning summary/raw deltas append only to matching existing slots. A reasoning-summary part end appends two newlines. Function argument deltas use partial JSON repair, and an arguments.done event replaces the accumulated value while emitting only an append-only suffix. Custom tool input uses the monotonic raw-input wrapper; protocol violations throw through AI-013. output_item.done can create a missing slot, finalizes its contents/signature, deletes scratch fields and removes the slot. Message signatures encode v1 id and optional phase; final_answer provisionally marks stop but never substitutes for a terminal response. Reasoning prefers final summary, then content, then accumulated text, retaining the complete item as its signature. A terminal response can backfill encrypted_content into an already finalized reasoning signature (ice:packages/ai/src/api/openai-responses-shared.ts:447-531 and ice:packages/ai/src/api/openai-responses-shared.ts:579-719).

response.completed/incomplete marks the terminal boundary, updates response ID and usage, computes cost and optional tier pricing, and maps status/reason. Only incomplete.max_output_tokens becomes length; other explicit reasons become errors with 'Response incomplete: <reason>', and missing incomplete reason is also an error. rawStopReason includes status.reason when present. completed with tool calls becomes toolUse. response.failed records raw status and throws a provider code/message or incomplete diagnostic; an error event also throws. EOF without any terminal response event is rejected even after a final-answer text item. The TS processor continues consuming after terminal metadata until iterable EOF, and the outer adapter performs the final abort check (ice:packages/ai/src/api/openai-responses-shared.ts:533-577 and ice:packages/ai/src/api/openai-responses-shared.ts:720-772).

The default TS mapper has no sequence-number deduplication or terminal-output reconstruction of arbitrary missing text/tool items; it only backfills reasoning encryption. Those native additions are credited below rather than counted as gaps.

### 5. Interfaces

processResponsesStream(asyncIterable, output, eventStream, model,{serviceTier?, grammarToolInputProperties?, resolveServiceTier?, applyServiceTierPricing?}); response.created/output_item.added/done, output_text/refusal/reasoning deltas, function/custom input delta/done, completed/incomplete/failed/error. rawStopReason distinguishes incomplete.max_output_tokens from incomplete.content_filter and other causes.

`OpenAIResponsesStreamOptions` is `{ serviceTier?: ResponseCreateParamsStreaming['service_tier']; grammarToolInputProperties?: ReadonlyMap<string,string>; resolveServiceTier?: (responseServiceTier, requestServiceTier) => ResponseCreateParamsStreaming['service_tier'] | undefined; applyServiceTierPricing?: (usage: Usage, serviceTier: ResponseCreateParamsStreaming['service_tier'] | undefined) => void }`. Both resolver inputs have the same service-tier type or undefined. These callbacks belong to stream usage finalization (ice:packages/ai/src/api/openai-responses-shared.ts:105-116).

### 6. Dependencies

`AI-011`, `AI-013`, `AI-026`

AI-026 owns outer request/start/terminal hooks and tier pricing; AI-011 parses function arguments; AI-013 handles custom input. Native SSE reader behavior is covered by AI-023. Consumers require stable contentIndex and complete terminal/error delivery.

### 7. ice-rust today

Native ResponsesStreamState is substantial: rust:crates/ice-ai/src/providers/openai_responses.rs:126-949 reconstructs missing added events, sparse/indexless slots, terminal output, signatures and one-time end events. It suppresses exact sequence replays only after seeing distinct numbers and disables deduplication on collisions. Those are deliberate compatibility additions. However, native finish discards incomplete_reason and maps every incomplete to Length with raw_stop_reason='incomplete' (rust:crates/ice-ai/src/providers/openai_responses.rs:2303-2351). Its comment claims TS equivalence but conflicts with the current TS function and native test incomplete_non_token_reason_maps_to_length at rust:crates/ice-ai/src/providers/openai_responses.rs:3551-3565. A bare [DONE] is accepted as successful completion without any terminal response (rust:crates/ice-ai/src/providers/openai_responses.rs:1206-1219), while TS rejects it; preserve this only as an explicit gateway compatibility choice.

Native batching has concrete delivery risks: append_text replaces pending_text_index/pending_thinking_index while appending to a shared buffer, without flushing when the output index changes. Interleaved same-kind slots can therefore emit combined bytes under the latest slot index, although final message contents remain separate (rust:crates/ice-ai/src/providers/openai_responses.rs:588-625). Text-to-thinking flush exists but thinking-to-text does not; tool delta emission also bypasses pending text/thinking flush (rust:crates/ice-ai/src/providers/openai_responses.rs:627-653). run_stream's error/failed/EOF/transport-error branches call emit_error without flushing state, so a short pending tail may be present in the terminal message but absent from delta events (rust:crates/ice-ai/src/providers/openai_responses.rs:1449-1479 and rust:crates/ice-ai/src/providers/openai_responses.rs:2352-2372). Native successful finalization returns immediately on the terminal frame, without TS's later EOF/abort check. These are source-level findings; no repro ran.

### 8. Port notes

Port the exact incomplete reason table and raw reason preservation, correcting the misleading parity comment and intentionally divergent native assertion. Keep gateway reconstruction and cautious sequence deduplication, but give every output slot its own pending buffer or flush before index/channel changes. Flush pending data on all terminal and error paths before emitting the final message. Gate bare [DONE] acceptance behind documented compatibility, and define cancellation precedence at a buffered terminal event. Future tests should compare per-index concatenated deltas with final blocks across interleaving, transport errors and content-filtered incomplete outcomes.

### 9. Size and priority

Estimated Rust implementation: **230-400 lines** (estimate, not measured source). **P1** - Content-filter errors currently masquerade as output limits, and batched stream consumers can receive missing, reordered or misattributed deltas.

## AI-054 - Responses transcript identities, reasoning signatures and deferred tool-search replay

### 1. Title and status

Responses transcript identities, reasoning signatures and deferred tool-search replay. **Status: partial.**

### 2. ice source

ice:packages/ai/src/api/openai-responses-shared.ts:48-103 (signatures and image outputs), ice:packages/ai/src/api/openai-responses-shared.ts:118-379 (message/tool conversion); ice:packages/ai/test/openai-responses-foreign-toolcall-id.test.ts:19-65, ice:packages/ai/test/openai-responses-message-id.test.ts:16-47, ice:packages/ai/test/openai-responses-reasoning-replay-e2e.test.ts:16-291.

Exported symbols: `ConvertResponsesMessagesOptions`, `ConvertResponsesToolsOptions` (ice:packages/ai/src/api/openai-responses-shared.ts:118; ice:packages/ai/src/api/openai-responses-shared.ts:125).

### 3. What it does

Responses replay carries both call IDs and output-item IDs, plus model-bound reasoning signatures and deferred tool-search history. Native code implements a robust replay pipeline but has identifiable identity, ordering and discovery differences.

### 4. How it works

The TS converter first normalizes the transcript through the shared transform, supplying a Responses-specific tool-ID callback. The resulting input contains completed assistant message items, function/custom calls and corresponding outputs rather than a plain Chat Completions transcript.

Allowed target providers openai/openai-codex/opencode preserve pipe-separated call/item pairs; other targets sanitize the whole ID. Each part is sanitized to ASCII alphanumeric/underscore/hyphen, clipped to 64 characters and stripped of trailing underscores. Foreign provider/API item IDs become fc_<shortHash(rawItem)>; ordinary paired items are normalized and prefixed fc_ when needed. Same-provider different-model fc_ item IDs are omitted to avoid reasoning pairing validation. Function replay drops non-fc item IDs, while custom grammar calls can carry their custom item ID. Tool-result IDs use the call part only. The foreign Copilot fixture includes a long opaque item part without fc_, which is explicitly hashed (ice:packages/ai/src/api/openai-responses-shared.ts:145-170 and ice:packages/ai/src/api/openai-responses-shared.ts:246-303).

Assistant thinking with a retained signature is JSON-parsed and replayed as a reasoning item; malformed JSON throws into the outer adapter's error handling. Text signatures accept v1 JSON or a legacy raw string, preserving only recognized commentary/final_answer phase. Fallback IDs use msg_ice_<messageIndex> plus a text-block suffix, and long IDs become msg_<shortHash>. The message index counts transformed messages, not just assistant records. Each text block becomes a completed output_text message; signed empty reasoning and foreign readable thinking behavior comes from AI-014.

Tool outputs are strings when there are no supported images, with explicit empty/image placeholders; for vision, all text is joined first, followed by image blocks with detail:auto. This TS helper groups text before images and therefore does not retain arbitrary text-image-text interleaving. Native ordered output below is an intentional improvement. For deferred tools, each addedToolNames entry found in the deferred map is loaded once across the request. A deterministic ice_tool_load_<hash> ID links a completed client tool_search_call (query/limit) and tool_search_output with defer_loading definitions. Unknown names and repeats are skipped. Function tools use original schema plus strict default false when supported; grammar tools use custom grammar shape. This is serialization of discovered definitions, not execution of an external search.

### 5. Interfaces

convertResponsesMessages(model, context, allowedToolCallProviders,{includeSystemPrompt?, grammarToolInputProperties?, deferredTools?, toolOptions?}); convertResponsesTools(tools,{strict?, supportsStrictMode?, supportsOpenAIGrammarTools?, deferLoading?}). TextSignatureV1 {v:1, id, phase?}; call_id|item_id neutral tool IDs; tool_search_call/output with execution='client' and defer_loading=true.

### 6. Dependencies

`AI-013`, `AI-014`, `AI-026`

AI-014 owns cross-model transcript preparation; AI-013 supplies constrained grammar/schema semantics; AI-026 selects immediate/deferred tools and invokes conversion. External protocol identifiers remain protocol data rather than product branding.

### 7. ice-rust today

Native build_request/ToolCallIndex at rust:crates/ice-ai/src/providers/openai_responses.rs:1484-1798 already repairs missing results, indexes legacy aliases, omits aborted/error turns, deduplicates calls/results and replays model-matched reasoning. It validates parsed reasoning as an object with type='reasoning' and silently skips invalid signatures instead of failing the request. It preserves vision text/image order at rust:crates/ice-ai/src/providers/openai_responses.rs:2011-2092, which is stronger than TS's grouping. Native split_responses_tool_call_id only recognizes call_*|fc_* or call_*|ctc_* (rust:crates/ice-ai/src/providers/openai_responses.rs:1068-1085), so the TS Copilot opaque item fixture is treated as one sanitized call ID rather than a hashed foreign item. Native normalizer supplies *_unknown for empty IDs; TS can return an empty normalized part. Fallback message IDs use msg_pi_ and count assistant records; message length uses UTF-8 bytes rather than TS UTF-16 length (rust:crates/ice-ai/src/providers/openai_responses.rs:1604-1656 and rust:crates/ice-ai/src/providers/openai_responses.rs:1800-1878). Native UTF-16 short_hash arithmetic is deliberately compatible at rust:crates/ice-ai/src/providers/openai_responses.rs:1980-2009. No added_tool_names/tool_search_call/output/defer_loading projection appears in the complete native request builder. Strict schema transformation is already present and should not be replaced with the weaker raw-schema assumption.

### 8. Port notes

Add deferred-tool search replay to the existing prepared-history pipeline. Define recognized foreign paired-ID formats without losing the native protection for arbitrary opaque pipe-containing IDs; preserve stable call/result aliases. Document or align fallback IDs/index counting and length units where cross-client fixture compatibility matters. Keep native duplicate-history filtering, validated reasoning items and ordered image outputs as explicit design choices. Future checks should use the real opaque Copilot fixture, multiple text blocks, model switches, malformed signatures, duplicate aliases, deferred names and text-image-text results.

### 9. Size and priority

Estimated Rust implementation: **220-380 lines** (estimate, not measured source). **P1** - Missing deferred definitions and incompatible replay IDs can make later provider turns reject otherwise valid tool history.

## AI-055 - Codex WebSocket pooling, continuation deltas and controlled SSE fallback

### 1. Title and status

Codex WebSocket pooling, continuation deltas and controlled SSE fallback. **Status: missing.**

### 2. ice source

ice:packages/ai/src/api/openai-codex-responses.ts:304-396 (transport policy), ice:packages/ai/src/api/openai-codex-responses.ts:853-1236 (pool and lifecycle), ice:packages/ai/src/api/openai-codex-responses.ts:1295-1567 (event reader and continuation); ice:packages/ai/test/openai-codex-stream.test.ts:1227-2531; ice:packages/ai/test/codex-websocket-cached-probe.ts:1-300.

Exported symbols: `OpenAICodexWebSocketDebugStats` (ice:packages/ai/src/api/openai-codex-responses.ts:881).
Exported symbols: `SessionResourceCleanup` (ice:packages/ai/src/session-resources.ts:1).

### 3. What it does

Reuse account-scoped Codex WebSockets, send exact continuation deltas and fall back to SSE only at the documented source boundary.

### 4. How it works

TS tries a WebSocket for every transport value except sse, unless the session has a sticky SSE fallback marker. Connection reuse and request-history reuse are separate: a cached session can reuse its socket, while continuation deltas are enabled only when options.transport is explicitly auto or websocket-cached.

The outer default chooses 'auto' locally, but does not write that value into options. Consequently an omitted transport attempts WebSocket and may reuse a session socket yet does not activate the inner cached-context condition at lines 1506–1510. This is an observable source inconsistency, not an inferred default of continuation caching. The pool is Map<rawSessionId, Map<accountId, connection>>. Idle TTL is five minutes and maximum connection age 55 minutes, checked when acquiring. A busy connection causes a new uncached one-shot socket rather than waiting. Missing session or cacheRetention:none is also one-shot. Successful release retains an eligible pooled socket and refreshes idle expiry; failure/abort closes it. Account isolation and sibling-session isolation are explicitly asserted by the tests. Pool identity does not include endpoint, provider, token revision, headers or environment, so a port must decide how changes to these invalidate live authentication/routing state.

Continuation stores the full previous request body, response ID and converted response items with synthetic missing-tool outputs removed. It requires JSON.stringify equality of all non-input/non-previous-response fields and an exact serialized prefix consisting of prior request input plus response items. It sends only the suffix with previous_response_id, including a valid empty suffix; mismatch drops continuation and sends full context. store remains false. Plain websocket uses full context. Request creation sends {type:'response.create',...requestBody}; hook-supplied type can therefore override the discriminator. No durable continuation state is written.

Connect timeout defaults 15 seconds; zero disables it. timeoutMs is an optional WebSocket idle timeout while the event queue is empty, distinct from connection timeout. Start is emitted on the first event yielded by the Codex mapper, which can include metadata such as codex.rate_limits, not only visible content. One missing-continuation error retry is allowed, even after that start boundary; failed continuation closes the connection and retries full context. A connection-limit error retries once only before start. API/protocol errors otherwise do not fall back. A transport failure before start adds a diagnostic, marks sticky session fallback and retries via SSE; after start it terminates with error. Explicit websocket modes follow this same fallback policy.

The message reader queues strings/binary frames/Blob conversions, drains queued items before close/error, and requires a terminal response event. Its queue has no local bound; asynchronous Blob conversion is not serialized across arrival callbacks. Closing sessions clears sockets but not debug statistics or sticky fallback; resetDebugStats clears those markers separately. Registered session-resource cleanup closes sockets only. The live probe is source evidence of intended multi-turn operation, not a run performed by this audit.

registerSessionResourceCleanup adds a callback to a Set and returns an unregister closure. cleanupSessionResources invokes every callback synchronously with the optional session ID, collects thrown errors and throws one AggregateError after all callbacks have been attempted. It does not await promises. The Codex registration closes sockets only; it does not reset the separate sticky fallback/debug state.

### 5. Interfaces

transport sse|websocket|websocket-cached|auto; websocketConnectTimeoutMs, timeoutMs, signal, sessionId, promptCacheKey; closeOpenAICodexWebSocketSessions(sessionId?), getOpenAICodexWebSocketDebugStats(sessionId), resetOpenAICodexWebSocketDebugStats(sessionId?); response.create, previous_response_id; OpenAI-Beta responses_websockets=2026-02-06.; registerSessionResourceCleanup(cleanup), cleanupSessionResources(sessionId?).

`SessionResourceCleanup = (sessionId?: string) => void` is the named synchronous callback type; its cleanup failure aggregation is described above (ice:packages/ai/src/session-resources.ts:1-24). `OpenAICodexWebSocketDebugStats` requires requests/connectionsCreated/connectionsReused/cachedContextRequests/storeTrueRequests/fullContextRequests/deltaRequests/lastInputItems/websocketFailures/sseFallbacks; lastDeltaInputItems/lastPreviousResponseId/websocketFallbackActive/lastWebSocketError are optional (ice:packages/ai/src/api/openai-codex-responses.ts:881-896).

### 6. Dependencies

`AI-009`, `AI-016`, `AI-027`, `AI-054`

Uses the Codex request dialect, Responses replay, transport diagnostics and scoped proxy policy. External runtime WebSocket support and any native WebSocket dependency remain implementation choices.

### 7. ice-rust today

The complete rust:crates/ice-ai/src/providers/openai_responses.rs:1-3735 implementation is an HTTP/SSE producer with full-history request generation. Native cancellation, bounded consumer channels and robust Responses replay are real building blocks, but it contains no Codex socket pool, connection-scoped continuation state or fallback policy. Production factories at rust:crates/ice-cli/src/provider/factories.rs:16-145 expose the external provider seam and three HTTP protocols only.

### 8. Port notes

Implement a separate connection manager with explicit keys covering account and effective endpoint/configuration, bounded event queues, serialized decoding and cancellation-safe leases. Preserve full-context recovery, sibling isolation and the start boundary; explicitly decide whether omitted auto should enable deltas and whether cleanup clears fallback state. Avoid replaying requests after visible output except a narrowly documented, safe recovery. Keep statistics non-secret and bounded, and expose deterministic lifecycle hooks.

### 9. Size and priority

Estimated Rust implementation: **620-1050 lines** (estimate, not measured source). **P1** - Missing connection-scoped continuation and fallback prevent parity for the default Codex transport and can corrupt account/session isolation if approximated by one global socket.

## AI-056 - Codex compressed SSE, terminal framing and HTTP retry boundaries

### 1. Title and status

Codex compressed SSE, terminal framing and HTTP retry boundaries. **Status: missing.**

### 2. ice source

ice:packages/ai/src/api/openai-codex-responses.ts:117-210 (retry and timeout), ice:packages/ai/src/api/openai-codex-responses.ts:216-243 (compression), ice:packages/ai/src/api/openai-codex-responses.ts:398-545 (HTTP and errors), ice:packages/ai/src/api/openai-codex-responses.ts:716-847 (event mapping and SSE), ice:packages/ai/src/api/openai-codex-responses.ts:1570-1599; ice:packages/ai/test/codex-sse-eof.test.ts:1-40; ice:packages/ai/test/openai-codex-stream.test.ts:208-490, ice:packages/ai/test/openai-codex-stream.test.ts:2533-2797.

### 3. What it does

Support the Codex HTTP fallback with its compression, response-header timeout, abortable SSE reader and protocol-aware terminal/error handling.

### 4. How it works

TS serializes the final request body once and attempts synchronous zstd compression at level three when the runtime exposes it. Missing compression support or compression failure falls back to plain JSON; small bodies are compressed too when support exists.

The default additional HTTP retry count is zero. Each attempt creates a response-header timeout when timeoutMs>0, combines it with caller cancellation, performs the injected/global fetch and awaits onResponse before examining success. The header timeout is cleaned up once headers arrive; the SSE reader then listens to the original cancellation signal, without a separate body-idle timeout. The retry classifier excludes insufficient-quota 429, then admits selected 429/5xx statuses or transient message patterns. Delay prefers finite retry-after-ms, then numeric/date Retry-After, then shared exponential delay from one second with a 60-second default cap. A positive maxRetryDelayMs rejects a requested delay above the limit; zero disables that particular cap.

The surrounding catch also retries ordinary errors while budget remains, except caller abort, RetryDelayExceededError or a message containing 'usage limit'. Thus the apparent HTTP classifier is not the sole gate: nonretryable HTTP failures or response-hook exceptions can reach this broader retry catch. Preserve this as a source limitation and decide a stricter native policy; do not claim that all source retries are confined to the status classifier. Friendly 429/usage-limit parsing can attach plan/reset information. Once the successful body begins streaming, this HTTP loop does not replay partial output.

The SSE parser uses a streaming TextDecoder, supports CRLF boundaries and multiline data, ignores empty/[DONE] frames, flushes a final unterminated event at EOF and throws a typed protocol error for malformed JSON. Abort cancels the reader; finally removes listeners, cancels and releases its lock. Codex error/response.failed become typed API errors. response.done/completed/incomplete are normalized to the shared completed event and the mapper returns immediately, so an open response body cannot hold a completed request forever. Only recognized status values survive normalization; a missing/unknown status remains undefined, and the shared mapper's default applies. Explicit incomplete max_output_tokens becomes length; other incomplete reasons follow AI-053. Tests cover single-byte chunking and all EOF suffixes, open-body completion, cancellation, retry headers/caps and zstd. None was executed.

### 5. Interfaces

SSE POST /codex/responses; content-encoding:zstd when available; onResponse({status, headers}, model); maxRetries, maxRetryDelayMs, timeoutMs, fetch, signal; CodexApiError/CodexProtocolError; response.done alias and usage-limit metadata.

### 6. Dependencies

`AI-001`, `AI-009`, `AI-010`, `AI-027`, `AI-053`

This transport reuses shared Responses event semantics and retry arithmetic, but its catch policy and compression are adapter-specific. It can be implemented before optional WebSocket pooling.

### 7. ice-rust today

Native generic Responses already decodes SSE and explicitly cancels/waits through its provider stream at rust:crates/ice-ai/src/providers/openai_responses.rs:1153-1482; AI-053 records its terminal and batching deltas rather than claiming SSE is absent. Native HTTP error/retry code exists at rust:crates/ice-ai/src/providers/anthropic/retry.rs:21-197 but production AiError loses retry headers (AI-004). There is no built-in Codex HTTP adapter or this compression/header-timeout/terminal-alias integration in the fully read factory/module inventory at rust:crates/ice-cli/src/provider/factories.rs:16-145 and rust:crates/ice-ai/src/providers/mod.rs:1-47.

### 8. Port notes

Implement Codex SSE atop reusable native framing, with a header timeout separate from body idle/cancellation, terminal-driven completion and optional zstd. Carry response status/headers into policy before converting to a generic error. Narrow retry decisions to deliberate transient failures and never automatically replay after output. Preserve EOF flush, request hooks and error diagnostics without exposing credentials or whole sensitive payloads.

### 9. Size and priority

Estimated Rust implementation: **260-440 lines** (estimate, not measured source). **P1** - SSE is the explicit and fallback Codex transport; incomplete framing, wrong retry boundaries or lost cancellation can hang or duplicate requests.

## AI-057 - OpenRouter image request hooks, retries, cancellation and data-URL validation

### 1. Title and status

OpenRouter image request hooks, retries, cancellation and data-URL validation. **Status: partial.**

### 2. ice source

ice:packages/ai/src/api/openrouter-images.ts:40-163 (request and output), ice:packages/ai/src/api/openrouter-images.ts:165-196 (usage); ice:packages/ai/test/openrouter-images.test.ts:61-140; ice:packages/ai/test/images.test.ts:14-89; ice:packages/ai/test/fetch-option.test.ts:160-177.

Exported symbols: `openrouterImagesApi` (ice:packages/ai/src/api/openrouter-images.lazy.ts:3).

### 3. What it does

Complete the existing native OpenRouter image adapter's request lifecycle and output-format compatibility.

### 4. How it works

TS sends one nonstreaming chat-completions request containing a single user message of text/image parts. Output modalities are image+text when the model advertises text, otherwise image only.

Input text is surrogate-sanitized; images become data:MIME;base64, payload URLs. The API key must be truthy, model/caller headers are merged with caller values last and custom fetch is passed to the SDK. onPayload can replace the whole request body. The SDK receives signal and optional timeout with SDK retries disabled; shared retryProviderRequest applies maxRetries/maxRetryDelayMs/cancellation. After the HTTP request, onResponse is awaited before ID/usage/output extraction. Errors become a structured image result with aborted when the supplied signal is aborted, otherwise error, and use shared provider-error formatting.

Only the first choice is consumed. Nonempty string content becomes text; image_url may be a string or {url}. Only data URLs matching a nonempty MIME, literal ;base64, and nonempty payload are accepted; remote URLs and malformed/non-base64 data URLs are skipped. The regexp does not itself validate the encoded bytes. Missing choices/images may still return stop with empty output; no requirement for at least one generated image is enforced. Usage splits reported prompt cache reads and writes without double-counting, clamps uncached input to zero, reconstructs total tokens and calculates four cost components from the image model's flat rates. Redacted assignment expressions at lines 173–175 remain excluded from direct-read claims; the visible downstream arithmetic and native matching implementation are recorded separately.

The mocked test proves the requested source shape for text+image, signal forwarding and aborted errors. The credential-gated E2E file calls the legacy generateImages without an explicit apiKey; the current direct adapter requires one, so its environment gate alone is not evidence that that credential is supplied. These tests were read, not run.

`openrouterImagesApi()` returns a ProviderImages object whose async generateImages loads the implementation with dynamic import and forwards model/context/options. It does not add lazyStream's event-stream error conversion; image result/error behavior remains that of the promise-returning adapter (ice:packages/ai/src/api/openrouter-images.lazy.ts:3-10).

### 5. Interfaces

ImagesOptions apiKey, headers, fetch, onPayload, onResponse, signal, timeoutMs, maxRetries, maxRetryDelayMs; ImagesContext.input; AssistantImages {api, provider, model, output, responseId?, usage?, stopReason, errorMessage?, timestamp}; POST /chat/completions with stream:false and modalities.

### 6. Dependencies

`AI-004`, `AI-010`, `AI-015`

Reuse the shared HTTP retry/error policy. Image model/auth collection ownership is AI-015; no chat-stream event loop is required for this nonstreaming request.

### 7. ice-rust today

Native rust:crates/ice-ai/src/images.rs:217-335 already performs the request, consumes the first choice, returns text plus data-URL images and reports response ID/usage. Its payload at rust:crates/ice-ai/src/images.rs:338-365 matches the principal TS modality/message shape; usage at rust:crates/ice-ai/src/images.rs:378-423 matches the intended cache split and rates. However native ImagesOptions at rust:crates/ice-ai/src/images.rs:137-145 lacks cancellation, fetch injection and payload/response hooks. max_retries and max_retry_delay_ms are declared but unused by the request path. Some('') is accepted as a key, unlike TS. Native parses JSON before checking HTTP status, so a non-JSON HTTP error loses the status-specific message. parse_data_url at rust:crates/ice-ai/src/images.rs:367-376 accepts data without a ;base64 marker, empty payload and extra metadata as if it were base64; its test only covers one valid URL and a remote URL. This can emit invalid neutral Image content from a malformed provider response. No execution was used to establish these source differences.

### 8. Port notes

Extend native image options with cancellation and hooks, wire declared retry controls into the actual request path, and retain status/body evidence before generic JSON parsing. Match or deliberately tighten data-URL validation; do not label arbitrary data payloads as base64. Preserve existing image output and cache arithmetic, and make credential handoff explicit in future source fixtures.

### 9. Size and priority

Estimated Rust implementation: **150-270 lines** (estimate, not measured source). **P2** - The working native image path lacks cancellation/retry controls and can misclassify malformed response data.

## AI-058 - Reproducible model catalogs, schema manifests and staged generation

### 1. Title and status

Reproducible model catalogs, schema manifests and staged generation. **Status: missing.**

### 2. ice source

ice:packages/ai/scripts/generate-models.ts:35-79; ice:packages/ai/scripts/generate-models.ts:442-961; ice:packages/ai/scripts/generate-models.ts:962-1105; ice:packages/ai/scripts/generate-models.ts:1200-2220; ice:packages/ai/scripts/generate-models.ts:2222-2908; ice:packages/ai/scripts/model-data.ts:61-132; ice:packages/ai/scripts/model-data.ts:133-273; ice:packages/ai/scripts/check-model-data.ts:1-17; ice:packages/ai/scripts/models-dev-reasoning-options.ts:1-32; ice:packages/ai/src/model-catalog.ts:1-28; ice:packages/ai/test/model-data-validation.test.ts:1-165; ice:packages/ai/test/model-catalog-types.test.ts:1-16; ice:packages/ai/test/reasoning-options.test.ts:1-45

### 3. What it does

Rebuild checked-in provider catalog bindings and their ignored JSON values from named upstream feeds using explicit provider policy, then validate the generated data before publication.

### 4. How it works

The TypeScript generator combines remote model feeds with curated provider rules, producing typed provider catalogs and a hash-checked data manifest. Generation separates collection, normalization, validation and publication so a failed validation can restore the prior data and generated bindings.

readGeneratorOptions parses --strict, --data-only, --json-only, --json-output and --pretty; data-only and JSON-only/output constraints are checked before work. Collection awaits models.dev, OpenRouter and Vercel feeds in source order, with an additional NVIDIA model-ID request when needed. A fetch failure throws in strict mode and otherwise logs and contributes an empty collection; this is not a guarantee of complete external data. Filters require supported text/tool behavior as appropriate, apply provider/model exclusions and overrides, and attach endpoint, input modalities, context/output limits, reasoning maps, cache/strict/grammar/tool-search compatibility and cost policies. These are frozen source policies, not a claim that external products or rates remain current. Effort metadata preserves explicitly advertised values and does not infer off support from an effort list. Azure models derive from OpenAI Responses with endpoint/cost-tier/context adjustments; Codex also has curated entries. First provider/model ID occurrence wins, then provider IDs and model IDs are sorted.

Internal values are grouped by API for type derivation; public JSON output stays flat. Full mode writes a .models.ts wrapper per provider using flattenModelCatalog and a shared aggregator. Data-only hydrates the providers already named by existing wrappers and fails if a requested provider cannot be supplied. Manifest schema v3 records generation time, a structural hash and per-file SHA-256 values. Validation checks exact provider files and API/model identities, finite positive contextWindow/maxTokens, supported input modalities, and finite cost components; it does not reject all negative prices. validateGeneratedModelData derives the expected structure from current provider data, so the structural hash is not an independently frozen model allowlist. Errors are accumulated and reported with a bounded list.

A temporary generation directory receives all JSON and the manifest, and validation precedes replacement. Source wrappers and the aggregator have a captured restoration closure; old data is renamed aside, staged data renamed into place, and post-publication validation can restore old data. Caught exceptions restore generated bindings and finally remove staging. This is coordinated rollback for caught errors, not crash-atomic publication of all source and data files. Separate --json-output processing recursively replaces its chosen output directory without this staging transaction; a port should constrain output destinations and publish safely. No generator, validation module or external fetch was executed in this audit.

### 5. Interfaces

generate-models CLI flags; src/providers/*.models.ts; src/providers/data/{provider}.json and .manifest.json; src/models.generated.ts MODELS; ModelCatalog and flattenModelCatalog; createModelDataManifest, validateModelDataDirectory, validateGeneratedModelData; check-model-data validation command.

### 6. Dependencies

`AI-005`, `AI-013`, `AUTH-011`, `AUTH-033`

AUTH-011 owns runtime built-in model availability and AUTH-033 the native compatibility schema consumed by generated values. AI-005 and AI-013 define reasoning and constrained-output metadata consumers. External dependencies are the models.dev/OpenRouter/Vercel/NVIDIA feed schemas and their explicitly versioned snapshots; a port should pin fetched inputs for reproducibility.

### 7. ice-rust today

rust:crates/ice-ai/src/providers/anthropic/models.rs:1-337 is a hand-maintained catalog with inline tests and an explicit generated-catalog TODO. Native OpenAI Responses also exposes a static catalog, and rust:crates/ice-cli/src/provider.rs:859-902 combines those catalogs, package manifests and user models, preserving provider identity. rust:crates/ice-ai/src/model.rs:1-284 supplies native metadata types; rust:crates/ice-cli/src/model_registry.rs:248-290 refreshes runtime user configuration. Those are real consumers and configuration paths, not a source-generation pipeline. The complete ice-ai source inventory, scripts inventory and targeted generate-models/models.dev references expose no native feed importer, schema-v3 manifest, typed-wrapper generation or staged catalog validator.

### 8. Port notes

Create a deterministic catalog import tool with recorded input snapshots, explicit source precedence, provider transformations and a validation-only mode. Emit native data or Rust constants compatible with existing Model and provider manifests, retain user overrides, and add strict validation for duplicate identities, API compatibility, finite limits and cost policy. Publish data and generated bindings together using a recoverable generation marker or single-directory swap. Port the model-data mismatch, missing/extra file, malformed metadata and type-shape fixtures; add interruption recovery checks without making runtime startup depend on network refresh.

### 9. Size and priority

Estimated Rust implementation: **500-900 lines** (estimate, not measured source). **P2** - Broad provider metadata will otherwise drift across limits, pricing and compatibility even after individual transports are implemented; runtime credential and hard output authority work remains earlier.

## Exact supporting contracts

## Evidence and source limits

All 67 assigned AI source rows are reconciled in evidence/proposal-ai.json. The generated image catalog is grouped by its fully read generator and consumer; only its first 35 lines were inspected as values, and no claim is made that every generated model record was read. Model aggregator and complete generator/validation sources were read. The provider/auth worker's model definitions and shared tests are credited by exact artifact and hash; the Bedrock/Google/Vertex/Mistral supplement is likewise attributed.

The 137 TypeScript AI test/helper files have complete available-body review: 127 are fully visible and 10 have explicitly excluded connector-redacted lines. All six native ice-ai integration test files and the relevant inline native tests were read. No source executable, generator, build, test or provider call was run. Credential-gated matrices, intentionally skipped cases and weak payload-only assertions establish test intent, not runtime success. See evidence/qa-ai-test-closure.json for exact paths, hashes, ranges and test limitations.

Connector-redacted expressions are excluded from read coverage and are not reconstructed. Current frozen catalog values and external-product descriptions are source observations, not independent claims about present provider availability or pricing. Native durable assistant frames are recorded as a Rust-only capability because the cited historical TS frame module no longer exists in the reviewed source; current TS JSONL and RPC projections are not that durable frame format.
