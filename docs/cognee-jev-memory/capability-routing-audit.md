# Capability routing audit: existing JEV, tools, and skills

Audited 2026-10-27 against `feat/cognee-helper` at `09a728cdc5b98e270b8f01e38d2575ca047d45ff`. Scope: read-only code, selected skill files, and the accompanying historical observability report. No live model requests, installation, configuration change, or source edit occurred.

## 1. Conclusion

Most of the infrastructure required by the proposal already exists. The missing piece is a policy that chooses the correct source and operation at the right time, plus state describing what is already in the model's context.

The native TypeSafe client should be reused. Existing skills and the MCP gateway already provide on-demand instructions and tool schemas. Historical memory should help find a prior decision or successful workflow. The current catalog should establish discovered/enabled capabilities and their sources; configured connection state and observed errors should separately describe last-known readiness. The agent should not search a memory graph to discover its present tool inventory.

There are also two distinct uses of the name “Jev” on this machine. The installed `jev-search` skill describes a web-search CLI using `jev.s1.dev`. The Rust decision client uses the TypeSafe System One protocol. The existence of that skill is not evidence that the Cognee helper is wired to a decision model.

## 2. What is present

| Capability | Evidence | Status and practical use |
|---|---|---|
| Typed TypeSafe Noul/Choice/Score codecs | `crates/ice-ai/src/decision/typesafe.rs`; `docs/decision-api-d1-implementation.md:24-97` | Existing typed request/response validation; reuse rather than a free-text JSON parser |
| One-shot decision HTTP transport | `crates/ice-ai/src/decision/client.rs:169-304` | Pooled client, protocol binding, timeout, cancellation, no automatic retry or chat fallback |
| Decision provider configuration binding | `crates/ice-cli/src/decisions.rs:42-105` | Select a named `decisionProviders` entry and existing credential configuration |
| Native model-facing memory search | `crates/ice-cli/src/cognee/command.rs:23-83`; `runtime.rs:1520-1573` | `cognee_search` already exists and respects runtime enablement/tool policy |
| Bundled Cognee skills | `cognee/command.rs:320-350`; `builtin_extensions.rs:173-176` | Search, remember, and sync/doctor instruction files are materialized and included in resource loading |
| Model-visible skill catalog | `crates/ice-harness/src/skills.rs:105-141`; `session.rs:640-696` | Descriptions/locations go into the system-prompt listing; hidden model-invocation skills are excluded |
| Full explicit skill invocation | `crates/ice-harness/src/skills.rs:85-99`; `interactive_tui/run.rs:336-377` | `$name` and `/skill:name` expand to full instruction bodies |
| Name-based skill autocomplete | `interactive_tui/autocomplete.rs:5-50,158-176` | Existing prefix/name selection UI, not semantic task-to-skill recommendation |
| Current MCP tool discovery | `crates/ice-cli/src/mcp/proxy.rs:90-119,400-484` | Search, describe, server instructions, and invocation of tools not directly registered |
| Local MCP ranking | `mcp/proxy.rs:487-499` and ranking implementation below it | Existing names/server/descriptions/keywords can produce a cheap shortlist |
| Multi-call MCP script support | `mcp/script.rs:1-7,25-29,172-181` | Existing tool orchestration facility; not a reason to create another dispatcher |

A search for `decision_client(` under current `crates/ice-cli/src` returned only the definition in `decisions.rs:44`. No Cognee caller appeared. This is evidence about the inspected checkout, not a claim that no other branch or external tool can call TypeSafe.

The general decision timeout constant is 120 seconds (`ice-ai/src/decision/client.rs:18-19`). Callers already supply a timeout to `decide`; the memory feature should supply its own short deadline rather than inherit that generic value. `decision_client()` loads configuration and creates the bound client, so call it on configuration change/startup and reuse the client instead of rebuilding it for every event.

## 3. Skill inventory actually inspected

The global native skill directory contained these four `SKILL.md` paths when inspected:

| Path under `~/.icers/agent/` | Declared purpose |
|---|---|
| `skills/context-mode/SKILL.md:1-18` | Process large outputs with context-mode tools |
| `skills/ctx-search/SKILL.md:1-7` | Search previously indexed project/document/session content |
| `skills/ctx-stats/SKILL.md:1-9` | Show context-mode usage and savings statistics |
| `skills/jev-search/SKILL.md:1-10` | Live web search through the Jev search CLI |

Three additional materialized Cognee skills were present:

- `ice-cognee/skills/cognee-search/SKILL.md`: prior decisions/preferences/history, native search tool or slash command.
- `ice-cognee/skills/cognee-remember/SKILL.md`: categories `user_context`, `project_docs`, and `agent_actions`; manual permanent remembering.
- `ice-cognee/skills/cognee-sync/SKILL.md`: improve, status/doctor, pending and uncertain queue recovery.

This is not an exhaustive inventory of all package/plugin/project skills or `~/.agents`. No application was launched to reconstruct every historical prompt. File existence establishes presence at the inspected path; it does not prove dependencies/credentials work or that a past model saw its descriptor, read its complete body, or used it.

The Connectme workspace's initial zero-skill inventory was a separate tool's workspace inventory; it does not establish that icers has zero skills. The native runtime explicitly adds Cognee builtin paths (`session.rs:553-567`). Keep these inventories conceptually separate.

## 4. Discovery roots and the historical planning-skill miss

Native resource discovery currently builds project `.icers` and global agent-directory paths, appends explicit skill arguments, plugin-discovered paths, and builtin paths, then loads with source precedence (`session.rs:553-567`). `resource_dirs.rs:43-60` defines project and global resource roots. First-registration name deduplication makes project resources override later sources (`resource_dirs.rs:113-141`).

The observability audit's planning-skill episode shows why this matters. A 4,375 ms memory lookup returned old autocomplete discussion. The assistant then reported no dedicated planning skill in its current session, and the user directed it to check `~/.agents`.

A useful fix is to support an explicit, trusted additional skill source through the current loader and show why a source is or is not included. It is not to automatically scan every home directory. Preserve project/global precedence, source identity and hidden-skill behavior; provide a diagnostic such as “discovered at this configured path, descriptor available, full instructions not loaded.”

For the request “which planning skills do I have?”, enumerate/filter the current catalog locally. No JEV call or Cognee recall is needed. For a substantive task where two applicable skills have overlapping descriptions, a JEV Choice over the bounded shortlist can be useful, provided an independent applicability check permits “none.”

## 5. Distinguish availability from context coverage

The loader reads `Skill.content` into the host, but the system prompt contains only name, description and location. Thus four states are needed:

1. **Discovered:** host has a valid descriptor/body at a known source revision.
2. **Advertised:** the model-visible prompt includes the descriptor, with automatic-invocation permission.
3. **Full instructions present:** the effective context contains the complete matching skill version from explicit invocation or verified complete read.
4. **Used:** an actual task action was performed under that skill, a separate and weaker-to-measure claim.

Historical invocation alone does not establish state 3 after compaction. A partial `read` is not a complete load. Different sources can have the same display name. Track IDs/source hashes and branch/context generation rather than one session-wide `loaded` boolean.

Retain the existing full-body formatting and explicit invocation semantics. Suggest only an absent body or newly relevant revision. When the body is already present, reuse it without another hint/load. Automatic suggestions must respect `disable_model_invocation`; explicit user invocation remains possible.

## 6. Preserve original task intent separately from expanded resources

`expand_skill_references` replaces `$name` or `/skill:name` with the full skill content (`interactive_tui/run.rs:336-377`). The streaming path obtains resources and passes the expanded text onward (`run.rs:459-461`). The Cognee prompt hook currently receives the newest prompt and uses it directly as the recall query (`cognee/runtime.rs:2322-2325`).

The database confirms the practical consequence: one query began with an expanded `context-mode` skill block, took 6,269 ms, and retrieved historical copies of those rules. The current instructions were already present.

Introduce an explicit `original_user_intent`/resource-expansion boundary, carried in host metadata rather than reconstructed by stripping arbitrary markup from a mixed string. The recall query should be derived from the original request plus relevant task entities. Loaded skill IDs/hashes are structured context-coverage inputs. Do not transmit a large expanded body to JEV merely to decide whether its own instructions should be recalled.

Old sessions without this metadata require conservative handling: recognize only well-formed host-generated resource envelopes where possible, otherwise avoid making an overconfident semantic query from the expansion. Keep original persisted messages readable and model input unchanged by a metadata migration.

## 7. Native and MCP memory paths need one policy view

The native built-in exposes search. The historical MCP catalog also exposed memory operations, including `cognee_remember`; do not claim the agent has no possible write tool merely because the native module registers only `cognee_search`.

The important gap is a shared curation and outcome contract. A memory proposal should carry source entries, category, scope, and revision and flow into the same durable operation policy. If the current MCP write can bypass selective rejection, it must be classified as an explicit action or routed through a policy-aware adapter. Do not silently apply new automatic restrictions to a direct user-authorized write. Conversely, a model-generated automatic candidate is not blanket permission to bypass the configured policy.

Add a proposal tool only if the available path cannot provide provenance-aware local candidate submission. It should enqueue a candidate, not announce that the remote graph already contains it. Avoid a new unrestricted write endpoint and a second retry queue.

Historical manual retrieval showed two distinct failures: a `cognee_recall` HTTP 404 after about 40 seconds and a native `cognee_search` timeout after 10 seconds. The native path already returns `details.error`, but its wrapper returns `Ok(AgentToolResult)` (`command.rs:80-83`). Propagate the existing semantic outcome into reporting. Do not count `state=done` as retrieval success, or repeatedly suggest the broken path because its catalog entry exists.

## 8. Suggested routing examples

| User/task situation | First action | JEV role |
|---|---|---|
| “Build this branch so I can try it” with known repository | Existing local build workflow/skill; inspect current instructions | None |
| “How many commits did we make?” | Current session and Git state | None |
| “What did we decide about retries yesterday?” | Current-context check, then scoped history search if needed | Optional result ambiguity only |
| “Which planning skill is available?” | Current skill catalog and configured roots | None |
| Long diagnostic output needs repeated search | Suggest `context-mode` or `ctx-search` only if relevant and absent | Only resolve an ambiguous shortlist |
| User explicitly revises a persistent constraint | Create a source-bound correction candidate | Optional classify/relate known candidates; code applies scope/policy |
| A previously failed workaround is proposed again | Retrieve the scoped failure lesson and verify conditions | Optional relevance judge; no automatic command execution |
| No known candidate fits | Return no suggestion; preserve direct search escape hatch | Abstain rather than force a Choice |

These are proposed behavior, not demonstrated performance improvements. The evaluation protocol must test whether suggestions reduce effort and remain useful after accounting for discovery, inference, and additional context tokens.

## 9. Upstream skills

Current upstream research identifies TypeSafe's `typesafe-ai` skill and Cognee's recall, session-improvement, and performance skills. They may help the implementer understand the API. They were not installed or applied to the user's runtime during this task. Existing bundled Cognee skills should be improved at their source in `command.rs` rather than editing materialized copies that the runtime rewrites.

See [upstream research](upstream-research.md) for official links and version cautions. No recommendation here requires installing all of them or adding their bodies to every model prompt.
