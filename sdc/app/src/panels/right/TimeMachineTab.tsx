import { GitCompare, History } from 'lucide-react';
import { useMemo } from 'react';

import { strings } from '../../strings';
import { openDiff, rewindTo } from '../../store/intents';
import { useAppStore } from '../../store/store';
import { useSessionsStore } from '../../store/sessions';
import { BTN, BTN_BLOCK, BTN_SECONDARY } from '../ui/button';

/**
 * The Time Machine tab - spec section 9.13 / 14.
 *
 * Newest checkpoint first: a 72x48 thumbnail, `turn 14 · now` in mono, and the title of what
 * changed. The current one is outlined in accent and wears a `CURRENT` pill on its top edge; the
 * others nudge 2px to the right when you hover them, which is the whole affordance for "this is
 * reversible".
 *
 * The checkpoints are *real data*: they are the `CheckpointSaved` events the daemon appended, folded
 * by the reducer (the seed ships the prototype's three, so the tab is not empty on first run).
 * Clicking one calls `rewindTo()`, which asks the daemon to restore the files and the conversation;
 * the daemon answers with `RewindApplied` plus a 10-second toast carrying `Undo this`. Nothing here
 * pretends a rewind happened - the tab only ever redraws what the log says.
 */
export function TimeMachineTab() {
  const { activeTab } = useSessionsStore();
  /*
   * This chat's checkpoints only. The tab used to list every chat's, and to send a rewind to the demo
   * session `s1` when no chat was open - so clicking a row could ask the daemon to restore a turn of a
   * different conversation.
   */
  const sessionId = activeTab;
  const all = useAppStore((state) => state.checkpoints);
  const entries = useMemo(() => (sessionId === null ? [] : all.filter((entry) => entry.sessionId === sessionId)), [all, sessionId]);

  /* Nothing has changed yet: the spec's empty line, in the same centred box the Console uses. */
  if (sessionId === null || entries.length === 0) {
    return (
      <div className="flex flex-1 items-center justify-center p-[24px] text-center text-[12.5px] text-text-muted">
        {strings.rightPanel.timeMachine.empty}
      </div>
    );
  }

  return (
    <>
      <div className="tm-list p-[12px]">
        {entries.map((entry, index) => {
          const current = index === 0;

          return (
            <div
              key={entry.id}
              data-checkpoint={entry.id}
              className={
                'tm-entry relative mb-[8px] flex cursor-pointer gap-[10px] rounded-md border bg-bg-raised p-[10px] transition-all duration-base ' +
                (current
                  ? 'current border-accent bg-accent-subtle'
                  : 'border-border-subtle hover:translate-x-[2px] hover:border-border-strong')
              }
              role="button"
              tabIndex={0}
              onClick={() => {
                if (!current) {
                  void rewindTo(sessionId, `turn-${entry.turn}`);
                }
              }}
              onKeyDown={(event) => {
                if (!current && event.key === 'Enter') {
                  void rewindTo(sessionId, `turn-${entry.turn}`);
                }
              }}
            >
              {current ? (
                <span className="absolute -top-[7px] right-[12px] rounded-full bg-accent-fill px-[6px] py-[1px] text-[9px] font-bold tracking-[.08em] text-text-on-accent">
                  {strings.rightPanel.timeMachine.current}
                </span>
              ) : null}

              {/* A screenshot when the preview wrote one; otherwise an icon, not a gradient that looks like one. */}
              {entry.thumbnail === null ? (
                <div className="tm-thumb grid h-[48px] w-[48px] shrink-0 place-items-center rounded-sm border border-border-subtle bg-bg-base text-text-muted" aria-hidden="true">
                  <History size={16} />
                </div>
              ) : (
                <img className="tm-thumb h-[48px] w-[72px] shrink-0 rounded-sm object-cover" src={entry.thumbnail} alt="" />
              )}

              <div className="tm-body min-w-0 flex-1">
                <div className="tm-turn font-mono text-[10.5px] text-text-muted">
                  {strings.rightPanel.timeMachine.when(entry.when)}
                </div>
                <div className="tm-title my-[3px] overflow-hidden text-ellipsis whitespace-nowrap text-[12.5px] font-medium text-text-primary">
                  {entry.title}
                </div>
                <div className="font-mono text-[10px] text-text-muted">{entry.filesHash}</div>
              </div>
            </div>
          );
        })}
      </div>

      <div className="tm-footer mt-auto border-t border-border-subtle p-[12px]">
        <button
          type="button"
          className={BTN + ' ' + BTN_SECONDARY + ' ' + BTN_BLOCK}
          onClick={() => void openDiff()}
        >
          <GitCompare size={12} aria-hidden="true" />
          {strings.rightPanel.timeMachine.compare}
        </button>
      </div>
    </>
  );
}
