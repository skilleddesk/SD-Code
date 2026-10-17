import { useEffect } from 'react';

import { strings } from '../strings';
import { applyPreset, loadFeatures, setUiMode, useFeatures } from '../store/features';
import { BTN, BTN_PRIMARY, BTN_SECONDARY } from '../panels/ui/button';

/**
 * Simple / Pro and the three profiles (Phase 4.4), at the top of Settings → Safety. The same components in both modes - Pro only adds the
 * Advanced tab. A profile sets the ~40 switches in one click; `Custom` appears when switches were changed by hand.
 */
export function ProfileCard() {
  const t = strings.profiles;
  const { mode, view } = useFeatures();

  useEffect(() => {
    void loadFeatures();
  }, []);

  const current = view?.preset ?? 'balanced';

  return (
    <section className="mb-[16px] rounded-md border border-border-subtle bg-bg-raised px-[14px] py-[12px]" data-profile-card="">
      <div className="mb-[8px] flex items-center gap-[8px]">
        <span className="text-[12.5px] font-semibold text-text-primary">{t.mode}</span>
        <div role="group" aria-label={t.mode} className="ml-auto flex gap-[6px]">
          {(['simple', 'pro'] as const).map((option) => (
            <button key={option} type="button" aria-pressed={mode === option} className={BTN + ' ' + (mode === option ? BTN_PRIMARY : BTN_SECONDARY)} onClick={() => setUiMode(option)} data-ui-mode={option}>
              {t[option]}
            </button>
          ))}
        </div>
      </div>
      <p className="m-0 mb-[10px] text-[11.5px] leading-[1.55] text-text-muted">{mode === 'pro' ? t.proHelp : t.simpleHelp}</p>

      <div className="mb-[6px] text-[12.5px] font-semibold text-text-primary">{t.profile}</div>
      <div role="group" aria-label={t.profile} className="flex flex-wrap gap-[6px]">
        {(['safe', 'balanced', 'economical'] as const).map((option) => (
          <button key={option} type="button" aria-pressed={current === option} className={BTN + ' ' + (current === option ? BTN_PRIMARY : BTN_SECONDARY)} onClick={() => void applyPreset(option)} data-profile={option}>
            {t[option]}
          </button>
        ))}
        {current === 'custom' ? <span className="self-center text-[11.5px] text-state-waiting">{t.custom}</span> : null}
      </div>
      <p className="m-0 mt-[8px] text-[11.5px] leading-[1.55] text-text-muted">{t[`${current === 'custom' ? 'balanced' : current}Help` as 'safeHelp']}</p>
      <p className="m-0 mt-[4px] text-[11px] text-text-faint">{t.q3}</p>
    </section>
  );
}
