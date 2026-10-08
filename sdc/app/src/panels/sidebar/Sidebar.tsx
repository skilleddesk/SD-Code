import { useState } from 'react';
import { ChevronRight, FolderPlus, Loader2, MessageSquarePlus, MoreHorizontal, Plus, Search, ServerCog, X } from 'lucide-react';

import { strings } from '../../strings';
import { closeFolder, openFolder, removeHost } from '../../store/intents';
import { useOverlayStore } from '../../store/overlays';
import { usePrefsStore } from '../../store/prefs';
import { matchesFilter, orderedSessions, useSessionsStore, type Host, type Session } from '../../store/sessions';
import { useAppStore } from '../../store/store';
import type { ProjectView } from '../../store/types';
import { HostIcon } from '../ui/HostIcon';
import { HOST_STATUS_CLASS, HOST_STATUS_LABEL } from '../ui/status';
import { FilesSection } from './FilesSection';
import { SessionRow } from './SessionRow';

/**
 * The sidebar (0.12.5 redesign): one machine at a time, its projects, each project's chats.
 *
 * The report: *"side bar ar style and sytem ta valo lagse nah … new design"*. The old tree put every host,
 * every folder and every chat in one long indented list, with the actions hidden until a hover. Now:
 *
 *   New chat + search           always at the top; New chat opens in the machine and project in view
 *   machine tabs                Local and every VPS as a tab - status, chat count, a spinner while one works
 *   Projects                    the machine's folders (sites, apps), each with its own chats and New chat
 *   Chats                       the machine's chats that belong to no folder
 *   Files                       the open chat's folder, as before
 *
 * A search looks through every machine's chats at once, so nothing is out of reach behind a tab.
 */
export function Sidebar() {
  const { hosts, activeTab, filter, filterSessions, newChatOnHost } = useSessionsStore();
  const activeHostId = usePrefsStore((state) => state.activeHostId);
  const openAddHost = useOverlayStore((state) => state.openAddHost);
  const openRemoteFolder = useOverlayStore((state) => state.openRemoteFolder);
  const allProjects = useAppStore((state) => state.projects);
  const host = hosts.find((candidate) => candidate.id === activeHostId) ?? hosts[0];
  const searching = filter.trim() !== '';

  const addFolder = (target: Host): void => {
    if (target.type === 'local') {
      void openFolder(target.id);
    } else {
      openRemoteFolder(target.id);
    }
  };

  return (
    <aside className="sidebar flex flex-col" id="sidebar">
      <div className="sidebar-top flex flex-col gap-[8px] px-[10px] pb-[8px] pt-[10px]">
        <button
          type="button"
          id="newChatBtn"
          className="new-chat-btn brand-fill flex items-center gap-[8px] rounded-lg bg-accent-fill px-[12px] py-[9px] text-[12.5px] font-semibold text-text-on-accent"
          onClick={() => host !== undefined && newChatOnHost(host.id)}
        >
          <MessageSquarePlus size={15} aria-hidden="true" />
          {strings.sidebar.newChat}
          <span className="kbd-lite ml-auto rounded-[3px] bg-black/20 px-[5px] py-[1px] font-mono text-[9.5px] font-medium max-900:hidden">
            {strings.sidebar.newChatShortcut}
          </span>
        </button>

        <div className="search-wrap flex items-center gap-[8px] rounded-lg border border-border-subtle bg-bg-input px-[10px] py-[6px] transition-all duration-fast ease-ease focus-within:border-border-strong">
          <Search size={13} aria-hidden="true" className="text-text-muted" />
          <input
            type="text"
            id="sidebarSearch"
            className="min-w-0 flex-1 bg-transparent text-[12px] text-text-primary placeholder:text-text-muted"
            placeholder={strings.sidebar.filterPlaceholder}
            value={filter}
            onChange={(event) => filterSessions(event.target.value)}
            aria-label={strings.sidebar.filterPlaceholder}
          />
          {searching ? (
            <button type="button" className="text-text-muted hover:text-text-primary" aria-label={strings.sidebar.clearSearch} onClick={() => filterSessions('')}>
              <X size={12} aria-hidden="true" />
            </button>
          ) : null}
        </div>
      </div>

      {/* The machines, as tabs. */}
      <div className="px-[18px] pb-[4px] text-[10px] font-semibold uppercase tracking-[.09em] text-text-muted">{strings.sidebar.machines}</div>
      <div className="machine-tabs flex flex-col gap-[2px] border-b border-border-subtle px-[10px] pb-[10px]">
        <div className="flex flex-col gap-[2px]" role="tablist" aria-label={strings.sidebar.machines}>
          {hosts.map((candidate) => (
            <MachineTab
              key={candidate.id}
              host={candidate}
              active={candidate.id === host?.id}
              onSelect={() => usePrefsStore.getState().setActiveHost(candidate.id)}
            />
          ))}
        </div>
        <button
          type="button"
          id="addHostBtn"
          className="flex h-[28px] w-full items-center gap-[8px] rounded-md px-[8px] text-[11.5px] text-text-muted transition-colors duration-fast hover:bg-bg-hover hover:text-text-primary"
          onClick={() => openAddHost()}
        >
          <Plus size={13} aria-hidden="true" className="ml-[1px]" />
          {strings.sidebar.addHost}
        </button>
      </div>

      <div className="sidebar-scroll flex-1 overflow-y-auto overflow-x-hidden px-[8px] pb-[16px] pt-[8px]" id="hostsList">
        {searching ? (
          <SearchResults hosts={hosts} filter={filter} activeTab={activeTab} />
        ) : host === undefined ? null : (
          <MachineView
            key={host.id}
            host={host}
            projects={allProjects.filter((project) => project.hostId === host.id)}
            activeTab={activeTab}
            onNewChat={(projectId) => newChatOnHost(host.id, projectId)}
            onOpenFolder={() => addFolder(host)}
          />
        )}

        <FilesSection />
      </div>
    </aside>
  );
}

/** One machine's tab: its icon, name, state, chat count - and a spinner while a chat on it works. */
function MachineTab({ host, active, onSelect }: { host: Host; active: boolean; onSelect: () => void }) {
  const working = host.sessions.some((session) => session.state === 'running');
  const waiting = host.sessions.some((session) => session.state === 'waiting' || session.attention !== undefined);

  return (
    <button
      type="button"
      role="tab"
      aria-selected={active}
      data-host={host.id}
      title={`${host.name} · ${HOST_STATUS_LABEL[host.status]}`}
      className={
        'machine-tab flex h-[30px] w-full min-w-0 items-center gap-[8px] rounded-md border px-[8px] text-[12px] transition-colors duration-fast ' +
        (active
          ? 'border-border-default bg-bg-active font-medium text-text-primary shadow-sm'
          : 'border-transparent text-text-secondary hover:bg-bg-hover hover:text-text-primary')
      }
      onClick={onSelect}
    >
      <HostIcon type={host.type} size={16} iconSize={9} />
      <span className="min-w-0 flex-1 truncate text-left">{host.type === 'local' ? strings.sidebar.thisComputer : shortHost(host.name)}</span>
      {working ? (
        <Loader2 size={11} aria-hidden="true" className="shrink-0 animate-spin text-accent motion-reduce:animate-none" />
      ) : (
        <span className={'h-[6px] w-[6px] shrink-0 rounded-full ' + (waiting ? 'bg-state-waiting' : HOST_STATUS_CLASS[host.status])} />
      )}
      <span className="shrink-0 font-mono text-[9.5px] text-text-muted">{host.sessions.length}</span>
    </button>
  );
}

/** `user@203.0.113.7` reads better on a tab as the address alone; the full name is the tooltip. */
function shortHost(name: string): string {
  return name.includes('@') ? (name.split('@').pop() ?? name) : name;
}

/** The machine in view: its projects with their chats, then its chats with no folder. */
function MachineView({
  host,
  projects: listed,
  activeTab,
  onNewChat,
  onOpenFolder,
}: {
  host: Host;
  projects: ProjectView[];
  activeTab: string | null;
  onNewChat: (projectId: string | null) => void;
  onOpenFolder: () => void;
}) {
  const sessions = orderedSessions(host.sessions);
  /* A chat bound to a folder the project list has not brought yet (it loads after the chats) still sits
     under its folder, named from its own root - never loose for a moment and then jumping. */
  const projects: ProjectView[] = [
    ...listed,
    ...sessions
      .filter((session) => session.projectId != null && !listed.some((project) => project.id === session.projectId))
      .filter((session, index, all) => all.findIndex((other) => other.projectId === session.projectId) === index)
      .map((session) => ({
        id: session.projectId ?? '',
        hostId: host.id,
        root: session.projectRoot ?? '',
        name: (session.projectRoot ?? '').split(/[\\/]/).filter((part) => part !== '').pop() ?? session.title,
        chats: 0,
      })),
  ].sort((left, right) => latest(sessions, right.id) - latest(sessions, left.id) || left.name.localeCompare(right.name));
  const loose = sessions.filter((session) => !projects.some((project) => project.id === session.projectId));

  const remove = (): void => {
    if (window.confirm(strings.sidebar.removeHostConfirm(host.name, host.sessions.length))) {
      void removeHost(host.id, host.name);
    }
  };

  return (
    <div className="machine-view" data-machine={host.id}>
      {host.type === 'local' ? null : (
        <div className="machine-line mb-[6px] flex min-w-0 items-center gap-[6px] px-[8px] text-[10.5px] text-text-muted">
          <span className={'h-[6px] w-[6px] shrink-0 rounded-full ' + HOST_STATUS_CLASS[host.status]} />
          <span className="min-w-0 flex-1 truncate font-mono" title={host.name}>
            {host.name} · {HOST_STATUS_LABEL[host.status]}
          </span>
          <button type="button" className="shrink-0 rounded-sm px-[4px] hover:bg-red-subtle hover:text-state-error" title={strings.sidebar.actions.removeHost} onClick={remove}>
            <ServerCog size={11} aria-hidden="true" />
          </button>
        </div>
      )}

      <SectionHeader label={strings.sidebar.projects} actionLabel={strings.sidebar.actions.openFolder} onAction={onOpenFolder} icon={FolderPlus} />

      {projects.length === 0 ? (
        <button
          type="button"
          className="mx-[4px] mb-[8px] flex w-[calc(100%-8px)] flex-col items-start gap-[3px] rounded-lg border border-dashed border-border-default px-[12px] py-[10px] text-left transition-colors duration-fast hover:border-border-strong hover:bg-bg-hover"
          onClick={onOpenFolder}
        >
          <span className="flex items-center gap-[6px] text-[12px] font-medium text-text-primary">
            <FolderPlus size={13} aria-hidden="true" className="text-accent" />
            {strings.sidebar.openFolderRow}
          </span>
          <span className="text-[11px] leading-[1.45] text-text-muted">
            {host.type === 'local' ? strings.sidebar.noProjectsLocal : strings.sidebar.noProjectsHost}
          </span>
        </button>
      ) : (
        projects.map((project) => (
          <ProjectItem
            key={project.id}
            project={project}
            sessions={sessions.filter((session) => session.projectId === project.id)}
            activeTab={activeTab}
            onNewChat={() => onNewChat(project.id)}
          />
        ))
      )}

      <SectionHeader label={strings.sidebar.hostChats} actionLabel={strings.sidebar.actions.newChatOnHost} onAction={() => onNewChat(null)} icon={Plus} />

      {loose.length === 0 ? (
        <p className="px-[10px] pb-[8px] text-[11px] italic text-text-muted">{strings.sidebar.noLooseChats}</p>
      ) : (
        loose.map((session) => <SessionRow key={session.id} session={session} active={session.id === activeTab} />)
      )}
    </div>
  );
}

/** The newest activity of a project, for the order: the one worked on last comes first. */
function latest(sessions: readonly Session[], projectId: string): number {
  const ages = sessions.filter((session) => session.projectId === projectId).map((session) => -session.minutesAgo);

  return ages.length === 0 ? -Infinity : Math.max(...ages);
}

function SectionHeader({
  label,
  actionLabel,
  onAction,
  icon: Icon,
}: {
  label: string;
  actionLabel: string;
  onAction: () => void;
  icon: typeof Plus;
}) {
  return (
    <div className="section-header mt-[4px] flex items-center gap-[6px] px-[8px] pb-[4px] pt-[6px]">
      <span className="flex-1 text-[10px] font-semibold uppercase tracking-[.09em] text-text-muted">{label}</span>
      <button
        type="button"
        className="grid h-[20px] w-[20px] place-items-center rounded-sm text-text-muted transition-colors duration-fast hover:bg-bg-hover hover:text-text-primary"
        title={actionLabel}
        aria-label={actionLabel}
        onClick={onAction}
      >
        <Icon size={12} aria-hidden="true" />
      </button>
    </div>
  );
}

/** A tint per project, from its name, so the same project always wears the same colour. */
const TINTS = [
  'bg-accent-subtle text-accent',
  'bg-purple-subtle text-purple',
  'bg-green-subtle text-state-success',
  'bg-orange-subtle text-state-waiting',
  'bg-red-subtle text-state-error',
];

function tintOf(name: string): string {
  let hash = 0;

  for (const character of name) {
    hash = (hash * 31 + character.charCodeAt(0)) >>> 0;
  }

  return TINTS[hash % TINTS.length] ?? TINTS[0] ?? '';
}

/** One project: its badge, name and chat count; open, its chats and a New chat for it. */
function ProjectItem({
  project,
  sessions,
  activeTab,
  onNewChat,
}: {
  project: ProjectView;
  sessions: readonly Session[];
  activeTab: string | null;
  onNewChat: () => void;
}) {
  const holdsActive = sessions.some((session) => session.id === activeTab);
  const [open, setOpen] = useState(holdsActive || sessions.length > 0);
  const working = sessions.some((session) => session.state === 'running');
  /* Removing a project from the list is two deliberate clicks behind a menu (0.12.6): it used to be an
     X right beside the +, and one slip closed example-shop.com. `armed` is the second click's state. */
  const [menu, setMenu] = useState(false);
  const [armed, setArmed] = useState(false);
  const close = (): void => {
    if (!armed) {
      setArmed(true);
      window.setTimeout(() => setArmed(false), 4000);

      return;
    }

    setMenu(false);
    setArmed(false);
    void closeFolder(project.id, project.name);
  };

  return (
    <div className="project-group mb-[2px]" data-project={project.id} data-project-host={project.hostId}>
      <div
        className={
          'project-header group/project flex min-w-0 items-center gap-[8px] rounded-md py-[5px] pr-[4px] pl-[6px] transition-colors duration-fast ' +
          (holdsActive && !open ? 'bg-bg-active' : 'hover:bg-bg-hover')
        }
      >
        <button
          type="button"
          className="flex min-w-0 flex-1 items-center gap-[8px] text-left"
          aria-expanded={open}
          aria-label={strings.sidebar.actions.toggleProject(project.name)}
          title={project.root}
          onClick={() => setOpen(!open)}
        >
          <span className={'grid h-[20px] w-[20px] shrink-0 place-items-center rounded-md text-[10.5px] font-bold uppercase ' + tintOf(project.name)} aria-hidden="true">
            {project.name.replace(/^[^a-z0-9]+/i, '').charAt(0) || '·'}
          </span>
          <span className="min-w-0 flex-1 truncate text-[12.5px] font-medium text-text-primary">{project.name}</span>
          {working ? <Loader2 size={11} aria-hidden="true" className="shrink-0 animate-spin text-accent motion-reduce:animate-none" /> : null}
          <span className="shrink-0 font-mono text-[9.5px] text-text-muted group-hover/project:hidden">{sessions.length}</span>
          <ChevronRight
            size={12}
            aria-hidden="true"
            className={'shrink-0 text-text-muted transition-transform duration-200 ease-ease group-hover/project:hidden ' + (open ? 'rotate-90' : '')}
          />
        </button>
        <button
          type="button"
          className="project-add hidden h-[20px] w-[20px] shrink-0 place-items-center rounded-sm text-text-muted hover:bg-bg-active hover:text-text-primary group-hover/project:grid"
          title={strings.sidebar.actions.newChatInProject(project.name)}
          aria-label={strings.sidebar.actions.newChatInProject(project.name)}
          onClick={onNewChat}
        >
          <Plus size={12} aria-hidden="true" />
        </button>
        <button
          type="button"
          className={
            'project-more h-[20px] w-[20px] shrink-0 place-items-center rounded-sm text-text-muted hover:bg-bg-active hover:text-text-primary ' +
            (menu ? 'grid bg-bg-active' : 'hidden group-hover/project:grid')
          }
          title={strings.sidebar.actions.projectMenu}
          aria-label={`${strings.sidebar.actions.projectMenu} ${project.name}`}
          aria-expanded={menu}
          onClick={() => {
            setMenu(!menu);
            setArmed(false);
          }}
        >
          <MoreHorizontal size={12} aria-hidden="true" />
        </button>
      </div>

      {menu ? (
        <div className="project-menu mx-[6px] mb-[4px] rounded-md border border-border-subtle bg-bg-raised p-[6px]">
          <div className="mb-[6px] truncate font-mono text-[10px] text-text-muted" title={project.root}>
            {project.root}
          </div>
          <button
            type="button"
            className={
              'flex w-full items-center justify-center gap-[6px] rounded-sm border px-[8px] py-[4px] text-[11px] font-medium transition-colors duration-fast ' +
              (armed
                ? 'border-state-error bg-state-error text-text-on-accent'
                : 'border-border-subtle text-text-secondary hover:border-state-error/60 hover:text-state-error')
            }
            onClick={close}
          >
            <X size={11} aria-hidden="true" />
            {armed ? strings.sidebar.closeFolderArmed(sessions.length) : strings.sidebar.closeFolderAction}
          </button>
        </div>
      ) : null}

      {open ? (
        <div className="project-sessions ml-[15px] border-l border-border-subtle pb-[2px] pl-[3px]">
          {sessions.map((session) => (
            <SessionRow key={session.id} session={session} active={session.id === activeTab} />
          ))}
          <button
            type="button"
            className="project-new-chat flex w-full items-center gap-[6px] rounded-md py-[5px] pl-[10px] pr-[8px] text-left text-[11px] text-text-muted transition-colors duration-fast hover:bg-bg-hover hover:text-text-primary"
            onClick={onNewChat}
          >
            <Plus size={11} aria-hidden="true" />
            {strings.sidebar.newChatShort}
          </button>
        </div>
      ) : null}
    </div>
  );
}

/** A search reaches every machine: the matching chats, each under its machine's name. */
function SearchResults({ hosts, filter, activeTab }: { hosts: readonly Host[]; filter: string; activeTab: string | null }) {
  const groups = hosts
    .map((host) => ({ host, sessions: orderedSessions(host.sessions).filter((session) => matchesFilter(session, filter)) }))
    .filter((group) => group.sessions.length > 0);

  if (groups.length === 0) {
    return <p className="px-[10px] py-[12px] text-[12px] text-text-muted">{strings.sidebar.noMatches}</p>;
  }

  return (
    <>
      {groups.map(({ host, sessions }) => (
        <div key={host.id} className="mb-[6px]">
          <div className="flex items-center gap-[6px] px-[8px] pb-[3px] pt-[6px] text-[10px] font-semibold uppercase tracking-[.09em] text-text-muted">
            <HostIcon type={host.type} size={14} iconSize={8} />
            <span className="truncate">{host.type === 'local' ? strings.sidebar.thisComputer : host.name}</span>
          </div>
          {sessions.map((session) => (
            <SessionRow key={session.id} session={session} active={session.id === activeTab} />
          ))}
        </div>
      ))}
    </>
  );
}
