import { describe, expect, it } from 'vitest';

import type { SdcpEvent } from '../../../protocol/types';
import { applyEvent, applyEvents, createInitialState, EMPTY_STATE, seedEvents } from './reducer';
import type { AppEvent, AppState } from './types';

/**
 * The reducer's acceptance test (master spec section 3.3).
 *
 * Three properties, in the order the spec asks for them:
 *
 *   1. PURITY        folding the same log twice gives a deep-equal state, and nothing in the fold
 *                    reads a wall clock, a random source or the network. The clock arrives inside
 *                    the event, which is what makes this assertion meaningful rather than lucky.
 *   2. TIME TRAVEL   replaying a prefix of the log gives the state the app was in at that point -
 *                    the "dispatch old events, view state matches expected" check.
 *   3. SEED          the demo the prototype ships is the fold of a log, so what a reader sees on
 *                    first run and what the last event says are the same thing.
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

describe('reducer', () => {
  it('is pure: the same log folds to a deep-equal state', () => {
    const first = applyEvents(EMPTY_STATE, seedEvents());
    const second = applyEvents(EMPTY_STATE, seedEvents());

    expect(second).toEqual(first);
    expect(second).not.toBe(first);
  });

  it('seeds the prototype demo: three hosts, six chats, nine providers, twelve models', () => {
    const state = createInitialState();

    expect(state.hosts.map((host) => host.name)).toEqual(['Local', 'prod-1', 'staging-2']);
    expect(state.hosts.map((host) => host.sessions.length)).toEqual([3, 2, 1]);
    expect(state.providers).toHaveLength(9);
    expect(state.registry).toHaveLength(12);
    expect(state.checkpoints.map((checkpoint) => checkpoint.turn)).toEqual([14, 13, 12]);
    /* Newest first, so the warn line the daemon pushed last is `console[0]`. */
    expect(state.console.find((line) => line.level === 'error')?.count).toBe(3);
    expect(state.duels[0]?.panes).toHaveLength(2);
  });

  it('travels: a prefix of the log is the state at that moment', () => {
    const log = seedEvents();
    const partial = applyEvents(EMPTY_STATE, log.slice(0, 14));

    /* Fourteen events in: the hosts and the six sessions are there; nothing else is. */
    expect(partial.hosts).toHaveLength(3);
    expect(partial.providers).toHaveLength(0);
    expect(partial.checkpoints).toHaveLength(0);
    expect(partial.seq).toBe(14);
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
    });

    state = fold(state, { type: 'StuckDetected', turnId: 't1', sessionId: 's1', sinceMs: 20000 });
    expect(state.turns[0]?.status).toBe('stuck');

    state = fold(state, { type: 'TurnDelta', turnId: 't1', delta: 'back' });
    expect(state.turns[0]?.status).toBe('running');
  });

  it('rewinds and redoes a turn', () => {
    let state = createInitialState();
    const before = state.checkpoints.length;

    state = fold(state, {
      type: 'RewindApplied',
      sessionId: 's1',
      direction: 'back',
      turn: 13,
      turns: 1,
      files: 1,
    });

    expect(state.checkpoints).toHaveLength(before - 1);
    expect(state.rewindStack).toHaveLength(1);

    state = fold(state, {
      type: 'RewindApplied',
      sessionId: 's1',
      direction: 'forward',
      turn: 14,
      turns: 1,
      files: 1,
    });

    expect(state.checkpoints).toHaveLength(before);
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
});
