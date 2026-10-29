# M4.2 remaining writer and policy evidence

- Date: 2026-10-28
- Scope: source-backed inventory of local native writer paths and host-visible MCP approval behavior. No deployed service, live MCP connection, user home, `~/.ice`, TypeScript ICE, or active `~/.icers` was inspected or probed.
- Result: local automatic admission/promotion paths remain gated and explicit commands remain separately available. The connected third-party MCP writer inventory is unavailable, so M4.2 remains **PARTIAL**.

## Local native writer routes

| Route | Trigger / behavior | Local host gate | Key evidence |
|---|---|---|---|
| Explicit `cognee_remember` / `/cognee remember` | User-directed durable write; uses persisted operation state and native `remember` request | Requires Cognee enabled and configured; explicit write is not disabled by selective/manual policy | `crates/ice-cli/src/cognee/runtime.rs::manual_remember_with`; `manual_remember_still_sends_under_selective_policy`; `manual_remember_persists_inflight_before_request_and_removes_on_success` |
| Explicit `cognee_improve` / `/cognee improve` | User-directed dataset/session improvement | Explicit path is distinct from automatic improve and may proceed when `auto_improve` is off | `crates/ice-cli/src/cognee/runtime.rs::improve_current`; `manual_improve_bypasses_failure_backoff`; `manual_improve_requires_cognee_enabled_but_not_auto_improve` |
| `cognee_propose_memory` | User/agent tool proposes a source-attributed candidate; acceptance is host-owned | Requires enabled Cognee and selective policy; checks complete supported source evidence, role, sensitivity, session/task/branch/revision/worktree, current source snapshot, expiry, and policy again before durable admission | `crates/ice-cli/src/cognee/propose.rs::propose_memory`; `crates/ice-cli/src/cognee/validation.rs::accept_validated_candidate`; `crates/ice-cli/src/cognee/source_validation.rs::validate_source` |
| Session/prompt/answer capture | Automatic session events | Legacy-only `captures_session`; runtime rechecks at dispatch, cancellation/shutdown, and settings changes | `crates/ice-cli/src/cognee/config.rs::captures_session`; `crates/ice-cli/src/cognee/runtime.rs::store_entry`, `automatic_capture_allowed` |
| Tool trace capture | Automatic tool-result events | Legacy-only `captures_tools`; runtime checks trace switch and policy before dispatch/replay | `crates/ice-cli/src/cognee/config.rs::captures_tools`; `crates/ice-cli/src/cognee/runtime.rs::store_entry`, `drain_warmup` |
| Warmup replay | Automatic replay of captured pending session entries | Legacy capture policy and cancellation are rechecked before each request; ambiguous/InFlight records are preserved as uncertain and not automatically replayed | `crates/ice-cli/src/cognee/runtime.rs::drain_warmup`; `crates/ice-cli/src/cognee/queue.rs`; `disabled_trace_warmup_is_preserved_while_answers_can_replay` |
| Automatic improve | Background promotion of captured data | `improves()` is legacy-policy gated; cancellation token and current policy are rechecked at dispatch | `crates/ice-cli/src/cognee/config.rs::improves`; `crates/ice-cli/src/cognee/runtime.rs::improve_current_guarded` |
| Dream pass / candidate promotion | Background extraction and admission from tool/answer records | Triggered only from eligible legacy automatic capture; source/candidate admission checks remain host-owned, helper verdict is advisory | `crates/ice-cli/src/cognee/runtime.rs::dream_pass`, `queue_dream_write`, `accept_validated_candidate` |
| Compaction remembering | Automatic compacted-context checkpoint | `remembers_compaction()` is legacy-only; current event source and policy are checked | `crates/ice-cli/src/cognee/config.rs::remembers_compaction`; `crates/ice-cli/src/cognee/runtime.rs::after_compact` |

`WritePolicy::Manual` and `WritePolicy::Selective` disable broad legacy captures/improve/compaction writes. Selective admission is not equivalent to helper authorization. The helper setting defaults on, including when an older config omits `helperEnabled`; an explicitly stored or environment-provided `false` remains off. This default does not bind a service: helper construction additionally requires Cognee enabled, selective writes, and an explicit decision provider/model binding to TypeSafe System One. Missing or invalid binding is nonfatal and visible, with no fallback provider/model. The helper uses bounded provider/global quotas and expiring positive/negative cache entries, and cannot supply source proof. Helper cache identity includes provider ID, sanitized endpoint, helper model, dataset, runtime context generation, and candidate identity/content/scope/expiry; context generation changes on session/repository identity and `save_and_adopt`. A settings revision is not separately hashed.

The local settings loaders read `models.json` and `settings.json` when a helper is bound. `CogneeRuntime::rebuild_helper` runs on initial runtime load and when `save_and_adopt` changes the Cognee helper signature; no settings-file watcher or automatic rebind was found. Editing provider credentials, endpoint, or `httpIdleTimeout` outside the running process does not update an already-bound `DecisionClient` until helper rebinding/restart. This is a lifecycle boundary, not evidence of an automatic-policy bypass; changes made through the runtime config path rebuild the helper and increment cache context generation. Stop/restart the isolated process after modifying provider/settings files.

## MCP routes and permission boundary

The local MCP layer is a generic direct/proxy catalog, not a Cognee-specific writer implementation. It discovers and executes whatever tool schemas a connected external server advertises. Host per-tool `approveTools` supports approve-all, named tools, or none; **none is the default**. Calls configured to require approval fail closed if an approval UI is unavailable (`MCP tool approval denied`). Direct and proxied registered calls share the approval policy.

This approval gate is not a writer allowlist and does not prove whether a third-party method writes memory. An external server may mutate its own store or call remote services outside this host's Cognee policy. No connected Cognee server, advertised method list/schema, endpoint, or service behavior was available in safe local manifests. Therefore there is no truthful external writer enumeration and no claim that native admission policy controls arbitrary MCP servers. For any future authorized inventory, record server identity/version, each advertised tool schema, read/write side effects, explicit-versus-background trigger, approval configuration/effective UI, data destination, and independent automatic integrations; do not infer writers from names alone.

## Regressions present and added

Existing focused coverage includes:

- source validation rejects missing/wrong-role/sensitive/unsupported, paraphrased or partial-condition spans, skill-injected user constraints, and unverified outcomes (`source_validation.rs` tests);
- unsupported source is rejected before remote helper judgment (`runtime.rs::helper_rejects_unsupported_source_before_remote_judgment`);
- selective policy disables all legacy writes even if legacy switches are enabled (`runtime.rs::selective_policy_disables_all_legacy_writes_even_with_switches_on`);
- explicit remember still sends under selective policy (`runtime.rs::manual_remember_still_sends_under_selective_policy`);
- queued capture/improve, warmup replay, compaction, delayed improve, disable/re-enable races, cancellation and uncertainty preservation (`closeout_runtime_tests.rs` and `runtime.rs` tests listed above);
- MCP approval-required calls without UI fail before tool execution (`mcp/proxy_tests.rs::proxied_calls_need_approval_like_direct_ones`).

This change adds `validation::tests::volatile_candidate_rejects_a_stale_repository_revision`, proving a volatile Outcome scoped to an old Git revision is rejected while the same source/scope with the current revision is accepted. No duplicate production guard was required: the scope validator already compares branch, revision and worktree identity.

## Remaining acceptance boundary

The local implementation supports a single host-owned native admission path and configured host approval for MCP calls, but M4.2 cannot be closed until the owner supplies an authorized, isolated connected MCP/service inventory (M0.2). Verify whether each external writer has independent automatic promotion and whether its own permission setting is enforceable. Do not disable explicit user commands as a substitute. Never treat this evidence as a live inventory or backend contract.