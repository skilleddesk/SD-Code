import { describe, expect, it } from 'vitest';

import type { TurnView } from '../../store/types';
import { collapsedSummary, toTurns } from './live';

/**
 * The stream's projection - and the regression behind *"sudu done lakha aslo, kono response pelam nah"*.
 *
 * `toTurns` builds the drawn turn out of the projected one, and it used to leave the answer behind:
 * `Turn` had no field for it, so the text the reducer accumulates in `TurnView.text` never reached the
 * screen. A finished turn drew the question, the meta line and `Done · $0.0295 · 1.6s · 2 in · 4 out`,
 * and nothing in between - on every engine.
 */
function turn(overrides: Partial<TurnView> = {}): TurnView {
  return {
    id: 't1',
    turnNumber: 1,
    sessionId: 's1',
    engine: 'claude_code',
    model: 'sonnet',
    tier: 'Balanced',
    prompt: 'Reply with exactly: OK',
    text: '',
    thinking: '',
    thinkingMs: 0,
    thinkingSince: null,
    plan: [],
    startedAt: '2026-09-25T10:00:00Z',
    status: 'running',
    stuckForMs: 0,
    tools: [],
    summary: '',
    meta: '',
    pass: null,
    ...overrides,
  };
}

describe('toTurns', () => {
  it('carries the answer the reducer accumulated', () => {
    const drawn = toTurns([turn({ text: 'OK', status: 'done', summary: 'Done', meta: '$0.03 · 1.6s' })], 's1');

    expect(drawn).toHaveLength(1);
    expect(drawn[0]?.answer).toEqual({ text: 'OK', streaming: false });
  });

  it('marks an answer that is still arriving', () => {
    const drawn = toTurns([turn({ text: 'O' })], 's1');

    expect(drawn[0]?.answer).toEqual({ text: 'O', streaming: true });
  });

  it('draws no answer block before the first delta', () => {
    expect(toTurns([turn()], 's1')[0]?.answer).toBeUndefined();
  });

  it('keeps only the session it was asked for', () => {
    const drawn = toTurns([turn({ id: 'a', sessionId: 's1' }), turn({ id: 'b', sessionId: 's2' })], 's2');

    expect(drawn.map((entry) => entry.id)).toEqual(['b']);
  });

  it('copies the question and the engine that answered it', () => {
    const drawn = toTurns([turn()], 's1')[0];

    expect(drawn?.user.body).toBe('Reply with exactly: OK');
    expect(drawn?.meta).toMatchObject({ tier: 'Balanced', engine: 'claude_code', model: 'sonnet' });
  });

  it('collapses nothing until the window is exceeded', () => {
    expect(collapsedSummary([turn()], 's1')).toBeNull();
  });
});

describe('toTurns (v4: live thinking and the checkpoint rail)', () => {
  it('marks thinking live while the turn runs and the engine is still thinking', () => {
    const [live] = toTurns([turn({ thinking: 'hm', thinkingSince: '2026-09-25T10:00:00Z' })], 's1');
    const [done] = toTurns([turn({ thinking: 'hm', thinkingMs: 6200, status: 'done' })], 's1');

    expect(live?.thinking).toEqual({ text: 'hm', ms: 0, since: '2026-09-25T10:00:00Z', live: true });
    expect(done?.thinking).toEqual({ text: 'hm', ms: 6200, since: null, live: false });
  });

  it('is not live once the engine has moved on, even though the turn is still running', () => {
    const [moved] = toTurns([turn({ thinking: 'hm', thinkingMs: 1200, thinkingSince: null, text: 'answer' })], 's1');

    expect(moved?.thinking?.live).toBe(false);
  });

  it('gives each turn only the checkpoints it wrote, oldest first', () => {
    const checkpoint = (id: string, turnId: string | null, ordinal: number) => ({
      id,
      sessionId: 's1',
      turnId,
      turn: ordinal,
      when: 'now',
      title: `Before ${id}`,
      thumbnail: null,
      filesHash: 'abc',
    });
    const [drawn] = toTurns([turn()], 's1', [
      checkpoint('late', 't1', 9),
      checkpoint('other', 't2', 5),
      checkpoint('early', 't1', 3),
      checkpoint('save', null, 4),
    ]);

    expect(drawn?.checkpoints.map((entry) => entry.id)).toEqual(['early', 'late']);
    expect(drawn?.checkpoints[0]).toEqual({ id: 'early', title: 'Before early', turn: 3 });
  });
});
