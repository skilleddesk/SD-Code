/**
 * **TOTP** (v3 X1): the second way into SDC Anywhere when the person's phone is the factor rather than their
 * inbox. A six-digit code, thirty seconds, checked against the RFC 6238 recipe with HMAC-SHA1 - and each accepted
 * step is remembered, so a code that was already used (a replay) is refused even inside its window.
 *
 * The secret is the person's own, from their authenticator app's QR: base32, which is what every app shows.
 */

const ALPHABET = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567';

/** The person's base32 secret as bytes (case and spaces do not matter; padding is ignored). */
export function decodeBase32(secret: string): Uint8Array {
  const clean = secret.toUpperCase().replace(/[\s=]/g, '');
  const bytes: number[] = [];
  let bits = 0;
  let value = 0;

  for (const character of clean) {
    const index = ALPHABET.indexOf(character);

    if (index < 0) throw new Error('not a base32 secret');

    value = (value << 5) | index;
    bits += 5;

    if (bits >= 8) {
      bytes.push((value >>> (bits - 8)) & 0xff);
      bits -= 8;
    }
  }

  return Uint8Array.from(bytes);
}

/** The counter a moment belongs to: thirty seconds each, from the epoch. */
export function stepOf(at: number): number {
  return Math.floor(at / 30_000);
}

function counterBytes(counter: number): Uint8Array {
  const bytes = new Uint8Array(8);

  /* Beyond 2^32 steps (the year 6000) the high half stops being zero; written out properly all the same. */
  new DataView(bytes.buffer).setUint32(4, counter >>> 0, false);
  new DataView(bytes.buffer).setUint32(0, Math.floor(counter / 2 ** 32), false);

  return bytes;
}

/** The code for a secret at a step (the raw HOTP of RFC 4226). */
export async function code(secret: string, counter: number): Promise<string> {
  const key = await crypto.subtle.importKey('raw', decodeBase32(secret), { name: 'HMAC', hash: 'SHA-1' }, false, ['sign']);
  const digest = new Uint8Array(await crypto.subtle.sign('HMAC', key, counterBytes(counter)));
  const offset = (digest[digest.length - 1] ?? 0) & 0x0f;
  const value = ((digest[offset] ?? 0) & 0x7f) * 2 ** 24 + ((digest[offset + 1] ?? 0) << 16) + ((digest[offset + 2] ?? 0) << 8) + (digest[offset + 3] ?? 0);

  return String(value % 1_000_000).padStart(6, '0');
}

/** The codes that are good right now: this step and one either side, for clocks a little apart. */
export async function currentCodes(secret: string, at: number): Promise<string[]> {
  const step = stepOf(at);

  return Promise.all([step - 1, step, step + 1].map((counter) => code(secret, counter)));
}

/**
 * Checks a code. Answers the step it matched, or `null`. The caller must remember accepted steps (`used`) and pass
 * the ones this account has already spent, so an old code cannot be replayed.
 */
export async function verify(secret: string, given: string, at: number, used: number[] = []): Promise<number | null> {
  const trimmed = given.replace(/\s/g, '');

  if (!/^\d{6}$/.test(trimmed)) return null;

  const step = stepOf(at);

  for (const counter of [step - 1, step, step + 1]) {
    if (used.includes(counter)) continue;

    if ((await code(secret, counter)) === trimmed) return counter;
  }

  return null;
}

/** A new secret for a person setting this up: 20 bytes, in the alphabet every authenticator app reads. */
export function newSecret(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(20));
  let bits = 0;
  let value = 0;
  let out = '';

  for (const byte of bytes) {
    value = (value << 8) | byte;
    bits += 8;

    while (bits >= 5) {
      out += ALPHABET[(value >>> (bits - 5)) & 31];
      bits -= 5;
    }
  }

  return out;
}

/** What an app is given to scan (`otpauth://`) - the person's own account for the label. */
export function otpauth(secret: string, account: string): string {
  return `otpauth://totp/SDC%20Anywhere:${encodeURIComponent(account)}?secret=${secret}&issuer=SDC%20Anywhere&digits=6&period=30`;
}
