import { indentWithTab } from '@codemirror/commands';
import { css } from '@codemirror/lang-css';
import { html } from '@codemirror/lang-html';
import { javascript } from '@codemirror/lang-javascript';
import { json } from '@codemirror/lang-json';
import { markdown } from '@codemirror/lang-markdown';
import { python } from '@codemirror/lang-python';
import { rust } from '@codemirror/lang-rust';
import { HighlightStyle, syntaxHighlighting, type LanguageSupport } from '@codemirror/language';
import { Compartment, EditorSelection, EditorState, type Extension } from '@codemirror/state';
import { EditorView, keymap } from '@codemirror/view';
import { tags } from '@lezer/highlight';
import { basicSetup } from 'codemirror';
import { useEffect, useRef } from 'react';

/**
 * The code editor (v4, docs/ROADMAP-v4.md decision 4) - CodeMirror 6, not Monaco.
 *
 * Monaco is VS Code's editor: about five megabytes, and the owner's brief was "not like VS Code". SDC
 * needs a file to be read, highlighted, searched and edited - not an IDE's language services - and
 * CodeMirror 6 is exactly that, in the parts this file imports. It is loaded lazily (`PreviewFile`
 * imports it with `React.lazy`), so a window that never opens a file never downloads it.
 *
 * Every colour is a design token (`var(--…)`), so the editor follows the light and dark themes with the
 * rest of the window instead of carrying a palette of its own.
 */
export interface CodeEditorProps {
  /** Identifies the document: a new path is a new editor state (its own undo history). */
  path: string;
  value: string;
  readOnly: boolean;
  onChange: (text: string) => void;
  onSave: () => void;
  /** A line to put the cursor on and scroll to (1-based) - a review issue, a search hit. */
  line?: number | null;
}

/** The language for a file name, by extension. Unknown files are plain text, not a guess. */
function languageFor(path: string): LanguageSupport | null {
  const extension = path.split('.').pop()?.toLowerCase() ?? '';

  switch (extension) {
    case 'ts':
    case 'mts':
    case 'cts':
      return javascript({ typescript: true });
    case 'tsx':
      return javascript({ typescript: true, jsx: true });
    case 'js':
    case 'mjs':
    case 'cjs':
      return javascript();
    case 'jsx':
      return javascript({ jsx: true });
    case 'py':
      return python();
    case 'rs':
      return rust();
    case 'html':
    case 'htm':
    case 'vue':
    case 'svelte':
      return html();
    case 'css':
    case 'scss':
    case 'less':
      return css();
    case 'json':
    case 'jsonc':
      return json();
    case 'md':
    case 'markdown':
      return markdown();
    default:
      return null;
  }
}

const theme = EditorView.theme({
  '&': {
    height: '100%',
    fontSize: '12px',
    color: 'var(--text-primary)',
    backgroundColor: 'var(--bg-input)',
  },
  '.cm-scroller': { fontFamily: 'var(--font-mono)', lineHeight: '1.6' },
  '.cm-content': { caretColor: 'var(--accent)', padding: '6px 0' },
  '.cm-cursor, .cm-dropCursor': { borderLeftColor: 'var(--accent)' },
  '&.cm-focused': { outline: 'none' },
  '&.cm-focused .cm-selectionBackground, .cm-selectionBackground, ::selection': {
    backgroundColor: 'var(--accent-subtle) !important',
  },
  '.cm-gutters': {
    backgroundColor: 'var(--bg-raised)',
    color: 'var(--text-faint)',
    border: 'none',
    borderRight: '1px solid var(--border-subtle)',
  },
  '.cm-activeLineGutter': { backgroundColor: 'var(--bg-hover)', color: 'var(--text-secondary)' },
  '.cm-activeLine': { backgroundColor: 'color-mix(in srgb, var(--bg-hover) 55%, transparent)' },
  '.cm-matchingBracket': { backgroundColor: 'var(--accent-subtle)', outline: '1px solid var(--border-focus)' },
  '.cm-searchMatch': { backgroundColor: 'var(--orange-subtle)', outline: '1px solid var(--orange)' },
  '.cm-searchMatch.cm-searchMatch-selected': { backgroundColor: 'var(--accent-subtle)' },
  '.cm-panels': { backgroundColor: 'var(--bg-raised)', color: 'var(--text-primary)', borderColor: 'var(--border-subtle)' },
  '.cm-panels input, .cm-panels button': { fontFamily: 'var(--font-ui)', fontSize: '11.5px' },
  '.cm-textfield': {
    backgroundColor: 'var(--bg-input)',
    border: '1px solid var(--border-default)',
    borderRadius: '5px',
    color: 'var(--text-primary)',
  },
  '.cm-button': {
    backgroundImage: 'none',
    backgroundColor: 'var(--bg-overlay)',
    border: '1px solid var(--border-default)',
    borderRadius: '5px',
    color: 'var(--text-secondary)',
  },
  '.cm-tooltip': { backgroundColor: 'var(--bg-overlay)', border: '1px solid var(--border-default)', color: 'var(--text-primary)' },
  '.cm-tooltip-autocomplete > ul > li[aria-selected]': { backgroundColor: 'var(--accent-subtle)', color: 'var(--text-primary)' },
  '.cm-foldPlaceholder': { backgroundColor: 'var(--bg-hover)', border: 'none', color: 'var(--text-muted)' },
});

/* Syntax colours from the palette's semantic tokens, so both themes read well without a second table. */
const highlight = HighlightStyle.define([
  { tag: [tags.keyword, tags.modifier, tags.controlKeyword, tags.operatorKeyword], color: 'var(--purple)' },
  { tag: [tags.string, tags.special(tags.string), tags.regexp], color: 'var(--green)' },
  { tag: [tags.number, tags.bool, tags.null, tags.atom], color: 'var(--orange)' },
  { tag: [tags.function(tags.variableName), tags.function(tags.propertyName)], color: 'var(--accent)' },
  { tag: [tags.typeName, tags.className, tags.namespace], color: 'var(--accent-hover)' },
  { tag: [tags.comment, tags.lineComment, tags.blockComment], color: 'var(--text-muted)', fontStyle: 'italic' },
  { tag: [tags.tagName, tags.angleBracket], color: 'var(--red)' },
  { tag: [tags.attributeName, tags.propertyName], color: 'var(--text-secondary)' },
  { tag: [tags.heading], color: 'var(--text-primary)', fontWeight: '600' },
  { tag: [tags.link, tags.url], color: 'var(--accent)', textDecoration: 'underline' },
  { tag: tags.invalid, color: 'var(--red)' },
]);

export default function CodeEditor({ path, value, readOnly, onChange, onSave, line = null }: CodeEditorProps) {
  const host = useRef<HTMLDivElement | null>(null);
  const view = useRef<EditorView | null>(null);
  const handlers = useRef({ onChange, onSave });
  const editable = useRef(new Compartment());

  handlers.current = { onChange, onSave };

  /* One editor per document: a different path starts a fresh state, with its own undo history. */
  useEffect(() => {
    const parent = host.current;

    if (parent === null) {
      return;
    }

    const language = languageFor(path);
    const extensions: Extension[] = [
      basicSetup,
      keymap.of([
        indentWithTab,
        {
          key: 'Mod-s',
          preventDefault: true,
          run: () => {
            handlers.current.onSave();

            return true;
          },
        },
      ]),
      theme,
      syntaxHighlighting(highlight),
      editable.current.of([EditorState.readOnly.of(readOnly), EditorView.editable.of(!readOnly)]),
      EditorView.updateListener.of((update) => {
        if (update.docChanged) {
          handlers.current.onChange(update.state.doc.toString());
        }
      }),
      EditorView.contentAttributes.of({ 'aria-label': path }),
    ];

    if (language !== null) {
      extensions.push(language);
    }

    const created = new EditorView({ parent, state: EditorState.create({ doc: value, extensions }) });

    view.current = created;

    return () => {
      created.destroy();
      view.current = null;
    };
    /* The document's identity is its path; `value` and `readOnly` are synced by the effects below. */
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [path]);

  /* The text changed from outside (a re-read after an agent wrote the file): take it in. */
  useEffect(() => {
    const current = view.current;

    if (current !== null && current.state.doc.toString() !== value) {
      current.dispatch({ changes: { from: 0, to: current.state.doc.length, insert: value } });
    }
  }, [value]);

  useEffect(() => {
    view.current?.dispatch({
      effects: editable.current.reconfigure([EditorState.readOnly.of(readOnly), EditorView.editable.of(!readOnly)]),
    });
  }, [readOnly]);

  /* Go to a line: the cursor on it, and the line in the middle of the view. */
  useEffect(() => {
    const current = view.current;

    if (current === null || line === null || line < 1) {
      return;
    }

    const target = current.state.doc.line(Math.min(line, current.state.doc.lines));

    current.dispatch({
      selection: EditorSelection.cursor(target.from),
      effects: EditorView.scrollIntoView(target.from, { y: 'center' }),
    });
    current.focus();
  }, [line, path]);

  return <div ref={host} className="code-editor min-h-0 flex-1 overflow-hidden" data-path={path} />;
}
