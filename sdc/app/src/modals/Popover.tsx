import { useEffect, useRef, type ReactNode } from 'react';

import type { Anchor } from '../store/overlays';

/**
 * The shared popover frame - spec section 9.5's `#newChatPopover` and 7.1's host switcher, which
 * are the same object with different rows in them: an overlay surface, a title, a list, and the
 * behaviour every popover in the app shares.
 *
 * Three things it owns so no caller has to:
 *
 *   Position    `anchor` is the trigger's viewport rect (`anchorBelow()` in the overlay store), and
 *               the frame is `position: fixed`, so it is drawn above every region no matter which
 *               one the trigger lives in. It is nudged left when it would fall off the right edge.
 *   Dismissal   a pointer press outside, or Escape, closes it - the same two gestures the spec
 *               gives every overlay (section 8.6: "Esc closes every overlay").
 *   Animation   the 180ms drop, from `animate-drop-down` (see tailwind.config.ts).
 *
 * The row classes below are exported because both popovers draw the same rows: an optional status
 * dot, a two-line body (name over mono detail) and a trailing affordance.
 */

/** One row: hover, radius and the 12.5px body. */
export const POPOVER_ITEM_CLASS =
  'popover-item flex items-center gap-[10px] px-[10px] py-[8px] rounded-md cursor-pointer text-[12.5px] text-text-secondary transition-colors duration-fast ease-ease hover:bg-bg-hover hover:text-text-primary';

/** The title strip above the rows: `New chat on…`, `Active host`. */
export const POPOVER_TITLE_CLASS =
  'popover-title px-[10px] pb-[6px] pt-[10px] text-[10px] font-bold uppercase tracking-[.1em] text-text-muted';

/** The mono second line of a row: `3 chats · connected`. */
export const POPOVER_DESC_CLASS =
  'pop-desc mt-[1px] font-mono text-[10.5px] text-text-muted';

/** The row's first line: the host's name. */
export const POPOVER_NAME_CLASS = 'pop-name font-medium text-text-primary';

export interface PopoverProps {
  /** Viewport point the popover hangs from; null means "closed", so the caller can pass its state. */
  anchor: Anchor | null;
  /** Title strip, from src/strings.ts. */
  title: string;
  onClose: () => void;
  /** Extra classes for the frame, e.g. a different minimum width. */
  className?: string;
  children: ReactNode;
}

/** Leaves room for the popover's own width + padding when clamping it against the right edge. */
const EDGE_INSET = 296;

export function Popover({ anchor, title, onClose, className, children }: PopoverProps) {
  const frameRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (anchor === null) {
      return;
    }

    const handlePointerDown = (event: PointerEvent): void => {
      if (frameRef.current && !frameRef.current.contains(event.target as Node)) {
        onClose();
      }
    };

    const handleKeyDown = (event: KeyboardEvent): void => {
      if (event.key === 'Escape') {
        onClose();
      }
    };

    /* Capture phase, so a stopPropagation() inside a trigger cannot leave the popover hanging. */
    document.addEventListener('pointerdown', handlePointerDown, true);
    document.addEventListener('keydown', handleKeyDown);

    return () => {
      document.removeEventListener('pointerdown', handlePointerDown, true);
      document.removeEventListener('keydown', handleKeyDown);
    };
  }, [anchor, onClose]);

  if (anchor === null) {
    return null;
  }

  const left = Math.max(8, Math.min(anchor.x, window.innerWidth - EDGE_INSET));

  return (
    <div
      ref={frameRef}
      className={
        'popover fixed z-[400] p-[6px] rounded-lg bg-bg-overlay border border-border-default shadow-xl animate-drop-down ' +
        (className ?? 'min-w-[280px]')
      }
      style={{ left, top: anchor.y, maxHeight: `calc(100vh - ${anchor.y + 16}px)` }}
      role="dialog"
    >
      <div className={POPOVER_TITLE_CLASS}>{title}</div>
      {children}
    </div>
  );
}
