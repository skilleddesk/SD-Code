import { strings } from '../strings';
import type { ConsoleLine } from './types';
import { create } from 'zustand';

/**
 * Right panel store - which of the six tabs is showing, on which session, and how wide the panel is
 * (spec sections 7.7-7.12, 7.2).
 *
 * The tab is sticky *per session*: the spec's rule is that switching to Preview on one chat and
 * then to another chat brings back whatever that other chat was last looking at. That is why the
 * store keeps `activeTab` (the fallback, and what a brand-new session inherits) next to
 * `tabBySession` (the memory), and why `tabForSession()` exists rather than a bare field read.
 *
 * The Console's entries live here too, because the tab strip's badge counts them: the badge has to
 * read the same list the Console tab renders, and the count is the number of *distinct* logged
 * errors - which is what the prototype's badge shows, not the sum of their occurrences.
 */

/** The six tabs, in the fixed order of spec section 7.7-7.12. */
export type PanelTabId = 'preview' | 'console' | 'timemachine' | 'duel' | 'verify' | 'analytics';

export interface PanelTabDefinition {
  id: PanelTabId;
  label: string;
  /** Lucide icon name; `RightPanel.tsx` owns the name-to-component map. */
  icon: 'eye' | 'terminal' | 'clock' | 'swords' | 'check' | 'chart';
}

/** Tab strip order is fixed by the spec: Preview, Console, Time Machine, Duel, Verify, Analytics. */
export const PANEL_TABS: readonly PanelTabDefinition[] = [
  { id: 'preview', label: strings.rightPanel.tabs.preview, icon: 'eye' },
  { id: 'console', label: strings.rightPanel.tabs.console, icon: 'terminal' },
  { id: 'timemachine', label: strings.rightPanel.tabs.timemachine, icon: 'clock' },
  { id: 'duel', label: strings.rightPanel.tabs.duel, icon: 'swords' },
  { id: 'verify', label: strings.rightPanel.tabs.verify, icon: 'check' },
  { id: 'analytics', label: strings.rightPanel.tabs.analytics, icon: 'chart' },
];

/** The panel folds to 320px and grows to 760px; 400px is the layout default (spec section 7.2). */
export const PANEL_MIN_WIDTH = 320;
export const PANEL_MAX_WIDTH = 760;

/** A console line, already deduplicated: `count` is how often the same source line was logged. */
export type ConsoleLevel = 'error' | 'warn' | 'info';

/**
 * One console row (spec section 7.8). The *list* is not here: it is folded from `ConsoleError`
 * events in `reducer.ts`, because a logged error is a fact about the session rather than a detail of
 * the panel. The tab strip's badge counts the same list the Console tab draws.
 */
export type ConsoleEntry = ConsoleLine;

/**
 * A Time Machine checkpoint (spec section 7.9). The seed copy lives in src/strings.ts; the shape
 * lives here, so a tab can hold its seed in a variable of this type and ask whether it is empty - a
 * literal tuple straight out of `strings` has a known length, and `length === 0` would be a type
 * error rather than a runtime branch.
 */
export interface TimeMachineEntry {
  turn: number;
  /** Relative age: `now`, `2 min ago`, `8 min ago`. */
  when: string;
  title: string;
}

/** One Verify row (spec section 7.11). */
export interface VerifyRow {
  name: string;
  pass: boolean;
  /** The check's wall-clock time: `2.1s`. */
  time: string;
}

export interface RightPanelState {
  /** The tab a session with no memory of its own opens on (spec section 7.7). */
  activeTab: PanelTabId;
  /** The sticky per-session tab, keyed by session id. */
  tabBySession: Record<string, PanelTabId>;
  /** Panel width in pixels, between PANEL_MIN_WIDTH and PANEL_MAX_WIDTH. */
  width: number;
}

export interface RightPanelActions {
  /**
   * Pick a tab. `sessionId` is optional so the initial render (no session open yet) still works;
   * when it is given, the choice is remembered for that session.
   */
  setActiveTab: (tab: PanelTabId, sessionId?: string | null) => void;
  /** The left-edge drag handle (spec section 7.2: minimum 320px). */
  setWidth: (width: number) => void;
}

const initialRightPanelState: RightPanelState = {
  activeTab: 'preview',
  tabBySession: {},
  width: 400,
};

export const useRightPanelStore = create<RightPanelState & RightPanelActions>()((set, get) => ({
  ...initialRightPanelState,

  setActiveTab: (tab, sessionId) =>
    set((state) => ({
      activeTab: tab,
      tabBySession:
        sessionId === undefined || sessionId === null
          ? state.tabBySession
          : { ...state.tabBySession, [sessionId]: tab },
    })),

  setWidth: (width) => {
    const clamped = Math.min(PANEL_MAX_WIDTH, Math.max(PANEL_MIN_WIDTH, Math.round(width)));

    if (get().width !== clamped) {
      set({ width: clamped });
    }
  },
}));

/** The tab a session is looking at: its own memory, or the panel-wide fallback. */
export function tabForSession(state: RightPanelState, sessionId: string | null): PanelTabId {
  if (sessionId === null) {
    return state.activeTab;
  }

  return state.tabBySession[sessionId] ?? state.activeTab;
}

/** Distinct errors - the Console tab's badge (spec section 7.8), from the folded list. */
export { consoleErrorCount } from './reducer';
