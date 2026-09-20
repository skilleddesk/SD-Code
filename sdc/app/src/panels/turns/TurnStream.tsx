import { ChevronRight } from 'lucide-react';

import { strings } from '../../strings';
import { toast } from '../../store/toast';
import { ErrorCard } from './ErrorCard';
import { AnswerBlock } from './AnswerBlock';
import { ThinkingBlock } from './ThinkingBlock';
import { ToolCard } from './ToolCard';
import { TurnFooter } from './TurnFooter';
import { UserMessage } from './UserMessage';
import {
  type CollapsedSummaryData,
  type Turn,
} from './types';

/**
 * The turn stream - spec section 7.5.
 *
 * A scrolling column of `.turn` blocks with one optional line above them: the collapsed summary,
 * which stands in for every turn older than the last five. It is dashed and mono, all numbers and
 * no prose, and clicking it expands what it stands for.
 *
 * `collapsed` is a prop rather than something this component derives, because the stream is given a
 * window of turns and has no way to know how many came before it. The pane passes the demo summary;
 * the real stream will pass the count and totals the daemon reports. `turns` defaults to the seed of
 * `types.ts`, so `<TurnStream />` renders the prototype's worked example.
 */
export interface TurnStreamProps {
  /** The session's turns, oldest first - the reducer's projection, reshaped by `live.ts`. */
  turns: readonly Turn[];
  /** The block above the turns, or null for a session that has not run more than five yet. */
  collapsed?: CollapsedSummaryData | null;
}

export function TurnStream({ turns, collapsed = null }: TurnStreamProps) {
  /*
   * The summary is drawn whenever the caller passes one. The caller is the thing that knows how
   * many turns came before this window - `OPEN_TURN_WINDOW` in types.ts is the threshold it applies
   * (the last five turns stay open) - so this component never has to guess from `turns.length`.
   */
  const showCollapsed = collapsed !== null;

  return (
    <>
      {showCollapsed ? (
        <div
          className="collapsed-summary mb-[18px] flex cursor-pointer items-center gap-[10px] rounded-md border border-dashed border-border-default bg-bg-raised px-[12px] py-[8px] font-mono text-[11.5px] text-text-secondary transition-all duration-fast ease-ease hover:border-solid hover:border-border-strong hover:bg-bg-hover hover:text-text-primary"
          role="button"
          tabIndex={0}
          onClick={() => toast(strings.turns.collapsed.toast)}
          onKeyDown={(event) => {
            if (event.key === 'Enter') {
              toast(strings.turns.collapsed.toast);
            }
          }}
        >
          <ChevronRight size={12} aria-hidden="true" />
          <span>{collapsed.label}</span>
          <span className="meta ml-auto text-text-muted">{collapsed.meta}</span>
        </div>
      ) : null}

      {turns.map((turn) => (
        <TurnBlock key={turn.id} turn={turn} />
      ))}
    </>
  );
}

/** One `.turn`: the user's message, the meta line, then whatever the engine produced. */
function TurnBlock({ turn }: { turn: Turn }) {
  return (
    <div className="turn mb-[26px]">
      <UserMessage message={turn.user} />

      <div className="turn-meta mb-[12px] flex flex-wrap items-center gap-[10px] font-mono text-[11px] text-text-muted">
        <span>
          {turn.meta.tier} · {turn.meta.engine} · {turn.meta.model}
        </span>
        {/* The forecast span, and why it is conditional: `TurnStarted` no longer carries a price, so an
            unconditional span left a `·` with nothing after it on screen - the kind of stray mark that
            makes a working window look broken. */}
        {turn.meta.forecast === '' ? null : (
          <span className="forecast before:mr-[10px] before:text-border-strong before:content-['·']">
            {turn.meta.forecast}
          </span>
        )}
      </div>

      {turn.thinking ? <ThinkingBlock thinking={turn.thinking} /> : null}

      {turn.tools.map((tool, index) => (
        <ToolCard key={`${tool.kind}-${index}`} tool={tool} />
      ))}

      {/* The answer sits between the work and the totals: thinking, the tool cards the answer came
          out of, then what the engine actually said - and the error card above it, because a failed
          turn has an explanation where its answer would be. */}
      {turn.answer ? <AnswerBlock answer={turn.answer} /> : null}

      {turn.error ? <ErrorCard error={turn.error} /> : null}

      <TurnFooter footer={turn.footer} />
    </div>
  );
}
