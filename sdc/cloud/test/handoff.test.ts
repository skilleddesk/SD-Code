// X4: the handoff lease, through the relay rather than as a pure function - two screens of one person, one session.

import { describe, expect, it } from 'vitest';

import { FakeBrowser, FakeDaemon, newKeyPair } from './helpers';

/** A computer with two signed-in screens: a phone and a desktop. */
async function twoScreens() {
  const daemon = await FakeDaemon.connect();
  const phoneKey = await newKeyPair();
  const deskKey = await newKeyPair();
  const phone = new FakeBrowser(daemon.id, 'phone-device-1', phoneKey);
  const desk = new FakeBrowser(daemon.id, 'desk-device-2', deskKey);

  await daemon.teach([
    { id: phone.deviceId, key: phoneKey },
    { id: desk.deviceId, key: deskKey },
  ]);

  const phoneSocket = await phone.connect(daemon);
  const phoneOpen = await daemon.socket.next();
  const deskSocket = await desk.connect(daemon);
  const deskOpen = await daemon.socket.next();

  return { daemon, phoneSocket, deskSocket, phoneOpen, deskOpen };
}

describe('handoff leases', () => {
  it('hands out a fence on a claim, and only the screen holding it may send frames', async () => {
    const { daemon, phoneSocket, deskSocket, phoneOpen, deskOpen } = await twoScreens();

    daemon.socket.send({ ctl: 'handoff.claim', holder: 'phone-device-1' });

    expect(await daemon.socket.next()).toEqual({ t: 'handoff.claimed', session: 'session', holder: 'phone-device-1', fence: 1, expires_at: expect.any(Number) });

    // The screen that was given the fence is relayed, unchanged.
    phoneSocket.send({ t: 'f', n: 0, ct: 'AAAA', fence: 1 });

    expect(await daemon.socket.next()).toEqual({ conn: phoneOpen.conn, msg: { t: 'f', n: 0, ct: 'AAAA', fence: 1 } });

    // Another screen of the same person is refused, and told which fence is current.
    deskSocket.send({ t: 'f', n: 0, ct: 'BBBB', fence: 1 });

    expect(await deskSocket.next()).toEqual({ t: 'handoff.refused', session: 'session', fence: 1, why: 'fence 1 belongs to another screen; claim the session to take over' });
    expect(await daemon.socket.quiet()).toBe(true);

    // A screen that carries no fence at all is a single-screen setup in the same session: it is refused once a lease exists.
    deskSocket.send({ t: 'f', n: 1, ct: 'CCCC' });

    expect((await deskSocket.next()).why).toBe('another screen is driving this session (fence 1); claim it to take over');

    expect(deskOpen.conn).toBeTruthy();
    phoneSocket.close();
    deskSocket.close();
  });

  it('a takeover raises the fence, drops the older screen\'s frames in both directions, and releasing needs the holding fence', async () => {
    const { daemon, phoneSocket, deskSocket, phoneOpen } = await twoScreens();

    daemon.socket.send({ ctl: 'handoff.claim', holder: 'phone-device-1' });
    await daemon.socket.next();

    daemon.socket.send({ ctl: 'handoff.claim', holder: 'desk-device-2', ttl: 60_000 });

    const second = await daemon.socket.next();

    expect(second.fence).toBe(2);

    // The screen that was taken over is behind now, and is told so.
    phoneSocket.send({ t: 'f', n: 1, ct: 'DDDD', fence: 1 });

    expect((await phoneSocket.next()).why).toBe('this screen is behind: the session is at fence 2');

    // The daemon's own answers belong to the lease they were written for: an answer to fence 1 never reaches the phone.
    daemon.socket.send({ conn: phoneOpen.conn, msg: { t: 'f', n: 9, ct: 'EEEE' }, fence: 1 });

    expect(await phoneSocket.quiet()).toBe(true);

    daemon.socket.send({ conn: phoneOpen.conn, msg: { t: 'f', n: 10, ct: 'FFFF' }, fence: 2 });

    expect(await phoneSocket.next()).toEqual({ t: 'f', n: 10, ct: 'FFFF', fence: 2 });

    // Releasing with a fence that does not hold the session changes nothing.
    daemon.socket.send({ ctl: 'handoff.release', holder: 'phone-device-1', fence: 1 });

    expect((await daemon.socket.next()).t).toBe('handoff.refused');

    daemon.socket.send({ ctl: 'handoff.state' });

    expect(await daemon.socket.next()).toMatchObject({ t: 'handoff.state', session: 'session', held: true, fence: 2, holder: 'desk-device-2' });

    // The holder gives it back: the session is free, and a frame without a fence is relayed as it always was.
    daemon.socket.send({ ctl: 'handoff.release', holder: 'desk-device-2', fence: 2 });

    expect(await daemon.socket.next()).toEqual({ t: 'handoff.released', session: 'session', fence: 2 });

    phoneSocket.send({ t: 'f', n: 2, ct: 'GGGG' });

    expect(await daemon.socket.next()).toEqual({ conn: phoneOpen.conn, msg: { t: 'f', n: 2, ct: 'GGGG' } });

    deskSocket.close();
  });
});
