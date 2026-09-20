import type {
  Envelope,
  ErrorCode,
  Notification,
  Response,
  SdcpEvent,
} from '../../../protocol/types';
import { strings } from '../strings';
import type { LoopbackDaemon } from './transport';

/**
 * The in-process daemon - `sdcd`, in miniature, for the browser and for tests.
 *
 * Why it exists: a WebView cannot open a unix socket, so in `pnpm dev` (and in the browser-based
 * verification harness) there is nothing on the other end of `sdcp_call`. Rather than let the UI
 * grow a "fake mode" of its own - which would be a second code path for every flow, and the exact
 * thing principle P4 forbids - the fake lives *behind the protocol*: it is an SDCP peer that
 * answers envelopes and pushes notifications, and the UI cannot tell the difference. Every flow
 * (the 1.2s key test, the 1.4s host connect, the streaming turn) is timed here rather than faked in
 * a component.
 *
 * When the Tauri bridge is present, `lib/sdcp.ts` picks the real transport and this file is never
 * constructed. The Rust `sdcd` crate is the same peer with real engines behind it: the timings,
 * the event order and the error shapes below are copied from it, so a UI run against this daemon is
 * a UI run against sdcd.
 */

/** Timings, in one place, so a test can shorten them and the code keeps its shape. */
export const DEMO_TIMINGS = {
  /** `provider.test` - spec section 9.10's "spinner for 1.2s then OK · 12 models". */
  keyTest: 1200,
  /** The OAuth round trip of the subscription flow. */
  oauth: 1500,
  /** `host.add` - an SSH connect, per spec section 9.12. */
  connect: 1400,
  /** Gap between two `TurnDelta` notifications of a streamed answer. */
  delta: 120,
  /** How long a tool call "runs" before it reports. */
  tool: 600,
  /** The native-engine timeout that raises `StuckDetected` (spec section 12.9). */
  stuck: 20000,
} as const;

type Push = (event: SdcpEvent, meta?: Partial<Notification>) => void;
type Reply = (response: Response) => void;

/** One in-flight turn, so `cancel`/`kill` know what they are stopping. */
interface LiveTurn {
  turnId: string;
  sessionId: string;
  engine: string;
  model: string;
  timers: number[];
}

/**
 * A tiny scheduler. `Later.push()` returns a handle so a cancelled turn can clear its remaining
 * timers - which is what makes `Esc` (interrupt) actually stop the stream instead of letting it
 * trickle on behind the toast.
 */
class Clock {
  private handles = new Set<number>();

  after(ms: number, action: () => void): number {
    const handle = window.setTimeout(() => {
      this.handles.delete(handle);
      action();
    }, ms);

    this.handles.add(handle);

    return handle;
  }

  every(ms: number, times: number, action: (index: number) => void): number[] {
    return Array.from({ length: times }, (_unused, index) => this.after(ms * (index + 1), () => action(index)));
  }

  cancel(handles: readonly number[]): void {
    for (const handle of handles) {
      window.clearTimeout(handle);
      this.handles.delete(handle);
    }
  }
}

type Params = Record<string, unknown>;

const text = (params: Params, key: string, fallback = ''): string =>
  typeof params[key] === 'string' ? (params[key] as string) : fallback;

const ok = (reply: Reply, envelope: Envelope, result: Record<string, unknown>): void => {
  reply({ v: '0.1', id: envelope.id, result });
};

const fail = (
  reply: Reply,
  envelope: Envelope,
  code: ErrorCode,
  message: string,
): void => {
  reply({ v: '0.1', id: envelope.id, error: { code, message } });
};

/**
 * The daemon. One instance per window; `lib/sdcp.ts` owns it.
 *
 * Sessions and hosts are tracked only far enough to make the flows coherent (a turn needs a
 * session id, a rewind needs a checkpoint list); the app's own event log is the record, exactly as
 * it is with the real daemon.
 */
export class DemoDaemon implements LoopbackDaemon {
  private readonly clock = new Clock();
  private readonly turns = new Map<string, LiveTurn>();
  private readonly checkpoints: { id: string; sessionId: string; turn: number; title: string }[] = [
    { id: 'cp-14', sessionId: 's1', turn: 14, title: strings.rightPanel.timeMachine.entries[0].title },
    { id: 'cp-13', sessionId: 's1', turn: 13, title: strings.rightPanel.timeMachine.entries[1].title },
    { id: 'cp-12', sessionId: 's1', turn: 12, title: strings.rightPanel.timeMachine.entries[2].title },
  ];

  private nextSession = 200;
  private nextTurn = 20;
  private nextPermission = 1;

  handle(envelope: Envelope, push: Push, reply: Reply): void {
    const params = (envelope.params ?? {}) as Params;

    switch (envelope.method) {
      case 'host.status':
        push({
          type: 'HostStatus',
          hostId: 'local',
          name: strings.sidebar.hosts.local,
          hostType: 'local',
          status: 'connected',
          sdcd: strings.seed.sdcdVersion,
          platform: strings.seed.hostPlatform,
        });
        ok(reply, envelope, {
          hostId: 'local',
          name: strings.sidebar.hosts.local,
          type: 'local',
          status: 'connected',
          sdcd: strings.seed.sdcdVersion,
        });
        return;

      case 'host.doctor': {
        const checks = strings.seed.doctor.map((check) => ({
          id: check.id,
          label: check.label,
          state: check.state,
          detail: check.detail,
          fix: check.fix ?? undefined,
        }));

        ok(reply, envelope, { checks });
        return;
      }

      case 'host.add':
        return this.addHost(envelope, params, push, reply);

      case 'session.open': {
        const sessionId = `n${this.nextSession++}`;
        const title = text(params, 'title', strings.sidebar.sessions.newChat.title);
        const prompt = text(params, 'prompt', strings.sidebar.sessions.newChat.prompt);
        const hostId = text(params, 'hostId', 'local');

        push({ type: 'SessionOpened', sessionId, hostId, title, prompt }, { sessionId });
        ok(reply, envelope, { sessionId });
        return;
      }

      case 'session.close':
        push({ type: 'SessionClosed', sessionId: text(params, 'sessionId') });
        ok(reply, envelope, {});
        return;

      case 'session.update': {
        const sessionId = text(params, 'sessionId');

        push(
          {
            type: 'SessionUpdated',
            sessionId,
            title: typeof params.title === 'string' ? params.title : undefined,
            state: typeof params.state === 'string' ? (params.state as 'idle') : undefined,
            minutesAgo: 0,
          },
          { sessionId },
        );
        ok(reply, envelope, {});
        return;
      }

      case 'session.list':
        ok(reply, envelope, { sessions: [] });
        return;

      case 'engine.start':
        return this.startEngine(envelope, params, push, reply);

      case 'engine.cancel':
      case 'engine.kill':
        return this.stopEngine(envelope, params, push, reply);

      case 'engine.switch':
        return this.switchEngine(envelope, params, push, reply);

      case 'engine.status': {
        const turn = this.turns.get(text(params, 'turnId'));

        ok(reply, envelope, {
          state: turn ? 'running' : 'idle',
          engine: turn?.engine ?? 'claude_code',
          model: turn?.model ?? 'sonnet',
        });
        return;
      }

      case 'provider.list':
        ok(reply, envelope, {
          providers: strings.seed.providers.map((provider) => ({
            id: provider.id,
            name: provider.name,
            kind: provider.kind,
            status: provider.status,
            detail: provider.detail,
            account: provider.account,
            logo: provider.logo,
          })),
        });
        return;

      case 'provider.test':
        return this.testProvider(envelope, params, reply);

      case 'provider.save':
        return this.saveProvider(envelope, params, push, reply);

      case 'provider.remove':
        push({ type: 'ProviderStatus', id: text(params, 'id'), status: 'available', account: null });
        ok(reply, envelope, { removed: true });
        return;

      case 'provider.oauth.open':
        return this.openOAuth(envelope, params, reply);

      case 'provider.oauth.callback':
        return this.finishOAuth(envelope, params, push, reply);

      case 'provider.local.doctor':
        return this.localDoctor(envelope, push, reply);

      case 'provider.registry.list':
        ok(reply, envelope, { models: strings.seed.models.map((model) => ({ ...model })) });
        return;

      case 'provider.registry.set': {
        const id = text(params, 'id');
        const enabled = params.enabled === true;

        push({
          type: 'RegistryLoaded',
          models: strings.seed.models.map((model) =>
            model.id === id ? { ...model, enabled } : { ...model },
          ),
        });
        push({ type: 'Toast', message: strings.hub.modelToggled(id, enabled) });
        ok(reply, envelope, {});
        return;
      }

      case 'permission.request': {
        const permissionId = `perm-${this.nextPermission++}`;

        push({
          type: 'PermissionRequested',
          permissionId,
          sessionId: text(params, 'sessionId', 's1'),
          turnId: text(params, 'turnId', this.lastTurnId()),
          title: strings.permission.title,
          sub: strings.permission.sub,
          action: text(params, 'action', 'delete'),
          target: text(params, 'target', strings.permission.target),
          risk: (params.risk === 'DANGEROUS' ? 'DANGEROUS' : 'MUTATING') as 'MUTATING',
          explain: strings.permission.explain,
          checkpointId: this.checkpoints[0]?.id ?? null,
        });
        ok(reply, envelope, { permissionId });
        return;
      }

      case 'permission.resolve': {
        const decision = text(params, 'decision', 'deny') as
          | 'allow_once'
          | 'always_allow'
          | 'deny'
          | 'show_me';

        push({ type: 'PermissionResolved', permissionId: text(params, 'permissionId'), decision });
        push({
          type: 'Toast',
          message:
            decision === 'deny' || decision === 'show_me'
              ? strings.permission.denied
              : strings.permission.granted,
        });
        ok(reply, envelope, {});
        return;
      }

      case 'checkpoint.list':
        ok(reply, envelope, {
          checkpoints: this.checkpoints.map((checkpoint) => ({
            id: checkpoint.id,
            turn: checkpoint.turn,
            ts: strings.rightPanel.timeMachine.current,
            title: checkpoint.title,
            thumbnail: null,
            filesHash: `hash-${checkpoint.id}`,
            rewindRef: null,
          })),
        });
        return;

      case 'checkpoint.create': {
        const turn = this.checkpoints.length + 13;
        const id = `cp-${turn + 1}`;

        this.checkpoints.unshift({ id, sessionId: text(params, 'sessionId', 's1'), turn, title: text(params, 'title', 'Checkpoint') });
        push(
          {
            type: 'CheckpointSaved',
            sessionId: text(params, 'sessionId', 's1'),
            checkpoint: {
              id,
              turn,
              ts: 'now',
              title: text(params, 'title', 'Checkpoint'),
              thumbnail: null,
              filesHash: `hash-${id}`,
              rewindRef: null,
            },
          },
          { sessionId: text(params, 'sessionId', 's1') },
        );
        ok(reply, envelope, { checkpointId: id });
        return;
      }

      case 'checkpoint.restore':
        ok(reply, envelope, { restored: 1 });
        return;

      case 'rewind.apply':
        return this.rewind(envelope, params, push, reply);

      case 'rewind.redo':
        return this.redo(envelope, params, push, reply);

      case 'duel.start': {
        const engines = Array.isArray(params.engines) ? (params.engines as string[]) : [];

        push({
          type: 'DuelStarted',
          duelId: `duel-${this.nextTurn++}`,
          sessionId: text(params, 'sessionId', 's1'),
          prompt: text(params, 'prompt', strings.turns.prompt),
          engines,
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
        ok(reply, envelope, { duelId: `duel-${this.nextTurn}` });
        return;
      }

      case 'duel.keep': {
        const keep = text(params, 'keep');

        push({ type: 'DuelResolved', duelId: text(params, 'duelId'), kept: keep });
        push({ type: 'Toast', message: strings.rightPanel.duel.kept(keep) });
        ok(reply, envelope, {});
        return;
      }

      case 'duel.discard':
        push({ type: 'DuelResolved', duelId: text(params, 'duelId'), kept: null });
        ok(reply, envelope, {});
        return;

      case 'console.attach': {
        const sessionId = text(params, 'sessionId', 's1');

        push({
          type: 'ConsoleError',
          sessionId,
          level: 'error',
          message: 'Uncaught ReferenceError: handleSubmit is not defined',
          source: 'at LoginForm.tsx:42:11',
          file: 'LoginForm.tsx',
          line: 42,
        });
        ok(reply, envelope, { attached: true });
        return;
      }

      case 'console.detach':
        ok(reply, envelope, { detached: true });
        return;

      case 'event.subscribe':
        ok(reply, envelope, { fromSeq: 0 });
        return;

      case 'event.list':
        ok(reply, envelope, { events: [] });
        return;

      case 'event.append':
        ok(reply, envelope, { seq: 0 });
        return;

    }

    /* Anything the UI has no business calling in the demo build answers honestly. */
    fail(reply, envelope, 'unsupported', `${envelope.method} is not implemented by this daemon`);
  }

  /* ---------------------------------------------------------------------------------------------
   * The flows. Each one is the daemon half of a spec flow, with the timings the prototype used.
   * ------------------------------------------------------------------------------------------- */

  /** Spec section 9.12: an SSH target is `connecting` for 1.4s, then connected with a sample chat. */
  private addHost(envelope: Envelope, params: Params, push: Push, reply: Reply): void {
    const type = text(params, 'type', 'ssh');

    if (type === 'local') {
      ok(reply, envelope, { hostId: 'local' });
      return;
    }

    const target = text(params, 'target');
    const label = text(params, 'label') || target.split('@')[1]?.split(':')[0] || 'new-host';
    const hostId = `h${this.nextSession++}`;

    push(
      {
        type: 'HostStatus',
        hostId,
        name: label,
        hostType: 'vps',
        status: 'connecting',
        sdcd: strings.seed.sdcdVersion,
        platform: 'linux · x64',
      },
    );
    push({ type: 'Toast', message: strings.addHost.connecting(label) });

    this.clock.after(DEMO_TIMINGS.connect, () => {
      push({
        type: 'HostStatus',
        hostId,
        name: label,
        hostType: 'vps',
        status: 'connected',
        sdcd: strings.seed.sdcdVersion,
        platform: 'linux · x64',
      });
      push(
        {
          type: 'SessionOpened',
          sessionId: `n${this.nextSession++}`,
          hostId,
          title: strings.addHost.welcomeTitle(label),
          prompt: strings.addHost.welcomePrompt,
        },
      );
      push({ type: 'Toast', message: strings.addHost.connected(label) });
    });

    ok(reply, envelope, { hostId });
  }

  /**
   * Spec section 9.10, flow 1: 1.2s of spinner, then the structured OK.
   *
   * `verified: true` because this daemon *is* the provider's stand-in: it asserts the happy path (the
   * demo the UI is built against). The real daemon answers `verified: false` for a saved key and says
   * in `detail` that the provider was not contacted, which is the difference the two modes are
   * supposed to have.
   */
  private testProvider(envelope: Envelope, params: Params, reply: Reply): void {
    if (text(params, 'key') === '') {
      fail(reply, envelope, 'bad_request', strings.hub.testEmpty);
      return;
    }

    this.clock.after(DEMO_TIMINGS.keyTest, () => {
      ok(reply, envelope, {
        ok: true,
        verified: true,
        models: strings.seed.testModelCount,
        detail: strings.hub.testOk(strings.seed.testModelCount),
      });
    });
  }

  /** Saving a key writes it to the keychain, so the UI only ever learns the masked label. */
  private saveProvider(envelope: Envelope, params: Params, push: Push, reply: Reply): void {
    const id = text(params, 'id');
    const key = text(params, 'key');
    const seed = strings.seed.providers.find((provider) => provider.id === id);
    const name = seed?.name ?? id;
    const account = key === '' ? (text(params, 'label') || 'configured') : `${key.slice(0, 5)}…${key.slice(-4)}`;

    push({
      type: 'ProviderStatus',
      id,
      name,
      status: 'connected',
      account,
      detail: seed?.detail,
      kind: seed?.kind,
      logo: seed?.logo,
      initial: seed?.initial,
      url: text(params, 'url') || undefined,
      protocol: text(params, 'protocol') || undefined,
      models: strings.seed.testModelCount,
    });
    push({ type: 'Toast', message: strings.hub.connectToast(name) });
    ok(reply, envelope, { id, status: 'connected', account });
  }

  /** The OAuth handshake's first half: SDC never sees the password, only a state token. */
  private openOAuth(envelope: Envelope, params: Params, reply: Reply): void {
    const id = text(params, 'id');

    ok(reply, envelope, {
      url: `https://auth.example.com/${id}/authorize?state=demo`,
      state: `state-${id}`,
    });
  }

  /** …and the second half: 1.5s of waiting, then the token arrives. */
  private finishOAuth(envelope: Envelope, params: Params, push: Push, reply: Reply): void {
    const id = text(params, 'id');
    const seed = strings.seed.providers.find((provider) => provider.id === id);

    this.clock.after(DEMO_TIMINGS.oauth, () => {
      push({
        type: 'ProviderStatus',
        id,
        name: seed?.name ?? id,
        status: 'connected',
        account: 'subscription',
        detail: seed?.detail,
        kind: seed?.kind,
        logo: seed?.logo,
        initial: seed?.initial,
      });
      push({ type: 'Toast', message: strings.hub.connectToast(seed?.name ?? id) });
      ok(reply, envelope, { ok: true, account: 'subscription' });
    });
  }

  /** The Local flow's two doctor rows: is the daemon up, and which models does it have. */
  private localDoctor(envelope: Envelope, push: Push, reply: Reply): void {
    const models = [...strings.seed.ollamaModels];

    push({ type: 'Toast', message: strings.hub.connectToast('Ollama') });
    ok(reply, envelope, { daemon: true, endpoint: 'http://localhost:11434', models });
  }

  /**
   * The engine stream of spec section 5.6: a turn is `TurnStarted`, thinking, tool calls, `TurnDelta`
   * per chunk, a `CheckpointSaved` before any mutating tool (principle P5) and `TurnCompleted`.
   */
  private startEngine(envelope: Envelope, params: Params, push: Push, reply: Reply): void {
    const sessionId = text(params, 'sessionId', 's1');
    const engine = text(params, 'engine', 'claude_code');
    const model = text(params, 'model', 'sonnet');
    const turnId = `turn-${this.nextTurn++}`;
    const live: LiveTurn = { turnId, sessionId, engine, model, timers: [] };

    this.turns.set(turnId, live);

    push(
      {
        type: 'TurnStarted',
        turnId,
        sessionId,
        engine,
        model,
        tier: 'Balanced',
        forecast: strings.turns.metaForecast,
      },
      { sessionId, turnId },
    );
    push({ type: 'ThinkingDelta', turnId, delta: strings.turns.thinking.body }, { sessionId, turnId });

    const words = strings.turns.answer.split(' ');
    let elapsed = DEMO_TIMINGS.tool;

    live.timers.push(
      this.clock.after(elapsed, () => {
        push(
          { type: 'ToolCallStarted', turnId, callId: `${turnId}-read`, tool: 'read', name: strings.turns.tools.read, target: 'src/auth.ts' },
          { sessionId, turnId },
        );
      }),
    );

    elapsed += DEMO_TIMINGS.tool;
    live.timers.push(
      this.clock.after(elapsed, () => {
        push({ type: 'ToolCallCompleted', turnId, callId: `${turnId}-read`, status: 'done', meta: strings.turns.tools.readStatus }, { sessionId, turnId });
        push(
          { type: 'ToolCallStarted', turnId, callId: `${turnId}-edit`, tool: 'edit', name: strings.turns.tools.edit, target: 'src/auth.ts' },
          { sessionId, turnId },
        );
      }),
    );

    elapsed += DEMO_TIMINGS.tool;
    live.timers.push(
      this.clock.after(elapsed, () => {
        /* A mutating tool always writes a checkpoint first (master spec principle P5). */
        const turn = this.checkpoints.length + 13;
        const checkpointId = `cp-${turn + 1}`;
        const title = strings.rightPanel.timeMachine.entries[0].title;

        this.checkpoints.unshift({ id: checkpointId, sessionId, turn, title });
        push(
          {
            type: 'CheckpointSaved',
            sessionId,
            checkpoint: {
              id: checkpointId,
              turn,
              ts: 'now',
              title,
              thumbnail: null,
              filesHash: `hash-${checkpointId}`,
              rewindRef: null,
            },
          },
          { sessionId, turnId },
        );
        push({ type: 'ToolCallCompleted', turnId, callId: `${turnId}-edit`, status: 'done', meta: strings.turns.tools.editStatus }, { sessionId, turnId });
      }),
    );

    live.timers.push(
      ...this.clock.every(DEMO_TIMINGS.delta, words.length, (index) => {
        push(
          {
            type: 'TurnDelta',
            turnId,
            delta: `${words[index]}${index === words.length - 1 ? '' : ' '}`,
          },
          { sessionId, turnId },
        );

        if (index === words.length - 1) {
          this.turns.delete(turnId);
          push(
            {
              type: 'TurnCompleted',
              turnId,
              summary: strings.turns.footer.summary,
              meta: strings.turns.footer.detail,
              pass: true,
            },
            { sessionId, turnId },
          );
          push({ type: 'SessionUpdated', sessionId, state: 'success', minutesAgo: 0, unread: 0 }, { sessionId });
        }
      }),
    );

    ok(reply, envelope, { turnId });
  }

  /** `Esc` and `Ctrl+Shift+Esc`: the stream stops and the turn reports what happened. */
  private stopEngine(envelope: Envelope, params: Params, push: Push, reply: Reply): void {
    const turnId = text(params, 'turnId');
    const live = this.turns.get(turnId);

    if (live) {
      this.clock.cancel(live.timers);
      this.turns.delete(turnId);
      push(
        {
          type: 'TurnCompleted',
          turnId,
          summary:
            envelope.method === 'engine.kill' ? strings.prompt.killed : strings.prompt.interrupt,
          meta: '',
          pass: false,
        },
        { sessionId: live.sessionId, turnId },
      );
      push(
        { type: 'SessionUpdated', sessionId: live.sessionId, state: 'idle', minutesAgo: 0 },
        { sessionId: live.sessionId },
      );
    }

    ok(reply, envelope, {});
  }

  /** Spec section 16.5: a mid-turn engine switch that replays the context into the new engine. */
  private switchEngine(envelope: Envelope, params: Params, push: Push, reply: Reply): void {
    const turnId = text(params, 'turnId');
    const live = this.turns.get(turnId);
    const to = text(params, 'engine', 'codex');
    const model = text(params, 'model', 'default');
    const from = live?.engine ?? 'claude_code';
    const sessionId = live?.sessionId ?? 's1';

    if (live) {
      this.clock.cancel(live.timers);
      live.engine = to;
      live.model = model;
    }

    push(
      { type: 'SessionBridged', sessionId, turnId, from, to, model, reason: text(params, 'reason') },
      { sessionId, turnId },
    );
    push({ type: 'Toast', message: strings.prompt.bridged(from, to) });
    ok(reply, envelope, { turnId, bridgedFrom: from });
  }

  /** Spec section 14: restore the files and the conversation as they were at a turn. */
  private rewind(envelope: Envelope, params: Params, push: Push, reply: Reply): void {
    const sessionId = text(params, 'sessionId', 's1');
    const turnId = text(params, 'turnId');
    const turn = Number(turnId.replace(/[^0-9]/g, '')) || this.checkpoints[0]?.turn || 14;
    const dropped = this.checkpoints.filter(
      (checkpoint) => checkpoint.sessionId === sessionId && checkpoint.turn > turn,
    ).length;
    const kept = this.checkpoints.filter((checkpoint) => checkpoint.turn <= turn);

    this.checkpoints.splice(0, this.checkpoints.length, ...kept);

    push(
      { type: 'RewindApplied', sessionId, direction: 'back', turn, turns: dropped, files: dropped },
      { sessionId },
    );
    push({
      type: 'Toast',
      message: strings.rightPanel.timeMachine.rewound(turn),
      action: strings.rightPanel.timeMachine.undo,
      /* Ten seconds, not three: undoing a rewind is a decision, not a notification. */
      holdMs: 10000,
    });
    ok(reply, envelope, { removedTurns: dropped, restoredFiles: dropped });
  }

  /** …and the other direction: pop the stack and re-apply what was dropped. */
  private redo(envelope: Envelope, params: Params, push: Push, reply: Reply): void {
    const sessionId = text(params, 'sessionId', 's1');
    const turn = this.checkpoints.length + 13;
    const title = strings.rightPanel.timeMachine.entries[0].title;

    this.checkpoints.unshift({ id: `cp-${turn}`, sessionId, turn, title });
    push(
      { type: 'RewindApplied', sessionId, direction: 'forward', turn, turns: 1, files: 1 },
      { sessionId },
    );
    push({ type: 'Toast', message: strings.rightPanel.timeMachine.rewound(turn) });
    ok(reply, envelope, { turn });
  }

  /** The turn a permission request belongs to when the caller does not say. */
  private lastTurnId(): string {
    return [...this.turns.keys()].at(-1) ?? 'turn-7';
  }
}

