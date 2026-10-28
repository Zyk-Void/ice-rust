# Cognee code audit for selective memory and JEV

Audited: 2026-10-27. Target: `<planning worktree>`, branch `feat/cognee-helper`, HEAD `09a728c` at the start of inspection. Scope: read-only code inspection; no model requests, Cognee requests, source changes, or tests were run by this audit lane. The main checkout at `dbbf8d2` has no diff against this target for `crates/ice-cli/src/cognee/` or `builtin_extensions.rs` (verified with `git diff --stat`). The runtime binary used in historical sessions must still be established separately.

## Conclusion

Cognee is already a substantial implementation. It has a real transport, prompt-context hook, dataset scoping, manual search, background capture, durable compaction writes, an improvement scheduler, and observability. The central mismatch is the policy: most sufficiently long prompts synchronously search memory, while most completed exchanges and tool actions are captured without an assessment of whether they will help a later task. A JEV request inserted immediately before every existing search would add a second serial network dependency without fixing dataset fragmentation, poor source selection, repeated context, or unverified consolidation.

The first implementation should make memory selective locally, preserve provenance and measurable outcomes, and make durable writes reliable. JEV can then classify a small number of ambiguous event-driven candidates off the main response path. Automatic recall should require a useful trigger and return only bounded evidence that is absent from the active context. Explicit model-tool and user requests should remain available independent of the automatic gate.

Code facts below are direct evidence. Proposed failure scenarios are labeled where a historical occurrence was not reproduced. Session statistics and claims about the deployed Cognee server belong in the separate observability report.

## Current flow and existing functionality to preserve

| Area | Current behavior | Evidence |
|---|---|---|
| Installation | Runtime considers Cognee installed only if `<agentDir>/ice-cognee/config.json` exists or an `ICERS_COGNEE_*` process variable exists. Installed, enabled, and online must all hold for service use. | `crates/ice-cli/src/cognee/runtime.rs:537-559` |
| Default configuration | Once installed, recall, capture, tool capture, improve, and compaction remembering default on. Recall budget is 10,000 ms, topK 5, recall cap 6,000 UTF-16 units. Checkpoint recall defaults off. | `crates/ice-cli/src/cognee/config.rs:103-123` |
| Prompt admission | Short prompts, an exact repeat of the immediately previous query within 15 seconds, and an open circuit suppress automatic search. Other eligible prompts search the raw newest prompt. | `crates/ice-cli/src/cognee/runtime.rs:54-90,2262-2337` |
| Automatic scopes | Automatic prompt recall explicitly requests session and graph, excluding the raw trace scope. | `crates/ice-cli/src/cognee/runtime.rs:79-81,2322-2337` |
| Recall transport | Uses `/api/v1/recall`, `onlyContext: true`, a one-dataset list, optional session ID, and a bounded transport deadline/cancellation token. | `crates/ice-cli/src/cognee/client.rs:439-483,631-672` |
| Injection | Memory is an untrusted custom message appended after the prompt and persisted before being sent. The system prompt stays stable for provider prefix caching. | `crates/ice-cli/src/cognee/runtime.rs:2422-2447`; `crates/ice-harness/src/agent_harness.rs:4652-4682`; `crates/ice-cli/src/cognee/text.rs:48-85` |
| Capture | The newest prompt is held until AgentEnd, then paired with the final visible assistant message. Capture requests run in background tasks. | `crates/ice-cli/src/cognee/runtime.rs:2281-2289,1992-2027,751-849` |
| Tool capture | At most 24 allowed traces per turn. Successful read/grep/find/ls/glob traces omit returned contents. Other successful outputs are capped at 600 characters; errors keep a bounded diagnostic. Cognee search itself is excluded. Tool inputs are correlated by tool-call ID. | `crates/ice-cli/src/cognee/runtime.rs:59,66-81,1906-1988,2054-2083` |
| Improvement | RunSettled launches a background wait, current-session improve with a five-minute cooldown, and an older-session dream pass. | `crates/ice-cli/src/cognee/runtime.rs:60-62,1031-1120,1169-1196,2030-2050` |
| Compaction | The final native/Blackhole checkpoint is captured if enabled, and optionally queued as permanent `agent_actions` memory. A local checkpoint is injected once on the next prompt, including short prompts. | `crates/ice-cli/src/cognee/runtime.rs:2243-2259,2503-2589,3063-3102` |
| Durable replay | Pending permanent-memory writes have bounded batches; timeout/abort during a queue drain marks the record uncertain and ordinary drains skip uncertain records. | `crates/ice-cli/src/cognee/queue.rs:17-21,50-69,216-227`; `crates/ice-cli/src/cognee/runtime.rs:895-1025,2391-2398` |
| Dream claims | A revision/lease ledger distinguishes sessions and datasets and protects ledger mutations with filesystem locks. | `crates/ice-cli/src/cognee/dream.rs:1-51,134-160` |
| Tools and skills | `cognee_search` is registered and enabled with the tool policy. Three bundled skills already cover search, remembering, and sync/doctor. | `crates/ice-cli/src/cognee/runtime.rs:1402-1455`; `crates/ice-cli/src/cognee/command.rs:23-85,320-350`; `crates/ice-cli/src/builtin_extensions.rs:138-176` |

Do not reimplement these foundations or claim that context-only retrieval, raw-read-output suppression, trace exclusion for automatic recall, or basic tool/skill support are absent.

## Findings

### C1 — High: almost every substantive prompt blocks on a network recall

`HarnessExtension::prompt_context` awaits `recall_for_prompt`, and the harness awaits that hook before configuring the model stream. The query is `event.prompt`; the runtime does not compare against information already present in the active conversation, infer a task boundary, or check whether the task requires history. The default 10-second network budget is therefore on the first-response path for most prompts.

Evidence: `runtime.rs:2262-2337,2426-2447`; `config.rs:103-123`; `agent_harness.rs:4639-4693`.

The four-word test is a length heuristic, not a usefulness rule. **“What did we decide?” is four words and is not rejected by this test.** Better examples of short meaningful requests that are currently skipped are “Previous decision?”, “Same setup?”, and “Continue that migration.” Exact duplication covers only the immediately previous prompt and only for 15 seconds; rewording bypasses it.

Change target: extract an automatic admission policy from `recall_for_prompt`, with cheap local intent/state checks and explicit skip reasons. Avoid a remote JEV query for routine turns. Handle exact requests directly; permit model-driven search when the model discovers a missing dependency. A JEV classification should be reserved for ambiguity after a local candidate trigger, preferably prefetched or run off the response path.

Verification: fake transport tests must prove zero memory/JEV network requests for self-contained turns, routine acknowledgments, already-resolved facts, and repeated tasks with adequate live context. Tests must include short history-dependent prompts and changing task topics. Set a measured local-admission overhead goal separately from optional network work.

### C2 — High: worktrees fragment default project memory

`find_git_root` stops at any `.git` entry, including a worktree `.git` file. The dataset hashes that lexical root, with a basename prefix. Consequently the main checkout and `.worktrees/extra-fx-bug-fix` resolve to distinct default datasets even though they share repository history. The code preserves intentional per-project isolation, but “same project” currently means the lexical checkout root, not the Git repository identity.

Evidence: `config.rs:550-598`; resolution at startup in `runtime.rs:1748-1781`.

Implication: a memory saved in one worktree is not requested by automatic search in another. This is a code-level consequence, not proof that every historical miss was caused by scoping.

Change target: introduce a stable repository identity derived from verified Git common-dir identity, with worktree/branch identity retained as filters. Keep user preferences separate from repository facts. Do not mix all memories into a global dataset as a shortcut. Add a read-only legacy mapping/fallback for prior lexical datasets; make any eventual consolidation explicit and deduplicated, without deleting old records. Branch-specific unresolved work must not be promoted to repository-wide fact merely because worktrees share a common dir.

Verification: fixture repos with main worktree, two linked worktrees, a nested cwd, a symlinked cwd, and a separate repo of the same basename. Assert shared repository identity where intended and branch isolation where needed. Confirm legacy dataset discovery with bounded fan-out and no write migration during first rollout.

### C3 — High: returned hit count is not evidence of relevant or useful memory

The client normalizes server results and takes the first topK. There is no local relevance threshold, novelty check, source-type exclusion for graph contents, contradiction handling, or result-content deduplication across queries. Graph context may contain many nodes inside one response object, so `hits=1` must not be interpreted as exactly one fact. The graph condenser flattens nodes into text and strips explicit session-ID lines. Then `recall_lines` retains only text and score, discarding result metadata before model injection.

Evidence: `client.rs:488-575,673-690`; `runtime.rs:2347-2366,2401-2408`; `text.rs:41-71`.

Change target: keep source IDs, source session, timestamps, project/worktree scope, entry type, and applicable validity constraints in an internal result envelope. Apply deterministic scope/expiry/duplicate checks before selecting compact snippets. Treat backend scores as ranking signals until calibrated against labeled examples, not as universal probabilities. Count retained evidence separately from raw result objects.

Verification: irrelevant, stale, contradicted, duplicate, cross-worktree, and metadata-free fixtures. Report each rejection reason. A successful recall with zero retained evidence should inject nothing and should not look like a successful useful recall.

### C4 — High: broad capture plus session-wide improve can defeat selective remembering

Every completed eligible exchange is saved as QA and allowed tools produce trace entries. Improvements are requested for entire session IDs with no entry/category selection. Excluding `trace` from automatic search scope only excludes direct trace retrieval; it cannot exclude graph facts already derived from traces or broad Q/A promotion. Tool/manual searches still use the client's default `session, trace, graph` scopes because they do not supply an explicit scope.

Evidence: `runtime.rs:751-791,1906-1988,1992-2027,1031-1120,1538-1546,1592-1601`; `client.rs:639-640,800-818`.

Inference boundary: the client sends whole-session improve requests; exactly which entries the deployed server promotes requires server inspection. Historic graph noise could also predate the current trace-output filters. Do not attribute every returned noisy fact to the present capture code without deployment history.

Change target: define one policy owner for all automatic durable-memory paths. In selective mode, a JEV “drop” must not be bypassed by general capture → improve or compaction → remember. Keep episodic capture for a bounded recent-session record only if desired; durable promotion should consume accepted candidates or an isolated curated session/dataset. If the server lacks per-entry promotion, do not pretend that a client label restricts its whole-session improve API. Use the existing permanent remember queue for accepted canonical facts, or isolate curated cache entries and promote only that scope.

Verification: a denied candidate stays absent from permanent memory under every enabled combination of capture, tool capture, improve, and compaction. Include an all-tools-only session to prove that successful commands do not become durable project beliefs.

### C5 — High: accepted improvement is treated as an improved revision without completion evidence

The client documents `improve` as “accepted, not completed,” sets `runInBackground: true`, and regards an HTTP success as success. The runtime then emits `improve/succeeded` and marks the claimed capture revision improved. There is no completion check in this path. The remember endpoint similarly sends `run_in_background=true` and returns after HTTP status success.

Evidence: `client.rs:694-751,800-820`; `runtime.rs:1121-1143`.

Implication: “improve succeeded” is proof of request acceptance, not proof that new facts are graph-searchable. A long-running or failed server-side job may leave a revision marked improved from the client's perspective. A timeout is also ambiguous: it does not prove that work never began.

Change target: represent `queued`, `accepted`, `processing`, `searchable`, `failed`, and `unknown` where the upstream API can support them. Preserve accepted job identity if provided; otherwise show accepted/unknown and verify with bounded later checks rather than inventing a completion state. Coalesce repeated requests for the same revision and use cooldown/backoff. JEV cannot repair this transport/state distinction.

Verification: mock 202 acceptance followed by delayed completion, terminal job failure, unsupported status endpoint, timeout after accepted work, and duplicate requests. Observability must distinguish request latency from time-to-searchability.

### C6 — High: manual permanent writes do not have the queue's uncertainty protection

The rough idea overstates the existing guarantee. `drain_pending` marks a timeout/abort uncertain. However `manual_remember` sends the request first and creates a new default `Pending` record on **any** error, including an ambiguous timeout. That record is eligible for later automatic replay. There is no server idempotency key in the shown multipart request. The pending operation ID combines timestamp and content hash; dataset and node-set are explicitly excluded.

Evidence: `runtime.rs:1636-1698,939-1001,2391-2398`; `queue.rs:99-115,137-155`; `client.rs:694-751`.

Change target before automatic selective writes: use a durable operation record before transmission and one state machine for manual, compaction, and selective writes. Preserve uncertainty across process restarts; include scope/category in a stable semantic key and use server idempotency only if actually supported. Preserve old readable records. A local hash cannot by itself guarantee exactly-once remote writes.

Verification: delayed/aborted success, process crash between server acceptance and local completion, duplicate candidate text across distinct scopes, concurrent processes, and explicit uncertain flush. Existing `uncertain_records_need_an_explicit_flush` covers queued records, not this manual path (`runtime.rs:3106-3133`).

### C7 — Medium: captured-memory retries can change ownership and duplicate writes

`store_entry` snapshots the original session and dataset for its network call. On failure it calls `buffer_warmup`, which reads the **current** state instead of the snapshot. If a session/dataset switch occurred while the call was pending, the failed entry can be buffered under a later identity. Success observations also read the current runtime identity in `observe`, which can misattribute a late completion even though its actual network write used the original identity. `drain_warmup` is not protected by the permanent queue's single-flight guard and retries all failures without an uncertain state. The buffer silently drops new entries after 256 files or an I/O error.

Evidence: `runtime.rs:670-709,752-791,831-889`; `queue.rs:238-268`; `runtime.rs:623-631` only resets `dream_dirty` when changing session identity.

These are code-derived race scenarios, not reproduced historical incidents in this audit. The session lifecycle may constrain some paths, but new background JEV work must not rely on that assumption.

Change target: capture an immutable operation context `{session_id, repository_id, worktree_id, dataset, branch/context_generation, policy_version}` at scheduling time and pass it to buffering, observations, and result delivery. Use generation checks for model-context injection, but do not silently discard already-authorized durable writes belonging to the old session. Expose queue drops/retries. Add a separate bounded single-flight drain for cache retry work.

Verification: controlled fake-transport completion after `/new`, resume, branch navigation, dataset settings changes, and shutdown; assert both storage identity and emitted observation identity. Include two overlapping warmup drains and a full buffer.

### C8 — Medium: memory is appended repeatedly to durable context

The stable-prefix design is correct: dynamic memory is appended as a persisted custom message instead of modifying the system prompt. But every distinct eligible prompt can append the same content again. No retrieved-item ID/content set is checked against the active branch. A compaction checkpoint and recall can each consume `recall_max_chars` and then be joined, so the combined memory section can exceed that named cap. Envelope text is truncated together with the body, which can remove its closing marker at the cap.

Evidence: `runtime.rs:2243-2259,2310-2366`; `agent_harness.rs:4652-4682`; `text.rs:48-85`.

Change target: track evidence already present in the active branch/context generation; inject only changed/new items, with a unified token budget including headers and checkpoint. Reset novelty state appropriately after compaction and branch navigation, when earlier evidence may no longer be present. Keep the system prompt stable and avoid later mutation/removal of persisted messages merely to dedupe. Reserve space for a complete untrusted-data envelope.

Verification: repeated differently worded queries returning the same result, corrected result versions, compaction then re-recall, branch switching, and combined checkpoint-plus-recall limits. Measure context growth in tokens and not only characters.

### C9 — Medium: observability counts operations but cannot establish benefit

Every installed prompt emits `recall/started` before admission. Disabled, auto-recall-off, circuit-open, short, and duplicate cases emit `succeeded` with skip metadata. Real responses emit `meta.hits`. The recorder projects started/terminal pairs into a single memory item and preserves only bounded previews, not a complete chain from retrieved source through answer use to correctness or time saved.

Evidence: `runtime.rs:2275-2319,2347-2362,2426-2447`; `observer.rs:17-59,113-157`; `observe/recorder.rs:970-1029,1089-1108`.

Change target: explicit decision events and distinct counters for eligible turns, local skips, JEV calls, network search requests, raw objects, retained snippets, actual injections, evidence consulted, stale/irrelevant rejection, and outcome. Emit source IDs/hashes and minimal metadata rather than dumping full private transcripts. Suggested memory should not claim it was used just because it was injected.

Verification: exact counting with one real search, one short skip, one disabled skip, one dedupe skip, one timeout, and one search returning no useful snippets. Benefit must be evaluated on held-out annotated cases or paired memory-on/off outcomes; operation count alone is insufficient.

### C10 — Medium: new JEV decisions need richer event contracts and bounded background ownership

`BeforeRunEvent` currently contains only newest prompt, composed system prompt, session ID, and cancellation token. It does not provide a bounded current-task summary, loaded memory IDs, branch generation, or loaded skills. Passing the whole system prompt to JEV would be wasteful and may leak unrelated instructions; deciding from the raw prompt alone repeats the current retrieval weakness.

Evidence: `crates/ice-harness/src/extension.rs:48-64`; prompt hook use in `agent_harness.rs:4639-4683`.

Change target: extend a narrowly scoped host context or add a coordinator-owned bounded state snapshot; include only relevant task/entity IDs, existing context coverage, candidate metadata, and a minimal recent excerpt when necessary. Explicit user actions and local policy are authoritative; a probability is only a routing hint. Inference should support abstention and should never select unavailable tools/skills or execute them by itself.

Shutdown currently awaits all tracked work and may run another improve, with individual service timeouts but no global drain deadline in this function (`runtime.rs:643-655,1869-1901`). Adding helper jobs to this list without cancellation/bounds could extend exit latency. Give new helper work a bounded queue, coalescing, cancellation, session-generation checks, one concurrent operation by default, and a bounded shutdown policy. Do not claim that merely moving inference to a background task eliminates backend contention.

## Proposed implementation boundaries

The exact file split is an implementation choice; the existing `ice-cli::cognee` module is the right starting point. Do not add a separate daemon or rewrite the Cognee client just to host decisions.

1. **Policy/admission module**: pure local `skip`, `use cached evidence`, `request evidence`, `ambiguous candidate`, and `explicit action` outcomes; no I/O. It should accept the effective memory mode, current task/state generation, existing context coverage, and event reason.
2. **Evidence selection module**: source-aware result normalization, scope/expiry checks, dedupe, and final token budget. Extend rather than discard `RecallResult.metadata`.
3. **Coordinator state**: per-context candidate cache, loaded-evidence set, inflight coalescing, immutable operation contexts, deadlines, JEV circuit/call budget. Reuse runtime transport and host lifecycle hooks.
4. **Selective memory candidates**: source excerpts or canonical summaries with provenance, validity, and category. A typed classifier cannot by itself synthesize a trustworthy fact. Start with exact source spans or already-produced concise completion summaries; avoid adding a second free-text generation call after every exchange.
5. **Write state machine**: extend the existing durable queue with common manual/automatic semantics, uncertainty preservation, semantic dedupe, and non-destructive format migration.
6. **Diagnostics**: retain the existing activity UI and add meaningful reasons and counts. Reuse existing skill invocation and MCP search mechanisms; return small suggestions for unloaded capabilities, not instructions to auto-run arbitrary tools.

## Important corrections to the pasted rough idea

| Rough idea | Correction grounded in code |
|---|---|
| Ask TypeSafe whether memory helps before each eligible prompt. | This still invokes another network dependency for most prompts. Make local admission first and JEV event-driven, coalesced, and preferably off the first-response path. |
| Fall back to existing automatic recall on decision timeout. | That can restore the full overhead and search pattern the user wants to avoid. For automatic ambiguous turns, fail to local-only behavior; preserve explicit search and already-ready evidence. |
| “What did we decide?” is skipped for being short. | It has four words and passes. Use genuine two/three-word continuity examples. |
| The durable queue prevents blind replay of uncertain writes. | True for `drain_pending`, not for `manual_remember` or cache warmup. Fix the common write path before broadening auto-save. |
| Improve success means memory is permanently usable. | It means HTTP acceptance of background work. Searchability is unverified. |
| Excluding trace recall solves tool-noise pollution. | It excludes direct automatic trace scope, not graph-derived trace facts. Manual/tool search still requests the default trace scope. |
| Each worktree should automatically recall the same project memory. | Current `$project` identity is lexical and splits worktrees. Introduce an intentional repository identity/migration strategy. |
| A yes/no decision is sufficient to save useful memory. | The payload still needs an attributable fact, scope, validity, dedupe, and conflict/correction semantics. |
| Capturing everything maximizes future usefulness. | The client stores final QA and command traces; these may be volatile, misleading, or already recoverable from the repository. Retain durable decisions/preferences/failed approaches with scope and evidence. |

## Focused verification plan for implementation

No tests were executed during this read-only audit. The code already has injectable `RuntimeOptions` environment, clock, home, transport, and offline flag (`runtime.rs:406-430`), plus `FakeTransport`/`FakeResponse` tests (`runtime.rs:2610-2679`; `client.rs:1018-1055`). Use those seams.

| Gate | Existing starting point | Required new proof |
|---|---|---|
| Fast local admission | `prompt_recall_injects_memory_and_deduplicates`, `low_signal_and_checkpoint_query_rules` | Zero network calls on ordinary prompts; short continuity and explicit requests handled correctly. |
| Optional decisions | Existing decision client in `ice-ai` and CLI selector owned by main plan | Timeout, cancelled, abstaining, invalid result, unavailable model, and policy-version cache miss cannot block a user response. |
| Scope | `config::tests::project_dataset_hashes_the_lexical_root` | Common-repo identity and branch restrictions; legacy mapping; distinct repos do not mix. |
| Selection | `client::tests::recall_posts_the_exact_body_and_caps_text`, graph condenser tests | Keep provenance, remove stale/duplicate/irrelevant snippets, preserve bounded envelope. |
| Writes | `uncertain_records_need_an_explicit_flush`, `commands_toggle_policy_and_remember_with_queue_fallback` | Manual and automatic timeout-after-acceptance remain uncertain; crash recovery; scoped dedupe. |
| Capture/promotion | `tool_traces_and_answers_are_captured_with_dream_revisions` | Selective deny cannot reach permanent memory through a different capture/improve path. |
| Lifecycle | `shutdown_improves_then_unregisters` | Stale helper/capture completion cannot attach to a new session; no unbounded exit wait from new tasks. |
| Context | Harness prompt-context persistence path | Preserve cache-stable message order; no repeated memory growth; safe reinjection after compaction. |
| Measurement | `observe::recorder` tests | Skips, network attempts, accepted jobs, searchable writes, and helpful uses remain distinct. |

Suggested focused offline command after implementation: `cargo test -p ice-cli cognee:: --locked`, plus targeted `ice-harness` prompt-context/lifecycle tests and `ice-ai` decision tests for changed modules. Use temporary `ICERS_CODING_AGENT_DIR`, explicit faux transports, and no real provider credentials. Any separately authorized live model evaluation must use only `cx/gpt-6-luna`; this audit made none.

## Invariants and rollout constraints

- Preserve current manual search access, tool policy, offline behavior, cancellation, and failure-to-no-memory behavior.
- Preserve untrusted-context treatment and the stable system prompt; memory cannot alter instructions, permissions, or task mode.
- Do not silently merge project datasets, overwrite existing memories, or reclassify branch-specific statements as global facts.
- Keep old configuration and queue files readable. Do not silently enable an external JEV provider during a migration.
- Distinguish available catalog entries from loaded skill content and from successful tool execution.
- Session/history state is the source of truth for current context coverage. Memory is advisory; current files and explicit user corrections win over old summaries.
- Retain the narrow credential-redaction limitation honestly. `redact_memory_text` is a heuristic, not a guarantee; the current recall query is raw `event.prompt` while captures are redacted (`text.rs:1-6,25-39`; `runtime.rs:2322-2325`). New external decision payloads should be bounded and scrubbed before dispatch.
- Start in shadow/local-only mode and record real examples before selecting probability thresholds or claiming saved time. Evaluate against a deterministic local gate and model-tool-only baseline, not only the current per-prompt recall baseline.

## Evidence limits

This report establishes current client behavior and concrete design gaps. It does not prove how the deployed Cognee server indexes a particular record, that every suspect race has happened, or that JEV improves recall quality. Those require the historical observability report, deployed-version/config evidence, and a later bounded evaluation. The specific user complaint is consistent with this code's broad search/capture policy, but the observed sessions must determine how much each factor contributed.
