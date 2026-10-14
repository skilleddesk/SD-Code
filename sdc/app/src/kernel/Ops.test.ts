import { describe, expect, it } from 'vitest';

import { ago, stateOf, whenOf } from './opsView';

/**
 * The Workbench's three readings of a daemon value. Each of them was a bug waiting to happen: `ago` on a run
 * that never happened (0 is a real epoch second), `stateOf` on a state a newer daemon might invent, and
 * `whenOf` on an automation written with one of its two clocks and not the other.
 */
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
