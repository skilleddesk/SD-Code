/**
 * A path a tool or a model named, inside the chat's folder.
 *
 * Reviewers, compilers and consoles name files relative to the project (`src/pay.js`); the daemon's
 * `fs.read` wants the path the folder's own machine spells. An absolute path (`/srv/app/x`, `C:\x`) is
 * kept as it is; a relative one is joined to the folder with the folder's own separator, so a Windows
 * folder gets backslashes and a host's folder gets slashes.
 */
export function inFolder(root: string, file: string): string {
  if (/^([a-zA-Z]:[\\/]|\/)/.test(file)) {
    return file;
  }

  const separator = root.includes('\\') ? '\\' : '/';
  const trimmed = root.replace(/[\\/]+$/, '');

  return `${trimmed}${separator}${file.replace(/[\\/]/g, separator)}`;
}

/**
 * One spelling of a path, for comparing two of them.
 *
 * On Windows the same file arrives as `C:/p/src\a.ts` from one call and `C:/p/src/a.ts` from another
 * (the daemon joins with `\`, a folder picked in the window uses `/`), and a drive's letter and names are
 * not case-sensitive - so a tab and a tree row for the same file compared unequal, and a rename left the
 * old tab open. Paths on a host (they start with `/`) keep their case: POSIX names are case-sensitive.
 */
export function pathKey(path: string): string {
  const slashed = path.replace(/\\/g, '/').replace(/\/+$/, '');

  return /^[a-zA-Z]:\//.test(slashed) ? slashed.toLowerCase() : slashed;
}

/** Whether two paths name the same file. */
export function samePath(left: string, right: string): boolean {
  return pathKey(left) === pathKey(right);
}

/** Whether `path` is `folder` itself or anything inside it. */
export function isUnder(path: string, folder: string): boolean {
  const inner = pathKey(path);
  const outer = pathKey(folder);

  return inner === outer || inner.startsWith(`${outer}/`);
}

/** The last part of a path, whichever separator it uses. */
export function baseName(path: string): string {
  return path.split(/[\\/]/).pop() ?? path;
}
