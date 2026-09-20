import { ThumbsDown, ThumbsUp } from 'lucide-react';
import { useState } from 'react';

import { strings } from '../../strings';
import { toast } from '../../store/toast';
import type { TurnFooterData } from './types';

/**
 * `.turn-footer` - what the turn cost, and what you thought of it (spec section 7.5, 19.4).
 *
 * A hairline above, then `Done · 1m 12s · 12,400 tokens · $0.16` on the left and the two feedback
 * buttons on the right. The verb is weighted (`Done` in --text-primary, the numbers muted) because
 * the numbers are reference material, not the message.
 *
 * Feedback is one-of-two, never both and never neither-forever: clicking the active button again
 * clears it, which is the only way back to "unrated". The choice is local state - there is no
 * backend to send it to yet, and the toast is the whole acknowledgement the spec asks for
 * (section 9.8 leaves the notification itself to a later step).
 */
export interface TurnFooterProps {
  footer: TurnFooterData;
}

type Feedback = 'up' | 'down' | null;

export function TurnFooter({ footer }: TurnFooterProps) {
  const [feedback, setFeedback] = useState<Feedback>(null);

  const rate = (direction: Exclude<Feedback, null>): void => {
    const next = feedback === direction ? null : direction;

    setFeedback(next);

    if (next !== null) {
      toast(
        next === 'up' ? strings.turns.footer.upToast : strings.turns.footer.downToast,
      );
    }
  };

  /*
   * The totals line, and the case that produced a bare `·` on screen.
   *
   * `TurnStarted` used to carry a fixed price, so `summary` and `detail` were never both empty and the
   * separator always had something to join. They are both empty until `TurnCompleted` arrives now - and
   * a separator with nothing on either side of it is a defect you can see. So the line is drawn only
   * when there is a line to draw; the parts that exist are joined, not concatenated.
   */
  const totals = [footer.summary, footer.detail].filter((part) => part.trim() !== '');

  return (
    <div className="turn-footer mt-[6px] flex flex-wrap items-center gap-[12px] border-t border-border-subtle py-[11px] text-[12px] text-text-secondary">
      <div className="summary min-w-[180px] flex-1">
        {totals.length === 0 ? (
          <span className="text-text-muted">{strings.turns.footer.pending}</span>
        ) : (
          totals.map((part, index) => (
            <span key={part}>
              {index === 0 ? (
                <strong className="font-medium text-text-primary">{part}</strong>
              ) : (
                <>{' · '}{part}</>
              )}
            </span>
          ))
        )}
      </div>

      <div className="feedback flex gap-[2px]">
        <button
          type="button"
          className={
            'feedback-btn grid h-[26px] w-[26px] place-items-center rounded-sm transition-all duration-fast ease-ease ' +
            (feedback === 'up'
              ? 'active up bg-green-subtle text-state-success'
              : 'text-text-muted hover:bg-bg-hover hover:text-text-primary')
          }
          title={strings.turns.footer.up}
          aria-label={strings.turns.footer.up}
          aria-pressed={feedback === 'up'}
          onClick={() => rate('up')}
        >
          <ThumbsUp size={13} aria-hidden="true" />
        </button>
        <button
          type="button"
          className={
            'feedback-btn grid h-[26px] w-[26px] place-items-center rounded-sm transition-all duration-fast ease-ease ' +
            (feedback === 'down'
              ? 'active down bg-red-subtle text-state-error'
              : 'text-text-muted hover:bg-bg-hover hover:text-text-primary')
          }
          title={strings.turns.footer.down}
          aria-label={strings.turns.footer.down}
          aria-pressed={feedback === 'down'}
          onClick={() => rate('down')}
        >
          <ThumbsDown size={13} aria-hidden="true" />
        </button>
      </div>
    </div>
  );
}
