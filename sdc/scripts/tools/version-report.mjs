/**
 * Where the version is written down, and what every place says right now.
 *
 *   node sdc/scripts/tools/version-report.mjs
 *
 * The version is not in one file. Five files carry it, two lockfiles repeat it, and the artifacts
 * (the daemon binary, the window, the installer) each print it from a different one of those - so a
 * bump that misses a file ships a build whose app, daemon and installer disagree about what they are.
 * `sdc/scripts/tools/bump-version.mjs` writes the five; `cargo` rewrites the two lockfiles on the next build.
 *
 * This prints all of them plus the built artifacts, and exits 1 if the five disagree, so a half-bump
 * is a failed command rather than a surprise on a user's machine.
 */
import { execFileSync } from 'node:child_process';
import { existsSync, readFileSync, readdirSync } from 'node:fs';

const root = new URL('../../../', import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, '$1');
const at = (path) => `${root}${path}`;

/** The five files `bump-version.mjs` writes, with the pattern that finds the version in each. */
const sources = [
  { file: 'sdc/package.json', pattern: /"version": "([^"]+)"/ },
  { file: 'sdc/app/package.json', pattern: /"version": "([^"]+)"/ },
  { file: 'sdc/app/src-tauri/tauri.conf.json', pattern: /"version": "([^"]+)"/ },
  { file: 'sdc/sdcd/Cargo.toml', pattern: /^version = "([^"]+)"/m },
  { file: 'sdc/app/src-tauri/Cargo.toml', pattern: /^version = "([^"]+)"/m },
];

const found = [];

console.log('written down:');

for (const source of sources) {
  const match = readFileSync(at(source.file), 'utf8').match(source.pattern);

  found.push(match?.[1] ?? '?');
  console.log(`  ${match?.[1] ?? 'NOT FOUND'.padEnd(9)}  ${source.file}`);
}

/* The lockfiles: `cargo` keeps these in step, so a mismatch here just means "not built yet". */
console.log('\nlockfiles (cargo rewrites these when it builds):');

for (const [lock, crate] of [
  ['sdc/sdcd/Cargo.lock', 'sdcd'],
  ['sdc/app/src-tauri/Cargo.lock', 'sdc'],
]) {
  const text = readFileSync(at(lock), 'utf8');
  const entry = new RegExp(`name = "${crate}"\\nversion = "([^"]+)"`).exec(text);

  console.log(`  ${entry?.[1] ?? '?'}  ${lock}`);
}

/* The artifacts, when they exist. */
console.log('\nbuilt:');

for (const binary of ['sdc/sdcd/target/release/sdcd.exe', 'sdc/sdcd/target/debug/sdcd.exe']) {
  if (!existsSync(at(binary))) {
    console.log(`  not built          ${binary}`);
    continue;
  }

  const printed = execFileSync(at(binary), ['--version'], { encoding: 'utf8' }).trim();

  console.log(`  ${printed}  <- ${binary}`);
}

const window = 'sdc/app/src-tauri/target/release/sdc.exe';

if (existsSync(at(window))) {
  const info = execFileSync(
    'powershell',
    [
      '-NoLogo',
      '-NoProfile',
      '-NonInteractive',
      '-Command',
      `(Get-Item '${at(window)}').VersionInfo | Select-Object -ExpandProperty FileVersion`,
    ],
    { encoding: 'utf8' },
  ).trim();

  console.log(`  ${info}  <- the window's own VersionInfo`);
}

const bundles = 'sdc/app/src-tauri/target/release/bundle';

if (existsSync(at(bundles))) {
  for (const kind of ['nsis', 'msi']) {
    const directory = `${bundles}/${kind}`;

    if (!existsSync(at(directory))) {
      continue;
    }

    for (const name of readdirSync(at(directory))) {
      console.log(`  ${name}  <- installer`);
    }
  }
}

/* And the tag a release would be cut from. */
const tags = execFileSync('git', ['tag', '--list', 'v*'], { encoding: 'utf8', cwd: root })
  .split('\n')
  .filter((tag) => tag !== '')
  .sort();

console.log(`\nlatest tag: ${tags.at(-1) ?? 'none'}`);

/* The agreement check, which is the point of the whole file. */
const versions = [...new Set(found)];

if (versions.length === 1) {
  console.log(`\nversion: ${versions[0]} - all five files agree`);
  process.exit(0);
}

console.log(`\nversion: ${versions.join(' / ')} - THE FIVE FILES DISAGREE`);
console.log('run: node sdc/scripts/tools/bump-version.mjs <major.minor.patch>');
process.exit(1);
