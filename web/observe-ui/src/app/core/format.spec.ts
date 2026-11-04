import { delta, fmtCount, fmtMs, oneLine, runActivity, runLabel } from './format';
import { actionSignature, cacheHit, turnUsage } from './items';
import type { ObserveItem } from './types';

describe('format', () => {
  it('formats durations and counts compactly', () => {
    expect(fmtMs(850)).toBe('850ms');
    expect(fmtMs(2300)).toBe('2.3s');
    expect(fmtMs(65_000)).toBe('1m 05s');
    expect(fmtMs(7_200_000)).toBe('2h 00m');
    expect(fmtMs(undefined)).toBe('–');
    expect(fmtCount(9_999)).toMatch(/9.?999/);
    expect(delta(15, 10)).toBeCloseTo(0.5);
    expect(delta(5, 0)).toBeNull();
    expect(delta(0, 0)).toBe(0);
  });

  it('labels runs by name, then prompt, then id', () => {
    const base = { runId: 'sess-123-abcdef0123', status: 'finished', startedAt: 0 };
    expect(runLabel({ ...base, sessionName: 'Named', prompt: 'p' })).toBe('Named');
    expect(runLabel({ ...base, prompt: 'fix it' })).toBe('fix it');
    expect(runLabel(base)).toBe('run abcdef01');
    expect(oneLine('  a\n\n b  ', 10)).toBe('a b');
    expect(oneLine('abcdefghijk', 4)).toBe('abcd…');
  });

  it('reports activity recency without claiming process liveness or a crash', () => {
    const now = 10_000_000;
    const run = { runId: 'r', status: 'running', startedAt: now - 3_600_000 };
    expect(runActivity({ ...run, lastActivity: now - 60_000 }, now)).toBe('recent');
    expect(runActivity({ ...run, lastActivity: now - 900_000 }, now)).toBe('quiet');
    expect(runActivity({ ...run, lastActivity: now - 600_000 }, now)).toBe('quiet');
    expect(runActivity({ ...run, status: 'finished' }, now)).toBe('finished');
    // A backwards wall clock is not evidence of recent activity or ownership.
    expect(runActivity({ ...run, lastActivity: now + 1 }, now)).toBe('unknown');
    expect(runActivity({ ...run, lastActivity: Number.NaN }, now)).toBe('unknown');
    expect(runActivity({ ...run, status: 'unknown' }, now)).toBe('unknown');
  });
});

describe('items', () => {
  const item = (extra: Partial<ObserveItem>): ObserveItem => ({
    threadKey: 't',
    threadId: 't',
    mainThreadId: 't',
    threadKind: 'main',
    threadName: 'main',
    threadColor: 'blue',
    runId: 'r',
    id: 1,
    ts: 1,
    kind: 'message',
    ...extra,
  });

  it('reads reply usage from the raw message', () => {
    const usage = turnUsage(
      item({
        role: 'assistant',
        raw: JSON.stringify({
          provider: 'omni',
          model: 'cx/m',
          stopReason: 'toolUse',
          usage: { input: 10, output: 2, cacheRead: 30, cacheWrite: 0, cost: { total: 0.5 } },
        }),
      }),
    );
    expect(usage).toMatchObject({
      input: 10,
      cacheRead: 30,
      cost: 0.5,
      model: 'omni/cx/m',
      stopReason: 'toolUse',
    });
    expect(cacheHit(usage!)).toBeCloseTo(0.75);
    expect(turnUsage(item({ role: 'user' }))).toBeUndefined();
    expect(turnUsage(item({ role: 'assistant', raw: '{truncated' }))?.input).toBe(0);
  });

  it('summarizes actions for run comparison', () => {
    const bash = item({
      kind: 'tool',
      toolName: 'bash',
      args: JSON.stringify({ command: 'cargo test -p x' }),
    });
    const read = item({ kind: 'tool', toolName: 'read', args: JSON.stringify({ path: 'a.rs' }) });
    expect(actionSignature(bash)).toBe('bash cargo test');
    expect(actionSignature(read)).toBe('read a.rs');
    expect(actionSignature(item({ role: 'user' }))).toBe('user message');
  });
});
