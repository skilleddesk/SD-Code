// Phase 2 benchmarks (docs/remote/PERF.md, rows 2 and 3 as far as they can be measured without a deployed relay, and the
// Worker CPU extras for push send and the sign-in link).
//
//   pnpm --filter @sdc/web bench:remote
//
// WHAT THIS MEASURES, AND WHAT IT DOES NOT. Push delivery time and email delivery time are decided by Google/Mozilla/Apple and by
// the mail provider and Gmail. None of them is involved here, so there is NO real delivery figure in this file. What can be
// measured on one machine is the software's share: the CPU spent composing a push, how late the relay's timer is for the email
// hand-off, and how long the sign-in link's button takes to get a pairing offer back from the daemon.

import { mkdirSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { startDaemon, startMailCatcher, startRelay, sleep, waitFor, type Daemon, type Relay } from './harness';

const results: Array<{ scenario: string; samples: number; p50: number; p95: number; max: number; unit: string; note?: string }> = [];
const now = () => Number(process.hrtime.bigint()) / 1e6;
const round = (n: number) => Math.round(n * 1000) / 1000;

function stat(scenario: string, values: number[], unit = 'ms', note?: string) {
  const sorted = [...values].sort((a, b) => a - b);
  const at = (q: number) => sorted[Math.min(sorted.length - 1, Math.ceil(q * sorted.length) - 1)] ?? 0;

  results.push({ scenario, samples: values.length, p50: round(at(0.5)), p95: round(at(0.95)), max: round(sorted[sorted.length - 1] ?? 0), unit, ...(note ? { note } : {}) });
}

afterAll(() => {
  const dir = resolve(import.meta.dirname, '..', '..', 'bench', 'remote', 'results');

  mkdirSync(dir, { recursive: true });
  writeFileSync(
    resolve(dir, `phase2-${new Date().toISOString().slice(0, 10)}.json`),
    JSON.stringify({ when: new Date().toISOString(), node: process.version, platform: `${process.platform} ${process.arch}`, topology: 'relay, daemon, mail catcher and browser code on one machine (loopback); no push service, no mail provider', results }, null, 2) + '\n',
  );
});

/** The relay's own push code, loaded by path: it is typed for workerd, not for this package, so this package does not type-check it. */
async function relayCode(): Promise<{
  approvalPayload(id: string): Uint8Array;
  encrypt(plain: Uint8Array, sub: { p256dh: string; auth: string }): Promise<Uint8Array>;
  generateVapid(): Promise<{ publicKey: string; privateKey: string }>;
  send(sub: { endpoint: string; p256dh: string; auth: string }, payload: Uint8Array, vapid: { publicKey: string; privateKey: string; subject: string }, fetcher: typeof fetch): Promise<string>;
  vapidHeader(endpoint: string, vapid: { publicKey: string; privateKey: string; subject: string }): Promise<string>;
  b64uEncode(bytes: Uint8Array): string;
}> {
  const webpush = pathToFileURL(resolve(import.meta.dirname, '..', '..', 'cloud', 'src', 'webpush.ts')).href;
  const util = pathToFileURL(resolve(import.meta.dirname, '..', '..', 'cloud', 'src', 'util.ts')).href;

  return { ...(await import(/* @vite-ignore */ webpush)), ...(await import(/* @vite-ignore */ util)) };
}

describe('composing a push (Node 24 WebCrypto, a proxy for Worker CPU)', () => {
  it('encrypt + sign', async () => {
    const { approvalPayload, encrypt, generateVapid, send, vapidHeader, b64uEncode } = await relayCode();
    const keys = await generateVapid();
    const vapid = { ...keys, subject: 'mailto:ops@example.test' };
    const pair = (await crypto.subtle.generateKey({ name: 'ECDH', namedCurve: 'P-256' }, true, ['deriveBits'])) as CryptoKeyPair;
    const sub = { endpoint: 'https://fcm.googleapis.com/fcm/send/bench', p256dh: b64uEncode(new Uint8Array((await crypto.subtle.exportKey('raw', pair.publicKey)) as ArrayBuffer)), auth: b64uEncode(crypto.getRandomValues(new Uint8Array(16))) };
    const payload = approvalPayload('apr_perm-turn-1234567890-1');
    const fetcher = (async () => new Response(null, { status: 201 })) as unknown as typeof fetch;
    const encrypting: number[] = [];
    const signing: number[] = [];
    const whole: number[] = [];

    for (let n = 0; n < 220; n++) {
      let t = now();

      await encrypt(payload, sub);
      const a = now() - t;

      t = now();
      await vapidHeader(sub.endpoint, vapid);
      const b = now() - t;

      t = now();
      expect(await send(sub, payload, vapid, fetcher)).toBe('sent');
      const c = now() - t;

      if (n >= 20) {
        encrypting.push(a);
        signing.push(b);
        whole.push(c);
      }
    }

    stat('push: RFC 8291 encrypt (ECDH + 2 HKDF + AES-GCM)', encrypting, 'ms', 'Node, not workerd; the message is 45 bytes');
    stat('push: VAPID JWT sign (ECDSA P-256)', signing, 'ms', 'Node, not workerd');
    stat('push: whole send() to a stub push service', whole, 'ms', 'Node, not workerd; includes both of the above');
    expect(results.at(-1)!.p95).toBeLessThan(10);
  });
});

describe('the relay with email on, one machine', () => {
  let relay: Relay;
  let daemon: Daemon;
  let mail: Awaited<ReturnType<typeof startMailCatcher>>;

  beforeAll(async () => {
    mail = await startMailCatcher();
    relay = await startRelay({ EMAIL_PROVIDER: 'generic', EMAIL_API_URL: mail.url, EMAIL_API_KEY: 'bench-not-a-real-key', EMAIL_FROM: 'SDC <notify@example.test>' });
    daemon = await startDaemon();
    await daemon.call('anywhere.configure', { relay: relay.wsUrl, acceptFileKey: true, notifyWhen: 'always', email: 'owner@example.test', escalateEmailSec: 1 });
    await daemon.call('anywhere.enable');
    await waitFor('the daemon to connect to the relay', async () => (await daemon.call('anywhere.status')).connected);
    // The address reaches the relay's database a moment after the connection.
    await sleep(1500);
  }, 240_000);

  afterAll(async () => {
    await daemon?.stop();
    await relay?.stop();
    await mail?.stop();
  });

  it('the email hand-off: how late the relay\'s timer is, with the delay set to 1 s', async () => {
    const late: number[] = [];

    for (let n = 0; n < 8; n++) {
      const before = mail.mails.length;
      const started = now();

      await daemon.call('permission.request', { sessionId: 's1', title: `Bench ${n}`, action: 'run', target: 'pnpm test', risk: 'MUTATING', explain: '' });

      const caught = await waitFor('the approval email', () => (mail.mails.length > before ? mail.mails[before] : null), 15_000);

      expect(caught.body.text).toContain('/a/apr_perm-');
      late.push(caught.at - started - 1000);
      await sleep(300);
    }

    stat('email hand-off lateness: request -> provider call, minus the 1 s delay', late, 'ms', 'the provider call is to a local catcher; delivery time itself is not measured');
  }, 120_000);

  it('the sign-in link: request -> mail handed over, and the button -> pairing offer', async () => {
    const handedOver: number[] = [];
    const button: number[] = [];
    const origin = relay.httpUrl;

    for (let n = 0; n < 5; n++) {
      const before = mail.mails.length;
      const started = now();
      const asked = await fetch(`${origin}/api/magic/request`, { method: 'POST', headers: { 'Content-Type': 'application/json', Origin: origin }, body: JSON.stringify({ email: 'owner@example.test' }) });

      expect(asked.status).toBe(202);

      const caught = await waitFor('the sign-in email', () => (mail.mails.length > before ? mail.mails[before] : null), 15_000);
      const link = /\/m#([A-Za-z0-9_-]{22})\.([A-Za-z0-9_-]+)/.exec(caught.body.text)!;

      handedOver.push(caught.at - started);

      // The computer makes one offer per ten seconds, so each press waits for the gap to pass.
      await sleep(n === 0 ? 0 : 10_500);

      const pressed = now();
      const redeemed = await fetch(`${origin}/api/magic/redeem`, { method: 'POST', headers: { 'Content-Type': 'application/json', Origin: origin }, body: JSON.stringify({ daemon: link[1], token: link[2] }) });
      const took = now() - pressed;

      expect(redeemed.status).toBe(200);
      expect(((await redeemed.json()) as { fragment: string }).fragment).toMatch(/^v1\./);
      button.push(took);
    }

    stat('sign-in link: request accepted -> mail handed to the provider', handedOver, 'ms', 'local catcher; includes D1 writes and the Worker\'s deferred send');
    stat('sign-in link: button -> pairing offer (Worker, D1, Durable Object, daemon, and back)', button, 'ms', 'loopback; 5 samples because the daemon allows one offer per 10 s');
  }, 180_000);
});
