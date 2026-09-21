import { beforeEach, describe, expect, it, vi } from 'vitest';

import { SdcpCallError } from '../lib/transport';
import type { SessionView } from './types';

/**
 * `chooseModel`'s acceptance test - the regression behind *"connect hoy claude but chat e kisu likhle kaj
 * hoy nah"*.
 *
 * The intent writes two places, and only one of them used to exist: `models.select` tells the daemon
 * which model the setting is, and the window's own model store has to move with it, because `sendPrompt`
 * reads the store. Pressing `Use` on a row in the Connect dialog therefore changed the daemon's setting,
 * showed `In use` on that row, and left the chat running the model it already had.
 *
 * The daemon is mocked rather than stood in for: this file is about what the *window* does with the
 * answer, and the stand-in refuses `models.select` on purpose (a browser has no daemon settings to
 * write).
 */
const sdcpCall = vi.hoisted(() => vi.fn());

vi.mock('../lib/sdcp', () => ({ sdcpCall }));

const { chooseModel, closeFolder, forkSession, loadCliRecipe, emptySessionOn, newChatOnHost, openFolderIn, loadDirectory, toggleDirectory, openFile, closeFile } = await import('./intents');
const { engineForProvider, useModelStore } = await import('./model');
const { useFilesStore } = await import('./files');
const { useLayoutStore } = await import('./layout');
const { usePrefsStore } = await import('./prefs');
const { useRightPanelStore } = await import('./rightPanel');
const { useAppStore } = await import('./store');

describe('chooseModel', () => {
  beforeEach(() => {
    sdcpCall.mockReset();
    sdcpCall.mockResolvedValue({});
    useModelStore.setState({
      tier: 'balanced',
      engine: 'claude_code',
      model: 'sonnet',
      providerId: 'claude',
      catalog: [
        { id: 'deepseek-v4-pro', providerId: 'deepseek', providerLabel: 'DeepSeek', tier: 'balanced', ctx: 0, cost: '', name: 'DeepSeek V4 Pro', source: 'cache' },
        { id: 'deepseek-flash', providerId: 'deepseek', providerLabel: 'DeepSeek', tier: 'fast', ctx: 0, cost: '', name: 'DeepSeek Flash', source: 'cache' },
      ],
    });
  });

  it('tells the daemon and moves the window', async () => {
    await expect(chooseModel('deepseek-v4-pro', 'deepseek')).resolves.toBe(true);

    expect(sdcpCall).toHaveBeenCalledWith('models.select', { modelId: 'deepseek-v4-pro', providerId: 'deepseek' });

    const state = useModelStore.getState();

    /* The window's four facts, which are what `sendPrompt` sends: without the last three the chat kept
       running claude_code/sonnet while the dialog said `In use`. */
    expect(state.model).toBe('deepseek-v4-pro');
    expect(state.providerId).toBe('deepseek');
    expect(state.engine).toBe(engineForProvider('deepseek'));
    expect(state.tier).toBe('balanced');
  });

  it('takes the tier from the row the catalogue listed', async () => {
    await chooseModel('deepseek-flash', 'deepseek');

    expect(useModelStore.getState().tier).toBe('fast');
    expect(useModelStore.getState().model).toBe('deepseek-flash');
  });

  it('leaves the window alone when the daemon refuses', async () => {
    sdcpCall.mockRejectedValue(new Error('unsupported'));

    await expect(chooseModel('deepseek-v4-pro', 'deepseek')).resolves.toBe(false);

    /* A model the daemon did not accept must not be the model the chat claims to be on. */
    expect(useModelStore.getState().model).toBe('sonnet');
    expect(useModelStore.getState().engine).toBe('claude_code');
  });

  it('resolves each provider to its own engine', () => {
    expect(engineForProvider('claude')).toBe('claude_code');
    expect(engineForProvider('openai')).toBe('codex');
    expect(engineForProvider('gemini')).toBe('gemini');
    expect(engineForProvider('deepseek')).toBe('native_api');
  });
});

/**
 * `+ New chat` - the regression behind *"bar bar new open korle onk chat open hoi"*.
 *
 * The row is created by the daemon (`session.open`) and the rule about *whether* to create one lives
 * in the window, so both halves are checked here: which chat the click lands on, and whether the
 * daemon was asked at all.
 */
describe('emptySessionOn', () => {
  const host = (sessions: string[]) => [
    {
      id: 'local',
      name: 'Local',
      type: 'local' as const,
      status: 'connected' as const,
      sdcd: '0.7.5',
      platform: 'Windows 11 · x64',
      sessions: sessions.map((id) => ({
        id,
        title: 'New chat',
        prompt: '',
        state: 'idle' as const,
        minutesAgo: 1,
        unread: 0,
      })),
    },
  ];
  const turns = (...ids: string[]) => ids.map((sessionId) => ({ sessionId }));

  it('lands on the host\'s empty chat instead of minting another one', () => {
    expect(emptySessionOn(host(['s1']), turns(), 'local', null)).toBe('s1');
    expect(emptySessionOn(host(['s1', 's2']), turns(), 'local', null)).toBe('s2');
  });

  it('prefers the chat the caret is already in', () => {
    expect(emptySessionOn(host(['s1', 's2']), turns(), 'local', 's1')).toBe('s1');
  });

  it('will not land on a chat that has run a turn', () => {
    expect(emptySessionOn(host(['s1', 's2']), turns('s2'), 'local', null)).toBe('s1');
    expect(emptySessionOn(host(['s1']), turns('s1'), 'local', null)).toBeNull();
  });

  it('answers nothing for a host that is not in the tree', () => {
    expect(emptySessionOn(host(['s1']), turns(), 'vps-1', null)).toBeNull();
  });
});

describe('newChatOnHost', () => {
  beforeEach(() => {
    sdcpCall.mockReset();
    sdcpCall.mockResolvedValue({ sessionId: 's9' });
    useAppStore.setState({
      hosts: [
        {
          id: 'local',
          name: 'Local',
          type: 'local',
          status: 'connected',
          sdcd: '0.7.5',
          platform: 'Windows 11 · x64',
          sessions: [{ id: 's1', title: 'New chat', prompt: '', state: 'idle', minutesAgo: 1, unread: 0 }],
        },
      ],
      turns: [],
    });
  });

  it('does not ask the daemon for a chat the window already has empty', async () => {
    await expect(newChatOnHost('local')).resolves.toBe('s1');

    /* Five clicks on `+ New chat` are one row, not five: the second click finds `s1` still empty. */
    expect(sdcpCall).not.toHaveBeenCalled();
  });

  it('asks the daemon when the host has no chat to reuse', async () => {
    /* A fresh host: nothing on it is empty *or* used, so this is the case `session.open` exists for. */
    useAppStore.setState({
      hosts: [
        {
          id: 'local',
          name: 'Local',
          type: 'local',
          status: 'connected',
          sdcd: '0.7.5',
          platform: 'Windows 11 · x64',
          sessions: [],
        },
      ],
    });

    await expect(newChatOnHost('local')).resolves.toBe('s9');
    expect(sdcpCall).toHaveBeenCalledWith('session.open', {
      hostId: 'local',
      title: 'New chat',
      prompt: 'Describe what you want to build…',
    });
  });
});

/**
 * `Open folder` (0.7.6) - the window's half of *"chat kono folder e kaj kore na"*.
 *
 * `openFolderIn` is the part after the dialog answers, and it is tested rather than `openFolder` because
 * the native dialog cannot be opened in a unit test (there is no window) while the three decisions it
 * leads to can be asserted exactly: the daemon validates the path, the host's *empty* chat is re-pointed
 * rather than a second row being left behind, and a folder the daemon refuses is reported instead of a
 * chat being opened on nothing.
 */
describe('openFolderIn', () => {
  const host = (sessions: SessionView[]) => [
    {
      id: 'local',
      name: 'Local',
      type: 'local' as const,
      status: 'connected' as const,
      sdcd: '0.7.6',
      platform: 'Windows 11 · x64',
      sessions,
    },
  ];

  const daemon = (methods: Record<string, unknown>): void => {
    sdcpCall.mockImplementation((method: string) =>
      method in methods ? Promise.resolve(methods[method]) : Promise.resolve({}),
    );
  };

  const added = {
    'project.add': { projectId: 'pr1', hostId: 'local', root: 'H:\\SDC', name: 'SDC' },
    'project.list': {
      projects: [{ projectId: 'pr1', hostId: 'local', root: 'H:\\SDC', name: 'SDC', chats: 0 }],
    },
  };

  beforeEach(() => {
    sdcpCall.mockReset();
    usePrefsStore.setState({ activeTab: null, openTabs: [] });
    useAppStore.setState({ hosts: host([]), turns: [], projects: [] });
  });

  it('opens the folder, then a chat in it, and says so', async () => {
    daemon({ ...added, 'session.open': { sessionId: 'n9' } });

    await expect(openFolderIn('H:\\SDC')).resolves.toBe('n9');

    /* The path goes to the daemon, which is the only thing that can say whether it is a folder. */
    expect(sdcpCall).toHaveBeenCalledWith('project.add', { hostId: 'local', root: 'H:\\SDC' });

    /* The chat is opened *with* the folder, so its first event carries it and the chip is right. */
    expect(sdcpCall).toHaveBeenCalledWith('session.open', {
      hostId: 'local',
      projectId: 'pr1',
      title: 'SDC',
      prompt: 'Describe what you want to build…',
    });

    /* The rows the daemon answered are folded, which is what turns the `No project` empty state into the
       ordinary one. */
    expect(useAppStore.getState().projects.map((project) => project.id)).toEqual(['pr1']);

    /* And the caret lands in the chat that now has the folder. */
    expect(usePrefsStore.getState().activeTab).toBe('n9');
  });

  it('re-points the empty chat instead of leaving a second row behind', async () => {
    useAppStore.setState({
      hosts: host([{ id: 's1', title: 'New chat', prompt: '', state: 'idle', minutesAgo: 1, unread: 0 }]),
    });
    daemon(added);

    await expect(openFolderIn('H:\\SDC')).resolves.toBe('s1');

    expect(sdcpCall).toHaveBeenCalledWith('session.update', { sessionId: 's1', projectId: 'pr1' });
    expect(sdcpCall).not.toHaveBeenCalledWith('session.open', expect.anything());
    expect(usePrefsStore.getState().activeTab).toBe('s1');
  });

  it('reports a folder the daemon refuses, and opens no chat', async () => {
    sdcpCall.mockImplementation((method: string) =>
      method === 'project.add'
        ? Promise.reject(new Error('`H:\\nope` is not a folder'))
        : Promise.resolve({}),
    );

    await expect(openFolderIn('H:\\nope')).resolves.toBeNull();
    expect(sdcpCall).not.toHaveBeenCalledWith('session.open', expect.anything());
    expect(sdcpCall).not.toHaveBeenCalledWith('session.update', expect.anything());
  });

  it('will not land on a chat that has already run a turn', async () => {
    /* The used chat is not a place to put a folder: `emptySessionOn` says so, so a new chat is opened. */
    useAppStore.setState({
      hosts: host([
        { id: 's1', title: 'Login bug', prompt: 'fix it', state: 'success', minutesAgo: 9, unread: 0 },
      ]),
      turns: [
        {
          id: 't1',
          sessionId: 's1',
          turnNumber: 1,
          engine: 'claude_code',
          model: 'sonnet',
          tier: 'Balanced',
          prompt: 'fix it',
          text: '',
          thinking: '',
          status: 'done',
          stuckForMs: 0,
          tools: [],
          summary: '',
          meta: '',
          pass: true,
        },
      ],
    });
    daemon({ ...added, 'session.open': { sessionId: 'n9' } });

    await expect(openFolderIn('H:\\SDC')).resolves.toBe('n9');
    expect(sdcpCall).toHaveBeenCalledWith('session.open', expect.objectContaining({ projectId: 'pr1' }));
  });
});

/**
 * The file tree (0.7.7) - `fs.list` and `fs.read`, which had no caller in the app until this release.
 *
 * Four behaviours are asserted, and each one is a decision rather than a detail:
 *
 *   1. the root read names **no path** - the session id is enough, which is the contract 0.7.6 gave the
 *      file tools, and the daemon's answer is what the tree shows as its root;
 *   2. opening a folder reads it once (expanding, collapsing and expanding again is not three requests);
 *   3. clicking a file fills the Preview with the daemon's text, and - the part a click that looks dead
 *      would fail - switches the right panel to Preview **and** unfolds it;
 *   4. a failed read puts the daemon's own sentence on screen and leaves the tree alone.
 */
describe('the file tree', () => {
  const listing = (path: string) => ({
    path,
    entries: [
      { name: 'src', path: `${path}\\src`, dir: true, size: 0 },
      { name: 'README.md', path: `${path}\\README.md`, dir: false, size: 42 },
    ],
    /* The `.env` in that folder is *not* in this array: the daemon filters it out and counts it. */
    hidden: 1,
  });

  beforeEach(() => {
    sdcpCall.mockReset();
    useFilesStore.getState().reset();
    usePrefsStore.setState({ activeTab: 's1', openTabs: ['s1'] });
  });

  it('reads the chat’s folder when no path is given, and takes the root from the answer', async () => {
    sdcpCall.mockResolvedValue(listing('H:\\SDC'));

    await loadDirectory(null);

    expect(sdcpCall).toHaveBeenCalledWith('fs.list', { sessionId: 's1' });

    const files = useFilesStore.getState();

    expect(files.root).toBe('H:\\SDC');
    expect(files.directories['H:\\SDC']?.entries.map((entry) => entry.name)).toEqual(['src', 'README.md']);
    /* The guard's count travels: the tree says `1 name hidden` rather than being quietly short. */
    expect(files.directories['H:\\SDC']?.hidden).toBe(1);
  });

  it('reads a folder once, however many times it is expanded', async () => {
    sdcpCall.mockResolvedValue(listing('H:\\SDC\\src'));

    await toggleDirectory('H:\\SDC\\src');
    expect(useFilesStore.getState().expanded).toEqual(['H:\\SDC\\src']);

    /* Collapsing reads nothing... */
    await toggleDirectory('H:\\SDC\\src');
    expect(useFilesStore.getState().expanded).toEqual([]);

    /* ...and opening it again does not re-ask: the entries are already in the store. */
    await toggleDirectory('H:\\SDC\\src');

    expect(sdcpCall).toHaveBeenCalledTimes(1);
    expect(sdcpCall).toHaveBeenCalledWith('fs.list', { path: 'H:\\SDC\\src' });
  });

  it('opens a file into the Preview, and unfolds the panel so the click is visible', async () => {
    useLayoutStore.setState({ right: 'hidden' });
    useRightPanelStore.setState({ activeTab: 'console', tabBySession: {} });
    sdcpCall.mockResolvedValue({
      path: 'H:\\SDC\\README.md',
      text: '# SDC\n',
      sha256: 'a'.repeat(64),
      bytes: 6,
      truncated: false,
    });

    await openFile('H:\\SDC\\README.md', 'README.md');

    const files = useFilesStore.getState();

    expect(files.open).toMatchObject({ name: 'README.md', text: '# SDC\n', truncated: false });
    expect(files.opening).toBeNull();
    expect(useLayoutStore.getState().right).toBe('visible');
    expect(useRightPanelStore.getState().tabBySession.s1).toBe('preview');

    closeFile();
    expect(useFilesStore.getState().open).toBeNull();
  });

  it('shows the daemon’s sentence when a read fails, and keeps the tree', async () => {
    sdcpCall.mockResolvedValue(listing('H:\\SDC'));
    await loadDirectory(null);

    /* The daemon's refusal, in the shape the transport really throws it (`blocked_path`): the tree has to
       show the sentence rather than its own generic one. */
    sdcpCall.mockRejectedValueOnce(
      new SdcpCallError({
        code: 'blocked_path',
        message: 'H:\\SDC\\.env was refused because `.env` holds secrets',
      }),
    );
    await openFile('H:\\SDC\\.env', '.env');

    const files = useFilesStore.getState();

    expect(files.open).toBeNull();
    expect(files.error).toContain('holds secrets');
    expect(files.directories['H:\\SDC']?.entries).toHaveLength(2);
  });
});

/**
 * `cli.recipes` (0.7.8) - the row that says "install `claude` first" *before* a sign-in starts.
 *
 * The daemon has answered this method since 0.7.0 and nothing in the app read it, so Connect on a provider
 * whose CLI was missing started the login and then reported the failure. The intent is what the dialog calls;
 * it picks the provider's own recipe out of the list and answers `null` for a provider that has none, which
 * is how an API-key provider is told apart from a subscription one.
 */
describe('loadCliRecipe', () => {
  const recipes = {
    recipes: [
      { providerId: 'claude', label: 'Claude', program: 'claude', note: 'npm i -g @anthropic-ai/claude-code', installed: true },
      { providerId: 'codex', label: 'Codex', program: 'codex', note: 'npm i -g @openai/codex', installed: false },
    ],
  };

  beforeEach(() => {
    sdcpCall.mockReset();
    sdcpCall.mockResolvedValue(recipes);
  });

  it('answers the provider’s own recipe, with the daemon’s install words', async () => {
    await expect(loadCliRecipe('codex')).resolves.toMatchObject({
      program: 'codex',
      installed: false,
      note: 'npm i -g @openai/codex',
    });
    expect(sdcpCall).toHaveBeenCalledWith('cli.recipes', {});
  });

  it('answers null for a provider with no CLI sign-in', async () => {
    await expect(loadCliRecipe('deepseek')).resolves.toBeNull();
  });

  it('answers null instead of failing when the daemon is unreachable', async () => {
    sdcpCall.mockRejectedValueOnce(new SdcpCallError({ code: 'not_ready', message: 'not connected' }));

    await expect(loadCliRecipe('claude')).resolves.toBeNull();
  });
});

/**
 * `session.fork` (0.7.8) - a method the schema declared and the daemon answered `unknown method` for.
 *
 * The window's half is two lines of intent, and both are worth asserting: the call carries the chat's id and
 * nothing else (the daemon derives the title, so the two cannot disagree), and the answer's `sessionId` is
 * what the caller opens a tab on.
 */
describe('forkSession', () => {
  beforeEach(() => {
    sdcpCall.mockReset();
  });

  it('asks the daemon to fork the chat and answers the new id', async () => {
    sdcpCall.mockResolvedValue({ sessionId: 'n9', turns: 3, title: 'Login bug (fork)' });

    await expect(forkSession('s1', 'Login bug')).resolves.toBe('n9');
    expect(sdcpCall).toHaveBeenCalledWith('session.fork', { sessionId: 's1' });
  });

  it('answers null when the daemon refuses, so no tab is opened on nothing', async () => {
    sdcpCall.mockRejectedValueOnce(
      new SdcpCallError({ code: 'not_found', message: 'no session `s1`' }),
    );

    await expect(forkSession('s1', 'Login bug')).resolves.toBeNull();
  });
});

/**
 * `project.remove` - closing a folder is not closing a chat (0.7.6).
 *
 * The daemon unbinds every chat that pointed at the folder and leaves the conversations alone; the window
 * re-reads both lists, because the rows it holds still name a folder that no longer exists.
 */
describe('closeFolder', () => {
  beforeEach(() => {
    sdcpCall.mockReset();
    usePrefsStore.setState({ activeTab: null, openTabs: [] });
    useAppStore.setState({ hosts: [], turns: [], projects: [] });
  });

  it('re-reads the workspace so the chats stop claiming a folder that is gone', async () => {
    sdcpCall.mockImplementation((method: string) => {
      if (method === 'project.remove') {
        return Promise.resolve({ removed: true, chats: 2 });
      }

      if (method === 'session.list') {
        return Promise.resolve({
          hosts: [
            {
              hostId: 'local',
              name: 'Local',
              hostType: 'local',
              status: 'connected',
              platform: null,
              target: null,
              sessions: [
                {
                  sessionId: 'n1',
                  hostId: 'local',
                  title: 'SDC',
                  prompt: '',
                  state: 'idle',
                  unread: 0,
                  minutesAgo: 1,
                  projectId: null,
                  projectRoot: null,
                },
              ],
            },
          ],
        });
      }

      return Promise.resolve({ projects: [] });
    });

    await expect(closeFolder('pr1', 'SDC')).resolves.toBe(true);

    expect(sdcpCall).toHaveBeenCalledWith('project.remove', { projectId: 'pr1' });
    expect(useAppStore.getState().projects).toEqual([]);
    expect(useAppStore.getState().hosts[0]?.sessions[0]?.projectRoot).toBeNull();
  });
});

