import { strings } from '../../strings';
import type { Host, Session } from '../../store/sessions';
import { PromptArea } from '../prompt';
import {
  DEMO_COLLAPSED,
  DEMO_TURNS,
  DEMO_TURNS_BEFORE,
  OPEN_TURN_WINDOW,
  TurnStream,
} from '../turns';
import { HostIcon } from '../ui/HostIcon';

/**
 * `.pane` - one chat column (spec sections 7.4 and 9.15).
 *
 * In single view a pane is the whole of `.main-content`, with no header and no divider: the tab
 * strip above already says which chat this is. In split view there are two of them side by side,
 * and then each carries a `.pane-header` - host icon, host name, a separator, the title - because
 * two chats on screen at once need to say which is which (spec section 9.15).
 *
 * The scroll area is inside a 780px column, the same width the prompt area caps itself at, so the
 * stream and the input line up down the middle of a wide pane.
 */
export interface PaneProps {
  session: Session;
  host: Host;
  /** True in split view only: a single pane has no header. */
  showHeader: boolean;
}

export function Pane({ session, host, showHeader }: PaneProps) {
  return (
    <div
      className={
        showHeader
          ? 'pane relative flex min-w-0 flex-1 flex-col overflow-hidden border-r border-border-subtle last:border-r-0'
          : 'flex min-w-0 flex-1 flex-col overflow-hidden'
      }
      data-session={session.id}
    >
      {showHeader ? (
        <div className="pane-header flex min-w-0 shrink-0 items-center gap-[8px] border-b border-border-subtle bg-bg-raised px-[12px] py-[6px] font-mono text-[11px] text-text-secondary">
          <HostIcon type={host.type} size={16} iconSize={9} />
          <span className="pane-host font-medium text-accent">{host.name}</span>
          <span className="text-border-strong" aria-hidden="true">
            {strings.main.paneHeaderSeparator}
          </span>
          <span className="pane-title min-w-0 flex-1 overflow-hidden text-ellipsis whitespace-nowrap">
            {session.title}
          </span>
        </div>
      ) : null}

      <div className="pane-scroll flex-1 overflow-y-auto px-[28px] pb-[16px] pt-[20px] max-600:px-[16px] max-600:pt-[14px]">
        <div className="pane-inner mx-auto max-w-[780px]">
          {/*
            The collapsing rule of spec section 7.5 belongs to whoever produces the stream, because
            only it knows how many turns came before this window: more than `OPEN_TURN_WINDOW` and
            the older ones fold into one line. The seed stands in for a session that has run six
            turns before this one, which is why the summary is drawn.
          */}
          <TurnStream
            turns={DEMO_TURNS}
            collapsed={
              DEMO_TURNS_BEFORE + DEMO_TURNS.length > OPEN_TURN_WINDOW ? DEMO_COLLAPSED : null
            }
          />
        </div>
      </div>

      <PromptArea />
    </div>
  );
}
