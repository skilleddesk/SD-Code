// The app's state, kept outside React so it can be tested and so a link's callbacks have somewhere to write.
// React reads it with `useSyncExternalStore` (see `ui/hooks.ts`).

import type { Card } from '../crypto/approval';
import { IndexedDbStore, MemoryStore, type PairedRecord, type Store } from '../crypto/device';
import { BrowserPasskey, type Passkey } from '../crypto/passkey';
import { fingerprint } from '../crypto/session';
import { browserPushEnv, PushControl, type PushEnv, type PushState } from '../push/push';
import { Link, RequestFailed, type Answer, type LinkState } from '../transport/link';
import { recover, RecoveryFailed } from '../transport/recovery';
import { pair, parseOffer, type Offer, type Progress } from '../transport/pairing';
import { Chat, emptyChat, type ChatState } from './chat';
import { Workspace, emptyWorkspace, type WorkspaceState } from './workspace';

export interface LiveItem {
  id: number;
  kind: 'user' | 'text' | 'tool' | 'info' | 'error';
  text: string;
  sessionId: string | null;
  turnId: string | null;
}

export type Phase = 'loading' | 'unpaired' | 'signin' | 'pairing' | 'ready';

/** The page a sign-in link opens. The link is spent by the button on it, never by opening it. */
export interface SignIn {
  daemon: string;
  token: string;
  step: 'ready' | 'working' | 'invalid' | 'offline' | 'refused' | 'busy' | 'error';
}

/** "Email me a link", on the first screen. */
export type MailLink = 'idle' | 'sending' | 'sent' | 'invalid' | 'busy' | 'error';

export interface Snapshot {
  phase: Phase;
  link: LinkState;
  cards: Card[];
  pending: number;
  live: LiveItem[];
  progress: Progress | null;
  offer: Offer | null;
  error: string | null;
  device: { name: string; guest: boolean; fingerprint: string; recoverable: boolean } | null;
  /** True while the person is picking their passkey to rejoin. */
  recovering: boolean;
  notice: string | null;
  workspace: WorkspaceState;
  chat: ChatState;
  signin: SignIn | null;
  mailLink: MailLink;
  push: PushState;
  /** Counts up when a notification was tapped while the page was open: the page then shows the Inbox. */
  inboxTick: number;
}

const MAX_LIVE = 300;

export interface ModelOptions {
  hubUrl: string;
  store?: Store;
  passkey?: Passkey;
  userAgent?: string;
  /** For tests: the network and the browser's push features. */
  fetch?: typeof fetch;
  pushEnv?: PushEnv;
}

export class Model {
  private snap: Snapshot = {
    phase: 'loading',
    link: { kind: 'connecting' },
    cards: [],
    pending: 0,
    live: [],
    progress: null,
    offer: null,
    error: null,
    device: null,
    recovering: false,
    notice: null,
    workspace: emptyWorkspace(),
    chat: emptyChat(),
    signin: null,
    mailLink: 'idle',
    push: { kind: 'checking' },
    inboxTick: 0,
  };
  private listeners = new Set<() => void>();
  private pushSynced = false;
  private link: Link | null = null;
  private record: PairedRecord | null = null;
  private nextLive = 1;
  private readonly store: Store;
  private readonly passkey: Passkey;
  private lastLevel: string = "locked";
  readonly workspace = new Workspace(
    () => this.link,
    () => this.update({ workspace: this.workspace.state }),
  );
  readonly chat = new Chat(
    () => this.link,
    () => this.workspace,
    () => this.update({ chat: this.chat.state }),
  );

  readonly push: PushControl;

  constructor(private readonly options: ModelOptions) {
    this.store = options.store ?? new IndexedDbStore();
    this.passkey = options.passkey ?? new BrowserPasskey();
    this.push = new PushControl(
      options.pushEnv ?? browserPushEnv(),
      () => this.link,
      () => this.update({ push: this.push.state }),
    );
  }

  // --- reading ---------------------------------------------------------------------------------

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);

    return () => this.listeners.delete(listener);
  };

  getSnapshot = (): Snapshot => this.snap;

  private update(patch: Partial<Snapshot>): void {
    this.snap = { ...this.snap, ...patch };
    this.listeners.forEach((listener) => listener());
  }

  // --- life cycle ---------------------------------------------------------------------------------

  async init(fragment: string | null): Promise<void> {
    if (fragment) {
      try {
        this.update({ phase: 'pairing', offer: await parseOffer(fragment), error: null });
      } catch (error) {
        this.update({ phase: 'unpaired', error: (error as Error).message });
      }

      return;
    }

    this.record = await this.store.load().catch(() => null);

    if (!this.record) return this.update({ phase: 'unpaired' });

    await this.attach(this.record);
  }

  private async attach(record: PairedRecord): Promise<void> {
    this.record = record;
    this.update({
      phase: 'ready',
      device: { name: record.name, guest: record.guest, fingerprint: await fingerprint(record.daemon.identityPublic), recoverable: !!record.vault },
    });

    this.link = new Link({
      hubUrl: this.options.hubUrl,
      record,
      store: this.store,
      passkey: this.passkey,
      events: {
        state: (link) => {
          this.update({ link });
          this.onLevel(link.kind);
          this.syncPush(link.kind);
        },
        card: (card) => this.update({ cards: [...this.snap.cards.filter((c) => c.envelope.request_id !== card.envelope.request_id), card], pending: 0 }),
        pending: (pending) => this.update({ pending }),
        closed: (id, how, by) =>
          this.update({
            cards: this.snap.cards.filter((card) => card.envelope.request_id !== id),
            notice: how === 'resolved' && by ? `decided:${by}` : null,
          }),
        event: (_seq, event, meta) => this.addLive(event, meta),
      },
    });
    this.link.start();
  }

  /** Once per connection: learn where notifications stand and tell the relay the current address (a browser can change it). */
  private syncPush(kind: string): void {
    if (kind === 'locked' || kind === 'view' || kind === 'operate') {
      if (!this.pushSynced) {
        this.pushSynced = true;
        void this.push.check();
      }
    } else if (kind === 'offline' || kind === 'connecting' || kind === 'closed') {
      this.pushSynced = false;
    }
  }

  // --- sign in by email link -----------------------------------------------------------------------------

  private get net(): typeof fetch {
    return this.options.fetch ?? ((input, init) => fetch(input, init));
  }

  /** The first screen's "email me a link". The answer is the same whoever the address belongs to, so it says nothing about that. */
  async requestLink(email: string): Promise<void> {
    this.update({ mailLink: 'sending' });

    try {
      const response = await this.net('/api/magic/request', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ email: email.trim() }) });

      this.update({ mailLink: response.status === 202 ? 'sent' : response.status === 400 ? 'invalid' : response.status === 429 ? 'busy' : 'error' });
    } catch {
      this.update({ mailLink: 'error' });
    }
  }

  /** Opens the page a sign-in link points to. `hash` is `#<computer id>.<token>`, which no server ever sees. */
  initLink(hash: string): void {
    const [daemon, token] = hash.replace(/^#/, '').split('.');

    if (!daemon || !token || !/^[A-Za-z0-9_-]{22}$/.test(daemon) || !/^[A-Za-z0-9_-]{20,100}$/.test(token)) {
      return this.update({ phase: 'unpaired', error: 'This sign-in link is damaged. Ask for a new one.' });
    }

    // The secret leaves the address bar at once, so it is not in history, a screenshot of the URL, or a shared tab.
    history.replaceState(null, '', '/m');
    this.update({ phase: 'signin', signin: { daemon, token, step: 'ready' }, error: null });
  }

  /** The button. Spends the link at the relay, which asks the computer for a pairing offer; pairing then goes on as usual. */
  async redeemLink(): Promise<void> {
    const signin = this.snap.signin;

    if (!signin || signin.step === 'working') return;

    this.update({ signin: { ...signin, step: 'working' } });

    try {
      const response = await this.net('/api/magic/redeem', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ daemon: signin.daemon, token: signin.token }) });
      const body = (await response.json().catch(() => ({}))) as { daemon?: string; fragment?: string };

      if (!response.ok || !body.fragment) {
        const step = response.status === 400 ? 'invalid' : response.status === 503 ? 'offline' : response.status === 502 ? 'refused' : response.status === 429 ? 'busy' : 'error';

        return this.update({ signin: { ...signin, step } });
      }

      const offer = await parseOffer(body.fragment);

      // The offer must be for the computer the link was made for. (Its own id is the hash of its key, so this cannot be faked
      // into another computer; the six digits compared on the computer cover the rest.)
      if (offer.daemon.id !== signin.daemon) return this.update({ signin: { ...signin, step: 'refused' } });

      history.replaceState(null, '', '/');
      this.update({ phase: 'pairing', offer, signin: null, error: null });
    } catch {
      this.update({ signin: { ...signin, step: 'error' } });
    }
  }

  // --- rejoining with the passkey ------------------------------------------------------------------------

  /** The browser forgot everything (its data was cleared) but the phone still has the passkey: bring the pairing back with it. */
  async recoverWithPasskey(): Promise<void> {
    this.update({ recovering: true, error: null });

    try {
      const record = await recover({ hubUrl: this.options.hubUrl, passkey: this.passkey, store: this.store });

      this.update({ recovering: false });
      await this.attach(record);
    } catch (error) {
      // Closing the passkey sheet is not a failure worth a red message.
      const quiet = error instanceof RecoveryFailed && error.reason === 'cancelled';

      this.update({ recovering: false, error: quiet || (error as Error).name === 'NotAllowedError' ? null : (error as Error).message });
    }
  }

  // --- pairing ----------------------------------------------------------------------------------------

  async startPairing(deviceName: string, guest: boolean): Promise<void> {
    const offer = this.snap.offer;

    if (!offer) return;

    this.update({ error: null, progress: { step: 'connecting' } });

    try {
      const record = await pair({
        hubUrl: this.options.hubUrl,
        offer,
        deviceName,
        userAgent: this.options.userAgent ?? navigator.userAgent,
        guest,
        passkey: this.passkey,
        // A guest keeps nothing on this browser after the tab closes.
        store: guest ? new MemoryStore() : this.store,
        onProgress: (progress) => this.update({ progress }),
      });

      this.update({ offer: null, progress: null });
      history.replaceState(null, '', '/');
      await this.attach(record);
    } catch (error) {
      this.update({ error: (error as Error).message, progress: null });
    }
  }

  // --- acting -------------------------------------------------------------------------------------------

  private async run(action: () => Promise<unknown>): Promise<boolean> {
    try {
      await action();
      this.update({ error: null });

      return true;
    } catch (error) {
      const message = error instanceof RequestFailed ? error.message : (error as Error).message;

      this.update({ error: message });

      return false;
    }
  }

  unlock(): Promise<boolean> {
    return this.run(() => this.link!.unlock('view'));
  }

  unlockOperate(): Promise<boolean> {
    return this.run(() => this.link!.unlock('operate'));
  }

  lock(): Promise<boolean> {
    return this.run(() => this.link!.lock());
  }

  async decide(card: Card, decision: 'allow_once' | 'deny', reason?: string): Promise<boolean> {
    return this.answer(card, decision === 'deny' ? { kind: 'deny', reason } : { kind: 'allow_once' });
  }

  async answer(card: Card, answer: Answer): Promise<boolean> {
    const done = await this.run(() => this.link!.answer(card, answer));

    // The computer tells the *other* sessions that a request closed; this one learns it from the reply.
    if (done) this.update({ cards: this.snap.cards.filter((c) => c.envelope.request_id !== card.envelope.request_id) });

    return done;
  }

  async kill(): Promise<boolean> {
    return this.run(async () => {
      await this.link!.kill();
      this.update({ notice: 'killed' });
    });
  }

  async forget(): Promise<void> {
    this.link?.stop();
    this.link = null;
    this.workspace.reset();
    this.chat.reset();
    this.lastLevel = "locked";
    await this.store.clear();
    this.record = null;
    this.update({ phase: 'unpaired', cards: [], pending: 0, live: [], device: null, link: { kind: 'closed', why: 'forgotten' } });
  }

  showInbox(): void {
    this.update({ inboxTick: this.snap.inboxTick + 1 });
  }

  clearNotice(): void {
    this.update({ notice: null });
  }

  /** The first time the session is unlocked (and again after each new session) the page asks what there is to show. */
  private onLevel(kind: string): void {
    const open = kind === "view" || kind === "operate";
    const was = this.lastLevel === "view" || this.lastLevel === "operate";

    this.lastLevel = kind;

    if (open && !was) {
      void this.workspace.loadHosts();
      void this.chat.loadOptions();
    }
  }

  // --- the live view ---------------------------------------------------------------------------------------

  private addLive(event: Record<string, unknown>, meta: { sessionId: string | null; turnId: string | null }): void {
    const live = [...this.snap.live];
    const text = (key: string): string => (typeof event[key] === 'string' ? (event[key] as string) : '');
    const type = text('type');
    const turnId = text('turnId') || meta.turnId;
    const item = (kind: LiveItem['kind'], value: string): LiveItem => ({ id: this.nextLive++, kind, text: value, sessionId: meta.sessionId, turnId });

    if (type === 'TurnDelta') {
      const last = live[live.length - 1];

      if (last && last.kind === 'text' && last.turnId === turnId) live[live.length - 1] = { ...last, text: (last.text + text('delta')).slice(-6000) };
      else live.push(item('text', text('delta')));
    } else if (type === 'TurnStarted') {
      if (text('prompt')) live.push(item('user', text('prompt')));
      live.push(item('info', `▶ ${text('engine')} ${text('model')}`.trim()));
    } else if (type === 'ToolCallStarted') {
      live.push(item('tool', `${text('name')} ${text('target')}`.trim()));
    } else if (type === 'TurnCompleted') {
      live.push(item('info', `■ ${text('summary')}`));
    } else if (type === 'ErrorRaised') {
      live.push(item('error', `${text('title')} ${text('explanation')}`.trim()));
    } else if (type === 'KillSwitch') {
      live.push(item('error', '■ stopped'));
    } else {
      return;
    }

    this.update({ live: live.slice(-MAX_LIVE) });
  }
}
