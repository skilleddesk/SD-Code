import { ShieldAlert, ShieldCheck, ShieldQuestion } from 'lucide-react';

import { strings } from '../strings';
import { measured, usd } from '../store/kernel';
import { useAppStore } from '../store/store';
import { useRightPanelStore } from '../store/rightPanel';
import { useLayoutStore } from '../store/layout';

/**
 * A turn's Trust score and cost, on its footer (0.12). The score never stands on colour alone: an icon and
 * a word go with it (spec section 8.6), and a click opens the Proof tab with the reasons.
 */
export function TrustChip({ turnId, sessionId }: { turnId: string; sessionId: string }) {
  const score = useAppStore((state) => state.kernel.scores[turnId]);
  const cost = useAppStore((state) => state.kernel.costs[turnId]);
  const stop = useAppStore((state) => state.kernel.stops[turnId]);

  if (score === undefined && cost === undefined && stop === undefined) {
    return null;
  }

  const Icon = score?.level === 'high' ? ShieldCheck : score?.level === 'medium' ? ShieldQuestion : ShieldAlert;
  const tone =
    score?.level === 'high' ? 'text-state-success bg-green-subtle' : score?.level === 'medium' ? 'text-state-waiting bg-orange-subtle' : 'text-state-error bg-red-subtle';

  return (
    <span className="trust-chip inline-flex flex-wrap items-center gap-[6px]">
      {score === undefined ? null : (
        <button
          type="button"
          className={'inline-flex items-center gap-[4px] rounded-full px-[8px] py-[1px] text-[10.5px] font-semibold ' + tone}
          title={score.reasons.map((reason) => reason.text).join(' · ')}
          onClick={() => {
            useLayoutStore.getState().showRight();
            useRightPanelStore.getState().setActiveTab('proof', sessionId);
          }}
        >
          <Icon size={11} aria-hidden="true" />
          {strings.kernel.trust.chip(score.score, strings.kernel.trust.level[score.level])}
        </button>
      )}
      {cost === undefined ? null : (
        <span className="font-mono text-[10.5px] text-text-muted" title={strings.kernel.cost.source[cost.costSource]}>
          {measured(cost.costSource) ? usd(cost.costUsd) : `${usd(cost.costUsd)} · ${strings.kernel.cost.sourceShort[cost.costSource]}`}
        </span>
      )}
      {stop === undefined ? null : <span className="text-[10.5px] font-semibold text-state-error">{stop.sentence}</span>}
    </span>
  );
}
