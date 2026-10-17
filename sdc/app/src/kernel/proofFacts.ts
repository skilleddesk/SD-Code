import type { ReceiptRow, ReceiptTotals } from '../../../protocol/types';
import { strings } from '../strings';
import type { TurnVerifyData } from '../panels/turns/types';

/* What the Proof-of-Work badge says, as plain functions of the facts - separate from the markup so a test reads them and
   so the component file exports only components. */
const t = strings.turns.proof;

export interface ProofFacts {
  verify: Pick<TurnVerifyData, 'state' | 'pass' | 'checksPassed' | 'checksFailed'> | null;
  /** Lines added + removed by the turn, or `null` when it changed nothing. */
  lines: number | null;
  totals: ReceiptTotals | null;
  rows: ReceiptRow[];
}

/** The distinct models of the turn, in the order they first ran, as `model (n steps)`. */
export function modelsLine(rows: readonly ReceiptRow[]): string {
  const byModel = new Map<string, number>();

  for (const row of rows) {
    byModel.set(row.modelKey, (byModel.get(row.modelKey) ?? 0) + row.steps);
  }

  return [...byModel].map(([model, steps]) => `${model} (${steps})`).join(' + ');
}

/** The badge's parts, as strings, in the order they are shown - separated from the markup so a test can read them. */
export function badgeParts(facts: ProofFacts): string[] {
  const parts: string[] = [];
  const verify = facts.verify;

  if (verify === null || verify.state !== 'done') {
    parts.push(t.unverified);
  } else if (verify.checksFailed > 0 || verify.pass === false) {
    parts.push(t.checksFailed(Math.max(1, verify.checksFailed)));
  } else if (verify.checksPassed > 0) {
    parts.push(t.verified(verify.checksPassed));
  } else {
    parts.push(t.unverified);
  }

  if (facts.lines !== null && facts.lines > 0) {
    parts.push(t.lines(facts.lines));
  }

  if (facts.rows.length > 0) {
    parts.push(modelsLine(facts.rows));
  }

  return parts;
}

