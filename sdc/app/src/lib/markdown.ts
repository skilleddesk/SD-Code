/**
 * A small Markdown reader for the engines' answers (v4) - into a tree, never into HTML.
 *
 * Models answer in Markdown, and the answer block used to print it raw: fences, `**` and backticks on
 * screen. This turns the subset models actually write into a tree the answer block renders as React
 * elements, so nothing a model writes is ever interpreted as HTML (a `<script>` in an answer is text).
 *
 * Blocks: fenced code (```lang), headings (#, ##, ###), bullet and numbered lists, block quotes,
 * horizontal rules, paragraphs. Inline: `code`, **bold**, *italic* / _italic_, [text](http…) links.
 * An unclosed fence - an answer still streaming - is a code block to the end, so a half-written block
 * does not flicker between prose and code.
 */

export type Inline =
  | { kind: 'text'; text: string }
  | { kind: 'code'; text: string }
  | { kind: 'strong'; children: Inline[] }
  | { kind: 'em'; children: Inline[] }
  | { kind: 'link'; href: string; children: Inline[] };

export type Block =
  | { kind: 'code'; lang: string; text: string }
  | { kind: 'heading'; level: 1 | 2 | 3; children: Inline[] }
  | { kind: 'list'; ordered: boolean; start: number; items: Inline[][] }
  | { kind: 'quote'; children: Inline[] }
  | { kind: 'rule' }
  | { kind: 'paragraph'; children: Inline[] };

const FENCE = /^\s*(```|~~~)\s*([\w+#.-]*)\s*$/;
const HEADING = /^(#{1,6})\s+(.*)$/;
const BULLET = /^\s*[-*+]\s+(.*)$/;
const NUMBERED = /^\s*(\d{1,4})[.)]\s+(.*)$/;
const RULE = /^\s*([-*_])(\s*\1){2,}\s*$/;
const QUOTE = /^\s*>\s?(.*)$/;

export function parseMarkdown(source: string): Block[] {
  const lines = source.replace(/\r\n?/g, '\n').split('\n');
  const blocks: Block[] = [];
  let paragraph: string[] = [];

  const flush = (): void => {
    if (paragraph.length > 0) {
      blocks.push({ kind: 'paragraph', children: parseInline(paragraph.join('\n')) });
      paragraph = [];
    }
  };

  for (let index = 0; index < lines.length; index += 1) {
    const line = lines[index] ?? '';
    const fence = FENCE.exec(line);

    if (fence !== null) {
      flush();

      const marker = fence[1] ?? '```';
      const body: string[] = [];

      index += 1;

      while (index < lines.length && !(lines[index] ?? '').trim().startsWith(marker)) {
        body.push(lines[index] ?? '');
        index += 1;
      }

      blocks.push({ kind: 'code', lang: fence[2] ?? '', text: body.join('\n') });
      continue;
    }

    if (line.trim() === '') {
      flush();
      continue;
    }

    const heading = HEADING.exec(line);

    if (heading !== null) {
      flush();

      const level = Math.min(3, (heading[1] ?? '#').length) as 1 | 2 | 3;

      blocks.push({ kind: 'heading', level, children: parseInline(heading[2] ?? '') });
      continue;
    }

    if (RULE.test(line)) {
      flush();
      blocks.push({ kind: 'rule' });
      continue;
    }

    const bullet = BULLET.exec(line);
    const numbered = NUMBERED.exec(line);

    if (bullet !== null || numbered !== null) {
      flush();

      const ordered = numbered !== null && bullet === null;
      const items: Inline[][] = [];
      const start = ordered ? Number(numbered?.[1] ?? 1) : 1;

      while (index < lines.length) {
        const current = lines[index] ?? '';
        const match = ordered ? NUMBERED.exec(current) : BULLET.exec(current);

        if (match === null) {
          /* An indented line under an item continues it. */
          if (items.length > 0 && /^\s{2,}\S/.test(current)) {
            const last = items[items.length - 1] ?? [];

            items[items.length - 1] = [...last, { kind: 'text', text: ' ' }, ...parseInline(current.trim())];
            index += 1;
            continue;
          }

          break;
        }

        items.push(parseInline((ordered ? match[2] : match[1]) ?? ''));
        index += 1;
      }

      index -= 1;
      blocks.push({ kind: 'list', ordered, start, items });
      continue;
    }

    const quote = QUOTE.exec(line);

    if (quote !== null) {
      flush();

      const body: string[] = [];

      while (index < lines.length) {
        const match = QUOTE.exec(lines[index] ?? '');

        if (match === null) {
          break;
        }

        body.push(match[1] ?? '');
        index += 1;
      }

      index -= 1;
      blocks.push({ kind: 'quote', children: parseInline(body.join('\n')) });
      continue;
    }

    paragraph.push(line);
  }

  flush();

  return blocks;
}

/** Inline marks, left to right; an unmatched marker is plain text. */
export function parseInline(text: string): Inline[] {
  const out: Inline[] = [];
  let buffer = '';
  let index = 0;

  const push = (node: Inline): void => {
    if (buffer !== '') {
      out.push({ kind: 'text', text: buffer });
      buffer = '';
    }

    out.push(node);
  };

  while (index < text.length) {
    const rest = text.slice(index);

    if (rest.startsWith('`')) {
      const end = text.indexOf('`', index + 1);

      if (end > index) {
        push({ kind: 'code', text: text.slice(index + 1, end) });
        index = end + 1;
        continue;
      }
    }

    if (rest.startsWith('**') || rest.startsWith('__')) {
      const marker = rest.slice(0, 2);
      const end = text.indexOf(marker, index + 2);

      if (end > index + 2) {
        push({ kind: 'strong', children: parseInline(text.slice(index + 2, end)) });
        index = end + 2;
        continue;
      }
    }

    if ((rest.startsWith('*') || rest.startsWith('_')) && !/^[*_]\s/.test(rest)) {
      const marker = rest[0] ?? '*';
      const end = text.indexOf(marker, index + 1);
      /* `snake_case_names` are not emphasis: an underscore inside a word stays an underscore. */
      const inWord = marker === '_' && /\w/.test(text[index - 1] ?? '');

      if (end > index + 1 && !inWord && text[end - 1] !== ' ') {
        push({ kind: 'em', children: parseInline(text.slice(index + 1, end)) });
        index = end + 1;
        continue;
      }
    }

    if (rest.startsWith('[')) {
      const link = /^\[([^\]]+)\]\((https?:\/\/[^)\s]+)\)/.exec(rest);

      if (link !== null) {
        push({ kind: 'link', href: link[2] ?? '', children: parseInline(link[1] ?? '') });
        index += link[0].length;
        continue;
      }
    }

    buffer += text[index] ?? '';
    index += 1;
  }

  if (buffer !== '') {
    out.push({ kind: 'text', text: buffer });
  }

  return out;
}
