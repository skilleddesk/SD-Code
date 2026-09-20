import { Sparkles } from 'lucide-react';

import { strings } from '../../strings';
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
  return (
    <div
      className="answer mb-[8px] rounded-md border border-border-subtle border-l-2 border-l-accent bg-bg-raised px-[13px] py-[10px]"
      data-answer={answer.streaming ? 'streaming' : 'done'}
    >
      <div className="answer-head mb-[6px] flex items-center gap-[8px] text-[10px] font-semibold uppercase tracking-[.08em] text-text-muted">
        <Sparkles size={12} aria-hidden="true" />
        <span>{strings.turns.answer.title}</span>
        {answer.streaming ? <span className="font-normal normal-case tracking-normal">{strings.turns.answer.streaming}</span> : null}
      </div>

      <div className="answer-body whitespace-pre-wrap text-[13px] leading-[1.6] text-text-primary">
        {answer.text}
        {answer.streaming ? <span className="answer-caret ml-[1px] animate-pulse text-accent">▍</span> : null}
      </div>
    </div>
  );
}
