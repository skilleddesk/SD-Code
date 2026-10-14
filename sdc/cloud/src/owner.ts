// The person's own cloud actions (X1, G36): the code their authenticator shows, and exporting or deleting the account.
//
// Every action is done with a link we mailed them - a row in `magic_links` that can be spent once - and, once they have set
// one up, a code from their authenticator. Nothing here is reachable with an address alone, and none of it reads a daemon's
// files: this is the cloud's own record of one computer.

import { deleteAccount, exportAccount } from './account';
import type { Env } from './hub';
import { mfaSecret, purgeTotpSteps, releaseMagic, setMfaSecret, spendMagic, spendTotpStep, withinLimit } from './store';
import { newSecret, otpauth, verify } from './totp';
import { DAEMON_ID } from './util';

/** How many owner actions one computer may make in an hour before the address has to ask for a new link. */
export const OWNER_PER_HOUR = 20;

/** A code from an authenticator: six digits, as every authenticator app shows them. */
const CODE = /^[0-9]{6}$/;

/** What a route hands back. `ok` is what the caller checks; `why` is short and says what to do about it. */
interface Answer {
  ok: boolean;
  why?: string;
  [key: string]: unknown;
}

function reply(body: Answer, status = 200): Response {
  return Response.json(body, { status, headers: { 'Cache-Control': 'no-store' } });
}

async function readBody(request: Request): Promise<Record<string, unknown>> {
  try {
    const value = await request.json();

    return typeof value === 'object' && value !== null ? (value as Record<string, unknown>) : {};
  } catch {
    return {};
  }
}

/**
 * The owner's actions, from `/api/mfa/...` and `/api/me/...`.
 *
 * A wrong code gives the link back (`releaseMagic`), so a mis-typed digit is not what ends the person's sign-in; the link
 * itself is what is spent, once, on the action that succeeds.
 */
export async function ownerRoute(request: Request, env: Env, path: string): Promise<Response> {
  if (request.method !== 'POST') return reply({ ok: false, why: 'a POST, with the link and the code in the body' }, 405);

  const body = await readBody(request);
  const daemon = typeof body.daemon === 'string' ? body.daemon : '';
  const token = typeof body.token === 'string' ? body.token : '';

  if (!DAEMON_ID.test(daemon) || token.length < 20) return reply({ ok: false, why: 'the computer and the token from your sign-in link are both needed' }, 400);

  if (!(await withinLimit(env.DB, `owner:${daemon}`, OWNER_PER_HOUR, 3600_000))) return reply({ ok: false, why: 'too many attempts; ask for a new sign-in link in an hour' }, 429);

  if (!(await spendMagic(env.DB, daemon, token))) return reply({ ok: false, why: 'that sign-in link has expired or has been used; ask for a new one' }, 401);

  const secret = await mfaSecret(env.DB, daemon);
  const code = typeof body.code === 'string' ? body.code : '';

  /* When a second factor is set, every action but enrolling one needs it. */
  if (path !== '/api/mfa/begin' && secret) {
    const step = CODE.test(code) ? await verify(secret, code, Date.now()) : null;

    if (step === null) {
      await releaseMagic(env.DB, daemon, token);

      return reply({ ok: false, why: 'the code from your authenticator, six digits, is needed for this one' }, 401);
    }

    /* The code works once: two requests with the same code land here together and one of them loses. */
    if (!(await spendTotpStep(env.DB, daemon, step))) {
      await releaseMagic(env.DB, daemon, token);

      return reply({ ok: false, why: 'that code has already been used; the next one is coming in a moment' }, 401);
    }
  }

  switch (path) {
    case '/api/mfa/begin': {
      /* The secret is not stored until a code made from it comes back: a half-finished enrolment locks nobody out. */
      const fresh = newSecret();

      return reply({ ok: true, secret: fresh, uri: otpauth(fresh, daemon), next: 'show the code from your authenticator to /api/mfa/confirm' });
    }
    case '/api/mfa/confirm': {
      const fresh = typeof body.secret === 'string' ? body.secret : '';
      const step = CODE.test(code) ? await verify(fresh, code, Date.now()) : null;

      if (fresh.length < 16 || step === null) return reply({ ok: false, why: 'the secret from /api/mfa/begin and a code from the authenticator that now holds it are both needed' }, 400);

      await setMfaSecret(env.DB, daemon, fresh);

      return reply({ ok: true, mfa: true });
    }
    case '/api/mfa/off': {
      if (!secret) return reply({ ok: true, mfa: false });

      await setMfaSecret(env.DB, daemon, null);

      return reply({ ok: true, mfa: false });
    }
    case '/api/me/export': {
      const account = await exportAccount(env.DB, daemon);

      return reply({ ok: true, mfa: Boolean(secret), account });
    }
    case '/api/me/wipe': {
      /* X5: the computer erases its own data (its keys, its sessions, its history) at its next start. The cloud cannot
         check that it did, so all this says is whether the machine was there to be asked. */
      const asked = await env.HUB.get(env.HUB.idFromName(daemon)).askWipe(daemon);

      return reply({ ok: asked.asked, asked: asked.asked, why: asked.asked ? undefined : 'the computer is not connected; it will erase the next time it starts if it was asked while it was away' });
    }
    case '/api/me/delete': {
      /* The cloud's record goes first: whatever happens to the computer, the account here is gone. */
      const { deleted } = await deleteAccount(env.DB, daemon);

      await purgeTotpSteps(env.DB, daemon);

      const asked = await env.HUB.get(env.HUB.idFromName(daemon)).askWipe('account deleted');

      return reply({ ok: true, deleted, wipe_asked: asked.asked });
    }
  }

  return reply({ ok: false, why: 'no such action' }, 404);
}
