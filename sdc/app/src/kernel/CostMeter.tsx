import { CircleDollarSign } from 'lucide-react';
import { useEffect, useState } from 'react';

import { strings } from '../strings';
import { usd } from '../store/kernel';
import { costSummary } from '../store/kernelIntents';
import { useKernelUi } from '../store/kernelUi';
import { useAppStore } from '../store/store';
import { StatusItem } from '../panels/statusbar/StatusItem';

/**
 * The status bar's live cost meter (0.12): today's spend, read from the daemon's ledger of turns, and -
 * while a turn runs - its estimate, labelled as one (P4). A click opens the Cost center.
 */
export function CostMeter() {
  const costs = useAppStore((state) => state.kernel.costs);
  /* The newest running turn, found without building a new array (a selector must return a stable value). */
  const running = useAppStore((state) => {
    for (let index = state.turns.length - 1; index >= 0; index -= 1) {
      const turn = state.turns[index];

      if (turn !== undefined && (turn.status === 'running' || turn.status === 'stuck')) {
        return turn;
      }
    }

    return null;
  });
  const openCost = useKernelUi((state) => state.openCost);
  const [today, setToday] = useState<number | null>(null);
  const count = Object.keys(costs).length;

  /* The day's total is the daemon's (it knows turns this window never saw); refreshed when a turn ends. */
  useEffect(() => {
    let alive = true;

    void costSummary().then((summary) => {
      if (alive && summary !== null) {
        setToday(summary.today);
      }
    });

    return () => {
      alive = false;
    };
  }, [count]);

  const estimate = running?.estimate?.usd ?? null;

  return (
    <StatusItem id="statusCost" title={strings.kernel.cost.meterTitle} hideTiny onClick={() => openCost()}>
      <CircleDollarSign size={11} aria-hidden="true" className="mr-[4px] inline" />
      {strings.kernel.cost.meter(today === null ? '—' : usd(today))}
      {running !== null && estimate !== null ? <span className="ml-[6px] text-text-muted">{strings.kernel.cost.meterTurn(usd(estimate))}</span> : null}
    </StatusItem>
  );
}
