import { ChangeDetectionStrategy, Component, computed, inject, resource } from '@angular/core';
import { RouterLink } from '@angular/router';
import { NgIcon, provideIcons } from '@ng-icons/core';
import { lucideTriangleAlert } from '@ng-icons/lucide';
import { HlmAlertImports } from '@spartan-ng/helm/alert';
import { HlmBadgeImports } from '@spartan-ng/helm/badge';
import { HlmCardImports } from '@spartan-ng/helm/card';
import { HlmSkeletonImports } from '@spartan-ng/helm/skeleton';
import { HlmTableImports } from '@spartan-ng/helm/table';
import { Api, Prefs } from '../core/api';
import {
  delta,
  fmtAgo,
  fmtBucket,
  fmtCost,
  fmtCount,
  fmtMs,
  fmtPercent,
  shortPath,
} from '../core/format';
import { KINDS, cacheHit } from '../core/items';
import { StackedBars, type BarRow, type BarSeries } from '../shared/stacked-bars';
import { BarList, Heatmap, Kpi, type BarItem } from '../shared/viz';

const TOKEN_SERIES: BarSeries[] = [
  { key: 'input', label: 'Input', color: 'var(--chart-1)' },
  { key: 'cacheRead', label: 'Cache read', color: 'var(--chart-3)' },
  { key: 'cacheWrite', label: 'Cache write', color: 'var(--chart-4)' },
  { key: 'output', label: 'Output', color: 'var(--chart-2)' },
];
const TOOL_SERIES: BarSeries[] = [
  { key: 'ok', label: 'Succeeded', color: 'var(--chart-1)' },
  { key: 'error', label: 'Failed', color: 'var(--chart-8)' },
];

@Component({
  selector: 'app-overview',
  imports: [
    RouterLink,
    NgIcon,
    HlmCardImports,
    HlmTableImports,
    HlmAlertImports,
    HlmBadgeImports,
    HlmSkeletonImports,
    Kpi,
    StackedBars,
    BarList,
    Heatmap,
  ],
  providers: [provideIcons({ lucideTriangleAlert })],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <div class="mx-auto flex max-w-[1400px] flex-col gap-4 p-4 md:p-6">
      <div class="flex flex-wrap items-end justify-between gap-2">
        <div>
          <h1 class="text-xl font-semibold tracking-tight">Overview</h1>
          <p class="text-muted-foreground text-sm">
            Agent activity over the last {{ prefs.range() }}, compared with the
            {{ prefs.range() }} before · source
            <span class="text-foreground font-medium">{{ data()?.source ?? '…' }}</span>
          </p>
        </div>
      </div>

      @if (overview.error(); as error) {
        <div hlmAlert variant="destructive">
          <ng-icon name="lucideTriangleAlert" />
          <h4 hlmAlertTitle>Could not load analytics</h4>
          <p hlmAlertDescription>{{ error.message }}</p>
        </div>
      }
      @if (data()?.sourceError; as sourceError) {
        <div hlmAlert>
          <ng-icon name="lucideTriangleAlert" />
          <h4 hlmAlertTitle>Showing local history</h4>
          <p hlmAlertDescription>{{ sourceError }}</p>
        </div>
      }

      @if (data(); as d) {
        <div class="grid grid-cols-2 gap-3 md:grid-cols-3 xl:grid-cols-6">
          <app-kpi
            label="Runs"
            [value]="count(d.totals.runs)"
            [delta]="change(d.totals.runs, d.previous.runs)"
            [hint]="'median ' + ms(d.totals.medianRunMs)"
            [spark]="spark('runs')"
          />
          <app-kpi
            label="Replies"
            [value]="count(d.totals.turns)"
            [delta]="change(d.totals.turns, d.previous.turns)"
            [hint]="d.totals.turnErrors + ' failed'"
            [spark]="spark('turns')"
          />
          <app-kpi
            label="Tool calls"
            [value]="count(d.totals.toolCalls)"
            [delta]="change(d.totals.toolCalls, d.previous.toolCalls)"
            [hint]="percent(errorRate(d.totals)) + ' failed · p95 ' + ms(d.totals.p95ToolMs)"
            [spark]="spark('toolCalls')"
          />
          <app-kpi
            label="Tokens"
            [value]="count(tokens(d.totals))"
            [delta]="change(tokens(d.totals), tokens(d.previous))"
            [hint]="count(d.totals.output) + ' output'"
            [spark]="spark('tokens')"
            sparkColor="var(--chart-3)"
          />
          <app-kpi
            label="Cache hit"
            [value]="percent(hit(d.totals))"
            [delta]="change(hit(d.totals) ?? 0, hit(d.previous) ?? 0)"
            [hint]="count(d.totals.cacheRead) + ' read from cache'"
          />
          <app-kpi
            label="Cost"
            [value]="cost(d.totals.cost)"
            [delta]="change(d.totals.cost, d.previous.cost)"
            [invert]="true"
            [hint]="d.totals.filesChanged + ' file changes'"
            [spark]="spark('cost')"
            sparkColor="var(--chart-2)"
          />
        </div>

        <div class="grid gap-4 xl:grid-cols-2">
          <section hlmCard>
            <div hlmCardHeader>
              <h2 hlmCardTitle>Tokens</h2>
              <p hlmCardDescription>Per {{ bucketLabel() }}, by kind</p>
            </div>
            <div hlmCardContent>
              <app-stacked-bars
                [rows]="tokenRows()"
                [series]="tokenSeries"
                label="Tokens over time"
                [format]="count"
                [xLabel]="bucketName()"
              />
            </div>
          </section>
          <section hlmCard>
            <div hlmCardHeader>
              <h2 hlmCardTitle>Tool calls</h2>
              <p hlmCardDescription>Per {{ bucketLabel() }}, failed calls on top</p>
            </div>
            <div hlmCardContent>
              <app-stacked-bars
                [rows]="toolRows()"
                [series]="toolSeries"
                label="Tool calls over time"
                [format]="count"
                [xLabel]="bucketName()"
              />
            </div>
          </section>
        </div>

        <div class="grid gap-4 xl:grid-cols-[minmax(0,3fr)_minmax(0,2fr)]">
          <section hlmCard>
            <div hlmCardHeader>
              <h2 hlmCardTitle>Tool performance</h2>
              <p hlmCardDescription>Calls, failures and latency percentiles</p>
            </div>
            <div hlmCardContent>
              <div hlmTableContainer>
                <table hlmTable>
                  <thead hlmTHead>
                    <tr hlmTr>
                      <th hlmTh>Tool</th>
                      <th hlmTh class="text-right">Calls</th>
                      <th hlmTh class="text-right">Failed</th>
                      <th hlmTh class="w-[34%]">Latency (p50 ● p95 ▏max)</th>
                    </tr>
                  </thead>
                  <tbody hlmTBody>
                    @for (t of d.tools.slice(0, 14); track t.tool) {
                      <tr hlmTr>
                        <td hlmTd class="font-mono text-xs">{{ t.tool }}</td>
                        <td hlmTd class="tabular text-right">{{ count(t.calls) }}</td>
                        <td hlmTd class="tabular text-right" [class.text-destructive]="t.errors">
                          {{ t.errors ? t.errors + ' (' + percent(t.errors / t.calls) + ')' : '–' }}
                        </td>
                        <td hlmTd>
                          <div class="flex items-center gap-2">
                            <svg
                              class="h-3 flex-1"
                              viewBox="0 0 100 12"
                              preserveAspectRatio="none"
                              aria-hidden="true"
                            >
                              <rect x="0" y="5" width="100" height="2" rx="1" fill="var(--muted)" />
                              <rect
                                [attr.x]="lat(t.p50Ms)"
                                y="3"
                                [attr.width]="max(1, lat(t.p95Ms) - lat(t.p50Ms))"
                                height="6"
                                rx="3"
                                fill="var(--chart-1)"
                                opacity="0.35"
                              />
                              <circle [attr.cx]="lat(t.p50Ms)" cy="6" r="3" fill="var(--chart-1)" />
                              <rect
                                [attr.x]="lat(t.maxMs)"
                                y="1"
                                width="1.2"
                                height="10"
                                fill="var(--muted-foreground)"
                              />
                            </svg>
                            <span class="tabular text-muted-foreground w-24 text-right text-xs"
                              >{{ ms(t.p50Ms) }} · {{ ms(t.p95Ms) }}</span
                            >
                          </div>
                        </td>
                      </tr>
                    } @empty {
                      <tr hlmTr>
                        <td hlmTd colspan="4" class="text-muted-foreground">
                          No tool calls in range.
                        </td>
                      </tr>
                    }
                  </tbody>
                </table>
              </div>
              <p class="text-muted-foreground mt-2 text-xs">
                Latency axis is logarithmic, 10ms – {{ ms(latMax()) }}.
              </p>
            </div>
          </section>
          <div class="flex flex-col gap-4">
            <section hlmCard>
              <div hlmCardHeader>
                <h2 hlmCardTitle>When the agents work</h2>
                <p hlmCardDescription>Tool calls and replies by weekday and hour (local time)</p>
              </div>
              <div hlmCardContent>
                <app-heatmap [matrix]="d.heatmap" />
              </div>
            </section>
            <section hlmCard>
              <div hlmCardHeader>
                <h2 hlmCardTitle>Activity mix</h2>
                <p hlmCardDescription>Recorded activities by kind, and how replies ended</p>
              </div>
              <div hlmCardContent class="flex flex-col gap-4">
                <app-bar-list [items]="kindItems()" color="var(--chart-7)" />
                <div class="flex flex-wrap gap-1.5">
                  @for (s of stopItems(); track s.key) {
                    <span
                      hlmBadge
                      [variant]="
                        s.key === 'error' || s.key === 'aborted' ? 'destructive' : 'secondary'
                      "
                    >
                      {{ s.label }} · {{ s.display }}
                    </span>
                  }
                </div>
              </div>
            </section>
          </div>
        </div>

        <div class="grid gap-4 lg:grid-cols-2">
          <section hlmCard>
            <div hlmCardHeader>
              <h2 hlmCardTitle>Models</h2>
              <p hlmCardDescription>Replies per model · tokens · cost</p>
            </div>
            <div hlmCardContent>
              <app-bar-list [items]="modelItems()" />
            </div>
          </section>
          <section hlmCard>
            <div hlmCardHeader>
              <h2 hlmCardTitle>Most-changed files</h2>
              <p hlmCardDescription>edit and write calls</p>
            </div>
            <div hlmCardContent>
              <app-bar-list [items]="fileItems()" color="var(--chart-3)" />
            </div>
          </section>
        </div>
        <p class="text-muted-foreground text-xs">
          Updated {{ ago(d.until) }} ·
          <a routerLink="/runs" class="underline underline-offset-2">all runs in range</a>
        </p>
      } @else if (overview.isLoading()) {
        <div class="grid grid-cols-2 gap-3 md:grid-cols-3 xl:grid-cols-6">
          @for (i of [1, 2, 3, 4, 5, 6]; track i) {
            <div hlmSkeleton class="h-28 rounded-xl"></div>
          }
        </div>
        <div class="grid gap-4 xl:grid-cols-2">
          <div hlmSkeleton class="h-72 rounded-xl"></div>
          <div hlmSkeleton class="h-72 rounded-xl"></div>
        </div>
      }
    </div>
  `,
})
export class OverviewPage {
  protected readonly prefs = inject(Prefs);
  private readonly api = inject(Api);
  protected readonly tokenSeries = TOKEN_SERIES;
  protected readonly toolSeries = TOOL_SERIES;
  protected readonly count = fmtCount;
  protected readonly cost = fmtCost;
  protected readonly ms = fmtMs;
  protected readonly percent = (r: number | null | undefined) => fmtPercent(r);
  protected readonly ago = fmtAgo;
  protected readonly hit = cacheHit;
  protected readonly change = delta;
  protected readonly max = Math.max;

  protected readonly overview = resource({
    params: () => ({
      range: this.prefs.rangeMs(),
      source: this.prefs.source(),
      tick: this.prefs.refreshTick(),
    }),
    loader: ({ params, abortSignal }) =>
      this.api.overview(params.range, params.source, abortSignal),
  });
  protected readonly data = computed(() =>
    this.overview.hasValue() ? this.overview.value() : undefined,
  );

  protected tokens(t: { input: number; output: number; cacheRead: number; cacheWrite: number }) {
    return t.input + t.output + t.cacheRead + t.cacheWrite;
  }

  protected errorRate(t: { toolCalls: number; toolErrors: number }) {
    return t.toolCalls ? t.toolErrors / t.toolCalls : null;
  }

  protected readonly bucketName = computed(() => {
    const bucket = this.data()?.bucketMs ?? 3_600_000;
    return (x: string) => fmtBucket(Number(x), bucket);
  });

  protected readonly bucketLabel = computed(() => {
    const ms = this.data()?.bucketMs ?? 0;
    if (ms >= 7 * 86_400_000) return 'week';
    if (ms >= 86_400_000) return 'day';
    return ms >= 3_600_000 ? `${ms / 3_600_000}h` : `${Math.round(ms / 60_000)}m`;
  });

  protected spark(field: 'runs' | 'turns' | 'toolCalls' | 'tokens' | 'cost'): number[] {
    return (this.data()?.series ?? []).map((p) =>
      field === 'tokens' ? p.input + p.output + p.cacheRead + p.cacheWrite : p[field],
    );
  }

  protected readonly tokenRows = computed<BarRow[]>(() => {
    const d = this.data();
    if (!d) return [];
    return d.series.flatMap((p) =>
      TOKEN_SERIES.map((s) => ({
        x: String(p.t),
        series: s.key,
        value: p[s.key as 'input' | 'cacheRead' | 'cacheWrite' | 'output'],
      })),
    );
  });

  protected readonly toolRows = computed<BarRow[]>(() => {
    const d = this.data();
    if (!d) return [];
    return d.series.flatMap((p) => [
      { x: String(p.t), series: 'ok', value: p.toolCalls - p.toolErrors },
      { x: String(p.t), series: 'error', value: p.toolErrors },
    ]);
  });

  protected readonly latMax = computed(() =>
    Math.max(1000, ...(this.data()?.tools ?? []).map((t) => t.maxMs ?? 0)),
  );

  /** Log-scale position (0-100) of a latency between 10ms and the max. */
  protected lat(ms: number | null | undefined): number {
    if (!ms || ms <= 10) return 0;
    return Math.min(100, (Math.log10(ms / 10) / Math.log10(this.latMax() / 10)) * 100);
  }

  protected readonly modelItems = computed<BarItem[]>(() =>
    (this.data()?.models ?? []).slice(0, 8).map((m) => ({
      key: m.model,
      label: m.model,
      value: m.turns,
      display: fmtCount(m.turns),
      sub: `${fmtCount(m.input + m.output + m.cacheRead + m.cacheWrite)} tok · ${fmtCost(m.cost)}${m.errors ? ` · ${m.errors} failed` : ''}`,
    })),
  );

  protected readonly fileItems = computed<BarItem[]>(() =>
    (this.data()?.files ?? []).slice(0, 8).map((f) => ({
      key: f.path,
      label: shortPath(f.path),
      value: f.edits + f.writes,
      display: fmtCount(f.edits + f.writes),
      sub: `${f.runs} run${f.runs === 1 ? '' : 's'} · ${fmtAgo(f.lastTs)}`,
    })),
  );

  protected readonly kindItems = computed<BarItem[]>(() => {
    const kinds = this.data()?.kinds ?? {};
    return KINDS.filter((k) => kinds[k.kind]).map((k) => ({
      key: k.kind,
      label: k.label,
      value: kinds[k.kind],
      display: fmtCount(kinds[k.kind]),
    }));
  });

  protected readonly stopItems = computed(() =>
    Object.entries(this.data()?.stopReasons ?? {})
      .sort((a, b) => b[1] - a[1])
      .map(([key, value]) => ({ key, label: key, display: fmtCount(value) })),
  );
}
