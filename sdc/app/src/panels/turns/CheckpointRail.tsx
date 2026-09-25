import { History } from 'lucide-react';
import { useState } from 'react';

import { strings } from '../../strings';
import { rewindTo } from '../../store/intents';
import { useLayoutStore } from '../../store/layout';
import { useRightPanelStore } from '../../store/rightPanel';
import type { TurnCheckpointData } from './types';

/**
 * The checkpoint rail (v4, docs/ROADMAP-v4.md §3 #4) - the way back, drawn where the change was made.
 *
 * The daemon writes a checkpoint before a turn's first mutating tool (P5), and until now the only place
 * to see one was the Time Machine tab. The rail puts it in the turn itself: a green dot on the stream's
 * left edge, the checkpoint's own title (`Before Edit src/pay.js`), and `Rewind here`.
 *
 * A rewind restores files, so the button asks once more before it does - the second click is the
 * confirmation, and it names what happens. Then the Time Machine opens, because that is where the
 * result (and `Undo this`) is.
 */
export interface CheckpointRailProps {
  sessionId: string;
  checkpoints: readonly TurnCheckpointData[];
}

export function CheckpointRail({ sessionId, checkpoints }: CheckpointRailProps) {
  const [armed, setArmed] = useState<string | null>(null);

  if (checkpoints.length === 0) {
    return null;
  }

  const rewind = (checkpoint: TurnCheckpointData): void => {
    if (armed !== checkpoint.id) {
      setArmed(checkpoint.id);
      return;
    }

    setArmed(null);
    useLayoutStore.getState().showRight();
    useRightPanelStore.getState().setActiveTab('timemachine', sessionId);
    void rewindTo(sessionId, `turn-${checkpoint.turn}`);
  };

  return (
    <div className="checkpoint-rail mb-[8px] flex flex-col gap-[4px]">
      {checkpoints.map((checkpoint) => (
        <div
          key={checkpoint.id}
          className="checkpoint relative flex items-center gap-[10px] py-[3px] pl-[20px] text-[11.5px]"
          data-checkpoint={checkpoint.id}
        >
          {/* The rail: a hairline down the left, and a green ring where the checkpoint sits on it. */}
          <span className="absolute bottom-[-4px] left-[5px] top-[-4px] w-px bg-border-subtle" aria-hidden="true" />
          <span
            className="absolute left-0 top-1/2 h-[11px] w-[11px] -translate-y-1/2 rounded-full border-2 border-state-success bg-bg-base shadow-[0_0_0_3px_var(--green-subtle)]"
            aria-hidden="true"
          />
          <span className="font-mono text-[10px] font-semibold uppercase tracking-[.08em] text-state-success">
            {strings.turns.checkpoint.label}
          </span>
          <span className="min-w-0 flex-1 truncate text-text-secondary" title={checkpoint.title}>
            {checkpoint.title}
          </span>
          <button
            type="button"
            className={
              'flex shrink-0 items-center gap-[5px] rounded-sm border px-[8px] py-[2px] text-[11px] transition-colors duration-fast focus-visible:outline focus-visible:outline-2 focus-visible:outline-border-focus ' +
              (armed === checkpoint.id
                ? 'border-state-waiting bg-orange-subtle text-state-waiting'
                : 'border-border-default text-text-muted hover:border-border-strong hover:text-text-primary')
            }
            aria-label={strings.turns.checkpoint.aria(checkpoint.title)}
            onClick={() => rewind(checkpoint)}
            onBlur={() => setArmed((current) => (current === checkpoint.id ? null : current))}
          >
            <History size={11} aria-hidden="true" />
            {armed === checkpoint.id ? strings.turns.checkpoint.confirm : strings.turns.checkpoint.rewind}
          </button>
        </div>
      ))}
    </div>
  );
}
