import { describe, expect, it } from 'vitest';

import { bn } from '../../locales/bn';
import { strings } from '../../strings';

describe('the model class label (P0-9)', () => {
  it('reads in English and in Bengali, with a ? for a provisional measurement', () => {
    expect(strings.prompt.model.probeClass('M?', 16384)).toBe('class M? · 16K context');
    expect(bn.prompt?.model?.probeClass?.('S', 8192)).toBe('ক্লাস S · 8K context');
  });
});
