/**
 * "Is this install actually the version I think it is?" - answered from inside the window.
 *
 *   node sdc/scripts/tools/probe-versions.mjs [cdp-port]
 *
 * Two places in the window say the version, and both are read from something real: the status bar's
 * `v0.7.5 · sdcd 0.7.5` cell (`package.json` for the app, the event log's `HostStatus` for the daemon)
 * and Settings -> About's three rows. The About rows were **literals** until 0.7.5 - on a 0.7.5 build the
 * dialog said `v0.4.4` - which is exactly the kind of number a person checks when they ask how to verify
 * a version, so this probe reads all of them and holds them against the tree.
 */
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const port = process.argv[2] ?? '9251';
const repo = fileURLToPath(new URL('../../../', import.meta.url));
const expected = JSON.parse(readFileSync(`${repo}sdc/app/package.json`, 'utf8')).version;

const list = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
const page = list.find((target) => target.type === 'page' && target.url.includes('tauri.localhost'));

if (page === undefined) {
  throw new Error(`no page on port ${port}`);
}

const socket = new WebSocket(page.webSocketDebuggerUrl);

await new Promise((resolve, reject) => {
  socket.addEventListener('open', resolve, { once: true });
  socket.addEventListener('error', reject, { once: true });
});

let nextId = 1;
const pending = new Map();

socket.addEventListener('message', (event) => {
  const message = JSON.parse(event.data);

  if (message.id && pending.has(message.id)) {
    pending.get(message.id)(message);
    pending.delete(message.id);
  }
});

const rpc = (method, params = {}) =>
  new Promise((resolve) => {
    const id = nextId++;

    pending.set(id, resolve);
    socket.send(JSON.stringify({ id, method, params }));
  });

await rpc('Runtime.enable');

const answer = await rpc('Runtime.evaluate', {
  expression: `(async () => {
    const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
    const status = document.querySelector('#statusVersion')?.textContent ?? null;
    /* Which host the window is pointed at: the cell's second half is *that* host's daemon, so the
       name is what decides whether the two numbers are even meant to match. */
    const host = document.querySelector('#statusHost')?.textContent ?? null;

    /* Settings -> About: the gear opens the dialog, then the About tab. */
    document.querySelector('#openSettings')?.click();
    await wait(400);
    document.querySelector('[data-settings-tab="about"]')?.click();
    await wait(400);

    const rows = [...document.querySelectorAll('.settings-dlg .flex.items-center')]
      .map((row) => {
        const label = row.querySelector('.text-\\\\[13px\\\\]')?.textContent ?? '';
        const value = row.querySelector('span.font-mono')?.textContent ?? '';
        return { label, value };
      })
      .filter((row) => row.label !== '' && row.value !== '');

    /* Leave the window as it was found. */
    document.querySelector('.settings-dlg [title="Close"], .settings-dlg button[aria-label="Close"]')?.click();

    return { status, host, rows };
  })()`,
  returnByValue: true,
  awaitPromise: true,
});

if (answer.result?.exceptionDetails) {
  throw new Error(String(answer.result.exceptionDetails.exception?.description ?? 'eval failed'));
}

const { status, host, rows } = answer.result.result.value;
const about = Object.fromEntries(rows.map((row) => [row.label, row.value]));

console.log(`status bar: ${JSON.stringify(status)}`);
console.log(`active host: ${JSON.stringify(host)}`);
console.log(`about: ${JSON.stringify(about)}`);

let failures = 0;

const check = (ok, sentence) => {
  console.log(`${ok ? 'ok  ' : 'FAIL'} ${sentence}`);

  if (!ok) {
    failures += 1;
  }
};

check(String(status ?? '').includes(`v${expected}`), `the status bar names the app version (v${expected})`);

/*
 * The cell's second half is the **active host's** daemon (`activeHost?.sdcd` in
 * `panels/statusbar/StatusBar.tsx`), and the active host is not necessarily this machine. Point the
 * window at a VPS and the cell reads `v0.11.3 · sdcd 0.11.2` - correctly: that box really is running
 * 0.11.2. So the two numbers are only required to match when the active host is the local one, and
 * the question that matters either way - "is the daemon behind *this* window this build?" - is asked
 * of the daemon itself in About's `sdcd daemon` row, which the bridge fills from its own `host.status`
 * call (`src-tauri/src/sdcp.rs`). Before 0.11.3 this probe called the mismatch a failure and went red
 * on a correct install; `sdc/scripts/tools/host-cell-check.mjs` is the companion that shows the cell following
 * the active host by switching it and reading the cell again.
 */
const localHostName = 'Local'; /* strings.sidebar.hosts.local - the name `host.add` gives the local daemon */

if (host === localHostName) {
  check(String(status ?? '').includes(`sdcd ${expected}`), `and the daemon's (sdcd ${expected})`);
} else {
  console.log(
    `note the cell's daemon half belongs to the active host, which is ${JSON.stringify(host)} - ` +
      'not this machine, so its number is not held against this tree',
  );
  check(
    /sdcd \d+\.\d+\.\d+/.test(String(status ?? '')),
    `the cell still names a daemon version (${JSON.stringify(status)})`,
  );
}
check(about['SDC App'] === `v${expected}`, `About says SDC App v${expected} (${JSON.stringify(about['SDC App'])})`);
check(
  about['sdcd daemon'] === `v${expected}`,
  `About says sdcd daemon v${expected} (${JSON.stringify(about['sdcd daemon'])})`,
);
check(about['SDCP protocol'] === '0.1', `About names the protocol (${JSON.stringify(about['SDCP protocol'])})`);

console.log(failures === 0 ? '\nversions: pass' : `\nversions: ${failures} check(s) failed`);

socket.close();
process.exit(failures === 0 ? 0 : 1);
