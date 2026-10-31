import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  computed,
  effect,
  inject,
  input,
  linkedSignal,
  resource,
  signal,
} from '@angular/core';
import { Router, RouterLink } from '@angular/router';
import { NgIcon, provideIcons } from '@ng-icons/core';
import { lucideChevronDown, lucideChevronRight, lucideDownload } from '@ng-icons/lucide';
import { HlmAlertImports } from '@spartan-ng/helm/alert';
import { HlmBadgeImports } from '@spartan-ng/helm/badge';
import { HlmBreadcrumbImports } from '@spartan-ng/helm/breadcrumb';
import { HlmButtonImports } from '@spartan-ng/helm/button';
import { HlmCardImports } from '@spartan-ng/helm/card';
import { HlmEmptyImports } from '@spartan-ng/helm/empty';
import { HlmResizableImports } from '@spartan-ng/helm/resizable';
import { HlmSkeletonImports } from '@spartan-ng/helm/skeleton';
import { HlmTableImports } from '@spartan-ng/helm/table';
import { HlmTabsImports } from '@spartan-ng/helm/tabs';
import { HlmToggleGroupImports } from '@spartan-ng/helm/toggle-group';
import { Api, Prefs } from '../core/api';
import { fileChanges } from '../core/diff';
import {
  fmtCost,
  fmtCount,
  fmtDateTime,
  fmtMs,
  fmtPercent,
  fmtTime,
  LIVE_WINDOW_MS,
  oneLine,
  runActivity,
  shortId,
} from '../core/format';
import { cacheHit, isFailed, threadColorVar, turnUsage } from '../core/items';
import { itemKey } from '../core/live';
import { RunsStore } from '../core/stores';
import type { ObserveItem } from '../core/types';
import { DiffView, type DiffMode } from '../shared/diff-view';
import { Inspector } from '../shared/inspector';
import { StackedBars, type BarRow, type BarSeries } from '../shared/stacked-bars';
import { Tape } from '../shared/tape';
import { BarList, Kpi, type BarItem } from '../shared/viz';
import { Waterfall } from '../shared/waterfall';

const LIVE_POLL_MS = 4_000;

const TURN_SERIES: BarSeries[] = [
  { key: 'input', label: 'Input', color: 'var(--chart-1)' },
  { key: 'cacheRead', label: 'Cache read', color: 'var(--chart-3)' },
  { key: 'cacheWrite', label: 'Cache write', color: 'var(--chart-4)' },
  { key: 'output', label: 'Output', color: 'var(--chart-2)' },
];
const PROMPT_SERIES: BarSeries[] = [
  { key: 'prompt', label: 'Prompt tokens', color: 'var(--chart-7)' },
];

function percentile(sorted: number[], p: number): number | undefined {
  if (!sorted.length) return undefined;
  return sorted[Math.min(sorted.length, Math.max(1, Math.ceil(p * sorted.length))) - 1];
}

@Component({
  selector: 'app-run-detail',
  imports: [
    RouterLink,
    NgIcon,
    HlmBreadcrumbImports,
    HlmBadgeImports,
    HlmButtonImports,
    HlmCardImports,
    HlmTabsImports,
    HlmTableImports,
    HlmResizableImports,
    HlmToggleGroupImports,
    HlmSkeletonImports,
    HlmEmptyImports,
    HlmAlertImports,
    Tape,
    Inspector,
    Waterfall,
    DiffView,
    StackedBars,
    BarList,
    Kpi,
  ],
  providers: [provideIcons({ lucideChevronDown, lucideChevronRight, lucideDownload })],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <div class="mx-auto flex max-w-[1600px] flex-col gap-4 p-4 md:p-6">
      <nav hlmBreadcrumb>
        <ol hlmBreadcrumbList>
          <li hlmBreadcrumbItem><a hlmBreadcrumbLink link="/runs">Runs</a></li>
          <li hlmBreadcrumbSeparator></li>
          <li hlmBreadcrumbItem>
            <span hlmBreadcrumbPage>{{ name() }}</span>
          </li>
        </ol>
      </nav>

      @if (state.error(); as error) {
        <div hlmAlert variant="destructive">
          <h4 hlmAlertTitle>Could not load this run</h4>
          <p hlmAlertDescription>{{ error.message }}</p>
        </div>
      }

      @if (run(); as r) {
        <header class="flex flex-wrap items-start justify-between gap-3">
          <div class="flex min-w-0 flex-col gap-1">
            <div class="flex flex-wrap items-center gap-2">
              <h1 class="truncate text-xl font-semibold tracking-tight">{{ name() }}</h1>
              <span
                hlmBadge
                [variant]="activity() === 'live' ? 'default' : 'secondary'"
                [title]="
                  activity() === 'stale' ? 'Never finished: no activity for over 10 minutes' : ''
                "
                >{{
                  activity() === 'live'
                    ? 'running'
                    : activity() === 'stale'
                      ? 'never finished'
                      : 'finished'
                }}</span
              >
            </div>
            <p class="text-muted-foreground text-sm">
              {{ r.model ?? 'unknown model' }}
              @if (r.thinkingLevel) {
                · thinking {{ r.thinkingLevel }}
              }
              · started {{ dateTime(r.startedAt) }} · {{ ms(stats().spanMs) }} ·
              <span class="font-mono text-xs">{{ runId() }}</span>
            </p>
          </div>
          <button hlmBtn variant="outline" size="sm" (click)="download()">
            <ng-icon name="lucideDownload" /> Export JSON
          </button>
        </header>

        <div class="grid grid-cols-2 gap-3 md:grid-cols-4 xl:grid-cols-7">
          <app-kpi
            label="Replies"
            [value]="count(stats().turns)"
            [hint]="stats().turnErrors + ' failed'"
          />
          <app-kpi
            label="Tool calls"
            [value]="count(stats().tools)"
            [hint]="stats().toolErrors + ' failed'"
          />
          <app-kpi
            label="Tokens"
            [value]="count(stats().tokens)"
            [hint]="count(stats().usage.output) + ' output'"
          />
          <app-kpi
            label="Cache hit"
            [value]="percent(hit(stats().usage))"
            [hint]="count(stats().usage.cacheRead) + ' cached'"
          />
          <app-kpi label="Cost" [value]="cost(stats().usage.cost)" />
          <app-kpi
            label="Files changed"
            [value]="count(files().length)"
            [hint]="'+' + stats().added + ' −' + stats().removed"
          />
          <app-kpi
            label="Threads"
            [value]="count(r.threads.length)"
            [hint]="r.threads.length - 1 + ' subagents'"
          />
        </div>

        <hlm-tabs [tab]="tab()" (tabActivated)="setTab($event)" class="gap-3">
          <hlm-tabs-list aria-label="Run views" class="max-w-full justify-start overflow-x-auto">
            <button hlmTabsTrigger="timeline">Timeline</button>
            <button hlmTabsTrigger="waterfall">Waterfall</button>
            <button hlmTabsTrigger="files">
              Files <span class="text-muted-foreground tabular text-xs">{{ files().length }}</span>
            </button>
            <button hlmTabsTrigger="tokens">Tokens</button>
            <button hlmTabsTrigger="tools">Tools</button>
            <button hlmTabsTrigger="conversation">Conversation</button>
          </hlm-tabs-list>

          <div hlmTabsContent="timeline">
            <section hlmCard class="h-[calc(100vh-20rem)] min-h-[480px] overflow-hidden py-0">
              <hlm-resizable-group class="h-full">
                <hlm-resizable-panel [defaultSize]="selectedItem() ? 58 : 100" [minSize]="35">
                  <app-tape [items]="r.items" [(selected)]="selected" />
                </hlm-resizable-panel>
                @if (selectedItem(); as item) {
                  <hlm-resizable-handle withHandle />
                  <hlm-resizable-panel [defaultSize]="42" [minSize]="25">
                    <app-inspector
                      [item]="item"
                      [context]="r.items"
                      (closed)="selected.set(null)"
                    />
                  </hlm-resizable-panel>
                }
              </hlm-resizable-group>
            </section>
          </div>

          <div hlmTabsContent="waterfall" class="flex flex-col gap-4">
            <app-waterfall
              [items]="r.items"
              [selected]="selected()"
              (selectedChange)="inspect($event)"
            />
          </div>

          <div hlmTabsContent="files" class="flex flex-col gap-3">
            <div class="flex flex-wrap items-center gap-2">
              <hlm-toggle-group
                type="single"
                size="sm"
                variant="outline"
                [value]="diffMode()"
                (valueChange)="diffMode.set($event === 'split' ? 'split' : 'unified')"
                aria-label="Diff layout"
              >
                <button hlmToggleGroupItem value="unified">Unified</button>
                <button hlmToggleGroupItem value="split">Split</button>
              </hlm-toggle-group>
              <button hlmBtn variant="ghost" size="sm" (click)="toggleAll()">
                {{ openFiles().size ? 'Collapse all' : 'Expand all' }}
              </button>
              <span class="text-muted-foreground ml-auto text-xs">
                {{ files().length }} files · <span class="text-success">+{{ stats().added }}</span>
                <span class="text-destructive">−{{ stats().removed }}</span>
              </span>
            </div>
            @for (file of files(); track file.path) {
              <section hlmCard class="gap-0 py-0">
                <button
                  type="button"
                  class="hover:bg-muted/40 flex w-full items-center gap-2 px-4 py-3 text-left"
                  (click)="toggleFile(file.path)"
                  [attr.aria-expanded]="openFiles().has(file.path)"
                >
                  <ng-icon
                    [name]="openFiles().has(file.path) ? 'lucideChevronDown' : 'lucideChevronRight'"
                  />
                  <span class="min-w-0 flex-1 truncate font-mono text-sm">{{ file.path }}</span>
                  <span class="text-muted-foreground text-xs"
                    >{{ file.changes.length }} change{{
                      file.changes.length === 1 ? '' : 's'
                    }}</span
                  >
                  <span class="tabular text-success text-xs">+{{ file.added }}</span>
                  <span class="tabular text-destructive text-xs">−{{ file.removed }}</span>
                  <span class="flex h-2 w-16 overflow-hidden rounded-full" aria-hidden="true">
                    <span class="bg-success" [style.flex]="file.added"></span>
                    <span class="bg-destructive" [style.flex]="file.removed"></span>
                  </span>
                </button>
                @if (openFiles().has(file.path)) {
                  <div class="flex flex-col gap-3 border-t px-4 py-3">
                    @for (change of file.changes; track change.key; let i = $index) {
                      <div class="flex flex-col gap-1.5">
                        <div
                          class="text-muted-foreground flex flex-wrap items-center gap-2 text-xs"
                        >
                          <span hlmBadge variant="outline" class="font-mono">{{
                            change.tool
                          }}</span>
                          <span>#{{ i + 1 }} · {{ time(change.ts) }}</span>
                          <span>from {{ change.basis }}</span>
                          @if (change.failed) {
                            <span hlmBadge variant="destructive">failed, not applied</span>
                          }
                          <button
                            hlmBtn
                            variant="link"
                            size="xs"
                            class="ml-auto"
                            (click)="inspect(change.key)"
                          >
                            Inspect call
                          </button>
                        </div>
                        <app-diff-view [diff]="change.diff" [mode]="diffMode()" />
                      </div>
                    }
                  </div>
                }
              </section>
            } @empty {
              <div hlmEmpty class="py-12">
                <div hlmEmptyHeader>
                  <h3 hlmEmptyTitle>No file changes</h3>
                  <p hlmEmptyDescription>This run made no edit or write calls.</p>
                </div>
              </div>
            }
          </div>

          <div hlmTabsContent="tokens" class="grid gap-4 xl:grid-cols-2">
            <section hlmCard>
              <div hlmCardHeader>
                <h2 hlmCardTitle>Tokens per reply</h2>
                <p hlmCardDescription>{{ turns().length }} replies, in order</p>
              </div>
              <div hlmCardContent>
                <app-stacked-bars
                  [rows]="turnRows()"
                  [series]="turnSeries"
                  label="Tokens per reply"
                  [format]="count"
                />
              </div>
            </section>
            <section hlmCard>
              <div hlmCardHeader>
                <h2 hlmCardTitle>Prompt size per reply</h2>
                <p hlmCardDescription>
                  Input + cache tokens: how full the context was
                  @if (contextWindow(); as w) {
                    · window {{ count(w) }}
                  }
                </p>
              </div>
              <div hlmCardContent>
                <app-stacked-bars
                  [rows]="promptRows()"
                  [series]="promptSeries"
                  label="Prompt size per reply"
                  [format]="count"
                />
              </div>
            </section>
            <section hlmCard class="xl:col-span-2">
              <div hlmCardHeader>
                <h2 hlmCardTitle>Replies</h2>
              </div>
              <div hlmCardContent>
                <div hlmTableContainer class="max-h-[420px] overflow-y-auto">
                  <table hlmTable>
                    <thead hlmTHead>
                      <tr hlmTr>
                        <th hlmTh>#</th>
                        <th hlmTh>Time</th>
                        <th hlmTh>Model</th>
                        <th hlmTh class="text-right">Input</th>
                        <th hlmTh class="text-right">Cache read</th>
                        <th hlmTh class="text-right">Output</th>
                        <th hlmTh class="text-right">Cache hit</th>
                        <th hlmTh class="text-right">Cost</th>
                        <th hlmTh>Stop</th>
                      </tr>
                    </thead>
                    <tbody hlmTBody>
                      @for (t of turns(); track t.key; let i = $index) {
                        <tr hlmTr class="cursor-pointer" (click)="inspect(t.key)">
                          <td hlmTd class="tabular">{{ i + 1 }}</td>
                          <td hlmTd class="tabular text-muted-foreground text-xs">
                            {{ time(t.ts) }}
                          </td>
                          <td hlmTd class="text-muted-foreground max-w-[14rem] truncate text-xs">
                            {{ t.usage.model ?? '–' }}
                          </td>
                          <td hlmTd class="tabular text-right">{{ count(t.usage.input) }}</td>
                          <td hlmTd class="tabular text-right">{{ count(t.usage.cacheRead) }}</td>
                          <td hlmTd class="tabular text-right">{{ count(t.usage.output) }}</td>
                          <td hlmTd class="tabular text-right">{{ percent(hit(t.usage)) }}</td>
                          <td hlmTd class="tabular text-right">{{ cost(t.usage.cost) }}</td>
                          <td hlmTd>
                            <span hlmBadge [variant]="t.failed ? 'destructive' : 'secondary'">{{
                              t.usage.stopReason ?? '–'
                            }}</span>
                          </td>
                        </tr>
                      }
                    </tbody>
                  </table>
                </div>
              </div>
            </section>
          </div>

          <div hlmTabsContent="tools" class="grid gap-4 xl:grid-cols-[minmax(0,3fr)_minmax(0,2fr)]">
            <section hlmCard>
              <div hlmCardHeader>
                <h2 hlmCardTitle>Tools in this run</h2>
              </div>
              <div hlmCardContent>
                <div hlmTableContainer>
                  <table hlmTable>
                    <thead hlmTHead>
                      <tr hlmTr>
                        <th hlmTh>Tool</th>
                        <th hlmTh class="text-right">Calls</th>
                        <th hlmTh class="text-right">Failed</th>
                        <th hlmTh class="text-right">p50</th>
                        <th hlmTh class="text-right">p95</th>
                        <th hlmTh class="text-right">Max</th>
                        <th hlmTh class="text-right">Total</th>
                      </tr>
                    </thead>
                    <tbody hlmTBody>
                      @for (t of toolStats(); track t.tool) {
                        <tr hlmTr>
                          <td hlmTd class="font-mono text-xs">{{ t.tool }}</td>
                          <td hlmTd class="tabular text-right">{{ t.calls }}</td>
                          <td hlmTd class="tabular text-right" [class.text-destructive]="t.errors">
                            {{ t.errors || '–' }}
                          </td>
                          <td hlmTd class="tabular text-right">{{ ms(t.p50) }}</td>
                          <td hlmTd class="tabular text-right">{{ ms(t.p95) }}</td>
                          <td hlmTd class="tabular text-right">{{ ms(t.max) }}</td>
                          <td hlmTd class="tabular text-right">{{ ms(t.total) }}</td>
                        </tr>
                      }
                    </tbody>
                  </table>
                </div>
              </div>
            </section>
            <section hlmCard>
              <div hlmCardHeader>
                <h2 hlmCardTitle>Where the time went</h2>
                <p hlmCardDescription>Total tool time, by tool</p>
              </div>
              <div hlmCardContent>
                <app-bar-list [items]="toolTimeItems()" />
              </div>
            </section>
          </div>

          <div hlmTabsContent="conversation" class="mx-auto flex w-full max-w-3xl flex-col gap-3">
            @for (m of conversation(); track m.key) {
              <article
                class="flex flex-col gap-1 rounded-xl border px-4 py-3"
                [class]="m.item.role === 'user' ? 'bg-muted/60 ml-8' : 'bg-card mr-8'"
              >
                <header class="text-muted-foreground flex items-center gap-2 text-xs">
                  <span
                    class="size-2 rounded-full"
                    [style.background]="thread(m.item.threadColor)"
                  ></span>
                  <span class="text-foreground font-medium">{{
                    m.item.kind === 'skill' ? 'skill' : m.item.role
                  }}</span>
                  <span>{{ m.item.threadName }}</span>
                  <span class="ml-auto tabular">{{ time(m.item.ts) }}</span>
                </header>
                <p
                  class="text-sm break-words whitespace-pre-wrap"
                  [class.text-destructive]="m.failed"
                  [textContent]="m.text"
                ></p>
                @if (m.usage; as u) {
                  <footer class="text-muted-foreground tabular text-[11px]">
                    {{ count(u.input + u.cacheRead) }} in · {{ count(u.output) }} out ·
                    {{ cost(u.cost) }}
                    @if (u.stopReason) {
                      · {{ u.stopReason }}
                    }
                  </footer>
                }
              </article>
            } @empty {
              <p class="text-muted-foreground text-sm">No messages recorded.</p>
            }
          </div>
        </hlm-tabs>
      } @else if (state.isLoading()) {
        <div hlmSkeleton class="h-10 w-1/3"></div>
        <div class="grid grid-cols-2 gap-3 md:grid-cols-4 xl:grid-cols-7">
          @for (i of [1, 2, 3, 4, 5, 6, 7]; track i) {
            <div hlmSkeleton class="h-24 rounded-xl"></div>
          }
        </div>
        <div hlmSkeleton class="h-96 rounded-xl"></div>
      }
    </div>
  `,
})
export class RunDetailPage {
  /** Route parameter. */
  readonly runId = input.required<string>();
  /** `?view=` query parameter: the open tab, so views can be linked. */
  readonly view = input<string | undefined>();
  private readonly api = inject(Api);
  private readonly prefs = inject(Prefs);
  private readonly runs = inject(RunsStore);
  protected readonly tab = linkedSignal(() => this.view() || 'timeline');
  private readonly router = inject(Router);
  protected readonly selected = signal<string | null>(null);
  protected readonly diffMode = signal<DiffMode>('unified');

  protected readonly turnSeries = TURN_SERIES;
  protected readonly promptSeries = PROMPT_SERIES;
  protected readonly count = fmtCount;
  protected readonly cost = fmtCost;
  protected readonly ms = fmtMs;
  protected readonly time = fmtTime;
  protected readonly dateTime = fmtDateTime;
  protected readonly hit = cacheHit;
  protected readonly thread = threadColorVar;
  protected readonly percent = (r: number | null | undefined) => fmtPercent(r);

  protected readonly state = resource({
    params: () => ({ runId: this.runId(), tick: this.prefs.refreshTick() }),
    loader: ({ params, abortSignal }) => this.api.runState(params.runId, abortSignal),
  });
  protected readonly run = computed(() => (this.state.hasValue() ? this.state.value() : undefined));
  private readonly row = computed(() => this.runs.runs().find((r) => r.runId === this.runId()));
  /** `live`, `stale` (never finished) or `finished`. */
  protected readonly activity = computed(() => {
    const row = this.row();
    if (row) return runActivity(row);
    // Outside the selected range: judge from the run's own activity.
    const items = this.run()?.items ?? [];
    const last = Math.max(0, ...items.map((i) => i.ts));
    const running = items.some((i) => i.state === 'running');
    return running ? (Date.now() - last < LIVE_WINDOW_MS ? 'live' : 'stale') : 'finished';
  });
  protected readonly live = computed(() => this.activity() === 'live');
  protected readonly name = computed(() => {
    const run = this.run();
    const prompt = run?.items.find((i) => i.kind === 'message' && i.role === 'user')?.text;
    return (
      run?.sessionName ||
      this.row()?.sessionName ||
      this.row()?.prompt ||
      (prompt ? oneLine(prompt) : '') ||
      `run ${shortId(this.runId())}`
    );
  });
  protected readonly contextWindow = computed(() => this.run()?.context?.contextWindow);
  protected readonly selectedItem = computed(() =>
    this.run()?.items.find((i) => itemKey(i) === this.selected()),
  );
  protected readonly files = computed(() => fileChanges(this.run()?.items ?? []));
  /** Expanded files; the most recently changed one starts open. */
  protected readonly openFiles = linkedSignal<string | undefined, Set<string>>({
    source: () => this.files()[0]?.path,
    computation: (first, previous) =>
      previous?.value.size ? previous.value : new Set(first ? [first] : []),
  });

  protected readonly turns = computed(() =>
    (this.run()?.items ?? []).flatMap((item) => {
      const usage = turnUsage(item);
      return usage ? [{ key: itemKey(item), ts: item.ts, usage, failed: isFailed(item) }] : [];
    }),
  );

  protected readonly stats = computed(() => {
    const items = this.run()?.items ?? [];
    const usage = { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, cost: 0 };
    for (const t of this.turns()) {
      usage.input += t.usage.input;
      usage.output += t.usage.output;
      usage.cacheRead += t.usage.cacheRead;
      usage.cacheWrite += t.usage.cacheWrite;
      usage.cost += t.usage.cost;
    }
    const tools = items.filter((i) => i.kind === 'tool');
    const start = this.run()?.startedAt ?? 0;
    const end = Math.max(start, ...items.map((i) => i.ts + (i.durationMs ?? 0)));
    return {
      usage,
      tokens: usage.input + usage.output + usage.cacheRead + usage.cacheWrite,
      turns: this.turns().length,
      turnErrors: this.turns().filter((t) => t.failed).length,
      tools: tools.length,
      toolErrors: tools.filter(isFailed).length,
      added: this.files().reduce((n, f) => n + f.added, 0),
      removed: this.files().reduce((n, f) => n + f.removed, 0),
      spanMs: this.row()?.durationMs ?? end - start,
    };
  });

  protected readonly turnRows = computed<BarRow[]>(() =>
    this.turns().flatMap((t, i) =>
      TURN_SERIES.map((s) => ({
        x: `#${i + 1}`,
        series: s.key,
        value: t.usage[s.key as 'input' | 'cacheRead' | 'cacheWrite' | 'output'],
      })),
    ),
  );
  protected readonly promptRows = computed<BarRow[]>(() =>
    this.turns().map((t, i) => ({
      x: `#${i + 1}`,
      series: 'prompt',
      value: t.usage.input + t.usage.cacheRead + t.usage.cacheWrite,
    })),
  );

  protected readonly toolStats = computed(() => {
    const groups = new Map<string, ObserveItem[]>();
    for (const item of this.run()?.items ?? []) {
      if (item.kind !== 'tool') continue;
      const name = item.toolName ?? 'tool';
      const list = groups.get(name);
      if (list) list.push(item);
      else groups.set(name, [item]);
    }
    return [...groups.entries()]
      .map(([tool, calls]) => {
        const ms = calls
          .flatMap((c) => (c.durationMs === undefined ? [] : [c.durationMs]))
          .sort((a, b) => a - b);
        return {
          tool,
          calls: calls.length,
          errors: calls.filter(isFailed).length,
          p50: percentile(ms, 0.5),
          p95: percentile(ms, 0.95),
          max: ms.at(-1),
          total: ms.reduce((a, b) => a + b, 0),
        };
      })
      .sort((a, b) => b.calls - a.calls);
  });
  protected readonly toolTimeItems = computed<BarItem[]>(() =>
    [...this.toolStats()]
      .sort((a, b) => b.total - a.total)
      .slice(0, 10)
      .map((t) => ({
        key: t.tool,
        label: t.tool,
        value: t.total,
        display: fmtMs(t.total),
        sub: `${t.calls} calls`,
      })),
  );

  protected readonly conversation = computed(() =>
    (this.run()?.items ?? [])
      .filter((i) => (i.kind === 'message' && i.role !== 'custom') || i.kind === 'skill')
      .map((item) => ({
        key: itemKey(item),
        item,
        text: item.text || (isFailed(item) ? '(failed reply)' : '(empty)'),
        usage: turnUsage(item),
        failed: isFailed(item),
      })),
  );

  constructor() {
    // Keep a running run fresh.
    const timer = setInterval(() => {
      if (this.live() && !this.state.isLoading()) this.state.reload();
    }, LIVE_POLL_MS);
    inject(DestroyRef).onDestroy(() => clearInterval(timer));
    // A new run starts with nothing selected and the first file open.
    effect(() => {
      this.runId();
      this.selected.set(null);
    });
  }

  protected setTab(tab: string) {
    this.tab.set(tab);
    void this.router.navigate([], {
      queryParams: { view: tab === 'timeline' ? null : tab },
      queryParamsHandling: 'merge',
      replaceUrl: true,
    });
  }

  protected inspect(key: string | null) {
    this.selected.set(key);
    if (key) this.setTab('timeline');
  }

  protected toggleFile(path: string) {
    this.openFiles.update((set) => {
      const next = new Set(set);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  }

  protected toggleAll() {
    this.openFiles.set(
      this.openFiles().size ? new Set() : new Set(this.files().map((f) => f.path)),
    );
  }

  protected download() {
    const run = this.run();
    if (!run) return;
    const blob = new Blob([JSON.stringify(run, null, 2)], { type: 'application/json' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = `icers-run-${shortId(run.runId, 12)}.json`;
    a.click();
    URL.revokeObjectURL(url);
  }
}
