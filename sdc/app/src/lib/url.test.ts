import { describe, expect, it } from 'vitest';

import { previewAddress } from './url';

describe('previewAddress', () => {
  it('accepts a bare host and port, the way a dev server prints it', () => {
    expect(previewAddress('localhost:5173')).toBe('http://localhost:5173/');
    expect(previewAddress('  http://127.0.0.1:3000/app ')).toBe('http://127.0.0.1:3000/app');
    expect(previewAddress('https://example.test')).toBe('https://example.test/');
  });

  it('refuses anything that is not http or https', () => {
    expect(previewAddress('file:///etc/passwd')).toBeNull();
    expect(previewAddress('javascript:alert(1)')).toBeNull();
    expect(previewAddress('')).toBeNull();
  });
});
