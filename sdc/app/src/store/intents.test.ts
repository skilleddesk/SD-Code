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

const { validName, renamePath, deletePath, createFolder, searchFolder, defaultReviewer, runVerify, autonomyFor, sendPrompt, interruptTurn, chooseModel, closeDiff, closeFolder, forkSession, loadCliRecipe, loadGitStatus, saveFile, emptySessionOn, newChatOnHost, openDiff, openFolderIn, loadDirectory, toggleDirectory, openFile, closeFile, addHost, hostKey, trustHost, listRemoteDirectory, runDoctor, runCommand, runInBackground, pollBackground, stopBackground, openTerminalForHost, installHostKey } = await import('./intents');
const { useTerminalStore } = await import('./terminal');
const { tabForSession } = await import('./rightPanel');
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
      detail: '',
      hostKey: '',
      address: '',
      pinned: '',
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
          detail: '',
          hostKey: '',
          address: '',
      pinned: '',
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
          detail: '',
          hostKey: '',
          address: '',
      pinned: '',
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
      detail: '',
      hostKey: '',
      address: '',
      pinned: '',
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
          thinkingMs: 0,
          thinkingSince: null,
          plan: [],
          startedAt: '2026-09-25T10:00:00Z',
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
 * `fs.write` (0.7.9) - the Save behind the file view's Edit button, and the P5 rule it brings with it.
 *
 * The checkpoint is **not** taken by this intent: `fs.write` takes it, in the daemon, where the file is
 * changed. That is the whole point - a rule enforced by the caller is a rule the next caller forgets. What
 * this asserts is that the window passes the chat's id (so the daemon can find the folder and hash the files)
 * and that the store's open file takes the daemon's new hash rather than keeping the old one.
 */
describe('saveFile', () => {
  beforeEach(() => {
    sdcpCall.mockReset();
    useFilesStore.getState().reset();
    usePrefsStore.setState({ activeTab: 's1', openTabs: ['s1'] });
  });

  it('writes through the daemon, with the chat id, and takes the new hash', async () => {
    useFilesStore.getState().setOpen({
      path: 'H:\\SDC\\README.md',
      name: 'README.md',
      text: '# old\n',
      sha256: 'a'.repeat(64),
      bytes: 6,
      truncated: false,
    });
    sdcpCall.mockResolvedValue({ path: 'H:\\SDC\\README.md', sha256: 'b'.repeat(64), bytes: 12 });

    await expect(saveFile('H:\\SDC\\README.md', '# new\n# more\n')).resolves.toBe(true);

    expect(sdcpCall).toHaveBeenCalledWith('fs.write', {
      path: 'H:\\SDC\\README.md',
      text: '# new\n# more\n',
      sessionId: 's1',
    });

    const opened = useFilesStore.getState().open;

    expect(opened?.text).toBe('# new\n# more\n');
    expect(opened?.sha256).toBe('b'.repeat(64));
    expect(opened?.bytes).toBe(12);
  });

  it('leaves the file as it was when the daemon refuses, and says so', async () => {
    useFilesStore.getState().setOpen({
      path: 'H:\\SDC\\.env',
      name: '.env',
      text: '',
      sha256: 'a'.repeat(64),
      bytes: 0,
      truncated: false,
    });
    sdcpCall.mockRejectedValueOnce(
      new SdcpCallError({ code: 'blocked_path', message: '`.env` holds secrets' }),
    );

    await expect(saveFile('H:\\SDC\\.env', 'SECRET=1')).resolves.toBe(false);
    expect(useFilesStore.getState().open?.text).toBe('');
  });
});

/**
 * `git.status` and `git.diff` (0.7.9) - the two methods that answer "what did the turn change?".
 *
 * Both took a `root` the app did not have; both now take the **session's** folder (0.7.6's contract), and the
 * window reads them from the Files header and the Preview's Diff button. A folder that is not a repository is
 * asserted too: `git.status` refusing is a normal answer, so the store clears the badge instead of toasting.
 */
describe('the folder’s git state', () => {
  beforeEach(() => {
    sdcpCall.mockReset();
    useFilesStore.getState().reset();
    usePrefsStore.setState({ activeTab: 's1', openTabs: ['s1'] });
  });

  it('reads the branch and the changed count for the chat’s folder', async () => {
    useFilesStore.getState().setRoot('H:\\SDC');
    sdcpCall.mockResolvedValue({ branch: 'main', dirty: 3 });

    await loadGitStatus();

    expect(sdcpCall).toHaveBeenCalledWith('git.status', { sessionId: 's1', root: 'H:\\SDC' });
    expect(useFilesStore.getState().git).toEqual({ branch: 'main', dirty: 3 });
  });

  it('re-reads it after a save, so the badge and the Diff button are never stale', async () => {
    useFilesStore.getState().setRoot('H:\\SDC');
    useFilesStore.getState().setOpen({
      path: 'H:\\SDC\\README.md',
      name: 'README.md',
      text: 'one\n',
      sha256: 'a'.repeat(64),
      bytes: 4,
      truncated: false,
    });

    /* The write answers first, then the status. A mock that returned the same object for both would hide the
       difference the probe found: the write succeeded and the badge still said `0 changed`. */
    sdcpCall.mockImplementation((method: string) =>
      method === 'fs.write'
        ? Promise.resolve({ path: 'H:\\SDC\\README.md', sha256: 'b'.repeat(64), bytes: 8 })
        : Promise.resolve({ branch: 'main', dirty: 1 }),
    );

    await expect(saveFile('H:\\SDC\\README.md', 'one\ntwo\n')).resolves.toBe(true);

    expect(sdcpCall).toHaveBeenCalledWith('git.status', { sessionId: 's1', root: 'H:\\SDC' });
    expect(useFilesStore.getState().git).toEqual({ branch: 'main', dirty: 1 });
  });

  it('shows no badge for a folder without git, rather than a failure', async () => {
    useFilesStore.getState().setRoot('H:\\notes');
    sdcpCall.mockRejectedValueOnce(
      new SdcpCallError({ code: 'internal', message: 'H:\\notes: not a git repository' }),
    );

    await loadGitStatus();

    expect(useFilesStore.getState().git).toBeNull();
  });

  it('opens the diff in the Preview, and closes it again', async () => {
    useFilesStore.getState().setRoot('H:\\SDC');
    sdcpCall.mockResolvedValue({ patch: 'diff --git a/README.md b/README.md\n' });

    await expect(openDiff()).resolves.toBe(true);
    expect(sdcpCall).toHaveBeenCalledWith('git.diff', { sessionId: 's1', root: 'H:\\SDC' });
    expect(useFilesStore.getState().diff).toContain('diff --git');

    closeDiff();
    expect(useFilesStore.getState().diff).toBeNull();
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


/**
 * The host key trust step (0.7.13) - the one question a remote connection asks a person.
 *
 * `host.add` scans the machine's key and answers with a host id; a machine SDC has never seen arrives
 * `untrusted` with its fingerprint in `HostStatus`, and the dialog hands that exact string to
 * `host.trust`. Two properties are asserted here, and both are the security design rather than
 * plumbing: the fingerprint travels as the value that was on screen, and the password is only sent when
 * there is one to send (the daemon spends it after the pin lands, never before).
 */
describe('addHost and trustHost', () => {
  beforeEach(() => {
    sdcpCall.mockReset();
    sdcpCall.mockResolvedValue({});
  });

  it('answers with the host id, because the trust step needs it', async () => {
    sdcpCall.mockResolvedValueOnce({ hostId: 'h7', reused: false });

    await expect(addHost({ type: 'ssh', target: 'root@vps.example -p 8443', label: '' })).resolves.toEqual({
      hostId: 'h7',
      reused: false,
    });

    /* The target travels exactly as typed: the daemon is what parses the port out of it (`0003-host-ssh`
       is the migration that stops it being dropped after the first probe). */
    expect(sdcpCall).toHaveBeenCalledWith('host.add', {
      type: 'ssh',
      target: 'root@vps.example -p 8443',
      label: '',
    });
  });

  it('pins the fingerprint it was given, and sends no password when there is none', async () => {
    await expect(trustHost('h7', 'SHA256:abc123')).resolves.toBe(true);

    expect(sdcpCall).toHaveBeenCalledWith('host.trust', { hostId: 'h7', fingerprint: 'SHA256:abc123' });
  });

  it('carries the password only after the decision, and only when the field has one', async () => {
    await trustHost('h7', 'SHA256:abc123', 'hunter2');

    expect(sdcpCall).toHaveBeenCalledWith('host.trust', {
      hostId: 'h7',
      fingerprint: 'SHA256:abc123',
      password: 'hunter2',
    });

    /* A refused pin is `false` and not a thrown promise: the dialog stays open, and the daemon's own
       sentence (a key that changed while the card was on screen) is the toast. */
    sdcpCall.mockRejectedValueOnce(new SdcpCallError({ code: 'bad_request', message: 'the key changed' }));

    await expect(trustHost('h7', 'SHA256:abc123')).resolves.toBe(false);
  });
});

/**
 * A chat whose folder is on a host (0.7.13): every file call names the machine.
 *
 * The daemon can resolve the host from a `sessionId`, but the tree asks for a *directory* (it knows the
 * path, not the chat), so the id has to travel with each request. Getting this wrong is not a crash: it
 * is a local `fs.list` of `/srv/app`, which answers "not a folder" on a machine that is fine.
 */
describe('a chat whose folder is on a host', () => {
  const remoteHost = {
    id: 'h7',
    name: 'prod-1',
    type: 'vps' as const,
    status: 'connected' as const,
    sdcd: '0.7.13',
    platform: '',
    detail: '',
    hostKey: '',
    address: 'root@vps.example:8443',
      pinned: '',
    sessions: [
      { id: 's7', title: 'Deploy', prompt: '', state: 'idle' as const, minutesAgo: 1, unread: 0, projectId: 'p1', projectRoot: '/srv/app' },
    ],
  };

  beforeEach(() => {
    sdcpCall.mockReset();
    sdcpCall.mockResolvedValue({});
    useFilesStore.getState().reset();
    usePrefsStore.setState({ activeTab: 's7', openTabs: ['s7'] });
    useAppStore.setState({ hosts: [remoteHost] });
  });

  it('tells the daemon which machine a directory is on', async () => {
    sdcpCall.mockResolvedValueOnce({ path: '/srv/app', entries: [], hidden: 0 });

    await loadDirectory('/srv/app/src');

    expect(sdcpCall).toHaveBeenCalledWith('fs.list', { path: '/srv/app/src', hostId: 'h7' });
  });

  it('reads the host\'s home when the browser asks for nothing in particular', async () => {
    sdcpCall.mockResolvedValueOnce({ path: '/home/me', entries: [{ name: 'app', path: '/home/me/app', dir: true, size: 0 }], hidden: 0 });

    const answer = await listRemoteDirectory('h7');

    expect(sdcpCall).toHaveBeenCalledWith('fs.list', { hostId: 'h7' });
    expect(answer?.path).toBe('/home/me');
  });

  it('takes a checkpoint on the host, and the toast says where', async () => {
    useFilesStore.getState().setOpen({
      path: '/srv/app/src/auth.ts',
      name: 'auth.ts',
      text: 'const a = 1;',
      sha256: 'a'.repeat(64),
      bytes: 12,
      truncated: false,
    });
    sdcpCall.mockResolvedValue({ path: '/srv/app/src/auth.ts', sha256: 'c'.repeat(64), bytes: 12 });

    await expect(saveFile('/srv/app/src/auth.ts', 'const a = 2;')).resolves.toBe(true);

    expect(sdcpCall).toHaveBeenCalledWith('fs.write', {
      path: '/srv/app/src/auth.ts',
      text: 'const a = 2;',
      sessionId: 's7',
      hostId: 'h7',
    });

    /* The checkpoint is the daemon's own work - it commits into a shadow repository **on that host**
       (`$HOME/.sdc/git/<hash>`, see `docs/REMOTE.md` §5) - and the toast names the host, because "where
       did that checkpoint go" is the question a person asks next. */
    const last = useAppStore.getState().toasts.at(-1);

    expect(last?.message).toContain('a checkpoint was taken on that host');
    expect(last?.message).toContain('prod-1');
  });
});



/**
 * `host.key` and a host's own doctor (0.7.13) - the two calls that make a host answerable *after* the
 * window that added it has closed.
 *
 * The case they exist for: a host added days ago, or in another window, sits at `needs your trust` with a
 * sentence and no fingerprint - and a button needs a value. `host.key` is the read; `host.doctor` with a
 * host id is the host's *own* environment rather than ten rows about this laptop, which is what it used
 * to answer.
 */
describe('asking a host about itself', () => {
  beforeEach(() => {
    sdcpCall.mockReset();
    sdcpCall.mockResolvedValue({});
  });

  it('reads the fingerprint a host presents now, and says whether it is the pinned one', async () => {
    sdcpCall.mockResolvedValueOnce({
      hostId: 'h7',
      hostKey: 'SHA256:now',
      keyType: 'ssh-ed25519',
      pinned: true,
      matches: false,
      pinnedKey: 'SHA256:before',
    });

    await expect(hostKey('h7')).resolves.toEqual({
      hostKey: 'SHA256:now',
      keyType: 'ssh-ed25519',
      pinned: true,
      matches: false,
      pinnedKey: 'SHA256:before',
    });
    expect(sdcpCall).toHaveBeenCalledWith('host.key', { hostId: 'h7' });

    /* A refusal (the host is gone, the scan failed) is `null` plus the daemon's sentence as a toast - the
       card keeps whatever it had rather than rendering half an answer. */
    sdcpCall.mockRejectedValueOnce(new SdcpCallError({ code: 'bad_request', message: 'no such host' }));

    await expect(hostKey('h7')).resolves.toBeNull();
  });

  it('runs the doctor about the host, not about this machine', async () => {
    sdcpCall.mockResolvedValueOnce({
      checks: [
        { id: 'ssh', label: 'SSH to root@vps.example:8443', state: 'ok', detail: 'root@vps.example:8443 is reachable' },
        { id: 'hostkey', label: 'Host key', state: 'fail', detail: 'SHA256:x · never trusted', fix: 'Trust' },
      ],
    });

    await runDoctor('h7');

    expect(sdcpCall).toHaveBeenCalledWith('host.doctor', { hostId: 'h7' });

    const stored = useAppStore.getState().doctor['h7'] ?? [];

    expect(stored).toHaveLength(2);
    expect(stored[1]?.fix).toBe('Trust');
    /* And nothing was written under `local`: a host's checks are the host's. */
    expect(useAppStore.getState().doctor['local']).toBeUndefined();
  });
});

/**
 * The Terminal tab's intents (0.7.13).
 *
 * Three things are worth a regression test, and each of them is a way a terminal lies:
 *
 *   1. **which machine** - the line has to run in the *chat's* folder on the *chat's* host, so `hostId`
 *      travels with it (and a local chat must not send one, or a local `git status` would run on a VPS);
 *   2. **what gets shown** - the answer's streams, exit code and duration land in the entry, and a
 *      refusal (the deny list) lands in the stderr slot rather than disappearing into a toast;
 *   3. **the background process** - `Run in background` opens a pty, `Stop` closes it, and the poll folds
 *      the output tail in.
 */
describe('the terminal', () => {
  const remoteHost = () => ({
    id: 'h7',
    name: 'prod-1',
    type: 'vps' as const,
    status: 'connected' as const,
    sdcd: '0.7.13',
    platform: '',
    detail: '',
    hostKey: '',
    address: 'root@vps.example',
    pinned: '',
    sessions: [
      {
        id: 's7',
        title: 'Deploy',
        prompt: '',
        state: 'idle' as const,
        minutesAgo: 1,
        unread: 0,
        projectId: 'p1',
        projectRoot: '/srv/app',
      },
    ],
  });

  const local = () => ({
    id: 'local',
    name: 'This machine',
    type: 'local' as const,
    status: 'connected' as const,
    sdcd: '0.7.13',
    platform: 'Windows 11 · x64',
    detail: '',
    hostKey: '',
    address: '',
    pinned: '',
    sessions: [
      {
        id: 's1',
        title: 'Landing',
        prompt: '',
        state: 'idle' as const,
        minutesAgo: 2,
        unread: 0,
        projectId: 'p1',
        projectRoot: 'H:\\SDC\\sdc',
      },
    ],
  });

  beforeEach(() => {
    sdcpCall.mockReset();
    sdcpCall.mockResolvedValue({});
    useTerminalStore.setState({ entries: [], history: [], background: null, busy: false, nextId: 1 });
    useRightPanelStore.setState({ activeTab: 'preview', tabBySession: {} });
  });

  it('runs the line in the chat folder on the chat host, and says so', async () => {
    usePrefsStore.setState({ activeTab: 's7', openTabs: ['s7'] });
    useAppStore.setState({ hosts: [remoteHost()] });
    sdcpCall.mockResolvedValueOnce({
      ok: true,
      exitCode: 0,
      stdout: 'deployed\n',
      stderr: '',
      durationMs: 42,
      timedOut: false,
    });

    await runCommand('deploy.sh --prod');

    expect(sdcpCall).toHaveBeenCalledWith('shell.run', {
      line: 'deploy.sh --prod',
      root: '/srv/app',
      sessionId: 's7',
      hostId: 'h7',
    });

    const entry = useTerminalStore.getState().entries[0];

    expect(entry?.where).toBe('/srv/app on prod-1');
    expect(entry?.state).toBe('done');
    expect(entry?.stdout).toBe('deployed\n');
    expect(entry?.code).toBe(0);
    expect(entry?.ms).toBe(42);
    /* The line is remembered for ↑, and the input is free again. */
    expect(useTerminalStore.getState().history).toEqual(['deploy.sh --prod']);
    expect(useTerminalStore.getState().busy).toBe(false);
  });

  it('sends no host for a local chat - a local command must not run on a VPS', async () => {
    usePrefsStore.setState({ activeTab: 's1', openTabs: ['s1'] });
    useAppStore.setState({ hosts: [local()] });

    await runCommand('pnpm test');

    expect(sdcpCall).toHaveBeenCalledWith('shell.run', { line: 'pnpm test', root: 'H:\\SDC\\sdc', sessionId: 's1' });
    expect(useTerminalStore.getState().entries[0]?.where).toBe('H:\\SDC\\sdc');
  });

  it('shows a refusal where output goes, and frees the input', async () => {
    usePrefsStore.setState({ activeTab: 's7', openTabs: ['s7'] });
    useAppStore.setState({ hosts: [remoteHost()] });
    sdcpCall.mockRejectedValueOnce(
      new SdcpCallError({
        code: 'permission_denied',
        message: '`shutdown` was refused because it powers the machine off under the user',
      }),
    );

    await runCommand('shutdown /s');

    const entry = useTerminalStore.getState().entries[0];

    expect(entry?.state).toBe('failed');
    expect(entry?.stderr).toContain('was refused');
    expect(entry?.code).toBeNull();
    expect(useTerminalStore.getState().busy).toBe(false);
  });

  it('runs a long line in the background, and Stop closes it', async () => {
    usePrefsStore.setState({ activeTab: 's7', openTabs: ['s7'] });
    useAppStore.setState({ hosts: [remoteHost()] });
    sdcpCall.mockResolvedValueOnce({ ptyId: 'pty-4', command: 'pnpm dev', tty: false, hostId: 'h7' });

    await runInBackground('pnpm dev');

    /* The daemon owns the shell: the window sends the line, not a program guessed for its platform - and
       the folder under `cwd`, which is what `pty.open` has called it since 0.7.0. */
    expect(sdcpCall).toHaveBeenCalledWith('pty.open', {
      line: 'pnpm dev',
      cwd: '/srv/app',
      sessionId: 's7',
      hostId: 'h7',
    });
    expect(useTerminalStore.getState().background).toMatchObject({ ptyId: 'pty-4', command: 'pnpm dev' });

    /* A second one is refused with a sentence rather than a second process. */
    await runInBackground('tail -f log');

    expect(sdcpCall).toHaveBeenCalledTimes(1);

    sdcpCall.mockResolvedValueOnce({ lines: ['ready in 300 ms'], state: 'running', ms: 1200 });
    await pollBackground();

    expect(useTerminalStore.getState().entries[0]?.stdout).toBe('ready in 300 ms');
    expect(useTerminalStore.getState().entries[0]?.state).toBe('running');

    sdcpCall.mockResolvedValueOnce({ closed: true });
    await stopBackground();

    expect(sdcpCall).toHaveBeenLastCalledWith('pty.close', { ptyId: 'pty-4' });
    expect(useTerminalStore.getState().background).toBeNull();
    expect(useTerminalStore.getState().entries[0]?.stderr).toBe('stopped');
  });

  it('frees the input when a background process ends on its own', async () => {
    usePrefsStore.setState({ activeTab: 's7', openTabs: ['s7'] });
    useAppStore.setState({ hosts: [remoteHost()] });
    sdcpCall.mockResolvedValueOnce({ ptyId: 'pty-9', command: 'build', tty: false });

    await runInBackground('pnpm build');

    sdcpCall.mockResolvedValueOnce({ lines: ['done'], state: 'exited', ms: 8000 });
    await pollBackground();

    expect(useTerminalStore.getState().entries[0]?.state).toBe('done');
    expect(useTerminalStore.getState().background).toBeNull();
  });

  it('says the process is gone when a restarted daemon has never heard of it', async () => {
    usePrefsStore.setState({ activeTab: 's7', openTabs: ['s7'] });
    useAppStore.setState({ hosts: [remoteHost()] });
    sdcpCall.mockResolvedValueOnce({ ptyId: 'pty-2', command: 'serve', tty: false });

    await runInBackground('serve');

    sdcpCall.mockRejectedValueOnce(
      new SdcpCallError({ code: 'not_found', message: 'pty-2 is not a process this daemon started' }),
    );
    await pollBackground();

    expect(useTerminalStore.getState().background).toBeNull();
    expect(useTerminalStore.getState().entries[0]?.stderr).toBe('ended');
  });

  /**
   * `Install` on a doctor row leads here: the terminal opens **about that host** or not at all.
   *
   * This is the assertion that keeps the surface from lying: the tab runs in the active chat's folder on
   * the active chat's host, so opening it while a *different* chat is focused would show a terminal about
   * the wrong computer - the exact mistake a remote-capable terminal must not make.
   */
  it('opens the terminal about the host by focusing its chat first', () => {
    const shown = vi.fn();

    usePrefsStore.setState({ activeTab: 's1', openTabs: ['s1'] });
    useAppStore.setState({ hosts: [local(), remoteHost()] });
    useLayoutStore.setState({ showRight: shown });

    openTerminalForHost('h7');

    expect(usePrefsStore.getState().activeTab).toBe('s7');
    expect(useRightPanelStore.getState().activeTab).toBe('terminal');
    expect(tabForSession(useRightPanelStore.getState(), 's7')).toBe('terminal');
    expect(shown).toHaveBeenCalled();
  });

  it('does not open a terminal pointed at another machine when the host has no chat', () => {
    const quiet = { ...remoteHost(), sessions: [] };

    usePrefsStore.setState({ activeTab: 's1', openTabs: ['s1'] });
    useAppStore.setState({ hosts: [local(), quiet] });
    useRightPanelStore.setState({ activeTab: 'preview', tabBySession: {} });

    openTerminalForHost('h7');

    expect(usePrefsStore.getState().activeTab).toBe('s1');
    expect(useRightPanelStore.getState().activeTab).toBe('preview');
  });

  /**
   * The step that finishes a VPS: SDC's key is copied over with the password, once.
   *
   * This is the half that was missing after the pin - a host could be trusted and still never connect,
   * with nowhere in the window to type the password. The proven path is `host.add` again (the daemon
   * reuses the row and, because the key is pinned, goes straight to the install), so the assertion is
   * that the window sends the address it *shows* - `user@host:8443`, port and all - because a target
   * without the port is a target that dials 22 and fails on a host whose sshd is elsewhere.
   */
  it('installs the key against the address the row shows, with the port in it', async () => {
    const host = { ...remoteHost(), address: 'root@vps.example:8443', pinned: 'SHA256:abc123', status: 'offline' as const };

    useAppStore.setState({ hosts: [{ ...host, sessions: [] }] });
    sdcpCall.mockResolvedValueOnce({ hostId: 'h7', reused: true });

    await expect(installHostKey('h7', 'hunter2')).resolves.toBe(true);

    expect(sdcpCall).toHaveBeenCalledWith('host.add', {
      type: 'ssh',
      target: 'root@vps.example:8443',
      label: 'prod-1',
      password: 'hunter2',
    });
  });

  it('says nothing to the daemon when the host is not in the list', async () => {
    useAppStore.setState({ hosts: [] });

    await expect(installHostKey('h9', 'hunter2')).resolves.toBe(false);

    expect(sdcpCall).not.toHaveBeenCalled();
  });
});


/*
 * v4: Agent mode travels with the turn, and the app's mode decides how much the agent may do alone.
 * Stop is a real `engine.cancel`.
 */
describe('sendPrompt in agent mode', () => {
  beforeEach(() => {
    sdcpCall.mockReset();
    sdcpCall.mockResolvedValue({ turnId: 'turn-9' });
    useModelStore.setState({ tier: 'deep', engine: 'native_api', model: 'claude-opus-5-5', providerId: 'anthropic-api', compose: 'agent' });
  });

  it('asks for the agent loop, with the autonomy of the current mode, for the pane that sent it', async () => {
    useLayoutStore.setState({ mode: 'simple' });

    await expect(sendPrompt('build the login page', 's7')).resolves.toBe('turn-9');

    expect(sdcpCall).toHaveBeenCalledWith('engine.start', {
      sessionId: 's7',
      prompt: 'build the login page',
      engine: 'native_api',
      model: 'claude-opus-5-5',
      tier: 'Deep',
      provider: 'anthropic-api',
      agent: true,
      autonomy: 'ask',
    });
  });

  it('sends a plain chat turn when Chat is chosen', async () => {
    useModelStore.setState({ compose: 'chat' });
    useLayoutStore.setState({ mode: 'auto' });

    await sendPrompt('what does this do?', 's7');

    expect(sdcpCall.mock.calls[0]?.[1]).toMatchObject({ agent: false, autonomy: 'auto' });
  });

  it('maps the three modes to the three autonomy levels', () => {
    expect(autonomyFor('simple')).toBe('ask');
    expect(autonomyFor('pro')).toBe('pro');
    expect(autonomyFor('auto')).toBe('auto');
  });

  it('stops a turn through the daemon', async () => {
    sdcpCall.mockResolvedValue({ state: 'killed', engine: 'cancel', stopped: true });

    await interruptTurn('turn-9');

    expect(sdcpCall).toHaveBeenCalledWith('engine.cancel', { turnId: 'turn-9' });
  });
});

describe('verify', () => {
  const connected = (id: string, name: string) => ({ id, name, kind: 'api-key' as const, status: 'connected' as const, detail: '', account: null, logo: id, initial: 'X' });

  beforeEach(() => {
    sdcpCall.mockReset();
    sdcpCall.mockResolvedValue({ verifyId: 'verify-3' });
    useModelStore.setState({
      catalog: [
        { id: 'sonnet', providerId: 'claude', providerLabel: 'Claude', tier: 'balanced', ctx: 0, cost: '', name: 'Claude Sonnet', source: 'bundled' },
        { id: 'gemini-3-pro', providerId: 'gemini', providerLabel: 'Gemini', tier: 'deep', ctx: 0, cost: '', name: 'Gemini 3 Pro', source: 'live' },
      ],
    });
    useAppStore.setState({
      providers: [connected('claude', 'Claude'), connected('gemini', 'Gemini')] as never,
      turns: [
        { id: 't1', sessionId: 's1', turnNumber: 1, engine: 'claude_code', model: 'sonnet', tier: 'Balanced', prompt: 'fix the 500', text: 'done', thinking: '', thinkingMs: 0, thinkingSince: null, plan: [], startedAt: '2026-09-25T10:00:00Z', status: 'done', stuckForMs: 0, tools: [], summary: '', meta: '', pass: null },
      ],
      checkpoints: [
        { id: 'cp-late', sessionId: 's1', turnId: 't1', turn: 9, when: 'now', title: 'Before Run', thumbnail: null, filesHash: 'b'.repeat(40) },
        { id: 'cp-first', sessionId: 's1', turnId: 't1', turn: 4, when: 'now', title: 'Before Edit', thumbnail: null, filesHash: 'a'.repeat(40) },
      ],
    });
  });

  it('picks a reviewer that is not the engine that wrote the change', () => {
    expect(defaultReviewer({ engine: 'claude_code', model: 'sonnet' })?.engine).toBe('gemini');
    expect(defaultReviewer({ engine: 'gemini', model: 'gemini-3-pro' })?.engine).toBe('claude_code');
  });

  it("sends the turn's first checkpoint and its own prompt, and opens the Verify tab", async () => {
    await expect(
      runVerify({ sessionId: 's1', turnId: 't1', reviewer: defaultReviewer({ engine: 'claude_code', model: 'sonnet' }) }),
    ).resolves.toBe('verify-3');

    expect(sdcpCall).toHaveBeenCalledWith('verify.run', expect.objectContaining({
      sessionId: 's1',
      turnId: 't1',
      task: 'fix the 500',
      since: 'a'.repeat(40),
      reviewer: { engine: 'gemini', model: 'gemini-3-pro', provider: 'gemini' },
    }));
    expect(tabForSession(useRightPanelStore.getState(), 's1')).toBe('verify');
  });
});

describe('the tree changes things through the daemon, inside the chat', () => {
  beforeEach(() => {
    sdcpCall.mockReset();
    sdcpCall.mockResolvedValue({ entries: [], hidden: 0, path: '/srv/app', branch: '', dirty: 0, hits: [] });
    usePrefsStore.setState({ activeTab: 's1' });
    useFilesStore.getState().reset();
    useFilesStore.getState().setRoot('/srv/app');
  });

  it('accepts a plain name and refuses one that climbs or carries a separator', () => {
    expect(validName('pay.test.ts')).toBe(true);
    expect(validName('../x')).toBe(false);
    expect(validName('a/b')).toBe(false);
    expect(validName('..')).toBe(false);
    expect(validName('  ')).toBe(false);
  });

  it('renames in the same folder and closes the tab of the old path', async () => {
    useFilesStore.getState().setOpen({ path: '/srv/app/src/a.ts', name: 'a.ts', text: '', sha256: '', bytes: 0, truncated: false });

    await expect(renamePath('/srv/app/src/a.ts', 'b.ts')).resolves.toBe(true);

    expect(sdcpCall).toHaveBeenCalledWith('fs.rename', expect.objectContaining({ path: '/srv/app/src/a.ts', to: '/srv/app/src/b.ts', sessionId: 's1' }));
    expect(useFilesStore.getState().tabs).toEqual([]);
  });

  it('deletes through fs.delete with the chat, so the daemon checkpoints first', async () => {
    await expect(deletePath('/srv/app/old')).resolves.toBe(true);

    expect(sdcpCall).toHaveBeenCalledWith('fs.delete', expect.objectContaining({ path: '/srv/app/old', sessionId: 's1' }));
  });

  it('refuses a bad folder name before it reaches the daemon', async () => {
    await expect(createFolder('/srv/app', 'a/b')).resolves.toBe(false);

    expect(sdcpCall).not.toHaveBeenCalledWith('fs.mkdir', expect.anything());
  });

  it('searches the chat folder', async () => {
    sdcpCall.mockResolvedValue({ hits: [{ path: '/srv/app/x.ts', line: 3, text: 'needle' }] });

    await expect(searchFolder('needle')).resolves.toEqual([{ path: '/srv/app/x.ts', line: 3, text: 'needle' }]);
    expect(sdcpCall).toHaveBeenCalledWith('fs.search', expect.objectContaining({ query: 'needle', sessionId: 's1' }));
  });
});
