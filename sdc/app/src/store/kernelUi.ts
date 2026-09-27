import { create } from 'zustand';

import type { TurnSeed } from './intents';

/**
 * Which kernel surface is open, and the one request waiting for its Intent Contract (0.12) - presentation,
 * not facts, so not in the log (the rule of `store/overlays.ts`). The facts they show - the reading, the
 * deploys, the health - are folded from events into `AppState.kernel`.
 */

export type AgencyTab = 'sites' | 'deploys' | 'approvals' | 'playbooks' | 'xray' | 'guardian';

export interface PendingIntent {
  intentId: string;
  /** What the person typed, exactly. */
  text: string;
  /** The turn it becomes once confirmed. */
  seed: TurnSeed;
}

interface KernelUiState {
  agencyOpen: boolean;
  agencyTab: AgencyTab;
  costOpen: boolean;
  policyOpen: boolean;
  onboardingOpen: boolean;
  /** The request each chat is waiting to confirm, by session id. */
  pending: Record<string, PendingIntent>;
  openAgency: (tab?: AgencyTab) => void;
  closeAgency: () => void;
  openCost: () => void;
  closeCost: () => void;
  openPolicy: () => void;
  closePolicy: () => void;
  openOnboarding: () => void;
  closeOnboarding: () => void;
  setPending: (sessionId: string, pending: PendingIntent) => void;
  clearPending: (sessionId: string) => void;
}

export const useKernelUi = create<KernelUiState>()((set) => ({
  agencyOpen: false,
  agencyTab: 'sites',
  costOpen: false,
  policyOpen: false,
  onboardingOpen: false,
  pending: {},
  openAgency: (tab = 'sites') => set({ agencyOpen: true, agencyTab: tab }),
  closeAgency: () => set({ agencyOpen: false }),
  openCost: () => set({ costOpen: true }),
  closeCost: () => set({ costOpen: false }),
  openPolicy: () => set({ policyOpen: true }),
  closePolicy: () => set({ policyOpen: false }),
  openOnboarding: () => set({ onboardingOpen: true }),
  closeOnboarding: () => set({ onboardingOpen: false }),
  setPending: (sessionId, pending) => set((state) => ({ pending: { ...state.pending, [sessionId]: pending } })),
  clearPending: (sessionId) =>
    set((state) => {
      const next = { ...state.pending };

      delete next[sessionId];

      return { pending: next };
    }),
}));

/** Whether the first-launch wizard has been seen (0.12). */
export function onboardedAlready(): boolean {
  try {
    return globalThis.localStorage?.getItem('sdc.onboarded.v1') === 'yes';
  } catch {
    return true;
  }
}
