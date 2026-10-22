# CLI package-management supplement — stable handoff

The source-only supplement is complete and worker-owned edits are finished. Coordinator review/publication remains required. Files: spec-cli-packaging-supplement.json, proposal-cli-packaging-supplement.json, cli-packaging-supplement-source-review.json and cli-packaging-supplement-qa.json.

## Coverage

27 entries CLI-020–046: 14 missing, 10 partial and 3 different-by-design. The three retention entries (040–042) have zero LOC; incremental work totals 3740–6285 Rust lines. Priority counts: 2 P0, 21 P1, 1 P2 and 3 P3. All 3 assigned ledger rows map to entries. All 26 mechanical candidates have one disposition: 2 real surface rows and 24 local-expression exclusions. There are 75 surfaces total, including 73 authored beyond the mechanical inventory.

Seven TS implementation/docs files and twelve TS suites were fully read. Five native implementation files including every inline test and one native integration suite were fully read. app.rs is limited to1–175 with connector-redacted line96 excluded. Two native package suites reuse previously verified AUTH/SUB reads and are not counted as new independent reads. Source hashes match the frozen coordinator manifest; added TS tests match the frozen TS Git revision. No tests, applications, installers, network commands, user-state operations or source edits were executed.

## Verified findings and limits

Five source-derived native findings: CLI-039 project-name/registry containment, CLI-043 duplicate work for a validated already-staged newer version, CLI-044 probe caps applied after full pipe collection, CLI-045 canonical repository case mismatch in the old-checkout prebuilt release path, CLI-046 PowerShell updatedAt versus Rust updatedAtMs status schema. No runtime reproduction is claimed. CLI-045 remains specific to the frozen old checkout; coordinator compares any active-checkout fix separately.

Final semantic checks preserve raw installed npm version truthiness, exact npm-view JSON selection, callback/error ordering and manifest/filter asymmetries. Broad surface references were replaced with precise declaration/branch anchors. Worker checks validate 262 source citation occurrences, 85 surface anchors, all schema fields, candidate conservation and no internal dependency cycles; zero errors. Cross-area dependencies are real stable IDs; coordinator validates the combined graph.

## Handoff

Root/audit_session owns integration and publication. This worker next performs independent first-pass coverage QA, then supplies bounded low-TUI defect/reachability candidates to root without editing the shared catalog.
