// Visual regression (P0-7): the built window, photographed, compared with what was approved.
//
//   pnpm --filter @sdc/app smoke:visual              # compare with tests/visual/*.png
//   UPDATE_VISUAL=1 pnpm --filter @sdc/app smoke:visual   # approve what the window looks like now
//
// Same approach as `a11y-bundle.mjs`, for the same reason: serve the built `dist`, open it in a real
// Chromium-family browser, and need no driver and no extra dependency. Each *view* is one browser size and
// one colour scheme - desktop and phone width, dark and light - of the screen the app opens on.
//
// What this is and is not. It catches an unintended change of how the opening screen looks at four sizes.
// It does not walk the app: a screen that needs a daemon, a project or a connected provider is not on this
// path, and a change there needs its own view added to `views` below when it gets a way to be reached
// without one. A difference is measured, not eyeballed: the share of pixels that differ by more than a few
// levels in any channel, against a tolerance that absorbs anti-aliasing and the one animation the opening
// screen has (measured on this machine: two runs of one build differ by about 0.01% of pixels).
import { spawn } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { createServer } from 'node:http';
import { tmpdir } from 'node:os';
import { dirname, extname, join, normalize } from 'node:path';
import { fileURLToPath } from 'node:url';
import { inflateSync } from 'node:zlib';

import { findBrowser, noBrowser } from './browser.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const dist = join(here, '..', 'dist');
const baselines = join(here, '..', 'tests', 'visual');
const update = Boolean(process.env.UPDATE_VISUAL);
/* A pixel counts as different past this many levels in a channel, and a view fails past this share. */
const LEVEL = 24;
const SHARE = 0.005;

const views = [
  { name: 'desktop-dark', width: 1280, height: 800, scheme: 'dark' },
  { name: 'desktop-light', width: 1280, height: 800, scheme: 'light' },
  { name: 'phone-dark', width: 390, height: 844, scheme: 'dark' },
  { name: 'phone-light', width: 390, height: 844, scheme: 'light' },
];

const types = { '.html': 'text/html; charset=utf-8', '.js': 'text/javascript; charset=utf-8', '.css': 'text/css; charset=utf-8', '.json': 'application/json; charset=utf-8', '.svg': 'image/svg+xml', '.woff': 'font/woff', '.woff2': 'font/woff2', '.png': 'image/png', '.ico': 'image/x-icon' };

/** A PNG as `{width, height, rgba}` - 8-bit, non-interlaced, which is what Chromium writes. */
export function decodePng(buffer) {
  let at = 8;
  let width = 0;
  let height = 0;
  let channels = 4;
  const data = [];

  while (at < buffer.length) {
    const size = buffer.readUInt32BE(at);
    const kind = buffer.toString('ascii', at + 4, at + 8);
    const body = buffer.subarray(at + 8, at + 8 + size);

    if (kind === 'IHDR') {
      width = body.readUInt32BE(0);
      height = body.readUInt32BE(4);
      const depth = body[8];
      const color = body[9];

      if (depth !== 8 || body[12] !== 0 || (color !== 6 && color !== 2)) {
        throw new Error(`unsupported PNG (depth ${depth}, colour ${color}, interlace ${body[12]})`);
      }

      channels = color === 6 ? 4 : 3;
    } else if (kind === 'IDAT') {
      data.push(body);
    }

    at += 12 + size;
  }

  const raw = inflateSync(Buffer.concat(data));
  const stride = width * channels;
  const out = Buffer.alloc(width * height * 4, 255);
  let previous = Buffer.alloc(stride);

  for (let row = 0; row < height; row += 1) {
    const filter = raw[row * (stride + 1)];
    const line = Buffer.from(raw.subarray(row * (stride + 1) + 1, (row + 1) * (stride + 1)));

    for (let i = 0; i < stride; i += 1) {
      const left = i >= channels ? line[i - channels] : 0;
      const up = previous[i];
      const upLeft = i >= channels ? previous[i - channels] : 0;
      let add = 0;

      if (filter === 1) add = left;
      else if (filter === 2) add = up;
      else if (filter === 3) add = (left + up) >> 1;
      else if (filter === 4) {
        const p = left + up - upLeft;
        const pa = Math.abs(p - left);
        const pb = Math.abs(p - up);
        const pc = Math.abs(p - upLeft);

        add = pa <= pb && pa <= pc ? left : pb <= pc ? up : upLeft;
      }

      line[i] = (line[i] + add) & 255;
    }

    for (let x = 0; x < width; x += 1) {
      for (let c = 0; c < channels; c += 1) {
        out[(row * width + x) * 4 + c] = line[x * channels + c];
      }
    }

    previous = line;
  }

  return { width, height, rgba: out };
}

/**
 * The toast strip: a browser tab has no daemon, so the app says so in a toast that appears and goes by the clock, and
 * which one a screenshot catches depends on how fast the machine is. It sits bottom-centre, fixed, up to ~110px tall
 * and 44px off the bottom; the rows it can occupy are not compared.
 */
const TOAST_FROM = 150;
const TOAST_TO = 30;

/** The share of pixels that differ by more than `level` in any channel (the toast strip excluded); `1` when the sizes differ. */
export function difference(a, b, level = LEVEL) {
  if (a.width !== b.width || a.height !== b.height) {
    return 1;
  }

  let different = 0;
  let counted = 0;

  for (let i = 0; i < a.rgba.length; i += 4) {
    const row = Math.floor(i / 4 / a.width);

    if (row >= a.height - TOAST_FROM && row < a.height - TOAST_TO) {
      continue;
    }

    counted += 1;

    if (Math.abs(a.rgba[i] - b.rgba[i]) > level || Math.abs(a.rgba[i + 1] - b.rgba[i + 1]) > level || Math.abs(a.rgba[i + 2] - b.rgba[i + 2]) > level) {
      different += 1;
    }
  }

  return different / counted;
}

function serve() {
  return new Promise((resolve) => {
    const server = createServer((request, response) => {
      const path = normalize(decodeURIComponent((request.url ?? '/').split('?')[0])).replace(/^(\.\.[/\\])+/, '');
      let file = join(dist, path);

      if (!existsSync(file) || statSync(file).isDirectory()) {
        file = join(dist, 'index.html');
      }

      response.writeHead(200, { 'content-type': types[extname(file)] ?? 'application/octet-stream' });
      response.end(readFileSync(file));
    });

    server.listen(0, '127.0.0.1', () => resolve(server));
  });
}

function shoot(browser, url, view, out) {
  return new Promise((resolve) => {
    const child = spawn(
      browser,
      [
        '--headless=new',
        '--disable-gpu',
        '--hide-scrollbars',
        '--force-device-scale-factor=1',
        `--window-size=${view.width},${view.height}`,
        `--blink-settings=preferredColorScheme=${view.scheme === 'dark' ? 0 : 1}`,
        '--virtual-time-budget=9000',
        `--screenshot=${out}`,
        url,
      ],
      { stdio: 'ignore' },
    );

    child.on('exit', resolve);
    child.on('error', () => resolve(1));
  });
}

async function main() {
  if (!existsSync(join(dist, 'index.html'))) {
    console.error(`visual: no build at ${dist} - run pnpm build first`);

    return 2;
  }

  const browser = findBrowser();

  if (!browser) {
    return noBrowser('visual');
  }

  mkdirSync(baselines, { recursive: true });

  const server = await serve();
  const url = `http://127.0.0.1:${server.address().port}/`;
  const scratch = join(tmpdir(), `sdc-visual-${process.pid}`);
  let failed = 0;

  mkdirSync(scratch, { recursive: true });

  for (const view of views) {
    const taken = join(scratch, `${view.name}.png`);
    const approved = join(baselines, `${view.name}.png`);

    await shoot(browser, url, view, taken);

    if (!existsSync(taken)) {
      console.error(`visual: ${view.name}: the browser wrote no screenshot`);
      failed += 1;
      continue;
    }

    if (update || !existsSync(approved)) {
      writeFileSync(approved, readFileSync(taken));
      console.log(`visual: ${view.name}: ${update ? 'approved' : 'no baseline - wrote one'} (${view.width}x${view.height}, ${view.scheme})`);
      continue;
    }

    const share = difference(decodePng(readFileSync(approved)), decodePng(readFileSync(taken)));
    const ok = share <= SHARE;

    console.log(`visual: ${view.name}: ${ok ? 'ok' : 'CHANGED'} - ${(share * 100).toFixed(3)}% of pixels differ (limit ${(SHARE * 100).toFixed(1)}%)`);

    if (!ok) {
      writeFileSync(join(baselines, `${view.name}.new.png`), readFileSync(taken));
      failed += 1;
    }
  }

  server.close();

  if (failed) {
    console.error(`visual: ${failed} view(s) changed - if intended, run with UPDATE_VISUAL=1 and review the diff of tests/visual/`);
  }

  return failed ? 1 : 0;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  process.exit(await main());
}
