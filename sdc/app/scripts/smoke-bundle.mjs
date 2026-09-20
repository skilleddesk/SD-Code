// The window a user sees, checked before an installer is allowed to carry it.
//
// Why this exists: 0.4.1-0.4.3 shipped a frontend that *built*, type-checked, linted and unit-tested,
// and then crashed the moment it mounted - a store selector returned a new array, React saw the
// snapshot change on every commit, hit its update limit and unmounted the tree. The window was black,
// and every gate in the repository was green. Nothing here is clever: serve the built `dist`, open it
// in a real browser, and assert that the app put something on the page and reported no errors.
//
// No dependencies, and no CDP: the page writes its verdict into `<pre id="smoke-report">`, and
// `--dump-dom --virtual-time-budget` makes the browser run the timers, print the finished DOM and exit
// on its own. A plain `--headless <url>` would exit *before* the harness looked at the app.
import { spawn } from 'node:child_process';
import { existsSync, readFileSync, statSync } from 'node:fs';
import { createServer } from 'node:http';
import { dirname, extname, join, normalize } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const dist = join(here, '..', 'dist');
const settleMs = Number(process.env.SDC_SMOKE_SETTLE_MS ?? 4000);

if (!existsSync(join(dist, 'index.html'))) {
  console.error(`smoke: no build at ${dist} - run \`pnpm build\` first`);
  process.exit(2);
}

const types = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.json': 'application/json; charset=utf-8',
  '.svg': 'image/svg+xml',
  '.woff': 'font/woff',
  '.woff2': 'font/woff2',
  '.png': 'image/png',
  '.ico': 'image/x-icon',
};

/* The page that does the checking, in the browser, on the app's own origin.
 *
 * An `iframe` rather than a reload, because a crash on mount has to be *observed*: the app's frame
 * reports its own errors, and the parent reads what is left of the root. */
const harness = `<!doctype html>
<meta charset="utf-8">
<title>SDC bundle smoke</title>
<style>html,body{margin:0;background:#111}iframe{width:1200px;height:800px;border:0}</style>
<pre id="smoke-report" style="color:#eee">pending</pre>
<script>
  const errors = [];
  const frame = document.createElement('iframe');

  window.addEventListener('error', (event) => errors.push('window: ' + event.message));
  window.addEventListener('unhandledrejection', (event) => errors.push('rejection: ' + event.reason));

  frame.src = '/';
  frame.addEventListener('load', () => {
    frame.contentWindow.addEventListener('error', (event) => errors.push('app: ' + event.message));
    frame.contentWindow.addEventListener('unhandledrejection', (event) => errors.push('app rejection: ' + event.reason));
  });

  document.body.append(frame);

  setTimeout(() => {
    let report;

    try {
      const page = frame.contentDocument;
      const root = page && page.querySelector('#root');

      report = {
        errors,
        rootChildren: root ? root.children.length : -1,
        appElement: Boolean(page && page.querySelector('#app')),
        textLength: page && page.body ? page.body.innerText.length : 0,
        headline: ((page && page.body && page.body.innerText) || '').slice(0, 60),
      };
    } catch (error) {
      report = { errors: errors.concat(['reading the frame: ' + error.message]) };
    }

    document.querySelector('#smoke-report').textContent = JSON.stringify(report);
  }, ${settleMs});
</script>`;

/** A Chromium-family browser, wherever this machine keeps one.
 *
 * No `--version` probe: on Windows `msedge.exe --version` *opens Edge* rather than printing and
 * exiting, so asking a browser binary what it is hangs the build. Looking for the file is enough. */
function browser() {
  const names = process.platform === 'win32'
    ? ['msedge.exe', 'chrome.exe', 'chromium.exe']
    : ['google-chrome', 'google-chrome-stable', 'chromium', 'chromium-browser', 'microsoft-edge'];

  const directories = (process.env.PATH ?? '').split(process.platform === 'win32' ? ';' : ':').filter(Boolean);

  const candidates = [
    process.env.SDC_BROWSER,
    process.env.CHROME_PATH,
    /* The usual places, for a shell whose PATH is not the desktop's. */
    ...directories.flatMap((directory) => names.map((name) => join(directory, name))),
    'C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe',
    'C:\\Program Files\\Microsoft\\Edge\\Application\\msedge.exe',
    'C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe',
    '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
    '/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge',
  ].filter(Boolean);

  return candidates.find((candidate) => existsSync(candidate)) ?? null;
}

/** The report the page wrote, out of the DOM the browser printed. */
function reportFrom(dom) {
  const match = /<pre id="smoke-report"[^>]*>([\s\S]*?)<\/pre>/.exec(dom);

  if (!match) {
    return null;
  }

  const text = match[1]
    .replace(/&quot;/g, '"')
    .replace(/&#39;/g, "'")
    .replace(/&lt;/g, '<')
    .replace(/&gt;/g, '>')
    .replace(/&amp;/g, '&')
    .trim();

  if (text === 'pending') {
    return null;
  }

  try {
    return JSON.parse(text);
  } catch {
    return null;
  }
}

const server = createServer((request, response) => {
  const url = new URL(request.url ?? '/', 'http://127.0.0.1');

  if (url.pathname === '/__smoke.html') {
    response.writeHead(200, { 'content-type': 'text/html; charset=utf-8' });

    return void response.end(harness);
  }

  let file = join(dist, normalize(decodeURIComponent(url.pathname)));

  if (!file.startsWith(dist) || !existsSync(file) || statSync(file).isDirectory()) {
    file = join(dist, 'index.html');
  }

  response.writeHead(200, { 'content-type': types[extname(file)] ?? 'application/octet-stream' });
  response.end(readFileSync(file));
});

await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));

const port = server.address().port;
const executable = browser();

if (!executable) {
  /* A developer without a Chromium on their machine is not blocked; a release is. */
  if (process.env.CI) {
    console.error('smoke: no Chromium-family browser found in CI (set SDC_BROWSER)');
    server.close();
    process.exit(2);
  }

  console.log('smoke: no Chromium-family browser found - skipped (set SDC_BROWSER to check locally)');
  server.close();
  process.exit(0);
}

console.log(`smoke: serving ${dist} on ${port}, checking with ${executable}`);

const child = spawn(
  executable,
  [
    '--headless=new',
    '--disable-gpu',
    '--no-sandbox',
    '--no-first-run',
    '--no-default-browser-check',
    '--disable-extensions',
    '--dump-dom',
    /* Virtual time makes the harness's timer fire before the DOM is printed, and the browser leave by
       itself: one process, one answer, nothing to kill and nothing to hang. */
    `--virtual-time-budget=${settleMs}`,
    `http://127.0.0.1:${port}/__smoke.html`,
  ],
  { stdio: ['ignore', 'pipe', 'ignore'] },
);

let dom = '';

child.stdout.on('data', (chunk) => {
  dom += chunk.toString();
});

const left = new Promise((resolve) => child.on('exit', resolve));
const tooLong = new Promise((resolve) => setTimeout(() => resolve(null), 60_000));

await Promise.race([left, tooLong]);

child.kill();
server.close();

const report = reportFrom(dom);

if (report === null) {
  console.error('smoke: the browser never reported back');
  console.error(dom.slice(0, 500) || '(no output from the browser)');
  process.exit(1);
}

const checks = [
  ['the app mounted', (report.rootChildren ?? 0) > 0, `#root children: ${report.rootChildren}`],
  ['the shell is in the page', report.appElement === true, `#app present: ${report.appElement}`],
  ['there is something to read', (report.textLength ?? 0) > 200, `text: ${report.textLength} chars, starts "${report.headline}"`],
  ['nothing threw', (report.errors ?? []).length === 0, (report.errors ?? []).join(' | ') || 'no errors'],
];

let failed = 0;

for (const [name, ok, detail] of checks) {
  console.log(`${ok ? 'ok  ' : 'FAIL'} ${name} - ${detail}`);

  if (!ok) {
    failed += 1;
  }
}

console.log(failed === 0 ? 'smoke: the built window renders' : `smoke: ${failed} check(s) failed`);
process.exit(failed === 0 ? 0 : 1);

