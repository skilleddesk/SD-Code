import { X } from 'lucide-react';

import { strings } from '../../strings';
import { useFilesStore } from '../../store/files';
import { closeFile } from '../../store/intents';
import { IconButton } from '../ui/IconButton';

/**
 * The file the tree opened, inside the Preview tab (0.7.7).
 *
 * A header with the file's name and its whole path, one meta line (`2.4 KB · 128 lines`, the `sha256`'s
 * first eight characters - the same hash a checkpoint stores, so "is this the version the engine wrote?"
 * is answerable), and the text itself in a `<pre>`.
 *
 * Three honest details, each of which could have been a lie:
 *
 *   * **the whole path is in the header** and again in the row's `title`. A tree shows a name; a person
 *     editing `src/routes/login.tsx` needs to know *which* `login.tsx` this is.
 *   * **`truncated` is shown, not swallowed.** The daemon caps a read at a megabyte, so a large file says
 *     `First 1 MB of 12.4 MB` rather than looking complete.
 *   * **no syntax highlighting.** There is no highlighter in this build, and a hand-rolled approximation
 *     would colour the wrong tokens - the text is what the file says, in the app's mono type.
 */
export function PreviewFile() {
  const file = useFilesStore((state) => state.open);

  if (file === null) {
    return null;
  }

  const lines = file.text === '' ? 0 : file.text.split('\n').length;

  return (
    <div
      className="preview-file flex min-h-0 flex-1 flex-col overflow-hidden"
      id="previewFile"
      data-file-path={file.path}
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

        <IconButton
          icon={X}
          label={strings.files.close}
          iconSize={14}
          onClick={() => closeFile()}
        />
      </div>

      <div className="preview-file-meta flex items-center gap-[8px] px-[10px] py-[6px] font-mono text-[10.5px] text-text-muted">
        <span>{strings.files.fileMeta(file.bytes, lines)}</span>

        {file.truncated ? (
          <span className="text-state-waiting">{strings.files.truncated(file.bytes)}</span>
        ) : null}

        <span className="ml-auto shrink-0 opacity-70" title={file.sha256}>
          {file.sha256.slice(0, 8)}
        </span>
      </div>

      <pre className="preview-file-text min-h-0 flex-1 overflow-auto whitespace-pre-wrap break-words px-[10px] pb-[12px] font-mono text-[11.5px] leading-[1.55] text-text-secondary">
        {file.text}
      </pre>
    </div>
  );
}
