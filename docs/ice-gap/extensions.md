# Extension loader, host API, lifecycle and native ABI gaps

This area covers all eight original extensions ledger files, their public contracts and behavior-specifying tests, with independent native SDK, loader, registry, event, action and UI counterpart comparison. Status describes behavioral parity at the frozen source identities, not the presence of an enum or method name. TypeScript source behavior, test expectations, native design differences and source-only defect findings are kept explicit; no application tests were run.

## EXT-001 - TypeScript factory execution versus the native ABI boundary

### 1. Title and status

TypeScript factory execution versus the native ABI boundary. **Status: different-by-design.**

### 2. ice source

ice:packages/coding-agent/src/core/extensions/loader.ts:572-675; ice:packages/coding-agent/src/core/extensions/loader.ts:735-787. loadExtensionModule/loadExtension/loadExtensionFromFactory/createExtension/loadExtensions. Tests: ice:packages/coding-agent/test/extensions-discovery.test.ts:339-347 (`reports errors for invalid extension code`); ice:packages/coding-agent/test/extensions-discovery.test.ts:414-427 (`reports error when extension has no default export`).

### 3. What it does

Load an in-process callable factory and retain its tools, handlers and UI registrations.

### 4. How it works

Extensions execute trusted TypeScript factories that register capabilities against a fresh instance. A failed file becomes a load diagnostic while later files can still load.

Resolve each file against cwd with Unicode-space normalization, create Jiti and import its default. A nonfunction returns an invalid-factory error; import/factory exceptions become per-path errors and later paths continue. Path resolution itself occurs before the catch. Create fresh maps and synthetic SourceInfo: angle-bracket identity uses its first colon-separated token or temporary as source; other paths use local plus resolved parent directory. Await factory initialization, then retain the instance. Inline loading uses <inline> by default, skips import/console-capture wrapping, resolves cwd and propagates initialization rejection. Timings are recorded after import/factory.

This loader is not a sandbox or project-trust gate. Module initialization precedes factory validation; ice.exec and arbitrary imported code can perform host I/O. The resource/startup layer must admit executable resources before calling it. Every required TS extension needs a native reimplementation or a separately authorized JS runtime. No plugin was loaded during this audit.

### 5. Interfaces

```ts
function createExtension(extensionPath: string, resolvedPath: string): Extension {
	const source =
		extensionPath.startsWith("<") && extensionPath.endsWith(">")
			? extensionPath.slice(1, -1).split(":")[0] || "temporary"
			: "local";
	const baseDir = extensionPath.startsWith("<") ? undefined : path.dirname(resolvedPath);

	return {
		path: extensionPath,
		resolvedPath,
		sourceInfo: createSyntheticSourceInfo(extensionPath, { source, baseDir }),
		handlers: new Map(),
		tools: new Map(),
		messageRenderers: new Map(),
		entryRenderers: new Map(),
		commands: new Map(),
		settings: new Map(),
		flags: new Map(),
		shortcuts: new Map(),
	};
}

async function loadExtension(
	extensionPath: string,
	cwd: string,
	eventBus: EventBus,
	runtime: ExtensionRuntime,
	cacheToken?: ExtensionCacheToken,
): Promise<{ extension: Extension | null; error: string | null }> {
	const resolvedPath = resolvePath(extensionPath, cwd, { normalizeUnicodeSpaces: true });

	try {
		const factory = await loadExtensionModule(resolvedPath, cacheToken);
		time(`${extensionPath} module import`, "extensions");
		if (!factory) {
			return { extension: null, error: `Extension does not export a valid factory function: ${extensionPath}` };
		}

		const extension = createExtension(extensionPath, resolvedPath);
		await initializeExtension(factory, extension, runtime, cwd, eventBus);
		time(`${extensionPath} factory`, "extensions");

		return { extension, error: null };
	} catch (err) {
		const message = err instanceof Error ? err.message : String(err);
		return { extension: null, error: `Failed to load extension: ${message}` };
	}
}

/**
 * Create an Extension from an inline factory function.
 */
export async function loadExtensionFromFactory(
	factory: ExtensionFactory,
	cwd: string,
	eventBus: EventBus,
	runtime: ExtensionRuntime,
	extensionPath = "<inline>",
): Promise<Extension> {
	const extension = createExtension(extensionPath, extensionPath);
	const resolvedCwd = resolvePath(cwd);
	await initializeExtension(factory, extension, runtime, resolvedCwd, eventBus);
	time(`${extensionPath} factory`, "extensions");
	return extension;
```
Source: ice:packages/coding-agent/src/core/extensions/loader.ts:612-675.
None: additional fixed CLI flags, slash commands, keybindings, settings, environment variables, RPC commands or persistent files beyond the explicitly listed registration/host contracts. Each dynamically registered name is chosen by the extension.

### 6. Dependencies

`EXT-002`, `EXT-003`, `EXT-004`, `EXT-006`

External services/binaries: none required by this host mechanism. Plugins may invoke arbitrary host I/O because they are trusted executable code; this audit did not load or run any plugin.

### 7. ice-rust today

rust:crates/ice-extensions/src/loader.rs:74-214. Rust loads native libraries, calls ice_plugin_register with ABI v4 and retains library/action handles. The host supplies a version; load_one does not independently query a plugin ABI before invoking it. Native library initializers can also run before symbol validation. A nonzero registration result discards that plugin registry and releases its stdio claim. This is deliberately not TS import execution.

### 8. Port notes

Keep native cdylib loading in ice-extensions and SDK ABI v4 as the chosen architecture. Port behavior into native plugins with version/size-checked registration and producer-owned buffers; a JS runtime would be a separate explicit product decision. Preserve libloading keepalives through every callback, and surface path-specific load failures through ice-cli diagnostics.

### 9. Size and priority

Estimated Rust implementation: **100-180 lines** (estimate, not measured source). **P0** - ICE-specific TS factories otherwise cannot run; execution and trust admission must not be confused.

## EXT-002 - Exact historical module aliases and runtime-specific import resolution

### 1. Title and status

Exact historical module aliases and runtime-specific import resolution. **Status: different-by-design.**

### 2. ice source

ice:packages/coding-agent/src/core/extensions/loader.ts:54-168; ice:packages/coding-agent/src/core/extensions/loader.ts:580-596; ice:packages/coding-agent/src/core/legacy-compat/extension-aliases.ts:1-36. VIRTUAL_MODULES/applyLegacyPiAliases/getAliases/getExtensionAliasMaps. Tests: ice:packages/coding-agent/test/legacy-compat/extension-alias-parity.test.ts:30-33 (`pins the policy table to the approved inventory and targets`); ice:packages/coding-agent/test/legacy-compat/extension-alias-parity.test.ts:35-44 (`applies every approved alias through the exact virtual-module policy`); ice:packages/coding-agent/test/legacy-compat/extension-alias-parity.test.ts:58-64 (`leaves unapproved Pi-like specifiers unresolved in both runtimes`).

### 3. What it does

Support only the approved legacy Pi package specifiers while preserving host module identity.

### 4. How it works

Approved historical Pi imports resolve to the same ICE module objects as their canonical names. Unapproved Pi-like subpaths remain unresolved.

Expose TypeBox root/compile/value, matching @sinclair aliases, ICE agent-core/TUI/coding-agent and ICE AI root/compat/OAuth/providers-all as exact virtual modules. AI root intentionally points to the compatibility module object. Overlay the fourteen exact legacy aliases quoted below; a missing canonical target throws. No unsupported subpath rewrite is permitted.

Bun uses virtualModules plus tryNative=false. Source TS uses virtualModules and tsconfigPaths=true. Built Node adds canonical dist-path aliases, preferring existing workspace entries then import.meta.resolve, while TypeBox uses require.resolve. Historical keys are excluded from prefix-based Node aliases. Canonical aliases are process-cached independently of factory cache invalidation. The inspection seam returns readonly-typed references, not deep immutable copies. No package install or on-disk migration occurs.

### 5. Interfaces

```ts
/** Modules available to extensions via virtualModules (for compiled Bun binary) */
const VIRTUAL_MODULES: Record<string, unknown> = {
	typebox: _bundledTypebox,
	"typebox/compile": _bundledTypeboxCompile,
	"typebox/value": _bundledTypeboxValue,
	"@sinclair/typebox": _bundledTypebox,
	"@sinclair/typebox/compile": _bundledTypeboxCompile,
	"@sinclair/typebox/value": _bundledTypeboxValue,
	"@zykairotis/ice-agent-core": _bundledIceAgentCore,
	"@zykairotis/ice-tui": _bundledIceTui,
	// Extensions resolve the ice-ai root to the compat entrypoint (a strict
	// superset of the core entrypoint): existing extensions using the old
	// global API keep working at runtime until compat is removed.
	"@zykairotis/ice-ai": _bundledIceAiCompat,
	"@zykairotis/ice-ai/compat": _bundledIceAiCompat,
	"@zykairotis/ice-ai/oauth": _bundledIceAiOauth,
	"@zykairotis/ice-ai/providers/all": _bundledIceAiProviders,
	"@zykairotis/ice-coding-agent": _bundledIceCodingAgent,
};

// Approved historical Pi specifiers reuse the already-bundled ICE modules
// (no extra bundle weight): each legacy key points at the same object as its
// ICE target.
applyLegacyPiAliases(VIRTUAL_MODULES);

const require = createRequire(import.meta.url);

const isTypeScriptSourceRuntime = !isBunBinary && path.extname(fileURLToPath(import.meta.url)) === ".ts";

/**
 * Overlay approved historical Pi specifiers onto a base map that already
 * defines every `@zykairotis/ice-*` target. Each legacy key reuses the existing
 * target value rather than resolving a separate module, so a target absent
 * from the base map is a development error, not a silent `undefined`.
 */
function applyLegacyPiAliases<V>(base: Record<string, V>): Record<string, V> {
	for (const [legacySpecifier, iceSpecifier] of Object.entries(LEGACY_PI_EXTENSION_ALIASES)) {
		if (!(iceSpecifier in base)) {
			throw new Error(`Unresolved legacy Pi alias target: ${legacySpecifier} -> ${iceSpecifier}`);
		}
		base[legacySpecifier] = base[iceSpecifier];
	}
	return base;
}
```
Source: ice:packages/coding-agent/src/core/extensions/loader.ts:54-97.
```ts
export const LEGACY_PI_EXTENSION_ALIASES = {
	"@earendil-works/pi-coding-agent": "@zykairotis/ice-coding-agent",
	"@earendil-works/pi-agent-core": "@zykairotis/ice-agent-core",
	"@earendil-works/pi-tui": "@zykairotis/ice-tui",
	"@earendil-works/pi-ai": "@zykairotis/ice-ai",
	"@earendil-works/pi-ai/compat": "@zykairotis/ice-ai/compat",
	"@earendil-works/pi-ai/oauth": "@zykairotis/ice-ai/oauth",
	"@earendil-works/pi-ai/providers/all": "@zykairotis/ice-ai/providers/all",
	"@mariozechner/pi-coding-agent": "@zykairotis/ice-coding-agent",
	"@mariozechner/pi-agent-core": "@zykairotis/ice-agent-core",
	"@mariozechner/pi-tui": "@zykairotis/ice-tui",
	"@mariozechner/pi-ai": "@zykairotis/ice-ai",
	"@mariozechner/pi-ai/compat": "@zykairotis/ice-ai/compat",
	"@mariozechner/pi-ai/oauth": "@zykairotis/ice-ai/oauth",
	"@mariozechner/pi-ai/providers/all": "@zykairotis/ice-ai/providers/all",
} as const;

export type LegacyPiSpecifier = keyof typeof LEGACY_PI_EXTENSION_ALIASES;

export type LegacyPiAliasTarget = (typeof LEGACY_PI_EXTENSION_ALIASES)[LegacyPiSpecifier];
```
Source: ice:packages/coding-agent/src/core/legacy-compat/extension-aliases.ts:17-36.
```ts
	const jiti = createJiti(import.meta.url, {
		moduleCache: false,
		// Bun uses modules embedded in the executable. Source TypeScript reuses the
		// host-resolved modules and root tsconfig paths. Built Node combines exact
		// virtual modules for extensions with canonical dist-path aliases.
		...(isBunBinary
			? { virtualModules: VIRTUAL_MODULES, tryNative: false }
			: isTypeScriptSourceRuntime
				? { virtualModules: VIRTUAL_MODULES, tsconfigPaths: true }
				: {
						// Jiti aliases are prefix-based. Keep canonical built-path aliases there,
						// but expose historical Pi compatibility through exact virtual-module
						// keys so unsupported legacy subpaths cannot be rewritten accidentally.
						virtualModules: VIRTUAL_MODULES,
						alias: getAliases(),
					}),
	});
```
Source: ice:packages/coding-agent/src/core/extensions/loader.ts:580-596.
None: additional fixed CLI flags, slash commands, keybindings, settings, environment variables, RPC commands or persistent files beyond the explicitly listed registration/host contracts. Each dynamically registered name is chosen by the extension.

### 6. Dependencies

`EXT-001`

External services/binaries: none required by this host mechanism. Plugins may invoke arbitrary host I/O because they are trusted executable code; this audit did not load or run any plugin.

### 7. ice-rust today

rust:crates/ice-extensions/src/loader.rs:119-141. Native dynamic loading has no JavaScript module-alias counterpart. Document TS packages as unsupported even where their native feature replacement exists. Native ABI symbol compatibility is a separate packaging contract.

### 8. Port notes

Port required features as native plugins; no alias table can make TS modules callable through C ABI. A JS runtime would be a separate explicit architecture decision.

### 9. Size and priority

Estimated Rust implementation: **0-80 lines** (estimate, not measured source). **P2** - Prevents accidental claims of TS ecosystem compatibility.

## EXT-003 - Cwd/generation factory cache with fresh instance initialization

### 1. Title and status

Cwd/generation factory cache with fresh instance initialization. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/extensions/loader.ts:172-194; ice:packages/coding-agent/src/core/extensions/loader.ts:564-607; ice:packages/coding-agent/src/core/extensions/loader.ts:735-787. clearExtensionCache/loadExtensionsCached/useExtensionCacheCwd. Tests: ice:packages/coding-agent/test/suite/regressions/extension-factory-cache.test.ts:69-81 (`caches extension modules for cached same-cwd loads but reruns factories`); ice:packages/coding-agent/test/suite/regressions/extension-factory-cache.test.ts:83-93 (`does not cache direct loadExtensions calls`); ice:packages/coding-agent/test/suite/regressions/extension-factory-cache.test.ts:95-113 (`clears the cache on resource loader reload`); ice:packages/coding-agent/test/suite/regressions/extension-factory-cache.test.ts:115-130 (`keeps the cache scoped to one cwd`).

### 3. What it does

Reuse a module factory in cached loads while rerunning initialization for every extension instance.

### 4. How it works

Cached loads reuse imported factory functions for one cwd while rerunning initialization for each session instance. A generation token prevents an obsolete import from repopulating a cleared cache.

Maintain one process-global cwd, generation and Map of resolved path to factory. Selecting a different resolved cwd clears the map, clears cwd and increments generation; explicit clear does the same. Capture cwd/generation in a token, and check both before reuse and before insertion so a late import cannot repopulate a newer generation. Ordinary loadExtensions passes no token and neither reads nor writes this cache. Jiti always has moduleCache=false. Cache callable defaults only, before factory initialization; a throwing factory can therefore remain cached.

There is no in-flight import deduplication, mtime/hash validation, eviction cap or multi-cwd cache. Every load creates fresh maps/runtime when not supplied and reruns the factory. Resource-loader reload explicitly clears the cache as tested. Do not cache mutable extension instances or equate OS shared-library caching with this lifecycle.

### 5. Interfaces

```ts
let extensionCacheCwd: string | undefined;
let extensionCacheGeneration = 0;
const extensionCache = new Map<string, ExtensionFactory>();

interface ExtensionCacheToken {
	cwd: string;
	generation: number;
}

export function clearExtensionCache(): void {
	extensionCache.clear();
	extensionCacheCwd = undefined;
	extensionCacheGeneration++;
}

function useExtensionCacheCwd(cwd: string): ExtensionCacheToken {
	const resolvedCwd = resolvePath(cwd);
	if (extensionCacheCwd !== undefined && extensionCacheCwd !== resolvedCwd) {
		clearExtensionCache();
	}
	extensionCacheCwd = resolvedCwd;
	return { cwd: resolvedCwd, generation: extensionCacheGeneration };
}
```
Source: ice:packages/coding-agent/src/core/extensions/loader.ts:172-194.
```ts
function isCurrentCacheToken(cacheToken: ExtensionCacheToken | undefined): cacheToken is ExtensionCacheToken {
	return (
		cacheToken !== undefined &&
		extensionCacheCwd === cacheToken.cwd &&
		extensionCacheGeneration === cacheToken.generation
	);
}

async function loadExtensionModule(extensionPath: string, cacheToken?: ExtensionCacheToken) {
	if (isCurrentCacheToken(cacheToken)) {
		const cachedFactory = extensionCache.get(extensionPath);
		if (cachedFactory) {
			return cachedFactory;
		}
	}

	const jiti = createJiti(import.meta.url, {
		moduleCache: false,
		// Bun uses modules embedded in the executable. Source TypeScript reuses the
		// host-resolved modules and root tsconfig paths. Built Node combines exact
		// virtual modules for extensions with canonical dist-path aliases.
		...(isBunBinary
			? { virtualModules: VIRTUAL_MODULES, tryNative: false }
			: isTypeScriptSourceRuntime
				? { virtualModules: VIRTUAL_MODULES, tsconfigPaths: true }
				: {
						// Jiti aliases are prefix-based. Keep canonical built-path aliases there,
						// but expose historical Pi compatibility through exact virtual-module
						// keys so unsupported legacy subpaths cannot be rewritten accidentally.
						virtualModules: VIRTUAL_MODULES,
						alias: getAliases(),
					}),
	});

	const module = await jiti.import(extensionPath, { default: true });
	const factory = module as ExtensionFactory;
	if (typeof factory !== "function") {
		return undefined;
	}
	if (isCurrentCacheToken(cacheToken)) {
		extensionCache.set(extensionPath, factory);
	}
	return factory;
}
```
Source: ice:packages/coding-agent/src/core/extensions/loader.ts:564-607.
None: additional fixed CLI flags, slash commands, keybindings, settings, environment variables, RPC commands or persistent files beyond the explicitly listed registration/host contracts. Each dynamically registered name is chosen by the extension.

### 6. Dependencies

`EXT-001`, `EXT-006`, `EXT-010`

External services/binaries: none required by this host mechanism. Plugins may invoke arbitrary host I/O because they are trusted executable code; this audit did not load or run any plugin.

### 7. ice-rust today

rust:crates/ice-extensions/src/loader.rs:119-214; rust:crates/ice-extensions/src/loader.rs:486-538. Rust creates fresh registries/session keepalives but has no equivalent generation-fenced TS factory cache. Preserve per-session native ownership; any native reload cache needs explicit invalidation rather than a shared mutable global registry.

### 8. Port notes

If native module reuse is needed, add a loader cache keyed by canonical binary identity plus reload generation, retaining an Arc<Library> while creating a fresh registry and scoped ActionBridge for each instance. Do not cache mutable registrations or confuse OS dynamic-loader reuse with rerunning TS factories. Verify generation changes, cross-cwd loads and failed initialization before enabling reuse.

### 9. Size and priority

Estimated Rust implementation: **70-130 lines** (estimate, not measured source). **P2** - Required for faithful native extension behavior and for ICE-specific plugins that depend on it.

## EXT-004 - Ordered discovery and lexical path deduplication

### 1. Title and status

Ordered discovery and lexical path deduplication. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/extensions/loader.ts:789-929; ice:packages/coding-agent/src/utils/readdir-order.ts:1-20. discoverExtensionsInDir/discoverAndLoadExtensions. Tests: ice:packages/coding-agent/test/extensions-discovery.test.ts:43-52 (`discovers direct .ts files in extensions/`); ice:packages/coding-agent/test/extensions-discovery.test.ts:100-111 (`discovers subdirectory with index.ts`); ice:packages/coding-agent/test/extensions-discovery.test.ts:264-276 (`does not recurse beyond one level`); ice:packages/coding-agent/test/extensions-discovery.test.ts:349-359 (`handles explicitly configured paths`).

### 3. What it does

Discover project/global/configured extensions without recursively importing arbitrary descendants.

### 4. How it works

Discovery loads project, global, and configured extensions in a defined precedence sequence. Lexical deduplication preserves the first occurrence without collapsing filesystem aliases.

Resolve cwd/agentDir; add project getProjectConfigDir(cwd)/extensions first, global agentDir/extensions second, explicit configured paths last. Deduplicate path.resolve strings, not realpaths, keeping the first display path. A directory scan includes direct files/symlinks ending lowercase .ts/.js, or directory/symlink children with manifest/index entries. No hidden-name filter. Bun sorts directory entries by UTF-8 bytes; Node uses its native enumeration order. Any exception inside a scan discards all findings from that scan and returns[].

An explicit existing directory resolves its own manifest/index first; otherwise scan its direct children. Explicit files/nonexistent paths are retained for later import errors. Explicit stat errors are outside the scan catch. No recursion beyond one discovered child directory, canonical-target deduplication, maximum count or trust gate is supplied. Lexical symlink aliases can load the same module twice. This convenience function’s precedence is not proof of every higher-level resource-loader ordering.

### 5. Interfaces

```ts
function discoverExtensionsInDir(dir: string): string[] {
	if (!fs.existsSync(dir)) {
		return [];
	}

	const discovered: string[] = [];

	try {
		const entries = inNodeReaddirOrder(fs.readdirSync(dir, { withFileTypes: true }));

		for (const entry of entries) {
			const entryPath = path.join(dir, entry.name);

			// 1. Direct files: *.ts or *.js
			if ((entry.isFile() || entry.isSymbolicLink()) && isExtensionFile(entry.name)) {
				discovered.push(entryPath);
				continue;
			}

			// 2 & 3. Subdirectories
			if (entry.isDirectory() || entry.isSymbolicLink()) {
				const entries = resolveExtensionEntries(entryPath);
				if (entries) {
					discovered.push(...entries);
				}
			}
		}
	} catch {
		return [];
	}

	return discovered;
```
Source: ice:packages/coding-agent/src/core/extensions/loader.ts:844-875.
```ts
export async function discoverAndLoadExtensions(
	configuredPaths: string[],
	cwd: string,
	agentDir: string = getAgentDir(),
	eventBus?: EventBus,
): Promise<LoadExtensionsResult> {
	const resolvedCwd = resolvePath(cwd);
	const resolvedAgentDir = resolvePath(agentDir);
	const allPaths: string[] = [];
	const seen = new Set<string>();

	const addPaths = (paths: string[]) => {
		for (const p of paths) {
			const resolved = path.resolve(p);
			if (!seen.has(resolved)) {
				seen.add(resolved);
				allPaths.push(p);
			}
		}
	};

	// 1. Project-local extensions: cwd/${CONFIG_DIR_NAME}/extensions/
	const localExtDir = path.join(getProjectConfigDir(resolvedCwd), "extensions");
	addPaths(discoverExtensionsInDir(localExtDir));

	// 2. Global extensions: agentDir/extensions/
	const globalExtDir = path.join(resolvedAgentDir, "extensions");
	addPaths(discoverExtensionsInDir(globalExtDir));

	// 3. Explicitly configured paths
	for (const p of configuredPaths) {
		const resolved = resolvePath(p, resolvedCwd, { normalizeUnicodeSpaces: true });
		if (fs.existsSync(resolved) && fs.statSync(resolved).isDirectory()) {
			// Check for package.json with ice manifest or index.ts
			const entries = resolveExtensionEntries(resolved);
			if (entries) {
				addPaths(entries);
				continue;
			}
			// No explicit entries - discover individual files in directory
			addPaths(discoverExtensionsInDir(resolved));
			continue;
		}

		addPaths([resolved]);
	}

	return loadExtensions(allPaths, resolvedCwd, eventBus);
}
```
Source: ice:packages/coding-agent/src/core/extensions/loader.ts:881-929.
```ts
const isBun = typeof process.versions.bun === "string";

function entryName(entry: string | { name: string }): string {
	return typeof entry === "string" ? entry : entry.name;
}

/** UTF-8 byte order of entry names, matching libuv's alphasort under the C locale. */
export function compareReaddirEntries(a: string | { name: string }, b: string | { name: string }): number {
	return Buffer.compare(Buffer.from(entryName(a)), Buffer.from(entryName(b)));
}

export function inNodeReaddirOrder<T extends string | { name: string }>(entries: T[]): T[] {
	return isBun ? entries.sort(compareReaddirEntries) : entries;
}
```
Source: ice:packages/coding-agent/src/utils/readdir-order.ts:7-20.
None: additional fixed CLI flags, slash commands, keybindings, settings, environment variables, RPC commands or persistent files beyond the explicitly listed registration/host contracts. Each dynamically registered name is chosen by the extension.

### 6. Dependencies

`EXT-001`, `EXT-005`

External services/binaries: none required by this host mechanism. Plugins may invoke arbitrary host I/O because they are trusted executable code; this audit did not load or run any plugin.

### 7. ice-rust today

rust:crates/ice-extensions/src/loader.rs:220-270; rust:crates/ice-extensions/src/loader.rs:486-538. Rust scans unsorted read_dir entries with case-insensitive native suffixes and no file-kind check, then explicit files. It canonicalizes identities for deduplication. That stronger dedup should be retained deliberately; deterministic collision order still requires sorting native discovery.

### 8. Port notes

Specify native discovery order in ice-cli resource_dirs and ice-extensions loader, then sort directory entries before registration so first-wins collisions are reproducible. Keep canonical deduplication as an explicit native difference; expose both display path and identity for diagnostics. Package expansion and explicit-file precedence must be resolved before loading, with trust applied by the resource owner.

### 9. Size and priority

Estimated Rust implementation: **90-160 lines** (estimate, not measured source). **P1** - Required for faithful native extension behavior and for ICE-specific plugins that depend on it.

## EXT-005 - ICE-over-Pi resource manifest and entrypoint fallback

### 1. Title and status

ICE-over-Pi resource manifest and entrypoint fallback. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/ice-manifest.ts:1-61; ice:packages/coding-agent/src/core/extensions/loader.ts:802-832. readCompatibleManifest/readIceManifest/resolveExtensionEntries. Tests: ice:packages/coding-agent/test/extensions-discovery.test.ts:138-160 (`discovers subdirectory with package.json ice field`); ice:packages/coding-agent/test/extensions-discovery.test.ts:162-185 (`keeps package.json ice extension entries with leading tilde package-relative`); ice:packages/coding-agent/test/extensions-discovery.test.ts:299-317 (`skips non-existent paths declared in package.json`); ice:packages/coding-agent/test/extensions-discovery.test.ts:233-250 (`ignores package.json without ice field, falls back to index.ts`).

### 3. What it does

Read package resource declarations without merging native and legacy blocks.

### 4. How it works

A package can declare extension, skill, prompt, and theme resources in its ICE manifest. Presence of the ICE block takes precedence over the legacy Pi block, even when the ICE block is malformed.

Parse package.json as a nonarray object. Presence of ice selects that block even when invalid; only absence permits pi. A selected nonobject/array returns null. For each of extensions/skills/prompts/themes independently copy an all-string array, including empty arrays; ignore other fields and malformed individual arrays. Catch read/parse errors and return null. readCompatibleManifest retains source ice or legacy-pi; readIceManifest returns only the selected manifest. No path validation or code execution occurs.

For extension discovery, a nonempty extensions array resolves members with path.resolve(packageDir, member), retaining existing targets only. Any survivor suppresses index fallback. With none, try index.ts then index.js, else null. Absolute/parent-relative paths and directories are accepted if they exist. A leading tilde is package-relative, not home expansion. An empty/invalid native block prevents legacy fallback but does not prevent index fallback.

### 5. Interfaces

```ts
export interface IceManifest {
	extensions?: string[];
	skills?: string[];
	prompts?: string[];
	themes?: string[];
}

/** Which manifest field supplied the resources. */
export type CompatibleManifestSource = "ice" | "legacy-pi";

export interface CompatibleManifestResult {
	manifest: IceManifest;
	source: CompatibleManifestSource;
}

const RESOURCE_FIELDS = ["extensions", "skills", "prompts", "themes"] as const;

function isObject(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null && !Array.isArray(value);
}

/**
 * Read a package resource manifest, accepting the legacy `pi` field as a
 * one-way compatibility input.
 *
 * Precedence is strict and never merges: `ice` wins by presence, even when it is
 * malformed or null, so an explicit `ice: null` stays a hard null rather than
 * silently downgrading to a valid `pi` block. The returned `source` lets callers
 * tell an ICE-native package from a legacy one without inspecting raw JSON.
 */
export function readCompatibleManifest(packageJsonPath: string): CompatibleManifestResult | null {
	try {
		const pkg: unknown = JSON.parse(readFileSync(packageJsonPath, "utf-8"));
		if (!isObject(pkg)) {
			return null;
		}
		// `ice` wins when present (even null/invalid: intentional, no fallback).
		// Fall back to the legacy `pi` field only when `ice` is absent entirely.
		const hasIce = "ice" in pkg;
		const resourceManifest = hasIce ? pkg.ice : pkg.pi;
		if (!isObject(resourceManifest)) return null;

		const manifest: IceManifest = {};
		for (const field of RESOURCE_FIELDS) {
			const entries = resourceManifest[field];
			if (Array.isArray(entries) && entries.every((entry) => typeof entry === "string")) {
				manifest[field] = entries;
			}
		}
		return { manifest, source: hasIce ? "ice" : "legacy-pi" };
	} catch {
		return null;
	}
}

/** Compatibility wrapper for callers that do not need provenance. */
export function readIceManifest(packageJsonPath: string): IceManifest | null {
	return readCompatibleManifest(packageJsonPath)?.manifest ?? null;
}
```
Source: ice:packages/coding-agent/src/core/ice-manifest.ts:3-61.
```ts
function resolveExtensionEntries(dir: string): string[] | null {
	// Check for package.json with "ice" field first
	const packageJsonPath = path.join(dir, "package.json");
	if (fs.existsSync(packageJsonPath)) {
		const manifest = readIceManifest(packageJsonPath);
		if (manifest?.extensions?.length) {
			const entries: string[] = [];
			for (const extPath of manifest.extensions) {
				const resolvedExtPath = path.resolve(dir, extPath);
				if (fs.existsSync(resolvedExtPath)) {
					entries.push(resolvedExtPath);
				}
			}
			if (entries.length > 0) {
				return entries;
			}
		}
	}

	// Check for index.ts or index.js
	const indexTs = path.join(dir, "index.ts");
	const indexJs = path.join(dir, "index.js");
	if (fs.existsSync(indexTs)) {
		return [indexTs];
	}
	if (fs.existsSync(indexJs)) {
		return [indexJs];
	}

	return null;
}
```
Source: ice:packages/coding-agent/src/core/extensions/loader.ts:802-832.
None: additional fixed CLI flags, slash commands, keybindings, settings, environment variables, RPC commands or persistent files beyond the explicitly listed registration/host contracts. Each dynamically registered name is chosen by the extension.

### 6. Dependencies

`EXT-004`

External services/binaries: none required by this host mechanism. Plugins may invoke arbitrary host I/O because they are trusted executable code; this audit did not load or run any plugin.

### 7. ice-rust today

rust:crates/ice-extensions/src/loader.rs:248-270; rust:crates/ice-extensions/src/registry.rs:536-583. Native loading does not consume package.json. Plugin declare JSON instead carries providers/priority/platforms/ownsStdio. Higher-level Rust package config is a separate surface; native declaration alone is not this resource-manifest contract.

### 8. Port notes

Implement package resource metadata in the existing ice-cli package/resource resolver rather than inside the C ABI registrar. Define ICE/Pi selection, per-field all-string validation and entrypoint fallback as a versioned parser contract using serde_json and PathBuf. Validate containment or deliberately document external paths; do not silently reinterpret package-relative tilde as a home path.

### 9. Size and priority

Estimated Rust implementation: **70-125 lines** (estimate, not measured source). **P1** - Required for faithful native extension behavior and for ICE-specific plugins that depend on it.

## EXT-006 - Factory-local registration staging and commit ordering

### 1. Title and status

Factory-local registration staging and commit ordering. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/extensions/loader.ts:474-562. initializeExtension and extension.dispose. Tests: ice:packages/coding-agent/test/suite/regressions/8423-extension-factory-transaction.test.ts:9-47 (`discards failed provider/default/subscription registrations and revokes the captured API`); ice:packages/coding-agent/test/suite/regressions/8423-extension-factory-transaction.test.ts:49-70 (`does not leak a bus listener after session invalidation`).

### 3. What it does

Discard failed factory defaults/provider operations/subscriptions and revoke a captured failed API.

### 4. How it works

Factory initialization stages selected registrations until the factory succeeds. The transaction isolates a failed factory's defaults and subscriptions, but it cannot undo arbitrary factory side effects or every late commit failure.

Initialize a copy of shared flag values, ordered deferred provider operations, pending subscription attach closures and a disposed flag. A runtime Proxy exposes this local flag Map and no-op refreshTools while initializing; registerProvider/registerNativeProvider/unregisterProvider append ordered closures rather than mutating the shared queues. The API methods call a scoped assert that checks both runtime staleness and disposed. A bus Proxy allows immediate emit but stages listener attachment; unsubscribe before commit cancels the pending attachment.

After factory fulfillment, assert active, execute all provider operations in order, add only absent shared flag values, attach uncancelled subscriptions, mark initialization false, then call shared refreshTools. On any exception dispose listeners/pending attaches and rethrow. Already committed providers/defaults are not rolled back if a later commit step/refresh fails. Arbitrary import/factory I/O and immediate bus emissions are never transactional. Disposal sets disposed=true before unsubscribing, preventing later API calls; an unsubscribe exception can interrupt cleanup. No file journal or cross-process transaction exists.

### 5. Interfaces

```ts
async function initializeExtension(
	factory: ExtensionFactory,
	extension: Extension,
	runtime: ExtensionRuntime,
	cwd: string,
	eventBus: EventBus,
): Promise<void> {
	let initializing = true;
	let disposed = false;
	const defaults = new Map(runtime.flagValues);
	const registrations: Array<() => void> = [];
	const subscriptions = new Set<() => void>();
	const pendingSubscriptions: Array<() => void> = [];
	const assertActive = () => {
		runtime.assertActive();
		if (disposed) throw new Error("Extension factory API is no longer active");
	};
	const scopedRuntime = new Proxy(runtime, {
		get(target, key) {
			if (key === "assertActive") return assertActive;
			if (key === "flagValues" && initializing) return defaults;
			if (key === "refreshTools" && initializing) return () => {};
			if (initializing && key === "registerProvider") {
				return (name: string, config: ProviderConfig, source?: string) => {
					registrations.push(() => runtime.registerProvider(name, config, source));
				};
			}
			if (initializing && key === "registerNativeProvider") {
				return (provider: Provider, source?: string) => {
					registrations.push(() => runtime.registerNativeProvider(provider, source));
				};
			}
			if (initializing && key === "unregisterProvider") {
				return (name: string, source?: string) => {
					registrations.push(() => runtime.unregisterProvider(name, source));
				};
			}
			return Reflect.get(target, key);
		},
	});
	const scopedBus = new Proxy(eventBus, {
		get(target, key) {
			if (key === "emit")
				return (channel: string, data: unknown) => {
					assertActive();
					eventBus.emit(channel, data);
				};
			if (key === "on")
				return (channel: string, handler: (data: unknown) => void) => {
					assertActive();
					let cancelled = false;
					let detach: (() => void) | undefined;
					const unsubscribe = () => {
						cancelled = true;
						detach?.();
						subscriptions.delete(unsubscribe);
					};
					const attach = () => {
						if (cancelled) return;
						detach = eventBus.on(channel, handler);
						subscriptions.add(unsubscribe);
					};
					if (initializing) pendingSubscriptions.push(attach);
					else attach();
					return unsubscribe;
				};
			return Reflect.get(target, key);
		},
	});
	extension.dispose = () => {
		disposed = true;
		for (const unsubscribe of [...subscriptions]) unsubscribe();
		pendingSubscriptions.length = 0;
	};
	try {
		await factory(createExtensionAPI(extension, scopedRuntime, cwd, scopedBus));
		assertActive();
		for (const register of registrations) register();
		for (const [name, value] of defaults) {
			if (!runtime.flagValues.has(name)) runtime.flagValues.set(name, value);
		}
		for (const attach of pendingSubscriptions) attach();
		initializing = false;
		runtime.refreshTools();
	} catch (error) {
		extension.dispose();
		throw error;
	}
}
```
Source: ice:packages/coding-agent/src/core/extensions/loader.ts:474-562.
None: additional fixed CLI flags, slash commands, keybindings, settings, environment variables, RPC commands or persistent files beyond the explicitly listed registration/host contracts. Each dynamically registered name is chosen by the extension.

### 6. Dependencies

`EXT-001`, `EXT-008`, `EXT-009`, `EXT-015`, `EXT-017`, `EXT-018`

External services/binaries: none required by this host mechanism. Plugins may invoke arbitrary host I/O because they are trusted executable code; this audit did not load or run any plugin.

### 7. ice-rust today

rust:crates/ice-extensions/src/loader.rs:151-203; rust:crates/ice-extensions/src/registry.rs:389-410. Rust drops a fresh native registry after nonzero registration and releases stdio ownership, but has no matching dynamic flag/default/provider/bus staging contract. Native initializer side effects also cannot be undone by dropping a registry. Preserve explicit commit/lifetime state in ports.

### 8. Port notes

Add a candidate-registration state in ice-extensions that stages provider changes, flag defaults and owned bus subscriptions, then commits only after successful registration. Use owner IDs and deterministic commit order; record which operations may already have committed when a later callback fails. Registration rollback must not unload code still referenced by pending callbacks or claim to reverse filesystem/network side effects.

### 9. Size and priority

Estimated Rust implementation: **110-190 lines** (estimate, not measured source). **P0** - Failed plugins must not leak shared registrations or retain a usable stale API.

## EXT-007 - Initialization console buffering and restoration

### 1. Title and status

Initialization console buffering and restoration. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/core/extensions/loader.ts:681-754. enableExtensionConsoleCapture/disableExtensionConsoleCapture/setExtensionConsoleSink/withExtensionConsoleCapture. Tests: ice:packages/coding-agent/test/extension-console-capture.test.ts:31-47 (`buffers load-time console output until a sink attaches, then streams to it`); ice:packages/coding-agent/test/extension-console-capture.test.ts:49-64 (`restores console after a failing extension and does nothing when disabled`).

### 3. What it does

Capture load-time console output until the interactive UI can display it safely.

### 4. How it works

Load-time console messages can be buffered with the originating extension path and flushed when a sink attaches. Capture temporarily replaces global console methods and restores them even after a factory failure.

Enable a single global capture only when absent. Each file load temporarily replaces console.log/info/warn/error; format args through formatWithOptions({},...args), attach extensionPath/level, and buffer or synchronously call the sink. Restore original methods in finally. Inline factory loading itself does not wrap capture. Unrelated concurrent logging can be attributed to the current extension; there is no async-context isolation.

Attaching a sink installs it then flushes buffer.splice(0) in order. Sink exceptions propagate and can lose unconsumed spliced entries or fail loading. Disable returns remaining entries and removes global capture without printing. No byte/count bound, redaction, callback deadline or disk persistence exists. If capture is disabled during an active wrapper, its closure still holds the old capture until restoration.

### 5. Interfaces

```ts
export type ExtensionConsoleLevel = "log" | "info" | "warn" | "error";

export interface ExtensionConsoleEntry {
	extensionPath: string;
	level: ExtensionConsoleLevel;
	text: string;
}

type ExtensionConsoleSink = (entry: ExtensionConsoleEntry) => void;

/**
 * Console output written by extensions while they load. Interactive mode enables
 * capture before the TUI owns the terminal so stray writes do not appear above it.
 */
let extensionConsoleCapture: { buffer: ExtensionConsoleEntry[]; sink?: ExtensionConsoleSink } | undefined;

export function enableExtensionConsoleCapture(): void {
	extensionConsoleCapture ??= { buffer: [] };
}

/** Stop capturing and return entries no sink consumed. */
export function disableExtensionConsoleCapture(): ExtensionConsoleEntry[] {
	const buffered = extensionConsoleCapture?.buffer ?? [];
	extensionConsoleCapture = undefined;
	return buffered;
}

/** Deliver captured entries to `sink`, flushing anything buffered before it was attached. */
export function setExtensionConsoleSink(sink: ExtensionConsoleSink): void {
	if (!extensionConsoleCapture) return;
	extensionConsoleCapture.sink = sink;
	const buffered = extensionConsoleCapture.buffer.splice(0);
	for (const entry of buffered) sink(entry);
}

async function withExtensionConsoleCapture<T>(extensionPath: string, run: () => Promise<T>): Promise<T> {
	const capture = extensionConsoleCapture;
	if (!capture) return run();
	const levels: ExtensionConsoleLevel[] = ["log", "info", "warn", "error"];
	const originals = levels.map((level) => [level, console[level]] as const);
	for (const level of levels) {
		console[level] = (...args: unknown[]) => {
			const entry: ExtensionConsoleEntry = { extensionPath, level, text: formatWithOptions({}, ...args) };
			if (capture.sink) capture.sink(entry);
			else capture.buffer.push(entry);
		};
	}
	try {
		return await run();
	} finally {
		for (const [level, original] of originals) console[level] = original;
	}
}
```
Source: ice:packages/coding-agent/src/core/extensions/loader.ts:681-733.
None: additional fixed CLI flags, slash commands, keybindings, settings, environment variables, RPC commands or persistent files beyond the explicitly listed registration/host contracts. Each dynamically registered name is chosen by the extension.

### 6. Dependencies

`EXT-001`

External services/binaries: none required by this host mechanism. Plugins may invoke arbitrary host I/O because they are trusted executable code; this audit did not load or run any plugin.

### 7. ice-rust today

rust:crates/ice-extensions/src/loader.rs:119-214; rust:crates/ice-extensions/src/lib.rs:343-415. Native load diagnostics do not intercept arbitrary plugin stdout/stderr or initializer logging. A native logging callback can provide safe attribution, but global descriptor redirection around concurrent plugins is not an equivalent safe implementation.

### 8. Port notes

Add an owner-attributed SDK logging callback with level, text and bounded startup buffering, then attach an ice-cli sink after UI initialization. A native plugin must opt into this logging API; arbitrary stdout interception requires a separately reviewed process/stdio design. Prefer a Mutex<VecDeque<LogRecord>> with explicit overflow behavior and retain restoration/lifetime tests for the host wrapper.

### 9. Size and priority

Estimated Rust implementation: **90-160 lines** (estimate, not measured source). **P2** - Required for faithful native extension behavior and for ICE-specific plugins that depend on it.

## EXT-008 - Copy-on-write event registration and idempotent unsubscribe

### 1. Title and status

Copy-on-write event registration and idempotent unsubscribe. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/extensions/loader.ts:268-281; ice:packages/coding-agent/src/core/extensions/runner.ts:888-923. ExtensionAPI.on. Test: ice:packages/coding-agent/test/suite/regressions/8423-extension-factory-transaction.test.ts:72-101 (`uses a stable dispatch snapshot and supports unsubscribe`).

Exported symbols: `ExtensionEvent`, `ExtensionHandler` (ice:packages/coding-agent/src/core/extensions/types.ts:1088; ice:packages/coding-agent/src/core/extensions/types.ts:1249).

### 3. What it does

Support late handlers and removal without mutating the handler list already captured by a dispatcher.

### 4. How it works

Handlers can be added and removed dynamically without rewriting a currently captured handler array. Unsubscribe is idempotent and takes effect for future collection of that array.

on asserts active and replaces the event array with [...old, handler]. Return an unsubscribe closure with a subscribed flag. Its first call copies the current array, removes the first reference-identical matching handler and stores the copy; later calls do nothing. Unsubscribe does not assert active, so it remains usable after invalidation. Registering the same function twice creates two registrations; each closure removes one matching occurrence.

A dispatcher captures each extension’s array on reaching it, so a handler appended to that same array does not run in the current captured iteration and a removed handler can still run in that iteration. Later extensions can observe changes made before they are reached. No event-name validation, priority, handler cap or cross-extension snapshot exists. The typed API return declaration must be reconciled with the actual unsubscribe return; the regression test explicitly requires a function.

### 5. Interfaces

```ts
		on(event: string, handler: HandlerFn): () => void {
			runtime.assertActive();
			// Copy-on-write arrays keep every dispatch's captured handler list stable.
			extension.handlers.set(event, [...(extension.handlers.get(event) ?? []), handler]);
			let subscribed = true;
			return () => {
				if (!subscribed) return;
				subscribed = false;
				const list = [...(extension.handlers.get(event) ?? [])];
				const index = list.indexOf(handler);
				if (index !== -1) list.splice(index, 1);
				extension.handlers.set(event, list);
			};
		},
```
Source: ice:packages/coding-agent/src/core/extensions/loader.ts:268-281.
None: additional fixed CLI flags, slash commands, keybindings, settings, environment variables, RPC commands or persistent files beyond the explicitly listed registration/host contracts. Each dynamically registered name is chosen by the extension.

`ExtensionHandler<E, R = undefined> = (event: E, ctx: ExtensionContext) => Promise<R | void> | R | void`. `ExtensionEvent` is the union of ProjectTrustEvent, ResourcesDiscoverEvent, SessionEvent, ContextEvent, BeforeProviderRequestEvent, BeforeProviderHeadersEvent, AfterProviderResponseEvent, BeforeAgentStartEvent, AgentStartEvent, AgentEndEvent, AgentSettledEvent, TurnStartEvent, TurnEndEvent, MessageStartEvent, MessageUpdateEvent, MessageEndEvent, ToolExecutionStartEvent, ToolExecutionUpdateEvent, ToolExecutionEndEvent, ModelSelectEvent, ThinkingLevelSelectEvent, UserBashEvent, InputEvent, ToolCallEvent and ToolResultEvent. These declarations do not add event-name validation to the runtime string-keyed map (ice:packages/coding-agent/src/core/extensions/types.ts:1088-1113; ice:packages/coding-agent/src/core/extensions/types.ts:1249).

### 6. Dependencies

`EXT-009`, `EXT-022`

External services/binaries: none required by this host mechanism. Plugins may invoke arbitrary host I/O because they are trusted executable code; this audit did not load or run any plugin.

### 7. ice-rust today

rust:crates/ice-extensions/src/registry.rs:478-502; rust:crates/ice-extensions/src/lib.rs:500-520. Native event registration is confined to registration scope and snapshots afterward; no dynamic unsubscribe handle is exposed. Native priorities/platform filters are additional behavior, not TS load-order parity. Add versioned dynamic registration or document that a port must pre-register conditional handlers.

### 8. Port notes

Add subscription handles and owner-scoped removal to ice-extensions if runtime registration is required. Store Arc-backed callback records, snapshot the relevant list before invocation and defer library release until all snapshots finish. Preserve existing native priority/platform selection as an explicit policy instead of silently applying TS load order.

### 9. Size and priority

Estimated Rust implementation: **70-125 lines** (estimate, not measured source). **P1** - Required for faithful native extension behavior and for ICE-specific plugins that depend on it.

## EXT-009 - Core action rebinding and explicit UI mode defaults

### 1. Title and status

Core action rebinding and explicit UI mode defaults. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/extensions/loader.ts:200-250; ice:packages/coding-agent/src/core/extensions/loader.ts:382-471; ice:packages/coding-agent/src/core/extensions/runner.ts:288-383; ice:packages/coding-agent/src/core/extensions/runner.ts:446-476. createExtensionRuntime/bindCore/bindCommandContext/setUIContext. Tests: ice:packages/coding-agent/test/extensions-runner.test.ts:537-545 (`exposes print mode and hasUI false by default`); ice:packages/coding-agent/test/extensions-runner.test.ts:559-568 (`exposes rpc mode with hasUI true when an RPC UI context is provided`); ice:packages/coding-agent/test/extensions-runner.test.ts:123-133 (`reflects the bound compaction state live`).

Exported symbols: `RefreshToolsHandler`, `ExtensionRuntimeState` (ice:packages/coding-agent/src/core/extensions/types.ts:1659; ice:packages/coding-agent/src/core/extensions/types.ts:1673).

### 3. What it does

Reject core actions before initialization, then bind existing API closures to the current host functions.

### 4. How it works

An extension receives one API whose session actions become available when the host binds the runtime. UI capability and command control are bound separately, so neither mode strings nor successful no-op placeholders prove that a real action exists.

createExtensionRuntime gives synchronous throwing stubs to session/message/tool/introspection actions, a rejected promise for setModel, no-op refreshTools, empty flags and separate legacy/native provider queues. The API object itself is ordinary, not a proxy; its methods assert scoped activity then read the shared runtime method at call time. ice.exec calls the process helper directly and can work during loading. Provider queues are the deliberate pre-bind exception.

bindCore directly assigns actions/context functions. Defaults are settingsManager undefined, isCompacting false, stopAfterTurn/compactAfterTurn no-op, hasPendingSteering falling back to hasPendingMessages, and system-prompt-options {cwd}. Before binding constructor defaults idle/trusted true and other queries empty/false. UI binding is independent: setUIContext(ui, mode="print") assigns the exact mode argument; hasUI is object identity against the no-op UI, not a mode check. A supplied UI with omitted mode can be hasUI=true and mode=print. Command-context handlers are bound separately or reset to permissive no-op placeholders. No automatic persistence is performed by rebinding.

### 5. Interfaces

```ts
export function createExtensionRuntime(): ExtensionRuntime {
	const notInitialized = () => {
		throw new Error("Extension runtime not initialized. Action methods cannot be called during extension loading.");
	};
	const state: { staleMessage?: string } = {};
	const assertActive = () => {
		if (state.staleMessage) {
			throw new Error(state.staleMessage);
		}
	};

	const runtime: ExtensionRuntime = {
		sendMessage: notInitialized,
		sendUserMessage: notInitialized,
		appendEntry: notInitialized,
		setSessionName: notInitialized,
		getSessionName: notInitialized,
		setLabel: notInitialized,
		getActiveTools: notInitialized,
		getAllTools: notInitialized,
		setActiveTools: notInitialized,
		// registerTool() is valid during extension load; refresh is only needed post-bind.
		refreshTools: () => {},
		getCommands: notInitialized,
		setModel: () => Promise.reject(new Error("Extension runtime not initialized")),
		getThinkingLevel: notInitialized,
		setThinkingLevel: notInitialized,
		flagValues: new Map(),
		pendingProviderRegistrations: [],
		pendingNativeProviderRegistrations: [],
		assertActive,
		invalidate: (message) => {
			state.staleMessage ??=
				message ??
				"This extension ctx is stale after session replacement or reload. Do not use a captured ice or command ctx after ctx.newSession(), ctx.fork(), ctx.switchSession(), or ctx.reload(). For newSession, fork, and switchSession, move post-replacement work into withSession and use the ctx passed to withSession. For reload, do not use the old ctx after await ctx.reload().";
		},
		// Pre-bind: queue registrations so bindCore() can flush them once the
		// model registry is available. bindCore() replaces both with direct calls.
		registerProvider: (name, config, extensionPath = "<unknown>") => {
			runtime.pendingProviderRegistrations.push({ name, config, extensionPath });
		},
		registerNativeProvider: (provider, extensionPath = "<unknown>") => {
			runtime.pendingNativeProviderRegistrations.push({ provider, extensionPath });
		},
		unregisterProvider: (name) => {
			runtime.pendingProviderRegistrations = runtime.pendingProviderRegistrations.filter((r) => r.name !== name);
			runtime.pendingNativeProviderRegistrations = runtime.pendingNativeProviderRegistrations.filter(
				(r) => r.provider.id !== name,
			);
		},
	};
```
Source: ice:packages/coding-agent/src/core/extensions/loader.ts:200-250.
```ts
	bindCore(
		actions: ExtensionActions,
		contextActions: ExtensionContextActions,
		providerActions?: {
			registerProvider?: (name: string, config: ProviderConfig) => void;
			registerNativeProvider?: (provider: Provider) => void;
			unregisterProvider?: (name: string) => void;
		},
	): void {
		// Copy actions into the shared runtime (all extension APIs reference this)
		this.runtime.sendMessage = actions.sendMessage;
		this.runtime.sendUserMessage = actions.sendUserMessage;
		this.runtime.appendEntry = actions.appendEntry;
		this.runtime.setSessionName = actions.setSessionName;
		this.runtime.getSessionName = actions.getSessionName;
		this.runtime.setLabel = actions.setLabel;
		this.runtime.getActiveTools = actions.getActiveTools;
		this.runtime.getAllTools = actions.getAllTools;
		this.runtime.setActiveTools = actions.setActiveTools;
		this.runtime.refreshTools = actions.refreshTools;
		this.runtime.getCommands = actions.getCommands;
		this.runtime.setModel = actions.setModel;
		this.runtime.getThinkingLevel = actions.getThinkingLevel;
		this.runtime.setThinkingLevel = actions.setThinkingLevel;

		// Context actions (required)
		this.getModel = contextActions.getModel;
		this.getScopedModels = contextActions.getScopedModels;
		this.getSettingsManager = contextActions.getSettingsManager ?? (() => undefined);
		this.isIdleFn = contextActions.isIdle;
		this.isCompactingFn = contextActions.isCompacting ?? (() => false);
		this.isProjectTrustedFn = contextActions.isProjectTrusted;
		this.getSignalFn = contextActions.getSignal;
		this.abortFn = contextActions.abort;
		this.stopAfterTurnFn = contextActions.stopAfterTurn ?? (() => {});
		this.compactAfterTurnFn = contextActions.compactAfterTurn ?? (() => {});
		this.hasPendingMessagesFn = contextActions.hasPendingMessages;
		this.hasPendingSteeringFn = contextActions.hasPendingSteering ?? contextActions.hasPendingMessages;
		this.shutdownHandler = contextActions.shutdown;
		this.getContextUsageFn = contextActions.getContextUsage;
		this.compactFn = contextActions.compact;
		this.getSystemPromptFn = contextActions.getSystemPrompt;
		this.getSystemPromptOptionsFn = contextActions.getSystemPromptOptions ?? (() => ({ cwd: this.cwd }));
```
Source: ice:packages/coding-agent/src/core/extensions/runner.ts:341-383.
```ts
	bindCommandContext(actions?: ExtensionCommandContextActions): void {
		if (actions) {
			this.waitForIdleFn = actions.waitForIdle;
			this.newSessionHandler = actions.newSession;
			this.forkHandler = actions.fork;
			this.navigateTreeHandler = actions.navigateTree;
			this.switchSessionHandler = actions.switchSession;
			this.reloadHandler = actions.reload;
			return;
		}

		this.waitForIdleFn = async () => {};
		this.newSessionHandler = async () => ({ cancelled: false });
		this.forkHandler = async () => ({ cancelled: false });
		this.navigateTreeHandler = async () => ({ cancelled: false });
		this.switchSessionHandler = async () => ({ cancelled: false });
		this.reloadHandler = async () => {};
	}

	setUIContext(uiContext?: ExtensionUIContext, mode: ExtensionMode = "print"): void {
		this.uiContext = uiContext ?? noOpUIContext;
		this.mode = mode;
	}

	getUIContext(): ExtensionUIContext {
		return this.uiContext;
	}

	hasUI(): boolean {
		return this.uiContext !== noOpUIContext;
	}
```
Source: ice:packages/coding-agent/src/core/extensions/runner.ts:446-476.
None: additional fixed CLI flags, slash commands, keybindings, settings, environment variables, RPC commands or persistent files beyond the explicitly listed registration/host contracts. Each dynamically registered name is chosen by the extension.

`RefreshToolsHandler = () => void`. `ExtensionRuntimeState` holds flagValues:Map<string, boolean|string>; pendingProviderRegistrations:Array<{name, config, extensionPath}>; pendingNativeProviderRegistrations:Array<{provider, extensionPath}>; assertActive:()=>void; invalidate:(message?:string)=>void; registerProvider:(name, config, extensionPath?)=>void; registerNativeProvider:(provider, extensionPath?)=>void; unregisterProvider:(name, extensionPath?)=>void. The callback/state declarations do not change the pre-bind queue and first-stale-cause semantics already described (ice:packages/coding-agent/src/core/extensions/types.ts:1659; ice:packages/coding-agent/src/core/extensions/types.ts:1673-1692).

### 6. Dependencies

`EXT-001`, `EXT-010`, `EXT-019`, `EXT-020`

External services/binaries: none required by this host mechanism. Plugins may invoke arbitrary host I/O because they are trusted executable code; this audit did not load or run any plugin.

### 7. ice-rust today

rust:crates/ice-extensions/src/actions.rs:527-695; rust:crates/ice-cli/src/extensions_actions.rs:54-146; rust:crates/ice-extensions/src/actions.rs:984-1040. Native ActionBridge is constructed with an Arc<dyn RuntimeActionHost>, runtime handle and session mailboxes. HarnessActionHost holds a OnceLock<Weak<AgentHarness>> populated after construction; calls fail when the harness is not initialized or has been dropped. This is host readiness and weak ownership, not a mutable TS runtime object or a later-filled raw bridge pointer. The native runtime has 26 distinct action IDs, but no full mode/hasUI/base-context surface; actual action implementations are compared separately.

### 8. Port notes

Document host readiness as an explicit ActionBridge capability state and provide structured unavailable errors before HarnessActionHost is bound. Add mode/capability and missing context queries to an owned SDK DTO; keep command operations separate from UI availability. Bind providers and flags through the existing host lifecycle, preserving queued-failure attribution and preventing success-shaped stubs from being treated as implemented actions.

### 9. Size and priority

Estimated Rust implementation: **120-210 lines** (estimate, not measured source). **P1** - Required for faithful native extension behavior and for ICE-specific plugins that depend on it.

## EXT-010 - First-cause invalidation and lazy stale-context guards

### 1. Title and status

First-cause invalidation and lazy stale-context guards. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/extensions/loader.ts:204-235; ice:packages/coding-agent/src/core/extensions/loader.ts:474-562; ice:packages/coding-agent/src/core/extensions/runner.ts:575-589; ice:packages/coding-agent/src/core/extensions/runner.ts:740-876. invalidate/assertActive/createContext/createCommandContext. Test: ice:packages/coding-agent/test/suite/regressions/8423-extension-factory-transaction.test.ts:49-70 (`does not leak a bus listener after session invalidation`).

### 3. What it does

Prevent captured extension API/context operations from mutating a replaced session.

### 4. How it works

Invalidation makes guarded APIs and contexts reject future access with the first stored cause. It revokes entry points without automatically stopping callbacks, processes, or objects already handed out.

runtime.invalidate stores only the first message. runner.invalidate likewise records its first stale message, invalidates runtime, then calls each extension.dispose. It does not reset UI, clear maps/listeners, cancel timers/processes or cancel handlers already running. If disposal throws, later disposals can be skipped and repeat invalidation does not retry them because staleMessage is already set. Scoped factory assert checks both disposed and shared runtime; failed/disposed APIs throw Extension factory API is no longer active unless shared staleness throws first.

Context getters/methods assert runner activity when invoked. createCommandContext copies property descriptors so getters remain lazy. A retained object obtained before invalidation, such as sessionManager/UI, is not itself automatically revoked; its own methods need appropriate guards. Some direct runner helpers, such as setFlagValue or shutdown, do not assert active. In-flight dispatch can continue using captured handlers, whose subsequent guarded accesses then fail. Unsubscribing bus listeners prevents new dispatch, not callbacks already executing.

### 5. Interfaces

```ts
	invalidate(
		message = "This extension ctx is stale after session replacement or reload. Do not use a captured ice or command ctx after ctx.newSession(), ctx.fork(), ctx.switchSession(), or ctx.reload(). For newSession, fork, and switchSession, move post-replacement work into withSession and use the ctx passed to withSession. For reload, do not use the old ctx after await ctx.reload().",
	): void {
		if (!this.staleMessage) {
			this.staleMessage = message;
			this.runtime.invalidate(message);
			for (const extension of this.extensions) extension.dispose?.();
		}
	}

	private assertActive(): void {
		if (this.staleMessage) {
			throw new Error(this.staleMessage);
		}
	}
```
Source: ice:packages/coding-agent/src/core/extensions/runner.ts:575-589.
```ts
	const defaults = new Map(runtime.flagValues);
	const registrations: Array<() => void> = [];
	const subscriptions = new Set<() => void>();
	const pendingSubscriptions: Array<() => void> = [];
	const assertActive = () => {
		runtime.assertActive();
		if (disposed) throw new Error("Extension factory API is no longer active");
	};
```
Source: ice:packages/coding-agent/src/core/extensions/loader.ts:483-490.
```ts
	extension.dispose = () => {
		disposed = true;
		for (const unsubscribe of [...subscriptions]) unsubscribe();
		pendingSubscriptions.length = 0;
	};
	try {
		await factory(createExtensionAPI(extension, scopedRuntime, cwd, scopedBus));
		assertActive();
		for (const register of registrations) register();
		for (const [name, value] of defaults) {
			if (!runtime.flagValues.has(name)) runtime.flagValues.set(name, value);
		}
		for (const attach of pendingSubscriptions) attach();
		initializing = false;
		runtime.refreshTools();
	} catch (error) {
		extension.dispose();
		throw error;
```
Source: ice:packages/coding-agent/src/core/extensions/loader.ts:543-560.
```ts
	createCommandContext(): ExtensionCommandContext {
		// Use property descriptors instead of object spread so the guarded getters from
		// createContext() stay lazy. A spread would eagerly read them once and freeze the
		// old values into the returned object, bypassing stale-instance checks.
		const context = Object.defineProperties(
			{},
			Object.getOwnPropertyDescriptors(this.createContext()),
		) as ExtensionCommandContext;
		context.getSystemPromptOptions = () => {
			this.assertActive();
			return this.getSystemPromptOptionsFn();
		};
		context.waitForIdle = () => {
			this.assertActive();
			return this.waitForIdleFn();
		};
		context.newSession = (options) => {
			this.assertActive();
			return this.newSessionHandler(options);
		};
		context.fork = (entryId, options) => {
			this.assertActive();
			return this.forkHandler(entryId, options);
		};
		context.navigateTree = (targetId, options) => {
			this.assertActive();
			return this.navigateTreeHandler(targetId, options);
		};
		context.switchSession = (sessionPath, options) => {
			this.assertActive();
			return this.switchSessionHandler(sessionPath, options);
		};
		context.reload = () => {
			this.assertActive();
			return this.reloadHandler();
		};
		return context;
```
Source: ice:packages/coding-agent/src/core/extensions/runner.ts:840-876.
None: additional fixed CLI flags, slash commands, keybindings, settings, environment variables, RPC commands or persistent files beyond the explicitly listed registration/host contracts. Each dynamically registered name is chosen by the extension.

### 6. Dependencies

`EXT-006`, `EXT-009`, `EXT-017`, `EXT-033`

External services/binaries: none required by this host mechanism. Plugins may invoke arbitrary host I/O because they are trusted executable code; this audit did not load or run any plugin.

### 7. ice-rust today

rust:crates/ice-extensions/src/registry.rs:378-380; rust:crates/ice-extensions/src/loader.rs:486-537; rust:crates/ice-extensions/src/translate.rs:342-354. Rust uses an active AtomicBool with keepalive handles to fence stale sessions without unloading in-use code. Preserve that improvement. Native cancellation/late callback and payload cleanup must still be audited; skipping an inactive event is not automatically a complete ownership cleanup.

### 8. Port notes

Retain native weak opaque handles and per-session active flags. Add revocation checks at the actual mutation/admission boundary of asynchronous runtime actions, not only before spawning them, and define cleanup for dialogs, status keys and owner components. Use Arc-owned guards for in-flight callbacks; invalidation cannot forcibly stop arbitrary foreign code.

### 9. Size and priority

Estimated Rust implementation: **80-150 lines** (estimate, not measured source). **P0** - Session replacement must revoke stale mutation capabilities without unloading live native callbacks.

## EXT-011 - Object-schema tool registration and immediate refresh without rollback

### 1. Title and status

Object-schema tool registration and immediate refresh without rollback. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/extensions/loader.ts:283-298; ice:packages/coding-agent/src/core/extensions/runner.ts:482-504. registerTool/getAllRegisteredTools/getToolDefinition. Tests: ice:packages/coding-agent/test/extensions-runner.test.ts:408-442 (`keeps first tool when two extensions register the same name`); ice:packages/coding-agent/test/suite/regressions/8423-extension-factory-transaction.test.ts:103-118 malformed schema case.

Exported symbols: `defineTool` (ice:packages/coding-agent/src/core/extensions/types.ts:563).

### 3. What it does

Register or replace a tool and refresh the host’s active definitions after loading.

### 4. How it works

Tool registration updates the extension's live descriptor map and asks the host to refresh its tool registry. The loader validates only a narrow object-schema condition; downstream execution and metadata handling remain separate.

registerTool requires parameters to be truthy, typeof object, contain a type property and equal type:"object"; otherwise throw the exact error below. It does not deeply validate JSON schema, validate name/execute, clone the descriptor or normalize prompt metadata. Store {definition:tool, sourceInfo} by exact tool.name, replacing a same-extension registration, then call runtime.refreshTools. During factory initialization refresh is suppressed until commit. After binding a throwing refresh leaves the new map entry installed: there is no local rollback.

getAllRegisteredTools traverses extension maps in load order and returns the first exact definition.name, retaining original references. getToolDefinition searches each map by requested key. A plugin can mutate its retained definition, so map key and later definition.name can diverge. No sorting, active-set change or snippet/guideline normalization occurs in this runner. Higher-level collection/preparation may normalize separately and must not be attributed here.

`defineTool(tool)` returns the identical descriptor reference through a TypeScript intersection cast to `ToolDefinition & AnyToolDefinition`. It neither clones nor registers the tool and performs no runtime validation; the object-schema check remains in registerTool (ice:packages/coding-agent/src/core/extensions/types.ts:563-567).

### 5. Interfaces

```ts
		registerTool(tool: ToolDefinition): void {
			runtime.assertActive();
			if (
				!tool.parameters ||
				typeof tool.parameters !== "object" ||
				!("type" in tool.parameters) ||
				tool.parameters.type !== "object"
			) {
				throw new Error(`Tool '${tool.name}' parameters must be an object schema`);
			}
			extension.tools.set(tool.name, {
				definition: tool,
				sourceInfo: extension.sourceInfo,
			});
			runtime.refreshTools();
		},
```
Source: ice:packages/coding-agent/src/core/extensions/loader.ts:283-298.
```ts
	/** Get all registered tools from all extensions (first registration per name wins). */
	getAllRegisteredTools(): RegisteredTool[] {
		const toolsByName = new Map<string, RegisteredTool>();
		for (const ext of this.extensions) {
			for (const tool of ext.tools.values()) {
				if (!toolsByName.has(tool.definition.name)) {
					toolsByName.set(tool.definition.name, tool);
				}
			}
		}
		return Array.from(toolsByName.values());
	}

	/** Get a tool definition by name. Returns undefined if not found. */
	getToolDefinition(toolName: string): RegisteredTool["definition"] | undefined {
		for (const ext of this.extensions) {
			const tool = ext.tools.get(toolName);
			if (tool) {
				return tool.definition;
			}
		}
		return undefined;
	}
```
Source: ice:packages/coding-agent/src/core/extensions/runner.ts:482-504.
None: additional fixed CLI flags, slash commands, keybindings, settings, environment variables, RPC commands or persistent files beyond the explicitly listed registration/host contracts. Each dynamically registered name is chosen by the extension.

### 6. Dependencies

`EXT-008`, `EXT-009`, `EXT-012`

External services/binaries: none required by this host mechanism. Plugins may invoke arbitrary host I/O because they are trusted executable code; this audit did not load or run any plugin.

### 7. ice-rust today

rust:crates/ice-extensions/src/lib.rs:343-415; rust:crates/ice-extensions/src/registry.rs:413-427. Rust requires a nonblank tool name and parses parameters as any JSON object; it does not require parameters.type === "object". Thus {type:"string"} is an object representation accepted by this registrar although TypeScript rejects it. The host owns copies of name/description/schema, fixes constrained_sampling to None, and retains first registration even within one plugin. Native TLS registration ends after the initializer and its immutable snapshot cannot implement late replacement/refresh. StableToolSchema lacks label, promptSnippet/promptGuidelines, prepareArguments, executionMode, access and render callbacks (rust:crates/ice-plugin-sdk/src/lib.rs:348-353). These differences are schema semantics and host metadata differences, not merely language syntax.

### 8. Port notes

Extend StableToolSchema through an appended, size-checked registration DTO or new optional registrar carrying metadata and hooks. Decide whether same-owner replacement is supported after registration; if so, atomically swap definition and executable registration only after validation and successful host refresh. Preserve first-owner conflict diagnostics and explicitly distinguish JSON object representation from schema type:"object" validation.

### 9. Size and priority

Estimated Rust implementation: **100-180 lines** (estimate, not measured source). **P1** - Required for faithful native extension behavior and for ICE-specific plugins that depend on it.

## EXT-012 - Fresh-context tool wrapper and additive active-tool attribution

### 1. Title and status

Fresh-context tool wrapper and additive active-tool attribution. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/extensions/wrapper.ts:1-45; ice:packages/coding-agent/src/core/tools/tool-definition-wrapper.ts:1-47. wrapRegisteredTool/wrapRegisteredTools/wrapToolDefinition/createToolDefinitionFromAgentTool. Exact subset/addition behavior is explicit source; next-turn tool installation is specified in agent/session integration tests.

### 3. What it does

Report tools that became active during successful execution without conflating additions with a narrowed tool set.

### 4. How it works

A registered tool executes with a fresh extension context and reports newly enabled tools when the active set only grows. The comparison surrounds execution without locking other tools' changes.

wrapToolDefinition projects name, label, description, parameters, constrainedSampling, prepareArguments, executionMode and execute; execute forwards an explicit context or obtains a fresh context from the factory. It does not itself dispatch tool_call/tool_result. Reverse adaptation creates a minimal ToolDefinition around a plain AgentTool.

wrapRegisteredTool snapshots active names, awaits execute, snapshots again, and only when every old name remains computes additions. Merge existing result.addedToolNames followed by new names through a Set. A removed old name or no addition returns the original result unchanged. No execution failure path adds names. Post-execute active-query failure can reject after side effects. There is no lock against concurrent tools changing the active set, so attribution can include sibling changes. Do not blindly retry after such a failure.

### 5. Interfaces

```ts
export function wrapToolDefinition<TDetails = unknown>(
	definition: ToolDefinition<any, TDetails>,
	ctxFactory?: () => ExtensionContext,
): AgentTool<any, TDetails> {
	return {
		name: definition.name,
		label: definition.label,
		description: definition.description,
		parameters: definition.parameters,
		constrainedSampling: definition.constrainedSampling,
		prepareArguments: definition.prepareArguments,
		executionMode: definition.executionMode,
		execute: (toolCallId, params, signal, onUpdate, ctx?: ExtensionContext) =>
			definition.execute(toolCallId, params, signal, onUpdate, ctx ?? (ctxFactory?.() as ExtensionContext)),
	};
}

/** Wrap multiple ToolDefinitions into AgentTools for the core runtime. */
export function wrapToolDefinitions(
	definitions: ToolDefinition<any, any>[],
	ctxFactory?: () => ExtensionContext,
): AgentTool<any>[] {
	return definitions.map((definition) => wrapToolDefinition(definition, ctxFactory));
}

/**
 * Synthesize a minimal ToolDefinition from an AgentTool.
 *
 * This keeps AgentSession's internal registry definition-first even when a caller
 * provides plain AgentTool overrides that do not include prompt metadata or renderers.
 */
export function createToolDefinitionFromAgentTool(tool: AgentTool<any>): ToolDefinition<any, unknown> {
	return {
		name: tool.name,
		label: tool.label,
		description: tool.description,
		parameters: tool.parameters as any,
		constrainedSampling: tool.constrainedSampling,
		prepareArguments: tool.prepareArguments,
		executionMode: tool.executionMode,
		execute: async (toolCallId, params, signal, onUpdate) => tool.execute(toolCallId, params, signal, onUpdate),
	};
```
Source: ice:packages/coding-agent/src/core/tools/tool-definition-wrapper.ts:5-46.
```ts
export function wrapRegisteredTool(registeredTool: RegisteredTool, runner: ExtensionRunner): AgentTool {
	const tool = wrapToolDefinition(registeredTool.definition, () => runner.createContext());
	const execute = tool.execute;
	return {
		...tool,
		execute: async (toolCallId, params, signal, onUpdate) => {
			const activeBefore = runner.getActiveTools();
			const result = await execute(toolCallId, params, signal, onUpdate);
			const activeAfter = runner.getActiveTools();
			if (!activeBefore.every((name) => activeAfter.includes(name))) return result;

			const beforeNames = new Set(activeBefore);
			const addedToolNames = activeAfter.filter((name) => !beforeNames.has(name));
			if (addedToolNames.length === 0) return result;
			return {
				...result,
				addedToolNames: [...new Set([...(result.addedToolNames ?? []), ...addedToolNames])],
			};
		},
	};
}

/**
 * Wrap all registered tools into AgentTools.
 * Uses the runner's createContext() for consistent context across tools and event handlers.
 */
export function wrapRegisteredTools(registeredTools: RegisteredTool[], runner: ExtensionRunner): AgentTool[] {
	return registeredTools.map((tool) => wrapRegisteredTool(tool, runner));
```
Source: ice:packages/coding-agent/src/core/extensions/wrapper.ts:17-44.
None: additional fixed CLI flags, slash commands, keybindings, settings, environment variables, RPC commands or persistent files beyond the explicitly listed registration/host contracts. Each dynamically registered name is chosen by the extension.

### 6. Dependencies

`AGENT-016`, `EXT-009`, `EXT-011`

External services/binaries: none required by this host mechanism. Plugins may invoke arbitrary host I/O because they are trusted executable code; this audit did not load or run any plugin.

### 7. ice-rust today

rust:crates/ice-extensions/src/tool.rs:107-170; rust:crates/ice-extensions/src/tool.rs:174-284; rust:crates/ice-extensions/src/tool.rs:382-615. Native PluginToolAdapter injects host cwd/sessionId and an invocation capability into a private argument copy, drives execute/poll/cancel/destroy, and accepts plugin-returned addedToolNames or added_tool_names. It does not compare active tool sets around execution, so the result field alone is not automatic TS attribution. It also parses valid usage from plugin results; the separate event serializer still omits usage. Native invocation keepalive and cancellation protection are real and should be retained.

### 8. Port notes

Add host-side active-tool generation snapshots around PluginToolAdapter execution or have the authoritative SetActiveTools operation return an owned delta associated with the invocation. Merge addedToolNames only for monotonic additions and do not retry a tool after an attribution read failure. Keep the existing four-function driver, cancellation token, producer deallocation and plugin keepalive.

### 9. Size and priority

Estimated Rust implementation: **55-105 lines** (estimate, not measured source). **P1** - Required for faithful native extension behavior and for ICE-specific plugins that depend on it.

## EXT-013 - Validated command registration and collision suffix aliases

### 1. Title and status

Validated command registration and collision suffix aliases. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/extensions/loader.ts:300-315; ice:packages/coding-agent/src/core/extensions/runner.ts:664-717. registerCommand/resolveRegisteredCommands/getCommand. Tests: ice:packages/coding-agent/test/extensions-runner.test.ts:491-515 (`suffixes duplicate extension commands in insertion order`); ice:packages/coding-agent/test/extensions-runner.test.ts:467-489 (`gets command by invocation name`); ice:packages/coding-agent/test/suite/regressions/8423-extension-factory-transaction.test.ts:103-118 malformed name.

### 3. What it does

Keep duplicate command registrations callable through deterministic invocation aliases.

### 4. How it works

Commands preserve their exact names and resolve cross-extension collisions into deterministic invocation aliases. Every colliding command remains reachable rather than being silently discarded.

registerCommand requires a string of nonzero length with no whitespace or slash anywhere and a callable options.handler. It preserves case and other punctuation. Store {name, sourceInfo,...options} by the name key; untyped options can overwrite the stored fields through the spread. Same-extension same-key registration replaces the prior entry.

Resolution collects stored commands in extension/map insertion order and counts exact case-sensitive command.name. Every occurrence of a duplicate gets name:1, name:2,..., including the first. A unique name retains its spelling. If the proposed invocation was already taken, increment a suffix based on the original name until unused. Preserve order; no sorting, case folding, builtin-name rejection, all-conflict disabling or callback wrapping occurs here. getCommand matches invocationName exactly. getRegisteredCommands resets diagnostics, but this resolver emits none. Builtin alias routing and command execution are higher-layer behavior.

### 5. Interfaces

```ts
		registerCommand(name: string, options: Omit<RegisteredCommand, "name" | "sourceInfo">): void {
			runtime.assertActive();
			if (
				typeof name !== "string" ||
				name.length === 0 ||
				/[\s/]/u.test(name) ||
				typeof options?.handler !== "function"
			) {
				throw new Error("Command registration requires a nonempty name without whitespace/slashes and a handler");
			}
			extension.commands.set(name, {
				name,
				sourceInfo: extension.sourceInfo,
				...options,
			});
		},
```
Source: ice:packages/coding-agent/src/core/extensions/loader.ts:300-315.
```ts
	private resolveRegisteredCommands(): ResolvedCommand[] {
		const commands: RegisteredCommand[] = [];
		const counts = new Map<string, number>();

		for (const ext of this.extensions) {
			for (const command of ext.commands.values()) {
				commands.push(command);
				counts.set(command.name, (counts.get(command.name) ?? 0) + 1);
			}
		}

		const seen = new Map<string, number>();
		const takenInvocationNames = new Set<string>();

		return commands.map((command) => {
			const occurrence = (seen.get(command.name) ?? 0) + 1;
			seen.set(command.name, occurrence);

			let invocationName = (counts.get(command.name) ?? 0) > 1 ? `${command.name}:${occurrence}` : command.name;

			if (takenInvocationNames.has(invocationName)) {
				let suffix = occurrence;
				do {
					suffix++;
					invocationName = `${command.name}:${suffix}`;
				} while (takenInvocationNames.has(invocationName));
			}

			takenInvocationNames.add(invocationName);
			return {
				...command,
				invocationName,
			};
		});
	}

	getModelRegistry(): ModelRegistry {
		return this.modelRegistry;
	}

	getRegisteredCommands(): ResolvedCommand[] {
		this.commandDiagnostics = [];
		return this.resolveRegisteredCommands();
	}

	getCommandDiagnostics(): ResourceDiagnostic[] {
		return this.commandDiagnostics;
	}

	getCommand(name: string): ResolvedCommand | undefined {
		const commands = this.resolveRegisteredCommands();
		// An explicitly registered user command wins over a historical built-in alias.
		return commands.find((command) => command.invocationName === name);
	}
```
Source: ice:packages/coding-agent/src/core/extensions/runner.ts:664-717.
None: additional fixed CLI flags, slash commands, keybindings, settings, environment variables, RPC commands or persistent files beyond the explicitly listed registration/host contracts. Each dynamically registered name is chosen by the extension.

### 6. Dependencies

`EXT-009`, `EXT-020`

External services/binaries: none required by this host mechanism. Plugins may invoke arbitrary host I/O because they are trusted executable code; this audit did not load or run any plugin.

### 7. ice-rust today

rust:crates/ice-extensions/src/registry.rs:429-447; rust:crates/ice-extensions/src/lib.rs:418-447. Rust registration is exact-name first-wins with no equivalent name/handler validation or suffix-alias collection. A duplicate native command is discarded rather than exposed under name:2. CLI builtin conflict handling is separate; do not infer this registrar implements it.

### 8. Port notes

Represent canonical registration name separately from resolved invocation alias in ice-extensions and ice-cli CommandRegistry. Add deterministic duplicate suffix allocation, validation, async completion request/result support and source ownership. Native command handlers should receive an owned argument string plus a current command-context handle; preserve built-in conflict policy in the CLI owner.

### 9. Size and priority

Estimated Rust implementation: **90-155 lines** (estimate, not measured source). **P1** - Required for faithful native extension behavior and for ICE-specific plugins that depend on it.

## EXT-014 - Reserved shortcut union and last-extension-wins diagnostics

### 1. Title and status

Reserved shortcut union and last-extension-wins diagnostics. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/extensions/loader.ts:331-340; ice:packages/coding-agent/src/core/extensions/runner.ts:74-118; ice:packages/coding-agent/src/core/extensions/runner.ts:526-573. registerShortcut/buildBuiltinKeybindings/getShortcuts. Tests: ice:packages/coding-agent/test/extensions-runner.test.ts:275-296 (`blocks shortcuts when reserved key is also bound to non-reserved actions`); ice:packages/coding-agent/test/extensions-runner.test.ts:251-273 (`blocks shortcuts for reserved actions even when rebound`); ice:packages/coding-agent/test/extensions-runner.test.ts:348-380 (`warns when two extensions register same shortcut`). Interactive invocation: ice:packages/coding-agent/src/modes/interactive/interactive-mode.ts:2801-2863.

### 3. What it does

Protect core keys after user rebinding while allowing explicit nonreserved overrides.

### 4. How it works

Extension shortcuts respect reserved application actions and expose diagnostics for collisions. Accepted extension conflicts use later-extension precedence, and the interactive host invokes the stored callback.

registerShortcut stores its raw key and callback, same-extension same-key last-wins, without key/callback validation. getShortcuts receives already-resolved keybindings, skips undefined values, normalizes each key to lowercase and marks the exact reserved action list below. When several actions share a key, any reserved action wins regardless of iteration order. No default merging occurs in this function.

Reset diagnostics for each collection. A reserved collision emits a warning and skips the extension key. A nonreserved builtin collision warns but accepts. A duplicate extension key warns and the later extension wins. Every warning is retained; it also console.warns when no UI context is attached. Return the stored shortcut/callback, not a new guarded or prompt-blocking wrapper. Caller routing determines editor/keypress conditions. getShortcutDiagnostics returns the actual diagnostic array, not a defensive clone.

### 5. Interfaces

```ts
const RESERVED_KEYBINDINGS_FOR_EXTENSION_CONFLICTS = [
	"app.interrupt",
	"app.clear",
	"app.exit",
	"app.suspend",
	"app.thinking.cycle",
	"app.model.cycleForward",
	"app.model.cycleBackward",
	"app.model.select",
	"app.tools.expand",
	"app.tools.collapse",
	"app.tools.expandLast",
	"app.thinking.toggle",
	"app.editor.external",
	"app.message.copy",
	"app.message.followUp",
	"tui.input.submit",
	"tui.select.confirm",
	"tui.select.cancel",
	"tui.input.copy",
	"tui.editor.deleteToLineEnd",
] as const;

type BuiltInKeyBindings = Partial<Record<KeyId, { keybinding: string; restrictOverride: boolean }>>;

const buildBuiltinKeybindings = (resolvedKeybindings: KeybindingsConfig): BuiltInKeyBindings => {
	const builtinKeybindings = {} as BuiltInKeyBindings;
	for (const [keybinding, keys] of Object.entries(resolvedKeybindings)) {
		if (keys === undefined) continue;
		const keyList = Array.isArray(keys) ? keys : [keys];
		const restrictOverride = (RESERVED_KEYBINDINGS_FOR_EXTENSION_CONFLICTS as readonly string[]).includes(keybinding);
		for (const key of keyList) {
			const normalizedKey = key.toLowerCase() as KeyId;
			// If multiple actions bind the same key, the reserved action wins so extensions
			// remain blocked by reserved shortcuts regardless of iteration order.
			const existing = builtinKeybindings[normalizedKey];
			if (existing?.restrictOverride && !restrictOverride) continue;
			builtinKeybindings[normalizedKey] = {
				keybinding,
				restrictOverride,
			};
		}
	}
	return builtinKeybindings;
};
```
Source: ice:packages/coding-agent/src/core/extensions/runner.ts:74-118.
```ts
	getShortcuts(resolvedKeybindings: KeybindingsConfig): Map<KeyId, ExtensionShortcut> {
		this.shortcutDiagnostics = [];
		const builtinKeybindings = buildBuiltinKeybindings(resolvedKeybindings);
		const extensionShortcuts = new Map<KeyId, ExtensionShortcut>();

		const addDiagnostic = (message: string, extensionPath: string) => {
			this.shortcutDiagnostics.push({ type: "warning", message, path: extensionPath });
			if (!this.hasUI()) {
				console.warn(message);
			}
		};

		for (const ext of this.extensions) {
			for (const [key, shortcut] of ext.shortcuts) {
				const normalizedKey = key.toLowerCase() as KeyId;

				const builtInKeybinding = builtinKeybindings[normalizedKey];
				if (builtInKeybinding?.restrictOverride === true) {
					addDiagnostic(
						`Extension shortcut '${key}' from ${shortcut.extensionPath} conflicts with built-in shortcut. Skipping.`,
						shortcut.extensionPath,
					);
					continue;
				}

				if (builtInKeybinding?.restrictOverride === false) {
					addDiagnostic(
						`Extension shortcut conflict: '${key}' is built-in shortcut for ${builtInKeybinding.keybinding} and ${shortcut.extensionPath}. Using ${shortcut.extensionPath}.`,
						shortcut.extensionPath,
					);
				}

				const existingExtensionShortcut = extensionShortcuts.get(normalizedKey);
				if (existingExtensionShortcut) {
					addDiagnostic(
						`Extension shortcut conflict: '${key}' registered by both ${existingExtensionShortcut.extensionPath} and ${shortcut.extensionPath}. Using ${shortcut.extensionPath}.`,
						shortcut.extensionPath,
					);
				}
				extensionShortcuts.set(normalizedKey, shortcut);
			}
		}
		return extensionShortcuts;
	}

	getShortcutDiagnostics(): ResourceDiagnostic[] {
		return this.shortcutDiagnostics;
	}
```
Source: ice:packages/coding-agent/src/core/extensions/runner.ts:526-573.
None: additional fixed CLI flags, slash commands, keybindings, settings, environment variables, RPC commands or persistent files beyond the explicitly listed registration/host contracts. Each dynamically registered name is chosen by the extension.

### 6. Dependencies

`EXT-009`, `EXT-019`

External services/binaries: none required by this host mechanism. Plugins may invoke arbitrary host I/O because they are trusted executable code; this audit did not load or run any plugin.

### 7. ice-rust today

rust:crates/ice-extensions/src/registry.rs:459-475; rust:crates/ice-extensions/src/lib.rs:449-473; rust:crates/ice-cli/src/interactive_tui.rs:390-505. Native shortcut registration keeps the first exact raw key/description. Interactive dispatch derives a bare lower-case key name, checks snapshot.has_shortcut(name), sends {type:"key", key, kind, ctrl, alt, shift} to every eligible Input subscriber, and regards any CLAIMED as consumption. It does not restrict dispatch to the shortcut owner. Non-function keys require an empty draft except a previously owned release; modals/search take precedence, and press/repeat/release are represented. Registered compound strings such as ctrl+x are not matched by this bare-name membership check. This differs from TS modified-key resolution, reserved-action union, last-wins collision and direct callback contexts.

### 8. Port notes

Define a canonical modified-key descriptor in ice-plugin-sdk and map it through ice-cli keybindings before collision resolution. Add owner-bound callbacks or explicit owner routing, reserved-action union checks and surfaced diagnostics; do not use bare key membership as a substitute for ctrl/alt/shift matching. Keep native press/repeat/release support as an additional contract and retain modal focus protections.

### 9. Size and priority

Estimated Rust implementation: **95-170 lines** (estimate, not measured source). **P1** - Required for faithful native extension behavior and for ICE-specific plugins that depend on it.

## EXT-015 - Flag type validation, staged defaults and owner-only lookup

### 1. Title and status

Flag type validation, staged defaults and owner-only lookup. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/extensions/loader.ts:342-380; ice:packages/coding-agent/src/core/extensions/loader.ts:483-555; ice:packages/coding-agent/src/core/extensions/runner.ts:506-524. registerFlag/getFlag/getFlags/setFlagValue/getFlagValues. Tests: ice:packages/coding-agent/test/extensions-runner.test.ts:679-707 (`keeps first flag when two extensions register the same name`); ice:packages/coding-agent/test/extensions-runner.test.ts:709-728 (`can set flag values`); ice:packages/coding-agent/test/suite/regressions/8423-extension-factory-transaction.test.ts:103-118 default-type rejection.

### 3. What it does

Declare boolean/string flags with matching defaults and expose only flags registered by the calling extension.

### 4. How it works

Extensions declare boolean or string flags with optional typed defaults. Reads are limited to names registered by that extension, even though values are shared across the runtime.

Reject types other than boolean/string or a supplied default whose typeof differs from the declared type. Name syntax is not checked here. Store the descriptor with name/path plus options; same-extension map replacement is last-wins, but only the first absent local/shared default is installed. During initialization flagValues is a snapshot Map and commit copies only names absent from shared runtime, preserving earlier values.

getFlag first checks this extension.flags.has(name); an unregistered name returns undefined even if the shared map contains it. Then return the exact shared value. Runner getFlags keeps the first descriptor per exact name; setFlagValue writes without declaration/type validation, while getFlagValues returns a Map copy. Dash stripping and CLI coercion are separate. No file/settings persistence is introduced.

### 5. Interfaces

```ts
		registerFlag(
			name: string,
			options: { description?: string; type: "boolean" | "string"; default?: boolean | string },
		): void {
			runtime.assertActive();
			if (
				(options.type !== "boolean" && options.type !== "string") ||
				(options.default !== undefined && typeof options.default !== options.type)
			) {
				throw new Error(`Flag '${name}' default must match its declared type`);
			}
			extension.flags.set(name, { name, extensionPath: extension.path, ...options });
			if (options.default !== undefined && !runtime.flagValues.has(name)) {
				runtime.flagValues.set(name, options.default);
			}
		},

		registerMessageRenderer<T>(customType: string, renderer: MessageRenderer<T>): void {
			runtime.assertActive();
			extension.messageRenderers.set(customType, renderer as MessageRenderer);
		},

		registerMarkdownTransformer(transformer: MarkdownTransformer): void {
			runtime.assertActive();
			extension.markdownTransformer = transformer;
		},

		registerEntryRenderer<T>(customType: string, renderer: EntryRenderer<T>): void {
			runtime.assertActive();
			extension.entryRenderers ??= new Map();
			extension.entryRenderers.set(customType, renderer as EntryRenderer);
		},

		// Flag access - checks extension registered it, reads from runtime
		getFlag(name: string): boolean | string | undefined {
			runtime.assertActive();
			if (!extension.flags.has(name)) return undefined;
			return runtime.flagValues.get(name);
		},
```
Source: ice:packages/coding-agent/src/core/extensions/loader.ts:342-380.
```ts
	getFlags(): Map<string, ExtensionFlag> {
		const allFlags = new Map<string, ExtensionFlag>();
		for (const ext of this.extensions) {
			for (const [name, flag] of ext.flags) {
				if (!allFlags.has(name)) {
					allFlags.set(name, flag);
				}
			}
		}
		return allFlags;
	}

	setFlagValue(name: string, value: boolean | string): void {
		this.runtime.flagValues.set(name, value);
	}

	getFlagValues(): Map<string, boolean | string> {
		return new Map(this.runtime.flagValues);
	}
```
Source: ice:packages/coding-agent/src/core/extensions/runner.ts:506-524.
None: additional fixed CLI flags, slash commands, keybindings, settings, environment variables, RPC commands or persistent files beyond the explicitly listed registration/host contracts. Each dynamically registered name is chosen by the extension.

### 6. Dependencies

`EXT-006`, `EXT-009`

External services/binaries: none required by this host mechanism. Plugins may invoke arbitrary host I/O because they are trusted executable code; this audit did not load or run any plugin.

### 7. ice-rust today

rust:crates/ice-extensions/src/registry.rs:449-457; rust:crates/ice-extensions/src/lib.rs:476-498; rust:crates/ice-cli/src/extensions_actions.rs:423-428. Native declarations contain name/description only and reject blank names, leading dash, equals and literal spaces. They have no declared type/default. GetCliFlag=16 returns {value: bool|string|null} from the shared parsed CLI map for any requested name without checking the calling plugin's registrations. Thus lookup exists but is owner-independent, with null for absence and no TS typed default contract.

### 8. Port notes

Append flag type and optional default to the native declaration schema and validate both before commit. Make GetCliFlag owner-aware, preserving absence separately from JSON null and distinguishing parsed user values from defaults. Reuse ice-cli argument parsing; define duplicate declarations and conversion diagnostics before updating help/introspection.

### 9. Size and priority

Estimated Rust implementation: **65-120 lines** (estimate, not measured source). **P1** - Required for faithful native extension behavior and for ICE-specific plugins that depend on it.

## EXT-016 - Named settings sections without automatic persistence

### 1. Title and status

Named settings sections without automatic persistence. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/core/extensions/loader.ts:317-329; ice:packages/coding-agent/src/core/extensions/runner.ts:719-721. registerSettings/getRegisteredSettings. Consumer COG-057 exercises this seam; failed-registration isolation is covered by ice:packages/coding-agent/test/suite/regressions/8423-extension-factory-transaction.test.ts:9-47 (`discards failed provider/default/subscription registrations and revokes the captured API`).

### 3. What it does

Contribute ordered settings descriptors and a change callback to the host settings UI.

### 4. How it works

An extension can contribute a named settings section with items and an onChange callback. Registration stores the supplied descriptor; the extension and settings UI own values and persistence.

registerSettings(name, options) lazily creates a settings Map, then stores {name, sourceInfo,...options} under the supplied name after active check. There is no name/item/callback validation, descriptor clone or cross-extension deduplication. Same-extension same-key last-wins; getRegisteredSettings concatenates all maps in extension order. This API calls the identifier name, not namespace, although consumers use it as a UI namespace.

The loader does not load current values, refresh stale descriptor currentValue fields, invoke onChange, await its persistence or create a nested core settings key. Those are owned by the extension and mode UI. Options spread can overwrite identity fields from an untyped caller. A retained callback needs its own guarded host accesses after replacement. No on-disk settings write occurs here.

### 5. Interfaces

```ts
		registerSettings(name: string, options: Omit<RegisteredSettings, "name" | "sourceInfo">): void {
			runtime.assertActive();
			let settings = extension.settings;
			if (!settings) {
				settings = new Map();
				extension.settings = settings;
			}
			settings.set(name, {
				name,
				sourceInfo: extension.sourceInfo,
				...options,
			});
		},
```
Source: ice:packages/coding-agent/src/core/extensions/loader.ts:317-329.
```ts
	getRegisteredSettings(): RegisteredSettings[] {
		return this.extensions.flatMap((extension) => Array.from(extension.settings?.values() ?? []));
	}
```
Source: ice:packages/coding-agent/src/core/extensions/runner.ts:719-721.
None: additional fixed CLI flags, slash commands, keybindings, settings, environment variables, RPC commands or persistent files beyond the explicitly listed registration/host contracts. Each dynamically registered name is chosen by the extension.

### 6. Dependencies

`EXT-009`

External services/binaries: none required by this host mechanism. Plugins may invoke arbitrary host I/O because they are trusted executable code; this audit did not load or run any plugin.

### 7. ice-rust today

rust:crates/ice-extensions/src/registry.rs:335-381; rust:crates/ice-plugin-sdk/src/lib.rs:1010-1074. Native registries/actions lack this ordered settings-section registration and change callback. Generic dialogs/status text are not settings-menu participation.

### 8. Port notes

Add a versioned settings-section declaration and owner-bound change request/result to the SDK, with ice-cli settings UI rendering native data rather than arbitrary plugin components. The extension should own its persisted values unless a separate settings namespace is explicitly introduced. Handle duplicate names, stale section refresh and callback failures without presenting uncommitted values as saved.

### 9. Size and priority

Estimated Rust implementation: **120-210 lines** (estimate, not measured source). **P1** - Cognee/provider configuration plugins require a real native settings-section seam.

## EXT-017 - Asynchronous shared event bus with staged listener ownership

### 1. Title and status

Asynchronous shared event bus with staged listener ownership. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/event-bus.ts:1-33; ice:packages/coding-agent/src/core/extensions/loader.ts:514-562. createEventBus/scopedBus/extension.dispose. Tests: ice:packages/coding-agent/test/suite/regressions/8423-extension-factory-transaction.test.ts:9-47 (`discards failed provider/default/subscription registrations and revokes the captured API`); ice:packages/coding-agent/test/suite/regressions/8423-extension-factory-transaction.test.ts:49-70 (`does not leak a bus listener after session invalidation`).

### 3. What it does

Exchange arbitrary in-process data while isolating handler failures and cleaning up a factory’s own listeners.

### 4. How it works

Extensions share a named event bus for asynchronous cooperation. Factory-owned subscriptions are staged and disposed, while emission remains immediate and does not wait for listeners.

createEventBus uses Node EventEmitter. on wraps a callback in an async try/await/catch that console.errors failures. emit is synchronous EventEmitter.emit returning void: it does not await callbacks, clone data, apply a schema, queue, cap or persist anything. Synchronous callback work before its first await runs during emit. All callbacks receive the same object. clear removes all listeners.

A factory receives a Proxy over the shared bus. emit checks scoped activity and is immediate even during initialization. on stages an attach closure until successful commit; its idempotent-ish unsubscribe marks cancelled, calls the stored detach when any, and removes itself from the ownership Set. Cancellation before commit prevents attachment. After initialization attachments are immediate. dispose sets disposed, unsubscribes tracked listeners and clears pending attaches; it cannot cancel already-running callbacks. Non-on/emit properties are reflected unchanged, so shared symbol capability registries and clear are not scoped or separately guarded. No retry/turn scheduling is performed.

### 5. Interfaces

```ts
export interface EventBus {
	emit(channel: string, data: unknown): void;
	on(channel: string, handler: (data: unknown) => void): () => void;
}

export interface EventBusController extends EventBus {
	clear(): void;
}

export function createEventBus(): EventBusController {
	const emitter = new EventEmitter();
	return {
		emit: (channel, data) => {
			emitter.emit(channel, data);
		},
		on: (channel, handler) => {
			const safeHandler = async (data: unknown) => {
				try {
					await handler(data);
				} catch (err) {
					console.error(`Event handler error (${channel}):`, err);
				}
			};
			emitter.on(channel, safeHandler);
			return () => emitter.off(channel, safeHandler);
		},
		clear: () => {
			emitter.removeAllListeners();
		},
	};
}
```
Source: ice:packages/coding-agent/src/core/event-bus.ts:3-33.
```ts
	const scopedBus = new Proxy(eventBus, {
		get(target, key) {
			if (key === "emit")
				return (channel: string, data: unknown) => {
					assertActive();
					eventBus.emit(channel, data);
				};
			if (key === "on")
				return (channel: string, handler: (data: unknown) => void) => {
					assertActive();
					let cancelled = false;
					let detach: (() => void) | undefined;
					const unsubscribe = () => {
						cancelled = true;
						detach?.();
						subscriptions.delete(unsubscribe);
					};
					const attach = () => {
						if (cancelled) return;
						detach = eventBus.on(channel, handler);
						subscriptions.add(unsubscribe);
					};
					if (initializing) pendingSubscriptions.push(attach);
					else attach();
					return unsubscribe;
				};
			return Reflect.get(target, key);
		},
	});
	extension.dispose = () => {
		disposed = true;
		for (const unsubscribe of [...subscriptions]) unsubscribe();
		pendingSubscriptions.length = 0;
	};
```
Source: ice:packages/coding-agent/src/core/extensions/loader.ts:514-547.
None: additional fixed CLI flags, slash commands, keybindings, settings, environment variables, RPC commands or persistent files beyond the explicitly listed registration/host contracts. Each dynamically registered name is chosen by the extension.

### 6. Dependencies

`EXT-006`, `EXT-009`, `EXT-010`

External services/binaries: none required by this host mechanism. Plugins may invoke arbitrary host I/O because they are trusted executable code; this audit did not load or run any plugin.

### 7. ice-rust today

rust:crates/ice-extensions/src/lib.rs:769-774; rust:crates/ice-extensions/src/registry.rs:478-502; rust:crates/ice-plugin-sdk/src/lib.rs:1010-1074. The native vtable exposes dispatch_event, but trampoline_dispatch_event currently returns 0 and drops the event without forwarding it to a host bus. Fixed EventTags and runtime actions are not an arbitrary named-data bus or Symbol.for capability registry. No dynamic named-bus subscription/unsubscribe or shared service registry exists in the fully read SDK/registry/host bridge.

### 8. Port notes

Add an owner-scoped arbitrary-event/capability registry to ice-extensions with named JSON payloads and opaque subscription handles. Implement queue/backpressure or synchronous fan-out deliberately, then document the difference from fire-and-forget EventEmitter semantics. For service capabilities use typed host handles, not unvalidated shared foreign pointers; dispose subscriptions on candidate failure and reload.

### 9. Size and priority

Estimated Rust implementation: **100-180 lines** (estimate, not measured source). **P1** - Required for faithful native extension behavior and for ICE-specific plugins that depend on it.

## EXT-018 - Overloaded provider registration, grouped queue flush and refresh callbacks

### 1. Title and status

Overloaded provider registration, grouped queue flush and refresh callbacks. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/extensions/loader.ts:236-249; ice:packages/coding-agent/src/core/extensions/loader.ts:453-466; ice:packages/coding-agent/src/core/extensions/runner.ts:385-444. registerProvider overload, runtime registerNativeProvider/unregisterProvider, bindCore. Tests: ice:packages/coding-agent/test/extensions-runner.test.ts:868-889 (`bindCore ignores invalid queued registrations and reports extension error`); ice:packages/coding-agent/test/extensions-runner.test.ts:891-913 (`pre-bind unregister removes all queued registrations for a provider`); ice:packages/coding-agent/test/extensions-runner.test.ts:915-936 (`post-bind register and unregister take effect immediately`).

### 3. What it does

Register a legacy configuration or native Provider object through the existing model registry.

### 4. How it works

Provider registration supports both compatibility configurations and native TypeScript Provider objects. Registrations queue before binding and become immediate host calls afterward.

ExtensionAPI exposes one overloaded registerProvider: a string requires a truthy config and forwards name/config/extension.path; a nonstring forwards the native Provider object and extension.path. There is no separate public registerNativeProvider method and no public refreshToken method. OAuth refreshToken is a callback supplied by the provider configuration, not a host action. The shared runtime has separate pending legacy/native registration arrays. Pre-bind unregister removes every matching name/id from both without owner restriction. Factory staging is EXT-006.

At bindCore, flush every pending legacy registration first, then every native registration. For each use optional providerActions override or modelRegistry.registerProvider (overloaded for native objects). Catch each queued failure, emit ExtensionError with the stored extensionPath and event register_provider (for both kinds), then continue; there is no special errorCode. Clear each array after its loop. Interleaved factory registration calls therefore need not stay interleaved across the two queue kinds. An error-listener exception can interrupt flushing before the array is cleared.

After binding replace runtime register/unregister with immediate forwarding functions. These do not catch registry errors: post-bind failures propagate to the calling extension. No /reload is required for success. Loader forwarding does not itself implement registry validation, catalog refresh, OAuth/key locking, persistence or provider retries; those belong to the existing model/auth owner.

### 5. Interfaces

```ts
export interface ProviderConfig {
	/** Display name for the provider in UI. */
	name?: string;
	/** Base URL for the API endpoint. Required when defining models. */
	baseUrl?: string;
	/** API key literal, env interpolation ($ENV_VAR or ${ENV_VAR}), or leading !command. Required when defining models (unless oauth provided). */
	apiKey?: string;
	/** API type. Required at provider or model level when defining models. */
	api?: Api;
	/**
	 * Optional streamSimple handler for custom APIs.
	 * Implementations must invoke `options.onPayload` before sending the provider request and use any
	 * returned replacement payload. They must invoke `options.onResponse` after receiving the response
	 * and before consuming its body, matching built-in providers.
	 */
	streamSimple?: (model: Model<Api>, context: Context, options?: SimpleStreamOptions) => AssistantMessageEventStream;
	/** Custom headers to include in requests. */
	headers?: Record<string, string>;
	/** If true, adds Authorization: Bearer header with the resolved API key. */
	authHeader?: boolean;
	/** Models to register. If provided, replaces all existing models for this provider. */
	models?: ProviderModelConfig[];
	/**
	 * Refresh this provider's model list. The returned list replaces extension-provided models.
	 * Use context.publish({ persist: entry }) when the catalog should persist across sessions.
	 */
	refreshModels?(context: RefreshModelsContext): Promise<ProviderModelConfig[]>;
	/** OAuth provider for /login support. The `id` is set automatically from the provider name. */
	oauth?: {
		/** Display name for the provider in login UI. */
		name: string;
		/** @deprecated Retained for source compatibility; canonical auth flows ignore it. */
		usesCallbackServer?: boolean;
		/** Run the login flow, return credentials to persist. */
		login(callbacks: OAuthLoginCallbacks): Promise<OAuthCredentials>;
		/** Refresh expired credentials, return updated credentials to persist. */
		refreshToken(credentials: OAuthCredentials, signal: AbortSignal): Promise<OAuthCredentials>;
		/** Convert credentials to API key string for the provider. */
		getApiKey(credentials: OAuthCredentials): string;
		/** Legacy synchronous credential-dependent model projection. */
		modifyModels?(models: Model<Api>[], credentials: OAuthCredentials): Model<Api>[];
	};
}
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:1514-1556.
```ts
export interface ProviderModelConfig {
	/** Model ID (e.g., "claude-sonnet-4-20250514"). */
	id: string;
	/** Display name (e.g., "Claude 4 Sonnet"). */
	name: string;
	/** API type override for this model. */
	api?: Api;
	/** API endpoint URL override for this model. */
	baseUrl?: string;
	/** Whether the model supports extended thinking. */
	reasoning: boolean;
	/** Maps ice thinking levels to provider/model-specific values; null marks a level unsupported. */
	thinkingLevelMap?: Model<Api>["thinkingLevelMap"];
	/** Supported input types. */
	input: ("text" | "image")[];
	/** Per-million-token cost rates and optional request-wide input pricing tiers. */
	cost: Model<Api>["cost"];
	/** Maximum context window size in tokens. */
	contextWindow: number;
	/** Maximum output tokens. */
	maxTokens: number;
	/** Custom headers for this model. */
	headers?: Record<string, string>;
	/** OpenAI compatibility settings. */
	compat?: Model<Api>["compat"];
}
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:1559-1584.
```ts
		registerProvider(providerOrName: Provider | string, config?: ProviderConfig) {
			runtime.assertActive();
			if (typeof providerOrName === "string") {
				if (!config) throw new Error("Provider config is required when registering by name");
				runtime.registerProvider(providerOrName, config, extension.path);
				return;
			}
			runtime.registerNativeProvider(providerOrName, extension.path);
		},

		unregisterProvider(name: string) {
			runtime.assertActive();
			runtime.unregisterProvider(name, extension.path);
		},
```
Source: ice:packages/coding-agent/src/core/extensions/loader.ts:453-466.
```ts
		// Flush provider registrations queued during extension loading
		for (const { name, config, extensionPath } of this.runtime.pendingProviderRegistrations) {
			try {
				if (providerActions?.registerProvider) {
					providerActions.registerProvider(name, config);
				} else {
					this.modelRegistry.registerProvider(name, config);
				}
			} catch (err) {
				this.emitError({
					extensionPath,
					event: "register_provider",
					error: err instanceof Error ? err.message : String(err),
					stack: err instanceof Error ? err.stack : undefined,
				});
			}
		}
		this.runtime.pendingProviderRegistrations = [];
		for (const { provider, extensionPath } of this.runtime.pendingNativeProviderRegistrations) {
			try {
				if (providerActions?.registerNativeProvider) {
					providerActions.registerNativeProvider(provider);
				} else {
					this.modelRegistry.registerProvider(provider);
				}
			} catch (err) {
				this.emitError({
					extensionPath,
					event: "register_provider",
					error: err instanceof Error ? err.message : String(err),
					stack: err instanceof Error ? err.stack : undefined,
				});
			}
		}
		this.runtime.pendingNativeProviderRegistrations = [];

		// From this point on, provider registration/unregistration takes effect immediately
		// without requiring a /reload.
		this.runtime.registerProvider = (name, config) => {
			if (providerActions?.registerProvider) {
				providerActions.registerProvider(name, config);
				return;
			}
			this.modelRegistry.registerProvider(name, config);
		};
		this.runtime.registerNativeProvider = (provider) => {
			if (providerActions?.registerNativeProvider) {
				providerActions.registerNativeProvider(provider);
				return;
			}
			this.modelRegistry.registerProvider(provider);
		};
		this.runtime.unregisterProvider = (name) => {
			if (providerActions?.unregisterProvider) {
				providerActions.unregisterProvider(name);
				return;
			}
			this.modelRegistry.unregisterProvider(name);
		};
	}
```
Source: ice:packages/coding-agent/src/core/extensions/runner.ts:385-444.
No fixed CLI flag, environment variable, command or file is introduced by this registry seam.

### 6. Dependencies

`EXT-006`, `EXT-009`

External services/binaries: none required by this host mechanism. Plugins may invoke arbitrary host I/O because they are trusted executable code; this audit did not load or run any plugin.

Final dependency classification. Shared contract references: `AUTH-004`, `AUTH-006`, `AUTH-008`, `AUTH-009`, `AI-002`, `AI-003`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

rust:crates/ice-extensions/src/lib.rs:569-691; rust:crates/ice-extensions/src/registry.rs:622-704; rust:crates/ice-plugin-sdk/src/lib.rs:1321-1332; rust:crates/ice-plugin-sdk/src/lib.rs:1431-1510. Rust supports native one-shot and streaming providers, OAuth callbacks, and model manifests at registration time. Within one registry a same-owner streaming provider can upgrade its one-shot registration, while first registered streaming providers and cross-plugin merge precedence remain first-wins. Registrars require the current TLS registration scope. There is no corresponding live register/replace/unregister runtime action or TS Provider object bridge. This entry covers registration lifecycle; provider/OAuth transport behavior belongs to its area.

### 8. Port notes

Add explicit owner-scoped register/replace/unregister provider actions through the existing native provider registry, retaining streaming and OAuth lifetimes. Commit catalog changes atomically and refresh the active model metadata through ice-cli without replacing the agent loop. Preserve separate compatibility/native queue order only if that compatibility mode is supported; cross-owner unregister and credential exposure require an explicit documented policy.

### 9. Size and priority

Estimated Rust implementation: **140-250 lines** (estimate, not measured source). **P1** - Required for faithful native extension behavior and for ICE-specific plugins that depend on it.

## EXT-019 - Live extension context and host control getters

### 1. Title and status

Live extension context and host control getters. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/extensions/runner.ts:302-329; ice:packages/coding-agent/src/core/extensions/runner.ts:365-383; ice:packages/coding-agent/src/core/extensions/runner.ts:740-837; ice:packages/coding-agent/src/core/extensions/types.ts:315-391. createContext/ExtensionContext. Tests: ice:packages/coding-agent/test/extensions-runner.test.ts:107-119 (`reflects the getScopedModels context action on ctx.scopedModels`); ice:packages/coding-agent/test/extensions-runner.test.ts:123-133 (`reflects the bound compaction state live`); ice:packages/coding-agent/test/extensions-runner.test.ts:547-557 (`exposes project trust state on ExtensionContext`); ice:packages/coding-agent/test/extensions-runner.test.ts:519-535 (`exposes the current abort signal on ExtensionContext`).

Exported symbols: `ShutdownHandler` (ice:packages/coding-agent/src/core/extensions/runner.ts:206).

### 3. What it does

Give handlers a lazily read view of the current session, model, queue, cancellation and compaction state.

### 4. How it works

The base extension context exposes live queries and controls through guarded getters and methods. It returns host objects directly and therefore cannot revoke an object already obtained before invalidation.

createContext returns guarded property getters for ui/mode/hasUI/cwd/sessionManager/settingsManager/modelRegistry/model/scopedModels/thinkingLevel/signal. It captures the getModel/getScopedModels function references when constructing the context; each access invokes those captured functions after checking staleness. Settings manager may be undefined; the getter does not throw merely because it is absent. Other getters read the current runner/runtime fields. Returned objects/arrays are not cloned or frozen here, so TypeScript readonly is not an independent security boundary.

Methods assert active then forward isIdle/isCompacting/isProjectTrusted, abort, stopAfterTurn, compactAfterTurn, hasPendingMessages/hasPendingSteering, shutdown, getContextUsage, compact and getSystemPrompt. There is no implicit await/queue in void controls. Before binding defaults can report trusted/idle true with no model. The context does not itself write session entries, enforce mode restrictions, cancel processes or compact; the bound owner supplies those semantics. Calling a saved getter after invalidation fails, but an object already obtained through it is not automatically revoked.

### 5. Interfaces

```ts
export interface ExtensionContext {
	/** UI methods for user interaction */
	ui: ExtensionUIContext;
	/** Current run mode. Use "tui" to guard terminal-only UI such as custom components. */
	mode: ExtensionMode;
	/** Whether dialog-capable UI is available (true in TUI and RPC modes) */
	hasUI: boolean;
	/** Current working directory */
	cwd: string;
	/** Session manager (read-only) */
	sessionManager: ReadonlySessionManager;
	/** Current scoped settings owner. Optional for lightweight extension test hosts. */
	settingsManager?: SettingsManager;
	/** Model registry for API key resolution */
	modelRegistry: ModelRegistry;
	/** Current model (may be undefined) */
	model: Model<any> | undefined;
	/** Models scoped to this session (resolved from `--models` /
	 *  `enabledModels` settings against the available catalogue). Same set
	 *  the `/scoped-models` command shows. Empty when no scoping is
	 *  configured (all available models are usable). Read-only snapshot. */
	scopedModels: readonly ScopedModel[];
	/** Current thinking level, when provided by the session runtime. */
	thinkingLevel?: ThinkingLevel;
	/** Whether the agent is idle (not streaming) */
	isIdle(): boolean;
	/** Whether compaction or branch summarization is running. Compaction aborts the active run first. */
	isCompacting(): boolean;
	/** Whether project-local trust is active for this context. */
	isProjectTrusted(): boolean;
	/** The current abort signal, or undefined when the agent is not streaming. */
	signal: AbortSignal | undefined;
	/** Abort the current agent operation */
	abort(): void;
	/** Finish the current tool turn normally, then stop before another model request. */
	stopAfterTurn(): void;
	/**
	 * Compact at the current tool-turn boundary without aborting the run, then resume or pause.
	 * Call from turn_end; the request is dropped when no further tool-turn boundary occurs.
	 */
	compactAfterTurn(mode: Exclude<MidRunCompaction, "off">): void;
	/** Whether there are queued messages waiting */
	hasPendingMessages(): boolean;
	/**
	 * Whether steering messages (user steers and steer-delivered custom messages) are queued.
	 * Blocking waits should end on this, not on hasPendingMessages(): follow-ups are delivered
	 * only when the agent stops, so waking a wait for them cannot deliver them.
	 */
	hasPendingSteering(): boolean;
	/** Gracefully shutdown ice and exit. Available in all contexts. */
	shutdown(): void;
	/** Get current context usage for the active model. */
	getContextUsage(): ContextUsage | undefined;
	/** Trigger compaction without awaiting completion. */
	compact(options?: CompactOptions): void;
	/** Get the current effective system prompt. */
	getSystemPrompt(): string;
}
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:334-391.
```ts
export interface ContextUsage {
	/** Estimated context tokens, or null if unknown (e.g. right after compaction, before next LLM response). */
	tokens: number | null;
	contextWindow: number;
	/** Context usage as percentage of context window, or null if tokens is unknown. */
	percent: number | null;
}
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:315-321.
```ts
export interface CompactOptions {
	customInstructions?: string;
	onComplete?: (result: CompactionResult) => void;
	onError?: (error: Error) => void;
}
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:323-327.
None: additional fixed CLI flags, slash commands, keybindings, settings, environment variables, RPC commands or persistent files beyond the explicitly listed registration/host contracts. Each dynamically registered name is chosen by the extension.

### 6. Dependencies

`EXT-009`, `EXT-010`

External services/binaries: none required by this host mechanism. Plugins may invoke arbitrary host I/O because they are trusted executable code; this audit did not load or run any plugin.

Final dependency classification. Shared contract references: `SESSION-021`, `SESSION-034`, `SESSION-037`, `AGENT-021`, `AGENT-027`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

rust:crates/ice-harness/src/agent_harness/child.rs:54-68; rust:crates/ice-cli/src/extensions_actions.rs:388-405; rust:crates/ice-extensions/src/actions.rs:984-1040. GetExecutionContext returns an owned JSON snapshot with model {id, provider, api}, thinkingLevel, tools and childAgent capability, then host cwd/sessionId and any effective child invocation scope. It deliberately excludes endpoints, headers and credential bindings. GetSystemPrompt and model/thinking actions exist separately. There is no full live sessionManager/settingsManager/modelRegistry object or every TS context control; an already returned JSON snapshot does not become live after a model/session change.

### 8. Port notes

Expand GetExecutionContext into a versioned, owned snapshot with session identity/revision and explicit absent values; add separate queries for context usage, pending steering, idle/compaction/trust and scoped models. Add cancellation/stop-after-turn/compact-after-turn controls at the authoritative harness lane. Never serialize credential stores, live Rust object pointers or settings managers into plugin JSON.

### 9. Size and priority

Estimated Rust implementation: **170-290 lines** (estimate, not measured source). **P1** - Required for faithful native extension behavior and for ICE-specific plugins that depend on it.

## EXT-020 - Command-only session replacement context and callbacks

### 1. Title and status

Command-only session replacement context and callbacks. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/extensions/runner.ts:446-464; ice:packages/coding-agent/src/core/extensions/runner.ts:840-876; ice:packages/coding-agent/src/core/extensions/types.ts:397-452. createCommandContext/bindCommandContext. Test: ice:packages/coding-agent/test/extensions-runner.test.ts:940-960 (`passes fork options through to the bound handler`). Replacement execution/order remains a session/mode integration dependency; no callback was invoked during the audit. Replacement: ice:packages/coding-agent/src/core/agent-session.ts:4012-4026; ice:packages/coding-agent/src/core/agent-session-runtime.ts:212-292.

Exported symbols: `SwitchSessionHandler`, `ReloadHandler` (ice:packages/coding-agent/src/core/extensions/runner.ts:199; ice:packages/coding-agent/src/core/extensions/runner.ts:204).

### 3. What it does

Expose session-changing actions only through the richer command context and return a fresh context after successful replacement.

### 4. How it works

Command handlers receive additional session-control methods beyond the base context. Successful session replacement can run a callback with a fresh replacement context and awaitable message methods.

createCommandContext copies property descriptors from the live base context, preserving lazy stale checks. Add guarded getSystemPromptOptions, waitForIdle, newSession, fork, navigateTree, switchSession and reload forwarding methods. The runner does not itself wait before each action; safe command admission and action implementations are supplied by the mode. Unbound placeholder actions return {cancelled:false} or resolve void without performing a session change, so a lightweight test host must not mistake them for implementation.

newSession accepts parentSession/setup/withSession; fork requires entryId and optional before|at position/withSession; switchSession accepts a session path and withSession. Replacement callbacks receive ReplacedSessionContext, which extends ExtensionCommandContext with Promise-returning sendMessage and sendUserMessage methods; there is no API getter. Tree navigation passes summarize/customInstructions/replaceInstructions/label. Preserve callback ordering in the actual session owner; this wrapper neither runs callbacks itself nor serializes functions into JSON. A retained old context is invalidated by that owner, not by the act of constructing a command context.

The replacement implementation copies property descriptors from a fresh command context, then binds its two send methods to the new AgentSession (ice:packages/coding-agent/src/core/agent-session.ts:4012-4026). AgentSessionRuntime first rebinds the replacement UI/host and then awaits withSession; a rebind failure makes public session access unavailable, while a later withSession failure propagates after rebinding (ice:packages/coding-agent/src/core/agent-session-runtime.ts:212-222).

### 5. Interfaces

```ts
export interface ExtensionCommandContext extends ExtensionContext {
	/** Get the current base system-prompt construction options. */
	getSystemPromptOptions(): BuildSystemPromptOptions;

	/** Wait for the agent to finish streaming */
	waitForIdle(): Promise<void>;

	/** Start a new session, optionally with initialization. */
	newSession(options?: {
		parentSession?: string;
		setup?: (sessionManager: SessionManager) => Promise<void>;
		withSession?: (ctx: ReplacedSessionContext) => Promise<void>;
	}): Promise<{ cancelled: boolean }>;

	/** Fork from a specific entry, creating a new session file. */
	fork(
		entryId: string,
		options?: { position?: "before" | "at"; withSession?: (ctx: ReplacedSessionContext) => Promise<void> },
	): Promise<{ cancelled: boolean }>;

	/** Navigate to a different point in the session tree. */
	navigateTree(
		targetId: string,
		options?: { summarize?: boolean; customInstructions?: string; replaceInstructions?: boolean; label?: string },
	): Promise<{ cancelled: boolean }>;

	/** Switch to a different session file. */
	switchSession(
		sessionPath: string,
		options?: { withSession?: (ctx: ReplacedSessionContext) => Promise<void> },
	): Promise<{ cancelled: boolean }>;

	/** Reload extensions, skills, prompts, themes, and context files. */
	reload(): Promise<void>;
}
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:397-431.
```ts
export interface ReplacedSessionContext extends ExtensionCommandContext {
	sendMessage<T = unknown>(
		message: Pick<CustomMessage<T>, "customType" | "content" | "display" | "details">,
		options?: { triggerTurn?: boolean; deliverAs?: "steer" | "followUp" | "nextTurn" },
	): Promise<void>;

	sendUserMessage(
		content: string | (TextContent | ImageContent)[],
		options?: { deliverAs?: "steer" | "followUp" },
	): Promise<void>;
}
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:438-448.
None: additional fixed CLI flags, slash commands, keybindings, settings, environment variables, RPC commands or persistent files beyond the explicitly listed registration/host contracts. Each dynamically registered name is chosen by the extension.

### 6. Dependencies

`EXT-009`, `EXT-010`, `EXT-019`, `SESSION-021`, `SESSION-026`, `SESSION-035`

External services/binaries: none required by this host mechanism. Plugins may invoke arbitrary host I/O because they are trusted executable code; this audit did not load or run any plugin.

### 7. ice-rust today

rust:crates/ice-cli/src/extensions_actions.rs:600-665. Rust NewSession creates and activates a JSONL session but ignores supplied options. Fork ignores args and forks the current storage rather than a requested entry/position. SwitchSession accepts id, not the TS path/callback contract. NavigateTree accepts targetId/summarize/customInstructions/label but not replaceInstructions, returning runId/status instead of cancelled. Reload without a bridge callback reaches an error fallback. With a reload callback, rust:crates/ice-extensions/src/actions.rs:1198-1205 returns null after the callback, while reload_callback_from_mailbox (rust:crates/ice-extensions/src/actions.rs:860-868) ignores missing/closed receiver failure; success then means neither completion nor necessarily a queued reload. Native command actions are thus partial, not absent. No setup/withSession function transport or waitForIdle action exists.

### 8. Port notes

Extend ice-cli session action DTOs for target entry/position, target path or stable ID, parent session and replacement completion. Represent setup/withSession as a post-rebind callback or follow-up request using a fresh context handle, not serialized functions. Return explicit cancelled/result states and make Reload distinguish queued, unavailable and completed; coordinate ordering with the session owner before revoking the caller.

### 9. Size and priority

Estimated Rust implementation: **190-330 lines** (estimate, not measured source). **P1** - Required for faithful native extension behavior and for ICE-specific plugins that depend on it.

## EXT-021 - Error notifications and post-completion slow-handler observation

### 1. Title and status

Error notifications and post-completion slow-handler observation. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/extensions/runner.ts:170-195; ice:packages/coding-agent/src/core/extensions/runner.ts:591-629; ice:packages/coding-agent/src/core/extensions/types.ts:1805-1810. onError/emitError/getActiveHandler/onSlowHandler/runPromptBlockingHandler. Existing runner error-emission tests cover listener delivery; the exact 1000 ms tracking/exception boundary is explicit source.

### 3. What it does

Attribute extension failures and identify slow prompt-blocking callbacks without silently timing them out.

### 4. How it works

The runner reports extension failures and measures selected prompt-blocking handlers after they settle. Its slow-handler threshold is diagnostic and does not cancel slow work.

onError/onSlowHandler insert listeners into Sets and return removal closures. emitError iterates synchronously without catching listener exceptions; a failing error reporter can escape a dispatcher’s catch path and stop later handlers. getActiveHandler returns the stored object, not a copy. runPromptBlockingHandler overwrites the single active slot with event/path/Date.now, awaits the callback and in finally clears only if it still owns the slot. It measures elapsed time after settlement; at >=1000 ms emit a slow-handler event to every listener. Listener failures are not caught and can replace the callback outcome.

Only input, before_agent_start, agent_start and turn_start use this tracker. It is not a timeout, queue lock or general count of active callbacks; overlapping/nested calls can overwrite the slot. Other event errors become ExtensionError records only through their specific dispatchers. No persistence, retry, text redaction or message cap is supplied here; observability/notice consumers own those policies.

### 5. Interfaces

```ts
export interface ExtensionError {
	extensionPath: string;
	event: string;
	error: string;
	stack?: string;
}
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:1805-1810.
```ts
 * and `turn_start` handlers before the submitted prompt is recorded and sent to the model.
 */
export interface ActiveExtensionHandler {
	extensionPath: string;
	event: "input" | "before_agent_start" | "agent_start" | "turn_start";
	startedAt: number;
}

export type SlowExtensionHandlerListener = (handler: ActiveExtensionHandler & { durationMs: number }) => void;

/** Prompt-blocking handlers at or above this duration are reported to slow-handler listeners. */
export const SLOW_EXTENSION_HANDLER_MS = 1000;

export type NewSessionHandler = (options?: {
	parentSession?: string;
	setup?: (sessionManager: SessionManager) => Promise<void>;
	withSession?: (ctx: ReplacedSessionContext) => Promise<void>;
}) => Promise<{ cancelled: boolean }>;

export type ForkHandler = (
	entryId: string,
	options?: { position?: "before" | "at"; withSession?: (ctx: ReplacedSessionContext) => Promise<void> },
) => Promise<{ cancelled: boolean }>;

export type NavigateTreeHandler = (
	targetId: string,
```
Source: ice:packages/coding-agent/src/core/extensions/runner.ts:170-195.
```ts
	onError(listener: ExtensionErrorListener): () => void {
		this.errorListeners.add(listener);
		return () => this.errorListeners.delete(listener);
	}

	emitError(error: ExtensionError): void {
		for (const listener of this.errorListeners) {
			listener(error);
		}
	}

	/** The prompt-blocking handler running now, so the UI can say what a submitted prompt is waiting on. */
	getActiveHandler(): ActiveExtensionHandler | undefined {
		return this.activeHandler;
	}

	onSlowHandler(listener: SlowExtensionHandlerListener): () => void {
		this.slowHandlerListeners.add(listener);
		return () => this.slowHandlerListeners.delete(listener);
	}

	private async runPromptBlockingHandler<T>(
		extensionPath: string,
		event: ActiveExtensionHandler["event"],
		run: () => T | Promise<T>,
	): Promise<T> {
		const active: ActiveExtensionHandler = { extensionPath, event, startedAt: Date.now() };
		this.activeHandler = active;
		try {
			return await run();
		} finally {
			if (this.activeHandler === active) this.activeHandler = undefined;
			const durationMs = Date.now() - active.startedAt;
			if (durationMs >= SLOW_EXTENSION_HANDLER_MS) {
				for (const listener of this.slowHandlerListeners) listener({ ...active, durationMs });
			}
		}
	}

```
Source: ice:packages/coding-agent/src/core/extensions/runner.ts:591-629.
None: additional fixed CLI flags, slash commands, keybindings, settings, environment variables, RPC commands or persistent files beyond the explicitly listed registration/host contracts. Each dynamically registered name is chosen by the extension.

### 6. Dependencies

`EXT-008`, `EXT-009`

External services/binaries: none required by this host mechanism. Plugins may invoke arbitrary host I/O because they are trusted executable code; this audit did not load or run any plugin.

### 7. ice-rust today

rust:crates/ice-extensions/src/translate.rs:342-416; rust:crates/ice-extensions/src/translate.rs:710-885. Rust emits tracing warnings and bounds selected lifecycle callbacks (5 s before TUI,15 s shutdown,10 s other lifecycle) plus tool_call 600 s. It has no equivalent active-handler/after-completion slow-event host API. Native timeouts return while spawn_blocking code may continue; they are not equivalent to TS awaiting every callback. Preserve diagnostics but expose accurate late-work state.

### 8. Port notes

Add owner/event/start-time diagnostics and a bounded active-handler view in ice-extensions, using Instant for durations and explicit completion/timeout/late-running states. Keep native deadlines, but report when foreign work survives them. Isolate observer/reporting failures from handler outcomes and expose the diagnostic stream to ice-cli notices and observability without changing event semantics.

### 9. Size and priority

Estimated Rust implementation: **90-160 lines** (estimate, not measured source). **P1** - Required for faithful native extension behavior and for ICE-specific plugins that depend on it.

## EXT-022 - Generic lifecycle dispatch and last-returned session veto payload

### 1. Title and status

Generic lifecycle dispatch and last-returned session veto payload. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/extensions/runner.ts:879-923; ice:packages/coding-agent/src/core/extensions/types.ts:632-723; ice:packages/coding-agent/src/core/extensions/types.ts:1157-1184. emit/isSessionBeforeEvent. Relevant compaction/tree execution tests are session-area dependencies; this aggregation rule is directly read source.

Exported symbols: `emitSessionShutdownEvent` (ice:packages/coding-agent/src/core/extensions/runner.ts:212).
Exported symbols: `SessionStartEvent`, `SessionCompactEvent`, `SessionShutdownEvent`, `SessionTreeEvent`, `SessionEvent`, `AfterProviderResponseEvent` (ice:packages/coding-agent/src/core/extensions/types.ts:616; ice:packages/coding-agent/src/core/extensions/types.ts:659; ice:packages/coding-agent/src/core/extensions/types.ts:670; ice:packages/coding-agent/src/core/extensions/types.ts:700; ice:packages/coding-agent/src/core/extensions/types.ts:708; ice:packages/coding-agent/src/core/extensions/types.ts:746).

### 3. What it does

Await lifecycle handlers in load order and let session-before hooks veto or replace the operation-specific result.

### 4. How it works

Generic events await each subscribed handler in extension and registration order. Selected session-before events reduce whole returned results and stop on a cancellation request.

Create one context per emission. For each extension capture its handler array when reached, iterate in registration order, and await each callback. agent_start/turn_start use the slow-handler tracker; other generic events do not. For session_before_switch/fork/compact/tree only, each truthy returned value replaces the entire accumulated result. A truthy cancel returns immediately; otherwise a later {} can erase an earlier summary/compaction/skipConversationRestore. These are not merged fields. Other generic event return values are ignored. Exceptions are reported and the next handler normally runs; an error-listener exception can still escape.

There is no handler timeout, cancellation race, deep event clone or per-emission global snapshot. The same event object is passed, so in-place mutations can affect later handlers/caller even when a return is ignored or a handler throws. Session-before results are trusted host extension data, not runtime-validated here. Emission itself does not append the resulting compaction/tree entry; the session owner applies the returned value.

`emitSessionShutdownEvent(runner, event)` returns false when there are no session_shutdown handlers; otherwise it awaits the existing generic emitter and returns true. True means emission was attempted, not that every handler succeeded, since normal handler errors are handled by emit. The SDK's provider onResponse callback likewise looks up the current runner, skips absent after_provider_response handlers, and awaits generic emission with status and headers. Supported provider adapters call this hook after response headers and before consuming the body; return values do not replace the response (ice:packages/coding-agent/src/core/extensions/runner.ts:212-221; ice:packages/coding-agent/src/core/sdk.ts:353-362).

### 5. Interfaces

```ts
export interface SessionBeforeSwitchEvent {
	type: "session_before_switch";
	reason: "new" | "resume";
	targetSessionFile?: string;
}
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:632-636.
```ts
export interface SessionBeforeForkEvent {
	type: "session_before_fork";
	entryId: string;
	position: "before" | "at";
}
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:639-643.
```ts
export interface SessionBeforeCompactEvent {
	type: "session_before_compact";
	preparation: CompactionPreparation;
	branchEntries: SessionEntry[];
	customInstructions?: string;
	/** What triggered the compaction: manual /compact, the context threshold, or context overflow recovery */
	reason: "manual" | "threshold" | "overflow";
	/** True when the aborted turn is retried after this compaction (overflow recovery) */
	willRetry: boolean;
	signal: AbortSignal;
}
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:646-656.
```ts
export interface SessionBeforeTreeEvent {
	type: "session_before_tree";
	preparation: TreePreparation;
	signal: AbortSignal;
}
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:693-697.
```ts
export interface SessionBeforeSwitchResult {
	cancel?: boolean;
}
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:1157-1159.
```ts
export interface SessionBeforeForkResult {
	cancel?: boolean;
	skipConversationRestore?: boolean;
}
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:1161-1164.
```ts
export interface SessionBeforeCompactResult {
	cancel?: boolean;
	compaction?: CompactionResult;
}
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:1166-1169.
```ts
export interface SessionBeforeTreeResult {
	cancel?: boolean;
	summary?: {
		summary: string;
		details?: unknown;
		usage?: Usage;
	};
	/** Override custom instructions for summarization */
	customInstructions?: string;
	/** Override whether customInstructions replaces the default prompt */
	replaceInstructions?: boolean;
	/** Override label to attach to the branch summary entry */
	label?: string;
}
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:1171-1184.
None: additional fixed CLI flags, slash commands, keybindings, settings, environment variables, RPC commands or persistent files beyond the explicitly listed registration/host contracts. Each dynamically registered name is chosen by the extension.

Exact additional DTOs: `SessionStartEvent { type: 'session_start'; reason: 'startup' | 'reload' | 'new' | 'resume' | 'fork'; previousSessionFile?: string }`; `SessionCompactEvent { type: 'session_compact'; compactionEntry: CompactionEntry; fromExtension: boolean; reason: 'manual' | 'threshold' | 'overflow'; willRetry: boolean }`; `SessionShutdownEvent { type: 'session_shutdown'; reason: 'quit' | 'reload' | 'new' | 'resume' | 'fork'; targetSessionFile?: string }`; `SessionTreeEvent { type: 'session_tree'; newLeafId: string | null; oldLeafId: string | null; summaryEntry?: BranchSummaryEntry; fromExtension?: boolean }`. `SessionEvent` is the union of these records, SessionInfoChangedEvent, and the four before-event types already quoted. `AfterProviderResponseEvent { type: 'after_provider_response'; status: number; headers: Record<string,string> }` is a response-header notification (ice:packages/coding-agent/src/core/extensions/types.ts:616-717; ice:packages/coding-agent/src/core/extensions/types.ts:746-750).

### 6. Dependencies

`EXT-008`, `EXT-019`, `EXT-021`, `EXT-033`

External services/binaries: none required by this host mechanism. Plugins may invoke arbitrary host I/O because they are trusted executable code; this audit did not load or run any plugin.

Final dependency classification. Consuming integrations: `SESSION-022`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

rust:crates/ice-extensions/src/translate.rs:32-223; rust:crates/ice-extensions/src/translate.rs:342-447; rust:crates/ice-extensions/src/translate.rs:642-885. All 33 TS event tags are named in the SDK, but Rust generic delivery serializes a reduced payload and ignores most return values. Lifecycle dispatch sends an Empty arm and treats ABORT as veto, with per-handler timeouts/platform/priority policy. It cannot return TS compaction/tree/skip-restore payloads through that integer result. Native turn_end/agent_end carry no TS messages/toolResults/turnIndex; tool updates omit partialResult. Enum parity is not payload or behavioral parity.

### 8. Port notes

Add dedicated versioned DTOs for each lifecycle payload and return-valued session-before hook. Route the returned result through the existing session owner, with explicit whole-result replacement and cancellation semantics or a documented stricter native schema. Preserve priority/platform filters and deadline policy, retain callbacks with keepalives, and serialize no arbitrary host pointers.

### 9. Size and priority

Estimated Rust implementation: **220-380 lines** (estimate, not measured source). **P0** - Compaction, mode policy and observability cannot be ported on an event enum that discards required data and return values.

## EXT-023 - Final-message replacement with same-role enforcement

### 1. Title and status

Final-message replacement with same-role enforcement. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/extensions/runner.ts:925-965; ice:packages/coding-agent/src/core/extensions/types.ts:810-814; ice:packages/coding-agent/src/core/extensions/types.ts:1146-1149. emitMessageEnd. No dedicated message-end replacement assertion was found in the inspected runner tests; exact behavior is specified directly by the implementation.

### 3. What it does

Allow a handler to replace a finalized message without changing its role.

### 4. How it works

A message_end handler can replace the completed message while preserving its role. Later handlers receive the accepted replacement, and the session owner decides how to persist and publish it.

Start with event.message and modified=false. For each message_end handler build a shallow event copy pointing to the current message, await it and accept only a truthy result.message. Require replacement.role===currentMessage.role; otherwise emit an error and retain the previous message. An accepted replacement becomes the next handler’s input and marks modified, even when reference-identical. Return the final message only if an explicit replacement was accepted, otherwise undefined.

No deep clone, schema/ID/timestamp/content validation, size limit or rollback is supplied. In-place mutation can alter the original message even when modified remains false or a callback later throws. Exceptions normally log and continue. Persistence/output ordering is determined by the session caller that consumes this replacement, not by this dispatcher.

### 5. Interfaces

```ts
export interface MessageEndEvent {
	type: "message_end";
	message: AgentMessage;
}
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:810-813.
```ts
export interface MessageEndEventResult {
	/** Replace the finalized message. The replacement must keep the original message role. */
	message?: AgentMessage;
}
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:1146-1149.
```ts
	async emitMessageEnd(event: MessageEndEvent): Promise<AgentMessage | undefined> {
		const ctx = this.createContext();
		let currentMessage = event.message;
		let modified = false;

		for (const ext of this.extensions) {
			const handlers = ext.handlers.get("message_end");
			if (!handlers || handlers.length === 0) continue;

			for (const handler of handlers) {
				try {
					const currentEvent: MessageEndEvent = { ...event, message: currentMessage };
					const handlerResult = (await handler(currentEvent, ctx)) as MessageEndEventResult | undefined;
					if (!handlerResult?.message) continue;

					if (handlerResult.message.role !== currentMessage.role) {
						this.emitError({
							extensionPath: ext.path,
							event: "message_end",
							error: "message_end handlers must return a message with the same role",
						});
						continue;
					}

					currentMessage = handlerResult.message;
					modified = true;
				} catch (err) {
					const message = err instanceof Error ? err.message : String(err);
					const stack = err instanceof Error ? err.stack : undefined;
					this.emitError({
						extensionPath: ext.path,
						event: "message_end",
						error: message,
						stack,
					});
				}
			}
		}

		return modified ? currentMessage : undefined;
	}
```
Source: ice:packages/coding-agent/src/core/extensions/runner.ts:925-965.
None: additional fixed CLI flags, slash commands, keybindings, settings, environment variables, RPC commands or persistent files beyond the explicitly listed registration/host contracts. Each dynamically registered name is chosen by the extension.

### 6. Dependencies

`AGENT-001`, `EXT-019`, `EXT-021`, `EXT-033`

External services/binaries: none required by this host mechanism. Plugins may invoke arbitrary host I/O because they are trusted executable code; this audit did not load or run any plugin.

### 7. ice-rust today

rust:crates/ice-extensions/src/translate.rs:59-76; rust:crates/ice-extensions/src/translate.rs:342-417. Rust MessageEnd serializes its AgentMessage into host-owned bytes and lends them to integer-return observers. No replacement message is accepted through this ABI; kind/data serialization differs from TS's flat role-based object. Message replacement requires a dedicated output channel and session-side application, not mutation of a plugin's parsed copy.

### 8. Port notes

Add an owned MessageEnd transform result ABI in ice-plugin-sdk and an ice-extensions adapter that validates role and typed message content. Apply it once before authoritative session persistence/output through the session owner; do not treat an observer's integer return as a replacement. Keep original identity/rollback policy explicit and hold the producer deallocator until output adoption completes.

### 9. Size and priority

Estimated Rust implementation: **100-180 lines** (estimate, not measured source). **P1** - Required for faithful native extension behavior and for ICE-specific plugins that depend on it.

## EXT-024 - Mutable pre-tool block decision with propagated hook failures

### 1. Title and status

Mutable pre-tool block decision with propagated hook failures. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/extensions/runner.ts:1022-1043; ice:packages/coding-agent/src/core/extensions/types.ts:907-966; ice:packages/coding-agent/src/core/extensions/types.ts:1125-1129. emitToolCall; low-level hook failure conversion is AGENT-012. Exact dispatch rule is source-read, not a test-run result.

Exported symbols: `isToolCallEventType` (ice:packages/coding-agent/src/core/extensions/types.ts:1072; ice:packages/coding-agent/src/core/extensions/types.ts:1073; ice:packages/coding-agent/src/core/extensions/types.ts:1074; ice:packages/coding-agent/src/core/extensions/types.ts:1075; ice:packages/coding-agent/src/core/extensions/types.ts:1076; ice:packages/coding-agent/src/core/extensions/types.ts:1077; ice:packages/coding-agent/src/core/extensions/types.ts:1078; ice:packages/coding-agent/src/core/extensions/types.ts:1079; ice:packages/coding-agent/src/core/extensions/types.ts:1083).

### 3. What it does

Let trusted hooks inspect or mutate validated tool input and block execution before the tool runs.

### 4. How it works

A tool-call handler can mutate the input in place or return a blocking decision before execution. Dispatch stops on a truthy block or a rejected handler; otherwise the last truthy returned result is preserved.

Create one live context and pass the same event object to handlers in extension and registration order. Each truthy return replaces the accumulated result; a truthy result.block returns it immediately. If no result blocks, return the last truthy result, which may be {block:false}, {}, or an untyped value; undefined/falsy returns leave the prior result intact. The runner performs no schema validation, argument revalidation, clone, deadline or cancellation race. In-place event/input mutation remains visible to later handlers and the caller, including mutations before failure.

This method has no try/catch and does not emit ExtensionError itself. A thrown value or rejected promise propagates unchanged and prevents later callbacks. Normal agent integration must convert that rejection to a failed tool result without executing the tool (AGENT-012); the caller owns this fail-closed boundary. No synthetic block reason is created here. The earlier draft incorrectly described both error reporting and discarded nonblocking returns.

All seven built-in `isToolCallEventType` overloads plus the custom generic overload share one implementation: return `event.toolName === toolName`. The custom overload only asserts a compile-time `{ toolName: TName; input: TInput }` intersection; it does not validate custom arguments. The nine QA declaration records are overloads plus one implementation, not nine independent runtime functions (ice:packages/coding-agent/src/core/extensions/types.ts:1072-1085).

### 5. Interfaces

```ts
	type: "tool_call";
	toolCallId: string;
}

export interface BashToolCallEvent extends ToolCallEventBase {
	toolName: "bash";
	input: BashToolInput;
}

export interface ReadToolCallEvent extends ToolCallEventBase {
	toolName: "read";
	input: ReadToolInput;
}

export interface EditToolCallEvent extends ToolCallEventBase {
	toolName: "edit";
	input: EditToolInput;
}

export interface WriteToolCallEvent extends ToolCallEventBase {
	toolName: "write";
	input: WriteToolInput;
}

export interface GrepToolCallEvent extends ToolCallEventBase {
	toolName: "grep";
	input: GrepToolInput;
}

export interface FindToolCallEvent extends ToolCallEventBase {
	toolName: "find";
	input: FindToolInput;
}

export interface LsToolCallEvent extends ToolCallEventBase {
	toolName: "ls";
	input: LsToolInput;
}

export interface CustomToolCallEvent extends ToolCallEventBase {
	toolName: string;
	input: Record<string, unknown>;
}

/**
 * Fired before a tool executes. Can block.
 *
 * `event.input` is mutable. Mutate it in place to patch tool arguments before execution.
 * Later `tool_call` handlers see earlier mutations. No re-validation is performed after mutation.
 */
export type ToolCallEvent =
	| BashToolCallEvent
	| ReadToolCallEvent
	| EditToolCallEvent
	| WriteToolCallEvent
	| GrepToolCallEvent
	| FindToolCallEvent
	| LsToolCallEvent
	| CustomToolCallEvent;

interface ToolResultEventBase {
	type: "tool_result";
	toolCallId: string;
	input: Record<string, unknown>;
	content: (TextContent | ImageContent)[];
	isError: boolean;
	/** Usage from the tool execution itself, if available. */
	usage?: Usage;
}

```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:908-977.
```ts
export interface ToolCallEventResult {
	/** Block tool execution. To modify arguments, mutate `event.input` in place instead. */
	block?: boolean;
	reason?: string;
}
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:1125-1129.
```ts
	async emitToolCall(event: ToolCallEvent): Promise<ToolCallEventResult | undefined> {
		const ctx = this.createContext();
		let result: ToolCallEventResult | undefined;

		for (const ext of this.extensions) {
			const handlers = ext.handlers.get("tool_call");
			if (!handlers || handlers.length === 0) continue;

			for (const handler of handlers) {
				const handlerResult = await handler(event, ctx);

				if (handlerResult) {
					result = handlerResult as ToolCallEventResult;
					if (result.block) {
						return result;
					}
				}
			}
		}

		return result;
	}
```
Source: ice:packages/coding-agent/src/core/extensions/runner.ts:1022-1043.
No tool, shell or model request is executed by this method.

### 6. Dependencies

`AGENT-012`, `EXT-019`, `EXT-021`, `EXT-033`

External services/binaries: none required by this host mechanism. Plugins may invoke arbitrary host I/O because they are trusted executable code; this audit did not load or run any plugin.

### 7. ice-rust today

rust:crates/ice-extensions/src/translate.rs:450-600; rust:crates/ice-extensions/src/translate.rs:674-690. Rust has a real fail-closed blocking hook: only CONTINUE allows, ABORT/other code/panic/join error/600 s timeout/stale/cancellation block. Callback runs in spawn_blocking with keepalive and a cancellation race; aborting its JoinHandle does not stop started native code. ToolCallEvent carries serialized args but cannot return a mutated value/reason through the return-code ABI. Do not weaken Rust’s fail-closed timeout policy; add an owned validated argument-patch result and explicit late-callback lifetime handling.

### 8. Port notes

Add an owned pre-tool response DTO to the existing ice-plugin-sdk approval seam with explicit continue/block and optional argument replacement. Preserve Rust's fail-closed stale/cancel/error/600-second timeout handling, then validate and atomically install any returned argument patch before existing agent preparation. Retain each native callback's keepalive and revoke cancelled invocation handles even when spawn_blocking code finishes late. Keep callback diagnostics separate from a successful block response.

### 9. Size and priority

Estimated Rust implementation: **160-270 lines** (estimate, not measured source). **P0** - Permission hooks must retain input semantics and fail closed without allowing timed-out callbacks to mutate replacement sessions.

## EXT-025 - Chained tool-result field patches and usage propagation

### 1. Title and status

Chained tool-result field patches and usage propagation. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/extensions/runner.ts:967-1020; ice:packages/coding-agent/src/core/extensions/types.ts:968-1027; ice:packages/coding-agent/src/core/extensions/types.ts:1139-1144. emitToolResult. Tests: ice:packages/coding-agent/test/extensions-runner.test.ts:776-864 cover ordered content and partial-field patches; read-only review, not execution.

Exported symbols: `BashToolResultEvent`, `ReadToolResultEvent`, `EditToolResultEvent`, `WriteToolResultEvent`, `GrepToolResultEvent`, `FindToolResultEvent`, `LsToolResultEvent`, `CustomToolResultEvent`, `isBashToolResult`, `isReadToolResult`, `isEditToolResult`, `isWriteToolResult`, `isGrepToolResult`, `isFindToolResult`, `isLsToolResult` (ice:packages/coding-agent/src/core/extensions/types.ts:978; ice:packages/coding-agent/src/core/extensions/types.ts:983; ice:packages/coding-agent/src/core/extensions/types.ts:988; ice:packages/coding-agent/src/core/extensions/types.ts:993; ice:packages/coding-agent/src/core/extensions/types.ts:998; ice:packages/coding-agent/src/core/extensions/types.ts:1003; ice:packages/coding-agent/src/core/extensions/types.ts:1008; ice:packages/coding-agent/src/core/extensions/types.ts:1013; ice:packages/coding-agent/src/core/extensions/types.ts:1030; ice:packages/coding-agent/src/core/extensions/types.ts:1033; ice:packages/coding-agent/src/core/extensions/types.ts:1036; ice:packages/coding-agent/src/core/extensions/types.ts:1039; ice:packages/coding-agent/src/core/extensions/types.ts:1042; ice:packages/coding-agent/src/core/extensions/types.ts:1045; ice:packages/coding-agent/src/core/extensions/types.ts:1048).

### 3. What it does

Transform a tool’s model-visible result in ordered stages without erasing fields omitted by later handlers.

### 4. How it works

Tool-result handlers share one shallow event copy and can return ordered patches to content, details, isError and usage. Only a defined supported returned field marks the emission modified; the final return includes all four fields after the chain.

Copy the top-level input event once with {...event}, then pass that same currentEvent to every handler. A falsy handler result is ignored. For each truthy result, independently replace each of content/details/isError/usage when its value !== undefined and set modified=true. Empty arrays, false and runtime null therefore replace the field; an empty {} does not mark modified. Direct assignments to currentEvent properties persist into later handlers, while nested input/content/details objects remain shared with the original event. Direct mutation alone does not set modified, so the method can return undefined despite mutations; if any later supported patch sets modified, the final returned fields include those earlier direct mutations.

Exceptions emit ExtensionError and normally continue without rollback; reporter exceptions may escape. Return undefined when modified is still false, otherwise {content, details, isError, usage} from the shared currentEvent. Returned terminate/addedToolNames or other extra fields are ignored. No deep validation, truncation, redaction, persistence or retry occurs here. Native parity must distinguish absent fields from explicit null or reject null as a documented stricter contract.

The seven `is*ToolResult` helpers compare only `event.toolName` to the corresponding exact built-in name. They do not inspect event.type, input, content or details, and therefore must not be treated as validation or an authorization gate. Built-in result subtypes specialize the common ToolResultEventBase: bash/read/edit/grep/find/ls use their respective Details type or undefined, write has details:undefined, and CustomToolResultEvent has toolName:string with details:unknown (ice:packages/coding-agent/src/core/extensions/types.ts:978-1049).

### 5. Interfaces

```ts
export interface ToolResultEventResult {
	content?: (TextContent | ImageContent)[];
	details?: unknown;
	isError?: boolean;
	usage?: Usage;
}
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:1139-1144.
```ts
	async emitToolResult(event: ToolResultEvent): Promise<ToolResultEventResult | undefined> {
		const ctx = this.createContext();
		const currentEvent: ToolResultEvent = { ...event };
		let modified = false;

		for (const ext of this.extensions) {
			const handlers = ext.handlers.get("tool_result");
			if (!handlers || handlers.length === 0) continue;

			for (const handler of handlers) {
				try {
					const handlerResult = (await handler(currentEvent, ctx)) as ToolResultEventResult | undefined;
					if (!handlerResult) continue;

					if (handlerResult.content !== undefined) {
						currentEvent.content = handlerResult.content;
						modified = true;
					}
					if (handlerResult.details !== undefined) {
						currentEvent.details = handlerResult.details;
						modified = true;
					}
					if (handlerResult.isError !== undefined) {
						currentEvent.isError = handlerResult.isError;
						modified = true;
					}
					if (handlerResult.usage !== undefined) {
						currentEvent.usage = handlerResult.usage;
						modified = true;
					}
				} catch (err) {
					const message = err instanceof Error ? err.message : String(err);
					const stack = err instanceof Error ? err.stack : undefined;
					this.emitError({
						extensionPath: ext.path,
						event: "tool_result",
						error: message,
						stack,
					});
				}
			}
		}

		if (!modified) {
			return undefined;
		}

		return {
			content: currentEvent.content,
			details: currentEvent.details,
			isError: currentEvent.isError,
			usage: currentEvent.usage,
		};
	}
```
Source: ice:packages/coding-agent/src/core/extensions/runner.ts:967-1020.
The quoted event/result schemas and dispatcher specify the shared-wrapper and field-replacement contract.

### 6. Dependencies

`AGENT-015`, `EXT-019`, `EXT-021`, `EXT-033`

External services/binaries: none required by this host mechanism. Plugins may invoke arbitrary host I/O because they are trusted executable code; this audit did not load or run any plugin.

### 7. ice-rust today

rust:crates/ice-extensions/src/translate.rs:97-112; rust:crates/ice-extensions/src/translate.rs:229-265; rust:crates/ice-extensions/src/translate.rs:342-417. Native ToolExecutionEnd serializes call ID/name and content/details/terminate/addedToolNames with a separate isError bit. Its manual result JSON omits usage; the integer observer callback cannot return a field patch. This is distinct from an unconnected core after-tool callback. Preserve native result ownership while adding ordered result-transform output, and explicitly maintain usage through both result conversion and plugin DTOs.

### 8. Port notes

Append a tool-result transform callback to ice-plugin-sdk and collect owned field-presence-aware patches in ice-extensions. Integrate it at the existing agent after-tool boundary before model/session publication, applying fields in registered order and carrying usage. Use serde_json or typed enums that distinguish omitted and null; preserve nested-data ownership rather than exposing mutable raw native memory. Keep terminate and active-tool attribution under the core's result contract.

### 9. Size and priority

Estimated Rust implementation: **130-225 lines** (estimate, not measured source). **P0** - Result sanitizers, cost accounting and memory capture depend on ordered full-fidelity tool results.

## EXT-026 - User-shell operation replacement and error propagation

### 1. Title and status

User-shell operation replacement and error propagation. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/extensions/runner.ts:1045-1094; ice:packages/coding-agent/src/core/extensions/types.ts:867-875; ice:packages/coding-agent/src/core/extensions/types.ts:1132-1137. emitUserBash validates the first non-undefined response and fails closed. Source-read behavior; relevant host shell routing remains separately cited below.

### 3. What it does

Allow an extension to replace a user-invoked shell operation without rerouting it as a model tool.

### 4. How it works

User-shell routing lets an extension supply execution operations or a complete result for a user !/!! command. The first non-undefined response must validate; malformed output or a thrown handler ends routing with an attributed error and no later-handler fallback.

Create one context and await handlers in extension/registration order using the same UserBashEvent. Only undefined continues. Null and nonobjects throw "user_bash handlers must return undefined, operations, or a BashResult". If operations is defined, operations?.exec must be a function or throw "user_bash operations must provide an exec function". If result is defined, it must be a nonnull object with string output; exitCode absent or Number.isFinite; boolean cancelled and truncated; fullOutputPath absent or string. Otherwise throw "user_bash result must be a valid BashResult". If both operations and result are absent, throw "user_bash response must provide operations or a result". When both exist both are validated; extra fields are not rejected. Return the first valid response unchanged, or undefined after all handlers fall through.

The catch records extensionPath, event:"user_bash", error message and optional stack, then throws Error(`User Bash routing failed (${ext.path}): ${message}`, {cause:err}). An error-listener exception can replace this wrapper. The dispatcher launches no process and performs no fallback after failure; event.command/cwd/excludeFromContext are data supplied by the caller. !/!! routing is independent of model tool_call authorization, and replacement operations own their execution/cancellation while the session owns persistence.

### 5. Interfaces

```ts
export interface UserBashEvent {
	type: "user_bash";
	/** The command to execute */
	command: string;
	/** True if !! prefix was used (excluded from LLM context) */
	excludeFromContext: boolean;
	/** Current working directory */
	cwd: string;
}
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:867-875.
```ts
export interface UserBashEventResult {
	/** Custom operations to use for execution */
	operations?: BashOperations;
	/** Full replacement: extension handled execution, use this result */
	result?: BashResult;
}
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:1132-1137.
```ts
	async emitUserBash(event: UserBashEvent): Promise<UserBashEventResult | undefined> {
		const ctx = this.createContext();

		for (const ext of this.extensions) {
			const handlers = ext.handlers.get("user_bash");
			if (!handlers || handlers.length === 0) continue;

			for (const handler of handlers) {
				try {
					const handlerResult: unknown = await handler(event, ctx);
					if (handlerResult === undefined) continue;
					if (handlerResult === null || typeof handlerResult !== "object") {
						throw new Error("user_bash handlers must return undefined, operations, or a BashResult");
					}
					const result = handlerResult as UserBashEventResult;
					if (result.operations !== undefined && typeof result.operations?.exec !== "function") {
						throw new Error("user_bash operations must provide an exec function");
					}
					if (
						result.result !== undefined &&
						(result.result === null ||
							typeof result.result !== "object" ||
							typeof result.result.output !== "string" ||
							(result.result.exitCode !== undefined && !Number.isFinite(result.result.exitCode)) ||
							typeof result.result.cancelled !== "boolean" ||
							typeof result.result.truncated !== "boolean" ||
							(result.result.fullOutputPath !== undefined && typeof result.result.fullOutputPath !== "string"))
					) {
						throw new Error("user_bash result must be a valid BashResult");
					}
					if (result.operations === undefined && result.result === undefined) {
						throw new Error("user_bash response must provide operations or a result");
					}
					return result;
				} catch (err) {
					const message = err instanceof Error ? err.message : String(err);
					const stack = err instanceof Error ? err.stack : undefined;
					this.emitError({
						extensionPath: ext.path,
						event: "user_bash",
						error: message,
						stack,
					});
					throw new Error(`User Bash routing failed (${ext.path}): ${message}`, { cause: err });
				}
			}
		}

		return undefined;
	}
```
Source: ice:packages/coding-agent/src/core/extensions/runner.ts:1045-1094.
None: additional fixed CLI flags, slash commands, keybindings, settings, environment variables, RPC commands or persistent files beyond the explicitly listed registration/host contracts. Each dynamically registered name is chosen by the extension.

### 6. Dependencies

`EXT-019`, `EXT-033`

External services/binaries: none required by this host mechanism. Plugins may invoke arbitrary host I/O because they are trusted executable code; this audit did not load or run any plugin.

### 7. ice-rust today

rust:crates/ice-plugin-sdk/src/events.rs:113-145; rust:crates/ice-extensions/src/translate.rs:423-447. Rust names UserBash and can deliver a JSON observer, but its generic event return code cannot carry custom BashOperations or a replacement BashResult. An event tag alone does not replace execution. Provide an explicit host shell-operation interface or returned complete result, without granting a plugin a second uncontrolled command loop.

### 8. Port notes

Add a native shell-routing callback with a tagged response: decline, execute-via-registered-operations handle, or completed BashResult. Validate all fields before dispatch and use the existing shell runner for execution; never convert a malformed response/error to local fallback. Propagate a structured owner-attributed error with cancellation and full-output provenance. Rust trait operations can remain native behind owner-scoped handles; avoid serializing function pointers as generic JSON.

### 9. Size and priority

Estimated Rust implementation: **100-180 lines** (estimate, not measured source). **P1** - Required for faithful native extension behavior and for ICE-specific plugins that depend on it.

## EXT-027 - Deep-cloned model-context transformation chain

### 1. Title and status

Deep-cloned model-context transformation chain. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/extensions/runner.ts:1097-1128; ice:packages/coding-agent/src/core/extensions/types.ts:724-729; ice:packages/coding-agent/src/core/extensions/types.ts:1119-1121. emitContext. Existing low-level context ordering is specified in AGENT-006; the clone/patch behavior here is source-defined.

### 3. What it does

Let extensions modify a request’s message context without mutating the stored transcript through shared initial references.

### 4. How it works

Context handlers transform the messages sent toward a model without directly editing the durable transcript. Listener presence triggers one initial deep clone, after which handler-returned arrays and mutations are chained.

When there are no context handlers, return the original messages array immediately without creating a context or cloning it. Otherwise structuredClone the entire input once before entering handler try/catch; clone failure rejects the operation. Each handler receives the current cloned/previously returned array. A truthy result.messages replaces it, including[], and is not cloned again. In-place changes inside that current array survive into later handlers and the final return; a handler that mutates then throws is logged but its mutations are not rolled back.

No token budget, role/content validation, tool-result repair, redaction or durable transcript write occurs. A returned array may reintroduce shared references. The no-listener fast path is intentionally identity-preserving, while listener presence isolates the original initial transcript at the clone boundary.

### 5. Interfaces

```ts
export interface ContextEvent {
	type: "context";
	messages: AgentMessage[];
}
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:724-727.
```ts
export interface ContextEventResult {
	messages?: AgentMessage[];
}
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:1119-1121.
```ts
	async emitContext(messages: AgentMessage[]): Promise<AgentMessage[]> {
		// The defensive clone below costs a full-history copy per model call; skip it when nobody listens.
		if (!this.hasHandlers("context")) return messages;
		const ctx = this.createContext();
		let currentMessages = structuredClone(messages);

		for (const ext of this.extensions) {
			const handlers = ext.handlers.get("context");
			if (!handlers || handlers.length === 0) continue;

			for (const handler of handlers) {
				try {
					const event: ContextEvent = { type: "context", messages: currentMessages };
					const handlerResult = await handler(event, ctx);

					if (handlerResult && (handlerResult as ContextEventResult).messages) {
						currentMessages = (handlerResult as ContextEventResult).messages!;
					}
				} catch (err) {
					const message = err instanceof Error ? err.message : String(err);
					const stack = err instanceof Error ? err.stack : undefined;
					this.emitError({
						extensionPath: ext.path,
						event: "context",
						error: message,
						stack,
					});
				}
			}
		}

		return currentMessages;
	}
```
Source: ice:packages/coding-agent/src/core/extensions/runner.ts:1096-1128.
No persistent message is written by this transformation.

### 6. Dependencies

`AGENT-006`, `EXT-019`, `EXT-021`, `EXT-033`

External services/binaries: none required by this host mechanism. Plugins may invoke arbitrary host I/O because they are trusted executable code; this audit did not load or run any plugin.

### 7. ice-rust today

rust:crates/ice-extensions/src/provider_hooks.rs:102-125; rust:crates/ice-extensions/src/prompt_transform.rs:54-114. Native provider transform_context only applies the separate systemPrompt transformer, cached by latest user timestamp; generic Context observers cannot return message arrays. Rust core has a transform hook, but no equivalent native extension message-transform ABI is wired here. Use owned cloned message snapshots and bounded output validation.

### 8. Port notes

Add a dedicated owned message-transform callback to ice-extensions and wire it at the existing agent/provider context transform seam. Clone once before the first extension, validate returned roles/content/tool-result coherence, and keep the durable transcript separate. Bound output size and cancellation explicitly, documenting any stricter validation relative to TS.

### 9. Size and priority

Estimated Rust implementation: **120-210 lines** (estimate, not measured source). **P1** - Required for faithful native extension behavior and for ICE-specific plugins that depend on it.

## EXT-028 - Actual provider-payload replacement chain

### 1. Title and status

Actual provider-payload replacement chain. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/extensions/runner.ts:1130-1162; ice:packages/coding-agent/src/core/extensions/types.ts:730-739; ice:packages/coding-agent/src/core/extensions/types.ts:1123-1123. emitBeforeProviderRequest. Wire-provider call sites are AI-layer dependencies, not executed in this audit.

### 3. What it does

Allow an extension to inspect and replace the outgoing provider payload immediately before transport.

### 4. How it works

Before-provider-request handlers can replace the exact outgoing provider payload. Each defined return becomes the next payload, with no merge or validation in the runner.

Accept an unknown payload by reference. For each before_provider_request handler, pass {type, payload:currentPayload}; any return other than undefined becomes the entire next payload, including null, false,0 or an empty string. There is no field merge or schema validation. In-place changes survive an undefined return and also survive a later thrown error; errors normally log and the chain continues. Return the final payload. No clone, key filtering, request retry, URL rewrite or persistence is performed by this dispatcher.

The bound AI provider decides when the payload exists and applies its wire validation. A generic model/context/options diagnostic is not a substitute for this actual post-serialization payload. Extension code is trusted and can break a request or expose secrets; repository text cannot register such authority without project trust.

### 5. Interfaces

```ts
export interface BeforeProviderRequestEvent {
	type: "before_provider_request";
	payload: unknown;
}
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:730-733.
```ts
export type BeforeProviderRequestEventResult = unknown;
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:1123-1123.
```ts
	async emitBeforeProviderRequest(payload: unknown): Promise<unknown> {
		const ctx = this.createContext();
		let currentPayload = payload;

		for (const ext of this.extensions) {
			const handlers = ext.handlers.get("before_provider_request");
			if (!handlers || handlers.length === 0) continue;

			for (const handler of handlers) {
				try {
					const event: BeforeProviderRequestEvent = {
						type: "before_provider_request",
						payload: currentPayload,
					};
					const handlerResult = await handler(event, ctx);
					if (handlerResult !== undefined) {
						currentPayload = handlerResult;
					}
				} catch (err) {
					const message = err instanceof Error ? err.message : String(err);
					const stack = err instanceof Error ? err.stack : undefined;
					this.emitError({
						extensionPath: ext.path,
						event: "before_provider_request",
						error: message,
						stack,
					});
				}
			}
		}

		return currentPayload;
	}
```
Source: ice:packages/coding-agent/src/core/extensions/runner.ts:1130-1162.
None: additional fixed CLI flags, slash commands, keybindings, settings, environment variables, RPC commands or persistent files beyond the explicitly listed registration/host contracts. Each dynamically registered name is chosen by the extension.

### 6. Dependencies

`EXT-019`, `EXT-021`, `EXT-033`

External services/binaries: none required by this host mechanism. Plugins may invoke arbitrary host I/O because they are trusted executable code; this audit did not load or run any plugin.

### 7. ice-rust today

rust:crates/ice-extensions/src/provider_hooks.rs:132-196. Rust emits a model/context/options envelope containing model id, provider, baseUrl, reasoning, pricingAvailable, apiKey, timeoutMs, headers, metadata, maxTokens, temperature and model-facing messages. It returns None rather than a payload replacement. This is a real observer, but not the provider's final transport-specific wire payload. The apiKey field is visible to these trusted observers; it must not be confused with the deliberately restricted GetExecutionContext snapshot.

### 8. Port notes

Add an owner-aware return-valued hook at each provider's final payload serialization seam, using a versioned owned JSON request/result. Distinguish no replacement from JSON null and validate the result using the transport's real schema before sending. Define whether authentication fields are included, and avoid copying secrets into general diagnostics or reusable context snapshots.

### 9. Size and priority

Estimated Rust implementation: **130-230 lines** (estimate, not measured source). **P0** - Provider adapters and routing extensions require actual wire mutation, not a secret-bearing observer that cannot affect the request.

## EXT-029 - In-place provider header-record mutation

### 1. Title and status

In-place provider header-record mutation. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/extensions/runner.ts:1164-1193; ice:packages/coding-agent/src/core/extensions/types.ts:740-745. emitBeforeProviderHeaders. Tests: ice:packages/coding-agent/test/extensions-runner.test.ts:981-999 (`lets a handler mutate headers in place and preserves existing headers`); ice:packages/coding-agent/test/extensions-runner.test.ts:1001-1031 (`isolates a throwing handler and still applies the others`).

### 3. What it does

Let extensions add, replace or remove HTTP headers using the same header object seen by later handlers.

### 4. How it works

Before-provider-headers handlers edit one shared plain header record. Their return values are ignored, and mutations before a failure remain visible to later handlers.

Pass the same supplied ProviderHeaders record to every before_provider_headers handler. Await in order and ignore return values. The intended mutation is ordinary property assignment/deletion, as the tests demonstrate; this is not a WHATWG Headers object with get/set/delete methods. Header-name normalization and transport serialization happen elsewhere. A plain record can hold differently cased keys simultaneously.

Return the same record. On handler failure log and continue, retaining earlier or pre-failure mutations. There is no clone, header allowlist, auth-header protection, redaction, size bound or rollback. A native patch API must distinguish deletion and replacement and preserve the selected case-normalization policy explicitly.

### 5. Interfaces

```ts
export interface BeforeProviderHeadersEvent {
	type: "before_provider_headers";
	headers: ProviderHeaders;
}
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:740-743.
```ts
	async emitBeforeProviderHeaders(headers: ProviderHeaders): Promise<ProviderHeaders> {
		const ctx = this.createContext();

		for (const ext of this.extensions) {
			const handlers = ext.handlers.get("before_provider_headers");
			if (!handlers || handlers.length === 0) continue;

			for (const handler of handlers) {
				try {
					// Handlers mutate `headers` in place; the return value is ignored.
					const event: BeforeProviderHeadersEvent = {
						type: "before_provider_headers",
						headers,
					};
					await handler(event, ctx);
				} catch (err) {
					const message = err instanceof Error ? err.message : String(err);
					const stack = err instanceof Error ? err.stack : undefined;
					this.emitError({
						extensionPath: ext.path,
						event: "before_provider_headers",
						error: message,
						stack,
					});
				}
			}
		}

		return headers;
	}
```
Source: ice:packages/coding-agent/src/core/extensions/runner.ts:1164-1193.
None: additional fixed CLI flags, slash commands, keybindings, settings, environment variables, RPC commands or persistent files beyond the explicitly listed registration/host contracts. Each dynamically registered name is chosen by the extension.

### 6. Dependencies

`EXT-019`, `EXT-021`, `EXT-033`

External services/binaries: none required by this host mechanism. Plugins may invoke arbitrary host I/O because they are trusted executable code; this audit did not load or run any plugin.

### 7. ice-rust today

rust:crates/ice-extensions/src/provider_hooks.rs:187-196. Rust emits {headers: opts.headers} as a serialized observer event and returns no StreamOptionsPatch. A plugin may parse and edit its copy, but no returned patch is adopted into transport headers. Static provider header configuration is separate from this missing per-request mutation channel.

### 8. Port notes

Expose an owned header edit result with set and remove operations and explicit case-handling in the provider transport. Chain edits in the selected registration order and validate names/values before HTTP serialization. Document whether protected authentication headers can be modified, and keep unredacted values out of event logs.

### 9. Size and priority

Estimated Rust implementation: **90-160 lines** (estimate, not measured source). **P1** - Required for faithful native extension behavior and for ICE-specific plugins that depend on it.

## EXT-030 - Per-turn system-prompt chain plus custom-message injection

### 1. Title and status

Per-turn system-prompt chain plus custom-message injection. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/extensions/runner.ts:1195-1261; ice:packages/coding-agent/src/core/extensions/types.ts:753-765; ice:packages/coding-agent/src/core/extensions/types.ts:1151-1155. emitBeforeAgentStart. Test: ice:packages/coding-agent/test/extensions-runner.test.ts:732-772 (`keeps ctx.getSystemPrompt() in sync with chained system prompt updates`). Cognee/guard use this exact chain in COG-046 and SAFE entries.

### 3. What it does

Combine extension context messages and system-prompt replacements once before a submitted prompt starts its agent run.

### 4. How it works

Before-agent-start handlers can supply run-local prompt replacements and custom messages. Prompt replacements chain in order while messages accumulate for the session caller.

Create a descriptor-preserving live context whose getSystemPrompt returns the locally chained current prompt. Every before_agent_start event receives the original prompt/images/systemPromptOptions and current systemPrompt. Run the handler through slow tracking. Accumulate every truthy result.message in order; any systemPrompt!==undefined replaces the current string and marks modified, including an empty string (untyped null is not runtime-rejected). Event.systemPrompt mutation alone is not the returned replacement, while shared options/images can be mutated by reference.

Errors log and later handlers continue. Return undefined when no message/replacement was returned; otherwise return {messages:array-or-undefined, systemPrompt:changed-value-or-undefined}. Do not directly persist messages, start another run or permanently change the base system prompt. The session caller assigns custom message role/timestamp and decides visibility/context persistence. No content/image/token cap or trust recheck occurs at this stage.

### 5. Interfaces

```ts
export interface BeforeAgentStartEvent {
	type: "before_agent_start";
	/** The raw user prompt text (after expansion). */
	prompt: string;
	/** Images attached to the user prompt, if any. */
	images?: ImageContent[];
	/** The fully assembled system prompt string. */
	systemPrompt: string;
	/** Structured options used to build the system prompt. Extensions can inspect this to understand what Ice loaded without re-discovering resources. */
	systemPromptOptions: BuildSystemPromptOptions;
}
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:753-763.
```ts
export interface BeforeAgentStartEventResult {
	message?: Pick<CustomMessage, "customType" | "content" | "display" | "details">;
	/** Replace the system prompt for this turn. If multiple extensions return this, they are chained. */
	systemPrompt?: string;
}
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:1151-1155.
```ts
	async emitBeforeAgentStart(
		prompt: string,
		images: ImageContent[] | undefined,
		systemPrompt: string,
		systemPromptOptions: BuildSystemPromptOptions,
	): Promise<BeforeAgentStartCombinedResult | undefined> {
		let currentSystemPrompt = systemPrompt;
		const ctx = Object.defineProperties(
			{},
			Object.getOwnPropertyDescriptors(this.createContext()),
		) as ExtensionContext;
		ctx.getSystemPrompt = () => {
			this.assertActive();
			return currentSystemPrompt;
		};
		const messages: NonNullable<BeforeAgentStartEventResult["message"]>[] = [];
		let systemPromptModified = false;

		for (const ext of this.extensions) {
			const handlers = ext.handlers.get("before_agent_start");
			if (!handlers || handlers.length === 0) continue;

			for (const handler of handlers) {
				try {
					const event: BeforeAgentStartEvent = {
						type: "before_agent_start",
						prompt,
						images,
						systemPrompt: currentSystemPrompt,
						systemPromptOptions,
					};
					const handlerResult = await this.runPromptBlockingHandler(ext.path, "before_agent_start", () =>
						handler(event, ctx),
					);

					if (handlerResult) {
						const result = handlerResult as BeforeAgentStartEventResult;
						if (result.message) {
							messages.push(result.message);
						}
						if (result.systemPrompt !== undefined) {
							currentSystemPrompt = result.systemPrompt;
							systemPromptModified = true;
						}
					}
				} catch (err) {
					const message = err instanceof Error ? err.message : String(err);
					const stack = err instanceof Error ? err.stack : undefined;
					this.emitError({
						extensionPath: ext.path,
						event: "before_agent_start",
						error: message,
						stack,
					});
				}
			}
		}

		if (messages.length > 0 || systemPromptModified) {
			return {
				messages: messages.length > 0 ? messages : undefined,
				systemPrompt: systemPromptModified ? currentSystemPrompt : undefined,
			};
		}

		return undefined;
	}
```
Source: ice:packages/coding-agent/src/core/extensions/runner.ts:1195-1261.
None: additional fixed CLI flags, slash commands, keybindings, settings, environment variables, RPC commands or persistent files beyond the explicitly listed registration/host contracts. Each dynamically registered name is chosen by the extension.

### 6. Dependencies

`EXT-019`, `EXT-021`, `EXT-033`

External services/binaries: none required by this host mechanism. Plugins may invoke arbitrary host I/O because they are trusted executable code; this audit did not load or run any plugin.

### 7. ice-rust today

rust:crates/ice-extensions/src/prompt_transform.rs:54-150; rust:crates/ice-extensions/src/provider_hooks.rs:81-158. Rust has a separate synchronous before-agent transformer accepting/returning only systemPrompt, without custom messages or full image/options/context. It caches by latest user timestamp and can reuse a result for distinct same-timestamp prompts; its observer sees imageCount rather than image payloads. Transformer registration order is used without generic priority/platform filters, and there is no async timeout/cancellation. Keep the existing transformer but extend its DTO/lifetime and cache key rather than declaring the event wholly missing.

### 8. Port notes

Extend the existing before-agent-start return DTO with custom messages, full prompt/image metadata and a run identity. Replace latest-user-timestamp caching with an explicit run key so equal timestamps and retry/continuation paths cannot reuse an unrelated transform. Wire messages through the session owner; retain producer deallocation, ordered prompt replacement, cancellation policy and late-callback keepalive.

### 9. Size and priority

Estimated Rust implementation: **170-290 lines** (estimate, not measured source). **P0** - Safe mode, Cognee recall and other core ICE factories depend on this return-valued per-turn injection contract.

## EXT-031 - Resource discovery results with declaring-extension attribution

### 1. Title and status

Resource discovery results with declaring-extension attribution. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/extensions/runner.ts:1263-1309; ice:packages/coding-agent/src/core/extensions/types.ts:598-614. emitResourcesDiscover. Discovery/loading/collision tests in resource-loader are a separate settings-area dependency; this method’s attribution is explicit source.

### 3. What it does

Allow extensions to contribute skill, prompt and theme paths while retaining the origin needed for trust and diagnostics.

### 4. How it works

Resource discovery collects extension-contributed skill, prompt, and theme paths. Each path retains the originating extension identity for later resolution and diagnostics.

For each resources_discover handler pass the same cwd/reason event and live context, await the result and append every truthy field array in handler order. For each returned path construct {path, extensionPath:owner.path}. No deduplication, normalization, glob expansion, existence check, type validation, trust decision or file load occurs in this method. Empty arrays contribute nothing. A truthy field with length but without a callable .map can throw during mapping; the error is logged and already appended paths remain. The result always contains all three arrays, even when empty.

The higher resource loader resolves relative paths and policy using this attribution; dropping it can change permissions and diagnostics. Discovery callbacks are trusted executable code and may have side effects, but returned file content must remain data until separately admitted.

### 5. Interfaces

```ts
export interface ResourcesDiscoverEvent {
	type: "resources_discover";
	cwd: string;
	reason: "startup" | "reload";
}
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:598-602.
```ts
export interface ResourcesDiscoverResult {
	skillPaths?: string[];
	promptPaths?: string[];
	themePaths?: string[];
}
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:605-609.
```ts
	async emitResourcesDiscover(
		cwd: string,
		reason: ResourcesDiscoverEvent["reason"],
	): Promise<{
		skillPaths: Array<{ path: string; extensionPath: string }>;
		promptPaths: Array<{ path: string; extensionPath: string }>;
		themePaths: Array<{ path: string; extensionPath: string }>;
	}> {
		const ctx = this.createContext();
		const skillPaths: Array<{ path: string; extensionPath: string }> = [];
		const promptPaths: Array<{ path: string; extensionPath: string }> = [];
		const themePaths: Array<{ path: string; extensionPath: string }> = [];

		for (const ext of this.extensions) {
			const handlers = ext.handlers.get("resources_discover");
			if (!handlers || handlers.length === 0) continue;

			for (const handler of handlers) {
				try {
					const event: ResourcesDiscoverEvent = { type: "resources_discover", cwd, reason };
					const handlerResult = await handler(event, ctx);
					const result = handlerResult as ResourcesDiscoverResult | undefined;

					if (result?.skillPaths?.length) {
						skillPaths.push(...result.skillPaths.map((path) => ({ path, extensionPath: ext.path })));
					}
					if (result?.promptPaths?.length) {
						promptPaths.push(...result.promptPaths.map((path) => ({ path, extensionPath: ext.path })));
					}
					if (result?.themePaths?.length) {
						themePaths.push(...result.themePaths.map((path) => ({ path, extensionPath: ext.path })));
					}
				} catch (err) {
					const message = err instanceof Error ? err.message : String(err);
					const stack = err instanceof Error ? err.stack : undefined;
					this.emitError({
						extensionPath: ext.path,
						event: "resources_discover",
						error: message,
						stack,
					});
				}
			}
		}

		return { skillPaths, promptPaths, themePaths };
	}
```
Source: ice:packages/coding-agent/src/core/extensions/runner.ts:1263-1309.
None: additional fixed CLI flags, slash commands, keybindings, settings, environment variables, RPC commands or persistent files beyond the explicitly listed registration/host contracts. Each dynamically registered name is chosen by the extension.

### 6. Dependencies

`EXT-019`, `EXT-021`, `EXT-033`

External services/binaries: none required by this host mechanism. Plugins may invoke arbitrary host I/O because they are trusted executable code; this audit did not load or run any plugin.

Final dependency classification. Consuming integrations: `SESSION-035`, `SET-021`, `TUI-003`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

rust:crates/ice-extensions/src/resources.rs:44-150. Rust has a real separate registration/return-valued resource hook and ordered merge, but returns bare path strings without owning extensionPath. It filters nonstrings, ignores malformed output and invokes synchronously without generic priority/platform policy or cancellation. Relative path trust/source provenance must be restored in the native host; the event enum alone is not the return channel.

### 8. Port notes

Store owner/source path on ResourcesDiscoverHandler and return attributed ResourcePath records to the existing ice-cli resource loader. Preserve deterministic handler order and define relative resolution, package trust and channel disable flags before reading contributed files. Keep native malformed-output filtering as a documented hardening choice and use producer-owned result buffers.

### 9. Size and priority

Estimated Rust implementation: **100-180 lines** (estimate, not measured source). **P1** - Required for faithful native extension behavior and for ICE-specific plugins that depend on it.

## EXT-032 - Submitted-input transformation and streaming-delivery metadata

### 1. Title and status

Submitted-input transformation and streaming-delivery metadata. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/extensions/runner.ts:1311-1353; ice:packages/coding-agent/src/core/extensions/types.ts:882-901. emitInput. ice:packages/coding-agent/test/extensions-input-event.test.ts:37-117 covers text/image chains, handled short-circuit, error isolation and streamingBehavior preservation; this test file is dirty and absent from the frozen source-hash manifest.

### 3. What it does

Transform or consume user/extension input before the host performs later prompt expansion and admission.

### 4. How it works

Submitted-input handlers can transform text/images or consume the input before later prompt processing. A handled result stops immediately; otherwise the runner reports a transform only when the final text or images reference differs from the original.

Keep currentText/currentImages locals and build a fresh event for each callback with those values and the original source and streamingBehavior. source is interactive|rpc|extension; streamingBehavior is optional steer|followUp. The event has no requestId, sendNow or syntheticKind field. Run each callback through prompt-blocking tracking. result?.action === "handled" returns that result unchanged. For action "transform", assign result.text without runtime type validation and assign currentImages = result.images ?? currentImages. Thus absent or untyped null images preserve the current images; [] clears with a new reference. Continue, undefined or unknown actions leave locals unchanged. Direct wrapper-property assignment is not a transform, although nested image mutation can affect shared data.

Errors emit diagnostics and normally continue, preserving prior local/shared changes. After all callbacks compare currentText !== text || currentImages !== images and return {action:"transform", text:currentText, images:currentImages} only if changed; transient transformations later reverted to the original text/reference yield continue. No explicit modified flag, deep comparison, cloning, validation, command expansion, turn admission or persistence exists here. The earlier draft invented request correlation fields and different null/final-state semantics.

### 5. Interfaces

```ts
export interface InputEvent {
	type: "input";
	/** The input text */
	text: string;
	/** Attached images, if any */
	images?: ImageContent[];
	/** Where the input came from */
	source: InputSource;
	/** How the input will be delivered during streaming, or undefined when idle */
	streamingBehavior?: "steer" | "followUp";
}
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:885-895.
```ts
export type InputEventResult =
	| { action: "continue" }
	| { action: "transform"; text: string; images?: ImageContent[] }
	| { action: "handled" };
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:898-901.
```ts
	/** Emit input event. Transforms chain, "handled" short-circuits. */
	async emitInput(
		text: string,
		images: ImageContent[] | undefined,
		source: InputSource,
		streamingBehavior?: "steer" | "followUp",
	): Promise<InputEventResult> {
		const ctx = this.createContext();
		let currentText = text;
		let currentImages = images;

		for (const ext of this.extensions) {
			for (const handler of ext.handlers.get("input") ?? []) {
				try {
					const event: InputEvent = {
						type: "input",
						text: currentText,
						images: currentImages,
						source,
						streamingBehavior,
					};
					const result = (await this.runPromptBlockingHandler(ext.path, "input", () => handler(event, ctx))) as
						| InputEventResult
						| undefined;
					if (result?.action === "handled") return result;
					if (result?.action === "transform") {
						currentText = result.text;
						currentImages = result.images ?? currentImages;
					}
				} catch (err) {
					this.emitError({
						extensionPath: ext.path,
						event: "input",
						error: err instanceof Error ? err.message : String(err),
						stack: err instanceof Error ? err.stack : undefined,
					});
				}
			}
		}
		return currentText !== text || currentImages !== images
			? { action: "transform", text: currentText, images: currentImages }
			: { action: "continue" };
	}
}
```
Source: ice:packages/coding-agent/src/core/extensions/runner.ts:1311-1354.
No new tool or file; the session caller owns subsequent expansion and message scheduling.

### 6. Dependencies

`EXT-019`, `EXT-021`, `EXT-033`

External services/binaries: none required by this host mechanism. Plugins may invoke arbitrary host I/O because they are trusted executable code; this audit did not load or run any plugin.

### 7. ice-rust today

rust:crates/ice-extensions/src/translate.rs:590-636; rust:crates/ice-plugin-sdk/src/lib.rs:931-958. Native Input is an integer-result claimable JSON event also used for raw shortcut keys. Claim dispatch visits every eligible handler and returns whether any claimed; it has no text/images transform output or handled short-circuit. Implement a separate submitted-input DTO preserving source and optional steer/followUp rather than treating raw key claims as this API. requestId is not part of the TypeScript reference contract.

### 8. Port notes

Add a distinct submitted-input callback/result to ice-plugin-sdk, keeping raw-key Input ownership separate. Chain owned text/image replacements in registration order and stop on handled; define image omission explicitly and compare final values according to the chosen native contract. Call the chain at the existing prompt ingress before expansion/queue admission. Preserve source and steer/followUp fields; do not invent requestId as an upstream requirement.

### 9. Size and priority

Estimated Rust implementation: **130-220 lines** (estimate, not measured source). **P0** - Prompt transforms and queue-delivery metadata must remain deterministic across handlers without conflating raw keys or adding nonexistent fields.

## EXT-033 - Native event payload ownership, field loss and late callback lifetime

### 1. Title and status

Native event payload ownership, field loss and late callback lifetime. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/extensions/types.ts:766-843; ice:packages/coding-agent/src/core/extensions/runner.ts:888-923. AgentEvent DTOs and awaited dispatch. Tests: SDK event-view tests rust:crates/ice-plugin-sdk/src/events.rs:146-224 and TypeScript runner tests are read evidence, not test results.

Exported symbols: `AgentStartEvent`, `AgentSettledEvent`, `MessageStartEvent` (ice:packages/coding-agent/src/core/extensions/types.ts:766; ice:packages/coding-agent/src/core/extensions/types.ts:777; ice:packages/coding-agent/src/core/extensions/types.ts:797).

### 3. What it does

Preserve TypeScript event values across a native ABI without leaking allocations or allowing a callback to retain freed payloads.

### 4. How it works

The native boundary must preserve event data and ownership in addition to event names. Current Rust delivery has concrete field losses and cleanup inconsistencies that remain distinct from missing return-valued hooks.

TypeScript passes managed event objects and awaits callbacks; no plugin free function is involved. The exact TS lifecycle/message/tool DTOs below include messages, turn indices, original inputs, partial results and usage-bearing results. Native dispatch must serialize these values faithfully, give each callback a bounded borrowing period, and reclaim host allocations once after all callbacks—not ask the plugin to free a borrowed event. A port must also preserve return-valued transformations through dedicated output DTOs rather than modifying borrowed bytes.

Current Rust creates host-owned StbString fields, passes a StablePluginEvent by value whose payload strings remain borrowed and frees at dispatch completion. SDK EventView correctly tells handlers to copy before retaining. The generic stale-session early return happens before free_dispatched_event, leaking an already-created payload. The free switch frees only selected Data tags; BeforeAgentStart is grouped as payload-free although its provider-hook caller constructs a Data arm, so that observed path does not reclaim data_json. Other tag/arm mismatches require their own caller proof. Conversely arbitrary mismatched tag/union-arm construction can make a free branch read the wrong arm; constructors must enforce the tag/arm invariant. The old PluginApi free_string comment contradicts EventView by suggesting plugin ownership of event payloads; following it would double-free host-owned memory.

Native timeout wrappers retain library/session handles, but aborting a started spawn_blocking task does not stop its callback. Its event buffers remain task-owned until it returns; its actions must still be stale/cancel-fenced. Outer catch_unwind cannot safely catch a panic that unwinds through an unguarded extern C plugin callback; plugins must use the SDK guard at that boundary. These are source-derived risks, not executed crash/leak tests.

### 5. Interfaces

```ts
export interface AgentEndEvent {
	type: "agent_end";
	messages: AgentMessage[];
}
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:771-774.
```ts
export interface TurnStartEvent {
	type: "turn_start";
	turnIndex: number;
	timestamp: number;
}
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:782-786.
```ts
export interface TurnEndEvent {
	type: "turn_end";
	turnIndex: number;
	message: AgentMessage;
	toolResults: ToolResultMessage[];
}
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:789-794.
```ts
export interface MessageUpdateEvent {
	type: "message_update";
	message: AgentMessage;
	assistantMessageEvent: AssistantMessageEvent;
}
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:803-807.
```ts
export interface ToolExecutionStartEvent {
	type: "tool_execution_start";
	toolCallId: string;
	toolName: string;
	args: any;
}
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:816-821.
```ts
export interface ToolExecutionUpdateEvent {
	type: "tool_execution_update";
	toolCallId: string;
	toolName: string;
	args: any;
	partialResult: any;
}
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:824-830.
```ts
export interface ToolExecutionEndEvent {
	type: "tool_execution_end";
	toolCallId: string;
	toolName: string;
	result: any;
	isError: boolean;
}
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:833-839.
None: additional fixed CLI flags, slash commands, keybindings, settings, environment variables, RPC commands or persistent files beyond the explicitly listed registration/host contracts. Each dynamically registered name is chosen by the extension.

Minimal lifecycle schemas also include `AgentStartEvent { type: 'agent_start' }`, `AgentSettledEvent { type: 'agent_settled' }` and `MessageStartEvent { type: 'message_start'; message: AgentMessage }`. The settled marker carries no additional DTO fields; scheduling and retry completion remain runtime-owned (ice:packages/coding-agent/src/core/extensions/types.ts:766-779; ice:packages/coding-agent/src/core/extensions/types.ts:797-800).

### 6. Dependencies

`EXT-022`, `EXT-023`, `EXT-024`, `EXT-025`, `EXT-028`, `EXT-029`, `EXT-030`, `EXT-031`, `EXT-032`

External services/binaries: none required by this host mechanism. Plugins may invoke arbitrary host I/O because they are trusted executable code; this audit did not load or run any plugin.

### 7. ice-rust today

rust:crates/ice-extensions/src/translate.rs:32-265; rust:crates/ice-extensions/src/translate.rs:342-447; rust:crates/ice-extensions/src/translate.rs:905-968; rust:crates/ice-plugin-sdk/src/events.rs:21-134; rust:crates/ice-plugin-sdk/src/lib.rs:1371-1375. Native serialization drops turn/agent payloads, partialResult, usage and full message_update shape. dispatch_to_handlers returns on stale state before freeing already allocated input; BeforeAgentStart is built through dispatch_data_event but listed in the no-payload cleanup branch. EventView safely copies borrowed strings, whereas PluginApi.free_string documentation incorrectly includes event payloads among allocations a plugin should free. These concrete source conflicts need distinct ownership fixes; callbacks were not executed in this audit.

### 8. Port notes

Replace ad hoc event allocation/free switches with one RAII event owner that retains the initialized payload arm independently of the external tag. Free before every stale/empty early return and cover BeforeAgentStart data payload cleanup; correct the SDK free_string documentation. Add field-complete DTOs and producer-specific output guards, then use dedicated lifetime/allocator tests for cancellation, late callbacks and unload only after this audit is approved for implementation.

### 9. Size and priority

Estimated Rust implementation: **200-350 lines** (estimate, not measured source). **P0** - The native ABI must not lose authoritative event data, leak per-event memory or invite double-free/late-session mutation.

## EXT-034 - Project-trust decision hook and malformed-return behavior

### 1. Title and status

Project-trust decision hook and malformed-return behavior. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/extensions/runner.ts:223-253; ice:packages/coding-agent/src/core/extensions/types.ts:573-596. emitProjectTrustEvent. Test: ice:packages/coding-agent/test/extensions-runner.test.ts:137-172 (`continues past undecided handlers and returns the first yes/no decision`).

Exported symbols: `ProjectTrustHandler` (ice:packages/coding-agent/src/core/extensions/types.ts:592).

### 3. What it does

Ask already-admitted extensions for a project trust decision before loading project executable resources.

### 4. How it works

Pre-trust handlers can return a three-way project trust decision before ordinary project resources are admitted. The standalone dispatcher reports handler errors and leaves the final trust policy to its caller.

The standalone emitProjectTrustEvent iterates loaded extensions and their project_trust handlers with a restricted ProjectTrustContext. Await each; only an exact trusted:"undecided" continues. Any other returned object is accepted immediately without validating yes/no. Returning undefined/null throws when accessing trusted, records an error and continues. A malformed object such as {} returns as a decision rather than being treated undecided. Exceptions accumulate in a returned errors array, not the runner error-listener channel.

Return {result, errors} for the first non-undecided result or {errors} when none. No timeout, signal, default trust decision, persistence or loading of untrusted project plugins occurs here. The caller must validate/admit the decision and preserve fail-closed headless policy. Do not use repository-local executable configuration to decide whether that same repository is trusted.

### 5. Interfaces

```ts
export interface ProjectTrustEvent {
	type: "project_trust";
	cwd: string;
}
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:573-576.
```ts
export interface ProjectTrustContext {
	cwd: string;
	mode: ExtensionMode;
	hasUI: boolean;
	ui: Pick<ExtensionUIContext, "select" | "confirm" | "input" | "notify">;
}
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:585-590.
```ts
export interface ProjectTrustEventResult {
	trusted: ProjectTrustEventDecision;
	remember?: boolean;
}
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:580-583.
```ts
export async function emitProjectTrustEvent(
	extensionsResult: LoadExtensionsResult,
	event: ProjectTrustEvent,
	ctx: ProjectTrustContext,
): Promise<{ result?: ProjectTrustEventResult; errors: ExtensionError[] }> {
	const errors: ExtensionError[] = [];
	for (const ext of extensionsResult.extensions) {
		// A single extension may register multiple handlers for the same event.
		// The first project_trust handler that returns yes/no wins; undecided falls through.
		const handlers = ext.handlers.get("project_trust");
		if (!handlers || handlers.length === 0) continue;

		for (const handler of handlers) {
			try {
				const handlerResult = (await handler(event, ctx)) as ProjectTrustEventResult;
				if (handlerResult.trusted === "undecided") {
					continue;
				}
				return { result: handlerResult, errors };
			} catch (error) {
				errors.push({
					extensionPath: ext.path,
					event: event.type,
					error: error instanceof Error ? error.message : String(error),
					stack: error instanceof Error ? error.stack : undefined,
				});
			}
		}
	}
	return { errors };
}
```
Source: ice:packages/coding-agent/src/core/extensions/runner.ts:223-253.
None: additional fixed CLI flags, slash commands, keybindings, settings, environment variables, RPC commands or persistent files beyond the explicitly listed registration/host contracts. Each dynamically registered name is chosen by the extension.

### 6. Dependencies

`EXT-001`, `EXT-004`, `EXT-006`, `SET-003`

External services/binaries: none required by the hook. SET-003 owns project-trust resolution/CLI policy and SET-004 owns canonical-path trust storage; this entry owns extension decision dispatch and the missing Rust hook wiring.

### 7. ice-rust today

rust:crates/ice-plugin-sdk/src/lib.rs:590-645; rust:crates/ice-cli/src/session.rs:1847-1901; rust:crates/ice-cli/src/config.rs:471-499; rust:crates/ice-cli/src/args.rs:477-478. ProjectTrust is tag 0, but a crates-wide search finds no host emitter for it: only SDK enumeration/tests and the cleanup switch. Native trust resolution uses --approve|-a / --no-approve|-na when supplied, otherwise stored canonical-path decisions, and defaults missing/error cases to true. It does not invoke global pre-trust handlers or default unresolved headless projects to denied. Ordinary denied loading selects global dirs, but --project-only selects project dirs before testing project_trusted, and explicit directories/files are added afterward. Stored decisions use the existing atomic trust writer; resolve_project_trust itself does not persist a new decision. The earlier draft's ICE_PROJECT_TRUST, tag 27, claimed-handler trust and headless-deny claims were unsupported and have been removed.

### 8. Port notes

Add a return-valued pre-project trust hook to ice-extensions with trusted yes|no|undecided plus an optional reason, and invoke only already-admitted global/inline policies before project code can load. Connect the decision to ice-cli resource admission and persist only through the existing trust store. Explicitly review unknown-project default and --project-only/explicit-path overrides; a claimed Input-style event is not a three-way decision.

### 9. Size and priority

Estimated Rust implementation: **100-180 lines** (estimate, not measured source). **P0** - The current default/admission behavior can execute project extension code before any explicit plugin trust decision; document and decide this boundary before claiming parity.

## EXT-035 - Extension tool metadata, argument preparation and execution policy

### 1. Title and status

Extension tool metadata, argument preparation and execution policy. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/extensions/types.ts:493-540; ice:packages/coding-agent/src/core/tools/tool-definition-wrapper.ts:1-47; ice:packages/coding-agent/src/core/agent-session.ts:1118-1158; ice:packages/coding-agent/src/core/agent-session.ts:1246-1310; ice:packages/coding-agent/src/core/agent-session.ts:3036-3135. ToolDefinition, tool-definition wrapper, tool metadata and system-prompt construction.

### 3. What it does

Let a plugin declare how its tool is described, prepared, scheduled and exposed to plan/build policy, in addition to providing an executable callback.

### 4. How it works

An extension tool carries behavior and policy metadata beyond a JSON parameter schema. The host passes execution metadata into the agent tool and separately derives the active tool descriptions and guidelines used in its system prompt.

ToolDefinition requires name, label, description, parameters and asynchronous execute; optional fields are promptSnippet, promptGuidelines, constrainedSampling, renderShell, prepareArguments, executionMode and access. The wrapper retains prepareArguments and executionMode, passes execution through the extension context wrapper, and preserves constrained sampling configuration. prepareArguments is a compatibility callback intended before validation; registering it does not itself execute or validate a call. Execution scheduling and schema validation belong to the agent/tool layer, so its failure and concurrency rules must remain shared rather than being reimplemented in the extension loader.

The session normalizes a prompt snippet by collapsing whitespace; guideline strings are trimmed, empty strings dropped and exact duplicates removed. Only active tools contribute their metadata to the rebuilt base prompt; an active prompt override still wins. getAllTools projects name, description, parameters, promptGuidelines, access when provided, and sourceInfo; label and snippet are not exposed there. The access declaration is read|write or absent, with the documented plan/build restriction; this entry does not elevate a declaration into proof of authorization. Loader validation remains the narrow object-schema check in EXT-011. Metadata registration is in-memory, not independently persisted, and late refresh can fail after replacing a descriptor.

### 5. Interfaces

```ts
export interface ToolDefinition<TParams extends TSchema = TSchema, TDetails = unknown, TState = any> {
	/** Tool name (used in LLM tool calls) */
	name: string;
	/** Human-readable label for UI */
	label: string;
	/** Description for LLM */
	description: string;
	/** Optional one-line snippet for the Available tools section in the default system prompt. Custom tools are omitted from that section when this is not provided. */
	promptSnippet?: string;
	/** Optional guideline bullets appended to the default system prompt Guidelines section when this tool is active. */
	promptGuidelines?: string[];
	/** Parameter schema (TypeBox) */
	parameters: TParams;
	/** Optional provider-side constrained sampling request for this tool. Set false to explicitly disable it, equivalent to leaving it undefined. */
	constrainedSampling?: false | ConstrainedSamplingConfig;
	/** Controls whether ToolExecutionComponent renders the standard colored shell or the tool renders its own framing. */
	renderShell?: "default" | "self";

	/** Optional compatibility shim to prepare raw tool call arguments before schema validation. Must return an object conforming to TParams. */
	prepareArguments?: (args: unknown) => Static<TParams>;

	/**
	 * Per-tool execution mode override.
	 * - "sequential": this tool must execute one at a time with other tool calls.
	 * - "parallel": this tool can execute concurrently with other tool calls.
	 *
	 * If omitted, the default execution mode applies.
	 */
	executionMode?: ToolExecutionMode;

	/**
	 * ICE mode policy class for an extension tool.
	 * - "read": never mutates workspace or external state; ICE exposes it in plan and build mode.
	 * - "write": may mutate state; ICE exposes it only in a trusted build session.
	 *
	 * If omitted, ICE plan/build modes keep the tool inactive. Package tools stay behind
	 * `--allow-external` regardless of this declaration. The tool must still enforce its own authorization.
	 */
	access?: "read" | "write";

	/** Execute the tool. */
	execute(
		toolCallId: string,
		params: Static<TParams>,
		signal: AbortSignal | undefined,
		onUpdate: AgentToolUpdateCallback<TDetails> | undefined,
		ctx: ExtensionContext,
	): Promise<AgentToolResult<TDetails>>;
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:493-540.
No fixed tool name is imposed: each extension chooses its tool name. The exact optional fields and enum values above are public contracts; scheduling, constrained sampling and plan/build enforcement depend on the shared agent and tools contracts.

### 6. Dependencies

`AGENT-010`, `AGENT-012`, `AI-012`, `AI-013`, `EXT-011`, `EXT-012`

External services/binaries: none required for registration. Resolve agent scheduling, constrained sampling, argument validation and plan/build policy to their existing area entries during publication.

Final dependency classification. Shared contract references: `TOOLS-018`, `SESSION-023`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

rust:crates/ice-plugin-sdk/src/lib.rs:348-353; rust:crates/ice-extensions/src/lib.rs:343-415; rust:crates/ice-extensions/src/tool.rs:300-395. StableToolSchema contains only name, description and parameters. The registrar builds an owned schema with constrained_sampling fixed to None. ExtensionTool uses the tool name as its label and reports parallel execution; there is no appended native metadata field or preparation callback for the TypeScript optional fields. Native execution and cancellation are real (EXT-012), but their presence does not supply these declarations. Native registries being immutable after loading also prevents the TypeScript live-refresh behavior.

### 8. Port notes

Append a size-checked tool registration descriptor in ice-plugin-sdk and store owned metadata in ice-extensions. Connect preparation and execution policy to ice-agent's existing Tool path; add policy classification to the existing tool admission layer and prompt metadata to the current prompt builder. Keep the old ABI defaults explicit, distinguish absent constrained sampling from a populated config, validate metadata once, and define who frees prepared JSON. Preserve old plugins through appended optional API fields rather than changing the existing struct layout in place.

### 9. Size and priority

Estimated Rust implementation: **160-300 lines** (estimate, not measured source). **P1** - Missing scheduling and access metadata changes which tools can run and how concurrent calls are admitted.

## EXT-036 - Per-tool render callbacks, row state and fallback rendering

### 1. Title and status

Per-tool render callbacks, row state and fallback rendering. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/core/extensions/types.ts:455-488; ice:packages/coding-agent/src/core/extensions/types.ts:542-552; ice:packages/coding-agent/src/modes/interactive/components/tool-execution.ts:1-631. ToolRenderContext, ToolRenderResultOptions, ToolExecutionComponent. Tests: ice:packages/coding-agent/test/tool-execution-component.test.ts:38-120; ice:packages/coding-agent/test/tool-execution-component.test.ts:209-365.

### 3. What it does

Allow a tool to render its call and result with persistent row state while preserving generic output when a renderer fails.

### 4. How it works

Interactive tool rows can use independent call and result renderers supplied by the tool definition. They receive one shared state object for the row and a separate previous component for each render slot, so a plugin can update an existing display without losing execution state.

The component selects extension renderCall/renderResult individually, falling back to available built-in renderers and then generic formatting. renderShell defaults to the standard colored shell; self delegates framing to the returned component while host spacing and image handling remain. Render context includes current args, toolCallId, invalidate, lastComponent, state, cwd, executionStarted, argsComplete, isPartial, expanded, showImages and isError. invalidate clears cached rendering and requests a redraw; the state object survives subsequent updates. Result callbacks are actually passed {content, details}, plus {expanded, isPartial}; usage is not included in that supplied result object, and isError is supplied through context.

Callback exceptions fall back to generic tool rendering. The host appends image rendering rather than delegating all image handling to the plugin. Compact display uses separate previews: ordinary content is limited to five lines, edit/write preview to fourteen, running output to a three-line tail, and relevant error/bash branches choose their own head views; expansion requests the fuller renderer. Completed render output is cached by available width until invalidated. Registration alone causes no process execution, persistence or RPC callback evaluation. This contract is specific to transcript tool rows, not the custom message renderer registry.

### 5. Interfaces

```ts
export interface ToolRenderResultOptions {
	/** Whether the result view is expanded */
	expanded: boolean;
	/** Whether this is a partial/streaming result */
	isPartial: boolean;
}

/** Context passed to tool renderers. */
export interface ToolRenderContext<TState = any, TArgs = any> {
	/** Current tool call arguments. Shared across call/result renders for the same tool call. */
	args: TArgs;
	/** Unique id for this tool execution. Stable across call/result renders for the same tool call. */
	toolCallId: string;
	/** Invalidate just this tool execution component for redraw. */
	invalidate: () => void;
	/** Previously returned component for this render slot, if any. */
	lastComponent: Component | undefined;
	/** Shared renderer state for this tool row. Initialized by tool-execution.ts. */
	state: TState;
	/** Working directory for this tool execution. */
	cwd: string;
	/** Whether the tool execution has started. */
	executionStarted: boolean;
	/** Whether the tool call arguments are complete. */
	argsComplete: boolean;
	/** Whether the tool result is partial/streaming. */
	isPartial: boolean;
	/** Whether the result view is expanded. */
	expanded: boolean;
	/** Whether inline images are currently shown in the TUI. */
	showImages: boolean;
	/** Whether the current result is an error. */
	isError: boolean;
}
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:455-488.
```ts
	/** Custom rendering for tool call display */
	renderCall?: (args: Static<TParams>, theme: Theme, context: ToolRenderContext<TState, Static<TParams>>) => Component;

	/** Custom rendering for tool result display */
	renderResult?: (
		result: AgentToolResult<TDetails>,
		options: ToolRenderResultOptions,
		theme: Theme,
		context: ToolRenderContext<TState, Static<TParams>>,
	) => Component;
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:542-551.
The renderShell field is exactly "default" | "self" (ice:packages/coding-agent/src/core/extensions/types.ts:508-509). No new slash command or setting is registered by these callbacks.

### 6. Dependencies

`EXT-035`

External services/binaries: none. Depends on the interactive transcript component/expansion contracts to be assigned by the TUI audit.

Final dependency classification. Consuming integrations: `TUI-031`, `TUI-032`, `TUI-229`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

rust:crates/ice-plugin-sdk/src/lib.rs:348-353; rust:crates/ice-extensions/src/registry.rs:413-427; rust:crates/ice-cli/src/interactive_tui/sessions.rs:425-610. Native tool schemas and executable registrations contain no call/result renderer callback, per-row plugin state, invalidation callback or render-shell choice. Native renderer kinds cover messages, entries and Markdown and do not implement a tool-row callback surface. JSON-rendered custom messages are therefore a related UI mechanism but cannot render an existing tool call through this contract.

### 8. Port notes

Define an owned, versioned tool-render request/response protocol in ice-plugin-sdk and route it through ice-tui's existing tool rows. Prefer declarative render output with explicit row state handles over foreign Component pointers. Give state and cached output runtime ownership, release them on row removal/reload, retain library keepalive across calls, and bound callback work. Implement call/result fallbacks and invalidation before enabling plugins to replace tool framing.

### 9. Size and priority

Estimated Rust implementation: **240-440 lines** (estimate, not measured source). **P2** - Custom tools currently lose their intended readable progress and result presentation.

## EXT-037 - Custom message renderer selection, expansion and fallback

### 1. Title and status

Custom message renderer selection, expansion and fallback. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/extensions/loader.ts:359-373; ice:packages/coding-agent/src/core/extensions/runner.ts:640-662; ice:packages/coding-agent/src/core/extensions/types.ts:1190-1212; ice:packages/coding-agent/src/core/extensions/types.ts:1359-1363; ice:packages/coding-agent/src/modes/interactive/components/custom-message.ts:1-239; ice:packages/coding-agent/src/modes/interactive/interactive-mode.ts:4646-4740. Tests: ice:packages/coding-agent/test/custom-message.test.ts:1-114.

### 3. What it does

Render an extension's displayed custom messages according to their customType, while retaining a useful default when no renderer succeeds.

### 4. How it works

A custom message renderer is registered under an exact customType and is selected for displayed custom messages of that type. Rendering receives the original message, expansion state, horizontal output padding and current theme; a failed or absent renderer falls back to the host's default custom-message view.

Registration overwrites the same owner's map entry, while runner lookup returns the first loaded owner with that type. The interactive transcript creates a custom message row only when display is true. CustomMessageComponent calls the selected renderer before its built-in appearance logic, accepts a returned component, and catches thrown errors or falsy returns to use its default. The default includes the custom-type label and text content rendered as Markdown; it is not a generic image renderer. Expanded state and outputPad changes rebuild the rendered view. The renderer does not modify the session record or decide whether the message reaches LLM context.

Built-in background-completion and subagent-completion views have their own compact presentation: the latter keeps a four-line head and three-line tail when sufficient lines would be hidden, with expansion restoring content. Those are host fallbacks, not registration names enforced by the extension API. Rendering runs synchronously in the UI, has no callback deadline, and the registry does not provide a renderer unsubscribe API. Loading/replacing an extension rebuilds the registry through runtime lifecycle handling.

### 5. Interfaces

```ts
export interface MessageRenderOptions {
	expanded: boolean;
	/** Horizontal padding configured by the outputPad setting. */
	outputPad: number;
}
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:1190-1194.
```ts
export type MessageRenderer<T = unknown> = (
	message: CustomMessage<T>,
	options: MessageRenderOptions,
	theme: Theme,
) => Component | undefined;
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:1208-1212.
```ts
	/** Register a custom renderer for CustomMessageEntry. */
	registerMessageRenderer<T = unknown>(customType: string, renderer: MessageRenderer<T>): void;
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:1359-1360.
The renderer key is the exact dynamically supplied customType. Message delivery fields and persistence are covered by EXT-051.

### 6. Dependencies

`EXT-006`, `EXT-010`, `EXT-051`

External services/binaries: none. Uses the interactive transcript's existing Component and theme abstractions.

### 7. ice-rust today

rust:crates/ice-extensions/src/registry.rs:255-275; rust:crates/ice-extensions/src/registry.rs:705-718; rust:crates/ice-extensions/src/registry.rs:1016-1026; rust:crates/ice-cli/src/interactive_tui/sessions.rs:470-559. Native registration retains renderer kind and name with first-wins collisions, but invoke_extension_renderer filters by kind only and ignores the registered name/customType. It returns the first successfully parsed JSON result even if subsequent render decoding cannot display it. Therefore an unrelated earlier Message renderer can capture another custom type's message and prevent a later matching renderer from being used. Input is serialized JSON rather than a typed Component callback; the TypeScript expansion/outputPad/theme contract is not supplied. Errors and invalid JSON are skipped, while successful-but-unrenderable JSON is a distinct failure path.

### 8. Port notes

First correct renderer dispatch in ice-cli to match kind and exact registered type before invocation, preserving declared precedence. Extend the existing native renderer DTO with message, expansion and layout/theme tokens; retain library keepalive and runtime liveness through the callback. Distinguish a deliberate decline from malformed render data so fallback can continue deterministically. Keep session data immutable and reuse ice-tui's generic custom-message fallback.

### 9. Size and priority

Estimated Rust implementation: **100-190 lines** (estimate, not measured source). **P1** - Current native renderer lookup can invoke a renderer registered for a different custom message type.

## EXT-038 - Persistent custom entry rendering and hidden-by-default records

### 1. Title and status

Persistent custom entry rendering and hidden-by-default records. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/extensions/types.ts:1204-1218; ice:packages/coding-agent/src/core/extensions/types.ts:1365-1366; ice:packages/coding-agent/src/core/extensions/loader.ts:359-373; ice:packages/coding-agent/src/core/extensions/runner.ts:640-651; ice:packages/coding-agent/src/modes/interactive/components/custom-entry.ts:1-63; ice:packages/coding-agent/src/modes/interactive/interactive-mode.ts:4603-4644. CustomEntryComponent and getEntryRenderer.

### 3. What it does

Optionally display extension state entries in the transcript without adding those entries to the model's conversation.

### 4. How it works

Custom entries are persistent extension state records that remain hidden unless an exact customType renderer is available. The renderer receives the original entry and current expansion/theme information, and its result controls whether any visible component exists.

The loader replaces a same-owner renderer by exact key; lookup selects the first owner. The transcript skips records with no matching renderer. CustomEntryComponent rebuilds from the retained entry with {expanded}, uses the current theme and tracks whether a component was produced. Falsy return hides the content. Unlike the custom-message renderer fallback, a thrown entry renderer becomes a visible error component describing the rendering failure. The surrounding host supplies transcript spacing; rebuilding does not explicitly dispose the previously returned Component.

setExpanded reruns the renderer. The renderer cannot turn the record into model context merely by showing it, and append/durable/label semantics remain session operations in EXT-053. No static customType names, file names or schema for extension-owned data are imposed. Plugins must read and interpret their own data version, and the UI callback has no timeout, clone or concurrency isolation.

### 5. Interfaces

```ts
export interface EntryRenderOptions {
	expanded: boolean;
}
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:1204-1206.
```ts
export type EntryRenderer<T = unknown> = (
	entry: CustomEntry<T>,
	options: EntryRenderOptions,
	theme: Theme,
) => Component | undefined;
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:1214-1218.
```ts
	/** Register a custom renderer for CustomEntry. Custom entries do not participate in LLM context. */
	registerEntryRenderer<T = unknown>(customType: string, renderer: EntryRenderer<T>): void;
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:1365-1366.

### 6. Dependencies

`EXT-010`, `EXT-053`

External services/binaries: none. The session audit owns the persisted CustomEntry record format and branch traversal semantics.

### 7. ice-rust today

rust:crates/ice-cli/src/interactive_tui/sessions.rs:470-508; rust:crates/ice-cli/src/interactive_tui/sessions.rs:581-610; rust:crates/ice-extensions/src/registry.rs:255-275; rust:crates/ice-extensions/src/registry.rs:705-718; rust:crates/ice-extensions/src/registry.rs:1016-1026. Rust has Entry renderer registration and emits customType/data JSON to that channel, but uses the same kind-only dispatch defect as Message renderers. It omits the original entry ID, parent/timestamp metadata, expansion option and theme callback context. A successfully parsed but unusable first renderer result can hide later candidates. Native JSON output may be a deliberate ABI representation, but neither its type routing nor its omitted record fields match TypeScript.

### 8. Port notes

Share exact-name routing and callback lifetime fixes with EXT-037. Pass a versioned complete entry DTO plus view options through the existing Entry renderer kind; distinguish hidden return from a renderer error and give the UI a deterministic error view. Keep state entries out of LLM messages and document render cache/disposal ownership rather than exposing native pointers into the session tree.

### 9. Size and priority

Estimated Rust implementation: **80-160 lines** (estimate, not measured source). **P1** - Native state entries can be rendered by the wrong plugin and lack identity information needed for reliable state views.

## EXT-039 - Ordered Markdown transformation with message and layout context

### 1. Title and status

Ordered Markdown transformation with message and layout context. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/extensions/loader.ts:369-373; ice:packages/coding-agent/src/core/extensions/runner.ts:654-662; ice:packages/coding-agent/src/core/extensions/types.ts:1196-1202; ice:packages/coding-agent/src/core/extensions/types.ts:1362-1363; ice:packages/coding-agent/src/modes/interactive/components/markdown-transform.ts:1-30. Tests: ice:packages/coding-agent/test/user-message.test.ts:1-67; ice:packages/coding-agent/test/assistant-message.test.ts:120-257.

### 3. What it does

Transform displayed user, assistant and assistant-thinking Markdown before rendering, without changing the stored or provider-facing message.

### 4. How it works

Each extension may install one Markdown transformer; all installed transformers then form an ordered display-only chain. Each receives the current Markdown and a context identifying message type, streaming state and available width, so the same stored message can render differently as layout changes.

A later registration by one owner replaces its prior transformer. getMarkdownTransformers traverses extensions in load order. The component helper synchronously invokes each transformer with the current string; only string results replace it, including the empty string, while nonstrings are ignored. If a transformer throws, the helper continues with the existing text and later transformers. User and assistant components apply this at render time, with assistant thinking using its own messageType and streaming/width changes triggering appropriate rendering. Tests explicitly check ordering, streaming context, width context, continuing after exceptions and preserving the underlying message.

The transform cannot add provider messages, change durable session text or consume an input event. No async return is awaited, no deadline is enforced and no owner-specific deregistration handle exists. It shares UI scheduling and therefore a synchronous nonreturning transformer can block rendering. This is distinct from the provider payload/context transformation hooks in EXT-027 and EXT-028.

### 5. Interfaces

```ts
export interface MarkdownTransformContext {
	messageType: "user" | "assistant" | "assistant-thinking";
	isStreaming: boolean;
	availableWidth: number;
}

export type MarkdownTransformer = (markdown: string, context: MarkdownTransformContext) => string;
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:1196-1202.
```ts
	/** Register a transformer for user and assistant Markdown before Ice renders it in the interactive transcript. */
	registerMarkdownTransformer(transformer: MarkdownTransformer): void;
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:1362-1363.

### 6. Dependencies

`EXT-006`, `EXT-010`

External services/binaries: none. User/assistant component invalidation and Markdown rendering belong to the TUI area.

### 7. ice-rust today

rust:crates/ice-cli/src/interactive_tui.rs:110-157; rust:crates/ice-extensions/src/registry.rs:255-275; rust:crates/ice-extensions/src/registry.rs:705-718; rust:crates/ice-extensions/src/registry.rs:1016-1026. Rust registers Markdown renderer callbacks and chains JSON {markdown} requests. The callback receives no messageType, isStreaming or availableWidth. Nonzero return and panic stop the remaining chain, while invalid JSON or absent output Markdown are skipped. That failure ordering differs from TypeScript's catch-and-continue contract. The closure captures renderer registrations; library/runtime lifetime ownership must be verified before making callbacks concurrent with reload, rather than assuming a raw function pointer itself keeps a plugin loaded.

### 8. Port notes

Extend the existing Markdown request with explicit context and a version field; preserve string-only output and empty-string replacement. Catch callback failure and continue from the last valid Markdown according to the chosen compatibility contract. Hold runtime/library ownership for each chain invocation, invalidate when relevant context changes, and use a bounded execution policy appropriate to the current synchronous TUI render loop.

### 9. Size and priority

Estimated Rust implementation: **80-150 lines** (estimate, not measured source). **P2** - Plugins cannot reproduce contextual formatting, and one failed native transformer currently prevents later transformers from running.

## EXT-040 - Extension dialogs, cancellation and interactive/RPC projection

### 1. Title and status

Extension dialogs, cancellation and interactive/RPC projection. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/extensions/types.ts:97-105; ice:packages/coding-agent/src/core/extensions/types.ts:154-162; ice:packages/coding-agent/src/core/extensions/types.ts:248-249; ice:packages/coding-agent/src/core/extensions/runner.ts:255-286; ice:packages/coding-agent/src/modes/interactive/interactive-mode.ts:3245-3423; ice:packages/coding-agent/src/modes/interactive/components/extension-selector.ts:1-129; ice:packages/coding-agent/src/modes/interactive/components/extension-input.ts:1-90; ice:packages/coding-agent/src/modes/interactive/components/extension-editor.ts:1-133; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:280-441. Tests: ice:packages/coding-agent/test/interactive-mode-status.test.ts:374-415.

### 3. What it does

Let an extension ask a selector, yes/no question, single-line input or multiline editor question and receive a cancellable result when the current mode supports it.

### 4. How it works

Dialog calls are asynchronous UI requests whose cancellation defaults are undefined for text/selection and false for confirmation. Interactive and RPC modes implement the same public signatures through different transports, while the no-UI context resolves immediately without displaying a dialog.

select, confirm and input take optional AbortSignal, timeout and Markdown content. Interactive mode checks an already-aborted signal, installs a one-shot abort listener, renders the dialog and cleans it up when submitted/cancelled/timed out. Confirmation maps the Yes choice to true; all cancellation paths are false. The selector consumes content and configured timeout. The input component accepts a placeholder argument but its implementation names it _placeholder and does not display it, so documented placeholder behavior is not actually present there. editor(title, prefill) has no dialog options or deadline and supports the component's external-editor path, selecting VISUAL, then EDITOR, then its platform fallback.

RPC uses request IDs, a pending-request map and extension_ui_request/extension_ui_response messages. It removes pending state on settlement and returns the same cancellation defaults; timeout is tested by truthiness, not a positive-value validator. Abort is local settlement, with no paired wire cancellation message from this path. Admission/shutdown may reject new control requests before sending them. These queues are not durable session records. The public no-UI implementation returns undefined/false, and custom editor creation is a separate EXT-045 contract.

### 5. Interfaces

```ts
/** Options for extension UI dialogs. */
export interface ExtensionUIDialogOptions {
	/** AbortSignal to programmatically dismiss the dialog. */
	signal?: AbortSignal;
	/** Timeout in milliseconds. Dialog auto-dismisses with live countdown display. */
	timeout?: number;
	/** Optional Markdown content to review before choosing an option. */
	content?: string;
}
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:97-105.
```ts
	/** Show a selector and return the user's choice. */
	select(title: string, options: string[], opts?: ExtensionUIDialogOptions): Promise<string | undefined>;

	/** Show a confirmation dialog. */
	confirm(title: string, message: string, opts?: ExtensionUIDialogOptions): Promise<boolean>;

	/** Show a text input dialog. */
	input(title: string, placeholder?: string, opts?: ExtensionUIDialogOptions): Promise<string | undefined>;
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:155-162.
```ts
	/** Show a multi-line editor for text editing. */
	editor(title: string, prefill?: string): Promise<string | undefined>;
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:248-249.
Interactive editor environment selection: VISUAL, then EDITOR (ice:packages/coding-agent/src/modes/interactive/components/extension-editor.ts:53-57). The RPC wire implementation is cited above; its common envelope and admission limits belong to the RPC audit.

### 6. Dependencies

`EXT-009`, `EXT-010`

External binary: the configured external editor only when that interactive editor action is invoked. Requires RPC request/response correlation and TUI modal focus contracts from their area audits.

Final dependency classification. Consuming integrations: `RPC-028`. Shared contract references: `TUI-073`, `TUI-050`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

rust:crates/ice-extensions/src/actions.rs:1-360; rust:crates/ice-plugin-sdk/src/dialogs.rs:1-106; rust:crates/ice-cli/src/interactive_tui/extensions.rs:77-495; rust:crates/ice-cli/src/interactive_tui/extensions.rs:529-939. Rust has real command-result selector/input/editor dialogs and a separate UiDialog mailbox for ask_user. The mailbox supports open/poll/cancel/respond with requestId, pending/terminal states, UI detach cancellation and remembered cancelled tool-call IDs; terminal polling consumes the result. It rejects opening without an attached UI rather than returning a fabricated answer. Native selectors show ten visible items, preserve a real input placeholder, and return structured values/kind JSON rather than the TypeScript scalar/boolean contracts. Confirm defaults to Yes/No choices but returns selector-shaped values, not boolean. Permission-purpose dialogs validate explicit allow/reject options and a toolCallId. There is no shared TypeScript AbortSignal/timeout/content contract, and the request mailbox is unbounded. A command dialog may resume its command with encoded action/value arguments instead of resolving a factory Promise.

### 8. Port notes

Build a compatibility facade on the existing UiDialog mailbox, with typed select/confirm/input/editor requests and scalar response decoding. Add absolute deadlines, cancellation on owner invalidation, bounded outstanding requests and deterministic terminal cleanup. Preserve native permission-purpose validation and UI-not-attached errors as explicit capability behavior, while translating TypeScript cancellation defaults only at the compatibility boundary. Reuse current TUI modal components and define RPC transport projection separately.

### 9. Size and priority

Estimated Rust implementation: **220-400 lines** (estimate, not measured source). **P1** - Dialog result shape and cancellation differences can leave plugins waiting or interpret user consent incorrectly.

## EXT-041 - Extension notifications, identity and sticky notices

### 1. Title and status

Extension notifications, identity and sticky notices. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/core/extensions/types.ts:107-116; ice:packages/coding-agent/src/core/extensions/types.ts:164-165; ice:packages/coding-agent/src/modes/interactive/interactive-mode.ts:3497-3533; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:400-605; ice:packages/coding-agent/src/core/extensions/runner.ts:255-286. ExtensionNotifyOptions and mode UI implementations.

### 3. What it does

Show extension informational messages and warnings, with optional stable notice identity and sticky retention in interactive mode.

### 4. How it works

notify delivers a UI message immediately and does not append an LLM message or durable session entry. The interactive implementation distinguishes identified notices from ordinary informational status and warning/error output; RPC forwards a simpler notification request.

The public level is info|warning|error, defaulting to info. Options carry id and sticky, with the type documentation requiring id when sticky is used. The implementation does not enforce that relation through a runtime validator. An identified interactive warning or error is entered in the notice service under that identity, using sticky when supplied. Notices are categorized according to the message/session or MCP context checks in the host. Without an ID, warning/error notification creates a notice and the host's visible warning/error output; information uses the ordinary status path. An ID is not a session-entry key and does not make the notification durable.

The RPC implementation emits extension_ui_request with method notify, message and notifyType; id/sticky options are not represented by this call path. The no-UI implementation does nothing. notify returns void, has no completion acknowledgement or abort/deadline, and any host exceptions propagate through the surrounding extension callback/error handling. Lifecycle reset owns interactive notice/UI cleanup separately from extension registration.

### 5. Interfaces

```ts
/** Options for extension notifications. */
export interface ExtensionNotifyOptions {
	/**
	 * Stable notice id. Interactive mode deduplicates notices with the same id and
	 * shows warnings/errors that carry one in the notices badge instead of the transcript.
	 */
	id?: string;
	/** Notice describes ongoing process state and cannot be dismissed. Requires `id`. */
	sticky?: boolean;
}
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:107-116.
```ts
	/** Show a notification to the user. */
	notify(message: string, type?: "info" | "warning" | "error", options?: ExtensionNotifyOptions): void;
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:164-165.
RPC notification projection is the notify method in ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:400-605; it carries message and notifyType, without the options object.

### 6. Dependencies

`EXT-009`, `EXT-010`

External services/binaries: none. The TUI notice system and RPC common message envelope are cross-area dependencies.

Final dependency classification. Shared contract references: `TUI-052`, `TUI-053`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

rust:crates/ice-plugin-sdk/src/lib.rs:1010-1084; rust:crates/ice-extensions/src/actions.rs:984-1265; rust:crates/ice-extensions/src/status.rs:1-416. The complete native runtime action list has SetStatus, UiDialog and UiComponent but no extension notification action with severity, ID or sticky lifecycle. Native logging is a diagnostic callback and status text/panels are persistent UI state; neither supplies the transient notification contract. Existing native dialogs are interactive requests, not informational notices. A behavior-based search and complete SDK/action inspection support the absent host API rather than relying only on method-name mismatch.

### 8. Port notes

Add a versioned Notify runtime action in ice-plugin-sdk and an owner-scoped request in ice-extensions, routed to ice-tui's notice model. Define level defaults, identity replacement, sticky removal and mode capability explicitly, then project the supported fields through RPC. Keep notification state distinct from session persistence and avoid implementing notifications through fabricated agent messages.

### 9. Size and priority

Estimated Rust implementation: **80-150 lines** (estimate, not measured source). **P2** - Extension warnings and status notifications have no equivalent native host API or consistent identity semantics.

## EXT-042 - Status text, working indicator and hidden-thinking controls

### 1. Title and status

Status text, working indicator and hidden-thinking controls. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/extensions/types.ts:139-146; ice:packages/coding-agent/src/core/extensions/types.ts:170-190; ice:packages/coding-agent/src/modes/interactive/interactive-mode.ts:2868-2953; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:400-605; ice:packages/coding-agent/src/core/extensions/runner.ts:255-286.

### 3. What it does

Allow plugins to publish footer status and configure the shared working row and hidden-thinking label.

### 4. How it works

setStatus stores keyed footer text, while the working and hidden-thinking controls modify shared interactive presentation. These calls return immediately and affect UI state rather than the session's messages or model behavior.

setStatus(key, text) delegates to footer data, clearing the key when text is undefined. setWorkingMessage stores a custom message or resets it when omitted. setWorkingVisible(false) clears the built-in working row; making it visible while streaming recreates an indicator as needed. Indicator options select frames and an optional interval; one frame produces a static display and an empty frame array hides the indicator. The hidden-thinking label is retained and applied to existing relevant components, not just future messages. Shared controls are global to the current UI context, so a later plugin call replaces earlier presentation rather than owning an independent spinner.

Interactive reset restores working visibility/default message, indicator and hidden-thinking label along with extension UI state. RPC forwards corresponding UI requests and cannot itself guarantee the remote client renders them. No-UI methods are no-ops. There is no independent persistence, reply, cancellation or owner token in these public setters; operation errors follow their caller's handling. Rendering cadence and spinner implementation belong to the TUI subsystem rather than the extension runner.

### 5. Interfaces

```ts
export interface WorkingIndicatorOptions {
	/** Animation frames. Use an empty array to hide the indicator entirely. Custom frames are rendered verbatim. */
	frames?: string[];
	/** Frame interval in milliseconds for animated indicators. */
	intervalMs?: number;
}

/** Wrap the current autocomplete provider with additional behavior. */
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:139-146.
```ts
	/** Set status text in the footer/status bar. Pass undefined to clear. */
	setStatus(key: string, text: string | undefined): void;

	/** Set the working/loading message shown during streaming. Call with no argument to restore default. */
	setWorkingMessage(message?: string): void;

	/** Show or hide the built-in interactive working loader row during streaming. */
	setWorkingVisible(visible: boolean): void;

	/**
	 * Configure the interactive working indicator shown during streaming.
	 *
	 * - Omit the argument to restore the default animated spinner.
	 * - Use `frames: ["●"]` for a static indicator.
	 * - Use `frames: []` to hide the indicator entirely.
	 * - Custom frames are rendered as provided, so extensions must add their own colors.
	 */
	setWorkingIndicator(options?: WorkingIndicatorOptions): void;

	/** Set the label shown for hidden thinking blocks. Call with no argument to restore default. */
	setHiddenThinkingLabel(label?: string): void;
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:170-190.

### 6. Dependencies

`EXT-009`, `EXT-010`

External services/binaries: none. Footer data, spinner scheduling and thinking components belong to the TUI audit.

Final dependency classification. Shared contract references: `TUI-037`, `TUI-038`, `TUI-039`, `TUI-041`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

rust:crates/ice-plugin-sdk/src/lib.rs:1042-1052; rust:crates/ice-extensions/src/status.rs:1-416; rust:crates/ice-extensions/src/actions.rs:527-793. Native SetStatus implements a real shared BTreeMap keyed by trimmed strings; blank keys are ignored, whitespace-only values clear entries, and changes increment a revision. Rendering joins values in sorted key order. Keys are not automatically namespaced by plugin owner, strings are not capped, and runtime invalidation removes owner panels but does not clear this legacy status map. UiComponent has owner-scoped bounded text panels (EXT-043), but no action configures the built-in working indicator or hidden-thinking label. These additional controls are missing even though footer status itself exists.

### 8. Port notes

Retain SetStatus compatibility while documenting native trim/clear/order differences and adding owner cleanup or explicit legacy ownership. Add distinct working/hidden-thinking operations to the UI bridge with defined last-writer behavior, defaults and reset on runtime replacement. Reuse ice-tui's spinner and thinking components, validate frame count/size and interval at the boundary, and expose mode capability without creating persistent session records.

### 9. Size and priority

Estimated Rust implementation: **110-210 lines** (estimate, not measured source). **P2** - Status partly works, but stale ownership and missing shared working controls prevent predictable extension UI cleanup.

## EXT-043 - Keyed extension widgets and headless-safe RPC projections

### 1. Title and status

Keyed extension widgets and headless-safe RPC projections. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/extensions/types.ts:119-133; ice:packages/coding-agent/src/core/extensions/types.ts:192-198; ice:packages/coding-agent/src/modes/interactive/interactive-mode.ts:2955-3077; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:400-605. Tests: ice:packages/coding-agent/test/suite/rpc-contract.test.ts:665-689.

### 3. What it does

Render extension widgets above or below the editor, with a text projection for RPC clients when the widget uses a local component factory.

### 4. How it works

A widget is stored under a plugin-supplied key in one of two editor-adjacent containers. Replacing or removing a widget removes any existing widget with that key from both placements and disposes its previous component before installing the replacement.

String-array content is rendered with a ten-line limit per widget and a truncation notice when more lines are supplied. Component factories receive the live TUI and theme and are not subject to that array-line limit. Placement defaults to aboveEditor and accepts belowEditor; replacing a widget changes insertion order in its container. Undefined content clears it. Keys are shared across extensions, so two plugins using the same key can replace each other. Factory and dispose exceptions are not independently swallowed in this setter, and bulk reset can stop early if a disposal throws.

The RPC branch sends text arrays directly. For a factory it uses options.rpcText and never calls the factory; a factory without rpcText produces no widget request. This is deliberate projection rather than remote execution of a UI callback, and the RPC branch does not apply the interactive ten-line truncation. No-UI setters return without rendering. Widgets are ephemeral UI state, not stored session entries; reset removes all widget state and invokes available dispose hooks.

### 5. Interfaces

```ts
export type WidgetPlacement = "aboveEditor" | "belowEditor";

/** Options for extension widgets. */
export interface ExtensionWidgetOptions {
	/** Where the widget is rendered. Defaults to "aboveEditor". */
	placement?: WidgetPlacement;
	/** Explicit text projection for RPC when content is a TUI component factory. The factory is never executed by RPC. */
	rpcText?: string[];
}

/** Options for a custom header or footer. */
export interface ExtensionChromeOptions {
	/** Explicit text projection for RPC. The factory is never executed by RPC. */
	rpcText?: string[];
}
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:119-133.
```ts
	/** Set a widget to display above or below the editor. Accepts string array or component factory. */
	setWidget(key: string, content: string[] | undefined, options?: ExtensionWidgetOptions): void;
	setWidget(
		key: string,
		content: ((tui: TUI, theme: Theme) => Component & { dispose?(): void }) | undefined,
		options?: ExtensionWidgetOptions,
	): void;
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:192-198.

### 6. Dependencies

`EXT-009`, `EXT-010`

External services/binaries: none. Depends on editor container layout, Component disposal and RPC UI method projection.

Final dependency classification. Consuming integrations: `RPC-028`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

rust:crates/ice-extensions/src/status.rs:1-416; rust:crates/ice-extensions/src/panel.rs:1-117; rust:crates/ice-plugin-sdk/src/lib.rs:1060-1061. Native UiComponent is an existing owner-scoped declarative text component system with register/configure/update/show/hide/remove. IDs are limited to 128 bytes and there are at most 32 components per owner. Supported slots are footer.after, sidebar.left, sidebar.right and overlay, not aboveEditor/belowEditor. Version-1 panels validate at most 64 lines, 2048 bytes per line, title at most 256 bytes and at most 32 highlights, along with geometry bounds. Unknown fields/types are rejected and repeated registration is idempotent. This gives useful native widget functionality but differs in placement, ownership, limits, disposal and projection; it is not an absent widget subsystem.

### 8. Port notes

Extend the existing UiComponent placement enum with editor-adjacent slots if parity requires them, and add a compatibility adapter for keyed string arrays. Preserve native owner scoping and bounds instead of weakening them to match cross-plugin key collisions. Decide how a portable component describes updates/disposal, reuse declarative text for RPC, and document the ten-line compatibility projection separately from native panel bounds.

### 9. Size and priority

Estimated Rust implementation: **130-240 lines** (estimate, not measured source). **P2** - Native panels exist but cannot reproduce editor-adjacent widgets or their mode-specific projection.

## EXT-044 - Custom footer, header and terminal title

### 1. Title and status

Custom footer, header and terminal title. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/extensions/types.ts:126-133; ice:packages/coding-agent/src/core/extensions/types.ts:200-220; ice:packages/coding-agent/src/modes/interactive/interactive-mode.ts:3079-3145; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:400-605; ice:packages/coding-agent/src/core/extensions/runner.ts:255-286.

### 3. What it does

Let plugins replace interactive header/footer components, inspect read-only footer data and set the terminal title.

### 4. How it works

Header and footer setters replace shared UI components rather than registering additional independent rows. Passing undefined restores the built-in component, and RPC can receive an explicit text projection without executing a component factory.

Replacing a footer disposes its previous custom component and invokes the new factory with TUI, theme and a ReadonlyFooterDataProvider for data such as branch/status information. Header replacement likewise disposes the prior custom component and preserves the header's position in the root layout; it returns early when the built-in header was never installed. Factories execute synchronously, with no separate exception-isolation or async return contract. The UI tracks expanded-header state while replacing it. setTitle writes the terminal title through the UI rather than changing session display name.

The RPC implementation sends setFooter/setHeader with rpcText when present, never invokes either factory and clears the projected content when no projection is provided. Title is a separate outgoing UI method. Headless setters do nothing. Reset restores built-in chrome and title. These calls hold no independent persisted configuration, owner key or revocation token, so multiple extensions can replace the same shared component and must release resources through dispose.

### 5. Interfaces

```ts
	rpcText?: string[];
}

/** Options for a custom header or footer. */
export interface ExtensionChromeOptions {
	/** Explicit text projection for RPC. The factory is never executed by RPC. */
	rpcText?: string[];
}
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:126-133.
```ts
	/** Set a custom footer component, or undefined to restore the built-in footer.
	 *
	 * The factory receives a FooterDataProvider for data not otherwise accessible:
	 * git branch and extension statuses from setStatus(). Token stats, model info,
	 * etc. are available via ctx.sessionManager and ctx.model.
	 */
	setFooter(
		factory:
			| ((tui: TUI, theme: Theme, footerData: ReadonlyFooterDataProvider) => Component & { dispose?(): void })
			| undefined,
		options?: ExtensionChromeOptions,
	): void;

	/** Set a custom header component (shown at startup, above chat), or undefined to restore the built-in header. */
	setHeader(
		factory: ((tui: TUI, theme: Theme) => Component & { dispose?(): void }) | undefined,
		options?: ExtensionChromeOptions,
	): void;

	/** Set the terminal window/tab title. */
	setTitle(title: string): void;
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:200-220.

### 6. Dependencies

`EXT-009`, `EXT-010`, `EXT-042`

External services/binaries: none. The TUI owns footer data and terminal title escapes; RPC owns transport capability.

Final dependency classification. Shared contract references: `TUI-037`, `TUI-038`, `TUI-039`, `TUI-040`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

rust:crates/ice-extensions/src/status.rs:1-416; rust:crates/ice-extensions/src/panel.rs:1-117; rust:crates/ice-plugin-sdk/src/lib.rs:1010-1084. Native SetStatus and UiComponent footer.after/sidebar/overlay slots allow bounded additional text presentation, but do not replace the main footer or header or provide a read-only FooterDataProvider callback. The complete runtime action enum exposes no terminal-title operation and no factory-bearing chrome operation. Native owner-scoped component cleanup is stronger than unkeyed replacement, but it does not implement these distinct controls.

### 8. Port notes

Add explicit optional chrome and title capabilities to the current UI bridge. Prefer declarative header/footer models with owner arbitration over transferring foreign Component pointers; expose safe footer data snapshots without leaking mutable host state. Reuse existing panel lifetime management and render defaults when the owner unloads, then project text representations to RPC clients that advertise support.

### 9. Size and priority

Estimated Rust implementation: **120-230 lines** (estimate, not measured source). **P2** - Plugins relying on custom chrome or terminal title currently have no matching native control.

## EXT-045 - Focused custom UI components, overlays and completion lifetime

### 1. Title and status

Focused custom UI components, overlays and completion lifetime. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/extensions/types.ts:222-237; ice:packages/coding-agent/src/modes/interactive/interactive-mode.ts:3535-3615; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:400-605; ice:packages/coding-agent/src/core/extensions/runner.ts:255-286. Tests: ice:packages/coding-agent/test/interactive-mode-status.test.ts:417-488.

### 3. What it does

Temporarily give a custom component keyboard focus and return its result when the component calls done.

### 4. How it works

custom creates a focused UI interaction whose factory receives TUI, theme, keybindings and a completion callback. Interactive mode either replaces the editor area or displays an overlay, then restores the prior editor state when the interaction finishes.

The host saves current editor text, accepts a synchronous or promised component and installs it when ready. done is idempotent: it closes/restores the relevant UI, requests redraw, disposes the active custom component through a guarded cleanup and resolves the Promise with the supplied result. Overlay options may be a function, but this invocation resolves them when the overlay is shown rather than continuously reevaluating them on every layout change. onHandle exposes the resulting overlay handle after installation. The nonoverlay branch preserves editor text/focus and restores the normal editor container on completion.

Factory rejection follows the host error path. An asynchronous factory that resolves after done has already settled can produce a late component without the normal installed-component disposal path; this is a lifetime edge worth testing rather than assuming every returned object is released. There is no AbortSignal/timeout option or generic factory cancellation. RPC and no-UI custom return undefined without invoking the factory, despite the generic Promise<T> signature. No custom component state is persisted, and this API is broader than the typed dialog helpers in EXT-040.

### 5. Interfaces

```ts
	/** Show a custom component with keyboard focus. */
	custom<T>(
		factory: (
			tui: TUI,
			theme: Theme,
			keybindings: KeybindingsManager,
			done: (result: T) => void,
		) => (Component & { dispose?(): void }) | Promise<Component & { dispose?(): void }>,
		options?: {
			overlay?: boolean;
			/** Overlay positioning/sizing options. Can be static or a function for dynamic updates. */
			overlayOptions?: OverlayOptions | (() => OverlayOptions);
			/** Called with the overlay handle after the overlay is shown. Use to control visibility. */
			onHandle?: (handle: OverlayHandle) => void;
		},
	): Promise<T>;
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:222-237.

### 6. Dependencies

`EXT-009`, `EXT-010`, `EXT-040`, `EXT-047`

External services/binaries: none. TUI focus, overlay placement/handles and component disposal are cross-area dependencies.

Final dependency classification. Shared contract references: `TUI-073`, `TUI-217`, `TUI-218`, `TUI-219`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

rust:crates/ice-extensions/src/status.rs:1-416; rust:crates/ice-extensions/src/panel.rs:1-117; rust:crates/ice-cli/src/interactive_tui/extensions.rs:77-495; rust:crates/ice-cli/src/interactive_tui/extensions.rs:529-939. Native UiComponent overlay panels are bounded declarative text, while existing selector/editor/input dialogs have structured focus and result handling. Neither accepts an arbitrary component factory, exposes a completion callback returning generic T, or hands a live overlay handle to a plugin. The existing pieces provide substantial UI functionality, but plugins cannot express this full custom interaction contract through them.

### 8. Port notes

Build a versioned declarative custom-view protocol on current native dialogs/panels, with request IDs, focus ownership, typed results and explicit owner-cancellation. Define late-result cleanup and restore previous editor state exactly once. Extend ice-tui only for capabilities that cannot be expressed by existing dialogs; avoid passing host object pointers or unrestricted native view ownership across the ABI.

### 9. Size and priority

Estimated Rust implementation: **220-420 lines** (estimate, not measured source). **P2** - Advanced extension interactions require a focused result-bearing view, which native text overlays do not provide.

## EXT-046 - Raw terminal input interception and data transformation

### 1. Title and status

Raw terminal input interception and data transformation. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/extensions/types.ts:135-137; ice:packages/coding-agent/src/core/extensions/types.ts:167-168; ice:packages/coding-agent/src/modes/interactive/interactive-mode.ts:3009-3077; ice:packages/coding-agent/src/modes/interactive/interactive-mode.ts:3147-3161; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:400-605.

### 3. What it does

Allow interactive extensions to inspect raw terminal input, consume it or replace the bytes passed to later input processing.

### 4. How it works

onTerminalInput installs an interactive raw-input listener and returns an unsubscribe function. It is distinct from a named shortcut and from the prompt input event: it runs at the terminal input layer before higher-level editor behavior is decided.

The handler receives a string and may return {consume?, data?} or undefined. The UI registration delegates to the TUI listener mechanism and records the unsubscribe handle in the extension UI's cleanup list. Calling the returned handle removes the listener; resetting extension UI invokes tracked removals so listeners do not accumulate across rebinds. Exact terminal listener sequencing and data/consume reduction belong to the TUI implementation, not the ExtensionRunner's sequential event loops.

RPC and no-UI contexts return a harmless no-op unsubscribe function and do not invoke the listener. There is no persistence, timeout, async-handler contract, owner priority or validation wrapper in this extension adapter. A raw listener may affect escape sequences and paste handling, so it cannot be faithfully substituted with an already-decoded key notification. Shortcut conflict/reserved-key behavior remains EXT-014.

### 5. Interfaces

```ts
/** Raw terminal input listener for extensions. */
export type TerminalInputHandler = (data: string) => { consume?: boolean; data?: string } | undefined;

```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:135-137.
```ts
	/** Listen to raw terminal input (interactive mode only). Returns an unsubscribe function. */
	onTerminalInput(handler: TerminalInputHandler): () => void;
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:167-168.

### 6. Dependencies

`EXT-009`, `EXT-010`, `EXT-014`

External services/binaries: none. Requires the TUI input-listener chain and terminal event decoding contracts.

Final dependency classification. Shared contract references: `TUI-206`, `TUI-208`, `TUI-209`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

rust:crates/ice-cli/src/interactive_tui.rs:390-505; rust:crates/ice-plugin-sdk/src/lib.rs:590-645; rust:crates/ice-extensions/src/translate.rs:342-450. Native keyboard routing emits a decoded Input event containing key/modifier/kind information and supports claiming it under editor/modal gating. It does not expose original terminal data or a return value that rewrites that data. A separate raw-terminal subscription/unsubscribe ABI is absent from the complete SDK. Native Input dispatch fans out through subscribers, which is also not the TypeScript TUI raw listener contract.

### 8. Port notes

If raw parity is required, add a scoped terminal-input capability before key decoding, with owned input/output byte strings and a bounded synchronous decision. Keep it separate from EventTag::Input and named shortcuts. Track subscriptions by runtime owner, remove them on invalidation, document listener ordering and limit replacement data to avoid unbounded expansion. Reuse ice-tui's input pipeline rather than adding another terminal reader.

### 9. Size and priority

Estimated Rust implementation: **110-210 lines** (estimate, not measured source). **P2** - Modal editors and terminal integrations need interception before decoded key events.

## EXT-047 - Editor text, bracketed paste and custom editor replacement

### 1. Title and status

Editor text, bracketed paste and custom editor replacement. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/extensions/types.ts:239-246; ice:packages/coding-agent/src/core/extensions/types.ts:254-290; ice:packages/coding-agent/src/modes/interactive/interactive-mode.ts:3188-3243; ice:packages/coding-agent/src/modes/interactive/interactive-mode.ts:3424-3495; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:400-605.

### 3. What it does

Read or replace the current prompt text, simulate paste behavior, and optionally install a custom editor while retaining host actions and autocomplete.

### 4. How it works

Editor text setters and paste are different operations: setEditorText directly sets content, while pasteToEditor routes a bracketed-paste sequence through the editor's input handler. Interactive getEditorText returns expanded text, including content represented by collapsed paste markers.

setEditorComponent accepts a factory or undefined to restore the default editor; getEditorComponent returns the configured factory rather than the component instance. The replacement keeps the existing editor text, wires the host submission/change callbacks, reapplies autocomplete and related editor options, and makes the new component the active editor. A component exposing the expected actionHandlers Map receives missing host action handlers; extending CustomEditor is the documented route for complete application keybindings. Existing custom action handlers are not blindly overwritten. The setter does not explicitly dispose the old custom editor.

RPC set/paste project to outgoing set_editor_text behavior, while getEditorText returns an empty string, editor factory set/get are no-ops/undefined and no local factory executes. No-UI setters likewise do nothing. These API calls do not themselves submit a user message or append a session entry. Restoring editor state and avoiding retained callbacks after runtime replacement depend on the UI reset path, not on the text strings.

### 5. Interfaces

```ts
	/** Paste text into the editor, triggering paste handling (collapse for large content). */
	pasteToEditor(text: string): void;

	/** Set the text in the core input editor. */
	setEditorText(text: string): void;

	/** Get the current text from the core input editor. */
	getEditorText(): string;
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:239-246.
```ts
	setEditorComponent(factory: EditorFactory | undefined): void;

	/** Get the currently configured custom editor factory, or undefined when using the default editor. */
	getEditorComponent(): EditorFactory | undefined;
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:287-290.
The interactive paste path wraps text with bracketed-paste input markers in ice:packages/coding-agent/src/modes/interactive/interactive-mode.ts:3188-3243. It is not equivalent to automatic prompt submission.

### 6. Dependencies

`EXT-009`, `EXT-010`, `EXT-046`

External services/binaries: none. The base editor, expanded paste storage and application keybinding hooks belong to TUI.

Final dependency classification. Shared contract references: `TUI-049`, `TUI-242`, `TUI-243`, `TUI-244`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

rust:crates/ice-plugin-sdk/src/lib.rs:1047-1052; rust:crates/ice-extensions/src/editor_text.rs:1-184; rust:crates/ice-extensions/src/actions.rs:984-1265. Native SetEditorText enqueues a real editor mutation. Its text field is required; mode defaults to replace and accepts the documented aliases/append branch, and optional positive autoSendMs schedules submission. The queue is unbounded, headless hosts can leave it undrained, and the receipt reports ok/chars before UI application; a poisoned queue lock may drop the push. It exposes neither expanded-text retrieval, a bracketed-paste operation nor custom editor factory get/set. Native auto-send is an additional side effect that TypeScript setEditorText does not request.

### 8. Port notes

Keep the native queued mutation path and add explicit replace/paste/read operations with acknowledgement semantics. Make auto-send opt-in and separate from compatibility setters; define bounded pending work and failure reporting when the UI is unavailable. Expose a declarative editor capability or controlled keybinding hooks for customization, retain text/callback state on replacement, and give installed editors an explicit dispose/revoke lifecycle.

### 9. Size and priority

Estimated Rust implementation: **180-320 lines** (estimate, not measured source). **P1** - Text mutation is only partly compatible, and native queued writes/auto-send differ materially from synchronous editor control.

## EXT-048 - Stacked autocomplete provider wrappers

### 1. Title and status

Stacked autocomplete provider wrappers. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/core/extensions/types.ts:251-252; ice:packages/coding-agent/src/modes/interactive/interactive-mode.ts:880-940; ice:packages/coding-agent/src/modes/interactive/interactive-mode.ts:3009-3077; ice:packages/coding-agent/src/modes/interactive/interactive-mode.ts:3424-3495; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:400-605. Tests: ice:packages/coding-agent/test/interactive-mode-status.test.ts:490-587.

### 3. What it does

Allow plugins to layer completion behavior around the host's current autocomplete provider and retain it when the editor changes.

### 4. How it works

addAutocompleteProvider records a provider factory and rebuilds the autocomplete chain. Each factory receives the provider produced by the previous layer, allowing it to delegate ordinary completion and specialize additional triggers.

Rebuilding begins with a fresh built-in provider, then applies wrapper factories in registration order; later wrappers sit outside earlier wrappers. The host collects trigger characters from each resulting provider and deduplicates them. It assigns the resulting provider to the default editor and, when supported, the custom editor. Subsequent editor replacement reapplies the chain, while extension UI reset clears the wrappers and restores ordinary behavior. Tests cover ordering, delegation, preserving host behavior and custom-editor wiring.

Factories execute synchronously; this adapter has no independent exception catch, timeout or unsubscribe handle for an individual wrapper. Completion itself follows the provider contract, including its own result types and cancellation behavior. RPC/no-UI implementations ignore the registration and never execute the factory. Wrappers are in-memory and cannot directly redefine slash-command registration precedence, which remains EXT-013.

### 5. Interfaces

```ts
	/** Stack additional autocomplete behavior on top of the built-in provider. */
	addAutocompleteProvider(factory: AutocompleteProviderFactory): void;
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:251-252.
The exact public parameter is AutocompleteProviderFactory; provider composition and trigger collection are implemented at ice:packages/coding-agent/src/modes/interactive/interactive-mode.ts:880-940. RegisteredCommand.getArgumentCompletions is a separate completion surface (ice:packages/coding-agent/src/core/extensions/types.ts:1224-1229).

### 6. Dependencies

`EXT-009`, `EXT-010`, `EXT-013`, `EXT-047`

External services/binaries: none. Resolve the concrete AutocompleteProviderFactory/provider result contract to the TUI autocomplete entry during publication.

Final dependency classification. Shared contract references: `TUI-238`, `TUI-239`, `TUI-240`, `TUI-241`, `TUI-255`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

rust:crates/ice-plugin-sdk/src/lib.rs:348-353; rust:crates/ice-plugin-sdk/src/lib.rs:1010-1084; rust:crates/ice-extensions/src/registry.rs:1-1330; rust:crates/ice-cli/src/interactive_tui/extensions.rs:34-75. Complete SDK/registry inspection finds registration for named commands and shortcuts, but no provider-wrapper registration, trigger-character contribution or host autocomplete callback. Native command invocation sends args/command JSON and is not an autocomplete composition API. Existing built-in TUI completion must therefore not be counted as extension wrapper parity.

### 8. Port notes

Add a versioned completion provider capability to ice-plugin-sdk and a runtime-owned chain in ice-extensions, with prefix/cursor/context request DTOs and bounded completion results. Reuse ice-tui's existing completion engine, preserve built-in fallback, define priority and cancellation, and remove owner registrations on runtime invalidation. Offer declarative trigger metadata rather than transferring provider objects over the ABI.

### 9. Size and priority

Estimated Rust implementation: **160-280 lines** (estimate, not measured source). **P2** - Plugins cannot contribute editor completions while retaining the host's normal completion behavior.

## EXT-049 - Theme inspection, loading and UI theme changes

### 1. Title and status

Theme inspection, loading and UI theme changes. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/core/extensions/types.ts:292-302; ice:packages/coding-agent/src/core/extensions/runner.ts:278-283; ice:packages/coding-agent/src/modes/interactive/interactive-mode.ts:3220-3236; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:400-605; ice:packages/coding-agent/src/core/extensions/types.ts:598-609. Tests: ice:packages/coding-agent/test/interactive-mode-status.test.ts:313-372.

### 3. What it does

Let extensions inspect the live theme, enumerate available themes, load a named theme or apply a theme in interactive mode.

### 4. How it works

The UI context exposes a live theme getter and separate enumeration/load/set operations. Loading a theme by name does not switch the UI; successful interactive name selection may update the stored theme setting, while applying a Theme instance only changes the active controller state.

getAllThemes returns {name, path} records, where path can be undefined for built-in themes. getTheme(name) delegates to named-theme lookup and returns undefined when not found. setTheme accepts a string or a Theme instance and returns {success, error?}. The interactive implementation dispatches instances to setThemeInstance; strings go through setThemeName and, only after success and only if changed, settingsManager.setTheme. The public adapter does not clone Theme objects or swallow controller errors. Theme resource parsing, file format and resource precedence belong to the theme/resource subsystems rather than this extension method.

RPC retains actual theme inspection/loading methods but returns a failed result for theme mutation. The no-UI context exposes the current global theme object, returns [] from enumeration, undefined from named loading and {success:false, error:"UI not available"} from setTheme. Therefore hasUI/mode must be considered instead of assuming all getters or setters share the same capability. Theme discovery arrays from resources_discover are a separate extension contribution in EXT-031.

### 5. Interfaces

```ts
	/** Get the current theme for styling. */
	readonly theme: Theme;

	/** Get all available themes with their names and file paths. */
	getAllThemes(): { name: string; path: string | undefined }[];

	/** Load a theme by name without switching to it. Returns undefined if not found. */
	getTheme(name: string): Theme | undefined;

	/** Set the current theme by name or Theme object. */
	setTheme(theme: string | Theme): { success: boolean; error?: string };
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:292-302.
The adapter persists the existing theme setting only after a successful named interactive switch (ice:packages/coding-agent/src/modes/interactive/interactive-mode.ts:3225-3235); no new setting key or theme-file format is introduced here.

### 6. Dependencies

`EXT-009`, `EXT-031`, `SET-017`, `SET-033`, `TUI-001`, `TUI-003`

External services/binaries: none. Resolve settings persistence and the theme catalog/parser to settings and TUI/resource entries.

### 7. ice-rust today

rust:crates/ice-plugin-sdk/src/lib.rs:1010-1084; rust:crates/ice-extensions/src/resources.rs:1-317; rust:crates/ice-cli/src/session.rs:499-539; rust:crates/ice-cli/src/session.rs:1395-1443. Native resources_discover accepts theme paths, but the inspected startup/reload caller consumes only skill and prompt arrays; theme paths do not enter these resource loaders. The complete runtime action list contains no theme enumeration/load/set action or Theme object handle. Existing native UI themes are not exposed to extensions through this contract. This missing API is separate from the caller gap that discards discovered themes, owned jointly with EXT-031 and the session audit.

### 8. Port notes

Expose safe theme names, provenance and style tokens through a versioned UI query action, plus explicit set-by-name and optional ephemeral theme DTO operations. Use the existing ice-tui theme system and settings writer, wire discovered paths into the actual catalog with existing trust policy, and return an explicit unsupported result in headless/RPC modes. Avoid sharing mutable host theme pointers with native plugins.

### 9. Size and priority

Estimated Rust implementation: **120-220 lines** (estimate, not measured source). **P2** - Plugins cannot query or select native themes, and discovered extension themes are not consumed by startup/reload.

## EXT-050 - Tool-output expansion controls and batch-state semantics

### 1. Title and status

Tool-output expansion controls and batch-state semantics. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/core/extensions/types.ts:304-308; ice:packages/coding-agent/src/modes/interactive/interactive-mode.ts:3237-3238; ice:packages/coding-agent/src/modes/interactive/interactive-mode.ts:5578-5653; ice:packages/coding-agent/src/modes/rpc/rpc-mode.ts:400-605; ice:packages/coding-agent/src/core/extensions/runner.ts:284-285. Tests: ice:packages/coding-agent/test/interactive-mode-status.test.ts:216-311.

### 3. What it does

Allow an extension to expand or collapse the transcript's tool output and query the host's current expansion flag.

### 4. How it works

setToolsExpanded chooses a full expand/collapse state for the transcript and requests a redraw. getToolsExpanded returns the host's batch expansion flag, which is not a count or a guarantee that every individual tool row is expanded.

The interactive setter chooses the "all" anchor for true and no anchor for false, clears per-row expansion overrides and shows the corresponding status message. Changing the shared expansion flag also updates expandable header and non-tool transcript/resource components. Each tool row is then assigned its effective expansion from any override or its position relative to the batch anchor. Interactive shortcuts can expand recent batches or toggle only the newest row; any nonundefined batch anchor makes the getter true, so a partially expanded transcript can return true. Individual override state is not represented by this boolean API.

RPC/no-UI getters always return false and setters do nothing. Expansion is ephemeral UI state, has no async acknowledgement or cancellation and does not change stored tool results or model messages. Custom renderers receive expansion through their own options/context, and explicit setToolsExpanded invalidates the host row rendering as appropriate.

### 5. Interfaces

```ts
	/** Get current tool output expansion state. */
	getToolsExpanded(): boolean;

	/** Set tool output expansion state. */
	setToolsExpanded(expanded: boolean): void;
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:304-308.
The extension API does not register its own keybinding; built-in batch and newest-tool toggles are separate TUI actions.

### 6. Dependencies

`EXT-036`, `EXT-037`, `EXT-038`

External services/binaries: none. Requires the TUI tool-cell identity, batch anchor and expansion rendering contracts.

Final dependency classification. Shared contract references: `TUI-031`, `TUI-034`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

rust:crates/ice-plugin-sdk/src/lib.rs:1010-1084; rust:crates/ice-cli/src/interactive_tui/sessions.rs:425-610. Native plugins cannot query/set transcript expansion through any runtime action. Native custom renderer requests also omit the TypeScript expansion field, so an extension cannot infer and reproduce it via the renderer protocol. Built-in TUI expansion, if used by the user, is not an extension host API and should be inventoried by the TUI area separately.

### 8. Port notes

Add explicit Get/SetToolExpansion operations to the existing UI bridge and define whether the query returns the compatibility batch flag or a richer state. Reuse ice-tui's expansion state rather than keeping a plugin-side copy. Clear individual overrides on an explicit all/none command, trigger renderer invalidation, and return a capability result for modes without a transcript.

### 9. Size and priority

Estimated Rust implementation: **60-120 lines** (estimate, not measured source). **P2** - Extensions cannot coordinate expanded tool views with their custom presentation.

## EXT-051 - Custom message delivery, next-turn deferral and background errors

### 1. Title and status

Custom message delivery, next-turn deferral and background errors. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/extensions/types.ts:1372-1376; ice:packages/coding-agent/src/core/extensions/loader.ts:383-394; ice:packages/coding-agent/src/core/agent-session.ts:1732-1768; ice:packages/coding-agent/src/core/agent-session.ts:2931-2941; ice:packages/coding-agent/src/core/agent-session.ts:4000-4030. sendMessage, sendCustomMessage and replacement-context async actions.

Exported symbols: `SendMessageHandler` (ice:packages/coding-agent/src/core/extensions/types.ts:1630).

### 3. What it does

Let an extension append a custom message, steer/follow up an active agent, trigger an idle turn or defer the message until the next turn.

### 4. How it works

sendMessage uses one typed custom-message shape with explicit triggerTurn and deliverAs options. The global extension API returns void and reports later asynchronous delivery failures through the runner error channel, while a replacement session context offers an awaited form.

The session creates role:"custom", copies customType/display/details, normalizes nullish content to [] in the in-memory message and timestamps it. deliverAs:"nextTurn" has first precedence and pushes into the next-turn queue even when otherwise idle. During streaming, triggerTurn:false or the absence of both delivery/trigger intent queues a pending custom message; followUp selects the follow-up lane, and remaining admitted delivery selects steer. When idle, triggerTurn:true runs the prompt path; otherwise it appends to agent state and the custom-message session log, then emits message_start/message_end. The idle append passes original message.content to the session writer, so ingestion normalization should not be assumed to rewrite every durable representation.

The runtime-bound void wrapper attaches a rejection handler that emits extensionPath:"<runtime>", event:"send_message" and the error text. It does not make the caller await delivery. Synchronous stale-context checks occur before dispatch, but queued/delayed work and runtime replacement need the surrounding lifecycle contract. display controls transcript visibility rather than whether a custom message is model context. None of these paths invokes an arbitrary shell or changes the customType registration.

### 5. Interfaces

```ts
	/** Send a custom message to the session. */
	sendMessage<T = unknown>(
		message: Pick<CustomMessage<T>, "customType" | "content" | "display" | "details">,
		options?: { triggerTurn?: boolean; deliverAs?: "steer" | "followUp" | "nextTurn" },
	): void;
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:1372-1376.
The result of the global method is void; the replaced-session context's async send variant is covered by EXT-020.

### 6. Dependencies

`AGENT-005`, `AGENT-018`, `AGENT-019`, `AGENT-022`, `AGENT-023`, `EXT-009`, `EXT-010`, `EXT-020`, `SESSION-020`

External services/binaries: the selected model/provider only when a turn is triggered. Agent scheduling and session message persistence are cross-area dependencies.

### 7. ice-rust today

rust:crates/ice-plugin-sdk/src/lib.rs:1011-1020; rust:crates/ice-cli/src/extensions_actions.rs:190-387; rust:crates/ice-cli/src/extensions_actions.rs:430-439. Native SendMessage is implemented, parses stored/custom message inputs and flat or nested delivery options, and delegates to the main harness lane with a structured result. This is substantial parity, not a stub. The SDK documents idle append, active steer and nextTurn deferral; TypeScript active delivery without options instead takes its pending-custom-message branch. Native errors are returned through the action result rather than a void method's <runtime>/send_message event. Exact harness queue persistence/turn sequencing is owned by the agent/session audit; the extension adapter must translate, not duplicate, it.

### 8. Port notes

Add a TypeScript-compatibility message facade over the current native SendMessage action, explicitly translating the active default/pending case, nextTurn precedence and void-versus-awaited error reporting. Retain typed serde input validation and the existing main-lane scheduler. Use runtime-owned queued requests so replacement can settle or reject them predictably, and keep custom data/display/LLM-context semantics separate.

### 9. Size and priority

Estimated Rust implementation: **100-190 lines** (estimate, not measured source). **P1** - Default active-run delivery and error observation differ even though native message sending already works.

## EXT-052 - Extension user-message text/image normalization and source routing

### 1. Title and status

Extension user-message text/image normalization and source routing. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/extensions/types.ts:1378-1385; ice:packages/coding-agent/src/core/extensions/loader.ts:388-394; ice:packages/coding-agent/src/core/agent-session.ts:1777-1808; ice:packages/coding-agent/src/core/agent-session.ts:2942-2950; ice:packages/coding-agent/src/core/extensions/types.ts:885-901.

Exported symbols: `SendUserMessageHandler` (ice:packages/coding-agent/src/core/extensions/types.ts:1635).

### 3. What it does

Let an extension submit user text and images through the host's prompt admission path, including a selected streaming delivery mode.

### 4. How it works

sendUserMessage accepts a string or a content array and converts it into text plus optional images for the ordinary session prompt path. That path is called with source:"extension", allowing input hooks and host admission to distinguish extension-originated user input.

A string is forwarded unchanged. For an array, text parts are collected and joined with newline; non-text parts are placed into the image list without additional validation at this normalization step. An empty image list becomes undefined. The adapter passes expandPromptTemplates:false, streamingBehavior from deliverAs and the normalized images to prompt. The source comment says this skips command handling and templates, but only the concrete option is established here; command admission behavior must follow the actual prompt implementation rather than that comment alone.

The global ExtensionAPI method is void. Its bound wrapper catches asynchronous prompt failure and emits <runtime>/send_user_message; replaced-session actions can be awaited through EXT-020. The public delivery enum is steer|followUp, with no nextTurn option. Input transformation uses the precise event fields in EXT-032 and does not invent a requestId. The caller has not asked for a custom message record, so this path must retain user-role and image handling through the shared session/agent pipeline.

### 5. Interfaces

```ts
	/**
	 * Send a user message to the agent. Always triggers a turn.
	 * When the agent is streaming, use deliverAs to specify how to queue the message.
	 */
	sendUserMessage(
		content: string | (TextContent | ImageContent)[],
		options?: { deliverAs?: "steer" | "followUp" },
	): void;
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:1378-1385.

### 6. Dependencies

`EXT-009`, `EXT-010`, `EXT-020`, `EXT-032`, `SESSION-019`, `SESSION-020`

External services/binaries: selected model/provider when prompt admission starts a turn. Session prompt admission and message/image storage are cross-area dependencies.

### 7. ice-rust today

rust:crates/ice-cli/src/extensions_actions.rs:441-452; rust:crates/ice-plugin-sdk/src/lib.rs:1015-1016. Native SendUserMessage requires a text string and calls lane.prompt_text(&text, Vec::new()); it does not consume a content/image array or deliverAs field from the action args. It returns an awaited run outcome, not a void API plus runner error event. Sending user input works, but extension images, streaming delivery selection and TypeScript source semantics are not supplied by this adapter. Native SendMessage is a different action and cannot be silently substituted without checking its role and queue semantics.

### 8. Port notes

Extend the existing SendUserMessage DTO with validated text/image content, source and explicit delivery selection. Route it through the same main-lane prompt admission and image validation as normal user input, preserving the extension source for hooks. Offer both awaited action results and a compatibility void/error-event facade; do not simulate images in text or bypass authorization/turn admission.

### 9. Size and priority

Estimated Rust implementation: **100-190 lines** (estimate, not measured source). **P1** - Native extension user submissions silently lack images and selected streaming delivery behavior.

## EXT-053 - Custom entry append, durable flush and session metadata actions

### 1. Title and status

Custom entry append, durable flush and session metadata actions. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/extensions/types.ts:1387-1401; ice:packages/coding-agent/src/core/extensions/loader.ts:395-409; ice:packages/coding-agent/src/core/agent-session.ts:2951-2964; ice:packages/coding-agent/src/core/agent-session.ts:3503-3511. appendEntry, setSessionName, getSessionName and setLabel.

Exported symbols: `SessionInfoChangedEvent`, `AppendEntryHandler`, `SetSessionNameHandler`, `GetSessionNameHandler`, `SetLabelHandler` (ice:packages/coding-agent/src/core/extensions/types.ts:625; ice:packages/coding-agent/src/core/extensions/types.ts:1640; ice:packages/coding-agent/src/core/extensions/types.ts:1642; ice:packages/coding-agent/src/core/extensions/types.ts:1644; ice:packages/coding-agent/src/core/extensions/types.ts:1667).

### 3. What it does

Allow extensions to persist their own state entries, request durable storage and update the session's name or entry labels.

### 4. How it works

appendEntry stores a custom state record without sending it to the model, while name and label setters delegate to the current session manager. An optional durable append brackets the write with durability flushes, making it distinct from an ordinary buffered state update.

The extension runtime's append callback calls flushDurably before appendCustomEntry and again afterward when options.durable is truthy. This is a synchronous void action: either flush or append can throw, and there is no rollback if a later flush fails after appending. The data value is plugin-defined; the adapter imposes no schema or customType normalization. Session serialization, entry IDs, parent links and filesystem location remain owned by the session store. Visibility is controlled by EXT-038's entry renderer, not by appendEntry.

setSessionName appends a session-info record, reads the effective name and emits session_info_changed through both the ordinary session event emitter and the extension runner. The extension emission is invoked without awaiting completion. getSessionName returns the manager's current value. setLabel appends a label-change record using entryId and string|undefined, where undefined is the clear operation at this API boundary. All actions use the runtime's current bound session and are subject to stale-runtime guards; retained raw session-manager references require EXT-019's separate lifetime analysis.

### 5. Interfaces

```ts
	/** Append a custom entry to the session for state persistence (not sent to LLM). */
	appendEntry<T = unknown>(customType: string, data?: T, options?: { durable?: boolean }): void;

	// =========================================================================
	// Session Metadata
	// =========================================================================

	/** Set the session display name (shown in session selector). */
	setSessionName(name: string): void;

	/** Get the current session name, if set. */
	getSessionName(): string | undefined;

	/** Set or clear a label on an entry. Labels are user-defined markers for bookmarking/navigation. */
	setLabel(entryId: string, label: string | undefined): void;
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:1387-1401.

`SessionInfoChangedEvent { type: 'session_info_changed'; name: string | undefined }` uses a required name property whose value may be undefined when cleared. The named action aliases at ice:packages/coding-agent/src/core/extensions/types.ts:1640-1644 and ice:packages/coding-agent/src/core/extensions/types.ts:1667 have the same generic append/void name-and-label signatures already quoted (ice:packages/coding-agent/src/core/extensions/types.ts:625-629; ice:packages/coding-agent/src/core/extensions/types.ts:1640-1644; ice:packages/coding-agent/src/core/extensions/types.ts:1667).

### 6. Dependencies

`EXT-009`, `EXT-010`, `EXT-022`, `SESSION-029`, `SESSION-031`, `SESSION-033`

External services/binaries: none. Resolve concrete session record formats, labels and durable-flush guarantees to session storage entries.

### 7. ice-rust today

rust:crates/ice-cli/src/extensions_actions.rs:454-514; rust:crates/ice-plugin-sdk/src/lib.rs:1017-1021; rust:crates/ice-plugin-sdk/src/lib.rs:1053-1055. Native AppendEntry implements customType/data append and additionally supports a complete AgentMessage append through the harness boundary writer; it returns entryId asynchronously. It does not inspect durable or bracket the custom append with the TypeScript flush calls. SetSessionName delegates to session.set_name and returns null. The complete action enum contains no GetSessionName or SetLabel operation. GetSessionBranch offers records, but is not an exact metadata getter or label mutation contract. Native underlying persistence may have its own guarantees; absent a durable option here does not prove every write is nondurable.

### 8. Port notes

Add explicit durability intent and metadata get/label actions to the existing native session action interface, preserving the store's own atomicity and flush semantics. Keep custom-state and AgentMessage append separate in typed DTOs so model-context behavior is unambiguous. Return clear errors for partial durability failures, preserve owner/runtime checks and route session-info events through the existing session emitter rather than maintaining a second metadata cache.

### 9. Size and priority

Estimated Rust implementation: **120-230 lines** (estimate, not measured source). **P1** - Extension state durability intent and label/name APIs are only partly represented in the native host.

## EXT-054 - Tool inventory, active-tool selection and command introspection

### 1. Title and status

Tool inventory, active-tool selection and command introspection. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/extensions/types.ts:1406-1416; ice:packages/coding-agent/src/core/extensions/types.ts:1646-1657; ice:packages/coding-agent/src/core/agent-session.ts:1118-1158; ice:packages/coding-agent/src/core/agent-session.ts:1246-1310; ice:packages/coding-agent/src/core/agent-session.ts:2905-2929; ice:packages/coding-agent/src/core/agent-session.ts:2965-2969; ice:packages/coding-agent/src/core/agent-session.ts:3036-3135.

Exported symbols: `GetActiveToolsHandler`, `GetAllToolsHandler`, `GetCommandsHandler`, `SetActiveToolsHandler` (ice:packages/coding-agent/src/core/extensions/types.ts:1646; ice:packages/coding-agent/src/core/extensions/types.ts:1653; ice:packages/coding-agent/src/core/extensions/types.ts:1655; ice:packages/coding-agent/src/core/extensions/types.ts:1657).

`SlashCommandSource` — `ice:packages/coding-agent/src/core/slash-commands.ts:4`.

### 3. What it does

Expose configured tool metadata and available extension/resource commands, and let extensions select the active tool set by name.

### 4. How it works

Tool inventory and active-tool queries are different: getAllTools describes configured definitions, while getActiveTools returns the names currently selected for the agent. setActiveTools updates that selection and rebuilds the base tool prompt through the host's existing policy and registry machinery.

ToolInfo projects name, description, parameters, promptGuidelines, optional access and sourceInfo; it does not expose every ToolDefinition field or clone every nested value. The name-selection method ignores unknown names, applies the session's bash/background-shell group handling, updates agent tools and rebuilds the base system prompt. Existing system-prompt override state remains effective. The surrounding registry refresh admits/excludes extension tools according to existing tool policy and available definitions; registration does not guarantee activation. Selection is not the same as registering a new tool or reviving a stale runtime.

getCommands combines resolved extension command invocation names, prompt templates and skill commands prefixed skill:. It preserves source and sourceInfo and does not include every built-in slash command. The return is a snapshot array; modifying it is not a command-registration API. Reads and setters are synchronous through the TypeScript runtime binding, while failures use the caller's handling. Per-turn dispatch snapshots and active-tool changes during execution are agent-layer contracts and must not be inferred solely from these getters.

### 5. Interfaces

```ts
	/** Get the list of currently active tool names. */
	getActiveTools(): string[];

	/** Get all configured tools with parameter schema, prompt guidelines, and source metadata. */
	getAllTools(): ToolInfo[];

	/** Set the active tools by name. */
	setActiveTools(toolNames: string[]): void;

	/** Get available slash commands in the current session. */
	getCommands(): SlashCommandInfo[];
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:1406-1416.
```ts
/** Tool info with name, description, parameter schema, prompt guidelines, and source metadata. */
export type ToolInfo = Pick<ToolDefinition, "name" | "description" | "parameters" | "promptGuidelines" | "access"> & {
	sourceInfo: SourceInfo;
};
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:1648-1651.

Exported supporting type (verbatim):
```ts
export type SlashCommandSource = "extension" | "prompt" | "skill";
```
Source: `ice:packages/coding-agent/src/core/slash-commands.ts:4-4`.

The getCommands() record uses the shared metadata interface (verbatim):
```ts
export interface SlashCommandInfo {
	name: string;
	description?: string;
	source: SlashCommandSource;
	sourceInfo: SourceInfo;
}
```
Source: `ice:packages/coding-agent/src/core/slash-commands.ts:6-11`.

### 6. Dependencies

`AGENT-018`, `AGENT-020`, `EXT-011`, `EXT-012`, `EXT-013`, `EXT-035`, `SESSION-023`

External services/binaries: none. Agent active-tool snapshots, tool allowlists and resource command catalogs require cross-area entry references.

### 7. ice-rust today

rust:crates/ice-cli/src/extensions_actions.rs:181-190; rust:crates/ice-cli/src/extensions_actions.rs:516-532; rust:crates/ice-plugin-sdk/src/lib.rs:1022-1023; rust:crates/ice-plugin-sdk/src/lib.rs:1062-1064. Native GetActiveTools/SetActiveTools are real main-lane operations returning {tools}/null. The args helper requires an array but filters out nonstring elements, unlike a deep element validation error. There is no GetAllTools metadata action or GetCommands action in the SDK. GetExecutionContext supplies a safe tools snapshot but not the complete ToolInfo/source metadata projection. Native scheduling and active-set enforcement are delegated to the harness and need agent comparison rather than a blanket missing claim.

### 8. Port notes

Extend the native query API with owned ToolInfo and command-catalog DTOs built from existing registries/resource catalogs. Translate the synchronous compatibility methods onto the action bridge, document invalid-name handling and retain current allowlist/policy enforcement. Centralize active-tool and prompt rebuild logic in the existing harness; avoid exposing mutable schema references or a second active-set cache.

### 9. Size and priority

Estimated Rust implementation: **130-230 lines** (estimate, not measured source). **P1** - Extension orchestration needs accurate metadata and active-set control without bypassing the existing tool policy.

## EXT-055 - Extension model selection, thinking levels and change events

### 1. Title and status

Extension model selection, thinking levels and change events. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/extensions/types.ts:845-860; ice:packages/coding-agent/src/core/extensions/types.ts:1422-1429; ice:packages/coding-agent/src/core/agent-session.ts:1923-1943; ice:packages/coding-agent/src/core/agent-session.ts:2021-2073; ice:packages/coding-agent/src/core/agent-session.ts:2970-2976. setModel, getThinkingLevel, setThinkingLevel and selection event schemas.

Exported symbols: `SetModelHandler`, `GetThinkingLevelHandler`, `SetThinkingLevelHandler` (ice:packages/coding-agent/src/core/extensions/types.ts:1661; ice:packages/coding-agent/src/core/extensions/types.ts:1663; ice:packages/coding-agent/src/core/extensions/types.ts:1665).

### 3. What it does

Allow an extension to select an authenticated model and change reasoning level while the host preserves capability clamping and change records.

### 4. How it works

The extension setModel action accepts a complete model object and returns a Promise<boolean>, while thinking-level get/set use the current session state. The host remains responsible for authentication, model capability clamping, persistence and change events.

The bound model action first checks configured authentication and returns false if absent; otherwise it awaits the session model setter and returns true. The setter performs an asynchronous authentication check that may still throw, captures the previous model and effective switch thinking level, updates agent.state.model, appends a model-change record and updates default model/provider settings. It then reclamps thinking level and awaits model_select emission with source:"set". A true initial configuration check therefore does not guarantee later selection cannot fail.

setThinkingLevel computes the supported level or clamps it, always assigns effective state, and persists/emits only if the level actually changed. It appends a thinking-level record, updates default thinking settings under the implementation's supportsThinking/non-off condition, emits thinking_level_changed to the session stream and invokes extension thinking_level_select with level/previousLevel without awaiting. The public setter returns void. Model cycling/restoration can emit model_select with cycle/restore sources through other session paths; they are not extra values accepted by setModel itself.

### 5. Interfaces

```ts
export type ModelSelectSource = "set" | "cycle" | "restore";

/** Fired when a new model is selected */
export interface ModelSelectEvent {
	type: "model_select";
	model: Model<any>;
	previousModel: Model<any> | undefined;
	source: ModelSelectSource;
}

/** Fired when a new thinking level is selected */
export interface ThinkingLevelSelectEvent {
	type: "thinking_level_select";
	level: ThinkingLevel;
	previousLevel: ThinkingLevel;
}
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:845-860.
```ts
	/** Set the current model. Returns false if no API key available. */
	setModel(model: Model<any>): Promise<boolean>;

	/** Get current thinking level. */
	getThinkingLevel(): ThinkingLevel;

	/** Set thinking level (clamped to model capabilities). */
	setThinkingLevel(level: ThinkingLevel): void;
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:1422-1429.

### 6. Dependencies

`AI-005`, `AUTH-006`, `AUTH-024`, `AUTH-025`, `EXT-009`, `EXT-010`, `EXT-018`, `EXT-022`, `SET-011`

External services: configured provider/authentication when selection checks credentials. Resolve model registry, capability tables and model/settings persistence to agent/session/settings entries.

### 7. ice-rust today

rust:crates/ice-cli/src/extensions_actions.rs:138-158; rust:crates/ice-cli/src/extensions_actions.rs:534-574; rust:crates/ice-plugin-sdk/src/lib.rs:1024-1026. Native SetModel accepts a model identifier string, searches the authenticated catalog using case-insensitive exact match and then case-insensitive substring match, and returns {model:id} after the main-lane operation. It does not accept an arbitrary Model object or return false for an absent provider credential. GetThinkingLevel returns {level}; SetThinkingLevel serde-decodes the supplied value and delegates to the lane, returning null. Underlying lane/model behavior exists and is owned by agent/session; native event-field loss and asynchronous emitter semantics are described in EXT-022/EXT-033.

### 8. Port notes

Provide a versioned model-selection DTO with unambiguous provider/id identity and a compatibility boolean outcome distinct from transport/runtime errors. Avoid silently resolving a partial name when an exact model was requested. Reuse the native catalog, credential checks, capability clamping and persistence; project complete model/previous model and thinking change events without duplicating the model registry.

### 9. Size and priority

Estimated Rust implementation: **110-210 lines** (estimate, not measured source). **P1** - Different model identity and failure contracts can select the wrong model or conceal an authentication failure.

## EXT-056 - Extension direct-process execution API and captured working directory

### 1. Title and status

Extension direct-process execution API and captured working directory. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/core/extensions/types.ts:1403-1404; ice:packages/coding-agent/src/core/extensions/loader.ts:413-416; ice:packages/coding-agent/src/core/exec.ts:1-108; ice:packages/coding-agent/src/utils/child-process.ts:1-138. ExtensionAPI.exec and execCommand delegation.

`spawnProcess` — `ice:packages/coding-agent/src/utils/child-process.ts:18`. `spawnProcess` — `ice:packages/coding-agent/src/utils/child-process.ts:23`. `spawnProcess` — `ice:packages/coding-agent/src/utils/child-process.ts:24`. `spawnProcessSync` — `ice:packages/coding-agent/src/utils/child-process.ts:28`.

### 3. What it does

Let trusted extension code run a command with an argument vector and collect separate stdout/stderr plus its exit result.

### 4. How it works

ExtensionAPI.exec checks that its runtime is active and delegates to the shared direct-process helper. It chooses options.cwd when supplied, otherwise the working directory captured by the extension loader, and forwards timeout and abort options.

Although comments call it shell execution, execCommand uses spawn(command, args,{cwd, shell:false, stdio:["ignore","pipe","pipe"]}). Shell metacharacters are therefore ordinary argument text unless the extension explicitly invokes a shell binary. Stdout/stderr accumulate as strings without a capture cap. The helper starts the process before checking a pre-aborted signal, so pre-abort still creates a child and then requests termination. A positive timeout requests SIGTERM; the later five-second branch checks proc.killed, which indicates signal delivery rather than confirmed process exit, so it is not a proven reliable forced-kill guarantee. waitForChildProcess handles process termination and inherited stdio cleanup.

The returned fields are stdout, stderr, code and killed. Failure of the wait path resolves code 1, while a missing exit code on the success path becomes 0. This entry owns the extension capability boundary and captured-cwd choice; TOOLS-017 owns detailed shared helper/process semantics. There is no tool-call authorization record, sandbox, output truncation or session persistence added by this API. Native extensions are trusted executable code regardless of whether a convenience host exec API exists.

### 5. Interfaces

```ts
	/** Execute a shell command. */
	exec(command: string, args: string[], options?: ExecOptions): Promise<ExecResult>;
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:1403-1404.
```ts
export interface ExecOptions {
	/** AbortSignal to cancel the command */
	signal?: AbortSignal;
	/** Timeout in milliseconds */
	timeout?: number;
	/** Working directory */
	cwd?: string;
}

/**
 * Result of executing a shell command.
 */
export interface ExecResult {
	stdout: string;
	stderr: string;
	code: number;
	killed: boolean;
}
```
Source: ice:packages/coding-agent/src/core/exec.ts:11-28.

### 6. Dependencies

`EXT-006`, `EXT-010`, `TOOLS-017`

External binary: exactly the command selected by the extension, resolved by ordinary process spawning. TOOLS-017 is the confirmed shared exec helper entry, not an invented future ID.

### 7. ice-rust today

rust:crates/ice-plugin-sdk/src/lib.rs:1010-1084; rust:crates/ice-plugin-sdk/src/process.rs:1-165; rust:crates/ice-extensions/src/actions.rs:984-1265. The full native action enum and dispatch have no direct argv-exec host operation. SDK process support is ProcessTree/lifecycle support for plugin-owned subprocess cleanup, not a host call implementing this request/result shape. Native plugins can execute processes through their own trusted Rust code; that does not provide a uniform host cwd/cancellation/capture contract. TOOLS-017 owns the shared exact-argv execution helper and compares it with the native command-string shell abstraction; this entry owns the absence of the matching extension host action.

### 8. Port notes

If a host convenience API is required, add a typed argv execution request to ice-plugin-sdk and route it through the existing execution environment/process supervision in ice-tools. Define captured versus current cwd, timeout units, cancellation ownership, output limits and explicit spawn errors. Preserve shell:false semantics by using Command arguments directly; do not reproduce proc.killed's mistaken exit inference. Keep plugin trust separate from tool authorization.

### 9. Size and priority

Estimated Rust implementation: **130-230 lines** (estimate, not measured source). **P2** - Native plugins lack the common host process contract used by TypeScript extensions.

## EXT-057 - Inline extension identity, hidden display and before-user ordering

### 1. Title and status

Inline extension identity, hidden display and before-user ordering. **Status: different-by-design.**

### 2. ice source

ice:packages/coding-agent/src/core/extensions/types.ts:1589-1602; ice:packages/coding-agent/src/core/extensions/loader.ts:612-675; ice:packages/coding-agent/src/core/resource-loader.ts:660-700; ice:packages/coding-agent/src/core/resource-loader.ts:1020-1065. Tests: ice:packages/coding-agent/test/suite/regressions/6260-inline-extension-naming.test.ts:1-161.

### 3. What it does

Allow an embedding application to supply extension factories directly, label them for display and place application-owned guards before discovered user extensions.

### 4. How it works

An inline extension is either a factory function or metadata containing name, factory, optional hidden and optional priority. The resource loader separates before-user inline factories from normal ones and executes them around discovered user extensions in a defined order.

The load sequence is before-user inline extensions, discovered effective extension paths, then normal inline extensions. Each inline factory executes sequentially through the ordinary factory loader with a synthetic display path; named forms use <inline:name>, while bare factories use an ordinal identity. hidden affects startup extension-list display rather than preventing execution, and priority is copied onto the loaded extension after factory success. A failure is recorded for that factory and later inline factories continue according to the resource loader loop. Inline factories bypass module import/Jiti cache because the function is already supplied, while their staged registrations and commit behavior remain EXT-006.

The priority value "before-user" is reserved by convention for application-owned guards; arbitrary discovered packages do not obtain that ordering merely by adding a manifest field. It is not the same as handler priority after loading, and a hidden factory is still fully active. Identity/order are in-memory loading metadata rather than a persisted plugin package format. Trust admission of application versus project code remains the caller's responsibility.

### 5. Interfaces

```ts
/** Load-order priority for application-supplied inline extensions. */
export type InlineExtensionPriority = "normal" | "before-user";

export type InlineExtension =
	| ExtensionFactory
	| {
			/** Display name shown as `<inline:name>` in the startup Extensions list. */
			name: string;
			factory: ExtensionFactory;
			/** Omit this extension from the startup Extensions list. */
			hidden?: boolean;
			/** Run before discovered user extensions. Reserved for application-owned guards. */
			priority?: InlineExtensionPriority;
	  };
```
Source: ice:packages/coding-agent/src/core/extensions/types.ts:1589-1602.

### 6. Dependencies

`EXT-001`, `EXT-003`, `EXT-004`, `EXT-006`, `EXT-034`

External services/binaries: none for already-supplied factories. Embedding/resource loader and startup display contracts are cross-area dependencies.

Final dependency classification. Consuming integrations: `SESSION-028`, `SESSION-035`, `CLI-002`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

rust:crates/ice-extensions/src/loader.rs:1-761; rust:crates/ice-extensions/src/registry.rs:1-1330; rust:crates/ice-plugin-sdk/src/lib.rs:1-2169. Native loading accepts shared-library paths and registration callbacks through the ABI; it has no InlineExtension factory/name/hidden/before-user DTO. Registry event priority and platforms filter already provide native handler ordering, but they do not order factory initialization before user plugin loading or implement hidden synthetic identities. This is a language/runtime design difference, with the application-owned guard ordering requirement still applicable to the native host.

### 8. Port notes

Keep native library loading as the primary plugin model. If applications need in-process guards, expose an application-owned registration phase before user-plugin admission, with explicit provenance and display metadata; do not require a JavaScript factory emulation layer. Reuse native registries and lifetime owners, distinguish initialization order from event priority, and prevent untrusted metadata from claiming the application guard phase.

### 9. Size and priority

Estimated Rust implementation: **90-180 lines** (estimate, not measured source). **P2** - Embedding applications need predictable guard initialization and display provenance even when factory syntax differs.

