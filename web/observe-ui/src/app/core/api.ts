import { Injectable, computed, effect, signal } from '@angular/core';
import type { Meta, ObserveState, Overview, RunList } from './types';

/** Fetch JSON from the dashboard server. The auth cookie rides along. */
export async function getJson<T>(path: string, signal?: AbortSignal): Promise<T> {
  const response = await fetch(path, { credentials: 'same-origin', signal });
  const text = await response.text();
  if (!response.ok) {
    let message = text;
    try {
      message = (JSON.parse(text) as { error?: string }).error ?? text;
    } catch {
      // Plain-text error body.
    }
    throw new Error(`${response.status}: ${message}`);
  }
  return JSON.parse(text) as T;
}

function query(params: Record<string, string | number | undefined>): string {
  const search = new URLSearchParams();
  for (const [key, value] of Object.entries(params)) {
    if (value !== undefined && value !== '') search.set(key, String(value));
  }
  const text = search.toString();
  return text ? `?${text}` : '';
}

export const RANGES = [
  { id: '24h', label: '24h', ms: 24 * 3_600_000 },
  { id: '7d', label: '7d', ms: 7 * 86_400_000 },
  { id: '30d', label: '30d', ms: 30 * 86_400_000 },
  { id: '90d', label: '90d', ms: 90 * 86_400_000 },
] as const;
export type RangeId = (typeof RANGES)[number]['id'];

function stored(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

function store(key: string, value: string) {
  try {
    localStorage.setItem(key, value);
  } catch {
    // Storage unavailable (private window, blocked site data).
  }
}

/** Viewer preferences shared by every page. */
@Injectable({ providedIn: 'root' })
export class Prefs {
  readonly range = signal<RangeId>((stored('icers-observe-range') as RangeId) || '7d');
  /** `''` picks the server default (ClickHouse when configured). */
  readonly source = signal<string>(stored('icers-observe-source') ?? '');
  readonly dark = signal<boolean>(
    stored('icers-observe-theme')
      ? stored('icers-observe-theme') === 'dark'
      : window.matchMedia?.('(prefers-color-scheme: dark)').matches === true,
  );
  /** Bumped to refetch every page's data. */
  readonly refreshTick = signal(0);
  readonly rangeMs = computed(() => RANGES.find((r) => r.id === this.range())?.ms ?? RANGES[1].ms);

  constructor() {
    effect(() => {
      document.documentElement.classList.toggle('dark', this.dark());
      store('icers-observe-theme', this.dark() ? 'dark' : 'light');
    });
    effect(() => store('icers-observe-range', this.range()));
    effect(() => store('icers-observe-source', this.source()));
  }

  refresh() {
    this.refreshTick.update((n) => n + 1);
  }
}

@Injectable({ providedIn: 'root' })
export class Api {
  meta(signal?: AbortSignal) {
    return getJson<Meta>('api/meta', signal);
  }

  overview(rangeMs: number, source: string, signal?: AbortSignal) {
    const until = Date.now();
    return getJson<Overview>(
      'api/overview' +
        query({
          since: until - rangeMs,
          until,
          tz: -new Date().getTimezoneOffset(),
          source,
        }),
      signal,
    );
  }

  runs(sinceMs: number, source: string, signal?: AbortSignal) {
    return getJson<RunList>('api/runs' + query({ since: Date.now() - sinceMs, source }), signal);
  }

  runState(runId: string, signal?: AbortSignal) {
    return getJson<ObserveState>('state' + query({ runId }), signal);
  }
}
