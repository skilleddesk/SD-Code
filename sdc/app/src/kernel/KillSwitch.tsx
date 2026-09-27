import { OctagonX } from 'lucide-react';
import { useState } from 'react';

import { strings } from '../strings';
import { killAll } from '../store/kernelIntents';
import { useAppStore } from '../store/store';

/**
 * The kill switch (0.12, P7): always on the topbar, one press or `Ctrl+Shift+.` stops every running turn,
 * Verify run, deploy and command, and leaves a checkpoint of each working folder as it was. It never
 * asks "are you sure" - stopping is always safe, and a question is a delay when something is going wrong.
 */
export function KillSwitch() {
  const running = useAppStore((state) => state.turns.some((turn) => turn.status === 'running' || turn.status === 'stuck'));
  const deploying = useAppStore((state) => Object.values(state.kernel.deploys).some((deploy) => deploy.state === 'running'));
  const [busy, setBusy] = useState(false);
  const live = running || deploying;

  return (
    <button
      type="button"
      id="killSwitch"
      className={
        'kill-switch inline-flex h-[28px] shrink-0 items-center gap-[6px] rounded-md border px-[9px] text-[11px] font-semibold transition-all duration-fast ease-ease active:scale-[.97] ' +
        (live
          ? 'border-state-error bg-red-subtle text-state-error hover:bg-state-error hover:text-text-on-accent'
          : 'border-border-subtle bg-bg-raised text-text-muted hover:border-state-error hover:text-state-error')
      }
      title={strings.kernel.kill.title}
      aria-label={strings.kernel.kill.title}
      aria-keyshortcuts="Control+Shift+."
      disabled={busy}
      onClick={() => {
        setBusy(true);
        void killAll().finally(() => setBusy(false));
      }}
    >
      <OctagonX size={13} aria-hidden="true" />
      <span className="max-1100:hidden">{strings.kernel.kill.label}</span>
    </button>
  );
}
