import { describe, expect, it } from 'vitest';

import { strings } from '../strings';
import { micProblem } from '../lib/mic';

/** Phase 1.1: the microphone failing used to be one shrug ("could not be opened"); each cause now says what to do. */
describe('micProblem', () => {
  const named = (name: string): Error => Object.assign(new Error(name), { name });

  it('tells a blocked permission from a missing or busy microphone', () => {
    expect(micProblem(named('NotAllowedError'))).toBe(strings.kernel.voice.micDenied);
    expect(micProblem(named('NotFoundError'))).toBe(strings.kernel.voice.micNotFound);
    expect(micProblem(named('NotReadableError'))).toBe(strings.kernel.voice.micBusy);
  });

  it('names an unknown failure instead of hiding it', () => {
    expect(micProblem(named('WeirdError'))).toContain('WeirdError');
    expect(micProblem('not an error')).toContain('unknown error');
  });
});
