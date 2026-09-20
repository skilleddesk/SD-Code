import { version as APP_VERSION } from '../../../package.json';
import { strings } from '../../strings';
import { useLayoutStore } from '../../store/layout';
import { anchorBelow, useOverlayStore } from '../../store/overlays';
import { useModelStore } from '../../store/model';
import { connectedProviderCount, useProviderStore } from '../../store/providers';
import {
  connectionState,
  findSession,
  sessionCount,
  useSessionsStore,
} from '../../store/sessions';
import { toast } from '../../store/toast';
import { CONNECTION_DOT_CLASS, HOST_STATUS_DOT_CLASS, HOST_STATUS_LABEL } from '../ui/status';
import { StatusItem } from './StatusItem';

/**
 * The status bar region - spec section 7.15.
 *
 *   ● prod-1 · claude_code · sonnet · 3 providers · 3 chats · 3 hosts · ● ready
 *
 * The dot comes first and the connection state comes last, and both are the same fact read at two
 * distances: the state of the fleet. Everything between them is what the app is doing right now -
 * which host, which engine, which model, and how much is connected.
 *
 * Every segment is a button, and each one goes where the number comes from:
 *
 *   host          the host switcher popover (the same one the topbar's pill opens)
 *   engine        the model dropdown, because the engine is one third of that choice
 *   model         the same dropdown
 *   providers     the Provider Hub
 *   chats         the search overlay, which is the list of chats
 *   hosts         the host switcher again
 *   ready         a toast naming the full connection state
 *
 * Shrinking the window hides segments from the right inward: `hide-sm` at 900px takes the three
 * counters, and 520px leaves nothing but the connection (spec section 7.15). The separators carry
 * the same classes so no stray `·` is left pointing at nothing - which is the difference between the
 * spec's "only the connection" and the prototype's `:not(.conn)` rule, that leaves the `·`s behind.
 */
export function StatusBar() {
  const { hosts, activeTab, activeHostId } = useSessionsStore();
  const model = useModelStore();
  const providers = useProviderStore((state) => state.providers);
  const mode = useLayoutStore((state) => state.mode);
  const openHostSwitcher = useOverlayStore((state) => state.openHostSwitcher);
  const openHub = useOverlayStore((state) => state.openHub);
  const openSearch = useOverlayStore((state) => state.openSearch);

  const activeHost =
    findSession(hosts, activeTab)?.host ??
    hosts.find((host) => host.id === activeHostId) ??
    hosts[0];

  const connection = connectionState(hosts);
  const connectionLabel =
    connection === 'success'
      ? strings.statusBar.connection.ready
      : connection === 'waiting'
        ? strings.statusBar.connection.degraded
        : strings.statusBar.connection.offline;

  return (
    <footer className="statusbar">
      <StatusItem
        id="statusHost"
        dotClass={HOST_STATUS_DOT_CLASS[activeHost?.status ?? 'connected']}
        title={strings.topbar.activeHostTitle}
        hideTiny
        onClick={(event) => openHostSwitcher(anchorBelow(event.currentTarget))}
      >
        {activeHost?.name ?? ''}
      </StatusItem>

      {/*
        The build this window is running.
        It is here because "is my install actually updated?" was a question nobody could answer from
        inside the app: the only place the version existed was the About tab, and a user looking at a
        white box in the sidebar has no reason to open an About tab. Both halves are shown, because
        they can differ - the app and the daemon ship together, and a stale daemon left listening on
        the port is exactly the case where the window is new and the thing doing the work is not.
        Clicking it copies the pair, which is what a bug report needs.
      */}
      <StatusItem
        id="statusVersion"
        tone="text-text-muted"
        title={strings.statusBar.versionTitle(APP_VERSION, activeHost?.sdcd ?? '')}
        hideSmall
        onClick={() => {
          const pair = strings.statusBar.version(APP_VERSION, activeHost?.sdcd ?? '');

          void navigator.clipboard?.writeText(pair);
          toast(strings.statusBar.versionCopied(APP_VERSION, activeHost?.sdcd ?? ''));
        }}
      >
        {strings.statusBar.version(APP_VERSION, activeHost?.sdcd ?? '')}
      </StatusItem>

      <span className="sep text-border-strong max-520:hidden" aria-hidden="true">
        ·
      </span>

      <StatusItem
        id="statusEngine"
        tone="engine text-accent"
        title={strings.prompt.model.groupTitles.engine}
        hideTiny
        onClick={() => model.toggleDropdown()}
      >
        {model.engine}
      </StatusItem>

      <span className="sep text-border-strong max-520:hidden" aria-hidden="true">
        ·
      </span>

      <StatusItem
        id="statusModel"
        tone="model text-text-primary"
        title={strings.prompt.model.groupTitles.model}
        hideTiny
        onClick={() => model.toggleDropdown()}
      >
        {model.model}
      </StatusItem>

      <span className="sep hide-sm text-border-strong max-900:hidden" aria-hidden="true">
        ·
      </span>

      <StatusItem
        hideSmall
        hideTiny
        title={strings.topbar.providers.title}
        onClick={() => openHub()}
      >
        <span id="statProviders">{connectedProviderCount(providers)}</span> {strings.statusBar.providers}
      </StatusItem>

      <div className="spacer flex-1 min-w-[4px]" />

      <StatusItem
        hideSmall
        hideTiny
        title={strings.topbar.palette.title}
        onClick={() => openSearch()}
      >
        <span id="statSessions">{sessionCount(hosts)}</span> {strings.statusBar.chats} ·{' '}
        <span id="statHosts">{hosts.length}</span> {strings.statusBar.hosts}
      </StatusItem>

      <span className="sep hide-sm text-border-strong max-900:hidden" aria-hidden="true">
        ·
      </span>

      <StatusItem
        dotClass={CONNECTION_DOT_CLASS[connection]}
        tone={'conn ' + (connection === 'error' ? 'text-state-error' : connection === 'waiting' ? 'text-state-waiting' : 'text-state-success')}
        title={`${strings.topbar.modes[mode]} · ${HOST_STATUS_LABEL[activeHost?.status ?? 'connected']}`}
        onClick={() => toast(`${connectionLabel} · ${activeHost?.name ?? ''}`)}
      >
        {connectionLabel}
      </StatusItem>
    </footer>
  );
}
