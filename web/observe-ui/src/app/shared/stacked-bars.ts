import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';
import { HLM_CHART_THEME, HlmChart, hlmChartTooltip } from '@spartan-ng/helm/chart';
import { Chart } from '@tanstack/angular-charts';
import { barY, defineChart } from '@tanstack/charts';
import { scaleBand } from '@tanstack/charts/scales/band';
import { scaleLinear } from '@tanstack/charts/scales/linear';

export interface BarSeries {
  key: string;
  label: string;
  /** A `--chart-N` slot; fixed per series so a filter never repaints it. */
  color: string;
}

export interface BarRow {
  /** Unique category key, in display order. */
  x: string;
  series: string;
  value: number;
}

/** A stacked bar chart (spartan chart / TanStack Charts) with a legend. */
@Component({
  selector: 'app-stacked-bars',
  imports: [Chart, HlmChart],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @if (series().length > 1) {
      <ul
        class="text-muted-foreground mb-2 flex flex-wrap gap-x-4 gap-y-1 text-xs"
        aria-label="Legend"
      >
        @for (s of series(); track s.key) {
          <li class="flex items-center gap-1.5">
            <span class="size-2.5 rounded-[3px]" [style.background]="s.color"></span>
            {{ s.label }}
          </li>
        }
      </ul>
    }
    <tanstack-chart hlmChart [options]="options()" />
  `,
})
export class StackedBars {
  readonly rows = input.required<BarRow[]>();
  readonly series = input.required<BarSeries[]>();
  readonly label = input.required<string>();
  readonly height = input(220);
  readonly format = input<(value: number) => string>((v) => String(v));
  /** Label for an `x` key; keys must be unique, labels need not be. */
  readonly xLabel = input<(x: string) => string>((x) => x);

  protected readonly options = computed(() => {
    const series = this.series();
    const labels = new Map(series.map((s) => [s.key, s.label]));
    const format = this.format();
    const xLabel = this.xLabel();
    const xs = [...new Set(this.rows().map((r) => r.x))];
    // Thin long axes to about eight labels.
    const step = Math.max(1, Math.ceil(xs.length / 8));
    const shown = new Set(xs.filter((_, i) => i % step === 0));
    return {
      ariaLabel: this.label(),
      height: this.height(),
      definition: defineChart({
        theme: HLM_CHART_THEME,
        color: {
          domain: series.map((s) => s.key),
          range: series.map((s) => s.color),
        },
        marks: [
          barY(this.rows(), {
            x: 'x',
            y: 'value',
            z: 'series',
            color: 'series',
            inset: 1,
            radius: 2,
          }),
        ],
        scales: {
          x: {
            scale: scaleBand,
            axis: { ticks: { values: xs.filter((x) => shown.has(x)), format: xLabel } },
          },
          y: {
            scale: scaleLinear,
            nice: true,
            grid: true,
            axis: { line: false, ticks: { format } },
          },
        },
        tooltip: hlmChartTooltip<BarRow, string, number>({
          format: (point) =>
            `${xLabel(point.datum.x)} · ${labels.get(point.datum.series)}: ${format(point.datum.value)}`,
        }),
      }),
    };
  });
}
