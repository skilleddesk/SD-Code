import { ReceiptText, ShieldCheck } from 'lucide-react';
import { useEffect, useState } from 'react';

import type { ReceiptRow, ReceiptTotals } from '../../../protocol/types';
import { turnReceipt } from '../store/kernelIntents';
import { strings } from '../strings';
import { badgeParts, type ProofFacts } from './proofFacts';

/**
 * The Proof-of-Work badge and the receipt (P0-15), under a finished turn.
 *
 * The badge is one line of facts, and **only what was measured is on it**: the checks Verify ran (`✓ 4 checks passed`),
 * the lines the turn changed, and which models did the steps - never a "0 LSP errors" for a language server that was not
 * asked. A turn Verify has not run says `not verified`, which is a fact too.
 *
 * The receipt opens under it: one row per model (and purpose) with its steps, time, tokens and cost, and whether those
 * steps ran on this computer, on the Rotation Chain, or on the model the person chose. Every number comes from the Step
 * Trace (`turn.receipt`), so the receipt and the trace cannot disagree, and a model with no price says so instead of
 * showing $0.00.
 */
const t = strings.turns.proof;

function money(value: number | null): string {
  return value === null ? t.free : value >= 1 ? `$${value.toFixed(2)}` : `$${value.toFixed(4)}`;
}

function duration(ms: number): string {
  return ms >= 60_000 ? `${Math.floor(ms / 60_000)}m ${Math.round((ms % 60_000) / 1000)}s` : ms >= 1000 ? `${(ms / 1000).toFixed(1)}s` : `${ms}ms`;
}

function who(row: ReceiptRow): string {
  return row.chain ? t.chain : row.local ? t.local : t.chosen;
}

export function ReceiptTable({ rows, totals }: { rows: readonly ReceiptRow[]; totals: ReceiptTotals }) {
  return (
    <table className="receipt-table mt-[6px] w-full border-collapse text-left font-mono text-[10.5px] text-text-secondary" aria-label={t.title}>
      <tbody>
        {rows.map((row) => (
          <tr key={`${row.modelKey}|${row.purpose}`} className="border-t border-border-subtle" data-receipt-row={row.modelKey}>
            <td className="py-[3px] pr-[8px] text-text-primary">{row.modelKey}</td>
            <td className="pr-[8px]">{who(row)}</td>
            <td className="pr-[8px]">{t.stepsLabel(row.steps)}</td>
            <td className="pr-[8px]">{duration(row.wallMs)}</td>
            <td className="pr-[8px]">
              {row.tokensIn} → {row.tokensOut}
            </td>
            <td className="text-right">{money(row.costUsd)}</td>
          </tr>
        ))}
        <tr className="border-t border-border-default font-semibold text-text-primary" data-receipt-total="">
          <td className="py-[3px] pr-[8px]">{t.total}</td>
          <td className="pr-[8px]">{t.split(totals.localSteps, totals.chainSteps)}</td>
          <td className="pr-[8px]">{t.stepsLabel(totals.steps)}</td>
          <td className="pr-[8px]">{duration(totals.wallMs)}</td>
          <td className="pr-[8px]">
            {totals.tokensIn} → {totals.tokensOut}
          </td>
          <td className="text-right">{money(totals.costUsd)}</td>
        </tr>
      </tbody>
    </table>
  );
}

/** The badge and (opened) the receipt, from facts already in hand. No network, no store: what a test renders. */
export function ProofBadgeView({ facts, open, onToggle }: { facts: ProofFacts; open: boolean; onToggle: () => void }) {
  const parts = badgeParts(facts);
  const failed = facts.verify !== null && facts.verify.state === 'done' && (facts.verify.checksFailed > 0 || facts.verify.pass === false);

  return (
    <div className="proof-badge mt-[4px]" data-proof-badge="">
      <div className="flex flex-wrap items-center gap-[6px]">
        <span
          className={
            'inline-flex items-center gap-[5px] rounded-full px-[8px] py-[1px] text-[10.5px] font-semibold ' +
            (failed ? 'bg-red-subtle text-state-error' : 'bg-green-subtle text-state-success')
          }
        >
          <ShieldCheck size={11} aria-hidden="true" />
          {parts.join(' · ')}
        </span>

        {facts.totals === null || facts.totals.steps === 0 ? null : (
          <button
            type="button"
            className="inline-flex items-center gap-[4px] rounded-full border border-border-default px-[8px] py-[1px] text-[10.5px] text-text-secondary transition-colors duration-fast hover:border-border-strong hover:text-text-primary"
            aria-expanded={open}
            onClick={onToggle}
          >
            <ReceiptText size={11} aria-hidden="true" />
            {open ? t.close : t.open}
          </button>
        )}
      </div>

      {open && facts.totals !== null ? <ReceiptTable rows={facts.rows} totals={facts.totals} /> : null}
    </div>
  );
}

/** The connected badge: reads the receipt once the turn is over. */
export function ProofBadge({ turnId, verify, lines }: { turnId: string; verify: ProofFacts['verify']; lines: number | null }) {
  const [receipt, setReceipt] = useState<{ rows: ReceiptRow[]; totals: ReceiptTotals } | null>(null);
  const [open, setOpen] = useState(false);

  useEffect(() => {
    let live = true;

    void turnReceipt(turnId).then((result) => {
      if (live && result !== null) {
        setReceipt({ rows: result.rows, totals: result.totals });
      }
    });

    return () => {
      live = false;
    };
  }, [turnId]);

  return <ProofBadgeView facts={{ verify, lines, totals: receipt?.totals ?? null, rows: receipt?.rows ?? [] }} open={open} onToggle={() => setOpen((value) => !value)} />;
}
