/**
 * Dev-only: the whole trust flow, against a real SSH server (0.7.13).
 *
 *   node sdc/scripts/remote/probe-remote.mjs [user@host] [port]
 *
 * Defaults to `root@github.com:22`, which is a real sshd that answers, presents a host key and then
 * refuses SDC's key - i.e. exactly the shape of a VPS nobody has set up yet, without needing a password
 * for anything.
 *
 * What it proves, in the order the daemon does it:
 *
 *   1. `host.add` **scans** the host key (`ssh-keyscan`, no authentication), records the host
 *      `untrusted`, and answers with the host id;
 *   2. the `HostStatus` event carries the sentence in `detail` and the fingerprint in `hostKey` - the
 *      two fields 0.7.13 added, because the sentences used to travel in `platform`;
 *   3. `host.trust` pins that fingerprint (re-scanning first: what was confirmed is what is stored),
 *      and the pin lands in SDC's own `<data>/ssh/known_hosts`;
 *   4. the probe then runs with `StrictHostKeyChecking=yes` against that pin, and the sentence that comes
 *      back is about *authentication* (SDC's key is not on the host yet), not about the host key. If it
 *      still said `Host key verification failed`, the pin would not be doing its job.
 *
 * `SDC_PORT` and the paths are the same the other probes use. Clean up afterwards with
 * removing the `probe` host in the app and by deleting the probe's line from
 * SDC's `known_hosts` if you do not want to keep it.
 */
import { createConnection } from 'node:net';

const target = process.argv[2] ?? 'root@github.com';
const port = Number(process.argv[3] ?? process.env.SDC_PORT ?? 7811);

const socket = createConnection({ port, host: '127.0.0.1' });
let buffer = '';
let seq = 0;
const started = Date.now();

/** The last `HostStatus` per host - the status, the sentence and the fingerprint the daemon pushed. */
const states = new Map();

/** The calls still waiting for an answer, by envelope id. One listener serves all of them - and it also
 *  keeps reading the *notifications*, which is the point: a probe that stopped listening after its first
 *  answer would miss every `HostStatus` the scan pushes (which is exactly how this file's first version
 *  reported "nothing to trust" for a host that was about to be untrusted). */
const pending = new Map();

socket.on('data', (chunk) => {
  buffer += chunk.toString();

  const lines = buffer.split('\n');
  buffer = lines.pop() ?? '';

  for (const line of lines) {
    if (line.trim() === '') {
      continue;
    }

    let message;

    try {
      message = JSON.parse(line);
    } catch {
      continue;
    }

    const at = `${String(Date.now() - started).padStart(6)}ms`;

    if (message.event?.type === 'HostStatus') {
      states.set(message.event.hostId, message.event);

      console.log(
        `${at} HostStatus ${message.event.status} · detail=${JSON.stringify(message.event.detail ?? null)} · hostKey=${message.event.hostKey ?? 'null'} · platform=${JSON.stringify(message.event.platform ?? null)}`,
      );
    }

    const waiting = pending.get(message.id);

    if (waiting === undefined) {
      continue;
    }

    pending.delete(message.id);

    if (message.error) {
      waiting.reject(new Error(`${message.error.code}: ${message.error.message}`));
    } else {
      waiting.resolve(message.result);
    }
  }
});

/** Sends one request and resolves with its result (or throws its error). */
function call(method, params) {
  return new Promise((resolve, reject) => {
    const id = `probe-remote-${(seq += 1)}`;

    pending.set(id, { resolve, reject });
    socket.write(`${JSON.stringify({ v: '0.1', id, method, params })}\n`);
  });
}

const main = async () => {
  const added = await call('host.add', { type: 'ssh', target, label: 'probe' });

  console.log(`answer: ${JSON.stringify(added)}`);

  /* Wait for the scan to reach a verdict: `connecting` is the daemon still working (keyscan + handshake
     fallback can take a few seconds), and this is the first thing the daemon pushes after the answer. */
  for (let waited = 0; waited < 30_000; waited += 500) {
    const state = states.get(added.hostId);

    if (state !== undefined && state.status !== 'connecting') {
      break;
    }

    await new Promise((resolve) => setTimeout(resolve, 500));
  }

  const { hosts } = await call('session.list', {});
  const host = hosts.find((candidate) => candidate.hostId === added.hostId);

  console.log(`row: ${JSON.stringify(host)}`);

  if (host?.target === null || host?.target === undefined) {
    console.log('the host was not recorded with an address - that is the 0.7.0 bug, still');

    socket.end();

    return;
  }

  console.log(`address: ${host.target}${host.port === null || host.port === undefined ? '' : `:${host.port}`}`);

  /* The **relaunch** path: ask what the host presents now, as a window that never saw the `HostStatus`
     would have to. This is the call that makes the trust card answerable days later. */
  const scanned = await call('host.key', { hostId: added.hostId });

  console.log(`host.key: ${JSON.stringify(scanned)}`);

  /* And the host's own environment - not the laptop's ten checks under the host's name. */
  const first = await call('host.doctor', { hostId: added.hostId });

  console.log(`host.doctor: ${first.checks.map((row) => `${row.id}=${row.state}${row.fix === undefined ? '' : `(${row.fix})`}`).join(' ')}`);

  const state = states.get(added.hostId);

  if (state?.status !== 'untrusted' || scanned.hostKey === undefined || scanned.hostKey === '') {
    console.log(`nothing to trust (status is ${state?.status ?? 'unknown'}) - the pin may already be in place`);

    /* Nothing to pin, but a pinned host is exactly the one a command can actually run on (0.7.13). */
    await exerciseRemotePaths(added.hostId);

    socket.end();

    return;
  }

  const trusted = await call('host.trust', { hostId: added.hostId, fingerprint: scanned.hostKey });

  console.log(`trust answer: ${JSON.stringify(trusted)}`);

  await new Promise((resolve) => setTimeout(resolve, 2000));

  /* After the pin: the same two calls, so the change is visible in the answers rather than asserted -
     `host.key` should now say `matches: true`, and the doctor's `hostkey` row `ok`. */
  const pinned = await call('host.key', { hostId: added.hostId });

  console.log(`host.key again: ${JSON.stringify(pinned)}`);

  const again = await call('host.doctor', { hostId: added.hostId });

  console.log(`host.doctor again: ${again.checks.map((row) => `${row.id}=${row.state}`).join(' ')}`);

  await exerciseRemotePaths(added.hostId);

  socket.end();
};

/**
 * The two paths 0.7.13 adds on top of the trust flow, and each one is a *command that actually ran on the
 * host* rather than a shape that looks right:
 *
 *   1. `shell.run { line }` - a whole line through the host's own shell (`pwd` says which folder answered);
 *   2. `pty.open { line }` + `pty.output` + `pty.close` - a long-running process, and the **group kill**
 *      that stops it. The proof is in the output: the line echoes the process's own pgid, which is what
 *      `kill -TERM -<pid>` addresses; if the group were not its own, a cancel would only kill the shell
 *      that started it.
 *
 * Both are wrapped in `try`, because a host is somebody's machine: no `setsid`, no `ps`, no writable
 * `$HOME` are all legitimate answers, and the log says which one came back.
 */
async function exerciseRemotePaths(hostId) {
  /*
   * `ssh.key` as well: SDC's own public key, which is the line a person pastes by hand when a host
   * requires a verification code. It is the one case the daemon deliberately does not automate, so a
   * probe that proves the app can *show* the way out is worth the two lines.
   */
  const sdcKey = await call('ssh.key', {});

  console.log(`ssh.key: exists=${sdcKey.exists} path=${sdcKey.path} key=${JSON.stringify(sdcKey.publicKey)}`);

  /*
   * The file and git layer, on the host (0.7.13) - the methods that made a remote chat usable at all.
   *
   * They are asked with the `hostId` and **no session**: the point is that the routing works (`fs.list` on
   * `/` of a host, `git.status` in a folder there), and each answer is either real data or the daemon's own
   * sentence. On a host whose key is not installed yet, the sentence is the authentication one - which is
   * the honest answer and the thing this probe exists to show.
   */
  try {
    const listing = await call('fs.list', { path: '/', hostId });

    console.log(`fs.list /: entries=${listing.entries.length} hidden=${listing.hidden} first=${JSON.stringify(listing.entries.slice(0, 3).map((row) => row.name))}`);
  } catch (error) {
    console.log(`fs.list /: ${error.message}`);
  }

  try {
    /* `git.status` names its folder `root` (its own contract since 0.7.9), not `cwd`. */
    const status = await call('git.status', { root: '/', hostId });

    console.log(`git.status /: branch=${JSON.stringify(status.branch)} changed=${status.changed?.length ?? 0}`);
  } catch (error) {
    console.log(`git.status /: ${error.message}`);
  }

  try {
    const ran = await call('shell.run', { line: 'pwd; echo sdc-line-ok', hostId });

    /* A failed remote run answers with a *shape* rather than an error (`ok: false`, no exit code), so the
       sentence is in `stderr`/`error` - printing only the exit code would hide the reason. */
    console.log(
      `shell.run line: ok=${ran.ok} exit=${ran.exitCode} stdout=${JSON.stringify(ran.stdout.trim())} stderr=${JSON.stringify((ran.stderr || ran.error?.explanation || '').slice(0, 160))}`,
    );
  } catch (error) {
    console.log(`shell.run line: refused or unreachable - ${error.message}`);
  }

  try {
    const opened = await call('pty.open', {
      line: 'echo "started $(id -un) in $(pwd)"; echo "pgid $(ps -o pgid= -p $$ | tr -d \' \')"; sleep 30',
      hostId,
    });

    console.log(`pty.open: ${JSON.stringify(opened)}`);

    await new Promise((resolve) => setTimeout(resolve, 1500));

    const tail = await call('pty.output', { ptyId: opened.ptyId });

    console.log(`pty.output: state=${tail.state} command=${tail.command} lines=${JSON.stringify(tail.lines)}`);

    const closed = await call('pty.close', { ptyId: opened.ptyId });

    console.log(`pty.close: ${JSON.stringify(closed)} - the far side was signalled with kill -TERM -<pgid>`);
  } catch (error) {
    console.log(`pty.open: refused or unreachable - ${error.message}`);
  }
}

main().catch((error) => {
  console.error(`failed: ${error.message}`);
  socket.end();
});

socket.setTimeout(90_000);
socket.on('timeout', () => socket.end());
socket.on('close', () => process.exit(0));
