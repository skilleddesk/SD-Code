// Web Push from a Worker: RFC 8030 (delivery), RFC 8291 (message encryption, aes128gcm), RFC 8292 (VAPID).
//
// Only WebCrypto is used, so this runs in the Worker unchanged. The payload is the smallest thing that is useful:
// "something needs you" and a link. Never a command, a file name or a host (plan 5.10): the push service sees the
// message, and it is a third party.

import { b64uDecode, b64uEncode } from './util';

const encoder = new TextEncoder();

export interface PushSubscription {
  endpoint: string;
  /** The browser's P-256 public key, base64url (65 bytes uncompressed). */
  p256dh: string;
  /** The browser's 16-byte authentication secret, base64url. */
  auth: string;
}

export interface Vapid {
  /** Uncompressed P-256 public key, base64url. Also what the page passes to `pushManager.subscribe`. */
  publicKey: string;
  /** The 32-byte private scalar, base64url. A Worker secret. */
  privateKey: string;
  /** `mailto:` or `https:` contact for the push service. */
  subject: string;
}

function concat(...parts: Uint8Array[]): Uint8Array {
  const out = new Uint8Array(parts.reduce((sum, part) => sum + part.length, 0));
  let offset = 0;

  for (const part of parts) {
    out.set(part, offset);
    offset += part.length;
  }

  return out;
}

async function hkdf(secret: Uint8Array, salt: Uint8Array, info: Uint8Array, length: number): Promise<Uint8Array> {
  const key = await crypto.subtle.importKey('raw', secret, 'HKDF', false, ['deriveBits']);

  return new Uint8Array(await crypto.subtle.deriveBits({ name: 'HKDF', hash: 'SHA-256', salt, info }, key, length * 8));
}

async function importPublic(raw: Uint8Array): Promise<CryptoKey> {
  return crypto.subtle.importKey('raw', raw, { name: 'ECDH', namedCurve: 'P-256' }, false, []);
}

/** ECDH between our private key and the peer's public key. The Workers typings name the field `$public`; the runtime wants `public`. */
async function agree(peer: CryptoKey, mine: CryptoKey): Promise<Uint8Array> {
  const algorithm = { name: 'ECDH', public: peer } as unknown as SubtleCryptoDeriveKeyAlgorithm;

  return new Uint8Array(await crypto.subtle.deriveBits(algorithm, mine, 256));
}

/** A private scalar plus its public point as a CryptoKey (WebCrypto only imports private keys as JWK or PKCS8). */
async function importPrivate(scalar: Uint8Array, publicRaw: Uint8Array, algorithm: 'ECDH' | 'ECDSA'): Promise<CryptoKey> {
  const jwk = {
    kty: 'EC',
    crv: 'P-256',
    d: b64uEncode(scalar),
    x: b64uEncode(publicRaw.slice(1, 33)),
    y: b64uEncode(publicRaw.slice(33, 65)),
  };

  return crypto.subtle.importKey('jwk', jwk, algorithm === 'ECDH' ? { name: 'ECDH', namedCurve: 'P-256' } : { name: 'ECDSA', namedCurve: 'P-256' }, false, algorithm === 'ECDH' ? ['deriveBits'] : ['sign']);
}

export interface Fixed {
  /** For the RFC's test vector: the sender's ephemeral key and the salt. In production both are random. */
  senderPrivate: Uint8Array;
  senderPublic: Uint8Array;
  salt: Uint8Array;
}

/** RFC 8291 section 3.4: the body of a push message, ready to POST. */
export async function encrypt(plaintext: Uint8Array, subscription: Pick<PushSubscription, 'p256dh' | 'auth'>, fixed?: Fixed): Promise<Uint8Array> {
  const receiver = b64uDecode(subscription.p256dh);
  const authSecret = b64uDecode(subscription.auth);

  if (receiver.length !== 65 || receiver[0] !== 4) throw new Error('the subscription key is not an uncompressed P-256 point');
  if (authSecret.length !== 16) throw new Error('the subscription auth secret is not 16 bytes');

  let senderPublic: Uint8Array;
  let senderKey: CryptoKey;

  if (fixed) {
    senderPublic = fixed.senderPublic;
    senderKey = await importPrivate(fixed.senderPrivate, fixed.senderPublic, 'ECDH');
  } else {
    const pair = (await crypto.subtle.generateKey({ name: 'ECDH', namedCurve: 'P-256' }, true, ['deriveBits'])) as CryptoKeyPair;

    senderPublic = new Uint8Array((await crypto.subtle.exportKey('raw', pair.publicKey)) as ArrayBuffer);
    senderKey = pair.privateKey;
  }

  const salt = fixed?.salt ?? crypto.getRandomValues(new Uint8Array(16));
  const shared = await agree(await importPublic(receiver), senderKey);
  const keyInfo = concat(encoder.encode('WebPush: info\0'), receiver, senderPublic);
  const ikm = await hkdf(shared, authSecret, keyInfo, 32);
  const cek = await hkdf(ikm, salt, encoder.encode('Content-Encoding: aes128gcm\0'), 16);
  const nonce = await hkdf(ikm, salt, encoder.encode('Content-Encoding: nonce\0'), 12);
  // One record: the data, then 0x02 as the delimiter of the last record.
  const record = concat(plaintext, Uint8Array.from([2]));

  if (record.length > 4096 - 17) throw new Error('a push message here is a few hundred bytes; this one is too long');

  const aes = await crypto.subtle.importKey('raw', cek, 'AES-GCM', false, ['encrypt']);
  const encrypted = new Uint8Array(await crypto.subtle.encrypt({ name: 'AES-GCM', iv: nonce }, aes, record));
  const header = new Uint8Array(21 + senderPublic.length);

  header.set(salt, 0);
  new DataView(header.buffer).setUint32(16, 4096);
  header[20] = senderPublic.length;
  header.set(senderPublic, 21);

  return concat(header, encrypted);
}

/** RFC 8291 decryption, by the receiver. Used by tests to prove `encrypt` against the RFC's own example. */
export async function decrypt(body: Uint8Array, receiverPrivate: Uint8Array, receiverPublic: Uint8Array, authSecret: Uint8Array): Promise<Uint8Array> {
  const salt = body.slice(0, 16);
  const idLength = body[20] ?? 0;
  const senderPublic = body.slice(21, 21 + idLength);
  const ciphertext = body.slice(21 + idLength);
  const shared = await agree(await importPublic(senderPublic), await importPrivate(receiverPrivate, receiverPublic, 'ECDH'));
  const ikm = await hkdf(shared, authSecret, concat(encoder.encode('WebPush: info\0'), receiverPublic, senderPublic), 32);
  const cek = await hkdf(ikm, salt, encoder.encode('Content-Encoding: aes128gcm\0'), 16);
  const nonce = await hkdf(ikm, salt, encoder.encode('Content-Encoding: nonce\0'), 12);
  const aes = await crypto.subtle.importKey('raw', cek, 'AES-GCM', false, ['decrypt']);
  const record = new Uint8Array(await crypto.subtle.decrypt({ name: 'AES-GCM', iv: nonce }, aes, ciphertext));
  let end = record.length;

  while (end > 0 && record[end - 1] === 0) end--;

  if (record[end - 1] !== 2 && record[end - 1] !== 1) throw new Error('no padding delimiter');

  return record.slice(0, end - 1);
}

/** RFC 8292: the `Authorization` header for a push to `endpoint`. */
export async function vapidHeader(endpoint: string, vapid: Vapid, now = Date.now()): Promise<string> {
  const audience = new URL(endpoint).origin;
  const header = b64uEncode(encoder.encode(JSON.stringify({ typ: 'JWT', alg: 'ES256' })));
  const claims = b64uEncode(encoder.encode(JSON.stringify({ aud: audience, exp: Math.floor(now / 1000) + 12 * 3600, sub: vapid.subject })));
  const signingInput = encoder.encode(`${header}.${claims}`);
  const key = await importPrivate(b64uDecode(vapid.privateKey), b64uDecode(vapid.publicKey), 'ECDSA');
  const signature = new Uint8Array(await crypto.subtle.sign({ name: 'ECDSA', hash: 'SHA-256' }, key, signingInput));

  return `vapid t=${header}.${claims}.${b64uEncode(signature)}, k=${vapid.publicKey}`;
}

export type PushResult = 'sent' | 'gone' | 'failed';

/** The push services browsers actually use. A subscription comes from a browser, so its endpoint is untrusted input: the Worker
 *  must not be turned into something that POSTs to any URL a stranger names (server-side request forgery). */
const PUSH_HOSTS = [/^fcm\.googleapis\.com$/, /(^|\.)push\.services\.mozilla\.com$/, /(^|\.)push\.apple\.com$/, /(^|\.)notify\.windows\.com$/];

export function isPushEndpoint(endpoint: string): boolean {
  let url: URL;

  try {
    url = new URL(endpoint);
  } catch {
    return false;
  }

  return url.protocol === 'https:' && !url.username && !url.password && (url.port === '' || url.port === '443') && endpoint.length <= 2048 && PUSH_HOSTS.some((host) => host.test(url.hostname));
}

/** Checks the shape of a subscription a browser sent, without touching the network. */
export function parseSubscription(value: unknown): PushSubscription | null {
  const sub = value as Partial<PushSubscription> | null;

  if (!sub || typeof sub.endpoint !== 'string' || typeof sub.p256dh !== 'string' || typeof sub.auth !== 'string') return null;
  if (!isPushEndpoint(sub.endpoint) || sub.p256dh.length > 100 || sub.auth.length > 40) return null;

  try {
    const key = b64uDecode(sub.p256dh);

    if (key.length !== 65 || key[0] !== 4 || b64uDecode(sub.auth).length !== 16) return null;
  } catch {
    return null;
  }

  return { endpoint: sub.endpoint, p256dh: sub.p256dh, auth: sub.auth };
}

/** What the push carries. No details, ever. */
export function approvalPayload(requestId: string): Uint8Array {
  return encoder.encode(JSON.stringify({ t: 'approval', url: `/a/${requestId}` }));
}

/**
 * Sends one push. `gone` (404/410) means the browser unsubscribed: the caller should forget the subscription.
 * `fetcher` is injectable so tests need no network.
 */
export async function send(subscription: PushSubscription, payload: Uint8Array, vapid: Vapid, fetcher: typeof fetch = fetch): Promise<PushResult> {
  if (!/^https:\/\//.test(subscription.endpoint)) return 'failed';

  const body = await encrypt(payload, subscription);
  const response = await fetcher(subscription.endpoint, {
    method: 'POST',
    headers: {
      Authorization: await vapidHeader(subscription.endpoint, vapid),
      'Content-Encoding': 'aes128gcm',
      'Content-Type': 'application/octet-stream',
      TTL: '1800',
      Urgency: 'high',
    },
    body,
  });

  if (response.status === 404 || response.status === 410) return 'gone';

  return response.ok ? 'sent' : 'failed';
}

/** A fresh VAPID key pair, for `scripts/vapid.mjs`: the owner runs it once and stores the private half as a secret. */
export async function generateVapid(): Promise<{ publicKey: string; privateKey: string }> {
  const pair = (await crypto.subtle.generateKey({ name: 'ECDSA', namedCurve: 'P-256' }, true, ['sign', 'verify'])) as CryptoKeyPair;
  const jwk = (await crypto.subtle.exportKey('jwk', pair.privateKey)) as JsonWebKey;
  const raw = new Uint8Array((await crypto.subtle.exportKey('raw', pair.publicKey)) as ArrayBuffer);

  return { publicKey: b64uEncode(raw), privateKey: jwk.d ?? '' };
}
