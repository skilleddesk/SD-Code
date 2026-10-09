import { describe, expect, it } from 'vitest';

import { cells, closeOpenMarks, parseInline, parseMarkdown } from './markdown';

describe('parseMarkdown', () => {
  it('reads the blocks a model writes', () => {
    const blocks = parseMarkdown(
      ['## Done', '', 'Changed **one** file:', '', '- `src/pay.js`', '- tests', '', '```js', 'const a = 1;', '```', '', '> note', '', '1. first', '2. second'].join('\n'),
    );

    expect(blocks.map((block) => block.kind)).toEqual(['heading', 'paragraph', 'list', 'code', 'quote', 'list']);
    expect(blocks[3]).toEqual({ kind: 'code', lang: 'js', text: 'const a = 1;' });
    expect(blocks[5]).toMatchObject({ kind: 'list', ordered: true, start: 1 });
  });

  it('keeps an unclosed fence as code - an answer that is still streaming', () => {
    const blocks = parseMarkdown('Here:\n```ts\nconst x =');

    expect(blocks[1]).toEqual({ kind: 'code', lang: 'ts', text: 'const x =' });
  });

  it('never turns text into markup - HTML stays text', () => {
    const blocks = parseMarkdown('<script>alert(1)</script>');

    expect(blocks).toEqual([{ kind: 'paragraph', children: [{ kind: 'text', text: '<script>alert(1)</script>' }] }]);
  });
});

describe('parseInline', () => {
  it('reads code, bold, italic and links', () => {
    expect(parseInline('a `b` **c** *d* [e](https://x.test)')).toEqual([
      { kind: 'text', text: 'a ' },
      { kind: 'code', text: 'b' },
      { kind: 'text', text: ' ' },
      { kind: 'strong', children: [{ kind: 'text', text: 'c' }] },
      { kind: 'text', text: ' ' },
      { kind: 'em', children: [{ kind: 'text', text: 'd' }] },
      { kind: 'text', text: ' ' },
      { kind: 'link', href: 'https://x.test', children: [{ kind: 'text', text: 'e' }] },
    ]);
  });

  it('leaves snake_case, lone stars and non-http links alone', () => {
    expect(parseInline('snake_case_name')).toEqual([{ kind: 'text', text: 'snake_case_name' }]);
    expect(parseInline('2 * 3 = 6')).toEqual([{ kind: 'text', text: '2 * 3 = 6' }]);
    expect(parseInline('[x](javascript:alert(1))')).toEqual([{ kind: 'text', text: '[x](javascript:alert(1))' }]);
  });
});

describe('tables (0.21)', () => {
  it('reads a header, its alignment and the rows', () => {
    const [table] = parseMarkdown('| Name | Size |\n| :--- | ---: |\n| `a.ts` | 12 |\n| b | 3 |');

    expect(table?.kind).toBe('table');

    if (table?.kind !== 'table') {
      return;
    }

    expect(table.align).toEqual(['left', 'right']);
    expect(table.head.map((cell) => cell[0])).toEqual([{ kind: 'text', text: 'Name' }, { kind: 'text', text: 'Size' }]);
    expect(table.rows).toHaveLength(2);
    expect(table.rows[0]?.[0]).toEqual([{ kind: 'code', text: 'a.ts' }]);
  });

  it('keeps an escaped pipe in its cell and leaves a lone pipe in prose alone', () => {
    expect(cells('| a \\| b | c |')).toEqual(['a | b', 'c']);
    expect(parseMarkdown('use a | b here\nnext line')[0]?.kind).toBe('paragraph');
  });
});

describe('closeOpenMarks (0.21)', () => {
  it('closes bold and code a streaming line has opened', () => {
    expect(closeOpenMarks('This is **bold so fa')).toBe('This is **bold so fa**');
    expect(closeOpenMarks('run `pnpm te')).toBe('run `pnpm te`');
    expect(closeOpenMarks('**a** and `b`')).toBe('**a** and `b`');
  });

  it('adds nothing inside an open fence or to a marker with nothing after it yet', () => {
    expect(closeOpenMarks('```ts\nconst a = `x')).toBe('```ts\nconst a = `x');
    expect(closeOpenMarks('Then **')).toBe('Then **');
    expect(closeOpenMarks('`a ** b')).toBe('`a ** b`');
  });
});
