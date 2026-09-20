import { WifiOff } from 'lucide-react';

import { strings } from '../../strings';
import { STALE_MISSES, useDaemonStore } from '../../store/daemon';
import { heartbeat } from '../../store/intents';
import { toast } from '../../store/toast';
import { unreachableHosts, useSessionsStore } from '../../store/sessions';

/**
 * `#degradedBanner` - the amber strip that says something the window is talking to is not answering
 * (spec section 7.4).
 *
 * Two cases, and one of them used to be silent:
 *
 * 1. **the daemon itself** - `sdcd` is started by this app and killed with it, but it can also die on
 *    its own: a crash, a task manager, a machine under memory pressure. Nothing said so. Every click
 *    answered nothing, the window looked normal, and the only way to find out was to notice that
 *    nothing happened. `store/daemon.ts` is where the heartbeat records it, and this is where it is
 *    said - with `Retry now`, because a person who sees this wants to act, not to wait.
 * 2. **one host of several** - the daemon is answering but cannot reach that machine. The copy is
 *    the design's and it is the important part: it says what is missing (the connection) and what is
 *    not (your files, your history).
 *
 * The daemon case is checked first because it makes the host case meaningless: when the daemon is
 * gone, every host is unreachable *including* `local`, and a banner naming one of them would be
 * noise in front of the real news.
 */
export function DegradedBanner() {
  const hosts = useSessionsStore((state) => state.hosts);
  const online = useDaemonStore((state) => state.online);
  const misses = useDaemonStore((state) => state.misses);

  const broken = unreachableHosts(hosts);

  if (!online) {
    const note = misses >= STALE_MISSES ? strings.daemon.bannerStale : strings.daemon.banner;

    return (
      <div
        className="degraded-banner flex shrink-0 items-center gap-[10px] border-b border-[rgba(245,165,36,.3)] bg-orange-subtle px-[14px] py-[8px] text-[12px] text-state-waiting"
        id="degradedBanner"
        data-daemon-offline="true"
        role="status"
      >
        <WifiOff size={14} aria-hidden="true" />
        <span id="degradedText">{note}</span>
        <div className="spacer flex-1" />
        <button
          type="button"
          className="rounded-sm px-[8px] py-[3px] font-semibold text-state-waiting transition-colors duration-fast ease-ease hover:bg-[rgba(245,165,36,.15)]"
          data-action="retry-daemon"
          onClick={() => void heartbeat()}
        >
          {strings.daemon.retry}
        </button>
      </div>
    );
  }

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
