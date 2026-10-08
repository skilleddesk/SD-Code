import { changedFiles, gather, summary, type Drawn } from './grouping';
import type { TimelineItem } from './types';

/**
 * A turn's work as timed steps (0.15) - the pure half of the two new stream styles, Flow and Console.
 *
 * The classic stream draws each stretch as its own card and says nothing about *when*. Both new styles
 * need the same three facts about every stretch: what it was (one line), how it went, and when it
 * started and ended - so they can draw a rail with durations, a log with offsets, and the time ribbon
 * that shows where a turn's minutes went. Every time here is a log stamp; a stretch the log did not
 * close (the words before the next action) ends where the next one starts.
 */

export type StepKind = 'think' | 'say' | 'answer' | 'read' | 'explore' | 'edit' | 'run' | 'agent' | 'question' | 'checkpoint' | 'steer';

/** The ribbon's colours: what kind of time it was. */
export type Phase = 'think' | 'write' | 'read' | 'edit' | 'run' | 'wait';

export interface Step {
  key: string;
  kind: StepKind;
  /** One line: `Read App.tsx`, the thought's headline, the first words said. */
  title: string;
  /** The quieter half of the line: a path, a pill, `+8 −2`. */
  detail: string;
  status: 'running' | 'done' | 'failed';
  /** Epoch ms, or null for an instant with no stamp (a checkpoint, a steer). */
  start: number | null;
  end: number | null;
  /** What the classic stream would have drawn - the step's expanded body. */
  drawn: Drawn;
}

const stamp = (value: string | null | undefined): number | null => {
  if (value === null || value === undefined) {
    return null;
  }

  const parsed = Date.parse(value);

  return Number.isFinite(parsed) ? parsed : null;
};

/**
 * The headline of a thought: the bold title Claude and GPT summaries open with (`**Inspecting the
 * router**`), a markdown heading, or the first sentence - cut at 90 characters.
 */
export function headline(text: string): string {
  const trimmed = text.trim();
  const bold = /^\*\*(.+?)\*\*/.exec(trimmed) ?? /^#{1,4}\s+(.+)$/m.exec(trimmed.split('\n')[0] ?? '');

  if (bold?.[1] !== undefined) {
    return bold[1].trim();
  }

  const first = trimmed.split(/\r?\n/).find((line) => line.trim() !== '')?.trim() ?? '';
  const sentence = /^(.+?[.!?।])(\s|$)/.exec(first)?.[1] ?? first;

  return sentence.length > 90 ? `${sentence.slice(0, 89)}…` : sentence;
}

/** The last few lines of a thought still going on, newest last - what a live node shows. */
export function tail(text: string, lines = 3): string[] {
  return text
    .trim()
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line !== '')
    .slice(-lines);
}

const short = (path: string): string => path.split(/[\\/]/).pop() ?? path;

function toolKind(name: string, kind: 'read' | 'edit' | 'run'): StepKind {
  if (name === 'Agent') {
    return 'agent';
  }

  if (name === 'Question') {
    return 'question';
  }

  return kind;
}

/**
 * The timeline as steps, in order, with times. `now` closes the step still going on; `endedAt` (the
 * turn's own end) closes the last one of a finished turn.
 */
export function steps(items: readonly TimelineItem[], now: number, endedAt: string | undefined, running: boolean, group = true): Step[] {
  /* `group: false` (0.19) keeps every read as its own step - the transcript shows what each one saw. */
  const drawn: Drawn[] = group ? gather(items) : items.map((item) => item);
  const out: Step[] = drawn.map((item): Step => {
    switch (item.kind) {
      case 'thinking':
        return {
          key: item.key,
          kind: 'think',
          title: headline(item.thinking.text),
          detail: '',
          status: item.thinking.live ? 'running' : 'done',
          start: stamp(item.startedAt ?? item.thinking.since),
          end: stamp(item.endedAt ?? null),
          drawn: item,
        };
      case 'text':
        return {
          key: item.key,
          kind: item.final ? 'answer' : 'say',
          title: item.text.trim().split(/\r?\n/).find((line) => line.trim() !== '')?.trim() ?? '',
          detail: '',
          status: item.streaming ? 'running' : 'done',
          start: stamp(item.startedAt),
          end: null,
          drawn: item,
        };
      case 'tool': {
        const tool = item.tool;
        const kind = toolKind(tool.name, tool.kind);
        const added = tool.kind === 'edit' ? tool.diff.filter((line) => line.change === 'add').length : 0;
        const removed = tool.kind === 'edit' ? tool.diff.filter((line) => line.change === 'rem').length : 0;
        /* The engine's pill often says it already (`done · +31`); the count is added only when it does not. */
        const pill = tool.kind === 'edit' && /[+−-]\d/.test(tool.meta) ? '' :tool.kind === 'edit' && added + removed > 0 ? ` +${added} −${removed}` : '';

        return {
          key: item.key,
          kind,
          title: kind === 'question' ? tool.target : `${tool.name} ${tool.kind === 'run' || tool.name === 'Grep' || tool.name === 'Glob' ? tool.target : short(tool.target)}`.trim(),
          detail: `${tool.meta}${pill}`.trim(),
          status: tool.status,
          start: stamp(tool.startedAt),
          end: stamp(tool.endedAt),
          drawn: item,
        };
      }
      case 'explore': {
        const ends = item.tools.map((tool) => stamp(tool.endedAt)).filter((value): value is number => value !== null);

        return {
          key: item.key,
          kind: 'explore',
          title: summary(item.tools),
          detail: `${item.tools.length}`,
          status: item.tools.some((tool) => tool.status === 'running') ? 'running' : item.tools.some((tool) => tool.status === 'failed') ? 'failed' : 'done',
          start: stamp(item.tools[0]?.startedAt),
          end: ends.length === item.tools.length && ends.length > 0 ? Math.max(...ends) : null,
          drawn: item,
        };
      }
      case 'checkpoint':
        return { key: item.key, kind: 'checkpoint', title: item.checkpoint.title, detail: '', status: 'done', start: null, end: null, drawn: item };
      case 'steer':
        return { key: item.key, kind: 'steer', title: item.text, detail: '', status: 'done', start: null, end: null, drawn: item };
    }
  });

  /* A stretch the log did not close ends where the next timed one starts - or now, or at the turn's end. */
  const close = running ? now : (stamp(endedAt) ?? null);

  out.forEach((step, index) => {
    if (step.start === null || step.end !== null) {
      return;
    }

    const next = out.slice(index + 1).find((candidate) => candidate.start !== null)?.start ?? null;
    const end = next ?? (step.status === 'running' || index === out.length - 1 ? close : null);

    step.end = end === null ? null : Math.max(step.start, end);
  });

  return out;
}

export function phaseOf(kind: StepKind): Phase | null {
  switch (kind) {
    case 'think':
      return 'think';
    case 'say':
    case 'answer':
      return 'write';
    case 'read':
    case 'explore':
    case 'agent':
      return 'read';
    case 'edit':
      return 'edit';
    case 'run':
      return 'run';
    case 'question':
      return 'wait';
    default:
      return null;
  }
}

export interface Segment {
  phase: Phase;
  /** Share of the turn, 0..1, and where it starts. */
  left: number;
  width: number;
  ms: number;
  /** The step it belongs to; empty for the gaps between steps (the model choosing, the network). */
  key: string;
  label: string;
}

/**
 * Where the turn's time went, as the ribbon draws it: one segment per timed step, and grey `wait`
 * segments for the gaps between them - the seconds nobody's UI shows, which are often most of a turn.
 */
export function ribbon(list: readonly Step[], startedAt: string, now: number, endedAt: string | undefined, running: boolean): { segments: Segment[]; totals: Record<Phase, number>; total: number } {
  const begin = stamp(startedAt) ?? now;
  const finish = running ? now : (stamp(endedAt) ?? Math.max(begin, ...list.map((step) => step.end ?? 0)));
  const total = Math.max(1, finish - begin);
  const totals: Record<Phase, number> = { think: 0, write: 0, read: 0, edit: 0, run: 0, wait: 0 };
  const segments: Segment[] = [];
  let cursor = begin;

  const push = (phase: Phase, from: number, to: number, key: string, label: string): void => {
    const ms = Math.max(0, to - from);

    if (ms <= 0) {
      return;
    }

    totals[phase] += ms;
    segments.push({ phase, left: (from - begin) / total, width: ms / total, ms, key, label });
  };

  const timed = list
    .filter((step) => step.start !== null && phaseOf(step.kind) !== null)
    .sort((left, right) => (left.start ?? 0) - (right.start ?? 0));

  for (const step of timed) {
    const from = Math.max(cursor, step.start ?? cursor);
    const to = Math.min(finish, step.end ?? (step.status === 'running' ? finish : from));

    if (from > cursor) {
      push('wait', cursor, from, '', '');
    }

    if (to > from) {
      push(phaseOf(step.kind) ?? 'wait', from, to, step.key, step.title);
      cursor = to;
    }
  }

  if (finish > cursor) {
    push('wait', cursor, finish, '', '');
  }

  return { segments, totals, total };
}

/** Counts per filter, and what each filter keeps - the Console's tabs. */
export type Filter = 'all' | 'think' | 'read' | 'edit' | 'run' | 'say';

export function matches(step: Step, filter: Filter): boolean {
  switch (filter) {
    case 'all':
      return true;
    case 'think':
      return step.kind === 'think';
    case 'read':
      return step.kind === 'read' || step.kind === 'explore' || step.kind === 'agent';
    case 'edit':
      return step.kind === 'edit' || step.kind === 'checkpoint';
    case 'run':
      return step.kind === 'run';
    case 'say':
      return step.kind === 'say' || step.kind === 'answer' || step.kind === 'steer' || step.kind === 'question';
  }
}

/** The turn's change so far, summed - live while it runs, not only at the end. */
export function changeTotals(items: readonly TimelineItem[]): { files: number; added: number; removed: number } {
  const files = changedFiles(items);

  return {
    files: files.length,
    added: files.reduce((sum, file) => sum + file.added, 0),
    removed: files.reduce((sum, file) => sum + file.removed, 0),
  };
}

/** `+00:12`, `+1:04`, `+12:30` - a step's offset from the turn's start. */
export function offset(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const minutes = Math.floor(total / 60);

  return `+${minutes < 10 ? `0${minutes}` : minutes}:${String(total % 60).padStart(2, '0')}`;
}

/** A plain-text transcript of the steps - the Console's "Copy log". */
export function transcript(list: readonly Step[], startedAt: string): string {
  const begin = stamp(startedAt) ?? 0;

  return list
    .map((step) => {
      const at = step.start === null ? '      ' : offset(step.start - begin);
      const took = step.start !== null && step.end !== null ? ` (${((step.end - step.start) / 1000).toFixed(1)}s)` : '';
      const mark = step.status === 'failed' ? '✗' : step.status === 'running' ? '…' : '✓';

      return `${at}  ${mark} [${step.kind}] ${step.title}${step.detail === '' ? '' : ` · ${step.detail}`}${took}`;
    })
    .join('\n');
}
