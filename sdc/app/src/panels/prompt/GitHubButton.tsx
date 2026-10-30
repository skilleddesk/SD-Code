import { ChevronUp, Copy, ExternalLink, Github, LoaderCircle, LogIn, Upload } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';

import { copyText, openOutside } from '../../lib/external';
import { nameOf } from '../../lib/picker';
import { strings } from '../../strings';
import { placeOf, useGitHubStore } from '../../store/github';
import { useOverlayStore } from '../../store/overlays';
import { findSession, useSessionsStore } from '../../store/sessions';
import { toast } from '../../store/toast';
import { RepoPicker } from '../../kernel/RepoPicker';
import { BTN_SM, BTN_PRIMARY, BTN_SECONDARY } from '../ui/button';

/**
 * The GitHub button beside Send (1.0.8) - this chat's project, pushed to **its own** repository in one click.
 *
 * The report: "a GitHub icon next to Send; click it and it pushes - and every project, local or on a VPS, has its own
 * repository". The project is the chat's folder on its machine, and its repository is that folder's `origin`
 * (`github.repo`), so two projects on one computer, or the same name on two servers, never share one.
 *
 *   not connected       a click starts GitHub's browser approval; SDC connects by itself when it is approved
 *   no repository yet   a click opens the menu to choose one (yours, a URL, or a new name) - then Save and push
 *   linked              a click pushes; the arrow opens the menu (change the repository, open it on GitHub)
 */
export function GitHubButton({ sessionId }: { sessionId: string | undefined }) {
  const k = strings.kernel.github;
  const { hosts } = useSessionsStore();
  const found = sessionId === undefined ? null : findSession(hosts, sessionId);
  const place = found === null ? null : placeOf(found.host.id, found.session.projectRoot);
  const status = useGitHubStore((state) => state.status);
  const code = useGitHubStore((state) => state.code);
  const info = useGitHubStore((state) => (place === null ? undefined : state.repos[place]));
  const pushing = useGitHubStore((state) => (place === null ? false : state.pushing[place] === true));
  const [open, setOpen] = useState(false);
  const [changing, setChanging] = useState(false);
  const rootRef = useRef<HTMLSpanElement | null>(null);
  const connected = status?.connected === true;
  const waiting = status?.pending?.state === 'waiting';

  useEffect(() => {
    if (useGitHubStore.getState().status === null) {
      void useGitHubStore.getState().refresh();
    }
  }, []);

  /* The project's repository, read once per project (and again after a push or a link). */
  useEffect(() => {
    if (sessionId !== undefined && place !== null && info === undefined) {
      void useGitHubStore.getState().loadRepo(sessionId, place);
    }
  }, [sessionId, place, info]);

  useEffect(() => {
    if (!open) {
      return;
    }

    const outside = (event: PointerEvent): void => {
      if (rootRef.current !== null && !rootRef.current.contains(event.target as Node)) {
        setOpen(false);
      }
    };
    const escape = (event: KeyboardEvent): void => {
      if (event.key === 'Escape') {
        setOpen(false);
      }
    };

    document.addEventListener('pointerdown', outside, true);
    document.addEventListener('keydown', escape);

    return () => {
      document.removeEventListener('pointerdown', outside, true);
      document.removeEventListener('keydown', escape);
    };
  }, [open]);

  if (sessionId === undefined || found === null) {
    return null;
  }

  const repo = info !== undefined && !('error' in info) && info.onGitHub ? info : null;
  const folderName = found.session.projectRoot === null || found.session.projectRoot === undefined ? '' : nameOf(found.session.projectRoot);

  const push = (): void => {
    if (place === null || repo === null) {
      return;
    }

    void useGitHubStore
      .getState()
      .push(sessionId, place, { private: true })
      .then((answer) => toast('url' in answer ? k.pushed(answer.url) : k.pushFailed(answer.error), undefined, 8000));
  };

  const press = (): void => {
    if (!connected) {
      setOpen(true);

      if (status !== null && (status.deviceFlow || status.credentialManager) && !waiting) {
        void useGitHubStore.getState().connect();
      }

      return;
    }

    if (repo !== null) {
      push();
      return;
    }

    setChanging(true);
    setOpen(true);
  };

  const title = !connected ? k.buttonConnect : pushing ? k.pushing : repo !== null ? k.buttonPush(repo.fullName ?? '') : k.buttonLink;
  const Icon = pushing || waiting ? LoaderCircle : Github;

  return (
    <span className="github-btn relative inline-flex shrink-0 items-center" ref={rootRef}>
      <button
        type="button"
        className={
          'relative grid h-[34px] w-[34px] place-items-center rounded-full border transition-all duration-fast ease-ease active:scale-[.94] ' +
          (repo !== null ? 'border-border-default bg-bg-raised text-text-primary hover:border-border-strong' : 'border-border-subtle bg-bg-raised text-text-muted hover:border-border-default hover:text-text-primary')
        }
        title={title}
        aria-label={title}
        disabled={pushing}
        onClick={press}
        data-github-button={repo !== null ? 'push' : connected ? 'link' : 'connect'}
      >
        <Icon size={15} aria-hidden="true" className={pushing || waiting ? 'animate-spin motion-reduce:animate-none' : ''} />
        {repo !== null && repo.changes > 0 && !pushing ? <span className="absolute right-[5px] top-[5px] h-[7px] w-[7px] rounded-full border border-bg-raised bg-accent" aria-hidden="true" /> : null}
      </button>
      {connected && place !== null ? (
        <button
          type="button"
          className="github-btn-menu -ml-[4px] grid h-[22px] w-[16px] place-items-center rounded-full text-text-muted hover:text-text-primary"
          title={k.buttonMenu}
          aria-label={k.buttonMenu}
          aria-expanded={open}
          onClick={() => {
            setChanging(repo === null);
            setOpen((value) => !value);
          }}
        >
          <ChevronUp size={11} aria-hidden="true" />
        </button>
      ) : null}

      {open ? (
        <div className="absolute bottom-full right-0 z-40 mb-[8px] w-[min(340px,calc(100vw-32px))] rounded-lg border border-border-default bg-bg-overlay p-[10px] text-[12px] shadow-xl" role="dialog" aria-label={k.projectTitle}>
          <div className="mb-[6px] flex items-center gap-[6px] text-[12.5px] font-semibold text-text-primary">
            <Github size={13} aria-hidden="true" />
            <span className="min-w-0 flex-1 truncate">{connected ? k.projectTitle : k.title}</span>
            {connected && status?.login ? <span className="font-mono text-[10.5px] font-normal text-text-muted">{status.login}</span> : null}
          </div>

          {!connected ? (
            <ConnectBody code={code} waiting={waiting} canConnect={status !== null && (status.deviceFlow || status.credentialManager)} />
          ) : place === null ? (
            <p className="m-0 text-text-secondary">{k.noFolder}</p>
          ) : info === undefined ? (
            <p className="m-0 flex items-center gap-[6px] text-text-muted">
              <LoaderCircle size={11} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />
              {k.loading}
            </p>
          ) : 'error' in info ? (
            <p className="m-0 text-state-error">{info.error}</p>
          ) : info.gitMissing ? (
            <p className="m-0 text-state-warning">{k.gitMissing}</p>
          ) : (
            <div className="flex flex-col gap-[8px]">
              <p className="m-0 truncate font-mono text-[10.5px] text-text-muted" title={found.session.projectRoot ?? ''}>
                {found.host.name} · {folderName}
              </p>
              {repo !== null && !changing ? (
                <>
                  <div className="flex items-center gap-[6px]">
                    <span className="min-w-0 flex-1 truncate font-mono text-[12px] text-text-primary">{repo.fullName}</span>
                    <button type="button" className={BTN_SM + ' ' + BTN_SECONDARY} onClick={() => setChanging(true)}>
                      {k.change}
                    </button>
                  </div>
                  <p className="m-0 text-[11px] text-text-muted">
                    {repo.branch === null ? '' : `${repo.branch} · `}
                    {repo.changes > 0 ? k.changes(repo.changes) : k.upToDate}
                  </p>
                  <div className="flex flex-wrap justify-end gap-[6px]">
                    {repo.url === null ? null : (
                      <button type="button" className={BTN_SM + ' ' + BTN_SECONDARY} onClick={() => void openOutside(repo.url ?? '')}>
                        <ExternalLink size={11} aria-hidden="true" />
                        {k.open}
                      </button>
                    )}
                    <button
                      type="button"
                      className={BTN_SM + ' ' + BTN_PRIMARY}
                      disabled={pushing}
                      onClick={() => {
                        setOpen(false);
                        push();
                      }}
                    >
                      {pushing ? <LoaderCircle size={11} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <Upload size={11} aria-hidden="true" />}
                      {pushing ? k.pushing : k.push}
                    </button>
                  </div>
                </>
              ) : (
                <>
                  <p className="m-0 text-[11.5px] text-text-secondary">{info.remote !== null && !info.onGitHub ? k.notGitHub(info.remote) : k.projectHelp}</p>
                  <RepoPicker
                    key={place}
                    sessionId={sessionId}
                    place={place}
                    folderName={folderName}
                    current={repo?.fullName ?? null}
                    offerPush
                    onDone={() => {
                      setChanging(false);
                      setOpen(false);
                    }}
                  />
                </>
              )}
            </div>
          )}
        </div>
      ) : null}
    </span>
  );
}

/** Not connected yet: GitHub's approval, opened by the click that showed this. */
function ConnectBody({ code, waiting, canConnect }: { code: { userCode: string; verificationUri: string } | null; waiting: boolean; canConnect: boolean }) {
  const k = strings.kernel.github;
  const error = useGitHubStore((state) => state.error);

  if (!canConnect) {
    return (
      <div className="flex flex-col gap-[8px]">
        <p className="m-0 text-text-secondary">{k.noRoute}</p>
        <button type="button" className={BTN_SM + ' ' + BTN_SECONDARY + ' self-end'} onClick={() => useOverlayStore.getState().openSettings('github')}>
          {k.title}
        </button>
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-[8px]">
      <p className="m-0 text-text-secondary">{code !== null ? k.deviceSteps : waiting ? k.gcmSteps : k.connectFirst}</p>
      {code === null ? null : (
        <div className="flex flex-wrap items-center gap-[6px]">
          <code className="rounded-md bg-bg-input px-[8px] py-[3px] font-mono text-[15px] font-bold tracking-[.12em] text-text-primary">{code.userCode}</code>
          <button type="button" className={BTN_SM + ' ' + BTN_SECONDARY} onClick={() => void copyText(code.userCode)}>
            <Copy size={11} aria-hidden="true" />
            {k.copyCode}
          </button>
          <button type="button" className={BTN_SM + ' ' + BTN_SECONDARY} onClick={() => void openOutside(code.verificationUri)}>
            <ExternalLink size={11} aria-hidden="true" />
            {k.openAgain}
          </button>
        </div>
      )}
      {error === null ? null : <p className="m-0 text-[11.5px] text-state-error">{k.failed(error)}</p>}
      {waiting ? (
        <p className="m-0 flex items-center gap-[6px] text-[11.5px] text-text-muted">
          <LoaderCircle size={11} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />
          {k.connecting}
        </p>
      ) : (
        <button type="button" className={BTN_SM + ' ' + BTN_PRIMARY + ' self-end'} onClick={() => void useGitHubStore.getState().connect()} data-github-connect="">
          <LogIn size={11} aria-hidden="true" />
          {k.connect}
        </button>
      )}
      <p className="m-0 text-[10.5px] text-text-muted">{k.connectHint}</p>
    </div>
  );
}
