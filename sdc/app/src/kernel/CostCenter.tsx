import { useEffect, useState } from 'react';

import type { CostSummary } from '../../../protocol/types';
import { strings } from '../strings';
import { usd } from '../store/kernel';
import { costSummary, setBudgets } from '../store/kernelIntents';
import { useKernelUi } from '../store/kernelUi';
import { useModelStore } from '../store/model';
import { Modal } from '../modals/Modal';
import { BTN, BTN_PRIMARY } from '../panels/ui/button';

/**
 * **The Cost center** (0.12, the cost governor's screen): what AI has cost - by day, by model, by project
 * and by site - the budgets that stop a turn before it overspends, and what was saved, which is only ever
 * a measured number against a baseline model the person chose (P4: no "you saved 40%" from a guess).
 */
const k = strings.kernel.cost;

function Stat({ label, value, note }: { label: string; value: string; note?: string }) {
  return (
    <div className="rounded-md border border-border-subtle bg-bg-raised px-[12px] py-[9px]">
      <div className="text-[10px] font-semibold uppercase tracking-[.1em] text-text-muted">{label}</div>
      <div className="mt-[2px] text-[18px] font-semibold tabular-nums text-text-primary">{value}</div>
      {note === undefined ? null : <div className="text-[10.5px] text-text-muted">{note}</div>}
    </div>
  );
}

export function CostCenter() {
  const open = useKernelUi((state) => state.costOpen);
  const close = useKernelUi((state) => state.closeCost);
  const catalog = useModelStore((state) => state.catalog);
  const [summary, setSummary] = useState<CostSummary | null>(null);
  const [draft, setDraft] = useState<Record<string, string>>({});
  const [baseline, setBaseline] = useState('');

  useEffect(() => {
    if (!open) {
      return;
    }

    void costSummary().then((loaded) => {
      setSummary(loaded);

      if (loaded !== null) {
        setDraft(Object.fromEntries(['turn', 'chat', 'day', 'month'].map((key) => [key, String(loaded.budgets[key as 'turn'] ?? '')])));
        setBaseline(loaded.baseline ?? '');
      }
    });
  }, [open]);

  const max = Math.max(0.0001, ...(summary?.days ?? []).map((day) => day.usd));

  return (
    <Modal open={open} label={k.title} onClose={close} center className="flex max-h-[92vh] w-[min(760px,96vw)] flex-col overflow-hidden">
      <div className="border-b border-border-subtle px-[22px] py-[16px]">
        <h2 className="text-[15px] font-semibold text-text-primary">{k.title}</h2>
        <p className="mt-[2px] text-[12px] text-text-muted">{k.subtitle}</p>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto px-[22px] py-[16px]">
        {summary === null ? (
          <p className="text-[12.5px] text-text-muted">{k.loading}</p>
        ) : (
          <div className="flex flex-col gap-[18px]">
            <div className="grid grid-cols-2 gap-[10px] md:grid-cols-4">
              <Stat label={k.today} value={usd(summary.spent.day)} />
              <Stat label={k.month} value={usd(summary.spent.month)} />
              <Stat
                label={k.saved}
                value={summary.baseline === null ? '—' : usd(summary.savedUsd)}
                note={summary.baseline === null ? k.noBaseline : k.savedNote(summary.measuredTurns)}
              />
              <Stat label={k.measured} value={`${summary.measuredTurns} / ${summary.measuredTurns + summary.unmeasuredTurns}`} note={k.measuredNote} />
            </div>

            <section aria-label={k.byDay}>
              <h3 className="mb-[8px] text-[10px] font-semibold uppercase tracking-[.1em] text-text-muted">{k.byDay}</h3>
              {summary.days.length === 0 ? <p className="text-[12px] text-text-muted">{k.none}</p> : null}
              <ul className="flex flex-col gap-[3px]">
                {summary.days.slice(-14).map((day) => (
                  <li key={day.day} className="flex items-center gap-[8px] text-[11px]">
                    <span className="w-[78px] shrink-0 font-mono text-text-muted">{day.day}</span>
                    <span className="h-[8px] rounded-sm bg-accent" style={{ width: `${Math.max(2, (day.usd / max) * 100)}%`, maxWidth: '70%' }} aria-hidden="true" />
                    <span className="font-mono tabular-nums text-text-secondary">{usd(day.usd)}</span>
                  </li>
                ))}
              </ul>
            </section>

            <section aria-label={k.byModel}>
              <h3 className="mb-[8px] text-[10px] font-semibold uppercase tracking-[.1em] text-text-muted">{k.byModel}</h3>
              <table className="w-full text-[11.5px]">
                <thead>
                  <tr className="text-left text-text-muted">
                    <th className="py-[3px] font-medium">{k.model}</th>
                    <th className="py-[3px] text-right font-medium">{k.turns}</th>
                    <th className="py-[3px] text-right font-medium">{k.tokens}</th>
                    <th className="py-[3px] text-right font-medium">{k.spend}</th>
                  </tr>
                </thead>
                <tbody>
                  {[...summary.models].sort((a, b) => b.usd - a.usd).map((row) => (
                    <tr key={row.model} className="border-t border-border-subtle">
                      <td className="py-[4px] font-mono text-text-primary" dir="ltr">{row.model}</td>
                      <td className="py-[4px] text-right tabular-nums">{row.turns}</td>
                      <td className="py-[4px] text-right font-mono tabular-nums text-text-muted">{`${Math.round(row.inputTokens / 1000)}k / ${Math.round(row.outputTokens / 1000)}k`}</td>
                      <td className="py-[4px] text-right font-mono tabular-nums">{usd(row.usd)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </section>

            {summary.projects.length + summary.sites.length === 0 ? null : (
              <section aria-label={k.byProject}>
                <h3 className="mb-[8px] text-[10px] font-semibold uppercase tracking-[.1em] text-text-muted">{k.byProject}</h3>
                <ul className="flex flex-col gap-[3px] text-[11.5px]">
                  {[...summary.projects.map((row) => ({ name: row.root, usd: row.usd })), ...summary.sites.map((row) => ({ name: `site ${row.siteId}`, usd: row.usd }))]
                    .sort((a, b) => b.usd - a.usd)
                    .map((row) => (
                      <li key={row.name} className="flex justify-between gap-[10px]">
                        <span className="truncate font-mono text-text-secondary" dir="ltr">{row.name}</span>
                        <span className="font-mono tabular-nums">{usd(row.usd)}</span>
                      </li>
                    ))}
                </ul>
              </section>
            )}

            <section aria-label={k.budgets}>
              <h3 className="mb-[4px] text-[10px] font-semibold uppercase tracking-[.1em] text-text-muted">{k.budgets}</h3>
              <p className="mb-[8px] text-[11.5px] text-text-muted">{k.budgetsNote}</p>
              <div className="grid grid-cols-2 gap-[10px] md:grid-cols-4">
                {(['turn', 'chat', 'day', 'month'] as const).map((key) => (
                  <label key={key} className="flex flex-col gap-[4px] text-[11px] text-text-secondary">
                    {k.budget[key]}
                    <input
                      type="number"
                      min="0"
                      step="0.01"
                      inputMode="decimal"
                      className="rounded-md border border-border-default bg-bg-input px-[8px] py-[5px] font-mono text-[12px] text-text-primary"
                      placeholder={k.noLimit}
                      value={draft[key] ?? ''}
                      onChange={(event) => setDraft({ ...draft, [key]: event.target.value })}
                    />
                  </label>
                ))}
              </div>
              <label className="mt-[10px] flex flex-col gap-[4px] text-[11px] text-text-secondary">
                {k.baseline}
                <span className="flex h-[30px] rounded-md border border-border-default bg-bg-input">
                  <select
                    className="h-full w-full bg-transparent px-[8px] text-[12px] text-text-primary outline-none [&>option]:bg-bg-overlay"
                    value={baseline}
                    onChange={(event) => setBaseline(event.target.value)}
                  >
                    <option value="">{k.noBaselineOption}</option>
                    {catalog
                      .filter((row) => row.cost !== '' && row.cost !== 'free' && row.cost !== 'subscription')
                      .map((row) => (
                        <option key={`${row.providerId}|${row.id}`} value={`${row.providerId}|${row.id}`}>
                          {`${row.providerLabel || row.providerId} · ${row.name || row.id} (${row.cost})`}
                        </option>
                      ))}
                  </select>
                </span>
              </label>
            </section>
          </div>
        )}
      </div>

      <div className="flex justify-end gap-[8px] border-t border-border-subtle px-[22px] py-[12px]">
        <button
          type="button"
          className={BTN + ' ' + BTN_PRIMARY}
          disabled={summary === null}
          onClick={() => {
            const value = (key: string): number | null => {
              const parsed = Number.parseFloat(draft[key] ?? '');

              return Number.isFinite(parsed) && parsed > 0 ? parsed : null;
            };

            void setBudgets({ turn: value('turn'), chat: value('chat'), day: value('day'), month: value('month'), baseline }).then((next) => {
              if (next !== null) {
                setSummary(next);
              }
            });
          }}
        >
          {k.save}
        </button>
      </div>
    </Modal>
  );
}
