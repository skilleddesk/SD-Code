import { useCallback, useEffect, useState } from 'react';
import { CornerLeftUp, Folder, FolderOpen, Loader, Plug } from 'lucide-react';

import { strings } from '../strings';
import { listRemoteDirectory, openRemoteFolder, rebindFolder } from '../store/intents';
import { useOverlayStore } from '../store/overlays';
import { useAppStore } from '../store/store';
import { BTN, BTN_PRIMARY, BTN_SECONDARY } from '../panels/ui/button';
import { Modal } from './Modal';

/** One directory as `fs.list` answered it - the same shape the sidebar's tree reads. */
interface Listing {
  path: string;
  entries: { name: string; path: string; dir: boolean; size: number }[];
  hidden: number;
}

/**
 * Open a folder **on a host** (0.7.13).
 *
 * `Open folder` in the empty Files state uses `tauri-plugin-dialog`, which shows *this* machine's
 * filesystem - and a VPS is not on it. The daemon's `fs.list` with a `hostId` is what can see that
 * machine's folders (it is the same call the sidebar's tree already makes), so the way in is this small
 * browser: it reads the host's home first, then one level per click, and the chosen path goes to
 * `project.add` with the same `hostId` - the daemon validates it with `test -d` **on that machine**, so
 * a typo is refused by the host that has the folder rather than by the laptop that does not.
 *
 * Only folders are listed: a project root is a directory, and a list of files would be a list of things
 * that cannot be opened. The path is also a field, because clicking from `/` to `/srv/app/www` is a lot
 * of clicks, and `~/app` works (the daemon expands it and answers with the absolute path it resolved).
 */
export function RemoteFolder() {
  const hostId = useOverlayStore((state) => state.remoteFolderHostId);
  const sessionId = useOverlayStore((state) => state.remoteFolderSessionId);
  const close = useOverlayStore((state) => state.closeRemoteFolder);
  const host = useAppStore((state) => state.hosts.find((candidate) => candidate.id === hostId));

  const [listing, setListing] = useState<Listing | null>(null);
  const [typed, setTyped] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const read = useCallback(
    (path?: string) => {
      if (hostId === null) {
        return;
      }

      setBusy(true);

      void listRemoteDirectory(hostId, path).then((answer) => {
        setBusy(false);

        if (answer === null) {
          setError(strings.remoteFolder.readFailed);

          return;
        }

        setListing({ path: answer.path, entries: answer.entries, hidden: answer.hidden });
        setTyped(answer.path);
        setError(null);
      });
    },
    [hostId],
  );

  /* Opening the dialog reads the host's home; closing forgets everything, so the next host starts from
     its own home rather than from the last one's path. */
  useEffect(() => {
    if (hostId === null) {
      setListing(null);
      setTyped('');
      setError(null);

      return;
    }

    read();
  }, [hostId, read]);

  const folders = listing === null ? [] : listing.entries.filter((entry) => entry.dir);
  const parent = listing === null ? null : listing.path.replace(/\/[^/]+\/?$/, '') || '/';

  const open = (): void => {
    if (hostId === null) {
      return;
    }

    const root = listing?.path ?? typed;

    if (root === '') {
      return;
    }

    setBusy(true);

    /* Opened from a chat's folder chip, the choice re-points that chat (0.10.0); opened from
       anywhere else, it lands in a chat on the host the way `Open folder` always has. */
    const landed =
      sessionId === null
        ? openRemoteFolder(hostId, root).then((opened) => opened !== null)
        : rebindFolder(sessionId, hostId, root);

    void landed.then((done) => {
      setBusy(false);

      if (done !== false) {
        close();
      }
    });
  };

  return (
    <Modal
      open={hostId !== null}
      label={strings.remoteFolder.title(host?.name ?? 'this host')}
      onClose={close}
      center
      className="remote-folder-dlg"
    >
      <div className="flex items-start gap-[12px] border-b border-border-subtle px-[18px] py-[16px]">
        <div className="grid h-[34px] w-[34px] shrink-0 place-items-center rounded-md bg-accent-subtle text-accent">
          <FolderOpen size={18} aria-hidden="true" />
        </div>
        <div>
          <div className="text-[14px] font-semibold text-text-primary">
            {strings.remoteFolder.title(host?.name ?? 'this host')}
          </div>
          <div className="mt-[2px] text-[12px] text-text-muted">{strings.remoteFolder.sub}</div>
        </div>
      </div>

      <div className="px-[18px] py-[14px]">
        <label className="flex flex-col gap-[5px]">
          <span className="text-[11.5px] font-medium text-text-secondary">{strings.remoteFolder.path}</span>
          <div className="flex items-center gap-[6px]">
            <button
              type="button"
              id="remoteFolderUp"
              className={BTN + ' ' + BTN_SECONDARY}
              disabled={parent === null || parent === listing?.path}
              title={strings.remoteFolder.up}
              aria-label={strings.remoteFolder.up}
              onClick={() => read(parent ?? undefined)}
            >
              <CornerLeftUp size={12} aria-hidden="true" />
            </button>

            <input
              type="text"
              id="remoteFolderPath"
              className="min-w-0 flex-1 rounded-md border border-border-default bg-bg-input px-[10px] py-[7px] font-mono text-[12.5px] text-text-primary placeholder:text-text-muted focus:border-border-strong"
              placeholder="~/app"
              value={typed}
              onChange={(event) => setTyped(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === 'Enter') {
                  read(typed);
                }
              }}
            />
          </div>
          <span className="text-[11px] text-text-muted">{strings.remoteFolder.needAbsolute}</span>
        </label>

        <div
          className="remote-folder-list mt-[12px] max-h-[240px] overflow-y-auto rounded-md border border-border-subtle bg-bg-raised"
          role="listbox"
          aria-label={strings.remoteFolder.path}
        >
          {busy ? (
            <div className="flex items-center gap-[6px] px-[12px] py-[10px] text-[11.5px] text-text-muted">
              <Loader size={12} aria-hidden="true" className="animate-spin" />
              {strings.remoteFolder.loading}
            </div>
          ) : folders.length === 0 ? (
            <div className="px-[12px] py-[10px] text-[11.5px] text-text-muted">{strings.remoteFolder.empty}</div>
          ) : (
            folders.map((entry) => (
              <button
                key={entry.path}
                type="button"
                role="option"
                aria-selected={false}
                data-remote-folder={entry.path}
                className="flex w-full items-center gap-[8px] px-[12px] py-[7px] text-left text-[12px] text-text-secondary transition-colors duration-fast ease-ease hover:bg-bg-hover hover:text-text-primary"
                onClick={() => read(entry.path)}
              >
                <Folder size={12} aria-hidden="true" className="shrink-0 text-accent" />
                <span className="min-w-0 flex-1 truncate font-mono">{entry.name}</span>
              </button>
            ))
          )}
        </div>

        {error === null ? null : <div className="mt-[8px] text-[11.5px] text-state-error">{error}</div>}
      </div>

      <div className="flex items-center gap-[8px] border-t border-border-subtle px-[18px] py-[12px]">
        <button type="button" className={BTN + ' ' + BTN_SECONDARY} onClick={close}>
          {strings.addHost.cancel}
        </button>

        <div className="flex-1" />

        <button
          type="button"
          className={BTN + ' ' + BTN_PRIMARY}
          id="remoteFolderOpen"
          disabled={busy || listing === null}
          onClick={open}
        >
          <Plug size={12} aria-hidden="true" />
          {strings.remoteFolder.open}
        </button>
      </div>
    </Modal>
  );
}
