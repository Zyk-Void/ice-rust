# Low-level TUI stable handoff

Ready for independent review and merge by audit_extensions; shared publication remains coordinator-owned.

## Deliverables

- spec-tui-terminal.json: 67 detailed entries, TUI-201 through TUI-267 (61 partial, 6 missing; priorities 4 P0, 60 P1, 3 P2).
- proposal-tui-terminal.json: 41 original ledger files / 15,282 ledger lines; 264 single-anchor surfaces; 197 original candidates accounted for exactly once (185 candidate surfaces, 12 explicit mechanical exclusions), plus 79 authored surfaces.
- tui-terminal-source-review.json: 127 unique source/test/documentation records, 125 fully read; two bounded native CLI excerpts are supporting evidence only.

## Coverage and validation

All assigned production TS/C bodies, all 33 directly relevant TS test files, matching native primitives and both native renderer integration suites were read. The existing tests were not executed. Final author QA found no schema, candidate ownership, dependency-cycle or range errors: 528 surface anchors and 317 fully qualified spec references were checked against actual read metadata. After connector handles expired and the same authorized workspaces were reopened, a metadata-only check returned all 127 paths with unchanged hashes.

Current TS editor.test.ts is a fully read supplemental dirty snapshot (9 added lines, zero deleted against frozen Git); its historical baseline hash is retained separately. Connector-redacted expressions in utils.ts902, editor.ts2405 and markdown.ts245 were not recovered. Full native CLI controller coverage belongs to audit_extensions; this fragment reads interactive_tui.rs1680–1855 and interactive_tui/run.rs300–440 only, plus the entire autocomplete helper.

## Review focus

The source-derived priority defects include native wrapped RGB parameter handling, omitted Kitty chunk continuation, editor word/jump byte-scalar confusion and unsorted paste-map renumbering. Additional partials include native transcript copy's hardcoded width/whole-row extraction, completion's suffix-space/undo/paste transaction, overlay handle focus ownership and scheduler restart. None is claimed as a runtime reproduction. Native scrollback rebuilds, theme-revision caches, selection/redo, passive overlay policy, scoped readerless rendering and jump-button primitive are explicitly preserved.

TUI-266 owns terminal Kitty cache limits (only offscreen entries:16 images,32MiB transmission,64MiB decoded) and placement/data lifecycle. TUI-267 owns shared11-family ChromeBorder glyph/layout semantics; main TUI-005 remains theme adapter ownership. Main TUI-057 owns installed label/message-count behavior. No source, user-state, shared coverage, README, progress or publication files were modified.
