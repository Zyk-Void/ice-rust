import { DestroyRef, Injectable, computed, inject, resource } from '@angular/core';
import { Api, Prefs } from './api';

const META_POLL_MS = 15_000;

/** Server metadata: sources and ClickHouse mirror health, polled. */
@Injectable({ providedIn: 'root' })
export class MetaStore {
  private readonly api = inject(Api);
  private readonly prefs = inject(Prefs);
  readonly meta = resource({
    params: () => ({ tick: this.prefs.refreshTick() }),
    loader: ({ abortSignal }) => this.api.meta(abortSignal),
  });
  readonly sources = computed(() => this.meta.value()?.sources ?? []);

  constructor() {
    const timer = setInterval(() => this.meta.reload(), META_POLL_MS);
    inject(DestroyRef).onDestroy(() => clearInterval(timer));
  }
}

/** Runs started in the selected range, with per-run totals. */
@Injectable({ providedIn: 'root' })
export class RunsStore {
  private readonly api = inject(Api);
  private readonly prefs = inject(Prefs);
  readonly list = resource({
    params: () => ({
      range: this.prefs.rangeMs(),
      source: this.prefs.source(),
      tick: this.prefs.refreshTick(),
    }),
    loader: ({ params, abortSignal }) => this.api.runs(params.range, params.source, abortSignal),
  });
  readonly runs = computed(() => (this.list.hasValue() ? this.list.value().runs : []));
}
