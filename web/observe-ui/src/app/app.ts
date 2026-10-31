import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core';
import { Router, RouterLink, RouterLinkActive, RouterOutlet } from '@angular/router';
import { NgIcon, provideIcons } from '@ng-icons/core';
import {
  lucideActivity,
  lucideChartColumn,
  lucideCircle,
  lucideDatabase,
  lucideGitCompareArrows,
  lucideList,
  lucideMoon,
  lucideRadio,
  lucideRefreshCw,
  lucideSearch,
  lucideSun,
} from '@ng-icons/lucide';
import { HlmButtonImports } from '@spartan-ng/helm/button';
import { HlmCommandImports } from '@spartan-ng/helm/command';
import { HlmKbdImports } from '@spartan-ng/helm/kbd';
import { HlmNativeSelectImports } from '@spartan-ng/helm/native-select';
import { HlmSidebarImports } from '@spartan-ng/helm/sidebar';
import { HlmToasterImports } from '@spartan-ng/helm/sonner';
import { HlmToggleGroupImports } from '@spartan-ng/helm/toggle-group';
import { HlmTooltipImports } from '@spartan-ng/helm/tooltip';
import { Prefs, RANGES, type RangeId } from './core/api';
import { fmtAgo, fmtCount, runActivity, runLabel, shortId } from './core/format';
import { MetaStore, RunsStore } from './core/stores';

interface Command {
  id: string;
  label: string;
  hint?: string;
  icon: string;
  run: () => void;
}

@Component({
  selector: 'app-root',
  imports: [
    RouterOutlet,
    RouterLink,
    RouterLinkActive,
    NgIcon,
    HlmSidebarImports,
    HlmButtonImports,
    HlmToggleGroupImports,
    HlmNativeSelectImports,
    HlmCommandImports,
    HlmKbdImports,
    HlmToasterImports,
    HlmTooltipImports,
  ],
  providers: [
    provideIcons({
      lucideActivity,
      lucideChartColumn,
      lucideCircle,
      lucideDatabase,
      lucideGitCompareArrows,
      lucideList,
      lucideMoon,
      lucideRadio,
      lucideRefreshCw,
      lucideSearch,
      lucideSun,
    }),
  ],
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { '(document:keydown)': 'onKey($event)' },
  template: `
    <div hlmSidebarWrapper>
      <hlm-sidebar collapsible="icon">
        <div hlmSidebarHeader>
          <a routerLink="/" class="flex items-center gap-2 px-1 py-1.5">
            <span
              class="bg-primary text-primary-foreground grid size-7 shrink-0 place-items-center rounded-md"
            >
              <ng-icon name="lucideActivity" />
            </span>
            <span class="flex min-w-0 flex-col leading-tight group-data-[collapsible=icon]:hidden">
              <span class="truncate text-sm font-semibold">icers observe</span>
              <span class="text-muted-foreground truncate text-xs"
                >v{{ meta.meta.value()?.version ?? '…' }}</span
              >
            </span>
          </a>
        </div>
        <div hlmSidebarContent>
          <div hlmSidebarGroup>
            <div hlmSidebarGroupLabel>Views</div>
            <div hlmSidebarGroupContent>
              <ul hlmSidebarMenu>
                @for (nav of navs; track nav.path) {
                  <li hlmSidebarMenuItem>
                    <a
                      hlmSidebarMenuButton
                      [routerLink]="nav.path"
                      routerLinkActive
                      #rla="routerLinkActive"
                      [routerLinkActiveOptions]="{ exact: nav.path === '/' }"
                      [isActive]="rla.isActive"
                      [tooltip]="nav.label"
                    >
                      <ng-icon [name]="nav.icon" />
                      <span>{{ nav.label }}</span>
                    </a>
                  </li>
                }
              </ul>
            </div>
          </div>
          <div hlmSidebarGroup class="group-data-[collapsible=icon]:hidden">
            <div hlmSidebarGroupLabel>Recent runs</div>
            <div hlmSidebarGroupContent>
              <ul hlmSidebarMenu>
                @for (run of recent(); track run.runId) {
                  <li hlmSidebarMenuItem>
                    <a
                      hlmSidebarMenuButton
                      size="sm"
                      [routerLink]="['/runs', run.runId]"
                      class="h-auto py-1.5"
                    >
                      <span
                        class="size-2 shrink-0 rounded-full"
                        [class]="
                          activity(run) === 'live'
                            ? 'bg-success'
                            : activity(run) === 'stale'
                              ? 'bg-warning'
                              : 'bg-muted-foreground/40'
                        "
                      ></span>
                      <span class="flex min-w-0 flex-col">
                        <span class="truncate" [title]="label(run)">{{ label(run) }}</span>
                        <span class="text-muted-foreground truncate text-[11px]">
                          {{ ago(run.startedAt) }} · {{ count(run.toolCalls) }} tools
                        </span>
                      </span>
                    </a>
                  </li>
                } @empty {
                  <li class="text-muted-foreground px-2 text-xs">No runs in range.</li>
                }
              </ul>
            </div>
          </div>
        </div>
        <div hlmSidebarFooter class="group-data-[collapsible=icon]:hidden">
          <div class="bg-muted/50 flex flex-col gap-1.5 rounded-md p-2 text-xs">
            <div class="flex items-center gap-2 font-medium">
              <ng-icon name="lucideDatabase" class="text-muted-foreground" /> History
            </div>
            <div class="text-muted-foreground flex items-center gap-2">
              <span class="bg-success size-1.5 rounded-full"></span> SQLite · {{ retentionDays() }}d
              retention
            </div>
            @if (meta.meta.value()?.clickhouse; as ch) {
              <div
                class="text-muted-foreground flex items-center gap-2"
                [title]="
                  ch.lastError ??
                  ch.url +
                    '/' +
                    ch.database +
                    ' · ' +
                    count(ch.eventsSent) +
                    ' events sent by this process'
                "
              >
                <span
                  class="size-1.5 rounded-full"
                  [class]="
                    ch.connected === false
                      ? 'bg-destructive'
                      : ch.connected
                        ? 'bg-success'
                        : 'bg-muted-foreground'
                  "
                ></span>
                ClickHouse ·
                {{
                  ch.connected === false
                    ? 'unreachable'
                    : ch.lastSyncAt
                      ? 'synced ' + ago(ch.lastSyncAt)
                      : 'connecting'
                }}
              </div>
            }
          </div>
        </div>
      </hlm-sidebar>
      <main hlmSidebarInset class="min-w-0">
        <header
          class="bg-background/90 sticky top-0 z-10 flex h-12 shrink-0 items-center gap-2 border-b px-3 backdrop-blur"
        >
          <button hlmSidebarTrigger aria-label="Toggle sidebar"></button>
          <div class="ml-auto flex items-center gap-2">
            <button
              hlmBtn
              variant="outline"
              size="sm"
              class="text-muted-foreground hidden w-56 justify-start gap-2 md:flex"
              (click)="paletteOpen.set(true)"
            >
              <ng-icon name="lucideSearch" /> Jump to…
              <kbd hlmKbd class="ml-auto">Ctrl K</kbd>
            </button>
            <hlm-toggle-group
              type="single"
              size="sm"
              variant="outline"
              [value]="prefs.range()"
              (valueChange)="setRange($event)"
              aria-label="Time range"
            >
              @for (r of ranges; track r.id) {
                <button hlmToggleGroupItem [value]="r.id">{{ r.label }}</button>
              }
            </hlm-toggle-group>
            @if (meta.sources().length > 1) {
              <hlm-native-select
                size="sm"
                [value]="prefs.source()"
                (valueChange)="prefs.source.set($event ?? '')"
                aria-label="History source"
              >
                <option hlmNativeSelectOption value="">
                  Auto ({{ meta.meta.value()?.defaultSource }})
                </option>
                @for (s of meta.sources(); track s) {
                  <option hlmNativeSelectOption [value]="s">{{ s }}</option>
                }
              </hlm-native-select>
            }
            <button
              hlmBtn
              variant="ghost"
              size="icon-sm"
              (click)="prefs.refresh()"
              aria-label="Refresh"
              hlmTooltip="Refresh"
            >
              <ng-icon name="lucideRefreshCw" />
            </button>
            <button
              hlmBtn
              variant="ghost"
              size="icon-sm"
              (click)="prefs.dark.set(!prefs.dark())"
              [attr.aria-label]="prefs.dark() ? 'Light theme' : 'Dark theme'"
            >
              <ng-icon [name]="prefs.dark() ? 'lucideSun' : 'lucideMoon'" />
            </button>
          </div>
        </header>
        <div class="min-w-0 flex-1">
          <router-outlet />
        </div>
      </main>
    </div>
    <hlm-command-dialog
      [state]="paletteOpen() ? 'open' : 'closed'"
      (stateChange)="paletteOpen.set($event === 'open')"
      title="Jump to"
      description="Pages, runs and actions"
    >
      <hlm-command>
        <hlm-command-input placeholder="Search pages, runs, actions…" />
        <div hlmCommandList>
          <div hlmCommandEmpty>No match.</div>
          @for (group of commandGroups(); track group.label) {
            <hlm-command-group>
              <hlm-command-group-label>{{ group.label }}</hlm-command-group-label>
              @for (cmd of group.items; track cmd.id) {
                <button
                  hlmCommandItem
                  [value]="cmd.label + ' ' + (cmd.hint ?? '')"
                  (selected)="runCommand(cmd)"
                >
                  <ng-icon [name]="cmd.icon" />
                  <span class="truncate">{{ cmd.label }}</span>
                  @if (cmd.hint) {
                    <span class="text-muted-foreground ml-auto truncate text-xs">{{
                      cmd.hint
                    }}</span>
                  }
                </button>
              }
            </hlm-command-group>
          }
        </div>
      </hlm-command>
    </hlm-command-dialog>
    <hlm-toaster />
  `,
})
export class App {
  protected readonly prefs = inject(Prefs);
  protected readonly meta = inject(MetaStore);
  private readonly runsStore = inject(RunsStore);
  private readonly router = inject(Router);
  protected readonly ranges = RANGES;
  protected readonly paletteOpen = signal(false);
  protected readonly shortId = shortId;
  protected readonly ago = fmtAgo;
  protected readonly count = fmtCount;
  protected readonly label = runLabel;
  protected readonly activity = (run: Parameters<typeof runActivity>[0]) => runActivity(run);
  protected readonly navs = [
    { path: '/', label: 'Overview', icon: 'lucideChartColumn' },
    { path: '/live', label: 'Live', icon: 'lucideRadio' },
    { path: '/runs', label: 'Runs', icon: 'lucideList' },
    { path: '/compare', label: 'Compare', icon: 'lucideGitCompareArrows' },
  ];
  protected readonly recent = computed(() => this.runsStore.runs().slice(0, 6));
  protected readonly retentionDays = computed(() =>
    Math.round((this.meta.meta.value()?.retentionMs ?? 30 * 86_400_000) / 86_400_000),
  );

  protected readonly commandGroups = computed(() => {
    const pages: Command[] = this.navs.map((nav) => ({
      id: `page:${nav.path}`,
      label: nav.label,
      icon: nav.icon,
      run: () => void this.router.navigateByUrl(nav.path),
    }));
    const runs: Command[] = this.runsStore
      .runs()
      .slice(0, 50)
      .map((run) => ({
        id: `run:${run.runId}`,
        label: runLabel(run),
        hint: `${run.model ?? ''} · ${fmtAgo(run.startedAt)}`,
        icon: runActivity(run) === 'live' ? 'lucideRadio' : 'lucideCircle',
        run: () => void this.router.navigate(['/runs', run.runId]),
      }));
    const actions: Command[] = [
      {
        id: 'theme',
        label: this.prefs.dark() ? 'Light theme' : 'Dark theme',
        icon: this.prefs.dark() ? 'lucideSun' : 'lucideMoon',
        run: () => this.prefs.dark.set(!this.prefs.dark()),
      },
      {
        id: 'refresh',
        label: 'Refresh data',
        icon: 'lucideRefreshCw',
        run: () => this.prefs.refresh(),
      },
      ...RANGES.map((r) => ({
        id: `range:${r.id}`,
        label: `Range: last ${r.label}`,
        icon: 'lucideChartColumn',
        run: () => this.prefs.range.set(r.id),
      })),
    ];
    return [
      { label: 'Pages', items: pages },
      { label: 'Runs', items: runs },
      { label: 'Actions', items: actions },
    ].filter((g) => g.items.length);
  });

  protected setRange(value: unknown) {
    if (RANGES.some((r) => r.id === value)) this.prefs.range.set(value as RangeId);
  }

  protected runCommand(cmd: Command) {
    this.paletteOpen.set(false);
    cmd.run();
  }

  protected onKey(event: KeyboardEvent) {
    if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'k') {
      event.preventDefault();
      this.paletteOpen.set(!this.paletteOpen());
    }
  }
}
