import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { HostView } from './types';

/**
 * *"ami deskvoy.com ar project file access nite cai"* on a VPS that was added by its IP (0.11.5).
 *
 * Neither saved matcher could see it: there is no project called that yet, and the host's address is
 * `109.199.108.216`, so `hostMentionedIn` has no domain to compare. The prompt ran in whatever chat was
 * open and the engine was never given the site's folder - with any model.
 */
const sdcpCall = vi.hoisted(() => vi.fn());

vi.mock('../lib/sdcp', () => ({ sdcpCall }));

const { domainMentionedIn, sendPrompt } = await import('./intents');
const { useAppStore } = await import('./store');
const { usePrefsStore } = await import('./prefs');

describe('domainMentionedIn', () => {
  it('finds the site a sentence names', () => {
    expect(domainMentionedIn('ami deskvoy.com ar project file access nite cai')).toBe('deskvoy.com');
    expect(domainMentionedIn('open https://shop.deskvoy.com.bd/admin please')).toBe('shop.deskvoy.com.bd');
    expect(domainMentionedIn('fix the header on DeskVoy.com.')).toBe('deskvoy.com');
  });

  it('never takes a file, an address or a version for a site', () => {
    expect(domainMentionedIn('update index.php and app.tsx')).toBeNull();
    expect(domainMentionedIn('mail me at web@example')).toBeNull();
    expect(domainMentionedIn('bump to 0.11.5')).toBeNull();
    expect(domainMentionedIn('connect to 109.199.108.216')).toBeNull();
    expect(domainMentionedIn('just chatting')).toBeNull();
  });
});

describe('a prompt that names a site on a VPS added by its IP', () => {
  const vps = (sessions: HostView['sessions']): HostView =>
    ({
      id: 'h1',
      name: 'mehedi@109.199.108.216',
      type: 'vps',
      status: 'connected',
      sdcd: '0.11.5',
      platform: '',
      detail: '',
      hostKey: '',
      address: 'mehedi@109.199.108.216:8443',
      pinned: '',
      sessions,
    }) as HostView;

  const chat = (id: string, projectRoot: string | null) => ({
    id,
    title: 'New chat',
    prompt: '',
    state: 'idle' as const,
    minutesAgo: 1,
    unread: 0,
    projectId: projectRoot === null ? null : 'p-other',
    projectRoot,
  });

  beforeEach(() => {
    sdcpCall.mockReset();
    sdcpCall.mockImplementation(async (method: string) => {
      switch (method) {
        case 'project.locate':
          return { candidates: [{ root: '/var/www/deskvoy.com/public_html', source: 'web server config' }] };
        case 'project.add':
          return { projectId: 'p-desk', hostId: 'h1', root: '/var/www/deskvoy.com/public_html', name: 'deskvoy.com' };
        case 'project.list':
          return { projects: [] };
        case 'session.open':
          return { sessionId: 's-desk' };
        case 'engine.start':
          return { turnId: 't1' };
        default:
          return {};
      }
    });
    useAppStore.setState({ projects: [] });
  });

  it('binds a folderless chat on that VPS to the site the machine serves', async () => {
    useAppStore.setState({ hosts: [vps([chat('s1', null)])] });
    usePrefsStore.setState({ activeTab: 's1', openTabs: ['s1'] });

    await sendPrompt('ami deskvoy.com ar project file access nite cai');

    expect(sdcpCall).toHaveBeenCalledWith('project.locate', { hostId: 'h1', query: 'deskvoy.com' });
    expect(sdcpCall).toHaveBeenCalledWith('project.add', { hostId: 'h1', root: '/var/www/deskvoy.com/public_html' });
    expect(sdcpCall).toHaveBeenCalledWith('session.update', { sessionId: 's1', projectId: 'p-desk' });
    expect(sdcpCall).toHaveBeenCalledWith('engine.start', expect.objectContaining({ sessionId: 's1' }));
  });

  it('gives the site a chat of its own when the open chat is working on another folder', async () => {
    useAppStore.setState({ hosts: [vps([chat('s1', '/var/www/other.com')])] });
    usePrefsStore.setState({ activeTab: 's1', openTabs: ['s1'] });

    await sendPrompt('now work on deskvoy.com');

    /* Saved under the domain, so the sidebar shows it by that name and the next prompt routes by it. */
    expect(sdcpCall).toHaveBeenCalledWith('project.add', {
      hostId: 'h1',
      root: '/var/www/deskvoy.com/public_html',
      name: 'deskvoy.com',
    });
    expect(sdcpCall).toHaveBeenCalledWith('session.open', expect.objectContaining({ hostId: 'h1', projectId: 'p-desk' }));
    expect(sdcpCall).toHaveBeenCalledWith('engine.start', expect.objectContaining({ sessionId: 's-desk' }));
  });

  it('runs where it was typed when no machine serves it', async () => {
    sdcpCall.mockImplementation(async (method: string) =>
      method === 'project.locate' ? { candidates: [] } : method === 'engine.start' ? { turnId: 't1' } : {},
    );
    useAppStore.setState({ hosts: [vps([chat('s1', null)])] });
    usePrefsStore.setState({ activeTab: 's1', openTabs: ['s1'] });

    await sendPrompt('what is example.org?');

    expect(sdcpCall).not.toHaveBeenCalledWith('project.add', expect.anything());
    expect(sdcpCall).toHaveBeenCalledWith('engine.start', expect.objectContaining({ sessionId: 's1' }));
  });
});
