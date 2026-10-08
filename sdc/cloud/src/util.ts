// Small helpers shared by the Worker and the Hub. Everything here runs on WebCrypto only.

const encoder = new TextEncoder();

export function b64uEncode(bytes: Uint8Array): string {
  let binary = '';
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

export function b64uDecode(text: string): Uint8Array {
  const padded = text.replace(/-/g, '+').replace(/_/g, '/') + '='.repeat((4 - (text.length % 4)) % 4);
  const binary = atob(padded);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return bytes;
}

export async function sha256(...parts: Uint8Array[]): Promise<Uint8Array> {
  const total = parts.reduce((sum, part) => sum + part.length, 0);
  const joined = new Uint8Array(total);
  let offset = 0;
  for (const part of parts) {
    joined.set(part, offset);
    offset += part.length;
  }
  return new Uint8Array(await crypto.subtle.digest('SHA-256', joined));
}

/** The id a daemon is known by: first 16 bytes of SHA-256 of its identity public key (matches sdcd). */
export async function daemonIdOf(identityPublic: Uint8Array): Promise<string> {
  return b64uEncode((await sha256(identityPublic)).slice(0, 16));
}

/** ECDSA P-256 / SHA-256, raw r||s signature, SEC1 public key: what sdcd and WebCrypto both use. */
export async function verifySignature(publicKey: Uint8Array, message: Uint8Array, signature: Uint8Array): Promise<boolean> {
  if (signature.length !== 64 || publicKey.length !== 65 || publicKey[0] !== 4) return false;
  try {
    const key = await crypto.subtle.importKey('raw', publicKey, { name: 'ECDSA', namedCurve: 'P-256' }, false, ['verify']);
    return await crypto.subtle.verify({ name: 'ECDSA', hash: 'SHA-256' }, key, signature, message);
  } catch {
    return false;
  }
}

/** What a daemon signs to prove it holds the key its id is the hash of (mirrors `hub_auth_message` in sdcd). */
export function hubAuthMessage(nonce: string, daemonId: string): Uint8Array {
  return encoder.encode(`sdc-anywhere/v1/hub-auth|${nonce}|${daemonId}`);
}

/** What a browser device signs to connect (device key, registered when the daemon paired it). */
export function deviceAuthMessage(nonce: string, daemonId: string, deviceId: string): Uint8Array {
  return encoder.encode(`sdc-anywhere/v1/hub-device-auth|${nonce}|${daemonId}|${deviceId}`);
}

export function randomToken(bytes = 16): string {
  return b64uEncode(crypto.getRandomValues(new Uint8Array(bytes)));
}

export const DAEMON_ID = /^[A-Za-z0-9_-]{22}$/;
export const DEVICE_ID = /^[A-Za-z0-9_-]{8,64}$/;
