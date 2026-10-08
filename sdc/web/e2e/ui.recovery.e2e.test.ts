// The user's actual problem, in a real browser with a virtual platform authenticator that has the PRF extension: pair the page,
// wipe everything the site stored (what "clear browsing data" does), and get back in with the passkey alone.

import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { startUi, type Ui } from './uikit';

let ui: Ui;

beforeAll(async () => {
  ui = await startUi();
  await ui.pairPage('Phone that will be cleared');
}, 240_000);

afterAll(async () => {
  await ui?.stop();
});

async function wipeSiteData(): Promise<void> {
  await ui.page.evaluate(async () => {
    for (const db of await indexedDB.databases()) {
      await new Promise((done) => {
        const gone = indexedDB.deleteDatabase(db.name!);

        gone.onsuccess = gone.onerror = gone.onblocked = () => done(null);
      });
    }

    localStorage.clear();
    sessionStorage.clear();
  });
}

describe('clearing the browser data', () => {
  it('says on the More tab that this pairing is saved for the passkey', async () => {
    await ui.page.getByRole('button', { name: 'More' }).click();
    await ui.page.getByText(/Saved for your passkey/).waitFor({ timeout: 10_000 });
  });

  it('after the data is gone, "Rejoin with my passkey" brings the phone back without the computer being asked', async () => {
    const devices = (await ui.daemon.call('anywhere.devices.list')).devices.length;

    // The saved copy is uploaded when the page connects; give the relay a moment, then wipe like "clear site data".
    await new Promise((resolve) => setTimeout(resolve, 1500));
    await wipeSiteData();
    await ui.page.goto(ui.relay.httpUrl);
    await ui.page.getByRole('heading', { name: 'Pair this browser' }).waitFor({ timeout: 15_000 });
    expect(await ui.axe(), 'the first screen with the rejoin panel has no WCAG 2 AA violations').toEqual([]);

    await ui.page.getByRole('button', { name: 'Rejoin with my passkey' }).click();
    await ui.page.getByText('Locked').first().waitFor({ timeout: 20_000 });

    expect((await ui.daemon.call('anywhere.pair.requests')).requests).toEqual([]);
    expect((await ui.daemon.call('anywhere.devices.list')).devices).toHaveLength(devices);

    // It is the same device, and its passkey still unlocks the session.
    await ui.page.getByRole('button', { name: 'Unlock with passkey' }).click();
    await ui.page.getByText('Viewing').first().waitFor({ timeout: 15_000 });
    expect(ui.errors).toEqual([]);
  });

  it('a phone removed on the computer is told there is nothing to restore', async () => {
    const { devices } = await ui.daemon.call('anywhere.devices.list');

    for (const device of devices) await ui.daemon.call('anywhere.devices.revoke', { deviceId: device.id });

    await new Promise((resolve) => setTimeout(resolve, 800));
    await wipeSiteData();
    await ui.page.goto(ui.relay.httpUrl);
    await ui.page.getByRole('button', { name: 'Rejoin with my passkey' }).click();
    await ui.page.getByRole('alert').getByText(/Nothing was saved|removed on your computer/).waitFor({ timeout: 20_000 });
  });
});
