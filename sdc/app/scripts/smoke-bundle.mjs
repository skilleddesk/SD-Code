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
        lightSurfaces: [],
        lightSurfaceCount: -1,
      };

      /*
       * The dark theme's own check: what is painted, not what was typed.
       *
       * 0.5.1 shipped a sidebar input with no background class, so the engine painted it with the
       * platform's field white - a solid rgb(255,255,255) rectangle in a dark window - and every gate
       * here passed, because "the app rendered" was the whole question. This walks the frame and fails
       * on any opaque, near-white background, which is what a light box in a dark theme is.
       *
       * Translucent white is deliberately not a hit: the kbd-lite chip on the accent-filled New chat
       * button is rgba(255,255,255,.16) and is supposed to be there.
       */
      const path = (element) => {
        const parts = [];
        let node = element;

        while (node && node.nodeType === 1 && parts.length < 4) {
          const id = node.id ? '#' + node.id : '';
          const cls =
            node.className && typeof node.className === 'string' && node.className.trim() !== ''
              ? '.' + node.className.trim().split(/\s+/).slice(0, 2).join('.')
              : '';

          parts.unshift(node.tagName.toLowerCase() + id + cls);
          node = node.parentElement;
        }

        return parts.join(' > ');
      };

      const styles = frame.contentWindow.getComputedStyle.bind(frame.contentWindow);

      for (const element of page.querySelectorAll('*')) {
        const rect = element.getBoundingClientRect();

        if (rect.width < 4 || rect.height < 4) continue;

        const match = styles(element).backgroundColor.match(/rgba?\(([^)]+)\)/);

        if (match === null) continue;

        const channels = match[1].split(',').map((part) => parseFloat(part));
        const alpha = channels.length > 3 ? channels[3] : 1;

        if (alpha < 0.9) continue;

        const luminance =
          (0.2126 * channels[0] + 0.7152 * channels[1] + 0.0722 * channels[2]) / 255;

        if (luminance > 0.75) {
          /* String concatenation rather than a template: this whole harness is itself inside one, and
             a nested template literal ends it early. */
          report.lightSurfaces.push(
            path(element) +
              ' [' + Math.round(rect.width) + 'x' + Math.round(rect.height) + '] = ' +
              styles(element).backgroundColor,
          );
        }
      }

      report.lightSurfaceCount = report.lightSurfaces.length;
      report.lightSurfaces = report.lightSurfaces.slice(0, 8);

      /* What each form control actually painted - the diagnostic that makes this check debuggable
         when it *does* fire (and the proof it is looking at the controls at all). */
      report.controls = [...page.querySelectorAll('input, textarea, select')].map((element) => {
        const style = styles(element);

        return (
          element.tagName.toLowerCase() +
          (element.id ? '#' + element.id : '') +
          ' [' + Math.round(element.getBoundingClientRect().width) + 'x' +
          Math.round(element.getBoundingClientRect().height) + '] bg=' + style.backgroundColor +
          ' appearance=' + style.appearance
        );
      });

      /*
       * And the other half of a control's look: focus has to be *visible*.
       *
       * The reset stops a control painting itself, and stops the engine drawing its own outline -
       * which is only acceptable because this app draws a token ring instead: the search wrapper and
       * the prompt box ring on focus-within, and the modal fields on focus:border-border-focus. This
       * measures that promise: focus the first control and require the paint to change.
       */
      const control = page.querySelector('input, textarea, select');

      if (control === null) {
        report.focusVisible = 'no control on this screen';
      } else {
        const surface = control.closest('.search-wrap, .prompt-box') ?? control;
        const before = styles(surface);
        const wasBorder = before.borderColor;
        const wasShadow = before.boxShadow;

        control.focus();

        const after = styles(surface);

        report.focusVisible =
          after.borderColor !== wasBorder || after.boxShadow !== wasShadow
            ? true
            : 'focused, and nothing painted differently (' + after.borderColor + ', ' + after.boxShadow + ')';
      }
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
  [
    'every form control is painted by a token',
    (report.controls ?? []).length === 0 ||
      (report.controls ?? []).every((line) => line.includes('bg=rgba(0, 0, 0, 0)')),
    (report.controls ?? []).join(' | ') || 'no form controls on this screen',
  ],
  [
    'keyboard focus is visible',
    report.focusVisible === true,
    report.focusVisible === true ? 'the ring is a token and it changed on focus' : String(report.focusVisible),
  ],
  [
    'no light box in a dark theme',
    report.lightSurfaceCount === 0,
    report.lightSurfaceCount === 0
      ? 'nothing opaque and near-white is painted'
      : `${report.lightSurfaceCount}: ${(report.lightSurfaces ?? []).join(' | ')}`,
  ],
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

