# Session, compaction and persistence

Source-based comparison of the abstract packages/agent harness session, the coding-agent runtime and version 3 SessionManager, and the SQLite adapters against the existing Rust implementations. All 48 assigned source files were read in full. Supporting source and test coverage, exact unread ranges, baseline limitations and redacted expressions are recorded in evidence/session-source-review.json and evidence/checkpoint-session.md. Application tests were not executed. Rust's native version 4 mutation log, canonical SQLite state and durable harness operations are preserved as distinct capabilities; a matching type name is not evidence of coding-session parity.

## SESSION-001 - Directional branch bounds and cursor semantics

### 1. Title and status

Directional branch bounds and cursor semantics. **Status: partial.**

### 2. ice source

ice:packages/agent/src/harness/session/types.ts:217-244 (EntryQuery, BranchBounds, RecordQuery); ice:packages/agent/src/harness/session/session.ts:226-254 (query validation and lane start); ice:packages/agent/src/harness/session/memory.ts:173-222 and ice:packages/agent/src/harness/session/memory.ts:268-309 (findEntriesOnBranch, walkToRoot, matchers); ice:packages/agent/src/harness/session/testing/conformance.ts:220-289 and ice:packages/agent/src/harness/session/testing/conformance.ts:352-397 (exact expected query results).

Exported symbols: `createSessionBackendConformance` (`ice:packages/agent/src/harness/session/testing/conformance.ts:83`).

Exported symbols: `SessionBackendFixture` (`ice:packages/agent/src/harness/session/testing/types.ts:4`), `SessionBackendFixtureFactory` (`ice:packages/agent/src/harness/session/testing/types.ts:9`), `SessionBackendConformanceCase` (`ice:packages/agent/src/harness/session/testing/types.ts:12`).

Exported symbols: `IdGenerator` (`ice:packages/agent/src/harness/session/types.ts:10`), `EntryOrder` (`ice:packages/agent/src/harness/session/types.ts:217`), `EntryCursor` (`ice:packages/agent/src/harness/session/types.ts:219`), `LanePointer` (`ice:packages/agent/src/harness/session/types.ts:260`), `LogOptions` (`ice:packages/agent/src/harness/session/types.ts:272`), `SessionStorage` (`ice:packages/agent/src/harness/session/types.ts:277`), `SessionErrorCode` (`ice:packages/agent/src/harness/session/types.ts:343`).

### 3. What it does

Callers can query all entries or a lane's ancestor chain in either order, stop at the first bound encountered in that order, apply filters and exclusive cursors, and cap matching results. Rust's oldest-first branch query currently returns the wrong segment when stopAtId or stopAtType is supplied.

### 4. How it works

Branch order determines where the stop boundary is first encountered. Bounds remain effective even when their entry fails the type/customType filter; the matching entry at the boundary is included before scanning stops.

The facade validates caller limit before any empty-lane return or limit-1 convenience override: supplied limit must be a positive integer; cursor sequence must be an integer >= 0. Errors are SessionError('invalid_query', 'limit must be a positive integer') and 'cursor sequence must be a non-negative integer'. The explicit start wins over the current lane leaf; a null current leaf yields []; nonexistent lane gives invalid_lane. In memory, newestFirst is the default and walks from start toward root, yielding each entry and then testing stopAtId/stopAtType. oldestFirst first walks the complete ancestor chain without bounds, reverses it, then tests the forward-order bound after filter-and-push. limit counts matches, not scanned nodes. Entry cursor is seq > afterSeq for oldestFirst and seq < afterSeq otherwise; it is independent of parent ancestry. Record afterSeq is always seq > afterSeq even when records are returned newest first. getLog is always chronological and excludes seq <= afterSeq. Missing explicit branch start is not_found; cycles or missing parents encountered after a valid start are invalid_entry. Reads return structured clones. No CLI flag, environment variable, network service or filesystem location belongs to this in-memory query operation.

### 5. Interfaces

EntryQuery = {type?: Entry['type']; customType?: string; order?: 'newestFirst'|'oldestFirst'; limit?: number; cursor?: {afterSeq:number}}. BranchBounds = {start?:string; stopAtType?:Entry['type']; stopAtId?:string}. Session.findEntriesOnBranch(query?), Session.findEntryOnBranch(query?), and view(lane) use lane leaf by default; storage.findEntriesOnBranch requires start. RecordQuery adds lane?, type?, runId?, afterSeq?, order?, limit?. No emitted event. Example fixture root -> old-note(custom) -> compact -> new-note(custom) -> tail: {start:'tail', stopAtType:'custom', order:'oldestFirst'} returns ['root','old-note']; the unfiltered default returns tip toward root. Filtering {stopAtId:'tail', type:'custom'} returns [].

Named storage/query support declarations:
```ts
export interface IdGenerator {
	next(): string;
}
export type EntryOrder = "newestFirst" | "oldestFirst";

export interface EntryCursor {
	afterSeq: number;
}

export interface EntryQuery {
	type?: Entry["type"];
	customType?: string;
	order?: EntryOrder;
	limit?: number;
	cursor?: EntryCursor;
}

export interface BranchBounds {
	start?: string;
	stopAtType?: Entry["type"];
	stopAtId?: string;
}

export interface RecordQuery {
	lane?: string;
	type?: LaneRecord["type"];
	runId?: string;
	afterSeq?: number;
	order?: EntryOrder;
	limit?: number;
}
export interface LanePointer {
	lane: string;
	leafId: string | null;
}

export type LogItem =
	| { kind: "entry"; seq: number; entry: Entry }
	| { kind: "record"; seq: number; record: LaneRecord }
	| { kind: "lane"; seq: number; lane: string; leafId: string | null }
	| { kind: "fact"; seq: number; fact: "name"; name: string }
	| { kind: "fact"; seq: number; fact: "label"; targetId: string; label: string | undefined };

export interface LogOptions {
	afterSeq?: number;
	limit?: number;
}

export interface SessionStorage<TMetadata extends SessionMetadata = SessionMetadata> {
	getMetadata(): Promise<TMetadata>;

	// Lanes
	getLanes(): Promise<{ lane: string; leafId: string | null }[]>;
	createLane(lane: string, at: string | null): Promise<void>;
	moveLane(lane: string, to: string | null): Promise<void>;

	// Entries and Records
	appendEntry<TEntry extends Entry>(entry: ProvisionedEntry<TEntry>, lane: string): Promise<TEntry>;
	appendRecord<TRecord extends LaneRecord>(record: NewRecord<TRecord>): Promise<TRecord>;

	// Reads
	getEntry(id: string): Promise<Entry | undefined>;
	findEntries(query?: EntryQuery): Promise<Entry[]>;
	/** start is mandatory here; defaulting to a lane's leaf is view sugar. */
	findEntriesOnBranch(query: EntryQuery & BranchBounds & { start: string }): Promise<Entry[]>;
	findRecords<K extends LaneRecord["type"]>(
		query: RecordQuery & { type: K },
	): Promise<Extract<LaneRecord, { type: K }>[]>;
	findRecords(query?: RecordQuery): Promise<LaneRecord[]>;
	getLog(options?: { afterSeq?: number; limit?: number }): Promise<LogItem[]>;

	// Global facts
	getName(): Promise<string | undefined>;
	setName(name: string): Promise<void>;
	getLabel(id: string): Promise<string | undefined>;
	setLabel(id: string, label: string | undefined): Promise<void>;
	getStats(): Promise<SessionStats>;
}
export type SessionErrorCode =
	| "not_found"
	| "already_exists"
	| "invalid_entry"
	| "invalid_payload"
	| "invalid_lane"
	| "invalid_query"
	| "invalid_fork_target"
	| "storage";
```

### 6. Dependencies

None. The same public query semantics must be maintained by each persistence backend; storage implementation comparisons are recorded separately.

### 7. ice-rust today

rust:crates/ice-harness/src/session/session.rs:55-83, rust:crates/ice-harness/src/session/session.rs:375-458 and rust:crates/ice-harness/src/session/state.rs:306-388 implement query validation and filtering. rust:crates/ice-harness/src/session/state.rs:546-580 walks backward with supplied bounds BEFORE rust:crates/ice-harness/src/session/state.rs:328-366 reverses the path for oldest-first and tests bounds again. In the cited TS conformance example this stops the backward walk at new-note, reverses [tail, new-note] to [new-note, tail], then stops immediately at new-note: ['new-note'] instead of ['root','old-note']. This is a source-read defect, not a runtime reproduction. Rust validates usize limit=0 and rejects u64::MAX as an artificial cursor sentinel; TS checks finite integer/nonnegative and has no matching explicit upper-bound sentinel. Rust also offers operation_kind filtering, guarded to operation_started, which is outside the TS RecordQuery surface. The shared Rust state routine is used by the in-memory backend at rust:crates/ice-harness/src/session/memory.rs:296-311.

### 8. Port notes

Target ice-harness session state. For oldest-first, materialize the full ancestor chain with no stop bounds, then apply the bound once while walking forward, after filter-and-push. Keep newest-first early termination. Preserve exclusive cursor direction, match-count limits, missing-start versus broken-parent errors, and validation before empty results. Port the concrete TS conformance fixture as a later implementation check; this audit did not create or execute tests. Decide and document the Rust-only u64 sentinel separately rather than silently claiming identical numeric input domains. Estimated work assumes existing query structs and storage delegation remain.

### 9. Size and priority

Estimated Rust implementation: **80-180 lines** (estimate, not measured source). **P1** - Wrong branch boundaries can omit the intended history or compaction boundary.

## SESSION-002 - Fork accounting, target errors and exact mutation history

### 1. Title and status

Fork accounting, target errors and exact mutation history. **Status: partial.**

### 2. ice source

ice:packages/agent/src/harness/session/types.ts:324-341 (ForkOptions and SessionRepo); ice:packages/agent/src/harness/session/memory.ts:69-113 and ice:packages/agent/src/harness/session/memory.ts:312-356 (fork and repository lifecycle); ice:packages/agent/src/harness/session/testing/conformance.ts:709-829 (branch/tree forks, zero statistics and default targets).

### 3. What it does

A fork creates a separate session containing selected branch history or the entire tree while preserving entry identities and selected global facts. Historical usage and recovery records are deliberately not charged or copied into the new session.

### 4. How it works

Forking copies structural history into a fresh store and starts its accounting at zero. Branch forks select a message and choose before/at semantics; tree forks preserve every lane and entry.

Repository create/fork accepts optional id; absent IDs use uuidv7, new createdAt uses Date.now(), duplicate id throws already_exists. Fork metadata parentSessionId defaults to the source id unless supplied. A missing source session gives not_found. Branch scope is the default. The selected entry is options.entryId or main leaf. If non-null, it must exist and be a message; either missing or non-message selection yields invalid_fork_target with 'Fork target is not a message entry: <id>'. Explicit target defaults position='before'; implicit main-leaf selection defaults 'at'; explicit position overrides either. Before means target.parentId; at means target.id. Null selected main leaf produces an empty branch. Branch entries are oldest-first ancestors through the selected boundary; only main lane survives. Tree scope copies all entries in insertion order and all lanes in insertion order. Each copied entry is structured-cloned, keeps id/parentId/timestamp, and receives a new consecutive seq starting at 1. Tree forks append lane log records after all entries; branch forks set main pointer directly without a lane log mutation. A defined name is copied next, then labels in their source Map insertion order only for copied targets. No record is copied; all five SessionStats fields, including messageCount, remain zero. Copied entry IDs remain reserved; omitted record IDs are not reserved in the child. The source stays untouched.

### 5. Interfaces

ForkOptions = {scope?:'branch';entryId?:string;position?:'before'|'at'} | {scope:'tree'}. SessionCreateOptions = {id?:string;parentSessionId?:string}. SessionRepo.fork(sourceMetadata, options) returns Promise<Session>. Base metadata is {id, createdAt, parentSessionId?}. LogItem entry carries {kind:'entry', seq, entry}; tree lane log carries {kind:'lane', seq, lane, leafId}; fact logs carry kind:'fact', fact:'name' or 'label'. The branch conformance fixture with three copied messages expects messageCount=0, cachedTokens=0, uncachedTokens=0, totalTokens=0, costTotal=0. No external interface, event, CLI flag or setting is defined here.

### 6. Dependencies

`SESSION-001`

### 7. ice-rust today

rust:crates/ice-harness/src/session/state.rs:438-541 correctly preserves IDs, timestamps, parent chains, default before/at behavior and excludes records. However missing selected id is not_found (461-463), unlike TS invalid_fork_target; all forks, including branch forks, append lane mutations (516-523), shifting subsequent fact seq values relative to TS; labels are copied in copied-entry order (531-539), not source label-insertion order. rust:crates/ice-harness/src/session/memory.rs:183-203 replays copied entries via apply_mutation; rust:crates/ice-harness/src/session/state.rs:191-208 increments message_count during that replay. Consequently Rust counts historical copied messages while TS deliberately returns all-zero fork stats. Rust's fork creates isolated owned state and retains no source records. Repo IDs, ancestry default and duplicate errors are at rust:crates/ice-harness/src/session/memory.rs:499-524. The Rust fork test at rust:crates/ice-harness/src/session/memory.rs:621-651 only checks that an entry exists; it does not establish the TS statistics/error/log contract.

### 8. Port notes

Target ice-harness fork mutation construction and initial accounting. Keep copied history structurally visible but reset all session-accounting fields after import, or use an import reducer mode which does not charge historical entries. Normalize selected missing/non-message error to invalid_fork_target if TS API parity is required. Preserve branch pointer without adding an observable lane mutation, or explicitly version the log translation and state why the additional mutation is intentional. Use an insertion-ordered fact/lane representation if exact mutation ordering is required. Do not copy source operation records or usage totals while correcting messageCount. Verify full-tree versus branch fork, explicit versus implicit target defaults, names/labels, ID reuse rules and independent mutation afterward during implementation.

### 9. Size and priority

Estimated Rust implementation: **120-300 lines** (estimate, not measured source). **P1** - Fork imports currently change accounting and observable recovery-log/error behavior.

## SESSION-003 - Stable insertion order of in-memory sessions, lanes and labels

### 1. Title and status

Stable insertion order of in-memory sessions, lanes and labels. **Status: partial.**

### 2. ice source

ice:packages/agent/src/harness/session/memory.ts:46-63, ice:packages/agent/src/harness/session/memory.ts:100-110, ice:packages/agent/src/harness/session/memory.ts:119-135 and ice:packages/agent/src/harness/session/memory.ts:312-348; ice:packages/agent/src/harness/session/testing/conformance.ts:87-132 and ice:packages/agent/src/harness/session/testing/conformance.ts:767-788.

### 3. What it does

The in-memory repository exposes session and lane lists in creation order and preserves global-label insertion order when copying fork facts. Rust currently uses lexical or unspecified map order for these surfaces.

### 4. How it works

These lists are projections of JavaScript Maps; creation order is observable and updating an existing key does not move it. No sort or timestamp tie-break is inserted by the TS storage.

The lane map begins with ['main', null]. createLane validates uniqueness/target, inserts the new lane, and logs its mutation. moveLane updates the existing key's leaf without reordering it. getLanes spreads that map. InMemorySessionRepo.create inserts new sessions and list obtains metadata with Promise.all over Map.values, retaining insertion order. delete is idempotent; deleting and recreating an id inserts it at the end. Name is a single last-value fact; setLabel replaces an existing value without reordering its key, removes the key when undefined, and re-adding a removed label inserts it at the end. Tree fork lane log order and copied-label fact order inherit these maps, after copied entries and optional name. No sort option is exposed and no user configuration changes this ordering.

### 5. Interfaces

getLanes(): Promise<{lane:string;leafId:string|null}[]>; SessionRepo.list(): Promise<SessionMetadata[]>; setLabel(id, label:string|undefined). createLane/moveLane return Promise<void>. Default first lane is 'main'. No CLI, environment variable, RPC shape or emitted event in this abstract storage layer.

### 6. Dependencies

`SESSION-002`

### 7. ice-rust today

rust:crates/ice-harness/src/session/state.rs:42-54 stores lanes in BTreeMap, labels in HashMap; get_lanes at 103-111 yields lexical lane order. Thus a newly created 'aaa' lane precedes 'main', unlike TS. rust:crates/ice-harness/src/session/memory.rs:435-490 stores sessions in HashMap and list iterates values without sorting or creation-order storage, so ordering is not guaranteed. Fork labels are traversed by copied-entry order at rust:crates/ice-harness/src/session/state.rs:531-539. Equivalent leaves, names and labels exist; the gap concerns ordering guarantees, not the existence of those capabilities.

### 8. Port notes

Target ice-harness in-memory session/state collections. Use an insertion-ordered map or maintain a separate insertion-order vector with well-defined delete/reinsert behavior. Retain direct lookup efficiency and synchronization under the existing mutex. Ensure all adapters which promise the same list surface preserve this order; do not infer an identical SQLite list contract from the in-memory contract. No additional service or host API is required.

### 9. Size and priority

Estimated Rust implementation: **40-110 lines** (estimate, not measured source). **P3** - Deterministic lists and fork fact sequence matter to consumers but do not block basic conversation persistence.

## SESSION-004 - Raw-entry session scanning and Unicode-safe search results

### 1. Title and status

Raw-entry session scanning and Unicode-safe search results. **Status: partial.**

### 2. ice source

ice:packages/agent/src/harness/session/search.ts:1-71 (SessionSearch, ScanningSessionSearch, createScanningSessionSearch and getFileSystemResultOrThrow).

### 3. What it does

The abstract TS search adapter scans every serialized session entry, including metadata and custom payload fields, and can restrict results to an exact working directory. Rust exposes a narrower ranked content search with a different result schema and unsafe Unicode slicing.

### 4. How it works

The TS adapter lists and opens sessions through a supplied repository, then scans all entries oldest first. It matches a case-insensitive substring of each complete serialized entry and returns every hit without ranking or truncation.

Normalize options.text with trim().toLowerCase(); an empty normalized query returns [] before repository reads. Iterate source.list() order. When cwd is supplied, compare it by strict equality to metadata.cwd; absent or non-string cwd cannot match a supplied string. Open each matching session, call findEntries({order:'oldestFirst'}), JSON.stringify each full entry, and compare payload.toLowerCase().includes(normalizedText). Return the original metadata, entryId, timestamp=new Date(entry.timestamp).toISOString(), and snippet=the complete JSON payload; score is omitted. The adapter does not catch listing, opening, query, serialization or timestamp failures. The auxiliary file-result adapter maps FileError.code=='not_found' to SessionError('not_found'), all other file errors to 'storage', prefixes '<message>: <file error message>', and retains the cause. There are no network requests, search indexes, tokenization, result limits, tool-name gates or custom-entry gates in this scanner.

### 5. Interfaces

SessionSearchOptions={text:string;cwd?:string}; SessionSearchHit<TMetadata>={metadata:TMetadata;entryId:string;timestamp:string;snippet?:string;score?:number}; SessionSearch.search(options):Promise<SessionSearchHit[]>; createScanningSessionSearch(source:Pick<SessionRepo,'list'|'open'>). No CLI flag, setting, environment variable, command or event is defined here. Query 'model_change' can match serialized entry type metadata; leading/trailing whitespace is ignored; an all-whitespace query yields no hits.

### 6. Dependencies

`SESSION-001`, `SESSION-003`

### 7. ice-rust today

rust:crates/ice-harness/src/session/search.rs:48-132 accepts a preloaded HashMap plus query and {limit, include_tool_calls, include_custom_entries}. It lowercases without trimming or an empty-query guard, ignores metadata, returns {session_id, entry_id, role, snippet, match_position, score}, sorts by occurrence/position score and optionally truncates. At 134-216 it searches user/assistant/tool-result text, optional tool names/arguments, and optional custom data; it skips custom-role messages and all model/thinking/tools/compaction/branch-summary entries. It therefore does not implement the TS raw JSON scanner or cwd/timestamp/metadata result contract. At 218-267, find on lowercased text yields UTF-8 byte offsets, then extract_snippet computes position +/- 100 and slices the original string at those raw offsets. Example source trace: text='中'.repeat(40)+'x', query='x' gives position=120 and start=20, which is not a UTF-8 boundary, so text[20..121] panics. Unicode lowercase length changes can also invalidate offsets. This is source-read evidence only; no runtime test was executed.

### 8. Port notes

Target ice-harness search. Add a repository list/open scanner compatible with the exact TS options/hit schema, without removing the Rust ranked search. Decide which API call sites need raw-entry versus ranked-content semantics. For the ranked implementation, use character-boundary-aware snippets and map normalized match offsets back to original text; never slice by unchecked byte arithmetic. Preserve all-whitespace behavior, oldest-first per-session order, exact cwd equality and error propagation in the TS-compatible adapter. No external search engine is necessary.

### 9. Size and priority

Estimated Rust implementation: **140-300 lines** (estimate, not measured source). **P1** - Current search loses queryable history fields and can panic on ordinary non-ASCII session text.

## SESSION-005 - SQLite session metadata and repository lifecycle

### 1. Title and status

SQLite session metadata and repository lifecycle. **Status: partial.**

### 2. ice source

`ice:packages/storage/sqlite-node/src/sqlite/types.ts:31-48`; `ice:packages/storage/sqlite-node/src/sqlite/storage/sessions.ts:1-92`; `ice:packages/storage/sqlite-node/src/sqlite/repo.ts:631-914`. Source-read tests: `ice:packages/storage/sqlite-node/test/repository.test.ts:91-115`, `ice:packages/storage/sqlite-node/test/repository.test.ts:156-258`.

Exported symbols: `SessionRow` (`ice:packages/storage/sqlite-node/src/sqlite/storage/sessions.ts:4`), `NewSessionRow` (`ice:packages/storage/sqlite-node/src/sqlite/storage/sessions.ts:12`), `sessionExists` (`ice:packages/storage/sqlite-node/src/sqlite/storage/sessions.ts:38`), `insertSessionRow` (`ice:packages/storage/sqlite-node/src/sqlite/storage/sessions.ts:51`), `readSessionRow` (`ice:packages/storage/sqlite-node/src/sqlite/storage/sessions.ts:61`), `readSessionRows` (`ice:packages/storage/sqlite-node/src/sqlite/storage/sessions.ts:67`), `deleteSessionRow` (`ice:packages/storage/sqlite-node/src/sqlite/storage/sessions.ts:79`).

Exported symbols: `SqliteSessionListOptions` (`ice:packages/storage/sqlite-node/src/sqlite/types.ts:44`), `SqliteSessionRepositoryEnv` (`ice:packages/storage/sqlite-node/src/sqlite/types.ts:48`).

### 3. What it does

SQLite sessions preserve the originating working directory, resolved database path, optional application-owned JSON metadata and parent override across creation, listing, reopening and forking. Callers can list without creating a missing database and explicitly close a reusable repository.

### 4. How it works

The repository lazily resolves one database path and memoizes one open connection; session metadata remains available through typed Session instances. Creating and forking preserve application metadata rather than treating it as an ignored create option.

Create accepts a required cwd, optional id (otherwise uuidv7), parentSessionId and metadata. It inserts an ISO created_at timestamp, a main lane with null leaf, next sequence 1 and zero statistics in one transaction, then acquires the writer claim. Metadata must be a non-null, non-array JSON object and pass assertJsonSerializable before encoding; missing metadata stores SQL NULL. Reading malformed JSON or a non-object metadata value is a storage error; rowToMetadata returns id, parsed createdAt, cwd, resolved path, and only present parentSessionId/metadata (`ice:packages/storage/sqlite-node/src/sqlite/storage/sessions.ts:12-79`).

list first resolves the path and asks env.exists; an absent database produces [] without opening or making directories. Existing rows sort created_at DESC. The current SQL helper applies a cwd predicate only when cwd is truthy, so empty-string cwd is not an exact filter here. Repeated open in one repository reuses an active storage handle after validating all lane leaves; a new repository must acquire a writer lease. Fork metadata defaults to the source metadata, but an explicitly supplied metadata object replaces it; parentSessionId defaults to the source id and can be overridden. Scope, copied entries and accounting are SESSION-002 (`ice:packages/storage/sqlite-node/src/sqlite/repo.ts:651-704`, `ice:packages/storage/sqlite-node/src/sqlite/repo.ts:743-849`).

close drains repository work, releases all active storages, closes the retained database and clears the open promise/database so later use can reopen it. Repeated close is harmless. Ordinary operation failure keeps the established connection for later use and disposal; setup failure closes the just-opened connection before propagating. Both async disposal and explicit close use this lifecycle (`ice:packages/storage/sqlite-node/src/sqlite/repo.ts:851-914`; source-read tests `ice:packages/storage/sqlite-node/test/repository.test.ts:156-206`).

### 5. Interfaces

SqliteSessionRepository({env:{absolutePath, createDir, exists}, sqlite:SqliteDatabaseFactory, databasePath, writerLease?}); create(SqliteSessionCreateOptions), open(SqliteSessionMetadata), list({cwd?}?), fork(source, options), delete(metadata), close(), [Symbol.asyncDispose](). SqliteSessionMetadata extends the base metadata with required cwd/path and optional parentSessionId/metadata. These paths describe canonical session storage, not observability.sqlite.

### 6. Dependencies

`SESSION-002`, `SESSION-006`

### 7. ice-rust today

Rust has a functioning SQLite repository, but its metadata row and public base metadata contain only id, created_at and parent_session_id (`rust:crates/ice-harness/src/session/sqlite.rs:191-219`, `rust:crates/ice-harness/src/session/sqlite.rs:246-331`). create ignores SessionCreateOptions.metadata; fork hardcodes Some(source.id) and ignores a supplied parent override (`rust:crates/ice-harness/src/session/sqlite.rs:280-295`, `rust:crates/ice-harness/src/session/sqlite.rs:342-369`). The JSONL-specific comment on SessionCreateOptions.metadata explicitly identifies another backend that preserves it; this is not a claim that all Rust storage loses metadata (`rust:crates/ice-harness/src/session/types.rs:1270-1278`). Rust opens/creates directories and the database eagerly, list has no cwd option, and connection lifetime follows Arc ownership rather than an explicit repository close/drain operation. The fully read SQLite tests cover durable reopen, sequential second-handle visibility, fork parent default, duplicate ids and delete, but not opaque metadata or cwd (`rust:crates/ice-harness/src/session/sqlite.rs:391-491`).

### 8. Port notes

Add a SQLite metadata wrapper or widen the repository trait without losing existing Rust JSONL metadata. Persist cwd and a validated JSON object, return the resolved database path, and implement list filtering/non-creating discovery and fork override semantics. Introduce an explicit close/drain lifecycle if exposing equivalent SDK ownership; retain safe Rust RAII as the fallback. Add source-derived tests for metadata round-trip and override, empty cwd behavior, absent-database list, repeated close and setup failure. Keep the storage schema migration decision in SESSION-007 rather than silently opening TS and Rust layouts interchangeably.

### 9. Size and priority

Estimated Rust implementation: **180-380 lines** (estimate, not measured source). **P2** - Missing metadata and cwd prevent equivalent repository/SDK integrations; basic durable session use already works.

## SESSION-006 - Fenced SQLite writer ownership and atomic session mutations

### 1. Title and status

Fenced SQLite writer ownership and atomic session mutations. **Status: partial.**

### 2. ice source

`ice:packages/storage/sqlite-node/src/sqlite/repo.ts:85-166`, `ice:packages/storage/sqlite-node/src/sqlite/repo.ts:330-489`, `ice:packages/storage/sqlite-node/src/sqlite/repo.ts:605-749`, `ice:packages/storage/sqlite-node/src/sqlite/repo.ts:851-914`; `ice:packages/storage/sqlite-node/src/sqlite/storage/leases.ts:1-65`; `ice:packages/storage/sqlite-node/src/index.ts:112-129`; `ice:packages/storage/sqlite-node/test/leases.test.ts:17-157`.

Exported symbols: `SqliteWriterLeaseOptions` (`ice:packages/storage/sqlite-node/src/sqlite/repo.ts:85`), `SqliteSessionRepositoryOptions` (`ice:packages/storage/sqlite-node/src/sqlite/repo.ts:92`).

Exported symbols: `acquireSessionLease` (`ice:packages/storage/sqlite-node/src/sqlite/storage/leases.ts:15`), `renewSessionLease` (`ice:packages/storage/sqlite-node/src/sqlite/storage/leases.ts:37`), `releaseSessionLease` (`ice:packages/storage/sqlite-node/src/sqlite/storage/leases.ts:55`), `deleteSessionLease` (`ice:packages/storage/sqlite-node/src/sqlite/storage/leases.ts:63`).

### 3. What it does

Only one repository owns a writable SQLite session at a time. Lease renewal, session mutations, sequence advancement, derived indexes and statistics commit together, and a writer whose lease expired cannot overwrite a new owner's work.

### 4. How it works

Each active session has a serialized operation queue and a renewable database lease identified by owner id and monotonically increasing fence. A write validates that same unexpired owner/fence inside its transaction before changing any session row; losing ownership permanently disables that handle's writes.

writerLease defaults to ttlMs=30000 and heartbeatIntervalMs=10000. Both must be positive safe integers and heartbeat must be less than TTL; invalid settings throw the exact positive/timing errors at construction (`ice:packages/storage/sqlite-node/src/sqlite/repo.ts:85-112`). Claim inserts fence=1, or replaces only an expired lease (expires_at_ms <= now), incrementing its fence. Renewal requires matching session/owner/fence and expires_at_ms > now; successful renewal sets now+TTL. Release deletes only matching owner/fence, so a stale close cannot delete the new owner's lease (`ice:packages/storage/sqlite-node/src/sqlite/storage/leases.ts:13-65`).

Repeated opens in one repository share storage and its write queue; another repository gets SessionError('storage', 'SQLite session <id> already has an active writer') until release/expiry. Each queued write uses a synchronous BEGIN IMMEDIATE transaction, renews the claim, then mutates entries/records/lane pointers/cache/statistics/next sequence. A failed renewal sets the handle's leaseError to 'SQLite session <id> writer lease was lost', stops its heartbeat and rejects future writes. The queue survives a failed operation so later work is not permanently rejected. Reads do not claim an additional lease (`ice:packages/storage/sqlite-node/src/sqlite/repo.ts:114-144`, `ice:packages/storage/sqlite-node/src/sqlite/repo.ts:330-489`).

An unref timer queues idle renewal at each heartbeat interval. Transient renewal errors are caught and retried on a later heartbeat, but each real write still checks ownership atomically. release marks closing before enqueueing its final lease deletion, clears the timer, drains earlier writes and removes the active handle in finally. New writes to a closed handle fail. Delete and repair release local handles, then acquire their own claim in their transaction; another writer still blocks them. Repository close drains and releases all handles before closing the connection (`ice:packages/storage/sqlite-node/src/sqlite/repo.ts:344-408`, `ice:packages/storage/sqlite-node/src/sqlite/repo.ts:697-749`, `ice:packages/storage/sqlite-node/src/sqlite/repo.ts:851-872`).

Database setup explicitly sets WAL, synchronous=FULL and busy_timeout=5000. The adapter's transaction rejects any returned thenable with TypeError('SQLite transaction callbacks must be synchronous'), rolls back on failure and preserves the original failure if rollback itself fails (`ice:packages/storage/sqlite-node/src/index.ts:112-129`). Source-read tests exercise shared-handle ordering, invalid timers, second-writer rejection, expiry takeover, stale close, cross-session writes and idle renewal; rollback tests preserve entries/lane/statistics after a trigger failure (`ice:packages/storage/sqlite-node/test/leases.test.ts:17-157`; `ice:packages/storage/sqlite-node/test/repository.test.ts:313-343`).

### 5. Interfaces

writerLease?: {ttlMs?: number, heartbeatIntervalMs?: number}; private lease state {sessionId, ownerId, fence, expiresAtMs}; leases(session_id PK, owner_id TEXT, fence INTEGER, expires_at_ms INTEGER). Ownership conflicts and stale owners use SessionError code storage. Mutation success must mean the entire canonical write committed, not merely that an in-memory snapshot changed.

### 6. Dependencies

`SESSION-007`

### 7. ice-rust today

The Rust adapter serializes individual SQL statements through Arc<Mutex<Connection>>, but refresh, in-memory mutation and persist are separate critical sections (`rust:crates/ice-harness/src/session/sqlite.rs:44-82`, `rust:crates/ice-harness/src/session/sqlite.rs:94-114`, `rust:crates/ice-harness/src/session/sqlite.rs:155-167`). No lease table, fence, version predicate or encompassing transaction exists; setup only selects WAL (`rust:crates/ice-harness/src/session/sqlite.rs:191-219`). Independent handles each load their own reducer state (`rust:crates/ice-harness/src/session/sqlite.rs:297-307`). Source-derived concurrent interleaving: A and B both refresh snapshot S; A appends entry a and writes S+a; B appends b to its S and writes S+b; both calls can succeed, losing a. This requires concurrent execution, such as separate threads/processes/independent repositories; the single-thread sequential test at rust:crates/ice-harness/src/session/sqlite.rs:391-460 does not exercise it. The same design permits refresh/persist races with cloned storage. This is a source-level concurrency finding, not an executed reproduction. The Rust insert and individual UPDATE remain atomic SQL statements; they do not make the whole read-modify-write atomic.

### 8. Port notes

Put all per-session read-modify-write state, durable update and statistics/sequence work under one transaction or compare-and-swap protocol; do not rely on a mutex that is released between snapshot load and save. Add a persisted owner/fence/expiry claim with the TS timing rules, shared handle queue, heartbeat and closed/lost terminal states. For an idiomatic snapshot backend, a revision CAS plus fenced owner check can preserve Rust's reducer model, but must reject or retry conflicting snapshots without losing successful writes. Keep claim release guarded by both owner and fence and drain outstanding mutations before close. Verify with controlled concurrent writers, stale takeover, failed commit and heartbeat/source-derived scenarios; no tests were run during this audit.

### 9. Size and priority

Estimated Rust implementation: **350-750 lines** (estimate, not measured source). **P1** - Concurrent accepted writes can overwrite one another; lease ownership and transaction boundaries protect durable conversation state.

## SESSION-007 - Relational canonical SQLite layout and versioned migration contract

### 1. Title and status

Relational canonical SQLite layout and versioned migration contract. **Status: different-by-design.**

### 2. ice source

`ice:packages/storage/sqlite-node/src/sqlite/migrations.ts:1-51`; `ice:packages/storage/sqlite-node/src/sqlite/migrations/001_initial.sql:1-124`; `ice:packages/storage/sqlite-node/src/sqlite/repo.ts:188-321`, `ice:packages/storage/sqlite-node/src/sqlite/repo.ts:410-603`; `ice:packages/storage/sqlite-node/src/sqlite/storage/records.ts:1-95`; `ice:packages/storage/sqlite-node/src/sqlite/storage/session-sequences.ts:1-28`; `ice:packages/storage/sqlite-node/src/sqlite/storage/session-stats.ts:1-63`.

Exported symbols: `SessionStopReason` (`ice:packages/agent/src/harness/session/types.ts:8`), `EntryBase` (`ice:packages/agent/src/harness/session/types.ts:14`), `MessageEntry` (`ice:packages/agent/src/harness/session/types.ts:22`), `ModelChangeEntry` (`ice:packages/agent/src/harness/session/types.ts:28`), `ThinkingLevelEntry` (`ice:packages/agent/src/harness/session/types.ts:34`), `ActiveToolsEntry` (`ice:packages/agent/src/harness/session/types.ts:39`), `RecordBase` (`ice:packages/agent/src/harness/session/types.ts:80`), `OperationStartedRecord` (`ice:packages/agent/src/harness/session/types.ts:87`), `AbortRequestedRecord` (`ice:packages/agent/src/harness/session/types.ts:115`), `OperationFinishedRecord` (`ice:packages/agent/src/harness/session/types.ts:120`), `CompactionReason` (`ice:packages/agent/src/harness/session/types.ts:127`), `StepAttemptRecord` (`ice:packages/agent/src/harness/session/types.ts:129`), `ToolStartedRecord` (`ice:packages/agent/src/harness/session/types.ts:150`), `QueueEnqueuedRecord` (`ice:packages/agent/src/harness/session/types.ts:162`), `QueueCancelledRecord` (`ice:packages/agent/src/harness/session/types.ts:178`), `WriteDeferredRecord` (`ice:packages/agent/src/harness/session/types.ts:184`), `UsageRecord` (`ice:packages/agent/src/harness/session/types.ts:190`), `NewRecord` (`ice:packages/agent/src/harness/session/types.ts:213`).

Exported symbols: `wrapNodeSqliteDatabase` (`ice:packages/storage/sqlite-node/src/index.ts:136`).

Exported symbols: `SqliteMigration` (`ice:packages/storage/sqlite-node/src/sqlite/migrations.ts:5`), `loadMigrations` (`ice:packages/storage/sqlite-node/src/sqlite/migrations.ts:15`).

Exported symbols: `EntryRow` (`ice:packages/storage/sqlite-node/src/sqlite/storage/entries.ts:4`), `NewEntryRow` (`ice:packages/storage/sqlite-node/src/sqlite/storage/entries.ts:14`), `entryPayload` (`ice:packages/storage/sqlite-node/src/sqlite/storage/entries.ts:23`), `insertEntryRow` (`ice:packages/storage/sqlite-node/src/sqlite/storage/entries.ts:32`), `readEntryRow` (`ice:packages/storage/sqlite-node/src/sqlite/storage/entries.ts:38`), `readEntryRows` (`ice:packages/storage/sqlite-node/src/sqlite/storage/entries.ts:46`), `idExistsInEntries` (`ice:packages/storage/sqlite-node/src/sqlite/storage/entries.ts:67`), `deleteEntryRows` (`ice:packages/storage/sqlite-node/src/sqlite/storage/entries.ts:73`).

Exported symbols: `FactRow` (`ice:packages/storage/sqlite-node/src/sqlite/storage/facts.ts:3`), `appendFact` (`ice:packages/storage/sqlite-node/src/sqlite/storage/facts.ts:11`), `readLatestFact` (`ice:packages/storage/sqlite-node/src/sqlite/storage/facts.ts:28`), `readLatestLabelFacts` (`ice:packages/storage/sqlite-node/src/sqlite/storage/facts.ts:40`), `readFactRows` (`ice:packages/storage/sqlite-node/src/sqlite/storage/facts.ts:54`), `deleteFactRows` (`ice:packages/storage/sqlite-node/src/sqlite/storage/facts.ts:71`).

Exported symbols: `LaneRow` (`ice:packages/storage/sqlite-node/src/sqlite/storage/lanes.ts:4`), `LaneMoveRow` (`ice:packages/storage/sqlite-node/src/sqlite/storage/lanes.ts:10`), `createInitialLane` (`ice:packages/storage/sqlite-node/src/sqlite/storage/lanes.ts:17`), `readLanes` (`ice:packages/storage/sqlite-node/src/sqlite/storage/lanes.ts:21`), `readLane` (`ice:packages/storage/sqlite-node/src/sqlite/storage/lanes.ts:44`), `readLaneHead` (`ice:packages/storage/sqlite-node/src/sqlite/storage/lanes.ts:50`), `setLaneLeaf` (`ice:packages/storage/sqlite-node/src/sqlite/storage/lanes.ts:80`), `readLaneMoveRows` (`ice:packages/storage/sqlite-node/src/sqlite/storage/lanes.ts:87`), `deleteLaneRows` (`ice:packages/storage/sqlite-node/src/sqlite/storage/lanes.ts:104`).

Exported symbols: `RecordRow` (`ice:packages/storage/sqlite-node/src/sqlite/storage/records.ts:3`), `NewRecordRow` (`ice:packages/storage/sqlite-node/src/sqlite/storage/records.ts:15`), `appendRecordRow` (`ice:packages/storage/sqlite-node/src/sqlite/storage/records.ts:26`), `idExistsInRecords` (`ice:packages/storage/sqlite-node/src/sqlite/storage/records.ts:44`), `deleteRecordRows` (`ice:packages/storage/sqlite-node/src/sqlite/storage/records.ts:50`), `readRecordRows` (`ice:packages/storage/sqlite-node/src/sqlite/storage/records.ts:54`).

Exported symbols: `createSequence` (`ice:packages/storage/sqlite-node/src/sqlite/storage/session-sequences.ts:4`), `getNextSequence` (`ice:packages/storage/sqlite-node/src/sqlite/storage/session-sequences.ts:8`), `setNextSequence` (`ice:packages/storage/sqlite-node/src/sqlite/storage/session-sequences.ts:18`), `advanceSequence` (`ice:packages/storage/sqlite-node/src/sqlite/storage/session-sequences.ts:22`), `deleteSequence` (`ice:packages/storage/sqlite-node/src/sqlite/storage/session-sequences.ts:26`).

Exported symbols: `SessionStatsRow` (`ice:packages/storage/sqlite-node/src/sqlite/storage/session-stats.ts:5`), `createStats` (`ice:packages/storage/sqlite-node/src/sqlite/storage/session-stats.ts:14`), `readStats` (`ice:packages/storage/sqlite-node/src/sqlite/storage/session-stats.ts:22`), `incrementMessageCount` (`ice:packages/storage/sqlite-node/src/sqlite/storage/session-stats.ts:40`), `addUsageToStats` (`ice:packages/storage/sqlite-node/src/sqlite/storage/session-stats.ts:47`), `deleteStats` (`ice:packages/storage/sqlite-node/src/sqlite/storage/session-stats.ts:61`).

Exported symbols: `SqliteRunResult` (`ice:packages/storage/sqlite-node/src/sqlite/types.ts:4`), `SqliteStatement` (`ice:packages/storage/sqlite-node/src/sqlite/types.ts:12`).

### 3. What it does

The TypeScript SQLite backend stores canonical entries and mutation history as queryable rows with an explicit migration ledger. Rust deliberately persists a serialized reducer snapshot, so its SQLite files are not interchangeable with this schema and its error/read granularity differs.

### 4. How it works

TypeScript uses entries.parent_id as the canonical conversation tree and a session-wide monotonically increasing sequence shared across entries, records, lane moves and facts. Derived branch membership, pointers, statistics and search indexes are separate rows maintained transactionally.

001_initial.sql creates sessions(id PK, created_at TEXT, cwd TEXT, parent_session_id nullable, metadata nullable); entries(session_id, id composite PK, seq unique per session, parent_id nullable, type, timestamp TEXT, payload JSON text); session_sequences(session_id PK, next_seq); session_stats(session_id PK, message_count, cached_tokens REAL, uncached_tokens REAL, total_tokens REAL, cost_total REAL); lanes(session_id, lane composite PK, leaf_id nullable); records(session_id, id composite PK, seq unique, lane, run_id nullable, type, op_kind nullable, timestamp, payload); lane_moves(session_id, seq composite PK, lane, leaf_id nullable); facts(session_id, seq composite PK, kind, key nullable, value nullable); branch_entries/branch_tips and leases are specified in SESSION-008/006. All tables except entries are WITHOUT ROWID. Indexes cover session time/cwd/parent, entry seq/parent/type, record lane/type/op-kind/run, lane leaves, fact kind/key and branch membership (`ice:packages/storage/sqlite-node/src/sqlite/migrations/001_initial.sql:1-124`).

The migration runner reads only its declared 001_initial.sql resource, creates migrations(id PK, applied_at TEXT), reads applied IDs and skips them. Each unapplied script and its ISO applied_at insertion run in one transaction; running again does not reapply it (`ice:packages/storage/sqlite-node/src/sqlite/migrations.ts:9-51`; source-read test `ice:packages/storage/sqlite-node/test/migrations.test.ts:6-40`). Sequence allocation starts at 1 and advances only with a successful transaction. Entry payload omits the base id/seq/parentId/type/timestamp columns. Record payload similarly excludes canonical seq/time; op_kind is populated only for operation_started and its run_id is its own id, whereas other records use their runId field. Entry/record ids share a session-wide uniqueness check. Statistics increase messageCount only for message entry acceptance and usage totals only for usage records; summary/assistant embedded usage alone is not charged (`ice:packages/storage/sqlite-node/src/sqlite/repo.ts:318-321`, `ice:packages/storage/sqlite-node/src/sqlite/repo.ts:410-489`; `ice:packages/storage/sqlite-node/src/sqlite/storage/session-stats.ts:1-63`).

Entry decoding validates object payloads, finite timestamps and each supported entry variant's required fields; invalid payloads are invalid_entry with an id-bearing error/cause. Record JSON decoding failures are storage errors. General entry queries decode the selected ordered rows before applying in-memory filters/limits, while branch windows limit rows before decoding (SESSION-008). getLog independently reads all four mutation sources after sequence, merges/sorts by seq and then applies limit. Name/label facts use latest-wins; label clearing stores NULL, reads omit cleared labels and sort surviving labels by key. Opening or listing lanes rejects dangling leaf pointers (`ice:packages/storage/sqlite-node/src/sqlite/repo.ts:188-307`, `ice:packages/storage/sqlite-node/src/sqlite/repo.ts:491-603`; `ice:packages/storage/sqlite-node/src/sqlite/storage/facts.ts:28-68`; `ice:packages/storage/sqlite-node/src/sqlite/storage/lanes.ts:21-64`).

The lower-level row helpers use bound SQL parameters. `entryPayload` removes type/id/seq/parentId/timestamp and returns the remaining fields; `readEntryRows` defaults to newest-first and applies `seq > afterSeq` independently of order. Fact logs use nullable key/value, latest facts order by seq descending, surviving labels use the latest per-key non-NULL value ordered by key, and raw fact/lane-move logs are chronological with an exclusive afterSeq. `createInitialLane` defaults to main/null without writing a lane-move record; `setLaneLeaf` also changes only the pointer and rejects an absent lane. `deleteLaneRows` removes move history before lanes. Sequence creation starts at 1, advancement stores acceptedSeq+1, and missing sequence/stat rows raise storage errors. Statistics add cacheRead to cached_tokens, input+cacheWrite to uncached_tokens, usage.totalTokens to total_tokens and usage.cost.total to cost_total (`ice:packages/storage/sqlite-node/src/sqlite/storage/entries.ts:23-75`; `ice:packages/storage/sqlite-node/src/sqlite/storage/facts.ts:11-73`; `ice:packages/storage/sqlite-node/src/sqlite/storage/lanes.ts:17-106`; `ice:packages/storage/sqlite-node/src/sqlite/storage/session-sequences.ts:4-27`; `ice:packages/storage/sqlite-node/src/sqlite/storage/session-stats.ts:14-62`).

### 5. Interfaces

Exported applyMigrations(SqliteDatabase), SqliteDatabaseFactory.open(path), synchronous SqliteDatabase.exec/prepare/transaction/close and statement run/get/all. Run results normalize changes and optional lastInsertRowid to numbers. The Node adapter prefers node:sqlite through process.getBuiltinModule and falls back to bun:sqlite; Bun no-row null becomes undefined and strict mode is enabled (`ice:packages/storage/sqlite-node/src/index.ts:1-170`). These JavaScript driver adapters are platform-specific, not a requirement to embed Node/Bun in Rust.

Canonical harness entry and record payload declarations supply the variant fields serialized in these relational rows. They are distinct from coding-agent JSONL v3 in SESSION-029:
```ts
export type SessionStopReason = Exclude<StopReason, "pending"> | "deferred";
export interface EntryBase {
	type: string;
	id: string;
	seq: number;
	parentId: string | null;
	timestamp: number;
}

export interface MessageEntry extends EntryBase {
	type: "message";
	message: AgentMessage;
	terminate?: true;
}

export interface ModelChangeEntry extends EntryBase {
	type: "model_change";
	provider: string;
	modelId: string;
}

export interface ThinkingLevelEntry extends EntryBase {
	type: "thinking_level_change";
	thinkingLevel: string;
}

export interface ActiveToolsEntry extends EntryBase {
	type: "active_tools_change";
	activeToolNames: string[];
}

export interface CompactionEntry extends EntryBase {
	type: "compaction";
	summary: string;
	retainedTail: AgentMessage[];
	tokensBefore: number;
	details?: unknown;
	usage?: Usage;
}

export interface BranchSummaryEntry extends EntryBase {
	type: "branch_summary";
	fromId: string;
	summary: string;
	details?: unknown;
	usage?: Usage;
}

export interface CustomEntry extends EntryBase {
	type: "custom";
	customType: string;
	data?: unknown;
}

export type Entry =
	| MessageEntry
	| ModelChangeEntry
	| ThinkingLevelEntry
	| ActiveToolsEntry
	| CompactionEntry
	| BranchSummaryEntry
	| CustomEntry;

export type ProvisionedEntry<TEntry extends Entry = Entry> = TEntry extends Entry
	? Omit<TEntry, "parentId" | "seq" | "timestamp">
	: never;

export interface RecordBase {
	id: string;
	seq: number;
	lane: string;
	timestamp: number;
}

export interface OperationStartedRecord extends RecordBase {
	type: "operation_started";
	sourceLeafId: string | null;
	intent:
		| {
				kind: "run";
				/** Normalized caller input before before_run; kept for suspended operations and before_resume. */
				originalPrompt: AgentMessage[];
				/** Captured nextRun items, then the prompt, then before_run injections. */
				initialMessages: ProvisionedEntry[];
				systemPromptOverride?: string;
				resumeData?: { [extensionId: string]: JsonValue };
		  }
		| {
				kind: "compaction";
				customInstructions?: string;
				resultEntryId: string;
		  }
		| {
				kind: "navigation";
				targetId: string | null;
				summarize: boolean;
				customInstructions?: string;
				label?: string;
				summaryEntryId?: string;
		  };
}

export interface AbortRequestedRecord extends RecordBase {
	type: "abort_requested";
	runId: string;
}

export interface OperationFinishedRecord extends RecordBase {
	type: "operation_finished";
	runId: string;
	outcome: "completed" | "aborted" | "failed" | "declined";
	error?: { code: string; message: string };
}

export type CompactionReason = "manual" | "threshold" | "overflow";

export type StepAttemptRecord = RecordBase &
	(
		| {
				type: "step_attempt";
				runId: string;
				step: "assistant" | "branch_summary";
				attempt: number;
				resultEntryId: string;
				compactionReason?: never;
		  }
		| {
				type: "step_attempt";
				runId: string;
				step: "compaction";
				attempt: number;
				resultEntryId: string;
				/** Persists why compaction summary generation started so recovery resumes the same work. */
				compactionReason: CompactionReason;
		  }
	);

export interface ToolStartedRecord extends RecordBase {
	type: "tool_started";
	runId: string;
	assistantEntryId: string;
	toolIndex: number;
	toolCallId: string;
	toolName: string;
	effectiveArgs: { [key: string]: unknown };
	resultEntryId: string;
	replay: "never" | "safe";
}

export type QueueEnqueuedRecord = RecordBase &
	(
		| {
				type: "queue_enqueued";
				queue: "steer" | "followUp";
				runId: string;
				target: ProvisionedEntry;
		  }
		| {
				type: "queue_enqueued";
				queue: "nextRun";
				runId?: never;
				target: ProvisionedEntry;
		  }
	);

export interface QueueCancelledRecord extends RecordBase {
	type: "queue_cancelled";
	runId?: string;
	entryId: string;
}

export interface WriteDeferredRecord extends RecordBase {
	type: "write_deferred";
	runId: string;
	target: ProvisionedEntry;
}

export type UsageRecord = RecordBase & { type: "usage"; usage: Usage } & (
		| {
				cause: "assistant" | "compaction" | "branch_summary" | "deferred_fetch";
				runId: string;
				entryId: string;
				attempt: number;
				stopReason: SessionStopReason;
		  }
		| { cause: "tool"; runId: string; entryId: string; toolCallId: string }
		| { cause: "hook"; runId: string; entryId: string }
		| { cause: "adjustment"; runId?: string; entryId?: string; details?: JsonValue }
	);

export type LaneRecord =
	| OperationStartedRecord
	| AbortRequestedRecord
	| OperationFinishedRecord
	| StepAttemptRecord
	| ToolStartedRecord
	| QueueEnqueuedRecord
	| QueueCancelledRecord
	| WriteDeferredRecord
	| UsageRecord;
export type NewRecord<TRecord extends LaneRecord = LaneRecord> = TRecord extends LaneRecord
	? Omit<TRecord, "seq" | "timestamp">
	: never;
```
The adapter's SQL statement contract is:
```ts
export interface SqliteRunResult {
	/** Number of rows changed by the statement. */
	changes: number;
	/** Inserted row id when the backend exposes one. */
	lastInsertRowid?: number;
}

/** Prepared SQLite statement capability used by the SQLite session backend. */
export interface SqliteStatement {
	run(...params: unknown[]): SqliteRunResult;
	get<TRow extends object>(...params: unknown[]): TRow | undefined;
	all<TRow extends object>(...params: unknown[]): TRow[];
}
```

### 6. Dependencies

`SESSION-001`, `SESSION-002`, `SESSION-006`, `SESSION-008`

### 7. ice-rust today

Rust explicitly chooses serialized SessionState as its canonical SQLite representation (`rust:crates/ice-harness/src/session/sqlite.rs:1-7`). The only table is sessions(id TEXT PK, created_at INTEGER, parent_session_id TEXT, state_json TEXT); no migration ledger or per-entry tables are present (`rust:crates/ice-harness/src/session/sqlite.rs:191-219`). Every read except metadata reloads and deserializes the entire state; decoding failure is a storage error and can affect a narrow branch query even when the damaged part would be outside the requested branch window (`rust:crates/ice-harness/src/session/sqlite.rs:66-170`, `rust:crates/ice-harness/src/session/sqlite.rs:246-273`). SQL list sorts by created_at, while entry/log/fact queries delegate to the already-reviewed in-memory reducer. This is a genuine alternate durable backend, not absence of SQLite storage. Its whole-snapshot architecture is separable from the concurrent-write defect in SESSION-006.

### 8. Port notes

Decide whether Rust must directly consume the TypeScript canonical database. If so, implement versioned schema detection and a transactional importer/relational backend using the exact row semantics, including id/seq uniqueness, NULL tombstones, ISO timestamps and distinct decode errors; never run CREATE IF NOT EXISTS against an incompatible sessions table and assume compatibility. If retaining snapshots, document the format boundary and preserve equivalent public queries/error guarantees where required. Introduce explicit migrations for future Rust state/schema changes. Keep JavaScript runtime adapter differences documented as platform adaptations. Estimated work below assumes an additive compatible relational backend, not merely a format disclaimer.

### 9. Size and priority

Estimated Rust implementation: **650-1300 lines** (estimate, not measured source). **P2** - The alternate layout is intentional, but migration/interoperability and query-local recovery require an explicit supported contract.

## SESSION-008 - Derived SQLite branch cache, bounded reads and explicit repair

### 1. Title and status

Derived SQLite branch cache, bounded reads and explicit repair. **Status: different-by-design.**

### 2. ice source

`ice:packages/storage/sqlite-node/src/sqlite/branch-cache.ts:14-105`; `ice:packages/storage/sqlite-node/src/sqlite/storage/branch-entries.ts:26-145`; `ice:packages/storage/sqlite-node/src/sqlite/storage/branch-tips.ts:1-40`; `ice:packages/storage/sqlite-node/src/sqlite/repo.ts:294-307`, `ice:packages/storage/sqlite-node/src/sqlite/repo.ts:504-528`, `ice:packages/storage/sqlite-node/src/sqlite/repo.ts:697-715`; source-read tests `ice:packages/storage/sqlite-node/test/branch-cache.test.ts:14-219`.

Exported symbols: `deleteBranchCache` (`ice:packages/storage/sqlite-node/src/sqlite/branch-cache.ts:14`), `rebuildBranchCache` (`ice:packages/storage/sqlite-node/src/sqlite/branch-cache.ts:19`), `buildCachedBranch` (`ice:packages/storage/sqlite-node/src/sqlite/branch-cache.ts:35`), `appendEntryToBranchCache` (`ice:packages/storage/sqlite-node/src/sqlite/branch-cache.ts:74`).

Exported symbols: `CachedBranch` (`ice:packages/storage/sqlite-node/src/sqlite/storage/branch-entries.ts:5`), `CachedBranchEntryRow` (`ice:packages/storage/sqlite-node/src/sqlite/storage/branch-entries.ts:10`), `CachedBranchQuery` (`ice:packages/storage/sqlite-node/src/sqlite/storage/branch-entries.ts:20`), `readCachedBranch` (`ice:packages/storage/sqlite-node/src/sqlite/storage/branch-entries.ts:26`), `queryCachedBranchRows` (`ice:packages/storage/sqlite-node/src/sqlite/storage/branch-entries.ts:36`), `deleteBranchEntries` (`ice:packages/storage/sqlite-node/src/sqlite/storage/branch-entries.ts:81`), `insertBranchEntry` (`ice:packages/storage/sqlite-node/src/sqlite/storage/branch-entries.ts:85`), `insertBranchEntriesForPath` (`ice:packages/storage/sqlite-node/src/sqlite/storage/branch-entries.ts:101`), `readBranchContainingEntry` (`ice:packages/storage/sqlite-node/src/sqlite/storage/branch-entries.ts:120`), `copyBranchEntriesThroughSeq` (`ice:packages/storage/sqlite-node/src/sqlite/storage/branch-entries.ts:133`).

Exported symbols: `readBranchTipIds` (`ice:packages/storage/sqlite-node/src/sqlite/storage/branch-tips.ts:3`), `readBranchTipBranchId` (`ice:packages/storage/sqlite-node/src/sqlite/storage/branch-tips.ts:10`), `insertBranchTip` (`ice:packages/storage/sqlite-node/src/sqlite/storage/branch-tips.ts:17`), `updateBranchTip` (`ice:packages/storage/sqlite-node/src/sqlite/storage/branch-tips.ts:25`), `deleteBranchTips` (`ice:packages/storage/sqlite-node/src/sqlite/storage/branch-tips.ts:38`).

### 3. What it does

The SQLite backend maintains complete root-to-tip branch membership as a derived acceleration structure, limits compaction-window reads before decoding payloads, and exposes explicit cache repair. Missing/stale cache state is reported rather than silently repaired during ordinary reads or writes.

### 4. How it works

Canonical parent links remain in entries; the cache records their complete paths, including entries before old compactions. Queries choose only the requested directional window from this cache so malformed historical payloads outside that window do not prevent reading current context.

Appending a root creates a new uuidv7 branch id, membership row and tip. Appending to an existing tip inserts membership and updates exactly that tip; a failed tip update throws invalid_entry('Branch tip <parent> changed during append'). Appending from an internal entry finds the lexicographically first containing branch, copies membership through the parent's seq into a new branch, then adds the entry/tip. A missing containing branch throws invalid_entry('Branch cache has no branch containing parent entry <id>'); append does not rebuild (`ice:packages/storage/sqlite-node/src/sqlite/branch-cache.ts:58-105`).

A query locates the first branch containing start and its entry_seq, constrains all rows to <= that start seq, and chooses an inclusive OR-boundary from stopAtType/stopAtId: MIN boundary seq for oldestFirst, MAX for newestFirst. With no matching boundary it includes the full requested path. Rows are ordered in the requested direction. The repository validates consecutive parent links, and when no stop bounds were supplied the earliest row must have null parent; then it decodes rows and applies entry filters/cursor/limit. A missing start is not_found; existing start without cache is invalid_entry. Parent inconsistency is invalid_entry. Malformed payload before a requested latest-compaction boundary is never decoded by that branch query (`ice:packages/storage/sqlite-node/src/sqlite/storage/branch-entries.ts:26-78`; `ice:packages/storage/sqlite-node/src/sqlite/repo.ts:294-307`, `ice:packages/storage/sqlite-node/src/sqlite/repo.ts:504-528`).

repairBranchCache first releases local active storage for the session, acquires a writer lease in a transaction and checks the session exists. It discovers canonical tips as entries with no children, ordered by seq; deletes tips then memberships; and rebuilds a complete recursive parent path for each tip. Each build has SAVEPOINT build_branch_cache, rollback/release on error, and preserves SessionError or wraps other failures as storage('Failed to build SQLite branch cache at entry <id>'). It releases its claim when done; callers can reopen after repair. Fork builds all resulting tip paths; a branch fork with absent source membership fails invalid_fork_target. Delete removes cache with the session. Tests explicitly cover complete paths after compaction, nested boundaries, corruption outside the window, missing/stale cache, repair, fork failure and deletion (`ice:packages/storage/sqlite-node/test/branch-cache.test.ts:14-219`).

`readBranchTipIds` orders returned tip IDs lexically by tip_id. `updateBranchTip` succeeds only when exactly one row matches sessionId, branchId and oldTipId, then replaces tip_id. These are cache implementation identities rather than canonical lane ordering (`ice:packages/storage/sqlite-node/src/sqlite/storage/branch-tips.ts:3-35`).

### 5. Interfaces

SqliteSessionRepository.repairBranchCache(SqliteSessionMetadata): Promise<void>; private branch_entries(session_id, branch_id, entry_id, entry_seq, entry_type nullable, custom_type nullable), composite membership PK; branch_tips(session_id, tip_id, branch_id) with unique branch id per session. Cache ids are implementation-owned and do not replace canonical entry ids or lane names.

```ts
export interface CachedBranch {
	branchId: string;
	leafSeq: number;
}

export interface CachedBranchEntryRow {
	session_id: string;
	id: string;
	entry_seq: number;
	parent_id: string | null;
	type: Entry["type"];
	timestamp: string;
	payload: string;
}

export interface CachedBranchQuery {
	stopAtType?: Entry["type"];
	stopAtId?: string;
	order?: "newestFirst" | "oldestFirst";
}
```

### 6. Dependencies

`SESSION-001`, `SESSION-006`, `SESSION-007`

### 7. ice-rust today

Rust branch queries use the reducer's parent traversal and have no derived SQLite branch cache or repair API (`rust:crates/ice-harness/src/session/state.rs:328-366`; `rust:crates/ice-harness/src/session/sqlite.rs:124-134`). Direct traversal is an intentional viable design, so absence of the cache alone is not a correctness defect. However the SQLite adapter must deserialize all state before traversal (`rust:crates/ice-harness/src/session/sqlite.rs:66-82`), which lacks TS query-window decode isolation and the explicit derived-index recovery operation. The separate oldestFirst stop-bound mismatch is SESSION-001.

### 8. Port notes

If adding the compatible relational backend, implement full-path cache membership and tip extension/forking in the same write transaction, directional SQL bounds before decoding, and an explicit lease-protected rebuild from canonical parents. Avoid automatic repair in ordinary APIs because it hides corruption and changes tested error behavior. If retaining direct traversal, preserve its simpler canonical architecture and document that cache repair is inapplicable; provide an alternative way to isolate readable branches from unrelated historical payload damage if that recovery contract is required.

### 9. Size and priority

Estimated Rust implementation: **230-500 lines** (estimate, not measured source). **P2** - The cache is an implementation choice, but efficient bounded reads and controlled recovery are exposed capabilities in the TypeScript backend.

## SESSION-009 - Co-located SQLite FTS5 session search and transactional indexing

### 1. Title and status

Co-located SQLite FTS5 session search and transactional indexing. **Status: missing.**

### 2. ice source

`ice:packages/storage/sqlite-node/src/sqlite/search-backend.ts:12-129`; source-read tests `ice:packages/storage/sqlite-node/test/search.test.ts:16-166`.

Exported symbols: `SqliteSessionSearchOptions` (`ice:packages/storage/sqlite-node/src/sqlite/search-backend.ts:26`).

### 3. What it does

SQLite session search indexes canonical entry payloads on demand and returns ranked entry hits filtered by working directory. Index triggers participate in the same transactions as canonical writes/deletes, and blank searches do not initialize storage.

### 4. How it works

The search adapter uses the same canonical database as the repository and lazily initializes FTS5 only for a nonblank query. It searches payload text using a quoted phrase rather than accepting caller-provided FTS syntax.

search trims options.text and returns [] immediately if empty. Otherwise it resolves/caches the absolute database path, creates its parent recursively, opens a fresh connection, applies WAL/synchronous=FULL/busy_timeout=5000 and canonical migrations, then ensures the FTS schema. Setup failure closes that connection and rethrows. A successful search also closes the connection in finally; the search object retains only the path (`ice:packages/storage/sqlite-node/src/sqlite/search-backend.ts:12-24`, `ice:packages/storage/sqlite-node/src/sqlite/search-backend.ts:70-102`, `ice:packages/storage/sqlite-node/src/sqlite/search-backend.ts:121-129`).

session_search_fts has payload as an external-content column linked to entries rowid. Source-read tests identify trigram substring behavior ('uth' matches 'auth'); the exact tokenizer declaration at source line 45 was redacted by retrieval and remains an explicit evidence limitation, not an inferred option list. INSERT/DELETE/UPDATE OF payload triggers maintain the external-content index; delete uses the FTS 'delete' command with old rowid/payload, and update removes old then inserts new. Initial creation detects that the FTS table was absent and issues rebuild to index preexisting entries (`ice:packages/storage/sqlite-node/src/sqlite/search-backend.ts:32-59`; `ice:packages/storage/sqlite-node/test/search.test.ts:16-35`).

The query is surrounded by double quotes after replacing every embedded quote with two quotes. SQL joins FTS→entries by rowid→sessions by session id, filters MATCH query and optional exact cwd using (? IS NULL OR s.cwd = ?), computes bm25(session_search_fts), and orders score ascending. It returns {metadata: rowToMetadata(row, path), entryId, timestamp, score}; there is no snippet, role field, default result cap or secondary tie-breaker. Unlike the repository's truthy cwd list filter, empty-string cwd is an exact search filter (`ice:packages/storage/sqlite-node/src/sqlite/search-backend.ts:99-123`).

Canonical writes alone and blank searches do not create FTS. Once installed, failing FTS trigger writes fail and roll back canonical append/delete as part of their transaction; they are not best-effort auxiliary writes. Source-read tests drop the virtual table while retaining triggers and expect append rollback, deletion rollback and still-present history, as well as search-before-first-create and proper close on setup failure (`ice:packages/storage/sqlite-node/test/search.test.ts:49-166`).

### 5. Interfaces

createSqliteSessionSearch({env:{absolutePath, createDir}, sqlite, databasePath}): SessionSearch<SqliteSessionMetadata>; search({text:string, cwd?:string}): Promise<SessionSearchHit[]> with metadata, entryId, timestamp, score. This is distinct from the raw-entry scanner in SESSION-004 and from the observability runs/events database.

### 6. Dependencies

`SESSION-005`, `SESSION-006`, `SESSION-007`

### 7. ice-rust today

The fully read Rust search implementation operates over a caller-provided HashMap of metadata and entries, performs substring matching/ranking and produces snippets; it has no database factory, FTS schema/triggers, cwd predicate or canonical-index transaction coupling (`rust:crates/ice-harness/src/session/search.rs:1-267`). The fully read SQLite adapter stores whole state_json snapshots and implements no search API (`rust:crates/ice-harness/src/session/sqlite.rs:1-371`). Its separate substring search remains useful but does not implement this backend.

### 8. Port notes

Add a SQLite search adapter tied to the selected canonical storage schema, initialize external-content FTS and triggers only on nonblank first use, rebuild existing payloads, parameterize the quoted phrase/cwd query and return the exact metadata/timestamp/score shape. Preserve rollback semantics if search triggers fail. Confirm the exact tokenizer declaration from an unredacted authorized source before treating the tokenizer configuration as fully specified; the read tests support trigram behavior but not undisclosed options. If keeping snapshot storage, build a transactional per-entry index and define its migration/import relationship rather than trying to FTS-query opaque state blobs.

### 9. Size and priority

Estimated Rust implementation: **170-350 lines** (estimate, not measured source). **P2** - Indexed cross-session search with cwd isolation is absent; the existing Rust search does not provide the same persistence or indexing contract.

## SESSION-010 - Unicode-consistent compaction token estimates and conversation serialization

### 1. Title and status

Unicode-consistent compaction token estimates and conversation serialization. **Status: partial.**

### 2. ice source

`ice:packages/agent/src/harness/compaction/compaction.ts:165-310`; `ice:packages/agent/src/harness/compaction/utils.ts:19-132`; `ice:packages/coding-agent/src/core/compaction/compaction.ts:155-327`; `ice:packages/coding-agent/src/core/compaction/utils.ts:21-158`; source-read tests `ice:packages/agent/test/harness/compaction.test.ts:143-360` and `ice:packages/agent/test/harness/compaction.test.ts:441-456`.

Exported symbols: `CompactionDetails` (`ice:packages/agent/src/harness/compaction/compaction.ts:30`).

Exported symbols: `FileOperations` (`ice:packages/agent/src/harness/compaction/utils.ts:5`), `extractFileOpsFromMessage` (`ice:packages/agent/src/harness/compaction/utils.ts:24`).

Exported symbols: `FileOperations` (`ice:packages/coding-agent/src/core/compaction/utils.ts:12`), `extractFileOpsFromMessage` (`ice:packages/coding-agent/src/core/compaction/utils.ts:29`), `SUMMARIZATION_SYSTEM_PROMPT` (`ice:packages/coding-agent/src/core/compaction/utils.ts:156`).

### 3. What it does

Token estimates decide where history is cut and when compaction runs. The transcript sent to the summarizer preserves role distinctions, thinking, tool calls and bounded tool output, with cumulative read/modified file lists.

### 4. How it works

Both TypeScript implementations estimate roughly one token per four JavaScript UTF-16 code units, with a fixed image allowance. They prefer valid provider usage and only estimate content after the latest usable assistant response; serialization is a separate bounded textual representation and does not omit tool output from the retained session.

calculateContextTokens uses usage.totalTokens when truthy, otherwise input+output+cacheRead+cacheWrite. A usable assistant has stopReason other than error/aborted and positive calculated context tokens. estimateContextTokens scans backward for the latest usable usage, adds estimates of every trailing message, and reports contextTokens, usageTokens, trailingTokens and lastUsageIndex; if none exists it estimates every message and reports usageTokens=0, lastUsageIndex=null (`ice:packages/agent/src/harness/compaction/compaction.ts:165-244`).

Estimate character contributions are user/custom/toolResult text plus 4800 per image, assistant text/thinking plus tool name and serialized arguments, bash command+output, and branch/compaction summary text. The final sum is ceil(chars/4); unknown roles contribute zero. The harness uses safe JSON serialization (fallback undefined or '[unserializable]'), while coding-agent JSON.stringify can throw for circular arguments. Count UTF-16 code units, not UTF-8 bytes or Unicode scalar values (`ice:packages/agent/src/harness/compaction/compaction.ts:252-310`; `ice:packages/coding-agent/src/core/compaction/compaction.ts:280-327`).

After convertToLlm, serializeConversation emits [User], [Assistant thinking], [Assistant], [Assistant tool calls], and [Tool result] sections separated by two newlines. Assistant thinking precedes visible text and calls; call arguments render key=value pairs, calls use '; ' separators. User/tool-result text blocks concatenate without separators. Tool-result text over 2000 UTF-16 units is sliced at 2000 and followed by a marker with the omitted unit count. The Rust variant must not create a zero-truncated marker just because UTF-8 encoding exceeds 2000 bytes (`ice:packages/agent/src/harness/compaction/utils.ts:91-132`; `ice:packages/coding-agent/src/core/compaction/utils.ts:97-158`).

File tracking inspects assistant tool calls named read/write/edit with a nonempty string arguments.path, accumulates read/written/edited sets, reports readFiles = read minus written/edited and modifiedFiles = written union edited, sorts both, and appends only nonempty <read-files>/<modified-files> blocks with two leading newlines. Retained-tail file operations are not newly attributed to the summarized span (`ice:packages/agent/src/harness/compaction/utils.ts:19-71`; `ice:packages/agent/src/harness/compaction/compaction.ts:616-687`).

### 5. Interfaces

calculateContextTokens(Usage):number; getLastAssistantUsage; estimateContextTokens(messages):{contextTokens, usageTokens, trailingTokens, lastUsageIndex}; estimateTokens(AgentMessage):number; createFileOps/extractFileOps/computeFileLists/formatFileOperations; serializeConversation(Message[]):string. Preserve the coding-agent versus harness JSON error behavior explicitly.

`FileOperations` is exactly `{ read: Set<string>; written: Set<string>; edited: Set<string> }` in both implementations. `CompactionDetails` stores `{ readFiles: string[]; modifiedFiles: string[] }`; these lists are derived by the accumulator behavior above.

### 6. Dependencies

`AGENT-063`, `AGENT-064`, `AGENT-065`

### 7. ice-rust today

Rust uses .len() UTF-8 byte counts for text and tool arguments, so 400 basic Chinese characters estimate 300 tokens instead of TypeScript's 100. Unknown custom roles fall through to text/image estimates rather than zero. Its truncation predicate tests bytes but retained text/omission count use .chars(): e.g. 1000 Chinese characters exceed the byte threshold but fit the 2000-scalar take, so a zero-omitted marker is appended. Empty file paths are accepted. The ASCII behavior, fixed image cost, sorted file sets, usage-plus-trailing strategy and role labels otherwise exist (`rust:crates/ice-harness/src/compaction/tokens.rs:44-135`; `rust:crates/ice-harness/src/compaction/tokens.rs:215-285`; full file source-read including tests).

### 8. Port notes

Introduce one explicit UTF-16-length/slicing compatibility helper and use it consistently for estimates and the 2000-unit serialization cutoff. Keep byte-safe Rust slicing, define behavior at a surrogate boundary without panicking, and document if exact JavaScript lone-surrogate output is intentionally normalized. Ignore empty file paths and unknown roles under the compatibility projection. Add focused source-derived cases for multibyte text, astral characters, no usable usage and nonempty trailing messages; do not substitute provider tokenization for the specified heuristic.

### 9. Size and priority

Estimated Rust implementation: **100-230 lines** (estimate, not measured source). **P1** - Incorrect estimates change compaction timing and retained history for non-ASCII conversations; the output truncation mismatch also changes the summarization prompt.

## SESSION-011 - Harness summarization routing, retry callbacks and exact result semantics

### 1. Title and status

Harness summarization routing, retry callbacks and exact result semantics. **Status: partial.**

### 2. ice source

`ice:packages/agent/src/harness/compaction/compaction.ts:102-121`; `ice:packages/agent/src/harness/compaction/compaction.ts:424-593`; `ice:packages/agent/src/harness/compaction/compaction.ts:707-848`; source-read tests `ice:packages/agent/test/harness/compaction.test.ts:458-692`.

Named export: `CompactionErrorCode` (`ice:packages/agent/src/harness/types.ts:179`).

`ice:packages/agent/src/harness/types.ts:178-191` (`CompactionErrorCode`, `CompactionError`).

Exported symbols: `DEFAULT_COMPACTION_SETTINGS` (`ice:packages/agent/src/harness/compaction/compaction.ts:158`), `findTurnStartIndex` (`ice:packages/agent/src/harness/compaction/compaction.ts:347`), `CutPointResult` (`ice:packages/agent/src/harness/compaction/compaction.ts:364`), `findCutPoint` (`ice:packages/agent/src/harness/compaction/compaction.ts:374`), `SUMMARIZATION_SYSTEM_PROMPT` (`ice:packages/agent/src/harness/compaction/compaction.ts:424`), `prepareCompaction` (`ice:packages/agent/src/harness/compaction/compaction.ts:616`).

### 3. What it does

Reusable harness compaction produces a structured summary, optionally combines a complete-history summary with a split-turn prefix, and reports usage and typed compaction errors. It accepts injected model/runtime routing and exposes retry progress to its caller.

### 4. How it works

The harness materializes retained messages into its checkpoint rather than referencing the old JSONL tree boundary. Summary calls are serial, use a fresh routing session ID per distinct call, and disable prompt-cache retention while retaining the same ID across retries of that call.

completeSimpleWithRetries receives Models and Model, spreads caller options, then forces cacheRetention:'none' and a fresh uuidv7 sessionId before invoking the shared retry helper around Models.completeSimple. Retry callbacks are caller-supplied scheduled/attempt-start/finished hooks. This Model/Models contract includes runtime authentication/routing and is not equivalent to requiring a caller-supplied raw API key (`ice:packages/agent/src/harness/compaction/compaction.ts:102-121`).

History summarization uses the fixed summarizer system instruction and a single current-timestamp user message containing serialized <conversation> content, optionally the truthy <previous-summary>, and the fixed new-summary or update-summary template. The template preserves Goal, Constraints & Preferences, Progress (Done/In Progress/Blocked), Key Decisions, Next Steps and Critical Context; it forbids continuing the underlying conversation. Truthy customInstructions append an Additional focus section. Output budget is min(floor(0.8*reserveTokens), model.maxTokens when >0); prefix summarization uses floor(0.5*reserveTokens) with the same positive model cap and its own split-turn prompt, without history/custom focus. Reasoning is forwarded only for a reasoning-capable model and a non-off requested level (`ice:packages/agent/src/harness/compaction/compaction.ts:424-593`; `ice:packages/agent/src/harness/compaction/compaction.ts:795-848`).

An aborted response returns Result error code aborted using nonempty errorMessage or 'Summarization aborted'; error stop returns summarization_failed and 'Summarization failed: ' plus nonempty message or 'Unknown error'. Other stop reasons, tool calls and empty text are accepted by this generic harness helper; coding-agent's additional validation is SESSION-014. generateSummary returns text only, generateSummaryWithUsage returns {text, usage} (`ice:packages/agent/src/harness/compaction/compaction.ts:502-593`).

compact summarizes complete history first, then the prefix for a split turn. If complete history is empty it uses 'No prior history.' and makes only the prefix call. It stops on the first Result failure. Summaries join with '\n\n---\n\n**Turn Context (split turn):**\n\n'; usage sums every core token/cost field and carries optional cacheWrite1h/reasoning fields when either side supplies them. It appends cumulative file-operation tags and returns summary, retainedTail, tokensBefore, details and optional usage (`ice:packages/agent/src/harness/compaction/compaction.ts:707-794`).

The generic harness additionally exports `DEFAULT_COMPACTION_SETTINGS = { enabled: true, reserveTokens: 16384, keepRecentTokens: 20000 }`. `findTurnStartIndex` scans backward to a branch-summary entry or a message with role user/bashExecution, returning -1 if none. `findCutPoint` takes [startIndex, endIndex), excludes tool-result cuts, accumulates only message estimates from newest backward, and chooses the first valid cut at or after the crossing. With no such later cut its initialized first cut remains. It backs over metadata until a message or compaction, then marks a split when the cut is not a user message and a turn start exists (`ice:packages/agent/src/harness/compaction/compaction.ts:147-162`; `ice:packages/agent/src/harness/compaction/compaction.ts:315-422`).

`prepareCompaction` returns `ok(undefined)` for an empty path or final compaction. It finds the latest compaction, materializes its retainedTail as synthetic message entries named `<compaction-id>:retained:<index>` with the checkpoint sequence and original message timestamps, then appends later entries. The selected cut divides full summary history, an optional split-turn prefix and retainedTail; file operations cover summarized history plus prefix and prior checkpoint details. It returns `Result<CompactionPreparation | undefined, CompactionError>`. The exact tokensBefore callee at source line 649 was connector-redacted and is excluded from this supplementary source verification (`ice:packages/agent/src/harness/compaction/compaction.ts:616-648`; `ice:packages/agent/src/harness/compaction/compaction.ts:650-687`).

### 5. Interfaces

CompactionLlmOptions contains Models, Model, optional thinkingLevel, signal, customInstructions, retry policy and callbacks; CompactionResult/Result<CompactionError> code aborted|summarization_failed. completeSimpleWithRetries and generateSummary[WithUsage]/generateTurnPrefixSummary are reusable public helpers. Generic helper result permissiveness is deliberate source scope, not evidence that coding-agent accepts invalid summaries.

`CompactionErrorCode = "aborted" | "summarization_failed"`. `CompactionError(code, message, cause?: Error)` extends `Error`, sets `name = "CompactionError"` and public `code`, and passes `{ cause }` to the base constructor only when `cause !== undefined`; the cause is not stringified.

```ts
export interface CutPointResult {
	/** Index of the first entry retained after compaction. */
	firstKeptEntryIndex: number;
	/** Index of the turn-start entry when the cut splits a turn, otherwise -1. */
	turnStartIndex: number;
	/** Whether the selected cut point splits an in-progress turn. */
	isSplitTurn: boolean;
}
```

### 6. Dependencies

`AI-001`, `SESSION-010`

### 7. ice-rust today

Rust has the fixed prompts, serial split-turn calls, 0.8/0.5 budgets, UUID/cache settings, cumulative file tags and usage aggregation. However CompactionLlmOptions stores Arc<dyn Provider>, Model, optional api_key and CancellationToken with no retry progress callbacks; stream_simple+result synthesizes an error for a missing terminal event. Some('') previous_summary/custom_instructions is treated as present, unlike TypeScript truthiness, and unwrap_or preserves empty error strings where TypeScript uses fallbacks. Negative reserve is clamped before unsigned conversion (`rust:crates/ice-harness/src/compaction/compaction.rs:149-161`; `rust:crates/ice-harness/src/compaction/compaction.rs:394-495`; `rust:crates/ice-harness/src/compaction/compaction.rs:578-645`; `rust:crates/ice-harness/src/compaction/compaction.rs:743-805`).

### 8. Port notes

Add retry lifecycle callbacks and a routing adapter able to preserve the Models/runtime abstraction without losing configured headers/environment/base URL. Keep new UUID generation outside each retry loop and independent between history and prefix calls. Match empty-string prompt/error selection and expose typed Result errors plus text-only/usage helpers. Keep generic harness permissiveness distinct from the coding-agent validated wrapper. Delegate shared retry classification/backoff implementation to AI-001; this entry owns call-site integration and callback delivery.

### 9. Size and priority

Estimated Rust implementation: **150-330 lines** (estimate, not measured source). **P1** - Compaction must preserve custom model routing and observable retry lifecycle; otherwise SDK/provider integrations behave differently even when the prompt text is ported.

## SESSION-012 - Branch-summary collection, token-budget grace and option edge cases

### 1. Title and status

Branch-summary collection, token-budget grace and option edge cases. **Status: partial.**

### 2. ice source

`ice:packages/agent/src/harness/compaction/branch-summarization.ts:81-280`; `ice:packages/coding-agent/src/core/compaction/branch-summarization.ts:108-376`.

Named export: `BranchSummaryErrorCode` (`ice:packages/agent/src/harness/types.ts:194`).

Named export: `BranchSummaryError` (`ice:packages/agent/src/harness/types.ts:197`).

`ice:packages/agent/src/harness/types.ts:193-206` (`BranchSummaryErrorCode`, `BranchSummaryError`).

Exported symbols: `BranchSummaryResult` (`ice:packages/agent/src/harness/compaction/branch-summarization.ts:26`), `BranchSummaryDetails` (`ice:packages/agent/src/harness/compaction/branch-summarization.ts:34`), `BranchPreparation` (`ice:packages/agent/src/harness/compaction/branch-summarization.ts:44`), `CollectEntriesResult` (`ice:packages/agent/src/harness/compaction/branch-summarization.ts:54`), `generateBranchSummary` (`ice:packages/agent/src/harness/compaction/branch-summarization.ts:208`).

Exported symbols: `BranchSummaryResult` (`ice:packages/coding-agent/src/core/compaction/branch-summarization.ts:34`), `BranchSummaryDetails` (`ice:packages/coding-agent/src/core/compaction/branch-summarization.ts:44`), `BranchPreparation` (`ice:packages/coding-agent/src/core/compaction/branch-summarization.ts:51`), `CollectEntriesResult` (`ice:packages/coding-agent/src/core/compaction/branch-summarization.ts:60`), `generateBranchSummary` (`ice:packages/coding-agent/src/core/compaction/branch-summarization.ts:293`).

### 3. What it does

Branch summarization collects only the abandoned path below the deepest common ancestor and prefers its newest context within an input budget. The generic harness and coding-agent helper expose different error and extension-history contracts.

### 4. How it works

Collection is separate from navigation and persistence. It finds the deepest shared ancestor, reverses the abandoned branch into chronological order, then selects newest useful messages without including tool-result entries as independent branch-summary input.

The harness treats a falsy old leaf as empty, computes old/target newest-first paths and finds the first shared target ancestor, then walks old leaf up to but excluding that ancestor; a missing traversed entry returns SessionError invalid_entry with 'Entry <id> not found'. Coding-agent traverses root-first SessionManager branches, scans target from the end, and breaks on a missing entry instead of erroring. Both return old-branch entries and commonAncestorId (`ice:packages/agent/src/harness/compaction/branch-summarization.ts:81-110`; `ice:packages/coding-agent/src/core/compaction/branch-summarization.ts:108-145`).

Preparation seeds file sets from prior branch-summary details. Coding-agent excludes fromHook summaries, whereas the generic harness has no such exclusion. It walks newest backward, skips toolResult messages and metadata, projects custom-message/branch/compaction summaries, and extracts file operations before checking the budget, so the first over-budget message may contribute file attribution even when its text is omitted. A positive budget is enforced; zero/negative means unlimited. If the next oversized message is a compaction or branch summary and accumulated tokens are below 90% of budget, it includes that summary then stops. The selected messages are returned oldest-first (`ice:packages/agent/src/harness/compaction/branch-summarization.ts:132-170`; `ice:packages/coding-agent/src/core/compaction/branch-summarization.ts:156-247`).

Default reserve is 16384; input budget = (model.contextWindow || 128000)-reserve. No content returns 'No content to summarize' without an LLM call (harness also returns empty read/modified arrays; coding-agent omits them). Replace the default summary instruction only when replaceInstructions && customInstructions is truthy; otherwise truthy custom instructions append focus. Send one serialized-conversation user message, fixed 2048 maxTokens, no requested thinking level, cacheRetention none and fresh routing UUID via the summary wrapper. A successful result prefixes the fixed abandoned-branch preamble and appends file tags, returning usage. Harness returns typed aborted/summarization_failed Result errors; coding-agent returns {aborted:true} or {error:...}, and native completion validation comes from SESSION-014 (`ice:packages/agent/src/harness/compaction/branch-summarization.ts:208-280`; `ice:packages/coding-agent/src/core/compaction/branch-summarization.ts:293-376`).

### 5. Interfaces

collectEntriesForBranchSummary(session, oldLeafId, targetId); prepareBranchEntries(entries, tokenBudget); GenerateBranchSummaryOptions (model/runtime or coding-agent auth/headers/env/streamFn, signal, reserveTokens, customInstructions, replaceInstructions, retry/callbacks); result contains summary, usage and file lists or the contract-specific failure representation.

`BranchSummaryErrorCode = "aborted" | "summarization_failed"`. `BranchSummaryError(code, message, cause?: Error)` extends `Error`, sets `name = "BranchSummaryError"` and public `code`, and preserves an optional `Error` cause through the base constructor. These are the generic harness helper's returned errors, separate from the coding-agent helper's thrown-error interface.

Harness result and preparation declarations:
```ts
export interface BranchSummaryResult {
	summary: string;
	usage?: Usage;
	readFiles: string[];
	modifiedFiles: string[];
}

/** File-operation details stored on generated branch summary entries. */
export interface BranchSummaryDetails {
	/** Files read while exploring the summarized branch. */
	readFiles: string[];
	/** Files modified while exploring the summarized branch. */
	modifiedFiles: string[];
}

export type { FileOperations } from "./utils.ts";

/** Prepared branch content for summarization. */
export interface BranchPreparation {
	/** Messages selected for the branch summary. */
	messages: AgentMessage[];
	/** File operations extracted from the branch. */
	fileOps: FileOperations;
	/** Estimated token count for selected messages. */
	totalTokens: number;
}

/** Entries selected for branch summarization. */
export interface CollectEntriesResult {
	/** Entries to summarize in chronological order. */
	entries: Entry[];
	/** Deepest common ancestor between the previous leaf and target entry. */
	commonAncestorId: string | null;
}
```
Coding-agent variants retain optional result/error fields:
```ts
export interface BranchSummaryResult {
	summary?: string;
	usage?: Usage;
	readFiles?: string[];
	modifiedFiles?: string[];
	aborted?: boolean;
	error?: string;
}

/** Details stored in BranchSummaryEntry.details for file tracking */
export interface BranchSummaryDetails {
	readFiles: string[];
	modifiedFiles: string[];
}

export type { FileOperations } from "./utils.ts";

export interface BranchPreparation {
	/** Messages extracted for summarization, in chronological order */
	messages: AgentMessage[];
	/** File operations extracted from tool calls */
	fileOps: FileOperations;
	/** Total estimated tokens in messages */
	totalTokens: number;
}

export interface CollectEntriesResult {
	/** Entries to summarize, in chronological order */
	entries: SessionEntry[];
	/** Common ancestor between old and new position, if any */
	commonAncestorId: string | null;
}
```

### 6. Dependencies

`AI-001`, `SESSION-010`, `SESSION-011`

### 7. ice-rust today

The reusable Rust branch_summary module implements ancestor collection, newest-first bounded preparation, preamble, default reserve and 2048-token summary calls. Its grace threshold floors 0.9*budget to i64: budget=3 and current=2 fails Rust's 2<2 while TypeScript's 2<2.7 includes a checkpoint. Some('') triggers empty replacement/focus behavior; Some('') old leaf is looked up rather than treated as empty. It forwards reasoning from the shared LLM options although the TypeScript branch helper has no thinking option. GenerateBranchSummaryOptions::default() unconditionally panics via unreachable!, while new(llm) supplies usable defaults (`rust:crates/ice-harness/src/compaction/branch_summary.rs:88-99`; `rust:crates/ice-harness/src/compaction/branch_summary.rs:137-272`; `rust:crates/ice-harness/src/compaction/branch_summary.rs:288-397`). This helper's existence alone does not prove navigation invokes it.

### 8. Port notes

Match the fractional 90% comparison, empty-string semantics and branch-specific options; expose constructors that cannot unexpectedly panic through Default (remove the impossible Default implementation or require a valid routing context). Keep source-specific collection failures and fromHook attribution explicit in the coding-agent adapter. Wire actual navigation to this helper separately, with one owned cancellation signal and hook-provided summary bypass; do not mistake the standalone helper for completed product integration.

### 9. Size and priority

Estimated Rust implementation: **90-220 lines** (estimate, not measured source). **P2** - Most reusable behavior exists, but boundary cases and an unconditional public Default panic undermine reliable branch-summary use.

## SESSION-013 - Coding-agent compaction thresholds and reference-based retained context

### 1. Title and status

Coding-agent compaction thresholds and reference-based retained context. **Status: partial.**

### 2. ice source

`ice:packages/coding-agent/src/core/compaction/compaction.ts:126-152`; `ice:packages/coding-agent/src/core/compaction/compaction.ts:251-279`; `ice:packages/coding-agent/src/core/compaction/compaction.ts:358-451`; `ice:packages/coding-agent/src/core/compaction/compaction.ts:761-970`; `ice:packages/coding-agent/src/core/session-manager.ts:427-478`; docs `ice:packages/coding-agent/docs/compaction.md:41-57` and `ice:packages/coding-agent/docs/compaction.md:133-211`.

Exported symbols: `CompactionDetails` (`ice:packages/coding-agent/src/core/compaction/compaction.ts:34`), `DEFAULT_COMPACTION_THRESHOLD_PERCENT` (`ice:packages/coding-agent/src/core/compaction/compaction.ts:136`), `MID_RUN_SAFETY_NET_PERCENT` (`ice:packages/coding-agent/src/core/compaction/compaction.ts:144`), `DEFAULT_COMPACTION_SETTINGS` (`ice:packages/coding-agent/src/core/compaction/compaction.ts:146`), `isPastMidRunSafetyNet` (`ice:packages/coding-agent/src/core/compaction/compaction.ts:257`), `resolveThresholdTokens` (`ice:packages/coding-agent/src/core/compaction/compaction.ts:262`), `modelAwareReserveTokens` (`ice:packages/coding-agent/src/core/compaction/compaction.ts:271`), `findTurnStartIndex` (`ice:packages/coding-agent/src/core/compaction/compaction.ts:404`), `CutPointResult` (`ice:packages/coding-agent/src/core/compaction/compaction.ts:413`), `findCutPoint` (`ice:packages/coding-agent/src/core/compaction/compaction.ts:438`), `prepareCompaction` (`ice:packages/coding-agent/src/core/compaction/compaction.ts:761`).

### 3. What it does

The coding-agent compaction policy triggers at a percentage of the model context window and records a firstKeptEntryId into the JSONL tree. This is a distinct live contract from the generic harness's reserve-based threshold and materialized retainedTail checkpoint.

### 4. How it works

Coding-agent defaults are enabled=true, thresholdPercent=85, reserveTokens=16384, keepRecentTokens=20000 and midRunCompaction='off'. reserveTokens budgets the summary response; it does not set the automatic trigger.

Normalize thresholdPercent to a finite positive number clamped to 1..99, otherwise use 85. shouldCompact is false when disabled or contextWindow<=0, and otherwise requires contextTokens > floor(window*percent/100), strictly greater. The mid-run safety helper requires window>0 and tokens > floor(window*95/100). Unless reserveTokens was explicitly configured, AgentSession substitutes min(model.maxTokens, floor(model.contextWindow/2)) (`ice:packages/coding-agent/src/core/compaction/compaction.ts:126-152`; `ice:packages/coding-agent/src/core/compaction/compaction.ts:251-279`; `ice:packages/coding-agent/src/core/agent-session.ts:2167-2182`).

The cut finder counts all context-visible projected entries, not just message entries, and accepts visible user/assistant/bash/custom/summary boundaries except compaction entries and tool results. A turn may begin at user, bashExecution, custom, branchSummary or compactionSummary. It scans newest backward until keepRecentTokens is reached and chooses the next valid cut; when none follows the crossing, it uses the last valid cut rather than the generic harness's first-cut fallback. It backs over adjacent metadata without crossing a compaction boundary (`ice:packages/coding-agent/src/core/compaction/compaction.ts:358-451`).

Preparation returns undefined for empty input or a final compaction. After a previous compaction it searches the full selected branch for its firstKeptEntryId and begins there; if missing, it begins after the previous compaction. tokensBefore is recalculated from the rebuilt context. Summary history ends at the new cut or split-turn start; the prefix is the split-turn interval. It refuses a missing kept entry ID or two empty summary spans. Prior file-operation details seed tracking only when not fromHook. Native compact returns summary, firstKeptEntryId, tokensBefore, details and optional usage; an absent final ID throws 'First kept entry has no UUID - session may need migration' (`ice:packages/coding-agent/src/core/compaction/compaction.ts:761-840`; `ice:packages/coding-agent/src/core/compaction/compaction.ts:868-970`).

Session context follows the selected leaf to root, resolves model/thinking from that entire path, and uses the latest compaction entry followed by pre-compaction entries starting at firstKeptEntryId and all post-compaction entries. This particular coding-agent implementation does not consume retainedTail, despite broader session-format docs describing newer harness checkpoints. The generic harness's retainedTail implementation remains separately valid (`ice:packages/coding-agent/src/core/session-manager.ts:343-478`; compare `ice:packages/coding-agent/docs/session-format.md:237-248` and `ice:packages/coding-agent/docs/session-format.md:320-342`).

### 5. Interfaces

Coding CompactionSettings extends enabled/reserveTokens/keepRecentTokens with thresholdPercent? and midRunCompaction off|pause|resume; CompactionPreparation contains firstKeptEntryId, boundary/cut/split-turn messages, tokensBefore, previousSummary, fileOps/settings. CompactionResult uses firstKeptEntryId, not retainedTail. Manual/automatic orchestration is specified separately.

### 6. Dependencies

`SESSION-010`

### 7. ice-rust today

The Rust reusable settings/cut/preparation implementation follows the generic harness: enabled/reserve/keepRecent fields, tokens > contextWindow-reserveTokens, message-entry token counting and materialized retainedTail. Its preparation deliberately returns Some even for a single retained user message with no summary history; tests assert that behavior. No coding-agent thresholdPercent or midRunCompaction fields appear in the fully read settings/preparation code (`rust:crates/ice-harness/src/compaction/settings.rs:1-83`; `rust:crates/ice-harness/src/compaction/cut_point.rs:1-174`; `rust:crates/ice-harness/src/compaction/compaction.rs:255-384`; `rust:crates/ice-harness/src/compaction/compaction.rs:861-875`). This is partial product parity, not a claim that Rust lacks compaction.

### 8. Port notes

Add a coding-agent policy adapter with exact percentage normalization and model-aware response reserve, while preserving the reusable harness's independently specified policy. Decide explicitly whether native storage remains retainedTail-based; if so, reproduce the same selected retained context and import/export the coding-agent firstKept boundary losslessly. Implement context-visible cut selection and empty-span rejection in the product adapter. Correct documentation that conflates the two live TypeScript implementations instead of silently porting a hybrid.

### 9. Size and priority

Estimated Rust implementation: **200-420 lines** (estimate, not measured source). **P1** - The current threshold and boundary policy changes when automatic compaction happens and what work survives repeated compactions.

## SESSION-014 - Coding-agent summary completion validation and injected stream contract

### 1. Title and status

Coding-agent summary completion validation and injected stream contract. **Status: missing.**

### 2. ice source

`ice:packages/coding-agent/src/core/compaction/compaction.ts:572-737`; `ice:packages/coding-agent/src/core/compaction/compaction.ts:975-1023`; `ice:packages/coding-agent/src/core/compaction/branch-summarization.ts:293-376`; docs `ice:packages/coding-agent/docs/compaction.md:25-37`.

### 3. What it does

Native coding-agent summaries are accepted only as completed, nonblank text with no tool calls. Custom SDK stream functions and resolved authentication options pass through the same validator so incomplete output cannot replace working context.

### 4. How it works

The wrapper first retries provider failures under the configured policy, then validates the final response. Validation failures do not enter another retry loop, and extension-provided summaries bypass native generation entirely.

createSummarizationOptions carries apiKey, headers, env, signal and maxTokens, adding reasoning only when the model supports it and the requested level is not off. completeSummarization clones the context with tools:undefined and clones options with toolChoice:undefined, cacheRetention:'none' and a fresh uuidv7 sessionId. It does not mutate caller objects. With streamFn it awaits that stream's result; otherwise it calls the compatibility completeSimple helper. The retry wrapper receives policy, signal and scheduled/attempt-start/finished callbacks (`ice:packages/coding-agent/src/core/compaction/compaction.ts:572-610`).

After retries, an aborted signal or response.stopReason='aborted' normalizes the response to aborted/'Summarization aborted', even if the provider returned success after cancellation. An existing error response passes through. Otherwise validate in this exact order: stopReason must equal 'stop' or return error 'Incomplete summarization (stop reason: <reason>)'; any toolCall block returns error 'Summarization returned tool calls'; no nonblank concatenated text returns error 'Summarization returned no text'. The invalid response retains its other fields/content/usage but stopReason becomes error (`ice:packages/coding-agent/src/core/compaction/compaction.ts:595-629`).

History summary callers throw DOMException('Summarization aborted','AbortError') on aborted and Error('Summarization failed: <message>') on error. Prefix callers use 'Turn prefix summarization aborted' for abort. Branch helpers return {aborted:true} or {error:errorMessage || 'Summarization failed'}. A successful helper returns the original text and usage; it does not trim successful text, only uses trim to test emptiness. Fresh routing IDs and no-cache settings are per distinct summary request (`ice:packages/coding-agent/src/core/compaction/compaction.ts:669-737`; `ice:packages/coding-agent/src/core/compaction/compaction.ts:975-1023`; `ice:packages/coding-agent/src/core/compaction/branch-summarization.ts:335-376`).

### 5. Interfaces

completeSummarization(model, context, options, streamFn?, retryPolicy?, retryCallbacks?):Promise<AssistantMessage>; coding helpers accept apiKey/headers/env and the session's Agent stream function. Error shape differs between throw-based compaction and value-based branch summary and must remain observable.

### 6. Dependencies

`AI-001`, `SESSION-011`, `SESSION-012`

### 7. ice-rust today

The Rust summary helper builds one user-message context, calls Provider::stream_simple and checks only aborted/error stops before extracting text. It does not reject length/toolUse/nonblank failures or tool-call blocks, has no custom Agent stream function/headers/env in CompactionLlmOptions, and does not recheck CancellationToken after an otherwise successful final response. Therefore the generic Rust helper can produce an empty/truncated/tool-using summary that the coding-agent wrapper rejects (`rust:crates/ice-harness/src/compaction/compaction.rs:149-161`; `rust:crates/ice-harness/src/compaction/compaction.rs:394-430`; `rust:crates/ice-harness/src/compaction/compaction.rs:578-645`; `rust:crates/ice-harness/src/compaction/branch_summary.rs:288-385`).

### 8. Port notes

Implement a coding-agent completion wrapper around the configured stream/routing abstraction. Clear tool definitions and tool selection on cloned request values, apply shared transient retry first, then execute the ordered post-response cancellation/content validator exactly once. Route history, prefix and branch generation through it and preserve each caller's error representation. Add source-derived validation coverage for length, toolUse, stop+toolCall, blank text, post-response cancellation and caller-object immutability before enabling it in persistence paths.

### 9. Size and priority

Estimated Rust implementation: **120-260 lines** (estimate, not measured source). **P1** - Persisting incomplete or empty summaries can discard useful context; the TypeScript product explicitly guards that boundary.

## SESSION-015 - Automatic compaction decision order, overflow recovery and turn-boundary mode

### 1. Title and status

Automatic compaction decision order, overflow recovery and turn-boundary mode. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/agent-session.ts:1311-1382; ice:packages/coding-agent/src/core/agent-session.ts:2408-2565; ice:packages/coding-agent/test/agent-session-auto-compaction-queue.test.ts:59-449; Additional complete test-source review: ice:packages/coding-agent/test/suite/agent-session-compaction-lock.test.ts:1-386; ice:packages/coding-agent/test/suite/agent-session-compaction.test.ts:1-705; ice:packages/coding-agent/test/suite/regressions/5217-compaction-reason.test.ts:1-96; ice:packages/coding-agent/test/suite/regressions/pre-prompt-compaction-no-continue.test.ts:1-76; ice:packages/coding-agent/test/suite/blackhole-compaction.test.ts:1-578; ice:packages/coding-agent/test/suite/regressions/6647-compaction-retries-transient-stream-drop.test.ts:1-193; ice:packages/coding-agent/test/suite/regressions/9178-tree-during-compaction.test.ts:1-188

### 3. What it does

Automatically compact at the correct boundary without repeating an already answered prompt or repeatedly retrying an unrecoverable overflow. Distinguish proactive threshold compaction, provider overflow and the mid-run pause/resume policy.

### 4. How it works

The coding AgentSession evaluates automatic compaction before an idle prompt and during run settlement, and can compact between tool turns. These paths use different continuation rules; a single call to the generic compaction function does not supply that behavior.

_runAgentPrompt owns the active-run lifetime and loops agent.continue only when post-run actions return true. Post-run order is last-assistant capture/reset, manual-compaction exclusion, eligible assistant retry, failed-retry accounting, _checkCompaction, successful mid-run resume, auto-compaction cancellation, then actual agent queues. The pre-prompt path calls _checkCompaction(lastAssistant, false) before adding the new user prompt and deliberately ignores its continuation decision (ice:packages/coding-agent/src/core/agent-session.ts:1311-1382; ice:packages/coding-agent/src/core/agent-session.ts:1384-1499).

_checkCompaction first rejects disabled compaction and, normally, aborted messages; pre-prompt passes false for that aborted guard. Overflow recognition requires the error's provider/model to match the currently selected model. It ignores an assistant message timestamped before the latest compaction entry and distinguishes recoverable length stops. A successful stop that reports overflow is compacted with willRetry=false to avoid repeating an answer. Other overflow gets at most one compact-and-retry: the flag is set before starting, the terminal assistant is removed from live agent state (durable history remains), and a second overflow emits compaction_end with the explicit one-attempt recovery error without another compaction_start. A consumed user message resets that guard. Threshold checking uses actual assistant usage unless the message is an error or has zero usage; then it estimates current messages but requires useful assistant usage after the latest compaction, not merely a retained pre-compaction assistant (ice:packages/coding-agent/src/core/agent-session.ts:800-886; ice:packages/coding-agent/src/core/agent-session.ts:2471-2565). The tests explicitly cover an agent-level custom queue with public pendingMessageCount=0, one overflow attempt, stale usage and the post-compaction usage requirement (ice:packages/coding-agent/test/agent-session-auto-compaction-queue.test.ts:59-449).

_compactBeforeNextTurn consumes an extension-requested boundary mode first. It otherwise requires completed tool results, enabled settings, a model, a live signal and no current compaction. The configured threshold is 85% by default; with mid-run mode off, only the strict 95% safety ceiling can compel a checkpoint, and that checkpoint pauses. A saved new compaction entry is necessary to report a stop boundary. Explicit resume or configured resume schedules continuation after settlement; pause/off-safety do not. Threshold and safety predicates are strict greater-than, not greater-than-or-equal (ice:packages/coding-agent/src/core/agent-session.ts:2408-2470; ice:packages/coding-agent/src/core/compaction/compaction.ts:135-174).

### 5. Interfaces

Session settings include enabled, thresholdPercent, keepRecentTokens, reserveTokens and mid-run mode; extension requestCompactionAfterTurn controls a single boundary. Public events use reason='threshold'|'overflow' and compaction_end.willRetry. _runAutoCompaction returns a continuation decision; it does not call agent.continue itself. pendingMessageCount is not a substitute for agent.hasQueuedMessages().

### 6. Dependencies

`AGENT-018`, `AGENT-019`, `AI-001`, `SESSION-013`, `SESSION-014`

### 7. ice-rust today

Rust has pre-run and mid-loop compaction, so the capability is partial. It persists the incoming prompt before pre-run compaction, snapshots configuration once, tests the harness reserve-token threshold, and aborts the run on summary failure. Its after_tool_results callback builds synthetic runtime entries, uses thinking_level=None and retry=None, immediately replaces the loop context and silently returns None on preparation, summary or persistence failure. There is no coding-session thresholdPercent/mid-run pause-or-resume branch, one-overflow guard or corresponding post-run overflow recovery in the inspected run implementation (rust:crates/ice-harness/src/agent_harness.rs:3547-3785; rust:crates/ice-harness/src/agent_harness.rs:3860-3970; rust:crates/ice-harness/src/agent_harness.rs:4012-4279). This does not negate Rust's durable compaction records and retained-tail support.

### 8. Port notes

Add a product-level decision policy with separate pre-prompt, turn-boundary and post-run entry points. Preserve Rust's durable operation/step records while mapping threshold settings and returning explicit continue/pause/failed/cancelled outcomes. Implement same-model overflow classification, one-attempt reset rules, post-compaction usage eligibility and pre-prompt no-extra-continue ordering. Forward the configured summary retry/thinking policy at all call sites. Verify source cases for successful-stop overflow, stale retained usage, hidden agent queues, mid-run off at 85% and above 95%, and cancellation after a saved checkpoint.

### 9. Size and priority

Estimated Rust implementation: **220-450 lines** (estimate, not measured source). **P1** - Wrong continuation or retry decisions can repeat user-visible work, loop on overflowing context, or silently ignore configured compaction behavior.

## SESSION-016 - Manual compaction ownership, cancellation and lifecycle events

### 1. Title and status

Manual compaction ownership, cancellation and lifecycle events. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/agent-session.ts:2167-2407; ice:packages/coding-agent/src/core/agent-session.ts:1865-1887; Additional complete test-source review: ice:packages/coding-agent/test/suite/agent-session-compaction-lock.test.ts:1-386; ice:packages/coding-agent/test/suite/agent-session-operation-cancellation.test.ts:1-231; ice:packages/coding-agent/test/suite/regressions/9178-tree-during-compaction.test.ts:1-188; ice:packages/coding-agent/test/suite/regressions/9340-compaction-auth-cancellation.test.ts:1-132

### 3. What it does

Make explicit compaction an awaited, cancellable session operation whose lock, lifecycle events, extension hooks and queue resumption remain coherent across overlapping requests.

### 4. How it works

Manual compaction claims a session-wide lock synchronously before asynchronous work and exposes a settlement promise. An overlapping manual request cancels the current compaction, awaits settlement, rechecks its caller signal and then claims ownership; branch summarization is rejected rather than overlapped (ice:packages/coding-agent/src/core/agent-session.ts:2167-2239).

After linking an optional caller AbortSignal to its owned controller, the operation aborts assistant retry and the agent and waits for the active run to settle. It emits compaction_start(reason='manual') before validating model/auth or preparation. Missing model/auth, an already compacted leaf and nothing to compact are explicit failures. The session_before_compact hook receives preparation, branch entries, custom instructions and signal; cancellation is checked after it, and a cancelled hook becomes a cancellation error. An extension may return the full compaction result; otherwise the native summarizer uses resolved auth/model routing, the injected session stream, current thinking and shared retry callbacks (ice:packages/coding-agent/src/core/agent-session.ts:2196-2310).

Cancellation is checked again before appending. Success appends the compaction, rebuilds live context, estimates tokensAfter, awaits session_compact, emits compaction_end with result and returns. Unlike automatic compaction, it does not recheck cancellation after the post-save hook: a signal aborted there can leave the saved result and a non-aborted end event, while suppressing queued-input startup. The source locates the saved entry by the first matching compaction summary string, so duplicate summary text can select an older entry; this is an observed source quirk, not a recommended Rust lookup. Catch can emit compaction_end even if cancellation occurred while waiting for idle before compaction_start. It classifies a cancelled controller, 'Compaction cancelled' or AbortError as aborted; aborted results are rethrown as DOMException AbortError, while other failures carry the explicit manual-compaction error text in the end event. Finally unlinks the caller signal, clears the owned controller, settles the lock and starts queued messages if the controller was not aborted—even after a non-cancellation failure (ice:packages/coding-agent/src/core/agent-session.ts:2311-2407).

Public abort cancels retry and compaction, aborts the agent and awaits idle; runtime replacement's stronger abortAndWait also cancels/awaits branch summarization and compaction settlement. These two entry points must not be collapsed (ice:packages/coding-agent/src/core/agent-session.ts:1865-1887).

### 5. Interfaces

compact(customInstructions?,{signal?}) returns the saved compaction result; compaction_start/end carry reason='manual', result, aborted, willRetry=false and optional errorMessage. isCompacting covers lock ownership, auto/manual controllers and branch summarization. before/after hooks are awaited. Runtime replacement requires an operation-settlement boundary, not only provider-stream termination. Result includes summary, firstKeptEntryId, tokensBefore, estimatedTokensAfter, usage?, details?. Explicit preparation errors are 'Already compacted' and 'Nothing to compact (session too small)'; overlapping branch summarization throws 'Wait for branch summarization to finish before compacting.'

### 6. Dependencies

`AI-001`, `EXT-022`, `SESSION-014`, `SESSION-015`

### 7. ice-rust today

Rust acquire_run rejects any active operation with LaneBusy; compact_core neither cancels/awaits an existing manual compaction nor exposes a caller AbortSignal. It records an operation and can return Declined, Completed or Failed. CompactionProgressGuard wraps only the LLM call, after preparation; it does not implement the TS start-before-validation and result/aborted/error payload contract. The product wrapper's compact discards the CompactionResult outcome and returns Ok(()) when the harness call itself succeeded, including a Failed outcome (rust:crates/ice-harness/src/agent_harness.rs:2239-2281; rust:crates/ice-harness/src/agent_harness.rs:4283-4442; rust:crates/ice-cli/src/agent_session.rs:1-240). Rust abort cancels the active signal and returns immediately; wait_for_idle exists separately (rust:crates/ice-harness/src/agent_harness.rs:4636-4666; rust:crates/ice-harness/src/agent_harness.rs:4882-4934). Preserve those useful primitives while implementing the product orchestration.

### 8. Port notes

Introduce owned compaction state plus a completion signal and explicit caller cancellation at the product layer. Map harness outcomes to the public success/error/cancellation API instead of discarding them; expose rich lifecycle events and awaited extension reducers with the exact ordering. Keep durable Rust records and locate the appended entry by its actual generated ID. Separate ordinary abort from replacement teardown and cover overlapping manual compactions, pre-aborted callers, auth cancellation, extension cancellation, non-cancellation failure with queued work and post-hook settlement.

### 9. Size and priority

Estimated Rust implementation: **230-480 lines** (estimate, not measured source). **P1** - Compaction races or a swallowed Failed outcome can report success without a checkpoint and can let session replacement proceed while old work still owns resources.

## SESSION-017 - Automatic compaction commit boundary and extension cancellation semantics

### 1. Title and status

Automatic compaction commit boundary and extension cancellation semantics. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/agent-session.ts:2567-2794; Additional complete test-source review: ice:packages/coding-agent/test/suite/agent-session-compaction-lock.test.ts:1-386; ice:packages/coding-agent/test/suite/agent-session-compaction.test.ts:1-705; ice:packages/coding-agent/test/suite/regressions/6647-compaction-retries-transient-stream-drop.test.ts:1-193; ice:packages/coding-agent/test/suite/regressions/9340-compaction-auth-cancellation.test.ts:1-132

### 3. What it does

Keep automatic compaction events, extension overrides and continuation decisions correct before and after the durable checkpoint is saved.

### 4. How it works

Automatic compaction shares the session lock but returns false immediately when another compaction or branch summary owns it. It performs model/auth lookup and preparation before emitting compaction_start, unlike the manual path; absence of model or preparation produces no start/end pair (ice:packages/coding-agent/src/core/agent-session.ts:2567-2627).

After start it checks cancellation, awaits session_before_compact with reason and overflow flag, and handles extension cancellation by emitting compaction_end(result undefined, aborted=true, willRetry=false). An extension-cancelled hook alone does not set the controller-aborted state used to suppress other queued work. A hook result overrides native summarization; otherwise runtime auth, stream, thinking and retry callbacks are forwarded. Cancellation before append leaves no compaction. Success appends, rebuilds live context and computes tokensAfter before awaiting session_compact. Cancellation during that post-save hook retains the saved checkpoint, marks the end event aborted and prevents automatic continuation. The terminal error/length assistant is removed from live context only for the retry path. Otherwise the method returns whether the actual agent queues contain work (ice:packages/coding-agent/src/core/agent-session.ts:2628-2752).

Catch emits compaction_end only if start had already been emitted, differentiates overflow-recovery and automatic-compaction error messages and returns false. Finally clears its controller and releases the lock. This creates a deliberate durable commit boundary: cancellation changes whether work continues, not whether an already saved entry is rolled back (ice:packages/coding-agent/src/core/agent-session.ts:2753-2794).

### 5. Interfaces

reason='threshold'|'overflow'; before hook preparation/branchEntries/customInstructions/signal; after hook compactionEntry/fromExtension; end result optional, aborted, willRetry and errorMessage optional. Controller cancellation, extension veto, preparation absence and saved-then-cancelled completion have distinct observable results.

### 6. Dependencies

`EXT-022`, `SESSION-014`, `SESSION-015`, `SESSION-016`

### 7. ice-rust today

Rust already writes write_deferred, a compaction entry and step_attempt around each checkpoint, preserving summary, retained tail, tokens, details and usage (rust:crates/ice-harness/src/agent_harness.rs:3206-3250). The inspected pre-run and mid-loop call sites have no corresponding before/after session compaction hook or saved-then-cancelled continuation policy. The mid-loop call suppresses errors and immediately installs a summary context; the pre-run path converts summary error into a failed run (rust:crates/ice-harness/src/agent_harness.rs:3697-3785; rust:crates/ice-harness/src/agent_harness.rs:3867-3915).

### 8. Port notes

Wrap Rust's durable checkpoint primitive with an automatic-compaction transaction state machine that records whether start was emitted and whether the entry was committed. Run extension reducers at the specified boundaries, check cancellation both before and after commit, and expose result-bearing end events. Preserve committed checkpoints on late cancellation and make continuation a returned decision. Add source-derived verification for silent no-op preparation, extension veto without controller abort, cancellation during auth, cancellation before append and cancellation during the after hook.

### 9. Size and priority

Estimated Rust implementation: **170-360 lines** (estimate, not measured source). **P1** - The commit boundary determines whether a user cancellation loses context, repeats work or incorrectly wakes the agent.

## SESSION-018 - Session retry admission, generation-local effect guard and retry events

### 1. Title and status

Session retry admission, generation-local effect guard and retry events. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/agent-session.ts:800-877; ice:packages/coding-agent/src/core/agent-session.ts:1337-1382; ice:packages/coding-agent/src/core/agent-session.ts:3256-3379; ice:packages/coding-agent/test/agent-session-retry.test.ts:241-452; Additional complete test-source review: ice:packages/coding-agent/test/suite/agent-session-retry-events.test.ts:1-371; ice:packages/coding-agent/test/suite/regressions/6647-compaction-retries-transient-stream-drop.test.ts:1-193

### 3. What it does

Retry a safely repeatable failed assistant generation while keeping durable history, cancellation and UI lifecycle consistent.

### 4. How it works

The session applies a safety gate before the shared retry classifier: context overflow, a toolCall block in the failed assistant, or a possible tool effect in that assistant generation all prohibit an ordinary retry. The effect flag resets at agent_start and each assistant message_start, and is set by tool execution events and tool-result message start; a completed earlier tool turn therefore does not itself prohibit retrying a later empty failed generation (ice:packages/coding-agent/src/core/agent-session.ts:800-877; ice:packages/coding-agent/src/core/agent-session.ts:3260-3267). Source tests explicitly distinguish incomplete tool calls from a transient failure after a completed tool turn and assert the earlier tool executes once (ice:packages/coding-agent/test/agent-session-retry.test.ts:166-306).

Post-run handling checks ordinary retry before automatic compaction and queued continuation. _prepareRetry reads live retry settings, returns false if disabled, increments the attempt, and decrements it again if it exceeds maxRetries so final failure reports completed attempts. It emits auto_retry_start with attempt/maxAttempts/delayMs/errorMessage ('Unknown error' fallback), removes only the terminal assistant from live agent state while preserving the session entry, and waits on its own abort controller. A cancelled sleep resets attempt to zero and emits auto_retry_end(success=false, attempt, finalError='Retry cancelled'); the controller is always cleared. A non-error assistant message_end ends a pending retry successfully, while the enclosing prompt promise remains pending through the resumed tool loop and final settlement (ice:packages/coding-agent/src/core/agent-session.ts:843-877; ice:packages/coding-agent/src/core/agent-session.ts:1337-1382; ice:packages/coding-agent/src/core/agent-session.ts:3305-3355; ice:packages/coding-agent/test/agent-session-retry.test.ts:366-452).

Summary retries reuse the settings policy but have three separate public callbacks: summarization_retry_scheduled carries attempt/maxAttempts/delayMs/errorMessage; summarization_retry_attempt_start carries source='branchSummary' or source='compaction' plus manual/threshold/overflow reason; summarization_retry_finished has no payload. abortRetry cancels only the wait; isRetrying means a wait controller exists, not the entire retry continuation. setAutoRetryEnabled persists the preference and emits a configuration change (ice:packages/coding-agent/src/core/agent-session.ts:3270-3379).

### 5. Interfaces

AgentSession events auto_retry_start, auto_retry_end, summarization_retry_scheduled, summarization_retry_attempt_start and summarization_retry_finished; settings.retry enabled/maxRetries/baseDelayMs; abortRetry(), isRetrying, autoRetryEnabled. Shared classifier and delay defaults belong to AI-001; this entry specifies the session call sites and event state.

### 6. Dependencies

`AGENT-024`, `AI-001`, `SESSION-015`, `SESSION-022`

### 7. ice-rust today

Rust has a real retry loop, durable retry_pending records, failed-frame retirement, a retry budget, capped backoff, cancellation and AgentEvent::RetryScheduled. It rebuilds context from durable state after the wait so a previously committed tool turn need not replay (rust:crates/ice-harness/src/agent_harness.rs:4034-4150). That call site uses the shared assistant-error classifier directly and has no coding-session generation-local effect flag, auto_retry_end lifecycle or source-bearing summary retry callback family. The classifier itself examines stop reason and error text (rust:crates/ice-harness/src/compaction/compaction.rs:500-563). Rust's explicit AgentSession::retry resubmits the last user message and is a different operation (rust:crates/ice-cli/src/agent_session.rs:237-245).

### 8. Port notes

Add a session retry coordinator around the existing Rust durable loop. Track side-effect possibility per assistant generation, apply the overflow/tool-call/effect gate, retain durable failure history, and expose the exact start/end and summary-source callbacks. Preserve Rust retry recovery records and context reconstruction. Verify source-derived cases for partial tool-call rejection, one execution of a completed earlier tool turn, budget exhaustion, cancellation during delay, success followed by a slow tool, and summary retry indicator recreation.

### 9. Size and priority

Estimated Rust implementation: **170-350 lines** (estimate, not measured source). **P1** - An overly broad retry can repeat effects; an overly restrictive guard or premature settlement breaks recoverable sessions and caller ordering.

## SESSION-019 - Prompt preflight, input transformation and busy-session routing

### 1. Title and status

Prompt preflight, input transformation and busy-session routing. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/agent-session.ts:1384-1669; ice:packages/coding-agent/test/agent-session-concurrent.test.ts:43-369; Additional complete test-source review: ice:packages/coding-agent/test/suite/agent-session-prompt.test.ts:1-444; ice:packages/coding-agent/test/suite/regressions/pre-prompt-compaction-no-continue.test.ts:1-76; ice:packages/coding-agent/test/suite/regressions/7150-rpc-prompt-during-compaction.test.ts:1-92; ice:packages/coding-agent/test/suite/regressions/9178-tree-during-compaction.test.ts:1-188

Exported symbols: `PromptOptions` (`ice:packages/coding-agent/src/core/agent-session.ts:292`).

### 3. What it does

Make every session prompt obey the same command, extension-input, expansion, compaction and authentication ordering.

### 4. How it works

Prompt preflight is a staged operation before the agent run. When expansion is enabled, an extension slash command is attempted first even while streaming; branch-summary activity is rejected next, then the awaited input hook can consume the input or transform text and optionally images before skill/template expansion. Idle prompts refresh resource-derived skill state before expansion. During compaction the prompt is queued as steering only when explicitly requested and as follow-up otherwise; during ordinary streaming an explicit streamingBehavior is required or it throws "Agent is already processing. Specify streamingBehavior ('steer' or 'followUp') to queue the message." (ice:packages/coding-agent/src/core/agent-session.ts:1384-1477).

For an idle run the order is flush pending user-bash messages, require a model, check configured authentication (hasConfiguredAuth short-circuits; otherwise checkAuth must return something other than undefined), inspect the prior assistant for pre-prompt compaction with continuation disabled, construct the new user message, append pending next-turn custom messages after that user message, and add a changed background-task reminder. The user message always contains a text block, even for empty text, followed by supplied images. before_agent_start then reduces custom messages/system prompt before _runAgentPrompt; preflightResult receives true for handled/queued/accepted input and false on failure. The pre-prompt compaction decision does not issue an extra continuation ahead of the new prompt (ice:packages/coding-agent/src/core/agent-session.ts:1478-1554).

Extension command parsing splits at the first space; its handler error is reported as an extension diagnostic and still counts as handled. /skill expansion reads and strips frontmatter, wraps source content and trailing arguments, and returns the original text with an extension error on read failure. steer/followUp run command rejection before and after the input transform, then apply skill/template expansion; each queued display item receives a UUID and retains the exact AgentMessage object used in the core queue (ice:packages/coding-agent/src/core/agent-session.ts:1556-1669).

### 5. Interfaces

prompt(text,{images?, streamingBehavior?:'steer'|'followUp', expandPromptTemplates?, source?, preflightResult?}); input hook result handled/transform/continue; source defaults to interactive for public input and extension for sendUserMessage. steer(text, images?) and followUp(text, images?) preserve transformed images when omitted by the hook. Command handlers and expansion diagnostics are extension interfaces, not provider messages. Branch-summary rejection is 'Cannot submit a prompt while the session tree is being navigated. Wait for it to finish and retry.' Extension-queue rejection identifies the command and directs the caller to prompt() or idle execution.

```ts
export interface PromptOptions {
	/** Whether to expand file-based prompt templates (default: true) */
	expandPromptTemplates?: boolean;
	/** Image attachments */
	images?: ImageContent[];
	/** When streaming, how to queue the message: "steer" (interrupt) or "followUp" (wait). Required if streaming. */
	streamingBehavior?: "steer" | "followUp";
	/** Source of input for extension input event handlers. Defaults to "interactive". */
	source?: InputSource;
	/** Internal hook used by RPC mode to observe prompt preflight acceptance or rejection. */
	preflightResult?: (success: boolean) => void;
}
```

### 6. Dependencies

`AGENT-059`, `AGENT-063`, `EXT-020`, `EXT-030`, `SESSION-015`, `SESSION-020`, `SESSION-024`, `SESSION-025`

### 7. ice-rust today

The Rust coding AgentSession prompt/send_message/steer/follow_up methods directly delegate to the harness (rust:crates/ice-cli/src/agent_session.rs:153-177). The harness normalizes images, acquires run admission and persists incoming prompts before its pre-run compaction; next-run messages precede the new prompt rather than following it (rust:crates/ice-harness/src/agent_harness.rs:3504-3511; rust:crates/ice-harness/src/agent_harness.rs:3547-3785). Explicit skill and prompt-template methods exist, but that does not implement the common coding-session input/command preflight or compaction queue-routing contract (rust:crates/ice-harness/src/agent_harness.rs:4553-4590). TUI input has a separate busy-steering path (rust:crates/ice-cli/src/interactive_tui.rs:1408-1476).

### 8. Port notes

Introduce a reusable coding-session prompt facade shared by TUI, RPC and SDK. Implement the specified command/input/expansion order, preflight completion callback and busy/compacting routes; authenticate and compact before committing the new prompt; place deferred custom messages after the user message. Preserve Rust run admission and image normalization beneath the facade. Verify consumed input, transformed images, extension-command errors, commands created by transforms, prompt during compaction, busy input without routing, and pre-prompt compaction without an extra provider call.

### 9. Size and priority

Estimated Rust implementation: **260-530 lines** (estimate, not measured source). **P1** - Different entry points currently accept and order user intent differently, especially while a run or compaction is active.

## SESSION-020 - Custom-message delivery modes and identity-preserving queue controls

### 1. Title and status

Custom-message delivery modes and identity-preserving queue controls. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/agent-session.ts:1671-1887; ice:packages/coding-agent/src/core/agent-session.ts:879-900; Additional complete test-source review: ice:packages/coding-agent/test/suite/agent-session-queue.test.ts:1-479; ice:packages/coding-agent/test/suite/regressions/9340-message-queue-integrity.test.ts:1-152

Exported symbols: `QueuedMessageEntry` (`ice:packages/coding-agent/src/core/agent-session.ts:229`).

### 3. What it does

Distinguish immediate context-only writes, steering, follow-up and next-turn custom messages without losing or mis-removing queued input.

### 4. How it works

sendCustomMessage assigns role='custom', the caller's customType/content/display/details and a fresh timestamp; live content normalizes null/missing input to []. The idle direct persistence call still receives the original caller content, an observed ingestion inconsistency. deliverAs='nextTurn' always appends to the session's pending-next-turn array, regardless of whether a run is active or triggerTurn is true. While streaming, triggerTurn=false or absence of both deliverAs and triggerTurn buffers context-only messages; otherwise followUp selects that queue and every other active delivery steers. When idle, triggerTurn=true awaits a complete new prompt run; otherwise it immediately adds the message to live state and SessionManager, then emits public message_start/message_end without passing through the internal extension-event reducer (ice:packages/coding-agent/src/core/agent-session.ts:1732-1783).

Pending context-only messages are flushed at turn_end and agent_end into live state and the separate _customMessagesForNextTurn context-preservation list, with synthetic start/end events routed through the internal event handler. That context-preservation list is cleared at final session settlement and used by prepareNextTurn to preserve messages against context replacement. It is distinct from explicit nextTurn delivery's _pendingNextTurnMessages, which is consumed by the next idle user prompt (ice:packages/coding-agent/src/core/agent-session.ts:680-717; ice:packages/coding-agent/src/core/agent-session.ts:774-797; ice:packages/coding-agent/src/core/agent-session.ts:879-900). sendUserMessage joins text blocks with newlines, collects images, invokes prompt with expansion disabled and source='extension', and inherits prompt busy routing.

The UI queue is not the authoritative core queue. Removal looks up the public UUID, asks the core to remove the identical AgentMessage object, and only then removes the UI item; an item already consumed by the core returns false and remains available for the normal message-start dequeue. message_start removes the matching steering item first, otherwise follow-up, by object identity. clearQueue clears steering/follow-up display and core queues, returning only their display texts; it does not clear custom deferred arrays. abort waits for the complete run, while runtime teardown additionally aborts branch summarization and waits for compaction settlement (ice:packages/coding-agent/src/core/agent-session.ts:1671-1730; ice:packages/coding-agent/src/core/agent-session.ts:806-826; ice:packages/coding-agent/src/core/agent-session.ts:1784-1887).

### 5. Interfaces

Custom delivery {deliverAs?:'steer'|'followUp'|'nextTurn', triggerTurn?:boolean}; internal pending item {id, message:string, agentMessage:AgentMessage}, with images inside agentMessage.content; removeQueuedMessage(id):boolean, getSteeringMessages():readonly string[], getFollowUpMessages():readonly string[], pendingMessageCount, clearQueue():{steering:string[], followUp:string[]}, abort(), waitForIdle(). queue_update exposes both text arrays and {id, message} entry arrays. Compaction must consult actual core queue predicates.

`QueuedMessageEntry` is `{ id: string; message: string }`; the private extension adds the exact core `agentMessage` reference (`ice:packages/coding-agent/src/core/agent-session.ts:229-236`).

### 6. Dependencies

`AGENT-005`, `AGENT-022`, `AGENT-027`, `EXT-020`, `SESSION-019`, `SESSION-021`, `SESSION-022`

### 7. ice-rust today

Rust already has serialized send_message admission, durable Context/NextRun/Steer/FollowUp queue records, reserved entry IDs and per-lane recovery. Idle context-only writes persist before appended-message events; active writes are durable before their eventual entry commit (rust:crates/ice-harness/src/agent_harness.rs:1151-1412). NextRun messages are drained ahead of the new prompt (rust:crates/ice-harness/src/agent_harness.rs:3555-3606). Queue cancellation removes in-memory state before writing a cancellation record and only logs persistence failure, so the acknowledged cancellation can reappear after recovery; clear_queued_messages excludes NextRun (rust:crates/ice-harness/src/agent_harness.rs:4691-4816). This is substantial functionality but a different durable queue/error contract, not the coding facade's object-identity display bookkeeping.

### 8. Port notes

Implement the coding delivery matrix and UI queue identities over Rust's durable queues, including explicit context-only behavior, next-turn ordering and await-to-settlement semantics. Preserve durable recovery rather than replacing it with transient arrays. Make cancellation acknowledgment contingent on a durable cancel record or return an explicit persistence failure, and define rollback/retry behavior. Verify same-text distinct messages, consumed-item removal, transformed images, deferred custom writes during event callbacks, cancellation-record failure and restart recovery.

### 9. Size and priority

Estimated Rust implementation: **210-450 lines** (estimate, not measured source). **P1** - Incorrect queue ownership or acknowledgment can drop user input, wake a run unexpectedly, or resurrect cancelled instructions.

## SESSION-021 - Runtime session replacement, teardown settlement and exclusive import

### 1. Title and status

Runtime session replacement, teardown settlement and exclusive import. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/agent-session-runtime.ts:1-476; ice:packages/coding-agent/src/core/session-cwd.ts:1-60; ice:packages/coding-agent/test/agent-session-runtime-events.test.ts:1-258; Additional complete test-source review: ice:packages/coding-agent/test/suite/agent-session-runtime.test.ts:1-664; ice:packages/coding-agent/test/suite/regressions/2860-replaced-session-context.test.ts:1-280; ice:packages/coding-agent/test/suite/regressions/9340-runtime-session-settlement.test.ts:1-205

Exported symbols: `CreateAgentSessionRuntimeResult` (`ice:packages/coding-agent/src/core/agent-session-runtime.ts:24`), `CreateAgentSessionRuntimeFactory` (`ice:packages/coding-agent/src/core/agent-session-runtime.ts:36`), `SessionImportFileNotFoundError` (`ice:packages/coding-agent/src/core/agent-session-runtime.ts:47`), `SessionRuntimeUnavailableError` (`ice:packages/coding-agent/src/core/agent-session-runtime.ts:57`).

Exported symbols: `SessionCwdIssue` (`ice:packages/coding-agent/src/core/session-cwd.ts:3`), `getMissingSessionCwdIssue` (`ice:packages/coding-agent/src/core/session-cwd.ts:14`), `formatMissingSessionCwdError` (`ice:packages/coding-agent/src/core/session-cwd.ts:35`), `formatMissingSessionCwdPrompt` (`ice:packages/coding-agent/src/core/session-cwd.ts:40`), `assertSessionCwdExists` (`ice:packages/coding-agent/src/core/session-cwd.ts:54`).

### 3. What it does

Replace the complete coding session with a well-defined veto, shutdown, invalidation, construction and rebinding lifecycle.

### 4. How it works

AgentSessionRuntime guards access with availability and exposes a stable runtime_unavailable error during an invalid or failed replacement. New/resume invoke session_before_switch while the old runtime remains usable; fork invokes session_before_fork before target validation. A veto returns cancellation without teardown. Switch opens and validates the target session's cwd before retiring the old session, so a missing cwd leaves the old session usable (ice:packages/coding-agent/src/core/agent-session-runtime.ts:79-202; ice:packages/coding-agent/src/core/session-cwd.ts:1-60).

Teardown is performed once: abort and await run/compaction/branch settlement; await session_shutdown with reason/target; call beforeInvalidate synchronously; mark unavailable, invalidate extension context, and dispose old resources. The factory then receives target cwd/manager, reason and previousSessionFile. The fresh session becomes available before rebinding; a rebind failure marks it unavailable, whereas a failure in the optional withSession callback occurs after a successful replacement and keeps the new session available. New-session setup can append initial entries before rebuilding context; setup failure leaves the runtime unavailable (ice:packages/coding-agent/src/core/agent-session-runtime.ts:109-271).

Fork's default position is before a selected user message; at-position supports any entry. It settles the active session before copying; persisted forks require an existing source file and use a separate opened manager, whereas an in-memory fork mutates that manager before teardown. Import resolves and validates the source, creates the destination directory before the veto hook, chooses a UUID suffix on collision and uses COPYFILE_EXCL. It copies before opening/cwd validation/teardown, so a later failure may leave the imported file; it never overwrites a same-name existing session. Dispose emits shutdown reason='quit' through the same teardown path (ice:packages/coding-agent/src/core/agent-session-runtime.ts:273-476). Source tests pin before-switch → shutdown → new session_start order and the validity boundary for old contexts (ice:packages/coding-agent/test/agent-session-runtime-events.test.ts:1-258).

### 5. Interfaces

AgentSessionRuntime current/getSession availability; newSession, switchSession, fork, importSession, dispose; factory context {cwd, sessionManager, reason, previousSessionFile}; beforeInvalidate and withSession callbacks; cancelled result vs runtime_unavailable vs missing-cwd error. Cwd validation precedes destructive teardown on switch/import.

`CreateAgentSessionRuntimeResult` extends the SDK result with `services: AgentSessionServices` and `diagnostics: AgentSessionRuntimeDiagnostic[]`. `CreateAgentSessionRuntimeFactory` takes `{ cwd, agentDir, sessionManager, sessionStartEvent?, projectTrustContext? }` and returns that result asynchronously. `SessionImportFileNotFoundError` retains filePath and `File not found: <path>`; `SessionRuntimeUnavailableError.errorCode` is `runtime_unavailable` (`ice:packages/coding-agent/src/core/agent-session-runtime.ts:24-63`).

### 6. Dependencies

`EXT-010`, `EXT-020`, `EXT-022`, `SESSION-016`, `SESSION-020`, `SESSION-029`, `SESSION-030`, `SESSION-033`

### 7. ice-rust today

Rust set_session has a real message-write gate, rejects closed/busy harnesses, validates the lane and reconstructs durable queues before swapping storage; it cancels child scopes and clears old queue state (rust:crates/ice-harness/src/agent_harness.rs:2056-2166). That preserves a single harness configuration rather than rebuilding the coding session and extension bindings. The TUI switch aborts/waits, opens and activates a session, then renders all entries; its import copies to the same basename with std::fs::copy, permitting overwrite, and its fork lacks the same entry-target/veto lifecycle (rust:crates/ice-cli/src/interactive_tui/sessions.rs:102-278).

### 8. Port notes

Create a runtime host that owns the Rust coding-session object and executes this replacement state machine. Reuse set_session's validated queue recovery where appropriate, but recreate cwd-bound tools/resources/auth/config and extension contexts at the documented boundaries. Replace import overwrite with exclusive destination creation and collision suffixing, expose cancellation separately from unavailable failures, and retain concrete copied-file behavior after late failure. Verify veto before target validation, missing cwd before teardown, slow compaction settlement, stale old contexts, rebind failure and colliding import.

### 9. Size and priority

Estimated Rust implementation: **330-670 lines** (estimate, not measured source). **P1** - Replacement must not overwrite sessions, strand in-flight mutations or let stale extension contexts act on a different conversation.

## SESSION-022 - Coding-session event ordering, durable writes and final settlement

### 1. Title and status

Coding-session event ordering, durable writes and final settlement. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/agent-session.ts:723-1089; ice:packages/coding-agent/src/core/agent-session.ts:1311-1382; ice:packages/coding-agent/test/agent-session-concurrent.test.ts:371-630; Additional complete test-source review: ice:packages/coding-agent/test/suite/agent-session-bash-persistence.test.ts:1-338; ice:packages/coding-agent/test/suite/agent-session-configuration-events.test.ts:1-145; ice:packages/coding-agent/test/suite/regressions/1717-2113-agent-session-event-settlement.test.ts:1-96

Exported symbols: `SessionConfiguration` (`ice:packages/coding-agent/src/core/agent-session.ts:162`), `AgentSessionEventListener` (`ice:packages/coding-agent/src/core/agent-session.ts:238`).

### 3. What it does

Expose a consistent event stream whose extension transformations, persistence and idle promises have explicit ordering.

### 4. How it works

AgentSession's internal subscription survives removal of public subscribers and processes core events serially. For ordinary events it first awaits the extension mapping/reducer, then calls public listeners synchronously in registration order, then persists message_end; public listener failures are not isolated here. A message_end extension replacement mutates the original message object in place after deleting old keys, and null/missing content on a replacement user/assistant/toolResult/custom message becomes [], preserving identity for other consumers. Custom messages append through appendCustomMessageEntry, standard messages through appendMessage, and user bash is persisted separately (ice:packages/coding-agent/src/core/agent-session.ts:723-900; ice:packages/coding-agent/src/core/agent-session.ts:927-1043).

State changes precede relevant callbacks: user message_start resets overflow recovery and dequeues the identical UI queue item; assistant message_start resets the possible-effect flag; assistant message_end records the last message and may finish retry. turn_start resets extension turn index at agent_start, and turn_end increments it only after the awaited extension handler. Context-only custom messages accepted during a run are flushed after turn_end/agent_end with recursively processed synthetic message events. Tests inspect durable user/assistant history from a later tool hook and prove slow message_end hooks are awaited before the next tool starts (ice:packages/coding-agent/src/core/agent-session.ts:800-900; ice:packages/coding-agent/src/core/agent-session.ts:927-1043; ice:packages/coding-agent/test/agent-session-concurrent.test.ts:371-630).

agent_end is not final session idleness: retry, automatic compaction, resumed turns and queued continuation can still follow. _runAgentPrompt sets a session-level active flag before awaiting the core and keeps it through post-run handling. Finally it clears extension/prompt state, flush pending user-bash results, then settle. Settlement first clears active and next-turn context state, awaits extension agent_settled, emits public agent_settled, and resolves the idle promise in finally. Configuration snapshots are JSON-deduplicated; isStreaming includes the complete session operation, and isCompacting includes lock/manual/auto/branch summary ownership (ice:packages/coding-agent/src/core/agent-session.ts:744-797; ice:packages/coding-agent/src/core/agent-session.ts:1045-1089; ice:packages/coding-agent/src/core/agent-session.ts:1311-1382).

### 5. Interfaces

Public AgentSessionEvent union includes core events plus agent_settled, configuration_changed, queue changes, compaction/retry and bash updates. subscribe(listener) returns unsubscribe; waitForIdle resolves after awaited settlement even if an earlier settlement callback throws. Extension message_end may replace or suppress content while retaining object identity. The main observer must not infer final idleness from core agent_end alone.

```ts
export interface SessionConfiguration {
	fastMode: boolean;
	steeringMode: "all" | "one-at-a-time";
	followUpMode: "all" | "one-at-a-time";
	autoCompactionEnabled: boolean;
	autoRetryEnabled: boolean;
	activeTools: string[];
	scopedModels: Array<{ provider: string; modelId: string; thinkingLevel?: ThinkingLevel }>;
}
export type AgentSessionEventListener = (event: AgentSessionEvent) => void;
```

### 6. Dependencies

`AGENT-021`, `EXT-008`, `EXT-022`, `SESSION-015`, `SESSION-018`, `SESSION-020`, `SESSION-024`

### 7. ice-rust today

Rust has durable operation boundaries and a finish_context_writes loop that drains reentrant writes before/after RunEnd and releases admission afterward; wait_for_idle registers its notification before rechecking to avoid a lost wake (rust:crates/ice-harness/src/agent_harness.rs:1442-1613; rust:crates/ice-harness/src/agent_harness.rs:4864-4896). Its standalone HarnessEventBus exposes RunStart, RunEnd and CompactionProgress, while core AgentEvents are tee-emitted to the UI before the extension emitter is awaited (rust:crates/ice-harness/src/events.rs:1-180; rust:crates/ice-cli/src/session.rs:692-708; rust:crates/ice-cli/src/session.rs:1533-1551). The native TUI marks itself idle and flushes bash results on AgentEnd, and tolerates broadcast lag by waiting for a later full snapshot (rust:crates/ice-cli/src/interactive_tui/events.rs:150-203). These are real but differently ordered streams; the coding agent_settled lifecycle is absent.

### 8. Port notes

Add a coding-session event adapter with one awaited internal reducer, explicit persistence boundary and final agent_settled event after retries/compaction/queued work. Keep Rust's durable operation/admission guarantees and document any intentional persist-before-observer improvement rather than hiding it behind identical event names. Ensure UI and observers consume final settlement and transformed messages, retain subscription-independent persistence, and verify slow extension hooks, reentrant context writes, public unsubscribe and final idle notification.

### 9. Size and priority

Estimated Rust implementation: **280-560 lines** (estimate, not measured source). **P1** - Observers, RPC callers and extensions depend on whether an event is transformed, durable or truly final; incorrect ordering causes stale state and interleaved writes.

## SESSION-023 - Session-owned tool registry, activation and background-control integration

### 1. Title and status

Session-owned tool registry, activation and background-control integration. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/agent-session.ts:1093-1164; ice:packages/coding-agent/src/core/agent-session.ts:3036-3210; ice:packages/coding-agent/test/agent-session-dynamic-tools.test.ts:1-257

### 3. What it does

Keep executable tools, public definitions, source attribution and system-prompt metadata synchronized as extensions and SDK tools change.

### 4. How it works

The session builds builtin definitions, then overlays registered extension definitions and finally SDK custom definitions by name. SDK and builtin entries receive synthetic sourceInfo ('<sdk:name>'/'<builtin:name>'); executable wrappers and the public definition registry use the same override order. Prompt snippets and guidelines are normalized and retained separately, so an active tool without a prompt snippet remains executable but is absent from the prompt's tool-description list (ice:packages/coding-agent/src/core/agent-session.ts:1093-1135; ice:packages/coding-agent/src/core/agent-session.ts:3036-3106; ice:packages/coding-agent/test/agent-session-dynamic-tools.test.ts:90-257).

Allowlisting governs bash_output/bash_kill/bash_watch/bash_input through the name 'bash'; exclusion is checked against each actual tool name. Registry refresh preserves previous active names unless explicitly supplied, then adds allowlisted tools, all extension tools when requested, or newly registered tools when no explicit activation was supplied. Refresh deduplicates with Set before activation. setActiveToolsByName first strips all four control names from the caller list and appends them only if bash is present, ignores unknown names, assigns the active executable array, rebuilds the base prompt, reapplies any per-run override and emits configuration change. That public setter itself does not deduplicate duplicate ordinary names (ice:packages/coding-agent/src/core/agent-session.ts:1144-1164; ice:packages/coding-agent/src/core/agent-session.ts:3108-3130).

Default construction uses read/bash/edit/write, with read image-resize settings and bash prefix/path/background registry/autoBackgroundMs. It creates the four control definitions against the same registry; bash_input reads approval through a live callback. A baseToolsOverride uses only supplied tools and skips automatic control-definition creation. The extension runtime receives the session context, including current cwd/id/file, and the actual core steering predicate; tool-level environment injection and its opt-out are specified in TOOLS-002. Source tests verify ICE_* injection before a custom spawn hook and explicit opt-out, as well as tools registered at session_start and during commands (ice:packages/coding-agent/src/core/agent-session.ts:2990-3005; ice:packages/coding-agent/src/core/agent-session.ts:3133-3210; ice:packages/coding-agent/test/agent-session-dynamic-tools.test.ts:1-257).

### 5. Interfaces

getAllTools/getToolDefinition/getRegisteredTool, setActiveToolsByName/getActiveToolNames; definition plus sourceInfo/promptSnippet/promptGuidelines/access; allowedToolNames/excludedToolNames/baseToolsOverride/customTools. Control names are bash_output, bash_kill, bash_watch, bash_input. Dynamic changes apply on the next core turn through the session's prepareNextTurn adapter.

### 6. Dependencies

`AGENT-018`, `EXT-019`, `EXT-024`, `SESSION-022`, `SESSION-025`, `TOOLS-002`, `TOOLS-006`, `TOOLS-008`, `TOOLS-009`, `TOOLS-011`

### 7. ice-rust today

Rust has mutable set_tools/set_active_tools and executable tool registries, but the inspected harness setters replace registry/name arrays without rebuilding a coding-session prompt or emitting configuration lifecycle events (rust:crates/ice-harness/src/agent_harness.rs:2023-2042; rust:crates/ice-harness/src/agent_harness.rs:4973-4986). Native CLI construction assembles tools/resources separately; the missing background/control mechanics are recorded in Tools. Rust already republishes ICERS_SESSION_ID and ToolCallContext identity on successful session activation, which must be preserved, rather than described as wholly absent session identity (rust:crates/ice-cli/src/session.rs:2521-2577).

### 8. Port notes

Implement one coding-session registry owner that produces definitions, executable wrappers, source metadata and prompt fragments from a deterministic overlay. Add activation rules for bash controls, per-name exclusions, dynamic registration refresh and the same default/override construction branches. Bind current session metadata and live steering/settings callbacks to tool context while retaining Rust's session-identity publication. Verify override precedence, unknown/duplicate names, removing bash, individual control exclusions, newly registered tools and base override behavior.

### 9. Size and priority

Estimated Rust implementation: **220-450 lines** (estimate, not measured source). **P1** - A stale or differently filtered tool registry can advertise unavailable tools, omit required controls or run tools under the wrong session context.

## SESSION-024 - User-bash operation ownership, streaming updates and ordered persistence

### 1. Title and status

User-bash operation ownership, streaming updates and ordered persistence. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/agent-session.ts:3396-3497; ice:packages/coding-agent/src/core/agent-session.ts:1311-1335; Additional complete test-source review: ice:packages/coding-agent/test/suite/agent-session-bash-persistence.test.ts:1-338

### 3. What it does

Expose user/extension shell execution independently of model bash tasks, with isolated cancellation and durable result ordering.

### 4. How it works

executeBash checks a pre-aborted caller signal before acquiring work, then creates a controller in a session-owned Set and links only that operation's caller signal. Settings lookup is inside the cleanup scope: prefix is prepended with a newline, shellPath configures local operations, cwd comes from SessionManager, and custom BashOperations may replace local execution. Each chunk first invokes the caller callback and then emits bash_execution_update{id?, delta}; a caller callback error therefore prevents that update and propagates through execution. The recorded message retains the original command, not the prefixed command. Finally always unlinks the signal and removes the controller (ice:packages/coding-agent/src/core/agent-session.ts:3396-3430).

recordBashResult accepts externally executed results too, constructing a bashExecution message with output/exitCode/cancelled/truncated/fullOutputPath/timestamp/excludeFromContext. While the session-level run is active it buffers the result so it cannot split assistant tool-call/result ordering; otherwise it appends to live state and durable session immediately, without synthetic message_start/message_end. Buffered results flush in insertion order before final agent_settled and before a new idle prompt. The buffer is cleared after all appends, so an append failure does not silently discard the unflushed collection (ice:packages/coding-agent/src/core/agent-session.ts:1311-1335; ice:packages/coding-agent/src/core/agent-session.ts:1478-1486; ice:packages/coding-agent/src/core/agent-session.ts:3432-3497).

abortBash snapshots all current controllers and aborts them; an individual options.signal affects only its own command. isBashRunning reports controller-set nonemptiness and hasPendingBashMessages reports unflushed results. These user-bash commands are separate from BashBackgroundRegistry tasks and their model-facing completion/watch messages.

### 5. Interfaces

executeBash(command, onChunk?,{excludeFromContext?, id?, operations?, signal?}):Promise<BashResult>; recordBashResult(command, result,{excludeFromContext?}); abortBash(), isBashRunning, hasPendingBashMessages; bash_execution_update{id?, delta}. bashExecution role projection/context exclusion is AGENT-063; capture/truncation mechanics remain Tools.

### 6. Dependencies

`AGENT-063`, `SESSION-022`, `SESSION-025`, `TOOLS-003`

### 7. ice-rust today

Rust genuinely implements asynchronous TUI !command/!!command execution: one UI run slot, cancellation, streamed output, capture result and a UserBashFinished message (rust:crates/ice-cli/src/interactive_tui.rs:543-647; rust:crates/ice-cli/src/interactive_tui.rs:1408-1423). Completion appends immediately when the UI status is idle or buffers otherwise; AgentEnd sets idle and flushes via lane.append_message (rust:crates/ice-cli/src/interactive_tui.rs:2913-2940; rust:crates/ice-cli/src/interactive_tui/events.rs:185-203). This path uses default capture options and lacks the reusable AgentSession executeBash/custom-operations/multiple-controller facade; its boundary is core AgentEnd rather than final coding-session settlement. It does not satisfy model durable background control contracts.

### 8. Port notes

Factor Rust's existing user-bash path into a session service callable from TUI, extensions and RPC. Add operation IDs, per-call signal ownership, custom operation backend, shell settings and final-settlement buffering while preserving original-command history and partial cancelled output. Keep the TUI's one-at-a-time UI policy as a caller restriction rather than the service's only ownership model. Verify simultaneous service calls, cancelling one versus all, setup failure cleanup, caller chunk callback failure and results arriving during retry/compaction.

### 9. Size and priority

Estimated Rust implementation: **190-390 lines** (estimate, not measured source). **P1** - Shell results and cancellation must attach to the correct conversation without interrupting tool-result ordering or cancelling unrelated work.

## SESSION-025 - Background-shell completion/watch delivery, idle wake and disposal wiring

### 1. Title and status

Background-shell completion/watch delivery, idle wake and disposal wiring. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/core/agent-session.ts:421-538; ice:packages/coding-agent/src/core/agent-session.ts:1045-1066; ice:packages/coding-agent/src/core/agent-session.ts:1180-1198; ice:packages/coding-agent/src/core/agent-session.ts:2999-3000; Additional complete test-source review: ice:packages/coding-agent/test/suite/agent-session-queue.test.ts:1-479

### 3. What it does

Connect model background-task registries to session context and wake behavior, while ensuring session teardown owns those tasks and watches.

### 4. How it works

Each AgentSession owns one BashBackgroundRegistry and one BashWatchRegistry. Watch silence/prompt timer settings are read through a live callback. Completion listeners keep at most the final 8,000 JavaScript string code units and prepend an omission marker that includes logPath/logIssue when available; commands are summarized for the notification. Normal completion emits a displayed bash_background_completed custom message and uses triggerTurn=true, deliverAs='steer', including a lingering-process instruction to use bash_kill. A deliberate user stop instead records a displayed 'do not restart unless asked' notification for nextTurn, without waking the model. Details carry taskId, summarized command, isError, durationMs and optional exitCode/description/stoppedByUser; notification-send rejection is swallowed (ice:packages/coding-agent/src/core/agent-session.ts:421-498).

Watch delivery depends on state before sending: busy always triggers steering; idle explicit watches and idle kind='prompt' alerts trigger a new turn; idle automatic non-prompt alerts append context/display only. A displayed bash_watch_fired message carries watchId, fireId, taskId, kind and auto plus reason, optional tail and log path. fireId is propagated as an identity only: this callback performs no duplicate lookup or suppression. A failure from sendCustomMessage is swallowed (ice:packages/coding-agent/src/core/agent-session.ts:499-522).

At idle user prompt, _backgroundShellReminder compares the comma-joined ordered running task IDs with the last reminded key. A changed empty set updates the key but emits nothing; a changed nonempty set emits hidden bash_background_running content listing elapsed runtime and first command line, after deferred next-turn messages. The live hasPendingSteering callback delegates to agent.hasQueuedSteering(), allowing waits to release at a real queued steering boundary rather than from stale display state. dispose attempts retry/compaction/branch/user-bash aborts, watch disposal, background disposal and agent abort in one try block, then invalidates/unsubscribes/clears listeners and calls cleanupSessionResources(sessionId); an exception in the first block skips its remaining operations, an observed cleanup limitation to avoid reproducing (ice:packages/coding-agent/src/core/agent-session.ts:1045-1066; ice:packages/coding-agent/src/core/agent-session.ts:1180-1198; ice:packages/coding-agent/src/core/agent-session.ts:1495-1520; ice:packages/coding-agent/src/core/agent-session.ts:2999-3000).

### 5. Interfaces

Custom types bash_background_completed, bash_watch_fired, bash_background_running; displayed completion/watch versus hidden changed-task reminder; 8,000-code-unit completion tail. Registry control APIs, fire construction, log storage and timer defaults belong to TOOLS-003 through TOOLS-013. Session owns routing, wake, settings callbacks, error handling and disposal.

### 6. Dependencies

`SESSION-020`, `SESSION-022`, `SESSION-023`, `TOOLS-003`, `TOOLS-004`, `TOOLS-005`, `TOOLS-008`, `TOOLS-010`, `TOOLS-011`, `TOOLS-012`, `TOOLS-013`

### 7. ice-rust today

The inspected Rust coding-session facade and CLI setup have no BashBackgroundRegistry/BashWatchRegistry delivery owner or these custom-message callbacks. The existing asynchronous user !command path is distinct and records bashExecution messages through TUI completion handling (rust:crates/ice-cli/src/agent_session.rs:95-426; rust:crates/ice-cli/src/interactive_tui.rs:543-647; rust:crates/ice-cli/src/interactive_tui.rs:2913-2940). Rust harness close marks closed and cancels child scope, not a model shell/watch registry (rust:crates/ice-harness/src/agent_harness.rs:2043-2054). Tools documents the missing lower-level registry/control contracts; this entry does not infer their existence from user-bash support.

### 8. Port notes

After implementing Tools registries, make the coding session their lifetime owner and wire the exact completion/watch delivery matrix through SESSION-020. Expose the real steering predicate, read timers/approval settings dynamically, add changed-running-set reminders, and propagate fire identity without inventing deduplication. Dispose independent resources even if one cleanup fails; retain the observed TypeScript limitation in compatibility notes. Verify busy watch wake, idle explicit/prompt wake, idle automatic silence without wake, deliberate user stop, long/log-backed completion and teardown during a pending wait.

### 9. Size and priority

Estimated Rust implementation: **180-380 lines** (estimate, not measured source). **P1** - Background work needs a reliable path back into the conversation; the wrong wake policy causes missed prompts or unwanted autonomous turns.

## SESSION-026 - Tree navigation with branch summarization, veto and commit-aware cancellation

### 1. Title and status

Tree navigation with branch summarization, veto and commit-aware cancellation. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/agent-session.ts:3529-3772; ice:packages/coding-agent/test/agent-session-tree-navigation.test.ts:1-324; Additional complete test-source review: ice:packages/coding-agent/test/suite/regressions/9178-tree-during-compaction.test.ts:1-188; ice:packages/coding-agent/test/suite/regressions/9340-runtime-session-settlement.test.ts:1-205

### 3. What it does

Navigate a conversation branch while preserving selected-user semantics, optional abandoned-branch summaries and extension lifecycle.

### 4. How it works

navigateTree first rejects a pre-aborted signal, active run or any compaction/branch summary; selecting the current leaf returns a no-op. Summarization requires a model and every selected target must exist. It computes the abandoned entries before acquiring a branch-summary controller/settlement promise, including when summarize=false. session_before_tree is always awaited and may cancel, provide a summary only when summarization was requested, or override customInstructions/replaceInstructions/label when those fields are not undefined (ice:packages/coding-agent/src/core/agent-session.ts:3529-3643).

If summarization is requested, there are abandoned entries and no extension summary, it resolves cancellation-aware runtime auth, applies branchSummary.reserveTokens, thinking/stream/environment and summary retry callbacks, then validates cancellation. Immediately before mutation it checks the signal again. Selecting a user message or custom_message moves to its parent and returns its text blocks concatenated without separators for the editor; selecting other entry types moves to that entry. A truthy summary appends a branch_summary at the destination parent (including a root destination), and an optional truthy label attaches to that summary; without a summary it branches/resets and labels the selected target. Empty summaries take the no-summary path (ice:packages/coding-agent/src/core/agent-session.ts:3644-3728).

After the leaf/summary commit it rebuilds agent messages, then awaits session_tree with old/new leaf, summary entry and fromExtension. Cancellation while that after hook runs returns an aborted result but does not roll back the saved summary or leaf move. AbortError is converted to cancelled/aborted; other errors propagate. Finally clears its controller and resolves branch settlement so runtime replacement can safely continue (ice:packages/coding-agent/src/core/agent-session.ts:3729-3772). Source E2E tests cover selecting users versus assistants and cancellation; they were inspected, not run (ice:packages/coding-agent/test/agent-session-tree-navigation.test.ts:1-324).

### 5. Interfaces

navigateTree(targetId,{summarize?, customInstructions?, replaceInstructions?, label?, signal?}) result cancelled/aborted/editorText/summaryEntry; session_before_tree and session_tree; abortBranchSummary and branch settlement. Normal selected-user navigation targets parent, while an explicit at-entry fork is a separate runtime operation.

### 6. Dependencies

`EXT-022`, `SESSION-012`, `SESSION-014`, `SESSION-018`, `SESSION-021`, `SESSION-033`

### 7. ice-rust today

Rust navigate_tree accepts target/summarize/custom instructions/label but discards the last three parameters. It chooses target or current tip, treats an empty string as no leaf and directly moves the lane, returning Completed{summary:None} or Failed. It does not acquire run admission, reject active compaction, run session tree hooks or generate a branch summary (rust:crates/ice-harness/src/agent_harness.rs:4592-4634). The coding facade's fork delegates to navigate_tree(None, false, None, label), so that method itself does not create a new session (rust:crates/ice-cli/src/agent_session.rs:399-405); the separate TUI storage-fork path is SESSION-021/033.

### 8. Port notes

Implement tree navigation as an admitted session operation with an owned cancellation/settlement boundary. Reuse branch summarization and durable leaf/entry primitives, add before/after extension reducers and selected-user/custom-message parent semantics, and return the editor text and summary entry. Keep post-commit cancellation distinct from rollback. Verify busy rejection, same-leaf no-op, root navigation, empty versus extension-provided summary, label overrides and cancellation during auth, summary generation and after-hook.

### 9. Size and priority

Estimated Rust implementation: **240-500 lines** (estimate, not measured source). **P1** - Ignoring summary/target semantics can discard conversational context or move a branch while another operation is still writing.

## SESSION-027 - All-history usage totals and post-compaction context uncertainty

### 1. Title and status

All-history usage totals and post-compaction context uncertainty. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/agent-session.ts:3774-3877; ice:packages/coding-agent/src/core/usage-totals.ts:1-116; ice:packages/coding-agent/test/agent-session-stats.test.ts:1-284

Exported symbols: `UsageTotals` (`ice:packages/coding-agent/src/core/usage-totals.ts:4`), `createUsageTotals` (`ice:packages/coding-agent/src/core/usage-totals.ts:12`), `addUsageToTotals` (`ice:packages/coding-agent/src/core/usage-totals.ts:22`), `subagentUsageFromEntry` (`ice:packages/coding-agent/src/core/usage-totals.ts:31`), `UsageCostBreakdownEntry` (`ice:packages/coding-agent/src/core/usage-totals.ts:72`), `getUsageCostBreakdown` (`ice:packages/coding-agent/src/core/usage-totals.ts:79`).

### 3. What it does

Report lifetime billing separately from current-branch context, including summary, tool and subagent usage without double-counting.

### 4. How it works

getSessionStats scans all SessionManager entries, including abandoned branches. It adds validated subagent usage custom entries, branch-summary/compaction usage, tool-result usage and assistant usage; counts message entries by user/assistant/toolResult role and counts toolCall blocks only in assistant content. The reported token total is input+output+cacheRead+cacheWrite rather than trusting usage.totalTokens, and cost is the sum of usage.cost.total. It also returns sessionFile/sessionId and current context usage (ice:packages/coding-agent/src/core/agent-session.ts:3774-3831).

Subagent usage accepts only entry.type='custom' with customType='subagent_usage' and an object data.usage containing numeric input/output/cacheRead/cacheWrite/cost and validates input/output/cacheRead/cacheWrite as nonnegative safe integers, total cost as finite/nonnegative, and combined token count as safe. Invalid entries contribute nothing. The usage breakdown groups assistant records by provider and responseModel when present (otherwise model), while tool and summary usage share key 'Tools/summaries' and subagent usage uses 'Subagents'; zero rows are omitted and rows sort by cost descending (ice:packages/coding-agent/src/core/usage-totals.ts:1-116; ice:packages/coding-agent/test/usage-totals.test.ts:1-83).

getContextUsage returns undefined without a model or positive window. On the active branch, after the latest compaction it requires a later non-error/non-aborted assistant with positive context usage; until then it returns {tokens:null, contextWindow, percent:null}, not the pre-compaction provider count or zero. Once valid usage exists it estimates the current live messages, including subsequent content, and reports an unclamped tokens/window*100 percentage. A later zero-usage message does not erase the fact that a valid post-compaction measurement exists. Tests distinguish lifetime subagent billing from context occupancy and confirm unknown context immediately after compaction (ice:packages/coding-agent/src/core/agent-session.ts:3833-3877; ice:packages/coding-agent/test/agent-session-stats.test.ts:1-284). The source expression at line 3855 is connector-redacted; surrounding control flow and inspected tests establish the positive-usage gate, not an invented literal implementation.

### 5. Interfaces

SessionStats {sessionFile?, sessionId, userMessages, assistantMessages, toolCalls, toolResults, totalMessages, tokens:{input, output, cacheRead, cacheWrite, total}, cost, contextUsage?}; ContextUsage {tokens:number|null, contextWindow, percent:number|null}; usage breakdown rows group model and non-model categories. Unknown context is distinct from absent model and from zero tokens.

### 6. Dependencies

`SESSION-010`, `SESSION-013`, `SESSION-029`

### 7. ice-rust today

Rust SessionState accumulates message_count from message entries and token/cost totals from separate LaneRecord::Usage records, including usage.total_tokens as supplied (rust:crates/ice-harness/src/session/state.rs:185-245). The native coding facade exposes those aggregate stats, while total_usage scans assistant messages only and does not add cost components, tool/summary/subagent usage or derive total from components (rust:crates/ice-cli/src/agent_session.rs:301-328). A usage record system is a valid architecture, but it does not itself satisfy this coding-session report schema or the post-compaction unknown-context rule.

### 8. Port notes

Implement a report projection over the Rust canonical log that preserves all-history billing while separating active-branch context. Define one authority for usage records versus entry-embedded usage to avoid counting both, include tool/summary/subagent contributions and validation, derive the compatibility token total from components, and expose null context until a valid post-compaction response. Preserve native usage-record detail as an additional capability. Verify abandoned branches, summary cost, malformed subagent usage, response-model grouping, post-compaction null and fresh measurement followed by zero usage.

### 9. Size and priority

Estimated Rust implementation: **180-370 lines** (estimate, not measured source). **P2** - Usage and context displays drive compaction decisions and cost interpretation; lifetime billing must not be confused with prompt occupancy.

## SESSION-028 - SDK construction, cwd-bound services and live request configuration

### 1. Title and status

SDK construction, cwd-bound services and live request configuration. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/sdk.ts:38-97; ice:packages/coding-agent/src/core/sdk.ts:171-414; ice:packages/coding-agent/src/core/agent-session-services.ts:1-224; Additional complete test-source review: ice:packages/coding-agent/test/suite/agent-session-model-extension.test.ts:1-398; ice:packages/coding-agent/test/suite/agent-session-tool-result-images.test.ts:1-110; ice:packages/coding-agent/test/suite/regressions/6768-copilot-compaction-base-url.test.ts:1-115

Exported symbols: `AgentSessionRuntimeDiagnostic` (`ice:packages/coding-agent/src/core/agent-session-services.ts:25`), `CreateAgentSessionServicesOptions` (`ice:packages/coding-agent/src/core/agent-session-services.ts:37`), `CreateAgentSessionFromServicesOptions` (`ice:packages/coding-agent/src/core/agent-session-services.ts:54`), `AgentSessionServices` (`ice:packages/coding-agent/src/core/agent-session-services.ts:74`).

Exported symbols: `AgentSessionConfig` (`ice:packages/coding-agent/src/core/agent-session.ts:250`).

Exported symbols: `CreateAgentSessionResult` (`ice:packages/coding-agent/src/core/sdk.ts:90`).

Exported symbols: `getDefaultSessionDir` (`ice:packages/coding-agent/src/core/session-manager.ts:492`).

### 3. What it does

Provide an embeddable session factory with deterministic dependency injection, restore precedence, startup diagnostics and live stream settings.

### 4. How it works

createAgentSession resolves cwd from explicit options, then the supplied SessionManager, then process.cwd; agentDir is explicit-resolved or the global config default. It preserves supplied modelRuntime/settingsManager/sessionManager/resourceLoader instances. Only an omitted loader is created and reloaded; explicit agentDir determines auth.json/models.json paths and the default encoded-cwd sessions directory. Existing-session restoration means buildSessionContext().messages.length>0, not merely a header or metadata entry. Explicit model wins; otherwise a saved provider/model is restored only if found and configured auth exists, then findInitialModel is called with an empty scopedModels selection and settings defaults. Failure/fallback is returned as modelFallbackMessage rather than terminating. Thinking uses explicit option, saved branch thinking when an entry exists, settings default, then medium, and is clamped or forced off when there is no model (ice:packages/coding-agent/src/core/sdk.ts:171-246).

Default active tools are read/bash/edit/write. An explicit tools list wins over noTools; noTools='all' supplies an empty allowlist, while noTools='builtin' suppresses only default builtin activation, leaving extension/custom tools available; excludeTools filters afterward. Provider conversion dynamically replaces user/toolResult image blocks with 'Image reading is disabled.' when blockImages is set and deduplicates consecutive identical placeholders only in content containing images. Each stream reads current provider retries and timeouts: explicit timeoutMs wins over providerRetry.timeoutMs over HTTP idle timeout; idle timeout zero becomes 2147483647. WebSocket timeout, maxRetries and maxRetryDelayMs prefer explicit call options. Fast priority tier is admitted only for advertised priority support on openai-responses/openai-codex-responses; attribution headers are composed before before_provider_headers, request payload is passed through before_provider_request, and response status/headers through awaited after_provider_response (ice:packages/coding-agent/src/core/sdk.ts:248-375).

The Agent receives the SessionManager ID for provider session resources, live extension context transformation and settings-derived queue modes/transport/thinking budgets. Existing context is restored and a missing thinking entry appended; new sessions append initial model and thinking entries, still subject to lazy persistence. Construction returns {session, extensionsResult, modelFallbackMessage}; it does not itself emit session_start through bindExtensions (ice:packages/coding-agent/src/core/sdk.ts:364-414).

createAgentSessionServices separately resolves cwd/config, creates model runtime with cancellation, constructs/reloads the loader, drains pending compatibility provider registrations then native registrations with per-registration diagnostics, clears both queues, and refreshes with allowNetwork=false before applying flags. Every supplied registered boolean flag is set true even if the supplied value is false; string flags require a string; missing values become error diagnostics and all unknown names form one aggregated diagnostic. It returns coherent services plus diagnostics without printing/exiting; createAgentSessionFromServices forwards these exact instances (ice:packages/coding-agent/src/core/agent-session-services.ts:83-224).

`getDefaultSessionDir(cwd, agentDir = getDefaultAgentDir())` obtains the resolved, encoded cwd path and creates the directory recursively only when absent before returning it (`ice:packages/coding-agent/src/core/session-manager.ts:487-498`).

### 5. Interfaces

CreateAgentSessionOptions includes cwd, agentDir, modelRuntime, model, thinkingLevel, fastMode, scopedModels, noTools, tools, excludeTools, customTools, resourceLoader, sessionManager, settingsManager, sessionStartEvent. Result exposes session/extensionsResult/modelFallbackMessage. Service diagnostics are {type:'info'|'warning'|'error', message}. Paths are configuration contracts only; no real user-state directory was opened for this audit.

Exact session constructor inputs:
```ts
export interface AgentSessionConfig {
	agent: Agent;
	sessionManager: SessionManager;
	settingsManager: SettingsManager;
	cwd: string;
	/** Models to cycle through with Ctrl+P (from --models flag) */
	scopedModels?: Array<{ model: Model<any>; thinkingLevel?: ThinkingLevel }>;
	/** Resource loader for extensions, skills, prompts, themes, context files, and system prompt */
	resourceLoader: ResourceLoader;
	/** SDK custom tools registered outside extensions */
	customTools?: ToolDefinition[];
	/** Canonical model/auth runtime used by coding-agent internals. */
	modelRuntime: ModelRuntime;
	/** Initial active built-in tool names. Default: [read, bash, edit, write] */
	initialActiveToolNames?: string[];
	/** Optional allowlist of tool names. When provided, only these tool names are exposed. */
	allowedToolNames?: string[];
	/** Optional denylist of tool names. When provided, these tool names are not exposed. */
	excludedToolNames?: string[];
	/**
	 * Override base tools (useful for custom runtimes).
	 *
	 * These are synthesized into minimal ToolDefinitions internally so AgentSession can keep
	 * a definition-first registry even when callers provide plain AgentTool instances.
	 */
	baseToolsOverride?: Record<string, AgentTool>;
	/** Mutable ref used by Agent to access the current ExtensionRunner */
	extensionRunnerRef?: { current?: ExtensionRunner };
	/** Session start event metadata emitted when extensions bind to this runtime. */
	sessionStartEvent?: SessionStartEvent;
}
```

### 6. Dependencies

`AI-001`, `EXT-009`, `EXT-031`, `SESSION-021`, `SESSION-023`, `SESSION-029`, `SESSION-035`, `SESSION-036`

### 7. ice-rust today

Rust has substantial CLI assembly: prepared extensions, cwd-bound OsExecutionEnv/tool context, merged tools, selected persistent/in-memory session, live plugin session identity and resource/prompt construction (rust:crates/ice-cli/src/session.rs:328-539). AgentHarness::create accepts resolved low-level options and the native AgentSession constructor wraps an existing harness/catalog/cwd (rust:crates/ice-harness/src/agent_harness.rs:682-810; rust:crates/ice-cli/src/agent_session.rs:95-151). These are not the injectable coding SDK/service factory or its restore/fallback/diagnostic contract. The underlying request hooks/providers are covered by their respective areas; the gap here is composing them consistently for non-CLI callers.

### 8. Port notes

Add a Rust coding-session builder and cwd-service bundle with the explicit injection and precedence rules, typed nonfatal diagnostics, controlled loader reload and separate extension bind/start stage. Route streaming through a live settings adapter, preserving option overrides and session identity. Keep CLI assembly as a client of the builder. Verify supplied dependency identity, manager-derived cwd, metadata-only restore, unavailable saved model, thinking fallback, noTools modes, disabled images and timeout/tier/header precedence.

### 9. Size and priority

Estimated Rust implementation: **320-650 lines** (estimate, not measured source). **P1** - Embedders and replacement sessions require the same resource/auth/model behavior as the CLI without hidden reloads or process exits.

## SESSION-029 - Coding-session version 3 schema, identity and legacy migrations

### 1. Title and status

Coding-session version 3 schema, identity and legacy migrations. **Status: different-by-design.**

### 2. ice source

ice:packages/coding-agent/src/core/session-manager.ts:33-305; ice:packages/coding-agent/src/core/session-manager.ts:940-965; ice:packages/coding-agent/docs/session-format.md:1-439; Additional complete test-source review: ice:packages/coding-agent/test/suite/regressions/9340-runtime-session-settlement.test.ts:1-205

Exported symbols: `NewSessionOptions` (`ice:packages/coding-agent/src/core/session-manager.ts:44`), `SessionEntryBase` (`ice:packages/coding-agent/src/core/session-manager.ts:49`), `SessionMessageEntry` (`ice:packages/coding-agent/src/core/session-manager.ts:56`), `ThinkingLevelChangeEntry` (`ice:packages/coding-agent/src/core/session-manager.ts:61`), `ModelChangeEntry` (`ice:packages/coding-agent/src/core/session-manager.ts:66`), `LabelEntry` (`ice:packages/coding-agent/src/core/session-manager.ts:114`), `SessionInfoEntry` (`ice:packages/coding-agent/src/core/session-manager.ts:121`).

### 3. What it does

Bridge the coding agent's entry-based v3 session files to Rust's native v4 mutation log without conflating the two formats.

### 4. How it works

Coding SessionManager writes a first parsed header {type:'session', version:3, id, timestamp, cwd, parentSession?}, where timestamp is ISO text and parentSession is a source file path. Every ordinary entry has type/id/parentId/timestamp. The entry union is message, thinking_level_change, model_change, compaction, branch_summary, custom, custom_message, label and session_info. A custom entry stores opaque extension data outside LLM context; custom_message stores text/image content, display and optional details for context/display. Compaction stores summary, firstKeptEntryId, tokensBefore, optional details/usage/fromHook; branch_summary stores fromId and analogous summary metadata (ice:packages/coding-agent/src/core/session-manager.ts:33-180).

New session IDs default to UUIDv7. Explicit IDs must be nonempty ASCII alphanumeric at both ends with only alphanumeric, dot, underscore and hyphen inside; the TS validator imposes no length maximum. Entry IDs use the first eight hex characters of randomUUID with up to 100 collision checks, then a full UUID fallback. Filenames are ISO timestamp with ':'/'.' replaced by '-' plus '_<id>.jsonl'. New-session state clears entry/label indices and leaf, remains unflushed, and preserves an optional parent path (ice:packages/coding-agent/src/core/session-manager.ts:217-237; ice:packages/coding-agent/src/core/session-manager.ts:940-965).

Missing version is treated as v1. v1→v2 assigns fresh IDs and linear parents and converts firstKeptEntryIndex to the referenced entry ID when valid; v2→v3 renames message role hookMessage to custom. Versions at least 3 are left untouched. Opening an old file rewrites migrated entries in place. The v1 loop supplies a Set for collision checking but never adds assigned IDs to it; record this observed limitation rather than pretending the migration guarantees uniqueness. Source tests pin v1 structure and v2 ID preservation (ice:packages/coding-agent/src/core/session-manager.ts:239-305; ice:packages/coding-agent/src/core/session-manager.ts:924-929; ice:packages/coding-agent/test/session-manager/migration.test.ts:1-79).

### 5. Interfaces

CURRENT_SESSION_VERSION=3; ISO header/entry timestamps; parentSession path; nine entry variants listed above; ReadonlySessionManager exports header/tree/context/identity accessors. Legacy parse/migrate exports are loadEntriesFromFile, parseSessionEntries and migrateSessionEntries. Native v4 session capabilities are additional, not proof of v3 compatibility.

### 6. Dependencies

`AGENT-063`, `AGENT-064`, `SESSION-013`, `SESSION-030`, `SESSION-031`, `SESSION-033`

### 7. ice-rust today

Rust JSONL writes {kind:'header', version:4, id, createdAt, cwd, parentSessionId?, legacyParentSessionPath?, metadata?} and tagged entry/record/lane/fact mutations. parse_header requires kind='header', exact version 4, typed timestamps and mutually exclusive parent ID/path; entry whitelist lacks coding custom_message/label/session_info because facts and message variants serve different roles (rust:crates/ice-harness/src/session/jsonl/codec.rs:4-65; rust:crates/ice-harness/src/session/jsonl/codec.rs:108-150). JsonlSourceFormat::V3 is an enum value with explanatory comments, not a decoder; the inspected crates search found no migration implementation. Rust explicit IDs additionally have a 64-byte limit (rust:crates/ice-harness/src/session/jsonl/repo.rs:74-104).

### 8. Port notes

Keep the native v4 durable architecture and add an explicit v1/v2/v3 coding-file import/export adapter. Translate timestamps, parent provenance, labels/names/custom messages and reference-based compaction, preserving original files and IDs wherever the source format permits. Expose source format in metadata and report unsupported versions; choose and document a compatibility policy for the extra Rust ID length limit. Fix migration collision tracking in the adapter rather than copying the TS omission. Verify old headerless-version files, hookMessage rename, compaction references, custom data/context separation and roundtrip labels/provenance.

### 9. Size and priority

Estimated Rust implementation: **420-850 lines** (estimate, not measured source). **P1** - Without an explicit format bridge, existing coding sessions cannot be resumed safely even though both implementations use JSONL.

## SESSION-030 - Bounded tolerant session reads versus native log recovery

### 1. Title and status

Bounded tolerant session reads versus native log recovery. **Status: different-by-design.**

### 2. ice source

ice:packages/coding-agent/src/core/session-manager.ts:504-632; ice:packages/coding-agent/src/core/session-manager.ts:905-938; ice:packages/coding-agent/src/core/session-manager.ts:1600-1647; ice:packages/coding-agent/test/session-manager/file-operations.test.ts:1-377; ice:packages/coding-agent/test/session-manager/unterminated-tail.test.ts:1-47

### 3. What it does

Make discovery/open/import behavior explicit for large files, malformed lines, oversized headers and unterminated tails.

### 4. How it works

loadEntriesFromFile normalizes the path, returns [] for absence, reads 1 MiB chunks with StringDecoder so UTF-8 boundaries are preserved, skips blank/malformed JSON lines and accepts a valid final line without a newline. The first successfully parsed item must have type='session' and a string id; otherwise the loader returns []. It does not fully validate the entry schema. Header discovery separately uses 4 KiB chunks with a 1 MiB scan ceiling, skips blank/malformed lines, stops on the first parsed non-header, and probes one byte beyond the ceiling so exact-limit EOF is accepted. Discovery swallows unreadable/oversized failures and returns null (ice:packages/coding-agent/src/core/session-manager.ts:504-632).

Explicit open normally reads only the header to derive cwd before constructing the manager; a scan-limit error falls back to full loading so an explicitly selected valid large header can open. Cwd uses explicit override, then string header cwd, then process.cwd. Session directory uses an explicit directory or the selected file's parent. A nonempty file that does not parse as a session throws 'Session file is not a valid ice session: <path>' without changing it; an empty existing file is initialized at that path, while a missing explicit path gets a lazy new session (ice:packages/coding-agent/src/core/session-manager.ts:905-938; ice:packages/coding-agent/src/core/session-manager.ts:1600-1647).

An ordinary v3 open does not repair bytes or add a terminal newline. On the next append, the writer checks the last byte and inserts a separator before its new record, preserving valid or malformed prior tail bytes. Only legacy-version migration or empty-file initialization rewrites on open. Tests inspect unchanged bytes for an unterminated final record and preservation of malformed tails; large-file tests validate chunking beyond Node's single-string limit and distinguish discovery's bounded scan from explicit open fallback (ice:packages/coding-agent/src/core/session-manager.ts:1025-1058; ice:packages/coding-agent/test/session-manager/unterminated-tail.test.ts:1-47; ice:packages/coding-agent/test/session-manager/file-operations.test.ts:1-377).

### 5. Interfaces

loadEntriesFromFile(path):FileEntry[], header discovery null on best-effort failure, explicit invalid-file error, SessionHeaderScanLimitError internal fallback, read buffer 1 MiB/header buffer 4 KiB/header ceiling 1 MiB. Tolerance applies to coding v3 source; it must not be silently imposed on a native durable mutation log.

### 6. Dependencies

`SESSION-029`, `SESSION-031`, `SESSION-032`

### 7. ice-rust today

Rust native load reads the entire file string and requires a valid v4 first physical line. It skips undecodable assistant-frame schema records, rejects other schema/nonterminal syntax errors, repairs terminal syntax errors by publishing the physical prefix, drops every seq<=current sequence as a duplicate, accepts forward sequence gaps and re-chains named-lane entry parents in memory. It rewrites dropped-sequence logs and appends a newline during open when absent (rust:crates/ice-harness/src/session/jsonl/storage.rs:133-276; rust:crates/ice-harness/src/session/jsonl/storage.rs:352-383). These are materially different recovery semantics. In particular, seq equality alone does not prove payload duplication, and parent correction is not reflected in kept original lines; the audit makes no crash-safety guarantee from the helper's name.

### 8. Port notes

Implement bounded, nonmutating coding-file inspection/import as a distinct adapter and retain native v4 recovery behind an explicit policy. Avoid whole-file string requirements for large logs. For native recovery, compare duplicate identity/payload before discarding conflicting mutations, report unresolved conflicts and decide whether parent repairs should be persisted; do not claim data is duplicate solely from sequence. Separate inspect from repair and make byte changes reviewable. Verify malformed leading lines, parsed non-header first, exact/over scan limit, multi-byte chunk boundaries, valid/malformed unterminated tails and conflicting duplicate sequences.

### 9. Size and priority

Estimated Rust implementation: **280-580 lines** (estimate, not measured source). **P1** - Opening a file must not silently destroy distinct history or turn a recoverable large session into an unavailable one.

## SESSION-031 - Lazy first-message persistence, exclusive creation and durable append barrier

### 1. Title and status

Lazy first-message persistence, exclusive creation and durable append barrier. **Status: different-by-design.**

### 2. ice source

ice:packages/coding-agent/src/core/session-manager.ts:905-1078; ice:packages/coding-agent/src/core/session-manager.ts:1150-1165; ice:packages/coding-agent/test/session-manager/custom-session-id.test.ts:1-170

### 3. What it does

Specify when a session file is created, what append failure means and which API promises an fsync barrier.

### 4. How it works

A new persistent SessionManager initially holds a header and metadata entries in memory. _persist returns without creating a file until a message entry with role user or assistant arrives, preserving metadata-only/read-only startup while saving the request before a provider response. The first flush opens with 'wx' and writes every buffered entry; later appends open 'a+', add a missing newline separator and append one record. The first ordinary append does not set an explicit file mode, and regular appends do not fsync. In-memory managers never write (ice:packages/coding-agent/src/core/session-manager.ts:940-965; ice:packages/coding-agent/src/core/session-manager.ts:1025-1058).

_appendEntry mutates fileEntries, byId and leaf before attempting persistence; a write failure leaves that in-memory mutation visible and prevents entry_appended notification. A successful append emits only after persistence returns. Empty explicit files are initialized immediately, old-version migration rewrites immediately, and metadata-only new sessions remain lazy unless the caller explicitly requests a durable barrier (ice:packages/coding-agent/src/core/session-manager.ts:905-938; ice:packages/coding-agent/src/core/session-manager.ts:1072-1078).

flushDurably requires persistent storage and a session path or throws 'Durable session entries require a persistent session; --no-session is not supported.' It opens 'wx' for an unflushed session or 'a' otherwise with mode 0600, writes buffered entries if needed, sets flushed, fsyncs the file and closes in finally. It does not fsync the containing directory. The extension appendEntry binding calls this barrier before and after its custom append only when options.durable is true, so durable extension state can materialize an otherwise metadata-only file (ice:packages/coding-agent/src/core/session-manager.ts:1150-1165; ice:packages/coding-agent/src/core/agent-session.ts:2920-2955).

### 5. Interfaces

isPersisted(), newSession(), append* methods, flushDurably(); persisted capability is distinct from whether the lazy file already exists. Exclusive first creation uses wx; explicit legacy rewrite uses w. These are observed TS guarantees/limitations, not claims of transactional memory rollback or power-loss-safe directory durability.

### 6. Dependencies

`EXT-020`, `SESSION-022`, `SESSION-029`, `SESSION-030`

### 7. ice-rust today

Rust native JsonlSessionStorage::create eagerly writes a header through FileSystem::write_file; repo creation checks an in-process claimed key and existing suffix before creation, rather than the coding manager's lazy first-message wx protocol (rust:crates/ice-harness/src/session/jsonl/storage.rs:111-130; rust:crates/ice-harness/src/session/jsonl/repo.rs:218-322). Mutations validate/stamp, append a line under a per-handle tail mutex, then apply in memory; drain merely waits for that mutex and is not an fsync barrier (rust:crates/ice-harness/src/session/jsonl/storage.rs:319-349; rust:crates/ice-harness/src/session/jsonl/storage.rs:400-438; rust:crates/ice-harness/src/session/jsonl/storage.rs:465-515). Preserve the stronger persist-before-memory ordering rather than copying the TS failure artifact.

### 8. Port notes

Expose an explicit coding-session creation policy (lazy metadata-only or eager native) and a durable flush API backed by actual file synchronization. Use exclusive file creation to prevent cross-process overwrite, retaining Rust's persist-before-apply ordering. Document ordinary append versus durability barrier and implement extension append's before/after barrier where supported; reject durable requests on ephemeral storage. Verify metadata-only startup, first user before provider response, existing-path collision, append failure, fsync failure and explicit custom durable writes.

### 9. Size and priority

Estimated Rust implementation: **230-470 lines** (estimate, not measured source). **P1** - Creation and durability promises affect whether user requests or extension state survive failures and whether concurrent creation can overwrite a session.

## SESSION-032 - Session discovery, activity timestamps, cwd scoping and deletion fallback

### 1. Title and status

Session discovery, activity timestamps, cwd scoping and deletion fallback. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/session-manager.ts:644-837; ice:packages/coding-agent/src/core/session-manager.ts:1563-1600; ice:packages/coding-agent/src/core/session-manager.ts:1708-1840; Additional complete test-source review: ice:packages/coding-agent/test/suite/regressions/7497-session-discovery-symlink.test.ts:1-76

Exported symbols: `SessionListProgress` (`ice:packages/coding-agent/src/core/session-manager.ts:776`).

### 3. What it does

Find and sort usable coding sessions consistently across default directories, custom flat directories and damaged files.

### 4. How it works

findMostRecentSession considers .jsonl files with a valid bounded-discovery header, optionally requires a nonempty header cwd whose resolved path matches the requested cwd, and sorts by filesystem mtime descending. Any directory/stat race makes that helper return null. continueRecent uses a custom-directory cwd filter when the directory differs from the default; absence creates a fresh session. Default directories encode the resolved cwd under the configured sessions root; explicit directories are flat (ice:packages/coding-agent/src/core/session-manager.ts:644-665; ice:packages/coding-agent/src/core/session-manager.ts:1563-1600).

Detailed list metadata streams the whole file, requires the first parsed item's type='session' (less strict than the ID check in header discovery), tracks the latest session_info name including clears, counts all message entries and builds joined text plus first nonempty user text. Modified activity is the maximum user/assistant message timestamp (or entry timestamp fallback), then header date, then mtime; tool/custom/settings appends do not make a conversation appear newly active. Session list runs at concurrency 10, reports progress for every attempted candidate including invalid files, filters custom directories by cwd and sorts by computed modified time. listAll scans a supplied flat directory or one level of directories/symlinks under the default root, skipping inaccessible directories and counting candidates before progress (ice:packages/coding-agent/src/core/session-manager.ts:667-837; ice:packages/coding-agent/src/core/session-manager.ts:1708-1790; ice:packages/coding-agent/test/session-info-modified-timestamp.test.ts:1-84).

Deletion first tries the platform trash helper, placing '--' before a leading-dash path, with a 10-second timeout and 64 KiB output cap. If the file is gone it succeeds even after a helper error; otherwise it falls back to unlink. On final failure it includes the first 200 characters of helper stderr as a hint. This is a session-file operation; it does not traverse or remove related sessions (ice:packages/coding-agent/src/core/session-manager.ts:1791-1840).

### 5. Interfaces

SessionInfo {path, id, cwd, name?, parentSessionPath?, created, modified, messageCount, firstMessage, allMessagesText}; findMostRecentSession():string|null; list(cwd, sessionDir?, onProgress?) and listAll(sessionDir?, onProgress?); continueRecent; deleteSessionFile. The recent helper's mtime ordering differs intentionally from detailed list's conversation-activity ordering.

### 6. Dependencies

`SESSION-029`, `SESSION-030`, `SESSION-033`

### 7. ice-rust today

Rust JSONL repo lists encoded cwd directories or root directory/symlink children and produces native metadata with filesystem modified_at. It reads file text/header and propagates filesystem errors for the overall list, while invalid headers are skipped; its shared metadata projection lacks coding name/firstMessage/allMessagesText/activity fields (rust:crates/ice-harness/src/session/jsonl/repo.rs:324-384; rust:crates/ice-harness/src/session/jsonl/repo.rs:479-614; rust:crates/ice-harness/src/session/jsonl/types.rs:69-99). Native deletion directly removes the file, and the generic repo trait resolves by the default-cwd metadata scan rather than carrying the rich path (rust:crates/ice-harness/src/session/jsonl/repo.rs:386-409; rust:crates/ice-harness/src/session/jsonl/repo.rs:624-798).

### 8. Port notes

Add a coding-session discovery projection with bounded headers, flat/default directory rules, per-file error isolation, conversation-activity timestamps and progress semantics. Preserve native metadata fields alongside the compatibility view and keep recent-by-mtime distinct from detailed activity sorting. Carry explicit paths through open/delete/fork APIs rather than losing them in base metadata. Add trash-first deletion with safe argument handling and unlink fallback where the product requires compatibility. Verify symlink roots, mixed cwd flat directories, malformed files, timestamp-only metadata appends and deletion helper failure.

### 9. Size and priority

Estimated Rust implementation: **200-430 lines** (estimate, not measured source). **P2** - Session selectors and resume must find the correct conversation without one corrupt file blocking unrelated history.

## SESSION-033 - Coding branch extraction, label history and parent-path provenance

### 1. Title and status

Coding branch extraction, label history and parent-path provenance. **Status: different-by-design.**

### 2. ice source

ice:packages/coding-agent/src/core/session-manager.ts:1273-1301; ice:packages/coding-agent/src/core/session-manager.ts:1408-1590; ice:packages/coding-agent/src/core/session-manager.ts:1649-1707; ice:packages/coding-agent/test/session-manager/fork-integrity.test.ts:1-47; ice:packages/coding-agent/test/session-manager/labels.test.ts:1-212

### 3. What it does

Extract a selected coding branch without orphaning entries, losing final labels or changing the retained compaction context.

### 4. How it works

branch validates the target, changes only the leaf pointer and emits leaf_changed only when it actually changes; resetLeaf behaves likewise. Neither writes a branch-move record. branchWithSummary saves the old leaf (or 'root') as fromId, moves to the requested parent, appends one branch_summary and emits one entry_appended with no separate leaf_changed. appendLabelChange requires an existing target, appends a real tree entry and updates resolved label/timestamp maps before notifying; truthy labels set and undefined/empty labels clear (ice:packages/coding-agent/src/core/session-manager.ts:1273-1301; ice:packages/coding-agent/src/core/session-manager.ts:1408-1458).

createBranchedSession follows root→selected leaf, rejects an empty path, removes all label entries and re-chains retained parents. If a compaction firstKeptEntryId named a removed label it maps to the next retained entry. It preserves retained entry IDs, generates a new UUIDv7 header and session filename, and uses the old persisted file path as parentSession. It recreates only final resolved labels whose targets survived, preserving label timestamps, generating new label-entry IDs and appending them at the new tail. Persisted branches with at least one user/assistant entry are written immediately; metadata-only branches stay lazy. The manager itself changes to the new session. In-memory extraction returns undefined and retains corresponding resolved label state without creating a file (ice:packages/coding-agent/src/core/session-manager.ts:1465-1590).

forkFrom is a separate whole-source copy into a new cwd/session directory: it loads source entries, replaces the header with v3/new ID/current timestamp/new cwd/parent source path, writes exclusively, and opens the result. It copies non-header entries unchanged rather than using createBranchedSession's label filtering. Because raw v1 data can be placed under a v3 header before open, legacy migration of that path is not established by the inspected tests; the adapter must handle it explicitly (ice:packages/coding-agent/src/core/session-manager.ts:1649-1707). Source tests establish label-final-value preservation, parent re-chaining, compaction-boundary remapping and unchanged source history (ice:packages/coding-agent/test/session-manager/fork-integrity.test.ts:1-47; ice:packages/coding-agent/test/session-manager/labels.test.ts:1-212).

### 5. Interfaces

branch(id), resetLeaf(), branchWithSummary(parentId|null, summary, details?, fromHook?, usage?), createBranchedSession(leafId):path|undefined, forkFrom(sourcePath, cwd, sessionDir?). Labels are entry history plus a resolved map in coding v3; parentSession is a file path. Runtime before/at selection and replacement hooks are SESSION-021/026.

### 6. Dependencies

`SESSION-002`, `SESSION-013`, `SESSION-021`, `SESSION-026`, `SESSION-029`, `SESSION-031`

### 7. ice-rust today

Rust implements native branch/tree fork mutations, retaining entry IDs and label/name facts and tracking parentSessionId, then stages a new v4 file via its native storage fork (rust:crates/ice-harness/src/session/state.rs:438-572; rust:crates/ice-harness/src/session/jsonl/storage.rs:279-317). Native lane moves are durable mutations and labels are separate facts, so there are no coding label-entry parents/timestamps to preserve automatically (rust:crates/ice-harness/src/session/state.rs:252-297). Existing fork/stat/order differences are SESSION-002; this entry focuses on coding-format extraction/provenance compatibility, not a claim that Rust lacks forks.

### 8. Port notes

Implement branch extraction at the coding-format adapter boundary while preserving native lane/fact capabilities. Carry source path and ID provenance explicitly, translate final labels/timestamps and rewrite parent/firstKept references after filtering. Distinguish same-file leaf navigation, extracted-session fork and whole-source fork in public APIs. Migrate legacy data before emitting a new v3 header. Verify labels outside the selected path, cleared labels, label nodes at compaction boundaries, user-only durable forks and in-memory identity changes.

### 9. Size and priority

Estimated Rust implementation: **230-470 lines** (estimate, not measured source). **P1** - Incorrect extraction can orphan the retained path or lose context/labels while appearing to create a valid new session.

## SESSION-034 - Coding SessionManager read views, context projection and local mutation events

### 1. Title and status

Coding SessionManager read views, context projection and local mutation events. **Status: different-by-design.**

### 2. ice source

ice:packages/coding-agent/src/core/session-manager.ts:307-503; ice:packages/coding-agent/src/core/session-manager.ts:1060-1078; ice:packages/coding-agent/src/core/session-manager.ts:1181-1207; ice:packages/coding-agent/src/core/session-manager.ts:1241-1396; ice:packages/coding-agent/test/session-manager/build-context.test.ts:1-306; ice:packages/coding-agent/test/session-manager-events.test.ts:1-63; Additional complete test-source review: ice:packages/coding-agent/test/suite/agent-session-runtime.test.ts:1-664; ice:packages/coding-agent/test/suite/regressions/5996-session-name-newlines.test.ts:1-41; ice:packages/coding-agent/test/suite/regressions/9340-runtime-session-settlement.test.ts:1-205

Exported symbols: `getLatestCompactionEntry` (`ice:packages/coding-agent/src/core/session-manager.ts:325`).

### 3. What it does

Expose coding tree and context views with clear snapshot/reference and local-event semantics over the native session model.

### 4. How it works

getEntries returns a new array excluding headers but shares entry objects; getEntry/getHeader/getLeafEntry return the stored objects, and getChildren walks insertion order. getBranch returns root-to-leaf by following parents and returns [] for an unknown start. getTree creates new node/children wrappers sharing entry objects, includes null/self-parent/orphan entries as roots, resolves labels with their latest-change timestamp and sorts each child array by timestamp using an iterative traversal. These are shallow defensive views, not immutable deep snapshots (ice:packages/coding-agent/src/core/session-manager.ts:1241-1396).

Standalone context building resolves an omitted/unknown leaf to its documented fallback path, while an explicit null leaf yields empty context. It derives model and thinking from the full selected path, including assistant model metadata, with default thinking='off' and model=null. Only context-bearing entries become messages: ordinary/custom_message entries, truthy branch summaries and compaction summaries; custom extension data, labels and session_info do not. Latest compaction changes the visible context to summary plus retained reference-based entries and subsequent entries, while model/thinking changes before that boundary still affect settings. Null/missing standard message content is normalized at projection (ice:packages/coding-agent/src/core/session-manager.ts:307-503; ice:packages/coding-agent/test/session-manager/build-context.test.ts:1-306).

Session names come from the latest session_info across all history, not the selected branch; writes collapse CR/LF runs to spaces and trim, and an empty latest name clears the title. subscribe uses a Set and snapshots listeners for each future local mutation, with no historical replay or failure isolation. Appends notify after persistence/index updates; label append waits until label maps agree; no-op leaf moves emit nothing. Opening/replacing a session is a host lifecycle and does not replay local append events (ice:packages/coding-agent/src/core/session-manager.ts:1060-1078; ice:packages/coding-agent/src/core/session-manager.ts:1181-1207; ice:packages/coding-agent/test/session-manager-events.test.ts:1-63).

### 5. Interfaces

ReadonlySessionManager covers cwd/sessionDir/id/file/leaf/entry/label/branch/context/header/allEntries/tree/name. SessionManagerEvent is exactly entry_appended{entry} or leaf_changed{leafId:string|null}. SessionContext {messages, thinkingLevel, model:{provider, modelId}|null}. Shallow references, root ordering and child timestamp ordering are separate contracts.

### 6. Dependencies

`AGENT-065`, `SESSION-001`, `SESSION-003`, `SESSION-013`, `SESSION-022`, `SESSION-029`, `SESSION-033`

### 7. ice-rust today

Rust provides clone-returning native entry/lane views, a context builder and separate global name/label facts; this supplies the underlying capabilities but not the coding v3 shallow-reference/event schema (rust:crates/ice-harness/src/session/session.rs:126-326; rust:crates/ice-harness/src/session/context.rs:1-464; rust:crates/ice-harness/src/session/state.rs:267-297). Its HarnessEventBus is a different RunStart/RunEnd/CompactionProgress stream (rust:crates/ice-harness/src/events.rs:1-180). Native context projection and any intentional provider-message repair remain AGENT-065 rather than being relabeled as missing storage.

### 8. Port notes

Provide a typed coding-session view adapter that defines root/branch ordering, names/labels and reference-based context, plus a local mutation subscription distinct from run events. Preserve Rust owned snapshots as a deliberate safety difference and document that consumers cannot mutate canonical storage through returned objects. Emit append/leaf events only after successful canonical mutation and make loaded/replaced lifecycle separate. Verify unknown/null leaves, orphan/self roots, child timestamps, pre-compaction settings, name clearing and event-time label visibility.

### 9. Size and priority

Estimated Rust implementation: **200-430 lines** (estimate, not measured source). **P2** - Extensions and session selectors depend on precise tree/context views and event payloads, even when storage internals differ.

## SESSION-035 - Extension binding, resource extension and reload sequencing

### 1. Title and status

Extension binding, resource extension and reload sequencing. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/agent-session.ts:2796-3035; ice:packages/coding-agent/src/core/agent-session.ts:3217-3250; ice:packages/coding-agent/src/core/agent-session.ts:4012-4019; Additional complete test-source review: ice:packages/coding-agent/test/suite/agent-session-model-extension.test.ts:1-398; ice:packages/coding-agent/test/suite/regressions/2860-replaced-session-context.test.ts:1-280; ice:packages/coding-agent/test/suite/regressions/5943-session-start-notify.test.ts:1-523; ice:packages/coding-agent/test/suite/regressions/startup-session-rebind-duplicate-subscription.test.ts:1-82

Exported symbols: `ExtensionBindings` (`ice:packages/coding-agent/src/core/agent-session.ts:282`).

### 3. What it does

Bind extensions to current session services and reload them in the correct shutdown, invalidation, startup and resource-discovery order.

### 4. How it works

bindExtensions updates only binding fields explicitly provided, applies UI/mode and command context, replaces the prior error subscription, awaits the configured session_start event (startup by default), then runs resources_discover. Discovery results retain each contributing extension path and are converted to temporary/top-level resource metadata: source is 'extension:' plus inline marker/name, baseDir is omitted for synthetic paths or set to the extension directory. Nonempty skills/prompts/themes extend the loader, rebuild the base prompt and install it on the agent (ice:packages/coding-agent/src/core/agent-session.ts:2796-2882).

Core bindings expose source-attributed commands in extension→prompt→skill order; ordinary sendMessage/sendUserMessage launch asynchronous work and report rejections through runtime extension diagnostics. Durable custom append explicitly flushes before and after only with options.durable. Model registration, native registration and removal update the canonical runtime and refresh the selected model by same provider/ID if metadata identity changes, without selecting a fallback. Context getters remain live for model/scoped models/settings/idle/compacting/trust/signal/prompt; hasPendingMessages uses display queue count while hasPendingSteering uses the actual core queue. Abort prefers a host override, compact calls onComplete/onError asynchronously, and shutdown invokes a host handler if present (ice:packages/coding-agent/src/core/agent-session.ts:2884-3035).

reload snapshots extension flags, awaits shutdown(reason='reload'), invalidates the old runner, reloads settings and synchronizes them, resets provider registrations, reloads resources, then builds a new runner/registry preserving active names and flags while enabling all extension tools. If UI context, command actions, shutdown handler or error listener is present, it awaits beforeSessionStart, emits session_start(reason='reload'), then extends discovered resources; an abort handler alone does not satisfy this condition. resources_changed(reason='reload') is emitted at the end. This method itself does not abort/wait active operations. refreshSkills is a narrower change detector that rebuilds the prompt and emits resources_changed(reason='skills') only if changed. ReplacedSessionContext copies property descriptors so getters remain live, then overrides send methods with awaited session methods (ice:packages/coding-agent/src/core/agent-session.ts:3217-3250; ice:packages/coding-agent/src/core/agent-session.ts:4012-4019).

Command-context getSystemPromptOptions returns the same mutable options object across calls; modifications are visible on a subsequent call, as asserted in ice:packages/coding-agent/test/suite/agent-session-model-extension.test.ts:307-333. This is a live object contract, not a cloned snapshot.

### 5. Interfaces

bindExtensions fields uiContext/mode/commandContextActions/abortHandler/shutdownHandler/onError; reload({beforeSessionStart?}); refreshSkills():boolean; resources_changed reasons reload/skills; session_start reasons provided by runtime; createReplacedSessionContext awaitable sendMessage/sendUserMessage. Runner dispatch/provider registration internals remain Extensions.

```ts
export interface ExtensionBindings {
	uiContext?: ExtensionUIContext;
	mode?: ExtensionMode;
	commandContextActions?: ExtensionCommandContextActions;
	abortHandler?: () => void;
	shutdownHandler?: ShutdownHandler;
	onError?: ExtensionErrorListener;
}
```

### 6. Dependencies

`EXT-008`, `EXT-009`, `EXT-010`, `EXT-019`, `EXT-020`, `EXT-022`, `EXT-031`, `SESSION-021`, `SESSION-023`, `SESSION-028`, `SESSION-036`, `SET-003`, `SET-033`

### 7. ice-rust today

Rust CLI has extensive extension bootstrap/reload, resource discovery and dynamic tools. Startup/reload load discovered skill and prompt paths, but discovered theme_paths is only included in a debug count and is not fed into theme loading at these call sites (rust:crates/ice-cli/src/session.rs:499-539; rust:crates/ice-cli/src/session.rs:647-672; rust:crates/ice-cli/src/session.rs:1395-1443). The inspected reload_extension_resources implementation emits shutdown/rebuild-related work but no SessionStart event in its full 1166-1636 body; direct startup lifecycle uses empty event data rather than the coding reason/previous-session payload (rust:crates/ice-cli/src/session.rs:1073-1090; rust:crates/ice-cli/src/session.rs:1166-1636). Native theme support exists elsewhere; this is specifically discovery-result wiring, not 'Rust has no themes'.

### 8. Port notes

Implement session binding/reload orchestration atop the existing native extension runtime. Preserve explicit binding updates, one error subscription, live context ownership and async send semantics; carry lifecycle reason/previous-session fields and perform resource discovery after startup. Add the JSON theme catalog/loader specified by SET-033 and the TUI theme entries, then consume discovered theme paths, and document/reject reload during active mutation unless the host supplies settlement. Verify startup dynamic tools/providers, repeated bind without duplicate error listeners, flags across reload, stale contexts, abort-only binding condition and awaited replacement-context sends.

### 9. Size and priority

Estimated Rust implementation: **240-510 lines** (estimate, not measured source). **P1** - Reload and session replacement must not retain stale context or silently ignore extension-contributed resources.

## SESSION-036 - Model/thinking selection, metadata refresh and settings synchronization

### 1. Title and status

Model/thinking selection, metadata refresh and settings synchronization. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/agent-session.ts:1893-2163; ice:packages/coding-agent/src/core/agent-session.ts:2884-2903; ice:packages/coding-agent/test/agent-session-dynamic-provider.test.ts:1-180; Additional complete test-source review: ice:packages/coding-agent/test/suite/agent-session-configuration-events.test.ts:1-145; ice:packages/coding-agent/test/suite/agent-session-model-extension.test.ts:1-398

### 3. What it does

Keep current model, reasoning preference, durable history, settings and public/extension notifications consistent.

### 4. How it works

setModel awaits checkAuth and throws 'No API key for provider/id' on a falsy result, then assigns the model, appends model_change, saves default provider/model and reclamps the reasoning preference. This write sequence occurs even when provider/id equals the current model; equality suppresses only the subsequent model_changed/model_select notifications. Public model_changed removes request headers from both model payloads, while the awaited model_select extension event receives the full models and source='set'|'cycle'|'restore' (ice:packages/coding-agent/src/core/agent-session.ts:1893-1943).

cycleModel uses scoped models when any were configured, filtering them against the current available snapshot by provider + NUL + ID; otherwise it uses the available snapshot directly. Zero/one candidate returns undefined. If current is absent, currentIndex becomes zero before moving forward/backward, so forward selects candidate index one when available. A scoped explicit thinking level wins; otherwise a reasoning-capable old model preserves its current level, while switching from a nonreasoning model restores the saved preference/default medium. Selected model and thinking changes are persisted before source='cycle' notification (ice:packages/coding-agent/src/core/agent-session.ts:1951-2015; ice:packages/coding-agent/src/core/agent-session.ts:2099-2111).

setThinkingLevel clamps to supported levels, assigns state, and only on actual change appends history and emits thinking_level_changed plus asynchronous thinking_level_select. It saves the default preference when the model supports reasoning or effective level is not off, preventing a nonreasoning model from erasing a prior reasoning preference. cycleThinkingLevel returns undefined for a model without reasoning; supported levels are delegated to model capability helpers. Fast mode is a stored preference with a capability check restricted to advertised priority on the two Responses APIs. syncSettingsFromManager updates queue modes/transport and clamped thinking, emits corresponding thinking events if changed, then deduplicated configuration_changed and unconditional settings_applied; it does not append thinking history. Metadata refresh selects the same provider/ID object from the registry, reclamps and emits source='refresh' only, without model_select or fallback selection (ice:packages/coding-agent/src/core/agent-session.ts:2026-2163; ice:packages/coding-agent/src/core/agent-session.ts:2884-2903).

### 5. Interfaces

setModel, cycleModel(direction='forward'), ModelCycleResult {model, thinkingLevel, isScoped}; setThinkingLevel/cycleThinkingLevel/getAvailableThinkingLevels/supportsThinking; getFastMode/setFastMode/supportsFastMode; syncSettingsFromManager; model_changed, model_select, thinking_level_changed, thinking_level_select, configuration_changed, settings_applied. Model capability enumeration/ultra support belongs to Providers/Auth and AI; this entry owns session application.

### 6. Dependencies

`AI-005`, `AI-006`, `AUTH-006`, `AUTH-024`, `AUTH-026`, `SESSION-022`, `SESSION-023`, `SESSION-028`, `SESSION-035`

### 7. ice-rust today

Rust coding set_model/set_thinking_level directly call harness setters, which assign state without auth checks, durable model/thinking entries, clamping, saved preferences or this event family (rust:crates/ice-cli/src/agent_session.rs:188-206; rust:crates/ice-harness/src/agent_harness.rs:4936-4971). Native cycle_catalog filters a static catalog by configured names but does not implement the full live available-model cycle transaction (rust:crates/ice-cli/src/agent_session.rs:208-227). CLI settings/provider machinery can supply inputs, but is not a substitute for the session methods' consistent state transitions.

### 8. Port notes

Add model/thinking transactions to the coding session facade, reusing native model capability/auth/settings services. Persist intended user selections, keep metadata-only refresh distinct, strip private request headers from public notifications and preserve reasoning preference across nonreasoning models. Implement live cycle filtering and the source's absent-current index behavior or explicitly document a deliberate change. Verify repeated same-model writes without duplicate select events, unauthenticated selection, dynamic provider metadata replacement, scoped thinking and settings sync without history append.

### 9. Size and priority

Estimated Rust implementation: **190-400 lines** (estimate, not measured source). **P1** - Model and reasoning changes affect subsequent requests and saved-session restoration; inconsistent setters make SDK and UI behavior diverge.

## SESSION-037 - Context occupancy breakdown and tool-source attribution

### 1. Title and status

Context occupancy breakdown and tool-source attribution. **Status: missing.**

### 2. ice source

ice:packages/coding-agent/src/core/context-breakdown.ts:1-230; ice:packages/coding-agent/src/core/agent-session.ts:3884-3911; ice:packages/coding-agent/test/context-breakdown.test.ts:1-215

Exported symbols: `ContextCategoryKey` (`ice:packages/coding-agent/src/core/context-breakdown.ts:15`), `ContextBreakdownItem` (`ice:packages/coding-agent/src/core/context-breakdown.ts:27`), `ContextCategory` (`ice:packages/coding-agent/src/core/context-breakdown.ts:32`), `ContextBreakdownTool` (`ice:packages/coding-agent/src/core/context-breakdown.ts:40`), `ContextBreakdownInput` (`ice:packages/coding-agent/src/core/context-breakdown.ts:47`).

### 3. What it does

Explain which prompt components consume context while distinguishing estimated category sizes from a measured total.

### 4. How it works

computeContextBreakdown uses ceil(JavaScript string length/4) and a fixed category order: systemPrompt, contextFiles, skills, tools, promptAdditions, conversation, toolResults, extensionMessages, summaries, shell. Base system tokens subtract context-file and skill estimates with a zero floor; extension additions are max(0, currentPromptEstimate-baseEstimate). Tool size is name+description+JSON parameters, grouped by source. Positive parts aggregate by label, sort descending stably and retain the largest six per category (ice:packages/coding-agent/src/core/context-breakdown.ts:15-136; ice:packages/coding-agent/src/core/context-breakdown.ts:197-213).

Conversation separates user messages, assistant text, thinking and serialized tool calls. Tool results group by tool name, custom messages by customType, branch/compaction summaries by kind, and user shell output excludes excludeFromContext commands. Largest individual messages retain position labels and the top five positive sizes; shell labels truncate the command to 40 UTF-16 code units. Source labels prioritize builtin→'built-in', sdk→'sdk', pi-mcp-adapter marker→'mcp', npm/git source prefixes stripped, inline marker contents, then extension filename stem or directory above src/dist/lib for index entrypoints; absent source is 'other' (ice:packages/coding-agent/src/core/context-breakdown.ts:79-195).

Estimated total is the sum of categories. Used total prefers measuredTokens when present, otherwise estimated; freeTokens=max(0, window-used), and compactAtTokens is omitted when undefined. The usedTokens source expression is connector-redacted, but inspected tests assert measured precedence and null fallback. AgentSession returns undefined without a positive context window, includes the skill list only when read is among selected tools (or tools unspecified), uses a measurement only when current context is known and estimate.usageTokens>0, and exposes the configured compaction threshold only when enabled (ice:packages/coding-agent/src/core/context-breakdown.ts:214-228; ice:packages/coding-agent/test/context-breakdown.test.ts:148-168; ice:packages/coding-agent/src/core/agent-session.ts:3884-3911).

### 5. Interfaces

ContextBreakdown {contextWindow, estimatedTokens, measuredTokens:number|null, usedTokens, freeTokens, compactAtTokens?, categories, largestItems}; category {key, label, tokens, parts}; part {label, tokens}; computeContextBreakdown(input), toolSourceLabel(sourceInfo), AgentSession.getContextBreakdown(). Counts are estimates except the optional measured total; category estimates need not sum to that measurement.

### 6. Dependencies

`EXT-031`, `SESSION-010`, `SESSION-013`, `SESSION-023`, `SESSION-027`

### 7. ice-rust today

Rust's native context panel lists discovered context files, skills and prompt templates, and its footer records positive usage from non-error/non-aborted assistant responses (rust:crates/ice-cli/src/interactive_tui/rendering.rs:494-557; rust:crates/ice-cli/src/interactive_tui/events.rs:455-475). Neither is the ten-category occupancy projection, source grouping or top-item report. Repository search for ContextBreakdown/context_breakdown/largest_messages found no matching implementation; existing token-estimation and resource metadata should be reused.

### 8. Port notes

Implement a pure Rust context-breakdown projection and a coding-session accessor using UTF-16-equivalent estimates where compatibility is required. Preserve fixed category order, source-label precedence, positive-only stable top lists, measured-versus-estimated separation and post-compaction unknown state. Feed it into UI/RPC separately from discovery lists. Verify non-ASCII strings, absent/inline/package sources, disabled read/skills, excluded shell commands, measured total lower than estimates and compaction disabled.

### 9. Size and priority

Estimated Rust implementation: **210-420 lines** (estimate, not measured source). **P2** - Users need to diagnose prompt growth and tool/resource overhead without mistaking an estimate for provider measurement.

## SESSION-038 - Active-branch JSONL export and current-context assistant text

### 1. Title and status

Active-branch JSONL export and current-context assistant text. **Status: partial.**

### 2. ice source

ice:packages/coding-agent/src/core/agent-session.ts:3913-4005; Additional complete test-source review: ice:packages/coding-agent/test/suite/agent-session-runtime.test.ts:1-664; ice:packages/coding-agent/test/suite/regressions/9340-runtime-session-settlement.test.ts:1-205

### 3. What it does

Export a reopenable coding branch and return the last relevant assistant text from current context.

### 4. How it works

exportToJsonl resolves an explicit path or a timestamped session-<ISO-with-colons/dots-replaced>.jsonl against process.cwd, creates parent directories and overwrites the target synchronously. It writes a fresh v3 header with current session ID, current export timestamp and SessionManager cwd, omitting parentSession. It then exports only getBranch() root-to-current-leaf entries, retaining entry IDs but re-chaining parentId into one linear sequence; each JSON line, including the last, ends with a newline. It returns the resolved output path (ice:packages/coding-agent/src/core/agent-session.ts:3936-3973).

exportToHtml resolves the configured theme only if getThemeByName recognizes it, constructs an extension-aware tool renderer with current definitions/theme/cwd and delegates SessionManager plus current state to the HTML exporter. Detailed HTML rendering belongs to the export/UI area; the session contract is supplying this state/theme/tool renderer rather than a bare text-only transcript (ice:packages/coding-agent/src/core/agent-session.ts:3913-3934).

getLastAssistantText searches current live messages backwards, skipping only assistants that are both aborted and content-empty. It selects the first remaining assistant, concatenates text blocks without separators, trims, and returns undefined for empty text; it does not fall back to an older assistant merely because the latest has only thinking/tool blocks. Thus it follows active/compacted context rather than all historical entries, and nonempty partial aborted text remains eligible (ice:packages/coding-agent/src/core/agent-session.ts:3979-4005).

### 5. Interfaces

exportToJsonl(outputPath?):string; exportToHtml(outputPath?):Promise<string>; getLastAssistantText():string|undefined. JSONL export contains a coding v3 header and a single active branch; it is not an untyped raw-entry dump or a whole-tree archive.

### 6. Dependencies

`SESSION-013`, `SESSION-029`, `SESSION-034`

Final dependency classification. Shared contract references: `MISC-017`, `MISC-018`, `MISC-019`, `MISC-020`, `TUI-004`. These links are retained in the typed graph and do not add a prerequisite edge.

### 7. ice-rust today

Rust AgentSession export chooses a filename from session name or leaf ID when passed a directory, then export_session fetches all entries oldest-first. Its JSONL writer serializes raw Entry lines with no header or mutation wrapper; the native v4 importer explicitly requires a header, and the export test checks only raw Entry deserialization, not reopening as a session (rust:crates/ice-cli/src/agent_session.rs:347-395; rust:crates/ice-cli/src/export.rs:49-117; rust:crates/ice-cli/src/export.rs:199-208; rust:crates/ice-cli/src/export.rs:345-355). Rust last_assistant_text scans all history and falls back until any assistant has nonempty text; its helper joins text blocks with newlines without trimming (rust:crates/ice-cli/src/agent_session.rs:330-345; rust:crates/ice-cli/src/agent_session.rs:428-440).

### 8. Port notes

Expose distinct native-log archive, coding-branch JSONL and display transcript exports. Implement the coding branch/header/path contract via the v3 adapter and verify the exported file can be reopened with equivalent branch/context. Pass current renderer/theme metadata to the existing HTML/export workstream. Make getLastAssistantText query current context with the exact aborted-empty and no-fallback rules, or label a broader historical search separately. Verify abandoned branches, a thinking-only newest assistant, nonempty aborted text, multiple text blocks and custom target directories.

### 9. Size and priority

Estimated Rust implementation: **140-300 lines** (estimate, not measured source). **P2** - An export should match its advertised session format and active conversation; copy/export must not silently retrieve abandoned historical content.

## Exact supporting contracts

Source-reviewed present behavior: buildSessionContext in ice:packages/agent/src/harness/session/context.ts:25-100 has a genuine counterpart at rust:crates/ice-harness/src/session/context.rs:73-221: full-path model/thinking/active-tool state, latest-compaction context truncation, ordered transforms, retainedTail expansion, custom projectors and deferred-assistant omission. Message projection differences remain in AGENT-063–065. This does not certify the entire session area.

Cache diagnostic preservation: ice:packages/coding-agent/src/core/cache-stats.ts:1-165 and rust:crates/ice-harness/src/cache_stats.rs:1-341 both implement batch cache-miss diagnostics with previous cached context, a five-minute TTL, a 1024-token noise threshold and resets at compaction/branch summaries; Rust additionally exposes an incremental tracker. The Rust expression at line 95 was redacted by retrieval. Surrounding control flow and inspected Rust tests at rust:crates/ice-harness/src/cache_stats.rs:301-325 support the bounded behavior, but literal formula parity is not certified. No independent missing cache-diagnostics capability was established, so its ledger row is justified not-applicable rather than mapped to an unrelated gap.

Coverage boundary: all assigned source files and every source range cited by an entry were substantively read. The coordinator additionally read all 28 inventoried suite/regression files in full (7,258 lines), checked their frozen identities and reconciled their assertions; evidence/qa-session-test-closure.json records each file, hash, range and findings. Supplemental Rust caller reads remain explicitly partial where recorded in evidence/session-source-review.json. Runtime/test results, forbidden home-state contents and connector-redacted expressions are not inferred.
