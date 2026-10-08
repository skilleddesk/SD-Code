import { Sparkles, Target } from 'lucide-react';

import { strings } from '../../strings';
import { Markdown } from './Markdown';
import { splitUnderstood, useSmoothText } from './smooth';
import type { AnswerData } from './types';

/**
 * `.answer` - what the engine said (spec section 7.5, and the block 0.7.3 added).
 *
 * The stream had every other part of a turn and not this one: the question, the meta line, the thinking
 * block, the tool cards, the error card and the totals - so a finished turn read
 *
 *     You · Reply with exactly: OK
 *     Balanced · claude_code · sonnet
 *     Done · $0.0295 · 1.6s · 2 in · 4 out
 *
 * with nothing between the third and the fourth line. The engine *had* answered; the text was thrown
 * away between the log and the screen, and the report was *"sudu done lakha aslo, kono response pelam
 * nah"*.
 *
 * Two decisions worth naming:
 *
 *   - **`whitespace-pre-wrap`.** A CLI answer is written text with its own line breaks and indented
 *     code, and the app has no markdown renderer. Re-flowing it would silently reflow code blocks, so
 *     it is drawn as it arrived, wrapped at the container's edge and selectable.
 *   - **A caret only while it streams.** `▍` is the difference between "still typing" and "that was the
 *     whole answer", which the footer's `Running · totals arrive with the last event` cannot say on its
 *     own for an engine that streams slowly.
 */
export interface AnswerBlockProps {
  answer: AnswerData;
}

export function AnswerBlock({ answer }: AnswerBlockProps) {
  /* 0.11.8: revealed evenly rather than in 50 ms lumps, and the `Understood:` line drawn as its own card. */
  const shown = useSmoothText(answer.text, answer.streaming);
  const { understood, rest } = splitUnderstood(shown, answer.streaming);

  return (
    <div
      className="answer mb-[10px] mt-[2px] text-[13.5px]"
      data-answer={answer.streaming ? 'streaming' : 'done'}
    >
      {/* 0.18: the answer is the conversation, so it reads as prose - no card, no label - under the work
          that produced it. The label stays for screen readers. */}
      <div className="answer-head sr-only">
        <Sparkles size={12} aria-hidden="true" />
        <span>{strings.turns.answer.title}</span>
        {answer.streaming ? <span>{strings.turns.answer.streaming}</span> : null}
      </div>

      {understood === null ? null : (
        <div
          className="understood mb-[10px] flex items-start gap-[8px] rounded-md border border-accent/25 bg-accent-subtle px-[10px] py-[7px]"
          data-understood
        >
          <Target size={13} aria-hidden="true" className="mt-[3px] shrink-0 text-accent" />
          <div className="min-w-0 text-[12.5px] leading-[1.55] text-text-primary">
            <span className="mr-[6px] text-[10px] font-semibold uppercase tracking-[.08em] text-accent">{strings.turns.understood}</span>
            {understood}
          </div>
        </div>
      )}

      <div className="answer-body" aria-live="polite">
        <Markdown text={rest} />
        {answer.streaming ? <span className="answer-caret ml-[1px] animate-pulse text-accent motion-reduce:animate-none">▍</span> : null}
      </div>
    </div>
  );
}
