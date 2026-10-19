/**
 * Diff Review and Select & Instruct (P3-9, P3-11): the pure part.
 *
 * `git.diff` answers a patch as git wrote it. Review needs it by file and by hunk so a person can point at one change - this reads the
 * patch's own markers (`diff --git`, `@@`) and nothing else: no line is re-derived, so a rename or a binary file git described is
 * shown the way git described it. The text of every hunk is kept exactly, so what is quoted back to the agent is what git printed.
 */

export interface Hunk {
  /** The `@@ -a,b +c,d @@ ...` line. */
  header: string;
  /** The hunk exactly as git printed it, header included. */
  text: string;
  added: number;
  removed: number;
}

export interface FileDiff {
  path: string;
  /** Everything before the first hunk (index, mode, rename and binary lines). */
  meta: string;
  hunks: Hunk[];
}

/** The path a `diff --git a/x b/x` line names (the new side). */
function pathOf(line: string): string {
  const match = / b\/(.+)$/.exec(line);

  return match === null ? line.replace(/^diff --git /, '') : match[1];
}

export function parseDiff(patch: string): FileDiff[] {
  const files: FileDiff[] = [];
  let file: FileDiff | null = null;
  let hunk: Hunk | null = null;
  let meta: string[] = [];
  let body: string[] = [];

  const closeHunk = (): void => {
    if (file !== null && hunk !== null) {
      hunk.text = body.join('\n');
      file.hunks.push(hunk);
    }

    hunk = null;
    body = [];
  };

  for (const line of patch.split('\n')) {
    if (line.startsWith('diff --git ')) {
      closeHunk();

      if (file !== null) {
        file.meta = meta.join('\n');
      }

      file = { path: pathOf(line), meta: '', hunks: [] };
      files.push(file);
      meta = [line];
    } else if (line.startsWith('@@') && file !== null) {
      closeHunk();
      hunk = { header: line, text: '', added: 0, removed: 0 };
      body = [line];
    } else if (hunk !== null) {
      body.push(line);

      if (line.startsWith('+') && !line.startsWith('+++')) {
        hunk.added += 1;
      } else if (line.startsWith('-') && !line.startsWith('---')) {
        hunk.removed += 1;
      }
    } else if (file !== null) {
      meta.push(line);
    }
  }

  closeHunk();

  if (file !== null) {
    (file as FileDiff).meta = meta.join('\n');
  }

  /* the trailing newline of a patch is not part of its last hunk */
  for (const each of files) {
    for (const part of each.hunks) {
      part.text = part.text.replace(/\n+$/, '');
    }
  }

  return files;
}

/** Hunk text longer than this is cut when it is put into the prompt. */
export const MAX_QUOTE = 4000;

function cut(text: string): string {
  return text.length <= MAX_QUOTE ? text : `${text.slice(0, MAX_QUOTE)}\n… (cut)`;
}

/** The prompt that points the agent at one hunk. */
export function hunkDraft(path: string, hunk: Hunk, ask: string): string {
  return `${ask}\n\nIn ${path}:\n\`\`\`diff\n${cut(hunk.text)}\n\`\`\`\n`;
}

/** The prompt that points the agent at selected text in a file; `line` is the first selected line when it is known. */
export function selectionDraft(path: string, selected: string, ask: string, line?: number): string {
  const text = selected.replace(/\s+$/, '');
  const where = line === undefined ? path : `${path}:${line}`;

  return `${ask}\n\n@${where}\n\`\`\`\n${cut(text)}\n\`\`\`\n`;
}
