/** How tall one bar of the wave is, 0..1: zero at the room's noise floor (a flat line), full for a loud voice. */
export function waveLevel(rms: number, floor: number): number {
  return Math.min(1, Math.max(0, (rms - floor * 1.5) * 18));
}
