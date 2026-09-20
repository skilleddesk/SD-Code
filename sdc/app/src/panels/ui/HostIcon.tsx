import { Laptop, Server } from 'lucide-react';

import type { HostType } from '../../store/sessions';

/**
 * `.host-icon` - the small square that says "this machine" or "that server" (spec section 7.3).
 *
 * It appears at three sizes: 18x18 in a sidebar's host header, 20x20 in the popovers' rows, and
 * 16x16 in a split view's pane header (spec section 9.15). All three are the same object - a
 * gradient chip with a 9-10px glyph inside - so it takes its size as props rather than being
 * written out three times.
 *
 * The two gradients are tokens (`--grad-host-local`, `--grad-host-vps`); the glyph colours are
 * `--accent-hover` and `--purple`, which is what the prototype's two literal blues and violets
 * resolve to.
 */
export interface HostIconProps {
  type: HostType;
  /** Chip edge length. 18 in the sidebar, 20 in a popover row, 16 in a pane header. */
  size?: number;
  /** Glyph edge length. The prototype uses 10 at 18px and 9 at 16px. */
  iconSize?: number;
  className?: string;
}

const TONE: Record<HostType, string> = {
  local: '[background-image:var(--grad-host-local)] text-accent-hover',
  vps: '[background-image:var(--grad-host-vps)] text-purple',
};

export function HostIcon({ type, size = 18, iconSize = 10, className }: HostIconProps) {
  const Glyph = type === 'local' ? Laptop : Server;

  return (
    <span
      className={
        'host-icon ' +
        type +
        ' grid place-items-center shrink-0 rounded-sm ' +
        TONE[type] +
        ' ' +
        (className ?? '')
      }
      style={{ width: size, height: size }}
      aria-hidden="true"
    >
      <Glyph size={iconSize} />
    </span>
  );
}
