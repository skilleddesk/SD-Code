import { useEffect, useState } from 'react';

import { strings } from '../strings';
import { readMemory, useAgentUi, writeMemory } from '../store/agentIntents';
import { useSessionsStore } from '../store/sessions';
import { toast } from '../store/toast';
import { Modal } from '../modals/Modal';
import { BTN, BTN_PRIMARY } from '../panels/ui/button';

/**
 * **Memory** (0.13): what every chat is told before the person's words - the project's `.sdc/memory.md`
 * (on the machine the project is on) and SDC's global memory for every project. The agent adds lines with
 * its `remember` tool and `/remember`; here the person reads and edits the whole of both.
 */
export function MemoryDialog() {
  const open = useAgentUi((state) => state.memoryOpen);
  const close = useAgentUi((state) => state.closeMemory);
  const { activeTab: sessionId } = useSessionsStore();
  const [project, setProject] = useState<{ text: string; path: string } | null | 'loading'>('loading');
  const [global, setGlobal] = useState<{ text: string; path: string } | null | 'loading'>('loading');
  const words = strings.agent.memory;

  useEffect(() => {
    if (!open) {
      return;
    }

    setProject('loading');
    setGlobal('loading');
    void readMemory(sessionId, 'project').then(setProject);
    void readMemory(sessionId, 'global').then(setGlobal);
  }, [open, sessionId]);

  const save = async (): Promise<void> => {
    let ok = true;

    if (project !== null && project !== 'loading') {
      ok = (await writeMemory(sessionId, 'project', project.text)) && ok;
    }

    if (global !== null && global !== 'loading') {
      ok = (await writeMemory(sessionId, 'global', global.text)) && ok;
    }

    if (ok) {
      toast(words.saved);
      close();
    }
  };

  const area = 'mt-[6px] h-[150px] w-full resize-y rounded-md border border-border-default bg-bg-base p-[10px] font-mono text-[12px] leading-[1.5] text-text-primary placeholder:text-text-muted';

  return (
    <Modal open={open} label={words.title} onClose={close} center className="memory-dlg w-[min(720px,94vw)] p-[22px]">
      <h2 className="text-[16px] font-semibold text-text-primary">{words.title}</h2>
      <p className="mt-[4px] text-[12.5px] text-text-secondary">{words.subtitle}</p>

      <label className="mt-[16px] block text-[12.5px] font-medium text-text-primary">
        {words.project}
        {project === 'loading' ? (
          <p className="mt-[6px] text-[12px] text-text-muted">{words.loading}</p>
        ) : project === null ? (
          <p className="mt-[6px] text-[12px] text-text-muted">{words.noProject}</p>
        ) : (
          <>
            <span className="ml-[8px] font-mono text-[10.5px] font-normal text-text-muted">{project.path}</span>
            <textarea className={area} value={project.text} placeholder={words.placeholder} onChange={(event) => setProject({ ...project, text: event.target.value })} />
          </>
        )}
      </label>

      <label className="mt-[14px] block text-[12.5px] font-medium text-text-primary">
        {words.global}
        {global === 'loading' || global === null ? (
          <p className="mt-[6px] text-[12px] text-text-muted">{words.loading}</p>
        ) : (
          <textarea className={area} value={global.text} placeholder={words.placeholder} onChange={(event) => setGlobal({ ...global, text: event.target.value })} />
        )}
      </label>

      <div className="mt-[16px] flex justify-end">
        <button type="button" className={BTN + ' ' + BTN_PRIMARY} onClick={() => void save()}>
          {words.save}
        </button>
      </div>
    </Modal>
  );
}
