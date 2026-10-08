/**
 * Dev-only: one screenshot of whatever WebView is on a CDP port, so an *installed* window can be
 * looked at rather than inferred. `node sdc/scripts/tools/shot.mjs 9227 sdc/scripts/tools/installed-0.4.4.png`
 *
 * The window is the app's WebView2, reachable because the app was started with
 * `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=<port>`.
 */
import { writeFileSync } from 'node:fs';

const port = process.argv[2] ?? '9227';
const out = process.argv[3] ?? 'installed.png';
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

const list = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
const page = list.find((target) => target.type === 'page');

if (!page) {
  throw new Error(`no page on ${port}: ${JSON.stringify(list)}`);
}

console.log(`target: ${page.url}`);

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

await rpc('Page.enable');
await sleep(500);

const shot = await rpc('Page.captureScreenshot', { format: 'png' });

writeFileSync(out, Buffer.from(shot.result.data, 'base64'));
console.log(`saved ${out}`);
socket.close();
