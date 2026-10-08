// Phase 1 benchmarks (docs/remote/PERF.md, rows 1, 4, 5, 15, 16 and the resource rows).
//
//   pnpm --filter @sdc/web bench:remote
//
// WHAT THIS MEASURES, AND WHAT IT DOES NOT. The relay, the daemon and the browser code all run on this one
// machine, so network latency is ~0 and these numbers are the **software overhead** of the design: sealing,
// routing through a Durable Object, opening, the daemon's checks. A real phone on 4G adds the radio's round trip
// twice (phone to relay, relay to PC). PERF.md says so next to every number; WAN rows need the deployed relay.

import { spawnSync } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { MemoryStore, type PairedRecord } from '../src/crypto/device';
import { SoftPasskey } from '../src/crypto/softpasskey';
import { Link } from '../src/transport/link';
import { pair, parseOffer } from '../src/transport/pairing';
import { startDaemon, startRelay, socketFor, sleep, waitFor, type Daemon, type Relay } from './harness';
import type { Card } from '../src/crypto/approval';

const SAMPLES = Number(process.env.BENCH_SAMPLES ?? 100);
const WARMUP = 10;

interface Stat {
  scenario: string;
  samples: number;
  p50: number;
  p95: number;
  max: number;
  unit: string;
}

const results: Stat[] = [];
const extra: Record<string, unknown> = {};

function stat(scenario: string, values: number[], unit = 'ms'): Stat {
  const sorted = [...values].sort((a, b) => a - b);
  const at = (q: number) => sorted[Math.min(sorted.length - 1, Math.ceil(q * sorted.length) - 1)] ?? 0;
  const entry = { scenario, samples: values.length, p50: round(at(0.5)), p95: round(at(0.95)), max: round(sorted[sorted.length - 1] ?? 0), unit };

  results.push(entry);

  return entry;
}

const round = (n: number) => Math.round(n * 100) / 100;
const now = () => Number(process.hrtime.bigint()) / 1e6;

let relay: Relay;
let daemon: Daemon;
let origin: string;
let record: PairedRecord;
let passkey: SoftPasskey;
let link: Link;
const cards = new Map<string, Card>();
/* When each card / state arrived, taken inside the callback: polling for it would round every sample up to the poll interval. */
const cardAt = new Map<string, number>();
const stateLog: Array<{ at: number; kind: string }> = [];
const events: Array<{ at: number; event: Record<string, unknown> }> = [];

function memoryOf(pid: number): number | null {
  if (process.platform !== 'win32') return null;

  const out = spawnSync('powershell', ['-NoProfile', '-Command', `(Get-Process -Id ${pid}).WorkingSet64`], { encoding: 'utf8' });
  const bytes = Number(out.stdout.trim());

  return Number.isFinite(bytes) && bytes > 0 ? bytes : null;
}

beforeAll(async () => {
  relay = await startRelay();
  origin = relay.httpUrl;
  daemon = await startDaemon();
  passkey = new SoftPasskey('127.0.0.1', origin);

  extra.daemonMemoryBeforeEnable = memoryOf(daemon.pid);

  await daemon.call('anywhere.configure', { relay: relay.wsUrl, acceptFileKey: true, notifyWhen: 'never' });
  await daemon.call('anywhere.enable');
  await waitFor('the daemon to connect to the relay', async () => (await daemon.call('anywhere.status')).connected);

  const begun = await daemon.call('anywhere.pair.begin', {});
  const offer = await parseOffer(new URL(begun.url).hash);
  const store = new MemoryStore();
  const paired = pair({
    hubUrl: relay.wsUrl, offer, deviceName: 'Bench phone', userAgent: 'bench', guest: false, passkey, store,
    openSocket: socketFor(origin), onProgress: () => undefined,
  });

  const request = await waitFor('the pairing request', async () => (await daemon.call('anywhere.pair.requests')).requests[0]);

  await daemon.call('anywhere.pair.confirm', { deviceId: request.deviceId, accept: true });
  record = await paired;
  link = new Link({
    hubUrl: relay.wsUrl, record, store, passkey, openSocket: socketFor(origin),
    events: { state: (state) => stateLog.push({ at: now(), kind: state.kind }), card: (card) => { cards.set(card.envelope.request_id, card); cardAt.set(card.envelope.request_id, now()); }, event: (_s, event) => events.push({ at: now(), event }) },
  });
  link.start();
  await waitFor('the link', () => link.current.kind === 'locked' || undefined);
  await link.unlock('operate');
  await waitFor('operate', () => link.current.kind === 'operate' || undefined);
  await link.request('stream.subscribe', { last_seq: 0 });
}, 240_000);

afterAll(async () => {
  link?.stop();
  await daemon?.stop();
  await relay?.stop();

  const dir = resolve(import.meta.dirname, '..', '..', 'bench', 'remote', 'results');

  mkdirSync(dir, { recursive: true });
  writeFileSync(
    resolve(dir, `phase1-${new Date().toISOString().slice(0, 10)}.json`),
    JSON.stringify({ when: new Date().toISOString(), node: process.version, platform: `${process.platform} ${process.arch}`, topology: 'relay, daemon and browser code on one machine (loopback)', results, ...extra }, null, 2) + '\n',
  );
});

describe('Phase 1 latency, same machine', () => {
  it('row 1: a request reaches an open page', async () => {
    const times: number[] = [];
    const callTimes: number[] = [];

    for (let i = 0; i < SAMPLES + WARMUP; i++) {
      const started = now();
      const { permissionId } = await daemon.call('permission.request', { sessionId: 's1', title: 'Bench', action: 'edit', target: `file-${i}`, risk: 'SAFE' });
      const answered = now();
      const id = `apr_${permissionId}`;

      await waitFor('the card', () => cards.get(id), 5000);

      if (i >= WARMUP) {
        times.push((cardAt.get(id) ?? now()) - started);
        callTimes.push(answered - started);
      }
      await daemon.call('permission.resolve', { permissionId, decision: 'deny' });
    }

    stat('1a the daemon’s own permission.request call (allocates an id: one SQLite commit), for reference', callTimes);

    expect(stat('1 request reaches an open page (from the start of the request)', times).p95).toBeLessThan(500);
  }, 120_000);

  it('row 4: tapping Allow until the daemon has the answer (round trip, an upper bound on one way)', async () => {
    const times: number[] = [];

    for (let i = 0; i < SAMPLES + WARMUP; i++) {
      const { permissionId } = await daemon.call('permission.request', { sessionId: 's1', title: 'Bench', action: 'run', target: `cmd-${i}`, risk: 'MUTATING' });
      const card = await waitFor('the card', () => cards.get(`apr_${permissionId}`), 5000);
      const started = now();

      await link.decide(card, 'allow_once');

      if (i >= WARMUP) times.push(now() - started);
    }

    expect(stat('4 allow: decision signed, sent, checked, gate resolved, answered', times).p95).toBeLessThan(500);
  }, 120_000);

  it('row 5: a stream event reaches the page', async () => {
    const times: number[] = [];

    for (let i = 0; i < SAMPLES + WARMUP; i++) {
      const marker = `bench-${i}-${Math.random()}`;
      const started = now();

      await daemon.call('event.append', { sessionId: 's1', event: { type: 'TurnDelta', turnId: 'bench', delta: marker } });

      const seen = await waitFor('the event', () => events.find((entry) => entry.event.delta === marker), 5000);

      if (i >= WARMUP) times.push(seen.at - started);
    }

    expect(stat('5 a stream event is shown', times).p95).toBeLessThan(500);
  }, 120_000);

  it('row 15: after the network drops, the page is back and caught up', async () => {
    const times: number[] = [];

    for (let i = 0; i < 20; i++) {
      const started = now();
      const mark = stateLog.length;
      // Cut the link the way a dead network would: the socket closes under the page.
      (link as unknown as { socket: { terminate(): void } }).socket.terminate();
      await waitFor('the link to drop', () => link.current.kind !== 'operate' && link.current.kind !== 'view' || undefined, 5000);
      await waitFor('the link to be back', () => link.current.kind === 'operate' || undefined, 15_000);

      const back = stateLog.slice(mark).find((entry) => entry.kind === 'operate');

      times.push((back?.at ?? now()) - started);

      // Inside the daemon's two-minute grace the session comes back at the level it had: no passkey, and the page
      // subscribes to the stream again by itself.
    }

    expect(stat('15 reconnect: socket lost, session back at the level it had (includes the first retry delay)', times).p95).toBeLessThan(5000);
  }, 240_000);

  it('resources: the daemon with SDC Anywhere on', async () => {
    await sleep(500);

    extra.daemonMemoryEnabledOneBrowser = memoryOf(daemon.pid);

    const before = extra.daemonMemoryBeforeEnable as number | null;
    const after = extra.daemonMemoryEnabledOneBrowser as number | null;

    if (before && after) extra.daemonExtraMemoryMB = round((after - before) / 1024 / 1024);
  });
});
