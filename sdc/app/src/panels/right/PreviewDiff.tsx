import { X } from 'lucide-react';

import { strings } from '../../strings';
import { useFilesStore } from '../../store/files';
import { closeDiff } from '../../store/intents';
import { IconButton } from '../ui/IconButton';

/**
 * The working tree's diff, inside the Preview tab (0.7.9).
 *
 * `git.diff` has been a real method since the schema was written and had no caller in the app; this is what
 * makes "what did the turn change?" answerable without leaving the window. It is the patch as `git` produced
 * it - `+` and `-` lines are coloured by the first character, and nothing is parsed into a diff model, because
 * a hand-rolled parser that disagrees with git about a rename or a binary file is worse than no parser.
 *
 * The header names the branch, so a person can tell a diff of `main` from a diff of a worktree branch, and the
 * × puts the file view (or the empty Preview) back.
 */
export function PreviewDiff() {
  const diff = useFilesStore((state) => state.diff);
  const git = useFilesStore((state) => state.git);

  if (diff === null) {
    return null;
  }

  const lines = diff === '' ? [] : diff.split('\n');

  return (
    <div className="preview-diff flex min-h-0 flex-1 flex-col overflow-hidden" id="previewDiff">
      <div className="preview-diff-head flex items-center gap-[6px] border-b border-border-subtle px-[10px] py-[8px]">
        <div className="min-w-0 flex-1 truncate font-mono text-[11px] text-text-secondary">
          {strings.files.diffTitle(git?.branch ?? 'HEAD')}
        </div>

        <IconButton icon={X} label={strings.files.diffClose} iconSize={14} onClick={() => closeDiff()} />
      </div>

      {lines.length === 0 ? (
        <div className="p-[12px] text-[11.5px] text-text-muted">{strings.files.diffEmpty}</div>
      ) : (
        <pre className="preview-diff-text min-h-0 flex-1 overflow-auto px-[10px] pb-[12px] font-mono text-[11px] leading-[1.5]">
          {lines.map((line, index) => (
            <div
              key={`${index}-${line.slice(0, 12)}`}
              className={
                line.startsWith('+')
                  ? 'text-state-success'
                  : line.startsWith('-')
                    ? 'text-state-error'
                    : line.startsWith('@@')
                      ? 'text-accent'
                      : 'text-text-secondary'
              }
            >
              {line === '' ? ' ' : line}
            </div>
          ))}
        </pre>
      )}
    </div>
  );
}
