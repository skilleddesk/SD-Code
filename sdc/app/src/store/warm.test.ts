import { beforeEach, describe, expect, it, vi } from 'vitest';

const call = vi.fn(() => Promise.resolve({ warming: true }));
let choice = { engine: 'native_api', model: 'qwen3-max', providerId: 'qwen' as string | null, tier: 'balanced' };

vi.mock('../lib/sdcp', () => ({ sdcpCall: (...args: unknown[]) => call(...(args as [])) }));
vi.mock('./chatModel', () => ({ chatChoice: () => choice }));

const { warmProvider } = await import('./warm');

describe('warmProvider (0.21)', () => {
  beforeEach(() => call.mockClear());

  it('opens the API provider connection once per 15 seconds of typing', () => {
    expect(warmProvider('s1', 1_000_000)).toBe(true);
    expect(call).toHaveBeenCalledWith('provider.warm', { model: 'qwen3-max', provider: 'qwen' });
    expect(warmProvider('s1', 1_005_000)).toBe(false);
    expect(warmProvider('s1', 1_016_000)).toBe(true);
    expect(call).toHaveBeenCalledTimes(2);
  });

  it('leaves CLI engines and local models alone', () => {
    choice = { engine: 'claude_code', model: 'sonnet', providerId: null, tier: 'balanced' };
    expect(warmProvider('s1', 2_000_000)).toBe(false);
    choice = { engine: 'native_api', model: 'llama3.2:3b', providerId: 'ollama', tier: 'fast' };
    expect(warmProvider('s1', 3_000_000)).toBe(false);
    expect(call).not.toHaveBeenCalled();
  });
});
