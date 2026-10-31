import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  ElementRef,
  computed,
  inject,
  input,
  model,
  signal,
} from '@angular/core';
import { HlmSwitchImports } from '@spartan-ng/helm/switch';
import { HlmToggleGroupImports } from '@spartan-ng/helm/toggle-group';
import { fmtMs } from '../core/format';
import { isFailed, itemSummary, threadColorVar } from '../core/items';
import { itemKey } from '../core/live';
import type { ObserveItem } from '../core/types';

const ROW = 16;
const LANE_GAP = 14;
const LABEL = 132;
const AXIS = 22;
const TOOL_SLOTS = 7;
/** Gaps longer than this fold into a fixed-width break when compressing. */
const IDLE_MS = 20_000;
const BREAK_PX = 22;
/** Minimum time width of an activity segment. */
const MIN_SEGMENT_MS = 1_000;

/** A stretch of activity and where it sits on the x axis. */
interface Segment {
  start: number;
  end: number;
  x: number;
  w: number;
}

interface Bar {
  key: string;
  x: number;
  y: number;
  w: number;
  color: string;
  failed: boolean;
  running: boolean;
  title: string;
}

interface Tick {
  key: string;
  x: number;
  y: number;
  kind: 'user' | 'assistant' | 'event';
  title: string;
}

interface Lane {
  key: string;
  name: string;
  color: string;
  y: number;
  height: number;
}

/**
 * When each tool call ran, per agent thread. Overlapping calls stack into
 * extra rows; replies and prompts are ticks on the thread's first row.
 */
@Component({
  selector: 'app-waterfall',
  imports: [HlmToggleGroupImports, HlmSwitchImports],
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { class: 'flex min-h-0 flex-col gap-3' },
  template: `
    <div class="flex flex-wrap items-center gap-3">
      <hlm-toggle-group
        type="single"
        size="sm"
        variant="outline"
        [value]="zoom()"
        (valueChange)="zoom.set(+($event ?? 1) || 1)"
        aria-label="Zoom"
      >
        @for (z of zooms; track z) {
          <button hlmToggleGroupItem [value]="z">{{ z === 1 ? 'Fit' : z + '×' }}</button>
        }
      </hlm-toggle-group>
      <label class="text-muted-foreground flex items-center gap-2 text-xs">
        <hlm-switch
          [checked]="compress()"
          (checkedChange)="compress.set($event)"
          aria-label="Compress idle time"
        />
        Compress idle
      </label>
      <ul
        class="text-muted-foreground flex flex-wrap gap-x-3 gap-y-1 text-xs"
        aria-label="Tool colors"
      >
        @for (t of legend(); track t.name) {
          <li class="flex items-center gap-1.5">
            <span class="size-2.5 rounded-[3px]" [style.background]="t.color"></span>{{ t.name }}
          </li>
        }
      </ul>
      <span class="text-muted-foreground ml-auto text-xs">span {{ span() }}</span>
    </div>
    <div class="bg-card overflow-x-auto rounded-md border">
      <svg
        [attr.width]="svgWidth()"
        [attr.height]="layout().height + AXIS"
        class="block select-none"
        role="img"
        aria-label="Tool call waterfall"
      >
        <defs>
          <pattern
            id="idle-hatch"
            width="6"
            height="6"
            patternUnits="userSpaceOnUse"
            patternTransform="rotate(45)"
          >
            <rect width="6" height="6" fill="var(--muted)" />
            <line x1="0" y1="0" x2="0" y2="6" stroke="var(--border)" stroke-width="2" />
          </pattern>
        </defs>
        @for (gap of gaps(); track gap.x) {
          <g>
            <title>{{ gap.title }}</title>
            <rect
              [attr.x]="gap.x + 3"
              [attr.y]="AXIS - 2"
              [attr.width]="BREAK_PX - 6"
              [attr.height]="layout().height + 2"
              fill="url(#idle-hatch)"
            />
            <text
              [attr.x]="gap.x + BREAK_PX / 2"
              y="13"
              text-anchor="middle"
              class="fill-muted-foreground text-[9px]"
            >
              ⋯
            </text>
          </g>
        }
        @for (tick of axis(); track tick.x) {
          <line
            [attr.x1]="tick.x"
            [attr.x2]="tick.x"
            [attr.y1]="AXIS - 4"
            [attr.y2]="layout().height + AXIS"
            stroke="var(--border)"
          />
          <text [attr.x]="tick.x + 3" y="13" class="fill-muted-foreground text-[10px]">
            {{ tick.label }}
          </text>
        }
        <g [attr.transform]="'translate(0,' + AXIS + ')'">
          @for (lane of layout().lanes; track lane.key) {
            <rect
              x="0"
              [attr.y]="lane.y - 4"
              [attr.width]="svgWidth()"
              [attr.height]="lane.height + 8"
              fill="var(--muted)"
              opacity="0.35"
            />
            <rect
              x="0"
              [attr.y]="lane.y - 4"
              width="3"
              [attr.height]="lane.height + 8"
              [attr.fill]="lane.color"
            />
            <text x="10" [attr.y]="lane.y + 11" class="fill-foreground text-[11px] font-medium">
              {{ lane.name }}
            </text>
          }
          @for (tick of layout().ticks; track tick.key) {
            <g class="cursor-pointer" (click)="selected.set(tick.key)">
              <title>{{ tick.title }}</title>
              @if (tick.kind === 'user') {
                <path
                  [attr.d]="'M' + tick.x + ',' + (tick.y + 2) + 'l4,7h-8z'"
                  fill="var(--foreground)"
                />
              } @else if (tick.kind === 'assistant') {
                <circle
                  [attr.cx]="tick.x"
                  [attr.cy]="tick.y + 7"
                  r="3.5"
                  fill="var(--background)"
                  stroke="var(--foreground)"
                  stroke-width="1.5"
                />
              } @else {
                <rect
                  [attr.x]="tick.x - 1"
                  [attr.y]="tick.y"
                  width="2"
                  height="14"
                  fill="var(--muted-foreground)"
                />
              }
            </g>
          }
          @for (bar of layout().bars; track bar.key) {
            <g class="cursor-pointer" (click)="selected.set(bar.key)">
              <title>{{ bar.title }}</title>
              <rect
                [attr.x]="bar.x"
                [attr.y]="bar.y + 1"
                [attr.width]="bar.w"
                [attr.height]="ROW - 3"
                rx="3"
                [attr.fill]="bar.color"
                [attr.opacity]="bar.running ? 0.55 : 0.9"
                [attr.stroke]="
                  bar.key === selected()
                    ? 'var(--foreground)'
                    : bar.failed
                      ? 'var(--destructive)'
                      : 'none'
                "
                [attr.stroke-width]="bar.key === selected() || bar.failed ? 2 : 0"
                [attr.stroke-dasharray]="bar.failed && bar.key !== selected() ? '3 2' : null"
              />
            </g>
          }
        </g>
      </svg>
    </div>
    <p class="text-muted-foreground text-xs">
      Bars are tool calls (dashed outline: failed, faded: still running). ▲ prompt · ○ reply · |
      other events. Hatched breaks are idle gaps over 20s. Click to inspect.
    </p>
  `,
})
export class Waterfall {
  readonly items = input.required<ObserveItem[]>();
  readonly selected = model<string | null>(null);
  protected readonly zoom = signal(1);
  protected readonly compress = signal(true);
  protected readonly BREAK_PX = BREAK_PX;
  protected readonly zooms = [1, 2, 4, 8, 16];
  protected readonly AXIS = AXIS;
  protected readonly ROW = ROW;
  private readonly width = signal(900);

  constructor() {
    const host = inject(ElementRef<HTMLElement>).nativeElement as HTMLElement;
    const observer = new ResizeObserver(([entry]) =>
      this.width.set(Math.max(480, entry.contentRect.width - 2)),
    );
    observer.observe(host);
    inject(DestroyRef).onDestroy(() => observer.disconnect());
  }

  private readonly extent = computed(() => {
    const items = this.items();
    if (!items.length) return { t0: 0, t1: 1 };
    const t0 = Math.min(...items.map((i) => i.ts));
    const t1 = Math.max(...items.map((i) => i.ts + (i.durationMs ?? 0)), t0 + 1000);
    return { t0, t1 };
  });
  protected readonly span = computed(() => fmtMs(this.extent().t1 - this.extent().t0));
  protected readonly svgWidth = computed(() => Math.round(this.width() * this.zoom()));
  /** Activity stretches; with `compress`, idle gaps between them shrink to breaks. */
  private readonly segments = computed<Segment[]>(() => {
    const { t0, t1 } = this.extent();
    const plot = this.svgWidth() - LABEL - 12;
    if (!this.compress()) return [{ start: t0, end: t1, x: LABEL, w: plot }];
    const spans = this.items()
      .map((i) => [i.ts, i.state === 'running' ? t1 : i.ts + (i.durationMs ?? 0)] as const)
      .sort((a, b) => a[0] - b[0]);
    const merged: Array<[number, number]> = [];
    for (const [start, end] of spans) {
      const last = merged.at(-1);
      if (last && start - last[1] <= IDLE_MS) last[1] = Math.max(last[1], end);
      else merged.push([start, Math.max(start + MIN_SEGMENT_MS, end)]);
    }
    if (!merged.length) return [{ start: t0, end: t1, x: LABEL, w: plot }];
    const active = merged.reduce((n, [s, e]) => n + (e - s), 0);
    const perMs = Math.max(0, plot - (merged.length - 1) * BREAK_PX) / active;
    let x = LABEL;
    return merged.map(([start, end]) => {
      const segment = { start, end, x, w: (end - start) * perMs };
      x += segment.w + BREAK_PX;
      return segment;
    });
  });

  private readonly scale = computed(() => {
    const segments = this.segments();
    return (ts: number) => {
      // The last segment starting at or before `ts`.
      let lo = 0;
      let hi = segments.length - 1;
      while (lo < hi) {
        const mid = (lo + hi + 1) >> 1;
        if (segments[mid].start <= ts) lo = mid;
        else hi = mid - 1;
      }
      const s = segments[lo];
      const clamped = Math.min(Math.max(ts, s.start), s.end);
      return s.x + ((clamped - s.start) / (s.end - s.start)) * s.w;
    };
  });

  protected readonly gaps = computed(() => {
    const segments = this.segments();
    return segments.slice(1).map((next, i) => {
      const prev = segments[i];
      return { x: prev.x + prev.w, title: `idle ${fmtMs(next.start - prev.end)}` };
    });
  });

  private readonly toolColors = computed(() => {
    const counts = new Map<string, number>();
    for (const item of this.items()) {
      if (item.kind === 'tool' || item.kind === 'memory') {
        const name = item.toolName ?? item.kind;
        counts.set(name, (counts.get(name) ?? 0) + 1);
      }
    }
    const ranked = [...counts.entries()].sort((a, b) => b[1] - a[1]).map(([name]) => name);
    const colors = new Map<string, string>();
    ranked.forEach((name, i) =>
      colors.set(name, i < TOOL_SLOTS ? `var(--chart-${i + 1})` : 'var(--thread-slate)'),
    );
    return { colors, ranked };
  });
  protected readonly legend = computed(() => {
    const { colors, ranked } = this.toolColors();
    const top = ranked.slice(0, TOOL_SLOTS).map((name) => ({ name, color: colors.get(name)! }));
    return ranked.length > TOOL_SLOTS
      ? [...top, { name: 'other', color: 'var(--thread-slate)' }]
      : top;
  });

  protected readonly layout = computed(() => {
    const x = this.scale();
    const { t1 } = this.extent();
    const { colors } = this.toolColors();
    const byThread = new Map<string, ObserveItem[]>();
    for (const item of [...this.items()].sort((a, b) => a.ts - b.ts)) {
      const key = `${item.runId}:${item.threadKey}`;
      const list = byThread.get(key);
      if (list) list.push(item);
      else byThread.set(key, [item]);
    }
    const lanes: Lane[] = [];
    const bars: Bar[] = [];
    const ticks: Tick[] = [];
    let y = 6;
    for (const [key, items] of byThread) {
      const rowEnds: number[] = [];
      const laneBars: Array<Omit<Bar, 'y'> & { row: number }> = [];
      for (const item of items) {
        const isSpan =
          (item.kind === 'tool' || item.kind === 'memory') &&
          (item.durationMs !== undefined || item.state === 'running');
        if (!isSpan) {
          ticks.push({
            key: itemKey(item),
            x: x(item.ts),
            y,
            kind: item.kind === 'message' ? (item.role === 'user' ? 'user' : 'assistant') : 'event',
            title: `${item.kind === 'message' ? item.role : item.kind} · ${itemSummary(item)}`,
          });
          continue;
        }
        const end = item.state === 'running' ? t1 : item.ts + (item.durationMs ?? 0);
        const left = x(item.ts);
        const w = Math.max(2, x(end) - left);
        // First row whose last bar ends before this one starts (1px gap).
        let row = rowEnds.findIndex((e) => e + 1 < left);
        if (row < 0) {
          row = rowEnds.length;
          rowEnds.push(0);
        }
        rowEnds[row] = left + w;
        const name = item.toolName ?? item.kind;
        laneBars.push({
          key: itemKey(item),
          x: left,
          w,
          row,
          color: colors.get(name) ?? 'var(--thread-slate)',
          failed: isFailed(item),
          running: item.state === 'running',
          title: `${name} · ${fmtMs(item.durationMs ?? end - item.ts)} · ${itemSummary(item)}`,
        });
      }
      // Row 0 holds the thread label and ticks; bars start below it.
      const rows = Math.max(1, rowEnds.length) + 1;
      for (const bar of laneBars) bars.push({ ...bar, y: y + (bar.row + 1) * ROW });
      lanes.push({
        key,
        name: items[0].threadName,
        color: threadColorVar(items[0].threadColor),
        y,
        height: rows * ROW,
      });
      y += rows * ROW + LANE_GAP;
    }
    return { lanes, bars, ticks, height: y };
  });

  protected readonly axis = computed(() => {
    const { t0 } = this.extent();
    const steps = [
      1e3, 2e3, 5e3, 1e4, 15e3, 3e4, 6e4, 12e4, 3e5, 6e5, 9e5, 18e5, 36e5, 72e5, 144e5,
    ];
    const ticks: Array<{ x: number; label: string }> = [];
    let lastX = -Infinity;
    for (const segment of this.segments()) {
      const span = segment.end - segment.start;
      const target = span / Math.max(1, Math.floor(segment.w / 90));
      const step = steps.find((s) => s >= target) ?? 288e5;
      for (let t = 0; t <= span; t += step) {
        const x = segment.x + (t / span) * segment.w;
        if (x - lastX < 64) continue;
        ticks.push({ x, label: `+${fmtMs(segment.start + t - t0)}` });
        lastX = x;
      }
    }
    return ticks;
  });
}
