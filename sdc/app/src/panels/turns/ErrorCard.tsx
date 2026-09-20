import { TriangleAlert, WandSparkles } from 'lucide-react';

import { strings } from '../../strings';
import { askPermission, fixWithAgent } from '../../store/intents';
import { tierName, useModelStore } from '../../store/model';
import { useSessionsStore } from '../../store/sessions';
import { toast } from '../../store/toast';
import { BTN, BTN_PRIMARY, BTN_SECONDARY } from '../ui/button';
import type { ErrorCardData } from './types';

/**
 * `.error-card` - a failure the stream wants you to act on (spec section 7.5).
 *
 * A red-tinted block with a 3px red edge: the title (where it failed), a paragraph in plain words
 * (why it failed), and three actions. `Fix this` is the primary one because it is the reason the
 * card exists - it hands the failure to the agent, which is what `#permissionBd` is for - while
 * `Show code` jumps to the failing line and `Explain more` asks for more prose.
 *
 * The two secondary buttons are toasts for now: jumping to a source line needs the editor
 * integration of a later step, and there is nothing more to explain until the engine answers. The
 * card itself is not collapsible - an error you can fold away is an error you will forget.
 */
export interface ErrorCardProps {
  error: ErrorCardData;
}

export function ErrorCard({ error }: ErrorCardProps) {
  const { activeTab } = useSessionsStore();
  const model = useModelStore();

  /**
   * Spec section 14.9: `Fix this` seeds a turn with the failure's own context and then asks for
   * permission to write the file it is about - which is the order the daemon enforces anyway
   * (principle P5: the checkpoint lands before the mutation).
   */
  const fix = (): void => {
    const sessionId = activeTab ?? 's1';

    void fixWithAgent({
      sessionId,
      engine: model.engine,
      model: model.model,
      tier: tierName(model.tier),
      title: error.title,
      explanation: error.explanation,
      source: error.title,
    }).then(() => askPermission({ sessionId, target: strings.permission.target }));
  };

  return (
    <div className="error-card mb-[8px] rounded-md border border-[rgba(242,94,104,.28)] border-l-[3px] border-l-state-error bg-red-subtle px-[15px] py-[12px]">
      <div className="error-title mb-[8px] flex items-center gap-[8px] text-[12.5px] font-semibold text-red-bright">
        <TriangleAlert size={14} aria-hidden="true" />
        {error.title}
      </div>

      <div className="error-explain mb-[12px] text-[12.5px] leading-[1.65] text-text-secondary">
        {error.explanation}
      </div>

      <div className="error-actions flex flex-wrap gap-[6px]">
        <button
          type="button"
          className={BTN + ' ' + BTN_PRIMARY}
          onClick={fix}
        >
          <WandSparkles size={12} aria-hidden="true" />
          {strings.turns.error.fix}
        </button>
        <button
          type="button"
          className={BTN + ' ' + BTN_SECONDARY}
          onClick={() => toast(strings.turns.error.showCodeToast)}
        >
          {strings.turns.error.showCode}
        </button>
        <button
          type="button"
          className={BTN + ' ' + BTN_SECONDARY}
          onClick={() => toast(strings.turns.error.explainMoreToast)}
        >
          {strings.turns.error.explainMore}
        </button>
      </div>
    </div>
  );
}
