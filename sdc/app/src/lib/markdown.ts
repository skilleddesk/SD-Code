/**
 * A small Markdown reader for the engines' answers (v4) - into a tree, never into HTML.
 *
 * Models answer in Markdown, and the answer block used to print it raw: fences, `**` and backticks on
 * screen. This turns the subset models actually write into a tree the answer block renders as React
 * elements, so nothing a model writes is ever interpreted as HTML (a `<script>` in an answer is text).
 *
 * Blocks: fenced code (```lang), headings (#, ##, ###), bullet and numbered lists, block quotes,
 * horizontal rules, tables (0.21), paragraphs. Inline: `code`, **bold**, *italic* / _italic_, [text](http…) links.
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
  | { kind: 'table'; align: Align[]; head: Inline[][]; rows: Inline[][][] }
  | { kind: 'paragraph'; children: Inline[] };

export type Align = 'left' | 'center' | 'right' | '';

const FENCE = /^\s*(```|~~~)\s*([\w+#.-]*)\s*$/;
const HEADING = /^(#{1,6})\s+(.*)$/;
const BULLET = /^\s*[-*+]\s+(.*)$/;
const NUMBERED = /^\s*(\d{1,4})[.)]\s+(.*)$/;
const RULE = /^\s*([-*_])(\s*\1){2,}\s*$/;
const QUOTE = /^\s*>\s?(.*)$/;
/** The line under a table's header: `| --- | :-: | --: |`. */
const TABLE_RULE = /^\s*\|?\s*:?-{2,}:?\s*(\|\s*:?-{2,}:?\s*)*\|?\s*$/;

/** The cells of one table row: the outer pipes are optional, an escaped `\|` stays in its cell. */
export function cells(line: string): string[] {
  const body = line.trim().replace(/^\|/, '').replace(/(?<!\\)\|$/, '');

  return body.split(/(?<!\\)\|/).map((cell) => cell.trim().replace(/\\\|/g, '|'));
}

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

    /* A table: a header row with a pipe, then the rule under it (0.21). */
    const under = lines[index + 1] ?? '';

    if (line.includes('|') && under.includes('|') && under.includes('-') && TABLE_RULE.test(under)) {
      flush();

      const head = cells(line);
      const align: Align[] = cells(under).map((cell) =>
        cell.startsWith(':') && cell.endsWith(':') ? 'center' : cell.endsWith(':') ? 'right' : cell.startsWith(':') ? 'left' : '',
      );
      const rows: Inline[][][] = [];

      index += 2;

      while (index < lines.length && (lines[index] ?? '').includes('|') && (lines[index] ?? '').trim() !== '') {
        rows.push(cells(lines[index] ?? '').map(parseInline));
        index += 1;
      }

      index -= 1;
      blocks.push({ kind: 'table', align, head: head.map(parseInline), rows });
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

/** Splits Markdown at blank lines that are not inside a fenced code block. */
export function paragraphs(text: string): string[] {
  const chunks: string[] = [];
  let current: string[] = [];
  let fenced = false;

  for (const line of text.split('\n')) {
    if (/^\s*(```|~~~)/.test(line)) {
      fenced = !fenced;
    }

    if (!fenced && line.trim() === '' && current.length > 0) {
      chunks.push(current.join('\n'));
      current = [];
      continue;
    }

    if (line.trim() !== '' || current.length > 0) {
      current.push(line);
    }
  }

  if (current.length > 0) {
    chunks.push(current.join('\n'));
  }

  return chunks;
}

/**
 * Closes what a streaming answer has opened but not yet closed (0.21), so its last line reads as it will
 * once it is done instead of flashing raw markers: `**bold so fa` is drawn bold and an inline code span
 * still being typed is drawn as code. Only the last line is looked at, and nothing is added inside an open
 * fence - the fence already draws as code to the end.
 */
export function closeOpenMarks(text: string): string {
  const fences = text.match(/^\s*(```|~~~)/gm)?.length ?? 0;

  if (fences % 2 === 1) {
    return text;
  }

  const line = text.slice(text.lastIndexOf('\n') + 1);
  const openTick = (line.match(/`/g)?.length ?? 0) % 2 === 1;
  /* `**` inside a code span is not a mark. */
  const outside = (openTick ? line.slice(0, line.lastIndexOf('`')) : line).replace(/`[^`]*`/g, '');
  const openStrong = (outside.match(/\*\*/g)?.length ?? 0) % 2 === 1 && !/\*\*\s*$/.test(outside);
  let tail = '';

  if (openTick && !/`\s*$/.test(line)) {
    tail += '`';
  }

  if (openStrong) {
    tail += '**';
  }

  return text + tail;
}
