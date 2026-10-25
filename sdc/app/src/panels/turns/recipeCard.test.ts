import { mkdirSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';

import type { SdcpEvent, TaskProgressEvent } from '../../../../protocol/types';
import { bn } from '../../locales/bn';
import { applyEvent, EMPTY_STATE } from '../../store/reducer';
import { strings } from '../../strings';
import { RecipeCardView } from './RecipeCard';
import { cardFacts, isRecipe, parseFindings } from './recipeFacts';

const lines = [
  'src/pay.js:12 · low · style · long line · 140 chars',
  'src/auth.js:40 · high · security · token is logged · console.log(token)',
  'https://shop.test/cart · medium · links · broken link to /old · 404',
  'src/db.js:7 · high · bug · query is never closed · conn.query(sql)',
  '#ran crawled 12 pages',
  '#skipped TLS expiry',
];

function progress(extra: Partial<TaskProgressEvent> = {}): TaskProgressEvent {
  return { type: 'TaskProgress', sessionId: 's1', turnId: 't1', taskId: 'k1', done: 3, blocked: 0, total: 4, label: 'done', recipe: 'R-REVIEW-CODE', ...extra };
}

function fold(state: typeof EMPTY_STATE, event: SdcpEvent) {
  return applyEvent(state, { seq: state.seq + 1, ts: '2026-10-26T10:00:00.000Z', event });
}

describe('the recipe card (P3-6)', () => {
  it('sorts findings by severity, keeps order inside a severity, and drops the # meta lines', () => {
    const found = parseFindings(lines);

    expect(found.map((finding) => finding.severity)).toEqual(['high', 'high', 'medium', 'low']);
    expect(found.map((finding) => finding.place)).toEqual(['src/auth.js:40', 'src/db.js:7', 'https://shop.test/cart', 'src/pay.js:12']);
  });

  it('turns a place into a file and a line, and leaves a web address as plain text', () => {
    const [auth, , cart] = parseFindings(lines);

    expect(auth).toMatchObject({ file: 'src/auth.js', line: 40, problem: 'token is logged', evidence: 'console.log(token)' });
    expect(cart).toMatchObject({ file: null, line: null });
    expect(parseFindings(['index.html · low · seo · no title · none'])[0]).toMatchObject({ file: 'index.html', line: null });
    expect(parseFindings(undefined)).toEqual([]);
  });

  it('knows its numbers: the bar fraction and the local/cloud split only when something ran', () => {
    expect(cardFacts(progress()).fraction).toBe(0.75);
    expect(cardFacts(progress({ total: 0, done: 0 })).fraction).toBe(0);
    expect(cardFacts(progress()).split).toBeNull();
    expect(cardFacts(progress({ local: 5, cloud: 0 })).split).toEqual([5, 0]);
    expect(isRecipe(progress())).toBe(true);
    expect(isRecipe(progress({ recipe: 'plain' }))).toBe(false);
    expect(isRecipe(undefined)).toBe(false);
  });

  it('keeps the newest event per turn and does not erase findings or the split when a later event has none', () => {
    let state = fold(EMPTY_STATE, progress({ done: 1, total: 4, label: 'paused' }));

    state = fold(state, progress({ done: 4, total: 4, findings: lines, local: 6, cloud: 0 }));
    state = fold(state, progress({ done: 4, total: 4 }));

    expect(state.taskProgress['t1']).toMatchObject({ done: 4, findings: lines, local: 6, cloud: 0 });
    expect(state.taskProgress['t2']).toBeUndefined();
  });

  it('draws the chip, unit n of m, the finding count, the split and the links', () => {
    const html = renderToStaticMarkup(createElement(RecipeCardView, { progress: progress({ findings: lines, local: 6, cloud: 0 }), onOpen: () => undefined }));

    expect(html).toContain('Code review');
    expect(html).toContain('unit 3 of 4');
    expect(html).toContain('4 findings');
    expect(html).toContain('6 on this computer · 0 in the cloud');
    expect(html).toContain('finding-link');
    expect(html.indexOf('src/auth.js:40')).toBeLessThan(html.indexOf('src/pay.js:12'));
    /* A web address is shown, not made a link. */
    expect(html).toContain('https://shop.test/cart');
    expect(html).not.toContain('title="Open https://shop.test/cart"');
  });

  it('shows no findings line while a recipe has none yet, and says so when a finished review found nothing', () => {
    const running = renderToStaticMarkup(createElement(RecipeCardView, { progress: progress({ label: 'running', recipe: 'R-BUILD-PAGE' }), onOpen: null }));

    expect(running).toContain('Build a page');
    expect(running).not.toContain('finding');

    const clean = renderToStaticMarkup(createElement(RecipeCardView, { progress: progress({ findings: ['#ran nothing'] }), onOpen: null }));

    expect(clean).toContain('No findings.');
    expect(clean).not.toContain('<button');
  });

  it('speaks Bengali too', () => {
    expect(bn.turns?.recipe?.units?.(3, 4)).toBe('ইউনিট 4-এর মধ্যে 3');
    expect(bn.turns?.recipe?.names?.['R-FIX']).toBe('ফেইলিং টেস্ট ঠিক করা');
    expect(bn.turns?.recipe?.findings?.(2)).toBe('2টা পাওয়া সমস্যা');
    expect(strings.turns.recipe.findings(1)).toBe('1 finding');
  });

  it('writes the component fixture the visual-regression script photographs in light and dark (tests/visual/fixtures)', () => {
    const card = (extra: Partial<TaskProgressEvent>) => renderToStaticMarkup(createElement(RecipeCardView, { progress: progress(extra), onOpen: () => undefined }));
    const page = `<!doctype html><html data-theme="{{THEME}}"><head><meta charset="utf-8"><link rel="stylesheet" href="{{CSS}}"></head><body style="margin:0;padding:16px;background:var(--bg-base);color:var(--text-primary);font-family:var(--font-ui)"><div style="width:620px">${card({ findings: lines, local: 6, cloud: 0 })}${card({ recipe: 'R-BUILD-PAGE', done: 2, total: 5, label: 'running' })}${card({ recipe: 'R-FIX', done: 1, blocked: 1, total: 4, label: 'blocked', findings: [] })}</div></body></html>`;
    const folder = join(dirname(fileURLToPath(import.meta.url)), '../../../tests/visual/fixtures');

    mkdirSync(folder, { recursive: true });
    writeFileSync(join(folder, 'recipe-card.html'), page);
    expect(page).toContain('data-recipe="R-REVIEW-CODE"');
  });
});
