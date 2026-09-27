import { Link2, ShieldCheck } from 'lucide-react';
import { useEffect, useState } from 'react';

import type { LedgerCheck, Policy } from '../../../protocol/types';
import { strings } from '../strings';
import { loadPolicy, savePolicy, verifyLedger } from '../store/kernelIntents';
import { useKernelUi } from '../store/kernelUi';
import { useSessionsStore } from '../store/sessions';
import { useAppStore } from '../store/store';
import { Modal } from '../modals/Modal';
import { BTN, BTN_PRIMARY, BTN_SECONDARY, BTN_SM } from '../panels/ui/button';

/**
 * **Policy** (0.12, the Trust Kernel's policy engine): what an AI may do in the open chat's folder, as the
 * folder's own `.sdc/policy.toml`. The form and the file are one thing - saving the form writes the file,
 * and the file can be edited as text. The defaults (protected secrets, keys, backups and `wp-config.php`;
 * commands that reach past the folder) always apply; a project adds to them.
 *
 * The audit ledger's check sits here too: whether the hash chain of everything every AI did is intact.
 */
const k = strings.kernel.policy;
const lines = (text: string): string[] => text.split('\n').map((line) => line.trim()).filter((line) => line !== '');

export function PolicyEditor() {
  const open = useKernelUi((state) => state.policyOpen);
  const close = useKernelUi((state) => state.closePolicy);
  const { activeTab: sessionId } = useSessionsStore();
  const violations = useAppStore((state) => state.kernel.violations);
  const [loaded, setLoaded] = useState<{ policy: Policy; text: string; exists: boolean; path: string | null; root: string | null; defaults: { protected: string[]; alwaysAsk: string[] } } | null>(null);
  const [policy, setPolicy] = useState<Policy | null>(null);
  const [lists, setLists] = useState({ protectedPaths: '', alwaysAsk: '', denyCommands: '' });
  const [raw, setRaw] = useState<string | null>(null);
  const [ledger, setLedger] = useState<LedgerCheck | null>(null);

  useEffect(() => {
    if (!open) {
      return;
    }

    setRaw(null);
    setLedger(null);
    void loadPolicy(sessionId).then((result) => {
      setLoaded(result);
      setPolicy(result?.policy ?? null);

      if (result !== null) {
        const defaults = new Set([...result.defaults.protected, ...result.defaults.alwaysAsk]);

        setLists({
          protectedPaths: result.policy.protectedPaths.filter((path) => !defaults.has(path)).join('\n'),
          alwaysAsk: result.policy.alwaysAsk.filter((line) => !defaults.has(line)).join('\n'),
          denyCommands: result.policy.denyCommands.join('\n'),
        });
      }
    });
  }, [open, sessionId]);

  const noFolder = loaded !== null && loaded.root === null;

  const save = (): void => {
    if (sessionId === null || policy === null) {
      return;
    }

    const input =
      raw !== null
        ? { text: raw }
        : {
            policy: {
              ...policy,
              protectedPaths: [...(loaded?.defaults.protected ?? []), ...lines(lists.protectedPaths)],
              alwaysAsk: [...(loaded?.defaults.alwaysAsk ?? []), ...lines(lists.alwaysAsk)],
              denyCommands: lines(lists.denyCommands),
            },
          };

    void savePolicy(sessionId, input).then((saved) => {
      if (saved !== null) {
        setPolicy(saved);
        setRaw(null);
      }
    });
  };

  return (
    <Modal open={open} label={k.title} onClose={close} center className="flex max-h-[92vh] w-[min(720px,96vw)] flex-col overflow-hidden">
      <div className="border-b border-border-subtle px-[22px] py-[16px]">
        <h2 className="flex items-center gap-[8px] text-[15px] font-semibold text-text-primary">
          <ShieldCheck size={16} className="text-accent" aria-hidden="true" />
          {k.title}
        </h2>
        <p className="mt-[2px] font-mono text-[11px] text-text-muted" dir="ltr">
          {loaded?.path ?? k.noFolder}
        </p>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto px-[22px] py-[16px] text-[12.5px]">
        {loaded === null || policy === null ? (
          <p className="text-text-muted">{sessionId === null ? k.noChat : k.loading}</p>
        ) : noFolder ? (
          <p className="text-text-muted">{k.noFolder}</p>
        ) : (
          <div className="flex flex-col gap-[14px]">
            <p className="text-text-secondary">{loaded.exists ? k.fromFile : k.defaults}</p>
            {policy.error === null ? null : <p className="rounded-md bg-red-subtle px-[10px] py-[6px] text-state-error">{policy.error}</p>}

            {raw !== null ? (
              <label className="flex flex-col gap-[4px]">
                <span className="text-[11px] text-text-muted">{k.rawHint}</span>
                <textarea rows={16} className="rounded-md border border-border-default bg-bg-input p-[10px] font-mono text-[11.5px]" dir="ltr" value={raw} onChange={(event) => setRaw(event.target.value)} />
              </label>
            ) : (
              <>
                <label className="flex items-start gap-[10px]">
                  <input type="checkbox" className="mt-[3px] accent-[var(--accent)]" checked={policy.production} onChange={(event) => setPolicy({ ...policy, production: event.target.checked })} />
                  <span>
                    <span className="font-medium text-text-primary">{k.production}</span>
                    <span className="block text-[11.5px] text-text-muted">{k.productionHelp}</span>
                  </span>
                </label>
                <label className="flex items-start gap-[10px]">
                  <input
                    type="checkbox"
                    className="mt-[3px] accent-[var(--accent)]"
                    checked={policy.privacy === 'local-only'}
                    onChange={(event) => setPolicy({ ...policy, privacy: event.target.checked ? 'local-only' : 'any' })}
                  />
                  <span>
                    <span className="font-medium text-text-primary">{k.privacy}</span>
                    <span className="block text-[11.5px] text-text-muted">{k.privacyHelp}</span>
                  </span>
                </label>
                <label className="flex items-start gap-[10px]">
                  <input type="checkbox" className="mt-[3px] accent-[var(--accent)]" checked={policy.autoRollback} onChange={(event) => setPolicy({ ...policy, autoRollback: event.target.checked })} />
                  <span>
                    <span className="font-medium text-text-primary">{k.autoRollback}</span>
                    <span className="block text-[11.5px] text-text-muted">{k.autoRollbackHelp}</span>
                  </span>
                </label>
                <div className="grid grid-cols-2 gap-[10px]">
                  <label className="flex flex-col gap-[4px] text-[11.5px] text-text-secondary">
                    {k.maxFiles}
                    <input
                      type="number"
                      min="0"
                      className="rounded-md border border-border-default bg-bg-input px-[8px] py-[5px] font-mono"
                      value={policy.maxFilesPerTurn}
                      onChange={(event) => setPolicy({ ...policy, maxFilesPerTurn: Math.max(0, Number.parseInt(event.target.value || '0', 10)) })}
                    />
                  </label>
                  <label className="flex flex-col gap-[4px] text-[11.5px] text-text-secondary">
                    {k.maxTurnUsd}
                    <input
                      type="number"
                      min="0"
                      step="0.05"
                      className="rounded-md border border-border-default bg-bg-input px-[8px] py-[5px] font-mono"
                      placeholder={strings.kernel.cost.noLimit}
                      value={policy.maxTurnUsd ?? ''}
                      onChange={(event) => {
                        const value = Number.parseFloat(event.target.value);

                        setPolicy({ ...policy, maxTurnUsd: Number.isFinite(value) && value > 0 ? value : null });
                      }}
                    />
                  </label>
                </div>
                {(['protectedPaths', 'alwaysAsk', 'denyCommands'] as const).map((key) => (
                  <label key={key} className="flex flex-col gap-[4px]">
                    <span className="font-medium text-text-primary">{k.lists[key]}</span>
                    <span className="text-[11px] text-text-muted">{k.listHelp[key]}</span>
                    <textarea
                      rows={3}
                      className="rounded-md border border-border-default bg-bg-input p-[8px] font-mono text-[11.5px]"
                      dir="ltr"
                      value={lists[key]}
                      onChange={(event) => setLists({ ...lists, [key]: event.target.value })}
                    />
                  </label>
                ))}
                <p className="text-[11px] text-text-muted">{k.alwaysOn((loaded?.defaults.protected ?? []).join(', '))}</p>
              </>
            )}

            <button type="button" className={BTN_SM + ' ' + BTN_SECONDARY + ' self-start'} onClick={() => setRaw(raw === null ? loaded.text : null)}>
              {raw === null ? k.editText : k.editForm}
            </button>
          </div>
        )}

        <section className="mt-[20px] border-t border-border-subtle pt-[14px]" aria-label={k.ledgerTitle}>
          <h3 className="mb-[6px] flex items-center gap-[6px] text-[10px] font-semibold uppercase tracking-[.1em] text-text-muted">
            <Link2 size={11} aria-hidden="true" />
            {k.ledgerTitle}
          </h3>
          <p className="mb-[8px] text-[11.5px] text-text-muted">{k.ledgerHelp}</p>
          <div className="flex items-center gap-[10px]">
            <button type="button" className={BTN_SM + ' ' + BTN_SECONDARY} onClick={() => void verifyLedger().then(setLedger)}>
              {k.verifyLedger}
            </button>
            {ledger === null ? null : (
              <span className={ledger.intact ? 'text-state-success' : 'text-state-error'}>
                {ledger.intact ? strings.kernel.proof.intact(ledger.entries) : `${strings.kernel.proof.broken(ledger.brokenAt ?? 0)} ${ledger.reason ?? ''}`}
              </span>
            )}
          </div>
          {violations.length === 0 ? null : (
            <div className="mt-[12px]">
              <div className="mb-[4px] text-[10px] font-semibold uppercase tracking-[.1em] text-text-muted">{k.violations}</div>
              <ul className="flex flex-col gap-[3px] text-[11.5px]">
                {[...violations].reverse().slice(0, 12).map((violation, index) => (
                  <li key={index} className="text-state-error">
                    <span className="font-mono">{violation.rule}</span> · {violation.sentence}
                  </li>
                ))}
              </ul>
            </div>
          )}
        </section>
      </div>

      <div className="flex justify-end gap-[8px] border-t border-border-subtle px-[22px] py-[12px]">
        <button type="button" className={BTN + ' ' + BTN_SECONDARY} onClick={close}>
          {k.close}
        </button>
        <button type="button" className={BTN + ' ' + BTN_PRIMARY} disabled={policy === null || noFolder || sessionId === null} onClick={save}>
          {k.save}
        </button>
      </div>
    </Modal>
  );
}
