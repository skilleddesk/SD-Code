import { describe, expect, it } from 'vitest';
import { approvalMail, emailConfigured, magicMail, sendMail, validAddress } from '../src/email';
import { isPushEndpoint, parseSubscription } from '../src/webpush';

const KEY = 'sk-test-this-must-never-appear-anywhere';
const generic = { EMAIL_PROVIDER: 'generic', EMAIL_API_URL: 'https://mail.example.test/v1/send', EMAIL_API_KEY: KEY, EMAIL_FROM: 'SDC <notify@example.test>' };
const mail = { to: 'owner@example.test', subject: 's', text: 't', html: 'h' };

function recorder(status = 200) {
  const calls: Array<{ url: string; init: RequestInit }> = [];
  const fetcher = (async (url: string, init: RequestInit) => {
    calls.push({ url, init });

    return new Response('provider says: ' + (init.headers as Record<string, string>).Authorization, { status });
  }) as unknown as typeof fetch;

  return { calls, fetcher };
}

describe('the email adapter', () => {
  it('is off until the provider, key and sender are all set', () => {
    expect(emailConfigured({})).toBe(false);
    expect(emailConfigured({ ...generic, EMAIL_API_KEY: undefined })).toBe(false);
    expect(emailConfigured({ ...generic, EMAIL_FROM: undefined })).toBe(false);
    expect(emailConfigured({ ...generic, EMAIL_API_URL: undefined })).toBe(false);
    expect(emailConfigured({ ...generic, EMAIL_API_URL: 'http://plain.example.test/send' })).toBe(false);
    expect(emailConfigured({ ...generic, EMAIL_API_URL: 'http://mail.example.test/send' })).toBe(false);
    expect(emailConfigured({ ...generic, EMAIL_API_URL: 'http://localhost.evil.example/send' })).toBe(false);
    expect(emailConfigured({ ...generic, EMAIL_API_URL: 'http://127.0.0.1:8025/send' })).toBe(true);
    expect(emailConfigured({ ...generic, EMAIL_PROVIDER: 'unknown-vendor' })).toBe(false);
    expect(emailConfigured(generic)).toBe(true);
    expect(emailConfigured({ EMAIL_PROVIDER: 'resend', EMAIL_API_KEY: KEY, EMAIL_FROM: 'a@b.test' })).toBe(true);
  });

  it('does nothing, without error, when it is off', async () => {
    const { calls, fetcher } = recorder();

    expect(await sendMail({}, mail, fetcher)).toEqual({ sent: false, reason: 'not_configured' });
    expect(calls).toHaveLength(0);
  });

  it('posts JSON with the key in one header and nowhere else', async () => {
    const { calls, fetcher } = recorder();

    expect(await sendMail(generic, mail, fetcher)).toEqual({ sent: true });
    expect(calls).toHaveLength(1);
    expect(calls[0]!.url).toBe('https://mail.example.test/v1/send');
    expect(calls[0]!.init.headers).toEqual({ Authorization: `Bearer ${KEY}`, 'Content-Type': 'application/json' });
    expect(JSON.parse(calls[0]!.init.body as string)).toEqual({ from: 'SDC <notify@example.test>', to: ['owner@example.test'], subject: 's', text: 't', html: 'h' });
    expect(calls[0]!.init.body as string).not.toContain(KEY);
  });

  it('removes a byte-order mark and a newline from the key, and refuses a key that is still not plain ASCII', async () => {
    const { calls, fetcher } = recorder();

    expect(await sendMail({ ...generic, EMAIL_API_KEY: `﻿${KEY}\r\n` }, mail, fetcher)).toEqual({ sent: true });
    expect(calls[0]!.init.headers).toMatchObject({ Authorization: `Bearer ${KEY}` });
    expect(await sendMail({ ...generic, EMAIL_API_KEY: `${KEY} é` }, mail, fetcher)).toEqual({ sent: false, reason: 'not_configured' });
    expect(calls).toHaveLength(1);
  });

  it('uses the provider\'s own header name and scheme when configured', async () => {
    const { calls, fetcher } = recorder();

    await sendMail({ ...generic, EMAIL_AUTH_HEADER: 'X-Api-Key', EMAIL_AUTH_SCHEME: '' }, mail, fetcher);
    expect(calls[0]!.init.headers).toMatchObject({ 'X-Api-Key': KEY });
  });

  it('sends to Resend\'s address by default for the resend provider', async () => {
    const { calls, fetcher } = recorder();

    await sendMail({ EMAIL_PROVIDER: 'resend', EMAIL_API_KEY: KEY, EMAIL_FROM: 'a@b.test' }, mail, fetcher);
    expect(calls[0]!.url).toBe('https://api.resend.com/emails');
  });

  it('speaks SendKnot\'s documented format: /v1/send, Bearer key, text_body, transactional stream', async () => {
    const { calls, fetcher } = recorder();

    expect(emailConfigured({ EMAIL_PROVIDER: 'sendknot', EMAIL_API_KEY: KEY, EMAIL_FROM: 'sdc@skilleddesk.com' })).toBe(true);
    expect(await sendMail({ EMAIL_PROVIDER: 'sendknot', EMAIL_API_KEY: KEY, EMAIL_FROM: 'sdc@skilleddesk.com' }, mail, fetcher)).toEqual({ sent: true });
    expect(calls[0]!.url).toBe('https://app.sendknot.com/v1/send');
    expect(calls[0]!.init.headers).toEqual({ Authorization: `Bearer ${KEY}`, 'Content-Type': 'application/json' });
    expect(JSON.parse(calls[0]!.init.body as string)).toEqual({ from: 'sdc@skilleddesk.com', to: ['owner@example.test'], subject: 's', text_body: 't', stream: 'transactional' });
  });

  it('reports a refusal or an unreachable provider without ever returning the key or the provider\'s body', async () => {
    const refused = await sendMail(generic, mail, recorder(401).fetcher);
    const down = await sendMail(generic, mail, (async () => {
      throw new Error(`could not reach with ${KEY}`);
    }) as unknown as typeof fetch);

    expect(refused).toEqual({ sent: false, reason: 'rejected' });
    expect(down).toEqual({ sent: false, reason: 'unreachable' });
    expect(JSON.stringify([refused, down])).not.toContain(KEY);
  });

  it('refuses an address that could inject a header or a second recipient', async () => {
    const { calls, fetcher } = recorder();

    for (const to of ['a@b.test\r\nBcc: x@y.test', 'a@b.test,c@d.test', 'a b@c.test', '<a@b.test>', 'a@b', '', 'a'.repeat(300) + '@b.test']) {
      expect(await sendMail(generic, { ...mail, to }, fetcher), to).toEqual({ sent: false, reason: 'rejected' });
    }

    expect(calls).toHaveLength(0);
    expect(validAddress('first.last+tag@sub.example.co.uk')).toBe(true);
  });

  it('the messages carry a link and nothing about the work, and escape what they embed', () => {
    const nudge = approvalMail('a@b.test', 'https://sdc.skilleddesk.com/a/apr_x');
    const login = magicMail('a@b.test', 'https://sdc.skilleddesk.com/m#abc.def');
    const hostile = approvalMail('a@b.test', 'https://x.test/"><script>alert(1)</script>');

    expect(nudge.text).toContain('https://sdc.skilleddesk.com/a/apr_x');
    expect(login.text).toContain('10 minutes');
    expect(login.text).toContain('approve this browser on your computer');
    expect(hostile.html).not.toContain('<script>');
    expect(`${nudge.text}${nudge.html}`).not.toMatch(/rm |sudo|\.env|password|C:\\/i);
  });
});

describe('push endpoints and subscriptions', () => {
  it('accepts the push services browsers use and nothing else', () => {
    for (const ok of [
      'https://fcm.googleapis.com/fcm/send/abc',
      'https://updates.push.services.mozilla.com/wpush/v2/abc',
      'https://web.push.apple.com/abc',
      'https://wns2-par02p.notify.windows.com/w/?token=abc',
    ]) expect(isPushEndpoint(ok), ok).toBe(true);

    for (const bad of [
      'http://fcm.googleapis.com/x',
      'https://evil.example/fcm.googleapis.com',
      'https://fcm.googleapis.com.evil.example/x',
      'https://evilfcm.googleapis.com/x',
      'https://xpush.apple.com/x',
      'https://u:p@fcm.googleapis.com/x',
      'https://fcm.googleapis.com:8443/x',
      'https://127.0.0.1/x',
      'https://localhost/x',
      'https://169.254.169.254/latest',
      'ftp://fcm.googleapis.com/x',
      'not a url',
      `https://fcm.googleapis.com/${'a'.repeat(2100)}`,
    ]) expect(isPushEndpoint(bad), bad).toBe(false);
  });

  it('checks the key and secret lengths', async () => {
    const pair = (await crypto.subtle.generateKey({ name: 'ECDH', namedCurve: 'P-256' }, true, ['deriveBits'])) as CryptoKeyPair;
    const raw = new Uint8Array((await crypto.subtle.exportKey('raw', pair.publicKey)) as ArrayBuffer);
    const encode = (bytes: Uint8Array) => btoa(String.fromCharCode(...bytes)).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
    const good = { endpoint: 'https://fcm.googleapis.com/fcm/send/x', p256dh: encode(raw), auth: encode(crypto.getRandomValues(new Uint8Array(16))) };

    expect(parseSubscription(good)).toEqual(good);
    expect(parseSubscription({ ...good, extra: 'dropped' })).toEqual(good);
    expect(parseSubscription({ ...good, auth: encode(new Uint8Array(15)) })).toBeNull();
    expect(parseSubscription({ ...good, p256dh: encode(raw.slice(0, 64)) })).toBeNull();
    expect(parseSubscription({ ...good, p256dh: '!!!' })).toBeNull();
    expect(parseSubscription(null)).toBeNull();
    expect(parseSubscription({ endpoint: 5 })).toBeNull();
  });
});
