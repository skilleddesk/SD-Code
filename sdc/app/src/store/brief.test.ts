import { beforeEach, describe, expect, it, vi } from 'vitest';

const sdcpCall = vi.hoisted(() => vi.fn());

vi.mock('../lib/sdcp', () => ({ sdcpCall }));

const { loadBrief, loadClientReport, setDaily, useBrief } = await import('./brief');

describe('the morning brief and the client report (Phase 4.8)', () => {
  beforeEach(() => {
    sdcpCall.mockReset();
    useBrief.setState({ open: true, busy: false, kind: null, headline: '', html: '', path: '', error: null, daily: false });
  });

  it('shows the daemon’s headline and page', async () => {
    sdcpCall.mockResolvedValueOnce({ brief: {}, headline: 'Morning brief: nothing needs your attention.', htmlPath: '/data/reports/brief.html', html: '<p>x</p>' });
    await loadBrief();

    expect(useBrief.getState().headline).toContain('nothing needs your attention');
    expect(useBrief.getState().kind).toBe('brief');
    expect(useBrief.getState().path).toBe('/data/reports/brief.html');
  });

  it('asks for seven days for the client report', async () => {
    sdcpCall.mockResolvedValueOnce({ report: {}, htmlPath: '/data/reports/client.html', html: '<p>y</p>' });
    await loadClientReport();

    expect(sdcpCall).toHaveBeenCalledWith('report.client', expect.objectContaining({ days: 7 }));
    expect(useBrief.getState().kind).toBe('client');
  });

  it('keeps an error as a sentence and the daily switch as the daemon accepted it', async () => {
    sdcpCall.mockRejectedValueOnce(new Error('x'));
    await loadBrief();

    expect(useBrief.getState().error).not.toBeNull();

    sdcpCall.mockResolvedValueOnce({});
    await setDaily(true);

    expect(sdcpCall).toHaveBeenLastCalledWith('settings.set', { key: 'brief.daily', value: 'on' });
    expect(useBrief.getState().daily).toBe(true);
  });
});
