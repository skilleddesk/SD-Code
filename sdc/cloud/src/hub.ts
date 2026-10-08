// The Hub: one Durable Object per daemon. It is a switchboard for ciphertext.
//
// What it knows: the daemon's public identity key (pinned on first contact; its id is the hash of it, so
// nobody else can claim the id), which device public keys the daemon told it about, and which connections
// are open. What it never sees: any key that opens a session, or any plaintext. The messages it forwards
// are the `hello`, `welcome` and `f` frames of the end-to-end protocol, which only the daemon and a paired
// browser can read. The Hub cannot read, alter without detection, or replay them (see sdcd's crypto.rs).
//
// Uses the WebSocket Hibernation API, so an idle account costs no compute: the object sleeps between
// messages, and per-socket state lives in the socket's attachment, not in memory.

import { DurableObject } from 'cloudflare:workers';
import { approvalMail, sendMail, validAddress, type EmailEnv } from './email';
import { dropEmail, dropPush, emailOf, ensureDaemon, listPush, saveEmail, savePush, withinLimit } from './store';
import { DEVICE_ID, b64uDecode, b64uEncode, daemonIdOf, deviceAuthMessage, hubAuthMessage, randomToken, verifySignature } from './util';
import { approvalPayload, parseSubscription, send as sendPush, type Vapid } from './webpush';

export interface Env extends EmailEnv {
  HUB: DurableObjectNamespace<Hub>;
  DB: D1Database;
  ASSETS: Fetcher;
  WEB_ORIGIN: string;
  RP_ID: string;
  /** Web Push (RFC 8292). The public half is also what the page subscribes with; the private half is a Worker secret. */
  VAPID_PUBLIC_KEY?: string;
  VAPID_PRIVATE_KEY?: string;
  VAPID_SUBJECT?: string;
  PAIR_LIMIT?: { limit(options: { key: string }): Promise<{ success: boolean }> };
  CONNECT_LIMIT?: { limit(options: { key: string }): Promise<{ success: boolean }> };
  MAGIC_LIMIT?: { limit(options: { key: string }): Promise<{ success: boolean }> };
}

/** What the browser needs to start pairing after it spent a sign-in link. */
export type Offer = { ok: true; fragment: string; fingerprint: string; expiresAt: number } | { ok: false; why: 'offline' | 'refused' };

type Role = 'daemon' | 'device' | 'pair';
type Phase = 'challenge' | 'ready';

interface Attachment {
  role: Role;
  /** The daemon this Hub belongs to, from the URL. Kept per socket, not in storage: an id nobody ever signed in for leaves nothing behind. */
  daemonId: string;
  phase: Phase;
  nonce?: string;
  since: number;
  device?: string;
  conn?: string;
  /** Messages a pairing socket has sent; it may send only a few. */
  sent?: number;
}

/** Largest message the Hub forwards. A sealed frame is at most 256 KB of payload plus framing. */
export const MAX_FROM_DEVICE = 320 * 1024;
export const MAX_FROM_DAEMON = 1024 * 1024;
/** A pairing socket sends one hello (a few KB), so it gets a small budget. */
export const MAX_FROM_PAIR = 16 * 1024;
export const MAX_PAIR_MESSAGES = 3;
export const MAX_PAIR_SOCKETS = 8;
/** How long a socket may sit unauthenticated. */
export const AUTH_WINDOW_MS = 15_000;
/** Per-socket message budget: this many per second, then the socket is closed. A browser sends small requests, so its budget is
 *  small; the daemon is authenticated and sends a big reply as many 64 KB pieces, so its budget is large (it is still a ceiling). */
export const RATE_PER_SECOND = 300;
export const DAEMON_RATE_PER_SECOND = 20_000;
/** Push first; if nobody has answered by then, one email. The daemon may ask for another time with `escalate_after_sec`. */
export const DEFAULT_ESCALATE_SEC = 60;
/** At most this many requests wait in a Hub; a daemon cannot make the relay hold more. */
export const MAX_PENDING = 100;
/** Ceilings on notifications per computer per hour, so a runaway loop cannot buzz a phone or fill a mailbox. */
export const PUSH_PER_HOUR = 60;
export const EMAIL_PER_HOUR = 10;
/** A vault is an AES-GCM blob of a key and two public keys: a couple of kilobytes at most. */
export const MAX_VAULT_CHARS = 4096;
export const MAX_VAULTS = 64;
const VAULT_ID = /^[A-Za-z0-9_-]{22}$/;
const VAULT_BLOB = /^[A-Za-z0-9_-]+$/;

/** How long the Hub waits for the daemon to make a pairing offer for a sign-in link. */
export const OFFER_WAIT_MS = 8_000;

/** A request id as the daemon makes it (`apr_` + permission id). It goes into a link, so it is restricted to safe characters. */
const REQUEST_ID = /^[A-Za-z0-9_][A-Za-z0-9_.-]{0,127}$/;

export const CLOSE = {
  replaced: 4000,
  denied: 4001,
  offline: 4002,
  daemonGone: 4003,
  tooBig: 1009,
  rate: 4008,
  timeout: 4009,
} as const;

export class Hub extends DurableObject<Env> {
  /** Message counters per socket, kept in memory only: a rate limit that forgets on hibernation is fine. */
  private readonly rates = new Map<WebSocket, { windowStart: number; count: number }>();
  /** Pairing offers the daemon has been asked for and has not answered yet. Memory only: an offer is a matter of seconds. */
  private readonly offers = new Map<string, (offer: Offer) => void>();

  constructor(ctx: DurableObjectState, env: Env) {
    super(ctx, env);
    ctx.blockConcurrencyWhile(async () => {
      const sql = this.ctx.storage.sql;

      sql.exec(`CREATE TABLE IF NOT EXISTS kv (k TEXT PRIMARY KEY, v TEXT NOT NULL)`);
      sql.exec(`CREATE TABLE IF NOT EXISTS devices (id TEXT PRIMARY KEY, pub TEXT NOT NULL, guest INTEGER NOT NULL DEFAULT 0)`);
      sql.exec(
        `CREATE TABLE IF NOT EXISTS pending (request TEXT PRIMARY KEY, created_at INTEGER NOT NULL, expires_at INTEGER NOT NULL, pushed_at INTEGER, emailed_at INTEGER)`,
      );

      // Sealed copies of paired browsers' keys (web/src/crypto/vault.ts): ciphertext this Hub cannot open, found by an id only the owner's passkey can produce.
      sql.exec(`CREATE TABLE IF NOT EXISTS vaults (id TEXT PRIMARY KEY, device TEXT NOT NULL, blob TEXT NOT NULL, saved_at INTEGER NOT NULL)`);

      // `escalate_at` came after the first version of this table; add it to a Hub that already has the table without it.
      const columns = sql.exec(`PRAGMA table_info(pending)`).toArray() as Array<{ name: string }>;

      if (!columns.some((column) => column.name === 'escalate_at')) sql.exec(`ALTER TABLE pending ADD COLUMN escalate_at INTEGER`);
    });
  }

  // --- entry --------------------------------------------------------------------------------------

  async fetch(request: Request): Promise<Response> {
    const url = new URL(request.url);
    const role = url.searchParams.get('role') as Role | null;
    const daemonId = url.searchParams.get('daemon') ?? '';

    if (request.headers.get('Upgrade') !== 'websocket') return new Response('Expected a WebSocket', { status: 426 });
    if (role !== 'daemon' && role !== 'device' && role !== 'pair') return new Response('Bad role', { status: 400 });

    if (role === 'pair' && this.ctx.getWebSockets('pair').length >= MAX_PAIR_SOCKETS) {
      return new Response('Too many pairing attempts at once', { status: 429 });
    }

    const device = url.searchParams.get('device') ?? undefined;

    if (role === 'device' && (!device || !DEVICE_ID.test(device))) return new Response('Bad device', { status: 400 });

    const pair = new WebSocketPair();
    const [client, server] = Object.values(pair) as [WebSocket, WebSocket];

    // A browser asking for a computer that has never signed in here: say it is offline and keep nothing.
    if (role !== 'daemon' && !this.pinnedKey()) {
      this.ctx.acceptWebSocket(server, ['stray']);
      server.send(JSON.stringify({ t: 'offline' }));
      server.close(CLOSE.offline, 'the computer is offline');

      return new Response(null, { status: 101, webSocket: client });
    }

    const attachment: Attachment = { role, daemonId, phase: role === 'pair' ? 'ready' : 'challenge', since: Date.now(), device };

    if (role !== 'pair') attachment.nonce = randomToken(24);
    if (role === 'daemon') {
      // Only one daemon connection at a time: a new one replaces the old.
      for (const old of this.ctx.getWebSockets('daemon')) old.close(CLOSE.replaced, 'replaced by a newer connection');
    }

    attachment.conn = role === 'daemon' ? undefined : randomToken(9);
    this.ctx.acceptWebSocket(server, attachment.conn ? [role, `conn:${attachment.conn}`] : [role]);
    server.serializeAttachment(attachment);

    if (role !== 'pair') await this.rearm();

    if (attachment.nonce) server.send(JSON.stringify({ t: 'challenge', nonce: attachment.nonce }));
    if (role === 'pair' && !this.toDaemon({ conn: attachment.conn, open: { pairing: true } })) this.refuse(server, 'offline');

    return new Response(null, { status: 101, webSocket: client });
  }

  // --- messages -------------------------------------------------------------------------------------

  async webSocketMessage(ws: WebSocket, data: string | ArrayBuffer): Promise<void> {
    const attachment = ws.deserializeAttachment() as Attachment | null;

    if (!attachment) return void ws.close(CLOSE.denied, 'no state');
    if (typeof data !== 'string') return void ws.close(1003, 'text only');

    const limit = attachment.role === 'daemon' ? MAX_FROM_DAEMON : attachment.role === 'pair' ? MAX_FROM_PAIR : MAX_FROM_DEVICE;

    if (data.length > limit) return void ws.close(CLOSE.tooBig, 'message too big');
    if (!this.withinRate(ws, attachment.role === 'daemon' ? DAEMON_RATE_PER_SECOND : RATE_PER_SECOND)) return void ws.close(CLOSE.rate, 'too many messages');

    let message: any;

    try {
      message = JSON.parse(data);
    } catch {
      return void ws.close(1003, 'not JSON');
    }

    if (typeof message !== 'object' || message === null) return void ws.close(1003, 'not an object');

    switch (attachment.role) {
      case 'daemon':
        return attachment.phase === 'challenge' ? this.daemonAuth(ws, attachment, message) : this.fromDaemon(message);
      case 'device':
        return attachment.phase === 'challenge' ? this.deviceAuth(ws, attachment, message) : this.fromDevice(ws, attachment, message);
      case 'pair':
        return this.fromPair(ws, attachment, message);
    }
  }

  async webSocketClose(ws: WebSocket): Promise<void> {
    this.rates.delete(ws);

    const attachment = ws.deserializeAttachment() as Attachment | null;

    if (!attachment) return;

    if (attachment.role === 'daemon') {
      // The PC went away: every browser connection through it is dead.
      for (const other of this.ctx.getWebSockets()) {
        const info = other.deserializeAttachment() as Attachment | null;
        if (info && info.role !== 'daemon') other.close(CLOSE.daemonGone, 'the computer went offline');
      }
    } else if (attachment.conn) {
      this.toDaemon({ conn: attachment.conn, close: true });
    }
  }

  async webSocketError(ws: WebSocket): Promise<void> {
    await this.webSocketClose(ws);
  }

  /** Closes sockets that never authenticated, expires old notifications, and sends the email for a request nobody answered. */
  async alarm(): Promise<void> {
    const now = Date.now();

    for (const ws of this.ctx.getWebSockets()) {
      const attachment = ws.deserializeAttachment() as Attachment | null;

      if (attachment && attachment.phase === 'challenge' && now - attachment.since >= AUTH_WINDOW_MS) ws.close(CLOSE.timeout, 'did not authenticate in time');
    }

    const sql = this.ctx.storage.sql;

    sql.exec(`DELETE FROM pending WHERE expires_at < ?`, now);

    // Every request whose time has come is marked first, then one email goes out for all of them: at most once, never a burst.
    const due = sql.exec(`SELECT request FROM pending WHERE emailed_at IS NULL AND escalate_at IS NOT NULL AND escalate_at <= ?`, now).toArray() as Array<{ request: string }>;

    if (due.length > 0) {
      sql.exec(`UPDATE pending SET emailed_at = ? WHERE emailed_at IS NULL AND escalate_at IS NOT NULL AND escalate_at <= ?`, now, now);
      await this.email(due[0]!.request);
    }

    await this.rearm();
  }

  /** One alarm serves three jobs; set it to the soonest of them (or clear it when there is nothing to wait for). */
  private async rearm(): Promise<void> {
    const now = Date.now();
    let next = Number.POSITIVE_INFINITY;

    for (const ws of this.ctx.getWebSockets()) {
      const attachment = ws.deserializeAttachment() as Attachment | null;

      // A socket already past its window is being closed by the alarm that is running; it must not set another.
      if (attachment && attachment.phase === 'challenge' && attachment.since + AUTH_WINDOW_MS > now) next = Math.min(next, attachment.since + AUTH_WINDOW_MS);
    }

    const row = this.ctx.storage.sql
      .exec(`SELECT MIN(CASE WHEN emailed_at IS NULL AND escalate_at IS NOT NULL THEN escalate_at END) AS escalate, MIN(expires_at) AS expires FROM pending`)
      .toArray()[0] as { escalate: number | null; expires: number | null } | undefined;

    if (row?.escalate != null) next = Math.min(next, Math.max(row.escalate, now + 1));
    if (row?.expires != null) next = Math.min(next, Math.max(row.expires + 1, now + 1));

    if (Number.isFinite(next)) await this.ctx.storage.setAlarm(next);
    else await this.ctx.storage.deleteAlarm();
  }

  // --- the daemon ---------------------------------------------------------------------------------------

  private pinnedKey(): string | null {
    const row = this.ctx.storage.sql.exec(`SELECT v FROM kv WHERE k = 'daemon_pub'`).toArray()[0] as { v: string } | undefined;

    return row?.v ?? null;
  }

  private async daemonAuth(ws: WebSocket, attachment: Attachment, message: any): Promise<void> {
    const daemonId = attachment.daemonId;
    let pub: Uint8Array;
    let sig: Uint8Array;

    try {
      pub = b64uDecode(String(message.pub));
      sig = b64uDecode(String(message.sig));
    } catch {
      return this.deny(ws, 'malformed');
    }

    if (message.t !== 'auth' || (await daemonIdOf(pub)) !== daemonId) return this.deny(ws, 'this key is not the one the id is the hash of');
    if (!(await verifySignature(pub, hubAuthMessage(attachment.nonce ?? '', daemonId), sig))) return this.deny(ws, 'bad signature');

    // Pin the key on first contact. The id already commits to it; this is belt and braces.
    const pinned = this.pinnedKey();

    if (pinned && pinned !== b64uEncode(pub)) return this.deny(ws, 'a different key is already pinned for this id');
    if (!pinned) this.ctx.storage.sql.exec(`INSERT INTO kv (k, v) VALUES ('daemon_pub', ?)`, b64uEncode(pub));

    attachment.phase = 'ready';
    ws.serializeAttachment(attachment);
    ws.send(JSON.stringify({ t: 'ready' }));
  }

  private async fromDaemon(message: any): Promise<void> {
    if (typeof message.ctl === 'string') return this.control(message);

    if (typeof message.conn !== 'string') return;

    const target = this.ctx.getWebSockets(`conn:${message.conn}`)[0];

    if (!target) return;

    if (message.close) return void target.close(1000, 'closed by the computer');

    if (message.msg !== undefined) target.send(JSON.stringify(message.msg));
  }

  private async control(message: any): Promise<void> {
    const sql = this.ctx.storage.sql;

    switch (message.ctl) {
      case 'ping':
        this.toDaemon({ t: 'pong' });
        return;
      case 'devices.sync': {
        if (!Array.isArray(message.devices) || message.devices.length > 64) return;
        sql.exec(`DELETE FROM devices`);
        for (const device of message.devices) this.addDevice(device);
        // A vault for a device the computer no longer lists is of no use to anyone: drop it.
        sql.exec(`DELETE FROM vaults WHERE device NOT IN (SELECT id FROM devices)`);
        return;
      }
      case 'device.add':
        this.addDevice(message);
        return;
      case 'device.remove': {
        const id = String(message.id ?? '');
        sql.exec(`DELETE FROM devices WHERE id = ?`, id);
        sql.exec(`DELETE FROM vaults WHERE device = ?`, id);
        for (const ws of this.ctx.getWebSockets('device')) {
          const info = ws.deserializeAttachment() as Attachment | null;
          if (info?.device === id) ws.close(CLOSE.denied, 'device removed');
        }
        // A removed device must stop being buzzed.
        const daemon = await this.daemonId();

        if (daemon) await dropPush(this.env.DB, daemon, id).catch(() => undefined);
        return;
      }
      case 'notify':
        return this.notify(message);
      case 'clear':
        sql.exec(`DELETE FROM pending WHERE request = ?`, String(message.request ?? ''));
        await this.rearm();
        return;
      case 'email.set':
        return this.setEmail(message.address);
      case 'offer':
        return this.offered(message);
    }
  }

  // --- notifications: live, then push, then email ----------------------------------------------------------------

  /** The id this Hub belongs to, from the key it pinned. `null` until a daemon has signed in. */
  private async daemonId(): Promise<string | null> {
    const pub = this.pinnedKey();

    return pub ? daemonIdOf(b64uDecode(pub)) : null;
  }

  /** A browser that is connected and signed in will show the card itself; no push is needed for it. */
  private hasLiveDevice(): boolean {
    return this.ctx.getWebSockets('device').some((ws) => (ws.deserializeAttachment() as Attachment | null)?.phase === 'ready');
  }

  private vapid(): Vapid | null {
    const { VAPID_PUBLIC_KEY: publicKey, VAPID_PRIVATE_KEY: privateKey, VAPID_SUBJECT: subject } = this.env;

    return publicKey && privateKey ? { publicKey, privateKey, subject: subject || 'mailto:notify@skilleddesk.com' } : null;
  }

  private async notify(message: any): Promise<void> {
    const request = String(message.request ?? '');
    const expires = Number(message.expires_at);

    if (!REQUEST_ID.test(request) || !Number.isFinite(expires)) return;

    const sql = this.ctx.storage.sql;
    const now = Date.now();
    const asked = message.escalate_after_sec === undefined ? DEFAULT_ESCALATE_SEC : Number(message.escalate_after_sec);
    const after = Math.min(3600, Math.max(1, Number.isFinite(asked) ? asked : DEFAULT_ESCALATE_SEC));

    // A second notice for the same request changes nothing about when it is pushed or emailed.
    sql.exec(
      `INSERT INTO pending (request, created_at, expires_at, escalate_at) VALUES (?, ?, ?, ?) ON CONFLICT(request) DO UPDATE SET expires_at = excluded.expires_at`,
      request,
      now,
      expires,
      now + after * 1000,
    );
    sql.exec(`DELETE FROM pending WHERE request IN (SELECT request FROM pending ORDER BY created_at DESC LIMIT -1 OFFSET ?)`, MAX_PENDING);
    await this.rearm();

    const row = sql.exec(`SELECT pushed_at FROM pending WHERE request = ?`, request).toArray()[0] as { pushed_at: number | null } | undefined;

    if (row && row.pushed_at === null && !this.hasLiveDevice()) await this.push(request);
  }

  /** One push to every browser of this computer. Carries a kind and a link, never a detail (plan 5.10). */
  private async push(request: string): Promise<void> {
    const vapid = this.vapid();
    const daemon = await this.daemonId();

    if (!vapid || !daemon) return;

    this.ctx.storage.sql.exec(`UPDATE pending SET pushed_at = ? WHERE request = ?`, Date.now(), request);

    try {
      if (!(await withinLimit(this.env.DB, `push:${daemon}`, PUSH_PER_HOUR, 3600_000))) return;

      const payload = approvalPayload(request);

      for (const sub of await listPush(this.env.DB, daemon)) {
        if ((await sendPush(sub, payload, vapid)) === 'gone') await dropPush(this.env.DB, daemon, sub.device);
      }
    } catch {
      // A failed push must never break the relay. The email that follows is the fallback.
    }
  }

  private async email(request: string): Promise<void> {
    const daemon = await this.daemonId();

    if (!daemon) return;

    try {
      const address = await emailOf(this.env.DB, daemon);

      if (!address) return void console.warn('email: no address is set for this computer');
      if (!(await withinLimit(this.env.DB, `email:${daemon}`, EMAIL_PER_HOUR, 3600_000))) return void console.warn('email: hourly limit reached');

      const result = await sendMail(this.env, approvalMail(address, `${this.env.WEB_ORIGIN}/a/${request}`));

      // Only the outcome is logged, never the address, the key or the provider's reply.
      if (!result.sent) console.warn(`email: not sent (${result.reason})`);
    } catch (error) {
      // Nothing to do for the person: the card is still on the computer and in the app. The log says why.
      console.warn(`email: failed (${(error as Error)?.name}: ${(error as Error)?.message})`);
    }
  }

  /** The owner typed an address on their own computer (the trust root), so it is the one to use. An empty address removes it. */
  private async setEmail(address: unknown): Promise<void> {
    const daemon = await this.daemonId();
    const pub = this.pinnedKey();

    if (!daemon || !pub) return;

    try {
      if (typeof address !== 'string' || address === '') return void (await dropEmail(this.env.DB, daemon));
      if (!validAddress(address)) return;

      await ensureDaemon(this.env.DB, daemon, pub);
      await saveEmail(this.env.DB, daemon, address);
    } catch {
      // D1 being unavailable must not break the relay.
    }
  }

  // --- sign in by link: the daemon makes the pairing offer ------------------------------------------------------

  /**
   * Called by the Worker once a sign-in link has been spent. Asks the daemon for a pairing offer. The browser then pairs the normal
   * way and the person still has to approve it on the computer, so the link by itself grants nothing.
   */
  async offer(): Promise<Offer> {
    const ticket = randomToken(12);

    return new Promise<Offer>((resolve) => {
      const timer = setTimeout(() => {
        this.offers.delete(ticket);
        resolve({ ok: false, why: 'offline' });
      }, OFFER_WAIT_MS);

      this.offers.set(ticket, (value) => {
        clearTimeout(timer);
        this.offers.delete(ticket);
        resolve(value);
      });

      if (!this.toDaemon({ t: 'magic', ticket })) {
        clearTimeout(timer);
        this.offers.delete(ticket);
        resolve({ ok: false, why: 'offline' });
      }
    });
  }

  private offered(message: any): void {
    const resolve = this.offers.get(String(message.ticket ?? ''));

    if (!resolve) return;

    const fragment = String(message.fragment ?? '');
    const fingerprint = String(message.fingerprint ?? '');
    const expiresAt = Number(message.expires_at);

    // The fragment is the same text the QR code carries; only that shape is passed on.
    if (message.error || !/^v1\.[A-Za-z0-9_.-]{20,700}$/.test(fragment) || fingerprint.length > 80 || !Number.isFinite(expiresAt)) {
      return resolve({ ok: false, why: 'refused' });
    }

    resolve({ ok: true, fragment, fingerprint, expiresAt });
  }

  private addDevice(device: any): void {
    const id = String(device?.id ?? '');
    const pub = String(device?.pub ?? '');

    if (!DEVICE_ID.test(id) || pub.length < 80 || pub.length > 100) return;
    this.ctx.storage.sql.exec(`INSERT OR REPLACE INTO devices (id, pub, guest) VALUES (?, ?, ?)`, id, pub, device.guest ? 1 : 0);
  }

  // --- browsers ---------------------------------------------------------------------------------------------

  private async deviceAuth(ws: WebSocket, attachment: Attachment, message: any): Promise<void> {
    const daemonId = attachment.daemonId;
    const row = this.ctx.storage.sql.exec(`SELECT pub FROM devices WHERE id = ?`, attachment.device ?? '').toArray()[0] as { pub: string } | undefined;

    // One answer for "unknown device" and "bad signature": a probe learns nothing about who is paired.
    if (message.t !== 'auth' || !row) return this.deny(ws, 'refused');

    let sig: Uint8Array;

    try {
      sig = b64uDecode(String(message.sig));
    } catch {
      return this.deny(ws, 'refused');
    }

    if (!(await verifySignature(b64uDecode(row.pub), deviceAuthMessage(attachment.nonce ?? '', daemonId, attachment.device ?? ''), sig))) {
      return this.deny(ws, 'refused');
    }

    attachment.phase = 'ready';
    ws.serializeAttachment(attachment);

    if (!this.toDaemon({ conn: attachment.conn, open: { pairing: false, device: attachment.device } })) return this.refuse(ws, 'offline');

    // The relay's clock, so a phone with a wrong clock still sends a hello the daemon accepts (it checks +-2 minutes).
    ws.send(JSON.stringify({ t: 'ready', now: Date.now() }));
  }

  private async fromDevice(ws: WebSocket, attachment: Attachment, message: any): Promise<void> {
    // A signed-in device may register or drop its own push subscription. That is the only thing the Hub itself answers.
    if (message.t === 'push.subscribe' || message.t === 'push.unsubscribe') return this.pushSettings(ws, attachment, message);
    if (message.t === 'vault.put') return this.putVault(ws, attachment, message);

    // Otherwise only the two kinds of end-to-end message pass; the Hub does not interpret them.
    if (message.t !== 'hello' && message.t !== 'f') return;
    if (!this.toDaemon({ conn: attachment.conn, msg: message })) this.refuse(ws, 'offline');
  }

  /** A signed-in device keeps its sealed key here. Only the device that first stored an id may replace it. */
  private putVault(ws: WebSocket, attachment: Attachment, message: any): void {
    const sql = this.ctx.storage.sql;
    const id = String(message.id ?? '');
    const blob = String(message.blob ?? '');
    const device = attachment.device ?? '';

    if (!VAULT_ID.test(id) || !VAULT_BLOB.test(blob) || blob.length > MAX_VAULT_CHARS) return void ws.send(JSON.stringify({ t: 'vault.refused', why: 'malformed' }));

    const existing = sql.exec(`SELECT device FROM vaults WHERE id = ?`, id).toArray()[0] as { device: string } | undefined;

    if (existing && existing.device !== device) return void ws.send(JSON.stringify({ t: 'vault.refused', why: 'taken' }));

    const count = (sql.exec(`SELECT count(*) AS n FROM vaults`).toArray()[0] as { n: number }).n;

    if (!existing && count >= MAX_VAULTS) return void ws.send(JSON.stringify({ t: 'vault.refused', why: 'full' }));

    // One vault per device: a device that sealed a new one (re-paired its passkey) does not leave the old one behind.
    sql.exec(`DELETE FROM vaults WHERE device = ? AND id != ?`, device, id);
    sql.exec(`INSERT OR REPLACE INTO vaults (id, device, blob, saved_at) VALUES (?, ?, ?, ?)`, id, device, blob, Date.now());
    ws.send(JSON.stringify({ t: 'vault.ok' }));
  }

  private async pushSettings(ws: WebSocket, attachment: Attachment, message: any): Promise<void> {
    const daemon = await this.daemonId();
    const pub = this.pinnedKey();
    const device = attachment.device ?? '';

    if (!daemon || !pub) return;

    try {
      if (message.t === 'push.unsubscribe') {
        await dropPush(this.env.DB, daemon, device);
        ws.send(JSON.stringify({ t: 'push.ok', subscribed: false }));

        return;
      }

      const sub = parseSubscription(message.subscription);

      if (!sub || !this.vapid()) {
        ws.send(JSON.stringify({ t: 'push.refused', why: sub ? 'push is not set up on this relay' : 'that is not a browser push subscription' }));

        return;
      }

      await ensureDaemon(this.env.DB, daemon, pub);
      await savePush(this.env.DB, daemon, device, sub);
      ws.send(JSON.stringify({ t: 'push.ok', subscribed: true }));
    } catch {
      ws.send(JSON.stringify({ t: 'push.refused', why: 'could not save it just now' }));
    }
  }

  private fromPair(ws: WebSocket, attachment: Attachment, message: any): void {
    attachment.sent = (attachment.sent ?? 0) + 1;
    ws.serializeAttachment(attachment);

    if (attachment.sent > MAX_PAIR_MESSAGES) return void ws.close(CLOSE.rate, 'a pairing socket sends one hello');

    // A browser that forgot everything asks for its sealed key by an id only its passkey can produce. The answer is ciphertext.
    if (message.t === 'vault.get') {
      const id = String(message.id ?? '');
      const row = VAULT_ID.test(id) ? (this.ctx.storage.sql.exec(`SELECT blob FROM vaults WHERE id = ?`, id).toArray()[0] as { blob: string } | undefined) : undefined;

      ws.send(JSON.stringify(row ? { t: 'vault.blob', blob: row.blob } : { t: 'vault.none' }));

      return;
    }

    if (message.t !== 'hello') return;
    if (!this.toDaemon({ conn: attachment.conn, msg: message })) this.refuse(ws, 'offline');
  }

  // --- plumbing -------------------------------------------------------------------------------------------------

  /** Sends to the daemon's socket. `false` when the daemon is not connected. */
  private toDaemon(value: unknown): boolean {
    for (const ws of this.ctx.getWebSockets('daemon')) {
      const info = ws.deserializeAttachment() as Attachment | null;

      if (info?.phase === 'ready') {
        ws.send(JSON.stringify(value));
        return true;
      }
    }

    return false;
  }

  private deny(ws: WebSocket, why: string): void {
    ws.send(JSON.stringify({ t: 'denied', why }));
    ws.close(CLOSE.denied, 'denied');
  }

  private refuse(ws: WebSocket, why: 'offline'): void {
    ws.send(JSON.stringify({ t: why }));
    ws.close(CLOSE.offline, 'the computer is offline');
  }

  private withinRate(ws: WebSocket, limit: number): boolean {
    const now = Date.now();
    const entry = this.rates.get(ws) ?? { windowStart: now, count: 0 };

    if (now - entry.windowStart >= 1000) {
      entry.windowStart = now;
      entry.count = 0;
    }

    entry.count += 1;
    this.rates.set(ws, entry);

    return entry.count <= limit;
  }
}
