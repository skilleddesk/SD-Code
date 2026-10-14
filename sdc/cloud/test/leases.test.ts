import { describe, expect, it } from 'vitest';

import { DEFAULT_TTL, MAX_TTL, allow, claim, leaseState, nextFence, release } from '../src/leases';

const NOW = 1_700_000_000_000;

describe('handoff leases', () => {
  it('gives each claim a higher fence and a lifetime the Hub caps', () => {
    const first = claim('s1', 'phone', undefined, NOW);

    expect(first).toEqual({ session: 's1', holder: 'phone', fence: 1, expires: NOW + DEFAULT_TTL });
    expect(nextFence(first)).toBe(2);

    const second = claim('s1', 'desktop', first, NOW, 10 * 60_000);

    expect(second.fence).toBe(2);
    expect(second.expires).toBe(NOW + MAX_TTL);
    expect(claim('s1', 'desktop', undefined, NOW, -5).expires).toBe(NOW + DEFAULT_TTL);
  });

  it('relays the holder and refuses a screen that is behind', () => {
    const lease = claim('s1', 'phone', undefined, NOW);

    expect(allow(lease, 'phone', 1, NOW)).toEqual({ ok: true });
    expect(allow(lease, 'phone', undefined, NOW).ok).toBe(false);
    expect(allow(lease, 'desktop', 1, NOW).ok).toBe(false);
    expect(allow(lease, 'phone', 0, NOW)).toEqual({ ok: false, reason: 'this screen is behind: the session is at fence 1' });
  });

  it('lets the session carry on once a lease has lapsed', () => {
    const lease = claim('s1', 'phone', undefined, NOW);

    expect(allow(lease, 'phone', 1, NOW + 1_000).ok).toBe(true);
    expect(allow(lease, 'phone', 1, NOW + DEFAULT_TTL + 1).ok).toBe(true);
    expect(allow(lease, 'desktop', undefined, NOW + DEFAULT_TTL + 1)).toEqual({ ok: true, warn: 'no live handoff lease' });
    expect(allow(undefined, 'phone', undefined, NOW).ok).toBe(true);
  });

  it('only the fence that holds the lease may release it, and status says who is driving', () => {
    const lease = claim('s1', 'phone', undefined, NOW);

    expect(release(lease, 'desktop', 1)).toBe(false);
    expect(release(lease, 'phone', 2)).toBe(false);
    expect(release(lease, 'phone', 1)).toBe(true);
    expect(release(undefined, 'phone', 1)).toBe(false);

    expect(leaseState(lease, NOW)).toEqual({ held: true, fence: 1, holder: 'phone', seconds_left: 120 });
    expect(leaseState(lease, NOW + DEFAULT_TTL)).toEqual({ held: false, fence: 1 });
    expect(leaseState(undefined, NOW)).toEqual({ held: false, fence: 0 });
  });

  it('a takeover means the replaced screen can no longer speak for the session', () => {
    const phone = claim('s1', 'phone', undefined, NOW);
    const desktop = claim('s1', 'desktop', phone, NOW + 1_000);

    expect(allow(desktop, 'desktop', 2, NOW + 2_000).ok).toBe(true);
    expect(allow(desktop, 'phone', 1, NOW + 2_000).ok).toBe(false);
    expect(release(desktop, 'phone', 1)).toBe(false);
  });
});
