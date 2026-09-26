import { ChevronRight, FilePen, FileText, Loader, Play } from 'lucide-react';
import { useEffect, useState } from 'react';

import { strings } from '../../strings';
import type { ToolCardData, ToolStatus } from './types';

/**
 * `.tool-card` - one thing the engine did (spec section 7.5).
 *
 * Three variants, one shell. All three are a header row - a 20x20 icon chip, the tool's name in
 * mono, the target path, a status pill, and either a chevron or a spinner - and only two of them
 * have a body:
 *
 *   Read   file-text   no body at all; `done · 42 ln` is the whole report
 *   Edit   file-pen    the diff, collapsed, opened by the header
 *   Run    play        the output, open by default, and a spinner while it is still going
 *
 * Those defaults are the prototype's, and they are the right ones: a diff is long and it waits to be
 * asked for, while the output of a command you just ran is the reason you ran it.
 *
 * The card collapses through its header only - unlike the thinking block, whose whole surface
 * toggles - because the Run body is its own scrollable window and a click inside it must not fold
 * the card up. `aria-expanded` is set only where there is something to expand.
 */

/** The pill's three tones: `done` is green, `running` blue, `failed` red. */
const STATUS_CLASS: Record<ToolStatus, string> = {
  done: 'bg-green-subtle text-state-success',
  running: 'bg-accent-subtle text-accent',
  failed: 'bg-red-subtle text-state-error',
};

/**
 * The three output tones. The first word is the prototype's class (`.ok`, `.fail`, `.dim`), kept so
 * a DOM diff against design/ui-prototype.html finds the same names; the utilities after it draw it.
 */
const RUN_LINE_CLASS: Record<'ok' | 'fail' | 'dim', string> = {
  ok: 'ok text-state-success',
  fail: 'fail text-state-error',
  dim: 'dim text-text-muted',
};

/** The word an output line is prefixed with - `PASS` / `FAIL` / nothing for a dim line. */
function runLineLabel(level: 'ok' | 'fail' | 'dim'): string {
  if (level === 'ok') {
    return strings.turns.tools.runPass;
  }

  if (level === 'fail') {
    return strings.turns.tools.runFail;
  }

  return '';
}

export interface ToolCardProps {
  tool: ToolCardData;
}

/**
 * `3.2s` next to a running pill (0.9.0): how long the call has been going, against the log's own
 * stamp. Its own component so only running cards pay for a timer, and it unmounts with the pill.
 */
function RunningFor({ since }: { since: string }) {
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 500);

    return () => window.clearInterval(timer);
  }, []);

  const started = Date.parse(since);

  if (!Number.isFinite(started)) {
    return null;
  }

  return <span data-running-for>{` · ${Math.max(0, (now - started) / 1000).toFixed(1)}s`}</span>;
}

export function ToolCard({ tool }: ToolCardProps) {
  /* A run is open by default; a diff waits to be asked for. */
  const [open, setOpen] = useState(tool.kind === 'run');

  const Icon = tool.kind === 'read' ? FileText : tool.kind === 'edit' ? FilePen : Play;
  const hasBody = tool.kind !== 'read';
  const spinning = tool.status === 'running';

  return (
    <div
      className={
        'tool-card mb-[6px] overflow-hidden rounded-md border border-border-subtle bg-bg-raised ' +
        'transition-all duration-fast ease-ease hover:border-border-default ' +
        (spinning ? 'running border-[rgba(91,156,255,.35)] shadow-[0_0_0_1px_rgba(91,156,255,.06)]' : '')
      }
    >
      <div
        className="tool-head flex cursor-pointer select-none items-center gap-[10px] px-[13px] py-[9px] text-[12px]"
        role="button"
        tabIndex={0}
        aria-expanded={hasBody ? open : undefined}
        onClick={() => {
          if (hasBody) {
            setOpen((current) => !current);
          }
        }}
        onKeyDown={(event) => {
          if (hasBody && event.key === 'Enter') {
            setOpen((current) => !current);
          }
        }}
      >
        <div
          className={
            'tool-icon grid h-[20px] w-[20px] shrink-0 place-items-center rounded-sm border ' +
            'border-border-subtle bg-bg-overlay ' +
            (spinning && tool.kind === 'run' ? 'text-accent' : 'text-text-secondary')
          }
        >
          <Icon size={12} aria-hidden="true" />
        </div>

        <span className="tool-name shrink-0 font-mono text-[11.5px] font-semibold text-text-primary">
          {tool.name}
        </span>

        <span className="tool-target min-w-0 flex-1 overflow-hidden text-ellipsis whitespace-nowrap font-mono text-[11.5px] text-text-secondary">
          {tool.target}
        </span>

        <span
          className={
            'tool-status ' +
            tool.status +
            ' shrink-0 rounded-sm px-[7px] py-[2px] font-mono text-[10px] font-medium ' +
            STATUS_CLASS[tool.status]
          }
        >
          {tool.meta}
          {spinning ? <RunningFor since={tool.startedAt} /> : null}
        </span>

        {spinning ? (
          <Loader size={12} className="animate-spin text-accent" aria-hidden="true" />
        ) : (
          <ChevronRight
            size={12}
            aria-hidden="true"
            className={
              'text-text-muted transition-transform duration-200 ease-ease ' +
              (open ? 'rotate-90' : '')
            }
          />
        )}
      </div>

      {hasBody && open ? <ToolBody tool={tool} /> : null}
    </div>
  );
}

/**
 * The body: a diff for an Edit, an output window for a Run.
 *
 * Both are the same box - `--font-mono`, 12px at 1.7, a 220px ceiling and its own scrollbar - which
 * is why the padding differs rather than the shape: a diff's rows carry their own 13px inset so the
 * add/remove tint can run the full width of the card, and an output line is plain text.
 */
function ToolBody({ tool }: { tool: Exclude<ToolCardData, { kind: 'read' }> }) {
  if (tool.kind === 'edit') {
    return (
      <div className="tool-body max-h-[220px] overflow-y-auto border-t border-border-subtle font-mono text-[12px] leading-[1.7] text-text-secondary">
        {tool.diff.map((line, index) => (
          <div
            key={`${line.lineNumber}-${line.change}-${index}`}
            className={
              'diff-line flex px-[13px] font-mono text-[12px] leading-[1.65] ' +
              (line.change === 'add'
                ? 'add bg-diff-addBg text-diff-addText'
                : 'rem bg-diff-removeBg text-diff-removeText')
            }
          >
            <span className="ln min-w-[30px] shrink-0 select-none pr-[12px] text-right text-text-muted">
              {line.lineNumber}
            </span>
            <span>{line.text}</span>
          </div>
        ))}
      </div>
    );
  }

  return (
    <div className="tool-body max-h-[220px] overflow-y-auto border-t border-border-subtle px-[13px] py-[10px] font-mono text-[12px] leading-[1.7] text-text-secondary">
      {tool.output.map((line, index) => (
        <div key={`${line.level}-${index}`}>
          <span className={RUN_LINE_CLASS[line.level]}>{runLineLabel(line.level)}</span>
          {line.level === 'dim' ? '' : ' '}
          {line.text}
        </div>
      ))}
    </div>
  );
}
