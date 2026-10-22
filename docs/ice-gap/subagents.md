# Subagents

Source-only audit of TypeScript subagent definitions, permissions, isolation, persistent child sessions, runtime controls and result contracts against the frozen Rust checkout.

## SUB-001 - Strict native Markdown and JSON agent definitions with source identity

### 1. Title and status

Strict native Markdown and JSON agent definitions with source identity. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/subagents/definitions/schema.ts:1-164 NativeAgentFrontmatterSchema,JsonAgentDefinitionSchema; ice:packages/coding-agent/src/subagents/definitions/parse.ts:24-234 parseNativeAgentMarkdown,parseJsonAgentDefinitions,canonicalJson; ice:packages/coding-agent/src/utils/frontmatter.ts:1-40; ice:packages/coding-agent/src/utils/redact.ts:1-9; ice:packages/coding-agent/test/subagents-v2-definitions.test.ts:51-103; ice:packages/coding-agent/test/subagents-v2-contracts.test.ts:23-88

Exported symbols: `AgentDefinitionIssue` (`ice:packages/coding-agent/src/subagents/definitions/parse.ts:24`), `LEGACY_AGENT_KEY_REPLACEMENTS` (`ice:packages/coding-agent/src/subagents/definitions/parse.ts:39`), `AGENT_FILE_MAX_BYTES` (`ice:packages/coding-agent/src/subagents/definitions/parse.ts:54`), `hashAgentSource` (`ice:packages/coding-agent/src/subagents/definitions/parse.ts:56`).

Exported symbols: `AGENT_NAME_PATTERN` (`ice:packages/coding-agent/src/subagents/definitions/schema.ts:11`), `AGENT_THINKING_LEVELS` (`ice:packages/coding-agent/src/subagents/definitions/schema.ts:13`), `AgentThinkingLevel` (`ice:packages/coding-agent/src/subagents/definitions/schema.ts:14`), `AGENT_ISOLATION_MODES` (`ice:packages/coding-agent/src/subagents/definitions/schema.ts:16`), `AgentIsolationMode` (`ice:packages/coding-agent/src/subagents/definitions/schema.ts:17`), `AGENT_DEFINITION_LIMITS` (`ice:packages/coding-agent/src/subagents/definitions/schema.ts:19`), `AGENT_DEFINITION_SOURCES` (`ice:packages/coding-agent/src/subagents/definitions/schema.ts:32`), `AgentDefinitionSource` (`ice:packages/coding-agent/src/subagents/definitions/schema.ts:41`), `NativeAgentFrontmatter` (`ice:packages/coding-agent/src/subagents/definitions/schema.ts:115`), `JsonAgentDefinition` (`ice:packages/coding-agent/src/subagents/definitions/schema.ts:125`), `ResolvedAgentDefinition` (`ice:packages/coding-agent/src/subagents/definitions/schema.ts:150`).

### 3. What it does

Users define named agents in Markdown or JSON with explicit prompts, tools, model preferences, permissions and resources. Invalid definitions remain diagnostic records instead of becoming callable agents.

### 4. How it works

The native parser treats the frontmatter name as identity, independent of the filename, and validates a strict schema before creating a frozen definition. It retains the source hash and source attribution so discovery, model listing and later child acceptance can detect which definition was selected.

Frontmatter parsing normalizes CRLF and lone CR to LF; it recognizes an opening prefix '---' only at byte/string start and the first '\n---' delimiter after offset 3, parses the intervening YAML and trims the extracted body. No delimiter or no frontmatter.name means not-agent. A YAML parse failure is invalid even without a usable name. Native fields are checked with TypeBox; unknown keys produce exact diagnostics, with replacement suggestions for fallbackModel/fallback_model, thinkingLevel, top_p/top-p, mcpTools/mcp_tools, adapters, startupTimeoutMs/startup-timeout-ms and iceUnsafeHostExec/ice-unsafe-host-exec. The startupTimeoutMs suggestion still names legacy ice.subagents.defaults.startupTimeoutMs; do not treat that suggestion as an active V2 setting. Empty trimmed Markdown body is invalid. Issues retain a name only when it matches the agent-name pattern. Diagnostics deduplicate; ordinary schema errors stop once the result reaches 16, but the additionalProperties branch can append multiple messages before that check.

JSON input undefined is empty; null/nonobject/array produces an expected-object-map issue. Each own map entry supplies its identity and is validated independently, retaining valid neighbors. A JSON entry may not repeat name; description and prompt are required. JSON prompt is passed through without a trim/nonblank check beyond the schema's minLength. Definition construction strips name/description/prompt from fields, normalizes a tools comma string by split/trim/drop-empty, redacts the prompt using the shared credential-pattern redactor, shallow-freezes the definition/fields/diagnostics, and preserves limited=false. Markdown sourceHash is SHA-256 of the original input content; JSON sourceHash is SHA-256 of canonical field JSON. Canonical JSON sorts object keys, omits undefined object properties, preserves array order and otherwise uses JSON.stringify. It is not a signature or authority grant.

Agent names match ^[a-z0-9][a-z0-9-]{0,63}$. Description maxLength1024; tools may be a list or comma string(maxLength4096); selected lists have at most 64 entries of 1..256 characters; model is one 1..256-character string or 1..8 such strings. initialPrompt maxLength4096; tags at most 8 of 1..32 characters; temperature 0..2 and topP0..1. Permission rules and presentation colors use shared vocabularies. The declared maxPromptBytes=65536 is a TypeBox maxLength for JSON prompt, not an explicit UTF-8 byte check in this parser. Markdown body has no separate 64 KiB parser check; discovery separately caps the entire file at 128 KiB. Later preflight/context limits must be audited separately rather than inferred from these names.

### 5. Interfaces

Native Markdown requires {name, description} plus a nonempty body. JSON map entries require {description, prompt}. Optional fields: tools, disallowedTools, model, thinking, permissionMode, permissions:{allow?, ask?, deny?}, background, isolation, contextFiles, skills, prompts, context, mcp, requiredMcpServers, hooks, initialPrompt, spawns, color, hidden, tags, temperature, topP. Reserved provenance fields: ice-agent-pack(maxLength256), ice-agent-pack-source(maxLength1024), ice-agent-pack-sha256(^[a-f0-9]{64}$). thinking=off|minimal|low|medium|high|xhigh|max|ultra; isolation=none|worktree; permissionMode=inherit|plan|review|build|bubble. ParsedAgentSource=not-agent|invalid{issue}|agent{definition}; issue availability=invalid|untrusted. Resolved definition carries name, description, prompt, fields, source, sourcePath?, sourceHash, diagnostics, limited. No provider is contacted by parsing.

Declaration supplement: AgentDefinitionIssue carries optional name/sourcePath, source, availability=invalid|untrusted and readonly diagnostics. ResolvedAgentDefinition carries name, description, prompt, fields=Omit<NativeAgentFrontmatter, name|description>, source, optional sourcePath, sourceHash, readonly diagnostics and limited. NativeAgentFrontmatter and JsonAgentDefinition are Static<typeof ...Schema> aliases; AgentThinkingLevel and AgentIsolationMode derive from their exported literal arrays. These aliases preserve the schema contract above and add no alternate parser.

### 6. Dependencies

Requires YAML parsing, strict schema validation, SHA-256 and the shared credential-pattern redactor. Settings storage is SET-020; file discovery and precedence are SUB-003. Model, tool and authority resolution are later runtime stages and must not be hidden inside parsing.

### 7. ice-rust today

rust:crates/ice-cli/src/child_agents.rs:207-251 accepts a bounded task string and free-form role label for a plugin-owned run; rust:crates/ice-cli/src/child_agents.rs:288-337 creates an in-memory child and a generic delegation prompt. rust:crates/ice-harness/src/agent_harness/child.rs:74-172 snapshots the existing parent model, tools, prompt/resources and hooks. Neither path loads or resolves named Markdown/JSON agent definitions. Behavior-based searches across crates for AgentDefinition/agent_definition and V2 tool/contract names found no equivalent definition catalog.

### 8. Port notes

Target a native subagent definition/policy/result module composed by ice-cli, with serializable Rust structs and the workspace JSON types; no provider service is required for these local rules. Add a pure definition model/parser in a subagent module or companion extension using serde plus an appropriate YAML parser and explicit schema checks. Preserve invalid/not-agent distinctions, source hashes and exact field names; never deserialize unknown fields permissively. Separate character/schema bounds from UTF-8 byte limits and deliberately enforce/document any stronger Rust bound instead of claiming byte parity. Port the named-invalid-file, unknown-key, JSON-map and field-limit test contracts; no tests were executed in this audit.

### 9. Size and priority

Estimated Rust implementation: **240-390 lines** (estimate, not measured source). **P1** - Named reusable agents and trustworthy diagnostics depend on a stable strict definition format.

## SUB-002 - Opt-in Claude agent compatibility with explicit model aliases

### 1. Title and status

Opt-in Claude agent compatibility with explicit model aliases. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/subagents/definitions/schema.ts:127-147 CLAUDE_COMPAT_AGENT_KEYS; ice:packages/coding-agent/src/subagents/definitions/parse.ts:236-361 parseClaudeCompatAgentMarkdown; ice:packages/coding-agent/src/subagents/definitions/load.ts:110-123; ice:packages/coding-agent/test/subagents-v2-definitions.test.ts:105-124; ice:packages/coding-agent/test/subagents-v2-definitions.test.ts:266-282

### 3. What it does

Users can opt into reading .claude/agents files without silently granting unsupported capabilities. Recognized fields are translated, unsupported fields are reported, and unresolved model aliases leave the definition unavailable.

### 4. How it works

Compatibility parsing maps recognized Claude fields into the native definition schema and records warnings for discarded fields. It remains a project source with project trust and lower precedence than native project agents.

Invalid YAML and absent frontmatter.name follow the native parser's invalid/not-agent behavior. Known keys are name, description, tools, disallowedTools, model, effort, permissionMode, mcpServers, hooks, skills, initialPrompt, memory, background, isolation, color; unknown keys produce warnings rather than native-schema unknown-key failure. Tool lists accept comma strings; exact case-sensitive mappings are Read→read, Grep→grep, Glob→find, LS→ls, Bash→bash, Edit→edit, MultiEdit→edit, Write→write, with unknown names preserved for later resolution.

For string model, trim first: case-insensitive inherit becomes inherit; otherwise use an exact global compat.modelAliases key, then accept a reference containing '/', else warn and set limited=true. A nonstring model is not copied. effort is copied to thinking only for an exact supported thinking value. permissionMode copies only plan or bubble; default is silently omitted and other supplied values warn and are discarded. background and initialPrompt copy through; skills normalize as a tool-style comma list; only worktree isolation is copied, with other values warned. Colors map red→error, green→success, yellow/orange→warning, blue/purple/pink/cyan→accent; other colors are dropped. mcpServers, hooks, memory always warn and are not copied. Native schema validation and nonempty trimmed body still apply. Source hash uses original content, prompt is redacted, source is project-compat and warning diagnostics remain with the accepted definition. limited definitions are excluded from callable listing.

### 5. Interfaces

Global opt-in consumer: ice.subagents.compat.claudeAgents (default false) and compat.modelAliases (string map), resolved by V2 settings. Source path pattern .claude/agents/**/*.md. parseClaudeCompatAgentMarkdown(content, sourcePath, modelAliases)->ParsedAgentSource. No implicit model alias service, permission upgrade, MCP connection or legacy delegate tool is installed.

### 6. Dependencies

`SET-020`, `SUB-001`

Uses the discovery/trust pipeline and current model/tool preflight. Compatibility warnings must remain visible in agent diagnostics; they do not constitute approval.

### 7. ice-rust today

rust:crates/ice-cli/src/child_agents.rs:207-251 accepts a bounded task string and free-form role label for a plugin-owned run; rust:crates/ice-cli/src/child_agents.rs:288-337 creates an in-memory child and a generic delegation prompt. rust:crates/ice-harness/src/agent_harness/child.rs:74-172 snapshots the existing parent model, tools, prompt/resources and hooks. Neither path loads or resolves named Markdown/JSON agent definitions. Behavior-based searches across crates for AgentDefinition/agent_definition and V2 tool/contract names found no equivalent definition catalog. No inspected native child action accepts a Claude-agent path or compatibility alias map.

### 8. Port notes

Target a native subagent definition/policy/result module composed by ice-cli, with serializable Rust structs and the workspace JSON types; no provider service is required for these local rules. Implement as an optional adapter feeding the native definition parser, with an explicit closed mapping table and source=project-compat. Keep unresolved aliases unavailable and retain warnings. Do not reuse a general permissive Markdown loader that drops diagnostics or upgrades permission modes. Future tests should preserve exact tool/color/permission mappings and native-over-compat precedence.

### 9. Size and priority

Estimated Rust implementation: **100-180 lines** (estimate, not measured source). **P2** - Compatibility is useful for existing agent files but must not widen authority or guess provider mappings.

## SUB-003 - Bounded agent discovery, trust filtering and whole-definition precedence

### 1. Title and status

Bounded agent discovery, trust filtering and whole-definition precedence. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/subagents/definitions/discover.ts:1-183; ice:packages/coding-agent/src/subagents/definitions/paths.ts:1-51; ice:packages/coding-agent/src/subagents/definitions/load.ts:1-157; ice:packages/coding-agent/src/subagents/definitions/precedence.ts:1-74; ice:packages/coding-agent/test/subagents-v2-definitions.test.ts:127-293

Exported symbols: `AgentDirectoryOptions` (`ice:packages/coding-agent/src/subagents/definitions/discover.ts:17`), `resolveProjectStopBoundary` (`ice:packages/coding-agent/src/subagents/definitions/discover.ts:66`), `DiscoveredAgentFile` (`ice:packages/coding-agent/src/subagents/definitions/discover.ts:101`), `AgentFileDiscovery` (`ice:packages/coding-agent/src/subagents/definitions/discover.ts:107`).

Exported symbols: `AgentDefinitionLoadOptions` (`ice:packages/coding-agent/src/subagents/definitions/load.ts:23`), `LoadedAgentDefinitions` (`ice:packages/coding-agent/src/subagents/definitions/load.ts:39`).

Exported symbols: `SubagentResourceResolutionOptions` (`ice:packages/coding-agent/src/subagents/definitions/paths.ts:12`), `getSubagentGlobalAgentsDir` (`ice:packages/coding-agent/src/subagents/definitions/paths.ts:27`), `hashSource` (`ice:packages/coding-agent/src/subagents/definitions/paths.ts:34`), `findNearestDirectory` (`ice:packages/coding-agent/src/subagents/definitions/paths.ts:39`).

Exported symbols: `AGENT_SOURCE_PRECEDENCE` (`ice:packages/coding-agent/src/subagents/definitions/precedence.ts:12`), `ShadowedAgentDefinition` (`ice:packages/coding-agent/src/subagents/definitions/precedence.ts:24`), `AgentDefinitionSet` (`ice:packages/coding-agent/src/subagents/definitions/precedence.ts:29`), `resolveAgentDefinitionSet` (`ice:packages/coding-agent/src/subagents/definitions/precedence.ts:37`).

### 3. What it does

The agent catalog combines installed, user, project, settings, startup and managed definitions predictably. It lists invalid, untrusted, disabled and shadowed definitions while preventing project content from silently becoming trusted authority.

### 4. How it works

Discovery walks bounded directories and deduplicates physical Markdown files before applying name precedence. Loading preserves separate issue/shadow/disabled records and caches only under an explicit caller key, so catalog refresh and child contract reuse can be controlled separately.

The default global definition root is the sibling agents directory of the normal agent directory: ~/.ice/agents when the normal agent directory is ~/.ice/agent. An environment override or an explicit effective agentDir differing from getAgentDir uses <agentDir>/agents. The code consumes ENV_AGENT_DIR through legacy-compatible getIceEnv; environment-name resolution belongs to config/compat. Project directories are collected nearest-first from canonical cwd, with at most 64 ancestor steps and no inclusion of canonical home. If no Git boundary is found, only cwd is considered. A nested repository inside the session project widens the stop boundary to the session project's root; otherwise the nearest repository root wins. Linked worktrees fall back to the standard main checkout's agent directory when the worktree root lacks its own directory, even if a nearer nested directory was found. Fallback requires common Git metadata to identify an ordinary main/.git layout.

For each existing root, recurse through at most depth 8 (root depth 0), sort names with localeCompare, skip dot-prefixed entries and accept case-sensitive .md regular files. Symlinks may be followed only when their canonical target stays inside the canonical root; escaping targets produce a diagnostic. A shared seen set uses device:inode when inode is truthy, otherwise canonical path, so physical files load once across roots. Count unique candidate Markdown files before size/read checks; after 512 files in one root stop its remainder and report the cap. Files above 128 KiB are skipped. Inaccessible directory/stat entries are silently skipped; read failure on an otherwise accepted file is diagnostic. A file enters seen before count/size/read checks. Directory symlink recursion has the depth bound but no separate visited-directory set.

Actual collection order is managed files first, then native project, opt-in project-compat, user, package, user-settings JSON, project-settings JSON and flag JSON; managed-first protects inode dedup. Untrusted project files are still discovered/parsed but yield only an untrusted name/path/diagnostic and never an active definition/prompt. Untrusted project JSON contributes untrusted records for valid parsed definitions and omits its parser issues. Precedence by name is builtin<package<user<user-settings<project-compat<project<project-settings<flag<managed. Within the same source, first input wins; nearer project roots therefore win. Replacement is whole-definition, never field merge. Every previous winner becomes a shadow record. Winners sort by localeCompare name; disabled names are removed unless the winning source is managed. AgentDefinitionCache holds one key/value and refreshes only when key changes or invalidate is called; it does not itself install watchers.

### 5. Interfaces

loadAgentDefinitions({cwd, userAgentsDir, projectTrusted, projectRoot?, managedAgentsDir?, packageAgentDirs?, settingsAgents?:{user?, project?}, flagAgents?, disabledAgents?, compat?, home?})->{active, shadowed, disabled, issues, searchedDirectories}. findProjectAgentDirectories({cwd, projectRoot?, subdir?, home?}), listAgentMarkdownFiles(roots, seen?). Sources/paths: ~/.ice/agents, custom <agentDir>/agents,.ice/agents,.claude/agents, packageAgentDirs, explicit managedAgentsDir; settings ice.subagents.agents and disabledAgents; startup --agents. No managed directory is searched unless supplied. All paths here were read as source strings only; actual user configuration was not accessed.

Discovery/load DTOs: AgentDirectoryOptions={cwd, projectRoot?, subdir?, home?}; DiscoveredAgentFile={path, root, content}; AgentFileDiscovery={files, diagnostics:[{path, message}]}. AgentDefinitionLoadOptions adds userAgentsDir, projectTrusted, managedAgentsDir?, packageAgentDirs?, settingsAgents?:{user?, project?}, flagAgents?, disabledAgents?, compat?:{claudeAgents?, modelAliases?}, home?. LoadedAgentDefinitions extends AgentDefinitionSet with searchedDirectories. AgentDefinitionSet={active:ReadonlyMap<name, definition>, shadowed:[{definition, shadowedBy:{source, sourcePath?}}], disabled, issues}. SubagentResourceResolutionOptions also carries agentDir?, projectTrusted?, includeSelf?, delegableTools?, parentActiveTools?, unsafeHostExec?; the optional invocation capabilities project availability and do not create a new source-precedence layer.

### 6. Dependencies

`SET-003`, `SET-020`, `SUB-001`, `SUB-002`

Requires project trust, configured global/project roots and Git metadata resolution. Package-discovery integration and watcher/settings-epoch invalidation are caller responsibilities to link from CLI/session; no Git mutation occurs during discovery.

### 7. ice-rust today

rust:crates/ice-cli/src/child_agents.rs:207-251 accepts a bounded task string and free-form role label for a plugin-owned run; rust:crates/ice-cli/src/child_agents.rs:288-337 creates an in-memory child and a generic delegation prompt. rust:crates/ice-harness/src/agent_harness/child.rs:74-172 snapshots the existing parent model, tools, prompt/resources and hooks. Neither path loads or resolves named Markdown/JSON agent definitions. Behavior-based searches across crates for AgentDefinition/agent_definition and V2 tool/contract names found no equivalent definition catalog. The existing Rust package/resource loader and AGENTS.md prompt resources are not an equivalent per-name agent-definition catalog. No behavior match was found for V2 name precedence, managed locks or agent directories in the inspected crates.

### 8. Port notes

Target a native subagent definition/policy/result module composed by ice-cli, with serializable Rust structs and the workspace JSON types; no provider service is required for these local rules. Add a bounded filesystem discovery service with explicit source attribution, physical-file dedup, project trust filtering and deterministic whole-record precedence. Use canonical paths and device/inode information where available; document platform fallback. Reuse Rust's package resource roots where appropriate but keep agent catalogs separate from instruction files. Preserve all diagnostic buckets and caller-controlled invalidation. Test nested repositories, linked worktrees, escaping symlinks, same-source order, managed overrides and untrusted project input.

### 9. Size and priority

Estimated Rust implementation: **330-520 lines** (estimate, not measured source). **P1** - Predictable discovery and trust boundaries are prerequisites for selecting and freezing a named child agent.

## SUB-004 - Built-in agents and incremental model-visible catalog

### 1. Title and status

Built-in agents and incremental model-visible catalog. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/subagents/definitions/builtin.ts:1-70; ice:packages/coding-agent/src/subagents/definitions/listing.ts:1-93; ice:packages/coding-agent/test/subagents-v2-definitions.test.ts:195-209; ice:packages/coding-agent/test/subagents-v2-definitions.test.ts:295-355

Exported symbols: `getBuiltinAgentDefinitions` (`ice:packages/coding-agent/src/subagents/definitions/builtin.ts:59`).

Exported symbols: `AgentListingState` (`ice:packages/coding-agent/src/subagents/definitions/listing.ts:16`), `formatAgentListingLine` (`ice:packages/coding-agent/src/subagents/definitions/listing.ts:43`), `AgentListingDelta` (`ice:packages/coding-agent/src/subagents/definitions/listing.ts:47`).

### 3. What it does

The model learns which agents are callable without changing the agent tool schema on each catalog update. Built-in general, exploration, planning, review and worker roles provide useful defaults that users can replace through the ordinary precedence rules.

### 4. How it works

Built-ins are ordinary lowest-precedence definitions with stable source hashes and no child-spawn names. The listing helper filters unavailable definitions, computes changes by name/source hash and emits a compact custom-message description for the caller to insert into model context.

Built-in names in order are general, explore, plan, review, worker. general requests inherited child-safe tools. explore and plan explicitly request read, grep, find, ls, set permissionMode=plan and contextFiles=false. review uses the same four tools with permissionMode=review. worker uses permissionMode=build and isolation=worktree. All set spawns=[]; prompts require bounded-scope work and concrete evidence/reporting, but actual authority comes from runtime guards. Each returned record/fields/diagnostics is frozen; source=builtin and sourceHash hashes canonical builtin content.

callableAgentDefinitions removes hidden and limited definitions. Every requiredMcpServers entry must be a case-insensitive substring of at least one available server name; this helper does not use glob matching or connect a server. Tool summary displays requested list items minus exact disallowed string matches; for omitted or ['*'] lists it says All tools or All tools except. That description is not the effective runtime permission set.

Listing state is Map<name, sourceHash>. With no previous state, emit full=true and every callable record as added; otherwise report newly added, same-name/different-hash changed and missing-name removed. next records every current callable name. Unchanged nonfull delta returns undefined. Full empty text explicitly says no agents are available. Message includes header, agent lines, optional removed names and an agent_control action=agents footer. Default target is 8192 UTF-8 bytes: first try full descriptions/tool summaries, then descriptions sliced to 80 JS code units, then one names-only line. The final names-only fallback and removed line are returned without a final byte check, so the helper does not guarantee the advertised cap for arbitrarily large catalogs/removal sets; the inspected 60-agent test proves only its fixture. Callers own listing-state reset and delivery around compaction/navigation/model changes.

### 5. Interfaces

BUILTIN_AGENT_NAMES=[general, explore, plan, review, worker]; AGENT_LISTING_CUSTOM_TYPE='ice_agent_listing'; AGENT_LISTING_MAX_BYTES=8192. computeAgentListingDelta(seen?:ReadonlyMap<string, string>, callable)->{full, added, changed, removed, next}; renderAgentListingMessage(delta, maxBytes?)->string|undefined. The custom message is the catalog interface; no additional listing model tool is declared by this module.

AgentListingState is ReadonlyMap<string, string> (advertised name to source hash). AgentListingDelta={full:boolean, added:readonly ResolvedAgentDefinition[], changed:readonly ResolvedAgentDefinition[], removed:readonly string[], next:AgentListingState}. formatAgentListingLine emits '- name: description (Tools: summary)'; getBuiltinAgentDefinitions returns fresh frozen definition/fields/diagnostics projections.

### 6. Dependencies

`SUB-001`, `SUB-003`

Requires current parent-exposed MCP server names, session custom-message insertion and context/compaction invalidation; those callers must be linked to the later runtime/session entries. This helper must not be treated as a source of permission grants.

### 7. ice-rust today

rust:crates/ice-cli/src/child_agents.rs:207-251 accepts a bounded task string and free-form role label for a plugin-owned run; rust:crates/ice-cli/src/child_agents.rs:288-337 creates an in-memory child and a generic delegation prompt. rust:crates/ice-harness/src/agent_harness/child.rs:74-172 snapshots the existing parent model, tools, prompt/resources and hooks. Neither path loads or resolves named Markdown/JSON agent definitions. Behavior-based searches across crates for AgentDefinition/agent_definition and V2 tool/contract names found no equivalent definition catalog. Native child role text is not a resolved builtin catalog or model-context delta protocol. No ice_agent_listing equivalent was found in the inspected source.

### 8. Port notes

Target a native subagent definition/policy/result module composed by ice-cli, with serializable Rust structs and the workspace JSON types; no provider service is required for these local rules. Implement builtin records and a pure delta renderer below session integration. Preserve source-hash change semantics and exact availability conditions. A Rust implementation should enforce the final byte budget as well as the intermediate attempts, with an explicit truncation marker and consistent next-state policy so unshown agents are not silently considered delivered. Keep catalog changes out of the model tool schema for cache stability.

### 9. Size and priority

Estimated Rust implementation: **140-230 lines** (estimate, not measured source). **P1** - The parent model needs an accurate current catalog to select existing definitions and avoid unnecessary discovery calls.

## SUB-005 - Strict scoped V2 settings and monotonic numeric limits

### 1. Title and status

Strict scoped V2 settings and monotonic numeric limits. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/subagents/v2/settings.ts:1-194; ice:packages/coding-agent/src/subagents/settings.ts:217-261

Exported symbols: `DEFAULT_FOREGROUND_DETACH_MS` (`ice:packages/coding-agent/src/subagents/v2/settings.ts:17`), `DEFAULT_CHILD_BASH_TIMEOUT_MS` (`ice:packages/coding-agent/src/subagents/v2/settings.ts:19`), `DEFAULT_CHILD_TURN_TIMEOUT_MS` (`ice:packages/coding-agent/src/subagents/v2/settings.ts:21`), `DEFAULT_CHILD_IDLE_TIMEOUT_MS` (`ice:packages/coding-agent/src/subagents/v2/settings.ts:23`), `DEFAULT_CHILD_IDLE_ALERT_MS` (`ice:packages/coding-agent/src/subagents/v2/settings.ts:25`), `SubagentsV2Settings` (`ice:packages/coding-agent/src/subagents/v2/settings.ts:89`).

### 3. What it does

Resolve the active child defaults, timeouts and permission rules while rejecting invalid settings and preventing project settings from weakening selected global ceilings.

### 4. How it works

The V2 resolver validates global and trusted-project scopes independently, then composes a typed result with diagnostics and deprecation notices. Storage is separate: this entry owns accepted keys, defaults and exact merge semantics.

The accepted keys are enabled, v2, defaultAgent, maxDepth, maxConcurrent, askTimeoutMs, foregroundDetachMs, bashTimeoutMs, turnTimeoutMs, idleTimeoutMs, idleAlertMs, permissions, agents, disabledAgents, compat. Undefined/null is an empty scope. Nonrecord/array is invalid. Undefined-valued keys are skipped. Unknown keys diagnose; deprecated defaults, allowedRoles, roleDefaults, restrictions, modelSelection, concurrency, routeTiers are ignored with deprecation records. Any validation errors empty that entire scope, not merely the bad field; launch code must separately refuse diagnostics. An untrusted project is not parsed at all. v2 accepts boolean but does not select an alternate implementation.

enabled defaults true and requires neither scope false. defaultAgent takes project, global, then general. maxDepth defaults 0, integer 0..4 and is reserved by runtime; maxConcurrent defaults 4, integer>=1 with no upper schema bound. Depth, concurrency, Bash, turn and idle ceilings use min(global value or bundled default, project value or Infinity). askTimeoutMs is global-only, default 600000, min 1000, max 3600000. foregroundDetachMs defaults 60000, range 0..3600000; idleAlertMs defaults 300000, range 0..86400000. Their shorten-only merge ignores a project 0, lets a positive project enable a globally-disabled 0 timer, otherwise takes the shorter. Therefore current idleAlertMs does accept a trusted-project shortening despite older global-only documentation. Bash timeout default 600000, turn 1800000, idle 600000; each 1000..86400000. Permissions retain user/settings and project/settings origins, agents remain separate maps for source precedence, disabled names union, and compat is global-only.

### 5. Interfaces

Settings namespace ice.subagents. permissions={allow?, ask?, deny?}, each list<=128 rules, rule maxLength640 plus grammar. defaultAgent uses native agent-name grammar. disabledAgents<=64 strings without individual agent-name validation. agents is an unknown-value object map for later definition validation. compat={claudeAgents?:boolean, modelAliases?:Record<string, string>} with alias targets 3..256 characters. resolveSubagentsV2Settings returns enabled/defaults/limits, ruleSets, agents:{user?, project?}, disabledAgents, compat, diagnostics, deprecations. readIceSettingsInput preserves unknown top-level fields; strict parseIceSettings accepts only subagents/hooks and throws combined diagnostics.

SubagentsV2Settings explicitly includes enabled, defaultAgent, maxDepth, maxConcurrent, askTimeoutMs, foregroundDetachMs, bashTimeoutMs, turnTimeoutMs, idleTimeoutMs, idleAlertMs, ruleSets, agents:{user?, project?}, disabledAgents, compat:{claudeAgents, modelAliases}, diagnostics, deprecations:[{scope:global|project, key, message}]. Exported defaults are DEFAULT_FOREGROUND_DETACH_MS=60000, DEFAULT_CHILD_BASH_TIMEOUT_MS=600000, DEFAULT_CHILD_TURN_TIMEOUT_MS=1800000, DEFAULT_CHILD_IDLE_TIMEOUT_MS=600000 and DEFAULT_CHILD_IDLE_ALERT_MS=300000.

### 6. Dependencies

`SET-020`, `SUB-001`

SET-020 owns namespace persistence/accessors; SUB-001 validates individual agent maps. Trust is supplied by SET-003 at the caller.

### 7. ice-rust today

rust:crates/ice-cli/src/child_agents.rs:207-251 parses plugin request maxTurns(default 8,1..32) and timeoutSeconds(default 300,10..1800), while rust:crates/ice-cli/src/child_agents.rs:254-283 enforces a fixed 16 active/retained cap. These are materially different controls. Neither this service nor rust:crates/ice-harness/src/agent_harness/child.rs:74-172 resolves ice.subagents scoped settings, definition maps, diagnostic launch blocking or shorten-only timers.

### 8. Port notes

Target a native subagent definition/policy/result module composed by ice-cli, with serializable Rust structs and the workspace JSON types; no provider service is required for these local rules. Add a typed subagent settings resolver above the existing native child host. Keep strict scope validation and diagnostics separate from storage and runtime launch. Use checked integer durations and explicitly reconcile TypeBox character bounds with Rust byte lengths. Preserve the host's hard resource protection while distinguishing it from the configurable V2 admission limit.

### 9. Size and priority

Estimated Rust implementation: **150-240 lines** (estimate, not measured source). **P1** - Every launch and resume depends on consistent trusted settings and timeout ceilings.

## SUB-006 - Permission rule grammar, tool subjects and conservative shell decomposition

### 1. Title and status

Permission rule grammar, tool subjects and conservative shell decomposition. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/subagents/permissions/rules.ts:1-141; ice:packages/coding-agent/src/subagents/permissions/subjects.ts:1-151; ice:packages/coding-agent/src/subagents/permissions/evaluate.ts:1-112; ice:packages/coding-agent/test/subagents-v2-permissions.test.ts:31-171

Exported symbols: `PermissionRuleSet` (`ice:packages/coding-agent/src/subagents/permissions/evaluate.ts:22`), `PermissionMatch` (`ice:packages/coding-agent/src/subagents/permissions/evaluate.ts:27`), `ruleMatchesSubject` (`ice:packages/coding-agent/src/subagents/permissions/evaluate.ts:33`), `matchPermissionRules` (`ice:packages/coding-agent/src/subagents/permissions/evaluate.ts:42`), `modeLimit` (`ice:packages/coding-agent/src/subagents/permissions/evaluate.ts:60`), `PermissionEvaluation` (`ice:packages/coding-agent/src/subagents/permissions/evaluate.ts:79`).

Exported symbols: `PERMISSION_DECISIONS` (`ice:packages/coding-agent/src/subagents/permissions/rules.ts:9`), `permissionRank` (`ice:packages/coding-agent/src/subagents/permissions/rules.ts:14`), `mostRestrictiveDecision` (`ice:packages/coding-agent/src/subagents/permissions/rules.ts:18`), `PERMISSION_RULE_PATTERN` (`ice:packages/coding-agent/src/subagents/permissions/rules.ts:26`), `PERMISSION_RULE_LIMITS` (`ice:packages/coding-agent/src/subagents/permissions/rules.ts:27`), `PermissionRuleLists` (`ice:packages/coding-agent/src/subagents/permissions/rules.ts:29`), `PermissionRuleScope` (`ice:packages/coding-agent/src/subagents/permissions/rules.ts:36`), `PermissionRuleCarrier` (`ice:packages/coding-agent/src/subagents/permissions/rules.ts:37`), `PermissionRuleOrigin` (`ice:packages/coding-agent/src/subagents/permissions/rules.ts:39`), `AGENT_PERMISSION_MODES` (`ice:packages/coding-agent/src/subagents/permissions/rules.ts:45`), `ParsedPermissionRule` (`ice:packages/coding-agent/src/subagents/permissions/rules.ts:107`).

Exported symbols: `AdapterAccess` (`ice:packages/coding-agent/src/subagents/permissions/subjects.ts:24`), `splitShellCommand` (`ice:packages/coding-agent/src/subagents/permissions/subjects.ts:34`).

### 3. What it does

Compute a decision for every relevant file path, shell fragment, adapter and delegation target, with explicit provenance and mode restrictions.

### 4. How it works

The evaluator combines all matching rules using deny before ask before allow, then applies opaque-command and mode ceilings. A tool invocation is only as permissive as its most restrictive subject. Shell processing is a bounded textual approximation rather than a shell parser.

A rule is a tool glob, optionally followed by a 1..512-character parenthesized subject glob; tool syntax is '*' or an alphabetic first character followed by at most 127 alphanumeric, underscore, dot, asterisk or hyphen characters. parsePermissionRule does not trim. Path glob '*' stays inside one component, '**' crosses components and '**/' also matches no prefix; '?' is one nonseparator character. Nonpath '*' matches any characters across newlines, while '?' uses regex dot. Matching is anchored. Decisions carry matching origin scope(builtin, managed, user, project, cli, session) and carrier(settings, agent-definition, compat-definition, flag, grant). No matching rule uses the supplied fallback. Stable sorting preserves equal-ranked match order.

read/grep/find/ls extract a read path; edit/write extract edit-write. A nonempty string args.path resolves lexically against cwd, otherwise '.'. In-cwd subjects use relative slash paths; outside paths use absolute values and add an external_directory subject. Unknown or unclassified adapters are mutation-adapter, never read. agent tasks contribute delegation subjects using the requested agent names. Bash splits on &&,||, semicolon, newline, unpaired '&' and '|', excluding descriptor operators by the literal regex; it does not honor shell quotes. Any command/process substitution syntax ($(...), backticks,<(,>()) marks every command fragment opaque. File output redirection marks that fragment opaque after allowing descriptor duplication and recognized /dev/null redirection; this cannot serve as complete shell-language validation.

Opaque Bash is capped at ask unless explicit subYolo is active; explicit denies and asks still win. Plan/review allow read, cap external-directory/delegation at ask and hard-deny Bash, edit-write and mutation-adapter. This is stricter than stale reference prose claiming Bash approval in those modes. Session Bash grant prefixes split whitespace, discard tokens beginning '-', then retain a known command/subcommand arity(git/npm/pnpm/yarn/bun/cargo/docker/kubectl/go/uv/pip 2, gh 3; package-manager run 3), otherwise one token; they are not parsed command ASTs.

### 5. Interfaces

PermissionDecision=deny|ask|allow. PermissionSubject={tool, value?, pathMode, subjectClass, opaque?}; classes read|edit-write|bash|mutation-adapter|external-directory|delegation. AgentPermissionMode=inherit|plan|review|build|bubble. APIs globMatches, parsePermissionRule, extractPermissionSubjects, evaluatePermission, evaluatePermissionSubjects, bashGrantPrefix. Evaluation returns decision, matched rules/origins and limitedByMode. Errors from invalid schema belong to SUB-005/SUB-001, not the matcher.

PermissionRuleLists has optional readonly allow/ask/deny arrays. PermissionRuleScope=builtin|managed|user|project|cli|session and PermissionRuleCarrier=settings|agent-definition|compat-definition|flag|grant; PermissionRuleOrigin stores scope/carrier. PermissionRuleSet={origin, rules}; PermissionMatch={rule, decision, origin}; PermissionEvaluation={decision, matches, limitedByMode}. ParsedPermissionRule={tool, pattern?}. AdapterAccess=read-only|mutation|unknown. PERMISSION_DECISIONS is deny, ask, allow in ascending permissiveness; permissionRank returns that index and mostRestrictiveDecision returns the lower-ranked input (left on a tie).

### 6. Dependencies

`SUB-005`

No external service is required by this pure contract.

### 7. ice-rust today

rust:crates/ice-harness/src/agent_harness/child.rs:102-155 limits requested tool names to the parent's current active set and inherits parent hooks. rust:crates/ice-cli/src/child_agents.rs:238-250 validates only a string tool list. Neither path classifies per-call subjects, composes provenance-bearing permission rules, decomposes Bash, creates session-grant prefixes or applies the V2 mode table. Native hook support remains a useful enforcement boundary, not evidence of this policy engine.

### 8. Port notes

Target a native subagent definition/policy/result module composed by ice-cli, with serializable Rust structs and the workspace JSON types; no provider service is required for these local rules. Implement pure rule parsing/subject extraction/decision functions with explicit origins and a table-driven mode ceiling. Port the exhaustive permission fixtures as behavioral contracts. Document textual shell limitations instead of presenting glob grants as shell isolation; ensure a stronger parser, if chosen, is an intentional deviation.

### 9. Size and priority

Estimated Rust implementation: **230-370 lines** (estimate, not measured source). **P0** - Incorrect rule composition can expand child authority or silently deny expected work.

## SUB-007 - Child permission ceilings and source-sensitive authority raisers

### 1. Title and status

Child permission ceilings and source-sensitive authority raisers. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/subagents/permissions/derive-child.ts:1-98; ice:packages/coding-agent/src/subagents/permissions/rules.ts:47-112; ice:packages/coding-agent/test/subagents-v2-permissions.test.ts:173-329

Exported symbols: `ChildPermissionContext` (`ice:packages/coding-agent/src/subagents/permissions/derive-child.ts:25`), `ChildPermissionDecision` (`ice:packages/coding-agent/src/subagents/permissions/derive-child.ts:36`), `deriveChildDecision` (`ice:packages/coding-agent/src/subagents/permissions/derive-child.ts:63`).

Exported symbols: `ChildSubjectClass` (`ice:packages/coding-agent/src/subagents/permissions/rules.ts:48`), `CeilingRaiser` (`ice:packages/coding-agent/src/subagents/permissions/rules.ts:57`), `ChildMutationCeilingEntry` (`ice:packages/coding-agent/src/subagents/permissions/rules.ts:59`).

### 3. What it does

Ensure child decisions never exceed their parent's current decision while allowing only named sources to lift the child-specific default ask ceiling.

### 4. How it works

Derivation starts from the class-specific child ceiling, applies an eligible raiser, narrows by child rules and agent mode, and finally intersects the parent decision. The final parent intersection is unconditional; sub-yolo, worktrees and allow rules cannot defeat it.

Read and delegation inherit the parent ceiling. edit-write and Bash start at ask; edit-write can rise through sub-yolo, user allow, managed allow or worktree isolation, while Bash can rise through sub-yolo, user allow ormanaged allow. Mutation adapters can rise only through user ormanaged allow, not sub-yolo orworktrees. External directories are capped at ask unless sub-yolo ormanaged allow raises them; user allow does not suffice. An opaque shell subject cannot use a rule-based raiser without subYolo. All child rules still narrow the result, and plan/review hard limits apply afterward. raisedBy is returned only when the result is actually allow.

Raiser detection checks origin.scope=user ormanaged, not the carrier. Project rules therefore cannot lift a ceiling; a claim that every agent-definition carrier is categorically excluded would exceed this helper's implementation. The runtime must assign definition origins correctly and revalidate current authority at execution. The pure childDelegationTools helper offers agent andagent_control only when depth<maxDepth, spawns is nonempty, the parent has agent and the target decision is not deny; it never offers agent_patch. This helper does not prove recursive V2 delegation is currently wired.

The parentDecision argument is caller-supplied. In the active V2 guard, external_directory receives a special parent branch: plan denies; an authorized sub-yolo host allows; otherwise it asks. That branch precedes ordinary parent rule-set evaluation (SUB-019), so the first-pass statement that sub-yolo can never permit external directories is not the current implementation.

### 5. Interfaces

Child derivation context: childRuleSets, agentMode, subYolo, worktree. Output {decision, raisedBy?}, where raisedBy=sub-yolo|user-allow|managed-allow|worktree-isolation. CHILD_MUTATION_CEILING is the authoritative raiser table. Delegation helper output={tools, spawnable}; denial is per target and ask remains eligible for later approval. No persisted grant or new RPC operation occurs in this pure step.

ChildPermissionContext={childRuleSets, agentMode, subYolo, worktree}; ChildPermissionDecision={decision, raisedBy?}. CeilingRaiser=sub-yolo|user-allow|managed-allow|worktree-isolation. ChildSubjectClass=read|edit-write|bash|mutation-adapter|external-directory|delegation. ChildMutationCeilingEntry carries subjectClass, childDefault (decision or parent), maxDecision, raisableBy; parent permission remains a further cap.

### 6. Dependencies

`SUB-006`

No external service is required by this pure contract.

### 7. ice-rust today

rust:crates/ice-harness/src/agent_harness/child.rs:82-87 rejects nested native children and rust:crates/ice-harness/src/agent_harness/child.rs:102-126 requires requested names to be a subset of active parent tools, excluding delegate_task. This preserves a useful tool-name ceiling. It lacks the per-subject child ask defaults, source/carrier semantics, worktree raiser, external-directory ceiling and explicit final decision algebra. Parent hooks are inherited at lines 145-155 but are not a substitute for a frozen child permission snapshot plus live revalidation.

### 8. Port notes

Target a native subagent definition/policy/result module composed by ice-cli, with serializable Rust structs and the workspace JSON types; no provider service is required for these local rules. Retain Rust's native tool subset and nested-child guard, then layer the pure child decision algebra at the shared child tool boundary. Keep an explicit final min(parent, child) operation and table tests across decisions, modes, subjects, raisers. Treat recursive delegation as separately gated runtime work.

### 9. Size and priority

Estimated Rust implementation: **140-230 lines** (estimate, not measured source). **P0** - The port must preserve parent authority while implementing narrower child defaults.

## SUB-008 - Approval broker with foreground prompts, inbox routing and owner cancellation

### 1. Title and status

Approval broker with foreground prompts, inbox routing and owner cancellation. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/subagents/permissions/approval.ts:1-165; ice:packages/coding-agent/test/subagents-v2-permissions.test.ts:331-419

Exported symbols: `DEFAULT_ASK_TIMEOUT_MS` (`ice:packages/coding-agent/src/subagents/permissions/approval.ts:15`), `ApprovalOutcome` (`ice:packages/coding-agent/src/subagents/permissions/approval.ts:34`), `PendingApproval` (`ice:packages/coding-agent/src/subagents/permissions/approval.ts:38`), `ApprovalBrokerOptions` (`ice:packages/coding-agent/src/subagents/permissions/approval.ts:44`), `ApprovalBroker` (`ice:packages/coding-agent/src/subagents/permissions/approval.ts:64`).

### 3. What it does

Resolve child asks through automated checks, a foreground UI prompt or a background inbox, and settle every pending request on answer, timeout, abort or owner disposal.

### 4. How it works

The broker owns pending approval identity and settlement, while callers own the policy decision and applying an offered session rule. Automated checks precede user interaction; absent an appropriate channel, the answer is deny.

Pre-aborted requests and disposed brokers deny immediately. automatedCheck is raced with the external abort signal and may return allow, deny orask; an ordinary checker exception propagates. There is no broker timeout around that preliminary automated check, only external cancellation. Afterward foreground requests use interactive when available; other requests require hasInbox. No channel denies with source no-channel. Duplicate pending IDs throw. A pending waiter records queuedAt/deadline, default 600000 ms timeout and its own AbortController. It is inserted before adding the external abort listener and rechecks aborted state to close that race. onChange observes the current pending array.

Interactive promise rejection becomes a user denial. respond returns false for an unknown/settled ID. deny settles deny; allow-once and allow-session settle allow, with the latter returning only the sessionRule already offered by the request. No arbitrary new rule is accepted here. Settlement deletes the waiter, cancels its timer, removes the external abort listener, aborts the UI controller, resolves and notifies. denyOwner cancels only that owner with source disposed; dispose cancels all and rejects future asks. onChange exceptions are not swallowed by this class, so a host should keep that callback nonthrowing. A broker does not itself persist grants.

### 5. Interfaces

ApprovalRequest={id, ownerSessionId, childId, agent, name?, tool, summary, sessionRule?, foreground}. Responses allow-once|allow-session|deny. ApprovalOutcome is {decision:allow, source:automated|user, sessionRule?} or {decision:deny, source:automated|user|timeout|no-channel|disposed|cancelled}. API request(request, signal?), respond(id, response), pending, denyOwner(ownerSessionId), dispose(). Dependencies include a host UI/inbox adapter; RPC method ownership belongs to the integration entry.

PendingApproval={request, queuedAt, deadline}. ApprovalBrokerOptions has optional interactive(request, signal), hasInbox(), automatedCheck(request, signal?), askTimeoutMs, now(), setTimer(callback, ms)->{cancel()}, onChange(pending). DEFAULT_ASK_TIMEOUT_MS=600000. The aggregate outcome is the discriminated ApprovalOutcome union above, with cancellation source exactly cancelled.

### 6. Dependencies

`SUB-007`

No external service is required by this pure contract.

### 7. ice-rust today

rust:crates/ice-cli/src/child_agents.rs:63-130 has cancellation and plugin-owner revocation but no approval request queue, interactive/inbox split, session grant orapproval deadline. rust:crates/ice-harness/src/agent_harness/child.rs:145-155 copies parent before/after-tool hooks; this lets a host enforce decisions but supplies none of the V2 broker lifecycle.

### 8. Port notes

Target the child-agent host in ice-cli and a V2 subagent extension module over ice-harness; reuse Tokio cancellation/lifetime primitives and add typed host methods for the specified owner-scoped operations. Add a broker with one settlement path, owner-scoped cancellation and an abortable UI/inbox adapter. Apply session grants only after validating the current child contract and offered prefix. Make automated-check deadline policy explicit if strengthening TS behavior; preserve fail-closed no-channel semantics.

### 9. Size and priority

Estimated Rust implementation: **180-290 lines** (estimate, not measured source). **P0** - An ask must neither become silent permission nor leave a child waiting indefinitely after its owner exits.

## SUB-009 - Startup authorization for sub-yolo and reduced delegated shell environment

### 1. Title and status

Startup authorization for sub-yolo and reduced delegated shell environment. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/subagents/types.ts:21-145; ice:packages/coding-agent/src/subagents/extension-support.ts:42-92; ice:packages/coding-agent/src/ice.ts:64-74

Exported symbols: `flagEnabled` (`ice:packages/coding-agent/src/subagents/extension-support.ts:42`), `UnsafeSubagentAuthorization` (`ice:packages/coding-agent/src/subagents/extension-support.ts:47`).

Exported symbols: `UnsafeSubagentStartupArgOptions` (`ice:packages/coding-agent/src/subagents/types.ts:43`).

### 3. What it does

Require an explicit host startup grant before enabling unattended child mutation, and give delegated shells a minimal inherited environment.

### 4. How it works

Startup argument normalization rejects ambiguous or noninteractive sub-yolo configurations before the subagent extension can use the grant. Runtime authorization then binds the grant to the current build mode, trusted project, recorded Bash grant and current active parent tools.

Normalization activates when --sub-yolo or --allow-external is present. Those flags accept bare or =true only; =false and duplicate occurrences throw. Both require a single explicit --ice-mode build; contradictory or duplicate mode flags fail. Both grant modes reject --no-approve/-na by presence. Sub-yolo additionally requires --ice-allow-bash true exactly once and requires both TTY unless explicitly in single --mode rpc. Print mode and non-RPC output modes are incompatible. --allow-external alone may be headless. Bare grant flags normalize to =true. These checks authorize nothing about project trust by themselves.

authorizeUnsafeSubagentHostExecution returns undefined unless startupSubYolo is true. An active request requires capability state, build mode, trusted project, recorded Bash allowance, Bash currently active and an interactive TUI or RPC context with hasUI. It freezes the current parent tool names and allowExternal. Delegated Bash environment separately copies only defined LANG, LC_ALL, LC_CTYPE, LC_MESSAGES, LC_MONETARY, LC_NUMERIC, LC_TIME, PATH, SHELL, TERM, TMPDIR. It does not inherit arbitrary credentials or stale session variables, but this whitelist is not an OS sandbox and the retained shell/path still influence execution.

### 5. Interfaces

Startup flags --sub-yolo[=true],--allow-external[=true],--ice-allow-bash[=true],--ice-mode build; interacting flags --mode rpc,--print/-p,--no-approve/-na. Runtime grant includes parentActiveTools and allowExternal. createDelegatedShellEnvironment(source) returns a fresh ProcessEnv whitelist. The environment is passed to the child Bash adapter; host session metadata behavior is separately TOOLS-002.

UnsafeSubagentStartupArgOptions={stdinIsTTY:boolean, stdoutIsTTY:boolean}. UnsafeSubagentAuthorization={readonly parentActiveTools:readonly string[], readonly allowExternal:boolean}. flagEnabled reads getFlag only when it is a function and accepts exactly boolean true or string 'true'; other truthy values do not enable the flag.

### 6. Dependencies

`SET-003`, `SUB-007`, `TOOLS-002`

No external service is required by this pure contract.

Final dependency classification. Shared contract references: `CLI-002`, `CLI-003`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

rust:crates/ice-cli/src/child_agents.rs:207-337 starts bounded plugin-owned children but accepts no sub-yolo/allow-external startup grant or equivalent frozen capability snapshot. rust:crates/ice-harness/src/agent_harness/child.rs:102-172 narrows tool names and inherits parent behavior, without this per-child environment whitelist. Generic Rust shell environment sanitization is covered by TOOLS-002 and is not the same allowlist.

### 8. Port notes

Target the child-agent host in ice-cli and a V2 subagent extension module over ice-harness; reuse Tokio cancellation/lifetime primitives and add typed host methods for the specified owner-scoped operations. Add startup grant parsing at the CLI/SDK boundary and bind it to trusted current capability state before any child starts. Pass a deliberate environment map with inherited-env disabled where required by Rust's BashPrepare API. Do not reinterpret a project setting as a startup authorization.

### 9. Size and priority

Estimated Rust implementation: **130-220 lines** (estimate, not measured source). **P0** - Unattended child writes and command execution require explicit, durable host intent and current authority.

## SUB-010 - Strict hook configuration, source precedence and revocable handler registration

### 1. Title and status

Strict hook configuration, source precedence and revocable handler registration. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/subagents/settings.ts:1-443; ice:packages/coding-agent/src/subagents/extension-support.ts:79-243; ice:packages/coding-agent/test/suite/subagents-v2-hooks.test.ts:165-191

Exported symbols: `normalizeSubagentHookIds` (`ice:packages/coding-agent/src/subagents/extension-support.ts:94`), `SubagentHookTarget` (`ice:packages/coding-agent/src/subagents/extension-support.ts:122`), `buildSubagentHookRuntime` (`ice:packages/coding-agent/src/subagents/extension-support.ts:135`).

Exported symbols: `IceHookDefinitionInput` (`ice:packages/coding-agent/src/subagents/settings.ts:23`), `IceHooksSettingsInput` (`ice:packages/coding-agent/src/subagents/settings.ts:34`), `IceSettingsInput` (`ice:packages/coding-agent/src/subagents/settings.ts:40`), `ICE_SUBAGENT_SETTINGS_LIMITS` (`ice:packages/coding-agent/src/subagents/settings.ts:45`), `ICE_SUBAGENT_HOOK_EVENTS` (`ice:packages/coding-agent/src/subagents/settings.ts:83`), `IceSubagentHookEvent` (`ice:packages/coding-agent/src/subagents/settings.ts:96`), `ICE_SUBAGENT_DECISION_EVENTS` (`ice:packages/coding-agent/src/subagents/settings.ts:98`), `ICE_SUBAGENT_OBSERVATION_EVENTS` (`ice:packages/coding-agent/src/subagents/settings.ts:104`), `IceHookKind` (`ice:packages/coding-agent/src/subagents/settings.ts:114`), `IceHookDefinition` (`ice:packages/coding-agent/src/subagents/settings.ts:117`), `ParsedIceHooksSettings` (`ice:packages/coding-agent/src/subagents/settings.ts:127`), `parseIceHookDefinition` (`ice:packages/coding-agent/src/subagents/settings.ts:141`), `normalizeIceRoleName` (`ice:packages/coding-agent/src/subagents/settings.ts:238`), `IceHookSource` (`ice:packages/coding-agent/src/subagents/settings.ts:246`), `IceResolvedHook` (`ice:packages/coding-agent/src/subagents/settings.ts:251`), `IceHookResolverInput` (`ice:packages/coding-agent/src/subagents/settings.ts:256`).

### 3. What it does

Select child lifecycle hooks from global and trusted-project configuration while keeping required global hooks mandatory and handler implementations parent-owned.

### 4. How it works

Hook configuration is strict data; executable handlers are registered separately under an owner object. Resolution orders mandatory global hooks, optional global hooks and trusted-project hooks, and definition/call selections can choose only known optional hooks.

Each definition accepts only id, event, roles, kind, timeoutMs, maxOutputBytes, required. ID is trimmed and matches [A-Za-z0-9][A-Za-z0-9_-]{0,127}; roles are trimmed lowercase, deduplicated, nonempty and at most 64 UTF-8 bytes each, with no role-count bound. kind defaults in-process, alternatively command. timeout defaults 1500 ms, range 1..30000; output defaults 8192 bytes, range 1..32768; required defaults true. At most 32 definitions and no duplicate IDs in a scope. Undefined/null hooks disables; enabled defaults true when definitions are nonempty. Global explicit disabled overrides project hooks. Project trust defaults true if a direct resolver caller omits it; runtime callers pass current trust.

An existing hook ID is never replaced by a lower layer's body/source. A required collision with a different event throws; a same-event required declaration can promote an optional global hook to required. Optional roleHookIds and callHookIds form a union; unknown IDs and attempts to select a mandatory hook throw. Empty selection suppresses optional hooks. Required hooks bypass the resolver's role-selection filter, but dispatcher eligibility still applies event and roles. Registration is a WeakMap keyed by the trusted owner, with duplicate ID rejection; unregister only removes the same handler reference. The integration snapshots handlers and rechecks registration identity at dispatch, so removal/replacement revokes a stale handler. The registration object, not arbitrary event data, conveys implementation authority.

### 5. Interfaces

Namespace ice.hooks={enabled?, definitions?, commandPolicy?}. Events: subagent.beforeLaunch, started, beforeTool, afterTool, checkpoint, attention, beforeAccept, completed, failed, cancelled. Decision events are beforeLaunch, beforeTool, beforeAccept; remaining events are observational. Hook definitions refer to IDs, never source-code strings. API parseIceHooksSettings, resolveIceSubagentHooks, registerIceSubagentHook, getIceSubagentHookHandlers; default integration can ask the current UI but no-UI/print/json returns false. V2 current runtime does not dispatch checkpoint orattention merely because these values are accepted.

Hook configuration DTOs distinguish loose IceHookDefinitionInput (optional unknown id/event/roles/kind/timeoutMs/maxOutputBytes/required and unknown keys) from validated IceHookDefinition={id, event, roles:readonly string[]|undefined, kind:in-process|command, timeoutMs, maxOutputBytes, required}. ParsedIceHooksSettings={commandPolicy?, enabled, definitions, diagnostics}; IceHookSource={layer:global|project|role|call, trusted}; IceResolvedHook extends a definition with source/order. IceHookResolverInput carries globalHooks?, projectHooks?, hooks?, projectTrusted?, roleHookIds?, callHookIds?, role?, event?. normalizeIceRoleName is trim().toLowerCase().

SubagentHookTarget={projectTrusted, role, roleHookIds?, callHookIds?, cwd, readOnly, ownerSessionId, runId}. normalizeSubagentHookIds requires an array of at most 32 IDs matching [A-Za-z0-9][A-Za-z0-9_-]{0,127}, rejects malformed input with SubagentError(malformed_result), and freezes the insertion-order deduplicated result. buildSubagentHookRuntime resolves this target against current settings and parent-owned handlers.

### 6. Dependencies

`SET-020`, `SUB-001`

No external service is intrinsically required. Shared host integration is separate from this contract.

### 7. ice-rust today

rust:crates/ice-cli/src/child_agents.rs:207-337 creates plugin-owned in-memory children, and rust:crates/ice-harness/src/agent_harness/child.rs:102-172 snapshots active tools, model, prompt, resources and hooks. Native before/after-tool hooks are inherited, but neither path implements the named ice.hooks configuration, source precedence, required-hook selection or owner-scoped handler registry. A general extension event system is not this lifecycle contract.

### 8. Port notes

Target a native subagent definition/policy/result module composed by ice-cli, with serializable Rust structs and the workspace JSON types; no provider service is required for these local rules. Implement typed hook configuration plus a separate parent-owned implementation registry. Preserve required/optional selection and revocation semantics; pass trust explicitly at every call site instead of depending on a permissive default. Keep checkpoint/attention reserved until actual runtime dispatch is implemented.

### 9. Size and priority

Estimated Rust implementation: **180-300 lines** (estimate, not measured source). **P1** - Definition-selected hooks must not replace mandatory host controls.

## SUB-011 - Bounded hook dispatch, durable intent and fail-closed decision events

### 1. Title and status

Bounded hook dispatch, durable intent and fail-closed decision events. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/subagents/settings.ts:444-916; ice:packages/coding-agent/src/subagents/extension-support.ts:102-243; ice:packages/coding-agent/src/subagents/v2/runtime.ts:353-364; ice:packages/coding-agent/src/subagents/v2/runtime.ts:834-861; ice:packages/coding-agent/src/subagents/v2/runtime.ts:1270-1295; ice:packages/coding-agent/test/suite/subagents-v2-hooks.test.ts:84-163

Exported symbols: `appendSubagentHookJournal` (`ice:packages/coding-agent/src/subagents/extension-support.ts:111`).

Exported symbols: `IceHookOutcome` (`ice:packages/coding-agent/src/subagents/settings.ts:115`), `IceHookContextAddition` (`ice:packages/coding-agent/src/subagents/settings.ts:361`), `IceHookDecisionInput` (`ice:packages/coding-agent/src/subagents/settings.ts:365`), `IceHookDecisionResult` (`ice:packages/coding-agent/src/subagents/settings.ts:377`), `IceHookHandler` (`ice:packages/coding-agent/src/subagents/settings.ts:384`), `ICE_HOOK_JOURNAL_ENTRY_TYPE` (`ice:packages/coding-agent/src/subagents/settings.ts:408`), `IceHookJournalEntry` (`ice:packages/coding-agent/src/subagents/settings.ts:410`), `IceHookDispatchRecord` (`ice:packages/coding-agent/src/subagents/settings.ts:416`), `ICE_HOOK_CONTEXT_ADDITION_LIMITS` (`ice:packages/coding-agent/src/subagents/settings.ts:430`), `IceHookDispatcherOptions` (`ice:packages/coding-agent/src/subagents/settings.ts:456`), `IceHookDispatchResult` (`ice:packages/coding-agent/src/subagents/settings.ts:480`), `IceSubagentHookDispatcher` (`ice:packages/coding-agent/src/subagents/settings.ts:566`), `projectIceHookRecords` (`ice:packages/coding-agent/src/subagents/settings.ts:886`).

### 3. What it does

Run eligible hooks in a stable order with one deadline budget, redacted payloads, explicit deny/ask outcomes and recorded intent before side effects.

### 4. How it works

The dispatcher serializes eligible hooks and keeps denial sticky: later hooks receive skipped-deny records without executing. Required decision hooks fail closed; observational failures become diagnostics rather than taking over the ordinary agent loop.

Payload sanitization redacts credential-like keys(api-key/token/secret/password/authorization/cookie), redacts text, limits depth to 4 and arrays/object entries to 32, handles cycles and unsupported values, then deep-freezes. Strings are first 4096 UTF-8 bytes plus an ellipsis, which may include a replacement character if cut mid-codepoint. A JSON payload above 16384 bytes is replaced by a digest-only truncated object. Fingerprint is the first 16 hex SHA-256 characters. Every dispatch record carries UUIDeventId, ownerSessionId, runId, attempt 1|2, hookId, event, outcome, required, reason?, durationMs and observational.

onIntent is awaited before the handler and consumes the same configured deadline. Remaining handler execution and approval share that original deadline; abort/timeouts abort the hook controller, while a noncooperating underlying promise may finish later without changing the settled result. Missing required handlers fail; optional ones diagnose. Only continue|deny|ask is valid, and observers may only continue. Ask requires requestApproval; missing channel denies. Approved asks retain ask in their record while allowing overall continuation. beforeLaunch may append at most 4 context items per hook, each trimmed/redacted nonempty and<=4096 bytes; other events reject additions. A later denial can leave prior additions in the returned array, so the caller must discard a denied result.

Required decision exceptions, invalid output, intent failure and timeout deny. Optional decision errors and all observational errors continue with redacted diagnostics, including required observers. Append/outcome/observation sink errors are caught; intent failure is authoritative. The integration appends custom journal type ice-subagent-hook-v1 with schemaVersion1 and phase intent|outcome, flushes intents durably and treats outcomes as best effort. Runtime keeps at most 64 observed records in its local projection. beforeLaunch gates before child acceptance; permission guard runs before beforeTool; beforeAccept denial converts a completed turn to failed; started/afterTool/terminal events observe execution.

### 5. Interfaces

Handler input contains hook, event, ownerSessionId, runId, attempt?, role?, payload, signal?. Output {outcome:continue|deny|ask, reason?, contextAdditions?:[{content}]}. Dispatcher output {decision, records, diagnostics, contextAdditions}; reasons are redacted and bounded. APIs dispatchSubagentHookDecision, dispatchSubagentHookObservation, flushSubagentHookObservations. Journal record identifies intent/outcome but is never instructions to replay external effects.

IceHookDecisionInput={hook, event, ownerSessionId, runId, attempt?:1|2, role?, payload:Readonly<Record<string, unknown>>, signal?}; IceHookHandler may return its IceHookDecisionResult synchronously or asynchronously. IceHookContextAddition={content}; maxItemsPerHook=4 and maxItemBytes=4096. IceHookDispatchRecord carries eventId, ownerSessionId, runId, attempt?, hookId, event, outcome, required, reason?, durationMs, observational. IceHookJournalEntry={schemaVersion:1, phase:intent|outcome, record}, custom type ice-subagent-hook-v1. appendSubagentHookJournal projects the record and requests durable append only for intent.

IceHookDispatcherOptions exposes handlers?, requestApproval? accepting one {hookId, event, runId, ownerSessionId, reason, signal?} record, now?, onObservation?, onIntent?, onRecord?. onIntent may be asynchronous and is authoritative before entering a handler; onObservation/onRecord are projection sinks. IceHookDispatchResult uses decision, records, diagnostics, contextAdditions. The per-handler result and record continue to use outcome.

### 6. Dependencies

`SUB-008`, `SUB-010`

No external service is intrinsically required. Shared host integration is separate from this contract.

### 7. ice-rust today

rust:crates/ice-cli/src/child_agents.rs:207-337 creates plugin-owned in-memory children, and rust:crates/ice-harness/src/agent_harness/child.rs:102-172 snapshots active tools, model, prompt, resources and hooks. The native child has ordinary inherited tool hooks, but no three decision-event contract, payload sanitization, serial required-hook chain, hook intent/outcome journal or bounded approval deadline. The native child service's catch_unwind/cancellation is separate process/task safety.

### 8. Port notes

Target the child-agent host in ice-cli and a V2 subagent extension module over ice-harness; reuse Tokio cancellation/lifetime primitives and add typed host methods for the specified owner-scoped operations. Build a deadline-aware dispatcher around the existing session loop and journal adapter. Use cancellation tokens and explicit settled state to ignore late completion; only durable intent may precede a command hook. Preserve failure asymmetry between decision and observational events and never auto-replay intents after restart.

### 9. Size and priority

Estimated Rust implementation: **260-430 lines** (estimate, not measured source). **P0** - A failed required decision hook must stop the action; telemetry failures must not invent authority.

## SUB-012 - Host-pinned command hooks with literal argv and bounded subprocess I/O

### 1. Title and status

Host-pinned command hooks with literal argv and bounded subprocess I/O. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/subagents/hooks/command-hooks.ts:1-208; ice:packages/coding-agent/src/subagents/extension-support.ts:123-219; ice:packages/coding-agent/test/ice-subagent-command-hooks.test.ts:1-94

Exported symbols: `IceCommandHookPolicy` (`ice:packages/coding-agent/src/subagents/hooks/command-hooks.ts:8`).

### 3. What it does

Execute only a globally configured, hash-pinned hook command under current trusted build authority, with a small JSON protocol and bounded process cleanup.

### 4. How it works

Command policy is host configuration rather than an agent-supplied command string. Dispatch rechecks current authority, configured identities and optional user approval before spawning literal argv without a shell.

Undefined policy disables and defaults approval ask. Strict keys are enabled, approval, commands; a present policy requires boolean enabled and a commands object with at most 32 named definitions. Each command requires a files object, even when empty. A command accepts argv, cwd, sha256, files only. argv has 1..32 strings, each no NUL and<=4096 bytes; argv[0] andcwd must be absolute. Executable SHA-256 is 64 lowercase hex; files is at most 16 absolute-path SHA pairs. Before spawn, canonical cwd must remain inside canonical workspace; executable and each pinned file must be regular and realpath(path)===resolve(path), rejecting symlink aliases, then content hashes must match. Hash reads have no explicit file-size bound. This is a pre-spawn identity check with a path-use race, not an atomic executable handle pin.

Execution requires enabled/id/current authorization; approval ask must return true, then abort and authority are checked again. Integration rejects enabled project command policy and uses only the global policy. Current authorization requires a non-read-only target, both current trust checks, build capability, recorded Bash grant, Bash still active and JSON equality with the accepted global policy. In-process handlers with the same ID are replaced by command handlers. Input is JSON {schemaVersion:1, event, ownerSessionId, runId, attempt, role?, payload}, at most 32768 bytes. Spawn is shell:false, detached off Windows, windowsHide, three pipes and environment exactly LANG=C.UTF-8, LC_ALL=C.UTF-8. stdout andstderr are bounded separately by hook.maxOutputBytes; stderr text is discarded. First failure stops stdin, kills the tracked process tree, and after 1000 ms destroys streams/unrefs before rejecting if cleanup has not settled. Deadline, abort, stdin/spawn errors, nonzero exit, oversize and malformed JSON all fail.

Output requires schemaVersion1, outcome continue|deny|ask, optional string reason and contextAdditions, with no extra keys; additions are validated by SUB-011. Exact parser discrepancy: policy approval validation checks String(value.approval??'ask') against ask|allow but stores the original uncoerced value. A malformed host value such as ['ask'] therefore passes that check yet fails the later strict equality approval==='ask', skipping the prompt. This is a source-read conditional behavior of malformed global host configuration, not a reproduced execution. The Rust port should use strict enum deserialization and document this deliberate correction.

### 5. Interfaces

ice.hooks.commandPolicy={enabled:boolean, approval?:ask|allow, commands:Record<id,{argv:string[], cwd:string, sha256:string, files:Record<absolutePath, sha256>}>}; omitting the whole policy disables it, while present policy requires enabled, commands and each files map (empty maps are valid). JSON stdin/stdout protocol version 1. No PATH or other ambient environment inherited. API parseIceCommandHookPolicy, createIceCommandHookHandler. Tests source-cover environment, authority, missing approval, changed script, output flood, deadline and malformed response; they were not run.

IceCommandHookPolicy is the normalized policy type: enabled:boolean, approval:ask|allow and commands:Readonly<Record<id,{argv:readonly string[], cwd:string, sha256:string, files:Readonly<Record<string, string>>}>>. approval is required in this normalized DTO even though the configuration parser may default the corresponding input.

### 6. Dependencies

`SUB-009`, `SUB-010`, `SUB-011`, `TOOLS-017`

No external service is intrinsically required. Shared host integration is separate from this contract.

### 7. ice-rust today

rust:crates/ice-cli/src/child_agents.rs:207-337 creates plugin-owned in-memory children, and rust:crates/ice-harness/src/agent_harness/child.rs:102-172 snapshots active tools, model, prompt, resources and hooks. No native host command-hook policy or JSON subprocess dispatcher appears in the child service/child factory. Rust's ordinary Bash tool and ExtensionAPI exec boundary execute different contracts and must not be treated as hash-pinned lifecycle-hook parity.

### 8. Port notes

Target the child-agent host in ice-cli and a V2 subagent extension module over ice-harness; reuse Tokio cancellation/lifetime primitives and add typed host methods for the specified owner-scoped operations. Implement global-only strict command policy, checked file identity, literal process invocation, bounded separate streams and one cleanup path. Reuse the audited process-tree and cancellation primitives while retaining a short cleanup deadline. Do not reproduce the String-coercion approval bug; cover it with a meaningful negative parser test in future implementation.

### 9. Size and priority

Estimated Rust implementation: **220-360 lines** (estimate, not measured source). **P0** - Command hooks cross the host execution boundary and must retain identity, approval and resource limits.

## SUB-013 - Frozen child runtime contract and hash-checked transcript initialization

### 1. Title and status

Frozen child runtime contract and hash-checked transcript initialization. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/subagents/contract/child-runtime-contract.ts:1-129; ice:packages/coding-agent/src/subagents/contract/serialize.ts:1-82; ice:packages/coding-agent/src/subagents/contract/validate-resume.ts:1-54; ice:packages/coding-agent/src/subagents/registry/child-sessions.ts:1-91; ice:packages/coding-agent/test/subagents-v2-contracts.test.ts:1-178

Exported symbols: `CHILD_ID_PATTERN_SOURCE` (`ice:packages/coding-agent/src/subagents/contract/child-runtime-contract.ts:18`), `CHILD_ID_PATTERN` (`ice:packages/coding-agent/src/subagents/contract/child-runtime-contract.ts:19`), `CHILD_NAME_PATTERN_SOURCE` (`ice:packages/coding-agent/src/subagents/contract/child-runtime-contract.ts:21`), `CHILD_GROUP_ID_PATTERN` (`ice:packages/coding-agent/src/subagents/contract/child-runtime-contract.ts:25`), `isChildId` (`ice:packages/coding-agent/src/subagents/contract/child-runtime-contract.ts:27`), `isChildGroupId` (`ice:packages/coding-agent/src/subagents/contract/child-runtime-contract.ts:31`), `NormalizedPermissionRule` (`ice:packages/coding-agent/src/subagents/contract/child-runtime-contract.ts:39`), `FrozenChildTool` (`ice:packages/coding-agent/src/subagents/contract/child-runtime-contract.ts:45`).

Exported symbols: `createChildId` (`ice:packages/coding-agent/src/subagents/contract/serialize.ts:32`), `createChildGroupId` (`ice:packages/coding-agent/src/subagents/contract/serialize.ts:36`), `ChildRuntimeContractInput` (`ice:packages/coding-agent/src/subagents/contract/serialize.ts:48`).

Exported symbols: `ResumeAuthorityContext` (`ice:packages/coding-agent/src/subagents/contract/validate-resume.ts:13`), `ResumeValidation` (`ice:packages/coding-agent/src/subagents/contract/validate-resume.ts:24`), `validateChildResume` (`ice:packages/coding-agent/src/subagents/contract/validate-resume.ts:26`).

Exported symbols: `CHILD_INIT_ENTRY_TYPE` (`ice:packages/coding-agent/src/subagents/registry/child-sessions.ts:14`), `childSessionDirectory` (`ice:packages/coding-agent/src/subagents/registry/child-sessions.ts:16`), `InterruptedToolCall` (`ice:packages/coding-agent/src/subagents/registry/child-sessions.ts:62`), `buildRecoveryNotice` (`ice:packages/coding-agent/src/subagents/registry/child-sessions.ts:85`).

### 3. What it does

Persist the accepted prompt, model, resources, tools, permissions, scope and isolation once so later turns use the accepted contract rather than silently re-resolving changed definitions.

### 4. How it works

Each child has a durable lifetime ID and a deeply frozen version 2 contract. Canonical SHA-256 ties the child transcript's initialization to the parent registry; it is an integrity reference, not a signature or permission grant.

IDs are ag_ plus 26 lowercase base32 characters; group display handles are agg_ plus 26; optional names are lowercase alphanumeric/hyphen1..40 without underscores. Generation uses 17 random bytes encoded to 26 symbols,130 bits despite the 128-bit comment. Construction verifies ID and depth 0..maxDepth, structured-clones input, redacts system/initial prompts, adds an ISO createdAt and recursively freezes. Canonical serialization sorts object keys, omits undefined object members and preserves array order. parseChildRuntimeContract validates object/version/ID and expected hash, not a complete recursive schema; a Rust parser may deliberately add stricter shape validation.

Contract groups: owner/root/optional parent child/depth; definition source/path/sourceHash/resolvedDefinitionHash/selected hook IDs; actual redacted system and initial prompts, contextFiles flag, resource hashes and selected resource records; provider/model/thinking/temperature/topP/skipped candidates; frozen tools{name, access, adapterId?, fingerprint?}, normalized rules{rule, decision, scope}, roots, targets, MCP servers, spawns, maxDepth, subYolo, agentMode, parentMode, parentPolicyHash, worktree flag; result schema, acceptance andtask contract; isolation baseline/repository/worktree fields. Carrier provenance is dropped by freezeRules and reconstructed as settings on reopen; scope remains. SOURCE_SCOPE assigns package/user/user-settings→user, managed→managed, project sources→project, flag→cli, builtin→builtin, so user/package definitions can supply rule raisers under SUB-007.

Child files live at <parent-session-dir>/subagents/<validated-parent-session-id>/. SessionManager creates a normal child session using the childID and optional parent-session path, appends first custom ice-subagent-v2-init{contract, contractHash}, then flushes durably before launch. Reopen finds the first matching init entry, checks its stored hash equals the registry hash and verifies the serialized contract. findInterruptedToolCalls tracks unresolved calls only from the latest assistant message, clearing on each assistant; recovery text names unknown outcomes and advises inspection, never replay.

### 5. Interfaces

CHILD_RUNTIME_CONTRACT_VERSION=2. First custom type ice-subagent-v2-init. ChildRuntimeContract exact shape at source 53–129. buildChildRuntimeContract, hashChildRuntimeContract, serializeChildRuntimeContract, parseChildRuntimeContract, createChildSession, openChildSession. Baseline child-resume checks owner, tombstone, live tool childSafe/access/adapter identity/fingerprint, sub-yolo continuation, build→plan mutation andmodel availability; runtime adds current trust/resource/policy checks.

NormalizedPermissionRule={rule, decision, scope}; FrozenChildTool={name, adapterId?, fingerprint?, access:read-only|mutation}. ChildRuntimeContractInput omits version and createdAt from ChildRuntimeContract and then permits optional createdAt. ResumeAuthorityContext carries ownerSessionId, tombstoned, parentTools, subYolo, parentMode and modelAvailable; ResumeValidation is {ok:true, activeTools} or {ok:false, reasons}. InterruptedToolCall={id, name}. Child identity predicate exports test their anchored ag_/agg_ base32 patterns; names have their separate 1..40-character grammar.

### 6. Dependencies

`SESSION-029`, `SESSION-031`, `SESSION-034`, `SUB-001`, `SUB-006`, `SUB-007`

Depends on ordinary durable session storage/journal APIs; coordinator should resolve exact SESSION prerequisite IDs. No new model loop.

### 7. ice-rust today

rust:crates/ice-cli/src/child_agents.rs:207-337 creates plugin-owned in-memory children, and rust:crates/ice-harness/src/agent_harness/child.rs:102-172 snapshots active tools, model, prompt, resources and hooks. Rust snapshots parent state into an in-memory child, which correctly avoids sharing the same session ID. It lacks this serialized frozen contract, definition/resource identities, hash-linked first child entry, persistent child directory and restart recovery notice. Native session storage capabilities are a prerequisite integration, not missing merely because the plugin service chooses memory.

### 8. Port notes

Target the child-agent host in ice-cli and a V2 subagent extension module over ice-harness; reuse Tokio cancellation/lifetime primitives and add typed host methods for the specified owner-scoped operations. Introduce a serializable versioned child contract and initialize an ordinary persistent native session durably before registering it as accepted. Keep canonical serialization stable across languages. On resume use accepted fields and current authority checks; never deserialize a pending tool call as permission to execute it.

### 9. Size and priority

Estimated Rust implementation: **230-380 lines** (estimate, not measured source). **P0** - Persistent child identity and restart safety require a stable accepted contract.

## SUB-014 - Explicit child-safe adapters, MCP bridge and capability fingerprints

### 1. Title and status

Explicit child-safe adapters, MCP bridge and capability fingerprints. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/subagents/tools/adapters.ts:1-365; ice:packages/coding-agent/src/subagents/tools/resolve.ts:1-98; ice:packages/coding-agent/src/subagents/types.ts:1-812; ice:packages/coding-agent/src/subagents/v2/runtime-support.ts:148-230; ice:packages/coding-agent/src/ice-mcp-bridge.ts:1-86

Example extension: ice:packages/coding-agent/examples/extensions/ice-delegable-workspace-info.ts:1-37 registers a read-only `workspace_info` parent tool and the same name through `registerIceDelegableTool` (`adapterId`/`origin` `example/workspace-info`, `access: "read-only"`, `childSafe: true`, reporting `child.scopeRoots` relative to `child.cwd`), and unregisters it on `session_shutdown`.

Exported symbols: `listPiMcpAuthorizedTools` (`ice:packages/coding-agent/src/ice-mcp-bridge.ts:40`), `createPiMcpSubagentAdapter` (`ice:packages/coding-agent/src/ice-mcp-bridge.ts:74`).

Exported symbols: `IceCapabilityAccess` (`ice:packages/coding-agent/src/subagents/tools/adapters.ts:8`), `IceChildDispatchContext` (`ice:packages/coding-agent/src/subagents/tools/adapters.ts:9`), `IceCapabilitySnapshot` (`ice:packages/coding-agent/src/subagents/tools/adapters.ts:34`), `IceResolvedDelegableTool` (`ice:packages/coding-agent/src/subagents/tools/adapters.ts:42`), `ICE_CAPABILITY_LIMITS` (`ice:packages/coding-agent/src/subagents/tools/adapters.ts:49`), `ICE_BUILTIN_CHILD_TOOLS` (`ice:packages/coding-agent/src/subagents/tools/adapters.ts:57`), `isIceChildToolName` (`ice:packages/coding-agent/src/subagents/tools/adapters.ts:85`), `isIceDelegableAdapterId` (`ice:packages/coding-agent/src/subagents/tools/adapters.ts:90`), `isIceParentManagementTool` (`ice:packages/coding-agent/src/subagents/tools/adapters.ts:94`), `normalizeIceToolSchema` (`ice:packages/coding-agent/src/subagents/tools/adapters.ts:106`), `snapshotIceCapability` (`ice:packages/coding-agent/src/subagents/tools/adapters.ts:159`), `resolveIceDelegableTools` (`ice:packages/coding-agent/src/subagents/tools/adapters.ts:226`), `boundedIceToolOutput` (`ice:packages/coding-agent/src/subagents/tools/adapters.ts:270`), `awaitIceToolDispatch` (`ice:packages/coding-agent/src/subagents/tools/adapters.ts:304`).

Exported symbols: `ParentToolInfo` (`ice:packages/coding-agent/src/subagents/tools/resolve.ts:18`), `ChildToolResolution` (`ice:packages/coding-agent/src/subagents/tools/resolve.ts:29`).

Exported symbols: `createSelectedMcpToolDefinitions` (`ice:packages/coding-agent/src/subagents/tools/scoped-tools.ts:248`).

Exported symbols: `IceSubagentMcpToolAccess` (`ice:packages/coding-agent/src/subagents/types.ts:195`), `IceSubagentMcpToolAuthorization` (`ice:packages/coding-agent/src/subagents/types.ts:197`), `IceSubagentMcpAdapter` (`ice:packages/coding-agent/src/subagents/types.ts:211`), `parentMcpAdapters` (`ice:packages/coding-agent/src/subagents/types.ts:218`), `registerIceSubagentMcpAdapter` (`ice:packages/coding-agent/src/subagents/types.ts:221`), `subagentMcpToolName` (`ice:packages/coding-agent/src/subagents/types.ts:231`).

Exported symbols: `READ_ONLY_BUILTINS` (`ice:packages/coding-agent/src/subagents/v2/runtime-support.ts:58`), `parentToolInfos` (`ice:packages/coding-agent/src/subagents/v2/runtime-support.ts:148`), `mcpDelegableTools` (`ice:packages/coding-agent/src/subagents/v2/runtime-support.ts:192`).

### 3. What it does

Expose only parent-owned eligible capabilities to a child, with stable identity, live revocation, strict argument validation and bounded redacted results.

### 4. How it works

Adapter registration is an explicit host assertion of independent child-safe execution, not a tool-description heuristic. The runtime intersects active parent tools with known builtins or live adapters and records the selected capability identity in the child contract.

At most 32 registrations per owner; name regex [A-Za-z][A-Za-z0-9_.-]{0,63}, adapter ID [A-Za-z][A-Za-z0-9_.:/-]{0,127}, origin similar with alphanumeric first. Management names are excluded case-insensitively and adapters cannot replace read/grep/find/ls/bash/edit/write. access must be read-only|mutation|unknown andchildSafe exactlytrue. Description<=4096 bytes andredacted. Parameter schema must be object-type, data-only JSON,<=16384 bytes, depth<=16, nodes<=2048, no $ref/$dynamicRef. A normalized JSON clone is recursively frozen. Fingerprint SHA covers adapterID, name, origin, access, childSafe, parameters, description, not function code. Owner WeakMap plus reference-aware unregister and optional isAuthorized provides live revocation.

ParentToolInfos marks unregistered tools unknown/ineligible; MCP capability enumeration requires active parent mcp proxy and current adapter. MCP entries need valid server/tool selector, known access and schema; malformed entries are omitted. Generated names use mcp_ plus a sanitized 40-character prefix and 12 hex SHA selector suffix. Dispatch stays with the same current parent adapter. Child selection excludes management, ineligible/unknown and disallowed globs. Omitted tools or exactly['*'] selects eligible non-MCP capabilities. Other glob lists can match MCP names, including mcp_* or mixed['*','read']; contrary to the exact-name comment, selection is not restricted to exact MCP names. Definition mcp selectors separately glob server andtool. Injected delegationTools bypass the ordinary eligibility/disallowed filter; current V2 factory cannot bind agent/control and therefore rejects such a frozen tool set rather than implementing recursion.

At dispatch check preabort, current registration, args nonarrayobject, JSON<=32768 bytes andschema. Clone/freeze arguments and context{parentSessionId, runId, cwd, scopeRoots, signal}; cancellation races the promise and postchecks abort, warning that external effects need inspection rather than replay. Adapter remains responsible for remote cleanup. Output recursively redacts secret keys, text and cycles, limits 4096 nodes/depth 8, arrays 128/object 64/keys 128 chars, then UTF-8 safe truncates to 24576 bytes with marker. isError propagates only literal result.isError===true; snapshot details accompany text.

### 5. Interfaces

registerIceDelegableTool, getIceDelegableTools, resolveIceDelegableTool(s), resolveChildTools, createIceDelegableToolDefinitions. API IceDelegableTool includes adapterId, name, origin, access, childSafe:true, description, parameters, execute, isAuthorized?. MCP parent registration is owner-scoped with optional identity; selector dispatch signature(server, tool, params, signal). Existing legacy selected-MCP helper accepts an empty permissive object schema when metadata is absent; active V2 bridge requires schema and uses the general adapter path.

IceChildDispatchContext={readonly parentSessionId, runId, cwd, scopeRoots, signal?}. IceCapabilitySnapshot={adapterId, name, origin, access, fingerprint}; IceResolvedDelegableTool adds definition, isCurrent() and optional mcp:{server, tool}. ParentToolInfo carries name, access, childSafe, adapterId?, fingerprint?, mcp?; ChildToolResolution={tools:readonly FrozenChildTool[], diagnostics:readonly string[]}. Access types include unknown before admission, while accepted FrozenChildTool excludes unknown. ICE_BUILTIN_CHILD_TOOLS is read, grep, find, ls, bash, edit, write; the V2 READ_ONLY_BUILTINS set contains only read, grep, find, ls.

IceSubagentMcpToolAuthorization={selector, access, parameters?:TSchema, description?}; IceSubagentMcpAdapter exposes optional getIdentity(), listAuthorizedTools(), dispatch(server, tool, params, signal?). parentMcpAdapters is the exported WeakMap<object, adapter>. Registration rejects a duplicate owner or missing list/dispatch functions and returns cleanup that deletes only the same registered adapter. subagentMcpToolName uses a 40-character alphanumeric/underscore selector label plus the first 12 SHA256 hex characters. createSelectedMcpToolDefinitions is the retained wrapper permitting absent authorization metadata; mcpDelegableTools is the active V2 selector requiring schema and known access.

### 6. Dependencies

`EXT-009`, `EXT-010`, `SUB-013`

No external service is intrinsically required. Shared host integration is separate from this contract.

### 7. ice-rust today

rust:crates/ice-harness/src/agent_harness/child.rs:15-46 wraps parent AgentTool execution under a tracked child scope and cancellation token, while lines 102-126 enforce active names andexclude delegate_task. That is a useful partial capability boundary. It does not classify child-safe access, hash schemas, filter management families, bridge MCP selectors or validate frozen adapter identity on later turns. Native extension runtime action binding is separately EXT-009/010.

### 8. Port notes

Target the child-agent host in ice-cli and a V2 subagent extension module over ice-harness; reuse Tokio cancellation/lifetime primitives and add typed host methods for the specified owner-scoped operations. Retain the native tracked ChildTool wrapper and add explicit capability metadata, versioned fingerprints, owner identity and current-policy checks. Validate schemas andarguments before dispatch; impose bounded output and avoid automatic replay after cancellation. Keep default MCP exclusion and any decision to replace the TS glob edge case explicit.

### 9. Size and priority

Estimated Rust implementation: **300-480 lines** (estimate, not measured source). **P0** - A child must not receive a privileged parent tool merely because its name is active.

## SUB-015 - Scoped file tools and guarded V2 child session construction

### 1. Title and status

Scoped file tools and guarded V2 child session construction. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/subagents/tools/paths.ts:1-60; ice:packages/coding-agent/src/subagents/tools/scoped-tools.ts:1-351; ice:packages/coding-agent/src/subagents/v2/session-factory.ts:1-176

Exported symbols: `isPathWithin` (`ice:packages/coding-agent/src/subagents/tools/paths.ts:5`), `isApprovedSubagentPath` (`ice:packages/coding-agent/src/subagents/tools/paths.ts:13`), `uniquePaths` (`ice:packages/coding-agent/src/subagents/tools/paths.ts:21`), `loadedSkillResourceReadRoots` (`ice:packages/coding-agent/src/subagents/tools/paths.ts:32`), `resolveScopeCandidate` (`ice:packages/coding-agent/src/subagents/tools/paths.ts:50`).

Exported symbols: `SubagentExecutionPolicyBox` (`ice:packages/coding-agent/src/subagents/tools/scoped-tools.ts:322`).

Exported symbols: `SubagentScope` (`ice:packages/coding-agent/src/subagents/types.ts:303`).

Exported symbols: `ChildToolObserver` (`ice:packages/coding-agent/src/subagents/v2/session-factory.ts:38`).

### 3. What it does

Build one ordinary child session from the frozen contract, with selected file scope, hard Bash timeout, a reduced environment and a guard at every dispatch.

### 4. How it works

V2 binds exactly the frozen tool names and wraps each in the current permission guard before recording dispatch observations. It loads selected resources deliberately and disables ambient packages/extensions rather than creating a second reasoning loop.

Path admission resolves a prospective path through the nearest existing canonical ancestor, checks approved task roots or read-only resource roots, then canonicalizes again if it exists and rechecks containment. Writer-mode definitions reject any literal .git path component, including read/grep/find/ls built through that family. V2 always uses this writer family, then filters frozen names. Selected skill parent directories expand only read scope; edit/write receive no resource roots. Tool eligibility runs independently after path checks and compares a lowercase requested name to the fixed eligible set. Under subYolo the scoped-path containment check is bypassed, but .git and any existing symlink component remain rejected. Missing ancestors stop the symlink walk. These are path-based checks with ordinary filesystem races, not descriptor-level filesystem isolation.

Bash uses no background registry, exposeSessionEnvironment:false, a whitelist spawn hook and hard maxTimeoutMs(default 600000). Thus child Bash cannot adopt background shells and stdin:true fails the ordinary tool prerequisites. Registered adapters are filtered to frozen names; every frozen name must have an actual implementation or the factory throws. The guard runs before observer start; observer marks success on resolved execution when signal is not aborted, even if an adapter result carries isError:true. That distinction affects the afterTool hook success projection and must not be described as semantic tool success. V2 file-evidence observations accept only read/grep/edit/write, so resolved adapter errors do not mint file evidence.

Factory revalidates selected resource hashes; reads/redacts selected contexts and prompts; creates SettingsManager with projectTrusted:false and DefaultResourceLoader disabling packages, extensions, ambient skills, prompt templates, themes. noContextFiles is the inverse of frozen contextFiles, so context files are not categorically disabled. Explicit selected skills are additional paths, contexts append to loader base contexts, and selected prompts append to the system prompt. SDK receives model/modelRuntime, thinking, exactcustomtools, resource/settings/session managers and startup session event. Frozen temperature/topP are not passed by this factory; merely storing them does not establish sampler application. The older wrapSubagentToolDefinitions helper uses a mutable per-run policy box for reused legacy sessions; active V2 uses guard closures and assertChildAuthority instead.

### 5. Interfaces

APIs createScopedReadOnlyToolDefinitions, createScopedWriterToolDefinitions, assertNoSymlinkComponents, assertSubagentToolEligible, buildV2ChildToolDefinitions, createV2ChildSession. V2ChildSessionOptions includes contract, sessionManager, cwd, agentDir, model, modelRuntime?, adapters, guard, observeTool?, bashTimeoutMs?. ChildToolGuard(tool, args, signal) rejects to deny. Session startup event reason=startup. No child plugin auto-loading or recursive management implementation.

SubagentScope={roots:string[], targets?:string[]}. ChildToolObserver(toolName, args:Readonly<Record<string, unknown>>) returns either undefined or a completion callback(success:boolean). SubagentExecutionPolicyBox={current:SubagentExecutionPolicy} is the legacy mutable wrapper indirection. uniquePaths deduplicates exact strings in first-occurrence order; it does not canonicalize by itself. loadedSkillResourceReadRoots canonicalizes selected skill files first, skips failed canonicalizations and returns unique parent directories.

### 6. Dependencies

`SUB-009`, `SUB-013`, `SUB-014`, `TOOLS-001`, `TOOLS-002`

No external service is intrinsically required. Shared host integration is separate from this contract.

### 7. ice-rust today

rust:crates/ice-harness/src/agent_harness/child.rs:102-172 creates a normal native AgentHarness with a tool subset and shared resources/provider binding, but does not rebuild scoped file tools or reduce environment per child. It inherits retry/compaction and hooks, whereas TS V2 later disables retries/compaction during bounded turns. Existing native filesystem protections remain valuable but are not per-child scope roots/resource-root exceptions.

### 8. Port notes

Target the child-agent host in ice-cli and a V2 subagent extension module over ice-harness; reuse Tokio cancellation/lifetime primitives and add typed host methods for the specified owner-scoped operations. Create native child-specific wrappers around existing file/Bash/adaptor tools, keeping the single AgentHarness loop. Bind the frozen contract and current guard at the final execution boundary. Use a deliberately restricted resource loader and propagate every intended sampler field; preserve explicit deviation notes for TS stored-but-unused fields and observer isError handling.

### 9. Size and priority

Estimated Rust implementation: **250-410 lines** (estimate, not measured source). **P0** - The permission contract must survive actual dispatch and cannot rely only on prompt instructions.

## SUB-016 - Owner-scoped child registry, generation checks, parking and tombstones

### 1. Title and status

Owner-scoped child registry, generation checks, parking and tombstones. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/subagents/registry/state-machine.ts:1-70; ice:packages/coding-agent/src/subagents/registry/registry.ts:1-323; ice:packages/coding-agent/src/subagents/registry/registry.ts:363-469

Exported symbols: `CHILD_RECORD_ENTRY_TYPE` (`ice:packages/coding-agent/src/subagents/registry/registry.ts:20`), `sessionJournal` (`ice:packages/coding-agent/src/subagents/registry/registry.ts:31`), `ChildUsage` (`ice:packages/coding-agent/src/subagents/registry/registry.ts:44`), `ResidentChild` (`ice:packages/coding-agent/src/subagents/registry/registry.ts:94`), `RegistryEvent` (`ice:packages/coding-agent/src/subagents/registry/registry.ts:98`), `ChildRegistryError` (`ice:packages/coding-agent/src/subagents/registry/registry.ts:102`), `ChildRegistry` (`ice:packages/coding-agent/src/subagents/registry/registry.ts:112`).

Exported symbols: `CHILD_INSTANCE_STATES` (`ice:packages/coding-agent/src/subagents/registry/state-machine.ts:9`), `ChildInstanceState` (`ice:packages/coding-agent/src/subagents/registry/state-machine.ts:19`), `CHILD_TURN_OUTCOMES` (`ice:packages/coding-agent/src/subagents/registry/state-machine.ts:21`), `CHILD_STATE_TRANSITIONS` (`ice:packages/coding-agent/src/subagents/registry/state-machine.ts:41`), `canTransitionChildState` (`ice:packages/coding-agent/src/subagents/registry/state-machine.ts:53`), `recoverChildStateAfterRestart` (`ice:packages/coding-agent/src/subagents/registry/state-machine.ts:64`).

Exported symbols: `ResidentChild` (`ice:packages/coding-agent/src/subagents/v2/runtime-support.ts:90`), `OwnerState` (`ice:packages/coding-agent/src/subagents/v2/runtime-support.ts:132`).

### 3. What it does

Retain children across turns, distinguish instance state from turn outcome, and make lifecycle races observable through generation-checked journal records.

### 4. How it works

The registry is the lifecycle owner; UI/RPC only project records. Owner mismatch and deletion appear as unknown IDs. Every accepted/transitioned record is journaled before becoming the in-memory current value.

States starting, running, idle, parked, stopping, blocked, aborted, deleted have the explicit transition table at state-machine41–51; same-state transitions are permitted and increment generation. A completed turn normally returns idle, not terminal. Outcomes completed, failed, cancelled, interrupted, budget_exhausted, timed_out, abandoned, incomplete are separate. accept refuses closed/duplicate, sets starting, generation 1, turnInFlight true, zero usage, timestamps, appends anddurablyflushes before storing/emitting. transition checks expected generation and legal edge, patches only result/outcome/usage/blocked metadata, clears blockedReasons away fromblocked, appends, optionallyflushes, then stores/emits. Registry listeners are not isolated from exceptions.

list is owner-filtered andcreation-ordered; inherited records from a fork retain their original owner. resident hides a child while parking. park only idle, marks parking/removes handle before state transition and awaited dispose, finally clears parking. revive shares one promise perID; existing idle resident returns immediately; onlyparked ornonresidentblocked may revive. After async construction it rechecks generation/closed anddisposes stale handles. Blocked revival records reasons without starting a turn; success goes blocked→parked→idle andattaches with failure cleanup. delete requires a legal deleted edge, takes awayresident, writes/flushtombstone first, then disposes/cleans; cleanup failure cannot restore the child.

Patch state open|applying|applied|rejected|unchanged|rollback_conflict is separately journaled with a generation bump anddurableflush; applied/rejected/unchanged are terminal andidempotent only to themselves. restore reconstructs latest records/completions/delivered markers from custom entries without deep schema validation. Resident states becomeparked; in-flight ones gaininterrupted andclear lastResult; aborted/deleted/alreadyparked remain unchanged. Recovery append is not itself explicitly flushed here. disposeAll closes, clears residenthandles andawaits allSettled disposal without rewriting durable state.

### 5. Interfaces

Custom types ice-subagent-v2-child, ice-subagent-v2-completion, ice-subagent-v2-delivered. ChildRecord includes IDs/owner/depth/agent/name?/description, contractHash, transcriptPath, state, generation, turnInFlight, lastTurnOutcome?, lastResult?, blockedReasons?, usage, patch metadata, timestamps. Errors unknown_child|stale_generation|illegal_transition|not_parked|busy. ParentJournal={append, flush, entries}. Residents must expose async dispose.

ChildUsage requires inputTokens, outputTokens, cost, turns and permits cacheReadTokens/cacheWriteTokens. RegistryEvent is {type:record, record:ChildRecord}|{type:completion, completion:ChildCompletion}. registry/registry.ts ResidentChild requires only async dispose; v2/runtime-support.ts ResidentChild additionally holds session, contract, grants, foreground and optional pending/recovery note, hooks, controller, budget, observations, detached, running, liveUsage, admitted, liveness. OwnerState carries sessionId, journal, registry, admission, broker, waiters, retireTimers, sending, noticed, closed. These two ResidentChild exports are distinct structural types, not the same declaration.

canTransitionChildState tests only membership in CHILD_STATE_TRANSITIONS, so it returns false for a same-state pair. The registry independently permits a same-state transition before consulting the table, explaining the generation-increment behavior above. recoverChildStateAfterRestart leaves aborted/deleted/parked unchanged with interruptedTurn=false; other states become parked and take interruptedTurn from the supplied turnInFlight flag.

### 6. Dependencies

`SESSION-029`, `SESSION-030`, `SESSION-031`, `SESSION-034`, `SUB-013`

No external service is intrinsically required. Shared host integration is separate from this contract.

### 7. ice-rust today

rust:crates/ice-cli/src/child_agents.rs:44-130 has owner-keyed live jobs, revoked owners andcancel/dispose, so basic ownership and lifecycle cancellation are partial matches. Its result/error polling state has no starting/idle/parked revival, generations, persisted tombstones, per-turn results orappend-only parent journal. InMemorySessionStorage at lines 288-305 does not survive restart.

### 8. Port notes

Target the child-agent host in ice-cli and a V2 subagent extension module over ice-harness; reuse Tokio cancellation/lifetime primitives and add typed host methods for the specified owner-scoped operations. Add a registry layered above native child session creation, with typed state transitions and checked generations. Preserve Rust owner capability checks while using durable acceptance/tombstones. Separate state writes from notifications so future implementation can deliberately isolate nonauthoritative listeners.

### 9. Size and priority

Estimated Rust implementation: **280-450 lines** (estimate, not measured source). **P0** - Without durable lifecycle identity, late callbacks and restart can misattribute child work.

## SUB-017 - Durable child completions, parent delivery confirmation and interruption notices

### 1. Title and status

Durable child completions, parent delivery confirmation and interruption notices. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/subagents/registry/registry.ts:391-456; ice:packages/coding-agent/src/subagents/v2/runtime.ts:1545-1803; ice:packages/coding-agent/src/subagents/v2/runtime-support.ts:29-44

Exported symbols: `CHILD_COMPLETION_ENTRY_TYPE` (`ice:packages/coding-agent/src/subagents/registry/registry.ts:21`), `CHILD_DELIVERED_ENTRY_TYPE` (`ice:packages/coding-agent/src/subagents/registry/registry.ts:22`), `ChildCompletion` (`ice:packages/coding-agent/src/subagents/registry/registry.ts:81`).

Exported symbols: `SUBAGENT_COMPLETED_CUSTOM_TYPE` (`ice:packages/coding-agent/src/subagents/v2/runtime-support.ts:29`), `SUBAGENT_COMPLETION_NOTICE_CUSTOM_TYPE` (`ice:packages/coding-agent/src/subagents/v2/runtime-support.ts:31`), `SUBAGENT_IDLE_ALERT_CUSTOM_TYPE` (`ice:packages/coding-agent/src/subagents/v2/runtime-support.ts:33`), `COMPLETION_SUMMARY_LIMIT` (`ice:packages/coding-agent/src/subagents/v2/runtime-support.ts:35`), `boundedText` (`ice:packages/coding-agent/src/subagents/v2/runtime-support.ts:299`).

### 3. What it does

Deliver child outcomes to the correct parent with bounded untrusted summaries, retaining pending completions through aborts and session shutdown.

### 4. How it works

Completion identity is childID:completed-turn-count and the completion record is durable before delivery. Current runtime waits for transcript confirmation or a finished send before writing the delivered marker; the registry helper's older claim-before-send comment is not the active ordering.

recordCompletion validates owner/child, deduplicates deliveryId, appends/flushtorecordthen emits. claimDelivery writes/flushthe delivered marker once andthen marks memory; it returns undefined forunknown/wrongowner/alreadyclaimed. pendingCompletions filters byowner andunclaimed. Runtime stores outcome, redacted boundedsummary, optional validation summary and time. Each summary targets 16384 UTF-8 bytes including an output-read continuation marker; the offset subtracts any prepended error anduses retained report bytes. Parent messages label child reports untrusted andsuggest continuation/deletion andsibling state.

flush refuses closed orcompactingparent. While streaming it sends one short steer notice perfreshcompletion using an in-memory noticed set; a rejected steer is swallowed anddoes not remove that noticed mark immediately. Full reports stay pending. Whileidle it selects up to 4 pending IDs notalreadyin sending, posts onecustommessage andtracksinflightIDs. message_end confirmation orsuccessful send callsclaimDelivery; rejected send leavespending; finallyclears sending/noticed. triggerTurn defaults true; false recursively posts remainingbatches because no parent turn willlaterflush. Restart restores explicit delivered markers; no scan of preexisting parent completion-message bodies occurs here. Therefore the send/confirm-to-durable-marker crash window is not a transactional exactly-once guarantee, and claims of unconditional at-most-once delivery would be too strong. Confirmed IDs are idempotent.

Idle alerts are separate nondurable custom messages, one perquiet stretch at min(configuredalert, floor(idletimeout/2)); streamingparentgetssteer, idleparentnextTurn. They do not stop the child. Current alert prose says idle-timeout stops it even for silent Bash, although actualBash execution pauses that idle deadline; preserve actual timer behavior in the run entry. Closed-session cancellations become interrupted outcomes with explicit resume advice. Forked children stay with original owners.

### 5. Interfaces

Custom types subagent_completed, subagent_completion_notice, subagent_idle_alert. Completion details {deliveryIds, completions:[{childId, agent, outcome, validation?}], single-child convenience fields?}. APIs flushCompletions({triggerTurn?}), confirmDelivery(message), hasPendingInterruption, inheritedChildren. Completion summary limit 16 KiB, batch 4; output uses agent_control action=output with byte offset. No arbitrary message from a child is trusted as host verification.

ChildCompletion={deliveryId, childId, ownerSessionId, agent, outcome, summary, artifactRef?, validation?:Pick<V2ResultValidation, reportMode|verified|assurance|reason>, at}. Its journal custom type is ice-subagent-v2-completion; the delivered marker is ice-subagent-v2-delivered. Parent-facing completion/notice/idle-alert constants retain the existing three message names. COMPLETION_SUMMARY_LIMIT=16384. boundedText(text, limit) returns {text, truncated}, keeping a UTF8-safe prefix and adding '\n[truncated]' only when needed; it does not redact by itself. For a limit smaller than the marker, the marker can exceed that arbitrary supplied limit; production summary allowances are larger.

### 6. Dependencies

`SESSION-020`, `SESSION-022`, `SESSION-031`, `SUB-016`

Depends on parent session custom-message delivery and durable journaling; coordinator must resolve exact SESSION integration prerequisites rather than adding consumer links.

### 7. ice-rust today

rust:crates/ice-cli/src/child_agents.rs:140-206 exposes owner-checked poll deltas, result, error andcancellation. It has no durable completion inbox, turn deliveryID, automaticparent custom messages, confirmation markers, busy-parent steer orrestart interruption notice. Native queue/message support may supply transport but is not this policy.

### 8. Port notes

Target the child-agent host in ice-cli and a V2 subagent extension module over ice-harness; reuse Tokio cancellation/lifetime primitives and add typed host methods for the specified owner-scoped operations. Add durable completion records and idempotent delivered IDs around ordinary parent message transport. Choose and document crash semantics explicitly; a transactional append of the message plus delivered marker would strengthen current TS ordering. Preserve failed-send retry eligibility and parent-compaction/streaming policy.

### 9. Size and priority

Estimated Rust implementation: **240-400 lines** (estimate, not measured source). **P1** - Background child results must remain findable when parent turns abort or sessions close.

## SUB-018 - Launch preflight, frozen model choice, unique names and staged batch acceptance

### 1. Title and status

Launch preflight, frozen model choice, unique names and staged batch acceptance. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/subagents/v2/runtime.ts:165-307; ice:packages/coding-agent/src/subagents/v2/runtime.ts:500-956; ice:packages/coding-agent/src/subagents/v2/runtime-support.ts:83-159; ice:packages/coding-agent/src/subagents/v2/child-names.ts:1-118; ice:packages/coding-agent/test/suite/subagents-v2-runtime.test.ts:359-391; ice:packages/coding-agent/test/suite/subagents-v2-runtime.test.ts:743-825; ice:packages/coding-agent/test/suite/subagents-v2-runtime.test.ts:1393-1420

Exported symbols: `CHILD_NAME_POOL` (`ice:packages/coding-agent/src/subagents/v2/child-names.ts:4`), `pickChildName` (`ice:packages/coding-agent/src/subagents/v2/child-names.ts:111`).

Exported symbols: `SubagentV2Error` (`ice:packages/coding-agent/src/subagents/v2/runtime-support.ts:117`), `splitModelReference` (`ice:packages/coding-agent/src/subagents/v2/runtime-support.ts:232`), `CHILD_PROMPT_SUFFIX` (`ice:packages/coding-agent/src/subagents/v2/runtime-support.ts:384`).

Exported symbols: `SubagentV2Runtime` (`ice:packages/coding-agent/src/subagents/v2/runtime.ts:143`).

### 3. What it does

Accept a named batch only after resolving every definition, model, scope and authority, then persist child identities and construct sessions before starting provider work.

### 4. How it works

Launch freezes a complete contract for each fresh child, checks the whole batch before acceptance, and retains names for the parent lineage. Provider turns are a separate runTurn action, so accepted does not mean running or completed.

Resolve task.agent, settings.defaultAgent, then the callable catalog; unknown agents report the available list. Delegation permission uses subject agent(name), fallback allow and mode inherit; deny fails preflight, ask is brokered serially before any child acceptance. Task isolation overrides the definition, otherwise none; worktree requires project trust. Resolve current parent tools and adapters, explicit selectors, unavailable-tool diagnostics and the recursion gate. Task.model overrides the definition's ordered model list; resolve the first available configured route or inherited parent through resolveChildModel, retaining skip reasons. The exact lowercase inherit branch uses the captured parent directly. Thinking is task, definition, then medium; parent thinking is not the fallback. Definition source affects rule origin. Scope roots default to cwd, must realpath to existing directories; targets must exist, be regular final non-symlink files, and canonicalize within a root. Ancestor symlinks can canonicalize successfully here; dispatch separately enforces path policy. A non-worktree scope may include external directories, subject to later authority checks.

Preflight pins selected resource hashes, output schema and acceptance, adds fixed child prompt framing plus unavailable-tool diagnostics, generates a child ID and freezes rootSessionId=ownerSessionId with depth 0. mcpServers remains an empty array even when tool records select MCP. The task contract is built from task.description with default fields only. SourceHash and resolvedDefinitionHash are equal here. Temperature/topP are stored but their active factory omission is SUB-015.

Names use the validated explicit name or one of 100 unused pool names; when exhausted, choose an unused name at the lowest suffix 2 or higher. Deleted and fork-inherited names remain reserved. Explicit duplicates within a batch or against prior names fail before acceptance. IDs can be used directly; known inherited IDs/names explain original ownership; missing names list the current names. Catalog cache identity includes cwd, trust, relevant settings and flags, not file mtime; startup/resume/new invalidation is separate.

All delegation approvals and beforeLaunch decision hooks run before persistence. Launch's AbortSignal reaches approval requests but is not forwarded to beforeLaunch hook dispatch. After hooks, recheck owner, parent, settings serialization and child authority. A worktree batch captures one shared dirty parent baseline and rewrites each child's cwd/roots/targets into its separate owned worktree. Stage every durable child transcript, accept every registry record, then construct resident sessions serially. Failure aborts/tombstones accepted children and removes staged worktrees/transcripts, recording cleanup errors where caught. A late factory session after owner shutdown is disposed. These boundaries do not promise a transaction against an OS crash or atomic rollback of all filesystem cleanup.

The exported aggregate is SubagentV2Runtime. Its constructor owns the host, one output artifact store beneath host.agentDir/artifacts/subagent-output, a definition cache and a patch-operation set. Owner state is initialized lazily from the current parent: the same session reuses it; replacing a parent that still has a resident running promise throws busy. Otherwise the previous owner is closed, retirement timers and approval broker are disposed, and resident disposal is initiated before rebinding. New state restores the registry, reconciles terminal worktrees, backfills usage, registers/downgrades saved output artifacts and finalizes owner restore before building admission/broker state. Those subordinate durable contracts remain SUB-016/027/028/031. The outer export line precedes QA's method ranges; it is not an uncovered execution engine.

### 5. Interfaces

V2RuntimeHost supplies getParentSession, getSettings, isProjectTrusted, getParentMode, isSubYolo, getAdapters, optional createChildAgentSession, promptApproval and resource paths. launch(tasks, signal?) returns ChildRecord[]; direct runtime callers have no agent-tool batch schema limit. agent tool limits tasks to 1..8. Child names match the contract's 1..40-character grammar; IDs ag_ plus 26 base32 characters. Fresh launch emits no provider request until runTurn.

SubagentV2Error.code=unknown_agent|preflight|blocked|busy. splitModelReference splits on the first slash, rejects an empty provider or model side, and preserves later slashes in the model string. CHILD_PROMPT_SUFFIX explicitly says the child cannot see the parent conversation, must obey approved scope/tools, and returns a self-contained final report. This string is framing, with enforcement provided by the frozen/live authority contracts.

### 6. Dependencies

`AI-002`, `AI-005`, `AUTH-024`, `SUB-003`, `SUB-004`, `SUB-005`, `SUB-008`, `SUB-011`, `SUB-013`, `SUB-014`, `SUB-015`, `SUB-016`

Requires SUB-003 (Bounded agent discovery, trust filtering and whole-definition precedence); SUB-004 (Built-in agents and incremental model-visible catalog); SUB-005 (Strict scoped V2 settings and monotonic numeric limits); SUB-008 (Approval broker with foreground prompts, inbox routing and owner cancellation); SUB-011 (Bounded hook dispatch, durable intent and fail-closed decision events); SUB-013 (Frozen child runtime contract and hash-checked transcript initialization); SUB-014 (Explicit child-safe adapters, MCP bridge and capability fingerprints); SUB-015 (Scoped file tools and guarded V2 child session construction); SUB-016 (Owner-scoped child registry, generation checks, parking and tombstones); AI-005 (Thinking-level availability, ultra and upward-first clamping); AI-002 (Provider collection with cache-first refresh and generation-checked publication); AUTH-024. Current model/auth catalogs and inherited provider bindings must already be supplied by the host; no new remote discovery service is owned here.

### 7. ice-rust today

rust:crates/ice-cli/src/child_agents.rs:207-418; rust:crates/ice-harness/src/agent_harness/child.rs:74-172 Native start parses one task, role, tools, turns and timeout; it reserves a handle/active permit before child construction and snapshots the parent model. It has no definition/model-candidate preflight, name reservation, batch staging, selected-resource contract or worktree baseline. Its resource reservation should be retained while adding these surfaces.

### 8. Port notes

Target the child-agent host in ice-cli and a V2 subagent extension module over ice-harness; reuse Tokio cancellation/lifetime primitives and add typed host methods for the specified owner-scoped operations. Implement a typed preflight result and a staged launch coordinator above the native child harness. Keep provider work out of construction; validate every fresh task before any acceptance and make cleanup state recoverable. Preserve frozen route/authority identity and name tombstones. Track the mixed fresh/resume tool-level caveat separately in SUB-023.

### 9. Size and priority

Estimated Rust implementation: **360-590 lines** (estimate, not measured source). **P0** - Every later guarantee depends on accepting a precise child contract before side effects begin.

## SUB-019 - Live authority checks, external paths and approval revalidation at dispatch

### 1. Title and status

Live authority checks, external paths and approval revalidation at dispatch. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/subagents/v2/runtime.ts:328-481; ice:packages/coding-agent/src/subagents/v2/runtime.ts:1824-1860; ice:packages/coding-agent/test/suite/subagents-v2-runtime.test.ts:589-628; ice:packages/coding-agent/test/suite/subagents-v2-runtime.test.ts:649-721; ice:packages/coding-agent/test/suite/subagents-v2-runtime.test.ts:827-908

Exported symbols: `ruleSetsFromFrozen` (`ice:packages/coding-agent/src/subagents/v2/runtime-support.ts:266`), `approvalSummary` (`ice:packages/coding-agent/src/subagents/v2/runtime-support.ts:291`).

### 3. What it does

Keep a child's frozen permissions below the parent's current authority, including after an approval dialog or resource change.

### 4. How it works

The live guard checks cancellation, budget and authority before executing each tool. Approval grants are temporary child-instance rules and never replace a parent denial.

Before dispatch, reject an aborted signal, ordinary work after the request budget, wrong owner or invalid frozen authority. For read/edit/write, canonicalize prospective paths; edit/write additionally use the protected mutation-path check. Extract all tool subjects and mirror worktree paths into the parent repository for parent policy, while evaluating child rules against the child path. Parent fallback is allow only for active tools, deny otherwise; bridged MCP requires the active parent mcp proxy plus the selected adapter and evaluates both generated-tool and proxy permissions. External-directory subjects take a dedicated early branch: plan denies, build+subYolo allows, otherwise ask; this branch does not apply general ruleSets. A pure read within another owned open patch worktree can omit that external-directory subject, but does not gain mutation permission.

Compose frozen child rules with the parent decision. A resident session grant can turn current child ask into allow only when parent allows and the shell subject is not opaque. Parent deny is final. Ask requests contain child identity, tool, bounded/redacted summary, foreground flag and an optional offered rule. Summaries replace whitespace runs with a visible newline marker and cap the subject preview at 300 characters. A simple Bash subject offers its command-prefix rule; opaque Bash offers no session grant. Non-Bash can offer the tool rule. The request identity combines childID, time and randomness.

After broker allow, recheck abort, budget, owner/frozen authority and current parent decision before storing an offered session rule or invoking the tool. Permission succeeds before the beforeTool decision hook. Current-authority validation compares owner, frozen tool identity/access/fingerprint, available model, subYolo, parent mode, trust for project resources/worktrees, parent policy hash and every selected resource's canonical identity/content hash. It can block on policy changes even if an individual invocation would otherwise be allowed. The launch-enabled setting is checked at fresh launch; current-authority validation does not globally revoke an existing child merely because enabled becomes false.

### 5. Interfaces

ChildToolGuard(tool, args, signal) is the dispatch boundary. ApprovalAnswer=allow-once|allow-session|deny; grants remain resident-memory state. Parent policy derives from current active tool names, settings ruleSets, mode and adapter identity; frozen child policy remains in ChildRuntimeContract. Guard errors become ordinary tool execution failures in the child loop.

ruleSetsFromFrozen groups rules by first-seen scope, reconstructs origin={scope, carrier:settings}, initializes allow/ask/deny arrays and retains rule order within each decision bucket. approvalSummary redacts credential-looking text, replaces newline runs with a visible return marker, collapses whitespace, trims and truncates the subject portion to 300 JavaScript code units, then prefixes toolName. The caller supplies the trusted tool identity.

### 6. Dependencies

`SUB-006`, `SUB-007`, `SUB-008`, `SUB-013`, `SUB-014`, `SUB-015`, `SUB-021`

Requires SUB-006 (Permission rule grammar, tool subjects and conservative shell decomposition); SUB-007 (Child permission ceilings and source-sensitive authority raisers); SUB-008 (Approval broker with foreground prompts, inbox routing and owner cancellation); SUB-013 (Frozen child runtime contract and hash-checked transcript initialization); SUB-014 (Explicit child-safe adapters, MCP bridge and capability fingerprints); SUB-015 (Scoped file tools and guarded V2 child session construction); SUB-021 (Model-request budgets, one tool-free wrap-up and temporary retry suppression). Uses host policy, resource identity and approval interfaces plus cancellation; no additional service.

### 7. ice-rust today

rust:crates/ice-cli/src/child_agents.rs:207-418; rust:crates/ice-harness/src/agent_harness/child.rs:74-172 Native ChildTool wraps execution in a tracked child cancellation scope and snapshots a parent tool subset. It does not re-evaluate scoped definition rules, selected-resource hashes, transient session grants or parent policy changes at each invocation. Native hook inheritance alone does not establish these exact checks.

### 8. Port notes

Target the child-agent host in ice-cli and a V2 subagent extension module over ice-harness; reuse Tokio cancellation/lifetime primitives and add typed host methods for the specified owner-scoped operations. Attach a single current-authority validator to both model-request admission and final tool dispatch, with the frozen contract as input. Recheck after awaits that can outlive authorization. Keep the special external-directory branch explicit and document any deliberate correction to its current rule-bypass semantics. Retain native cancellation tracking under the policy layer.

### 9. Size and priority

Estimated Rust implementation: **270-440 lines** (estimate, not measured source). **P0** - Cached approval or tool visibility cannot safely stand in for current authority.

## SUB-020 - Admission, liveness deadlines and abandonment of uncooperative child turns

### 1. Title and status

Admission, liveness deadlines and abandonment of uncooperative child turns. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/subagents/registry/admission.ts:1-70; ice:packages/coding-agent/src/subagents/v2/runtime.ts:958-1309; ice:packages/coding-agent/src/subagents/v2/runtime.ts:1360-1542; ice:packages/coding-agent/test/suite/subagents-v2-runtime.test.ts:284-357; ice:packages/coding-agent/test/suite/subagents-v2-runtime.test.ts:393-505; ice:packages/coding-agent/test/suite/subagents-v2-runtime.test.ts:630-647; ice:packages/coding-agent/test/suite/regressions/execution-reliability-3.1-subagent-liveness.test.ts:16-60

Exported symbols: `SUBAGENT_CONCURRENCY_LIMITS` (`ice:packages/coding-agent/src/subagents/registry/admission.ts:9`), `SubagentConcurrencyAdmission` (`ice:packages/coding-agent/src/subagents/registry/admission.ts:21`).

Exported symbols: `MAX_RESIDENT_CHILDREN` (`ice:packages/coding-agent/src/subagents/v2/runtime.ts:122`), `RETIRE_GRACE_MS` (`ice:packages/coding-agent/src/subagents/v2/runtime.ts:124`), `CHILD_ABORT_GRACE_MS` (`ice:packages/coding-agent/src/subagents/v2/runtime.ts:126`).

### 3. What it does

Bound admitted child work, distinguish queued/running/idle/abandoned outcomes, and report liveness without claiming that an abort undid tool effects.

### 4. How it works

A parent owner has a configurable admission pool. Accepted turn intent is durable before admission, while timers start only after a permit is acquired. An uncooperative aborted turn is fenced from later result publication.

runTurn rejects unknown, nonresident, already-running or patch-closed children despite its older 'never throws' comment. It validates narrowed acceptance and authority, cancels retirement, publishes turnInFlight=true durably, then invokes accepted(). This acknowledgement can precede a concurrency slot. acquire loops until a permit is available; waiting promises are queued and a release wakes one. A wake is not a reserved permit, so a newcomer can win the next acquisition; do not claim strict FIFO admission. Caller abort removes a queued waiter. No queue timeout runs before admission, and the pool/maxConcurrent is captured when the owner is constructed rather than rebuilt on every settings change.

After admission, start wall and idle deadlines, default 1800000 ms and 600000 ms, plus an idle alert at min(configured alert, floor(idleTimeout/2)); configured 0 disables the alert. Model/tool activity rearms idle and alert timers. Only in-flight Bash increments the counter that pauses the idle deadline; a silent long non-Bash tool can still reach it. Bash still has its tool ceiling and the child wall deadline, and still receives idle alerts. Track startedAt, lastActivityAt, current tool/arguments for status; send at most one alert per quiet stretch. The alert is informational, delivered as steer when the parent is busy and nextTurn when idle.

Caller cancellation is attached after the internal abort-grace listener, then pre-aborted signals are applied. Expiry records the deadline reason and aborts. After 10000 ms without settlement, mark finishClaimed/fenced, clear timers, release the admission permit once, clear foreground and record abandoned with explicit unknown last-tool effects. The underlying promise may still be running; resident.running remains until its real settlement, and late settlement cannot record a second outcome. This releases TypeScript concurrency before every underlying effect is known to stop. Native cleanup-permit retention is a stronger resource bound worth preserving.

On admitted start, recheck authority, persist running, install the bounded-turn wrapper and emit started observation. Queued cancellation reports cancelled and may delete a never-run child without a patch. Provider stop, error, length and empty-text outcomes are classified separately; cancellation or deadline overrides completion. Finalization claims the outcome before asynchronous acceptance hooks, preventing grace from double-publishing while bookkeeping completes. Timers/listeners/budget state and permits are cleaned up. Finish records usage and validated output durably, and closes unchanged worktrees. Idle children retire after 30000 ms if not displayed, foreground, running or in a patch operation; excess idle residents are parked oldest-first beyond 8, and that separate excess-parking path does not consult displayed status.

### 5. Interfaces

States live separately from ChildTurnOutcome completed|failed|cancelled|timed_out|budget_exhausted|incomplete|abandoned|interrupted. runTurn options foreground, background, acceptance, accepted, signal. Overview capacity={running, queued, max}; queued means a turn promise exists but admitted=false. Liveness status includes elapsed/quiet age and last tool or waiting-on-model. Retirement is a memory policy, not a deletion of persistent identity.

MAX_RESIDENT_CHILDREN=8; RETIRE_GRACE_MS=30000; CHILD_ABORT_GRACE_MS=10000. SUBAGENT_CONCURRENCY_LIMITS={min:1, bundledDefault:8} is helper metadata; resolved V2 maxConcurrent defaults are specified separately in SUB-005. SubagentConcurrencyAdmission requires a safe integer >=1 and imposes no additional upper bound.

### 6. Dependencies

`SUB-005`, `SUB-013`, `SUB-016`, `SUB-017`, `SUB-021`

Requires SUB-005 (Strict scoped V2 settings and monotonic numeric limits); SUB-013 (Frozen child runtime contract and hash-checked transcript initialization); SUB-016 (Owner-scoped child registry, generation checks, parking and tombstones); SUB-017 (Durable child completions, parent delivery confirmation and interruption notices); SUB-021 (Model-request budgets, one tool-free wrap-up and temporary retry suppression). Requires monotonic timers, cancellation and retained task handles; preserve Tokio scope accounting.

### 7. ice-rust today

rust:crates/ice-cli/src/child_agents.rs:207-418; rust:crates/ice-harness/src/agent_harness/child.rs:74-172 Native service has fixed 16 active and retained limits, no admission queue, wall timeout default 300 s,2 s abort grace and child execution_scope.wait_idle before releasing an active permit. It already fans cancellation from job, invocation, parent child-scope and parent run. It lacks V2 idle alerts/deadlines, queued intent, eight-resident retirement and the full outcome vocabulary. Preserve its scope-idle lifetime protection rather than weakening it to TypeScript's abandoned-permit release.

### 8. Port notes

Target the child-agent host in ice-cli and a V2 subagent extension module over ice-harness; reuse Tokio cancellation/lifetime primitives and add typed host methods for the specified owner-scoped operations. Integrate a parent-owned queue and explicit liveness state with the native execution scope. Keep accepted intent distinct from admitted work. Use checked timers and one terminal-claim guard; preserve unknown-effects outcomes. Decide explicitly whether abandoned jobs consume native capacity until scope idle, and expose the difference rather than hiding the stronger native bound.

### 9. Size and priority

Estimated Rust implementation: **340-540 lines** (estimate, not measured source). **P0** - A hung provider or tool must not leave the parent waiting without a truthful bounded outcome.

## SUB-021 - Model-request budgets, one tool-free wrap-up and temporary retry suppression

### 1. Title and status

Model-request budgets, one tool-free wrap-up and temporary retry suppression. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/subagents/v2/turn-budget.ts:1-222; ice:packages/coding-agent/src/subagents/runtime/budgets.ts:1-68; ice:packages/coding-agent/src/subagents/v2/runtime.ts:1165-1309; ice:packages/coding-agent/test/suite/subagents-v2-runtime.test.ts:113-148

Exported symbols: `ChildBudgetLimits` (`ice:packages/coding-agent/src/subagents/runtime/budgets.ts:4`), `BudgetState` (`ice:packages/coding-agent/src/subagents/runtime/budgets.ts:10`), `ChildBudgetTracker` (`ice:packages/coding-agent/src/subagents/runtime/budgets.ts:14`), `budgetExhaustedMessage` (`ice:packages/coding-agent/src/subagents/runtime/budgets.ts:66`).

### 3. What it does

Limit ordinary child model work and allow one explicitly bounded final report after the request ceiling.

### 4. How it works

ChildTurnBudget wraps the existing session stream/stop/tool hooks rather than adding a second loop. Active V2 gives each run 100 model requests, then at most one tool-free wrap-up.

Count a model request before calling the stream function; authorize current child state and abort status first. This counts wrapper stream invocations, not necessarily internal HTTP retries inside a provider. Record completed assistant usage once per message using a WeakSet; the generic tracker counts input+cacheWrite+output tokens, excluding cacheRead, and checks turns before tokens before elapsed time. Active V2 passes maxTurns=100; generic token/time budget helpers are retained APIs, not public V2 settings. Reaching the last request with a normal stop can remain completed; cutting off a request that still needs tools marks budget_exhausted. Ordinary tools assert the budget before effects.

During the bounded run, disable automatic session retry and compaction and save prior settings/hooks/stream state. Restore them on disposal. A budget interruption permits a single wrap-up: clear queued messages, remove active tools, cap output to 1024 tokens, set a 30000 ms abort timer, and ask for a report using only existing context. Restore the active tool list afterward, clear the queue again, and clear the timer when complete; queued contents are discarded, not restored. The timeout calls session.abort; a noncooperative prompt can still outlive that local await, with outer turn cancellation/abandonment providing the broader terminal path. Never describe the wrap-up as proof that every tool or provider stopped.

The generic resolveTurnBudget helper accepts only positive integer limits and can narrow, never raise or remove, an existing ceiling. claimWrapUp is one-shot. Active runtime observes assistant and compaction usage separately for billing, while disabling automatic compaction during these runs. Final outcome classification does not turn a wrap-up report into independently verified success.

The current wrapper sets only maxTokens at ice:packages/coding-agent/src/subagents/v2/turn-budget.ts:95 and :109, not hardMaxOutputTokens. The 1024 value is therefore the ordinary allowance presented to the provider adapter, not proof of an immutable final wire limit: reasoning-budget adjustment or a payload callback can widen an ordinary maxTokens value. Suppression at lines 130-144 changes session retry/compaction enabled flags; it does not set provider maxRetries and therefore does not disable or restore internal HTTP retries. AI-006 documents the distinct hard-limit contract that a strengthened port should use.

### 5. Interfaces

Constants MAX_CHILD_MODEL_REQUESTS_PER_RUN=100; CHILD_WRAP_UP_LIMITS maxTokens=1024, timeoutMs=30000. ChildTurnBudget interrupted(), assertOrdinaryWork(), wrapUp(signal), dispose(). Generic TurnBudget maxTurns?, maxTokens?, timeoutMs?; counters remain per-run. No automatic retry-safe fallback execution is wired through retained legacy routing helpers.

ChildBudgetLimits={maxTurns?, maxTokens?, timeoutMs?}; BudgetState={status:ok}|{status:exhausted, limit:keyof ChildBudgetLimits, used, max}. ChildBudgetTracker copies the limits and captures its supplied/default monotonic clock on construction. budgetExhaustedMessage renders the exhausted limit and used/max with a final-report/no-tools instruction; the text itself does not enforce a dispatch boundary.

### 6. Dependencies

`AGENT-007`, `AGENT-012`, `AGENT-019`, `AI-006`, `SESSION-018`, `SET-012`, `SUB-013`, `SUB-015`

AGENT-007/012/019 provide stream options, tool-boundary and turn-stop hooks; SESSION-018 and SET-012 cover session retry and compaction overrides. AI-006 is the prerequisite for deliberately strengthening the current ordinary output allowance into a non-widening final wire limit. Internal provider HTTP retries remain independent.

### 7. ice-rust today

rust:crates/ice-cli/src/child_agents.rs:207-418; rust:crates/ice-harness/src/agent_harness/child.rs:74-172 Native create_child applies maxTurns default 8, range 1..32 and inherits retry/compaction/stream options. The child service reports turnLimited when observed turns reach the cap, but no separate tool-free wrap-up or temporary retry/compaction suppression was found. Its maxTurns must not be equated to 100 wrapper model requests.

### 8. Port notes

Target the child-agent host in ice-cli and a V2 subagent extension module over ice-harness; reuse Tokio cancellation/lifetime primitives and add typed host methods for the specified owner-scoped operations. Add a scoped budget guard around native stream and tool dispatch, saving/restoring session settings on every exit. Retain the native turn ceiling as a separately named protection or deliberately reconcile the API. Test normal completion exactly at the ceiling, unfinished tool use, pre-abort, wrap-up timer cleanup and restoration without running tests during this audit. Clear queued child work according to the source contract instead of claiming to restore discarded messages. If enforcing a strict wrap-up output ceiling, populate hardMaxOutputTokens and preserve it after provider adjustments/hooks; identify that as an intentional correction to the current TS wrapper.

### 9. Size and priority

Estimated Rust implementation: **170-280 lines** (estimate, not measured source). **P1** - Budget exhaustion must stop ordinary work while allowing a bounded, clearly classified report.

## SUB-022 - Resume, steering, stop and deletion of persistent child instances

### 1. Title and status

Resume, steering, stop and deletion of persistent child instances. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/subagents/v2/runtime.ts:1862-2043; ice:packages/coding-agent/src/subagents/registry/registry.ts:191-337; ice:packages/coding-agent/test/suite/subagents-v2-runtime.test.ts:1069-1094; ice:packages/coding-agent/test/suite/subagents-v2-runtime.test.ts:1314-1391; ice:packages/coding-agent/test/suite/subagents-v2-results.test.ts:311-395

### 3. What it does

Continue a known child under its accepted contract, send messages with distinct semantics, stop a turn, or durably delete the instance.

### 4. How it works

Revival reopens the child's durable transcript and frozen contract, checks ownership/hash/current authority and worktree existence/ownership, resolves the frozen model, and only then attaches a resident session. Blocked reasons remain visible without manufacturing a live handle.

continueChild refuses a closed patch or patch operation, revives parked/blocked children and runs a new turn. It may add task-local acceptance to the frozen baseline but cannot replace model, scope, tools, isolation or output schema. Resume-added criteria do not mutate the persistent baseline: a later turn without those additions requires only the original criteria. Interrupted revival adds a recovery notice listing unmatched tool calls and their unknown effects; it does not replay them. One child cannot run two turns concurrently.

send(message) while running uses session.steer; send(followup) queues session.followUp. For an idle child, message revives if needed and appends a pending note without starting provider work; followup starts a new background turn. interrupt aborts, awaits resident.running, then starts the follow-up. That await is unbounded for an uncooperative underlying promise, unlike stop. stop aborts and waits at most 10500 ms for a running turn; the run's 10000 ms grace can report abandoned. detachToBackground only changes resident foreground/background bookkeeping; it does not itself detach a caller AbortSignal already installed by runTurn.

Delete refuses applying/rollback_conflict or another patch operation, stops the child, writes the registry tombstone before disposal/cleanup, removes an owned worktree and releases output artifacts. Disposal can still await session.abort, so delete is not globally bounded merely because stop is. Shutdown closes the owner, cancels broker requests/timers and all resident controllers, waits for running turns up to 10500 ms and disposeAll up to 10000 ms, then clears owner state. Stale factory/revival handles are disposed rather than reattached. Closing-session cancellation becomes interrupted for restart delivery.

outputOf returns a clone of the saved latest result. Interrupted state without a valid saved result returns undefined instead of recycling a previous verified answer. Legacy transcript fallback takes the latest nonempty assistant text, redacts and caps it at 48 KiB, and labels it unverified_plain with cumulative usage. A result snapshot can remain available after an unchanged worktree has been cleaned, even though its previously checked evidence paths no longer exist.

### 5. Interfaces

continueChild(id, prompt,{foreground, acceptance?, signal?, accepted?}); send(id, text, kind=followup), kind=message|followup|interrupt; stop(id); delete(id); shutdown(); detachToBackground(ids); outputOf(id). IDs and reserved names resolve within the owner lineage. Patch decisions are independent of turn outcome and can permanently close continuation.

### 6. Dependencies

`SUB-013`, `SUB-016`, `SUB-019`, `SUB-020`, `SUB-027`, `SUB-028`

Requires SUB-013 (Frozen child runtime contract and hash-checked transcript initialization); SUB-016 (Owner-scoped child registry, generation checks, parking and tombstones); SUB-019 (Live authority checks, external paths and approval revalidation at dispatch); SUB-020 (Admission, liveness deadlines and abandonment of uncooperative child turns); SUB-027 (Per-turn observed file evidence, acceptance criteria and honest assurance); SUB-028 (Owner-scoped output artifacts, UTF-8 range reads and durable retention). Requires the persistent child session and owned artifact/worktree services behind those entries; no remote service.

### 7. ice-rust today

rust:crates/ice-cli/src/child_agents.rs:140-206; rust:crates/ice-cli/src/child_agents.rs:288-418; rust:crates/ice-harness/src/agent_harness/child.rs:74-172 Native child_cancel cancels a handle; child_dispose removes it; owner teardown revokes and removes its jobs. There is no persistent child revival, followup/steer control, task-local acceptance or saved-result restart fallback. Native cleanup retains the execution permit until its scope is idle and should remain stronger than TypeScript's early abandonment release.

### 8. Port notes

Target the child-agent host in ice-cli and a V2 subagent extension module over ice-harness; reuse Tokio cancellation/lifetime primitives and add typed host methods for the specified owner-scoped operations. Build lifecycle methods over the durable registry and existing native harness queue/cancellation APIs. Keep message, followup and interrupt distinct. Preserve precise timeout promises and expose unknown cleanup effects. Add an explicit cancellation-aware interrupt wait if deliberately strengthening TypeScript behavior, and record that as a port decision.

### 9. Size and priority

Estimated Rust implementation: **230-390 lines** (estimate, not measured source). **P1** - Resuming and stopping persistent children must preserve both authority and result identity.

## SUB-023 - Agent tool batches, background acknowledgement and foreground detachment

### 1. Title and status

Agent tool batches, background acknowledgement and foreground detachment. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/subagents/api/agent.ts:1-86; ice:packages/coding-agent/src/subagents/v2/extension.ts:458-767; ice:packages/coding-agent/test/subagents-v2-extension.test.ts:77-159; ice:packages/coding-agent/test/subagents-v2-extension.test.ts:263-507; ice:packages/coding-agent/test/suite/regressions/execution-reliability-0.3-foreground-agent-auto-detach.test.ts:16-72

Exported symbols: `AGENT_TOOL_LIMITS` (`ice:packages/coding-agent/src/subagents/api/agent.ts:13`), `AGENT_CONTEXT_ITEM_KINDS` (`ice:packages/coding-agent/src/subagents/api/agent.ts:25`), `AgentTaskSchema` (`ice:packages/coding-agent/src/subagents/api/agent.ts:27`), `AgentToolParametersSchema` (`ice:packages/coding-agent/src/subagents/api/agent.ts:78`), `AgentToolParameters` (`ice:packages/coding-agent/src/subagents/api/agent.ts:86`).

Exported symbols: `SUBAGENT_PRESENTATION_LIMIT_BYTES` (`ice:packages/coding-agent/src/subagents/types.ts:301`).

### 3. What it does

Expose reusable child execution as the agent model tool, with ordered batch results, independent background handles and bounded foreground waiting.

### 4. How it works

The tool validates the strict task schema and resume exclusions, normalizes every context packet, resolves references, launches fresh children, then dispatches fresh and resumed turns in original task order. Background acknowledgement means durable turn acceptance, not a completed result or guaranteed admission.

agent requires 1..8 tasks; each requires description 1..120 characters and prompt with maxLength65536. Optional fields include agent, name, resume, model, thinking, isolation, scope, outputSchema, acceptance and context. On resume, reject outputSchema, agent, name, model, thinking, isolation and scope before launching fresh siblings. Context/acceptance remain permitted. All context packets are normalized before effects. Existing name/ID references are resolved before fresh launch, but full resumed contract/runtime availability is checked only when continueChild runs afterward. A mixed fresh/resume batch can therefore persist fresh children before a resumed child fails authority, busy or patch checks; do not advertise all-or-nothing behavior across those cases.

With background=true, create one acceptance promise per run; runTurn's accepted callback follows durable turn intent and may precede admission. Return accepted IDs and immediate rejections after those acknowledgements, while child result promises continue with handlers. Caller signal is forwarded to launch approvals, but not to background runTurn. Rejected background resumes are reported as rejected failed results and are never counted as started. A later background failure travels through the completion mechanism, not a claimed aggregate result.

Foreground dispatch passes caller cancellation, waits concurrently, and maintains an abort listener that stops accepted children. A 250 ms poll releases the wait when hasPendingSteering becomes true; queued follow-up alone does not release it. Ctrl+B releases registered blocking waits. foregroundDetachMs defaults 60000,0 disables the timer. Either release path marks still-running children background, returns ordered IDs with detached=true and explains how to inspect them; already completed results remain accessible. Removing the tool-level abort listener does not remove the runTurn signal listener itself. A 500 ms progress timer emits only changed summaries. All listeners/timers/wait registration are cleaned up.

When every foreground turn settles, format per-child reports in input order and, for a multi-child batch, prepend deterministic host batch synthesis. Result details omit large text but retain IDs, outcomes, usage, validation and synthesis. Model-visible text is redacted then head-truncated at 2000 lines and 65536-256 bytes with a presentation marker. Error catches return normal text results without setting isError, so consumers must inspect the stated outcome/details instead of assuming a transport error flag.

### 5. Interfaces

Tool agent({tasks:AgentTaskInput[], background?:boolean}). Scope roots 1..32 and targets<=32 strings; acceptance<=16 strings maxLength1024; context has<=16 typed items. Foreground returns structured result details; background details {ids, rejected}; detached details include ordered ids and detached=true. No legacy delegate/review_batch tool is registered by V2.

### 6. Dependencies

`SUB-018`, `SUB-020`, `SUB-022`, `SUB-025`, `SUB-027`, `SUB-029`

Requires SUB-018 (Launch preflight, frozen model choice, unique names and staged batch acceptance); SUB-020 (Admission, liveness deadlines and abandonment of uncooperative child turns); SUB-022 (Resume, steering, stop and deletion of persistent child instances); SUB-025 (Selected child resources and explicit untrusted context packets); SUB-027 (Per-turn observed file evidence, acceptance criteria and honest assurance); SUB-029 (Deterministic assurance helpers and bounded batch synthesis). Uses extension tool registration and structured argument validation; no remote service.

### 7. ice-rust today

rust:crates/ice-cli/src/child_agents.rs:140-206; rust:crates/ice-cli/src/child_agents.rs:288-418; rust:crates/ice-harness/src/agent_harness/child.rs:74-172 The native plugin boundary exposes child_run/poll/cancel/dispose behavior but no agent task-array schema, ordered foreground aggregation, background acceptance callback, automatic foreground detach or steering-aware wait. A plugin tool may supply a caller-facing wrapper, but no equivalent implementation was found in the audited repository.

### 8. Port notes

Target the child-agent host in ice-cli and a V2 subagent extension module over ice-harness; reuse Tokio cancellation/lifetime primitives and add typed host methods for the specified owner-scoped operations. Implement one V2 agent tool over native persistent child services. Keep schema validation, whole-fresh-batch preflight, acceptance acknowledgement and result settlement as separate phases. Preserve all immediate rejections and document mixed-resume partial acceptance. Use the existing parent steering/queue signal and current cancellation scope for wait release.

### 9. Size and priority

Estimated Rust implementation: **240-390 lines** (estimate, not measured source). **P1** - The model-facing delegation contract determines whether users receive usable child handles and truthful batch results.

## SUB-024 - Agent control queries, range output and repeated-wait suppression

### 1. Title and status

Agent control queries, range output and repeated-wait suppression. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/subagents/api/agent-control.ts:1-55; ice:packages/coding-agent/src/subagents/v2/extension.ts:768-950; ice:packages/coding-agent/src/subagents/v2/runtime.ts:2044-2357; ice:packages/coding-agent/test/subagents-v2-extension.test.ts:161-260; ice:packages/coding-agent/test/subagents-v2-extension.test.ts:509-558; ice:packages/coding-agent/test/suite/regressions/execution-reliability-3.2-repeat-wait-guard.test.ts:15-49

Exported symbols: `AGENT_CONTROL_ACTIONS` (`ice:packages/coding-agent/src/subagents/api/agent-control.ts:11`), `AgentControlAction` (`ice:packages/coding-agent/src/subagents/api/agent-control.ts:22`), `AGENT_SEND_DELIVERIES` (`ice:packages/coding-agent/src/subagents/api/agent-control.ts:24`), `AgentSendDelivery` (`ice:packages/coding-agent/src/subagents/api/agent-control.ts:25`), `AGENT_CONTROL_LIMITS` (`ice:packages/coding-agent/src/subagents/api/agent-control.ts:27`), `AgentControlParametersSchema` (`ice:packages/coding-agent/src/subagents/api/agent-control.ts:37`), `AgentControlParameters` (`ice:packages/coding-agent/src/subagents/api/agent-control.ts:55`).

### 3. What it does

Let the parent list, inspect, wait for, read, message, resume, stop and delete owned children without relaunching them.

### 4. How it works

agent_control resolves ids in preference to id and checks supplied references even for actions that later use the whole list. Queries project registry/runtime state; lifecycle operations call the same owner methods as the UI.

Actions are list, status, wait, output, send, resume, stop, delete, agents. ids permits 1..8 references. list reports children and descriptions; status includes current state, turn count, model/patch information and liveness. Tokens in status include input, output, cacheRead, cacheWrite, whereas overview live token totals use input+output. agents lists the definition catalog. send requires a message and acts on the first selected child, defaulting to followup; resume, stop and delete process selected IDs serially.

wait uses registry subscriptions and a predicate requiring turnInFlight=false and state neither running nor starting; an idle-looking queued continuation is not settled. Default timeout 60000 ms, schema range 1..1800000. The signature includes selected IDs, state, generation, turnInFlight. If the same signature timed out less than 30000 ms ago, return immediately with the unchanged-wait explanation rather than block again. A state change or elapsed cooldown allows another wait. Ctrl+B and a 250 ms hasPendingSteering poll end only the parent wait; they do not stop children. External abort also ends the wait, but this wrapper installs its abort listener without first checking an already-aborted signal, unlike runTurn. It can classify an unlabelled abort as timedOut and update the repeated-wait record. Do not claim universal pre-abort immediacy for this tool.

output without offset/length formats each selected child's saved result with assurance. A range request requires exactly one child and delegates to runtime.readOutput. The public schema caps offset at 4194304 and length at 49152; the internal store accepts reads to 65536. Output reports totalBytes, bytesRead, nextOffset, eof, sha256, truncated and content type. Inline and artifact ranges share UTF-8 boundary checks, so callers can append chunks without corruption. Deleted/cross-owner/unknown children fail owner lookup. All control text uses the common 64 KiB/2000-line presentation cap; caught errors are normal text, not isError=true.

### 5. Interfaces

agent_control action vocabulary above; id?, ids?, message?, delivery=message|followup|interrupt, timeoutMs?, offset?, length?. Registry wait returns {timedOut, records}. Host control bridge separately exposes stop|resume|delete|send. Range output retains artifactId only for artifact-backed data, not a model-supplied filesystem path.

### 6. Dependencies

`SUB-004`, `SUB-016`, `SUB-020`, `SUB-022`, `SUB-028`

Requires SUB-004 (Built-in agents and incremental model-visible catalog); SUB-016 (Owner-scoped child registry, generation checks, parking and tombstones); SUB-020 (Admission, liveness deadlines and abandonment of uncooperative child turns); SUB-022 (Resume, steering, stop and deletion of persistent child instances); SUB-028 (Owner-scoped output artifacts, UTF-8 range reads and durable retention). Uses owner-scoped runtime and artifact reads; no remote service.

### 7. ice-rust today

rust:crates/ice-cli/src/child_agents.rs:140-206; rust:crates/ice-cli/src/child_agents.rs:288-418; rust:crates/ice-harness/src/agent_harness/child.rs:74-172 Native poll destructively drains queued text toward64KiB and withholds final result/error until deltas drain. It is not a persistent named-child query, nondestructive byte-range result read, subscription wait, repeated-wait guard or resume/send API. Preserve native output limits while adding explicit V2 control semantics.

### 8. Port notes

Target the child-agent host in ice-cli and a V2 subagent extension module over ice-harness; reuse Tokio cancellation/lifetime primitives and add typed host methods for the specified owner-scoped operations. Add a strict control dispatcher over the persistent registry, separate output read storage and parent wait cancellation. Keep query/lifecycle semantics and timeout classification explicit. Port unchanged-signature suppression and UTF-8 pagination; decide and document correction of the already-aborted wrapper case.

### 9. Size and priority

Estimated Rust implementation: **180-290 lines** (estimate, not measured source). **P1** - Control and output tools make background and parked children usable across parent turns.

## SUB-025 - Selected child resources and explicit untrusted context packets

### 1. Title and status

Selected child resources and explicit untrusted context packets. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/subagents/runtime/resources.ts:1-206; ice:packages/coding-agent/src/subagents/runtime/helpers.ts:1-389; ice:packages/coding-agent/src/subagents/runtime/prompt.ts:64-105; ice:packages/coding-agent/src/subagents/v2/runtime.ts:592-633; ice:packages/coding-agent/test/suite/subagents-v2-runtime.test.ts:1460-1502

Exported symbols: `MAX_SELECTED_RESOURCES_BYTES` (`ice:packages/coding-agent/src/subagents/runtime/resources.ts:42`), `readValidatedResource` (`ice:packages/coding-agent/src/subagents/runtime/resources.ts:179`).

Exported symbols: `SubagentResourceKind` (`ice:packages/coding-agent/src/subagents/types.ts:277`), `SubagentResourceSelection` (`ice:packages/coding-agent/src/subagents/types.ts:279`), `SubagentResourceProvenance` (`ice:packages/coding-agent/src/subagents/types.ts:285`), `ResolvedSubagentResources` (`ice:packages/coding-agent/src/subagents/types.ts:294`), `SUBAGENT_CONTEXT_PACKET_LIMITS` (`ice:packages/coding-agent/src/subagents/types.ts:314`), `SubagentContextItemKind` (`ice:packages/coding-agent/src/subagents/types.ts:320`), `SubagentContextItemInput` (`ice:packages/coding-agent/src/subagents/types.ts:322`), `SubagentContextItem` (`ice:packages/coding-agent/src/subagents/types.ts:328`), `SubagentContextPacketInput` (`ice:packages/coding-agent/src/subagents/types.ts:332`), `SubagentContextPacket` (`ice:packages/coding-agent/src/subagents/types.ts:336`).

### 3. What it does

Load only explicitly selected child skills, prompts and context, pin their identity, and bound parent-provided context without granting additional authority.

### 4. How it works

Resource resolution records source and canonical path plus content hash during preflight, and revalidates those fields before session creation/revival and current-authority dispatch. Parent-supplied context is separately framed as untrusted data.

User roots are agentDir/skills, prompts, context; project roots are the nearest .ice/skills and .ice/prompts, with project context rooted at cwd. Named resources try user first and project second. Skills select name/SKILL.md, prompts name.md, context the given name. Explicit absolute, ./, slash or backslash paths resolve from cwd and must be lexically inside an allowed root. Candidate names must be nonempty and no more than 4096 code units/UTF-8 bytes. Each kind permits 16 selections, canonical duplicates are removed within that kind, each file is<=128 KiB, and aggregate selected bytes are<=512 KiB. Stat and bounded read enforce individual size. Project content requires trust. Selected skill parent directories extend read-only resource roots.

Resolution canonicalizes both root and candidate and requires containment. In the explicit-path loop, an existing explicit project file can be examined against an existing user root first and throw as outside that root before trying project; this is a reachable source quirk, not a claim that explicit project paths always work. Revalidation checks aggregate metadata, canonical source identity and exact bounded content SHA; changed resources reject rather than silently reload. Selected prompts remove frontmatter and trim/redact their body. Selected contexts are redacted into child resources; trust never comes from file prose.

normalizeSubagentContextPacket accepts a packet with items plus optional legacyContext. legacyContext becomes a parent-context item after trim/redaction. Combined item count<=16; IDs trim and match [A-Za-z0-9][A-Za-z0-9_-]{0,63}, kinds are parent_note|verified_fact|evidence_ref|artifact_ref, and content is nonempty after trim/redaction. Duplicate IDs reject rather than deduplicate. Per-item content<=32768 UTF-8 bytes and total<=262144. Freeze packet/items and append their id, kind, content under explicit untrusted framing to the task prompt. There is no ambient parent conversation fork in active V2 launch merely because the older prompt-cache helper exists.

### 5. Interfaces

resolveSubagentResources({skills?, prompts?, context?},{cwd, agentDir, projectTrusted}); revalidateSubagentResources; loadSelectedPromptContent; normalizeSubagentContextPacket; withContextPacket. Stored resource fields kind, name, source, sourcePath, canonicalPath, sourceHash. Task context schema supplies typed items; direct legacyContext helper remains a retained API.

SubagentResourceKind=skill|prompt|context. SubagentResourceSelection has optional skills/prompts/context string arrays; ResolvedSubagentResources has all three provenance arrays. Each provenance entry carries kind, name, source, sourcePath, canonicalPath, sourceHash. SubagentContextItemInput={id, kind, content}; normalized SubagentContextItem adds readonly bytes. Packet input is {items}; normalized packet is {readonly items, readonly totalBytes}. The exported limits are 16 items,32768 bytes per item and 262144 total; MAX_SELECTED_RESOURCES_BYTES=524288.

### 6. Dependencies

`SET-003`, `SET-019`, `SET-023`, `SET-025`, `SET-028`, `SET-030`, `SUB-001`, `SUB-013`

Requires SUB-001 (Strict native Markdown and JSON agent definitions with source identity); SUB-013 (Frozen child runtime contract and hash-checked transcript initialization); SET-003 (Project trust resolution before loading executable resources); SET-019 (Scoped resource arrays, package filters and skill-command preference); SET-023 (Resource provenance, canonical deduplication and extension-added paths); SET-025 (Project instructions, ancestor ordering and linked-worktree shadow suppression); SET-028 (Coding prompt-template discovery, metadata and source-aware collision handling); SET-030 (Skill discovery, ignore rules and canonical-file/name deduplication). Requires canonical filesystem paths and SHA-256 content identity from the resource loader.

### 7. ice-rust today

rust:crates/ice-cli/src/child_agents.rs:140-206; rust:crates/ice-cli/src/child_agents.rs:288-418; rust:crates/ice-harness/src/agent_harness/child.rs:74-172 Native create_child clones parent resources/system prompt and has a distinct tool-free side_question context helper. It does not resolve per-definition selected files, trust-gated source hashes, resource-root exceptions or bounded typed context packets. Preserve native side_question as a separate capability, not evidence of V2 selected-resource parity.

### 8. Port notes

Target the child-agent host in ice-cli and a V2 subagent extension module over ice-harness; reuse Tokio cancellation/lifetime primitives and add typed host methods for the specified owner-scoped operations. Add deterministic resource selection and content revalidation before child construction and effects, with explicit source attribution. Use bounded descriptor reads/canonical containment and clarify the current explicit-project-path fallback defect. Implement context packet validation as a pure helper separate from authority.

### 9. Size and priority

Estimated Rust implementation: **160-280 lines** (estimate, not measured source). **P1** - Children need reproducible context without silently inheriting unrelated resources or trusting task text.

## SUB-026 - Bounded report envelope and local output-schema validation

### 1. Title and status

Bounded report envelope and local output-schema validation. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/subagents/results/report.ts:1-541; ice:packages/coding-agent/src/subagents/results/v2-result.ts:1-91; ice:packages/coding-agent/test/suite/subagents-v2-results.test.ts:168-224; ice:packages/coding-agent/test/suite/subagents-v2-results.test.ts:397-436; ice:packages/coding-agent/test/suite/subagents-v2-results.test.ts:520-559

Exported symbols: `SubagentReportParseOutcome` (`ice:packages/coding-agent/src/subagents/results/report.ts:279`), `SubagentParsedReport` (`ice:packages/coding-agent/src/subagents/results/report.ts:284`), `CANDIDATE_EVIDENCE_PATH_LIMIT` (`ice:packages/coding-agent/src/subagents/results/report.ts:292`), `extractCandidateEvidencePaths` (`ice:packages/coding-agent/src/subagents/results/report.ts:300`), `parseSubagentReportOutcome` (`ice:packages/coding-agent/src/subagents/results/report.ts:359`).

Exported symbols: `V2ChildTurnResult` (`ice:packages/coding-agent/src/subagents/results/v2-result.ts:33`), `v2ReportMode` (`ice:packages/coding-agent/src/subagents/results/v2-result.ts:66`).

Exported symbols: `SUBAGENT_REPORT_LIMITS` (`ice:packages/coding-agent/src/subagents/types.ts:343`), `SUBAGENT_REPORT_PARSER_LIMITS` (`ice:packages/coding-agent/src/subagents/types.ts:349`), `SubagentReportMode` (`ice:packages/coding-agent/src/subagents/types.ts:358`), `SubagentRequirementClaimStatus` (`ice:packages/coding-agent/src/subagents/types.ts:369`), `SubagentRequirementClaim` (`ice:packages/coding-agent/src/subagents/types.ts:371`), `SubagentOutputSchemaType` (`ice:packages/coding-agent/src/subagents/types.ts:434`), `SubagentOutputSchemaNode` (`ice:packages/coding-agent/src/subagents/types.ts:437`), `SUBAGENT_OUTPUT_SCHEMA_LIMITS` (`ice:packages/coding-agent/src/subagents/types.ts:445`), `EvidenceRef` (`ice:packages/coding-agent/src/subagents/types.ts:683`), `ReviewFinding` (`ice:packages/coding-agent/src/subagents/types.ts:687`), `SubagentEvidence` (`ice:packages/coding-agent/src/subagents/types.ts:694`).

Exported symbols: `ChildTurnResult` (`ice:packages/coding-agent/src/subagents/v2/runtime-support.ts:115`).

### 3. What it does

Parse a bounded final report and requested structured payload without interpreting child claims as host verification.

### 4. How it works

V2 selects structured_report when the task has outputSchema, any acceptance criteria, or agent name exactly review; otherwise plain_final_turn stays explicitly unverified. Shape parsing precedes observed-evidence acceptance.

The local schema subset permits only type, properties, required, items, additionalProperties. Reject unknown keys, refs, unions and remote schemas. Serialized schema<=8192 bytes, maximum depth 5, nodes 64, object properties 16, required 16, property-name 64 bytes matching the declared letter/alphanumeric/underscore/hyphen pattern. Root must be object and every object requires additionalProperties=false; absent properties/required become empty, required names must exist and are deduplicated. Arrays require items. Primitive types string, number, integer, boolean, null are supported; irrelevant but otherwise allowed schema keys on a primitive are silently dropped rather than rejected. Values enforce finite number, safe integer, strings<=8192 bytes, arrays<=64 items and payload JSON<=16384 bytes; object extras reject. The returned normalized payload is cloned and top-level frozen, not recursively frozen by this parser.

The final report is bounded at 24 KiB before parsing. Empty/oversize input gets malformed/truncated status. Parse direct JSON, otherwise try balanced objects beginning at at most 8 opening braces with quoted-string/escape handling. The first parseable object is accepted for shape checking; a wrong-shaped first object does not cause a search for a later valid report. Require nonempty trimmed summary and evidence.paths with 1..64 nonempty paths<=4096 bytes. Extra envelope/evidence keys are not rejected. Summary is bounded by total report size rather than a separate small field limit. Requested payload is mandatory and schema-checked; unrequested payload is ignored.

Optional findings permit<=32 entries with severity low|medium|high, category nonempty<=256 bytes, claim nonempty<=8192 bytes and 1..16 evidence paths. Optional requirements permit<=16 claims, nonempty id<=64 bytes and one of satisfied|partial|blocked|failed|not_attempted; note is retained only when a string<=1024 bytes; evidencePaths<=64 can be empty at parse time. Duplicate requirement IDs are first-claim-wins. Parser path discovery for malformed output is diagnostic only and never verifies evidence. Structured provider output truncated by a length stop is not accepted, even if its prefix is valid JSON. No repair-model request or verification command is launched here.

### 5. Interfaces

normalizeSubagentOutputSchema, validateSubagentPayload, parseSubagentReport, normalizePlainFinalAnswer. Report shape {summary, evidence:{paths}, payload?, findings?, requirements?}. Parse and final-validation outcomes are separate; raw final text remains retained on validation failure. V2ResultValidation assurance is decided by SUB-027, not envelope fields such as testsPassed.

SubagentReportParseOutcome={kind:valid, report:SubagentParsedReport}|{kind:malformed, diagnostic}|{kind:truncated, diagnostic}. SubagentParsedReport carries summary, paths, findings, requirementClaims, payload?. SubagentRequirementClaim={id, status, note?, evidencePaths?}, with the exact five statuses in the corrected parser description. CANDIDATE_EVIDENCE_PATH_LIMIT=16 and extracted candidate paths are diagnostic only. EvidenceRef={path}; ReviewFinding={severity:low|medium|high, category, claim, evidence:EvidenceRef[]}; SubagentEvidence={paths:string[]}.

V2ChildTurnResult={id, agent, outcome, text, truncated, output?:V2OutputCapture, usage:{inputTokens, outputTokens, cacheReadTokens?, cacheWriteTokens?, cost}, validation?:V2ResultValidation, error?}. v2/runtime-support.ts ChildTurnResult is exactly a type alias to V2ChildTurnResult. v2ReportMode returns structured_report for an output schema, nonempty acceptance list or agent name exactly review, otherwise plain_final_turn. SubagentOutputSchemaNode has readonly type, properties?, required?, items?, additionalProperties?:false and the exact restricted primitive/object/array vocabulary already described.

### 6. Dependencies

`SUB-013`

Requires SUB-013 (Frozen child runtime contract and hash-checked transcript initialization). Requires JSON parsing and the specified bounded local schema subset; no remote schema resolver.

### 7. ice-rust today

rust:crates/ice-cli/src/child_agents.rs:140-206; rust:crates/ice-cli/src/child_agents.rs:288-418; rust:crates/ice-harness/src/agent_harness/child.rs:74-172 Native completion serializes the child final text plus turn count/turnLimited. It has no local output-schema subset, report envelope parser, required acceptance claims or separate shape-verification result. Native output byte bounds are not schema/evidence validation.

### 8. Port notes

Target a native subagent definition/policy/result module composed by ice-cli, with serializable Rust structs and the workspace JSON types; no provider service is required for these local rules. Implement a pure bounded schema/report parser with a deliberately small supported subset, explicit UTF-8 sizes and exact first-object/first-claim behavior. Keep raw output and diagnostics on failure. Do not promote child-supplied assurance keys or execute declared commands.

### 9. Size and priority

Estimated Rust implementation: **210-350 lines** (estimate, not measured source). **P1** - Typed child results need predictable parsing and honest failure behavior before they enter parent decisions.

## SUB-027 - Per-turn observed file evidence, acceptance criteria and honest assurance

### 1. Title and status

Per-turn observed file evidence, acceptance criteria and honest assurance. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/subagents/results/v2-result.ts:1-261; ice:packages/coding-agent/src/subagents/v2/runtime.ts:1360-1485; ice:packages/coding-agent/test/suite/subagents-v2-results.test.ts:226-395; ice:packages/coding-agent/test/suite/subagents-v2-results.test.ts:492-600

Exported symbols: `TurnResultContext` (`ice:packages/coding-agent/src/subagents/v2/runtime-support.ts:125`), `backfillSubagentUsage` (`ice:packages/coding-agent/src/subagents/v2/runtime-support.ts:319`).

### 3. What it does

Verify that a structured child report cites files successfully accessed in the current turn and still unchanged, while leaving semantic correctness explicitly unverified.

### 4. How it works

Each run creates a fresh V2ResultObservations object. Previous turns, siblings, model prose and directory listings cannot satisfy the current turn's evidence requirements.

Observe only read, grep, edit, write with string args.path. At dispatch after approval, resolve the prospective canonical path and require an approved root. read/grep need a preexisting regular file; grep over a directory does not create evidence for every match. Capture at most 1024 distinct paths. After successful execution and no abort, require the same canonical identity and a regular final non-symlink file. For read/grep, dev/ino, size, mtime, ctime must be unchanged before/after; for edit/write, record the post-operation stat. Final citations resolve first against child cwd, then roots, must canonicalize within scope, and must still match the recorded stat. These metadata checks are evidence-of-access/stability, not cryptographic content or semantic proof. Bash, ls, find and custom adapters do not add V2 file evidence.

Normalize frozen acceptance plus task-local additions: each input list<=16, each raw string<=1024 UTF-8 bytes and nonempty after trim; redact/trim, deduplicate and require<=16 unique criteria. IDs criterion-1 etc follow normalized position. A claim verifies only when status=satisfied, evidencePaths is nonempty and every path passes the current observation check. Every required baseline/additional criterion must verify. All top-level report paths and finding evidence also must pass. A valid structured report with zero criteria can be verified when its other evidence checks pass. Payload is exposed only on a verified result.

Plain final turns always return verified=false and assurance=unverified_plain even on outcome completed. Noncompleted turns are not promoted. Structured completed work that fails validation becomes failed with raw answer and diagnostics retained. Successful V2 structured validation returns evidence_checked, with semanticVerified=false, commandsObserved=false, commandsPassed=false, reviewerVerified=false. Calling the agent review or writing testsPassed:true does not change that level. Validation occurs before unchanged-worktree cleanup; saved verified metadata remains historical evidence after those files are removed.

Finish preserves the latest assistant response observed in events even if the session's message array is replaced. It accounts assistant/compaction usage, appends nonzero subagent_usage before the durable turn-settled record, and backfills missing older usage once from persisted totals. Saved lastResult replaces prior output only after durable transition; capture failure is explicit artifact_unavailable. Interrupted restore does not serve an earlier verified snapshot as the new turn's result.

### 5. Interfaces

V2ResultObservations before(tool, args), after(token, success), verifyPath; normalizeV2Acceptance; validateV2Result; renderV2ResultInstructions. Validation contains reportMode, verified, assurance, reason, paths, requirements, findings?, payload?. Requirements retain required=true, claim, verified and semanticVerified=false. No independent reviewer or test runner is part of this validation.

TurnResultContext={acceptance:readonly string[], observations:V2ResultObservations, final?:AssistantMessage, usage:ChildTurnResult['usage']}. backfillSubagentUsage sums finite existing subagent_usage values by child ID, compares every owned registry record including deleted records, clamps token deltas at zero and retains a cost delta only when greater than 1e-9. It appends {childId, agent, usage, backfill:true} only for a nonzero delta and flushes once only if anything was appended. Repeated calls therefore reconcile durable totals rather than relying on a separate one-shot flag.

### 6. Dependencies

`SUB-013`, `SUB-015`, `SUB-026`

Requires SUB-013 (Frozen child runtime contract and hash-checked transcript initialization); SUB-015 (Scoped file tools and guarded V2 child session construction); SUB-026 (Bounded report envelope and local output-schema validation). Requires observations emitted by guarded tool dispatch, not a new verifier process.

### 7. ice-rust today

rust:crates/ice-cli/src/child_agents.rs:140-206; rust:crates/ice-cli/src/child_agents.rs:288-418; rust:crates/ice-harness/src/agent_harness/child.rs:74-172 The native child service observes text deltas/turn endings and captures final output. It has no per-turn file-access observation map, stat freshness checks, criterion claims or semantic-assurance distinctions. Generic native tool results do not imply this acceptance contract.

### 8. Port notes

Target the native V2 child result module composed by ice-cli over ice-harness, with host-owned per-turn observation snapshots and serde_json report validation. Place observation capture after authorization and around actual file dispatch, then validate all cited paths before publishing a successful structured result. Preserve plain/unverified outcomes and raw failed reports. Keep billing durability and latest-result replacement ordered, with no reuse of previous-turn evidence.

### 9. Size and priority

Estimated Rust implementation: **210-340 lines** (estimate, not measured source). **P1** - A child saying it checked a file is insufficient evidence for accepting its report.

## SUB-028 - Owner-scoped output artifacts, UTF-8 range reads and durable retention

### 1. Title and status

Owner-scoped output artifacts, UTF-8 range reads and durable retention. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/subagents/results/output-artifacts.ts:1-770; ice:packages/coding-agent/src/subagents/v2/runtime.ts:1360-1485; ice:packages/coding-agent/src/subagents/v2/runtime.ts:2044-2142; ice:packages/coding-agent/test/ice-subagent-output-artifacts.test.ts:44-267; ice:packages/coding-agent/test/suite/subagents-v2-results.test.ts:438-490

Exported symbols: `SUBAGENT_OUTPUT_ARTIFACT_LIMITS` (`ice:packages/coding-agent/src/subagents/results/output-artifacts.ts:22`), `SubagentOutputContentType` (`ice:packages/coding-agent/src/subagents/results/output-artifacts.ts:31`), `SubagentOutputCaptureStatus` (`ice:packages/coding-agent/src/subagents/results/output-artifacts.ts:33`), `SubagentOutputArtifactRef` (`ice:packages/coding-agent/src/subagents/results/output-artifacts.ts:39`), `SubagentOutput` (`ice:packages/coding-agent/src/subagents/results/output-artifacts.ts:49`), `SubagentOutputRead` (`ice:packages/coding-agent/src/subagents/results/output-artifacts.ts:58`), `SubagentOutputArtifactErrorCode` (`ice:packages/coding-agent/src/subagents/results/output-artifacts.ts:71`), `SubagentOutputArtifactError` (`ice:packages/coding-agent/src/subagents/results/output-artifacts.ts:78`), `SubagentOutputArtifactStoreLimits` (`ice:packages/coding-agent/src/subagents/results/output-artifacts.ts:88`), `SubagentOutputArtifactStoreOptions` (`ice:packages/coding-agent/src/subagents/results/output-artifacts.ts:97`), `CaptureSubagentOutputInput` (`ice:packages/coding-agent/src/subagents/results/output-artifacts.ts:102`), `ReadSubagentOutputInput` (`ice:packages/coding-agent/src/subagents/results/output-artifacts.ts:109`), `RetainSubagentOutputInput` (`ice:packages/coding-agent/src/subagents/results/output-artifacts.ts:116`), `ReleaseSubagentOutputInput` (`ice:packages/coding-agent/src/subagents/results/output-artifacts.ts:122`), `isSubagentOutputArtifactRef` (`ice:packages/coding-agent/src/subagents/results/output-artifacts.ts:208`), `isSubagentOutput` (`ice:packages/coding-agent/src/subagents/results/output-artifacts.ts:238`).

Exported symbols: `V2OutputCapture` (`ice:packages/coding-agent/src/subagents/results/v2-result.ts:25`).

Exported symbols: `RESULT_TEXT_LIMIT` (`ice:packages/coding-agent/src/subagents/v2/runtime-support.ts:34`), `durableOutputReferenceId` (`ice:packages/coding-agent/src/subagents/v2/runtime-support.ts:37`), `outputArtifactUnavailable` (`ice:packages/coding-agent/src/subagents/v2/runtime-support.ts:41`).

### 3. What it does

Keep large child answers readable through opaque owned artifact references, with explicit truncation/quota failures and restart retention.

### 4. How it works

The store redacts captured output, keeps a bounded inline prefix, and writes larger output behind an owner-scoped UUID. Registry retention determines when the artifact remains durable or can be reclaimed.

Default inline cap 48 KiB, per-artifact 4 MiB, read default 16 KiB/max 64 KiB, owner quota 16 MiB and store-global quota 128 MiB. Quotas are in-memory accounting for this store instance, not a filesystem-wide synchronized quota. originalBytes counts raw input; stored redaction can increase bytes. Inline text is a UTF-8 prefix without a marker. Small output returns inline_complete; large output returns artifact_complete or artifact_truncated; capacity/storage failure preserves inline text with artifact_unavailable. Metadata validates schemaVersion2, UUIDv4, positive storedBytes<=4 MiB, nonnegative originalBytes,64 hex SHA, plain/json content type, truncated boolean and no unknown keys. No arbitrary filesystem path is accepted in the reference.

Resolve storage under agentDir/artifacts/subagent-output/SHA256(ownerSessionId)/uuid/output.txt or output.json; owner ID1..512 bytes and root path<=4096. Refuse symlink components/canonical escape, create private directories 0700 and exclusive O_NOFOLLOW regular file, then mode 0400. This capture path does not fsync file/directory, so a durable registry reference is not a guarantee of power-loss persistence of artifact bytes. Read opens the owned regular file with no-follow, checks dev/ino and bounded size, reads and verifies exact stored length, SHA and UTF-8 round trip before returning any chunk. A concurrent file growth after fstat can still cause readFileSync to read more than the prechecked size before integrity failure; do not claim descriptor read is allocation-bounded by that earlier stat.

read requires safe integer offset>=0, length 1..65536 and owned known UUID; cross-owner and unknown IDs share the same not-found error. Offset must be on a UTF-8 boundary, end backs up to a boundary, and a requested length too short for the next code point rejects. EOF offset==stored length returns an empty final chunk. Reply includes nextOffset, totalBytes, bytesRead, eof, contentType, SHA, truncated. Runtime applies equivalent range checks to inline data; public agent_control length max 49152 is a narrower schema bound.

References are idempotent logical owners. capture ownership can be promoted to durable:v2:childID; retainDurable removes the capture ref. release removes only the exact matching ref, and deletes the file only when none remain; it removes accounting before best-effort cleanup. dispose drops process-local refs while preserving durable ones. Restore registers each persisted reference only after ownership/integrity/capacity checks, downgrades invalid saved artifacts explicitly, then finalizeRestore sweeps unregistered UUID children for that owner, deleting only known plain output files and preserving unexpected contents/other owners. It does not discover all owners for global quota accounting. Replacing the latest durable child result releases its predecessor after the new record is durable; deletion releases the current ref.

### 5. Interfaces

SubagentOutputArtifactStore capture, read, retain, retainDurable, registerDurable, release, finalizeRestore, dispose. Output {text, textBytes, originalBytes, inlineTruncated, captureStatus, artifact?}; artifact {schemaVersion:2, id, storedBytes, originalBytes, sha256, contentType, truncated}. Errors artifact_not_found|artifact_invalid|artifact_integrity_failure|artifact_offset_invalid|artifact_storage_failure. Legacy schemaVersion1 report artifacts are a separate retained helper.

V2OutputCapture is {textBytes, originalBytes, inlineTruncated, captureStatus, artifact?}, the metadata-only subset of SubagentOutput. SubagentOutputArtifactStoreOptions={artifactRoot?, limits?:Partial<SubagentOutputArtifactStoreLimits>}; the limits type names inlineBytes, maxArtifactBytes, defaultReadBytes, maxReadBytes, maxOwnerBytes, maxGlobalBytes. CaptureSubagentOutputInput={ownerSessionId, text, contentType?, lifecycle?:process_local|durable}; ReadSubagentOutputInput={ownerSessionId, artifactId, offset?, length?}; RetainSubagentOutputInput adds required referenceId, while ReleaseSubagentOutputInput permits optional referenceId. RESULT_TEXT_LIMIT=49152; durableOutputReferenceId(childId) yields durable:v2:<childId>; outputArtifactUnavailable removes artifact and preserves inline metadata with captureStatus=artifact_unavailable.

### 6. Dependencies

`SESSION-031`, `SUB-013`, `SUB-016`

Requires SUB-013 (Frozen child runtime contract and hash-checked transcript initialization); SUB-016 (Owner-scoped child registry, generation checks, parking and tombstones); SESSION-031 (Lazy first-message persistence, exclusive creation and durable append barrier). Requires private local directories, exclusive/no-follow file operations, SHA-256 and UTF-8 validation.

### 7. ice-rust today

rust:crates/ice-cli/src/child_agents.rs:140-206; rust:crates/ice-cli/src/child_agents.rs:288-418; rust:crates/ice-harness/src/agent_harness/child.rs:74-172 Native child output is retained in memory under 4 MiB cumulative delta+final and 8192 delta-event limits; polls destructively drain deltas and final result is withheld until they drain. It has no opaque disk artifact, nondestructive UTF-8 range read, persistent retention ref or restart-integrity check. Preserve the native total output bound as a separate protection.

### 8. Port notes

Target the child-agent host in ice-cli and a V2 subagent extension module over ice-harness; reuse Tokio cancellation/lifetime primitives and add typed host methods for the specified owner-scoped operations. Implement a bounded owned artifact service with exact metadata and reference lifecycle. Prefer descriptor-bounded reads and fsync before durable-reference publication if strengthening current TS behavior. Keep inline capture on quota/storage failure and ensure replacing/deleting results releases only their own artifact.

### 9. Size and priority

Estimated Rust implementation: **270-450 lines** (estimate, not measured source). **P1** - Large results must remain accessible without unbounded model context or cross-owner path access.

## SUB-029 - Deterministic assurance helpers and bounded batch synthesis

### 1. Title and status

Deterministic assurance helpers and bounded batch synthesis. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/subagents/results/assurance.ts:1-466; ice:packages/coding-agent/src/subagents/results/batch-synthesis.ts:1-471; ice:packages/coding-agent/test/ice-subagent-assurance.test.ts:15-560; ice:packages/coding-agent/test/ice-subagent-batch-synthesis.test.ts:29-426; ice:packages/coding-agent/test/suite/subagents-v2-results.test.ts:1-696

Exported symbols: `SubagentAssuranceLevel` (`ice:packages/coding-agent/src/subagents/results/assurance.ts:19`), `SubagentReportMode` (`ice:packages/coding-agent/src/subagents/results/assurance.ts:26`), `SubagentAssuranceLevelInput` (`ice:packages/coding-agent/src/subagents/results/assurance.ts:28`), `SUBAGENT_ASSURANCE_LEVEL_TEXT` (`ice:packages/coding-agent/src/subagents/results/assurance.ts:42`), `SubagentEvidenceDescriptor` (`ice:packages/coding-agent/src/subagents/results/assurance.ts:72`), `SubagentObservedCommand` (`ice:packages/coding-agent/src/subagents/results/assurance.ts:79`), `SubagentEvidenceObservation` (`ice:packages/coding-agent/src/subagents/results/assurance.ts:84`), `SubagentEvidenceValidationResult` (`ice:packages/coding-agent/src/subagents/results/assurance.ts:95`), `SubagentImplementationFileMismatchReason` (`ice:packages/coding-agent/src/subagents/results/assurance.ts:230`), `SubagentImplementationCommandMismatchReason` (`ice:packages/coding-agent/src/subagents/results/assurance.ts:231`), `SubagentImplementationFileMismatch` (`ice:packages/coding-agent/src/subagents/results/assurance.ts:233`), `SubagentImplementationCommandMismatch` (`ice:packages/coding-agent/src/subagents/results/assurance.ts:238`), `SubagentImplementationClaimsInput` (`ice:packages/coding-agent/src/subagents/results/assurance.ts:243`), `SubagentImplementationClaimsObservation` (`ice:packages/coding-agent/src/subagents/results/assurance.ts:248`), `SubagentImplementationClaimsResult` (`ice:packages/coding-agent/src/subagents/results/assurance.ts:256`), `SUBAGENT_ASSURANCE_SECTION_LIMITS` (`ice:packages/coding-agent/src/subagents/results/assurance.ts:372`), `SubagentAssuranceSectionValidation` (`ice:packages/coding-agent/src/subagents/results/assurance.ts:382`).

Exported symbols: `SubagentBatchSynthesisStatus` (`ice:packages/coding-agent/src/subagents/results/batch-synthesis.ts:22`), `SubagentBatchContradictionKind` (`ice:packages/coding-agent/src/subagents/results/batch-synthesis.ts:24`), `SubagentBatchSynthesisItemResult` (`ice:packages/coding-agent/src/subagents/results/batch-synthesis.ts:32`), `SubagentBatchSynthesisItem` (`ice:packages/coding-agent/src/subagents/results/batch-synthesis.ts:46`), `V2BatchSynthesisResult` (`ice:packages/coding-agent/src/subagents/results/batch-synthesis.ts:52`), `SubagentBatchSynthesisCounts` (`ice:packages/coding-agent/src/subagents/results/batch-synthesis.ts:67`), `SubagentBatchSynthesisCoverageRow` (`ice:packages/coding-agent/src/subagents/results/batch-synthesis.ts:74`), `SubagentBatchEvidenceOverlapPair` (`ice:packages/coding-agent/src/subagents/results/batch-synthesis.ts:82`), `SubagentBatchContradiction` (`ice:packages/coding-agent/src/subagents/results/batch-synthesis.ts:88`), `SubagentBatchSynthesis` (`ice:packages/coding-agent/src/subagents/results/batch-synthesis.ts:96`), `SubagentBatchDuplicatePair` (`ice:packages/coding-agent/src/subagents/results/batch-synthesis.ts:103`), `summarizeV2BatchSynthesis` (`ice:packages/coding-agent/src/subagents/results/batch-synthesis.ts:252`).

### 3. What it does

Expose explicit assurance levels and preserve conflicting child evidence in a deterministic, bounded batch summary.

### 4. How it works

The assurance library is pure and performs no I/O. Levels are unverified_plain, evidence_checked, criteria_verified, verification_executed and review_verified.

A plain_final_turn is always unverified_plain. Otherwise reviewerVerified wins independently; then an observed and successful verification command; then all required criteria verified; then evidence checked; otherwise unverified. These are precedence rules, not a requirement that every lower level also be true. V2 uses the level/render helpers with its observed-file verifier; it does not wire the library's generic command/diff-claim verifiers into the active child result path.

Generic evidence validation checks lexically normalized paths against exact observed paths, artifacts against owned IDs, commands after collapsing whitespace against observed command/optional claimed exit status, and every diff path against the owned changed-path set. An empty diff path list verifies vacuously; undefined ownership data leaves a diff unverified. not_run is always verified as reporting uncertainty. Unknown/unobserved claims are fabricated; precision is verified descriptors divided by total, or zero when empty. Implementation claims first enforce optional lexical workspace containment, then compare modified paths to the patch and commands to observed command/status pairs. Reasons are outside_workspace, not_in_patch, not_observed and exit_status_mismatch. This library neither canonicalizes filesystem paths nor executes a verifier. Assurance rendering sorts IDs, redacts strings, caps each item at 256 UTF-8 bytes, lists 8 mismatches per kind and caps the section at 2048 bytes with a UTF-8-safe truncation marker.

The duplicate-task helper compares whitespace-normalized task text, sorted scope roots/targets, kind and deliverable, retaining the first occurrence. It ignores model/tool differences and is not called by V2 launch: do not add a live duplicate-rejection rule on the strength of this exported helper. Active V2 synthesis projects checked validation state; criterion identity is based on redacted, trimmed, whitespace-normalized lowercase criterion text rather than task-local positional IDs. The generic synthesizer itself joins requirement states by their supplied IDs.

Synthesis sorts tasks lexically and counts completed/failed/other, verified/unresolved criteria and original evidence-path array length. Pairwise overlap is exact-path set intersection, sorted and capped at 16 paths per pair. A finding contradiction is a high-severity same normalized category with opposing substring-based polarity; negative markers include no , not , cannot, unable, missing and absent. This heuristic is not semantic review. Requirement contradictions preserve a shared criterion with differing verified booleans. Contradictions sort by task pair, kind and identity and cap at 32; emitted summaries redact and cap at 200 Unicode code points. Results are deeply frozen. Rendering is deterministic, capped at 4096 UTF-8 bytes with a marker, and explicitly preserves disagreement for the parent instead of averaging it away.

### 5. Interfaces

deriveSubagentAssuranceLevel, validateSubagentEvidenceClaims, validateSubagentImplementationClaims, renderSubagentAssuranceSection; findExactDuplicateBatchTasks, summarizeBatchSynthesis, renderBatchSynthesisSection. These exported library contracts and the narrower V2 call sites must remain separately identifiable.

Assurance input explicitly carries reportMode, evidenceChecked, requiredCriteriaAllVerified, verificationCommandObserved, verificationCommandPassed, reviewerVerified. Generic evidence descriptors are path{id, path}, artifact{id, artifactId}, command{id, command, claimedExitStatus?}, diff{id, changedPaths}, not_run{id, reason}; observations carry observedPaths, observedCommands, ownedArtifactIds, ownedChangedPaths?. Validation returns verifiedIds, unverifiedIds, fabricatedIds, precision. Implementation claim results carry fileMismatches(path, not_in_patch|outside_workspace), commandMismatches(command, not_observed|exit_status_mismatch) and consistent.

SubagentBatchSynthesis={counts:{total, completed, failed, other}, coverage:[{taskId, status:completed|failed|other, verifiedCriteria, unresolvedCriteria, evidencePathCount}], evidenceOverlap:[{taskIdA, taskIdB, sharedPaths}], contradictions:[{taskIdA, taskIdB, kind:finding_claim|requirement_claim, summaryA, summaryB}]}. Duplicate pairs are {firstId, duplicateId}. summarizeV2BatchSynthesis projects id/outcome and validation paths/findings/requirements into the generic input; optional source summary is structural compatibility metadata and is not prose synthesized into a verdict.

### 6. Dependencies

`SUB-026`, `SUB-027`

Requires SUB-026 (Bounded report envelope and local output-schema validation); SUB-027 (Per-turn observed file evidence, acceptance criteria and honest assurance). Pure local result aggregation; no external service.

### 7. ice-rust today

rust:crates/ice-cli/src/child_agents.rs:288-418 returns bounded text, turns and turnLimited without observed-evidence assurance, generic claim validation, multi-child overlap or contradiction synthesis. rust:crates/ice-harness/src/agent_harness/child.rs:74-172 executes child turns but adds no batch report layer.

### 8. Port notes

Target a native subagent definition/policy/result module composed by ice-cli, with serializable Rust structs and the workspace JSON types; no provider service is required for these local rules. Add pure typed helpers with deterministic ordering and explicit observation inputs, then wire the active V2 projection to the host-owned result verifier. Keep heuristic contradictions labeled as disagreements. Do not treat reviewed/verified flags as proof of an executed command and do not activate dormant duplicate-task rejection implicitly.

### 9. Size and priority

Estimated Rust implementation: **320-500 lines** (estimate, not measured source). **P1** - Parents need truthful evidence summaries and preserved disagreements when several children work concurrently.

## SUB-030 - Raw dirty-tree snapshots and owned isolated Git worktrees

### 1. Title and status

Raw dirty-tree snapshots and owned isolated Git worktrees. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/subagents/isolation/worktree.ts:1-487; ice:packages/coding-agent/test/subagents-v2-worktree.test.ts:1-414; ice:packages/coding-agent/test/suite/subagents-v2-runtime.test.ts:1-1824

Exported symbols: `WORKTREE_LIMITS` (`ice:packages/coding-agent/src/subagents/isolation/worktree.ts:26`), `WorktreeIsolationError` (`ice:packages/coding-agent/src/subagents/isolation/worktree.ts:36`), `worktreeGit` (`ice:packages/coding-agent/src/subagents/isolation/worktree.ts:52`), `worktreeGitText` (`ice:packages/coding-agent/src/subagents/isolation/worktree.ts:112`), `resolveWorktreePath` (`ice:packages/coding-agent/src/subagents/isolation/worktree.ts:126`), `RawTreeEntry` (`ice:packages/coding-agent/src/subagents/isolation/worktree.ts:146`), `readRawTree` (`ice:packages/coding-agent/src/subagents/isolation/worktree.ts:152`), `WorktreeOwner` (`ice:packages/coding-agent/src/subagents/isolation/worktree.ts:324`).

Exported symbols: `childCwd` (`ice:packages/coding-agent/src/subagents/v2/runtime-support.ts:378`).

### 3. What it does

Start an isolated child from the parent's actual dirty worktree bytes while preserving the parent's index and providing verifiable cleanup ownership.

### 4. How it works

The Git wrapper uses literal execFileSync argv with a 60 s timeout, ordinarily9MiB maxBuffer and a larger260MiB cat-file bound. It removes inherited GIT_* environment variables; disables prompts, system/global Git configuration, system attributes, lazy fetch, hooks, fsmonitor, untracked cache, split index and merge renormalization, and supplies synthetic author identity.

Local repository configuration still exists; external merge drivers are rejected separately during apply. It never invokes the user's shell for Git.

Require a repository with HEAD. Enumerate cached stage-zero entries and nonignored untracked files, rejecting unmerged entries, submodules/gitlinks and nested repositories. Sort their union; omit missing tracked paths. Reject empty/NUL paths, empty/dot/dotdot segments, case-insensitive .git segments, lexical escapes, symlink ancestors and nested repository ancestors. Leaf symlinks are allowed in the baseline as raw link-target bytes; regular-file contents and executable bits are captured without clean filters, text conversion or newline normalization. The snapshot includes untracked raw bytes, CRLF, non-UTF-8 contents and unusual names. Limit a snapshot to 20000 files/256 MiB, including an untracked subset at 20000 files/64 MiB. Check sizes before reads and recheck content length against the stat. Compute expected Git blob IDs for SHA-1 or SHA-256 repositories; compare hash-object results to detect content drift. Newline-containing names and symlinks use per-object stdin paths instead of line-oriented batching.

Build a tree with an owned temporary index using read-tree --empty, update-index -z and write-tree, then remove that temporary index. The real index is untouched. A synthetic commit uses the captured tree and HEAD parent; retain cwd offset within the repository. Child worktree destination must be absent, outside the parent, and free of symlink components. Write a private adjacent .ice-owner.json before git worktree add --detach --no-checkout. Materialize raw blobs/modes/link targets, initialize its index to the baseline and recapture to verify equality; failures clean up owned state.

Ownership validation requires matching repository/path/baseline marker and an exact git worktree list entry. The marker is read without an explicit byte cap. Pin the worktree snapshot under refs/ice/subagent-v2/worktrees/<SHA256(resolved path)> for Git GC retention. Cleanup force-removes only an owned worktree, then its ref and marker. The force behavior may delete child edits on explicit rejection, but ordinary pruning first checks for changes.

Delta generation uses no-renames name-status -z and binary patch output with external diff/textconv/color disabled. Limit changed paths to 512 and patch bytes to 8 MiB. Keep both raw patch bytes and display text; lossy text is not the integrity source. File-to-directory and directory-to-file replacements are unsupported. Index identity is SHA-256 over its raw bytes. Snapshotting may write Git objects and temporary indexes even when a caller merely asks whether a worktree changed.

### 5. Interfaces

captureWorktreeBaseline, createIsolatedWorktree, snapshotWorktree, computeChildDelta, deltaFromTree, worktreeHasChanges, removeIsolatedWorktree, assertOwnedWorktree, readWorktreeOwner, worktreeSnapshotRef and worktreeIndexHash. WorktreeBaseline carries repoRoot, headCommit, baselineCommit, baselineTree and cwd offset; ChildDelta carries final tree, changed statuses/paths, raw patch bytes and lossy display text. Index hashing is a separate operation.

RawTreeEntry={path, mode:100644|100755|120000, blob}; WorktreeOwner={repoRoot, worktreePath, baselineCommit}. WorktreeIsolationError.code=not_a_repository|no_commits|nested_repository|baseline_too_large|git_failed|unsafe_path|ownership. worktreeGitText decodes stdout as UTF8 and trims it. childCwd returns parentCwd outside an available worktree isolation; otherwise it returns worktreePath joined with optional captured cwdOffset.

### 6. Dependencies

`SUB-013`, `SUB-018`

Requires SUB-013 (Frozen child runtime contract and hash-checked transcript initialization); SUB-018 (Launch preflight, frozen model choice, unique names and staged batch acceptance). Requires a local Git executable and filesystem access under explicit workspace ownership; use literal argv.

### 7. ice-rust today

rust:crates/ice-harness/src/agent_harness/child.rs:74-172 clones parent HarnessTool instances and execution context; rust:crates/ice-cli/src/child_agents.rs:208-286 creates only an in-memory child session. Neither constructs an isolated Git checkout or snapshots dirty files/index identity. Native inherited execution scopes remain useful for lifecycle control but do not isolate filesystem mutations.

### 8. Port notes

Target an owned isolation service composed by ice-cli, using literal std::process/Tokio Git invocations, bounded byte buffers and serializable recovery metadata; Git is a required local binary. Implement a raw-byte Git snapshot/worktree service with explicit object-format support, literal command arguments and bounded subprocess output. Preserve dirty baseline and real-index invariants; distinguish lexical path checks, filesystem ownership checks and the remaining external-writer race. Keep unsupported structural replacements explicit instead of silently losing data.

### 9. Size and priority

Estimated Rust implementation: **430-680 lines** (estimate, not measured source). **P0** - An isolated child must start from the actual parent state and must never mutate or discard unrelated parent work.

## SUB-031 - Pinned immutable patch artifacts and private recovery records

### 1. Title and status

Pinned immutable patch artifacts and private recovery records. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/subagents/isolation/patch-state.ts:1-133; ice:packages/coding-agent/src/subagents/api/agent-patch.ts:1-21; ice:packages/coding-agent/src/subagents/isolation/worktree.ts:1-487; ice:packages/coding-agent/test/suite/subagents-v2-runtime.test.ts:1-1824

Exported symbols: `AGENT_PATCH_ACTIONS` (`ice:packages/coding-agent/src/subagents/api/agent-patch.ts:11`), `AgentPatchAction` (`ice:packages/coding-agent/src/subagents/api/agent-patch.ts:12`), `AgentPatchParametersSchema` (`ice:packages/coding-agent/src/subagents/api/agent-patch.ts:14`), `AgentPatchParameters` (`ice:packages/coding-agent/src/subagents/api/agent-patch.ts:21`).

Exported symbols: `reconcileTerminalWorktrees` (`ice:packages/coding-agent/src/subagents/v2/runtime-support.ts:357`).

### 3. What it does

Make patch inspection select an immutable child delta and keep enough private transaction evidence to diagnose an interrupted integration.

### 4. How it works

Place patch artifacts outside both parent and child worktrees and reject symlink components. Create the artifact directory privately 0700.

The filename combines final tree and patch SHA as <finalTree>-<patchsha>.patch. Create an exclusive 0444 file, fsync it and fsync its directory on non-Windows. If the exact file already exists, require a regular file with identical raw bytes; never replace mismatching data. Pin a synthetic final-tree commit whose parent is the baseline commit under the owned worktree GC ref.

Persist finalTree, baselineTree, contractHash, patchRef, patchSha256 and patchBytes with the child's patch state. Validation checks the filename form, a 40–64 hex final-tree identity, baseline and contract equality, regular file type, size<=8 MiB, recorded length and SHA-256, then regenerates the tree delta and requires byte-for-byte equality. Model-visible display text is not used to validate or apply the patch. Inspect establishes this pinned artifact; a controller preview merely computes a delta and does not satisfy the inspect-before-apply requirement.

Before parent writes, the apply layer can durably write a private UUID.recovery.json containing base64 preimages and transaction metadata at 0600, with file/directory fsync. This is recovery evidence, not model context. The runtime persists the applying state before mutation and commits the terminal applied state before discarding recovery preimages. Restart can recognize open/terminal worktree state and perform terminal cleanup, but the audited code does not automatically replay or roll back an interrupted applying transaction. Do not turn a recovery record into an unreviewed restart mutation.

reconcileTerminalWorktrees visits only the current owner's records including deleted ones, and selects state=deleted or patchState=applied|rejected|unchanged. It reopens the hash-checked child contract, requires recorded repoRoot/worktreePath, then removes an existing owned worktree or deletes its snapshot ref if the directory is already absent. Missing transcripts, ownership failures and Git failures are caught and leave the remainder for inspection; there is no interrupted applying replay.

### 5. Interfaces

persistWorktreePatch, validateWorktreePatch and persistPatchRecovery; WorktreePatchArtifact {finalTree, baselineTree, contractHash, patchRef, patchSha256, patchBytes}. agent_patch action inspect|apply|reject with a child id/name reference matching CHILD_REF_PATTERN_SOURCE; no additional schema properties.

### 6. Dependencies

`SUB-013`, `SUB-016`, `SUB-030`

Requires SUB-013 (Frozen child runtime contract and hash-checked transcript initialization); SUB-016 (Owner-scoped child registry, generation checks, parking and tombstones); SUB-030 (Raw dirty-tree snapshots and owned isolated Git worktrees). Requires local Git object/ref operations, SHA-256 and durable private-file creation.

### 7. ice-rust today

rust:crates/ice-cli/src/child_agents.rs:140-286 has owner-scoped volatile job IDs but no isolated tree identity, immutable patch file, inspect/apply state or durable preimage record. rust:crates/ice-harness/src/agent_harness/child.rs:74-172 does not add a filesystem transaction layer.

### 8. Port notes

Target an owned isolation service composed by ice-cli, using literal std::process/Tokio Git invocations, bounded byte buffers and serializable recovery metadata; Git is a required local binary. Add an owned artifact format and fsync publication sequence, reuse the baseline service to prove that pinned bytes describe the expected tree, and keep recovery records out of model-visible output. Model interrupted-apply state explicitly; implement any replay policy as a separate deliberate recovery feature rather than claiming existing TS automation.

### 9. Size and priority

Estimated Rust implementation: **190-310 lines** (estimate, not measured source). **P0** - The user must approve and apply exactly the inspected child change, with durable evidence of any interrupted parent write.

## SUB-032 - Authorized patch integration, verification and conflict-preserving rollback

### 1. Title and status

Authorized patch integration, verification and conflict-preserving rollback. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/subagents/isolation/apply.ts:1-289; ice:packages/coding-agent/src/subagents/isolation/patch-state.ts:1-133; ice:packages/coding-agent/src/subagents/api/agent-patch.ts:1-21; ice:packages/coding-agent/test/subagents-v2-worktree.test.ts:1-414; ice:packages/coding-agent/test/suite/subagents-v2-runtime.test.ts:1-1824

Exported symbols: `ApplyOutcome` (`ice:packages/coding-agent/src/subagents/isolation/apply.ts:19`), `ApplyRecovery` (`ice:packages/coding-agent/src/subagents/isolation/apply.ts:29`), `ApplyOptions` (`ice:packages/coding-agent/src/subagents/isolation/apply.ts:36`).

### 3. What it does

Integrate the exact inspected child delta into the parent only under current authority, preserving concurrent user edits and returning an honest verification outcome.

### 4. How it works

Runtime apply requires a trusted parent in build mode, active agent_patch, an open patch and explicit allow for every affected parent write; ask is not silently approved. It validates the inspected artifact and rejects child changes made after inspection.

Parent authority is checked before preparation, immediately before writing and after verification. Resume/delete are blocked while integration is in progress. A failed verification reopens the patch; applied/empty parks the child with best-effort cleanup. Reject records its terminal state before owned force-removal.

The integration helper obtains a synthetic repository lock through the shared in-process mutation queue, then sorted individual path locks. These coordinate application tools, not unrelated OS writers. An empty delta returns empty. Reject out-of-scope/protected paths, symlink leaves in baseline/final state, unsupported file/directory replacements and configured external Git merge drivers. Capture real-index hash and current raw parent tree. If the parent diverged from the child baseline, construct ours/theirs commits with a common baseline and use git merge-tree --write-tree --no-messages --merge-base; a conflict returns bounded 4096-character diagnostics before parent writes. The resulting changed paths must be a subset of the child's delta; no new merge-introduced paths are accepted.

Capture regular-file preimages with raw blob/mode checks. Preserve existing non-executable mode bits while taking the resulting Git executable bit; the tests include 0600→0711. Combined preimage/postimage bytes must fit 16 MiB. Under locks recheck index and preimages; drift before writes returns changed_during_apply with the path or Git index. Reauthorize, then invoke beforeWrite to durably publish recovery data/applying state. For each path, recheck expected current contents and use an exclusive temporary file, chmod and rename, or delete the expected regular file. Track only writes that actually completed. Optional verification must return passed; afterward reauthorize and verify all postimages, unchanged real index and complete parent snapshot==result tree. Commit the durable applied result before discarding preimages. Without a verifier, successful apply remains unverified=true.

On any failure, walk completed writes in reverse. Restore only when current content still equals the transaction's expected postimage; a path already equal to its preimage needs no work. Preserve newer content and report rollback_conflict with affected paths rather than overwriting it. Otherwise report verification_failed after rollback. Do not restore a changed index or unrelated external writes; created empty directories may remain. Thus the contract is conflict-preserving best-effort file rollback, not an atomic repository transaction. Tests cover nonoverlapping dirty-parent merge, conflicts, verification rollback, index mutation during verification and newer-file preservation.

### 5. Interfaces

applyChildDelta with authorize, beforeWrite, verify, commit hooks; result statuses include empty, applied, conflict, changed_during_apply, verification_failed and rollback_conflict. agent_patch inspect|apply|reject returns normal tool text/details; external RPC execution must pass through the ordinary tool hooks rather than directly invoking the helper.

ApplyOutcome is applied{writtenPaths, verification} or applied{writtenPaths, unverified:true}, empty, conflict{details}, changed_during_apply{path}, verification_failed{details, verification?}, or rollback_conflict{details, paths, verification?}. ApplyRecovery={repoRoot, indexHash, parentTree, paths:[{path, before?:{base64, mode}, after?:{sha256, mode}}]}. ApplyOptions={scopeRoots?, verify?:(root)=>Promise<IceVerifierState>, authorize?:()=>void, beforeWrite?:(recovery)=>void, commit?:()=>void}. These discriminants preserve unverified successful integration separately from verified success.

### 6. Dependencies

`SAFE-004`, `SAFE-019`, `SAFE-020`, `SAFE-025`, `SET-003`, `SUB-007`, `SUB-016`, `SUB-030`, `SUB-031`, `TOOLS-021`

A trusted verifier/ice-safe-verify integration and ordinary RPC tool-hook path need coordinator-assigned cross-area prerequisites; no unresolved placeholder IDs are asserted.

Final dependency classification. Shared contract references: `SAFE-018`, `SAFE-024`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

rust:crates/ice-harness/src/agent_harness/child.rs:74-172 inherits parent tools directly; rust:crates/ice-cli/src/child_agents.rs:208-418 provides child lifecycle/output only. No isolated child patch, inspected identity, parent merge verification or preimage rollback service is present in these native child paths.

### 8. Port notes

Target an owned isolation service composed by ice-cli, using literal std::process/Tokio Git invocations, bounded byte buffers and serializable recovery metadata; Git is a required local binary. Implement the transaction with explicit authority checks and immutable patch input. Reuse the in-process mutation queue but still compare pre/postimages for external races. Keep verification failure separate from rollback conflict, preserve the real index, and ensure all UI/SDK/RPC apply entry points share the same ordinary tool hooks and trusted verifier contract.

### 9. Size and priority

Estimated Rust implementation: **420-700 lines** (estimate, not measured source). **P0** - Applying delegated changes is a user-data mutation and must preserve current permissions, concurrent edits and an auditable rollback outcome.

## SUB-033 - Agent catalog CLI, definition creation and conservative orphan pruning

### 1. Title and status

Agent catalog CLI, definition creation and conservative orphan pruning. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/subagents/v2/cli.ts:1-136; ice:packages/coding-agent/src/subagents/v2/agent-manager.ts:1-55; ice:packages/coding-agent/src/subagents/v2/prune.ts:1-159; ice:packages/coding-agent/src/ice.ts:1-131; ice:packages/coding-agent/test/subagents-v2-cli-events.test.ts:1-169; ice:packages/coding-agent/test/subagents-v2-prune.test.ts:1-114; ice:packages/coding-agent/test/subagents-v2-hub.test.ts:345-376

Exported symbols: `formatAgentDefinitions` (`ice:packages/coding-agent/src/subagents/v2/agent-manager.ts:14`), `agentDefinitionTemplate` (`ice:packages/coding-agent/src/subagents/v2/agent-manager.ts:30`).

Exported symbols: `AgentsCliResult` (`ice:packages/coding-agent/src/subagents/v2/cli.ts:20`), `loadAgentsForCli` (`ice:packages/coding-agent/src/subagents/v2/cli.ts:36`).

Exported symbols: `PruneFolder` (`ice:packages/coding-agent/src/subagents/v2/prune.ts:19`), `PruneResult` (`ice:packages/coding-agent/src/subagents/v2/prune.ts:30`), `formatPruneResult` (`ice:packages/coding-agent/src/subagents/v2/prune.ts:144`).

### 3. What it does

Inspect and validate subagent configuration without starting a chat, safely create a definition, and explicitly prune old orphaned child data.

### 4. How it works

The ice agents dispatch runs before normal startup and supports list [--json], validate [paths...] and prune [--apply] [--json]; unknown command/options return exit 2. List returns 0 and renders active/shadowed/disabled definitions and discovery issues, or equivalent JSON.

It does not fail the exit code for invalid settings and does not include settings diagnostics as a substitute for validation. Validate with no paths combines scoped settings diagnostics and invalid definitions: errors return 1, deprecation warnings alone 0, and an untrusted definition is not inherently malformed. Explicit non-flag paths are read and parsed as native user definitions; those reads have no additional CLI file-size bound, non-agent Markdown succeeds, invalid/unreadable files fail, and this branch does not also perform global catalog/settings validation.

Definition creation validates the native lowercase name, requires trust when creating in the project directory, refuses an existing destination, creates the directory and writes with wx so it never overwrites. Global creation has no extra project-trust requirement. The interactive /agents new command uses this helper.

Pruning scans one level of plain session directories beneath sessionsRoot. It collects live parent IDs by the filename pattern _([^_]+)\.jsonl$ across those directories, without an is-file test for the matching entry. It considers plain subagents/<parentId> folders orphaned only if that ID is absent everywhere. A recent folder or direct child mtime within 10 minutes holds the folder; this is not a recursive newest-descendant scan. It counts direct transcript JSONL files and worktree entries, excluding owner marker files. Every candidate worktree must be plain, have matching ownership metadata and have no delta from its baseline. Unknown, malformed, changed or failed worktrees hold the whole parent folder.

Default is deletion-free planning; --apply removes each verified owned clean worktree through Git, then removes the lexically direct orphan folder recursively. Partial failures become keep reasons and preserve the remainder. Other parents and changed worktrees are retained. The dry-run uses worktreeHasChanges→snapshotWorktree, which can write Git objects and a temporary index: it is not a fully read-only command, although it performs no planned deletions. The audit did not invoke it. The code checks the scanned entries, but does not establish that every ancestor of sessionsRoot is nonsymlink.

### 5. Interfaces

runAgentsCli returns AgentsCliResult {exitCode:number, stdout:string}; createAgentDefinitionFile; pruneOrphanedSubagents returns planned/kept folders, transcript/worktree counts and reasons. CLI --json exposes the same decisions; PRUNE_RECENT_MS=600000.

PruneFolder={path, parentSessionId, transcripts, worktrees, action:remove|keep, reasons:string[]}; PruneResult={applied:boolean, folders:PruneFolder[]}. formatPruneResult reports no orphan data for an empty folder list, otherwise Removed/Would remove and Kept sections with counts/reasons. agentDefinitionTemplate emits native Markdown with read/grep/find/ls, model inherit and thinking medium. AgentsCliResult is the two-field result specified above.

### 6. Dependencies

`SET-003`, `SUB-001`, `SUB-003`, `SUB-005`, `SUB-016`, `SUB-030`

Requires SUB-001 (Strict native Markdown and JSON agent definitions with source identity); SUB-003 (Bounded agent discovery, trust filtering and whole-definition precedence); SUB-005 (Strict scoped V2 settings and monotonic numeric limits); SUB-016 (Owner-scoped child registry, generation checks, parking and tombstones); SUB-030 (Raw dirty-tree snapshots and owned isolated Git worktrees); SET-003 (Project trust resolution before loading executable resources). Requires CLI dispatch, local agent-definition storage and owned Git worktree inspection.

### 7. ice-rust today

rust:crates/ice-cli/src/child_agents.rs:140-286 exposes start/poll/cancel/dispose for volatile jobs; rust:crates/ice-harness/src/agent_harness/child.rs:74-172 has no Markdown catalog or orphaned disk child data. The audited native child surfaces provide no equivalent agents list/validate/prune or definition authoring workflow.

### 8. Port notes

Target an owned isolation service composed by ice-cli, using literal std::process/Tokio Git invocations, bounded byte buffers and serializable recovery metadata; Git is a required local binary. Add CLI dispatch and structured diagnostics over the same discovery/settings validator used at runtime. Preserve exclusive creation and trust checks. Implement pruning as an explicit owner-verified deletion plan with recent/change holds; if promising a strictly read-only preview, avoid snapshot routines that write Git objects and record that deliberate improvement.

### 9. Size and priority

Estimated Rust implementation: **250-400 lines** (estimate, not measured source). **P1** - Users need diagnosable configuration and safe cleanup for persistent child transcripts/worktrees.

## SUB-034 - SDK composition and versioned trusted-extension controller

### 1. Title and status

SDK composition and versioned trusted-extension controller. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/subagents/v2/sdk.ts:1-85; ice:packages/coding-agent/src/subagents/v2/controller.ts:1-163; ice:packages/coding-agent/test/suite/subagents-v2-sdk.test.ts:1-323; ice:packages/coding-agent/test/suite/subagents-v2-controller.test.ts:1-201; ice:packages/coding-agent/test/suite/subagents-v2-approval.test.ts:1-62

Exported symbols: `IceSubagentTask` (`ice:packages/coding-agent/src/subagents/v2/controller.ts:17`), `IceSubagentStartOptions` (`ice:packages/coding-agent/src/subagents/v2/controller.ts:19`), `IceSubagentRunResult` (`ice:packages/coding-agent/src/subagents/v2/controller.ts:26`), `IceSubagentPatchPreview` (`ice:packages/coding-agent/src/subagents/v2/controller.ts:51`), `IceSubagentOpenPatch` (`ice:packages/coding-agent/src/subagents/v2/controller.ts:72`), `createIceSubagentController` (`ice:packages/coding-agent/src/subagents/v2/controller.ts:102`).

Exported symbols: `IceSubagentsSdkOptions` (`ice:packages/coding-agent/src/subagents/v2/sdk.ts:25`), `IceSubagentsSdk` (`ice:packages/coding-agent/src/subagents/v2/sdk.ts:41`).

### 3. What it does

Provide programmatic subagent access through the same parent-owned configuration, approval, runtime and patch contracts used by the model-facing tools.

### 4. How it works

createIceSubagents builds a hidden inline ice-subagents extension at before-user priority, attaches an IceAgentViewBridge and exposes attach, overview and respondToApproval; subscriptions are available through the bridge. Options provide plan/build mode, explicit agents, host hook handlers, MCP adapter and agent directory; mode defaults to plan.

Early overview access must use the actual parent session settings and trust even before session_start, rather than constructing defaults. The wrapper does not independently require a persistent session before construction; the runtime's durable barrier rejects unsupported launches later. In headless build mode an ask permission fails closed without a foreground approval UI/inbox, while an explicit user allow rule may grant the corresponding write.

Trusted extensions find a version 1 controller on their shared events owner using Symbol.for("ice.subagents.controller.v1"). Registration defines a configurable owner property and replaces any previous controller. Its cleanup removes the property only if it still points to the same controller, so stale cleanup cannot erase a replacement. No process-global singleton is required.

start accepts the agent task schema but forbids resume, checks an already-aborted signal, normalizes context into the prompt, calls the normal runtime launch/preflight/approval path, captures the resident frozen model and calls onLaunched before running. runTurn is foreground:true, returns directly to the caller and does not enqueue parent-conversation completion delivery. Return id, agent, frozen {provider, id}, outcome, bounded text/truncated/error, input/output/cache read/cache write/cost usage, optional patchState, verified structured payload and any validation findings. The signal is passed to runTurn but not runtime.launch: cancellation arriving during launch approval does not itself cancel that pending launch. A throwing onLaunched callback can leave an accepted resident child before its first turn; record that behavior instead of promising transactional callback cleanup.

stop delegates to runtime.stop. openPatches lists this owner's patchState=open children. preview computes the open child delta without inspecting/pinning it, returning worktree root, changed paths, display patch and truncated flag. The default cap is 64 KiB, implemented by raw-byte subarray followed by UTF-8 decoding, so it can replace a cut multibyte character; maxBytes is not independently schema-validated. Another child may read an owner's open patch worktree when its allowed scope includes it, while the protected-path guard forbids writing there. Integration still requires ordinary agent_patch inspect/apply.

### 5. Interfaces

createIceSubagents; IceSubagentController version 1 start, stop, preview, openPatches; ICE_SUBAGENT_CONTROLLER_KEY, registerIceSubagentController, getIceSubagentController. The native ChildAgent runtime action is a different host boundary and must not be mistaken for this controller's SDK contract. SDK result {extension, bridge, attach(session), overview(), respondToApproval(id, answer)}; options {mode?, agents?, hookHandlers?, mcpAdapter?, agentDir?}.

IceSubagentTask=Omit<AgentTaskInput, resume>; IceSubagentStartOptions={signal?, onLaunched?}. IceSubagentRunResult includes id, agent, model?:{provider, id}, outcome, text, truncated, error?, usage:{inputTokens, outputTokens, cacheReadTokens?, cacheWriteTokens?, cost}, patchState?, payload?, findings?. IceSubagentPatchPreview={root, changedPaths:[{status, path}], patch, truncated}; IceSubagentOpenPatch={id, name?, agent, description}. SDK readonly extension/bridge fields and attach/overview/respondToApproval methods are separate from the native child-run host actions.

### 6. Dependencies

`EXT-009`, `EXT-010`, `SUB-005`, `SUB-008`, `SUB-018`, `SUB-020`, `SUB-027`, `SUB-028`, `SUB-030`, `SUB-031`

Requires SUB-005 (Strict scoped V2 settings and monotonic numeric limits); SUB-008 (Approval broker with foreground prompts, inbox routing and owner cancellation); SUB-018 (Launch preflight, frozen model choice, unique names and staged batch acceptance); SUB-020 (Admission, liveness deadlines and abandonment of uncooperative child turns); SUB-027 (Per-turn observed file evidence, acceptance criteria and honest assurance); SUB-028 (Owner-scoped output artifacts, UTF-8 range reads and durable retention); SUB-030 (Raw dirty-tree snapshots and owned isolated Git worktrees); SUB-031 (Pinned immutable patch artifacts and private recovery records); EXT-009 (Core action rebinding and explicit UI mode defaults); EXT-010 (First-cause invalidation and lazy stale-context guards). Requires the existing extension host, SDK session binding and trusted controller registry.

### 7. ice-rust today

rust:crates/ice-cli/src/child_agents.rs:140-286 implements an owner-scoped native host start/poll/cancel/dispose protocol and rust:crates/ice-harness/src/agent_harness/child.rs:74-172 creates inherited child harnesses. This is useful partial infrastructure, but has no versioned per-events-owner V2 controller, plan-default SDK wrapper, durable result/patch preview or equivalent task schema. rust:crates/ice-cli/tests/subagent_packages.rs:23-375 and rust:crates/ice-cli/tests/subagent_packages/lifetime.rs:3-63 exercise an independently supplied DLL only when ICERS_TEST_SUBAGENT_LIBRARY is set; the audited repository did not establish that package's implementation.

### 8. Port notes

Target the child-agent host in ice-cli and a V2 subagent extension module over ice-harness; reuse Tokio cancellation/lifetime primitives and add typed host methods for the specified owner-scoped operations. Compose the public SDK/controller over the same native V2 runtime and owner lifetime. Preserve native stale-handle rejection and weak host bindings. Explicitly decide whether to strengthen cancellation during launch and onLaunched cleanup; implement a bounded UTF-8 preview without equating preview with inspection.

### 9. Size and priority

Estimated Rust implementation: **200-340 lines** (estimate, not measured source). **P1** - SDK and trusted-extension callers need the same permission and result guarantees as interactive/model-driven delegation.

## SUB-035 - Subagent extension binding, session catalog refresh and lifecycle events

### 1. Title and status

Subagent extension binding, session catalog refresh and lifecycle events. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/subagents/v2/extension.ts:1-1048; ice:packages/coding-agent/src/subagents/v2/events.ts:1-74; ice:packages/coding-agent/src/subagents/extension-support.ts:22-243; ice:packages/coding-agent/src/ice.ts:1-131; ice:packages/coding-agent/test/subagents-v2-extension.test.ts:1-657; ice:packages/coding-agent/test/suite/subagents-v2-lifecycle.test.ts:1-349; ice:packages/coding-agent/test/subagents-v2-cli-events.test.ts:1-169

Exported symbols: `V2_SUBAGENT_TOOL_NAMES` (`ice:packages/coding-agent/src/subagents/api/tool-set.ts:2`).

Exported symbols: `SUBAGENT_EVENT_CHANNEL` (`ice:packages/coding-agent/src/subagents/v2/events.ts:10`), `SubagentLifecycleEventType` (`ice:packages/coding-agent/src/subagents/v2/events.ts:12`), `SubagentLifecycleEvent` (`ice:packages/coding-agent/src/subagents/v2/events.ts:25`).

### 3. What it does

Bind one runtime to the current parent session, make the catalog and child state visible at the right conversation boundaries, and expose bounded lifecycle metadata to extensions.

### 4. How it works

The inline ice-subagents extension registers agent, agent_control and agent_patch; patch execution is sequential. Caught model-facing errors become normal text/details rather than automatically setting isError.

It registers the agents string flag and sub-yolo boolean flag, uses the current parent mode as build only when explicitly build, and combines sub-yolo input with the startup authorization contract. Settings are memoized by serialized global/project ice.subagents values and current trust. Host-pinned hook/MCP/verifier adapters are supplied through the extension options rather than arbitrary model arguments.

Parent binding follows the latest extension context and attached bridge. A shut-down-parent WeakSet prevents later callbacks from journaling against a closed parent; resume/switch rebinds to the new parent before launch. session_start invalidates configuration cache, initializes/restores runtime ownership, reconnects registry/live controls and prepares catalog/roster delivery. Startup/resume may flush persisted interruption/completion notices; a forced model turn occurs only when hasUI and pending interruption are both true. New/fork initialization does not replay another owner's work. Fork reports the count of children still owned by the original session rather than adopting them.

before_agent_start injects the incremental agent listing and, after startup/resume/compaction, a roster of the last 20 depth-zero children with description prefixes 80 characters. The combined catalog/roster text is not recapped as a whole. Ordinary repeat turns suppress unchanged catalog/roster data; compaction makes it eligible again. message_end confirms posted completion delivery, and session_compact/agent_settled flush pending idle deliveries. session_shutdown cancels/settles the runtime, removes controller/listener bindings and prevents stale callbacks from acquiring the departed owner. The exact durable delivery transaction and crash window are SUB-017.

Registry event serialization emits subagent_start, subagent_turn_start, subagent_turn_end (with blocked-transition exceptions), subagent_parked, subagent_revived, subagent_blocked, subagent_end, subagent_interrupted and subagent_patch; completion notification is a separate path. Events use the ice:subagent channel and metadata derived from the child record, including type, childId, agent, ownerSessionId, depth, state, optional lastTurnOutcome and optional patchState; no name or usage is added by lifecycleEvents/completionEvent. They do not include raw task prompts or raw report output; bounds come from validated contracts/record fields, not a generic serializer-wide cap.

Foreground approval presents Allow once, an offered allow-session rule, or Deny. Background requests surface through the owner inbox and event/UI notifications; no UI/inbox means deny. The view bridge exposes owner overview, approval answers, durable/live messages and stop/resume/delete/send. Runtime errors for unknown/cross-owner messages return undefined through the bridge. The command layer opens /agents hub and native definition creation. Main ice.ts loads the inline extension and MCP bridge before normal execution, with session shutdown releasing the parent MCP proxy binding.

completionEvent emits subagent_completion_queued with state=idle and the completion outcome. The extension separately emits subagent_permission_request with depth=0, state=blocked and the requested tool after in-memory per-request announcement suppression. The lifecycle union permits both, but lifecycleEvents alone does not emit them.

### 5. Interfaces

iceSubagentsV2(ice, IceSubagentsOptions); agent/agent_control/agent_patch registration; --agents and --sub-yolo; /agents; ice:subagent lifecycle events; session_start, before_agent_start, message_end, session_compact, agent_settled, session_shutdown listeners; view/controller binding cleanup.

V2_SUBAGENT_TOOL_NAMES is Object.freeze(['agent','agent_control','agent_patch'] as const). SubagentLifecycleEvent={type, childId, agent, ownerSessionId, depth, state, lastTurnOutcome?, patchState?}. The event type union includes start, turn_start, turn_end, parked, revived, blocked, interrupted, end, patch, completion_queued and permission_request with the subagent_ prefix; the channel is exactly ice:subagent.

### 6. Dependencies

`EXT-009`, `EXT-010`, `EXT-013`, `EXT-015`, `EXT-017`, `EXT-022`, `EXT-054`, `SUB-003`, `SUB-004`, `SUB-005`, `SUB-008`, `SUB-009`, `SUB-016`, `SUB-017`, `SUB-022`, `SUB-023`, `SUB-024`, `SUB-032`, `SUB-033`, `SUB-034`

Session-start/rebind/compaction and RPC command integration require the coordinator's final session/RPC IDs. SUB owns the extension consumer behavior; source session persistence/transport stays with those areas.

### 7. ice-rust today

rust:crates/ice-cli/src/child_agents.rs:140-418 has a manager tied to plugin owner and parent cancellation, but no V2 inline extension catalog, durable owner rebind, named lifecycle event family, approval inbox or model-facing three-tool contract. rust:crates/ice-cli/tests/subagent_packages.rs:291-371 demonstrates native plugin registration/reload binding isolation in conditional DLL tests; retain that native lifetime guarantee.

### 8. Port notes

Target the child-agent host in ice-cli and a V2 subagent extension module over ice-harness; reuse Tokio cancellation/lifetime primitives and add typed host methods for the specified owner-scoped operations. Create a native extension/runtime composition layer that uses existing event dispatch and owner lifetimes. Route every UI/SDK/model command to the same runtime. Keep catalog dedup tied to actual conversation boundaries and do not infer exactly-once completion delivery from in-memory suppression.

### 9. Size and priority

Estimated Rust implementation: **300-480 lines** (estimate, not measured source). **P1** - Correct session ownership and event wiring determine whether already implemented child features are reachable without cross-session leakage.

## SUB-036 - Bounded child view bridge, historical transcripts and live input control

### 1. Title and status

Bounded child view bridge, historical transcripts and live input control. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/ice-agent-view-bridge.ts:1-904; ice:packages/coding-agent/src/subagents/runtime/live-session.ts:1-529; ice:packages/coding-agent/src/subagents/runtime/view-state.ts:1-97; ice:packages/coding-agent/src/subagents/v2/extension.ts:354-464; ice:packages/coding-agent/test/ice-agent-view-bridge.test.ts:1-186; ice:packages/coding-agent/test/subagent-footer.test.ts:1-387; ice:packages/coding-agent/test/suite/rpc-subagents.test.ts:224-359

Exported symbols: `IceAgentViewKind` (`ice:packages/coding-agent/src/ice-agent-view-bridge.ts:10`), `IceAgentViewAuthority` (`ice:packages/coding-agent/src/ice-agent-view-bridge.ts:11`), `SUBAGENT_PROFILE_COLORS` (`ice:packages/coding-agent/src/ice-agent-view-bridge.ts:14`), `SubagentProfileColor` (`ice:packages/coding-agent/src/ice-agent-view-bridge.ts:15`), `isSubagentProfileColor` (`ice:packages/coding-agent/src/ice-agent-view-bridge.ts:17`), `ICE_AGENT_VIEW_COLORS` (`ice:packages/coding-agent/src/ice-agent-view-bridge.ts:22`), `IceAgentViewColor` (`ice:packages/coding-agent/src/ice-agent-view-bridge.ts:23`), `isIceAgentViewColor` (`ice:packages/coding-agent/src/ice-agent-view-bridge.ts:25`), `IceAgentViewFinalResult` (`ice:packages/coding-agent/src/ice-agent-view-bridge.ts:29`), `IceAgentViewPresentation` (`ice:packages/coding-agent/src/ice-agent-view-bridge.ts:42`), `IceAgentViewPresentationPatch` (`ice:packages/coding-agent/src/ice-agent-view-bridge.ts:68`), `IceAgentViewInteractionMode` (`ice:packages/coding-agent/src/ice-agent-view-bridge.ts:70`), `IceAgentViewControlState` (`ice:packages/coding-agent/src/ice-agent-view-bridge.ts:71`), `IceAgentViewLiveSessionControl` (`ice:packages/coding-agent/src/ice-agent-view-bridge.ts:78`), `IceAgentViewControlResult` (`ice:packages/coding-agent/src/ice-agent-view-bridge.ts:98`), `IceAgentViewDescriptor` (`ice:packages/coding-agent/src/ice-agent-view-bridge.ts:102`), `IceAgentViewLiveSession` (`ice:packages/coding-agent/src/ice-agent-view-bridge.ts:127`), `IceAgentViewLiveSessionSource` (`ice:packages/coding-agent/src/ice-agent-view-bridge.ts:140`), `AgentViewUiState` (`ice:packages/coding-agent/src/ice-agent-view-bridge.ts:145`), `IceAgentViewSnapshotInput` (`ice:packages/coding-agent/src/ice-agent-view-bridge.ts:152`), `normalizeIceAgentViewPresentation` (`ice:packages/coding-agent/src/ice-agent-view-bridge.ts:383`), `IceBlockingWaitSource` (`ice:packages/coding-agent/src/ice-agent-view-bridge.ts:495`), `IceSubagentOverviewChild` (`ice:packages/coding-agent/src/ice-agent-view-bridge.ts:505`), `IceSubagentApprovalAnswer` (`ice:packages/coding-agent/src/ice-agent-view-bridge.ts:530`), `IceSubagentOverviewApproval` (`ice:packages/coding-agent/src/ice-agent-view-bridge.ts:532`), `IceSubagentOverview` (`ice:packages/coding-agent/src/ice-agent-view-bridge.ts:549`), `IceSubagentTranscript` (`ice:packages/coding-agent/src/ice-agent-view-bridge.ts:563`), `IceSubagentControlAction` (`ice:packages/coding-agent/src/ice-agent-view-bridge.ts:570`), `IceSubagentControlSource` (`ice:packages/coding-agent/src/ice-agent-view-bridge.ts:573`).

Exported symbols: `SubagentLiveSession` (`ice:packages/coding-agent/src/subagents/runtime/live-session.ts:202`), `SubagentLiveSessionRegistration` (`ice:packages/coding-agent/src/subagents/runtime/live-session.ts:215`), `formatSubagentTranscript` (`ice:packages/coding-agent/src/subagents/runtime/live-session.ts:422`).

Exported symbols: `SubagentSupervisorState` (`ice:packages/coding-agent/src/subagents/runtime/view-state.ts:9`), `SubagentSupervisorPhase` (`ice:packages/coding-agent/src/subagents/runtime/view-state.ts:10`), `SubagentRetryStateKind` (`ice:packages/coding-agent/src/subagents/runtime/view-state.ts:11`), `SubagentRetryState` (`ice:packages/coding-agent/src/subagents/runtime/view-state.ts:13`), `SubagentToolActivityOutcome` (`ice:packages/coding-agent/src/subagents/runtime/view-state.ts:21`), `SubagentToolActivityDigest` (`ice:packages/coding-agent/src/subagents/runtime/view-state.ts:23`), `SubagentRepeatedFailureAdvisory` (`ice:packages/coding-agent/src/subagents/runtime/view-state.ts:37`), `SubagentRepeatedActionAdvisory` (`ice:packages/coding-agent/src/subagents/runtime/view-state.ts:43`), `SubagentUsageSnapshot` (`ice:packages/coding-agent/src/subagents/runtime/view-state.ts:48`), `SubagentRuntimeAttention` (`ice:packages/coding-agent/src/subagents/runtime/view-state.ts:56`), `SubagentManagementChildState` (`ice:packages/coding-agent/src/subagents/runtime/view-state.ts:74`), `SubagentCheckInDelivery` (`ice:packages/coding-agent/src/subagents/runtime/view-state.ts:82`), `SubagentCheckInState` (`ice:packages/coding-agent/src/subagents/runtime/view-state.ts:90`).

### 3. What it does

Let users inspect and optionally control a live child while retaining bounded read-only history and preserving parent/runtime ownership.

### 4. How it works

IceAgentViewBridge distinguishes parent, live subagent and historical-subagent descriptors, with mirror and controlled modes. Live entries take precedence over matching history.

Known display IDs may be selected/cycled; switching releases previous control. Taking control requires a live working control object; sending interactive input requires controlled mode. Historical views are always read-only. Parent replacement returns display to parent and releases control, but setParentSession does not itself clear the historical map. Disconnect releases controls/live bindings; it is not a claim that every history/control-source reference is erased. Owner checks are enforced by the supplied runtime source, not by arbitrary descriptor IDs alone.

Historical snapshots clone/redact/deep-freeze at most the last 96 source messages, skipping a message over 16 KiB and stopping at an aggregate256KiB. Sensitive key names and credential-looking strings are redacted; cycles are redacted. Source message indices are remapped into the retained subset. Retain at most 32 historical children with oldest-history eviction and accompanying UI state cleanup. Parent transcript query returns undefined; child transcript retrieval prefers live bounded messages, then the durable runtime control source, then historical snapshots. Observer exceptions are swallowed, unknown display IDs fail softly, overview source errors yield undefined, and subscribers receive updates without being allowed to break the runtime.

Presentation fields are separately bounded: task/summary/diagnostic text16KiB; up to 16 labels at 4096 bytes and 64 evidence paths at 4096; safe message indexes through 1000000; marker lists through 4096; retry attempts 1..64 and delays<=600000 ms; runtime activity tail 12 with bounded tool/path/action/error text, safe elapsed/count/usage values and exit status<=65535. Runtime attention is advisory and lifetimeDeadline is false. Tool transport redacted four numeric assignment source lines during this audit; surrounding guards and tests establish bounded nonnegative usage projection, but the hidden expressions are not independently quoted or reconstructed.

SubagentLiveSessionRegistry stores frozen entries keyed by run ID, normalizes semantic colors/presentation, publishes a frozen list and catches listener failures. Replacing an entry is allowed; an old unregister callback removes it only if its session is still the registered instance. Check-in state updates affect advisory UI attention without changing execution status.

The base control tracks working→awaiting-finalization→final-report-requested→final-report-received→terminal and controlled/steered/pending-input state. Finalization waits until uncontrolled with no pending interactive input; terminal releases waiters. Base followUp rejects while controlled, marks steering and calls session.steer. Interactive steer uses session.prompt with interactive source and streamingBehavior=steer only when currently streaming, and decrements pending state in finally. Active V2 overlays stopRuntime with runtime.stop and followUp with runtime.send(...,"followup"); do not describe the base followUp implementation as the live V2 follow-up path. The formatted text transcript uses a redacted48KiB prefix of the last 96 messages, rendering text/thinking and tool-call labels.

### 5. Interfaces

IceAgentViewBridge parent/live/history sources, requestDisplay/cycle/take-control/send/release, getSubagentOverview/respondToSubagentApproval/getSubagentMessages/control; SubagentLiveSessionRegistry register/list/subscribe/updateCheckInState; createSubagentLiveSessionControl and runtime attention/presentation types.

Exported UI vocabularies: IceAgentViewKind=parent|subagent|historical-subagent; authority=safe|yolo; semantic colors=accent|success|warning|error|muted|dim|text. Profile/view color aliases share this same array. Approval answers=allow-once|allow-session|deny and control actions=stop|resume|delete|send. Runtime attention phases=startup|working|wrapping_up|controlled_wait|finalization; supervisor state=running|terminal; tool activity outcome=running|ok|error|aborted. Retry states=scheduled|recovered|failed. Check-in delivery=armed|due|queued_for_parent|consumed|acknowledged|owner_unavailable. These are projection contracts, and lifetimeDeadline remains literally false.

IceAgentViewControlResult is {accepted:true, mode:mirror|controlled}|{accepted:false, reason}. IceAgentViewPresentationPatch is an alias to the all-optional presentation DTO (delegatedTask, scopeLabels, authority, marker/index fields, protocolReportPending, compacting, finalizationStarted, reportMode, runtimeAttention, retry, finalResult). IceAgentViewDescriptor requires kind, id, label, live, readOnly and permits role/run/task/session, retentionState:reusable|history-only, interactionMode, controlState, authority, color, model, status, cwd, times, presentation, checkIn, messages. AgentViewUiState requires editorDraft/followTranscript and permits scrollOffset/fullscreen. Live-session sources expose list/subscribe; historical snapshots require runId, role, status, finishedAt, messages.

### 6. Dependencies

`SUB-016`, `SUB-020`, `SUB-022`, `SUB-028`

Coding-agent footer, transcript panels and keybinding presentation remain TUI-area consumers. RPC serialization/command wiring requires its actual coordinator ID; this entry owns the source bridge and runtime semantics.

Final dependency classification. Consuming integrations: `RPC-027`, `TUI-027`, `TUI-028`, `TUI-029`, `TUI-031`, `TUI-060`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

rust:crates/ice-cli/src/child_agents.rs:35-206 retains owner job state and destructive delta polling, without a live/historical view registry, bounded immutable transcript snapshot, controlled child input or parked transcript retrieval. rust:crates/ice-harness/src/agent_harness/child.rs:178-300 separately implements native side questions: tool-free one-turn children with a 64 KiB UTF-8 tail of committed user/assistant text and lifetime independent of the parent run. Preserve that native-only capability; it is not a V2 controllable child view.

### 8. Port notes

Target a child-view adapter between ice-cli's runtime and ice-tui's existing component/event system, keeping mutable owner state in the runtime. Implement a bridge over the durable owner registry with explicit live/read-only states, bounded immutable snapshots and revocable input control. Reuse the TUI's existing rendering and keybinding infrastructure without duplicating runtime ownership there. Preserve side-question lifetime separately and decide explicit history clearing policy at parent changes.

### 9. Size and priority

Estimated Rust implementation: **370-590 lines** (estimate, not measured source). **P1** - Users must be able to inspect and steer delegated work without sending input to the wrong session or retaining unbounded sensitive transcript data.

## SUB-037 - Agent Hub roster, child inspector and scoped approval controls

### 1. Title and status

Agent Hub roster, child inspector and scoped approval controls. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/subagents/ui/hub.ts:1-424; ice:packages/coding-agent/src/subagents/v2/extension.ts:1-1048; ice:packages/coding-agent/test/subagents-v2-hub.test.ts:13-343

### 3. What it does

Show active and finished children, capacity, pending approvals and runtime actions in one keyboard-driven owner view.

### 4. How it works

HubSource supplies rows, capacity, approvals, selected-child details, focus/stop/resume/delete/respond and subscriptions. Active rows appear before parked rows, preserving registry order within groups; selection follows the same child ID when it moves groups.

Initial selection prefers the first child awaiting approval. Parked children display as finished with their last outcome and Enter resumes before focusing; a child with no live view leaves the Hub open with a notice.

Approvals are scoped to the selected child. Requests follow child roster order, then oldest request within that child; the next-approval key cycles across this sequence. Allow-once, allow-session and deny target only the selected request; allow-session requires an offered rule. If another child owns the request, the selected child cannot accidentally answer it. The view ignores the boolean response return, so a raced/stale rejection can still yield an Approved-style local notice; runtime acceptance is authoritative.

Up/down move selection, confirm/attach focus, stop/delete call async runtime actions with notices, resume opens a parked child and Escape/hub key closes. Key names are read from the user's keybindings. Runtime changes request render; dispose unsubscribes. Exceptions from async actions are rendered as notices rather than silently closing the view.

At width>=110 render a 55-percent roster beside an inspector; otherwise stack them. Window the roster to 14 child rows around selection with above/below counts. The child name/agent/state columns use bounded visible-width formatting, and counts distinguish running/queued/idle/finished with capacity shown once. Running uses an accented dot, queued an open dot, blocked a warning exclamation, failed/abandoned a failure cross, timed-out/incomplete/budget-exhausted warning, cancelled/interrupted a square and completed a check. Other states use an open circle. Details and approval summaries appear in the selected inspector. The oneRow helper replaces line breaks and applies terminal-width truncation; it does not strip every possible terminal control/ANSI sequence. The 14-row roster window is not a hard cap on total Hub height because inspector detail/request rows may be longer.

### 5. Interfaces

AgentHubView Component render/handleInput/dispose; HubRow, HubSource, statusMark. Source actions stay runtime-owned and approvals use ApprovalAnswer. Ordinary TUI keybinding/theme/width primitives are prerequisites, not new subagent authority. Consumed keybindings: tui.select.cancel/up/down/confirm, app.agents.hub/nextApproval/stop/delete/approve/approveSession/deny and app.subagents.attach.

### 6. Dependencies

`SUB-008`, `SUB-016`, `SUB-020`, `SUB-022`, `SUB-036`, `TUI-001`, `TUI-048`, `TUI-201`, `TUI-202`, `TUI-203`, `TUI-205`, `TUI-210`, `TUI-257`

The coordinator should attach actual TUI keybinding/terminal-width/overlay prerequisites after TUI entries are assigned; no invented cross-area ID is used here.

### 7. ice-rust today

rust:crates/ice-cli/src/child_agents.rs:140-206 provides opaque owner jobs and polling, without the V2 named roster, parked-child state, pending approval inbox or HubSource. This entry concerns the subagent-specific hub contract rather than claiming the native TUI lacks generic components.

### 8. Port notes

Target a child-view adapter between ice-cli's runtime and ice-tui's existing component/event system, keeping mutable owner state in the runtime. Build the Hub over the owner overview and shared TUI primitives. Keep focus/control separate from state queries, preserve ID-based selection across row movement and let the runtime decide whether an approval is still pending. Consider displaying the respond return value to avoid stale success notices.

### 9. Size and priority

Estimated Rust implementation: **240-380 lines** (estimate, not measured source). **P2** - A consolidated child view makes parallel work and pending approvals manageable without changing runtime authority.

## SUB-038 - Bounded Ruflo agent-pack conversion and generated-format incompatibility

### 1. Title and status

Bounded Ruflo agent-pack conversion and generated-format incompatibility. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/ice-agent-packs.ts:1-260; ice:packages/coding-agent/src/ice.ts:1-131; ice:packages/coding-agent/src/subagents/definitions/parse.ts:24-234; ice:packages/coding-agent/src/subagents/definitions/schema.ts:1-164

Exported symbols: `AgentPackImportOptions` (`ice:packages/coding-agent/src/ice-agent-packs.ts:10`), `AgentPackSkip` (`ice:packages/coding-agent/src/ice-agent-packs.ts:14`), `AgentPackImportResult` (`ice:packages/coding-agent/src/ice-agent-packs.ts:21`), `AgentPackToolMapping` (`ice:packages/coding-agent/src/ice-agent-packs.ts:67`), `mapAgentPackTools` (`ice:packages/coding-agent/src/ice-agent-packs.ts:76`), `AGENT_PACK_UNSUPPORTED_FIELD_NAMES` (`ice:packages/coding-agent/src/ice-agent-packs.ts:113`), `unsupportedAgentPackFields` (`ice:packages/coding-agent/src/ice-agent-packs.ts:123`), `formatAgentPackImportResult` (`ice:packages/coding-agent/src/ice-agent-packs.ts:252`).

### 3. What it does

Import a local foreign Markdown agent pack as explicitly untrusted read-only profiles, while documenting that the current generated format is rejected by strict V2 parsing.

### 4. How it works

The pre-startup --import-agent-pack flag accepts one nonempty path in separate or equals form; a resolved value beginning with a dash is not accepted in either form. Success prints the import report and exits 0 even when everything was skipped; an exception exits 1.

Import scans source/.claude/agents recursively in lexical order, skips child symlinks, accepts case-insensitive .md and rejects more than 128 discovered files. The root stat follows a symlink; this is not a promise that every source ancestor is nonsymlink. The target directory may be created before any candidate validates.

Skip README/index files, files over 64 KiB, duplicate content SHA, malformed frontmatter, missing name/description and empty body. Deduplication records the content hash before later parse checks. Slugify the foreign name to lowercase alphanumeric/hyphen, trim to 56 characters, then add a ruflo prefix within 64. Resolve collisions by source-relative path and then an 8-hex hash suffix. Existing exact generated content is already-imported; differing content is a conflict and is not overwritten. The final write uses 0600 after the existence check but not exclusive wx, leaving an external-writer race.

Map Read/read_file to read, Grep/search/search_files to grep, Find/glob/list_files to find, and ls/list to ls. Unknown tools are reported with bounded names; if no known tools remain, use the four read-only tools. Foreign metadata outside name/description/tools/model is reported unsupported, and model is ignored. The generated description uses a 512-code-unit prefix, carries pack/path/content-SHA provenance and frames the body as untrusted source text. A file can appear in both imported and unsupported-metadata reporting; JSON lists are bounded by the 128-file scan.

Source-read defect: nativeMarkdown unconditionally emits ice-unsafe-host-exec: allowed at ice-agent-packs.ts148. The current strict native definition parser rejects that removed key. Therefore a successfully written imported profile can be unusable in active V2 discovery; do not count this converter as a working V2 import path solely because files are created. This is established by composing the two source paths, without running an import or modifying user profiles.

### 5. Interfaces

importRufloAgentPack and --import-agent-pack; report includes imported/skipped paths and reasons, unsupported metadata/tool names, source hashes and collision resolution. Import creates local native-style Markdown; it performs no package/network fetch.

AgentPackImportOptions={targetDir}; AgentPackSkip={path, reason, unsupported?}; AgentPackImportResult={pack:ruflo, sourceDir, targetDir, imported, skipped, conflicts}; AgentPackToolMapping={tools, unsupported}. The exported unsupported-field metadata list is hooks, mcp, permissions, network, sandbox, exec, command, but unsupportedAgentPackFields actually reports all metadata keys outside name/description/tools/model (bounded and sorted), not only that list. formatAgentPackImportResult bounds skipped/conflicts arrays at the scan cap and retains imported paths and target directory.

### 6. Dependencies

`SUB-001`, `SUB-003`

Requires SUB-001 (Strict native Markdown and JSON agent definitions with source identity); SUB-003 (Bounded agent discovery, trust filtering and whole-definition precedence). Requires local filesystem traversal, YAML/frontmatter parsing and exclusive destination creation.

### 7. ice-rust today

rust:crates/ice-cli/src/child_agents.rs:208-286 starts a task/role directly and has no profile conversion/discovery input. The native child harness in rust:crates/ice-harness/src/agent_harness/child.rs:74-172 likewise has no foreign-pack Markdown import.

### 8. Port notes

Target a local import utility in ice-cli using the native definition parser and exclusive filesystem writes. If porting the converter, preserve read-only tool mapping, provenance, caps, explicit conflicts and untrusted body framing, but emit a valid current V2 definition and create destinations exclusively. Add format compatibility validation before reporting imported success. Keep the audited TypeScript incompatibility visible as a source defect, not intended authority behavior.

### 9. Size and priority

Estimated Rust implementation: **150-240 lines** (estimate, not measured source). **P2** - Local pack migration should produce usable definitions without importing unsupported tools or silently overwriting user files.

## SUB-039 - Frozen task contracts and explicit quality/reviewer recommendations

### 1. Title and status

Frozen task contracts and explicit quality/reviewer recommendations. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/subagents/results/task-contract.ts:1-458; ice:packages/coding-agent/src/subagents/v2/runtime.ts:640-685; ice:packages/coding-agent/test/ice-subagent-task-contract.test.ts:1-500

Exported symbols: `SUBAGENT_TASK_CONTRACT_LIMITS` (`ice:packages/coding-agent/src/subagents/results/task-contract.ts:22`), `SubagentTaskContractKind` (`ice:packages/coding-agent/src/subagents/results/task-contract.ts:31`), `SubagentTaskVerificationMode` (`ice:packages/coding-agent/src/subagents/results/task-contract.ts:32`), `SubagentQualityMode` (`ice:packages/coding-agent/src/subagents/results/task-contract.ts:33`), `SubagentTaskContractKnownFact` (`ice:packages/coding-agent/src/subagents/results/task-contract.ts:35`), `SubagentTaskContractVerification` (`ice:packages/coding-agent/src/subagents/results/task-contract.ts:40`), `SubagentTaskContractKnownFactInput` (`ice:packages/coding-agent/src/subagents/results/task-contract.ts:55`), `SubagentTaskContractVerificationInput` (`ice:packages/coding-agent/src/subagents/results/task-contract.ts:60`), `SubagentTaskContractInput` (`ice:packages/coding-agent/src/subagents/results/task-contract.ts:72`), `SubagentTaskContractErrorCode` (`ice:packages/coding-agent/src/subagents/results/task-contract.ts:81`), `DeriveSubagentQualityModeOptions` (`ice:packages/coding-agent/src/subagents/results/task-contract.ts:316`).

### 3. What it does

Represent the delegated objective and declared deliverable/verification expectations without granting tools, scope or model authority.

### 4. How it works

buildSubagentTaskContract takes a separate nonblank objective plus strict optional kind, deliverable, constraints, nonGoals, knownFacts and verification. Kind is explore|research|review|implement|verify, default explore.

Default deliverables respectively request bounded evidence-backed answer, research summary, file-level review, scoped implementation with observed verification or a verification report. Verification is evidence|tests|review|none, default none. All nested objects reject unsupported keys. Text is trimmed and credential-looking substrings redacted. The objective itself has no builder-specific byte maximum; callers and the rendered section impose separate bounds.

Deliverable and each constraint/non-goal are capped at 1024 UTF-8 bytes; constraints/non-goals at 8 each. Known facts are at most 8, each with a nonduplicated ID beginning alphanumeric then alphanumeric/dot/underscore/hyphen up to 64 characters and text<=2048 bytes. Verification commands are at most 4 strings of 512 bytes; they are declarations only and do not execute or enlarge authority. Reject malformed structures, empty trimmed strings, invalid enums and excess lengths/counts with contract_invalid or contract_oversized. Deep-freeze all nested values. Canonical hashing sorts object keys while preserving array order and yields a SHA-256 identity that changes with any contract field.

Quality mode is strict when forced, when acceptance criteria or output schema exist, or when verification mode is review. An explicitly supplied explore kind with verification none/evidence becomes explore; a defaulted kind remains standard, and other combinations are standard. Render a 4096-byte UTF-8-safe section with a truncation marker and explicit no-authority framing; a large objective can consume space before later sections. The reviewer recommendation applies to implement/verify with verification none or matching high-risk terms (security, migration/migrate, refactor, shared/core infrastructure or shared runtime), unless already review verification. It recommends a review_batch-style check but does not launch it.

Active V2 only calls the builder with the task description and defaults. The quality-mode and reviewer-gate helpers have no production caller found under packages/coding-agent/src, and V2 does not register a review_batch tool. Preserve these exported library contracts as dormant capabilities rather than attributing them to V2's live acceptance/budget behavior.

### 5. Interfaces

SubagentTaskContract {kind, objective, deliverable, constraints, nonGoals, knownFacts, verification}; buildSubagentTaskContract, hashSubagentTaskContract, deriveSubagentQualityMode, renderSubagentTaskContractSection, recommendSubagentReviewerGate and SubagentTaskContractError.

SubagentTaskContractInput permits kind, deliverable, constraints, nonGoals, knownFacts, verification; known facts have id/text and verification input permits mode/commands. Normalized verification requires mode. DeriveSubagentQualityModeOptions={hasAcceptanceCriteria, hasOutputSchema, forced?:strict, defaultedKind?}. The defaultedKind flag distinguishes an omitted default explore kind from an explicit explore request.

### 6. Dependencies

`SUB-013`

Requires SUB-013 (Frozen child runtime contract and hash-checked transcript initialization). Pure local strict data parsing, redaction and rendering; no external service.

### 7. ice-rust today

rust:crates/ice-cli/src/child_agents.rs:208-286 validates a task/role and numeric limits and constructs a generic delegation prompt; it has no canonical frozen task-deliverable contract, quality-mode derivation or reviewer recommendation. The native child harness can remain the execution substrate.

### 8. Port notes

Target a native subagent definition/policy/result module composed by ice-cli, with serializable Rust structs and the workspace JSON types; no provider service is required for these local rules. Implement the pure strict contract/hash/render layer independently of permission policy. Wire only the live default builder behavior for V2 parity; expose richer contracts or reviewer automation through an explicit API decision rather than silently enabling dormant legacy guidance.

### 9. Size and priority

Estimated Rust implementation: **150-240 lines** (estimate, not measured source). **P2** - Structured delegation expectations improve result interpretation while retaining a clear separation from executable authority.

## SUB-040 - Bounded instruction-chain manifest and repository orientation helper

### 1. Title and status

Bounded instruction-chain manifest and repository orientation helper. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/subagents/runtime/context-bootstrap.ts:1-603; ice:packages/coding-agent/src/subagents/runtime/prompt.ts:151-199; ice:packages/coding-agent/test/ice-subagent-context-bootstrap.test.ts:1-390

Exported symbols: `SubagentContextSourceType` (`ice:packages/coding-agent/src/subagents/runtime/context-bootstrap.ts:46`), `SubagentContextTrust` (`ice:packages/coding-agent/src/subagents/runtime/context-bootstrap.ts:47`), `SubagentContextManifestEntry` (`ice:packages/coding-agent/src/subagents/runtime/context-bootstrap.ts:49`), `SubagentContextManifestOmission` (`ice:packages/coding-agent/src/subagents/runtime/context-bootstrap.ts:59`), `SubagentContextManifest` (`ice:packages/coding-agent/src/subagents/runtime/context-bootstrap.ts:64`), `SubagentContextInstruction` (`ice:packages/coding-agent/src/subagents/runtime/context-bootstrap.ts:72`), `SubagentContextOrientationPacket` (`ice:packages/coding-agent/src/subagents/runtime/context-bootstrap.ts:80`), `SubagentContextBootstrapInput` (`ice:packages/coding-agent/src/subagents/runtime/context-bootstrap.ts:86`).

### 3. What it does

Produce deterministic provenance for selected repository instructions and a small orientation packet without treating repository text as tool authority.

### 4. How it works

This exported bootstrap library has no active V2 production caller in the audited src inventory. Instruction selection always considers cwd/AGENTS.md, even when roots and targets are empty; the header claim that an empty scope selects nothing is broader than the implementation.

For contained scope roots and target parent directories, walk ancestors back to cwd, deduplicate and sort relative paths. Root instructions have the root reason; shared nonroot candidates choose a deterministic lexical governing-root/target reason. Lexical escapes become omissions.

Instruction reads reject symlink components beneath the trusted cwd, open with O_NOFOLLOW where available, require a regular file and allocate at most per-file limit+1 before validating. Limits are 64 KiB per file,256 KiB selected total and 16 instruction files. Candidates are read before the selected-count/total checks, so the total selected-byte cap does not bound every attempted read combined. Missing files are skipped; over-limit/symlink/escape omissions are recorded. Store relative identifier, sourceType instruction, SHA-256, byte count, reason and untrusted_repo trust, not the instruction body.

Orientation always lists root '.', then contained explicit roots, and target parent/file metadata. It reports regular files/directories in deterministic directories-first order, skips symlink directory entries, caps each directory listing at 40 and reads package.json name only through a 64 KiB bounded file read. The resulting text is capped at 8192 UTF-8 bytes at a line boundary with a marker and includes bytes/SHA. A supplied root path that is lexically contained but itself traverses a symlink can be followed by readdirSync: this orientation branch does not run the instruction branch's symlink-component check. Do not generalize the stronger instruction loader containment to the whole helper.

Only a nonempty roots/targets input adds an orientation repo_map entry to the manifest; root AGENTS.md can still appear for empty scope. Manifest entries sort by identifier; omissions deduplicate identifier/reason, selected bytes sum, and a canonical SHA covers metadata and omissions. Deep-freeze output. Render up to 4096 UTF-8 bytes with an explicit truncation marker, listing identifiers/reasons/bounds and untrusted guidance framing. Because the framing is at the end, oversized earlier entries can truncate it. The prompt helper later rereads selected instruction files and requires matching length/SHA, silently skipping a changed file; it is a separate read from the provenance snapshot.

### 5. Interfaces

SUBAGENT_CONTEXT_BOOTSTRAP_LIMITS; selectInstructionChain, buildRepoOrientationPacket, buildSubagentContextManifest, renderSubagentContextManifestSection; manifest {entries, omitted, totalBytes, hash}. Metadata is provenance, not permission or complete file content.

SubagentContextSourceType=instruction|repo_map and SubagentContextTrust=untrusted_repo|parent_verified. Manifest entries={sourceType, identifier, sha256, bytes, reasonSelected, trust}; omissions={identifier, reason}; manifest={entries, omitted, totalBytes, hash}. Selected instruction metadata={absolutePath, relativePath, bytes, sha256, reason}; orientation packet={text, bytes, sha256}. Bootstrap input={cwd, roots, targets?}. The trust type permits parent_verified, while the repository instruction builder described above records untrusted_repo.

### 6. Dependencies

`SUB-025`, `SUB-039`

Requires SUB-025 (Selected child resources and explicit untrusted context packets); SUB-039 (Frozen task contracts and explicit quality/reviewer recommendations). Requires bounded local filesystem traversal, canonical paths and SHA-256.

### 7. ice-rust today

rust:crates/ice-harness/src/agent_harness/child.rs:74-172 inherits parent system/resources rather than computing a per-child scoped instruction manifest. rust:crates/ice-cli/src/child_agents.rs:208-286 has no selected instruction chain/orientation metadata. Parent resource loading may exist elsewhere but is not this child-scoped helper contract.

### 8. Port notes

Target a native subagent definition/policy/result module composed by ice-cli, with serializable Rust structs and the workspace JSON types; no provider service is required for these local rules. If retaining this library, implement deterministic bounded descriptor reads and manifests; use canonical/symlink-safe handling consistently for orientation roots as an explicit strengthening. Keep dormant status visible, and do not infer authority from untrusted AGENTS.md or manifest metadata.

### 9. Size and priority

Estimated Rust implementation: **230-380 lines** (estimate, not measured source). **P2** - Scoped repository context can reduce repeated exploration, but provenance and containment must be accurate before the helper is activated.

## SUB-041 - Legacy child model routing, captured capability identity and sampling helpers

### 1. Title and status

Legacy child model routing, captured capability identity and sampling helpers. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/subagents/runtime/routing.ts:1-207; ice:packages/coding-agent/src/subagents/runtime/live-session.ts:465-529; ice:packages/coding-agent/test/ice-subagent-routing.test.ts:1-164; ice:packages/coding-agent/src/subagents/v2/runtime-support.ts:1-398

Exported symbols: `IceModelCandidateSkip` (`ice:packages/coding-agent/src/subagents/runtime/routing.ts:32`), `IceModelCandidateRequirements` (`ice:packages/coding-agent/src/subagents/runtime/routing.ts:37`), `IceModelCandidateResult` (`ice:packages/coding-agent/src/subagents/runtime/routing.ts:46`).

Exported symbols: `SubagentThinkingLevel` (`ice:packages/coding-agent/src/subagents/types.ts:41`), `SubagentExecutionOverrideInput` (`ice:packages/coding-agent/src/subagents/types.ts:506`), `SubagentExecutionContract` (`ice:packages/coding-agent/src/subagents/types.ts:516`).

### 3. What it does

Retain explicit model-route fallback and sampling rules as library behavior while distinguishing them from V2's active frozen model resolver.

### 4. How it works

snapshotIceSubagentRoute stores provider, modelId and SHA of capability fields (API, base URL, reasoning/thinking mapping, input modalities, context window and max tokens), without credentials. Primary/fallback references require exact provider/model syntax, no whitespace/control and length<=256.

Candidates deduplicate in order; an explicitly excluded reference is skipped with a bounded reason. Policy denial throws instead of silently rerouting. Missing catalog entry, configured auth or required capabilities becomes a skip; capabilities require positive context/output limits, text input, optional image support and requested minimums/thinking compatibility. The parent fallback is policy/capability checked, but not passed through the candidate auth test; an excluded parent throws.

A captured route is authoritative before newly supplied primary/fallback metadata. Routing must be enabled, the captured model must remain available/authenticated and its capability hash must match, otherwise reject instead of silently rerouting. With no requested/fallback/captured route, return the exact parent identity without catalog/capability work, even when routing is disabled. Opt-in controls selection, not mere parent inheritance.

The sampling helper omits temperature for Anthropic with thinking enabled or a model that explicitly disallows temperature. It emits topP only for azure-openai-responses, openai-completions and openai-responses; unsupported values produce diagnostics. The thinking helper rejects explicitly requested unsupported levels and clamps a default level to supported capability. These helpers do not alter the parent model.

Source caller searches found only definitions/internal helper calls for resolveIceSubagentRoute/Candidates, resolveSubagentSamplingOptions and resolveSubagentThinkingLevel, with no active V2 production caller. V2 has its separate definition/model-list resolver and freezes model metadata; its session factory does not apply the stored temperature/topP. Do not claim these legacy fallback/sampling checks currently run for every V2 child.

### 5. Interfaces

IceSubagentRouteSnapshot {provider, modelId, capabilityHash}; snapshotIceSubagentRoute, resolveIceSubagentCandidates, resolveIceSubagentRoute; resolveSubagentSamplingOptions, resolveSubagentThinkingLevel. Runtime model/auth/policy capabilities are caller-supplied; no auth command or network lookup is performed by these pure selectors.

IceModelCandidateRequirements={thinking?, minimumContextTokens?, minimumOutputTokens?, requiresImages?}; IceModelCandidateSkip={reference, reason}; IceModelCandidateResult={model, selected, skipped}, where selected is the exact chosen reference or parent for the captured fallback. SubagentThinkingLevel has off, minimal, low, medium, high, xhigh, max, ultra. Legacy SubagentExecutionOverrideInput carries model?, hooks?, thinking?, tools?, temperature?, topP?; SubagentExecutionContract requires thinking and tools (possibly undefined), with optional temperature/topP.

### 6. Dependencies

`SUB-013`, `SUB-018`

Requires SUB-013 (Frozen child runtime contract and hash-checked transcript initialization); SUB-018 (Launch preflight, frozen model choice, unique names and staged batch acceptance). Requires the host's current model/auth catalog and policy callbacks; retained helper does not itself establish an active V2 caller.

### 7. ice-rust today

rust:crates/ice-harness/src/agent_harness/child.rs:74-172 snapshots the parent's private model/provider binding, thinking and stream options. This already preserves exact inherited model identity. It has no legacy opt-in primary/fallback route list, captured capability-hash resume validation or these per-provider sampling helpers. Preserve the native inherited private binding and sanitized public context.

### 8. Port notes

Target a native subagent definition/policy/result module composed by ice-cli, with serializable Rust structs and the workspace JSON types; no provider service is required for these local rules. Expose the optional selector as a typed helper with explicit policy and auth inputs if legacy API compatibility is needed. Keep active V2 model selection separate and resolve whether stored sampling fields should become effective as a deliberate behavior change.

### 9. Size and priority

Estimated Rust implementation: **140-230 lines** (estimate, not measured source). **P2** - Model identity and fallback must remain predictable without attributing dormant routing policies to live child execution.

## SUB-042 - Legacy fork prompt identity, report validation and schema-v1 report artifacts

### 1. Title and status

Legacy fork prompt identity, report validation and schema-v1 report artifacts. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/subagents/runtime/prompt.ts:1-437; ice:packages/coding-agent/src/subagents/results/report.ts:542-829; ice:packages/coding-agent/src/subagents/runtime/live-session.ts:431-464; ice:packages/coding-agent/src/subagents/types.ts:1-812

Exported symbols: `SUBAGENT_HANDOFF_MARKER` (`ice:packages/coding-agent/src/subagents/results/report.ts:25`), `INTERACTIVE_FINAL_REPORT_MARKER` (`ice:packages/coding-agent/src/subagents/results/report.ts:26`), `INTERACTIVE_FINAL_REPORT_PROMPT` (`ice:packages/coding-agent/src/subagents/results/report.ts:27`), `INTERACTIVE_PLAIN_FINAL_PROMPT` (`ice:packages/coding-agent/src/subagents/results/report.ts:31`), `SUBAGENT_WRAP_UP_MARKER` (`ice:packages/coding-agent/src/subagents/results/report.ts:35`), `SUBAGENT_WRAP_UP_PROMPT` (`ice:packages/coding-agent/src/subagents/results/report.ts:36`), `SUBAGENT_REPORT_REPAIR_PROMPT` (`ice:packages/coding-agent/src/subagents/results/report.ts:41`), `normalizeReviewFindings` (`ice:packages/coding-agent/src/subagents/results/report.ts:476`), `formatSubagentRequirementSummary` (`ice:packages/coding-agent/src/subagents/results/report.ts:819`).

Exported symbols: `mergeTrustedChildContext` (`ice:packages/coding-agent/src/subagents/runtime/live-session.ts:444`).

Exported symbols: `SubagentProfileSource` (`ice:packages/coding-agent/src/subagents/types.ts:189`), `SubagentProfile` (`ice:packages/coding-agent/src/subagents/types.ts:236`), `ResolvedSubagentProfile` (`ice:packages/coding-agent/src/subagents/types.ts:270`), `SubagentContextMode` (`ice:packages/coding-agent/src/subagents/types.ts:341`), `SUBAGENT_REPORT_ARTIFACT_LIMITS` (`ice:packages/coding-agent/src/subagents/types.ts:360`), `SubagentReportProtocolStatus` (`ice:packages/coding-agent/src/subagents/types.ts:366`), `SubagentReportArtifact` (`ice:packages/coding-agent/src/subagents/types.ts:378`), `SubagentWorkArtifact` (`ice:packages/coding-agent/src/subagents/types.ts:394`), `SubagentAcceptanceEvidenceKind` (`ice:packages/coding-agent/src/subagents/types.ts:414`), `SubagentAcceptanceCriterionInput` (`ice:packages/coding-agent/src/subagents/types.ts:416`), `SubagentAcceptanceCriterion` (`ice:packages/coding-agent/src/subagents/types.ts:426`), `SUBAGENT_ACCEPTANCE_LIMITS` (`ice:packages/coding-agent/src/subagents/types.ts:457`), `SubagentRequirementState` (`ice:packages/coding-agent/src/subagents/types.ts:465`), `SanitizedForkMessage` (`ice:packages/coding-agent/src/subagents/types.ts:475`), `SubagentForkDropped` (`ice:packages/coding-agent/src/subagents/types.ts:482`), `SubagentForkContext` (`ice:packages/coding-agent/src/subagents/types.ts:491`), `SubagentRequest` (`ice:packages/coding-agent/src/subagents/types.ts:523`), `SelfDelegationInput` (`ice:packages/coding-agent/src/subagents/types.ts:554`), `NormalizedSubagentRequest` (`ice:packages/coding-agent/src/subagents/types.ts:565`), `SubagentBatchTask` (`ice:packages/coding-agent/src/subagents/types.ts:640`), `SubagentUsage` (`ice:packages/coding-agent/src/subagents/types.ts:662`), `SubagentResult` (`ice:packages/coding-agent/src/subagents/types.ts:714`), `SubagentVerification` (`ice:packages/coding-agent/src/subagents/types.ts:788`), `SubagentRequirementSummary` (`ice:packages/coding-agent/src/subagents/types.ts:805`).

### 3. What it does

Document the exported legacy prompt/report path precisely, including its weaker evidence checks and different artifact format, without presenting it as active V2 behavior.

### 4. How it works

deriveSubagentPromptCacheKey returns a key only for fork context. Canonical serialization sorts object keys, preserves array order and has explicit handling of undefined.

It hashes captured model route, prompt/profile, scope, execution authority, resources, context packets, fork messages and MCP/adapter metadata; delegated task text is excluded. The key is ice-fork-v1- plus 52 hex characters. This is fork cache identity, not a permission token.

buildSubagentPrompt composes handoff and selected context/resource/provenance/task/scoped-target/report sections, labels inherited repository material untrusted and explains unsafe-host constraints. Selected prompt content is parsed as frontmatter/body, trimmed and redacted; that loader is reused by active V2 resources. Instruction-manifest content is reread from a contained nonsymlink leaf and accepted only on exact byte count/SHA; mismatch silently omits it. No new aggregate cap is imposed by this reread helper. buildUnsafeSubagentSystemPrompt advertises the captured tool allowlist and lack of host sandboxing, but prose is not the enforcement boundary. Fork transcript sanitization occurs upstream.

Legacy verifySubagentResult checks lineage/run/profile/source/status/child identity, partial state and output materialization. A bounded plain result can be verified=true in this legacy type even though no structured evidence was checked; active V2 uses unverified plain output. Legacy structured evidence validates path existence and scope, not observations from successful tools in the current child turn. Required satisfied criteria may accept an explicitly empty evidencePaths array because the generic all-path check is vacuous; V2 requires nonempty observed evidence. Repair/wrap-up/final-report prompt builders are library helpers and do not establish automatic V2 repair behavior.

writeSubagentReportArtifact uses schemaVersion1 with an absolute file path, distinct from V2's opaque schemaVersion2 owned output UUID. Require a nonempty raw report and a run ID of 1..128 alphanumeric/dot/underscore/hyphen characters. Redact then keep a UTF-8 prefix up to 512 KiB. Store under agentDir/artifacts/subagent/<runId>/report.txt or report.json; reject symlink components, require a new private 0700 run directory and an exclusive no-follow regular file, then chmod 0444. This helper does not fsync, refuses an existing run directory and returns undefined on failure, potentially leaving an orphan directory. truncateSubagentOutput is a UTF-8 prefix without a marker. extractAssistantText selects the most recent assistant at/after the supplied index and concatenates text blocks, which can be empty.

Caller searches found no production invocation of the fork key, full legacy prompt, legacy verifier or schema-v1 writer under src. Only the selected-content and several small shared utilities are reused. The active V2 result verifier/artifact store is specified separately in SUB-027/028.

normalizeReviewFindings, the legacy normalized-request helper, accepts absent/empty findings as [], otherwise caps 32 findings and enforces severity/category/claim and 1..16 bounded evidence paths. It resolves evidence against request.cwd then approved roots, canonicalizes the existing path and enforces root containment unless request.allowExternal is true. It returns canonical evidence paths; it does not check current-turn tool observations. mergeTrustedChildContext is separately a base-first file-list merge using canonical path, with resolved-path fallback on canonicalization failure; first identity wins, returned file objects are retained, and no packet-ID normalization occurs there.

### 5. Interfaces

deriveSubagentPromptCacheKey, buildSubagentPrompt, buildAcceptanceCriteriaContract, truncateSubagentOutput, writeSubagentReportArtifact, extractAssistantText; legacy verifySubagentResult and repair/finalization prompt helpers. Schema-v1 absolute-path artifacts must not be accepted as schema-v2 opaque references.

Legacy profile/request DTO supplement: SubagentProfileSource=user|project|self. SubagentProfile requires name, description, systemPrompt, requestedTools, tools, thinkingLevel, timeoutMs, modelPolicy:inherit-parent; it permits tags, hooks, temperature, topP, color, hidden, resources, unsafeHostExec, requestedModel, fallbackModel, adapterIds, mcpTools, selfDelegated, diagnostics. tools is the compatibility alias for requestedTools; source provenance is added by ResolvedSubagentProfile as source, sourcePath, canonicalPath, sourceHash. These types do not replace the strict native ResolvedAgentDefinition schema.

SubagentContextMode=fresh|fork. SanitizedForkMessage={readonly index, role:user|assistant|summary, content, bytes}; SubagentForkDropped counts thinking, toolCalls, toolResults, images, custom, empty; SubagentForkContext={mode, sourceSessionId?, sourceLeafId?, messages, totalBytes, dropped}. SubagentReportProtocolStatus=valid|malformed|missing|truncated|plain. SubagentReportArtifact is schemaVersion1 with id, path, bytes, originalBytes, sha256, contentType:text/plain|application/json, truncated; its 512 KiB cap and 8192-byte inline summary allowance are distinct from schema-v2 output artifacts.

SubagentWorkArtifact is schemaVersion1 with runId, childSessionId?, profile, startedAtMs, finishedAtMs?, lastActivities, observedOutputBytes, touchedPaths, candidateEvidencePaths, reportProtocol:{status, diagnostic?}, requirementClaims?. touchedPaths are runtime-observed write/edit paths; candidateEvidencePaths are unverified report claims/extractions. This snapshot can survive report protocol failure without becoming verified evidence.

Legacy acceptance objects are distinct from V2's string criteria. SubagentAcceptanceCriterionInput={id, requirement, required?, evidence?, dimension?}; the normalized type requires required/evidence. Evidence kind=path|test|behavior|finding|none. SUBAGENT_ACCEPTANCE_LIMITS={maxCriteria:16, maxIdBytes:64, maxRequirementBytes:1024, maxAggregateBytes:16384, maxDimensionBytes:32}. SubagentRequirementState={id, required, dimension?, claim?, verified, note?}; SubagentRequirementSummary={total, required, requiredSatisfied, states, visualAcceptancePending}. formatSubagentRequirementSummary uses verified/required markers, prefers note over an unverified claim for detail, and adds the visual-acceptance-pending sentence only when flagged.

SubagentRequest requires parentSessionId, role, task, scope and permits cwd, context, contextPacket, contextMode, kind, deliverable, constraints, nonGoals, verification, checkInIntervalMs, timeoutMs, execution, resources, acceptanceCriteria, preflight, outputSchema, self. SelfDelegationInput permits instructions, capabilities, inheritSkills, mcp. SubagentBatchTask requires id, role, task, scope and carries the request's optional context/task/verification/execution/resource/acceptance/preflight/schema fields, without a parentSessionId or self field.

NormalizedSubagentRequest replaces raw role/scope/cwd/timeouts/execution/resources/context/packet/mode/check-in/acceptance/preflight/schema fields with host-resolved values. It requires runId, role, agentKind:file|self, contextMode, checkInIntervalMs, profile, scope, cwd, timeoutMs, execution, executionThinkingExplicit, requestedTools (possibly undefined), deniedTools, contextPacket, forkContext, resources, projectTrusted, allowExternal, acceptanceCriteria, preflight, reportMode, taskContract, qualityMode, taskContractHash, routeSource:explicit|tier|parent. Optional retained fields include hookIds, requestedModel, fallbackModel, selectedMcpTools, selectedAdapterIds, mcpAuthorizations, mcpAuthorityStillValid, delegatedTools, inheritedSkills, readOnlyReview, routeSnapshot, resolveStartupFallback, retryModel, modelCandidates, modelCandidateSkips, outputSchema, contextManifest, semanticWorkerTier:mechanical|judgment. Function-valued adapters/recovery seams are in-process DTO members, not serialized durable authority.

Legacy SubagentResult requires runId, parentSessionId, profile, source, status, summary, observedOutputBytes, partial, diagnostics. Optional fields include model, retrySafeStartup, modelCandidateSkips, childSessionId, resumedFromRunId, attention, truncated, usage, recovery, evidence, findings, scopeTargets, workArtifact, output, requirementClaims, requirementStates, payload, observedTurns, hookRecords, reportMode, qualityMode, taskContractHash, contextManifestHash, assurance, reasoningLevel, followUpCount, repeatedFailureCount, reviewerUsed, reviewerOutcome:verified|disagreed, reviewerGateRecommended, semanticWorkerTier, routeSource, evidenceTypes:{path, command, artifact, diff, blocked}. SubagentUsage requires inputTokens, outputTokens, cacheReadTokens, cacheWriteTokens, cost, unlike V2 usage DTOs with optional cache counters.

SubagentVerification={verified, reason, paths, unresolvedClaims, kind?:structured|plain_bounds, structuredVerified?, requirementSummary?}. The optional kind/structuredVerified distinction preserves historical producers that omit them and keeps legacy plain-bounds success separate from structured evidence verification.

Retained prompt exports identify handoff as [ICE VOID SUBAGENT HANDOFF], finalization as 'Your interactive work is complete.', and wrap-up as [ICE VOID SUBAGENT WRAP UP]. Structured finalization requests the bounded JSON report, plain finalization requests ordinary prose/Markdown, wrap-up requests completion from existing state without new tool work, and repair requests one bounded JSON report with tools disabled. All explicitly deny granting new task/permission authority; exporting these strings alone does not schedule the prompts.

### 6. Dependencies

`SUB-025`, `SUB-026`, `SUB-039`, `SUB-040`, `SUB-041`

Requires SUB-025 (Selected child resources and explicit untrusted context packets); SUB-026 (Bounded report envelope and local output-schema validation); SUB-039 (Frozen task contracts and explicit quality/reviewer recommendations); SUB-040 (Bounded instruction-chain manifest and repository orientation helper); SUB-041 (Legacy child model routing, captured capability identity and sampling helpers). Requires deterministic serialization, SHA-256, local report parsing and owned artifact storage.

### 7. ice-rust today

rust:crates/ice-cli/src/child_agents.rs:208-418 uses a generic delegation prompt and bounded in-memory final text, without this fork cache identity, legacy structured report verifier or schema-v1 report directory. rust:crates/ice-harness/src/agent_harness/child.rs:178-300 provides a distinct native side-question text reference; it is not a fork prompt-cache implementation.

### 8. Port notes

Target a native subagent definition/policy/result module composed by ice-cli, with serializable Rust structs and the workspace JSON types; no provider service is required for these local rules. Keep old format compatibility, if required, behind explicit types and APIs. Never let legacy verified=true or existence-only path checks satisfy V2 evidence assurance. Prefer the owner-scoped schema-v2 artifact service for new runtime output, and do not activate the dormant fork prompt path without explicit context/privacy review.

### 9. Size and priority

Estimated Rust implementation: **250-420 lines** (estimate, not measured source). **P2** - Legacy exports can otherwise be mistaken for active verification or accepted as stronger artifact guarantees than they provide.

## SUB-043 - Declarative environment/path/command preflight helper

### 1. Title and status

Declarative environment/path/command preflight helper. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/subagents/runtime/preflight.ts:1-239; ice:packages/coding-agent/src/subagents/types.ts:1-812

Exported symbols: `SubagentPreflightRequirementInput` (`ice:packages/coding-agent/src/subagents/runtime/preflight.ts:16`), `SubagentPreflightState` (`ice:packages/coding-agent/src/subagents/runtime/preflight.ts:30`), `SubagentPreflightCheck` (`ice:packages/coding-agent/src/subagents/runtime/preflight.ts:32`), `SubagentPreflightEvaluation` (`ice:packages/coding-agent/src/subagents/runtime/preflight.ts:41`), `SUBAGENT_PREFLIGHT_LIMITS` (`ice:packages/coding-agent/src/subagents/runtime/preflight.ts:49`), `resolveCommandOnPath` (`ice:packages/coding-agent/src/subagents/runtime/preflight.ts:133`).

### 3. What it does

Check declared local prerequisites without executing commands or revealing environment values, as an exported helper separate from active V2 launch admission.

### 4. How it works

Normalize at most 8 requirements with nonblank values<=512 characters, ID<=64 characters using the allowed alphanumeric/dot/underscore/hyphen form, kind command|path|env-present and required default true unless explicitly false. The normalizer rejects malformed fields and duplicates but does not reject every unknown input key. The result is declarative metadata, not a tool grant.

Command values must be bare executable names using alphanumeric/dot/underscore/plus/hyphen, with no path separator or shell syntax. Search nonempty PATH directories for a regular executable file using X_OK; no command runs, no shell parsing and no Windows PATHEXT expansion are performed. Environment checks use own-property presence rather than truthiness, so a present property with undefined still counts PRESENT; neither response nor summary includes the value. Path checks use lexical containment in approved roots and existence checks that can follow symlinks; they do not prove canonical containment.

Checks return FOUND/UNAVAILABLE for command, PRESENT/ABSENT for env and corresponding path availability or OUT_OF_SCOPE. Required unsuccessful checks block; optional failures do not. Formatting includes redacted, per-item bounded identifiers/values/status/reason and no separate global aggregate byte cap beyond normalized item limits. The full caller search found no active V2 production call to evaluateSubagentPreflight; the V2 launch validation in SUB-018 is a distinct live path.

### 5. Interfaces

normalizeSubagentPreflightRequirements, evaluateSubagentPreflight, formatSubagentPreflight, formatSubagentPreflightFailure; SubagentPreflightRequirement/Input/Check/Evaluation and state enum. The caller supplies cwd/approved roots/environment/PATH.

SubagentPreflightRequirementInput={id, kind:command|path|env-present, value, required?}; SubagentPreflightCheck={id, kind, value, required, state, detail?}; SubagentPreflightEvaluation={readonly checks, blocked, failedRequiredIds, summary}. SUBAGENT_PREFLIGHT_LIMITS names maxRequirements=8, maxIdBytes=64, maxValueBytes=512; the implementation's character-versus-byte checks remain as described above.

### 6. Dependencies

No capability-entry prerequisite. Uses the filesystem/process/environment probe interfaces described in this entry; no remote service.

### 7. ice-rust today

rust:crates/ice-cli/src/child_agents.rs:208-286 validates child task/numeric/tool arguments only and has no this declarative prerequisite schema. rust:crates/ice-harness/src/agent_harness/child.rs:74-172 likewise does not add preflight command/path/environment metadata.

### 8. Port notes

Target a native subagent definition/policy/result module composed by ice-cli, with serializable Rust structs and the workspace JSON types; no provider service is required for these local rules. Implement this small helper only if retaining the exported legacy contract. Keep availability separate from authority, never execute the probed command, and explicitly strengthen symlink containment if used to make security decisions.

### 9. Size and priority

Estimated Rust implementation: **100-170 lines** (estimate, not measured source). **P2** - Optional prerequisite checks should remain deterministic and must not accidentally become a new command-execution or authority path.

## SUB-044 - Shared child activity, hook flushing, lifecycle races and legacy error envelopes

### 1. Title and status

Shared child activity, hook flushing, lifecycle races and legacy error envelopes. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/subagents/runtime/helpers.ts:1-389; ice:packages/coding-agent/src/subagents/runtime/live-session.ts:1-191; ice:packages/coding-agent/src/subagents/types.ts:1-812; ice:packages/coding-agent/test/ice-subagent-command-hooks.test.ts:1-95; ice:packages/coding-agent/test/suite/subagents-v2-hooks.test.ts:1-192

Exported symbols: `requireParentModel` (`ice:packages/coding-agent/src/subagents/runtime/helpers.ts:28`), `extractProgressPath` (`ice:packages/coding-agent/src/subagents/runtime/helpers.ts:34`), `stringToolArgument` (`ice:packages/coding-agent/src/subagents/runtime/helpers.ts:71`), `extractBashExitCode` (`ice:packages/coding-agent/src/subagents/runtime/helpers.ts:147`), `shutdownChildSession` (`ice:packages/coding-agent/src/subagents/runtime/helpers.ts:190`), `SubagentErrorDetails` (`ice:packages/coding-agent/src/subagents/runtime/helpers.ts:198`), `SubagentToolErrorDetails` (`ice:packages/coding-agent/src/subagents/runtime/helpers.ts:254`), `formatManagedIdentifierError` (`ice:packages/coding-agent/src/subagents/runtime/helpers.ts:278`), `formatDurableJobIdentifierError` (`ice:packages/coding-agent/src/subagents/runtime/helpers.ts:286`).

Exported symbols: `PendingSubagentHookObservation` (`ice:packages/coding-agent/src/subagents/runtime/live-session.ts:43`), `SubagentHookRuntime` (`ice:packages/coding-agent/src/subagents/runtime/live-session.ts:48`), `NativeSubagentSessionOptions` (`ice:packages/coding-agent/src/subagents/runtime/live-session.ts:61`), `NativeSubagentSession` (`ice:packages/coding-agent/src/subagents/runtime/live-session.ts:192`).

Exported symbols: `SubagentStatus` (`ice:packages/coding-agent/src/subagents/types.ts:153`), `SubagentFailureCode` (`ice:packages/coding-agent/src/subagents/types.ts:163`), `SUBAGENT_SCOPE_FIELD` (`ice:packages/coding-agent/src/subagents/types.ts:308`), `SUBAGENT_SCOPE_TARGET_FIELD` (`ice:packages/coding-agent/src/subagents/types.ts:309`), `SUBAGENT_SCOPE_ROOT_HINT` (`ice:packages/coding-agent/src/subagents/types.ts:310`), `SUBAGENT_SCOPE_TARGET_HINT` (`ice:packages/coding-agent/src/subagents/types.ts:312`), `SubagentDiagnostic` (`ice:packages/coding-agent/src/subagents/types.ts:670`), `SubagentAttemptNumber` (`ice:packages/coding-agent/src/subagents/types.ts:698`), `SubagentAttemptSummary` (`ice:packages/coding-agent/src/subagents/types.ts:700`), `SubagentRecoveryMetadata` (`ice:packages/coding-agent/src/subagents/types.ts:707`).

### 3. What it does

Preserve the exact shared observation and cleanup utilities while separating legacy error/prompt helpers from live V2 lifecycle behavior.

### 4. How it works

Activity digests pick a path from path, file_path, filePath, root, cwd, display only lexically scoped paths and use redacted compact action text. General activity text has a 192-byte prefix before adding an ellipsis, so the final marker can exceed that nominal prefix cap; grep text uses 96 and final status 256. argsDigest is a 16-hex SHA over JSON serialization, whose input serialization is not itself bounded. Bash exit status prefers a safe integer details value 0..65535; otherwise a trailing numeric status expression can also match a timeout duration, so that fallback is not an independently observed process exit status.

Hook decision dispatch is awaited and uses the configured bounded dispatcher. Observation dispatch retains promises/controllers without delaying the tool path; flush waits up to 2000 ms, aborts remaining controllers and then awaits the pending promises. The final await is not an independent hard deadline if a supplied dispatcher remains uncooperative; the ordinary dispatcher has its own deadline. Disposal and scope changes must prevent observations from being applied to another child/turn. Active V2 tool observations/evidence rules are SUB-015/027 and bounded hook decisions are SUB-011.

abortChildSession races abort against a 1000 ms grace. Shutdown emits the session shutdown event and disposes in finally. awaitSubagentStartup races a startup promise with cancellation/deadline and leaves resource cleanup to its caller; a timeout result alone is not proof that underlying work ended. Context-packet normalization rejects malformed/oversized items and duplicate IDs. The separate mergeTrustedChildContext file-list helper deduplicates canonical paths (falling back to resolved paths) with base files winning over selected files. Active V2's packet authority and limits are SUB-025.

The legacy SubagentError helpers carry the 24-member SubagentFailureCode vocabulary with caller-supplied retryability, bounded hints, optional path/task context and normalized tool-error details. They are not the active V2 three-tool error envelope, whose caught errors are usually ordinary text/details. Do not infer retry behavior or isError flags from this exported legacy type. Preserve exact classes and caller reachability when porting shared utilities, and retain the stronger native cleanup accounting described below.

### 5. Interfaces

displayScopedSubagentPath, buildSubagentToolActivityDigest, invalidSubagentScope, invalidSubagentTarget, withBatchTaskContext, formatSubagentToolError, awaitSubagentStartup, normalizeSubagentContextPacket; dispatchSubagentHookDecision/Observation, flushSubagentHookObservations and child abort/shutdown helpers.

SubagentFailureCode is unknown_profile|untrusted_profile|untrusted_resource|invalid_scope|invalid_request|invalid_resource|capability_denied|model_unavailable|auth_missing|child_startup_failure|child_runtime_failure|child_protocol_failure|timeout|cancellation|output_truncated|malformed_result|report_protocol_failure|preflight_failed|verification_failure|batch_admission_exhausted|provider_option_unsupported|integration_conflict|rollback_conflict|unsafe_parent_workspace. SubagentStatus=created|running|completed|failed|cancelled|timed_out|needs_time|verification_failed. These legacy vocabularies are distinct from ChildInstanceState and ChildTurnOutcome.

SubagentErrorDetails permits readonly field, path, hint, taskId; SubagentToolErrorDetails={error:{code, message, retryable, details?}}. The formatter wraps unknown errors as malformed_result, redacts the message and retains existing details. SubagentDiagnostic={code, message, retryable?}, adding output_artifact_truncated, output_artifact_unavailable, output_artifact_integrity_failure, report_spilled, report_artifact_failure to the failure-code vocabulary. Attempt number is 1|2; attempt summary={attempt, status, failureCode?, observedOutputBytes}; recovery metadata={attemptCount, retried, totalObservedOutputBytes, attempts}.

PendingSubagentHookObservation={readonly promise:Promise<void>, readonly controller:AbortController}; SubagentHookRuntime carries dispatcher, hooks, records, pendingObservations?, ownerSessionId, runId, role, attempt?. NativeSubagentSessionOptions requires request, parentActiveTools and permits noLifetimeTimeout, unsafeHostExec, model, modelRuntime, agentDir, sessionStartEvent, hookRuntime, retrySettings, mcpDispatch, parentMcpTools, mcpToolAccess, beforeTool, shouldStopAfterTurn, reportOnly, policyBox. NativeSubagentSession returns session, profile, tools, prompt, diagnostics?, promptCacheKey?. requireParentModel throws model_unavailable when no current model exists; shutdownChildSession emits session_shutdown(reason=quit) and disposes in finally.

### 6. Dependencies

`SUB-011`, `SUB-015`, `SUB-020`, `SUB-025`

Requires SUB-011 (Bounded hook dispatch, durable intent and fail-closed decision events); SUB-015 (Scoped file tools and guarded V2 child session construction); SUB-020 (Admission, liveness deadlines and abandonment of uncooperative child turns); SUB-025 (Selected child resources and explicit untrusted context packets). Requires the host activity/hook interfaces and cancellation/cleanup primitives; no new service.

### 7. ice-rust today

rust:crates/ice-cli/src/child_agents.rs:288-418 has cancellation/error text and catches child task panics. It closes/aborts with a 2 s cleanup grace but retains the active permit until the execution scope is idle, including uncooperative/background drivers. rust:crates/ice-cli/src/child_agents/tests/security.rs:101-229 specifically covers permit retention beyond grace and panic; these tests were read, not run. Native infrastructure lacks the TS activity/legacy error helper contracts, but its permit lifetime is stronger than releasing admission while detached work remains.

### 8. Port notes

Target the child-agent host in ice-cli and a V2 subagent extension module over ice-harness; reuse Tokio cancellation/lifetime primitives and add typed host methods for the specified owner-scoped operations. Implement bounded redacted observations and exact shared APIs where active callers need them; avoid claiming process exit or completed cancellation from heuristic status/deadline text. Keep legacy envelopes explicitly typed and preserve native scope/permit accounting rather than copying TS's weaker abandoned-work slot release.

### 9. Size and priority

Estimated Rust implementation: **160-270 lines** (estimate, not measured source). **P1** - Observation and cleanup helpers must not overstate success, leak context across owners or weaken native cancellation accounting.

## Exact supporting contracts

## Source-reading and review closure

All 71 assigned source/document files and the 37 relevant TypeScript plus 7 native Rust test bodies were read, covering 121 distinct files. No application, build or test was executed. Credential-like fixture values and four source expressions were redacted by transport; no hidden value was recovered or inferred. Hashed ranges are retained in evidence/subagents-read-evidence.json and the area evidence manifest. Independent coordinator reads and corrections are in evidence/qa-coordinator-subagents.json. The coordinator's final acceptance artifacts record cross-area dependencies, source-drift checks and active-checkout citation validation.

