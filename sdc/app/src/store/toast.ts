import { useMemo } from 'react';

import { toast as raiseToast } from './intents';
import { dispatch, useAppStore } from './store';
import type { ToastRecord } from './types';

/**
 * The toast facade - spec section 9.14.
 *
 * Toasts are events like everything else: the daemon's `Toast` event (or the UI's own, raised by
 * `intents.toast()`) lands in the log, the reducer collects it, and the stack renders the
 * collection. Dismissal is an event too (`ToastDismissed`), which is why this file has no timer of
 * its own - the *component* holds the 3s timer, and it reports what happened rather than owning
 * state (master spec section 3.3).
 *
 * `toast(...)` is the same thing without the hook, for the places that are not components.
 */

export type Toast = ToastRecord;

export interface ToastStore {
  toasts: Toast[];
  dismiss: (id: number) => void;
}

const actions = {
  dismiss: (id: number): void => {
    dispatch({ type: 'ToastDismissed', id });
  },
};

export function useToastStore(): ToastStore;
export function useToastStore<T>(selector: (state: ToastStore) => T): T;
export function useToastStore<T>(selector?: (state: ToastStore) => T): ToastStore | T {
  const toasts = useAppStore((state) => state.toasts);

  const slice = useMemo(() => ({ toasts, ...actions }), [toasts]);

  return selector === undefined ? slice : selector(slice);
}

/** Show a message. Repeat calls stack upward, newest at the bottom. */
export function toast(message: string, action?: string, holdMs?: number): void {
  raiseToast(message, action, holdMs);
}
