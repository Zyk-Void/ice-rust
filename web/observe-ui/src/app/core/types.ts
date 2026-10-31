// Wire types served by `icers --observe`. They mirror the Rust DTOs in
// crates/ice-cli/src/observe/{types,analytics,clickhouse}.rs (camelCase).

export type ItemKind =
  'message' | 'tool' | 'subagent' | 'compaction' | 'retry' | 'skill' | 'queue' | 'memory' | 'goal';

export type ThreadColor =
  'blue' | 'violet' | 'cyan' | 'green' | 'amber' | 'rose' | 'orange' | 'indigo' | 'slate';

export interface ThreadIdentity {
  threadKey: string;
  threadId: string;
  parentThreadId?: string;
  mainThreadId: string;
  threadKind: 'main' | 'subagent';
  threadName: string;
  threadColor: ThreadColor;
}

export interface ObserveThread extends ThreadIdentity {
  runId?: string;
  role?: string;
  profileColor?: string;
}

export interface ObserveItem extends ThreadIdentity {
  runId: string;
  id: number;
  ts: number;
  kind: ItemKind;
  eventType?: string;
  role?: 'user' | 'assistant' | 'custom';
  text?: string;
  raw?: string;
  toolCallId?: string;
  toolName?: string;
  state?: 'running' | 'done' | 'error';
  args?: string;
  result?: string;
  durationMs?: number;
  updates?: number;
  delegationToolName?: string;
}

export interface Usage {
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
  cost: number;
}

export interface ObserveHeader {
  runId: string;
  sessionId: string;
  mainThreadId: string;
  mainThreadName: string;
  sessionName?: string;
  model?: string | null;
  thinkingLevel?: string | null;
  context?: { tokens?: number | null; contextWindow: number; percent?: number | null } | null;
  usage: Usage;
  startedAt: number;
}

export interface ObserveState extends ObserveHeader {
  items: ObserveItem[];
  lanes: unknown[];
  threads: ObserveThread[];
}

export interface RunSummary {
  runId: string;
  sessionId: string;
  mainThreadId: string;
  sessionName?: string;
  model?: string | null;
  thinkingLevel?: string | null;
  startedAt: number;
  endedAt?: number;
  status: 'running' | 'finished';
}

export interface CombinedState extends ObserveState {
  view: string;
  liveRunId: string;
  runs: RunSummary[];
}

export interface NamedCount {
  name: string;
  count: number;
}

export interface TokenTotals {
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
  cost: number;
}

export interface RunRow extends RunSummary, TokenTotals {
  lastActivity?: number | null;
  durationMs?: number | null;
  turns: number;
  turnErrors: number;
  toolCalls: number;
  toolErrors: number;
  subagentItems: number;
  files: number;
  models: string[];
  topTools: NamedCount[];
  /** The start of the first user prompt. */
  prompt?: string | null;
}

export interface SourceInfo {
  source: string;
  sourceError?: string;
}

export interface RunList extends SourceInfo {
  since: number;
  until: number;
  runs: RunRow[];
}

export interface Totals extends TokenTotals {
  runs: number;
  turns: number;
  turnErrors: number;
  toolCalls: number;
  toolErrors: number;
  subagentItems: number;
  filesChanged: number;
  medianRunMs?: number | null;
  p95ToolMs?: number | null;
}

export interface SeriesPoint extends TokenTotals {
  t: number;
  runs: number;
  turns: number;
  toolCalls: number;
  toolErrors: number;
}

export interface ModelStat extends TokenTotals {
  model: string;
  runs: number;
  turns: number;
  errors: number;
}

export interface ToolStat {
  tool: string;
  calls: number;
  errors: number;
  runs: number;
  p50Ms?: number | null;
  p95Ms?: number | null;
  maxMs?: number | null;
  totalMs: number;
}

export interface FileStat {
  path: string;
  edits: number;
  writes: number;
  runs: number;
  lastTs: number;
}

export interface Overview extends SourceInfo {
  since: number;
  until: number;
  bucketMs: number;
  totals: Totals;
  previous: Totals;
  series: SeriesPoint[];
  models: ModelStat[];
  tools: ToolStat[];
  kinds: Record<string, number>;
  stopReasons: Record<string, number>;
  heatmap: number[][];
  files: FileStat[];
}

export interface ClickHouseStatus {
  url: string;
  database: string;
  connected?: boolean | null;
  lastError?: string | null;
  lastSyncAt?: number | null;
  eventsSent: number;
}

export interface Meta {
  version: string;
  liveRunId: string;
  retentionMs: number;
  sources: string[];
  defaultSource?: string | null;
  clickhouse?: ClickHouseStatus | null;
}
