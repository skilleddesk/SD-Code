// Byte and text helpers used by the protocol code. WebCrypto only; no dependencies.

const encoder = new TextEncoder();

export type Bytes = Uint8Array<ArrayBuffer>;

export const utf8 = (text: string): Bytes => encoder.encode(text) as Bytes;

export function b64u(bytes: Bytes): string {
  let binary = '';
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

export function fromB64u(text: string): Bytes {
  const clean = text.replace(/=+$/, '');
  if (!/^[A-Za-z0-9_-]*$/.test(clean)) throw new Error('not base64url');
  const padded = clean.replace(/-/g, '+').replace(/_/g, '/') + '='.repeat((4 - (clean.length % 4)) % 4);
  const binary = atob(padded);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return bytes;
}

export function concat(...parts: Bytes[]): Bytes {
  const out = new Uint8Array(parts.reduce((sum, part) => sum + part.length, 0));
  let offset = 0;
  for (const part of parts) {
    out.set(part, offset);
    offset += part.length;
  }
  return out;
}

export async function sha256(...parts: Bytes[]): Promise<Bytes> {
  return new Uint8Array(await crypto.subtle.digest('SHA-256', concat(...parts)));
}

export function random(length: number): Bytes {
  return crypto.getRandomValues(new Uint8Array(length));
}

/** Big-endian 8-byte counter: the associated data of every frame. */
export function counterBytes(n: number): Bytes {
  const bytes = new Uint8Array(8);
  new DataView(bytes.buffer).setBigUint64(0, BigInt(n));
  return bytes;
}

export function toHex(bytes: Bytes): string {
  return [...bytes].map((byte) => byte.toString(16).padStart(2, '0')).join('');
}

/** Constant-time comparison for values an attacker chooses. */
export function equalBytes(a: Bytes, b: Bytes): boolean {
  if (a.length !== b.length) return false;
  let diff = 0;
  for (let i = 0; i < a.length; i++) diff |= (a[i] ?? 0) ^ (b[i] ?? 0);
  return diff === 0;
}
