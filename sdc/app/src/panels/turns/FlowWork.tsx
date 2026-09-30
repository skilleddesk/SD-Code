import { ChevronRight, Copy, FileDiff } from 'lucide-react';
import { useState } from 'react';

import { strings } from '../../strings';
import { toast } from '../../store/toast';
import { AnswerBlock } from './AnswerBlock';
import { DraftCard } from './DraftCard';
import { look } from './draftLook';
import { changeTotals, matches, ribbon, steps, tail, transcript, type Filter, type Step } from './flow';
import { Sparkline, StatusMark, StepBody, StepIcon, TimeRibbon } from './StepParts';
import { duration, hasBody, reveal, useNow, usePace } from './stepKit';
import { Narration } from './Narration';
import type { Turn } from './types';
import { copyText } from '../../lib/external';

const FILTERS: Filter[] = ['all', 'think', 'read', 'edit', 'run', 'say'];

/**
 * **Flow** (0.15): the turn's work as a timed rail - what the stream is now.
 *
 * What the others do: Claude Code prints each tool as a line and a spinner verb; Codex folds reads into
 * "Explored"; Cursor and ChatGPT fold everything behind "Thought for 12s"; Devin keeps a progress list.
 * Flow keeps all of that and adds what none of them show:
 *
 *   * **when** - every step carries its own measured duration, and a ribbon above the rail shows the whole
 *     turn's time by kind (thinking, reading, editing, commands, writing - and the grey *deciding* between
 *     steps, the model choosing and the network, which is often most of a slow turn). A click on a stretch
 *     takes you to its step;
 *   * **now** - the bottom node always says what is happening this second: a thought's newest lines, the
 *     command's output, the file being written, or the pause before the next step, with its clock;
 *   * **how fast** - while it runs, the head carries the output pace over the last forty seconds;
 *   * **just the commands** - on a long run, filters with counts keep one kind of step, and "Copy log"
 *     puts the whole run on the clipboard as text.
 *
 * A second style, Console, was built beside it for the person to compare. Flow was chosen, and the best of
 * Console - the pace line, the filters and the log - moved in here.
 *
 * Finished, the rail folds to one line - `Worked for 1m 12s · 14 steps · 3 files +40 −2` and the ribbon -
 * and the answer stands on its own under it.
 */
export function FlowWork({ turn, sessionId }: { turn: Turn; sessionId: string }) {
  const now = useNow(250, turn.running);
  const list = steps(turn.timeline, now, turn.endedAt, turn.running);
  const work = list.filter((step) => step.kind !== 'answer');
  const answers = list.filter((step) => step.kind === 'answer');
  const { segments, totals, total } = ribbon(list, turn.startedAt, now, turn.endedAt, turn.running);
  const change = changeTotals(turn.timeline);
  const pace = usePace(turn.stats?.chars ?? 0, turn.running);
  /* `null` = follow the turn (open while it runs, folded after); a boolean = the person chose. */
  const [chosen, setChosen] = useState<boolean | null>(null);
  const [filter, setFilter] = useState<Filter>('all');
  const open = chosen ?? turn.running;
  const pick = (key: string): void => {
    setChosen(true);
    setFilter('all');
    window.setTimeout(() => reveal(`step-${turn.id}-${key}`), 30);
  };

  const answerBlocks = answers.map((step) =>
    step.drawn.kind === 'text' ? <AnswerBlock key={step.key} answer={{ text: step.drawn.text, streaming: false }} /> : null,
  );

  if (work.length === 0 && turn.draft === undefined && !turn.running) {
    return <>{answerBlocks}</>;
  }

  const shown = work.filter((step) => matches(step, filter));
  const tools = work.filter((step) => step.drawn.kind === 'tool' || step.drawn.kind === 'explore').length;
  const copy = (): void => {
    void copyText(`${turn.user.body}\n\n${transcript(list, turn.startedAt)}`).then((ok) => {
      if (ok) {
        toast(strings.turns.flow.copied);
      }
    });
  };

  return (
    <div className="flow-work mb-[8px]" data-flow>
      <div className="flow-head mb-[8px] flex w-full flex-col gap-[6px] rounded-md border border-border-subtle bg-bg-raised px-[12px] py-[8px] transition-colors duration-fast hover:border-border-default">
        <div className="flex w-full min-w-0 items-center gap-[8px] text-[12px]">
          <button
            type="button"
            className="flex min-w-0 flex-1 items-center gap-[8px] text-left"
            aria-expanded={open}
            title={open ? strings.turns.flow.hide : strings.turns.flow.show}
            onClick={() => setChosen(!open)}
          >
            <ChevronRight size={12} aria-hidden="true" className={'shrink-0 text-text-muted transition-transform duration-200 ' + (open ? 'rotate-90' : '')} />
            <span className={'min-w-0 truncate font-medium ' + (turn.running ? 'shimmer-text' : 'text-text-primary')}>
              {turn.running ? strings.turns.flow.working(work.length) : strings.turns.flow.worked(strings.turns.thinking.seconds(total), work.length)}
            </span>
            {change.files > 0 ? (
              <span className="inline-flex shrink-0 items-center gap-[4px] font-mono text-[10.5px] text-text-muted">
                <FileDiff size={11} aria-hidden="true" />
                {change.files}
                <span className="text-diff-addText">+{change.added}</span>
                <span className="text-diff-removeText">−{change.removed}</span>
              </span>
            ) : null}
          </button>
          {turn.running ? (
            <span className="hidden shrink-0 items-center gap-[5px] font-mono text-[10.5px] tabular-nums text-text-muted sm:inline-flex" title={strings.turns.flow.pace}>
              <Sparkline samples={pace} width={56} height={14} />
              {strings.turns.stats.pace(pace[pace.length - 1] ?? 0)}
            </span>
          ) : null}
          {tools > 0 ? <span className="hidden shrink-0 font-mono text-[10.5px] text-text-muted md:inline">{strings.turns.stats.tools(tools)}</span> : null}
          <span className="shrink-0 font-mono text-[10.5px] tabular-nums text-text-muted">{strings.turns.thinking.seconds(total)}</span>
          {open ? (
            <button
              type="button"
              className="grid h-[20px] w-[20px] shrink-0 place-items-center rounded-sm text-text-muted hover:bg-bg-hover hover:text-text-primary"
              title={strings.turns.flow.copy}
              aria-label={strings.turns.flow.copy}
              onClick={copy}
            >
              <Copy size={11} aria-hidden="true" />
            </button>
          ) : null}
        </div>
        <TimeRibbon segments={segments} totals={totals} total={total} running={turn.running} onPick={pick} compact={!open} />
      </div>

      {/* A long run gets filters: "what did it run?" in one click instead of a scroll. */}
      {open && work.length >= 6 ? (
        <div className="mb-[6px] ml-[2px] flex flex-wrap items-center gap-[2px]" role="group" aria-label={strings.turns.flow.filter}>
          {FILTERS.map((key) => {
            const count = work.filter((step) => matches(step, key)).length;

            return count === 0 && key !== 'all' ? null : (
              <button
                key={key}
                type="button"
                aria-pressed={filter === key}
                className={
                  'rounded-full px-[8px] py-[1px] text-[10.5px] transition-colors ' +
                  (filter === key ? 'bg-accent-subtle font-medium text-accent' : 'text-text-muted hover:bg-bg-hover hover:text-text-secondary')
                }
                onClick={() => setFilter(key)}
              >
                {strings.turns.flow.filters[key]}
                <span className="ml-[4px] font-mono text-[9.5px] opacity-70">{count}</span>
              </button>
            );
          })}
        </div>
      ) : null}

      {open ? (
        <ol className="flow-rail relative ml-[9px] border-l border-border-default pl-[18px]">
          {shown.map((step, index) => (
            <FlowStep key={step.key} step={step} turnId={turn.id} sessionId={sessionId} now={now} newest={turn.running && index === shown.length - 1 && turn.draft === undefined} />
          ))}

          {turn.draft === undefined || filter !== 'all' ? null : (
            <li className="relative mb-[8px]">
              <Node running kind={look(turn.draft.name).verb === strings.turns.draft.run ? 'run' : 'edit'} />
              <DraftCard draft={turn.draft} />
            </li>
          )}

          {turn.running && turn.live !== undefined && filter === 'all' ? <NowNode turn={turn} now={now} /> : null}
        </ol>
      ) : null}

      {answerBlocks}
    </div>
  );
}

/** The dot on the rail: the step's icon in a ring, pulsing while it runs. */
function Node({ kind, running = false, failed = false }: { kind: Step['kind']; running?: boolean; failed?: boolean }) {
  return (
    <span
      className={
        'absolute left-[-29px] top-[3px] grid h-[21px] w-[21px] place-items-center rounded-full border bg-bg-base ' +
        (failed ? 'border-state-error' : running ? 'node-live border-accent' : 'border-border-default')
      }
      aria-hidden="true"
    >
      <StepIcon kind={kind} size={11} />
    </span>
  );
}

function FlowStep({ step, turnId, sessionId, now, newest }: { step: Step; turnId: string; sessionId: string; now: number; newest: boolean }) {
  const running = step.status === 'running';
  const autoOpen = running && (step.kind === 'run' || step.kind === 'agent');
  const [chosen, setChosen] = useState<boolean | null>(null);
  const open = chosen ?? autoOpen;
  const body = hasBody(step);
  const took = duration(step, now);
  const id = `step-${turnId}-${step.key}`;

  /* Words to the person are the conversation, not a detail: always shown, as prose. */
  if (step.kind === 'say' && step.drawn.kind === 'text') {
    return (
      <li id={id} className="relative mb-[8px]" data-step="say">
        <Node kind="say" running={running} />
        <Narration text={step.drawn.text} streaming={step.drawn.streaming} />
      </li>
    );
  }

  if (step.kind === 'steer' && step.drawn.kind === 'steer') {
    return (
      <li id={id} className="relative mb-[8px]" data-step="steer">
        <Node kind="steer" />
        <div className="rounded-lg border border-accent/30 bg-accent-subtle px-[10px] py-[6px] text-[12.5px] text-text-primary">
          <span className="mr-[6px] text-[10px] font-semibold uppercase tracking-[.08em] text-accent">{strings.turns.steered}</span>
          {step.drawn.text}
        </div>
      </li>
    );
  }

  const thinkingLive = step.kind === 'think' && running;

  return (
    <li id={id} className="relative mb-[6px]" data-step={step.kind} data-status={step.status}>
      <Node kind={step.kind} running={running} failed={step.status === 'failed'} />
      <button
        type="button"
        className={
          'flex w-full min-w-0 items-center gap-[8px] rounded-sm px-[4px] py-[3px] text-left text-[12.5px] ' +
          (body ? 'cursor-pointer hover:bg-bg-hover' : 'cursor-default')
        }
        aria-expanded={body ? open : undefined}
        onClick={() => {
          if (body) {
            setChosen(!open);
          }
        }}
      >
        <span
          className={
            'min-w-0 flex-1 truncate ' +
            (step.kind === 'think' ? 'italic text-text-secondary ' : 'text-text-primary ') +
            (step.kind === 'run' || step.kind === 'read' || step.kind === 'edit' || step.kind === 'explore' ? 'font-mono text-[11.5px] ' : '') +
            (thinkingLive ? 'shimmer-text' : '')
          }
          title={step.title}
        >
          {thinkingLive && step.title === '' ? strings.turns.thinking.title : step.title}
        </span>
        {step.detail === '' || step.kind === 'explore' ? null : (
          <span className={'shrink-0 font-mono text-[10.5px] ' + (step.status === 'failed' ? 'text-state-error' : 'text-text-muted')}>{step.detail}</span>
        )}
        {took === '' ? null : <span className="w-[46px] shrink-0 text-right font-mono text-[10.5px] tabular-nums text-text-muted">{took}</span>}
        <span className="grid w-[12px] shrink-0 place-items-center">
          {step.kind === 'think' || step.kind === 'checkpoint' ? null : <StatusMark status={step.status} />}
        </span>
        {body ? (
          <ChevronRight size={11} aria-hidden="true" className={'shrink-0 text-text-muted transition-transform duration-200 ' + (open ? 'rotate-90' : '')} />
        ) : (
          <span className="w-[11px] shrink-0" />
        )}
      </button>

      {/* A thought still going on shows its newest lines under the headline, fading upward - the
          reasoning as it forms, without a box that pushes everything else away. */}
      {thinkingLive && !open && step.drawn.kind === 'thinking' ? (
        <div className="think-tail mt-[2px] px-[4px] text-[11.5px] italic leading-[1.55] text-text-muted" data-newest={newest}>
          {tail(step.drawn.thinking.text).map((line, index) => (
            <div key={`${index}-${line}`} className="truncate">
              {line}
            </div>
          ))}
        </div>
      ) : null}

      {open && body ? (
        <div className="mb-[6px] mt-[4px] pl-[4px]">
          <StepBody step={step} sessionId={sessionId} />
        </div>
      ) : null}
    </li>
  );
}

/** The bottom of the rail while the turn runs: what is happening this second, and for how long. */
function NowNode({ turn, now }: { turn: Turn; now: number }) {
  const live = turn.live;

  if (live === undefined || live.phase === 'thinking' || live.phase === 'writing' || live.phase === 'drafting') {
    return null;
  }

  const since = Date.parse(live.since);
  const ms = Number.isFinite(since) ? Math.max(0, now - since) : 0;
  const label =
    live.phase === 'tool'
      ? `${strings.turns.live.tool} ${live.detail}`
      : live.phase === 'waiting'
        ? strings.turns.live.waiting(turn.meta.model)
        : strings.turns.live.deciding;

  return (
    <li className="relative mb-[6px]" data-now={live.phase}>
      <span className="node-live absolute left-[-24.5px] top-[6px] h-[12px] w-[12px] rounded-full border-2 border-accent bg-bg-base" aria-hidden="true" />
      <div className="flex items-center gap-[8px] px-[4px] py-[3px] text-[12.5px]" role="status">
        <span className="text-[10px] font-semibold uppercase tracking-[.08em] text-accent">{strings.turns.flow.now}</span>
        <span className="shimmer-text min-w-0 flex-1 truncate">{label}</span>
        <span className="shrink-0 font-mono text-[10.5px] tabular-nums text-text-muted">{strings.turns.thinking.seconds(ms)}</span>
      </div>
    </li>
  );
}
