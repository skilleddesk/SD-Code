import { CircleCheckBig, CircleX, Play } from 'lucide-react';

import { strings } from '../../strings';
import type { VerifyRow } from '../../store/rightPanel';
import { toast } from '../../store/toast';
import { BTN, BTN_BLOCK, BTN_PRIMARY } from '../ui/button';

/**
 * The Verify tab - spec section 7.11.
 *
 * The four checks every turn has to pass before it can be called done: typecheck, build, the test
 * file that covers the change, and lint. Each row is a pass/fail icon, the check's name in mono, and
 * how long it took. The failing one is red, which is the whole point of the list - the answer to
 * "why is this turn not finished" is one glance away.
 *
 * `Run verify (⌘⏎)` is the primary action and the keyboard's way back to it (spec section 9.1).
 * Running is a toast for now; the rows are the daemon's to fill in.
 */
export function VerifyTab() {
  const rows: readonly VerifyRow[] = strings.rightPanel.verify.rows;

  /* No run yet: the tab says so rather than showing an empty list and a live button. */
  if (rows.length === 0) {
    return (
      <div className="flex flex-1 items-center justify-center p-[24px] text-center text-[12.5px] text-text-muted">
        {strings.rightPanel.verify.empty}
      </div>
    );
  }

  return (
    <>
      <div className="verify-list p-[12px]">
        {rows.map((row) => (
          <div
            key={row.name}
            className="verify-row mb-[6px] flex items-center gap-[10px] rounded-md border border-border-subtle bg-bg-raised px-[12px] py-[10px] text-[12px]"
          >
            {row.pass ? (
              <CircleCheckBig size={14} className="verify-pass text-state-success" aria-hidden="true" />
            ) : (
              <CircleX size={14} className="verify-fail text-state-error" aria-hidden="true" />
            )}
            <span className="verify-name flex-1 font-mono text-[11.5px] text-text-primary">
              {row.name}
            </span>
            <span className="verify-time font-mono text-[10.5px] text-text-muted">{row.time}</span>
          </div>
        ))}
      </div>

      <div className="mt-auto px-[12px] pb-[12px]">
        <button
          type="button"
          className={BTN + ' ' + BTN_PRIMARY + ' ' + BTN_BLOCK}
          onClick={() => toast(strings.rightPanel.verify.result)}
        >
          <Play size={12} aria-hidden="true" />
          {strings.rightPanel.verify.run}
        </button>
      </div>
    </>
  );
}
