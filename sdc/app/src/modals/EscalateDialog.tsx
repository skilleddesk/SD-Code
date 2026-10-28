import { ShieldCheck } from 'lucide-react';

import { strings } from '../strings';
import { cancelEscalate, sendEscalate, useEscalate } from '../store/escalate';
import { BTN, BTN_PRIMARY, BTN_SECONDARY } from '../panels/ui/button';
import { Modal } from './Modal';

/**
 * "Send this step to [chain model]?" - with the estimated cost and, in full, what would be sent. It opens only after the person pressed
 * Escalate on a failed local step, and sends only when they press Send here.
 */
export function EscalateDialog() {
  const { sessionId, preview, busy, error } = useEscalate();
  const t = strings.escalate;
  const open = sessionId !== null && (preview !== null || error !== null);

  return (
    <Modal open={open} label={preview === null ? t.failed : t.title(preview.model)} onClose={cancelEscalate} center className="w-[640px] max-w-[94vw]">
      <div className="flex max-h-[84vh] flex-col gap-[10px] p-[18px]" data-escalate="">
        {preview === null ? (
          <p className="m-0 text-[12.5px] text-state-error" role="alert">
            {error}
          </p>
        ) : (
          <>
            <div className="flex items-center gap-[8px] text-[15px] font-semibold text-text-primary">
              <ShieldCheck size={16} className="text-accent" aria-hidden="true" />
              {t.title(preview.model)}
            </div>
            <p className="m-0 text-[12px] text-text-muted">{t.position(preview.position, preview.of)}</p>
            <p className="m-0 text-[12px] text-text-secondary">{t.what}</p>
            <pre className="m-0 max-h-[34vh] overflow-auto rounded-md border border-border-subtle bg-bg-input p-[10px] font-mono text-[11px] leading-[1.5] text-text-secondary" data-escalate-capsule="">
              {preview.capsule}
            </pre>
            {preview.files && preview.files.length > 0 ? <p className="m-0 text-[11.5px] text-text-muted">{t.files(preview.files.join(', '))}</p> : null}
            <p className="m-0 text-[11.5px] text-text-muted">{t.removed(preview.redactions?.length ?? 0)}</p>
            <p className="m-0 text-[11.5px] text-text-muted">{t.audit}</p>
            {error === null ? null : (
              <p className="m-0 text-[12px] text-state-error" role="alert">
                {error}
              </p>
            )}
            <div className="flex justify-end gap-[8px]">
              <button type="button" className={BTN + ' ' + BTN_SECONDARY} onClick={cancelEscalate} disabled={busy}>
                {t.cancel}
              </button>
              <button type="button" className={BTN + ' ' + BTN_PRIMARY} onClick={() => void sendEscalate()} disabled={busy} data-action="escalate-send">
                {t.send}
              </button>
            </div>
          </>
        )}
      </div>
    </Modal>
  );
}
