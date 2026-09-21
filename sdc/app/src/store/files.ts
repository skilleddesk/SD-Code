import { create } from 'zustand';

import type { FsEntry } from '../../../protocol/types';

/**
 * The file tree's view state - which folders are open, what was read, and the file on screen (0.7.7).
 *
 * View state, not world state, which is why it lives here and not in the event log: expanding a folder
 * is what *this window* is looking at, the way `prefs.openTabs` is (master spec section 3.3). Nothing
 * about the folder itself is decided here - `fs.list` answers the entries and the daemon owns the
 * project, so a reload re-reads both.
 *
 * Three things are deliberately *not* here:
 *
 *   * **the root.** The tree shows the active chat's folder, and that is a fact about the session
 *     (`session.projectRoot`), so the component reads it from the session and this store is told what
 *     to draw. A copy of it here would be a second answer to "which folder am I in".
 *   * **the requests.** `fs.list` and `fs.read` are called by `store/intents.ts`, because the UI never
 *     talks to the daemon itself (P3); this store only records what came back.
 *   * **the ordering.** Entries arrive name-sorted; the tree draws folders first, and that is a
 *     presentation choice made where the entries are drawn (`FilesSection.tsx`) rather than a rule the
 *     daemon has to know.
 */

/** One directory's contents, as `fs.list` answered them. */
export interface DirectoryView {
  entries: FsEntry[];
  /** How many names the daemon's guard kept out of the listing, so the tree can say so. */
  hidden: number;
}

/** The file the Preview tab is showing. */
export interface OpenFileView {
  path: string;
  name: string;
  text: string;
  sha256: string;
  bytes: number;
  /** True when the daemon returned only the first megabyte of a larger file. */
  truncated: boolean;
}

/** The branch and changed-file count for the chat's folder, as `git.status` answered it. */
export interface GitView {
  branch: string;
  dirty: number;
}

export interface FilesState {
  /** The folder being shown, or `null` when the chat has none. */
  root: string | null;
  /** Read directories, keyed by their absolute path. Absent means "not read yet". */
  directories: Record<string, DirectoryView>;
  /** The directories currently being read, so a row can say so instead of looking broken. */
  loading: string[];
  /** The directories whose row is open. A list rather than a map: it is small and it is ordered. */
  expanded: string[];
  /** The daemon's own words, when a read failed. */
  error: string | null;
  /** The file on screen, or `null`. */
  open: OpenFileView | null;
  /** The path being opened, so the row that was clicked can show a spinner. */
  opening: string | null;
  /** The folder's git state (0.7.9), or `null` for a folder with no git - which is not a failure. */
  git: GitView | null;
  /** The working tree's diff (0.7.9), shown in the Preview instead of a file. */
  diff: string | null;
}

export interface FilesActions {
  /** Point the tree at a folder (or at nothing). Replaces the *root*, and forgets the old one's contents. */
  setRoot: (root: string | null) => void;
  startLoading: (path: string) => void;
  /** A directory arrived: its entries, and what the guard hid. */
  fill: (path: string, directory: DirectoryView) => void;
  fail: (message: string) => void;
  setExpanded: (path: string, open: boolean) => void;
  startOpening: (path: string) => void;
  /** The file arrived - or `null`, when one was closed. */
  setOpen: (file: OpenFileView | null) => void;
  /** The folder's git state - or `null` when there is none to show. */
  setGit: (git: GitView | null) => void;
  /** The working tree's diff - or `null` when the diff was closed. */
  setDiff: (patch: string | null) => void;
  /** Forget everything, for a chat whose folder changed: the old paths are not in the new tree. */
  reset: () => void;
}

const initialFilesState: FilesState = {
  root: null,
  directories: {},
  loading: [],
  expanded: [],
  error: null,
  open: null,
  opening: null,
  git: null,
  diff: null,
};

export const useFilesStore = create<FilesState & FilesActions>()((set, get) => ({
  ...initialFilesState,

  setRoot: (root) => {
    if (get().root === root) {
      return;
    }

    /* A different folder means every path this store holds is wrong - the entries, the open folders and
       the open file all belong to the old one. */
    set({ ...initialFilesState, root });
  },

  startLoading: (path) =>
    set((state) =>
      state.loading.includes(path) ? state : { ...state, loading: [...state.loading, path], error: null },
    ),

  fill: (path, directory) =>
    set((state) => ({
      ...state,
      directories: { ...state.directories, [path]: directory },
      loading: state.loading.filter((candidate) => candidate !== path),
    })),

  fail: (message) =>
    set((state) => ({ ...state, loading: [], opening: null, error: message })),

  setExpanded: (path, open) =>
    set((state) => ({
      ...state,
      expanded: open
        ? state.expanded.includes(path)
          ? state.expanded
          : [...state.expanded, path]
        : state.expanded.filter((candidate) => candidate !== path),
    })),

  startOpening: (path) => set((state) => ({ ...state, opening: path, error: null })),

  setOpen: (file) => set((state) => ({ ...state, open: file, opening: null, diff: null })),

  setGit: (git) => set((state) => ({ ...state, git })),

  /* Opening a diff takes the tab over from the file it was showing - the two are the same surface, and a
     diff *beside* a file would be two answers to one question on a 400px panel. */
  setDiff: (patch) => set((state) => ({ ...state, diff: patch, open: patch === null ? state.open : null })),

  reset: () => set({ ...initialFilesState }),
}));

/** Is this folder's row open? */
export function isExpanded(state: FilesState, path: string): boolean {
  return state.expanded.includes(path);
}

/** Is this directory being read? */
export function isLoading(state: FilesState, path: string): boolean {
  return state.loading.includes(path);
}
