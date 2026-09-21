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

const triple = process.env.SDC_TARGET_TRIPLE?.trim() || hostTriple();
const cross = process.env.SDC_TARGET_TRIPLE?.trim() ? ['--target', triple] : [];

/* 0.7.10: the shipped daemon gets the OS keychain where there is one. `keyring` v3 enables no backend by
   default, so the feature alone would compile everywhere and store nothing - Cargo.toml picks
   `windows-native` (DPAPI) and `apple-native` (Keychain) per platform, and the packaged daemon is built
   with the feature on those two. Linux stays on the documented file fallback: the Secret Service needs
   `dbus` headers and a session bus, and a daemon that cannot start on a headless box is worse than one
   that reports `file`. A store that is compiled in but unreachable (a locked keychain, a service account)
   is still handled at runtime - `keychain::backend()` probes it and falls back. */
const keychain = process.platform === 'win32' || process.platform === 'darwin';
const features = keychain ? ['--features', 'keychain'] : [];

console.log(`sdcd: building release for ${triple} (key store: ${keychain ? 'os' : 'file'})`);

cargo(['build', '--release', ...cross, ...features]);

const built = cross.length > 0
  ? join(daemon, 'target', triple, 'release', binary)
  : join(daemon, 'target', 'release', binary);

if (!existsSync(built)) {
  throw new Error(`the daemon build produced no ${built}`);
}

const target = join(workspace, 'app', 'src-tauri', 'binaries', `sdcd-${triple}${windows ? '.exe' : ''}`);

mkdirSync(dirname(target), { recursive: true });
copyFileSync(built, target);

console.log(`sdcd: bundled → ${target}`);
