# Settings, trust, resource discovery and prompt composition

This area specifies the running TypeScript coding-agent configuration layer, its launcher-selected precedence, and the resource loaders that consume it. The agent/harness copies of prompt and skill helpers are separately specified in AGENT-055–062; this file records the newer coding-agent behavior and the actual Rust CLI wiring. Source inspection only; no configuration commands, tests, application processes or user-state reads were executed.

## SET-001 - Distribution identity, agent roots and bundled resource path resolution

### 1. Title and status

Distribution identity, agent roots and bundled resource path resolution. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/config.ts:21-25; ice:packages/coding-agent/src/config.ts:369-605 getPackageDir, asset/path helpers and identity constants; ice:packages/coding-agent/src/utils/paths.ts:1-119 normalizePath/resolvePath/canonicalPath; ice:packages/coding-agent/src/core/legacy-compat/paths.ts:1-7; ice:packages/coding-agent/src/core/legacy-compat/env.ts:1-32; ice:packages/coding-agent/docs/environment-variables.md:1-89.

`markPathIgnoredByCloudSync` — `ice:packages/coding-agent/src/utils/paths.ts:103`.

### 3. What it does

Every settings, resource and session consumer resolves a consistent distribution identity and directory tree, whether ICE runs from TypeScript, a package installation or a Bun binary. Environment overrides and resource assets have explicit path rules.

### 4. How it works

The configuration module derives identity from the packaged manifest and computes resource paths without opening user state. It distinguishes a development/package layout from a Bun binary so callers can find the same logical assets in different distributions.

getPackageDir honors a truthy ICE_PACKAGE_DIR after path normalization; otherwise a Bun binary uses the executable directory and normal execution walks upward to package.json with a module-directory fallback. Asset helpers choose Bun-relative directories or the src-versus-dist package layout for interactive assets, theme files, export HTML, bundled Cognee skills and the observation dashboard. Manifest parsing suppresses a missing package file but propagates other errors. Defaults are PACKAGE_NAME='@zykairotis/ice-coding-agent', APP_NAME='ice', APP_TITLE='ICE', CONFIG_DIR_NAME='.ice', VERSION='0.0.0'; a configured iceConfig.name changes the application name/title and uppercased environment prefixes.

ENV_AGENT_DIR is APP_NAME.toUpperCase()+'_CODING_AGENT_DIR'; ENV_SESSION_DIR uses '_CODING_AGENT_SESSION_DIR'. getAgentDir normalizes a truthy override, otherwise joins homedir(), CONFIG_DIR_NAME and 'agent'. getProjectConfigDir joins cwd and CONFIG_DIR_NAME. Other helpers append models.json, auth.json, settings.json, tools, bin, prompts, sessions, themes and app-debug.log. getShareViewerUrl uses a truthy ICE_SHARE_VIEWER_URL or 'https://pi.dev/session/' and appends '#'+gistId. The current legacy compatibility helpers do not search older pi roots or synthesize older environment aliases.

normalizePath can trim input, normalize selected Unicode spaces and strip a leading '@' when requested; it expands '~' and '~/' (also '~\\' on Windows), decodes file URLs and leaves ordinary relative paths relative. resolvePath supplies a base; canonicalPath tries realpath and falls back to the resolved path. Do not equate these string and filesystem operations or claim all callers perform every optional normalization.

The explicit markPathIgnoredByCloudSync(path) utility sets Darwin attributes com.dropbox.ignored and com.apple.fileprovider.ignore#P with xattr -w <attr> 1 <path>, or Linux user.com.dropbox.ignored with setfattr -n <attr> -v 1 <path>. It calls spawnProcessSync with UTF-8 encoding and ignored stdio, ignores returned status/error records, and has no attributes to invoke on other platforms. This source contract belongs to the side-effecting helper; no directory traversal or private user state was exercised (`ice:packages/coding-agent/src/utils/paths.ts:103-118`).

### 5. Interfaces

Exports include isBunBinary, isBunRuntime, getPackageDir, getThemesDir, getExportTemplateDir, getPackageJsonPath, getReadmePath, getDocsPath, getExamplesPath, getChangelogPath, getInteractiveAssetsDir, getCogneeSkillsDir, getObserveDashboardPath, getBundledInteractiveAssetPath, expandTildePath, getShareViewerUrl, getAgentDir, getProjectConfigDir, getCustomThemesDir, getModelsPath, getAuthPath, getSettingsPath, getToolsDir, getBinDir, getPromptsDir, getSessionsDir, getDebugLogPath. Identity fields are PACKAGE_NAME, APP_NAME, APP_TITLE, CONFIG_DIR_NAME, VERSION, ENV_AGENT_DIR, ENV_SESSION_DIR. No migration or directory creation is implied by a path getter.

markPathIgnoredByCloudSync(path:string):void.

### 6. Dependencies

Settings storage (SET-007), resource loading (SET-021), session identity (SESSION-028/029) and observation paths consume these helpers. External dependencies are Node fs/path/os, package metadata and Bun runtime metadata.

### 7. ice-rust today

rust:crates/ice-cli/src/config.rs:42-108; rust:crates/ice-cli/src/config.rs:123-166 has a real nested agent root and concrete auth/models/settings/trust paths. The active native constants are .icers and ICERS_CODING_AGENT_DIR; a present empty, relative or tilde override is rejected as RelativeOverride instead of following TS truthy/normalizePath behavior. There is no package-manifest-driven application identity or Bun asset layout in this native binary. rust:crates/ice-cli/src/config.rs:168-255 has a separate legacy flat-layout migration; that is not a TypeScript alias search. The active checkout's intentional icers/.icers/ICERS_* identity must be retained during final citation relocation, never changed back to the TypeScript namespace.

### 8. Port notes

Centralize native identity/path getters in ice-cli and enumerate all callers before adding normalization compatibility. Preserve icers state isolation and the native asset strategy; choose and document whether relative/~ overrides are accepted or rejected. Add explicit getters for new observation/resource locations as their areas are ported, and keep migration separate from pure resolution. Future verification should use isolated supplied directories for empty/relative/home/file-URL cases and packaged-asset discovery, never the user's real configuration roots.

### 9. Size and priority

Estimated Rust implementation: **180-320 lines** (estimate, not measured source). **P1** - Consistent identity and paths are prerequisites for settings/resources and prevent accidental use of the TypeScript application's state.

## SET-002 - Install-method detection and safe self-update command selection

### 1. Title and status

Install-method detection and safe self-update command selection. **Status: different-by-design.**

### 2. ice source

ice:packages/coding-agent/src/config.ts:27-367 detectInstallMethod,getSelfUpdateCommand,getSelfUpdateUnavailableInstruction,getUpdateInstruction and root/writability helpers; ice:packages/coding-agent/src/core/settings-manager.ts:1252-1260 getNpmCommand/setNpmCommand.

### 3. What it does

ICE offers an update command appropriate to the package manager and installation that owns the running executable, while withholding automatic updates for unmanaged or unwritable installations.

### 4. How it works

The TypeScript helpers identify the running distribution before constructing argv, and the executable self-update path additionally checks ownership and writability. A printable update instruction is less restrictive than permission to execute an automatic update.

Detection prioritizes a Bun binary, pnpm path markers, yarn markers, Bun runtime/global installation markers, npm/node_modules, then unknown. It compares normalized lowercase package and executable paths. String update targets use the same package name/spec; object targets can distinguish the current name from the install spec. npm uses configured argv and only infers a prefix when no override is supplied; scoped package node_modules structure and Unix lib prefixes are considered, while ambiguous Windows layouts are not inferred. pnpm asks for its global root and has a path fallback, with PNPM_HOME or the inferred bin directory. Yarn and Bun use their own global roots; realpath and lexical absolute candidates are both compared, with case folding on Windows and path-separator boundaries.

Commands disable install scripts. pnpm adds --config.minimumReleaseAge=0, Bun adds --minimum-release-age=0 and npm adds --min-release-age=0; yarn uses global add. When the installed package name is changing, the command plan includes removal of the old package. Bun-binary and unknown methods return no package-manager self-update command. The managed-install check verifies the running package is within a discovered global root; the writable check tests both package directory and parent. getSelfUpdateCommand requires both, while getUpdateInstruction may still display an available manual command. Metadata commands use synchronous spawn helpers and treat unsuccessful/empty output according to the caller's requireSuccess policy.

### 5. Interfaces

InstallMethod='bun-binary'|'npm'|'pnpm'|'yarn'|'bun'|'unknown'; SelfUpdatePackageTarget=string|{packageName:string;installSpec?:string}; SelfUpdateCommand contains executable argv and optional prior steps as defined at ice:packages/coding-agent/src/config.ts:33-41. Public getters return a command or explanatory instruction. Stored npmCommand is an argv array, copied on get/set; it is not a shell command string.

### 6. Dependencies

`SET-001`, `SET-007`, `SET-009`

CLI packaging owns actual update execution, lifecycle and user-facing commands. This entry compares selection policy and stored npm argv. External dependencies are npm/pnpm/yarn/Bun, synchronous child-process metadata queries, fs access/realpath and package manifests; none were invoked as application work during this audit.

### 7. ice-rust today

rust:crates/ice-cli/src/updates.rs:119-181 explicitly implements native-only update discovery for the Rust binary and installed cdylib extensions; it does not shell out to Node package managers. Its own bounded HTTP client, update cache and native package registry are substantial counterparts to the outcome, not absent update support. The TS install-manager argv and npmCommand field are not appropriate defaults for a Cargo/prebuilt Rust distribution. Broader native update execution and rebrand package ownership remain cli-packaging findings.

### 8. Port notes

Retain native update backends and the icers identity. Apply the same policy boundaries to native installation detection: show a truthful manual instruction, check that an automatic updater owns the running installation, and reject unsafe replacement targets before launching it. Only implement npmCommand if deliberately supporting Node-managed compatibility packages. Expose update-plan metadata for review and test detection using synthetic layouts rather than invoking package installs.

### 9. Size and priority

Estimated Rust implementation: **80-180 lines** (estimate, not measured source). **P2** - The native distribution needs equivalent ownership checks and useful instructions, but reproducing TypeScript package-manager machinery is not required for native updates.

## SET-003 - Project trust resolution before loading executable resources

### 1. Title and status

Project trust resolution before loading executable resources. **Status: partial.**

### 2. ice source

`ice:packages/coding-agent/src/core/project-trust.ts:12-96` (`resolveProjectTrusted`, prompt selection); `ice:packages/coding-agent/src/core/trust-manager.ts:29-38` and `:178-207` (`hasTrustRequiringProjectResources`); `ice:packages/coding-agent/src/cli/project-trust.ts:1-62` (`createProjectTrustContext`); `ice:packages/coding-agent/src/core/settings-manager.ts:1231-1239` (global-only fallback); `ice:packages/coding-agent/test/trust-manager.test.ts:39-65`; `ice:packages/coding-agent/docs/security.md:1-61`; `ice:packages/coding-agent/docs/settings.md:14-24`.

Named export: `AppMode` (`ice:packages/coding-agent/src/core/project-trust.ts:12`).

Named export: `DefaultProjectTrust` (`ice:packages/coding-agent/src/core/settings-manager.ts:80`).

`ice:packages/coding-agent/src/core/settings-manager.ts:80` (`DefaultProjectTrust`).

### 3. What it does

A project must obtain an explicit or policy-derived trust decision before its configuration, packages, extensions and other executable instruction resources become active. Interactive users can persist a folder or parent decision, or make a decision for the current session.

### 4. How it works

Trust is a resource-loading decision computed before normal project resources are loaded. It gives explicit invocation overrides priority, and otherwise combines resource detection, extension decisions, saved ancestor decisions and the user's global fallback policy.

`resolveProjectTrusted` first returns a defined `trustOverride`, including false. Next it checks for any of `settings.json`, `agents`, `extensions`, `skills`, `prompts`, `themes`, `SYSTEM.md`, `APPEND_SYSTEM.md` under the canonical current directory's project configuration root. `existsSync` is sufficient; the detector does not require the resource to be a regular file or directory. A bare `.ice` directory is insufficient. It additionally walks cwd through the filesystem root looking for `.agents/skills`, excluding the canonical user-home `.agents/skills` even when cwd is home. Home comes from truthy `HOME` or `homedir()`. With no such resources the function returns true before consulting saved denials or `defaultProjectTrust`.

When extensions are provided, it emits `{type:"project_trust",cwd}` to the pre-trust extension set. Errors are reported through `onExtensionError` as extension-path-qualified messages. A returned result ends resolution: only `trusted === "yes"` grants trust, and only `remember === true` persists the boolean at cwd. EXT-034 specifies hook dispatch and bootstrap ownership. If no extension answers, the nearest saved ancestor decision from SET-004 wins. Otherwise global `defaultProjectTrust` is `always` → true, `never` → false, or `ask` → continue. The SettingsManager getter recognizes only always/never, defaulting everything else to ask.

For ask, `hasUI === false` returns false. Otherwise the selector receives the folder path and a warning that trust permits settings/resource loading, package installation and project-extension execution. Options, in order, are `Trust`, `Trust parent folder (<canonical parent>)` when not at a filesystem root, `Trust (this session only)`, `Do not trust`, and `Do not trust (this session only)`. Selection is matched by exact label; cancellation or any unmatched result returns false. Persisted trust writes cwd=true; parent trust writes parent=true and removes cwd's override; denial writes cwd=false; session-only options have no updates. Persistence errors are not silently converted into approval.

The CLI context labels interactive mode `tui`, with print/json/rpc retained. UI select/confirm/input adapters work only with both hasUI and interactive mode. Confirmation is a Yes/No selection and cancellation is false. Notifications in noninteractive modes use stderr with severity coloring. Context files such as AGENTS.md/CLAUDE.md are standing instructions loaded independently of this project resource gate; this gate is not an operating-system sandbox.

### 5. Interfaces

`ResolveProjectTrustedOptions {cwd:string; trustStore:ProjectTrustStore; trustOverride?:boolean; defaultProjectTrust?:"ask"|"always"|"never"; extensionsResult?:LoadExtensionsResult; projectTrustContext:ProjectTrustContext; onExtensionError?:(message:string)=>void}` → `Promise<boolean>`.

Hook input `{type:"project_trust",cwd:string}`; result trust string and optional `remember` are interpreted with strict equality as above. Setting `defaultProjectTrust` is global-only, default `"ask"`. CLI `--approve`/`-a` and `--no-approve`/`-na` select per-invocation trust overrides; `/trust` persists future-session decisions (the interactive command behavior belongs to TUI). Resource paths and trust file are specified in SET-001/004. No network service is needed for the decision itself.

### 6. Dependencies

`SET-004`

EXT-034 supplies the pre-trust extension hook and loader integration. Resource consumers must take the resolved boolean explicitly. External dependencies: filesystem, proper-lockfile via SET-004, interactive selector where available.

### 7. ice-rust today

`rust:crates/ice-cli/src/session.rs:1847-1857` is the actual CLI resolver: an explicit `args.trust_override` wins, then `config::project_trust_decision(cwd).ok().flatten().unwrap_or(true)`. Unknown decisions and read/parse errors therefore grant trust. `rust:crates/ice-cli/src/config.rs:449-500` reads a flat nullable-boolean map but performs exact canonical-path lookup, not nearest-ancestor inheritance. The runtime has no resource-absence detector, global ask/always/never fallback or interactive trust prompt at this point. Some downstream resource loaders do honor a false decision, so trust is partial, not entirely absent. `rust:crates/ice-cli/src/trust.rs:1-357` defines a separate richer store and home-path heuristic, but inspected production callers use the config/session path, not that class. Its tests do not establish runtime trust behavior. EXT-034 records the unconnected native project-trust hook.

### 8. Port notes

Implement the resolver in the CLI configuration/resource orchestration layer and pass its result through startup, reload, config/package commands and session switches. Keep the native plugin decision hook separate from UI prompting so headless modes have the same explicit denial fallback. Preserve invocation overrides and the no-sensitive-resource fast path, and distinguish malformed trust storage from an absent decision. Port the exact parent/session option effects through SET-004; do not replace them with a home-prefix heuristic. Use explicit policy types rather than conflating missing, false and parse failure.

### 9. Size and priority

Estimated Rust implementation: **280-480 lines** (estimate, not measured source). **P0** - Unknown projects currently load by default, changing the authority boundary for project-supplied code and configuration.

## SET-004 - Locked trust store with canonical ancestor inheritance and grouped updates

### 1. Title and status

Locked trust store with canonical ancestor inheritance and grouped updates. **Status: partial.**

### 2. ice source

`ice:packages/coding-agent/src/core/trust-manager.ts:8-27`, `:40-175`, `:209-244` (`ProjectTrustStore`, nearest lookup and locking); `ice:packages/coding-agent/src/utils/paths.ts:22-34`, `:57-85`; `ice:packages/coding-agent/test/trust-manager.test.ts:24-37`.

Named export: `getProjectTrustParentPath` (`ice:packages/coding-agent/src/core/trust-manager.ts:60`).

`ice:packages/coding-agent/src/core/trust-manager.ts:60-95` (`getProjectTrustParentPath`, parent option construction).

### 3. What it does

Trust saved for a parent applies to descendants unless a closer folder overrides it. Saving a parent and clearing a child is one protected read-modify-write operation, and independent processes see current decisions.

### 4. How it works

The trust store is a flat JSON object keyed by canonical absolute directory paths. Every lookup and update obtains the same filesystem lock and rereads the file, so a long-lived object does not retain stale decisions.

The file is `<resolvedAgentDir>/trust.json`. `normalizeCwd` resolves relative paths and supported home/file-URL input, then follows symlinks with `realpathSync`, falling back to the resolved spelling if canonicalization fails. Lookup checks cwd, then each parent up to the root; only booleans terminate the walk, while null and missing keys continue. `getEntry` returns `{path,decision}` for the actual matching ancestor; `get` returns its boolean or null.

Missing file means `{}`. JSON parse/read failures throw a path-qualified error. Top-level null, arrays and nonobjects are rejected; each value must be true, false or null. `setMany` locks once, reads once, normalizes each requested path and applies updates in supplied order; null deletes a key instead of persisting null. The writer sorts keys, serializes with two-space indentation and a final newline, creates the parent directory, and writes the file directly. The TS writer is not an atomic-rename/fsync implementation.

The lock is on the parent directory with `realpath:false` and explicit lock path `<trust.json>.lock`. It creates the parent even for a lookup. Acquisition tries at most 10 times; only ELOCKED retries, with a synchronous Date.now-based busy wait of 20 ms between attempts. Other errors and the tenth lock failure propagate. All successful acquisitions release in finally, including read, parse and write failures.

`getProjectTrustParentPath(cwd)` canonicalizes with the same `normalizeCwd`, obtains `dirname`, and returns `undefined` when the normalized path is already its own parent. The parent-trust option is offered only for a returned parent; its grouped update sets that parent true and removes the current child decision. Session-only options use empty updates and do not persist a decision.

### 5. Interfaces

`ProjectTrustDecision = boolean | null`; `ProjectTrustStoreEntry {path:string; decision:boolean}`; `ProjectTrustUpdate {path:string; decision:boolean|null}`. Methods `get(cwd)`, `getEntry(cwd)`, `set(cwd,decision)`, `setMany(updates)` are synchronous. JSON example: `{"/work":true,"/work/untrusted":false}`. Explicit nulls can be read but API null updates remove the key. No environment aliases or alternate trust schema are read.

### 6. Dependencies

Filesystem and proper-lockfile. SET-003 consumes the result; path normalization follows SET-001 and the shared environment principles in AGENT-038.

### 7. ice-rust today

`rust:crates/ice-cli/src/config.rs:449-500` uses `BTreeMap<String,Option<bool>>`, reads missing files as empty, and supports set/remove. It canonicalizes existing paths but falls back to cwd's supplied spelling, performs exact lookup only, and reads/modifies/writes without an interprocess lock. Thus two concurrent writes can lose one decision even though the final write is atomic. `rust:crates/ice-cli/src/config.rs:981-1047` creates owner-only directories/files on Unix, writes a unique sibling temporary file, syncs it, renames it and removes staging files on write/rename failure. These are useful Rust behaviors to retain. The separate richer `rust:crates/ice-cli/src/trust.rs:1-357` format is not the flat TS trust contract and is not the resolver's current production store.

### 8. Port notes

Extend the flat config store rather than silently switching users to the unused richer schema. Add a process-safe lock around both read and grouped read-modify-write, canonical absolute fallback, nearest-ancestor lookup and getEntry provenance. Retain Rust's atomic staging/sync/owner permissions while adding concurrency control. Keep removal distinct from a negative trust decision. An fs2-style file lock or equivalent cross-platform advisory lock is appropriate if every writer cooperates; lock-path and contention semantics must be documented for compatibility.

### 9. Size and priority

Estimated Rust implementation: **170-280 lines** (estimate, not measured source). **P0** - Trust inheritance and durable grouped decisions must be correct before project-resource gating can match TypeScript.

## SET-005 - Configuration environment templates, introspection and strict header resolution

### 1. Title and status

Configuration environment templates, introspection and strict header resolution. **Status: partial.**

### 2. ice source

`ice:packages/coding-agent/src/core/resolve-config-value.ts:1-156`, `:201-289` (template parsing, environment introspection, resolution and headers); `ice:packages/coding-agent/src/core/legacy-compat/env.ts:1-15`; `ice:packages/coding-agent/test/resolve-config-value.test.ts:27-49`, `:84-93`.

### 3. What it does

API-key and header settings can combine literal text with environment references. Callers can inspect missing variable names and choose permissive omission or an explicit failure when a value cannot be resolved.

### 4. How it works

Configuration strings are parsed as environment templates unless their first character is an exclamation mark, which selects the command path in SET-006. Environment values are resolved afresh on each call and a missing reference makes the entire template unavailable.

The environment-name grammar is ASCII `[A-Za-z_][A-Za-z0-9_]*`. `$NAME` consumes the longest matching name prefix; `${NAME}` requires the complete braced name to match. Invalid braced names remain literal, a missing closing brace leaves the dollar literal and resumes scanning, and unrelated dollar characters remain literal. `$$` emits one literal dollar; `$!` emits one literal exclamation mark. Adjacent literal spans are combined. Replacement values are appended, not recursively parsed.

Lookup uses an own, non-undefined property of the optional scoped environment first, then an own, non-undefined ambient process property. Empty strings are valid resolved values, not missing variables. Inherited properties are ignored. No old product-name aliases are synthesized. `getConfigValueEnvVarName` returns a name only when the entire parsed template is exactly one environment part; `getConfigValueEnvVarNames` returns unique referenced names in first-occurrence order; `getMissingConfigValueEnvVarNames` filters those with undefined lookup. A command has no environment-template names, and `isConfigValueConfigured` checks missing references rather than executing commands.

The permissive `resolveHeaders` uses cached config resolution, omits every falsy result (including an empty string), and returns undefined for absent headers or an empty resolved object. The strict `resolveHeadersOrThrow` uses uncached resolution, retains empty strings, and throws a header-key-qualified error for an unresolved value. `resolveConfigValueOrThrow` also resolves uncached and reports a command failure, a single missing environment name, multiple names, or a generic unresolved-value error according to the parsed input.

### 5. Interfaces

Template forms `$NAME`, `${NAME}`, `$$`, `$!`; literal and mixed strings are supported. `resolveConfigValue(config,env?)` and `resolveConfigValueUncached(config,env?)` → `string|undefined`; `resolveConfigValueOrThrow(config,description,env?)` → string or Error. Header input/output is `Record<string,string>|undefined`. Exported inspection helpers do not run a shell. Empty values are accepted by template resolution and strict headers, but omitted by permissive headers. `resolveHeaders(headers:Record<string,string>|undefined,env?:Record<string,string>):Record<string,string>|undefined`; `resolveHeadersOrThrow(headers,description:string,env?):Record<string,string>|undefined`. Empty input/output header maps return undefined.

### 6. Dependencies

SET-006 owns leading-! execution. Provider/auth entries own the call sites that choose strict versus permissive resolution. External service: none; process/scoped environment only.

### 7. ice-rust today

`rust:crates/ice-cli/src/config.rs:583-750` has the same core ASCII template grammar, literal escapes, overlay-first environment lookup and whole-value failure on an unset name, with UTF-8-safe char boundaries. Existing source tests at `rust:crates/ice-cli/src/config.rs:1648-1698` cover interpolation, overlays, missing references and escapes. This core is present. Differences: no public matching environment-introspection/OrThrow API was found in the inspected config layer; `resolve_headers` at `:606-620` always resolves uncached, silently drops None, retains empty strings, and returns a BTreeMap even when empty. It therefore matches neither complete TypeScript header contract. Do not describe the entire environment-template engine as missing.

### 8. Port notes

Retain the Rust parser and add a typed parsed-template/introspection layer plus explicit strict/permissive header adapters. Preserve template resolution separately from provider policy. A Result-based strict API should carry which key or environment reference failed without exposing resolved credentials. Compare strings using the stated ASCII identifier rules and avoid shell expansion of ordinary templates.

### 9. Size and priority

Estimated Rust implementation: **150-260 lines** (estimate, not measured source). **P1** - Silent omission and different cache/empty-value semantics can change authentication and rotating request headers.

## SET-006 - Bounded command-backed configuration values and cached failures

### 1. Title and status

Bounded command-backed configuration values and cached failures. **Status: partial.**

### 2. ice source

`ice:packages/coding-agent/src/core/resolve-config-value.ts:126-246`, `:285-288` (command execution/cache and public adapters); `ice:packages/coding-agent/test/resolve-config-value.test.ts:51-82`, `:95-120`.

Named export: `isCommandConfigValue` (`ice:packages/coding-agent/src/core/resolve-config-value.ts:131`).

### 3. What it does

A setting beginning with `!` obtains its value by running a local command, supporting credential helpers. Successful and failed command results may be cached for the process, while callers that need a fresh token can bypass that cache.

### 4. How it works

Only a leading exclamation mark selects command execution, and the remainder is the exact command string. The normal resolver memoizes its trimmed, nonempty stdout or undefined result under the original string including the leading exclamation mark.

On non-Windows platforms the TypeScript helper uses `execSync` with UTF-8 output, a 10000 ms timeout and ignored stdin/stderr with captured stdout. A thrown spawn, timeout or nonzero-status failure resolves undefined. On Windows it first obtains the configured shell with `getShellConfig`; stdin-transport shells receive the command on stdin, while ordinary shells receive configured arguments followed by the command. This path uses spawnSync with `shell:false`, `windowsHide:true`, UTF-8 decoding and the same 10000 ms timeout. A missing executable (ENOENT), shell-discovery exception or setup exception permits fallback to the execSync path. A shell that actually ran and returned failure does not trigger a second execution. Successful stdout is trimmed; empty output resolves undefined.

The command cache stores failures as well as successes and does not expire. `clearConfigValueCache` clears all entries. Uncached and OrThrow adapters always execute again. Scoped credential environment is used by ordinary templates but is not injected as an execution environment into these command calls; the shell inherits the real process environment. The source tests exercise output trimming, multi-line values, successful and failed caching, explicit invalidation, uncached execution and Windows stdin transport. They were read, not run.

### 5. Interfaces

Input syntax `!<command>` with no preliminary trimming. Return `string|undefined`, or the SET-005 strict error. Timeout 10000 ms. Cache key includes `!`; lifetime is the process or explicit clear. Shell discovery is shared with the shell platform layer; there is no network request in this helper unless the user's configured command performs one.

### 6. Dependencies

`SET-005`

Platform shell/child-process execution, plus provider/auth callers selecting cached or uncached resolution. External binaries are the selected shell and the user-specified command.

### 7. ice-rust today

`rust:crates/ice-cli/src/config.rs:560-567`, `:755-806` provides a Mutex/OnceLock command-result cache and uncached execution. It runs `sh -c` on Unix and `cmd /C` with CREATE_NO_WINDOW on Windows, captures stdout, ignores stdin/stderr, checks success and trims output. The comment at `:752-754` promises a 10-second timeout, but `spawn_shell_command` at `:767-790` calls blocking `Command::output()` with no timeout or cancellation. A never-ending credential command therefore never returns from this resolver. This is a source-confirmed Rust defect, not a reproduced hang. The cache lock is released while execution runs, so simultaneous cache misses may execute the command more than once; TypeScript's synchronous event-loop call does not overlap that way. There is no configured Windows-shell/stdin-transport path. Existing Rust tests `:1701-1709` cover a short successful command and nonzero exit, not a deadline.

### 8. Port notes

Implement bounded child execution in the CLI config resolver, with concurrent stdout draining, deadline termination and cleanup, then preserve successful/failed caching. Avoid holding a global mutex through an arbitrary command; use per-key in-flight coordination if single execution is required across threads. Support the configured Windows shell/transport when applicable. Keep this helper distinct from model-facing background shells and from shell-command prefix/session metadata injection. Record the missing deadline in the Rust-defect section with source-only verification.

### 9. Size and priority

Estimated Rust implementation: **140-240 lines** (estimate, not measured source). **P1** - An unbounded credential helper can block startup or provider resolution indefinitely.

## SET-007 - Settings storage abstraction, lock discipline and parse-failure preservation

### 1. Title and status

Settings storage abstraction, lock discipline and parse-failure preservation. **Status: partial.**

### 2. ice source

`ice:packages/coding-agent/src/core/settings-manager.ts:229-330`, `:422-493`, `:795-860` (`SettingsStorage`, `FileSettingsStorage`, `InMemorySettingsStorage`, load/error paths); `ice:packages/coding-agent/src/core/legacy-compat/identity.ts:5-9`. `ice:packages/coding-agent/test/settings-manager.test.ts:275-387`; `ice:packages/coding-agent/test/suite/regressions/3616-settings-inmemory-reload.test.ts:24-85`.

### 3. What it does

Settings can be backed by files or an in-memory store without changing the manager API. Reads avoid creating a project settings directory, and an invalid file does not get overwritten by an unrelated preference change.

### 4. How it works

The manager reads separate global and project JSON documents through a synchronous lock callback. It records load failures per scope and retains in-memory usability while protecting the invalid file from automatic writes.

`FileSettingsStorage` resolves cwd and agentDir and uses `<agentDir>/settings.json` and `<cwd>/.ice/settings.json`. `withLock(scope,fn)` checks existence. For an existing file it acquires a proper-lockfile lock with `realpath:false`, reads UTF-8, and calls fn with current text; for a missing file it calls fn(undefined) without creating or locking anything first. Undefined callback output means no write. If a string is returned it creates the directory when necessary, obtains a lock if none is held, writes directly, and releases in finally. A first-file write therefore computes the callback before acquiring its write lock; the audit does not claim that initial creation is a fully serialized read-modify-write. Lock acquisition retries ELOCKED at most 10 times with 20 ms synchronous waits; other/final failures propagate.

The in-memory implementation stores one optional string per scope and applies the same undefined-means-no-write callback contract without filesystem I/O. `inMemory(initial)` structured-clones and migrates initial settings, seeds the global store, then uses the ordinary fromStorage path; later reload can recover that stored state.

Loading an untrusted project returns an empty object before touching project storage. Missing or empty text means an empty object. Otherwise JSON.parse and migration require a non-null non-array object; comments are not accepted by this TS settings reader. Each scope is loaded independently: tryLoad returns an empty object plus the error on first-load failure. Error diagnostics retain the failing scope. `getLoadErrors` returns a frozen non-consuming list of active load errors; `drainErrors` consumes the accumulated diagnostic queue. Save updates effective memory and notifies listeners, but does not schedule a write for a scope with an unresolved initial/load error.

### 5. Interfaces

`SettingsScope = "global"|"project"`; `SettingsStorage.withLock(scope,fn:(current:string|undefined)=>string|undefined):void`; `SettingsError {scope,error:Error}`. Factories `create(cwd,agentDir?,options?)`, `fromStorage(storage,options?)`, `inMemory(settings?,options?)`. Files are two-space JSON on writes, with unknown keys retained by the changed-field merge in SET-009. No automatic file watcher is installed.

### 6. Dependencies

Filesystem, proper-lockfile, JSON parsing and structuredClone. SET-008 owns effective precedence; SET-009 owns queued changed-field persistence. SET-003 controls whether project storage may be read or written.

### 7. ice-rust today

`rust:crates/ice-cli/src/settings.rs:357-447` reads global and current-directory project settings into a serde struct. Global missing files default cleanly; the convenience project loader at `:382-387` discards load errors, while `load_active_project_settings` retains them. `:468-490` accepts strict JSON or //-comment-stripped JSON, unlike TS. `:493-667` rereads raw JSON on save, rejects malformed/nonobject files and preserves unknown keys while replacing its modeled whitelist. It has no SettingsStorage callback abstraction, in-memory manager with reload semantics, per-scope diagnostic queue or matching lock/retry contract. Atomic staging is already present in `rust:crates/ice-cli/src/config.rs:998-1038` and should be retained.

### 8. Port notes

Introduce a configuration store trait with explicit global/project scopes and transactional read-modify-write operations, plus an in-memory implementation for embedded sessions. Preserve Rust's atomic writer, expose parse/read errors instead of flattening them into absence, and decide/document whether comment-tolerant input remains a Rust compatibility extension. Keep load-error protection independent from whether in-memory settings may change. Do not port the TS first-create race as a requirement.

### 9. Size and priority

Estimated Rust implementation: **260-420 lines** (estimate, not measured source). **P1** - Correct settings persistence and trustworthy diagnostics are prerequisites for many user-visible parity features.

## SET-008 - Scoped deep precedence, launcher global-first policy and change notifications

### 1. Title and status

Scoped deep precedence, launcher global-first policy and change notifications. **Status: partial.**

### 2. ice source

`ice:packages/coding-agent/src/core/settings-manager.ts:187-213`, `:332-420`, `:632-696`; `ice:packages/coding-agent/src/ice.ts:99-102`; `ice:packages/coding-agent/src/main.ts:523-549`, `:630-634`, `:708-711`; `ice:packages/coding-agent/test/ice-resource-precedence.test.ts:33-82`; `ice:packages/coding-agent/docs/settings.md:5-10`, `:350-370`. `ice:packages/coding-agent/test/settings-manager.test.ts:191-230`, `:291-343`; `ice:packages/coding-agent/test/suite/regressions/7572-provider-retry-settings-merge.test.ts:4-34`.

### 3. What it does

Ordinary settings are merged recursively across global and trusted project scopes. The ICE launcher explicitly selects global-first preferences, while the generic coding-agent defaults to project-first; subscribers receive coalesced in-memory invalidation notifications.

### 4. How it works

The manager keeps global, project and effective settings separately, including the provenance of explicitly configured leaves. Its immutable globalFirst option chooses which stored scope wins ordinary preference collisions and remains distinct from permission-denial composition.

A mergeable value is any non-null non-array object. Merge starts with a shallow base copy, iterates own override keys, skips undefined overrides, recursively merges only when both values are mergeable objects, and otherwise replaces the value—including arrays, nulls and false. Generic creation defaults projectTrusted=true and globalFirst=false, yielding global then project. The `ice.ts` launcher passes `globalFirstSettings:true` to main; main threads that option into bootstrap, startup/session lookup and runtime SettingsManagers. Global-first merges project then global. Explicit empty global arrays therefore remain explicit values rather than falling through to project arrays. Resource and permission consumers have additional rules specified in their own entries.

`getEffectiveSettings`, `getGlobalSettings` and `getProjectSettings` return structured clones. `getSettingSource("a.b")` walks own properties, treats a final undefined as absent, and returns the first present stored scope in policy order. It does not claim runtime overrides come from either scope. `applyOverrides` deep-merges a validated object into effective memory only and emits a notification; a later save, trust transition or reload rebuilds effective settings from stored scopes, so such overrides are not permanent.

Trust changes clear modified-project tracking. Revocation immediately drops project settings, clears its load-error marker and rebuilds effective preferences. Granting trust rereads project storage and records any load error. No-op trust changes return without notification. Reload first awaits the existing write queue, then independently reloads scopes. A failed reload keeps that scope's previous in-memory settings, updates its active load error and records a diagnostic. Modified-field tracking is cleared; effective settings are rebuilt and listeners notified.

Subscribers are a Set of no-argument callbacks. Notifications coalesce within one queued microtask; the pending flag is cleared before iterating a snapshot of listeners, and listener exceptions are ignored. Unsubscribe removes the callback. This is in-memory preference invalidation, not a filesystem watcher or a durability acknowledgement.

### 5. Interfaces

`SettingsManagerCreateOptions {projectTrusted?:boolean; globalFirst?:boolean}` defaults true/false. APIs `isGlobalFirst()`, `getSettingSource(key):"global"|"project"|undefined`, `getEffectiveSettings()`, `applyOverrides(overrides)`, `setProjectTrusted(bool)`, `reload():Promise<void>`, `subscribeChanges(listener):()=>void`. Ordinary deep merge treats arrays as replacement values. Permission denies, global-only controls and restrictive limits must use their own accessors.

### 6. Dependencies

`SET-007`

SET-003 supplies trust. SET-009 supplies queued persistence. Subagent entries own deny/ask/cap composition and must not substitute this ordinary preference merge.

### 7. ice-rust today

`rust:crates/ice-cli/src/settings.rs:405-430` overlays only defaultProvider, defaultModel, defaultThinkingLevel and theme from trusted project settings onto global settings. It is project-first and is not a generic recursive effective manager. `rust:crates/ice-cli/src/resource_dirs.rs:109-308` applies separate project-first resource ordering. The inspected Rust CLI has no globalFirst option, stored-leaf source query, runtime override layer or coalesced SettingsManager subscriber API. Native UI/settings actions may rebuild their own state, but that is not the shared cross-consumer contract described here.

### 8. Port notes

Create a shared effective-settings service in ice-cli with typed scope and source provenance. Implement both precedence modes, explicitly opt the ICE distribution into global-first, and thread the same service through startup, reload and session replacement. Provide a clone/snapshot plus version/change notification mechanism for async consumers. Keep source-of-value and runtime override provenance distinct, and retain narrower global-only and security accessors.

### 9. Size and priority

Estimated Rust implementation: **260-430 lines** (estimate, not measured source). **P0** - Using the wrong precedence activates different models, packages and resources across the entire ICE distribution.

## SET-009 - Queued changed-field settings writes, nested merges and scoped mutation APIs

### 1. Title and status

Queued changed-field settings writes, nested merges and scoped mutation APIs. **Status: partial.**

### 2. ice source

`ice:packages/coding-agent/src/core/settings-manager.ts:566-630`, `:698-844`, `:1252-1378` (`setIceSettingsValue`, `setSettingValue`, persistence queue and scoped helpers). `ice:packages/coding-agent/test/settings-manager.test.ts:60-143`, `:314-326`.; ice:packages/coding-agent/test/settings-manager-bug.test.ts:37-164 (external array, project sibling and same-field write assertions; read, not run).

### 3. What it does

A preference update changes effective state immediately and later persists only the fields changed by that manager. Unrelated disk edits made after loading remain intact, while project writes are rejected when trust has been revoked.

### 4. How it works

Settings writes carry a snapshot of both values and the top-level or nested fields changed by this manager. A single promise queue serializes the actual storage callbacks and records write failures without making subsequent writes unusable.

Global/project modified fields are Sets; nested modifications are Maps from top-level key to Sets of immediate child names. Ordinary setters mutate their owned scope, mark fields and call save. Save recomputes effective settings and schedules the coalesced notification before persistence. Unless a load error blocks writing, it structured-clones the scope and copies tracking sets/maps into the queued task. Project saves assert trust before changing memory and again when the task executes. Each successful queued task clears live modified tracking for that scope; each failed task is caught and appended to diagnostics. `flush()` awaits the queue but does not itself throw a recorded write error; callers must inspect/drain diagnostics.

Inside the storage lock, persistence parses and migrates the latest current file, shallow-copies it, and applies only snapshotted modified top-level keys. If a key is tracked as nested and its snapshot value is a non-null object, it copies the latest nested object and assigns only the recorded immediate child keys; otherwise it replaces the whole top-level value. Undefined assignments disappear during JSON.stringify, enabling removal. This is not a recursive patch at arbitrary depth: updating `retry.provider` replaces that immediate child value. Unrelated latest-disk keys survive and this manager's changed key wins a same-key collision. The result is pretty JSON with two-space indentation.

`setSettingValue` accepts a dotted key with one or two nonempty components; deeper or empty components throw `Invalid settings key`. It assumes its caller already validated allowed keys/types (RPC owns that validation), structured-clones values and routes to global or trusted-project persistence. An unsupported scope throws. `setIceSettingsValue` replaces the cloned `ice` namespace but marks the union of old/new immediate keys, preserving unrelated top-level settings and deleting omitted namespace children on serialization. It rejects an untrusted project before mutation. Resource-array helpers write their requested scope; global-only setters are described in the relevant setting-family entries.

### 5. Interfaces

`setSettingValue(scope,key,value):void`, key depth 1–2; `getIceSettingsValue(scope):IceSettingsValue|undefined`; `setIceSettingsValue(scope,value):void`; `flush():Promise<void>`; `drainErrors():SettingsError[]`. Project-write error: `Project is not trusted; refusing to write project settings`. In-memory change notification and durable write completion are different observable events.

### 6. Dependencies

`SET-007`, `SET-008`

RPC setting validation and subagent namespace validation are separate entry owners. Filesystem/proper-lockfile are used by SET-007.

### 7. ice-rust today

`rust:crates/ice-cli/src/settings.rs:510-667` preserves unknown keys from the latest raw object and writes atomically, but overwrites/removes the entire hard-coded set of modeled preferences using the caller's Settings snapshot. It does not track which fields that caller changed, so another process's changes to an unrelated modeled key can be replaced. There is no two-scope queued manager, immediate-child patch tracking, generic validated scoped setter or second trust check at queued execution. `save_project_settings` at `:505-508` writes the selected path directly and does not itself enforce project trust. Other callers may preflight trust, so this statement is about the persistence API boundary.

### 8. Port notes

Add changed-key patches and an ordered write service above the native atomic writer. Re-read and merge under an interprocess lock, preserve unknown fields, and define removal explicitly. The returned acknowledgement should identify persistence failure without conflating it with notification. Check trust when accepting and executing project writes. If Rust uses deeper typed patches, retain the TS externally visible leaf behavior or record a deliberate compatible improvement.

### 9. Size and priority

Estimated Rust implementation: **240-420 lines** (estimate, not measured source). **P1** - Concurrent or delayed preference writes otherwise overwrite unrelated settings or outlive revoked project trust.

## SET-010 - Legacy settings migration without implicit product aliases

### 1. Title and status

Legacy settings migration without implicit product aliases. **Status: partial.**

### 2. ice source

`ice:packages/coding-agent/src/core/settings-manager.ts:457-481`, `:495-555`, `:770-773`; `ice:packages/coding-agent/src/core/legacy-compat/identity.ts:5-9`.

### 3. What it does

Old preference shapes are normalized when read, including queue mode, WebSocket transport, skill-directory settings and provider retry-delay limits. Modern explicit settings take precedence over the corresponding legacy form.

### 4. How it works

Migration runs when file or in-memory settings are loaded and again when the latest file is read for a changed-field write. The helper first requires a non-null non-array object and shallow-copies it; its legacy-compat name does not imply old product namespaces are translated.

If `queueMode` exists and `steeringMode` does not, copy queueMode to steeringMode and delete queueMode. If steeringMode exists, leave both as supplied. If transport does not exist and websockets is a boolean, convert true to websocket or false to sse and delete websockets; an existing transport prevents that conversion and deletion.

For a non-null non-array object-valued `skills`, copy its enableSkillCommands only when the top-level field is undefined. If customDirectories is a nonempty array, replace skills with that array; otherwise remove the old skills object. For object-valued retry, if maxDelayMs is numeric and retry.provider.maxRetryDelayMs is null or undefined, merge that value into the provider object. In all object-valued retry cases remove maxDelayMs, even when an explicit new delay wins. There is no global eager rewrite at load time: migration affects memory, and a later scoped save serializes the migrated latest file.

### 5. Interfaces

Legacy keys `queueMode`, `websockets:boolean`, `skills:{enableSkillCommands?,customDirectories?}`, `retry.maxDelayMs:number`. Targets `steeringMode`, `transport:"websocket"|"sse"`, top-level enableSkillCommands/skills array, `retry.provider.maxRetryDelayMs`. Root object validation error: `Invalid settings: expected an object`. This migration does not translate `.pi`, `PI_*` or arbitrary namespace aliases.

### 6. Dependencies

`SET-007`, `SET-009`

No external service or binary. Runtime retry behavior and resource loading remain in their owning entries.

### 7. ice-rust today

The Rust typed serde reader in `rust:crates/ice-cli/src/settings.rs:17-254`, `:468-475` accepts selected aliases such as enabledModels/skills/prompts/extensions/httpIdleTimeoutMs, but does not perform these queueMode, websockets, skills-object or retry.maxDelayMs migrations. An object-valued legacy skills setting cannot deserialize into Option<Vec<String>>; unknown keys may simply be ignored. Comment tolerance and Rust's native names are separate compatibility choices, not evidence of this migration.

### 8. Port notes

Normalize legacy JSON values before typed deserialization, with explicit field-presence checks and preservation of unrelated keys. Make precedence and deletion rules exact, including explicit null versus missing where the TS code distinguishes them. Apply the same migration to file and in-memory paths and on write merge, avoiding a destructive unconditional rewrite at startup.

### 9. Size and priority

Estimated Rust implementation: **90-150 lines** (estimate, not measured source). **P2** - Existing user settings should retain their intended behavior when brought to Rust.

## SET-011 - Model defaults, queue modes, transport preferences and global favorites

### 1. Title and status

Model defaults, queue modes, transport preferences and global favorites. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/settings-manager.ts:126-184; ice:packages/coding-agent/src/core/settings-manager.ts:877-924; ice:packages/coding-agent/src/core/settings-manager.ts:959-987; ice:packages/coding-agent/src/core/settings-manager.ts:1391-1393; ice:packages/coding-agent/src/core/settings-manager.ts:1502-1521; ice:packages/coding-agent/docs/settings.md:1-372.

Named export: `TransportSetting` (`ice:packages/coding-agent/src/core/settings-manager.ts:82`).

`ice:packages/coding-agent/src/core/settings-manager.ts:82` declares `TransportSetting = Transport`; this is a type alias of the AI transport contract, with no separate runtime behavior.

### 3. What it does

Saved defaults select the initial provider/model/thinking mode and queue delivery behavior, while transport/fast preferences and model lists reach their own runtime consumers.

### 4. How it works

These accessors read the effective settings snapshot but deliberately treat favorites as global user state. Setters update global fields, mark the changed keys and use the shared queued persistence path.

Default provider, model and thinking level remain undefined when unset; setDefaultModelAndProvider updates both before one save. steeringMode and followUpMode use a truthy stored value or 'one-at-a-time'. fastMode defaults false and transport defaults 'auto'. These getters are not whole-file validators: a hand-authored invalid truthy queue/transport string is not normalized here. thinkingBudgets returns the effective object reference with optional minimal, low, medium, high numbers; this layer does not infer extra levels or clamp budgets.

enabledModels is an optional effective string-pattern array and is returned without a defensive copy; undefined differs from an explicitly empty array. favoriteModels ignores project scope, filters a stored global array to strings and otherwise returns []; its setter deduplicates exact strings in first-occurrence order. Model availability, pattern matching, cycling, supported reasoning levels and fast-tier eligibility are runtime consumer policies, not settings-parser guarantees.

### 5. Interfaces

Keys: defaultProvider, defaultModel, defaultThinkingLevel, steeringMode, followUpMode, fastMode, transport, thinkingBudgets, enabledModels, favoriteModels. Queue values are all|one-at-a-time; transport uses the AI Transport type. get/set methods retain the source casing; getThinkingBudgets():ThinkingBudgetsSettings|undefined and getFavoriteModels():string[]. Default model/provider setters and setDefaultModelAndProvider return void.

### 6. Dependencies

`SET-008`, `SET-009`

AGENT-007/025 describe per-request options; AGENT-022 describes queue delivery. Providers/model-selection and TUI entries must supply capability validation and favorites UI. The settings entry does not duplicate those algorithms.

Final dependency classification. Consuming integrations: `AUTH-024`, `AUTH-025`, `AUTH-026`, `AI-005`, `AI-006`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

rust:crates/ice-cli/src/settings.rs:28-110; rust:crates/ice-cli/src/settings.rs:256-329; rust:crates/ice-cli/src/settings.rs:387-418 already parses provider/model/thinking defaults and queue modes, constructs harness policies, and aliases enabledModels to scopedModels. Trusted project model defaults win in load_effective_model_settings, differing from ICE's global-first entry point. The typed schema at rust:crates/ice-cli/src/settings.rs:28-238 has no fastMode, transport, thinkingBudgets or favoriteModels fields, so those stored preferences do not get corresponding typed accessors here. rust:crates/ice-cli/src/settings.rs:925-1026 verifies queue/default policy and scoped-model round trips in existing test source. Provider-level ThinkingBudgets exists elsewhere; absence of the settings field is not absence of all low-level budget support.

### 8. Port notes

Extend the typed settings model and persistence projection for the missing preferences, preserving undefined versus explicit-empty lists and global-only favorites. Route validated preferences into the existing harness/provider construction instead of merely accepting JSON keys. Implement explicit conversion diagnostics for unsupported queue/transport/thinking values and decide compatibility with TS's permissive getters. Add future isolated-source tests for precedence, deduplication, list clearing and options reaching their consumers.

### 9. Size and priority

Estimated Rust implementation: **160-280 lines** (estimate, not measured source). **P1** - A parsed configuration must actually select the intended model behavior and queue modes; ignored preferences can silently change requests.

## SET-012 - Compaction, branch-summary and two-layer retry settings

### 1. Title and status

Compaction, branch-summary and two-layer retry settings. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/settings-manager.ts:25-49; ice:packages/coding-agent/src/core/settings-manager.ts:989-1094; ice:packages/coding-agent/src/core/settings-manager.ts:1131-1137; ice:packages/coding-agent/test/settings-manager.test.ts:170-307; ice:packages/coding-agent/test/suite/regressions/7572-provider-retry-settings-merge.test.ts:1-36.

Named export: `BranchSummarySettings` (`ice:packages/coding-agent/src/core/settings-manager.ts:33`).

Named export: `ProviderRetrySettings` (`ice:packages/coding-agent/src/core/settings-manager.ts:38`).

Named export: `RetrySettings` (`ice:packages/coding-agent/src/core/settings-manager.ts:44`).

### 3. What it does

Agent-level retry, provider request retry and compaction each have separate settings and defaults. Nested edits preserve unrelated options rather than replacing the whole policy object.

### 4. How it works

The settings manager exposes compaction and retry policies from its merged snapshot, applying defaults at read time. Threshold and mid-run mode have explicit normalization, while several token/count fields remain raw numeric settings and are validated or consumed later.

Compaction defaults enabled=true, reserveTokens=16384, keepRecentTokens=20000, thresholdPercent=85, midRunCompaction='off'. hasExplicitCompactionReserveTokens tests presence rather than comparing the value with the default. A non-number, non-finite or non-positive threshold reads as 85; other values are clamped to 1..99. The threshold setter rejects non-finite/non-positive input and clamps valid values. Only 'pause' and 'resume' survive the mid-run getter; everything else reads as 'off'. Setters for enabled, threshold and mid-run mode mark the corresponding immediate nested key before saving.

Branch summaries default reserveTokens=16384 and skipPrompt=false; the convenience skip getter reads that same result. Agent retry defaults enabled=true, maxRetries=3, baseDelayMs=2000. Provider retry is a distinct retry.provider object: timeoutMs and maxRetries remain optional, while maxRetryDelayMs defaults to 60000. These are provider request controls, not the agent retry count. The regression test demonstrates that changing retry.enabled preserves existing retry.provider fields through merge and save. The old retry.maxDelayMs migration is SET-010.

### 5. Interfaces

CompactionSettings={enabled?, thresholdPercent?, reserveTokens?, keepRecentTokens?, midRunCompaction?}; getCompactionSettings returns all five resolved fields. MidRunCompaction='off'|'pause'|'resume'. getBranchSummarySettings():{reserveTokens:number;skipPrompt:boolean}; getRetrySettings():{enabled:boolean;maxRetries:number;baseDelayMs:number}; getProviderRetrySettings():{timeoutMs?:number;maxRetries?:number;maxRetryDelayMs:number}. No setting in this group itself starts a retry, compaction or model request.

### 6. Dependencies

`SET-008`, `SET-009`

SESSION-015/016/017 own compaction decisions/lifecycle; SESSION-018 owns retry generation/callback lifecycle. AI-001 is the shared provider retry/classifier finding. They consume this policy rather than making this settings layer execute requests.

Final dependency classification. Consuming integrations: `SESSION-015`, `SESSION-016`, `SESSION-018`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

rust:crates/ice-cli/src/settings.rs:28-79; rust:crates/ice-cli/src/settings.rs:266-329 already constructs retry and compaction policies from parsed fields, including enabled/maxRetries/baseDelayMs and reserveTokens/keepRecentTokens. It also parses branch-summary reserve/skip and an extra maxAgentDelayMs. thresholdPercent and midRunCompaction are absent from the typed CompactionSettings; provider is retained as an unstructured JSON value but the inspected retry_policy conversion does not forward it. Compaction modelOverrides is retained separately without demonstrating consumption. Existing source tests at rust:crates/ice-cli/src/settings.rs:925-1009 confirm partial defaults and branch getters, not the complete TS policy or concurrent setting writes. Runtime use is evaluated by the session/AI owners.

### 8. Port notes

Add typed provider retry settings and threshold/mid-run fields, then connect them to the exact session/provider boundaries. Preserve explicit-reserve presence and nested changed-key persistence; do not merge agent retry and provider retry into one counter. Keep Rust maxAgentDelayMs if intentional and document how it composes. Future checks should cover invalid thresholds, nested provider preservation, explicit false/zero values, and the settings selected after acquiring the relevant session operation guard.

### 9. Size and priority

Estimated Rust implementation: **130-240 lines** (estimate, not measured source). **P1** - Missing thresholds, provider retry options and mode handling change recovery and compaction behavior under load.

## SET-013 - HTTP proxy application, active-request idle deadlines and dispatcher replacement

### 1. Title and status

HTTP proxy application, active-request idle deadlines and dispatcher replacement. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/http-dispatcher.ts:1-112; ice:packages/coding-agent/src/core/settings-manager.ts:1118-1141; ice:packages/coding-agent/src/main.ts:543-549; ice:packages/coding-agent/test/settings-manager.test.ts:390-411.

### 3. What it does

The saved HTTP timeout controls stalled response headers and bodies, and proxy configuration reaches ICE-managed HTTP clients without overwriting explicitly supplied environment values.

### 4. How it works

Timeout parsing accepts nonnegative finite numbers, numeric strings and case-insensitive 'disabled', then floors numeric values. The settings getter rejects an explicitly supplied invalid value instead of quietly falling back, and the dispatcher applies the resolved timeout to active HTTP response phases.

parseHttpIdleTimeoutMs trims strings, returns 0 for disabled, undefined for an empty or invalid value, and recursively converts other strings through Number. getHttpIdleTimeoutMs defaults to 300000 only when absent; its setter requires a finite nonnegative number and persists the floored result. websocketConnectTimeoutMs uses the same parser but remains undefined when absent; its actual connection default belongs to the provider layer. The shared timeout chooser contains 30000,60000,120000,300000 and disabled=0, with custom labels expressed in seconds.

applyHttpProxySettings trims a configured proxy, ignores an empty result and fills HTTP_PROXY/HTTPS_PROXY only with nullish assignment, preserving existing strings including empty strings. Bootstrap applies the global setting before project trust. configureHttpDispatcher creates an Undici EnvHttpProxyAgent with allowH2=false, bodyTimeout and headersTimeout set to the normalized value, and connect.autoSelectFamilyAttemptTimeout=2000. Single-connection origins get a Client, others a Pool whose clients carry the same benign error listener. That listener prevents an unhandled internal EventEmitter error from crashing the process; stream readers still receive body failures.

The new dispatcher is installed globally. The module remembers the original fetch and its own installed fetch; undici.install is called only if fetch still matches one of those expected values, preserving a caller's deliberate replacement. The source does not close previous dispatchers here or promise that all third-party HTTP libraries use Undici.

### 5. Interfaces

DEFAULT_HTTP_IDLE_TIMEOUT_MS=300000; HTTP_IDLE_TIMEOUT_CHOICES; parseHttpIdleTimeoutMs(unknown):number|undefined; formatHttpIdleTimeoutMs(number):string; applyHttpProxySettings(string|undefined):void; configureHttpDispatcher(timeoutMs=300000):void. Stored keys httpProxy, httpIdleTimeoutMs, websocketConnectTimeoutMs. Invalid explicit settings throw 'Invalid <key> setting: <value>'; invalid dispatcher input throws 'Invalid HTTP idle timeout: <value>'.

### 6. Dependencies

`SET-008`, `SET-009`

External dependency Undici supplies dispatcher/client/pool/fetch APIs; native Rust can implement the observable request policy without reproducing global JavaScript monkey-patching. Provider retry and transport adapters consume these timeouts.

### 7. ice-rust today

rust:crates/ice-cli/src/settings.rs:125-130; rust:crates/ice-cli/src/settings.rs:231-263 accepts httpProxy and aliases httpIdleTimeoutMs onto a raw httpIdleTimeout value. rust:crates/ice-ai/src/http.rs:66-84 accepts only integer JSON numbers or integer strings, rejecting fractions/exponent strings that TS Number/floor accepts; invalid values become None rather than a settings diagnostic. Most importantly, rust:crates/ice-ai/src/http.rs:183-213 applies pool_idle_timeout, which expires idle pooled connections, not stalled active headers/body reads. Matching the 300000 constant therefore does not establish the TS timeout contract. rust:crates/ice-cli/src/provider.rs:211-222 uses this builder but falls back to a fresh reqwest::Client on builder errors, potentially hiding an explicit proxy configuration error. The schema has no websocketConnectTimeoutMs field. Preserve the existing native proxy resolver while auditing its AI-level matching separately. Global httpProxy is actually applied at rust:crates/ice-cli/src/session.rs:708-723 by filling only absent HTTP_PROXY/HTTPS_PROXY; unlike TS, this block does not trim or skip an empty configured proxy.

### 8. Port notes

Represent active header/body idle deadlines separately from connection-pool retention. Add explicit invalid-setting diagnostics, preserve disabled=0 and define accepted numeric conversion. Pass validated proxy/timeout configuration through all owned clients, and propagate builder failures instead of silently changing routing policy. Keep custom/native HTTP clients isolated where necessary and make their timeout policy explicit. Future injected-client checks should distinguish a quiet active stream from an unused pooled connection and verify user-supplied proxy errors remain visible.

### 9. Size and priority

Estimated Rust implementation: **180-320 lines** (estimate, not measured source). **P1** - The current similarly named Rust timeout governs a different lifecycle and can leave a stalled active request unaffected.

## SET-014 - Shell path, background timers and restrictive stdin approval composition

### 1. Title and status

Shell path, background timers and restrictive stdin approval composition. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/settings-manager.ts:1096-1116; ice:packages/coding-agent/src/core/settings-manager.ts:1179-1188; ice:packages/coding-agent/src/core/settings-manager.ts:1231-1250; ice:packages/coding-agent/test/settings-manager.test.ts:308-388; ice:packages/coding-agent/test/settings-manager.test.ts:514-601; ice:packages/coding-agent/src/core/http-dispatcher.ts:19-35.

Named export: `DEFAULT_BASH_AUTO_BACKGROUND_MS` (`ice:packages/coding-agent/src/core/settings-manager.ts:216`).

`ice:packages/coding-agent/src/core/settings-manager.ts:216` defines `DEFAULT_BASH_AUTO_BACKGROUND_MS = 60_000`, the named default consumed by this setting.

### 3. What it does

Shell settings choose the configured shell and command prefix, release/alert timers and whether background stdin requires confirmation. A project cannot use ordinary precedence to grant itself unprompted stdin access.

### 4. How it works

The timer getters share strict explicit-value timeout parsing, so zero disables a timer while an invalid supplied value raises a diagnostic. Stdin approval instead composes the raw global and trusted-project policies restrictively, independently of global-first precedence.

bashAutoBackgroundMs defaults to 60000, bashSilenceAlertMs to 120000 and bashPromptAlertMs to 45000. Numeric values are finite/nonnegative/floored; numeric strings and 'disabled' are accepted through the shared parser, and absent settings get the defaults. These values configure different foreground-release, silence and prompt detectors; they are not the explicit bash_watch minimum bounds.

getBashInputApproval returns 'allow' only when globalSettings.bashInputApproval is exactly 'allow' and projectSettings.bashInputApproval is not exactly 'ask'; otherwise it returns 'ask'. Untrusted project settings have already been removed by SET-003/008. A project 'allow' cannot override a global ask, and a project ask constrains global allow even in global-first mode. Tool consumption then decides whether UI confirmation exists (TOOLS-009); no-UI denial applies to ask, not an unconditional ban.

getShellPath normalizes a truthy configured path, including leading home expansion, and otherwise returns undefined. Its setter can clear the global field. shellCommandPrefix is returned as stored and is prepended by tool construction, not executed by the settings accessor. defaultProjectTrust is global-only and normalizes only always/never, with ask fallback (SET-003). All dedicated shell setters use changed-field queued saves.

### 5. Interfaces

Keys shellPath, shellCommandPrefix, bashAutoBackgroundMs, bashSilenceAlertMs, bashPromptAlertMs, bashInputApproval. getBashInputApproval():'ask'|'allow'; timer getters return milliseconds. getShellPath():string|undefined; setShellPath(path?:string):void; get/setShellCommandPrefix. Generic scoped editing uses SET-009 when no dedicated timer/approval setter exists.

### 6. Dependencies

`SET-003`, `SET-008`, `SET-009`

TOOLS-001/004 consume foreground timing, TOOLS-009 consumes approval, TOOLS-013 consumes alert timers. AGENT-048 owns shell discovery/transport. These consumer references do not make the core settings manager depend on tool implementations.

Final dependency classification. Consuming integrations: `TOOLS-001`, `TOOLS-005`, `TOOLS-009`, `TOOLS-011`, `TOOLS-012`, `TOOLS-013`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

rust:crates/ice-cli/src/settings.rs:113-123 parses shellPath and shellCommandPrefix; rust:crates/ice-cli/src/settings.rs:897-910 explicitly tests parsing while noting shellPath is unconsumed. rust:crates/ice-cli/src/session.rs:1932-1955 reloads global settings for command_prefix and supplies a separate ICERS_BASH_TIMEOUT default of 120 seconds. It does not pass shellPath, the TS release/alert timer settings or stdin approval policy. The Rust execution environment already exposes with_shell_path (covered by AGENT-048), so missing CLI wiring must not be mistaken for a missing low-level facility. The typed settings schema has no background timer or bashInputApproval fields.

### 8. Port notes

Add and validate the timer/approval fields, pass shellPath into the existing execution environment and connect command-prefix resolution to the chosen scoped policy. Keep execution timeout separate from automatic background release. Implement approval composition before enabling retained stdin: only a global explicit allow may relax confirmation, and trusted projects may tighten it. Future checks should cover global-first mode, untrusted projects, project ask over global allow, zero/disabled/fractional timers, and the selected shell reaching tool construction.

### 9. Size and priority

Estimated Rust implementation: **120-220 lines** (estimate, not measured source). **P0** - Background-shell ports require the exact restrictive approval rule and timer separation before exposing writable stdin.

## SET-015 - Display, editor, cursor and warning preferences with exact fallback rules

### 1. Title and status

Display, editor, cursor and warning preferences with exact fallback rules. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/settings-manager.ts:1143-1177; ice:packages/coding-agent/src/core/settings-manager.ts:1190-1229; ice:packages/coding-agent/src/core/settings-manager.ts:1425-1474; ice:packages/coding-agent/src/core/settings-manager.ts:1523-1587; ice:packages/coding-agent/src/core/settings-manager.ts:1602-1610; ice:packages/coding-agent/test/settings-manager.test.ts:413-512.

Named export: `UiMode` (`ice:packages/coding-agent/src/core/settings-manager.ts:51`).

Named export: `MarkdownSettings` (`ice:packages/coding-agent/src/core/settings-manager.ts:72`).

Named export: `WarningSettings` (`ice:packages/coding-agent/src/core/settings-manager.ts:76`).

`ice:packages/coding-agent/src/core/settings-manager.ts:51-82` (`UiMode`, `MarkdownSettings`, `WarningSettings`).

### 3. What it does

Display and editor preferences have specific defaults and validation boundaries, including terminal progress, cursor behavior, padding, tree filtering and external-editor selection.

### 4. How it works

Most display getters read the effective snapshot and default only their own key, while setters save global values through changed-key persistence. Validation is intentionally uneven in the source, so read-time fallback and setter-time clamping must be documented separately.

hideThinkingBlock=false, showCacheMissNotices=false, quietStartup=false; an explicit hideThinking presence check is separate from its value. footerStyle accepts legacy otherwise compact; toolStyle accepts boxed otherwise compact; symbols accepts unicode/nerd/ascii otherwise auto. uiMode is fullscreen only for that exact value, otherwise regular. fullscreenScrollbar accepts always/auto otherwise hidden. doubleEscapeAction uses nullish fallback 'tree' without validating other stored strings; treeFilterMode checks default/no-tools/user-only/labeled-only/all and otherwise returns default.

terminal.clearOnShrink prefers any defined setting, then only ICE_CLEAR_ON_SHRINK==='1'; terminal.showTerminalProgress defaults false. showHardwareCursor similarly prefers a defined setting, then only ICE_HARDWARE_CURSOR==='1'. editorPaddingX reads raw or 0, while its setter floors and clamps 0..3. autocompleteMaxVisible reads raw or 5, while its setter floors and clamps 3..20. outputPad returns 0 only for exact numeric zero, otherwise 1. markdown.codeBlockIndent defaults to two spaces. Thus a hand-edited invalid numeric value can differ from a value passed through a setter.

externalEditor wins when it is a nonblank string, but the returned command preserves its original whitespace. Otherwise choose truthy VISUAL, then EDITOR, then notepad on Windows or nano elsewhere. getWarnings returns a shallow copy of the effective warnings object; setWarnings replaces a copied global object. warnings.anthropicExtraUsage is an optional field whose default-true behavior belongs to the warning consumer, not a default injected by getWarnings.

### 5. Interfaces

Keys hideThinkingBlock, showCacheMissNotices, externalEditor, quietStartup, footerStyle, toolStyle, symbols, terminal.clearOnShrink, terminal.showTerminalProgress, uiMode, fullscreenScrollbar, doubleEscapeAction, treeFilterMode, showHardwareCursor, editorPaddingX, outputPad, autocompleteMaxVisible, markdown.codeBlockIndent, warnings.anthropicExtraUsage. Environment fallback names are ICE_CLEAR_ON_SHRINK, ICE_HARDWARE_CURSOR, VISUAL, EDITOR. Dedicated setters return void and use global scope.

The source declares `UiMode = TuiMode`, `MarkdownSettings { codeBlockIndent?: string }`, and `WarningSettings { anthropicExtraUsage?: boolean }`. The two-space Markdown fallback and the warning consumer's default are the separate behaviors above; the optional interfaces themselves do not inject values.

### 6. Dependencies

`SET-008`, `SET-009`

TUI entries own rendering, key events, cursor/mouse operation, warning conditions and external-editor process execution. This entry supplies the stored policy and accessor behavior; terminal images are SET-016 and appearance/theme state is SET-017.

Final dependency classification. Consuming integrations: `TUI-025`, `TUI-049`, `TUI-052`, `TUI-053`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

rust:crates/ice-cli/src/settings.rs:143-253 already includes hide-thinking, startup, progress, editor/autocomplete padding, output padding and some compatibility flat/nested terminal keys. rust:crates/ice-cli/src/interactive_tui.rs:802-868 uses trusted project-first values, but editor padding defaults to 1 and clamps to at most 16; autocomplete clamps 1..20; terminal progress defaults true. Those differ from TS 0/0..3, 3..20 and false. The typed schema does not represent footerStyle/toolStyle/symbols/uiMode/fullscreenScrollbar/treeFilterMode/externalEditor/markdown/warnings/showHardwareCursor as the same settings contract. Existing rust:crates/ice-cli/src/settings.rs:1094-1155 tests establish preservation and writes for supported TUI fields, not all listed preferences. Rust's native fullscreenCopyOnSelect is an additional capability to preserve.

### 8. Port notes

Define the supported display schema and conversion/default rules centrally, then make startup and live settings changes use the same resolver. Preserve intentional Rust-specific options and record deliberate default differences instead of silently rebranding them as parity. Add missing keys only with their TUI consumers, keep process command parsing separate from editor selection and maintain global-only mutation semantics where applicable. Future verification should compare malformed on-disk values with setter inputs and ensure explicit false/zero survives precedence.

### 9. Size and priority

Estimated Rust implementation: **200-360 lines** (estimate, not measured source). **P2** - These preferences affect everyday interaction and consistency, but most do not block the underlying execution model.

## SET-016 - Independent image transmission, resizing and terminal presentation settings

### 1. Title and status

Independent image transmission, resizing and terminal presentation settings. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/settings-manager.ts:53-63; ice:packages/coding-agent/src/core/settings-manager.ts:1395-1423; ice:packages/coding-agent/src/core/settings-manager.ts:1476-1500; ice:packages/coding-agent/docs/settings.md:1-372.

Named export: `TerminalSettings` (`ice:packages/coding-agent/src/core/settings-manager.ts:53`).

Named export: `ImageSettings` (`ice:packages/coding-agent/src/core/settings-manager.ts:60`).

### 3. What it does

A user can separately control image payloads sent to providers, automatic image resizing and terminal image display. Disabling terminal images is not the same as blocking model-bound images.

### 4. How it works

The settings manager exposes two separate nested objects: images governs provider payload processing, while terminal governs display. Their setters mark individual child keys so changing one preference preserves sibling settings.

images.autoResize defaults true and images.blockImages defaults false. getImageAutoResize/getBlockImages return the effective nested values with nullish defaults; their dedicated setters create the global images object when absent and update only autoResize or blockImages. The runtime read-tool option is named autoResizeImages, but that is not the stored JSON key.

terminal.showImages defaults true. terminal.imageWidthCells reads as 60 unless it is a finite number, then floors and clamps to a minimum of 1 without a maximum. The width setter performs floor/minimum normalization but has no finite-number guard; read-time handling of a persisted invalid/null result is therefore material. Terminal show/width setters mark one terminal child key. Image decoding, orientation, MIME/quality handling and coordinate hints are tools behavior; filtering provider-bound content and user attachments is session/provider behavior.

### 5. Interfaces

Stored JSON keys images.autoResize, images.blockImages, terminal.showImages, terminal.imageWidthCells. Accessors get/setImageAutoResize, get/setBlockImages, get/setShowImages, get/setImageWidthCells. Defaults true, false, true,60 respectively. Runtime autoResizeImages is an option consumed by TOOLS-022/023 and must not be advertised as a persisted top-level key.

### 6. Dependencies

`SET-008`, `SET-009`

TOOLS-022/023 own read-tool and image-processing contracts. Session/provider owners must connect blockImages before model requests; the TUI owns rendering. These are three distinct consumers of the settings policy.

Final dependency classification. Consuming integrations: `TUI-225`, `TUI-228`, `TUI-229`, `TOOLS-023`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

rust:crates/ice-cli/src/settings.rs:223-253; rust:crates/ice-cli/src/settings.rs:339-351 supports showImages as a native flat key with terminal.showImages fallback, and TUI startup honors it at rust:crates/ice-cli/src/interactive_tui.rs:849-852. The schema has no images.autoResize/images.blockImages or terminal.imageWidthCells fields. rust:crates/ice-cli/src/session.rs:1972-1981 installs the real InlineImageProcessor using default ReadToolOptions, without a stored autoResizeImages override here. Native image processing therefore exists, but that does not establish user control over resizing or provider transmission. Existing terminal-setting source tests at rust:crates/ice-cli/src/settings.rs:1130-1155 establish flat-key precedence, which differs from the TS nested-only storage contract.

### 8. Port notes

Add separate typed image-policy and terminal-display fields and wire them to their own consumers. Preserve the native processor; allow the user to control its resize policy through the canonical images.autoResize key. Implement provider-content filtering for images.blockImages without conflating it with hiding UI previews, and propagate the resolved image width to rendering. Future checks should cover each switch independently, nested-field preservation, flat compatibility-key precedence and malformed width values.

### 9. Size and priority

Estimated Rust implementation: **100-190 lines** (estimate, not measured source). **P1** - Image transmission is a user policy distinct from UI display, and silently ignoring it changes data sent in model requests.

## SET-017 - Typed fixed/automatic themes and global appearance persistence

### 1. Title and status

Typed fixed/automatic themes and global appearance persistence. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/settings-manager.ts:925-957; ice:packages/coding-agent/src/core/settings-manager.ts:1589-1600; ice:packages/coding-agent/src/modes/interactive/appearance/appearance-types.ts:262-319.

`serializeAppearanceThemeSetting` — `ice:packages/coding-agent/src/modes/interactive/appearance/appearance-types.ts:316`.

### 3. What it does

Theme selection has a typed fixed/automatic form, while the broader appearance object is global user configuration. Saving a theme through the customizer targets the scope that currently supplies it.

### 4. How it works

The theme getter and appearance getter deliberately use different scope rules. Theme selection follows effective precedence, whereas the appearance object is read only from global settings and validated by the appearance subsystem.

getThemeSetting returns only a string. getTheme returns undefined when that string contains '/', keeping the legacy fixed-theme getter from treating an automatic pair as a name. parseAppearanceThemeSetting rejects a missing/empty string; a string without '/' becomes {mode:'fixed', theme:raw} without trimming. Exactly one slash yields an automatic pair only when both trimmed sides are nonempty; multiple slashes are rejected. Serialization returns the fixed name or light+'/'+dark.

setTheme writes global scope. In contrast, setAppearanceThemeSetting asks getSettingSource('theme') and writes back to that scope, falling back to global when absent, so a project override cannot hide a customizer save. That call still uses trusted-project checks. getAppearanceSettings validates globalSettings.appearance and returns the normalized appearance even when the stored input is invalid. setAppearanceSettings returns the validation result without writing on invalid input; on success it stores a structured clone and marks appearance. The source's Settings comment still says v1, but the imported type and persisted version are AppearanceSettingsV2/version:2.

### 5. Interfaces

AppearanceThemeSetting={mode:'fixed';theme:string}|{mode:'automatic';light:string;dark:string}; get/setAppearanceThemeSetting. AppearanceSettingsV2 has version:2 plus userMessage, assistantMessage, inputBox, thinking, statusIndicators, markdown, tools, bash, diff, systemCards, footer, chrome, subagentChrome, inputHighlighters. getAppearanceSettings():AppearanceSettingsV2; setAppearanceSettings(...):AppearanceValidationResult. Detailed nested token/validation/rendering contracts belong to TUI.

### 6. Dependencies

`SET-003`, `SET-008`, `SET-009`

The TUI area owns appearance validation, customizer controls, theme format and automatic terminal-background switching. SET-033 owns resource discovery. EXT-049 exposes theme operations to extensions.

Final dependency classification. Consuming integrations: `TUI-001`, `TUI-003`, `TUI-009`, `TUI-010`, `TUI-024`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

rust:crates/ice-cli/src/settings.rs:86-110; rust:crates/ice-cli/src/settings.rs:387-418; rust:crates/ice-cli/src/settings.rs:539-563 parses and persists a theme string with project-first effective model/theme selection. It has no typed appearance object or scope-preserving customizer setter. A native theme engine is an existing separate capability; this entry does not infer its format support from the settings string alone. Active first-pass §11a's historical lavender-ice import omission requires the TUI format comparison, and actual private theme files were not accessed.

### 8. Port notes

Add the typed theme persistence adapter and explicit global appearance validation boundary, reusing native rendering types where they can represent the needed tokens. Preserve the distinction between fixed-theme helpers and automatic pairs; target the effective theme's scope when saving through the customizer. Treat version conversion as a validated migration, and expose normalized diagnostics rather than silently dropping an imported appearance object. Future checks should cover whitespace, slash/path ambiguity, invalid appearance, project trust and global-first save scope.

### 9. Size and priority

Estimated Rust implementation: **100-200 lines** (estimate, not measured source). **P2** - Correct scope and version handling are needed to make appearance changes persist and imported theme choices understandable.

## SET-018 - Install telemetry, analytics consent and changelog persistence

### 1. Title and status

Install telemetry, analytics consent and changelog persistence. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/settings-manager.ts:128; ice:packages/coding-agent/src/core/settings-manager.ts:151-154; ice:packages/coding-agent/src/core/settings-manager.ts:862-870; ice:packages/coding-agent/src/core/settings-manager.ts:1262-1299; ice:packages/coding-agent/docs/settings.md:1-372.

### 3. What it does

Install telemetry, optional analytics and changelog markers are separate settings. Opting into analytics creates a persistent tracking identifier only when one is absent.

### 4. How it works

The manager provides explicit accessors for update-related state and two independent data-sharing switches. Values are persisted through the same global changed-field queue as other preferences.

getLastChangelogVersion returns the optional stored marker; setLastChangelogVersion writes it globally. collapseChangelog defaults false and its setter changes only that field. enableInstallTelemetry defaults true, independent of enableAnalytics, which defaults false. getTrackingId returns the optional effective value.

setEnableAnalytics writes the global enabled flag. When enabling and the global trackingId is falsy, it generates randomUUID(), marks trackingId and saves both changes; disabling does not remove the existing identifier. The function tests the global value, not a project-provided identifier. These accessors do not themselves transmit anything, and the install telemetry sender has additional environment/offline/update gates described in OBS-001.

### 5. Interfaces

Keys lastChangelogVersion, collapseChangelog, enableInstallTelemetry, enableAnalytics, trackingId; getters return optional string or boolean as specified. setEnableAnalytics(boolean):void may generate a UUID through crypto.randomUUID. There is no implied analytics network endpoint in this module.

### 6. Dependencies

`SET-008`, `SET-009`

OBS-001 owns installation telemetry transmission and its gates. CLI changelog/update logic consumes lastChangelogVersion/collapseChangelog. External dependency crypto.randomUUID supplies the persistent identifier.

### 7. ice-rust today

rust:crates/ice-cli/src/settings.rs:170-196; rust:crates/ice-cli/src/settings.rs:539-670 already persists changelog markers and collapseChangelog. It has an additional lastIceChangelogVersion marker so native release history need not overwrite a foreign baseline; rust:crates/ice-cli/src/settings.rs:1030-1068 tests that behavior. The typed schema has no enableInstallTelemetry, enableAnalytics or trackingId, and the native telemetry crate is a span backend rather than evidence of these consent settings. OBS-001 separately compares the missing TS install ping and preserves existing Rust instrumentation.

### 8. Port notes

Retain the separate native changelog marker. If adding the corresponding transmission features, first add independent consent fields and exactly define effective versus global ID behavior; never infer analytics consent from install telemetry. Keep network actions in their owning subsystem, persist identifier generation with the enabled change and preserve it on opt-out unless a separately specified deletion policy applies. Future isolated tests should distinguish enabling twice, disabling, project-provided values and malformed existing IDs.

### 9. Size and priority

Estimated Rust implementation: **60-120 lines** (estimate, not measured source). **P2** - The consent and marker contracts must be explicit before related telemetry or update features are ported.

## SET-019 - Scoped resource arrays, package filters and skill-command preference

### 1. Title and status

Scoped resource arrays, package filters and skill-command preference. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/settings-manager.ts:84-99; ice:packages/coding-agent/src/core/settings-manager.ts:155-160; ice:packages/coding-agent/src/core/settings-manager.ts:1301-1389; ice:packages/coding-agent/test/ice-resource-precedence.test.ts:1-129.

### 3. What it does

Users and trusted projects can select packages and additional extension/skill/prompt/theme paths. Package filters and an explicitly empty array have effects distinct from a missing setting.

### 4. How it works

Resource getters return shallow copies of their effective arrays, and user/project setters store the chosen scope through the settings persistence path. The arrays are replacement values under scoped merging; they are not automatically concatenated by the settings manager.

packages accepts strings or objects with source:string, autoload?:boolean and optional extensions/skills/prompts/themes string arrays. A string requests all resources from that package; autoload=false means start empty and apply explicit resource patterns, as consumed by the package manager. extensions, skills, prompts, themes are additional local file/directory path arrays; the getter copy prevents direct array mutation, but package object elements are not deep-cloned by those getters.

setPackages/setExtensionPaths/setSkillPaths/setPromptTemplatePaths/setThemePaths write global scope, and the corresponding setProject* methods write project scope only through the trusted-project persistence rules. In global-first ICE, a defined global empty packages array can intentionally suppress a project array; the precedence regression checks this. enableSkillCommands defaults true and controls later slash-command exposure, not whether skills can be loaded or included in a model prompt. Path expansion, package discovery, disabled-resource reporting and collision selection occur in the resource loader rather than these accessors.

### 5. Interfaces

PackageSource=string|{source:string;autoload?:boolean;extensions?:string[];skills?:string[];prompts?:string[];themes?:string[]}; resource keys packages, extensions, skills, prompts, themes, enableSkillCommands. All array getters return [] when absent. Global/project setters are separate; getEnableSkillCommands()/setEnableSkillCommands(boolean).

### 6. Dependencies

`SET-003`, `SET-008`, `SET-009`

SET-021/023/028/030/033 consume resource paths and metadata; cli-packaging owns npm/git package acquisition and filtering. Skills command registration is a session/TUI consumer. External package-manager dependencies are not required merely to read the stored arrays.

### 7. ice-rust today

rust:crates/ice-cli/src/settings.rs:132-153 aliases skills/prompts/extensions onto skillDirs/promptDirs/extensionDirs and rust:crates/ice-cli/src/resource_dirs.rs:43-228 builds a real project/global path list. Native package resource discovery is separate and does not by itself implement the TS PackageSource object or global-first array replacement. The settings type lacks themes and enableSkillCommands. Existing Rust resource loaders deduplicate names in source order (rust:crates/ice-cli/src/resource_dirs.rs:232-294), so ordering changes user-visible winners. Do not label all resource loading missing merely because the TS accessor names differ.

### 8. Port notes

Introduce a resource settings projection that preserves scope, defined-empty arrays and object filters without forcing Node package support into the native loader. Translate accepted aliases at the edge and use one explicit precedence order for every resource type. Connect enableSkillCommands to command exposure separately from loading/model visibility. Future source changes should verify package objects, empty-list suppression, copy semantics and trusted-project updates with isolated resource trees.

### 9. Size and priority

Estimated Rust implementation: **130-240 lines** (estimate, not measured source). **P1** - Resource selection and precedence determine which executable extensions and instructions reach a session.

## SET-020 - Scoped ice.subagents and ice.hooks storage without permissive authority merging

### 1. Title and status

Scoped ice.subagents and ice.hooks storage without permissive authority merging. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/settings-manager.ts:101-124; ice:packages/coding-agent/src/core/settings-manager.ts:558-631; ice:packages/coding-agent/docs/settings.md:1-372.

### 3. What it does

The settings manager stores raw subagent and hook configuration per scope so their owning subsystem can validate and combine permissions deliberately.

### 4. How it works

The ice namespace has a typed storage shape but is not validated by these accessors. Reads and writes select the requested raw scope, preserving the distinction required by restrictive subagent authority rules.

getIceSettingsValue(scope) selects globalSettings or projectSettings, then returns a structuredClone of the ice value when present. setIceSettingsValue rejects project writes unless the project is trusted. It computes the union of old and new immediate namespace keys, assigns a clone of the new value and marks each child key so persistence can remove or update those keys without replacing unrelated settings. It then saves the chosen scope and rebuilds the effective snapshot.

ice.subagents allows enabled, defaultAgent, maxDepth, maxConcurrent, askTimeoutMs, permissions.allow/ask/deny, agents, disabledAgents and compat.claudeAgents/modelAliases. An index signature retains legacy keys; retaining them does not mean honoring removed behavior. ice.hooks allows enabled and definitions:Array<Record<string, unknown>>. The subsystem validators decide allowed types, limits, deprecations and authority; ordinary deepMerge of a settings object is not proof that a child may inherit broader permissions. The manager's generic setSettingValue supports only one- or two-component paths, so deep subagent edits must go through a validated namespace update rather than arbitrary dot traversal.

### 5. Interfaces

IceSettingsValue={subagents?:IceSubagentSettingsValue;hooks?:IceHooksSettingsValue}; getIceSettingsValue(scope:SettingsScope):IceSettingsValue|undefined; setIceSettingsValue(scope, value):void. Stored keys are ice.subagents.* and ice.hooks.*; defaultAgent, disabledAgents and askTimeoutMs are not top-level Settings fields in this frozen source.

### 6. Dependencies

`SET-003`, `SET-007`, `SET-008`, `SET-009`

The SUB area owns subagents/v2/settings.ts and subagents/settings.ts validation, default caps, permissions, legacy diagnostics and runtime consumption. This entry owns only scoped storage/accessor semantics and must not invent those defaults.

Final dependency classification. Consuming integrations: `SUB-005`, `SUB-006`, `SUB-007`, `SUB-010`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

rust:crates/ice-cli/src/settings.rs:28-253 has no typed ice namespace or corresponding scoped accessor. save_settings preserves unknown raw JSON members at rust:crates/ice-cli/src/settings.rs:506-670, which can keep an existing ice object on disk but cannot validate it, edit it with child-key tracking or apply its subagent/hook policies. Native child agents and plugin hooks are separate existing capabilities reviewed in SUB/extensions; unknown-field preservation is not implementation of this configuration contract.

### 8. Port notes

Add raw scoped namespace access with defensive clones and trusted-project writes, then call the SUB validators before installing a runtime policy. Track changed immediate namespace keys and preserve unrelated settings during concurrent edits. Keep legacy keys round-trippable where required but emit their owning subsystem's diagnostics instead of silently enabling them. Future tests should exercise project denial, namespace deletion, nested permission edits and concurrent changes to another settings family.

### 9. Size and priority

Estimated Rust implementation: **80-150 lines** (estimate, not measured source). **P0** - Subagent and hook authority needs validated per-scope configuration before runtime permissions can be trusted.

## SET-021 - Resource reload orchestration, disable flags and precedence

### 1. Title and status

Resource reload orchestration, disable flags and precedence. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/resource-loader.ts:36-65; ice:packages/coding-agent/src/core/resource-loader.ts:172-322; ice:packages/coding-agent/src/core/resource-loader.ts:363-625; ice:packages/coding-agent/test/ice-resource-precedence.test.ts:1-129; ice:packages/coding-agent/test/resource-loader.test.ts:396-440.

### 3. What it does

One resource-loader lifecycle resolves settings, package resources, explicit paths, extensions, prompts, skills, themes and system instructions into a session-visible snapshot.

### 4. How it works

reload rebuilds resource collections after reloading settings for the current trust state. The disable flags primarily suppress defaults, and explicit additions, inline extensions and package resolution have separate gates.

The loader begins with empty results and loaded=false. On subsequent reloads it clears the extension factory cache and resets extension timings. An optional trust resolver triggers the bootstrap in SET-022; settings then reload under the chosen trust. Unless noPackages is set, the package manager resolves ordinary resources and temporary CLI extension sources. noPackages bypasses both calls, preventing ambient package resolution/install hooks; it does not suppress explicit skill/prompt/theme paths or inline extension factories.

The first metadata record for a resolved path is retained. Disabled resources are filtered out. In global-first mode enabled resources are stable-sorted temporary→user→project; ordinary mode preserves the package manager's order. Canonical mergePaths deduplicates physical paths while retaining the first resolved logical spelling. noExtensions still allows the CLI-enabled extension set; noSkills/noPromptTemplates/noThemes still allow explicit/additional paths. The Rust port must not flatten these into 'disable the entire family'. Missing explicitly supplied local paths produce path-specific diagnostics.

The final extension set is loaded before skills/prompts/themes. Overrides can replace each result and its diagnostics; extension source metadata is applied afterward. Skills retain last paths/signature for refresh. Context files, SYSTEM/base/append prompts are computed afterward through separate rules and optional overrides. loaded becomes true after the completed pass. This method does not provide a transactional rollback of arbitrary extension side effects; native atomic plugin swap is a separate useful capability.

### 5. Interfaces

DefaultResourceLoaderOptions includes cwd, agentDir, settingsManager?, eventBus?, additionalExtensionPaths?, additionalSkillPaths?, additionalPromptTemplatePaths?, additionalThemePaths?, extensionFactories?, noExtensions?, noSkills?, noPromptTemplates?, noThemes?, noContextFiles?, noPackages?, systemPrompt?, appendSystemPrompt? and result override callbacks. reload({resolveProjectTrust?}):Promise<void>; getExtensions/getSkills/getPrompts/getThemes/getAgentsFiles/system-prompt getters; extendResources(ResourceExtensionPaths):void.

### 6. Dependencies

`CLI-020`, `CLI-029`, `CLI-030`, `CLI-031`, `CLI-032`, `CLI-033`, `EXT-003`, `EXT-004`, `EXT-006`, `SET-001`, `SET-003`, `SET-008`, `SET-019`

SET-022/023/024 cover bootstrap, attribution and conflicts; SET-025–033 cover each resource family. Package acquisition/filter resolution is a cli-packaging dependency; native Rust need not run Node package managers to implement equivalent scoped resource selection.

### 7. ice-rust today

rust:crates/ice-cli/src/resource_dirs.rs:43-294 and rust:crates/ice-cli/src/session.rs:499-607 already discover, load and deduplicate skills/prompts and compose context. Native reload reruns resources_discover and the loaders at rust:crates/ice-cli/src/session.rs:1395-1452 and rebuilds prompt state at 1491-1557. It is not wholly missing. However startup no_skills/no_prompt_templates wrap the whole block including explicit CLI additions, unlike TS's additive disable semantics. Paths are project-first, metadata is less expressive, there is no corresponding theme-path pass in the read reload blocks, and js/package arrays are empty placeholders at 1412-1413/1430-1431. Native extension swap/keepalive guarantees remain extensions/session-owned strengths, not evidence of full TS resource parity.

### 8. Port notes

Create a native resource-load plan that keeps source scope, enabled state, explicit additions and noPackages-like ambient-resolution controls separate. Reuse current loaders and atomic native plugin lifecycle while adding missing families and consistent global-first selection where required. Preserve detailed diagnostics and avoid executing a resource twice during trust bootstrap. Future verification should cover disabled defaults plus explicit resources, global empty arrays, isolated child noPackages behavior and reload retaining CLI paths.

### 9. Size and priority

Estimated Rust implementation: **260-440 lines** (estimate, not measured source). **P1** - Resource lifecycle and precedence determine loaded code/instructions and must be consistent across startup and reload.

## SET-022 - Trust bootstrap extension reuse without duplicate factory side effects

### 1. Title and status

Trust bootstrap extension reuse without duplicate factory side effects. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/resource-loader.ts:403-426; ice:packages/coding-agent/src/core/resource-loader.ts:576-699; ice:packages/coding-agent/src/core/resource-loader.ts:1019-1042; ice:packages/coding-agent/test/ice-resource-precedence.test.ts:1-129.

### 3. What it does

Global and explicitly supplied trusted extensions can participate in the project-trust decision before project resources load, and those bootstrap instances are reused in the final session.

### 4. How it works

The loader first forces projectTrusted=false and reloads settings so project packages/extensions cannot participate in their own trust decision. It loads global/temporary extensions and inline factories, asks the supplied resolver, then constructs the final ordered set using those same instances and runtime.

Without a trust callback, loadFinalExtensionSet loads the requested paths through the cache, runs inline factories against that runtime, orders them and records conflicts. With a bootstrap result, it indexes non-inline instances by resolvedPath and normalizes error paths into a failedPreloadPaths set. Only paths neither loaded nor failed in bootstrap are attempted afterward, using preTrustExtensions.runtime. Failed bootstrap paths are not retried during the same reload.

Final user extensions follow final extensionPaths order; lookup uses the same resolveExtensionLoadPath normalization. Bootstrap inline instances are retained and not executed again. before-user inline extensions precede user extensions; ordinary inline instances follow. Named inline identity is '<inline:name>' and unnamed identity is '<inline:1>' etc.; hidden and priority metadata are retained. Errors from bootstrap and remaining loads are combined, then conflicts are reported. This protects factory side effects and shared event-bus state from accidental duplication during a single trust decision.

### 5. Interfaces

loadProjectTrustExtensions():Promise<LoadExtensionsResult>; ResourceLoaderReloadOptions.resolveProjectTrust(input:{extensionsResult:LoadExtensionsResult}):Promise<boolean>; private loadFinalExtensionSet(paths, preTrustExtensions). InlineExtension can be a factory or named metadata wrapper; name, hidden and priority='before-user' are host options, not persisted project authority.

### 6. Dependencies

`EXT-003`, `EXT-006`, `EXT-034`, `EXT-057`, `SET-003`, `SET-008`, `SET-021`

EXT-034 owns the actual project_trust event and return contract; SET-003 owns the decision/store policy. Extension factories and package manager code are executable dependencies and must remain outside untrusted project selection during bootstrap.

### 7. ice-rust today

rust:crates/ice-cli/src/session.rs:1847-1857 resolves trust through override/store with an unknown decision accepted by default in the frozen checkout. Its startup/resource selection at 382-539 then uses that boolean; there is no equivalent two-pass global/temporary plugin trust hook with reuse in these callers. EXT-034 separately verifies that the native ProjectTrust tag is not enough without a host dispatch path. Rust's reload keepalive and atomic native registry replacement are worth preserving, but solve unload safety rather than this bootstrap side-effect contract.

### 8. Port notes

Add a pre-project trust phase that loads only authorized global/explicit native extensions and retains their registration instances. Invoke the trust decision through the actual host hook, then add newly allowed project resources without rerunning bootstrap code or retrying its failed paths. Preserve order and source metadata, and keep permission tightening independent of factory cache identity. Future tests should count factory invocations and verify that denied projects never contribute a decision hook.

### 9. Size and priority

Estimated Rust implementation: **170-300 lines** (estimate, not measured source). **P0** - An untrusted project must not authorize its own executable resources, and duplicate initialization can repeat arbitrary extension side effects.

## SET-023 - Resource provenance, canonical deduplication and extension-added paths

### 1. Title and status

Resource provenance, canonical deduplication and extension-added paths. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/source-info.ts:1-41; ice:packages/coding-agent/src/core/resource-loader.ts:363-401; ice:packages/coding-agent/src/core/resource-loader.ts:435-499; ice:packages/coding-agent/src/core/resource-loader.ts:701-935; ice:packages/coding-agent/src/core/prompt-templates.ts:195-267; ice:packages/coding-agent/src/core/skills.ts:137-167.

Named export: `SourceScope` (`ice:packages/coding-agent/src/core/source-info.ts:3`).

Named export: `SourceOrigin` (`ice:packages/coding-agent/src/core/source-info.ts:4`).

### 3. What it does

Resources carry their source, scope, origin and base directory into diagnostics, commands and tools. Later extension additions reuse that metadata instead of guessing ownership from filenames alone.

### 4. How it works

SourceInfo records both a logical resource path and the metadata that explains why it was loaded. Lookup order is explicit and insertion-ordered; it does not select the longest matching directory prefix.

createSourceInfo copies source/scope/origin/baseDir from package metadata; createSyntheticSourceInfo defaults scope='temporary' and origin='top-level'. The loader retains the first metadata for a path. For attribution, synthetic '<...>' paths use the synthetic default. Otherwise the first matching exact-or-ancestor entry in extra extension source maps wins, then an exact ordinary metadata entry, then the first matching ancestor metadata entry. Returned path is the actual resource path even when inherited metadata came from a directory.

Default attribution checks agent skills/prompts/themes/extensions roots first, then project roots, using normalized lexical boundaries. Such resources are local/top-level with user or project scope and that root as baseDir. Other real paths are local/temporary and use the directory itself or the file's parent; stat errors in this fallback are not globally suppressed. Synthetic source names are the first colon-separated token inside angle brackets.

extendResources normalizes paths and optional metadata.baseDir, adds extension source maps, merges with prior paths and reruns affected family loaders. mergePaths canonicalizes resolved paths for first-wins deduplication but returns the retained resolved logical path. Automatic/package skill directory resources map to their SKILL.md when present and transfer metadata to that file. applyExtensionSourceInfo also assigns each extension command/tool the extension's sourceInfo. The caller can therefore distinguish source scope from eventual same-name collision ownership.

### 5. Interfaces

SourceInfo={path:string;source:string;scope:'user'|'project'|'temporary';origin:'package'|'top-level';baseDir?:string}; createSourceInfo(path, PathMetadata); createSyntheticSourceInfo(path,{source, scope?, origin?, baseDir?}). ResourceExtensionPaths contains skillPaths, promptPaths, themePaths entries carrying path and PathMetadata. extendResources is synchronous and does not persist new settings.

### 6. Dependencies

`EXT-031`, `SET-001`, `SET-021`

AGENT-058/062 describe lower-level generic source-tagged loaders; coding-agent SourceInfo is a richer concrete host schema. Prompt/skill/theme collision entries consume provenance. External fs/path/realpath operations define alias behavior.

### 7. ice-rust today

rust:crates/ice-harness/src/prompt_templates.rs:108-163 and rust:crates/ice-harness/src/skills.rs:241-280 already support generic caller-owned provenance wrappers. rust:crates/ice-cli/src/resource_dirs.rs:232-294 emits structured winner/loser collisions, so provenance and collisions are not wholly absent. The CLI loaders consume plain PathBuf lists and do not expose the concrete SourceInfo scope/origin/baseDir schema or the extra-map/exact/ancestor precedence described here. rust:crates/ice-cli/src/session.rs:1395-1443 consumes discovered skill/prompt paths but leaves richer package/JS arrays empty, and has no corresponding theme-path consumer in that pass.

### 8. Port notes

Carry concrete source metadata through native discovery and caller adapters rather than replacing the existing generic library wrappers. Implement stable first-wins canonical path deduplication, explicit directory attribution and metadata propagation to tools/commands. Keep collision winners separate from source provenance and validate extension-supplied paths/base directories. Future checks should distinguish logical symlink paths from canonical identity and test overlapping source prefixes in insertion order.

### 9. Size and priority

Estimated Rust implementation: **150-270 lines** (estimate, not measured source). **P1** - Accurate ownership and scope are needed for resource selection, diagnostics and trust-sensitive extension behavior.

## SET-024 - Extension resource conflict diagnostics without unloading registrations

### 1. Title and status

Extension resource conflict diagnostics without unloading registrations. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/resource-loader.ts:692-699; ice:packages/coding-agent/src/core/resource-loader.ts:1138-1175.

### 3. What it does

The resource loader reports conflicting tool names and incompatible flag types while retaining the loaded extensions. Diagnostics do not themselves roll back registrations or select every command winner.

### 4. How it works

Conflict detection walks the final extension order and remembers the first owner of each tool/flag name. It appends path-specific errors to the load result and keeps all extensions loaded for the downstream registration rules to resolve.

A tool name registered by another extension path produces 'Tool "<name>" conflicts with <owner>'; repeated registration from the same extension path does not. The original owner remains the comparison point after a conflict. Flags share a CLI namespace intentionally: multiple owners may declare the same flag type without conflict, but a later different type produces a diagnostic including name, both types and owner path. The first flag owner/type is retained.

The source comment mentions commands, but detectExtensionConflicts contains no command loop. Command-name validation/collision aliases and shortcut precedence are separate extension-runner behavior (EXT-013/014). Likewise these diagnostics do not prove the duplicate tool was rejected, that a factory rolled back, or that a same-type flag has independent values. The loader runs the check after the final bootstrap-reused/inline ordering is established.

### 5. Interfaces

private detectExtensionConflicts(extensions:Extension[]):Array<{path:string;message:string}>; addExtensionConflictDiagnostics appends {path, error:message} to LoadExtensionsResult.errors. Inputs are ext.tools and ext.flags Maps; flag.type is the declared ExtensionFlag type. No new user setting, persistence file or CLI switch.

### 6. Dependencies

`EXT-011`, `EXT-013`, `EXT-014`, `EXT-015`, `SET-021`, `SET-022`

Extension registration and runner dispatch own actual collision resolution. This entry owns only the loader-level diagnostics and their ordering.

### 7. ice-rust today

The native registry is a real registration/collision boundary reviewed in EXT-006/011/013/015. A native duplicate-registration rejection or native name map is not the same as retaining all extensions and appending these TS tool/flag diagnostics. rust:crates/ice-cli/src/session.rs:1914-1928 applies plugin tools by replacement of a matching harness schema name; downstream replacement behavior must be considered alongside, not inferred from, load diagnostics. Native rust:crates/ice-cli/src/resource_dirs.rs:232-294 collision records apply to skills/prompts rather than this tool/flag contract.

### 8. Port notes

Keep native registration safety guarantees and add a distinct diagnostic pass over accepted extension metadata when compatibility requires reporting all collisions. Define tool selection and shared flag storage in the registry instead of letting diagnostic order implicitly decide it. Reuse structured owner/source data and surface warnings without losing other successfully loaded resources. Future checks should cover same-owner repeats, same-type multi-owner flags, incompatible flags and the absence of command checks in this particular pass.

### 9. Size and priority

Estimated Rust implementation: **70-130 lines** (estimate, not measured source). **P2** - Precise conflict reporting reduces silent resource shadowing and avoids confusing diagnostics with registration enforcement.

## SET-025 - Project instructions, ancestor ordering and linked-worktree shadow suppression

### 1. Title and status

Project instructions, ancestor ordering and linked-worktree shadow suppression. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/resource-loader.ts:84-170; ice:packages/coding-agent/src/core/resource-loader.ts:548-557; ice:packages/coding-agent/test/resource-loader.test.ts:348-579.

### 3. What it does

Loads standing instructions from the user configuration directory and each ancestor of the working directory, with a special rule preventing a nested linked worktree from inheriting a duplicate main-worktree file.

### 4. How it works

The loader chooses the first readable regular instruction file in each directory and places global instructions before root-to-working-directory instructions. These context files are loaded independently of the project-resource trust gate unless noContextFiles disables them or an explicit override changes the result.

Candidate order is AGENTS.md, AGENTS.MD, CLAUDE.md, CLAUDE.MD; AGENTS.override.md is not a TS candidate here. exists/stat/read failures warn and continue to the next candidate. The global file is inserted first, logical path strings are tracked in seenPaths, and ancestor matches are unshifted so the closest directory appears last. A global directory that is also visited in the ancestor walk does not add the same logical path twice.

findShadowedContextFile resolves Git paths and canonicalizes commonGitDir and the worktree root. Only a linked worktree nested under the main checkout can suppress a main-checkout context file, and the main root's .git must resolve to the common Git directory. A worktree-local readable context file shadows the same filename under the main root. Ordinary checkouts, sibling worktrees, bare layouts and submodule git directories retain normal ancestor inheritance; a different main-root context filename is not indiscriminately suppressed. The comparison of the ancestor candidate with the shadowed path is canonical, distinct from the general logical-path seen set.

DefaultResourceLoader calls agentsFilesOverride on the resulting {agentsFiles} object even when noContextFiles produced an empty list. The raw contents become system-prompt context; loading them does not execute their text or confer extension/package trust.

### 5. Interfaces

loadProjectContextFiles({cwd:string, agentDir:string}):Array<{path:string;content:string}>; private loadContextFileFromDir and findShadowedContextFile. DefaultResourceLoaderOptions.noContextFiles and agentsFilesOverride; ResourceLoader.getAgentsFiles(). Filenames are the four candidates above. No persistent cache or file watcher is owned by this function.

### 6. Dependencies

`SET-001`, `SET-021`

SET-027 consumes these context records. The source uses findGitPaths plus realpath-capable path utilities; it does not read a user's private state as part of this audit.

### 7. ice-rust today

rust:crates/ice-harness/src/context_files.rs:30-138 already implements first-file selection and global/root-to-cwd assembly through ExecutionEnv. Its candidate list adds AGENTS.override.md, its file-kind check requires FileKind::File, read failures are quiet, and it has no linked-worktree suppression. The helper named canonical at rust:crates/ice-harness/src/context_files.rs:164-185 filters out ParentDir components instead of resolving them and does not resolve symlinks. More significantly, rust:crates/ice-cli/src/session.rs:1445-1462 gates the whole context load on project_trusted, so an untrusted project also loses global context; TS ice:packages/coding-agent/src/core/resource-loader.ts:548-557 does not apply that gate. The native context formatter escapes paths, which should be retained.

### 8. Port notes

Specify the intended trust policy for standing instructions explicitly, then align the caller so global context is not accidentally disabled along with executable project resources. Add the Git-layout-aware same-filename suppression using filesystem paths already known to the session, preserve deterministic ancestor order and warnings, and replace the misleading lexical ParentDir filter with correct normalization/canonicalization as appropriate. Keep native path escaping. Future source-focused checks should distinguish nested versus sibling worktrees, same versus different filenames, unreadable first candidates, symlinks and noContextFiles overrides.

### 9. Size and priority

Estimated Rust implementation: **180-320 lines** (estimate, not measured source). **P1** - Duplicated, missing or differently ordered standing instructions change every model request and can defeat user-selected global instructions.

## SET-026 - SYSTEM.md, APPEND_SYSTEM.md and global base-template source resolution

### 1. Title and status

SYSTEM.md, APPEND_SYSTEM.md and global base-template source resolution. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/resource-loader.ts:67-82; ice:packages/coding-agent/src/core/resource-loader.ts:559-595; ice:packages/coding-agent/src/core/resource-loader.ts:1097-1127; ice:packages/coding-agent/test/resource-loader.test.ts:1008-1170; ice:packages/coding-agent/test/ice-resource-precedence.test.ts:1-129.

### 3. What it does

Selects replacement, appended and templated system-prompt sources with explicit input precedence, project trust and globalFirst behavior.

### 4. How it works

The loader resolves explicit prompt input or discovers a permitted configuration file, and records both the resulting text and any existing source path. Replacement SYSTEM.md, appended APPEND_SYSTEM.md and the global system-base.md template are separate inputs with different empty-value and fallback rules.

For SYSTEM.md and APPEND_SYSTEM.md discovery, globalFirst chooses an existing global file first. Otherwise a trusted project's file wins, followed by the global file. Untrusted project files are excluded while global files remain eligible. SYSTEM source selection uses nullish coalescing, so an explicit empty string suppresses discovery, although resolvePromptInput then returns undefined for that falsy input. A truthy input naming an existing file is read as UTF-8; read failure prints a warning and returns the original input literally. A nonexisting path-shaped string is also literal text. Existence alone determines recorded source paths, so an unreadable existing source can still be listed.

Append input is an optional array. An explicit empty array is truthy and prevents discovery; otherwise one discovered append file becomes a one-element array. Each source is resolved independently, undefined results are removed, and appendSystemPromptOverride can replace the text array. systemPromptOverride receives the optional resolved replacement text. Source-path metadata still describes input files, not arbitrary override output.

Only agentDir/system-base.md is read as a base template. Its trailing whitespace is removed, a blank result leaves the built-in template selected, and read errors warn without replacing the built-in fallback. No project-level base template is read. This file is a template with the five placeholders in SET-027, while SYSTEM.md is a replacement prompt and APPEND_SYSTEM.md is appended text.

### 5. Interfaces

DefaultResourceLoaderOptions.systemPrompt?:string, appendSystemPrompt?:string[], systemPromptOverride and appendSystemPromptOverride; getSystemPrompt(), getSystemPromptSourcePath(), getAppendSystemPrompt(), getAppendSystemPromptSourcePaths(), getBaseSystemPromptTemplate():{path:string;content:string}|undefined. Configuration filenames SYSTEM.md, APPEND_SYSTEM.md and system-base.md; globalFirst and projectTrusted affect discovery only as described.

### 6. Dependencies

`SET-001`, `SET-003`, `SET-008`, `SET-021`

SET-027 owns final composition and template substitution. CLI option spelling and argument parsing belong to CLI/packaging; no CLI name is inferred from these internal option fields.

### 7. ice-rust today

rust:crates/ice-cli/src/session.rs:1491-1557 recomposes native base/append/context, but the explicit system prompt is used as literal text, append file reads fall back to the literal value, and an empty append vector permits discovered fallback. Native discovery is guarded by project_trusted, which also excludes the global SYSTEM.md/APPEND_SYSTEM.md source in that branch. rust:crates/ice-cli/src/session.rs:382-650 contains the startup counterpart; rust:crates/ice-harness/src/system_prompt.rs:48-77 composes strings but has no system-base.md discovery or placeholder engine. Thus the native existence of SYSTEM/APPEND support does not establish the TS input-resolution or globalFirst contract.

### 8. Port notes

Represent absent, explicit empty and nonempty source inputs distinctly. Introduce one source resolver used at startup and reload, retain file-origin metadata, and apply trust only to project-owned discovered sources. Add global base-template loading with trailing-only trimming and fallback. Decide whether a read failure should retain TS's literal-path fallback or become an explicit diagnostic; document any safer intentional difference rather than silently treating it as parity.

### 9. Size and priority

Estimated Rust implementation: **160-280 lines** (estimate, not measured source). **P1** - These files control the system prompt, and current trust/empty-value differences silently select a different prompt.

## SET-027 - Dynamic coding system prompt, tool metadata and one-pass base placeholders

### 1. Title and status

Dynamic coding system prompt, tool metadata and one-pass base placeholders. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/system-prompt.ts:8-183; ice:packages/coding-agent/src/core/skills.ts:397-432.

### 3. What it does

Builds the coding-agent system prompt from the selected tools, tool snippets, deduplicated guidelines, optional templates, project instructions and model-visible skill metadata.

### 4. How it works

A truthy customPrompt replaces the default scaffold but still receives append text, context files, eligible skills and the working-directory line. Otherwise the builder renders a base template in one pass and then appends those same contextual sections in a fixed order.

The default selected tools are read, bash, edit and write. Only selected tool names with a truthy caller-supplied snippet appear under Available tools; without such snippets the list is '(none)'. The builder does not infer descriptions from a hard-coded tool list. It adds the bash file-exploration guideline only when bash is selected and grep, find and ls are all absent. Additional guidelines are trimmed, blank values dropped and exact duplicates suppressed in insertion order, followed by the two built-in concise/path guidelines.

The base template is options.baseTemplate ?? DEFAULT_BASE_TEMPLATE, so the builder itself accepts an explicitly empty template even though file-based loading drops blank files. A single replace pass recognizes {{tools}}, {{guidelines}}, {{readmePath}}, {{docsPath}} and {{examplesPath}} through the word-key matcher; unknown placeholders remain literal and substituted values are not expanded again. Documentation paths come from installation configuration. The static scaffold contains explicit documentation-navigation instructions as part of its text.

Append text is added only when truthy and retains its whitespace. Context records are emitted in their supplied order inside project_context/project_instructions; TS inserts paths and file contents without XML escaping. Skills are listed only when read is selected (or selectedTools is absent in the customPrompt branch), and disabled model-invocation skills are filtered by the formatter. Both branches end with Current working directory after replacing backslashes with forward slashes. Context and append content are not interpreted as further template syntax.

### 5. Interfaces

BuildSystemPromptOptions {customPrompt?, selectedTools?, toolSnippets?, promptGuidelines?, baseTemplate?, appendSystemPrompt?, cwd, contextFiles?, skills?}; buildSystemPrompt(options):string; DEFAULT_BASE_TEMPLATE. Placeholder names and project_context/project_instructions/available_skills wrapper strings are externally observable prompt text. No network request or persistent write occurs in this builder.

### 6. Dependencies

`EXT-035`, `SET-001`, `SET-025`, `SET-026`, `SET-031`

EXT-035 supplies extension tool snippets/guidelines; callers decide the selected tool set. SESSION resource refresh and turn construction consume the composed text and must preserve any active per-turn override.

Final dependency classification. Consuming integrations: `SESSION-019`, `SESSION-023`, `SESSION-028`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

rust:crates/ice-cli/src/session.rs:68-131 builds a static native default scaffold with fixed tool descriptions and native reasoning-replay/working-state instructions. rust:crates/ice-harness/src/system_prompt.rs:48-77 has the useful base→append→context→skills order, but trims base and append, delegates skill formatting and has no dynamic five-placeholder template or guideline deduplication. rust:crates/ice-harness/src/context_files.rs:146-161 escapes path attributes, unlike TS; retain that protection. Native cwd placement is inside the base scaffold rather than an unconditional final line. Missing dynamic composition should not be confused with the absence of a native system prompt.

### 8. Port notes

Add a typed composition input carrying the active tools, snippets and guidelines, and a one-pass named-placeholder renderer. Keep the native reasoning instructions and XML path escaping, documenting their deliberate differences, while making source selection and whitespace decisions explicit. Rebuild the base prompt when tools or skills change, then reapply active session/turn overrides. Future verification should include no snippets, tools lacking read, unknown placeholders, placeholder text inside values, custom empty/truthy prompts and Windows-style cwd text.

### 9. Size and priority

Estimated Rust implementation: **180-310 lines** (estimate, not measured source). **P1** - Tool availability and resource changes must be reflected accurately in model instructions; static text can describe unavailable tools or omit extension guidance.

## SET-028 - Coding prompt-template discovery, metadata and source-aware collision handling

### 1. Title and status

Coding prompt-template discovery, metadata and source-aware collision handling. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/prompt-templates.ts:12-19; ice:packages/coding-agent/src/core/prompt-templates.ts:105-264; ice:packages/coding-agent/src/core/resource-loader.ts:768-790; ice:packages/coding-agent/src/core/resource-loader.ts:1044-1068; ice:packages/coding-agent/test/prompt-templates.test.ts:1-625.

### 3. What it does

Loads Markdown slash-command templates with descriptions, optional argument hints, absolute file paths and source attribution, then applies first-name-wins diagnostics in the resource loader.

### 4. How it works

Prompt discovery reads direct Markdown children of each selected directory and preserves the discovery order used by the resource loader. Individual parse/read failures are skipped, while name collisions are reported by a separate resource-loader pass that retains the first template.

With includeDefaults true, user prompts precede project prompts, followed by explicit paths in caller order. DefaultResourceLoader normally supplies its already-resolved path list with includeDefaults false. Explicit paths are resolved against cwd with trimming and supported home/file-URL normalization. Missing paths and failed reads are ignored by this low-level loader. Directory scanning is nonrecursive, uses inNodeReaddirOrder, accepts .md regular files and symlinks to regular files, skips broken links, and returns any templates collected before a directory-level exception.

Template name is the filename with its final .md suffix removed; a frontmatter name does not replace it. Description uses truthy frontmatter.description or the first nonblank body line, truncated to 60 JavaScript UTF-16 code units plus '...' when longer. A truthy argument-hint is retained as argumentHint. Content is the parsed body, with an absolute filePath and SourceInfo. Source classification uses lexical containment under global/project prompt roots; other explicit files get a local source base directory.

The low-level function returns duplicates. DefaultResourceLoader.dedupePrompts keeps the first name, silently ignores a repeat of the exact same filePath and reports differing files with winnerPath/loserPath. Overrides run after deduplication and can supply their own templates/diagnostics; final source metadata is then enriched from package/extension ownership according to SET-023. Slash expansion is a separate contract in SET-029.

### 5. Interfaces

PromptTemplate {name, description, argumentHint?, content, sourceInfo, filePath}; LoadPromptTemplatesOptions {cwd, agentDir, promptPaths, includeDefaults}; loadPromptTemplates(options):PromptTemplate[]. DefaultResourceLoader.getPrompts() exposes {prompts, diagnostics}; collision.resourceType='prompt'. Frontmatter keys description and argument-hint; resource setting prompts:string[].

### 6. Dependencies

`SET-019`, `SET-021`, `SET-023`

SET-029 consumes the resulting ordered templates; TUI autocomplete and RPC get_commands expose metadata. External YAML frontmatter and filesystem ordering helpers must keep the source semantics.

### 7. ice-rust today

rust:crates/ice-harness/src/prompt_templates.rs:1-319 already loads Markdown templates, supports sourced wrappers and emits structured diagnostics. It sorts directory entries lexicographically; the basic PromptTemplate holds name/description/content without TS argumentHint, filePath or SourceInfo, and description truncation counts Rust characters rather than JavaScript UTF-16 units. Native frontmatter handling also retains an explicitly empty description instead of the TS truthy fallback. rust:crates/ice-cli/src/resource_dirs.rs:232-294 supplies native resource collision records, but CLI source precedence and no_prompt_templates gating differ as described in SET-021. Existing discovery is partial parity, not a missing subsystem.

### 8. Port notes

Extend the native command-template metadata so UI/RPC consumers can show argument hints and source ownership without reparsing files. Align or explicitly document ordering, duplicate-path suppression and description semantics; reuse SET-023 source information. Keep useful native diagnostics rather than reproducing silent failures without explanation. Future checks should cover duplicate names from several sources, a repeated path, symlink files, nested directories, blank descriptions and astral Unicode near the truncation boundary.

### 9. Size and priority

Estimated Rust implementation: **170-290 lines** (estimate, not measured source). **P2** - Accurate command discovery and metadata improve slash-command behavior without requiring replacement of the existing native loader.

## SET-029 - Coding slash-template argument parsing and nonrecursive fallback substitution

### 1. Title and status

Coding slash-template argument parsing and nonrecursive fallback substitution. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/prompt-templates.ts:25-103; ice:packages/coding-agent/src/core/prompt-templates.ts:270-287; ice:packages/coding-agent/test/prompt-templates.test.ts:1-625.

### 3. What it does

Expands a matching coding-agent slash template using quote-aware argument splitting, positional/all-argument placeholders, slices and fallback values without recursively interpreting inserted text.

### 4. How it works

Expansion applies only to a leading slash command that exactly matches the first template name, otherwise preserving the original input. A single placeholder replacement pass processes the original template text so argument or fallback strings containing placeholder syntax remain literal.

parseCommandArgs toggles single or double quotes, preserves the other quote character while quoted, splits on JavaScript whitespace outside quotes and has no backslash-escape syntax. Empty quoted arguments are dropped because only a nonempty current buffer is emitted; an unmatched opening quote simply leaves the accumulated argument at end. expandPromptTemplate uses an exact case-sensitive name from /^\/([^\s]+)(?:\s+([\s\S]*))?$/ and passes the optional multiline argument tail to this parser.

substituteArgs supports $1, $2 and larger decimal positions; $ARGUMENTS and $@ join arguments with spaces. ${N:-default}, ${ARGUMENTS:-default} and ${@:-default} use the fallback when the selected value is missing or empty. Defaults stop at the first closing brace and are not nested expressions. ${@:N} and ${@:N:L} use one-based slicing, treating N=0 as 1; length 0 yields no arguments. Missing positional values, including $0, become empty strings. The one regex pass recognizes alternatives together, so no later positional, slice or all-argument pass can rewrite a replacement value.

This is the current coding-agent implementation, separate from the older packages/agent harness substitution audited in AGENT-057. Their algorithm must not be conflated merely because both export similarly named helpers.

### 5. Interfaces

parseCommandArgs(argsString:string):string[]; substituteArgs(content:string, args:string[]):string; expandPromptTemplate(text:string, templates:PromptTemplate[]):string. Supported syntax is the exact placeholder family above; no shell is invoked and no environment-variable expansion occurs.

### 6. Dependencies

`SET-028`

AGENT-057 documents the separate lower-level harness implementation and its Rust Unicode defect. SESSION input processing and EXT-032 user-message dispatch decide whether this expansion runs for a submitted message.

### 7. ice-rust today

rust:crates/ice-harness/src/prompt_templates.rs:320-479 implements the older sequential scanner: positional substitution, slice substitution, then all-argument replacements. It does not support the coding-agent fallback forms, can re-expand placeholder-like text inserted by an earlier pass and treats only space/tab as unquoted argument separators. Both byte scanners push individual UTF-8 bytes as Rust chars, the separately catalogued AGENT-057 corruption defect. In slice substitution, start + length at rust:crates/ice-harness/src/prompt_templates.rs:450-455 is unchecked usize addition; extreme parsed values can overflow before the slice bounds are built. This is a source-level failure condition, not an executed reproduction.

### 8. Port notes

Use a Unicode-safe one-pass parser that recognizes the complete coding-agent placeholder grammar and appends replacement values directly. Bound or saturate numeric slice arithmetic before indexing; do not preserve byte-to-char corruption or unchecked addition. Specify empty-quote and whitespace behavior deliberately, and share code only if the harness and coding-agent contracts are intentionally unified. Future checks should include literal $@ inside arguments/defaults, fallback values, zero/out-of-range slices, huge integers, multiline whitespace and non-ASCII text.

### 9. Size and priority

Estimated Rust implementation: **120-210 lines** (estimate, not measured source). **P1** - Template substitution can currently alter user content, miss fallback syntax or panic on extreme indices; it is a common input path.

## SET-030 - Skill discovery, ignore rules and canonical-file/name deduplication

### 1. Title and status

Skill discovery, ignore rules and canonical-file/name deduplication. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/skills.ts:17-66; ice:packages/coding-agent/src/core/skills.ts:130-276; ice:packages/coding-agent/src/core/skills.ts:434-549; ice:packages/coding-agent/src/core/resource-loader.ts:701-759; ice:packages/coding-agent/docs/skills.md:1-241.

### 3. What it does

Finds skills in ordered source roots, applies nested ignore files and preserves one winner per canonical file and skill name while reporting source collisions.

### 4. How it works

The loader treats a directory containing a regular SKILL.md as a skill root and stops recursion there. Otherwise it accepts direct Markdown files only at the requested root and recursively searches child directories for their own skill roots.

Discovery skips dot entries and node_modules, follows symlinks through stat and skips broken links. A SKILL.md stops recursion even when parsing or validation rejects the skill. A directory read failure returns collected results without throwing. Directory traversal uses inNodeReaddirOrder. The recursive discovery walk does not maintain a visited-directory set, so symlink cycles are not explicitly prevented by this algorithm; the separate signature walk in SET-032 does have cycle tracking.

.gitignore, .ignore and .fdignore are read in that order for each directory. Rules are prefixed with the POSIX relative-directory path, leading slash is stripped, and negation/escaped leading exclamation handling is preserved before passing rules to the ignore package. Empty/comment lines are ignored; file-read failures are silent. The same matcher is passed through the recursion with prefixed rules rather than resetting ignore state at each child.

With includeDefaults true, user skills are loaded before project skills and explicit paths follow. Explicit files/directories are normalized against cwd; missing paths, non-Markdown file targets and read/parse failures produce warning diagnostics. addSkills first checks the canonical real file path, silently suppressing an already accepted file, then selects the first skill name. A different file with that name yields a collision carrying winnerPath/loserPath. Losing paths are not added to the accepted real-path set, so repeating a losing file can repeat its diagnostic. DefaultResourceLoader supplies resolved path order with includeDefaults false and can enrich metadata or override results after this step.

### 5. Interfaces

LoadSkillsFromDirOptions {dir, source}; loadSkillsFromDir(options):LoadSkillsResult; LoadSkillsOptions {cwd, agentDir, skillPaths, includeDefaults}; loadSkills(options):{skills, diagnostics}. Ignore filenames .gitignore/.ignore/.fdignore; SKILL.md roots and direct .md files; collision.resourceType='skill'. Resource setting skills:string[] and noSkills/additionalSkillPaths admission are described in SET-019/021.

### 6. Dependencies

`SET-019`, `SET-021`, `SET-023`

SET-031 owns metadata validation/listing. External dependencies are the filesystem/path utilities, YAML frontmatter and the ignore package; source discovery itself does not execute skill instructions.

### 7. ice-rust today

rust:crates/ice-harness/src/skills.rs:1-280; rust:crates/ice-harness/src/skills.rs:296-567 provides a substantive native skill loader, sourced wrappers, recursion, warnings and deduplication. It sorts names lexicographically, clones a native IgnoreStack for child traversal and uses a handwritten ignore matcher instead of the TS ignore package. Full matcher parity is separately assessed in AGENT-060; this entry does not claim unreviewed matcher branches were read again. rust:crates/ice-cli/src/resource_dirs.rs:1-294 supplies native roots/collision handling with project-first precedence. rust:crates/ice-cli/src/session.rs:1408-1427 gates no_skills over the whole native load rather than retaining explicitly added skills as TS does. Existing loaders should be extended rather than replaced.

### 8. Port notes

Unify source precedence with SET-021 and retain explicit-path behavior. Use a tested gitignore-compatible parser or specify the native subset precisely; add canonical-directory cycle protection instead of copying the TS discovery hazard. Preserve first-name/canonical-file winner rules and structured source collisions. Future checks should include invalid SKILL.md blocking child recursion, symlink aliases/cycles, nested negated rules, repeated losing paths and global/project/explicit order.

### 9. Size and priority

Estimated Rust implementation: **190-340 lines** (estimate, not measured source). **P1** - Skill discovery order and ignore semantics determine which instructions are exposed and whether a source is silently shadowed.

## SET-031 - Skill frontmatter validation and model-visible XML listing

### 1. Title and status

Skill frontmatter validation and model-visible XML listing. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/skills.ts:11-15; ice:packages/coding-agent/src/core/skills.ts:68-128; ice:packages/coding-agent/src/core/skills.ts:278-326; ice:packages/coding-agent/src/core/skills.ts:397-432; ice:packages/coding-agent/docs/skills.md:1-241.

### 3. What it does

Creates validated skill metadata without retaining the instruction body, and exposes only model-invocable skills through an escaped system-prompt listing.

### 4. How it works

Skill metadata comes from frontmatter and the containing directory, with warnings for most invalid fields and rejection only when the description is missing or blank. The system-prompt formatter lists name, description and location for visible skills so the model can read the full file when relevant.

Name uses a truthy frontmatter.name or the parent directory name. Warnings cover more than 64 JavaScript UTF-16 code units, characters outside lowercase ASCII letters/digits/hyphens, leading/trailing hyphens and consecutive hyphens. The implementation does not require the name to equal the parent directory. Description is required and must contain non-whitespace text; more than 1,024 UTF-16 code units is a warning but still loads. Parser/read exceptions become warning diagnostics and no skill. Other frontmatter keys are not stored on the public Skill record.

The public record retains name, description, absolute filePath, baseDir, SourceInfo and disableModelInvocation, with the last field true only for the boolean frontmatter value true. The parsed Markdown body is not held in the coding-agent Skill object. Thus refresh updates metadata and the listing, while explicit invocation must obtain the instruction content from its own file path.

formatSkillsForPrompt drops disabled model-invocation records and returns an empty string when none remain. Otherwise it emits the exact available_skills/skill/name/description/location XML structure, escaping ampersand, angle brackets, double quote and apostrophe. Its leading prose tells the model to use read and resolve relative references against the skill directory. The system-prompt caller additionally gates the listing on read-tool availability; disabling model invocation does not itself remove explicit /skill:name commands.

### 5. Interfaces

SkillFrontmatter {name?, description?,'disable-model-invocation'?:boolean,[key:string]:unknown}; Skill {name, description, filePath, baseDir, sourceInfo, disableModelInvocation}; formatSkillsForPrompt(skills):string. Name limit 64 and description limit 1024 are UTF-16 code-unit checks, not Rust Unicode scalar or byte counts. enableSkillCommands is a separate settings switch.

### 6. Dependencies

`SET-023`, `SET-030`

SET-027 supplies the read-tool gate; TUI/SESSION own explicit skill command expansion. No remote service is required for metadata loading.

### 7. ice-rust today

rust:crates/ice-harness/src/skills.rs:296-567 implements native metadata parsing, validation and system-prompt formatting. Native Skill retains content and represents disable_model_invocation as Option<bool>; its name validation additionally warns when the frontmatter name differs from the parent directory and counts Rust characters. These are meaningful differences from the coding-agent record even where they align with an older harness. rust:crates/ice-harness/src/system_prompt.rs:48-77 already uses the native listing, with caller-side filtering documented at lines 19-23. An existing XML listing is therefore partial parity; the missing work is schema/validation/source/refresh compatibility, not wholesale listing creation.

### 8. Port notes

Decide whether native Skill should retain both instruction content and coding metadata, and prevent stale cached content from surviving metadata refresh. Carry baseDir and SourceInfo to command/UI consumers, align validation units or document their deliberate difference, and preserve XML escaping. Verify that disable-model-invocation affects the listing while enableSkillCommands controls explicit commands independently. Future checks should cover blank/oversized descriptions, mismatched directory names, astral Unicode and hidden skills.

### 9. Size and priority

Estimated Rust implementation: **90-170 lines** (estimate, not measured source). **P2** - Schema and visibility mismatches confuse skill selection and command discovery, but the native formatter and validator already exist.

## SET-032 - Skill signatures, automatic refresh and prompt/autocomplete propagation

### 1. Title and status

Skill signatures, automatic refresh and prompt/autocomplete propagation. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/skills.ts:328-387; ice:packages/coding-agent/src/core/resource-loader.ts:737-766; ice:packages/coding-agent/src/core/agent-session.ts:1425-1436; ice:packages/coding-agent/src/core/agent-session.ts:3210-3228; ice:packages/coding-agent/src/modes/interactive/interactive-mode.ts:891-915; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:1770-1779.; ice:packages/coding-agent/test/suite/regressions/2753-reload-stale-resource-settings.test.ts:25-103 (runtime reload applies edited prompt exclusions; read, not run).

### 3. What it does

Detects changes in skill source trees without reading every file body, reloads metadata when the signature changes and rebuilds session/UI command state.

### 4. How it works

The resource loader compares a metadata signature of the last resolved skill paths with the previous signature and reloads only when it changes. Session and UI consumers then update the composed base prompt and autocomplete, so newly added or edited skill metadata can become visible during an existing session.

The signature includes each root path, directory modification time, and every traversed file's path/mtimeMs/size. Dot entries are skipped except the three ignore filenames, node_modules is skipped, and non-Markdown files still contribute. Missing/stat-failed paths get a missing marker; unreadable directories get an unreadable marker. Entries are locale-sorted by name. Symlink targets are statted, and canonical visited directories prevent recursive cycles within each root; each root gets its own visited set. It is metadata-based, so an edit preserving both size and mtime can escape detection.

refreshSkillsIfChanged returns false before initial load or for an equal signature. On change it reruns updateSkillsFromPaths, source enrichment, diagnostics and the signature update. AgentSession.refreshSkills rebuilds the base prompt, retains an active turn override when present, updates the agent system prompt and emits resources_changed. Prompt submission invokes refresh after its isStreaming early-return guard.

InteractiveMode starts a 5,000 ms interval that calls session.refreshSkills and rebuilds autocomplete after a change; its callback catches and logs failures. This periodic callback has no idle/isStreaming guard. RPC get_commands also refreshes unconditionally before enumerating commands. Therefore the earlier report's 'poll while idle' wording is too broad: one prompt path is guarded, while the periodic and RPC paths are not. Disposal stops the interactive interval. This is signature polling, not a filesystem notification subscription.

### 5. Interfaces

computeSkillsSignature(paths:string[]):string; ResourceLoader.refreshSkillsIfChanged():boolean; AgentSession.refreshSkills():boolean; resources_changed session event. Interactive interval 5000 ms; RPC command get_commands. Last skill path/signature state is in memory; no signature persistence file or standalone user setting.

### 6. Dependencies

`SET-021`, `SET-027`, `SET-030`, `SET-031`

SESSION-023/028 integrate resources into active sessions. TUI owns autocomplete presentation and RPC/remote owns the command response schema. Active prompt overrides must survive refresh.

Final dependency classification. Consuming integrations: `RPC-022`, `RPC-031`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

rust:crates/ice-cli/src/fs_watch.rs:1-162 has a generic modification-time polling watcher with a 1,000 ms default interval, callback wiring and stop flag. It does not compute this recursive skill signature or itself rebuild the active skill catalog. rust:crates/ice-cli/src/session.rs:1380-1557 explicitly reloads extension/skill/prompt/context resources and recomposes the native prompt, but there is no corresponding TS signature-driven skill-only refresh path in these callers. The fs_watch Drop implementation stops the flag despite an older module comment saying drop does not stop; the actual implementation is the evidence. A generic watcher is useful infrastructure rather than completed skill hot reload.

### 8. Port notes

Add a cancellable signature or filesystem-event service around the resolved skill roots and route successful changes through a single session resource-update operation. Rebuild base prompt and command metadata while preserving an active turn override; decide and document whether updates are deferred during streaming instead of inheriting an accidental race. Retain diagnostics and stop the watcher on session disposal. Future checks should include add/remove/ignore-file changes, equal metadata edits, symlink cycles, active-turn overrides and RPC enumeration after a change.

### 9. Size and priority

Estimated Rust implementation: **180-310 lines** (estimate, not measured source). **P1** - Long-running sessions otherwise expose stale skills, and refresh ordering can overwrite the active system prompt.

## SET-033 - Theme-file discovery, source ownership and first-name collision diagnostics

### 1. Title and status

Theme-file discovery, source ownership and first-name collision diagnostics. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/core/resource-loader.ts:539-546; ice:packages/coding-agent/src/core/resource-loader.ts:792-811; ice:packages/coding-agent/src/core/resource-loader.ts:938-1017; ice:packages/coding-agent/src/core/resource-loader.ts:1070-1095.

### 3. What it does

Collects file-based themes from resolved package/global/project/explicit sources, reports invalid paths and duplicate names, and associates each theme with its origin.

### 4. How it works

Theme discovery reads JSON files from the ordered resource paths and isolates each file's load failure as a warning. A separate name-deduplication pass keeps the first theme and reports the source path of later collisions before optional caller overrides and source enrichment.

With includeDefaults true the low-level helper visits agentDir/themes then cwd/.ice/themes; the resource loader normally supplies already-resolved paths with includeDefaults false. Explicit directories are scanned nonrecursively in inNodeReaddirOrder. Regular .json files and symlinks resolving to regular files are accepted; broken symlinks are skipped. Missing explicit paths warn, non-JSON/non-file paths warn, and directory/stat/parser exceptions become diagnostics while other themes continue loading. Additional missing theme paths also receive the resource-loader's explicit-path error diagnostic.

Each file is delegated to loadThemeFromPath, whose JSON/color/variable validation belongs to TUI. The deduper selects by theme.name; when two different source paths share a name, it records winnerPath/loserPath. Repeating the same source path is ignored. Missing source paths use the builtin marker when forming collision details, and undefined names follow the same Map-key rule rather than gaining generated identities. themesOverride receives the deduplicated list and accumulated diagnostics, after which package/extension SourceInfo is attached from sourcePath.

noThemes removes automatic sources but explicitly supplied additional paths can still be loaded through the resource orchestration contract. Merely collecting extension themePaths does not prove a host can parse, list or select those themes.

### 5. Interfaces

DefaultResourceLoader.getThemes():{themes:Theme[], diagnostics:ResourceDiagnostic[]}; private loadThemes(paths, includeDefaults), loadThemesFromDir, loadThemeFromFile and dedupeThemes. Resource setting themes:string[]; noThemes/additionalThemePaths/themesOverride. Theme.sourcePath/sourceInfo and collision.resourceType='theme'; JSON schema details are owned by the TUI audit.

### 6. Dependencies

`EXT-031`, `SET-019`, `SET-021`, `SET-023`, `TUI-001`, `TUI-002`, `TUI-003`

TUI supplies loadThemeFromPath and the actual palette/schema/switching behavior. EXT-031 owns the resources_discover contribution boundary.

### 7. ice-rust today

rust:crates/ice-extensions/src/resources.rs:37-101 collects theme_paths along with skills and prompts, and rust:crates/ice-cli/src/session.rs:660-678 reports the discovery counts. That does not provide the described file catalog. Native UI has genuine theme support: rust:crates/ice-cli/src/interactive_tui.rs:972-984 selects dark/light/monochrome presets, and rust:crates/ice-tui/src/theme.rs:343-386 owns current palette state and revision invalidation. The Rust resource directory/session paths inspected for SET-021 load skills/prompts but do not build this source-aware JSON theme registry. The older resources.rs comment 'no theme system yet' is stale if read as a statement about all native palettes.

### 8. Port notes

Connect resolved theme paths to a native theme-file parser/catalog, using the existing palette manager and cache-revision mechanism. Carry source metadata and structured diagnostics, honor explicit paths under noThemes, and make first-wins collision policy visible in the selector. Keep JSON schema compatibility and preset support in the TUI implementation rather than inventing a second palette engine in resource loading. Future checks should cover symlink files, same-name different-source themes, repeated paths, parser failures and package/extension contributions.

### 9. Size and priority

Estimated Rust implementation: **160-280 lines** (estimate, not measured source). **P2** - Custom and package-provided themes are not selectable through the existing preset-only path, despite accepting theme contribution names.

## Exact supporting contracts

## Source-reading closure

All 17 assigned source/document files and the 10 identified core settings/resource/trust/config test bodies were read. Their full hashes, including the completed 1–1170 resource-loader test, are recorded in evidence/qa-settings-test-closure.json. Source assertions were inspected; no tests or application code were executed. UI selector/style, RPC settings and provider settings consumer suites are assigned to those area audits and are part of the final global closure. The coordinator's final acceptance artifacts record cross-area dependency resolution and active-checkout validation.

