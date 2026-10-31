import {
  ChangeDetectionStrategy,
  Component,
  computed,
  inject,
  input,
  resource,
} from '@angular/core';
import { Router, RouterLink } from '@angular/router';
import { diffArrays, diffWordsWithSpace } from 'diff';
import { HlmAlertImports } from '@spartan-ng/helm/alert';
import { HlmBadgeImports } from '@spartan-ng/helm/badge';
import { HlmCardImports } from '@spartan-ng/helm/card';
import { HlmEmptyImports } from '@spartan-ng/helm/empty';
import { HlmNativeSelectImports } from '@spartan-ng/helm/native-select';
import { HlmTableImports } from '@spartan-ng/helm/table';
import { Api } from '../core/api';
import { fileChanges } from '../core/diff';
import {
  fmtAgo,
  fmtCost,
  fmtCount,
  fmtMs,
  fmtPercent,
  oneLine,
  runLabel,
  shortId,
} from '../core/format';
import { actionSignature, cacheHit, isFailed, turnUsage } from '../core/items';
import { RunsStore } from '../core/stores';
import type { ObserveState } from '../core/types';

interface RunFacts {
  state: ObserveState;
  name: string;
  turns: number;
  tools: number;
  toolErrors: number;
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
  cost: number;
  spanMs: number;
  toolCounts: Map<string, number>;
  files: Set<string>;
  actions: string[];
  prompt: string;
}

function facts(state: ObserveState): RunFacts {
  const usage = { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, cost: 0 };
  let turns = 0;
  const toolCounts = new Map<string, number>();
  for (const item of state.items) {
    const u = turnUsage(item);
    if (u) {
      turns++;
      usage.input += u.input;
      usage.output += u.output;
      usage.cacheRead += u.cacheRead;
      usage.cacheWrite += u.cacheWrite;
      usage.cost += u.cost;
    }
    if (item.kind === 'tool')
      toolCounts.set(item.toolName ?? 'tool', (toolCounts.get(item.toolName ?? 'tool') ?? 0) + 1);
  }
  const tools = state.items.filter((i) => i.kind === 'tool');
  const end = Math.max(state.startedAt, ...state.items.map((i) => i.ts + (i.durationMs ?? 0)));
  return {
    state,
    name:
      state.sessionName ||
      oneLine(state.items.find((i) => i.kind === 'message' && i.role === 'user')?.text ?? '', 60) ||
      `run ${shortId(state.runId)}`,
    turns,
    tools: tools.length,
    toolErrors: tools.filter(isFailed).length,
    ...usage,
    spanMs: end - state.startedAt,
    toolCounts,
    files: new Set(fileChanges(state.items).map((f) => f.path)),
    actions: state.items
      .filter((i) => i.kind === 'tool' || (i.kind === 'message' && i.role === 'user'))
      .map(actionSignature),
    prompt: state.items.find((i) => i.kind === 'message' && i.role === 'user')?.text ?? '',
  };
}

/** Collapse unchanged stretches longer than this in the action diff. */
const KEEP = 3;

@Component({
  selector: 'app-compare',
  imports: [
    RouterLink,
    HlmCardImports,
    HlmTableImports,
    HlmNativeSelectImports,
    HlmBadgeImports,
    HlmEmptyImports,
    HlmAlertImports,
  ],
  changeDetection: ChangeDetectionStrategy.OnPush,
  styles: `
    .add {
      background: var(--diff-add);
    }
    .del {
      background: var(--diff-del);
    }
    mark.add {
      background: var(--diff-add-strong);
      color: inherit;
    }
    mark.del {
      background: var(--diff-del-strong);
      color: inherit;
      text-decoration: line-through;
    }
  `,
  template: `
    <div class="mx-auto flex max-w-[1400px] flex-col gap-4 p-4 md:p-6">
      <div>
        <h1 class="text-xl font-semibold tracking-tight">Compare runs</h1>
        <p class="text-muted-foreground text-sm">
          What changed between two runs: cost, tools, files and the sequence of actions.
        </p>
      </div>
      <div class="grid gap-3 md:grid-cols-2">
        @for (side of sides; track side) {
          <label class="flex flex-col gap-1 text-sm">
            <span class="text-muted-foreground text-xs font-medium"
              >Run {{ side.toUpperCase() }}</span
            >
            <hlm-native-select
              [value]="side === 'a' ? (a() ?? '') : (b() ?? '')"
              (valueChange)="pick(side, $event)"
            >
              <option hlmNativeSelectOption value="">Choose a run…</option>
              @for (run of options(side); track run.runId) {
                <option
                  hlmNativeSelectOption
                  [value]="run.runId"
                  [selected]="run.runId === (side === 'a' ? a() : b())"
                >
                  {{ label(run) }} · {{ run.model ?? '' }} ·
                  {{ ago(run.startedAt) }}
                </option>
              }
            </hlm-native-select>
          </label>
        }
      </div>

      @if (left.error() || right.error()) {
        <div hlmAlert variant="destructive">
          <h4 hlmAlertTitle>Could not load a run</h4>
          <p hlmAlertDescription>{{ left.error()?.message ?? right.error()?.message }}</p>
        </div>
      }

      @if (pair(); as p) {
        <div class="grid gap-4 xl:grid-cols-2">
          <section hlmCard>
            <div hlmCardHeader>
              <h2 hlmCardTitle>Metrics</h2>
              <p hlmCardDescription class="flex flex-col">
                <a [routerLink]="['/runs', p.a.state.runId]" class="truncate hover:underline"
                  >A · {{ p.a.name }}</a
                >
                <a [routerLink]="['/runs', p.b.state.runId]" class="truncate hover:underline"
                  >B · {{ p.b.name }}</a
                >
              </p>
            </div>
            <div hlmCardContent>
              <div hlmTableContainer>
                <table hlmTable>
                  <thead hlmTHead>
                    <tr hlmTr>
                      <th hlmTh>Metric</th>
                      <th hlmTh class="text-right">A</th>
                      <th hlmTh class="text-right">B</th>
                      <th hlmTh class="text-right">Δ</th>
                    </tr>
                  </thead>
                  <tbody hlmTBody>
                    @for (m of metrics(); track m.label) {
                      <tr hlmTr>
                        <td hlmTd>{{ m.label }}</td>
                        <td hlmTd class="tabular text-right">{{ m.a }}</td>
                        <td hlmTd class="tabular text-right">{{ m.b }}</td>
                        <td hlmTd class="tabular text-right" [class]="m.tone">{{ m.delta }}</td>
                      </tr>
                    }
                  </tbody>
                </table>
              </div>
              <p class="text-muted-foreground mt-2 text-xs">
                Green is better for B (fewer tokens, lower cost, fewer failures).
              </p>
            </div>
          </section>

          <section hlmCard>
            <div hlmCardHeader>
              <h2 hlmCardTitle>Tool calls</h2>
              <p hlmCardDescription>A to the left, B to the right</p>
            </div>
            <div hlmCardContent class="flex flex-col gap-1">
              @for (t of toolRows(); track t.tool) {
                <div
                  class="grid grid-cols-[minmax(0,1fr)_11rem_minmax(0,1fr)] items-center gap-2 text-xs"
                >
                  <div class="flex items-center justify-end gap-1.5">
                    <span class="tabular text-muted-foreground">{{ t.a || '' }}</span>
                    <span
                      class="h-3.5 rounded-l-[3px]"
                      [style.width.%]="(t.a / toolMax()) * 100"
                      style="background: var(--chart-1)"
                    ></span>
                  </div>
                  <span class="truncate text-center font-mono" [title]="t.tool">{{ t.tool }}</span>
                  <div class="flex items-center gap-1.5">
                    <span
                      class="h-3.5 rounded-r-[3px]"
                      [style.width.%]="(t.b / toolMax()) * 100"
                      style="background: var(--chart-2)"
                    ></span>
                    <span class="tabular text-muted-foreground">{{ t.b || '' }}</span>
                  </div>
                </div>
              } @empty {
                <p class="text-muted-foreground text-sm">Neither run called a tool.</p>
              }
              <div class="text-muted-foreground mt-2 flex justify-center gap-4 text-xs">
                <span class="flex items-center gap-1.5"
                  ><span class="size-2.5 rounded-[3px]" style="background: var(--chart-1)"></span
                  >A</span
                >
                <span class="flex items-center gap-1.5"
                  ><span class="size-2.5 rounded-[3px]" style="background: var(--chart-2)"></span
                  >B</span
                >
              </div>
            </div>
          </section>
        </div>

        <section hlmCard>
          <div hlmCardHeader>
            <h2 hlmCardTitle>Files changed</h2>
          </div>
          <div hlmCardContent class="grid gap-4 md:grid-cols-3">
            @for (col of fileCols(); track col.label) {
              <div>
                <h3 class="mb-2 text-xs font-medium">
                  {{ col.label }} <span class="text-muted-foreground">{{ col.files.length }}</span>
                </h3>
                <ul class="flex flex-col gap-1">
                  @for (f of col.files; track f) {
                    <li
                      class="truncate rounded px-2 py-0.5 font-mono text-xs"
                      [class]="col.tone"
                      [title]="f"
                    >
                      {{ f }}
                    </li>
                  } @empty {
                    <li class="text-muted-foreground text-xs">none</li>
                  }
                </ul>
              </div>
            }
          </div>
        </section>

        <section hlmCard>
          <div hlmCardHeader>
            <h2 hlmCardTitle>First prompt</h2>
            <p hlmCardDescription>Word diff, A → B</p>
          </div>
          <div hlmCardContent>
            <p class="bg-muted/40 rounded-md p-3 text-sm break-words whitespace-pre-wrap">
              @for (part of promptDiff(); track $index) {
                @if (part.added) {
                  <mark class="add rounded-sm">{{ part.value }}</mark>
                } @else if (part.removed) {
                  <mark class="del rounded-sm">{{ part.value }}</mark>
                } @else {
                  <span>{{ part.value }}</span>
                }
              }
            </p>
          </div>
        </section>

        <section hlmCard>
          <div hlmCardHeader>
            <h2 hlmCardTitle>Action sequence</h2>
            <p hlmCardDescription>
              Prompts and tool calls aligned between the runs:
              <span class="text-destructive">− only in A</span>,
              <span class="text-success">+ only in B</span>
            </p>
          </div>
          <div hlmCardContent>
            <ol class="overflow-hidden rounded-md border font-mono text-xs">
              @for (row of actionRows(); track $index) {
                @if (row.kind === 'gap') {
                  <li class="bg-muted text-muted-foreground px-3 py-1">
                    ⋯ {{ row.count }} matching steps
                  </li>
                } @else {
                  <li
                    class="flex gap-3 px-3 py-0.5"
                    [class.add]="row.kind === 'add'"
                    [class.del]="row.kind === 'del'"
                  >
                    <span class="text-muted-foreground w-3 select-none">{{
                      row.kind === 'add' ? '+' : row.kind === 'del' ? '−' : ''
                    }}</span>
                    <span class="truncate">{{ row.text }}</span>
                  </li>
                }
              }
            </ol>
          </div>
        </section>
      } @else {
        <div hlmEmpty class="py-16">
          <div hlmEmptyHeader>
            <h3 hlmEmptyTitle>Pick two runs</h3>
            <p hlmEmptyDescription>Choose runs above, or select two on the Runs page.</p>
          </div>
        </div>
      }
    </div>
  `,
})
export class ComparePage {
  /** Query parameters. */
  readonly a = input<string | undefined>();
  readonly b = input<string | undefined>();
  private readonly api = inject(Api);
  private readonly router = inject(Router);
  protected readonly runs = inject(RunsStore);
  protected readonly sides = ['a', 'b'] as const;
  protected readonly shortId = shortId;
  protected readonly ago = fmtAgo;
  protected readonly label = (run: Parameters<typeof runLabel>[0]) => oneLine(runLabel(run), 70);

  protected readonly left = resource({
    params: () => this.a() || undefined,
    loader: ({ params, abortSignal }) => this.api.runState(params, abortSignal),
  });
  protected readonly right = resource({
    params: () => this.b() || undefined,
    loader: ({ params, abortSignal }) => this.api.runState(params, abortSignal),
  });
  protected readonly pair = computed(() =>
    this.left.hasValue() && this.right.hasValue() && this.a() && this.b()
      ? { a: facts(this.left.value()), b: facts(this.right.value()) }
      : undefined,
  );

  /** Runs in range, plus the picked run when it is older than the range. */
  protected options(side: 'a' | 'b') {
    const picked = side === 'a' ? this.a() : this.b();
    const runs = this.runs.runs();
    if (!picked || runs.some((r) => r.runId === picked)) return runs;
    const state = side === 'a' ? this.left.value() : this.right.value();
    return [
      {
        runId: picked,
        sessionName: state?.sessionName,
        prompt: undefined,
        model: state?.model ?? undefined,
        status: 'finished',
        startedAt: state?.startedAt ?? 0,
      },
      ...runs,
    ];
  }

  protected pick(side: 'a' | 'b', runId: string | null | undefined) {
    void this.router.navigate([], {
      queryParams: { [side]: runId || null },
      queryParamsHandling: 'merge',
    });
  }

  protected readonly metrics = computed(() => {
    const p = this.pair();
    if (!p) return [];
    const row = (
      label: string,
      a: number,
      b: number,
      fmt: (n: number) => string,
      lowerIsBetter = true,
    ) => {
      const d = b - a;
      const better = lowerIsBetter ? d < 0 : d > 0;
      const rel = a ? ` (${fmtPercent(d / a)})` : '';
      return {
        label,
        a: fmt(a),
        b: fmt(b),
        delta: d === 0 ? '=' : `${d > 0 ? '+' : '−'}${fmt(Math.abs(d))}${rel}`,
        tone: d === 0 ? 'text-muted-foreground' : better ? 'text-success' : 'text-destructive',
      };
    };
    const tok = (f: RunFacts) => f.input + f.output + f.cacheRead + f.cacheWrite;
    return [
      row('Duration', p.a.spanMs, p.b.spanMs, fmtMs),
      row('Replies', p.a.turns, p.b.turns, fmtCount),
      row('Tool calls', p.a.tools, p.b.tools, fmtCount),
      row('Failed tool calls', p.a.toolErrors, p.b.toolErrors, fmtCount),
      row('Tokens', tok(p.a), tok(p.b), fmtCount),
      row('Output tokens', p.a.output, p.b.output, fmtCount),
      row(
        'Cache hit',
        (cacheHit(p.a) ?? 0) * 100,
        (cacheHit(p.b) ?? 0) * 100,
        (n) => `${n.toFixed(0)}%`,
        false,
      ),
      row('Cost', p.a.cost, p.b.cost, fmtCost),
      row('Files changed', p.a.files.size, p.b.files.size, fmtCount, false),
    ];
  });

  protected readonly toolRows = computed(() => {
    const p = this.pair();
    if (!p) return [];
    const names = new Set([...p.a.toolCounts.keys(), ...p.b.toolCounts.keys()]);
    return [...names]
      .map((tool) => ({ tool, a: p.a.toolCounts.get(tool) ?? 0, b: p.b.toolCounts.get(tool) ?? 0 }))
      .sort((x, y) => y.a + y.b - (x.a + x.b))
      .slice(0, 16);
  });
  protected readonly toolMax = computed(() =>
    Math.max(1, ...this.toolRows().flatMap((t) => [t.a, t.b])),
  );

  protected readonly fileCols = computed(() => {
    const p = this.pair();
    if (!p) return [];
    const onlyA = [...p.a.files].filter((f) => !p.b.files.has(f)).sort();
    const both = [...p.a.files].filter((f) => p.b.files.has(f)).sort();
    const onlyB = [...p.b.files].filter((f) => !p.a.files.has(f)).sort();
    return [
      { label: 'Only in A', files: onlyA, tone: 'del' },
      { label: 'Both', files: both, tone: 'bg-muted/50' },
      { label: 'Only in B', files: onlyB, tone: 'add' },
    ];
  });

  protected readonly promptDiff = computed(() => {
    const p = this.pair();
    return p ? diffWordsWithSpace(p.a.prompt, p.b.prompt) : [];
  });

  protected readonly actionRows = computed(() => {
    const p = this.pair();
    if (!p) return [];
    type Row = { kind: 'same' | 'add' | 'del'; text: string } | { kind: 'gap'; count: number };
    const rows: Row[] = [];
    for (const part of diffArrays(p.a.actions, p.b.actions)) {
      const kind = part.added ? 'add' : part.removed ? 'del' : 'same';
      const values = part.value;
      if (kind === 'same' && values.length > KEEP * 2 + 1) {
        values.slice(0, KEEP).forEach((text) => rows.push({ kind, text }));
        rows.push({ kind: 'gap', count: values.length - KEEP * 2 });
        values.slice(-KEEP).forEach((text) => rows.push({ kind, text }));
      } else {
        values.forEach((text) => rows.push({ kind, text }));
      }
    }
    return rows;
  });
}
