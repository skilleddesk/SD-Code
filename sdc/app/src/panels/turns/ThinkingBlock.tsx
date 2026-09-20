import { Brain, ChevronRight } from 'lucide-react';
import { useState } from 'react';

import { strings } from '../../strings';
import type { ThinkingData } from './types';

/**
 * `.thinking` - the engine's reasoning, collapsed by default (spec section 7.5).
 *
 * A raised block with a 2px far-left edge: `--border-strong` at rest, `--accent` on hover, which is
 * the whole affordance - there is no chevron-button, just the row. The head shows the brain icon,
 * the word `Thinking`, the duration in parentheses (un-uppercased, because it is a measurement and
 * not a label) and a chevron that rotates a quarter turn when the body opens.
 *
 * The click handler is on the block, not on the head, because that is what the prototype does: a
 * click anywhere in the open reasoning closes it again. `aria-expanded` on the block keeps that
 * honest for a keyboard user, who reaches it with Tab and triggers it with Enter.
 */
export interface ThinkingBlockProps {
  thinking: ThinkingData;
}

export function ThinkingBlock({ thinking }: ThinkingBlockProps) {
  const [open, setOpen] = useState(false);

  return (
    <div
      className="thinking mb-[8px] cursor-pointer rounded-md border border-border-subtle border-l-2 border-l-border-strong bg-bg-raised px-[13px] py-[9px] text-[12px] text-text-secondary transition-all duration-fast ease-ease hover:border-l-accent hover:bg-bg-hover"
      role="button"
      tabIndex={0}
      aria-expanded={open}
      aria-label={strings.turns.thinking.title}
      onClick={() => setOpen((current) => !current)}
      onKeyDown={(event) => {
        if (event.key === 'Enter') {
          setOpen((current) => !current);
        }
      }}
    >
      <div className="thinking-head flex items-center gap-[8px] text-[10px] font-semibold uppercase tracking-[.08em] text-text-muted">
        <Brain size={12} aria-hidden="true" />
        <span>{strings.turns.thinking.title}</span>
        <span className="font-normal normal-case tracking-normal text-text-muted">
          {thinking.duration}
        </span>
        <ChevronRight
          size={12}
          aria-hidden="true"
          className={'ml-auto transition-transform duration-200 ease-ease ' + (open ? 'rotate-90' : '')}
        />
      </div>

      {open ? (
        <div className="thinking-body mt-[10px] text-[12px] italic leading-[1.65] text-text-secondary">
          {thinking.text}
        </div>
      ) : null}
    </div>
  );
}
