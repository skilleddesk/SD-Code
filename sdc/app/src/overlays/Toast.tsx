import { useEffect } from 'react';

import { useAppStore } from '../store/store';
import { useToastStore } from '../store/toast';
import { strings } from '../strings';

/**
 * `#toastWrap` - the one toast stack (spec section 9.14).
 *
 * Fixed to the bottom centre, 44px up, stacked upward, newest last. The queue is the event log's
 * (`Toast` events in, `ToastDismissed` events out); this component holds the only timer, and all it
 * does with it is *report* that the hold elapsed. That is the difference between a component that
 * shows state and a component that owns it (master spec section 3.3).
 *
 * Each toast's hold is its own: 3s by default, 10s for the rewind's `Undo this`, because undoing a
 * rewind is a decision rather than a notification. Clicking the action chip dismisses it right away.
 */
export function Toast() {
  const toasts = useToastStore((state) => state.toasts);
  const dismiss = useToastStore((state) => state.dismiss);
  const holdFors = useAppStore((state) => state.toasts.map((toast) => `${toast.id}:${toast.holdMs}`));

  useEffect(() => {
    const timers = toasts.map((toast) =>
      window.setTimeout(() => dismiss(toast.id), toast.holdMs),
    );

    return () => {
      for (const timer of timers) {
        window.clearTimeout(timer);
      }
    };
    /* The key is the ids *and* their holds, so a re-render that changes neither restarts nothing. */
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [holdFors.join('|'), dismiss]);

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
          <span>{toast.message}</span>
          {toast.action === null ? null : (
            <button
              type="button"
              className="toast-action ml-auto whitespace-nowrap px-[4px] font-semibold text-accent hover:text-accent-hover"
              title={strings.toast.dismiss}
              onClick={() => dismiss(toast.id)}
            >
              {toast.action}
            </button>
          )}
        </div>
      ))}
    </div>
  );
}
