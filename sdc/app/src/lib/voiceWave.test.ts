import { describe, expect, it } from 'vitest';

import { waveLevel } from '../lib/voiceWave';

describe('the voice wave', () => {
  it('is flat in a quiet room', () => {
    expect(waveLevel(0.003, 0.003)).toBe(0);
    expect(waveLevel(0.004, 0.003)).toBe(0);
  });

  it('rises and falls with a voice, and never leaves 0..1', () => {
    const quiet = waveLevel(0.02, 0.003);
    const loud = waveLevel(0.08, 0.003);

    expect(quiet).toBeGreaterThan(0.2);
    expect(loud).toBeGreaterThan(quiet);
    expect(waveLevel(1, 0.003)).toBe(1);
  });
});
