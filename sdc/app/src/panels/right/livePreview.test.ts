import { describe, expect, it } from 'vitest';

import type { TurnView } from '../../store/types';
import { domainIn, joinPage, lastChange, localPort, pageChanges, pageForFile, previewCandidates } from './livePreview';

function turn(overrides: Partial<TurnView>): TurnView {
  return {
    id: 't1',
    turnNumber: 1,
    sessionId: 's1',
    engine: 'native_api',
    model: 'qwen3.8-max',
    tier: 'Balanced',
    prompt: '',
    text: '',
    thinking: '',
    thinkingMs: 0,
    thinkingSince: null,
    plan: [],
    startedAt: '2026-09-28T10:00:00Z',
    status: 'done',
    stuckForMs: 0,
    tools: [],
    timeline: [],
    summary: '',
    meta: '',
    pass: null,
    ...overrides,
  };
}

const run = (output: string[], target = 'npm run dev') => ({
  callId: 'c1',
  startedAt: '2026-09-28T10:00:01Z',
  tool: 'run' as const,
  name: 'Run',
  target,
  status: 'done' as const,
  meta: '',
  diff: [],
  output: output.map((text) => ({ level: 'dim' as const, text })),
});

describe('live preview', () => {
  it('finds the dev server a chat started, and a site by its domain', () => {
    const turns = [turn({ tools: [run(['  VITE v6  ready', '  ➜  Local:   http://localhost:5173/', '  ➜  Network: http://0.0.0.0:5173/'])] })];
    const project = { id: 'pr1', hostId: 'h1', root: '/var/www/example-shop.com', name: 'example-shop.com', chats: 1 };

    expect(previewCandidates(turns, 's1', project)).toEqual(['http://localhost:5173/', 'https://example-shop.com/']);
    expect(previewCandidates(turns, 'other', undefined)).toEqual([]);
  });

  it('reloads after the newest finished edit', () => {
    const edit = { ...run([], 'src/App.tsx'), callId: 'c2', tool: 'edit' as const, name: 'Edit' };

    expect(lastChange([turn({ tools: [run([]), edit] })], 's1')).toEqual({ key: 't1/c2', target: 'src/App.tsx' });
    expect(lastChange([turn({ tools: [run([])] })], 's1')).toBeNull();
  });
});

describe('the page a change is on (0.14.4)', () => {
  it('maps the framework files to the page they serve', () => {
    /* The report's own file: a new page on the live VPS site. */
    expect(pageForFile('/var/www/skilleddesk.com/public_html/src/app/(public)/email-marketing-agency-usa/page.tsx')).toBe(
      '/email-marketing-agency-usa',
    );
    expect(pageForFile('src/app/page.tsx')).toBe('/');
    expect(pageForFile('app/blog/[slug]/page.tsx')).toBe('/blog');
    expect(pageForFile('src/routes/about/+page.svelte')).toBe('/about');
    expect(pageForFile('pages/index.vue')).toBe('/');
    expect(pageForFile('src/pages/docs/intro.astro')).toBe('/docs/intro');
    expect(pageForFile('pages/api/hello.ts')).toBeNull();
    expect(pageForFile('wp-content/themes/shop/page-contact.php')).toBe('/contact/');
    expect(pageForFile('/var/www/example.com/public_html/about.php')).toBe('/about.php');
    expect(pageForFile('/var/www/example.com/public_html/blog/index.html')).toBe('/blog/');
    expect(pageForFile('H:\\site\\about.html', 'H:\\site')).toBe('/about.html');
  });

  it('keeps the page for a file that is not one', () => {
    expect(pageForFile('src/components/Header.tsx')).toBeNull();
    expect(pageForFile('src/app/globals.css')).toBeNull();
    expect(pageForFile('/var/www/example.com/public_html/includes/header.php')).toBeNull();
    expect(pageForFile('H:\\elsewhere\\about.html', 'H:\\site')).toBeNull();
  });

  it('reads the domain from the path, not from a file name', () => {
    expect(domainIn('/var/www/www.skilleddesk.com/public_html/src/app/page.tsx')).toBe('skilleddesk.com');
    expect(domainIn('/home/me/project/index.html')).toBeNull();
  });

  it('finds the site of the edits, and the newest edit first', () => {
    const edit = (callId: string, target: string) => ({ ...run([], target), callId, tool: 'edit' as const, name: 'Edit' });
    const turns = [
      turn({ tools: [run(['ready on http://127.0.0.1:3000']), edit('c2', '/var/www/example.com/public_html/src/app/pricing/page.tsx'), edit('c3', '/var/www/example.com/public_html/src/app/globals.css')] }),
    ];
    const project = { id: 'pr1', hostId: 'h1', root: '/home/me/SDC Workspaces/chat', name: 'chat', chats: 1 };

    expect(previewCandidates(turns, 's1', project)).toEqual(['http://localhost:3000', 'https://example.com/']);
    expect(pageChanges(turns, 's1').map((change) => change.page)).toEqual([null, '/pricing']);
    expect(localPort('http://127.0.0.1:3000')).toBe(3000);
    expect(localPort('https://example.com/')).toBeNull();
    expect(joinPage('https://example.com/', '/pricing')).toBe('https://example.com/pricing');
  });
});
