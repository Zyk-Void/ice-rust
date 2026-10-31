import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  computed,
  inject,
  signal,
} from '@angular/core';
import { RouterLink } from '@angular/router';
import { HlmBadgeImports } from '@spartan-ng/helm/badge';
import { HlmCardImports } from '@spartan-ng/helm/card';
import { HlmProgressImports } from '@spartan-ng/helm/progress';
import { HlmResizableImports } from '@spartan-ng/helm/resizable';
import { HlmToggleGroupImports } from '@spartan-ng/helm/toggle-group';
import { fmtAgo, fmtCost, fmtCount, fmtPercent, shortId } from '../core/format';
import { threadColorVar } from '../core/items';
import { Live, itemKey } from '../core/live';
import { Inspector } from '../shared/inspector';
import { Tape } from '../shared/tape';

const BINS = 60;

@Component({
  selector: 'app-live',
  imports: [
    RouterLink,
    HlmCardImports,
    HlmBadgeImports,
    HlmProgressImports,
    HlmResizableImports,
    HlmToggleGroupImports,
    Tape,
    Inspector,
  ],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <div class="flex h-[calc(100vh-3rem)] flex-col gap-3 p-4 md:p-6">
      <div class="flex flex-wrap items-center gap-3">
        <h1 class="text-xl font-semibold tracking-tight">Live</h1>
        <span
          hlmBadge
          [variant]="live.status() === 'open' ? 'default' : 'secondary'"
          class="gap-1.5"
        >
          <span
            class="size-1.5 rounded-full"
            [class]="live.status() === 'open' ? 'bg-success animate-pulse' : 'bg-muted-foreground'"
          ></span>
          {{ live.status() === 'open' ? 'streaming' : live.status() }}
        </span>
        <span class="text-muted-foreground text-sm">
          {{ live.runs().length }} live run{{ live.runs().length === 1 ? '' : 's' }} ·
          {{ live.items().length }} activities
        </span>
        <hlm-toggle-group
          type="single"
          size="sm"
          variant="outline"
          class="ml-auto"
          [value]="window()"
          (valueChange)="window.set(+($event ?? 10) || 10)"
          aria-label="Pulse window"
        >
          @for (m of windows; track m) {
            <button hlmToggleGroupItem [value]="m">{{ m }}m</button>
          }
        </hlm-toggle-group>
      </div>

      <div class="grid gap-3 lg:grid-cols-[minmax(0,2fr)_minmax(0,3fr)]">
        @if (live.header(); as h) {
          <section hlmCard class="gap-2 py-3">
            <div hlmCardContent class="flex flex-col gap-2 px-4">
              <div class="flex items-center justify-between gap-2">
                <a [routerLink]="['/runs', h.runId]" class="truncate font-medium hover:underline">
                  {{ h.sessionName || h.mainThreadName }}
                </a>
                <span class="text-muted-foreground font-mono text-xs">{{ short(h.runId) }}</span>
              </div>
              <p class="text-muted-foreground truncate text-xs">
                {{ h.model ?? 'unknown model' }} · thinking {{ h.thinkingLevel ?? 'off' }} · started
                {{ ago(h.startedAt) }}
              </p>
              @if (h.context; as c) {
                <div class="flex items-center gap-2 text-xs">
                  <span class="text-muted-foreground w-14">Context</span>
                  <hlm-progress class="h-1.5 flex-1" [value]="c.percent ?? 0">
                    <hlm-progress-indicator />
                  </hlm-progress>
                  <span class="tabular w-28 text-right"
                    >{{ count(c.tokens ?? 0) }} / {{ count(c.contextWindow) }}</span
                  >
                </div>
              }
              <dl class="grid grid-cols-4 gap-2 text-xs">
                @for (cell of usage(); track cell[0]) {
                  <div>
                    <dt class="text-muted-foreground">{{ cell[0] }}</dt>
                    <dd class="tabular font-medium">{{ cell[1] }}</dd>
                  </div>
                }
              </dl>
            </div>
          </section>
        }
        <section hlmCard class="gap-1 py-3">
          <div hlmCardContent class="px-4">
            <div class="text-muted-foreground mb-1 flex items-center justify-between text-xs">
              <span>Activity pulse · last {{ window() }} min</span>
              <span class="flex gap-3">
                @for (lane of pulse().lanes; track lane.key) {
                  <span class="flex items-center gap-1"
                    ><span class="size-2 rounded-full" [style.background]="lane.color"></span
                    >{{ lane.name }}</span
                  >
                }
              </span>
            </div>
            <svg
              viewBox="0 0 600 64"
              preserveAspectRatio="none"
              class="h-16 w-full"
              role="img"
              [attr.aria-label]="'Activity over the last ' + window() + ' minutes'"
            >
              @for (bar of pulse().bars; track bar.key) {
                <rect
                  [attr.x]="bar.x"
                  [attr.y]="bar.y"
                  [attr.width]="bar.w"
                  [attr.height]="bar.h"
                  [attr.fill]="bar.color"
                  rx="1.5"
                >
                  <title>{{ bar.title }}</title>
                </rect>
              }
              <line x1="0" x2="600" y1="63.5" y2="63.5" stroke="var(--border)" />
            </svg>
          </div>
        </section>
      </div>

      <section hlmCard class="min-h-0 flex-1 overflow-hidden py-0">
        <hlm-resizable-group class="h-full">
          <hlm-resizable-panel [defaultSize]="selectedItem() ? 60 : 100" [minSize]="35">
            <app-tape [items]="live.items()" [(selected)]="selected" [(follow)]="follow" />
          </hlm-resizable-panel>
          @if (selectedItem(); as item) {
            <hlm-resizable-handle withHandle />
            <hlm-resizable-panel [defaultSize]="40" [minSize]="25">
              <app-inspector [item]="item" [context]="live.items()" (closed)="selected.set(null)" />
            </hlm-resizable-panel>
          }
        </hlm-resizable-group>
      </section>
    </div>
  `,
})
export class LivePage {
  protected readonly live = inject(Live);
  protected readonly selected = signal<string | null>(null);
  protected readonly follow = signal<boolean | undefined>(true);
  protected readonly window = signal(10);
  protected readonly windows = [1, 3, 5, 10];
  protected readonly count = fmtCount;
  protected readonly ago = fmtAgo;
  protected readonly short = shortId;
  private readonly now = signal(Date.now());

  constructor() {
    this.live.connect();
    const timer = setInterval(() => this.now.set(Date.now()), 2_000);
    inject(DestroyRef).onDestroy(() => clearInterval(timer));
  }

  protected readonly selectedItem = computed(() =>
    this.live.items().find((i) => itemKey(i) === this.selected()),
  );

  protected readonly usage = computed(() => {
    const u = this.live.header()?.usage;
    if (!u) return [];
    const prompt = u.input + u.cacheRead + u.cacheWrite;
    return [
      ['Input', fmtCount(u.input)],
      ['Output', fmtCount(u.output)],
      ['Cache hit', fmtPercent(prompt ? u.cacheRead / prompt : null)],
      ['Cost', fmtCost(u.cost)],
    ];
  });

  /** Epoch-anchored bins, one stacked segment per live run. */
  protected readonly pulse = computed(() => {
    const span = this.window() * 60_000;
    const size = span / BINS;
    const end = Math.ceil(this.now() / size) * size;
    const start = end - span;
    const runs = new Map<string, { name: string; color: string; bins: number[] }>();
    for (const item of this.live.items()) {
      if (item.ts < start || item.ts >= end) continue;
      const key = item.runId || item.mainThreadId;
      let lane = runs.get(key);
      if (!lane) {
        lane = {
          name: item.threadKind === 'main' ? item.threadName : shortId(key),
          color: threadColorVar(item.threadColor),
          bins: new Array(BINS).fill(0),
        };
        runs.set(key, lane);
      }
      lane.bins[Math.floor((item.ts - start) / size)]++;
    }
    const lanes = [...runs.entries()].map(([key, lane], i) => ({
      key,
      ...lane,
      color: runs.size > 1 ? `var(--chart-${(i % 8) + 1})` : lane.color,
    }));
    const totals = Array.from({ length: BINS }, (_, b) => lanes.reduce((n, l) => n + l.bins[b], 0));
    const max = Math.max(1, ...totals);
    const w = 600 / BINS;
    const bars: Array<{
      key: string;
      x: number;
      y: number;
      w: number;
      h: number;
      color: string;
      title: string;
    }> = [];
    for (let b = 0; b < BINS; b++) {
      let y = 64;
      for (const lane of lanes) {
        const n = lane.bins[b];
        if (!n) continue;
        const h = (n / max) * 60;
        y -= h;
        bars.push({
          key: `${lane.key}:${b}`,
          x: b * w + 1,
          y,
          w: w - 2,
          h: Math.max(1, h - 1),
          color: lane.color,
          title: `${lane.name}: ${n} events`,
        });
      }
    }
    return { lanes: lanes.length > 1 ? lanes : [], bars };
  });
}
