import { diffWordsWithSpace, structuredPatch } from 'diff';
import { argPath, itemArgs, parseJson } from './items';
import type { ObserveItem } from './types';

export interface DiffSegment {
  text: string;
  changed: boolean;
}

export interface DiffLine {
  kind: 'add' | 'del' | 'ctx' | 'hunk' | 'meta';
  oldNo?: number;
  newNo?: number;
  text: string;
  /** Word-level change marks for a paired removed/added line. */
  segments?: DiffSegment[];
}

export interface FileDiff {
  lines: DiffLine[];
  added: number;
  removed: number;
}

const HUNK = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/;
/** Longest line pair that gets word-level highlighting. */
const WORD_DIFF_MAX = 600;

function finish(lines: DiffLine[]): FileDiff {
  highlightWords(lines);
  return {
    lines,
    added: lines.filter((l) => l.kind === 'add').length,
    removed: lines.filter((l) => l.kind === 'del').length,
  };
}

/** Parse a unified patch (`--- a`, `+++ b`, `@@ -1,2 +1,3 @@`, ...). */
export function parseUnifiedPatch(patch: string): FileDiff {
  const lines: DiffLine[] = [];
  let oldNo = 0;
  let newNo = 0;
  for (const line of patch.replace(/\n$/, '').split('\n')) {
    if (
      line.startsWith('--- ') ||
      line.startsWith('+++ ') ||
      line.startsWith('diff ') ||
      line.startsWith('index ')
    ) {
      continue;
    }
    const hunk = HUNK.exec(line);
    if (hunk) {
      oldNo = Number(hunk[1]);
      newNo = Number(hunk[2]);
      lines.push({ kind: 'hunk', text: line });
    } else if (line.startsWith('+')) {
      lines.push({ kind: 'add', newNo: newNo++, text: line.slice(1) });
    } else if (line.startsWith('-')) {
      lines.push({ kind: 'del', oldNo: oldNo++, text: line.slice(1) });
    } else if (line.startsWith('\\')) {
      lines.push({ kind: 'meta', text: line });
    } else {
      lines.push({ kind: 'ctx', oldNo: oldNo++, newNo: newNo++, text: line.slice(1) });
    }
  }
  return finish(lines);
}

/** Line diff of two texts with `context` unchanged lines around changes. */
export function diffTexts(before: string, after: string, context = 3): FileDiff {
  const patch = structuredPatch('a', 'b', before, after, '', '', { context });
  const lines: DiffLine[] = [];
  for (const hunk of patch.hunks) {
    lines.push({
      kind: 'hunk',
      text: `@@ -${hunk.oldStart},${hunk.oldLines} +${hunk.newStart},${hunk.newLines} @@`,
    });
    let oldNo = hunk.oldStart;
    let newNo = hunk.newStart;
    for (const line of hunk.lines) {
      const text = line.slice(1);
      if (line.startsWith('+')) lines.push({ kind: 'add', newNo: newNo++, text });
      else if (line.startsWith('-')) lines.push({ kind: 'del', oldNo: oldNo++, text });
      else if (line.startsWith('\\')) lines.push({ kind: 'meta', text: line });
      else lines.push({ kind: 'ctx', oldNo: oldNo++, newNo: newNo++, text });
    }
  }
  return finish(lines);
}

/** A new file: every line added. */
export function addedOnly(text: string): FileDiff {
  const lines: DiffLine[] = text
    .replace(/\n$/, '')
    .split('\n')
    .map((line, i) => ({ kind: 'add', newNo: i + 1, text: line }));
  return finish(lines);
}

/** Pair each block of removed lines with the added lines after it. */
function highlightWords(lines: DiffLine[]) {
  let i = 0;
  while (i < lines.length) {
    if (lines[i].kind !== 'del') {
      i++;
      continue;
    }
    const delStart = i;
    while (i < lines.length && lines[i].kind === 'del') i++;
    const addStart = i;
    while (i < lines.length && lines[i].kind === 'add') i++;
    const pairs = Math.min(addStart - delStart, i - addStart);
    for (let k = 0; k < pairs; k++) {
      const del = lines[delStart + k];
      const add = lines[addStart + k];
      if (del.text.length > WORD_DIFF_MAX || add.text.length > WORD_DIFF_MAX) continue;
      const parts = diffWordsWithSpace(del.text, add.text);
      // Skip pairs that share almost nothing: highlighting every word is noise.
      const common = parts
        .filter((p) => !p.added && !p.removed)
        .reduce((n, p) => n + p.value.length, 0);
      if (common < Math.min(del.text.length, add.text.length) * 0.3) continue;
      del.segments = parts
        .filter((p) => !p.added)
        .map((p) => ({ text: p.value, changed: !!p.removed }));
      add.segments = parts
        .filter((p) => !p.removed)
        .map((p) => ({ text: p.value, changed: !!p.added }));
    }
  }
}

export interface SplitRow {
  hunk?: DiffLine;
  left?: DiffLine;
  right?: DiffLine;
}

/** Side-by-side rows: context on both sides, removed lines beside added. */
export function splitRows(lines: DiffLine[]): SplitRow[] {
  const rows: SplitRow[] = [];
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    if (line.kind === 'hunk' || line.kind === 'meta') {
      rows.push({ hunk: line });
      i++;
    } else if (line.kind === 'ctx') {
      rows.push({ left: line, right: line });
      i++;
    } else {
      const dels: DiffLine[] = [];
      const adds: DiffLine[] = [];
      while (i < lines.length && lines[i].kind === 'del') dels.push(lines[i++]);
      while (i < lines.length && lines[i].kind === 'add') adds.push(lines[i++]);
      for (let k = 0; k < Math.max(dels.length, adds.length); k++)
        rows.push({ left: dels[k], right: adds[k] });
    }
  }
  return rows;
}

export interface FileChange {
  key: string;
  item: ObserveItem;
  path: string;
  tool: string;
  ts: number;
  diff: FileDiff;
  failed: boolean;
  /** How the diff was obtained. */
  basis: 'patch' | 'edits' | 'previous write' | 'new content';
}

export interface FileGroup {
  path: string;
  changes: FileChange[];
  added: number;
  removed: number;
  lastTs: number;
}

interface EditArg {
  oldText?: unknown;
  newText?: unknown;
  old_string?: unknown;
  new_string?: unknown;
}

function editsOf(item: ObserveItem): Array<{ oldText: string; newText: string }> {
  const args = itemArgs(item) ?? {};
  let edits: unknown = args['edits'];
  if (typeof edits === 'string') edits = parseJson(edits);
  const list: EditArg[] = Array.isArray(edits) ? (edits as EditArg[]) : [args as EditArg];
  return list
    .map((e) => ({ oldText: e.oldText ?? e.old_string, newText: e.newText ?? e.new_string }))
    .filter(
      (e): e is { oldText: string; newText: string } =>
        typeof e.oldText === 'string' && typeof e.newText === 'string',
    );
}

/** The diff of one `edit` call: the tool's own patch when recorded. */
function editDiff(item: ObserveItem): { diff: FileDiff; basis: FileChange['basis'] } | undefined {
  const raw = parseJson<{ details?: { patch?: unknown } }>(item.raw);
  const patch = raw?.details?.patch;
  if (typeof patch === 'string' && patch.includes('@@')) {
    return { diff: parseUnifiedPatch(patch), basis: 'patch' };
  }
  const edits = editsOf(item);
  if (!edits.length) return undefined;
  const lines: DiffLine[] = [];
  edits.forEach((edit, i) => {
    const part = diffTexts(edit.oldText, edit.newText, 2);
    lines.push({ kind: 'hunk', text: `@@ replacement ${i + 1} of ${edits.length} @@` });
    lines.push(...part.lines.filter((l) => l.kind !== 'hunk'));
  });
  return { diff: finish(lines), basis: 'edits' };
}

/** Every `edit`/`write` in `items`, grouped by file, newest file first. */
export function fileChanges(items: ObserveItem[]): FileGroup[] {
  const lastWrite = new Map<string, string>();
  const groups = new Map<string, FileGroup>();
  const sorted = [...items].sort((a, b) => a.ts - b.ts || a.id - b.id);
  for (const item of sorted) {
    if (item.kind !== 'tool') continue;
    const tool = (item.toolName ?? '').toLowerCase();
    if (tool !== 'edit' && tool !== 'write') continue;
    const path = argPath(item);
    if (!path) continue;
    let change: Pick<FileChange, 'diff' | 'basis'> | undefined;
    if (tool === 'edit') {
      change = editDiff(item);
    } else {
      const content = itemArgs(item)?.['content'];
      if (typeof content === 'string') {
        const previous = lastWrite.get(path);
        change =
          previous === undefined
            ? { diff: addedOnly(content), basis: 'new content' }
            : { diff: diffTexts(previous, content), basis: 'previous write' };
        if (item.state !== 'error') lastWrite.set(path, content);
      }
    }
    if (!change) continue;
    const group = groups.get(path) ?? { path, changes: [], added: 0, removed: 0, lastTs: 0 };
    group.changes.push({
      key: `${item.runId}:${item.id}`,
      item,
      path,
      tool,
      ts: item.ts,
      failed: item.state === 'error',
      ...change,
    });
    if (item.state !== 'error') {
      group.added += change.diff.added;
      group.removed += change.diff.removed;
    }
    group.lastTs = Math.max(group.lastTs, item.ts);
    groups.set(path, group);
  }
  return [...groups.values()].sort((a, b) => b.lastTs - a.lastTs);
}
