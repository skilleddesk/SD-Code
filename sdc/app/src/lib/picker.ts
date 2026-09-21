/**
 * The GUI file picker - the paperclip, for real.
 *
 * The report was one line: *"GUI file-picker nai"*. Both buttons in the prompt box were `toast(...)`
 * calls - the paperclip promised `Attach a file` and attached nothing - while `tauri-plugin-dialog`
 * was already a dependency, already registered in `src-tauri/src/lib.rs`, and already allowed by
 * `capabilities/default.json` (`dialog:default`). Nothing was asking it for a path.
 *
 * A picked path is what an engine can act on, so the caller puts it in the prompt as `@<path>` - the
 * file reference Claude Code, Codex and Cline all read. A browser tab has no filesystem to point at,
 * so the fallback picks names through an `<input type="file">` and returns them as `path === name`;
 * `pnpm dev` in a browser therefore still opens *a* picker, and the desktop build is where the
 * reference is a real path. Saying which of the two happened is the caller's job, and it has the
 * `path === name` fact to say it with.
 */

/** One picked entry. */
export interface PickedFile {
  /** The absolute path on the desktop; the bare name in a browser. */
  path: string;
  /** The last segment - for a chip's title, not for the prompt. */
  name: string;
}

export type PickKind = 'file' | 'image' | 'folder';

/** What the dialog accepts for `image`: Tauri's filter shape, lower-case without the dot. */
const IMAGE_EXTENSIONS = ['png', 'jpg', 'jpeg', 'gif', 'webp', 'bmp', 'svg'];

/**
 * Is this the desktop shell? Same test as `lib/sdcp.ts` uses, and for the same reason: the global is
 * injected before any script runs, and a plain browser has no native dialog to fall back on.
 */
function desktop(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

/** The last segment of a path, whichever separator it uses. `C:\a\b.ts` and `/a/b.ts` both give `b.ts`. */
export function nameOf(path: string): string {
  const parts = path.split(/[\\/]/);

  return parts[parts.length - 1] ?? path;
}

/**
 * Strings, from whichever shape the dialog answered with: `open()` returns a `string` for one pick, an
 * array for many, and `null` for a cancel. All three are ordinary, so all three are handled here.
 */
export function pathsOf(selected: string | string[] | null | undefined): string[] {
  if (selected === null || selected === undefined) {
    return [];
  }

  return Array.isArray(selected) ? selected : [selected];
}

/** Opens the picker and answers with what was chosen - empty for a cancel. */
export async function pickFiles(kind: PickKind): Promise<PickedFile[]> {
  const chosen = desktop() ? await pickWithDialog(kind) : await pickInBrowser(kind);

  return chosen.map((path) => ({ path, name: nameOf(path) }));
}

/** Opens the picker for one **folder** and answers with its path, or `null` for a cancel. */
export async function pickFolder(): Promise<string | null> {
  const [picked] = await pickFiles('folder');

  return picked === undefined ? null : picked.path;
}

/**
 * The desktop dialog.
 *
 * The plugin is imported *here* rather than at the top of the file, because importing it is a desktop
 * act: a browser tab has no business loading it, and a unit test (which runs in node, with no window)
 * still gets to import this module for `nameOf` and `pathsOf`.
 */
async function pickWithDialog(kind: PickKind): Promise<string[]> {
  const { open } = await import('@tauri-apps/plugin-dialog');
  const selected = await open({
    multiple: kind !== 'folder',
    directory: kind === 'folder',
    ...(kind === 'image' ? { filters: [{ name: 'Images', extensions: IMAGE_EXTENSIONS }] } : {}),
  });

  return pathsOf(selected as string | string[] | null);
}

/**
 * A browser tab's picker: an `<input type="file">`, which is all a page may open by itself. The
 * browser hides the path, so the name is what comes back - and `path === name` is how the caller can
 * tell that this reference points at nothing the daemon could read.
 */
function pickInBrowser(kind: PickKind): Promise<string[]> {
  return new Promise((resolve) => {
    const input = document.createElement('input');

    input.type = 'file';
    input.multiple = kind !== 'folder';
    input.style.display = 'none';

    if (kind === 'image') {
      input.accept = IMAGE_EXTENSIONS.map((extension) => `.${extension}`).join(',');
    }

    if (kind === 'folder') {
      input.setAttribute('webkitdirectory', '');
    }

    document.body.append(input);

    const finish = (names: string[]): void => {
      input.remove();
      resolve(names);
    };

    input.addEventListener('change', () => {
      finish([...(input.files ?? [])].map((file) => file.name));
    });

    /* A cancelled dialog fires `cancel` where it is implemented and nothing at all where it is not.
       The second case leaves this promise pending, which holds nothing and leaks nothing: the input
       element is in the DOM only until the page reloads, and no caller waits on it for its own sake. */
    input.addEventListener('cancel', () => finish([]));

    input.click();
  });
}
