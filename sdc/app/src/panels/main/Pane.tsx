import { MessageSquare } from 'lucide-react';

import { strings } from '../../strings';
import type { Host, Session } from '../../store/sessions';
import { useAppStore } from '../../store/store';
import { PromptArea } from '../prompt';
import { collapsedSummary, toTurns } from '../turns/live';
import { TurnStream } from '../turns';
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
  /*
   * The session's turns, straight out of the log.
   *
   * Until this change the stream was handed `DEMO_TURNS` - a hardcoded prototype turn - so a real run
   * happened behind a window that showed the same fiction every time. `turns` here is the reducer's
   * projection of `TurnStarted`/`TurnDelta`/`ToolCall*`/`TurnCompleted`/`ErrorRaised`, which is what
   * the daemon actually pushed, and `live.ts` only reshapes it.
   */
  const turns = useAppStore((state) => state.turns);
  const streamTurns = toTurns(turns, session.id);
  const collapsed = collapsedSummary(turns, session.id);

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
          {streamTurns.length === 0 ? (
            /* A fresh session, and the app says so instead of drawing someone else's conversation. */
            <div
              className="pane-empty grid place-items-center py-[56px] text-center"
              id="paneEmpty"
            >
              <div>
                <span className="mx-auto mb-[12px] grid h-[38px] w-[38px] place-items-center rounded-lg border border-border-subtle bg-bg-raised text-text-muted">
                  <MessageSquare size={17} aria-hidden="true" />
                </span>
                <p className="text-[13.5px] font-medium text-text-primary">{strings.main.emptyPane.title}</p>
                <p className="mx-auto mt-[6px] max-w-[360px] text-[12.5px] text-text-secondary">
                  {strings.main.emptyPane.body}
                </p>
              </div>
            </div>
          ) : (
            <TurnStream turns={streamTurns} collapsed={collapsed} />
          )}
        </div>
      </div>

      <PromptArea />
    </div>
  );
}
