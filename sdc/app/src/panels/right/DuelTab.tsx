import { strings } from '../../strings';
import { keepDuel, startDuel } from '../../store/intents';
import { useAppStore } from '../../store/store';
import { useSessionsStore } from '../../store/sessions';
import { toast } from '../../store/toast';
import { BTN, BTN_SM, BTN_PRIMARY, BTN_SECONDARY } from '../ui/button';

/**
 * The Duel tab - spec section 7.10 / 16.6.
 *
 * Two runs of the same prompt side by side, so the choice between engines is a comparison rather
 * than a guess: engine and model at the top, then time, cost and the pass/fail verdict, then the
 * files each run touched, then `Diff` and `Keep`.
 *
 * The panes are *real data*: they are the `DuelStarted` event's `panes`, folded by the reducer (the
 * seed carries the prototype's pair so the tab is not empty). `Keep` is an intent -
 * `duel.keep` - which archives the other run rather than deleting it (nothing this app produces is
 * ever destroyed), and the daemon answers with `DuelResolved` plus the toast that says which one
 * won. `Keep neither` archives both, which is the third answer spec section 16.6 allows and the one
 * a two-button design would quietly deny you.
 *
 * The tab is hidden in Simple mode: a three-way depth setting whose middle setting still offers a
 * side-by-side engine race would not be simple. `RightPanel` does the hiding, and it is the only
 * place in the UI where the mode switch changes what exists.
 */
export function DuelTab() {
  const { activeTab } = useSessionsStore();
  const sessionId = activeTab ?? 's1';
  const duel = useAppStore((state) => state.duels[0] ?? null);
  const engines = strings.rightPanel.duel.panes.map((pane) => pane.engine);

  if (duel === null || duel.panes.length === 0) {
    return (
      <div className="flex flex-1 flex-col items-center justify-center gap-[10px] p-[24px] text-center">
        <p className="text-[12.5px] text-text-muted">{strings.rightPanel.duel.empty}</p>
        <button
          type="button"
          className={BTN + ' ' + BTN_SECONDARY}
          data-action="duel-start"
          onClick={() => void startDuel(sessionId, strings.turns.prompt, engines)}
        >
          {strings.rightPanel.duel.run}
        </button>
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col">
      <div className="duel-wrap grid flex-1 grid-cols-2 gap-[8px] p-[12px]" data-duel={duel.id}>
        {duel.panes.map((pane) => (
          <div
            key={pane.engine}
            data-duel-pane={pane.engine}
            className={
              'duel-pane flex flex-col overflow-hidden rounded-md border bg-bg-raised ' +
              (duel.resolved && duel.kept === pane.engine
                ? 'border-accent'
                : 'border-border-subtle')
            }
          >
            <div className="duel-head border-b border-border-subtle px-[12px] py-[10px] font-mono text-[10.5px] text-text-secondary">
              <div className="duel-model text-[11.5px] font-semibold text-text-primary">
                {pane.engine} · {pane.model}
              </div>
              <div className="duel-meta mt-[4px] flex flex-wrap gap-[8px]">
                <span>{pane.time}</span>
                <span>{pane.cost}</span>
                <span className={pane.pass ? 'pass text-state-success' : 'fail text-state-error'}>
                  {pane.pass ? 'PASS' : 'FAIL'}
                </span>
                {duel.resolved && duel.kept === pane.engine ? (
                  <span className="text-accent">{strings.rightPanel.duel.keptLabel}</span>
                ) : null}
              </div>
            </div>

            <div className="duel-body flex-1 overflow-y-auto px-[12px] py-[10px] font-mono text-[11px] leading-[1.75] text-text-secondary">
              <div>{pane.headline}</div>
              <div className="h-[8px]" />
              {pane.files.map((file) => (
                <div key={file} className="add text-diff-addText">
                  {file}
                </div>
              ))}
            </div>

            <div className="duel-foot flex gap-[4px] border-t border-border-subtle px-[10px] py-[8px]">
              <button
                type="button"
                className={BTN_SM + ' ' + BTN_SECONDARY + ' flex-1 justify-center'}
                onClick={() => toast(strings.rightPanel.duel.diff)}
              >
                {strings.rightPanel.duel.diff}
              </button>
              <button
                type="button"
                className={BTN_SM + ' ' + BTN_PRIMARY + ' flex-1 justify-center'}
                data-action={`keep-${pane.engine}`}
                onClick={() => void keepDuel(duel.id, pane.engine)}
              >
                {strings.rightPanel.duel.keep}
              </button>
            </div>
          </div>
        ))}
      </div>

      <div className="flex gap-[6px] border-t border-border-subtle p-[10px]">
        <button
          type="button"
          className={BTN + ' ' + BTN_SECONDARY + ' flex-1 justify-center'}
          data-action="duel-restart"
          onClick={() => void startDuel(duel.sessionId, duel.prompt, engines)}
        >
          {strings.rightPanel.duel.runAgain}
        </button>
        <button
          type="button"
          className={BTN + ' ' + BTN_SECONDARY + ' flex-1 justify-center'}
          data-action="duel-discard"
          onClick={() => void keepDuel(duel.id, null)}
        >
          {strings.rightPanel.duel.keepNeither}
        </button>
      </div>
    </div>
  );
}
