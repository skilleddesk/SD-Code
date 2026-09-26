import { useEffect, useState } from 'react';
import { Activity } from 'lucide-react';

import { strings } from '../../strings';
import type { TurnStatsData } from './types';

/**
 * `.live-stats` - the measured line under a running turn (0.9.0).
 *
 * Claude Code's own stream shows elapsed time and token counts while it works, and that visibility is
 * exactly what the report asked this stream to beat ("live streaming a aro advance and featurefull
 * indeatils"). Every number here is measured, never estimated from nothing:
 *
 *   elapsed    now minus the log's `TurnStarted` stamp, ticking twice a second
 *   thinking   the reducer's measured stretches, plus the stretch still open
 *   ~tokens    the streamed text's length over four - marked `~` because it is an approximation
 *   pace       those tokens over the elapsed seconds
 *   tools      how many calls the turn has made so far
 *
 * It exists only while the turn runs. The finished turn's footer carries the daemon's real totals
 * (cost, exact tokens), which this line never pretends to know.
 */
export function LiveStats({ stats }: { stats: TurnStatsData }) {
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 500);

    return () => window.clearInterval(timer);
  }, []);

  const started = Date.parse(stats.startedAt);
  const elapsedMs = Number.isFinite(started) ? Math.max(0, now - started) : 0;
  const thinkingOpen =
    stats.thinkingSince === null ? 0 : Math.max(0, now - Date.parse(stats.thinkingSince));
  const thinkingMs = stats.thinkingMs + (Number.isFinite(thinkingOpen) ? thinkingOpen : 0);
  const tokens = Math.round(stats.chars / 4);
  const pace = elapsedMs > 1000 ? Math.round(tokens / (elapsedMs / 1000)) : 0;

  const parts: string[] = [strings.turns.stats.elapsed(seconds(elapsedMs))];

  if (thinkingMs > 200) {
    parts.push(strings.turns.stats.thinking(seconds(thinkingMs)));
  }

  if (tokens > 0) {
    parts.push(strings.turns.stats.tokens(tokens));
  }

  if (pace > 0) {
    parts.push(strings.turns.stats.pace(pace));
  }

  if (stats.tools > 0) {
    parts.push(strings.turns.stats.tools(stats.tools));
  }

  return (
    <div
      className="live-stats mb-[10px] flex items-center gap-[8px] font-mono text-[10.5px] text-text-muted"
      data-live-stats
    >
      <Activity size={11} aria-hidden="true" className="shrink-0 animate-pulse text-accent" />
      <span>{parts.join(' · ')}</span>
    </div>
  );
}

/** `12.4s` or `2m 05s`, the same shapes the thinking block uses. */
function seconds(ms: number): string {
  const total = ms / 1000;

  if (total < 60) {
    return `${total.toFixed(1)}s`;
  }

  const minutes = Math.floor(total / 60);
  const rest = Math.floor(total % 60);

  return `${minutes}m ${String(rest).padStart(2, '0')}s`;
}
