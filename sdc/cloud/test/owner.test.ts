// X1 and G36: the person's own actions - the authenticator code, and exporting or deleting the account.

import { env, exports } from 'cloudflare:workers';
import { beforeAll, beforeEach, describe, expect, it } from 'vitest';

import { code, newSecret } from '../src/totp';
import { b64uEncode } from '../src/util';
import { createMagic, ensureDaemon, mfaSecret, purgeTotpSteps, saveEmail, setMfaSecret } from '../src/store';
import { FakeDaemon, migrate, ORIGIN } from './helpers';

let daemon = '';
let minutes = 0;

async function post(path: string, body: Record<string, unknown>): Promise<{ status: number; body: any }> {
  const response = await exports.default.fetch(`${ORIGIN}${path}`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', Origin: ORIGIN },
    body: JSON.stringify(body),
  });

  return { status: response.status, body: await response.json() };
}

/** The token from a sign-in mail, as the routes take them. */
function link(): Promise<string> {
  return createMagic(env.DB, daemon);
}

/** A code from the authenticator. Three of them per 30-second window are what `verify` accepts: the step behind, this one, and the one ahead. */
const OFFSETS = [-1, 0, 1];
let codes = 0;
let handed = '';

async function nextCode(secret: string): Promise<string> {
  const offset = OFFSETS[codes];

  if (offset === undefined) throw new Error('these tests use three codes per window: the step behind, this one, and the one ahead');

  codes += 1;
  handed = await code(secret, Math.floor(Date.now() / 30_000) + offset);

  return handed;
}

/** The code just handed out, again: what a replay looks like. */
function lastCode(): string {
  return handed;
}

describe('the owner\'s own actions', () => {
  beforeAll(async () => {
    await migrate();

    /* A computer that is on and connected: the sign-in route asks its Hub for a pairing offer before it answers. */
    const connected = await FakeDaemon.connect();

    daemon = connected.id;
    await ensureDaemon(env.DB, daemon, b64uEncode(connected.identity.publicBytes));
  });

  beforeEach(async () => {
    codes = 0;
    handed = '';
    /* The limit on these routes is real, and counted per computer an hour: the tests clear it, not the code. */
    await env.DB.prepare('DELETE FROM rate_limits').run();
    /* Each test starts with no second factor, which is where the first test leaves the account. */
    await setMfaSecret(env.DB, daemon, null);
    await purgeTotpSteps(env.DB, daemon);
  });

  it('enrols an authenticator, and then the sign-in link alone no longer signs in', async () => {
    const started = await post('/api/mfa/begin', { daemon, token: await link() });

    expect(started.status).toBe(200);

    const secret: string = started.body.secret;

    /* The secret is not stored until a code made from it comes back: a half-finished enrolment locks nobody out. */
    expect(await mfaSecret(env.DB, daemon)).toBeNull();

    const confirm = await post('/api/mfa/confirm', { daemon, token: await link(), secret, code: await nextCode(secret) });

    expect(confirm.body).toEqual({ ok: true, mfa: true });
    expect(await mfaSecret(env.DB, daemon)).toBe(secret);

    const without = await post('/api/magic/redeem', { daemon, token: await link() });

    expect(without.status).toBe(401);
    expect(without.body.error).toBe('code_needed');

    /* A wrong code is refused the same way, and the link is not spent by the refusal. */
    const wrongCode = await post('/api/magic/redeem', { daemon, token: await link(), code: '000000' });

    expect(wrongCode.body.error).toBe('code_needed');
  });

  it('spends a code once, and gives the link back when the code is wrong', async () => {
    const secret = newSecret();

    await post('/api/mfa/confirm', { daemon, token: await link(), secret, code: await nextCode(secret) });

    const token = await link();
    const first = await post('/api/me/export', { daemon, token, code: await nextCode(secret) });

    expect(first.body.ok).toBe(true);

    const reused = await post('/api/me/export', { daemon, token: await link(), code: lastCode() });

    expect(reused.status).toBe(401);
    expect(reused.body.why).toContain('already been used');

    /* A wrong code does not spend the link: the next request with a good one still works. */
    const wrongToken = await link();
    const wrong = await post('/api/me/export', { daemon, token: wrongToken, code: '000000' });

    expect(wrong.status).toBe(401);

    const right = await post('/api/me/export', { daemon, token: wrongToken, code: await nextCode(secret) });

    expect(right.body.ok).toBe(true);
  });

  it('exports what it holds, deletes all of it on request, and asks the computer to wipe itself', async () => {
    await saveEmail(env.DB, daemon, 'person@example.com');

    const exported = await post('/api/me/export', { daemon, token: await link() });

    expect(exported.body.account.email).toBe('person@example.com');

    /* X5: the cloud's record goes, and the computer is asked to erase its own data at its next start. */
    const deleted = await post('/api/me/delete', { daemon, token: await link() });

    expect(deleted.body.ok).toBe(true);
    expect(deleted.body.deleted.length).toBeGreaterThan(0);
    expect(deleted.body.wipe_asked).toBe(true);

    const after = await post('/api/me/export', { daemon, token: await link() });

    expect(after.body.account.email).toBeNull();
  });

  it('asks the computer to wipe itself, and says so when it was not there to be asked', async () => {
    const asked = await post('/api/me/wipe', { daemon, token: await link() });

    expect(asked.body).toMatchObject({ ok: true, asked: true });

    /* A computer that is not connected: the ask is refused rather than pretended. */
    const away = await FakeDaemon.connect();

    await ensureDaemon(env.DB, away.id, b64uEncode(away.identity.publicBytes));
    away.socket.close();

    const missing = await post('/api/me/wipe', { daemon: away.id, token: await createMagic(env.DB, away.id) });

    expect(missing.body.ok).toBe(false);
    expect(missing.body.why).toContain('not connected');
  });

  it('will not act on a link that has been used, or without one at all', async () => {
    const token = await link();

    expect((await post('/api/me/export', { daemon, token })).body.ok).toBe(true);
    expect((await post('/api/me/export', { daemon, token })).status).toBe(401);
    expect((await post('/api/me/export', { daemon })).status).toBe(400);
    expect((await post('/api/me/nonsense', { daemon, token: await link() })).status).toBe(404);
    expect((await post('/api/me/export', { daemon: 'short', token })).status).toBe(400);
  });
});
