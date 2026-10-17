import { useEffect } from 'react';

import { strings } from '../strings';
import { loadFeatures, setSwitch, useFeatures } from '../store/features';
import { Badge } from '../panels/ui/Badge';

/**
 * Settings → Advanced (Pro mode, Phase 4.4): every switch, its state and what it does. A switch that is on but does nothing yet says so
 * (`Inactive`), and a token technique says it stays off until the bench lets it on for a model (the Q3 rule).
 */
export function AdvancedTab() {
  const t = strings.profiles;
  const { view, failed } = useFeatures();

  useEffect(() => {
    void loadFeatures();
  }, []);

  return (
    <div data-advanced-tab="">
      <div className="mb-[14px]">
        <h2 className="text-[17px] font-semibold text-text-primary">{t.advancedTitle}</h2>
        <p className="mt-[4px] text-[12.5px] text-text-muted">{t.advancedDesc}</p>
      </div>
      {failed ? <p className="text-[12px] text-state-waiting">{t.unavailable}</p> : null}
      <ul className="m-0 flex list-none flex-col p-0">
        {(view?.features ?? []).map((feature) => (
          <li key={feature.name} className="flex items-start gap-[12px] border-b border-border-subtle py-[9px]" data-feature={feature.name} data-active={feature.active}>
            <input
              type="checkbox"
              role="switch"
              className="mt-[3px]"
              checked={feature.on}
              aria-label={feature.name}
              onChange={(event) => void setSwitch(feature.name, event.target.checked)}
            />
            <div className="min-w-0 flex-1">
              <div className="flex flex-wrap items-center gap-[6px]">
                <span className="font-mono text-[12px] text-text-primary">{feature.name}</span>
                {feature.tokenTechnique ? <Badge tone="muted">{t.technique}</Badge> : null}
                {feature.active ? null : <Badge tone="warning">{t.inactive}</Badge>}
                {feature.on === feature.default ? null : <Badge tone="muted">{t.changed}</Badge>}
              </div>
              <p className="m-0 mt-[2px] text-[11.5px] leading-[1.5] text-text-muted">{feature.what}</p>
              {feature.inactiveReason === null ? null : <p className="m-0 mt-[2px] text-[11.5px] leading-[1.5] text-state-waiting">{feature.inactiveReason}</p>}
              {feature.tokenTechnique ? <p className="m-0 mt-[2px] text-[11px] text-text-faint">{t.techniqueNote}</p> : null}
            </div>
          </li>
        ))}
      </ul>
    </div>
  );
}
