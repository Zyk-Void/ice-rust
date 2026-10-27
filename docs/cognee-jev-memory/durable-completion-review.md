# Cognee durable completion review

## Implemented in this worktree

- Added a process-wide, nonblocking advisory dispatch lease at the queue directory (`.dispatch.lock`) using the already-declared workspace `fs2` 0.4 dependency. Automatic drain holds the lease from before scanning through durable outcome processing; immediate manual remember uses the same lease discipline. A second owner returns without sending. Manual intent is enqueued first and remains `Pending` when dispatch is busy.
- Split the short enqueue/capacity critical section into `.enqueue.lock`; it is independent of the HTTP dispatch lease, so producers can enqueue while a sender waits on Cognee. Queue capacity counts durable JSON operation records, excludes lock files, and does not evict existing records.
- Startup inflight recovery now acquires the dispatch lease nonblocking. A live owner’s `InFlight` records are not changed; once the owner disappears, recovery changes orphaned inflight operations to `Uncertain`, which is never auto-selected. Explicit uncertain flush remains deliberate. There is no exactly-once claim.
- Outcome admission now requires the claimed normalized text to occur in both the assistant source and the cited successful tool result. An unrelated successful command cannot verify an arbitrary outcome. Exact source excerpts remain accepted; paraphrased user constraints remain pending under the existing policy.
- The proposal tool honors cancellation. Candidate text is not truncated after verification: over-budget content fails admission to the durable queue. Supersession targets are checked for accepted state, same dataset/branch and user source before queueing.
- Candidate attribution is written as `Accepted` before the outbox enqueue and drain scheduling, so accepted durable intent has local source attribution independent of remote indexing. Existing candidate record shape is unchanged; no migration is needed.
- Added `candidates::query_accepted`, a bounded local-only query returning `RecallResult` values with candidate/source/category/dataset/branch/expiry/supersession metadata. It filters expired and superseded facts and performs no network operation; callers should load/refresh at admission or lifecycle boundaries.

## On-disk compatibility

Existing pending remember JSON records and candidate JSON records retain their format and continue to load. The lease and enqueue lock are new persistent zero-content files adjacent to pending operation records; they are not counted against queue capacity and may safely remain after process exit. Kernel lock ownership is released on handle drop/process death. Existing `InFlight` operation records are recovered only when no process owns dispatch. Suppression record schema was not changed. No Cargo.lock change was required because `fs2` was already present.

## Verification

- `cargo test -p ice-cli --lib --locked cognee:: --quiet` — **174 passed, 0 failed, 0 ignored, 1,253 filtered out** (final captured run; faux transports and temporary directories).
- Added/updated regression coverage for attributable verifier text, simultaneous independent lease acquisition/live-owner recovery versus orphan recovery, and local accepted evidence provenance/expiry/supersession filtering.
- Formatting was run only on owned Rust files: `queue.rs`, `runtime.rs`, `candidates.rs`, and `propose.rs`.
- No Cognee/backend or inference calls were made. No full-workspace gates were run, as assigned to the lead.

## Important limits and integration required

1. Candidate scope currently has no host-provided conversation branch or Git revision identity: proposals set `branch: None`. The lead must supply cached repository/conversation scope when constructing candidates and pass it to admission/supersession validation; do not infer it with per-prompt Git calls.
2. Candidate review storage remains file-backed. `query_accepted` is a pure disk-reading API; the lead’s resident-cache work must call it at admission/lifecycle boundaries and retain results, not scan on ordinary prompts.
3. The current suppression ledger is append-only and exact-text based. Existing `record_accepted_suppressions` integration provides local recall filtering, but a crash-atomic transaction spanning suppression, candidate state, and outbox is not provided by the current storage APIs. The lead should ensure local delivery does not treat a new correction as effective until the suppression write and accepted candidate attribution both succeed; preserve old remote evidence.
4. The proposal API accepts source entry IDs and an optional `verified_by` ID, but `accept_candidate(runtime, &candidate)` remains the compatibility entry point for helper calls. It revalidates source attribution through `propose_memory` at explicit tool entry; helpers operating on stored candidate snapshots must refresh source/branch validity before calling the shared acceptance path. Helper candidate promotion and manual `/cognee remember` are distinct legacy operations.
5. A known-existing accepted duplicate path returns the existing queued result label but may still write a new candidate attribution row. A future API should return the actual existing candidate/operation identity to callers.
6. Accepted and searchable remain distinct; this change does not add a Cognee job/status API or claim remote indexing/searchability. No receipt fields were altered.
7. The cross-process lease protects senders using this queue directory. It cannot coordinate external writers that bypass this implementation, and advisory-lock correctness depends on the filesystem honoring kernel locks.

No branch merge, publication, or protected-branch operation was performed.
