import { describe, expect, it } from 'vitest';

import type { SdcpEvent } from '../../../protocol/types';
import { applyEvent, applyEvents, createInitialState, EMPTY_STATE, withProviders, withWorkspace } from './reducer';
import type { AppEvent, AppState } from './types';

/**
 * The reducer's acceptance test (master spec section 3.3).
 *
 * Four properties, in the order the spec asks for them:
 *
 *   1. EMPTY BOOT      a fresh window folds an *empty* log: no hosts, no chats, no providers, no
 *                      turns. It used to fold a demo - three hosts, six chats, nine providers, twelve
 *                      models - which made every screenshot a screenshot of fiction (0.5.0).
 *   2. PURITY          folding the same log twice gives a deep-equal state, and nothing in the fold
 *                      reads a wall clock, a random source or the network. The clock arrives inside
 *                      the event, which is what makes this assertion meaningful rather than lucky.
 *   3. TIME TRAVEL     replaying a prefix of the log gives the state the app was in at that point -
 *                      the "dispatch old events, view state matches expected" check.
 *   4. LIVE TURNS      the turn projections the stream draws come from `TurnStarted`/`TurnDelta`/
 *                      `ToolCall*`, and `TurnStarted` carries the user's own prompt back.
 */

/** Folds one event onto a state, with the envelope fields a daemon would supply. */
function fold(state: AppState, event: SdcpEvent, seq = state.seq + 1): AppState {
  const entry: AppEvent = {
    seq,
    ts: `2026-09-20T14:02:${String(seq % 60).padStart(2, '0')}.000Z`,
    event,
  };

  return applyEvent(state, entry);
}

/**
 * A log to fold, built here rather than shipped.
 *
 * The app used to export `seedEvents()` for exactly this: the demo lived in `src/`, so a test could
 * fold it. That made the demo part of the product. This fixture is a *test's* log - two hosts, two
 * chats, three turns - and nothing outside this file can reach it.
 */
function fixtureLog(): AppEvent[] {
  const events: SdcpEvent[] = [
    { type: 'HostStatus', hostId: 'local', name: 'Local', hostType: 'local', status: 'connected', sdcd: '0.5.0' },
    { type: 'HostStatus', hostId: 'vps1', name: 'prod-1', hostType: 'vps', status: 'connecting' },
    { type: 'SessionOpened', sessionId: 's1', hostId: 'local', title: 'Rate limiting', prompt: 'Add rate limiting' },
    { type: 'SessionOpened', sessionId: 's2', hostId: 'vps1', title: 'Logs', prompt: 'Find the 500s' },
    {
      type: 'TurnStarted',
      turnId: 't1',
      sessionId: 's1',
      engine: 'claude_code',
      model: 'sonnet',
      tier: 'Balanced',
      prompt: 'Add rate limiting',
    },
    { type: 'TurnDelta', turnId: 't1', delta: 'Added the limiter.' },
    { type: 'TurnCompleted', turnId: 't1', summary: 'Done', meta: '2s', pass: true },
    { type: 'TurnStarted', turnId: 't2', sessionId: 's2', engine: 'codex', model: 'default', tier: 'Fast', prompt: 'Find the 500s' },
  ];

  return events.map((event, index) => ({
    seq: index + 1,
    ts: `2026-09-20T14:0${index}:00.000Z`,
    event,
  }));
}

describe('reducer', () => {
  it('boots empty: a fresh window claims nothing it has not been told', () => {
    const state = createInitialState();

    expect(state).toBe(EMPTY_STATE);
    expect(state.hosts).toEqual([]);
    expect(state.providers).toEqual([]);
    expect(state.registry).toEqual([]);
    expect(state.turns).toEqual([]);
    expect(state.checkpoints).toEqual([]);
    expect(state.seq).toBe(0);
  });

  it('is pure: the same log folds to a deep-equal state', () => {
    const first = applyEvents(EMPTY_STATE, fixtureLog());
    const second = applyEvents(EMPTY_STATE, fixtureLog());

    expect(second).toEqual(first);
    expect(second).not.toBe(first);
  });

  it('travels: a prefix of the log is the state at that moment', () => {
    const log = fixtureLog();
    const partial = applyEvents(EMPTY_STATE, log.slice(0, 4));

    /* Four events in: the two hosts and the two chats are there; nothing has run yet. */
    expect(partial.hosts).toHaveLength(2);
    expect(partial.hosts[0]?.sessions).toHaveLength(1);
    expect(partial.turns).toHaveLength(0);
    expect(partial.seq).toBe(4);
  });

  it('draws the stream from the log, prompt included', () => {
    const state = applyEvents(EMPTY_STATE, fixtureLog());

    expect(state.turns.map((turn) => turn.prompt)).toEqual(['Add rate limiting', 'Find the 500s']);
    expect(state.turns[0]?.text).toBe('Added the limiter.');
    expect(state.turns[0]?.status).toBe('done');
    expect(state.activeTurnId).toBe('t2');
  });

  it('never mutates the state it is handed', () => {
    const before = createInitialState();
    const snapshot = structuredClone(before);

    fold(before, { type: 'Toast', message: 'hello' });

    expect(before).toEqual(snapshot);
  });

  it('appends a toast and removes it again on ToastDismissed', () => {
    const withToast = fold(EMPTY_STATE, { type: 'Toast', message: 'one' });

    expect(withToast.toasts).toHaveLength(1);

    const gone = fold(withToast, { type: 'ToastDismissed', id: withToast.toasts[0]!.id });

    expect(gone.toasts).toHaveLength(0);
  });

  it('keeps a session inside its host and ignores a session with no host', () => {
    const orphan = fold(EMPTY_STATE, {
      type: 'SessionOpened',
      sessionId: 'x1',
      hostId: 'nowhere',
      title: 'orphan',
      prompt: '',
    });

    expect(orphan.hosts).toHaveLength(0);

    const hosted = fold(
      fold(EMPTY_STATE, {
        type: 'HostStatus',
        hostId: 'h1',
        name: 'h1',
        hostType: 'vps',
        status: 'connecting',
      }),
      { type: 'SessionOpened', sessionId: 's1', hostId: 'h1', title: 'first', prompt: 'p' },
    );

    expect(hosted.hosts[0]?.sessions).toHaveLength(1);
  });

  it('streams a turn: started, deltas, a tool call and a checkpoint', () => {
    let state = fold(EMPTY_STATE, {
      type: 'TurnStarted',
      turnId: 't1',
      sessionId: 's1',
      engine: 'claude_code',
      model: 'sonnet',
      tier: 'Balanced',
      prompt: 'Add rate limiting',
    });

    state = fold(state, { type: 'TurnDelta', turnId: 't1', delta: 'hel' });
    state = fold(state, { type: 'TurnDelta', turnId: 't1', delta: 'lo' });
    state = fold(state, {
      type: 'ToolCallStarted',
      turnId: 't1',
      callId: 'c1',
      tool: 'edit',
      name: 'Edit',
      target: 'src/auth.ts',
    });
    state = fold(state, {
      type: 'CheckpointSaved',
      sessionId: 's1',
      checkpoint: {
        id: 'cp-1',
        turn: 1,
        ts: 'now',
        title: 'Added validation',
        thumbnail: null,
        filesHash: 'abc',
        rewindRef: null,
      },
    });

    expect(state.turns[0]?.text).toBe('hello');
    expect(state.turns[0]?.tools[0]?.status).toBe('running');
    expect(state.checkpoints[0]?.filesHash).toBe('abc');
  });

  it('marks a silent turn stuck and clears the mark on the next delta', () => {
    let state = fold(EMPTY_STATE, {
      type: 'TurnStarted',
      turnId: 't1',
      sessionId: 's1',
      engine: 'native_api',
      model: 'gpt-5',
      tier: 'Deep',
      prompt: 'why did it stop',
    });

    state = fold(state, { type: 'StuckDetected', turnId: 't1', sessionId: 's1', sinceMs: 20000 });
    expect(state.turns[0]?.status).toBe('stuck');

    state = fold(state, { type: 'TurnDelta', turnId: 't1', delta: 'back' });
    expect(state.turns[0]?.status).toBe('running');
  });

  it('rewinds and redoes a turn', () => {
    /*
     * A rewind drops the checkpoints *newer* than the turn it goes back to, so this fixture has to
     * have one of those: a checkpoint at turn 2, then a rewind to turn 1. (The old version of this
     * test rewound into the seeded three, which is why it passed while proving nothing about an empty
     * window.)
     */
    let state = fold(EMPTY_STATE, {
      type: 'CheckpointSaved',
      sessionId: 's1',
      checkpoint: {
        id: 'cp-1',
        turn: 2,
        ts: 'now',
        title: 'Added validation',
        thumbnail: null,
        filesHash: 'abc',
        rewindRef: null,
      },
    });

    expect(state.checkpoints).toHaveLength(1);

    state = fold(state, {
      type: 'RewindApplied',
      sessionId: 's1',
      direction: 'back',
      turn: 1,
      turns: 1,
      files: 1,
    });

    expect(state.checkpoints).toHaveLength(0);
    expect(state.rewindStack).toHaveLength(1);

    state = fold(state, {
      type: 'RewindApplied',
      sessionId: 's1',
      direction: 'forward',
      turn: 2,
      turns: 1,
      files: 1,
    });

    expect(state.checkpoints).toHaveLength(1);
    expect(state.rewindStack).toHaveLength(0);
  });

  it('records an approval by risk and remembers an `always allow`', () => {
    const requested = fold(EMPTY_STATE, {
      type: 'PermissionRequested',
      permissionId: 'p1',
      sessionId: 's1',
      title: 'Delete a file',
      sub: 'mutating',
      action: 'delete',
      target: 'src/database.js',
      risk: 'MUTATING',
    });

    expect(requested.permission?.risk).toBe('MUTATING');

    const resolved = fold(requested, {
      type: 'PermissionResolved',
      permissionId: 'p1',
      decision: 'always_allow',
    });

    expect(resolved.permission).toBeNull();
    expect(resolved.resolvedPermissions['p1']).toBe('always_allow');

    /* A second request for a remembered action must not re-open the dialog. */
    const again = fold(resolved, {
      type: 'PermissionRequested',
      permissionId: 'p1',
      sessionId: 's1',
      title: 'Delete a file',
      sub: 'mutating',
      action: 'delete',
      target: 'src/database.js',
      risk: 'MUTATING',
    });

    expect(again.permission).toBeNull();
  });

  it('deduplicates console lines by file and line, and counts the repeats', () => {
    const line = {
      type: 'ConsoleError' as const,
      sessionId: 's1',
      level: 'error' as const,
      message: 'boom',
      source: 'at a.ts:1:1',
      file: 'a.ts',
      line: 1,
    };

    const twice = fold(fold(EMPTY_STATE, line), line);

    expect(twice.console).toHaveLength(1);
    expect(twice.console[0]?.count).toBe(2);
  });

  it('bridges a session and resolves a duel', () => {
    let state = fold(EMPTY_STATE, {
      type: 'SessionBridged',
      sessionId: 's1',
      turnId: 't1',
      from: 'claude_code',
      to: 'codex',
      model: 'default',
    });

    expect(state.bridges[0]?.to).toBe('codex');

    state = fold(state, {
      type: 'DuelStarted',
      duelId: 'd1',
      sessionId: 's1',
      prompt: 'p',
      engines: ['claude_code', 'codex'],
      panes: [
        { engine: 'claude_code', model: 'sonnet', time: '1s', cost: '$0.1', pass: true, headline: 'x', files: [] },
        { engine: 'codex', model: 'default', time: '2s', cost: '$0.2', pass: false, headline: 'y', files: [] },
      ],
    });

    state = fold(state, { type: 'DuelResolved', duelId: 'd1', kept: 'codex' });

    expect(state.duels[0]).toMatchObject({ kept: 'codex', resolved: true });
  });

describe('the two host facts that were missing', () => {
  const listed = [
    { type: 'HostStatus', hostId: 'local', name: 'Local', hostType: 'local', status: 'connected' },
    { type: 'HostStatus', hostId: 'h1', name: 'Website', hostType: 'vps', status: 'connecting' },
    { type: 'SessionOpened', sessionId: 's1', hostId: 'h1', title: 'Fix it', prompt: 'Fix it' },
  ] as const;

  it('removes a host, and the chats under it, when the daemon says it is gone', () => {
    let state = applyEvents(
      EMPTY_STATE,
      listed.map((event, index) => ({ seq: index + 1, ts: `2026-09-20T14:0${index}:00.000Z`, event })),
    );

    expect(state.hosts.map((host) => host.id)).toEqual(['local', 'h1']);

    state = fold(state, { type: 'HostRemoved', hostId: 'h1', name: 'Website', sessions: 1 });

    /* Nothing is left behind: a host that is no longer in the list cannot be switched to, and its
       sessions went with it in the daemon too (`host.remove` deletes the rows). */
    expect(state.hosts.map((host) => host.id)).toEqual(['local']);
    expect(state.hosts.every((host) => host.sessions.every((session) => session.id !== 's1'))).toBe(true);
  });

  it('folds the list a restart would otherwise lose', () => {
    /*
     * What `session.list` answers on the next launch. `host.add` wrote the row and pushed one event;
     * without this patch the window showed only `local` and every added host looked as though adding
     * it had failed - the row was in the daemon's database the whole time.
     */
    const state = withWorkspace(EMPTY_STATE, [
      {
        hostId: 'local',
        name: 'Local',
        hostType: 'local',
        status: 'connected',
        platform: 'windows · x86_64',
        target: null,
        sessions: [],
      },
      {
        hostId: 'h1',
        name: 'Website',
        hostType: 'vps',
        status: 'offline',
        platform: null,
        target: 'root@vps.example',
        sessions: [
          {
            sessionId: 'n7',
            hostId: 'h1',
            title: 'Fix it',
            prompt: 'Fix it',
            state: 'error',
            unread: 2,
            minutesAgo: 14,
          },
        ],
      },
    ]);

    expect(state.hosts.map((host) => host.id)).toEqual(['local', 'h1']);
    expect(state.hosts[1]?.sessions[0]).toMatchObject({ id: 'n7', title: 'Fix it', minutesAgo: 14, unread: 2 });
  });

  it('does not open the same chat twice when the daemon replays it', () => {
    /*
     * The duplicate-row report: one click on `New chat` drew two rows, and the count pill, the tab
     * strip and the sidebar all disagreed about how many chats existed.
     *
     * Two ways the same `SessionOpened` reaches the fold, both legitimate: the bridge replays the log
     * on every connect (`event.list since=0`), and `session.list` may already have listed the session
     * in between - the replay and that read are two sockets, so the order is not guaranteed. The
     * reducer has to make the second copy a no-op rather than a second row.
     */
    let state = applyEvents(
      EMPTY_STATE,
      listed.map((event, index) => ({ seq: index + 1, ts: `2026-09-20T14:0${index}:00.000Z`, event })),
    );

    const opened = { seq: 4, ts: '2026-09-20T14:04:00.000Z', event: listed[2] };

    state = fold(state, opened.event);
    state = fold(state, opened.event);

    expect(state.hosts[1]?.sessions.map((session) => session.id)).toEqual(['s1']);

    /* And the boot race, in the other order: the list arrives first, then the replay of the open. */
    const listedAgain = withWorkspace(state, [
      {
        hostId: 'h1',
        name: 'Website',
        hostType: 'vps',
        status: 'connecting',
        platform: null,
        target: null,
        sessions: [
          {
            sessionId: 's1',
            hostId: 'h1',
            title: 'Fix it',
            prompt: 'Fix it',
            state: 'idle',
            unread: 0,
            minutesAgo: 3,
          },
        ],
      },
    ]);

    const afterReplay = fold(listedAgain, listed[2]);

    expect(afterReplay.hosts[0]?.sessions.map((session) => session.id)).toEqual(['s1']);
  });

  it('folds the provider list a read used to be thrown away', () => {
    /*
     * `provider.list` answers with the cards and appends nothing, so the Hub - which reads the event
     * log - drew `0 connected · None yet` on every launch while the daemon knew about eleven
     * providers. The answer has to be folded, and this is the fold.
     */
    const state = withProviders(EMPTY_STATE, [
      {
        id: 'claude',
        name: 'Claude',
        kind: 'subscription',
        status: 'needs-auth',
        detail: '`claude` is installed · Connect starts its own sign-in',
        logo: 'claude',
        initial: 'C',
      },
      { id: 'groq', name: 'Groq', kind: 'api-key', status: 'available' },
    ]);

    expect(state.providers).toHaveLength(2);
    expect(state.providers[0]).toMatchObject({
      id: 'claude',
      kind: 'subscription',
      status: 'needs-auth',
      logo: 'claude',
      initial: 'C',
      account: null,
    });

    /* A card the daemon sends without a logo or an initial still has both: the Hub draws them. */
    expect(state.providers[1]).toMatchObject({ logo: 'custom', initial: 'G', detail: '' });
  });
});

});
