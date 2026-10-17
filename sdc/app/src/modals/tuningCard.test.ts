import { mkdirSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';

import type { LocalTuning } from '../../../protocol/types';
import { bn } from '../locales/bn';
import { TuningCard } from './TuningCard';

const tuning: LocalTuning = {
  hardware: { gpu: 'NVIDIA GeForce RTX 3070', gpuBytes: 8_589_934_592, ramBytes: 34_359_738_368, unified: false },
  proposals: [
    {
      model: 'qwen3.5:9b',
      env: [
        { name: 'OLLAMA_FLASH_ATTENTION', value: '1' },
        { name: 'OLLAMA_KV_CACHE_TYPE', value: 'q8_0' },
      ],
      numCtx: 31_744,
      keepAlive: '-1',
      fitsOnGpu: true,
      needsRestart: true,
      notes: [],
      applied: false,
    },
    { model: 'llama3.1:70b', env: [], numCtx: 4096, keepAlive: '-1', fitsOnGpu: false, needsRestart: false, notes: [], applied: false },
  ],
  note: 'A proposal only.',
};

describe('the tuning proposal card (P0-16)', () => {
  const html = renderToStaticMarkup(createElement(TuningCard, { tuning }));

  it('shows the GPU, the context that fits, the two settings and the restart - and says it is a proposal', () => {
    expect(html).toContain('RTX 3070');
    expect(html).toContain('8.6 GB');
    expect(html).toContain('up to 31,744 tokens of context stay fully on the GPU');
    expect(html).toContain('OLLAMA_FLASH_ATTENTION=1');
    expect(html).toContain('OLLAMA_KV_CACHE_TYPE=q8_0');
    expect(html).toContain('must be restarted');
    expect(html).toContain('A proposal only');
  });

  it('says a model that will not fit will spill, instead of promising a context', () => {
    expect(html).toContain('llama3.1:70b: this card is too small');
    expect(html).not.toContain('llama3.1:70b: up to');
  });

  it('has no button: nothing here applies anything', () => {
    expect(html).not.toContain('<button');
    expect(html).not.toContain('<input');
  });

  it('speaks Bengali', () => {
    expect(bn.hub?.tuning?.title).toBe('এই কম্পিউটারের জন্য টিউনিং');
    expect(bn.hub?.tuning?.fits?.('m', 1024)).toContain('GPU-তে থাকে');
  });

  it('writes the component fixture the visual-regression script photographs', () => {
    const page = `<!doctype html><html data-theme="{{THEME}}"><head><meta charset="utf-8"><link rel="stylesheet" href="{{CSS}}"></head><body style="margin:0;padding:16px;background:var(--bg-base);color:var(--text-primary);font-family:var(--font-ui)"><div style="width:520px">${html}</div></body></html>`;
    const folder = join(dirname(fileURLToPath(import.meta.url)), '../../tests/visual/fixtures');

    mkdirSync(folder, { recursive: true });
    writeFileSync(join(folder, 'tuning-card.html'), page);
    expect(page).toContain('data-tuning');
  });
});
