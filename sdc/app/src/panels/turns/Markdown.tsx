import { Check, Copy } from 'lucide-react';
import { Fragment, useMemo, useState, type ReactNode } from 'react';

import { openOutside } from '../../lib/external';
import { parseMarkdown, type Block, type Inline } from '../../lib/markdown';
import { strings } from '../../strings';

/**
 * An engine's answer, drawn from Markdown as React elements (v4) - see `lib/markdown.ts` for why it is a
 * tree and never HTML. Code blocks get a language label and a Copy button; links open outside the
 * window; everything takes the window's type scale and tokens.
 */
export function Markdown({ text }: { text: string }) {
  const blocks = useMemo(() => parseMarkdown(text), [text]);

  return (
    <div className="markdown flex flex-col gap-[8px] text-[13px] leading-[1.65] text-text-primary">
      {blocks.map((block, index) => (
        <BlockView key={index} block={block} />
      ))}
    </div>
  );
}

function BlockView({ block }: { block: Block }) {
  switch (block.kind) {
    case 'code':
      return <CodeBlock lang={block.lang} text={block.text} />;
    case 'heading': {
      const size = block.level === 1 ? 'text-[15px]' : block.level === 2 ? 'text-[14px]' : 'text-[13px]';

      return <div className={`mt-[4px] font-semibold text-text-primary ${size}`} role="heading" aria-level={block.level + 2}>{inlines(block.children)}</div>;
    }
    case 'list': {
      const items = block.items.map((item, index) => (
        <li key={index} className="pl-[2px]">
          {inlines(item)}
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
      return <blockquote className="whitespace-pre-wrap border-l-2 border-border-strong pl-[10px] text-text-secondary">{inlines(block.children)}</blockquote>;
    case 'rule':
      return <hr className="border-border-subtle" />;
    default:
      return <p className="whitespace-pre-wrap break-words">{inlines(block.children)}</p>;
  }
}

function inlines(nodes: Inline[]): ReactNode {
  return nodes.map((node, index) => <Fragment key={index}>{inline(node)}</Fragment>);
}

function inline(node: Inline): ReactNode {
  switch (node.kind) {
    case 'code':
      return <code className="rounded-xs border border-border-subtle bg-bg-input px-[4px] py-[1px] font-mono text-[11.5px] text-text-primary">{node.text}</code>;
    case 'strong':
      return <strong className="font-semibold">{inlines(node.children)}</strong>;
    case 'em':
      return <em>{inlines(node.children)}</em>;
    case 'link':
      return (
        <a
          href={node.href}
          target="_blank"
          rel="noreferrer noopener"
          className="text-accent underline decoration-accent/40 underline-offset-2 hover:decoration-accent"
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

function CodeBlock({ lang, text }: { lang: string; text: string }) {
  const [copied, setCopied] = useState(false);

  const copy = (): void => {
    void navigator.clipboard
      .writeText(text)
      .then(() => {
        setCopied(true);
        window.setTimeout(() => setCopied(false), 1500);
      })
      .catch(() => setCopied(false));
  };

  return (
    <div className="code-block overflow-hidden rounded-md border border-border-subtle bg-bg-input">
      <div className="flex items-center gap-[8px] border-b border-border-subtle px-[10px] py-[4px] font-mono text-[10px] text-text-muted">
        <span>{lang === '' ? strings.turns.answer.code : lang}</span>
        <button
          type="button"
          className="ml-auto flex items-center gap-[4px] rounded-sm px-[5px] py-[1px] text-text-muted hover:bg-bg-hover hover:text-text-primary"
          aria-label={strings.turns.answer.copy}
          onClick={copy}
        >
          {copied ? <Check size={10} aria-hidden="true" /> : <Copy size={10} aria-hidden="true" />}
          {copied ? strings.turns.answer.copied : strings.turns.answer.copy}
        </button>
      </div>
      <pre className="overflow-x-auto px-[10px] py-[8px] font-mono text-[11.5px] leading-[1.6] text-text-primary">
        <code>{text}</code>
      </pre>
    </div>
  );
}
