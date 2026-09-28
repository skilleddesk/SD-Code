import { useState } from 'react';

import { strings } from '../strings';
import { storedSettings, storeSetting } from '../lib/settings';
import { eraseEverything, useAgentUi } from '../store/agentIntents';
import { daemonSetting } from '../store/kernelIntents';
import { BTN, BTN_SECONDARY, BTN_SM } from '../panels/ui/button';

/**
 * Settings → Safety's **Agent** block (0.13): the completion check, the memory, and erase-everything -
 * the switch the uninstaller's "Delete the application data" box also throws.
 */
export function AgentSettings() {
  const words = strings.agent.settings;
  const [autoCheck, setAutoCheck] = useState(() => storedSettings()['agent-auto-check'] !== false);
  const [confirming, setConfirming] = useState(false);
  const [typed, setTyped] = useState('');

  return (
    <section className="mb-[20px] border-b border-border-subtle pb-[18px]">
      <h3 className="text-[14px] font-semibold text-text-primary">{words.title}</h3>
      <p className="mt-[2px] text-[12px] text-text-muted">{words.desc}</p>

      <label className="mt-[12px] flex items-start gap-[10px]">
        <input
          type="checkbox"
          className="mt-[3px] accent-[var(--accent)]"
          checked={autoCheck}
          onChange={(event) => {
            setAutoCheck(event.target.checked);
            storeSetting('agent-auto-check', event.target.checked);
            void daemonSetting('agent.autoCheck', event.target.checked ? 'on' : 'off');
          }}
        />
        <span>
          <span className="block text-[12.5px] text-text-primary">{words.autoCheck}</span>
          <span className="block text-[11.5px] text-text-muted">{words.autoCheckHelp}</span>
        </span>
      </label>

      <div className="mt-[14px] flex items-start justify-between gap-[12px]">
        <span>
          <span className="block text-[12.5px] text-text-primary">{words.memory}</span>
          <span className="block text-[11.5px] text-text-muted">{words.memoryHelp}</span>
        </span>
        <button type="button" className={BTN_SM + ' ' + BTN_SECONDARY + ' shrink-0'} onClick={() => useAgentUi.getState().openMemory()}>
          {words.openMemory}
        </button>
      </div>

      <div className="mt-[14px] rounded-lg border border-state-error p-[12px]">
        <span className="block text-[12.5px] font-medium text-state-error">{words.erase}</span>
        <span className="mt-[2px] block text-[11.5px] text-text-muted">{words.eraseHelp}</span>
        {confirming ? (
          <div className="mt-[10px] flex flex-wrap items-center gap-[8px]">
            <span className="w-full text-[11.5px] text-text-secondary">{words.eraseConfirm}</span>
            <input
              className="h-[30px] w-[140px] rounded-md border border-border-default bg-bg-base px-[10px] font-mono text-[12.5px] text-text-primary"
              value={typed}
              aria-label={words.eraseConfirm}
              onChange={(event) => setTyped(event.target.value)}
            />
            <button
              type="button"
              disabled={typed !== 'ERASE'}
              className={BTN_SM + ' rounded-md bg-state-error px-[10px] text-text-on-accent disabled:opacity-40'}
              onClick={() => void eraseEverything()}
            >
              {words.eraseButton}
            </button>
          </div>
        ) : (
          <button type="button" className={BTN + ' ' + BTN_SECONDARY + ' mt-[10px] text-state-error'} onClick={() => setConfirming(true)}>
            {words.erase}
          </button>
        )}
      </div>
    </section>
  );
}
