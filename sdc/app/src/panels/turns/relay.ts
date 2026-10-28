import type { RelayReport, RelayStepEvent, RelaySummaryEvent } from '../../../../protocol/types';
import { strings } from '../../strings';

/**
 * Silent Relay's face in a turn (M1-7, M1-8): the chip while it runs, the receipt after it.
 *
 * Both are drawn from the events the daemon wrote - the attempts, and the split it summed from them - and nothing is added here, so what a person
 * reads is what the Trace holds. A cost is shown with the word for how it was learned; an estimate is never worded as measured.
 */

/** How many steps of the turn the chain finished, and the newest attempt (for the chip's tooltip). */
export function chipOf(steps: readonly RelayStepEvent[]): { cloud: number; title: string } | null {
  const done = steps.filter((step) => step.outcome === 'done' || step.outcome === 'done-unverified');

  if (steps.length === 0) {
    return null;
  }

  const last = steps[steps.length - 1] as RelayStepEvent;
  const reason = strings.turns.relay.reasons[last.reason] ?? last.reason;

  return { cloud: done.length, title: strings.turns.relay.chipTitle(last.toModel, reason) };
}

function shortTokens(tokens: number): string {
  return tokens >= 1000 ? `${(tokens / 1000).toFixed(1)}k` : String(tokens);
}

/** The receipt's lines for a finished turn. Empty when the turn had no steps to count. */
export function receiptOf(summary: RelaySummaryEvent): string[] {
  const t = strings.turns.relay;

  if (summary.stepsTotal === 0) {
    return [];
  }

  if (summary.attempts === 0) {
    return [t.allLocal(summary.stepsTotal)];
  }

  const percent = Math.floor((summary.stepsLocal * 100) / summary.stepsTotal);
  const models = summary.byModel.filter((row) => row.steps > 0).map((row) => row.model).join(', ') || summary.byModel.map((row) => row.model).join(', ');
  const reasons = summary.reasons.map((row) => `${row.count} ${(t.reasons[row.reason] ?? row.reason)}`).join(', ');
  const lines = [
    t.split(summary.stepsTotal, summary.stepsLocal, percent, summary.stepsCloud, models),
    t.sent(shortTokens(summary.tokensIn), shortTokens(summary.tokensOut)),
    t.why(reasons),
  ];

  if (summary.unverified > 0) {
    lines.push(t.unverified(summary.unverified));
  }

  if (summary.failed > 0) {
    lines.push(t.failed(summary.failed));
  }

  return lines;
}

/** The start of today and of this month in this computer's own time, as the instants `relay.report` takes. */
export function periodStarts(now: Date): { today: string; month: string } {
  return { today: new Date(now.getFullYear(), now.getMonth(), now.getDate()).toISOString(), month: new Date(now.getFullYear(), now.getMonth(), 1).toISOString() };
}

/** One period's line: how many steps the local model did and how many the chain, and what the chain cost. */
export function relayLine(report: RelayReport): string {
  const label = strings.turns.relay.labels[report.costLabel] ?? report.costLabel;

  return strings.kernel.cost.relay.line(report.stepsLocal, report.stepsCloud, report.cloudPercent, `$${report.costUsd.toFixed(4)}`, report.costLabel === 'none' ? '—' : label);
}
