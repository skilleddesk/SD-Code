import { create } from 'zustand';

import type { ResearchPlan, ResearchStatus } from '../../../protocol/types';
import { sdcpCall } from '../lib/sdcp';
import { isSdcpError } from '../lib/transport';
import { strings } from '../strings';
import { chatChoice } from './chatModel';
import { interruptTurn, sendPrompt } from './intents';
import { useAppStore } from './store';
import { toast } from './toast';

/**
 * `/research` in the window (0.16.1).
 *
 * The question is not sent at once: the daemon is asked what the research will do (`research.plan` -
 * which model and where it runs, the search service, the limits, about what it will cost) and that is
 * shown on a card above the box. `Start research` sends it; `Cancel` puts the words back. A paid model
 * never starts a web crawl the person did not see the price of.
 */
export interface PendingResearch {
  question: string;
  plan: ResearchPlan | null;
}

interface ResearchUi {
  /** The card waiting for a decision, by chat (`''` for a box with no chat yet). */
  pending: Record<string, PendingResearch>;
  setPending: (key: string, pending: PendingResearch | null) => void;
}

export const useResearchUi = create<ResearchUi>()((set) => ({
  pending: {},
  setPending: (key, pending) =>
    set((state) => {
      const next = { ...state.pending };

      if (pending === null) {
        delete next[key];
      } else {
        next[key] = pending;
      }

      return { pending: next };
    }),
}));

function failed(error: unknown, fallback: string): void {
  toast(isSdcpError(error) ? error.message : fallback);
}

/** Asks what a research would do for this chat's model, and puts the card up. */
export async function prepareResearch(sessionId: string | null, question: string): Promise<void> {
  const { engine, model, providerId } = chatChoice(sessionId);
  const key = sessionId ?? '';

  try {
    const plan = await sdcpCall('research.plan', {
      engine,
      model,
      prompt: question,
      ...(providerId === null ? {} : { provider: providerId }),
    });

    useResearchUi.getState().setPending(key, { question, plan });
  } catch (error) {
    /* A daemon that cannot say what it will do still lets the person go ahead - without the details. */
    failed(error, strings.research.planFailed);
    useResearchUi.getState().setPending(key, { question, plan: null });
  }
}

/** The card's `Start research`. */
export async function startResearch(sessionId: string | null): Promise<string | null> {
  const key = sessionId ?? '';
  const pending = useResearchUi.getState().pending[key];

  if (pending === undefined) {
    return null;
  }

  useResearchUi.getState().setPending(key, null);

  return sendPrompt(pending.question, sessionId ?? undefined, { research: true });
}

/** The card's `Cancel`: the question goes back to the box. */
export function cancelResearch(sessionId: string | null): string | null {
  const key = sessionId ?? '';
  const pending = useResearchUi.getState().pending[key];

  useResearchUi.getState().setPending(key, null);

  return pending === undefined ? null : `/research ${pending.question}`;
}

/** `/research stop`: the chat's running turn stops, like Stop. */
export async function stopResearch(sessionId: string | null): Promise<void> {
  const running = useAppStore
    .getState()
    .turns.find((turn) => turn.sessionId === sessionId && (turn.status === 'running' || turn.status === 'stuck'));

  if (running === undefined) {
    toast(strings.research.nothingToStop);

    return;
  }

  await interruptTurn(running.id);
}

/** Settings → Research, as the daemon holds it. */
export async function researchStatus(): Promise<ResearchStatus | null> {
  try {
    return await sdcpCall('research.status', {});
  } catch (error) {
    failed(error, strings.research.settings.failed);

    return null;
  }
}

/** A search service's key into the OS keychain (empty removes it). Answers the new status. */
export async function setResearchKey(provider: 'tavily' | 'brave' | 'serper', key: string): Promise<ResearchStatus | null> {
  try {
    const status = await sdcpCall('research.key.set', { provider, key });

    toast(key.trim() === '' ? strings.research.settings.keyRemovedToast : strings.research.settings.keySavedToast);

    return status;
  } catch (error) {
    failed(error, strings.research.settings.failed);

    return null;
  }
}

/** One research or local-model setting, written to the daemon. */
export async function setResearchSetting(key: string, value: string): Promise<boolean> {
  try {
    await sdcpCall('settings.set', { key, value });

    return true;
  } catch (error) {
    failed(error, strings.research.settings.failed);

    return false;
  }
}
