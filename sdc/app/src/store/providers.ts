import { useMemo } from 'react';

import { dispatch, useAppStore } from './store';
import type { ProviderView } from './types';

/**
 * The provider facade - spec sections 7.1 row 7 and 9.10.
 *
 * The list itself is the event log's `ProviderStatus` fold: a connect flow ends with the daemon
 * appending one, and the card, the topbar dot and the status bar's count all move together because
 * they read the same array. `setProviderStatus` is the UI's only write, and it writes an *event* -
 * which is how a Manage button could mark a key rotated without ever holding the key.
 *
 * The full Provider Hub is `src/modals/ProviderHub.tsx`; this module is only the two facts the
 * chrome needs plus the one action.
 */

export type ProviderKind = ProviderView['kind'];
export type ProviderStatus = ProviderView['status'];
export type Provider = ProviderView;

export interface ProvidersStore {
  providers: Provider[];
  setProviderStatus: (id: string, status: ProviderStatus, account?: string) => void;
}

const actions = {
  setProviderStatus: (id: string, status: ProviderStatus, account?: string): void => {
    dispatch({ type: 'ProviderStatus', id, status, ...(account === undefined ? {} : { account }) });
  },
};

export function useProviderStore(): ProvidersStore;
export function useProviderStore<T>(selector: (state: ProvidersStore) => T): T;
export function useProviderStore<T>(selector?: (state: ProvidersStore) => T): ProvidersStore | T {
  const providers = useAppStore((state) => state.providers);

  const slice = useMemo(() => ({ providers, ...actions }), [providers]);

  return selector === undefined ? slice : selector(slice);
}

/** True while any provider still needs authentication - the `has-dot` condition (7.1, row 7). */
export { anyProviderNeedsAuth, connectedProviderCount } from './reducer';
