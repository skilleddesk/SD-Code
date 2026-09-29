import { ChevronRight, Copy, Radio } from 'lucide-react';
import { useLayoutEffect, useRef, useState, type KeyboardEvent } from 'react';

import { strings } from '../../strings';
import { toast } from '../../store/toast';
import { AnswerBlock } from './AnswerBlock';
import { DraftCard } from './DraftCard';
import { changeTotals, matches, offset, ribbon, steps, tail, transcript, type Filter, type Step } from './flow';
import { Sparkline, StatusMark, StepBody, StepIcon, TimeRibbon } from './StepParts';
import { duration, hasBody, PHASE_COLOR, useNow, usePace } from './stepKit';
import { Narration } from './Timeline';
import type { LiveBarData, Turn } from './types';

/**
 * Style B - **Console** (0.15): mission control for one turn.
 *
 * Where Flow reads like a story, Console reads like instruments. One card holds the whole run:
 *
 *   HUD       the phase right now with its clock, elapsed, ~tokens, a live pace sparkline (tokens per
 *             second over the last forty seconds, measured by the window), tool calls, files changed so
 *             far with +/−, and the plan's progress - every number measured, none estimated from nothing;
 *   ribbon    the same time ribbon as Flow, thin;
 *   filters   All · Thinking · Reads · Edits · Commands · Messages, each with its count - the question
 *             "what did it run?" answered in one click instead of a scroll;
 *   log       one line per step: offset from the start (`+01:04`), status, kind, what, how long. It
 *             follows the newest line while you are at the bottom and stops when you scroll up. ↑/↓
 *             move, Enter opens a line, Esc closes it. "Copy log" puts the whole run on the clipboard
 *             as text, for a bug report or a colleague.
 *
 * Finished, the card folds to its HUD line; the answer stands under it.
 */
export function ConsoleWork({ turn, sessionId }: { turn: Turn; sessionId: string }) {
  const now = useNow(250, turn.running);
  const list = steps(turn.timeline, now, turn.endedAt, turn.running);
  const work = list.filter((step) => step.kind !== 'answer');
  const answers = list.filter((step) => step.kind === 'answer');
  const { segments, totals, total } = ribbon(list, turn.startedAt, now, turn.endedAt, turn.running);
  const change = changeTotals(turn.timeline);
  const pace = usePace(turn.stats?.chars ?? 0, turn.running);
  const [chosen, setChosen] = useState<boolean | null>(null);
  const open = chosen ?? turn.running;
  const [filter, setFilter] = useState<Filter>('all');
  const [selected, setSelected] = useState<string | null>(null);
  const [expanded, setExpanded] = useState<Record<string, boolean>>({});
  const [following, setFollowing] = useState(true);
  const log = useRef<HTMLDivElement>(null);
  const shown = work.filter((step) => matches(step, filter));
  const begin = Date.parse(turn.startedAt);

  /* Follow the newest line while the reader is at the bottom of the log. */
  useLayoutEffect(() => {
    const element = log.current;

    if (element !== null && following && turn.running) {
      element.scrollTop = element.scrollHeight;
    }
  });

  if (work.length === 0 && turn.draft === undefined && !turn.running) {
    return (
      <>
        {answers.map((step) => (step.drawn.kind === 'text' ? <AnswerBlock key={step.key} answer={{ text: step.drawn.text, streaming: false }} /> : null))}
      </>
    );
  }

  const counts: Record<Filter, number> = {
    all: work.length,
    think: work.filter((step) => matches(step, 'think')).length,
    read: work.filter((step) => matches(step, 'read')).length,
    edit: work.filter((step) => matches(step, 'edit')).length,
    run: work.filter((step) => matches(step, 'run')).length,
    say: work.filter((step) => matches(step, 'say')).length,
  };
  const tokens = Math.round((turn.stats?.chars ?? 0) / 4);
  const perSecond = pace.length > 0 ? pace[pace.length - 1] ?? 0 : 0;
  const planDone = turn.plan.filter((step) => step.status === 'done').length;

  const toggle = (key: string): void => setExpanded((current) => ({ ...current, [key]: !(current[key] ?? false) }));

  const onKey = (event: KeyboardEvent<HTMLDivElement>): void => {
    if (shown.length === 0) {
      return;
    }

    const index = shown.findIndex((step) => step.key === selected);

    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault();
      const next = event.key === 'ArrowDown' ? Math.min(shown.length - 1, index + 1) : Math.max(0, index < 0 ? shown.length - 1 : index - 1);
      const key = shown[next]?.key ?? null;

      setSelected(key);
      setFollowing(event.key === 'ArrowDown' && next === shown.length - 1);
      document.getElementById(`log-${turn.id}-${key}`)?.scrollIntoView({ block: 'nearest' });
    } else if (event.key === 'Enter' && selected !== null) {
      event.preventDefault();
      toggle(selected);
    } else if (event.key === 'Escape' && selected !== null) {
      setExpanded((current) => ({ ...current, [selected]: false }));
    } else if (event.key === 'End') {
      setFollowing(true);
    }
  };

  const copy = (): void => {
    void navigator.clipboard
      ?.writeText(`${turn.user.body}\n\n${transcript(list, turn.startedAt)}`)
      .then(() => toast(strings.turns.console.copied))
      .catch(() => undefined);
  };

  return (
    <div className="console-work mb-[8px]" data-console>
      <div className={'overflow-hidden rounded-lg border bg-bg-raised ' + (turn.running ? 'console-live border-accent/40' : 'border-border-subtle')}>
        {/* HUD */}
        <div className="flex min-w-0 flex-wrap items-center gap-x-[12px] gap-y-[4px] border-b border-border-subtle px-[12px] py-[7px] font-mono text-[10.5px] tabular-nums text-text-muted">
          <button
            type="button"
            className="flex min-w-0 items-center gap-[6px] font-ui text-[11.5px] font-semibold text-text-primary"
            aria-expanded={open}
            onClick={() => setChosen(!open)}
          >
            <ChevronRight size={12} aria-hidden="true" className={'shrink-0 text-text-muted transition-transform duration-200 ' + (open ? 'rotate-90' : '')} />
            {turn.running ? <Radio size={12} aria-hidden="true" className="shrink-0 animate-pulse text-state-error motion-reduce:animate-none" /> : null}
            <span className="truncate">{strings.turns.console.title}</span>
          </button>
          {turn.running && turn.live !== undefined ? <PhaseChip live={turn.live} now={now} model={turn.meta.model} /> : null}
          <span title={strings.turns.stats.elapsed('')}>{strings.turns.thinking.seconds(total)}</span>
          {tokens > 0 ? <span>{strings.turns.stats.tokens(tokens)}</span> : null}
          {turn.running ? (
            <span className="inline-flex items-center gap-[5px]">
              <Sparkline samples={pace} />
              <span className="w-[62px]">{strings.turns.stats.pace(perSecond)}</span>
            </span>
          ) : null}
          <span>{strings.turns.stats.tools(work.filter((step) => step.drawn.kind === 'tool' || step.drawn.kind === 'explore').length)}</span>
          {change.files > 0 ? (
            <span>
              {change.files}f <span className="text-diff-addText">+{change.added}</span>{' '}
              <span className="text-diff-removeText">−{change.removed}</span>
            </span>
          ) : null}
          {turn.plan.length > 0 ? (
            <span className="inline-flex items-center gap-[5px]">
              <span className="relative h-[4px] w-[44px] overflow-hidden rounded-full bg-bg-overlay">
                <span className="absolute inset-y-0 left-0 rounded-full bg-state-success" style={{ width: `${(planDone / turn.plan.length) * 100}%` }} />
              </span>
              {strings.turns.console.plan(planDone, turn.plan.length)}
            </span>
          ) : null}
          <button
            type="button"
            className="ml-auto inline-flex items-center gap-[4px] rounded-sm px-[6px] py-[1px] font-ui text-[10.5px] text-text-secondary hover:bg-bg-hover hover:text-text-primary"
            onClick={copy}
          >
            <Copy size={11} aria-hidden="true" />
            {strings.turns.console.copy}
          </button>
        </div>

        <div className="px-[12px] py-[6px]">
          <TimeRibbon segments={segments} totals={totals} total={total} running={turn.running} compact={!open} onPick={(key) => {
            setChosen(true);
            setFilter('all');
            setSelected(key);
            setExpanded((current) => ({ ...current, [key]: true }));
            setFollowing(false);
            window.setTimeout(() => document.getElementById(`log-${turn.id}-${key}`)?.scrollIntoView({ block: 'center', behavior: 'smooth' }), 30);
          }} />
        </div>

        {open ? (
          <>
            <div className="flex flex-wrap items-center gap-[2px] border-y border-border-subtle px-[8px] py-[4px]" role="tablist" aria-label={strings.turns.console.title}>
              {(Object.keys(counts) as Filter[]).map((key) => (
                <button
                  key={key}
                  type="button"
                  role="tab"
                  aria-selected={filter === key}
                  className={
                    'rounded-sm px-[8px] py-[2px] text-[11px] transition-colors ' +
                    (filter === key ? 'bg-bg-active font-medium text-text-primary' : 'text-text-muted hover:bg-bg-hover hover:text-text-secondary')
                  }
                  onClick={() => setFilter(key)}
                >
                  {strings.turns.console.filters[key]}
                  <span className="ml-[4px] font-mono text-[10px] text-text-faint">{counts[key]}</span>
                </button>
              ))}
            </div>

            <div
              ref={log}
              className="console-log max-h-[380px] overflow-y-auto py-[4px] outline-none focus-visible:ring-1 focus-visible:ring-border-focus"
              tabIndex={0}
              role="log"
              aria-label={strings.turns.console.title}
              onKeyDown={onKey}
              onScroll={() => {
                const element = log.current;

                if (element !== null) {
                  setFollowing(element.scrollHeight - element.scrollTop - element.clientHeight <= 16);
                }
              }}
            >
              {shown.length === 0 ? <div className="px-[12px] py-[10px] text-[11.5px] text-text-muted">{strings.turns.console.empty}</div> : null}

              {shown.map((step) => (
                <LogLine
                  key={step.key}
                  step={step}
                  turnId={turn.id}
                  sessionId={sessionId}
                  now={now}
                  begin={begin}
                  selected={selected === step.key}
                  open={expanded[step.key] ?? (step.status === 'running' && (step.kind === 'run' || step.kind === 'agent'))}
                  onClick={() => {
                    setSelected(step.key);
                    toggle(step.key);
                  }}
                />
              ))}

              {turn.draft !== undefined && filter === 'all' ? (
                <div className="px-[12px] py-[4px]">
                  <DraftCard draft={turn.draft} />
                </div>
              ) : null}
            </div>

            <div className="flex items-center gap-[10px] border-t border-border-subtle px-[12px] py-[4px] font-mono text-[10px] text-text-faint">
              <span>{strings.turns.console.keys}</span>
              {turn.running ? (
                <button type="button" className={'ml-auto ' + (following ? 'text-state-success' : 'text-state-waiting')} onClick={() => setFollowing(true)}>
                  {following ? `● ${strings.turns.console.follow}` : strings.turns.console.paused}
                </button>
              ) : null}
            </div>
          </>
        ) : null}
      </div>

      {answers.map((step) => (step.drawn.kind === 'text' ? <AnswerBlock key={step.key} answer={{ text: step.drawn.text, streaming: false }} /> : null))}
    </div>
  );
}

function PhaseChip({ live, now, model }: { live: LiveBarData; now: number; model: string }) {
  const since = Date.parse(live.since);
  const ms = Number.isFinite(since) ? Math.max(0, now - since) : 0;
  const phase = live.phase === 'thinking' ? 'think' : live.phase === 'writing' ? 'write' : live.phase === 'tool' ? 'run' : live.phase === 'drafting' ? 'edit' : 'wait';
  const label =
    live.phase === 'thinking'
      ? strings.turns.live.thinking
      : live.phase === 'writing'
        ? strings.turns.live.writing
        : live.phase === 'tool'
          ? strings.turns.live.tool
          : live.phase === 'drafting'
            ? strings.turns.draft.write
            : live.phase === 'waiting'
              ? strings.turns.live.waiting(model)
              : strings.turns.live.deciding;

  return (
    <span className="inline-flex min-w-0 max-w-[260px] items-center gap-[5px] rounded-full border border-border-default px-[7px] py-[1px] font-ui" role="status">
      <span className="h-[6px] w-[6px] shrink-0 animate-pulse rounded-full motion-reduce:animate-none" style={{ background: PHASE_COLOR[phase] }} aria-hidden="true" />
      <span className="truncate text-text-secondary">
        {label}
        {live.detail === '' || live.phase === 'thinking' ? '' : ` ${live.detail}`}
      </span>
      <span className="shrink-0 font-mono">{strings.turns.thinking.seconds(ms)}</span>
    </span>
  );
}

function LogLine({
  step,
  turnId,
  sessionId,
  now,
  begin,
  selected,
  open,
  onClick,
}: {
  step: Step;
  turnId: string;
  sessionId: string;
  now: number;
  begin: number;
  selected: boolean;
  open: boolean;
  onClick: () => void;
}) {
  const body = hasBody(step);
  const took = duration(step, now);
  const at = step.start === null || !Number.isFinite(begin) ? '' : offset(step.start - begin);
  const say = step.kind === 'say' && step.drawn.kind === 'text';
  const thinkingLive = step.kind === 'think' && step.status === 'running' && step.drawn.kind === 'thinking';

  return (
    <div id={`log-${turnId}-${step.key}`} className={'console-line ' + (selected ? 'bg-bg-active' : '')} data-step={step.kind} data-status={step.status}>
      <button
        type="button"
        tabIndex={-1}
        className={'flex w-full min-w-0 items-center gap-[8px] px-[12px] py-[2px] text-left font-mono text-[11.5px] leading-[1.7] ' + (body ? 'hover:bg-bg-hover' : 'cursor-default')}
        onClick={onClick}
      >
        <span className="w-[42px] shrink-0 text-[10.5px] tabular-nums text-text-faint">{at}</span>
        <span className="grid w-[12px] shrink-0 place-items-center">
          {step.kind === 'think' || step.kind === 'say' || step.kind === 'steer' || step.kind === 'checkpoint' ? null : <StatusMark status={step.status} size={10} />}
        </span>
        <StepIcon kind={step.kind} size={11} />
        <span
          className={
            'min-w-0 flex-1 truncate ' +
            (step.kind === 'think' ? 'font-ui italic text-text-secondary ' : step.kind === 'say' || step.kind === 'steer' ? 'font-ui text-text-primary ' : 'text-text-primary ') +
            (thinkingLive ? 'shimmer-text' : '')
          }
          title={step.title}
        >
          {thinkingLive && step.title === '' ? strings.turns.thinking.title : step.title}
        </span>
        {step.detail === '' || step.kind === 'explore' ? null : (
          <span className={'max-w-[34%] shrink-0 truncate text-[10.5px] ' + (step.status === 'failed' ? 'text-state-error' : 'text-text-muted')}>{step.detail}</span>
        )}
        <span className="w-[44px] shrink-0 text-right text-[10.5px] tabular-nums text-text-muted">{took}</span>
        {body ? <ChevronRight size={10} aria-hidden="true" className={'shrink-0 text-text-faint transition-transform ' + (open ? 'rotate-90' : '')} /> : <span className="w-[10px] shrink-0" />}
      </button>

      {/* What it says while it works is for the person: the newest words stay readable under the line. */}
      {say && !open && step.status === 'running' && step.drawn.kind === 'text' ? (
        <div className="py-[2px] pl-[82px] pr-[12px]">
          <Narration text={step.drawn.text} streaming bare />
        </div>
      ) : null}

      {thinkingLive && !open && step.drawn.kind === 'thinking' ? (
        <div className="pb-[2px] pl-[82px] pr-[12px] text-[11px] italic leading-[1.5] text-text-muted">
          {tail(step.drawn.thinking.text, 2).map((line, index) => (
            <div key={`${index}-${line}`} className="truncate">
              {line}
            </div>
          ))}
        </div>
      ) : null}

      {open && body ? (
        <div className="py-[4px] pl-[82px] pr-[12px]">
          <StepBody step={step} sessionId={sessionId} />
        </div>
      ) : null}
    </div>
  );
}

