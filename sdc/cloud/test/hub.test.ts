import { exports } from 'cloudflare:workers';
import { describe, expect, it } from 'vitest';
import { b64uEncode, daemonIdOf, hubAuthMessage } from '../src/util';
import { CLOSE, MAX_FROM_DEVICE, MAX_FROM_PAIR, MAX_PAIR_MESSAGES } from '../src/hub';
import { FakeBrowser, FakeDaemon, ORIGIN, newKeyPair, open, sign } from './helpers';

const fetchRoot = (path: string, init?: RequestInit) => exports.default.fetch(`https://sdc.skilleddesk.com${path}`, init);

describe('the Worker', () => {
  it('answers health checks without touching a Hub', async () => {
    const response = await fetchRoot('/api/health');

    expect(response.status).toBe(200);
    expect(await response.json()).toEqual({ ok: true });
  });

  it('serves the app with a strict content security policy', async () => {
    const response = await fetchRoot('/');
    const csp = response.headers.get('Content-Security-Policy') ?? '';

    expect(csp).toContain("default-src 'none'");
    expect(csp).toContain("script-src 'self'");
    expect(csp).toContain("frame-ancestors 'none'");
    expect(csp).toContain('connect-src \'self\' wss://sdc.skilleddesk.com');
    expect(csp).not.toContain('unsafe-eval');
    expect(response.headers.get('X-Content-Type-Options')).toBe('nosniff');
    expect(response.headers.get('Strict-Transport-Security')).toContain('max-age');
  });

  it('the static-assets _headers file carries exactly the same policy (assets are served without running the Worker)', async () => {
    const file = (await import('../../web/public/_headers?raw')).default as string;
    const block = file.split('\n\n')[0]!.split('\n').slice(1).map((line) => line.trim().split(/: (.*)/s).slice(0, 2));
    const fromFile = Object.fromEntries(block);
    const { securityHeaders } = await import('../src/worker');

    expect(fromFile).toEqual(securityHeaders({ WEB_ORIGIN: 'https://sdc.skilleddesk.com' }));
  });

  it('refuses a malformed id', async () => {
    const response = await fetchRoot('/d/not-an-id', { headers: { Upgrade: 'websocket' } });

    expect(response.status).toBe(400);
  });

  it('refuses a plain request to a socket route', async () => {
    const response = await fetchRoot('/d/AAAAAAAAAAAAAAAAAAAAAA');

    expect(response.status).toBe(426);
  });

  it('refuses a browser WebSocket from another origin', async () => {
    const { response } = await open('/p/AAAAAAAAAAAAAAAAAAAAAA', 'https://evil.example');

    expect(response.status).toBe(403);
  });

  it('refuses a browser WebSocket with no origin', async () => {
    const { response } = await open('/c/AAAAAAAAAAAAAAAAAAAAAA?device=device-one', null);

    expect(response.status).toBe(403);
  });

  it('does not expose other /api paths', async () => {
    expect((await fetchRoot('/api/anything-else')).status).toBe(404);
  });
});

describe('daemon authentication', () => {
  it('accepts a daemon whose key matches its id', async () => {
    const daemon = await FakeDaemon.connect();

    expect(daemon.id).toHaveLength(22);
  });

  it('denies a key that is not the one the id is the hash of', async () => {
    const real = await newKeyPair();
    const thief = await newKeyPair();
    const id = await daemonIdOf(real.publicBytes);
    const { socket } = await open(`/d/${id}`);
    const challenge = await socket!.next();

    socket!.send({ t: 'auth', pub: b64uEncode(thief.publicBytes), sig: await sign(thief, hubAuthMessage(challenge.nonce, id)) });

    expect((await socket!.next()).t).toBe('denied');
    expect(await socket!.closedWith()).toBe(CLOSE.denied);
  });

  it('denies a signature made over the wrong challenge', async () => {
    const key = await newKeyPair();
    const id = await daemonIdOf(key.publicBytes);
    const { socket } = await open(`/d/${id}`);

    await socket!.next();
    socket!.send({ t: 'auth', pub: b64uEncode(key.publicBytes), sig: await sign(key, hubAuthMessage('an old nonce', id)) });

    expect((await socket!.next()).t).toBe('denied');
  });

  it('denies a signature made for another daemon id', async () => {
    const key = await newKeyPair();
    const id = await daemonIdOf(key.publicBytes);
    const { socket } = await open(`/d/${id}`);
    const challenge = await socket!.next();

    socket!.send({ t: 'auth', pub: b64uEncode(key.publicBytes), sig: await sign(key, hubAuthMessage(challenge.nonce, 'BBBBBBBBBBBBBBBBBBBBBB')) });

    expect((await socket!.next()).t).toBe('denied');
  });

  it('closes a socket that never authenticates', async () => {
    const { socket } = await open('/d/AAAAAAAAAAAAAAAAAAAAAA');

    await socket!.next();
    socket!.send('this is not json');

    expect(await socket!.closedWith()).toBe(1003);
  });

  it('a second connection replaces the first', async () => {
    const identity = await newKeyPair();
    const first = await FakeDaemon.connect(identity);
    const second = await FakeDaemon.connect(identity);

    expect(await first.socket.closedWith()).toBe(CLOSE.replaced);
    expect(second.socket.closed).toBeNull();
  });
});

describe('a paired browser', () => {
  async function setup() {
    const daemon = await FakeDaemon.connect();
    const key = await newKeyPair();
    const browser = new FakeBrowser(daemon.id, 'phone-device-1', key);

    await daemon.teach([{ id: browser.deviceId, key }]);

    return { daemon, browser, key };
  }

  it('connects, and the daemon is told', async () => {
    const { daemon, browser } = await setup();
    const socket = await browser.connect(daemon);
    const opened = await daemon.socket.next();

    expect(opened.open).toEqual({ pairing: false, device: 'phone-device-1' });
    expect(typeof opened.conn).toBe('string');
    socket.close();
  });

  it('forwards end-to-end messages both ways and nothing else', async () => {
    const { daemon, browser } = await setup();
    const socket = await browser.connect(daemon);
    const { conn } = await daemon.socket.next();

    socket.send({ t: 'hello', device: 'phone-device-1', enc: 'AAAA', ct: 'BBBB', sig: 'CCCC' });

    expect(await daemon.socket.next()).toEqual({ conn, msg: { t: 'hello', device: 'phone-device-1', enc: 'AAAA', ct: 'BBBB', sig: 'CCCC' } });

    socket.send({ t: 'f', n: 0, ct: 'DDDD' });

    expect((await daemon.socket.next()).msg).toEqual({ t: 'f', n: 0, ct: 'DDDD' });

    // Anything that is not a hello or a frame is dropped, not forwarded.
    socket.send({ t: 'ctl', ctl: 'device.remove', id: 'phone-device-1' });
    socket.send({ ctl: 'notify', request: 'x' });
    daemon.socket.send({ ctl: 'ping' });

    expect((await daemon.socket.next()).t).toBe('pong');

    daemon.socket.send({ conn, msg: { t: 'welcome', enc: 'E', ct: 'F', sig: 'G' } });

    expect(await socket.next()).toEqual({ t: 'welcome', enc: 'E', ct: 'F', sig: 'G' });
  });

  it('is closed when the daemon closes its connection', async () => {
    const { daemon, browser } = await setup();
    const socket = await browser.connect(daemon);
    const { conn } = await daemon.socket.next();

    daemon.socket.send({ conn, close: true });

    expect(await socket.closedWith()).toBe(1000);
  });

  it('tells the daemon when the browser goes away', async () => {
    const { daemon, browser } = await setup();
    const socket = await browser.connect(daemon);
    const { conn } = await daemon.socket.next();

    socket.close();

    expect(await daemon.socket.next()).toEqual({ conn, close: true });
  });

  it('is cut off when the daemon disconnects', async () => {
    const { daemon, browser } = await setup();
    const socket = await browser.connect(daemon);

    await daemon.socket.next();
    daemon.socket.close();

    expect(await socket.closedWith()).toBe(CLOSE.daemonGone);
  });

  it('is cut off when the daemon removes the device', async () => {
    const { daemon, browser } = await setup();
    const socket = await browser.connect(daemon);

    await daemon.socket.next();
    daemon.socket.send({ ctl: 'device.remove', id: browser.deviceId });

    expect(await socket.closedWith()).toBe(CLOSE.denied);
  });

  it('cannot connect once removed', async () => {
    const { daemon, browser } = await setup();

    daemon.socket.send({ ctl: 'device.remove', id: browser.deviceId });
    daemon.socket.send({ ctl: 'ping' });
    await daemon.socket.next();

    await expect(browser.connect(daemon)).rejects.toThrow();
  });

  it('is told the computer is offline when no daemon is connected', async () => {
    const key = await newKeyPair();
    const idle = await newKeyPair();
    const id = await daemonIdOf(idle.publicBytes);

    // Teach a Hub a device without ever leaving a daemon connected: connect, teach, disconnect.
    const daemon = await FakeDaemon.connect(idle);

    await daemon.teach([{ id: 'phone-device-1', key }]);
    daemon.socket.close();
    await new Promise((resolve) => setTimeout(resolve, 100));

    const { socket } = await open(`/c/${id}?device=phone-device-1`, ORIGIN);
    const challenge = await socket!.next();

    socket!.send({ t: 'auth', sig: await sign(key, new TextEncoder().encode(`sdc-anywhere/v1/hub-device-auth|${challenge.nonce}|${id}|phone-device-1`)) });

    expect((await socket!.next()).t).toBe('offline');
    expect(await socket!.closedWith()).toBe(CLOSE.offline);
  });

  it('is denied with a bad signature, and the same answer as an unknown device', async () => {
    const { daemon, browser } = await setup();
    const impostor = new FakeBrowser(daemon.id, browser.deviceId, await newKeyPair());
    const stranger = new FakeBrowser(daemon.id, 'nobody-registered', await newKeyPair());

    await expect(impostor.connect(daemon)).rejects.toThrow(/denied/);
    await expect(stranger.connect(daemon)).rejects.toThrow(/denied/);
  });

  it('is denied when it signs another device id', async () => {
    const { daemon, browser, key } = await setup();
    const { socket } = await open(`/c/${daemon.id}?device=${browser.deviceId}`, ORIGIN);
    const challenge = await socket!.next();

    socket!.send({ t: 'auth', sig: await sign(key, new TextEncoder().encode(`sdc-anywhere/v1/hub-device-auth|${challenge.nonce}|${daemon.id}|someone-else`)) });

    expect((await socket!.next()).t).toBe('denied');
  });

  it('is closed for a message that is too big', async () => {
    const { daemon, browser } = await setup();
    const socket = await browser.connect(daemon);

    await daemon.socket.next();
    socket.send(JSON.stringify({ t: 'f', n: 0, ct: 'A'.repeat(MAX_FROM_DEVICE + 1) }));

    expect(await socket.closedWith()).toBe(CLOSE.tooBig);
  });

  it('is closed for a flood', async () => {
    const { daemon, browser } = await setup();
    const socket = await browser.connect(daemon);

    await daemon.socket.next();

    for (let i = 0; i < 400; i++) socket.send({ t: 'f', n: i, ct: 'AAAA' });

    expect(await socket.closedWith()).toBe(CLOSE.rate);
  });

  it('two browsers get separate connections', async () => {
    const { daemon, browser } = await setup();
    const keyTwo = await newKeyPair();
    const two = new FakeBrowser(daemon.id, 'laptop-device-2', keyTwo);

    await daemon.teach([
      { id: browser.deviceId, key: browser.key },
      { id: two.deviceId, key: keyTwo },
    ]);

    const a = await browser.connect(daemon);
    const b = await two.connect(daemon);
    const first = await daemon.socket.next();
    const second = await daemon.socket.next();

    expect(first.conn).not.toEqual(second.conn);

    daemon.socket.send({ conn: first.conn, msg: { t: 'f', n: 0, ct: 'ONLY-A' } });

    expect((await a.next()).ct).toBe('ONLY-A');
    expect(await b.quiet()).toBe(true);
  });
});

describe('abuse', () => {
  it('keeps nothing for a computer that never signed in, and tells the browser it is offline', async () => {
    const stranger = await newKeyPair();
    const id = await daemonIdOf(stranger.publicBytes);
    const { socket } = await open(`/p/${id}`, ORIGIN);

    expect((await socket!.next()).t).toBe('offline');
    expect(await socket!.closedWith()).toBe(CLOSE.offline);

    const device = await open(`/c/${id}?device=phone-device-1`, ORIGIN);

    expect((await device.socket!.next()).t).toBe('offline');
  });

  it('a daemon id cannot be claimed by a different key once one has signed in', async () => {
    const real = await newKeyPair();
    const first = await FakeDaemon.connect(real);
    const id = first.id;

    first.socket.close();
    await new Promise((resolve) => setTimeout(resolve, 100));

    // Another key that does not hash to this id cannot sign in as it, whatever it signs.
    const other = await newKeyPair();
    const { socket } = await open(`/d/${id}`);
    const challenge = await socket!.next();

    socket!.send({ t: 'auth', pub: b64uEncode(other.publicBytes), sig: await sign(other, hubAuthMessage(challenge.nonce, id)) });

    expect((await socket!.next()).t).toBe('denied');
  });
});

describe('pairing route', () => {
  it('reaches the daemon with no authentication, as a pairing connection', async () => {
    const daemon = await FakeDaemon.connect();
    const { socket } = await open(`/p/${daemon.id}`, ORIGIN);
    const opened = await daemon.socket.next();

    expect(opened.open).toEqual({ pairing: true });

    socket!.send({ t: 'hello', device: 'brand-new-device', enc: 'A', ct: 'B', sig: 'C' });

    expect((await daemon.socket.next()).msg.t).toBe('hello');

    daemon.socket.send({ conn: opened.conn, msg: { t: 'welcome', enc: 'x', ct: 'y', sig: 'z' } });

    expect((await socket!.next()).t).toBe('welcome');
  });

  it('forwards only hello, and only a few times', async () => {
    const daemon = await FakeDaemon.connect();
    const { socket } = await open(`/p/${daemon.id}`, ORIGIN);

    await daemon.socket.next();
    socket!.send({ t: 'f', n: 0, ct: 'A' });
    socket!.send({ ctl: 'device.add', id: 'sneaky-device', pub: 'x' });
    daemon.socket.send({ ctl: 'ping' });

    expect((await daemon.socket.next()).t).toBe('pong');

    for (let i = 0; i < MAX_PAIR_MESSAGES; i++) socket!.send({ t: 'hello', device: 'd', enc: 'A', ct: 'B', sig: 'C' });

    expect(await socket!.closedWith()).toBe(CLOSE.rate);
  });

  it('is told the computer is offline', async () => {
    const identity = await newKeyPair();
    const id = await daemonIdOf(identity.publicBytes);
    const { socket } = await open(`/p/${id}`, ORIGIN);

    expect((await socket!.next()).t).toBe('offline');
    expect(await socket!.closedWith()).toBe(CLOSE.offline);
  });

  it('cannot send a big message', async () => {
    const daemon = await FakeDaemon.connect();
    const { socket } = await open(`/p/${daemon.id}`, ORIGIN);

    await daemon.socket.next();
    socket!.send(JSON.stringify({ t: 'hello', ct: 'A'.repeat(MAX_FROM_PAIR + 1) }));

    expect(await socket!.closedWith()).toBe(CLOSE.tooBig);
  });
});

describe('control messages', () => {
  it('a daemon cannot be impersonated by a browser socket', async () => {
    const daemon = await FakeDaemon.connect();
    const key = await newKeyPair();
    const browser = new FakeBrowser(daemon.id, 'phone-device-1', key);

    await daemon.teach([{ id: browser.deviceId, key }]);

    const socket = await browser.connect(daemon);

    await daemon.socket.next();

    // A browser sends what only the daemon may send; the Hub ignores it.
    socket.send({ ctl: 'devices.sync', devices: [] });
    socket.send({ t: 'f', n: 0, ct: 'AAAA' });

    expect((await daemon.socket.next()).msg.t).toBe('f');

    // The device is still registered: a second browser connection with the same key still works.
    const again = await browser.connect(daemon);

    again.close();
  });
});
