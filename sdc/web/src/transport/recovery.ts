// Rejoining after this browser's own data was cleared (crypto/vault.ts explains the idea).
//
//   1. The person picks their SDC passkey and verifies (fingerprint, face or PIN). The passkey hands back the computer's id (the user
//      handle it was made with) and, through the PRF extension, 32 bytes only it can produce.
//   2. Those bytes name and open a vault at the relay: this device's signing key and the computer's public keys.
//   3. The device is saved again exactly as if it had just been paired, and connects. The computer still decides: a device its owner
//      removed is denied there whatever the vault holds, and a hello from this device is still checked against its key.

import { b64u, fromB64u } from '../crypto/bytes';
import type { PairedRecord, Store } from '../crypto/device';
import type { Passkey } from '../crypto/passkey';
import { daemonIdOf } from '../crypto/session';
import { PRF_SALT, openVault, vaultId } from '../crypto/vault';
import type { SocketLike } from './link';

export interface RecoverOptions {
  hubUrl: string;
  passkey: Passkey;
  store: Store;
  openSocket?: (url: string) => SocketLike;
  clock?: () => number;
}

/** Why rejoining did not work, in words for the person. */
export class RecoveryFailed extends Error {
  constructor(
    readonly reason: 'unsupported' | 'cancelled' | 'no_prf' | 'offline' | 'none' | 'damaged' | 'wrong_computer',
    message: string,
  ) {
    super(message);
  }
}

function ask(options: RecoverOptions, daemon: string, id: string): Promise<string> {
  const open = options.openSocket ?? ((url: string) => new WebSocket(url) as unknown as SocketLike);
  const socket = open(`${options.hubUrl.replace(/\/$/, '')}/p/${daemon}`);

  return new Promise<string>((resolve, reject) => {
    const timer = setTimeout(() => {
      socket.close(1000, 'timeout');
      reject(new RecoveryFailed('offline', 'The relay did not answer. Check your connection and try again.'));
    }, 20_000);
    const done = (action: () => void) => {
      clearTimeout(timer);
      socket.onclose = null;
      socket.close(1000, 'done');
      action();
    };

    socket.onopen = () => socket.send(JSON.stringify({ t: 'vault.get', id }));
    socket.onerror = () => done(() => reject(new RecoveryFailed('offline', 'Could not reach the relay.')));
    socket.onclose = () => done(() => reject(new RecoveryFailed('offline', 'Your computer is offline. Turn it on and try again.')));
    socket.onmessage = (event) => {
      try {
        const message = JSON.parse(String(event.data));

        if (message.t === 'vault.blob' && typeof message.blob === 'string') return done(() => resolve(message.blob));
        if (message.t === 'vault.none') return done(() => reject(new RecoveryFailed('none', 'Nothing was saved for this passkey, or this phone was removed on your computer. Pair it again.')));
        if (message.t === 'offline') return done(() => reject(new RecoveryFailed('offline', 'Your computer is offline. Turn it on and try again.')));
      } catch {
        // Not for us.
      }
    };
  });
}

/** Restores the pairing saved for the passkey the person picks. Resolves with the record, already saved in `store`. */
export async function recover(options: RecoverOptions): Promise<PairedRecord> {
  if (!options.passkey.recover) throw new RecoveryFailed('unsupported', 'This browser cannot find a saved passkey.');

  const found = await options.passkey.recover(PRF_SALT);

  if (!found) throw new RecoveryFailed('cancelled', 'No passkey was chosen.');
  if (!found.prf) throw new RecoveryFailed('no_prf', 'This passkey cannot restore a connection (it has no secret to seal it with). Pair this phone again.');
  if (found.userHandle.length < 16) throw new RecoveryFailed('wrong_computer', 'This passkey does not belong to a computer.');

  const daemon = b64u(found.userHandle.slice(0, 16));
  const blob = await ask(options, daemon, await vaultId(found.prf));
  let content;

  try {
    content = await openVault(found.prf, blob);
  } catch (error) {
    throw new RecoveryFailed('damaged', (error as Error).message);
  }

  const identityPublic = fromB64u(content.daemon.identityPublic);

  // The vault must be for the computer the passkey says it is: its id is the hash of its key.
  if ((await daemonIdOf(identityPublic)) !== daemon) throw new RecoveryFailed('wrong_computer', 'The saved connection is for a different computer.');

  const record: PairedRecord = {
    deviceId: content.deviceId,
    name: content.name,
    guest: false,
    signKey: await crypto.subtle.importKey('pkcs8', fromB64u(content.pkcs8), { name: 'ECDSA', namedCurve: 'P-256' }, false, ['sign']),
    signPublic: fromB64u(content.signPublic),
    passkeyId: found.id,
    daemon: { id: daemon, identityPublic, kemPublic: fromB64u(content.daemon.kemPublic) },
    lastSeq: 0,
    pairedAt: (options.clock ?? Date.now)(),
    vault: { id: await vaultId(found.prf), blob },
  };

  await options.store.save(record);

  return record;
}
