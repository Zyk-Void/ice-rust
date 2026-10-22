# Branch summaries

Tree navigation can keep a summary of the branch being left. The summary uses
the active model, contains the abandoned tail after the common ancestor, and
accepts optional instructions. It records the original leaf, provider usage,
and file-operation details. Existing history remains available in the tree.

Choosing navigation without a summary makes no provider request. Navigating to
the current point is a no-op; moving forward along the same branch has no
abandoned tail to summarize. A navigation operation owns lane admission while
it runs. Cancellation or a provider failure during generation preserves the
original branch.

The durable commit starts only after generation succeeds. Once that commit
starts, later cancellation does not roll it back. A storage failure during the
move or summary append attempts to restore the original branch and reports a
failed outcome. Once the summary append (or a plain navigation's lane move) is
durable, failures writing the step-attempt, label, or acceptance record return
a harness error explicitly saying the navigation committed but finalization
failed. They do not report a rejected move or roll back durable history. The
intent stays open until recovery completes those records. If storage remains
unavailable, recovery fails closed rather than admitting another operation.
The existing navigation intent reserves the summary entry ID and records
the source and target. On reopening after a crash, an incomplete move returns
to the source; an already committed summary is retained and the operation is
completed without a second provider request. An exact live retry of a summary
request whose finalization failed also recognizes the accepted result without
summarizing it again. Recovery does not automatically submit the restored user
message.

The public AgentLane navigation signature continues to target an exact entry
(or the root when no target is supplied). This feature uses existing JSONL
navigation, write-deferred, branch-summary, and operation-finished records; it
introduces no on-disk format fields or plugin ABI changes.

## Summary token reserve (public API)

`AgentHarness::set_branch_summary_reserve_tokens(i64).await` configures the
`branchSummary.reserveTokens` setting for subsequent summary requests and returns
`HarnessResult<()>`. The default is 16384. Nonpositive values clamp to 1; the
request caps the reserve at one less than the active model's context window.
This preserves a positive input budget, because the shared preparation helper
interprets a nonpositive budget as unlimited. A model declaring fewer than two
context tokens fails generation without moving the lane; an unspecified window
uses 128000. A reserve leaving too little room for any message produces a
`No content to summarize` entry without calling the provider.

`AgentHarness::branch_summary_reserve_tokens().await -> HarnessResult<i64>`
returns the configured value before the per-model cap. Both methods reject a
closed harness. The setting is shared across lanes, survives session replacement,
and is snapshotted for generation; changing it does not alter an in-flight
provider request. These are additive public methods: existing harness options,
navigation signatures, and persistence schemas are unchanged.
