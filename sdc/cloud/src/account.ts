/**
 * **The person's own data** (v3 G36): what SDC Anywhere holds about one daemon - its email, its push endpoints,
 * its fixed address - can be read out as one JSON file, and can be deleted. Deletion is not a flag on a row: the
 * rows go, the relay is handed a new secret by the daemon itself (a deleted relay that kept answering would still
 * carry the person's traffic), and what remains is said plainly.
 *
 * The same file also says how long each kind of record is kept, so the answer to "how long do you keep it" is in
 * the code the person can read (`RETENTION`), and `purge` enforces it rather than leaving it to chance.
 */

import { dropEmail, dropPush, emailOf, listPush, purgeMagic } from './store';

/** How long each kind of record is kept, in the account's own words. */
export const RETENTION = {
  'one-time sign-in links': '24 hours, or until used (whichever comes first)',
  'push endpoints': 'until the device signs out or the account is deleted',
  'the daemon’s address and name': 'until the account is deleted',
  'relayed traffic': 'never stored - frames are passed through, sealed end to end',
  'rate-limit counters': '1 hour',
};

/** Everything the server holds about one daemon, ready to hand back to its owner. */
export interface Account {
  daemon: string;
  email: string | null;
  devices: Array<{ device: string; endpoint: string }>;
  retention: Record<string, string>;
  exported_at: string;
}

/** Reads one daemon's account. */
export async function exportAccount(db: D1Database, daemon: string, now = Date.now()): Promise<Account> {
  const email = await emailOf(db, daemon);
  const push = await listPush(db, daemon);

  return {
    daemon,
    email,
    devices: push.map((subscription) => ({ device: subscription.device, endpoint: subscription.endpoint })),
    retention: RETENTION,
    exported_at: new Date(now).toISOString(),
  };
}

/**
 * Deletes everything the server holds about one daemon: its email, its push endpoints and its one-time links.
 * Answers what was deleted, so the person is told rather than reassured.
 */
export async function deleteAccount(db: D1Database, daemon: string): Promise<{ deleted: string[] }> {
  const deleted: string[] = [];
  const push = await listPush(db, daemon);

  for (const subscription of push) {
    await dropPush(db, daemon, subscription.device);
    deleted.push(`push endpoint: ${subscription.device}`);
  }

  if (await emailOf(db, daemon)) {
    await dropEmail(db, daemon);
    deleted.push('email address');
  }

  const links = await db.prepare('DELETE FROM magic_links WHERE daemon_id = ?').bind(daemon).run();
  const counters = await db.prepare('DELETE FROM rate_limits WHERE bucket LIKE ?').bind(`%${daemon}%`).run();

  if ((links.meta?.changes ?? 0) > 0) deleted.push(`${links.meta?.changes} one-time link(s)`);
  if ((counters.meta?.changes ?? 0) > 0) deleted.push(`${counters.meta?.changes} rate-limit counter(s)`);

  return { deleted };
}

/** Enforces the retention above: sign-in links past their day go, whatever else is left of a half-finished sign-in. */
export async function purge(db: D1Database, now = Date.now()): Promise<void> {
  await purgeMagic(db, now);
  await db.prepare('DELETE FROM rate_limits WHERE window_start + 3600000 < ?').bind(now).run();
}
