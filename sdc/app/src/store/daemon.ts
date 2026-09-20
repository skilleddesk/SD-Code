import { create } from 'zustand';

/**
 * Is the daemon answering?
 *
 * This is a UI-local fact, not an event, and the distinction is the point. The **host list** is the
 * daemon's (it is folded from `HostStatus`/`HostRemoved`), so a host that says `connected` says what
 * the daemon last knew. "The daemon is not answering at all" is a different sentence: nothing in the
 * event log can carry it, because the log's writer is the thing that stopped.
 *
 * Before this file, losing the daemon was silent. `sdcd` is started by the app and killed with it,
 * but it can also die on its own (a crash, a task manager, a laptop's memory pressure), and the
 * window would sit there looking normal while every click answered nothing.
 *
 * `misses` is deliberate rather than a bare boolean: one dropped heartbeat is a hiccup on a busy
 * machine, and a tool that cries wolf on the first one is a tool whose warnings get ignored. The
 * banner appears on the first miss; `STALE_MISSES` is where the copy stops hedging.
 */
export interface DaemonStore {
  /** True until a heartbeat is missed. The window always starts believing the daemon is there. */
  online: boolean;
  /** Consecutive missed heartbeats. Reset to zero by the first answer. */
  misses: number;
  /** The daemon's own sentence for why it is not answering, or the transport's. */
  note: string | null;
  /** One heartbeat's outcome. */
  beat: (online: boolean, note?: string) => void;
}

/** After this many consecutive misses the copy stops saying "retrying" and calls it down. */
export const STALE_MISSES = 3;

export const useDaemonStore = create<DaemonStore>()((set) => ({
  online: true,
  misses: 0,
  note: null,

  beat: (online, note) =>
    set((state) =>
      online
        ? state.online && state.misses === 0
          ? state
          : { online: true, misses: 0, note: null }
        : { online: false, misses: state.misses + 1, note: note ?? state.note },
    ),
}));
