import { ChangeDetectionStrategy, Component, computed, input, output, signal } from '@angular/core';
import { NgIcon, provideIcons } from '@ng-icons/core';
import { lucideCopy, lucideX } from '@ng-icons/lucide';
import { toast } from '@spartan-ng/brain/sonner';
import { HlmBadgeImports } from '@spartan-ng/helm/badge';
import { HlmButtonImports } from '@spartan-ng/helm/button';
import { HlmTabsImports } from '@spartan-ng/helm/tabs';
import { HlmToggleGroupImports } from '@spartan-ng/helm/toggle-group';
import { fileChanges } from '../core/diff';
import { fmtCost, fmtCount, fmtDateTime, fmtMs } from '../core/format';
import { kindMeta, prettyJson, threadColorVar, turnUsage } from '../core/items';
import type { ObserveItem } from '../core/types';
import { DiffView, type DiffMode } from './diff-view';

/** Everything recorded about one activity. */
@Component({
  selector: 'app-inspector',
  imports: [
    HlmTabsImports,
    HlmButtonImports,
    HlmBadgeImports,
    HlmToggleGroupImports,
    NgIcon,
    DiffView,
  ],
  providers: [provideIcons({ lucideCopy, lucideX })],
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { class: 'flex h-full min-h-0 flex-col' },
  styles: `
    pre {
      font-family: var(--font-mono);
      font-size: 12px;
      line-height: 1.5;
      white-space: pre-wrap;
      word-break: break-word;
    }
  `,
  template: `
    @let it = item();
    <header class="flex items-start gap-2 border-b px-4 py-3">
      <span class="mt-1.5 size-2.5 shrink-0 rounded-full" [style.background]="color()"></span>
      <div class="min-w-0 flex-1">
        <h2 class="truncate text-sm font-semibold">{{ title() }}</h2>
        <p class="text-muted-foreground truncate text-xs">{{ it.threadName }} · {{ time() }}</p>
      </div>
      <button hlmBtn variant="ghost" size="icon-sm" (click)="copy()" aria-label="Copy event JSON">
        <ng-icon name="lucideCopy" />
      </button>
      <button
        hlmBtn
        variant="ghost"
        size="icon-sm"
        (click)="closed.emit()"
        aria-label="Close inspector"
      >
        <ng-icon name="lucideX" />
      </button>
    </header>
    <hlm-tabs
      [tab]="tab()"
      (tabActivated)="tab.set($event)"
      class="flex min-h-0 flex-1 flex-col gap-0"
    >
      <hlm-tabs-list class="mx-4 mt-3 w-auto self-start" aria-label="Event sections">
        <button hlmTabsTrigger="details">Details</button>
        @if (it.args) {
          <button hlmTabsTrigger="input">Input</button>
        }
        @if (it.result || it.text) {
          <button hlmTabsTrigger="output">{{ it.kind === 'tool' ? 'Output' : 'Text' }}</button>
        }
        @if (changes().length) {
          <button hlmTabsTrigger="diff">Diff</button>
        }
        @if (it.raw) {
          <button hlmTabsTrigger="raw">Raw</button>
        }
      </hlm-tabs-list>
      <div class="min-h-0 flex-1 overflow-auto px-4 py-3">
        <div hlmTabsContent="details">
          <dl class="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-1.5 text-sm">
            @for (row of details(); track row[0]) {
              <dt class="text-muted-foreground">{{ row[0] }}</dt>
              <dd class="tabular truncate" [title]="row[1]">{{ row[1] }}</dd>
            }
          </dl>
          @if (usage(); as u) {
            <h3 class="mt-4 mb-2 text-xs font-medium">Tokens</h3>
            <dl class="grid grid-cols-2 gap-2 text-sm sm:grid-cols-4">
              @for (cell of usageCells(); track cell[0]) {
                <div class="bg-muted/50 rounded-md px-2 py-1.5">
                  <dt class="text-muted-foreground text-xs">{{ cell[0] }}</dt>
                  <dd class="tabular font-medium">{{ cell[1] }}</dd>
                </div>
              }
            </dl>
          }
        </div>
        <div hlmTabsContent="input">
          <pre>{{ args() }}</pre>
        </div>
        <div hlmTabsContent="output">
          <pre>{{ it.result ?? it.text }}</pre>
        </div>
        <div hlmTabsContent="diff" class="flex flex-col gap-3">
          <hlm-toggle-group
            type="single"
            size="sm"
            variant="outline"
            [value]="mode()"
            (valueChange)="mode.set($event === 'split' ? 'split' : 'unified')"
            aria-label="Diff layout"
          >
            <button hlmToggleGroupItem value="unified">Unified</button>
            <button hlmToggleGroupItem value="split">Split</button>
          </hlm-toggle-group>
          @for (change of changes(); track change.key) {
            <p class="text-muted-foreground text-xs">
              <span class="text-foreground font-mono">{{ change.path }}</span>
              · <span class="text-success">+{{ change.diff.added }}</span>
              <span class="text-destructive">−{{ change.diff.removed }}</span>
              · from {{ change.basis }}
            </p>
            <app-diff-view [diff]="change.diff" [mode]="mode()" />
          }
        </div>
        <div hlmTabsContent="raw">
          <pre>{{ raw() }}</pre>
        </div>
      </div>
    </hlm-tabs>
  `,
})
export class Inspector {
  readonly item = input.required<ObserveItem>();
  /** Earlier activities of the run, so a `write` can diff against them. */
  readonly context = input<ObserveItem[]>([]);
  readonly closed = output<void>();
  protected readonly tab = signal('details');
  protected readonly mode = signal<DiffMode>('unified');

  protected readonly color = computed(() => threadColorVar(this.item().threadColor));
  protected readonly time = computed(() => fmtDateTime(this.item().ts));
  protected readonly title = computed(() => {
    const it = this.item();
    if (it.kind === 'tool') return it.toolName ?? 'tool';
    if (it.kind === 'message') return `${it.role ?? 'message'} message`;
    return kindMeta(it.kind).label;
  });
  protected readonly args = computed(() => prettyJson(this.item().args));
  protected readonly raw = computed(() => prettyJson(this.item().raw));
  protected readonly usage = computed(() => turnUsage(this.item()));
  protected readonly usageCells = computed(() => {
    const u = this.usage();
    if (!u) return [];
    return [
      ['Input', fmtCount(u.input)],
      ['Output', fmtCount(u.output)],
      ['Cache read', fmtCount(u.cacheRead)],
      ['Cost', fmtCost(u.cost)],
    ];
  });
  protected readonly changes = computed(() => {
    const it = this.item();
    if (it.kind !== 'tool') return [];
    const earlier = this.context().filter(
      (c) => c.ts <= it.ts && c.id !== it.id && c.runId === it.runId,
    );
    const groups = fileChanges([...earlier, it]);
    return groups
      .flatMap((g) => g.changes)
      .filter((c) => c.item.id === it.id && c.item.runId === it.runId);
  });
  protected readonly details = computed(() => {
    const it = this.item();
    const rows: Array<[string, string]> = [
      ['Kind', it.kind + (it.eventType ? ` · ${it.eventType}` : '')],
      ['State', it.state ?? '–'],
      ['Started', fmtDateTime(it.ts)],
    ];
    if (it.durationMs !== undefined) rows.push(['Duration', fmtMs(it.durationMs)]);
    if (it.updates) rows.push(['Updates', String(it.updates)]);
    if (it.toolCallId) rows.push(['Call id', it.toolCallId]);
    if (it.delegationToolName) rows.push(['Delegated by', it.delegationToolName]);
    rows.push(
      ['Thread', `${it.threadName} (${it.threadKind})`],
      ['Run', it.runId],
      ['Item', String(it.id)],
    );
    const u = this.usage();
    if (u?.model) rows.push(['Model', u.model]);
    if (u?.stopReason) rows.push(['Stop reason', u.stopReason]);
    return rows;
  });

  async copy() {
    try {
      await navigator.clipboard.writeText(JSON.stringify(this.item(), null, 2));
      toast('Event JSON copied');
    } catch {
      toast.error('Clipboard unavailable');
    }
  }
}
