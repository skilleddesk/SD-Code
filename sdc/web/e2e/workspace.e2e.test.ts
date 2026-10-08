// Phase 2 "Done", against the real stack: a phone browses a project's files on the real daemon, opens them, searches,
// reads git state, attaches paths to a chat - and every way of reaching what it must not reach is refused.

import { execFileSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { RequestFailed } from '../src/transport/link';
import { newPhone, startStack, unlock, type Phone, type Stack } from './kit';
import { waitFor } from './harness';

let stack: Stack;
let phone: Phone;
let project: string;
let outside: string;
let rootId: string;
let hasGit = false;

const entries = (listing: any) => listing.entries as Array<{ name: string; path_id: string | null; dir: boolean; protected: boolean; link: boolean; outside: boolean; size: number }>;
const byName = (listing: any, name: string) => entries(listing).find((entry) => entry.name === name);

beforeAll(async () => {
  project = mkdtempSync(join(tmpdir(), 'sdc-project-'));
  outside = mkdtempSync(join(tmpdir(), 'sdc-outside-'));

  mkdirSync(join(project, 'src', 'deep'), { recursive: true });
  mkdirSync(join(project, 'many'));
  mkdirSync(join(project, 'backup'));
  writeFileSync(join(project, 'README.md'), '# Shop\n\nThe checkout page lives in src/app.ts.\n');
  writeFileSync(join(project, 'src', 'app.ts'), 'export const checkout = () => "needle in a haystack";\n');
  writeFileSync(join(project, 'src', 'deep', 'util.ts'), 'export const util = 1;\n');
  writeFileSync(join(project, '.env'), 'STRIPE_KEY=sk_live_not_a_real_key\n');
  writeFileSync(join(project, 'server.pem'), 'not a real key');
  writeFileSync(join(project, 'wp-config.php'), "<?php define('DB_PASSWORD', 'hunter2-not-real');\n");
  writeFileSync(join(project, 'backup', 'dump.sql'), 'INSERT INTO users VALUES (1);\n');
  writeFileSync(join(project, 'big.txt'), 'line of text for the big file\n'.repeat(40_000));
  writeFileSync(join(project, 'logo.png'), Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13, 0xff, 0xfe]));
  writeFileSync(join(project, 'blob.bin'), Buffer.from([0, 1, 2, 3, 0, 255, 254, 0]));
  writeFileSync(join(outside, 'secret-elsewhere.txt'), 'outside the project');

  for (let n = 0; n < 450; n++) writeFileSync(join(project, 'many', `file-${String(n).padStart(3, '0')}.txt`), `${n}\n`);

  try {
    symlinkSync(outside, join(project, 'escape'), 'dir');
    symlinkSync(join(project, 'README.md'), join(project, 'readme-link'));
  } catch {
    // Creating a symlink needs a privilege on Windows; the suite then skips the symlink checks.
  }

  try {
    execFileSync('git', ['init', '-q'], { cwd: project });
    hasGit = true;
  } catch {
    hasGit = false;
  }

  stack = await startStack();
  await stack.daemon.call('project.add', { hostId: 'local', root: project, name: 'Shop' });
  phone = await newPhone(stack, 'Workspace phone');
  await unlock(phone, 'view');

  const hosts = await phone.link.rpc('hosts.list');

  rootId = hosts.hosts.find((host: any) => host.host === 'local').roots.find((root: any) => root.name === 'Shop').path_id;
}, 240_000);

afterAll(async () => {
  phone?.link.stop();
  await stack?.stop();
  rmSync(project, { recursive: true, force: true });
  rmSync(outside, { recursive: true, force: true });
});

describe('Remote workspace', () => {
  it('lists the hosts and only the projects registered on them', async () => {
    const { hosts } = await phone.link.rpc('hosts.list');
    const local = hosts.find((host: any) => host.host === 'local');

    expect(local.roots.map((root: any) => root.name)).toEqual(['Shop']);
    expect(local.roots[0].path, "the folder is shown by its real path, for the person").toContain("sdc-project-");
    expect(local.roots[0].path_id).toMatch(/^p[A-Za-z0-9_-]{12}$/);
  });

  it('lists a folder: folders first, secrets counted not named, protected files locked', async () => {
    const listing = await phone.link.rpc('fs.list', { path_id: rootId });
    const names = entries(listing).map((entry) => entry.name);

    const dirs = ['backup', 'many', 'src'].map((name) => names.indexOf(name));
    const files = ['README.md', 'big.txt', 'logo.png', 'wp-config.php'].map((name) => names.indexOf(name));

    expect(Math.max(...dirs), 'every folder comes before every file').toBeLessThan(Math.min(...files));
    expect(names).not.toContain('.env');
    expect(names).not.toContain('server.pem');
    expect(listing.hidden).toBeGreaterThanOrEqual(2);
    expect(byName(listing, 'wp-config.php')?.protected).toBe(true);
    expect(byName(listing, 'README.md')?.protected).toBe(false);
    expect(byName(listing, 'src')?.dir).toBe(true);
    expect(JSON.stringify(listing)).not.toContain('sk_live');
  });

  it('pages a big folder with a cursor', async () => {
    const { entries: top } = await phone.link.rpc('fs.list', { path_id: rootId });
    const many = top.find((entry: any) => entry.name === 'many').path_id;
    const first = await phone.link.rpc('fs.list', { path_id: many });

    expect(first.entries).toHaveLength(200);
    expect(first.total).toBe(450);
    expect(first.next_cursor).toBeTruthy();

    const second = await phone.link.rpc('fs.list', { path_id: many, cursor: first.next_cursor });
    const third = await phone.link.rpc('fs.list', { path_id: many, cursor: second.next_cursor });
    const names = [...first.entries, ...second.entries, ...third.entries].map((entry: any) => entry.name);

    expect(third.next_cursor).toBeNull();
    expect(new Set(names).size).toBe(450);
    expect(names[0]).toBe('file-000.txt');
    expect(names[449]).toBe('file-449.txt');
  });

  it('opens a folder inside, with breadcrumbs back to the root', async () => {
    const top = await phone.link.rpc('fs.list', { path_id: rootId });
    const src = await phone.link.rpc('fs.list', { path_id: byName(top, 'src')!.path_id });
    const deep = await phone.link.rpc('fs.list', { path_id: byName(src, 'deep')!.path_id });

    expect(deep.breadcrumbs.map((crumb: any) => crumb.name).slice(-2)).toEqual(['src', 'deep']);
    expect(deep.breadcrumbs[0].path_id).toBe(rootId);
    expect(byName(deep, 'util.ts')).toBeTruthy();
  });

  it('opens a text file, then reads a big one in windows that join up exactly', async () => {
    const top = await phone.link.rpc('fs.list', { path_id: rootId });
    const readme = await phone.link.readFile(byName(top, 'README.md')!.path_id!);

    expect(readme.text).toContain('checkout page');
    expect(readme.binary).toBe(false);
    expect(readme.next_offset).toBeNull();
    expect(readme.sha256).toMatch(/^[0-9a-f]{64}$/);

    const big = byName(top, 'big.txt')!.path_id!;
    const first = await phone.link.readFile(big, 0);

    expect(first.size).toBe(Buffer.byteLength('line of text for the big file\n'.repeat(40_000)));
    expect(first.next_offset).toBeGreaterThan(0);
    expect(first.text.length).toBeLessThanOrEqual(512 * 1024);

    let text = first.text as string;
    let next = first.next_offset as number | null;

    while (next !== null) {
      const more = await phone.link.readFile(big, next);

      text += more.text;
      next = more.next_offset;
    }

    expect(text).toBe('line of text for the big file\n'.repeat(40_000));
  });

  it('says a binary file is binary, and previews a small picture', async () => {
    const top = await phone.link.rpc('fs.list', { path_id: rootId });
    const blob = await phone.link.readFile(byName(top, 'blob.bin')!.path_id!);
    const logo = await phone.link.readFile(byName(top, 'logo.png')!.path_id!);

    expect(blob.binary).toBe(true);
    expect(blob.text).toBeUndefined();
    expect(logo.image.mime).toBe('image/png');
    expect(Buffer.from(logo.image.data, 'base64').length).toBe(14);
  });

  it('cannot ask for a secret file: it has no id, and a made-up id opens nothing', async () => {
    await expect(phone.link.readFile('pNOT-A-REAL-ID')).rejects.toMatchObject({ code: 'not_found' });
    await expect(phone.link.rpc('fs.read', { path_id: '../../.env' })).rejects.toBeInstanceOf(RequestFailed);
    await expect(phone.link.rpc('fs.read', { path: join(project, '.env') })).rejects.toBeInstanceOf(RequestFailed);
  });

  it('a protected file needs a fresh passkey every time, and gives nothing before it', async () => {
    const top = await phone.link.rpc('fs.list', { path_id: rootId });
    const id = byName(top, 'wp-config.php')!.path_id!;
    const bare = await phone.link.rpc('fs.read', { path_id: id }).then(
      () => null,
      (error: RequestFailed) => error,
    );

    expect(bare?.code).toBe('needs_critical');
    expect(JSON.stringify(bare?.data)).not.toContain('hunter2');

    const opened = await phone.link.readFile(id);

    expect(opened.text).toContain('DB_PASSWORD');

    // The proof was spent by that read: the next one asks again.
    const again = await phone.link.rpc('fs.read', { path_id: id }).then(
      () => null,
      (error: RequestFailed) => error,
    );

    expect(again?.code).toBe('needs_critical');

    const { events } = await stack.daemon.call('event.list', { since: 0 });

    expect(JSON.stringify(events)).toContain('protected.read');
  });

  it('search finds names and lines, and never quotes a protected file', async () => {
    const found = await phone.link.rpc('fs.search', { path_id: rootId, query: 'needle' });

    expect(found.hits.some((hit: any) => hit.rel.replace(/\\/g, '/') === 'src/app.ts' && /needle in a haystack/.test(hit.text))).toBe(true);

    const names = await phone.link.rpc('fs.search', { path_id: rootId, query: 'util', mode: 'name' });

    expect(names.names.map((n: any) => n.name)).toContain('util.ts');

    const secret = await phone.link.rpc('fs.search', { path_id: rootId, query: 'hunter2' });
    const hit = secret.hits.find((h: any) => h.rel === 'wp-config.php');

    expect(hit?.text ?? null).toBeNull();
    expect(hit?.protected ?? true).toBe(true);

    const env = await phone.link.rpc('fs.search', { path_id: rootId, query: 'sk_live' });

    expect(env.hits).toEqual([]);
  });

  it('a link that leads out of the project is shown but cannot be followed', async (context) => {
    const top = await phone.link.rpc('fs.list', { path_id: rootId });
    const escape = byName(top, 'escape');

    if (!escape) return context.skip();

    expect(escape.outside).toBe(true);
    expect(escape.path_id).toBeNull();
    expect(byName(top, 'readme-link')?.path_id).toBeTruthy();
  });

  it('reports git state', async (context) => {
    if (!hasGit) return context.skip();

    const git = await phone.link.rpc('fs.git', { path_id: rootId });

    expect(typeof git.branch).toBe('string');
    expect(git.files.some((file: any) => file.rel === 'README.md' && file.status === '??')).toBe(true);
    expect(git.files.every((file: any) => !/\.env$|\.pem$/.test(file.rel) || file.hidden)).toBe(true);
  });

  it('starting a chat needs the Operate window, and attaches paths not contents', async () => {
    await expect(phone.link.rpc('chat.send', { text: 'look at the readme', root_id: rootId })).rejects.toMatchObject({ code: 'needs_operate' });

    await unlock(phone, 'operate');

    const top = await phone.link.rpc('fs.list', { path_id: rootId });
    const readme = byName(top, 'README.md')!.path_id!;
    const sent = await phone.link.rpc('chat.send', { text: 'explain this page', attachments: [readme], engine: 'ollama', model: 'none', agent: true });

    expect(sent.session_id).toBeTruthy();
    expect(sent.attached).toBe(1);

    const { events } = await stack.daemon.call('event.list', { since: 0 });
    const started = events.map((e: any) => e.event ?? e).find((e: any) => e.type === 'TurnStarted' && e.turnId === sent.turn_id);

    expect(started?.prompt ?? JSON.stringify(started)).toContain('explain this page');
    expect(JSON.stringify(started)).toContain('README.md');
    expect(JSON.stringify(started)).not.toContain('The checkout page lives in');
  });

  it('a protected file cannot be attached to a chat', async () => {
    const top = await phone.link.rpc('fs.list', { path_id: rootId });

    await expect(phone.link.rpc('chat.send', { text: 'read this', attachments: [byName(top, 'wp-config.php')!.path_id] })).rejects.toMatchObject({ code: 'permission_denied' });
  });

  it('many requests at once all arrive in order (a real page sends several together)', async () => {
    const top = await phone.link.rpc('fs.list', { path_id: rootId });
    const files = entries(top).filter((entry) => !entry.dir && !entry.protected && entry.path_id);
    const calls: Array<Promise<any>> = [];

    // Different sizes finish sealing in different orders; the daemon must still see them in the order they were sealed.
    for (let round = 0; round < 6; round++) {
      calls.push(phone.link.rpc('hosts.list'));
      calls.push(phone.link.rpc('fs.list', { path_id: rootId }));
      calls.push(phone.link.rpc('chat.options'));

      for (const file of files.slice(0, 4)) calls.push(phone.link.readFile(file.path_id!));
    }

    const results = await Promise.all(calls);

    expect(results).toHaveLength(calls.length);
    expect(phone.link.current.kind, 'the session was not cut by an out-of-order frame').toBe('operate');
  });

  it('ids survive a dropped connection, so an open file keeps working', async () => {
    const top = await phone.link.rpc('fs.list', { path_id: rootId });
    const id = byName(top, 'README.md')!.path_id!;
    const before = phone.states.length;

    (phone.link as unknown as { socket: { terminate(): void } }).socket.terminate();
    await waitFor('the link to drop', () => phone.states.slice(before).some((s) => s.kind === 'offline') || undefined);
    await waitFor('the link to return', () => phone.link.current.kind === 'operate' || undefined);

    expect((await phone.link.readFile(id)).text).toContain('checkout page');
  });
});
