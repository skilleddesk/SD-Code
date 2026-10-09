import { CircleCheckBig, Download, LoaderCircle, RotateCw } from 'lucide-react';

import { strings } from '../../strings';
import { toast } from '../../store/toast';
import { fraction, sizeOf, useJob, type JobStart } from '../../store/tools';
import { BTN_PRIMARY, BTN_SECONDARY, BTN_SM } from './button';

/**
 * One button that installs something on this computer (0.21) - a tool, or an Ollama model - and turns
 * into its own progress bar while the daemon works. What used to be a command to copy into a terminal.
 */
export function InstallButton({
  begin,
  label,
  size,
  onDone,
  compact = false,
}: {
  begin: JobStart;
  /** What is being installed, for the button and the toast. */
  label: string;
  /** Roughly what the download weighs, said before the press. */
  size?: string;
  onDone?: () => void;
  compact?: boolean;
}) {
  const { job, error, start } = useJob(begin, () => {
    toast(strings.tools.installed(label));
    onDone?.();
  });

  if (job?.state === 'running') {
    const part = fraction(job);

    return (
      <div className={'install-progress flex min-w-0 flex-col gap-[4px] ' + (compact ? 'w-[180px]' : 'w-full')} role="status" data-install-state="running">
        <div className="flex items-center gap-[6px] text-[11px] text-text-secondary">
          <LoaderCircle size={11} className="shrink-0 animate-spin text-accent motion-reduce:animate-none" aria-hidden="true" />
          <span className="min-w-0 truncate">{job.step}</span>
          {part === null ? null : <span className="ml-auto shrink-0 font-mono tabular-nums text-text-muted">{Math.round(part * 100)}%</span>}
        </div>
        <div className="h-[4px] w-full overflow-hidden rounded-full bg-bg-overlay">
          <div
            className={'h-full rounded-full transition-[width] duration-300 ' + (part === null ? 'install-indeterminate w-1/3' : '')}
            style={{ background: 'var(--grad-brand)', width: part === null ? undefined : `${Math.max(2, part * 100)}%` }}
          />
        </div>
        {job.total !== null && job.total > 0 ? (
          <span className="font-mono text-[10px] tabular-nums text-text-muted">
            {sizeOf(job.done)} / {sizeOf(job.total)}
          </span>
        ) : null}
      </div>
    );
  }

  if (job?.state === 'done') {
    return (
      <span className="inline-flex items-center gap-[5px] text-[11px] font-medium text-state-success" data-install-state="done">
        <CircleCheckBig size={12} aria-hidden="true" />
        {strings.tools.done}
      </span>
    );
  }

  const failed = job?.state === 'failed' ? (job.error ?? job.step) : error;

  return (
    <div className="flex min-w-0 flex-col items-start gap-[4px]" data-install-state={failed === null ? 'idle' : 'failed'}>
      <button type="button" className={BTN_SM + ' ' + (failed === null ? BTN_PRIMARY : BTN_SECONDARY)} onClick={start} data-action="install">
        {failed === null ? <Download size={11} aria-hidden="true" /> : <RotateCw size={11} aria-hidden="true" />}
        {failed === null ? strings.tools.install(label) : strings.tools.retry}
        {failed === null && size !== undefined && size !== 'system' ? <span className="font-normal opacity-75">· {size}</span> : null}
      </button>
      {failed === null ? null : <span className="max-w-[340px] text-[10.5px] leading-[1.45] text-state-error">{failed}</span>}
    </div>
  );
}
