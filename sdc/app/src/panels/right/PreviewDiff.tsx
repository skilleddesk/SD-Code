import { MessageSquare, Undo2, X } from 'lucide-react';

import { hunkDraft, parseDiff } from '../../lib/diffReview';
import { strings } from '../../strings';
import { useModelStore } from '../../store/model';
import { useFilesStore } from '../../store/files';
import { closeDiff } from '../../store/intents';
import { IconButton } from '../ui/IconButton';

function tone(line: string): string {
  return line.startsWith('+') ? 'text-state-success' : line.startsWith('-') ? 'text-state-error' : line.startsWith('@@') ? 'text-accent' : 'text-text-secondary';
}

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
  const files = parseDiff(diff);

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
        <div className="preview-diff-text min-h-0 flex-1 overflow-auto px-[10px] pb-[12px] font-mono text-[11px] leading-[1.5]">
          {files.length === 0 ? (
            <pre>{diff}</pre>
          ) : (
            files.map((file) => (
              <section key={file.path} className="mb-[10px]" aria-label={file.path}>
                <div className="sticky top-0 truncate border-b border-border-subtle bg-bg-raised py-[4px] text-text-primary" title={file.path}>
                  {file.path}
                </div>

                {file.hunks.length === 0 ? <div className="py-[4px] text-text-muted">{strings.review.noHunks}</div> : null}

                {file.hunks.map((hunk, index) => (
                  <div key={`${index}-${hunk.header}`} className="diff-hunk mt-[6px] rounded-sm border border-border-subtle">
                    <div className="flex items-center gap-[6px] bg-bg-raised px-[6px] py-[3px] text-text-muted">
                      <span className="min-w-0 flex-1 truncate">{hunk.header}</span>
                      <span>{strings.review.hunkOf(hunk.added, hunk.removed)}</span>
                      <IconButton
                        icon={MessageSquare}
                        label={strings.review.ask}
                        iconSize={12}
                        onClick={() => useModelStore.getState().setDraft(hunkDraft(file.path, hunk, strings.review.askDefault))}
                      />
                      <IconButton
                        icon={Undo2}
                        label={strings.review.undo}
                        iconSize={12}
                        onClick={() => useModelStore.getState().setDraft(hunkDraft(file.path, hunk, strings.review.undoDefault))}
                      />
                    </div>
                    <pre className="overflow-x-auto px-[6px] py-[4px]">
                      {hunk.text.split('\n').slice(1).map((line, row) => (
                        <div key={`${row}-${line.slice(0, 12)}`} className={tone(line)}>
                          {line === '' ? ' ' : line}
                        </div>
                      ))}
                    </pre>
                  </div>
                ))}
              </section>
            ))
          )}
        </div>
      )}
    </div>
  );
}
