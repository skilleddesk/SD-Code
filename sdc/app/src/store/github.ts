import { create } from 'zustand';

import type { GitHubRepoInfo, GitHubRepoRow, GitHubStatus } from '../../../protocol/types';
import { copyText, openOutside } from '../lib/external';
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
  /**
   * 1.0.8: each project's own repository, by project (`placeOf`: the host and the folder) - two chats in one folder
   * share it, the same folder name on two machines does not.
   */
  repos: Record<string, GitHubRepoInfo | { error: string }>;
  /** The projects being pushed right now. */
  pushing: Record<string, boolean>;
  /** The account's repositories, for the picker; `null` until asked. */
  accountRepos: GitHubRepoRow[] | null;
  /** Why the account's repositories could not be read (1.0.9) - shown in the picker instead of an empty list. */
  accountReposError: string | null;
  loadRepo: (sessionId: string, place: string) => Promise<GitHubRepoInfo | null>;
  link: (sessionId: string, place: string, repo: string, isPrivate: boolean) => Promise<GitHubRepoInfo | { error: string }>;
  loadAccountRepos: () => Promise<void>;
  push: (sessionId: string, place: string, options?: { private?: boolean; message?: string }) => Promise<{ url: string } | { error: string }>;
}

const message = (reason: unknown): string => (isSdcpError(reason) ? reason.message : String(reason));

let polling: ReturnType<typeof setInterval> | null = null;

export const useGitHubStore = create<GitHubState>((set, get) => ({
  status: null,
  code: null,
  error: null,
  repos: {},
  pushing: {},
  accountRepos: null,
  accountReposError: null,

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
        /* 1.0.8: the code is already on the clipboard when GitHub's page opens - paste, Authorize, done. */
        void copyText(started.userCode);
        void openOutside(started.verificationUri);
      } else {
        void get().refresh();
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

            /* Connected: the projects' repositories are read again, now as the account. */
            if (state === 'done') {
              set({ repos: {}, accountRepos: null });
            }
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

    set({ accountRepos: null });
    await get().refresh();
  },

  loadRepo: async (sessionId, place) => {
    try {
      const info = await sdcpCall('github.repo', { sessionId });

      set((state) => ({ repos: { ...state.repos, [place]: info } }));

      return info;
    } catch (reason) {
      set((state) => ({ repos: { ...state.repos, [place]: { error: message(reason) } } }));

      return null;
    }
  },

  link: async (sessionId, place, repo, isPrivate) => {
    try {
      const info = await sdcpCall('github.link', { sessionId, repo, private: isPrivate });

      set((state) => ({ repos: { ...state.repos, [place]: info } }));

      return info;
    } catch (reason) {
      return { error: message(reason) };
    }
  },

  loadAccountRepos: async () => {
    try {
      const answer = await sdcpCall('github.repos', {});

      set({ accountRepos: answer.repos, accountReposError: null });
    } catch (reason) {
      set({ accountRepos: [], accountReposError: message(reason) });
    }
  },

  push: async (sessionId, place, options = {}) => {
    if (get().pushing[place] === true) {
      return { error: 'already pushing' };
    }

    set((state) => ({ pushing: { ...state.pushing, [place]: true } }));

    const answer = await pushToGitHub({ sessionId }, options);

    set((state) => ({ pushing: { ...state.pushing, [place]: false } }));

    return answer;
  },
}));

/** Which project a chat works in, as the repository map keys it: the host and the folder. `null` with no folder. */
export function placeOf(hostId: string, projectRoot: string | null | undefined): string | null {
  return projectRoot === null || projectRoot === undefined || projectRoot === '' ? null : `${hostId}:${projectRoot}`;
}

/** Pushes a chat's folder (local or on its VPS), or a folder on this computer. Answers the repository page. */
export async function pushToGitHub(target: { sessionId?: string; root?: string }, options: { private?: boolean; message?: string } = {}): Promise<{ url: string } | { error: string }> {
  try {
    const answer = await sdcpCall('github.push', { ...target, ...options });

    /* The projects' repositories are read again: a push made a commit, may have made a repository, and the chat-box
       button's "changed files" dot must not outlive it - whichever way the push came (button, chip, "push"). */
    useGitHubStore.setState({ repos: {} });

    return { url: answer.url };
  } catch (reason) {
    return { error: message(reason) };
  }
}

/** The name a new repository gets from what was typed: the same rule the daemon applies (`github::repo_name`). */
export function repoNameOf(text: string): string {
  const name = text
    .replace(/[^A-Za-z0-9._-]/g, '-')
    .replace(/-{2,}/g, '-')
    .replace(/^[-.]+|[-.]+$/g, '');

  return name === '' ? 'sdc-project' : name;
}

/** A chat message that only asks to push: `push`, `/push`, `push koro`, `পুশ করো`, `push to github`… */
export function isPushRequest(text: string): boolean {
  const said = text.trim().toLowerCase().replace(/[.!।?]+$/u, '').trim();

  return /^(\/push|push|push (it|now|koro|koren|kore dao|kore den|to github|github e)|github e push( koro| koren| kore dao)?|পুশ( করো| করুন| করে দাও| করে দিন)?|গিটহাবে পুশ( করো| করুন| করে দাও| করে দিন)?)$/u.test(said);
}
