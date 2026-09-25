import { LoaderCircle, RotateCcw, Save, X } from 'lucide-react';
import { lazy, Suspense, useState } from 'react';

import { strings } from '../../strings';
import { useFilesStore } from '../../store/files';
import { closeFile, saveFile } from '../../store/intents';
import { BTN_PRIMARY, BTN_SECONDARY, BTN_SM } from '../ui/button';
import { IconButton } from '../ui/IconButton';

/* The editor is loaded the first time a file is opened, not with the window (see `CodeEditor`). */
const CodeEditor = lazy(() => import('./CodeEditor'));

/**
 * The open files (v4): a tab per file, and an editor for the one in front.
 *
 * The file view used to be one file at a time, read-only until `Edit` turned the text into a textarea.
 * Now every open file has a tab, the front one is a real editor (highlighting, search with Ctrl+F,
 * undo, bracket matching), and the tab says when it has unsaved changes. `Save` - or Ctrl+S in the
 * editor - writes through `fs.write`, which takes a checkpoint first, here or on the host (P5).
 *
 * A file the daemon cut at a megabyte opens read-only: saving the first megabyte of a larger file would
 * cut the file itself.
 */
export function PreviewFile() {
  const file = useFilesStore((state) => state.open);
  const tabs = useFilesStore((state) => state.tabs);
  const drafts = useFilesStore((state) => state.drafts);
  const reveal = useFilesStore((state) => state.reveal);
  const [busy, setBusy] = useState(false);
  /* A dirty tab asks once before it closes: the first click arms, the second discards. */
  const [armed, setArmed] = useState<string | null>(null);

  if (file === null) {
    return null;
  }

  const draft = drafts[file.path];
  const shown = draft ?? file.text;
  const changed = draft !== undefined && draft !== file.text;
  const lines = shown === '' ? 0 : shown.split('\n').length;

  const save = (): void => {
    if (!changed || busy || file.truncated) {
      return;
    }

    setBusy(true);
    void saveFile(file.path, shown).finally(() => setBusy(false));
  };

  const close = (path: string): void => {
    if (drafts[path] !== undefined && armed !== path) {
      setArmed(path);

      return;
    }

    setArmed(null);

    if (path === file.path) {
      closeFile();
    } else {
      useFilesStore.getState().closeTab(path);
    }
  };

  return (
    <div
      className="preview-file flex min-h-0 flex-1 flex-col overflow-hidden"
      id="previewFile"
      data-file-path={file.path}
      data-file-dirty={changed ? 'true' : 'false'}
    >
      <div className="file-tabs flex shrink-0 overflow-x-auto border-b border-border-subtle bg-bg-raised" role="tablist" aria-label={strings.files.tabs}>
        {tabs.map((tab) => {
          const active = tab.path === file.path;
          const dirty = drafts[tab.path] !== undefined;

          return (
            <div
              key={tab.path}
              className={
                'file-tab group flex max-w-[180px] shrink-0 items-center gap-[6px] border-r border-border-subtle pl-[10px] pr-[4px] text-[11.5px] ' +
                (active ? 'bg-bg-base text-text-primary shadow-[inset_0_-2px_0_var(--accent)]' : 'text-text-muted hover:bg-bg-hover hover:text-text-secondary')
              }
              title={tab.path}
            >
              <button
                type="button"
                role="tab"
                aria-selected={active}
                className="min-w-0 truncate py-[7px] text-left"
                onClick={() => useFilesStore.getState().activate(tab.path)}
              >
                {tab.name}
              </button>
              <button
                type="button"
                className={
                  'grid h-[18px] w-[18px] shrink-0 place-items-center rounded-sm ' +
                  (armed === tab.path ? 'bg-red-subtle text-state-error' : 'text-text-muted hover:bg-bg-active hover:text-text-primary')
                }
                aria-label={armed === tab.path ? strings.files.discard(tab.name) : strings.files.closeTab(tab.name)}
                title={armed === tab.path ? strings.files.discard(tab.name) : strings.files.closeTab(tab.name)}
                onClick={() => close(tab.path)}
                onBlur={() => setArmed((current) => (current === tab.path ? null : current))}
              >
                {dirty && armed !== tab.path ? (
                  <span className="h-[7px] w-[7px] rounded-full bg-state-waiting group-hover:hidden" aria-hidden="true" />
                ) : null}
                <X size={11} aria-hidden="true" className={dirty && armed !== tab.path ? 'hidden group-hover:block' : ''} />
              </button>
            </div>
          );
        })}
      </div>

      <div className="preview-file-head flex items-center gap-[6px] border-b border-border-subtle px-[10px] py-[6px]">
        <div className="min-w-0 flex-1 truncate font-mono text-[10.5px] text-text-muted" title={file.path}>
          {file.path}
        </div>

        {changed ? (
          <button
            type="button"
            className={BTN_SM + ' ' + BTN_SECONDARY}
            title={strings.files.revert}
            onClick={() => useFilesStore.getState().setDraft(file.path, null)}
          >
            <RotateCcw size={11} aria-hidden="true" />
            {strings.files.cancel}
          </button>
        ) : null}

        {file.truncated ? null : (
          <button
            type="button"
            id="previewFileSave"
            className={BTN_SM + ' ' + BTN_PRIMARY}
            disabled={busy || !changed}
            title={strings.files.saveHint}
            onClick={save}
          >
            {busy ? <LoaderCircle size={11} className="animate-spin" aria-hidden="true" /> : <Save size={11} aria-hidden="true" />}
            {busy ? strings.files.saving : strings.files.save}
          </button>
        )}

        <IconButton icon={X} label={strings.files.close} iconSize={14} onClick={() => close(file.path)} />
      </div>

      <div className="preview-file-meta flex items-center gap-[8px] border-b border-border-subtle px-[10px] py-[4px] font-mono text-[10.5px] text-text-muted">
        <span>{strings.files.fileMeta(file.bytes, lines)}</span>
        {file.truncated ? <span className="text-state-waiting">{strings.files.truncated(file.bytes)}</span> : null}
        {changed ? <span className="text-state-waiting">{strings.files.unsaved}</span> : null}
        <span className="ml-auto shrink-0 opacity-70" title={file.sha256}>
          {file.sha256.slice(0, 8)}
        </span>
      </div>

      <Suspense
        fallback={
          <pre className="min-h-0 flex-1 overflow-auto whitespace-pre px-[10px] py-[8px] font-mono text-[11.5px] leading-[1.6] text-text-secondary">
            {shown}
          </pre>
        }
      >
        <CodeEditor
          path={file.path}
          value={shown}
          readOnly={file.truncated}
          line={reveal !== null && reveal.path === file.path ? reveal.line : null}
          onChange={(text) => useFilesStore.getState().setDraft(file.path, text === file.text ? null : text)}
          onSave={save}
        />
      </Suspense>
    </div>
  );
}
