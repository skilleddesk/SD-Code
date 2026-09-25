import type { TurnView } from './types';

/**
 * The Analytics tab's numbers (v4), computed from the turns in the log - nothing else.
 *
 * The tab used to print a fixed "$4.12 this week", seven fixed bars, a 68 / 22 / 10 engine split and
 * "Claude Max ~60%, resets in 2h 14m" - numbers a person could make a decision on, none of them measured.
 * Everything here is read from what the engines reported at the end of each turn (`TurnCompleted.meta`):
 * the cost they stated (`$0.0290`, `≈$0.11`) and their token counts (`12.4k in · 3.1k out`). A turn
 * that reported neither adds nothing rather than a guess.
 */

export interface TurnUsage {
  cost: number | null;
  input: number;
  output: number;
}

/** `12.4k` -> 12400, `900` -> 900. */
function count(raw: string): number {
  const value = Number.parseFloat(raw);

  if (!Number.isFinite(value)) {
    return 0;
  }

  return Math.round(raw.toLowerCase().endsWith('k') ? value * 1000 : value);
}

/** What one turn's footer says it used. */
export function usageOf(meta: string): TurnUsage {
  const cost = /\$\s*([0-9]+(?:\.[0-9]+)?)/.exec(meta);
  const input = /([0-9]+(?:\.[0-9]+)?k?)\s+in\b/i.exec(meta);
  const output = /([0-9]+(?:\.[0-9]+)?k?)\s+out\b/i.exec(meta);

  return {
    cost: cost?.[1] === undefined ? null : Number.parseFloat(cost[1]),
    input: input?.[1] === undefined ? 0 : count(input[1]),
    output: output?.[1] === undefined ? 0 : count(output[1]),
  };
}

export interface DayTotal {
  /** `YYYY-MM-DD`, in the viewer's own time zone. */
  day: string;
  cost: number;
  turns: number;
}

export interface EngineShare {
  engine: string;
  turns: number;
  percent: number;
}

export interface Analytics {
  turns: number;
  failed: number;
  /** Turns whose engine reported a cost, and what they add up to. */
  priced: number;
  cost: number;
  input: number;
  output: number;
  /** The last seven days, oldest first, including the empty ones. */
  days: DayTotal[];
  engines: EngineShare[];
}

/** A stamp as the viewer's calendar day. `null` for a stamp that is not a date. */
function dayOf(stamp: string): string | null {
  const time = Date.parse(stamp);

  if (!Number.isFinite(time)) {
    return null;
  }

  const date = new Date(time);

  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, '0')}-${String(date.getDate()).padStart(2, '0')}`;
}

export function analyticsOf(turns: readonly TurnView[], now: Date = new Date()): Analytics {
  const days: DayTotal[] = [];

  for (let back = 6; back >= 0; back -= 1) {
    const date = new Date(now.getFullYear(), now.getMonth(), now.getDate() - back);

    days.push({ day: dayOf(date.toISOString()) ?? '', cost: 0, turns: 0 });
  }

  const byEngine = new Map<string, number>();
  let failed = 0;
  let priced = 0;
  let cost = 0;
  let input = 0;
  let output = 0;

  for (const turn of turns) {
    const usage = usageOf(turn.meta);

    byEngine.set(turn.engine, (byEngine.get(turn.engine) ?? 0) + 1);
    failed += turn.status === 'failed' ? 1 : 0;
    input += usage.input;
    output += usage.output;

    if (usage.cost !== null) {
      priced += 1;
      cost += usage.cost;
    }

    const slot = days.find((day) => day.day === dayOf(turn.startedAt));

    if (slot !== undefined) {
      slot.turns += 1;
      slot.cost += usage.cost ?? 0;
    }
  }

  const engines = [...byEngine.entries()]
    .sort((left, right) => right[1] - left[1])
    .map(([engine, count]) => ({ engine, turns: count, percent: Math.round((count / Math.max(1, turns.length)) * 100) }));

  return { turns: turns.length, failed, priced, cost, input, output, days, engines };
}
