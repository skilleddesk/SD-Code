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

/** What is never walked: a build that has to look at every file in node_modules is slower than the build itself. */
const SKIP = new Set(['node_modules', '.git']);

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
process.stdout.write(`apple-double: ${removed} stray \`._\` file(s) removed\n`);
