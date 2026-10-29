# Installed Cognee MCP contract and writer verification

- Date: 2026-10-28
- Worktree: `/home/mewtwo/Zykairotis/ice-rust/.worktrees/cognee-remaining-acceptance`
- Revision examined: `87ea8968bda22fd3977b827b6109ba10b0153526`
- Scope: offline inspection of the locally installed Cognee MCP Python source, AST-extracted wrapper calls against an in-memory fake backend, and host-side native/MCP policy. No MCP launcher, live server, Cognee API, credentials, or real data operations were used.
- Result: the installed source defines memory writers and maintenance behavior beyond what an advertised tool list alone would prove. The local fake checks passed, but they do not establish deployed MCP schemas, backend authorization, or service behavior. M0.2 and M4.2 remain **PARTIAL**.

## Source identity and method

The installed source files were read without importing the server package:

| Source | SHA-256 |
|---|---|
| `src/server.py` | `7bb6ef9d60452909b1121e6383c9677f56a8adadbbb9dddf3ac92eca105ce14e` |
| `src/tool_registry.py` | `cd4632ab9fabe33ee683c08982a855f7a8371e0e4bcb077d51380d314a9d74b7` |
| `src/cognee_client.py` | `a128b686d734c937c5279e109d6e6ff82a840e8d7d4fab92a319a5b9d99dab7e` |

The test harness at `target/cognee-remaining-validation/fake_installed_mcp_wrappers.py` parses the installed `server.py`, extracts selected wrapper functions, removes their MCP decorators, and supplies fake client methods and local formatting/validation helpers. The harness does not import the installed package, start FastMCP, read configuration, create network clients, or call a real backend. It checks wrapper argument normalization and forwarding; it does not test `cognee_client.py` HTTP transport or backend semantics.

Command: `python3 -I target/cognee-remaining-validation/fake_installed_mcp_wrappers.py`

Result: exit 0; output retained in `target/cognee-remaining-validation/fake-installed-mcp-wrappers.log`. Assertions covered regular and background `remember`, the default `self_improvement=True`, explicit `self_improvement=False`, invalid remember input without a backend call, recall argument normalization and result metadata, `code_search` delegation, scoped fake `forget` and invalid scope rejection, the extracted `improve` function, and status lookup through fake dataset and pipeline methods. The fake `forget` invocation was not connected to any real store.

## Installed tool surface

Signatures below are from the installed `server.py`; they are not live MCP `tools/list` schemas.

| Function | Installed inputs and defaults | Registration / classification | Behavior visible in source |
|---|---|---|---|
| `remember` (`server.py:357`) | `data=None`, `filename=None`, `content_base64=None`, `dataset_name=None`, `session_id=None`, `custom_prompt=None`, `background=False`, `ontology_key=None`, `self_improvement=True` | Registered with `default` and `memory` tags; writer | Text or file ingestion. Permanent memory runs ingestion/cognify and, by default, self-improvement. Session mode stores a session entry; direct mode may bridge it to graph memory. Permanent `background=True` queues work and points callers to `cognify_status`. Uploads are limited to 10 MiB and cannot be combined with session mode. The installed client routes permanent writes to `POST /api/v1/remember` and session writes to `POST /api/v1/remember/entry` (`cognee_client.py:605-691`). |
| `recall` (`server.py:531`) | Required `query`; `search_type=None`, `datasets=None`, `session_id=None`, `system_prompt=None`, `top_k=15`, `scope=None`, `code_query=None` | Registered with `default` and `memory` tags; reader, with completion/search modes that may invoke model work | Normalizes CSV dataset/scope inputs and delegates to client recall. The installed client uses `POST /api/v1/recall` (`cognee_client.py:824-839`). A source-level reader label alone is not a guarantee that every selected search mode is inference-free. |
| `code_search` (`server.py:654`) | Required `operation` from `query_facts`, `explore`, `traverse`, `find_path`, `impact_analysis`, `insights`, `architecture`, `delta`; `arguments=None`, `datasets=None`, `query=""`, `top_k=15` | Registered with `default`; delegates to `recall` with `search_type="CODE"` and `scope="code"` | Code-graph query wrapper, not a separate backend client method. |
| `forget` (`server.py:714`) | `dataset=None`, `everything=False`, `data_id=None`, `dataset_id=None` | Registered with `default` and `memory`; destructive writer | Deletes data from the relational, graph, and vector stores. Requires a dataset/everything selector or a scoped item identifier. Client route is `POST /api/v1/forget` (`cognee_client.py:884-898`). Only a fake, scoped call was exercised; no real deletion was attempted. |
| `cognify_status` (`server.py:899`) | `dataset_name=None`, `pipelines=None` | Registered with `status`; status/reader | Resolves a dataset and reads pipeline status. API mode lists datasets and queries `GET /api/v1/datasets/status` (`cognee_client.py:476-545`). It is not pinned in the minimal memory catalog. |
| `improve` (`server.py:791`) | `dataset_name=None`, `session_ids=None`, `node_name=None`, `build_global_context_index=False`, `build_truth_subspace=False` | **Not registered** with `registry.tool`; function has usage logging only. Not an independent MCP tool entry point in this source. | Calls the installed client improvement operation (`POST /api/v1/improve`, `cognee_client.py:918-945`). It still matters as an internal side effect: permanent `remember` defaults `self_improvement=True` and passes that flag to the client, whose permanent path runs improvement unless disabled. |

The installed registry tags functions when decorated (`tool_registry.py:31-65`). `apply_tool_mode` (`server.py:209-253`) selects the advertised catalog: `all` advertises every registered tool; `default` pins tools tagged `default`; `minimal` pins tools tagged `memory`. In the search modes the code comments explicitly say the transform only changes `tools/list`: the other registered tools remain searchable and directly callable by name. Thus `minimal` is an advertisement tier, not a strict callable allowlist. `improve`, by contrast, has no registry entry in the inspected source; it is not made callable as an MCP tool merely because the Python function exists.

## Host boundary and explicit native commands

Native Cognee controls and generic third-party MCP calls are different trust boundaries:

- The Rust host's `Approval` is `All`, named `Tools`, or `None`; `ServerConfig.approve_tools` defaults to `None` (`crates/ice-cli/src/mcp.rs:58-74,104-119`). Configured approval-required direct and proxied calls fail closed when an approval UI is unavailable (`crates/ice-cli/src/mcp/tests.rs::approvals_are_interactive_per_call_and_fail_closed`, `crates/ice-cli/src/mcp/proxy_tests.rs::proxied_calls_need_approval_like_direct_ones`). This is a host call-approval gate, not a Cognee writer allowlist; with the default `None`, it does not require per-call approval.
- The host cannot infer or constrain side effects performed internally by an arbitrary third-party MCP server. In particular, a tool's advertised tag/name does not prove that the server has no autonomous integrations or secondary writes.
- Native explicit operations remain distinct: `manual_remember` and `manual_improve` have explicit user-facing paths, and tests assert that explicit remember remains available under selective policy (`crates/ice-cli/src/cognee/runtime.rs:3008-3013,5045,6514`). Broad automatic capture/admission policy should not be replaced by disabling those explicit commands.

No native policy or MCP approval configuration was changed for this evidence. The installed third-party source is not evidence that the service itself enforces a particular authorization policy.

## Safety and limitations

- No connected server identity/version or live advertised schema was captured; these signatures are local-source contracts only.
- No deployed native wrapper compatibility, API authorization, backend writer policy, actual completion, indexing, or searchability was established.
- No inference-free property was asserted for the complete external server. `recall` can route across search modes, and tool availability is not a data-side-effect proof.
- No `forget` call reached a service. No endpoint was contacted, and no launcher was run.
- Earlier in this investigation, an unsuccessful probe imported the installed server module. That import initialized the Cognee logger, removed an older file under `/home/mewtwo/.cognee/logs/`, created a new log, and reported loading `/home/mewtwo/.env`. The environment values were not displayed; no launcher, service request, or destructive Cognee operation ran. This was unintended access to private state and violated the stated boundary. The successful harness above avoids module import and did not repeat that interaction.

## Acceptance boundary

Keep M0.2 and M4.2 partial until an authorized isolated service or authoritative deployment manifest establishes the actual native/MCP version and complete advertised/callable inventory, including writer side effects and approval enforcement. Do not infer third-party enforcement from native policy, tool tags, or the AST-extracted fake checks. See also [remaining contract evidence](remaining-contract-evidence.md) and [remaining writer evidence](remaining-writer-evidence.md).
