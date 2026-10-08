// Phase 2: a request that needs the owner reaches them live, or by push, or (after a minute with no answer) by email.
// The network is replaced by a recorder, so no real push service or mail provider is contacted.

import { runInDurableObject } from 'cloudflare:test';
import { env } from 'cloudflare:workers';
import { afterEach, beforeAll, describe, expect, it } from 'vitest';
import { b64uDecode, b64uEncode } from '../src/util';
import { decrypt } from '../src/webpush';
import { FakeBrowser, FakeDaemon, captureFetch, eventually, migrate, newKeyPair, type Outgoing } from './helpers';

let net: ReturnType<typeof captureFetch>;

beforeAll(migrate);
afterEach(() => net?.restore());

/** A browser's push subscription: real keys, so the test can open what the relay encrypted for it. */
async function subscriber(endpoint = `https://fcm.googleapis.com/fcm/send/${crypto.randomUUID()}`) {
  const pair = (await crypto.subtle.generateKey({ name: 'ECDH', namedCurve: 'P-256' }, true, ['deriveBits'])) as CryptoKeyPair;
  const publicRaw = new Uint8Array((await crypto.subtle.exportKey('raw', pair.publicKey)) as ArrayBuffer);
  const jwk = (await crypto.subtle.exportKey('jwk', pair.privateKey)) as JsonWebKey;
  const auth = crypto.getRandomValues(new Uint8Array(16));

  return {
    subscription: { endpoint, p256dh: b64uEncode(publicRaw), auth: b64uEncode(auth) },
    open: (body: Uint8Array) => decrypt(body, b64uDecode(jwk.d!), publicRaw, auth).then((bytes) => JSON.parse(new TextDecoder().decode(bytes))),
  };
}

async function setup(options: { address?: string; connect?: boolean } = {}) {
  const daemon = await FakeDaemon.connect();
  const key = await newKeyPair();
  const browser = new FakeBrowser(daemon.id, 'phone-device-1', key);

  await daemon.teach([{ id: browser.deviceId, key }]);

  if (options.address) {
    daemon.socket.send({ ctl: 'email.set', address: options.address });
    await eventually('the address to be saved', async () => (await env.DB.prepare(`SELECT address FROM emails WHERE daemon_id = ?`).bind(daemon.id).first()) ?? undefined);
  }

  // The browser connects, subscribes to push, then (unless the test wants it away) stays or goes.
  const socket = await browser.connect(daemon);

  await daemon.socket.next();

  return { daemon, browser, socket, connected: options.connect ?? false };
}

async function subscribe(socket: Awaited<ReturnType<typeof setup>>['socket'], sub: { endpoint: string; p256dh: string; auth: string }) {
  socket.send({ t: 'push.subscribe', subscription: sub });

  return socket.next();
}

const pushes = (sent: Outgoing[]) => sent.filter((item) => item.url.startsWith('https://fcm.googleapis.com'));
const mails = (sent: Outgoing[]) => sent.filter((item) => item.url === 'https://mail.example.test/send');

describe('push subscriptions', () => {
  it('a signed-in browser can register its subscription, and only a real push service is accepted', async () => {
    const { socket, daemon } = await setup();
    const good = await subscriber();

    expect(await subscribe(socket, good.subscription)).toEqual({ t: 'push.ok', subscribed: true });

    const row = await env.DB.prepare(`SELECT endpoint, device_id FROM push_subscriptions WHERE daemon_id = ?`).bind(daemon.id).first<{ endpoint: string; device_id: string }>();

    expect(row).toEqual({ endpoint: good.subscription.endpoint, device_id: 'phone-device-1' });

    // Not a push service: the relay must never be made to POST to an address a browser names.
    for (const endpoint of ['https://evil.example/collect', 'http://fcm.googleapis.com/x', 'https://fcm.googleapis.com.evil.example/x', 'https://user:pw@fcm.googleapis.com/x', 'https://127.0.0.1/x']) {
      const answer = await subscribe(socket, { ...good.subscription, endpoint });

      expect(answer.t, endpoint).toBe('push.refused');
    }

    // Keys of the wrong shape.
    expect((await subscribe(socket, { ...good.subscription, p256dh: 'AAAA' })).t).toBe('push.refused');
    expect((await subscribe(socket, { ...good.subscription, auth: 'AAAA' })).t).toBe('push.refused');
    expect((await subscribe(socket, 'nonsense' as any)).t).toBe('push.refused');
  });

  it('unsubscribing removes it', async () => {
    const { socket, daemon } = await setup();

    await subscribe(socket, (await subscriber()).subscription);
    socket.send({ t: 'push.unsubscribe' });

    expect(await socket.next()).toEqual({ t: 'push.ok', subscribed: false });
    expect(await env.DB.prepare(`SELECT 1 FROM push_subscriptions WHERE daemon_id = ?`).bind(daemon.id).first()).toBeNull();
  });

  it('removing a device on the computer stops its pushes', async () => {
    const { socket, daemon, browser } = await setup();

    await subscribe(socket, (await subscriber()).subscription);
    daemon.socket.send({ ctl: 'device.remove', id: browser.deviceId });
    await eventually('the subscription to be dropped', async () => (await env.DB.prepare(`SELECT 1 FROM push_subscriptions WHERE daemon_id = ?`).bind(daemon.id).first()) === null || undefined);
  });

  it('is stored under the computer and the device that signed in, whatever the message says', async () => {
    const one = await setup();
    const two = await setup();
    const sub = (await subscriber()).subscription;

    await subscribe(one.socket, { ...sub, device: 'someone-else', daemon: two.daemon.id } as any);

    const rows = await env.DB.prepare(`SELECT daemon_id, device_id FROM push_subscriptions WHERE endpoint = ?`).bind(sub.endpoint).all<{ daemon_id: string; device_id: string }>();

    expect(rows.results).toEqual([{ daemon_id: one.daemon.id, device_id: 'phone-device-1' }]);
  });
});

describe('push on a request', () => {
  it('sends a push at once when no browser is open, and the push says only that something needs you', async () => {
    net = captureFetch();

    const { daemon, socket } = await setup();
    const phone = await subscriber();

    await subscribe(socket, phone.subscription);
    socket.close();
    await new Promise((resolve) => setTimeout(resolve, 80));

    daemon.socket.send({ ctl: 'notify', request: 'apr_perm-turn-7-1', expires_at: Date.now() + 60_000 });

    const sent = await eventually('a push', () => pushes(net.sent)[0]);
    const message = await phone.open(sent.body as Uint8Array);

    expect(message).toEqual({ t: 'approval', url: '/a/apr_perm-turn-7-1' });
    expect(sent.headers['content-encoding']).toBe('aes128gcm');
    expect(sent.headers.authorization).toMatch(/^vapid t=.+, k=BKvi_/);
    expect(sent.headers.urgency).toBe('high');
    // Nothing else was sent: no email yet.
    expect(mails(net.sent)).toHaveLength(0);
  });

  it('does not push when a browser is connected and will show the card itself', async () => {
    net = captureFetch();

    const { daemon, socket } = await setup();

    await subscribe(socket, (await subscriber()).subscription);
    daemon.socket.send({ ctl: 'notify', request: 'apr_live', expires_at: Date.now() + 60_000 });
    daemon.socket.send({ ctl: 'ping' });
    await daemon.socket.next();
    await new Promise((resolve) => setTimeout(resolve, 200));

    expect(pushes(net.sent)).toHaveLength(0);
  });

  it('forgets a subscription the browser has dropped (410)', async () => {
    net = captureFetch(() => 410);

    const { daemon, socket } = await setup();

    await subscribe(socket, (await subscriber()).subscription);
    socket.close();
    await new Promise((resolve) => setTimeout(resolve, 80));
    daemon.socket.send({ ctl: 'notify', request: 'apr_gone', expires_at: Date.now() + 60_000 });

    await eventually('the push attempt', () => pushes(net.sent)[0]);
    await eventually('the subscription to be forgotten', async () => (await env.DB.prepare(`SELECT 1 FROM push_subscriptions WHERE daemon_id = ?`).bind(daemon.id).first()) === null || undefined);
  });

  it('survives a push service that is down: no crash, and the email still follows', async () => {
    net = captureFetch((url) => (url.startsWith('https://fcm') ? 503 : 200));

    const { daemon, socket } = await setup({ address: 'owner@example.test' });

    await subscribe(socket, (await subscriber()).subscription);
    socket.close();
    await new Promise((resolve) => setTimeout(resolve, 80));
    daemon.socket.send({ ctl: 'notify', request: 'apr_down', expires_at: Date.now() + 60_000, escalate_after_sec: 1 });

    await eventually('the email', () => mails(net.sent)[0], 5000);
  });
});

describe('what the Hub keeps while a request waits', () => {
  const rows = (daemonId: string) =>
    runInDurableObject(env.HUB.get(env.HUB.idFromName(daemonId)), (_hub, state) => state.storage.sql.exec(`SELECT request, pushed_at, emailed_at, escalate_at FROM pending ORDER BY created_at`).toArray() as Array<Record<string, any>>);

  it('holds at most a hundred, the newest', async () => {
    net = captureFetch();

    const { daemon } = await setup();

    for (let n = 0; n < 130; n++) daemon.socket.send({ ctl: 'notify', request: `apr_flood-${n}`, expires_at: Date.now() + 600_000 });
    daemon.socket.send({ ctl: 'ping' });
    await eventually('the Hub to catch up', async () => (await rows(daemon.id)).length >= 100 || undefined);
    await new Promise((resolve) => setTimeout(resolve, 300));

    const kept = await rows(daemon.id);

    expect(kept.length).toBeLessThanOrEqual(100);
    expect(kept.map((row) => row.request)).toContain('apr_flood-129');
  });

  it('keeps the first push and email times when the same request is announced again', async () => {
    net = captureFetch();

    const { daemon, socket } = await setup();

    socket.close();
    await new Promise((resolve) => setTimeout(resolve, 80));
    daemon.socket.send({ ctl: 'notify', request: 'apr_again', expires_at: Date.now() + 60_000, escalate_after_sec: 30 });
    await eventually('the row', async () => ((await rows(daemon.id)).length === 1 ? true : undefined));

    const first = (await rows(daemon.id))[0]!;

    daemon.socket.send({ ctl: 'notify', request: 'apr_again', expires_at: Date.now() + 90_000, escalate_after_sec: 5 });
    await new Promise((resolve) => setTimeout(resolve, 200));

    const second = (await rows(daemon.id))[0]!;

    expect(second.escalate_at).toBe(first.escalate_at);
    expect((await rows(daemon.id)).length).toBe(1);
  });

  it('clears the alarm when nothing is waiting', async () => {
    net = captureFetch();

    const { daemon, socket } = await setup();

    socket.close();
    daemon.socket.send({ ctl: 'notify', request: 'apr_gone-soon', expires_at: Date.now() + 60_000, escalate_after_sec: 600 });
    await eventually('the row', async () => ((await rows(daemon.id)).length === 1 ? true : undefined));
    expect(await runInDurableObject(env.HUB.get(env.HUB.idFromName(daemon.id)), (_hub, state) => state.storage.getAlarm())).not.toBeNull();

    daemon.socket.send({ ctl: 'clear', request: 'apr_gone-soon' });
    await eventually('the alarm to be cleared', async () => (await runInDurableObject(env.HUB.get(env.HUB.idFromName(daemon.id)), (_hub, state) => state.storage.getAlarm())) === null || undefined);
  });
});

describe('email after no answer', () => {
  it('sends one email after the delay, with a link and no details', async () => {
    net = captureFetch((url) => (url.startsWith('https://mail') ? 200 : 201));

    const { daemon, socket } = await setup({ address: 'Owner@Example.Test' });

    socket.close();
    daemon.socket.send({ ctl: 'notify', request: 'apr_perm-turn-9-2', expires_at: Date.now() + 60_000, escalate_after_sec: 1 });

    // Not before its time.
    await new Promise((resolve) => setTimeout(resolve, 300));
    expect(mails(net.sent)).toHaveLength(0);

    const mail = await eventually('the email', () => mails(net.sent)[0], 5000);
    const body = JSON.parse(mail.body as string);

    expect(body.to).toEqual(['owner@example.test']);
    expect(body.from).toBe('SDC <notify@example.test>');
    expect(body.text).toContain('https://sdc.skilleddesk.com/a/apr_perm-turn-9-2');
    expect(body.html).toContain('href="https://sdc.skilleddesk.com/a/apr_perm-turn-9-2"');
    // The provider key goes in one header, and nowhere else.
    expect(mail.headers.authorization).toBe('Bearer test-key-not-a-real-key');
    expect(mail.body as string).not.toContain('test-key-not-a-real-key');

    // Once only, even when the daemon repeats itself.
    daemon.socket.send({ ctl: 'notify', request: 'apr_perm-turn-9-2', expires_at: Date.now() + 60_000, escalate_after_sec: 1 });
    await new Promise((resolve) => setTimeout(resolve, 1500));
    expect(mails(net.sent)).toHaveLength(1);
  });

  it('sends nothing when the request was answered in time', async () => {
    net = captureFetch();

    const { daemon, socket } = await setup({ address: 'owner@example.test' });

    socket.close();
    daemon.socket.send({ ctl: 'notify', request: 'apr_answered', expires_at: Date.now() + 60_000, escalate_after_sec: 1 });
    await new Promise((resolve) => setTimeout(resolve, 200));
    daemon.socket.send({ ctl: 'clear', request: 'apr_answered' });
    await new Promise((resolve) => setTimeout(resolve, 1600));

    expect(mails(net.sent)).toHaveLength(0);
  });

  it('sends nothing for a request that expired, or when no address was ever given', async () => {
    net = captureFetch();

    const withAddress = await setup({ address: 'owner@example.test' });
    const without = await setup();

    withAddress.daemon.socket.send({ ctl: 'notify', request: 'apr_stale', expires_at: Date.now() + 300, escalate_after_sec: 1 });
    without.daemon.socket.send({ ctl: 'notify', request: 'apr_nobody', expires_at: Date.now() + 60_000, escalate_after_sec: 1 });
    await new Promise((resolve) => setTimeout(resolve, 1800));

    expect(mails(net.sent)).toHaveLength(0);
  });

  it('survives a mail provider that refuses', async () => {
    net = captureFetch((url) => (url.startsWith('https://mail') ? 500 : 201));

    const { daemon, socket } = await setup({ address: 'owner@example.test' });

    socket.close();
    daemon.socket.send({ ctl: 'notify', request: 'apr_refused', expires_at: Date.now() + 60_000, escalate_after_sec: 1 });
    await eventually('the attempt', () => mails(net.sent)[0], 5000);
    // The Hub is still serving.
    daemon.socket.send({ ctl: 'ping' });

    const seen: string[] = [];

    for (let n = 0; n < 4 && !seen.includes('pong'); n++) seen.push((await daemon.socket.next()).t ?? 'other');

    expect(seen).toContain('pong');
  });

  it('refuses a request id that could not be put safely in a link', async () => {
    net = captureFetch();

    const { daemon } = await setup({ address: 'owner@example.test' });

    daemon.socket.send({ ctl: 'notify', request: 'apr_x/../../evil?x=1#', expires_at: Date.now() + 60_000, escalate_after_sec: 1 });
    daemon.socket.send({ ctl: 'notify', request: 'x'.repeat(200), expires_at: Date.now() + 60_000, escalate_after_sec: 1 });
    await new Promise((resolve) => setTimeout(resolve, 1600));

    expect(mails(net.sent)).toHaveLength(0);
  });

  it('only the computer may set the address, and a bad one is ignored', async () => {
    const { daemon, socket } = await setup();

    socket.send({ ctl: 'email.set', address: 'attacker@example.test' });
    daemon.socket.send({ ctl: 'email.set', address: 'not an address' });
    daemon.socket.send({ ctl: 'email.set', address: 'a@b.test\r\nBcc: x@y.test' });
    daemon.socket.send({ ctl: 'ping' });
    await daemon.socket.next();
    await new Promise((resolve) => setTimeout(resolve, 150));

    expect(await env.DB.prepare(`SELECT 1 FROM emails WHERE daemon_id = ?`).bind(daemon.id).first()).toBeNull();

    daemon.socket.send({ ctl: 'email.set', address: 'ok@example.test' });
    await eventually('the address', async () => (await env.DB.prepare(`SELECT address FROM emails WHERE daemon_id = ?`).bind(daemon.id).first()) ?? undefined);
    daemon.socket.send({ ctl: 'email.set', address: '' });
    await eventually('the address to be removed', async () => (await env.DB.prepare(`SELECT 1 FROM emails WHERE daemon_id = ?`).bind(daemon.id).first()) === null || undefined);
  });
});
