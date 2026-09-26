import { useState } from 'react';
import { FolderPlus, Loader, Sparkles } from 'lucide-react';

import { strings } from '../strings';
import { pickFolder } from '../lib/picker';
import { scaffoldProject } from '../store/intents';
import { useOverlayStore } from '../store/overlays';
import { useAppStore } from '../store/store';
import { BTN, BTN_PRIMARY, BTN_SECONDARY } from '../panels/ui/button';
import { toast } from '../store/toast';
import { Modal } from './Modal';

/**
 * `Start from scratch` (0.9.0) - the whole beginning of a project as one dialog.
 *
 * Before this, an empty folder took four surfaces: a file manager to make it, `Open folder` to add it,
 * `+ New chat` to talk in it, and the prompt to say what to build. The report's sentence - *"scratch
 * thake kaj suru korbe tokhon sudu command dilai jano kora jai"* - is the spec: say where, say a name,
 * say what to build, and everything after the button is SDC's job (`project.scaffold`, then
 * `scaffoldProject` opens the chat and starts the agent).
 *
 * The host row exists because the folder can be made **on a VPS** exactly as locally - the daemon
 * routes `project.scaffold` through the host's signed-in connection. `Browse…` uses the native picker,
 * which can only see this machine, so it is offered for `local` only; for a host, `~/projects` works
 * (the daemon expands it).
 */
export function NewProject() {
  const open = useOverlayStore((state) => state.newProjectOpen);
  const close = useOverlayStore((state) => state.closeNewProject);
  const hosts = useAppStore((state) => state.hosts);

  const [hostId, setHostId] = useState('local');
  const [parent, setParent] = useState('');
  const [name, setName] = useState('');
  const [prompt, setPrompt] = useState('');
  const [busy, setBusy] = useState(false);

  const reset = (): void => {
    setParent('');
    setName('');
    setPrompt('');
    setHostId('local');
  };

  const shutdown = (): void => {
    close();
    reset();
  };

  const create = (): void => {
    if (name.trim() === '') {
      toast(strings.scaffold.needName);

      return;
    }

    if (parent.trim() === '') {
      toast(strings.scaffold.needParent);

      return;
    }

    setBusy(true);

    void scaffoldProject({ hostId, parent, name, prompt }).then((sessionId) => {
      setBusy(false);

      if (sessionId !== null) {
        close();
        reset();
      }
    });
  };

  return (
    <Modal open={open} label={strings.scaffold.title} onClose={shutdown} center className="newproject-dlg">
      <div className="flex items-start gap-[12px] border-b border-border-subtle px-[18px] py-[16px]">
        <div className="grid h-[34px] w-[34px] shrink-0 place-items-center rounded-md bg-accent-subtle text-accent">
          <Sparkles size={18} aria-hidden="true" />
        </div>
        <div>
          <div className="text-[14px] font-semibold text-text-primary">{strings.scaffold.title}</div>
          <div className="mt-[2px] text-[12px] text-text-muted">{strings.scaffold.sub}</div>
        </div>
      </div>

      <div className="flex flex-col gap-[12px] px-[18px] py-[16px]">
        {/* Where: this machine, or any connected host. One row, because most of the time it is local. */}
        <label className="flex flex-col gap-[5px]">
          <span className="text-[11.5px] font-medium text-text-secondary">{strings.scaffold.host}</span>
          <div className="rounded-md border border-border-default bg-bg-input focus-within:border-border-strong">
            <select
              id="newProjectHost"
              className="w-full bg-transparent px-[10px] py-[7px] text-[12.5px] text-text-primary outline-none"
              value={hostId}
              onChange={(event) => setHostId(event.target.value)}
            >
              {hosts.map((host) => (
                <option key={host.id} value={host.id}>
                  {host.name}
                </option>
              ))}
              {hosts.some((host) => host.id === 'local') ? null : <option value="local">Local</option>}
            </select>
          </div>
        </label>

        <label className="flex flex-col gap-[5px]">
          <span className="text-[11.5px] font-medium text-text-secondary">{strings.scaffold.parent}</span>
          <div className="flex gap-[8px]">
            <input
              type="text"
              id="newProjectParent"
              className="min-w-0 flex-1 rounded-md border border-border-default bg-bg-input px-[10px] py-[7px] font-mono text-[12.5px] text-text-primary placeholder:text-text-muted focus:border-border-strong"
              placeholder={strings.scaffold.parentPlaceholder}
              value={parent}
              onChange={(event) => setParent(event.target.value)}
            />
            {hostId === 'local' ? (
              <button
                type="button"
                className={BTN + ' ' + BTN_SECONDARY + ' shrink-0'}
                onClick={() => {
                  void pickFolder().then((picked) => {
                    if (picked !== null) {
                      setParent(picked);
                    }
                  });
                }}
              >
                {strings.scaffold.browse}
              </button>
            ) : null}
          </div>
          <span className="text-[11px] text-text-muted">{strings.scaffold.parentHelp}</span>
        </label>

        <label className="flex flex-col gap-[5px]">
          <span className="text-[11.5px] font-medium text-text-secondary">{strings.scaffold.name}</span>
          <input
            type="text"
            id="newProjectName"
            className="rounded-md border border-border-default bg-bg-input px-[10px] py-[7px] font-mono text-[12.5px] text-text-primary placeholder:text-text-muted focus:border-border-strong"
            placeholder={strings.scaffold.namePlaceholder}
            value={name}
            onChange={(event) => setName(event.target.value)}
          />
          <span className="text-[11px] text-text-muted">{strings.scaffold.nameHelp}</span>
        </label>

        <label className="flex flex-col gap-[5px]">
          <span className="text-[11.5px] font-medium text-text-secondary">{strings.scaffold.prompt}</span>
          <textarea
            id="newProjectPrompt"
            rows={3}
            className="resize-none rounded-md border border-border-default bg-bg-input px-[10px] py-[7px] text-[12.5px] leading-[1.5] text-text-primary placeholder:text-text-muted focus:border-border-strong"
            placeholder={strings.scaffold.promptPlaceholder}
            value={prompt}
            onChange={(event) => setPrompt(event.target.value)}
          />
          <span className="text-[11px] text-text-muted">{strings.scaffold.promptHelp}</span>
        </label>
      </div>

      <div className="flex items-center gap-[8px] border-t border-border-subtle px-[18px] py-[12px]">
        <button type="button" className={BTN + ' ' + BTN_SECONDARY} onClick={shutdown}>
          {strings.addHost.cancel}
        </button>

        <div className="flex-1" />

        <button
          type="button"
          className={BTN + ' ' + BTN_PRIMARY}
          id="newProjectCreate"
          disabled={busy}
          onClick={create}
        >
          {busy ? <Loader size={12} aria-hidden="true" className="animate-spin" /> : <FolderPlus size={12} aria-hidden="true" />}
          {busy ? strings.scaffold.creating : strings.scaffold.create}
        </button>
      </div>
    </Modal>
  );
}
