import { Plus, Search, ServerCog } from 'lucide-react';

import { strings } from '../../strings';
import { anchorBelow, useOverlayStore } from '../../store/overlays';
import { useSessionsStore } from '../../store/sessions';
import { HostGroup } from './HostGroup';

/**
 * The sidebar region - spec section 7.3, top to bottom:
 *
 *   .sidebar-actions   `+ New chat` (full width, accent, with the ⌘N hint)
 *   .sidebar-search    `Filter chats…`
 *   .sidebar-scroll    the host-grouped session tree, `#hostsList`
 *   .sidebar-bottom    `+ Add host (VPS or local)`, dashed until you hover it
 *
 * The region element (`.sidebar`) is the shell's - 280px wide, collapses to 0 for Ctrl+B, becomes a
 * drawer below 900px - so this component only fills it (src/layout/Shell.css).
 *
 * The filter is store state rather than local state because the session store is the only thing
 * that knows how a session matches: a row is visible when its title or its first prompt contains
 * the query, and the host headers never filter away (spec section 7.3).
 *
 * `#newChatBtn` opens the host picker (spec section 9.5) rather than creating a session outright,
 * because a new chat needs to know *where* - and `#addHostBtn` opens the Add host dialog, which is a
 * later step: the overlay flag flips so the surface has somewhere to mount, and a toast answers the
 * click for now.
 */
export function Sidebar() {
  const { hosts, activeTab, filter, collapsedHosts, filterSessions } = useSessionsStore();
  const openNewChat = useOverlayStore((state) => state.openNewChat);
  const openAddHost = useOverlayStore((state) => state.openAddHost);

  return (
    <aside className="sidebar" id="sidebar">
      <div className="sidebar-actions flex gap-[6px] px-[10px] pb-[6px] pt-[10px]">
        <button
          type="button"
          id="newChatBtn"
          className="new-chat-btn flex flex-1 items-center gap-[8px] rounded-md bg-accent px-[12px] py-[8px] text-[12.5px] font-semibold text-text-on-accent shadow-sm transition-all duration-fast ease-ease hover:bg-accent-hover hover:shadow-[0_3px_12px_var(--accent-glow)] active:scale-[.98]"
          onClick={(event) => openNewChat(anchorBelow(event.currentTarget))}
        >
          <Plus size={14} aria-hidden="true" />
          {strings.sidebar.newChat}
          <span className="kbd-lite ml-auto rounded-[3px] bg-[rgba(255,255,255,.16)] px-[5px] py-[1px] font-mono text-[9.5px] font-medium max-900:hidden">
            {strings.sidebar.newChatShortcut}
          </span>
        </button>
      </div>

      <div className="sidebar-search px-[10px] pb-[8px] pt-[4px]">
        <div className="search-wrap flex items-center gap-[8px] rounded-md border border-border-subtle bg-bg-input px-[10px] py-[6px] transition-all duration-fast ease-ease focus-within:border-border-strong">
          <Search size={13} aria-hidden="true" className="text-text-muted" />
          <input
            type="text"
            id="sidebarSearch"
            /* `bg-transparent` is explicit even though `globals.css` now defaults a control to no
               background of its own: this input is a hole in `.search-wrap`'s `bg-bg-input`, and the
               class says so where a reader is looking. Before the base reset this element had no
               background at all and the browser painted it white. */
            className="min-w-0 flex-1 bg-transparent text-[12px] text-text-primary placeholder:text-text-muted"
            placeholder={strings.sidebar.filterPlaceholder}
            value={filter}
            onChange={(event) => filterSessions(event.target.value)}
            aria-label={strings.sidebar.filterPlaceholder}
          />
        </div>
      </div>

      <div className="sidebar-scroll flex-1 overflow-y-auto overflow-x-hidden px-[8px] pb-[16px] pt-[4px]" id="hostsList">
        {hosts.map((host) => (
          <HostGroup
            key={host.id}
            host={host}
            filter={filter}
            activeTab={activeTab}
            collapsed={collapsedHosts[host.id] === true}
          />
        ))}
      </div>

      <div className="sidebar-bottom shrink-0 border-t border-border-subtle px-[10px] py-[8px]">
        <button
          type="button"
          id="addHostBtn"
          className="add-host-btn flex w-full items-center justify-center gap-[8px] rounded-md border border-dashed border-border-default bg-transparent p-[8px] text-[11.5px] text-text-secondary transition-all duration-fast ease-ease hover:border-solid hover:border-border-strong hover:bg-bg-hover hover:text-text-primary"
          onClick={() => openAddHost()}
        >
          <ServerCog size={13} aria-hidden="true" />
          {strings.sidebar.addHost}
        </button>
      </div>
    </aside>
  );
}
