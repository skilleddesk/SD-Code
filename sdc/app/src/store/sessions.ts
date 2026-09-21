import { useMemo } from 'react';

import { useAppStore } from './store';
import { ensureSplitSecondary, usePrefsStore } from './prefs';
import {
  closeSession,
  forkSession as forkIntent,
  newChatOnHost as newChatIntent,
  renameSession as renameIntent,
} from './intents';
import type { HostView, SessionView } from './types';

/**
 * The session facade - spec sections 7.3, 7.4 and 9.15 - over two stores that each own half the
 * truth (master spec section 3.3):
 *
 *   hosts, sessions   the event log's fold (`store/store.ts` → `reducer.ts`). The daemon owns them;
 *                     the UI can only *ask* for a change by dispatching an intent.
 *   tabs, focus       UI preferences (`store/prefs.ts`). Which chat you were looking at is not a
 *                     fact about the world, so it is not an event.
 *
 * The composition is memoized on its inputs, so the object a component destructures has a stable
 * identity - which is what keeps `const { hosts, activeTab } = useSessionsStore()` a one-line read
 * instead of a selector with a `shallow` comparator at forty call sites.
 *
 * The verbs keep the shape they always had, and each one goes where its truth lives:
 * `renameSession` and `deleteSession` call the daemon (`session.update` / `session.close`), while
 * `openSession`, `closeTab` and the filter write preferences.
 */

export type HostType = HostView['type'];
export type HostStatus = HostView['status'];
export type SessionState = SessionView['state'];
export type AttentionReason = NonNullable<SessionView['attention']>;
export type Session = SessionView;
export type Host = HostView;
export type SessionRef = { host: HostView; session: SessionView };

/** Everything the sidebar, the tab strip and the prompt area read, plus the verbs they call. */
export interface SessionsStore {
  hosts: Host[];
  openTabs: string[];
  activeTab: string | null;
  activeHostId: string;
  splitSecondary: string | null;
  collapsedHosts: Record<string, boolean>;
  filter: string;
  counter: number;
  openSession: (id: string) => void;
  closeTab: (id: string) => void;
  newChatOnHost: (hostId: string) => void;
  renameSession: (id: string, title: string) => void;
  deleteSession: (id: string) => void;
  /** Branch this chat into a new one, and open it (0.7.8). */
  forkSession: (id: string, title: string) => void;
  toggleHostCollapsed: (hostId: string) => void;
  filterSessions: (query: string) => void;
  setActiveHost: (hostId: string) => void;
  setSplitSecondary: (id: string | null) => void;
  ensureSplitSecondary: () => void;
}

/**
 * The verbs, defined once at module scope so they never take part in the memo key.
 *
 * Exported as `sessionActions` as well, because a *command* (the palette, the keymap) is not a component and
 * cannot call a hook - `_verify`-style probes and the registry both need the same behaviour as the sidebar's
 * buttons, not a second implementation of it.
 *
 * `newChatOnHost` is the one that spans both halves: the daemon appends `SessionOpened` *before* it
 * answers the call, so by the time the promise resolves the session is in the log and the tab can
 * be opened on a session that already exists.
 */
export const sessionActions = {
  openSession: (id: string): void => {
    usePrefsStore.getState().openTab(id);
  },

  closeTab: (id: string): void => {
    usePrefsStore.getState().closeTab(id);
  },

  newChatOnHost: (hostId: string): void => {
    usePrefsStore.getState().setActiveHost(hostId);

    void newChatIntent(hostId).then((created) => {
      if (created !== null) {
        usePrefsStore.getState().openTab(created);
      }
    });
  },

  renameSession: (id: string, title: string): void => {
    void renameIntent(id, title);
  },

  deleteSession: (id: string): void => {
    usePrefsStore.getState().closeTab(id);
    void closeSession(id);
  },

  forkSession: (id: string, title: string): void => {
    void forkIntent(id, title).then((forked) => {
      if (forked !== null) {
        usePrefsStore.getState().openTab(forked);
      }
    });
  },

  toggleHostCollapsed: (hostId: string): void => {
    usePrefsStore.getState().toggleCollapsed(hostId);
  },

  filterSessions: (query: string): void => {
    usePrefsStore.getState().setFilter(query);
  },

  setActiveHost: (hostId: string): void => {
    usePrefsStore.getState().setActiveHost(hostId);
  },

  setSplitSecondary: (id: string | null): void => {
    usePrefsStore.getState().setSplitSecondary(id);
  },

  ensureSplitSecondary,
};

/** The memoized composition: eight inputs, one object. */
function useSessionsSlice(): SessionsStore {
  const hosts = useAppStore((state) => state.hosts);
  const openTabs = usePrefsStore((state) => state.openTabs);
  const activeTab = usePrefsStore((state) => state.activeTab);
  const activeHostId = usePrefsStore((state) => state.activeHostId);
  const splitSecondary = usePrefsStore((state) => state.splitSecondary);
  const collapsedHosts = usePrefsStore((state) => state.collapsedHosts);
  const filter = usePrefsStore((state) => state.filter);
  const counter = usePrefsStore((state) => state.counter);

  return useMemo(
    () => ({
      hosts,
      openTabs,
      activeTab,
      activeHostId,
      splitSecondary,
      collapsedHosts,
      filter,
      counter,
      ...sessionActions,
    }),
    [hosts, openTabs, activeTab, activeHostId, splitSecondary, collapsedHosts, filter, counter],
  );
}

export function useSessionsStore(): SessionsStore;
export function useSessionsStore<T>(selector: (state: SessionsStore) => T): T;
export function useSessionsStore<T>(selector?: (state: SessionsStore) => T): SessionsStore | T {
  const slice = useSessionsSlice();

  return selector === undefined ? slice : selector(slice);
}

/* ------------------------------------------------------------------------------------------------
 * Derivations. They live in `reducer.ts` (they are pure functions of the folded state) and are
 * re-exported here, so every existing `import { findSession } from '../store/sessions'` still
 * resolves to the single implementation.
 * ---------------------------------------------------------------------------------------------- */

export {
  allSessions,
  connectionState,
  findSession,
  formatRelativeTime,
  matchesFilter,
  orderedSessions,
  sessionCount,
  unreachableHosts,
  type ConnectionState,
  type HostSession,
} from './reducer';

