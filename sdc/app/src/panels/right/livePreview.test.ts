import { describe, expect, it } from 'vitest';

import type { TurnView } from '../../store/types';
import { lastChange, previewCandidates } from './livePreview';

function turn(overrides: Partial<TurnView>): TurnView {
  return {
    id: 't1',
    turnNumber: 1,
    sessionId: 's1',
    engine: 'native_api',
    model: 'qwen3.8-max',
    tier: 'Balanced',
    prompt: '',
    text: '',
    thinking: '',
    thinkingMs: 0,
    thinkingSince: null,
    plan: [],
    startedAt: '2026-09-28T10:00:00Z',
    status: 'done',
    stuckForMs: 0,
    tools: [],
    timeline: [],
    summary: '',
    meta: '',
    pass: null,
    ...overrides,
  };
}

const run = (output: string[], target = 'npm run dev') => ({
  callId: 'c1',
  startedAt: '2026-09-28T10:00:01Z',
  tool: 'run' as const,
  name: 'Run',
  target,
  status: 'done' as const,
  meta: '',
  diff: [],
  output: output.map((text) => ({ level: 'dim' as const, text })),
});

describe('live preview', () => {
  it('finds the dev server a chat started, and a site by its domain', () => {
    const turns = [turn({ tools: [run(['  VITE v6  ready', '  ➜  Local:   http://localhost:5173/', '  ➜  Network: http://0.0.0.0:5173/'])] })];
    const project = { id: 'pr1', hostId: 'h1', root: '/var/www/example-shop.com', name: 'example-shop.com', chats: 1 };

    expect(previewCandidates(turns, 's1', project)).toEqual(['http://localhost:5173/', 'https://example-shop.com/']);
    expect(previewCandidates(turns, 'other', undefined)).toEqual([]);
  });

  it('reloads after the newest finished edit', () => {
    const edit = { ...run([], 'src/App.tsx'), callId: 'c2', tool: 'edit' as const, name: 'Edit' };

    expect(lastChange([turn({ tools: [run([]), edit] })], 's1')).toEqual({ key: 't1/c2', target: 'src/App.tsx' });
    expect(lastChange([turn({ tools: [run([])] })], 's1')).toBeNull();
  });
});
