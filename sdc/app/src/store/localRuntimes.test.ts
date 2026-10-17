import { beforeEach, describe, expect, it, vi } from 'vitest';

const sdcpCall = vi.hoisted(() => vi.fn());
const refreshProviders = vi.hoisted(() => vi.fn());
const refreshCatalog = vi.hoisted(() => vi.fn(async () => undefined));

vi.mock('../lib/sdcp', () => ({ sdcpCall }));
vi.mock('./tools', () => ({ refreshProviders }));
vi.mock('./intents', () => ({ refreshCatalog }));

const { rescanLocalRuntimes, useLocalRuntimes } = await import('./localRuntimes');

const none = { runtimes: [], none: true, noModels: false, installHint: 'No local model runtime found.', scannedAt: 'now', ms: 12, changed: false };

describe('local runtime rescan (Phase 2)', () => {
  beforeEach(() => {
    sdcpCall.mockReset();
    refreshProviders.mockReset();
    refreshCatalog.mockClear();
    useLocalRuntimes.setState({ scan: null, scanning: false, lastAt: 0 });
  });

  it('keeps "nothing found" as an ordinary result and does not touch the model menu', async () => {
    sdcpCall.mockResolvedValueOnce(none);
    await rescanLocalRuntimes();

    expect(useLocalRuntimes.getState().scan?.none).toBe(true);
    expect(refreshProviders).not.toHaveBeenCalled();
  });

  it('re-reads the providers and the catalogue only when the scan changed them', async () => {
    sdcpCall.mockResolvedValueOnce({ ...none, none: false, changed: true, runtimes: [{ id: 'local-1', kind: 'openai-compatible', label: 'LM Studio', base: 'x', models: [{ id: 'm', ctx: 8192 }], state: 'ready' }] });
    await rescanLocalRuntimes();

    expect(refreshProviders).toHaveBeenCalledTimes(1);
    expect(refreshCatalog).toHaveBeenCalledTimes(1);
  });

  it('never throws at the person when the daemon cannot answer, and keeps the last result', async () => {
    useLocalRuntimes.setState({ scan: none });
    sdcpCall.mockRejectedValueOnce(new Error('daemon down'));

    await expect(rescanLocalRuntimes()).resolves.toBeUndefined();
    expect(useLocalRuntimes.getState().scan).toEqual(none);
    expect(useLocalRuntimes.getState().scanning).toBe(false);
  });

  it('does not scan again inside the minimum gap (focus changes must not loop)', async () => {
    sdcpCall.mockResolvedValue(none);
    await rescanLocalRuntimes();
    await rescanLocalRuntimes(60_000);

    expect(sdcpCall).toHaveBeenCalledTimes(1);
  });
});
