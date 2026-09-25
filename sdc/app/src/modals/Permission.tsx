import { ShieldCheck, TriangleAlert } from 'lucide-react';
import { useState } from 'react';

import { strings } from '../strings';
import { resolvePermission } from '../store/intents';
import { useAppStore } from '../store/store';
import { useOverlayStore } from '../store/overlays';
import { BTN, BTN_PRIMARY, BTN_SECONDARY } from '../panels/ui/button';
import { Modal } from './Modal';

/**
 * `#permissionBd` - the approval dialog (spec section 9.13).
 *
 * The content is not written here: it is the daemon's `PermissionRequested` event, folded by the
 * reducer into `state.permission`. That is why the dialog can name the file the engine wants to
 * touch, why the checkpoint note is real ("a checkpoint was saved before this turn"), and why the
 * four decisions go back to the daemon as an intent rather than being applied locally.
 *
 * RISK DECIDES THE DEFAULT. A `DANGEROUS` action defaults to **Deny** and a `MUTATING` one to
 * **Allow once**, which is the one place in the UI where a button's emphasis and the Enter key
 * disagree - and they disagree on purpose: the safe answer is the one Enter gives you.
 *
 * The keys themselves come from the registry (`permission.*`), so `Enter`, `A`, `Shift+A`, `S`,
 * `D` and `Esc` do here exactly what the F1 reference says.
 */
export function Permission() {
  const close = useOverlayStore((state) => state.closePermission);
  const permission = useAppStore((state) => state.permission);
  /* The question just answered, hidden until the daemon's `PermissionResolved` clears it for good. */
  const [answered, setAnswered] = useState<string | null>(null);

  if (permission === null) {
    return null;
  }

  /*
   * Open whenever a question is waiting. It used to open only when `askPermission` in this window called
   * `openPermission()` - so a question the daemon asked on its own (an agent about to edit a file) was
   * folded into the state and never shown, and the agent waited for an answer behind a closed dialog.
   */
  const open = answered !== permission.id;
  const dangerous = permission.risk === 'DANGEROUS';

  const decide = (decision: 'allow_once' | 'always_allow' | 'deny' | 'show_me'): void => {
    setAnswered(permission.id);
    close();
    void resolvePermission(permission.id, decision, permission.target);
  };

  return (
    /* Closing without choosing is `Deny` - the safe answer - so nothing is left waiting on a dialog that
       is no longer on screen. */
    <Modal open={open} label={permission.title} onClose={() => decide('deny')} center className="permission-dlg">
      <div className="flex items-start gap-[12px] border-b border-border-subtle px-[18px] py-[16px]">
        <div
          className={
            'grid h-[34px] w-[34px] shrink-0 place-items-center rounded-md ' +
            (dangerous ? 'bg-red-subtle text-state-error' : 'bg-orange-subtle text-state-warning')
          }
        >
          <TriangleAlert size={18} aria-hidden="true" />
        </div>
        <div className="min-w-0 flex-1">
          <div className="text-[14px] font-semibold text-text-primary">{permission.title}</div>
          <div className="mt-[2px] text-[12px] text-text-muted">{permission.sub}</div>
          <span
            data-risk={permission.risk}
            className={
              'mt-[6px] inline-flex items-center rounded-full px-[7px] py-[1px] text-[10px] font-bold uppercase tracking-[0.08em] ' +
              (dangerous
                ? 'bg-red-subtle text-state-error'
                : 'bg-orange-subtle text-state-warning')
            }
          >
            {strings.permission.risk[permission.risk]}
          </span>
        </div>
      </div>

      <div className="px-[18px] py-[16px]">
        <div className="dialog-target rounded-md border border-border-subtle bg-bg-input px-[12px] py-[9px] font-mono text-[12px] text-text-primary">
          {permission.target}
        </div>

        <div className="dialog-explain mt-[12px] text-[12.5px] leading-[1.65] text-text-secondary">
          <strong className="text-text-primary">{strings.permission.explainStrong}</strong>{' '}
          {permission.explain}
        </div>

        <div className="dialog-note mt-[14px] flex items-start gap-[8px] rounded-md bg-accent-subtle px-[12px] py-[9px] text-[11.5px] text-text-secondary">
          <ShieldCheck size={14} aria-hidden="true" className="mt-[1px] shrink-0 text-accent" />
          {strings.permission.note}
        </div>
      </div>

      <div className="flex items-center gap-[8px] border-t border-border-subtle px-[18px] py-[12px]">
        <button
          type="button"
          className={BTN + ' ' + BTN_SECONDARY}
          data-permission-action="deny"
          onClick={() => decide('deny')}
        >
          {strings.permission.deny}
        </button>

        <div className="flex-1" />

        <button
          type="button"
          className={BTN + ' ' + BTN_SECONDARY + ' max-600:hidden'}
          data-permission-action="show_me"
          onClick={() => decide('show_me')}
        >
          {strings.permission.showMe}
        </button>

        {permission.risk === 'MUTATING' ? (
          <button
            type="button"
            className={BTN + ' ' + BTN_SECONDARY}
            data-permission-action="always_allow"
            onClick={() => decide('always_allow')}
          >
            {strings.permission.allowAlways}
          </button>
        ) : null}

        <button
          type="button"
          className={BTN + ' ' + BTN_PRIMARY}
          data-permission-action="allow_once"
          /* Enter's default: the accent button, except when the risk is DANGEROUS (spec 9.13). */
          onClick={() => decide('allow_once')}
        >
          {strings.permission.allowOnce}
        </button>
      </div>
    </Modal>
  );
}
