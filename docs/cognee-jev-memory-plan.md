# Selective Cognee memory and event-driven JEV: implementation plan

Date: 2026-10-27. Status when written: **proposed implementation; audit completed, implementation not performed**. This copy is the planning record. Implementation status per task is in [`cognee-jev-memory/completion-evidence-matrix.md`](cognee-jev-memory/completion-evidence-matrix.md); the 25 task IDs below are that matrix's denominator.

Target worktree: `<planning worktree>`.

Inspected branch and revision: `feat/cognee-helper` at `09a728cdc5b98e270b8f01e38d2575ca047d45ff`. The main checkout was on `docs/adding-tools-guide` at `dbbf8d2`. This plan uses the target working tree as the implementation authority. Recheck the diff before starting: other work can continue after this snapshot.

## 1. Decision and intended outcome

**Fix memory selection, source quality, project identity, and write reliability first. Add JEV as an optional judge of a small candidate set at meaningful events. Do not put a JEV request in front of every prompt.**

For an ordinary turn, the target is a local eligibility check and use of evidence already in context or in a resident cache: no Cognee HTTP request, no JEV request, no new memory block if it would duplicate active context. For a request that actually needs earlier work, retrieve scoped evidence or let the main agent call the existing `cognee_search` tool. For an ambiguous shortlist, JEV can help decide what is relevant or worth retaining. Permanent memory should contain attributable decisions, constraints, verified outcomes, and reusable lessons rather than a running account of successful commands.

Fresh remote evidence has a cost. The implementation must choose explicitly whether to prefetch, wait within a deadline for an explicit request, or defer optional evidence. It cannot guarantee both immediately fresh remote recall and zero added latency. Ordinary turns can avoid the extra wait completely.

Success means that the agent carries a relevant earlier decision into a later task, avoids a demonstrably repeated investigation, or finds a useful capability it would otherwise miss, with less total effort and acceptable latency. A nonempty search response, an accepted write, or a visible memory badge is not sufficient evidence of success.

### Supporting reports

- [Code audit](cognee-jev-memory/code-audit.md): exact code paths, ten findings, existing invariants, and corrections to the rough proposal.
- Observability audit (**withheld from this repository**): actual session cohorts, latency, representative failures and relevant retrieval, and evidence limitations. It and its evidence snapshots quote private local session activity, so they stay out of this public repository until a privacy-safe version exists. The aggregate figures in section 2.1 below are the published summary.
- [Capability routing audit](cognee-jev-memory/capability-routing-audit.md): existing JEV client, skill loading, MCP search, and what to reuse.
- [Upstream research](cognee-jev-memory/upstream-research.md): current primary documentation, adopted and rejected patterns, version caveats.
- [Evaluation protocol](cognee-jev-memory/evaluation-protocol.md): chronological cases, baselines, labels, budgets, and release gates.

## 2. What is going wrong, with evidence

The evidence supports an integration-policy problem more strongly than a claim that the user is failing to prompt the agent correctly.

### 2.1 Observed behavior

The database snapshot covers **46 runs and 3,601 events**, from **2026-10-26 03:01:41 UTC to 2026-10-27 05:08:59 UTC**; 40 runs contain memory activity. These are a short, retained window across several projects, not the user's lifetime history. In the client timezone, the window is October 26 06:01:41 to October 27 08:08:59, UTC+03:00.

| Observation | What it establishes | What it does not establish |
|---|---|---|
| 60 actual automatic recall terminal outcomes: 56 responses, 2 timeouts, 2 aborts | Real recall operations occurred; the hook can delay the main model | All were necessary or useful |
| 22 additional short-prompt skips recorded as recall `succeeded` | Raw success counts overstate executed recalls | A skipped query is a successful retrieval |
| Actual recall p50 3,891 ms, p95 5,233 ms, maximum 10,007 ms | Existing first-response overhead is material | These are JEV latencies or future performance guarantees |
| Summed recall latency 203,470 ms across the 60 operations | A measured aggregate of recorded recall waits | Exactly 203 seconds of recoverable wall time or user time saved |
| Target lexical dataset: 21 actual recall outcomes, p50 3,841 ms, p95 5,233 ms | The target worktree cohort has the same practical problem | Every run executed the inspected binary revision |
| 53 successful responses have `hits=1`; 3 have `hits=0` | Most successful responses contained a normalized result object | 53 useful memories; one object can contain many graph facts |
| 75 Q/A capture terminals; 254 trace terminals, including 8 timeouts | The memory input stream includes substantial routine activity | Every accepted entry was durably stored/indexed |
| 18 improvement acceptances, 58 improvement timeouts, 32 started-only items | Consolidation attempts have an operational problem and incomplete outcomes | 18 completed promotions, or 32 confirmed failures |

Examples in the observability report include an underspecified “which one is main” receiving unrelated earlier topics, a build request receiving command-success records, and a commit-count request receiving a count of successful operations. A Skyrim continuation retrieved relevant earlier migration paths; the agent still verified the live state. That is a relevant retrieval candidate, not proof of net time saved. The report preserves episode IDs and sequence numbers.

The current stored Cognee configuration has automatic recall, session capture, tool capture, automatic improve, and compaction remembering enabled, a `$project` dataset, and a 10,000 ms recall budget. This is a file snapshot; effective process environment overrides were not inspected. There is no evidence in this audit of a curated manual permanent-memory habit: the two permanent remember observations were compaction checkpoints.

### 2.2 Code mechanisms explaining those observations

1. **Recall is nearly indiscriminate and synchronous.** The default gate is principally a four-word minimum and an exact-last-prompt duplicate window of 15 seconds. It searches the raw prompt. `prompt_context` and then the harness await it before the model stream is prepared. Evidence: `crates/ice-cli/src/cognee/runtime.rs:54-90,2262-2447`; `crates/ice-harness/src/agent_harness.rs:4639-4693`.
2. **The default project identity fragments worktrees.** `$project` hashes the lexical checkout root, and a linked worktree's `.git` file stops the root walk. The database contains distinct `ice-ice-rust-0235eae4` and `ice-extra-fx-bug-fix-353ce83f` datasets. This makes cross-worktree recall unavailable through the automatic one-dataset request. It does not prove every observed miss was a scope error. Evidence: `cognee/config.rs:550-598`; observability cohort table.
3. **Returned material is not checked for novelty, durable value, or current applicability.** Metadata is dropped before injection; a graph response can flatten multiple facts into one object. Different prompts can append the same text repeatedly. Evidence: `cognee/client.rs:488-575,673-690`; `cognee/runtime.rs:2243-2259,2347-2408`.
4. **Broad capture and whole-session promotion can undo a selective-write decision.** Turning down a new candidate does not help if old capture/improve paths promote the same exchange anyway. Evidence: `cognee/runtime.rs:751-791,1031-1120,1906-2027`; `cognee/client.rs:800-818`.
5. **Acceptance is confused with completion.** Improve returns after HTTP acceptance of a background job, but the runtime marks the revision improved. Searchability is not verified there. Evidence: `cognee/client.rs:800-820`; `cognee/runtime.rs:1121-1143`.
6. **Some write paths are weaker than the rough proposal claims.** Queue drains preserve timeout uncertainty, but manual remember queues any first-call error as `Pending`; warmup capture retries also have ambiguity and ownership risks. Fix these before expanding automatic saving. Evidence: `cognee/runtime.rs:752-889,1636-1698,2391-2398`; `cognee/queue.rs:99-115`.
7. **JEV is not yet connected to this policy.** The typed client and CLI binding exist, but the inspected CLI search finds only the definition of `decision_client`, not a Cognee caller. Adding another transport is unnecessary. Evidence: `crates/ice-cli/src/decisions.rs:42-83`; `crates/ice-ai/src/decision/client.rs:185-304`.

### 2.3 Existing behavior that should remain

Context-only Cognee retrieval is already sent as `onlyContext: true`. Automatic recall already excludes direct trace scope. Successful file-reading traces already omit raw file contents. The existing search tool, three bundled Cognee skills, durable queue, transport cancellation, circuit breakers, redaction, and activity UI should be extended, not replaced.

The stable-prefix design is especially valuable: memory is a persisted hidden message following the user prompt, not a rewritten system prompt. Keep that ordering. Improve novelty and budgeting without mutating earlier persisted messages. Evidence: `cognee/client.rs:645-650`; `cognee/runtime.rs:66-81,1906-1988,2422-2447`; `ice-harness/src/extension.rs:143-148`; `agent_harness.rs:4665-4682`.

## 3. Scope, constraints, and non-goals

This work changes the memory policy and its supporting contracts. It does not replace Cognee, build a new agent harness, expand provider coverage, train a model, install plugins, or rewrite current project documentation into memory.

Use the existing `ice-cli::cognee` module and `ice-ai::decision` transport. Keep MSRV **1.88**, as required by the current `AGENTS.md`; old notes mentioning 1.78 are stale. Tests must use fake transports, faux providers, temporary agent directories, and isolated fixtures. Do not touch the TypeScript installation, its directories, or its environment variables. No commits, pushes, releases, installations, or upstream reviews are part of this plan.

The present task authorizes reading the named observability database and writing these reports to the target worktree. It does not require changing live settings or existing memory. **Any live model testing in this task is restricted to `cx/gpt-6-luna`.** This restriction also covers inference triggered indirectly by a Cognee recall, remember, improve, or evaluation request. Do not assume an HTTP backend check is inference-free: use mocks, recorded fixtures, or a confirmed inference-disabled adapter unless every downstream model meets the restriction. No live model or Cognee service requests were made for this audit. JEV quality and latency remain unmeasured; a real JEV evaluation would require the user to expand the current model authorization.

Memory is advisory. Current user instructions, repository rules, current files, and validated live state take precedence over remembered statements. Remembered tool instructions are untrusted data and do not authorize execution. Honor explicit dataset settings, tool availability, hidden skills, offline mode, cancellation, and current trust decisions.

## 4. Target design

### 4.1 One coordinator with four paths

Add a small coordinator inside the Cognee integration, not a separate agent or daemon. The coordinator owns local admission, cache/state, candidate work, and budgets. Reuse the existing transport, queue, lifecycle hooks, and observations.

| Path | Trigger | Action | JEV | Foreground behavior |
|---|---|---|---|---|
| Ordinary | Self-contained request, acknowledgment, continuation with sufficient active context | Local check; use resident facts only if new and relevant | None | No Cognee/JEV request or network wait |
| Explicit history | User explicitly asks for earlier work; main agent requests missing evidence | Existing scoped recall tool/command, with query expansion from known task entities | Bypass | Normal tool result or clearly bounded explicit wait |
| Task event | Resume, a genuinely new task/entity, confirmed correction, post-compaction gap, verified resolution | Local shortlist; prefetch/candidate work if needed | Only unresolved ambiguity | Cached evidence may be used immediately; optional late results wait for a valid boundary |
| Ambiguous candidates | Relevant finite shortlist survives local filters but selection remains uncertain | Typed JEV decisions, cached by event and revisions | At most one coalesced request per eligible event | Default background; never append into an already-streaming request |

Local checks still execute at prompt admission; “not every prompt” means **no per-prompt model/service work**, not an absence of cheap control flow. Negative decisions are first-class results, with reasons such as `already_in_context`, `self_contained`, `no_candidates`, `same_task`, `budget_exhausted`, `offline`, or `stale_generation`.

An explicit user request is not sent to JEV to ask whether it should be honored. A short message such as “Previous decision?” can trigger retrieval when the task identity is known. An underspecified “continue” with a sufficient active conversation should not. If the referent is genuinely unknowable, do not query all projects or invent a task identity; the main agent can ask a focused question.

**Task events must have a concrete producer.** Keep a stable `task_id` across follow-up prompts. Resume, compaction, repository change and reload events come from host lifecycle hooks. A semantic task change or missing-history signal comes from an explicit user marker or a structured signal/tool call in the main agent's already-needed generation. It must not require an additional classifier on every prompt. A changed prompt hash, changed context digest, or new model request may invalidate a cache entry; none by itself authorizes another prefetch. Unknown task continuity remains unknown and uses current context/on-demand search. Deduplicate work by `(task_id, event_kind, relevant_revision)` separately from the delivery generation. Ten differently worded follow-ups to the same task must not create ten task events.

### 4.2 Separate current context, facts, episodes, and capabilities

| Information | Authority and storage | Retrieval rule |
|---|---|---|
| Current source, test outcomes, branch state | Repository/tools and current session | Inspect current state; memory may point to where/why |
| User constraint or confirmed project decision | Curated fact with source, scope, revision, validity | Recall when applicable and absent from active context |
| Prior task episode or resolved failure | Bounded outcome/lesson with provenance and freshness | Retrieve for the same failure/task; verify volatile details |
| Raw conversation/tool activity | Observability or optional bounded episodic capture | Diagnostic/explicit history only; no unrestricted durable promotion |
| Skill/tool capability | Current resource/MCP catalog | Local discovery first; load exact body/schema only when needed |

This prevents using Cognee to answer “how many commits are there?” when Git is authoritative, and prevents using old chat recall to answer “what skills are installed?” when the current resource catalog is authoritative.

### 4.3 Repository and worktree identity

Introduce `RepositoryIdentity { repository_id, canonical_common_dir, worktree_id, git_ref, git_revision }`. Keep Git identity distinct from the conversation's branch or leaf ID. Resolve Git common-dir identity once at session/repository setup using an existing Git facility or a bounded read-only Git invocation; cache it. Refresh revision/ref only on a relevant repository change or bounded explicit inspection, not through a subprocess on every prompt. Canonicalize symlinks and handle `.git` files, main worktrees, bare repositories, nested paths, and non-Git projects deliberately.

Shared repository facts use the stable repository identity. Work-in-progress plans, unmerged implementation details, and transient file state retain worktree/branch restrictions. Global user preferences require explicit scope; they do not justify a single global dataset for everything. Existing explicit dataset configuration wins.

The initial migration is **read-through aliases, not a bulk merge**. Record known old lexical datasets and their source identities. Query the current canonical scope first; permit at most one mapped legacy scope on a relevant miss, inside the same overall budget. Unattributed legacy material should remain legacy-scoped or be rejected for automatic cross-worktree injection. Do not guess that two same-basename repositories are the same project. Do not delete or rewrite old datasets during rollout.

Keep a local mapping so a later directory move can be reconciled explicitly. Common-dir paths are machine-local identifiers, not universal identities across independent clones. Cross-machine/repo-clone unification is out of scope for this first version.

### 4.4 Minimal contracts and state

Proposed internal contracts, to be adapted to existing types:

```rust
enum MemoryAdmission {
    Skip(SkipReason),
    UseResident(Vec<MemoryId>),
    RecallExplicit(RecallIntent),
    Prefetch(TaskEvent),
    JudgeCandidates(CandidateBatch),
}

struct OperationContext {
    repository_id: String,
    worktree_id: String,
    host_session_id: String,
    task_id: String,
    session_branch_or_leaf_id: String,
    git_ref: Option<String>,
    git_revision: Option<String>,
    context_generation: u64,
    policy_revision: String,
    source_revision: String,
    operation_id: String,
}
```

An evidence item additionally needs a stable ID/content hash, selected text, category, source entry or document ID, source role, source timestamp, repository/worktree scope, validity or expiry, supersession relation, and observed searchability. Keep unknown provenance explicitly unknown. Do not fill absent IDs/timestamps with invented values.

`ContextCoverage` describes evidence IDs/versions actually present in the active branch after compaction, and skills whose full instructions are still available. It is not a claim of perfect semantic entailment. Exact content hashes and known fact keys remove obvious duplication; genuinely ambiguous novelty can be judged only on a small context excerpt.

The current `BeforeRunEvent` has only prompt, system prompt, session ID, and cancellation. Add a narrow host snapshot or coordinator access to existing session state for task identity, session branch/generation, Git applicability, selected source entries, loaded capability IDs, and current memory coverage. Preserve `original_user_intent` before `$skill` or `/skill:name` expansion, with host-authored resource IDs/hashes in separate metadata. Do not recover intent by parsing arbitrary user-supplied XML, and do not use a full expanded skill body as a retrieval query. Do not send the entire system prompt or transcript to JEV. Prefer an additive Rust-only contract over plugin ABI changes. Evidence: `ice-harness/src/extension.rs:48-64`; `ice-cli/src/interactive_tui/run.rs:336-377,459-461`.

For the first version, a bounded private JSON sidecar under the existing Cognee storage is sufficient for curated resident facts and their revisions; reuse atomic private-file helpers. Atomic rename alone does not prevent lost updates across sessions/processes: use a scoped file lock with reread/merge and revision validation, reusing the dream ledger's locking pattern, or immutable per-record files with a coordinated index. Read once at startup/reload and refresh on observed revision changes. Verify two concurrent sessions can update distinct facts without either update being lost. Introduce a new database/index only if measured scale requires it. Keep observability separate from the memory source of truth.

### 4.5 Retrieval and injection

1. Resolve the required fact or task from the current request plus a bounded known task summary. Do not forward greetings, entire pasted plans, or skill bodies as the search query.
2. Consult applicable resident facts and an event-keyed result cache. Check project/branch constraints and validity before a hit is accepted.
3. If retrieval is justified, request bounded context through the current adapter. Keep `onlyContext: true`. Do not assume newer Python API options such as `session_first` exist on the installed HTTP service; capability-check the actual adapter first.
4. Preserve server provenance through normalization. If the server returns rendered graph text, parse known node boundaries into candidates while retaining the original source envelope. Where reliable provenance is unavailable, expose that limitation and lower automatic eligibility rather than inventing metadata.
5. Remove wrong-scope, expired, superseded, duplicate, known-in-context, and routine-success-only candidates. Avoid a universal numeric score threshold: backend scores are not a calibrated probability that a fact is useful.
6. JEV is optional for remaining ambiguity; ordinary retrieval must work without it. On JEV timeout or unavailability, use already-vetted local evidence or abstain. Do not fall back to the old broad automatic search pattern.
7. Inject only the selected new facts, with IDs, source/time, and uncertainty where relevant. Budget checkpoint and recall together, including delimiters and provenance. Preserve the complete untrusted-data envelope.
8. Deliver ready evidence at the next valid **model-request construction boundary within the same task**, including a boundary after a tool result. Reuse the durable context-only delivery queue or add a narrow harness gate that appends context before constructing the next request. The existing first-request `prompt_context` hook is not by itself a complete mid-task delivery mechanism. Do not perform persistence side effects in the synchronous `transform_context` hook, mutate a streaming request, or start an unsolicited assistant continuation just to deliver optional memory. Revalidate task/scope/generation before append. If the task finishes first, record `ready_not_consumed` or `late_after_task_complete`, cancel remaining optional work, and do not count that prefetch as useful. Retain a bounded result for future use only under an explicit cache policy. Required evidence should use direct retrieval before the consequential action instead of relying on a race with background prefetch.

Current context coverage must be recomputed or invalidated after compaction, session/branch switch, `/new`, `/resume`, `/reload`, or relevant source changes. A previously injected ID may need reinjection after its source message leaves context. Do not permanently suppress it because it was once seen.

### 4.6 Selective remembering

Create candidates from meaningful events: an explicit user “remember” request, a confirmed constraint, an explicit correction, an accepted architecture decision, or a verified completed task with a reusable lesson. A successful tool call alone is not a lesson. An assistant proposal is not a user decision. A failed experiment may be valuable only when its relevant conditions and resolution are retained.

Use exact bounded source spans or an already-produced final task summary. JEV classifies/selects supplied candidates; it does not write prose. If a canonical new summary would require another generative request, defer that improvement or reuse the main agent's existing output rather than adding a model call after every exchange.

The JEV-off acceptance path must be useful on its own. An explicit remember request with valid source and scope can enter the durable queue directly. An explicit correction can supersede a known scoped fact when the referent is unambiguous. For a proposed user constraint, the host resolves the source entry and exact quoted span; a model-supplied source ID does not authenticate newly generated prose. A verified outcome must reference the relevant artifact, tool result or test evidence and retain its conditions. Routine success and assistant speculation are not promoted. Ambiguous optional candidates remain pending or are dropped with a reason; begin with at most 32 pending candidates and a 24-hour expiry. Explicit durable intent belongs in the durable outbox and must not expire under this optional-candidate limit.

The main agent needs a way to propose a candidate when deterministic signals are insufficient. First inventory existing MCP write capabilities and how they interact with this policy. If none can supply a provenance-checked local proposal, add a small `cognee_propose_memory` tool that only enqueues a candidate, with source entry IDs/category/scope and optional supersedes ID. Do not equate a model proposal with successful permanent storage, and do not create a second unrestricted remote remember path.

Modes:

| Mode | Capture and promotion |
|---|---|
| `legacy` | Existing behavior, preserved for rollback/compatibility and clearly labeled |
| `selective` | Accepted candidates use the common durable write path; broad session capture is optional episodic history and cannot auto-promote unchecked content |
| `manual` | Explicit user requests or policy-authorized main-agent tool actions only; no automatic recall or promotion |
| `off` | No Cognee or JEV operation |

Implement read and write policy independently before the final UI presets: `readPolicy = legacy | on_demand | event` and `writePolicy = legacy | manual | selective`. The early canary is `event + manual`, with JEV off and local continuity anchors retained. Disable unchecked automatic promotion in that canary while the selective outbox is being built; do not label a partial read-only rollout as the complete selective mode. The `selective` preset can enable both only after the write-policy gate passes. Preserve old settings explicitly for rollback and explain their interactions.

Selective mode must own all automatic durable paths: Q/A capture, tool traces, idle improve, dream, and compaction remember. If the server only supports whole-session improve, either stop automatic promotion of the broad capture session or use a separate curated session/dataset. A client label cannot make an unfiltered server request selective. Compaction anchors remain available for continuity without automatically treating every compressed summary as durable truth.

Use one durable write state machine for manual and automatic writes. Persist the operation before transmission; bind its original scope; move ambiguous timeout/abort results to `uncertain`; do not replay automatically. Preserve old record formats with versioned readers. A semantic dedupe key includes scope, category, fact key/source, and content version. A local hash prevents some duplicate dispatches, not exactly-once remote execution.

Represent `accepted` separately from `searchable`. If the backend exposes a job/status handle, persist and reconcile it. If it does not, record `accepted_unverified` and perform a bounded later verification only for selected facts. Do not launch a probe after every capture. Coalesce work by revision; unknown jobs need reconciliation/backoff rather than repeated unbounded improve requests. The local curated fact may remain usable, with its local source, before the remote graph is ready.

Corrections mark prior facts superseded. Keep provenance for audit, but exclude old versions from automatic injection. Repository decisions last until revised; volatile environment facts get a validity rule and require rechecking. Do not use one arbitrary TTL for everything.

### 4.7 Where JEV helps

Reuse `decisions::decision_client(provider_id)` and the existing `DecisionClient::decide` with an explicit feature deadline and cancellation token. Cache the bound client rather than rebuilding it from configuration per event. Its transport already pools HTTP connections, disables redirects and automatic retries, and validates typed responses. The generic 120-second constant is not a helper latency budget. Evidence: `ice-cli/src/decisions.rs:42-83`; `ice-ai/src/decision/client.rs:18-19,169-304`.

Good uses:

- Does this supplied candidate contain a durable project constraint rather than a temporary status?
- Does this candidate affect the current task and add information absent from the supplied current-context excerpt?
- Does an explicit correction supersede a known fact under the same conditions?
- Which of the supplied, currently available skills supplies a needed capability?

For every candidate in a small shared batch, use independent Noul judgments for applicability/durability and, only where needed, a category Choice. A choice includes a `none` alternative or a separately evaluated applicability condition. The application's exact loaded-ID/scope checks remain deterministic. Independent questions in one request do not consume one another's answers. For example, do not ask whether “the skill selected by question 1” is loaded in question 2 of the same call.

Keep the initial batch at **at most 8 candidates and about 2,000 input tokens** including relevant state and questions; these are engineering starting limits, not vendor limits. Use a finite set of candidate IDs. A returned ID must be one of the supplied IDs. Confidence is a property of a returned distribution, not measured task correctness or permission to act. Calibrate thresholds against held-out cases; unresolved cases abstain.

Use the configured TypeSafe provider and a pinned supported model for future evaluations. Current official docs list `jev-1.14.0`; the configured routing may use another valid alias. Do not confuse the installed `jev-search` web-search skill with the TypeSafe decision client. No model autodiscovery or provider expansion is required in the first implementation.

### 4.8 Tools and unloaded skills

The worktree already has `$skill`/`/skill:name` expansion, name-based completion, a model-visible description catalog, and an MCP gateway with local search/describe/call. Reuse them. A fresh catalog is the right source for capabilities; Cognee may hold a past successful workflow but cannot establish that a tool still exists.

Track four distinct states: discovered, descriptor advertised, full instructions currently in context, and invoked/used. Host memory containing `Skill.content` does not mean the model has read it. After compaction, an old invocation may no longer provide full instructions. Respect `disable_model_invocation` for automatic suggestions while retaining explicit invocation.

At a task start, capability miss, or explicit inventory question, shortlist by current names, descriptions, configured triggers, and availability. If a directly matching tool/skill is obvious, return the hint without JEV. If two or more plausible candidates remain, JEV can rank the shortlist once. Emit at most one small skill suggestion or two tool handles, with a reason and exact load path. Do not paste all skill bodies, reconnect every MCP server, or execute suggested tools automatically.

Fix discovery roots deliberately. Current native skill directories cover `.icers` resources and explicit/package/builtin paths; the historical request to check `~/.agents` is a real missed discovery opportunity. Add a supported opt-in resource source or explicit path mapping through the existing loader, with collision precedence and trust respected. Do not scan all home directories or silently import unrelated app instructions.

Treat errors semantically. The observed `cognee_recall` HTTP 404 and `cognee_search` timeout were displayed as tool `done` items. A completed tool callback is not successful retrieval. Expose an explicit unavailable/error outcome to the agent and telemetry; do not count it as a helpful tool recommendation.

## 5. Initial budgets and failure behavior

These are proposed limits to test and adjust, not measured guarantees.

| Work | Initial budget/policy | Failure behavior |
|---|---|---|
| Ordinary local admission | p95 added CPU/wall overhead ≤10 ms with initialized resident state | Local-only; no remote fallback |
| Automatic prompt-time network wait | 0 ms by default | Use ready evidence or continue |
| Automatic prefetch | One request per admitted task event/relevant revision; 6 s wall deadline initially | Cache miss/unknown, not “nothing remembered” |
| Explicit recall/tool | Reuse 10 s total deadline initially, one bounded attempt | Clear no-result vs timeout/unavailable distinction |
| JEV helper | One in-flight request per session, one per unique event/revision, initially ≤2 requests/minute | Coalesce/delay; no foreground retry |
| Shared process/provider admission | Initially one optional inference request in flight per provider per process; a shared rate/token budget across sessions | Explicit current-task work has priority; expire queued optional work before dispatch |
| JEV deadline | Initially 1.5 s background; no use of 120 s generic default | Abstain/retain pending bounded candidate |
| Background queue | Initially 32 event batches; prioritize corrections and explicit requests | Observable coalescing/drop reason; no silent loss of explicit durable writes |
| Automatic evidence injection | Initially 800 tokens or 1% of available model window, whichever is lower | Select fewer snippets; reserve envelope/provenance space |
| Per-response repeated memory | Only new/revised evidence absent from active branch | No duplicate append |
| Helper shutdown | Cancel optional inference immediately; bounded 1 s drain for new helper work | Persist bounded candidate state; do not wait indefinitely |

The existing recall p95 exceeds 5 seconds, so a 100–300 ms remote-recall deadline alone would mostly suppress results. Use prefetched/resident information for the fast path; do not advertise a tiny timeout as a working recall solution. Likewise, if JEV cannot provide useful decisions within the background budget, keep it out of automatic behavior until evaluation justifies a different trade-off.

Keep retrieval and judgment caches separate. Retrieval keys use scope, normalized query/task identity and a known corpus revision. If remote corpus version is unavailable, use a bounded positive-result age, initially at most five minutes and at most 60 seconds for volatile facts; expiry invalidates reuse but does not itself authorize another service call. Judgment keys use candidate hashes, the relevant context-coverage digest, policy revision, provider protocol and pinned model version. A delivery generation is a safety fence, not a reason to destroy otherwise reusable evidence. If a model alias is used, its resolved version may be unknown until the response; bound its cache lifetime and invalidate on a newly observed version. Prefer pinned models for evaluations.

Local correction, accepted curated writes, known indexing transitions and capability availability changes invalidate the relevant caches. Use a short negative-result TTL for optional automatic lookups; explicit search can bypass a prior no-result decision. Per-session quotas do not bound a multi-session process: enforce the shared provider semaphore/rate budget too. Independent processes still need the existing server's capacity controls; do not claim a process-local limiter caps the entire deployment.

## 6. Implementation stages and atomic work

All checkboxes below are future work. Existing foundations are described above and are not counted as new implementation completion. No percentage is claimed for this proposed plan.

### Stage 0 — Establish truthful diagnostics and adapter contracts

- [ ] **M0.1 — Emit distinct admission, retrieval, injection, and promotion outcomes.**
  - Files: `cognee/observer.rs`, `cognee/runtime.rs::observe`, `observe/recorder.rs`, `interactive_tui/cognee_activity.rs`.
  - Depends on: none. Preserve old event readers; add versioned optional metadata rather than reinterpreting old `succeeded` values.
  - Acceptance: a skip is not a request; accepted improvement is not searchable; a tool error is not useful evidence. Include operation/source/context IDs and actual wait/injected-token counts without full transcript dumps.
  - Verify: recorder fixtures with skip, success-empty, success-retained, timeout, abort, accepted-job, and late completion; reproduce the old snapshot totals using the report's script.
- [ ] **M0.2 — Inventory native and MCP Cognee wire contracts and expose capability/health state.**
  - Files: `cognee/client.rs`, `cognee/runtime.rs::collect_doctor`, `mcp` catalog diagnostics; use the logged 404 case.
  - Depends on: none. Do not change endpoints by guess or print credentials.
  - Acceptance: status distinguishes native `/api/v1/recall`, any MCP wrapper, supported fields, and unsupported status/recall methods. A 404 is not retried with a different random tool.
  - Verify: contract fixtures for existing camel-case recall and remember/improve shapes, 404/422 responses, and unsupported completion status. Any later live probe must satisfy the model restriction for all downstream inference; prefer mocks and confirmed inference-free metadata.

### Stage 1 — Correct scope without losing old memory

- [ ] **M1.1 — Introduce cached repository/worktree identity.**
  - Files: `cognee/config.rs::find_git_root/resolve_project_dataset`, `runtime.rs::session_start_for`; new small `identity.rs` if needed.
  - Depends on: M0.1 IDs. Preserve explicit dataset names and current non-Git behavior through documented migration.
  - Acceptance: linked worktrees share repository facts but retain branch restrictions; unrelated same-name repos remain separate; no per-prompt subprocess.
  - Verify: temporary Git main/linked worktrees, symlink/nested roots, non-Git directory, path move and detached HEAD fixtures.
- [ ] **M1.2 — Add non-destructive legacy aliases and bounded read-through.**
  - Files: identity/config and retrieval coordinator.
  - Depends on: M1.1 and M3.1 evidence scope metadata. Build the alias mapping earlier if useful, but do not enable cross-worktree read-through before provenance checks exist.
  - Acceptance: known old datasets remain discoverable; at most one relevant legacy fallback; uncertain legacy ownership never becomes an automatic cross-project fact. No bulk reingestion or deletion.
  - Verify: root/target dataset fixtures from this report, alias collision, unavailable legacy dataset, shared deadline, and rollback to original explicit dataset.

### Stage 2 — Remove automatic service work from ordinary turns

- [ ] **M2.1 — Add a pure event/admission policy.**
  - Files: new `cognee/policy.rs`, minimal `config.rs` read/write policy and command wiring; replace the unconditional length gate in `runtime.rs::recall_for_prompt`.
  - Depends on: M0.1. Default legacy configs to legacy behavior until a policy is explicitly selected; do not silently enable a provider. Expose the minimum reversible `event + manual` canary settings here, before final UI polish.
  - Acceptance: explicit history bypasses JEV; sufficient-context continuation and self-contained requests make zero service calls; short meaningful references are eligible; stable task events have host/explicit/main-agent producers; every decision has a reason.
  - Verify: table-driven cases with exact service-call counters, ten differently worded same-task follow-ups, unknown task identity, source-hash change without a new event, original intent versus expanded resources, and the four-word example corrected in the audit.
- [ ] **M2.2 — Add resident evidence/result caches and coalesced task prefetch.**
  - Files: runtime state and a small `coordinator.rs`/`resident.rs`; reuse atomic private-file helpers.
  - Depends on: M2.1, M1.1. Keep caches bounded and source-versioned.
  - Acceptance: ordinary prompt hook reads ready state only; repeated events share one operation; background work does not block model startup or rebuild a client each turn; retrieval freshness and judgment reuse have separate keys/limits.
  - Verify: fake-clock positive/negative expiry, unknown remote revision, local correction/index transition invalidation, duplicate scheduling, queue pressure, two-process resident-file updates and local-admission benchmark fixtures.
- [ ] **M2.3 — Bind work to immutable session/branch/context generations.**
  - Files: `ice-harness/src/extension.rs`, `agent_harness.rs`/`settle.rs` context-only delivery boundary, `interactive_tui/run.rs` original-intent metadata, `cognee/runtime.rs` background tasks and warmup buffering.
  - Depends on: M2.2. Avoid changing a public plugin ABI for an internal requirement.
  - Acceptance: session branch, Git ref/revision and delivery generation remain distinct; ready same-task evidence can reach a later model request without starting an unsolicited turn; old completions cannot inject into `/new` or a different task; failed captures retain original ownership; authorized durable writes retain their original context.
  - Verify: fake transport paused across session/resume/branch/dataset/reload switches, result before/after task completion, post-tool delivery, no persistence side effects in `transform_context`, original intent preservation, overlapping warmup drain, cancellation and bounded helper shutdown.

### Stage 3 — Return useful, attributable, bounded evidence

- [ ] **M3.1 — Preserve and normalize retrieval provenance.**
  - Files: `cognee/client.rs::normalize_recall/condense_graph_context`, `runtime.rs::recall_lines`, `text.rs`.
  - Depends on: M0.2. Keep old response shapes readable; absence of metadata stays explicit.
  - Acceptance: candidate-level source/scope/time survives to selection; raw object counts and selected snippet counts differ; context-only retrieval remains enabled.
  - Verify: captured/synthetic graph, session and rendered-context response fixtures, malformed/large response, missing IDs, and Unicode limits.
- [ ] **M3.2 — Add deterministic usefulness filters and active-context novelty.**
  - Files: new `cognee/evidence.rs`, runtime/context coverage.
  - Depends on: M3.1, M2.3. Do not interpret server score as a probability or rely on English keywords alone.
  - Acceptance: routine-success-only, stale, superseded, wrong-scope and exact duplicate items are not injected; necessary short history questions can retrieve facts; active context wins.
  - Verify: the observed commit-count and unrelated-topic cases plus positive decision/constraint cases; different queries with the same result; compaction reinjection.
- [ ] **M3.3 — Enforce a single token budget and safe append boundary.**
  - Files: `text.rs::format_recall`, `runtime.rs::turn_scoped_memory`, harness prompt-context append path.
  - Depends on: M3.2. Retain the stable system prompt and append-only persisted history.
  - Acceptance: checkpoint+recall+headers fit one budget, delimiters remain complete, no partial-stream mutation, and repeated evidence does not grow context.
  - Verify: tiny/large model windows, combined checkpoint plus recall, multibyte text, append failure, and late result fixtures.

### Stage 4 — Make durable memory selective and reliable

- [ ] **M4.1 — Unify permanent-write intent, uncertainty, and replay.**
  - Files: `cognee/queue.rs`, `runtime.rs::manual_remember/drain_pending/after_compact`, new candidate dispatch path.
  - Depends on: M0.1, M2.3. Keep old queue records readable and explicit uncertain flush semantics.
  - Acceptance: durable intent precedes request; timeout-after-acceptance cannot become automatically replayable Pending; semantic dedupe includes scope/category; no exactly-once claim without backend support.
  - Verify: crash/timeout/abort after server acceptance, duplicate text in different scopes, concurrent drains, restart, old record migration and explicit uncertain recovery.
- [ ] **M4.2 — Add attributable candidates and one promotion policy.**
  - Files: runtime capture/compaction/improve paths, `command.rs` skills/tool descriptions, bounded candidate store.
  - Depends on: M4.1, M3.1. Optional model proposal must flow through the same policy; inventory MCP writes first.
  - Acceptance: explicit remember/correction and attributable constraints/outcomes have a deterministic path with JEV off; the host resolves source IDs/spans; a rejected candidate cannot be promoted through capture, improve, dream, or compaction; optional pending candidates are bounded separately from explicit durable intent.
  - Verify: a policy matrix across all capture/improve switches; user correction vs assistant speculation; fabricated source ID, generated text attached to a real unrelated source, valid exact span, outcome without verification, pending expiry, and whole-session backend without per-entry filters.
- [ ] **M4.3 — Add correction, supersession, and freshness semantics.**
  - Files: candidate/evidence storage and selection.
  - Depends on: M4.2. Preserve audit history; do not delete old facts as part of default rollout.
  - Acceptance: an explicit newer correction supersedes the same scoped fact; unrelated branch claims remain separate; volatile facts require live verification.
  - Verify: contradictory statements by different roles, correction after compaction, changed source revision, ambiguous correction, and expiry.
- [ ] **M4.4 — Track accepted versus searchable and reconcile by revision.**
  - Files: `cognee/client.rs::remember/improve`, `runtime.rs::improve_current/dream_pass`, `dream.rs`.
  - Depends on: M0.2, M4.1. Do not mark a job completed merely from a 2xx response.
  - Acceptance: supported job handles are persisted; unsupported status yields accepted-unverified; same-revision jobs coalesce; retry/backoff is bounded; local curated facts remain usable with provenance.
  - Verify: accepted/delayed/failure/unsupported status, timed-out accepted job, no-new-data revision, restart and searchability lag using fake/recorded backend responses. Real Cognee probes may invoke downstream models and remain subject to the same test-model restriction.

### Stage 5 — Add optional JEV decisions after the deterministic baseline

- [ ] **M5.1 — Wire feature configuration to the existing decision client.**
  - Files: `cognee/config.rs`, `command.rs`, `ice-cli/src/decisions.rs`, a small `cognee/helper.rs`.
  - Depends on: M2–M4 contracts. Reuse TypeSafe protocol and existing credential binding; preserve explicit provider/model identity.
  - Acceptance: helper off uses no client or network; helper on binds a selected decision provider once per config revision; invalid binding is nonfatal and visible; no chat-provider fallback.
  - Verify: missing provider/model/credential, protocol mismatch, config round-trip and unknown legacy fields; no real credentials.
- [ ] **M5.2 — Implement bounded typed candidate judgments with abstention.**
  - Files: helper request construction and policy composition; existing TypeSafe codecs.
  - Depends on: M5.1, M3.2, M4.2.
  - Acceptance: candidate-ID validation, applicability plus classification, independent questions, explicit none/uncertain states, bounded/redacted state. JEV supplies no invented memory text.
  - Verify: yes/no/uncertain, malformed/unknown ID, wrong distribution, conflicting source roles, one-question dependency trap, redaction and token cap.
- [ ] **M5.3 — Enforce budgets, pooling, coalescing and failure isolation.**
  - Files: helper/coordinator and existing deadline/cancellation APIs.
  - Depends on: M5.2, M2.3.
  - Acceptance: no default 120 s helper timeout; no per-prompt or per-tool JEV loop; no foreground retries; process/provider quotas cover multiple sessions and queued work; queue wait expires obsolete work; settings/offline/session change cancels optional work.
  - Verify: hanging/mock slow endpoint, 429/529, simultaneous multi-session events, explicit-priority dispatch, expired queue item, canceled decode, stale result, alias/resolved model changes and bounded exit.
- [ ] **M5.4 — Gate JEV effectiveness independently from integration correctness.**
  - Files: evaluation fixtures/replay runner and feature rollout config.
  - Depends on: M5.3, deterministic baseline results.
  - Acceptance: mocks prove control flow only; reports keep JEV quality/latency unverified until authorized held-out tests show improvement over the local selective baseline.
  - Verify: four-arm protocol, chronological split, failure-inclusive accounting, calibrated thresholds, and ablation results. Do not call JEV under the current Luna-only testing authorization.

### Stage 6 — Capability suggestions without repeated context loading

- [ ] **M6.1 — Build a bounded local capability catalog from current registries.**
  - Files: `resource_dirs.rs`, `session.rs` resource loading/reload, `ice-harness/src/skills.rs`, MCP manager/proxy catalog.
  - Depends on: M2.3. Reuse project/global/package/builtin precedence; explicitly configure any `~/.agents` source.
  - Acceptance: descriptors have stable IDs/hash/availability; namespaced sources do not collide; hidden/disabled entries stay excluded from automatic suggestions; no broad home scan or per-prompt reconnect.
  - Verify: missing source, malformed metadata, source collision, reload, disabled tool/server, and explicit hidden-skill invocation.
- [ ] **M6.2 — Track whether full skill instructions are still in model context.**
  - Files: explicit invocation/skill-read tracking, context coverage, `interactive_tui/run.rs` and harness message metadata.
  - Depends on: M6.1, M3.3.
  - Acceptance: advertised metadata is different from full content; a partial file read is not a full load; compaction/branch switch invalidates coverage correctly; existing `$skill` rendering remains intact.
  - Verify: full/partial read, explicit invocation, skill revision update, same-name different-source skill, compaction and restored session.
- [ ] **M6.3 — Suggest only relevant missing capabilities at task events.**
  - Files: coordinator/catalog ranking plus optional helper finite Choice.
  - Depends on: M6.2; JEV ranking optional after M5.
  - Acceptance: exact inventory requests use local catalog; obvious matches bypass JEV; ambiguous shortlist can be judged once; suggestions are advisory and bounded, never auto-execution.
  - Verify: current “planning skill” case, already-loaded skill, unavailable tool, no useful candidate, repeated task, and built-in vs MCP memory routing.
- [ ] **M6.4 — Make memory tool result errors unambiguous.**
  - Files: `cognee/command.rs::CogneeSearchTool::execute`, runtime search result and recorder mapping; preserve MCP wrapper errors.
  - Depends on: M0.1–M0.2.
  - Acceptance: no-result, timeout, canceled, unsupported endpoint, and successful evidence are distinct; callback completion cannot inflate success metrics; tools remain nonfatal to the conversation.
  - Verify: native timeout/HTTP404/MCP error fixtures matching observed episodes and retry policy.

### Stage 7 — Settings, rollout and evidence of benefit

- [ ] **M7.1 — Expose coherent mode/settings and concise diagnostics.**
  - Files: `cognee/command.rs::settings_rows`, `builtin_extensions.rs`, TUI settings/activity.
  - Depends on: policy/config stages.
  - Acceptance: presets show manual/legacy/selective/off while the read/write policies remain explicit; helper provider/model, budgets and overlapping legacy capture settings are explained. The partial read canary is identifiable. Choosing the full selective preset applies its verified read/write policy atomically.
  - Verify: round-trip and live change, invalid provider, offline, rollback, canceled selector, legacy configs and environment overrides.
- [ ] **M7.2 — Run focused offline verification and chronological evaluation.**
  - Files: touched module tests and the evaluation protocol artifacts.
  - Depends on: relevant stages complete.
  - Acceptance: meaningful failure-path tests pass; baseline and candidate reports include denominators, unknown outcomes, harm cases and latency; no real agent directory is used by tests.
  - Verify: targeted `ice-cli` Cognee/MCP/skill tests, `ice-harness` prompt-context tests, and `ice-ai` decisions tests. Run repository CI gates at the eventual implementation handoff, not as a substitute for memory-quality evaluation.
- [ ] **M7.3 — Roll out local selective mode first; retain instant rollback.**
  - Files: config/migration, docs, evaluation summary.
  - Depends on: M7.2 and release gates.
  - Acceptance: an initial local-only shadow phase produces no JEV calls; selective recall can be enabled for the target project; automatic writes and JEV are enabled only at their respective verified gates; prior datasets remain intact.
  - Verify: rollback switches to legacy/manual/off, no queued optional job continues after disabled, and existing explicit tools still work. Update source docs and embedded copies only when their source files are changed.

### Dependency and parallel-work order

Start M0.1/M0.2 and define the operation/evidence contracts. Identity work (M1.1), pure admission/minimal config (M2.1), and provenance normalization (M3.1) can proceed with disjoint ownership. M1.2 read-through waits for M3.1. The write-state implementation M4.1 waits for M2.3's immutable ownership contract; preliminary queue design can happen earlier. Integrate generation-safe coordinator state before background delivery or JEV. Evidence filtering and selective-write policy must exist before trusting JEV outputs. Capability catalog work can run beside memory curation, but suggestion delivery must share context coverage and budgets. Evaluation and rollback are gates on every behavioral rollout, not a final optional polish phase.

The first useful deliverable is **M0–M3 with JEV off, `readPolicy=event`, and `writePolicy=manual`**: truthful diagnostics, correct scope, no blind prompt recall, and useful bounded evidence. Keep manual tools and local continuity anchors available; leave unchecked automatic promotion disabled in this canary. M4 supplies the reliable selective-write path before enabling the full selective preset. M5 should have to prove that it improves on those changes. This order avoids spending time integrating a classifier that hides existing storage and retrieval faults.

## 7. Verification and definition of done

Use the accompanying evaluation protocol as the quality gate. Minimum functional proof:

| Area | Required proof |
|---|---|
| Fast path | Zero Cognee/JEV requests for ordinary turns; initialized local admission meets the target budget |
| Recall | Short explicit history queries work; relevant old decisions are found; no broad automatic fallback after helper failure |
| Context | Scoped provenance retained; stale/duplicate/contradicted facts filtered; bounded complete envelope; stable prefix |
| Identity | Correct sharing across linked worktrees with branch restrictions and non-destructive legacy access |
| Remember | Every automatic promotion path respects selective policy; correction/supersession works |
| Reliability | Timeout uncertainty, original operation ownership, no late-session injection, bounded queues and shutdown |
| Capabilities | Current catalog supplies exact tool/skill; full-content coverage is accurate; suggestions do not execute |
| Measurement | Request acceptance, searchability, injection, use, task benefit, and errors remain different metrics |
| Effectiveness | Held-out local selective policy reduces unnecessary work without losing required historical facts; JEV demonstrates additional benefit before default enablement |

Recommended offline commands after code changes, using isolated fixtures and no provider credentials:

```text
cargo test -p ice-cli cognee:: --locked
cargo test -p ice-cli decisions:: --locked
cargo test -p ice-ai decision:: --locked
cargo test -p ice-cli mcp::proxy_tests --locked
cargo test -p ice-cli skill --locked
cargo test -p ice-harness prompt_context --locked
```

Test filters must be checked to ensure they select nonzero relevant tests; replace filters with the exact new test names as implemented. Required final repository gates remain those in `AGENTS.md`, including formatting, workspace check/test, documentation sync where applicable, and the prebuilt release script tests. Offline unit tests do not establish remote JEV latency or retrieval utility.

## 8. Risks, recovery, and decisions already resolved

**False-negative local admission:** the main agent's direct search tool remains the escape hatch. Evaluate genuine short continuity cases; do not solve false positives by blocking all automatic memory indefinitely.

**Scope migration leakage:** use known aliases with restrictions and provenance. Keep old datasets and configuration; rollback does not require a destructive reverse migration.

**Background contention:** give prefetch, improvement, capture, and JEV one shared concurrency/budget view. Moving work off the prompt await does not remove backend load, CPU use, or competition with explicit recall. Explicit current-task retrieval has priority over optional curation.

**Stale or misleading memories:** current files/user corrections win. A remembered path/test result is a pointer to verify, not proof that the current checkout behaves the same.

**Privacy and replay:** redact/minimize external decision payloads; keep local policy authoritative. Current redaction is heuristic. Treat timeouts after writes as unknown outcomes, not automatic retry permission.

**Overengineering:** no new memory platform, broad provider framework, universal skill planner, or separate summarizing agent in the first version. Add a local index/database or more complex model only when the measured workload justifies it.

Resolved design choices for the next implementer:

- Local admission first; ordinary turns do not call JEV or Cognee.
- Explicit memory requests bypass a classifier.
- Failure fallback is local-only/abstain for optional work, not legacy broad recall.
- Shared repository identity with worktree applicability; no global memory bucket.
- Source-selected facts and existing summaries; no mandatory extra generative summarizer.
- Selective mode controls every automatic durable-write path.
- Preserve existing transports, queue foundations, skills, MCP gateway, and append-only memory injection.
- JEV and automatic promotion remain individually gated until their quality and reliability are shown.

## 9. External references applied

Accessed 2026-10-27; fuller citations and caveats are in the upstream report.

- TypeSafe [System One](https://docs.typesafe.ai/api/system-one), [Introduction](https://docs.typesafe.ai/introduction), and [Models](https://docs.typesafe.ai/models): reuse typed state/questions and independent batching; pin versions for evaluation. Do not infer a latency guarantee.
- TypeSafe [Simple memory cookbook](https://docs.typesafe.ai/cookbooks/basic_memory): retain selection as an idea; reject its extra-call-after-every-request schedule for this task.
- TypeSafe [official skill](https://raw.githubusercontent.com/typesafe-ai/skills/main/skills/typesafe-ai/SKILL.md): finite candidate selection and applicability checks; code owns policy and actions.
- Cognee [Recall API](https://docs.cognee.ai/python-api/recall), [Remember](https://docs.cognee.ai/core-concepts/main-operations/remember), and [Improve](https://docs.cognee.ai/core-concepts/main-operations/improve): distinguish session/graph behavior and promotion stages. These current Python contracts are not proof that the installed HTTP wrapper exposes every feature.

## 10. Handoff

Read this plan, the code audit, and the observability report before editing. Recheck current `AGENTS.md`, branch, source diff and line locations. Start with M0 and the first three implementation stages; do not begin by adding a JEV call to `prompt_context`. Preserve concurrent changes. Keep a coverage ledger keyed by the task IDs above, with evidence and unresolved criteria; count completed work only after its acceptance and verification conditions are satisfied.
