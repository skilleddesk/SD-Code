import { FileDiff } from 'lucide-react';

import { strings } from '../../strings';
import { changedFiles } from './grouping';
import type { TimelineItem } from './types';

/**
 * The turn's change at a glance, under its answer (0.13) - what Codex calls the turn diff: which files,
 * how much, before the numbers of the footer. Absent when the turn changed nothing.
 */
export function ChangedFiles({ items }: { items: readonly TimelineItem[] }) {
  const files = changedFiles(items);

  if (files.length === 0) {
    return null;
  }

  const added = files.reduce((sum, file) => sum + file.added, 0);
  const removed = files.reduce((sum, file) => sum + file.removed, 0);

  return (
    <div className="changed-files mb-[8px] mt-[4px] rounded-lg border border-border-subtle bg-bg-raised px-[13px] py-[9px] shadow-sm" data-changed-files={files.length}>
      <div className="flex items-center gap-[8px] text-[11.5px] text-text-secondary">
        <FileDiff size={13} aria-hidden="true" className="text-text-muted" />
        <span className="font-medium text-text-primary">{strings.turns.changed(files.length)}</span>
        <span className="font-mono text-[11px] text-diff-addText">+{added}</span>
        <span className="font-mono text-[11px] text-diff-removeText">−{removed}</span>
      </div>
      <div className="mt-[4px] flex flex-col gap-[1px]">
        {files.map((file) => (
          <div key={file.path} className="flex items-center gap-[8px] font-mono text-[11px]">
            <span className="min-w-0 flex-1 truncate text-text-secondary" title={file.path}>
              {file.path}
            </span>
            <span className="shrink-0 text-diff-addText">+{file.added}</span>
            <span className="shrink-0 text-diff-removeText">−{file.removed}</span>
          </div>
        ))}
      </div>
    </div>
  );
}
