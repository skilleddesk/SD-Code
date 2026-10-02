import { useEffect } from 'react';

import { choiceOf, providerForEngine, tierFromName, useModelStore, type EngineId, type ModelChoice } from './model';
import { usePrefsStore } from './prefs';
import { useAppStore } from './store';

/**
 * Each chat's model (0.16.1).
 *
 * The model store keeps a choice per chat (`chats`) and a starting point for a chat that has none. This
 * file is where the two meet the conversation: a chat that never had a pick of its own - one from before
 * this version, or one a fresh window has not shown yet - is shown the model its **last turn ran**, not
 * whatever was picked last in some other chat. Once a chat is on screen it adopts that choice, so a later
 * pick elsewhere cannot move it.
 */

const ENGINES: readonly EngineId[] = ['claude_code', 'codex', 'gemini', 'native_api'];

/** The model a chat's newest turn ran, as a choice - or `null` for a chat with no turns. */
export function lastTurnChoice(sessionId: string): ModelChoice | null {
  const turns = useAppStore.getState().turns;
  let last = null as (typeof turns)[number] | null;

  for (const turn of turns) {
    if (turn.sessionId === sessionId) {
      last = turn;
    }
  }

  if (last === null || last.model === '' || !ENGINES.includes(last.engine as EngineId)) {
    return null;
  }

  const engine = last.engine as EngineId;
  /* A turn names its model, not its provider: a CLI's provider follows from the engine, an API model's
     from the catalogue row with that id. */
  const providerId =
    providerForEngine(engine) ??
    useModelStore.getState().catalog.find((row) => row.id === last.model)?.providerId ??
    null;

  return { engine, model: last.model, providerId, tier: tierFromName(last.tier) };
}

/** The model a chat runs next: its own pick, else its last turn's, else the starting point. */
export function chatChoice(sessionId: string | null | undefined): ModelChoice {
  const state = useModelStore.getState();

  if (sessionId && state.chats[sessionId] === undefined) {
    return lastTurnChoice(sessionId) ?? choiceOf(state);
  }

  return choiceOf(state, sessionId);
}

/**
 * Pins every chat that is open in a tab to what it shows now, when it has no pick of its own. Called
 * before a pick moves the starting point, so the empty chat beside the one being changed stays put.
 */
export function pinOpenChats(except?: string | null): void {
  const { openTabs, splitSecondary, activeTab } = usePrefsStore.getState();
  const ids = new Set([...openTabs, splitSecondary, activeTab]);

  for (const id of ids) {
    if (id && id !== except && useModelStore.getState().chats[id] === undefined) {
      useModelStore.getState().adopt(id, chatChoice(id));
    }
  }
}

/** A dropdown row picked in one chat's box: that chat changes, and no other. */
export function chooseForChat(
  sessionId: string | null | undefined,
  choice: { engine: EngineId; providerId: string; model: string; tier: ModelChoice['tier'] },
): void {
  pinOpenChats(sessionId);
  useModelStore.getState().choose(choice, sessionId);
}

/** The tier picked in one chat's box (the dropdown's tier rows, Alt+M). */
export function setTierForChat(sessionId: string | null | undefined, tier: ModelChoice['tier']): void {
  pinOpenChats(sessionId);
  seed(sessionId);
  useModelStore.getState().setTier(tier, sessionId);
}

/** The engine picked for one chat (Alt+E). */
export function setEngineForChat(sessionId: string | null | undefined, engine: EngineId): void {
  pinOpenChats(sessionId);
  seed(sessionId);
  useModelStore.getState().setEngine(engine, sessionId);
}

/** A chat about to change its tier or engine starts from what it shows, not from the starting point. */
function seed(sessionId: string | null | undefined): void {
  if (sessionId && useModelStore.getState().chats[sessionId] === undefined) {
    useModelStore.getState().adopt(sessionId, chatChoice(sessionId));
  }
}

/**
 * The hook a chat's box reads its model through. It re-renders when that chat's pick changes, and a chat
 * whose history is on screen is pinned to the model its last turn ran.
 */
export function useChatModel(sessionId: string | null | undefined): ModelChoice {
  const own = useModelStore((state) => (sessionId ? state.chats[sessionId] : undefined));
  const tier = useModelStore((state) => state.tier);
  const engine = useModelStore((state) => state.engine);
  const model = useModelStore((state) => state.model);
  const providerId = useModelStore((state) => state.providerId);
  /* Re-read when the chat's turns arrive, so a reopened chat shows what it last ran. */
  const turnCount = useAppStore((state) => state.turns.length);

  useEffect(() => {
    /* Only once the history is in: pinning before the replay arrives would pin the starting point. */
    if (sessionId && useModelStore.getState().chats[sessionId] === undefined) {
      const ran = lastTurnChoice(sessionId);

      if (ran !== null) {
        useModelStore.getState().adopt(sessionId, ran);
      }
    }
  }, [sessionId, turnCount]);

  if (own !== undefined) {
    return own;
  }

  return (sessionId ? lastTurnChoice(sessionId) : null) ?? { tier, engine, model, providerId };
}
