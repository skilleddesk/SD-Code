import { FileText, TerminalSquare } from 'lucide-react';

import { strings } from '../../strings';

/**
 * The list the composer opens above the box (0.13): `/` shows the commands, `@` the chat's files. The
 * keyboard stays in the textarea - ↑↓ move, ↵ or Tab choose, Esc closes - so typing never stops to aim
 * a mouse; a click chooses too.
 */
export interface MenuItem {
  key: string;
  label: string;
  detail: string;
  badge?: string;
}

export interface ComposerMenuProps {
  kind: 'slash' | 'mention';
  items: readonly MenuItem[];
  index: number;
  loading: boolean;
  onChoose: (index: number) => void;
  onHover: (index: number) => void;
}

export function ComposerMenu({ kind, items, index, loading, onChoose, onHover }: ComposerMenuProps) {
  const words = kind === 'slash' ? strings.agent.commands : strings.agent.mention;
  const Icon = kind === 'slash' ? TerminalSquare : FileText;

  return (
    <div
      role="listbox"
      aria-label={words.title}
      className="composer-menu absolute bottom-full left-0 right-0 z-30 mb-[6px] max-h-[280px] overflow-y-auto rounded-lg border border-border-default bg-bg-raised p-[4px] shadow-lg"
    >
      <div className="px-[8px] pb-[4px] pt-[2px] font-mono text-[10px] uppercase tracking-wide text-text-muted">{words.title}</div>
      {items.length === 0 ? (
        <div className="px-[8px] py-[6px] text-[12px] text-text-muted">
          {loading && kind === 'mention' ? strings.agent.mention.searching : words.empty}
        </div>
      ) : (
        items.map((item, position) => (
          <button
            key={item.key}
            type="button"
            role="option"
            aria-selected={position === index}
            className={
              'flex w-full items-center gap-[8px] rounded-md px-[8px] py-[5px] text-left text-[12.5px] ' +
              (position === index ? 'bg-accent-subtle text-accent' : 'text-text-secondary hover:bg-bg-hover')
            }
            onMouseDown={(event) => {
              /* Chosen before the textarea loses focus, so the caret is where the item goes. */
              event.preventDefault();
              onChoose(position);
            }}
            onMouseEnter={() => onHover(position)}
          >
            <Icon size={13} aria-hidden="true" className="shrink-0 opacity-70" />
            <span className="shrink-0 font-mono text-text-primary">{item.label}</span>
            <span className="min-w-0 flex-1 truncate text-[11.5px] text-text-muted">{item.detail}</span>
            {item.badge === undefined ? null : (
              <span className="shrink-0 rounded-full border border-border-subtle px-[6px] font-mono text-[9.5px] text-text-muted">{item.badge}</span>
            )}
          </button>
        ))
      )}
      {kind === 'slash' ? <div className="px-[8px] pt-[4px] font-mono text-[10px] text-text-muted">{strings.agent.commands.hint}</div> : null}
    </div>
  );
}
