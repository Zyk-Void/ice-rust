# Cognee observability audit — 2026-10-27

> **Redacted for publication.** Run, session and request IDs, local paths, path-derived dataset names and personal application names are removed or generalised. All counts, percentiles and technical findings are unchanged. The reproduction script and evidence JSON files this audit was built from are withheld because they embed private session text.

## Verdict

The recorded sessions support the user's impression that automatic memory often adds visible work without a clear benefit. Retrieval is running and frequently returns text, so the immediate problem is not simply that Cognee is disabled. The strongest evidence points to broad retrieval triggers, raw ambiguous queries, low-value captured material, project/worktree separation, and a manual-tool compatibility problem.

This is a retrospective audit of the retained database, not proof that memory has never helped. Relevant memories do appear. The available events do not establish whether a returned memory changed an answer, avoided a tool call, or saved elapsed time.

## Scope and evidence handling

- Target code: the planning worktree on branch `feat/cognee-helper`, initial HEAD `09a728c`.
- Source: the user-authorized local observability database in the icers agent directory.
- Access: Python SQLite URI `mode=ro`, `PRAGMA query_only=ON`, a read transaction per aggregate pass, no schema changes or application startup.
- No model or service request was made. Historical calls to other models are evidence from the database, not tests performed during this audit.
- Configuration inspection used individual noncredential JSON pointers in `~/.icers/agent/ice-cognee/config.json`. No credential file or environment variable values were read.
- All source timestamps below are UTC. The retained window is **2026-10-26 03:01:41.742 UTC through 2026-10-27 05:08:59.998 UTC**, about 26 hours 7 minutes. In the client's local timezone (UTC+03:00), that is 06:01:41.742 on October 26 through 08:08:59.998 on October 27.
- Snapshot counts remained unchanged across focused query passes.

### Reproducibility

The reproduction script and the JSON snapshots it produced are withheld: they embed local paths, request IDs and quoted session text. Every aggregate below comes from the SQL that follows, run read-only against the local observability database; the same queries reproduce the counts on any retained database.

Core SQL:

```sql
SELECT name, sql FROM sqlite_master WHERE type = 'table';
SELECT count(*), count(DISTINCT session_id),
       min(started_at_ms), max(started_at_ms) FROM observe_runs;
SELECT status, count(*) FROM observe_runs GROUP BY status;
SELECT run_id, sequence, ts_ms, event_type, payload_json
FROM observe_events ORDER BY run_id, sequence;
```

The source schema has `observe_runs` and append-only `observe_events`. It does not have a table of demonstrated memory usefulness. Exact schema is recorded in the summary JSON and matches `crates/ice-cli/src/observe/store.rs:33-56`.

## 1. Coverage and denominator discipline

| Measure | Observed |
|---|---:|
| Runs / distinct sessions | 46 / 46 |
| Runs with a Cognee memory observation | 40 |
| Persisted events | 3,601 |
| Activity items after replaying updates by (run_id, item.id) | 1,759 |
| Memory activity items after replay | 562 |
| User message items | 145 |
| Assistant message items | 341 |
| Tool activity items | 695 |
| Compaction items | 8 |
| Skill items | 3 |
| Runs marked finished | 5 |
| Runs marked running | 41 |

The 41 `running` rows do not prove that 41 processes are active. They are stored lifecycle status. Abrupt termination, incomplete shutdown recording, or still-running processes cannot be distinguished from this field alone.

The recorder updates an existing memory activity on terminal phases; counting every `activity_update` as a new operation inflates counts. See `crates/ice-cli/src/observe/recorder.rs:970-1014` and the regression seam `crates/ice-cli/src/observe/recorder_tests.rs:309-331`. The audit first replays items, then validates request IDs. There are **zero duplicate terminal request groups**. One separately retained recall `started` item shares a request ID with a succeeded skip; it is not an unfinished recall.

Retention is configured in current code as 30 days by run start, with pruning on store open (`crates/ice-cli/src/observe/store.rs:26-27,188-195,232-244`). The available 26-hour window may reflect recent installation, recording coverage, or prior pruning; this audit cannot infer which. It must not be treated as all historical conversations.

## 2. Automatic recall: frequent text, material waiting, no utility measurement

The 82 terminal automatic-recall observations divide as follows:

| Outcome | Count | Interpretation |
|---|---:|---|
| Succeeded with `meta.skipped=true, reason=short_prompt` | 22 | No retrieval attempt; not a memory success |
| Succeeded with `meta.hits=0` | 3 | Retrieval completed with no normalized results |
| Succeeded with `meta.hits=1` | 53 | Retrieval returned one normalized result object |
| Failed with timeout | 2 | Around the configured 10-second ceiling |
| Failed with aborted | 2 | Cancellation; distinguish from backend failure |
| Actual terminal retrieval attempts | **60** | 56 completed plus 4 failed/cancelled |

Of the 56 completed retrievals, 53 (94.6%) returned text. That is a **nonempty-result rate**, not a relevance or usefulness rate.

The graph client can condense multiple graph nodes into one text object; `hits=1` does not mean “one fact” or “one useful memory.” See `crates/ice-cli/src/cognee/client.rs:490-525`. Current recall terminal metadata does not preserve enough evidence to calculate whether the model consumed a specific fact or whether it was already present in context.

### Recorded recall durations

Nearest-rank percentiles, based on the recorded `latencyMs` values:

| Cohort | n | Median | p95 | Maximum | Summed durations |
|---|---:|---:|---:|---:|---:|
| Actual completed retrievals | 56 | 3,889 ms | 5,022 ms | 6,269 ms | 177,481 ms |
| Actual terminal retrievals, including timeout/cancellation | 60 | 3,891 ms | 5,233 ms | 10,007 ms | **203,470 ms** |
| Nonempty retrievals | 53 | 3,928 ms | 5,022 ms | 6,269 ms | 175,875 ms |

That is approximately **203.5 seconds of summed recorded recall duration across runs**. It is not a measured amount of recoverable user time: calls can occur in different or overlapping sessions, and useful recall may save later work. It also excludes independent manual-tool calls discussed below.

For comparison, including the 22 near-zero-duration skips would misleadingly reduce the displayed “recall median” to 2,552 ms. A dashboard should split decisions, skips, actual requests, timeouts, cancellation, returned results, injection, and demonstrated use.

## 3. Project and worktree cohorts

These are dataset labels observed in events. They are not inferred from the run's model or session name.

| Observed dataset | Memory runs | Actual terminal recalls | Nonempty / empty / failed | Short skips | Median / p95 | Summed recall duration |
|---|---:|---:|---:|---:|---:|---:|
| main ice-rust checkout (A) | 10 | 10 | 10 / 0 / 0 | 7 | 3,705 / 4,447 ms | 28,071 ms |
| feature worktree (B) | 10 | 21 | 16 / 2 / 3 | 7 | 3,841 / 5,233 ms | 72,068 ms |
| feature worktree, second label (C) | 2 | 0 | 0 / 0 / 0 | 0 | unavailable | 0 |
| home directory (D) | 15 | 25 | 25 / 0 / 0 | 6 | 3,993 / 4,735 ms | 84,840 ms |
| another ICE project (E) | 2 | 3 | 2 / 0 / 1 | 1 | 5,022 / 10,007 ms | 18,298 ms |
| probe (F) | 1 | 1 | 0 / 1 / 0 | 1 | 193 / 193 ms | 193 ms |

The target worktree's observed cohort (B) alone accounts for 72.1 seconds of summed automatic-recall duration. The main ice-rust dataset and the worktree dataset are distinct. A second worktree-looking label with another hash also exists (C).

This supports investigating canonical repository identity and intentional worktree overlays. The database alone does **not** reveal why the two worktree hashes differ or establish that cross-dataset information was desired. It does not justify merging all datasets: the home-directory and unrelated-project cohorts show why scope boundaries still matter.

## 4. Capturing and consolidating much more than durable facts

| Operation / observed terminal status | Count | Median | p95 | Maximum |
|---|---:|---:|---:|---:|
| `remember_entry` succeeded | 75 | 1,091 ms | 2,530 ms | 3,887 ms |
| `trace` succeeded | 246 | 90 ms | 14,101 ms | 26,113 ms |
| `trace` timeout | 8 | 30,001 ms | 30,002 ms | 30,002 ms |
| `remember` succeeded | 2 | 153 ms | 157 ms | 157 ms |
| `improve` accepted (logged succeeded) | 18 | 2,991 ms | 20,980 ms | 20,980 ms |
| `improve` timeout | 58 | 30,001 ms | 30,001 ms | 30,005 ms |
| `improve` started with no recorded terminal | 32 | unknown | unknown | unknown |

**An accepted improve request is not proof of completed consolidation.** The client explicitly submits `runInBackground=true` and only checks the HTTP response (`crates/ice-cli/src/cognee/client.rs:800-818`). Use “accepted,” not “18 successful memory promotions.”

The 58 timed-out improve calls represent 1,740,034 ms of summed request duration (about 29 minutes), often background work. This is not evidence that the foreground was blocked for 29 minutes. Of the timeouts, 33 carry a dream reason; 24 of those target the same recorded session ID. That is evidence worth investigating for retry/backoff and session-revision handling, not proof that every request was redundant.

The 32 started-only improve requests are **outcome unknown**. They must not be counted as failed or as successfully completed. No end-to-end confirmation of graph readiness was present in this database.

### What was stored

- A greeting exchange (a short hello and its reply) was stored as a Q/A entry in one run, sequence 10. That succeeded after 2,430 ms. Sequence 13 then records an improve request accepted after 16,439 ms. A greeting is a concrete example of captured material with little durable recall value.
- Both observed permanent `remember` operations contain `ICE compaction checkpoint`, with reason `threshold` and engine `native`. One concerns a web service, the other a code audit. They are compaction checkpoints, not evidence of curated user-preference or project-decision records.
- Retrieved text includes repeated summaries such as `bash succeeded` and `read succeeded`, and a graph summary describing thirty-one successful tool operations. This is directly observed low-information recall content.
- Auto-recall currently searches session and graph scopes. The available result previews do not retain origin metadata sufficient to prove whether low-value trace summaries came from the session scope, graph promotion, or an imported historical corpus. Do not claim a specific provenance without server-side evidence.

## 5. Representative episodes

The following are selected diagnostic cases, not a random sample. Do not compute precision or an improvement rate from this table.

### A. Relevant retrieval that could help orientation

- Run: (ID withheld).
- Recall sequence 4; 3,769 ms.
- User asked to explore the codebase and explain it. Returned context describes ice-rust/icers and its crate layout.
- Assessment: topically relevant orientation. Current code still needs inspection, and no counterfactual run establishes saved work. This is evidence that memory can return useful-looking context, not proof of measured benefit.

### B. Relevant continuity with fresh verification still required

- Run: (ID withheld).
- Recall sequence 116; 4,003 ms.
- User asked which version of a game-modding setup was last used. Returned context included a prior migration and a launcher.
- Downstream, the agent queries the launcher's database, inspects its configuration, and reports the current last-used record and paths.
- Assessment: plausible historical context, but the answer is freshly verified. The evidence does not establish that memory saved a lookup; keeping current-state verification was correct.

### C. Ambiguous prompt retrieves the wrong subject

- Same run as B, recall sequence 184.
- 3,993 ms.
- Query: `which one is main`.
- Preview: older unofficial Python API, a gateway source file, Handy's primary window, and unrelated directories.
- Assessment: the user was discussing their game installation. Searching the raw four-word prompt lost the active subject. A binary “memory might help” decision alone will not repair this; the retrieval query needs the task/entity context or the lookup should be skipped in favor of current visible context.

### D. Commit count retrieves tool-success counts

- Run: (ID withheld).
- Recall sequence 530; 510 ms.
- Query asks how many commits were made in the worktree.
- Retrieved preview leads with thirty-one successful tool operations and repeated `bash succeeded` / `read succeeded` entries. It also mentions a commit title, so the entire result is not wholly unrelated.
- Downstream sequence 532 correctly identifies two commits already made during that conversation.
- Assessment: current conversation state was sufficient, while the retrieved leading content confused operations with the requested concept. The result's presence is not evidence it contributed to the answer.

### E. A routine build pays a multi-second memory cost

- Same run as D, recall sequence 537.
- **4,423 ms**.
- Query asks to build the current branch to try it.
- Returned preview includes terminal/image support and repeated tool-success strings.
- Downstream sequence 542 executes the expected `cargo build --locked -p ice-cli --bin icers`; its captured Cargo output says the incremental build itself finished in 0.33 seconds.
- Assessment: this is a concrete candidate for skipping automatic retrieval. The comparison is illustrative, not a universal build benchmark; the recall duration exceeded that recorded incremental compile duration.

### F. Skill inventory question gets memory rather than capability discovery

- Run: (ID withheld).
- Recall sequence 127; **4,375 ms**.
- Query: whether there is a skill for planning.
- Returned context discusses the previous `$` autocomplete implementation.
- Assistant sequence 129 says no dedicated planning skill is available in the current session. The user then explicitly directs it to check `~/.agents`.
- Assessment: installed/available skill discovery belongs to a capability catalog, with live availability and loaded-in-context state. Historical skill names and remembered descriptions cannot establish that a tool is installed, permitted, or already loaded.

### G. Expanded skill instructions become the recall query

- Run: (ID withheld).
- Recall sequence 4; **6,269 ms**.
- The recorded query begins with an expanded `<skill name="context-mode" ...>` block. Returned text repeats historical context-mode rules and an older skill location.
- Assessment: retrieval should receive the user's task intent and structured resource state, not a full expanded skill body. Recalling historical instructions when current instructions are already present adds duplication and risks confusing source freshness.

### H. Short prompts are skipped mechanically

Observed skipped prompts include greetings, partial slash commands, `continue`, `commit this`, a short question about which launcher is used, `replace it`, and `check in ~/.agents`. These are not all equivalent. Some can be answered from current context or local tools; some can refer to prior work. The data demonstrates the lexical rule, not that every skip was wrong.

## 6. Manual tool failures and discoverability

Only three Cognee-named tool calls appear among the 695 recorded tool items:

| Tool / episode | Recorded behavior | Duration |
|---|---|---:|
| `cognee_search_tools`, sequence 39 | Request to enumerate memories returned `code_search` and `cognify_status` capability descriptions | 5 ms |
| `cognee_recall`, sequence 44 | Result reports HTTP 404 for `/api/v1/recall` | **40,363 ms** |
| Built-in `cognee_search`, sequence 47 | Result reports `Cognee search failed: timeout` | **10,001 ms** |

All three are in one run. The two failed operations are stored as tool `state=done`; semantic failure is in the result text. A dashboard or evaluation that treats `done` as successful will be wrong. The native path already returns a structured `details.error` (`runtime.rs:1570-1571`), but `command.rs:80-83` wraps that response in `Ok(AgentToolResult)`. Propagate that existing error information into the tool and observation outcome rather than inventing an unrelated error channel. `search_for_tool()` at `runtime.rs:1520-1573` does not emit the automatic recall observations, so the 60 automatic terminal requests exclude this manual timeout.

This is a **historical endpoint compatibility failure** between the exposed `cognee_recall` tool and the responding service. This audit did not make a live HTTP request, so it does not claim the endpoint remains broken. A future scoped smoke test should discover and validate the available adapter/server capabilities before routing work there.

In the current helper-planning run, sequence 10 records `mcp` searching for `decision api`. It returned six entries including `cognee_remember` and context-mode tools, with no TypeSafe decision endpoint. Generic tool-description matching is not equivalent to an authoritative capability map.

There is no observed successful manual Cognee retrieval in this small retained window. This says nothing about earlier unrecorded sessions. The two `remember` observations are compaction writes, not manual fact-curation examples.

## 7. Safe configuration snapshot

Observed noncredential file values:

```json
{
  "enabled": true,
  "autoRecall": true,
  "autoRemember": "compaction",
  "captureSession": true,
  "captureTools": true,
  "autoImprove": true,
  "compactionSummaryMode": "auto",
  "checkpointRecall": false,
  "dataset": "$project",
  "recallBudgetMs": 10000,
  "recallMaxChars": 6000,
  "captureMaxChars": 8000,
  "topK": 5,
  "queueLimit": 64
}
```

This is the stored configuration at audit time, not a reconstruction of every session's effective environment. Environment overrides and historical settings can differ. The behavior in the window is consistent with broad capture and recall.

The user is not simply failing to enable memory. The selected configuration enables automatic recall, Q/A capture, tool capture, and automatic promotion together. A selective-remember helper would be undermined if those independent broad ingestion paths continue to promote discarded material.

## 8. What the plan should fix first

1. **Stop unnecessary foreground requests.** Use explicit recall requests, resume/compaction/task transitions, missing historical context, and local capability discovery as distinct routes. Existing context sufficient for a build, current commit count, or loaded skill should suppress retrieval. Do not add a remote JEV call to every prompt.
2. **Preserve task/entity context in queries.** Distinguish “which one is main” in the current task from a generic global memory question. Construct bounded queries from the original user intent plus relevant resolved entities; exclude expanded skill text, injected memory, and large tool outputs.
3. **Make memory records useful and scoped.** Prefer verified decisions, corrections, stable preferences, failed-attempt lessons, and compact next-step pointers over greetings and tool-success transcripts. Separate temporary scratch/capture, resumable checkpoints, durable facts, and procedural skill indexes.
4. **Repair the manual path and names.** Validate the server's supported endpoint/schema; distinguish code search, conversational recall, native `cognee_search`, MCP `cognee_recall`, and TypeSafe decisions. Propagate existing semantic error details into tool status so timeout/404 cannot be mistaken for a successful retrieval.
5. **Define repository/worktree identity deliberately.** Share stable project facts across related worktrees while keeping branch-specific status and unrelated home-directory activity separate. The observed dataset split is a diagnostic lead, not authorization to bulk-merge old graphs.
6. **Bound consolidation independently.** Track accepted versus completed ingestion, use finite retries/backoff and a shared scheduler, and avoid retrying unchanged revisions. Preserve unknown outcomes instead of replaying uncertain writes blindly.
7. **Use JEV on shortlisted decisions.** An occasional typed judgment can prioritize a candidate memory, rank a bounded capability shortlist, or decide a genuinely ambiguous retrieval event. Local deterministic gates should handle obvious skip/manual/capability cases. These are proposed roles; no JEV benefit or latency was measured here.
8. **Measure marginal benefit.** Record context-satisfied, requested, executed, timed-out, hit IDs, selected/injected token count, source timestamps, model-visible use evidence, and a small human usefulness label. Compare against local-only and current behavior on a consented replay set. Hit rate alone is not a success criterion.

## 9. Limits and unanswered questions

- No randomized memory-on/off comparison, model replay, or JEV test was performed.
- No successful recall-to-use trace with stable fact IDs exists in the inspected schema. A model could have used a returned fact silently; absence of explicit citation is not proof of nonuse.
- Preview text is bounded (current observation preview ceiling is 900 characters; `crates/ice-cli/src/cognee/observer.rs:17-22,105-117`). Selected excerpts can demonstrate visible irrelevant material, but they cannot fully score every returned item.
- The live Cognee service, graph contents, indexes, readiness, and backend versions were not inspected. A successful HTTP acceptance does not prove durable graph availability.
- Dataset naming history and actual per-run CWD are not encoded in the run schema. Two similarly named datasets require further scoped investigation.
- The cohort includes home-directory activity, another ICE project, and a probe; all-run metrics must not be presented as target-worktree-only statistics.
- Historical logs contain user requests and tool outputs. The artifacts intentionally contain aggregates and a few relevant excerpts, not a dump of private sessions.
