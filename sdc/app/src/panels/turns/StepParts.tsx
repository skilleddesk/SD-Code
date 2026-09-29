import { Bot, Brain, Check, CornerDownRight, FilePen, FileSearch, FileText, History, Loader, MessageCircleQuestion, MessageSquareText, Play, Sparkles, X } from 'lucide-react';
import { useEffect, useRef } from 'react';

import { strings } from '../../strings';
import { CheckpointRail } from './CheckpointRail';
import type { Phase, Segment, Step, StepKind } from './flow';
import { Markdown } from './Markdown';
import { Narration } from './Narration';
import { PHASE_COLOR } from './stepKit';
import { ToolBody, ToolCard } from './ToolCard';

/**
 * The pieces Flow and Console share (0.15): a clock, the time ribbon, a step's glyph and its body.
 */

const KIND_ICON: Record<StepKind, typeof Brain> = {
  think: Brain,
  say: MessageSquareText,
  answer: Sparkles,
  read: FileText,
  explore: FileSearch,
  edit: FilePen,
  run: Play,
  agent: Bot,
  question: MessageCircleQuestion,
  checkpoint: History,
  steer: CornerDownRight,
};

const KIND_TONE: Record<StepKind, string> = {
  think: 'text-purple',
  say: 'text-green',
  answer: 'text-green',
  read: 'text-text-secondary',
  explore: 'text-text-secondary',
  edit: 'text-orange',
  run: 'text-accent',
  agent: 'text-accent',
  question: 'text-accent',
  checkpoint: 'text-state-success',
  steer: 'text-accent',
};

export function StepIcon({ kind, size = 12 }: { kind: StepKind; size?: number }) {
  const Icon = KIND_ICON[kind];

  return <Icon size={size} aria-hidden="true" className={KIND_TONE[kind]} />;
}

/** The status mark: a spinner while it runs, a tick, a cross. */
export function StatusMark({ status, size = 11 }: { status: Step['status']; size?: number }) {
  if (status === 'running') {
    return <Loader size={size} aria-hidden="true" className="animate-spin text-accent motion-reduce:animate-none" />;
  }

  return status === 'failed' ? (
    <X size={size} aria-hidden="true" className="text-state-error" />
  ) : (
    <Check size={size} aria-hidden="true" className="text-state-success" />
  );
}

/**
 * The time ribbon: one bar, the whole turn, coloured by what each stretch was. The grey is the time
 * between steps - the model choosing, the network - which no other agent UI shows and which is often
 * most of a slow turn. A click on a stretch takes the view to its step.
 */
export function TimeRibbon({
  segments,
  totals,
  total,
  running,
  onPick,
  compact = false,
}: {
  segments: readonly Segment[];
  totals: Record<Phase, number>;
  total: number;
  running: boolean;
  onPick: (key: string) => void;
  compact?: boolean;
}) {
  const phases = (Object.keys(totals) as Phase[]).filter((phase) => totals[phase] >= 50);

  return (
    <div className="time-ribbon" data-time-ribbon>
      <div
        className={'relative w-full overflow-hidden rounded-full bg-bg-overlay ' + (compact ? 'h-[5px]' : 'h-[7px]')}
        title={strings.turns.flow.ribbon}
      >
        {segments.map((segment, index) => (
          <button
            key={`${segment.key}-${index}`}
            type="button"
            tabIndex={-1}
            className={'absolute top-0 h-full transition-opacity hover:opacity-70 ' + (segment.key === '' ? 'cursor-default' : 'cursor-pointer')}
            style={{
              left: `${segment.left * 100}%`,
              width: `max(${segment.width * 100}%, 2px)`,
              background: PHASE_COLOR[segment.phase],
              opacity: segment.phase === 'wait' ? 0.45 : 1,
            }}
            title={`${strings.turns.phase[segment.phase]} · ${strings.turns.thinking.seconds(segment.ms)}${segment.label === '' ? '' : ` · ${segment.label}`}`}
            onClick={() => {
              if (segment.key !== '') {
                onPick(segment.key);
              }
            }}
          />
        ))}
        {running ? <span className="ribbon-edge absolute right-0 top-0 h-full w-[18px]" aria-hidden="true" /> : null}
      </div>

      {compact ? null : (
        <div className="mt-[5px] flex flex-wrap items-center gap-x-[12px] gap-y-[2px] font-mono text-[10px] text-text-muted">
          {phases.map((phase) => (
            <span key={phase} className="inline-flex items-center gap-[5px]">
              <span className="h-[6px] w-[6px] rounded-full" style={{ background: PHASE_COLOR[phase], opacity: phase === 'wait' ? 0.6 : 1 }} aria-hidden="true" />
              {strings.turns.phase[phase]} {strings.turns.thinking.seconds(totals[phase])}
              <span className="text-text-faint">{Math.round((totals[phase] / total) * 100)}%</span>
            </span>
          ))}
        </div>
      )}
    </div>
  );
}

/** What a step opens to: the full thought, the words, the diff or output, the reads, the rewind. */
export function StepBody({ step, sessionId }: { step: Step; sessionId: string }) {
  const drawn = step.drawn;
  const box = useRef<HTMLDivElement>(null);
  const live = drawn.kind === 'thinking' && drawn.thinking.live;
  const text = drawn.kind === 'thinking' ? drawn.thinking.text : '';

  useEffect(() => {
    if (live && box.current !== null) {
      box.current.scrollTop = box.current.scrollHeight;
    }
  }, [live, text]);

  switch (drawn.kind) {
    case 'thinking':
      return (
        <div ref={box} className="max-h-[260px] overflow-y-auto whitespace-pre-wrap text-[12px] italic leading-[1.65] text-text-secondary">
          {drawn.thinking.text}
        </div>
      );
    case 'text':
      return drawn.final ? <Markdown text={drawn.text} /> : <Narration text={drawn.text} streaming={drawn.streaming} />;
    case 'tool':
      return drawn.tool.name === 'Question' ? (
        <ToolCard tool={drawn.tool} />
      ) : (
        <div className="overflow-hidden rounded-md border border-border-subtle bg-bg-raised">
          <ToolBody tool={drawn.tool} />
        </div>
      );
    case 'explore':
      return (
        <div className="rounded-md border border-border-subtle bg-bg-raised px-[10px] py-[5px]">
          {drawn.tools.map((tool, index) => (
            <div key={`${tool.startedAt}-${index}`} className="flex items-center gap-[8px] py-[2px] font-mono text-[11px]">
              <span className="w-[64px] shrink-0 text-text-primary">{tool.name}</span>
              <span className="min-w-0 flex-1 truncate text-text-secondary" title={tool.target}>
                {tool.target}
              </span>
              <span className={'shrink-0 text-[10px] ' + (tool.status === 'failed' ? 'text-state-error' : 'text-text-muted')}>{tool.meta}</span>
            </div>
          ))}
        </div>
      );
    case 'checkpoint':
      return <CheckpointRail sessionId={sessionId} checkpoints={[drawn.checkpoint]} />;
    case 'steer':
      return null;
  }
}

export function Sparkline({ samples, width = 72, height = 16 }: { samples: readonly number[]; width?: number; height?: number }) {
  if (samples.length < 2) {
    return <span className="inline-block" style={{ width, height }} aria-hidden="true" />;
  }

  const peak = Math.max(1, ...samples);
  const step = width / (samples.length - 1);
  const points = samples.map((value, index) => `${(index * step).toFixed(1)},${(height - 1 - (value / peak) * (height - 2)).toFixed(1)}`).join(' ');

  return (
    <svg width={width} height={height} viewBox={`0 0 ${width} ${height}`} aria-hidden="true" className="shrink-0 overflow-visible">
      <polyline points={`0,${height} ${points} ${width},${height}`} fill="var(--accent-subtle)" stroke="none" />
      <polyline points={points} fill="none" stroke="var(--accent)" strokeWidth="1.25" strokeLinejoin="round" strokeLinecap="round" />
    </svg>
  );
}

