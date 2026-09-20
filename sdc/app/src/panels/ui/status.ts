import { strings } from '../../strings';
import type { HostStatus, SessionState } from '../../store/sessions';

/**
 * The two state vocabularies of the app, in class form: a host's four states (spec section 7.3) and
 * a session's five (7.3, 7.4).
 *
 * They are here rather than inside one component because the same dot is drawn in five places - the
 * topbar's host pill, a sidebar host header, a sidebar session row, a tab, and the status bar -
 * and a colour that means "waiting" in one of them and something else in another would be a bug
 * nobody would see until it mattered. Colour never carries the meaning alone (spec section 8.6):
 * every one of these is paired with a label or an icon by its caller.
 */

/** `.host-status.*` - 7px, with a halo except when offline. */
export const HOST_STATUS_CLASS: Record<HostStatus, string> = {
  connected: 'bg-state-success shadow-[0_0_0_2px_var(--green-subtle)]',
  degraded: 'bg-state-waiting shadow-[0_0_0_2px_var(--orange-subtle)]',
  offline: 'bg-state-idle',
  connecting: 'bg-accent animate-pulse-ring',
};

/** The word next to it, for tooltips, popover rows and `aria-label`s. */
export const HOST_STATUS_LABEL: Record<HostStatus, string> = {
  connected: strings.popover.connected,
  degraded: strings.popover.degraded,
  offline: strings.popover.offline,
  connecting: strings.popover.connecting,
};

/** The dot at the head of a host's status (the host pill's 6px dot, the status bar's 6px dot). */
export const HOST_STATUS_DOT_CLASS: Record<HostStatus, string> = {
  connected: 'bg-state-success shadow-[0_0_6px_var(--state-success)]',
  degraded: 'bg-state-waiting shadow-[0_0_6px_var(--state-waiting)]',
  offline: 'bg-state-idle',
  connecting: 'bg-accent animate-pulse-ring',
};

/** `.sdot.*` / `.tab-dot.*` - running pulses, waiting and error glow, the rest are flat. */
export const SESSION_STATE_CLASS: Record<SessionState, string> = {
  idle: 'bg-state-idle',
  running: 'bg-state-running animate-pulse-dot',
  waiting: 'bg-state-waiting shadow-[0_0_6px_var(--state-waiting)]',
  success: 'bg-state-success',
  error: 'bg-state-error shadow-[0_0_6px_var(--state-error)]',
};

/**
 * The status bar's single connection dot (spec section 7.15). It is driven by the worst host state,
 * so `connectionState()` in the session store decides which of these three is used.
 */
export const CONNECTION_DOT_CLASS = {
  success: 'bg-state-success',
  waiting: 'bg-state-waiting',
  error: 'bg-state-error',
} as const;
