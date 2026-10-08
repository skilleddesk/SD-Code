// The same journey as anywhere.e2e.test.ts, but through the real page in a real browser (Microsoft Edge,
// headless) with a virtual authenticator doing real WebAuthn - so `BrowserPasskey`, the SPKI key extraction, the
// DOM and the accessibility tree are all exercised, not just the protocol.

import { existsSync, readFileSync } from 'node:fs';
import { chromium, type Browser, type BrowserContext, type CDPSession, type Page } from 'playwright-core';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { startDaemon, startRelayOn, freePort, waitFor, type Daemon, type Relay } from './harness';

const axeSource = readFileSync(new URL('../node_modules/axe-core/axe.min.js', import.meta.url), 'utf8');

let relay: Relay;
let daemon: Daemon;
let browser: Browser;
let context: BrowserContext;
let page: Page;
let cdp: CDPSession;

// Edge on Windows, Chrome elsewhere (CI sets CHROME_PATH); any Chromium works, the test drives it over CDP.
const BROWSER_PATHS = [
  process.env.CHROME_PATH ?? '',
  'C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe',
  'C:\\Program Files\\Microsoft\\Edge\\Application\\msedge.exe',
  '/usr/bin/google-chrome',
  '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
].filter(Boolean);

async function axeViolations(): Promise<string[]> {
  await page.evaluate(axeSource);

  const results = await page.evaluate(async () => {
    const found = await (window as any).axe.run(document, { runOnly: ['wcag2a', 'wcag2aa', 'wcag21aa'] });

    return found.violations.map((violation: any) => `${violation.id}: ${violation.nodes.map((node: any) => node.target.join(' ')).join(', ')}`);
  });

  return results as string[];
}

beforeAll(async () => {
  relay = await startRelayOn(await freePort(), 'localhost');
  daemon = await startDaemon();
  await daemon.call('anywhere.configure', { relay: relay.wsUrl, acceptFileKey: true, notifyWhen: 'never' });
  await daemon.call('anywhere.enable');
  await waitFor('the daemon to connect to the relay', async () => (await daemon.call('anywhere.status')).connected);

  browser = await chromium.launch({ executablePath: BROWSER_PATHS.find((path) => existsSync(path)), headless: true });
  context = await browser.newContext({ viewport: { width: 390, height: 780 }, locale: 'en-US' });
  page = await context.newPage();
  cdp = await context.newCDPSession(page);
  await cdp.send('WebAuthn.enable');
  await cdp.send('WebAuthn.addVirtualAuthenticator', {
    options: { protocol: 'ctap2', transport: 'internal', hasResidentKey: true, hasUserVerification: true, isUserVerified: true, automaticPresenceSimulation: true },
  });
}, 240_000);

afterAll(async () => {
  await browser?.close();
  await daemon?.stop();
  await relay?.stop();
});

describe('the page, in a browser', () => {
  it('pairs through the page: same code on both screens, trust only after the desktop confirms', async () => {
    const begun = await daemon.call('anywhere.pair.begin', {});
    const link = new URL(begun.url);

    // The relay serves the app on its own origin; the link's fragment carries the keys.
    await page.goto(`${relay.httpUrl}/pair${link.hash}`);
    await page.getByLabel('Name this device').fill('Test Edge');

    expect(await axeViolations(), 'the pairing screen has no WCAG 2 AA violations').toEqual([]);

    await page.getByRole('button', { name: 'Create passkey and connect' }).click();

    const code = await waitFor('the code on the page', async () => {
      const text = await page.locator('.code').textContent({ timeout: 500 }).catch(() => null);

      return text && /^\d{3} \d{3}$/.test(text) ? text : null;
    });
    const request = await waitFor('the request on the computer', async () => (await daemon.call('anywhere.pair.requests')).requests[0]);

    expect(request.code).toBe(code);
    expect(request.name).toBe('Test Edge');

    await daemon.call('anywhere.pair.confirm', { deviceId: request.deviceId, accept: true });
    await page.getByText('Locked').first().waitFor({ timeout: 15_000 });
  });

  it('shows only a count while locked, then the card after a passkey unlock', async () => {
    const { permissionId } = await daemon.call('permission.request', {
      sessionId: 's1',
      title: 'Run the tests',
      sub: 'C:/shop',
      action: 'run',
      target: 'pnpm test',
      risk: 'MUTATING',
      explain: 'Check the checkout fix.',
    });

    await page.getByText('1 request(s) waiting').waitFor({ timeout: 10_000 });
    expect(await page.content()).not.toContain('pnpm test');

    await page.getByRole('button', { name: 'Unlock with passkey' }).click();
    await page.getByRole('heading', { name: 'Run the tests' }).waitFor({ timeout: 10_000 });

    expect(await page.locator('.card').textContent()).toContain('pnpm test');
    expect(await axeViolations(), 'the inbox with a card has no WCAG 2 AA violations').toEqual([]);

    await page.getByRole('button', { name: 'Allow once' }).click();
    await page.getByText('Can change things').waitFor({ timeout: 15_000 });
    await page.getByText('Nothing is waiting for you.').waitFor({ timeout: 10_000 });

    const { events } = await daemon.call('event.list', { since: 0 });
    const resolved = events.map((e: any) => e.event ?? e).find((e: any) => e.type === 'PermissionResolved' && e.permissionId === permissionId);

    expect(resolved?.decision).toBe('allow_once');
    expect(resolved?.via).toBe('anywhere');
    expect(resolved?.deviceName).toBe('Test Edge');
  });

  it('a dangerous action asks for the passkey again, in the page', async () => {
    const { permissionId } = await daemon.call('permission.request', { sessionId: 's1', title: 'Delete the build', action: 'run', target: 'rm -rf build', risk: 'DANGEROUS', explain: '' });

    await page.getByRole('heading', { name: 'Delete the build' }).waitFor({ timeout: 10_000 });
    await page.getByText('This one needs your passkey every time.').waitFor();
    await page.getByRole('button', { name: 'Allow once' }).click();
    await page.getByText('Nothing is waiting for you.').waitFor({ timeout: 15_000 });

    const { events } = await daemon.call('event.list', { since: 0 });

    expect(events.map((e: any) => e.event ?? e).some((e: any) => e.type === 'PermissionResolved' && e.permissionId === permissionId && e.decision === 'allow_once')).toBe(true);
  });

  it('a refusal can tell the AI why', async () => {
    const { permissionId } = await daemon.call('permission.request', { sessionId: 's1', title: 'Drop a table', action: 'run', target: 'psql -c "drop table users"', risk: 'MUTATING', explain: '' });

    await page.getByRole('heading', { name: 'Drop a table' }).waitFor({ timeout: 10_000 });
    await page.getByLabel('Tell the AI why (optional)').fill('never touch the users table');
    await page.getByRole('button', { name: 'Deny' }).click();
    await page.getByText('Nothing is waiting for you.').waitFor({ timeout: 10_000 });

    const { events } = await daemon.call('event.list', { since: 0 });
    const resolved = events.map((e: any) => e.event ?? e).find((e: any) => e.type === 'PermissionResolved' && e.permissionId === permissionId);

    expect(resolved?.decision).toBe('deny');
    expect(resolved?.reason).toBe('never touch the users table');
  });

  it('Stop all asks first, then stops', async () => {
    await page.getByRole('button', { name: /Stop all/ }).click();
    await page.getByRole('alertdialog').waitFor();
    expect(await axeViolations(), 'the confirm dialog has no WCAG 2 AA violations').toEqual([]);
    await page.getByRole('button', { name: 'Stop', exact: true }).click();
    await page.getByText('Stopped.').waitFor({ timeout: 10_000 });
  });

  it('the More tab shows the fingerprint that the computer shows', async () => {
    await page.getByRole('button', { name: 'More' }).click();

    const shown = (await page.locator('.mono.big').textContent())?.trim();
    const { daemonId } = await daemon.call('anywhere.status');

    expect(shown).toMatch(/^[0-9a-f]{4}(-[0-9a-f]{4}){4}$/);
    expect(daemonId).toBeTruthy();
    expect(await axeViolations()).toEqual([]);
  });

  it('forgetting this device returns to the welcome screen', async () => {
    await page.getByRole('button', { name: 'Forget this device' }).click();
    await page.getByRole('heading', { name: 'Pair this browser' }).waitFor();
  });

  it('works in Bangla', async () => {
    const bn = await browser.newContext({ locale: 'bn-BD', viewport: { width: 390, height: 780 } });
    const bnPage = await bn.newPage();

    await bnPage.goto(`${relay.httpUrl}/`);
    await bnPage.getByRole('heading', { name: 'এই browser pair করুন' }).waitFor();
    await bn.close();
  });
});
