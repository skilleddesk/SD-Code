import { describe, expect, it } from 'vitest';

import { analyticsOf, usageOf } from './analytics';
import type { TurnView } from './types';

function turn(overrides: Partial<TurnView>): TurnView {
  return {
    id: 't',
    turnNumber: 1,
    sessionId: 's1',
    engine: 'claude_code',
    model: 'sonnet',
    tier: 'Balanced',
    prompt: '',
    text: '',
    thinking: '',
    thinkingMs: 0,
    thinkingSince: null,
    plan: [],
    startedAt: '2026-09-25T10:00:00',
    status: 'done',
    stuckForMs: 0,
    tools: [],
    summary: 'Done',
    meta: '',
    pass: null,
    ...overrides,
  };
}

describe('usageOf', () => {
  it('reads what the CLIs and the agent report', () => {
    expect(usageOf('$0.0290 · 3.3s · 12926 in · 5 out')).toEqual({ cost: 0.029, input: 12926, output: 5 });
    expect(usageOf('7 steps · 12.4k in · 3.1k out · ≈$0.11')).toEqual({ cost: 0.11, input: 12400, output: 3100 });
  });

  it('reports nothing for a footer that says nothing', () => {
    expect(usageOf('')).toEqual({ cost: null, input: 0, output: 0 });
    expect(usageOf('Interrupted')).toEqual({ cost: null, input: 0, output: 0 });
  });
});

describe('analyticsOf', () => {
  const now = new Date(2026, 8, 25, 18, 0, 0);

  it('adds up only what the turns reported, by day and by engine', () => {
    const result = analyticsOf(
      [
        turn({ id: 'a', meta: '$0.10 · 100 in · 10 out', startedAt: new Date(2026, 8, 25, 9).toISOString() }),
        turn({ id: 'b', meta: '$0.05', startedAt: new Date(2026, 8, 24, 9).toISOString() }),
        turn({ id: 'c', engine: 'native_api', meta: '', status: 'failed', startedAt: new Date(2026, 8, 25, 11).toISOString() }),
        turn({ id: 'old', meta: '$9.00', startedAt: new Date(2026, 7, 1).toISOString() }),
      ],
      now,
    );

    expect(result.turns).toBe(4);
    expect(result.failed).toBe(1);
    expect(result.priced).toBe(3);
    expect(result.cost).toBeCloseTo(9.15);
    expect(result.days).toHaveLength(7);
    expect(result.days[6]).toMatchObject({ turns: 2 });
    expect(result.days[6]?.cost).toBeCloseTo(0.1);
    expect(result.days[5]?.cost).toBeCloseTo(0.05);
    expect(result.engines[0]).toEqual({ engine: 'claude_code', turns: 3, percent: 75 });
  });

  it('is all zeros for an empty log', () => {
    const result = analyticsOf([], now);

    expect(result.turns).toBe(0);
    expect(result.engines).toEqual([]);
    expect(result.days.every((day) => day.turns === 0)).toBe(true);
  });
});
