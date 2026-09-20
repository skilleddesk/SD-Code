import { useEffect, useMemo, useRef, useState } from 'react';
import {
  Brain,
  CircleCheckBig,
  Columns2,
  Keyboard,
  PanelLeft,
  PanelRight,
  Plug,
  Search,
  Server,
  Settings as SettingsIcon,
  Stethoscope,
  X,
  Zap,
  type LucideIcon,
} from 'lucide-react';

import { matchCommands, type Command, type CommandIcon } from '../commands/registry';
import { useOverlayStore } from '../store/overlays';
import { strings } from '../strings';
import { Modal } from '../modals/Modal';

/**
 * `#paletteBd` - the command palette (spec section 9.2).
 *
 * The list is not written here: it is `matchCommands()` over the command registry, so every
 * registered command appears automatically and the palette can never fall behind the keyboard map
 * (principle P7, spec section 9.1). `Recent` and `Actions` are the registry's `global`/`actions`
 * groups in the order the prototype shows.
 *
 * Keys: arrow keys move the selection, `Enter` runs it, `Esc` closes. They are handled *inside* the
 * input and stopped there, which is what keeps the global listener from also seeing them.
 */
const ICONS: Partial<Record<CommandIcon, LucideIcon>> = {
  plus: Server,
  plug: Plug,
  serverPlus: Server,
  columns: Columns2,
  search: Search,
  check: CircleCheckBig,
  stethoscope: Stethoscope,
  settings: SettingsIcon,
  keyboard: Keyboard,
  panelLeft: PanelLeft,
  panelRight: PanelRight,
  x: X,
  zap: Zap,
  brain: Brain,
};

export function Palette() {
  const open = useOverlayStore((state) => state.paletteOpen);
  const close = useOverlayStore((state) => state.closePalette);
  const [query, setQuery] = useState('');
  const [index, setIndex] = useState(0);
  const listRef = useRef<HTMLDivElement | null>(null);

  const results = useMemo(() => matchCommands(query), [query]);

  useEffect(() => {
    if (open) {
      setQuery('');
      setIndex(0);
    }
  }, [open]);

  const run = (command: Command | undefined): void => {
    if (!command) {
      return;
    }

    close();
    command.run();
  };

  /** Group headers only make sense for an unfiltered list - a search is one ranked list. */
  const grouped = query.trim() === '';

  return (
    <Modal open={open} label={strings.topbar.palette.title} onClose={close} className="palette">
      <div
        className="flex items-center gap-[10px] border-b border-border-subtle px-[18px] py-[15px]"
        onKeyDown={(event) => {
          if (event.key === 'ArrowDown') {
            event.preventDefault();
            event.stopPropagation();
            setIndex((current) => Math.min(current + 1, results.length - 1));
          } else if (event.key === 'ArrowUp') {
            event.preventDefault();
            event.stopPropagation();
            setIndex((current) => Math.max(current - 1, 0));
          } else if (event.key === 'Enter') {
            event.preventDefault();
            event.stopPropagation();
            run(results[index]);
          } else if (event.key === 'Escape') {
            event.preventDefault();
            event.stopPropagation();
            close();
          }
        }}
      >
        <span className="font-mono text-[16px] text-text-muted">&gt;</span>
        <input
          className="palette-input min-w-0 flex-1 bg-transparent text-[15px] text-text-primary placeholder:text-text-muted"
          placeholder={strings.palette.placeholder}
          aria-label={strings.palette.placeholder}
          value={query}
          autoFocus
          onChange={(event) => {
            setQuery(event.target.value);
            setIndex(0);
          }}
        />
        <span className="kbd inline-flex items-center rounded-[3px] border border-border-default border-b-2 bg-bg-base px-[5px] py-[1px] font-mono text-[9.5px] text-text-secondary">
          Esc
        </span>
      </div>

      <div ref={listRef} className="max-h-[52vh] overflow-y-auto p-[6px]">
        {results.length === 0 ? (
          <div className="p-[24px] text-center text-[12.5px] text-text-muted">
            {strings.palette.empty}
          </div>
        ) : null}

        {results.map((command, position) => {
          const Icon = command.icon === undefined ? undefined : ICONS[command.icon];
          const first = results[position - 1];
          const newGroup = grouped && (position === 0 || first?.group !== command.group);

          return (
            <div key={command.id}>
              {newGroup ? (
                <div className="palette-group-title px-[10px] pb-[4px] pt-[8px] text-[10px] font-bold uppercase tracking-[0.1em] text-text-muted">
                  {command.group === 'actions' ? strings.palette.actions : strings.palette.recent}
                </div>
              ) : null}

              <button
                type="button"
                data-command={command.id}
                aria-selected={position === index}
                className={
                  'palette-item flex w-full items-center gap-[10px] rounded-md px-[10px] py-[8px] text-left text-[13px] transition-colors duration-fast ease-ease ' +
                  (position === index
                    ? 'selected bg-accent-subtle text-text-primary'
                    : 'text-text-secondary hover:bg-bg-hover hover:text-text-primary')
                }
                onMouseEnter={() => setIndex(position)}
                onClick={() => run(command)}
              >
                <span className="grid h-[20px] w-[20px] place-items-center text-text-muted">
                  {Icon === undefined ? null : <Icon size={14} aria-hidden="true" />}
                </span>
                <span className="palette-label flex-1">{command.label}</span>
                {command.hint === undefined ? null : (
                  <span className="palette-hint font-mono text-[10px] text-text-muted">
                    {command.hint}
                  </span>
                )}
              </button>
            </div>
          );
        })}
      </div>
    </Modal>
  );
}
