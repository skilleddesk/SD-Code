import { useEffect, useRef, useState } from 'react';

import { strings } from '../../strings';
import type { Phase, Step } from './flow';

/** The non-component half of the Flow and Console pieces (0.15): clocks, colours, a step's facts. */

/** `Date.now()`, re-read every `ms` while `active` - one timer per component that needs it. */
export function useNow(ms: number, active: boolean): number {
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    if (!active) {
      setNow(Date.now());
      return;
    }

    const timer = window.setInterval(() => setNow(Date.now()), ms);

    return () => window.clearInterval(timer);
  }, [ms, active]);

  return now;
}

export const PHASE_COLOR: Record<Phase, string> = {
  think: 'var(--purple)',
  write: 'var(--green)',
  read: 'var(--text-muted)',
  edit: 'var(--orange)',
  run: 'var(--accent)',
  wait: 'var(--border-strong)',
};

/** `0.4s`, `12s`, `1m 04s` - or nothing when the step has no measured time. */
export function duration(step: Step, now: number): string {
  if (step.start === null) {
    return '';
  }

  const end = step.end ?? (step.status === 'running' ? now : null);

  return end === null ? '' : strings.turns.thinking.seconds(Math.max(0, end - step.start));
}

/** Does a step have more to show than its line? */
export function hasBody(step: Step): boolean {
  const drawn = step.drawn;

  switch (drawn.kind) {
    case 'thinking':
      return drawn.thinking.text.trim() !== '';
    case 'text':
      return drawn.text.trim() !== '';
    case 'tool':
      return drawn.tool.kind === 'edit' ? drawn.tool.diff.length > 0 : (drawn.tool.output?.length ?? 0) > 0 || drawn.tool.name === 'Question';
    case 'explore':
      return true;
    case 'checkpoint':
      return true;
    case 'steer':
      return false;
  }
}

/**
 * Output pace, sampled once a second while the turn runs: the last forty seconds as a sparkline. The
 * samples live in the component - a pace the window measured, not one the daemon reported.
 */
export function usePace(chars: number, running: boolean): number[] {
  const latest = useRef(chars);
  const last = useRef(chars);
  const [samples, setSamples] = useState<number[]>([]);

  latest.current = chars;

  useEffect(() => {
    if (!running) {
      return;
    }

    last.current = latest.current;

    const timer = window.setInterval(() => {
      const delta = Math.max(0, latest.current - last.current);

      last.current = latest.current;
      setSamples((current) => [...current.slice(-39), Math.round(delta / 4)]);
    }, 1000);

    return () => window.clearInterval(timer);
  }, [running]);

  return samples;
}

/** Take the view to a step and flash it: what a ribbon click does. */
export function reveal(id: string): void {
  const element = document.getElementById(id);

  if (element === null) {
    return;
  }

  element.scrollIntoView({ behavior: 'smooth', block: 'center' });
  element.classList.remove('step-flash');
  void element.offsetWidth;
  element.classList.add('step-flash');
}
