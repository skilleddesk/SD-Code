import { ShieldAlert, ShieldCheck, ShieldQuestion } from 'lucide-react';

import { strings } from '../strings';
import { useAppStore } from '../store/store';
import { useRightPanelStore } from '../store/rightPanel';
import { useLayoutStore } from '../store/layout';

/**
 * A turn's Trust score on its footer (0.12; no cost since 1.0.0). The score never stands on colour alone: an icon and
 * a word go with it (spec section 8.6), and a click opens the Proof tab with the reasons.
 */
export function TrustChip({ turnId, sessionId }: { turnId: string; sessionId: string }) {
  const score = useAppStore((state) => state.kernel.scores[turnId]);
  const stop = useAppStore((state) => state.kernel.stops[turnId]);

  if (score === undefined && stop === undefined) {
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
      {/* The sentence itself is the turn's own ending, just above (0.14.1); printed here too, every stop
          said the same thing twice. The chip only names the kind. */}
      {stop === undefined ? null : (
        <span className="text-[10.5px] font-semibold text-state-error" title={stop.sentence}>
          {strings.kernel.stop[stop.kind]}
        </span>
      )}
    </span>
  );
}
