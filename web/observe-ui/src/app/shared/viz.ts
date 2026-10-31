import { ChangeDetectionStrategy, Component, computed, input, output } from '@angular/core';
import { NgIcon, provideIcons } from '@ng-icons/core';
import { lucideArrowDownRight, lucideArrowUpRight, lucideMinus } from '@ng-icons/lucide';
import { HlmCardImports } from '@spartan-ng/helm/card';
import { fmtPercent } from '../core/format';

/** A tiny trend line; decorative beside a number, so hidden from AT. */
@Component({
  selector: 'app-sparkline',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { class: 'block', 'aria-hidden': 'true' },
  template: `
    <svg viewBox="0 0 100 28" preserveAspectRatio="none" class="h-7 w-full overflow-visible">
      <path [attr.d]="area()" [attr.fill]="color()" fill-opacity="0.14" />
      <path
        [attr.d]="line()"
        fill="none"
        [attr.stroke]="color()"
        stroke-width="1.5"
        vector-effect="non-scaling-stroke"
        stroke-linejoin="round"
      />
    </svg>
  `,
})
export class Sparkline {
  readonly values = input.required<number[]>();
  readonly color = input('var(--chart-1)');

  private readonly points = computed(() => {
    const values = this.values();
    if (values.length < 2) return [] as Array<[number, number]>;
    const max = Math.max(...values, 1);
    return values.map(
      (v, i) => [(i / (values.length - 1)) * 100, 26 - (v / max) * 24] as [number, number],
    );
  });
  protected readonly line = computed(() =>
    this.points()
      .map(([x, y], i) => `${i ? 'L' : 'M'}${x.toFixed(2)},${y.toFixed(2)}`)
      .join(''),
  );
  protected readonly area = computed(() =>
    this.points().length ? `${this.line()}L100,28L0,28Z` : '',
  );
}

/** A headline number with its change against the previous window. */
@Component({
  selector: 'app-kpi',
  imports: [HlmCardImports, NgIcon, Sparkline],
  providers: [provideIcons({ lucideArrowUpRight, lucideArrowDownRight, lucideMinus })],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <section hlmCard class="h-full gap-2 py-4">
      <div hlmCardContent class="flex flex-col gap-1 px-4">
        <h3 class="text-muted-foreground text-xs font-medium">{{ label() }}</h3>
        <div class="flex items-baseline gap-2">
          <span class="tabular text-2xl font-semibold tracking-tight">{{ value() }}</span>
          @if (delta() !== undefined) {
            <span class="tabular flex items-center gap-0.5 text-xs" [class]="deltaClass()">
              <ng-icon [name]="deltaIcon()" class="text-[length:--spacing(3.5)]" />
              {{ deltaText() }}
            </span>
          }
        </div>
        @if (hint()) {
          <p class="text-muted-foreground truncate text-xs">{{ hint() }}</p>
        }
        @if (spark().length > 1) {
          <app-sparkline class="mt-1" [values]="spark()" [color]="sparkColor()" />
        }
      </div>
    </section>
  `,
})
export class Kpi {
  readonly label = input.required<string>();
  readonly value = input.required<string>();
  /** Relative change; `null` means no baseline, `undefined` hides it. */
  readonly delta = input<number | null | undefined>(undefined);
  /** A rise is bad (errors, cost). */
  readonly invert = input(false);
  readonly hint = input('');
  readonly spark = input<number[]>([]);
  readonly sparkColor = input('var(--chart-1)');

  protected readonly deltaText = computed(() => {
    const d = this.delta();
    return d === null || d === undefined ? 'new' : fmtPercent(Math.abs(d));
  });
  protected readonly deltaIcon = computed(() => {
    const d = this.delta();
    if (d === null || d === undefined || Math.abs(d) < 0.005) return 'lucideMinus';
    return d > 0 ? 'lucideArrowUpRight' : 'lucideArrowDownRight';
  });
  protected readonly deltaClass = computed(() => {
    const d = this.delta();
    if (d === null || d === undefined || Math.abs(d) < 0.005) return 'text-muted-foreground';
    return d > 0 !== this.invert() ? 'text-success' : 'text-destructive';
  });
}

export interface BarItem {
  key: string;
  label: string;
  value: number;
  display: string;
  sub?: string;
  /** Part of `value` drawn in the status color (e.g. errors). */
  alert?: number;
}

/** Ranked horizontal bars; each row is a button when `pick` is bound. */
@Component({
  selector: 'app-bar-list',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <ul class="flex flex-col gap-1.5">
      @for (item of items(); track item.key) {
        <li>
          <button
            type="button"
            class="hover:bg-muted/60 focus-visible:ring-ring group grid w-full grid-cols-[minmax(0,1fr)_auto] items-center gap-x-3 rounded-md px-1.5 py-1 text-left outline-none focus-visible:ring-2"
            (click)="pick.emit(item.key)"
          >
            <span class="relative h-6 min-w-0">
              <span
                class="absolute inset-y-0 left-0 rounded-[4px]"
                [style.width.%]="(item.value / max()) * 100"
                [style.background]="color()"
                style="opacity: 0.18"
              ></span>
              @if (item.alert) {
                <span
                  class="bg-destructive absolute inset-y-0 left-0 rounded-[4px] opacity-60"
                  [style.width.%]="(item.alert / max()) * 100"
                ></span>
              }
              <span
                class="relative flex h-6 items-center gap-2 truncate px-2 text-sm"
                [title]="item.label"
              >
                <span class="truncate font-medium">{{ item.label }}</span>
                @if (item.sub) {
                  <span class="text-muted-foreground truncate text-xs">{{ item.sub }}</span>
                }
              </span>
            </span>
            <span class="tabular text-sm">{{ item.display }}</span>
          </button>
        </li>
      }
    </ul>
  `,
})
export class BarList {
  readonly items = input.required<BarItem[]>();
  readonly color = input('var(--chart-1)');
  readonly pick = output<string>();
  protected readonly max = computed(() => Math.max(1, ...this.items().map((i) => i.value)));
}

const DAYS = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'];

/** Activity by weekday and hour, on the sequential blue ramp. */
@Component({
  selector: 'app-heatmap',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <div class="overflow-x-auto">
      <div
        class="grid min-w-[560px] gap-[2px] text-[10px]"
        style="grid-template-columns: 2.25rem repeat(24, minmax(0, 1fr))"
        role="table"
        [attr.aria-label]="label()"
      >
        <span></span>
        @for (h of hours; track h) {
          <span class="text-muted-foreground text-center" role="columnheader">{{
            h % 6 === 0 ? h : ''
          }}</span>
        }
        @for (row of matrix(); track $index; let d = $index) {
          <span class="text-muted-foreground pr-1 leading-5" role="rowheader">{{ days[d] }}</span>
          @for (value of row; track $index; let h = $index) {
            <span
              role="cell"
              class="h-5 rounded-[3px]"
              [style.background]="shade(value)"
              [title]="days[d] + ' ' + h + ':00 – ' + value + ' events'"
              [attr.aria-label]="days[d] + ' ' + h + ':00, ' + value + ' events'"
            ></span>
          }
        }
      </div>
      <div class="text-muted-foreground mt-2 flex items-center justify-end gap-1 text-[10px]">
        less
        @for (step of steps; track step) {
          <span class="size-3 rounded-[3px]" [style.background]="'var(--seq-' + step + ')'"></span>
        }
        more
      </div>
    </div>
  `,
})
export class Heatmap {
  readonly matrix = input.required<number[][]>();
  readonly label = input('Activity by weekday and hour');
  protected readonly days = DAYS;
  protected readonly hours = Array.from({ length: 24 }, (_, i) => i);
  protected readonly steps = [0, 1, 2, 3, 4, 5, 6];
  private readonly max = computed(() => Math.max(1, ...this.matrix().flat()));

  protected shade(value: number): string {
    if (!value) return 'var(--seq-0)';
    // Square-root steps keep a few busy hours from washing out the rest.
    const step = Math.min(6, 1 + Math.floor(Math.sqrt(value / this.max()) * 5.999));
    return `var(--seq-${step})`;
  }
}
