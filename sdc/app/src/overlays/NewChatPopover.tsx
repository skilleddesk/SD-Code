import { useAppStore } from '../store/store';
import { useOverlayStore } from '../store/overlays';
import { newChatOnHost } from '../store/intents';
import { usePrefsStore } from '../store/prefs';
import { HostIcon } from '../panels/ui/HostIcon';
import { HOST_STATUS_DOT_CLASS, HOST_STATUS_LABEL } from '../panels/ui/status';
import {
  POPOVER_DESC_CLASS,
  POPOVER_ITEM_CLASS,
  POPOVER_NAME_CLASS,
  Popover,
} from '../modals/Popover';
import { strings } from '../strings';

/**
 * `#newChatPopover` - "New chat on…" (spec section 9.5).
 *
 * Opened by the sidebar's `+ New chat` button, the tab strip's `+` and Ctrl+N, and aligned with the
 * trigger's left edge. One row per host, with the host icon, its name, `N chats · status` and a
 * status dot. Picking one creates a session *on that host* through the daemon (`session.open`),
 * opens its tab and puts the caret in the prompt box.
 *
 * The hosts are the event log's - so a host that just connected appears here without anything
 * telling this component about it - while the tab it opens is a UI preference. That split is the
 * whole point of spec section 3.3, and this popover is where you can see it: the row list is
 * server truth, the tab strip is taste.
 */
export function NewChatPopover() {
  const anchor = useOverlayStore((state) => state.newChat);
  const close = useOverlayStore((state) => state.closeNewChat);
  const hosts = useAppStore((state) => state.hosts);

  const startOn = (hostId: string): void => {
    close();

    void newChatOnHost(hostId).then((sessionId) => {
      if (sessionId === null) {
        return;
      }

      const prefs = usePrefsStore.getState();

      prefs.openTab(sessionId);
      prefs.setActiveHost(hostId);

      /* The pane renders a tick later; a macrotask is enough and does not depend on a frame clock. */
      window.setTimeout(() => {
        document.querySelector<HTMLTextAreaElement>('.prompt-box textarea')?.focus();
      }, 0);
    });
  };

  return (
    <Popover anchor={anchor} title={strings.popover.newChatTitle} onClose={close}>
      {hosts.map((host) => (
        <div
          key={host.id}
          className={POPOVER_ITEM_CLASS}
          role="button"
          tabIndex={0}
          data-new-chat-host={host.id}
          onClick={() => startOn(host.id)}
          onKeyDown={(event) => {
            if (event.key === 'Enter') {
              startOn(host.id);
            }
          }}
        >
          <HostIcon type={host.type} size={20} />

          <div className="pop-body min-w-0 flex-1">
            <div className={POPOVER_NAME_CLASS}>{host.name}</div>
            <div className={POPOVER_DESC_CLASS}>
              {strings.popover.chatsAndStatus(host.sessions.length, HOST_STATUS_LABEL[host.status])}
            </div>
          </div>

          <span
            className={
              'pop-status h-[7px] w-[7px] shrink-0 rounded-full ' + HOST_STATUS_DOT_CLASS[host.status]
            }
            title={HOST_STATUS_LABEL[host.status]}
          />
        </div>
      ))}
    </Popover>
  );
}
