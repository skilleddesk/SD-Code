import { strings } from '../strings';
import type { RegistryModel, SdcpEvent } from '../../../protocol/types';
import { stepClock } from './events';
import type {
  AppEvent,
  AppState,
  CheckpointView,
  ConsoleLine,
  DoctorCheckView,
  HostView,
  PermissionView,
  ProviderView,
  SessionView,
  ToastRecord,
  TurnView,
} from './types';

/**
 * The reducer: `(state, event) → state`, pure, and the only writer of what the UI shows
 * (master spec sections 3.3 and 5.4).
 *
 * PURITY IS THE CONTRACT. There is no `fetch`, no `Date.now()`, no `crypto.randomUUID()` and no
 * DOM read anywhere below. Everything that varies with the outside world travels *in the event*:
 * the daemon stamps `ts`, the daemon counts `minutesAgo`, the daemon names the checkpoint. That is
 * what makes `applyEvents(EMPTY_STATE, events)` a total function of the log, and what turns the
 * time-travel acceptance check ("dispatch old events, view state matches expected") into a plain
 * assertion rather than a mock-heavy test.
 *
 * The state is seeded by folding events, not by building an object: `createInitialState()` is
 * `applyEvents(EMPTY_STATE, seedEvents())`. The demo the prototype ships is therefore *the fold of
 * a log*, like every other state in the app - the strongest form of "every UI change is derived
 * from an append-only event stream" available without a daemon.
 *
 * The folded shapes are deliberately the shapes the components already render - `hosts` is the
 * host-grouped tree of spec section 7.3, `providers` is the Provider Hub's card list of 9.10 - so a
 * panel never has to re-derive them. Two fields are bookkeeping: `seq`, the highest sequence folded
 * in (a gap is a replay request, not a lost repaint), and `lastTs`.
 *
 * UI *preferences* are NOT here. `openTabs`, `activeTab`, the shell layout, the theme and which
 * overlay is open are local storage's business (spec section 3.3: "persist to localStorage ONLY for
 * UI prefs"), so they live in src/store/prefs.ts, src/store/layout.ts and src/store/overlays.ts.
 * The daemon owns server state; the browser owns taste.
 */

/** The state before the log has any events: every slice empty, both counters zero. */
export const EMPTY_STATE: AppState = {
  seq: 0,
  lastTs: '',
  hosts: [],
  providers: [],
  registry: [],
  toasts: [],
  toastsIssued: 0,
  turns: [],
  activeTurnId: null,
  checkpoints: [],
  rewindStack: [],
  bridges: [],
  duels: [],
  console: [],
  permission: null,
  resolvedPermissions: {},
  doctor: {},
};

/** A `push` per event, with a deterministic clock so the seed's timestamps are stable per run. */
function seeder(): { events: AppEvent[]; push: (event: SdcpEvent, sessionId?: string | null) => void } {
  const clock = stepClock();
  const events: AppEvent[] = [];

  return {
    events,
    push: (event, sessionId = null) => {
      events.push({ seq: events.length + 1, ts: clock(), event, sessionId, turnId: null });
    },
  };
}

/**
 * The demo log: one event per line of `strings.seed`, in the order a daemon would have emitted
 * them. The relative ages are the prototype's (`now`, `2m`, `2h`, `1d`, `3d`, `5d`) expressed as
 * the minutes a daemon would have reported.
 */
export function seedEvents(): AppEvent[] {
  const { events, push } = seeder();

  push({ type: 'HostStatus', hostId: 'local', name: strings.sidebar.hosts.local, hostType: 'local', status: 'connected', sdcd: strings.seed.sdcdVersion, platform: strings.seed.hostPlatform });
  push({ type: 'HostStatus', hostId: 'vps1', name: strings.sidebar.hosts.prod1, hostType: 'vps', status: 'connected', sdcd: strings.seed.sdcdVersion, platform: 'Debian 12 · x64' });
  push({ type: 'HostStatus', hostId: 'vps2', name: strings.sidebar.hosts.staging2, hostType: 'vps', status: 'degraded', sdcd: strings.seed.sdcdVersion, platform: 'Ubuntu 24.04 · x64' });

  /* The six sessions of the prototype and the state each one is in (spec section 7.3). */
  const sessions: {
    id: string;
    hostId: string;
    title: string;
    prompt: string;
    state: SessionView['state'];
    minutesAgo: number;
    unread: number;
    attention?: SessionView['attention'];
  }[] = [
    { id: 's1', hostId: 'local', title: strings.sidebar.sessions.rateLimiting.title, prompt: strings.sidebar.sessions.rateLimiting.prompt, state: 'waiting', minutesAgo: 0, unread: 0, attention: 'awaiting_approval' },
    { id: 's2', hostId: 'local', title: strings.sidebar.sessions.loginBug.title, prompt: strings.sidebar.sessions.loginBug.prompt, state: 'running', minutesAgo: 2, unread: 0 },
    { id: 's3', hostId: 'local', title: strings.sidebar.sessions.refactorAuth.title, prompt: strings.sidebar.sessions.refactorAuth.prompt, state: 'idle', minutesAgo: 120, unread: 0 },
    { id: 's4', hostId: 'vps1', title: strings.sidebar.sessions.deployScript.title, prompt: strings.sidebar.sessions.deployScript.prompt, state: 'error', minutesAgo: 1440, unread: 1 },
    { id: 's5', hostId: 'vps1', title: strings.sidebar.sessions.logAggregation.title, prompt: strings.sidebar.sessions.logAggregation.prompt, state: 'idle', minutesAgo: 4320, unread: 0 },
    { id: 's6', hostId: 'vps2', title: strings.sidebar.sessions.updateReadme.title, prompt: strings.sidebar.sessions.updateReadme.prompt, state: 'success', minutesAgo: 7200, unread: 0 },
  ];

  for (const session of sessions) {
    push(
      {
        type: 'SessionOpened',
        sessionId: session.id,
        hostId: session.hostId,
        title: session.title,
        prompt: session.prompt,
      },
      session.id,
    );
    push(
      {
        type: 'SessionUpdated',
        sessionId: session.id,
        state: session.state,
        minutesAgo: session.minutesAgo,
        unread: session.unread,
        attention: session.attention ?? null,
      },
      session.id,
    );
  }

  /* The nine provider cards, then the twelve-model registry (spec section 9.10). */
  for (const provider of strings.seed.providers) {
    push({
      type: 'ProviderStatus',
      id: provider.id,
      name: provider.name,
      status: provider.status,
      detail: provider.detail,
      account: provider.account,
      kind: provider.kind,
      logo: provider.logo,
      initial: provider.initial,
    });
  }

  push({ type: 'RegistryLoaded', models: [...strings.seed.models] });

  /* Time Machine: three checkpoints (spec section 14). */
  const checkpoints = [
    { id: 'cp-14', turn: 14, when: 'now', title: strings.rightPanel.timeMachine.entries[0].title, filesHash: 'a4f19c2' },
    { id: 'cp-13', turn: 13, when: '2 min ago', title: strings.rightPanel.timeMachine.entries[1].title, filesHash: '77b0e41' },
    { id: 'cp-12', turn: 12, when: '8 min ago', title: strings.rightPanel.timeMachine.entries[2].title, filesHash: '1de9a30' },
  ];

  for (const checkpoint of checkpoints) {
    push(
      {
        type: 'CheckpointSaved',
        sessionId: 's1',
        checkpoint: {
          id: checkpoint.id,
          turn: checkpoint.turn,
          ts: checkpoint.when,
          title: checkpoint.title,
          thumbnail: null,
          filesHash: checkpoint.filesHash,
          rewindRef: null,
        },
      },
      's1',
    );
  }

  /* The Console's two lines; the first was logged three times, which is its count pill. */
  for (const entry of strings.rightPanel.console.entries) {
    for (let repeat = 0; repeat < entry.count; repeat += 1) {
      push({
        type: 'ConsoleError',
        sessionId: 's1',
        level: entry.level,
        message: entry.message,
        source: entry.source,
        file: entry.file,
        line: entry.line,
      });
    }
  }

  /* Duel mode's seeded pair and the Session Bridge frame (spec sections 16.5, 16.6). */
  push({
    type: 'DuelStarted',
    duelId: 'duel-1',
    sessionId: 's1',
    prompt: strings.turns.prompt,
    engines: strings.rightPanel.duel.panes.map((pane) => pane.engine),
    panes: strings.rightPanel.duel.panes.map((pane) => ({
      engine: pane.engine,
      model: pane.model,
      time: pane.time,
      cost: pane.cost,
      pass: pane.pass,
      headline: pane.headline,
      files: [...pane.files],
    })),
  });

  push({
    type: 'SessionBridged',
    sessionId: 's2',
    turnId: 'turn-8',
    from: 'claude_code',
    to: 'codex',
    model: 'default',
    reason: 'switched mid-turn',
  });

  return events;
}

/** The state the app boots into: the fold of the demo log (spec section 3.3). */
export function createInitialState(): AppState {
  return applyEvents(EMPTY_STATE, seedEvents());
}

/**
 * Folds one event into the state. Returns a NEW object only when the log's bookkeeping changed, so a
 * subscriber that receives an unrelated event does not re-render (zustand compares references).
 */
export function applyEvent(state: AppState, entry: AppEvent): AppState {
  const next = reduce(state, entry);

  return next === state ? state : { ...next, seq: entry.seq, lastTs: entry.ts };
}

/** Folds a list; `applyEvents(EMPTY_STATE, seedEvents())` is the boot state. */
export function applyEvents(state: AppState, entries: readonly AppEvent[]): AppState {
  return entries.reduce<AppState>((current, entry) => applyEvent(current, entry), state);
}

/** The switch. One arm per catalogue entry; each arm returns the whole next state. */
function reduce(state: AppState, entry: AppEvent): AppState {
  const { event } = entry;

  switch (event.type) {
    case 'HostStatus': {
      const existing = state.hosts.find((host) => host.id === event.hostId);
      const host: HostView = {
        id: event.hostId,
        name: event.name || existing?.name || event.hostId,
        type: event.hostType,
        status: event.status,
        sdcd: event.sdcd ?? existing?.sdcd ?? '',
        platform: event.platform ?? existing?.platform ?? '',
        sessions: existing?.sessions ?? [],
      };

      return {
        ...state,
        hosts: existing
          ? state.hosts.map((candidate) => (candidate.id === event.hostId ? host : candidate))
          : [...state.hosts, host],
      };
    }

    case 'ProviderStatus': {
      const existing = state.providers.find((provider) => provider.id === event.id);
      const provider: ProviderView = {
        id: event.id,
        name: event.name ?? existing?.name ?? event.id,
        kind: event.kind ?? existing?.kind ?? 'api-key',
        status: event.status,
        detail: event.detail ?? existing?.detail ?? '',
        account: event.account === undefined ? (existing?.account ?? null) : event.account,
        logo: event.logo ?? existing?.logo ?? 'custom',
        initial:
          event.initial ??
          existing?.initial ??
          (event.name ?? event.id).slice(0, 1).toUpperCase(),
        url: event.url ?? existing?.url,
        protocol: event.protocol ?? existing?.protocol,
      };

      return {
        ...state,
        providers: existing
          ? state.providers.map((candidate) => (candidate.id === event.id ? provider : candidate))
          : [...state.providers, provider],
      };
    }

    case 'RegistryLoaded':
      return { ...state, registry: event.models.map((model) => ({ ...model })) };

    case 'SessionOpened': {
      const host = state.hosts.find((candidate) => candidate.id === event.hostId);

      /* A session only exists inside a host; the daemon always sends `HostStatus` first. */
      if (!host) {
        return state;
      }

      const session: SessionView = {
        id: event.sessionId,
        title: event.title,
        prompt: event.prompt,
        state: 'idle',
        minutesAgo: 0,
        unread: 0,
      };

      return {
        ...state,
        hosts: state.hosts.map((candidate) =>
          candidate.id === event.hostId
            ? { ...candidate, sessions: [session, ...candidate.sessions] }
            : candidate,
        ),
      };
    }

    case 'SessionClosed':
      return {
        ...state,
        hosts: state.hosts.map((host) =>
          host.sessions.some((session) => session.id === event.sessionId)
            ? {
                ...host,
                sessions: host.sessions.filter((session) => session.id !== event.sessionId),
              }
            : host,
        ),
      };

    case 'SessionUpdated':
      return {
        ...state,
        hosts: state.hosts.map((host) => {
          if (!host.sessions.some((session) => session.id === event.sessionId)) {
            return host;
          }

          return {
            ...host,
            sessions: host.sessions.map((session) => {
              if (session.id !== event.sessionId) {
                return session;
              }

              const attention =
                event.attention === null ? undefined : (event.attention ?? session.attention);

              return {
                ...session,
                title: event.title ?? session.title,
                prompt: event.prompt ?? session.prompt,
                state: event.state ?? session.state,
                minutesAgo: event.minutesAgo ?? session.minutesAgo,
                unread: event.unread ?? session.unread,
                attention,
              };
            }),
          };
        }),
      };

    case 'Toast': {
      const record: ToastRecord = {
        id: state.toastsIssued + 1,
        message: event.message,
        action: event.action ?? null,
        holdMs: event.holdMs ?? 3000,
      };

      return { ...state, toasts: [...state.toasts, record], toastsIssued: record.id };
    }

    case 'ToastDismissed':
      return state.toasts.some((toast) => toast.id === event.id)
        ? { ...state, toasts: state.toasts.filter((toast) => toast.id !== event.id) }
        : state;

    case 'PermissionRequested': {
      /* `always_allow` is a decision about the action, so a repeat never asks again. */
      if (state.resolvedPermissions[event.permissionId] === 'always_allow') {
        return state;
      }

      return {
        ...state,
        permission: {
          id: event.permissionId,
          sessionId: event.sessionId,
          turnId: event.turnId ?? null,
          title: event.title,
          sub: event.sub,
          action: event.action,
          target: event.target,
          risk: event.risk,
          explain: event.explain ?? '',
          checkpointId: event.checkpointId ?? null,
        },
      };
    }

    case 'PermissionResolved':
      return {
        ...state,
        permission: state.permission?.id === event.permissionId ? null : state.permission,
        resolvedPermissions: {
          ...state.resolvedPermissions,
          [event.permissionId]: event.decision,
        },
      };

    case 'CheckpointSaved': {
      const checkpoint: CheckpointView = {
        id: event.checkpoint.id,
        sessionId: event.sessionId,
        turn: event.checkpoint.turn,
        /*
         * A seeded checkpoint's `ts` is already the human string (`now`, `2 min ago`) because a pure
         * reducer cannot format a relative age; a live one carries the daemon's RFC 3339 stamp.
         */
        when: event.checkpoint.ts,
        title: event.checkpoint.title ?? '',
        thumbnail: event.checkpoint.thumbnail ?? null,
        filesHash: event.checkpoint.filesHash,
      };

      const others = state.checkpoints.filter((entryPoint) => entryPoint.id !== checkpoint.id);

      return {
        ...state,
        checkpoints: [checkpoint, ...others].sort((a, b) => b.turn - a.turn),
      };
    }

    case 'RewindApplied': {
      if (event.direction === 'forward') {
        const restored = state.rewindStack.slice(0, event.turns);

        return {
          ...state,
          checkpoints: [...restored, ...state.checkpoints].sort((a, b) => b.turn - a.turn),
          rewindStack: state.rewindStack.slice(event.turns),
        };
      }

      const dropped = state.checkpoints.filter(
        (checkpoint) => checkpoint.sessionId === event.sessionId && checkpoint.turn > event.turn,
      );

      return {
        ...state,
        checkpoints: state.checkpoints.filter((checkpoint) => !dropped.includes(checkpoint)),
        rewindStack: [...dropped, ...state.rewindStack],
        turns: state.turns.filter(
          (turn) => !(turn.sessionId === event.sessionId && turn.turnNumber > event.turn),
        ),
      };
    }

    case 'TurnStarted': {
      const turn: TurnView = {
        id: event.turnId,
        turnNumber:
          state.turns.filter((candidate) => candidate.sessionId === event.sessionId).length + 1,
        sessionId: event.sessionId,
        engine: event.engine,
        model: event.model,
        tier: event.tier,
        text: '',
        thinking: '',
        status: 'running',
        stuckForMs: 0,
        tools: [],
        summary: '',
        meta: event.forecast ?? '',
        pass: null,
      };

      return { ...state, turns: [...state.turns, turn], activeTurnId: turn.id };
    }

    case 'TurnDelta':
      return patchTurn(state, event.turnId, (turn) => ({
        ...turn,
        text: turn.text + event.delta,
        status: 'running',
        stuckForMs: 0,
      }));

    case 'ThinkingDelta':
      return patchTurn(state, event.turnId, (turn) => ({
        ...turn,
        thinking: turn.thinking + event.delta,
        status: 'running',
        stuckForMs: 0,
      }));

    case 'ToolCallStarted':
      return patchTurn(state, event.turnId, (turn) => ({
        ...turn,
        tools: [
          ...turn.tools,
          {
            callId: event.callId,
            tool: event.tool,
            name: event.name,
            target: event.target,
            status: 'running',
            meta: '',
            diff: [],
            output: [],
          },
        ],
      }));

    case 'ToolCallOutput':
      return patchTurn(state, event.turnId, (turn) => ({
        ...turn,
        tools: turn.tools.map((tool) =>
          tool.callId === event.callId
            ? { ...tool, output: [...tool.output, { level: event.level, text: event.text }] }
            : tool,
        ),
      }));

    case 'ToolCallCompleted':
      return patchTurn(state, event.turnId, (turn) => ({
        ...turn,
        tools: turn.tools.map((tool) =>
          tool.callId === event.callId
            ? { ...tool, status: event.status, meta: event.meta, diff: event.diff ?? tool.diff }
            : tool,
        ),
      }));

    case 'TurnCompleted':
      return patchTurn(state, event.turnId, (turn) => ({
        ...turn,
        status: turn.status === 'failed' ? 'failed' : 'done',
        summary: event.summary,
        meta: event.meta,
        pass: event.pass ?? turn.pass,
      }));

    case 'StuckDetected':
      return patchTurn(state, event.turnId, (turn) => ({
        ...turn,
        status: 'stuck',
        stuckForMs: event.sinceMs,
      }));

    case 'ErrorRaised':
      return patchTurn(state, event.turnId ?? state.activeTurnId ?? '', (turn) => ({
        ...turn,
        status: 'failed',
        error: {
          title: event.title,
          explanation: event.explanation,
          source: event.source,
          fixable: event.fixable ?? true,
        },
      }));

    case 'ConsoleError': {
      const existing = state.console.find(
        (line) => line.file === event.file && line.line === event.line,
      );

      return existing
        ? {
            ...state,
            console: state.console.map((line) =>
              line === existing ? { ...line, count: line.count + 1 } : line,
            ),
          }
        : {
            ...state,
            console: [
              {
                level: event.level,
                message: event.message,
                source: event.source,
                file: event.file,
                line: event.line,
                count: 1,
              },
              ...state.console,
            ],
          };
    }

    case 'DuelStarted':
      return {
        ...state,
        duels: [
          {
            id: event.duelId,
            sessionId: event.sessionId,
            prompt: event.prompt,
            engines: [...event.engines],
            panes: (event.panes ?? []).map((pane) => ({ ...pane, files: [...pane.files] })),
            kept: null,
            resolved: false,
          },
          ...state.duels,
        ],
      };

    case 'DuelResolved':
      return {
        ...state,
        duels: state.duels.map((duel) =>
          duel.id === event.duelId ? { ...duel, kept: event.kept, resolved: true } : duel,
        ),
      };

    case 'SessionBridged':
      return {
        ...state,
        bridges: [
          {
            sessionId: event.sessionId,
            turnId: event.turnId,
            from: event.from,
            to: event.to,
            model: event.model,
            reason: event.reason ?? '',
          },
          ...state.bridges,
        ],
      };
  }

  /* Exhaustiveness: every catalogue entry has an arm above, so this is unreachable. */
  return state;
}

/** Applies a change to one turn, leaving every other turn's object identity alone. */
function patchTurn(
  state: AppState,
  turnId: string,
  change: (turn: TurnView) => TurnView,
): AppState {
  if (turnId === '' || !state.turns.some((turn) => turn.id === turnId)) {
    return state;
  }

  return {
    ...state,
    turns: state.turns.map((turn) => (turn.id === turnId ? change(turn) : turn)),
  };
}

/* ------------------------------------------------------------------------------------------------
 * Derivations. Pure functions of the folded state, named after the facts the UI asks for - so a
 * panel reads `sessionCount(hosts)` rather than re-inventing a reduce, and the two places that need
 * "is the fleet healthy" cannot disagree.
 * ---------------------------------------------------------------------------------------------- */

/** A session together with the host it belongs to - what every renderer actually needs. */
export interface HostSession {
  host: HostView;
  session: SessionView;
}

/**
 * Relative age, exactly as the prototype's `.session-time` shows it: `now`, then minutes, then
 * hours, then days. One unit, rounded down - `119` minutes is `1h`, not `2h`.
 */
export function formatRelativeTime(minutesAgo: number): string {
  if (minutesAgo <= 0) {
    return 'now';
  }

  if (minutesAgo < 60) {
    return `${minutesAgo}m`;
  }

  if (minutesAgo < 60 * 24) {
    return `${Math.floor(minutesAgo / 60)}h`;
  }

  return `${Math.floor(minutesAgo / (60 * 24))}d`;
}

/**
 * Waiting first (spec gap #12): blocked sessions come first, everything else keeps the order its
 * host gave it. Two filters rather than a comparator, so two blocked sessions keep their relative
 * order.
 */
export function orderedSessions(sessions: readonly SessionView[]): SessionView[] {
  const blocked = sessions.filter((session) => session.attention !== undefined);
  const rest = sessions.filter((session) => session.attention === undefined);

  return [...blocked, ...rest];
}

/** True while a session should be visible in the sidebar's filtered list (spec section 7.3). */
export function matchesFilter(session: SessionView, filter: string): boolean {
  const query = filter.trim().toLowerCase();

  if (query === '') {
    return true;
  }

  return (
    session.title.toLowerCase().includes(query) || session.prompt.toLowerCase().includes(query)
  );
}

/** The session and its host, or null when the id is unknown (a closed host, a stale tab). */
export function findSession(hosts: readonly HostView[], id: string | null): HostSession | null {
  if (id === null) {
    return null;
  }

  for (const host of hosts) {
    const session = host.sessions.find((candidate) => candidate.id === id);

    if (session) {
      return { host, session };
    }
  }

  return null;
}

/** Every session, across every host - what the Search overlay of spec section 9.4 walks. */
export function allSessions(hosts: readonly HostView[]): HostSession[] {
  return hosts.flatMap((host) => host.sessions.map((session) => ({ host, session })));
}

/** The status bar's `N chats`. */
export function sessionCount(hosts: readonly HostView[]): number {
  return hosts.reduce((total, host) => total + host.sessions.length, 0);
}

/** Hosts that cannot take work right now - what `#degradedBanner` reports (spec section 7.4). */
export function unreachableHosts(hosts: readonly HostView[]): HostView[] {
  return hosts.filter((host) => host.status === 'degraded' || host.status === 'offline');
}

/** The status bar's single connection dot (spec section 7.15): the worst state wins. */
export type ConnectionState = 'success' | 'waiting' | 'error';

export function connectionState(hosts: readonly HostView[]): ConnectionState {
  if (hosts.some((host) => host.status === 'offline')) {
    return 'error';
  }

  if (hosts.some((host) => host.status === 'degraded' || host.status === 'connecting')) {
    return 'waiting';
  }

  return 'success';
}

/** True while any provider still needs authentication - the `has-dot` condition (7.1, row 7). */
export function anyProviderNeedsAuth(providers: readonly ProviderView[]): boolean {
  return providers.some((provider) => provider.status === 'needs-auth');
}

/** How many providers are usable - the status bar's count (7.15). */
export function connectedProviderCount(providers: readonly ProviderView[]): number {
  return providers.filter((provider) => provider.status === 'connected').length;
}

/** Distinct errors - the Console tab's badge (spec section 7.8). */
export function consoleErrorCount(lines: readonly ConsoleLine[]): number {
  return lines.filter((line) => line.level === 'error').length;
}

/** The local host, or the first one - what the About tab's "This host" line names. */
export function localHost(hosts: readonly HostView[]): HostView | null {
  return hosts.find((host) => host.type === 'local') ?? hosts[0] ?? null;
}

/* ------------------------------------------------------------------------------------------------
 * Selectors. These are the app store's read API (spec section 3.3: "Zustand store exposes
 * dispatch(event), selectActiveSession(), selectHosts()").
 * ---------------------------------------------------------------------------------------------- */

export function selectHosts(state: AppState): HostView[] {
  return state.hosts;
}

/** The focused session's ref, given the active tab id from the UI-prefs store. */
export function selectActiveSession(state: AppState, sessionId: string | null): HostSession | null {
  return findSession(state.hosts, sessionId ?? null);
}

export function selectToasts(state: AppState): ToastRecord[] {
  return state.toasts;
}

export function selectRegistry(state: AppState): RegistryModel[] {
  return state.registry;
}

export function selectConsole(state: AppState): ConsoleLine[] {
  return state.console;
}

/** The Time Machine list for a session, newest first; every session's when `sessionId` is null. */
export function selectCheckpoints(state: AppState, sessionId: string | null): CheckpointView[] {
  return sessionId === null
    ? state.checkpoints
    : state.checkpoints.filter((checkpoint) => checkpoint.sessionId === sessionId);
}

/** The newest duel for a session, so the Duel tab has something to draw (spec section 16.6). */
export function selectDuel(state: AppState, sessionId: string | null) {
  return state.duels.find((duel) => sessionId === null || duel.sessionId === sessionId) ?? null;
}

/** The live (or newest) turn for a session - what `ErrorCard`'s `Fix this` is seeded from. */
export function selectTurn(state: AppState, sessionId: string | null): TurnView | null {
  const own =
    sessionId === null ? state.turns : state.turns.filter((turn) => turn.sessionId === sessionId);

  return own.at(-1) ?? null;
}

export function selectPermission(state: AppState): PermissionView | null {
  return state.permission;
}

export function selectDoctor(state: AppState, hostId: string | null): DoctorCheckView[] {
  return hostId === null ? [] : (state.doctor[hostId] ?? []);
}

/**
 * Records a doctor run. It is a state patch rather than an event because the *checks* travel as
 * `host.doctor`'s result - the event catalogue carries what happened, and a doctor run is a read.
 * `intents.runDoctor()` is the caller, and it also raises the toast the acceptance list wants.
 */
export function withDoctorRun(
  state: AppState,
  hostId: string,
  checks: readonly DoctorCheckView[],
): AppState {
  return { ...state, doctor: { ...state.doctor, [hostId]: [...checks] } };
}

