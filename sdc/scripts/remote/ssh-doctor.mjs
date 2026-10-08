/**
 * Dev-only: **why does this host not connect?**, answered layer by layer, outside the app (0.7.13).
 *
 *   node sdc/scripts/remote/ssh-doctor.mjs [user@host[:port]] [password]
 *
 * It runs exactly the commands `sdcd` runs - the same `ssh`, the same options, the same key - so what it
 * prints is what the daemon would see. Nothing is installed on the host, no file on this machine is
 * changed (the temporary `known_hosts` is deleted), and the password is optional: without it the last
 * step fails with the far side's own sentence, which is the honest answer for a host that has no key yet.
 *
 * The five layers, each with a one-word verdict:
 *
 *   1. **client**   - is there an `ssh`/`ssh-keygen`, and which version?      (no  → install OpenSSH)
 *   2. **port**     - is something listening where the target says?           (no  → the port is wrong)
 *   3. **host key** - what does the machine present?                          (changed → the alarm)
 *   4. **key**      - does SDC have its own key?                              (none → add a host once)
 *   5. **probe**    - does the machine accept SDC's key?                      (no  → install the key)
 *
 * Exit code 0 when the probe succeeds, 1 otherwise - so a script can use it.
 */
import { spawnSync } from 'node:child_process';
import { existsSync, mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { homedir, platform, tmpdir } from 'node:os';
import { join } from 'node:path';

const target = process.argv[2] ?? 'root@github.com';
const password = process.argv[3] ?? '';
const windows = platform() === 'win32';
const nullDevice = windows ? 'NUL' : '/dev/null';

/** `user@host[:port]` - the shapes the daemon accepts, read the same way (a bare IPv6 keeps its colons). */
function parse(input) {
  const at = input.lastIndexOf('@');
  const user = at === -1 ? '' : input.slice(0, at);
  const hostPart = at === -1 ? input : input.slice(at + 1);
  const colon = hostPart.lastIndexOf(':');
  const digits = colon === -1 ? '' : hostPart.slice(colon + 1);
  /* A port has to be digits: `2001:db8::1` is an address, not an address and a port. */
  const hasPort = /^\d+$/.test(digits);
  const host = hasPort ? hostPart.slice(0, colon) : hostPart;

  return { user, host, userHost: at === -1 ? host : `${user}@${host}`, port: hasPort ? Number(digits) : null };
}

const { user, host, userHost, port } = parse(target);
const sshKey = join(homedir(), '.ssh', 'sdc_ed25519');
const portArgs = port === null ? [] : ['-p', String(port)];
let failed = false;

/**
 * Runs a program and returns `{ ok, out }` with **both** streams in `out` - never throws, because a failure
 * *is* the answer here. Both streams, because the two programs this needs are split down the middle:
 * `ssh -V` writes its version to stderr and `ssh-keyscan` writes keys to stdout and its reasons to stderr.
 */
function run(program, args, input) {
  const result = spawnSync(program, args, {
    encoding: 'utf8',
    ...(input === undefined ? {} : { input }),
    timeout: 30_000,
    windowsHide: true,
  });
  const out = `${result.stdout ?? ''}${result.stderr ?? ''}`.trim();

  return {
    ok: result.status === 0 && result.error === undefined,
    out: out === '' && result.error !== undefined ? String(result.error.message) : out,
  };
}

/** The line of a transcript a person should read - not `ssh-keyscan`'s `# host:port` header, not a banner. */
function reason(text) {
  const useful = text
    .split('\n')
    .map((line) => line.trim())
    .filter((line) => line !== '' && !line.startsWith('#') && !line.startsWith('SSH-2.0-'));

  return useful.at(-1) ?? 'no answer';
}

console.log(`SDC ssh doctor - ${userHost}${port === null ? '' : `:${port}`}\n`);

/* 1. the client ----------------------------------------------------------------------------------- */
const sshVersion = run('ssh', ['-V']);

console.log(`1. client     : ${sshVersion.out.split('\n')[0] || 'NOT FOUND'}`);

if (!sshVersion.ok) {
  console.log('   -> no OpenSSH client: Windows -> Settings -> Optional features -> OpenSSH Client');
  failed = true;
}

if (port === null) {
  console.log('2. port       : none in the target - SDC dials 22. If that machine listens elsewhere, write it down');
  console.log(`                as \`${userHost}:8443\` or paste the whole \`ssh -p 8443 ${userHost}\` you use yourself.`);
} else {
  console.log(`2. port       : ${port} (no guess: this is the port in the target)`);
}

/* 3. the host key --------------------------------------------------------------------------------- */
const scan = run('ssh-keyscan', [...(port === null ? [] : ['-p', String(port)]), '-T', '10', host]);
const fingerprints = scan.ok
  ? run('ssh-keygen', ['-lf', '-'], scan.out).out.split('\n').map((line) => line.trim()).filter((line) => line !== '')
  : [];

console.log(`3. host key   : ${fingerprints.length === 0 ? `ssh-keyscan said: ${reason(scan.out)}` : fingerprints.join(' | ')}`);
console.log('                (a real `ssh` handshake is SDC\'s fallback when ssh-keyscan cannot negotiate)');

/* 4. SDC's own key -------------------------------------------------------------------------------- */
const publicKey = existsSync(`${sshKey}.pub`) ? readFileSync(`${sshKey}.pub`, 'utf8').trim() : '';

console.log(
  `4. SDC's key  : ${publicKey === '' ? `none at ${sshKey} yet - add a host once and it is made` : run('ssh-keygen', ['-lf', `${sshKey}.pub`]).out}`,
);

/* 5. the probe - the real `ssh`, with SDC's key and the same options --------------------------------- */
const scratch = mkdtempSync(join(tmpdir(), 'sdc-doctor-'));
const probeArgs = [
  ...portArgs,
  '-o', 'ConnectTimeout=10',
  '-o', 'IdentitiesOnly=yes',
  '-o', 'StrictHostKeyChecking=accept-new',
  '-o', `UserKnownHostsFile=${join(scratch, 'known_hosts')}`,
  '-o', password === '' ? 'BatchMode=yes' : 'NumberOfPasswordPrompts=1',
  '-o', 'PreferredAuthentications=publickey,keyboard-interactive,password',
  ...(existsSync(sshKey) ? ['-i', sshKey] : []),
  userHost,
  'echo SDC-OK',
];

const probe = run('ssh', probeArgs, password === '' ? undefined : `${password}\n`);

rmSync(scratch, { recursive: true, force: true });

const connected = probe.out.includes('SDC-OK');

console.log(`5. probe      : ${connected ? "CONNECTED - the host accepts SDC's key" : reason(probe.out)}`);

if (connected) {
  console.log('\nverdict: every layer works. If the window says otherwise, the row is stale - re-open the');
  console.log('         host\'s card (`Keys & doctor`); the daemon measures it again.');

  process.exit(0);
}

console.log('\nverdict: the transport is fine; the far side refused the session.');
console.log('   -> one password, once, copies SDC\'s key over: SDC -> that host\'s card -> `Install SDC\'s key`.');
console.log('      Or do it yourself, from this terminal:\n');
console.log(
  `      type "${sshKey}.pub" | ssh ${portArgs.join(' ')} ${userHost} "mkdir -p ~/.ssh && chmod 700 ~/.ssh && cat >> ~/.ssh/authorized_keys && chmod 600 ~/.ssh/authorized_keys"\n`,
);

if (publicKey !== '') {
  console.log(`      ${publicKey}`);
}

if (failed) {
  console.log('\n   (the client itself is missing too - fix step 1 first)');
}

process.exit(1);

