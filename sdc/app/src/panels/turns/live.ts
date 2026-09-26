import { strings } from '../../strings';
import type { CheckpointView, TurnView, VerifyView } from '../../store/types';
import type { CollapsedSummaryData, ToolCardData, Turn } from './types';
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
      user: {
        who: strings.turns.who,
        body: turn.prompt,
        /* Attachments are not in the log yet: the prompt area cannot attach one, so claiming a chip
           here would be the same kind of decoration this change is removing. */
        attachments: [],
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
      plan: turn.plan.map((step) => ({ text: step.text, status: step.status })),
      running: turn.status === 'running' || turn.status === 'stuck',
      /* Running with nothing on screen yet: the pulse that stands in for the answer until the first
         event arrives, so a slow first token never reads as a dead turn (0.10.0). */
      waiting:
        turn.status === 'running' &&
        turn.text === '' &&
        turn.thinking === '' &&
        turn.tools.length === 0 &&
        turn.plan.length === 0,
      stats:
        turn.status === 'running' || turn.status === 'stuck'
          ? {
              startedAt: turn.startedAt,
              chars: turn.text.length + turn.thinking.length,
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

/** One tool call, in the card variant that matches what it did. */
function toToolCard(tool: TurnView['tools'][number]): ToolCardData {
  const base = {
    name: tool.name,
    startedAt: tool.startedAt,
    target: tool.target,
    status: tool.status,
    meta: tool.meta,
  };

  if (tool.tool === 'edit') {
    return { kind: 'edit', ...base, diff: tool.diff };
  }

  if (tool.tool === 'run') {
    return { kind: 'run', ...base, output: tool.output };
  }

  return { kind: 'read', ...base };
}
