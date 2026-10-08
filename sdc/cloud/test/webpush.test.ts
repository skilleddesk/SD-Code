import { describe, expect, it } from 'vitest';
import { approvalPayload, decrypt, encrypt, generateVapid, send, vapidHeader } from '../src/webpush';
import { b64uDecode, b64uEncode } from '../src/util';

// RFC 8291 appendix A: the example message, with every key and the salt fixed.
const rfc = {
  plaintext: 'When I grow up, I want to be a watermelon',
  senderPrivate: 'yfWPiYE-n46HLnH0KqZOF1fJJU3MYrct3AELtAQ-oRw',
  senderPublic: 'BP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A8',
  receiverPrivate: 'q1dXpw3UpT5VOmu_cf_v6ih07Aems3njxI-JWgLcM94',
  receiverPublic: 'BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4',
  auth: 'BTBZMqHH6r4Tts7J_aSIgg',
  salt: 'DGv6ra1nlYgDCS1FRnbzlw',
  body: 'DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPTpK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLVWGNWQexSgSxsj_Qulcy4a-fN',
};

describe('RFC 8291 message encryption', () => {
  it('reproduces the RFC\'s own example byte for byte', async () => {
    const body = await encrypt(new TextEncoder().encode(rfc.plaintext), { p256dh: rfc.receiverPublic, auth: rfc.auth }, {
      senderPrivate: b64uDecode(rfc.senderPrivate),
      senderPublic: b64uDecode(rfc.senderPublic),
      salt: b64uDecode(rfc.salt),
    });

    expect(b64uEncode(body)).toBe(rfc.body);
  });

  it('decrypts the RFC\'s example with the receiver\'s key', async () => {
    const plain = await decrypt(b64uDecode(rfc.body), b64uDecode(rfc.receiverPrivate), b64uDecode(rfc.receiverPublic), b64uDecode(rfc.auth));

    expect(new TextDecoder().decode(plain)).toBe(rfc.plaintext);
  });

  it('round-trips a fresh message with random keys and salt', async () => {
    const pair = (await crypto.subtle.generateKey({ name: 'ECDH', namedCurve: 'P-256' }, true, ['deriveBits'])) as CryptoKeyPair;
    const publicRaw = new Uint8Array((await crypto.subtle.exportKey('raw', pair.publicKey)) as ArrayBuffer);
    const jwk = (await crypto.subtle.exportKey('jwk', pair.privateKey)) as JsonWebKey;
    const auth = crypto.getRandomValues(new Uint8Array(16));
    const payload = approvalPayload('apr_perm-turn-3-1');
    const one = await encrypt(payload, { p256dh: b64uEncode(publicRaw), auth: b64uEncode(auth) });
    const two = await encrypt(payload, { p256dh: b64uEncode(publicRaw), auth: b64uEncode(auth) });

    expect(b64uEncode(one)).not.toBe(b64uEncode(two));
    expect(new TextDecoder().decode(await decrypt(one, b64uDecode(jwk.d!), publicRaw, auth))).toBe('{"t":"approval","url":"/a/apr_perm-turn-3-1"}');
  });

  it('refuses a malformed subscription', async () => {
    await expect(encrypt(new Uint8Array(4), { p256dh: 'AAAA', auth: rfc.auth })).rejects.toThrow(/uncompressed/);
    await expect(encrypt(new Uint8Array(4), { p256dh: rfc.receiverPublic, auth: 'AAAA' })).rejects.toThrow(/16 bytes/);
  });

  it('carries no details: the payload is a kind and a link', () => {
    const text = new TextDecoder().decode(approvalPayload('apr_x'));

    expect(JSON.parse(text)).toEqual({ t: 'approval', url: '/a/apr_x' });
  });
});

describe('VAPID (RFC 8292)', () => {
  it('signs a token the push service can verify with the public key', async () => {
    const keys = await generateVapid();
    const header = await vapidHeader('https://push.example.net/send/abc', { ...keys, subject: 'mailto:ops@example.org' }, 1_800_000_000_000);
    const match = /^vapid t=([^.]+)\.([^.]+)\.([^,]+), k=(.+)$/.exec(header)!;
    const claims = JSON.parse(new TextDecoder().decode(b64uDecode(match[2]!)));
    const key = await crypto.subtle.importKey('raw', b64uDecode(match[4]!), { name: 'ECDSA', namedCurve: 'P-256' }, false, ['verify']);
    const ok = await crypto.subtle.verify({ name: 'ECDSA', hash: 'SHA-256' }, key, b64uDecode(match[3]!), new TextEncoder().encode(`${match[1]}.${match[2]}`));

    expect(ok).toBe(true);
    expect(claims).toEqual({ aud: 'https://push.example.net', exp: 1_800_000_000 + 12 * 3600, sub: 'mailto:ops@example.org' });
    expect(match[4]).toBe(keys.publicKey);
  });
});

describe('sending', () => {
  const keys = generateVapid();

  async function subscription() {
    const pair = (await crypto.subtle.generateKey({ name: 'ECDH', namedCurve: 'P-256' }, true, ['deriveBits'])) as CryptoKeyPair;

    return { endpoint: 'https://push.example.net/send/abc', p256dh: b64uEncode(new Uint8Array((await crypto.subtle.exportKey('raw', pair.publicKey)) as ArrayBuffer)), auth: b64uEncode(crypto.getRandomValues(new Uint8Array(16))) };
  }

  it('posts an encrypted body with the right headers', async () => {
    const vapid = { ...(await keys), subject: 'mailto:ops@example.org' };
    let seen: { url: string; init: RequestInit } | null = null;
    const result = await send(await subscription(), approvalPayload('apr_x'), vapid, (async (url: string, init: RequestInit) => {
      seen = { url, init };

      return new Response(null, { status: 201 });
    }) as unknown as typeof fetch);
    const headers = seen!.init.headers as Record<string, string>;

    expect(result).toBe('sent');
    expect(seen!.url).toBe('https://push.example.net/send/abc');
    expect(headers['Content-Encoding']).toBe('aes128gcm');
    expect(headers.Authorization).toMatch(/^vapid t=.+, k=.+$/);
    expect(headers.TTL).toBe('1800');
    expect((seen!.init.body as Uint8Array).length).toBeGreaterThan(80);
  });

  it('reports a subscription the browser has dropped, and a failure', async () => {
    const vapid = { ...(await keys), subject: 'mailto:ops@example.org' };
    const sub = await subscription();

    expect(await send(sub, new Uint8Array(3), vapid, (async () => new Response(null, { status: 410 })) as unknown as typeof fetch)).toBe('gone');
    expect(await send(sub, new Uint8Array(3), vapid, (async () => new Response(null, { status: 500 })) as unknown as typeof fetch)).toBe('failed');
  });

  it('never posts to a plain http endpoint', async () => {
    const vapid = { ...(await keys), subject: 'mailto:ops@example.org' };
    let called = false;
    const result = await send({ ...(await subscription()), endpoint: 'http://push.example.net/x' }, new Uint8Array(3), vapid, (async () => {
      called = true;

      return new Response(null, { status: 201 });
    }) as unknown as typeof fetch);

    expect(result).toBe('failed');
    expect(called).toBe(false);
  });
});
