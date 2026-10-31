import { NgTemplateOutlet } from '@angular/common';
import { ChangeDetectionStrategy, Component, computed, input, signal } from '@angular/core';
import { HlmButtonImports } from '@spartan-ng/helm/button';
import { type DiffLine, type FileDiff, splitRows } from '../core/diff';

export type DiffMode = 'unified' | 'split';

/** Renders a line diff, unified or side by side, with word highlights. */
@Component({
  selector: 'app-diff-view',
  imports: [HlmButtonImports, NgTemplateOutlet],
  changeDetection: ChangeDetectionStrategy.OnPush,
  styles: `
    .code {
      font-family: var(--font-mono);
      font-size: 12px;
      line-height: 1.55;
      tab-size: 4;
    }
    .num {
      user-select: none;
      text-align: right;
      color: var(--muted-foreground);
      padding: 0 0.5rem;
      width: 1%;
      white-space: nowrap;
      vertical-align: top;
    }
    .sign {
      user-select: none;
      width: 1rem;
      padding-left: 0.25rem;
      color: var(--muted-foreground);
      vertical-align: top;
    }
    .text {
      white-space: pre-wrap;
      word-break: break-word;
      padding-right: 0.75rem;
    }
    .add {
      background: var(--diff-add);
    }
    .del {
      background: var(--diff-del);
    }
    .add mark {
      background: var(--diff-add-strong);
      color: inherit;
      border-radius: 2px;
    }
    .del mark {
      background: var(--diff-del-strong);
      color: inherit;
      border-radius: 2px;
    }
    .hunk td {
      background: var(--muted);
      color: var(--muted-foreground);
      padding: 0.15rem 0.75rem;
    }
    .empty {
      background: repeating-linear-gradient(
        135deg,
        transparent 0 6px,
        color-mix(in oklab, var(--muted-foreground) 8%, transparent) 6px 7px
      );
    }
  `,
  template: `
    <div class="bg-card overflow-x-auto rounded-md border">
      @if (mode() === 'unified') {
        <table class="code w-full border-collapse">
          <tbody>
            @for (line of shown(); track $index) {
              @if (line.kind === 'hunk' || line.kind === 'meta') {
                <tr class="hunk">
                  <td colspan="4">{{ line.text }}</td>
                </tr>
              } @else {
                <tr [class.add]="line.kind === 'add'" [class.del]="line.kind === 'del'">
                  <td class="num">{{ line.oldNo ?? '' }}</td>
                  <td class="num">{{ line.newNo ?? '' }}</td>
                  <td class="sign">{{ sign(line) }}</td>
                  <td class="text">
                    <ng-container *ngTemplateOutlet="text; context: { $implicit: line }" />
                  </td>
                </tr>
              }
            }
          </tbody>
        </table>
      } @else {
        <table class="code w-full table-fixed border-collapse">
          <colgroup>
            <col style="width: 3.5rem" />
            <col />
            <col style="width: 3.5rem" />
            <col />
          </colgroup>
          <tbody>
            @for (row of split(); track $index) {
              @if (row.hunk) {
                <tr class="hunk">
                  <td colspan="4">{{ row.hunk.text }}</td>
                </tr>
              } @else {
                <tr>
                  @if (row.left) {
                    <td class="num" [class.del]="row.left.kind === 'del'">{{ row.left.oldNo }}</td>
                    <td class="text" [class.del]="row.left.kind === 'del'">
                      <ng-container *ngTemplateOutlet="text; context: { $implicit: row.left }" />
                    </td>
                  } @else {
                    <td class="num empty"></td>
                    <td class="empty"></td>
                  }
                  @if (row.right) {
                    <td class="num border-l" [class.add]="row.right.kind === 'add'">
                      {{ row.right.newNo }}
                    </td>
                    <td class="text" [class.add]="row.right.kind === 'add'">
                      <ng-container *ngTemplateOutlet="text; context: { $implicit: row.right }" />
                    </td>
                  } @else {
                    <td class="num empty border-l"></td>
                    <td class="empty"></td>
                  }
                </tr>
              }
            }
          </tbody>
        </table>
      }
      <!-- Every interpolation sits inside an element: Angular collapses the
           whitespace around a bare one into a space, which would alter code. -->
      <ng-template #text let-line>
        @if (line.segments) {
          @for (seg of line.segments; track $index) {
            @if (seg.changed) {
              <mark>{{ seg.text }}</mark>
            } @else {
              <span>{{ seg.text }}</span>
            }
          }
        } @else {
          <span>{{ line.text }}</span>
        }
      </ng-template>
    </div>
    @if (hidden() > 0) {
      <button hlmBtn variant="ghost" size="sm" class="mt-1" (click)="expanded.set(true)">
        Show {{ hidden() }} more lines
      </button>
    }
  `,
})
export class DiffView {
  readonly diff = input.required<FileDiff>();
  readonly mode = input<DiffMode>('unified');
  readonly limit = input(400);
  protected readonly expanded = signal(false);

  protected readonly shown = computed(() => {
    const lines = this.diff().lines;
    return this.expanded() ? lines : lines.slice(0, this.limit());
  });
  protected readonly hidden = computed(() => this.diff().lines.length - this.shown().length);
  protected readonly split = computed(() => splitRows(this.shown()));

  protected sign(line: DiffLine): string {
    return line.kind === 'add' ? '+' : line.kind === 'del' ? '−' : '';
  }
}
