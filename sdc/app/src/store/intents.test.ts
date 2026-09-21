import { beforeEach, describe, expect, it, vi } from 'vitest';

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

const { chooseModel, closeFolder, emptySessionOn, newChatOnHost, openFolderIn } = await import('./intents');
const { engineForProvider, useModelStore } = await import('./model');
const { usePrefsStore } = await import('./prefs');
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

