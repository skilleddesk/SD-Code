import { useEffect, useState } from 'react';
import { CornerDownRight, Sparkles, Target } from 'lucide-react';

import { strings } from '../../strings';
import { AnswerBlock } from './AnswerBlock';
import { CheckpointRail } from './CheckpointRail';
import { DraftCard } from './DraftCard';
import { ExploreGroup } from './ExploreGroup';
import { Markdown } from './Markdown';
import { splitUnderstood, useSmoothText } from './smooth';
import { ThinkingBlock } from './ThinkingBlock';
import { ToolCard } from './ToolCard';
import { gather } from './grouping';
import type { DraftData, LiveBarData, TimelineItem } from './types';

/**
 * The turn as it happened (0.12.5): thought, words, action, thought again - in that order.
 *
 * The report: *"ki korse ki think korse ... every step every process jano dakha jai"*. The stream used to
 * draw one thinking box at the top, every tool card under it and every word the agent said glued into
 * one answer at the bottom - so while it worked, the newest thought was a box that had scrolled away and
 * its "Now I understand the project, let me…" arrived only at the end. Here each stretch sits where it
 * happened; the newest one is at the bottom, next to the input, where the auto-scroll keeps the eye.
 */
export function Timeline({
  items,
  sessionId,
  live,
  draft,
}: {
  items: readonly TimelineItem[];
  sessionId: string;
  live: LiveBarData | undefined;
  /** The tool call still being written (0.14.2), drawn at the bottom where the eye is. */
  draft?: DraftData | undefined;
}) {
  return (
    <div className="timeline flex flex-col" data-timeline>
      {gather(items).map((item) => {
        switch (item.kind) {
          case 'explore':
            return <ExploreGroup key={item.key} tools={item.tools} />;
          case 'thinking':
            return <ThinkingBlock key={item.key} thinking={item.thinking} />;
          case 'text':
            return item.final ? (
              <AnswerBlock key={item.key} answer={{ text: item.text, streaming: false }} />
            ) : (
              <Narration key={item.key} text={item.text} streaming={item.streaming} />
            );
          case 'tool':
            return <ToolCard key={item.key} tool={item.tool} />;
          case 'checkpoint':
            return <CheckpointRail key={item.key} sessionId={sessionId} checkpoints={[item.checkpoint]} />;
          case 'steer':
            return (
              <div key={item.key} className="steer mb-[10px] ml-auto max-w-[85%] rounded-lg border border-accent/30 bg-accent-subtle px-[11px] py-[7px]" data-steer>
                <div className="mb-[2px] flex items-center gap-[6px] text-[10px] font-semibold uppercase tracking-[.08em] text-accent">
                  <CornerDownRight size={11} aria-hidden="true" />
                  {strings.turns.steered}
                </div>
                <div className="whitespace-pre-wrap text-[12.5px] leading-[1.55] text-text-primary">{item.text}</div>
              </div>
            );
        }
      })}

      {draft === undefined ? null : <DraftCard draft={draft} />}

      {live?.phase === 'deciding' ? <Deciding since={live.since} /> : null}
    </div>
  );
}

/** Words the agent says to the person while it works - prose, not a card, the way a colleague talks. */
function Narration({ text, streaming }: { text: string; streaming: boolean }) {
  const shown = useSmoothText(text, streaming);
  /* 0.14.2: the `Understood:` line gets its own chip here too - it was drawn as plain prose whenever the
     agent went on to use a tool, which is most of the time. */
  const { understood, rest } = splitUnderstood(shown, streaming);

  return (
    <div className="narration mb-[10px] flex gap-[9px] px-[2px]" data-narration={streaming ? 'streaming' : 'done'}>
      <Sparkles size={13} aria-hidden="true" className="mt-[4px] shrink-0 text-accent" />
      <div className="min-w-0 flex-1" aria-live={streaming ? 'polite' : undefined}>
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
    </div>
  );
}

/** The pause between two actions, with its own clock - the gap that used to read as a stalled turn. */
function Deciding({ since }: { since: string }) {
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 250);

    return () => window.clearInterval(timer);
  }, []);

  const started = Date.parse(since);
  const ms = Number.isFinite(started) ? Math.max(0, now - started) : 0;

  return (
    <div className="deciding mb-[8px] flex items-center gap-[9px] px-[2px] py-[6px] text-[12px] text-text-muted" role="status" data-deciding>
      <span className="flex gap-[3px]" aria-hidden="true">
        {[0, 1, 2].map((dot) => (
          <span
            key={dot}
            className="h-[5px] w-[5px] animate-pulse rounded-full bg-accent motion-reduce:animate-none"
            style={{ animationDelay: `${dot * 220}ms` }}
          />
        ))}
      </span>
      <span>{strings.turns.deciding}</span>
      {ms >= 1000 ? <span className="font-mono text-[10.5px] tabular-nums">{strings.turns.thinking.seconds(ms)}</span> : null}
    </div>
  );
}
