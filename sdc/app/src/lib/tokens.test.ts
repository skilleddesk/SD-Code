import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

import { estimateTokens } from './tokens';

interface Sample {
  name: string;
  text: string;
  cl100k: number;
  o200k: number;
  estimate: number;
}

const fixtures = JSON.parse(readFileSync(resolve(__dirname, '../../../protocol/token-fixtures.json'), 'utf8')) as { samples: Sample[] };

describe('estimateTokens (shared fixtures with sdcd/src/tokens.rs)', () => {
  for (const sample of fixtures.samples) {
    it(`${sample.name}: equals the Rust estimate and never undercounts a real tokenizer`, () => {
      const got = estimateTokens(sample.text);

      expect(got).toBe(sample.estimate);
      expect(got).toBeGreaterThanOrEqual(Math.max(sample.cl100k, sample.o200k));
    });
  }

  it('counts Bengali far above the old chars / 4 rule', () => {
    const bengali = fixtures.samples.find((sample) => sample.name === 'bengali')!;

    expect(estimateTokens(bengali.text)).toBeGreaterThan((bengali.text.length / 4) * 3);
  });

  it('counts an empty text as nothing', () => {
    expect(estimateTokens('')).toBe(0);
  });
});
