import { describe, expect, it } from 'vitest';

import {
  engineConnected,
  groupCatalog,
  latestVersions,
  nextEngine,
  parseModelId,
  type CatalogModel,
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

    expect(groups.map((group) => group.providerId)).toEqual(['claude', 'anthropic-api']);
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

  it('has no groups when nothing is connected', () => {
    const { groups, disconnected } = groupCatalog(catalog, []);

    expect(groups).toEqual([]);
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
