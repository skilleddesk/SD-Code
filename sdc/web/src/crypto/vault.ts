// Getting back in after the browser forgot everything.
//
// Clearing a site's data deletes the device key and the pairing from this browser. The passkey is not in the site's data (the
// phone's passkey store holds it, and syncs it), so it is what survives. A passkey with the WebAuthn `prf` extension can also answer
// "give me 32 bytes only this passkey can produce". Those bytes are the key to a small **vault**: this device's signing key and the
// computer's public keys, sealed with AES-GCM and kept at the relay.
//
//   - the relay holds ciphertext it cannot open (the key never leaves the phone) and an id it cannot link to anyone;
//   - to open it, the passkey must be used here, with a fingerprint, face or PIN;
//   - the computer still decides who may connect: a device the owner removed there is denied no matter what the vault holds.
//
// This is a deliberate trade, described in docs/remote/THREAT-MODEL.md: the device key alone no longer needs the phone's own
// storage, so the passkey (and the account that syncs it) is now enough to rejoin.

import { b64u, concat, fromB64u, random, utf8, type Bytes } from './bytes';

/** The fixed input to the passkey's PRF. Different credentials give different outputs for the same salt. */
export const PRF_SALT: Bytes = utf8('sdc-anywhere/v1/vault');

export interface VaultContent {
  v: 1;
  deviceId: string;
  name: string;
  /** PKCS#8 of the device signing key. */
  pkcs8: string;
  /** SEC1 public half. */
  signPublic: string;
  daemon: { identityPublic: string; kemPublic: string };
}

async function derive(prf: Bytes, info: string, bits: number): Promise<Bytes> {
  const key = await crypto.subtle.importKey('raw', prf, 'HKDF', false, ['deriveBits']);

  return new Uint8Array(await crypto.subtle.deriveBits({ name: 'HKDF', hash: 'SHA-256', salt: utf8('sdc-anywhere'), info: utf8(info) }, key, bits)) as Bytes;
}

/** Where the vault is kept at the relay: 16 bytes nobody can predict without the passkey. */
export async function vaultId(prf: Bytes): Promise<string> {
  return b64u(await derive(prf, 'vault-id/v1', 128));
}

async function vaultKey(prf: Bytes): Promise<CryptoKey> {
  return crypto.subtle.importKey('raw', await derive(prf, 'vault-key/v1', 256), 'AES-GCM', false, ['encrypt', 'decrypt']);
}

/** `iv || ciphertext`, base64url, with the vault id as associated data so a blob cannot be passed off under another id. */
export async function sealVault(prf: Bytes, content: VaultContent): Promise<{ id: string; blob: string }> {
  const id = await vaultId(prf);
  const iv = random(12);
  const sealed = new Uint8Array(await crypto.subtle.encrypt({ name: 'AES-GCM', iv, additionalData: utf8(id) }, await vaultKey(prf), utf8(JSON.stringify(content)))) as Bytes;

  return { id, blob: b64u(concat(iv, sealed)) };
}

/** Throws when the blob is not authentic under this passkey's output (wrong passkey, changed, or from another id). */
export async function openVault(prf: Bytes, blob: string): Promise<VaultContent> {
  const bytes = fromB64u(blob);

  if (bytes.length < 12 + 16) throw new Error('the saved connection is damaged');

  try {
    const plain = await crypto.subtle.decrypt({ name: 'AES-GCM', iv: bytes.slice(0, 12), additionalData: utf8(await vaultId(prf)) }, await vaultKey(prf), bytes.slice(12));
    const content = JSON.parse(new TextDecoder().decode(plain)) as VaultContent;

    if (content.v !== 1 || !content.pkcs8 || !content.deviceId) throw new Error('not a vault');

    return content;
  } catch {
    throw new Error('this passkey cannot open the saved connection');
  }
}
