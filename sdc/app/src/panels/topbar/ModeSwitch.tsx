import { strings } from '../../strings';
import { useLayoutStore, type AppMode } from '../../store/layout';
import { toast } from '../../store/toast';

/**
 * `.mode-switch` - Simple | Pro | Auto (spec section 7.1, row 5).
 *
 * A three-segment pill, 28px tall, sitting between the host pill's spacer and the palette button.
 * Pro is the selected segment on a fresh install, the choice persists in the layout store, and the
 * whole control folds away at 900px because at that width the buttons it would sit next to are
 * already gone.
 *
 * The selected segment is drawn with `--bg-active` plus an inset hairline, exactly as the prototype
 * describes it - the pill itself keeps `--bg-raised` so the selection reads as a raised chip inside
 * a recessed track.
 */

const MODES: readonly AppMode[] = ['simple', 'pro', 'auto'];

export function ModeSwitch() {
  const { mode, setMode } = useLayoutStore();

  const select = (next: AppMode): void => {
    setMode(next);
    toast(strings.topbar.modeChanged(strings.topbar.modes[next]));
  };

  return (
    <div
      className="mode-switch flex items-center gap-[2px] p-[3px] h-[28px] shrink-0 rounded-md bg-bg-raised border border-border-subtle max-900:hidden"
      id="modeSwitch"
      role="group"
      aria-label={strings.topbar.modeSwitchTitle}
    >
      {MODES.map((candidate) => (
        <button
          key={candidate}
          type="button"
          data-mode={candidate}
          onClick={() => select(candidate)}
          aria-pressed={mode === candidate}
          className={
            'px-[10px] py-[3px] text-[11px] font-medium rounded-sm transition-all duration-fast ease-ease ' +
            (mode === candidate
              ? 'bg-bg-active text-text-primary shadow-[0_1px_2px_rgba(0,0,0,.2),inset_0_0_0_1px_var(--border-default)]'
              : 'text-text-secondary hover:text-text-primary')
          }
        >
          {strings.topbar.modes[candidate]}
        </button>
      ))}
    </div>
  );
}
