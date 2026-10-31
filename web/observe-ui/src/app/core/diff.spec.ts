import { addedOnly, diffTexts, fileChanges, parseUnifiedPatch, splitRows } from './diff';
import type { ObserveItem } from './types';

function tool(
  id: number,
  name: string,
  args: unknown,
  extra: Partial<ObserveItem> = {},
): ObserveItem {
  return {
    threadKey: 't',
    threadId: 't',
    mainThreadId: 't',
    threadKind: 'main',
    threadName: 'main',
    threadColor: 'blue',
    runId: 'r1',
    id,
    ts: 1000 + id,
    kind: 'tool',
    toolName: name,
    state: 'done',
    args: JSON.stringify(args),
    ...extra,
  };
}

describe('parseUnifiedPatch', () => {
  it('numbers lines from hunk headers and skips file headers', () => {
    const diff = parseUnifiedPatch(
      [
        '--- a.rs',
        '+++ a.rs',
        '@@ -10,3 +10,3 @@',
        ' keep',
        '-let x = 1;',
        '+let x = 2;',
        ' tail',
      ].join('\n'),
    );
    expect(diff.added).toBe(1);
    expect(diff.removed).toBe(1);
    expect(diff.lines.map((l) => [l.kind, l.oldNo, l.newNo])).toEqual([
      ['hunk', undefined, undefined],
      ['ctx', 10, 10],
      ['del', 11, undefined],
      ['add', undefined, 11],
      ['ctx', 12, 12],
    ]);
  });

  it('marks the changed words of a paired line', () => {
    const diff = parseUnifiedPatch('@@ -1 +1 @@\n-let x = 1;\n+let x = 2;');
    const del = diff.lines[1];
    const add = diff.lines[2];
    expect(del.segments?.filter((s) => s.changed).map((s) => s.text)).toEqual(['1']);
    expect(add.segments?.filter((s) => s.changed).map((s) => s.text)).toEqual(['2']);
    expect(add.segments?.map((s) => s.text).join('')).toBe('let x = 2;');
  });
});

describe('diffTexts and splitRows', () => {
  it('pairs removed and added lines side by side', () => {
    const diff = diffTexts('a\nb\nc\n', 'a\nB\nc\nd\n');
    expect(diff.added).toBe(2);
    expect(diff.removed).toBe(1);
    const rows = splitRows(diff.lines).filter((r) => !r.hunk);
    expect(rows.map((r) => [r.left?.text ?? null, r.right?.text ?? null])).toEqual([
      ['a', 'a'],
      ['b', 'B'],
      ['c', 'c'],
      [null, 'd'],
    ]);
  });

  it('treats a first write as all additions', () => {
    const diff = addedOnly('one\ntwo\n');
    expect(diff.lines.map((l) => [l.kind, l.newNo, l.text])).toEqual([
      ['add', 1, 'one'],
      ['add', 2, 'two'],
    ]);
  });
});

describe('fileChanges', () => {
  it('prefers the recorded patch and diffs rewrites against the earlier write', () => {
    const patched = tool(
      1,
      'edit',
      { path: 'src/a.rs', edits: [{ oldText: 'x', newText: 'y' }] },
      { raw: JSON.stringify({ details: { patch: '@@ -5 +5 @@\n-x\n+y' } }) },
    );
    const synthesized = tool(2, 'edit', {
      path: 'src/a.rs',
      edits: [{ oldText: 'p\nq', newText: 'p\nr' }],
    });
    const first = tool(3, 'write', { path: 'notes.md', content: 'one\n' });
    const failed = tool(4, 'write', { path: 'notes.md', content: 'broken\n' }, { state: 'error' });
    const second = tool(5, 'write', { path: 'notes.md', content: 'one\ntwo\n' });
    const groups = fileChanges([second, failed, first, synthesized, patched]);

    expect(groups.map((g) => g.path)).toEqual(['notes.md', 'src/a.rs']);
    const [notes, source] = groups;
    expect(notes.changes.map((c) => c.basis)).toEqual([
      'new content',
      'previous write',
      'previous write',
    ]);
    // The failed write is shown but neither counted nor used as a baseline.
    expect(notes.changes[1].failed).toBe(true);
    expect(notes.changes[2].diff.added).toBe(1);
    expect(notes.added).toBe(2);
    expect(source.changes.map((c) => c.basis)).toEqual(['patch', 'edits']);
    expect(source.changes[0].diff.lines.find((l) => l.kind === 'add')?.newNo).toBe(5);
    expect(source.added).toBe(2);
    expect(source.removed).toBe(2);
  });

  it('ignores calls without a path or content', () => {
    expect(
      fileChanges([tool(1, 'write', { content: 'x' }), tool(2, 'edit', { path: 'a' })]),
    ).toEqual([]);
  });
});
