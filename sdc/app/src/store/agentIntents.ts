import { create } from 'zustand';

import { sdcpCall } from '../lib/sdcp';
import { strings } from '../strings';
import { useAppStore } from './store';
import { toast } from './toast';

/**
 * 0.13's calls from the window: the composer's `/` commands and `@` files, the agent's questions, the
 * memory files, background processes, the context meter and erase-everything. Each answers a value or
 * `null`/`false` - a failure is a toast here, never an exception in a click handler.
 */

export interface CommandRow {
  name: string;
  description: string;
  source: string;
  body?: string;
}

/** The `/` list for a chat: SDC's commands, then the project's own. Cached per chat for a minute. */
const commandCache = new Map<string, { at: number; rows: CommandRow[] }>();

export async function listCommands(sessionId: string | null): Promise<CommandRow[]> {
  const key = sessionId ?? '';
  const cached = commandCache.get(key);

  if (cached !== undefined && Date.now() - cached.at < 60_000) {
    return cached.rows;
  }

  try {
    const { commands } = await sdcpCall('commands.list', sessionId === null ? {} : { sessionId });

    commandCache.set(key, { at: Date.now(), rows: commands });

    return commands;
  } catch {
    return [];
  }
}

/** The `@` list: files under the chat's folder whose name contains the query. */
export async function findFiles(sessionId: string, query: string): Promise<{ path: string; dir: boolean }[]> {
  try {
    const { files } = await sdcpCall('files.find', { sessionId, query, limit: 30 });

    return files.filter((file) => !file.dir).slice(0, 20);
  } catch {
    return [];
  }
}

export async function answerQuestion(questionId: string, answer: string, turnId: string, sessionId: string): Promise<boolean> {
  try {
    const { answered } = await sdcpCall('question.answer', { questionId, answer, turnId, sessionId });

    if (!answered) {
      toast(strings.agent.question.failed);
    }

    return answered;
  } catch {
    toast(strings.agent.question.failed);

    return false;
  }
}

export async function readMemory(sessionId: string | null, scope: 'project' | 'global'): Promise<{ text: string; path: string } | null> {
  try {
    return await sdcpCall('memory.get', { ...(sessionId === null ? {} : { sessionId }), scope });
  } catch {
    return null;
  }
}

export async function writeMemory(sessionId: string | null, scope: 'project' | 'global', text: string): Promise<boolean> {
  try {
    await sdcpCall('memory.set', { ...(sessionId === null ? {} : { sessionId }), scope, text });

    return true;
  } catch (error) {
    toast(error instanceof Error ? error.message : String(error));

    return false;
  }
}

/** `/remember <fact>`: into the project's memory, or the global one when the chat has no folder. */
export async function rememberFact(sessionId: string | null, fact: string): Promise<boolean> {
  const attempt = async (scope: 'project' | 'global'): Promise<string | null> => {
    try {
      const { path } = await sdcpCall('memory.add', { ...(sessionId === null ? {} : { sessionId }), scope, text: fact });

      return path;
    } catch {
      return null;
    }
  };
  const path = (await attempt('project')) ?? (await attempt('global'));

  toast(path === null ? strings.agent.commands.rememberFailed : strings.agent.commands.remembered(path));

  return path !== null;
}

export interface ProcessRow {
  processId: string;
  sessionId: string;
  command: string;
  place: string;
  seconds: number;
  running: boolean;
}

export async function listProcesses(sessionId: string | null): Promise<ProcessRow[]> {
  try {
    const { processes } = await sdcpCall('process.list', sessionId === null ? {} : { sessionId });

    return processes;
  } catch {
    return [];
  }
}

export async function stopProcess(processId: string): Promise<boolean> {
  try {
    const { stopped } = await sdcpCall('process.stop', { processId });

    return stopped;
  } catch {
    return false;
  }
}

/** The meter before any turn: asks the daemon, and keeps the answer where `ContextUpdated` keeps its own. */
export async function refreshContext(sessionId: string, engine: string, model: string, provider: string | null): Promise<void> {
  try {
    const meter = await sdcpCall('context.get', { sessionId, engine, model, ...(provider === null ? {} : { provider }) });

    useAppStore.setState((state) => ({ ...state, contexts: { ...state.contexts, [sessionId]: meter } }));
  } catch {
    /* An older daemon, or none: no meter is shown rather than a made-up one. */
  }
}

export async function eraseEverything(): Promise<boolean> {
  try {
    await sdcpCall('app.erase', { confirm: 'ERASE' });
    toast(strings.agent.settings.erasing);

    /* The daemon exits and the bridge starts a new, empty one; the window starts again with it. */
    try {
      globalThis.localStorage?.clear();
    } catch {
      /* Private storage: nothing to clear. */
    }

    setTimeout(() => globalThis.location?.reload(), 4_000);

    return true;
  } catch (error) {
    toast(`${strings.agent.settings.eraseFailed}: ${error instanceof Error ? error.message : String(error)}`);

    return false;
  }
}

/** Which of 0.13's dialogs is open. */
interface AgentUiState {
  memoryOpen: boolean;
  openMemory: () => void;
  closeMemory: () => void;
}

export const useAgentUi = create<AgentUiState>()((set) => ({
  memoryOpen: false,
  openMemory: () => set({ memoryOpen: true }),
  closeMemory: () => set({ memoryOpen: false }),
}));
