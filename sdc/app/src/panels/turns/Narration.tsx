import { Target } from 'lucide-react';

import { strings } from '../../strings';
import { Markdown } from './Markdown';
import { splitUnderstood, useSmoothText } from './smooth';

/**
 * Words the agent says to the person while it works (0.12.5) - prose, not a card, the way a colleague
 * talks. Flow draws it beside its rail node, which already marks it (0.15).
 */
export function Narration({ text, streaming }: { text: string; streaming: boolean }) {
  const shown = useSmoothText(text, streaming);
  /* 0.14.2: the `Understood:` line gets its own chip here too - it was drawn as plain prose whenever the
     agent went on to use a tool, which is most of the time. */
  const { understood, rest } = splitUnderstood(shown, streaming);

  return (
    <div className="narration mb-[10px] px-[2px]" data-narration={streaming ? 'streaming' : 'done'} aria-live={streaming ? 'polite' : undefined}>
      {understood === null ? null : (
        <div className="understood mb-[6px] flex items-start gap-[8px] rounded-md border border-accent/25 bg-accent-subtle px-[10px] py-[6px]" data-understood>
          <Target size={13} aria-hidden="true" className="mt-[3px] shrink-0 text-accent" />
          <div className="min-w-0 text-[12.5px] leading-[1.55] text-text-primary">
            <span className="mr-[6px] text-[10px] font-semibold uppercase tracking-[.08em] text-accent">{strings.turns.understood}</span>
            {understood}
          </div>
        </div>
      )}
      {rest === '' ? null : <Markdown text={rest} />}
      {streaming ? (
        <span className="ml-[1px] animate-pulse text-accent motion-reduce:animate-none" aria-hidden="true">
          ▍
        </span>
      ) : null}
    </div>
  );
}
