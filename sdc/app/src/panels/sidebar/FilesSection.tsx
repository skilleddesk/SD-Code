import {
  ChevronDown,
  ChevronRight,
  File as FileIcon,
  FilePlus,
  Folder,
  FolderOpen,
  FolderPlus,
  Loader,
  Pencil,
  RotateCw,
  Search,
  Trash2,
  X,
} from 'lucide-react';
import { createContext, useContext, useEffect, useMemo, useRef, useState, type FormEvent } from 'react';

import type { FsEntry } from '../../../../protocol/types';
import { baseName } from '../../lib/paths';
import { nameOf } from '../../lib/picker';
import { sizeOf, strings } from '../../strings';
import { useFilesStore, type DirectoryView } from '../../store/files';
import {
  createFile,
  createFolder,
  deletePath,
  loadDirectory,
  loadGitStatus,
  openDiff,
  refreshAfterTurn,
  openFile,
  refreshDirectory,
  renamePath,
  revealFolder,
  searchFolder,
  toggleDirectory,
  type FolderSearchResult,
  type SearchHit,
} from '../../store/intents';
import { useOverlayStore } from '../../store/overlays';
import { findSession, useSessionsStore } from '../../store/sessions';
import { useAppStore } from '../../store/store';

/**
 * The Files section of the sidebar (0.7.7) - the folder the chat works in, and since v4 the place to
 * change it by hand as well as to look at it.
 *
 *   the tree       one level at a time (`fs.list`), folders first, the guard's hidden names counted
 *   the header     New file, New folder, Search in folder, Refresh
 *   a row          click to open or expand; on hover or focus, Rename (in place) and Delete (a second
 *                  click confirms). Every change goes through the daemon, which takes a checkpoint
 *                  first - so a delete from here is one Rewind away from undone (P5).
 *   the search     a literal search of every file (`fs.search`, here or on the host); a hit opens the
 *                  file on its line.
 *
 * All of it works the same on a VPS: the daemon answers `fs.*` from whichever machine the folder is on.
 */
interface TreeState {
  directories: Record<string, DirectoryView>;
  expanded: string[];
  loading: string[];
}

/** What the tree is in the middle of: renaming a row, or naming something new in a folder. */
interface Editing {
  renaming: string | null;
  creating: { directory: string; kind: 'file' | 'folder' } | null;
  armed: string | null;
  setRenaming: (path: string | null) => void;
  setCreating: (creating: Editing['creating']) => void;
  setArmed: (path: string | null) => void;
}

const EditingContext = createContext<Editing | null>(null);

function useEditing(): Editing {
  const editing = useContext(EditingContext);

  if (editing === null) {
    throw new Error('the tree rows are drawn inside FilesSection');
  }

  return editing;
}

const HEADER_BUTTON =
  'grid h-[18px] w-[18px] place-items-center rounded-sm text-text-muted transition-colors duration-fast ease-ease hover:bg-bg-hover hover:text-text-primary focus-visible:outline focus-visible:outline-2 focus-visible:outline-border-focus';

export function FilesSection() {
  const { hosts, activeTab } = useSessionsStore();
  const session = activeTab === null ? null : findSession(hosts, activeTab);
  const root = session?.session.projectRoot ?? null;
  /* The host this chat is on (0.7.13). A chat on a VPS has no local folder to pick, so the way in is
     the host's own browser (`RemoteFolder`) - and this is where it is offered. */
  const host = activeTab === null ? null : hosts.find((candidate) => candidate.sessions.some((item) => item.id === activeTab)) ?? null;
  const openRemoteFolder = useOverlayStore((state) => state.openRemoteFolder);

  const { root: storedRoot, directories, expanded, loading, error, git } = useFilesStore();
  /* How many of this chat's turns have ended - a number, so the effect below runs once per ended turn. */
  const ended = useAppStore(
    (state) => state.turns.filter((turn) => turn.sessionId === activeTab && turn.status !== 'running' && turn.status !== 'stuck').length,
  );
  const [renaming, setRenaming] = useState<string | null>(null);
  const [creating, setCreating] = useState<Editing['creating']>(null);
  const [armed, setArmed] = useState<string | null>(null);
  const editing = useMemo<Editing>(
    () => ({ renaming, creating, armed, setRenaming, setCreating, setArmed }),
    [renaming, creating, armed],
  );

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

    /* The branch and the changed-file count come with the folder (0.7.9). */
    void loadGitStatus();
  }, [root]);

  /* A turn ended: show what it changed (the tree, the badge, the open files). */
  useEffect(() => {
    if (ended > 0 && root !== null) {
      void refreshAfterTurn();
    }
  }, [ended, root]);

  if (root === null) {
    return (
      <div className="files-section files-empty px-[10px] pb-[8px]" id="filesSection">
        <div className="files-title mb-[4px] flex items-center gap-[6px] text-[11px] font-semibold uppercase tracking-wide text-text-muted">
          <FolderOpen size={12} aria-hidden="true" />
          {strings.files.title}
        </div>
        <div className="files-hint text-[11px] leading-[1.5] text-text-muted">{strings.files.noFolder}</div>

        {/* A chat on a host: the folder it works in lives on *that* machine, so the native picker (which
            shows this one) cannot find it and `fs.list` on the host is the way in (0.7.13). */}
        {host === null || host.id === 'local' ? null : (
          <button
            type="button"
            id="openRemoteFolder"
            className="files-remote mt-[6px] flex w-full items-center gap-[6px] rounded-md border border-dashed border-border-default px-[8px] py-[6px] text-[11px] text-text-secondary transition-all duration-fast ease-ease hover:border-solid hover:border-border-strong hover:bg-bg-hover hover:text-text-primary"
            onClick={() => openRemoteFolder(host.id)}
          >
            <FolderOpen size={12} aria-hidden="true" />
            {strings.remoteFolder.title(host.name)}
          </button>
        )}
      </div>
    );
  }

  const top = storedRoot ?? root;
  const listing = directories[top];

  return (
    <EditingContext.Provider value={editing}>
      <div className="files-section px-[10px] pb-[8px]" id="filesSection">
        <div className="files-title mb-[4px] flex items-center gap-[4px] text-[11px] font-semibold uppercase tracking-wide text-text-muted">
          <FolderOpen size={12} aria-hidden="true" />
          <span className="mr-auto">{strings.files.title}</span>
          <button type="button" className={HEADER_BUTTON} title={strings.files.newFile} aria-label={strings.files.newFile} onClick={() => setCreating({ directory: top, kind: 'file' })}>
            <FilePlus size={11} aria-hidden="true" />
          </button>
          <button type="button" className={HEADER_BUTTON} title={strings.files.newFolder} aria-label={strings.files.newFolder} onClick={() => setCreating({ directory: top, kind: 'folder' })}>
            <FolderPlus size={11} aria-hidden="true" />
          </button>
          <button type="button" className={'files-refresh ' + HEADER_BUTTON} title={strings.files.refresh} aria-label={strings.files.refresh} onClick={() => void refreshDirectory(root)}>
            <RotateCw size={11} aria-hidden="true" />
          </button>
        </div>

        <div
          className="files-root mb-[2px] flex items-center gap-[6px] font-mono text-[11px] text-text-secondary"
          title={strings.files.rootTitle(root)}
          data-files-root={root}
        >
          <span className="min-w-0 truncate">{nameOf(root)}</span>

          {/* The branch and the changed count (0.7.9), from `git.status`, and Diff when there is one. */}
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

        {/* The search bar lives above the tree, always (0.11.0): "where is index.php" is the first
            question a project's sidebar is asked, and an icon-toggle hid the answer's door. It
            searches names *and* contents as you type; empty, it costs one quiet row. */}
        <FolderSearch root={root} />

        <div className="files-tree" role="tree" aria-label={strings.files.title}>
          {creating !== null && creating.directory === top ? <NameInput depth={0} kind={creating.kind} /> : null}
          {listing === undefined ? (
            <div className="files-status px-[4px] py-[2px] text-[11px] text-text-muted">{strings.files.loading}</div>
          ) : (
            <Rows path={top} state={{ directories, expanded, loading }} />
          )}
        </div>

        {error === null ? null : <div className="files-error pt-[2px] text-[11px] text-state-error">{error}</div>}
      </div>
    </EditingContext.Provider>
  );
}

/** One directory's rows: folders first, then files, then the guard's count. */
function Rows({ path, state, depth = 0 }: { path: string; state: TreeState; depth?: number }) {
  const listing = state.directories[path];

  if (listing === undefined) {
    return null;
  }

  if (listing.entries.length === 0) {
    return (
      <div className="files-empty-row py-[2px] text-[11px] text-text-muted" style={{ paddingLeft: `${4 + depth * 10}px` }}>
        {strings.files.empty}
      </div>
    );
  }

  /* `.git` is the repository's machinery, not the project's files - the badge above stands for it. */
  const ordered = listing.entries.filter((entry) => entry.name !== '.git').sort((left, right) =>
    left.dir === right.dir ? left.name.localeCompare(right.name) : left.dir ? -1 : 1,
  );

  return (
    <>
      {ordered.map((entry) => (
        <Row key={entry.path} entry={entry} state={state} depth={depth} />
      ))}

      {listing.hidden === 0 ? null : (
        <div className="files-hidden py-[2px] text-[10.5px] text-text-muted" style={{ paddingLeft: `${4 + depth * 10}px` }}>
          {strings.files.hidden(listing.hidden)}
        </div>
      )}
    </>
  );
}

/** One row: a folder that expands or a file that opens - and, on hover, its Rename and Delete. */
function Row({ entry, state, depth }: { entry: FsEntry; state: TreeState; depth: number }) {
  const editing = useEditing();
  const open = entry.dir && state.expanded.includes(entry.path);
  const busy = entry.dir && state.loading.includes(entry.path);
  const armed = editing.armed === entry.path;

  if (editing.renaming === entry.path) {
    return (
      <>
        <NameInput depth={depth} kind={entry.dir ? 'folder' : 'file'} renaming={entry} />
        {open ? <Rows path={entry.path} state={state} depth={depth + 1} /> : null}
      </>
    );
  }

  return (
    <>
      <div className="files-row-wrap group relative flex items-center rounded-sm hover:bg-bg-hover focus-within:bg-bg-hover">
        <button
          type="button"
          role="treeitem"
          aria-expanded={entry.dir ? open : undefined}
          data-file-row={entry.path}
          data-file-dir={entry.dir ? 'true' : 'false'}
          title={entry.path}
          className={
            'files-row flex min-w-0 flex-1 items-center gap-[5px] py-[2px] pr-[4px] text-left text-[11.5px] ' +
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

          {busy ? <Loader size={10} aria-hidden="true" className="shrink-0 animate-spin text-text-muted" /> : null}

          {entry.dir ? null : (
            <span className="files-size shrink-0 font-mono text-[10px] text-text-muted group-hover:hidden group-focus-within:hidden">
              {sizeOf(entry.size)}
            </span>
          )}
        </button>

        <div className={'files-actions shrink-0 items-center gap-[1px] pr-[2px] ' + (armed ? 'flex' : 'hidden group-hover:flex group-focus-within:flex')}>
          {entry.dir ? (
            <button
              type="button"
              className={HEADER_BUTTON}
              title={strings.files.newFile}
              aria-label={`${strings.files.newFile} · ${entry.name}`}
              onClick={() => {
                if (!open) {
                  void toggleDirectory(entry.path);
                }

                editing.setCreating({ directory: entry.path, kind: 'file' });
              }}
            >
              <FilePlus size={10} aria-hidden="true" />
            </button>
          ) : null}
          <button
            type="button"
            className={HEADER_BUTTON}
            title={strings.files.rename}
            aria-label={`${strings.files.rename} ${entry.name}`}
            onClick={() => editing.setRenaming(entry.path)}
          >
            <Pencil size={10} aria-hidden="true" />
          </button>
          <button
            type="button"
            className={
              'grid h-[18px] place-items-center rounded-sm transition-colors duration-fast ' +
              (armed ? 'w-auto bg-red-subtle px-[5px] text-[10px] font-semibold text-state-error' : 'w-[18px] text-text-muted hover:bg-red-subtle hover:text-state-error')
            }
            title={armed ? strings.files.confirmDelete(entry.name) : strings.files.delete}
            aria-label={armed ? strings.files.confirmDelete(entry.name) : `${strings.files.delete} ${entry.name}`}
            onClick={() => {
              if (!armed) {
                editing.setArmed(entry.path);

                return;
              }

              editing.setArmed(null);
              void deletePath(entry.path);
            }}
            onBlur={() => {
              if (armed) {
                editing.setArmed(null);
              }
            }}
          >
            {armed ? strings.files.delete : <Trash2 size={10} aria-hidden="true" />}
          </button>
        </div>
      </div>

      {editing.creating !== null && editing.creating.directory === entry.path ? (
        <NameInput depth={depth + 1} kind={editing.creating.kind} />
      ) : null}
      {open ? <Rows path={entry.path} state={state} depth={depth + 1} /> : null}
    </>
  );
}

/** The in-place name box: a new file or folder, or a rename. Enter commits, Escape leaves it as it was. */
function NameInput({ depth, kind, renaming }: { depth: number; kind: 'file' | 'folder'; renaming?: FsEntry }) {
  const editing = useEditing();
  const [value, setValue] = useState(renaming?.name ?? '');
  const [busy, setBusy] = useState(false);
  const input = useRef<HTMLInputElement | null>(null);

  useEffect(() => {
    const element = input.current;

    if (element === null) {
      return;
    }

    element.focus();

    /* A rename selects the name without its extension, the way every file manager does. */
    const dot = renaming === undefined || renaming.dir ? -1 : renaming.name.lastIndexOf('.');

    element.setSelectionRange(0, dot > 0 ? dot : element.value.length);
  }, [renaming]);

  const done = (): void => {
    editing.setRenaming(null);
    editing.setCreating(null);
  };

  const submit = (event: FormEvent): void => {
    event.preventDefault();

    if (busy) {
      return;
    }

    if (value.trim() === '' || (renaming !== undefined && value.trim() === renaming.name)) {
      done();

      return;
    }

    setBusy(true);

    const work =
      renaming !== undefined
        ? renamePath(renaming.path, value)
        : editing.creating === null
          ? Promise.resolve(false)
          : kind === 'file'
            ? createFile(editing.creating.directory, value)
            : createFolder(editing.creating.directory, value);

    void work.then((ok) => {
      setBusy(false);

      if (ok) {
        done();
      }
    });
  };

  return (
    <form className="files-name flex items-center gap-[5px] py-[1px] pr-[4px]" style={{ paddingLeft: `${4 + depth * 10 + 16}px` }} onSubmit={submit}>
      {kind === 'folder' ? (
        <Folder size={11} aria-hidden="true" className="shrink-0 text-accent" />
      ) : (
        <FileIcon size={11} aria-hidden="true" className="shrink-0 text-text-muted" />
      )}
      {/* The wrapper paints the box: a control itself stays transparent (the smoke gate's rule). */}
      <span className="flex h-[20px] min-w-0 flex-1 rounded-sm border border-border-focus bg-bg-input">
      <input
        ref={input}
        className="h-full min-w-0 flex-1 bg-transparent px-[5px] font-mono text-[11px] text-text-primary"
        value={value}
        placeholder={strings.files.namePlaceholder}
        aria-label={renaming === undefined ? (kind === 'file' ? strings.files.newFile : strings.files.newFolder) : strings.files.rename}
        disabled={busy}
        spellCheck={false}
        onChange={(event) => setValue(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === 'Escape') {
            event.preventDefault();
            event.stopPropagation();
            done();
          }
        }}
        onBlur={() => {
          if (!busy) {
            done();
          }
        }}
      />
      </span>
    </form>
  );
}

/**
 * The folder's search bar (0.11.0) - always above the tree, searching **as you type**.
 *
 * Two kinds of answer, in the order a person wants them: files and folders whose *name* matches
 * (click a file to open it, a folder to unfold the tree down to it), then the lines inside files
 * that contain the words. Both come from one `fs.search`, debounced 300ms so a fast typist asks
 * once; a stale answer is dropped by sequence number rather than trusted by luck. Escape clears.
 */
function FolderSearch({ root }: { root: string }) {
  const [query, setQuery] = useState('');
  const [result, setResult] = useState<FolderSearchResult | null>(null);
  const [busy, setBusy] = useState(false);
  const [asked, setAsked] = useState('');
  const sequence = useRef(0);

  useEffect(() => {
    const trimmed = query.trim();

    if (trimmed.length < 2) {
      sequence.current += 1;
      setResult(null);
      setBusy(false);

      return;
    }

    const mine = (sequence.current += 1);
    const timer = window.setTimeout(() => {
      setBusy(true);
      void searchFolder(trimmed).then((found) => {
        if (sequence.current !== mine) {
          return;
        }

        setBusy(false);
        setAsked(trimmed);
        setResult(found);
      });
    }, 300);

    return () => window.clearTimeout(timer);
  }, [query]);

  /* The search clears with the folder it searched: a stale hit list over a new root would lie. */
  useEffect(() => {
    setQuery('');
    setResult(null);
  }, [root]);

  const groups = useMemo(() => {
    const byFile = new Map<string, SearchHit[]>();

    for (const hit of result?.hits ?? []) {
      byFile.set(hit.path, [...(byFile.get(hit.path) ?? []), hit]);
    }

    return [...byFile.entries()];
  }, [result]);

  const relative = (path: string): string => {
    const trimmed = path.startsWith(root) ? path.slice(root.length).replace(/^[\\/]/, '') : path;

    return trimmed === '' ? baseName(path) : trimmed;
  };

  const empty = result !== null && result.files.length === 0 && result.hits.length === 0;

  return (
    <div className="files-search mb-[6px]">
      <div className="flex h-[24px] items-center gap-[5px] rounded-md border border-border-subtle bg-bg-input px-[6px] transition-colors duration-fast ease-ease focus-within:border-border-focus">
        <Search size={11} aria-hidden="true" className="shrink-0 text-text-muted" />
        <input
          className="h-full min-w-0 flex-1 bg-transparent text-[11.5px] text-text-primary placeholder:text-text-muted"
          placeholder={strings.files.searchPlaceholder}
          aria-label={strings.files.search}
          value={query}
          spellCheck={false}
          onChange={(event) => setQuery(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === 'Escape' && query !== '') {
              event.preventDefault();
              event.stopPropagation();
              setQuery('');
            }
          }}
        />
        {busy ? <Loader size={10} className="shrink-0 animate-spin text-text-muted" aria-hidden="true" /> : null}
        {query === '' ? null : (
          <button type="button" className={HEADER_BUTTON} aria-label={strings.files.closeSearch} title={strings.files.closeSearch} onClick={() => setQuery('')}>
            <X size={10} aria-hidden="true" />
          </button>
        )}
      </div>

      {result === null ? null : empty ? (
        <div className="px-[2px] pt-[6px] text-[11px] text-text-muted">{strings.files.noHits(asked)}</div>
      ) : (
        <div className="mt-[4px] max-h-[300px] overflow-y-auto rounded-md border border-border-subtle bg-bg-raised p-[4px]">
          {result.files.length === 0 ? null : (
            <div className="mb-[4px]">
              <div className="px-[2px] pb-[2px] text-[10px] font-semibold uppercase tracking-wide text-text-muted">
                {strings.files.nameHits(result.files.length)}
              </div>
              {result.files.map((found) => (
                <button
                  key={found.path}
                  type="button"
                  className="flex w-full items-center gap-[5px] rounded-sm px-[4px] py-[2px] text-left hover:bg-bg-hover"
                  title={found.path}
                  onClick={() => {
                    if (found.dir) {
                      void revealFolder(found.path);

                      return;
                    }

                    void openFile(found.path, baseName(found.path));
                  }}
                >
                  {found.dir ? (
                    <Folder size={11} aria-hidden="true" className="shrink-0 text-accent" />
                  ) : (
                    <FileIcon size={11} aria-hidden="true" className="shrink-0 text-text-muted" />
                  )}
                  <span className="min-w-0 truncate font-mono text-[10.5px] text-text-primary">{relative(found.path)}</span>
                </button>
              ))}
            </div>
          )}

          {result.hits.length === 0 ? null : (
            <>
              <div className="px-[2px] pb-[2px] text-[10px] font-semibold uppercase tracking-wide text-text-muted">
                {strings.files.hits(result.hits.length)}
              </div>
              {groups.map(([path, fileHits]) => (
                <div key={path} className="mb-[4px]">
                  <div className="truncate px-[2px] font-mono text-[10.5px] text-text-secondary" title={path}>
                    {relative(path)}
                  </div>
                  {fileHits.map((hit) => (
                    <button
                      key={`${hit.path}:${hit.line}`}
                      type="button"
                      className="flex w-full items-baseline gap-[6px] rounded-sm px-[4px] py-[1px] text-left hover:bg-bg-hover"
                      onClick={() => void openFile(hit.path, baseName(hit.path), hit.line)}
                    >
                      <span className="shrink-0 font-mono text-[10px] tabular-nums text-text-muted">{hit.line}</span>
                      <span className="min-w-0 truncate font-mono text-[10.5px] text-text-primary">{hit.text}</span>
                    </button>
                  ))}
                </div>
              ))}
            </>
          )}
        </div>
      )}
    </div>
  );
}
