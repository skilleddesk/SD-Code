import { framelessWindow } from './frameless';

/**
 * The title bar's drag and double-click on Linux (0.22).
 *
 * Tauri's own drag script starts a window move on the first mouse press. On Linux that move hands the pointer to
 * the window manager, so the second press of a double-click often never reaches the page and the window does not
 * maximise - it passed one CI run and failed the next (xdotool under openbox). Here, on Linux only, a press on a
 * drag region starts nothing; the move begins once the mouse actually moves a few pixels with the button held, and
 * a double press toggles maximise. Windows and macOS keep Tauri's behaviour, which works there.
 */
export function installLinuxCaption(): void {
  if (!framelessWindow() || !/Linux/i.test(navigator.userAgent) || /Android/i.test(navigator.userAgent)) {
    return;
  }

  const onCaption = (target: EventTarget | null): boolean =>
    target instanceof HTMLElement && target.hasAttribute('data-tauri-drag-region') && target.getAttribute('data-tauri-drag-region') !== 'false';
  const windowApi = () => import('@tauri-apps/api/window').then(({ getCurrentWindow }) => getCurrentWindow());
  let pressed: { x: number; y: number } | null = null;

  /* Capture phase on window: runs before Tauri's listener on document, which is then stopped. */
  window.addEventListener(
    'mousedown',
    (event) => {
      if (event.button !== 0 || !onCaption(event.target)) {
        return;
      }

      event.preventDefault();
      event.stopImmediatePropagation();

      if (event.detail >= 2) {
        pressed = null;
        void windowApi().then((win) => win.toggleMaximize());
        return;
      }

      pressed = { x: event.clientX, y: event.clientY };
    },
    true,
  );

  window.addEventListener(
    'mousemove',
    (event) => {
      if (pressed === null || (event.buttons & 1) === 0) {
        pressed = null;
        return;
      }

      if (Math.abs(event.clientX - pressed.x) + Math.abs(event.clientY - pressed.y) >= 4) {
        pressed = null;
        void windowApi().then((win) => win.startDragging());
      }
    },
    true,
  );

  window.addEventListener('mouseup', () => (pressed = null), true);
}
