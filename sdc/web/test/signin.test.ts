// The sign-in link flow in the app's state: asking for a link, opening one (inert), the button that spends it, and what happens
// to the offer that comes back. The network is a fake that records every call.

import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { MemoryStore } from '../src/crypto/device';
import type { Passkey } from '../src/crypto/passkey';
import { b64u } from '../src/crypto/bytes';
import { daemonIdOf } from '../src/crypto/session';
import { Model } from '../src/state/model';
import type { PushEnv } from '../src/push/push';

const noPush: PushEnv = {
  supported: () => false,
  ios: () => false,
  permission: () => 'denied',
  requestPermission: async () => 'denied',
  registration: async () => {
    throw new Error('no push in this test');
  },
  serverKey: async () => null,
};

async function realFragment(): Promise<{ fragment: string; daemon: string }> {
  const pair = await crypto.subtle.generateKey({ name: 'ECDSA', namedCurve: 'P-256' }, true, ['sign', 'verify']);
  const identity = new Uint8Array(await crypto.subtle.exportKey('raw', pair.publicKey));
  const kem = crypto.getRandomValues(new Uint8Array(32));

  return { fragment: `v1.${b64u(crypto.getRandomValues(new Uint8Array(32)))}.${b64u(identity)}.${b64u(kem)}`, daemon: await daemonIdOf(identity) };
}

interface Call {
  url: string;
  method: string;
  body: any;
}

function fakeNet(answer: (call: Call) => { status: number; body?: unknown }) {
  const calls: Call[] = [];
  const fetcher = (async (url: string, init: RequestInit) => {
    const call = { url, method: init.method ?? 'GET', body: init.body ? JSON.parse(String(init.body)) : null };
    const reply = answer(call);

    calls.push(call);

    return new Response(JSON.stringify(reply.body ?? {}), { status: reply.status });
  }) as unknown as typeof fetch;

  return { calls, fetcher };
}

function model(fetcher: typeof fetch) {
  return new Model({ hubUrl: 'wss://sdc.test', store: new MemoryStore(), passkey: {} as Passkey, fetch: fetcher, pushEnv: noPush });
}

const replaced: string[] = [];

beforeEach(() => {
  replaced.length = 0;
  (globalThis as any).history = { replaceState: (_s: unknown, _t: string, url: string) => replaced.push(url) };
});
afterEach(() => {
  delete (globalThis as any).history;
});

const DAEMON = 'A'.repeat(22);
const TOKEN = 'T'.repeat(43);

describe('asking for a link', () => {
  it('posts the address and shows the same wording for any answer of "accepted"', async () => {
    const net = fakeNet(() => ({ status: 202, body: { ok: true } }));
    const m = model(net.fetcher);

    await m.requestLink('  Owner@Example.com ');

    expect(net.calls).toEqual([{ url: '/api/magic/request', method: 'POST', body: { email: 'Owner@Example.com' } }]);
    expect(m.getSnapshot().mailLink).toBe('sent');
  });

  it('tells a bad address, a flood and a failure apart, and a network error is just an error', async () => {
    for (const [status, expected] of [[400, 'invalid'], [429, 'busy'], [500, 'error']] as const) {
      const m = model(fakeNet(() => ({ status })).fetcher);

      await m.requestLink('a@b.test');
      expect(m.getSnapshot().mailLink, String(status)).toBe(expected);
    }

    const down = model((async () => {
      throw new Error('offline');
    }) as unknown as typeof fetch);

    await down.requestLink('a@b.test');
    expect(down.getSnapshot().mailLink).toBe('error');
  });
});

describe('opening a link', () => {
  it('only shows a page with a button: nothing is sent, and the secret leaves the address bar', () => {
    const net = fakeNet(() => ({ status: 500 }));
    const m = model(net.fetcher);

    m.initLink(`#${DAEMON}.${TOKEN}`);

    expect(net.calls).toEqual([]);
    expect(m.getSnapshot().phase).toBe('signin');
    expect(m.getSnapshot().signin).toEqual({ daemon: DAEMON, token: TOKEN, step: 'ready' });
    expect(replaced).toEqual(['/m']);
  });

  it('rejects a link that is damaged, without calling anything', () => {
    for (const hash of ['', '#', '#abc', `#${DAEMON}`, `#short.${TOKEN}`, `#${DAEMON}.x`, `#${DAEMON}.${'T'.repeat(200)}`, `#${DAEMON}.${TOKEN}!`, '#<script>.x']) {
      const net = fakeNet(() => ({ status: 500 }));
      const m = model(net.fetcher);

      m.initLink(hash);

      expect(m.getSnapshot().phase, hash).toBe('unpaired');
      expect(m.getSnapshot().error, hash).toContain('damaged');
      expect(net.calls, hash).toEqual([]);
    }
  });
});

describe('the button', () => {
  it('spends the link once and goes on to pairing with the offer', async () => {
    const { fragment, daemon } = await realFragment();
    const net = fakeNet(() => ({ status: 200, body: { daemon, fragment, fingerprint: 'x', expiresAt: 1 } }));
    const m = model(net.fetcher);

    m.initLink(`#${daemon}.${TOKEN}`);
    await m.redeemLink();

    expect(net.calls).toEqual([{ url: '/api/magic/redeem', method: 'POST', body: { daemon, token: TOKEN } }]);
    expect(m.getSnapshot().phase).toBe('pairing');
    expect(m.getSnapshot().offer?.daemon.id).toBe(daemon);
    expect(m.getSnapshot().signin).toBeNull();
    // The person still compares the six digits on the computer: the device is not trusted by this.
    expect(m.getSnapshot().device).toBeNull();
  });

  it('pressing twice quickly sends one request', async () => {
    const { fragment, daemon } = await realFragment();
    const net = fakeNet(() => ({ status: 200, body: { daemon, fragment } }));
    const m = model(net.fetcher);

    m.initLink(`#${daemon}.${TOKEN}`);
    await Promise.all([m.redeemLink(), m.redeemLink()]);

    expect(net.calls).toHaveLength(1);
  });

  it('refuses an offer for a different computer than the link was made for', async () => {
    const { fragment } = await realFragment();
    const other = await realFragment();
    const net = fakeNet(() => ({ status: 200, body: { fragment: other.fragment } }));
    const m = model(net.fetcher);

    m.initLink(`#${(await realFragment()).daemon}.${TOKEN}`);
    await m.redeemLink();

    void fragment;
    expect(m.getSnapshot().phase).toBe('signin');
    expect(m.getSnapshot().signin?.step).toBe('refused');
  });

  it('shows each kind of failure, and an offer that is not a pairing link is a failure too', async () => {
    const cases: Array<[number, unknown, string]> = [
      [400, { error: 'link_invalid' }, 'invalid'],
      [503, { error: 'offline' }, 'offline'],
      [502, { error: 'refused' }, 'refused'],
      [429, {}, 'busy'],
      [500, {}, 'error'],
      [200, { fragment: 'v1.not-a-real-offer' }, 'error'],
      [200, {}, 'error'],
    ];

    for (const [status, body, step] of cases) {
      const m = model(fakeNet(() => ({ status, body })).fetcher);

      m.initLink(`#${DAEMON}.${TOKEN}`);
      await m.redeemLink();

      expect(m.getSnapshot().phase, `${status}`).toBe('signin');
      expect(m.getSnapshot().signin?.step, `${status} ${JSON.stringify(body)}`).toBe(step);
    }
  });

  it('a link given back (computer was offline) can be pressed again', async () => {
    const { fragment, daemon } = await realFragment();
    let online = false;
    const m = model(fakeNet(() => (online ? { status: 200, body: { daemon, fragment } } : { status: 503, body: { error: 'offline' } })).fetcher);

    m.initLink(`#${daemon}.${TOKEN}`);
    await m.redeemLink();
    expect(m.getSnapshot().signin?.step).toBe('offline');

    online = true;
    await m.redeemLink();
    expect(m.getSnapshot().phase).toBe('pairing');
  });
});
