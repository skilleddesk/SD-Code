import { Brain, ChevronRight } from 'lucide-react';
import { useEffect, useLayoutEffect, useRef, useState } from 'react';

import { strings } from '../../strings';
import type { ThinkingData } from './types';

/**
 * `.thinking` - the engine's reasoning (spec section 7.5, v4).
 *
 * Two states, and the block moves between them by itself:
 *
 *   live    the engine is thinking right now: the block is **open**, the reasoning streams in as it
 *           arrives, a clock counts the seconds, and a caret sits at the end of the text. The body is
 *           capped at 180px and follows the newest line, so a long think does not push the answer off
 *           the screen.
 *   folded  the engine has moved on (an answer, a tool call, the end of the turn): the block folds to
 *           one line, `Thought for 6.2s`, measured from the log's own timestamps.
 *
 * A click always wins: it opens a folded block or folds a live one, and from then on the block stays
 * where the person put it. The time is never written by hand - `ms` and `since` come from the reducer,
 * which measures them between events.
 */
export interface ThinkingBlockProps {
  thinking: ThinkingData;
}

/** How long it has been thinking, now: the ended stretches plus the one still going on. */
function elapsed(thinking: ThinkingData, now: number): number {
  if (thinking.since === null) {
    return thinking.ms;
  }

  const started = Date.parse(thinking.since);

  return thinking.ms + (Number.isFinite(started) ? Math.max(0, now - started) : 0);
}

export function ThinkingBlock({ thinking }: ThinkingBlockProps) {
  /* `null` = follow the engine (open while live); a boolean = the person chose. */
  const [chosen, setChosen] = useState<boolean | null>(null);
  const [now, setNow] = useState(() => Date.now());
  const body = useRef<HTMLDivElement>(null);
  const open = chosen ?? thinking.live;

  /* The clock ticks only while the block is live; a folded block has a fixed time and no timer. */
  useEffect(() => {
    if (!thinking.live) {
      return;
    }

    const timer = window.setInterval(() => setNow(Date.now()), 100);

    return () => window.clearInterval(timer);
  }, [thinking.live]);

  /* While live, the capped body follows the newest line the way a terminal does. */
  useLayoutEffect(() => {
    const element = body.current;

    if (thinking.live && element !== null) {
      element.scrollTop = element.scrollHeight;
    }
  }, [thinking.live, thinking.text]);

  const ms = elapsed(thinking, now);
  const time = ms > 0 ? strings.turns.thinking.seconds(ms) : '';
  const label = thinking.live
    ? strings.turns.thinking.title
    : time === ''
      ? strings.turns.thinking.thought
      : strings.turns.thinking.thoughtFor(time);
  const toggle = (): void => setChosen(!open);

  return (
    <div
      className={
        'thinking mb-[8px] rounded-md border border-border-subtle border-l-2 bg-bg-raised text-[12px] text-text-secondary transition-colors duration-fast ease-ease ' +
        (thinking.live ? 'live border-l-purple' : 'border-l-border-strong hover:border-l-accent')
      }
      data-live={thinking.live ? 'true' : 'false'}
    >
      <button
        type="button"
        className="thinking-head flex w-full cursor-pointer items-center gap-[8px] rounded-md px-[13px] py-[9px] text-left text-[10px] font-semibold uppercase tracking-[.08em] text-text-muted hover:bg-bg-hover focus-visible:outline focus-visible:outline-2 focus-visible:outline-border-focus"
        aria-expanded={open}
        aria-label={open ? strings.turns.thinking.collapse : strings.turns.thinking.expand}
        onClick={toggle}
      >
        <Brain size={12} aria-hidden="true" className={thinking.live ? 'text-purple' : undefined} />
        <span className={thinking.live ? 'text-purple' : undefined}>{label}</span>
        {thinking.live && time !== '' ? (
          <span className="font-mono font-normal normal-case tracking-normal tabular-nums text-text-muted">{time}</span>
        ) : null}
        <ChevronRight
          size={12}
          aria-hidden="true"
          className={'ml-auto transition-transform duration-200 ease-ease ' + (open ? 'rotate-90' : '')}
        />
      </button>

      {open ? (
        <div
          ref={body}
          className={
            'thinking-body whitespace-pre-wrap px-[13px] pb-[10px] text-[12px] italic leading-[1.65] text-text-secondary ' +
            (thinking.live ? 'max-h-[180px] overflow-y-auto' : '')
          }
          aria-live={thinking.live ? 'polite' : undefined}
        >
          {thinking.text}
          {thinking.live ? (
            <span
              className="ml-[2px] inline-block h-[12px] w-[6px] translate-y-[2px] animate-pulse bg-purple motion-reduce:animate-none"
              aria-hidden="true"
            />
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
