# Live acceptance evidence: deployed MCP surface, JEV transport, isolated canary

- Date: 2026-10-28
- Source: branch `fix/cognee-remaining-acceptance` at `a3b09836` plus the bounded-polling change to `live_service_tests.rs` committed with this report.
- Authorization: the owner approved live JEV calls (at most 20), an isolated Cognee API and MCP server, and cleanup of test data in the live Cognee store.
- Result: M0.2, M4.2 and M7.3 are now evidence-backed. M5.4 (held-out effectiveness) is still blocked: live JEV transport works, but no frozen held-out data or independent labels exist.

## Incident: the first "isolated" service used the live store

The installed Cognee package runs `dotenv.load_dotenv(override=True)` on import (`cognee/__init__.py`, `cognee/shared/__init__.py`). With no path given, python-dotenv searches upward from the package directory and found `/home/mewtwo/Zykairotis/cognee/.env`. That file replaced the scratch `DATA_ROOT_DIRECTORY`, `SYSTEM_ROOT_DIRECTORY` and JWT secret, so the process on port 18771 opened the live `data/.cognee_system`. Separately, the config bug fixed in `704f2c9b` made a canary runtime fall back to the default base URL `127.0.0.1:8211`, the live API. The open storage paths were not checked before the first writes.

The following was written to the live store, all of it synthetic: a user `iso-test@example.com`, an API key `iso-acceptance`, dataset `iso_acceptance` (2 items, fully processed and graphed through the configured LLM and embedding providers), and datasets `iso_canary_1793194360321` and `iso_canary_1793194401306` (1 item each). With the owner's approval, the three datasets were deleted through the API as that user (each returned 200), and the key was revoked (200; it then received 401). One orphaned 57-byte raw file holding only the canary sentence was removed by hand. A search of `data/.cognee_data` then found no test markers. The test user account remains, because the API has no route to delete users; it owns nothing and has no key.

The rerun disabled dotenv before Cognee was imported (`nodotenv_uvicorn.py`) and started the service through `env -i` with a scratch HOME, XDG directories, data and system roots, and port. Before any write, the process's open files were checked and showed zero paths under the live data directory. Its system database was created in the scratch folder. The live `cognee_db` modification time (`2026-10-28 16:46:47 +0300`) did not change across registration, key minting or the canary run.

## M0.2: deployed native/MCP contract

- MCP server: installed `cognee-mcp` reports `{"name":"Cognee","version":"0.5.7"}` with protocol `2025-06-18`. The SHA-256 prefixes of `server.py`, `tool_registry.py` and `cognee_client.py` match those recorded in [connected-contract-writer-verification](connected-contract-writer-verification.md). It was launched with the active launch arguments (`--transport stdio --api-auth-scheme x-api-key --tool-mode minimal`), with only `--api-url` pointing at the isolated API. The launcher wrapper was not used because it hard-codes port 8211 and a HOME-relative path.
- Advertised `tools/list`: `remember`, `recall`, `forget`, `search_tools`, `call_tool`, with no MCP annotations. The full schemas are in [evidence/live-mcp-tools-list.json](evidence/live-mcp-tools-list.json).
- Callable but not advertised: `code_search` and `cognify_status`, found via `search_tools` and invoked via `call_tool`. `improve` is not callable (`Unknown tool: 'improve'`), which matches the static registry.
- Against API 1.4.0: `remember` returned `Stored permanently … status=completed`. `call_tool` → `remember` also stored permanently. `recall` with `datasets` as a string returned the stored fact; passing a list is rejected by the wrapper's schema. `cognify_status` returned `DATASET_PROCESSING_COMPLETED`.
- `remember` with `self_improvement: false` still logged `running self-improvement` on the server. The flag is not honoured end to end, so server-internal improvement must be treated as unconditional.
- The native icers client (real `ReqwestTransport`, `x-api-key`) against API 1.4.0 got `POST /api/v1/remember` 200 and the item became readable. Inference-free reads (`GET /api/v1/datasets`, `/{id}/data`, `/{id}/data/{data_id}/raw`) were used to verify it.

## M4.2: generic MCP writers

The deployed run confirms the offline [cross-route report](cross-route-writer-report.md): the forwarder `call_tool` reaches `remember` and writes permanent memory. With owner approval, the active `~/.icers/agent/mcp.json` `cognee` entry now has `"approveTools": true`, which gates every route including forwarders and fails closed when headless. The backup is `mcp.json.bak-2026-10-28`. Explicit native remember is unchanged.

## M7.3: isolated canary and rollback against a real service

`cognee::runtime::tests::live_isolated_canary_and_rollback` (ignored by default) passed in 41.2 s. Log: `target/cognee-remaining-validation/live-canary.log`.

| Step | Observed on the service and in the local queue |
|---|---|
| 1. Disabled; queue seeded with pending, uncertain and in-flight records | 0 service items; queue files unchanged |
| 2. Enabled (canary) | exactly the pending intent delivered (polled until visible); uncertain and in-flight preserved |
| 3. Explicit `manual_remember` | delivered |
| 4. Disabled again (rollback) with a new pending intent | after a 20 s settle wait, no new service items; queue files unchanged |

The first live attempt exposed the config bug that `704f2c9b` fixes: a stored config without a helper binding was rejected, so a runtime configured disabled ran as the enabled defaults. The old-binary-against-new-queue hazard in [remaining-canary-rollback](remaining-canary-rollback.md) still applies. This run used the current binary only.

## M5.4: live JEV

`cognee::helper::tests::live_jev_helper_smoke` (ignored by default) ran through `Helper::bind` → `ClientDecider` → `DecisionClient` against `https://api.typesafe.ai/v1/systemone`, model `jev-latest`, with the key taken from the owner's environment (never printed). Log: `target/cognee-remaining-validation/live-jev.log`.

- Requests: 7 (6 single-candidate, 1 batch of 4), no retries, real 1.5 s deadline. All 7 were judged, at 294–402 ms; there were no errors, timeouts or abstentions.
- Categories matched the author's synthetic smoke labels in 6 of 6 cases (constraint, none, decision, preference, lesson, none), and the batch agreed with the single calls. Durability: `Yes` for the constraint and the decision, `No` for the greeting, the transient lesson and the question, `Uncertain` for the preference. Every `applicable` came back `Uncertain`, most likely because the context string was generic.

This establishes transport, binding and decoding, and plausible behavior on 6 hand-written cases. It is not effectiveness evidence: the labels are the author's own, the data is not chronological held-out data, and no paired D-arm run against arm C exists. M5.4 stays **BLOCKED** until frozen held-out data with independent labels is available (see [evaluation protocol](evaluation-protocol.md)).
