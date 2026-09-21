import { ChevronDown, ChevronRight, File as FileIcon, Folder, FolderOpen, Loader, RotateCw } from 'lucide-react';
import { useEffect } from 'react';

import type { FsEntry } from '../../../../protocol/types';
import { nameOf } from '../../lib/picker';
import { sizeOf, strings } from '../../strings';
import { useFilesStore, type DirectoryView } from '../../store/files';
import { loadDirectory, loadGitStatus, openDiff, openFile, refreshDirectory, toggleDirectory } from '../../store/intents';
import { findSession, useSessionsStore } from '../../store/sessions';

/**
 * What a row needs to draw itself and its children: the read directories, which rows are open, and which
 * are still being read. A `FilesState` would carry the open *file* and the error into every row, which is
 * how a small component ends up re-rendering on things it does not show.
 */
interface TreeState {
  directories: Record<string, DirectoryView>;
  expanded: string[];
  loading: string[];
}

/**
 * `.files-section` - the folder the active chat works in, as a tree (0.7.7).
 *
 * The other half of 0.7.6. That release gave a chat a working directory and put its name in the prompt
 * toolbar, so "which folder am I in" became answerable - but nothing showed what was *in* it: `fs.list`
 * and `fs.read` had been real methods with no caller in the app since the schema was written.
 *
 * Four decisions worth stating:
 *
 *   * **the tree is the active chat's folder**, not a list of folders. A chat works in one directory
 *     (0.7.6), so this section follows the session the way the pane does; switching chats switches the
 *     tree. A chat with no folder gets the sentence that says how to get one.
 *   * **lazy, one `fs.list` per folder that is opened** - rather than a walk of `node_modules`. The
 *     daemon's listing is one level deep and says which rows are folders, so no extra `fs.stat` is needed.
 *   * **folders first, then files**, each sorted by name: a tree convention, decided here rather than by
 *     the daemon, whose listing is name-sorted.
 *   * **a click on a file opens the Preview**, and `openFile` unfolds the right panel - a click that shows
 *     nothing is the kind of lie this build keeps removing.
 *
 * The guard's hidden names are counted out loud under a folder's rows (`3 names hidden`): the daemon
 * refuses `.env`, `*.pem` and its own data directory, and a tree that is quietly three rows short would be
 * a half-truth (principle P4).
 */
export function FilesSection() {
  const { hosts, activeTab } = useSessionsStore();
  const session = activeTab === null ? null : findSession(hosts, activeTab);
  const root = session?.session.projectRoot ?? null;

  const { root: storedRoot, directories, expanded, loading, error, git } = useFilesStore();

  /* The session's folder is the tree's root, and the store is *told* rather than asked: `session.projectRoot`
     is the one answer to "which folder am I in", and a second copy in the store would be a second answer. */
  useEffect(() => {
    const files = useFilesStore.getState();

    if (root === null) {
      files.reset();

      return;
    }

    files.setRoot(root);

    if (useFilesStore.getState().directories[root] === undefined) {
      void loadDirectory(null);
    }

    /* The branch and the changed-file count come with the folder (0.7.9): the same question at the same time,
       and `git.status` takes a session id exactly as `fs.list` does. */
    void loadGitStatus();
  }, [root]);

  if (root === null) {
    return (
      <div className="files-section files-empty px-[10px] pb-[8px]" id="filesSection">
        <div className="files-title mb-[4px] flex items-center gap-[6px] text-[11px] font-semibold uppercase tracking-wide text-text-muted">
          <FolderOpen size={12} aria-hidden="true" />
          {strings.files.title}
        </div>
        <div className="files-hint text-[11px] leading-[1.5] text-text-muted">
          {strings.files.noFolder}
        </div>
      </div>
    );
  }

  const listing = directories[storedRoot ?? root];

  return (
    <div className="files-section px-[10px] pb-[8px]" id="filesSection">
      <div className="files-title mb-[4px] flex items-center gap-[6px] text-[11px] font-semibold uppercase tracking-wide text-text-muted">
        <FolderOpen size={12} aria-hidden="true" />
        {strings.files.title}
        <button
          type="button"
          className="files-refresh ml-auto grid h-[18px] w-[18px] place-items-center rounded-sm text-text-muted transition-colors duration-fast ease-ease hover:bg-bg-hover hover:text-text-primary"
          title={strings.files.refresh}
          aria-label={strings.files.refresh}
          onClick={() => void refreshDirectory(root)}
        >
          <RotateCw size={11} aria-hidden="true" />
        </button>
      </div>

      <div
        className="files-root mb-[2px] flex items-center gap-[6px] font-mono text-[11px] text-text-secondary"
        title={strings.files.rootTitle(root)}
        data-files-root={root}
      >
        <span className="min-w-0 truncate">{nameOf(root)}</span>

        {/*
          The branch and the changed count (0.7.9), from `git.status` - and a Diff button beside them when
          there is something to see. A folder without git shows neither: no badge, no error, because a folder
          that is not a repository is a normal folder.
        */}
        {git === null ? null : (
          <>
            <span
              className={
                'files-git shrink-0 rounded-sm px-[5px] py-[1px] text-[10px] ' +
                (git.dirty > 0 ? 'bg-orange-subtle text-state-waiting' : 'bg-bg-raised text-text-muted')
              }
              title={git.branch}
              data-git-branch={git.branch}
              data-git-dirty={String(git.dirty)}
            >
              {git.dirty > 0 ? strings.files.gitDirty(git.branch, git.dirty) : strings.files.gitClean(git.branch)}
            </span>

            {git.dirty === 0 ? null : (
              <button
                type="button"
                id="filesDiff"
                className="files-diff shrink-0 rounded-sm border border-border-subtle px-[5px] py-[1px] text-[10px] text-text-secondary transition-colors duration-fast ease-ease hover:border-border-default hover:text-text-primary"
                onClick={() => void openDiff()}
              >
                {strings.files.diff}
              </button>
            )}
          </>
        )}
      </div>

      <div className="files-tree" role="tree" aria-label={strings.files.title}>
        {listing === undefined ? (
          <div className="files-status px-[4px] py-[2px] text-[11px] text-text-muted">
            {strings.files.loading}
          </div>
        ) : (
          <Rows path={root} state={{ directories, expanded, loading }} />
        )}
      </div>

      {error === null ? null : <div className="files-error pt-[2px] text-[11px] text-state-error">{error}</div>}
    </div>
  );
}

/**
 * One directory's rows, folders first - recursively, for each folder that is expanded.
 *
 * The row list is rebuilt from the store on every render (a `Record` of paths), so expanding a folder
 * does not re-request its parents: they are already in the store.
 */
function Rows({ path, state, depth = 0 }: { path: string; state: TreeState; depth?: number }) {
  const listing = state.directories[path];

  if (listing === undefined) {
    return null;
  }

  if (listing.entries.length === 0) {
    return (
      <div className="files-empty-row px-[4px] py-[2px] text-[11px] text-text-muted">
        {strings.files.empty}
      </div>
    );
  }

  const ordered = [...listing.entries].sort((left, right) =>
    left.dir === right.dir ? left.name.localeCompare(right.name) : left.dir ? -1 : 1,
  );

  return (
    <>
      {ordered.map((entry) => (
        <Row key={entry.path} entry={entry} state={state} depth={depth} />
      ))}

      {listing.hidden === 0 ? null : (
        <div className="files-hidden px-[4px] py-[2px] text-[10.5px] text-text-muted">
          {strings.files.hidden(listing.hidden)}
        </div>
      )}
    </>
  );
}

/** One row: a folder with a chevron that expands it, or a file that opens it. Both are buttons. */
function Row({ entry, state, depth }: { entry: FsEntry; state: TreeState; depth: number }) {
  const open = entry.dir && state.expanded.includes(entry.path);
  const busy = entry.dir && state.loading.includes(entry.path);

  return (
    <>
      <button
        type="button"
        role="treeitem"
        aria-expanded={entry.dir ? open : undefined}
        data-file-row={entry.path}
        data-file-dir={entry.dir ? 'true' : 'false'}
        title={entry.path}
        className={
          'files-row flex w-full min-w-0 items-center gap-[5px] rounded-sm py-[2px] pr-[4px] text-left text-[11.5px] transition-colors duration-fast ease-ease hover:bg-bg-hover ' +
          (entry.dir ? 'text-text-secondary' : 'text-text-primary')
        }
        style={{ paddingLeft: `${4 + depth * 10}px` }}
        onClick={() => {
          if (entry.dir) {
            void toggleDirectory(entry.path);

            return;
          }

          void openFile(entry.path, entry.name);
        }}
      >
        {entry.dir ? (
          open ? (
            <ChevronDown size={11} aria-hidden="true" className="shrink-0 text-text-muted" />
          ) : (
            <ChevronRight size={11} aria-hidden="true" className="shrink-0 text-text-muted" />
          )
        ) : (
          <span className="w-[11px] shrink-0" aria-hidden="true" />
        )}

        {entry.dir ? (
          <Folder size={11} aria-hidden="true" className="shrink-0 text-accent" />
        ) : (
          <FileIcon size={11} aria-hidden="true" className="shrink-0 text-text-muted" />
        )}

        <span className="min-w-0 flex-1 truncate">{entry.name}</span>

        {busy ? (
          <Loader size={10} aria-hidden="true" className="shrink-0 animate-spin text-text-muted" />
        ) : null}

        {entry.dir ? null : (
          <span className="files-size shrink-0 font-mono text-[10px] text-text-muted">
            {sizeOf(entry.size)}
          </span>
        )}
      </button>

      {open ? <Rows path={entry.path} state={state} depth={depth + 1} /> : null}
    </>
  );
}
