# CLI, packaging and startup

Complete source-derived CLI and packaging specification, including startup, credential commands, migrations, npm/Git/local resource backends, native Cargo/prebuilt preservation and updater defects. No application, test, installer or updater was executed. The coordinator's final acceptance artifacts record global and active-checkout reconciliation.

## CLI-001 - Executable identity, stock launcher and Bun bootstrap ordering

### 1. Title and status

Executable identity, stock launcher and Bun bootstrap ordering. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/cli.ts:1-21; ice:packages/coding-agent/src/bun/cli.ts:1-15; ice:packages/coding-agent/src/bun/register-bedrock.ts:1-4; ice:packages/coding-agent/src/bun/restore-sandbox-env.ts:1-36; rust:crates/ice-cli/src/bin/icers.rs:1-21.

### 3. What it does

Starting Ice establishes its process identity and runtime adapters before invoking the main command. The distribution has separate Node/source and compiled Bun launch paths, whose ordering affects provider availability and inherited environment state.

### 4. How it works

Each executable establishes process identity before handing control to the main application. The Bun build also installs runtime-specific provider adapters and repairs only a completely empty environment.

The stock cli.ts imports legacy bootstrap before main, sets process.title to APP_NAME, assigns ICE_CODING_AGENT='true' and AI_AGENT='ice', replaces process.emitWarning with a no-op, configures the global HTTP dispatcher, then calls main(process.argv.slice(2)) without a top-level await. The bootstrap module itself only assigns ICE_CODING_AGENT; it does not translate historical environment names (ice:packages/coding-agent/src/cli.ts:1-21; ice:packages/coding-agent/src/core/legacy-compat/bootstrap.ts:1-5).

The Bun launcher first sets title and suppresses process warnings, registers Bun OAuth flows, invokes restoreSandboxEnv, awaits registration of the injected Bun Bedrock adapter, and only then dynamically imports the ICE distribution entrypoint. The Bedrock registration calls setInjectedBedrockModule with the Bun module; provider implementation remains AI/AUTH ownership (ice:packages/coding-agent/src/bun/cli.ts:1-15; ice:packages/coding-agent/src/bun/register-bedrock.ts:1-4).

restoreSandboxEnv is a narrowly gated workaround: return unless process.versions.bun is truthy and Object.keys(process.env).length is zero. Otherwise read /proc/self/environ as UTF-8, split NUL records, take the first '=' only when its index is positive, and assign each exact name/value into process.env. All read failures are ignored. There is no explicit platform check; unavailable /proc is handled by that catch. The candidate called an indexed environment read at line 30 is actually an assignment. Existing source tests cover non-Bun, already-populated environment and successful NUL restoration; they were read, not executed (ice:packages/coding-agent/src/bun/restore-sandbox-env.ts:17-36; ice:packages/coding-agent/test/restore-sandbox-env.test.ts:11-77).

### 5. Interfaces

Stock executable passes argv excluding executable/script to main. Process markers are ICE_CODING_AGENT='true' and AI_AGENT='ice'; title uses APP_NAME. Bun restoration changes only a completely empty process.env and does not fill missing names in a nonempty map. The raw /proc path is an input to the TS workaround, not a required persistent file.

### 6. Dependencies

`AI-003`, `AUTH-012`, `AUTH-015`, `CLI-002`, `CLI-018`, `SET-013`

CLI-002 owns the ICE distribution wrapper. CLI-018 owns exact-name compatibility helpers. AI-003 and AUTH-012/015 own provider dispatch, Bedrock credentials and generic OAuth registration. SET-013 owns the HTTP dispatcher algorithm; this entry owns invocation order.

### 7. ice-rust today

The native binary, icers, uses a multi-thread Tokio runtime, awaits app::run and converts its integer code to ExitCode through a u8 cast; lib.rs supplies HOST_NAME/HOST_VERSION. It has no JavaScript warning hook, Bun injection layer or empty-process.env bug to reproduce (rust:crates/ice-cli/src/bin/icers.rs:1-21; rust:crates/ice-cli/src/lib.rs:82-89). Full app.rs startup and a scoped crates search found no corresponding assignments for ICE_CODING_AGENT/AI_AGENT, or PI_CODING_AGENT_DIR launcher bridge. The native environment configuration name ICERS_CODING_AGENT_DIR exists and must not be confused with the ICE_CODING_AGENT marker (rust:crates/ice-cli/src/app.rs:82-190; rust:crates/ice-cli/src/config.rs:89-108).

### 8. Port notes

Keep native startup/runtime construction. Add the documented process markers before provider/extension initialization if compatibility consumers require them, and define the native process-title equivalent explicitly. Do not copy JS-only warning suppression or the Bun /proc workaround into ordinary Rust environment handling. Verify the stock/distribution entrypoint distinction and adapter registration order in future implementation checks; this audit executed neither entrypoint.

### 9. Size and priority

Estimated Rust implementation: **35-80 lines** (estimate, not measured source). **P2** - Launch identity is a small compatibility boundary; Bun-specific workarounds should remain runtime-specific rather than inflate the native gap.

## CLI-002 - ICE distribution preflight, early agent commands and builtin factory order

### 1. Title and status

ICE distribution preflight, early agent commands and builtin factory order. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/ice.ts:1-130; rust:crates/ice-cli/src/app.rs:82-172.

Named export: `MainOptions` (`ice:packages/coding-agent/src/main.ts:522`).

`ice:packages/coding-agent/src/main.ts:522-533` (`MainOptions`, main's builtin factory composition).

### 3. What it does

The ICE launcher adds guarded distribution behavior before the reusable coding-agent main function runs. Its early command handling and factory installation determine which policies and integrations are available to later startup.

### 4. How it works

The distribution launcher handles a few commands before normal startup and installs its policy integrations in a fixed order. Its callbacks keep the parent session and current mode live as the runtime changes.

ice.ts sets ICE_CODING_AGENT, AI_AGENT and process title, fills PI_CODING_AGENT_DIR with getAgentDir only when the existing value is nullish, suppresses process warnings and configures the HTTP dispatcher. An existing empty PI_CODING_AGENT_DIR is preserved. It scans raw arguments for exact --import-agent-pack or the '=...' spelling anywhere in argv. More than one occurrence errors and exits 1. The single source is the inline suffix or following token; empty or leading-dash source errors. Successful import targets the global agent definitions directory, prints its summary and exits 0; failure prints its message and exits 1. This occurs before normal flag parsing and before the agents subcommand (ice:packages/coding-agent/src/ice.ts:24-57).

When argv[0] is 'agents', runAgentsCli receives the remaining arguments, current cwd, agent directory and stdout writer, and its returned code becomes process exit. Otherwise normalizeUnsafeSubagentStartupArgs receives strict stdin/stdout isTTY===true, then validateIceStartupArgs runs; validation errors print and exit 1. The normalized arguments then determine offline and metadata-only flags. refreshLocalModelsForStartup is awaited with skip/onFailure options before runtime factories are constructed; AUTH-032 records that the current helper does not actually perform the implied remote refresh (ice:packages/coding-agent/src/ice.ts:59-86).

The wrapper creates one AgentViewBridge and one safe-verify factory whose change callbacks update local mode/capability variables. It awaits main with globalFirstSettings=true and hidden builtin factories in this order: safe verify, provider settings, Cognee, subagents. The subagent factory registers the MCP adapter using a live getParentSession callback, installs session_shutdown unregistration, then binds subagent hooks, bridge and live mode/capability getters. This ordering precedes user extension factories through main's builtin factory composition (ice:packages/coding-agent/src/ice.ts:88-130; ice:packages/coding-agent/src/main.ts:530-532).

### 5. Interfaces

Early raw surfaces: --import-agent-pack <source>, --import-agent-pack=<source>, and 'ice agents ...'. Compatibility marker PI_CODING_AGENT_DIR uses nullish assignment. Distribution policy flags are normalized/validated before main; their authority semantics belong to SUB. main receives extensionFactories plus globalFirstSettings and agentViewBridge rather than a separate application implementation.

The exact public startup options are `MainOptions { extensionFactories?: InlineExtension[]; globalFirstSettings?: boolean; agentViewBridge?: IceAgentViewBridge }`. The caller's extension factories follow `builtInExtensions` in `main`; the distribution opts into global-first settings and supplies its optional view bridge.

### 6. Dependencies

`AUTH-028`, `AUTH-032`, `CLI-001`, `CLI-005`, `COG-043`, `EXT-002`, `EXT-057`, `SAFE-001`, `SAFE-025`, `SUB-005`, `SUB-007`, `SUB-019`, `SUB-035`, `SUB-036`

SUB owns agent definitions, guarded mode/authority policy, agent view bridge and extension lifecycle. AUTH-028 owns provider settings/catalog submenu behavior, AUTH-032 the actual startup model helper, EXT-002 exact historical package imports. Cognee/MCP mechanisms require their owning area links; this entry owns only installation/order and live callbacks.

### 7. ice-rust today

Native startup has substantial early dispatch of its own: hidden dev cleanup, --connect, offline normalization, dev/dev-local wrapping, and auth/events/update/install/uninstall/package commands. Historical pi-package/pi-update/install-pi/uninstall-pi names are explicitly rejected with usage code 2 (rust:crates/ice-cli/src/app.rs:82-172). That dispatch is not the TS ICE agents/import-pack/preflight/factory stack. Native extensions and package integrations may supply related capabilities, but the inspected app startup does not install these four TS distribution factories or expose the same raw command routes. The --connect assignment line 96 is connector-redacted and is not reconstructed.

### 8. Port notes

Add an explicit distribution startup layer around the existing native app, preserving native commands while implementing the authorized agents/import-pack routing and policy normalization through SUB-owned components. Install runtime integrations in deterministic order with live bridge/mode getters and matching shutdown unregistration. Decide the PI_CODING_AGENT_DIR bridge only at the actual package boundary, without treating it as permission to load arbitrary legacy modules or read historical user state.

### 9. Size and priority

Estimated Rust implementation: **130-280 lines** (estimate, not measured source). **P1** - Distribution policy and subagent integrations must be installed before user extensions and session execution; a parser-only port does not provide this behavior.

## CLI-003 - Argument token grammar, complete flag surface and diagnostic semantics

### 1. Title and status

Argument token grammar, complete flag surface and diagnostic semantics. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/cli/args.ts:1-247; ice:packages/coding-agent/test/args.test.ts:1-466; rust:crates/ice-cli/src/args.rs:76-578.

Named export: `isValidThinkingLevel` (`ice:packages/coding-agent/src/cli/args.ts:65`).

Named export: `printHelp` (`ice:packages/coding-agent/src/cli/args.ts:249`).

`ice:packages/coding-agent/src/cli/args.ts:249-443` (`printHelp`).

### 3. What it does

The command parser separates messages, file arguments, known settings and extension flags in one ordered pass. Small token-consumption differences change which prompt or option reaches the application, even when both implementations recognize most flag names.

### 4. How it works

The parser walks arguments once and decides whether each token is a known option, extension flag, file or message. Repeated options, missing values and flag-like prompt text follow precise rules that affect the resulting request.

TS initializes ordered messages/fileArgs/diagnostics arrays and an insertion-ordered unknownFlags Map. Scalar repeats use the last value; append-system-prompt, extension, skill, prompt-template and theme append in encounter order. Known value flags normally require only that another token exists and consume it even when it begins '-' or '@'; special --name and --ui-mode branches add their own errors. A missing ordinary long value flag falls through to unknown-long handling, while a missing recognized short value spelling becomes an unknown-short error. Known '=value' spellings are not generally recognized: except --observe=, they enter unknownFlags under the stripped name (ice:packages/coding-agent/src/cli/args.ts:69-247).

The boolean/action surface is --help/-h, --version/-v, --print-system-template, --observe, --continue/-c, --resume/-r, --no-session, --no-tools/-nt, --no-builtin-tools/-nbt, --fast, --no-fast, --print/-p, --no-extensions/-ne, --no-skills/-ns, --no-prompt-templates/-np, --no-themes, --no-context-files/-nc, --verbose, --approve/-a, --no-approve/-na and --offline. Value surfaces are --mode, --provider, --model, --api-key, --system-prompt, repeated --append-system-prompt, --name/-n, --session, --session-id, --fork, --session-dir, --models, --tools/-t, --exclude-tools/-xt, --thinking, --export, repeated --extension/-e, --skill, --prompt-template, --theme, optional --list-models and --ui-mode (ice:packages/coding-agent/src/cli/args.ts:80-221).

--mode consumes its next token but assigns only text/json/rpc; invalid values are silently ignored. --thinking accepts exactly off/minimal/low/medium/high/xhigh/max/ultra; invalid values add a warning and leave a prior valid value intact. --models splits commas and trims but retains empty entries; tools/exclude-tools remove empty entries. --observe= converts using Number and requires an integer 1..65535; bare --observe is true and consumes no port token. --ui-mode accepts exact regular/fullscreen; missing or leading-dash value errors without consuming the next flag, while an invalid ordinary value is consumed and reported (ice:packages/coding-agent/src/cli/args.ts:63-67; ice:packages/coding-agent/src/cli/args.ts:86-100; ice:packages/coding-agent/src/cli/args.ts:133-168; ice:packages/coding-agent/src/cli/args.ts:194-213).

-p may consume the following prompt unless it begins '@' or begins '-' without the special '---' prefix. That prefix deliberately accepts YAML frontmatter. Optional --list-models consumes only a following non-flag/non-file token. Any token starting '@' becomes fileArgs with its first character removed, including bare '@' becoming an empty path. Unknown long flags support '--name=value', or consume one following non-flag/non-file string, otherwise true. Bare '--' is therefore an unknown flag with an empty name, not an end-of-options marker. Every other leading '-' token, including bare '-', reports an error. Existing tests cover representative aliases, repeatable options, tool flags, unknown flags and YAML prompts; they do not establish every boundary listed here by execution (ice:packages/coding-agent/src/cli/args.ts:163-247; ice:packages/coding-agent/test/args.test.ts:36-61; ice:packages/coding-agent/test/args.test.ts:411-466).

`printHelp(extensionFlags?: ExtensionFlag[]): void` prints the static usage, package/auth command list, options, examples, environment descriptions and seven builtin tool names through `console.log`. A nonempty extension flag list adds an Extension CLI Flags section in supplied order. Each row uses `--${flag.name}`, appends ` <value>` only for `type === "string"`, pads the flag column to 30 characters, and uses `flag.description ?? "Registered by " + flag.extensionPath`. An absent or empty list adds no section. The printed examples and advertised defaults are presentation strings; they do not validate inputs or prove live provider availability.

### 5. Interfaces

parseArgs(string[])→Args with diagnostics [{type:'warning'|'error', message}], unknownFlags Map<string, string|boolean>, ordered messages/fileArgs and optional settings. Main reports warnings and errors separately; credential-print treats any diagnostic as command failure. Exact field consumers are CLI-004/006/007/011/017/019.

### 6. Dependencies

`AI-005`, `CLI-004`, `CLI-006`, `CLI-007`, `CLI-011`, `CLI-017`, `CLI-019`, `SET-003`

AI-005 owns thinking-level clamp behavior after parsing, SET-003 project trust policy. Parser acceptance alone does not implement service-tier, dashboard, resource or model behavior.

### 7. ice-rust today

Native args.rs genuinely parses most common flags and has extensive inline tests. It additionally recognizes --base-url, --timeout, --extensions-dir/ICERS_EXTENSIONS_DIR, --dev-local-only, --project-only and --debug-system-prompt. It peels '=value' for all long flags; take_value rejects a following dash-prefixed value except bare '-'. It stores unknown flags in a sorted BTreeMap; CSV values drop empty fields; theme is a last-wins scalar; --thinking lacks ultra; invalid --mode is a usage error. Native --tui-mode accepts case-insensitive regular/fullscreen and defaults to regular on macOS, fullscreen elsewhere, while TS's public flag is --ui-mode. It does not recognize fast/no-fast/observe/print-system-template as builtins. Bare '@' and '-' become messages in Rust. The -p branch lacks the TS '---' prompt exception (rust:crates/ice-cli/src/args.rs:76-270; rust:crates/ice-cli/src/args.rs:271-555; rust:crates/ice-cli/src/args.rs:718-1120).

### 8. Port notes

Build an explicit compatibility table for every public spelling and token-consumption rule, retaining native-only flags as documented additions. Implement ultra, fast/no-fast, observe, print-system-template and --ui-mode routing through their owners. Preserve the YAML prompt case and ordered repeated values. Where TS behavior is surprising—missing values, unknown empty '--', empty @ path—document and consciously choose compatibility or a diagnosed correction instead of silently changing semantics. Future verification should cover token boundaries and precedence, not merely recognized names.

### 9. Size and priority

Estimated Rust implementation: **180-360 lines** (estimate, not measured source). **P1** - Incorrect token consumption can discard a user prompt or reinterpret options; missing flag names are only part of the actual parser gap.

## CLI-004 - Run-mode resolution and metadata short-circuit precedence

### 1. Title and status

Run-mode resolution and metadata short-circuit precedence. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/main.ts:91-160; ice:packages/coding-agent/src/main.ts:554-627; ice:packages/coding-agent/src/main.ts:834-895; rust:crates/ice-cli/src/app.rs:523-579; rust:crates/ice-cli/src/args.rs:556-578.

### 3. What it does

Startup chooses interactive, print, JSON or RPC behavior and decides how early metadata requests may exit. These choices also determine which errors are reported and which services or migrations have already run.

### 4. How it works

Mode selection determines who owns the terminal streams and whether startup may run without a model. Metadata commands exit at different stages, so their error precedence and side effects must be tracked separately.

TS resolveAppMode chooses explicit rpc first, explicit json second, then print when -p is set or either stdin or stdout is not a TTY; otherwise interactive. Explicit --mode text alone does not force print on two TTYs. A plain runtime metadata command is exactly !print && mode===undefined && (help || listModels!==undefined); this separate predicate controls stdout takeover. envTruthy recognizes '1' or case-insensitive true/yes without trimming (ice:packages/coding-agent/src/main.ts:91-115).

Main routes package commands, config command and credential-print before normal parse. It reports all parser diagnostics and exits 1 on any error before --version. Version then prints VERSION alone and exits 0; print-system-template writes DEFAULT_BASE_TEMPLATE plus one newline and exits 0; export uses parsed.export and the first positional output name, reporting an export error and exiting 1 or printing the resulting path and exiting 0. These precede mode selection, session option validation and startup migrations. Thus --version wins over --help after valid parsing, but does not suppress a parse error (ice:packages/coding-agent/src/main.ts:554-617).

Help and list-models are deliberately later: they select an in-memory session, run migrations and create the runtime/services/extensions, then print extension-aware help or available models before ordinary runtime error diagnostics are reported. Help receives extension flag registrations from the built runtime. A list-models failure can still reject through its awaited helper; model-list success exits 0. RPC rejects CLI file arguments before migrations and never drains piped stdin. After runtime creation, non-RPC piped content can downgrade interactive to print. Noninteractive execution without a resolved model is fatal after runtime diagnostics; the startup benchmark additionally requires interactive mode (ice:packages/coding-agent/src/main.ts:620-627; ice:packages/coding-agent/src/main.ts:831-895).

### 5. Interfaces

App modes are interactive/print/json/rpc. --mode text is the text output selection when print mode is otherwise chosen. TS normal startup usage/runtime errors mostly exit 1; successful metadata paths exit 0. --print-system-template exposes the actual base template, not the expanded per-session system prompt. HTML export implementation is a MISC dependency.

### 6. Dependencies

`AUTH-026`, `CLI-003`, `CLI-005`, `CLI-006`, `CLI-017`, `CLI-019`, `RPC-016`

RPC-016 owns the JSONL command-mode protocol boundary, while this entry owns its selection. CLI-005 owns runtime creation and CLI-019 stream routing/cleanup. Model resolution is AUTH-026.

### 7. ice-rust today

Native handle_parse_short_circuits checks help first, version second and parser errors third, so --help suppresses errors and beats --version. Native version prints 'icers VERSION'; its help is static and does not create services or list extension flags. It migrates its layout after cwd resolution, then handles export and list-models before provider credentials, resume and harness construction (rust:crates/ice-cli/src/app.rs:174-219; rust:crates/ice-cli/src/app.rs:523-579; rust:crates/ice-cli/src/args.rs:589-716). Native resolve_mode gives the presence of unknown flag 'server' highest priority, then rpc/json, then -p/non-TTY; later ICERS_FORCE_TUI='1' can override it until a plugin stdio claim authoritatively forces RPC. A genuine JSONL branch exists; comments claiming no built-in RPC are stale (rust:crates/ice-cli/src/args.rs:556-578; rust:crates/ice-cli/src/app.rs:234-235; rust:crates/ice-cli/src/app.rs:352-381; rust:crates/ice-cli/src/app.rs:434-505).

### 8. Port notes

Define and implement explicit short-circuit precedence for compatibility, including diagnostics before version and extension-aware help, while preserving the native server/stdio ownership path. Add --print-system-template as a real early output route. Keep list-models independent of authenticated prompt execution when adopting the native convenience, and document that deliberate difference if retained. Do not infer resource-free TS help from the flag name; match or consciously simplify its actual service side effects.

### 9. Size and priority

Estimated Rust implementation: **100-200 lines** (estimate, not measured source). **P1** - Metadata and mode precedence affect scripts, startup side effects and protocol ownership; stale comments currently obscure implemented native behavior.

## CLI-005 - Cwd-bound runtime factory, trust staging and startup service composition

### 1. Title and status

Cwd-bound runtime factory, trust staging and startup service composition. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/main.ts:530-558; ice:packages/coding-agent/src/main.ts:628-852; rust:crates/ice-cli/src/app.rs:182-350; rust:crates/ice-cli/src/session.rs:209-240; rust:crates/ice-cli/src/session.rs:345-466.

### 3. What it does

CLI startup builds services around the selected session's working directory and repeats that construction when the runtime changes session context. Trust resolution and settings must occur at the correct stage so the session receives the intended resources, model and tool policy.

### 4. How it works

Startup first resolves the session, then builds settings and resources for that session's working directory. The same factory can rebuild services for a later session replacement while retaining the intended trust and explicit-path boundaries.

Main starts with launch cwd/agent dir and a bootstrap SettingsManager using projectTrusted=false plus the caller's globalFirst option, applies its global HTTP proxy and configures the dispatcher before early package/config/auth routes. After parsing and session-option validation it runs all startup migrations, creates startup settings, drains their errors, optionally performs first setup and resolves the session manager. A requested session name is trimmed, validated and appended before model/runtime validation. Missing stored cwd is handled before the session cwd becomes the runtime construction cwd (ice:packages/coding-agent/src/main.ts:530-558; ice:packages/coding-agent/src/main.ts:628-676).

The reusable runtime factory receives a session manager and sessionStartEvent. It caches project trust by cwd, uses explicit overrides first, and detects whether project resources require a trust decision. When a decision is still required it initially creates settings with projectTrusted=false and installs a resource-loader resolveProjectTrust callback. That callback evaluates the current factory cwd with startup global defaultProjectTrust and either the supplied extension context or a CLI context whose metadata-command prompt mode is print. Its result is cached for subsequent construction. When no decision is required, the trusted value comes from cache, explicit override, absence of trust-requiring resources or a stored true decision. Exact policy and parent-folder matching remain SET-003; this is the call order and cache boundary (ice:packages/coding-agent/src/main.ts:677-747).

Explicit extension/skill/prompt/theme paths resolve once against the original launch cwd, even if a stored session changes cwd; local paths are normalized and package source strings remain package sources. Each factory call constructs cwd-bound settings and services, supplies parsed unknown extension flags, a 15-second model-runtime signal, all explicit/no-resource overrides, system/append prompt and ordered factories. Diagnostics combine trust, service, settings-read and extension-load errors. Scoped model patterns are CLI models if defined, otherwise settings enabled models, resolved with a separate 15-second signal. The factory builds model/tool options, applies a truthy CLI API key only after a provider/model has been selected, creates the agent session, and persists explicit or shorthand thinking by calling setThinkingLevel on the actual created level (ice:packages/coding-agent/src/main.ts:508-510; ice:packages/coding-agent/src/main.ts:690-831).

createAgentSessionRuntime owns replacement around this factory. The AgentViewBridge is attached to runtime.session after creation, and the resulting runtime's global proxy/idle timeout are applied before metadata or mode execution. The 15-second signals bound model operations, not every synchronous migration, extension hook or entire startup (ice:packages/coding-agent/src/main.ts:834-852).

### 5. Interfaces

main(args,{extensionFactories?, globalFirstSettings?, agentViewBridge?}) composes a runtime factory receiving SessionManager plus optional lifecycle context. Runtime diagnostics remain values until the mode/metadata branch decides to report them. Trust cache keys and service cwd use the selected session cwd; explicit path arguments use launch cwd. CLI-006/007 specify selection and model option precedence.

### 6. Dependencies

`AUTH-024`, `AUTH-026`, `CLI-002`, `CLI-004`, `CLI-006`, `CLI-007`, `CLI-013`, `CLI-014`, `EXT-009`, `EXT-031`, `SESSION-021`, `SESSION-028`, `SESSION-035`, `SET-003`, `SET-007`, `SET-008`, `SET-009`, `SET-013`, `SET-021`, `SET-022`

SESSION-021/028/035 own replaceable runtime, SDK construction and reload sequencing. SET owns settings storage/trust policy, AUTH owns model matching/initial defaults, EXT owns extension binding/resource attribution. This entry does not duplicate those mechanisms.

### 7. ice-rust today

Native app prepares extensions once before provider resolution, builds authenticated catalog and persists its --models scope, then calls build_prepared. The prepared extension action host is initially unbound; build_prepared installs the catalog, cwd-bound OsExecutionEnv/tool context, merged tool policy, selected session and identity before prompt/harness creation. It is real composition, not absent SDK functionality (rust:crates/ice-cli/src/app.rs:246-313; rust:crates/ice-cli/src/session.rs:209-240; rust:crates/ice-cli/src/session.rs:345-466). However startup uses process cwd, direct resolve_project_trust and native reload context; it does not implement the TS factory's per-session cwd trust cache, startup diagnostic value list, launch-relative explicit paths, global-first option or typed runtime replacement flow. Native missing-cwd restore warns rather than using TS interactive approval/headless failure (rust:crates/ice-cli/src/session.rs:2676-2713).

### 8. Port notes

Reuse prepared native extensions and harness assembly behind a cwd-aware runtime factory. Connect trust and settings before loading project resources, keep launch-relative explicit paths stable across restored cwd changes, and give replacement operations a single owner through SESSION-021. Preserve typed startup diagnostics and model-operation deadlines. Ensure migration order and requested-name timing are deliberate; permission policy must not be inferred from a successfully constructed settings object.

### 9. Size and priority

Estimated Rust implementation: **220-430 lines** (estimate, not measured source). **P0** - Startup service composition determines which project code and policy enter a session; wrong cwd or trust staging creates functional and authority mismatches.

## CLI-006 - Startup session selection, exact-ID validation, naming and missing-cwd recovery

### 1. Title and status

Startup session selection, exact-ID validation, naming and missing-cwd recovery. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/main.ts:203-405; ice:packages/coding-agent/src/main.ts:648-674; ice:packages/coding-agent/test/startup-session-name.test.ts:1-135; rust:crates/ice-cli/src/session.rs:130-192; rust:crates/ice-cli/src/session.rs:2070-2184.

### 3. What it does

Session flags select the conversation before the runtime is created, and some combinations are rejected rather than silently prioritized. Opening a different project or a session whose cwd disappeared has distinct confirmation, failure and persistence behavior.

### 4. How it works

Session selection follows a fixed precedence after conflicting options are rejected. Naming and missing-directory recovery happen before model validation, so they can have visible effects even when later startup fails.

A truthy --fork conflicts with truthy session/continue/resume/noSession, but may combine with --session-id. A defined sessionId conflicts with truthy session/continue/resume and is always passed through assertValidSessionId; it may combine with no-session. These checks run after metadata/export short-circuits and before migrations. Directory precedence is truthy CLI session-dir normalized as a path, then truthy ENV_SESSION_DIR with tilde expansion, then startup settings sessionDir (ice:packages/coding-agent/src/main.ts:255-292; ice:packages/coding-agent/src/main.ts:620-627; ice:packages/coding-agent/src/main.ts:648-652).

Selection branch order is noSession/help/listModels→inMemory, fork, session, resume, continue, sessionId lookup, then create. In-memory creation receives a defined requested ID. Fork first checks an exact local target-ID collision when a target ID is truthy, resolves its source and creates a fork with requested cwd/sessionDir/id. A session argument containing slash, backslash or ending '.jsonl' is resolved as a direct path; other values search local exact ID, local first prefix, global exact ID, global first prefix. There is no ambiguity rejection. Direct/local session matches open; a global ID match prints its source cwd and asks whether to fork into launch cwd. Only case-insensitive exact y/yes accepts, without trimming. That readline confirmation has no app-mode guard. Declining exits 0. This is a source-visible protocol risk if invoked through a noninteractive startup route, not a claim that such an invocation was executed (ice:packages/coding-agent/src/main.ts:213-253; ice:packages/coding-agent/src/main.ts:314-369).

Resume supplies both local and all-project asynchronous loaders to the startup picker, exits 0 on no selection, and stops the theme watcher in finally. Continue delegates to continueRecent; its absence fallback creates a new session under SESSION-032. A remaining requested ID opens an exact local match or warns and creates a new session with that ID. Open/fork errors are caught, printed and exit 1 (ice:packages/coding-agent/src/main.ts:294-312; ice:packages/coding-agent/src/main.ts:371-405).

After selection, a missing stored cwd prompts Continue with the current cwd or Cancel only in interactive mode; acceptance reopens the session with the fallback cwd, cancellation exits 0, and noninteractive modes fail with MissingSessionCwdError and code 1. A defined --name is trimmed and must be nonempty; it appends session_info before any model validation. Tests explicitly expect the name to persist even when a later missing-model error exits 1, and no append for whitespace-only name (ice:packages/coding-agent/src/main.ts:654-674; ice:packages/coding-agent/test/startup-session-name.test.ts:110-135).

### 5. Interfaces

Selection accepts --no-session, --fork <path|id>, --session <path|id>, --resume/-r, --continue/-c, --session-id, --session-dir and --name/-n. Direct path resolution is distinct from ID discovery. Empty versus undefined option values matter. SESSION-029/031 own valid-ID and creation/persistence rules; CLI owns validation timing and branch precedence.

### 6. Dependencies

`CLI-003`, `CLI-005`, `CLI-016`, `SESSION-029`, `SESSION-031`, `SESSION-032`, `SESSION-033`

Session storage, rich discovery, fork branch projection and identity validation remain SESSION contracts. TUI session picker behavior is reached through CLI-016.

### 7. ice-rust today

Native select_session prioritizes no_session, then continue/resume, fork, session_id, session. It applies no TS conflict validation and defaults its root to cwd/.icers/sessions, with only flag then saved setting overriding it. app resolves --resume into a picker result before provider/harness creation and requires both TTYs; the picker uses the default project directory rather than the supplied session-dir. --continue has an error when no resumable metadata exists instead of the TS fresh-session fallback (rust:crates/ice-cli/src/session.rs:154-192; rust:crates/ice-cli/src/app.rs:617-638; rust:crates/ice-cli/src/resume_picker.rs:16-26; rust:crates/ice-cli/src/session.rs:2096-2135).

Native ById searches the selected directory using one first-match predicate (exact ID OR path contains query OR query contains ID); it does not perform the TS direct-path/global exact-before-prefix algorithm. Fork searches only its default root, ignores selected session-dir and requested target ID, and creates with id=None. Native name is applied during build_prepared after provider resolution, whitespace clears the name, and failures are only warned when verbose. Missing-cwd restore always warns and continues using native startup cwd, without TS interactive approval or headless failure (rust:crates/ice-cli/src/session.rs:2498-2519; rust:crates/ice-cli/src/session.rs:2137-2184; rust:crates/ice-cli/src/session.rs:400-422; rust:crates/ice-cli/src/session.rs:2676-2713).

### 8. Port notes

Implement one explicit validated startup selection plan before runtime creation. Preserve exact-ID collision checks, path-versus-ID resolution, custom directory handling and requested-name timing. Add cwd recovery with a typed noninteractive error and interactive decision. Keep the native TTY check for resume as a useful safeguard; avoid copying the unguarded cross-project readline prompt into RPC/JSON execution. Resolve global-session behavior through a mode-aware decision channel. Retain native storage formats under SESSION's migration plan rather than treating a path match as format compatibility.

### 9. Size and priority

Estimated Rust implementation: **220-390 lines** (estimate, not measured source). **P1** - Incorrect session precedence or cwd selection can append to the wrong conversation or execute in an unintended project; naming timing is independently tested.

## CLI-007 - CLI model, thinking, fast-mode and tool-option precedence

### 1. Title and status

CLI model, thinking, fast-mode and tool-option precedence. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/main.ts:407-505; ice:packages/coding-agent/src/main.ts:770-831; rust:crates/ice-cli/src/app.rs:255-302; rust:crates/ice-cli/src/session.rs:379-398.

### 3. What it does

Parsed model and tool flags are converted into a session configuration without erasing saved or restored choices unnecessarily. Scoped models and thinking suffixes have their own fallback precedence, and a CLI API key is a runtime-only override.

### 4. How it works

CLI model choices override saved or scoped defaults only under specific conditions. The resulting options preserve the distinction between an omitted setting, an explicit false value and an empty list.

buildSessionOptions resolves a CLI model only when parsed.model is truthy, passing provider/model/thinking together. It preserves warning/error diagnostics and installs a resolved model when present. A resolved ':thinking' suffix applies only without an explicit parsed.thinking and sets cliThinkingFromModel. A provider-only flag does not itself assign options.model in this helper (ice:packages/coding-agent/src/main.ts:407-447).

If no model was selected, scopedModels is nonempty and the existing session context has no messages, choose the saved provider/model when that exact model is in scope, otherwise the first scoped model. Its explicit scoped thinking applies only without CLI thinking. A truthy explicit thinking value then overrides both suffix and scope. A defined fastMode, including false, is copied. Nonempty scoped models are copied as {model, thinkingLevel}, retaining undefined thinking to mean inherit during cycling. This helper does not apply a scope fallback to an existing conversation and does not silently discard earlier model-resolution errors (ice:packages/coding-agent/src/main.ts:449-487; ice:packages/coding-agent/src/main.ts:780-787).

Tool precedence is noTools→'all', else noBuiltinTools→'builtin'; parsed tools and excludeTools arrays are copied even when empty arrays. The downstream registry decides active tool semantics. CLI model patterns override settings enabledModels by nullish choice, so an explicit empty array is distinct from absence. A truthy --api-key is installed through modelRuntime.setRuntimeApiKey only when session options already contain a model; otherwise a diagnostic requires --model or --models. No credential is persisted by this step. After session creation, explicit CLI thinking or a model suffix triggers setThinkingLevel on the created session's actual level, ensuring its durable change is recorded (ice:packages/coding-agent/src/main.ts:489-505; ice:packages/coding-agent/src/main.ts:770-831).

### 5. Interfaces

buildSessionOptions returns {options, cliThinkingFromModel, diagnostics}; options include model, thinkingLevel, fastMode, scopedModels, noTools, tools, excludeTools. Model matching and scope syntax belong to AUTH-024/025, shared thinking support/clamp to AI-005 and session changes to SESSION-036.

### 6. Dependencies

`AI-005`, `AUTH-005`, `AUTH-024`, `AUTH-025`, `AUTH-026`, `CLI-003`, `CLI-005`, `SESSION-023`, `SESSION-036`, `SET-011`

AUTH-005 owns runtime credential override semantics; SET-011 owns default/scoped/queue persistence. SESSION-023 applies the tool registry policy and SESSION-036 owns resulting model/thinking state transitions.

### 7. ice-rust today

Native app resolves the model/provider before restoring/building the session. Its --models branch compares each supplied value to catalog model IDs using ASCII case-insensitive equality, reports unmatched values, preserves repeated matches, and writes the result directly into global settings.scoped_models; zero matches become None. It is neither TS pattern resolution nor transient session scope construction (rust:crates/ice-cli/src/app.rs:255-302). Native tool construction and merged active-name filtering are real and must be reused (rust:crates/ice-cli/src/session.rs:379-398; rust:crates/ice-cli/src/session.rs:2060-2068). Native parser lacks fast/no-fast and ultra as builtin options; AUTH-025/026 and AI-005 record broader model/restore/clamp differences instead of duplicating them here.

### 8. Port notes

Move native CLI option application behind the same resolved-session/service stage as its SDK, preserving existing native tool filtering. Implement scoped pattern matching through AUTH-024, keep CLI scope transient unless the product explicitly chooses persistence, apply saved-in-scope fallback only for an empty conversation, and persist the actual explicit thinking change through SESSION-036. Install API-key overrides only in memory and preserve false/empty-array distinctions.

### 9. Size and priority

Estimated Rust implementation: **120-240 lines** (estimate, not measured source). **P1** - Model/scope precedence changes the provider used for a prompt and currently mutates saved native scope unexpectedly compared with TS.

## CLI-008 - Piped stdin ownership, complete-input collection and read failures

### 1. Title and status

Piped stdin ownership, complete-input collection and read failures. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/cli/piped-stdin.ts:1-55; ice:packages/coding-agent/test/piped-stdin.test.ts:1-53; ice:packages/coding-agent/src/main.ts:861-870; rust:crates/ice-cli/src/app.rs:315-342; rust:crates/ice-cli/src/app.rs:770-800.

### 3. What it does

A piped prompt is consumed only when the application owns stdin, and is accepted only after a complete successful read. Read failures must not turn into an apparently valid prompt that omits the user's piped input.

### 4. How it works

A pipe is accepted as prompt content only after a complete read. The application leaves stdin untouched when a terminal or protocol endpoint owns it and reports read failures instead of silently omitting input.

readPipedStdin first returns undefined for isTTY, then throws a preexisting input.errored, then returns undefined for readableEnded or destroyed. Consequently a TTY takes precedence over an error and an already destroyed nonerrored stream is treated as exhausted. For the real process.stdin under Bun with Bun.stdin, it awaits Bun.stdin.text, trims the outer whitespace and returns undefined for an empty result (ice:packages/coding-agent/src/cli/piped-stdin.ts:1-21).

The Node stream path installs data/end/error/close listeners, uses setEncoding('utf8') and resume, accumulates the full text without a size limit, and resolves only on end after trim. A settled guard makes completion idempotent. Cleanup removes all listeners. Error rejects with the actual error. Close succeeds only if readableEnded is already true; otherwise it rejects input.errored or a specific premature-EOF error. There is no deadline or streaming prompt delivery. The tests cover split multibyte UTF-8, internal whitespace, empty/exhausted input, no TTY consumption, premature close and propagated failures (ice:packages/coding-agent/src/cli/piped-stdin.ts:23-55; ice:packages/coding-agent/test/piped-stdin.test.ts:6-53).

Main invokes it only outside RPC, after the runtime has been built. A returned nonempty value can downgrade interactive mode to print and drain captured extension console output. RPC stdin remains the JSONL command channel (ice:packages/coding-agent/src/main.ts:861-870).

### 5. Interfaces

readPipedStdin(input:Readable & {isTTY?:boolean}=process.stdin)→Promise<string|undefined>. Undefined means no usable piped content; rejection means the input was not read completely. Outer whitespace is removed; internal whitespace is preserved. No default byte cap or timeout is imposed at this helper.

### 6. Dependencies

`CLI-004`, `CLI-010`, `CLI-019`, `RPC-016`

CLI-010 combines the completed text with attachments and the first CLI message. CLI-019 owns output-stream routing; RPC-016 owns stdin when command mode is selected.

### 7. ice-rust today

Native app deliberately waits until extensions are loaded to determine owns_stdio and validates the stdio lifecycle before reading. It skips reads for rpc/server or a plugin stdio claim, a native capability that should be retained (rust:crates/ice-cli/src/app.rs:221-235; rust:crates/ice-cli/src/app.rs:315-342). Its reader has an ICERS_SKIP_STDIN='1' escape hatch, skips TTYs, calls blocking read_to_string and trims successful input, but maps every read/UTF-8 error to None. Thus it can proceed with remaining CLI/file input after silently losing a failed pipe. It also performs the blocking read directly in the async startup path (rust:crates/ice-cli/src/app.rs:770-800).

### 8. Port notes

Return a typed Result<Option<String>> from native stdin collection and fail startup when input is incomplete or unreadable. Retain stdio-claim/RPC ownership checks and exact outer trimming. Move blocking collection to an appropriate blocking task if the startup runtime must remain responsive. Define any future byte/time bound separately rather than claiming one exists in TS. Preserve the native test escape hatch only as an explicit documented extension.

### 9. Size and priority

Estimated Rust implementation: **50-110 lines** (estimate, not measured source). **P1** - Silently dropping a failed piped prompt can execute an unintended partial request; complete-input semantics are directly covered by TS test source.

## CLI-009 - CLI file attachments, typed text failures and image preprocessing

### 1. Title and status

CLI file attachments, typed text failures and image preprocessing. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/cli/file-processor.ts:1-100; ice:packages/coding-agent/src/main.ts:163-195; ice:packages/coding-agent/test/block-images.test.ts:102-147; ice:packages/coding-agent/test/image-resize-callers.test.ts:44-52; rust:crates/ice-cli/src/app.rs:802-856.

Named export: `ProcessedFiles` (`ice:packages/coding-agent/src/cli/file-processor.ts:12`).

Named export: `ProcessFileOptions` (`ice:packages/coding-agent/src/cli/file-processor.ts:17`).

### 3. What it does

File arguments become ordered text references and image attachments with safe preprocessing. The native CLI already attaches supported images; the remaining differences are path normalization, text validation and image transformation/error behavior.

### 4. How it works

File arguments are processed in order into text references and image attachments. Image conversion and resizing run before delivery, while text failures carry a path and a typed reason.

processFileArguments accepts autoResizeImages default true, cwd default process.cwd and optional maxTextBytes default unlimited. It processes fileArgs sequentially. Each path passes resolveReadPath then resolve(cwd,...), which supplies tilde and platform filename normalization through the path helper. stat failure is reported as FileArgumentError('not_found'); nonregular files become 'unreadable'; zero-byte files are skipped entirely. It detects supported image MIME from file contents before choosing the branch (ice:packages/coding-agent/src/cli/file-processor.ts:17-59).

Images are read and passed to processImage with the resolved MIME and auto-resize setting. A failed processing result adds a '<file name="absolutePath">message</file>\n' text reference and no image. Success appends an ImageContent carrying processed mime/data and a text reference containing joined processing hints or empty content. Conversion/resize logic belongs to TOOLS-023; the CLI always invokes it and does not consult blockImages here. The blockImages filter belongs to the later model conversion layer. Text files enforce maxTextBytes only when supplied, then read UTF-8 and wrap content in '<file name="absolutePath">\ncontent\n</file>\n'. Node decoding does not reject all invalid byte sequences like strict String::from_utf8. A text read exception becomes 'unreadable' with path and reason; image detection/read/processing exceptions are not wrapped by this text catch. Filename/content strings are not XML-escaped (ice:packages/coding-agent/src/cli/file-processor.ts:61-99).

Main bypasses the helper for no file arguments. Otherwise it passes the session settings auto-resize flag, converts FileArgumentError into a printed error and process exit 1, and lets other exceptions propagate. The standalone CLI supplies neither a maxTextBytes limit nor a cwd option, so attachment relative paths use process cwd, distinct from launch-relative resource paths and potentially restored session cwd. RPC attachment handling supplies its own bounded options under RPC ownership. Existing tests assert BMP→PNG conversion hints, images even when later filtering is available, normal text, and omission when safe resize fails; none were executed here (ice:packages/coding-agent/src/main.ts:163-195; ice:packages/coding-agent/test/block-images.test.ts:102-147; ice:packages/coding-agent/test/image-resize-callers.test.ts:44-52).

### 5. Interfaces

processFileArguments(string[],{autoResizeImages?, cwd?, maxTextBytes?})→Promise<{text, images}>. FileArgumentError codes: not_found, too_large, unreadable. Text references and images preserve argument order within their respective arrays/concatenation; empty files produce neither. Image processing is enabled by default.

### 6. Dependencies

`CLI-008`, `CLI-010`, `MISC-003`, `RPC-021`, `SESSION-019`, `TOOLS-020`, `TOOLS-023`

RPC-021 owns its attachment request boundary and size limits. TOOLS-020 owns shared read-path normalization, TOOLS-022 the ReadTool caller and TOOLS-023 image MIME/conversion/resize mechanics. SESSION-019 owns model-input preflight and later image policy. MISC-003 distinguishes later native user-image normalization from the CLI ingestion boundary and owns tool-result image normalization.

### 7. ice-rust today

The native process_file_args resolves relative paths against startup cwd, checks existence, reads bytes, recognizes supported image MIME and base64-encodes images into the returned vector. Those images are passed into print/json/interactive mode calls. The app.rs header's claim that images are not wired is therefore contradicted by implementation and in-file image tests (rust:crates/ice-cli/src/app.rs:238-244; rust:crates/ice-cli/src/app.rs:393-423; rust:crates/ice-cli/src/app.rs:807-856; rust:crates/ice-cli/src/app.rs:988-1013). Native image attachments have no associated text reference, conversion/resize/hints or settings-driven autoResize step at this boundary. Empty text files produce an empty file wrapper, non-image invalid UTF-8 errors, and no typed maxTextBytes/regular-file contract exists here. Its errors return usage code 2. Later native user-message preprocessing calls normalize_user_images, which runs normalize_image_blocks on a blocking worker before prompt/queue admission; thus the missing CLI text references and settings option at ingestion do not establish globally absent native resizing (rust:crates/ice-harness/src/agent_harness.rs:4478-4495). MISC-003 records that later transport boundary and TOOLS-023 its algorithm.

### 8. Port notes

Retain native image attachment delivery and add the shared image preprocessing pipeline plus text reference/hints. Normalize paths through the shared tools path contract, skip zero-byte files, validate regular files and expose typed read/size errors. Decide strict versus replacement UTF-8 handling explicitly and document it. Keep CLI's currently unbounded text limit distinct from RPC's finite request contract. Do not mark image support missing because of stale comments.

### 9. Size and priority

Estimated Rust implementation: **110-230 lines** (estimate, not measured source). **P1** - Attachment preprocessing controls usable image dimensions, format support and prompt provenance; native raw-image support is present but incomplete.

## CLI-010 - Initial prompt concatenation and remaining-message ownership

### 1. Title and status

Initial prompt concatenation and remaining-message ownership. **Status: different-by-design.**

### 2. ice source

ice:packages/coding-agent/src/cli/initial-message.ts:1-43; ice:packages/coding-agent/test/initial-message.test.ts:1-48; rust:crates/ice-cli/src/app.rs:344-350; rust:crates/ice-cli/src/app.rs:858-886.

Named export: `InitialMessageInput` (`ice:packages/coding-agent/src/cli/initial-message.ts:4`).

Named export: `InitialMessageResult` (`ice:packages/coding-agent/src/cli/initial-message.ts:11`).

### 3. What it does

The initial prompt combines completed stdin, attachment text and the first positional message in that exact order. Remaining positional messages are retained for later turns rather than merged into one request.

### 4. How it works

The initial prompt concatenates stdin, attachment text and one positional message without adding separators. Remaining messages stay available for later turns, and images travel separately.

buildInitialMessage pushes stdinContent whenever it is not undefined, including an empty string; pushes fileText only when truthy; and removes the first element from parsed.messages with shift whenever that array is nonempty, including an empty first message. It joins the pieces with an empty separator, so callers must supply any desired newlines. If no pieces exist the initialMessage is undefined, otherwise it can be the empty string. initialImages is the supplied nonempty image array itself or undefined; there is no clone or image-to-text merge. The function mutates parsed.messages and returns only {initialMessage, initialImages} (ice:packages/coding-agent/src/cli/initial-message.ts:18-43).

The CLI passes attachment results and completed stdin into this helper, then hands the mutated remaining parsed.messages to interactive or print mode. Tests assert stdin+first-message combination and retention of a second message, with exact newline behavior supplied by fixture strings (ice:packages/coding-agent/src/main.ts:163-180; ice:packages/coding-agent/src/main.ts:953-991; ice:packages/coding-agent/test/initial-message.test.ts:14-48).

### 5. Interfaces

buildInitialMessage({parsed, stdinContent?, fileText?, fileImages?})→{initialMessage?:string, initialImages?:ImageContent[]}. No implicit separator, no trimming at this stage, and no deduplication. Whitespace normalization is performed earlier only by the stdin reader.

### 6. Dependencies

`CLI-008`, `CLI-009`, `SESSION-019`

The stdin and attachment owners produce the pieces. Session prompt execution decides how initial images and subsequent messages are delivered; the helper only assembles and transfers ownership.

### 7. ice-rust today

Native build_initial_message clones parsed.messages, removes its first element from the clone, joins optional stdin/file text/first message with no separator and returns (Option<String>, Vec<String>). App converts empty aggregate file text to None before calling it. Images are passed separately into mode execution, so immutable Args plus an explicit remaining vector is a sound native representation of the live CLI contract. Direct helper calls with Some("") file text differ from TS's truthy fileText check, but the native app normalizes that case before use. Inline tests cover ordering, no messages, no inputs and one-message shifting (rust:crates/ice-cli/src/app.rs:344-350; rust:crates/ice-cli/src/app.rs:858-886; rust:crates/ice-cli/src/app.rs:952-986).

### 8. Port notes

Keep the native clone-and-return representation and exact concatenation/order. Document direct helper empty-file-text normalization if exposing it as a public API, and preserve empty first messages plus separate image ownership. No replacement implementation is needed for the live startup concatenation itself; downstream attachment and session delivery differences are owned elsewhere.

### 9. Size and priority

Estimated Rust implementation: **0-25 lines** (estimate, not measured source). **P2** - The live native assembly already matches the meaningful startup behavior; only helper representation/empty-option edges differ.

## CLI-011 - Credential-print command grammar, output discipline and duration parsing defect

### 1. Title and status

Credential-print command grammar, output discipline and duration parsing defect. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/cli/credential-print.ts:6-76; ice:packages/coding-agent/src/main.ts:117-160; ice:packages/coding-agent/test/credential-print.test.ts:72-103; rust:crates/ice-cli/src/auth.rs:35-65.

Named export: `CredentialPrintKind` (`ice:packages/coding-agent/src/cli/credential-print.ts:6`).

Named export: `CredentialPrintCommand` (`ice:packages/coding-agent/src/cli/credential-print.ts:10`).

Named export: `printCredentialPrintHelp` (`ice:packages/coding-agent/src/cli/credential-print.ts:24`).

Named export: `validateCredentialPrintArgs` (`ice:packages/coding-agent/src/cli/credential-print.ts:66`).

### 3. What it does

Credential-print commands resolve a configured credential for external scripts and write only the value to stdout. They use an early, restricted command path so normal session startup output does not contaminate the result.

### 4. How it works

Credential printing is handled before ordinary startup migrations and session creation. It has its own help, validation and error boundary, while sharing the normal model/auth resolver for the actual credential.

isCredentialPrintHelp accepts argv[0]='auth' with an absent second token or exact help/--help/-h, regardless of trailing tokens. Otherwise parseCredentialPrintCommand recognizes only print-api-key→api_key and print-bearer-token→bearer_token; any other auth subcommand throws CredentialPrintError. The parser removes exact '--min-expiry' plus its next token from the normal args, permits it only for bearer mode, and uses the last repeated value. Other spellings such as '--min-expiry=30m' pass through to normal parsing and are subsequently rejected as unknown flags (ice:packages/coding-agent/src/cli/credential-print.ts:18-64).

The duration regexp accepts decimal digits followed by ms/s/m/h with case-insensitive matching, but the multiplier compares the captured unit case-sensitively. Thus '30S' or '30M' is accepted and multiplied as hours; 'MS' similarly falls into the hours branch. This is a concrete source defect, not intended minimum-expiry behavior. Zero is accepted and no safe-integer/finite cap exists, so extremely large numeric text can produce an infinite value. No whitespace, sign or decimal fractions are accepted. Default bearer minimum expiry is 30 minutes (ice:packages/coding-agent/src/cli/credential-print.ts:8-13; ice:packages/coding-agent/src/cli/credential-print.ts:43-63).

Validation requires a nonblank model, forbids any defined apiKey and rejects positional messages, file arguments and nonempty unknownFlags. It does not reject every other recognized normal flag despite the message saying only provider/model are accepted. Main treats any parse diagnostic, including a warning, as failure. It creates ModelRuntime with allowModelNetwork=false and one AbortSignal.timeout(15000) shared through runtime creation and credential resolution, writes the returned credential plus '\n' directly to stdout, and returns true. CredentialPrintError messages are printed to stderr; other errors use a generic message to avoid leaking resolver details. Failure sets process.exitCode=1 rather than immediately exiting (ice:packages/coding-agent/src/cli/credential-print.ts:66-76; ice:packages/coding-agent/src/main.ts:117-160).

### 5. Interfaces

'ice auth print-api-key --model <model> [--provider <provider>]' and 'ice auth print-bearer-token ... [--min-expiry <duration>]'. Success: raw credential plus one LF, no JSON/banner. Failure: stderr plus exitCode 1. Model-network catalog discovery is disabled, but OAuth request-auth refresh can still occur through CLI-012.

### 6. Dependencies

`AUTH-004`, `CLI-003`, `CLI-012`

CLI-012 owns candidate-provider inference and request-auth extraction. AUTH-004 owns refresh/credential persistence; disabling model discovery is not a blanket network prohibition on an explicitly requested credential refresh.

### 7. ice-rust today

Native auth dispatch supports login, check/status, refresh, logout and help. Other subcommands, including these two print names, produce an unknown-command error and code 2. Native check JSON reports readiness/source; it is not a credential-value endpoint (rust:crates/ice-cli/src/auth.rs:35-65; rust:crates/ice-cli/src/auth.rs:115-150). The required early output discipline, duration option and one credential-result boundary are missing.

### 8. Port notes

Add the two explicit print routes before session/runtime output, using the shared native credential resolver and a bounded cancellation/deadline context. Normalize the duration unit before multiplying and reject nonfinite/overflowing durations; decide whether zero is supported. Replace TS's misleading partial argument whitelist with an explicit accepted set, documenting any compatibility change. Preserve credential-only stdout and generic handling of unexpected resolver failures. Do not copy the uppercase-unit defect.

### 9. Size and priority

Estimated Rust implementation: **100-190 lines** (estimate, not measured source). **P1** - Scripts need a deterministic credential-only command; parser errors and incidental startup output can otherwise break integrations or expose diagnostics.

## CLI-012 - Credential-print provider inference, refresh policy and ambiguity handling

### 1. Title and status

Credential-print provider inference, refresh policy and ambiguity handling. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/cli/credential-print.ts:78-151; ice:packages/coding-agent/test/credential-print.test.ts:13-103; rust:crates/ice-cli/src/auth.rs:44-65.

### 3. What it does

Credential printing chooses one provider/model request credential, using configured credential metadata to avoid guessing among unrelated providers. It rejects ambiguous usable matches and refreshes OAuth through the same request-auth path used for model calls.

### 4. How it works

The resolver first validates the CLI arguments and lists credential metadata. It uses that metadata to select eligible providers, then resolves and extracts a single usable value without exposing candidate credentials in an ambiguity error.

A Map of providerId→credential type is built from modelRuntime.listCredentials({signal}). With a truthy explicit provider, resolveCliModel receives the requested provider/model and any error or absent model becomes CredentialPrintError. Without a provider, iterate getProviders in registry order, skip providers absent from the credential metadata map, and resolve the model independently under each provider. Accept a model only without an error and without a warning containing the exact substring 'Using custom model id'. No accepted model is an explicit not-found error. Explicit provider resolution can admit a custom model; inferred candidates deliberately do not (ice:packages/coding-agent/src/cli/credential-print.ts:91-114).

Candidates are processed sequentially. api_key mode skips a provider whose credential type is oauth but allows another or absent type; bearer mode requires type oauth. Each eligible model calls getAuth with the shared signal and, only for bearer mode, minOAuthValidityMs equal to the supplied duration or 1,800,000. The comment about a fixed five-minute refresh does not describe this caller's requested validity. It finds the first header whose name lowercases to authorization, parses a case-insensitive 'Bearer' followed by whitespace and nonempty content, and chooses auth.apiKey ?? parsedBearer for bearer mode or only auth.apiKey for key mode. An empty apiKey suppresses header fallback through nullish selection, then is rejected by the truthy-value check. Values are not additionally trimmed (ice:packages/coding-agent/src/cli/credential-print.ts:116-132).

Exactly one usable value succeeds. Zero values yields a tailored wrong-credential-type error when an explicit provider was supplied, otherwise a generic no-usable-credential error. More than one usable candidate is ambiguous even when their credential strings happen to be equal; the error lists provider IDs only. Resolver/getAuth exceptions are not swallowed per candidate and abort the entire operation through main's generic error boundary. Tests exercise API keys, Authorization-header bearer extraction, expired-token refresh/persistence and explicit type rejection; source inspection does not imply the tests ran (ice:packages/coding-agent/src/cli/credential-print.ts:134-151; ice:packages/coding-agent/test/credential-print.test.ts:22-103).

### 5. Interfaces

resolveCredentialForPrint(args, modelRuntime,'api_key'|'bearer_token', minExpiryMs?, signal?)→Promise<string>. Uses listCredentials/getProviders/resolveCliModel/getAuth. Does not return a list of secrets, select the first ambiguous candidate, or persist the CLI apiKey flag.

### 6. Dependencies

`AUTH-003`, `AUTH-004`, `AUTH-024`, `AUTH-025`, `CLI-011`

AUTH-003 provides credential metadata enumeration, AUTH-004 request-auth refresh and storage preservation, AUTH-024/025 model reference/matching semantics. CLI owns only the single-value resolution policy.

### 7. ice-rust today

Native auth commands can inspect readiness, perform provider-owned OAuth refresh and store credentials, but the dispatch has no print-value command or this model-based candidate inference. A native provider credential source is not sufficient without the explicit type filtering, minimum-validity request, header fallback and ambiguity contract (rust:crates/ice-cli/src/auth.rs:44-65; rust:crates/ice-cli/src/auth.rs:115-150). Provider-specific OAuth implementations remain separately verified/unverified under AUTH; this entry does not assume concrete flows exist merely because generic host hooks do.

### 8. Port notes

Implement candidate inference using the native provider catalog and credential metadata, then call AUTH's resolver rather than reading auth.json directly. Preserve type filtering, requested validity, cancellation and ambiguity. Use a structured custom-model indicator instead of depending on a human warning substring when designing the native API, while matching observed TS selection behavior. Retain stored credentials on transient refresh failure according to AUTH-004.

### 9. Size and priority

Estimated Rust implementation: **100-190 lines** (estimate, not measured source). **P1** - Returning a credential from the wrong provider or skipping refresh breaks consumers; the inference and failure rules are more specific than ordinary auth readiness checks.

## CLI-013 - Credential startup migration and recoverable native layout migration

### 1. Title and status

Credential startup migration and recoverable native layout migration. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/migrations.ts:15-72; ice:packages/coding-agent/test/config-value-migration.test.ts:41-87; rust:crates/ice-cli/src/config.rs:169-225; rust:crates/ice-cli/src/config.rs:260-269; rust:crates/ice-cli/src/config.rs:408-416.

### 3. What it does

Startup migrations convert old credential storage into the current location without changing current literal configuration values. Both implementations contain failure-order problems that must be repaired when carrying migration support forward.

### 4. How it works

The TS conversion is skipped entirely when auth.json already exists. Otherwise it gathers old OAuth entries and settings API keys into one object, then writes the new credential file after changing the old sources.

For oauth.json, parse JSON and iterate Object.entries without validating each credential shape; assign {type:'oauth',...cred}, so a supplied cred.type can override the inserted discriminator, append provider names and rename the old file to oauth.json.migrated. All errors in that block are swallowed, including a failed rename after values were already gathered. For settings.json, parse it and inspect truthy object-valued apiKeys; arrays are not explicitly rejected. Add only string values whose provider currently has no truthy entry, so gathered OAuth entries take precedence. Delete settings.apiKeys and rewrite settings with two-space JSON. This block also swallows errors. If the gathered object is nonempty, mkdir and write auth.json with mode 0600 occur outside both catches; an error propagates and prevents later migrations (ice:packages/coding-agent/src/migrations.ts:20-72).

The order is not transactional: a successfully rewritten settings file may no longer contain its API keys when final auth.json creation fails, and oauth.json may already have been renamed. This is a source-derived data-loss/recovery risk, not a reproduced runtime incident. Existing tests intentionally leave current uppercase API-key/header strings unchanged, leave malformed/blank models.json for its normal parser, and require no automatic conversion of bare uppercase literals into environment references. They do not validate final-write-failure rollback (ice:packages/coding-agent/test/config-value-migration.test.ts:41-177; ice:packages/coding-agent/test/suite/regressions/5661-uppercase-header-values.test.ts:33-90).

### 5. Interfaces

migrateAuthToAuthJson()→string[] migrated provider names. Inputs are only current agentDir oauth.json/settings.json, output auth.json, and OAuth backup suffix '.migrated'. runMigrations invokes this before session/tools/keybindings/resource migration. Actual user credential files were not read during this audit.

### 6. Dependencies

`AUTH-001`, `AUTH-002`, `CLI-014`, `CLI-018`, `SET-005`, `SET-006`

AUTH owns credential schema/current store writes; SET owns literal/environment/command config resolution. Migration must not reinterpret bare uppercase strings or infer that old namespaces are aliases.

### 7. ice-rust today

Rust migrates a different layout: default flat config-root auth.json/models.json/.setup_done into agent/. It skips whenever CONFIG_DIR_ENV is set, even empty, and when agent already exists. It only begins when flat auth or models exists, creates agent, attempts rename then copy/remove for each file, and increments moved even if both rename and copy fail. Since a created agent directory causes the next call to return 0, partial transfers do not resume despite the comment. app discards the Result; read_auth/load_models_config pass None as the fallback path, so stranded flat files are not read through that helper. This concrete failure/count defect is distinct from TS's credential-format conversion (rust:crates/ice-cli/src/config.rs:176-225; rust:crates/ice-cli/src/app.rs:187-191; rust:crates/ice-cli/src/config.rs:260-269; rust:crates/ice-cli/src/config.rs:408-416). Inline tests cover successful moves and existing-directory/no-input/env gates, not failed transfer recovery (rust:crates/ice-cli/src/config.rs:1565-1611).

### 8. Port notes

Implement credential-format conversion as a staged, validated transaction: prepare and atomically publish the new auth store before retiring old sources, keep recoverable backups, and report failed finalization. For native flat-layout migration, decide per-file completion rather than agent-directory existence, increment counts only after verified transfer, and retain explicit failures for retry. Preserve existing destination data and override boundaries. Do not copy either implementation's failure-order defect or read real home state as part of this audit.

### 9. Size and priority

Estimated Rust implementation: **130-240 lines** (estimate, not measured source). **P1** - Migration failure can strand credentials or remove their last normal lookup location; startup must remain recoverable without silently claiming success.

## CLI-014 - Session, managed-binary, prompt-directory and keybinding startup migrations

### 1. Title and status

Session, managed-binary, prompt-directory and keybinding startup migrations. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/migrations.ts:74-314; ice:packages/coding-agent/src/main.ts:628-641; ice:packages/coding-agent/src/main.ts:872-875; rust:crates/ice-cli/src/config.rs:169-225.

Named export: `migrateSessionsFromAgentRoot` (`ice:packages/coding-agent/src/migrations.ts:83`).

### 3. What it does

The startup migration sequence repairs several old layouts and reports deprecated extension directories. Each step has a separate scope and failure policy, and some project filesystem changes occur before the normal trust decision.

### 4. How it works

runMigrations calls credential migration, flat-session migration, managed-binary relocation, keybinding-file migration and extension-system migration in that exact order. Its result is migratedAuthProviders plus deprecationWarnings; main later displays warnings only in interactive mode.

Session migration scans only *.jsonl directly in getAgentDir. It reads each entire file to inspect the first line, skips blank/invalid/non-session/missing-cwd headers, computes '--'+cwd with one leading slash/backslash removed and remaining slash/backslash/colon replaced by '-'+ '--', creates agentDir/sessions/encoded-cwd, skips an existing target and renames. Directory scan failure returns; each file's errors are swallowed. The filename expression file.split('/').pop() || file.split('\\').pop() is incorrect for an ordinary Windows backslash path: the first split already returns the entire truthy path, so the fallback never supplies a basename. This is a source-only platform defect; use a path basename operation in a port (ice:packages/coding-agent/src/migrations.ts:83-130).

Managed binaries are exactly fd, rg, fd.exe, rg.exe in agentDir/tools. Create getBinDir if needed, move a missing destination, or remove the old copy when the destination exists. Move/remove errors are ignored, but mkdir is outside their catch and can abort the sequence. A success message appears only after at least one rename; tools/ itself is not removed. Keybinding migration parses only a nonnull nonarray object, calls migrateKeybindingsConfig, writes two-space JSON plus LF only when changed, and swallows read/parse/write errors. TUI-048 owns the exact historical key translation (ice:packages/coding-agent/src/migrations.ts:156-215).

Extension migration attempts global commands→prompts, then project commands→prompts, only when source exists and target does not; directories or symlinks are renamed, and failures are printed as warnings. It then collects global and project warnings for hooks/ and for tools/ entries other than case-insensitive fd/rg/exe names and dot-prefixed names. The main call at line 628 precedes project trust resolution around lines 677 onward, so the project directory rename is not gated by the later project trust decision. The deprecation display prints warnings/guide URLs, enables raw input when supported, waits for one data event, disables raw mode and pauses stdin; it has no error/EOF listener (ice:packages/coding-agent/src/migrations.ts:136-154; ice:packages/coding-agent/src/migrations.ts:221-314; ice:packages/coding-agent/src/main.ts:628-641; ice:packages/coding-agent/src/main.ts:677-705; ice:packages/coding-agent/src/main.ts:872-875).

### 5. Interfaces

runMigrations(cwd)→{migratedAuthProviders:string[], deprecationWarnings:string[]}. Inputs include global agentDir and project configuration directory for the supplied launch cwd. Targets are sessions/<encoded-cwd>, bin, prompts and keybindings.json. showDeprecationWarnings returns immediately for an empty list; otherwise awaits a keypress.

### 6. Dependencies

`CLI-005`, `CLI-013`, `CLI-018`, `MISC-010`, `SESSION-029`, `SESSION-032`, `SET-003`, `TUI-048`

SESSION owns actual session format migration/discovery, TUI-048 pure keybinding renames, SET-003 trust decisions. This entry owns on-disk invocation, ordering, boundaries and failures; managed-tool installation is a separate MISC dependency.

### 7. ice-rust today

The inspected native startup invokes only config::migrate_legacy_layout at this stage, which moves three flat credential/model/setup files. It does not implement these TS session-placement, fd/rg relocation, commands/prompts or keybinding-file transformations. Native session storage has its own format/version and current project-root layout, so renaming a TS file alone would not make it loadable (rust:crates/ice-cli/src/app.rs:187-203; rust:crates/ice-cli/src/config.rs:169-225; rust:crates/ice-cli/src/session.rs:154-192). Existing native configuration and package discovery are retained rather than replaced by these legacy helpers.

### 8. Port notes

Implement each needed migration as a separately recorded, repeatable operation with explicit source/destination validation and error reporting. Use platform basename logic, keep existing targets intact, and connect session format conversion to SESSION-029 rather than blindly relocating incompatible logs. Gate project mutations at the intended trust stage instead of inheriting an implicit pre-trust rename. Give interactive warning acknowledgement an EOF/error/cancellation exit so it cannot wait forever on a closed terminal.

### 9. Size and priority

Estimated Rust implementation: **160-300 lines** (estimate, not measured source). **P2** - Legacy users need predictable migration, but the current functions contain swallowed failures and a Windows path defect that should not be preserved.

## CLI-015 - Official first-run gate and startup theme bootstrap

### 1. Title and status

Official first-run gate and startup theme bootstrap. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/cli/startup-ui.ts:26-132; ice:packages/coding-agent/src/cli/startup-ui.ts:165-205; ice:packages/coding-agent/test/first-time-setup.test.ts:1-95; ice:packages/coding-agent/test/first-time-setup-fork.test.ts:1-39; rust:crates/ice-cli/src/extras.rs:1-94.

`areExperimentalFeaturesEnabled` — `ice:packages/coding-agent/src/core/experimental.ts:3`.

### 3. What it does

The TS first-run dialog is a narrowly gated onboarding flow that selects a theme and analytics preference. Startup selectors share a smaller theme loader that avoids relying on project resources before trust is resolved.

### 4. How it works

First-run setup is shown only for the official experimental distribution with no existing settings file. Startup theme preparation uses global settings and installed enabled themes, then adapts to the terminal's background without blocking normal startup on a broken theme.

Official identity requires the exact triple @zykairotis/ice-coding-agent, ice and .ice. shouldRunFirstTimeSetup returns false for any other triple, when experimental features are disabled, or when process.env[ENV_AGENT_DIR] is truthy; an empty override does not suppress it. Otherwise it returns !existsSync(settingsPath). Main additionally requires interactive mode and neither help nor list-models, and invokes it after migrations but before session selection (ice:packages/coding-agent/src/cli/startup-ui.ts:26-42; ice:packages/coding-agent/src/cli/startup-ui.ts:108-132; ice:packages/coding-agent/src/main.ts:632-641).

loadStartupThemes constructs in-memory settings from the global snapshot with projectTrusted=false, creates a package manager at process cwd/agent dir and resolves missing packages with an async 'skip' answer. Only enabled theme resources are loaded. A duplicate truthy theme name is first-wins; unnamed themes are not deduplicated. Theme load errors are ignored for later normal diagnostics. createStartupTui registers these themes, selects the environment-detected background and resolved setting, installs global keybindings, and creates a TuiMainScreen with hardware-cursor and clear-on-shrink settings. startStartupTui starts the terminal then fires an unawaited detection update only for absent/auto theme settings; detection is capped at 100 ms, then setTheme/invalidate/requestRender run without a disposal-generation check (ice:packages/coding-agent/src/cli/startup-ui.ts:44-100).

The first-time dialog starts UI, awaits its own 100-ms terminal theme detection, sets that theme and installs preview/submit/cancel callbacks. A settled guard is set before asynchronous finalization. Submit sets theme and shareAnalytics, awaits settings.flush, clears/render-waits 25 ms, stops UI and resolves. Cancel omits persistence but follows clear/stop. Normal SettingsManager.flush awaits a queue whose write errors are caught and recorded, so ordinary persistence failure can be recorded while the dialog proceeds (SET-009). There is no outer rejection/finally around finalization: an unexpected rejected finalizer or terminal operation is not propagated by the void callbacks and can leave the resolve-only startup Promise unsettled. This narrower source-derived cleanup risk is not an executed failure (ice:packages/coding-agent/src/cli/startup-ui.ts:102-106; ice:packages/coding-agent/src/cli/startup-ui.ts:165-205).

The shared areExperimentalFeaturesEnabled() gate returns getIceEnv("ICE_EXPERIMENTAL") === "1" on each call. It performs no trimming or caching; the first-run setup caller described above consumes that exact boolean (`ice:packages/coding-agent/src/core/experimental.ts:1-5`).

### 5. Interfaces

shouldRunFirstTimeSetup(settingsPath=getSettingsPath())→boolean; createStartupTui(settings)→Promise<TUI>; startStartupTui(ui, settings)→void; showFirstTimeSetup(settings)→Promise<void>. Persistence uses the normal settings manager. TUI-070 owns the dialog's visual fields and input behavior; this entry owns its startup gate and lifetime.

areExperimentalFeaturesEnabled(): boolean; exact enabling value for ICE_EXPERIMENTAL is "1".

### 6. Dependencies

`CLI-005`, `CLI-016`, `CLI-029`, `CLI-032`, `SET-007`, `SET-009`, `SET-017`, `SET-018`, `TUI-007`, `TUI-048`, `TUI-070`, `TUI-220`

SET-017 owns theme preferences and SET-018 analytics consent/tracking persistence. The package supplement owns startup theme resource resolution, and TUI-070 owns questionnaire presentation. This entry owns the caller gate and terminal lifetime.

### 7. ice-rust today

Native extras uses an agentDir/.setup_done sentinel. Whenever absent, maybe_first_time_setup adds a static welcome/theme/help banner, best-effort writes '1' and returns true. It honors a custom native config-dir path and does not check official distribution, experimental features or settings-file existence. The interactive caller captures existing-profile status, calls this banner and then handles changelog presentation; no theme/analytics questionnaire is wired there. Native in-file tests cover sentinel placement under an override (rust:crates/ice-cli/src/extras.rs:11-64; rust:crates/ice-cli/src/extras.rs:75-93; rust:crates/ice-cli/src/interactive_tui.rs:1009-1028). This is a real first-launch banner with a different gate, not equivalent TS onboarding.

### 8. Port notes

Choose and document the native first-run product policy, then implement the required official/experimental/settings gate if TS compatibility is intended. Reuse native theme/input components for TUI-070 and persist selections through SET. Make asynchronous finalization reject or settle exactly once with guaranteed terminal cleanup, and prevent delayed theme detection from rendering after disposal. Keep the existing banner as a distinct optional native feature rather than counting it as completed onboarding.

### 9. Size and priority

Estimated Rust implementation: **130-250 lines** (estimate, not measured source). **P2** - Onboarding is conditional rather than a universal startup gap, but terminal lifetime and preference persistence need precise behavior.

## CLI-016 - Startup selectors, config wrapper and resume-picker terminal lifetime

### 1. Title and status

Startup selectors, config wrapper and resume-picker terminal lifetime. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/cli/startup-ui.ts:134-163; ice:packages/coding-agent/src/cli/startup-ui.ts:207-239; ice:packages/coding-agent/src/cli/config-selector.ts:1-56; ice:packages/coding-agent/src/cli/session-picker.ts:1-55; rust:crates/ice-cli/src/resume_picker.rs:1-247.

Named export: `ConfigSelectorOptions` (`ice:packages/coding-agent/src/cli/config-selector.ts:10`).

### 3. What it does

Short-lived startup dialogs own the terminal until they choose, cancel or quit, then return control to startup. The wrappers pass settings, scope and asynchronous data loaders into richer TUI components rather than implementing those widgets themselves.

### 4. How it works

Generic startup selector/input wrappers resolve once and stop their temporary TUI after clearing it. Config and session wrappers additionally define which child receives focus and which operation exits the whole process.

showStartupSelector creates the shared startup TUI, passes only option labels to ExtensionSelectorComponent and maps a selected label back with options.find, so duplicate labels choose the first matching value. Its finish marks settled before awaiting clearStartupTui's 25-ms render wait, then stops and resolves; cancellation resolves undefined. showStartupInput has the same guard/order but first disposes its input component, then clears/stops and resolves the string or undefined. Both start through startStartupTui, which may have an asynchronous theme-detection update in flight. There is no outer rejection/finally path around failed asynchronous completion (ice:packages/coding-agent/src/cli/startup-ui.ts:134-163; ice:packages/coding-agent/src/cli/startup-ui.ts:207-239).

selectConfig initializes the current theme with watching enabled, creates a TuiMainScreen with supplied agentDir, and passes resolved resource paths/settings/cwd/agentDir, terminal rows, writeScope ('global'|'project') and projectModeAvailable to ConfigSelectorComponent. Done is guarded, stops UI and theme watcher, then resolves. Quit stops both and process.exit(0). Render requests delegate to UI, and initial focus is the resource list (ice:packages/coding-agent/src/cli/config-selector.ts:10-55).

selectSession awaits startup TUI, creates/installs keybindings and passes current/all-session loaders to SessionSelectorComponent. Selection resolves a session path, cancel resolves null and both stop UI once; quit stops UI and exits 0. It disables the rename hint and focuses the session list. The wrapper itself does not stop the theme watcher, but main's resume branch does so in finally. TUI-062/063/064 own search/list/loading/rename/delete details (ice:packages/coding-agent/src/cli/session-picker.ts:12-55; ice:packages/coding-agent/src/main.ts:371-385).

### 5. Interfaces

showStartupSelector<T>(settings, title,{label, value}[])→Promise<T|undefined>; showStartupInput(settings, title, placeholder?)→Promise<string|undefined>; selectConfig(options)→Promise<void>; selectSession(currentLoader, allLoader, settings)→Promise<string|null>. Scope availability and resource mutation remain the package/TUI component contracts.

### 6. Dependencies

`CLI-006`, `CLI-015`, `TUI-048`, `TUI-062`, `TUI-063`, `TUI-064`, `TUI-068`, `TUI-069`, `TUI-073`

TUI owns widgets, keybindings and rich session operations. The package supplement owns 'ice config' routing and resource resolution, while this entry owns the callable wrapper and terminal cleanup. Generic extension selector/input components need their TUI entry links before publication.

### 7. ice-rust today

Native resume_picker is implemented: it lists project metadata, computes bounded summaries on spawn_blocking, removes empty rows, creates a 12-row SelectList with 56–72-column primary layout, and starts TuiAltScreen with a render scheduler. A blocking crossterm loop polls every 100 ms, handles resize/mouse scroll, filters typed text, clears a nonempty query on Esc and cancels on empty-query Esc or Ctrl+C. It stops the TUI after the input task completes, including task errors. Empty metadata or no resumable rows is an error rather than an empty rich selector. It does not provide current/all loaders, the TS startup config resource wrapper or generic typed label-value/input wrappers (rust:crates/ice-cli/src/resume_picker.rs:16-188). Its tests cover age/label/byte/id formatting; source only (rust:crates/ice-cli/src/resume_picker.rs:190-246).

### 8. Port notes

Reuse the native temporary terminal and selector lifecycle, extending it through the TUI owners for all-project loading and config resources. Add generic typed selector/input adapters with stable value identity rather than label lookup when duplicate labels are allowed. Guarantee stop and callback settlement on every error/cancel path, coordinate delayed theme work with disposal, and preserve the explicit distinction between cancel-return and process quit.

### 9. Size and priority

Estimated Rust implementation: **100-220 lines** (estimate, not measured source). **P2** - A native resume picker exists, but startup configuration and consistent temporary-terminal cleanup need additional integration.

## CLI-017 - Available-model listing, fuzzy filtering and stable tabular output

### 1. Title and status

Available-model listing, fuzzy filtering and stable tabular output. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/cli/list-models.ts:1-115; ice:packages/coding-agent/src/main.ts:847-858; rust:crates/ice-cli/src/app.rs:197-203; rust:crates/ice-cli/src/app.rs:652-768.

### 3. What it does

The TS list-models command reports models usable with current runtime credentials, then filters and sorts the result for display. The native command intentionally inspects a broader catalog before authenticated session creation, which is useful but not the same contract.

### 4. How it works

Model listing warns about configuration load errors before querying availability. It then prints a six-column table or a clear empty-result message, with provider/model ordering independent of fuzzy-match score.

listModels first reads modelRuntime.getError and prints a yellow models.json warning when present. It awaits getAvailable(undefined,{signal}) and copies the result. A completely empty available set prints the shared no-models guidance and returns before filtering. A truthy search pattern uses fuzzyFilter over only 'provider id', not display name. Zero filtered matches prints the exact requested pattern. Remaining models sort by provider.localeCompare, then id.localeCompare, so fuzzy relevance does not control final row ordering (ice:packages/coding-agent/src/cli/list-models.ts:29-62).

Rows contain provider, model ID, context window, max output, reasoning yes/no and image-input yes/no. Token counts use M at one million or above, K at one thousand or above, with no decimal for an exact integral multiple and one decimal otherwise. Column widths use the maximum header/string length, each cell is padEnd-aligned and columns join with two spaces. Main passes a 15-second signal and a string search only when listModels is a string, after runtime/extensions are created but before ordinary startup diagnostic failure handling (ice:packages/coding-agent/src/cli/list-models.ts:14-24; ice:packages/coding-agent/src/cli/list-models.ts:64-115; ice:packages/coding-agent/src/main.ts:847-858).

### 5. Interfaces

listModels(modelRuntime, searchPattern?, signal?)→Promise<void>; --list-models optionally consumes one nonflag/nonfile search token. Table headers are provider, model, context, max-out, thinking, images. Output uses no JSON schema. Availability and fuzzy matching are dependency contracts, not inferred from catalog membership.

### 6. Dependencies

`AI-002`, `AUTH-006`, `AUTH-024`, `CLI-003`, `CLI-004`, `TUI-235`

AUTH-006 owns availability with credential metadata, AUTH-024 model reference matching, AI-002 catalog/runtime lifecycle. The actual fuzzyFilter utility is low-level TUI-owned; this entry records its key and subsequent sorting.

### 7. ice-rust today

Native list_models runs before provider credentials, session restoration and harness creation. It prepares extensions, obtains their manifests and loads the merged catalog. Config error warns and yields an empty catalog. Search trims and ASCII-lowercases a substring over provider, id, name; it does not call TS fuzzyFilter or available-model filtering. It retains catalog order without the explicit provider/ID locale sort. The six columns and K/M formatting are substantially equivalent, using native byte lengths for computed widths. Its broader unauthenticated catalog inspection is explicitly stated at the call site and should be preserved as a deliberate mode if desired (rust:crates/ice-cli/src/app.rs:197-203; rust:crates/ice-cli/src/app.rs:654-768).

### 8. Port notes

Provide a clear choice between available models and all catalog models, rather than silently equating the two. For TS-compatible --list-models, use AUTH availability, the agreed fuzzy matcher and deterministic provider/model sorting, with the same empty/error output and cancellation. Retain native unauthenticated catalog inspection under an explicit documented option or deliberate compatibility decision.

### 9. Size and priority

Estimated Rust implementation: **65-130 lines** (estimate, not measured source). **P2** - Both list models, but filtering and availability semantics differ enough to mislead users about which model can actually run.

## CLI-018 - Exact identity helpers, canonical paths and deliberately limited legacy compatibility

### 1. Title and status

Exact identity helpers, canonical paths and deliberately limited legacy compatibility. **Status: different-by-design.**

### 2. ice source

ice:packages/coding-agent/src/core/legacy-compat/env.ts:1-31; ice:packages/coding-agent/src/core/legacy-compat/identity.ts:1-10; ice:packages/coding-agent/src/core/legacy-compat/paths.ts:1-6; ice:packages/coding-agent/src/core/legacy-compat/cognee.ts:1-9; ice:packages/coding-agent/src/core/legacy-compat/extension-aliases.ts:1-36.

### 3. What it does

The current compatibility helpers preserve exact current names and paths rather than broadly translating old Pi or PIV state. Historical module imports are supported through a small explicit alias table, which is separate from environment, settings and storage migration.

### 4. How it works

Canonical configuration lookup does not search old product trees or reinterpret namespace names. The only broad-looking legacy helpers are shallow copies; actual module aliases are explicit data consumed by the extension loader.

getCompatibleConfigEnv checks an own property of the scoped map first when its value is not undefined, then the same exact own name in ambient environment, otherwise undefined. Empty strings are valid and override ambient values; scoped undefined falls back. getIceEnv reads only an own property of the supplied exact name. normalizeLegacyEnvironment shallow-copies all enumerable own fields and does not create ICE aliases. clearSessionEnvironment deletes exactly ICE_SESSION_ID, ICE_SESSION_FILE, ICE_PROVIDER, ICE_MODEL, ICE_REASONING_LEVEL (ice:packages/coding-agent/src/core/legacy-compat/env.ts:1-31).

matchesEntryType is strict equality. normalizeLegacySettings rejects null, arrays and nonobjects, then returns a shallow copy without converting a piv namespace. compatibleConfigRoot simply joins parent and configDirName (default '.ice'); it does not inspect existence, legacy .pi directories or dangling canonical symlinks. Cognee and Blackhole helpers return agentDir/ice-cognee and agentDir/ice-blackhole/ice-blackhole-config.json. Tests explicitly reject implicit PIV/PI environment promotion, historical journal-envelope equality and old-tree fallback while preserving exact-name scoped precedence (ice:packages/coding-agent/src/core/legacy-compat/identity.ts:1-10; ice:packages/coding-agent/src/core/legacy-compat/paths.ts:1-6; ice:packages/coding-agent/src/core/legacy-compat/cognee.ts:1-9; ice:packages/coding-agent/test/legacy-compat/identity-inputs.test.ts:5-34; ice:packages/coding-agent/test/legacy-compat/paths-and-cli.test.ts:17-40; ice:packages/coding-agent/test/legacy-compat/config-references.test.ts:11-46).

The alias table contains seven exact suffixes for each of @earendil-works and @mariozechner: pi-coding-agent→@zykairotis/ice-coding-agent, pi-agent-core→ice-agent-core, pi-tui→ice-tui, pi-ai→ice-ai, pi-ai/compat→ice-ai/compat, pi-ai/oauth→ice-ai/oauth, and pi-ai/providers/all→ice-ai/providers/all, with the @zykairotis scope on every target. The data module does no filesystem or loader work. EXT-002 owns runtime-specific resolution and unsupported-subpath rejection. The full built-runtime test source checks opt-in Node/Bun artifacts and exact aliases, but was not executed or treated as proof that built outputs are fresh (ice:packages/coding-agent/src/core/legacy-compat/extension-aliases.ts:17-36; ice:packages/coding-agent/test/legacy-compat/built-runtime-smoke.test.ts:10-24; ice:packages/coding-agent/test/legacy-compat/built-runtime-smoke.test.ts:62-212).

### 5. Interfaces

CompatibilityEnvironment is Readonly<Record<string, string|undefined>>. Helpers are exact-name operations with shallow-copy semantics. The one launcher PI_CODING_AGENT_DIR nullish bridge belongs to CLI-002, and current on-disk transformations belong to CLI-013/014. No general old-home-state import is implied.

### 6. Dependencies

`CLI-001`, `CLI-002`, `CLI-013`, `CLI-014`, `EXT-001`, `EXT-002`, `SET-001`, `SET-005`, `SET-010`

SET-001 owns current distribution paths, SET-005 config expression lookup and SET-010 actual settings migration. EXT-002 owns historical module aliases. Cognee/Blackhole storage users keep their existing area contracts.

### 7. ice-rust today

Native agent_dir uses exactly ICERS_CODING_AGENT_DIR when present and absolute, otherwise home/.icers/agent; its default session root is cwd/.icers/sessions. These are canonical native rules, with a separate default flat-layout migration rather than implicit .pi/PIV fallback (rust:crates/ice-cli/src/config.rs:89-108; rust:crates/ice-cli/src/config.rs:176-225; rust:crates/ice-cli/src/session.rs:189-192). Rust typed settings/environment APIs do not need literal JS shallow-copy functions, but must preserve their exact-name contract at compatibility boundaries. Native package/extension interoperability is assessed by EXT-001/002 and the package supplement; no blanket 'Rust cannot load any Pi package' conclusion is drawn from these helper files.

The active native names deliberately differ from the TypeScript ice/.ice/ICE_* names; these path and environment rules do not establish automatic migration of historical TypeScript or old-native directories.

### 8. Port notes

Preserve canonical-name and no-implicit-old-tree behavior in native configuration and migration APIs. Retain exact supported module aliases through the real bridge/loader owner, without wildcard prefix rewrites or guessed subpaths. Do not implement the first-pass claim of broad old environment/path mapping because the frozen TS code intentionally does not do it. Document any deliberate native path-layout difference under SET/SESSION rather than hiding it behind 'legacy compatibility'.

### 9. Size and priority

Estimated Rust implementation: **20-80 lines** (estimate, not measured source). **P2** - The main audit correction is avoiding an invented migration feature; actual alias support and small identity boundaries remain important.

## CLI-019 - Offline normalization, startup diagnostics, output ownership and shutdown integration

### 1. Title and status

Offline normalization, startup diagnostics, output ownership and shutdown integration. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/main.ts:73-115; ice:packages/coding-agent/src/main.ts:530-552; ice:packages/coding-agent/src/main.ts:615-627; ice:packages/coding-agent/src/main.ts:861-1007; rust:crates/ice-cli/src/app.rs:73-103; rust:crates/ice-cli/src/app.rs:315-520.

### 3. What it does

Startup coordinates output ownership, optional recording/dashboard services and mode shutdown so protocol output remains usable. Offline and benchmark flags also change startup work before the selected mode begins.

### 4. How it works

Offline mode is normalized before early command routing, and stream ownership is chosen before migrations or runtime creation can print. Mode execution later shares one recorder/dashboard lifetime with an explicit cleanup boundary.

Main checks raw args.includes('--offline') or envTruthy(ICE_OFFLINE) and, when enabled, assigns both ICE_OFFLINE='1' and ICE_SKIP_VERSION_CHECK='1'. This raw scan is independent of whether the normal parser would consume that token as another flag's value. On win32 it then invokes self-update quarantine cleanup, captures launch cwd and applies bootstrap global proxy settings. SET-013 owns proxy/dispatcher behavior; MISC-002 owns Windows native-dependency quarantine cleanup. reportDiagnostics sends errors, warnings and neutral diagnostics to stderr with red/yellow/dim coloring and matching prefixes; settings errors are drained into warning values annotated with startup/runtime and scope (ice:packages/coding-agent/src/main.ts:73-94; ice:packages/coding-agent/src/main.ts:530-552).

Noninteractive mode takes over stdout unless it is the exact plain-runtime-metadata predicate. Interactive nonmetadata startup enables extension console capture. After runtime creation, help/list can exit before normal runtime error reporting. For a later stdin-driven downgrade to print, captured console output is disabled/drained to stderr. Once input/theme setup is complete, interactive mode alone shows deprecation warnings; then runtime diagnostics print, any error exits 1, and an extension-load error adds the '-ne' hint. These are ordered caller actions; EXT-007 owns capture buffering and RPC-017/output guard owns the lower stream mechanism (ice:packages/coding-agent/src/main.ts:615-627; ice:packages/coding-agent/src/main.ts:847-887).

Nonoffline RPC starts a background modelRuntime.refresh with a 15-second AbortController timer, ignores failure and clears the timer in finally. Every ordinary agent run then attempts to open agentDir/observability.sqlite and start one recorder with Cognee observations; failure warns and continues. Optional --observe starts the dashboard with that recorder/replay store, agentDir/observe.token and cwd-bound Cognee source. Its URL is queued as an interactive notice or printed to stderr. Dashboard startup failure stops the recorder and rethrows before the mode try/finally. OBS-003/004/014/016 own the services themselves (ice:packages/coding-agent/src/main.ts:896-946).

The mode try prints timings and invokes RPC, interactive or print with the initial/remaining messages. ICE_STARTUP_BENCHMARK uses the same truthy convention, errors outside interactive mode, initializes the TUI without running a conversation, waits 150 ms for terminal replies, stops UI/theme watching, prints timings and waits for stdout then stderr drain only when each writableLength is positive. Print-mode completion stops theme watching, restores stdout, sets a nonzero exitCode when returned, then returns. The final block closes an observeServer when present, otherwise stops the recorder. It is not a general runtime.dispose finally; individual modes own their own runtime/terminal cleanup (ice:packages/coding-agent/src/main.ts:890-895; ice:packages/coding-agent/src/main.ts:948-1007).

### 5. Interfaces

Environment surfaces: ICE_OFFLINE, ICE_SKIP_VERSION_CHECK, ICE_STARTUP_BENCHMARK. --observe is true or a validated port; dashboard token/history paths are agentDir-relative. Startup errors and notices route to stderr unless explicitly buffered for TUI. No tests or benchmark were executed during the audit.

### 6. Dependencies

`AI-002`, `CLI-003`, `CLI-004`, `CLI-005`, `CLI-008`, `CLI-014`, `EXT-007`, `MISC-001`, `MISC-002`, `OBS-002`, `OBS-003`, `OBS-004`, `OBS-014`, `OBS-016`, `RPC-017`, `RPC-020`, `SET-013`

OBS owns timing, recording, token/HTTP and shared-port behavior. SET-013 owns proxy/idle preferences and the HTTP dispatcher; MISC-001 owns version metadata/checks and MISC-002 Windows loaded native-dependency quarantine. RPC-020 owns JSONL shutdown/rebinding, not every CLI mode.

### 7. ice-rust today

Native run wraps run_inner and prints timing after it returns. Native offline normalization sets ICERS_OFFLINE but not the TS ICE_SKIP_VERSION_CHECK marker. It has its own --connect early route, static warnings only under verbose, plugin-owned stdio validation, ICERS_SKIP_STDIN/ICERS_FORCE_TUI controls, and cleanup of dev watcher/context/harness after mode execution. RPC/server dispatch contains real extension lifecycle and JSONL/plugin-stdio choices (rust:crates/ice-cli/src/app.rs:73-103; rust:crates/ice-cli/src/args.rs:31-71; rust:crates/ice-cli/src/app.rs:315-381; rust:crates/ice-cli/src/app.rs:434-520; rust:crates/ice-cli/src/app.rs:640-650). The startup path lacks the TS recorder/dashboard, extension console buffering, benchmark and stdout takeover sequence; these are specific integrations rather than absence of all cleanup or timing.

### 8. Port notes

Integrate the existing native modes with explicit output ownership and one cleanup owner for each service. Reuse OBS's recorder/dashboard without duplicating subscriptions, preserve plugin stdio ownership, and make offline gating consistent across raw preflight and parsed commands. Implement the startup benchmark only as a bounded initialization path with guaranteed terminal restoration, rather than a hidden conversation run. Keep stream restoration and runtime disposal contracts visible at mode boundaries.

### 9. Size and priority

Estimated Rust implementation: **140-280 lines** (estimate, not measured source). **P1** - Startup output contamination and mismatched cleanup can break RPC/scripts; several native lifecycle pieces already exist and should be preserved.

## CLI-020 - Package source grammar, canonical identity and scoped local-path resolution

### 1. Title and status

Package source grammar, canonical identity and scoped local-path resolution. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/core/package-manager.ts:1352-1789; ice:packages/coding-agent/src/utils/git.ts:1-227; ice:packages/coding-agent/src/utils/paths.ts:1-119; ice:packages/coding-agent/test/git-ssh-url.test.ts:1-92; ice:packages/coding-agent/test/paths.test.ts:1-151; rust:crates/ice-cli/src/install.rs:1075-1300; rust:crates/ice-cli/src/packages.rs:48-136.; ice:packages/coding-agent/src/core/package-manager.ts:2008-2136.

`canonicalizePath` — `ice:packages/coding-agent/src/utils/paths.ts:28`. `isLocalPath` — `ice:packages/coding-agent/src/utils/paths.ts:41`. `formatPathRelativeToCwdOrAbsolute` — `ice:packages/coding-agent/src/utils/paths.ts:98`.

### 3. What it does

Users can identify extension/resource packages with npm specifications, Git repositories with optional refs, or local files/directories. The parser determines cache identity and pin behavior before any install or settings mutation.

### 4. How it works

A package source selects one backend and one stable identity. Local paths resolve against the applicable scope, while npm versions and Git refs affect installation without changing that identity.

parseSource checks the literal npm: prefix first. It then uses the local-path predicate before attempting parseGitUrl and otherwise returns a local source. Bare names and unprefixed host/user/repo or git@host:path shorthand therefore remain local; protocol Git URLs are accepted without git:, while the explicit git: prefix admits historical shorthand. Source comparison is npm package name, Git host plus normalized repository path, or a lexically resolved absolute local path. Git transport and ref, and npm version, do not participate in identity (ice:packages/coding-agent/src/core/package-manager.ts:1352-1432; ice:packages/coding-agent/src/core/package-manager.ts:1691-1789).

The npm parser accepts an optional scope, lower-case ASCII letter/digit initial name characters and the remaining permitted dot/underscore/dash characters; package-name length is bounded at 214. A valid exact semver is pinned; a valid semver range is a range; a dist-tag/non-range has no satisfiable range check. A version suffix alone does not make every npm source pinned. Git parsing trims, separates a ref at the first path-side @ without confusing SSH userinfo, retains slashes and further @ characters in the ref, tries hosted-git-info formats then generic parsing, removes a terminal .git, and requires a host and at least two path components. Host/path validation rejects raw or decoded NUL/backslash, absolute path prefixes, any '..' path segment and invalid percent decoding. Ref contents are passed as structured Git arguments; they are not shell-interpolated (ice:packages/coding-agent/src/utils/git.ts:1-227).

resolvePath expands only '~', '~/...' and the platform-specific home separator form, converts file URLs, and resolves relative names against the supplied base. Literal percent sequences in ordinary POSIX paths stay literal. '~draft' stays a local name. Configured local sources resolve from the user agent directory or project config directory; interactive command input initially resolves from cwd. Manifest entry paths instead use package-relative path.resolve and do not apply home expansion. Canonical realpath dedup is a later resolver operation, separate from source identity. The direct URL/path suites and broader package-manager tests were fully read, not executed.

Shared path helpers retain distinct contracts: canonicalizePath(path) returns realpathSync(path), falling back to the original string on any thrown error; isLocalPath(value) trims, then rejects the case-sensitive prefixes npm:, git:, github:, http:, https:, ssh:, while file: and bare names are local. formatPathRelativeToCwdOrAbsolute(filePath, cwd) resolves through resolvePath, chooses getCwdRelativePath when inside cwd (including '.' for the root), otherwise the absolute path, and converts platform separators to '/' (`ice:packages/coding-agent/src/utils/paths.ts:22-55`; `ice:packages/coding-agent/src/utils/paths.ts:98-101`).

### 5. Interfaces

Source strings include npm:name[@version], npm:@scope/name[@version], git:host/owner/repo[@ref], explicit git:git@host:owner/repo[@ref], protocol Git URLs, relative/absolute/home/file-URL local paths. GitSource carries repo, host, path, optional ref and pinned. Stable identity must be reused by add/remove/update and scoped dedup; it must not silently turn a bare local name into an npm or Git source.

canonicalizePath(path:string):string; isLocalPath(value:string):boolean; formatPathRelativeToCwdOrAbsolute(filePath:string, cwd:string):string.

### 6. Dependencies

This is the backend-independent parsing/identity prerequisite for the package entries below. Generic path consumers may reuse the read helper evidence without being counted as package implementations.

### 7. ice-rust today

The global native installer parses Cargo crate names plus --version/--path/--build/--github; project 'package link' parses Cargo workspace/package selection and a display name. Neither is the npm/Git/local resource-source union, semver-range/tag policy or settings-relative identity map described here (rust:crates/ice-cli/src/install.rs:1075-1300; rust:crates/ice-cli/src/packages.rs:48-136). Native GitHub repository canonicalization is a prebuilt-distribution contract, not a general Git checkout parser. Existing Rust package backends are retained under CLI-039–041.

### 8. Port notes

Add a typed npm/Git/local source layer above the existing native package facility, with explicit scope bases and stable identity. Port the exact source-disambiguation, npm pin/range distinctions, SSH/ref parsing and containment rejection cases. Keep resource-manifest paths package-relative and defer physical dedup to CLI-032. Preserve Cargo/prebuilt commands as distinct supported backends instead of treating their crate parser as npm parity. Future implementation tests should cover encoded traversal, SSH userinfo/ref boundaries, tags versus exact semver, tilde literals, file URLs and scope-relative trailing-slash equivalence.

### 9. Size and priority

Estimated Rust implementation: **220-360 lines** (estimate, not measured source). **P1** - Every package mutation and resolver depends on correct backend selection, identity and path scope; mistaken parsing can target the wrong installation.

## CLI-021 - Scoped package settings, identity-preserving edits and payload-before-config ordering

### 1. Title and status

Scoped package settings, identity-preserving edits and payload-before-config ordering. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/core/package-manager.ts:782-887; ice:packages/coding-agent/src/core/package-manager.ts:973-1041; ice:packages/coding-agent/src/core/package-manager.ts:1691-1789; ice:packages/coding-agent/test/package-command-paths.test.ts:107-132; rust:crates/ice-cli/src/packages.rs:168-232; rust:crates/ice-cli/src/install.rs:18-83.; ice:packages/coding-agent/src/core/package-manager.ts:960-987.

Named export: `ConfiguredPackage` (`ice:packages/coding-agent/src/core/package-manager.ts:97`).

Named export: `DefaultPackageManager` (`ice:packages/coding-agent/src/core/package-manager.ts:782`).

### 3. What it does

Install/remove commands persist the package source in user or project settings, retaining filter objects and using the same source identity as resolution and updates.

### 4. How it works

A package setting can be a source string or an object carrying resource filters. Edits preserve that object when its source changes and normalize local input so it continues to point to the same location from the settings file.

addSourceToSettings uses project settings only when local=true, otherwise global/user settings. Local command input resolves against cwd and is serialized relative to the scope base, with './' added when the relative form has no dot prefix and '.' for an empty relative path. It finds the first existing equal source identity; an identical stored source returns false, while a changed source replaces only that source value and preserves the rest of a filter object. With no match it appends. removeSourceFromSettings removes all equal-identity entries and reports whether anything changed. Identity ignores npm version and Git ref (ice:packages/coding-agent/src/core/package-manager.ts:800-854; ice:packages/coding-agent/src/core/package-manager.ts:1691-1789).

installAndPersist first awaits installation and then adds the source to settings. removeAndPersist first removes the managed payload and then removes matching settings entries. A settings-write failure does not roll back payload changes. Local installations only validate the cwd-resolved path and do not copy it; local removals leave source files untouched. Project mutations call the trust assertion before the progress operation. This manager does not implement a transaction spanning filesystem, process execution and SettingsManager writes (ice:packages/coding-agent/src/core/package-manager.ts:973-1041).

listConfiguredPackages reads the raw global list followed by the raw project list, returning source, scope and filtered=true for every object form. It does not apply resolver identity dedup or globalFirst suppression. getInstalledPath computes the relevant npm/Git/local path and checks existence; existence is not package validity or successful extension loading. Command-path tests verify global relative paths are stored relative to settings.json and trailing slashes still identify the same removable local package.

### 5. Interfaces

PackageSource is a string or {source, autoload?, extensions?, skills?, prompts?, themes?}; add/remove settings operations return boolean. Public manager operations include install/installAndPersist/remove/removeAndPersist, listConfiguredPackages, getInstalledPath, setProgressCallback and resolveExtensionSources. The settings arrays belong to user/project scope and must remain separate from the native artifact ownership registry.

### 6. Dependencies

`CLI-020`, `SET-003`, `SET-007`, `SET-008`, `SET-009`

CLI-020 supplies identity and local path bases. SET-003 owns project trust; SET-007/008/009 own scoped storage, precedence and durable writes. This entry owns package-specific array edits and mutation ordering.

### 7. ice-rust today

Native project packages.json records name/version/source/cargo_package/artifacts, and the global registry records installed native artifacts/provenance. They do not store TS packages arrays, filter objects, identity-preserving source updates or raw configured package listing. Native global artifact transactions are stronger within their own registry and must remain available; project package operations have separate nontransactional behavior described in CLI-039 (rust:crates/ice-cli/src/packages.rs:168-232; rust:crates/ice-cli/src/install.rs:18-83; rust:crates/ice-cli/src/install.rs:641-850).

### 8. Port notes

Implement package-specific scoped CRUD over SettingsManager equivalents, preserving filter objects and relative-path intent. Define payload/config error ordering explicitly and report partial completion honestly; either preserve the TS ordering or strengthen it with a documented recoverable transaction without claiming the TS source already has rollback. Keep artifact ownership records distinct from user package configuration. Verify add/replace/no-op/remove-all behavior across npm versions, Git refs, relative aliases and failed payload/config steps.

### 9. Size and priority

Estimated Rust implementation: **160-260 lines** (estimate, not measured source). **P1** - Package commands must persist exactly what later startup resolves; incorrect scope or loss of filters changes which code and resources load.

## CLI-022 - Managed cache layout, temporary extension caches and lexical containment

### 1. Title and status

Managed cache layout, temporary extension caches and lexical containment. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/core/package-manager.ts:205-223; ice:packages/coding-agent/src/core/package-manager.ts:2008-2136; ice:packages/coding-agent/src/core/package-manager.ts:1962-2006; rust:crates/ice-cli/src/install.rs:93-154; rust:crates/ice-cli/src/packages.rs:136-166.; ice:packages/coding-agent/src/core/package-manager.ts:2008-2136.

Named export: `getExtensionTempFolder` (`ice:packages/coding-agent/src/core/package-manager.ts:218`).

### 3. What it does

Package installations use predictable user/project roots, while temporary extension sources use persistent private cache directories. Managed target checks keep derived npm/Git paths lexically below those roots.

### 4. How it works

Installed resource packages live in dedicated caches instead of the project's ordinary dependency tree. Temporary extension sources use a separate owner-private cache that survives the current process.

User npm packages resolve under agentDir/npm/node_modules, project npm packages under getProjectConfigDir(cwd)/npm/node_modules, and Git checkouts under the respective git/<host>/<repository path>. Project write paths require trust. Temporary npm roots use agentDir/tmp/extensions/npm/<eight-hex hash>/node_modules; temporary Git roots use the corresponding git-<host> prefix/hash plus repository path. The helper uses the first eight hex characters of SHA-256(prefix + '-' + (suffix or empty)), then joins prefix/hash/suffix; npm uses prefix 'npm' and no suffix, while Git uses 'git-<host>' and repository path. Ref/version is not part of that cache identity. A temporary option takes precedence over local scope. The temporary root is created recursively and chmodded to 0700; it is not automatically removed after a run (ice:packages/coding-agent/src/core/package-manager.ts:205-223; ice:packages/coding-agent/src/core/package-manager.ts:2008-2136).

Derived managed package targets must be strict lexical descendants: resolve root/target and require the target to start with root plus the platform separator. Equality with the root is rejected. This prevents encoded/path-parser traversal from selecting the whole cache but is not a canonical symlink containment check. Local user-specified source paths are allowed outside managed caches and are not subject to this installed-target restriction.

An npm root gets a minimal {name:'ice-extensions', private:true} package.json only when absent. A managed directory gets .gitignore containing '*' and '!.gitignore' only when absent. Existing files are retained rather than rewritten. Removing Git checkouts prunes empty parent directories only below the install root and also removes the sibling incomplete-update marker (ice:packages/coding-agent/src/core/package-manager.ts:1962-2006). HOME fallback for discovery uses process.env.HOME or homedir(), not a new configuration directory.

### 5. Interfaces

Persistent managed paths: <agentDir>/npm, <projectConfigDir>/npm, corresponding git/<host>/<path>, and <agentDir>/tmp/extensions/{npm, git-host}/<hash>. Managed target derivation is separate from local source resolution. Temporary cache retention and private mode are observable filesystem behavior; there is no TTL, cleanup handle or per-run disposal API in this manager.

### 6. Dependencies

`CLI-020`, `SET-001`, `SET-003`

SET-001 supplies native/user/project configuration roots; SET-003 supplies trust gating. Source normalization is CLI-020. Native artifact roots remain distinct.

### 7. ice-rust today

Native global installation targets the agent's native extension directory and a native package registry; project links target cwd/.icers/packages/<name>/extensions. These are valid native artifacts paths but do not implement npm/Git resource caches or the temporary TS layout. The native global installer validates flat artifact names, while project name containment is weaker and is separately corrected in CLI-039 (rust:crates/ice-cli/src/install.rs:93-154; rust:crates/ice-cli/src/packages.rs:136-166). No npm/Git cache manager was found in the fully read native package modules.

### 8. Port notes

Add managed resource cache roots, exact temporary identity/private permissions and strict target derivation to the new source backends. Retain the native artifact directories/registries. Document temporary cache persistence rather than repeating the package guide's 'current run only' wording as an automatic-deletion guarantee. If adding physical containment against symlink escapes, make it a deliberate strengthening with tests for existing symlinked roots and targets. Preserve user files when initializing package.json/.gitignore.

### 9. Size and priority

Estimated Rust implementation: **150-240 lines** (estimate, not measured source). **P1** - Cache isolation and containment affect installation safety, project cleanliness and repeatable temporary extension loading.

## CLI-023 - Package subprocess environment, completion events, timeouts and progress callbacks

### 1. Title and status

Package subprocess environment, completion events, timeouts and progress callbacks. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/package-manager.ts:1-54; ice:packages/coding-agent/src/core/package-manager.ts:65-118; ice:packages/coding-agent/src/core/package-manager.ts:855-887; ice:packages/coding-agent/src/core/package-manager.ts:2603-2696; rust:crates/ice-cli/src/packages.rs:109-135; rust:crates/ice-cli/src/install.rs:223-398; rust:crates/ice-cli/src/updates.rs:1785-1858.; ice:packages/coding-agent/src/core/package-manager.ts:1630-1638.

Named export: `ProgressCallback` (`ice:packages/coding-agent/src/core/package-manager.ts:88`).

### 3. What it does

Package operations execute external package managers and Git with structured arguments and operation-specific stream/timeout behavior. Progress is an operation lifecycle callback, not a cancellable task protocol.

### 4. How it works

An operation reports start, completion or failure around its actual work. Metadata probes capture output and use timeouts, while installs normally inherit the terminal and can run without a package-manager deadline.

withProgress invokes a start event before its try block, awaits the operation, then emits complete inside the try. An operation or completion-callback failure emits error with a string message and rethrows; a start-callback throw escapes before that error wrapper. Events carry type start/progress/complete/error, action install/remove/update/clone/pull, source and optional message, although the manager's wrapper emits the lifecycle subset. There is one mutable progress callback and no AbortSignal/cancel handle in the PackageManager API (ice:packages/coding-agent/src/core/package-manager.ts:65-118; ice:packages/coding-agent/src/core/package-manager.ts:855-887).

getEnv returns process.env except on Linux when it is entirely empty. Only that case reads /proc/self/environ, splits NUL records, splits each record at its first '=', and falls back on read failure. It does not fill missing variables in a nonempty environment. Captured probes spawn with null stdin and piped stdout/stderr, merge supplied environment overrides over this map and accumulate unbounded strings. Completion waits for close, not exit, so buffered output after child exit is included. An optional timeout kills with SIGKILL and rejects immediately without proving the process has exited; child error/close clear the timer. Nonzero/signal errors use stderr then stdout for detail (ice:packages/coding-agent/src/core/package-manager.ts:7-24; ice:packages/coding-agent/src/core/package-manager.ts:2603-2696).

Ordinary asynchronous install commands inherit stdio unless stdout has been taken over, in which case stdin is ignored and output is routed to fd 2. They resolve/reject on exit, have no capture timeout and use structured executable/argument arrays rather than shell concatenation. Synchronous metadata helpers use piped output and return trimmed stdout or stderr, with their own command-specific cwd behavior. Package metadata probes use 10000 ms; remote Git checks additionally set GIT_TERMINAL_PROMPT=0, while clone/fetch paths do not universally set it. Source tests explicitly assert delayed close output and rejection when a mocked child ignores kill; this audit did not run them.

### 5. Interfaces

ProgressEvent fields and action/type enums above; setProgressCallback replaces one callback. Process inputs are command plus args, optional cwd/env/timeout. /proc/self/environ is an empty-Linux-environment input, not a managed resource. ICE_OFFLINE uses CLI-018's exact-name environment helper and the package parser's '1'/'true'/'yes' truth semantics; it is not cancellation of already started operations.

### 6. Dependencies

`CLI-018`

CLI-018 owns the exact-name environment compatibility helper. Installation and metadata callers below choose timeout, stdio and concurrency; they are consumers, not prerequisites.

### 7. ice-rust today

Native Cargo builds already use structured Command arguments and inherited terminal streams, while cargo metadata captures output. Native self-update executable probes have a real timeout but cap returned bytes only after wait_with_output, a separate defect in CLI-044. There is no common TS PackageManager progress callback or matching close-versus-exit contract across these native commands (rust:crates/ice-cli/src/packages.rs:109-135; rust:crates/ice-cli/src/install.rs:223-398; rust:crates/ice-cli/src/updates.rs:1785-1858). The Linux JS-empty-environment workaround is runtime-specific and should not be copied mechanically.

### 8. Port notes

Provide a package process abstraction with explicit capture/inherit modes, operation progress and command-specific timeouts. Preserve argument boundaries and terminal routing. Specify cancellation as a new bounded native capability if introduced; do not describe SIGKILL delivery or Promise rejection as process-tree completion. Prefer streaming output bounds for native probes, recording this as a strengthening of the TS implementation. Future verification should cover process exit before pipe closure, timeout/kill failure, callback throws, inherited output routing and offline decisions before spawning.

### 9. Size and priority

Estimated Rust implementation: **160-280 lines** (estimate, not measured source). **P1** - Reliable process completion and error reporting are prerequisites for correct install/update state and avoid hanging metadata discovery.

## CLI-024 - npm, pnpm and Bun package installation with legacy global-cache fallback

### 1. Title and status

npm, pnpm and Bun package installation with legacy global-cache fallback. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/core/package-manager.ts:1433-1690; ice:packages/coding-agent/src/core/package-manager.ts:1810-1825; ice:packages/coding-agent/src/core/package-manager.ts:1981-2135; ice:packages/coding-agent/test/package-manager.test.ts:1-2658; rust:crates/ice-cli/src/install.rs:155-398.; ice:packages/coding-agent/src/core/package-manager.ts:1730-1812; ice:packages/coding-agent/src/core/package-manager.ts:2019-2077.

### 3. What it does

The package manager honors a configurable executable/argument prefix and selects npm-, pnpm- or Bun-specific install/remove flags, while retaining read compatibility with older globally installed npm resources.

### 4. How it works

Package-manager commands retain the user's configured wrapper arguments. New installs and updates go into managed scope roots even when older global installations remain available for resource lookup.

npmCommand defaults to npm when absent or an empty array; a present falsy first element is an error. The executable and all prefix arguments are preserved. Manager detection examines the command after the last '--' when wrapping, uses the basename, strips .cmd/.exe case-insensitively, and then compares the resulting manager name to bun/pnpm. Managed npm installs use 'install <specs> --prefix <root> --legacy-peer-deps'. pnpm uses install with --prefix plus config.auto-install-peers=false, config.strict-peer-dependencies=false and config.strict-dep-builds=false. Bun uses install with --cwd and --omit=peer. An unversioned update targets @latest; explicitly configured suffixes remain in the specification (ice:packages/coding-agent/src/core/package-manager.ts:1730-1812).

Removal uses Bun 'uninstall <name> --cwd <root>' or 'uninstall <name> --prefix <root>' for other managers, adding legacy-peer-deps except for pnpm. Git dependency installation is a different path: the default npm invocation uses install --omit=dev, whereas a nonempty configured npmCommand runs its plain install command (an empty array still uses the default omit-dev branch). Batch updates can pass multiple npm package specifications in one scoped install. The manager initializes its root manifest and ignore file through CLI-022.

For user-scope reads, a missing managed package can fall back to the configured command's global installation. Global root lookup is cached by the exact command plus argument prefix (NUL-separated), so changing npmCommand invalidates the key. Bun derives its global node_modules root from 'pm bin -g'; other managers use 'root -g'. pnpm additionally reads 'list -g --depth 0 --json' and dependencies[name].path, with malformed output handled by the enclosing fallback. Project lookups do not inherit this global fallback. Explicit installs/updates always target managed roots, and removals only remove managed payloads, so a legacy global installation can remain visible after a managed removal (ice:packages/coding-agent/src/core/package-manager.ts:2019-2077).

Installed-version lookup parses package.json but returns raw pkg.version without an explicit type or semver validation. A falsy value does not satisfy an installation; with a parsed range it calls semver.satisfies, while without a range any truthy value is accepted. Thus dist-tag/un-ranged reuse is weaker than proof of a valid/current published version, and a mismatched exact/range constraint triggers reinstallation during resolve. This is separate from latest-version checking. Full tests cover wrapper arguments, manager-specific flags, managed migration, changing command cache keys and malformed pnpm global output; no npm, pnpm or Bun process was run in this audit.

### 5. Interfaces

Settings npmCommand is a string array, including wrapper args. User/project/temporary install roots come from CLI-022. npm source strings and parsed exact/range/tag metadata come from CLI-020. Managed package.json version reads and legacy global paths are lookup inputs, not authority to modify the user's unrelated global installation.

### 6. Dependencies

`CLI-020`, `CLI-022`, `CLI-023`, `SET-002`

SET-002 owns stored npmCommand/install-method selection shared with self-update. This entry owns per-package backend flags and cache fallback, not self-update command planning.

### 7. ice-rust today

The native installer uses Cargo or verified prebuilt libraries and supports its own --build/--path/--github grammar. It does not invoke npm/pnpm/Bun, derive global Node roots, resolve npm semver ranges, or manage resource packages in node_modules (rust:crates/ice-cli/src/install.rs:155-398). Native artifact transactions and verified prebuilt distribution remain useful and must be preserved under CLI-040/041.

### 8. Port notes

Add npm-family backends behind the typed package source layer and preserve configured executable/argument arrays exactly. Implement manager-specific install/remove flags, command-keyed global lookup and the distinction between read fallback and managed mutation. Keep Cargo/prebuilt support intact. Future tests should verify wrapper delimiters, Windows executable suffixes, scoped batch args, changed global roots, malformed pnpm JSON, exact/range/tag satisfaction and removal revealing a remaining legacy installation.

### 9. Size and priority

Estimated Rust implementation: **240-390 lines** (estimate, not measured source). **P1** - Most TS resource packages use npm-compatible distribution; missing command and compatibility semantics block practical package migration.

## CLI-025 - Git installation, clone failure cleanup and persistent temporary checkout refresh

### 1. Title and status

Git installation, clone failure cleanup and persistent temporary checkout refresh. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/core/package-manager.ts:1200-1320; ice:packages/coding-agent/src/core/package-manager.ts:1814-1863; ice:packages/coding-agent/src/core/package-manager.ts:1962-1980; ice:packages/coding-agent/test/git-update.test.ts:1-487; rust:crates/ice-cli/src/packages.rs:48-190.; ice:packages/coding-agent/src/core/package-manager.ts:1943-1962.

### 3. What it does

Git sources install complete checkouts with optional ref selection and package dependencies. Temporary unpinned sources refresh their persistent cache opportunistically without making a failed refresh fatal to the current run.

### 4. How it works

A new Git source is cloned before it can contribute resources. Temporary cached sources may be refreshed, but a failed optional refresh leaves the previously available checkout usable.

A missing repository creates the managed parent and runs 'git clone <repo> <target>'; if the source carries a ref it checks out that ref. A package.json triggers dependency installation using the npmCommand policy in CLI-024. Clone, checkout or initial dependency failure removes the target and prunes empty parent directories below the install root, then propagates the error. There is no shallow/sparse clone or downloaded archive substitution. Progress action is clone for initial acquisition, with source preserved (ice:packages/coding-agent/src/core/package-manager.ts:1814-1863; ice:packages/coding-agent/src/core/package-manager.ts:1962-1980).

An ordinary existing Git source is returned during resource resolution without automatically reconciling a changed pin. Explicit install/update paths can reconcile that checkout through CLI-026. An existing temporary source with no pinned ref is refreshed online via the pull/reconcile path; the enclosing refresh catches failures and keeps the cached clone. A temporary pinned checkout is not reconciled automatically just because its configured ref changes. Offline mode suppresses optional refresh and missing acquisition. The 'temporary' API therefore means non-persisted configuration plus a reusable cache, not deletion on process exit (ice:packages/coding-agent/src/core/package-manager.ts:1200-1320).

Git removal deletes the checkout and its sibling .<repository>.ice-update-incomplete marker, then walks upward removing empty directories until the install root boundary. It does not remove a local source tree. Empty-parent cleanup is best-effort and must not be generalized to deleting nonempty scope roots. Full source tests cover clone failure, dependency-install failure, temporary unpinned refresh, pinned cache reuse and isolated global/project targets.

### 5. Interfaces

GitSource repo/ref/pinned plus scope/temporary options; progress clone/pull; persistent checkout path and optional sibling incomplete marker. Command execution is inherited/structured and lacks a public cancellation signal. MissingSourceAction is integrated in CLI-033.

### 6. Dependencies

`CLI-020`, `CLI-022`, `CLI-023`, `CLI-024`

Parsing, managed containment, process execution and dependency installation are prerequisites. CLI-026 extends an already installed checkout with explicit reconciliation and retry markers; it is not needed to define initial clone/cleanup.

### 7. ice-rust today

Project native package link builds from an existing Cargo path and copies its dynamic library. The global native installer resolves crates/prebuilt releases. Neither module creates and refreshes general Git resource checkouts or implements the temporary cache policy (rust:crates/ice-cli/src/packages.rs:48-190; rust:crates/ice-cli/src/install.rs:155-398). GitHub release download support is a different verified binary distribution mechanism, retained separately.

### 8. Port notes

Implement Git checkout acquisition and cleanup under managed roots, using exact argument vectors and the scoped npm dependency policy. Separate required acquisition failures from optional temporary-refresh failures and expose that distinction in diagnostics. Preserve the cached-pinned behavior unless deliberately changing it with an explicit compatibility note. Future tests should verify cleanup after each initial failure stage, no local-source deletion, pinned versus unpinned temporary reuse, offline cache hits and scope-safe parent pruning.

### 9. Size and priority

Estimated Rust implementation: **180-300 lines** (estimate, not measured source). **P1** - Git extension packages and temporary source loading are unavailable without checkout acquisition and clear failure/cache behavior.

## CLI-026 - Git update reconciliation, forced remote refs and incomplete-update recovery

### 1. Title and status

Git update reconciliation, forced remote refs and incomplete-update recovery. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/core/package-manager.ts:1814-1962; ice:packages/coding-agent/src/core/package-manager.ts:1550-1638; ice:packages/coding-agent/test/git-update.test.ts:1-487; rust:crates/ice-cli/src/packages.rs:233-386.

### 3. What it does

Updating an installed Git resource package moves the checkout to the selected remote commit, including rewritten remote history, and records when destructive reconciliation or dependency installation did not finish.

### 4. How it works

Git updates compare commit identities before changing files. When a checkout must move, an on-disk marker survives failures so a later retry can finish cleaning and reinstalling dependencies.

Pinned sources fetch origin/ref and reconcile to FETCH_HEAD. Unpinned sources prefer a local upstream whose remote is origin and otherwise attempt 'remote set-head origin -a' before using origin/HEAD. Branch fetches use --prune --no-tags and a forced refspec '+refs/heads/<branch>:refs/remotes/origin/<branch>'; a HEAD fallback covers the corresponding default-head case. This accepts force-pushed remote history instead of requiring a fast-forward. After fetch, HEAD and the target's commit form are read with 10000 ms metadata probes (ice:packages/coding-agent/src/core/package-manager.ts:1814-1962; ice:packages/coding-agent/src/core/package-manager.ts:1550-1638).

If the commits match and no incomplete marker exists, the manager only repairs missing dependencies. It does not reset or clean an otherwise current checkout. If a marker exists, it performs the clean/install completion path even at the same commit. When commits differ, it writes the sibling .<repo>.ice-update-incomplete marker before reset --hard and clean -fdx, installs dependencies when package.json exists, then removes the marker only after success. A reset or dependency-install failure leaves the marker. If clean fails, the code attempts missing-dependency repair, ignores that secondary repair error, and rethrows the original clean failure. This is retry metadata, not rollback to the previous commit.

Missing-dependency detection parses package.json, looks only at the dependencies object, resolves each dependency path under node_modules with a strict descendant check, and triggers install when any dependency is missing. It does not validate every installed dependency version, dev dependency, peer dependency or lockfile consistency. Malformed manifest handling does not convert into an unconditional full install. These rules preserve an already-current checkout while allowing recovery after an interrupted update.

The git-update suite uses local repository fixtures for normal updates, detached-head fallback, rewritten history and annotated tags. Its introductory comment claiming force-push failure is stale relative to the current forced refspec/reset implementation and assertions. No Git command from those tests was executed by this audit.

### 5. Interfaces

Explicit install/update of a Git source, selected target commit, 10000 ms rev-parse probes, 'pull' progress and the sibling incomplete marker. Reconciliation can remove untracked/ignored checkout files through clean -fdx; callers must not apply it to arbitrary local source directories. There is no rollback snapshot or public abort signal in the TS manager.

### 6. Dependencies

`CLI-023`, `CLI-024`, `CLI-025`

CLI-025 establishes owned Git checkout roots. CLI-024 supplies dependency installation; CLI-023 supplies structured process execution and errors.

### 7. ice-rust today

Native 'package update' rebuilds linked Cargo projects and invokes the native installer for eligible global records, continuing after individual failures. It does not fetch Git branches, reset resource checkouts, track incomplete reconciliation markers or repair Node dependencies (rust:crates/ice-cli/src/packages.rs:233-386). These native update routes are retained under CLI-039/041 rather than counted as Git parity.

### 8. Port notes

Add explicit Git reconciliation with forced remote-ref updates, commit-equality fast path and a durable incomplete marker. Preserve ownership/containment checks before destructive reset/clean, and ensure every failed stage leaves retryable state. Do not characterize the marker as a transaction or claim dependency repair validates the full installation. Future verification should cover force-push/total rewrite, pinned refs, equal-commit marker recovery, clean failure with missing dependencies, dependency install failure and marker cleanup on removal.

### 9. Size and priority

Estimated Rust implementation: **250-420 lines** (estimate, not measured source). **P1** - Without exact reconciliation and recovery rules, package updates can silently remain stale or leave unusable checkouts after failure.

## CLI-027 - Explicit package-update selection, pin policy and bounded batch concurrency

### 1. Title and status

Explicit package-update selection, pin policy and bounded batch concurrency. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/core/package-manager.ts:1042-1200; ice:packages/coding-agent/src/core/package-manager.ts:1456-1513; ice:packages/coding-agent/test/package-manager.test.ts:1-2658; rust:crates/ice-cli/src/packages.rs:233-386.

### 3. What it does

An explicit update selects configured sources across scopes, skips exact npm pins, reconciles Git pins, groups npm installs by scope and reports errors without promising cancellation of concurrent work.

### 4. How it works

Updating packages uses the raw configured scope lists. Exact npm pins are skipped, while Git refs still participate in explicit reconciliation.

update reads global packages followed by project packages. An optional source is normalized by the same identity rules and filters all matching entries across both scopes; it does not apply resolver dedup/globalFirst suppression first. No matching package throws with a suggestion when the user appears to have omitted the configured npm: prefix. Without a source, all configured entries are considered. Local sources require no update, offline mode returns before remote/update work, and exact-semver npm pins are skipped. Git sources are retained even when pinned so an explicit update can reconcile their configured ref (ice:packages/coding-agent/src/core/package-manager.ts:1042-1200).

For npm candidates, shouldUpdateNpm reads the managed target rather than relying on the legacy global read fallback. A missing managed installation needs an update. Otherwise it compares the installed version with the queried target version by inequality, not semver 'newer than'. Query failure returns true so the actual install is attempted. Eligible npm specifications are grouped into one user install and one project install; the two scope operations run concurrently with a Git worker queue of concurrency four. Metadata checks also use concurrency four. Duplicate raw configuration entries can be considered; this route is not the effective-resource dedup pipeline.

Promise.all propagates a rejection but does not cancel already running sibling operations. One scope can succeed while another fails, and a CLI caller must not report a fully rolled-back update. Per-operation progress wraps clone/pull/install/update work using CLI-023; grouped npm calls keep the configured specification, adding @latest only for unversioned packages. The manager does not expose a global cancellation/transaction result.

Read tests cover scope batching, exact pin skips, range updates, managed migration, bounded metadata/Git workers, explicit Git pin reconciliation and missing-prefix suggestions. The native package-update loop's continue-on-error behavior is a separate retained contract, not evidence that this TS batch algorithm exists.

### 5. Interfaces

update(source?) with no additional scope parameter; configured source identity comes from CLI-020. Concurrency constants are four for metadata and Git work, plus at most one grouped npm install per scope. Errors are thrown after one Promise rejects and may coexist with successful sibling mutations.

### 6. Dependencies

`CLI-020`, `CLI-021`, `CLI-024`, `CLI-026`, `CLI-028`

CLI-028 provides target-version/head query mechanisms. Other dependencies provide configured selection, npm commands and Git reconciliation. The CLI target router is a consumer.

### 7. ice-rust today

Native packages::run_update enumerates linked project records and eligible global native records, rebuilds/reinstalls sequentially while accumulating failure status, and does not use npm pin/range rules, settings source identity or four-worker metadata/Git queues (rust:crates/ice-cli/src/packages.rs:233-386). Native behavior must be retained and explicitly distinguished when adding a resource-package update facade.

### 8. Port notes

Implement update selection and batching above the new source backends, with explicit raw-scope versus effective-resource semantics. Retain exact npm pin skips and explicit Git pin reconciliation. Provide per-source outcomes if strengthening the API, but do not claim Promise rejection cancels peers. Future tests should cover duplicate identities across scopes, pinned npm versus pinned Git, a failed metadata lookup followed by install, one failed scope alongside a successful scope, four-worker bounds and offline no-spawn behavior.

### 9. Size and priority

Estimated Rust implementation: **160-270 lines** (estimate, not measured source). **P1** - Correct update selection prevents accidental version changes and misleading success after partially completed concurrent operations.

## CLI-028 - Available package-update probes, remote-head comparison and failure suppression

### 1. Title and status

Available package-update probes, remote-head comparison and failure suppression. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/core/package-manager.ts:45-64; ice:packages/coding-agent/src/core/package-manager.ts:1169-1232; ice:packages/coding-agent/src/core/package-manager.ts:1456-1662; ice:packages/coding-agent/test/package-manager.test.ts:1-2658; rust:crates/ice-cli/src/updates.rs:146-521.

### 3. What it does

Update notices query installed, unpinned configured packages without changing them. Discovery failures suppress notices, while explicit updates use a different failure policy.

### 4. How it works

An available-update check is read-only with respect to installed packages. It compares remote and local identities and omits results when metadata cannot be determined.

checkForAvailableUpdates deduplicates configured packages with project precedence, skips local sources and all pinned sources (both exact npm versions and Git refs), and only probes installed packages. Work runs with concurrency four while returned results preserve candidate order. ICE_OFFLINE accepts exact '1' or case-insensitive 'true'/'yes' without trimming and returns no remote results. The output records source, displayName, npm/git type and user/project scope (ice:packages/coding-agent/src/core/package-manager.ts:45-64; ice:packages/coding-agent/src/core/package-manager.ts:1169-1232).

npm checks run '<configured command/prefix> view <source spec or unversioned name> version --json' with cwd and the 10000 ms timeout. Empty output errors. A JSON string is returned without extra version validation; a JSON array is filtered to nonempty strings, then selects semver.maxSatisfying for a parsed range or reverse-semver-sorted first element otherwise. Unsupported/no-result JSON errors. Installed package.json lookup also returns raw pkg.version, and the notice path first rejects a falsy installed value, then compares target with installed by inequality. A different version is considered an available update; this is not an ancestry or strictly increasing-semver proof. Errors produce no notice. Explicit shouldUpdateNpm deliberately behaves differently: missing metadata causes an install attempt. Git checks compare local HEAD with a remote branch/head SHA, first considering an origin upstream and then the remote HEAD. For an upstream branch the first output line matching 40 lower-case hexadecimal characters followed by whitespace supplies the SHA; it does not independently recheck the returned ref label. The fallback requires that SHA followed by the exact HEAD label. Comparison is inequality, not reachability or fast-forward ancestry. Remote checks set GIT_TERMINAL_PROMPT=0 and use 10000 ms captured-process timeouts (ice:packages/coding-agent/src/core/package-manager.ts:1456-1638).

Pinned Git packages can be explicitly updated through CLI-027/026 but are absent from notification checks. A missing installed package is not itself an update notice. This path has no durable metadata cache/freshness timestamp or network retry loop in the package manager; error suppression and concurrency are the primary controls. Captured stdout/stderr are not streaming-bounded in the TS helper, as documented in CLI-023.

Full tests cover offline short-circuiting, pinned filtering, current-versus-remote differences, metadata failure suppression and worker limits. Native startup notices have substantial caching and different backend semantics; their preservation is separately specified in CLI-042.

### 5. Interfaces

PackageUpdate[] with source, displayName, type:'npm'|'git', scope:'user'|'project'; remote metadata timeout 10000 ms and concurrency four. No package mutation or settings write occurs in this check. Environment ICE_OFFLINE/GIT_TERMINAL_PROMPT affects whether/how probes run.

### 6. Dependencies

`CLI-020`, `CLI-022`, `CLI-023`

Source identities/pins, managed installed paths and captured subprocess probes are prerequisites. Native durable update cache is CLI-042, not a substitute for this npm/Git notice contract.

### 7. ice-rust today

Native updates.rs checks Cargo/prebuilt extension versions and the CLI, supports six-hour transient-failure cache fallback and generates native command notices. It does not inspect npm resource installations or general Git repository HEADs (rust:crates/ice-cli/src/updates.rs:146-521). Its bounded parallel request/freshness mechanisms should be retained while resource-package checks are added, with explicit backend identities.

### 8. Port notes

Add read-only npm/Git update discovery with the exact pin and missing-install filters, ordering and metadata failure policy. Integrate results with native notices through backend-tagged records rather than overwriting the native cache schema. Preserve or deliberately strengthen probe output bounds. Future tests should distinguish explicit-update fallback from notice suppression, local/remote commit inequality from ancestry, pinned Git reconciliation from pinned notice exclusion, and result ordering under out-of-order completion.

### 9. Size and priority

Estimated Rust implementation: **160-260 lines** (estimate, not measured source). **P2** - Update notices improve maintainability, but package installation and explicit update correctness are the higher-priority prerequisites.

## CLI-029 - Resource manifests, convention fallback and glob expansion by resource type

### 1. Title and status

Resource manifests, convention fallback and glob expansion by resource type. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/core/package-manager.ts:423-629; ice:packages/coding-agent/src/core/package-manager.ts:2138-2601; ice:packages/coding-agent/src/core/ice-manifest.ts:1-62; ice:packages/coding-agent/test/suite/regressions/7187-malformed-package-manifest.test.ts:1-48; ice:packages/coding-agent/test/legacy-compat/runtime-contracts.test.ts:59-88; rust:crates/ice-cli/src/packages.rs:606-626.

### 3. What it does

An installed resource package can declare extensions, skills, prompts and themes in package.json, or use conventional directories. Manifest presence and user filters change fallback behavior in ways that must not be flattened into one directory scan.

### 4. How it works

A resource package chooses explicit manifest entries when available and otherwise uses resource-specific conventions. The presence of a filter object changes which fallback branches run, so an empty manifest and an omitted field are meaningful inputs.

The compatible manifest reader selects an own ice field wholesale before legacy pi; malformed ice does not downgrade to a valid pi field. The resource arrays are sanitized by the reader, and arbitrary malformed values must not enter file expansion. This selection contract is EXT-005; this entry owns its package-resolution consequences (ice:packages/coding-agent/src/core/ice-manifest.ts:1-62).

Without a filter object, any valid manifest, including an empty object, makes collectPackageResources treat the directory as a package and load only declared resource kinds. Omitted fields do not automatically scan conventional directories. With no valid manifest it tries extensions/, skills/, prompts/ and themes/ conventions; an existing conventional directory can establish package status even if it yields no files. With a user filter object the function returns true even when no resources are selected. For an omitted kind in that filter object, collectDefaultResources honors an existing manifest array, including an empty array, then returns; only a missing array permits convention fallback (ice:packages/coding-agent/src/core/package-manager.ts:2138-2601).

For a filtered kind, collectManifestFiles has a separate branch: nonempty manifest entries are expanded, collected by resource type and passed through manifest include/exclude rules; empty entries fall back to the conventional directory. Positive glob expansion uses globSync with absolute results, dot=false and nodir=false, then resource-specific traversal. Existing explicit files can be accepted without rechecking the conventional suffix. Manifest paths resolve relative to the package root and preserve a literal tilde. Manifest exclusions remove files before user filtering, so a user filter cannot re-enable a file excluded from that candidate set.

Extension directory entrypoints use existing manifest extension entries first; if none exist, index.ts precedes index.js, then a one-level extension discovery fallback considers direct .ts/.js files and subdirectories with a manifest/index entrypoint. This prevents every helper module inside an extension directory from becoming a separately loaded extension. A valid manifest entry is tested for existence; the helper does not impose a universal .ts/.js suffix on explicitly declared entries. Skills/prompts/themes use their own traversal rules in CLI-030. Full tests cover malformed manifest fields, ICE-over-Pi selection without merging, literal tilde paths, multi-file extension packages and the filter fallback asymmetries.

### 5. Interfaces

package.json ice/pi objects with extensions/skills/prompts/themes arrays; paths, globs and +/-/! entries; conventional resource directories; package status boolean and four resource result arrays. Runtime import aliases and execution are downstream EXT responsibilities, not package discovery.

### 6. Dependencies

`CLI-030`, `CLI-031`, `EXT-005`

EXT-005 owns compatible manifest selection/entrypoint bridge. CLI-030 supplies file discovery and CLI-031 supplies pattern selection. Resource loader attribution/catalog publication remain EXT-031/SET-033 consumers.

### 7. ice-rust today

Native project_package_dirs supplies per-kind directories from a native registry and native extensions are dynamic libraries. The fully read packages/install/prebuilt modules do not parse TS package.json ice/pi manifests, expand resource globs or reproduce the filter-dependent convention branches (rust:crates/ice-cli/src/packages.rs:606-626). Native library discovery elsewhere is credited to EXT; directory availability alone is not resource-package manifest parity.

### 8. Port notes

Implement a manifest-to-resource expansion layer with separate no-filter, omitted-filter-kind and filtered-kind branches. Reuse the compatible-manifest reader contract, preserve explicit path handling and resource-specific collectors, and make manifest exclusions an upper bound on later user selection. Do not collapse empty arrays and missing fields. Future tests should enumerate empty/malformed/partial manifests with each user-filter form, convention directories, explicit suffix exceptions, glob directories and helper-module entrypoint suppression.

### 9. Size and priority

Estimated Rust implementation: **230-380 lines** (estimate, not measured source). **P1** - Incorrect manifest fallback can execute unintended extension files or silently omit package resources.

## CLI-030 - Resource directory walks, ignore files, symlinks and entrypoint boundaries

### 1. Title and status

Resource directory walks, ignore files, symlinks and entrypoint boundaries. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/core/package-manager.ts:225-629; ice:packages/coding-agent/src/utils/readdir-order.ts:1-21; ice:packages/coding-agent/test/readdir-order.test.ts:1-33; ice:packages/coding-agent/test/package-manager.test.ts:1-2658; rust:crates/ice-cli/src/packages.rs:606-626.

### 3. What it does

Resource discovery has distinct traversal rules for skills, extensions, prompts and themes, with nested ignore handling and symlink support. Those differences determine which files actually become loadable resources.

### 4. How it works

Each resource type has a deliberate scan depth and stopping rule. Ignore files apply from the scan root downward, and symlinks are followed only when their targets can be read.

Walks skip dot entries and node_modules by default and load .gitignore, .ignore and .fdignore files from directories visited inside the scan root. Ignore text is prefixed to the relative nested directory: blank/comment lines are skipped, escaped '#' is not a comment, leading '/' is stripped, negation '!' is retained, and escaped '!' stays literal. One matcher is shared as traversal progresses. Ignore files above the scan root are not imported, so a repository's parent .gitignore cannot accidentally suppress the entire explicitly scanned .ice resource tree (ice:packages/coding-agent/src/core/package-manager.ts:225-422).

Generic recursive file traversal follows symlink targets through statSync and ignores broken links or read/stat failures. The source has no visited-directory cycle set; final realpath dedup in CLI-032 removes duplicate results but is not a recursion-cycle guard. Enumeration calls inNodeReaddirOrder: Bun results are sorted by UTF-8 byte order to match Node's expected directory ordering, while Node results are retained. This is byte comparison, not locale-aware collation (ice:packages/coding-agent/src/utils/readdir-order.ts:1-21).

Skill discovery first checks SKILL.md. When a readable, unignored skill file is found, the directory contributes that skill and recursion stops there. ICE-style roots can accept other .md files, while .agents/skills mode requires the nested SKILL.md convention. Extension auto-discovery considers direct .ts/.js files and one level of child directories whose manifest/index entrypoint resolves; it does not recurse through every helper file. Auto-discovered prompt/theme directories inspect direct .md/.json files only. Package-declared prompt/theme directories use the generic recursive collector, so package resource traversal is deeper than top-level auto-discovery (ice:packages/coding-agent/src/core/package-manager.ts:423-629).

Existing explicit file paths are handled independently of convention suffix checks, and collection failures are generally ignored so other resources survive. The tests establish root-relative ignore behavior, symlink canonical dedup downstream, skill stopping, entrypoint selection and byte-order expectations; they do not establish safety against every symlink cycle or external target.

### 5. Interfaces

Scan roots, resource kind and skill discovery mode; nested ignore file names; enabled candidate file paths. The low-level walk may follow links outside the lexical root, while managed installation target containment is a different contract. No recursive watcher, directory mutation or cancellation API exists in these collectors.

### 6. Dependencies

`EXT-005`

Extension directory manifest/index lookup uses EXT-005. Resource selection patterns and final scope/realpath dedup are separate CLI-031/032 consumers.

### 7. ice-rust today

Native project_package_dirs only constructs resource directory paths from native package records. The reviewed native package management modules have no TS ignore-file walker, skill stopping rule, one-level JS entrypoint discovery or different package-versus-auto prompt/theme scan depth (rust:crates/ice-cli/src/packages.rs:606-626). Existing native extension/resource loaders are separately credited; this entry does not infer their absence from a filename search alone.

### 8. Port notes

Implement explicit per-kind collectors and shared ignore normalization, retaining deterministic enumeration and tolerant per-entry errors. Keep final resource dedup separate from traversal safety. A native port should add a bounded/canonical visited-directory guard if following directory symlinks, documenting that as a strengthening rather than claiming the TS walker already has it. Future tests should cover nested negated ignore rules, escaped markers, scan-root boundaries, broken/cyclic links, SKILL.md recursion stops and auto-versus-package depth.

### 9. Size and priority

Estimated Rust implementation: **250-410 lines** (estimate, not measured source). **P1** - Discovery precision prevents helper modules, ignored files or duplicate skills from becoming unintended runtime resources.

## CLI-031 - Resource filter precedence and sequential autoload-false project overrides

### 1. Title and status

Resource filter precedence and sequential autoload-false project overrides. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/core/package-manager.ts:631-780; ice:packages/coding-agent/src/core/package-manager.ts:2138-2601; ice:packages/coding-agent/test/package-command-paths.test.ts:430-459; rust:crates/ice-cli/src/packages.rs:606-626.

### 3. What it does

Resource settings support include globs, negative globs and exact enable/disable overrides. autoload:false packages use a different ordered matching rule to represent narrow project changes to inherited resources.

### 4. How it works

Ordinary resource filters combine categories in a fixed precedence order. autoload:false instead evaluates matching entries sequentially so a project override can touch only selected inherited resources.

A pattern matches normalized package-relative path, basename or absolute path. For SKILL.md, glob matching also considers the skill's parent relative path, parent basename and absolute parent path. Exact matching strips one leading './' or platform-equivalent form and compares relative/absolute file paths; SKILL.md additionally matches relative/absolute parent paths, without the same generic parent-basename shortcut. isPattern recognizes a leading !, + or -, or a '*'/'?' anywhere. Bracket or brace syntax alone does not make a string a glob in this classifier (ice:packages/coding-agent/src/core/package-manager.ts:631-780).

Normal filtering starts from the union of positive include patterns, or all candidates if there are no positive includes. It applies ! glob exclusions, then + exact reenables, then - exact disables, irrespective of their order in the input list. Automatic-resource overrides similarly start enabled and apply !, then +, then - precedence. An empty filter array disables every resource of that kind; an omitted kind follows its default manifest/convention branch. Final minus overrides win over plus even if listed earlier.

For autoload:false, only paths matched by at least one filter are returned. Patterns are processed in input order; positive/plus matches enable and negative/minus matches disable, with the last matching pattern winning. The resolver can pair this project delta with the matching user installation, retaining the user's base resources for untouched paths. Thus it is not equivalent to ordinary include/exclude filtering with an empty starting set. Manifest exclusions are applied first and cannot be widened by a later user + override because the excluded file is no longer in the candidate set (ice:packages/coding-agent/src/core/package-manager.ts:2138-2601).

The ConfigSelector test constructs a user npm:ice-tools package and cycles a project resource through autoload:false with -extensions/bar.ts, then +extensions/bar.ts, then removal of the project package override. TUI-069 owns that mutation UI; this entry owns the meaning of those arrays when resolved. Full manager tests also cover empty arrays, positive-only autoload:false without a base and manifest exclusion boundaries.

### 5. Interfaces

Package source object fields autoload and per-kind string arrays; markers ! (glob exclusion), + (exact enable), - (exact disable), unmarked includes/globs. ResolvedResource retains enabled=false rows for applicable overrides, while manifest-excluded/unmatched delta resources can be absent entirely. These cases must not be collapsed.

### 6. Dependencies

This pure selection grammar is consumed by manifest expansion, automatic resources and scoped resolution. TUI-069 is a caller, not a prerequisite. Raw settings storage remains SET-owned.

### 7. ice-rust today

The native package registry has no per-resource include/exclude arrays or autoload:false inherited-package delta representation. Native project_package_dirs emits directories only (rust:crates/ice-cli/src/packages.rs:606-626). A generic enabled native extension setting does not implement path-level pattern precedence, SKILL.md parent aliases or last-match delta semantics.

### 8. Port notes

Implement separate normal-filter, automatic-override and autoload:false evaluators with explicit path aliases and deterministic rule order. Preserve disabled result rows where the UI needs them, and never reintroduce manifest-excluded files. Integrate project deltas over the same installed user package in CLI-032. Future tests should contrast input-order independence in normal filters with last-match behavior in autoload:false, exact versus glob SKILL.md parents, empty versus omitted arrays and minus-over-plus precedence.

### 9. Size and priority

Estimated Rust implementation: **180-300 lines** (estimate, not measured source). **P1** - Filter mistakes can enable resources a user explicitly disabled or erase inherited project override intent.

## CLI-032 - Effective resource resolution, global-first suppression, scope ranks and physical dedup

### 1. Title and status

Effective resource resolution, global-first suppression, scope ranks and physical dedup. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/core/package-manager.ts:120-204; ice:packages/coding-agent/src/core/package-manager.ts:888-987; ice:packages/coding-agent/src/core/package-manager.ts:1691-1789; ice:packages/coding-agent/src/core/package-manager.ts:2138-2601; ice:packages/coding-agent/test/ice-resource-precedence.test.ts:1-129; rust:crates/ice-cli/src/packages.rs:606-626.

### 3. What it does

The resolver combines package resources, explicit paths and automatic discovery into four ordered arrays, retaining source/scope/origin metadata and enabled state. Distribution-level globalFirst changes both which settings are considered and which duplicate resource wins.

### 4. How it works

Resolution combines several sources in an explicit order and preserves why each resource was discovered. Scope precedence is applied before final physical-file dedup, so the retained metadata and enabled state depend on that order.

The manager copies raw user/project settings. When globalFirst is enabled, each own global key among packages/extensions/skills/prompts/themes suppresses the same project key, even if the global value is an empty array. Otherwise project sources take normal precedence. Packages are identity-deduplicated with project replacing user, except a project autoload:false entry preserves the matching user base entry as well. The project delta resolves against the matching user's physical installed source/cache while retaining project metadata; the user base can then supply untouched resources. Package processing itself is sequential (ice:packages/coding-agent/src/core/package-manager.ts:888-987; ice:packages/coding-agent/src/core/package-manager.ts:1691-1789).

After packages, the resolver collects explicit local resources and automatic roots. Project automatic discovery is disabled when untrusted. Trusted project .ice resource directories participate, and .agents/skills are discovered from cwd upward through the nearest Git root inclusive; if there is no .git entry, traversal continues to filesystem root. An existing .git entry is the boundary test, not an executed Git query. The exact home .agents/skills path is excluded from project ancestors and included as user scope. User agent resource roots and home .agents/skills always use their own metadata base directory. Metadata includes source, scope user/project/temporary, origin package/top-level and optional baseDir.

Normal rank is temporary -1, project explicit 0, project automatic 1, user explicit 2, user automatic 3, package 4. Under globalFirst it is temporary -1, user explicit 0, user automatic 1, user package 2, project explicit 3, project automatic 4, project package 5. addResource normally keeps the first raw path; globalFirst can replace it when a lower-ranked incoming resource wins. Final arrays are stably sorted by rank and deduplicated by realpath where possible, falling back to the raw path on canonicalization failure. The first physical match retains its metadata and enabled state. This is physical output dedup, not canonical source identity or a directory cycle guard (ice:packages/coding-agent/src/core/package-manager.ts:120-204; ice:packages/coding-agent/src/core/package-manager.ts:2138-2601).

resolveExtensionSources(sources,{local?, temporary?}) processes its explicit inputs without the normal configured-identity dedup; temporary takes precedence over local. Its installation/default missing-source behavior is CLI-033. The full precedence suite verifies global-first empty-array suppression and no unintended project installation, while manager tests cover inherited delta packages, home/ancestor skill scoping and symlink result dedup.

### 5. Interfaces

ResolvedPaths has extensions/skills/prompts/themes arrays of {path, enabled, metadata:{source, scope, origin, baseDir?}}. resolve(), resolveExtensionSources() and globalFirst option; source identity versus raw-path versus physical-path dedup are distinct stages. Downstream resource loaders use enabled state and declaring provenance rather than rescanning arbitrary directories.

### 6. Dependencies

`CLI-021`, `CLI-029`, `CLI-030`, `CLI-031`, `CLI-033`, `SET-003`

Package settings, manifest/scan/filter primitives, acquisition and trust are prerequisites. EXT-004/031, SET-033 and SESSION-035 consume these resolved results for loading/catalog publication; they are not reverse dependencies here.

### 7. ice-rust today

Native project_package_dirs returns registry-derived directories, and native resource loading has its own discovery/precedence described in EXT/SET. The fully read package modules do not construct TS ResolvedPaths, pair autoload:false deltas with user caches, suppress project keys by global-own-key presence or apply these rank tables (rust:crates/ice-cli/src/packages.rs:606-626). Existing native lexical extension dedup is a separate, partial discovery capability.

### 8. Port notes

Introduce a typed resolved-resource model and deterministic pipeline implementing global-own-key suppression, identity dedup, inherited project deltas, automatic ancestor scopes and stable final physical dedup. Feed that model into existing native loaders while retaining native plugin discovery. Tests should assert both selected paths and surviving source/scope/enabled metadata, including global empty arrays, user/project same physical file, symlink aliases, untrusted projects and home .agents boundary cases.

### 9. Size and priority

Estimated Rust implementation: **300-490 lines** (estimate, not measured source). **P1** - The effective resource set controls loaded code, prompts, skills and themes; precedence must be predictable across distribution and project settings.

## CLI-033 - Missing package-source policy, offline short-circuiting and local-source fallback

### 1. Title and status

Missing package-source policy, offline short-circuiting and local-source fallback. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/core/package-manager.ts:65-179; ice:packages/coding-agent/src/core/package-manager.ts:1200-1349; ice:packages/coding-agent/src/core/package-manager.ts:2138-2601; ice:packages/coding-agent/test/package-manager.test.ts:1-2658; rust:crates/ice-cli/src/install.rs:155-221.; ice:packages/coding-agent/src/core/package-manager.ts:1456-1492.

### 3. What it does

Resolving resources may require installing a missing source, but callers can choose skip or error. Offline resolution and existing-cache validation have specific behavior distinct from an explicit installation command.

### 4. How it works

Resource resolution decides whether a source can be used before collecting files. A missing source can be installed, skipped or rejected, and offline mode bypasses acquisition even before asking the missing-source callback.

MissingSourceAction is install, skip or error, with install as the default. For a missing npm/Git source, offline mode returns no source before invoking the callback; otherwise the callback/action determines whether to acquire, silently omit or throw a plain Error with the message prefix 'Missing source:'. resolveExtensionSources defaults to installation and chooses temporary scope over local when both are requested. Existing sources are reused according to backend-specific validity rules rather than blindly reinstalled (ice:packages/coding-agent/src/core/package-manager.ts:65-179; ice:packages/coding-agent/src/core/package-manager.ts:1200-1349).

For npm, installed package.json is parsed and its raw version property is returned without explicit runtime type/semver validation. A falsy version fails; a parsed exact/range constraint is checked with semver.satisfies, while a source without a parsed range accepts any truthy version value. A mismatch can trigger installation during resolve. A dist-tag/non-range source therefore neither proves a valid semver nor proves the value is still the tag's current target. User-scope cache lookup may use the legacy global fallback from CLI-024. For Git, ordinary existing resolution does not reconcile a new configured pin; explicit installation/update does. Temporary unpinned online Git refresh is attempted but failures preserve the cache; pinned temporary resolution reuses its cached checkout. Offline skips missing acquisition and temporary refresh, and explicit update/check paths also short-circuit in their own methods.

Local sources resolve against the relevant scope base. An existing file is treated as one enabled extension even when the source object has resource filters. An existing directory is first treated as a package; if it is not recognized as one, it is treated as an extension directory. Stat/collection failures are caught so the local source can contribute no resources rather than aborting all resolution. Local files/directories are never cloned, copied or deleted by these source operations (ice:packages/coding-agent/src/core/package-manager.ts:1320-1349).

A recognized package with an empty valid manifest or a filter object may intentionally contribute zero files and must not be reinterpreted as an arbitrary extension directory. The tests exercise skip/error/default install, offline missing-source behavior, npm version mismatches, local file/filter behavior and temporary cached sources. No app resolution or acquisition was executed.

### 5. Interfaces

MissingSourceAction callback/policy; plain Error('Missing source: <source>'); resolveExtensionSources options local/temporary; offline truth parsing from CLI-028/018; local file-as-extension fallback. This API has no abort signal or global transaction and does not promise every source will produce a resource.

### 6. Dependencies

`CLI-020`, `CLI-022`, `CLI-024`, `CLI-025`, `CLI-026`, `CLI-029`

Source parsing, cache location, backend installation/reconciliation and package recognition are prerequisites. CLI-032 consumes acquisition results to build the effective ordered resource set.

### 7. ice-rust today

Native install::run_async refuses remote prebuilt work offline and can route explicit --build/--path work to the Cargo backend. Startup discovery can read existing native artifacts, but the reviewed code has no generic missing npm/Git source action callback, package-resource resolution default install, dist-tag validation policy or local-file-as-JS-extension fallback (rust:crates/ice-cli/src/install.rs:155-221). Those native offline checks are retained rather than replaced.

### 8. Port notes

Add acquisition policy as an explicit resolver dependency, preserving offline-before-callback behavior and backend-specific cache validity. Distinguish a valid empty package from a missing/invalid source and from a local extension file. Make install/skip/error outcomes observable without widening resource filters or mutating local source trees. Future tests should assert zero callback/spawn offline, exact/range mismatch reinstall, tag cache reuse, ordinary versus temporary Git pin behavior and empty-manifest directory handling.

### 9. Size and priority

Estimated Rust implementation: **160-270 lines** (estimate, not measured source). **P1** - Automatic resource acquisition must respect offline mode, caller policy and local-source ownership while still resolving reusable caches.

## CLI-034 - Package-command parsing, help/error precedence and update target matrix

### 1. Title and status

Package-command parsing, help/error precedence and update target matrix. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/package-manager-cli.ts:35-68; ice:packages/coding-agent/src/package-manager-cli.ts:79-413; ice:packages/coding-agent/src/package-manager-cli.ts:694-787; ice:packages/coding-agent/test/package-command-paths.test.ts:150-164; ice:packages/coding-agent/test/package-command-paths.test.ts:375-428; ice:packages/coding-agent/test/package-command-paths.test.ts:461-490; rust:crates/ice-cli/src/app.rs:136-172; rust:crates/ice-cli/src/updates.rs:522-551.

Named export: `PackageCommand` (`ice:packages/coding-agent/src/package-manager-cli.ts:33`).

`ice:packages/coding-agent/src/package-manager-cli.ts:33-35` (`PackageCommand`).

### 3. What it does

Package commands are intercepted before ordinary agent startup with their own flags, aliases, update targets and friendly errors. Native Cargo/prebuilt commands overlap in name but have different grammar.

### 4. How it works

The package parser recognizes a bounded set of subcommands and validates their flags before doing package work. Help takes precedence over otherwise invalid input, and update targets are resolved through an explicit conflict matrix.

The parser removes leading well-formed --ice-mode <value> or --ice-mode=<value> tokens, then recognizes install, remove, update and list; uninstall aliases remove. An unsupported first command returns undefined so the caller can continue other startup routing. -h/--help anywhere sets help. -l/--local is accepted only for install/remove, --force only for update, and -a/--approve versus -na/--no-approve uses the last occurrence. At most one positional source is accepted; unknown options and missing values are retained for ordered reporting. A standalone '--' is not a generic end-of-options escape here (ice:packages/coding-agent/src/package-manager-cli.ts:189-405).

Update with no explicit target selects self and prints that extension updates were skipped. Positional 'self' or 'ice' and --self select self; adding --extensions selects all. --all selects self plus extensions but conflicts with other target flags or a positional source. --models is exclusive with every other target/source. --extension requires one non-flag value, rejects a second occurrence even if it repeats the source, and conflicts with other target forms. A non-self positional source selects that extension package and conflicts with --self/--extensions/--all. --self plus --extensions is the supported explicit all combination. A source supplied to list is parsed but unused by its handler rather than receiving a dedicated list-source error.

The handler checks help first, then reports invalid option, missing option value, invalid extra argument, conflicting target, and required install/remove source errors. These friendly failures set process.exitCode=1 and return handled=true; they do not start model/provider/session work. Missing install source and unknown-option messages include command usage. Leading --ice-mode support matters because the distribution launcher injects it before package commands. Native usage-code differences must remain explicit.

The complete command-path suite covers injected mode, help, unknown/missing arguments, --models conflicts and selected update behavior. It is not an exhaustive executed matrix in this audit; the full parser body supplies the remaining branch contract.

### 5. Interfaces

install <source> [-l]; remove/uninstall <source> [-l]; list; update [source|self|ice] with --self, --extensions, --extension <source>, --models, --all, --force; -a/--approve, -na/--no-approve, -h/--help. handlePackageCommand returns boolean handled; command errors set exitCode 1.

The exported command discriminant is exactly `PackageCommand = "install" | "remove" | "update" | "list"`; `uninstall` is normalized by the parser rather than being an additional union member.

### 6. Dependencies

This parser is a prerequisite for trust and command execution. Generic startup/CLI-002 owns when the early handler is invoked.

### 7. ice-rust today

Native app.rs dispatches update/install/uninstall/package before generic argument parsing. Native update accepts --help/-h, --force/-f and --build; native install/uninstall and package link/list/update/unlink use Cargo/prebuilt grammar and usage failures generally return 2. They do not implement the TS update target matrix, remove alias semantics, package list route or --models command (rust:crates/ice-cli/src/app.rs:136-172; rust:crates/ice-cli/src/updates.rs:522-551; rust:crates/ice-cli/src/install.rs:1075-1300; rust:crates/ice-cli/src/packages.rs:30-107). Historical pi package command names are explicitly rejected, a native behavior to retain.

### 8. Port notes

Add resource-package command parsing and an explicit dispatch policy that preserves native package/build routes. Port help-first and target-conflict behavior with deterministic exit messages. Resolve any intentional exit-code difference in the compatibility contract instead of claiming code 2 equals TS code 1. Future table-driven checks should cover every mutually exclusive target, missing --extension values, repeated approval flags, injected mode prefixes and help mixed with invalid input.

### 9. Size and priority

Estimated Rust implementation: **180-290 lines** (estimate, not measured source). **P1** - Overlapping command names make ambiguous routing dangerous; scripts need predictable targets, flags and error status.

## CLI-035 - Package-command project trust and saved-trust-only update policy

### 1. Title and status

Package-command project trust and saved-trust-only update policy. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/package-manager-cli.ts:563-620; ice:packages/coding-agent/src/package-manager-cli.ts:694-821; ice:packages/coding-agent/test/package-command-paths.test.ts:134-374; rust:crates/ice-cli/src/packages.rs:48-190; rust:crates/ice-cli/src/updates.rs:819-1050.

Named export: `PackageCommandRuntimeOptions` (`ice:packages/coding-agent/src/package-manager-cli.ts:554`).

`ice:packages/coding-agent/src/package-manager-cli.ts:554-556` (`PackageCommandRuntimeOptions`).

### 3. What it does

Package commands resolve project trust before exposing project settings or executing scoped package-manager commands. Updates deliberately use saved trust rather than prompting or invoking project-trust extensions.

### 4. How it works

Command settings start with project access disabled. Normal package commands can resolve trust through the configured policy, while update uses only remembered trust plus an explicit command-line override.

createCommandSettingsManager constructs SettingsManager with projectTrusted=false. For update, useSavedProjectTrustOnly reads whether the trust store explicitly marks cwd true and applies any explicit approval override; it does not treat defaultProjectTrust='always' or a project_trust extension as saved trust. The update path does not prompt and does not ask extensions to grant project trust. Only then is trusted project settings access enabled (ice:packages/coding-agent/src/package-manager-cli.ts:573-620).

Other package/config commands can load trust extensions when there is no explicit override and the project contains trust-requiring resources. Warnings are collected and printed; trust resolution receives the current application mode and interactive=true only when both stdin and stdout are TTY. Explicit --approve/--no-approve overrides take precedence, remembered false can override a permissive default, and fresh projects with no trust-requiring resources can initialize local settings according to the trust resolver's policy. The package handler drains SettingsManager errors as warnings rather than silently losing parse/write issues.

Local install/remove/config mutations reject an untrusted project with the explicit instruction to use --approve. This occurs before invoking the package mutation. User-scope operations remain available while untrusted project package settings are hidden. For self-update, npmCommand is read from global settings only, even if a trusted project's settings contain a different command prefix. The explicit model-catalog target runs before command settings/trust construction and therefore does not load project package configuration.

The command-path suite verifies hidden untrusted packages, remembered trust, explicit overrides, default policy, extension trust for list, saved-only update even with default 'always', no extension trust callback during update, local mutation blocking and fresh-project local initialization. These were source-read assertions, not executed trust prompts.

### 5. Interfaces

Command trust override is boolean/undefined with last approval flag winning. SettingsManager projectTrusted state gates project packages and npmCommand. update's saved-trust-only policy is distinct from install/remove/list/config trust resolution. Warnings are printed with context; handled failures set exitCode 1.

`PackageCommandRuntimeOptions { extensionFactories?: InlineExtension[] }` carries optional inline factories into package-command resource/trust initialization; it adds no new command-line flag.

### 6. Dependencies

`CLI-021`, `CLI-034`, `SET-003`, `SET-007`, `SET-008`

SET-003 owns trust persistence/extension policy; SET-007/008 scoped settings. This entry owns command-specific trust call order and the stricter update policy.

### 7. ice-rust today

Native project package link/update builds Cargo code directly from its source and does not invoke the TS command SettingsManager/trust-extension pipeline. Native self-update does independently harden build context by using a private command directory, removing Rust wrapper environment variables and rejecting project-local configured CARGO_HOME; those safeguards are retained under CLI-043 (rust:crates/ice-cli/src/packages.rs:48-190; rust:crates/ice-cli/src/updates.rs:819-1050). They do not supply saved-trust-only access to project npm resource settings.

### 8. Port notes

Wire package commands to the shared native trust/settings service with separate normal-command and saved-only-update policies. Gate local mutations before invoking processes and use global command configuration for self-update. Preserve native self-update build-context isolation. Future tests should assert no project extension load/prompt during update, false remembered trust versus permissive defaults, command-line overrides, fresh local initialization and no project npmCommand influence on self-update.

### 9. Size and priority

Estimated Rust implementation: **130-220 lines** (estimate, not measured source). **P0** - Package-manager commands can execute code; project-controlled settings must not gain execution authority through an update or self-update path.

## CLI-036 - Install/remove/list output and config-selector resolution/launch

### 1. Title and status

Install/remove/list output and config-selector resolution/launch. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/package-manager-cli.ts:621-693; ice:packages/coding-agent/src/package-manager-cli.ts:788-905; ice:packages/coding-agent/test/package-command-paths.test.ts:107-238; ice:packages/coding-agent/test/package-command-paths.test.ts:430-459; rust:crates/ice-cli/src/packages.rs:192-232; rust:crates/ice-cli/src/install.rs:399-483.

### 3. What it does

Command handlers call scoped package operations and present configured packages, while config resolves resources separately for global and project views before opening the resource selector.

### 4. How it works

Command output reflects the operation that actually completed. The config command builds resource views for the selector rather than directly rewriting every resource setting itself.

Install awaits installAndPersist and prints the installed source. Remove awaits removeAndPersist; false means no matching configured package and sets exitCode=1 even though the payload-removal phase already ran. The progress callback prints start events, not every possible progress event. List groups raw configured entries into User packages and Project packages, marks object entries as '(filtered)', and prints the installed path when available. Empty configuration prints 'No packages installed.'; a configured but missing payload still appears as a configured package, so the heading is not an integrity guarantee (ice:packages/coding-agent/src/package-manager-cli.ts:788-905).

handleConfigCommand strips leading mode arguments and recognizes config, checks help before errors, accepts -l/--local and last-wins approval/no-approval flags, and rejects other options/positionals. It resolves command trust and blocks an untrusted local write scope. For the global view it creates a settings manager with project access disabled and resolves global resources. If project access is trusted it also resolves a project-effective view; otherwise the project view reuses the global result. The selector receives cwd, agentDir, settings manager, writeScope and projectModeAvailable (ice:packages/coding-agent/src/package-manager-cli.ts:621-693).

The CLI wrapper waits for the terminal selector and then exits 0. CLI-016 owns its terminal setup/teardown wrapper, TUI-068 owns grouping/navigation and TUI-069 owns the exact enable/disable/inherit setting mutations. This handler owns acquisition/resolution before launch: resolution may install missing sources through CLI-033 and must obey its trust/offline policy. Error handling prints contextual settings/trust warnings and a friendly command error with exitCode=1.

The read command tests cover relative install/remove identity, trusted list grouping and the project resource override cycle. They do not demonstrate a native config UI invocation; the Rust command names currently differ.

### 5. Interfaces

handlePackageCommand(args, options?) and handleConfigCommand(args, options?) return handled booleans. List output is source plus optional resolved path, grouped by scope; filtered means object form. Config selector inputs include resolved global/project views, writeScope and projectModeAvailable. TUI resource mutations are not duplicated here.

### 6. Dependencies

`CLI-016`, `CLI-021`, `CLI-032`, `CLI-034`, `CLI-035`

The package manager and trust/parser provide operation inputs. CLI-016 is the existing selector wrapper prerequisite; TUI-068/069 are its component dependencies and should not become reverse links.

### 7. ice-rust today

Native package list reads only the project native registry and reports its native records; native uninstall operates the global artifact registry and returns success when a crate is not installed. These are materially different from TS configured-package listing and remove-no-match exit 1. No same early 'config' resource-selector route is present in the inspected native app dispatcher (rust:crates/ice-cli/src/packages.rs:192-232; rust:crates/ice-cli/src/install.rs:399-483; rust:crates/ice-cli/src/app.rs:136-172). Existing native package commands remain supported.

### 8. Port notes

Implement thin command handlers over the new resource manager, keeping configured versus installed state explicit and matching scope/output/error semantics. Supply the selector with independently resolved global/project views and trust-aware write scope through CLI-016. Preserve native package list/uninstall routes under an unambiguous backend grammar. Future tests should cover missing configured payloads, object filter labels, remove false after payload handling, untrusted project view suppression, selector cancellation and settings-warning drainage.

### 9. Size and priority

Estimated Rust implementation: **130-220 lines** (estimate, not measured source). **P1** - Users need reliable command feedback and a resource configuration route that reflects the actual resolver and trust state.

## CLI-037 - Explicit model-catalog refresh through update --models

### 1. Title and status

Explicit model-catalog refresh through update --models. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/package-manager-cli.ts:414-441; ice:packages/coding-agent/src/package-manager-cli.ts:694-787; ice:packages/coding-agent/test/package-command-paths.test.ts:393-428; rust:crates/ice-cli/src/updates.rs:522-551.

### 3. What it does

The models update target refreshes model catalogs without updating packages or the CLI and without constructing project package settings.

### 4. How it works

The models target performs one dedicated catalog refresh and then returns. It separates local runtime construction from the explicitly network-enabled refresh call.

After parser validation, update --models takes an early branch before command SettingsManager/project-trust/package-manager construction. It creates an AbortController and a 15000 ms timer, calls ModelRuntime.create with agentDir/auth.json, agentDir/models.json, allowModelNetwork=false and the signal, then refreshes with allowNetwork=true, force=true and the same signal. The timer requests cooperative cancellation; it is not an independent hard-kill deadline for an implementation that ignores the signal (ice:packages/coding-agent/src/package-manager-cli.ts:414-441).

An aborted refresh becomes an error. Provider errors returned by refresh are combined into command failure rather than printing a success for a partial failed catalog refresh. The timer is cleared in finally. On success the command prints 'Model catalogs refreshed'; error handling prints the error and sets exitCode=1. The target cannot combine with self/extensions/all/individual package targets, as enforced before runtime creation.

The command-path test asserts exact auth/models paths, no network during create, explicit forced network permission during refresh, shared AbortSignal and early rejection of conflicting targets. General catalog publication, provider-specific data and cancellation semantics are owned by AI-002/AUTH; this entry specifies only the CLI boundary and its ordering.

### 5. Interfaces

update --models; ModelRuntime.create({authPath, modelsPath, allowModelNetwork:false, signal}); runtime.refresh({allowNetwork:true, force:true, signal}); 15000 ms cooperative abort timer; success message and command failure status.

### 6. Dependencies

`AI-002`, `CLI-034`, `SET-001`

AI-002 owns ModelRuntime catalog lifecycle, network permissions, publication and cancellation. SET-001 supplies agent paths. AUTH-032 is a different startup helper and is intentionally not used as the generic refresh dependency.

### 7. ice-rust today

Native self-update parser accepts only help/force/build and the inspected app update route delegates to it. It exposes no --models target that constructs a local catalog runtime then explicitly enables a forced refresh (rust:crates/ice-cli/src/updates.rs:522-551; rust:crates/ice-cli/src/app.rs:136-172). Native model/provider catalogs exist elsewhere and are credited under AI/AUTH, but that does not implement this command.

### 8. Port notes

Add the dedicated model-refresh route above the native self-update parser and call the AI-002 runtime with separate create/refresh network permissions. Keep project package trust/settings out of this branch. Aggregate errors and propagate cooperative cancellation accurately. Future tests should prove no package/self-update operation occurs, conflicting targets fail before runtime creation, the deadline signal reaches both stages and partial provider failures do not print success.

### 9. Size and priority

Estimated Rust implementation: **55-95 lines** (estimate, not measured source). **P1** - Users need an explicit catalog refresh independent of package or binary updates, with clear network authority and failure reporting.

## CLI-038 - Self-update planning, global command selection and extension-before-self sequencing

### 1. Title and status

Self-update planning, global command selection and extension-before-self sequencing. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/package-manager-cli.ts:442-562; ice:packages/coding-agent/src/package-manager-cli.ts:788-905; ice:packages/coding-agent/test/package-command-paths.test.ts:492-771; rust:crates/ice-cli/src/updates.rs:668-786.

### 3. What it does

TS self-update asks for an exact target release and derives a platform/install-method command from global settings. Combined updates complete extension updates first and never claim self-update success after a command failure.

### 4. How it works

Self-update checks the target release before deciding whether to run an installer. When extensions and self are both selected, extension updates finish first; a failure stops the later self-update stage.

getSelfUpdatePlan calls getLatestRelease(VERSION), wraps lookup failure and rejects missing metadata. It takes the returned packageName when available or the current PACKAGE_NAME, builds the exact '<package>@<version>' install spec, and runs when forced, when the package name changed or when the version is newer. Otherwise it reports already up to date. Explicit self-update still checks when automatic version checking is disabled; the version-check helper owns that distinction. A forced update still uses the checked version, not an unqualified latest install (ice:packages/coding-agent/src/package-manager-cli.ts:493-520; ice:packages/coding-agent/test/package-command-paths.test.ts:492-615).

The handler reads npmCommand from global settings only. Install-method/command derivation is SET-002. Windows permits only npm/pnpm self-update methods; unavailable methods print a manual fallback. Before an npm update on Windows it invokes cleanup and loaded-native-module quarantine helpers (MISC-002). Steps run sequentially with structured argv and inherited stdio, waiting for close; spawn, nonzero exit or signal errors fail the operation. A package rename can execute uninstall-old then install-new; failure of the second step does not reinstall the old package. Tests explicitly assert this nontransactional sequence and failure status (ice:packages/coding-agent/src/package-manager-cli.ts:521-562; ice:packages/coding-agent/test/package-command-paths.test.ts:617-769).

An optional release note is trimmed and rendered as Markdown at at least 20 columns (stdout width or fallback 80), with plain-text fallback if rendering fails. A pnpm failure adds the metadata/store-prune hint and manual command guidance. Success reports previous and target version only after every command step succeeds. The default no-target update selects self and prints that extension updates were skipped; --all or self+extensions first awaits the package update path, then runs this plan (ice:packages/coding-agent/src/package-manager-cli.ts:442-492; ice:packages/coding-agent/src/package-manager-cli.ts:788-905).

Remote metadata validation and package-migration trust are MISC-001, not reimplemented here. Native self-update has a different artifact-staging/activation architecture that should be retained rather than replaced with an npm-only strategy.

### 5. Interfaces

SelfUpdatePlan includes checked target version/package install spec, note and shouldRun decision. SelfUpdateCommand supplies method, ordered steps and manual command. --force affects version comparison but does not bypass the TS release lookup. Global npmCommand only; Windows quarantine is an invocation prerequisite, not proof of native executable activation.

### 6. Dependencies

`CLI-027`, `CLI-034`, `CLI-035`, `MISC-001`, `MISC-002`, `SET-002`

CLI-027 supplies extension updates for combined targets; SET-002 chooses safe commands/install method; MISC-001 validates release metadata/comparison and MISC-002 owns loaded-DLL quarantine/cleanup. Native activation counterparts are preserved in CLI-043–046.

### 7. ice-rust today

Native update checks crates.io, stages a Cargo-built or verified prebuilt executable, validates it and activates it directly or through a detached Windows helper. It does not use the TS package-name migration/global npmCommand plan, --self/--extensions target matrix or note rendering. Native --force --build can skip metadata and Cargo build does not pin the looked-up version, unlike the TS exact spec (rust:crates/ice-cli/src/updates.rs:668-786; rust:crates/ice-cli/src/updates.rs:819-895). Native safeguards and defects are specified separately.

### 8. Port notes

Unify user-visible update target sequencing while retaining the native staged binary updater. Reuse MISC-001 for any TS-compatible update-service metadata and SET-002/MISC-002 only when updating a JS installation. Never let project npmCommand affect self-update. Report partial package-name migration accurately and choose a recovery strategy rather than claiming current TS rollback. Future checks should cover current forced version, renamed package install failure, extensions failure before self, unsupported Windows methods, close/signal errors and Markdown fallback.

### 9. Size and priority

Estimated Rust implementation: **150-250 lines** (estimate, not measured source). **P1** - Self-update can replace the running installation; exact target selection, trusted command scope and honest failure sequencing are essential.

## CLI-039 - Native project package links: preserve Cargo workflow and close name-containment holes

### 1. Title and status

Native project package links: preserve Cargo workflow and close name-containment holes. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/package-manager.ts:973-1041; ice:packages/coding-agent/src/core/package-manager.ts:2008-2136; rust:crates/ice-cli/src/packages.rs:1-819; rust:crates/ice-cli/src/app.rs:136-172.

### 3. What it does

Rust already supports project-local Cargo package link/list/update/unlink commands. This native-only workflow should remain available, but its permissive project-name validator and unvalidated registry names can escape the intended package-name directory.

### 4. How it works

TS local resource installation records an existing source path without copying it. Rust's project link command instead builds a Cargo dynamic library and installs a copied artifact under the project, so its filesystem mutation boundary must be evaluated independently.

Native 'package link' accepts --path, --package/-P, --name or one positional display name, --debug, --locked and --force/-f. It resolves the source against cwd, runs cargo metadata --format-version 1 --no-deps, selects the requested Cargo package or a unique cdylib candidate (preferring the canonical source-containing manifest where unambiguous), then builds using JSON compiler-artifact messages and release by default. The build occurs before the existing target/no-force check. It selects a dynamic library artifact and targets cwd/.icers/packages/<name>/extensions. Force removes the old target before copying through a fixed adjacent temporary filename and rename. Registry parsing/updating occurs after artifact activation, so a corrupt registry or later write failure can leave changed payload without matching metadata (rust:crates/ice-cli/src/packages.rs:48-190; rust:crates/ice-cli/src/packages.rs:468-605).

The project registry is {packages:[{name, version, source, cargo_package, artifacts?}]} with missing artifacts defaulting empty. Writes use .packages.json.tmp then rename, without the global installer's mutation lock or multi-file rollback. List reads only this project registry. Update strictly reads both project and global registries before rebuilding; it relinks selected project records with force, then invokes the current executable's native install command for eligible global records. It preserves custom GitHub/prebuilt versus --build backend, excludes local-source global entries, continues after individual failures and returns aggregate failure. Project --locked/debug affect Cargo rebuilds, not every global installer invocation. Unlink validates one name, requires a matching record, removes its directory before rewriting the registry, and does not have rollback (rust:crates/ice-cli/src/packages.rs:192-467).

Source-proven containment defect: valid_name accepts any nonempty ASCII alphanumeric/dash/underscore/dot sequence, including '.' and '..'. Direct app dispatch reaches package::run; a supplied --name '..' passes validation. Given a valid source/build, joining that name can direct the artifact into cwd/.icers/extensions and persist '..' if the registry write succeeds. Unlink '..' then requires that registry record and calls remove_dir_all on cwd/.icers/packages/.. before updating metadata. The audit did not execute this or establish OS-specific deletion effects for a terminal parent component. Cargo-derived selected.name uses the same validator, but Cargo's own permitted-name constraints were not independently audited. Parsed registry names have no semantic revalidation, and project_package_dirs directly joins registry name/kind, so malformed stored names affect discovery as well (rust:crates/ice-cli/src/packages.rs:123-166; rust:crates/ice-cli/src/packages.rs:388-421; rust:crates/ice-cli/src/packages.rs:573-626).

Inline tests cover backend selection, custom repositories, continuing after failures and corrupt-global-registry early abort. The name test rejects slash traversal and empty names but does not cover '.'/'..'. All tests were read only. Native package linking copies the library, not all skills/prompts/themes; the directory-discovery interface can expose those kinds only when corresponding directories exist through other means.

### 5. Interfaces

Native commands package link/list/update/unlink; .icers/packages/<name>/extensions; .icers/packages.json and fixed .packages.json.tmp; Cargo package/display-name split. Source-only preconditions for the defect are a directly supplied dot name plus successful metadata/build/link persistence, or an existing matching malformed registry record for unlink.

### 6. Dependencies

No cross-area prerequisite is required to correct the native path boundary. New npm/Git/local compatibility sources remain CLI-020–033 and must not be conflated with this Cargo workflow.

### 7. ice-rust today

The native implementation is substantial and directly reachable at rust:crates/ice-cli/src/app.rs:157. Its global installer has stricter non-dot crate names and transactional artifact handling; those safeguards do not automatically apply to project package link/unlink. The exact difference is unchecked dot/registry-name containment and nontransactional project mutation, not absence of all package management. Native source hashes and all inline tests are recorded in cli-packaging-supplement-source-review.json.

### 8. Port notes

Keep native Cargo link/update/list/unlink, but reject '.'/'..' and any non-single-component name at every direct and registry-derived boundary. Validate a target as a strict descendant of the intended package root before copy/removal/discovery, and revalidate registry names before using them. Treat symlink/canonical containment and existing project data compatibility explicitly. Reuse the global mutation staging/locking pattern where practical so registry corruption is detected before payload activation and failed force replacement can recover. The estimated change covers this boundary/ordering correction, not reimplementation of Cargo link or TS source backends. Future source-driven checks should cover dot names, corrupted/untrusted registries, symlinked destinations and failure between payload/registry updates without running destructive operations during this audit.

### 9. Size and priority

Estimated Rust implementation: **100-190 lines** (estimate, not measured source). **P0** - A user-controlled package name reaches recursive removal and artifact paths without enforcing the intended project package boundary.

## CLI-040 - Native Cargo artifact installation, ownership locking and rollback to preserve

### 1. Title and status

Native Cargo artifact installation, ownership locking and rollback to preserve. **Status: different-by-design.**

### 2. ice source

ice:packages/coding-agent/src/core/package-manager.ts:973-1041; ice:packages/coding-agent/src/core/package-manager.ts:1730-1812; rust:crates/ice-cli/src/install.rs:1-1928.

### 3. What it does

The Rust package installer already provides a native Cargo/prebuilt artifact registry with ownership checks, locked mutations and rollback. These capabilities are distinct from TS npm/Git resource packages and should be retained when compatibility backends are added.

### 4. How it works

TS resource-package installation delegates to npm-family or Git commands and then edits settings. Rust's global installer instead owns a set of native library artifacts and updates that set together with its registry under a lock.

Native run_async reads the registry strictly before work. --build or --path uses a blocking Cargo path; otherwise online prebuilt installation is the default, with no automatic Cargo fallback after prebuilt failure. The parser accepts crate name, --version/-V, --path, --locked, --force/-f, --build and --github with mutually exclusive combinations. Local path can infer the selected crate via canonical manifest metadata. Global crate names allow ASCII alphanumeric/dash/underscore and reject dot components (rust:crates/ice-cli/src/install.rs:155-398; rust:crates/ice-cli/src/install.rs:1075-1362).

Cargo installation creates a temporary build workspace and dependency alias, performs fetch/metadata, identifies the selected package and requires a cdylib, builds release artifacts and finds platform dynamic libraries (including .pyd in the artifact matcher). Version policy is semver precedence: force or no current record permits installation, greater versions upgrade, equal versions skip, and downgrade/invalid version comparison errors unless force bypasses that comparison. The Cargo command inherits stdio and forwards locked/offline where applicable; there is no package-build cancellation/deadline contract in this path.

Mutation takes an exclusive fs2 try-lock at native-packages.lock, rereads the registry and rechecks the version under lock. Artifact inputs must be regular nonsymlink files with flat UTF-8 dynamic-library filenames, unique by the platform's identity rule (case-insensitive on Windows). Another package's owned filename conflicts even with force. Unowned existing target files require force. The installer stages create_new adjacent UUID files, copies/syncs contents and preserves permissions, backs up affected regular nonsymlink files, activates replacements and writes the sorted registry last. On failure it reverses activations/backups; rollback errors are surfaced and backup cleanup failures warn. This is a multi-file rollback protocol, not a crash-atomic filesystem transaction or parent-directory fsync guarantee (rust:crates/ice-cli/src/install.rs:484-1074).

Uninstall uses the same lock and strict registry. Explicit recorded artifacts are authoritative; legacy records use bounded normalized stem matching. It scans dynamic library files only, removes transactionally with the remaining sorted registry, and removes the registry when last package is gone. A not-installed crate prints a message and exits successfully. Startup discovery uses best-effort registry reading, unlike fail-closed mutation. Inline tests cover corrupt registry, forced upgrades, unowned/foreign-owned collisions, multiple artifact replacement, rollback, legacy provenance and uninstallation; they were read, not run.

### 5. Interfaces

InstalledNativePackage includes name, version, optional source, optional prebuilt provenance and optional artifacts. Native package registry, native-packages.lock and platform library directory are independent from TS settings packages arrays. Registry version/artifact ownership and force are native protocol surfaces.

### 6. Dependencies

This is a preservation entry. New source parsing/resource caches are separately estimated in CLI-020–033; native prebuilt validation is CLI-041. No missing transaction implementation is inferred.

### 7. ice-rust today

All described native transaction behavior is present in install.rs and its inline tests. TS npm/Git command delegation does not provide this same artifact-level ownership transaction. The absence of npm/Git resource backends remains a gap elsewhere; it must not be used to erase or double-count the native installer. Native comparison source: `rust:crates/ice-cli/src/install.rs:1-1928`.

### 8. Port notes

Retain the Cargo backend, strict mutation registry, artifact ownership checks, lock, staging and rollback when adding resource-package commands. Do not replace them with TS's weaker payload-then-settings ordering. Keep backend-specific registries separate and expose native versus resource-package identities explicitly in the command facade. No new Rust implementation is required for this retained capability; size is zero and compatibility work is counted in the owning source/CLI entries.

### 9. Size and priority

Estimated Rust implementation: **0-0 lines** (estimate, not measured source). **P3** - Preservation-only native capability; existing transactional implementation should not be counted as missing port work.

## CLI-041 - Verified native extension prebuilt distribution and publisher provenance to preserve

### 1. Title and status

Verified native extension prebuilt distribution and publisher provenance to preserve. **Status: different-by-design.**

### 2. ice source

ice:packages/coding-agent/src/core/package-manager.ts:1352-1432; ice:packages/coding-agent/src/core/package-manager.ts:1826-1861; rust:crates/ice-cli/src/prebuilt_extensions.rs:1-1403; rust:crates/ice-cli/src/install.rs:155-221.

### 3. What it does

Rust can install verified prebuilt native extensions from publisher releases without building Cargo source. The protocol validates manifest identity, platform/ABI and archive/library integrity before handing artifacts to the transactional installer.

### 4. How it works

TS npm/Git packages use their external package manager or checkout transport. Native prebuilt extensions instead follow a bounded release-manifest protocol and retain publisher provenance for later updates.

The native host target set is Windows x86_64 MSVC, Linux x86_64/aarch64 GNU and macOS x86_64/aarch64; unsupported targets must use the build path. Repository canonicalization accepts owner/repo or HTTPS GitHub forms (including supported repository subpaths), trims git+/.git/trailing syntax and lowercases owner/repository, rejecting auth, ports and invalid/dot components. crates.io resolution normalizes '-'/'_' identity, chooses the highest non-yanked stable version when unspecified and requires an exact requested non-yanked version when specified. Explicit --github plus version can bypass crates.io; explicit repository without version chooses the highest stable '<name>-v<version>' tag. A changed publisher must be explicit rather than silently switching an existing installation (rust:crates/ice-cli/src/prebuilt_extensions.rs:1-424).

The ice-extension.json schema must be version 1 with matching package/version/tag/repository, 40-character lower-case source commit, valid SDK semver, matching plugin ABI, exact host target and a zip/tar.gz artifact. Archive URL must be HTTPS on the same GitHub repository/tag, with the expected release asset path and no auth/port/query/fragment. Library filename is flat, ASCII-constrained and platform-suffixed. SHA-256 fields are 64 lower-case hex characters. Metadata is bounded at 8 MiB, manifest at 256 KiB, archive at 256 MiB, library at 128 MiB and entries at 128; archive size must be positive and match the manifest (rust:crates/ice-cli/src/prebuilt_extensions.rs:425-803).

HTTP metadata uses a 15-second client timeout and archive work 180 seconds. Limited-body reading checks both declared content length and streamed bytes. ZIP/tar.gz extraction never blindly unpacks paths: it rejects traversal, absolute/backslash/colon forms, links and special entries, counts entries and expanded sizes, and extracts the single declared library once. It verifies archive and extracted-library hashes. Object validation checks PE64 AMD64 DLL, ELF64 little-endian shared-object/architecture, or Mach-O64 little-endian dylib/CPU type without executing the library. Exported runtime ABI is still checked when loading later; manifest SDK/ABI checks do not prove every exported function is correct.

The persisted provenance records repository, target, manifest/source/SDK/ABI/archive/library hashes and whether the repository was custom. Updates preserve the chosen backend/publisher and validate compatibility for a newer release before offering it. An unchanged/older version can be returned without fetching a new target manifest. Installation delegates to CLI-040's artifact transaction. All inline tests and local-HTTP fixtures were read, including platform/object validation, publisher/tag scoping, archive bounds and explicit unpublished package installation; none ran.

### 5. Interfaces

ice-extension.json schema 1; '<name>-v<version>' release tag; native artifact/provenance registry; --github, --version, --build and --force. Supported target triples and byte/entry limits are protocol constraints. No npm resource manifest or general Git checkout semantics are implied.

### 6. Dependencies

`CLI-040`

The verified output library is activated through the existing locked transaction. Runtime plugin loading/ABI export checks remain EXT-owned. This retained protocol is also reused by CLI prebuilt self-update helpers, with a distinct repository-case defect in CLI-045.

### 7. ice-rust today

The implementation and inline tests are fully present in prebuilt_extensions.rs. It is a useful native-only capability absent from TS's npm/Git package transport. The helper canonicalization is internally consistent for its canonical repository inputs; the separate CLI prebuilt caller's historical mixed-case constant is corrected in the active checkout (RUST-045), and never established that every extension prebuilt install was broken. Native comparison source: `rust:crates/ice-cli/src/prebuilt_extensions.rs:1-1403`; `rust:crates/ice-cli/src/install.rs:155-221`.

### 8. Port notes

Preserve the bounded manifest/archive protocol, publisher continuity, provenance and object validation while adding TS resource backends. Keep verification before activation and runtime ABI loading after it. No new Rust implementation is estimated for this retained capability. Do not loosen archive/path/hash limits to make TS package naming fit, and do not treat downloaded native libraries as equivalent to JS package directories.

### 9. Size and priority

Estimated Rust implementation: **0-0 lines** (estimate, not measured source). **P3** - Preservation-only verified native distribution; it should remain intact and contributes no missing-port LOC.

## CLI-042 - Native startup update cache, per-entry freshness and backend-aware notices to preserve

### 1. Title and status

Native startup update cache, per-entry freshness and backend-aware notices to preserve. **Status: different-by-design.**

### 2. ice source

ice:packages/coding-agent/src/core/package-manager.ts:1120-1200; ice:packages/coding-agent/src/core/package-manager.ts:2138-2510; rust:crates/ice-cli/src/updates.rs:1-521; rust:crates/ice-cli/src/updates.rs:2408-3531.

### 3. What it does

Native startup checks already use bounded concurrency, per-entry metadata freshness and backend-aware update notices. Resource-package checks can be added without replacing those native reliability properties.

### 4. How it works

TS package update discovery probes installed npm/Git packages and suppresses metadata errors. Native startup additionally retains recent successful version metadata and uses it only for transient lookup failures.

check_startup ignores its cwd parameter for native package selection; it checks the global native registry and the CLI. ICERS_OFFLINE or the presence of ICERS_DISABLE_UPDATE_CHECK, even an empty value, disables remote checks while still consuming self-update status warnings. The startup HTTP client has a 1800 ms request timeout and the native user-agent configuration. Extension checks use buffer_unordered with concurrency four; the CLI version check runs concurrently outside that four-item pool. Locally sourced global native packages are excluded (rust:crates/ice-cli/src/updates.rs:120-253).

Registry results distinguish Found(version), Missing for HTTP 404/410, and Transient for other failures including throttling, timeouts and malformed metadata. Cached metadata is reused only on Transient and only when checkedAt>0 with age between zero and six hours inclusive. Fallback does not renew freshness. Missing removes the old result. The cache file is update-check.json with legacy checkedAt plus newer per-entry timestamps; cache updates lock, reread and merge sections before best-effort atomic writing so concurrent CLI/native results do not clobber each other. There is no claim that all cached metadata is freshly network-verified (rust:crates/ice-cli/src/updates.rs:254-357; rust:crates/ice-cli/src/updates.rs:445-493).

Cargo entries use a crate-name cache identity; prebuilt entries include the distribution backend, repository/name and target to avoid cross-publisher/platform reuse. Before reporting, the code rereads the installed registry and checks semver 'newer than', not mere string inequality. Notice commands preserve the appropriate native update/install backend and custom repository. CLI notices use 'icers update'. These semantics differ from TS npm/Git inequality probes and are native capabilities to preserve, not a missing one-to-one algorithm (rust:crates/ice-cli/src/updates.rs:358-419; rust:crates/ice-cli/src/updates.rs:474-521).

Explicit offline self-update uses cached_latest_ice without this startup six-hour freshness check; that separate informational path is CLI-043. Full inline tests cover transient versus missing fallback, freshness boundaries, cache merging, publisher/platform keys, worker bounds and notice behavior. No network or startup check ran during this audit.

In the active checkout, the persisted freshness object uses the required icers field; the older ice field has no serde alias or field default. Reusing a pre-rename cache with that older freshness object makes read_cache return None on deserialization failure, so it contributes no cached metadata until rebuilt. The enclosing freshness field still defaults when entirely absent, and the aggregate checkedAt fallback remains. This is a source-derived compatibility consequence for reused/copied cache files; no private cache or runtime migration was examined (rust:crates/ice-cli/src/updates.rs:64-82; rust:crates/ice-cli/src/updates.rs:254-259; rust:crates/ice-cli/src/updates.rs:321-340; rust:crates/ice-cli/src/updates.rs:482-484).

### 5. Interfaces

ICERS_DISABLE_UPDATE_CHECK presence semantics; ICERS_OFFLINE native gate; update-check.json cache; UpdateReport notices/warnings; Found/Missing/Transient classification; six-hour fallback, 1800 ms requests and concurrency four plus independent CLI request. TS ICE_SKIP_VERSION_CHECK/update-service semantics remain MISC-001.

### 6. Dependencies

`CLI-041`

Prebuilt provenance and compatibility lookup supply backend-aware identities. New npm/Git discovery is separately owned/estimated by CLI-028; TS release HTTP metadata is MISC-001.

### 7. ice-rust today

All described native caching and notice behavior is present in updates.rs. It is stronger/different than the package manager's uncached npm/Git notice probes. The missing source backends and explicit CLI target grammar are separately captured, and should not be double-counted as absent caching infrastructure. Native comparison source: `rust:crates/ice-cli/src/updates.rs:1-521`; `rust:crates/ice-cli/src/updates.rs:2408-3531`.

### 8. Port notes

Retain native result classification, freshness, merge locking and publisher-aware keys. Add resource-package notices through explicit backend-tagged records and shared presentation only after CLI-028 exists. Preserve the distinction between automatic native disable flags and TS explicit update-service behavior. No replacement implementation is required for this retained capability; size is zero.

### 9. Size and priority

Estimated Rust implementation: **0-0 lines** (estimate, not measured source). **P3** - Preservation-only native reliability behavior; resource-package notice work is already counted elsewhere.

## CLI-043 - Native self-update source selection, isolated builds and already-staged retry defect

### 1. Title and status

Native self-update source selection, isolated builds and already-staged retry defect. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/package-manager-cli.ts:493-562; rust:crates/ice-cli/src/updates.rs:522-1090; rust:crates/ice-cli/src/updates.rs:2408-3531.

### 3. What it does

Native self-update prepares a new executable in an isolated staging directory and selects Cargo-build versus prebuilt acquisition. Its already-staged branch does not actually suppress all duplicate work claimed by the surrounding comments.

### 4. How it works

TS self-update selects an exact checked package version and runs package-manager steps. Rust prepares a native executable, with separate metadata lookup, staged-version selection and platform activation.

Native arguments accept help, force and build after the app's offline normalization; other target flags produce usage code 2. Offline self-update consumes existing status/artifact information, reports a last-known cached version and exits without downloading/building. That informational cached version is not subject to the startup six-hour freshness test. Online lookup runs through a separate Tokio thread/runtime with a 15-second request timeout and can fall back to fresh cached metadata marked verified=false. The prebuilt path requires a known target version but does not reject that fallback solely because verified is false; artifact manifest/hash validation still occurs later (rust:crates/ice-cli/src/updates.rs:522-606; rust:crates/ice-cli/src/updates.rs:668-818).

When --force --build is selected, metadata lookup can be skipped. Cargo preparation uses a private command directory under the agent home instead of the project cwd, removes the four Rust compiler wrapper/workspace wrapper environment overrides in the implementation, validates configured CARGO_HOME as absolute and outside the project (including missing-path canonicalization), and runs cargo install --root <staging> ice-cli --locked --force --quiet. It does not pass the previously looked-up version with --version. The final binary validation enforces acceptable identity/version; it does not make that build an exact replay of the queried release. Cargo build duration is not bounded by the 15-second metadata timer (rust:crates/ice-cli/src/updates.rs:819-1050).

Source-proven staged-selection defect: run_self_update calls pending_staged_version, whose staged_pending_version helper scans eligible real staging directories, validates executable --version and selects an acceptable highest staged version. With !force, current C, staged S and reported latest L, S>=L causes staged_wins to select S. However the 'already staged' return is nested inside !is_newer(C, S). When C<S and S>=L, the function continues into preparation/activation instead of returning, so an already staged newer executable does not prevent duplicate work. Availability of a valid staged executable and successful validation are prerequisites; the audit did not create staging state or execute this branch (rust:crates/ice-cli/src/updates.rs:607-641; rust:crates/ice-cli/src/updates.rs:715-749).

The selected acquisition then goes through direct activation or the Windows detached helper. Temporary staging cleanup uses owned real-directory checks; successful Windows scheduling retains staging for the helper. Native comments that imply every newer staged version suppresses duplicate preparation must be corrected against this actual branch.

### 5. Interfaces

Native update [--force|-f] [--build] [--help|-h]; process-wide offline normalization; self-update staging and cached latest metadata; known target plus verified flag; CARGO_HOME and Rust wrapper environment checks. No TS --self/--extensions/--models target handling exists inside this native parser.

### 6. Dependencies

`CLI-042`, `CLI-044`, `CLI-045`, `CLI-046`

Native cached metadata, executable validation/activation, prebuilt acquisition and Windows scheduling are prerequisites. TS command routing/explicit target sequencing is CLI-034/038 and is not counted again here.

### 7. ice-rust today

The isolated command cwd, CARGO_HOME validation, executable staging and platform activation are present. The concrete gaps are the already-staged branch mismatch and an explicit target-version difference from TS: Cargo build installs the registry-selected current crate rather than passing the looked-up version. Fresh-cache fallback is visibly allowed for prebuilt target selection. All implementation and inline test bodies were read without running an updater. Native comparison source: `rust:crates/ice-cli/src/updates.rs:522-1090`; `rust:crates/ice-cli/src/updates.rs:2408-3531`.

### 8. Port notes

Preserve isolated build context and native staging. Move/express the already-staged decision so a validated newer/equal-to-target pending update can suppress redundant preparation according to the intended policy, with force behavior explicit. Decide whether Cargo builds must use an exact checked version; if so pass it deliberately and handle forced/no-metadata builds separately. Keep cached-unverified metadata labeled as such and rely on verified artifact identity for acquisition. Estimated work covers those selection corrections and target integration, not rewriting activation. Future branch tests should cover C<S>=L, C=S, stale/invalid staging, force/build combinations and a looked-up version differing from Cargo's chosen version.

### 9. Size and priority

Estimated Rust implementation: **55-110 lines** (estimate, not measured source). **P1** - Update selection should avoid redundant staged work and make the actual installed target predictable.

## CLI-044 - Native executable activation safeguards and the post-collection output-cap defect

### 1. Title and status

Native executable activation safeguards and the post-collection output-cap defect. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/package-manager-cli.ts:521-543; rust:crates/ice-cli/src/updates.rs:877-895; rust:crates/ice-cli/src/updates.rs:1051-1268; rust:crates/ice-cli/src/updates.rs:1764-1989; rust:crates/ice-cli/src/updates.rs:2408-3531.

### 3. What it does

Rust already verifies staged executable identity and performs guarded replacement with rollback. Its executable probe timeout is real, but the advertised output limit does not bound memory while the child is running.

### 4. How it works

TS self-update delegates activation to an external package manager. Native activation validates both staged and destination files, replaces the executable through adjacent files and attempts rollback if post-replacement validation fails.

The direct plan requires a real nonempty regular current executable and owned real staging directory, creates unique adjacent new/old paths, and preflights replacement permissions with create_new probes. It validates staged --version, copies the staged executable, verifies byte equality/version, creates a hard-link backup of the current executable, checks that the destination has not changed and rejects a downgrade, then renames the replacement. Post-activation version/byte checks can trigger rollback through the backup; rollback failure is included in the returned error. Cleanup is best-effort. These are concrete preservation requirements, not a claim of crash-atomic multi-file state or universal platform support (rust:crates/ice-cli/src/updates.rs:1051-1268).

validate_ice_executable_version invokes the candidate with --version, requires successful exit and no stderr, and accepts one exact 'icers <semver>' line with at most its allowed terminal newline. It rejects extra lines/whitespace in the version and disallowed older versions. Downloaded release validation additionally compares the declared expected version. File validation rejects symlink leaf files/directories and checks canonical paths; it does not categorically reject every symlink in an ancestor path (rust:crates/ice-cli/src/updates.rs:877-895; rust:crates/ice-cli/src/updates.rs:1879-1989).

Source-proven bound defect: run_bounded_process_blocking launches a Tokio child with kill_on_drop and wraps wait_with_output in a 10-second timeout. Only after wait_with_output completes does cap_bytes truncate stdout and stderr to 4096 bytes each. Therefore the timeout limits waiting, and returned buffers are capped, but stdout/stderr can grow without that byte bound during collection. It is incorrect to call this a streaming memory cap. A child killed on timeout is not evidence of complete descendant-process cleanup (rust:crates/ice-cli/src/updates.rs:1785-1840).

The inline tests include malformed version output, file/path checks, byte mismatch, replacement failure and rollback fixtures. They were read only. No executable probe, replacement, crash simulation or platform-specific updater was run.

### 5. Interfaces

Staged/current executable paths, --version protocol, 10-second cooperative child timeout, 4096-byte retained stdout/stderr caps, unique adjacent replacement/backup files, exact expected release version for downloads. Activation returns a version or contextual failure including rollback outcome.

### 6. Dependencies

This native validation primitive is used by self-update source selection, direct activation, prebuilt release validation and Windows staging. It does not depend on the TS package-manager command executor.

### 7. ice-rust today

The direct replacement, version validation, leaf symlink rejection and rollback logic exist. The missing bound is during pipe ingestion: cap_bytes is called after full child output collection. Native safeguards are stronger than simply running an npm command, but that does not justify overstating the output limit or cleanup guarantee. Native comparison source: `rust:crates/ice-cli/src/updates.rs:877-895`; `rust:crates/ice-cli/src/updates.rs:1051-1268`; `rust:crates/ice-cli/src/updates.rs:1764-1989`; `rust:crates/ice-cli/src/updates.rs:2408-3531`.

### 8. Port notes

Keep the native activation/rollback protocol. Replace wait_with_output's unbounded collection with concurrent bounded stdout/stderr readers and an explicit overflow policy, while preserving timeout, child reaping and exact version parsing. Reject overflow rather than accepting a silently truncated valid prefix as full protocol evidence. Define descendant cleanup separately if required. Future checks should cover output exceeding the cap before exit, independent stderr overflow, timeout while pipes remain open, valid first line followed by excess output, changed destination and failed rollback.

### 9. Size and priority

Estimated Rust implementation: **80-145 lines** (estimate, not measured source). **P1** - A staged binary can consume unbounded probe memory before the current post-collection cap; activation must validate a complete bounded response.

## CLI-045 - Native prebuilt CLI release verification and corrected repository identity

### 1. Title and status

Native prebuilt CLI release verification and corrected repository identity. **Status: different-by-design.**

### 2. ice source

ice:packages/coding-agent/src/package-manager-cli.ts:493-543; rust:crates/ice-cli/src/prebuilt_release.rs:1-373; rust:crates/ice-cli/src/prebuilt_extensions.rs:158-195; rust:crates/ice-cli/src/prebuilt_extensions.rs:425-803.

### 3. What it does

Rust's default self-update path downloads and verifies a platform CLI executable. The active checkout corrects the repository-case rejection recorded as RUST-045; preserve the native verified-release backend while adding any TypeScript update-selection compatibility.

### 4. How it works

TS self-update uses a checked npm package/version. Native prebuilt self-update instead fetches an ice-release.json manifest for the exact CLI release and verifies its archive and executable before activation.

The native manifest requires schema version 1, package name ice-cli, the requested semver, v<version> tag, the fixed official repository, a 40-character lower-case source commit, exact supported target and a flat platform binary name (icers or icers.exe). The archive must be zip/tar.gz with bounded positive size, matching SHA-256 and a release asset URL on the same repository/tag. It reuses the limited-body, safe extraction, hash and object-format validation helpers from prebuilt_extensions. Manifest fetch uses 15 seconds and archive fetch uses 180 seconds. The staged binary is create_new, synced and made executable on Unix, then its --version must match the requested version before activation (rust:crates/ice-cli/src/prebuilt_release.rs:1-189).

The active REPOSITORY constant is the canonical lowercase https://github.com/zyk-void/ice-rust. It now agrees with canonical_repository at both manifest identity and release-asset URL validation boundaries, correcting the old checkout's source-proven case mismatch. The changelog URL uses the same lowercase owner. RUST-045 preserves the old finding and local-rebrand provenance with active_status=fixed (rust:crates/ice-cli/src/prebuilt_release.rs:11; rust:crates/ice-cli/src/prebuilt_release.rs:50; rust:crates/ice-cli/src/prebuilt_release.rs:92; rust:crates/ice-cli/src/prebuilt_extensions.rs:158-195; rust:crates/ice-cli/src/changelog.rs:27).

The positive fixture's repository now uses the corrected constant; the flat executable fixture follows icers.exe. The ignored real-network smoke test was not run. Source verification of these corrections does not establish that remote assets exist or that a platform download/activation succeeds.

Keep the exact manifest, hash, path, archive and executable checks. The default CLI update still has no automatic Cargo fallback when the prebuilt protocol fails; users can separately select the build route. The manifest filename ice-release.json and crate name ice-cli remain unchanged protocol identities.

### 5. Interfaces

GitHub release v<version>/ice-release.json; canonical official repository https://github.com/zyk-void/ice-rust; schema 1 CLI release manifest; exact version/target/binary/archive/hash fields; flat icers or icers.exe executable; --build as explicit alternative acquisition. Remote asset availability remains unverified.

### 6. Dependencies

`CLI-041`, `CLI-044`

CLI-041 owns shared canonical repository/archive/object helpers; CLI-044 owns staged executable --version validation. CLI-043 selects this acquisition path.

### 7. ice-rust today

The complete protocol and inline tests are present. The active native source corrects the old mixed-case repository constant consistently for manifest and release-asset validation, so that specific rejection is fixed. The native prebuilt protocol remains a deliberate backend difference from TypeScript package-manager activation. Source verification was performed without a download, application command or runtime test. Native comparison source: rust:crates/ice-cli/src/prebuilt_release.rs:1-373; rust:crates/ice-cli/src/prebuilt_extensions.rs:158-195; rust:crates/ice-cli/src/prebuilt_extensions.rs:425-803; rust:crates/ice-cli/src/changelog.rs:27.

### 8. Port notes

Preserve the canonical repository, publisher/tag/path constraints and positive/negative manifest fixtures when extending update command compatibility. No additional repository-case correction remains in the active checkout. Future implementation verification should exercise the retained fixtures and platform path without loosening identity checks or adding silent unverified fallback. Related acquisition, activation and command-selection work is estimated in CLI-038 and CLI-041–044.

### 9. Size and priority

Estimated Rust implementation: **0-0 lines** (estimate, not measured source). **P2** - Preserve the verified native release boundary during compatibility work; the old default-update rejection is already fixed.

## CLI-046 - Windows detached native activation, status consumption and timestamp-schema mismatch

### 1. Title and status

Windows detached native activation, status consumption and timestamp-schema mismatch. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/package-manager-cli.ts:521-562; rust:crates/ice-cli/src/updates.rs:99-119; rust:crates/ice-cli/src/updates.rs:1297-1783; rust:crates/ice-cli/src/updates.rs:1990-2407; rust:crates/ice-cli/src/updates.rs:2408-3531.

### 3. What it does

Native Windows update schedules a detached helper that waits for the current executable to exit and then replaces it with rollback support. Helper-written status timestamps use a different schema from the Rust consumer, so age-based cleanup does not work for those statuses.

### 4. How it works

TS Windows npm self-update quarantines loaded dependency files before invoking its package manager. Rust has a separate executable activation helper that retains staging until the parent process exits.

The Rust path stages and validates the new executable, builds replacement/backup/status paths and writes preparing/scheduled status. It locates PowerShell through GetWindowsDirectoryW and the fixed System32 WindowsPowerShell executable, validates its MZ file header and never searches PATH. The helper command uses structured arguments with no profile, noninteractive mode, a system-only PSModulePath, null stdio and CREATE_NO_WINDOW. Successful spawn returns while the helper waits, retaining the staging directory and printing its status path (rust:crates/ice-cli/src/updates.rs:1551-1783; rust:crates/ice-cli/src/updates.rs:2288-2407).

The embedded helper validates paths, hashes and PE marker, verifies that the given parent PID belongs to the expected target executable, and waits up to 600000 ms. After the parent exits it rehashes staged content, copies/verifies it and uses File.Replace with backup. Post-replacement hash failure invokes the backup/displaced-file recovery paths and records failure including rollback details. Success/failure cleanup removes owned staged files and empty directories best-effort. It does not claim that returning from the Rust command means activation already happened (rust:crates/ice-cli/src/updates.rs:1297-1548).

Status consumption accepts bounded regular status files under the owned update root, with 32-hex status names and a 64 KiB read bound. Messages have control/whitespace normalized and are bounded to the implementation's 2048-character accumulation target. Failed statuses become warnings and are consumed independently of age; success statuses normally expire after 24 hours, nonterminal statuses after one hour, but age checks require updatedAtMs>0. Separate pruning handles eligible unclaimed pending staging directories: empty directories can be removed immediately, while payload/age/obsolete rules protect live work. Unknown-age status data is not assumed stale (rust:crates/ice-cli/src/updates.rs:1990-2287).

Source-proven schema defect: Rust SelfUpdateStatus deserializes camel-case updatedAtMs as a numeric value with default zero, and write_windows_update_status emits that field. The embedded PowerShell Write-UpdateStatus emits ISO-string updatedAt instead and replaces the status file. Helper waiting/succeeded statuses therefore read back with updatedAtMs=0, so the one-hour/24-hour TTL branches do not age them out as intended. Failed-status handling and independent orphan/empty staging cleanup can still run. Inline TTL fixtures manually supply updatedAtMs, so they do not establish the real helper output schema works (rust:crates/ice-cli/src/updates.rs:99-119; rust:crates/ice-cli/src/updates.rs:1312-1332; rust:crates/ice-cli/src/updates.rs:2123-2159; rust:crates/ice-cli/src/updates.rs:2395-2407).

All helper text and inline Windows/status tests were read without executing PowerShell, creating status files or replacing an executable. The TS loaded-DLL quarantine algorithm is MISC-002 and remains a separate runtime-specific mechanism.

### 5. Interfaces

Detached helper arguments include parent PID, staged/target/replacement/backup/status paths and expected hashes; status fields state, message, updatedAtMs (Rust) versus updatedAt (current helper). Limits: parent wait 600000 ms, status 64 KiB, nonterminal TTL one hour, success TTL 24 hours. Success scheduling and completed activation are distinct states.

### 6. Dependencies

`CLI-044`

CLI-044 supplies staged executable/path/version validation. MISC-002 describes TS dependency quarantine and is only a comparison, not an implementation prerequisite for native executable replacement.

### 7. ice-rust today

The detached helper, fixed PowerShell path, verified parent wait, hash checks, replacement/rollback and bounded status reads exist. The concrete missing compatibility is internal writer/reader timestamp agreement, not the whole Windows updater. The native helper's ISO updatedAt is ignored by the Rust updatedAtMs consumer; no OS execution was performed to claim end-to-end success. Native comparison source: `rust:crates/ice-cli/src/updates.rs:99-119`; `rust:crates/ice-cli/src/updates.rs:1297-1783`; `rust:crates/ice-cli/src/updates.rs:1990-2407`; `rust:crates/ice-cli/src/updates.rs:2408-3531`.

### 8. Port notes

Keep the guarded detached activation workflow. Make helper and Rust writers share one timestamp field/type, preferably epoch milliseconds, and decide how to read already-written ISO statuses without treating unknown times as immediately expired. Exercise the actual helper-generated schema in future source/fixture verification rather than only hand-authored Rust records. Preserve failed warning consumption and staging ownership checks, and report scheduled versus activated state accurately. Estimated changes cover schema/cleanup correction, not replacement of the Windows activation mechanism.

### 9. Size and priority

Estimated Rust implementation: **45-90 lines** (estimate, not measured source). **P1** - Real helper statuses bypass intended age cleanup because their timestamp field is not read, leaving stale pending/success state.

