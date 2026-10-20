# Helper migration review

This preparation review records the current helper roles and exact transformation rules. Execution status is established by `lane-exports-rpc-ai-auth-migration-result.json` and `relocated-helper-validation.json`, not by this review.

## Boundaries and current evidence

The actual original tree contains 135 pre-existing audit helpers plus the five recovered migration modules. Two narrowly scoped support modules were added: the original-only plan preparer and the active-only relocated validator. The resulting explicit inventory is 142 helpers. The move map relocates all 36 root and 22 evidence helpers into `tools/`; 84 helpers already reside there after adding support.

Reviewed rules are saved in `lane-exports-rpc-ai-auth-migration-rules.json`. The mandatory explicit plan is `helper-migration-plan.json`; it contains each source and destination, full before and after hashes, exact original and replacement lines, added imports, completion guards, support dependencies, and every unchanged independent-review file hash. The final plan is generated only after the coordinator freezes helper edits. The transactional runner rejects a draft plan.

The current rules produce 73 exact line patches in 64 helpers, 93 changed helper bodies, and 61 new completion guards: 52 unconditional guards for historical writers and nine guards restricted to mutation modes. Existing completion checks on 15 historical helpers are retained. Six current helpers retain their own phase and mode gates: the final index builder, active coordinator, whole-tree relocator, metadata finalizer, Rust citation remapper, and spacing normalizer.

No application source, test, build, network, installation, branch or commit operation is part of this migration. Historical mutation helpers are parsed but never executed for validation.

## Root and import decisions

Audit files are resolved from the module location using the pure `lane-exports-rpc-ai-auth-audit-paths.mjs` exports. Generic native readers (`audit-util`, `direct-source`, `inspect-spec`, `read-evidence`) use the native checkout containing the audit. Existing TS roots remain unchanged.

Historical comparison and frozen-evidence readers deliberately retain the original Rust root and/or the explicit old/active pair. This includes `active-checkout-compare.mjs`, the extension and TUI frozen review readers, the historical baseline/surface generators, and the old canonical read-coverage generator. Moving these files does not reinterpret their historical source evidence as active evidence. Their audit output paths still follow the copied tree.

Relative imports between author helpers remain valid because both importer and dependency move into the same `tools/` directory. Direct invocations and historical helper-rewrite targets are updated to `tools/`. The common path and guard modules introduce no application or third-party dependency. The current index/spacing integration is owned by the coordinator and retains its reviewed pure import behavior.

The historical `verify-defect-provenance.mjs` is a canonical writer despite its name, so it receives an unconditional completion guard. Read-only inspector modes remain callable after completion. `coordinator-active-closeout.mjs complete-checkpoint`, `build-final-index.py complete`, `finalize-audit-metadata.mjs`, and the spacing helper's pure export/current validation modes retain their existing phase-specific behavior. The remapper bytes are preserved.

## Per-script map and role review

The final plan converts the reviewed token operations below into exact whole-line patches. Operation counts are not line-patch counts: multiple operations can affect one line, and one operation can affect two independently recorded lines.

| Original path | Final path | Role | Completion behavior | Reviewed path operations |
|---|---|---|---|---:|
| `audit-util.mjs` | `tools/audit-util.mjs` | mixed historical writer and inspector | conditional-prefix (record-reads) | 2 |
| `author-agent-common.mjs` | `tools/author-agent-common.mjs` | historical audit writer | unconditional-prefix | 1 |
| `author-agent-env.mjs` | `tools/author-agent-env.mjs` | historical audit writer | unconditional-prefix | 0 |
| `author-agent-loop.mjs` | `tools/author-agent-loop.mjs` | historical audit writer | unconditional-prefix | 0 |
| `author-agent-resources.mjs` | `tools/author-agent-resources.mjs` | historical audit writer | unconditional-prefix | 0 |
| `author-agent-runtime.mjs` | `tools/author-agent-runtime.mjs` | historical audit writer | unconditional-prefix | 0 |
| `author-blackhole-rest.mjs` | `tools/author-blackhole-rest.mjs` | historical audit writer | unconditional-prefix | 1 |
| `author-blackhole.mjs` | `tools/author-blackhole.mjs` | historical audit writer | unconditional-prefix | 1 |
| `author-cognee-client.mjs` | `tools/author-cognee-client.mjs` | historical audit writer | unconditional-prefix | 1 |
| `author-cognee-runtime.mjs` | `tools/author-cognee-runtime.mjs` | historical audit writer | unconditional-prefix | 1 |
| `author-extensions-common.mjs` | `tools/author-extensions-common.mjs` | historical audit writer | unconditional-prefix | 1 |
| `author-extensions-dispatch.mjs` | `tools/author-extensions-dispatch.mjs` | historical audit writer | unconditional-prefix | 0 |
| `author-extensions-loader.mjs` | `tools/author-extensions-loader.mjs` | historical audit writer | unconditional-prefix | 0 |
| `author-safe-verify.mjs` | `tools/author-safe-verify.mjs` | historical audit writer | unconditional-prefix | 1 |
| `compare-blackhole-copies.mjs` | `tools/compare-blackhole-copies.mjs` | historical audit writer | unconditional-prefix | 1 |
| `direct-source.mjs` | `tools/direct-source.mjs` | mixed historical writer and inspector | conditional-prefix (read, integrity) | 2 |
| `evidence/active-checkout-compare.mjs` | `tools/active-checkout-compare.mjs` | read-only inspector | none-read-only | 1 |
| `evidence/ai-schema-read.mjs` | `tools/ai-schema-read.mjs` | read-only inspector | none-read-only | 1 |
| `evidence/cli-packaging-supplement-read.mjs` | `tools/cli-packaging-supplement-read.mjs` | read-only inspector | none-read-only | 1 |
| `evidence/extensions-coverage-gaps.mjs` | `tools/extensions-coverage-gaps.mjs` | read-only inspector | none-read-only | 1 |
| `evidence/extensions-final-review.mjs` | `tools/extensions-final-review.mjs` | read-only inspector | none-read-only | 1 |
| `evidence/extensions-review-apply.mjs` | `tools/extensions-review-apply.mjs` | mixed historical writer and inspector | conditional-prefix (apply) | 1 |
| `evidence/extensions-review-read.mjs` | `tools/extensions-review-read.mjs` | read-only inspector | none-read-only | 1 |
| `evidence/qa-observability-independent-helper.mjs` | `tools/qa-observability-independent-helper.mjs` | read-only inspector | none-read-only | 0 |
| `evidence/semantic-dependency-read.mjs` | `tools/semantic-dependency-read.mjs` | read-only inspector | none-read-only | 1 |
| `evidence/subagents-read.mjs` | `tools/subagents-read.mjs` | read-only inspector | none-read-only | 1 |
| `evidence/tools-read-metadata.mjs` | `tools/tools-read-metadata.mjs` | read-only inspector | none-read-only | 1 |
| `evidence/tools-read.mjs` | `tools/tools-read.mjs` | read-only inspector | none-read-only | 1 |
| `evidence/tui-candidates-read.mjs` | `tools/tui-candidates-read.mjs` | read-only inspector | none-read-only | 1 |
| `evidence/tui-closeout-build-proposal.mjs` | `tools/tui-closeout-build-proposal.mjs` | historical audit writer | unconditional-prefix | 1 |
| `evidence/tui-closeout-inspect.mjs` | `tools/tui-closeout-inspect.mjs` | read-only inspector | none-read-only | 1 |
| `evidence/tui-closeout-merge-spec.mjs` | `tools/tui-closeout-merge-spec.mjs` | historical audit writer | unconditional-prefix | 1 |
| `evidence/tui-closeout-normalize.mjs` | `tools/tui-closeout-normalize.mjs` | read-only inspector | none-read-only | 1 |
| `evidence/tui-closeout-read.mjs` | `tools/tui-closeout-read.mjs` | read-only inspector | none-read-only | 1 |
| `evidence/tui-owned-evidence-read.mjs` | `tools/tui-owned-evidence-read.mjs` | read-only inspector | none-read-only | 1 |
| `evidence/tui-publication-readonly-review.mjs` | `tools/tui-publication-readonly-review.mjs` | read-only inspector | none-read-only | 1 |
| `evidence/tui-review-read.mjs` | `tools/tui-review-read.mjs` | read-only inspector | none-read-only | 1 |
| `evidence/tui-terminal-read.mjs` | `tools/tui-terminal-read.mjs` | read-only inspector | none-read-only | 1 |
| `extract-surfaces.mjs` | `tools/extract-surfaces.mjs` | historical audit writer | unconditional-prefix | 1 |
| `finalize-agent.mjs` | `tools/finalize-agent.mjs` | historical audit writer | unconditional-prefix | 0 |
| `finalize-blackhole.mjs` | `tools/finalize-blackhole.mjs` | historical audit writer | unconditional-prefix | 1 |
| `finalize-cognee.mjs` | `tools/finalize-cognee.mjs` | historical audit writer | unconditional-prefix | 1 |
| `finalize-extension-core-text.mjs` | `tools/finalize-extension-core-text.mjs` | historical audit writer | unconditional-prefix | 0 |
| `finalize-safe-verify.mjs` | `tools/finalize-safe-verify.mjs` | historical audit writer | unconditional-prefix | 1 |
| `inspect-blackhole.mjs` | `tools/inspect-blackhole.mjs` | historical audit writer | unconditional-prefix | 1 |
| `inspect-evidence.mjs` | `tools/inspect-evidence.mjs` | read-only inspector | none-read-only | 1 |
| `inspect-spec.mjs` | `tools/inspect-spec.mjs` | read-only inspector | none-read-only | 2 |
| `inventory.mjs` | `tools/inventory.mjs` | historical audit writer | unconditional-prefix | 1 |
| `merge-progress.mjs` | `tools/merge-progress.mjs` | historical audit writer | unconditional-prefix | 1 |
| `prepare-workers.mjs` | `tools/prepare-workers.mjs` | historical audit writer | unconditional-prefix | 1 |
| `publish-area.mjs` | `tools/publish-area.mjs` | historical audit writer | unconditional-prefix | 3 |
| `read-evidence.mjs` | `tools/read-evidence.mjs` | read-only inspector | none-read-only | 1 |
| `refine-blackhole-evidence.mjs` | `tools/refine-blackhole-evidence.mjs` | historical audit writer | unconditional-prefix | 2 |
| `render-area.mjs` | `tools/render-area.mjs` | historical audit writer | unconditional-prefix | 1 |
| `repair-extensions-dispatch.mjs` | `tools/repair-extensions-dispatch.mjs` | historical audit writer | unconditional-prefix | 1 |
| `resolve-extension-test-refs.mjs` | `tools/resolve-extension-test-refs.mjs` | historical audit writer | unconditional-prefix | 3 |
| `resume-integrity.py` | `tools/resume-integrity.py` | safe validation/report writer | none-validation-only | 0 |
| `supplement-surfaces.mjs` | `tools/supplement-surfaces.mjs` | historical audit writer | unconditional-prefix | 1 |
| `tools/accept-canonical-read-coverage.mjs` | `tools/accept-canonical-read-coverage.mjs` | historical audit writer | existing-completion-guard | 0 |
| `tools/accept-final-area.mjs` | `tools/accept-final-area.mjs` | historical audit writer | existing-completion-guard | 1 |
| `tools/append-native-queue-defect.mjs` | `tools/append-native-queue-defect.mjs` | historical audit writer | unconditional-prefix | 0 |
| `tools/apply-auth-json-precision.mjs` | `tools/apply-auth-json-precision.mjs` | historical audit writer | unconditional-prefix | 0 |
| `tools/apply-native-command-provenance.mjs` | `tools/apply-native-command-provenance.mjs` | historical audit writer | unconditional-prefix | 0 |
| `tools/build-final-index.py` | `tools/build-final-index.py` | current gated closeout helper | existing-current-gates | 0 |
| `tools/clean-session-audit-aliases.mjs` | `tools/clean-session-audit-aliases.mjs` | historical audit writer | unconditional-prefix | 0 |
| `tools/coordinator-active-closeout.mjs` | `tools/coordinator-active-closeout.mjs` | current gated closeout helper | existing-current-gates | 0 |
| `tools/coordinator-build-root-export-proposal.mjs` | `tools/coordinator-build-root-export-proposal.mjs` | historical audit writer | unconditional-prefix | 0 |
| `tools/coordinator-checkpoint.mjs` | `tools/coordinator-checkpoint.mjs` | historical audit writer | unconditional-prefix | 0 |
| `tools/coordinator-closeout-inspect.mjs` | `tools/coordinator-closeout-inspect.mjs` | mixed historical writer and inspector | conditional-prefix (declarations, source) | 0 |
| `tools/coordinator-complete-original-publication.mjs` | `tools/coordinator-complete-original-publication.mjs` | historical audit writer | existing-completion-guard | 0 |
| `tools/coordinator-fast-preflight.py` | `tools/coordinator-fast-preflight.py` | safe validation/report writer | none-validation-only | 0 |
| `tools/coordinator-inspect-relocation.mjs` | `tools/coordinator-inspect-relocation.mjs` | read-only inspector | none-read-only | 0 |
| `tools/coordinator-merge-exports.mjs` | `tools/coordinator-merge-exports.mjs` | historical audit writer | existing-completion-guard | 0 |
| `tools/coordinator-normalize-tui-metadata.mjs` | `tools/coordinator-normalize-tui-metadata.mjs` | historical audit writer | existing-completion-guard | 1 |
| `tools/coordinator-prepare-native-prose.mjs` | `tools/coordinator-prepare-native-prose.mjs` | historical audit writer | unconditional-prefix | 0 |
| `tools/coordinator-publish-closeout.mjs` | `tools/coordinator-publish-closeout.mjs` | historical audit writer | existing-completion-guard | 1 |
| `tools/coordinator-record-native-hunks.mjs` | `tools/coordinator-record-native-hunks.mjs` | historical audit writer | unconditional-prefix | 0 |
| `tools/coordinator-relocate-audit.mjs` | `tools/coordinator-relocate-audit.mjs` | current gated closeout helper | existing-current-gates | 0 |
| `tools/extend-relocation-map.mjs` | `tools/extend-relocation-map.mjs` | historical audit writer | existing-completion-guard | 0 |
| `tools/finalize-audit-metadata.mjs` | `tools/finalize-audit-metadata.mjs` | current gated closeout helper | existing-current-gates | 0 |
| `tools/fix-spacing.mjs` | `tools/fix-spacing.mjs` | current gated closeout helper | existing-current-gates | 0 |
| `tools/inspect-authored.mjs` | `tools/inspect-authored.mjs` | read-only inspector | none-read-only | 0 |
| `tools/inspect-closeout.mjs` | `tools/inspect-closeout.mjs` | read-only inspector | none-read-only | 0 |
| `tools/inspect-final-structure.mjs` | `tools/inspect-final-structure.mjs` | read-only inspector | none-read-only | 0 |
| `tools/inspect-global-audit.mjs` | `tools/inspect-global-audit.mjs` | read-only inspector | none-read-only | 0 |
| `tools/inspect-prose-queues.mjs` | `tools/inspect-prose-queues.mjs` | read-only inspector | none-read-only | 0 |
| `tools/integrate-ai-schema-supplement.mjs` | `tools/integrate-ai-schema-supplement.mjs` | historical audit writer | existing-completion-guard | 0 |
| `tools/lane-exports-rpc-ai-auth-final-inspect.mjs` | `tools/lane-exports-rpc-ai-auth-final-inspect.mjs` | read-only inspector | none-read-only | 1 |
| `tools/lane-exports-rpc-ai-auth-inspect.mjs` | `tools/lane-exports-rpc-ai-auth-inspect.mjs` | mixed historical writer and inspector | conditional-prefix (batch) | 0 |
| `tools/lane-exports-rpc-ai-auth-migration-inspect.mjs` | `tools/lane-exports-rpc-ai-auth-migration-inspect.mjs` | read-only inspector | none-read-only | 0 |
| `tools/lane-exports-rpc-ai-auth-native-metadata.mjs` | `tools/lane-exports-rpc-ai-auth-native-metadata.mjs` | read-only inspector | none-read-only | 1 |
| `tools/lane-exports-rpc-ai-auth-native-prose.mjs` | `tools/lane-exports-rpc-ai-auth-native-prose.mjs` | read-only inspector | none-read-only | 1 |
| `tools/lane-exports-sub-session-batch.mjs` | `tools/lane-exports-sub-session-batch.mjs` | historical audit writer | unconditional-prefix | 1 |
| `tools/lane-exports-sub-session-decls.mjs` | `tools/lane-exports-sub-session-decls.mjs` | read-only inspector | none-read-only | 1 |
| `tools/lane-exports-sub-session-inspect.mjs` | `tools/lane-exports-sub-session-inspect.mjs` | mixed historical writer and inspector | conditional-prefix (source) | 1 |
| `tools/lane-exports-sub-session-match.mjs` | `tools/lane-exports-sub-session-match.mjs` | read-only inspector | none-read-only | 1 |
| `tools/lane-exports-sub-session-native-hunks.mjs` | `tools/lane-exports-sub-session-native-hunks.mjs` | read-only inspector | none-read-only | 1 |
| `tools/lane-exports-sub-session-native-prose.mjs` | `tools/lane-exports-sub-session-native-prose.mjs` | read-only inspector | none-read-only | 1 |
| `tools/lane-final-areas-active-identity-proposal.mjs` | `tools/lane-final-areas-active-identity-proposal.mjs` | historical audit writer | unconditional-prefix | 0 |
| `tools/lane-final-areas-active-identity.mjs` | `tools/lane-final-areas-active-identity.mjs` | mixed historical writer and inspector | conditional-prefix (candidates, first-pass-findings) | 0 |
| `tools/lane-final-areas-active-noncrates-review.mjs` | `tools/lane-final-areas-active-noncrates-review.mjs` | historical audit writer | unconditional-prefix | 0 |
| `tools/lane-final-areas-active-review.mjs` | `tools/lane-final-areas-active-review.mjs` | read-only inspector | none-read-only | 0 |
| `tools/lane-final-areas-build-misc.mjs` | `tools/lane-final-areas-build-misc.mjs` | historical audit writer | unconditional-prefix | 0 |
| `tools/lane-final-areas-exports.mjs` | `tools/lane-final-areas-exports.mjs` | mixed historical writer and inspector | conditional-prefix (review) | 0 |
| `tools/lane-final-areas-first-pass-findings-proposal.mjs` | `tools/lane-final-areas-first-pass-findings-proposal.mjs` | historical audit writer | unconditional-prefix | 0 |
| `tools/lane-final-areas-inspect.mjs` | `tools/lane-final-areas-inspect.mjs` | read-only inspector | none-read-only | 0 |
| `tools/lane-final-areas-prepare.mjs` | `tools/lane-final-areas-prepare.mjs` | mixed historical writer and inspector | conditional-prefix (tui-qa) | 0 |
| `tools/lane-final-areas-read-coverage-inspect.mjs` | `tools/lane-final-areas-read-coverage-inspect.mjs` | read-only inspector | none-read-only | 0 |
| `tools/lane-final-areas-read-coverage-verify.mjs` | `tools/lane-final-areas-read-coverage-verify.mjs` | safe validation/report writer | none-validation-only | 0 |
| `tools/lane-final-areas-read-coverage.mjs` | `tools/lane-final-areas-read-coverage.mjs` | historical audit writer | unconditional-prefix | 1 |
| `tools/lane-final-areas-read-ranges.mjs` | `tools/lane-final-areas-read-ranges.mjs` | historical audit writer | unconditional-prefix | 1 |
| `tools/lane-final-areas-readme-limits-review.mjs` | `tools/lane-final-areas-readme-limits-review.mjs` | safe validation/report writer | none-validation-only | 0 |
| `tools/lane-final-areas-remap-review.mjs` | `tools/lane-final-areas-remap-review.mjs` | safe validation/report writer | none-validation-only | 0 |
| `tools/lane-final-areas-type-contracts.mjs` | `tools/lane-final-areas-type-contracts.mjs` | historical audit writer | unconditional-prefix | 1 |
| `tools/lane-final-areas-verify-exports.mjs` | `tools/lane-final-areas-verify-exports.mjs` | safe validation/report writer | none-validation-only | 1 |
| `tools/lane-final-areas-verify.mjs` | `tools/lane-final-areas-verify.mjs` | read-only inspector | none-read-only | 0 |
| `tools/lookup-read-credit.mjs` | `tools/lookup-read-credit.mjs` | read-only inspector | none-read-only | 0 |
| `tools/merge-cli-fragments.mjs` | `tools/merge-cli-fragments.mjs` | historical audit writer | unconditional-prefix | 0 |
| `tools/normalize-citations.mjs` | `tools/normalize-citations.mjs` | historical audit writer | existing-completion-guard | 0 |
| `tools/normalize-final-handoff.mjs` | `tools/normalize-final-handoff.mjs` | historical audit writer | existing-completion-guard | 0 |
| `tools/normalize-final-schema.mjs` | `tools/normalize-final-schema.mjs` | historical audit writer | existing-completion-guard | 0 |
| `tools/prepare-citation-normalizer.mjs` | `tools/prepare-citation-normalizer.mjs` | historical audit writer | existing-completion-guard | 0 |
| `tools/qa-first-pass-read.mjs` | `tools/qa-first-pass-read.mjs` | read-only inspector | none-read-only | 0 |
| `tools/read-audit-json-part.mjs` | `tools/read-audit-json-part.mjs` | read-only inspector | none-read-only | 0 |
| `tools/reconcile-final-dependencies.mjs` | `tools/reconcile-final-dependencies.mjs` | historical audit writer | existing-completion-guard | 0 |
| `tools/reconcile-final-first-pass.mjs` | `tools/reconcile-final-first-pass.mjs` | historical audit writer | existing-completion-guard | 0 |
| `tools/reconcile-first-pass-records.mjs` | `tools/reconcile-first-pass-records.mjs` | historical audit writer | unconditional-prefix | 0 |
| `tools/reconcile-readability.mjs` | `tools/reconcile-readability.mjs` | historical audit writer | existing-completion-guard | 0 |
| `tools/remap-rust-citations.mjs` | `tools/remap-rust-citations.mjs` | current gated closeout helper | existing-current-gates | 0 |
| `tools/repair-agent57-literal.mjs` | `tools/repair-agent57-literal.mjs` | historical audit writer | unconditional-prefix | 0 |
| `tools/review-spacing.mjs` | `tools/review-spacing.mjs` | read-only inspector | none-read-only | 0 |
| `tools/verify-area-evidence.mjs` | `tools/verify-area-evidence.mjs` | read-only inspector | none-read-only | 0 |
| `tools/verify-complete-audit.py` | `tools/verify-complete-audit.py` | safe validation/report writer | none-validation-only | 0 |
| `tools/verify-defect-provenance.mjs` | `tools/verify-defect-provenance.mjs` | historical audit writer | unconditional-prefix | 0 |
| `tools/verify-supplemental-baseline.mjs` | `tools/verify-supplemental-baseline.mjs` | read-only inspector | none-read-only | 0 |

## Preparation and coordinator execution order

1. After the coordinator's helper freeze, run `node docs/ice-gap/tools/lane-exports-rpc-ai-auth-prepare-migration.mjs --freeze` in the original checkout.
2. Run `node docs/ice-gap/tools/lane-exports-rpc-ai-auth-migration-qa.mjs` in the original checkout. This validates the fresh exact helper inventory, transformations, import map, independent-review hashes, Node syntax, and Python AST syntax.
3. The coordinator preserves the passing original gate, copies the whole audit with `/bin/cp -a`, and verifies every copied file's hash.
4. The coordinator runs `node docs/ice-gap/tools/lane-exports-rpc-ai-auth-migrate-helpers.mjs --preview` and then `--apply` in the active checkout.
5. Current active closeout performs `render-pending` through the migrated `tools/render-area.mjs`.
6. Run `node docs/ice-gap/tools/lane-exports-rpc-ai-auth-relocated-validation.mjs` in the active checkout after active rendering. It checks actual migrated hashes, parses every helper, and runs the 16 current spec inspectors plus the current structural inspector and the visible-file local Git baseline validator.
7. Run the comprehensive active final gate with `/usr/bin/python3 -I docs/ice-gap/tools/verify-complete-audit.py active`, then follow the coordinator's existing completion metadata/index/final-checkpoint sequence and required final gate rerun.

Every Python parser subprocess uses `/usr/bin/python3 -I`. Parsing uses `ast.parse` from stdin and does not import audited modules or create bytecode. Residue checks cover the audit tree and repository-root `.venv`/`__pycache__` names after Python.

## Safe execution evidence

The actual relocated validator saves `relocated-helper-validation.json` with `passed`, `safe_execution_complete`, `issues`, per-helper syntax results, the 18 safe execution records, input hashes, receipt hash, and residue checks. It does not run historical authoring, publication, merge, read-credit, or defect-provenance generators. Its diagnostics are confined to that validation report.

The current per-area `verify-area-evidence.mjs` remains available for optional schema/proposal diagnosis. The comprehensive final gate is authoritative for current accepted documentation. Historical proposals are preserved as historical artifacts and are not rewritten merely to satisfy a stale proposal comparison.

## Transaction and rollback

The active runner requires the exact active real path and expected active Git HEAD, a preserved passing original source-only gate, a ready plan, matching preparation QA, all reviewed hashes, unique absent move destinations, a complete helper inventory and unchanged independent-review files. It stages transformed bytes, rechecks all inputs immediately before mutation, keeps exact original helper bytes in `lane-exports-rpc-ai-auth-migration-originals.json`, and records every applied output hash. Failure restores original helper bytes and the independent-review location before emitting a failure record.

The independent review's existing README and baseline move unchanged from `independent-review/` to `evidence/independent-review/`. Its unfinished status, findings and limitations remain untouched. Relocation provides no new independent semantic certification.

## Remaining verification authority

A prepared plan is not proof of applied relocation. The final hash-bound preparation QA, transactional receipt, actual relocated-helper report and comprehensive active final gate establish the completed state. The coordinator owns active writes and final publication metadata.

## Focused coordinator helper integration review

Reviewed on 2026-10-20T06:57:41.800Z from the original audit. The coordinator independently confirmed these exact four hashes and froze these helpers. No helper was edited in this review, and no application, test, build, historical mutator, or source-read credit operation ran.

| Helper | Reviewed SHA-256 | Integration result |
|---|---|---|
| `tools/build-final-index.py` | `ad67ecbb592a1135e4c84edf07fa3ffa77574df8944aedb63efbc02bab089c36` | Complete mode requires a passing active gate whose Rust root resolves to this containing checkout. The generated README passes through the pure normalizer imported by absolute helper URL. The generated final gate command explicitly uses `/usr/bin/python3 -I`. |
| `tools/verify-complete-audit.py` | `2a863d60b2f4439313e1f8a72feb046880801de61081b55c83b1c802c84739d5` | Native ROOT derives from the containing audit; the TypeScript root and explicit already-active alias remain deliberate. Residual validation requires positive applied, structure, citation, identifier and protected-span proofs, current tool and dry-run/approval hash binding, every reviewed change index, zero remaining glue, and a new read-only residual check on the current audit. |
| `tools/coordinator-checkpoint.mjs` | `8e36be058fbeabaaa38e3b30270ae3f7e168bdc797dee8e539a0d6f6ab489649` | Its original checkpoint requires the passing original gate, preserves that report, and leaves completion false. Only the new header and appended checkpoint table are normalized; the previous text from the first historical divider onward is retained byte for byte. The migration adds its reviewed historical-writer completion guard. |
| `tools/coordinator-relocate-audit.mjs` | `9cd00b53e87cd675cafc4a776b78729a1b1c0532f91e751383a4504589d67e44` | The old-only controller requires the preserved passing original source-only gate and the exact clean active identity, refuses an existing destination, and uses explicit `/bin/cp -a`. It rejects audit symlinks and dependency/Python residue, then compares the complete path/byte/hash inventory against both the unchanged source and copied destination before writing the active copy receipt. |

No concrete blocker was found in these four helpers for the coordinator's remaining sequence. This static review does not replace the original gate, final freeze-bound preparation QA, active copy receipt, relocated helper execution report, or final active gate. The active native `render-pending` step must run immediately after helper migration and before metadata finalization, because its pending record binds the native-apply spec hashes.
