import { ChevronRight, Copy, FileDiff } from 'lucide-react';
import { useEffect, useRef, useState, type ReactNode } from 'react';

import { strings } from '../../strings';
import { toast } from '../../store/toast';
import { AnswerBlock } from './AnswerBlock';
import { DraftCard } from './DraftCard';
import { changeTotals, headline, matches, ribbon, steps, transcript, type Filter, type Step } from './flow';
import { Markdown } from './Markdown';
import { Narration } from './Narration';
import { Sparkline, StepBody, TimeRibbon } from './StepParts';
import { duration, hasBody, reveal, useNow, usePace } from './stepKit';
import { ToolCard } from './ToolCard';
import type { DiffLine, RunLine, Turn } from './types';
import { copyText } from '../../lib/external';

const FILTERS: Filter[] = ['all', 'think', 'read', 'edit', 'run', 'say'];

/** How many lines a folded box keeps: the top of a file someone read, the end of a command's output. */
const FOLDED_READ = 6;
const FOLDED_OUT = 10;
const FOLDED_DIFF = 14;

/**
 * **The live transcript** (0.19) - everything the agent does, as it does it, the way Claude Code shows it.
 *
 * One rail, one dot per step, in the order things happened:
 *
 *   ○ Thinking…            the reasoning itself, streaming in its own box while the model thinks;
 *                          `Thought for 3.2s` afterwards, one click to read it again
 *   ● Read src/auth.ts     what the model read - the first lines of the file, numbered
 *   ● Grep "redirectTo"    the matches it found
 *   ● Edit guard.ts +4 −2  the diff
 *   ● Bash                 IN the command, OUT what it printed, live
 *   ✻ Cerebrating…         what is happening this second: the clock, the tokens, how to stop it
 *
 * Every step keeps its own measured time and its outcome (green done, red failed, violet running). Nothing
 * is grouped away: a turn that read nine files shows nine reads. A long box folds to its first (or last)
 * lines with a fade and one click for all of it.
 *
 * Under the steps, once the turn is over: where its time went (the ribbon, by kind, the grey being the
 * model deciding between steps), filters for a long run, and Copy log. The newest turn stays open;
 * older ones fold to that one summary line.
 */
export function FlowWork({ turn, sessionId, latest = true }: { turn: Turn; sessionId: string; latest?: boolean }) {
  const now = useNow(250, turn.running);
  const list = steps(turn.timeline, now, turn.endedAt, turn.running, false);
  const work = list.filter((step) => step.kind !== 'answer');
  const answers = list.filter((step) => step.kind === 'answer');
  const { segments, totals, total } = ribbon(list, turn.startedAt, now, turn.endedAt, turn.running);
  const change = changeTotals(turn.timeline);
  const pace = usePace(turn.stats?.chars ?? 0, turn.running);
  /* `null` = follow the turn (open while it runs and while it is the newest); a boolean = the person chose. */
  const [chosen, setChosen] = useState<boolean | null>(null);
  const [filter, setFilter] = useState<Filter>('all');
  const open = chosen ?? (turn.running || latest);
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
  const tools = work.filter((step) => step.drawn.kind === 'tool').length;
  const copy = (): void => {
    void copyText(`${turn.user.body}\n\n${transcript(list, turn.startedAt)}`).then((ok) => {
      if (ok) {
        toast(strings.turns.flow.copied);
      }
    });
  };

  /* The one line a folded (or finished) turn is summed up in. */
  const summaryLine = (
    <div className="flow-head group flex w-full min-w-0 items-center gap-[8px] text-[12px]">
      <button
        type="button"
        className="flex min-w-0 flex-1 items-center gap-[7px] rounded-sm py-[2px] text-left"
        aria-expanded={open}
        title={open ? strings.turns.flow.hide : strings.turns.flow.show}
        onClick={() => setChosen(!open)}
      >
        <ChevronRight size={13} aria-hidden="true" className={'shrink-0 text-text-muted transition-transform duration-200 ' + (open ? 'rotate-90' : '')} />
        <span className={'min-w-0 truncate font-medium ' + (turn.running ? 'shimmer-text' : 'text-text-secondary')}>
          {turn.running ? strings.turns.flow.working(work.length) : strings.turns.flow.worked(strings.turns.thinking.seconds(total), work.length)}
        </span>
        {change.files > 0 ? (
          <span className="inline-flex shrink-0 items-center gap-[4px] rounded-full border border-border-subtle bg-bg-raised px-[7px] py-[1px] font-mono text-[10.5px] text-text-muted">
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
      <button
        type="button"
        className="grid h-[22px] w-[22px] shrink-0 place-items-center rounded-sm text-text-muted hover:bg-bg-hover hover:text-text-primary"
        title={strings.turns.flow.copy}
        aria-label={strings.turns.flow.copy}
        onClick={copy}
      >
        <Copy size={12} aria-hidden="true" />
      </button>
    </div>
  );

  return (
    <div className="flow-work mb-[10px]" data-flow>
      {/* While it runs, the summary rides on top - steps so far, pace, clock - so the newest step stays at the
          bottom where the eye already is. */}
      {turn.running || !open ? <div className="mb-[10px] flex flex-col gap-[6px]">{summaryLine}<TimeRibbon segments={segments} totals={totals} total={total} running={turn.running} onPick={pick} compact /></div> : null}

      {open && work.length >= 6 ? (
        <div className="mb-[8px] ml-[2px] flex flex-wrap items-center gap-[3px]" role="group" aria-label={strings.turns.flow.filter}>
          {FILTERS.map((key) => {
            const count = work.filter((step) => matches(step, key)).length;

            return count === 0 && key !== 'all' ? null : (
              <button
                key={key}
                type="button"
                aria-pressed={filter === key}
                className={
                  'rounded-full border px-[9px] py-[1px] text-[10.5px] transition-colors ' +
                  (filter === key ? 'border-accent/30 bg-accent-subtle font-medium text-accent' : 'border-transparent text-text-muted hover:bg-bg-hover hover:text-text-secondary')
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
        <ol className="tl-rail pl-[24px]">
          {shown.map((step) => (
            <TimelineStep key={step.key} step={step} turnId={turn.id} sessionId={sessionId} now={now} />
          ))}

          {turn.draft === undefined || filter !== 'all' ? null : (
            <li className="relative pb-[12px]">
              <span className="tl-dot" data-state="running" aria-hidden="true" />
              <DraftCard draft={turn.draft} />
            </li>
          )}

          {turn.running && filter === 'all' ? <NowLine turn={turn} now={now} /> : null}
        </ol>
      ) : null}

      {answerBlocks}

      {/* Finished and open: where the time went, under everything it measured. */}
      {!turn.running && open ? (
        <div className="mt-[6px] flex flex-col gap-[6px] rounded-lg border border-border-subtle bg-bg-raised px-[12px] py-[8px]">
          {summaryLine}
          <TimeRibbon segments={segments} totals={totals} total={total} running={false} onPick={pick} />
        </div>
      ) : null}
    </div>
  );
}

/** The dot's colour: how the step went, or what kind of step it is when it has no outcome. */
function dotState(step: Step): string {
  if (step.status === 'running') {
    return step.kind === 'think' ? 'thinking' : 'running';
  }

  if (step.status === 'failed') {
    return 'failed';
  }

  return step.kind === 'think' ? 'think' : step.kind === 'say' || step.kind === 'steer' ? 'say' : 'done';
}

function TimelineStep({ step, turnId, sessionId, now }: { step: Step; turnId: string; sessionId: string; now: number }) {
  return (
    <li id={`step-${turnId}-${step.key}`} className="tl-step relative rounded-sm pb-[13px]" data-step={step.kind} data-status={step.status}>
      <span className="tl-dot" data-state={dotState(step)} aria-hidden="true" />
      <StepContent step={step} sessionId={sessionId} now={now} />
    </li>
  );
}

/** A read-like tool: its result is what the model saw, so the top of it is what is shown. */
const READS = new Set(['Read', 'View', 'List', 'Grep', 'Glob', 'Search', 'Search web', 'Fetch', 'Diff', 'Output', 'WebFetch', 'WebSearch', 'LS']);

function StepContent({ step, sessionId, now }: { step: Step; sessionId: string; now: number }) {
  const drawn = step.drawn;
  const took = duration(step, now);

  /* Words to the person are the conversation, not a detail: always shown, as prose. */
  if (step.kind === 'say' && drawn.kind === 'text') {
    return <Narration text={drawn.text} streaming={drawn.streaming} />;
  }

  if (step.kind === 'steer' && drawn.kind === 'steer') {
    return (
      <div className="rounded-lg border border-accent/30 bg-accent-subtle px-[10px] py-[6px] text-[12.5px] text-text-primary">
        <span className="mr-[6px] text-[10px] font-semibold uppercase tracking-[.08em] text-accent">{strings.turns.steered}</span>
        {drawn.text}
      </div>
    );
  }

  if (drawn.kind === 'thinking') {
    return <ThoughtBlock text={drawn.thinking.text} live={step.status === 'running'} took={took} />;
  }

  if (drawn.kind === 'tool') {
    const tool = drawn.tool;
    const failed = tool.status === 'failed';
    const live = tool.status === 'running';

    if (tool.name === 'Question') {
      return <ToolCard tool={tool} />;
    }

    if (tool.kind === 'run') {
      return (
        <>
          <StepLine name={tool.name} target="" detail={tool.meta} took={took} failed={failed} />
          <IoBox command={tool.target} output={tool.output} live={live} />
        </>
      );
    }

    if (tool.kind === 'edit') {
      return (
        <>
          <StepLine name={tool.name} target={tool.target} detail={step.detail} took={took} failed={failed} />
          {tool.diff.length > 0 ? <DiffBox diff={tool.diff} /> : null}
        </>
      );
    }

    /* A read, a search, a fetch - or a sub-agent, whose report is its output. */
    const output = tool.output ?? [];

    return (
      <>
        <StepLine name={tool.name} target={tool.target} detail={tool.meta} took={took} failed={failed} />
        {output.length > 0 ? <OutBox output={output} live={live} head={READS.has(tool.name)} /> : null}
      </>
    );
  }

  /* A checkpoint: its line, and the way back to it one click away. */
  return <Expandable title={<StepLine name={step.title} target="" detail="" took="" />} body={hasBody(step) ? <StepBody step={step} sessionId={sessionId} /> : null} />;
}

/** `Bash  …  exit 0 · 3.8s` - the name in the strong weight, the target in the accent. */
function StepLine({ name, target, detail, took, failed = false }: { name: string; target: string; detail: string; took: string; failed?: boolean }) {
  return (
    <div className="flex min-w-0 items-baseline gap-[8px] text-[13px] leading-[1.5]">
      <span className="shrink-0 font-semibold text-text-primary">{name}</span>
      {target === '' ? null : (
        <span className="min-w-0 truncate font-mono text-[12px] text-accent" title={target}>
          {target}
        </span>
      )}
      <span className="ml-auto flex shrink-0 items-baseline gap-[8px] pl-[6px] font-mono text-[10.5px] tabular-nums">
        {detail === '' ? null : <span className={failed ? 'text-state-error' : 'text-text-muted'}>{detail}</span>}
        {took === '' ? null : <span className="text-text-muted">{took}</span>}
      </span>
    </div>
  );
}

/**
 * The reasoning. While the model thinks, its words stream into their own box, newest at the bottom; once
 * it moves on, the box folds to `Thought for 3.2s · its headline`, and one click reads it again.
 */
function ThoughtBlock({ text, live, took }: { text: string; live: boolean; took: string }) {
  const [chosen, setChosen] = useState<boolean | null>(null);
  const open = chosen ?? live;
  const box = useRef<HTMLDivElement>(null);
  const title = headline(text);

  useEffect(() => {
    if (open && live && box.current !== null) {
      box.current.scrollTop = box.current.scrollHeight;
    }
  }, [open, live, text]);

  return (
    <div className="thought" data-thought={live ? 'live' : 'done'}>
      <button
        type="button"
        className="flex w-full min-w-0 items-baseline gap-[7px] rounded-sm text-left text-[13px] leading-[1.5] text-text-muted transition-colors hover:text-text-secondary"
        aria-expanded={text.trim() === '' ? undefined : open}
        title={open ? strings.turns.thinking.collapse : strings.turns.thinking.expand}
        onClick={() => setChosen(!open)}
      >
        <span className={'shrink-0 font-medium ' + (live ? 'text-purple' : '')}>
          {live ? `${strings.turns.flow.thinking}…` : took === '' ? strings.turns.thinking.thought : strings.turns.thinking.thoughtFor(took)}
        </span>
        {live && took !== '' ? <span className="shrink-0 font-mono text-[10.5px] tabular-nums">{took}</span> : null}
        {title === '' || open ? null : <span className="min-w-0 truncate italic text-text-muted opacity-80">· {title}</span>}
        <ChevronRight size={11} aria-hidden="true" className={'ml-auto shrink-0 self-center transition-transform duration-200 ' + (open ? 'rotate-90' : '')} />
      </button>

      {open && text.trim() !== '' ? (
        <div
          ref={box}
          className={'thought-md mt-[6px] overflow-y-auto rounded-md border border-border-subtle bg-bg-input px-[12px] py-[8px] ' + (live ? 'think-live max-h-[220px]' : 'max-h-[360px]')}
        >
          <Markdown text={text} />
          {live ? <span className="ml-[1px] animate-pulse text-purple motion-reduce:animate-none" aria-hidden="true">▍</span> : null}
        </div>
      ) : null}
    </div>
  );
}

/** One line of output: a file's line number set apart, coloured by what the engine said it was. */
function OutLine({ line }: { line: RunLine }) {
  const numbered = /^\s*(\d+)\t(.*)$/.exec(line.text);
  const tone = line.level === 'ok' ? 'text-state-success' : line.level === 'fail' ? 'text-state-error' : 'text-text-secondary';

  if (numbered !== null) {
    return (
      <div className={'flex ' + tone}>
        <span className="w-[38px] shrink-0 select-none pr-[10px] text-right text-text-muted opacity-70">{numbered[1]}</span>
        <span className="min-w-0 whitespace-pre-wrap break-words">{numbered[2] === '' ? ' ' : numbered[2]}</span>
      </div>
    );
  }

  return <div className={'whitespace-pre-wrap break-words ' + (line.text.startsWith('… ') ? 'italic text-text-muted' : tone)}>{line.text === '' ? ' ' : line.text}</div>;
}

/**
 * The command and what it printed: `IN` the command, `OUT` the output. While it runs the output follows
 * its newest line; finished, a long one folds to its last lines (where the result is).
 */
function IoBox({ command, output, live }: { command: string; output: readonly RunLine[]; live: boolean }) {
  const copy = (): void => {
    void copyText(command).then((ok) => {
      if (ok) {
        toast(strings.turns.flow.copiedCommand);
      }
    });
  };

  return (
    <div className="io-box group/io mt-[7px] font-mono text-[12px] leading-[1.6]" data-io={live ? 'live' : 'done'}>
      <div className="io-row grid grid-cols-[42px_1fr]">
        <div className="io-label select-none px-[9px] py-[7px] text-[9.5px] font-semibold tracking-[.1em] text-text-muted">{strings.turns.flow.in}</div>
        <div className="relative min-w-0 py-[7px] pr-[32px]">
          <span className="whitespace-pre-wrap break-words text-text-primary">{command}</span>
          <button
            type="button"
            className="absolute right-[6px] top-[6px] grid h-[20px] w-[20px] place-items-center rounded-sm text-text-muted opacity-0 transition-opacity hover:bg-bg-hover hover:text-text-primary focus-visible:opacity-100 group-hover/io:opacity-100"
            title={strings.turns.flow.copyCommand}
            aria-label={strings.turns.flow.copyCommand}
            onClick={copy}
          >
            <Copy size={11} aria-hidden="true" />
          </button>
        </div>
      </div>
      <div className="io-row grid grid-cols-[42px_1fr]">
        <div className="io-label select-none px-[9px] py-[7px] text-[9.5px] font-semibold tracking-[.1em] text-text-muted">{strings.turns.flow.out}</div>
        <OutLines output={output} live={live} head={false} />
      </div>
    </div>
  );
}

/** Output with no command - the file read, the matches, a sub-agent's report. */
function OutBox({ output, live, head }: { output: readonly RunLine[]; live: boolean; head: boolean }) {
  return (
    <div className="io-box mt-[7px] font-mono text-[12px] leading-[1.6]">
      <div className="io-row grid grid-cols-[42px_1fr]">
        <div className="io-label select-none px-[9px] py-[7px] text-[9.5px] font-semibold tracking-[.1em] text-text-muted">{strings.turns.flow.out}</div>
        <OutLines output={output} live={live} head={head} />
      </div>
    </div>
  );
}

function OutLines({ output, live, head }: { output: readonly RunLine[]; live: boolean; head: boolean }) {
  const [all, setAll] = useState(false);
  const box = useRef<HTMLDivElement>(null);
  const keep = head ? FOLDED_READ : FOLDED_OUT;
  const folded = !live && !all && output.length > keep;
  const lines = folded ? (head ? output.slice(0, keep) : output.slice(-keep)) : output;

  useEffect(() => {
    if (live && box.current !== null) {
      box.current.scrollTop = box.current.scrollHeight;
    }
  }, [live, output.length]);

  if (output.length === 0) {
    return <div className="py-[7px] italic text-text-muted">{live ? '…' : strings.turns.flow.noOutput}</div>;
  }

  const toggle =
    output.length > keep && !live ? (
      <button type="button" className="font-ui text-[11px] text-text-muted hover:text-accent" onClick={() => setAll(!all)}>
        {all ? strings.turns.flow.showLess : strings.turns.flow.showAll(output.length)}
      </button>
    ) : null;

  return (
    <div className="min-w-0 py-[7px] pr-[10px]">
      {folded && !head ? <div className="mb-[2px]">{toggle}</div> : null}
      <div ref={box} className={(live || all ? 'max-h-[320px] overflow-y-auto ' : '') + (folded ? (head ? 'io-fade' : 'io-fade-top') : '')}>
        {lines.map((line, index) => (
          <OutLine key={`${index}-${line.text}`} line={line} />
        ))}
      </div>
      {(folded && head) || all ? <div className="mt-[2px]">{toggle}</div> : null}
    </div>
  );
}

/** An edit's diff: its first lines in view, a fade, and all of it one click away. */
function DiffBox({ diff }: { diff: readonly DiffLine[] }) {
  const [all, setAll] = useState(false);
  const folded = !all && diff.length > FOLDED_DIFF;
  const rows = folded ? diff.slice(0, FOLDED_DIFF) : diff;

  return (
    <div className="io-box mt-[7px]" data-diff>
      <div className={'max-h-[440px] overflow-auto py-[4px] font-mono text-[12px] leading-[1.65] ' + (folded ? 'io-fade' : '')}>
        {rows.map((line, index) => (
          <div
            key={`${line.lineNumber}-${line.change}-${index}`}
            className={'diff-line flex min-w-max pr-[12px] ' + (line.change === 'add' ? 'add bg-diff-addBg text-diff-addText' : 'rem bg-diff-removeBg text-diff-removeText')}
          >
            <span className="ln w-[42px] shrink-0 select-none pr-[8px] text-right text-text-muted opacity-70">{line.lineNumber}</span>
            <span className="w-[14px] shrink-0 select-none opacity-80">{line.change === 'add' ? '+' : '−'}</span>
            <span className="whitespace-pre">{line.text}</span>
          </div>
        ))}
      </div>
      {diff.length > FOLDED_DIFF ? (
        <button type="button" className="w-full border-t border-border-subtle px-[10px] py-[4px] text-left text-[11px] text-text-muted hover:text-accent" onClick={() => setAll(!all)}>
          {all ? strings.turns.flow.showLess : strings.turns.flow.showAll(diff.length)}
        </button>
      ) : null}
    </div>
  );
}

/** A line that opens to a body - a checkpoint and its rewind. */
function Expandable({ title, body }: { title: ReactNode; body: ReactNode }) {
  const [open, setOpen] = useState(false);

  if (body === null) {
    return <>{title}</>;
  }

  return (
    <>
      <button type="button" className="block w-full text-left" aria-expanded={open} onClick={() => setOpen(!open)}>
        {title}
      </button>
      {open ? <div className="mt-[6px]">{body}</div> : null}
    </>
  );
}

/** Claude Code's spinner: a star that grows and shrinks through these shapes, and back. */
const SPARK = ['·', '✢', '✳', '✶', '✻', '✽', '✻', '✶', '✳', '✢'];

/**
 * The bottom of a running turn: what is happening this second. A turning spark, a word for the phase
 * (`Running Bash`, `Writing the answer`, or - while the model decides - a word that changes every few
 * seconds so a long wait never looks frozen), then the clock, the tokens so far, the plan step and how to
 * stop it.
 */
function NowLine({ turn, now }: { turn: Turn; now: number }) {
  const live = turn.live;
  const started = Date.parse(turn.stats?.startedAt ?? turn.startedAt);
  const elapsed = Number.isFinite(started) ? Math.max(0, now - started) : 0;
  const tokens = Math.round((turn.stats?.chars ?? 0) / 4);
  const verbs = strings.turns.flow.verbs;
  const seed = turn.id.length + turn.id.charCodeAt(turn.id.length - 1);
  const musing = verbs[(seed + Math.floor(elapsed / 4000)) % verbs.length] ?? strings.turns.flow.thinking;

  const label =
    live === undefined
      ? musing
      : live.phase === 'tool'
        ? strings.turns.flow.running(live.tool ?? (live.detail.split(' ')[0] || ''))
        : live.phase === 'writing'
          ? strings.turns.flow.writingReply
          : live.phase === 'waiting'
            ? strings.turns.live.waiting(turn.meta.model)
            : musing;
  const facts = [strings.turns.thinking.seconds(elapsed), ...(tokens > 0 ? [`↓ ${strings.turns.stats.tokens(tokens)}`] : []), strings.turns.flow.stop];

  return (
    <li className="now-line relative pb-[4px]" data-now={live?.phase ?? 'waiting'}>
      <span className="sdc-spark absolute left-[-25px] top-[1px] w-[16px] text-center text-[15px] leading-none" aria-hidden="true">
        {SPARK[Math.floor(now / 140) % SPARK.length]}
      </span>
      <div className="flex min-w-0 flex-wrap items-baseline gap-x-[8px] gap-y-[3px] text-[13px]" role="status">
        <span className="now-label font-medium">{label}…</span>
        <span className="font-mono text-[11px] tabular-nums text-text-muted">({facts.join(' · ')})</span>
        {live?.step === null || live?.step === undefined ? null : (
          <span className="min-w-0 max-w-full truncate rounded-full bg-accent-subtle px-[8px] py-[1px] text-[10.5px] text-accent" title={live.step.text}>
            {strings.turns.flow.step(live.step.index, live.step.total, live.step.text)}
          </span>
        )}
      </div>
    </li>
  );
}
