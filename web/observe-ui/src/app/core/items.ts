import type { ItemKind, ObserveItem, ThreadColor } from './types';

export interface KindMeta {
  kind: ItemKind;
  label: string;
  icon: string;
  /** Number key filter, as in the original dashboard tape. */
  key: string;
}

export const KINDS: KindMeta[] = [
  { kind: 'tool', label: 'Tools', icon: 'lucideWrench', key: '1' },
  { kind: 'message', label: 'Messages', icon: 'lucideMessageSquare', key: '2' },
  { kind: 'skill', label: 'Skills', icon: 'lucideSparkles', key: '3' },
  { kind: 'compaction', label: 'Compaction', icon: 'lucideMinimize2', key: '4' },
  { kind: 'retry', label: 'Retries', icon: 'lucideRotateCcw', key: '5' },
  { kind: 'queue', label: 'Queue', icon: 'lucideListOrdered', key: '6' },
  { kind: 'subagent', label: 'Subagents', icon: 'lucideGitBranch', key: '7' },
  { kind: 'memory', label: 'Memory', icon: 'lucideBrain', key: '8' },
  { kind: 'goal', label: 'Goals', icon: 'lucideTarget', key: '9' },
];

export function kindMeta(kind: ItemKind): KindMeta {
  return KINDS.find((k) => k.kind === kind) ?? KINDS[0];
}

export function threadColorVar(color: ThreadColor | string | undefined): string {
  return `var(--thread-${color ?? 'slate'})`;
}

/** Parse a JSON string field; `undefined` when absent or not JSON. */
export function parseJson<T = unknown>(text: string | undefined | null): T | undefined {
  if (!text) return undefined;
  try {
    return JSON.parse(text) as T;
  } catch {
    return undefined;
  }
}

export function prettyJson(text: string | undefined | null): string {
  if (!text) return '';
  const value = parseJson(text);
  return value === undefined ? text : JSON.stringify(value, null, 2);
}

type Args = Record<string, unknown>;

export function itemArgs(item: ObserveItem): Args | undefined {
  const args = parseJson<unknown>(item.args);
  return args && typeof args === 'object' && !Array.isArray(args) ? (args as Args) : undefined;
}

function str(value: unknown): string | undefined {
  return typeof value === 'string' && value ? value : undefined;
}

/** `path` / `file_path` argument of a file tool. */
export function argPath(item: ObserveItem): string | undefined {
  const args = itemArgs(item);
  return str(args?.['path']) ?? str(args?.['file_path']);
}

function firstLine(text: string | undefined, max = 160): string {
  const line = (text ?? '').trim().split('\n', 1)[0] ?? '';
  return line.length > max ? `${line.slice(0, max)}…` : line;
}

/** A one-line description of what an activity did. */
export function itemSummary(item: ObserveItem): string {
  if (item.kind === 'tool') {
    const args = itemArgs(item);
    const subject =
      argPath(item) ??
      str(args?.['command']) ??
      str(args?.['pattern']) ??
      str(args?.['query']) ??
      str(args?.['url']) ??
      (args ? firstLine(JSON.stringify(args), 120) : '');
    return firstLine(subject);
  }
  return firstLine(item.text ?? item.result ?? item.eventType);
}

export interface TurnUsage {
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
  cost: number;
  model?: string;
  stopReason?: string;
}

interface RawMessage {
  provider?: string;
  model?: string;
  stopReason?: string;
  usage?: {
    input?: number;
    output?: number;
    cacheRead?: number;
    cacheWrite?: number;
    cost?: { total?: number };
  };
}

/** Token use of an assistant reply, read from its raw message. */
export function turnUsage(item: ObserveItem): TurnUsage | undefined {
  if (item.kind !== 'message' || item.role !== 'assistant') return undefined;
  const raw = parseJson<RawMessage>(item.raw) ?? {};
  const usage = raw.usage ?? {};
  return {
    input: usage.input ?? 0,
    output: usage.output ?? 0,
    cacheRead: usage.cacheRead ?? 0,
    cacheWrite: usage.cacheWrite ?? 0,
    cost: usage.cost?.total ?? 0,
    model: raw.provider && raw.model ? `${raw.provider}/${raw.model}` : raw.model,
    stopReason: raw.stopReason,
  };
}

/** Share of prompt tokens served from cache. */
export function cacheHit(t: {
  input: number;
  cacheRead: number;
  cacheWrite: number;
}): number | null {
  const prompt = t.input + t.cacheRead + t.cacheWrite;
  return prompt ? t.cacheRead / prompt : null;
}

export function isFailed(item: ObserveItem): boolean {
  if (item.state === 'error') return true;
  const stop = turnUsage(item)?.stopReason;
  return stop === 'error' || stop === 'aborted';
}

/** Lowercase text searched by the tape filter. */
export function haystack(item: ObserveItem): string {
  return [
    item.kind,
    item.toolName,
    item.threadName,
    item.state,
    item.role,
    item.eventType,
    item.text?.slice(0, 20_000),
    item.args?.slice(0, 20_000),
    item.result?.slice(0, 20_000),
  ]
    .filter(Boolean)
    .join('\n')
    .toLowerCase();
}

/** A stable label for comparing what two runs did, step by step. */
export function actionSignature(item: ObserveItem): string {
  if (item.kind === 'tool') {
    const args = itemArgs(item);
    const subject =
      argPath(item) ??
      str(args?.['command'])?.split(/\s+/).slice(0, 2).join(' ') ??
      str(args?.['pattern']) ??
      '';
    return `${item.toolName ?? 'tool'} ${subject}`.trim();
  }
  if (item.kind === 'message') return `${item.role ?? 'message'} message`;
  return item.kind;
}
