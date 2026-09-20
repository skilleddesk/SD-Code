import { X } from 'lucide-react';

import { strings } from '../../strings';
import type { SessionState } from '../../store/sessions';
import { SESSION_STATE_CLASS } from '../ui/status';

/**
 * One tab in the strip - spec section 7.4.
 *
 * Four things: a state dot, the title (which ellipsises), the host chip, and a close button. The
 * tab is 140px at its narrowest and 220px at its widest, so a strip of them scrolls rather than
 * squashing; the host chip drops out at 700px because by then the title needs the room more.
 *
 * The close button is `opacity: 0` until you hover the tab or the tab is the active one - so it is
 * always reachable on the tab you are looking at, and never in the way on the one you are not.
 * Clicking it stops the event: closing a tab is not opening it.
 *
 * The active marker is a 2px accent bar along the bottom edge, in `@layer components` in
 * src/styles/globals.css (`.tab.active::after`), because a bar in a pseudo-element is not something
 * a utility class expresses.
 */
export interface TabProps {
  id: string;
  title: string;
  hostName: string;
  state: SessionState;
  active: boolean;
  onOpen: () => void;
  onClose: () => void;
}

export function Tab({ id, title, hostName, state, active, onOpen, onClose }: TabProps) {
  return (
    <div
      className={
        'tab group relative flex min-w-[140px] max-w-[220px] shrink-0 cursor-pointer items-center gap-[8px] ' +
        'whitespace-nowrap border-r border-border-subtle py-0 pl-[12px] pr-[10px] text-[12px] ' +
        'transition-all duration-fast ease-ease ' +
        (active
          ? 'active bg-bg-base text-text-primary'
          : 'text-text-secondary hover:bg-bg-hover hover:text-text-primary')
      }
      role="button"
      tabIndex={0}
      title={title}
      aria-current={active}
      onClick={onOpen}
      onKeyDown={(event) => {
        if (event.key === 'Enter') {
          onOpen();
        }
      }}
      data-tab={id}
    >
      <span
        className={'tab-dot h-[6px] w-[6px] shrink-0 rounded-full ' + SESSION_STATE_CLASS[state]}
        aria-hidden="true"
      />
      <span className="tab-title min-w-0 flex-1 overflow-hidden text-ellipsis">{title}</span>
      <span className="tab-host shrink-0 rounded-[3px] bg-bg-raised px-[5px] py-[1px] font-mono text-[9.5px] text-text-muted max-700:hidden">
        {hostName}
      </span>
      <button
        type="button"
        className={
          'tab-close grid h-[16px] w-[16px] shrink-0 place-items-center rounded-sm text-text-muted ' +
          'transition-all duration-fast ease-ease hover:bg-bg-active hover:text-text-primary ' +
          (active ? 'opacity-100' : 'opacity-0 group-hover:opacity-100')
        }
        title={strings.tabs.close}
        aria-label={strings.tabs.close}
        onClick={(event) => {
          event.stopPropagation();
          onClose();
        }}
      >
        <X size={12} aria-hidden="true" />
      </button>
    </div>
  );
}
