import { describe, expect, it } from 'vitest';

import { code, currentCodes, decodeBase32, newSecret, otpauth, stepOf, verify } from '../src/totp';

/** The RFC 6238 test secret: "12345678901234567890", in base32. */
const RFC_SECRET = 'GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ';

describe('TOTP', () => {
  it('matches the RFC 6238 vectors at every point of the message', async () => {
    /* The vectors, from the RFC: 8 digits; SDC keeps the last 6 of the same counter. */
    const vectors: [number, string][] = [
      [59_000, '94287082'],
      [1_111_111_109_000, '07081804'],
      [1_111_111_111_000, '14050471'],
      [1_234_567_890_000, '89005924'],
      [2_000_000_000_000, '69279037'],
      [20_000_000_000_000, '65353130'],
    ];

    for (const [at, expected] of vectors) {
      expect((await code(RFC_SECRET, stepOf(at))).slice(-6)).toBe(expected.slice(-6));
    }
  });

  it('accepts the code of this step and one either side, and refuses anything else', async () => {
    const at = 1_700_000_000_000;

    expect(await verify(RFC_SECRET, await code(RFC_SECRET, stepOf(at)), at)).toBe(stepOf(at));
    expect(await verify(RFC_SECRET, await code(RFC_SECRET, stepOf(at) - 1), at)).toBe(stepOf(at) - 1);
    expect(await verify(RFC_SECRET, await code(RFC_SECRET, stepOf(at) + 1), at)).toBe(stepOf(at) + 1);
    expect(await verify(RFC_SECRET, '000000', at)).not.toBe(stepOf(at));
    expect(await verify(RFC_SECRET, '12345', at)).toBeNull();
    expect(await verify(RFC_SECRET, 'abcdef', at)).toBeNull();
    expect(await verify(RFC_SECRET, await code(RFC_SECRET, stepOf(at) - 4), at)).toBeNull();
  });

  it('refuses a code whose step was already spent', async () => {
    const at = 1_700_000_000_000;
    const good = await code(RFC_SECRET, stepOf(at));

    expect(await verify(RFC_SECRET, good, at, [stepOf(at)])).toBeNull();
    expect(await verify(RFC_SECRET, good, at, [stepOf(at) - 1, stepOf(at) + 1])).toBe(stepOf(at));
  });

  it('reads the secrets a phone shows, and refuses what is not one', () => {
    expect(new TextDecoder().decode(decodeBase32('GEZDGNBVGY3TQOJQ'))).toBe('1234567890');
    expect(decodeBase32('gezd gnbv gy3t qojq').length).toBe(10);
    expect(() => decodeBase32('1234567890')).toThrow('not a base32 secret');

    const secret = newSecret();
    const uri = otpauth(secret, 'someone@example.com');

    expect(secret).toMatch(/^[A-Z2-7]{32}$/);
    expect(decodeBase32(secret).length).toBe(20);
    expect(uri).toContain(`secret=${secret}`);
    expect(uri.startsWith('otpauth://totp/SDC%20Anywhere:')).toBe(true);
  });

  it('gives a fresh code as the seconds pass', async () => {
    const codes = await currentCodes(RFC_SECRET, 1_700_000_000_000);

    expect(codes).toHaveLength(3);
    expect(new Set(codes).size).toBe(3);
  });
});
