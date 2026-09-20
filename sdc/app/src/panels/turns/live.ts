import { strings } from '../../strings';
import type { TurnView } from '../../store/types';
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
export function toTurns(turns: readonly TurnView[], sessionId: string): Turn[] {
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
          : { duration: strings.turns.thinking.duration, text: turn.thinking },
      /* The answer, which `TurnView.text` has held all along. Left out while it is empty, so a turn
         that has not produced a word yet shows no empty block. */
      answer:
        turn.text === ''
          ? undefined
          : { text: turn.text, streaming: turn.status === 'running' },
      tools: turn.tools.map(toToolCard),
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

/** One tool call, in the card variant that matches what it did. */
function toToolCard(tool: TurnView['tools'][number]): ToolCardData {
  const base = { name: tool.name, target: tool.target, status: tool.status, meta: tool.meta };

  if (tool.tool === 'edit') {
    return { kind: 'edit', ...base, diff: tool.diff };
  }

  if (tool.tool === 'run') {
    return { kind: 'run', ...base, output: tool.output };
  }

  return { kind: 'read', ...base };
}
