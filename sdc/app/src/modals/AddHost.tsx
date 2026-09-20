import { useState } from 'react';
import { Laptop, Plug, Server } from 'lucide-react';

import { strings } from '../strings';
import { addHost } from '../store/intents';
import { useOverlayStore } from '../store/overlays';
import { BTN, BTN_PRIMARY, BTN_SECONDARY } from '../panels/ui/button';
import { Modal } from './Modal';

/**
 * `#addHostBd` - Add a host (spec section 9.12).
 *
 * Two shapes, one dialog: `Local` (this computer, always already connected) and `SSH / VPS`
 * (`user@host`), which reveals the target and the optional label. The footer is the spec's
 * `Cancel` / `Connect`, with Connect as the primary.
 *
 * Submitting is not a fake: the dialog hands the target to `intents.addHost()`, the daemon appends
 * `HostStatus { status: 'connecting' }`, and 1.4 seconds later the *same* host turns `connected`
 * and gains a welcome chat. This component never sets a status - it only stops accepting input
 * while its own request is in flight, which is what the spinner and the disabled button mean.
 */
export function AddHost() {
  const open = useOverlayStore((state) => state.addHostOpen);
  const close = useOverlayStore((state) => state.closeAddHost);
  const [type, setType] = useState<'local' | 'ssh'>('local');
  const [target, setTarget] = useState('');
  const [label, setLabel] = useState('');
  const [busy, setBusy] = useState(false);

  const submit = (): void => {
    setBusy(true);

    void addHost({
      type,
      target: target.trim(),
      label: label.trim(),
    }).then(() => {
      setBusy(false);
      close();
      setTarget('');
      setLabel('');
      setType('local');
    });
  };

  return (
    <Modal open={open} label={strings.addHost.title} onClose={close} center className="addhost-dlg">
      <div className="flex items-start gap-[12px] border-b border-border-subtle px-[18px] py-[16px]">
        <div className="grid h-[34px] w-[34px] shrink-0 place-items-center rounded-md bg-accent-subtle text-accent">
          <Server size={18} aria-hidden="true" />
        </div>
        <div>
          <div className="text-[14px] font-semibold text-text-primary">{strings.addHost.title}</div>
          <div className="mt-[2px] text-[12px] text-text-muted">{strings.addHost.sub}</div>
        </div>
      </div>

      <div className="px-[18px] py-[16px]">
        <div className="flex gap-[8px]">
          {(['local', 'ssh'] as const).map((option) => {
            const Icon = option === 'local' ? Laptop : Server;
            const selected = option === type;

            return (
              <button
                key={option}
                type="button"
                data-host-type={option}
                aria-pressed={selected}
                className={
                  'host-type-option flex flex-1 flex-col items-start gap-[4px] rounded-md border px-[12px] py-[10px] text-left transition-all duration-fast ease-ease ' +
                  (selected
                    ? 'selected border-accent bg-accent-subtle'
                    : 'border-border-default bg-bg-raised hover:border-border-strong')
                }
                onClick={() => setType(option)}
              >
                <span className="flex items-center gap-[6px] text-[12.5px] font-medium text-text-primary">
                  <Icon size={16} aria-hidden="true" />
                  {strings.addHost.types[option].label}
                </span>
                <span className="font-mono text-[10.5px] text-text-muted">
                  {strings.addHost.types[option].desc}
                </span>
              </button>
            );
          })}
        </div>

        {type === 'ssh' ? (
          <div className="mt-[14px] flex flex-col gap-[12px]">
            <label className="flex flex-col gap-[5px]">
              <span className="text-[11.5px] font-medium text-text-secondary">
                {strings.addHost.sshTarget}
              </span>
              <input
                type="text"
                id="sshTarget"
                className="rounded-md border border-border-default bg-bg-input px-[10px] py-[7px] font-mono text-[12.5px] text-text-primary placeholder:text-text-muted focus:border-border-focus"
                placeholder={strings.addHost.sshTargetPlaceholder}
                value={target}
                onChange={(event) => setTarget(event.target.value)}
              />
            </label>

            <label className="flex flex-col gap-[5px]">
              <span className="text-[11.5px] font-medium text-text-secondary">
                {strings.addHost.labelField}
              </span>
              <input
                type="text"
                id="sshLabel"
                className="rounded-md border border-border-default bg-bg-input px-[10px] py-[7px] text-[12.5px] text-text-primary placeholder:text-text-muted focus:border-border-focus"
                placeholder={strings.addHost.labelPlaceholder}
                value={label}
                onChange={(event) => setLabel(event.target.value)}
              />
            </label>
          </div>
        ) : null}
      </div>

      <div className="flex items-center gap-[8px] border-t border-border-subtle px-[18px] py-[12px]">
        <button type="button" className={BTN + ' ' + BTN_SECONDARY} onClick={close}>
          {strings.addHost.cancel}
        </button>

        <div className="flex-1" />

        <button
          type="button"
          className={BTN + ' ' + BTN_PRIMARY}
          id="addHostSubmit"
          disabled={busy}
          onClick={submit}
        >
          {busy ? <span className="spinner" aria-hidden="true" /> : <Plug size={12} aria-hidden="true" />}
          {strings.addHost.connect}
        </button>
      </div>
    </Modal>
  );
}
