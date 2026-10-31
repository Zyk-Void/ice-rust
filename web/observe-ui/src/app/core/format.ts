// Number, time and size formatting shared by every view.

const compact = new Intl.NumberFormat(undefined, { notation: 'compact', maximumFractionDigits: 1 });
const whole = new Intl.NumberFormat();

/** 1234 -> "1.2K"; exact below 10 000. */
export function fmtCount(n: number | null | undefined): string {
  if (n === null || n === undefined || !Number.isFinite(n)) return '–';
  return Math.abs(n) < 10_000 ? whole.format(n) : compact.format(n);
}

export function fmtExact(n: number | null | undefined): string {
  return n === null || n === undefined ? '–' : whole.format(n);
}

export function fmtCost(n: number | null | undefined): string {
  if (n === null || n === undefined || !Number.isFinite(n)) return '–';
  if (n === 0) return '$0';
  if (Math.abs(n) < 0.01) return `$${n.toFixed(4)}`;
  if (Math.abs(n) < 100) return `$${n.toFixed(2)}`;
  return `$${compact.format(n)}`;
}

export function fmtPercent(ratio: number | null | undefined, digits = 0): string {
  if (ratio === null || ratio === undefined || !Number.isFinite(ratio)) return '–';
  return `${(ratio * 100).toFixed(digits)}%`;
}

/** 850 -> "850ms", 2300 -> "2.3s", 65000 -> "1m 05s", 7_200_000 -> "2h 00m". */
export function fmtMs(ms: number | null | undefined): string {
  if (ms === null || ms === undefined || !Number.isFinite(ms)) return '–';
  if (ms < 1000) return `${Math.round(ms)}ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(ms < 10_000 ? 1 : 0)}s`;
  const s = Math.round(ms / 1000);
  if (s < 3600) return `${Math.floor(s / 60)}m ${String(s % 60).padStart(2, '0')}s`;
  const m = Math.round(s / 60);
  if (m < 48 * 60) return `${Math.floor(m / 60)}h ${String(m % 60).padStart(2, '0')}m`;
  return `${Math.floor(m / 1440)}d ${Math.floor((m % 1440) / 60)}h`;
}

const rtf = new Intl.RelativeTimeFormat(undefined, { numeric: 'auto', style: 'narrow' });

export function fmtAgo(ts: number | null | undefined, now = Date.now()): string {
  if (!ts) return '–';
  const s = Math.round((ts - now) / 1000);
  const abs = Math.abs(s);
  if (abs < 45) return 'just now';
  if (abs < 3600) return rtf.format(Math.round(s / 60), 'minute');
  if (abs < 86_400) return rtf.format(Math.round(s / 3600), 'hour');
  return rtf.format(Math.round(s / 86_400), 'day');
}

const dateTime = new Intl.DateTimeFormat(undefined, {
  month: 'short',
  day: 'numeric',
  hour: '2-digit',
  minute: '2-digit',
});
const timeOnly = new Intl.DateTimeFormat(undefined, {
  hour: '2-digit',
  minute: '2-digit',
  second: '2-digit',
  hourCycle: 'h23',
});

export function fmtDateTime(ts: number | null | undefined): string {
  return ts ? dateTime.format(ts) : '–';
}

export function fmtTime(ts: number | null | undefined): string {
  return ts ? timeOnly.format(ts) : '–';
}

/** Axis label for a series bucket of width `bucketMs`. */
export function fmtBucket(ts: number, bucketMs: number): string {
  const d = new Date(ts);
  if (bucketMs < 86_400_000) {
    return d.toLocaleString(undefined, { weekday: 'short', hour: '2-digit' });
  }
  return d.toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
}

/** Relative change; `null` when there is no baseline. */
export function delta(current: number, previous: number): number | null {
  if (!previous) return current ? null : 0;
  return (current - previous) / previous;
}

export function fmtBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}

export function shortId(id: string, size = 8): string {
  const tail = id.split('-').pop() ?? id;
  return tail.length > size ? tail.slice(0, size) : tail;
}

/** Collapse whitespace and cut to `max` characters. */
export function oneLine(text: string, max = 160): string {
  const line = text.split(/\s+/).filter(Boolean).join(' ');
  return line.length > max ? `${line.slice(0, max)}…` : line;
}

/** Server window for "recently live" runs (LIVE_RUN_ACTIVITY_MS). */
export const LIVE_WINDOW_MS = 600_000;

export interface RunLike {
  runId: string;
  sessionName?: string;
  prompt?: string | null;
  status: string;
  startedAt: number;
  lastActivity?: number | null;
}

/** Session name, else the first prompt, else the run id. */
export function runLabel(run: RunLike): string {
  return run.sessionName || run.prompt || `run ${shortId(run.runId)}`;
}

/**
 * `live` while a running run has recent activity; a running run that went
 * quiet probably ended without finishing (a crash or kill), so it is `stale`.
 */
export function runActivity(run: RunLike, now = Date.now()): 'live' | 'stale' | 'finished' {
  if (run.status !== 'running') return 'finished';
  return now - (run.lastActivity ?? run.startedAt) < LIVE_WINDOW_MS ? 'live' : 'stale';
}

/** Last two path segments. */
export function shortPath(path: string): string {
  const parts = path.split('/').filter(Boolean);
  return parts.length <= 2 ? path : `…/${parts.slice(-2).join('/')}`;
}
