import { Check, Plus, ShieldCheck } from 'lucide-react';

import { strings } from '../strings';
import { useOverlayStore } from '../store/overlays';
import { useSessionsStore } from '../store/sessions';
import { HostIcon } from '../panels/ui/HostIcon';
import { HOST_STATUS_DOT_CLASS, HOST_STATUS_LABEL } from '../panels/ui/status';
import {
  POPOVER_DESC_CLASS,
  POPOVER_ITEM_CLASS,
  POPOVER_NAME_CLASS,
  Popover,
} from './Popover';

/**
 * The host switcher popover - the topbar's host pill opening, and the status bar's host item
 * (spec sections 7.1 row 3 and 7.15: "every item is clickable").
 *
 * One row per host: its icon, its name, `N chats · status` in mono, and the status dot. The row
 * that is already active carries a check, so "which host am I on" is answerable without reading
 * the topbar. Choosing a host switches the pill immediately, and - because a host is only
 * interesting because of what is running on it - opens that host's topmost session if it has one.
 *
 * The trailing `+` is the same action as `.host-add` in the sidebar (7.3): start a new chat there.
 * It is a separate hit target, so it stops the row's own click.
 */
export function HostSwitcherPopover() {
  const anchor = useOverlayStore((state) => state.hostSwitcher);
  const close = useOverlayStore((state) => state.closeHostSwitcher);
  const openAddHost = useOverlayStore((state) => state.openAddHost);
  const { hosts, activeHostId, openSession, setActiveHost, newChatOnHost } = useSessionsStore();

  const chooseHost = (hostId: string, firstSessionId: string | undefined): void => {
    setActiveHost(hostId);

    if (firstSessionId !== undefined) {
      openSession(firstSessionId);
    }

    close();
  };

  return (
    <Popover anchor={anchor} title={strings.popover.hostSwitcherTitle} onClose={close}>
      {hosts.map((host) => (
        <div
          key={host.id}
          className={POPOVER_ITEM_CLASS}
          role="button"
          tabIndex={0}
          onClick={() => chooseHost(host.id, host.sessions[0]?.id)}
          onKeyDown={(event) => {
            if (event.key === 'Enter') {
              chooseHost(host.id, host.sessions[0]?.id);
            }
          }}
        >
          <HostIcon type={host.type} size={20} />

          <div className="pop-body flex-1 min-w-0">
            <div className={POPOVER_NAME_CLASS}>{host.name}</div>
            <div className={POPOVER_DESC_CLASS}>
              {strings.popover.chatsAndStatus(
                host.sessions.length,
                HOST_STATUS_LABEL[host.status],
              )}
            </div>
            {/* The address a host was added with (0.7.13): `user@host:8443` - the port is the fact 0.7.0
                threw away, and this is where a person checks that the row says what they meant. */}
            {host.address === '' ? null : (
              <div className="pop-address truncate font-mono text-[10.5px] text-text-muted" title={host.address}>
                {host.address}
              </div>
            )}
          </div>

          {/*
            The way back to a host's key and its own environment (0.7.13) - the surface that answers
            *"host key changed — needs re-pin"* long after the dialog that added the host has closed.
            Every host gets it, because every host has a key; a `local` host's card simply has nothing
            to pin, and the dialog says so by having no fingerprint to show.
          */}
          <button
            type="button"
            className="pop-keys grid h-[20px] w-[20px] place-items-center rounded-sm text-text-muted transition-all duration-fast ease-ease hover:bg-bg-active hover:text-text-primary"
            title={strings.popover.keysAndDoctor}
            aria-label={strings.popover.keysAndDoctor}
            onClick={(event) => {
              event.stopPropagation();
              close();
              openAddHost(host.id);
            }}
          >
            <ShieldCheck size={12} aria-hidden="true" />
          </button>

          <button
            type="button"
            className="grid h-[20px] w-[20px] place-items-center rounded-sm text-text-muted transition-all duration-fast ease-ease hover:bg-bg-active hover:text-text-primary"
            title={strings.sidebar.actions.newChatOnHost}
            aria-label={strings.sidebar.actions.newChatOnHost}
            onClick={(event) => {
              event.stopPropagation();
              newChatOnHost(host.id);
              close();
            }}
          >
            <Plus size={12} aria-hidden="true" />
          </button>

          <span
            className={'pop-status h-[7px] w-[7px] shrink-0 rounded-full ' + HOST_STATUS_DOT_CLASS[host.status]}
            title={HOST_STATUS_LABEL[host.status]}
          />

          {host.id === activeHostId ? (
            <Check size={14} className="shrink-0 text-accent" aria-hidden="true" />
          ) : null}
        </div>
      ))}
    </Popover>
  );
}
