import { describe, expect, it } from 'vitest';

import { modelMode } from './model';
import { strings } from '../strings';
import { bn } from '../locales/bn';

describe('the model-selection contract badge (P0-10)', () => {
  it('a cloud, API or CLI model is Pinned whatever chain is on file', () => {
    for (const provider of ['claude', 'deepseek', 'openai-api', 'anthropic-api', 'codex', 'gemini', null]) {
      expect(modelMode(provider, 0)).toBe('pinned');
      expect(modelMode(provider, 3)).toBe('pinned');
    }
  });

  it('a local model is Local Rotation with an enabled chain row and Local-only without one', () => {
    expect(modelMode('ollama', 2)).toBe('local-rotation');
    expect(modelMode('ollama', 0)).toBe('local-only');
  });

  it('says so in English and in Bengali', () => {
    expect(strings.prompt.model.mode.pinned).toBe('Pinned');
    expect(strings.prompt.model.mode.localRotation(3)).toBe('Local Rotation (3)');
    expect(strings.prompt.model.mode.localOnly).toBe('Local-only');
    expect(bn.prompt?.model?.mode?.localRotation?.(3)).toBe('লোকাল রোটেশন (3)');
    expect(bn.prompt?.model?.mode?.localOnly).toBe('শুধু লোকাল');
  });
});
