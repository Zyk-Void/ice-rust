# TUI audit — owner closeout released

The TUI owner work is complete and released to the coordinator for independent acceptance. This was a source-only audit. No application, build, test, install, network, Git/source modification or real user-profile access was performed.

## Current authoritative artifacts

| Artifact | SHA-256 |
| --- | --- |
| spec-tui.json | 6348be7e858ee5d22c78cfe04f6fb003b00b9f9ed55b66c0271d78995c38254d |
| proposal-tui.json | 9403cd7fd852645553548fb60bb10192b5a38386d66dc3fd24f088221e10de89 |
| tui-source-review.json | 0cf812a4eb70b5e0bbecdd5c0263a772305ef0cb51568274c37ee5d1c366e20f |
| tui-closeout-coverage.json | 625cbe2cf2931ea2e5081a33b508333302904ee579e8b582bf871772bb2dc81d |

The merged spec contains **151 entries**: TUI-001–084 and TUI-201–267. The matching proposal has **224 original ledger rows** and **1,151 surfaces**. The source manifest has **440 unique files** and **135,550 visibly credited lines** with exact ranges, hashes and reader attribution. The detailed validation and release record is tui-closeout-qa.json.

## Candidate and source closure

All **923 original mechanical leads** are explicitly disposed: **897 mapped, 26 excluded, zero pending**. The closeout added 376 mapped leads, 14 justified local-variable/preview-fixture exclusions and 75 explicit menu, command, path, signal and widget surfaces. Main and terminal entries are fully merged.

All assigned TypeScript files have complete visible coverage except seven explicit connector-redacted production lines. Every recorded relevant test body has full visible coverage except oauth-selector.test.ts line 54 and interactive-mode-anthropic-warning.test.ts line 19, which were connector-redacted. There are no ordinary required-source or test gaps. Native controller, tests, helpers and the coordinator's four documentation reads are merged with attribution.

Normalization removes every redacted or explicitly unverified line before taking the union, even where an old raw shared record called its file fully read. The current manifest retains **10 connector-redacted source/test lines** and the separate **native sessions.rs line 627 read-credit conflict**. The original TUI record skipped that line while accepted SESSION records claimed full coverage; the coordinator is reconciling provenance. No recovery of possibly redacted text was attempted.

status-line.test.ts and packages/tui/test/editor.test.ts were dirty at baseline without captured original working-tree hashes. Their current bodies are source-read and are not described as new drift.

## Validation and handoff

The read-only audit validator command `node docs/ice-gap/tools/verify-area-evidence.mjs tui` passed with **zero problems** across **2,441 unique source references**, 151 matching proposal entries and 440 read records. Candidate closure has no mapped/excluded overlap and no unresolved leads.

The proposal has `draft_complete: true`, `complete: false` and no owner open questions. The false complete flag is intentional: final independent acceptance and shared publication belong to /root. Root also owns global dependencies, first-pass report reconciliation, the defect catalog and final prose normalization.

The 13 new controller/widget entries incorporate the coordinator's independent review, full source-path/range expansion and exact native distinctions. TUI-073 remains extension widgets and TUI-078 is sharing. TUI-074 has TOOLS-004, source-exact steering/follow-up failures and separate internal/host history qualifications. TUI-005/055/058/059/083 dependency and scope corrections, plus native TUI-253/255 defects, are reflected in the final draft.

## Frozen baselines

- TypeScript: `3ab05193147f5abb8eb8ee6c0178f6dbce8918b0`.
- Rust: `ff9270f26e263ea9ce54349786bf2d93c4517476`.

All artifacts are under `/home/mewtwo/Zks/ice-rust/docs/ice-gap/evidence`. The stale-write-protected merge/build helpers were successfully consumed; their original input hashes intentionally no longer match after finalization.
