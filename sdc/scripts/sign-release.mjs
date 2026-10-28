#!/usr/bin/env node
/**
 * Signs a published release's update files and uploads the `latest.json` the app's updater reads (1.0.0).
 *
 *   node sdc/scripts/sign-release.mjs v1.0.0
 *
 * The release workflow signs these itself when the repository holds TAURI_SIGNING_PRIVATE_KEY. Without that secret
 * the installers are still built and published, and this script - run on the machine that holds the key - does the
 * rest: it downloads each platform's update file, signs it with `tauri signer sign`, and uploads one `latest.json`
 * naming every platform and kind of install (setup.exe, .msi, .app, .AppImage, .deb, .rpm).
 *
 * The key is read from SDC_UPDATER_KEY (default `%LOCALAPPDATA%/sdc-signing/sdc-updater.key`) and its password from
 * SDC_UPDATER_PASSWORD_FILE (default: `sdc-updater.password.txt` beside it). The GitHub token comes from GITHUB_TOKEN,
 * else from `git credential fill`.
 */
import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

const REPO = 'skilleddesk/SD-Code';
const tag = process.argv[2];

if (!tag) {
  console.error('usage: node sdc/scripts/sign-release.mjs <tag>');
  process.exit(2);
}

const signingDir = join(process.env.LOCALAPPDATA ?? join(process.env.HOME ?? '.', '.local', 'share'), 'sdc-signing');
const keyFile = process.env.SDC_UPDATER_KEY ?? join(signingDir, 'sdc-updater.key');
const passwordFile = process.env.SDC_UPDATER_PASSWORD_FILE ?? join(signingDir, 'sdc-updater.password.txt');
const token =
  process.env.GITHUB_TOKEN ??
  execFileSync('git', ['credential', 'fill'], { input: 'protocol=https\nhost=github.com\n\n' })
    .toString()
    .split('\n')
    .find((line) => line.startsWith('password='))
    ?.slice('password='.length);
const headers = { Authorization: `Bearer ${token}`, Accept: 'application/vnd.github+json', 'User-Agent': 'sdc-sign-release' };

/** The update file of each updater target, matched against the release's asset names. */
const TARGETS = [
  ['windows-x86_64-nsis', /_x64-setup\.exe$/],
  ['windows-x86_64-msi', /_x64_en-US\.msi$/],
  ['darwin-x86_64-app', /_x64\.app\.tar\.gz$/],
  ['darwin-aarch64-app', /_aarch64\.app\.tar\.gz$/],
  ['linux-x86_64-appimage', /\.AppImage$/],
  ['linux-x86_64-deb', /_amd64\.deb$/],
  ['linux-x86_64-rpm', /\.x86_64\.rpm$/],
];
/** The plain `{os}-{arch}` key an updater falls back to when it does not know its kind of install. */
const FALLBACK = { 'windows-x86_64': 'windows-x86_64-nsis', 'darwin-x86_64': 'darwin-x86_64-app', 'darwin-aarch64': 'darwin-aarch64-app', 'linux-x86_64': 'linux-x86_64-appimage' };

async function api(path, init = {}) {
  const response = await fetch(path.startsWith('http') ? path : `https://api.github.com/repos/${REPO}${path}`, { ...init, headers: { ...headers, ...init.headers } });

  if (!response.ok && response.status !== 204) {
    throw new Error(`${init.method ?? 'GET'} ${path}: HTTP ${response.status} ${await response.text()}`);
  }

  return response.status === 204 ? null : response.json();
}

const release = await api(`/releases/tags/${tag}`);
const version = tag.replace(/^v/, '');
const work = join(tmpdir(), `sdc-sign-${version}`);
const platforms = {};

mkdirSync(work, { recursive: true });

for (const [target, pattern] of TARGETS) {
  const asset = release.assets.find((candidate) => pattern.test(candidate.name));

  if (asset === undefined) {
    console.warn(`no asset for ${target} - that kind of install will not update itself in this release`);
    continue;
  }

  const file = join(work, asset.name);

  if (!existsSync(file) || readFileSync(file).length !== asset.size) {
    console.log(`downloading ${asset.name}`);
    const response = await fetch(asset.url, { headers: { ...headers, Accept: 'application/octet-stream' } });

    writeFileSync(file, Buffer.from(await response.arrayBuffer()));
  }

  execFileSync(
    process.platform === 'win32' ? 'pnpm.cmd' : 'pnpm',
    ['exec', 'tauri', 'signer', 'sign', '-f', keyFile, '-p', readFileSync(passwordFile, 'utf8').trim(), file],
    { cwd: resolve(import.meta.dirname, '..', 'app'), stdio: ['ignore', 'ignore', 'inherit'], shell: process.platform === 'win32' },
  );
  platforms[target] = { signature: readFileSync(`${file}.sig`, 'utf8').trim(), url: asset.browser_download_url };
  console.log(`signed ${target}`);
}

for (const [plain, specific] of Object.entries(FALLBACK)) {
  if (platforms[specific] !== undefined) {
    platforms[plain] = platforms[specific];
  }
}

const manifest = { version, notes: `SDC ${version}`, pub_date: new Date().toISOString(), platforms };
const manifestFile = join(work, 'latest.json');

writeFileSync(manifestFile, JSON.stringify(manifest, null, 2));

for (const old of release.assets.filter((asset) => asset.name === 'latest.json')) {
  await api(`/releases/assets/${old.id}`, { method: 'DELETE' });
}

await api(`https://uploads.github.com/repos/${REPO}/releases/${release.id}/assets?name=latest.json`, {
  method: 'POST',
  headers: { 'Content-Type': 'application/json' },
  body: readFileSync(manifestFile),
});

console.log(`latest.json uploaded with ${Object.keys(platforms).length} targets`);
