import { strings } from '../../strings';
import type { CheckpointView, TurnView, VerifyView } from '../../store/types';
import type { CollapsedSummaryData, LiveBarData, TimelineItem, ToolCardData, Turn, TurnCheckpointData } from './types';
import { OPEN_TURN_WINDOW } from './types';

/**
 * The live turn stream: the daemon's events, in the shape the stream draws.
 *
 * This file is the bridge that did not exist. `reducer.ts` has projected `TurnView`s from the event
 * log all along - `TurnStarted`, `ThinkingDelta`, `ToolCallCompleted`, `TurnCompleted`, `ErrorRaised`
 * - and `Pane.tsx` drew a hardcoded demo turn instead, so a real turn could run to completion behind
 * a window that showed the same fiction every time. Everything below is a pure function of the log;
 * nothing here invents a number, and a field the daemon did not report is left out rather than
 * estimated.
 */

/** The turns of one session, oldest first, as the stream wants them. */
export function toTurns(
  turns: readonly TurnView[],
  sessionId: string,
  checkpoints: readonly CheckpointView[] = [],
  verifies: readonly VerifyView[] = [],
): Turn[] {
  return turns
    .filter((turn) => turn.sessionId === sessionId)
    .map((turn) => ({
      id: turn.id,
      startedAt: turn.startedAt,
      ...(turn.endedAt === undefined ? {} : { endedAt: turn.endedAt }),
      user: {
        who: strings.turns.who,
        body: turn.prompt,
        /* Attachments are not in the log yet: the prompt area cannot attach one, so claiming a chip
           here would be the same kind of decoration this change is removing. */
        attachments: [],
        ...(turn.reading === undefined ? {} : { reading: { label: turn.reading.label, reply: turn.reading.reply } }),
      },
      meta: {
        tier: turn.tier,
        engine: turn.engine,
        model: turn.model,
        /* No forecast: a price for a turn nobody has measured is not a fact, and `TurnCompleted`
           carries the real totals when the run ends. */
        forecast: '',
      },
      thinking:
        turn.thinking === ''
          ? undefined
          : {
              text: turn.thinking,
              ms: turn.thinkingMs,
              since: turn.thinkingSince,
              /* Live while the turn runs and the thinking is still the newest thing it produced. */
              live:
                (turn.status === 'running' || turn.status === 'stuck') &&
                turn.thinkingSince !== null,
            },
      /* The answer, which `TurnView.text` has held all along. Left out while it is empty, so a turn
         that has not produced a word yet shows no empty block. */
      answer:
        turn.text === ''
          ? undefined
          : { text: turn.text, streaming: turn.status === 'running' },
      tools: turn.tools.map(toToolCard),
      timeline: timelineOf(turn, checkpoints),
      ...(turn.status === 'running' || turn.status === 'stuck' ? { live: liveBarOf(turn) } : {}),
      ...((turn.status === 'running' || turn.status === 'stuck') && turn.draft !== undefined ? { draft: { ...turn.draft } } : {}),
      plan: turn.plan.map((step) => ({ text: step.text, status: step.status })),
      running: turn.status === 'running' || turn.status === 'stuck',
      /* Running with nothing on screen yet: the pulse that stands in for the answer until the first
         event arrives, so a slow first token never reads as a dead turn (0.10.0). */
      waiting:
        turn.status === 'running' &&
        turn.text === '' &&
        turn.thinking === '' &&
        turn.tools.length === 0 &&
        turn.plan.length === 0 &&
        turn.draft === undefined,
      stats:
        turn.status === 'running' || turn.status === 'stuck'
          ? {
              startedAt: turn.startedAt,
              /* What a file being written adds counts too (0.14.2): the pace read ~1 tok/s while it wrote 6k characters. */
              chars: turn.text.length + turn.thinking.length + (turn.draft?.chars ?? 0),
              thinkingMs: turn.thinkingMs,
              thinkingSince: turn.thinkingSince,
              tools: turn.tools.length,
            }
          : undefined,
      author: { engine: turn.engine, model: turn.model },
      ...verifyOf(verifies, turn.id),
      checkpoints: checkpoints
        .filter((checkpoint) => checkpoint.turnId === turn.id)
        .sort((left, right) => left.turn - right.turn)
        .map((checkpoint) => ({ id: checkpoint.id, title: checkpoint.title, turn: checkpoint.turn })),
      error:
        turn.error === undefined
          ? undefined
          : { title: turn.error.title, explanation: turn.error.explanation },
      footer:
        turn.status === 'failed'
          ? /* An `ErrorRaised` turn never reaches `TurnCompleted`, so its totals can never arrive.
               Saying `Failed` is what happened; `Running · totals arrive…` would be a promise the
               log has no way to keep. */
            { summary: strings.turns.footer.failed, detail: '' }
          : { summary: turn.summary, detail: turn.meta },
    }));
}

/** `Turns 1–6 collapsed · …`, or null while the session is short enough to show whole. */
export function collapsedSummary(
  turns: readonly TurnView[],
  sessionId: string,
): CollapsedSummaryData | null {
  const count = turns.filter((turn) => turn.sessionId === sessionId).length;

  if (count <= OPEN_TURN_WINDOW) {
    return null;
  }

  return {
    label: strings.turns.collapsed.labelFor(count - OPEN_TURN_WINDOW),
    /* A summary line that carried a token count and a price would have to make them up: the daemon
       reports what a turn used on the turn itself. */
    meta: strings.turns.collapsed.windowMeta(OPEN_TURN_WINDOW),
  };
}

/** The newest verify run of a turn, as the footer chip needs it - or nothing. */
function verifyOf(verifies: readonly VerifyView[], turnId: string): { verify?: Turn['verify'] } {
  const run = [...verifies].reverse().find((candidate) => candidate.turnId === turnId);

  if (run === undefined) {
    return {};
  }

  return {
    verify: {
      state: run.state,
      pass: run.pass,
      reviewer: run.review === null ? '' : run.review.model === '' ? run.review.engine : run.review.model,
      issues: run.review?.issues?.length ?? 0,
    },
  };
}

/**
 * The turn's stretches in the order they happened (0.12.5), resolved to what each one draws.
 *
 * A log written before 0.12.5 has no timeline; it is rebuilt from the totals in the old order
 * (thinking, tools, answer) so an old chat still reads.
 */
function timelineOf(turn: TurnView, checkpoints: readonly CheckpointView[]): TimelineItem[] {
  const running = turn.status === 'running' || turn.status === 'stuck';
  const entries: TurnView['timeline'] =
    (turn.timeline ?? []).length > 0 || (turn.text === '' && turn.thinking === '' && turn.tools.length === 0)
      ? (turn.timeline ?? [])
      : [
          ...(turn.thinking === '' ? [] : [{ kind: 'thinking' as const, text: turn.thinking, startedAt: turn.startedAt, endedAt: turn.startedAt }]),
          ...turn.tools.map((tool) => ({ kind: 'tool' as const, callId: tool.callId })),
          ...(turn.text === '' ? [] : [{ kind: 'text' as const, text: turn.text, startedAt: turn.startedAt }]),
        ];
  const lastText = entries.map((entry) => entry.kind).lastIndexOf('text');
  const lastTool = entries.map((entry) => entry.kind).lastIndexOf('tool');
  const items: TimelineItem[] = [];

  entries.forEach((entry, index) => {
    const key = `${entry.kind}-${index}`;
    const newest = index === entries.length - 1;

    if (entry.kind === 'thinking') {
      const live = running && newest && entry.endedAt === null;

      items.push({
        kind: 'thinking',
        key,
        startedAt: entry.startedAt,
        endedAt: entry.endedAt,
        thinking: {
          text: entry.text,
          ms: entry.endedAt === null ? 0 : Math.max(0, Date.parse(entry.endedAt) - Date.parse(entry.startedAt)) || 0,
          since: entry.endedAt === null && running ? entry.startedAt : null,
          live,
        },
      });
    } else if (entry.kind === 'text') {
      items.push({
        kind: 'text',
        key,
        text: entry.text,
        startedAt: entry.startedAt,
        /* A tool call being written (0.14.2) comes after these words: they are finished, not still typing. */
        streaming: running && newest && turn.draft === undefined,
        /* The words after the last tool call, once the turn has ended, are its answer. */
        final: !running && index === lastText && index > lastTool,
      });
    } else if (entry.kind === 'tool') {
      const tool = turn.tools.find((candidate) => candidate.callId === entry.callId);

      if (tool !== undefined) {
        items.push({ kind: 'tool', key: `tool-${tool.callId}`, tool: toToolCard(tool) });
      }
    } else if (entry.kind === 'steer') {
      items.push({ kind: 'steer', key, text: entry.text });
    } else {
      const checkpoint = checkpoints.find((candidate) => candidate.id === entry.id);

      if (checkpoint !== undefined) {
        const data: TurnCheckpointData = { id: checkpoint.id, title: checkpoint.title, turn: checkpoint.turn };

        items.push({ kind: 'checkpoint', key: `cp-${checkpoint.id}`, checkpoint: data });
      }
    }
  });

  return items;
}

/** The sticky bar's line for a running turn (0.12.5): what is happening this second, and since when. */
function liveBarOf(turn: TurnView): LiveBarData {
  const steps = turn.plan;
  const current = steps.findIndex((step) => step.status === 'in_progress');
  const index = current >= 0 ? current : steps.findIndex((step) => step.status === 'pending');
  const step = index >= 0 ? { text: steps[index]?.text ?? '', index: index + 1, total: steps.length } : null;
  const timeline = turn.timeline ?? [];
  const last = timeline[timeline.length - 1];
  const running = turn.tools.find((tool) => tool.status === 'running');

  if (running !== undefined) {
    return { phase: 'tool', detail: `${running.name} ${running.target}`.trim(), step, since: running.startedAt };
  }

  /* A tool call still being written (0.14.2): the bar says which, instead of "deciding" for half a minute. */
  if (turn.draft !== undefined) {
    const name = turn.draft.target.split(/[\\/]/).pop() ?? '';

    return { phase: 'drafting', detail: name, tool: turn.draft.name, step, since: turn.draft.since };
  }

  if (last === undefined) {
    return { phase: 'waiting', detail: '', step, since: turn.startedAt };
  }

  if (last.kind === 'thinking' && last.endedAt === null) {
    const lines = last.text.trim().split(/\r?\n/).filter((line) => line.trim() !== '');

    return { phase: 'thinking', detail: lines[lines.length - 1]?.trim() ?? '', step, since: last.startedAt };
  }

  if (last.kind === 'text') {
    return { phase: 'writing', detail: '', step, since: last.startedAt };
  }

  /* A finished tool, a checkpoint or an ended thought: the model is choosing what to do next - the
     gap that used to look like nothing was happening. */
  const endedTool = [...turn.tools].reverse().find((tool) => tool.endedAt !== undefined);
  const since = last.kind === 'thinking' && last.endedAt !== null ? last.endedAt : (endedTool?.endedAt ?? turn.startedAt);

  return { phase: 'deciding', detail: '', step, since };
}

/** One tool call, in the card variant that matches what it did. */
function toToolCard(tool: TurnView['tools'][number]): ToolCardData {
  const base = {
    name: tool.name,
    startedAt: tool.startedAt,
    target: tool.target,
    status: tool.status,
    meta: tool.meta,
    ...(tool.endedAt === undefined ? {} : { endedAt: tool.endedAt }),
  };

  if (tool.tool === 'edit') {
    return { kind: 'edit', ...base, diff: tool.diff };
  }

  if (tool.tool === 'run') {
    return { kind: 'run', ...base, output: tool.output };
  }

  return { kind: 'read', ...base, output: tool.output };
}
