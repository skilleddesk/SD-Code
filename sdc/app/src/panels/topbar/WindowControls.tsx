import { useEffect, useState } from 'react';
import { createPortal } from 'react-dom';

import { framelessWindow } from '../../lib/frameless';
import { strings } from '../../strings';

/**
 * Minimise, maximise and close, drawn by the window itself (0.21).
 *
 * The native caption bar was a white strip above a dark app on Windows - each system paints it in its own
 * colours, not the window's - so the window has no native frame on any platform and the topbar is the
 * caption: it drags the window (`data-tauri-drag-region`), a double-click maximises, and these three
 * buttons close the row - the same on Windows, macOS and Linux, as the owner asked.
 */
export function WindowControls() {
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    if (!framelessWindow()) {
      return;
    }

    let off: (() => void) | undefined;
    let gone = false;

    void import('@tauri-apps/api/window').then(async ({ getCurrentWindow }) => {
      const win = getCurrentWindow();
      const sync = async (): Promise<void> => setMaximized(await win.isMaximized());

      await sync();

      const unlisten = await win.onResized(() => void sync());

      if (gone) {
        unlisten();
      } else {
        off = unlisten;
      }
    });

    return () => {
      gone = true;
      off?.();
    };
  }, []);

  if (!framelessWindow()) {
    return null;
  }

  const run = (action: 'minimize' | 'toggleMaximize' | 'close'): void => {
    void import('@tauri-apps/api/window').then(({ getCurrentWindow }) => getCurrentWindow()[action]());
  };

  /* 0.22: drawn on the window's top layer (a portal over everything), so a dialog's backdrop never covers them -
     the window could not be minimised, closed or dragged while the welcome wizard or any dialog was open (found by
     the Linux real-mouse check). The caption strip is the drag area while a dialog is open (CSS: body:has(.modal-bd)). */
  return createPortal(
    <>
    <div className="caption-strip" data-tauri-drag-region aria-hidden="true" />
    <div className="win-controls" role="group" aria-label={strings.topbar.window.group}>
      <button type="button" className="win-btn" title={strings.topbar.window.minimize} aria-label={strings.topbar.window.minimize} onClick={() => run('minimize')}>
        <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
          <path d="M0 5h10" stroke="currentColor" strokeWidth="1" />
        </svg>
      </button>
      <button
        type="button"
        className="win-btn"
        title={maximized ? strings.topbar.window.restore : strings.topbar.window.maximize}
        aria-label={maximized ? strings.topbar.window.restore : strings.topbar.window.maximize}
        onClick={() => run('toggleMaximize')}
      >
        {maximized ? (
          <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true" fill="none" stroke="currentColor" strokeWidth="1">
            <rect x="0.5" y="2.5" width="7" height="7" rx="1" />
            <path d="M2.5 2.5V1.5a1 1 0 0 1 1-1h5a1 1 0 0 1 1 1v5a1 1 0 0 1-1 1h-1" />
          </svg>
        ) : (
          <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true" fill="none" stroke="currentColor" strokeWidth="1">
            <rect x="0.5" y="0.5" width="9" height="9" rx="1.5" />
          </svg>
        )}
      </button>
      <button type="button" className="win-btn win-close" title={strings.topbar.window.close} aria-label={strings.topbar.window.close} onClick={() => run('close')}>
        <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
          <path d="M0.5 0.5l9 9M9.5 0.5l-9 9" stroke="currentColor" strokeWidth="1.05" />
        </svg>
      </button>
    </div>
    </>,
    document.body,
  );
}

