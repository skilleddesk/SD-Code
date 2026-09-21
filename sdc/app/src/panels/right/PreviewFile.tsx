import { X } from 'lucide-react';
import { useEffect, useState } from 'react';

import { strings } from '../../strings';
import { useFilesStore } from '../../store/files';
import { closeFile, saveFile } from '../../store/intents';
import { BTN_PRIMARY, BTN_SECONDARY, BTN_SM } from '../ui/button';
import { IconButton } from '../ui/IconButton';

/**
 * The file the tree opened, inside the Preview tab (0.7.7), and the two gestures 0.7.9 added: **Edit** and
 * **Save**.
 *
 * A header with the file's name and its whole path, one meta line (`2.4 KB · 128 lines`, the `sha256`'s
 * first eight characters - the same hash a checkpoint stores, so "is this the version the engine wrote?" is
 * answerable), and the text itself in a `<pre>`.
 *
 * Editing is deliberately small: a `<textarea>` holding the file's text and a Save button, and **the daemon
 * takes the checkpoint** (`fs.write` with the chat's id - principle P5). There is no syntax highlighting, no
 * multi-file tab set and no auto-save: each of those is a feature with its own questions (which file is
 * "current" when two are open, what a half-typed line means), and none is needed to make Save honest. Cancel
 * puts the read text back.
 *
 * Three honest details in the read view, each of which could have been a lie:
 *
 *   * **the whole path is in the header** and again in the row's `title`. A tree shows a name; a person
 *     editing `src/routes/login.tsx` needs to know *which* `login.tsx` this is.
 *   * **`truncated` is shown, not swallowed** - and such a file **cannot be edited**, because saving the
 *     visible megabyte over the whole file would silently delete the rest of it.
 *   * **no syntax highlighting.** There is no highlighter in this build, and a hand-rolled approximation
 *     would colour the wrong tokens: the text is what the file says, in the app's mono type.
 */
export function PreviewFile() {
  const file = useFilesStore((state) => state.open);
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState('');
  const [busy, setBusy] = useState(false);

  /* A different file ends the edit: a draft belongs to the file it was typed into. */
  useEffect(() => {
    setEditing(false);
    setBusy(false);
  }, [file?.path]);

  if (file === null) {
    return null;
  }

  const shown = editing ? draft : file.text;
  const lines = shown === '' ? 0 : shown.split('\n').length;
  const changed = editing && draft !== file.text;

  const save = (): void => {
    setBusy(true);
    void saveFile(file.path, draft).then((ok) => {
      setBusy(false);

      if (ok) {
        setEditing(false);
      }
    });
  };

  return (
    <div
      className="preview-file flex min-h-0 flex-1 flex-col overflow-hidden"
      id="previewFile"
      data-file-path={file.path}
      data-file-editing={editing ? 'true' : 'false'}
    >
      <div className="preview-file-head flex items-center gap-[6px] border-b border-border-subtle px-[10px] py-[8px]">
        <div className="min-w-0 flex-1">
          <div className="truncate text-[12px] font-medium text-text-primary" title={file.path}>
            {file.name}
          </div>
          <div className="truncate font-mono text-[10.5px] text-text-muted" title={file.path}>
            {file.path}
          </div>
        </div>

        {file.truncated ? null : editing ? (
          <>
            <button
              type="button"
              id="previewFileSave"
              className={BTN_SM + ' ' + BTN_PRIMARY}
              disabled={busy || !changed}
              onClick={save}
            >
              {busy ? strings.files.saving : strings.files.save}
            </button>
            <button
              type="button"
              id="previewFileCancel"
              className={BTN_SM + ' ' + BTN_SECONDARY}
              onClick={() => {
                setDraft(file.text);
                setEditing(false);
              }}
            >
              {strings.files.cancel}
            </button>
          </>
        ) : (
          <button
            type="button"
            id="previewFileEdit"
            className={BTN_SM + ' ' + BTN_SECONDARY}
            onClick={() => {
              setDraft(file.text);
              setEditing(true);
            }}
          >
            {strings.files.edit}
          </button>
        )}

        <IconButton
          icon={X}
          label={strings.files.close}
          iconSize={14}
          onClick={() => {
            setEditing(false);
            closeFile();
          }}
        />
      </div>

      <div className="preview-file-meta flex items-center gap-[8px] px-[10px] py-[6px] font-mono text-[10.5px] text-text-muted">
        <span>{strings.files.fileMeta(file.bytes, lines)}</span>

        {file.truncated ? (
          <span className="text-state-waiting">{strings.files.truncated(file.bytes)}</span>
        ) : null}

        {changed ? <span className="text-state-waiting">{strings.files.unsaved}</span> : null}

        <span className="ml-auto shrink-0 opacity-70" title={file.sha256}>
          {file.sha256.slice(0, 8)}
        </span>
      </div>

      {editing ? (
        <textarea
          id="previewFileDraft"
          className="min-h-0 flex-1 resize-none overflow-auto border-0 bg-bg-input px-[10px] pb-[12px] font-mono text-[11.5px] leading-[1.55] text-text-primary outline-none"
          value={draft}
          spellCheck={false}
          onChange={(event) => setDraft(event.target.value)}
        />
      ) : (
        <pre className="preview-file-text min-h-0 flex-1 overflow-auto whitespace-pre-wrap break-words px-[10px] pb-[12px] font-mono text-[11.5px] leading-[1.55] text-text-secondary">
          {file.text}
        </pre>
      )}
    </div>
  );
}

