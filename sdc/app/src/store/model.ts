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
  /** The newest versions of each family - what the menu shows without being asked. */
  models: CatalogModel[];
  /** Every other version the provider still lists, behind `Older versions…`. */
  older: CatalogModel[];
}

/** The menu's answer: the connected providers, and how many are not. */
export interface ConnectedCatalog {
  groups: CatalogGroup[];
  /** Providers the catalogue has rows for whose card does not say `connected`. */
  disconnected: number;
}

/**
 * How many versions of one model family the menu shows before `Older versions…`.
 *
 * The owner's words were "each one's latest 2 or 3 versions, not all of them". Two keeps a provider
 * that lists three families (Opus, Sonnet, Haiku) at six rows, which fits the dropdown without a
 * scroll; the rest are one click away rather than gone, because hiding a model is not the same as it
 * not existing.
 */
export const VERSIONS_PER_FAMILY = 2;

/**
 * Ids a provider lists that are not chat models.
 *
 * OpenAI's `/v1/models` answers with embeddings, speech, image and moderation models next to the chat
 * ones, and a turn sent to `text-embedding-3-small` fails in a way nobody can read. None of these can
 * run a coding turn, so they are left out of the menu rather than labelled.
 */
const NOT_A_CHAT_MODEL =
  /(embed|tts|whisper|dall-e|davinci|babbage|moderation|image|audio|realtime|transcribe|search|speech|vision-preview|guard)/i;

/** A model id taken apart: the family it belongs to, and where it sits in that family. */
export interface ModelVersion {
  /** The id with its version numbers and dates removed: `claude-sonnet-4-5-20250929` -> `claude-sonnet`. */
  family: string;
  /** The version numbers in order: `claude-3-5-sonnet` -> `[3, 5]`, `llama3.2:3b` -> `[3, 2]`. */
  version: number[];
  /** A dated snapshot (`-20250929`, `-2024-08-06`, `-001`) rather than the moving alias. */
  dated: boolean;
  /** A preview or experimental build. */
  preview: boolean;
}

/**
 * Where a model id sits in its family - a spelling rule, not a catalogue.
 *
 * Providers name versions in a handful of shapes, and every one of them is covered by a test:
 *
 *   claude-sonnet-4-5 / claude-3-5-sonnet-20240620   numbers anywhere, a date at the end
 *   gpt-5-mini / gpt-4o / o4-mini                   a number glued to a letter
 *   llama3.2:3b / llama-3.3-70b-versatile           a size (`3b`, `70b`) is part of the family
 *   gemini-2.5-pro-preview-05-06                    a preview with a date behind it
 *
 * An id with no numbers at all (`sonnet`, `default`) is its own family with an empty version, so it
 * is always shown: that is what a CLI's aliases look like, and they always mean "the current one".
 */
export function parseModelId(id: string): ModelVersion {
  const base = (id.split('/').pop() ?? id).toLowerCase();
  const tokens = base.split(/[-_:]/).filter((token) => token !== '');
  const family: string[] = [];
  const version: number[] = [];
  let dated = false;
  let preview = false;
  let inDate = false;

  for (const token of tokens) {
    if (/^\d{8}$/.test(token) || /^20\d{2}$/.test(token) || /^0\d{2}$/.test(token)) {
      dated = true;
      inDate = true;
      continue;
    }

    /* The two-digit month and day after a year, or after `preview`, belong to the date. */
    if (inDate && /^\d{2}$/.test(token)) {
      continue;
    }

    inDate = false;

    if (token === 'latest') {
      continue;
    }

    if (token === 'preview' || token === 'exp' || token === 'experimental') {
      preview = true;
      inDate = true;
      continue;
    }

    const match = /^([a-z]*)(\d+(?:\.\d+)*)([a-z]*)$/.exec(token);

    if (match === null) {
      family.push(token);
      continue;
    }

    const [, prefix = '', number = '', suffix = ''] = match;

    /* `70b`, `8b`, `500m`: a parameter count names a different model, not a newer one. */
    if (prefix === '' && /^[bmk]$/.test(suffix)) {
      family.push(token);
      continue;
    }

    if (prefix !== '' && prefix !== 'v') {
      family.push(prefix);
    }

    version.push(...number.split('.').map(Number));

    if (suffix !== '') {
      family.push(suffix);
    }
  }

  return { family: family.join('-'), version, dated, preview };
}

/** Newer first: `[4, 5]` before `[4, 1]` before `[4]`. */
function compareVersions(left: readonly number[], right: readonly number[]): number {
  const length = Math.max(left.length, right.length);

  for (let index = 0; index < length; index += 1) {
    const difference = (right[index] ?? -1) - (left[index] ?? -1);

    if (difference !== 0) {
      return difference;
    }
  }

  return 0;
}

/**
 * One provider's rows, split into the newest versions of each family and the rest.
 *
 * Within a version, the moving alias wins over a dated snapshot and a release over a preview - so
 * `claude-sonnet-4-5` is shown and `claude-sonnet-4-5-20250929` goes behind `Older versions…`, where
 * a person who needs to pin the snapshot can still find it.
 */
export function latestVersions(
  models: readonly CatalogModel[],
  perFamily: number = VERSIONS_PER_FAMILY,
): { current: CatalogModel[]; older: CatalogModel[] } {
  const families = new Map<string, { model: CatalogModel; parsed: ModelVersion }[]>();

  for (const model of models) {
    const parsed = parseModelId(model.id);
    const members = families.get(parsed.family) ?? [];

    members.push({ model, parsed });
    families.set(parsed.family, members);
  }

  const current: CatalogModel[] = [];
  const older: CatalogModel[] = [];

  for (const members of families.values()) {
    members.sort(
      (left, right) =>
        compareVersions(left.parsed.version, right.parsed.version) ||
        Number(left.parsed.preview) - Number(right.parsed.preview) ||
        Number(left.parsed.dated) - Number(right.parsed.dated) ||
        left.model.id.localeCompare(right.model.id),
    );

    const shown: number[][] = [];

    for (const { model, parsed } of members) {
      const sameVersionShown = shown.some((version) => compareVersions(version, parsed.version) === 0);

      if (!sameVersionShown && shown.length < perFamily) {
        shown.push(parsed.version);
        current.push(model);
      } else {
        older.push(model);
      }
    }
  }

  return { current, older };
}

/**
 * The catalogue the menu draws: **connected providers only**, newest versions first.
 *
 * The owner's rule: "above the chat, only the models and agents that are connected - not the rest".
 * It is also P4 - a model row that fails the moment it is picked is the menu lying. So a provider whose
 * card does not say `connected` has no group at all; the menu counts them in one line instead
 * (`2 providers not connected · Manage…`), which keeps the way to connect them one click away without
 * dressing their models up as choices.
 *
 * The rows themselves are the daemon's own `models.list` - live where the provider answered, cached or
 * bundled where it did not - and nothing is invented here. What this function adds is the connected
 * filter, the version split, and the *engine* each group runs on, all facts about the provider.
 */
export function groupCatalog(
  catalog: readonly CatalogModel[],
  providers: readonly { id: string; name: string; status: string }[],
): ConnectedCatalog {
  const connected = new Set(
    providers.filter((provider) => provider.status === 'connected').map((provider) => provider.id),
  );
  const rows = new Map<string, { label: string; models: CatalogModel[] }>();

  for (const row of catalog) {
    if (NOT_A_CHAT_MODEL.test(row.id)) {
      continue;
    }

    const entry = rows.get(row.providerId) ?? {
      label: row.providerLabel === '' ? row.providerId : row.providerLabel,
      models: [],
    };

    entry.models.push(row);
    rows.set(row.providerId, entry);
  }

  const groups: CatalogGroup[] = [];
  let disconnected = 0;

  for (const [providerId, entry] of rows) {
    if (!connected.has(providerId)) {
      disconnected += 1;
      continue;
    }

    const { current, older } = latestVersions(entry.models);
    const byTier = (left: CatalogModel, right: CatalogModel): number =>
      TIER_ORDER[right.tier] - TIER_ORDER[left.tier] || left.name.localeCompare(right.name);

    groups.push({
      providerId,
      providerLabel: entry.label,
      engine: engineForProvider(providerId),
      models: current.sort(byTier),
      older: older.sort(byTier),
    });
  }

  /* Subscriptions first - they are paid for already - then keyed and local providers by name. */
  groups.sort((left, right) => {
    const leftCli = left.engine === 'native_api' ? 1 : 0;
    const rightCli = right.engine === 'native_api' ? 1 : 0;

    return leftCli - rightCli || left.providerLabel.localeCompare(right.providerLabel);
  });

  return { groups, disconnected };
}

/** Whether an engine has at least one connected provider behind it - the Alt+E cycle skips the rest. */
export function engineConnected(
  engine: EngineId,
  providers: readonly { id: string; status: string }[],
): boolean {
  return providers.some(
    (provider) => provider.status === 'connected' && engineForProvider(provider.id) === engine,
  );
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

/**
 * Alt+E (spec section 9.1): Claude Code -> Codex -> Gemini -> Native API -> Claude Code.
 *
 * Given `usable`, the cycle skips an engine nothing is connected behind - the same rule the menu
 * follows. When nothing at all is usable the plain order is kept, so the key still does something a
 * person can see, and the turn that follows says what to connect.
 */
export function nextEngine(engine: EngineId, usable?: (engine: EngineId) => boolean): EngineId {
  const index = ENGINES.findIndex((candidate) => candidate.id === engine);

  for (let step = 1; step <= ENGINES.length; step += 1) {
    const candidate = ENGINES[(index + step) % ENGINES.length]?.id ?? 'claude_code';

    if (usable === undefined || usable(candidate)) {
      return candidate;
    }
  }

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
