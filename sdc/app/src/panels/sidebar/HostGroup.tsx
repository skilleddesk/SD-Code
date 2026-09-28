import { useState } from 'react';
import { ChevronDown, Folder, FolderOpen, FolderPlus, MessageSquare, Plus, ServerOff, X } from 'lucide-react';

import { strings } from '../../strings';
import { closeFolder, openFolder, removeHost } from '../../store/intents';
import { useOverlayStore } from '../../store/overlays';
import { useAppStore } from '../../store/store';
import type { ProjectView } from '../../store/types';
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
  const listed = useAppStore((state) => state.projects).filter((project) => project.hostId === host.id);
  /* A chat bound to a folder the project list has not brought yet (it loads after the chats) still sits
     under its folder, named from its own root - never loose for a moment and then jumping. */
  const projects: ProjectView[] = [
    ...listed,
    ...host.sessions
      .filter((session) => session.projectId != null && !listed.some((project) => project.id === session.projectId))
      .filter((session, index, all) => all.findIndex((other) => other.projectId === session.projectId) === index)
      .map((session) => ({
        id: session.projectId ?? '',
        hostId: host.id,
        root: session.projectRoot ?? '',
        name: (session.projectRoot ?? '').split(/[\\/]/).filter((part) => part !== '').pop() ?? session.title,
        chats: 0,
      })),
  ];
  const openRemoteFolder = useOverlayStore((state) => state.openRemoteFolder);
  /* A local folder comes from this machine's own picker; a VPS folder from the host's browser. */
  const addFolder = (): void => {
    if (host.type === 'local') {
      void openFolder(host.id);
    } else {
      openRemoteFolder(host.id);
    }
  };
  /* Each project with its own chats (0.12.5): "project base alada chat ... multiple vps, project thakle
     everytar jonno alada hobe". A project with no chat yet still shows, so a chat can be started in it. */
  const groups = projects
    .map((project) => ({ project, sessions: visible.filter((session) => session.projectId === project.id) }))
    .filter((group) => filter.trim() === '' || group.sessions.length > 0)
    .sort((left, right) => left.project.name.localeCompare(right.project.name));
  const loose = visible.filter((session) => !projects.some((project) => project.id === session.projectId));

  /**
   * `host.remove` - spec section 9.12's other half, and the control that was missing.
   *
   * The confirmation names what goes with the host (its chats), because that is the part a person
   * cannot see from the sidebar: a host with a collapsed group looks empty either way.
   */
  const remove = (): void => {
    if (window.confirm(strings.sidebar.removeHostConfirm(host.name, host.sessions.length))) {
      void removeHost(host.id, host.name);
    }
  };

  return (
    <div className={'host-group mb-[2px]' + (collapsed ? ' collapsed' : '')} data-host={host.id}>
      {/*
        The header is a **container**, and the toggle is a real button inside it (0.7.12).

        It used to be a `div role="button"` wrapping the whole row - including the `+` and the remove button,
        which are buttons themselves. `nested-interactive` is what axe calls that, and it is a real problem
        rather than a rule for its own sake: a button inside a button is not reachable by keyboard, and a screen
        reader announces one control where there are three. The audit only *found* it once the app opened with
        a host that had chats in it, which is why the first green run was green.
      */}
      <div className="host-header group flex min-w-0 items-center gap-[8px] rounded-md py-[6px] pr-[6px] pl-[4px] text-[11.5px] text-text-secondary transition-colors duration-fast ease-ease hover:bg-bg-hover hover:text-text-primary">
        <button
          type="button"
          className="host-toggle flex min-w-0 flex-1 items-center gap-[8px] text-left"
          aria-expanded={!collapsed}
          aria-label={strings.sidebar.actions.toggleHost(host.name)}
          onClick={() => toggleHostCollapsed(host.id)}
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
        </button>
        <button
          type="button"
          className="host-folder grid h-[18px] w-[18px] shrink-0 place-items-center rounded-sm text-text-muted opacity-0 transition-all duration-fast ease-ease group-hover:opacity-100 hover:bg-bg-active hover:text-text-primary"
          title={strings.sidebar.actions.openFolder}
          aria-label={strings.sidebar.actions.openFolder}
          onClick={addFolder}
        >
          <FolderPlus size={11} aria-hidden="true" />
        </button>
        <button
          type="button"
          className="host-add grid h-[18px] w-[18px] shrink-0 place-items-center rounded-sm text-text-muted opacity-0 transition-all duration-fast ease-ease group-hover:opacity-100 hover:bg-bg-active hover:text-text-primary"
          title={strings.sidebar.actions.newChatOnHost}
          aria-label={strings.sidebar.actions.newChatOnHost}
          onClick={() => newChatOnHost(host.id)}
        >
          <Plus size={11} aria-hidden="true" />
        </button>

        {/* `local` is the machine this daemon runs on and the daemon refuses to remove it, so the
            button is not offered for it: a control whose only outcome is an error is worse than no
            control. Every other host gets one, because a host added by mistake (or added three times
            under the same name) used to be permanent. */}
        {host.id === 'local' ? null : (
          <button
            type="button"
            className="host-remove grid h-[18px] w-[18px] shrink-0 place-items-center rounded-sm text-text-muted opacity-0 transition-all duration-fast ease-ease group-hover:opacity-100 hover:bg-red-subtle hover:text-state-error"
            title={strings.sidebar.actions.removeHost}
            aria-label={strings.sidebar.actions.removeHost}
            onClick={(event) => {
              event.stopPropagation();
              remove();
            }}
          >
            <ServerOff size={11} aria-hidden="true" />
          </button>
        )}
      </div>

      {collapsed ? null : (
        <div className="host-sessions ml-[10px] border-l border-border-subtle pl-[4px]">
          {/* 1. The host's projects (sites, apps, folders), each with its own chats (0.12.5). */}
          {groups.map(({ project, sessions }) => (
            <ProjectGroup
              key={project.id}
              project={project}
              hostId={host.id}
              sessions={sessions}
              activeTab={activeTab}
              onNewChat={() => newChatOnHost(host.id, project.id)}
            />
          ))}

          {/* 2. Chats that belong to no folder - the host's own chats. */}
          {loose.length > 0 ? (
            <>
              {groups.length > 0 ? (
                <div className="flex items-center gap-[6px] px-[10px] pb-[2px] pt-[8px] text-[9.5px] font-semibold uppercase tracking-[.08em] text-text-muted">
                  <MessageSquare size={10} aria-hidden="true" />
                  {strings.sidebar.hostChats}
                </div>
              ) : null}
              {loose.map((session) => (
                <SessionRow key={session.id} session={session} active={session.id === activeTab} />
              ))}
            </>
          ) : null}

          {/* 3. The two ways to begin, always in view: a chat on this machine, or a folder to work in. */}
          <div className="host-actions flex gap-[4px] px-[6px] pb-[6px] pt-[4px]">
            <button
              type="button"
              className="host-new-chat flex min-w-0 flex-1 items-center justify-center gap-[5px] rounded-md border border-dashed border-border-subtle py-[4px] text-[10.5px] text-text-muted transition-colors duration-fast ease-ease hover:border-solid hover:border-border-default hover:bg-bg-hover hover:text-text-primary"
              title={strings.sidebar.actions.newChatOnHost}
              onClick={() => newChatOnHost(host.id, null)}
            >
              <Plus size={10} aria-hidden="true" />
              <span className="truncate">{strings.sidebar.newChatShort}</span>
            </button>
            <button
              type="button"
              className="host-open-folder flex min-w-0 flex-1 items-center justify-center gap-[5px] rounded-md border border-dashed border-border-subtle py-[4px] text-[10.5px] text-text-muted transition-colors duration-fast ease-ease hover:border-solid hover:border-border-default hover:bg-bg-hover hover:text-text-primary"
              title={strings.sidebar.actions.openFolder}
              onClick={addFolder}
            >
              <FolderPlus size={10} aria-hidden="true" />
              <span className="truncate">{strings.sidebar.openFolderRow}</span>
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

/**
 * One project under its host (0.12.5): the folder's name and its chat count, a `+` for a new chat **in this
 * folder**, a way to close the folder (its chats are kept), and its chats under a guide line.
 */
function ProjectGroup({
  project,
  hostId,
  sessions,
  activeTab,
  onNewChat,
}: {
  project: ProjectView;
  hostId: string;
  sessions: readonly Host['sessions'][number][];
  activeTab: string | null;
  onNewChat: () => void;
}) {
  /* A folder with no chat yet starts folded: its row and `+` are enough until it has one. */
  const [folded, setFolded] = useState(sessions.length === 0);
  const close = (): void => {
    if (window.confirm(strings.sidebar.closeFolderConfirm(project.name, sessions.length))) {
      void closeFolder(project.id, project.name);
    }
  };

  return (
    <div className="project-group mb-[1px]" data-project={project.id} data-project-host={hostId}>
      <div className="project-header group/project flex min-w-0 items-center gap-[6px] rounded-md py-[4px] pr-[4px] pl-[6px] text-[11.5px] text-text-secondary hover:bg-bg-hover hover:text-text-primary">
        <button
          type="button"
          className="flex min-w-0 flex-1 items-center gap-[6px] text-left"
          aria-expanded={!folded}
          aria-label={strings.sidebar.actions.toggleProject(project.name)}
          title={project.root}
          onClick={() => setFolded(!folded)}
        >
          <ChevronDown
            size={10}
            aria-hidden="true"
            className={'shrink-0 text-text-muted transition-transform duration-200 ease-ease ' + (folded ? '-rotate-90' : '')}
          />
          {folded ? (
            <Folder size={12} aria-hidden="true" className="shrink-0 text-accent" />
          ) : (
            <FolderOpen size={12} aria-hidden="true" className="shrink-0 text-accent" />
          )}
          <span className="min-w-0 flex-1 truncate font-medium">{project.name}</span>
          <span className="shrink-0 font-mono text-[9.5px] text-text-muted group-hover/project:hidden">{sessions.length}</span>
        </button>
        <button
          type="button"
          className="project-add grid h-[18px] w-[18px] shrink-0 place-items-center rounded-sm text-text-muted opacity-0 transition-all duration-fast ease-ease group-hover/project:opacity-100 hover:bg-bg-active hover:text-text-primary focus-visible:opacity-100"
          title={strings.sidebar.actions.newChatInProject(project.name)}
          aria-label={strings.sidebar.actions.newChatInProject(project.name)}
          onClick={onNewChat}
        >
          <Plus size={11} aria-hidden="true" />
        </button>
        <button
          type="button"
          className="project-close grid h-[18px] w-[18px] shrink-0 place-items-center rounded-sm text-text-muted opacity-0 transition-all duration-fast ease-ease group-hover/project:opacity-100 hover:bg-red-subtle hover:text-state-error focus-visible:opacity-100"
          title={strings.sidebar.actions.closeFolder}
          aria-label={`${strings.sidebar.actions.closeFolder} ${project.name}`}
          onClick={close}
        >
          <X size={11} aria-hidden="true" />
        </button>
      </div>

      {folded ? null : (
        <div className="project-sessions ml-[12px] border-l border-border-subtle pl-[4px]">
          {sessions.map((session) => (
            <SessionRow key={session.id} session={session} active={session.id === activeTab} />
          ))}
          <button
            type="button"
            className="project-new-chat flex w-full items-center gap-[6px] rounded-md py-[4px] pr-[10px] pl-[10px] text-left text-[10.5px] text-text-muted hover:bg-bg-hover hover:text-text-secondary"
            onClick={onNewChat}
          >
            <Plus size={10} aria-hidden="true" />
            {strings.sidebar.actions.newChatInProject(project.name)}
          </button>
        </div>
      )}
    </div>
  );
}
