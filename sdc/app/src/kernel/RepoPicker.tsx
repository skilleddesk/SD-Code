import { LoaderCircle, Lock, Plus, Upload } from 'lucide-react';
import { useEffect, useMemo, useState } from 'react';

import { storedSettings, storeSetting } from '../lib/settings';
import { strings } from '../strings';
import { repoNameOf, useGitHubStore } from '../store/github';
import { toast } from '../store/toast';
import { BTN_SM, BTN_PRIMARY, BTN_SECONDARY } from '../panels/ui/button';

const inputClass = 'w-full rounded-md border border-border-default bg-bg-input px-[8px] py-[5px] text-[12px] text-text-primary outline-none focus:border-border-focus';

export interface RepoPickerProps {
  sessionId: string;
  /** The project, as the store keys it (`placeOf`). */
  place: string;
  /** The project folder's own name - the new repository's name when nothing is typed. */
  folderName: string;
  /** What is linked now, shown as the starting value. */
  current: string | null;
  /** Offer "Save and push" beside "Save". */
  offerPush?: boolean;
  onDone?: () => void;
}

/**
 * Choosing **one project's** repository (1.0.8) - the same form in the chat box's GitHub menu and in Settings → GitHub.
 *
 * One field takes all three ways a person names a repository: one of theirs (the list below it, filtered as they
 * type), a pasted URL or `owner/name`, or a new name - which the daemon makes under the account. Saving points the
 * project folder's `origin` there, on whichever machine the folder is.
 */
export function RepoPicker({ sessionId, place, folderName, current, offerPush = false, onDone }: RepoPickerProps) {
  const k = strings.kernel.github;
  const accountRepos = useGitHubStore((state) => state.accountRepos);
  const [value, setValue] = useState(current ?? '');
  const [privateRepo, setPrivateRepo] = useState(() => storedSettings()['github-private'] !== false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (accountRepos === null) {
      void useGitHubStore.getState().loadAccountRepos();
    }
  }, [accountRepos]);

  const query = value.trim().toLowerCase();
  const matches = useMemo(
    () => (accountRepos ?? []).filter((repo) => query === '' || repo.fullName.toLowerCase().includes(query)).slice(0, 6),
    [accountRepos, query],
  );
  /* A bare word that is none of theirs is a new repository; empty means one named after the folder. */
  const fresh = !query.includes('/') && !matches.some((repo) => repo.fullName.toLowerCase().endsWith(`/${query}`)) ? repoNameOf(value.trim() === '' ? folderName : value.trim()) : null;

  const save = (andPush: boolean): void => {
    const repo = value.trim() === '' ? repoNameOf(folderName) : value.trim();

    setBusy(true);
    setError(null);
    void useGitHubStore
      .getState()
      .link(sessionId, place, repo, privateRepo)
      .then(async (info) => {
        if ('error' in info) {
          setError(info.error);
          return;
        }

        toast(k.linked(info.fullName ?? repo));

        if (andPush) {
          const pushed = await useGitHubStore.getState().push(sessionId, place, { private: privateRepo });

          toast('url' in pushed ? k.pushed(pushed.url) : k.pushFailed(pushed.error), undefined, 8000);
        }

        onDone?.();
      })
      .finally(() => setBusy(false));
  };

  return (
    <div className="flex flex-col gap-[8px]" data-repo-picker="">
      <input
        className={inputClass + ' font-mono'}
        dir="ltr"
        value={value}
        placeholder={k.repoPlaceholder}
        aria-label={k.projectTitle}
        onChange={(event) => setValue(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === 'Enter' && !busy) {
            event.preventDefault();
            save(offerPush);
          }
        }}
      />

      <div className="flex max-h-[180px] flex-col overflow-y-auto rounded-md border border-border-subtle" role="listbox" aria-label={k.projectTitle}>
        {fresh === null ? null : (
          <button type="button" role="option" aria-selected={false} className="flex items-center gap-[6px] px-[8px] py-[5px] text-left text-[12px] text-accent hover:bg-bg-hover" onClick={() => setValue(fresh)}>
            <Plus size={12} aria-hidden="true" className="shrink-0" />
            <span className="truncate">{k.newRepo(fresh)}</span>
          </button>
        )}
        {accountRepos === null ? (
          <span className="flex items-center gap-[6px] px-[8px] py-[5px] text-[11.5px] text-text-muted">
            <LoaderCircle size={11} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />
            {k.loading}
          </span>
        ) : (
          matches.map((repo) => (
            <button
              key={repo.fullName}
              type="button"
              role="option"
              aria-selected={value === repo.fullName}
              className={'flex items-center gap-[6px] px-[8px] py-[5px] text-left font-mono text-[11.5px] hover:bg-bg-hover ' + (value === repo.fullName ? 'bg-bg-active text-text-primary' : 'text-text-secondary')}
              onClick={() => setValue(repo.fullName)}
            >
              <span className="min-w-0 flex-1 truncate">{repo.fullName}</span>
              {repo.private ? <Lock size={10} aria-hidden="true" className="shrink-0 text-text-muted" /> : null}
            </button>
          ))
        )}
      </div>

      <label className="flex items-center gap-[6px] text-[11.5px] text-text-secondary">
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

      {error === null ? null : <p className="m-0 text-[11.5px] text-state-error">{k.linkFailed(error)}</p>}

      <div className="flex flex-wrap justify-end gap-[6px]">
        <button type="button" className={BTN_SM + ' ' + (offerPush ? BTN_SECONDARY : BTN_PRIMARY)} disabled={busy} onClick={() => save(false)}>
          {busy && !offerPush ? <LoaderCircle size={11} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : null}
          {k.linkOnly}
        </button>
        {offerPush ? (
          <button type="button" className={BTN_SM + ' ' + BTN_PRIMARY} disabled={busy} onClick={() => save(true)} data-repo-link-push="">
            {busy ? <LoaderCircle size={11} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <Upload size={11} aria-hidden="true" />}
            {k.linkAndPush}
          </button>
        ) : null}
      </div>
    </div>
  );
}
