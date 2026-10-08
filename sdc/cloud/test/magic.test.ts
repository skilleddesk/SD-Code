// Sign in by email link (plan 5.1, threat T5): a link is single use, lives ten minutes, is spent by a button and never by opening
// it, and by itself grants nothing: the computer is asked for a pairing offer that the owner still has to approve.

import { env, exports } from 'cloudflare:workers';
import { afterEach, beforeAll, beforeEach, describe, expect, it } from 'vitest';
import { MAGIC_PER_DAEMON_PER_HOUR, createMagic, spendMagic, withinLimit } from '../src/store';
import { FakeDaemon, ORIGIN, captureFetch, eventually, migrate } from './helpers';

let net: ReturnType<typeof captureFetch>;

/** Addresses are unique per test (the test database lives for the whole run), so one test never mails another test's computer. */
let run = '';
const addr = (name: string) => `${name}-${run}@example.test`;

beforeAll(migrate);
beforeEach(() => void (run = crypto.randomUUID().slice(0, 8)));
afterEach(() => net?.restore());

/** Each call comes from a fresh address unless the test names one, so the per-address limit does not couple tests together. */
const post = (path: string, body: unknown, origin: string | null = ORIGIN, ip: string = `10.${Math.floor(Math.random() * 250)}.${Math.floor(Math.random() * 250)}.${Math.floor(Math.random() * 250)}`) =>
  exports.default.fetch(`https://sdc.skilleddesk.com${path}`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', 'CF-Connecting-IP': ip, ...(origin ? { Origin: origin } : {}) },
    body: typeof body === 'string' ? body : JSON.stringify(body),
  });

const mails = () => net.sent.filter((item) => item.url === 'https://mail.example.test/send');

const FRAGMENT = `v1.${'A'.repeat(43)}.${'B'.repeat(87)}.${'C'.repeat(43)}`;

type Behaviour = 'offer' | 'junk' | 'silent';

/** The computer answers every `magic` it is sent, the way sdcd does (or badly, or not at all). Runs until the socket closes. */
function answering(daemon: FakeDaemon, behaviour: Behaviour, asked: any[]): void {
  void (async () => {
    while (!daemon.socket.closed) {
      let message: any;

      try {
        message = await daemon.socket.next();
      } catch {
        continue;
      }

      if (message.t !== 'magic') continue;
      asked.push(message);

      if (behaviour === 'offer') daemon.socket.send({ ctl: 'offer', ticket: message.ticket, fragment: FRAGMENT, fingerprint: 'ABCD EFGH IJKL MNOP', expires_at: Date.now() + 600_000 });
      if (behaviour === 'junk') daemon.socket.send({ ctl: 'offer', ticket: message.ticket, fragment: '<script>alert(1)</script>', fingerprint: 'x', expires_at: Date.now() + 1 });
    }
  })();
}

/** A computer that has told the relay its address. */
async function computer(address: string, behaviour: Behaviour = 'offer') {
  const daemon = await FakeDaemon.connect();
  const asked: any[] = [];

  daemon.socket.send({ ctl: 'email.set', address });
  await eventually('the address', async () => (await env.DB.prepare(`SELECT 1 FROM emails WHERE daemon_id = ?`).bind(daemon.id).first()) ?? undefined);
  answering(daemon, behaviour, asked);

  return { daemon, asked };
}

/** Asks for a link and returns the daemon id and token from the mail that went out. */
async function linkFor(address: string): Promise<{ daemon: string; token: string; mail: any }> {
  const before = mails().length;
  const response = await post('/api/magic/request', { email: address });

  expect(response.status).toBe(202);

  const mail = await eventually('the mail', () => mails()[before], 4000);
  const body = JSON.parse(mail.body as string);
  const link = /https:\/\/sdc\.skilleddesk\.com\/m#([A-Za-z0-9_-]{22})\.([A-Za-z0-9_-]+)/.exec(body.text)!;

  return { daemon: link[1]!, token: link[2]!, mail: body };
}

describe('asking for a link', () => {
  it('mails a link to a known address; the secret is in the fragment, so a scanner fetching it gets nothing', async () => {
    net = captureFetch((url) => (url.startsWith('https://mail') ? 200 : 201));

    const { daemon } = await computer(addr('owner'));
    const { daemon: id, token, mail } = await linkFor(addr('owner'));

    expect(id).toBe(daemon.id);
    expect(token.length).toBeGreaterThanOrEqual(40);
    expect(mail.to).toEqual([addr('owner')]);
    expect(mail.subject).toBe('Your SDC sign-in link');
    expect(mail.text).toContain('10 minutes');
    expect(mail.html).toContain('href="https://sdc.skilleddesk.com/m#');
    // Only a hash is kept: a copy of the database holds no usable link.
    const rows = await env.DB.prepare(`SELECT token_hash FROM magic_links WHERE daemon_id = ?`).bind(daemon.id).all<{ token_hash: string }>();

    expect(rows.results).toHaveLength(1);
    expect(JSON.stringify(rows.results)).not.toContain(token);
  });

  it('gives the same answer for an unknown address and sends nothing', async () => {
    net = captureFetch((url) => (url.startsWith('https://mail') ? 200 : 201));

    await computer(addr('known'));

    const known = await post('/api/magic/request', { email: addr('known') });
    const unknown = await post('/api/magic/request', { email: addr('nobody-here') });

    expect(unknown.status).toBe(known.status);
    expect(await unknown.json()).toEqual(await known.json());
    await eventually('exactly one mail', () => mails().length === 1 || undefined);
    await new Promise((resolve) => setTimeout(resolve, 300));
    expect(mails().map((mail) => JSON.parse(mail.body as string).to)).toEqual([[addr('known')]]);
  });

  it('refuses a bad address, a request from another site, and anything but POST', async () => {
    net = captureFetch();

    expect((await post('/api/magic/request', { email: 'not an address' })).status).toBe(400);
    expect((await post('/api/magic/request', { email: 'a@b.test\r\nBcc: x@y.test' })).status).toBe(400);
    expect((await post('/api/magic/request', { email: 5 })).status).toBe(400);
    expect((await post('/api/magic/request', 'not json')).status).toBe(400);
    expect((await post('/api/magic/request', { email: 'a@b.test', pad: 'x'.repeat(5000) })).status).toBe(400);
    expect((await post('/api/magic/request', { email: 'a@b.test' }, 'https://evil.example')).status).toBe(403);
    expect((await post('/api/magic/request', { email: 'a@b.test' }, null)).status).toBe(403);
    expect((await exports.default.fetch('https://sdc.skilleddesk.com/api/magic/request')).status).toBe(405);
    expect((await exports.default.fetch('https://sdc.skilleddesk.com/api/magic/redeem')).status).toBe(405);
    expect(mails()).toHaveLength(0);
  });

  it('slows one address down when it asks over and over', async () => {
    net = captureFetch();

    const statuses: number[] = [];

    for (let n = 0; n < 14; n++) statuses.push((await post('/api/magic/request', { email: addr('nobody') }, ORIGIN, '203.0.113.9')).status);

    expect(statuses.slice(0, 10).every((status) => status === 202)).toBe(true);
    expect(statuses.slice(10).every((status) => status === 429)).toBe(true);
  });

  it('stops after a few links an hour for one computer, and says nothing different', async () => {
    net = captureFetch((url) => (url.startsWith('https://mail') ? 200 : 201));

    await computer(addr('busy'));

    for (let n = 0; n < MAGIC_PER_DAEMON_PER_HOUR + 3; n++) expect((await post('/api/magic/request', { email: addr('busy') })).status).toBe(202);

    await eventually('the allowed mails', () => mails().length === MAGIC_PER_DAEMON_PER_HOUR || undefined, 6000);
    await new Promise((resolve) => setTimeout(resolve, 400));
    expect(mails()).toHaveLength(MAGIC_PER_DAEMON_PER_HOUR);
  });
});

describe('using a link', () => {
  it('opening it (GET) spends nothing; the button (POST) spends it once, and returns the pairing offer', async () => {
    net = captureFetch((url) => (url.startsWith('https://mail') ? 200 : 201));

    const { asked } = await computer(addr('owner'));
    const { daemon, token } = await linkFor(addr('owner'));

    // A mail scanner (or the person) opening the link: the page, and no change.
    for (const path of [`/m`, `/m/${token}`, `/m?t=${token}`]) {
      const page = await exports.default.fetch(`https://sdc.skilleddesk.com${path}`);

      expect(page.status, path).toBeLessThan(500);
    }

    const row = await env.DB.prepare(`SELECT used_at FROM magic_links WHERE daemon_id = ?`).bind(daemon).first<{ used_at: number | null }>();

    expect(row?.used_at).toBeNull();

    const spent = await post('/api/magic/redeem', { daemon, token });
    const offer: any = await spent.json();

    expect(spent.status).toBe(200);
    expect(offer).toMatchObject({ daemon, fragment: FRAGMENT, fingerprint: 'ABCD EFGH IJKL MNOP' });
    expect(asked).toHaveLength(1);

    // A second press, from anyone, gets nothing and the computer is not asked again.
    const again = await post('/api/magic/redeem', { daemon, token });

    expect(again.status).toBe(400);
    expect(await again.json()).toEqual({ error: 'link_invalid' });
    expect(asked).toHaveLength(1);
  });

  it('two presses at the same moment: exactly one wins', async () => {
    net = captureFetch((url) => (url.startsWith('https://mail') ? 200 : 201));

    const { asked } = await computer(addr('owner'));
    const { daemon, token } = await linkFor(addr('owner'));
    const answers = await Promise.all(Array.from({ length: 6 }, () => post('/api/magic/redeem', { daemon, token })));
    const statuses = answers.map((answer) => answer.status).sort();

    expect(statuses.filter((status) => status === 200)).toHaveLength(1);
    expect(statuses.filter((status) => status === 400)).toHaveLength(5);
    expect(asked).toHaveLength(1);
  });

  it('a link expires after ten minutes, and a made-up or other computer\'s token fails alike', async () => {
    net = captureFetch();

    const one = await computer(addr('one'));
    const two = await computer(addr('two'));
    const old = await createMagic(env.DB, one.daemon.id, Date.now() - 11 * 60_000);
    const fresh = await createMagic(env.DB, one.daemon.id);

    expect((await post('/api/magic/redeem', { daemon: one.daemon.id, token: old })).status).toBe(400);
    expect((await post('/api/magic/redeem', { daemon: one.daemon.id, token: 'x'.repeat(43) })).status).toBe(400);
    expect((await post('/api/magic/redeem', { daemon: one.daemon.id, token: '' })).status).toBe(400);
    expect((await post('/api/magic/redeem', { daemon: 'short', token: fresh })).status).toBe(400);
    // Made for one computer, offered to another: refused, and the real owner can still use it.
    expect((await post('/api/magic/redeem', { daemon: two.daemon.id, token: fresh })).status).toBe(400);
    expect(await spendMagic(env.DB, one.daemon.id, fresh)).toBe(true);
    expect(two.asked).toHaveLength(0);
  });

  it('refuses a press from another site', async () => {
    net = captureFetch();

    const { daemon } = await computer(addr('owner'));
    const token = await createMagic(env.DB, daemon.id);

    expect((await post('/api/magic/redeem', { daemon: daemon.id, token }, 'https://evil.example')).status).toBe(403);
    expect((await post('/api/magic/redeem', { daemon: daemon.id, token }, null)).status).toBe(403);
    expect(await spendMagic(env.DB, daemon.id, token)).toBe(true);
  });

  it('with the computer off, says so and gives the link back for later', async () => {
    net = captureFetch();

    const { daemon } = await computer(addr('owner'));
    const token = await createMagic(env.DB, daemon.id);

    daemon.socket.close();
    await new Promise((resolve) => setTimeout(resolve, 150));

    const refused = await post('/api/magic/redeem', { daemon: daemon.id, token });

    expect(refused.status).toBe(503);
    expect(await refused.json()).toEqual({ error: 'offline' });

    // Back on, the same link works.
    const back = await FakeDaemon.connect(daemon.identity);
    const asked: any[] = [];

    answering(back, 'offer', asked);

    expect(await env.DB.prepare(`SELECT used_at FROM magic_links WHERE daemon_id = ?`).bind(daemon.id).first()).toEqual({ used_at: null });
    expect((await post('/api/magic/redeem', { daemon: daemon.id, token })).status).toBe(200);
    expect(asked).toHaveLength(1);
  });

  it('refuses an offer that is not a pairing fragment, and gives the link back', async () => {
    net = captureFetch();

    const { daemon } = await computer(addr('owner'), 'junk');
    const token = await createMagic(env.DB, daemon.id);
    const answer = await post('/api/magic/redeem', { daemon: daemon.id, token });

    expect(answer.status).toBe(502);
    expect(JSON.stringify(await answer.json())).not.toContain('script');
    expect(await env.DB.prepare(`SELECT used_at FROM magic_links WHERE daemon_id = ?`).bind(daemon.id).first()).toEqual({ used_at: null });
  });

  it('a computer that never answers does not hang the request for long', async () => {
    net = captureFetch();

    const { daemon } = await computer(addr('owner'), 'silent');
    const token = await createMagic(env.DB, daemon.id);
    const started = Date.now();
    const answer = await post('/api/magic/redeem', { daemon: daemon.id, token });

    expect(answer.status).toBe(503);
    expect(Date.now() - started).toBeLessThan(10_000);
  }, 15_000);

});

describe('the rate limit counter', () => {
  it('allows up to the limit in a window, then refuses, then starts again', async () => {
    const bucket = `test:${crypto.randomUUID()}`;
    const results: boolean[] = [];

    for (let n = 0; n < 5; n++) results.push(await withinLimit(env.DB, bucket, 3, 1000, 1_000_000));

    expect(results).toEqual([true, true, true, false, false]);
    expect(await withinLimit(env.DB, bucket, 3, 1000, 1_000_000 + 1000)).toBe(true);
    expect(await withinLimit(env.DB, bucket, 3, 1000, 1_000_000 + 1001)).toBe(true);
  });
});


