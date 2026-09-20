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
