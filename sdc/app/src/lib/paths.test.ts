import { describe, expect, it } from 'vitest';

import { baseName, inFolder } from './paths';

describe('inFolder', () => {
  it('joins a relative path with the folder’s own separator', () => {
    expect(inFolder('H:\\proj', 'src/pay.js')).toBe('H:\\proj\\src\\pay.js');
    expect(inFolder('/srv/app/', 'src/pay.js')).toBe('/srv/app/src/pay.js');
  });

  it('keeps an absolute path as it is', () => {
    expect(inFolder('/srv/app', '/etc/hosts')).toBe('/etc/hosts');
    expect(inFolder('H:\\proj', 'C:\\x.txt')).toBe('C:\\x.txt');
  });

  it('names the last part whichever separator is used', () => {
    expect(baseName('src\\pay.js')).toBe('pay.js');
    expect(baseName('/srv/app/pay.js')).toBe('pay.js');
  });
});
