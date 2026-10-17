import { readFileSync } from 'node:fs';
import { join } from 'node:path';

import { describe, expect, it } from 'vitest';

const root = join(__dirname, '..');
const tokens = readFileSync(join(root, 'src/styles/tokens.css'), 'utf8');
const guide = readFileSync(join(root, '../docs/UI-GUIDE.md'), 'utf8');

describe('docs/UI-GUIDE.md', () => {
  it('lists every design token the stylesheet declares', () => {
    const declared = [...new Set([...tokens.matchAll(/^\s*(--[a-z0-9-]+):/gm)].map((match) => match[1]))];
    const missing = declared.filter((name) => !guide.includes(`\`${name}\``));

    expect(declared.length).toBeGreaterThan(50);
    expect(missing).toEqual([]);
  });

  it('names the components the plan says new UI is modelled on', () => {
    for (const name of ['ModelSelector', 'TrustChip', 'PlanCard', 'QuestionCard', 'ResearchCard', 'CostMeter', 'PolicyEditor', 'ProofTab', 'TimeMachineTab']) {
      expect(guide).toContain(name);
    }
  });
});
