# Remaining fixes and dependency order

This execution list follows the frozen 25-task matrix. It does not redefine completion.

1. Integrate the two outstanding helper-gate/suppression commits into the isolated completion branch, reconciling overlap without dropping either change. Review their functional diffs and rerun targeted tests.
2. Add cached repository identity (common directory, worktree, branch, revision), immutable task/context ownership, and typed original prompt metadata. Wire host lifecycle and main-agent task/capability-miss events; prompt text changes alone never schedule work.
3. Add a safe persisted context-only callback at subsequent model-request boundaries. Read only ready optional state; no waiting, extra turns or session mutation in transform_context. Persist before counting delivery/novelty.
4. Preserve provenance through caches and graph normalization; apply branch, revision, expiry, supersession and active-context filters at delivery. Fit a single min(800, 1% model-window) estimated-token budget including wrappers/checkpoint/hints.
5. Repair candidate verification/expiry/corrections, store source attribution durably before dispatch, distinguish pending saturation from accepted writes, and provide local curated evidence while graph indexing is unverified. Apply one policy to all legacy automatic promotion paths and advertised MCP writers.
6. Add cross-process queue claims/recovery ownership; preserve uncertain writes and original dataset/category/session across restarts and configuration changes. Retain optional accepted-job receipts without inferring completion.
7. Complete explicit helper provider/model/settings binding and independent disabled mode; shared process/provider rate/concurrency limits, cancellation, bounded candidate cache and abstention. No live JEV requests or silent fallback models.
8. Wire current capability catalogs and real active-message skill coverage into task-event suggestions/inventory. Refresh from current registries without remote reconnects or broad filesystem scans. Never execute a suggestion.
9. Add chronological synthetic/recorded-safe replay and counterfactual arms, scoped local performance measurements, rollout/rollback runbook, privacy-safe progress and review summary. JEV live utility remains explicitly unverified/off, as required by M5.4.
10. Run repository formatting, all-target workspace check, full workspace tests, embedded-doc synchronization check, prebuilt script tests and focused regression suites. Review staged diffs for secrets and unintended changes; make atomic Conventional Commits with co-author trailers. Leave main untouched and final completion branch ready for review only when supported by evidence.
