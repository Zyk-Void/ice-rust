# Targeted quality check: agent area and extension draft

- Date: 2026-10-20
- Scope: read-only source comparison for AGENT-018, AGENT-049, AGENT-050 and AGENT-057; structural validation of the unpublished extension draft.
- Limit: This is a spot check, not a certification of the other 61 agent entries, their tests, the full ledger, or the entire extension area. No tests were executed, no network calls or agents were launched, and no source files were changed.

## Verified agent comparisons

| Entry | TypeScript evidence | Rust evidence | Verdict |
|---|---|---|---|
| AGENT-018 | `packages/agent/src/agent-loop.ts:224-245` applies context, model and thinkingLevel from `prepareNextTurn`. | `crates/ice-agent/src/agent_loop.rs:360-375` reads only `upd.context`. | Confirmed partial. |
| AGENT-049 | `packages/agent/src/harness/env/nodejs.ts:237-248` returns only explicit `extraEnv` when `inheritEnv=false`; call site `:413-424`. | `crates/ice-tools/src/os_env.rs:840-867` applies `self.shell_env` outside `inherit_env` condition. | Confirmed partial. The failure mode is possible configured-default environment disclosure, not a demonstrated credential leak. |
| AGENT-050 | `packages/agent/src/harness/env/nodejs.ts:364-497` registers output data handlers while the child is live. | `crates/ice-tools/src/os_env.rs:1035-1057` pins reader futures but awaits `status_fut` first, not polling reader futures during wait. | Confirmed source-level deadlock risk for a child exceeding OS pipe capacity. No process reproduction was run. |
| AGENT-057 | `packages/agent/src/harness/prompt-templates.ts:243-256` uses JavaScript string replacement and Unicode-preserving string handling. | `crates/ice-harness/src/prompt_templates.rs:393-419` casts individual UTF-8 bytes to `char` while copying literals. | Confirmed source-level non-ASCII corruption risk. |

All four confirmations are from the current checked-out source, not simply from identifiers or the first-pass report. The Rust fixes have not been implemented.

## Document precision notes

- `agent.md:5790` says the TypeScript lexer iterates “Unicode characters”; `packages/agent/src/harness/prompt-templates.ts:214-240` indexes JavaScript UTF-16 code units. It preserves the input string, but this is not the same as iterating Unicode scalar values or grapheme clusters. Consider tightening the wording.
- `agent.md:5792` contains `including0`; insert a space to conform to the writing-style requirement.
- The detailed specifications quote existing source; source hashes alone do not validate their interpretation.

## Extension staging

- `docs/ice-gap/evidence/spec-extensions.json` was modified at 2026-10-20 00:35:00 +03:00, after `agent.md` and `README.md` were last updated at approximately 00:03 +03:00.
- `node docs/ice-gap/inspect-spec.mjs extensions` returned: 34 entries, 248 unique line references, zero structural problems.
- These entries have not been merged into `README.md`, `ledger.csv` or the final per-surface inventory, and the extension area is not certified complete.
- Modification timestamps show subsequent drafting, not proof that an independent worker is still running.

## Remaining audit blockers

The published `evidence/progress.json` records 860 ledger rows: 84 mapped, 4 justified not-applicable, 772 pending; 200 published entries; 52 first-pass claims reviewed. Every remaining ledger row, surface row and unreviewed first-pass claim must still be reconciled under the original completion criteria.
