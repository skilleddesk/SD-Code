import { create } from 'zustand';

/**
 * Terminal store - the command surface of the right panel (spec section 7.7's panel, 0.7.13).
 *
 * It holds what the Terminal tab *shows*: the runs a person started, the output each one printed, the
 * line history for ↑, and the one background process that may be running right now. It is the same
 * shape the Console tab uses for its log entries - a list folded from answers, not from events - because
 * a command typed into the window is a chat's own step: the *durable* record of it is the tool-call pair
 * the daemon writes into the session's log (`shell.run` announces itself), and this store is the screen.
 *
 * `where` is not decoration: a terminal that does not say **which machine** it is about is the single
 * most dangerous surface in a remote-capable app - `rm -rf build` on a laptop and on production look
 * identical until one of them is not your laptop. Every entry carries its folder and, when it was not
 * this machine, the host's name.
 */

/** One command and its outcome, as the tab draws it. */
export interface TerminalEntry {
  id: string;
  /** The line as typed. */
  command: string;
  /** `~/app/landing` or `~/app/landing on prod-1` - the folder, and the host when there is one. */
  where: string;
  /**
   * `running` while a background process is alive, `done`/`failed` once it has answered. A refused line
   * (the deny list) is `failed` with the daemon's sentence in `stderr`, because that is what a terminal
   * shows: the reason, in the place output goes.
   */
  state: 'running' | 'done' | 'failed';
  stdout: string;
  stderr: string;
  /** The exit code, or `null` while it is running. */
  code: number | null;
  ms: number;
  /** The command outlived its timeout and was stopped. */
  timedOut: boolean;
}

/** How many runs the tab keeps. The log is a screen, not an archive. */
const ENTRY_LIMIT = 50;

/** How many typed lines ↑ walks back through. */
const HISTORY_LIMIT = 50;

export interface TerminalState {
  entries: TerminalEntry[];
  /** Recent lines, newest last - what ↑ recalls. */
  history: string[];
  /** The background process, when one is running (`pty.open`). */
  background: { id: string; ptyId: string; command: string; where: string } | null;
  /** A foreground run is in flight; the input is disabled until it answers. */
  busy: boolean;
  nextId: number;
}

export interface TerminalActions {
  /** Adds a run that has just started. Returns the id it was given. */
  start: (command: string, where: string) => string;
  /** Replaces a run's outcome - the answer of `shell.run`, or a refusal. */
  finish: (id: string, patch: Partial<TerminalEntry> & Pick<TerminalEntry, 'state'>) => void;
  /** Replaces a background run's output tail (`pty.output` polls it). */
  update: (id: string, patch: Partial<TerminalEntry>) => void;
  /** Remembers a line for ↑, newest last and without duplicates in a row. */
  remember: (command: string) => void;
  setBusy: (busy: boolean) => void;
  setBackground: (background: TerminalState['background']) => void;
  clear: () => void;
}

const initialTerminalState: TerminalState = {
  entries: [],
  history: [],
  background: null,
  busy: false,
  nextId: 1,
};

export const useTerminalStore = create<TerminalState & TerminalActions>()((set, get) => ({
  ...initialTerminalState,

  start: (command, where) => {
    const id = `run-${get().nextId}`;

    set((state) => ({
      nextId: state.nextId + 1,
      entries: [
        ...state.entries.slice(-(ENTRY_LIMIT - 1)),
        { id, command, where, state: 'running' as const, stdout: '', stderr: '', code: null, ms: 0, timedOut: false },
      ],
    }));

    return id;
  },

  finish: (id, patch) =>
    set((state) => ({
      entries: state.entries.map((entry) => (entry.id === id ? { ...entry, ...patch } : entry)),
    })),

  update: (id, patch) =>
    set((state) => ({
      entries: state.entries.map((entry) => (entry.id === id ? { ...entry, ...patch } : entry)),
    })),

  remember: (command) =>
    set((state) => {
      const line = command.trim();

      if (line === '' || state.history.at(-1) === line) {
        return state;
      }

      return { history: [...state.history.slice(-(HISTORY_LIMIT - 1)), line] };
    }),

  setBusy: (busy) => set({ busy }),

  setBackground: (background) => set({ background }),

  clear: () => set({ entries: [] }),
}));

/** The last line ↑ should offer, before any recalling has happened. */
export function lastLine(state: TerminalState): string {
  return state.history.at(-1) ?? '';
}
