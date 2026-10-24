import { describe, expect, it } from 'vitest';

import { bn } from '../../locales/bn';
import { strings } from '../../strings';

describe('the model class label (P0-9)', () => {
  it('reads in English and in Bengali, with a ? for a provisional measurement', () => {
    expect(strings.prompt.model.probeClass('M?', 16384)).toBe('class M? · 16K context');
    expect(bn.prompt?.model?.probeClass?.('S', 8192)).toBe('ক্লাস S · 8K context');
  });

  it('has a line of advice for a 4-bit setup, in English and in Bengali (N1-2)', () => {
    for (const code of ['quant4', 'kv4', 'quant4kv4']) {
      expect(strings.prompt.model.quantAdvice(code).length).toBeGreaterThan(20);
      expect(bn.prompt?.model?.quantAdvice?.(code)).toMatch(/[\u0980-\u09FF]/);
      expect(bn.prompt?.model?.quantAdvice?.(code)).not.toBe(strings.prompt.model.quantAdvice(code));
    }

    expect(strings.prompt.model.quantAdvice('quant4')).toContain('Q5_K_M');
  });
});
