import { beforeEach, describe, expect, it, vi } from 'vitest';

const sdcpCall = vi.hoisted(() => vi.fn());

vi.mock('../lib/sdcp', () => ({ sdcpCall }));
vi.mock('./intents', () => ({ refreshCatalog: vi.fn(async () => undefined) }));

const { applyPreset, loadFeatures, setSwitch, setUiMode, useFeatures } = await import('./features');

const view = (preset: string) => ({ features: [{ name: 'redact', on: true, default: true, tokenTechnique: false, scope: 'all', measured: true, what: 'x', active: true, inactiveReason: null }], preset, maxQuality: preset === 'safe' });

describe('profiles and Simple/Pro (Phase 4.4)', () => {
  beforeEach(() => {
    sdcpCall.mockReset();
    useFeatures.setState({ view: null, failed: false, mode: 'simple' });
  });

  it('loads the switches and the profile the daemon recognises', async () => {
    sdcpCall.mockResolvedValueOnce(view('balanced'));
    await loadFeatures();

    expect(useFeatures.getState().view?.preset).toBe('balanced');
  });

  it('applies a profile through the daemon and shows what it answered', async () => {
    sdcpCall.mockResolvedValueOnce(view('safe'));
    await applyPreset('safe');

    expect(sdcpCall).toHaveBeenCalledWith('features.preset', { preset: 'safe' });
    expect(useFeatures.getState().view?.maxQuality).toBe(true);
  });

  it('flips one switch by hand, then reads the list again (the profile may now be custom)', async () => {
    sdcpCall.mockResolvedValueOnce({}).mockResolvedValueOnce(view('custom'));
    await setSwitch('fleet', true);

    expect(sdcpCall).toHaveBeenNthCalledWith(1, 'settings.set', { key: 'v3.fleet', value: 'on' });
    expect(useFeatures.getState().view?.preset).toBe('custom');
  });

  it('says so, quietly, when the daemon cannot answer', async () => {
    sdcpCall.mockRejectedValueOnce(new Error('down'));
    await loadFeatures();

    expect(useFeatures.getState().failed).toBe(true);
  });

  it('keeps Simple or Pro', () => {
    setUiMode('pro');
    expect(useFeatures.getState().mode).toBe('pro');
    setUiMode('simple');
    expect(useFeatures.getState().mode).toBe('simple');
  });
});
