// The relay keeps one sealed blob per paired browser so a passkey can bring the pairing back after the browser's data was cleared.
// It is ciphertext the relay cannot read; what is tested here is who may store it, who may fetch it, and when it goes away.

import { describe, expect, it } from 'vitest';
import { CLOSE, MAX_VAULT_CHARS } from '../src/hub';
import { FakeBrowser, FakeDaemon, ORIGIN, newKeyPair, open } from './helpers';

/** A pairing socket from a fresh address each time: the route is rate limited per address, and every test here opens several. */
const pairing = (daemon: FakeDaemon) => open(`/p/${daemon.id}`, ORIGIN, { 'CF-Connecting-IP': `10.9.${Math.floor(Math.random() * 250)}.${Math.floor(Math.random() * 250)}` });

const ID = 'A'.repeat(22);
const BLOB = 'B'.repeat(200);

async function setup(deviceId = 'phone-device-1') {
  const daemon = await FakeDaemon.connect();
  const key = await newKeyPair();
  const browser = new FakeBrowser(daemon.id, deviceId, key);

  await daemon.teach([{ id: deviceId, key }]);

  return { daemon, browser, key };
}

async function put(browser: FakeBrowser, daemon: FakeDaemon, message: unknown) {
  const socket = await browser.connect(daemon);

  await daemon.socket.until((m) => m.open?.device === browser.deviceId);
  socket.send(message);

  return { socket, answer: await socket.next() };
}

async function get(daemon: FakeDaemon, id: unknown) {
  const { socket } = await pairing(daemon);

  await daemon.socket.until((m) => m.open?.pairing === true);
  socket!.send({ t: 'vault.get', id });

  return socket!.next();
}

describe('storing a vault', () => {
  it('a signed-in browser stores one, and anyone who knows its id can fetch the ciphertext', async () => {
    const { daemon, browser } = await setup();

    expect((await put(browser, daemon, { t: 'vault.put', id: ID, blob: BLOB })).answer).toEqual({ t: 'vault.ok' });
    expect(await get(daemon, ID)).toEqual({ t: 'vault.blob', blob: BLOB });
  });

  it('an id nobody stored, or a malformed one, finds nothing and says the same', async () => {
    const { daemon } = await setup();

    expect(await get(daemon, 'Z'.repeat(22))).toEqual({ t: 'vault.none' });
    expect(await get(daemon, 'short')).toEqual({ t: 'vault.none' });
    expect(await get(daemon, 5)).toEqual({ t: 'vault.none' });
    expect(await get(daemon, "' OR 1=1 --")).toEqual({ t: 'vault.none' });
  });

  it('refuses a malformed id or blob, and a blob that is too big', async () => {
    const { daemon, browser } = await setup();
    const socket = await browser.connect(daemon);

    await daemon.socket.until((m) => m.open?.device === browser.deviceId);

    for (const bad of [
      { t: 'vault.put', id: 'short', blob: BLOB },
      { t: 'vault.put', id: ID, blob: 'not base64url!' },
      { t: 'vault.put', id: ID, blob: '' },
      { t: 'vault.put', id: ID, blob: 'C'.repeat(MAX_VAULT_CHARS + 1) },
      { t: 'vault.put', id: ID },
    ]) {
      socket.send(bad);
      expect((await socket.next()).t).toBe('vault.refused');
    }

    expect(await get(daemon, ID)).toEqual({ t: 'vault.none' });
  });

  it('only the device that stored an id may replace it', async () => {
    const { daemon, browser } = await setup();
    const keyTwo = await newKeyPair();
    const two = new FakeBrowser(daemon.id, 'laptop-device-2', keyTwo);

    await daemon.teach([
      { id: browser.deviceId, key: browser.key },
      { id: two.deviceId, key: keyTwo },
    ]);

    await put(browser, daemon, { t: 'vault.put', id: ID, blob: BLOB });

    expect((await put(two, daemon, { t: 'vault.put', id: ID, blob: 'D'.repeat(100) })).answer).toEqual({ t: 'vault.refused', why: 'taken' });
    expect(await get(daemon, ID)).toEqual({ t: 'vault.blob', blob: BLOB });

    // The owner of it may.
    expect((await put(browser, daemon, { t: 'vault.put', id: ID, blob: 'E'.repeat(100) })).answer).toEqual({ t: 'vault.ok' });
    expect(await get(daemon, ID)).toEqual({ t: 'vault.blob', blob: 'E'.repeat(100) });
  });

  it('a device keeps one vault: sealing a new one removes the old', async () => {
    const { daemon, browser } = await setup();

    await put(browser, daemon, { t: 'vault.put', id: ID, blob: BLOB });
    await put(browser, daemon, { t: 'vault.put', id: 'F'.repeat(22), blob: BLOB });

    expect(await get(daemon, ID)).toEqual({ t: 'vault.none' });
    expect((await get(daemon, 'F'.repeat(22))).t).toBe('vault.blob');
  });

  it('an unauthenticated pairing socket cannot store one', async () => {
    const { daemon } = await setup();
    const { socket } = await pairing(daemon);

    await daemon.socket.until((m) => m.open?.pairing === true);
    socket!.send({ t: 'vault.put', id: ID, blob: BLOB });
    daemon.socket.send({ ctl: 'ping' });
    expect((await daemon.socket.until((m) => m.t === 'pong')).t).toBe('pong');
    expect(await get(daemon, ID)).toEqual({ t: 'vault.none' });
  });
});

describe('when a vault goes away', () => {
  it('removing the device on the computer removes its vault', async () => {
    const { daemon, browser } = await setup();

    await put(browser, daemon, { t: 'vault.put', id: ID, blob: BLOB });
    daemon.socket.send({ ctl: 'device.remove', id: browser.deviceId });
    daemon.socket.send({ ctl: 'ping' });
    expect((await daemon.socket.until((m) => m.t === 'pong')).t).toBe('pong');
    expect(await get(daemon, ID)).toEqual({ t: 'vault.none' });
  });

  it('a computer that re-teaches its devices without one drops that vault, and keeps the others', async () => {
    const { daemon, browser } = await setup();
    const keyTwo = await newKeyPair();
    const two = new FakeBrowser(daemon.id, 'laptop-device-2', keyTwo);

    await daemon.teach([
      { id: browser.deviceId, key: browser.key },
      { id: two.deviceId, key: keyTwo },
    ]);
    await put(browser, daemon, { t: 'vault.put', id: ID, blob: BLOB });
    await put(two, daemon, { t: 'vault.put', id: 'G'.repeat(22), blob: BLOB });

    await daemon.teach([{ id: two.deviceId, key: keyTwo }]);

    expect(await get(daemon, ID)).toEqual({ t: 'vault.none' });
    expect((await get(daemon, 'G'.repeat(22))).t).toBe('vault.blob');
  });

  it('a pairing socket may ask only a few times', async () => {
    const { daemon } = await setup();
    const { socket } = await pairing(daemon);

    await daemon.socket.until((m) => m.open?.pairing === true);

    for (let n = 0; n < 5; n++) socket!.send({ t: 'vault.get', id: ID });

    expect(await socket!.closedWith()).toBe(CLOSE.rate);
  });
});
