# Cognee and JEV: upstream findings for selective memory

Research date: 2026-10-27.

Status: primary-source research and proposed design guidance. This report does not establish the behavior, version, latency, configuration, or usefulness of the installed ice-rust integration. Those require the accompanying repository and observability audits.

No live model, paid inference API, icers session, or remote repository write was used for this research. Live testing authorized in the present task is restricted to cx/gpt-6-luna; JEV-specific tests described here are future validation work, not completed experiments.

## 1. Recommended direction

Use JEV after the application has identified a meaningful event and a small set of relevant candidates. Keep ordinary turns on a local path. The main opportunities are selecting durable facts after confirmed decisions, filtering ambiguous retrieval results, and selecting an unloaded skill from a local shortlist.

A remote “would memory help?” classifier before every prompt would add a dependency to every response. It would also ask the classifier to judge the value of information it has not retrieved. The better question is whether an available candidate contains a fact that changes the current task, is absent from active context, and applies to this project.

This distinction affects both usefulness and latency. Useful memory reduces repeated investigation, preserves a constraint the agent would otherwise miss, or supplies an earlier decision. A large volume of captured conversation is not itself evidence that any of those outcomes occurred.

The target should therefore be a selective retrieval and curation policy with an optional JEV component, rather than making JEV a prerequisite for all memory operations.

## 2. What current TypeSafe documentation establishes

### Typed judgments and model identity

JEV accepts supplied state and typed questions. Choice selects from an enumerated set; Score evaluates an ordered rubric; Noul estimates the probability of a yes answer. Questions can share state in one request and are evaluated independently. One question cannot consume another question’s answer within that request. These capabilities support bounded classification and ranking; they do not provide text generation or an agent tool loop. [T1][T2]

The current official model table identifies jev-1.14.0 as the newest release, available from October 14, 2026. The jev-latest alias currently points to that version. The response reports the actual model ID. Pin a version when tuning policy; an alias can change without a code change. The listed default limits are 64k tokens per request and 32k for state plus any single question. The documentation imposes no separate question-count limit within those budgets. [T3]

Those are upstream identifiers. A proxy can expose different model names, so implementation must preserve the configured provider’s routing contract. Third-party model directories are not authoritative for the native TypeSafe API.

### Confidence needs interpretation

Choice and Score expose confidence derived from the concentration of their probability distributions. Noul has no separate confidence field. Concentration is not a measured probability that the entire application action is correct. A confidently selected skill can still be inapplicable if the candidate set contains no suitable skill. [T4][T5]

The current official skill recommends checking applicability separately, often with a companion Noul, and keeping decisions in code. It gives 0.3 and 0.7 as possible initial Noul boundaries but explicitly treats thresholds as application-dependent. These must not be presented as validated recall or memory-promotion thresholds. The same skill advises looking at Score distributions because a mean can conceal competing high and low judgments. [T6]

For example, selection can ask both “Does any candidate supply a capability needed for this task?” and “Which candidate supplies the most relevant capability?” Application code ignores the selection when applicability is low or unresolved.

### Selection is easier to audit than generation

TypeSafe’s extraction cookbook uses ordinary code to find candidate spans, JEV to select one, and code to copy the selected text. This constrains output to observed evidence. It is a useful model for memory curation: preserve the user’s statement or a verified task result, attach source identity, and classify its durability. It does not make the underlying statement automatically true. [T7]

Do not ask JEV to author a concise new memory sentence. It does not generate prose. A new summary would require a separate generative step, reuse of a summary the main agent already produced, or deterministic formatting around selected source spans.

## 3. What current Cognee documentation establishes

### Storage paths are different

Remember without a session ID runs permanent ingestion. With a session ID it writes to a session cache, and self-improvement can start a background bridge into the graph. Upstream defaults therefore matter: choosing not to enqueue a selective permanent write does not necessarily prevent another capture path from promoting the same content. [C1]

The remember documentation also warns that an unavailable cache may still return a session-style result without normal session persistence. A successful response is weaker evidence than a later scoped retrieval of the intended fact. This is a specific reason to distinguish accepted, stored, indexed, and retrievable states in diagnostics. [C1]

### Recall is not one uniform operation

The current recall documentation describes keyword matching over session question, context, and answer fields. Graph recall commonly falls through to HYBRID_COMPLETION. Dataset, query-type, session and scope arguments affect the path chosen. Supplying a session ID alone does not prove a request performed only a cheap cache search. [C2]

The Python API documents explicit graph, session, trace and session-context lanes. Session-first can short-circuit graph retrieval after a session hit. Tools and code are opt-in lanes even when the caller requests all ordinary sources. Consequently, an integration should not assume ordinary memory recall also discovers suitable tools or unloaded skills. These surfaces must be checked against the deployed service version. [C3]

For an agent that generates the final answer itself, request evidence rather than an additional generated answer when the adapter supports it. Cognee’s only_context option avoids the completion step. It can still involve embeddings and context assembly, and the returned text can contain a rendered prompt rather than separate source records. It is not a guarantee of zero model work or a ready-to-inject evidence format. [C4][C5]

The accompanying code audit confirms that ice-rust already sends `onlyContext: true`; retain this behavior. The proposed changes concern selection, provenance, duplicate suppression and scheduling.

### Promotion and improvement require outcome evidence

Improve can bridge sessions, distill guidance, apply feedback and run optional enrichment or conflict-review stages. Current documentation describes stage-level outcomes such as completed, skipped, already completed and error. Session IDs and opt-in settings determine whether relevant stages have work to do. Recording only that an improve request succeeded does not establish that useful session content became durable, retrievable memory. [C6]

Session identity also matters. The current documentation explains dataset-scoped default sessions and older global-default behavior. Historical entries are not automatically migrated to newer scopes. This creates a compatibility question for an existing integration, not proof that ice-rust currently has a scope bug. [C7]

## 4. Patterns to adopt, adapt and reject

| Upstream pattern | Decision | Reason |
|---|---|---|
| Batch independent questions sharing useful state | Adopt | Avoid repeated input and serial remote round trips. |
| Select pre-existing candidate spans | Adopt | Preserves provenance and prevents invented memory text. |
| Code owns permissions, exact checks and final actions | Adopt | Model output remains a fallible signal. |
| TypeSafe simple-memory cookbook | Adapt narrowly | Its selection idea is relevant; its per-call schedule is unsuitable. |
| Classify retrieved passages before injection | Adapt selectively | Useful for ambiguous or conflicting results, but adds latency if universal. |
| Load all memories or skill bodies into every prompt | Reject | Consumes context and weakens relevance. |
| Interpret API success as proven usefulness | Reject | It does not establish retrieval, use or time saved. |

The official simple-memory example adds a JEV request after each call, stores whole requests and answers, and reuses a small in-memory customer history. Its six-ticket comparison is an illustrative demonstration. Copying that schedule would violate this task’s requirement to avoid triggering a remote helper on every prompt. [T8]

The RAG-passage cookbook distinguishes useful evidence, conflicts and attempts to instruct the answering model. This is a good separation of concerns, but its example adds several judgments per passage. Use that pattern only where a bounded result set is genuinely ambiguous; local duplicate, scope and expiry checks should run first. [T9]

## 5. Proposed execution paths

These are design recommendations, not claims about current code.

### Path A: ordinary conversation

Run deterministic eligibility checks and consult already-resident context. Acknowledgements, straightforward continuations and questions fully answerable from active context should cause no remote helper call and no automatic remote recall.

The check must consider conversational state. A short message such as “what did we decide?” can need memory; a long self-contained prompt might not. Word count alone is a poor eligibility policy.

### Path B: explicit retrieval or unresolved reference

An explicit request to retrieve previous work should call the existing scoped memory tool directly. A helper should not veto it. The main agent should retain an on-demand retrieval escape hatch when it notices a missing decision, environment fact or previous outcome.

Local retrieval can be attempted first when available. The query should include the relevant project, task and named entity rather than blindly forwarding the whole latest prompt.

### Path C: task boundaries and resumed work

At a resumed session, task change or post-compaction continuation, inspect a local index of project facts and completed outcomes. Prefetch a small relevant set when a clear need exists. Cache results with project identity, task signature, memory revision and policy version.

Fresh remote evidence cannot always arrive at zero latency. Each path must choose explicitly between waiting within a bounded deadline, prefetching before it is needed, or allowing the main agent to retrieve it later. Avoid promising all three properties simultaneously.

### Path D: ambiguous candidates

Use JEV only when the shortlist remains ambiguous after cheap checks. Appropriate judgments include whether a candidate changes the current action, conflicts with an established fact, represents a durable constraint, or supplies an otherwise unavailable capability.

Batch independent questions where state overlaps. If a later question requires the answer to an earlier one to construct its input, keep that dependency explicit rather than pretending parallel execution resolves it.

## 6. Selective remembering and skill discovery

Candidate creation should be driven by substantive events: a confirmed project decision, user correction, accepted constraint, verified fix, task completion or compaction checkpoint. Coalesce candidates before background classification. A new tool result is not automatically a new learning.

Every saved candidate should retain its source role, project identity, timestamp, evidence location, confidence category and revision relationship. Distinguish an assistant proposal from a user-approved decision and a command’s intended effect from its verified result. Corrections should supersede earlier entries without silently preserving both as equally current.

Use one policy for all automatic promotion paths. Session capture can remain useful for diagnostics, but its promotion behavior must not contradict selective remembering. Test this interaction explicitly.

For skills and tools, maintain a small local catalog of stable IDs, descriptions, trigger examples, availability and revisions. Compare the shortlist with IDs already loaded. Load the chosen skill body or exact tool schema on demand. A tool suggestion should never imply a connection exists or grant permission to invoke it.

Official upstream skills relevant to implementation include TypeSafe’s typesafe-ai, plus Cognee’s cognee-recall, cognee-improve-sessions and cognee-performance. Their existence is verified; installation and compatibility in the user’s harness are not. Use descriptions for discovery, and inspect full instructions only when a skill is actually relevant. [T10][C8]

## 7. Performance and usefulness evaluation

Compare four policies on chronological held-out cases: memory off, current behavior, deterministic selective retrieval, and selective retrieval with JEV. Separate retrieval quality from answer quality.

The replay set should include earlier-decision questions, tasks fully answerable from active context, short continuation messages, corrections, repeated resolved failures, helpful unloaded skills, unrelated tasks and misleading near matches.

Measure whether the needed fact was retrieved and actually used; unnecessary recall rate; stale or conflicting injections; repeated investigation avoided; correct skill suggestion rate; redundant loads; remote request count; injected tokens; time to first response; and total task time.

Do not infer time savings from fewer search calls alone. A blocked classifier can erase the benefit. Likewise, an injected memory block does not prove the answer used it correctly. Report the counterfactual comparison and inspect representative failures, not only aggregate hit rates.

Temporal replay must exclude memories created after the historical query, later solutions and evaluation labels. Split by session or task rather than scattering adjacent turns across training and evaluation.

No source reviewed establishes a latency target or expected quality gain for ice-rust. TypeSafe’s batching cookbook supports reducing redundant serial calls on its own workload; its reported speedup should not be copied into this project’s success criteria. [T11]

### Starter budgets to test, not promises

Possible initial engineering limits are zero helper network calls for ordinary turns; at most one optional JEV request for an ambiguous eligible event; one in-flight background decision per session; and a small bounded shortlist rather than the entire memory corpus or skill catalog.

A candidate foreground deadline, cache lifetime or injection-token cap should be selected from the observed distribution and model context budget. The plan should label any starting values as hypotheses and include a rollback condition if task latency worsens.

A timeout is an unavailable decision, not a negative judgment. A still-relevant candidate may be rescheduled under the shared rate, concurrency and expiry policy; do not automatically retry foreground decisions or keep obsolete events alive. Foreground uncertainty should normally continue with existing local information or the direct retrieval path appropriate to an explicit request.

## 8. Validation boundaries

Mocked JEV responses can verify policy composition, malformed output handling, cancellation, cache invalidation, deadlines and promotion gates without calling JEV. All live model testing in this task is restricted to `cx/gpt-6-luna`, including model calls indirectly triggered by Cognee operations. Real remember/improve/search probes require inspection of downstream inference; HTTP calls to Cognee are not automatically exempt. Use mocks for JEV and inference-producing backend contract tests.

Mock success does not establish JEV accuracy or latency. A later explicitly authorized JEV evaluation should use the pinned configured model, representative redacted inputs, exact policy version and held-out cases. It should preserve failures as failures and avoid tuning to a handful of examples.

Repository analysis must still establish which of these capabilities already exist. The implementation plan should reuse the current transport, decision client, durable queue, untrusted-context wrapper, settings and cancellation machinery where present, rather than introducing parallel systems based solely on upstream examples.

## Primary sources

All accessed 2026-10-27. URLs point to current upstream documentation; behavior must be reconciled with installed versions.

- [T1] TypeSafe, Introduction: https://docs.typesafe.ai/introduction
- [T2] TypeSafe, Jev with coding agents: https://docs.typesafe.ai/introduction/coding-agents
- [T3] TypeSafe, Models: https://docs.typesafe.ai/models
- [T4] TypeSafe, Confidence: https://docs.typesafe.ai/confidence
- [T5] TypeSafe, Noul: https://docs.typesafe.ai/primitives/noul
- [T6] TypeSafe, official agent skill: https://raw.githubusercontent.com/typesafe-ai/skills/main/skills/typesafe-ai/SKILL.md
- [T7] TypeSafe, Pre-parsed value extraction: https://docs.typesafe.ai/cookbooks/pre_parsed_value_extraction_cookbook
- [T8] TypeSafe, Simple memory system: https://docs.typesafe.ai/cookbooks/basic_memory
- [T9] TypeSafe, Classifying RAG passages: https://docs.typesafe.ai/cookbooks/classifying_rag_passages
- [T10] TypeSafe, Agent skill: https://docs.typesafe.ai/agent-skill
- [T11] TypeSafe, Parallel questions: https://docs.typesafe.ai/cookbooks/parallel_questions
- [C1] Cognee, Remember: https://docs.cognee.ai/core-concepts/main-operations/remember
- [C2] Cognee, Recall: https://docs.cognee.ai/core-concepts/main-operations/recall
- [C3] Cognee, Python recall API: https://docs.cognee.ai/python-api/recall
- [C4] Cognee, Inspecting Graph Completion Context: https://docs.cognee.ai/guides/graph-completion
- [C5] Cognee, Understand Recall with RAG Completion: https://docs.cognee.ai/guides/rag-recall
- [C6] Cognee, Improve: https://docs.cognee.ai/core-concepts/main-operations/improve
- [C7] Cognee, Sessions and Caching: https://docs.cognee.ai/core-concepts/sessions-and-caching
- [C8] Cognee, Agent Skills: https://docs.cognee.ai/getting-started/llm-quickstart-skill
