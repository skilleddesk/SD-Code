import { beforeEach, describe, expect, it, vi } from 'vitest';

const sdcpCall = vi.hoisted(() => vi.fn());

vi.mock('../lib/sdcp', () => ({ sdcpCall }));

const { runShipSafe, useShipSafe } = await import('./shipSafe');
const { usePrefsStore } = await import('./prefs');
const { useAppStore } = await import('./store');
const { strings } = await import('../strings');

describe('Ship Safe (Phase 4.5)', () => {
  beforeEach(() => {
    sdcpCall.mockReset();
    useShipSafe.setState({ open: true, running: false, report: null, error: null, url: '' });
    usePrefsStore.setState({ activeTab: null });
  });

  it('asks for a folder instead of calling the daemon when none is open', async () => {
    await runShipSafe();

    expect(useShipSafe.getState().error).toBe(strings.shipSafe.noFolder);
    expect(sdcpCall).not.toHaveBeenCalled();
  });

  it('runs the checks on the open local folder with the site address, and keeps the report', async () => {
    useAppStore.setState({
      hosts: [{ id: 'local', name: 'This computer', type: 'local', sessions: [{ id: 's1', projectRoot: '/work/app', title: 'x' }] }] as never,
    });
    usePrefsStore.setState({ activeTab: 's1' });
    useShipSafe.setState({ url: 'https://staging.example.com' });
    sdcpCall.mockResolvedValueOnce({ verdict: 'warn', checks: [], site: 'https://staging.example.com' });

    await runShipSafe();

    expect(sdcpCall).toHaveBeenCalledWith('ship.check', { path: '/work/app', url: 'https://staging.example.com' });
    expect(useShipSafe.getState().report?.verdict).toBe('warn');
  });

  it('says a skipped check is not a pass', () => {
    expect(strings.shipSafe.verdict.warn).toMatch(/not a pass/);
  });
});
