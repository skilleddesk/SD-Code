import { describe, expect, it } from 'vitest';

import { merge } from '../i18n';
import { PACKS } from '../locales';
import { applyKernel, chatCost, EMPTY_KERNEL, measured, usd } from './kernel';
import { applyEvent, EMPTY_STATE } from './reducer';

describe('the Trust Kernel slice (0.12)', () => {
  it('folds costs, scores and a kill switch from the log', () => {
    let state = applyKernel(
      EMPTY_KERNEL,
      { type: 'CostUpdated', sessionId: 's1', turnId: 't1', engine: 'native_api', model: 'deepseek-chat', inputTokens: 1000, outputTokens: 200, costUsd: 0.0021, costSource: 'priced', estimateUsd: 0.003, savedUsd: null },
      '2026-09-28T10:00:00Z',
    );

    state = applyKernel(state, { type: 'TrustScored', sessionId: 's1', turnId: 't1', score: 70, level: 'medium', reasons: [{ text: 'Not verified yet', delta: -30 }] }, '2026-09-28T10:00:01Z');
    state = applyKernel(state, { type: 'KillSwitch', stopped: [], checkpoints: [] }, '2026-09-28T10:00:02Z');

    expect(chatCost(state, 's1')).toBeCloseTo(0.0021);
    expect(state.scores.t1?.level).toBe('medium');
    expect(state.killSwitch?.at).toBe('2026-09-28T10:00:02Z');
  });

  it('routes kernel events through the app reducer into their slice, and labels checkpoints', () => {
    const withCheckpoint = applyEvent(EMPTY_STATE, {
      seq: 1,
      ts: '2026-09-28T10:00:00Z',
      event: { type: 'CheckpointSaved', sessionId: 's1', checkpoint: { id: 'cp-1', turn: 3, ts: 'now', title: 'Before Edit a.ts', filesHash: 'abc', label: null, irreversible: null } },
    });
    const labelled = applyEvent(withCheckpoint, {
      seq: 2,
      ts: '2026-09-28T10:00:01Z',
      event: { type: 'CheckpointUpdated', sessionId: 's1', checkpoint: { id: 'cp-1', turn: 3, ts: 'now', title: 'Before Edit a.ts', filesHash: 'abc', label: 'Before deploy', irreversible: 'ran `git push`' } },
    });
    const scored = applyEvent(labelled, {
      seq: 3,
      ts: '2026-09-28T10:00:02Z',
      event: { type: 'TrustScored', sessionId: 's1', turnId: 't9', score: 100, level: 'high', reasons: [] },
    });

    expect(labelled.checkpoints[0]?.label).toBe('Before deploy');
    expect(labelled.checkpoints[0]?.irreversible).toContain('git push');
    expect(scored.kernel.scores.t9?.score).toBe(100);
  });

  it('says a number is an estimate unless it was measured', () => {
    expect(measured('measured')).toBe(true);
    expect(measured('subscription')).toBe(false);
    expect(usd(0.0031)).toBe('$0.0031');
    expect(usd(1.2)).toBe('$1.20');
    expect(usd(null)).toBe('—');
  });
});

describe('the language packs (0.12)', () => {
  it('lay a language over English and fall back for what they leave out', () => {
    const base = { a: 'one', nested: { b: 'two', c: (n: number) => `${n} items` }, list: ['x'] };
    const merged = merge(base, { nested: { b: 'দুই', c: (n: number) => `${n}টি` } });

    expect(merged.a).toBe('one');
    expect(merged.nested.b).toBe('দুই');
    expect(merged.nested.c(3)).toBe('3টি');
    expect(merged.list).toEqual(['x']);
  });

  it('ships ten languages, English being the base', () => {
    expect(Object.keys(PACKS).sort()).toEqual(['ar', 'bn', 'en', 'es', 'fr', 'hi', 'id', 'pt', 'ur', 'zh']);
  });
});
