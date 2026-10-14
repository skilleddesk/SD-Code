/**
 * **Handoff leases** (v3 X4): two of the person's screens can be looking at the same session at once - a phone on
 * the train, the desktop at home. Without a lease, both send turns and the session's state is written by whichever
 * answer arrives last. A lease says *this* device is driving until it stops: claiming hands out a fence - a number
 * that only ever goes up - and every frame that device sends afterwards carries the fence it was given. A frame
 * from an older fence is stale: another screen has taken over, and the Hub refuses it instead of relaying it, so
 * the two can never both be right.
 *
 * The Hub keeps the lease; the payloads stay sealed. Nothing here reads a frame's content - only its fence.
 */

/** How long a lease lasts without being renewed, in milliseconds (a claim is renewed by claiming again). */
export const DEFAULT_TTL = 120_000;

/** The longest a lease may be asked for, so a screen that goes away cannot lock the session out for hours. */
export const MAX_TTL = 600_000;

/** One session's lease. */
export interface Lease {
  session: string;
  /** The connection driving the session (its device, or its socket when the device is not named). */
  holder: string;
  /** The number every frame from the holder must carry. */
  fence: number;
  expires: number;
}

export type Verdict =
  /** The frame is from the device holding a live lease - relay it. */
  | { ok: true }
  /** No lease has ever been claimed for this session, or the last one has lapsed: the session is free, and a
   * frame without a fence is what a single-screen setup always sent - relay it, as before. */
  | { ok: true; warn: string }
  /** Another screen holds the session, or this frame is from a lease that was taken over - refuse it. */
  | { ok: false; reason: string };

/** The fence a claim gets: one more than the session's last, whether or not that lease is still alive. */
export function nextFence(previous: Lease | undefined): number {
  return (previous?.fence ?? 0) + 1;
}

/** A claim, with the lifetime asked for (clamped to what the Hub allows). */
export function claim(session: string, holder: string, previous: Lease | undefined, now: number, ttl?: number): Lease {
  const wanted = typeof ttl === 'number' && ttl > 0 ? ttl : DEFAULT_TTL;

  return { session, holder, fence: nextFence(previous), expires: now + Math.min(wanted, MAX_TTL) };
}

/**
 * May this frame be relayed? `fence` is what the sender carried (`undefined` when it carried none).
 *
 * - a live lease and a frame from its holder with its fence: yes;
 * - a live lease and a frame from an older fence, or from another device: no, and the sender is told which fence
 *   is current, so it can claim the session for itself if the person meant to drive from there;
 * - no live lease: yes, as before (a single screen needs no handoff).
 */
export function allow(lease: Lease | undefined, sender: string, fence: number | undefined, now: number): Verdict {
  if (!lease || lease.expires <= now) {
    return { ok: true, warn: 'no live handoff lease' };
  }

  if (fence !== undefined && fence < lease.fence) {
    return { ok: false, reason: `this screen is behind: the session is at fence ${lease.fence}` };
  }

  if (fence === undefined) {
    return { ok: false, reason: `another screen is driving this session (fence ${lease.fence}); claim it to take over` };
  }

  if (lease.holder !== sender) {
    return { ok: false, reason: `fence ${fence} belongs to another screen; claim the session to take over` };
  }

  return { ok: true };
}

/** Releasing is only allowed from the fence that holds it, so a takeover is never undone by the screen it replaced. */
export function release(lease: Lease | undefined, holder: string, fence: number): boolean {
  return Boolean(lease) && lease!.holder === holder && lease!.fence === fence;
}

/** What a screen is told about a session's lease (`status`): who is driving and until when. */
export function leaseState(lease: Lease | undefined, now: number): { held: boolean; fence: number; holder?: string; seconds_left?: number } {
  if (!lease || lease.expires <= now) return { held: false, fence: lease?.fence ?? 0 };

  return { held: true, fence: lease.fence, holder: lease.holder, seconds_left: Math.round((lease.expires - now) / 1000) };
}
