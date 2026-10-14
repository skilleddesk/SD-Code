// X4: the fence a screen's frames carry. The shape is what the relay reads, so it is pinned here.

import { describe, expect, it } from 'vitest';

import { frameOn } from '../src/transport/link';

describe('a frame on the wire', () => {
  it('carries no fence until a screen holds the session', () => {
    expect(frameOn(7, 'AAAA', 0)).toEqual({ t: 'f', n: 7, ct: 'AAAA' });
  });

  it('carries the fence it was given, so the relay can tell whose turn it is', () => {
    expect(frameOn(7, 'AAAA', 3)).toEqual({ t: 'f', n: 7, ct: 'AAAA', fence: 3 });
  });

  it('keeps the sealed payload exactly as it was', () => {
    const one = frameOn(1, 'ZZZZ', 0);
    const two = frameOn(1, 'ZZZZ', 9);

    expect(one.ct).toBe(two.ct);
    expect(one.n).toBe(two.n);
  });
});
