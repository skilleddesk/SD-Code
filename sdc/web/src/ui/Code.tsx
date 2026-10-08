// A read-only, syntax-highlighted view of a file. CodeMirror 6, loaded only when a file is opened, and each language
// only when a file of that language is. (Editing is a later phase and goes through the daemon's Trust Kernel.)

import { HighlightStyle, syntaxHighlighting, type LanguageSupport } from '@codemirror/language';
import { EditorState, type Extension } from '@codemirror/state';
import { EditorView, lineNumbers } from '@codemirror/view';
import { tags } from '@lezer/highlight';
import { useEffect, useRef } from 'react';

const highlight = HighlightStyle.define([
  { tag: [tags.keyword, tags.operatorKeyword, tags.modifier], color: 'var(--purple)' },
  { tag: [tags.string, tags.special(tags.string)], color: 'var(--green)' },
  { tag: [tags.number, tags.bool, tags.null], color: 'var(--orange)' },
  { tag: [tags.comment, tags.lineComment, tags.blockComment], color: 'var(--text-muted)', fontStyle: 'italic' },
  { tag: [tags.function(tags.variableName), tags.definition(tags.variableName), tags.propertyName], color: 'var(--accent)' },
  { tag: [tags.typeName, tags.className, tags.tagName], color: 'var(--accent-hover)' },
  { tag: [tags.heading], color: 'var(--accent)', fontWeight: '600' },
  { tag: [tags.invalid], color: 'var(--red-bright)' },
]);

const theme = EditorView.theme(
  {
    '&': { backgroundColor: 'var(--bg-input)', color: 'var(--text-primary)', fontSize: '13px', borderRadius: '8px' },
    '.cm-content': { fontFamily: 'var(--font-mono)', padding: '8px 0' },
    '.cm-gutters': { backgroundColor: 'var(--bg-raised)', color: 'var(--text-muted)', border: 'none', borderRadius: '8px 0 0 8px' },
    '&.cm-focused': { outline: '2px solid var(--border-focus)' },
    '.cm-scroller': { overflow: 'auto', maxHeight: '70vh' },
  },
  { dark: true },
);

/** The language for a file name, or none. Each branch is its own chunk. */
export async function languageFor(name: string): Promise<LanguageSupport | null> {
  const extension = name.toLowerCase().split('.').pop() ?? '';

  switch (extension) {
    case 'js':
    case 'mjs':
    case 'cjs':
    case 'jsx':
    case 'ts':
    case 'tsx':
      return (await import('@codemirror/lang-javascript')).javascript({ jsx: extension.endsWith('x'), typescript: extension.startsWith('ts') });
    case 'json':
      return (await import('@codemirror/lang-json')).json();
    case 'css':
      return (await import('@codemirror/lang-css')).css();
    case 'html':
    case 'htm':
      return (await import('@codemirror/lang-html')).html();
    case 'md':
    case 'markdown':
      return (await import('@codemirror/lang-markdown')).markdown();
    case 'py':
      return (await import('@codemirror/lang-python')).python();
    case 'php':
      return (await import('@codemirror/lang-php')).php();
    case 'sql':
      return (await import('@codemirror/lang-sql')).sql();
    case 'rs':
      return (await import('@codemirror/lang-rust')).rust();
    case 'xml':
    case 'svg':
      return (await import('@codemirror/lang-xml')).xml();
    default:
      return null;
  }
}

export default function Code({ text, name }: { text: string; name: string }) {
  const host = useRef<HTMLDivElement>(null);
  const view = useRef<EditorView | null>(null);
  const latest = useRef(text);

  latest.current = text;

  useEffect(() => {
    let alive = true;

    void languageFor(name).then((language) => {
      if (!alive || !host.current) return;

      const extensions: Extension[] = [
        lineNumbers(),
        syntaxHighlighting(highlight),
        theme,
        EditorState.readOnly.of(true),
        EditorView.editable.of(false),
        EditorView.lineWrapping,
        EditorView.contentAttributes.of({ 'aria-label': name, 'aria-readonly': 'true', tabindex: '0' }),
        ...(language ? [language] : []),
      ];

      view.current = new EditorView({ parent: host.current, state: EditorState.create({ doc: latest.current, extensions }) });
    });

    return () => {
      alive = false;
      view.current?.destroy();
      view.current = null;
    };
    // The view is made once per file; text that arrives later is appended below.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [name]);

  useEffect(() => {
    const current = view.current;

    if (current && current.state.doc.toString() !== text) {
      current.dispatch({ changes: { from: 0, to: current.state.doc.length, insert: text } });
    }
  }, [text]);

  return <div ref={host} className="code-view" />;
}
