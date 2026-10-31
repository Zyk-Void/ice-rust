import { Injectable, computed, signal } from '@angular/core';
import type { CombinedState, ObserveHeader, ObserveItem, ObserveThread, RunSummary } from './types';

/** Keep at most this many live activities (the server sends up to 1000). */
export const LIVE_LIMIT = 2_000;

export function itemKey(item: Pick<ObserveItem, 'runId' | 'mainThreadId' | 'id'>): string {
  return `${item.runId || item.mainThreadId}:${item.id}`;
}

export type LiveStatus = 'connecting' | 'open' | 'closed';

/**
 * The `/events` Server-Sent Events stream: a combined snapshot of every
 * recently live run, then incremental activity, thread, header and run-list
 * frames. Connects on first use and reconnects with the browser's default
 * EventSource retry.
 */
@Injectable({ providedIn: 'root' })
export class Live {
  private source?: EventSource;
  private readonly itemMap = signal(new Map<string, ObserveItem>());
  readonly status = signal<LiveStatus>('closed');
  readonly header = signal<ObserveHeader | null>(null);
  readonly runs = signal<RunSummary[]>([]);
  readonly threads = signal<ObserveThread[]>([]);
  readonly liveRunId = signal<string>('');
  readonly items = computed(() =>
    [...this.itemMap().values()].sort((a, b) => a.ts - b.ts || a.id - b.id),
  );

  connect() {
    if (this.source) return;
    this.status.set('connecting');
    const source = new EventSource('events');
    this.source = source;
    source.onopen = () => this.status.set('open');
    source.onerror = () =>
      this.status.set(source.readyState === EventSource.CLOSED ? 'closed' : 'connecting');
    const on = <T>(name: string, handle: (data: T) => void) =>
      source.addEventListener(name, (event) => {
        try {
          handle(JSON.parse((event as MessageEvent<string>).data) as T);
        } catch {
          // A malformed frame is dropped; the next snapshot repairs state.
        }
      });
    on<CombinedState>('snapshot', (snapshot) => {
      const map = new Map<string, ObserveItem>();
      for (const item of snapshot.items.slice(-LIVE_LIMIT)) map.set(itemKey(item), item);
      this.itemMap.set(map);
      this.threads.set(snapshot.threads);
      this.runs.set(snapshot.runs);
      this.liveRunId.set(snapshot.liveRunId);
      const {
        items: _items,
        lanes: _lanes,
        threads: _threads,
        runs: _runs,
        view: _view,
        liveRunId: _live,
        ...header
      } = snapshot;
      this.header.set(header);
    });
    const upsert = (item: ObserveItem) => {
      this.itemMap.update((map) => {
        const next = new Map(map);
        next.set(itemKey(item), item);
        while (next.size > LIVE_LIMIT) next.delete(next.keys().next().value!);
        return next;
      });
    };
    on<ObserveItem>('activity', upsert);
    on<ObserveItem>('activity_update', upsert);
    on<ObserveThread>('thread', (thread) =>
      this.threads.update((list) => [
        ...list.filter((t) => !(t.threadKey === thread.threadKey && t.runId === thread.runId)),
        thread,
      ]),
    );
    on<ObserveHeader>('header', (header) => this.header.set(header));
    on<RunSummary[]>('runs', (runs) => this.runs.set(runs));
  }

  disconnect() {
    this.source?.close();
    this.source = undefined;
    this.status.set('closed');
  }
}
