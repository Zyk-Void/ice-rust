# Tools audit checkpoint

## Worker status

Worker draft complete; independent coordinator review and publication pending. This is source-only audit execution, not an implementation change. No application code or tests were executed.

- Assigned area: tools. Stable entries: TOOLS-001 through TOOLS-031 (21 partial, 10 missing).
- All 33 tools ledger files were read in full through their ledger line counts.
- 104 source/test files have verified hashes and actual merged read ranges in tools-read-evidence.json. This includes 24 standalone TypeScript test files, 10 standalone Rust test files and embedded test bodies where read.
- All 35 mechanical candidates are assigned once: 22 occurrences map to 15 substantive surfaces; 13 dynamic caller-path expressions are precisely excluded as mechanical false positives. Ten additional authored surfaces cover five session environment variables, two setting consumers, BashSpawnHook and the two background event producers.
- tools proposal contains 25 surfaces in total. Authored consumer rows do not create new stored settings or change source-ledger denominators.
- inspect-spec.mjs reports 31 entries, 202 unique line references and no range problems. Separate in-memory checks find no missing mandatory fields, internal dependency cycles, uncovered cited ranges, unmapped ledger files or duplicate/missing candidate dispositions. These checks do not prove semantics.

## Authoritative artifacts

- evidence/spec-tools.json: complete nine-field behavioral entries and Rust comparisons.
- evidence/proposal-tools.json: proposed entry metadata, ledger, surfaces, candidate exclusions, first-pass claim dispositions, dependencies and Rust-only capabilities.
- evidence/tools-read-evidence.json plus tools-read-requests.json: actual-read ranges and full-file SHA-256 basis.
- evidence/tools-source-review.json: source-only defect conditions, test gaps and partial-source boundaries.
- evidence/tools-defect-evidence.json: TOOLS-010 full-log prefix-loss reasoning and local Git attribution.
- tools-findings-draft.json is an earlier working note, not the final entry authority.

## Source identity and dirty baseline

TypeScript void: 3ab05193147f5abb8eb8ee6c0178f6dbce8918b0. Rust main: ff9270f26e263ea9ce54349786bf2d93c4517476. File hashes are verified against resume-source-identities-2026-10-19T222055Z.json or the frozen clean tracked Git file. No remote/network provenance was queried.

bash-watch.ts matches the manifest's current dirty implementation baseline: 273cc9518bb656ff8598ad0cc9dc084b21f07f69a15ced5c42163610c37ff074. Two supplemental dirty tests have current-only snapshots, not historical baseline evidence:

- bash-watch.test.ts: d9caac6a28b34495f29b3b1ea227760d15073bc7200b332665436dac4c61e579.
- execution-reliability-1-watch-alerts.test.ts: 753e67671a176f10b9fe8136f119f1efba3d1f38a6162ef5c363a9b7b1e665ba.

## Material review targets

TOOLS-010: output over the 102400-byte retained-tail cap can lose its prefix before the advertised full log is initialized. Local blame attributes the late spill block to 909e85120146cfcf46b7835d69a8805bce3edcff at former crates/pi-tools/src/shell_output.rs:262-286. Existing spill tests cross line presentation limits, not the retained byte cap.

TOOLS-019: byte-limited multi-line tails can report too many output lines. TOOLS-021: key-resolution ordering and missing-target symlink aliases differ from the current coding-agent queue. TOOLS-025: exact-match precedence differs from current TypeScript. TOOLS-026: Replace display branches reuse new lines for removals; unified patch generation is separate. TOOLS-008: filesystem cleanup kills PIDs but does not own/remove detached temporary logs. All are conditional source findings, not runtime reproductions.

TOOLS-031 preserves the actual Rust 64-message RPC writer and identifies missing byte budgets, transient retries and stdout isolation. TOOLS-017 preserves the distinct direct-argv API scope; extension host integration belongs to the extensions owner. Rust-native search, image processing, mutation capability, edit compatibility and optional PowerShell are explicitly preserved.

## Exact unread shared-source ranges

These files were read only for their tools-related integration slices. No whole-file read is claimed; their owners must audit the rest. Every tools-ledger source line is read. A final empty split line alone is not counted as an unread behavior.

- rust:crates/ice-tools/src/os_env.rs: 1-134, 211-384, 421-529, 1111-1141.
- ice:packages/coding-agent/src/utils/shell.ts: 1-189.
- rust:crates/ice-cli/src/session.rs: 1-1934, 2051-4016.
- rust:crates/ice-cli/src/interactive_tui.rs: 1-554, 676-1369, 1511-3398.
- rust:crates/ice-cli/src/modes.rs: 1-99, 147-569, 659-1108, 1303-1566.
- rust:crates/ice-tui/src/tool_execution.rs: 745-779, 883-1523, 1597-1631.
- rust:ICE_PORTING.md: 1-280, 327-623, 649-678.

## Cross-area handoff and next action

Coordinator owns independent review, final IDs/dependency resolution and publication. Confirmed settings references are recorded in dependency_requests: SET-014 shell settings; SET-007/008/009 scoped storage/precedence/writes; SET-003 trust. Exact autoResizeImages setting owner remains for coordinator resolution. Background session delivery/wake/dedup/active-tool composition, TUI shells panel/previews, RPC controls/output lifecycle, and extension adapters remain explicitly requested integrations, not invented dependencies.

The worker now proceeds to the separately assigned subagents area. No published area Markdown, shared surfaces/ledger/read-coverage/progress or active checkout has been written by this worker.
