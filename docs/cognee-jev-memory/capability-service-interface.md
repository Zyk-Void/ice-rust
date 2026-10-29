# Capability service interface (M6)

`CapabilityService` is a session-owned, read-only capability registry. The harness/session owns an `Arc<CapabilityService>`; tools and the host task-event callback hold weak references so the harness does not form a reference cycle. A snapshot is rebuilt only when skill resources or the resident MCP registry revision changes. Catalog creation uses already-loaded skills and MCP snapshots only: no filesystem walk, remote reconnect, Cognee query, or inference.

The local `capability_search` tool supports a 2,048-byte API-level query bound plus `limit`/`offset` pagination, including offsets above 64. Catalog construction has a 4,096-descriptor deterministic cap; pages report `total`, `truncated`, and `next_offset`. Inventory questions can scope `list skills` or `list tools`, and hide disabled, shadowed, and unavailable entries. Stable IDs are source-qualified. Active native tool descriptors use the exact registered schema fingerprint; resident MCP descriptors use stable fingerprints of name, description, and cached/discovered JSON schema when known. Empty/unknown schema remains explicitly un-hashed. Skills include validated local paths/load hints, never fabricated invocation text. Tool policy and server/skill visibility filters apply at registration and query time.

`prepare_hint(session_id, task_id, branch_generation, query, messages, task_event, max_chars) -> Option<PromptContext>` is a resident-only, event-triggered calculation. It returns a whole `ice-capability-hint` packet or omits it when the budget cannot fit it; it never truncates the hint or marks it delivered. The production `HarnessExtension` adapter uses `TaskCursor` per bounded session/lane scope and observes `RequestContextEvent.original_prompt` and compaction timestamps on provider request boundaries. It gates on cancellation/context budget and forwards `prompt_context_persisted` to service acknowledgment. The separate `capability_search` tool currently has no optional `RequestContextEvent` argument and therefore cannot signal a capability miss directly. Failed persistence, cancellation, or stale task/registry callbacks do not suppress future hints. Inventory questions are answered locally and excluded from automatic hints.

Skill coverage is derived from the active branch's existing messages and exact current full instruction body; it is not inferred from historical “ever loaded” state. Context/message revision changes, compaction, branch switches, and catalog body changes invalidate cached coverage. It performs no file reads. A per-session/task/registry bounded suggestion ledger prevents duplicate hints only after successful persistence.

`CapabilityService::doctor_cognee_contract() -> serde_json::Value` is the Cognee doctor binding accessor. It returns `{"tools":[{"name":String,"id":String,"schema_fingerprint":String,"status":"last_known_available_unprobed"|"offline_or_disabled","methods":[]}],"probed":false}` for Cognee-named entries in the same resident service catalog, without probing. Native and known MCP schema fingerprints are exact; unknown MCP schemas are not fabricated. Do not interpret `last_known_available_unprobed` as healthy and do not issue a connection or request from this accessor.

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

Production wiring is installed by session construction: the same `Arc<CapabilityService>` is registered as the extension and the `capability_search` tool, and `/reload` refreshes that Arc from the current tool/resource registries. Request-context hints are cursor-event-driven, not once-per-session. Direct capability-miss context signaling from `capability_search` remains unimplemented.
