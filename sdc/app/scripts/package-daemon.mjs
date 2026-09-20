// Packages the host daemon into what the app bundles (master spec section 7.4).
//
// `pnpm tauri:build` runs this first, because the installer has to carry `sdcd`: the app's bridge
// starts the daemon itself (see `src-tauri/src/sdcp.rs`), so an install that has no daemon would
// open a window where every call fails.
//
// Tauri's `externalBin` needs the file named with the target triple, which is why the copy is
// `sdcd-<triple>[.exe]` and not `sdcd`: the bundler picks the file for the platform it is building
// for, and the name is the only way it can tell them apart.
import { execFileSync } from 'node:child_process';
import { copyFileSync, existsSync, mkdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const workspace = join(here, '..', '..'); // sdc/
const daemon = join(workspace, 'sdcd');
const windows = process.platform === 'win32';
const binary = windows ? 'sdcd.exe' : 'sdcd';

/** The triple Tauri expects: it is what `rustc -vV` reports as `host`. */
function hostTriple() {
  const output = execFileSync('rustc', ['-vV'], { encoding: 'utf8' });
  const line = output.split('\n').find((candidate) => candidate.startsWith('host:'));

  if (!line) {
    throw new Error('rustc -vV did not report a host triple');
  }

  return line.replace('host:', '').trim();
}

function cargo(args) {
  try {
    execFileSync('cargo', args, { cwd: daemon, stdio: 'inherit' });
  } catch (error) {
    /* rustup installs cargo in ~/.cargo/bin, which a shell that predates the install may not have on
       PATH - the same reason `SETUP.md` tells a fresh machine to reopen its terminal. */
    const fallback = join(process.env.USERPROFILE ?? process.env.HOME ?? '', '.cargo', 'bin', windows ? 'cargo.exe' : 'cargo');

    if (!existsSync(fallback)) {
      throw error;
    }

    execFileSync(fallback, args, { cwd: daemon, stdio: 'inherit' });
  }
}

const triple = hostTriple();

console.log(`sdcd: building release for ${triple}`);

cargo(['build', '--release']);

const built = join(daemon, 'target', 'release', binary);

if (!existsSync(built)) {
  throw new Error(`the daemon build produced no ${built}`);
}

const target = join(workspace, 'app', 'src-tauri', 'binaries', `sdcd-${triple}${windows ? '.exe' : ''}`);

mkdirSync(dirname(target), { recursive: true });
copyFileSync(built, target);

console.log(`sdcd: bundled → ${target}`);
