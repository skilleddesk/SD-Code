import { create } from 'zustand';

import { strings } from '../strings';
import { toast } from './toast';

import type { TierName } from '../../../protocol/types';

/**
 * Model store - the three choices the prompt area shows and the model dropdown edits: tier,
 * engine, model (spec sections 7.6 and 9.3).
 *
 * The three are not independent, and that is the whole point of this file:
 *
 *   Tier -> Model    changing the tier moves the model with it. Fast is the first model an engine
 *                    offers, Balanced the second, Deep the third (spec section 9.3: "fast->Haiku,
 *                    balanced->Sonnet, deep->Opus"). An engine with fewer than three models clamps
 *                    to its last one, which is why Codex's Fast and Balanced both end up on
 *                    `default`.
 *   Engine -> Model  changing the engine re-picks the model for the tier you are already on.
 *
 * Everything the UI needs to draw the dropdown lives here too - the tier and engine catalogs and
 * the per-engine model list - so `ModelDropdown` and `ModelSelector` are pure rendering and the
 * mapping cannot drift between them.
 */

/** The three tiers, in the order the dropdown lists them (spec section 9.3). */
export type Tier = 'fast' | 'balanced' | 'deep';

/** The four engines, in the order the dropdown lists them (spec section 9.3). */
export type EngineId = 'claude_code' | 'codex' | 'gemini' | 'native_api';

export interface TierDefinition {
  id: Tier;
  label: string;
  description: string;
  /** Lucide icon name; `ModelDropdown.tsx` owns the name-to-component map. */
  icon: 'zap' | 'scale' | 'brain';
}

export interface EngineDefinition {
  id: EngineId;
  name: string;
  description: string;
  /** `native_api` is the only engine that is not a CLI, so it gets the terminal icon. */
  icon: 'sparkles' | 'terminal';
}

export interface ModelDefinition {
  /** The value stored in `model` and shown in the trigger, the turn meta and the status bar. */
  id: string;
  name: string;
  description: string;
}

/** Tier rows: `Fast quick edits` / `Balanced everyday` / `Deep architecture`. */
export const TIERS: readonly TierDefinition[] = [
  {
    id: 'fast',
    label: strings.prompt.model.tiers.fast.label,
    description: strings.prompt.model.tiers.fast.description,
    icon: 'zap',
  },
  {
    id: 'balanced',
    label: strings.prompt.model.tiers.balanced.label,
    description: strings.prompt.model.tiers.balanced.description,
    icon: 'scale',
  },
  {
    id: 'deep',
    label: strings.prompt.model.tiers.deep.label,
    description: strings.prompt.model.tiers.deep.description,
    icon: 'brain',
  },
];

/** Engine rows: `Claude Code Claude Max subscription` and friends. */
export const ENGINES: readonly EngineDefinition[] = [
  {
    id: 'claude_code',
    name: strings.prompt.model.engines.claude_code.name,
    description: strings.prompt.model.engines.claude_code.description,
    icon: 'sparkles',
  },
  {
    id: 'codex',
    name: strings.prompt.model.engines.codex.name,
    description: strings.prompt.model.engines.codex.description,
    icon: 'sparkles',
  },
  {
    id: 'gemini',
    name: strings.prompt.model.engines.gemini.name,
    description: strings.prompt.model.engines.gemini.description,
    icon: 'sparkles',
  },
  {
    id: 'native_api',
    name: strings.prompt.model.engines.native_api.name,
    description: strings.prompt.model.engines.native_api.description,
    icon: 'terminal',
  },
];

/** The MODEL group, per engine - the list the tier indexes into (spec section 9.3). */
export const ENGINE_MODELS: Record<EngineId, readonly ModelDefinition[]> = {
  claude_code: [
    { id: 'haiku', ...strings.prompt.model.models.haiku },
    { id: 'sonnet', ...strings.prompt.model.models.sonnet },
    { id: 'opus', ...strings.prompt.model.models.opus },
  ],
  codex: [
    { id: 'default', ...strings.prompt.model.models.codexDefault },
    { id: 'gpt-5', ...strings.prompt.model.models.gpt5 },
  ],
  gemini: [
    { id: 'flash', ...strings.prompt.model.models.flash },
    { id: 'pro', ...strings.prompt.model.models.pro },
  ],
  native_api: [
    { id: 'claude-sonnet-4-5', ...strings.prompt.model.models.claudeSonnet },
    { id: 'gpt-5', ...strings.prompt.model.models.gpt5 },
    { id: 'deepseek-chat', ...strings.prompt.model.models.deepseekChat },
    { id: 'llama3.2', ...strings.prompt.model.models.llama },
  ],
};

/** Which slot of an engine's model list each tier means. */
const TIER_SLOT: Record<Tier, number> = { fast: 0, balanced: 1, deep: 2 };

/** At most three prompts may wait behind the running turn (spec section 9.7). */
export const MAX_QUEUED_PROMPTS = 3;

/** The engine's model for a tier, clamped to whatever the engine actually offers. */
export function modelForTier(engine: EngineId, tier: Tier): string {
  const models = ENGINE_MODELS[engine];
  const slot = Math.min(TIER_SLOT[tier], models.length - 1);

  return models[slot]?.id ?? models[0]?.id ?? 'default';
}

/** Alt+M (spec section 9.1): Fast -> Balanced -> Deep -> Fast. */
export function nextTier(tier: Tier): Tier {
  const index = TIERS.findIndex((candidate) => candidate.id === tier);

  return TIERS[(index + 1) % TIERS.length]?.id ?? 'balanced';
}

/** Alt+E (spec section 9.1): Claude Code -> Codex -> Gemini -> Native API -> Claude Code. */
export function nextEngine(engine: EngineId): EngineId {
  const index = ENGINES.findIndex((candidate) => candidate.id === engine);

  return ENGINES[(index + 1) % ENGINES.length]?.id ?? 'claude_code';
}

export interface ModelState {
  tier: Tier;
  engine: EngineId;
  model: string;
  /** The dropdown's own open flag, so an outside click can close it from anywhere. */
  dropdownOpen: boolean;
  /** Steering prompts queued behind the running turn (spec section 9.7). */
  queued: string[];
}

export interface ModelActions {
  /** Pick a tier; the model follows (spec section 9.3). */
  setTier: (tier: Tier) => void;
  /** Pick an engine; the model is re-picked for the current tier. */
  setEngine: (engine: EngineId) => void;
  setModel: (model: string) => void;
  openDropdown: () => void;
  closeDropdown: () => void;
  toggleDropdown: () => void;
  /** Alt+M. Announces the new tier the way the prototype does. */
  cycleTier: () => void;
  /** Alt+E. */
  cycleEngine: () => void;
  /** Queue a steering prompt; ignored once three are waiting. */
  enqueue: (prompt: string) => void;
  dequeue: (prompt: string) => void;
  clearQueue: () => void;
}

const initialModelState: ModelState = {
  /* The prototype's defaults: Balanced / claude_code / sonnet (design/ui-prototype.html, `S`). */
  tier: 'balanced',
  engine: 'claude_code',
  model: 'sonnet',
  dropdownOpen: false,
  queued: [...strings.prompt.queued.seed],
};

export const useModelStore = create<ModelState & ModelActions>()((set, get) => ({
  ...initialModelState,

  setTier: (tier) => {
    const state = get();
    const model = modelForTier(state.engine, tier);

    set({ tier, model, dropdownOpen: false });
    toast(strings.prompt.model.tierChanged(tierLabel(tier)));
  },

  setEngine: (engine) => {
    const state = get();

    set({ engine, model: modelForTier(engine, state.tier), dropdownOpen: false });
    toast(strings.prompt.model.engineChanged(engine));
  },

  setModel: (model) => {
    set({ model, dropdownOpen: false });
    toast(strings.prompt.model.modelChanged(model));
  },

  openDropdown: () => {
    if (!get().dropdownOpen) {
      set({ dropdownOpen: true });
    }
  },

  closeDropdown: () => {
    if (get().dropdownOpen) {
      set({ dropdownOpen: false });
    }
  },

  toggleDropdown: () => set((state) => ({ dropdownOpen: !state.dropdownOpen })),

  cycleTier: () => get().setTier(nextTier(get().tier)),

  cycleEngine: () => get().setEngine(nextEngine(get().engine)),

  enqueue: (prompt) => {
    const { queued } = get();

    if (queued.length < MAX_QUEUED_PROMPTS) {
      set({ queued: [...queued, prompt] });
    }
  },

  dequeue: (prompt) => set((state) => ({ queued: state.queued.filter((item) => item !== prompt) })),

  clearQueue: () => {
    if (get().queued.length > 0) {
      set({ queued: [] });
    }
  },
}));

/** The tier's display label - `Balanced`, not `balanced` (spec section 7.6). */
export function tierLabel(tier: Tier): string {
  return TIERS.find((candidate) => candidate.id === tier)?.label ?? 'Balanced';
}

/** The same three tiers under the protocol's spelling, for an `engine.start`'s `tier` field. */
export function tierName(tier: Tier): TierName {
  return tier === 'fast' ? 'Fast' : tier === 'deep' ? 'Deep' : 'Balanced';
}
