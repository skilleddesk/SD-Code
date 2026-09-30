import { afterEach, describe, expect, it, vi } from 'vitest';

import { platformHint } from './external';

/* The window's platform, as the webview reports it (0.15.8: the macOS/Linux fixes key off it). */
function onPlatform(userAgent: string): void {
  vi.stubGlobal('navigator', { userAgent });
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('platformHint', () => {
  it('names the Mac keys on a Mac', () => {
    onPlatform('Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15');

    expect(platformHint('Ctrl K')).toBe('⌘ K');
    expect(platformHint('Ctrl Shift Z')).toBe('⌘ ⇧ Z');
  });

  it('leaves Windows and Linux as they are', () => {
    onPlatform('Mozilla/5.0 (Windows NT 10.0; Win64; x64) Edg/140.0');
    expect(platformHint('Ctrl Shift Z')).toBe('Ctrl Shift Z');

    onPlatform('Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15');
    expect(platformHint('Ctrl K')).toBe('Ctrl K');
  });
});
