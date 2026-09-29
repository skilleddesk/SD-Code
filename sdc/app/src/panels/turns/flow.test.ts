import { describe, expect, it } from 'vitest';

import { changeTotals, headline, matches, offset, ribbon, steps, transcript } from './flow';
import type { TimelineItem } from './types';

const at = (seconds: number): string => new Date(Date.UTC(2026, 8, 29, 10, 0, seconds)).toISOString();
const ms = (seconds: number): number => Date.parse(at(seconds));

const items: TimelineItem[] = [
  { kind: 'thinking', key: 'thinking-0', startedAt: at(1), endedAt: at(4), thinking: { text: '**Reading the router**\nIt lives in src.', ms: 3000, since: null, live: false } },
  { kind: 'tool', key: 'tool-a', tool: { kind: 'read', name: 'Read', target: 'src/router.ts', status: 'done', meta: 'done · 40 ln', startedAt: at(5), endedAt: at(6) } },
  { kind: 'tool', key: 'tool-b', tool: { kind: 'read', name: 'Read', target: 'src/app.ts', status: 'done', meta: 'done · 12 ln', startedAt: at(6), endedAt: at(7) } },
  { kind: 'text', key: 'text-3', text: 'Now I will fix it.', streaming: false, final: false, startedAt: at(8) },
  {
    kind: 'tool',
    key: 'tool-c',
    tool: { kind: 'edit', name: 'Edit', target: 'src/router.ts', status: 'done', meta: 'done', startedAt: at(10), endedAt: at(12), diff: [{ lineNumber: '3', text: 'a', change: 'add' }, { lineNumber: '4', text: 'b', change: 'rem' }] },
  },
  { kind: 'text', key: 'text-5', text: 'Fixed.', streaming: false, final: true, startedAt: at(13) },
];

describe('steps', () => {
  it('gathers reads, times every step and closes open stretches at the next one or the turn end', () => {
    const list = steps(items, ms(30), at(15), false);

    expect(list.map((step) => step.kind)).toEqual(['think', 'explore', 'say', 'edit', 'answer']);
    expect(list[0]?.title).toBe('Reading the router');
    expect(list[1]).toMatchObject({ start: ms(5), end: ms(7) });
    /* The words end where the edit starts; the answer at the turn's end. */
    expect(list[2]).toMatchObject({ start: ms(8), end: ms(10) });
    expect(list[4]).toMatchObject({ start: ms(13), end: ms(15) });
    expect(list[3]?.detail).toBe('done +1 −1');
  });

  it('runs the newest stretch up to now while the turn runs', () => {
    const running: TimelineItem[] = [{ kind: 'text', key: 't', text: 'Hi', streaming: true, final: false, startedAt: at(2) }];

    expect(steps(running, ms(9), undefined, true)[0]).toMatchObject({ status: 'running', start: ms(2), end: ms(9) });
  });
});

describe('ribbon', () => {
  it('colours each step by kind and fills the gaps with waiting', () => {
    const list = steps(items, ms(30), at(15), false);
    const { segments, totals, total } = ribbon(list, at(0), ms(30), at(15), false);

    expect(total).toBe(15_000);
    expect(totals).toEqual({ think: 3000, read: 2000, write: 4000, edit: 2000, run: 0, wait: 4000 });
    expect(segments[0]).toMatchObject({ phase: 'wait', left: 0 });
    expect(segments.reduce((sum, segment) => sum + segment.width, 0)).toBeCloseTo(1);
  });
});

describe('helpers', () => {
  it('finds a headline in a heading, a bold title or the first sentence', () => {
    expect(headline('## Planning\nmore')).toBe('Planning');
    expect(headline('I should read the file first. Then edit.')).toBe('I should read the file first.');
    expect(headline('x'.repeat(120))).toHaveLength(90);
  });

  it('formats offsets, filters and the transcript', () => {
    expect(offset(64_000)).toBe('+01:04');
    const list = steps(items, ms(30), at(15), false);

    expect(list.filter((step) => matches(step, 'read'))).toHaveLength(1);
    expect(list.filter((step) => matches(step, 'say'))).toHaveLength(2);
    expect(transcript(list, at(0)).split('\n')[1]).toContain('+00:05  ✓ [explore]');
    expect(changeTotals(items)).toEqual({ files: 1, added: 1, removed: 1 });
  });
});
