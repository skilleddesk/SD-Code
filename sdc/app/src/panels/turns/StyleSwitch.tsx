import { Activity, LayoutList, Rows3 } from 'lucide-react';

import { strings } from '../../strings';
import { useStreamStyle, type StreamStyle } from '../../store/streamStyle';

const OPTIONS: { id: StreamStyle; icon: typeof Rows3 }[] = [
  { id: 'classic', icon: Rows3 },
  { id: 'flow', icon: LayoutList },
  { id: 'console', icon: Activity },
];

/** The pane's switch between the three stream styles (0.15) - the same turns, drawn another way at once. */
export function StyleSwitch() {
  const style = useStreamStyle((state) => state.style);
  const setStyle = useStreamStyle((state) => state.setStyle);

  return (
    <div
      className="style-switch flex items-center gap-[1px] rounded-full border border-border-subtle bg-bg-glass p-[2px] shadow-sm backdrop-blur"
      role="radiogroup"
      aria-label={strings.turns.style.label}
      data-stream-style={style}
    >
      {OPTIONS.map(({ id, icon: Icon }) => (
        <button
          key={id}
          type="button"
          role="radio"
          aria-checked={style === id}
          title={`${strings.turns.style[id]} - ${strings.turns.style.hint[id]}`}
          className={
            'inline-flex items-center gap-[4px] rounded-full px-[8px] py-[2px] text-[10.5px] transition-colors duration-fast ' +
            (style === id ? 'bg-accent-subtle font-semibold text-accent' : 'text-text-muted hover:text-text-primary')
          }
          onClick={() => setStyle(id)}
        >
          <Icon size={11} aria-hidden="true" />
          {strings.turns.style[id]}
        </button>
      ))}
    </div>
  );
}
