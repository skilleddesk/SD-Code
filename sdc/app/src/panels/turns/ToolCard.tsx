import { Bot, ChevronRight, FilePen, FileText, Loader, MessageCircleQuestion, Play } from 'lucide-react';
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
  /* A run is open by default, and so is a short diff (0.13, as Claude Code shows an edit inline); a long
     diff waits to be asked for. A sub-agent's card is open while it works. */
  const [open, setOpen] = useState(
    tool.kind === 'run' || (tool.kind === 'edit' && tool.diff.length > 0 && tool.diff.length <= 16) || (tool.kind === 'read' && tool.name === 'Agent'),
  );

  /* A short diff opens when it arrives - the card was drawn before the edit finished - unless the person
     already opened or closed it themselves. */
  const [touched, setTouched] = useState(false);
  const smallDiff = tool.kind === 'edit' && tool.diff.length > 0 && tool.diff.length <= 16;

  useEffect(() => {
    if (smallDiff && !touched) {
      setOpen(true);
    }
  }, [smallDiff, touched]);

  if (tool.kind === 'read' && tool.name === 'Question') {
    return <QuestionAsked tool={tool} />;
  }

  const Icon = tool.kind === 'read' ? (tool.name === 'Agent' ? Bot : FileText) : tool.kind === 'edit' ? FilePen : Play;
  /* A body only when there is something in it (0.14.1): an empty box under every Claude Bash card read as
     a broken stream. */
  const hasBody = tool.kind === 'edit' ? tool.diff.length > 0 : (tool.output?.length ?? 0) > 0;
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
            setTouched(true);
            setOpen((current) => !current);
          }
        }}
        onKeyDown={(event) => {
          if (hasBody && event.key === 'Enter') {
            setTouched(true);
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
 * The agent's question, as it stays in the turn (0.13): the whole question and the person's answer - the
 * card above the input that asked it has gone, and this is the record of what was decided.
 */
function QuestionAsked({ tool }: { tool: Extract<ToolCardData, { kind: 'read' }> }) {
  const answer = tool.output?.find((line) => line.level === 'ok')?.text;

  return (
    <div className="question-asked mb-[6px] rounded-md border border-border-subtle border-l-[3px] border-l-accent bg-bg-raised px-[13px] py-[9px]" data-question>
      <div className="flex items-center gap-[8px] font-mono text-[10.5px] uppercase tracking-wide text-accent">
        <MessageCircleQuestion size={12} aria-hidden="true" />
        {strings.agent.question.label}
        {tool.status === 'running' ? <Loader size={11} className="animate-spin" aria-hidden="true" /> : null}
      </div>
      <p className="mt-[4px] whitespace-pre-wrap text-[12.5px] leading-[1.55] text-text-primary">{tool.target}</p>
      {answer === undefined ? null : (
        <p className="mt-[5px] text-[12.5px] text-text-secondary">
          <span className="font-semibold text-accent">→ </span>
          {answer}
        </p>
      )}
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
function ToolBody({ tool }: { tool: ToolCardData }) {
  if (tool.kind === 'read') {
    return (
      <div className="tool-body max-h-[220px] overflow-y-auto border-t border-border-subtle px-[13px] py-[8px] font-mono text-[11.5px] leading-[1.65] text-text-muted">
        {(tool.output ?? []).map((line, index) => (
          <div key={`${index}-${line.text}`} className="truncate" title={line.text}>
            {line.text}
          </div>
        ))}
      </div>
    );
  }

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
            <span className="whitespace-pre">{line.text}</span>
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
