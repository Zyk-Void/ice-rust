# Capability service interface (M6)

`CapabilityService` is a session-owned, read-only capability registry. The harness/session owns an `Arc<CapabilityService>`; tools and the host task-event callback hold weak references so the harness does not form a reference cycle. A snapshot is rebuilt only when skill resources or the resident MCP registry revision changes. Catalog creation uses already-loaded skills and MCP snapshots only: no filesystem walk, remote reconnect, Cognee query, or inference.

The local `capability_search` tool supports bounded query plus `limit`/`offset` pagination. Exact inventory is answered from this snapshot and reports stable source-qualified IDs, schema/instruction fingerprints, configured/enabled/readiness/availability, and skill-context state. Skills include validated local paths/load hints, never fabricated invocation text. Tool descriptors retain the exact native or server-prefixed route; use still goes through normal harness approvals. Tool policy and server/skill visibility filters apply at registration and query time.

`prepare_hint(session_id, task_id, branch_generation, query, messages, task_event, max_chars) -> Option<PromptContext>` is a resident-only, event-triggered calculation. It returns the opaque `ice-capability-hint` packet with correlation ID but does not mark a hint delivered. The host must call `acknowledge_hint(&context, persisted)` only after confirming persistence; acknowledgment is fenced by task and registry ownership. Failed persistence, cancellation, or stale task callbacks do not suppress future hints. Inventory questions are local and are not routed to memory or JEV.

Skill coverage is derived from the active branch's existing messages and exact current full instruction body; it is not inferred from historical “ever loaded” state. Context/message revision changes, compaction, branch switches, and catalog body changes invalidate cached coverage. It performs no file reads. A per-session/task/registry bounded suggestion ledger prevents duplicate hints only after successful persistence.

`doctor_cognee_contract()` provides local MCP contract inventory from resident manager snapshots: known tool names, schema fingerprints/method capabilities, and configured/disabled/last-known status. Unprobed, unsupported, failed/offline, and tested-healthy are distinct; the method never probes or guesses endpoint/alias routes.

## Lead binding example

Keep the service handle from `ReloadContext::capability_service` for the lifetime of the registered tool. At the host's task-event callback, call the service directly with the current branch snapshot; append the returned context only through the normal persisted-context path, then acknowledge the persistence result:

```rust,ignore
if let Some(service) = reload_context.capability_service.as_ref() {
    if let Some(context) = service.prepare_hint(
        session_id,
        task_cursor.task_id(),
        task_cursor.branch_generation(),
        task_event.query(),
        &task_cursor.messages(),
        task_event.kind(), // TaskStart or CapabilityMiss; exclude ordinary followups
        280,
    ) {
        let persisted = append_prompt_context(&context).await.is_ok();
        service.acknowledge_hint(&context, persisted);
    }
}
```

The present harness has no `RequestContextEvent` safe pre-request callback or task cursor in the Rust session consumer. Therefore the automatic host callback attachment remains the single lead-owned integration hook; the registered tool and resident service work independently now. Do not turn regular follow-up prompts into `TaskStart` events.
