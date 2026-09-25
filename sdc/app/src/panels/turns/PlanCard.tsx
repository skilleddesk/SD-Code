import { Check, Circle, ListChecks, LoaderCircle } from 'lucide-react';

import { strings } from '../../strings';
import type { PlanStepData } from './types';

/**
 * The agent's plan card (v4, docs/ROADMAP-v4.md §3 #5).
 *
 * On a long agent run the question a person has is "what is it doing, and how much is left?". The
 * agent answers it with `update_plan`, and this card draws the newest answer: every step, the one in
 * progress spinning while the turn runs, the finished ones ticked and dimmed, and a count in the head.
 *
 * The steps are the model's own words, as sent - nothing here infers progress from tool calls, because
 * a card that guessed would be wrong exactly when it matters.
 */
export interface PlanCardProps {
  steps: readonly PlanStepData[];
  running: boolean;
}

export function PlanCard({ steps, running }: PlanCardProps) {
  if (steps.length === 0) {
    return null;
  }

  const done = steps.filter((step) => step.status === 'done').length;

  return (
    <section
      className="plan-card mb-[8px] overflow-hidden rounded-md border border-border-subtle bg-bg-raised"
      aria-label={strings.turns.plan.title}
    >
      <div className="plan-head flex items-center gap-[8px] bg-purple-subtle px-[13px] py-[8px] text-[11px]">
        <ListChecks size={13} className="text-purple" aria-hidden="true" />
        <span className="font-semibold uppercase tracking-[.08em] text-purple">{strings.turns.plan.title}</span>
        <span className="ml-auto font-mono tabular-nums text-text-muted">{strings.turns.plan.progress(done, steps.length)}</span>
      </div>

      <ol className="plan-steps flex flex-col gap-[5px] px-[13px] py-[9px]">
        {steps.map((step, index) => (
          <li
            key={`${index}-${step.text}`}
            className={
              'plan-step flex items-start gap-[9px] text-[12.5px] leading-[1.5] ' +
              (step.status === 'done'
                ? 'text-text-muted line-through decoration-border-strong'
                : step.status === 'in_progress'
                  ? 'font-medium text-text-primary'
                  : 'text-text-secondary')
            }
            data-status={step.status}
          >
            <span className="mt-[2px] flex h-[14px] w-[14px] shrink-0 items-center justify-center" aria-hidden="true">
              {step.status === 'done' ? (
                <Check size={13} className="text-state-success" />
              ) : step.status === 'in_progress' ? (
                <LoaderCircle
                  size={13}
                  className={'text-accent ' + (running ? 'animate-spin motion-reduce:animate-none' : '')}
                />
              ) : (
                <Circle size={11} className="text-text-faint" />
              )}
            </span>
            <span className="min-w-0 flex-1 break-words">{step.text}</span>
            <span className="sr-only">{strings.turns.plan.status[step.status]}</span>
          </li>
        ))}
      </ol>
    </section>
  );
}
