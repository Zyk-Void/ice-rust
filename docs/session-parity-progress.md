# Session parity implementation progress

> Recovery note: the prior untracked version of this file was accidentally
> overwritten during this work and no backup was available. This file is a
> best-effort reconstruction from retained conversation evidence, not a verbatim
> restoration; `goal.md` is the authoritative checklist. Earlier detailed source
> maps, execution history, and intermediate verification notes may be missing.

This is the implementation checklist for the session worktree. The source audit
is `docs/ice-gap/session.md`; its findings are a baseline, not implementation
completion evidence. The 31 `missing`/`partial` SESSION items are an audit
catalog, not a requirement to implement every TypeScript feature. The current
direction is solo implementation, review of existing changes for demonstrated
need, and verification of useful session-management correctness/safety fixes.
Broader SDK, background-shell and lifecycle additions are deferred unless a
concrete use case justifies them. Pending rows below record gaps, not automatic
implementation commitments. The seven `different-by-design` items require
explicit compatibility decisions and must not silently replace Rust's existing
durable formats.

## Constraints

- Rust 1.78-compatible, additive public APIs where possible.
- Keep native v4 JSONL and canonical SQLite recovery guarantees.
- No access to TypeScript ice's runtime state; reference source/tests only.
- No real-provider tests, commits, pushes, releases, or version changes.
- Complete a row only with implementation and observed regression-test evidence.
- External Rust guidance: Apollo `rust-code-review`, revision
  `0727a45244caa460be11d2c446e9b79a518bd8c4`, fetched outside the repository.

## Reliable-session goal execution update

`goal.md` is the acceptance plan; unchecked tasks remain open. This work added
and verified native JSONL writer ownership/conflict handling, bounded compaction
and cancellation behavior, coherent model selection, session utilities, export,
and usage projections. No commits, pushes, releases, credential access, live
providers, TypeScript runtime/config access, or user configuration changes were
made.

### Latest completed checklist evidence

- **T01.6 — Branch/fork correctness:** added `u64::MAX` cursor assertions for
  both directions; oldest-first returns an empty page, while newest-first returns
  the full branch in reverse order. Existing tests cover inclusive/directional
  bounds, limits, invalid targets, broken ancestry, fork copies/leaf/labels,
  and live fork reopen. Native active-branch export reopens with compaction-aware
  context and excludes abandoned messages. Verification: session-state 7 passed;
  live fork/reopen 1; fork publication 2; native export round-trip 1; context/
  message-order 37.
- **T02.1 — Retry admission:** added a bounded-attempt regression. Existing
  faux-provider coverage verifies safe transient recovery, refusal to retry a
  partial tool call or context overflow, no repeated completed tool effect,
  generation-scoped safety reset, cancellation during backoff, salvage of a
  started retry, and exact retry limits. Verification: retry-safety unit 2;
  coding retry integration 4; context/message-order 37; full harness run e2e 38.
- **T04.3 — Discovery/Unicode:** added Unicode preview truncation and fuzzy-search
  assertions, and made the newest-first metadata comparator directly testable for
  deterministic path ties. Existing coverage verifies custom-root/cwd filtering,
  absent-root noncreation, malformed/unreadable/disappearing-file isolation,
  actionable root errors, recognizable bounded previews and recent ordering.
  Verification: CLI session summaries 4; resume picker 3; fuzzy suite 9; JSONL
  repository 14; unreadable/root-error and disappearing-file regressions 2.
- **T04.4 — Usage/occupancy:** verified embedded usage totals across compaction and
  abandoned branches, separation from native usage records, malformed usage
  rejection, active-branch context, and measured/estimated/unknown values.
  Focused usage tests 3 and context tests 3 passed.
- **T04.5 — Export/share scope:** verified active-branch display/share versus
  all-entry raw JSONL archive; native v4 active-branch export reopens with
  checkpoint context, refuses overwrite, and preserves source bytes. Stubbed
  gist/clipboard tests exclude abandoned content and cover fallback behavior.
  Export tests 7 and share transport tests 2 passed; no real transport/clipboard
  invoked and no secret-redaction guarantee is implied.

`cargo fmt --all -- --check` and `git diff --check` pass after the latest edits.
The checklist has **31 completed and 1 deferred** item. T05.2 is deferred at
user direction and is not counted complete; this is not overall goal completion. T00.1's inventory, T00.2's G1–G9 outcome map and T00.4's bounded
D1–D7 decisions are recorded in `goal.md`; these do not approve shipping or
deleting flagged additions. T02.6 is complete for the supported entry-point
contracts: RPC rejects image-bearing and concurrent prompts explicitly, text
survives overflow recovery, and a deterministic TUI regression queues an image
follow-up while automatic compaction is blocked, then verifies delivery and one
durable append. T05.2 has a detailed API/schema and scope review with explicit exclusion
recommendations in `goal.md` D7, but delivery disposition remains open. Basic
G8 usage/context reporting and support for already-stored branch-summary entries
are in scope; detailed cost/context APIs, scanner search, SQLite FTS, new
branch-summary generation/inert settings, and the unrelated `AGENTS.md` language
rule are recommended out of this focused deliverable. All flagged work remains
preserved locally; SQLite FTS is opt-in at search time but then maintained by
writes. Rust 1.78 and Windows runtime
behavior remain unverified; T05.2 is deferred at user direction. Its scope audit
is recorded, but no keep/exclude delivery decision was made. All worktree changes
are preserved, and no further scope input is requested in this session. T05.4's
local failure-path, artifact and status review is recorded in `goal.md`; it does
not approve shipping deferred additions.
The map covers native ownership/replay
and SQLite fencing (G1), branch/fork (G2), compaction (G3), automatic retries
(G4), queue/cancellation/session replacement (G5), model/resume (G6), discovery
and native-session import (G7), basic usage and active-context reporting (G8),
and export/share scope (G9). Public API review flagged unconsumed
repository-wide scanning/FTS search, detailed context-breakdown/cost-attribution
helpers and branch-summary generation; their delivery disposition is deferred at
user direction. No files were removed or staged. The latest full
warning-as-error workspace check passed. The parallel test run had one
process-global `ICERS_MAX_TURNS_PER_RUN` race; the affected test passed alone,
and the serialized workspace gate passed with 2,142 passed, 0 failed, 5 ignored
across 80 result summaries. Warning-as-error harness
checks previously exposed seven unrelated `ice-ai/src/strict_schema.rs` dead-code
warnings under narrow feature sets. All work remains uncommitted; unrelated dirty
work and `.serena/` were preserved.

### Queue identity and durable cancellation

T02.3 is complete in `goal.md`. The new restart regression queues two equal-text
messages with distinct IDs and valid, different image payloads, reopens the
session, and verifies both duplicate text values and image attachments return in
order from `clear_queue`. A deterministic race pauses at the queued message's
`MessageStart` after drain: cancellation returns `AlreadyConsumed`, then the
reserved entry commits exactly once. The injected cancellation-persistence
failure leaves its enqueue pending and writes no cancellation record. Existing
post-run clear/reopen, restart and cancel/reducer checks also pass. The complete
`context_message_order` suite passed (42 tests); three focused
`harness_run_e2e` queue tests passed.

### Cancellation settlement follow-up

T02.4 is complete in `goal.md`: existing harness tests distinguish cancellation
request from settlement during retry/backoff and summary generation, before
checkpoint publication and after acknowledged checkpoint writes; they assert
manual/automatic outcomes and required final-record failures. The TUI no longer
sets `Idle` in `AgentEnd`; only `run_prompt_streaming` makes that transition
after the harness operation returns and its durable operation outcome is
settled. The new event-handler regression passed, as did the CLI library suite
(617 passed, 2 ignored), the 40-test
`context_message_order` suite, and compaction/retry/cut-point suites (23 + 4 + 8
passed). A delayed `!command` flush race and full UI/RPC entry-point matrix are
not claimed complete and remain under T02.6/final review.

### RPC/TUI prompt and queue contracts

T02.6 is complete in `goal.md` for the supported entry-point contracts. RPC has
no queue command, rejects a second active prompt, and rejects image-bearing
prompts explicitly; it does not silently drop image content. Faux-provider
coverage verifies a later text prompt succeeds and an accepted text prompt
survives automatic overflow compaction in both checkpoint retained tail and
retry context. TUI follows its queue contract: a deterministic provider barrier
holds automatic compaction while an image follow-up is enqueued; after release,
the follow-up reaches the provider with its image and is persisted once.
Verification: RPC mode subset 13 passed; coding compaction policy suite 25
passed including the in-flight-compaction image-follow-up regression; formatting
and whitespace checks passed. This records deliberate protocol differences,
not identical TUI/RPC features or full TypeScript preflight parity.

### Commit-aware compaction cancellation

T03.6 is complete in `goal.md`. The provider test seam now supports deterministic
cancellation before a summary response; the new manual-compaction test confirms
an aborted outcome, no checkpoint, and an aborted operation record. Existing
coding-policy tests cover cancellation after a valid response but before commit,
after acknowledged checkpoint persistence, mid-run recovery, failed summaries,
and explicit pause. TUI outcome mapping distinguishes completed, declined,
aborted, and failed results. Verification: `coding_compaction_policy` (25
passed, including the active-compaction queue case), explicit-pause test (1
passed), TUI outcome-mapping test (1 passed), formatting and whitespace checks
passed. T02.6 separately records supported RPC/TUI differences and coverage.

### Compaction policy review

T03.1 is complete: the generic embedded-harness reserve trigger remains
unchanged; the coding CLI keeps its configurable strict-greater-than threshold;
summary validation does not require a heading; explicit pause is a visible
non-success outcome and the default safety-net continues. Verification passed:
`compaction::settings::tests` (11), plus exact-threshold, generic after-prompt,
and explicit-pause tests (3). This records a Rust product policy, not a claim of
TypeScript parity.

### Compacted-history restart verification

T03.7 is complete in `goal.md`. A native v4 JSONL temp-file test releases and
reloads a compacted session, confirms the original pre-compaction entry and
checkpoint ID remain available, verifies rebuilt main-branch summary/tail and
matched tool-call/result order, then continues with a new faux-provider tool
turn and validates the durable record log. The focused regression and the full
39-test `context_message_order` suite passed.

### Session replacement isolation

T02.5 is complete in `goal.md`: switching rejects while an operation is active
without replacing the old session; a later switch succeeds after settlement;
source/target queues, interrupted resumes and late writes remain session-scoped.
A temp JSONL target with a different stored cwd confirms existing tools keep the
harness launch-root cwd across storage switching. The full 39-test
`context_message_order` suite passed.

### Documentation verification

T05.3 is complete in `goal.md`. The user guide documents cancellation versus
settlement, checkpoint persistence and recovery/conflict limits, model/default/
resume behavior, usage estimate uncertainty, and active-branch versus archive/
export/share scope. `sh scripts/sync-embedded-docs.sh` completed; the source and
embedded guide compare byte-for-byte and share SHA-256
`3aa2e13442fdab1d23a8b8f7a46b45514a3ce6ddf83f7adb52e0b5425ace0079`.
