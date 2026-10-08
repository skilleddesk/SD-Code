// Shared set-up for the end-to-end suites: a relay, a daemon, a paired device and a connected link.

import { expect } from 'vitest';
import type { Card } from '../src/crypto/approval';
import { MemoryStore, type PairedRecord } from '../src/crypto/device';
import { SoftPasskey } from '../src/crypto/softpasskey';
import { Link, type LinkState } from '../src/transport/link';
import { pair, parseOffer } from '../src/transport/pairing';
import { socketFor, startDaemon, startRelay, waitFor, type Daemon, type Relay } from './harness';

export interface Stack {
  relay: Relay;
  daemon: Daemon;
  origin: string;
  stop(): Promise<void>;
}

export async function startStack(): Promise<Stack> {
  const relay = await startRelay();
  const daemon = await startDaemon();

  await daemon.call('anywhere.configure', { relay: relay.wsUrl, acceptFileKey: true, notifyWhen: 'never' });
  await daemon.call('anywhere.enable');
  await waitFor('the daemon to connect to the relay', async () => (await daemon.call('anywhere.status')).connected);

  return {
    relay,
    daemon,
    origin: relay.httpUrl,
    async stop() {
      await daemon.stop();
      await relay.stop();
    },
  };
}

export interface Phone {
  record: PairedRecord;
  passkey: SoftPasskey;
  link: Link;
  cards: Card[];
  states: LinkState[];
  events: Array<Record<string, unknown>>;
}

/** Pairs a new device (confirming on the "desktop") and connects it. */
export async function newPhone(stack: Stack, name = 'Test phone'): Promise<Phone> {
  const passkey = new SoftPasskey('127.0.0.1', stack.origin);
  const store = new MemoryStore();
  const begun = await stack.daemon.call('anywhere.pair.begin', {});
  const offer = await parseOffer(new URL(begun.url).hash);
  const paired = pair({
    hubUrl: stack.relay.wsUrl,
    offer,
    deviceName: name,
    userAgent: 'vitest',
    guest: false,
    passkey,
    store,
    openSocket: socketFor(stack.origin),
    onProgress: () => undefined,
  });
  const request = await waitFor('the pairing request', async () => (await stack.daemon.call('anywhere.pair.requests')).requests[0]);

  await stack.daemon.call('anywhere.pair.confirm', { deviceId: request.deviceId, accept: true });

  const record = await paired;
  const phone: Phone = { record, passkey, link: undefined as unknown as Link, cards: [], states: [], events: [] };

  phone.link = new Link({
    hubUrl: stack.relay.wsUrl,
    record,
    store,
    passkey,
    openSocket: socketFor(stack.origin),
    events: {
      state: (state) => phone.states.push(state),
      card: (card) => phone.cards.push(card),
      event: (_seq, event) => phone.events.push(event),
    },
  });
  phone.link.start();
  await waitFor('the link', () => phone.link.current.kind === 'locked' || undefined);

  return phone;
}

export async function unlock(phone: Phone, level: 'view' | 'operate'): Promise<void> {
  await phone.link.unlock(level);
  await waitFor(`${level}`, () => phone.link.current.kind === level || undefined);
}

export { expect };
