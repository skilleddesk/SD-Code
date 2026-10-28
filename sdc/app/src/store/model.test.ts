import { describe, expect, it } from 'vitest';

import {
  familiesOf,
  filterGroups,
  MAX_QUEUED_PROMPTS,
  choiceOf,
  useModelStore,
  engineConnected,
  groupCatalog,
  latestVersions,
  nextEngine,
  parseModelId,
  type CatalogModel,
  type ModelState,
} from './model';

function row(id: string, providerId = 'anthropic-api', tier: CatalogModel['tier'] = 'balanced'): CatalogModel {
  return { id, providerId, providerLabel: 'Anthropic API', name: id, tier, ctx: 0, cost: '', source: 'live' };
}

const connected = (...ids: string[]) => ids.map((id) => ({ id, name: id, status: 'connected' }));

describe('parseModelId', () => {
  it.each([
    ['claude-sonnet-4-5', 'claude-sonnet', [4, 5], false],
    ['claude-sonnet-4-5-20250929', 'claude-sonnet', [4, 5], true],
    ['claude-3-5-sonnet-20240620', 'claude-sonnet', [3, 5], true],
    ['claude-opus-5-5', 'claude-opus', [5, 5], false],
    ['claude-haiku-4-5-20251001', 'claude-haiku', [4, 5], true],
    ['gpt-5-mini', 'gpt-mini', [5], false],
    ['gpt-4o', 'gpt-o', [4], false],
    ['gpt-4o-2024-08-06', 'gpt-o', [4], true],
    ['o4-mini', 'o-mini', [4], false],
    ['llama3.2:3b', 'llama-3b', [3, 2], false],
    ['llama-3.3-70b-versatile', 'llama-70b-versatile', [3, 3], false],
    ['gemini-2.5-pro', 'gemini-pro', [2, 5], false],
    ['gemini-2.0-flash-001', 'gemini-flash', [2, 0], true],
    ['deepseek-r1', 'deepseek-r', [1], false],
    ['openrouter/anthropic/claude-sonnet-4-5', 'claude-sonnet', [4, 5], false],
    ['sonnet', 'sonnet', [], false],
  ])('%s -> family %s, version %j', (id, family, version, dated) => {
    const parsed = parseModelId(id);

    expect(parsed.family).toBe(family);
    expect(parsed.version).toEqual(version);
    expect(parsed.dated).toBe(dated);
  });

  it('marks a preview, and does not read its date as a version', () => {
    const parsed = parseModelId('gemini-2.5-pro-preview-05-06');

    expect(parsed).toEqual({ family: 'gemini-pro', version: [2, 5], dated: false, preview: true });
  });
});

describe('latestVersions', () => {
  it('keeps the newest two versions of each family and moves the rest behind "older"', () => {
    const { current, older } = latestVersions([
      row('claude-opus-4'),
      row('claude-opus-4-1'),
      row('claude-opus-5-5'),
      row('claude-sonnet-4-5'),
      row('claude-sonnet-5'),
      row('claude-3-5-sonnet-20240620'),
    ]);

    expect(current.map((model) => model.id).sort()).toEqual(
      ['claude-opus-4-1', 'claude-opus-5-5', 'claude-sonnet-4-5', 'claude-sonnet-5'].sort(),
    );
    expect(older.map((model) => model.id).sort()).toEqual(['claude-3-5-sonnet-20240620', 'claude-opus-4'].sort());
  });

  it('shows the moving alias and hides the dated snapshot of the same version', () => {
    const { current, older } = latestVersions([row('claude-sonnet-4-5-20250929'), row('claude-sonnet-4-5')]);

    expect(current.map((model) => model.id)).toEqual(['claude-sonnet-4-5']);
    expect(older.map((model) => model.id)).toEqual(['claude-sonnet-4-5-20250929']);
  });

  it('prefers a release over a preview of the same version', () => {
    const { current } = latestVersions([row('gemini-2.5-pro-preview-05-06'), row('gemini-2.5-pro')], 1);

    expect(current.map((model) => model.id)).toEqual(['gemini-2.5-pro']);
  });

  it('always shows version-less aliases (a CLI plan’s own names)', () => {
    const { current, older } = latestVersions([row('sonnet'), row('opus'), row('haiku')]);

    expect(current).toHaveLength(3);
    expect(older).toHaveLength(0);
  });
});

describe('groupCatalog', () => {
  const catalog = [
    row('claude-sonnet-5', 'anthropic-api'),
    row('gpt-5', 'openai-api'),
    row('text-embedding-3-small', 'openai-api'),
    row('sonnet', 'claude'),
    row('llama3.2:3b', 'ollama'),
  ];

  it('draws a group only for a connected provider, and counts the rest', () => {
    const { groups, disconnected } = groupCatalog(catalog, [
      ...connected('claude', 'anthropic-api'),
      { id: 'openai-api', name: 'OpenAI API', status: 'available' },
      { id: 'ollama', name: 'Ollama', status: 'needs-auth' },
    ]);

    /* 1.0.0, the owner's rule: only the providers that were selected. */
    expect(groups.map((group) => group.providerId).sort()).toEqual(['anthropic-api', 'claude']);
    expect(disconnected).toBe(2);
  });

  it('puts subscription CLIs before API providers', () => {
    const { groups } = groupCatalog(catalog, connected('anthropic-api', 'claude', 'openai-api'));

    expect(groups[0]?.engine).toBe('claude_code');
    expect(groups.slice(1).every((group) => group.engine === 'native_api')).toBe(true);
  });

  it('leaves out models that cannot run a chat turn', () => {
    const { groups } = groupCatalog(catalog, connected('openai-api'));

    expect(groups[0]?.models.map((model) => model.id)).toEqual(['gpt-5']);
  });

  /* 0.12.4: "multiple use ar option rakba" - the models put in use are the provider's menu. */
  it('shows only the models in use for a provider', () => {
    const alibaba = [
      row('qwen3.8-max', 'qwen'),
      row('qwen3.8-flash', 'qwen'),
      row('deepseek-v4.1-flash', 'qwen'),
      row('qwen3-tts-flash', 'qwen'),
    ];
    const { groups } = groupCatalog(alibaba, connected('qwen'), [
      { modelId: 'qwen3.8-max', providerId: 'qwen' },
      { modelId: 'deepseek-v4.1-flash', providerId: 'qwen' },
    ]);

    expect(groups[0]?.models.map((model) => model.id).sort()).toEqual(['deepseek-v4.1-flash', 'qwen3.8-max']);
    expect(groups[0]?.older).toEqual([]);
  });

  it('shows only the newest version of each family, of the current generation', () => {
    const claude = ['claude-opus-5-5', 'claude-opus-4-8', 'claude-sonnet-5-5', 'claude-sonnet-4-6', 'claude-haiku-3-5', 'claude-haiku-5-5', 'claude-fable-5-1', 'claude-fable-5'].map((id) => row(id, 'claude'));
    const gemini = ['gemini-4-argon', 'gemini-3.8-flash', 'gemini-3.6-flash', 'gemini-2.5-pro', 'gemini-2.0-flash'].map((id) => row(id, 'gemini'));
    const { groups } = groupCatalog([...claude, ...gemini], connected('claude', 'gemini'));
    const ids = (provider: string): string[] => groups.find((group) => group.providerId === provider)?.models.map((model) => model.id).sort() ?? [];

    expect(ids('claude')).toEqual(['claude-fable-5-1', 'claude-haiku-5-5', 'claude-opus-5-5', 'claude-sonnet-5-5']);
    expect(ids('gemini')).toEqual(['gemini-3.8-flash', 'gemini-4-argon']);
  });

  it('keeps the two versions before the newest of each shown family, and nothing of a dropped family', () => {
    const claude = ['claude-opus-5-5', 'claude-opus-5', 'claude-opus-4-8', 'claude-opus-4-6', 'claude-opus-5-20260101', 'claude-haiku-3-5', 'claude-sonnet-5-5'].map((id) => row(id, 'claude'));
    const { groups } = groupCatalog(claude, connected('claude'));

    expect(groups[0]?.models.map((model) => model.id).sort()).toEqual(['claude-opus-5-5', 'claude-sonnet-5-5']);
    expect(groups[0]?.older.map((model) => model.id)).toEqual(['claude-opus-5', 'claude-opus-4-8']);
  });

  it('folds a model served in two regions into its -us build, and drops batch twins and aliases', () => {
    const rows = ['qwen3.7-max', 'qwen3.7-max-us', 'qwen3.8-flash', 'x/model-a:batch', '~x/model-latest'].map((id) => row(id, 'qwen'));
    const { groups } = groupCatalog(rows, connected('qwen'));

    expect(groups[0]?.models.map((model) => model.id).sort()).toEqual(['qwen3.7-max-us', 'qwen3.8-flash']);
  });

  it('keeps every model of a local server', () => {
    const { groups } = groupCatalog([row('qwen3.5:9b', 'ollama'), row('qwen3.5-high-ctx:latest', 'ollama')], connected('ollama'));

    expect(groups[0]?.models).toHaveLength(2);
  });

  it('with nothing connected, the menu is empty and counts every provider', () => {
    const { groups, disconnected } = groupCatalog(catalog, []);

    expect(groups).toHaveLength(0);
    expect(disconnected).toBe(4);
  });
});

describe('nextEngine', () => {
  it('skips an engine with nothing connected behind it', () => {
    const providers = connected('claude', 'anthropic-api');
    const usable = (engine: Parameters<typeof engineConnected>[0]) => engineConnected(engine, providers);

    expect(nextEngine('claude_code', usable)).toBe('native_api');
    expect(nextEngine('native_api', usable)).toBe('claude_code');
  });

  it('keeps the plain order when nothing is connected', () => {
    expect(nextEngine('claude_code', () => false)).toBe('codex');
    expect(nextEngine('claude_code')).toBe('codex');
  });
});

describe('filterGroups', () => {
  const catalog = [
    row('claude-sonnet-5', 'anthropic-api'),
    row('claude-sonnet-4-6', 'anthropic-api'),
    row('claude-sonnet-4-5', 'anthropic-api'),
    { ...row('gpt-5', 'openai-api'), providerLabel: 'OpenAI API' },
  ];
  const { groups } = groupCatalog(catalog, connected('anthropic-api', 'openai-api'));

  it('is the identity for an empty query', () => {
    expect(filterGroups(groups, '  ')).toEqual(groups);
  });

  it('narrows to matching rows, the family’s recent versions included', () => {
    const found = filterGroups(groups, 'sonnet');

    expect(found).toHaveLength(1);
    expect(found[0].models.map((model) => model.id)).toEqual(['claude-sonnet-5', 'claude-sonnet-4-6', 'claude-sonnet-4-5']);
    expect(filterGroups(groups, 'sonnet-4-5')[0]?.models.map((model) => model.id)).toEqual(['claude-sonnet-4-5']);
  });

  it('matches a whole provider by its label, and drops groups with nothing', () => {
    const anthropic = filterGroups(groups, 'anthropic');

    expect(anthropic).toHaveLength(1);
    expect(filterGroups(groups, 'no-such-model')).toHaveLength(0);
  });
});

describe('the prompt queue', () => {
  it('keeps each chat’s prompts apart, oldest first, and at most three per chat', () => {
    useModelStore.setState({ queued: [] });

    const { enqueue, takeNext } = useModelStore.getState();

    expect(enqueue('s1', 'first')).toBe(true);
    expect(enqueue('s2', 'other chat')).toBe(true);
    expect(enqueue('s1', 'second')).toBe(true);
    expect(enqueue('s1', 'third')).toBe(true);
    expect(enqueue('s1', 'fourth')).toBe(false);
    expect(MAX_QUEUED_PROMPTS).toBe(3);

    expect(takeNext('s1')).toBe('first');
    expect(takeNext('s1')).toBe('second');
    expect(takeNext('s2')).toBe('other chat');
    expect(takeNext('s2')).toBeNull();
  });

  it('starts empty - a fresh window has nothing waiting that nobody typed', () => {
    useModelStore.setState({ queued: [] });

    expect(useModelStore.getState().queued).toEqual([]);
  });
});

/*
 * 0.16.1: "akta chat a je model select kora onno chat onno model select korle aita automatic sob chat
 * thake model change hoi". A pick belongs to the chat it was made in.
 */
describe('each chat keeps its own model', () => {
  it('a pick in one chat leaves the other chat on its own model', () => {
    useModelStore.setState({ chats: {} });
    const store = useModelStore.getState();

    store.choose({ engine: 'native_api', providerId: 'deepseek', model: 'deepseek-chat', tier: 'balanced' }, 'n-local');
    store.choose({ engine: 'claude_code', providerId: 'claude', model: 'opus', tier: 'deep' }, 'n-vps');

    expect(choiceOf(useModelStore.getState(), 'n-local')).toMatchObject({ model: 'deepseek-chat', providerId: 'deepseek' });
    expect(choiceOf(useModelStore.getState(), 'n-vps')).toMatchObject({ model: 'opus', engine: 'claude_code' });
  });

  it('a chat that adopted its model is not moved by a later pick elsewhere', () => {
    useModelStore.setState({ chats: {} });
    const store = useModelStore.getState();

    store.adopt('n-old', { engine: 'codex', providerId: 'openai', model: 'default', tier: 'fast' });
    store.choose({ engine: 'gemini', providerId: 'gemini', model: 'gemini-2.5-pro', tier: 'deep' }, 'n-new');
    /* Adopting again never overwrites a chat's own pick. */
    store.adopt('n-new', { engine: 'codex', providerId: 'openai', model: 'default', tier: 'fast' });

    expect(choiceOf(useModelStore.getState(), 'n-old').model).toBe('default');
    expect(choiceOf(useModelStore.getState(), 'n-new').model).toBe('gemini-2.5-pro');
  });

  it('a new chat starts from the latest pick, and the tier changes only its own chat', () => {
    useModelStore.setState({ chats: {} });
    const store = useModelStore.getState();

    store.choose({ engine: 'claude_code', providerId: 'claude', model: 'sonnet', tier: 'balanced' }, 'n-a');
    store.adopt('n-b', choiceOf(useModelStore.getState(), 'n-b'));
    store.setTier('deep', 'n-a');

    expect(choiceOf(useModelStore.getState(), 'n-b').tier).toBe('balanced');
    expect(choiceOf(useModelStore.getState(), 'n-a').tier).toBe('deep');
    expect(choiceOf(useModelStore.getState(), 'n-brand-new').tier).toBe('deep');
  });

  it('only the box that was clicked opens its dropdown', () => {
    useModelStore.setState({ dropdownOpen: null });
    useModelStore.getState().toggleDropdown('n-a');

    expect(useModelStore.getState().dropdownOpen).toBe('n-a');
    useModelStore.getState().toggleDropdown('n-a');
    expect(useModelStore.getState().dropdownOpen).toBeNull();
  });
});

describe('a fresh install runs on what the computer has', () => {
  const fresh: Partial<ModelState> = { picked: false, engine: 'claude_code', providerId: 'claude', model: 'sonnet', tier: 'balanced', inUse: [] };
  const local = row('qwen3.5:9b', 'ollama', 'balanced');

  it('moves the untouched default to the local model when only Ollama is connected', () => {
    useModelStore.setState({ ...fresh, catalog: [row('sonnet', 'claude'), local] });
    useModelStore.getState().preferConnected(connected('ollama'));

    expect(useModelStore.getState()).toMatchObject({ engine: 'native_api', providerId: 'ollama', model: 'qwen3.5:9b' });
  });

  it('prefers the local model over another connected provider', () => {
    useModelStore.setState({ ...fresh, catalog: [row('deepseek-chat', 'deepseek'), local] });
    useModelStore.getState().preferConnected(connected('deepseek', 'ollama'));

    expect(useModelStore.getState().providerId).toBe('ollama');
  });

  it('never moves a pick, nor a default that is connected', () => {
    useModelStore.setState({ ...fresh, picked: true, catalog: [local] });
    useModelStore.getState().preferConnected(connected('ollama'));
    expect(useModelStore.getState().model).toBe('sonnet');

    useModelStore.setState({ ...fresh, catalog: [row('sonnet', 'claude'), local] });
    useModelStore.getState().preferConnected(connected('claude', 'ollama'));
    expect(useModelStore.getState().model).toBe('sonnet');
  });

  it('keeps the default when nothing is connected', () => {
    useModelStore.setState({ ...fresh, catalog: [local] });
    useModelStore.getState().preferConnected([]);

    expect(useModelStore.getState().model).toBe('sonnet');
  });

  it('repairs a vanished model inside its own provider, never with another provider\'s', () => {
    useModelStore.setState({ ...fresh, model: 'gone', catalog: [] });
    useModelStore.getState().setCatalog([local, row('opus', 'claude')]);
    expect(useModelStore.getState().model).toBe('opus');

    useModelStore.setState({ ...fresh, model: 'gone', catalog: [] });
    useModelStore.getState().setCatalog([local]);
    expect(useModelStore.getState().model).toBe('gone');
  });
});

describe('familiesOf', () => {
  const named = (id: string, name: string, providerId = 'claude'): CatalogModel => ({ ...row(id, providerId), name });

  it('puts a family on one line, its versions newest first', () => {
    const families = familiesOf([
      named('claude-opus-4-8', 'Claude Opus 4.8'),
      named('claude-opus-5-5', 'Claude Opus 5.5'),
      named('claude-sonnet-5-5', 'Claude Sonnet 5.5'),
      named('claude-opus-5', 'Claude Opus 5'),
    ]);

    expect(families.map((family) => family.label)).toEqual(['Claude Opus', 'Claude Sonnet']);
    expect(families[0]?.versions.map((version) => version.label)).toEqual(['5.5', '5', '4.8']);
    expect(families[0]?.versions[0]?.row.id).toBe('claude-opus-5-5');
    expect(families[1]?.versions.map((version) => version.label)).toEqual(['5.5']);
  });

  it('keeps a name that does not spell its version whole, without a chip', () => {
    const families = familiesOf([named('o4-mini', 'o4 Mini', 'openai-api'), named('sonnet', 'Sonnet')]);

    expect(families.map((family) => family.label)).toEqual(['o4 Mini', 'Sonnet']);
    expect(families.every((family) => family.versions.length === 1 && family.versions[0]?.label === '')).toBe(true);
  });

  it('splits a hyphenated vendor version', () => {
    const families = familiesOf([named('gpt-5-5', 'GPT-5.5', 'openai-api'), named('gpt-5', 'GPT-5', 'openai-api')]);

    expect(families[0]?.label).toBe('GPT');
    expect(families[0]?.versions.map((version) => version.label)).toEqual(['5.5', '5']);
  });
});
