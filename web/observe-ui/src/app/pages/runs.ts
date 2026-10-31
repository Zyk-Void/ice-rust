import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core';
import { Router, RouterLink } from '@angular/router';
import { NgIcon, provideIcons } from '@ng-icons/core';
import {
  lucideArrowDown,
  lucideArrowUp,
  lucideChevronLeft,
  lucideChevronRight,
  lucideGitCompareArrows,
  lucideSearch,
  lucideTriangleAlert,
} from '@ng-icons/lucide';
import { HlmAlertImports } from '@spartan-ng/helm/alert';
import { HlmBadgeImports } from '@spartan-ng/helm/badge';
import { HlmButtonImports } from '@spartan-ng/helm/button';
import { HlmCardImports } from '@spartan-ng/helm/card';
import { HlmCheckboxImports } from '@spartan-ng/helm/checkbox';
import { HlmEmptyImports } from '@spartan-ng/helm/empty';
import { HlmInputGroupImports } from '@spartan-ng/helm/input-group';
import { HlmNativeSelectImports } from '@spartan-ng/helm/native-select';
import { HlmSkeletonImports } from '@spartan-ng/helm/skeleton';
import { HlmTableImports } from '@spartan-ng/helm/table';
import { HlmSwitchImports } from '@spartan-ng/helm/switch';
import { HlmToggleGroupImports } from '@spartan-ng/helm/toggle-group';
import { Prefs } from '../core/api';
import {
  fmtAgo,
  fmtCost,
  fmtCount,
  fmtDateTime,
  fmtMs,
  fmtPercent,
  runActivity,
  runLabel,
  shortId,
} from '../core/format';
import { cacheHit } from '../core/items';
import { RunsStore } from '../core/stores';
import type { RunRow } from '../core/types';

type SortKey = 'startedAt' | 'durationMs' | 'turns' | 'toolCalls' | 'tokens' | 'cost' | 'files';
const PAGE = 25;

function tokens(r: RunRow) {
  return r.input + r.output + r.cacheRead + r.cacheWrite;
}

@Component({
  selector: 'app-runs',
  imports: [
    RouterLink,
    NgIcon,
    HlmTableImports,
    HlmCardImports,
    HlmBadgeImports,
    HlmButtonImports,
    HlmCheckboxImports,
    HlmInputGroupImports,
    HlmNativeSelectImports,
    HlmToggleGroupImports,
    HlmSwitchImports,
    HlmEmptyImports,
    HlmSkeletonImports,
    HlmAlertImports,
  ],
  providers: [
    provideIcons({
      lucideArrowDown,
      lucideArrowUp,
      lucideChevronLeft,
      lucideChevronRight,
      lucideGitCompareArrows,
      lucideSearch,
      lucideTriangleAlert,
    }),
  ],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <div class="mx-auto flex max-w-[1400px] flex-col gap-4 p-4 md:p-6">
      <div class="flex flex-wrap items-end justify-between gap-3">
        <div>
          <h1 class="text-xl font-semibold tracking-tight">Runs</h1>
          <p class="text-muted-foreground text-sm">
            {{ filtered().length }} of {{ store.runs().length }} runs started in the last
            {{ prefs.range() }}
          </p>
        </div>
        <button hlmBtn size="sm" [disabled]="picked().length !== 2" (click)="compare()">
          <ng-icon name="lucideGitCompareArrows" /> Compare {{ picked().length }}/2
        </button>
      </div>

      @if (store.list.error(); as error) {
        <div hlmAlert variant="destructive">
          <h4 hlmAlertTitle>Could not load runs</h4>
          <p hlmAlertDescription>{{ error.message }}</p>
        </div>
      }
      @if (store.list.hasValue() && store.list.value().sourceError; as sourceError) {
        <div hlmAlert>
          <h4 hlmAlertTitle>Showing local history</h4>
          <p hlmAlertDescription>{{ sourceError }}</p>
        </div>
      }

      <div class="flex flex-wrap items-center gap-2">
        <hlm-input-group class="w-full max-w-sm">
          <hlm-input-group-addon><ng-icon name="lucideSearch" /></hlm-input-group-addon>
          <input
            hlmInputGroupInput
            type="search"
            placeholder="Search name, model, run id, tool"
            aria-label="Search runs"
            [value]="query()"
            (input)="query.set($any($event.target).value); page.set(0)"
          />
        </hlm-input-group>
        <hlm-native-select
          size="sm"
          [value]="model()"
          (valueChange)="model.set($event ?? ''); page.set(0)"
          aria-label="Model"
        >
          <option hlmNativeSelectOption value="">All models</option>
          @for (m of models(); track m) {
            <option hlmNativeSelectOption [value]="m">{{ m }}</option>
          }
        </hlm-native-select>
        <hlm-toggle-group
          type="single"
          size="sm"
          variant="outline"
          [value]="status()"
          (valueChange)="status.set($any($event) || 'all'); page.set(0)"
          aria-label="Status"
        >
          <button hlmToggleGroupItem value="all">All</button>
          <button hlmToggleGroupItem value="running">Running</button>
          <button hlmToggleGroupItem value="finished">Finished</button>
          <button hlmToggleGroupItem value="errors">With errors</button>
        </hlm-toggle-group>
        <label class="text-muted-foreground flex items-center gap-2 text-xs">
          <hlm-switch
            [checked]="hideEmpty()"
            (checkedChange)="hideEmpty.set($event); page.set(0)"
            aria-label="Hide empty runs"
          />
          Hide empty
        </label>
      </div>

      <section hlmCard class="py-0">
        <div hlmTableContainer>
          <table hlmTable>
            <thead hlmTHead>
              <tr hlmTr>
                <th hlmTh class="w-10"><span class="sr-only">Select</span></th>
                <th hlmTh>Run</th>
                <th hlmTh>Model</th>
                @for (col of columns; track col.key) {
                  <th hlmTh [class.text-right]="col.key !== 'startedAt'">
                    <button
                      type="button"
                      class="hover:text-foreground inline-flex items-center gap-1"
                      (click)="sortBy(col.key)"
                      [attr.aria-sort]="
                        sort() === col.key ? (desc() ? 'descending' : 'ascending') : null
                      "
                    >
                      {{ col.label }}
                      @if (sort() === col.key) {
                        <ng-icon [name]="desc() ? 'lucideArrowDown' : 'lucideArrowUp'" />
                      }
                    </button>
                  </th>
                }
                <th hlmTh>Top tools</th>
              </tr>
            </thead>
            <tbody hlmTBody>
              @for (run of pageRows(); track run.runId) {
                <tr hlmTr class="cursor-pointer" (click)="open(run)">
                  <td hlmTd (click)="$event.stopPropagation()">
                    <hlm-checkbox
                      [checked]="picked().includes(run.runId)"
                      (checkedChange)="toggle(run.runId, $event)"
                      [disabled]="!picked().includes(run.runId) && picked().length >= 2"
                      [attr.aria-label]="'Select ' + label(run) + ' for comparison'"
                    />
                  </td>
                  <td hlmTd class="max-w-[16rem]">
                    <a
                      [routerLink]="['/runs', run.runId]"
                      class="flex items-center gap-2"
                      (click)="$event.stopPropagation()"
                    >
                      <span
                        class="size-2 shrink-0 rounded-full"
                        [class]="activityDot(run)"
                        [attr.aria-label]="activity(run)"
                        [title]="activityTitle(run)"
                      ></span>
                      <span class="flex min-w-0 flex-col">
                        <span class="truncate font-medium" [title]="label(run)">{{
                          label(run)
                        }}</span>
                        <span class="text-muted-foreground truncate font-mono text-[11px]">{{
                          shortId(run.runId, 12)
                        }}</span>
                      </span>
                    </a>
                  </td>
                  <td hlmTd class="text-muted-foreground max-w-[12rem] truncate text-xs">
                    {{ run.model ?? '–' }}
                  </td>
                  <td hlmTd class="text-muted-foreground text-xs" [title]="dateTime(run.startedAt)">
                    {{ ago(run.startedAt) }}
                  </td>
                  <td hlmTd class="tabular text-right">{{ ms(run.durationMs) }}</td>
                  <td hlmTd class="tabular text-right">
                    {{ count(run.turns) }}
                    @if (run.turnErrors) {
                      <span class="text-destructive text-xs">({{ run.turnErrors }}✕)</span>
                    }
                  </td>
                  <td hlmTd class="tabular text-right">
                    {{ count(run.toolCalls) }}
                    @if (run.toolErrors) {
                      <span class="text-destructive text-xs">({{ run.toolErrors }}✕)</span>
                    }
                  </td>
                  <td hlmTd class="tabular text-right" [title]="'cache hit ' + percent(hit(run))">
                    {{ count(total(run)) }}
                  </td>
                  <td hlmTd class="tabular text-right">{{ cost(run.cost) }}</td>
                  <td hlmTd class="tabular text-right">{{ run.files || '–' }}</td>
                  <td hlmTd>
                    <div class="flex max-w-[20rem] flex-wrap gap-1">
                      @for (t of run.topTools; track t.name) {
                        <span hlmBadge variant="outline" class="font-mono text-[10px]"
                          >{{ t.name }} {{ t.count }}</span
                        >
                      }
                    </div>
                  </td>
                </tr>
              } @empty {
                <tr hlmTr>
                  <td hlmTd colspan="11">
                    @if (store.list.isLoading()) {
                      <div class="flex flex-col gap-2 py-2">
                        @for (i of [1, 2, 3, 4]; track i) {
                          <div hlmSkeleton class="h-8 w-full"></div>
                        }
                      </div>
                    } @else {
                      <div hlmEmpty class="py-10">
                        <div hlmEmptyHeader>
                          <h3 hlmEmptyTitle>No runs</h3>
                          <p hlmEmptyDescription>Nothing matches in the selected range.</p>
                        </div>
                      </div>
                    }
                  </td>
                </tr>
              }
            </tbody>
          </table>
        </div>
      </section>

      @if (pages() > 1) {
        <nav class="flex items-center justify-end gap-2" aria-label="Pagination">
          <span class="text-muted-foreground text-sm">Page {{ page() + 1 }} of {{ pages() }}</span>
          <button
            hlmBtn
            variant="outline"
            size="icon-sm"
            [disabled]="page() === 0"
            (click)="page.set(page() - 1)"
            aria-label="Previous page"
          >
            <ng-icon name="lucideChevronLeft" />
          </button>
          <button
            hlmBtn
            variant="outline"
            size="icon-sm"
            [disabled]="page() + 1 >= pages()"
            (click)="page.set(page() + 1)"
            aria-label="Next page"
          >
            <ng-icon name="lucideChevronRight" />
          </button>
        </nav>
      }
    </div>
  `,
})
export class RunsPage {
  protected readonly prefs = inject(Prefs);
  protected readonly store = inject(RunsStore);
  private readonly router = inject(Router);
  protected readonly query = signal('');
  protected readonly model = signal('');
  protected readonly hideEmpty = signal(true);
  protected readonly status = signal<'all' | 'running' | 'finished' | 'errors'>('all');
  protected readonly sort = signal<SortKey>('startedAt');
  protected readonly desc = signal(true);
  protected readonly page = signal(0);
  protected readonly picked = signal<string[]>([]);
  protected readonly columns: Array<{ key: SortKey; label: string }> = [
    { key: 'startedAt', label: 'Started' },
    { key: 'durationMs', label: 'Duration' },
    { key: 'turns', label: 'Replies' },
    { key: 'toolCalls', label: 'Tools' },
    { key: 'tokens', label: 'Tokens' },
    { key: 'cost', label: 'Cost' },
    { key: 'files', label: 'Files' },
  ];
  protected readonly count = fmtCount;
  protected readonly cost = fmtCost;
  protected readonly ms = fmtMs;
  protected readonly ago = fmtAgo;
  protected readonly dateTime = fmtDateTime;
  protected readonly shortId = shortId;
  protected readonly total = tokens;
  protected readonly hit = cacheHit;
  protected readonly percent = (r: number | null) => fmtPercent(r);

  protected readonly models = computed(() =>
    [
      ...new Set(
        this.store
          .runs()
          .flatMap((r) => [r.model ?? '', ...r.models])
          .filter(Boolean),
      ),
    ].sort(),
  );

  protected readonly filtered = computed(() => {
    const q = this.query().trim().toLowerCase();
    const model = this.model();
    const status = this.status();
    const key = this.sort();
    const dir = this.desc() ? -1 : 1;
    const value = (r: RunRow) => (key === 'tokens' ? tokens(r) : (r[key] ?? 0));
    return this.store
      .runs()
      .filter((r) => {
        if (model && r.model !== model && !r.models.includes(model)) return false;
        if (status === 'running' && runActivity(r) !== 'live') return false;
        if (status === 'finished' && runActivity(r) === 'live') return false;
        if (this.hideEmpty() && !r.turns && !r.toolCalls) return false;
        if (status === 'errors' && !r.toolErrors && !r.turnErrors) return false;
        if (!q) return true;
        return [
          r.sessionName,
          r.prompt,
          r.runId,
          r.model,
          r.sessionId,
          ...r.models,
          ...r.topTools.map((t) => t.name),
        ]
          .filter(Boolean)
          .some((s) => s!.toLowerCase().includes(q));
      })
      .sort((a, b) => (value(a) - value(b)) * dir);
  });
  protected readonly pages = computed(() => Math.max(1, Math.ceil(this.filtered().length / PAGE)));
  protected readonly pageRows = computed(() =>
    this.filtered().slice(this.page() * PAGE, (this.page() + 1) * PAGE),
  );

  protected label(run: RunRow) {
    return runLabel(run);
  }

  protected activity(run: RunRow) {
    return runActivity(run);
  }

  protected activityDot(run: RunRow) {
    const state = runActivity(run);
    return state === 'live'
      ? 'bg-success animate-pulse'
      : state === 'stale'
        ? 'bg-warning'
        : 'bg-muted-foreground/40';
  }

  protected activityTitle(run: RunRow) {
    const state = runActivity(run);
    return state === 'stale'
      ? 'Never finished: no activity for over 10 minutes (crashed or killed?)'
      : state;
  }

  protected sortBy(key: SortKey) {
    if (this.sort() === key) this.desc.set(!this.desc());
    else {
      this.sort.set(key);
      this.desc.set(true);
    }
  }

  protected toggle(runId: string, on: boolean) {
    this.picked.update((list) =>
      on
        ? [...list.filter((id) => id !== runId), runId].slice(-2)
        : list.filter((id) => id !== runId),
    );
  }

  protected open(run: RunRow) {
    void this.router.navigate(['/runs', run.runId]);
  }

  protected compare() {
    const [a, b] = this.picked();
    void this.router.navigate(['/compare'], { queryParams: { a, b } });
  }
}
