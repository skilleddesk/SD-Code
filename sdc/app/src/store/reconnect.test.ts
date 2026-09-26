import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { HostView } from './types';

/**
 * 0.11.5's two reports, both about a window that stopped asking.
 *
 * * *"amr CLI API providers sob kisu remove hoye gese"* - the daemon still had four connected providers;
 *   the window had asked for them once, at a moment the daemon was being replaced, and never again.
 * * *"reconnect ar jonno clash hole request korse nah"* - a VPS whose sign-in died went `offline`, and
 *   nothing opened the card where the password and the code are typed.
 *
 * Its own file because both behaviours live in module state (`loaded`, the armed watcher), and a fresh
 * import is the only honest starting point for them.
 */
const sdcpCall = vi.hoisted(() => vi.fn());

vi.mock('../lib/sdcp', () => ({ sdcpCall }));

const { connectDaemon, heartbeat, watchHosts } = await import('./intents');
const { useAppStore } = await import('./store');
const { useDaemonStore } = await import('./daemon');
const { useOverlayStore } = await import('./overlays');

const deepseek = { id: 'deepseek', name: 'DeepSeek', kind: 'api-key', status: 'connected' };

function answers(method: string): unknown {
  switch (method) {
    case 'provider.list':
      return { providers: [deepseek] };
    case 'session.list':
      return { hosts: [] };
    case 'project.list':
      return { projects: [] };
    default:
      return {};
  }
}

describe('the lists the window missed at launch', () => {
  beforeEach(() => {
    sdcpCall.mockReset();
    useAppStore.setState({ providers: [] });
    useDaemonStore.setState({ online: true, misses: 0, note: null });
  });

  it('are asked for again by the next heartbeat that reaches the daemon', async () => {
    /* The launch: the daemon is being replaced, so the very first call fails. */
    sdcpCall.mockRejectedValueOnce(new Error('the daemon closed the connection'));

    await expect(connectDaemon()).resolves.toBe(false);
    expect(useAppStore.getState().providers).toEqual([]);

    /* Five seconds later it answers. The window believed it was online all along, so the old heartbeat
       had nothing to say - and the Hub stayed empty until a relaunch. */
    sdcpCall.mockImplementation(async (method: string) => answers(method));

    await expect(heartbeat()).resolves.toBe(true);
    await vi.waitFor(() => expect(useAppStore.getState().providers.map((p) => p.id)).toEqual(['deepseek']));
    expect(sdcpCall).toHaveBeenCalledWith('provider.list', {});
  });
});

describe('a VPS whose connection died', () => {
  const vps = (status: HostView['status']): HostView => ({
    id: 'h7',
    name: 'prod-1',
    type: 'vps',
    status,
    sdcd: '0.11.5',
    platform: '',
    detail: '',
    hostKey: '',
    address: 'root@vps.example:8443',
    pinned: '',
    sessions: [],
  }) as HostView;

  let stop: () => void = () => {};

  beforeEach(async () => {
    vi.useFakeTimers();
    sdcpCall.mockReset();
    sdcpCall.mockImplementation(async (method: string) => answers(method));
    useOverlayStore.getState().closeAll();
    /* `loaded` - the watcher only arms once the daemon's lists are in. */
    await connectDaemon();
    useAppStore.setState({ hosts: [vps('connected')] });
    stop = watchHosts();
    await vi.advanceTimersByTimeAsync(4000);
  });

  afterEach(() => {
    stop();
    vi.useRealTimers();
  });

  it('opens its sign-in card when it turns offline', () => {
    useAppStore.setState({ hosts: [vps('offline')] });

    expect(useOverlayStore.getState().addHostOpen).toBe(true);
    expect(useOverlayStore.getState().addHostHostId).toBe('h7');
    expect(useAppStore.getState().toasts.at(-1)?.message).toBe(
      'Lost the connection to prod-1 - sign in again to reconnect',
    );
  });

  it('opens it after a Reconnect that measured offline', () => {
    useAppStore.setState({ hosts: [vps('offline')] });
    useOverlayStore.getState().closeAll();
    useAppStore.setState({ hosts: [vps('connecting')] });
    useAppStore.setState({ hosts: [vps('offline')] });

    expect(useOverlayStore.getState().addHostHostId).toBe('h7');
  });

  it('does not open over a dialog the person is already using', () => {
    useOverlayStore.getState().openSettings();
    useAppStore.setState({ hosts: [vps('offline')] });

    expect(useOverlayStore.getState().addHostOpen).toBe(false);
  });
});
