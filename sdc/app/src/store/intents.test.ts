import { beforeEach, describe, expect, it, vi } from 'vitest';

/**
 * `chooseModel`'s acceptance test - the regression behind *"connect hoy claude but chat e kisu likhle kaj
 * hoy nah"*.
 *
 * The intent writes two places, and only one of them used to exist: `models.select` tells the daemon
 * which model the setting is, and the window's own model store has to move with it, because `sendPrompt`
 * reads the store. Pressing `Use` on a row in the Connect dialog therefore changed the daemon's setting,
 * showed `In use` on that row, and left the chat running the model it already had.
 *
 * The daemon is mocked rather than stood in for: this file is about what the *window* does with the
 * answer, and the stand-in refuses `models.select` on purpose (a browser has no daemon settings to
 * write).
 */
const sdcpCall = vi.hoisted(() => vi.fn());

vi.mock('../lib/sdcp', () => ({ sdcpCall }));

const { chooseModel } = await import('./intents');
const { engineForProvider, useModelStore } = await import('./model');

describe('chooseModel', () => {
  beforeEach(() => {
    sdcpCall.mockReset();
    sdcpCall.mockResolvedValue({});
    useModelStore.setState({
      tier: 'balanced',
      engine: 'claude_code',
      model: 'sonnet',
      providerId: 'claude',
      catalog: [
        { id: 'deepseek-v4-pro', providerId: 'deepseek', providerLabel: 'DeepSeek', tier: 'balanced', ctx: 0, cost: '', name: 'DeepSeek V4 Pro', source: 'cache' },
        { id: 'deepseek-flash', providerId: 'deepseek', providerLabel: 'DeepSeek', tier: 'fast', ctx: 0, cost: '', name: 'DeepSeek Flash', source: 'cache' },
      ],
    });
  });

  it('tells the daemon and moves the window', async () => {
    await expect(chooseModel('deepseek-v4-pro', 'deepseek')).resolves.toBe(true);

    expect(sdcpCall).toHaveBeenCalledWith('models.select', { modelId: 'deepseek-v4-pro', providerId: 'deepseek' });

    const state = useModelStore.getState();

    /* The window's four facts, which are what `sendPrompt` sends: without the last three the chat kept
       running claude_code/sonnet while the dialog said `In use`. */
    expect(state.model).toBe('deepseek-v4-pro');
    expect(state.providerId).toBe('deepseek');
    expect(state.engine).toBe(engineForProvider('deepseek'));
    expect(state.tier).toBe('balanced');
  });

  it('takes the tier from the row the catalogue listed', async () => {
    await chooseModel('deepseek-flash', 'deepseek');

    expect(useModelStore.getState().tier).toBe('fast');
    expect(useModelStore.getState().model).toBe('deepseek-flash');
  });

  it('leaves the window alone when the daemon refuses', async () => {
    sdcpCall.mockRejectedValue(new Error('unsupported'));

    await expect(chooseModel('deepseek-v4-pro', 'deepseek')).resolves.toBe(false);

    /* A model the daemon did not accept must not be the model the chat claims to be on. */
    expect(useModelStore.getState().model).toBe('sonnet');
    expect(useModelStore.getState().engine).toBe('claude_code');
  });

  it('resolves each provider to its own engine', () => {
    expect(engineForProvider('claude')).toBe('claude_code');
    expect(engineForProvider('openai')).toBe('codex');
    expect(engineForProvider('gemini')).toBe('gemini');
    expect(engineForProvider('deepseek')).toBe('native_api');
  });
});
