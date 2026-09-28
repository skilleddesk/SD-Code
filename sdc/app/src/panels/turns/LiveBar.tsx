import { useEffect, useState } from 'react';
import { ArrowDown, Brain, Loader2, PenLine, Sparkles, Terminal } from 'lucide-react';

import { strings } from '../../strings';
import type { LiveBarData, TurnStatsData } from './types';

/**
 * The sticky line above the input while a turn runs (0.12.5).
 *
 * It replaces the stats line that sat under the turn's meta and scrolled out of view as soon as the
 * turn grew. This one stays put: what is happening this second (and for how long), the plan step it
 * belongs to, and the turn's running totals - elapsed, tokens, pace, tool calls. When the reader has
 * scrolled up, it also carries the way back to the newest line.
 */
export function LiveBar({
  live,
  stats,
  model,
  behind,
  onJump,
}: {
  live: LiveBarData;
  stats: TurnStatsData;
  model: string;
  behind: boolean;
  onJump: () => void;
}) {
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 250);

    return () => window.clearInterval(timer);
  }, []);

  const since = Date.parse(live.since);
  const phaseMs = Number.isFinite(since) ? Math.max(0, now - since) : 0;
  const started = Date.parse(stats.startedAt);
  const elapsedMs = Number.isFinite(started) ? Math.max(0, now - started) : 0;
  const tokens = Math.round(stats.chars / 4);
  const pace = elapsedMs > 1000 ? Math.round(tokens / (elapsedMs / 1000)) : 0;
  const totals = [
    strings.turns.thinking.seconds(elapsedMs),
    ...(tokens > 0 ? [strings.turns.stats.tokens(tokens)] : []),
    ...(pace > 0 ? [strings.turns.stats.pace(pace)] : []),
    ...(stats.tools > 0 ? [strings.turns.stats.tools(stats.tools)] : []),
  ];

  const { icon: Icon, label, tone } = phaseLook(live.phase, model);

  return (
    <div className="live-bar mx-auto w-full max-w-[780px] px-[28px] max-600:px-[16px]" data-live-bar={live.phase}>
      <div className="flex min-w-0 items-center gap-[9px] rounded-md border border-border-subtle bg-bg-raised px-[11px] py-[6px] text-[11.5px] shadow-sm">
        <Icon
          size={13}
          aria-hidden="true"
          className={'shrink-0 ' + tone + (live.phase === 'tool' || live.phase === 'waiting' ? ' animate-spin motion-reduce:animate-none' : ' animate-pulse motion-reduce:animate-none')}
        />
        <span className={'shrink-0 font-semibold ' + tone}>{label}</span>
        <span className="shrink-0 font-mono text-[10.5px] tabular-nums text-text-muted">{strings.turns.thinking.seconds(phaseMs)}</span>

        {live.detail === '' ? null : (
          <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-text-secondary" title={live.detail}>
            {live.detail}
          </span>
        )}

        {live.step === null ? (
          live.detail === '' ? <span className="flex-1" /> : null
        ) : (
          <span
            className="hidden min-w-0 max-w-[40%] shrink truncate rounded-sm bg-accent-subtle px-[6px] py-[1px] text-[10.5px] text-accent sm:inline"
            title={live.step.text}
          >
            {strings.turns.live.step(live.step.index, live.step.total)} · {live.step.text}
          </span>
        )}

        <span className="hidden shrink-0 font-mono text-[10.5px] tabular-nums text-text-muted md:inline" role="status" aria-live="off">
          {totals.join(' · ')}
        </span>

        {behind ? (
          <button
            type="button"
            className="flex shrink-0 items-center gap-[4px] rounded-full border border-border-default px-[8px] py-[2px] text-[10.5px] font-medium text-text-primary hover:border-border-strong"
            onClick={onJump}
          >
            <ArrowDown size={11} aria-hidden="true" />
            {strings.turns.live.jump}
          </button>
        ) : null}
      </div>
    </div>
  );
}

function phaseLook(phase: LiveBarData['phase'], model: string): { icon: typeof Brain; label: string; tone: string } {
  switch (phase) {
    case 'thinking':
      return { icon: Brain, label: strings.turns.live.thinking, tone: 'text-purple' };
    case 'writing':
      return { icon: PenLine, label: strings.turns.live.writing, tone: 'text-accent' };
    case 'tool':
      return { icon: Loader2, label: strings.turns.live.tool, tone: 'text-state-warning' };
    case 'deciding':
      return { icon: Sparkles, label: strings.turns.live.deciding, tone: 'text-accent' };
    case 'waiting':
      return { icon: Loader2, label: strings.turns.live.waiting(model), tone: 'text-text-secondary' };
  }

  return { icon: Terminal, label: '', tone: 'text-text-muted' };
}
