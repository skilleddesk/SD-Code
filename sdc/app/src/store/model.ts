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

/** The MODEL group, per engine - **the fallback list, used only before the catalogue has arrived**.
 *
 * 0.7.0 replaced the hardcoded list with the daemon's own catalogue (`models.list`, which is live
 * where a provider answers, cached where it did not, and the shipped bundle underneath both). The
 * report that caused it: "the model thing above the chat box is dummy - the models that are
 * *connected and verified* should be there". This map is what the trigger falls back to for the few
 * hundred milliseconds before the first `models.list` answers, and it is deliberately the smallest
 * honest thing: the ids every Claude Code install has.
 */
export const FALLBACK_MODELS: Record<EngineId, readonly ModelDefinition[]> = {
  claude_code: [
    { id: 'haiku', ...strings.prompt.model.models.haiku },
    { id: 'sonnet', ...strings.prompt.model.models.sonnet },
    { id: 'opus', ...strings.prompt.model.models.opus },
  ],
  codex: [{ id: 'default', ...strings.prompt.model.models.codexDefault }],
  gemini: [
    { id: 'flash', ...strings.prompt.model.models.flash },
    { id: 'pro', ...strings.prompt.model.models.pro },
  ],
  native_api: [],
};

/** One row of the daemon's catalogue, as `models.list` answers it. */
export interface CatalogModel {
  id: string;
  /** The provider's own id, and the readable name this build gives it. */
  providerId: string;
  providerLabel: string;
  name: string;
  tier: Tier;
  ctx: number;
  cost: string;
  /** Where the row came from: the provider just now, its last answer, or this build's bundle. */
  source: 'live' | 'cache' | 'bundled';
}

/** One provider's models, as the dropdown draws them. */
export interface CatalogGroup {
  providerId: string;
  providerLabel: string;
  engine: EngineId;
  /** Whether the card behind this provider says `connected` - i.e. whether its models can run. */
  connected: boolean;
  models: CatalogModel[];
}

/**
 * The catalogue, grouped by provider, connected first.
 *
 * This is the answer to "the model thing above the chat box is dummy": the rows come from the daemon's
 * own `models.list`, which for a signed-in CLI is the plan's own models and for a keyed provider is
 * that provider's live list. Nothing is invented here - the only thing this function adds is the
 * ordering and the *engine* each group runs on, both of which are facts about the provider.
 *
 * A provider whose card does **not** say `connected` still has a group, and the dropdown draws its rows
 * with `Connect` beside them: hiding a provider that the user has never signed into would hide the thing
 * they have to do, and showing its models as if they were usable would be the same lie in the other
 * direction.
 */
export function groupCatalog(
  catalog: readonly CatalogModel[],
  providers: readonly { id: string; name: string; status: string }[],
): CatalogGroup[] {
  const connected = new Set(
    providers.filter((provider) => provider.status === 'connected').map((provider) => provider.id),
  );
  const groups = new Map<string, CatalogGroup>();

  for (const row of catalog) {
    const group = groups.get(row.providerId) ?? {
      providerId: row.providerId,
      providerLabel: row.providerLabel === '' ? row.providerId : row.providerLabel,
      engine: engineForProvider(row.providerId),
      connected: connected.has(row.providerId),
      models: [],
    };

    group.models.push(row);
    groups.set(row.providerId, group);
  }

  for (const group of groups.values()) {
    group.models.sort((left, right) => TIER_ORDER[left.tier] - TIER_ORDER[right.tier] || left.name.localeCompare(right.name));
  }

  return [...groups.values()].sort((left, right) => {
    if (left.connected !== right.connected) {
      return left.connected ? -1 : 1;
    }

    return left.providerLabel.localeCompare(right.providerLabel);
  });
}

/** Fast, Balanced, Deep - the order the tier group is written in, reused for the model rows. */
const TIER_ORDER: Record<Tier, number> = { fast: 0, balanced: 1, deep: 2 };

/**
 * The protocol spells the tiers `Fast` / `Balanced` / `Deep`; this store spells them lowercase.
 *
 * One direction only, and only here: the wire keeps its own spelling and the store keeps its own, and
 * this is the single place the two meet. An unknown word becomes `balanced` rather than throwing - a
 * provider that invents a tier should not empty the menu.
 */
export function tierFromName(name: string): Tier {
  const lowered = name.toLowerCase();

  return lowered === 'fast' || lowered === 'deep' ? lowered : 'balanced';
}

/** The engine that runs a provider's models.
 *
 * A subscription provider is reachable only through its own CLI - that is what the plan *is* - and
 * every other provider is an API endpoint, which is what `native_api` speaks. This mapping is the
 * reason a model row can set the engine as well as the model: picking `Opus` from Claude Code and
 * picking `claude-opus-4` from the Anthropic API are two different routes to the same family, and the
 * row knows which one it is.
 */
export function engineForProvider(providerId: string): EngineId {
  switch (providerId) {
    case 'claude':
      return 'claude_code';
    case 'openai':
      return 'codex';
    case 'gemini':
      return 'gemini';
    default:
      return 'native_api';
  }
}

/** The provider a subscription engine signs in through, or `null` for an API engine. */
export function providerForEngine(engine: EngineId): string | null {
  switch (engine) {
    case 'claude_code':
      return 'claude';
    case 'codex':
      return 'openai';
    case 'gemini':
      return 'gemini';
    default:
      return null;
  }
}

/** How a provider is connected: its CLI's own sign-in, or an API key.
 *
 * The three subscription providers are the ones with a CLI recipe (`claude`, `openai`'s Codex and
 * `gemini`); everything else in the catalogue is reached with a key. The Connect modal needs to be told
 * which of the two to draw, and this is that answer - it is not a preference.
 */
export function connectModeFor(providerId: string): 'login' | 'api' {
  return providerForEngine(engineForProvider(providerId)) === null ? 'api' : 'login';
}

/** The engine's model for a tier, from the catalogue when it has one, else the fallback. */
export function modelForTier(engine: EngineId, tier: Tier, catalog: readonly CatalogModel[] = []): string {
  const fromCatalog = catalog.find((model) => engineForProvider(model.providerId) === engine && model.tier === tier);

  if (fromCatalog !== undefined) {
    return fromCatalog.id;
  }

  const models = FALLBACK_MODELS[engine];
  const slot = Math.min(TIER_SLOT[tier], models.length - 1);

  return models[slot]?.id ?? models[0]?.id ?? '';
}

/** Which slot of an engine's fallback model list each tier means. */
const TIER_SLOT: Record<Tier, number> = { fast: 0, balanced: 1, deep: 2 };

/** At most three prompts may wait behind the running turn (spec section 9.7). */
export const MAX_QUEUED_PROMPTS = 3;

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
  /**
   * The provider the current model belongs to, when it came from the catalogue.
   *
   * `null` means "nobody has picked a provider yet", which is the state a fresh window is in: the
   * trigger then names the engine's own default and the first model the catalogue offers for it.
   */
  providerId: string | null;
  /** The daemon's catalogue, as `models.list` last answered it. */
  catalog: CatalogModel[];
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
  /** Fold a `models.list` answer in - called on boot and whenever the dropdown opens. */
  setCatalog: (models: readonly CatalogModel[]) => void;
  /** One row of the dropdown: engine, provider, model and tier together. */
  choose: (choice: { engine: EngineId; providerId: string; model: string; tier: Tier }) => void;
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
  providerId: 'claude',
  catalog: [],
  dropdownOpen: false,
  queued: [...strings.prompt.queued.seed],
};

export const useModelStore = create<ModelState & ModelActions>()((set, get) => ({
  ...initialModelState,

  setTier: (tier) => {
    const state = get();
    const model = modelForTier(state.engine, tier, state.catalog);

    set({ tier, ...(model === '' ? {} : { model }), dropdownOpen: false });
    toast(strings.prompt.model.tierChanged(tierLabel(tier)));
  },

  setEngine: (engine) => {
    const state = get();
    const model = modelForTier(engine, state.tier, state.catalog);
    const providerId = providerForEngine(engine);

    set({
      engine,
      ...(model === '' ? {} : { model }),
      ...(providerId === null ? {} : { providerId }),
      dropdownOpen: false,
    });
    toast(strings.prompt.model.engineChanged(engine));
  },

  setModel: (model) => {
    set({ model, dropdownOpen: false });
    toast(strings.prompt.model.modelChanged(model));
  },

  /*
   * Folding the catalogue in also repairs a choice that no longer exists.
   *
   * A window whose daemon has just been told that Claude Code is signed out still holds `opus` from
   * the last session; once the catalogue arrives and `opus` is not in it, the trigger would name a
   * model that cannot run. The first catalogue model for the current engine is the honest repair, and
   * the `models.list` rows are exactly "what is connected and verified".
   */
  setCatalog: (models) => {
    const state = get();
    const catalog = [...models];
    const known = catalog.some((model) => model.id === state.model);

    set(known || catalog.length === 0 ? { catalog } : { catalog, model: catalog[0]?.id ?? state.model });
  },

  choose: ({ engine, providerId, model, tier }) => {
    set({ engine, providerId, model, tier, dropdownOpen: false });
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
