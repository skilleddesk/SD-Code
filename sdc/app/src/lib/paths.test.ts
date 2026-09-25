import { describe, expect, it } from 'vitest';

import { baseName, inFolder, isUnder, samePath } from './paths';

describe('inFolder', () => {
  it('joins a relative path with the folder’s own separator', () => {
    expect(inFolder('H:\\proj', 'src/pay.js')).toBe('H:\\proj\\src\\pay.js');
    expect(inFolder('/srv/app/', 'src/pay.js')).toBe('/srv/app/src/pay.js');
  });

  it('keeps an absolute path as it is', () => {
    expect(inFolder('/srv/app', '/etc/hosts')).toBe('/etc/hosts');
    expect(inFolder('H:\\proj', 'C:\\x.txt')).toBe('C:\\x.txt');
  });

  it('treats one Windows file spelled two ways as the same file', () => {
    expect(samePath('C:/p/demo\\notes.md', 'C:/p/demo/notes.md')).toBe(true);
    expect(samePath('c:\\P\\Demo\\a.ts', 'C:/p/demo/a.ts')).toBe(true);
    expect(isUnder('C:/p/demo\\src\\a.ts', 'C:/p/demo/src')).toBe(true);
    expect(isUnder('C:/p/demo/srcs/a.ts', 'C:/p/demo/src')).toBe(false);
  });

  it('keeps a host path case-sensitive', () => {
    expect(samePath('/srv/App/a.ts', '/srv/app/a.ts')).toBe(false);
    expect(samePath('/srv/app/', '/srv/app')).toBe(true);
  });

  it('names the last part whichever separator is used', () => {
    expect(baseName('src\\pay.js')).toBe('pay.js');
    expect(baseName('/srv/app/pay.js')).toBe('pay.js');
  });
});
