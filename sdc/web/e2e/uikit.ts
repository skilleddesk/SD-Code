// Shared browser set-up for the UI suites: a relay on `localhost` (passkeys need a real host name), a daemon,
// Edge/Chrome with a virtual authenticator, and a helper that pairs the page through its own screens.

import { existsSync, readFileSync } from 'node:fs';
import { chromium, type Browser, type BrowserContext, type CDPSession, type Page } from 'playwright-core';
import { freePort, startDaemon, startRelayOn, waitFor, type Daemon, type Relay } from './harness';

const axeSource = readFileSync(new URL('../node_modules/axe-core/axe.min.js', import.meta.url), 'utf8');

const BROWSER_PATHS = [
  process.env.CHROME_PATH ?? '',
  'C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe',
  'C:\\Program Files\\Microsoft\\Edge\\Application\\msedge.exe',
  '/usr/bin/google-chrome',
  '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
].filter(Boolean);

export interface Ui {
  relay: Relay;
  daemon: Daemon;
  browser: Browser;
  context: BrowserContext;
  page: Page;
  cdp: CDPSession;
  stop(): Promise<void>;
  axe(): Promise<string[]>;
  /** Uncaught errors and console errors from the page, so a blank screen explains itself. */
  errors: string[];
  /** Pairs the page through its own screens; the "desktop" confirms. */
  pairPage(name?: string): Promise<void>;
}

export async function startUi(options: { vars?: Record<string, string> } = {}): Promise<Ui> {
  const relay = await startRelayOn(await freePort(), 'localhost', options.vars);
  const daemon = await startDaemon();

  await daemon.call('anywhere.configure', { relay: relay.wsUrl, acceptFileKey: true, notifyWhen: 'never' });
  await daemon.call('anywhere.enable');
  await waitFor('the daemon to connect to the relay', async () => (await daemon.call('anywhere.status')).connected);

  const browser = await chromium.launch({ executablePath: BROWSER_PATHS.find((path) => existsSync(path)), headless: true });
  const context = await browser.newContext({ viewport: { width: 390, height: 780 }, locale: 'en-US' });
  const page = await context.newPage();
  const cdp = await context.newCDPSession(page);
  const errors: string[] = [];

  page.on('pageerror', (error) => errors.push(`pageerror: ${error.message}`));
  page.on('console', (message) => message.type() === 'error' && errors.push(`console: ${message.text()}`));

  await cdp.send('WebAuthn.enable');
  await cdp.send('WebAuthn.addVirtualAuthenticator', {
    options: { protocol: 'ctap2', transport: 'internal', hasResidentKey: true, hasUserVerification: true, isUserVerified: true, automaticPresenceSimulation: true, hasPrf: true },
  });

  return {
    relay,
    daemon,
    browser,
    context,
    page,
    cdp,
    errors,
    async stop() {
      await browser.close();
      await daemon.stop();
      await relay.stop();
    },
    async axe() {
      await page.evaluate(axeSource);

      return (await page.evaluate(async () => {
        const found = await (window as any).axe.run(document, { runOnly: ['wcag2a', 'wcag2aa', 'wcag21aa'] });

        return found.violations.map((violation: any) => `${violation.id}: ${violation.nodes.map((node: any) => node.target.join(' ')).join(', ')}`);
      })) as string[];
    },
    async pairPage(name = 'Test Edge') {
      const begun = await daemon.call('anywhere.pair.begin', {});

      await page.goto(`${relay.httpUrl}/pair${new URL(begun.url).hash}`);
      await page.getByLabel('Name this device').fill(name);
      await page.getByRole('button', { name: 'Create passkey and connect' }).click();

      const request = await waitFor('the request on the computer', async () => (await daemon.call('anywhere.pair.requests')).requests[0]);

      await daemon.call('anywhere.pair.confirm', { deviceId: request.deviceId, accept: true });
      await page
        .getByText('Locked')
        .first()
        .waitFor({ timeout: 15_000 })
        .catch(async (error: Error) => {
          /* A pairing that never reaches Locked says what the page showed instead (0.21.1). */
          const shown = await page.locator('body').innerText().catch(() => '(no page text)');

          throw new Error(`${error.message}\n--- the page said:\n${shown.slice(0, 800)}\n--- page errors:\n${errors.join('\n')}`);
        });
    },
  };
}
