import { create } from 'zustand';

import type { PickedFile } from '../lib/picker';
import type { TurnImage } from './intents';

/**
 * Each chat's own unsent message (1.0.8): the words, the images and the picked files, kept **per chat**.
 *
 * The report: "what I type in one chat shows in every chat - another project, another VPS, another chat; voice and
 * images too". The box was one textarea that React reused for whichever chat was open, and its images lived in that
 * one component, so a chat switch only changed the label on the same box. Now the box belongs to its chat (the pane is
 * keyed by the chat) and what it holds is kept here under the chat's id - so leaving a chat half-written and coming
 * back finds it as it was, and nothing ever shows in another chat. Voice writes here too: a dictation that finishes
 * after the person moved to another chat lands in the chat it was spoken in.
 *
 * The words survive a restart (localStorage, a UI preference like the open tabs); images and picked files are this
 * run's only - a screenshot does not belong in storage.
 */
export interface Draft {
  text: string;
  images: readonly TurnImage[];
  attached: readonly PickedFile[];
}

interface DraftsState {
  drafts: Record<string, Draft>;
  setText: (sessionId: string, text: string) => void;
  setImages: (sessionId: string, images: readonly TurnImage[]) => void;
  setAttached: (sessionId: string, attached: readonly PickedFile[]) => void;
  clear: (sessionId: string) => void;
}

const STORAGE_KEY = 'sdc.drafts.v1';
/** A draft longer than this is not remembered across a restart (it still is for this run). */
const KEEP_CHARS = 20_000;

export const EMPTY_DRAFT: Draft = { text: '', images: [], attached: [] };

function load(): Record<string, Draft> {
  try {
    const raw = globalThis.localStorage?.getItem(STORAGE_KEY);
    const texts = raw === null || raw === undefined ? {} : (JSON.parse(raw) as Record<string, unknown>);

    return Object.fromEntries(
      Object.entries(texts)
        .filter((entry): entry is [string, string] => typeof entry[1] === 'string' && entry[1] !== '')
        .map(([id, text]) => [id, { ...EMPTY_DRAFT, text }]),
    );
  } catch {
    return {};
  }
}

let saveTimer: ReturnType<typeof setTimeout> | null = null;

function save(drafts: Record<string, Draft>): void {
  if (saveTimer !== null) {
    clearTimeout(saveTimer);
  }

  saveTimer = setTimeout(() => {
    saveTimer = null;

    try {
      const texts = Object.fromEntries(
        Object.entries(drafts)
          .filter(([, draft]) => draft.text.trim() !== '' && draft.text.length <= KEEP_CHARS)
          .map(([id, draft]) => [id, draft.text]),
      );

      globalThis.localStorage?.setItem(STORAGE_KEY, JSON.stringify(texts));
    } catch {
      /* A storage that refuses: the drafts still hold for this run. */
    }
  }, 300);
}

function patch(sessionId: string, change: Partial<Draft>): (state: DraftsState) => Partial<DraftsState> {
  return (state) => {
    const drafts = { ...state.drafts, [sessionId]: { ...(state.drafts[sessionId] ?? EMPTY_DRAFT), ...change } };

    save(drafts);

    return { drafts };
  };
}

export const useDraftsStore = create<DraftsState>((set) => ({
  drafts: load(),
  setText: (sessionId, text) => set(patch(sessionId, { text })),
  setImages: (sessionId, images) => set(patch(sessionId, { images })),
  setAttached: (sessionId, attached) => set(patch(sessionId, { attached })),
  clear: (sessionId) =>
    set((state) => {
      const drafts = { ...state.drafts };

      delete drafts[sessionId];
      save(drafts);

      return { drafts };
    }),
}));

/** One chat's draft, read outside a component. */
export function draftOf(sessionId: string): Draft {
  return useDraftsStore.getState().drafts[sessionId] ?? EMPTY_DRAFT;
}

/** Whether a chat holds an unsent message - words, an image or a picked file. */
export function hasDraft(sessionId: string): boolean {
  const draft = useDraftsStore.getState().drafts[sessionId];

  return draft !== undefined && (draft.text.trim() !== '' || draft.images.length > 0 || draft.attached.length > 0);
}
