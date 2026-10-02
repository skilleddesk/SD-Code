import { Globe, TriangleAlert } from 'lucide-react';

import { strings } from '../../strings';
import { cancelResearch, startResearch, useResearchUi } from '../../store/research';
import { BTN, BTN_PRIMARY, BTN_SECONDARY } from '../ui/button';

/**
 * The card a `/research` waits on (0.16.1): what will run, where, with which search service, how far it
 * may go, and about what it will cost - before anything is searched. `Start research` sends it;
 * `Cancel` hands the words back to the box.
 */
export function ResearchCard({ sessionId, onCancel }: { sessionId: string | null; onCancel: (text: string) => void }) {
  const pending = useResearchUi((state) => state.pending[sessionId ?? '']);

  if (pending === undefined) {
    return null;
  }

  const words = strings.research.card;
  const plan = pending.plan;
  const money = (usd: number | null | undefined): string | null => (usd === null || usd === undefined ? null : usd < 0.01 ? usd.toFixed(4) : usd.toFixed(2));
  const cost =
    plan === null
      ? null
      : plan.place === 'local'
        ? words.free
        : plan.place === 'cli'
          ? words.subscription
          : money(plan.estimate.usd) === null
            ? words.unknown
            : words.estimate(money(plan.estimate.usd) ?? '');

  const rows: [string, string][] =
    plan === null
      ? []
      : [
          [words.model, `${plan.model === '' ? plan.engine : plan.model} · ${words.place[plan.place]}${plan.localContext === null ? '' : ` · ${words.context(plan.localContext)}`}`],
          [words.search, `${plan.search} · ${words.limits(plan.limits.maxSearches, plan.limits.maxPages, plan.limits.maxMinutes)}`],
          [words.cost, plan.synthesis === null ? (cost ?? '') : `${cost ?? ''} · ${words.synthesis(plan.synthesis.model, money(plan.estimate.synthesisUsd))}`],
        ];

  return (
    <section
      className="research-card mb-[8px] rounded-md border border-border-default bg-bg-raised px-[14px] py-[11px] text-[12.5px]"
      aria-label={words.title}
    >
      <div className="mb-[8px] flex items-center gap-[8px]">
        <Globe size={14} className="text-accent" aria-hidden="true" />
        <span className="text-[11px] font-semibold uppercase tracking-[.08em] text-accent">{words.title}</span>
      </div>

      <p className="mb-[8px] text-text-primary">
        <span className="text-text-muted">{words.question}: </span>
        {pending.question}
      </p>

      {rows.length > 0 ? (
        <dl className="mb-[8px] grid grid-cols-[auto_1fr] gap-x-[12px] gap-y-[3px] text-[12px]">
          {rows.map(([label, value]) => (
            <div key={label} className="contents">
              <dt className="text-text-muted">{label}</dt>
              <dd className="min-w-0 break-words text-text-secondary">{value}</dd>
            </div>
          ))}
        </dl>
      ) : null}

      {plan !== null && plan.searchKeyMissing ? <Warning text={words.keyMissing(plan.search)} /> : null}
      {plan !== null && plan.ollamaRunning === false ? <Warning text={words.ollamaDown} /> : null}

      <div className="mt-[10px] flex items-center gap-[8px]">
        <button
          type="button"
          className={BTN + ' ' + BTN_PRIMARY}
          onClick={() => void startResearch(sessionId)}
        >
          {words.start}
        </button>
        <button
          type="button"
          className={BTN + ' ' + BTN_SECONDARY}
          onClick={() => {
            const text = cancelResearch(sessionId);

            if (text !== null) {
              onCancel(text);
            }
          }}
        >
          {words.cancel}
        </button>
      </div>
    </section>
  );
}

function Warning({ text }: { text: string }) {
  return (
    <p className="mb-[4px] flex items-start gap-[6px] text-[12px] text-state-warning">
      <TriangleAlert size={13} className="mt-[2px] shrink-0" aria-hidden="true" />
      <span>{text}</span>
    </p>
  );
}
