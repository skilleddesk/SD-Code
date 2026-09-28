import { describe, expect, it } from 'vitest';

import { changedFiles, gather, summary } from './grouping';
import type { ReadToolData, TimelineItem } from './types';

const read = (name: string, target: string): TimelineItem => ({
  kind: 'tool',
  key: `${name}-${target}`,
  tool: { kind: 'read', name, target, status: 'done', meta: 'done', startedAt: '' },
});

const edit = (target: string, meta: string): TimelineItem => ({
  kind: 'tool',
  key: `edit-${target}-${meta}`,
  tool: { kind: 'edit', name: 'Edit', target, status: 'done', meta, startedAt: '', diff: [] },
});

describe('the tidy timeline (0.13)', () => {
  it('folds two or more reads in a row into one explored line, and leaves a lone read alone', () => {
    const items: TimelineItem[] = [
      read('Grep', 'add'),
      read('Read', 'src/math.js'),
      read('Read', 'test.js'),
      edit('src/math.js', 'done · +8 −4'),
      read('Read', 'src/math.js'),
      read('Agent', 'List functions'),
    ];
    const drawn = gather(items);

    expect(drawn.map((item) => item.kind)).toEqual(['explore', 'tool', 'tool', 'tool']);
    expect(drawn[0]?.kind === 'explore' ? drawn[0].tools.length : 0).toBe(3);
  });

  it('summarises reads by kind with file names', () => {
    const tools = gather([read('Grep', 'add'), read('Read', 'src/a.js'), read('Read', 'src/b.js'), read('Read', 'c.js'), read('Read', 'd.js')]);
    const group = tools[0];

    expect(group?.kind === 'explore' ? summary(group.tools as ReadToolData[]) : '').toBe('Grep add · Read a.js, b.js, c.js +1');
  });

  it('adds up the files a turn changed', () => {
    expect(changedFiles([edit('a.js', 'done · +3 −1'), edit('b.js', 'done · +10 −0'), edit('a.js', 'done · +2 −2')])).toEqual([
      { path: 'a.js', added: 5, removed: 3 },
      { path: 'b.js', added: 10, removed: 0 },
    ]);
  });
});
