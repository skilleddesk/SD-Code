import { Activity } from 'lucide-react';
import { useEffect, useState } from 'react';

import type { TraceStep } from '../../../../protocol/types';
import { strings } from '../../strings';
import { traceList } from '../../store/kernelIntents';
import { useSessionsStore } from '../../store/sessions';
import { Badge, type BadgeTone } from '../ui/Badge';

/**
 * The Step Trace tab (P0-12): one row per model call of this chat, newest first - which model, under which contract,
 * how long it waited for the first token and then wrote, what it cost in tokens, and whether Verify passed.
 *
 * The timeline is one bar per step, in two parts that share the step's width: the wait for the first token (muted) and
 * the writing (accent). Bars are scaled to the slowest step on screen, so the eye finds where the time went without
 * reading a number; the numbers are beside it for anyone who wants them. Everything here is read from what the daemon
 * measured (`trace.list`) - the tab computes nothing it was not told.
 */
const t = strings.rightPanel.trace;

const VERIFY_TONE: Record<string, BadgeTone> = { PASS: 'success', FAIL: 'warning', UNPROVEN: 'muted' };

function Row({ step, slowest }: { step: TraceStep; slowest: number }) {
  const width = slowest === 0 ? 0 : Math.max(2, (step.wallMs / slowest) * 100);
  const waiting = step.wallMs === 0 ? 0 : Math.min(100, (step.ttftMs / step.wallMs) * 100);
  const verify = step.verify === null ? null : (t.verify as Record<string, string>)[step.verify] ?? step.verify;

  return (
    <li className="trace-step mb-[8px] rounded-md border border-border-subtle bg-bg-raised p-[10px]" data-trace-step={step.stepId}>
      <div className="flex items-center gap-[6px]">
        <span className="font-mono text-[11px] text-text-primary">#{step.step}</span>
        <span className="min-w-0 truncate font-mono text-[11px] text-text-secondary" title={step.modelKey}>
          {step.modelKey}
        </span>
        <span className="ml-auto flex shrink-0 gap-[4px]">
          <Badge tone="muted" title={step.mode}>
            {step.mode}
          </Badge>
          {verify === null ? null : <Badge tone={VERIFY_TONE[step.verify ?? ''] ?? 'neutral'}>{verify}</Badge>}
        </span>
      </div>

      <div
        className="mt-[8px] h-[6px] rounded-full bg-bg-input"
        role="img"
        aria-label={`${t.timelineLabel}: ${t.wall(step.wallMs)}`}
      >
        <div className="flex h-full overflow-hidden rounded-full" style={{ width: `${width}%` }}>
          <div className="h-full bg-text-faint" style={{ width: `${waiting}%` }} />
          <div className="h-full flex-1 bg-accent" />
        </div>
      </div>

      <div className="mt-[6px] flex flex-wrap gap-x-[10px] gap-y-[2px] font-mono text-[10px] text-text-muted">
        <span>{t.wall(step.wallMs)}</span>
        <span>{t.ttft(step.ttftMs)}</span>
        <span>{t.tokens(step.tokensIn, step.tokensOut)}</span>
        {step.tokPerS > 0 ? <span>{t.rate(step.tokPerS)}</span> : null}
        {step.thinkingTokens > 0 ? <span>{t.thinking(step.thinkingTokens)}</span> : null}
        {step.costUsd === null ? null : <span>${step.costUsd.toFixed(4)}</span>}
        {step.escalationReason === null ? null : <span>{step.escalationReason}</span>}
      </div>
    </li>
  );
}

export function TraceTab() {
  const sessionId = useSessionsStore((state) => state.activeTab);
  const [steps, setSteps] = useState<TraceStep[] | null>(null);

  useEffect(() => {
    let live = true;
    const load = (): void => {
      void traceList(sessionId).then((rows) => {
        if (live && rows !== null) {
          setSteps(rows);
        }
      });
    };

    load();

    /* A turn in flight adds rows; reading again every few seconds keeps the timeline current without a subscription. */
    const timer = setInterval(load, 4000);

    return () => {
      live = false;
      clearInterval(timer);
    };
  }, [sessionId]);

  const newestFirst = [...(steps ?? [])].reverse();
  const slowest = Math.max(0, ...newestFirst.map((step) => step.wallMs));

  return (
    <div className="trace-tab px-[12px] py-[10px]">
      <h3 className="mb-[8px] flex items-center gap-[6px] text-[10px] font-bold uppercase tracking-[.12em] text-text-muted">
        <Activity size={12} aria-hidden="true" />
        {t.title}
      </h3>

      {steps === null || steps.length === 0 ? (
        <p className="text-[12px] text-text-muted">{t.empty}</p>
      ) : (
        <ol className="m-0 list-none p-0">
          {newestFirst.map((step) => (
            <Row key={step.stepId} step={step} slowest={slowest} />
          ))}
        </ol>
      )}
    </div>
  );
}
