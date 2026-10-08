// The service worker is plain JS served as /sw.js. It is run here in a sandbox with a fake `self`, so what it does with a hostile
// or odd push is checked without a browser.

import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { describe, expect, it } from 'vitest';

const source = readFileSync(new URL('../public/sw.js', import.meta.url), 'utf8');
const ORIGIN = 'https://sdc.skilleddesk.com';

function load(languages: string[] = ['en-US']) {
  const handlers: Record<string, (event: any) => void> = {};
  const shown: Array<{ title: string; options: any }> = [];
  const opened: string[] = [];
  const posted: any[] = [];
  let windows: any[] = [];
  const self = {
    addEventListener: (type: string, handler: (event: any) => void) => (handlers[type] = handler),
    skipWaiting: () => undefined,
    clients: {
      claim: async () => undefined,
      matchAll: async () => windows,
      openWindow: async (path: string) => void opened.push(path),
    },
    registration: { showNotification: async (title: string, options: any) => void shown.push({ title, options }) },
    location: { origin: ORIGIN },
    navigator: { languages },
  };

  runInNewContext(source, { self, URL });

  return {
    shown,
    opened,
    posted,
    setWindows: (list: Array<{ url: string }>) => (windows = list.map((w) => ({ ...w, focus: async () => 'focused', postMessage: (m: any) => posted.push(m) }))),
    async push(data: unknown) {
      const waits: Promise<unknown>[] = [];

      handlers.push!({ data: data === null ? null : { json: () => (typeof data === 'string' ? JSON.parse(data) : data) }, waitUntil: (p: Promise<unknown>) => waits.push(p) });
      await Promise.all(waits);
    },
    async click(url: unknown) {
      const waits: Promise<unknown>[] = [];
      let closed = false;

      handlers.notificationclick!({ notification: { close: () => (closed = true), data: url === undefined ? undefined : { url } }, waitUntil: (p: Promise<unknown>) => waits.push(p) });
      await Promise.all(waits);

      return closed;
    },
  };
}

describe('showing a push', () => {
  it('shows the plain wording for an approval and points at its page', async () => {
    const sw = load();

    await sw.push({ t: 'approval', url: '/a/apr_perm-turn-7-1' });

    expect(sw.shown).toHaveLength(1);
    expect(sw.shown[0]!.title).toBe('SDC needs you');
    expect(sw.shown[0]!.options.data).toEqual({ url: '/a/apr_perm-turn-7-1' });
    expect(sw.shown[0]!.options.tag).toBe('sdc/apr_perm-turn-7-1');
    // It says nothing about what is being asked.
    expect(JSON.stringify(sw.shown[0])).not.toMatch(/rm |sudo|file|folder|\.env|password/i);
  });

  it('uses Bangla when the browser prefers it', async () => {
    const sw = load(['bn-BD', 'en']);

    await sw.push({ t: 'approval', url: '/a/x' });
    expect(sw.shown[0]!.title).toBe('SDC-র আপনাকে দরকার');
  });

  it('always shows something, as browsers require, even for a payload that is empty or broken', async () => {
    const sw = load();

    await sw.push(null);
    await sw.push('not json {{');
    await sw.push({ t: 'approval' });

    expect(sw.shown).toHaveLength(3);
    expect(sw.shown.every((item) => item.options.data.url === '/' && item.title === 'SDC needs you')).toBe(true);
  });

  it('does not follow a link in a push that is not one of our request pages', async () => {
    const sw = load();

    for (const url of ['https://evil.example/phish', '//evil.example/x', '/a/../../admin', '/a/..', '/a/', '/a/x y', 'javascript:alert(1)', '/other', `/a/${'x'.repeat(200)}`, 5]) {
      await sw.push({ t: 'approval', url });
    }

    expect(sw.shown.every((item) => item.options.data.url === '/')).toBe(true);
  });

  it('ignores text a push adds: only the fixed wording is shown', async () => {
    const sw = load();

    await sw.push({ t: 'approval', url: '/a/x', title: 'Your bank', body: 'Click here' });

    expect(sw.shown[0]!.title).toBe('SDC needs you');
    expect(sw.shown[0]!.options.body).not.toContain('bank');
  });
});

describe('tapping the notification', () => {
  it('opens the request page when no window is open', async () => {
    const sw = load();

    expect(await sw.click('/a/apr_x')).toBe(true);
    expect(sw.opened).toEqual(['/a/apr_x']);
  });

  it('brings an open page forward and tells it where to go', async () => {
    const sw = load();

    sw.setWindows([{ url: `${ORIGIN}/` }]);
    await sw.click('/a/apr_x');

    expect(sw.opened).toEqual([]);
    expect(sw.posted).toEqual([{ t: 'open', url: '/a/apr_x' }]);
  });

  it('never opens another site, whatever the notification data says', async () => {
    const sw = load();

    for (const url of ['https://evil.example/x', '//evil.example', 'javascript:alert(1)', undefined, '', 5]) await sw.click(url);

    expect(sw.opened.every((path) => path.startsWith('/') && !path.startsWith('//'))).toBe(true);
    expect(sw.opened.filter((path) => path !== '/')).toEqual([]);
  });

  it('ignores a window of another origin', async () => {
    const sw = load();

    sw.setWindows([{ url: 'https://evil.example/' }]);
    await sw.click('/a/x');

    expect(sw.posted).toEqual([]);
    expect(sw.opened).toEqual(['/a/x']);
  });
});

describe('what it does not do', () => {
  it('has no fetch handler and no cache', () => {
    expect(source).not.toMatch(/addEventListener\(\s*['"]fetch['"]/);
    expect(source).not.toMatch(/caches\./);
  });
});
