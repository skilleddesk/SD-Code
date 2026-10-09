import { Check, Copy } from 'lucide-react';
import { Fragment, memo, useMemo, useState, type ReactNode } from 'react';

import { copyText, openOutside } from '../../lib/external';
import { useHighlight } from '../../lib/highlight';
import { closeOpenMarks, paragraphs, parseMarkdown, type Align, type Block, type Inline } from '../../lib/markdown';
import { strings } from '../../strings';

/**
 * An engine's answer, drawn from Markdown as React elements (v4) - see `lib/markdown.ts` for why it is a
 * tree and never HTML. Code blocks get a language label, colours (0.21) and a Copy button; links open
 * outside the window; everything takes the window's type scale and tokens.
 *
 * `streaming` (0.21): the text is still arriving. Marks the model has opened but not closed yet are
 * closed for the drawing (`closeOpenMarks`), and the caret sits at the very end of the last word - inside
 * the paragraph, list item or code line being written - the way a person typing would see it.
 */
export function Markdown({ text, streaming = false }: { text: string; streaming?: boolean }) {
  const chunks = useMemo(() => paragraphs(streaming ? closeOpenMarks(text) : text), [text, streaming]);

  return (
    <div className="markdown flex flex-col gap-[9px] text-[13.5px] leading-[1.7] text-text-primary">
      {chunks.map((chunk, index) => (
        <Chunk key={index} source={chunk} caret={streaming && index === chunks.length - 1} />
      ))}
      {streaming && chunks.length === 0 ? (
        <div>
          <Caret />
        </div>
      ) : null}
    </div>
  );
}

/** The streaming caret: a soft brand-coloured bar that breathes rather than blinks. */
export function Caret() {
  return <span className="stream-caret" aria-hidden="true" />;
}

/**
 * One paragraph's worth of Markdown, parsed and drawn once (0.11.8). While an answer streams only its
 * last paragraph changes, so only that one is parsed again - a long answer no longer re-parses and
 * re-draws from the top on every frame of the reveal.
 */
const Chunk = memo(function Chunk({ source, caret }: { source: string; caret: boolean }) {
  const blocks = useMemo(() => parseMarkdown(source), [source]);

  return (
    <>
      {blocks.map((block, index) => (
        <BlockView key={index} block={block} caret={caret && index === blocks.length - 1} />
      ))}
    </>
  );
});

function BlockView({ block, caret }: { block: Block; caret: boolean }) {
  const tail = caret ? <Caret /> : null;

  switch (block.kind) {
    case 'code':
      return <CodeBlock lang={block.lang} text={block.text} caret={caret} />;
    case 'heading': {
      const size = block.level === 1 ? 'text-[17px] tracking-[-0.02em]' : block.level === 2 ? 'text-[15px] tracking-[-0.015em]' : 'text-[13.5px]';

      return (
        <div className={`mt-[4px] font-semibold text-text-primary ${size}`} role="heading" aria-level={block.level + 2}>
          {inlines(block.children)}
          {tail}
        </div>
      );
    }
    case 'list': {
      const items = block.items.map((item, index) => (
        <li key={index} className="pl-[2px]">
          {inlines(item)}
          {index === block.items.length - 1 ? tail : null}
        </li>
      ));

      return block.ordered ? (
        <ol className="flex list-decimal flex-col gap-[3px] pl-[20px] marker:text-text-muted" start={block.start}>
          {items}
        </ol>
      ) : (
        <ul className="flex list-disc flex-col gap-[3px] pl-[18px] marker:text-text-muted">{items}</ul>
      );
    }
    case 'quote':
      return (
        <blockquote className="whitespace-pre-wrap border-l-2 border-border-strong pl-[10px] text-text-secondary">
          {inlines(block.children)}
          {tail}
        </blockquote>
      );
    case 'rule':
      return <hr className="border-border-subtle" />;
    case 'table':
      return (
        <>
          <TableView block={block} />
          {tail}
        </>
      );
    default:
      return (
        <p className="whitespace-pre-wrap break-words">
          {inlines(block.children)}
          {tail}
        </p>
      );
  }
}

const ALIGN: Record<Align, string> = { left: 'text-left', center: 'text-center', right: 'text-right', '': 'text-left' };

function TableView({ block }: { block: Extract<Block, { kind: 'table' }> }) {
  return (
    <div className="md-table overflow-x-auto rounded-lg border border-border-subtle">
      <table className="w-full border-collapse text-[12.5px] leading-[1.55]">
        <thead>
          <tr className="bg-bg-raised">
            {block.head.map((cell, index) => (
              <th key={index} className={`border-b border-border-subtle px-[10px] py-[6px] font-semibold text-text-primary ${ALIGN[block.align[index] ?? '']}`}>
                {inlines(cell)}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {block.rows.map((row, rowIndex) => (
            <tr key={rowIndex} className="border-b border-border-subtle last:border-b-0 hover:bg-bg-hover">
              {block.head.map((_, index) => (
                <td key={index} className={`px-[10px] py-[5px] align-top text-text-secondary ${ALIGN[block.align[index] ?? '']}`}>
                  {inlines(row[index] ?? [])}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function inlines(nodes: Inline[]): ReactNode {
  return nodes.map((node, index) => <Fragment key={index}>{inline(node)}</Fragment>);
}

function inline(node: Inline): ReactNode {
  switch (node.kind) {
    case 'code':
      return <code className="rounded-[5px] border border-border-subtle bg-bg-active px-[5px] py-[1px] font-mono text-[12px] text-accent-hover">{node.text}</code>;
    case 'strong':
      return <strong className="font-semibold">{inlines(node.children)}</strong>;
    case 'em':
      return <em>{inlines(node.children)}</em>;
    case 'link':
      return (
        <a
          href={node.href}
          rel="noreferrer noopener"
          className="text-accent underline decoration-accent-glow underline-offset-2 hover:decoration-accent"
          title={node.href}
          onClick={(event) => {
            /* The window is not a browser: a link opens in the person's own. */
            event.preventDefault();
            void openOutside(node.href);
          }}
        >
          {inlines(node.children)}
        </a>
      );
    default:
      return node.text;
  }
}

function CodeBlock({ lang, text, caret }: { lang: string; text: string; caret: boolean }) {
  const [copied, setCopied] = useState(false);
  const lines = useHighlight(text, lang);

  const copy = (): void => {
    void copyText(text).then((ok) => {
      setCopied(ok);

      if (ok) {
        window.setTimeout(() => setCopied(false), 1500);
      }
    });
  };

  return (
    <div className="code-block overflow-hidden rounded-lg border border-border-subtle bg-bg-input shadow-sm" data-lang={lang}>
      <div className="flex items-center gap-[8px] border-b border-border-subtle bg-bg-raised px-[12px] py-[5px] font-mono text-[10.5px] text-text-muted">
        <span className="flex items-center gap-[6px]">
          <span className="h-[6px] w-[6px] rounded-full bg-accent opacity-70" aria-hidden="true" />
          {lang === '' ? strings.turns.answer.code : lang}
        </span>
        <button
          type="button"
          className="ml-auto flex items-center gap-[4px] rounded-sm px-[6px] py-[1px] text-text-muted hover:bg-bg-hover hover:text-text-primary"
          aria-label={strings.turns.answer.copy}
          onClick={copy}
        >
          {copied ? <Check size={11} aria-hidden="true" /> : <Copy size={11} aria-hidden="true" />}
          {copied ? strings.turns.answer.copied : strings.turns.answer.copy}
        </button>
      </div>
      <pre className="overflow-x-auto px-[14px] py-[10px] font-mono text-[12px] leading-[1.65] text-text-primary">
        <code>
          {lines === null
            ? text
            : lines.map((line, index) => (
                <Fragment key={index}>
                  {index > 0 ? '\n' : null}
                  {line.map((span, at) => (span.cls === '' ? <Fragment key={at}>{span.text}</Fragment> : <span key={at} className={span.cls}>{span.text}</span>))}
                </Fragment>
              ))}
          {caret ? <Caret /> : null}
        </code>
      </pre>
    </div>
  );
}
