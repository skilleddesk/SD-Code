import { ChevronDown } from 'lucide-react';

import { strings } from '../../strings';
import { anchorBelow, useOverlayStore } from '../../store/overlays';
import { findSession, useSessionsStore } from '../../store/sessions';
import { HOST_STATUS_DOT_CLASS } from '../ui/status';

/**
 * `#activeHostBtn` - the host pill (spec section 7.1, row 3).
 *
 * Three pieces, left to right: a 6px state dot with a halo, the host's name, and an 11px chevron.
 * The pill names the host of the *active session* - not a separately remembered "current host" -
 * so opening a chat on prod-1 moves the pill with it; `activeHostId` is the fallback for when no
 * chat is open at all, which is why both are consulted here.
 *
 * Clicking it opens the host switcher popover, which is the same popover the status bar's host item
 * opens (spec section 7.15: "every item is clickable"). That popover is rendered from src/App.tsx
 * rather than from inside this button, because the topbar is a stacking context and anything drawn
 * inside it would sit under the right panel's drawer on a narrow window.
 *
 * The name hides at 520px, and that rule is not a utility class here: it is
 * `src/layout/Shell.css`'s `.host-pill .host-label`, which was written for exactly this element
 * back when the topbar was a placeholder. The class names below are what it keys off.
 */

export function HostPill() {
  const { hosts, activeTab, activeHostId } = useSessionsStore();
  const openHostSwitcher = useOverlayStore((state) => state.openHostSwitcher);

  const activeHost = findSession(hosts, activeTab)?.host ?? hosts.find((host) => host.id === activeHostId) ?? hosts[0];

  if (!activeHost) {
    return null;
  }

  return (
    <button
      type="button"
      id="activeHostBtn"
      className="host-pill flex items-center gap-[6px] px-[10px] py-[5px] h-[28px] shrink-0 rounded-md bg-bg-raised border border-border-subtle font-mono text-[11px] text-text-secondary transition-all duration-fast ease-ease hover:border-border-default hover:bg-bg-hover hover:text-text-primary"
      title={strings.topbar.activeHostTitle}
      onClick={(event) => openHostSwitcher(anchorBelow(event.currentTarget))}
    >
      <span className={'dot h-[6px] w-[6px] rounded-full ' + HOST_STATUS_DOT_CLASS[activeHost.status]} />
      <span className="host-label max-w-[110px] overflow-hidden text-ellipsis whitespace-nowrap">
        {activeHost.name}
      </span>
      <ChevronDown size={11} aria-hidden="true" />
    </button>
  );
}
