import { useEffect } from 'react';
import { create } from 'zustand';

import { strings } from '../strings';
import { useAppStore } from './store';
import { toast } from './toast';

/**
 * UI preferences - the half of the state that is the *browser's*, not the daemon's
 * (master spec section 3.3).
 *
 * The spec is explicit about the split:
 *
 *   > Persist to localStorage ONLY for UI prefs (theme, layout, activeTab); server state persists
 *   > via daemon.
 *
 * So the tabs, the focused session, which host groups are folded, the sidebar filter and the id
 * counter live here; the hosts and sessions they point at live in the event log's fold
 * (`reducer.ts`). That is why a "new chat" is two steps - the daemon appends `SessionOpened` and
 * this store records that its tab is open - and why a closed tab is not a closed session.
 *
 * Only three fields survive a reload (`openTabs`, `activeTab`, `activeHostId`): everything else is
 * either re-derived or was never worth remembering, and a stale `splitSecondary` pointing at a
 * session the daemon no longer knows about would be a bug on the second launch.
 *
 * The counter is a preference rather than a daemon fact on purpose: it only names tabs created in
 * this window, and a daemon session id never comes from here.
 */

const STORAGE_KEY = 'sdc.prefs.v1';

export interface Prefs {
  /** Tab order, left to right (spec section 7.4). */
  openTabs: string[];
  /** The focused session, or null when no chat is open - the empty state of spec section 7.13. */
  activeTab: string | null;
  /** The host the topbar pill names (spec section 7.1, row 3). */
  activeHostId: string;
  /** The second pane's session in split view; null until split is turned on (spec section 9.15). */
  splitSecondary: string | null;
  /** `host-group.collapsed`, keyed by host id (spec section 7.3). */
  collapsedHosts: Record<string, boolean>;
  /** The sidebar's `Filter chats…` box; only session rows react to it (spec section 7.3). */
  filter: string;
  /** Ids for things created at runtime, so a new tab never collides with the seed. */
  counter: number;
}

export interface PrefsActions {
  openTab: (id: string) => void;
  closeTab: (id: string) => void;
  focusTab: (id: string) => void;
  setOpenTabs: (ids: string[]) => void;
  setActiveHost: (hostId: string) => void;
  setSplitSecondary: (id: string | null) => void;
  toggleCollapsed: (hostId: string) => void;
  setFilter: (query: string) => void;
  /** Claims the next id: `n100`, `n101`, … */
  nextId: () => string;
}

/** The three fields a reload restores; everything else starts fresh. */
interface PersistedPrefs {
  openTabs: string[];
  activeTab: string | null;
  activeHostId: string;
}

const DEFAULT_PREFS: Prefs = {
  /* The prototype's initial strip: three tabs, the first focused, `local` as the active host. */
  openTabs: ['s1', 's2', 's4'],
  activeTab: 's1',
  activeHostId: 'local',
  splitSecondary: null,
  collapsedHosts: {},
  filter: '',
  counter: 100,
};

/** Reads the remembered three; a corrupt or absent blob is simply the defaults. */
export function loadPrefs(): Prefs {
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);

    if (raw === null) {
      return { ...DEFAULT_PREFS };
    }

    const stored = JSON.parse(raw) as Partial<PersistedPrefs>;

    return {
      ...DEFAULT_PREFS,
      openTabs: Array.isArray(stored.openTabs) ? stored.openTabs : DEFAULT_PREFS.openTabs,
      activeTab: stored.activeTab ?? DEFAULT_PREFS.activeTab,
      activeHostId: stored.activeHostId ?? DEFAULT_PREFS.activeHostId,
    };
  } catch {
    /* Private mode, a disabled storage API or a half-written blob: the defaults are always fine. */
    return { ...DEFAULT_PREFS };
  }
}

function persist(prefs: Prefs): void {
  try {
    const snapshot: PersistedPrefs = {
      openTabs: prefs.openTabs,
      activeTab: prefs.activeTab,
      activeHostId: prefs.activeHostId,
    };

    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(snapshot));
  } catch {
    /* Losing a preference is not worth failing a click over. */
  }
}

export const usePrefsStore = create<Prefs & PrefsActions>()((set, get) => ({
  ...loadPrefs(),

  openTab: (id) => {
    const state = get();

    set({
      openTabs: state.openTabs.includes(id) ? state.openTabs : [...state.openTabs, id],
      activeTab: id,
    });
    persist(get());
  },

  closeTab: (id) => {
    const state = get();
    const index = state.openTabs.indexOf(id);

    if (index < 0) {
      return;
    }

    const openTabs = state.openTabs.filter((tab) => tab !== id);
    const activeTab =
      state.activeTab === id ? (openTabs[index] ?? openTabs[index - 1] ?? null) : state.activeTab;

    set({
      openTabs,
      activeTab,
      splitSecondary:
        state.splitSecondary !== null && !openTabs.includes(state.splitSecondary)
          ? null
          : state.splitSecondary,
    });
    persist(get());
  },

  focusTab: (id) => {
    if (get().activeTab !== id) {
      set({ activeTab: id });
      persist(get());
    }
  },

  setOpenTabs: (ids) => {
    set({ openTabs: [...ids] });
    persist(get());
  },

  setActiveHost: (hostId) => {
    if (get().activeHostId !== hostId) {
      set({ activeHostId: hostId });
      persist(get());
    }
  },

  setSplitSecondary: (id) => {
    if (get().splitSecondary !== id) {
      set({ splitSecondary: id });
    }
  },

  toggleCollapsed: (hostId) =>
    set((state) => ({
      collapsedHosts: { ...state.collapsedHosts, [hostId]: !state.collapsedHosts[hostId] },
    })),

  setFilter: (query) => {
    if (get().filter !== query) {
      set({ filter: query });
    }
  },

  nextId: () => {
    const counter = get().counter + 1;

    set({ counter });

    return `n${counter}`;
  },
}));

/**
 * Picks a second pane if there is not one yet - the first open tab that is not the active one.
 * `MainContent` calls it when split view turns on.
 */
export function ensureSplitSecondary(): void {
  const state = usePrefsStore.getState();
  const current = state.splitSecondary;

  if (current !== null && state.openTabs.includes(current) && current !== state.activeTab) {
    return;
  }

  state.setSplitSecondary(state.openTabs.find((tab) => tab !== state.activeTab) ?? null);
}

/**
 * The one startup toast of spec section 9.14 / the prototype: 1.4s after load, the tip about the
 * plug icon, with a `Got it` chip. It is a hook rather than a `setTimeout` at module scope so the
 * timer belongs to the app's lifetime and is cancelled if the app unmounts (a StrictMode
 * double-mount in development would otherwise show it twice).
 */
export function usePrefixHint(): void {
  /* The one startup toast (spec section 9.14). It used to announce a seeded tip about a project the
     window had not opened; a fresh install gets the sentence that helps instead: what the window is
     waiting for. It disappears on the first real connection, because then it is no longer true. */
  useEffect(() => {
    const timer = window.setTimeout(() => {
      if (useAppStore.getState().hosts.length === 0) {
        toast(strings.daemon.waiting);
      }
    }, 1400);

    return () => {
      window.clearTimeout(timer);
    };
  }, []);
}
