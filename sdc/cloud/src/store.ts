// Account-level data in D1: who to nudge, and how. No content, ever (plan 4.3). Message routing state is in each Hub.

import { b64uEncode, randomToken, sha256 } from './util';
import type { PushSubscription } from './webpush';

const encoder = new TextEncoder();

export const MAGIC_TTL_MS = 10 * 60_000;

/** Magic-link requests allowed per computer per hour, and per address-sender per hour. Beyond this the answer is the same, silently. */
export const MAGIC_PER_DAEMON_PER_HOUR = 5;

/** The foreign key on every table needs a `daemons` row. Created the first time anything is stored for a daemon. */
export async function ensureDaemon(db: D1Database, id: string, pub: string): Promise<void> {
  await db.prepare(`INSERT OR IGNORE INTO daemons (id, pub, created_at) VALUES (?, ?, ?)`).bind(id, pub, Date.now()).run();
}

// --- push subscriptions ---------------------------------------------------------------------------------

export async function savePush(db: D1Database, daemon: string, device: string, sub: PushSubscription): Promise<void> {
  await db
    .prepare(`INSERT OR REPLACE INTO push_subscriptions (daemon_id, device_id, endpoint, p256dh, auth, created_at) VALUES (?, ?, ?, ?, ?, ?)`)
    .bind(daemon, device, sub.endpoint, sub.p256dh, sub.auth, Date.now())
    .run();
}

export async function dropPush(db: D1Database, daemon: string, device: string): Promise<void> {
  await db.prepare(`DELETE FROM push_subscriptions WHERE daemon_id = ? AND device_id = ?`).bind(daemon, device).run();
}

export async function listPush(db: D1Database, daemon: string): Promise<Array<PushSubscription & { device: string }>> {
  const rows = await db.prepare(`SELECT device_id, endpoint, p256dh, auth FROM push_subscriptions WHERE daemon_id = ?`).bind(daemon).all<{ device_id: string; endpoint: string; p256dh: string; auth: string }>();

  return rows.results.map((row) => ({ device: row.device_id, endpoint: row.endpoint, p256dh: row.p256dh, auth: row.auth }));
}

// --- email address ----------------------------------------------------------------------------------------

/** The owner typed it on their own computer, which is the trust root, so it counts as verified. */
export async function saveEmail(db: D1Database, daemon: string, address: string): Promise<void> {
  await db
    .prepare(`INSERT INTO emails (daemon_id, address, verified_at) VALUES (?, ?, ?) ON CONFLICT(daemon_id) DO UPDATE SET address = excluded.address, verified_at = excluded.verified_at`)
    .bind(daemon, address.toLowerCase(), Date.now())
    .run();
}

export async function dropEmail(db: D1Database, daemon: string): Promise<void> {
  await db.prepare(`DELETE FROM emails WHERE daemon_id = ?`).bind(daemon).run();
}

export async function emailOf(db: D1Database, daemon: string): Promise<string | null> {
  const row = await db.prepare(`SELECT address FROM emails WHERE daemon_id = ? AND verified_at IS NOT NULL`).bind(daemon).first<{ address: string }>();

  return row?.address ?? null;
}

/** Computers that have this verified address. A few at most. */
export async function daemonsForEmail(db: D1Database, address: string): Promise<string[]> {
  const rows = await db.prepare(`SELECT daemon_id FROM emails WHERE address = ? AND verified_at IS NOT NULL LIMIT 3`).bind(address.toLowerCase()).all<{ daemon_id: string }>();

  return rows.results.map((row) => row.daemon_id);
}

// --- magic links -------------------------------------------------------------------------------------------

const hashOf = async (token: string) => b64uEncode(await sha256(encoder.encode(token)));

/** A fresh single-use token. Only its hash is stored, so a copy of the database holds no usable link. */
export async function createMagic(db: D1Database, daemon: string, now = Date.now()): Promise<string> {
  const token = randomToken(32);

  await db.prepare(`INSERT INTO magic_links (token_hash, daemon_id, created_at, expires_at) VALUES (?, ?, ?, ?)`).bind(await hashOf(token), daemon, now, now + MAGIC_TTL_MS).run();

  return token;
}

/**
 * Spends a token, once. The UPDATE is the check: of two requests racing with the same token exactly one changes a row.
 * `false` for a wrong, expired, already used or other-computer token, all alike.
 */
export async function spendMagic(db: D1Database, daemon: string, token: string, now = Date.now()): Promise<boolean> {
  if (token.length < 20 || token.length > 100) return false;

  const result = await db
    .prepare(`UPDATE magic_links SET used_at = ? WHERE token_hash = ? AND daemon_id = ? AND used_at IS NULL AND expires_at > ?`)
    .bind(now, await hashOf(token), daemon, now)
    .run();

  return (result.meta.changes ?? 0) === 1;
}

/** Gives a token back when nothing was granted (the computer turned out to be offline), so the person can try again. */
export async function releaseMagic(db: D1Database, daemon: string, token: string): Promise<void> {
  await db.prepare(`UPDATE magic_links SET used_at = NULL WHERE token_hash = ? AND daemon_id = ?`).bind(await hashOf(token), daemon).run();
}

export async function purgeMagic(db: D1Database, now = Date.now()): Promise<void> {
  await db.prepare(`DELETE FROM magic_links WHERE expires_at < ?`).bind(now - MAGIC_TTL_MS).run();
  await db.prepare(`DELETE FROM rate_limits WHERE window_start < ?`).bind(now - 2 * 3600_000).run();
}

// --- rate limits ----------------------------------------------------------------------------------------------

/** Counts one event in a fixed window. `true` while the count is within `limit`. */
export async function withinLimit(db: D1Database, bucket: string, limit: number, windowMs: number, now = Date.now()): Promise<boolean> {
  const row = await db
    .prepare(
      `INSERT INTO rate_limits (bucket, window_start, count) VALUES (?1, ?2, 1)
       ON CONFLICT(bucket) DO UPDATE SET
         count = CASE WHEN window_start + ?3 <= ?2 THEN 1 ELSE count + 1 END,
         window_start = CASE WHEN window_start + ?3 <= ?2 THEN ?2 ELSE window_start END
       RETURNING count`,
    )
    .bind(bucket, now, windowMs)
    .first<{ count: number }>();

  return (row?.count ?? limit + 1) <= limit;
}
