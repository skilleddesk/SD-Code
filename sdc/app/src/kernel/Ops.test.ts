import { describe, expect, it } from 'vitest';

import { ago, stateOf, whenOf } from './opsView';

/**
 * The Workbench's three readings of a daemon value. Each of them was a bug waiting to happen: `ago` on a run
 * that never happened (0 is a real epoch second), `stateOf` on a state a newer daemon might invent, and
 * `whenOf` on an automation written with one of its two clocks and not the other.
 */
import { bn } from '../locales/bn';
import { kernelStrings } from '../kernelStrings';

describe('the Workbench readings', () => {
  const now = 1_700_000_000;

  it('says a run that never happened never happened, rather than 1970', () => {
    expect(ago(null, now)).toBe('Never run');
    expect(ago(undefined, now)).toBe('Never run');
    expect(ago(0, now)).toBe('Never run');
  });

  it('reads a run in the unit a person would use', () => {
    expect(ago(now - 20, now)).toBe('just now');
    expect(ago(now - 60 * 5, now)).toBe('5 min ago');
    expect(ago(now - 60 * 60 * 3, now)).toBe('3 h ago');
    expect(ago(now - 60 * 60 * 50, now)).toBe('2 d ago');
  });

  it('never counts a clock that is ahead as time going backwards', () => {
    expect(ago(now + 600, now)).toBe('just now');
  });

  it('reads an agent state as a word, and agrees with one it has never heard of', () => {
    expect(stateOf('running').label).toBe('Running');
    expect(stateOf('done').label).toBe('Done');
    expect(stateOf('failed').label).toBe('Stopped by an error');
    expect(stateOf('stopped').label).toBe('Stopped');
    expect(stateOf('something-new').label).toBe('Stopped');
  });

  it('reads an automation with a span, one with a time of day, and one with neither', () => {
    expect(whenOf({ every: '6h', at: null })).toBe('Every 6h');
    expect(whenOf({ every: null, at: '09:30' })).toBe('Every day at 09:30');
    expect(whenOf({ every: null, at: null })).toBe('Only when you run it');
  });
});

/**
 * The Workbench in Bengali. A group that is missing from a pack falls back to English word by word (locales/index.ts),
 * so the person who reads Bengali would see half a window in the other language. This is the check that the pack has
 * the whole group, with the same keys the English has.
 */
describe('the Workbench in Bengali', () => {
  it('has the whole group, key for key', () => {
    const english = kernelStrings.ops as unknown as Record<string, unknown>;
    const bengali = bn.kernel?.ops as unknown as Record<string, unknown> | undefined;

    expect(bengali).toBeTruthy();
    expect(Object.keys(bengali!).sort()).toEqual(Object.keys(english).sort());
  });

  it('translates the three tabs and the tab bodies, not just the title', () => {
    const english = kernelStrings.ops as unknown as Record<string, unknown>;
    const bengali = bn.kernel?.ops as unknown as Record<string, unknown>;

    for (const group of ['title', 'loading', 'failed']) {
      expect(bengali[group]).toBeTruthy();
      expect(bengali[group]).not.toBe(english[group]);
    }

    for (const group of ['runners', 'agents', 'inbox']) {
      const keys = Object.keys(english[group] as object).sort();

      expect(Object.keys(bengali[group] as object).sort()).toEqual(keys);
    }
  });

  it('writes the numbers into the same sentences, so a value is never left out', () => {
    const english = kernelStrings.ops.agents.started('a1');
    const bengali = (bn.kernel?.ops as typeof kernelStrings.ops).agents.started('a1');

    expect(english).toContain('a1');
    expect(bengali).toContain('a1');
    expect((bn.kernel?.ops as typeof kernelStrings.ops).inbox.ran('nightly')).toContain('nightly');
  });
});
