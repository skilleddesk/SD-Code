import type { ReadToolData, TimelineItem } from './types';

/**
 * How a turn's timeline is drawn tidily (0.13) - the pure half, kept out of the components so they stay
 * components: reads gathered into one "Explored" line, and the files a turn changed.
 */

/** Reads that fold into one "Explored" line when two or more come in a row. */
const EXPLORING = new Set(['Read', 'List', 'Grep', 'Glob', 'Search', 'Search web', 'Fetch', 'View', 'Diff', 'Output']);

export type Drawn = TimelineItem | { kind: 'explore'; key: string; tools: ReadToolData[] };

/** The timeline with each run of consecutive reads gathered into one item. */
export function gather(items: readonly TimelineItem[]): Drawn[] {
  const drawn: Drawn[] = [];
  let run: { key: string; tools: ReadToolData[] } | null = null;

  const close = (): void => {
    if (run === null) {
      return;
    }

    if (run.tools.length >= 2) {
      drawn.push({ kind: 'explore', key: run.key, tools: run.tools });
    } else {
      for (const tool of run.tools) {
        drawn.push({ kind: 'tool', key: `${run.key}-single`, tool });
      }
    }

    run = null;
  };

  for (const item of items) {
    if (item.kind === 'tool' && item.tool.kind === 'read' && EXPLORING.has(item.tool.name)) {
      run ??= { key: item.key, tools: [] };
      run.tools.push(item.tool);
      continue;
    }

    close();
    drawn.push(item);
  }

  close();

  return drawn;
}

/** `Grep add · Read math.js, test.js +1 · List .` - each kind once, in order, its targets by file name. */
export function summary(tools: readonly ReadToolData[]): string {
  const order: string[] = [];
  const targets = new Map<string, string[]>();

  for (const tool of tools) {
    if (!targets.has(tool.name)) {
      order.push(tool.name);
      targets.set(tool.name, []);
    }

    const short = tool.name === 'Read' || tool.name === 'View' ? (tool.target.split(/[\\/]/).pop() ?? tool.target) : tool.target;

    targets.get(tool.name)?.push(short);
  }

  return order
    .map((name) => {
      const list = targets.get(name) ?? [];
      const shown = list.slice(0, 3).join(', ');

      return `${name} ${shown}${list.length > 3 ? ` +${list.length - 3}` : ''}`;
    })
    .join(' · ');
}

/** One file the turn changed, with what it added and removed. */
export interface ChangedFile {
  path: string;
  added: number;
  removed: number;
}

/**
 * The files a turn changed, once each: `+8 −4` read from the Edit card's pill when it says so, counted
 * from its diff otherwise. A file written twice is one row with both changes summed.
 */
export function changedFiles(items: readonly TimelineItem[]): ChangedFile[] {
  const files = new Map<string, ChangedFile>();

  for (const item of items) {
    if (item.kind !== 'tool' || item.tool.kind !== 'edit' || item.tool.status !== 'done' || item.tool.target === '' || item.tool.name === 'Remember') {
      continue;
    }

    const tool = item.tool;
    const pill = /\+(\d+)\s*[−-]\s*(\d+)/.exec(tool.meta);
    const added = pill !== null ? Number(pill[1]) : tool.diff.filter((line) => line.change === 'add').length;
    const removed = pill !== null ? Number(pill[2]) : tool.diff.filter((line) => line.change === 'rem').length;
    const known = files.get(tool.target);

    files.set(tool.target, { path: tool.target, added: (known?.added ?? 0) + added, removed: (known?.removed ?? 0) + removed });
  }

  return [...files.values()];
}
