// The accessibility audit the README has promised since 0.6 and never run (0.7.10).
//
//   pnpm --filter @sdc/app smoke:a11y
//
// The README's "What is deliberately absent" said: "An axe-core pass and a screen-reader sweep. The
// accessibility work is in the markup (roles, `aria-current`, focus traps, `Escape` handling, the keymap
// reference), but the automated audit has not been run in CI yet." This is that audit.
//
// It reuses `smoke-bundle.mjs`'s approach on purpose - serve the built `dist`, open it in a real browser, let
// the page write its own verdict into a `<pre>`, and print the DOM with `--dump-dom --virtual-time-budget` -
// because that trick needs no CDP, no driver and no extra dependency, and a gate that only runs on a machine
// with a debugging port is a gate that never runs in CI.
//
// What it asserts:
//
//   * **no `serious` or `critical` violation** anywhere on the screen the app opens on, over the WCAG 2.0/2.1 A
//     and AA rules. Those are the ones that mean "a person cannot use this", which is the bar a gate can hold;
//   * and it **prints** the `moderate`/`minor` findings with their counts, so "we know" is a fact in the log
//     rather than a claim in a document.
//
// It runs axe *inside* the app's own frame, against the app's own document: axe's colour-contrast rule needs
// the real computed styles, and a cross-frame run cannot see them.
import { spawn } from 'node:child_process';
import { existsSync, readFileSync, statSync } from 'node:fs';
import { createServer } from 'node:http';
import { dirname, extname, join, normalize } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const dist = join(here, '..', 'dist');
const axe = join(here, '..', 'node_modules', 'axe-core', 'axe.min.js');
const settleMs = Number(process.env.SDC_A11Y_SETTLE_MS ?? 9000);

if (!existsSync(join(dist, 'index.html'))) {
  console.error(`a11y: no build at ${dist} - run pnpm build first`);
  process.exit(2);
}

if (!existsSync(axe)) {
  console.error(`a11y: no axe-core at ${axe} - run pnpm install`);
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

/* The harness, in the browser. Same single rule as the smoke harness: this is a template literal, so no
   backtick and no `${` of its own - write class names bare. */
const harness = `<!doctype html>
<meta charset="utf-8">
<title>SDC accessibility audit</title>
<style>html,body{margin:0;background:#111}iframe{width:1280px;height:900px;border:0}
pre{color:#eee;font:12px monospace}</style>
<pre id="a11y-report">pending</pre>
<script>
  const frame = document.createElement('iframe');

  frame.src = '/';
  document.body.append(frame);

  frame.addEventListener('load', () => {
    const page = frame.contentDocument;
    const script = page.createElement('script');

    script.src = '/axe.min.js';
    script.onload = () => run(page);
    page.head.append(script);
  });

  async function run(page) {
    const api = frame.contentWindow.axe;
    let report;

    try {
      if (!api) {
        throw new Error('axe did not load into the frame');
      }

      /* The app mounts asynchronously and reads its own state; the audit waits for the shell rather than for
         a timer, so a slow mount is a slow audit and not a false pass. */
      for (let waited = 0; waited < 6000 && !page.querySelector('#app'); waited += 200) {
        await new Promise((resolve) => setTimeout(resolve, 200));
      }

      const results = await api.run(page, {
        resultTypes: ['violations'],
        runOnly: { type: 'tag', values: ['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa'] },
      });

      report = {
        mounted: Boolean(page.querySelector('#app')),
        violations: results.violations.map((violation) => ({
          impact: violation.impact,
          id: violation.id,
          help: violation.help,
          nodes: violation.nodes.length,
          where: violation.nodes.slice(0, 3).map((node) => String(node.target).slice(0, 120)),
          /* Why it failed, not just where: axe's own sentence carries the measured ratio and the two colours
             for a contrast failure, which is the difference between a number to fix and a number to guess. */
          why: violation.nodes.slice(0, 3).map((node) =>
            [...(node.any ?? []), ...(node.all ?? [])]
              .map((check) => String(check.message))
              .join(' ')
              .slice(0, 200),
          ),
          /* And the markup itself, because the aria-allowed-attr and nested-interactive sentences do not say
             which attribute or which child. No backticks here: this whole harness is a template literal. */
          html: violation.nodes.slice(0, 3).map((node) => String(node.html ?? '').slice(0, 160)),
        })),
      };
    } catch (error) {
      report = { error: String(error && error.message ? error.message : error) };
    }

    report.axe = Boolean(frame.contentWindow.axe);
    document.querySelector('#a11y-report').textContent = JSON.stringify(report);
  }
</script>`;

function browser() {
  const names =
    process.platform === 'win32'
      ? ['msedge.exe', 'chrome.exe', 'chromium.exe']
      : ['google-chrome', 'google-chrome-stable', 'chromium', 'chromium-browser', 'microsoft-edge'];

  const directories = (process.env.PATH ?? '')
    .split(process.platform === 'win32' ? ';' : ':')
    .filter(Boolean);

  const candidates = [
    process.env.SDC_BROWSER,
    process.env.CHROME_PATH,
    ...directories.flatMap((directory) => names.map((name) => join(directory, name))),
    'C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe',
    'C:/Program Files/Google/Chrome/Application/chrome.exe',
    '/usr/bin/google-chrome',
    '/usr/bin/chromium',
    '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
  ].filter(Boolean);

  return candidates.find((candidate) => existsSync(candidate)) ?? null;
}

const server = createServer((request, response) => {
  const url = new URL(request.url ?? '/', 'http://127.0.0.1');

  if (url.pathname === '/__a11y.html') {
    response.writeHead(200, { 'content-type': 'text/html; charset=utf-8' });

    return void response.end(harness);
  }

  if (url.pathname === '/axe.min.js') {
    response.writeHead(200, { 'content-type': 'text/javascript; charset=utf-8' });

    return void response.end(readFileSync(axe));
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
  if (process.env.CI) {
    console.error('a11y: no Chromium-family browser found in CI (set SDC_BROWSER)');
    server.close();
    process.exit(2);
  }

  console.log('a11y: no Chromium-family browser found - skipped (set SDC_BROWSER to check locally)');
  server.close();
  process.exit(0);
}

console.log(`a11y: serving ${dist} on ${port}, auditing with ${executable}`);

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
    `--virtual-time-budget=${settleMs}`,
    `http://127.0.0.1:${port}/__a11y.html`,
  ],
  { stdio: ['ignore', 'pipe', 'ignore'] },
);

let dom = '';

child.stdout.on('data', (chunk) => {
  dom += chunk.toString();
});

const left = new Promise((resolve) => child.on('exit', resolve));
const tooLong = new Promise((resolve) => setTimeout(() => resolve(null), 90_000));

await Promise.race([left, tooLong]);

child.kill();
server.close();

const match = /<pre id="a11y-report"[^>]*>([\s\S]*?)<\/pre>/.exec(dom);
const text = (match?.[1] ?? '')
  .replace(/&quot;/g, '"')
  .replace(/&#39;/g, "'")
  .replace(/&lt;/g, '<')
  .replace(/&gt;/g, '>')
  .replace(/&amp;/g, '&')
  .trim();

let report = null;

try {
  report = text === 'pending' || text === '' ? null : JSON.parse(text);
} catch {
  report = null;
}

if (report === null) {
  console.error('a11y: the browser never reported back');
  console.error(dom.slice(0, 500) || '(no output from the browser)');
  process.exit(1);
}

if (report.error) {
  console.error(`a11y: ${report.error}`);
  process.exit(1);
}

const violations = report.violations ?? [];
const blocking = violations.filter(
  (violation) => violation.impact === 'critical' || violation.impact === 'serious',
);

for (const violation of violations) {
  console.log(
    `${blocking.includes(violation) ? 'FAIL' : 'note'} ${violation.id} (${violation.impact}) x${violation.nodes} - ${violation.help}`,
  );

  for (const where of violation.where ?? []) {
    console.log(`      at ${where}`);
  }

  for (const why of violation.why ?? []) {
    if (why !== '') {
      console.log(`      because: ${why}`);
    }
  }

  for (const html of violation.html ?? []) {
    if (html !== '') {
      console.log(`      html: ${html}`);
    }
  }
}

for (const name of ['moderate', 'minor']) {
  const found = violations.filter((violation) => violation.impact === name).length;

  if (found > 0) {
    console.log(`note ${found} ${name} rule(s) - reported here, not blocking`);
  }
}

console.log(
  `a11y: ${violations.length} violation(s), ${blocking.length} serious or critical, ` +
    `axe loaded: ${report.axe === true}, app mounted: ${report.mounted === true}`,
);

if (blocking.length > 0) {
  process.exit(1);
}

console.log('a11y: no serious or critical violation on the screen the app opens on');
