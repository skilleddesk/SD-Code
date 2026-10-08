// Pairing a new device (plan 5.1).
//
// The QR code carries `/pair#v1.<token>.<daemon identity key>.<daemon kem key>`. The part after `#` is a
// URL fragment: browsers never send it to a server, so the relay never sees the token or the keys. This
// device pins the daemon's keys from it, makes its own keys, and sends a first `hello` that also carries
// the pairing body and a passkey proof bound to that very hello. The computer then shows the same six
// digits as this screen; the person compares them and confirms **on the computer**. Trust starts there, not
// at any server.

import { b64u, concat, fromB64u, random, type Bytes } from '../crypto/bytes';
import { MemoryStore, newSigningKey, newVaultableSigningKey, type PairedRecord, type Store } from '../crypto/device';
import type { Passkey } from '../crypto/passkey';
import { PRF_SALT, sealVault } from '../crypto/vault';
import { daemonIdOf, fingerprint, pairChallenge, sasCode, startHandshake, type DaemonPin } from '../crypto/session';
import { Reassembler } from '../crypto/frame';
import type { SocketLike } from './link';

export interface Offer {
  token: string;
  daemon: DaemonPin;
  fingerprint: string;
}

/** Parses the fragment of a pairing link. Throws a sentence the person can read. */
export async function parseOffer(fragment: string): Promise<Offer> {
  const parts = fragment.replace(/^#/, '').split('.');

  if (parts.length !== 4 || parts[0] !== 'v1') throw new Error('This is not an SDC Anywhere pairing link.');

  try {
    const identityPublic = fromB64u(parts[2] ?? '');
    const kemPublic = fromB64u(parts[3] ?? '');

    if (identityPublic.length !== 65 || identityPublic[0] !== 4) throw new Error('bad identity key');
    if (kemPublic.length !== 32) throw new Error('bad key');

    return { token: parts[1] ?? '', daemon: { id: await daemonIdOf(identityPublic), identityPublic, kemPublic }, fingerprint: await fingerprint(identityPublic) };
  } catch {
    throw new Error('This pairing link is damaged. Make a new one on your computer.');
  }
}

export type Progress =
  | { step: 'connecting' }
  | { step: 'passkey' }
  | { step: 'confirm'; code: string }
  | { step: 'done' };

export interface PairOptions {
  hubUrl: string;
  offer: Offer;
  deviceName: string;
  userAgent: string;
  guest: boolean;
  passkey: Passkey;
  store: Store;
  onProgress(progress: Progress): void;
  openSocket?: (url: string) => SocketLike;
  clock?: () => number;
}

/** Runs the whole pairing. Resolves with the saved record once the computer has confirmed it. */
export async function pair(options: PairOptions): Promise<PairedRecord> {
  const { offer, guest } = options;
  const clock = options.clock ?? Date.now;

  options.onProgress({ step: 'connecting' });

  // A guest keeps nothing; anyone else gets a key that can also be sealed into the rejoin vault (if the passkey turns out to have a PRF).
  const { key, publicKey, pkcs8 } = guest ? { ...(await newSigningKey()), pkcs8: null } : await newVaultableSigningKey();
  const deviceId = b64u(random(12));
  let passkeyId = '';
  let passkeyPublic = new Uint8Array(0);
  let prf: Bytes | null = null;

  if (!guest) {
    options.onProgress({ step: 'passkey' });

    // The passkey remembers which computer it is for (the user handle), so it can find its way back after the browser forgets.
    // The tail is random so a second phone on the same account does not replace this one's passkey.
    const created = await options.passkey.create(options.deviceName, concat(fromB64u(offer.daemon.id), random(16)));

    passkeyId = created.id;
    passkeyPublic = created.publicKey;
  }

  options.onProgress({ step: 'connecting' });

  const open = options.openSocket ?? ((url: string) => new WebSocket(url) as unknown as SocketLike);
  const socket = open(`${options.hubUrl.replace(/\/$/, '')}/p/${offer.daemon.id}`);
  const opened = new Promise<void>((resolve, reject) => {
    socket.onopen = () => resolve();
    socket.onerror = () => reject(new Error('Could not reach the relay.'));
  });

  await opened;

  /** The pairing assertion; the same prompt also yields the passkey's PRF output when it has one. */
  const assertion = async (nonce: string) => {
    const challenge = await pairChallenge(nonce);

    if (!options.passkey.assertWithPrf) return options.passkey.assert(passkeyId, challenge);

    const result = await options.passkey.assertWithPrf(passkeyId, challenge, PRF_SALT);

    prf = result.prf;

    return result.wire;
  };
  const handshake = await startHandshake({
    signKey: key,
    deviceId,
    daemon: offer.daemon,
    lastSeq: 0,
    now: clock(),
    pair: async (nonce) => ({
      token: offer.token,
      name: options.deviceName,
      user_agent: options.userAgent,
      sign_pub: b64u(publicKey),
      passkey_id: passkeyId,
      passkey_pub: b64u(passkeyPublic),
      assertion: guest ? null : await assertion(nonce),
    }),
  });

  return new Promise<PairedRecord>((resolve, reject) => {
    let established: Awaited<ReturnType<typeof handshake.finish>> | null = null;
    let chain: Promise<unknown> = Promise.resolve();
    const rx = new Reassembler();
    const fail = (message: string) => {
      socket.close(1000, 'pairing failed');
      reject(new Error(message));
    };

    socket.onclose = () => {
      if (!established) fail('The computer refused the pairing. The link may have expired or been used already; make a new one.');
      else fail('The pairing ended before your computer confirmed it.');
    };
    socket.onmessage = (event) => {
      chain = chain.then(async () => {
        try {
          const message = JSON.parse(String(event.data));

          if (message.t === 'offline') return fail('Your computer is offline. Turn it on and try again.');

          if (message.t === 'welcome' && !established) {
            established = await handshake.finish(message);
            options.onProgress({ step: 'confirm', code: await sasCode(offer.token, publicKey, offer.daemon.identityPublic) });

            return;
          }

          if (message.t === 'f' && established) {
            const body = rx.push(await established.recv.open(message.n, fromB64u(message.ct)));

            if (!body) return;

            const plain = JSON.parse(new TextDecoder().decode(body));

            if (plain.type === 'pair.rejected') return fail('You declined the pairing on your computer.');

            if (plain.type === 'pair.done') {
              const record: PairedRecord = {
                deviceId,
                name: options.deviceName,
                guest,
                signKey: key,
                signPublic: publicKey,
                passkeyId,
                daemon: offer.daemon,
                lastSeq: 0,
                pairedAt: clock(),
              };

              // Seal the pairing for the passkey, so clearing this browser's data is not the end of it.
              if (prf && pkcs8) {
                record.vault = await sealVault(prf, {
                  v: 1,
                  deviceId,
                  name: options.deviceName,
                  pkcs8: b64u(pkcs8),
                  signPublic: b64u(publicKey),
                  daemon: { identityPublic: b64u(offer.daemon.identityPublic), kemPublic: b64u(offer.daemon.kemPublic) },
                });
              }

              await options.store.save(record);
              options.onProgress({ step: 'done' });
              socket.onclose = null;
              socket.close(1000, 'done');
              resolve(record);
            }
          }
        } catch {
          fail('The computer did not answer as the one you paired with. Nothing was saved.');
        }
      });
    };

    socket.send(JSON.stringify(handshake.hello));
  });
}

export { MemoryStore };
