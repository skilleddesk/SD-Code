import { Keyboard } from 'lucide-react';

import { commandsByGroup } from '../commands/registry';
import { Modal } from '../modals/Modal';
import { useOverlayStore } from '../store/overlays';
import { strings } from '../strings';

/**
 * The F1 keyboard reference (spec section 9.1).
 *
 * The list is `commandsByGroup()` - the registry, grouped and ordered - so this overlay is
 * *incapable* of drifting from the keyboard map: adding a shortcut is one entry in
 * `commands/registry.ts` and it appears here, in the palette and in the Settings → Keymap tab at
 * once. Principle P7 in one component.
 *
 * A row that fires while a field has focus is marked, because "Ctrl+K works while typing" is not
 * something a user should have to discover by accident.
 */
export function KeymapReference() {
  const open = useOverlayStore((state) => state.keymapOpen);
  const close = useOverlayStore((state) => state.closeKeymap);
  const sections = commandsByGroup();

  return (
    <Modal open={open} label={strings.keymap.title} onClose={close} center className="keymap-dlg">
      <div className="flex items-center gap-[10px] border-b border-border-subtle px-[18px] py-[14px]">
        <Keyboard size={16} aria-hidden="true" className="text-accent" />
        <div className="flex-1">
          <div className="text-[13.5px] font-semibold text-text-primary">{strings.keymap.title}</div>
          <div className="text-[11.5px] text-text-muted">{strings.keymap.sub}</div>
        </div>
        <button
          type="button"
          className="btn rounded-md border border-border-default bg-bg-raised px-[10px] py-[4px] text-[11.5px] text-text-primary hover:bg-bg-hover"
          onClick={close}
        >
          {strings.keymap.close}
        </button>
      </div>

      <div className="grid max-h-[70vh] grid-cols-2 gap-[16px] overflow-y-auto p-[18px] max-700:grid-cols-1">
        {sections.map((section) => (
          <section key={section.group} data-keymap-group={section.group}>
            <h3 className="mb-[6px] text-[10.5px] font-bold uppercase tracking-[0.09em] text-text-muted">
              {section.label}
            </h3>

            <div className="rounded-md border border-border-subtle">
              {section.rows.map((command) => (
                <div
                  key={command.id}
                  data-keymap-row={command.id}
                  className="flex items-center gap-[12px] border-b border-border-subtle px-[12px] py-[8px] text-[12.5px] last:border-b-0"
                >
                  <span className="flex-1 text-text-primary">{command.label}</span>
                  {command.inInput === true ? (
                    <span className="text-[10.5px] text-text-muted" title={strings.keymap.firesInInputs}>
                      ⏎
                    </span>
                  ) : null}
                  <span className="kbd inline-flex items-center rounded-[3px] border border-border-default border-b-2 bg-bg-base px-[6px] py-[1px] font-mono text-[10.5px] text-text-secondary">
                    {command.hint ?? ''}
                  </span>
                </div>
              ))}
            </div>
          </section>
        ))}
      </div>
    </Modal>
  );
}
