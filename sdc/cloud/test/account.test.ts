import { env } from 'cloudflare:test';
import { beforeAll, beforeEach, describe, expect, it } from 'vitest';

import { migrate } from './helpers';

import { RETENTION, deleteAccount, exportAccount, purge } from '../src/account';
import { createMagic, ensureDaemon, saveEmail, savePush } from '../src/store';

const DAEMON = 'a'.repeat(64);
const SUBSCRIPTION = { endpoint: 'https://push.example/phone', p256dh: 'BOrX', auth: 'auth' } as unknown as Parameters<typeof savePush>[3];

async function reset(): Promise<void> {
  await env.DB.prepare('DELETE FROM magic_links').run();
  await env.DB.prepare('DELETE FROM push_subscriptions').run();
  await env.DB.prepare('DELETE FROM emails').run();
  await env.DB.prepare('DELETE FROM rate_limits').run();
  await env.DB.prepare('DELETE FROM daemons').run();
}

describe('the account the person owns', () => {
  beforeAll(migrate);

  beforeEach(async () => {
    await reset();
    await ensureDaemon(env.DB, DAEMON, 'pub');
  });

  it('hands back everything held about a daemon, with how long each thing is kept', async () => {
    await saveEmail(env.DB, DAEMON, 'someone@example.com');
    await savePush(env.DB, DAEMON, 'phone', SUBSCRIPTION);

    const account = await exportAccount(env.DB, DAEMON, 1_700_000_000_000);

    expect(account.daemon).toBe(DAEMON);
    expect(account.email).toBe('someone@example.com');
    expect(account.devices).toEqual([{ device: 'phone', endpoint: 'https://push.example/phone' }]);
    expect(account.exported_at).toBe('2023-11-14T22:13:20.000Z');
    expect(account.retention).toEqual(RETENTION);
    expect(account.retention['relayed traffic']).toContain('never stored');

    const empty = await exportAccount(env.DB, 'b'.repeat(64));

    expect(empty.email).toBeNull();
    expect(empty.devices).toEqual([]);
  });

  it('deletes the rows and says what went', async () => {
    await saveEmail(env.DB, DAEMON, 'someone@example.com');
    await savePush(env.DB, DAEMON, 'phone', SUBSCRIPTION);
    await createMagic(env.DB, DAEMON);

    const { deleted } = await deleteAccount(env.DB, DAEMON);
    const after = await exportAccount(env.DB, DAEMON);

    expect(deleted).toContain('email address');
    expect(deleted.some((line) => line.startsWith('push endpoint'))).toBe(true);
    expect(deleted.some((line) => line.includes('one-time link'))).toBe(true);
    expect(after.email).toBeNull();
    expect(after.devices).toEqual([]);
    expect((await env.DB.prepare('SELECT COUNT(*) AS n FROM magic_links').first<{ n: number }>())?.n).toBe(0);
  });

  it('keeps nothing past its retention', async () => {
    await createMagic(env.DB, DAEMON, 1_000);
    await env.DB.prepare('INSERT INTO rate_limits (bucket, window_start, count) VALUES (?, ?, 1)').bind('magic:' + DAEMON, 1_000).run();

    await purge(env.DB, 48 * 3600_000);

    expect((await env.DB.prepare('SELECT COUNT(*) AS n FROM magic_links').first<{ n: number }>())?.n).toBe(0);
    expect((await env.DB.prepare('SELECT COUNT(*) AS n FROM rate_limits').first<{ n: number }>())?.n).toBe(0);
  });
});
