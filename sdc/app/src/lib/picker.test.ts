import { describe, expect, it } from 'vitest';

import { nameOf, pathsOf } from './picker';

/**
 * The two shapes a picked file arrives in - the part of `lib/picker.ts` that is a decision rather than
 * a dialog.
 *
 * `nameOf` is what a chip would show beside a path, and it must not care which way Windows or POSIX
 * writes a separator (a person can be handed `C:\work\a.ts` by this machine and `/home/x/a.ts` by a
 * VPS in the same list). `pathsOf` is the dialog's three answers - one string, an array, or `null` for
 * a cancel - collapsed into one shape, which is what keeps a cancel from becoming a path called
 * `"null"`.
 */
describe('nameOf', () => {
  it('takes the last segment whichever separator the path uses', () => {
    expect(nameOf('C:\\work\\src\\a.ts')).toBe('a.ts');
    expect(nameOf('/home/x/src/a.ts')).toBe('a.ts');
    expect(nameOf('a.ts')).toBe('a.ts');
    expect(nameOf('C:\\work\\src\\')).toBe('');
  });
});

describe('pathsOf', () => {
  it('reads all three answers the dialog can give', () => {
    expect(pathsOf(null)).toEqual([]);
    expect(pathsOf(undefined)).toEqual([]);
    expect(pathsOf('C:\\a.ts')).toEqual(['C:\\a.ts']);
    expect(pathsOf(['C:\\a.ts', 'C:\\b.ts'])).toEqual(['C:\\a.ts', 'C:\\b.ts']);
  });
});
