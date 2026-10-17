// Removes the `._name` files macOS writes beside every file.
//
// This project lives on a volume macOS does not treat as native, so it writes an AppleDouble sidecar - a copy of the
// file's extended attributes - next to each one. They are not source, but they are files: the Tauri build script reads
// every file in its generated permissions folder as UTF-8 and stops on them, Vite's dependency scanner trips over them,
// and `find`/`grep` in CI see them as source. None of that is visible in the diff, so they are cleared before a build
// rather than after a failure.
//
// `COPYFILE_DISABLE=1` is what stops new ones: it is set for the build in `package.json`, and this script handles what
// is already there.

import { readdir, rm, stat } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

/** What is never walked: a Rust `target` tree is a hundred thousand files, and walking it here cost nine minutes on this
 * external volume - the build looks hung to whoever is watching it. What Tauri's build script does need is
 * `ALSO` below: the generated permissions folder is inside it, and one `._default.toml` there fails the whole build
 * ("stream did not contain valid UTF-8"). */
const SKIP = new Set(['node_modules', '.git', 'target']);

/** The one place inside a build tree that still has to be walked - small, and the only one Tauri reads as text. */
const ALSO = ['app/src-tauri/target/release/build', 'app/src-tauri/target/debug/build'];

let removed = 0;

async function strip(directory, depth) {
  /* A build tree is deep - `target/release/build/<crate>/out/...` - so this is generous. It is only a guard against a
   * symlink loop that walks the whole volume. */
  if (depth > 14) return;

  const entries = await readdir(directory, { withFileTypes: true }).catch(() => []);

  await Promise.all(
    entries.map(async (entry) => {
      const path = join(directory, entry.name);

      if (entry.name.startsWith('._')) {
        const removed_file = await rm(path, { recursive: true, force: true }).then(() => true).catch(() => false);

        if (removed_file) removed += 1;

        return;
      }

      if (SKIP.has(entry.name)) return;

      if (entry.isDirectory()) return strip(path, depth + 1);

      /* Only files: a symlink to a folder is left alone, because walking it may leave the project. */
      if (entry.isSymbolicLink() && (await stat(path).catch(() => null))?.isDirectory()) return;
    }),
  );
}

const root = join(dirname(fileURLToPath(import.meta.url)), '..', '..');

await strip(root, 0);

/* And the one build folder Tauri reads as text (§`ALSO`). `COPYFILE_DISABLE=1` in `package.json` stops the ones this
 * build would write itself, so what is left is only what an earlier build left behind. Only the Tauri crate's generated
 * permissions are walked: the rest of a build tree is never read as text, and walking it is what took minutes. */
for (const inside of ALSO) {
  let crates = [];
  try {
    crates = await readdir(join(root, inside));
  } catch {
    continue;
  }

  for (const crate of crates.filter((name) => name.startsWith('tauri-'))) {
    await strip(join(root, inside, crate, 'out', 'permissions'), 0);
  }
}

process.stdout.write(`apple-double: ${removed} stray \`._\` file(s) removed\n`);
