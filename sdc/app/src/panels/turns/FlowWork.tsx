import { ChevronRight, Copy, FileDiff } from 'lucide-react';
import { useEffect, useRef, useState, type ReactNode } from 'react';

import { strings } from '../../strings';
import { toast } from '../../store/toast';
import { AnswerBlock } from './AnswerBlock';
import { DraftCard } from './DraftCard';
import { changeTotals, headline, matches, ribbon, steps, tail, transcript, type Filter, type Step } from './flow';
import { Sparkline, StepBody, TimeRibbon } from './StepParts';
import { duration, hasBody, reveal, useNow, usePace } from './stepKit';
import { Markdown } from './Markdown';
import { Narration } from './Narration';
import { ToolCard } from './ToolCard';
import type { DiffLine, RunLine, Turn } from './types';
import { copyText } from '../../lib/external';

const FILTERS: Filter[] = ['all', 'think', 'read', 'edit', 'run', 'say'];

/** How many lines of output or diff a folded box shows before "Show all". */
const FOLDED_OUT = 8;
const FOLDED_DIFF = 12;

/**
 * **The work timeline** (0.18) - what the agent does, step by step, as it does it.
 *
 * One hairline rail with a dot per step, the way a careful colleague would narrate: `Thought for 3.2s`,
 * `Read router.ts · 84 ln`, `Edit guard.ts +4 −2` with the diff right under it, `Bash` with its command
 * (IN) and what it printed (OUT), live while it runs. The dot says how each step went - green done, red
 * failed, violet breathing while it runs - and every step keeps its own measured time.
 *
 * What it keeps from Flow (0.15), because no other agent UI shows it: the time ribbon (where the turn's
 * seconds went, by kind, including the grey *deciding* between steps), the output pace while it runs,
 * filters on a long run, and "Copy log". What it adds: the IN / OUT boxes and diffs are visible without
 * a click (folded to their first lines, faded, one click for all of them), thinking reads as one quiet
 * `Thought for…` line, and the bottom of a running turn always says what is happening this second with a
 * turning spark, its clock, the tokens so far and how to stop it.
 *
 * The newest turn stays open when it finishes; older turns fold to their one-line summary and ribbon.
 */
export function FlowWork({ turn, sessionId, latest = true }: { turn: Turn; sessionId: string; latest?: boolean }) {
  const now = useNow(250, turn.running);
  const list = steps(turn.timeline, now, turn.endedAt, turn.running);
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
  const tools = work.filter((step) => step.drawn.kind === 'tool' || step.drawn.kind === 'explore').length;
  const copy = (): void => {
    void copyText(`${turn.user.body}\n\n${transcript(list, turn.startedAt)}`).then((ok) => {
      if (ok) {
        toast(strings.turns.flow.copied);
      }
    });
  };

  return (
    <div className="flow-work mb-[10px]" data-flow>
      <div className="flow-head group mb-[10px] flex w-full flex-col gap-[7px]">
        <div className="flex w-full min-w-0 items-center gap-[8px] text-[12px]">
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
            className="grid h-[22px] w-[22px] shrink-0 place-items-center rounded-sm text-text-muted opacity-0 transition-opacity hover:bg-bg-hover hover:text-text-primary focus-visible:opacity-100 group-hover:opacity-100"
            title={strings.turns.flow.copy}
            aria-label={strings.turns.flow.copy}
            onClick={copy}
          >
            <Copy size={12} aria-hidden="true" />
          </button>
        </div>
        <TimeRibbon segments={segments} totals={totals} total={total} running={turn.running} onPick={pick} compact={!open} />
      </div>

      {/* A long run gets filters: "what did it run?" in one click instead of a scroll. */}
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
        <ol className="tl-rail pl-[22px]">
          {shown.map((step) => (
            <TimelineStep key={step.key} step={step} turnId={turn.id} sessionId={sessionId} now={now} />
          ))}

          {turn.draft === undefined || filter !== 'all' ? null : (
            <li className="relative pb-[10px]">
              <span className="tl-dot" data-state="running" aria-hidden="true" />
              <DraftCard draft={turn.draft} />
            </li>
          )}

          {turn.running && filter === 'all' && turn.live?.phase !== 'drafting' ? <NowLine turn={turn} now={now} /> : null}
        </ol>
      ) : null}

      {answerBlocks}
    </div>
  );
}

/** The dot's colour: how the step went, or what kind of step it is when it has no outcome. */
function dotState(step: Step): string {
  if (step.status === 'running') {
    return 'running';
  }

  if (step.status === 'failed') {
    return 'failed';
  }

  return step.kind === 'think' ? 'think' : step.kind === 'say' || step.kind === 'steer' ? 'say' : 'done';
}

function TimelineStep({ step, turnId, sessionId, now }: { step: Step; turnId: string; sessionId: string; now: number }) {
  const id = `step-${turnId}-${step.key}`;

  return (
    <li id={id} className="relative rounded-sm pb-[11px]" data-step={step.kind} data-status={step.status}>
      <span className="tl-dot" data-state={dotState(step)} aria-hidden="true" />
      <StepContent step={step} sessionId={sessionId} now={now} />
    </li>
  );
}

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
    return <ThoughtLine text={drawn.thinking.text} live={step.status === 'running'} took={took} />;
  }

  if (drawn.kind === 'explore') {
    return <ExploreStep step={step} took={took} />;
  }

  if (drawn.kind === 'tool') {
    const tool = drawn.tool;

    if (tool.name === 'Question') {
      return <ToolCard tool={tool} />;
    }

    if (tool.kind === 'run') {
      return (
        <>
          <StepLine name={tool.name} target="" detail={tool.meta} took={took} failed={tool.status === 'failed'} />
          <IoBox command={tool.target} output={tool.output} live={tool.status === 'running'} />
        </>
      );
    }

    if (tool.kind === 'edit') {
      return (
        <>
          <StepLine name={tool.name} target={tool.target} detail={step.detail} took={took} failed={tool.status === 'failed'} />
          {tool.diff.length > 0 ? <DiffBox diff={tool.diff} /> : null}
        </>
      );
    }

    /* A read, a search, a fetch - or a sub-agent, whose report is its output. */
    return (
      <>
        <StepLine name={tool.name} target={tool.target} detail={tool.meta} took={took} failed={tool.status === 'failed'} />
        {(tool.output?.length ?? 0) > 0 ? <OutBox output={tool.output ?? []} live={tool.status === 'running'} /> : null}
      </>
    );
  }

  /* A checkpoint: its line, and the way back to it one click away. */
  return <Expandable title={<StepLine name={step.title} target="" detail="" took="" />} body={hasBody(step) ? <StepBody step={step} sessionId={sessionId} /> : null} />;
}

/** `Bash  pnpm test …  exit 0 · 3.8s` - the name in the strong weight, the target in the accent. */
function StepLine({ name, target, detail, took, failed = false }: { name: string; target: string; detail: string; took: string; failed?: boolean }) {
  return (
    <div className="flex min-w-0 items-baseline gap-[8px] text-[12.5px] leading-[1.5]">
      <span className="shrink-0 font-semibold text-text-primary">{name}</span>
      {target === '' ? null : (
        <span className="min-w-0 truncate font-mono text-[11.5px] text-accent" title={target}>
          {target}
        </span>
      )}
      <span className="ml-auto flex shrink-0 items-baseline gap-[8px] pl-[6px] font-mono text-[10.5px] tabular-nums">
        {detail === '' ? null : <span className={failed ? 'text-state-error' : 'text-text-muted'}>{detail}</span>}
        {took === '' ? null : <span className="text-text-faint">{took}</span>}
      </span>
    </div>
  );
}

/** `Thought for 3.2s · Tracing the redirect` - one quiet line; a click opens the whole thought. */
function ThoughtLine({ text, live, took }: { text: string; live: boolean; took: string }) {
  const [open, setOpen] = useState(false);
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
        className="flex w-full min-w-0 items-baseline gap-[7px] rounded-sm text-left text-[12.5px] leading-[1.5] text-text-muted transition-colors hover:text-text-secondary"
        aria-expanded={text.trim() === '' ? undefined : open}
        title={open ? strings.turns.thinking.collapse : strings.turns.thinking.expand}
        onClick={() => setOpen((current) => !current)}
      >
        <span className={'shrink-0 font-medium ' + (live ? 'shimmer-text' : '')}>
          {live ? strings.turns.flow.thinking : took === '' ? strings.turns.thinking.thought : strings.turns.thinking.thoughtFor(took)}
        </span>
        {live && took !== '' ? <span className="shrink-0 font-mono text-[10.5px] tabular-nums">{took}</span> : null}
        {title === '' ? null : <span className="min-w-0 truncate italic text-text-faint">· {title}</span>}
        <ChevronRight size={11} aria-hidden="true" className={'ml-auto shrink-0 self-center transition-transform duration-200 ' + (open ? 'rotate-90' : '')} />
      </button>

      {/* A thought still going on shows its newest lines, fading upward - the reasoning as it forms. */}
      {live && !open ? (
        <div className="think-tail mt-[3px] border-l border-purple/40 pl-[10px] text-[11.5px] italic leading-[1.6] text-text-muted">
          {tail(text, 4)
            .map((line) => line.replace(/\*\*/g, '').replace(/^#{1,4}\s+/, ''))
            .filter((line) => line !== title)
            .slice(-3)
            .map((line, index) => (
              <div key={`${index}-${line}`} className="truncate">
                {line}
              </div>
            ))}
        </div>
      ) : null}

      {open ? (
        <div ref={box} className="thought-md mt-[5px] max-h-[320px] overflow-y-auto border-l border-purple/40 pl-[11px]">
          <Markdown text={text} />
        </div>
      ) : null}
    </div>
  );
}

/** Reads in a row, gathered: `Explored 3 files, 1 search`, and each one under it, one row apiece. */
function ExploreStep({ step, took }: { step: Step; took: string }) {
  const drawn = step.drawn;
  const [all, setAll] = useState(false);

  if (drawn.kind !== 'explore') {
    return null;
  }

  const searches = drawn.tools.filter((tool) => tool.name === 'Grep' || tool.name === 'Glob' || tool.name.startsWith('Search')).length;
  const shown = all ? drawn.tools : drawn.tools.slice(0, 4);
  const failed = drawn.tools.filter((tool) => tool.status === 'failed').length;

  return (
    <>
      <StepLine
        name={strings.turns.explored}
        target=""
        detail={[strings.turns.flow.explored(drawn.tools.length - searches, searches), failed > 0 ? strings.turns.exploredFailed(failed) : ''].filter((part) => part !== '').join(' · ')}
        took={took}
        failed={failed > 0}
      />
      <div className="mt-[3px] flex flex-col gap-[1px]">
        {shown.map((tool, index) => (
          <div key={`${tool.startedAt}-${index}`} className="flex min-w-0 items-baseline gap-[8px] text-[11.5px]">
            <span className="w-[14px] shrink-0 text-center text-text-faint" aria-hidden="true">
              ⎿
            </span>
            <span className="w-[44px] shrink-0 text-text-muted">{tool.name}</span>
            <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-text-secondary" title={tool.target}>
              {tool.target}
            </span>
            <span className={'shrink-0 font-mono text-[10px] ' + (tool.status === 'failed' ? 'text-state-error' : 'text-text-faint')}>{tool.meta}</span>
          </div>
        ))}
        {drawn.tools.length > 4 ? (
          <button type="button" className="ml-[22px] self-start text-[11px] text-text-muted hover:text-accent" onClick={() => setAll(!all)}>
            {all ? strings.turns.flow.showLess : `+${drawn.tools.length - 4}`}
          </button>
        ) : null}
      </div>
    </>
  );
}

/** One line of output, coloured by what the engine said it was. */
function OutLine({ line }: { line: RunLine }) {
  return (
    <div className={'whitespace-pre-wrap break-words ' + (line.level === 'ok' ? 'text-state-success' : line.level === 'fail' ? 'text-state-error' : 'text-text-secondary')}>
      {line.text === '' ? ' ' : line.text}
    </div>
  );
}

/**
 * The command and what it printed: `IN` the command, `OUT` the output. While it runs the output
 * follows its newest line; finished, a long one folds to its last lines (where the result is) and one
 * click shows all of it.
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
    <div className="io-box group/io mt-[6px] font-mono text-[11.5px] leading-[1.6]" data-io={live ? 'live' : 'done'}>
      <div className="io-row grid grid-cols-[38px_1fr]">
        <div className="select-none px-[8px] py-[6px] text-[9.5px] font-semibold tracking-[.08em] text-text-muted">{strings.turns.flow.in}</div>
        <div className="relative min-w-0 py-[6px] pr-[30px]">
          <span className="whitespace-pre-wrap break-words text-text-primary">{command}</span>
          <button
            type="button"
            className="absolute right-[6px] top-[5px] grid h-[20px] w-[20px] place-items-center rounded-sm text-text-muted opacity-0 transition-opacity hover:bg-bg-hover hover:text-text-primary focus-visible:opacity-100 group-hover/io:opacity-100"
            title={strings.turns.flow.copyCommand}
            aria-label={strings.turns.flow.copyCommand}
            onClick={copy}
          >
            <Copy size={11} aria-hidden="true" />
          </button>
        </div>
      </div>
      <div className="io-row grid grid-cols-[38px_1fr]">
        <div className="select-none px-[8px] py-[6px] text-[9.5px] font-semibold tracking-[.08em] text-text-muted">{strings.turns.flow.out}</div>
        <OutLines output={output} live={live} />
      </div>
    </div>
  );
}

/** Output with no command - a sub-agent's report, an answered read. */
function OutBox({ output, live }: { output: readonly RunLine[]; live: boolean }) {
  return (
    <div className="io-box mt-[6px] font-mono text-[11.5px] leading-[1.6]">
      <div className="pl-[10px]">
        <OutLines output={output} live={live} />
      </div>
    </div>
  );
}

function OutLines({ output, live }: { output: readonly RunLine[]; live: boolean }) {
  const [all, setAll] = useState(false);
  const box = useRef<HTMLDivElement>(null);
  const folded = !live && !all && output.length > FOLDED_OUT;
  const lines = folded ? output.slice(-FOLDED_OUT) : output;

  useEffect(() => {
    if (live && box.current !== null) {
      box.current.scrollTop = box.current.scrollHeight;
    }
  }, [live, output.length]);

  if (output.length === 0) {
    return <div className="py-[6px] italic text-text-faint">{live ? '…' : strings.turns.flow.noOutput}</div>;
  }

  return (
    <div className="min-w-0 py-[6px] pr-[10px]">
      {folded ? (
        <button type="button" className="mb-[2px] font-ui text-[11px] text-text-muted hover:text-accent" onClick={() => setAll(true)}>
          {strings.turns.flow.showAll(output.length)}
        </button>
      ) : null}
      <div ref={box} className={(live || all ? 'max-h-[300px] overflow-y-auto ' : '') + (folded ? 'io-fade-top' : '')}>
        {lines.map((line, index) => (
          <OutLine key={`${index}-${line.text}`} line={line} />
        ))}
      </div>
      {all && output.length > FOLDED_OUT ? (
        <button type="button" className="mt-[2px] font-ui text-[11px] text-text-muted hover:text-accent" onClick={() => setAll(false)}>
          {strings.turns.flow.showLess}
        </button>
      ) : null}
    </div>
  );
}

/** An edit's diff: its first lines in view, a fade, and all of it one click away. */
function DiffBox({ diff }: { diff: readonly DiffLine[] }) {
  const [all, setAll] = useState(false);
  const folded = !all && diff.length > FOLDED_DIFF;
  const rows = folded ? diff.slice(0, FOLDED_DIFF) : diff;

  return (
    <div className="io-box mt-[6px]" data-diff>
      <div className={'max-h-[420px] overflow-auto py-[4px] font-mono text-[11.5px] leading-[1.65] ' + (folded ? 'io-fade' : '')}>
        {rows.map((line, index) => (
          <div
            key={`${line.lineNumber}-${line.change}-${index}`}
            className={'diff-line flex min-w-max pr-[12px] ' + (line.change === 'add' ? 'add bg-diff-addBg text-diff-addText' : 'rem bg-diff-removeBg text-diff-removeText')}
          >
            <span className="ln w-[40px] shrink-0 select-none pr-[8px] text-right text-text-faint">{line.lineNumber}</span>
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

const SPARK = ['✻', '✽', '✶', '✳', '✢', '·', '✢', '✳', '✶', '✽'];

/**
 * The bottom of a running turn: what is happening this second. A turning spark, a word for the phase
 * (`Running Bash`, `Writing the answer`, or - while the model decides - a word that changes every few
 * seconds so a long wait never looks frozen), then the clock, the tokens so far and how to stop it.
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
    <li className="relative pb-[4px]" data-now={live?.phase ?? 'waiting'}>
      <span className="sdc-spark absolute left-[-24px] top-[0px] text-[15px] leading-none" aria-hidden="true">
        {SPARK[Math.floor(now / 160) % SPARK.length]}
      </span>
      <div className="flex min-w-0 flex-wrap items-baseline gap-x-[8px] gap-y-[2px] text-[12.5px]" role="status">
        <span className="shimmer-text font-medium">{label}…</span>
        <span className="font-mono text-[10.5px] tabular-nums text-text-muted">({facts.join(' · ')})</span>
        {live?.step === null || live?.step === undefined ? null : (
          <span className="min-w-0 max-w-full truncate rounded-full bg-accent-subtle px-[8px] py-[1px] text-[10.5px] text-accent" title={live.step.text}>
            {strings.turns.flow.step(live.step.index, live.step.total, live.step.text)}
          </span>
        )}
      </div>
    </li>
  );
}
