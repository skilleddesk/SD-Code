// Test doubles: a daemon and browsers that speak to the Hub the way sdcd and the PWA do.

import { env, exports } from 'cloudflare:workers';
import init from '../migrations/0001_init.sql?raw';
import lookup from '../migrations/0002_lookup_indexes.sql?raw';
import { b64uEncode, daemonIdOf, deviceAuthMessage, hubAuthMessage } from '../src/util';

export const ORIGIN = 'https://sdc.skilleddesk.com';

/** Applies the D1 migrations to the test database, the way `wrangler d1 migrations apply` does for the real one. */
export async function migrate(): Promise<void> {
  for (const sql of [init, lookup]) {
    const statements = sql
      .replace(/--.*$/gm, '')
      .split(';')
      .map((statement) => statement.trim())
      .filter(Boolean);

    await env.DB.batch(statements.map((statement) => env.DB.prepare(statement)));
  }
}

export interface Outgoing {
  url: string;
  headers: Record<string, string>;
  body: Uint8Array | string;
}

/**
 * Stands in for the network: everything the relay sends out (to a push service, to the mail provider) is recorded and answered
 * with `answer` instead of leaving the machine. Returns the record and a function that puts the real `fetch` back.
 */
export function captureFetch(answer: (url: string) => number = () => 201): { sent: Outgoing[]; restore(): void } {
  const real = globalThis.fetch;
  const sent: Outgoing[] = [];

  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
    const headers: Record<string, string> = {};

    new Headers(init?.headers).forEach((value, name) => (headers[name] = value));
    sent.push({ url, headers, body: init?.body instanceof Uint8Array ? init.body : String(init?.body ?? '') });

    return new Response(null, { status: answer(url) });
  }) as typeof fetch;

  return { sent, restore: () => void (globalThis.fetch = real) };
}

/** Waits until `check` returns something, or throws. For things that happen after the reply (an alarm, a deferred send). */
export async function eventually<T>(what: string, check: () => T | undefined | false | Promise<T | undefined | false>, ms = 4000): Promise<T> {
  const deadline = Date.now() + ms;

  for (;;) {
    const value = await check();

    if (value) return value;
    if (Date.now() > deadline) throw new Error(`timed out waiting for ${what}`);
    await new Promise((resolve) => setTimeout(resolve, 40));
  }
}

export interface KeyPair {
  privateKey: CryptoKey;
  /** SEC1 uncompressed public key, 65 bytes. */
  publicBytes: Uint8Array;
}

export async function newKeyPair(): Promise<KeyPair> {
  const pair = (await crypto.subtle.generateKey({ name: 'ECDSA', namedCurve: 'P-256' }, true, ['sign', 'verify'])) as CryptoKeyPair;
  const publicBytes = new Uint8Array((await crypto.subtle.exportKey('raw', pair.publicKey)) as ArrayBuffer);

  return { privateKey: pair.privateKey, publicBytes };
}

export async function sign(key: KeyPair, message: Uint8Array): Promise<string> {
  return b64uEncode(new Uint8Array(await crypto.subtle.sign({ name: 'ECDSA', hash: 'SHA-256' }, key.privateKey, message)));
}

/** A WebSocket with a message queue and close tracking, so a test can `await next()`. */
export class Socket {
  readonly queue: any[] = [];
  closed: { code: number; reason: string } | null = null;
  private waiting: Array<() => void> = [];

  constructor(readonly ws: WebSocket) {
    ws.accept();
    ws.addEventListener('message', (event) => {
      this.queue.push(JSON.parse(event.data as string));
      this.waiting.splice(0).forEach((wake) => wake());
    });
    ws.addEventListener('close', (event) => {
      this.closed = { code: event.code, reason: event.reason };
      this.waiting.splice(0).forEach((wake) => wake());
    });
  }

  /** The next message, or throws after a second. */
  async next(): Promise<any> {
    const deadline = Date.now() + 1500;

    while (this.queue.length === 0) {
      if (this.closed) throw new Error(`socket closed (${this.closed.code} ${this.closed.reason}) while waiting for a message`);
      if (Date.now() > deadline) throw new Error('no message arrived');
      await new Promise<void>((resolve) => {
        this.waiting.push(resolve);
        setTimeout(resolve, 50);
      });
    }

    return this.queue.shift();
  }

  /** The next message that satisfies `wanted`, skipping the others (a daemon socket also carries `open` and `close` notices). */
  async until(wanted: (message: any) => boolean): Promise<any> {
    for (;;) {
      const message = await this.next();

      if (wanted(message)) return message;
    }
  }

  /** Waits until the socket is closed and returns the close code. */
  async closedWith(): Promise<number> {
    const deadline = Date.now() + 1500;

    while (!this.closed) {
      if (Date.now() > deadline) throw new Error('socket did not close');
      await new Promise<void>((resolve) => {
        this.waiting.push(resolve);
        setTimeout(resolve, 50);
      });
    }

    return this.closed.code;
  }

  /** Whether nothing arrives for a short while (a negative assertion). */
  async quiet(ms = 150): Promise<boolean> {
    await new Promise((resolve) => setTimeout(resolve, ms));
    return this.queue.length === 0;
  }

  send(value: unknown): void {
    this.ws.send(typeof value === 'string' ? value : JSON.stringify(value));
  }

  close(): void {
    this.ws.close(1000, 'test over');
  }
}

export async function open(path: string, origin: string | null = null, extra: Record<string, string> = {}): Promise<{ response: Response; socket?: Socket }> {
  const headers: Record<string, string> = { Upgrade: 'websocket', ...extra };

  if (origin) headers.Origin = origin;

  const response = await exports.default.fetch(`https://sdc.skilleddesk.com${path}`, { headers });

  return response.webSocket ? { response, socket: new Socket(response.webSocket) } : { response };
}

export class FakeDaemon {
  id = '';
  identity!: KeyPair;
  socket!: Socket;

  static async connect(identity?: KeyPair): Promise<FakeDaemon> {
    const daemon = new FakeDaemon();

    daemon.identity = identity ?? (await newKeyPair());
    daemon.id = await daemonIdOf(daemon.identity.publicBytes);

    const { socket } = await open(`/d/${daemon.id}`);

    daemon.socket = socket!;

    const challenge = await daemon.socket.next();

    daemon.socket.send({
      t: 'auth',
      pub: b64uEncode(daemon.identity.publicBytes),
      sig: await sign(daemon.identity, hubAuthMessage(challenge.nonce, daemon.id)),
    });

    const answer = await daemon.socket.next();

    if (answer.t !== 'ready') throw new Error(`the Hub did not accept the daemon: ${JSON.stringify(answer)}`);

    return daemon;
  }

  async teach(devices: Array<{ id: string; key: KeyPair; guest?: boolean }>): Promise<void> {
    this.socket.send({ ctl: 'devices.sync', devices: devices.map((d) => ({ id: d.id, pub: b64uEncode(d.key.publicBytes), guest: !!d.guest })) });
    // devices.sync has no reply; a ping/pong round trip proves it was processed (messages are handled in order).
    this.socket.send({ ctl: 'ping' });
    expectPong(await this.socket.next());
  }
}

export function expectPong(message: any): void {
  if (message.t !== 'pong') throw new Error(`expected pong, got ${JSON.stringify(message)}`);
}

export class FakeBrowser {
  constructor(readonly daemonId: string, readonly deviceId: string, readonly key: KeyPair) {}

  /** Connects and authenticates. Returns the socket and the `open` the daemon was told. */
  async connect(daemon: FakeDaemon, options: { origin?: string | null } = {}): Promise<Socket> {
    const { socket, response } = await open(`/c/${this.daemonId}?device=${this.deviceId}`, options.origin === undefined ? ORIGIN : options.origin);

    if (!socket) throw new Error(`upgrade refused: ${response.status}`);

    const challenge = await socket.next();

    socket.send({ t: 'auth', sig: await sign(this.key, deviceAuthMessage(challenge.nonce, this.daemonId, this.deviceId)) });

    const answer = await socket.next();

    if (answer.t !== 'ready') throw new Error(`the Hub did not accept the device: ${JSON.stringify(answer)}`);

    void daemon;

    return socket;
  }
}
