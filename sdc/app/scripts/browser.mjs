// Where a Chromium-family browser is on this machine, for the two scripts that need one.
//
// `smoke-bundle.mjs` and `a11y-bundle.mjs` both open the built window in a real browser, and they had **two
// copies of this list** - which is exactly how 0.7.10's macOS release job failed: the smoke script knew about
// `/Applications/Microsoft Edge.app` and the a11y script did not, so on a runner with Edge and no Chrome the
// window rendered and the audit could not start. One list, in one file.
//
// No `--version` probe: on Windows `msedge.exe --version` *opens Edge* rather than printing and exiting, so
// asking a browser binary what it is hangs the build. Looking for the file is enough.
import { existsSync } from 'node:fs';
import { join } from 'node:path';

const MAC = '/Applications';
const WINDOWS = 'C:\\Program Files';

/** The browser to run, or `null` when this machine has none. */
export function findBrowser() {
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
    /* The usual places, for a shell whose PATH is not the desktop's - and both macOS bundles, because a
       runner may have Edge and no Chrome. */
    ...directories.flatMap((directory) => names.map((name) => join(directory, name))),
    `${WINDOWS} (x86)\\Microsoft\\Edge\\Application\\msedge.exe`,
    `${WINDOWS}\\Microsoft\\Edge\\Application\\msedge.exe`,
    `${WINDOWS}\\Google\\Chrome\\Application\\chrome.exe`,
    `${MAC}/Google Chrome.app/Contents/MacOS/Google Chrome`,
    `${MAC}/Microsoft Edge.app/Contents/MacOS/Microsoft Edge`,
    '/usr/bin/google-chrome',
    '/usr/bin/chromium',
  ].filter(Boolean);

  return candidates.find((candidate) => existsSync(candidate)) ?? null;
}

/** What to say when none was found: a developer is not blocked, a release is. */
export function noBrowser(what) {
  if (process.env.CI) {
    console.error(`${what}: no Chromium-family browser found in CI (set SDC_BROWSER)`);

    return 2;
  }

  console.log(`${what}: no Chromium-family browser found - skipped (set SDC_BROWSER to check locally)`);

  return 0;
}

/**
 * GitHub reads `::error::` lines from a step's stdout and turns them into **annotations**, which are the one
 * part of a failing run that can be read from the API without being the repository's owner. 0.7.10's macOS
 * failure was found this way - the log was unreadable, the annotation named the command that exited 2.
 */
export function annotate(title, message) {
  console.error(`::error title=${title}::${message}`);
}
