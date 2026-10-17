import { create } from 'zustand';

import type { SecretFinding } from '../../../protocol/types';

/**
 * The first-open secret sweep's result (Phase 0.4): which files of a just-opened folder already hold a secret.
 * Presentation state only - the findings carry a file, a line and a kind, never a value.
 */
interface SecretSweepState {
  root: string | null;
  findings: SecretFinding[];
  truncated: boolean;
  /** `file:line` of each finding the person moved to the keychain, with the keychain name it went under. */
  moved: Record<string, string>;
  failed: Record<string, string>;
}

interface SecretSweepActions {
  show: (root: string, findings: SecretFinding[], truncated: boolean) => void;
  markMoved: (id: string, name: string) => void;
  markFailed: (id: string, message: string) => void;
  close: () => void;
}

export const findingId = (finding: Pick<SecretFinding, 'file' | 'line'>): string => `${finding.file}:${finding.line}`;

export const useSecretSweep = create<SecretSweepState & SecretSweepActions>()((set) => ({
  root: null,
  findings: [],
  truncated: false,
  moved: {},
  failed: {},
  show: (root, findings, truncated) => set({ root, findings, truncated, moved: {}, failed: {} }),
  markMoved: (id, name) => set((state) => ({ moved: { ...state.moved, [id]: name }, failed: { ...state.failed, [id]: '' } })),
  markFailed: (id, message) => set((state) => ({ failed: { ...state.failed, [id]: message } })),
  close: () => set({ root: null, findings: [], truncated: false, moved: {}, failed: {} }),
}));
