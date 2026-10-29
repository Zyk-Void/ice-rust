# Cognee durable completion review

## Admission receipt implementation

Selective candidate admission now uses a versioned `AdmissionReceipt` in `candidates/admissions/<id>.json` as the single-record commit point. The receipt contains the complete accepted candidate attribution, immutable pending remember intent, optional superseded-target source/scope/hash, and commit timestamp. Receipt IDs and paths are validated. The atomic private-file write is the only event that makes an admission accepted and locally recallable.

Before commit, admission acquires the short process-safe `.admission.lock`, checks committed receipts and legacy accepted candidates for exact dataset/category/branch/revision-and-validity deduplication, and reserves queue capacity under `.enqueue.lock` as `PendingState::Prepared` carrying the admission ID. Prepared entries are excluded from dispatch and orphan-inflight ambiguity recovery. Queue capacity exhaustion returns failure without evicting existing intents. A failed receipt write leaves the Prepared reservation blocked; it does not create an accepted local fact or suppression and cannot trigger HTTP. The admission lock is never held across network or model work; dispatch continues to use the separate `.dispatch.lock` ownership.

After receipt commit, candidate JSON and activated queue records are projections and may be rebuilt by `admission::reconcile`, which runs from actual runtime initialization before dispatch. Receipt-backed queue activation is idempotent. A remote success writes a receipt-derived completion marker before queue cleanup; reconciliation will not recreate a completed operation after queue-file deletion. If remote acknowledgement or its marker is uncertain, the intent remains ambiguous and is not auto-replayed. Existing `InFlight` records still mean wire activity may have occurred and recover to `Uncertain` only when the dispatch lease is not held.

`candidates::query_accepted` now accepts new records only from valid committed receipts; candidate projections with `admission_id` cannot prove acceptance. Legacy accepted candidate records with no admission ID remain readable. Correction/supersession is derived from the same committed receipt collection, so uncommitted changes cannot suppress old facts; remote graph history is never removed. Candidate and queue projections are repairable from receipts. Older candidate and pending-operation formats remain readable through serde defaults.

## Integrated source-validation boundary

Subsequent integration in `440edddc` and `6297a800` added current source, session, task, branch and repository validation before helper dispatch and again before acceptance. See `validation.rs`, `source_validation.rs`, and the helper admission tests in `runtime.rs`. Helper verdicts are advisory and cannot replace those checks; the receipt remains the admission commit point. The receipt implementation itself does not control helper scheduling or capability routing.

## Verification

Focused Cognee tests are run with a unique per-run Cargo target directory to avoid shared-target contamination. Receipt-specific failure/recovery cases cover blocked uncommitted Prepared state, receipt-backed projection replay, completion-marker no-resend behavior, and legacy candidate deserialization; runtime integration coverage exercises the production admission and dispatch paths.
