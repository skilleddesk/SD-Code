import { useEffect, useRef, type ReactNode } from 'react';
import { X } from 'lucide-react';

import { strings } from '../strings';
import { useOverlayStore } from '../store/overlays';

/**
 * The shared modal frame - spec sections 9.10-9.13 and 8.6.
 *
 * Everything the six surfaces of this step share lives here, once:
 *
 *   role/aria      `role="dialog"`, `aria-modal="true"` and an `aria-label`, which is what makes a
 *                  screen reader announce the dialog instead of reading the page behind it.
 *   Backdrop       `modal-bd` (src/styles/globals.css) - the dimmed, blurred layer. A click on it
 *                  closes, which is the gesture the prototype wires for every modal.
 *   Escape         closes. The global keyboard map also has an `Esc` command, but an overlay must
 *                  not depend on that listener being mounted, so this one is local and runs first
 *                  (the global handler checks `defaultPrevented`).
 *   Focus trap     Tab cycles inside the dialog, and focus lands on the dialog when it opens. This
 *                  is the accessibility requirement of spec section 8.6, and it is the reason the
 *                  trap is in the frame rather than in each modal.
 *
 * `center` moves the dialog to the vertical middle (the hub, Settings, Add host and Permission);
 * the palette and search use the top-aligned default.
 */

export interface ModalProps {
  open: boolean;
  /** Accessible name. Comes from src/strings.ts, never a literal. */
  label: string;
  onClose: () => void;
  /** Vertically centre the dialog (spec section 9.10's hub, 9.11's Settings, 9.12, 9.13). */
  center?: boolean;
  /** The dialog's own width/height classes; each surface has its own geometry. */
  className?: string;
  /** `true` renders the frame without a close button - the surfaces that carry their own header. */
  bare?: boolean;
  children: ReactNode;
}

/** Tab-able elements, in the order the trap walks them. */
const FOCUSABLE =
  'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

export function Modal({ open, label, onClose, center, className, bare, children }: ModalProps) {
  const frameRef = useRef<HTMLDivElement | null>(null);
  const closeAll = useOverlayStore((state) => state.closeAll);

  /*
   * The newest `onClose`, in a ref.
   *
   * This is not a style preference, it is the fix for a reported bug: the effect below used to depend
   * on `onClose`, and every caller passes an inline arrow (`onClose={() => close()}`), so the effect
   * re-ran on *every render*. The Connect dialog renders once a second while it polls a sign-in, and
   * each of those renders yanked focus to the dialog's first control - the URL field above the code
   * field. Pasting a code looked like it "did not work": the paste went nowhere, and the caret jumped
   * up a row. A modal focuses once, when it opens.
   */
  const closeRef = useRef(onClose);

  closeRef.current = onClose;

  useEffect(() => {
    if (!open) {
      return undefined;
    }

    const frame = frameRef.current;
    const first = frame?.querySelector<HTMLElement>(FOCUSABLE);

    /* Focus the first control, or the dialog itself when it holds none yet (spec section 8.6). */
    (first ?? frame)?.focus();

    const handleKeyDown = (event: KeyboardEvent): void => {
      if (event.key === 'Escape') {
        event.preventDefault();

        /*
         * The global map's `Esc` closes *everything*; a modal's own `Esc` closes the modal. Both
         * are wanted: Esc is the app-level "get me out", and the permission dialog's `when()`
         * predicate keeps the two from fighting while an approval is up.
         */
        if (frame?.dataset.stacked === 'true') {
          closeAll();
        } else {
          closeRef.current();
        }

        return;
      }

      if (event.key !== 'Tab' || !frame) {
        return;
      }

      const focusable = [...frame.querySelectorAll<HTMLElement>(FOCUSABLE)].filter(
        (element) => element.offsetParent !== null,
      );

      if (focusable.length === 0) {
        event.preventDefault();
        return;
      }

      const firstElement = focusable[0];
      const lastElement = focusable[focusable.length - 1];

      if (event.shiftKey && document.activeElement === firstElement) {
        event.preventDefault();
        lastElement?.focus();
      } else if (!event.shiftKey && document.activeElement === lastElement) {
        event.preventDefault();
        firstElement?.focus();
      }
    };

    document.addEventListener('keydown', handleKeyDown);

    return () => {
      document.removeEventListener('keydown', handleKeyDown);
    };
  }, [open, closeAll]);

  if (!open) {
    return null;
  }

  return (
    <div
      className={'modal-bd' + (center === true ? ' center' : '')}
      data-modal={label}
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) {
          onClose();
        }
      }}
    >
      <div
        ref={frameRef}
        role="dialog"
        aria-modal="true"
        aria-label={label}
        tabIndex={-1}
        className={(bare === true ? '' : 'dialog ') + (className ?? '')}
      >
        {/*
          The frame's own close button.

          `ModalProps.bare` has documented "`true` renders the frame without a close button" since this
          file was written, and the button itself was never rendered - so every modal this frame carries
          could only be closed by Escape or by clicking the dimmed backdrop, which is not something a
          person should have to know. Reported as "or cross thakbe close korar".
        */}
        {bare === true ? null : (
          <button
            type="button"
            className="dialog-close absolute right-[8px] top-[8px] z-[1] grid h-[24px] w-[24px] place-items-center rounded-md text-text-muted transition-colors duration-fast ease-ease hover:bg-bg-hover hover:text-text-primary"
            aria-label={strings.modal.close}
            title={strings.modal.close}
            onClick={() => closeRef.current()}
          >
            <X size={14} aria-hidden="true" />
          </button>
        )}

        {children}
      </div>
    </div>
  );
}
