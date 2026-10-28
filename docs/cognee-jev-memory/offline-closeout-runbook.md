# Cognee selective-memory recovery and isolated rollout runbook

This is an operational boundary, not evidence of a live rollout. The closeout tests used temporary state, fake transports and offline providers. Existing user settings, datasets and live services were not modified.

## Effective modes

`/cognee mode manual` selects on-demand reads and manual writes. `/cognee mode canary` selects event-driven reads and manual writes. The canary/manual policy prevents automatic helper dispatch even when an earlier helper-enabled flag is retained in saved settings. Disable the helper explicitly before a test migration; do not assume the flag was erased. `/cognee mode selective` enables source-validated candidate admission; it does not prove that the real backend or JEV is useful. Legacy restores the older broad read/write behavior and is not the safest rollback mode.

Inspect effective environment overrides as well as saved settings. Explicit remember commands and previously authorized durable write intents are not erased by selecting manual mode. Disable Cognee entirely to prevent new dispatch. Optional automatic capture/improve/dream work is canceled on effective policy changes; the cancellation token is captured before scheduling so a quick disable/re-enable cannot revive an obsolete job.

## HTTP acceptance is not searchability

`remember_with_receipt` and `improve_with_receipt` retain a bounded optional job ID from a successful HTTP acknowledgment. They do not poll an invented status endpoint. The private `accepted-jobs/` ledger records operation ID, original dataset/session where known, acceptance time, optional handle and `searchability: unverified`.

Optional acknowledgment parsing is bounded to 4 KiB and 250 ms. Malformed, absent or slow acknowledgment bodies do not retroactively turn an accepted request into a new automatic retry. Existing `remember`/`improve` client methods remain compatible wrappers.

Local admitted facts may be available before remote indexing is known. They remain untrusted reference data: current instructions, repository files and live verification take precedence.

## Permanent-write and warmup states

Permanent admission receipts under `candidates/admissions/` are the local commit point; candidate/queue files are recoverable projections. A completed marker prevents a confirmed operation from being recreated for dispatch. Candidate terminal states cannot be rewritten by a stale helper snapshot.

Warmup cache records now distinguish Pending, InFlight and Uncertain. A directory dispatch lease serializes replay across processes. InFlight is persisted before an HTTP call. Successful removal is the local replay acknowledgment. Generic transport/server errors, cancellation after a possible send, or failed local acknowledgment preserve uncertainty. Only an explicit rejection is eligible for an ordinary automatic retry.

Old warmup files have no recorded outcome. They remain readable and preserved, but are treated as Uncertain rather than silently resent. An unknown state also fails closed. Uncertain records are skipped before selecting the bounded replay batch so they do not starve later genuinely Pending records.

The existing explicit uncertain permanent-queue flush is an operator decision that may duplicate a remotely accepted write. It is not proof of backend idempotency and is not a warmup reconciliation command. Do not hand-edit uncertain warmup files into Pending merely to clear the queue. Establish backend outcome or preserve the files for an explicit recovery procedure.

## Isolated test procedure

Use a new temporary `ICERS_CODING_AGENT_DIR`, a dedicated project/profile and a separate test dataset or confirmed inference-disabled adapter. Do not point a test process at the user's actual agent directory or historical datasets. Confirm every downstream model before making a service request; the current authorization allows only `cx/gpt-6-luna`, not a real JEV model.

Begin with helper dispatch off and manual writes. Confirm the resident capability inventory is unprobed rather than assuming it is connected. Exercise self-contained turns, explicit history, context compaction, mode changes and shutdown with observable request counts. Check accepted versus searchable separately. The committed fake-transport tests exercise these mechanics, but are not a representative user-effectiveness study.

For an authorized real canary, compare actual task correctness and repeated investigations against memory-off and on-demand baselines. Measure full time to first token and total task latency, including failed or unconsumed background work. Do not promote JEV based on the synthetic replay; its D arm was not executed.

## Safe rollback and mixed-version warning

1. Stop automatic/helper work and all processes sharing the test storage. Preserve logs and effective configuration.
2. Back up the entire relevant memory state together: configuration, pending queue, warmup files, candidate records, admission receipts, completion markers, suppression history and accepted-job records. Do not delete old datasets.
3. Prefer a mode rollback on the same compatible binary: helper disabled plus manual, or Cognee off. Verify that optional work stops and explicit tools retain their documented behavior.
4. **Do not point an older binary directly at the new warmup queue.** Old code may ignore the new state field and replay an Uncertain/InFlight record. A binary downgrade requires a separately reviewed isolated copy or compatible restoration, not merely replacing the executable.
5. Keep ambiguous outcomes ambiguous until the backend or operator reconciles them. Do not claim exactly-once delivery without a backend idempotency contract.

The temporary-profile mode/rollback dry-run passed. A real user-facing canary, mixed-version storage migration and production downgrade have not been performed and remain M7.3's acceptance gate.
