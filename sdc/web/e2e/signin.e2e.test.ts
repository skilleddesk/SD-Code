// Sign in by email link, against the real stack: the real sdcd, the real Worker and Durable Object on workerd, a real local D1,
// a real browser, and a mail catcher standing in for the provider. A browser with no paired device asks for a link, opens it,
// presses the button, compares the six digits, and is let in only when the computer confirms.

import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { startMailCatcher, waitFor } from './harness';
import { startUi, type Ui } from './uikit';

let ui: Ui;
let mail: Awaited<ReturnType<typeof startMailCatcher>>;

const KEY = 'e2e-not-a-real-key';

beforeAll(async () => {
  mail = await startMailCatcher();
  ui = await startUi({ vars: { EMAIL_PROVIDER: 'generic', EMAIL_API_URL: mail.url, EMAIL_API_KEY: KEY, EMAIL_FROM: 'SDC <notify@example.test>' } });
  await ui.daemon.call('anywhere.configure', { email: 'owner@example.test' });
}, 240_000);

afterAll(async () => {
  await ui?.stop();
  await mail?.stop();
});

/** The page as a browser that has never been paired: its stored keys are removed first. */
async function unpairedPage(): Promise<void> {
  await ui.page.goto(ui.relay.httpUrl);
  await ui.page.evaluate(() => indexedDB.databases().then((dbs) => Promise.all(dbs.map((db) => new Promise((done) => { const gone = indexedDB.deleteDatabase(db.name!); gone.onsuccess = gone.onerror = gone.onblocked = () => done(null); })))));
  await ui.page.goto(ui.relay.httpUrl);
}

/** Asks for a link on the first screen and returns the URL in the email. */
async function askForLink(): Promise<string> {
  const before = mail.mails.length;

  await unpairedPage();
  await ui.page.getByLabel('Email address').fill('owner@example.test');
  await ui.page.getByRole('button', { name: 'Email me a link' }).click();
  await ui.page.getByText('If that address is set up, a link is on its way.').waitFor({ timeout: 10_000 });

  const sent = await waitFor('the email', () => mail.mails[before], 15_000);
  const link = /http:\/\/localhost:\d+\/m#[A-Za-z0-9_.-]+/.exec(sent.body.text)?.[0];

  expect(link, 'the email carries the sign-in link').toBeTruthy();
  expect(sent.headers.authorization, 'the provider key is in the header').toBe(`Bearer ${KEY}`);
  expect(JSON.stringify(sent.body)).not.toContain(KEY);
  expect(sent.body.to).toEqual(['owner@example.test']);

  return link!;
}

describe('sign in by email link', () => {
  it('the computer reports its address to the relay, and the status shows it', async () => {
    const status = await ui.daemon.call('anywhere.status');

    expect(status.settings.email).toBe('owner@example.test');
    expect(status.settings.escalateEmailSec).toBe(60);
  });

  it('a link in an email takes a new browser through to a confirmed pairing', async () => {
    const link = await askForLink();
    const requests: string[] = [];

    ui.page.on('request', (request) => requests.push(`${request.method()} ${new URL(request.url()).pathname}`));

    // Opening the link shows a button and spends nothing, and the secret leaves the address bar.
    await ui.page.goto(link);
    await ui.page.getByRole('heading', { name: 'Add this browser' }).waitFor({ timeout: 10_000 });
    await new Promise((resolve) => setTimeout(resolve, 500));

    expect(requests.filter((request) => request.includes('/api/magic/redeem'))).toEqual([]);
    expect(new URL(ui.page.url()).hash).toBe('');
    expect(await ui.axe(), 'the sign-in page has no WCAG 2 AA violations').toEqual([]);

    // The button spends it; pairing goes on exactly as with a QR code.
    await ui.page.getByRole('button', { name: 'Continue' }).click();
    await ui.page.getByLabel('Name this device').fill('Phone from a link');
    await ui.page.getByRole('button', { name: 'Create passkey and connect' }).click();

    const code = await waitFor('the code on the page', async () => {
      const text = await ui.page.locator('.code').textContent({ timeout: 500 }).catch(() => null);

      return text && /^\d{3} \d{3}$/.test(text) ? text : null;
    });
    const request = await waitFor('the request on the computer', async () => (await ui.daemon.call('anywhere.pair.requests')).requests[0]);

    // Not trusted yet: the link by itself granted nothing.
    expect(request.code).toBe(code);
    expect(request.guest).toBe(false);
    expect((await ui.daemon.call('anywhere.devices.list')).devices.map((d: any) => d.name)).not.toContain('Phone from a link');

    await ui.daemon.call('anywhere.pair.confirm', { deviceId: request.deviceId, accept: true });
    await ui.page.getByText('Locked').first().waitFor({ timeout: 15_000 });

    expect((await ui.daemon.call('anywhere.devices.list')).devices.map((d: any) => d.name)).toContain('Phone from a link');
  });

  it('the same link cannot be used again', async () => {
    const link = await askForLink();

    // A mail scanner opens it first (a plain GET): nothing is spent.
    const scanned = await fetch(link);

    expect(scanned.status).toBe(200);

    // The computer makes a pairing offer for a link at most once every ten seconds (the previous test just used one).
    await new Promise((resolve) => setTimeout(resolve, 11_000));

    // The person presses the button: this one works.
    await ui.page.goto(link);
    await ui.page.getByRole('button', { name: 'Continue' }).click();
    await ui.page.getByRole('heading', { name: 'Pair this device' }).waitFor({ timeout: 10_000 });

    // Anyone pressing it again, from the same link, is refused.
    const second = await ui.context.newPage();

    await second.goto(link);
    await second.getByRole('button', { name: 'Continue' }).click();
    await second.getByRole('alert').getByText(/already used|expired|not valid/).waitFor({ timeout: 10_000 });
    await second.close();

    // Turn the half-made pairing down so the next test starts clean.
    const pending = (await ui.daemon.call('anywhere.pair.requests')).requests;

    for (const request of pending) await ui.daemon.call('anywhere.pair.confirm', { deviceId: request.deviceId, accept: false });
  });

  it('an address that nobody set up gets the same answer and no email', async () => {
    const before = mail.mails.length;

    await unpairedPage();
    await ui.page.getByLabel('Email address').fill('stranger@example.test');
    await ui.page.getByRole('button', { name: 'Email me a link' }).click();
    await ui.page.getByText('If that address is set up, a link is on its way.').waitFor({ timeout: 10_000 });
    await new Promise((resolve) => setTimeout(resolve, 1500));

    expect(mail.mails.length).toBe(before);
  });

  it('a computer whose owner removed the address refuses even a link that is still valid', async () => {
    const link = await askForLink();

    await ui.daemon.call('anywhere.configure', { email: '' });
    expect((await ui.daemon.call('anywhere.status')).settings.email).toBe('');

    // The link is genuine and unused, but this computer no longer takes part in email sign-in.
    await ui.page.goto(link);
    await ui.page.getByRole('button', { name: 'Continue' }).click();
    await ui.page.getByRole('alert').getByText(/did not accept/).waitFor({ timeout: 15_000 });
    expect((await ui.daemon.call('anywhere.pair.requests')).requests).toEqual([]);

    // And with no address the relay no longer sends links for it at all.
    const before = mail.mails.length;

    await unpairedPage();
    await ui.page.getByLabel('Email address').fill('owner@example.test');
    await ui.page.getByRole('button', { name: 'Email me a link' }).click();
    await ui.page.getByText('If that address is set up, a link is on its way.').waitFor({ timeout: 10_000 });
    await new Promise((resolve) => setTimeout(resolve, 1500));
    expect(mail.mails.length).toBe(before);
  });

  it('a bad address is refused by the computer', async () => {
    await expect(ui.daemon.call('anywhere.configure', { email: 'not an address' })).rejects.toThrow(/email address/);
    await expect(ui.daemon.call('anywhere.configure', { escalateEmailSec: 0 })).rejects.toThrow(/1 to 3600/);
  });
});
