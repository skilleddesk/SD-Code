import { highlightCode, tagHighlighter, tags, type Highlighter } from '@lezer/highlight';
import { useEffect, useMemo, useState } from 'react';

/**
 * Colours for code in an answer (0.21) - the same palette as the file editor (`panels/right/CodeEditor`),
 * so a snippet in the chat and the file it lands in look alike.
 *
 * The parsers are CodeMirror's own Lezer grammars, which the app already ships for the editor; they are
 * imported on first use, one language at a time, so a chat that never shows code never loads them. Until
 * a parser has arrived the block is drawn plain, and it stays plain for a language without a grammar
 * here. A shell block (the most common one in an answer) gets a small hand-written colouring instead.
 *
 * It runs on every frame of a streaming answer, which is fine: Lezer parses a few kilobytes in well
 * under a millisecond, and an unfinished block is still a valid prefix for it.
 */

/** A Lezer parser - typed by what `highlightCode` takes, so `@lezer/common` need not be a direct dependency. */
interface Parser {
  parse(input: string): Parameters<typeof highlightCode>[1];
}

/** One coloured run of text; `cls` is empty for plain text. */
export interface Span {
  text: string;
  cls: string;
}

const highlighter: Highlighter = tagHighlighter([
  { tag: [tags.keyword, tags.modifier, tags.controlKeyword, tags.operatorKeyword, tags.definitionKeyword, tags.moduleKeyword], class: 'hl-kw' },
  { tag: [tags.string, tags.special(tags.string), tags.regexp, tags.character], class: 'hl-str' },
  { tag: [tags.number, tags.bool, tags.null, tags.atom], class: 'hl-num' },
  { tag: [tags.function(tags.variableName), tags.function(tags.propertyName), tags.macroName], class: 'hl-fn' },
  { tag: [tags.typeName, tags.className, tags.namespace], class: 'hl-type' },
  { tag: [tags.comment, tags.lineComment, tags.blockComment, tags.docComment], class: 'hl-cmt' },
  { tag: [tags.tagName, tags.angleBracket], class: 'hl-tag' },
  { tag: [tags.attributeName, tags.propertyName, tags.labelName], class: 'hl-prop' },
  { tag: [tags.heading, tags.strong], class: 'hl-head' },
  { tag: [tags.link, tags.url], class: 'hl-link' },
  { tag: [tags.meta, tags.processingInstruction], class: 'hl-meta' },
  { tag: tags.invalid, class: 'hl-bad' },
]);

type Loader = () => Promise<Parser>;

const LOADERS: Record<string, Loader> = {
  js: async () => (await import('@codemirror/lang-javascript')).javascriptLanguage.parser.configure({ dialect: 'jsx' }),
  ts: async () => (await import('@codemirror/lang-javascript')).javascriptLanguage.parser.configure({ dialect: 'ts jsx' }),
  py: async () => (await import('@codemirror/lang-python')).pythonLanguage.parser,
  rs: async () => (await import('@codemirror/lang-rust')).rustLanguage.parser,
  css: async () => (await import('@codemirror/lang-css')).cssLanguage.parser,
  html: async () => (await import('@codemirror/lang-html')).htmlLanguage.parser,
  json: async () => (await import('@codemirror/lang-json')).jsonLanguage.parser,
  md: async () => (await import('@codemirror/lang-markdown')).markdownLanguage.parser,
};

const ALIASES: Record<string, string> = {
  javascript: 'js', jsx: 'js', mjs: 'js', cjs: 'js', node: 'js',
  typescript: 'ts', tsx: 'ts', mts: 'ts',
  python: 'py', python3: 'py',
  rust: 'rs',
  scss: 'css', less: 'css',
  xml: 'html', svg: 'html', vue: 'html', htm: 'html',
  jsonc: 'json', json5: 'json',
  markdown: 'md',
  bash: 'sh', shell: 'sh', zsh: 'sh', console: 'sh', powershell: 'sh', ps1: 'sh', pwsh: 'sh', cmd: 'sh', bat: 'sh', dockerfile: 'sh',
};

/** The grammar a fence's language label names, or `''` for none. */
export function languageOf(label: string): string {
  const key = label.trim().toLowerCase();

  return ALIASES[key] ?? (key in LOADERS || key === 'sh' ? key : '');
}

const loaded = new Map<string, Parser>();
const loading = new Map<string, Promise<Parser | null>>();

function load(lang: string): Promise<Parser | null> {
  const known = loading.get(lang);

  if (known !== undefined) {
    return known;
  }

  const loader = LOADERS[lang];
  const next = loader === undefined
    ? Promise.resolve(null)
    : loader().then(
        (parser) => {
          loaded.set(lang, parser);
          return parser;
        },
        () => null,
      );

  loading.set(lang, next);
  return next;
}

/** Splits code into lines of coloured spans with a Lezer grammar. */
export function spansWith(parser: Parser, code: string): Span[][] {
  const lines: Span[][] = [[]];

  highlightCode(
    code,
    parser.parse(code),
    highlighter,
    (text, classes) => {
      lines[lines.length - 1]?.push({ text, cls: classes });
    },
    () => {
      lines.push([]);
    },
  );

  return lines;
}

const SHELL = /(#[^\n]*)|("(?:[^"\\\n]|\\.)*"?|'[^'\n]*'?)|(\$\{?[A-Za-z_][\w]*\}?|\$env:[A-Za-z_]\w*)|(\s--?[A-Za-z][\w-]*)|(&&|\|\||[|;<>])/g;

/** A shell command, coloured by hand: comments, strings, variables, flags and operators. */
export function shellSpans(code: string): Span[][] {
  return code.split('\n').map((line) => {
    const spans: Span[] = [];
    let at = 0;
    /* The first word of a line is the command. */
    const head = /^(\s*)([^\s#|;&<>"']+)/.exec(line);

    if (head !== null && !head[2]?.startsWith('$')) {
      spans.push({ text: head[1] ?? '', cls: '' }, { text: head[2] ?? '', cls: 'hl-fn' });
      at = head[0].length;
    }

    SHELL.lastIndex = at;

    for (let match = SHELL.exec(line); match !== null; match = SHELL.exec(line)) {
      if (match.index > at) {
        spans.push({ text: line.slice(at, match.index), cls: '' });
      }

      const cls = match[1] !== undefined ? 'hl-cmt' : match[2] !== undefined ? 'hl-str' : match[3] !== undefined ? 'hl-num' : match[4] !== undefined ? 'hl-prop' : 'hl-kw';

      spans.push({ text: match[0], cls });
      at = match.index + match[0].length;
    }

    if (at < line.length) {
      spans.push({ text: line.slice(at), cls: '' });
    }

    return spans;
  });
}

/** The coloured lines of a code block, or `null` while its grammar loads (or when it has none). */
export function useHighlight(code: string, label: string): Span[][] | null {
  const lang = languageOf(label);
  const [found, setFound] = useState<{ lang: string; parser: Parser | null }>(() => ({ lang, parser: loaded.get(lang) ?? null }));
  const parser = found.lang === lang ? found.parser : (loaded.get(lang) ?? null);

  useEffect(() => {
    if (lang === '' || lang === 'sh') {
      return;
    }

    let live = true;

    void load(lang).then((next) => {
      if (live) {
        setFound({ lang, parser: next });
      }
    });

    return () => {
      live = false;
    };
  }, [lang]);

  return useMemo(() => {
    if (lang === 'sh') {
      return shellSpans(code);
    }

    if (parser === null || lang === '' || code.length > 200_000) {
      return null;
    }

    try {
      return spansWith(parser, code);
    } catch {
      return null;
    }
  }, [code, lang, parser]);
}
