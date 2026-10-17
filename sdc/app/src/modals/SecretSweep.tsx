import { KeyRound, ShieldAlert } from 'lucide-react';

import { strings } from '../strings';
import { moveSecretToKeychain } from '../store/intents';
import { findingId, useSecretSweep } from '../store/secretSweep';
import { BTN, BTN_PRIMARY, BTN_SECONDARY } from '../panels/ui/button';
import { Modal } from './Modal';

/**
 * The first-open secret sweep (Phase 0.4): shown once, when a folder is opened for the first time and already holds what
 * looks like a secret. Lists where (file and line - never the value) and offers, per finding, to move the value into the OS
 * keychain. A file whose *name* alone says it holds a secret has no value to move; it is listed so the person knows the agent
 * will ask before reading it.
 */
export function SecretSweep() {
  const { root, findings, truncated, moved, failed } = useSecretSweep();
  const close = useSecretSweep((state) => state.close);
  const t = strings.secretSweep;
  const open = root !== null && findings.length > 0;

  return (
    <Modal open={open} label={t.title} onClose={close} center className="w-[620px] max-w-[92vw]">
      <div className="secret-sweep flex max-h-[80vh] flex-col gap-[12px] p-[18px]" data-secret-sweep="">
        <div className="flex items-center gap-[8px] text-[15px] font-semibold text-text-primary">
          <ShieldAlert size={16} className="text-state-waiting" aria-hidden="true" />
          {t.title}
        </div>
        <p className="m-0 text-[12.5px] leading-[1.6] text-text-secondary">{t.body(findings.length)}</p>

        <ul className="m-0 flex min-h-0 list-none flex-col gap-[6px] overflow-y-auto p-0">
          {findings.map((finding) => {
            const id = findingId(finding);
            const done = moved[id];

            return (
              <li key={id} className="flex items-center gap-[10px] rounded-md border border-border-subtle bg-bg-raised px-[12px] py-[8px] text-[12px]" data-finding={id}>
                <KeyRound size={13} className="shrink-0 text-text-muted" aria-hidden="true" />
                <span className="min-w-0 flex-1">
                  <span className="block truncate font-mono text-text-primary">{finding.line > 0 ? `${finding.file}:${finding.line}` : finding.file}</span>
                  <span className="block truncate text-text-muted">{finding.what}</span>
                  {failed[id] ? <span className="block text-state-error">{failed[id]}</span> : null}
                  {done ? <span className="block text-state-success">{t.movedTo(done)}</span> : null}
                </span>
                {finding.byName || done ? null : (
                  <button type="button" className={BTN + ' ' + BTN_SECONDARY} onClick={() => void moveSecretToKeychain(finding)}>
                    {t.move}
                  </button>
                )}
              </li>
            );
          })}
        </ul>

        {truncated ? <p className="m-0 text-[11.5px] text-text-muted">{t.truncated}</p> : null}
        <p className="m-0 text-[11.5px] leading-[1.55] text-text-muted">{t.rotate}</p>

        <div className="flex justify-end">
          <button type="button" className={BTN + ' ' + BTN_PRIMARY} onClick={close}>
            {t.close}
          </button>
        </div>
      </div>
    </Modal>
  );
}
