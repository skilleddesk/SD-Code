// Clearing the browser's data does not cost the pairing: the passkey brings it back. Real sdcd, real relay (workerd, DO and local D1),
// a software passkey that behaves like a platform passkey with the PRF extension.

import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { MemoryStore } from '../src/crypto/device';
import { Link } from '../src/transport/link';
import { recover, RecoveryFailed } from '../src/transport/recovery';
import { socketFor, waitFor } from './harness';
import { newPhone, startStack, type Phone, type Stack } from './kit';

let stack: Stack;

beforeAll(async () => {
  stack = await startStack();
}, 240_000);

afterAll(async () => {
  await stack?.stop();
});

/** A phone whose browser data was just cleared: a new empty store, the same passkey. */
async function rejoin(phone: Phone) {
  const store = new MemoryStore();
  const record = await waitFor('the sealed copy to reach the relay', async () => {
    try {
      return await recover({ hubUrl: stack.relay.wsUrl, passkey: phone.passkey, store, openSocket: socketFor(stack.origin) });
    } catch (error) {
      if (error instanceof RecoveryFailed && error.reason === 'none') return null;

      throw error;
    }
  });
  const states: string[] = [];
  const link = new Link({
    hubUrl: stack.relay.wsUrl,
    record,
    store,
    passkey: phone.passkey,
    openSocket: socketFor(stack.origin),
    events: { state: (state) => states.push(state.kind) },
  });

  return { record, store, link, states };
}

describe('rejoining after the browser forgot everything', () => {
  it('the passkey alone brings the phone back, with no QR code, no email and no confirmation on the computer', async () => {
    const phone = await newPhone(stack, 'Cleared phone');

    expect(phone.record.vault, 'a regular pairing seals a copy for the passkey').toBeTruthy();
    phone.link.stop();

    const before = (await stack.daemon.call('anywhere.devices.list')).devices.length;
    const back = await rejoin(phone);

    expect(back.record.deviceId).toBe(phone.record.deviceId);
    expect(back.record.name).toBe('Cleared phone');
    expect(back.record.signKey.extractable).toBe(false);

    back.link.start();
    await waitFor('the restored phone to connect', () => back.link.current.kind === 'locked' || undefined);

    // Nothing was waiting on the computer, and no new device was added: it is the same device.
    expect((await stack.daemon.call('anywhere.pair.requests')).requests).toEqual([]);
    expect((await stack.daemon.call('anywhere.devices.list')).devices).toHaveLength(before);

    // And the restored passkey still opens the session.
    await back.link.unlock('view');
    await waitFor('view', () => back.link.current.kind === 'view' || undefined);
    back.link.stop();
  });

  it('a phone the owner removed on the computer cannot come back, even with its passkey', async () => {
    const phone = await newPhone(stack, 'Removed phone');

    // Let the sealed copy reach the relay first.
    phone.link.stop();
    await rejoin(phone).then((back) => back.link.stop());

    await stack.daemon.call('anywhere.devices.revoke', { deviceId: phone.record.deviceId });

    await expect(recover({ hubUrl: stack.relay.wsUrl, passkey: phone.passkey, store: new MemoryStore(), openSocket: socketFor(stack.origin) })).rejects.toMatchObject({ reason: 'none' });
  });

  it('a passkey with no PRF pairs fine but cannot be restored, and says so', async () => {
    const passkey = (await newPhone(stack, 'No PRF phone')).passkey;

    passkey.prf = false;

    await expect(recover({ hubUrl: stack.relay.wsUrl, passkey, store: new MemoryStore(), openSocket: socketFor(stack.origin) })).rejects.toMatchObject({ reason: 'no_prf' });
  });

  it('two phones on one account are restored separately', async () => {
    const one = await newPhone(stack, 'First');
    const two = await newPhone(stack, 'Second');

    one.link.stop();
    two.link.stop();

    const a = await rejoin(one);
    const b = await rejoin(two);

    expect(a.record.deviceId).toBe(one.record.deviceId);
    expect(b.record.deviceId).toBe(two.record.deviceId);
    expect(a.record.deviceId).not.toBe(b.record.deviceId);
    a.link.stop();
    b.link.stop();
  });
});
