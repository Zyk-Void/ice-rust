import {
  ChangeDetectionStrategy,
  Component,
  ElementRef,
  afterRenderEffect,
  computed,
  input,
  model,
  signal,
  viewChild,
} from '@angular/core';
import { NgIcon, provideIcons } from '@ng-icons/core';
import {
  lucideBrain,
  lucideCircleAlert,
  lucideGitBranch,
  lucideListOrdered,
  lucideLoaderCircle,
  lucideMessageSquare,
  lucideMinimize2,
  lucideRotateCcw,
  lucideSearch,
  lucideSparkles,
  lucideTarget,
  lucideWrench,
} from '@ng-icons/lucide';
import { HlmInputGroupImports } from '@spartan-ng/helm/input-group';
import { HlmKbdImports } from '@spartan-ng/helm/kbd';
import { HlmSwitchImports } from '@spartan-ng/helm/switch';
import { HlmToggleGroupImports } from '@spartan-ng/helm/toggle-group';
import { HlmEmptyImports } from '@spartan-ng/helm/empty';
import { fmtMs, fmtTime } from '../core/format';
import { KINDS, haystack, isFailed, itemSummary, kindMeta, threadColorVar } from '../core/items';
import { itemKey } from '../core/live';
import type { ItemKind, ObserveItem } from '../core/types';

interface Row {
  key: string;
  item: ObserveItem;
  icon: string;
  label: string;
  summary: string;
  failed: boolean;
  color: string;
}

/**
 * The activity tape: every recorded activity as a row, filterable by kind
 * (keys 0-9), searchable, navigable with ↑/↓.
 */
@Component({
  selector: 'app-tape',
  imports: [
    NgIcon,
    HlmToggleGroupImports,
    HlmInputGroupImports,
    HlmSwitchImports,
    HlmKbdImports,
    HlmEmptyImports,
  ],
  providers: [
    provideIcons({
      lucideBrain,
      lucideCircleAlert,
      lucideGitBranch,
      lucideListOrdered,
      lucideLoaderCircle,
      lucideMessageSquare,
      lucideMinimize2,
      lucideRotateCcw,
      lucideSearch,
      lucideSparkles,
      lucideTarget,
      lucideWrench,
    }),
  ],
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: {
    class: 'flex h-full min-h-0 flex-col',
    '(document:keydown)': 'onKey($event)',
  },
  template: `
    <div class="flex flex-wrap items-center gap-2 border-b px-3 py-2">
      <hlm-input-group class="w-full max-w-xs">
        <hlm-input-group-addon><ng-icon name="lucideSearch" /></hlm-input-group-addon>
        <input
          hlmInputGroupInput
          type="search"
          placeholder="Search activity"
          aria-label="Search activity"
          [value]="query()"
          (input)="query.set($any($event.target).value)"
        />
      </hlm-input-group>
      <hlm-toggle-group
        type="single"
        size="sm"
        variant="outline"
        [value]="kind()"
        (valueChange)="kind.set($any($event) || 'all')"
        aria-label="Filter by kind"
        class="flex-wrap"
      >
        <button hlmToggleGroupItem value="all" aria-label="All kinds">
          All <span class="text-muted-foreground tabular text-xs">{{ items().length }}</span>
        </button>
        @for (k of presentKinds(); track k.kind) {
          <button
            hlmToggleGroupItem
            [value]="k.kind"
            [attr.aria-label]="k.label + ' (key ' + k.key + ')'"
          >
            <ng-icon [name]="k.icon" />
            <span class="hidden xl:inline">{{ k.label }}</span>
            <span class="text-muted-foreground tabular text-xs">{{ counts()[k.kind] }}</span>
          </button>
        }
      </hlm-toggle-group>
      <label class="text-muted-foreground ml-auto flex items-center gap-2 text-xs">
        <hlm-switch
          [checked]="errorsOnly()"
          (checkedChange)="errorsOnly.set($event)"
          aria-label="Errors only"
        />
        Errors only
      </label>
    </div>
    <div
      #list
      class="min-h-0 flex-1 overflow-y-auto outline-none"
      tabindex="0"
      role="listbox"
      aria-label="Activity"
      (scroll)="onScroll()"
    >
      @for (row of rows(); track row.key) {
        <button
          type="button"
          role="option"
          [attr.aria-selected]="row.key === selected()"
          [attr.data-key]="row.key"
          class="hover:bg-muted/50 grid w-full grid-cols-[4.5rem_0.25rem_1.25rem_minmax(5rem,9rem)_minmax(0,1fr)_auto] items-center gap-2 border-b px-3 py-1.5 text-left text-sm [content-visibility:auto] [contain-intrinsic-size:auto_34px]"
          [class.bg-accent]="row.key === selected()"
          (click)="selected.set(row.key)"
        >
          <span class="text-muted-foreground tabular font-mono text-[11px]">{{
            time(row.item.ts)
          }}</span>
          <span
            class="h-5 rounded-full"
            [style.background]="row.color"
            [title]="row.item.threadName"
          ></span>
          <ng-icon [name]="row.icon" class="text-muted-foreground" />
          <span class="truncate font-medium" [class.text-destructive]="row.failed">{{
            row.label
          }}</span>
          <span class="text-muted-foreground truncate font-mono text-xs">{{ row.summary }}</span>
          <span class="flex items-center gap-1.5">
            @if (row.item.state === 'running') {
              <ng-icon
                name="lucideLoaderCircle"
                class="text-muted-foreground animate-spin"
                aria-label="running"
              />
            } @else if (row.failed) {
              <ng-icon name="lucideCircleAlert" class="text-destructive" aria-label="error" />
            }
            @if (row.item.durationMs !== undefined) {
              <span class="text-muted-foreground tabular text-xs">{{
                ms(row.item.durationMs)
              }}</span>
            }
          </span>
        </button>
      } @empty {
        <div hlmEmpty class="py-16">
          <div hlmEmptyHeader>
            <h3 hlmEmptyTitle>No activity</h3>
            <p hlmEmptyDescription>
              {{ items().length ? 'Nothing matches the filters.' : 'Nothing recorded yet.' }}
            </p>
          </div>
        </div>
      }
    </div>
    @if (keyboard()) {
      <p
        class="text-muted-foreground hidden items-center gap-1.5 border-t px-3 py-1.5 text-[11px] md:flex"
      >
        <kbd hlmKbd>0</kbd>–<kbd hlmKbd>9</kbd> filter · <kbd hlmKbd>↑</kbd
        ><kbd hlmKbd>↓</kbd> select · <kbd hlmKbd>/</kbd> search
        @if (follow() !== undefined) {
          · <kbd hlmKbd>Space</kbd> {{ follow() ? 'pause' : 'follow' }}
        }
      </p>
    }
  `,
})
export class Tape {
  readonly items = input.required<ObserveItem[]>();
  readonly selected = model<string | null>(null);
  /** Live mode: keep the newest row in view while `true`. */
  readonly follow = model<boolean | undefined>(undefined);
  readonly keyboard = input(true);
  protected readonly query = signal('');
  protected readonly kind = signal<ItemKind | 'all'>('all');
  protected readonly errorsOnly = signal(false);
  private readonly list = viewChild.required<ElementRef<HTMLElement>>('list');
  private readonly cache = new WeakMap<ObserveItem, string>();

  protected readonly counts = computed(() => {
    const counts: Partial<Record<ItemKind, number>> = {};
    for (const item of this.items()) counts[item.kind] = (counts[item.kind] ?? 0) + 1;
    return counts;
  });
  protected readonly presentKinds = computed(() => KINDS.filter((k) => this.counts()[k.kind]));
  protected readonly rows = computed<Row[]>(() => {
    const q = this.query().trim().toLowerCase();
    const kind = this.kind();
    const errorsOnly = this.errorsOnly();
    const rows: Row[] = [];
    for (const item of this.items()) {
      if (kind !== 'all' && item.kind !== kind) continue;
      const failed = isFailed(item);
      if (errorsOnly && !failed) continue;
      if (q && !this.search(item).includes(q)) continue;
      rows.push({
        key: itemKey(item),
        item,
        icon: kindMeta(item.kind).icon,
        label:
          item.kind === 'tool'
            ? (item.toolName ?? 'tool')
            : item.kind === 'message'
              ? (item.role ?? 'message')
              : item.kind,
        summary: itemSummary(item),
        failed,
        color: threadColorVar(item.threadColor),
      });
    }
    return rows;
  });

  protected readonly time = fmtTime;
  protected readonly ms = fmtMs;

  constructor() {
    afterRenderEffect(() => {
      this.rows();
      if (this.follow()) {
        const el = this.list().nativeElement;
        el.scrollTop = el.scrollHeight;
      }
    });
  }

  private search(item: ObserveItem): string {
    let text = this.cache.get(item);
    if (text === undefined) {
      text = haystack(item);
      this.cache.set(item, text);
    }
    return text;
  }

  protected onScroll() {
    if (this.follow() === undefined) return;
    const el = this.list().nativeElement;
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
    if (!atBottom && this.follow()) this.follow.set(false);
  }

  private move(step: number) {
    const rows = this.rows();
    if (!rows.length) return;
    const index = rows.findIndex((r) => r.key === this.selected());
    const next =
      rows[
        Math.max(
          0,
          Math.min(rows.length - 1, index < 0 ? (step > 0 ? 0 : rows.length - 1) : index + step),
        )
      ];
    this.selected.set(next.key);
    this.list()
      .nativeElement.querySelector(`[data-key="${CSS.escape(next.key)}"]`)
      ?.scrollIntoView({ block: 'nearest' });
  }

  protected onKey(event: KeyboardEvent) {
    if (!this.keyboard() || event.metaKey || event.ctrlKey || event.altKey) return;
    const target = event.target as HTMLElement | null;
    if (target?.closest('input, textarea, select, [contenteditable="true"], [role="dialog"]'))
      return;
    if (/^[0-9]$/.test(event.key)) {
      const meta = KINDS.find((k) => k.key === event.key);
      this.kind.set(event.key === '0' || !meta ? 'all' : meta.kind);
    } else if (event.key === '/') {
      event.preventDefault();
      this.list()
        .nativeElement.parentElement?.querySelector<HTMLInputElement>('input[type="search"]')
        ?.focus();
    } else if (event.key === ' ' && this.follow() !== undefined) {
      event.preventDefault();
      this.follow.set(!this.follow());
    } else if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault();
      this.move(event.key === 'ArrowDown' ? 1 : -1);
    } else if (event.key === 'Escape') {
      this.selected.set(null);
    }
  }
}
