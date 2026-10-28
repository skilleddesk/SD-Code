import { create } from 'zustand';

import type { GitHubStatus } from '../../../protocol/types';
import { openOutside } from '../lib/external';
import { sdcpCall } from '../lib/sdcp';
import { isSdcpError } from '../lib/transport';

/**
 * GitHub (1.0.0): the connected account, a browser approval in progress, and pushes.
 *
 * Connecting is GitHub's own approval page - the daemon asks GitHub for a code, the browser opens, the person
 * presses Authorize, and the daemon receives the token (`github.connect`). The window only polls `github.status`
 * until the daemon says the approval is `done` or `failed`.
 */
interface GitHubState {
  status: GitHubStatus | null;
  /** The device code to type on GitHub, while an approval is waiting. */
  code: { userCode: string; verificationUri: string } | null;
  error: string | null;
  refresh: () => Promise<GitHubStatus | null>;
  connect: () => Promise<void>;
  disconnect: () => Promise<void>;
}

const message = (reason: unknown): string => (isSdcpError(reason) ? reason.message : String(reason));

let polling: ReturnType<typeof setInterval> | null = null;

export const useGitHubStore = create<GitHubState>((set, get) => ({
  status: null,
  code: null,
  error: null,

  refresh: async () => {
    try {
      const status = await sdcpCall('github.status', {});

      set({ status });

      return status;
    } catch (reason) {
      set({ error: message(reason) });

      return null;
    }
  },

  connect: async () => {
    set({ error: null, code: null });

    try {
      const started = await sdcpCall('github.connect', {});

      if (started.method === 'device' && started.userCode !== undefined && started.verificationUri !== undefined) {
        set({ code: { userCode: started.userCode, verificationUri: started.verificationUri } });
        void openOutside(started.verificationUri);
      }
    } catch (reason) {
      set({ error: message(reason) });

      return;
    }

    if (polling !== null) {
      clearInterval(polling);
    }

    polling = setInterval(() => {
      void get()
        .refresh()
        .then((status) => {
          const state = status?.pending?.state;

          if (state === 'done' || state === 'failed' || status === null) {
            if (polling !== null) {
              clearInterval(polling);
              polling = null;
            }

            set({ code: null, error: state === 'failed' ? (status?.pending?.error ?? null) : null });
          }
        });
    }, 2000);
  },

  disconnect: async () => {
    try {
      await sdcpCall('github.disconnect', {});
    } catch (reason) {
      set({ error: message(reason) });
    }

    await get().refresh();
  },
}));

/** Pushes a chat's folder (local or on its VPS), or a folder on this computer. Answers the repository page. */
export async function pushToGitHub(target: { sessionId?: string; root?: string }, options: { private?: boolean; message?: string } = {}): Promise<{ url: string } | { error: string }> {
  try {
    const answer = await sdcpCall('github.push', { ...target, ...options });

    return { url: answer.url };
  } catch (reason) {
    return { error: message(reason) };
  }
}

/** A chat message that only asks to push: `push`, `/push`, `push koro`, `পুশ করো`, `push to github`… */
export function isPushRequest(text: string): boolean {
  const said = text.trim().toLowerCase().replace(/[.!।?]+$/u, '').trim();

  return /^(\/push|push|push (it|now|koro|koren|kore dao|kore den|to github|github e)|github e push( koro| koren| kore dao)?|পুশ( করো| করুন| করে দাও| করে দিন)?|গিটহাবে পুশ( করো| করুন| করে দাও| করে দিন)?)$/u.test(said);
}
