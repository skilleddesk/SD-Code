// What this browser remembers: its signing key, which computer it is paired with, and where it was in the
// stream. IndexedDB, because a CryptoKey can be stored there **without ever being readable**: the signing
// key is generated non-extractable, so script running in the page (including a hostile one) can ask the
// browser to sign but can never copy the key out.
//
// Guest sessions use `sessionStorage`-lifetime semantics instead: the record is kept in memory only and is
// gone when the tab closes.

import type { Bytes } from './bytes';
import type { DaemonPin } from './session';

export interface PairedRecord {
  deviceId: string;
  name: string;
  guest: boolean;
  signKey: CryptoKey;
  signPublic: Bytes;
  passkeyId: string;
  daemon: DaemonPin;
  lastSeq: number;
  pairedAt: number;
  /** The sealed copy of this pairing kept at the relay so the passkey can restore it (crypto/vault.ts). Absent when the passkey has no PRF. */
  vault?: { id: string; blob: string };
}

export interface Store {
  load(): Promise<PairedRecord | null>;
  save(record: PairedRecord): Promise<void>;
  clear(): Promise<void>;
}

const DB = 'sdc-anywhere';
const STORE = 'paired';
const KEY = 'current';

function request<T>(req: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
}

function openDb(factory: IDBFactory): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const opening = factory.open(DB, 1);

    opening.onupgradeneeded = () => opening.result.createObjectStore(STORE);
    opening.onsuccess = () => resolve(opening.result);
    opening.onerror = () => reject(opening.error);
  });
}

export class IndexedDbStore implements Store {
  constructor(private readonly factory: IDBFactory = indexedDB) {}

  async load(): Promise<PairedRecord | null> {
    const db = await openDb(this.factory);

    try {
      return ((await request(db.transaction(STORE).objectStore(STORE).get(KEY))) as PairedRecord | undefined) ?? null;
    } finally {
      db.close();
    }
  }

  async save(record: PairedRecord): Promise<void> {
    const db = await openDb(this.factory);

    try {
      await request(db.transaction(STORE, 'readwrite').objectStore(STORE).put(record, KEY));
    } finally {
      db.close();
    }
  }

  async clear(): Promise<void> {
    const db = await openDb(this.factory);

    try {
      await request(db.transaction(STORE, 'readwrite').objectStore(STORE).delete(KEY));
    } finally {
      db.close();
    }
  }
}

/** A store that lives and dies with the page: for guest sessions and for tests. */
export class MemoryStore implements Store {
  private record: PairedRecord | null = null;

  async load(): Promise<PairedRecord | null> {
    return this.record;
  }

  async save(record: PairedRecord): Promise<void> {
    this.record = record;
  }

  async clear(): Promise<void> {
    this.record = null;
  }
}

/**
 * Like `newSigningKey`, for a device whose passkey can seal a rejoin vault. The key is made exportable for one moment so the vault can
 * hold it; the key this device **uses** is a fresh import of it that is not extractable, so the page can ask it to sign and never copy it.
 * The returned PKCS#8 must go only into the vault.
 */
export async function newVaultableSigningKey(): Promise<{ key: CryptoKey; publicKey: Bytes; pkcs8: Bytes }> {
  const pair = (await crypto.subtle.generateKey({ name: 'ECDSA', namedCurve: 'P-256' }, true, ['sign', 'verify'])) as CryptoKeyPair;
  const pkcs8 = new Uint8Array(await crypto.subtle.exportKey('pkcs8', pair.privateKey)) as Bytes;
  const key = await crypto.subtle.importKey('pkcs8', pkcs8, { name: 'ECDSA', namedCurve: 'P-256' }, false, ['sign']);

  return { key, publicKey: new Uint8Array(await crypto.subtle.exportKey('raw', pair.publicKey)), pkcs8 };
}

/** Makes this device's signing key: ECDSA P-256, **not extractable**. */
export async function newSigningKey(): Promise<{ key: CryptoKey; publicKey: Bytes }> {
  const pair = (await crypto.subtle.generateKey({ name: 'ECDSA', namedCurve: 'P-256' }, false, ['sign', 'verify'])) as CryptoKeyPair;

  return { key: pair.privateKey, publicKey: new Uint8Array(await crypto.subtle.exportKey('raw', pair.publicKey)) };
}
