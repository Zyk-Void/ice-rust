# Parallel audit worker contract — 2026-10-20

Read requirements.md completely and evidence/resume-request-2026-10-20.md completely before substantive work. The user explicitly authorized real parallel agents in the original attached resume request; evidence/execution-policy.json is preserved historical policy and is superseded only on delegation by evidence/execution-authorization-2026-10-20.json.

## Access
Initial Rust workspace: ws_0e25a2707810aacc6b30bda5 (/home/mewtwo/Zks/ice-rust).
TypeScript workspace: ws_d62232a3dcdfcedb170213cb (/home/mewtwo/Zks/ice).
Active Rust workspace for read-only later comparison: ws_0235eae4b04eada508c345c4.
Discover tool schemas through ALL_TOOLS; use the exposed ConnectMe tools, not local /workspace as a substitute for workstation paths.
Do not launch local agent binaries or agent_start; they may write outside permitted directories or access forbidden state. Use only the actual delegated session.

## Source identities
TS void: 3ab05193147f5abb8eb8ee6c0178f6dbce8918b0.
Initial Rust main: ff9270f26e263ea9ce54349786bf2d93c4517476.
Preflight: evidence/resume-integrity-2026-10-19T222055Z.json.
Current verified hashes: evidence/resume-source-identities-2026-10-19T222055Z.json.
No differences against 831 original and 29 supplemental TS baseline files, or recorded read hashes. 540 Rust files hashed.
Earlier integrity run flagged only git --porcelain versus --short submodule m/M notation; direct --short matched historical baseline. No source-content drift.
For source ranges use the read-only read-evidence.mjs helper if needed, compare returned SHA256 to the source manifest before using findings. Never use direct-source.mjs read mode from multiple workers because it writes shared direct-last-read.json.
If a newly inspected TS test is not in the source manifest, record its current hash and baseline limitation; verify clean tracked tests against the recorded Git revision and do not invent a historical hash for dirty tests.

## Boundaries
Only your assigned area-specific drafts, evidence fragments, checkpoint, and authoring helpers under /home/mewtwo/Zks/ice-rust/docs/ice-gap/ may be written.
No application source changes, commits, branches, installs, network commands, app execution, application tests, or real ~/.ice or ~/.icers access. Read source strings naming those paths only.
No scratch or /tmp artifacts; any necessary working artifact belongs in your assigned area files within the allowed audit tree.
Do not write the active checkout yet. Do not edit README.md, ledger.csv, surfaces/*.csv, evidence/progress.json, evidence/RESUME.md, evidence/read-coverage.json, or published area .md files.
Do not run render-area.mjs, publish-area.mjs, merge-progress.mjs, or audit-util record-reads; the coordinator owns these operations.
Use installed tools; inspect helper side effects. Source-only audit, no code tests. User task overrides irrelevant repository implementation workflows.

## Nine mandatory entry fields (exact original labels)
1. Title and status: missing, partial, or different-by-design; stable AREA-NNN ID.
2. ice source: every relevant file path and line range, exported symbols and behavior-specifying tests.
3. What it does: user-visible behavior in plain language.
4. How it works: entry points, control flow, data flow, state/transitions, persistence format/location, concurrency/locking/ordering, exact limits/constants, errors/failures and security/trust checks.
5. Interfaces: exact tool parameter schemas, RPC shapes, setting keys/types/defaults, environment variables, CLI flags, slash commands, keybindings, emitted/consumed events and file formats. Quote short schemas verbatim.
6. Dependencies: other entry IDs and external services/binaries.
7. ice-rust today: Rust paths and lines, existing behavior and exact differences.
8. Port notes: target crate/plugin, required host API additions, applicable Rust crates and risks.
9. Size and priority: estimated Rust lines, P0-P3 and one-line reason.

Existing spec schema uses title, introduction, entries, optional appendix. Each entry: id, title, status, source, what, mechanism, interfaces, dependencies (array), dependency_text if useful, rust, port, size_min, size_max, priority, priority_reason.
Open mechanism with a concise 2–3 sentence behavior summary, then exact details. Use ice:path:start-end and rust:path:start-end citations.
Full subsystems and relevant tests must actually be read, not merely searched or mechanically processed. Missing claims require behavior-based Rust searches.

## Deliverables per assigned area
- Preserve existing evidence/spec-<area>.json and IDs; extend/correct only assigned content.
- Save evidence/spec-<area>.json incrementally.
- Save proposed publication manifest as evidence/proposal-<area>.json, NOT evidence/<area>.json until coordinator accepts it.
- Existing manifest schema: area, complete, entries (metadata), ledger (path, entry_ids array, status, notes), surfaces (category,name,ice_ref,rust_ref,status,entry_id,candidate_ids,notes), read_evidence (path with ice:/rust:,ranges,notes), candidate_exclusions if mechanically false positive with reason, claims, dependency_requests, rust_only, open_questions, supplemental_ledger if needed.
- Save coverage/checkpoint evidence in evidence/checkpoint-<area>.md, including exact unread ranges, source hashes, verified/unverified distinctions, unresolved questions, and next action.
- Save source review evidence and any defect candidates separately in your area-owned evidence files.
- Do not bulk map every candidate to a generic entry. Exact names, contracts, ranges, IDs and classifications must be justified.
- Present/false-positive surface dispositions must reflect the existing schema. No arbitrary denominator change.
- Retain unresolved cross-area dependency descriptions in dependency_requests until real IDs exist. Do not invent future IDs. Coordinator will resolve and enforce publication order.
- Run inspect-spec.mjs <area> only after inspecting it (read-only citation-range checker); report what it does and does not validate.

Report checkpoints and actual artifact paths. No final success claims based solely on structural validation. When one assigned area is complete, request another bounded assignment; do not edit another area or another worker's files unilaterally.
