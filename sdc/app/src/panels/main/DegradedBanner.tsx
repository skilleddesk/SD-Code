import { WifiOff } from 'lucide-react';

import { strings } from '../../strings';
import { toast } from '../../store/toast';
import { unreachableHosts, useSessionsStore } from '../../store/sessions';

/**
 * `#degradedBanner` - the amber strip that says a host is not reachable (spec section 7.4).
 *
 * > Not connected to prod-1. Files and history are still available.   [Reconnect]
 *
 * It appears only while some host is `degraded` or `offline`, and it names the *first* such host:
 * one line is all the strip has room for, and the sidebar's status dots carry the rest. The copy is
 * the important part of the design - it says what is missing (the connection) and what is not (your
 * files, your history), which is the difference between an app that lost data and one that is
 * waiting for a network.
 *
 * `Reconnect` is a toast for now: there is nothing to reconnect to until the daemon of a later step
 * exists, and the host's own status is what will clear this strip.
 */
export function DegradedBanner() {
  const hosts = useSessionsStore((state) => state.hosts);
  const broken = unreachableHosts(hosts);

  if (broken.length === 0) {
    return null;
  }

  return (
    <div
      className="degraded-banner flex shrink-0 items-center gap-[10px] border-b border-[rgba(245,165,36,.3)] bg-orange-subtle px-[14px] py-[8px] text-[12px] text-state-waiting"
      id="degradedBanner"
      role="status"
    >
      <WifiOff size={14} aria-hidden="true" />
      <span id="degradedText">{strings.main.degraded.message(broken[0]?.name ?? '')}</span>
      <div className="spacer flex-1" />
      <button
        type="button"
        className="rounded-sm px-[8px] py-[3px] font-semibold text-state-waiting transition-colors duration-fast ease-ease hover:bg-[rgba(245,165,36,.15)]"
        onClick={() => toast(strings.main.degraded.reconnected)}
      >
        {strings.main.degraded.reconnect}
      </button>
    </div>
  );
}
