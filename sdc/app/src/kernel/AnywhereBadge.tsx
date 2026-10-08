import { useEffect, useState } from 'react';

import type { AnywhereStatus } from '../../../protocol/types';
import { StatusItem } from '../panels/statusbar/StatusItem';
import { anywhereStatus } from '../store/anywhere';
import { useOverlayStore } from '../store/overlays';
import { strings } from '../strings';

/**
 * The status bar's SDC Anywhere segment (0.17): nothing while it is off; while it is on, a dot for the relay link
 * and a count of what is waiting - a request from a phone, or a phone asking to pair. It opens Settings → SDC
 * Anywhere, where both are answered.
 */
export function AnywhereBadge() {
  const [status, setStatus] = useState<AnywhereStatus | null>(null);
  const openSettings = useOverlayStore((state) => state.openSettings);

  useEffect(() => {
    let alive = true;
    const refresh = (): void => {
      void anywhereStatus().then((next) => {
        if (alive) setStatus(next);
      });
    };

    refresh();

    const timer = setInterval(refresh, 5000);

    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, []);

  if (status === null || !status.running) return null;

  const waiting = status.waitingApprovals + status.pairingRequests;
  const words = strings.anywhere.badge;

  return (
    <>
      <StatusItem
        id="statusAnywhere"
        dotClass={status.connected ? 'bg-state-success' : 'bg-state-waiting'}
        tone={waiting > 0 ? 'text-accent' : 'text-text-muted'}
        title={status.connected ? words.titleConnected : words.titleConnecting}
        hideSmall
        hideTiny
        onClick={() => openSettings('anywhere')}
      >
        {waiting > 0 ? words.waiting(waiting) : words.label}
      </StatusItem>
      <span className="sep hide-sm text-border-strong max-900:hidden" aria-hidden="true">
        ·
      </span>
    </>
  );
}
