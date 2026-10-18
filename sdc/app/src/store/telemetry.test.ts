import { beforeEach, describe, expect, it, vi } from 'vitest';

const sdcpCall = vi.hoisted(() => vi.fn());

vi.mock('../lib/sdcp', () => ({ sdcpCall }));

const { loadTelemetry, sendTelemetry, setTelemetry } = await import('./telemetry');
const { strings } = await import('../strings');

describe('opt-in telemetry in the window (Phase 4.2)', () => {
  beforeEach(() => sdcpCall.mockReset());

  it('turning it on sends only the choice (and an endpoint when one was typed)', async () => {
    sdcpCall.mockResolvedValue({ optIn: true });
    await setTelemetry(true, '');
    await setTelemetry(true, ' https://example.org/c ');

    expect(sdcpCall).toHaveBeenNthCalledWith(1, 'telemetry.set', { optIn: true });
    expect(sdcpCall).toHaveBeenNthCalledWith(2, 'telemetry.set', { optIn: true, endpoint: 'https://example.org/c' });
  });

  it('says why a send did not happen, in the daemon’s own words, and never throws', async () => {
    sdcpCall.mockRejectedValueOnce(new Error('plain failure'));
    await expect(sendTelemetry()).resolves.toBe(strings.telemetry.sendFailed);
  });

  it('reads nothing when the daemon is away', async () => {
    sdcpCall.mockRejectedValueOnce(new Error('down'));
    await expect(loadTelemetry()).resolves.toBeNull();
  });

  it('promises off by default, counts only, and no server yet', () => {
    expect(strings.telemetry.promise).toMatch(/Off unless you turn it on/);
    expect(strings.telemetry.never).toMatch(/code, your prompts/);
    expect(strings.telemetry.noServer).toMatch(/no server of its own yet/);
  });
});
