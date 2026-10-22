# Observability checkpoint — draft complete, independent QA pending

29 entries OBS-001–029 are in evidence/spec-observability.json; no placeholders remain. inspect-spec reported 210 unique references and zero invalid ranges. evidence/proposal-observability.json covers all 8 ledger files, all 22 candidates (13 mapped occurrences, 9 justified extraction false positives), 110 authored surfaces and 10 first-pass claim dispositions. All 39 actual read rows/ranges are recorded in observability-read-progress.json. All source and test reading described by the earlier checkpoint is complete.

Independent reviewer /root/audit_extensions will inspect OBS-013 replay pagination, OBS-014 token/Host boundary, OBS-016 combined first-250-event snapshots/tail cursors and OBS-018 source TTL/concurrency after releasing extensions. Proposal complete=false until this QA and root review are reconciled. Shared publication remains root-only and waits for preceding tools/session/subagents dependencies.

Remaining: review independent QA, assign concrete remaining cross-area dependency IDs, accept manifest and publish. Root corrected authored surface spelling/ranges from direct source-line inventory; 110 actual surface rows are current. SpanGuard race is independently confirmed and provenance verified in local upstream v0.6.1; included in README defect catalog. No runtime reproduction.

Coordinator has moved to settings while other workers continue extensions/tools/session. No application source, user-state directory, network, build, test or installation was touched.
