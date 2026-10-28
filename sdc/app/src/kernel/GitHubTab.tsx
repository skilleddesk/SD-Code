import { Copy, ExternalLink, GitBranch, LoaderCircle, LogOut, Upload } from 'lucide-react';
import { useEffect, useState } from 'react';

import { copyText, openOutside } from '../lib/external';
import { pickFolder } from '../lib/picker';
import { storedSettings, storeSetting } from '../lib/settings';
import { strings } from '../strings';
import { pushToGitHub, useGitHubStore } from '../store/github';
import { daemonSetting } from '../store/kernelIntents';
import { useSessionsStore } from '../store/sessions';
import { toast } from '../store/toast';
import { Badge } from '../panels/ui/Badge';
import { BTN, BTN_PRIMARY, BTN_SECONDARY, BTN_SM } from '../panels/ui/button';

const inputClass = 'rounded-md border border-border-default bg-bg-input px-[8px] py-[5px] text-[12px] text-text-primary';

/**
 * Settings → GitHub (1.0.0): connect by approving on GitHub (no token to make or paste), disconnect, and push the
 * open chat's folder - on this computer or on its VPS - or any folder on this computer.
 */
export function GitHubTab() {
  const k = strings.kernel.github;
  const { status, code, error, refresh, connect, disconnect } = useGitHubStore();
  const { activeTab } = useSessionsStore();
  const [clientId, setClientId] = useState('');
  const [privateRepo, setPrivateRepo] = useState(() => storedSettings()['github-private'] !== false);
  const [commitMessage, setCommitMessage] = useState('');
  const [pushing, setPushing] = useState(false);
  const [result, setResult] = useState<{ url: string } | { error: string } | null>(null);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const waiting = status?.pending?.state === 'waiting';
  const canConnect = status !== null && (status.deviceFlow || status.credentialManager);

  const push = (target: { sessionId?: string; root?: string }): void => {
    setPushing(true);
    setResult(null);
    void pushToGitHub(target, { private: privateRepo, message: commitMessage.trim() === '' ? undefined : commitMessage.trim() })
      .then((answer) => {
        setResult(answer);
        toast('url' in answer ? k.pushed(answer.url) : k.pushFailed(answer.error));
      })
      .finally(() => setPushing(false));
  };

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

      <section className="mt-[16px] rounded-lg border border-border-subtle p-[14px]" aria-disabled={!status?.connected}>
        <h3 className="m-0 text-[13.5px] font-semibold text-text-primary">{k.pushTitle}</h3>
        <p className="mb-[10px] mt-[4px] text-[11.5px] text-text-muted">{k.pushHelp}</p>
        <div className="flex flex-col gap-[8px]">
          <label className="flex items-center gap-[8px] text-[12px] text-text-primary">
            <input
              type="checkbox"
              className="accent-[var(--accent)]"
              checked={privateRepo}
              onChange={(event) => {
                setPrivateRepo(event.target.checked);
                storeSetting('github-private', event.target.checked);
              }}
            />
            {k.private}
          </label>
          <label className="flex flex-col gap-[4px] text-[12px] text-text-primary">
            {k.message}
            <input className={inputClass} value={commitMessage} placeholder={k.messagePlaceholder} onChange={(event) => setCommitMessage(event.target.value)} />
          </label>
          <div className="flex flex-wrap gap-[8px]">
            <button
              type="button"
              className={BTN + ' ' + BTN_PRIMARY}
              disabled={!status?.connected || pushing || activeTab === null}
              onClick={() => activeTab !== null && push({ sessionId: activeTab })}
              data-github-push=""
            >
              {pushing ? <LoaderCircle size={13} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <Upload size={13} aria-hidden="true" />}
              {pushing ? k.pushing : k.push}
            </button>
            <button
              type="button"
              className={BTN + ' ' + BTN_SECONDARY}
              disabled={!status?.connected || pushing}
              onClick={() =>
                void pickFolder().then((root) => {
                  if (root !== null) {
                    push({ root });
                  }
                })
              }
            >
              {k.pickFolder}
            </button>
          </div>
          {activeTab === null ? <p className="m-0 text-[11.5px] text-text-muted">{k.noChatFolder}</p> : null}
          {result === null ? null : 'url' in result ? (
            <p className="m-0 flex items-center gap-[8px] text-[12px] text-state-success">
              {k.pushed(result.url)}
              <button type="button" className="text-accent hover:underline" onClick={() => void openOutside(result.url)}>
                {k.open}
              </button>
            </p>
          ) : (
            <p className="m-0 text-[12px] text-state-error">{k.pushFailed(result.error)}</p>
          )}
        </div>
      </section>
    </div>
  );
}
