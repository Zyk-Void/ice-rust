# Independent review of the published gap audit

Status: IN PROGRESS. This is not certification of the audit. Astra's reports and evidence are not modified by this review.

## Scope and acceptance standard

Review each published entry against TypeScript implementation, relevant existing tests, original Rust implementation, and (separately) the active Rust checkout. For each area, also enumerate implementation paths and public contracts independently to search for omitted capabilities. Record concrete evidence and counterexamples; distinguish an implementation difference from an established defect or intentional product choice. Do not treat valid citation ranges as proof of semantic accuracy. Do not execute tests or application code without a separate decision.

`baseline.json` freezes the published entry IDs and specification hashes at review start. Reports can change concurrently; later conclusions must identify the reviewed revision. This directory is owned by the independent reviewer, not the audit publisher.

## Initial findings

### AGENT-002 — asynchronous stream factory

Verdict: core contract claim verified against the original Rust checkout; full entry acceptance pending caller/error/cancellation and existing-test review.

TypeScript `packages/agent/src/types.ts` defines the provider callable contract permitting a promised stream. Rust `crates/ice-agent/src/stream_fn.rs:21-41` defines a callable returning `AssistantMessageEventStream` synchronously. Its opening comment claims the TS callable is never awaited; that comment is not sufficient evidence of parity. A producer spawned behind a synchronously returned stream can bridge asynchronous work, but is not itself the same public callable signature. The audit correctly identifies a public SDK contract difference rather than claiming asynchronous providers are impossible in Rust.

### AGENT-003 — default stream replacement and clearing

Verdict: registry behavior claim verified; full entry acceptance pending constructor/fallback timing and export review.

TypeScript `packages/agent/src/stream-fn.ts:3-20` assigns every supplied function or undefined and throws when no default exists. Rust `crates/ice-agent/src/stream_fn.rs:44-77` stores the default in `OnceLock`; later Some assignments fail silently and None does nothing. The audit's replacement/clear gap is supported. The Rust explanatory comment suggesting the TS default can only be cleared before first use is misleading: the TS implementation contains no such restriction.

## Earlier broad screening, not semantic certification

596 published entries: no missing schema fields and no duplicate IDs. 4,177 distinct parsed citation ranges across 936 existing files. 372 ranges end exactly one line past physical EOF, with valid starts; no larger overrun or missing file was found by that screen. Twenty dependency edges target unpublished MISC/TUI entries. These results describe the original published state, not the active checkout or later changes.

Several high-risk claims were previously source-spot-checked (Unicode scanner, pipe draining, retry shift, environment gating, mutation registration, Cognee capture), but those spot checks do not constitute completed area reviews.

## Remaining work

- Finish AGENT-001 through AGENT-065, including source/test reading and independent omission search.
- Apply the same process to every other frozen published area.
- Independently compare original versus active Rust checkout and reassess affected verdicts.
- Record per-entry verdicts and per-area confidence; assign a final score only after coverage is explicit.

No full area has yet passed independent acceptance. No exhaustive rating is warranted at this checkpoint.
