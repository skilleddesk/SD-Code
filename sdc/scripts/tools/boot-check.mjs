// Dev-only: is the *built* frontend actually booting? Serves `app/dist` (or uses `vite preview`), drives
// Edge over CDP, and prints what `#root` ended up containing plus every console error.
//
//   node sdc/scripts/tools/serve-dist.mjs 4599          # in one shell
//   msedge --headless=new --remote-debugging-port=9222 http://127.0.0.1:4599/
//   node sdc/scripts/tools/boot-check.mjs 9222
//
// Exit code: 0 when the window rendered, 1 when it did not (`#root` empty, no `#app`, unpainted
// `#app` - and, on the window's own `tauri.localhost` URL, anything at all in the console), 3 when CDP
// never answered. 0.11.3 added the code and the deadlines below: on the installed 0.11.3 check this
// script printed its first line and then hung - `Runtime.evaluate` posted across a `Page.reload` is
// answered by nobody, and the old `send` waited forever, so the check that exists to catch a window
// which does not boot was the one that could not finish.
const port = process.argv[2] ?? '9222';
const appUrl = process.argv[3] ?? 'http://127.0.0.1:4599';
const deadlineMs = Number(process.env.BOOT_CHECK_DEADLINE_MS ?? 45000);
const callMs = Number(process.env.BOOT_CHECK_CALL_MS ?? 8000);
/* A reload in a plain browser legitimately logs the bridge's absence; the window has no such excuse. */
const strict = appUrl.includes('tauri.localhost');
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

const watchdog = setTimeout(() => {
  console.log(`boot-check: nothing answered within ${deadlineMs}ms - the page is up but not debuggable`);
  process.exit(3);
}, deadlineMs);

async function page() {
  for (let attempt = 0; attempt < 40; attempt += 1) {
    try {
      const list = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
      /* Only the app's own page: a fresh Edge profile also opens its own internal tabs, and attaching
         to one of those measures Microsoft's UI instead of ours. */
      const found = list.find((target) => target.type === 'page' && target.url.startsWith(appUrl));

      if (found) {
        return found;
      }
    } catch {
      /* not up yet */
    }

    await sleep(250);
  }

  throw new Error(`no debuggable page on ${port}`);
}

const target = await page();
const socket = new WebSocket(target.webSocketDebuggerUrl);

await new Promise((resolve, reject) => {
  socket.addEventListener('open', resolve, { once: true });
  socket.addEventListener('error', reject, { once: true });
});

let nextId = 1;
const pending = new Map();
const logs = [];

socket.addEventListener('message', (event) => {
  const message = JSON.parse(event.data);

  if (message.method === 'Runtime.consoleAPICalled') {
    logs.push(`console.${message.params.type}: ${message.params.args.map((arg) => arg.value ?? arg.description ?? '').join(' ')}`);
  }

  if (message.method === 'Runtime.exceptionThrown') {
    const details = message.params.exceptionDetails;

    logs.push(`exception: ${details.exception?.description ?? details.text}`);
  }

  if (message.id && pending.has(message.id)) {
    pending.get(message.id)(message);
    pending.delete(message.id);
  }
});

const send = (method, params = {}) =>
  new Promise((resolve) => {
    const id = nextId++;
    const timer = setTimeout(() => {
      pending.delete(id);
      logs.push(`cdp: no answer to ${method} within ${callMs}ms - the page reloaded or went away`);
      resolve({ timedOut: true });
    }, callMs);

    pending.set(id, (message) => {
      clearTimeout(timer);
      resolve(message);
    });

    try {
      socket.send(JSON.stringify({ id, method, params }));
    } catch (error) {
      clearTimeout(timer);
      pending.delete(id);
      logs.push(`cdp: ${method} could not be sent - ${error.message}`);
      resolve({ timedOut: true });
    }
  });

const evaluate = async (expression) => {
  const answer = await send('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });

  return answer.result?.result?.value;
};

await send('Runtime.enable');
await send('Log.enable');
/* Reload so the exceptions from boot are captured, not missed. */
await send('Page.enable');
await send('Page.reload', { ignoreCache: true });
await sleep(4000);

const href = await evaluate('location.href');
const rootChildren = await evaluate("document.querySelector('#root')?.children.length ?? -1");
const appPresent = await evaluate("!!document.querySelector('#app')");
const appDisplay = await evaluate(
  "document.querySelector('#app') ? getComputedStyle(document.querySelector('#app')).display : null",
);
const bodyText = await evaluate('document.body.innerText ?? ""');

console.log(`url: ${href}`);
console.log(`root children: ${rootChildren}`);
console.log(`root html (first 300): ${JSON.stringify(String(await evaluate("document.querySelector('#root')?.innerHTML?.slice(0,300) ?? null")))}`);
console.log(`body text (first 300): ${JSON.stringify(String(bodyText).slice(0, 300))}`);
console.log(`#app present: ${appPresent}`);
console.log(`computed display of #app: ${appDisplay ?? 'no #app'}`);
console.log(`scripts loaded: ${await evaluate("Array.from(document.scripts).map(s => s.src.split('/').pop()).join(',')")}`);
console.log(`stylesheets: ${await evaluate("document.styleSheets.length")}`);
console.log('--- browser log ---');

for (const line of logs.slice(0, 25)) {
  console.log(`  ${line}`);
}

console.log(logs.length === 0 ? '  (nothing)' : `  (${logs.length} entries)`);

let failures = 0;

const check = (ok, sentence) => {
  console.log(`${ok ? 'ok  ' : 'FAIL'} ${sentence}`);

  if (!ok) {
    failures += 1;
  }
};

const errors = logs.filter((line) => line.startsWith('exception:') || line.startsWith('console.error'));

check(Number(rootChildren) >= 1, `the app mounted - #root children: ${rootChildren}`);
check(appPresent === true, 'the shell is in the page - #app present');
check(appDisplay !== null && appDisplay !== 'none', `#app is painted - display: ${appDisplay ?? 'no #app'}`);
check(String(bodyText).trim().length > 0, `there is something to read - ${String(bodyText).trim().length} chars`);

if (strict) {
  check(errors.length === 0, `nothing threw - ${errors.length} browser error(s)`);
} else {
  console.log(
    `${errors.length === 0 ? 'ok  ' : 'note'} nothing threw - ${errors.length} browser error(s) ` +
      '(a plain-browser run logs the missing bridge, so this is not counted here)',
  );
}

console.log(failures === 0 ? '\nboot-check: the window rendered' : `\nboot-check: ${failures} check(s) failed`);

clearTimeout(watchdog);
socket.close();
process.exit(failures === 0 ? 0 : 1);
