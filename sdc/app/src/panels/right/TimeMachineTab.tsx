import { GitCompare, History, Redo2, RotateCcw } from 'lucide-react';
import { useMemo, useState } from 'react';

import { strings } from '../../strings';
import { openDiff, redoRewind, rewindTo } from '../../store/intents';
import { useAppStore } from '../../store/store';
import { useSessionsStore } from '../../store/sessions';
import { BTN, BTN_BLOCK, BTN_SECONDARY } from '../ui/button';

/**
 * The Time Machine tab - spec section 9.13 / 14, and v4's rewind that restores.
 *
 * Newest checkpoint first. A checkpoint is the folder **before** a change, so every one of them - the
 * newest included - is a place to go back to: the newest is how the last change is undone. (The tab used
 * to mark the newest as `CURRENT` and refuse it, and a click on any other restored the newest instead of
 * the one clicked; see `rewind::apply`.)
 *
 * A rewind changes files, so a row asks once more before it does - the first click arms it and says what
 * will happen, the second restores. `Redo` puts back the last rewind: the files as they were just before
 * it, and its checkpoints.
 *
 * Everything here is the log: `CheckpointSaved` rows, and the `RewindApplied` that moves them.
 */
export function TimeMachineTab() {
  const { activeTab } = useSessionsStore();
  const sessionId = activeTab;
  const all = useAppStore((state) => state.checkpoints);
  const stack = useAppStore((state) => state.rewindStack);
  const [armed, setArmed] = useState<string | null>(null);
  /* This chat's checkpoints only - the tab once listed every chat's (and rewound the demo session). */
  const entries = useMemo(() => (sessionId === null ? [] : all.filter((entry) => entry.sessionId === sessionId)), [all, sessionId]);
  const canRedo = sessionId !== null && stack.some((entry) => entry.sessionId === sessionId);

  if (sessionId === null || (entries.length === 0 && !canRedo)) {
    return (
      <div className="flex flex-1 items-center justify-center p-[24px] text-center text-[12.5px] text-text-muted">
        {strings.rightPanel.timeMachine.empty}
      </div>
    );
  }

  const choose = (id: string, turn: number): void => {
    if (armed !== id) {
      setArmed(id);

      return;
    }

    setArmed(null);
    void rewindTo(sessionId, `turn-${turn}`);
  };

  return (
    <>
      <p className="px-[12px] pt-[12px] text-[11.5px] leading-[1.55] text-text-muted">{strings.rightPanel.timeMachine.hint}</p>

      <div className="tm-list p-[12px]">
        {entries.map((entry) => {
          const isArmed = armed === entry.id;

          return (
            <button
              key={entry.id}
              type="button"
              data-checkpoint={entry.id}
              className={
                'tm-entry relative mb-[8px] flex w-full gap-[10px] rounded-md border p-[10px] text-left transition-all duration-base focus-visible:outline focus-visible:outline-2 focus-visible:outline-border-focus ' +
                (isArmed
                  ? 'border-state-waiting bg-orange-subtle'
                  : 'border-border-subtle bg-bg-raised hover:translate-x-[2px] hover:border-border-strong')
              }
              aria-label={isArmed ? strings.rightPanel.timeMachine.confirm(entry.title) : strings.rightPanel.timeMachine.choose(entry.title)}
              onClick={() => choose(entry.id, entry.turn)}
              onBlur={() => setArmed((current) => (current === entry.id ? null : current))}
            >
              {/* A screenshot when the preview wrote one; otherwise an icon, not a gradient that looks like one. */}
              {entry.thumbnail === null ? (
                <div className="tm-thumb grid h-[48px] w-[48px] shrink-0 place-items-center rounded-sm border border-border-subtle bg-bg-base text-text-muted" aria-hidden="true">
                  <History size={16} />
                </div>
              ) : (
                <img className="tm-thumb h-[48px] w-[72px] shrink-0 rounded-sm object-cover" src={entry.thumbnail} alt="" />
              )}

              <div className="tm-body min-w-0 flex-1">
                <div className="tm-turn font-mono text-[10.5px] text-text-muted">{strings.rightPanel.timeMachine.when(entry.when)}</div>
                <div className="tm-title my-[3px] overflow-hidden text-ellipsis whitespace-nowrap text-[12.5px] font-medium text-text-primary">
                  {entry.title}
                </div>
                {isArmed ? (
                  <div className="flex items-center gap-[5px] text-[11px] font-semibold text-state-waiting">
                    <RotateCcw size={11} aria-hidden="true" />
                    {strings.rightPanel.timeMachine.clickAgain}
                  </div>
                ) : (
                  <div className="font-mono text-[10px] text-text-muted">{entry.filesHash.slice(0, 12)}</div>
                )}
              </div>
            </button>
          );
        })}
      </div>

      <div className="tm-footer mt-auto flex flex-col gap-[8px] border-t border-border-subtle p-[12px]">
        {canRedo ? (
          <button type="button" className={BTN + ' ' + BTN_SECONDARY + ' ' + BTN_BLOCK} onClick={() => void redoRewind(sessionId)}>
            <Redo2 size={12} aria-hidden="true" />
            {strings.rightPanel.timeMachine.redo}
          </button>
        ) : null}
        <button type="button" className={BTN + ' ' + BTN_SECONDARY + ' ' + BTN_BLOCK} onClick={() => void openDiff()}>
          <GitCompare size={12} aria-hidden="true" />
          {strings.rightPanel.timeMachine.compare}
        </button>
      </div>
    </>
  );
}
