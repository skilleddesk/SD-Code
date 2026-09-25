import { describe, expect, it } from 'vitest';

import { parseInline, parseMarkdown } from './markdown';

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
