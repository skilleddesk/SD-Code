// Real processes for the end-to-end check: the relay (wrangler dev, i.e. the real Worker and Durable Object
// on workerd), the real sdcd binary, and a browser that uses the real code in web/src. Nothing is mocked
// between them; the only stand-in is the passkey (a software authenticator signing real WebAuthn bytes).

import { spawn, type ChildProcess } from 'node:child_process';
import { mkdtempSync, rmSync } from 'node:fs';
import net from 'node:net';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import WebSocket from 'ws';
import type { SocketLike } from '../src/transport/link';

const root = resolve(import.meta.dirname, '..', '..');

export async function freePort(): Promise<number> {
  return new Promise((done, fail) => {
    const server = net.createServer();

    server.listen(0, '127.0.0.1', () => {
      const { port } = server.address() as net.AddressInfo;

      server.close(() => done(port));
    });
    server.on('error', fail);
  });
}

export const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

export async function waitFor<T>(what: string, probe: () => Promise<T | false | undefined | null> | T | false | undefined | null, ms = 20_000): Promise<T> {
  const deadline = Date.now() + ms;
  let last: unknown;

  while (Date.now() < deadline) {
    try {
      const value = await probe();

      if (value) return value as T;
    } catch (error) {
      last = error;
    }

    await sleep(100);
  }

  throw new Error(`timed out waiting for ${what}${last ? `: ${String(last)}` : ''}`);
}

export interface Relay {
  /** What the relay process has printed so far (a dropped socket usually explains itself here). */
  log(): string;
  port: number;
  httpUrl: string;
  wsUrl: string;
  stop(): Promise<void>;
}

export async function startRelay(extra: Record<string, string> = {}): Promise<Relay> {
  return startRelayOn(await freePort(), '127.0.0.1', extra);
}

/** A stand-in mail provider on this machine: records what the relay posts to it, so a test can read the link in the email. */
export interface CaughtMail {
  headers: Record<string, string | string[] | undefined>;
  body: any;
  /** When it arrived, in ms on this process's monotonic clock (for latency figures). */
  at: number;
}

export async function startMailCatcher(): Promise<{ url: string; mails: CaughtMail[]; stop(): Promise<void> }> {
  const { createServer } = await import('node:http');
  const mails: CaughtMail[] = [];
  const server = createServer((request, response) => {
    let body = '';

    request.on('data', (chunk) => (body += chunk));
    request.on('end', () => {
      try {
        mails.push({ headers: request.headers, body: JSON.parse(body), at: Number(process.hrtime.bigint()) / 1e6 });
      } catch {
        // Not JSON: not a mail.
      }

      response.writeHead(200).end('{}');
    });
  });

  await new Promise<void>((done) => server.listen(0, '127.0.0.1', () => done()));

  const { port } = server.address() as net.AddressInfo;

  return { url: `http://127.0.0.1:${port}/send`, mails, stop: () => new Promise<void>((done) => server.close(() => done())) };
}

/** Applies the D1 migrations to the relay's local database, the way `wrangler d1 migrations apply` does for the real one. */
function migrate(state: string): Promise<void> {
  const wrangler = join(root, 'cloud', 'node_modules', 'wrangler', 'bin', 'wrangler.js');

  return new Promise((done, fail) => {
    const child = spawn(process.execPath, [wrangler, 'd1', 'migrations', 'apply', 'DB', '--local', '--persist-to', state], { cwd: join(root, 'cloud'), stdio: ['ignore', 'pipe', 'pipe'], windowsHide: true });
    let log = '';

    child.stdout?.on('data', (chunk) => (log += chunk));
    child.stderr?.on('data', (chunk) => (log += chunk));
    child.on('exit', (code) => (code === 0 ? done() : fail(new Error(`the D1 migrations did not apply:\n${log}`))));
  });
}

/** `extra` are more Worker variables (the mail provider, for the sign-in link test); they are local to this process. */
export async function startRelayOn(port: number, host = '127.0.0.1', extra: Record<string, string> = {}): Promise<Relay> {
  const state = mkdtempSync(join(tmpdir(), 'sdc-relay-'));

  await migrate(state);

  const child = spawn(
    process.execPath,
    [
      join(root, 'cloud', 'node_modules', 'wrangler', 'bin', 'wrangler.js'),
      'dev', '--port', String(port), '--ip', '127.0.0.1', '--local', '--persist-to', state,
      '--var', `WEB_ORIGIN:http://${host}:${port}`, '--var', `RP_ID:${host}`,
      ...Object.entries(extra).flatMap(([name, value]) => ['--var', `${name}:${value}`]),
    ],
    /* `detached` on Linux and macOS makes the relay the leader of its own process group, so `killTree` can end
       wrangler *and* the workerd it starts - killing only wrangler left workerd serving the port. */
    { cwd: join(root, 'cloud'), stdio: ['ignore', 'pipe', 'pipe'], windowsHide: true, detached: process.platform !== 'win32' },
  );
  let log = '';

  child.stdout?.on('data', (chunk) => (log += chunk));
  child.stderr?.on('data', (chunk) => (log += chunk));

  await waitFor('the relay to be ready', () => log.includes('Ready on'), 60_000).catch((error) => {
    child.kill();
    throw new Error(`${error.message}\n${log}`);
  });

  return {
    log: () => log,
    port,
    httpUrl: `http://${host}:${port}`,
    wsUrl: `ws://${host}:${port}`,
    async stop() {
      await killTree(child);
      tryRemove(state);
    },
  };
}

async function killTree(child: ChildProcess): Promise<void> {
  if (child.exitCode !== null || !child.pid) return;

  if (process.platform === 'win32') {
    await new Promise<void>((done) => spawn('taskkill', ['/pid', String(child.pid), '/t', '/f'], { stdio: 'ignore' }).on('exit', () => done()));
  } else {
    /* The whole group (see the spawn): `child.kill` reached wrangler only, and its workerd kept the relay up -
       the reason "the relay goes away" and the page-pairing suites failed on Linux CI and passed on Windows. */
    try {
      process.kill(-child.pid, 'SIGKILL');
    } catch {
      child.kill('SIGKILL');
    }

    await new Promise<void>((done) => (child.exitCode !== null ? done() : child.once('exit', () => done())));
  }
}

export interface Daemon {
  /** What the daemon has printed (its errors and the reason it stopped, if it did). */
  log(): string;
  pid: number;
  port: number;
  dataDir: string;
  call(method: string, params?: Record<string, unknown>): Promise<any>;
  stop(): Promise<void>;
}

/** A line-oriented SDCP client over TCP. */
class Sdcp {
  private socket!: net.Socket;
  private buffer = '';
  private nextId = 1;
  private pending = new Map<string, { resolve(value: any): void; reject(error: Error): void }>();

  async connect(port: number): Promise<void> {
    this.socket = net.connect(port, '127.0.0.1');
    this.socket.setEncoding('utf8');
    this.socket.on('data', (chunk: string) => {
      this.buffer += chunk;

      for (let newline = this.buffer.indexOf('\n'); newline >= 0; newline = this.buffer.indexOf('\n')) {
        const line = this.buffer.slice(0, newline);

        this.buffer = this.buffer.slice(newline + 1);

        try {
          const message = JSON.parse(line);
          const waiting = this.pending.get(message.id);

          if (waiting && message.v) {
            this.pending.delete(message.id);
            message.error ? waiting.reject(Object.assign(new Error(`${message.error.code}: ${message.error.message}`), { code: message.error.code })) : waiting.resolve(message.result);
          }
        } catch {
          // A notification line or noise.
        }
      }
    });
    await new Promise<void>((done, fail) => {
      this.socket.once('connect', () => done());
      this.socket.once('error', fail);
    });
  }

  call(method: string, params: Record<string, unknown> = {}): Promise<any> {
    const id = `e2e-${this.nextId++}`;

    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.socket.write(`${JSON.stringify({ v: '0.1', id, method, params })}\n`);
    });
  }

  close(): void {
    this.socket.destroy();
  }
}

export async function startDaemon(): Promise<Daemon> {
  const port = await freePort();
  const dataDir = mkdtempSync(join(tmpdir(), 'sdc-data-'));
  const binary = join(root, 'sdcd', 'target', 'debug', process.platform === 'win32' ? 'sdcd.exe' : 'sdcd');
  const child = spawn(binary, ['--port', String(port), '--database', join(dataDir, 'sdc.db')], {
    env: { ...process.env, SDC_DATA_DIR: dataDir, XDG_RUNTIME_DIR: dataDir },
    stdio: ['ignore', 'pipe', 'pipe'],
    windowsHide: true,
  });
  let log = '';

  child.stdout?.on('data', (chunk) => (log += chunk));
  child.stderr?.on('data', (chunk) => (log += chunk));

  const client = new Sdcp();

  await waitFor('sdcd to accept connections', async () => {
    try {
      await client.connect(port);
      return true;
    } catch {
      return false;
    }
  }).catch((error) => {
    child.kill();
    throw new Error(`${error.message}\n${log}`);
  });

  return {
    log: () => log,
    pid: child.pid ?? 0,
    port,
    dataDir,
    call: (method, params) => client.call(method, params),
    async stop() {
      client.close();
      await killTree(child);
      tryRemove(dataDir);
    },
  };
}

/** A browser WebSocket that sends the Origin a real page would (the relay refuses other origins). */
export type TestSocket = SocketLike & { terminate(): void };

export function socketFor(origin: string): (url: string) => TestSocket {
  return (url) => {
    const socket = new WebSocket(url, { origin });
    const like: TestSocket = {
      terminate: () => socket.terminate(),
      send: (data) => socket.send(data),
      close: (code, reason) => socket.close(code, reason),
      onopen: null,
      onmessage: null,
      onclose: null,
      onerror: null,
    };

    socket.on('open', () => like.onopen?.({}));
    socket.on('message', (data, isBinary) => like.onmessage?.({ data: isBinary ? data : data.toString() }));
    socket.on('close', (code, reason) => like.onclose?.({ code, reason: reason.toString() }));
    socket.on('error', (error) => like.onerror?.(error));

    return like;
  };
}

/** Windows keeps files open a moment after a process dies; a temp directory left behind is not a failure. */
function tryRemove(path: string): void {
  try {
    rmSync(path, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
  } catch {
    // The OS temp cleaner will get it.
  }
}
