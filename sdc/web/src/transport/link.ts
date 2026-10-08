// One live connection from this browser to the user's computer, through the relay.
//
// Layers, outermost first: WebSocket to the relay (it authenticates this device with a signature on its
// challenge) → the end-to-end session (`crypto/session.ts`) → channel messages `{ch, type, id, body}`.
// The relay carries the first two as opaque JSON; only the daemon can read the third.

import { b64u, fromB64u, utf8 } from '../crypto/bytes';
import type { PairedRecord, Store } from '../crypto/device';
import { decisionMessage, signWith, startHandshake, type Established } from '../crypto/session';
import type { Passkey } from '../crypto/passkey';
import { hashBytes, hashMatches, needsFreshPasskey, type Card } from '../crypto/approval';
import { pieces, Reassembler } from '../crypto/frame';

export type LinkState =
  | { kind: 'connecting' }
  | { kind: 'offline'; why: string }
  | { kind: 'locked' }
  | { kind: 'view' }
  | { kind: 'operate'; until: number }
  | { kind: 'closed'; why: string };

export type Level = 'locked' | 'view' | 'operate';

/** What a person can answer a card with. */
export type Answer =
  | { kind: 'allow_once' }
  /** Allow this and the same kind of action in this folder, on this computer, for a while. */
  | { kind: 'allow_scoped'; minutes: number }
  | { kind: 'deny'; reason?: string }
  /** Refuse and tell the AI to stop and wait. */
  | { kind: 'deny_pause' }
  /** Refuse the AI's command and give it this one to run instead; the new command is judged afresh. */
  | { kind: 'edit'; command: string };

/** The exact string that is signed and sent. Everything the person typed is inside it. */
export function wireOf(answer: Answer): string {
  switch (answer.kind) {
    case 'allow_once':
      return 'allow_once';
    case 'allow_scoped':
      return `allow_scoped:${Math.max(1, Math.min(60, Math.round(answer.minutes)))}`;
    case 'deny':
      return answer.reason?.trim() ? `deny:${answer.reason.trim().slice(0, 500)}` : 'deny';
    case 'deny_pause':
      return 'deny_pause';
    case 'edit':
      return `edit:${answer.command.trim()}`;
  }
}

const allows = (answer: Answer): boolean => answer.kind === 'allow_once' || answer.kind === 'allow_scoped';

export interface LinkEvents {
  state(state: LinkState): void;
  /** A card that needs an answer. */
  card(card: Card): void;
  /** A card was answered elsewhere or expired. */
  closed(requestId: string, how: 'resolved' | 'expired', by?: string, decision?: string): void;
  /** How many requests are waiting while this session is still locked (no content). */
  pending(count: number): void;
  event(seq: number, event: Record<string, unknown>, meta: { sessionId: string | null; turnId: string | null }): void;
}

export interface SocketLike {
  send(data: string): void;
  close(code?: number, reason?: string): void;
  onopen: ((ev: unknown) => void) | null;
  onmessage: ((ev: { data: unknown }) => void) | null;
  onclose: ((ev: { code: number; reason: string }) => void) | null;
  onerror: ((ev: unknown) => void) | null;
}

export interface LinkOptions {
  hubUrl: string;
  record: PairedRecord;
  store: Store;
  passkey: Passkey;
  events: Partial<LinkEvents>;
  /** How to open a socket; the browser's `WebSocket` by default. */
  openSocket?: (url: string) => SocketLike;
  clock?: () => number;
}

export class RequestFailed extends Error {
  constructor(
    readonly code: string,
    message: string,
    /** The rest of the daemon's error object (a protected file's `challenge`, for one). */
    readonly data: Record<string, any> = {},
  ) {
    super(message);
  }
}

const REQUEST_TIMEOUT_MS = 30_000;

interface Waiting {
  resolve(body: any): void;
  reject(error: Error): void;
  timer: ReturnType<typeof setTimeout>;
}

export class Link {
  private socket: SocketLike | null = null;
  private session: Established | null = null;
  private rx = new Reassembler();
  private nextMessage = 1;
  /** What has been opened since the last acknowledgement; the daemon holds bulk back until it hears one. */
  private unacked = { frames: 0, bytes: 0 };
  private ackTimer: ReturnType<typeof setTimeout> | null = null;
  private waiting = new Map<string, Waiting>();
  private nextId = 1;
  private stopped = false;
  private clockOffset = 0;
  private state: LinkState = { kind: 'connecting' };
  private cards = new Map<string, Card>();
  private queue: Promise<unknown> = Promise.resolve();
  private attempt = 0;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  /** How the last socket ended, for support and for tests. */
  lastClose: { code: number; reason: string } | null = null;
  lastError: string | null = null;
  /** Sealing is asynchronous, and a frame must reach the daemon in the order it was sealed: one at a time. */
  private sending: Promise<unknown> = Promise.resolve();
  /** Callers of `registerPush` waiting for the relay's answer. */
  private pushAnswers: Array<(answer: { t: string; why?: string }) => void> = [];

  constructor(private readonly options: LinkOptions) {}

  get current(): LinkState {
    return this.state;
  }

  get level(): Level {
    return this.state.kind === 'operate' ? 'operate' : this.state.kind === 'view' ? 'view' : 'locked';
  }

  private now(): number {
    return (this.options.clock ?? Date.now)() + this.clockOffset;
  }

  private set(state: LinkState): void {
    this.state = state;
    this.options.events.state?.(state);
  }

  /** Opens the connection and keeps it open, with backoff, until `stop()`. */
  start(): void {
    this.stopped = false;
    void this.connect();
  }

  stop(): void {
    this.stopped = true;

    if (this.reconnectTimer) clearTimeout(this.reconnectTimer);
    if (this.ackTimer) clearTimeout(this.ackTimer);

    this.ackTimer = null;
    this.socket?.close(1000, 'bye');
    this.socket = null;
    this.session = null;
    this.failAll(new Error('the connection was closed'));
    this.set({ kind: 'closed', why: 'stopped' });
  }

  private failAll(error: Error): void {
    for (const [, waiting] of this.waiting) {
      clearTimeout(waiting.timer);
      waiting.reject(error);
    }

    this.waiting.clear();
  }

  private scheduleReconnect(why: string): void {
    if (this.stopped) return;

    this.attempt += 1;

    // 0.5 s, 1 s, 2 s ... capped at 30 s, with jitter: the first retry is quick because most drops are a network blip.
    const delay = Math.min(30_000, 500 * 2 ** Math.min(this.attempt - 1, 6)) * (0.75 + Math.random() * 0.5);

    this.set({ kind: 'offline', why });
    this.reconnectTimer = setTimeout(() => void this.connect(), delay);
  }

  private async connect(): Promise<void> {
    const { record, hubUrl } = this.options;
    const open = this.options.openSocket ?? ((url: string) => new WebSocket(url) as unknown as SocketLike);
    const socket = open(`${hubUrl.replace(/\/$/, '')}/c/${record.daemon.id}?device=${encodeURIComponent(record.deviceId)}`);

    this.socket = socket;
    this.session = null;
    this.rx = new Reassembler();
    this.unacked = { frames: 0, bytes: 0 };
    this.set({ kind: 'connecting' });

    let phase: 'challenge' | 'ready' | 'welcomed' = 'challenge';
    let finish: ((welcome: any) => Promise<Established>) | null = null;

    socket.onmessage = (event) => {
      this.queue = this.queue.then(async () => {
        try {
          const message = JSON.parse(String(event.data));

          if (phase !== 'welcomed') {
            if (message.t === 'challenge') {
              const sig = await signWith(record.signKey, utf8(`sdc-anywhere/v1/hub-device-auth|${message.nonce}|${record.daemon.id}|${record.deviceId}`));

              socket.send(JSON.stringify({ t: 'auth', sig: b64u(sig) }));
            } else if (message.t === 'ready') {
              if (typeof message.now === 'number') this.clockOffset = message.now - Date.now();

              phase = 'ready';

              const handshake = await startHandshake({
                signKey: record.signKey,
                deviceId: record.deviceId,
                daemon: record.daemon,
                lastSeq: record.lastSeq,
                now: this.now(),
              });

              finish = (welcome) => handshake.finish(welcome);
              socket.send(JSON.stringify(handshake.hello));
            } else if (message.t === 'welcome' && finish) {
              this.session = await finish(message);
              phase = 'welcomed';
              this.attempt = 0;
              this.saveVault(socket);
              this.applyLevel(this.session.welcome.level);
              void this.afterWelcome();
            } else if (message.t === 'offline') {
              this.set({ kind: 'offline', why: 'your computer is offline' });
            } else if (message.t === 'denied') {
              this.set({ kind: 'closed', why: 'this device is not (or no longer) paired with that computer' });
              this.stopped = true;
            }

            return;
          }

          // The relay's own answer to a push subscription (the one thing it answers itself; everything else is end-to-end).
          if (message.t === 'push.ok' || message.t === 'push.refused') {
            this.pushAnswers.splice(0).forEach((answer) => answer(message));

            return;
          }

          if (message.t === 'f' && this.session) {
            const plain = await this.session.recv.open(message.n, fromB64u(message.ct));
            const whole = this.rx.push(plain);

            this.noteReceived(plain.length);

            if (whole) this.handle(JSON.parse(new TextDecoder().decode(whole)));
          }
        } catch (error) {
          // A frame that will not open means the session is no longer trustworthy: drop it and start over.
          this.lastError = `session error: ${(error as Error)?.message ?? error}`;
          socket.close(4000, 'session error');
          this.session = null;
        }
      });
    };

    socket.onclose = (event) => {
      if (this.socket !== socket) return;

      this.lastClose = { code: event.code, reason: event.reason };
      this.session = null;
      this.failAll(new Error('the connection dropped'));

      if (event.code === 4001) {
        this.set({ kind: 'closed', why: 'this device is not (or no longer) paired with that computer' });
        this.stopped = true;

        return;
      }

      this.scheduleReconnect(event.code === 4003 || event.code === 4002 ? 'your computer is offline' : 'reconnecting');
    };
    socket.onerror = (error) => {
      this.lastError = String((error as { message?: string })?.message ?? error);
    };
  }

  /**
   * Gives the relay this browser's push subscription, or takes it back (`null`). Not end-to-end: the relay needs the address to send
   * a push at all. It sees an address and two keys, never a message. Resolves when the relay answers.
   */
  registerPush(subscription: { endpoint: string; p256dh: string; auth: string } | null): Promise<void> {
    const socket = this.socket;

    if (!socket || this.state.kind === 'connecting' || this.state.kind === 'offline' || this.state.kind === 'closed') {
      return Promise.reject(new Error('not connected to your computer right now'));
    }

    return new Promise<void>((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error('the relay did not answer')), 10_000);

      this.pushAnswers.push((answer) => {
        clearTimeout(timer);
        answer.t === 'push.ok' ? resolve() : reject(new Error(String(answer.why ?? 'the relay refused the subscription')));
      });
      socket.send(JSON.stringify(subscription ? { t: 'push.subscribe', subscription } : { t: 'push.unsubscribe' }));
    });
  }

  /**
   * Keeps the sealed copy of this pairing at the relay (crypto/vault.ts), so the passkey can bring it back if this browser's data is
   * cleared. It is ciphertext the relay cannot open, sent on every connect, so a relay that lost it gets it again.
   */
  private saveVault(socket: SocketLike): void {
    const vault = this.options.record.vault;

    if (vault) socket.send(JSON.stringify({ t: 'vault.put', id: vault.id, blob: vault.blob }));
  }

  /** Tells the daemon how far we have read, often enough that bulk keeps flowing and rarely enough to cost nothing. */
  private noteReceived(bytes: number): void {
    this.unacked.frames += 1;
    this.unacked.bytes += bytes;

    if (this.unacked.frames >= 8 || this.unacked.bytes >= 128 * 1024) {
      this.flushAck();
    } else if (!this.ackTimer) {
      this.ackTimer = setTimeout(() => this.flushAck(), 30);
    }
  }

  private flushAck(): void {
    if (this.ackTimer) clearTimeout(this.ackTimer);

    this.ackTimer = null;
    this.unacked = { frames: 0, bytes: 0 };

    const session = this.session;

    if (session) void this.post('ack', { n: session.recv.received });
  }

  /** A message that expects no answer. */
  private async post(type: string, body: unknown): Promise<void> {
    const session = this.session;

    if (!session) return;

    const plain = utf8(JSON.stringify({ ch: 'control', type, body }));
    const sent = this.sending.then(async () => {
      for (const piece of pieces(plain, this.nextMessage++)) {
        const sealed = await session.send.seal(piece);

        this.socket?.send(JSON.stringify({ t: 'f', n: sealed.n, ct: b64u(sealed.ct) }));
      }
    });

    this.sending = sent.catch(() => undefined);
    await sent.catch(() => undefined);
  }

  private async afterWelcome(): Promise<void> {
    try {
      await this.request('stream.subscribe', { last_seq: this.options.record.lastSeq }, 'control');
    } catch {
      // Locked sessions cannot subscribe yet; the subscribe is repeated after an unlock.
    }
  }

  private applyLevel(level: string, operateUntil?: number | null): void {
    if (level === 'operate') this.set({ kind: 'operate', until: operateUntil ?? this.now() + 5 * 60_000 });
    else if (level === 'view') this.set({ kind: 'view' });
    else this.set({ kind: 'locked' });
  }

  private handle(frame: any): void {
    const { type, body } = frame;

    if (type === 'res') {
      const waiting = this.waiting.get(frame.id);

      if (!waiting) return;

      clearTimeout(waiting.timer);
      this.waiting.delete(frame.id);

      if (frame.ok) waiting.resolve(frame.body);
      else waiting.reject(new RequestFailed(frame.error?.code ?? 'error', frame.error?.message ?? 'failed', frame.error ?? {}));

      return;
    }

    switch (type) {
      case 'capability.state':
        this.applyLevel(body.level, body.operate_until);
        if (body.level !== 'locked') void this.afterWelcome();
        break;
      case 'approval.requested': {
        const card = body as Card;

        this.cards.set(card.envelope.request_id, card);
        this.options.events.card?.(card);
        break;
      }
      case 'approval.pending':
        this.options.events.pending?.(body.count);
        break;
      case 'approval.resolved':
        this.cards.delete(body.request_id);
        this.options.events.closed?.(body.request_id, 'resolved', body.by, body.decision);
        break;
      case 'approval.expired':
        this.cards.delete(body.request_id);
        this.options.events.closed?.(body.request_id, 'expired');
        break;
      case 'stream.event':
        this.options.record.lastSeq = Math.max(this.options.record.lastSeq, body.seq);
        void this.options.store.save(this.options.record);
        this.options.events.event?.(body.seq, body.event, { sessionId: body.session_id ?? null, turnId: body.turn_id ?? null });
        break;
    }
  }

  // --- requests ------------------------------------------------------------------------------------

  async request(type: string, body: unknown = {}, ch = 'control'): Promise<any> {
    const session = this.session;

    if (!session) throw new RequestFailed('offline', 'not connected');

    const id = String(this.nextId++);
    const answer = new Promise<any>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.waiting.delete(id);
        reject(new RequestFailed('timeout', 'the computer did not answer'));
      }, REQUEST_TIMEOUT_MS);

      this.waiting.set(id, { resolve, reject, timer });
    });
    const plain = utf8(JSON.stringify({ ch, type, id, body }));
    const sent = this.sending.then(async () => {
      // All the pieces of one message are sealed and sent together, in order.
      for (const piece of pieces(plain, this.nextMessage++)) {
        const sealed = await session.send.seal(piece);

        this.socket?.send(JSON.stringify({ t: 'f', n: sealed.n, ct: b64u(sealed.ct) }));
      }
    });

    this.sending = sent.catch(() => undefined);

    try {
      await sent;
    } catch (error) {
      const waiting = this.waiting.get(id);

      if (waiting) {
        clearTimeout(waiting.timer);
        this.waiting.delete(id);
      }

      throw error;
    }

    return answer;
  }

  /** Opens View or Operate with a passkey. */
  async unlock(level: 'view' | 'operate'): Promise<void> {
    const challenge = await this.request('capability.challenge', { level });
    const assertion = await this.options.passkey.assert(this.options.record.passkeyId, fromB64u(challenge.challenge));

    await this.request('capability.unlock', { assertion });
  }

  async lock(): Promise<void> {
    await this.request('capability.lock');
  }

  /** The kill switch. Needs no unlock. */
  async kill(): Promise<unknown> {
    return this.request('control.kill');
  }

  /** A call to the daemon: a tunnelled SDCP method or one of the file gateway's (`hosts.list`, `fs.list`, ...). */
  async rpc(method: string, params: unknown = {}): Promise<any> {
    return this.request('rpc', { method, params });
  }

  /**
   * Reads a file window. A protected file answers `needs_critical` with a challenge for exactly that file; the
   * passkey signs it and the read is repeated with the proof. The proof is spent by that one read.
   */
  async readFile(pathId: string, offset = 0, length?: number): Promise<any> {
    const params: Record<string, unknown> = { path_id: pathId, offset };

    if (length) params.length = length;

    try {
      return await this.rpc('fs.read', params);
    } catch (error) {
      if (!(error instanceof RequestFailed) || error.code !== 'needs_critical') throw error;

      const assertion = await this.options.passkey.assert(this.options.record.passkeyId, fromB64u(String(error.data.challenge)));

      return this.rpc('fs.read', { ...params, assertion });
    }
  }

  /**
   * Answers a card. What backs the answer is chosen from what the card is, not from what the user clicked:
   * a deny is signed by the device; an allow is signed by the device and, when the action is dangerous, by a
   * fresh passkey assertion over the action's own hash. Before signing, the browser recomputes the hash of the
   * card it was shown and refuses if it does not match.
   */
  async decide(card: Card, decision: 'allow_once' | 'deny', reason?: string): Promise<void> {
    return this.answer(card, decision === 'deny' ? { kind: 'deny', reason } : { kind: 'allow_once' });
  }

  async answer(card: Card, answer: Answer): Promise<void> {
    if (!(await hashMatches(card))) throw new RequestFailed('tampered', 'The card does not match its hash; it was not signed.');

    const hash = hashBytes(card);
    // The words of a refusal or an edit are part of the signed string, so the relay cannot change them: they go to
    // the AI as the person's instruction.
    const text = wireOf(answer);
    const decision = allows(answer) ? 'allow_once' : 'deny';
    const body: Record<string, unknown> = {
      request_id: card.envelope.request_id,
      decision: text,
      action_hash: card.action_hash,
      device_sig: b64u(await signWith(this.options.record.signKey, decisionMessage(hash, text))),
    };

    if (allows(answer)) {
      if (needsFreshPasskey(card.envelope)) {
        body.assertion = await this.options.passkey.assert(this.options.record.passkeyId, hash);
      } else if (this.level !== 'operate') {
        await this.unlock('operate');
      }
    }

    await this.request('approval.decision', body);
    this.cards.delete(card.envelope.request_id);
  }

  openCards(): Card[] {
    return [...this.cards.values()];
  }
}
