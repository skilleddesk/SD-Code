import { ChevronRight } from 'lucide-react';

import { strings } from '../../strings';
import { toast } from '../../store/toast';
import { FlowWork } from './FlowWork';
import { ErrorCard } from './ErrorCard';
import { PlanCard } from './PlanCard';
import { SourcesList } from './SourcesList';
import { TurnFooter } from './TurnFooter';
import { ChangedFiles } from './ChangedFiles';
import { UserMessage } from './UserMessage';
import { BrandLogo } from '../ui/BrandLogo';
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
  /** The chat these turns belong to - what a rewind from the checkpoint rail restores. */
  sessionId: string;
}

export function TurnStream({ turns, collapsed = null, sessionId }: TurnStreamProps) {
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

      {turns.map((turn, index) => (
        <TurnBlock key={turn.id} turn={turn} sessionId={sessionId} latest={index === turns.length - 1} />
      ))}
    </>
  );
}

/** One `.turn`: the user's message, the meta line, then whatever the engine produced. */
function TurnBlock({ turn, sessionId, latest }: { turn: Turn; sessionId: string; latest: boolean }) {
  return (
    <div className="turn mb-[30px]">
      <UserMessage message={turn.user} />

      {/* Who is answering (0.18): the mark, the name, and the engine and model in quiet mono. */}
      <div className="turn-meta mb-[12px] flex flex-wrap items-center gap-[8px] text-[11px] text-text-muted">
        <BrandLogo size={18} className={turn.running ? 'animate-pulse motion-reduce:animate-none' : ''} />
        <span className="text-[12.5px] font-semibold text-text-primary">{strings.topbar.brandText}</span>
        <span className="rounded-full border border-border-subtle bg-bg-raised px-[8px] py-[1px] font-mono text-[10.5px]">
          {turn.meta.tier} · {turn.meta.engine} · {turn.meta.model}
        </span>
        {/* The forecast span, and why it is conditional: `TurnStarted` no longer carries a price, so an
            unconditional span left a `·` with nothing after it on screen - the kind of stray mark that
            makes a working window look broken. */}
        {turn.meta.forecast === '' ? null : (
          <span className="forecast font-mono text-[10.5px]">
            {turn.meta.forecast}
          </span>
        )}
      </div>

      {/* The plan first: it is the map of everything below it. */}
      <PlanCard steps={turn.plan} running={turn.running} />

      {/* Everything the turn did, in the order it did it (0.12.5). The live line moved to the sticky bar
          above the input (`LiveBar`), where it cannot scroll away. */}
      <FlowWork turn={turn} sessionId={sessionId} latest={latest} />

      {/* Between Send and the first event there used to be nothing at all here, and a slow first
          token read as a dead turn. Three pulsing dots and a sentence are the honest version of
          "still alive": they claim no progress, only that the turn is waiting on the engine. */}
      {turn.waiting === true ? (
        <div
          className="waiting mb-[8px] flex items-center gap-[9px] px-[2px] py-[6px] text-[12px] text-text-muted"
          role="status"
        >
          <span className="flex gap-[3px]" aria-hidden="true">
            {[0, 1, 2].map((dot) => (
              <span
                key={dot}
                className="h-[5px] w-[5px] animate-pulse rounded-full bg-accent motion-reduce:animate-none"
                style={{ animationDelay: `${dot * 220}ms` }}
              />
            ))}
          </span>
          <span>{strings.turns.answer.waiting(turn.meta.model)}</span>
        </div>
      ) : null}

      {/* The answer sits between the work and the totals: thinking, the tool cards the answer came
          out of, then what the engine actually said - and the error card above it, because a failed
          turn has an explanation where its answer would be. */}
      {turn.error ? <ErrorCard error={turn.error} /> : null}

      {/* A `/research` answer's numbered sources (0.16.1): what every [n] in it points at. */}
      {turn.sources !== undefined && turn.sources.length > 0 ? <SourcesList sources={turn.sources} /> : null}

      {turn.running ? null : <ChangedFiles items={turn.timeline} />}

      <TurnFooter
        footer={turn.footer}
        verify={turn.running ? null : { sessionId, turnId: turn.id, author: turn.author, result: turn.verify ?? null }}
      />
    </div>
  );
}
