import { describe, expect, it } from 'vitest';

import type { RelayStepEvent, RelaySummaryEvent } from '../../../../protocol/types';
import { chipOf, periodStarts, receiptOf, relayLine } from './relay';

const step = (over: Partial<RelayStepEvent> = {}): RelayStepEvent => ({
  type: 'RelayStep',
  sessionId: 's1',
  turnId: 't1',
  stepId: 't1.r1',
  fromModel: 'ollama:qwen3.5:9b',
  toModel: 'deepseek:deepseek-chat',
  reason: 'verify_fail',
  attempt: 1,
  tokensOut: 500,
  tokensIn: 4200,
  costUsd: 0.0016,
  costLabel: 'priced',
  durationMs: 900,
  overheadMs: 0,
  outcome: 'done',
  ...over,
});

const summary = (over: Partial<RelaySummaryEvent> = {}): RelaySummaryEvent => ({
  type: 'RelaySummary',
  sessionId: 's1',
  turnId: 't1',
  stepsTotal: 12,
  stepsLocal: 10,
  stepsCloud: 2,
  attempts: 2,
  failed: 0,
  unverified: 0,
  byModel: [{ model: 'deepseek:deepseek-chat', steps: 2, tokensIn: 8400, tokensOut: 1100, costUsd: 0.0031 }],
  tokensIn: 8400,
  tokensOut: 1100,
  costUsd: 0.0031,
  costLabel: 'priced',
  reasons: [
    { reason: 'verify_fail', count: 1 },
    { reason: 'loop', count: 1 },
  ],
  ...over,
});

describe('the relay chip', () => {
  it('shows nothing until a step has gone to the chain', () => {
    expect(chipOf([])).toBeNull();
  });

  it('counts the steps the chain finished, and names the newest attempt', () => {
    const chip = chipOf([step({ outcome: 'failed' }), step({ outcome: 'done', stepId: 't1.r2' })]);

    expect(chip?.cloud).toBe(1);
    expect(chip?.title).toContain('deepseek:deepseek-chat');
    expect(chip?.title).toContain('a check kept failing');
  });
});

describe('the relay receipt', () => {
  it('says the local model did everything when nothing went to the chain', () => {
    expect(receiptOf(summary({ attempts: 0, stepsCloud: 0, stepsLocal: 12, byModel: [], reasons: [] }))).toEqual(['This work: 12 steps - every step on the local model']);
  });

  it('is the plan example: the split, what was sent, and why - never a price', () => {
    const lines = receiptOf(summary());

    expect(lines[0]).toBe('This work: 12 steps - 10 on the local model (83%), 2 on the chain (deepseek:deepseek-chat)');
    expect(lines[1]).toBe('Sent to the chain: 8.4k tokens out, 1.1k back');
    expect(lines[2]).toBe('Why: 1 a check kept failing, 1 the steps went round in a circle · details: Trace');
  });

  it('says what could not be checked', () => {
    const lines = receiptOf(summary({ costLabel: 'estimate', unverified: 1, failed: 1 }));

    expect(lines.join('\n')).not.toContain('$');
    expect(lines.join('\n')).not.toContain('(measured)');
    expect(lines).toContain('1 relayed step could not be checked (this folder has no checks).');
    expect(lines).toContain('1 step nobody could do.');
  });

  it('has no receipt for a turn without steps', () => {
    expect(receiptOf(summary({ stepsTotal: 0, attempts: 0 }))).toEqual([]);
  });
});

describe('the Cost Center line', () => {
  it('starts today and this month at midnight on this computer\'s own clock', () => {
    const { today, month } = periodStarts(new Date(2026, 9, 23, 15, 30));

    expect(new Date(today).getTime()).toBe(new Date(2026, 9, 23).getTime());
    expect(new Date(month).getTime()).toBe(new Date(2026, 9, 1).getTime());
  });

  it('says how the steps split and what the chain cost, with the word for how it was learned', () => {
    const line = relayLine({
      turns: 3,
      turnsWithCloud: 1,
      stepsTotal: 20,
      stepsLocal: 18,
      stepsCloud: 2,
      cloudPercent: 10,
      attempts: 2,
      failed: 0,
      unverified: 0,
      tokensIn: 8000,
      tokensOut: 900,
      costUsd: 0.0042,
      costLabel: 'priced',
      byModel: [],
      reasons: [],
      overhead: null,
      trend: [],
    });

    expect(line).toBe('18 steps on the local model · 2 on the chain (10%) · $0.0042 (priced)');
  });
});
