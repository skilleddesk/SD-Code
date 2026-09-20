import { create } from 'zustand';

import type { SdcpEvent } from '../../../protocol/types';
import { eventLog } from './events';
import {
  applyEvent,
  createInitialState,
  selectActiveSession as projectActiveSession,
  selectCheckpoints,
  selectConsole,
  selectDuel,
  selectHosts as projectHosts,
  selectPermission,
  selectRegistry,
  selectToasts,
  selectTurn,
  type HostSession,
} from './reducer';
import { usePrefsStore } from './prefs';
import type { AppEvent, AppState, CheckpointView, ConsoleLine, DuelView, HostView, PermissionView, ToastRecord, TurnView } from './types';

/**
 * The app store - the event log's projection, plus the one action that can change it
 * (master spec section 3.3).
 *
 * There is exactly one writer: `dispatch()`. It appends to the log (`events.ts`), the log notifies
 * its subscribers, and the single subscriber below folds the new event with `applyEvent`. A
 * component can therefore never hold a fact the log does not have - which is what "the UI never
 * mutates state directly; it dispatches intents" means in code.
 *
 *   UI click ──▶ intents.ts ──▶ sdcp_call (SDCP) ──▶ daemon appends ──▶ notification
 *                                                                        │
 *                                       eventLog.accept() ◀──────────────┘
 *                                                │
 *                                       applyEvent() ──▶ useAppStore
 *
 * `dispatch()` is also the seam the acceptance test uses: replaying old events and reading the
 * projected state is the same call the daemon's notifications take.
 *
 * UI preferences are deliberately absent (see src/store/prefs.ts): `selectActiveSession()` reads
 * the tab the user picked from there and the session itself from here, because the daemon owns the
 * session and the browser owns which one you were looking at.
 */

export interface AppStoreState extends AppState {
  /**
   * Appends one event to the log. `payload.seq`/`ts` are only supplied when folding a daemon
   * notification, whose counters are authoritative.
   */
  dispatch: (event: SdcpEvent, payload?: Partial<AppEvent>) => AppEvent;
}

/** The initial fold - the demo log of `reducer.ts#seedEvents`. */
const INITIAL_STATE = createInitialState();

export const useAppStore = create<AppStoreState>()((set, get) => ({
  ...INITIAL_STATE,

  dispatch: (event, payload) => {
    const entry = eventLog.append(event, payload);

    set(applyEvent(get(), entry));

    return entry;
  },
}));

/**
 * The log's only subscriber. It is registered at module scope rather than from an effect so that an
 * event accepted before React mounts (a daemon handshake, a replay) still lands in the store.
 */
eventLog.subscribe((entry) => {
  useAppStore.setState((state) => applyEvent(state, entry));
});

/** Same thing without the hook, for stores and intents that are not components. */
export function dispatch(event: SdcpEvent, payload?: Partial<AppEvent>): AppEvent {
  return eventLog.append(event, payload);
}

/* ------------------------------------------------------------------------------------------------
 * The store's read API (spec section 3.3 names `selectActiveSession()` and `selectHosts()`).
 * ---------------------------------------------------------------------------------------------- */

export function selectHosts(): HostView[] {
  return projectHosts(useAppStore.getState());
}

/** The focused session: the log's session, the prefs' tab. */
export function selectActiveSession(): HostSession | null {
  return projectActiveSession(useAppStore.getState(), usePrefsStore.getState().activeTab);
}

export function selectPermissionState(): PermissionView | null {
  return selectPermission(useAppStore.getState());
}

export function selectToastsState(): ToastRecord[] {
  return selectToasts(useAppStore.getState());
}

export function selectCheckpointState(sessionId: string | null = null): CheckpointView[] {
  return selectCheckpoints(useAppStore.getState(), sessionId);
}

export function selectConsoleState(): ConsoleLine[] {
  return selectConsole(useAppStore.getState());
}

export function selectDuelState(sessionId: string | null = null): DuelView | null {
  return selectDuel(useAppStore.getState(), sessionId);
}

export function selectTurnState(sessionId: string | null = null): TurnView | null {
  return selectTurn(useAppStore.getState(), sessionId);
}

export function selectRegistryState() {
  return selectRegistry(useAppStore.getState());
}

/** A test seam: the whole projected state, without the dispatch handle. */
export function currentState(): AppState {
  const { dispatch, ...state } = useAppStore.getState();

  /* `dispatch` is destructured only to be left out: the projected state is data, and handing a test
     the store's dispatch would let it bypass the event log - which is the one thing the store's
     design forbids (spec section 3.3). */
  void dispatch;

  return state;
}
