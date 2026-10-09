import { describe, expect, it } from 'vitest';

import { languageOf, shellSpans, spansWith } from './highlight';

describe('highlight (0.21)', () => {
  it('maps fence labels to grammars', () => {
    expect(languageOf('TypeScript')).toBe('ts');
    expect(languageOf('tsx')).toBe('ts');
    expect(languageOf('bash')).toBe('sh');
    expect(languageOf('py')).toBe('py');
    expect(languageOf('cobol')).toBe('');
  });

  it('colours TypeScript with the editor grammar, line by line, losing no text', async () => {
    const { javascriptLanguage } = await import('@codemirror/lang-javascript');
    const code = 'const a: number = 1; // one\nfunction f() { return "x"; }';
    const lines = spansWith(javascriptLanguage.parser.configure({ dialect: 'ts' }), code);

    expect(lines).toHaveLength(2);
    expect(lines.map((line) => line.map((span) => span.text).join('')).join('\n')).toBe(code);
    expect(lines[0]?.find((span) => span.text === 'const')?.cls).toBe('hl-kw');
    expect(lines[0]?.find((span) => span.text === '// one')?.cls).toBe('hl-cmt');
    expect(lines[1]?.find((span) => span.text === '"x"')?.cls).toBe('hl-str');
  });

  it('colours a shell command by hand', () => {
    const [line] = shellSpans('pnpm test --run "a b" # done');

    expect(line?.map((span) => span.text).join('')).toBe('pnpm test --run "a b" # done');
    expect(line?.[1]).toEqual({ text: 'pnpm', cls: 'hl-fn' });
    expect(line?.find((span) => span.cls === 'hl-str')?.text).toBe('"a b"');
    expect(line?.find((span) => span.cls === 'hl-cmt')?.text).toBe('# done');
    expect(line?.find((span) => span.cls === 'hl-prop')?.text).toBe(' --run');
  });
});
