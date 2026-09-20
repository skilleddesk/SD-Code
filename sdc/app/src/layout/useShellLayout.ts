import { useEffect, useLayoutEffect } from 'react';

import { useLayoutStore, type ShellViewport } from '../store/layout';

/**
 * The React side of the shell: the two hooks that connect it to things outside the store - the
 * window width and the keyboard.
 *
 * Both are exported here and called from src/App.tsx (or, for the keyboard, from STEP 13) so the
 * store stays the only state and this file stays the only place that touches browser APIs.
 *
 * The two widths below are the responsive rules of spec section 7.2, and they appear twice in the
 * codebase on purpose: as `@media` blocks in src/layout/Shell.css, which is what actually re-lays
 * out the grid, and as these `matchMedia` queries, which is what tells the store what the CSS just
 * decided. There is no build step that could share them, so a change to one is a change to both.
 * Both are max-width, so they are inclusive at exactly 1200px and 900px, like the CSS.
 */

/** Matches the shell's `@media (max-width: 1200px)` block: right panel off unless forced on. */
const NARROW_RIGHT_PANEL_QUERY = '(max-width: 1200px)';

/** Matches the shell's `@media (max-width: 900px)` block: both side regions are drawers. */
const MOBILE_DRAWERS_QUERY = '(max-width: 900px)';

/** The current answer to both queries. */
function readViewport(
  narrowRightPanel: MediaQueryList,
  mobileDrawers: MediaQueryList,
): ShellViewport {
  return {
    narrowRightPanel: narrowRightPanel.matches,
    mobileDrawers: mobileDrawers.matches,
  };
}

/**
 * Keeps the store in step with the window width (spec section 7.2): opening or resizing a window
 * into the <=1200px range hides the right panel, into the <=900px range turns both side regions into
 * closed drawers, and going back above 1200px brings the panel back - the same result the
 * prototype's pure-CSS breakpoints produce, but recorded in state so a toggle can then override it.
 *
 * `useLayoutEffect`, not `useEffect`: a window that opens narrow would otherwise paint one frame of
 * a three-column layout before the class changes.
 *
 * Call it once, from the composition root. It subscribes to nothing in the store - it writes
 * through `getState()`, so it costs no re-renders, and `syncViewport` is idempotent, so React 18's
 * double-invoked effects in development are harmless.
 */
export function useResponsiveShell(): void {
  useLayoutEffect(() => {
    const narrowRightPanel = window.matchMedia(NARROW_RIGHT_PANEL_QUERY);
    const mobileDrawers = window.matchMedia(MOBILE_DRAWERS_QUERY);

    const sync = (): void => {
      useLayoutStore.getState().syncViewport(readViewport(narrowRightPanel, mobileDrawers));
    };

    /* Once on mount: the window is already some width when the app starts. */
    sync();

    narrowRightPanel.addEventListener('change', sync);
    mobileDrawers.addEventListener('change', sync);

    return () => {
      narrowRightPanel.removeEventListener('change', sync);
      mobileDrawers.removeEventListener('change', sync);
    };
  }, []);
}

/**
 * The three layout shortcuts of spec section 9.1:
 *
 *   Ctrl+B   toggleSidebar()   Sidebar toggle
 *   Ctrl+J   toggleRight()     Right panel toggle
 *   Ctrl+\\   toggleSplit()     Split view
 *
 * Ported from the prototype's single `keydown` listener (design/ui-prototype.html, KEYBOARD):
 * `ctrlKey || metaKey` counts as the modifier, the two letter shortcuts stand down while a text
 * field has focus, and Ctrl+\\ does not (there is no way to type it inside a field). The rest of the
 * prototype's list - Ctrl+K, Ctrl+P, Ctrl+N, Ctrl+,, Ctrl+Enter, Ctrl+W, Escape - belongs to the
 * modules that own those actions, so they are not here.
 *
 * KEYBOARD REGISTRATION IS STEP 13. This hook is deliberately not called from src/App.tsx yet; the
 * step that lands the keyboard will call it once, at the same level as `useResponsiveShell()`.
 */
export function useLayoutShortcuts(): void {
  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent): void => {
      if (!event.ctrlKey && !event.metaKey) {
        return;
      }

      const activeElement = document.activeElement;
      const inInput =
        activeElement instanceof HTMLElement &&
        ['INPUT', 'TEXTAREA', 'SELECT'].includes(activeElement.tagName);

      const { toggleSidebar, toggleRight, toggleSplit } = useLayoutStore.getState();

      if (!inInput && event.key.toLowerCase() === 'b') {
        event.preventDefault();
        toggleSidebar();
        return;
      }

      if (!inInput && event.key.toLowerCase() === 'j') {
        event.preventDefault();
        toggleRight();
        return;
      }

      if (event.key === '\\') {
        event.preventDefault();
        toggleSplit();
      }
    };

    document.addEventListener('keydown', handleKeyDown);

    return () => {
      document.removeEventListener('keydown', handleKeyDown);
    };
  }, []);
}
