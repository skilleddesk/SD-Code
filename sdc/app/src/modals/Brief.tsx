import { FileText, LoaderCircle, Sunrise } from 'lucide-react';

import { openOutside } from '../lib/external';
import { strings } from '../strings';
import { closeBrief, loadBrief, loadClientReport, setDaily, useBrief } from '../store/brief';
import { BTN, BTN_PRIMARY, BTN_SECONDARY } from '../panels/ui/button';
import { Modal } from './Modal';

/**
 * The morning brief (Phase 4.8): one page across every site and project, and the weekly client report. The pages are built by the daemon
 * from what SDC recorded and shown here as they are (a sandboxed frame: no scripts). The daily notification is off until switched on.
 */
export function Brief() {
  const { open, busy, kind, headline, html, path, error, daily } = useBrief();
  const t = strings.brief;

  return (
    <Modal open={open} label={t.title} onClose={closeBrief} center className="flex max-h-[92vh] w-[min(860px,96vw)] flex-col overflow-hidden">
      <div className="flex flex-wrap items-center gap-[8px] border-b border-border-subtle px-[22px] py-[14px]">
        <Sunrise size={16} className="text-accent" aria-hidden="true" />
        <h2 className="m-0 text-[15px] font-semibold text-text-primary">{t.title}</h2>
        <div className="ml-auto flex flex-wrap items-center gap-[8px]">
          <button type="button" className={BTN + ' ' + (kind === 'brief' ? BTN_PRIMARY : BTN_SECONDARY)} onClick={() => void loadBrief()} disabled={busy} data-action="brief-today">
            {t.today}
          </button>
          <button type="button" className={BTN + ' ' + (kind === 'client' ? BTN_PRIMARY : BTN_SECONDARY)} onClick={() => void loadClientReport()} disabled={busy} data-action="brief-client">
            <FileText size={12} aria-hidden="true" />
            {t.client}
          </button>
        </div>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto px-[22px] py-[14px]">
        {busy ? (
          <p className="flex items-center gap-[8px] text-[12.5px] text-text-muted">
            <LoaderCircle size={13} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />
            {t.working}
          </p>
        ) : null}
        {error === null ? null : (
          <p className="text-[12.5px] text-state-error" role="alert">
            {error}
          </p>
        )}
        {headline === '' || busy ? null : <p className="m-0 mb-[8px] text-[13px] font-medium text-text-primary" data-brief-headline="">{headline}</p>}
        {html === '' || busy ? null : <iframe title={t.title} sandbox="" srcDoc={html} className="h-[56vh] w-full rounded-md border border-border-subtle bg-white" />}
      </div>

      <div className="flex flex-wrap items-center gap-[10px] border-t border-border-subtle px-[22px] py-[12px] text-[12px] text-text-secondary">
        <label className="flex items-center gap-[6px]">
          <input type="checkbox" role="switch" checked={daily} onChange={(event) => void setDaily(event.target.checked)} />
          {t.daily}
        </label>
        <span className="text-text-faint">{t.dailyHelp}</span>
        {path === '' ? null : (
          <button type="button" className={BTN + ' ' + BTN_SECONDARY + ' ml-auto'} onClick={() => void openOutside(path)}>
            {t.open}
          </button>
        )}
      </div>
    </Modal>
  );
}
