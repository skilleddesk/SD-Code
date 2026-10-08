// The Worker in front of the Hubs: routes WebSocket upgrades to the right Durable Object and serves the PWA.
//
// Nothing here can read a message. It decides which Hub gets a connection, keeps browsers from other
// origins out (cross-site WebSocket hijacking), and puts the security headers on the app.

import { magicMail, sendMail, validAddress } from './email';
import { Hub, type Env } from './hub';
import { MAGIC_PER_DAEMON_PER_HOUR, createMagic, daemonsForEmail, purgeMagic, releaseMagic, spendMagic, withinLimit } from './store';
import { DAEMON_ID } from './util';

export { Hub };

const JSON_HEADERS = { 'Cache-Control': 'no-store' };

/** Reads a small JSON object. Anything bigger or malformed is `null`, so a handler never sees a hostile body. */
async function smallJson(request: Request, maxBytes = 2048): Promise<Record<string, unknown> | null> {
  const text = await request.text();

  if (text.length > maxBytes) return null;

  try {
    const value = JSON.parse(text);

    return typeof value === 'object' && value !== null && !Array.isArray(value) ? value : null;
  } catch {
    return null;
  }
}

/** The sign-in endpoints answer only our own page: a form on another site must not be able to make us email people. */
function ownPage(request: Request, env: Env): boolean {
  return request.headers.get('Origin') === env.WEB_ORIGIN;
}

async function perAddress(env: Env, request: Request, scope: string): Promise<boolean> {
  if (!env.MAGIC_LIMIT) return true;

  return (await env.MAGIC_LIMIT.limit({ key: `${scope}:${request.headers.get('CF-Connecting-IP') ?? 'unknown'}` })).success;
}

/**
 * "Email me a link." The answer is the same whether or not the address belongs to anyone, and whether or not a mail was sent,
 * so this cannot be used to find out who has an SDC. The mail itself goes out after the answer.
 */
async function magicRequest(request: Request, env: Env, ctx: ExecutionContext): Promise<Response> {
  if (!ownPage(request, env)) return new Response('Wrong origin', { status: 403 });
  if (!(await perAddress(env, request, 'magic'))) return new Response('Slow down', { status: 429 });

  const body = await smallJson(request);
  const address = typeof body?.email === 'string' ? body.email.trim().toLowerCase() : '';

  if (!validAddress(address)) return Response.json({ error: 'bad_email' }, { status: 400, headers: JSON_HEADERS });

  ctx.waitUntil(
    (async () => {
      try {
        for (const daemon of await daemonsForEmail(env.DB, address)) {
          if (!(await withinLimit(env.DB, `magic:${daemon}`, MAGIC_PER_DAEMON_PER_HOUR, 3600_000))) continue;

          const token = await createMagic(env.DB, daemon);

          // The secret part is in the fragment, which a browser never sends to any server: a mail scanner that fetches the
          // link gets the page and nothing else.
          await sendMail(env, magicMail(address, `${env.WEB_ORIGIN}/m#${daemon}.${token}`));
        }

        await purgeMagic(env.DB);
      } catch {
        // Same answer either way.
      }
    })(),
  );

  return Response.json({ ok: true }, { status: 202, headers: JSON_HEADERS });
}

/**
 * The button on the sign-in page. This is the only thing that spends a link; opening it (a GET) never does. A spent link asks the
 * computer for a pairing offer; the person must still approve the new browser there.
 */
async function magicRedeem(request: Request, env: Env): Promise<Response> {
  if (!ownPage(request, env)) return new Response('Wrong origin', { status: 403 });
  if (!(await perAddress(env, request, 'redeem'))) return new Response('Slow down', { status: 429 });

  const body = await smallJson(request);
  const daemon = typeof body?.daemon === 'string' ? body.daemon : '';
  const token = typeof body?.token === 'string' ? body.token : '';

  if (!DAEMON_ID.test(daemon) || !(await spendMagic(env.DB, daemon, token))) {
    return Response.json({ error: 'link_invalid' }, { status: 400, headers: JSON_HEADERS });
  }

  const offer = await env.HUB.get(env.HUB.idFromName(daemon)).offer();

  if (!offer.ok) {
    // Nothing was granted, so the link is given back: switch the computer on and try again.
    await releaseMagic(env.DB, daemon, token);

    return Response.json({ error: offer.why === 'offline' ? 'offline' : 'refused' }, { status: offer.why === 'offline' ? 503 : 502, headers: JSON_HEADERS });
  }

  return Response.json({ daemon, fragment: offer.fragment, fingerprint: offer.fingerprint, expiresAt: offer.expiresAt }, { headers: JSON_HEADERS });
}

const ROUTES: Record<string, 'daemon' | 'device' | 'pair'> = { d: 'daemon', c: 'device', p: 'pair' };

/** Headers the app must always carry. The CSP is what stops a script from anywhere else running in it. */
export function securityHeaders(env: Pick<Env, 'WEB_ORIGIN'>): Record<string, string> {
  const socket = env.WEB_ORIGIN.replace(/^https:/, 'wss:').replace(/^http:/, 'ws:');

  return {
    'Content-Security-Policy': [
      `default-src 'none'`,
      `script-src 'self'`,
      `style-src 'self' 'unsafe-inline'`,
      `img-src 'self' data:`,
      `font-src 'self'`,
      `connect-src 'self' ${socket}`,
      `manifest-src 'self'`,
      `worker-src 'self'`,
      `base-uri 'none'`,
      `form-action 'none'`,
      `frame-ancestors 'none'`,
    ].join('; '),
    'X-Content-Type-Options': 'nosniff',
    'Referrer-Policy': 'no-referrer',
    'X-Frame-Options': 'DENY',
    'Permissions-Policy': 'camera=(), microphone=(), geolocation=(), publickey-credentials-get=(self)',
    'Strict-Transport-Security': 'max-age=31536000',
    'Cross-Origin-Opener-Policy': 'same-origin',
  };
}

export default {
  async fetch(request: Request, env: Env, ctx: ExecutionContext): Promise<Response> {
    const url = new URL(request.url);
    const parts = url.pathname.split('/').filter(Boolean);
    const role = parts.length === 2 ? ROUTES[parts[0] ?? ''] : undefined;

    if (url.pathname === '/api/health') return Response.json({ ok: true }, { headers: { 'Cache-Control': 'no-store' } });

    // The page subscribes to push with this key. It is public by design.
    if (url.pathname === '/api/push/key' && request.method === 'GET') {
      return env.VAPID_PUBLIC_KEY ? Response.json({ key: env.VAPID_PUBLIC_KEY }, { headers: JSON_HEADERS }) : new Response('Push is not set up', { status: 404 });
    }

    if (url.pathname === '/api/magic/request' || url.pathname === '/api/magic/redeem') {
      if (request.method !== 'POST') return new Response('POST only', { status: 405, headers: { Allow: 'POST' } });

      return url.pathname.endsWith('/request') ? magicRequest(request, env, ctx) : magicRedeem(request, env);
    }

    if (role) {
      const daemon = parts[1] ?? '';

      if (!DAEMON_ID.test(daemon)) return new Response('Bad id', { status: 400 });
      if (request.headers.get('Upgrade') !== 'websocket') return new Response('Expected a WebSocket', { status: 426 });

      // Every socket route is rate limited per address, so a stranger cannot make the relay wake Durable Objects for
      // ids nobody owns. The pairing route, the only unauthenticated one for browsers, has a tighter budget.
      if (env.CONNECT_LIMIT) {
        const { success } = await env.CONNECT_LIMIT.limit({ key: `${role}:${request.headers.get('CF-Connecting-IP') ?? 'unknown'}` });

        if (!success) return new Response('Slow down', { status: 429 });
      }

      // Browsers must come from our own page. A daemon is not a browser and sends no Origin.
      if (role !== 'daemon') {
        const origin = request.headers.get('Origin');

        if (origin !== env.WEB_ORIGIN) return new Response('Wrong origin', { status: 403 });

        // Pairing is the one unauthenticated route, so it is the one that is rate limited per address.
        if (role === 'pair' && env.PAIR_LIMIT) {
          const { success } = await env.PAIR_LIMIT.limit({ key: request.headers.get('CF-Connecting-IP') ?? 'unknown' });

          if (!success) return new Response('Slow down', { status: 429 });
        }
      }

      const stub = env.HUB.get(env.HUB.idFromName(daemon));
      const target = new URL(request.url);

      target.searchParams.set('role', role);
      target.searchParams.set('daemon', daemon);

      return stub.fetch(new Request(target, request));
    }

    if (url.pathname.startsWith('/api/')) return new Response('Not found', { status: 404 });

    const response = await env.ASSETS.fetch(request);
    const headers = new Headers(response.headers);

    for (const [name, value] of Object.entries(securityHeaders(env))) headers.set(name, value);

    return new Response(response.body, { status: response.status, statusText: response.statusText, headers });
  },
};
