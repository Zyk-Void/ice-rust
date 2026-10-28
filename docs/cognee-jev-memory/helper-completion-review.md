# Helper completion review

## Implemented controls

- The helper enable flag defaults on, including for older configs that omit `helperEnabled`; explicit saved/environment `false` remains off. This does not auto-configure service access: provider/model binding stays explicit, and unsupported protocols or missing bindings remain nonfatal without chat fallback or network retries. Manual/canary write policies suppress helper dispatch.
- Production admission uses one process-wide `SharedHelperBudget`, keyed by a SHA-256 digest of the configured provider identity (never credentials). Defaults are process concurrency 2, provider concurrency 1, 6 requests/provider/minute, 12 requests/process/minute, with the existing per-session 2/minute gate. All admission is try-only. Provider history is capped at 32 identities; eviction only removes identities with no active permit and no rolling-window requests. Canceled/panicked flights release owned permits by RAII. Session/rebind cancellation does not reset budget history.
- The in-memory batch cache is capped at 128 FIFO entries, with 10-minute judged and 1-minute abstention TTLs. Cache identity includes provider/model, active dataset and context generation, sorted candidate ID/text hash/source entry/category/scope/branch/supersedes/expiry. Cached verdicts do not consume quota. Source/config/task changes naturally produce new keys.
- Helper work is candidate-revision driven. Successful proposal-tool completion marks a revision; run-settled events only dispatch an unseen revision and perform no candidate-directory scan on unchanged revisions. Reads are capped at 32 records and source context comes only from bounded current-branch excerpts named by candidates, never the latest prompt. Outbound candidate/context text passes the existing redactor.
- Application requires explicit durable `Yes` and applicable `Yes`; `Uncertain`, unknown IDs, and explicit `Category::None` do not apply. Integrated `accept_validated_candidate` validates current source and task/session ownership again before receipt-backed acceptance, including cached judgments.
- Helper observations use one UUID captured for the scheduled logical operation, retain elapsed request latency, and report separate requested/judged/abstained/cached/busy/rate-limited/stale/application-failed/persisted fields. Candidate text and prompts are not included in helper observation metadata.

## Verification

- `CARGO_BUILD_JOBS=4 cargo test -p ice-cli --lib --locked cognee:: --no-fail-fast --quiet`: **173 passed, 0 failed, 0 ignored** (1,253 filtered out at the time of this run).
- Admission tests inject isolated `Arc<SharedHelperBudget>` instances and cover provider/global rolling quotas, bounded FIFO cache, negative TTL, and stale-flight ownership. No network/model/Cognee request or real user configuration access was used.
- Formatting was applied only to owned `helper_gate.rs`, `helper.rs`, and `runtime.rs` using `rustfmt --config skip_children=true`.

## Remaining independent gates

- Fresh shared-acceptance validation is integrated in `440edddc` and `6297a800`. The helper branch's remaining helper/config differences are formatting or test-only visibility; its older runtime does not include later receipt and source safety, so it must not replace the integrated runtime.
- JEV quality/effectiveness and latency remain **UNVERIFIED**. No quality or performance claim is made; the enabled-by-default flag is not endpoint authorization and missing binding results in no dispatch.

## Worktree note

`crates/ice-cli/src/cognee/coordinator.rs` had a pre-existing rustfmt-only working-tree diff before this slice and is intentionally excluded from staging/commit.
