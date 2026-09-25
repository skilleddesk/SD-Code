import type { LucideIcon } from 'lucide-react';
import type { MouseEvent } from 'react';

/**
 * `#openProviders` / `#themeToggle` / `#toggleSidebar` / `#toggleRight` / `#openSettings` (spec
 * section 7.1, rows 7-11) - and every other 28px square button in the app, because the spec gives
 * them one shape (section 8.3: "button md 28px").
 *
 * The four states are the prototype's `.icon-btn` rules, in token terms:
 *
 *   rest      28x28, radius --r-md, text-secondary
 *   hover     background --bg-hover, text-primary           (150ms would be wrong: 90ms, --fast)
 *   active    scale(.94), which is the pressed state
 *   selected  background --accent-subtle, text --accent     (the split button while split is on)
 *
 * `hasDot` is separate from `active` on purpose: the plug button carries a dot because a provider
 * needs authentication (spec section 7.1, row 7) while staying visually unselected. Its styling
 * lives in `@layer components` in src/styles/globals.css, because a dot in a pseudo-element is not
 * something a utility class can express.
 */
export interface IconButtonProps {
  icon: LucideIcon;
  /** Tooltip and accessible name. Comes from src/strings.ts, never from a literal. */
  label: string;
  onClick?: (event: MouseEvent<HTMLButtonElement>) => void;
  /** Draw the button in its selected state (`.icon-btn.active`). */
  active?: boolean;
  /** Add the `has-dot` marker. */
  hasDot?: boolean;
  /** Button edge length in pixels. 28 is the spec's `md`; the tab strip's two are 26. */
  size?: number;
  /** Icon edge length in pixels. The prototype uses 16 in the topbar and 12-14 elsewhere. */
  iconSize?: number;
  /** `id` from the spec's element inventory, so the DOM can be diffed against the prototype. */
  id?: string;
  className?: string;
  /** Greyed out and not clickable - for an action that has nothing to act on yet. */
  disabled?: boolean;
}

export function IconButton({
  icon: Icon,
  label,
  onClick,
  active = false,
  hasDot = false,
  size = 28,
  iconSize = 16,
  id,
  className,
  disabled = false,
}: IconButtonProps) {
  const classes = [
    'icon-btn',
    'grid place-items-center rounded-md shrink-0 transition-all duration-fast ease-ease',
    active
      ? 'bg-accent-subtle text-accent'
      : 'text-text-secondary hover:bg-bg-hover hover:text-text-primary',
    'active:scale-[.94] disabled:pointer-events-none disabled:opacity-40',
    hasDot ? 'has-dot' : '',
    className ?? '',
  ]
    .filter(Boolean)
    .join(' ');

  return (
    <button
      type="button"
      id={id}
      className={classes}
      style={{ width: size, height: size }}
      title={label}
      aria-label={label}
      disabled={disabled}
      onClick={onClick}
    >
      <Icon size={iconSize} aria-hidden="true" />
    </button>
  );
}
