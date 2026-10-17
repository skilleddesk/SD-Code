import { CircleAlert, CircleCheckBig, CircleDashed, CircleX, LoaderCircle, ShieldCheck, Wand2 } from 'lucide-react';

import type { ShipCheck } from '../../../protocol/types';
import { strings } from '../strings';
import { sweepFolder } from '../store/intents';
import { useModelStore } from '../store/model';
import { closeShipSafe, runShipSafe, setShipUrl, shipRoot, useShipSafe } from '../store/shipSafe';
import { BTN, BTN_PRIMARY, BTN_SECONDARY, BTN_SM } from '../panels/ui/button';
import { Modal } from './Modal';

/**
 * Ship Safe (Phase 4.5): one press runs the five checks and answers in plain words, each with a Fix button. A check that could not run
 * says so (`Skipped`) - it is never shown as passed.
 */
const ICON = {
  pass: <CircleCheckBig size={15} className="shrink-0 text-state-success" aria-hidden="true" />,
  warn: <CircleAlert size={15} className="shrink-0 text-state-waiting" aria-hidden="true" />,
  fail: <CircleX size={15} className="shrink-0 text-state-error" aria-hidden="true" />,
  skipped: <CircleDashed size={15} className="shrink-0 text-text-faint" aria-hidden="true" />,
} as const;

export function ShipSafe() {
  const { open, running, report, error, url } = useShipSafe();
  const t = strings.shipSafe;

  const fix = (check: ShipCheck): void => {
    const action = check.fix;

    if (action === null) {
      return;
    }

    if (action.kind === 'prompt') {
      useModelStore.getState().setDraft(action.prompt);
      closeShipSafe();

      return;
    }

    const root = shipRoot();

    if (root !== null) {
      closeShipSafe();
      void sweepFolder(root, true);
    }
  };

  return (
    <Modal open={open} label={t.title} onClose={closeShipSafe} center className="flex max-h-[90vh] w-[min(720px,96vw)] flex-col overflow-hidden">
      <div className="flex flex-col gap-[10px] border-b border-border-subtle px-[22px] py-[16px]">
        <div className="flex items-center gap-[8px] text-[15px] font-semibold text-text-primary">
          <ShieldCheck size={16} className="text-accent" aria-hidden="true" />
          {t.title}
        </div>
        <p className="m-0 text-[12px] text-text-muted">{t.subtitle}</p>
        <div className="flex items-center gap-[8px]">
          <input
            className="min-w-0 flex-1 rounded-md border border-border-default bg-bg-input px-[9px] py-[5px] font-mono text-[11.5px] text-text-primary placeholder:text-text-faint"
            placeholder="https://staging.example.com"
            aria-label={t.url}
            value={url}
            onChange={(event) => setShipUrl(event.target.value)}
            spellCheck={false}
          />
          <button type="button" className={BTN + ' ' + BTN_PRIMARY} disabled={running} onClick={() => void runShipSafe()} data-action="ship-run">
            {running ? <LoaderCircle size={12} className="animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <ShieldCheck size={12} aria-hidden="true" />}
            {running ? t.running : report === null ? t.run : t.again}
          </button>
        </div>
        <p className="m-0 text-[11px] text-text-faint">{t.network}</p>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto px-[22px] py-[14px]">
        {error === null ? null : (
          <p className="text-[12.5px] text-state-error" role="alert">
            {error}
          </p>
        )}

        {report === null ? null : (
          <div className="flex flex-col gap-[10px]">
            <div className={'rounded-md border px-[12px] py-[9px] text-[12.5px] font-semibold ' + (report.verdict === 'pass' ? 'border-state-success bg-green-subtle text-state-success' : report.verdict === 'fail' ? 'border-state-error bg-red-subtle text-state-error' : 'border-state-waiting bg-orange-subtle text-state-waiting')} role="status" data-ship-verdict={report.verdict}>
              {t.verdict[report.verdict]}
            </div>
            {report.checks.map((check) => (
              <section key={check.id} className="rounded-md border border-border-subtle bg-bg-raised px-[12px] py-[10px]" data-ship-check={check.id} data-status={check.status}>
                <div className="flex items-center gap-[8px]">
                  {ICON[check.status]}
                  <span className="text-[12.5px] font-semibold text-text-primary">{t.names[check.id]}</span>
                  <span className="ml-auto text-[10.5px] uppercase tracking-[.08em] text-text-muted">{t.status[check.status]}</span>
                </div>
                <p className="m-0 mt-[4px] text-[12px] leading-[1.55] text-text-secondary">{check.summary}</p>
                {check.details.length === 0 ? null : (
                  <ul className="m-0 mt-[4px] list-disc pl-[18px] text-[11.5px] leading-[1.55] text-text-muted">
                    {check.details.map((line) => (
                      <li key={line}>{line}</li>
                    ))}
                  </ul>
                )}
                {check.fix === null ? null : (
                  <button type="button" className={BTN_SM + ' ' + BTN_SECONDARY + ' mt-[8px]'} onClick={() => fix(check)} data-action="ship-fix">
                    <Wand2 size={11} aria-hidden="true" />
                    {check.fix.label}
                  </button>
                )}
              </section>
            ))}
          </div>
        )}
      </div>
    </Modal>
  );
}
