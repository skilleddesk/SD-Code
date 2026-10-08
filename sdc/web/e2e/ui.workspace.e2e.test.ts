// The Files and Chat tabs and the richer approval card, in a real browser with real WebAuthn.

import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { startUi, type Ui } from './uikit';

let ui: Ui;
let project: string;

beforeAll(async () => {
  project = mkdtempSync(join(tmpdir(), 'sdc-uiproject-'));
  mkdirSync(join(project, 'src'));
  writeFileSync(join(project, 'README.md'), '# Shop\n\nThe checkout page lives in src/app.ts.\n');
  writeFileSync(join(project, 'src', 'app.ts'), 'export const checkout = () => "needle in a haystack";\n');
  writeFileSync(join(project, 'wp-config.php'), "<?php define('DB_PASSWORD', 'hunter2-not-real');\n");
  writeFileSync(join(project, '.env'), 'STRIPE_KEY=sk_live_not_real\n');

  ui = await startUi();
  await ui.daemon.call('project.add', { hostId: 'local', root: project, name: 'Shop' });
  await ui.pairPage('Files Edge');
  await ui.page.getByRole('button', { name: 'Unlock with passkey' }).click();
  await ui.page.getByText('Viewing').first().waitFor({ timeout: 15_000 });
}, 240_000);

afterAll(async () => {
  // A blank screen explains itself: nothing the page threw may go unnoticed (Chrome's note about RS256 is not ours to fix: this app takes ES256 only).
  expect(ui.errors.filter((line) => !/pubKeyCredParams/.test(line))).toEqual([]);
  await ui?.stop();
  rmSync(project, { recursive: true, force: true });
});

describe('Files, in a browser', () => {
  it('lists the project: folders, files, a locked file, and the secrets only as a count', async () => {
    const { page } = ui;

    await page.getByRole('button', { name: 'Files' }).click();
    await page.getByRole('button', { name: /README\.md/ }).waitFor({ timeout: 10_000 });

    const text = await page.locator('.files').innerText();

    expect(text).toContain('src');
    expect(text).toContain('wp-config.php');
    expect(text).toContain('Protected');
    expect(text).not.toContain('.env');
    expect(text).not.toContain('sk_live');
    expect(text).toMatch(/1 item\(s\) are hidden/);
    expect(await ui.axe(), 'the folder view has no WCAG 2 AA violations').toEqual([]);
  });

  it('opens a file with syntax highlighting, and goes back', async () => {
    const { page } = ui;

    await page.getByRole('button', { name: /README\.md/ }).click();
    await page.locator('.cm-content').waitFor({ timeout: 10_000 });

    expect(await page.locator('.cm-content').innerText()).toContain('checkout page');
    expect(await page.locator('.cm-content').getAttribute('aria-readonly')).toBe('true');
    expect(await ui.axe(), 'the file view has no WCAG 2 AA violations').toEqual([]);

    await page.getByRole('button', { name: /Back to the folder/ }).click();
    await page.getByRole('button', { name: /src/ }).first().waitFor();
  });

  it('opens a protected file only through the passkey', async () => {
    const { page } = ui;

    await page.getByRole('button', { name: /wp-config\.php/ }).click();
    await page.locator('.cm-content').waitFor({ timeout: 15_000 });

    expect(await page.locator('.cm-content').innerText()).toContain('DB_PASSWORD');

    const { events } = await ui.daemon.call('event.list', { since: 0 });

    expect(JSON.stringify(events)).toContain('protected.read');

    await page.getByRole('button', { name: /Back to the folder/ }).click();
  });

  it('searches names and text', async () => {
    const { page } = ui;

    await page.getByLabel('Search names and text').fill('needle');
    await page.getByText('src/app.ts:1').waitFor({ timeout: 10_000 });

    expect(await page.locator('.files').innerText()).toContain('needle in a haystack');

    await page.getByLabel('Search names and text').fill('');
  });

  it('picks a file for the chat and says only its name travels', async () => {
    const { page } = ui;

    await page.getByRole('checkbox', { name: /README\.md/ }).check();
    await page.getByText('1 picked for chat').waitFor();
    await page.getByRole('button', { name: 'Chat', exact: true }).click();
    await page.getByText(/Looking at: README\.md/).waitFor();

    expect(await page.locator('.composer').innerText()).toContain('Only the names of the files are sent');
    // Whether this computer has an AI connected decides the rest; either way the composer is there and says so.
    await page.locator('.chat-head select, .chat-head p').first().waitFor({ timeout: 30_000 });
    await page.getByLabel('Ask the AI to do something…').waitFor();
    expect(await ui.axe(), 'the chat view has no WCAG 2 AA violations').toEqual([]);
  });
});

describe('The approval card, in a browser', () => {
  it('shows the extra ways to answer, and not "allow for a while" on a dangerous action', async () => {
    const { page } = ui;

    await page.getByRole('button', { name: 'Inbox' }).click();

    const { permissionId } = await ui.daemon.call('permission.request', { sessionId: 's1', title: 'Install packages', sub: project, action: 'run', target: 'pnpm install', risk: 'MUTATING', explain: '' });

    await page.getByRole('heading', { name: 'Install packages' }).waitFor({ timeout: 10_000 });
    await page.getByRole('button', { name: /More ways to answer/ }).click();

    expect(await page.getByRole('button', { name: /Allow this kind of action for 30 minutes/ }).count()).toBe(1);
    expect(await ui.axe(), 'the card with its options open has no WCAG 2 AA violations').toEqual([]);

    await page.getByRole('button', { name: 'Deny', exact: true }).click();
    await page.getByText('Nothing is waiting for you.').waitFor({ timeout: 10_000 });

    const danger = await ui.daemon.call('permission.request', { sessionId: 's1', title: 'Wipe the build', sub: project, action: 'run', target: 'rm -rf build', risk: 'DANGEROUS', explain: '' });

    void permissionId;

    await page.getByRole('heading', { name: 'Wipe the build' }).waitFor({ timeout: 10_000 });
    await page.getByRole('button', { name: /More ways to answer/ }).click();

    expect(await page.getByRole('button', { name: /Allow this kind of action/ }).count()).toBe(0);

    await ui.daemon.call('permission.resolve', { permissionId: danger.permissionId, decision: 'deny' });
  });

  it('allow for 30 minutes: the second one of the same kind needs no tap', async () => {
    const { page } = ui;

    await ui.daemon.call('permission.resolve', { permissionId: 'none', decision: 'deny' }).catch(() => undefined);

    const first = await ui.daemon.call('permission.request', { sessionId: 's1', title: 'Run the tests', sub: project, action: 'run', target: 'pnpm test --run', risk: 'MUTATING', explain: '' });

    await page.getByRole('heading', { name: 'Run the tests' }).waitFor({ timeout: 10_000 });
    await page.getByRole('button', { name: /More ways to answer/ }).click();
    await page.getByRole('button', { name: /Allow this kind of action for 30 minutes/ }).click();
    await page.getByText('Nothing is waiting for you.').waitFor({ timeout: 15_000 });

    const second = await ui.daemon.call('permission.request', { sessionId: 's1', title: 'Run the tests again', sub: project, action: 'run', target: 'pnpm test --coverage', risk: 'MUTATING', explain: '' });
    const { events } = await ui.daemon.call('event.list', { since: 0 });
    const resolved = events.map((e: any) => e.event ?? e).filter((e: any) => e.type === 'PermissionResolved');

    expect(resolved.some((e: any) => e.permissionId === first.permissionId && e.scoped_minutes === 30)).toBe(true);
    await expect.poll(async () => JSON.stringify((await ui.daemon.call('event.list', { since: 0 })).events), { timeout: 5000 }).toContain(`"permissionId":"${second.permissionId}"`);
    expect(await page.getByRole('heading', { name: 'Run the tests again' }).count(), 'no card was shown').toBe(0);
  });

  it('edit the command: the original is refused and the AI is given yours', async () => {
    const { page } = ui;
    const asked = await ui.daemon.call('permission.request', { sessionId: 's1', title: 'Migrate the database', sub: project, action: 'run', target: 'pnpm prisma migrate deploy', risk: 'MUTATING', explain: '' });

    await page.getByRole('heading', { name: 'Migrate the database' }).waitFor({ timeout: 10_000 });
    await page.getByRole('button', { name: /More ways to answer/ }).click();
    await page.getByLabel('Edit the command first').fill('pnpm prisma migrate deploy --dry-run');
    await page.getByRole('button', { name: 'Run my version instead' }).click();
    await page.getByText('Nothing is waiting for you.').waitFor({ timeout: 10_000 });

    const { events } = await ui.daemon.call('event.list', { since: 0 });
    const resolved = events.map((e: any) => e.event ?? e).find((e: any) => e.type === 'PermissionResolved' && e.permissionId === asked.permissionId);

    expect(resolved?.decision).toBe('deny');
    expect(resolved?.edited_to).toBe('pnpm prisma migrate deploy --dry-run');
  });
});
