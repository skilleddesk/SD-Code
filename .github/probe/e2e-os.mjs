/**
 * Dev-only: one end-to-end pass over the daemon, the same on Windows, macOS and Linux.
 *
 *   node _verify/e2e-os.mjs [port]                      local checks only
 *   SDC_E2E_SSH="user@127.0.0.1" SDC_E2E_PASS=... node _verify/e2e-os.mjs [port]   + a real SSH host
 *
 * Prints PASS/FAIL per step and exits 1 when any step failed. Never prints a password.
 */
import fs from 'node:fs';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';

const port = Number(process.argv[2] ?? 7811);
const results = [];
let seq = 0;

function call(method, params = {}, timeout = 90000) {
  return new Promise((resolve) => {
    const id = `e-${++seq}`;
    const socket = net.connect(port, '127.0.0.1');
    let buffer = '';
    const timer = setTimeout(() => { socket.destroy(); resolve({ error: { message: `${method}: no answer in ${timeout} ms` } }); }, timeout);

    socket.on('connect', () => socket.write(`${JSON.stringify({ v: '0.1', id, method, params })}\n`));
    socket.on('data', (chunk) => {
      buffer += chunk.toString();
      for (const line of buffer.split('\n')) {
        try {
          const message = JSON.parse(line);
          if (message.id === id) { clearTimeout(timer); socket.end(); resolve(message); }
        } catch { /* partial line */ }
      }
      buffer = buffer.slice(buffer.lastIndexOf('\n') + 1);
    });
    socket.on('error', (error) => { clearTimeout(timer); resolve({ error: { message: error.message } }); });
  });
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

function check(name, ok, detail) {
  results.push({ name, ok });
  const text = typeof detail === 'string' ? detail : JSON.stringify(detail);
  console.log(`${ok ? 'PASS' : 'FAIL'}  ${name}${text ? `  ${text.slice(0, ok ? 200 : 1500)}` : ''}`);
}

async function shellRoundTrip(label, hostId, cwd) {
  const opened = await call('pty.open', { command: 'shell', shell: true, hostId, cwd, cols: 100, rows: 30 });
  const ptyId = opened.result?.ptyId;

  if (!ptyId) {
    check(`${label}: terminal opens`, false, opened.error ?? opened);
    return;
  }

  const marker = `sdc-e2e-${Date.now()}`;
  const newline = hostId === 'local' && process.platform === 'win32' ? '\r\n' : '\n';

  await sleep(1500);
  await call('pty.write', { ptyId, data: `echo ${marker}${newline}` });

  let seen = '';

  for (let i = 0; i < 20 && !seen.includes(marker); i++) {
    await sleep(500);
    seen = JSON.stringify((await call('pty.output', { ptyId })).result ?? '');
  }

  check(`${label}: terminal runs a command`, seen.includes(marker), seen.includes(marker) ? '' : seen.slice(-800));
  await call('pty.close', { ptyId });
}

async function filesRoundTrip(label, hostId, root) {
  const file = `${root}/e2e.txt`.replace(/\\/g, '/');
  const text = `hello from ${os.platform()} ${Date.now()}\n`;
  const written = await call('fs.write', { hostId, path: file, text });

  check(`${label}: fs.write`, !written.error, written.error ?? '');

  const read = await call('fs.read', { hostId, path: file });
  const got = read.result?.text ?? read.result?.content ?? '';

  check(`${label}: fs.read returns what was written`, got === text, read.error ?? { got });

  const listed = await call('fs.list', { hostId, path: root });

  check(`${label}: fs.list sees the file`, JSON.stringify(listed.result ?? '').includes('e2e.txt'), listed.error ?? '');

  const ran = await call('shell.run', { hostId, command: 'shell', line: 'echo e2e-run-ok', timeoutMs: 60000 });

  check(`${label}: shell.run`, ran.result?.ok === true && String(ran.result?.stdout ?? '').includes('e2e-run-ok'), ran.error ?? ran.result);
}

/* ---- the daemon itself ---- */
const status = await call('host.status');

check('host.status answers', status.result?.status === 'connected', status.result ? `${status.result.platform} · sdcd ${status.result.sdcd} · keychain ${status.result.keychain}` : status.error);

const providers = await call('provider.list');
const list = providers.result?.providers ?? providers.result ?? [];

check('provider.list answers', Array.isArray(list) && list.length > 0, providers.error ?? `${list.length} providers`);

for (const id of ['claude', 'openai']) {
  const card = list.find((provider) => provider.id === id);

  check(`${id} CLI is found`, card !== undefined && !/not installed/.test(card.detail ?? ''), card ? `${card.status} · ${card.detail}` : 'no card');
}

const doctor = await call('host.doctor', {}, 120000);
const rows = doctor.result?.checks ?? [];

check('host.doctor answers', rows.length > 0, doctor.error ?? rows.map((row) => `${row.id}:${row.state}`).join(' '));
check('doctor sees node and git', ['node', 'git'].every((id) => rows.find((row) => row.id === id)?.state === 'ok'), rows.filter((row) => ['node', 'git', 'ssh'].includes(row.id)));

/* ---- a local project ---- */
const root = fs.mkdtempSync(path.join(os.tmpdir(), 'sdc e2e '));

fs.writeFileSync(path.join(root, 'package.json'), '{ "name": "e2e", "version": "1.0.0" }\n');

const project = await call('project.add', { hostId: 'local', root });
const projectId = project.result?.projectId ?? project.result?.id;

check('local: project.add (a folder with a space)', projectId !== undefined, project.error ?? project.result);

const session = await call('session.open', { hostId: 'local', projectId, title: 'e2e' });

check('local: session.open', session.result?.sessionId !== undefined, session.error ?? '');
await filesRoundTrip('local', 'local', root);
await shellRoundTrip('local', 'local', root);

/* ---- a CLI sign-in starts and shows its link ---- */
const login = await call('cli.login', { providerId: 'claude' });
const loginId = login.result?.loginId;
let loginState = null;

for (let i = 0; loginId && i < 20; i++) {
  await sleep(1000);
  loginState = (await call('cli.login.status', { loginId })).result;
  if (loginState?.url) break;
}

check('claude sign-in shows its link', Boolean(loginState?.url), login.error ?? loginState?.lines ?? '');

if (loginId) await call('cli.login.cancel', { loginId });

/* ---- a real CLI turn: it must END - with an answer, or with a sentence saying why not - never hang ---- */
async function cliTurn(engine, provider, model) {
  const opened = await call('session.open', { hostId: 'local', projectId, title: `e2e ${engine}` });
  const sessionId = opened.result?.sessionId;
  const before = (await call('event.list', {})).result;
  const since = Math.max(0, ...((before?.events ?? before ?? []).map((row) => row.seq ?? 0)));
  const started = await call('engine.start', { sessionId, prompt: 'Reply with just the word: ok', engine, model, provider, tier: 'Balanced', agent: false, autonomy: 'auto', understand: false });

  if (started.error) {
    check(`${engine} turn starts`, false, started.error);
    return;
  }

  let end = null;
  let text = '';

  for (let i = 0; i < 120 && end === null; i++) {
    await sleep(1000);
    const listed = (await call('event.list', { since })).result;
    const events = (listed?.events ?? listed ?? []).map((row) => row.event ?? row);

    text = '';

    for (const event of events) {
      if (event.type === 'TurnDelta') text += event.delta ?? '';
      if (event.type === 'ErrorRaised' && event.turnId === started.result?.turnId) end = event;
      if (['TurnCompleted', 'TurnFailed', 'TurnCancelled'].includes(event.type) && event.turnId === started.result?.turnId) end = event;
    }
  }

  const said = end?.type === 'TurnCompleted' ? text : (end?.title ? `${end.title} - ${end.explanation ?? ''}` : JSON.stringify(end));

  check(`${engine} turn ends within 2 min`, end !== null, end === null ? 'still running after 120 s - the hang the user sees' : `${end.type}: ${String(said).slice(0, 400)}`);
}

if (process.env.SDC_E2E_TURNS) {
  await cliTurn('claude_code', 'claude', 'sonnet');
  await cliTurn('codex', 'openai', 'gpt-5.1-codex');
}

/* ---- a real SSH host ---- */
const target = process.env.SDC_E2E_SSH;
const password = process.env.SDC_E2E_PASS ?? '';

async function hostEvent(hostId, since, want) {
  for (let i = 0; i < 90; i++) {
    const listed = await call('event.list', { since });
    const events = (listed.result?.events ?? listed.result ?? []).map((row) => row.event ?? row.payload ?? row);
    const mine = events.filter((event) => event.type === 'HostStatus' && event.hostId === hostId);
    const last = mine.at(-1);

    if (last && want.includes(last.status)) return last;
    await sleep(1000);
  }

  return null;
}

if (target) {
  const before = (await call('event.list', {})).result;
  const since = Math.max(0, ...((before?.events ?? before ?? []).map((row) => row.seq ?? 0)));
  const added = await call('host.add', { target, password }, 120000);
  const hostId = added.result?.hostId;

  check('ssh: host.add answers', Boolean(hostId), added.error ?? added.result);

  if (hostId) {
    let state = await hostEvent(hostId, since, ['untrusted', 'connected', 'offline', 'changed']);

    if (state?.status === 'untrusted' && state.hostKey) {
      const trusted = await call('host.trust', { hostId, fingerprint: state.hostKey, password }, 120000);

      check('ssh: host.trust answers', !trusted.error, trusted.error ?? '');
      state = await hostEvent(hostId, since, ['connected', 'offline']);
    }

    check('ssh: host signs in and is connected', state?.status === 'connected', state ?? 'no HostStatus');

    const remoteRoot = '/tmp/sdc e2e';

    await call('shell.run', { hostId, command: 'shell', line: `mkdir -p '${remoteRoot}'`, timeoutMs: 60000 });
    await filesRoundTrip('ssh', hostId, remoteRoot);
    await shellRoundTrip('ssh', hostId, remoteRoot);

    const remoteDoctor = await call('host.doctor', { hostId }, 120000);

    check('ssh: host.doctor answers', (remoteDoctor.result?.checks ?? []).length > 0, remoteDoctor.error ?? '');

    /* SDC_E2E_KEEP: a real signed-in host whose connection must not be closed by the test. */
    if (!process.env.SDC_E2E_KEEP) {
      const removed = await call('host.remove', { hostId });

      check('ssh: host.remove', !removed.error, removed.error ?? '');
    }
  }
}

const failed = results.filter((result) => !result.ok);

console.log(`\n${results.length - failed.length}/${results.length} passed${failed.length ? ` · FAILED: ${failed.map((result) => result.name).join('; ')}` : ''}`);
fs.rmSync(root, { recursive: true, force: true });
process.exit(failed.length ? 1 : 0);
