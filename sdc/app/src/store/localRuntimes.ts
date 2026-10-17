import { create } from 'zustand';

import type { LocalScan } from '../../../protocol/types';
import { sdcpCall } from '../lib/sdcp';
import { refreshCatalog } from './intents';
import { refreshProviders } from './tools';

/**
 * Local runtime detection (Phase 2): what model runtimes this computer has (Ollama, LM Studio, llama.cpp, ...), found by the
 * daemon and kept here. A scan never errors at the person: a daemon that cannot answer leaves the last result as it was, and
 * "nothing found" is an ordinary state. It never changes the chosen model.
 */
interface LocalRuntimeState {
  scan: LocalScan | null;
  scanning: boolean;
  lastAt: number;
}

export const useLocalRuntimes = create<LocalRuntimeState>()(() => ({ scan: null, scanning: false, lastAt: 0 }));

/** Asks the daemon to look again. `minGapMs` keeps focus changes from scanning in a loop. */
export async function rescanLocalRuntimes(minGapMs = 0): Promise<void> {
  const state = useLocalRuntimes.getState();

  if (state.scanning || Date.now() - state.lastAt < minGapMs) {
    return;
  }

  useLocalRuntimes.setState({ scanning: true });

  try {
    const scan = await sdcpCall('local.scan', {});

    useLocalRuntimes.setState({ scan, scanning: false, lastAt: Date.now() });

    if (scan.changed) {
      refreshProviders();
      void refreshCatalog();
    }
  } catch {
    useLocalRuntimes.setState({ scanning: false, lastAt: Date.now() });
  }
}

/** Scan now and whenever the window regains focus (at most every 20 s), plus every `everyMs` while `hubOpen()` says the hub is open. */
export function startLocalRescan(hubOpen: () => boolean, everyMs = 180_000): () => void {
  void rescanLocalRuntimes();

  const onFocus = (): void => void rescanLocalRuntimes(20_000);
  const timer = globalThis.setInterval(() => {
    if (hubOpen()) {
      void rescanLocalRuntimes(30_000);
    }
  }, everyMs);

  globalThis.addEventListener?.('focus', onFocus);

  return () => {
    globalThis.clearInterval(timer);
    globalThis.removeEventListener?.('focus', onFocus);
  };
}
