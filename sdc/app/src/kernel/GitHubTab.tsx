import { Copy, ExternalLink, GitBranch, LoaderCircle, LogOut, Server } from 'lucide-react';
import { useEffect, useState } from 'react';

import { copyText, openOutside } from '../lib/external';
import { nameOf } from '../lib/picker';
import { strings } from '../strings';
import { placeOf, useGitHubStore } from '../store/github';
import { daemonSetting } from '../store/kernelIntents';
import { useSessionsStore } from '../store/sessions';
import { toast } from '../store/toast';
import { RepoPicker } from './RepoPicker';
import { Badge } from '../panels/ui/Badge';
import { BTN, BTN_PRIMARY, BTN_SECONDARY, BTN_SM } from '../panels/ui/button';

const inputClass = 'rounded-md border border-border-default bg-bg-input px-[8px] py-[5px] text-[12px] text-text-primary';

/**
 * Settings → GitHub (1.0.0): connect by approving on GitHub (no token to make or paste), and disconnect.
 *
 * 1.0.8: pushing left this tab - it happens in the chat, from the GitHub button beside Send (or by typing "push"). What
 * is here instead is every project with the repository it pushes to, each changeable: a project on this computer and
 * one on a VPS, or two projects on one machine, each have their own.
 */
export function GitHubTab() {
  const k = strings.kernel.github;
  const { status, code, error, refresh, connect, disconnect } = useGitHubStore();
  const [clientId, setClientId] = useState('');

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const waiting = status?.pending?.state === 'waiting';
  const canConnect = status !== null && (status.deviceFlow || status.credentialManager);

  return (
    <div data-github-tab="">
      <div className="mb-[18px]">
        <h2 className="flex items-center gap-[8px] text-[17px] font-semibold text-text-primary">
          <GitBranch size={17} aria-hidden="true" />
          {k.title}
        </h2>
        <p className="mt-[4px] text-[12.5px] text-text-muted">{k.desc}</p>
      </div>

      <section className="rounded-lg border border-border-subtle bg-bg-raised p-[14px]">
        {status?.connected ? (
          <div className="flex items-center gap-[12px]">
            {status.avatarUrl === null ? null : <img src={status.avatarUrl} alt="" className="h-[36px] w-[36px] rounded-full" />}
            <div className="min-w-0 flex-1">
              <div className="text-[13px] font-semibold text-text-primary">{status.name ?? status.login}</div>
              <Badge tone="success">{k.connected(status.login ?? '')}</Badge>
            </div>
            <button type="button" className={BTN + ' ' + BTN_SECONDARY} onClick={() => void disconnect().then(() => toast(k.disconnected))}>
              <LogOut size={13} aria-hidden="true" />
              {k.disconnect}
            </button>
          </div>
        ) : (
          <div className="flex flex-col gap-[10px]">
            <div className="flex items-center gap-[10px]">
              <Badge tone="muted">{k.notConnected}</Badge>
              <button type="button" className={BTN + ' ' + BTN_PRIMARY} disabled={!canConnect || waiting} onClick={() => void connect()} data-github-connect="">
                {waiting ? <LoaderCircle size={13} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <GitBranch size={13} aria-hidden="true" />}
                {waiting ? k.connecting : k.connect}
              </button>
            </div>
            {code === null ? null : (
              <div className="rounded-md border border-accent bg-accent-subtle p-[10px] text-[12.5px] text-text-primary" role="status">
                <p className="m-0">{k.deviceSteps}</p>
                <div className="mt-[8px] flex flex-wrap items-center gap-[8px]">
                  <code className="rounded-md bg-bg-input px-[10px] py-[4px] font-mono text-[18px] font-bold tracking-[.15em]">{code.userCode}</code>
                  <button type="button" className={BTN_SM + ' ' + BTN_SECONDARY} onClick={() => void copyText(code.userCode)}>
                    <Copy size={11} aria-hidden="true" />
                    {k.copyCode}
                  </button>
                  <button type="button" className={BTN_SM + ' ' + BTN_SECONDARY} onClick={() => void openOutside(code.verificationUri)}>
                    <ExternalLink size={11} aria-hidden="true" />
                    {k.openAgain}
                  </button>
                </div>
              </div>
            )}
            {waiting && code === null && status?.pending?.method === 'credential-manager' ? <p className="m-0 text-[12px] text-text-secondary">{k.gcmSteps}</p> : null}
            {error === null ? null : <p className="m-0 text-[12px] text-state-error">{k.failed(error)}</p>}
            {status !== null && !canConnect ? <p className="m-0 text-[12px] text-state-waiting">{k.noRoute}</p> : null}
            <p className="m-0 text-[11.5px] text-text-muted">{k.permissions}</p>
            {status !== null && !status.deviceFlow ? (
              <label className="flex flex-col gap-[4px] text-[12px] text-text-primary">
                {k.clientId}
                <span className="text-[11px] text-text-muted">{k.clientIdHelp}</span>
                <span className="flex gap-[6px]">
                  <input className={inputClass + ' flex-1 font-mono'} value={clientId} onChange={(event) => setClientId(event.target.value)} placeholder="Ov23li…" />
                  <button
                    type="button"
                    className={BTN_SM + ' ' + BTN_SECONDARY}
                    disabled={clientId.trim() === ''}
                    onClick={() => void daemonSetting('github.clientId', clientId.trim()).then(() => refresh())}
                  >
                    {k.save}
                  </button>
                </span>
              </label>
            ) : null}
          </div>
        )}
      </section>

      {status?.connected ? <ProjectRepos /> : null}
    </div>
  );
}

/** Every project the chats work in - here and on each server - with its own repository. */
function ProjectRepos() {
  const k = strings.kernel.github;
  const { hosts } = useSessionsStore();
  const repos = useGitHubStore((state) => state.repos);
  const [editing, setEditing] = useState<string | null>(null);
  const projects = new Map<string, { sessionId: string; hostName: string; root: string }>();

  for (const host of hosts) {
    for (const session of host.sessions) {
      const place = placeOf(host.id, session.projectRoot);

      if (place !== null && session.projectRoot && !projects.has(place)) {
        projects.set(place, { sessionId: session.id, hostName: host.name, root: session.projectRoot });
      }
    }
  }

  useEffect(() => {
    for (const [place, project] of projects) {
      if (useGitHubStore.getState().repos[place] === undefined) {
        void useGitHubStore.getState().loadRepo(project.sessionId, place);
      }
    }
    /* Read once per project that appears; the map is rebuilt every render, its keys are what matter. */
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [[...projects.keys()].join('|')]);

  return (
    <section className="mt-[16px] rounded-lg border border-border-subtle p-[14px]" data-github-projects="">
      <h3 className="m-0 text-[13.5px] font-semibold text-text-primary">{k.projectsTitle}</h3>
      <p className="mb-[10px] mt-[4px] text-[11.5px] text-text-muted">{k.projectsHelp}</p>
      {projects.size === 0 ? <p className="m-0 text-[12px] text-text-muted">{k.noProjects}</p> : null}
      <ul className="m-0 flex list-none flex-col gap-[6px] p-0">
        {[...projects].map(([place, project]) => {
          const info = repos[place];
          const linked = info !== undefined && !('error' in info) && info.onGitHub ? info : null;

          return (
            <li key={place} className="rounded-md border border-border-subtle bg-bg-raised px-[10px] py-[8px]">
              <div className="flex items-center gap-[8px]">
                <Server size={12} aria-hidden="true" className="shrink-0 text-text-muted" />
                <div className="min-w-0 flex-1">
                  <div className="truncate text-[12.5px] text-text-primary" title={project.root}>
                    {nameOf(project.root)} <span className="font-mono text-[10.5px] text-text-muted">· {project.hostName}</span>
                  </div>
                  <div className="truncate font-mono text-[11px] text-text-secondary">
                    {info === undefined ? k.loading : 'error' in info ? info.error : info.gitMissing ? k.gitMissing : linked !== null ? linked.fullName : k.noRepo}
                  </div>
                </div>
                {linked?.url ? (
                  <button type="button" className={BTN_SM + ' ' + BTN_SECONDARY} onClick={() => void openOutside(linked.url ?? '')} title={k.open} aria-label={k.open}>
                    <ExternalLink size={11} aria-hidden="true" />
                  </button>
                ) : null}
                <button type="button" className={BTN_SM + ' ' + BTN_SECONDARY} aria-expanded={editing === place} onClick={() => setEditing(editing === place ? null : place)}>
                  {k.change}
                </button>
              </div>
              {editing === place ? (
                <div className="mt-[8px]">
                  <RepoPicker sessionId={project.sessionId} place={place} folderName={nameOf(project.root)} current={linked?.fullName ?? null} onDone={() => setEditing(null)} />
                </div>
              ) : null}
            </li>
          );
        })}
      </ul>
    </section>
  );
}
