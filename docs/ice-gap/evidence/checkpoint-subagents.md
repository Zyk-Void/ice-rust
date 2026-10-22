# Subagents source-audit handoff

Stable worker draft for coordinator review. No application/source changes or test execution occurred.

## Complete reading and mapping

- All 71 original ledger files (20516 ledger lines), including docs and implementation, were fully read.
- 3 supplementary TS implementation files, 37 TS test files, 3 native implementation/documentation files and 7 native test files were fully read: 121 distinct files total.
- Full-file identities are in subagents-read-evidence.json and proposal-subagents.json. Original source snapshots match the frozen manifest; supplementary TS tests match clean tracked frozen HEAD.
- 44 entries SUB-001 through SUB-044: {"missing":33,"partial":11}. Estimated added Rust source 10140–16550 lines, with cross-area prerequisites excluded.
- All 192 original candidate IDs are covered exactly once: 67 candidate IDs across 58 surface rows; 125 reasoned exclusions. There are 249 total surface rows, including 191 authored rows beyond candidates.
- Document checks: required fields, dependency graph, candidate identity and all surface anchors passed; read-only inspect-spec found 201 unique references with zero problems. No substantive read-range gaps.

## Key decisions

Native owner-scoped execution, inherited private model binding, cumulative output bounds, invocation/run/session cancellation, tracked cleanup permits and independent side questions are retained. Native child execution is partial infrastructure, not absent. A companion DLL implementation was not established by bounded inventory; conditional external-library tests are not proof of a bundled package.

Active V2 launch does not call the legacy instruction/bootstrap/preflight/routing helpers. The 8 alleged ICE_SUBAGENT/ICE_CAPABILITY/ICE_HOOK environment knobs are constants; defaultAgent/disabledAgents/askTimeoutMs are nested ice.subagents settings. Batch synthesis reports overlaps/disagreements and does not deduplicate sibling findings. --agents is JSON definitions, not a path list.

Source-only discrepancy details and exact hashes are in subagents-source-review.json, including malformed command-policy approval coercion, importer output rejected by current native schema, at-least-once completion crash window, abandonment/admission lifetime, already-aborted control wait, artifact allocation/durability limits and controller launch/callback cancellation limits.

## Pending coordinator work

Review the stable spec/proposal, reconcile 13 explicit dependency requests, and publish centrally. AI-006/AI-004 are direct bounded-finalization/retry prerequisites; AI-005 is the thinking-level prerequisite. TUI/RPC consumers and session journal/message integration remain named requests until assigned real IDs. No fake dependency IDs were introduced.

An automatic approval review rejected a later overwrite of worker read-request metadata as shared read-coverage mutation. It was not retried. The previously persisted requests already cover every original file; proposed additions were repeated anchor reads only. Root confirmed reuse of those complete records. Shared coverage/publication files remain coordinator-owned.

The worker is moving to the assigned low-level TUI fragment, TUI-201–299, under audit_extensions coordination.
