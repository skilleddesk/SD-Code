import { ChevronDown, Plus } from 'lucide-react';

import { strings } from '../../strings';
import { matchesFilter, orderedSessions, useSessionsStore, type Host } from '../../store/sessions';
import { HostIcon } from '../ui/HostIcon';
import { HOST_STATUS_CLASS, HOST_STATUS_LABEL } from '../ui/status';
import { SessionRow } from './SessionRow';

/**
 * `.host-group` - one host and its sessions (spec section 7.3).
 *
 * The header is a row of six things: the collapse chevron, the 18x18 host icon, the name (which
 * ellipsises), the state dot, a mono count pill, and a `+` that only exists on hover. Clicking
 * anywhere on it collapses the group; the chevron rotates -90deg while collapsed and the session
 * list disappears. The `+` stops the click, because it means "new chat on this host", not
 * "collapse".
 *
 * Two rules decide what the session list contains:
 *
 *   Waiting first (spec gap #12)  `orderedSessions()` floats blocked sessions to the top. It is a
 *                                 display-time sort, so the store keeps the host's own order.
 *   The filter                   rows that do not match `Filter chats…` are dropped; the host
 *                                 header stays, which is what the spec asks for (section 7.3).
 *
 * A host with no sessions at all shows `+ Start a chat` in italics instead of an empty list. That
 * is a different case from "filtered down to nothing", and it is decided on the raw count so the
 * two do not get confused.
 */
export interface HostGroupProps {
  host: Host;
  filter: string;
  activeTab: string | null;
  collapsed: boolean;
}

export function HostGroup({ host, filter, activeTab, collapsed }: HostGroupProps) {
  const { toggleHostCollapsed, newChatOnHost } = useSessionsStore();

  const visible = orderedSessions(host.sessions).filter((session) => matchesFilter(session, filter));

  return (
    <div className={'host-group mb-[2px]' + (collapsed ? ' collapsed' : '')} data-host={host.id}>
      <div
        className="host-header group flex min-w-0 cursor-pointer select-none items-center gap-[8px] rounded-md py-[6px] pr-[6px] pl-[4px] text-[11.5px] text-text-secondary transition-colors duration-fast ease-ease hover:bg-bg-hover hover:text-text-primary"
        role="button"
        tabIndex={0}
        aria-expanded={!collapsed}
        onClick={() => toggleHostCollapsed(host.id)}
        onKeyDown={(event) => {
          if (event.key === 'Enter') {
            toggleHostCollapsed(host.id);
          }
        }}
      >
        <ChevronDown
          size={11}
          aria-hidden="true"
          className={
            'host-chev shrink-0 text-text-muted transition-transform duration-200 ease-ease ' +
            (collapsed ? '-rotate-90' : '')
          }
        />
        <HostIcon type={host.type} />
        <span className="host-name min-w-0 flex-1 overflow-hidden text-ellipsis whitespace-nowrap font-medium">
          {host.name}
        </span>
        <span
          className={'host-status h-[7px] w-[7px] shrink-0 rounded-full ' + HOST_STATUS_CLASS[host.status]}
          title={HOST_STATUS_LABEL[host.status]}
        />
        <span className="host-count shrink-0 rounded-full border border-border-subtle bg-bg-raised px-[6px] py-[1px] font-mono text-[9.5px] font-medium text-text-muted">
          {host.sessions.length}
        </span>
        <button
          type="button"
          className="host-add grid h-[18px] w-[18px] shrink-0 place-items-center rounded-sm text-text-muted opacity-0 transition-all duration-fast ease-ease group-hover:opacity-100 hover:bg-bg-active hover:text-text-primary"
          title={strings.sidebar.actions.newChatOnHost}
          aria-label={strings.sidebar.actions.newChatOnHost}
          onClick={(event) => {
            event.stopPropagation();
            newChatOnHost(host.id);
          }}
        >
          <Plus size={11} aria-hidden="true" />
        </button>
      </div>

      {collapsed ? null : (
        <div className="host-sessions">
          {host.sessions.length === 0 ? (
            <div
              className="host-empty flex cursor-pointer items-center gap-[6px] rounded-md py-[8px] pr-[12px] pl-[26px] text-[11.5px] italic text-text-muted transition-all duration-fast ease-ease hover:bg-bg-hover hover:not-italic hover:text-text-secondary"
              role="button"
              tabIndex={0}
              onClick={() => newChatOnHost(host.id)}
              onKeyDown={(event) => {
                if (event.key === 'Enter') {
                  newChatOnHost(host.id);
                }
              }}
            >
              <Plus size={11} aria-hidden="true" />
              {strings.sidebar.startChat}
            </div>
          ) : (
            visible.map((session) => (
              <SessionRow
                key={session.id}
                session={session}
                active={session.id === activeTab}
              />
            ))
          )}
        </div>
      )}
    </div>
  );
}
