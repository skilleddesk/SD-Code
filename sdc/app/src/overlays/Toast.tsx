import { X } from 'lucide-react';
import { useEffect, useRef } from 'react';

import { redoRewind } from '../store/intents';
import { usePrefsStore } from '../store/prefs';
import { useAppStore } from '../store/store';
import { useToastStore } from '../store/toast';
import { strings } from '../strings';

/** The action the daemon's rewind toast carries (`rewind.apply`). */
const UNDO_REWIND = 'Undo this';

/**
 * `#toastWrap` - the one toast stack (spec section 9.14).
 *
 * Fixed to the bottom centre, 44px up, stacked upward, newest last. The queue is the event log's
 * (`Toast` events in, `ToastDismissed` events out); this component holds the only timer, and all it
 * does with it is *report* that the hold elapsed. That is the difference between a component that
 * shows state and a component that owns it (master spec section 3.3).
 *
 * Each toast's hold is its own: 3s by default, 10s for the rewind's `Undo this`, because undoing a
 * rewind is a decision rather than a notification. **Every toast also carries an ×** (0.7.5): the
 * report was *"delete korle notification ashe ... remove ar option thake nah"*, and it was exact - the
 * close control existed only next to an action chip, so a message with no action (`Chat deleted`) could
 * not be closed by hand at all, only waited out.
 *
 * The timers live in a map keyed by toast id rather than in one effect run's array, and that is the
 * other half of the same report: an effect that re-armed *every* toast whenever any toast changed meant
 * a window with any traffic at all kept pushing the oldest message's deadline back for ever, so a toast
 * that looked immortal was simply never given its 3 seconds.
 */
export function Toast() {
  const toasts = useToastStore((state) => state.toasts);
  const dismiss = useToastStore((state) => state.dismiss);
  /* One *string*, not an array, and that is load-bearing: a store selector has to return a stable
     value, because `useSyncExternalStore` calls it again on every commit and compares with
     `Object.is`. `state.toasts.map(...)` makes a brand-new array each call, so React saw the
     snapshot "change" every time, re-rendered, saw it change again, hit its 50-update limit and
     unmounted the tree - a window with nothing in it. A joined string is a primitive, so two equal
     holds are the same value and the subscription settles. (This is the bug that shipped in
     0.4.1-0.4.3; `App.render.test.tsx` is the guard that now catches the whole class.) */
  const holdKey = useAppStore((state) =>
    state.toasts.map((toast) => `${toast.id}:${toast.holdMs}`).join('|'),
  );
  /** One timer per toast id, so a toast that arrives later cannot extend an earlier one's life. */
  const timers = useRef(new Map<number, number>());

  useEffect(() => {
    const live = new Set(toasts.map((toast) => toast.id));

    for (const [id, timer] of timers.current) {
      if (!live.has(id)) {
        window.clearTimeout(timer);
        timers.current.delete(id);
      }
    }

    for (const toast of toasts) {
      if (timers.current.has(toast.id)) {
        continue;
      }

      timers.current.set(
        toast.id,
        window.setTimeout(() => {
          timers.current.delete(toast.id);
          dismiss(toast.id);
        }, toast.holdMs),
      );
    }
    /* The key is the ids *and* their holds, so a re-render that changes neither touches no timer. */
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [holdKey, dismiss]);

  useEffect(() => {
    const running = timers.current;

    return () => {
      for (const timer of running.values()) {
        window.clearTimeout(timer);
      }

      running.clear();
    };
  }, []);

  if (toasts.length === 0) {
    return null;
  }

  return (
    <div
      className="toast-wrap fixed bottom-[44px] left-1/2 z-[2000] flex max-w-[90vw] -translate-x-1/2 flex-col items-center gap-[8px] pointer-events-none"
      id="toastWrap"
      role="status"
      aria-live="polite"
    >
      {toasts.map((toast) => (
        <div
          key={toast.id}
          role="alert"
          className="toast pointer-events-auto flex max-w-[520px] animate-toast-in items-center gap-[12px] rounded-lg border border-border-default bg-bg-overlay px-[16px] py-[10px] text-[12.5px] text-text-primary shadow-lg"
        >
          <span
            /*
             * Two lines, however long the sentence is.
             *
             * A toast is a message, not a report: the daemon used to push the whole `ssh` refusal here
             * and three of them covered the Provider Hub. The full sentence lives on the row it is about
             * (the host's status line, the error card), so this clamps and keeps the title for the rest.
             */
            className="toast-text line-clamp-2"
            title={toast.message}
          >
            {toast.message}
          </span>
          {toast.action === null ? null : (
            <button
              type="button"
              className="toast-action ml-auto whitespace-nowrap px-[4px] font-semibold text-accent hover:text-accent-hover"
              title={toast.action === UNDO_REWIND ? strings.rightPanel.timeMachine.redo : strings.toast.dismiss}
              onClick={() => {
                /* The rewind's toast says "Undo this", and until v4 the button only closed the toast. It is
                   the Time Machine's Redo now, for the chat on screen. */
                const sessionId = usePrefsStore.getState().activeTab;

                if (toast.action === UNDO_REWIND && sessionId !== null) {
                  void redoRewind(sessionId);
                }

                dismiss(toast.id);
              }}
            >
              {toast.action}
            </button>
          )}
          <button
            type="button"
            className={
              'toast-close grid h-[18px] w-[18px] shrink-0 place-items-center rounded-sm text-text-muted transition-colors duration-fast ease-ease hover:bg-bg-hover hover:text-text-primary ' +
              (toast.action === null ? 'ml-auto' : '')
            }
            title={strings.toast.close}
            aria-label={strings.toast.close}
            onClick={() => dismiss(toast.id)}
          >
            <X size={12} aria-hidden="true" />
          </button>
        </div>
      ))}
    </div>
  );
}
