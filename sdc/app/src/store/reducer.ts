import type { HostRecord, ProjectRecord, ProviderRecord, RegistryModel } from '../../../protocol/types';
import type {
  AppEvent,
  AppState,
  CheckpointView,
  VerifyView,
  ConsoleLine,
  DoctorCheckView,
  HostView,
  PermissionView,
  ProjectView,
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
  /** The folders chats can work in (0.7.6), filled by `project.list` - see `withProjects`. */
  projects: [],
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
  verifies: [],
};


/**
 * The state the app boots into: **empty** (spec section 3.3).
 *
 * There used to be a demo log here - three hosts, six chats, nine providers, twelve models, a handful
 * of turns - so that the shell had something to draw on first run. It made every screenshot a
 * screenshot of fiction: a window that looked connected, busy and expensive while the daemon behind it
 * was answering nothing (and, more than once, while it was not running at all). What the app shows now
 * is the log: host.status, session.list, provider.list and the turn events. An empty window is
 * an honest one, and the empty states of spec section 7.13 are what make it readable.
 */
export function createInitialState(): AppState {
  return EMPTY_STATE;
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

/** Milliseconds between two log stamps; 0 when either is not a date (a seeded event's human `ts`). */
function between(from: string, to: string): number {
  const ms = Date.parse(to) - Date.parse(from);

  return Number.isFinite(ms) && ms > 0 ? ms : 0;
}

/**
 * Close the stretch of thinking a turn is in, at `ts`.
 *
 * Any event that is not more thinking ends a stretch - the answer starting, a tool call, the end of
 * the turn - and a model that thinks again between tool calls opens a new one, so the total is time
 * spent *thinking*, not time since the first thought.
 */
function endThinking(turn: TurnView, ts: string): TurnView {
  if (turn.thinkingSince === null) {
    return turn;
  }

  return { ...turn, thinkingMs: turn.thinkingMs + between(turn.thinkingSince, ts), thinkingSince: null };
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
        /* The sentence and the fingerprint travel with the status (0.7.13): `untrusted` is a *question*
           and the answer needs the exact string `host.trust` takes, so it is a field rather than
           something a card would have to read out of a paragraph. */
        detail: event.detail ?? existing?.detail ?? '',
        hostKey: event.hostKey ?? existing?.hostKey ?? '',
        pinned: existing?.pinned ?? '',
        address: existing?.address ?? '',
        sessions: existing?.sessions ?? [],
      };

      return {
        ...state,
        hosts: existing
          ? state.hosts.map((candidate) => (candidate.id === event.hostId ? host : candidate))
          : [...state.hosts, host],
      };
    }

    case 'HostRemoved':
      /* The row, its sessions and everything they carried are gone. Nothing is left behind on the
         screen either: a host that is no longer in the list cannot be switched to. */
      return {
        ...state,
        hosts: state.hosts.filter((host) => host.id !== event.hostId),
      };

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

      /*
       * Idempotent, and this is not decoration.
       *
       * The same `SessionOpened` legitimately reaches this reducer more than once: the bridge asks for
       * the log on every connect (`event.list since=0` on a fresh process) and forwards the replay one
       * event at a time, while `session.list` may already have listed the session in between. It also
       * arrived twice when the app opened two notification sockets. Appending unconditionally drew one
       * row per copy - the "clicking New chat opens two chats" report - and the duplicate React keys
       * that came with it left ghost rows behind on the next render.
       *
       * The session id is the identity, so a second open of an id that is already on screen is the
       * same session and changes nothing.
       */
      const known = state.hosts.some((candidate) =>
        candidate.sessions.some((session) => session.id === event.sessionId),
      );

      if (known) {
        return state;
      }

      const session: SessionView = {
        id: event.sessionId,
        title: event.title,
        prompt: event.prompt,
        state: 'idle',
        minutesAgo: 0,
        unread: 0,
        /* `Open folder` creates the chat *with* its folder, so the window knows which directory it is in
           from the first render rather than after a second round trip (0.7.6). */
        projectId: event.projectId ?? null,
        projectRoot: event.projectRoot ?? null,
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
                /* `Open folder` on an existing chat (0.7.6): the folder the turn will run in. An event
                   that does not mention a project leaves what the session already had alone. */
                projectId: event.projectId ?? session.projectId,
                projectRoot: event.projectRoot ?? session.projectRoot,
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
        /* The envelope says which turn wrote it; the record's own `turn` is an ordinal, not an id. */
        turnId: entry.turnId ?? null,
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

      /*
       * The checkpoint rewound to and every later one leave the list (v4: the chosen one is included - it
       * is the state the folder is now in). The conversation stays: `event.turn` is a checkpoint's ordinal,
       * not a turn number, and the filter that compared the two never matched anything. Keeping the turns
       * on screen is also the honest picture - they happened, and their changes are what was undone.
       */
      const dropped = state.checkpoints.filter(
        (checkpoint) => checkpoint.sessionId === event.sessionId && checkpoint.turn >= event.turn,
      );

      return {
        ...state,
        checkpoints: state.checkpoints.filter((checkpoint) => !dropped.includes(checkpoint)),
        rewindStack: [...dropped, ...state.rewindStack],
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
        prompt: event.prompt,
        text: '',
        thinking: '',
        thinkingMs: 0,
        thinkingSince: null,
        plan: [],
        startedAt: entry.ts,
        status: 'running',
        stuckForMs: 0,
        tools: [],
        summary: '',
        /* The totals line is filled by `TurnCompleted`, which carries what the run actually cost.
           Nothing here guesses a price: this build has no way to know one. */
        meta: '',
        pass: null,
      };

      return { ...state, turns: [...state.turns, turn], activeTurnId: turn.id };
    }

    case 'TurnDelta':
      return patchTurn(state, event.turnId, (turn) => ({
        ...endThinking(turn, entry.ts),
        text: turn.text + event.delta,
        status: 'running',
        stuckForMs: 0,
      }));

    case 'ThinkingDelta':
      return patchTurn(state, event.turnId, (turn) => ({
        ...turn,
        thinking: turn.thinking + event.delta,
        thinkingSince: turn.thinkingSince ?? entry.ts,
        status: 'running',
        stuckForMs: 0,
      }));

    case 'ToolCallStarted':
      return patchTurn(state, event.turnId, (turn) => ({
        ...endThinking(turn, entry.ts),
        tools: [
          ...turn.tools,
          {
            callId: event.callId,
            startedAt: entry.ts,
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

    case 'VerifyUpdated': {
      /* Whole each time: the newest snapshot of a run replaces the last one, in place. */
      const run: VerifyView = {
        verifyId: event.verifyId,
        sessionId: event.sessionId,
        turnId: event.turnId ?? null,
        state: event.state,
        pass: event.pass,
        checks: event.checks,
        review: event.review,
        note: event.note,
      };
      const others = state.verifies.filter((existing) => existing.verifyId !== run.verifyId);

      return { ...state, verifies: [...others, run] };
    }

    case 'PlanUpdated':
      /* Whole each time: the newest checklist replaces the last, so a step that finished is ticked. */
      return patchTurn(state, event.turnId, (turn) => ({
        ...turn,
        plan: event.steps.map((step) => ({ text: step.text, status: step.status })),
      }));

    case 'TurnCompleted': {
      const next = patchTurn(state, event.turnId, (turn) => ({
        ...endThinking(turn, entry.ts),
        status: turn.status === 'failed' ? 'failed' : 'done',
        summary: event.summary,
        meta: event.meta,
        pass: event.pass ?? turn.pass,
        /* A tool cannot still be running when its turn has ended. An engine that never reported the
           result (older Claude Code logs) left the card spinning for ever; it now says it ended. */
        tools: turn.tools.map((tool) =>
          tool.status === 'running' ? { ...tool, status: 'done', meta: tool.meta === '' ? 'ended with the turn' : tool.meta } : tool,
        ),
      }));

      /* A question the turn was still asking ends with it: a stopped agent is no longer waiting, and a
         dialog left open would ask the person to approve something nothing will do. */
      return next.permission !== null && next.permission.turnId === event.turnId ? { ...next, permission: null } : next;
    }

    case 'StuckDetected':
      return patchTurn(state, event.turnId, (turn) => ({
        ...turn,
        status: 'stuck',
        stuckForMs: event.sinceMs,
      }));

    case 'ErrorRaised':
      return patchTurn(state, event.turnId ?? state.activeTurnId ?? '', (turn) => ({
        ...endThinking(turn, entry.ts),
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

    /* The catalogue rows live in the model store, not here: `lib/sdcp.ts` answers this event by
       calling `refreshCatalog()`, because the reducer must stay a pure fold with no I/O in it. */
    case 'ModelsUpdated':
      return state;
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
 * Records the answer to `provider.list` - the cards the Provider Hub, the topbar's plug and the
 * status bar's count are built from.
 *
 * A state patch rather than a stream of events, for the same reason `withWorkspace` is one: a list is
 * a *read*'s result, and re-emitting eleven `ProviderStatus` events on every launch would append a
 * copy of the catalogue to the log each time a window opened.
 *
 * This is the missing half of 0.6.0's fix. That release started calling `provider.list` at startup,
 * which was right, but nothing folded the answer: the daemon replies with the list and appends
 * nothing, so the Hub drew `0 connected · None yet` while the daemon was answering eleven cards -
 * "the connect screen is empty", exactly as reported. A read whose result nobody keeps is the same as
 * no read at all.
 *
 * `ProviderStatus` remains the event for a *change* (`provider.save`, a CLI login finishing), so both
 * paths converge on the same array.
 */
export function withProviders(state: AppState, providers: readonly ProviderRecord[]): AppState {
  return {
    ...state,
    providers: providers.map(
      (provider): ProviderView => ({
        id: provider.id,
        name: provider.name,
        kind: provider.kind,
        status: provider.status,
        detail: provider.detail ?? '',
        account: provider.account ?? null,
        logo: provider.logo ?? 'custom',
        initial: provider.initial ?? provider.name.slice(0, 1).toUpperCase(),
        ...(provider.url === null || provider.url === undefined ? {} : { url: provider.url }),
        ...(provider.protocol === null || provider.protocol === undefined
          ? {}
          : { protocol: provider.protocol }),
      }),
    ),
  };
}

/**
 * Records the answer to `session.list` - the host-and-session tree the daemon already had.
 *
 * A state patch rather than a stream of events, for the same reason `withDoctorRun` is one: the
 * *list* is a read's result, and re-emitting an event per host on every launch would append a copy
 * of the whole tree to the log each time the window opened. What is folded from events is what
 * *happens* (`HostStatus`, `HostRemoved`, `SessionOpened`); what is read is read.
 *
 * Replacing rather than merging is the honest direction: the daemon's rows are the authority, so a
 * host the daemon does not have is a host this window must stop drawing. Two fields survive a replace,
 * because the list does not carry them: `sdcd` (`host.status` reported it, and it is still true) and
 * `platform` (the same - the `hosts` table has no such column, so a replace was blanking it and About's
 * `This host` row read `Local · ` with a separator pointing at nothing).
 */
export function withWorkspace(state: AppState, hosts: readonly HostRecord[]): AppState {
  return {
    ...state,
    hosts: hosts.map((host) => {
      const known = state.hosts.find((candidate) => candidate.id === host.hostId);

      return {
        id: host.hostId,
        name: host.name,
        type: host.hostType,
        status: host.status,
        sdcd: known?.sdcd ?? '',
        platform: host.platform ?? known?.platform ?? '',
        /* The row does not carry the sentence or the fingerprint - those arrive with a `HostStatus` -
           so the last thing the log said about this host is kept rather than blanked on every read.
           (This is the same rule `sdcd` and `platform` already followed.) */
        detail: known?.detail ?? '',
        hostKey: known?.hostKey ?? '',
        /* The row's own copy of the decision: a window that never saw the `host.trust` event still knows
           which key this host is (0.7.13). */
        pinned: host.hostKey ?? known?.pinned ?? '',
        /* The address comes from the row, and the port is the fact 0.7.0 dropped: `user@host:8443`. */
        address: host.target === null || host.target === undefined
          ? ''
          : host.port === null || host.port === undefined
            ? host.target
            : `${host.target}:${host.port}`,
        sessions: host.sessions.map((session) => ({
          id: session.sessionId,
          title: session.title,
          prompt: session.prompt,
          state: session.state,
          minutesAgo: session.minutesAgo,
          unread: session.unread,
          /* The folder the chat works in (0.7.6): the engines run there, and the prompt area says so. */
          projectId: session.projectId ?? null,
          projectRoot: session.projectRoot ?? null,
          ...(session.attention === null || session.attention === undefined
            ? {}
            : { attention: session.attention }),
        })),
      };
    }),
  };
}

/**
 * Records `project.list` - the folders this host has opened (0.7.6).
 *
 * A state patch rather than a stream of events, for the same reason `withWorkspace` is one: the list is
 * a *read's* result. The alternative was a new `ProjectAdded` event in the catalogue, and a folder is not
 * something that *happens* to a chat the way a turn does - it is where the chat is. A window that reloads
 * asks again, and a second window sees the change on its next ask, which is the same contract
 * `session.list` has had since 0.6.1.
 *
 * Replacing rather than merging, because the daemon's rows are the authority: a folder the daemon does
 * not have is a folder this window must stop offering.
 */
export function withProjects(state: AppState, projects: readonly ProjectRecord[]): AppState {
  return {
    ...state,
    projects: projects.map(
      (project): ProjectView => ({
        id: project.projectId,
        hostId: project.hostId,
        root: project.root,
        name: project.name,
        chats: project.chats,
      }),
    ),
  };
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

