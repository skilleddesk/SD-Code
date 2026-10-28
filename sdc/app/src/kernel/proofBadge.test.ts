import { mkdirSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';

import type { ReceiptRow, ReceiptTotals } from '../../../protocol/types';
import { bn } from '../locales/bn';
import { strings } from '../strings';
import { ProofBadgeView } from './ProofBadge';
import { badgeParts, modelsLine, type ProofFacts } from './proofFacts';

const rows: ReceiptRow[] = [
  { modelKey: 'ollama:qwen3.5:9b', mode: 'local-rotation', purpose: 'main', local: true, chain: false, steps: 2, tokensIn: 2500, tokensOut: 180, wallMs: 5500, costUsd: null },
  { modelKey: 'deepseek:deepseek-chat', mode: 'local-rotation', purpose: 'chain', local: false, chain: true, steps: 1, tokensIn: 2000, tokensOut: 300, wallMs: 6000, costUsd: 0.0012 },
];
const totals: ReceiptTotals = { steps: 3, tokensIn: 4500, tokensOut: 480, wallMs: 11500, costUsd: 0.0012, localSteps: 2, chainSteps: 1 };
const passed: ProofFacts['verify'] = { state: 'done', pass: true, checksPassed: 4, checksFailed: 0 };

describe('the Proof-of-Work badge (P0-15)', () => {
  it('shows only what was measured', () => {
    expect(badgeParts({ verify: passed, lines: 14, totals, rows })).toEqual(['✓ 4 checks passed', '14 lines changed', 'ollama:qwen3.5:9b (2) + deepseek:deepseek-chat (1)']);
    /* No Verify run, nothing changed, no steps traced: the badge says it is not verified and nothing else. */
    expect(badgeParts({ verify: null, lines: null, totals: null, rows: [] })).toEqual(['not verified']);
    expect(badgeParts({ verify: { state: 'running', pass: null, checksPassed: 0, checksFailed: 0 }, lines: 0, totals: null, rows: [] })).toEqual(['not verified']);
    expect(badgeParts({ verify: { state: 'done', pass: false, checksPassed: 3, checksFailed: 2 }, lines: null, totals: null, rows: [] })).toEqual(['✗ 2 checks failed']);
  });

  it('names each model once with its steps', () => {
    expect(modelsLine([...rows, { ...rows[0]!, purpose: 'sub', steps: 1 }])).toBe('ollama:qwen3.5:9b (3) + deepseek:deepseek-chat (1)');
  });

  it('renders the badge, and the receipt with a total that is the sum of its rows', () => {
    const html = renderToStaticMarkup(createElement(ProofBadgeView, { facts: { verify: passed, lines: 14, totals, rows }, open: true, onToggle: () => undefined }));

    expect(html).toContain('✓ 4 checks passed · 14 lines changed');
    expect(html).toContain('data-receipt-row="ollama:qwen3.5:9b"');
    expect(html).toContain('data-receipt-row="deepseek:deepseek-chat"');
    expect(html).toContain('2 local · 1 chain');
    /* 1.0.0: no price anywhere on screen. */
    expect(html).not.toContain('$');
    expect(html).toContain('aria-expanded="true"');
    expect(rows.reduce((sum, row) => sum + row.steps, 0)).toBe(totals.steps);
    expect(rows.reduce((sum, row) => sum + row.tokensIn, 0)).toBe(totals.tokensIn);
    expect(rows.reduce((sum, row) => sum + row.wallMs, 0)).toBe(totals.wallMs);
  });

  it('closed, shows the badge and the button but no table; with no steps traced, no receipt button', () => {
    const closed = renderToStaticMarkup(createElement(ProofBadgeView, { facts: { verify: passed, lines: null, totals, rows }, open: false, onToggle: () => undefined }));

    expect(closed).toContain('Receipt') ;
    expect(closed).not.toContain('receipt-table');

    const none = renderToStaticMarkup(createElement(ProofBadgeView, { facts: { verify: null, lines: null, totals: null, rows: [] }, open: false, onToggle: () => undefined }));

    expect(none).not.toContain('<button');
  });

  it('a failed run is drawn as a failure, not a pass', () => {
    const html = renderToStaticMarkup(createElement(ProofBadgeView, { facts: { verify: { state: 'done', pass: false, checksPassed: 1, checksFailed: 1 }, lines: 3, totals: null, rows: [] }, open: false, onToggle: () => undefined }));

    expect(html).toContain('✗ 1 check failed');
    expect(html).toContain('text-state-error');
  });

  it('speaks Bengali too', () => {
    expect(bn.turns?.proof?.verified?.(4)).toBe('✓ 4টা check পাস');
    expect(bn.turns?.proof?.split?.(2, 1)).toBe('2টা লোকাল · 1টা chain');
    expect(bn.turns?.proof?.unverified).toBe('যাচাই হয়নি');
    expect(strings.turns.proof.verified(1)).toBe('✓ 1 check passed');
  });

  it('writes the component fixture the visual-regression script photographs (tests/visual/fixtures)', () => {
    const view = (open: boolean, failed: boolean) =>
      renderToStaticMarkup(
        createElement(ProofBadgeView, { facts: { verify: failed ? { state: 'done', pass: false, checksPassed: 1, checksFailed: 2 } : passed, lines: 14, totals, rows }, open, onToggle: () => undefined }),
      );
    /* `{{CSS}}` is the built stylesheet's name, which the script knows and a test does not. */
    const page = `<!doctype html><html data-theme="{{THEME}}"><head><meta charset="utf-8"><link rel="stylesheet" href="{{CSS}}"></head><body style="margin:0;padding:16px;background:var(--bg-base);color:var(--text-primary);font-family:var(--font-ui)"><div style="width:600px">${view(false, false)}${view(true, false)}${view(false, true)}</div></body></html>`;
    const folder = join(dirname(fileURLToPath(import.meta.url)), '../../tests/visual/fixtures');

    mkdirSync(folder, { recursive: true });
    writeFileSync(join(folder, 'proof-badge.html'), page);
    expect(page).toContain('data-proof-badge');
  });
});
