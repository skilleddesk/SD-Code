import { describe, expect, it } from 'vitest';
import { b64u, random, type Bytes } from '../src/crypto/bytes';
import { MemoryStore, newVaultableSigningKey } from '../src/crypto/device';
import { SoftPasskey } from '../src/crypto/softpasskey';
import { daemonIdOf } from '../src/crypto/session';
import { PRF_SALT, openVault, sealVault, vaultId, type VaultContent } from '../src/crypto/vault';
import { recover, RecoveryFailed } from '../src/transport/recovery';
import type { SocketLike } from '../src/transport/link';

async function content(daemonPublic: Bytes): Promise<VaultContent> {
  const { pkcs8, publicKey } = await newVaultableSigningKey();

  return { v: 1, deviceId: 'device-abc', name: 'My phone', pkcs8: b64u(pkcs8), signPublic: b64u(publicKey), daemon: { identityPublic: b64u(daemonPublic), kemPublic: b64u(random(32)) } };
}

async function daemonKey(): Promise<Bytes> {
  const pair = await crypto.subtle.generateKey({ name: 'ECDSA', namedCurve: 'P-256' }, true, ['sign', 'verify']);

  return new Uint8Array(await crypto.subtle.exportKey('raw', pair.publicKey)) as Bytes;
}

describe('the vault', () => {
  it('opens with the same passkey output and holds what was put in', async () => {
    const prf = random(32);
    const original = await content(await daemonKey());
    const sealed = await sealVault(prf, original);

    expect(sealed.id).toMatch(/^[A-Za-z0-9_-]{22}$/);
    expect(sealed.id).toBe(await vaultId(prf));
    expect(await openVault(prf, sealed.blob)).toEqual(original);
  });

  it('the stored blob does not show the key', async () => {
    const original = await content(await daemonKey());
    const sealed = await sealVault(random(32), original);

    expect(sealed.blob).not.toContain(original.pkcs8.slice(10, 40));
    expect(atob(sealed.blob.replace(/-/g, '+').replace(/_/g, '/'))).not.toContain('pkcs8');
  });

  it('another passkey, a changed blob, or the blob under another id cannot open it', async () => {
    const prf = random(32);
    const sealed = await sealVault(prf, await content(await daemonKey()));
    const raw = Uint8Array.from(atob(sealed.blob.replace(/-/g, '+').replace(/_/g, '/')), (c) => c.charCodeAt(0));

    await expect(openVault(random(32), sealed.blob)).rejects.toThrow(/cannot open/);

    raw[raw.length - 1]! ^= 1;

    await expect(openVault(prf, b64u(raw as Bytes))).rejects.toThrow(/cannot open/);
    await expect(openVault(prf, 'AAAA')).rejects.toThrow(/damaged/);

    // Sealed for one passkey's id, replayed for another: the id is part of what is authenticated.
    const other = random(32);
    const replay = await sealVault(other, await content(await daemonKey()));

    await expect(openVault(prf, replay.blob)).rejects.toThrow();
  });

  it('two different passkeys get different ids', async () => {
    expect(await vaultId(random(32))).not.toBe(await vaultId(random(32)));
  });

  it('the key restored from a vault signs for the same public key', async () => {
    const { key, publicKey, pkcs8 } = await newVaultableSigningKey();
    const again = await crypto.subtle.importKey('pkcs8', pkcs8, { name: 'ECDSA', namedCurve: 'P-256' }, false, ['sign']);
    const message = new Uint8Array([1, 2, 3]);
    const verifyKey = await crypto.subtle.importKey('raw', publicKey, { name: 'ECDSA', namedCurve: 'P-256' }, false, ['verify']);

    expect(await crypto.subtle.verify({ name: 'ECDSA', hash: 'SHA-256' }, verifyKey, await crypto.subtle.sign({ name: 'ECDSA', hash: 'SHA-256' }, again, message), message)).toBe(true);
    expect(key.extractable).toBe(false);
    expect(again.extractable).toBe(false);
  });
});

/** A relay that answers `vault.get` from a table, or says the computer is offline. */
function relay(vaults: Map<string, string>, offline = false) {
  const asked: string[] = [];
  const open = (): SocketLike => {
    const socket: SocketLike = {
      send: (data: string) => {
        const message = JSON.parse(data);

        asked.push(message.id);
        queueMicrotask(() => {
          const reply = offline ? { t: 'offline' } : vaults.has(message.id) ? { t: 'vault.blob', blob: vaults.get(message.id) } : { t: 'vault.none' };

          socket.onmessage?.({ data: JSON.stringify(reply) });
        });
      },
      close: () => undefined,
      onopen: null,
      onmessage: null,
      onclose: null,
      onerror: null,
    };

    queueMicrotask(() => socket.onopen?.({}));

    return socket;
  };

  return { open, asked };
}

describe('rejoining with the passkey', () => {
  async function paired() {
    const passkey = new SoftPasskey();
    const daemon = await daemonKey();
    const daemonId = await daemonIdOf(daemon);
    const { id } = await passkey.create('My phone', Uint8Array.from([...Uint8Array.from(atob(daemonId.replace(/-/g, '+').replace(/_/g, '/') + '=='), (c) => c.charCodeAt(0)), ...random(16)]) as Bytes);
    const found = (await passkey.recover(PRF_SALT))!;
    const sealed = await sealVault(found.prf!, await content(daemon));

    return { passkey, daemonId, id, sealed, vaults: new Map([[sealed.id, sealed.blob]]) };
  }

  it('finds the computer from the passkey, opens the vault and saves the record', async () => {
    const { passkey, daemonId, sealed, vaults, id } = await paired();
    const store = new MemoryStore();
    const net = relay(vaults);
    const record = await recover({ hubUrl: 'wss://relay.test', passkey, store, openSocket: net.open });

    expect(net.asked).toEqual([sealed.id]);
    expect(record.deviceId).toBe('device-abc');
    expect(record.daemon.id).toBe(daemonId);
    expect(record.passkeyId).toBe(id);
    expect(record.signKey.extractable).toBe(false);
    expect(record.vault).toEqual(sealed);
    expect(await store.load()).toBe(record);
  });

  it('says so when nothing was saved, when the computer is off, and when the passkey has no PRF', async () => {
    const { passkey, vaults } = await paired();

    await expect(recover({ hubUrl: 'wss://r', passkey, store: new MemoryStore(), openSocket: relay(new Map()).open })).rejects.toMatchObject({ reason: 'none' });
    await expect(recover({ hubUrl: 'wss://r', passkey, store: new MemoryStore(), openSocket: relay(vaults, true).open })).rejects.toMatchObject({ reason: 'offline' });

    passkey.prf = false;

    await expect(recover({ hubUrl: 'wss://r', passkey, store: new MemoryStore(), openSocket: relay(vaults).open })).rejects.toMatchObject({ reason: 'no_prf' });
  });

  it('a vault that names a different computer than the passkey does is refused, and nothing is saved', async () => {
    const passkey = new SoftPasskey();
    const real = await daemonKey();
    const other = await daemonKey();
    const handle = Uint8Array.from(atob((await daemonIdOf(real)).replace(/-/g, '+').replace(/_/g, '/') + '=='), (c) => c.charCodeAt(0));

    await passkey.create('My phone', Uint8Array.from([...handle, ...random(16)]) as Bytes);

    const found = (await passkey.recover(PRF_SALT))!;
    const forged = await sealVault(found.prf!, await content(other));
    const store = new MemoryStore();

    await expect(recover({ hubUrl: 'wss://r', passkey, store, openSocket: relay(new Map([[forged.id, forged.blob]])).open })).rejects.toMatchObject({ reason: 'wrong_computer' });
    expect(await store.load()).toBeNull();
  });

  it('a browser with no way to pick a passkey, or a cancelled pick, fails with a plain reason', async () => {
    await expect(recover({ hubUrl: 'wss://r', passkey: { create: async () => ({ id: '', publicKey: new Uint8Array() as Bytes }), assert: async () => ({}) as never }, store: new MemoryStore() })).rejects.toBeInstanceOf(RecoveryFailed);
    await expect(recover({ hubUrl: 'wss://r', passkey: new SoftPasskey(), store: new MemoryStore() })).rejects.toMatchObject({ reason: 'cancelled' });
  });
});
