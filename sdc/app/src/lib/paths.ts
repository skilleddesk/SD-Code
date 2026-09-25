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

/** The last part of a path, whichever separator it uses. */
export function baseName(path: string): string {
  return path.split(/[\\/]/).pop() ?? path;
}
