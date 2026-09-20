import type { MouseEvent, ReactNode } from 'react';

/**
 * One segment of the status bar (spec section 7.15).
 *
 * Either a piece of text with mono styling and a hover, or - for the two ends - a state dot plus
 * text. Every segment is a button: the spec says each one opens the surface it summarises, so none
 * of them is decoration.
 *
 * `hideSmall` is the `hide-sm` class of the prototype: at 900px the providers, chats and hosts
 * segments fold away, leaving the dot, the host, the engine, the model and the connection. `tone` is
 * the one difference between them otherwise - the engine's value is accented, the model's is
 * primary, and the connection label carries the state colour.
 */
export interface StatusItemProps {
  /** The segment's own id from the spec's inventory, e.g. `statusEngine`. */
  id?: string;
  /** A 6px dot before the text, coloured by `dotClass`. */
  dotClass?: string;
  /** Class for the text: `text-accent` for the engine, `text-text-primary` for the model. */
  tone?: string;
  /** Fold this segment away at 900px (`.hide-sm`). */
  hideSmall?: boolean;
  /** Fold it away at 520px too - everything except the connection (spec section 7.15). */
  hideTiny?: boolean;
  onClick: (event: MouseEvent<HTMLButtonElement>) => void;
  title: string;
  children: ReactNode;
}

export function StatusItem({
  id,
  dotClass,
  tone,
  hideSmall = false,
  hideTiny = false,
  onClick,
  title,
  children,
}: StatusItemProps) {
  const classes = [
    'item',
    'flex items-center gap-[6px] whitespace-nowrap rounded-sm px-[6px] py-[3px] transition-all duration-fast ease-ease',
    'hover:bg-bg-hover hover:text-text-primary',
    hideSmall ? 'hide-sm max-900:hidden' : '',
    hideTiny ? 'max-520:hidden' : '',
  ]
    .filter(Boolean)
    .join(' ');

  return (
    <button type="button" id={id} className={classes} title={title} onClick={onClick}>
      {dotClass === undefined ? null : (
        <span className={'dot h-[6px] w-[6px] rounded-full ' + dotClass} aria-hidden="true" />
      )}
      <span className={tone ?? ''}>{children}</span>
    </button>
  );
}
